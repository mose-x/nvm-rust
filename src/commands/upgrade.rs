//! `nvm upgrade` — self-update nvm-rust from a GitHub/Gitee release.
//!
//! Flow:
//!   1. Query the latest release tag from the GitHub API (or Gitee API).
//!   2. Compare against `CARGO_PKG_VERSION` (compiled-in current version).
//!   3. Pick the asset matching the host target triple
//!      (`nvm-<target>.tar.gz` / `nvm-<target>.zip`).
//!   4. Download + verify SHA256 against `sha256sums.txt`.
//!   5. Extract the new binary to a temp file, swap it into place, and
//!      keep the previous binary as `nvm.bak` for `--rollback`.
//!
//! The binary lives at `~/.nvm.rust/bin/nvm` (per install.sh); replacing
//! it does NOT require sudo. `/usr/local/bin/nvm` is a symlink install.sh
//! optionally creates, and points at the user-dir binary, so the swap is
//! transparent to that link.
//!
//! The release API client, download/verify helpers, and binary swap/rollback
//! logic live in sibling modules and are re-exported below for backward
//! compatibility.

use anyhow::{Context, Result};
use colored::Colorize;
use std::path::PathBuf;

use crate::i18n::{format_t, T};
use crate::proxy::build_http_client;

// Re-export extracted modules so `commands::*` call sites remain unchanged.
pub(crate) use super::{binary_swap::*, download_verify::*, release_api::*};

/// Entry point invoked from `main.rs` dispatch.
///
/// `check`    — only print "newer version available" (or "up to date"), exit 0.
/// `force`    — reinstall even if the latest tag equals the current version.
/// `from_gitee` — use the Gitee mirror for both API and download.
/// `from_mirror` — use a custom mirror URL prefix (e.g. `https://ghproxy.com/`).
///                  Applied to GitHub download URLs only (the API still hits
///                  github.com so the version check works without a mirror).
/// `rollback` — restore `nvm.bak` over the live binary and exit.
pub fn upgrade(
    check: bool,
    force: bool,
    from_gitee: bool,
    from_mirror: Option<String>,
    rollback: bool,
) -> Result<()> {
    // Reject mutually-exclusive flag combos up front so users get a clear
    // error instead of silent precedence (e.g. `--check --rollback` used to
    // silently run rollback because rollback returns first).
    let mut active: Vec<&str> = Vec::new();
    if check {
        active.push("--check");
    }
    if force {
        active.push("--force");
    }
    if rollback {
        active.push("--rollback");
    }
    if active.len() > 1 {
        anyhow::bail!(
            "{}",
            format_t(
                "upgrade_conflict_flags",
                std::slice::from_ref(&active.join(", "))
            )
        );
    }

    // --from-gitee and --from-mirror are not conflicting (both can be set),
    // but --from-mirror is silently ignored when --from-gitee is active
    // because Gitee serves its own download URLs. Warn so the user knows.
    if from_gitee && from_mirror.is_some() {
        eprintln!(
            "  {} {}",
            "⚠".yellow().bold(),
            T("upgrade_gitee_ignores_mirror").yellow()
        );
    }

    let bin_path = current_binary_path()?;

    if rollback {
        return rollback_binary(&bin_path);
    }

    let client = build_http_client();

    // 1. Resolve the latest release tag + asset list.
    let (latest_tag, assets, api_source) = fetch_latest_release(&client, from_gitee)?;

    let current = env!("CARGO_PKG_VERSION");
    println!(
        "{}  {} {}",
        "ℹ".cyan().bold(),
        T("upgrade_current").cyan(),
        current.white().bold()
    );
    println!(
        "  {} {} ({})",
        T("upgrade_latest_label").dimmed(),
        latest_tag.white().bold(),
        api_source.dimmed()
    );

    // 2. Compare versions. `compare_semver` returns Greater when a > b,
    //    so latest > current means an upgrade is available.
    let newer = crate::utils::compare_semver(&latest_tag, &format!("v{}", current))
        == std::cmp::Ordering::Greater;

    if !newer && !force {
        println!(
            "{}  {}",
            "✓".green().bold(),
            T("upgrade_up_to_date").green()
        );
        return Ok(());
    }

    if check {
        if newer {
            println!(
                "{}  {}",
                "↑".yellow().bold(),
                format_t("upgrade_available", std::slice::from_ref(&latest_tag)).yellow()
            );
        } else {
            println!(
                "{}  {}",
                "✓".green().bold(),
                T("upgrade_up_to_date").green()
            );
        }
        return Ok(());
    }

    if !newer && force {
        println!(
            "{}  {}",
            "ℹ".cyan().bold(),
            T("upgrade_force_reinstall").cyan()
        );
    }

    // 3. Pick the asset matching the host target.
    //    Asset naming: `nvm-<version>-<os>-<arch>[<variant>].<ext>`
    //    e.g. nvm-2.0.0-linux-x64.tar.gz, nvm-2.0.0-linux-musl-x64.tar.gz,
    //         nvm-2.0.0-macos-arm64.tar.gz, nvm-2.0.0-windows-arm64.zip
    //    `version` is the latest release tag (without leading `v`); we
    //    don't need it for matching — we look up the asset by the
    //    `<os>-<arch>.<ext>` suffix, which is unique per release.
    let target = host_target()?;
    let ext = if cfg!(windows) { "zip" } else { "tar.gz" };
    let suffix = format!("-{}.{}", target, ext);
    let asset = assets
        .iter()
        .find(|a| a.name.ends_with(&suffix) && a.name.starts_with("nvm-"))
        .ok_or_else(|| {
            anyhow::anyhow!(
                "{}",
                format_t(
                    "upgrade_no_asset",
                    std::slice::from_ref(&format!("nvm-*{}.{}", target, ext))
                )
            )
        })?;
    let asset_name = asset.name.clone();
    let asset_url = asset.url.clone();

    // Apply a custom mirror prefix to the GitHub download URL.
    // `--from-mirror https://ghproxy.com/` rewrites
    //   https://github.com/.../nvm-2.0.0-linux-x64.tar.gz
    // to
    //   https://ghproxy.com/https://github.com/.../nvm-...tar.gz
    // The API (version check) still hits github.com directly — mirrors
    // usually only proxy raw download URLs, not the API. `--from-gitee`
    // already gave us gitee URLs from the Gitee API; mirroring those is
    // pointless, so we skip the rewrite in that case.
    let asset_url = if let Some(prefix) = from_mirror.as_deref() {
        if !from_gitee && asset_url.starts_with("https://github.com/") {
            let trimmed = prefix.trim_end_matches('/');
            format!("{}/{}", trimmed, asset_url)
        } else {
            asset_url
        }
    } else {
        asset_url
    };

    println!("  {} {}", T("url_label").dimmed(), asset_url);

    // 4. Download to a temp file (NOT the cache: the cache lives under
    //    ~/.nvm.rust/cache and is keyed by node version names; nvm's own
    //    binary has a different lifecycle and shouldn't pollute it).
    let tmp_dir = tempfile::tempdir().context(T("upgrade_tmp_dir_failed"))?;
    let archive_path = tmp_dir.path().join(&asset_name);
    download_file(&client, &asset_url, &archive_path)?;

    // 5. Verify SHA256 against sha256sums.txt from the same release.
    //    Defends against CDN tampering and mirror corruption.
    //    Apply the same mirror rewrite as the asset URL.
    let checksums_url = assets
        .iter()
        .find(|a| a.name == "sha256sums.txt")
        .map(|a| a.url.clone())
        .map(|url| {
            if let Some(prefix) = from_mirror.as_deref() {
                if !from_gitee && url.starts_with("https://github.com/") {
                    let trimmed = prefix.trim_end_matches('/');
                    return format!("{}/{}", trimmed, url);
                }
            }
            url
        });
    if let Some(url) = checksums_url {
        verify_sha256(&client, &url, &asset_name, &archive_path)?;
    } else {
        eprintln!(
            "{}  {}",
            "⚠".yellow().bold(),
            T("upgrade_no_checksums").yellow()
        );
    }

    // 6. Extract the binary from the archive.
    //    - Unix tar.gz: `tar -czf ... -C target/.../release nvm` → archive
    //      root contains just `nvm`.
    //    - Windows zip: `7z a ... target/.../release/nvm.exe` (no `-C`/`-spf`)
    //      → the entry may carry its source path prefix
    //      (`target/x86_64-pc-windows-msvc/release/nvm.exe`). `extract_binary`
    //      matches by `ends_with("/nvm.exe")` so both layouts work.
    let extracted_bin = extract_binary(&archive_path, tmp_dir.path())?;

    // 7. Swap into place: backup current → move new into bin_path.
    //    On Unix, renaming over a running binary is fine (the kernel keeps
    //    the old inode alive for the running process). On Windows the exe
    //    is locked, so we rename the old one to .bak first, then write new.
    //
    //    If the binary lives in a system path like /usr/local/bin (not
    //    user-writable), the three-step swap can't write temp/backup files
    //    there. No auto-sudo — print explicit manual instructions so the
    //    user can run the sudo cp themselves. We continue with shims/
    //    completions/refresh since the binary was downloaded and the user
    //    can update manually.
    let bin_parent = bin_path.parent().unwrap_or(std::path::Path::new("."));
    let can_write = is_dir_writable(bin_parent);
    if can_write {
        // User-writable path (e.g. ~/.nvm.rust/bin/) — use the atomic swap.
        swap_binary(&bin_path, &extracted_bin)?;
    } else {
        // System path (e.g. /usr/local/bin/nvm) — not writable.
        // No auto-sudo: print manual instructions and continue.
        eprintln!(
            "  {} System path not writable. To update manually:",
            "⚠".yellow().bold()
        );
        eprintln!(
            "    sudo cp -f {} {} && sudo chmod 755 {}",
            extracted_bin.display(),
            bin_path.display(),
            bin_path.display()
        );
    }

    // Sync user-dir copy on Windows — install.ps1 admin mode creates two copies
    // (Program Files real + user dir copy). Upgrade only updates the one
    // current_exe() resolves to. Sync the other to prevent version drift.
    #[cfg(windows)]
    if let Err(e) = sync_user_dir_copy(&bin_path, &extracted_bin, can_write) {
        eprintln!(
            "  {} Failed to sync user-dir copy: {}",
            "⚠".yellow().bold(),
            e
        );
    }

    // Migrate to shim architecture: create shim scripts for all SHIM_COMMANDS.
    // `migrate_to_shims` handles first-time setup (creates shims dir + ensures
    // `current` file exists). `create_shims` then (re)writes the shim scripts
    // with the latest content — this is critical on upgrade because the shim
    // script logic may have changed between nvm versions. Both are idempotent.
    if let Err(e) = crate::shim::migrate_to_shims() {
        eprintln!(
            "  {} {}",
            "⚠".yellow().bold(),
            format_t("shim_create_failed", &[e.to_string()])
        );
    }
    if let Err(e) = crate::shim::create_shims() {
        eprintln!(
            "  {} {}",
            "⚠".yellow().bold(),
            format_t("shim_create_failed", &[e.to_string()])
        );
    }

    // Regenerate shell completion scripts if they're already installed.
    // Keeps completions in sync with the new nvm version (new commands,
    // updated descriptions, fixed bash _init_completion fallback, etc.).
    // Only overwrites files that already exist — never creates new ones.
    let _ = crate::completions::regenerate_completions_if_installed();

    // Exec the NEW binary with "refresh" to run Full Shim migration.
    // The old binary (still in memory) can't call v2 functions like
    // migrate_to_full_shim — the new binary on disk can. Suppress
    // stdout/stderr so clap errors don't confuse the user if the new
    // binary doesn't support "refresh" (too old).
    let refresh_ok = std::process::Command::new(&bin_path)
        .arg("refresh")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if refresh_ok {
        println!("  {} {}", "✓".green().bold(), T("shim_migrated"));
    } else {
        eprintln!("  {} {}", "⚠".yellow().bold(), T("upgrade_refresh_hint"));
    }

    println!(
        "{}  {} ({} → {})",
        "✓".green().bold(),
        T("upgrade_done").green().bold(),
        format!("v{}", current).dimmed(),
        latest_tag.white().bold()
    );
    println!(
        "  {} {}",
        T("tip_label").dimmed(),
        T("upgrade_restart_hint").dimmed()
    );

    Ok(())
}

/// Sync the user-dir copy of the binary on Windows (admin installs have two
/// copies: the real one in Program Files and a synced copy in the user dir).
/// `swapped` tells whether `swap_binary` ran: it MOVES the extracted temp
/// binary into `bin_path`, so after a swap the temp file no longer exists
/// and the fresh bytes must be copied from `bin_path` itself. Copying the
/// consumed temp path was the os error 2 bug. No-op when there is no
/// user-dir copy or it IS the binary that was just updated.
#[cfg(windows)]
fn sync_user_dir_copy(
    bin_path: &std::path::Path,
    extracted_bin: &std::path::Path,
    swapped: bool,
) -> std::io::Result<()> {
    let user_bin = crate::system::get_nvm_dir().join("bin").join("nvm.exe");
    if !user_bin.exists() || user_bin == bin_path {
        return Ok(());
    }
    let source = if swapped { bin_path } else { extracted_bin };
    std::fs::copy(source, &user_bin).map(|_| ())
}

/// Where the currently-running nvm binary lives on disk.
/// `std::env::current_exe` follows symlinks, so if the user installed via
/// install.sh's `/usr/local/bin/nvm` symlink, this returns the real path
/// under `~/.nvm.rust/bin/nvm` — which is what we want to replace.
///
/// With the new EDR-safe layout (real binary in /usr/local/bin, symlink in
/// ~/.nvm.rust/bin/), `current_exe` returns /usr/local/bin/nvm (the real
/// binary), which is what we want to replace on upgrade.
fn current_binary_path() -> Result<PathBuf> {
    let exe = std::env::current_exe().context(T("upgrade_find_exe_failed"))?;
    // Canonicalize to resolve any symlinks in the parent path. The binary
    // itself may be the target of a symlink (e.g. /usr/local/bin/nvm →
    // ~/.nvm.rust/bin/nvm); current_exe already resolves that, but the
    // parent dir might have its own symlinks.
    let exe = exe.canonicalize().unwrap_or(exe);
    Ok(exe)
}

/// Host platform identifier used in release asset names.
/// Returns the `<os>-<arch>[<variant>]` portion of the asset name, e.g.
/// `linux-x64`, `linux-arm64`, `linux-musl-x64`, `macos-arm64`,
/// `windows-x64`, `windows-arm64`.
///
/// We use the user-facing `os-arch` form (matching Node.js's own naming)
/// rather than the verbose Rust target triple (`x86_64-unknown-linux-gnu`)
/// so users can tell at a glance which asset to download from the release
/// page. `std::env::consts` reports the host we're running on, so this
/// correctly handles cross-arch situations (e.g. x64 nvm running under
/// Rosetta on an Apple Silicon Mac still reports `x86_64`).
///
/// On Linux we distinguish musl from gnu via `/proc/self/cputype`-free
/// heuristic: we check `ldd --version` output for "musl". This catches
/// Alpine/distroless users who want the fully-static binary; glibc users
/// (the overwhelming majority) get the gnu build.
///
/// Unknown platforms bail instead of silently falling back to x86_64-linux,
/// which would download an unrunnable binary and brick the next `nvm` call.
fn host_target() -> Result<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => {
            // Detect musl libc (Alpine, distroless). `ldd --version` on musl
            // systems prints "musl libc"; on glibc it prints "ldd (GNU libc)".
            // Failure to detect → fall back to gnu (the common case).
            if is_musl_libc() {
                Ok("linux-musl-x64")
            } else {
                Ok("linux-x64")
            }
        }
        ("linux", "aarch64") => {
            // Detect musl libc on ARM64 (Alpine ARM64, distroless).
            // Same heuristic as x86_64: check `ldd --version` for "musl".
            if is_musl_libc() {
                Ok("linux-musl-arm64")
            } else {
                Ok("linux-arm64")
            }
        }
        ("macos", "x86_64") => Ok("macos-x64"),
        ("macos", "aarch64") => Ok("macos-arm64"),
        ("windows", "x86_64") => Ok("windows-x64"),
        ("windows", "aarch64") => Ok("windows-arm64"),
        (os, arch) => anyhow::bail!("{}: {}-{}", T("upgrade_unsupported_platform"), os, arch),
    }
}

/// Detect musl libc on Linux by inspecting `ldd --version` output.
/// Returns `true` if the system uses musl, `false` for glibc or unknown.
///
/// Some Alpine versions print the version banner to stderr instead of
/// stdout, so we merge both streams before checking for "musl".
fn is_musl_libc() -> bool {
    std::process::Command::new("ldd")
        .arg("--version")
        .output()
        .ok()
        .and_then(|o| {
            let mut combined = String::from_utf8_lossy(&o.stdout).into_owned();
            combined.push_str(&String::from_utf8_lossy(&o.stderr));
            if combined.contains("musl") {
                Some(true)
            } else {
                None
            }
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_host_target_returns_valid_asset_suffix() {
        // host_target() returns a suffix like "linux-x64", "linux-musl-arm64",
        // "macos-arm64", "windows-x64". Verify the result is one of the
        // known targets in build_assets_from_tag.
        let target = host_target().expect("host_target should succeed on supported platforms");
        let assets = build_assets_from_tag("v9.9.9");
        let expected_name = format!("nvm-9.9.9-{}.tar.gz", target);
        let expected_zip = format!("nvm-9.9.9-{}.zip", target);
        assert!(
            assets
                .iter()
                .any(|a| a.name == expected_name || a.name == expected_zip),
            "host_target() returned '{}' but no matching asset found in build_assets_from_tag",
            target
        );
    }

    #[cfg(windows)]
    mod sync_tests {
        use super::*;
        use crate::system::ENV_TESTS_MUTEX;
        use std::env;
        use tempfile::TempDir;

        struct NvmDirGuard {
            old_value: Option<String>,
            _dir: TempDir,
            _mutex: std::sync::MutexGuard<'static, ()>,
        }

        impl Drop for NvmDirGuard {
            fn drop(&mut self) {
                match &self.old_value {
                    Some(v) => env::set_var("NVM_DIR", v),
                    None => env::remove_var("NVM_DIR"),
                }
            }
        }

        fn setup() -> (NvmDirGuard, TempDir) {
            let mutex = ENV_TESTS_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
            let old_value = env::var("NVM_DIR").ok();
            let nvm_dir = tempfile::tempdir().expect("tempdir");
            env::set_var("NVM_DIR", nvm_dir.path());
            let work = tempfile::tempdir().expect("tempdir");
            (
                NvmDirGuard {
                    old_value,
                    _dir: nvm_dir,
                    _mutex: mutex,
                },
                work,
            )
        }

        fn write_user_bin(nvm_dir: &std::path::Path, content: &[u8]) -> std::path::PathBuf {
            let bin_dir = nvm_dir.join("bin");
            std::fs::create_dir_all(&bin_dir).unwrap();
            let user_bin = bin_dir.join("nvm.exe");
            std::fs::write(&user_bin, content).unwrap();
            user_bin
        }

        /// The regression: after a successful swap the extracted temp binary
        /// is gone (moved into bin_path). Syncing must copy from bin_path,
        /// not from the consumed temp path (that was os error 2).
        #[test]
        fn sync_after_swap_copies_from_bin_path() {
            let (guard, work) = setup();
            let user_bin = write_user_bin(guard._dir.path(), b"old");
            let bin_path = work.path().join("ProgramFiles").join("nvm.exe");
            std::fs::create_dir_all(bin_path.parent().unwrap()).unwrap();
            std::fs::write(&bin_path, b"new").unwrap();
            let consumed_temp = work.path().join("temp").join("nvm.exe"); // never created

            sync_user_dir_copy(&bin_path, &consumed_temp, true).unwrap();
            assert_eq!(std::fs::read(&user_bin).unwrap(), b"new");
        }

        /// Swap skipped (system path not writable): the temp binary still
        /// exists and is the right source.
        #[test]
        fn sync_without_swap_copies_from_extracted() {
            let (guard, work) = setup();
            let user_bin = write_user_bin(guard._dir.path(), b"old");
            let bin_path = work.path().join("ProgramFiles").join("nvm.exe");
            std::fs::create_dir_all(bin_path.parent().unwrap()).unwrap();
            std::fs::write(&bin_path, b"stale").unwrap();
            let extracted = work.path().join("temp").join("nvm.exe");
            std::fs::create_dir_all(extracted.parent().unwrap()).unwrap();
            std::fs::write(&extracted, b"new").unwrap();

            sync_user_dir_copy(&bin_path, &extracted, false).unwrap();
            assert_eq!(std::fs::read(&user_bin).unwrap(), b"new");
        }

        #[test]
        fn sync_noop_when_user_bin_is_the_updated_binary() {
            let (guard, work) = setup();
            let user_bin = write_user_bin(guard._dir.path(), b"keep");

            sync_user_dir_copy(&user_bin, &work.path().join("missing.exe"), true).unwrap();
            assert_eq!(std::fs::read(&user_bin).unwrap(), b"keep");
        }

        #[test]
        fn sync_noop_when_no_user_copy_exists() {
            let (guard, work) = setup();
            let bin_path = work.path().join("nvm.exe");
            std::fs::write(&bin_path, b"new").unwrap();

            sync_user_dir_copy(&bin_path, &work.path().join("missing.exe"), true).unwrap();
            assert!(!guard._dir.path().join("bin").join("nvm.exe").exists());
        }
    }
}
