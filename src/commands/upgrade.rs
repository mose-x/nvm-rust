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

/// Rewrite a GitHub download URL to go through a mirror prefix.
///
/// `--from-mirror https://ghproxy.com/` turns
/// `https://github.com/o/r/releases/download/v1/a.tar.gz` into
/// `https://ghproxy.com/https://github.com/o/r/releases/download/v1/a.tar.gz`.
///
/// Two cases deliberately do NOT rewrite:
/// - `from_gitee` — the Gitee API already returned Gitee-hosted URLs, and
///   prefixing those with a GitHub mirror produces a nonsense URL.
/// - any URL that is not `https://github.com/` — mirrors proxy GitHub raw
///   download URLs, so rewriting anything else yields a broken URL.
///
/// Extracted because the asset URL and the checksums URL were two separate
/// copies of this logic, and they must be rewritten identically: a checksum
/// fetched from a different origin than the archive it verifies proves nothing
/// about that archive.
fn apply_mirror_prefix(url: String, mirror: Option<&str>, from_gitee: bool) -> String {
    let Some(prefix) = mirror else {
        return url;
    };
    if from_gitee || !url.starts_with("https://github.com/") {
        return url;
    }
    format!("{}/{}", prefix.trim_end_matches('/'), url)
}

/// Pick this host's asset out of a release's asset list.
///
/// Matches on the `-<target>.<ext>` suffix *and* an `nvm-` name prefix. The
/// prefix check is not cosmetic: `upgrade` swaps whatever this returns over the
/// running binary, so without it any asset whose name merely ended with the
/// host suffix — `notnvm-2.0.0-linux-x64.tar.gz`, say — would be installed.
fn select_asset<'a>(assets: &'a [Asset], target: &str, ext: &str) -> Option<&'a Asset> {
    let suffix = format!("-{target}.{ext}");
    assets
        .iter()
        .find(|a| a.name.starts_with("nvm-") && a.name.ends_with(&suffix))
}

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

    // Windows admin-install guard: the binary sits in a directory this
    // (non-elevated) process cannot write — typically Program Files from an
    // admin install. Bail BEFORE downloading instead of failing halfway with
    // a misleading "upgrade complete". The read-only paths above (--check,
    // up-to-date notice) are intentionally not guarded. Unix flow untouched.
    #[cfg(windows)]
    if upgrade_blocked_needs_admin(&bin_path) {
        let path_str = bin_path.display().to_string();
        eprintln!(
            "{}  {}",
            "✗".red().bold(),
            format_t("upgrade_admin_required", std::slice::from_ref(&path_str)).red()
        );
        eprintln!("  {} {}", "ℹ".cyan().bold(), T("upgrade_admin_hint").cyan());
        eprintln!("    {}", T("upgrade_admin_command").dimmed());
        anyhow::bail!("{}", T("upgrade_needs_admin"));
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
    let asset = select_asset(&assets, target, ext).ok_or_else(|| {
        anyhow::anyhow!(
            "{}",
            format_t(
                "upgrade_no_asset",
                std::slice::from_ref(&format!("nvm-*{target}.{ext}"))
            )
        )
    })?;
    let asset_name = asset.name.clone();

    // The API (version check) still hits github.com directly — mirrors usually
    // only proxy raw download URLs, not the API. `apply_mirror_prefix` handles
    // the `--from-gitee` and non-GitHub cases itself.
    let asset_url = apply_mirror_prefix(asset.url.clone(), from_mirror.as_deref(), from_gitee);

    println!("  {} {}", T("url_label").dimmed(), asset_url);

    // 4. Download to a temp file (NOT the cache: the cache lives under
    //    ~/.nvm.rust/cache and is keyed by node version names; nvm's own
    //    binary has a different lifecycle and shouldn't pollute it).
    let tmp_dir = tempfile::tempdir().context(T("upgrade_tmp_dir_failed"))?;
    let archive_path = tmp_dir.path().join(&asset_name);
    download_file(&client, &asset_url, &archive_path)?;

    // 5. Verify SHA256 against sha256sums.txt from the same release.
    //    Defends against CDN tampering and mirror corruption.
    //    Must go through the SAME rewrite as the asset URL above — that is why
    //    the rewrite is a shared helper rather than two inline copies. A
    //    checksum fetched from a different origin than the archive proves
    //    nothing about it.
    let checksums_url = assets
        .iter()
        .find(|a| a.name == "sha256sums.txt")
        .map(|a| apply_mirror_prefix(a.url.clone(), from_mirror.as_deref(), from_gitee));
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

/// Windows-only: true when the binary lives in a directory the current
/// process cannot write — an admin (Program Files) install being updated
/// from a non-elevated terminal. `nvm upgrade` is then blocked up front
/// with clear guidance instead of failing halfway with a false success.
/// User-dir installs are writable, so they pass through untouched.
#[cfg(windows)]
fn upgrade_blocked_needs_admin(bin_path: &std::path::Path) -> bool {
    let bin_parent = bin_path.parent().unwrap_or(std::path::Path::new("."));
    !is_dir_writable(bin_parent)
}

/// Sync the user-dir copy of the binary on Windows (admin installs have two
/// copies: the real one in Program Files and a synced copy in the user dir).
/// `swapped` tells whether `swap_binary` ran: it MOVES the extracted temp
/// binary into `bin_path`, so after a swap the temp file no longer exists
/// and the fresh bytes must be copied from `bin_path` itself. Copying the
/// consumed temp path was the os error 2 bug. No-op when there is no
/// user-dir copy or it IS the binary that was just updated (compared after
/// canonicalization — a direct `==` never matches on Windows because
/// `bin_path` carries a `\\?\` prefix, and the self-copy triggered a
/// sharing violation, os error 32).
#[cfg(windows)]
fn sync_user_dir_copy(
    bin_path: &std::path::Path,
    extracted_bin: &std::path::Path,
    swapped: bool,
) -> std::io::Result<()> {
    let user_bin = crate::system::get_nvm_dir().join("bin").join("nvm.exe");
    if !user_bin.exists() || crate::system::same_file_canonical(bin_path, &user_bin) {
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

        /// The os error 32 regression: `bin_path` arrives canonicalized
        /// (`\\?\...` on Windows) and never `==`-matches the plain
        /// `user_bin` path even when it IS the same file. The sync must
        /// still recognize the same file and skip — otherwise it copies the
        /// binary over itself and gets a sharing violation.
        #[test]
        fn sync_noop_when_canonicalized_paths_match() {
            let (guard, work) = setup();
            let user_bin = write_user_bin(guard._dir.path(), b"keep");
            let canonical = user_bin.canonicalize().unwrap();

            sync_user_dir_copy(&canonical, &work.path().join("missing.exe"), true).unwrap();
            assert_eq!(std::fs::read(&user_bin).unwrap(), b"keep");
        }

        /// Normal user-dir installs are writable and must NOT be blocked.
        #[test]
        fn admin_guard_passes_for_writable_dir() {
            let (_guard, work) = setup();
            let bin = work.path().join("nvm.exe");
            std::fs::write(&bin, b"x").unwrap();
            assert!(!upgrade_blocked_needs_admin(&bin));
        }

        /// Admin install (binary dir not writable by this terminal) must be
        /// blocked up front. Windows ignores the read-only attribute for
        /// directory access (verified empirically), so simulate the
        /// unwritable-location decision deterministically: a parent that is
        /// a file, and a parent that doesn't exist.
        #[test]
        fn admin_guard_blocks_non_writable_dir() {
            let (_guard, work) = setup();

            // Parent exists but is a FILE — not a writable directory.
            let not_a_dir = work.path().join("blocked");
            std::fs::write(&not_a_dir, b"x").unwrap();
            let bin = not_a_dir.join("nvm.exe");
            assert!(upgrade_blocked_needs_admin(&bin));

            // Missing parent dir is equally non-writable.
            let missing = work.path().join("no-such-dir").join("nvm.exe");
            assert!(upgrade_blocked_needs_admin(&missing));
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

    // --- U-3: the parts of `upgrade()`'s main flow that are reachable --------
    //
    // `upgrade()` cannot be driven end to end from a test: `current_binary_path()`
    // resolves `current_exe()`, which inside the test binary *is* the test
    // binary, and the release API URLs are hardcoded constants. Redirecting them
    // via an env override was considered and rejected — anyone able to set an env
    // var could then point `nvm upgrade` at their own binary, and the SHA-256
    // check would not help because `sha256sums.txt` comes from the same
    // redirected origin.
    //
    // So the decision logic is extracted and tested directly. What remains
    // untested here is the orchestration order and the network calls themselves.

    fn asset(name: &str) -> Asset {
        Asset {
            name: name.to_string(),
            url: format!("https://github.com/mose-x/nvm-rust/releases/download/v2.0.0/{name}"),
        }
    }

    #[test]
    fn mirror_prefix_rewrites_github_download_urls() {
        let url = "https://github.com/mose-x/nvm-rust/releases/download/v2.0.0/nvm-2.0.0-linux-x64.tar.gz";
        assert_eq!(
            apply_mirror_prefix(url.to_string(), Some("https://ghproxy.com"), false),
            format!("https://ghproxy.com/{url}")
        );
        // A trailing slash on the prefix must not produce `//`.
        assert_eq!(
            apply_mirror_prefix(url.to_string(), Some("https://ghproxy.com/"), false),
            format!("https://ghproxy.com/{url}")
        );
        assert_eq!(
            apply_mirror_prefix(url.to_string(), Some("https://ghproxy.com///"), false),
            format!("https://ghproxy.com/{url}")
        );
    }

    #[test]
    fn mirror_prefix_leaves_gitee_and_non_github_urls_alone() {
        let gitee = "https://gitee.com/mose-x/nvm-rust/releases/download/v2.0.0/nvm.tar.gz";
        // The Gitee API already returned Gitee-hosted URLs; prefixing them with
        // a GitHub mirror would produce a URL that resolves to nothing.
        assert_eq!(
            apply_mirror_prefix(gitee.to_string(), Some("https://ghproxy.com"), true),
            gitee
        );

        let other = "https://example.invalid/nvm-2.0.0-linux-x64.tar.gz";
        assert_eq!(
            apply_mirror_prefix(other.to_string(), Some("https://ghproxy.com"), false),
            other,
            "mirrors proxy GitHub download URLs only"
        );

        // No mirror requested at all.
        let gh = "https://github.com/mose-x/nvm-rust/x.tar.gz";
        assert_eq!(apply_mirror_prefix(gh.to_string(), None, false), gh);
    }

    /// The `from_gitee` guard is only observable when a Gitee-sourced asset URL
    /// *does* look like a GitHub download URL — which is legal, since a Gitee
    /// release can link its assets anywhere. The gitee.com case above is already
    /// excluded by the non-GitHub check, so without this test deleting the
    /// `from_gitee` guard would change nothing and the deletion would go
    /// unnoticed.
    #[test]
    fn mirror_prefix_skips_gitee_sourced_urls_that_point_at_github() {
        let url = "https://github.com/mose-x/nvm-rust/releases/download/v2.0.0/nvm-2.0.0-linux-x64.tar.gz";
        // `--from-gitee` means the Gitee API already chose the origin, so a
        // GitHub mirror must not be layered on top of it.
        assert_eq!(
            apply_mirror_prefix(url.to_string(), Some("https://ghproxy.com"), true),
            url,
            "from_gitee must suppress the rewrite on its own"
        );
        // And the same URL *is* rewritten when not from Gitee, so the assertion
        // above is not passing for some unrelated reason.
        assert_ne!(
            apply_mirror_prefix(url.to_string(), Some("https://ghproxy.com"), false),
            url
        );
    }

    /// The reason the rewrite was extracted: the archive and the checksum that
    /// vouches for it must come from the *same* origin. Two inline copies could
    /// drift, and a checksum fetched from GitHub while the archive comes from a
    /// mirror proves nothing about the bytes actually installed.
    #[test]
    fn mirror_prefix_treats_the_asset_and_the_checksums_identically() {
        let mirror = Some("https://ghproxy.com/");
        let asset_url = apply_mirror_prefix(asset("nvm-2.0.0-linux-x64.tar.gz").url, mirror, false);
        let sums_url = apply_mirror_prefix(asset("sha256sums.txt").url, mirror, false);

        let origin = |u: &str| {
            u.trim_start_matches("https://")
                .split('/')
                .next()
                .unwrap_or("")
                .to_string()
        };
        assert_eq!(origin(&asset_url), "ghproxy.com");
        assert_eq!(
            origin(&asset_url),
            origin(&sums_url),
            "archive and checksum must be fetched from the same origin"
        );
    }

    /// Source-level guard on the refactor: the rewrite must stay in one place.
    /// Locking only the call sites would let someone inline the logic back at
    /// one of them and leave the helper as dead code.
    #[test]
    fn mirror_rewrite_is_not_re_inlined_at_the_call_sites() {
        let src = include_str!("upgrade.rs");
        // Only the production half: this test module itself calls the helper, so
        // counting the whole file would include the tests' own call sites.
        let production = src
            .split("\nmod tests {")
            .next()
            .expect("upgrade.rs must contain a test module");
        assert!(
            production.len() < src.len(),
            "the test-module boundary was not found; the guard would be vacuous"
        );

        let calls = production.matches("apply_mirror_prefix(").count();
        // One definition + the asset URL + the checksums URL.
        assert_eq!(
            calls, 3,
            "expected the definition plus exactly two call sites, found {calls}"
        );
        // The trailing-slash trim is the distinguishing part of the rewrite; if
        // it appears more than once, a copy has been inlined somewhere.
        assert_eq!(
            production.matches("trim_end_matches('/')").count(),
            1,
            "the mirror rewrite must live only in apply_mirror_prefix"
        );
    }

    #[test]
    fn select_asset_picks_the_host_target_from_a_real_release() {
        let assets = build_assets_from_tag("v2.0.0");
        // Every target the release workflow publishes, with its own extension.
        for (target, ext) in [
            ("linux-x64", "tar.gz"),
            ("linux-musl-x64", "tar.gz"),
            ("linux-arm64", "tar.gz"),
            ("linux-musl-arm64", "tar.gz"),
            ("macos-x64", "tar.gz"),
            ("macos-arm64", "tar.gz"),
            ("windows-x64", "zip"),
            ("windows-arm64", "zip"),
        ] {
            let picked = select_asset(&assets, target, ext)
                .unwrap_or_else(|| panic!("no asset selected for {target}.{ext}"));
            assert_eq!(
                picked.name,
                format!("nvm-2.0.0-{target}.{ext}"),
                "wrong asset for {target}"
            );
        }
    }

    #[test]
    fn select_asset_does_not_confuse_musl_with_gnu() {
        let assets = build_assets_from_tag("v2.0.0");
        // `linux-x64` must not match `nvm-2.0.0-linux-musl-x64.tar.gz`: installing
        // a musl build on a glibc host (or the reverse) produces a binary that
        // fails to load at runtime, long after the upgrade reported success.
        assert_eq!(
            select_asset(&assets, "linux-x64", "tar.gz").map(|a| a.name.as_str()),
            Some("nvm-2.0.0-linux-x64.tar.gz")
        );
        assert_eq!(
            select_asset(&assets, "linux-musl-x64", "tar.gz").map(|a| a.name.as_str()),
            Some("nvm-2.0.0-linux-musl-x64.tar.gz")
        );
        assert_eq!(
            select_asset(&assets, "linux-arm64", "tar.gz").map(|a| a.name.as_str()),
            Some("nvm-2.0.0-linux-arm64.tar.gz")
        );
    }

    /// `upgrade` swaps whatever `select_asset` returns over the running binary,
    /// so the `nvm-` prefix check is a security control, not tidiness.
    #[test]
    fn select_asset_rejects_lookalike_names() {
        let assets = vec![
            asset("notnvm-2.0.0-linux-x64.tar.gz"),
            asset("evil-nvm-2.0.0-linux-x64.tar.gz"),
            asset("xnvm-2.0.0-linux-x64.tar.gz"),
        ];
        assert!(
            select_asset(&assets, "linux-x64", "tar.gz").is_none(),
            "only names starting with `nvm-` may be installed"
        );

        // With a legitimate asset also present, it is the one chosen.
        let mut mixed = assets;
        mixed.push(asset("nvm-2.0.0-linux-x64.tar.gz"));
        assert_eq!(
            select_asset(&mixed, "linux-x64", "tar.gz").map(|a| a.name.as_str()),
            Some("nvm-2.0.0-linux-x64.tar.gz")
        );
    }

    #[test]
    fn select_asset_never_picks_the_checksums_file() {
        let assets = build_assets_from_tag("v2.0.0");
        assert!(assets.iter().any(|a| a.name == "sha256sums.txt"));
        for (target, ext) in [("linux-x64", "tar.gz"), ("windows-x64", "zip")] {
            let picked = select_asset(&assets, target, ext).expect("an asset must match");
            assert_ne!(picked.name, "sha256sums.txt");
        }
    }

    #[test]
    fn select_asset_is_extension_sensitive() {
        let assets = build_assets_from_tag("v2.0.0");
        // Windows assets ship as .zip; asking for .tar.gz must not silently
        // match the zip (which `extract_binary` would then fail on).
        assert!(
            select_asset(&assets, "windows-x64", "tar.gz").is_none(),
            "a .zip asset must not satisfy a .tar.gz request"
        );
        assert!(
            select_asset(&assets, "linux-x64", "zip").is_none(),
            "a .tar.gz asset must not satisfy a .zip request"
        );
    }

    #[test]
    fn select_asset_returns_none_when_the_host_is_absent() {
        assert!(
            select_asset(&[], "linux-x64", "tar.gz").is_none(),
            "an empty asset list must yield no match"
        );
        let only_windows = build_assets_from_tag("v2.0.0")
            .into_iter()
            .filter(|a| a.name.contains("windows"))
            .collect::<Vec<_>>();
        assert!(
            select_asset(&only_windows, "linux-x64", "tar.gz").is_none(),
            "a release without this host's target must not fall back to another"
        );
    }

    /// `upgrade()` rejects mutually-exclusive flags before it touches
    /// `current_binary_path()` or the network, so this is directly callable.
    /// `--check --rollback` used to silently run the rollback because rollback
    /// returned first; the guard exists to make that a clear error instead.
    #[test]
    fn upgrade_rejects_conflicting_flags_before_any_io() {
        // (check, force, rollback) — every combination with more than one set.
        for (check, force, rollback) in [
            (true, true, false),
            (true, false, true),
            (false, true, true),
            (true, true, true),
        ] {
            let err = upgrade(check, force, false, None, rollback)
                .expect_err("conflicting flags must be rejected");
            let msg = format!("{err}");
            // The flag names are interpolated into the message, so they are
            // stable across locales (the surrounding wording is not).
            for (flag, on) in [
                ("--check", check),
                ("--force", force),
                ("--rollback", rollback),
            ] {
                assert_eq!(
                    msg.contains(flag),
                    on,
                    "the error should name exactly the conflicting flags, got: {msg}"
                );
            }
        }
    }
}
