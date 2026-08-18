//! Source-build install path (`nvm install --source`).
//!
//! `install_from_source` compiles Node.js from a source tarball using the
//! Unix toolchain (`./configure`, `make`). It is unreachable on Windows
//! (rejected up-front in `build_install_target` with `source_unsupported_windows`).
//! io.js source compilation is also rejected upstream.
//!
//! `SourceGuard`, `command_failed`, and `InstallTarget` live in
//! [`super::install`] because they are shared with the binary-install path
//! (`install_binary`) and the orchestrator (`install`).

use anyhow::{Context, Result};
use colored::Colorize;
use std::fs;
use std::path::Path;
use std::process::Command;

use super::version_resolve::get_source_url;
use crate::download::{copy_from_cache, download_to_cache, is_cached};
use crate::i18n::{format_t, T};

use super::install::{command_failed, InstallTarget, SourceGuard};
use super::package_upgrade::download_prebuilt_npm;

/// Compile and install Node.js from source. Used when `--source` is passed.
/// io.js source compilation is rejected upstream in `build_install_target`.
///
/// External toolchain required: a POSIX `sh`, `make`, a C compiler, and `tar`
/// supporting `--strip-components=1` (GNU tar; bsdtar on Windows 10 build
/// 17063+ behaves differently for that flag — `--source` is primarily a
/// Unix power-user feature, prebuilt binaries are the default install path).
pub(crate) fn install_from_source(
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

    // Extract straight from the cache file. The previous code copied the
    // ~80MB source tarball from `cache/<name>` to `<nvm_dir>/<ver>.src.tmp`
    // and then had `tar` read that copy — a redundant full-file disk write +
    // read on every source install. We now point `tar` directly at the cache
    // file. `owns_src` tracks whether that file is our temp copy (offline
    // path, must be cleaned up) or the shared cache (online path, must stay).
    let temp_src = nvm_dir.join(format!("{}.src.tmp", target.target_version));
    let (src_path, owns_src) = if offline {
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
        copy_from_cache(archive_name, &temp_src)?;
        (temp_src, true)
    } else {
        let cached_path = download_to_cache(&source_url, archive_name)?;
        (cached_path, false)
    };

    // RAII guard so the temp source copy is removed on every exit path
    // (including `?` early-returns from tar/configure/make failures), not
    // just the success path. A `let _ = fs::remove_file` at the end of the
    // function was skipped whenever an error bailed out, leaving the ~80MB
    // temp file behind on every failed source install.
    let src_guard = owns_src.then(|| SourceGuard::file(src_path.clone()));

    let build_dir = nvm_dir.join(format!("node-v{}.build", target.target_version));
    fs::create_dir_all(&build_dir)?;
    // Same RAII pattern for the build dir: `remove_dir_all` at the end was
    // skipped on every `?` failure, leaving the extracted source tree behind.
    let build_guard = SourceGuard::dir(build_dir.clone());

    println!("  {} {}", "›".dimmed(), T("source_extract"));
    let status = Command::new("tar")
        .arg("xf")
        .arg(&src_path)
        .arg("-C")
        .arg(&build_dir)
        .arg("--strip-components=1")
        .status()
        .context(T("tar_extract_failed"))?;
    if !status.success() {
        anyhow::bail!(command_failed("extract_source_failed", status));
    }
    // Source extracted into build_dir; the temp tarball copy is no longer
    // needed. Drop the guard early so it doesn't outlive its usefulness
    // (and so a later failure doesn't re-delete an already-removed file).
    drop(src_guard);

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
        anyhow::bail!(command_failed("configure_failed", cfg));
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
        anyhow::bail!(command_failed("make_failed", m));
    }

    println!("  {} {}", "›".dimmed(), T("source_install"));
    let mi = Command::new("make")
        .arg("install")
        .current_dir(&build_dir)
        .status()
        .context(T("make_install_failed"))?;
    if !mi.success() {
        anyhow::bail!(command_failed("make_install_failed", mi));
    }

    // Install succeeded — clean up the build tree, matching the previous
    // `remove_dir_all(&build_dir).ok()` at the end of the happy path. The
    // guard's Drop would do this anyway, but doing it explicitly + disarming
    // avoids a redundant stat in Drop and makes the intent obvious.
    drop(build_guard);

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
