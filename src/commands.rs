use anyhow::{Context, Result};
use colored::Colorize;
use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::{
    handle_mirror, list_all_aliases, load_config, remove_alias, remove_from_shell_config,
    resolve_alias, save_config, set_alias, update_shell_config, Config,
};
use crate::download::{copy_from_cache, download_to_cache, is_cached};
use crate::extract::{extract_archive, extract_iojs_archive};
use crate::system::{get_cache_dir, get_nvm_dir, get_tags, GpgStatus, IOJS_URI, os_suffix, verify_checksum, verify_gpg_signature, URI};
use crate::utils::{get_installed_versions, is_lts_version, iojs_version_number, lts_codename_to_major, normalize_iojs_version, parse_major};
use crate::i18n::{T, format_t};
use indicatif::{ProgressBar, ProgressStyle};

fn get_codename(version: &str) -> String {
    parse_major(version)
        .and_then(|m| {
            let map = lts_codename_to_major();
            for (name, major) in map {
                if major == m {
                    return Some(name.to_string());
                }
            }
            None
        })
        .unwrap_or_else(|| "-".to_string())
}

/// Calculate display width of a string (ignoring ANSI color codes)
fn display_width(s: &str) -> usize {
    let mut width = 0;
    let mut in_escape = false;
    for c in s.chars() {
        if in_escape {
            if c == 'm' {
                in_escape = false;
            }
        } else if c == '\x1b' {
            in_escape = true;
        } else {
            let cp = c as u32;
            // Approximate: CJK characters and wide symbols take 2 columns
            let w = if (0x1100..=0x115F).contains(&cp)    // Hangul Jamo
                || (0x2E80..=0x303E).contains(&cp)     // CJK Radicals etc.
                || (0x3041..=0x33FF).contains(&cp)     // Hiragana etc.
                || (0x3400..=0x4DBF).contains(&cp)     // CJK Ext A
                || (0x4E00..=0x9FFF).contains(&cp)     // CJK Unified
                || (0xA000..=0xA4CF).contains(&cp)     // Yi Syllables
                || (0xAC00..=0xD7A3).contains(&cp)     // Hangul
                || (0xF900..=0xFAFF).contains(&cp)     // CJK Compat
                || (0xFE30..=0xFE4F).contains(&cp)     // CJK Compat Forms
                || (0xFF00..=0xFF60).contains(&cp)     // Fullwidth Forms
                || (0xFFE0..=0xFFE6).contains(&cp)     // Fullwidth Forms
                || (0x20000..=0x2FFFD).contains(&cp)   // CJK Ext B-D
                || (0x30000..=0x3FFFD).contains(&cp)
            {
                2
            } else {
                1
            };
            width += w;
        }
    }
    width
}

fn pad_right(s: &str, width: usize) -> String {
    let w = display_width(s);
    if w >= width {
        s.to_string()
    } else {
        format!("{}{}", s, " ".repeat(width - w))
    }
}

fn pad_left(s: &str, width: usize) -> String {
    let w = display_width(s);
    if w >= width {
        s.to_string()
    } else {
        format!("{}{}", " ".repeat(width - w), s)
    }
}

/// Render a beautiful bordered table.
/// columns: (header_text, alignment) where alignment is 0=left, 1=right, 2=center
/// header can be "" to indicate no header text for that column
fn render_table(title: &str, columns: &[(&str, u8)], rows: &[Vec<String>]) {
    let n_cols = columns.len();
    let padding = 1;

    // Calculate column widths based on content
    let mut col_widths: Vec<usize> = columns
        .iter()
        .map(|(h, _)| if h.is_empty() { 0 } else { display_width(h) })
        .collect();
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            if i < n_cols {
                let w = display_width(cell);
                if w > col_widths[i] {
                    col_widths[i] = w;
                }
            }
        }
    }
    // Ensure minimum width for marker column
    for w in col_widths.iter_mut() {
        if *w == 0 {
            *w = 1;
        }
    }

    let inner_total: usize = col_widths.iter().sum::<usize>() + padding * 2 * n_cols + (n_cols - 1);
    let title_width = display_width(title);
    let total_width = inner_total.max(title_width + padding * 2);

    // Build a separator line with custom corner/join chars
    let sep_line = |left: char, mid: char, right: char, fill: char| -> String {
        let mut s = String::new();
        s.push(left);
        for (i, w) in col_widths.iter().enumerate() {
            s.push_str(&fill.to_string().repeat(w + padding * 2));
            if i < n_cols - 1 {
                s.push(mid);
            }
        }
        s.push(right);
        s
    };

    // Top border
    println!("  ╭{}╮", "─".repeat(total_width));

    // Title row
    if !title.is_empty() {
        let title_str = format!(" {} ", title.bold().cyan());
        let tw = display_width(&title_str);
        let right_pad = total_width.saturating_sub(tw);
        println!("  │{}{}│", title_str, " ".repeat(right_pad));
        println!("  {}", sep_line('├', '┼', '┤', '─'));
    }

    // Header row (skip empty header columns)
    let has_any_header = columns.iter().any(|(h, _)| !h.is_empty());
    if has_any_header {
        let mut header = String::from("  │");
        for (i, (h, _)) in columns.iter().enumerate() {
            let content = if h.is_empty() {
                " ".repeat(col_widths[i])
            } else {
                h.bold().cyan().to_string()
            };
            let padded = format!("{}{}{}", " ".repeat(padding), content, " ".repeat(padding));
            let padded = pad_right(&padded, col_widths[i] + padding * 2);
            header.push_str(&padded);
            if i < n_cols - 1 {
                header.push('│');
            }
        }
        header.push('│');
        println!("{}", header);
        println!("  {}", sep_line('├', '┼', '┤', '─'));
    }

    // Data rows
    for row in rows {
        let mut line = String::from("  │");
        for (i, cell) in row.iter().enumerate() {
            let align = columns.get(i).map(|(_, a)| *a).unwrap_or(0);
            let content = match align {
                1 => pad_left(cell, col_widths[i]),
                2 => {
                    let w = col_widths[i];
                    let dw = display_width(cell);
                    let lp = (w - dw) / 2;
                    let rp = w - dw - lp;
                    format!("{}{}{}", " ".repeat(lp), cell, " ".repeat(rp))
                }
                _ => pad_right(cell, col_widths[i]),
            };
            let padded = format!("{}{}{}", " ".repeat(padding), content, " ".repeat(padding));
            let padded = pad_right(&padded, col_widths[i] + padding * 2);
            line.push_str(&padded);
            if i < n_cols - 1 {
                line.push('│');
            }
        }
        line.push('│');
        println!("{}", line);
    }

    // Bottom border
    println!("  ╰{}╯", "─".repeat(total_width));
}

fn version_codename_colored(version: &str) -> String {
    let code = get_codename(version);
    if code == "-" {
        "-".dimmed().to_string()
    } else {
        code.magenta().bold().to_string()
    }
}

fn version_lts_colored(version: &str) -> String {
    if is_lts_version(version) {
        T("lts_badge").green().to_string()
    } else {
        "".to_string()
    }
}

/// Compare two version strings by semantic version (major.minor.patch).
/// Returns greater if a is newer than b. Delegates to `utils::compare_semver`
/// so all version comparisons share one implementation.
fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    crate::utils::compare_semver(a, b)
}

fn get_current_version() -> Result<Option<String>> {
    let nvm_dir = get_nvm_dir();
    let current_file = nvm_dir.join("current");

    if current_file.exists() {
        let version = fs::read_to_string(&current_file)?.trim().to_string();
        if !version.is_empty() {
            return Ok(Some(version));
        }
    }

    Ok(None)
}

fn get_base_url(config: &Config) -> &str {
    config.mirror.as_deref().unwrap_or(URI)
}

pub fn install(
    version: Option<String>,
    lts: bool,
    latest: bool,
    lts_newer: bool,
    offline: bool,
    reinstall_packages_from: Option<String>,
    latest_npm: bool,
    latest_yarn: bool,
    latest_pnpm: bool,
    source: bool,
    no_gpg_verify: bool,
) -> Result<()> {
    let config = load_config()?;
    let base_url = get_base_url(&config);
    let iojs_base_url = IOJS_URI;
    let nvm_dir = get_nvm_dir();
    let ncpus = std::thread::available_parallelism().map(|p| p.get()).unwrap_or(1);

    // Check if this is an io.js install request
    let is_iojs = if let Some(ref v) = version {
        let lv = v.to_lowercase();
        lv.starts_with("iojs") || lv.starts_with("io.js") || lv == "iojs" || lv == "io.js"
    } else {
        false
    };

    if is_iojs && source {
        anyhow::bail!("{}", T("iojs_source_unsupported"));
    }

    let (target_version, download_url, archive_name, product_name): (String, String, String, &str);

    if is_iojs {
        let ver = version.as_ref().unwrap();
        let lv = ver.to_lowercase();
        let ver_input = if lv == "iojs" || lv == "io.js" {
            "3.3.1".to_string()
        } else {
            lv.clone()
        };
        let resolved = resolve_iojs_version(&ver_input, iojs_base_url)?;
        let url = get_iojs_download_url(&resolved, iojs_base_url)?;
        let ver_num = iojs_version_number(&resolved).unwrap_or_default();
        archive_name = format!("iojs-v{}-{}", ver_num, os_suffix());
        target_version = resolved;
        download_url = url;
        product_name = "io.js";
    } else {
        // `--lts-newer` acts like `--lts` but skips download when the latest
        // LTS is already installed. Useful in setup scripts that want "ensure
        // some LTS is present" without re-downloading on every run. Treat it
        // as `--lts` for version resolution, then short-circuit if installed.
        let effective_lts = lts || lts_newer;
        let target = if effective_lts {
            get_latest_lts_version(base_url)?
        } else if latest {
            get_latest_version(base_url)?
        } else if let Some(v) = version {
            resolve_version(&v, base_url)?
        } else {
            anyhow::bail!("{}", T("specify_version_lts_latest"));
        };
        // `--lts-newer` short-circuit: skip install if already installed.
        if lts_newer && !lts {
            let version_dir = nvm_dir.join(&target);
            if version_dir.exists() {
                println!(
                    "{} {}",
                    "ℹ".cyan().bold(),
                    format_t("already_installed", &[target.clone()]).cyan()
                );
                println!(
                    "  {} {}",
                    T("run_label").dimmed(),
                    format_t("run_command", &[target.to_string()]).yellow().bold()
                );
                return Ok(());
            }
        }
        // `--offline` must skip `get_download_url` (which hits the network
        // via `get_tags`). Build the URL locally from the well-known layout
        // `{base_url}{version}/node-{version}-{suffix}` — this matches every
        // real release on nodejs.org/mirrors, so the only thing that can
        // fail afterwards is a cache miss, which the binary-install block
        // below reports as `offline_no_cache`.
        let url = if offline {
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
        let arch = if source {
            format!("node-{}.tar.gz", target)
        } else {
            format!("node-{}-{}", target, os_suffix())
        };
        target_version = target;
        download_url = url;
        archive_name = arch;
        product_name = "Node.js";
    }

    let version_dir = nvm_dir.join(&target_version);

    // If the version is already installed (non-empty dir), skip the download/
    // extract — matches nvm-sh's "already installed" behavior. Avoids the
    // "Directory not empty" error from extracting into an existing dir.
    // Source installs are allowed to proceed (user explicitly wants a rebuild).
    if !source && version_dir.exists() {
        let not_empty = fs::read_dir(&version_dir)
            .map(|mut rd| rd.next().is_some())
            .unwrap_or(false);
        if not_empty {
            println!(
                "{} {}",
                "ℹ".cyan().bold(),
                format_t("already_installed", &[target_version.clone()]).cyan()
            );
            println!(
                "  {} {}",
                T("run_label").dimmed(),
                format_t("run_command", &[target_version.to_string()]).yellow().bold()
            );
            return Ok(());
        }
    }

    println!(
        "{} {} {}",
        "▶".cyan().bold(),
        format_t(if source { "compiling_product" } else { "installing_product" }, &[product_name.to_string()]).cyan().bold(),
        target_version.white().bold()
    );
    println!("  {} {}", T("url_label").dimmed(), download_url);

    // Source install (Node.js only)
    if source {
        let source_url = get_source_url(&target_version, base_url)?;
        if offline {
            if !is_cached(&archive_name) {
                anyhow::bail!(
                    "{}",
                    format_t("offline_source_no_cache", &[archive_name.clone()])
                );
            }
            println!("  {} {}", "ℹ".cyan().bold(), T("using_cache").cyan());
            copy_from_cache(&archive_name, &nvm_dir.join(format!("{}.src.tmp", target_version)))?;
        } else {
            let cached_path = download_to_cache(&source_url, &archive_name)?;
            fs::copy(&cached_path, nvm_dir.join(format!("{}.src.tmp", target_version)))?;
        }

        let src_tmp = nvm_dir.join(format!("{}.src.tmp", target_version));
        let build_dir = nvm_dir.join(format!("node-v{}.build", target_version));
        fs::create_dir_all(&build_dir)?;

        println!("  {} {}", "›".dimmed(), T("source_extract"));
        let status = Command::new("tar")
            .arg("xf").arg(&src_tmp).arg("-C").arg(&build_dir).arg("--strip-components=1")
            .status().context(T("tar_extract_failed"))?;
        if !status.success() { anyhow::bail!("{}", T("extract_source_failed")); }
        fs::remove_file(&src_tmp).ok();

        println!("  {} {}", "›".dimmed(), format_t("source_configure", &[version_dir.display().to_string()]));
        let cfg = Command::new("./configure")
            .arg(format!("--prefix={}", version_dir.display()))
            .current_dir(&build_dir)
            .status()
            .context(T("configure_spawn_failed"))?;
        if !cfg.success() { anyhow::bail!("{}", T("configure_failed")); }

        println!("  {} {}", "›".dimmed(), format_t("source_make", &[ncpus.to_string()]));
        let m = Command::new("make").args(["-j", &ncpus.to_string()])
            .current_dir(&build_dir).status().context(T("make_failed"))?;
        if !m.success() { anyhow::bail!("{}", T("make_failed")); }

        println!("  {} {}", "›".dimmed(), T("source_install"));
        let mi = Command::new("make").arg("install")
            .current_dir(&build_dir).status().context(T("make_install_failed"))?;
        if !mi.success() { anyhow::bail!("{}", T("make_install_failed")); }

        fs::remove_dir_all(&build_dir).ok();

        let npm_path = version_dir.join("bin").join("npm");
        if !npm_path.exists() {
            println!("  {} {}", "ℹ".cyan().bold(), T("source_npm_fetch"));
            download_prebuilt_npm(&version_dir, &target_version)?;
        }

        println!();
        println!(
            "{} {} {}",
            "✓".green().bold(),
            product_name.green().bold(),
            format_t("compiled", &[target_version.clone()]).white().bold()
        );
    } else {
        // Binary install
        let temp_file = nvm_dir.join(format!("{}.tmp", target_version));

        if offline {
            if is_cached(&archive_name) {
                println!("  {} {}", "ℹ".cyan().bold(), T("using_cache").cyan());
                copy_from_cache(&archive_name, &temp_file)?;
            } else {
                anyhow::bail!(
                    format_t("offline_no_cache", &[archive_name])
                );
            }
        } else {
            let cached_path = download_to_cache(&download_url, &archive_name)?;
            if cached_path != temp_file {
                fs::copy(&cached_path, &temp_file).context(T("copy_from_cache_failed"))?;
            }
        }

        if !is_iojs {
            print!("  {} ", T("checksum_label").dimmed());
            if !offline && verify_checksum(&temp_file, &archive_name, base_url, &target_version)? {
                println!("{}", T("checksum_verified").green().bold());
            } else if offline {
                println!("{}", T("checksum_offline").dimmed());
            } else {
                println!("{}", T("checksum_skipped").yellow().bold());
            }

            // GPG signature verification of SHASUMS256.txt. This is an extra
            // trust layer on top of the SHA-256 checksum and degrades
            // gracefully (skip) when gpg is missing, the mirror lacks the
            // .sig file, or --no-gpg-verify is passed. It never aborts the
            // install, so existing behavior is preserved.
            print!("  {} ", T("gpg_label").dimmed());
            match verify_gpg_signature(base_url, &target_version, no_gpg_verify, offline)? {
                GpgStatus::Verified => println!("{}", T("gpg_verified").green().bold()),
                GpgStatus::SkippedDisabled => println!("{}", T("gpg_disabled").dimmed()),
                GpgStatus::SkippedOffline => println!("{}", T("gpg_offline").dimmed()),
                GpgStatus::SkippedNoGpg => println!("{}", T("gpg_no_gpg").dimmed()),
                GpgStatus::SkippedNoSig => println!("{}", T("gpg_no_sig").dimmed()),
                GpgStatus::SkippedKeyImport => println!("{}", T("gpg_key_import_failed").yellow().bold()),
                GpgStatus::Failed => println!("{}", T("gpg_failed").red().bold()),
            }
        }

        // Extract with correct directory prefix
        if is_iojs {
            extract_iojs_archive(&temp_file, &version_dir, &target_version)?;
        } else {
            extract_archive(&temp_file, &version_dir, &target_version)?;
        }
        fs::remove_file(&temp_file).ok();

        println!();
        println!(
            "{} {} {}",
            "✓".green().bold(),
            product_name.green().bold(),
            format_t("installed_exclaim", &[target_version.clone()]).white().bold()
        );
    }

    // --latest-npm after install (skip for io.js: npm is bundled)
    if latest_npm && !is_iojs {
        println!();
        install_latest_package_inner(&target_version, "npm")?;
    }
    // --latest-yarn / --latest-pnpm after install. Unlike npm, yarn and pnpm
    // are not bundled with node, so installing them right after `nvm install`
    // is a common setup step and applies to io.js installs too.
    if latest_yarn {
        println!();
        install_latest_package_inner(&target_version, "yarn")?;
    }
    if latest_pnpm {
        println!();
        install_latest_package_inner(&target_version, "pnpm")?;
    }

    // --reinstall-packages-from after install
    if let Some(from_ver) = reinstall_packages_from {
        // Resolve aliases (default, lts/iron, bare "22.22.2", etc.) the same
        // way `nvm reinstall-packages` does, so the option accepts the same
        // identifiers users already use elsewhere.
        let from_resolved = match crate::config::resolve_alias(&from_ver) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("  {} {}", "⚠".yellow().bold(), format_t("migration_failed", &[e.to_string()]));
                return Ok(());
            }
        };
        println!();
        println!(
            "{} {}",
            "▶".cyan().bold(),
            format_t("migrating_packages", &[from_resolved.clone(), target_version.clone()]).cyan().bold()
        );
        let current_file = nvm_dir.join("current");
        fs::write(&current_file, &target_version).ok();
        if let Err(e) = reinstall_packages_inner(&from_resolved, &target_version) {
            eprintln!("  {} {}", "⚠".yellow().bold(), format_t("migration_failed", &[e.to_string()]));
        }
    }

    println!(
        "  {} {}",
        T("run_label").dimmed(),
        format_t("run_command", &[target_version.to_string()]).yellow().bold()
    );

    Ok(())
}

/// Upgrade a globally-installed package manager (`npm`, `yarn`, or `pnpm`)
/// to its latest release, using the bundled npm in `version`'s bin dir as the
/// installer.
///
/// The flow mirrors `nvm install-latest-npm`:
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
        anyhow::bail!("{}", format_t("not_installed", &[resolved.clone()]));
    }
    let npm_path = version_dir.join("bin").join("npm");
    if !npm_path.exists() {
        anyhow::bail!("{}", format_t("version_no_npm", &[resolved.clone()]));
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
        format_t(upgrading_key, &[resolved.clone()]).cyan()
    );
    let path_env = format!(
        "{}:{}",
        version_dir.join("bin").display(),
        env::var("PATH").unwrap_or_default()
    );
    // First attempt: plain `npm install -g <package>@latest`. Works for
    // yarn/pnpm (they don't replace themselves) and for npm 11+ (whose
    // reify no longer moves its own deps out from under itself).
    let pkg_spec = format!("{}@latest", package);
    let status = Command::new(&npm_path)
        .args(["install", "-g", &pkg_spec])
        .env("PATH", &path_env)
        .status()
        .context(format_t("package_upgrade_spawn_failed", &[package.to_string()]))?;
    if status.success() {
        println!(
            "    {} {}",
            "✓".green().bold(),
            T(upgraded_key).green()
        );
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
                "exec", "--yes", "npm@latest", "--",
                "install", "-g", "npm@latest",
                "--prefix",
            ])
            .arg(version_dir.display().to_string())
            .env("PATH", &path_env)
            .status()
            .context(T("npm_upgrade_failed"))?;
        if status.success() {
            println!(
                "    {} {}",
                "✓".green().bold(),
                T("npm_upgraded").green()
            );
            return Ok(());
        }
    }
    anyhow::bail!("{}", T(failed_key));
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
        .env(
            "PATH",
            format!(
                "{}:{}",
                from_dir.join("bin").display(),
                env::var("PATH").unwrap_or_default()
            ),
        )
        .output()
        .context(T("list_global_packages_failed"))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap_or_default();
    if let Some(deps) = json.get("dependencies").and_then(|d| d.as_object()) {
        let new_path = format!(
            "{}:{}",
            to_dir.join("bin").display(),
            env::var("PATH").unwrap_or_default()
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
                    format_t("package_failed_code", &[status.code().unwrap_or(-1).to_string()]).red()
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

fn resolve_version(input: &str, base_url: &str) -> Result<String> {
    // Fully-specified version "vX.Y.Z" / "X.Y.Z" with two dots: use as-is.
    if input.starts_with('v') && input.matches('.').count() >= 2 {
        return Ok(input.to_string());
    }
    if input.matches('.').count() >= 2 && input.chars().next().map_or(false, |c| c.is_ascii_digit()) {
        return Ok(format!("v{}", input));
    }

    // `lts`, `lts/*`, `lts/-1` (bare) → newest LTS across all lines.
    let lower = input.to_lowercase();
    if lower == "lts" || lower == "lts/*" || lower == "lts/-1" {
        return get_latest_lts_version(base_url);
    }

    // `lts/krypton`, `lts/iron`, ... → newest release in that LTS line.
    // nodejs.org exposes `latest-v{major}.x/` for every major, so we can
    // resolve any LTS codename (or any bare major) by listing that dir.
    if lower.starts_with("lts/") {
        let aliases = crate::config::named_lts_aliases();
        if let Some(prefix) = aliases.get(lower.as_str()) {
            let major = prefix.trim_start_matches('v');
            return get_latest_version_in_major(major, base_url);
        }
        anyhow::bail!("{}", format_t("unknown_lts_alias", &[input.to_string()]));
    }

    // Bare major ("20") or major.minor ("20.5") → newest release in that
    // major line. Matches nvm-sh: `nvm install 20` installs the latest v20.x.x.
    if let Some(major) = bare_major_for_install(input) {
        return get_latest_version_in_major(&major, base_url);
    }

    // Fall back to listing `{base_url}v{input}/` — used for partial versions
    // like "20.11" that should match a specific patch release directory.
    let version_num = input.trim_start_matches('v');
    let tags = get_tags(format!("{}v{}/", base_url, version_num));

    if tags.is_empty() {
        anyhow::bail!("{}", T("cannot_fetch_versions"));
    }

    let suffix = os_suffix();
    let re = regex::Regex::new(r"node-(v[\d.]+)-").unwrap();
    for tag in tags.iter().rev() {
        if tag.ends_with(suffix) {
            if let Some(caps) = re.captures(tag) {
                return Ok(caps[1].to_string());
            }
        }
    }

    anyhow::bail!("{}", format_t("cannot_resolve", &[input.to_string()]))
}

/// If `input` is a bare major ("22") or major.minor ("22.5"), optionally with
/// a leading `v` ("v22", "v22.5"), return the major as a string so we can
/// fetch the `latest-v{major}.x/` directory. Returns `None` for full versions
/// ("22.5.1"), aliases, io.js names, `system`, etc.
fn bare_major_for_install(input: &str) -> Option<String> {
    let s = input.strip_prefix('v').unwrap_or(input);
    let dots = s.matches('.').count();
    if dots > 1 {
        return None;
    }
    if s.is_empty() || !s.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return None;
    }
    if s.chars().all(|c| !c.is_ascii_digit()) {
        return None;
    }
    s.split('.').next().map(|m| m.to_string())
}

/// Fetch the newest release in a major line by listing nodejs.org's
/// `latest-v{major}.x/` directory. Works for both LTS and non-LTS lines.
fn get_latest_version_in_major(major: &str, base_url: &str) -> Result<String> {
    let dir = format!("latest-v{}.x/", major);
    let tags = get_tags(format!("{}{}", base_url, dir));
    if tags.is_empty() {
        anyhow::bail!("{}", T("cannot_fetch_versions"));
    }
    let suffix = os_suffix();
    let re = regex::Regex::new(r"node-(v[\d.]+)-").unwrap();
    for tag in tags.iter().rev() {
        if tag.ends_with(suffix) {
            if let Some(caps) = re.captures(tag) {
                return Ok(caps[1].to_string());
            }
        }
    }
    anyhow::bail!("{}", format_t("cannot_resolve", &[format!("v{}.", major)]))
}

fn get_latest_lts_version(base_url: &str) -> Result<String> {
    // Prefer the official `index.json` manifest — it explicitly tags each
    // release with its LTS codename, which is far more reliable than scraping
    // the `latest-vXX.x/` directory links (those exist for every major,
    // including non-LTS odd ones, and the "highest even" heuristic breaks
    // when a newer non-LTS even major ships before the LTS line bumps).
    let index_url = format!("{}index.json", base_url);
    let client = crate::proxy::build_http_client();
    if let Ok(resp) = client.get(&index_url).send() {
        if resp.status().is_success() {
            if let Ok(text) = resp.text() {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                    if let Some(arr) = json.as_array() {
                        for entry in arr {
                            // Each entry: { "version": "v24.18.0", "lts": "Krypton", ... }
                            // Non-LTS releases have `"lts": false`.
                            let is_lts = entry
                                .get("lts")
                                .and_then(|v| v.as_str())
                                .is_some();
                            if is_lts {
                                if let Some(ver) = entry.get("version").and_then(|v| v.as_str()) {
                                    return Ok(ver.to_string());
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    anyhow::bail!("{}", T("cannot_determine_lts"))
}

fn get_latest_version(base_url: &str) -> Result<String> {
    let tags = get_tags(base_url.to_string());
    let re = regex::Regex::new(r"node-(v[\d.]+)-").unwrap();
    for tag in tags {
        if tag == "latest/" {
            let sub_tags = get_tags(format!("{}{}", base_url, tag));
            let suffix = os_suffix();
            for sub_tag in sub_tags {
                if sub_tag.ends_with(suffix) {
                    if let Some(caps) = re.captures(&sub_tag) {
                        return Ok(caps[1].to_string());
                    }
                }
            }
        }
    }
    anyhow::bail!("{}", T("cannot_determine_latest"))
}

fn get_download_url(version: &str, base_url: &str) -> Result<String> {
    let suffix = os_suffix();
    let version_dir = format!("{}{}/", base_url, version);

    let tags = get_tags(version_dir.clone());
    for tag in tags {
        if tag.ends_with(suffix) {
            return Ok(format!("{}{}", version_dir, tag));
        }
    }

    anyhow::bail!("{}", format_t("cannot_find_url", &[version.to_string()]))
}

/// Build source tarball URL: {base_url}/v{version}/node-v{version}.tar.gz
fn get_source_url(version: &str, base_url: &str) -> Result<String> {
    let url = format!("{}/v{}/node-v{}.tar.gz", base_url.trim_end_matches('/'), version, version);
    Ok(url)
}

/// Resolve an io.js version string (e.g., "iojs-3.3.1", "io.js-v2.5.0", "1.0.0")
/// Returns canonical "iojs-vX.Y.Z"
fn resolve_iojs_version(input: &str, iojs_base_url: &str) -> Result<String> {
    let mut ver = input.trim().to_lowercase();

    // Normalize prefix variations
    ver = ver.replace("io.js-", "iojs-").replace("io.js", "iojs");

    if !ver.starts_with("iojs") {
        ver = format!("iojs-v{}", ver);
    }
    if !ver.starts_with("iojs-v") {
        ver = ver.replace("iojs-", "iojs-v");
    }

    // If already fully specified (three parts), use as-is
    if ver.matches('.').count() >= 2 {
        return Ok(normalize_iojs_version(&ver));
    }

    // Partial version, fetch from remote
    let v_num = ver.trim_start_matches("iojs-v").trim_start_matches("iojs-");
    let tags = get_tags(format!("{}v{}/", iojs_base_url, v_num));
    if tags.is_empty() {
        anyhow::bail!("{}", format_t("no_iojs_match", &[input.to_string()]));
    }

    let suffix = os_suffix();
    let re = regex::Regex::new(r"iojs-(v[\d.]+)-").unwrap();
    for tag in tags.iter().rev() {
        if tag.ends_with(&suffix) {
            if let Some(caps) = re.captures(tag) {
                return Ok(format!("iojs-{}", &caps[1]));
            }
        }
    }

    anyhow::bail!("{}", format_t("cannot_resolve_iojs", &[input.to_string()]))
}

/// Build the download URL for an io.js binary tarball.
fn get_iojs_download_url(version: &str, iojs_base_url: &str) -> Result<String> {
    let ver_num = iojs_version_number(version).unwrap_or_else(|| version.to_string());
    let suffix = os_suffix();
    let version_dir = format!("{}v{}/", iojs_base_url, ver_num);
    let filename = format!("iojs-v{}-{}", ver_num, suffix);
    Ok(format!("{}{}", version_dir, filename))
}

/// Download a prebuilt npm tarball and install it into the version's lib/node_modules.
fn download_prebuilt_npm(version_dir: &Path, version: &str) -> Result<()> {
    let npm_tarball = format!("npm-v{}.tgz", version.trim_start_matches('v'));
    let npm_url = format!(
        "https://registry.npmjs.org/npm/-/npm-{}.tgz",
        version.trim_start_matches('v')
    );
    let npm_tar_path = get_nvm_dir().join(&npm_tarball);

    if !npm_tar_path.exists() {
        println!("  {} {}", "›".dimmed(), T("downloading_npm"));
        let client = crate::proxy::build_http_client();
        let response = client
            .get(&npm_url)
            .send()
            .context(T("npm_tarball_download_failed"))?;
        if !response.status().is_success() {
            anyhow::bail!("{}", format_t("npm_download_failed", &[npm_url.clone()]));
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
        std::io::copy(&mut src, &mut dest).context(T("npm_tarball_write_failed"))?;
        pb.finish_with_message(T("progress_done"));
    }

    // Extract npm tarball into lib/node_modules
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

    // Symlink bin
    let npm_bin_src = node_modules.join("bin").join("npm");
    let npm_bin_dst = version_dir.join("bin").join("npm");
    let npm_bin_dst_parent = version_dir.join("bin");
    std::fs::create_dir_all(&npm_bin_dst_parent).ok();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&npm_bin_src, &npm_bin_dst).ok();
    #[cfg(windows)]
    std::fs::copy(&npm_bin_src, &npm_bin_dst).ok();

    std::fs::remove_file(&npm_tar_path).ok();
    Ok(())
}

pub fn uninstall(version: &str) -> Result<()> {
    let resolved = resolve_alias(version)?;
    let nvm_dir = get_nvm_dir();
    let version_dir = nvm_dir.join(&resolved);

    if !version_dir.exists() {
        anyhow::bail!("{}", format_t("not_installed", &[resolved.clone()]));
    }

    let is_current_active = match get_current_version()? {
        Some(current) if current == resolved => {
            println!(
                "{} {}",
                "⚠".yellow().bold(),
                T("uninstall_warning").yellow()
            );
            true
        }
        _ => false,
    };

    print!("{} {} ", "▶".red().bold(), T("uninstalling_label").red().bold());
    print!("{}", resolved.white().bold());
    fs::remove_dir_all(&version_dir).context(T("uninstall_failed"))?;
    println!(" {}", "✓".green().bold());

    // Clear `current` if we just removed the active version, so subsequent
    // `nvm current` / `nvm ls` don't point at a deleted directory.
    if is_current_active {
        let current_file = nvm_dir.join("current");
        let _ = fs::remove_file(&current_file);
    }

    Ok(())
}

pub fn list_versions() -> Result<()> {
    let current = get_current_version()?;
    let mut versions = get_installed_versions();

    if versions.is_empty() {
        println!();
        println!("  {}  {}", "ℹ".cyan().bold(), T("no_installed_versions").cyan());
        println!("  {}  {}", "→".dimmed(), format_t("run_get_started", &["nvm install <version>".to_string()]).dimmed());
        println!();
        return Ok(());
    }

    // Sort descending (newest first) using semantic version comparison
    versions.sort_by(|a, b| compare_versions(b, a));

    let has_iojs = versions.iter().any(|v| v.starts_with("iojs-"));

    let columns: &[(&str, u8)] = &[
        (&T("version"), 0),
        (&T("type_col"), 0),
        (&T("lts_col"), 0),
        (&T("codename"), 0),
    ];

    let mut rows: Vec<Vec<String>> = Vec::new();
    for v in versions.iter() {
        let is_current = current.as_ref().map(|c| c == v).unwrap_or(false);
        let is_iojs = v.starts_with("iojs-");
        let version_str = if is_current {
            format!("{} {}", "●".green().bold(), v.green().bold())
        } else {
            format!("  {}", v.white())
        };
        let type_str = if is_iojs {
            "io.js".blue().to_string()
        } else {
            "node".dimmed().to_string()
        };
        let lts_str = if is_iojs {
            "-".dimmed().to_string()
        } else {
            version_lts_colored(v)
        };
        let codename_str = if is_iojs {
            "-".dimmed().to_string()
        } else {
            version_codename_colored(v)
        };

        rows.push(vec![version_str, type_str, lts_str, codename_str]);
    }

    let title = if has_iojs {
        T("installed_all_title")
    } else {
        T("installed_versions_title")
    };

    println!();
    render_table(&title, columns, &rows);
    println!();

    // Summary
    print!("  ");
    print!("{}", format_t("installed", &[versions.len().to_string()]).white().bold());
    print!("    ");
    if let Some(curr) = &current {
        print!("{}", T("active").replace("{0}", curr).green().bold());
    } else {
        print!("{}", T("no_active").dimmed());
    }
    println!();
    println!();

    Ok(())
}

pub fn remote_versions(lts_only: bool, lts_old: bool, filter: Option<&str>, sort: Option<&str>, page: Option<usize>) -> Result<()> {
    let config = load_config()?;
    let base_url = get_base_url(&config);

    print!("  {} {}", "⟳".cyan().bold(), T("fetching_remote").cyan());
    let tags = get_tags(base_url.to_string());
    println!(" {}", "✓".green().bold());

    let mut all_versions: Vec<(String, bool, String)> = Vec::new();

    for tag in tags {
        if tag.starts_with('v') && tag.ends_with('/') {
            let version = tag.trim_end_matches('/').to_string();
            let is_lts = is_lts_version(&version);
            let codename = get_codename(&version);
            all_versions.push((version, is_lts, codename));
        }
    }

    // Sort descending by semantic version (newest first) or ascending if requested
    let ascending = sort.map(|s| s.to_lowercase() == "asc").unwrap_or(false);
    if ascending {
        all_versions.sort_by(|a, b| compare_versions(&a.0, &b.0));
    } else {
        all_versions.sort_by(|a, b| compare_versions(&b.0, &a.0));
    }

    // Apply filters
    let filtered: Vec<(String, bool, String)> = {
        let mut result = if lts_only {
            all_versions.iter().filter(|(_, lts, _)| *lts).cloned().collect::<Vec<_>>()
        } else if lts_old {
            // Older LTS lines (major <= 18): v4 argon, v6 boron, v8 carbon,
            // v10 dubnium, v12 erbium, v14 fermium, v16 gallium, v18 hydrogen.
            all_versions
                .iter()
                .filter(|(_, lts, _)| *lts)
                .filter(|(v, _, _)| parse_major(v).is_some_and(|m| m <= 18))
                .cloned()
                .collect::<Vec<_>>()
        } else {
            all_versions.clone()
        };

        // Apply version filter if specified
        if let Some(f) = filter {
            let f_lower = f.to_lowercase();
            result.retain(|(v, _, _)| {
                v.to_lowercase().contains(&f_lower)
            });
        }

        result
    };

    if filtered.is_empty() {
        println!();
        println!("  {}  {}", "ℹ".cyan().bold(), T("no_versions_found").cyan());
        return Ok(());
    }

    let page_size = 20;
    let total_count = filtered.len();
    let total_pages = total_count.div_ceil(page_size);
    let page_num = page.unwrap_or(1).max(1).min(total_pages.max(1));
    let start = (page_num - 1) * page_size;
    let end = (start + page_size).min(total_count);

    let page_items = &filtered[start..end];

    // Build table data
    let columns: &[(&str, u8)] = &[
        ("#", 1),
        (&T("version"), 0),
        (&T("lts_col"), 0),
        (&T("codename"), 0),
    ];

    let mut rows: Vec<Vec<String>> = Vec::new();
    for (i, (v, _is_lts, _codename)) in page_items.iter().enumerate() {
        let idx = (start + i + 1).to_string();
        let idx_str = idx.dimmed().to_string();
        let version_str = v.white().to_string();
        let lts_str = version_lts_colored(v);
        let codename_str = version_codename_colored(v);
        rows.push(vec![idx_str, version_str, lts_str, codename_str]);
    }

    let title = if lts_only {
        T("remote_lts_title")
    } else if lts_old {
        T("remote_lts_old_title")
    } else {
        T("remote_title")
    };

    println!();
    render_table(&title, columns, &rows);
    println!();

    // Pagination info
    print!("  ");
    print!("{}", format_t("page_info", &[
        page_num.to_string(),
        total_pages.max(1).to_string(),
        (start + 1).to_string(),
        end.to_string(),
        total_count.to_string(),
    ]).cyan());
    println!();

    // Navigation hints
    let mut nav_parts: Vec<String> = Vec::new();
    if page_num > 1 {
        nav_parts.push(format_t("prev_page", &[(page_num - 1).to_string()]).yellow().to_string());
    }
    if page_num < total_pages {
        nav_parts.push(format_t("next_page", &[(page_num + 1).to_string()]).yellow().to_string());
    }
    if !nav_parts.is_empty() {
        println!("  {}", nav_parts.join("    "));
    }
    println!();

    Ok(())
}

pub fn use_version(version: Option<&str>, install_if_missing: bool, save: bool, use_on_cd: bool) -> Result<()> {
    use_version_silent(version, install_if_missing, save, use_on_cd, false)
}

/// Same as use_version but optionally suppresses all human-facing output.
/// Used by `nvm auto --silent` (the cd hook) so switching versions on every
/// directory change does not flood the terminal.
pub fn use_version_silent(
    version: Option<&str>,
    install_if_missing: bool,
    save: bool,
    use_on_cd: bool,
    silent: bool,
) -> Result<()> {
    // `nvm use` (no arg): fall back to .nvmrc / .node-version /
    // package.json#engines.node lookup, mirroring nvm-sh. If none of those
    // are found, surface a clear error rather than the clap "missing
    // required argument" usage message.
    //
    // Lookup priority matches nvm-sh user expectations: an explicit .nvmrc
    // wins over a package.json engines.node range, because .nvmrc is the
    // nvm-native config file and the user put it there on purpose. Only
    // when neither .nvmrc nor .node-version is present do we consult
    // package.json (a project-wide constraint, not a per-developer choice).
    let version = match version {
        Some(v) => v.to_string(),
        None => match find_nvmrc_recursive(silent)? {
            Some(v) => v,
            None => match find_package_json_node_version(silent)? {
                Some(v) => v,
                None => {
                    if !silent {
                        println!(
                            "{} {}",
                            "ℹ".cyan().bold(),
                            T("no_nvmrc_found").cyan()
                        );
                    }
                    anyhow::bail!("{}", T("specify_version"));
                }
            }
        },
    };
    let resolved = resolve_alias(&version)?;
    let nvm_dir = get_nvm_dir();

    if resolved.starts_with("system:") {
        let current_file = nvm_dir.join("current");
        fs::write(&current_file, &resolved).context(T("cannot_write_current"))?;
        if !silent {
            println!(
                "{} {} {}",
                "✓".green().bold(),
                T("now_using").green().bold(),
                T("system_node").white().bold()
            );
        }
        return Ok(());
    }

    let version_dir = nvm_dir.join(&resolved);
    if !version_dir.exists() {
        if install_if_missing {
            if !silent {
                println!(
                    "{} {} {}",
                    "ℹ".cyan().bold(),
                    T("version").cyan(),
                    format_t("version_not_installed_installing", &[resolved.clone()]).cyan()
                );
            }
            // Install the version
            install(Some(resolved.clone()), false, false, false, false, None, false, false, false, false, false)?;
            // Check if installation succeeded
            if !nvm_dir.join(&resolved).exists() {
                anyhow::bail!("{}", format_t("install_failed", &[resolved.clone()]));
            }
        } else {
            anyhow::bail!(
                "{}",
                format_t("not_installed_run_install", &[resolved.clone(), resolved.clone()])
            );
        }
    }

    let current_file = nvm_dir.join("current");
    fs::write(&current_file, &resolved).context(T("cannot_write_current"))?;

    // Determine if cd hook should be written: explicit --use-on-cd or config has it on
    let cd_hook = if use_on_cd {
        let mut config = load_config()?;
        config.use_on_cd = Some(true);
        save_config(&config)?;
        true
    } else {
        load_config()?.use_on_cd.unwrap_or(false)
    };

    update_shell_config(&resolved, cd_hook)?;

    // --save: persist this version as the default in config.json
    if save {
        let mut config = load_config()?;
        config.default_version = Some(resolved.clone());
        save_config(&config)?;
        if !silent {
            println!(
                "  {} {}",
                "✓".green().bold(),
                format_t("default_saved", &[resolved.clone()]).green()
            );
        }
    }

    if use_on_cd && !silent {
        println!(
            "  {} {}",
            "✓".green().bold(),
            T("use_on_cd_enabled").green()
        );
    }

    if !silent {
        // Use the correct product name in the success message: io.js versions
        // (iojs-vX.Y.Z / io.js-vX.Y.Z) should say "io.js" rather than
        // "Node.js" so the output matches what `nvm install` prints.
        let product_msg = if crate::utils::is_iojs_version(&resolved) {
            T("now_using_iojs")
        } else {
            T("now_using_node")
        };
        println!(
            "{} {} {}",
            "✓".green().bold(),
            product_msg.green().bold(),
            resolved.white().bold()
        );
        println!(
            "  {} {}",
            T("tip_label").dimmed(),
            T("tip_apply_shell").dimmed()
        );
    }

    Ok(())
}

pub fn current_version() -> Result<()> {
    match get_current_version()? {
        Some(version) => {
            let resolved = version;
            if resolved.starts_with("system:") {
                println!(
                    "{} {}",
                    "system".cyan().bold(),
                    format!("({})", resolved.trim_start_matches("system:")).dimmed()
                );
            } else {
                let nvm_dir = get_nvm_dir();
                let node_path = nvm_dir.join(&resolved).join("bin").join("node");

                println!("{}", resolved.green().bold());

                if let Ok(output) = Command::new(&node_path).arg("--version").output() {
                    let v = String::from_utf8_lossy(&output.stdout);
                    println!("  {} {}", T("node_label").dimmed(), v.trim().white());
                }
                if let Ok(output) = Command::new(nvm_dir.join(&resolved).join("bin").join("npm"))
                    .arg("--version")
                    .output()
                {
                    let v = String::from_utf8_lossy(&output.stdout);
                    println!("  {} {}", T("npm_label").dimmed(), v.trim().white());
                }
            }
        }
        None => println!(
            "{} {}",
            "✗".red().bold(),
            T("no_active_use").red()
        ),
    }

    Ok(())
}

pub fn deactivate() -> Result<()> {
    let nvm_dir = get_nvm_dir();
    let current_file = nvm_dir.join("current");
    if current_file.exists() {
        fs::remove_file(&current_file)?;
    }
    println!(
        "{} {}",
        "✓".green().bold(),
        T("deactivated").green()
    );
    Ok(())
}

pub fn unload() -> Result<()> {
    remove_from_shell_config()
}

// ---------------------------------------------------------------------------
// Cache management (P0-1)
// ---------------------------------------------------------------------------

pub fn cache_dir() -> Result<()> {
    let cache_dir = get_cache_dir();
    println!("{}", cache_dir.display().to_string().white().bold());
    Ok(())
}

pub fn cmd_dir() -> Result<()> {
    let nvm_dir = get_nvm_dir();
    let home = crate::system::get_home_dir();
    let dot_nvm = std::path::PathBuf::from(&home).join(".nvm.rust");

    println!("{}", T("nvm_dir_title").cyan().bold());
    println!("  {} {}", T("nvm_dir_path").white(), nvm_dir.display().to_string().green());
    println!();
    println!("{}", T("nvm_home_title").cyan().bold());
    println!("  {} {}", T("nvm_home_path").white(), dot_nvm.display().to_string().green());
    Ok(())
}

pub fn cache_list() -> Result<()> {
    use crate::download::list_cached_files;

    let files = list_cached_files()?;

    if files.is_empty() {
        println!();
        println!("  {}  {}", "ℹ".cyan().bold(), T("cache_empty").cyan());
        println!();
        return Ok(());
    }

    let total_bytes: u64 = files.iter().map(|(_, s)| s).sum();

    let columns: &[(&str, u8)] = &[
        (&T("file_col"), 0),
        (&T("size_col"), 1),
    ];

    let mut rows: Vec<Vec<String>> = Vec::new();
    for (name, size) in &files {
        rows.push(vec![name.clone(), format_size(*size)]);
    }

    println!();
    let cache_title = T("cache_title");
    render_table(&cache_title, columns, &rows);
    println!();
    print!("  ");
    print!("{}", format_t("cache_files", &[files.len().to_string()]).dimmed());
    print!("    ");
    print!("{}", T("cache_total").replace("{0}", &format_size(total_bytes)).cyan());
    println!();
    println!();

    Ok(())
}

pub fn cache_clear() -> Result<()> {
    use crate::download::clear_cache;

    let cleared = clear_cache()?;
    println!();
    println!(
        "  {} {}",
        "✓".green().bold(),
        format_t("cache_cleared", &[format_size(cleared)]).green()
    );
    println!();
    Ok(())
}

// ---------------------------------------------------------------------------
// Uninstall LTS (P3-7)
// ---------------------------------------------------------------------------

pub fn uninstall_latest_lts() -> Result<()> {
    // Pick the newest LTS that is *actually installed locally*, rather than the
    // latest LTS reported by the remote registry. This works offline and matches
    // user expectation: "uninstall the latest LTS I have".
    let versions = get_installed_versions();
    if versions.is_empty() {
        anyhow::bail!("{}", T("no_installed_versions"));
    }

    // Reuse the existing LTS detection (utils::is_lts_version) so the notion
    // of "LTS" stays consistent across the codebase.
    let mut lts: Vec<String> = versions
        .into_iter()
        .filter(|v| is_lts_version(v))
        .collect();
    if lts.is_empty() {
        anyhow::bail!("{}", T("no_installed_lts"));
    }
    lts.sort_by(|a, b| compare_versions(b, a));
    uninstall(&lts[0])
}

/// Uninstall the newest installed version (any kind, including io.js).
/// Mirrors `uninstall --lts` but without the LTS filter.
pub fn uninstall_latest() -> Result<()> {
    let versions = get_installed_versions();
    if versions.is_empty() {
        anyhow::bail!("{}", T("no_installed_versions"));
    }
    let mut all: Vec<String> = versions;
    all.sort_by(|a, b| compare_versions(b, a));
    uninstall(&all[0])
}

// ---------------------------------------------------------------------------
// Helper
// ---------------------------------------------------------------------------

fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

pub fn run_version(version: &str, args: &[String]) -> Result<()> {
    let resolved = resolve_alias(version)?;
    let nvm_dir = get_nvm_dir();

    let node_path = if resolved.starts_with("system:") {
        PathBuf::from("node")
    } else {
        nvm_dir.join(&resolved).join("bin").join("node")
    };

    if !resolved.starts_with("system:") && !node_path.exists() {
        anyhow::bail!("{}", format_t("not_installed", &[resolved.clone()]));
    }

    let status = Command::new(&node_path)
        .args(args)
        .status()
        .context(T("execution_failed"))?;

    std::process::exit(status.code().unwrap_or(1));
}

pub fn exec_version(version: &str, args: &[String]) -> Result<()> {
    if args.is_empty() {
        anyhow::bail!("{}", T("specify_command"));
    }

    let resolved = resolve_alias(version)?;
    let nvm_dir = get_nvm_dir();

    let bin_dir = if resolved.starts_with("system:") {
        if let Ok(output) = Command::new("which").arg("node").output() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if let Some(parent) = std::path::Path::new(&path).parent() {
                parent.to_path_buf()
            } else {
                anyhow::bail!("{}", T("system_node_not_found"));
            }
        } else {
            anyhow::bail!("{}", T("system_node_not_found"));
        }
    } else {
        // Verify the requested version is actually installed, so we never
        // silently fall back to a system node found later on PATH.
        let version_dir = nvm_dir.join(&resolved);
        if !version_dir.exists() {
            anyhow::bail!(
                "{}",
                format_t("not_installed_run_install", &[resolved.clone(), resolved.clone()])
            );
        }
        nvm_dir.join(&resolved).join("bin")
    };

    let cmd = &args[0];
    let cmd_args = &args[1..];

    let path = env::var("PATH").unwrap_or_default();
    let new_path = format!("{}:{}", bin_dir.display(), path);

    // `Command::new(cmd).status()` fails synchronously when `cmd` is not on
    // PATH (or is not an executable). The raw io::Error surfaces as
    // "No such file or directory (os error 2)", which is confusing because it
    // doesn't name the command the user typed. Detect that specific case and
    // bail with an i18n message that includes `cmd`.
    let status = Command::new(cmd)
        .args(cmd_args)
        .env("PATH", &new_path)
        .status()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                anyhow::anyhow!("{}", format_t("exec_command_not_found", &[cmd.clone()]))
            } else {
                anyhow::Error::new(e).context(T("execution_failed"))
            }
        })?;

    std::process::exit(status.code().unwrap_or(1));
}

pub fn which_version(version: Option<&str>) -> Result<()> {
    let resolved = match version {
        Some(v) => resolve_alias(v)?,
        None => match get_current_version()? {
            Some(v) => v,
            None => anyhow::bail!("{}", T("no_current_version_set")),
        },
    };

    if resolved.starts_with("system:") {
        if let Ok(output) = Command::new("which").arg("node").output() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path.is_empty() {
                println!("{}", path.white().bold());
                return Ok(());
            }
        }
        anyhow::bail!("{}", T("system_node_not_found"));
    }

    let nvm_dir = get_nvm_dir();
    let node_path = nvm_dir.join(&resolved).join("bin").join("node");

    if !node_path.exists() {
        anyhow::bail!("{}", format_t("not_installed", &[resolved.clone()]));
    }

    println!("{}", node_path.display().to_string().white().bold());
    Ok(())
}

pub fn auto_switch(silent: bool) -> Result<()> {
    // `nvm auto` is now an alias for `nvm use` (no arg): both look up the
    // version from .nvmrc / .node-version / package.json and switch. Keep
    // the explicit entry point so existing shell hooks (`nvm auto --silent`)
    // keep working.
    use_version_silent(None, false, false, false, silent)
}

/// Recursively search for .nvmrc or .node-version file from current directory up to root
fn find_nvmrc_recursive(silent: bool) -> Result<Option<String>> {
    let current_dir = std::env::current_dir()?;
    let mut dir = current_dir.as_path();

    // Read the first non-comment, non-empty line from a .nvmrc /
    // .node-version file. nvm-sh itself only reads the first line, but many
    // real-world .nvmrc files start with a `# comment` (editor templates,
    // per-project docs) — without this filter the comment text would be
    // passed to resolve_alias and produce a confusing error like
    // "Version v# comment\nv18.20.4 is not installed".
    let read_first_version_line = |path: &Path| -> Option<String> {
        let content = fs::read_to_string(path).ok()?;
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if trimmed.starts_with('#') {
                continue;
            }
            return Some(trimmed.to_string());
        }
        None
    };

    loop {
        let nvmrc = dir.join(".nvmrc");
        if nvmrc.exists() {
            if let Some(version) = read_first_version_line(&nvmrc) {
                if !silent {
                    println!(
                        "{} {} {}",
                        "ℹ".cyan().bold(),
                        T("found_nvmrc").cyan(),
                        dir.display().to_string().dimmed()
                    );
                }
                return Ok(Some(version));
            }
        }

        let node_version = dir.join(".node-version");
        if node_version.exists() {
            if let Some(version) = read_first_version_line(&node_version) {
                if !silent {
                    println!(
                        "{} {} {}",
                        "ℹ".cyan().bold(),
                        T("found_node_version").cyan(),
                        dir.display().to_string().dimmed()
                    );
                }
                return Ok(Some(version));
            }
        }

        // Move to parent directory
        dir = match dir.parent() {
            Some(parent) => parent,
            None => break,
        };

        // Stop at filesystem root
        if dir.parent().is_none() {
            break;
        }
    }

    Ok(None)
}

/// Find Node.js version from package.json engines.node field.
///
/// `engines.node` may be:
///   - a bare version:        `"22.0.0"` or `"v22.0.0"`
///   - a range expression:    `">=18.0.0"`, `"^20.11.0"`, `"~22.0.0"`,
///                            `"22.x"`, `"22 || 20"`, etc.
///   - the wildcard `"*"` / `"x"` / `""`  (no preference)
///
/// For ranges we pick the newest locally installed version that satisfies the
/// range. If none is installed we return the range expression itself verbatim,
/// so the caller can show a helpful "not installed, run nvm install <ver>"
/// message (matching the original behavior for bare versions).
fn find_package_json_node_version(silent: bool) -> Result<Option<String>> {
    let current_dir = std::env::current_dir()?;
    let package_json = current_dir.join("package.json");

    if !package_json.exists() {
        return Ok(None);
    }

    let content = fs::read_to_string(&package_json)?;
    // A malformed package.json shouldn't crash auto-detection — skip it and
    // fall through to the .nvmrc/.node-version lookup.
    let json: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return Ok(None),
    };

    let raw = match json
        .get("engines")
        .and_then(|e| e.get("node"))
        .and_then(|n| n.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    {
        Some(v) => v,
        None => return Ok(None),
    };

    // "lts/*", "lts", "node", "stable", "latest" — resolve as aliases against
    // the installed set: lts/* → newest LTS installed, node/stable/latest →
    // newest installed. Falls through to range parsing if not an alias.
    let installed = get_installed_versions();

    // Resolve alias-like expressions before range parsing so that "lts/*" /
    // "lts" don't get misinterpreted as version strings.
    let lower = raw.to_lowercase();
    if lower == "lts" || lower == "lts/*" || lower == "lts/-1" {
        let mut lts: Vec<String> = installed
            .iter()
            .filter(|v| is_lts_version(v))
            .cloned()
            .collect();
        lts.sort_by(|a, b| compare_versions(a, b));
        if let Some(chosen) = lts.last() {
            if !silent {
                println!(
                    "{} {} {} {}",
                    "ℹ".cyan().bold(),
                    T("found_engines_node").cyan(),
                    raw.white().bold(),
                    format!("→ {}", chosen).dimmed()
                );
            }
            return Ok(Some(chosen.clone()));
        }
        // No LTS installed — surface the alias so use_version reports it.
        return Ok(Some(raw));
    }
    if lower == "node" || lower == "stable" || lower == "latest" || lower == "*" || lower == "x" {
        if let Some(chosen) = installed
            .iter()
            .max_by(|a, b| compare_versions(a, b))
            .cloned()
        {
            if !silent {
                println!(
                    "{} {} {} {}",
                    "ℹ".cyan().bold(),
                    T("found_engines_node").cyan(),
                    raw.white().bold(),
                    format!("→ {}", chosen).dimmed()
                );
            }
            return Ok(Some(chosen));
        }
        return Ok(None);
    }

    // Try to satisfy as a range expression. This also handles bare versions
    // with wildcards ("22.x"), unions ("22 || 20"), compound (" >=20 <22 "),
    // caret/tilde, and operator-prefixed forms. If it resolves to an installed
    // version we return that; otherwise we fall back to the raw expression so
    // use_version prints the standard "not installed" hint.
    if let Some(chosen) = pick_version_for_range(&raw, &installed) {
        if !silent {
            println!(
                "{} {} {} {}",
                "ℹ".cyan().bold(),
                T("found_engines_node").cyan(),
                raw.white().bold(),
                format!("→ {}", chosen).dimmed()
            );
        }
        return Ok(Some(chosen));
    }

    // Plain bare version like "22.0.0" or "v22.0.0" — pass through verbatim.
    if raw.starts_with(|c: char| c.is_ascii_digit() || c == 'v') && !raw.contains(' ') {
        return Ok(Some(raw));
    }

    // Nothing installed satisfies the range and it isn't a bare version. Surface
    // the original constraint so the user sees what was requested.
    Ok(Some(raw))
}

/// Best-effort semver-ish range matcher. Supports `>=`, `>`, `<=`, `<`, `^`,
/// `~`, `x`/`*` wildcards, `||` unions, and space-separated compound ranges
/// (e.g. `>=20 <22` means both must hold). Picks the highest installed
/// version that satisfies the constraint.
fn pick_version_for_range(range: &str, installed: &[String]) -> Option<String> {
    if installed.is_empty() {
        return None;
    }

    // Union: "a || b"
    let ors: Vec<&str> = range.split("||").map(|s| s.trim()).collect();
    let mut candidates: Vec<String> = Vec::new();
    for part in &ors {
        // Within a union arm, space-separated tokens form an AND:
        // ">=20 <22" means both >=20 AND <22 must hold.
        let tokens: Vec<&str> = part.split_whitespace().collect();
        if tokens.is_empty() {
            continue;
        }
        if tokens.len() == 1 {
            if let Some(v) = pick_version_for_range_single(tokens[0], installed) {
                candidates.push(v);
            }
            continue;
        }
        // Compound AND: keep only installed versions satisfying every token.
        let mut matching: Vec<String> = installed
            .iter()
            .filter(|v| tokens.iter().all(|t| version_matches_simple(t, v)))
            .cloned()
            .collect();
        if !matching.is_empty() {
            matching.sort_by(|a, b| compare_versions(a, b));
            candidates.push(matching.pop().unwrap());
        }
    }
    candidates
        .into_iter()
        .max_by(|a, b| compare_versions(a, b))
}

/// Lightweight single-token matcher used by the compound AND branch above.
/// `token` is one of `>=`, `>`, `<=`, `<`, `^`, `~`, `=`, or a bare version.
fn version_matches_simple(token: &str, version: &str) -> bool {
    let (op, rest) = if let Some(r) = token.strip_prefix(">=") {
        (">=", r)
    } else if let Some(r) = token.strip_prefix("<=") {
        ("<=", r)
    } else if let Some(r) = token.strip_prefix('>') {
        (">", r)
    } else if let Some(r) = token.strip_prefix('<') {
        ("<", r)
    } else if let Some(r) = token.strip_prefix('=') {
        ("=", r)
    } else if let Some(r) = token.strip_prefix('^') {
        ("^", r)
    } else if let Some(r) = token.strip_prefix('~') {
        ("~", r)
    } else {
        ("=", token)
    };
    let rest = rest.trim().trim_start_matches('v');
    let comps: Vec<&str> = rest.split('.').collect();
    let wild = comps.iter().any(|c| *c == "x" || *c == "X" || *c == "*");
    version_matches_op(version, op, rest, wild)
}

fn pick_version_for_range_single(expr: &str, installed: &[String]) -> Option<String> {
    let expr = expr.trim();
    if expr.is_empty() {
        return None;
    }

    // Parse operator + remainder
    let (op, rest) = if let Some(r) = expr.strip_prefix(">=") {
        (">=", r.trim_start())
    } else if let Some(r) = expr.strip_prefix("<=") {
        ("<=", r.trim_start())
    } else if let Some(r) = expr.strip_prefix('>') {
        (">", r.trim_start())
    } else if let Some(r) = expr.strip_prefix('<') {
        ("<", r.trim_start())
    } else if let Some(r) = expr.strip_prefix('=') {
        ("=", r.trim_start())
    } else if let Some(r) = expr.strip_prefix('^') {
        ("^", r.trim_start())
    } else if let Some(r) = expr.strip_prefix('~') {
        ("~", r.trim_start())
    } else {
        ("=", expr)
    };

    let rest = rest.trim().trim_start_matches('v').to_string();
    if rest.is_empty() || rest == "*" || rest == "x" || rest == "X" {
        // Match any — pick newest installed
        return installed
            .iter()
            .max_by(|a, b| compare_versions(a, b))
            .cloned();
    }

    // Detect wildcard in major.minor.patch, e.g. "22.x", "22.*", "20.11.x"
    let comps: Vec<&str> = rest.split('.').collect();
    let wild = comps.iter().any(|c| *c == "x" || *c == "X" || *c == "*");

    // A bare major like "22" (no dots) is shorthand for "22.x.x" — treat as
    // wildcard so `22 || 20` matches any installed 22.x or 20.x.
    let effective_wild = wild || (!rest.contains('.') && op == "=");
    let effective_rest = if effective_wild && !rest.contains('.') && op == "=" {
        format!("{}.x", rest)
    } else {
        rest
    };

    let mut matching: Vec<String> = installed
        .iter()
        .filter(|v| version_matches_op(v, op, &effective_rest, effective_wild))
        .cloned()
        .collect();

    if matching.is_empty() {
        return None;
    }
    matching.sort_by(|a, b| compare_versions(a, b));
    matching.pop() // newest
}

fn parse_v_tuple(v: &str) -> Option<(u64, u64, u64)> {
    let s = v.trim_start_matches("iojs-v").trim_start_matches("iojs-").trim_start_matches('v');
    let parts: Vec<&str> = s.split('-').next().unwrap_or("").split('.').collect();
    Some((
        parts.first().and_then(|s| s.parse().ok())?,
        parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0),
        parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(0),
    ))
}

fn version_matches_op(version: &str, op: &str, target: &str, wildcard: bool) -> bool {
    let (maj, min, pat) = match parse_v_tuple(version) {
        Some(t) => t,
        None => return false,
    };
    let comps: Vec<&str> = target.split('.').collect();
    let t_maj: u64 = comps.first().and_then(|s| s.parse().ok()).unwrap_or(0);
    let t_min: u64 = comps.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    let t_pat: u64 = comps.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);

    match op {
        ">=" => maj > t_maj || (maj == t_maj && (min > t_min || (min == t_min && pat >= t_pat))),
        ">" => maj > t_maj || (maj == t_maj && (min > t_min || (min == t_min && pat > t_pat))),
        "<=" => maj < t_maj || (maj == t_maj && (min < t_min || (min == t_min && pat <= t_pat))),
        "<" => maj < t_maj || (maj == t_maj && (min < t_min || (min == t_min && pat < t_pat))),
        "^" => {
            // Compatible with: same major, >= target
            if maj != t_maj {
                return false;
            }
            (min, pat) >= (t_min, t_pat)
        }
        "~" => {
            // Same major.minor, >= target patch
            if maj != t_maj || min != t_min {
                return false;
            }
            pat >= t_pat
        }
        _ => {
            // "=" — exact, or wildcard match
            if wildcard {
                if comps.first().map(|s| *s == "x" || *s == "X" || *s == "*").unwrap_or(true) {
                    return false; // shouldn't happen — handled above
                }
                if maj != t_maj {
                    return false;
                }
                if comps.len() > 1 {
                    let m = comps[1];
                    if !(m == "x" || m == "X" || m == "*") {
                        let m: u64 = m.parse().unwrap_or(0);
                        if min != m {
                            return false;
                        }
                    }
                }
                if comps.len() > 2 {
                    let p = comps[2];
                    if !(p == "x" || p == "X" || p == "*") {
                        let p: u64 = p.parse().unwrap_or(0);
                        if pat != p {
                            return false;
                        }
                    }
                }
                true
            } else {
                (maj, min, pat) == (t_maj, t_min, t_pat)
            }
        }
    }
}

fn get_installed_version(version: &str) -> Result<String> {
    let resolved = resolve_alias(version)?;
    if resolved.starts_with("system:") {
        return Ok(resolved);
    }
    let nvm_dir = get_nvm_dir();
    let version_dir = nvm_dir.join(&resolved);
    if !version_dir.exists() {
        anyhow::bail!("{}", format_t("not_installed", &[resolved.clone()]));
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
        None => match get_current_version()? {
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
        anyhow::bail!("{}", format_t("source_not_installed", &[resolved_from.clone()]));
    }
    let current = get_current_version()?
        .ok_or_else(|| anyhow::anyhow!("{}", T("no_current_version_set")))?;
    reinstall_packages_inner(&resolved_from, &current)
}

pub fn show_version_info() -> Result<()> {
    match get_current_version()? {
        Some(v) if v.starts_with("system:") => {
            println!(
                "{} {}",
                T("system_node_label").cyan().bold(),
                v.trim_start_matches("system:").white()
            );
        }
        Some(v) => {
            let nvm_dir = get_nvm_dir();
            let bin = nvm_dir.join(&v).join("bin");
            let node_bin = bin.join("node");
            println!(
                "{} {}",
                T("active_node_label").green().bold(),
                v.white().bold()
            );

            // LTS badge + codename (cheap, no spawn).
            if is_lts_version(&v) {
                let codename = get_codename(&v);
                let codename_str = if codename == "-" {
                    String::new()
                } else {
                    format!("  {} {}", T("version_codename_label").dimmed(), codename.magenta().bold())
                };
                println!("  {}{}", T("lts_badge").green(), codename_str);
            }

            // Single node invocation to get node + npm + yarn + pnpm versions.
            // Each tool is probed via require.resolve: if the package is
            // installed globally (in node_modules), resolve returns its path
            // and we read the version from require().version; otherwise we
            // emit "none" so the caller can show an install hint.
            let probe_script = concat!(
                "(",
                "function(){",
                "function v(name){",
                "try{",
                "var p=require.resolve(name+'/package.json');",
                "return require(p).version||'none';",
                "}catch(e){return 'none'}",
                "}",
                "return [process.version,",
                "(process.versions.npm||'none'),",
                "v('yarn'),v('pnpm')].join('|')",
                "}()",
                ")"
            );
            if let Ok(out) = Command::new(&node_bin)
                .arg("-e")
                .arg(probe_script)
                .output()
            {
                let line = String::from_utf8_lossy(&out.stdout).trim().to_string();
                let parts: Vec<&str> = line.split('|').collect();
                if parts.len() == 4 {
                    // node
                    println!("  {} {}", T("node_label").dimmed(), parts[0].white());
                    // npm
                    if parts[1] != "none" {
                        println!("  {} {}", T("npm_label").dimmed(), parts[1].white());
                    }
                    // yarn
                    if parts[2] != "none" {
                        println!("  {} {}", T("yarn_label").dimmed(), parts[2].white());
                    } else {
                        println!("  {} {} {}",
                            T("yarn_label").dimmed(),
                            T("version_not_installed").yellow(),
                            T("version_install_hint_yarn").dimmed());
                    }
                    // pnpm
                    if parts[3] != "none" {
                        println!("  {} {}", T("pnpm_label").dimmed(), parts[3].white());
                    } else {
                        println!("  {} {} {}",
                            T("pnpm_label").dimmed(),
                            T("version_not_installed").yellow(),
                            T("version_install_hint_pnpm").dimmed());
                    }
                }
            }

            // Binary path (reuse which-style output, no extra spawn).
            if node_bin.exists() {
                println!(
                    "  {} {}",
                    T("version_path_label").dimmed(),
                    node_bin.display().to_string().white()
                );
            }
        }
        None => println!(
            "{} {}",
            "✗".red().bold(),
            T("no_active_version_set").red()
        ),
    }
    Ok(())
}

pub fn show_remote_version_info() -> Result<()> {
    let config = load_config()?;
    let base_url = get_base_url(&config);
    let tags = get_tags(base_url.to_string());

    let mut versions: Vec<String> = Vec::new();
    for tag in tags {
        if tag.starts_with("v") && tag.ends_with('/') {
            versions.push(tag.trim_end_matches('/').to_string());
        }
    }
    versions.sort_by(|a, b| compare_versions(b, a));

    println!();
    print!("  ");
    print!("{}", T("latest_remote_versions").cyan().bold());
    print!("  ");
    print!("{}", format_t("remote_total_count", &[versions.len().to_string()]).dimmed());
    println!();

    for v in versions.iter().take(5) {
        let is_lts = is_lts_version(v);
        let lts_mark = if is_lts {
            format!("  {} ", T("lts_badge").green())
        } else {
            "       ".to_string()
        };
        let codename = get_codename(v);
        let codename_str = if codename == "-" {
            "".to_string()
        } else {
            format!("  {}", codename.magenta())
        };
        println!("    {}  {}{}{}", "│".dimmed(), v.white().bold(), lts_mark, codename_str);
    }
    println!();

    Ok(())
}

pub fn cmd_set_alias(name: &str, version: Option<&str>) -> Result<()> {
    set_alias(name, version)
}

pub fn cmd_remove_alias(name: &str) -> Result<()> {
    remove_alias(name)
}

pub fn cmd_list_aliases() -> Result<()> {
    list_all_aliases()
}

pub fn cmd_mirror(mirror: Option<&str>) -> Result<()> {
    handle_mirror(mirror)
}

// ---------------------------------------------------------------------------
// Language / i18n
// ---------------------------------------------------------------------------

pub fn cmd_language(lang: Option<&str>) -> Result<()> {
    use crate::i18n::{available_lang_codes, get_language, set_language, Lang};

    // Build the `<en|cn|jp>` hint and the `en, cn, jp` list once; both are
    // rendered from LANG_CODES so they stay in sync with whatever locale
    // files are present at build time.
    let codes = available_lang_codes();
    let usage_hint = codes.join("|");
    let available_list = codes.join(", ");

    match lang {
        Some(l) => {
            if let Some(parsed) = Lang::from_str(l) {
                set_language(parsed)?;
                println!(
                    "  {} {} {}",
                    "✓".green().bold(),
                    T("language_set_label").green(),
                    parsed.display_name().white().bold()
                );
            } else {
                anyhow::bail!(
                    "{}",
                    format_t("lang_unknown", &[l.to_string(), available_list.to_string()])
                );
            }
        }
        None => {
            let current = get_language();
            println!();
            println!(
                "  {} {} {}",
                "▶".cyan().bold(),
                T("current_language_label").cyan(),
                current.display_name().white().bold()
            );
            println!(
                "  {} {}",
                "→".dimmed(),
                format_t("lang_usage", &[usage_hint.to_string()]).dimmed()
            );
            println!();
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Proxy management
// ---------------------------------------------------------------------------

pub fn cmd_proxy(action: Option<&str>) -> Result<()> {
    use crate::proxy::{get_system_proxy, proxy_status, set_proxy_enabled, test_connectivity};

    match action {
        Some("on") => {
            let sys_proxy = get_system_proxy();
            if sys_proxy.is_none() {
                println!();
                println!("  {}  {}", "⚠".yellow().bold(), T("proxy_no_system_proxy").yellow());
                println!("  {}  {}", "→".dimmed(), T("proxy_set_env_vars").dimmed());
                println!();
                return Ok(());
            }

            // Enable proxy first, so the connectivity test routes through it.
            set_proxy_enabled(true)?;

            // Test connectivity via the now-enabled proxy.
            println!("  {} {}", "›".dimmed(), T("testing_connectivity"));
            let (baidu_ok, google_ok) = test_connectivity();

            if baidu_ok || google_ok {
                println!();
                println!(
                    "  {} {} {}",
                    "✓".green().bold(),
                    T("proxy_enabled").green(),
                    T("proxy_will_be_used").green()
                );
                print!("    ");
                if baidu_ok {
                    print!("{}  ", T("proxy_test_baidu_ok").green());
                } else {
                    print!("{}  ", T("proxy_test_baidu_fail").red());
                }
                if google_ok {
                    println!("{}", T("proxy_test_google_ok").green());
                } else {
                    println!("{}", T("proxy_test_google_fail").red());
                }
                println!();
            } else {
                // Proxy did not work; roll back so downloads do not hang.
                set_proxy_enabled(false)?;
                println!();
                println!(
                    "  {}  {}",
                    "⚠".yellow().bold(),
                    T("neither_reachable").yellow()
                );
                println!(
                    "  {}  {}",
                    "→".dimmed(),
                    T("check_proxy_settings").dimmed()
                );
                println!();
            }
        }
        Some("off") => {
            set_proxy_enabled(false)?;
            println!();
            println!(
                "  {} {}",
                "✓".green().bold(),
                T("proxy_disabled").green()
            );
            println!();
        }
        Some(other) => {
            anyhow::bail!("{}", format_t("unknown_action", &[other.to_string()]));
        }
        None => {
            let status = proxy_status();
            let sys_proxy = status.system_proxy.clone();

            println!();
            println!("  {}", T("proxy_status_title").cyan().bold());
            println!();

            // NVM proxy toggle
            let nvm_state = if status.nvm_proxy_enabled {
                T("proxy_state_on").green().bold().to_string()
            } else {
                T("proxy_state_off").red().bold().to_string()
            };
            println!(
                "    {} {}{}",
                "nvm:".dimmed(),
                " ".repeat(10 - "nvm:".len()),
                nvm_state
            );

            // System proxy env
            let sys_state = match &sys_proxy {
                Some(p) => format!("{}", p.as_str().dimmed()),
                None => T("not_set").red().to_string(),
            };
            println!(
                "    {} {}{}",
                "system:".dimmed(),
                " ".repeat(10 - "system:".len()),
                sys_state
            );

            println!();

            if status.nvm_proxy_enabled {
                if sys_proxy.is_some() {
                    println!(
                        "  {} {}",
                        "✓".green().bold(),
                        T("proxy_active").green()
                    );
                } else {
                    println!(
                        "  {} {}",
                        "⚠".yellow().bold(),
                        T("proxy_on_no_env").yellow()
                    );
                }
            } else {
                println!(
                    "  {} {}",
                    "ℹ".cyan().bold(),
                    T("proxy_off_direct").cyan()
                );
            }

            println!();
            println!(
                "  {} {}",
                T("usage_label").dimmed(),
                T("proxy_usage_hint").yellow().bold()
            );
            println!();
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Migration from nvm-sh / nvm-windows
// ---------------------------------------------------------------------------

/// Locate the source versions directory for a given migration source.
fn resolve_migration_source(source: &str) -> Option<PathBuf> {
    let home = crate::system::get_home_dir();
    match source.to_lowercase().as_str() {
        "nvm" | "nvm-sh" | "nvm_sh" => {
            // nvm-sh stores versions under ~/.nvm/versions/node/. We deliberately do NOT
            // honor NVM_DIR here because that variable is also what nvm-rust itself uses
            // for its own install dir — honoring it would make source and destination
            // point at the same place and silently skip everything.
            // The legacy NVM_SH_HOME override lets tests (and advanced users) point at
            // a non-default nvm-sh install location.
            let nvm_sh_root = env::var("NVM_SH_HOME").unwrap_or_else(|_| home.clone());
            let versions_dir = PathBuf::from(&nvm_sh_root).join(".nvm").join("versions").join("node");
            if versions_dir.is_dir() {
                Some(versions_dir)
            } else {
                None
            }
        }
        "nvm-windows" | "nvm_windows" | "nvmwindows" => {
            // nvm-windows stores versions under $NVM_HOME or $NVM_SYMLINK root.
            // These env vars are nvm-windows specific and do not conflict with nvm-rust.
            let root = env::var("NVM_HOME")
                .or_else(|_| env::var("NVM_SYMLINK"))
                .unwrap_or_else(|_| format!("{}\\nvm4w", home));
            let p = PathBuf::from(&root);
            if p.is_dir() {
                Some(p)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Copy a source version directory into the nvm-rust store.
/// Returns true if the version was newly imported, false if already present.
///
/// Always performs a deep copy (never a symlink). A symlinked import would
/// dangle as soon as the user removes the source tree — e.g. after running
/// `rm -rf ~/.nvm` to clean up nvm-sh, every imported version would turn
/// into a broken symlink and `nvm use <version>` would fail with
/// "No such file or directory" instead of the expected "version not
/// installed". Copying makes the import self-contained, matching nvm-sh's
/// own `nvm install` semantics where the version lives entirely under
/// `NVM_DIR`.
fn import_version(src: &Path, dest: &Path) -> Result<bool> {
    if dest.exists() {
        return Ok(false);
    }

    copy_dir_recursive(src, dest).context(T("copy_version_dir_failed"))?;
    Ok(true)
}

/// Recursively copy a directory tree. Used when symlink is not permitted.
fn copy_dir_recursive(src: &Path, dest: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dest)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        let target = dest.join(&name);
        if path.is_dir() {
            copy_dir_recursive(&path, &target)?;
        } else {
            fs::copy(&path, &target)?;
        }
    }
    Ok(())
}

/// Migrate installed Node.js versions from nvm-sh or nvm-windows.
///
/// Versions are deep-copied into the nvm-rust store (see `import_version`) so
/// the import is self-contained and survives deletion of the source tree.
/// Already-present versions are skipped. The `default` alias from nvm-sh is
/// also carried over when present.
pub fn cmd_migrate(source: &str) -> Result<()> {
    let src_dir = resolve_migration_source(source).ok_or_else(|| {
        anyhow::anyhow!("{}", format_t("migrate_source_not_found", &[source.to_string()]))
    })?;

    // Compute the nvm-sh install root (the dir that *contains* `.nvm/`),
    // mirroring the logic in `resolve_migration_source` for "nvm"/"nvm-sh".
    // `detect_nvm_sh_default` needs the same root it used to find versions,
    // otherwise it would read the alias from `NVM_DIR` (which is nvm-rust's
    // own store, not the nvm-sh install we just migrated from).
    let nvm_sh_root: Option<PathBuf> = match source.to_lowercase().as_str() {
        "nvm" | "nvm-sh" | "nvm_sh" => {
            let home = crate::system::get_home_dir();
            Some(PathBuf::from(env::var("NVM_SH_HOME").unwrap_or(home)))
        }
        _ => None, // nvm-windows: no `~/.nvm/alias/default` concept.
    };

    let nvm_dir = get_nvm_dir();
    ensure_nvm_dir_or_fail()?;

    println!();
    println!(
        "  {} {}",
        "▶".cyan().bold(),
        format_t("migrate_scanning", &[src_dir.display().to_string()]).cyan()
    );
    println!();

    let mut imported = 0usize;
    let mut skipped = 0usize;

    // Enumerate version directories. nvm-sh uses "vX.Y.Z", nvm-windows too.
    let mut entries: Vec<PathBuf> = Vec::new();
    if let Ok(rd) = fs::read_dir(&src_dir) {
        for entry in rd.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if name.starts_with('v') || name.starts_with("iojs-") {
                        entries.push(path);
                    }
                }
            }
        }
    }
    entries.sort();

    if entries.is_empty() {
        println!(
            "  {} {}",
            "⚠".yellow().bold(),
            T("migrate_no_versions").yellow()
        );
        println!();
        return Ok(());
    }

    for src_version_dir in &entries {
        let version_name = src_version_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");
        if version_name.is_empty() {
            continue;
        }
        let dest_version_dir = nvm_dir.join(version_name);

        match import_version(src_version_dir, &dest_version_dir) {
            Ok(true) => {
                println!(
                    "  {} {}",
                    "✓".green().bold(),
                    format_t("migrate_imported", &[version_name.to_string()]).green()
                );
                imported += 1;
            }
            Ok(false) => {
                println!(
                    "  {} {}",
                    "·".dimmed(),
                    format_t("migrate_skipped", &[version_name.to_string()]).dimmed()
                );
                skipped += 1;
            }
            Err(e) => {
                println!(
                    "  {} {} ({})",
                    "✗".red().bold(),
                    format_t("migrate_failed", &[version_name.to_string()]).red(),
                    e
                );
            }
        }
    }

    // Carry over the `default` alias from nvm-sh if we touched anything at
    // all (imported OR skipped — already-imported versions still mean the
    // nvm-sh tree was found and is the source of truth for "default").
    if imported + skipped > 0 {
        if let Some(root) = nvm_sh_root {
            if let Some(default_ver) = detect_nvm_sh_default(&root) {
                let dest = nvm_dir.join(&default_ver);
                if dest.exists() {
                    let mut config = load_config()?;
                    config.default_version = Some(default_ver.clone());
                    save_config(&config)?;
                    // Also overwrite `aliases.aliases["default"]` if it exists:
                    // `resolve_alias("default")` checks user-defined aliases
                    // FIRST and only falls back to `config.default_version`,
                    // so writing config alone is not enough — a pre-existing
                    // `default` alias (e.g. from an earlier `nvm alias default
                    // lts`) would shadow the migrated value and `nvm use
                    // default` would resolve to the old alias instead.
                    if let Ok(mut aliases) = crate::config::load_aliases() {
                        if aliases.aliases.contains_key("default") {
                            aliases.aliases.insert("default".to_string(), default_ver.clone());
                            let _ = crate::config::save_aliases(&aliases);
                        }
                    }
                    println!();
                    println!(
                        "  {} {}",
                        "✓".green().bold(),
                        format_t("migrate_default_set", &[default_ver]).green()
                    );
                }
            }
        }
    }

    println!();
    println!(
        "  {} {}",
        "✓".green().bold(),
        format_t("migrate_summary", &[imported.to_string(), skipped.to_string()]).green()
    );
    println!();
    Ok(())
}

/// Read the nvm-sh default alias from <nvm_sh_root>/.nvm/alias/default.
/// `nvm_sh_root` is the directory that contains the `.nvm` subdir of the
/// nvm-sh install being migrated from (i.e. the same root
/// `resolve_migration_source` derives its `versions/node` path from). We MUST
/// NOT consult `NVM_DIR` here: that variable is what nvm-rust itself uses for
/// its own store, and could point at a different location than the nvm-sh
/// install we just migrated from — reading the alias from there would either
/// find a stale/empty file or the wrong default.
fn detect_nvm_sh_default(nvm_sh_root: &Path) -> Option<String> {
    let default_file = nvm_sh_root.join(".nvm").join("alias").join("default");
    let raw = fs::read_to_string(&default_file).ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    // Already a fully-qualified version: keep as-is.
    if trimmed.starts_with('v') || trimmed.starts_with("iojs-") {
        return Some(trimmed.to_string());
    }
    // Full version without "v" prefix (e.g. "20.11.0", "22.5.1"): add prefix.
    // We detect "full" as exactly two dots among digits.
    let dots = trimmed.matches('.').count();
    if dots == 2 && trimmed.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return Some(format!("v{}", trimmed));
    }
    // Bare major ("20"), bare major.minor ("20.5"), "node", "stable", etc.
    // — resolve against the SOURCE nvm-sh install so "20" maps to the latest
    // v20.x.y that nvm-sh actually has installed.
    let versions_root = nvm_sh_root.join(".nvm").join("versions").join("node");
    let mut candidates: Vec<String> = Vec::new();
    if let Ok(rd) = fs::read_dir(&versions_root) {
        for entry in rd.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                if name.starts_with('v') {
                    candidates.push(name.to_string());
                }
            }
        }
    }
    // For a bare major like "20", restrict to matching "v20.*". For generic
    // aliases ("node", "stable") we take the latest of everything.
    if let Some(major) = trimmed
        .split('.')
        .next()
        .and_then(|s| s.parse::<u32>().ok())
    {
        let prefix = format!("v{}.", major);
        candidates.retain(|v| v.starts_with(&prefix));
    }
    // Sort semantically — alphabetical sort would put `v20.5.0` after
    // `v20.20.2` ('5' > '2') and pick the older version as "latest".
    candidates.sort_by(|a, b| crate::utils::compare_semver(a, b));
    candidates.last().cloned()
}

fn ensure_nvm_dir_or_fail() -> Result<()> {
    let nvm_dir = get_nvm_dir();
    if !nvm_dir.exists() {
        fs::create_dir_all(&nvm_dir).context(T("cannot_create_nvm_dir"))?;
    }
    Ok(())
}
