use anyhow::{Context, Result};
use colored::Colorize;
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::process::Command;

use super::version_resolve::{
    get_download_url, get_iojs_download_url, get_latest_lts_version, get_latest_version,
    get_source_url, resolve_iojs_version, resolve_version,
};
use crate::config::{load_config, resolve_alias};
use crate::download::{copy_from_cache, download_to_cache, is_cached};
use crate::extract::{extract_archive, extract_iojs_archive};
use crate::i18n::{format_t, T};
use crate::system::{
    get_nvm_dir, os_suffix, prepend_to_path, verify_checksum, verify_gpg_signature, GpgStatus,
    IOJS_URI,
};
use crate::utils::{atomic_write, iojs_version_number};
use indicatif::{ProgressBar, ProgressStyle};

/// Resolved target for an install operation. Built by `build_install_target`
/// and consumed by the source/binary/post-install phases so `install` itself
/// stays a thin orchestrator.
struct InstallTarget {
    target_version: String,
    download_url: String,
    archive_name: String,
    product_name: &'static str,
    is_iojs: bool,
}

/// Bundles the 11 install-related CLI flags so they can be passed to
/// `build_install_target` / `run_post_install_hooks` as a single value
/// instead of an unwieldy 11-parameter signature (which forced
/// `#[allow(clippy::too_many_arguments)]` on both functions).
pub struct InstallConfig {
    pub version: Option<String>,
    pub lts: bool,
    pub latest: bool,
    pub lts_newer: bool,
    pub offline: bool,
    pub reinstall_packages_from: Option<String>,
    pub latest_npm: bool,
    pub latest_yarn: bool,
    pub latest_pnpm: bool,
    pub source: bool,
    pub no_gpg_verify: bool,
}

/// Resolve what to install. Returns `Ok(None)` when `--lts-newer` short-
/// circuits because the latest LTS is already installed (the "already
/// installed" message has already been printed in that case, so the caller
/// should just return `Ok(())`).
fn build_install_target(
    cfg: &InstallConfig,
    base_url: &str,
    nvm_dir: &Path,
) -> Result<Option<InstallTarget>> {
    // io.js detection: "iojs", "io.js", "iojs-3.3.1", "io.js-3.3.1"
    let is_iojs = if let Some(v) = &cfg.version {
        let lv = v.to_lowercase();
        lv.starts_with("iojs") || lv.starts_with("io.js")
    } else {
        false
    };

    if is_iojs && cfg.source {
        anyhow::bail!("{}", T("iojs_source_unsupported"));
    }

    if is_iojs {
        let ver = cfg.version.as_ref().unwrap();
        let lv = ver.to_lowercase();
        let ver_input = if lv == "iojs" || lv == "io.js" {
            "3.3.1".to_string()
        } else {
            lv
        };
        let resolved = resolve_iojs_version(&ver_input, IOJS_URI)?;
        let url = get_iojs_download_url(&resolved, IOJS_URI)?;
        let ver_num = iojs_version_number(&resolved).unwrap_or_default();
        return Ok(Some(InstallTarget {
            target_version: resolved,
            download_url: url,
            archive_name: format!("iojs-v{}-{}", ver_num, os_suffix()),
            product_name: "io.js",
            is_iojs: true,
        }));
    }

    // `--lts-newer` acts like `--lts` but skips download when the latest
    // LTS is already installed. Useful in setup scripts that want "ensure
    // some LTS is present" without re-downloading on every run.
    let effective_lts = cfg.lts || cfg.lts_newer;
    let target = if effective_lts {
        get_latest_lts_version(base_url)?
    } else if cfg.latest {
        get_latest_version(base_url)?
    } else if let Some(v) = &cfg.version {
        resolve_version(v, base_url)?
    } else {
        anyhow::bail!("{}", T("specify_version_lts_latest"));
    };

    // `--lts-newer` short-circuit: skip install if already installed.
    if cfg.lts_newer && !cfg.lts {
        let version_dir = nvm_dir.join(&target);
        if version_dir.exists() {
            println!(
                "{} {}",
                "ℹ".cyan().bold(),
                format_t("already_installed", std::slice::from_ref(&target)).cyan()
            );
            println!(
                "  {} {}",
                T("run_label").dimmed(),
                format_t("run_command", std::slice::from_ref(&target))
                    .yellow()
                    .bold()
            );
            return Ok(None);
        }
    }

    // `--offline` must skip `get_download_url` (which hits the network
    // via `get_tags`). Build the URL locally from the well-known layout
    // `{base_url}{version}/node-{version}-{suffix}` — this matches every
    // real release on nodejs.org/mirrors, so the only thing that can
    // fail afterwards is a cache miss, which the binary-install phase
    // reports as `offline_no_cache`.
    let url = if cfg.offline {
        let suffix = os_suffix();
        format!(
            "{}/{}/node-{}-{}",
            base_url.trim_end_matches('/'),
            target,
            target,
            suffix
        )
    } else {
        get_download_url(&target, base_url)?
    };
    let archive_name = if cfg.source {
        format!("node-{}.tar.gz", target)
    } else {
        format!("node-{}-{}", target, os_suffix())
    };

    Ok(Some(InstallTarget {
        target_version: target,
        download_url: url,
        archive_name,
        product_name: "Node.js",
        is_iojs: false,
    }))
}

/// Compile and install Node.js from source. Used when `--source` is passed.
/// io.js source compilation is rejected upstream in `build_install_target`.
///
/// External toolchain required: a POSIX `sh`, `make`, a C compiler, and `tar`
/// supporting `--strip-components=1` (GNU tar; bsdtar on Windows 10 build
/// 17063+ behaves differently for that flag — `--source` is primarily a
/// Unix power-user feature, prebuilt binaries are the default install path).
fn install_from_source(
    target: &InstallTarget,
    base_url: &str,
    offline: bool,
    nvm_dir: &Path,
    version_dir: &Path,
) -> Result<()> {
    let ncpus = std::thread::available_parallelism()
        .map(|p| p.get())
        .unwrap_or(1);
    let source_url = get_source_url(&target.target_version, base_url)?;
    let archive_name = &target.archive_name;

    if offline {
        if !is_cached(archive_name) {
            anyhow::bail!(
                "{}",
                format_t(
                    "offline_source_no_cache",
                    std::slice::from_ref(archive_name)
                )
            );
        }
        println!("  {} {}", "ℹ".cyan().bold(), T("using_cache").cyan());
        copy_from_cache(
            archive_name,
            &nvm_dir.join(format!("{}.src.tmp", target.target_version)),
        )?;
    } else {
        let cached_path = download_to_cache(&source_url, archive_name)?;
        fs::copy(
            &cached_path,
            nvm_dir.join(format!("{}.src.tmp", target.target_version)),
        )?;
    }

    let src_tmp = nvm_dir.join(format!("{}.src.tmp", target.target_version));
    let build_dir = nvm_dir.join(format!("node-v{}.build", target.target_version));
    fs::create_dir_all(&build_dir)?;

    println!("  {} {}", "›".dimmed(), T("source_extract"));
    let status = Command::new("tar")
        .arg("xf")
        .arg(&src_tmp)
        .arg("-C")
        .arg(&build_dir)
        .arg("--strip-components=1")
        .status()
        .context(T("tar_extract_failed"))?;
    if !status.success() {
        anyhow::bail!(
            "{} ({})",
            T("extract_source_failed"),
            status.code().unwrap_or(-1)
        );
    }
    fs::remove_file(&src_tmp).ok();

    println!(
        "  {} {}",
        "›".dimmed(),
        format_t("source_configure", &[version_dir.display().to_string()])
    );
    let cfg = Command::new("./configure")
        .arg(format!("--prefix={}", version_dir.display()))
        .current_dir(&build_dir)
        .status()
        .context(T("configure_spawn_failed"))?;
    if !cfg.success() {
        anyhow::bail!("{} ({})", T("configure_failed"), cfg.code().unwrap_or(-1));
    }

    println!(
        "  {} {}",
        "›".dimmed(),
        format_t("source_make", &[ncpus.to_string()])
    );
    let m = Command::new("make")
        .args(["-j", &ncpus.to_string()])
        .current_dir(&build_dir)
        .status()
        .context(T("make_failed"))?;
    if !m.success() {
        anyhow::bail!("{} ({})", T("make_failed"), m.code().unwrap_or(-1));
    }

    println!("  {} {}", "›".dimmed(), T("source_install"));
    let mi = Command::new("make")
        .arg("install")
        .current_dir(&build_dir)
        .status()
        .context(T("make_install_failed"))?;
    if !mi.success() {
        anyhow::bail!("{} ({})", T("make_install_failed"), mi.code().unwrap_or(-1));
    }

    fs::remove_dir_all(&build_dir).ok();

    let npm_path = version_dir.join("bin").join("npm");
    if !npm_path.exists() {
        println!("  {} {}", "ℹ".cyan().bold(), T("source_npm_fetch"));
        download_prebuilt_npm(version_dir, &target.target_version)?;
    }

    println!();
    println!(
        "{} {} {}",
        "✓".green().bold(),
        target.product_name.green().bold(),
        format_t("compiled", std::slice::from_ref(&target.target_version))
            .white()
            .bold()
    );
    Ok(())
}

/// Download and extract a prebuilt binary tarball. Performs SHA-256 checksum
/// and GPG signature verification (Node.js only; io.js mirrors don't ship
/// SHASUMS256.txt). A *failed* GPG signature aborts the install, since that
/// indicates the tarball or SHASUMS256.txt was tampered with.
fn install_binary(
    target: &InstallTarget,
    base_url: &str,
    offline: bool,
    no_gpg_verify: bool,
    nvm_dir: &Path,
    version_dir: &Path,
) -> Result<()> {
    let temp_file = nvm_dir.join(format!("{}.tmp", target.target_version));

    if offline {
        if is_cached(&target.archive_name) {
            println!("  {} {}", "ℹ".cyan().bold(), T("using_cache").cyan());
            copy_from_cache(&target.archive_name, &temp_file)?;
        } else {
            anyhow::bail!(format_t(
                "offline_no_cache",
                std::slice::from_ref(&target.archive_name)
            ));
        }
    } else {
        let cached_path = download_to_cache(&target.download_url, &target.archive_name)?;
        if cached_path != temp_file {
            fs::copy(&cached_path, &temp_file).context(T("copy_from_cache_failed"))?;
        }
    }

    if !target.is_iojs {
        print!("  {} ", T("checksum_label").dimmed());
        if !offline
            && verify_checksum(
                &temp_file,
                &target.archive_name,
                base_url,
                &target.target_version,
            )?
        {
            println!("{}", T("checksum_verified").green().bold());
        } else if offline {
            println!("{}", T("checksum_offline").dimmed());
        } else {
            println!("{}", T("checksum_skipped").yellow().bold());
        }

        // GPG signature verification of SHASUMS256.txt — extra trust layer
        // on top of the SHA-256 checksum. Degrades gracefully (skip) when
        // gpg is missing, the mirror lacks the .sig file, or --no-gpg-verify
        // is passed. A *failed* signature (gpg ran and rejected it) aborts,
        // since that indicates tampering.
        print!("  {} ", T("gpg_label").dimmed());
        match verify_gpg_signature(base_url, &target.target_version, no_gpg_verify, offline)? {
            GpgStatus::Verified => println!("{}", T("gpg_verified").green().bold()),
            GpgStatus::SkippedDisabled => println!("{}", T("gpg_disabled").dimmed()),
            GpgStatus::SkippedOffline => println!("{}", T("gpg_offline").dimmed()),
            GpgStatus::SkippedNoGpg => println!("{}", T("gpg_no_gpg").dimmed()),
            GpgStatus::SkippedNoSig => println!("{}", T("gpg_no_sig").dimmed()),
            GpgStatus::SkippedKeyImport => {
                println!("{}", T("gpg_key_import_failed").yellow().bold())
            }
            GpgStatus::Failed => {
                println!("{}", T("gpg_failed").red().bold());
                anyhow::bail!("{}", T("gpg_failed_abort"));
            }
        }
    }

    if target.is_iojs {
        extract_iojs_archive(&temp_file, version_dir, &target.target_version)?;
    } else {
        extract_archive(&temp_file, version_dir, &target.target_version)?;
    }
    fs::remove_file(&temp_file).ok();

    println!();
    println!(
        "{} {} {}",
        "✓".green().bold(),
        target.product_name.green().bold(),
        format_t(
            "installed_exclaim",
            std::slice::from_ref(&target.target_version)
        )
        .white()
        .bold()
    );
    Ok(())
}

/// Run post-install hooks requested via CLI flags: `--latest-npm`,
/// `--latest-yarn`, `--latest-pnpm`, and `--reinstall-packages-from`.
/// Errors from `--reinstall-packages-from` are reported but do not fail the
/// install (the version itself was installed successfully); the other three
/// propagate errors normally since the user explicitly asked for them.
fn run_post_install_hooks(
    target: &InstallTarget,
    cfg: &InstallConfig,
    nvm_dir: &Path,
) -> Result<()> {
    // --latest-npm after install (skip for io.js: npm is bundled)
    if cfg.latest_npm && !target.is_iojs {
        println!();
        install_latest_package_inner(&target.target_version, "npm")?;
    }
    // --latest-yarn / --latest-pnpm after install. Unlike npm, yarn and pnpm
    // are not bundled with node, so installing them right after `nvm install`
    // is a common setup step and applies to io.js installs too.
    if cfg.latest_yarn {
        println!();
        install_latest_package_inner(&target.target_version, "yarn")?;
    }
    if cfg.latest_pnpm {
        println!();
        install_latest_package_inner(&target.target_version, "pnpm")?;
    }

    // --reinstall-packages-from after install
    if let Some(from_ver) = &cfg.reinstall_packages_from {
        // Resolve aliases (default, lts/iron, bare "22.22.2", etc.) the same
        // way `nvm reinstall-packages` does, so the option accepts the same
        // identifiers users already use elsewhere.
        let from_resolved = match crate::config::resolve_alias(from_ver) {
            Ok(v) => v,
            Err(e) => {
                eprintln!(
                    "  {} {}",
                    "⚠".yellow().bold(),
                    format_t("migration_failed", &[e.to_string()])
                );
                return Ok(());
            }
        };
        println!();
        println!(
            "{} {}",
            "▶".cyan().bold(),
            format_t(
                "migrating_packages",
                &[from_resolved.clone(), target.target_version.clone()]
            )
            .cyan()
            .bold()
        );
        // reinstall_packages_inner reads `current` to find the source
        // version's global packages; point it at the freshly installed
        // version first.
        let current_file = nvm_dir.join("current");
        atomic_write(&current_file, &target.target_version).ok();
        if let Err(e) = reinstall_packages_inner(&from_resolved, &target.target_version) {
            eprintln!(
                "  {} {}",
                "⚠".yellow().bold(),
                format_t("migration_failed", &[e.to_string()])
            );
        }
    }

    Ok(())
}

pub fn install(cfg: InstallConfig) -> Result<()> {
    let config = load_config()?;
    let base_url = super::get_base_url(&config);
    let nvm_dir = get_nvm_dir();

    let target = match build_install_target(&cfg, base_url, &nvm_dir)? {
        Some(t) => t,
        None => return Ok(()), // --lts-newer short-circuited (already installed)
    };

    let version_dir = nvm_dir.join(&target.target_version);

    // If the version is already installed (non-empty dir), skip the download/
    // extract — matches nvm-sh's "already installed" behavior. Avoids the
    // "Directory not empty" error from extracting into an existing dir.
    // Source installs are allowed to proceed (user explicitly wants a rebuild).
    if !cfg.source && version_dir.exists() {
        let not_empty = fs::read_dir(&version_dir)
            .map(|mut rd| rd.next().is_some())
            .unwrap_or(false);
        if not_empty {
            println!(
                "{} {}",
                "ℹ".cyan().bold(),
                format_t(
                    "already_installed",
                    std::slice::from_ref(&target.target_version)
                )
                .cyan()
            );
            println!(
                "  {} {}",
                T("run_label").dimmed(),
                format_t("run_command", std::slice::from_ref(&target.target_version))
                    .yellow()
                    .bold()
            );
            return Ok(());
        }
    }

    println!(
        "{} {} {}",
        "▶".cyan().bold(),
        format_t(
            if cfg.source {
                "compiling_product"
            } else {
                "installing_product"
            },
            &[target.product_name.to_string()]
        )
        .cyan()
        .bold(),
        target.target_version.white().bold()
    );
    println!("  {} {}", T("url_label").dimmed(), target.download_url);

    if cfg.source {
        install_from_source(&target, base_url, cfg.offline, &nvm_dir, &version_dir)?;
    } else {
        install_binary(
            &target,
            base_url,
            cfg.offline,
            cfg.no_gpg_verify,
            &nvm_dir,
            &version_dir,
        )?;
    }

    run_post_install_hooks(&target, &cfg, &nvm_dir)?;

    println!(
        "  {} {}",
        T("run_label").dimmed(),
        format_t("run_command", std::slice::from_ref(&target.target_version))
            .yellow()
            .bold()
    );

    Ok(())
}

/// Upgrade a globally-installed package manager (`npm`, `yarn`, or `pnpm`)
/// to its latest release, using the bundled npm in `version`'s bin dir as the
/// installer.
///
/// The flow mirrors `nvm install-npm`:
///   1. Resolve + validate the target version (must be installed, must ship npm).
///   2. Print an "Upgrading X for vX.Y.Z" banner.
///   3. Run `npm install -g <package>@latest` with that version's bin on PATH.
///   4. On failure for `npm` only: retry via `npm exec --yes npm@latest --`
///      to dodge npm 10.x's self-upgrade bug. yarn/pnpm don't have this bug
///      (they install into their own dirs, npm doesn't replace itself), so
///      their first-attempt failure is a real failure and we bail.
fn install_latest_package_inner(version: &str, package: &str) -> Result<()> {
    let nvm_dir = get_nvm_dir();
    let resolved = version.to_string();
    let version_dir = nvm_dir.join(&resolved);
    if !version_dir.exists() {
        anyhow::bail!(
            "{}",
            format_t("not_installed", std::slice::from_ref(&resolved))
        );
    }
    let npm_path = version_dir.join("bin").join("npm");
    if !npm_path.exists() {
        anyhow::bail!(
            "{}",
            format_t("version_no_npm", std::slice::from_ref(&resolved))
        );
    }
    // Per-package i18n keys so each tool reports its own name in messages.
    let (upgrading_key, upgraded_key, failed_key) = match package {
        "yarn" => ("upgrading_yarn", "yarn_upgraded", "yarn_upgrade_failed"),
        "pnpm" => ("upgrading_pnpm", "pnpm_upgraded", "pnpm_upgrade_failed"),
        _ => ("upgrading_npm", "npm_upgraded", "npm_upgrade_failed"),
    };
    println!(
        "  {} {}",
        "▶".cyan().bold(),
        format_t(upgrading_key, std::slice::from_ref(&resolved)).cyan()
    );
    let path_env = prepend_to_path(&version_dir.join("bin"));
    // First attempt: plain `npm install -g <package>@latest`. Works for
    // yarn/pnpm (they don't replace themselves) and for npm 11+ (whose
    // reify no longer moves its own deps out from under itself).
    let pkg_spec = format!("{}@latest", package);
    let status = Command::new(&npm_path)
        .args(["install", "-g", &pkg_spec])
        .env("PATH", &path_env)
        .status()
        .context(format_t(
            "package_upgrade_spawn_failed",
            &[package.to_string()],
        ))?;
    if status.success() {
        println!("    {} {}", "✓".green().bold(), T(upgraded_key).green());
        return Ok(());
    }
    // npm-specific retry: npm 10.x has a self-upgrade bug (reify moves its
    // own node_modules, then crashes with "Cannot find module
    // 'promise-retry'" when creating bin links). Retry via `npm exec --yes
    // npm@latest --` which downloads a fresh npm to a temp dir and runs it
    // from there. yarn/pnpm don't have this bug, so we bail immediately.
    if package == "npm" {
        eprintln!(
            "  {} {}",
            "↻".yellow().bold(),
            T("npm_upgrade_retry_npx").yellow()
        );
        let status = Command::new(&npm_path)
            .args([
                "exec",
                "--yes",
                "npm@latest",
                "--",
                "install",
                "-g",
                "npm@latest",
                "--prefix",
            ])
            .arg(version_dir.display().to_string())
            .env("PATH", &path_env)
            .status()
            .context(T("npm_upgrade_failed"))?;
        if status.success() {
            println!("    {} {}", "✓".green().bold(), T("npm_upgraded").green());
            return Ok(());
        }
    }
    anyhow::bail!("{} ({})", T(failed_key), status.code().unwrap_or(-1));
}

fn reinstall_packages_inner(from: &str, to: &str) -> Result<()> {
    let nvm_dir = get_nvm_dir();
    let from_dir = nvm_dir.join(from);
    if !from_dir.exists() {
        anyhow::bail!("{}", format_t("source_not_installed", &[from.to_string()]));
    }
    let to_dir = nvm_dir.join(to);
    if !to_dir.exists() {
        anyhow::bail!("{}", format_t("target_not_installed", &[to.to_string()]));
    }
    let from_npm = from_dir.join("bin").join("npm");
    let to_npm = to_dir.join("bin").join("npm");

    let output = Command::new(&from_npm)
        .arg("list")
        .arg("-g")
        .arg("--depth=0")
        .arg("--json")
        .env("PATH", prepend_to_path(&from_dir.join("bin")))
        .output()
        .context(T("list_global_packages_failed"))?;

    // `npm list --json` only writes the dependency tree to stdout on exit
    // success. On a non-zero exit (broken install, corrupt node_modules, npm
    // crash) stdout is empty or an error blob, so the previous
    // `from_str(...).unwrap_or_default()` silently produced `Null` and
    // `reinstall-packages` reported "0 packages migrated" instead of failing.
    // Bail explicitly so the user sees the real cause.
    if !output.status.success() {
        let code = output.status.code().unwrap_or(-1);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = stderr.trim();
        let msg = format_t("npm_list_failed_code", &[code.to_string()]);
        if detail.is_empty() {
            anyhow::bail!("{}", msg);
        } else {
            anyhow::bail!("{}: {}", msg, detail);
        }
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap_or_default();
    if let Some(deps) = json.get("dependencies").and_then(|d| d.as_object()) {
        let new_path = prepend_to_path(&to_dir.join("bin"));
        // Exclude npm/corepack from the count: they are bundled, not migrated.
        let pkg_count = deps
            .keys()
            .filter(|k| *k != "npm" && *k != "corepack")
            .count();
        println!(
            "  {} {}",
            "ℹ".cyan().bold(),
            format_t("reinstall_count", &[pkg_count.to_string()])
        );
        let mut migrated = 0usize;
        let mut failed: Vec<String> = Vec::new();
        for pkg in deps.keys() {
            if pkg == "npm" || pkg == "corepack" {
                continue;
            }
            print!("    {} {}... ", "•".cyan(), pkg);
            io::stdout().flush().ok();
            let status = match Command::new(&to_npm)
                .arg("install")
                .arg("-g")
                .arg(pkg)
                .env("PATH", &new_path)
                .status()
            {
                Ok(s) => s,
                Err(_) => {
                    println!("{}", "✗".red().bold());
                    failed.push(pkg.clone());
                    continue;
                }
            };
            if status.success() {
                println!("{}", "✓".green().bold());
                migrated += 1;
            } else {
                println!(
                    "{} {}",
                    "✗".red().bold(),
                    format_t(
                        "package_failed_code",
                        &[status.code().unwrap_or(-1).to_string()]
                    )
                    .red()
                );
                failed.push(pkg.clone());
            }
        }
        println!(
            "    {} {} {} {}",
            "✓".green().bold(),
            format_t("packages_migrated", &[migrated.to_string()]).green(),
            "→".dimmed(),
            to.white().bold()
        );
        if !failed.is_empty() {
            anyhow::bail!(
                "{}",
                format_t("reinstall_failed_list", &[failed.join(", ")])
            );
        }
    }
    Ok(())
}

/// Download a prebuilt npm tarball and install it into the version's lib/node_modules.
///
/// The npm registry (npmjs.org) is distinct from the Node.js binary mirror
/// (`config.mirror`): `config.mirror` only mirrors `nodejs.org/dist/`, while
/// npm tarballs live on the npm registry. We therefore always hit
/// `registry.npmjs.org` here — the user's npm CLI itself uses the same
/// registry by default (configurable via `~/.npmrc`).
///
/// For tamper resistance we fetch the per-version registry metadata
/// (`/npm/{version}`) and verify the downloaded tarball's SHA-512 against the
/// `dist.integrity` field. If the metadata fetch fails (e.g. offline), we
/// fall back to the hardcoded tarball URL and skip verification — same
/// "Skipped vs Failed" pattern as GPG verification.
fn download_prebuilt_npm(version_dir: &Path, version: &str) -> Result<()> {
    let ver_num = version.trim_start_matches('v');
    let npm_tarball = format!("npm-v{}.tgz", ver_num);
    let fallback_url = format!("https://registry.npmjs.org/npm/-/npm-{}.tgz", ver_num);
    let npm_tar_path = get_nvm_dir().join(&npm_tarball);

    if !npm_tar_path.exists() {
        println!("  {} {}", "›".dimmed(), T("downloading_npm"));
        let client = crate::proxy::build_http_client();

        // Fetch registry metadata for the canonical tarball URL + integrity
        // hash. On any failure we fall back to the hardcoded URL and skip
        // integrity verification (with a warning), so a transient registry
        // outage doesn't block source-build npm installs.
        let meta_url = format!("https://registry.npmjs.org/npm/{}", ver_num);
        let registry_result: Option<(String, Option<String>)> = (|| {
            let resp = client.get(&meta_url).send().ok()?;
            if !resp.status().is_success() {
                return None;
            }
            let body = resp.text().ok()?;
            let json: serde_json::Value = serde_json::from_str(&body).ok()?;
            let dist = json.get("dist")?;
            let tarball = dist.get("tarball")?.as_str()?.to_string();
            let integrity = dist
                .get("integrity")
                .and_then(|i| i.as_str())
                .map(|s| s.to_string());
            Some((tarball, integrity))
        })();
        let (npm_url, expected_integrity) = match registry_result {
            Some((url, Some(int))) => (url, Some(int)),
            Some((url, None)) => {
                eprintln!(
                    "  {} {}",
                    "⚠".yellow().bold(),
                    T("npm_integrity_skipped").yellow()
                );
                (url, None)
            }
            None => (fallback_url, None),
        };

        let response = client
            .get(&npm_url)
            .send()
            .context(T("npm_tarball_download_failed"))?;
        if !response.status().is_success() {
            anyhow::bail!(
                "{}",
                format_t("npm_download_failed", std::slice::from_ref(&npm_url))
            );
        }
        let total = response.content_length().unwrap_or(0);
        let pb = ProgressBar::new(total);
        pb.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta})")
                .unwrap()
                .progress_chars("#>-"),
        );
        let mut src = pb.wrap_read(response);
        let mut dest = std::fs::File::create(&npm_tar_path)?;
        let bytes_copied =
            std::io::copy(&mut src, &mut dest).context(T("npm_tarball_write_failed"))?;
        pb.finish_with_message(T("progress_done"));

        // Detect truncated downloads: if the server advertised a content
        // length and we got fewer bytes, the connection dropped mid-transfer.
        // Without this check, `tar xzf` below would fail with a confusing
        // "unexpected EOF" instead of a clear "truncated" message.
        if total > 0 && bytes_copied < total {
            std::fs::remove_file(&npm_tar_path).ok();
            anyhow::bail!("{}", T("npm_download_truncated"));
        }

        // Verify SHA-512 integrity against the registry's `dist.integrity`.
        // This catches a compromised CDN cache serving a tampered tarball at
        // the legitimate URL — TLS alone doesn't protect against that.
        if let Some(integrity) = expected_integrity {
            if verify_npm_integrity(&npm_tar_path, &integrity).is_err() {
                std::fs::remove_file(&npm_tar_path).ok();
                anyhow::bail!("{}", T("npm_integrity_failed"));
            }
        }
    }

    // Extract npm tarball into lib/node_modules.
    // Requires `tar` with `--strip-components=1` (GNU tar; Windows 10 build
    // 17063+ ships bsdtar which supports this flag). On older Windows the
    // prebuilt-binary install path is used instead (npm ships bundled).
    let node_modules = version_dir.join("lib").join("node_modules");
    std::fs::create_dir_all(&node_modules)?;

    let status = Command::new("tar")
        .arg("xzf")
        .arg(&npm_tar_path)
        .arg("-C")
        .arg(&node_modules)
        .arg("--strip-components=1")
        .status()
        .context(T("npm_extract_failed"))?;
    if !status.success() {
        anyhow::bail!("{}", T("npm_extract_failed"));
    }

    // Wire up the `npm` executable so it lands on PATH. The tarball ships
    // `bin/npm` (a JS launcher); we symlink (Unix) or copy (Windows) it into
    // the version's `bin/` dir alongside `node`, so `nvm use <ver>` exposes
    // npm immediately.
    //
    // These used to be `.ok()` — silently swallowing a read-only `bin/`,
    // a Windows AV file lock, or a full disk. The result was "npm installed!"
    // with no npm on PATH. Propagate the error instead so the failure is
    // visible and the install is reported as failed.
    let npm_bin_src = node_modules.join("bin").join("npm");
    let npm_bin_dst = version_dir.join("bin").join("npm");
    let npm_bin_dst_parent = version_dir.join("bin");
    std::fs::create_dir_all(&npm_bin_dst_parent)?;
    #[cfg(unix)]
    std::os::unix::fs::symlink(&npm_bin_src, &npm_bin_dst)
        .with_context(|| format!("failed to symlink npm bin at {}", npm_bin_dst.display()))?;
    #[cfg(windows)]
    std::fs::copy(&npm_bin_src, &npm_bin_dst)
        .map(|_| ())
        .with_context(|| format!("failed to copy npm bin at {}", npm_bin_dst.display()))?;

    // Best-effort cleanup of the downloaded tarball; a failure here doesn't
    // invalidate the install, so don't surface it as an error.
    let _ = std::fs::remove_file(&npm_tar_path);
    Ok(())
}

/// Verify a downloaded npm tarball against the registry's `dist.integrity`
/// field. The field is in the Subresource Integrity format: `<algo>-<b64>`.
/// We support `sha512` (the algorithm npm uses for all current releases).
fn verify_npm_integrity(file_path: &Path, integrity: &str) -> Result<()> {
    let (algo, expected_b64) = integrity
        .split_once('-')
        .ok_or_else(|| anyhow::anyhow!("malformed integrity field"))?;
    if algo != "sha512" {
        anyhow::bail!("unsupported integrity algorithm: {} (only sha512)", algo);
    }
    use base64::Engine;
    let expected = base64::engine::general_purpose::STANDARD
        .decode(expected_b64)
        .map_err(|e| anyhow::anyhow!("invalid base64 in integrity field: {}", e))?;

    use sha2::Digest;
    let mut file = std::fs::File::open(file_path)?;
    let mut hasher = sha2::Sha512::new();
    std::io::copy(&mut file, &mut hasher)?;
    let actual = hasher.finalize();

    if actual.as_slice() != expected.as_slice() {
        anyhow::bail!("sha512 mismatch");
    }
    Ok(())
}

fn get_installed_version(version: &str) -> Result<String> {
    let resolved = resolve_alias(version)?;
    if resolved.starts_with("system:") {
        return Ok(resolved);
    }
    let nvm_dir = get_nvm_dir();
    let version_dir = nvm_dir.join(&resolved);
    if !version_dir.exists() {
        anyhow::bail!(
            "{}",
            format_t("not_installed", std::slice::from_ref(&resolved))
        );
    }
    Ok(resolved)
}

/// Resolve the target version for a package-upgrade command. With no
/// argument, act on the *current* version (matches nvm-sh behavior). Only
/// fall back to `default` when there's no current set — `nvm install-latest-<pkg>`
/// with no current and no default is a setup error.
fn resolve_install_target(version: Option<&str>) -> Result<String> {
    let target = match version {
        Some(v) => v.to_string(),
        None => match super::get_current_version()? {
            Some(v) => v,
            None => {
                let config = load_config()?;
                match config.default_version {
                    Some(v) => v,
                    None => anyhow::bail!("{}", T("no_current_version_set")),
                }
            }
        },
    };
    get_installed_version(&target)
}

pub fn install_latest_npm(version: Option<&str>) -> Result<()> {
    let resolved = resolve_install_target(version)?;
    install_latest_package_inner(&resolved, "npm")
}

pub fn install_latest_yarn(version: Option<&str>) -> Result<()> {
    let resolved = resolve_install_target(version)?;
    install_latest_package_inner(&resolved, "yarn")
}

pub fn install_latest_pnpm(version: Option<&str>) -> Result<()> {
    let resolved = resolve_install_target(version)?;
    install_latest_package_inner(&resolved, "pnpm")
}

pub fn reinstall_packages(from_version: &str) -> Result<()> {
    // Resolve aliases (default, lts/iron, bare "22.22.2", etc.) so the
    // user can pass the same kind of identifier they would to `nvm use`.
    let resolved_from = crate::config::resolve_alias(from_version)?;
    // Validate the source version *before* requiring a current version: the
    // user-facing input is "from_version", and a missing current is a setup
    // problem that should be reported only if the source is otherwise valid.
    let nvm_dir = get_nvm_dir();
    let from_dir = nvm_dir.join(&resolved_from);
    if !from_dir.exists() {
        anyhow::bail!(
            "{}",
            format_t("source_not_installed", std::slice::from_ref(&resolved_from))
        );
    }
    let current = super::get_current_version()?
        .ok_or_else(|| anyhow::anyhow!("{}", T("no_current_version_set")))?;
    reinstall_packages_inner(&resolved_from, &current)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a real SRI `sha512-<base64>` integrity string for `contents`
    /// so the verify_npm_integrity tests don't depend on a hardcoded hash.
    fn make_integrity(contents: &str) -> String {
        use base64::Engine;
        use sha2::Digest;
        let mut hasher = sha2::Sha512::new();
        hasher.update(contents.as_bytes());
        let digest = hasher.finalize();
        format!(
            "sha512-{}",
            base64::engine::general_purpose::STANDARD.encode(digest)
        )
    }

    #[test]
    fn test_verify_npm_integrity_accepts_correct_hash() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("npm.tgz");
        let contents = "fake npm tarball contents";
        std::fs::write(&file, contents).expect("write");
        let integrity = make_integrity(contents);
        verify_npm_integrity(&file, &integrity).expect("matching integrity should verify");
    }

    #[test]
    fn test_verify_npm_integrity_rejects_tampered_file() {
        // The download flow computes the hash over the bytes on disk. If the
        // tarball was truncated or replaced after the metadata fetch, the
        // hash must not match — this is the tamper-detection guarantee.
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("npm.tgz");
        std::fs::write(&file, "original contents").expect("write");
        let integrity = make_integrity("tampered contents");
        let err = verify_npm_integrity(&file, &integrity).expect_err("mismatched hash should fail");
        assert!(
            err.to_string().contains("mismatch"),
            "expected mismatch error, got: {}",
            err
        );
    }

    #[test]
    fn test_verify_npm_integrity_rejects_wrong_algorithm() {
        // npm registry only ships sha512 SRI; sha256 or others must be
        // rejected so we never silently skip verification for an algo we
        // can't check.
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("npm.tgz");
        std::fs::write(&file, "x").expect("write");
        let err =
            verify_npm_integrity(&file, "sha256-AAAA").expect_err("non-sha512 algo should fail");
        assert!(err.to_string().contains("sha512"));
    }

    #[test]
    fn test_verify_npm_integrity_rejects_malformed_field() {
        // No `-` separator → can't split algo from hash. Must fail rather
        // than panic on a None unwrap.
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("npm.tgz");
        std::fs::write(&file, "x").expect("write");
        verify_npm_integrity(&file, "noseparator").expect_err("malformed integrity should fail");
    }

    #[test]
    fn test_verify_npm_integrity_rejects_invalid_base64() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("npm.tgz");
        std::fs::write(&file, "x").expect("write");
        verify_npm_integrity(&file, "sha512-!!!not-base64!!!")
            .expect_err("invalid base64 should fail");
    }
}
