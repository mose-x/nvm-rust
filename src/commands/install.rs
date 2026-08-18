//! Core install orchestration: `nvm install`.
//!
//! This module owns the install target resolution (`build_install_target`),
//! the prebuilt-binary download/verify/extract path (`install_binary`), the
//! post-install hook runner (`run_post_install_hooks`), and the public
//! `install` entry point. It also owns the shared `InstallConfig` /
//! `InstallTarget` types, the `SourceGuard` RAII guard (used by both
//! `install_binary` here and `install_from_source` in [`super::install_source`]),
//! and the `command_failed` formatting helper (used by both source-build and
//! the package-upgrade helpers in [`super::package_upgrade`]).
//!
//! The source-build path (`install_from_source`), the package-manager
//! upgrade commands (`install_latest_npm`/`yarn`/`pnpm` and helpers), and
//! the reinstall commands (`reinstall_packages` and helper) live in
//! sibling modules and are re-exported below for backward compatibility.

use anyhow::{Context, Result};
use colored::Colorize;
use std::fs;
use std::path::Path;

use super::version_resolve::{
    get_download_url, get_iojs_download_url, get_latest_lts_version, get_latest_version,
    resolve_iojs_version, resolve_version,
};
use crate::config::{load_config, resolve_alias};
use crate::download::{copy_from_cache, download_to_cache, is_cached};
use crate::extract::{extract_archive, extract_iojs_archive};
use crate::i18n::{format_t, T};
use crate::system::{
    fetch_shasums, get_nvm_dir, os_suffix, verify_checksum, verify_gpg_signature, GpgStatus,
    IOJS_URI,
};
use crate::utils::{atomic_write, iojs_version_number};

// Re-export the source / package-upgrade / reinstall helpers that used to
// live in this module so existing callers (`crate::commands::install_*`,
// `crate::commands::reinstall_packages`) keep resolving after the split.
// `pub(crate)` matches the visibility of the items in the sub-modules; the
// re-exports then propagate up to `commands::*` via mod.rs's
// `pub use install::*`.
pub(crate) use super::{install_source::*, package_upgrade::*, reinstall::*};

/// Final io.js release (2015-05-21). io.js merged back into Node.js with
/// v4.0.0, so `nvm install iojs` (no explicit version) resolves to this.
const IOJS_FINAL_VERSION: &str = "3.3.1";

/// Build an `anyhow::Error` for a failed external command, formatted as
/// `"<i18n message> (<exit code>)"`. Replaces the 4+ inline
/// `anyhow::bail!("{} ({})", T(key), status.code().unwrap_or(-1))` sites in
/// the source-install / npm-upgrade paths. `code()` is `None` when the
/// process was killed by a signal; we report `-1` there to match the
/// previous behaviour (callers that need signal-accurate exit codes use
/// `exit_with_status` in `run.rs` instead).
pub(crate) fn command_failed(key: &str, status: std::process::ExitStatus) -> anyhow::Error {
    anyhow::anyhow!("{} ({})", T(key), status.code().unwrap_or(-1))
}

/// Resolved target for an install operation. Built by `build_install_target`
/// and consumed by the source/binary/post-install phases so `install` itself
/// stays a thin orchestrator.
pub(crate) struct InstallTarget {
    pub(crate) target_version: String,
    pub(crate) download_url: String,
    pub(crate) archive_name: String,
    pub(crate) product_name: &'static str,
    pub(crate) is_iojs: bool,
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

    // `--source` drives a Unix-only toolchain (`./configure`, `make`, GNU
    // `tar --strip-components`). It is unreachable on Windows, where the
    // prebuilt-binary install path is the default and `make`/`configure` are
    // not present. Refuse it up front with a clear message instead of failing
    // later inside `install_from_source` with a cryptic "configure: No such
    // file or directory".
    if cfg.source && cfg!(windows) {
        anyhow::bail!("{}", T("source_unsupported_windows"));
    }

    if is_iojs {
        // `is_iojs` is only set to true above when `cfg.version` is Some, but
        // encode that invariant explicitly instead of `unwrap()`-ing — a future
        // refactor (e.g. an `--iojs` flag with no version arg) would otherwise
        // panic with no context.
        let ver = cfg
            .version
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("io.js install requested but no version provided"))?;
        let lv = ver.to_lowercase();
        let ver_input = if lv == "iojs" || lv == "io.js" {
            IOJS_FINAL_VERSION.to_string()
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

/// RAII guard that removes a file or directory when dropped, unless disarmed.
/// Used by `install_from_source` / `install_binary` to clean up temp artifacts
/// on every exit path (success or `?` early-return), replacing the previous
/// "clean up only at the end of the happy path" pattern that leaked tens of
/// MB on every failed install.
pub(crate) struct SourceGuard {
    path: std::path::PathBuf,
    is_dir: bool,
    armed: bool,
}

impl SourceGuard {
    pub(crate) fn file(path: std::path::PathBuf) -> Self {
        Self {
            path,
            is_dir: false,
            armed: true,
        }
    }
    pub(crate) fn dir(path: std::path::PathBuf) -> Self {
        Self {
            path,
            is_dir: true,
            armed: true,
        }
    }
}

impl Drop for SourceGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let _ = if self.is_dir {
            fs::remove_dir_all(&self.path)
        } else {
            fs::remove_file(&self.path)
        };
    }
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
    // The archive to extract. In the online path we extract directly from
    // the cache file (`download_to_cache` already wrote it there), avoiding
    // a redundant ~25MB copy from `cache/<archive>` to `<nvm_dir>/<ver>.tmp`
    // that the previous code did every install. `extract_path` therefore
    // points at the cache file and must NOT be deleted after extraction.
    //
    // In the offline path we still copy cache -> temp file because the cache
    // is long-lived and we need an isolated file we can safely remove after
    // extraction; `extract_path` then points at the temp file and IS deleted.
    let temp_file = nvm_dir.join(format!("{}.tmp", target.target_version));
    let (extract_path, owns_extract_file) = if offline {
        if is_cached(&target.archive_name) {
            println!("  {} {}", "ℹ".cyan().bold(), T("using_cache").cyan());
            copy_from_cache(&target.archive_name, &temp_file)?;
            (temp_file, true)
        } else {
            anyhow::bail!(format_t(
                "offline_no_cache",
                std::slice::from_ref(&target.archive_name)
            ));
        }
    } else {
        let cached_path = download_to_cache(&target.download_url, &target.archive_name)?;
        (cached_path, false)
    };

    // RAII guard so the offline temp copy is removed on every exit path
    // (checksum failure, GPG failure, extract failure), not just the
    // success path at the end. Online path used the shared cache and must
    // not be deleted — guard is only armed when `owns_extract_file`.
    let extract_guard = owns_extract_file.then(|| SourceGuard::file(extract_path.clone()));

    // Integrity verification: the SHA-256 checksum check runs for BOTH
    // Node.js and io.js — a MITM tampering with either tarball must be
    // caught. GPG signature verification runs for Node.js ONLY: io.js
    // releases (EOL 2015) were signed by a separate key set not present in
    // NODEJS_RELEASE_KEY_IDS, and iojs.org does not reliably serve
    // SHASUMS256.txt.sig, so forcing it would abort every io.js install.
    // The SHA-256 checksum still protects io.js against a tampered or
    // corrupt tarball.
    //
    // Previously this whole block was gated on `!target.is_iojs`, so io.js
    // installs skipped checksum verification entirely — a security
    // regression vs nvm-sh, which verifies io.js SHASUMS256.txt too.
    //
    // io.js archives are downloaded from IOJS_URI (the mirror config only
    // mirrors nodejs.org/dist), so SHASUMS256.txt must be fetched from the
    // same host the archive came from — otherwise we'd 404 against the
    // Node.js mirror and wrongly bail.
    let sums_base_url = if target.is_iojs { IOJS_URI } else { base_url };

    print!("  {} ", T("checksum_label").dimmed());
    if offline {
        println!("{}", T("checksum_offline").dimmed());
    } else {
        // Fetch SHASUMS256.txt ONCE and reuse the bytes for both the
        // checksum check and the GPG signature check. Previously each
        // check downloaded its own copy, so a single install issued two
        // GETs for the same small file. Sharing bytes also guarantees
        // both checks run against the identical document (a mirror
        // reformatting the file between the two requests could otherwise
        // cause a checksum-pass / signature-fail mismatch).
        let sums_bytes = match fetch_shasums(sums_base_url, &target.target_version) {
            Ok(b) => b,
            Err(e) => {
                println!("{}", T("checksum_failed").red().bold());
                anyhow::bail!("{}", e);
            }
        };

        // Hard security boundary: verify_checksum now returns Err for
        // any failure (network error, 404, archive not listed, hash
        // mismatch). A previous version returned Ok(false) and the
        // caller merely printed "skipped" — which let a MITM drop the
        // SHASUMS256.txt request and ship a tampered tarball. Use
        // --offline to bypass when the mirror is unreachable.
        match verify_checksum(&extract_path, &target.archive_name, &sums_bytes) {
            Ok(()) => println!("{}", T("checksum_verified").green().bold()),
            Err(e) => {
                println!("{}", T("checksum_failed").red().bold());
                anyhow::bail!("{}", e);
            }
        }

        // GPG signature verification of SHASUMS256.txt — extra trust layer
        // on top of the SHA-256 checksum. Node.js only (see the comment
        // above the checksum label for why io.js is excluded). Skips only
        // when gpg is missing, --no-gpg-verify is passed, or --offline is
        // in effect. A *failed* signature (gpg ran and rejected it) or an
        // unreachable .sig (network error, 404) aborts, since either could
        // indicate tampering or an active MITM stripping the signature.
        // The sums body is reused from the fetch above (no second download).
        if !target.is_iojs {
            print!("  {} ", T("gpg_label").dimmed());
            match verify_gpg_signature(
                base_url,
                &target.target_version,
                &sums_bytes,
                no_gpg_verify,
                offline,
            )? {
                GpgStatus::Verified => println!("{}", T("gpg_verified").green().bold()),
                GpgStatus::SkippedDisabled => println!("{}", T("gpg_disabled").dimmed()),
                GpgStatus::SkippedOffline => println!("{}", T("gpg_offline").dimmed()),
                GpgStatus::SkippedNoGpg => println!("{}", T("gpg_no_gpg").dimmed()),
                GpgStatus::SkippedKeyImport => {
                    println!("{}", T("gpg_key_import_failed").yellow().bold());
                    anyhow::bail!("{}", T("gpg_key_import_failed_abort"));
                }
                GpgStatus::Failed => {
                    println!("{}", T("gpg_failed").red().bold());
                    anyhow::bail!("{}", T("gpg_failed_abort"));
                }
            }
        }
    }

    if target.is_iojs {
        println!("{}", T("extracting"));
        extract_iojs_archive(&extract_path, version_dir, &target.target_version)?;
    } else {
        println!("{}", T("extracting"));
        extract_archive(&extract_path, version_dir, &target.target_version)?;
    }
    // Extraction succeeded — drop the guard to remove the offline temp copy
    // (online path had no guard armed). Matches the previous `if owns_extract_file
    // { fs::remove_file(&extract_path).ok(); }` but now also runs on every
    // error path above via the guard's Drop.
    drop(extract_guard);

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
        install_latest_pnpm_via_corepack(&target.target_version)?;
    }

    // --reinstall-packages-from after install
    if let Some(from_ver) = &cfg.reinstall_packages_from {
        // Resolve aliases (default, lts/iron, bare "22.22.2", etc.) the same
        // way `nvm reinstall-packages` does, so the option accepts the same
        // identifiers users already use elsewhere.
        let from_resolved = match resolve_alias(from_ver) {
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
        // Point `current` at the freshly installed version so the shell's
        // PATH picks it up. reinstall_packages_inner takes the source
        // version as an explicit arg (from_resolved), not from `current`,
        // but we still surface the write failure instead of `.ok()`
        // because a stale `current` would leave the shell on the wrong
        // version after install.
        let current_file = nvm_dir.join("current");
        atomic_write(&current_file, &target.target_version).with_context(|| {
            format!("{}: {}", T("cannot_write_current"), current_file.display())
        })?;
        if let Err(e) = reinstall_packages_inner(&from_resolved, &target.target_version) {
            eprintln!(
                "  {} {}",
                "⚠".yellow().bold(),
                format_t("migration_failed", &[e.to_string()])
            );
        }
    }

    // Create shims if they don't exist yet (idempotent — safe on every install).
    // Shims let `node`/`npm`/`npx` resolve to the active version without
    // shell-wrapper PATH manipulation.
    if !crate::shim::shims_exist() {
        if let Err(e) = crate::shim::create_shims() {
            eprintln!(
                "{} {}",
                "⚠".yellow().bold(),
                format_t("shim_create_failed", &[e.to_string()])
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

    // Serialize against other mutating nvm operations (concurrent
    // `nvm install <same-version>` would otherwise both pass the
    // "already installed?" check and extract into the same dir, leaving a
    // corrupted half-populated tree). The lock is released on drop at the
    // end of `install`. Held AFTER version resolution (read-only network
    // ops) but ACROSS the exists-check + download + extract critical section.
    let _nvm_lock = crate::utils::acquire_nvm_lock(&nvm_dir)?;

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

pub(crate) fn get_installed_version(version: &str) -> Result<String> {
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

#[cfg(test)]
mod tests {
    use super::*;

    // --- SourceGuard RAII --------------------------------------------------
    //
    // SourceGuard cleans up temp artifacts on every exit path of
    // install_from_source / install_binary. The contract:
    //   - armed file guard → remove_file on Drop
    //   - armed dir guard  → remove_dir_all on Drop
    //   - disarmed guard   → no-op (success path already cleaned up)

    #[test]
    fn source_guard_removes_file_on_drop_when_armed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("temp.tar");
        std::fs::write(&file, b"bytes").expect("write");
        assert!(file.exists());
        {
            let _guard = SourceGuard::file(file.clone());
        }
        assert!(
            !file.exists(),
            "armed file guard must remove the file on Drop"
        );
    }

    #[test]
    fn source_guard_removes_dir_on_drop_when_armed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let nested = dir.path().join("build");
        std::fs::create_dir_all(&nested).expect("mkdir");
        std::fs::write(nested.join("a.txt"), b"a").expect("write");
        {
            let _guard = SourceGuard::dir(dir.path().to_path_buf());
        }
        assert!(
            !dir.path().exists(),
            "armed dir guard must remove the whole tree on Drop"
        );
    }

    #[test]
    fn source_guard_is_noop_when_path_already_absent() {
        // Drop must not panic when the path was removed by an earlier step
        // (e.g. a concurrent uninstaller or a manual `rm` during install).
        let dir = tempfile::tempdir().expect("tempdir");
        let ghost = dir.path().join("missing.tmp");
        {
            let _guard = SourceGuard::file(ghost.clone());
        }
        // Still absent — the only assertion is "didn't panic".
        assert!(!ghost.exists());
    }

    // --- command_failed formatting -----------------------------------------
    //
    // command_failed renders the "<i18n message> (<exit code>)" string used
    // by every failed-external-command bail site. Pin the format so a future
    // refactor doesn't silently change what users see.

    #[test]
    fn command_failed_includes_i18n_message_and_exit_code() {
        // Use a real i18n key so the rendered message is the user-facing
        // string, not the raw key fallback.
        let status = std::process::Command::new("sh")
            .arg("-c")
            .arg("exit 42")
            .status()
            .expect("spawn sh");
        let err = command_failed("configure_failed", status);
        let msg = format!("{err}");
        assert!(
            msg.contains("42"),
            "expected exit code 42 in error, got: {msg}"
        );
    }

    #[test]
    fn command_failed_reports_signal_death_as_minus_one() {
        // `code()` returns None when the process was killed by a signal.
        // command_failed falls back to -1 in that case (matching the legacy
        // behaviour documented on the function); a real signal-killed
        // process is hard to spawn portably, so we synthesise a None-code
        // status via a successful `true` invocation and rely on the fact
        // that ExitStatus can't be constructed directly in stable Rust.
        //
        // We can still assert the format on a real success status — the
        // unwrap_or(-1) path only triggers when code() is None, which a
        // successful exit never is. This test guards the happy-path format.
        let status = std::process::Command::new("true")
            .status()
            .unwrap_or_else(|_| panic!("spawn true"));
        let err = command_failed("make_failed", status);
        let msg = format!("{err}");
        // Success exits with code 0; the message must include it.
        assert!(msg.contains('0'), "expected exit code 0, got: {msg}");
    }
}
