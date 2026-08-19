//! Binary extraction, swap, rollback, and crash-recovery for `nvm upgrade`.
//!
//! Extracted from `upgrade.rs` as part of P1-7 module refactoring. All
//! items are re-exported through `upgrade::*` so existing call sites are
//! unchanged.

use anyhow::{Context, Result};
use colored::Colorize;
use std::fs;
use std::path::{Path, PathBuf};

use crate::i18n::{format_t, T};

/// Backup file written next to the live binary before each swap. `--rollback`
/// restores from this. Overwritten on every upgrade, so it always holds the
/// immediately previous version.
const BAK_SUFFIX: &str = ".bak";

/// Extract the `nvm` (or `nvm.exe`) binary from the release archive.
/// The archive contains just the binary at the root (release.yml packages
/// with `tar -C target/.../release nvm`), so we extract into `dest_dir`
/// and return the path to the binary inside it.
pub(crate) fn extract_binary(archive_path: &Path, dest_dir: &Path) -> Result<PathBuf> {
    fs::create_dir_all(dest_dir).context(T("cannot_create_dir"))?;
    let bin_name = if cfg!(windows) { "nvm.exe" } else { "nvm" };

    #[cfg(not(windows))]
    {
        // tar.gz extraction. The release archive is gzip-compressed tar.
        let f = fs::File::open(archive_path)
            .with_context(|| format!("{}: {}", T("write_failed"), archive_path.display()))?;
        let gz = flate2::read::GzDecoder::new(f);
        let mut tar = tar::Archive::new(gz);
        tar.unpack(dest_dir).context(T("extract_failed"))?;
    }
    #[cfg(windows)]
    {
        // zip extraction. `zip` crate is already a dependency (used by extract.rs
        // for Windows node archives). Extract just the binary entry.
        let f = fs::File::open(archive_path)
            .with_context(|| format!("{}: {}", T("write_failed"), archive_path.display()))?;
        let mut za = zip::ZipArchive::new(f).context(T("extract_failed"))?;
        for i in 0..za.len() {
            let mut entry = za.by_index(i).context(T("extract_failed"))?;
            let name = entry.name().to_string();
            if name == bin_name || name.ends_with(&format!("/{}", bin_name)) {
                let out = dest_dir.join(bin_name);
                let mut out_file = fs::File::create(&out)
                    .with_context(|| format!("{}: {}", T("write_failed"), out.display()))?;
                std::io::copy(&mut entry, &mut out_file).context(T("write_failed"))?;
                break;
            }
        }
    }

    let bin = dest_dir.join(bin_name);
    if !bin.exists() {
        anyhow::bail!(
            "{}",
            format_t("upgrade_no_binary_in_archive", &[bin_name.to_string()])
        );
    }

    // chmod +x on Unix so the swapped-in binary is executable.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&bin)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&bin, perms)?;
    }

    Ok(bin)
}

/// Check if a directory is writable by attempting to create a temp file.
/// Returns false if the dir doesn't exist or is not writable.
/// Used to detect system paths like `/usr/local/bin` that require sudo.
pub(crate) fn is_dir_writable(dir: &Path) -> bool {
    if !dir.is_dir() {
        return false;
    }
    let tmp = dir.join(format!(".nvm_writable_test_{}", std::process::id()));
    match fs::File::create(&tmp) {
        Ok(_) => {
            let _ = fs::remove_file(&tmp);
            true
        }
        Err(_) => false,
    }
}

/// Swap `new_bin` into `bin_path`, backing up the current binary to
/// `bin_path.bak` first.
///
/// On Unix, `rename` over an existing file atomically replaces it, and the
/// kernel keeps the old inode alive for the running process — so the
/// currently-executing nvm keeps running until the user starts a new shell.
///
/// On Windows, the running exe is locked and cannot be overwritten. We
/// rename the current binary to `.bak` first (Windows allows renaming a
/// locked file as long as we don't delete it), then move the new one in.
///
/// If `bin_path`'s parent dir is not writable (system path like
/// `/usr/local/bin`), the three-step swap can't write temp/backup files
/// there. Instead, on Unix we fall back to `sudo cp`; on Windows we print
/// instructions to run as admin.
pub(crate) fn swap_binary(bin_path: &Path, new_bin: &Path) -> Result<()> {
    // Append `.bak` to the full filename (e.g. `nvm` → `nvm.bak`,
    // `nvm.exe` → `nvm.exe.bak`). Using `with_extension` would mangle the
    // `.exe` on Windows, so we append to the file name directly.
    let bak = bin_path.parent().unwrap_or(Path::new(".")).join(format!(
        "{}{}",
        bin_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        BAK_SUFFIX
    ));
    // Crash recovery marker: written to the user bin dir
    // (`~/.nvm.rust/bin/.swap-pending`) which is always user-writable, so
    // crash recovery works even when the binary lives in a system path like
    // `/usr/local/bin` (where we can't write the marker next to the binary).
    let pending = crate::system::get_nvm_dir()
        .join("bin")
        .join(".swap-pending");

    // If the binary's parent dir is not writable (system path), skip the
    // three-step swap. No auto-sudo — print explicit manual instructions so
    // the user can run the sudo cp themselves (avoids unexpected password
    // prompts, root-owned files, and non-interactive failures).
    let bin_parent = bin_path.parent().unwrap_or(Path::new("."));
    if !is_dir_writable(bin_parent) {
        eprintln!(
            "  {} System path not writable. To update manually:",
            "⚠".yellow().bold()
        );
        eprintln!(
            "    sudo cp -f {} {} && sudo chmod 755 {}",
            new_bin.display(),
            bin_path.display(),
            bin_path.display()
        );
        return Ok(());
    }

    println!(
        "  {} {} → {}",
        T("upgrade_backing_up").dimmed(),
        bin_path.display(),
        bak.display()
    );

    #[cfg(unix)]
    {
        // Three-step atomic swap that survives both cross-device (EXDEV) and
        // mid-swap crashes:
        //   1. copy new_bin → bin_path.tmp  (same dir as bin_path → same fs)
        //   2. write .swap-pending marker
        //   3. rename bin_path → bak        (atomic, same fs)
        //   4. rename bin_path.tmp → bin_path (atomic, same fs)
        //   5. remove .swap-pending marker
        // If we crash between 3 and 4, bin_path.tmp is still on disk and a
        // retry (or the user) can finish the swap; bin_path is gone but the
        // .bak holds the previous version.
        use std::os::unix::fs::PermissionsExt;
        let tmp = bin_path.with_extension("tmp");
        fs::copy(new_bin, &tmp)
            .with_context(|| format!("{}: {}", T("upgrade_swap_failed"), tmp.display()))?;
        // fs::copy applies the source file's mode on Unix, but be explicit so
        // a 0o644 source never yields a non-executable nvm.
        fs::set_permissions(&tmp, fs::Permissions::from_mode(0o755))
            .with_context(|| format!("{}: {}", T("upgrade_swap_failed"), tmp.display()))?;
        // Write marker BEFORE step 3 so a crash between 3 and 4 is detectable.
        let _ = fs::write(&pending, b"pending");
        if bin_path.exists() {
            fs::rename(bin_path, &bak)
                .with_context(|| format!("{}: {}", T("upgrade_backup_failed"), bak.display()))?;
        }
        fs::rename(&tmp, bin_path)
            .with_context(|| format!("{}: {}", T("upgrade_swap_failed"), bin_path.display()))?;
        // Swap completed — remove marker.
        let _ = fs::remove_file(&pending);
    }
    #[cfg(windows)]
    {
        // Windows: rename the locked exe to .bak (allowed), then move new in.
        let _ = fs::write(&pending, b"pending");
        if bin_path.exists() {
            // If a previous .bak exists, remove it first (rename won't overwrite).
            let _ = fs::remove_file(&bak);
            fs::rename(bin_path, &bak)
                .with_context(|| format!("{}: {}", T("upgrade_backup_failed"), bak.display()))?;
        }
        // On Windows, `fs::rename` across the same volume works for files.
        // If the new binary is on a different volume, fall back to copy+delete.
        if fs::rename(new_bin, bin_path).is_err() {
            fs::copy(new_bin, bin_path)
                .with_context(|| format!("{}: {}", T("upgrade_swap_failed"), bin_path.display()))?;
            let _ = fs::remove_file(new_bin);
        }
        // Swap completed — remove marker.
        let _ = fs::remove_file(&pending);
    }

    Ok(())
}

/// Check for and recover an interrupted swap_binary() operation.
/// Called from main() on startup. If `.swap-pending` exists in the user
/// bin dir (`~/.nvm.rust/bin/`), a previous upgrade crashed mid-swap. Try
/// to recover:
/// - If `nvm.tmp` exists → rename it to `nvm` (finish step 4)
/// - If `nvm.bak` exists and `nvm` doesn't → rename `bak` back to `nvm`
/// - Clean up the marker
///
/// The marker and recovery files live in the user bin dir (always
/// user-writable) so recovery works even when the binary itself is in a
/// system path like `/usr/local/bin` (where we can't write temp files).
pub fn check_swap_recovery() {
    // The marker is always written to the user bin dir, not next to the
    // binary. This ensures crash recovery works regardless of where the
    // binary lives (system path or user path).
    let user_bin_dir = crate::system::get_nvm_dir().join("bin");
    let pending = user_bin_dir.join(".swap-pending");
    if !pending.exists() {
        return;
    }

    eprintln!(
        "{} Detected interrupted upgrade, attempting recovery...",
        "⚠".yellow().bold()
    );

    let bin_name = if cfg!(windows) { "nvm.exe" } else { "nvm" };
    let bin_path = user_bin_dir.join(bin_name);
    let tmp_path = user_bin_dir.join("nvm.tmp");
    let bak_path = user_bin_dir.join(format!("{}{}", bin_name, BAK_SUFFIX));

    // Case 1: nvm.tmp exists → swap was interrupted at step 4, finish it
    if tmp_path.exists() && !bin_path.exists() {
        match fs::rename(&tmp_path, &bin_path) {
            Ok(()) => {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = fs::set_permissions(&bin_path, fs::Permissions::from_mode(0o755));
                }
                eprintln!("{} Recovered: renamed nvm.tmp → nvm", "✓".green().bold());
            }
            Err(e) => {
                eprintln!("{} Failed to recover from nvm.tmp: {}", "✗".red().bold(), e);
            }
        }
    }
    // Case 2: nvm.bak exists, nvm doesn't → swap was interrupted at step 3, restore old
    else if bak_path.exists() && !bin_path.exists() {
        match fs::rename(&bak_path, &bin_path) {
            Ok(()) => {
                eprintln!("{} Recovered: restored nvm from .bak", "✓".green().bold());
            }
            Err(e) => {
                eprintln!("{} Failed to recover from .bak: {}", "✗".red().bold(), e);
            }
        }
    }
    // Case 3: nvm exists → swap actually completed, marker was just left behind
    else if bin_path.exists() {
        eprintln!(
            "{} Binary exists, swap was completed — cleaning up marker",
            "✓".green().bold()
        );
    }

    // Clean up marker regardless
    let _ = fs::remove_file(&pending);
}

/// Restore `nvm.bak` over the live binary.
///
/// Fails with a clear message if no backup exists (e.g. this is the first
/// install, or the user deleted `.bak`). The restored binary becomes the
/// new live binary; the current live binary is NOT saved as a new `.bak`
/// (rollback is one-shot — repeated rollback would just toggle back to the
/// version the user just escaped from).
pub(crate) fn rollback_binary(bin_path: &Path) -> Result<()> {
    let bak = bin_path.parent().unwrap_or(Path::new(".")).join(format!(
        "{}{}",
        bin_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        BAK_SUFFIX
    ));

    if !bak.exists() {
        anyhow::bail!(
            "{}",
            format_t(
                "upgrade_no_backup",
                std::slice::from_ref(&bak.display().to_string())
            )
        );
    }

    println!(
        "  {} {} → {}",
        T("upgrade_restoring").dimmed(),
        bak.display(),
        bin_path.display()
    );

    #[cfg(unix)]
    {
        // Overwrite the live binary with the backup. Atomic on Unix.
        fs::rename(&bak, bin_path)
            .with_context(|| format!("{}: {}", T("upgrade_swap_failed"), bin_path.display()))?;
    }
    #[cfg(windows)]
    {
        // On Windows the running exe is locked; rename it out of the way first.
        if bin_path.exists() {
            if let Err(e) = fs::remove_file(bin_path) {
                eprintln!(
                    "  {} failed to remove current binary: {} — attempting rename anyway",
                    "⚠".yellow().bold(),
                    e
                );
            }
        }
        fs::rename(&bak, bin_path)
            .with_context(|| format!("{}: {}", T("upgrade_swap_failed"), bin_path.display()))?;
    }

    println!(
        "{}  {}",
        "✓".green().bold(),
        T("upgrade_rollback_done").green().bold()
    );
    println!(
        "  {} {}",
        T("tip_label").dimmed(),
        T("upgrade_restart_hint").dimmed()
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::ENV_TESTS_MUTEX;
    use std::env;
    use tempfile::TempDir;

    /// Guard that sets NVM_DIR to a temp dir for the test and restores the
    /// old value on drop. This prevents the marker write in swap_binary
    /// from touching the real ~/.nvm.rust/ directory.
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

    fn setup_temp_nvm_dir() -> NvmDirGuard {
        let mutex = ENV_TESTS_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let old_value = env::var("NVM_DIR").ok();
        let dir = tempfile::tempdir().expect("tempdir");
        env::set_var("NVM_DIR", dir.path());
        NvmDirGuard {
            old_value,
            _dir: dir,
            _mutex: mutex,
        }
    }

    #[test]
    fn test_swap_binary_creates_backup_and_replaces() {
        // Set NVM_DIR to a temp dir so the .swap-pending marker (now written
        // to the user bin dir) doesn't touch the real ~/.nvm.rust/.
        let _guard = setup_temp_nvm_dir();

        let dir = std::env::temp_dir();
        let bin_path = dir.join(format!("nvm_swap_test_{}", std::process::id()));
        let new_bin = dir.join(format!("nvm_swap_new_{}", std::process::id()));
        let bak_path = dir.join(format!("nvm_swap_test_{}.bak", std::process::id()));

        // Clean up any leftover files from previous runs
        std::fs::remove_file(&bin_path).ok();
        std::fs::remove_file(&new_bin).ok();
        std::fs::remove_file(&bak_path).ok();

        // Create original and new binaries
        std::fs::write(&bin_path, "original").expect("write original");
        std::fs::write(&new_bin, "replacement").expect("write replacement");

        // Swap: original → .bak, replacement → original
        swap_binary(&bin_path, &new_bin).expect("swap should succeed");

        // Verify: bin_path now has replacement content
        let content = std::fs::read_to_string(&bin_path).expect("read swapped");
        assert_eq!(
            content, "replacement",
            "bin_path should have replacement content"
        );

        // Verify: .bak file exists with original content
        let bak_content = std::fs::read_to_string(&bak_path).expect("read backup");
        assert_eq!(
            bak_content, "original",
            "bak file should have original content"
        );

        // Note: on Unix, swap_binary copies (not moves) new_bin to a temp
        // file, so new_bin may still exist. This is by design — the copy
        // is more robust across filesystems than rename. Don't assert
        // new_bin is gone.

        // Clean up
        std::fs::remove_file(&bin_path).ok();
        std::fs::remove_file(&bak_path).ok();
        std::fs::remove_file(&new_bin).ok();
    }

    #[test]
    fn test_is_dir_writable_true_for_writable_dir() {
        let _guard = setup_temp_nvm_dir();
        let dir = std::env::temp_dir();
        assert!(dir.is_dir(), "temp dir should exist");
        assert!(
            is_dir_writable(&dir),
            "is_dir_writable should return true for a writable dir"
        );
        // Verify the probe temp file was cleaned up
        let probe = dir.join(format!(".nvm_writable_test_{}", std::process::id()));
        assert!(
            !probe.exists(),
            "is_dir_writable should clean up its probe temp file"
        );
    }

    #[test]
    fn test_is_dir_writable_false_for_nonexistent_dir() {
        let _guard = setup_temp_nvm_dir();
        let fake_dir = std::path::Path::new("/nonexistent/nvm/writable/test");
        assert!(
            !is_dir_writable(fake_dir),
            "is_dir_writable should return false for a non-existent dir"
        );
    }
}
