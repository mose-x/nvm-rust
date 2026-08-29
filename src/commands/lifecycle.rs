//! Lifecycle commands: `deactivate`, `unload`, `uninstall-self`, `uninstall-all`.
//!
//! These manage the nvm installation itself -- clearing the active version,
//! removing shims, or wiping the whole installation. They are destructive and
//! require confirmation where appropriate.

use anyhow::{Context, Result};
use colored::Colorize;
use std::fs;
use std::io::Write;

use crate::config::remove_from_shell_config;
use crate::i18n::{format_t, T};
use crate::system::get_nvm_dir;
use crate::utils::atomic_write;

/// Returns true if `user_bin` is a symlink pointing to `system_bin`, meaning
/// the system binary is ours (EDR-safe layout) and safe to remove. If
/// `user_bin` is a real file (old layout) or doesn't exist, returns false —
/// the system bin might be a symlink pointing the other way or belong to
/// another tool entirely.
#[cfg(unix)]
fn is_system_bin_ours(user_bin: &std::path::Path, system_bin: &std::path::Path) -> bool {
    std::fs::read_link(user_bin)
        .map(|target| target == system_bin)
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Windows self-delete helpers.
//
// Windows refuses to DELETE a running executable but allows RENAMING it.
// Uninstalling nvm with nvm itself running therefore always hits a sharing
// violation on the binary. Strategy: rename the locked file out of the way,
// delete everything else immediately, then spawn a detached, windowless cmd
// process that waits for us to exit and removes the leftovers. No reboot,
// no manual steps, no console window.
// ---------------------------------------------------------------------------

#[cfg(windows)]
const PENDING_DELETE_SUFFIX: &str = ".pending-delete";
/// CREATE_NO_WINDOW | DETACHED_PROCESS — the cleanup cmd must never show a
/// console window to the user.
#[cfg(windows)]
const CLEANUP_SPAWN_FLAGS: u32 = 0x0800_0000 | 0x0000_0008;

/// Batch script that waits for `pid` to exit, then deletes `target`
/// (directory when `is_dir`, single file otherwise) and removes itself.
/// Uses `ping` for the sleep because `timeout` aborts immediately when
/// stdin is redirected (which it is for a detached process).
#[cfg(windows)]
fn build_cleanup_script(pid: u32, target: &std::path::Path, is_dir: bool) -> String {
    let remove_cmd = if is_dir {
        format!("rd /s /q \"{}\"", target.display())
    } else {
        format!("del /q \"{}\"", target.display())
    };
    format!(
        "@echo off\r\n\
         :wait\r\n\
         tasklist /FI \"PID eq {pid}\" /NH 2>nul | find \"{pid} \" >nul\r\n\
         if not errorlevel 1 (\r\n\
         \x20   ping -n 2 127.0.0.1 >nul\r\n\
         \x20   goto wait\r\n\
         )\r\n\
         {remove_cmd}\r\n\
         del /q \"%~f0\"\r\n",
    )
}

/// Write the cleanup script to %TEMP% and spawn it detached + windowless.
/// Returns false when the script could not be written or spawned.
#[cfg(windows)]
fn schedule_cleanup(target: &std::path::Path, is_dir: bool) -> bool {
    let script = build_cleanup_script(std::process::id(), target, is_dir);
    let script_path = std::env::temp_dir().join(format!("nvm-cleanup-{}.cmd", std::process::id()));
    if fs::write(&script_path, script).is_err() {
        return false;
    }
    let Some(script_str) = script_path.to_str() else {
        return false;
    };
    use std::os::windows::process::CommandExt;
    std::process::Command::new("cmd.exe")
        .args(["/c", script_str])
        .creation_flags(CLEANUP_SPAWN_FLAGS)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .is_ok()
}

/// `current_exe()` may carry a `\\?\` prefix; strip it so path-prefix
/// comparisons against plain paths work.
#[cfg(windows)]
fn current_exe_plain() -> Option<std::path::PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let s = exe.to_string_lossy();
    let stripped = s.strip_prefix(r"\\?\").unwrap_or(&s);
    Some(std::path::PathBuf::from(stripped))
}

/// Rename a locked file (running binary) to `*.pending-delete`. Renaming is
/// permitted by Windows even while the file is locked. Returns the new path.
#[cfg(windows)]
fn rename_locked(path: &std::path::Path) -> Option<std::path::PathBuf> {
    let name = path.file_name()?.to_string_lossy().into_owned();
    let renamed = path.with_file_name(format!("{name}{PENDING_DELETE_SUFFIX}"));
    let _ = fs::remove_file(&renamed); // rename refuses to overwrite
    fs::rename(path, &renamed).ok().map(|_| renamed)
}

/// Remove a directory on Windows, tolerating the running binary being locked
/// inside it. Falls back to the detached cleanup process; only errors when
/// even that cannot be arranged.
#[cfg(windows)]
fn remove_dir_windows_best_effort(dir: &std::path::Path) -> Result<()> {
    if fs::remove_dir_all(dir).is_ok() {
        return Ok(());
    }
    // Isolate the running binary if it lives inside `dir`, then retry —
    // everything except the renamed (still locked) file goes away now.
    if let Some(exe) = current_exe_plain() {
        if exe.starts_with(dir) {
            let _ = rename_locked(&exe);
        }
    }
    if fs::remove_dir_all(dir).is_ok() {
        return Ok(());
    }
    if schedule_cleanup(dir, true) {
        println!(
            "  {} {}",
            "ℹ".cyan().bold(),
            T("uninstall_background_cleanup")
        );
        return Ok(());
    }
    anyhow::bail!(
        "failed to remove {} and could not schedule background cleanup",
        dir.display()
    )
}

/// Remove a single file on Windows, tolerating it being the running binary.
#[cfg(windows)]
fn remove_file_windows_best_effort(path: &std::path::Path) {
    if fs::remove_file(path).is_ok() || !path.exists() {
        return;
    }
    if let Some(renamed) = rename_locked(path) {
        if schedule_cleanup(&renamed, false) {
            println!(
                "  {} {}",
                "ℹ".cyan().bold(),
                T("uninstall_background_cleanup")
            );
            return;
        }
    }
    eprintln!(
        "  {} could not remove {} — delete it manually after closing this terminal",
        "⚠".yellow().bold(),
        path.display()
    );
}

pub fn deactivate() -> Result<()> {
    let nvm_dir = get_nvm_dir();
    let current_file = nvm_dir.join("current");
    // Write "none" marker instead of deleting the file. This prevents shims
    // from auto-recovering (calling `nvm auto --silent`) after deactivate --
    // the shim reads "none" and exits with an error instead of trying to
    // find a version. `nvm use <version>` overwrites the marker, restoring
    // normal operation.
    if let Err(e) = atomic_write(&current_file, "none") {
        eprintln!(
            "{} failed to write 'none' marker: {} -- shims may still resolve the old version",
            "⚠".yellow().bold(),
            e
        );
    }
    // Remove active symlink so global packages stop resolving via active/bin
    if let Err(e) = crate::shim::remove_active_symlink(&nvm_dir) {
        eprintln!(
            "  {} failed to remove active symlink: {}",
            "⚠".yellow().bold(),
            e
        );
    }
    println!("{} {}", "✓".green().bold(), T("deactivated").green());
    Ok(())
}

pub fn unload() -> Result<()> {
    let nvm_dir = get_nvm_dir();
    // Remove shims directory so node/npm/etc. stop resolving via nvm.
    // Warn on error -- if shims can't be removed (permission denied, Windows
    // file lock), the user needs to know the shell rc was cleaned but shims
    // are still active (inconsistent state).
    if let Err(e) = crate::shim::remove_shims() {
        eprintln!("{} nvm: failed to remove shims: {}", "⚠".yellow().bold(), e);
    }
    // Clear current version file.
    let current_file = nvm_dir.join("current");
    if let Err(e) = fs::remove_file(&current_file) {
        if e.kind() != std::io::ErrorKind::NotFound {
            eprintln!(
                "{} failed to remove current file: {} -- shims may still resolve the old version",
                "⚠".yellow().bold(),
                e
            );
        }
    }
    remove_from_shell_config()
}

/// Remove nvm itself: binary, nvm.sh, shims, shell config.
/// Keeps all installed Node versions, config.json, alias.json, cache, completions.
/// Requires y/N confirmation from stdin.
pub fn uninstall_self() -> Result<()> {
    let nvm_dir = get_nvm_dir();
    let nvm_dir_str = nvm_dir.display().to_string();

    // Confirmation
    print!("{} ", T("uninstall_self_confirm"));
    std::io::stdout().flush().ok();
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    if !input.trim().eq_ignore_ascii_case("y") {
        println!("{}", T("uninstall_cancelled"));
        return Ok(());
    }

    // Remove shims
    if let Err(e) = crate::shim::remove_shims() {
        eprintln!("{} failed to remove shims: {}", "⚠".yellow().bold(), e);
    }

    // Remove active symlink (Full Shim mode)
    if let Err(e) = crate::shim::remove_active_symlink(&nvm_dir) {
        eprintln!(
            "{} failed to remove active symlink: {}",
            "⚠".yellow().bold(),
            e
        );
    }

    // Remove current file
    let current_file = nvm_dir.join("current");
    let _ = fs::remove_file(&current_file);

    // Clean shell config
    crate::config::remove_from_shell_config()?;

    // Remove nvm.sh
    let nvm_sh = nvm_dir.join("bin").join("nvm.sh");
    let _ = fs::remove_file(&nvm_sh);

    // Remove /usr/local/bin/nvm — only if it's provably ours: the user-dir
    // copy must be a symlink pointing to it (EDR-safe layout). Must check
    // BEFORE removing the user binary, since read_link needs the symlink to
    // still exist. If the user-dir copy is a real file (old layout), leave
    // /usr/local/bin/nvm alone — it might point the other way or be another
    // tool's binary entirely.
    #[cfg(unix)]
    {
        let system_bin = std::path::Path::new("/usr/local/bin/nvm");
        let user_bin = nvm_dir.join("bin").join("nvm");
        if system_bin.exists() {
            let is_ours = is_system_bin_ours(&user_bin, system_bin);
            if is_ours && fs::remove_file(system_bin).is_err() {
                eprintln!(
                    "  {} /usr/local/bin/nvm may be root-owned. Remove: sudo rm -f /usr/local/bin/nvm",
                    "⚠".yellow().bold()
                );
            }
            // If not ours, leave it alone — could be another tool's binary.
        }
    }

    // Remove nvm binary
    let bin_name = if cfg!(windows) { "nvm.exe" } else { "nvm" };
    let nvm_bin = nvm_dir.join("bin").join(bin_name);
    #[cfg(windows)]
    remove_file_windows_best_effort(&nvm_bin);
    #[cfg(not(windows))]
    let _ = fs::remove_file(&nvm_bin);

    #[cfg(windows)]
    {
        let system_dir = std::path::Path::new(&std::env::var("ProgramFiles").unwrap_or_default())
            .join("nvm-rust");
        if system_dir.exists() && fs::remove_dir_all(&system_dir).is_err() {
            eprintln!(
                "  {} Cannot remove {} (needs admin)",
                "⚠".yellow().bold(),
                system_dir.display()
            );
        }
    }

    println!(
        "{} {}",
        "✓".green().bold(),
        format_t("uninstall_self_done", std::slice::from_ref(&nvm_dir_str))
    );
    println!("  {} reinstall: curl -fsSL https://raw.githubusercontent.com/mose-x/nvm-rust/main/install.sh | bash", T("tip_label").dimmed());
    Ok(())
}

/// Remove everything: nvm binary, nvm.sh, shims, all Node versions,
/// config.json, alias.json, cache, completions, shell config.
/// Requires y/N confirmation from stdin.
pub fn uninstall_all() -> Result<()> {
    let nvm_dir = get_nvm_dir();

    // Confirmation
    print!("{} ", T("uninstall_all_confirm"));
    std::io::stdout().flush().ok();
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    if !input.trim().eq_ignore_ascii_case("y") {
        println!("{}", T("uninstall_cancelled"));
        return Ok(());
    }

    // Clean shell config first (before removing nvm dir, so remove_from_shell_config
    // can still read the nvm dir path for stripping)
    crate::config::remove_from_shell_config()?;

    // Remove /usr/local/bin/nvm — only if it's provably ours: the user-dir
    // copy must be a symlink pointing to it (EDR-safe layout). Must check
    // BEFORE removing the nvm directory, since read_link needs the symlink
    // to still exist. If not ours, leave it alone.
    #[cfg(unix)]
    {
        let system_bin = std::path::Path::new("/usr/local/bin/nvm");
        let user_bin = nvm_dir.join("bin").join("nvm");
        if system_bin.exists() {
            let is_ours = is_system_bin_ours(&user_bin, system_bin);
            if is_ours && fs::remove_file(system_bin).is_err() {
                eprintln!(
                    "  {} /usr/local/bin/nvm may be root-owned. Remove: sudo rm -f /usr/local/bin/nvm",
                    "⚠".yellow().bold()
                );
            }
            // If not ours, leave it alone — could be another tool's binary.
        }
    }

    // Remove the entire ~/.nvm.rust/ directory
    // This removes: binary, nvm.sh, shims, all v* version dirs, config.json,
    // alias.json, cache/, completions/, current, .nvm.lock
    if nvm_dir.exists() {
        #[cfg(windows)]
        remove_dir_windows_best_effort(&nvm_dir).context("failed to remove nvm directory")?;
        #[cfg(not(windows))]
        fs::remove_dir_all(&nvm_dir).context("failed to remove nvm directory")?;
    }

    #[cfg(windows)]
    {
        let system_dir = std::path::Path::new(&std::env::var("ProgramFiles").unwrap_or_default())
            .join("nvm-rust");
        if system_dir.exists() && fs::remove_dir_all(&system_dir).is_err() {
            eprintln!(
                "  {} Cannot remove {} (needs admin)",
                "⚠".yellow().bold(),
                system_dir.display()
            );
        }
    }

    println!("{} {}", "✓".green().bold(), T("uninstall_all_done"));
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    // EDR-safe layout: user_bin is a symlink to system_bin → ours.
    #[cfg(unix)]
    #[test]
    fn test_is_system_bin_ours_symlink_points_to_system() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let user_bin = tmp.path().join("nvm");
        let system_bin = std::path::Path::new("/usr/local/bin/nvm");
        std::os::unix::fs::symlink(system_bin, &user_bin).expect("create symlink");
        assert!(super::is_system_bin_ours(&user_bin, system_bin));
    }

    // Old layout: user_bin is a real file, not a symlink → not ours.
    #[cfg(unix)]
    #[test]
    fn test_is_system_bin_ours_real_file_not_symlink() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let user_bin = tmp.path().join("nvm");
        std::fs::write(&user_bin, b"binary").expect("write file");
        let system_bin = std::path::Path::new("/usr/local/bin/nvm");
        assert!(!super::is_system_bin_ours(&user_bin, system_bin));
    }

    // user_bin doesn't exist → can't be ours.
    #[cfg(unix)]
    #[test]
    fn test_is_system_bin_ours_nonexistent_file() {
        let user_bin = std::path::Path::new("/nonexistent/nvm");
        let system_bin = std::path::Path::new("/usr/local/bin/nvm");
        assert!(!super::is_system_bin_ours(user_bin, system_bin));
    }

    // user_bin is a symlink but points somewhere else → not ours.
    #[cfg(unix)]
    #[test]
    fn test_is_system_bin_ours_symlink_points_elsewhere() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let user_bin = tmp.path().join("nvm");
        let other_target = std::path::Path::new("/some/other/path");
        std::os::unix::fs::symlink(other_target, &user_bin).expect("create symlink");
        let system_bin = std::path::Path::new("/usr/local/bin/nvm");
        assert!(!super::is_system_bin_ours(&user_bin, system_bin));
    }

    // --- Windows self-delete helpers ------------------------------------

    // Dir cleanup script: waits for the PID, then `rd /s /q` the quoted
    // directory, then deletes itself. CRLF line endings required by cmd.
    #[cfg(windows)]
    #[test]
    fn test_cleanup_script_dir_waits_then_removes() {
        let target = std::path::PathBuf::from(r"C:\Users\someone\.nvm.rust");
        let script = super::build_cleanup_script(4242, &target, true);
        assert!(script.starts_with("@echo off\r\n"));
        assert!(script.contains(":wait"), "needs the wait loop label");
        assert!(
            script.contains("tasklist /FI \"PID eq 4242\""),
            "must wait for our own PID"
        );
        assert!(
            script.contains(r#"rd /s /q "C:\Users\someone\.nvm.rust""#),
            "must remove the directory quoted"
        );
        assert!(script.contains("del /q \"%~f0\""), "must self-delete");
        // `timeout` breaks under redirected stdin — must use ping to sleep.
        assert!(script.contains("ping -n 2 127.0.0.1"));
        assert!(!script.contains("timeout /t"));
    }

    // File cleanup script (uninstall --self path) uses `del /q`, not `rd`.
    #[cfg(windows)]
    #[test]
    fn test_cleanup_script_file_uses_del() {
        let target = std::path::PathBuf::from(r"C:\Users\x\.nvm.rust\bin\nvm.exe.pending-delete");
        let script = super::build_cleanup_script(7, &target, false);
        assert!(script.contains(r#"del /q "C:\Users\x\.nvm.rust\bin\nvm.exe.pending-delete""#));
        assert!(!script.contains("rd /s /q"));
    }

    // Paths containing spaces must stay quoted end-to-end.
    #[cfg(windows)]
    #[test]
    fn test_cleanup_script_quotes_paths_with_spaces() {
        let target = std::path::PathBuf::from(r"C:\My Projects\nvm dir");
        let script = super::build_cleanup_script(1, &target, true);
        assert!(script.contains(r#"rd /s /q "C:\My Projects\nvm dir""#));
    }

    // rename_locked moves the file to `*.pending-delete`, replacing any
    // stale leftover from a previous interrupted uninstall.
    #[cfg(windows)]
    #[test]
    fn test_rename_locked_moves_and_replaces_stale() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let file = tmp.path().join("nvm.exe");
        std::fs::write(&file, b"binary").expect("write");
        // Stale leftover from a previous run.
        std::fs::write(file.with_file_name("nvm.exe.pending-delete"), b"old").expect("write");

        let renamed = super::rename_locked(&file).expect("rename");
        assert!(renamed.ends_with("nvm.exe.pending-delete"));
        assert!(!file.exists());
        assert_eq!(std::fs::read(&renamed).expect("read"), b"binary");
    }

    // Happy path: nothing locked → directory gone immediately, no cleanup
    // process needed.
    #[cfg(windows)]
    #[test]
    fn test_remove_dir_best_effort_deletes_unlocked_dir() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let dir = tmp.path().join("nvm.rust");
        std::fs::create_dir_all(dir.join("bin")).expect("mkdir");
        std::fs::write(dir.join("bin").join("nvm.exe"), b"x").expect("write");

        super::remove_dir_windows_best_effort(&dir).expect("remove");
        assert!(!dir.exists());
    }
}
