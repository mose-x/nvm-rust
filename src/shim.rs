use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

#[cfg(windows)]
use colored::Colorize;

#[cfg(windows)]
use std::process::Command;

use crate::system::get_nvm_dir;

/// Commands that get a shim. The first four ship with every Node.js
/// distribution (bin/ directory). The remaining four are corepack-managed
/// tools that appear in bin/ after `nvm corepack enable`. All are covered
/// by the same shim script via `basename "$0"` at runtime, so they must
/// be listed here to ensure the shim files are actually created on disk.
pub const SHIM_COMMANDS: &[&str] = &[
    "node", "npm", "npx", "corepack", "pnpm", "pnpx", "yarn", "yarnpkg",
];

/// The Unix shim script. Self-contained — does not depend on the nvm binary
/// having "shim mode". Reads `current` file, execs the real binary.
/// If the version is missing, calls `nvm auto --silent` (which the old binary
/// also supports) to auto-recover, then retries.
fn unix_shim_script() -> &'static str {
    r#"#!/bin/sh
NVM_DIR="${NVM_DIR:-$HOME/.nvm.rust}"
CMD=$(basename "$0")
read_current() {
    cat "$NVM_DIR/current" 2>/dev/null | tr -d '[:space:]'
}
CURRENT=$(read_current)
if [ "$CURRENT" = "none" ]; then
    echo "nvm: deactivated. Run 'nvm use <version>' to reactivate." >&2
    exit 1
fi
# Reject path traversal: a poisoned `current` file could contain `../../tmp/evil`.
# validate_version_name() guards the write path, but the shim reads the file
# directly, so this is defense-in-depth.
case "$CURRENT" in
    *..*|*/*|*\\*)
        echo "nvm: corrupted current file. Run 'nvm use <version>' to fix." >&2
        exit 1
        ;;
esac
if [ -z "$CURRENT" ] || [ ! -x "$NVM_DIR/$CURRENT/bin/$CMD" ]; then
    "$NVM_DIR/bin/nvm" auto --silent 2>/dev/null
    CURRENT=$(read_current)
fi
if [ -z "$CURRENT" ] || [ "$CURRENT" = "none" ] || [ ! -x "$NVM_DIR/$CURRENT/bin/$CMD" ]; then
    echo "nvm: $CMD not found. Run 'nvm use <version>' or 'nvm install <version>'." >&2
    exit 1
fi
exec "$NVM_DIR/$CURRENT/bin/$CMD" "$@"
"#
}

/// The Windows shim batch script. Same logic as Unix but in batch syntax.
/// Checks for .exe first (node), then .cmd (npm/npx/corepack).
fn windows_shim_script() -> &'static str {
    r#"@echo off
setlocal
set NVM_DIR=%USERPROFILE%\.nvm.rust
set CMD=%~n0
set CURRENT=
if exist "%NVM_DIR%\current" for /f "delims=" %%a in (%NVM_DIR%\current) do set CURRENT=%%a
if "%CURRENT%"=="none" (
    echo nvm: deactivated. Run 'nvm use ^<version^>' to reactivate.
    exit /b 1
)
REM Defense-in-depth: reject path traversal in CURRENT (matches Unix shim guard)
if not "%CURRENT%"=="" (
    echo %CURRENT% | findstr /C:".." >nul && (
        echo nvm: invalid current version
        exit /b 1
    )
)
call :resolve
if not defined BIN (
    "%NVM_DIR%\bin\nvm.exe" auto --silent 2>nul
    set CURRENT=
    if exist "%NVM_DIR%\current" for /f "delims=" %%a in (%NVM_DIR%\current) do set CURRENT=%%a
    call :resolve
)
if not defined BIN (
    echo nvm: %CMD% not found. Run 'nvm use ^<version^>' or 'nvm install ^<version^>'.
    exit /b 1
)
"%BIN%" %*
goto :eof

:resolve
set BIN=
if not "%CURRENT%"=="" (
    if exist "%NVM_DIR%\%CURRENT%\bin\%CMD%.exe" set "BIN=%NVM_DIR%\%CURRENT%\bin\%CMD%.exe"
    if not defined BIN if exist "%NVM_DIR%\%CURRENT%\bin\%CMD%.cmd" set "BIN=%NVM_DIR%\%CURRENT%\bin\%CMD%.cmd"
)
goto :eof
"#
}

/// Create shim scripts for all SHIM_COMMANDS in `~/.nvm.rust/shims/`.
/// Idempotent — safe to call on every install/upgrade.
/// On Windows, creates `.cmd` files. On Unix, creates extensionless scripts
/// and chmods them executable.
pub fn create_shims() -> Result<()> {
    let nvm_dir = get_nvm_dir();
    let shims_dir = nvm_dir.join("shims");
    fs::create_dir_all(&shims_dir).context("failed to create shims directory")?;

    let (script, ext): (&str, &str) = if cfg!(target_os = "windows") {
        (windows_shim_script(), "cmd")
    } else {
        (unix_shim_script(), "")
    };

    for cmd in SHIM_COMMANDS {
        let shim_name = if ext.is_empty() {
            (*cmd).to_string()
        } else {
            format!("{}.{}", cmd, ext)
        };
        let shim_path = shims_dir.join(&shim_name);
        fs::write(&shim_path, script)
            .with_context(|| format!("failed to write shim: {}", shim_name))?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&shim_path, fs::Permissions::from_mode(0o755))
                .with_context(|| format!("failed to chmod shim: {}", shim_name))?;
        }
    }

    Ok(())
}

/// Remove all shim scripts. Used by `nvm unload`.
pub fn remove_shims() -> Result<()> {
    let shims_dir = get_nvm_dir().join("shims");
    if shims_dir.exists() {
        fs::remove_dir_all(&shims_dir).context("failed to remove shims directory")?;
    }
    Ok(())
}

/// Check if shims are already set up (directory exists with at least node).
pub fn shims_exist() -> bool {
    let shims_dir = get_nvm_dir().join("shims");
    let node_shim = if cfg!(target_os = "windows") {
        shims_dir.join("node.cmd")
    } else {
        shims_dir.join("node")
    };
    shims_dir.is_dir() && node_shim.exists()
}

/// One-time migration from old wrapper-based setup to shims.
/// Called by `nvm upgrade` after downloading the new binary.
/// Creates shims if they don't exist. Does NOT modify rc files —
/// that's handled by `update_shell_config` which already knows how to
/// add/remove PATH entries.
pub fn migrate_to_shims() -> Result<()> {
    if shims_exist() {
        return Ok(());
    }
    create_shims()?;
    // Ensure `current` file is up to date — if the user had an active version
    // before upgrade, `current` already has it. If not, try to pick the latest
    // installed version so shims work immediately after migration.
    let nvm_dir = get_nvm_dir();
    let current_file = nvm_dir.join("current");
    if !current_file.exists() {
        if let Some(latest) = list_installed_versions()?.into_iter().max() {
            crate::utils::atomic_write(&current_file, &latest)?;
        }
    }
    Ok(())
}

/// List installed version directories (e.g., ["v18.0.0", "v20.0.0"]).
/// Reads directory names from `~/.nvm.rust/` that start with "v".
fn list_installed_versions() -> Result<Vec<String>> {
    let nvm_dir = get_nvm_dir();
    let mut versions = Vec::new();
    if !nvm_dir.is_dir() {
        return Ok(versions);
    }
    for entry in fs::read_dir(&nvm_dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = match name.to_str() {
            Some(n) => n,
            None => continue,
        };
        // Version directories are named like "v20.0.0" or "v18.0.0".
        // Skip non-directory entries and non-version files.
        if entry.file_type()?.is_dir() && name.starts_with('v') && name.len() > 1 {
            let rest = &name[1..];
            if rest
                .chars()
                .next()
                .map(|c| c.is_ascii_digit())
                .unwrap_or(false)
            {
                versions.push(name.to_string());
            }
        }
    }
    Ok(versions)
}

/// Get the next available version to switch to (highest semver).
/// Used by `uninstall` when the active version is being removed.
pub fn next_available_version(exclude: &str) -> Option<String> {
    let versions = list_installed_versions().ok()?;
    versions
        .into_iter()
        .filter(|v| v != exclude)
        .max_by(|a, b| crate::utils::compare_semver(a, b))
}

// ---------------------------------------------------------------------------
// Full Shim mode: active symlink management + migration
// ---------------------------------------------------------------------------

/// Create `~/.nvm.rust/active` symlink → version directory.
/// On Unix: atomic temp-symlink-then-rename. On Windows: junction (no admin).
pub fn create_active_symlink(nvm_dir: &Path, version: &str) -> Result<()> {
    let target = nvm_dir.join(version);
    let link = nvm_dir.join("active");

    #[cfg(unix)]
    {
        // Atomic: create temp symlink, rename over existing.
        let tmp = nvm_dir.join("active.tmp");
        let _ = fs::remove_file(&tmp);
        std::os::unix::fs::symlink(&target, &tmp)
            .with_context(|| format!("failed to create active symlink: {}", tmp.display()))?;
        fs::rename(&tmp, &link)
            .with_context(|| format!("failed to rename active symlink: {}", link.display()))?;
    }

    #[cfg(windows)]
    {
        // Junction: remove old, create new. `mklink /J` doesn't need admin.
        // Use remove_file (not remove_dir) — it correctly removes junctions
        // without following the reparse point. remove_dir could follow the
        // junction and attempt to delete the target directory's contents.
        let _ = fs::remove_file(&link);
        let status = Command::new("cmd")
            .args([
                "/C",
                "mklink",
                "/J",
                &link.display().to_string(),
                &target.display().to_string(),
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .with_context(|| format!("failed to create junction: {}", link.display()))?;
        if !status.success() {
            anyhow::bail!("mklink /J failed for active junction");
        }
    }

    Ok(())
}

/// Update `active` symlink to point to a new version (idempotent overwrite).
pub fn update_active_symlink(nvm_dir: &Path, version: &str) -> Result<()> {
    create_active_symlink(nvm_dir, version)
}

/// Remove the `active` symlink (used by `nvm deactivate` and `nvm use system`).
pub fn remove_active_symlink(nvm_dir: &Path) -> Result<()> {
    let link = nvm_dir.join("active");
    // Use symlink_metadata to avoid following the symlink/junction.
    // This confirms the entry is a link, not a real directory that
    // a user might have created accidentally.
    match fs::symlink_metadata(&link) {
        Ok(meta) => {
            #[cfg(unix)]
            {
                if meta.file_type().is_symlink() {
                    fs::remove_file(&link)?;
                } else {
                    // Not a symlink — could be a real file or dir.
                    // Only remove if it's a file; don't touch real dirs.
                    if meta.file_type().is_file() {
                        fs::remove_file(&link)?;
                    }
                }
            }
            #[cfg(windows)]
            {
                // On Windows, junctions are reparse points. remove_file
                // correctly removes junctions (and symlinks) without
                // following them. Never use remove_dir — it could delete
                // a real directory's contents if the entry is not a junction.
                let _ = meta; // suppress unused on Windows
                if let Err(e) = fs::remove_file(&link) {
                    if e.kind() != std::io::ErrorKind::NotFound {
                        eprintln!(
                            "  {} failed to remove active junction: {}",
                            "⚠".yellow().bold(),
                            e
                        );
                    }
                }
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e).context("failed to stat active symlink"),
    }
    Ok(())
}

/// Check if the `active` symlink/junction exists.
pub fn active_exists(nvm_dir: &Path) -> bool {
    nvm_dir.join("active").exists()
}

/// Read the `current` file and return the version string.
/// Returns None if file is missing, empty, "none", or fails validation.
/// Validation prevents path escape (`../../tmp/evil`) and Windows cmd.exe
/// metacharacter injection — consistent with the shell shim's `case` guard.
fn read_current_version(nvm_dir: &Path) -> Option<String> {
    let current_file = nvm_dir.join("current");
    match fs::read_to_string(&current_file) {
        Ok(content) => {
            let v = content.trim();
            if v.is_empty() || v == "none" {
                None
            } else if crate::utils::validate_version_name(v).is_err() {
                // Poisoned current file — don't create a symlink to a
                // traversed/injected path. Treat as no active version.
                None
            } else {
                Some(v.to_string())
            }
        }
        Err(_) => None,
    }
}

/// Migrate to Full Shim mode. Idempotent and self-healing.
///
/// - First call (active doesn't exist): create symlink + migrate rc → returns Ok(true)
/// - Subsequent calls (active exists): fix stale symlink + re-migrate rc if needed → returns Ok(false)
/// - No active version (current = "none" or missing): skip → returns Ok(false)
pub fn migrate_to_full_shim(nvm_dir: &Path) -> Result<bool> {
    // 1. Read current version
    let current = match read_current_version(nvm_dir) {
        Some(v) if nvm_dir.join(&v).is_dir() => v,
        _ => return Ok(false), // No active version, skip
    };

    // 2. First migration: active doesn't exist
    if !active_exists(nvm_dir) {
        create_active_symlink(nvm_dir, &current)?;
        crate::config::migrate_rc_to_shim_mode()?;
        return Ok(true);
    }

    // 3. active exists: fix stale symlink (always update to match current)
    update_active_symlink(nvm_dir, &current)?;

    // 4. Check if rc was reverted to old format (e.g. by old nvm version)
    if crate::config::rc_has_version_specific_path()? {
        crate::config::migrate_rc_to_shim_mode()?;
    }

    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::fs;

    /// Guard that restores NVM_DIR to its original value when dropped,
    /// BEFORE releasing the ENV_TESTS_MUTEX. This prevents other tests
    /// from seeing a stale NVM_DIR pointing to a deleted temp dir.
    struct NvmDirGuard {
        old_value: Option<String>,
        _dir: tempfile::TempDir,
        _mutex: std::sync::MutexGuard<'static, ()>,
    }

    impl Drop for NvmDirGuard {
        fn drop(&mut self) {
            // Restore NVM_DIR BEFORE the mutex is released (fields drop
            // after Drop::drop returns, in declaration order: _mutex first,
            // then _dir, then old_value — but NVM_DIR is already restored
            // here, so other tests waiting on the mutex see the right value).
            match &self.old_value {
                Some(v) => env::set_var("NVM_DIR", v),
                None => env::remove_var("NVM_DIR"),
            }
        }
    }

    impl NvmDirGuard {
        fn path(&self) -> &std::path::Path {
            self._dir.path()
        }
    }

    fn setup_temp_nvm_dir() -> NvmDirGuard {
        let mutex = crate::system::ENV_TESTS_MUTEX
            .lock()
            .expect("ENV_TESTS_MUTEX poisoned");
        let old_value = env::var("NVM_DIR").ok();
        let dir = tempfile::tempdir().expect("tempdir");
        env::set_var("NVM_DIR", dir.path());
        NvmDirGuard {
            old_value,
            _dir: dir,
            _mutex: mutex,
        }
    }

    fn create_fake_version(nvm_dir: &std::path::Path, version: &str) {
        let version_dir = nvm_dir.join(version);
        fs::create_dir_all(&version_dir).expect("create version dir");
        let bin_dir = version_dir.join("bin");
        fs::create_dir_all(&bin_dir).expect("create bin dir");
        // Create a fake node binary
        let node_name = if cfg!(windows) { "node.exe" } else { "node" };
        fs::write(bin_dir.join(node_name), "fake").expect("create fake node");
    }

    #[test]
    fn test_list_installed_versions_empty() {
        let _guard = setup_temp_nvm_dir();
        let versions = list_installed_versions().expect("list");
        assert!(versions.is_empty());
    }

    #[test]
    fn test_list_installed_versions_finds_versions() {
        let guard = setup_temp_nvm_dir();
        create_fake_version(guard.path(), "v18.0.0");
        create_fake_version(guard.path(), "v20.0.0");
        // Create a non-version dir that should be skipped
        fs::create_dir(guard.path().join("cache")).ok();
        fs::create_dir(guard.path().join("shims")).ok();

        let versions = list_installed_versions().expect("list");
        assert!(versions.contains(&"v18.0.0".to_string()));
        assert!(versions.contains(&"v20.0.0".to_string()));
        assert!(!versions.contains(&"cache".to_string()));
        assert!(!versions.contains(&"shims".to_string()));
    }

    #[test]
    fn test_next_available_version_picks_highest() {
        let guard = setup_temp_nvm_dir();
        create_fake_version(guard.path(), "v18.0.0");
        create_fake_version(guard.path(), "v20.0.0");
        create_fake_version(guard.path(), "v19.0.0");

        let next = next_available_version("v20.0.0");
        assert_eq!(next, Some("v19.0.0".to_string())); // 19 > 18

        let next = next_available_version("v18.0.0");
        assert_eq!(next, Some("v20.0.0".to_string())); // 20 > 19
    }

    #[test]
    fn test_next_available_version_none_when_no_others() {
        let guard = setup_temp_nvm_dir();
        create_fake_version(guard.path(), "v20.0.0");

        let next = next_available_version("v20.0.0");
        assert_eq!(next, None);
    }

    #[test]
    fn test_next_available_version_none_when_empty() {
        let _guard = setup_temp_nvm_dir();
        let next = next_available_version("v20.0.0");
        assert_eq!(next, None);
    }

    #[test]
    fn test_shims_exist_false_before_creation() {
        let _guard = setup_temp_nvm_dir();
        assert!(!shims_exist());
    }

    #[test]
    fn test_create_shims_creates_all_commands() {
        let _guard = setup_temp_nvm_dir();
        create_shims().expect("create shims");

        let shims_dir = get_nvm_dir().join("shims");
        assert!(shims_dir.exists());

        if cfg!(windows) {
            for cmd in SHIM_COMMANDS {
                assert!(
                    shims_dir.join(format!("{}.cmd", cmd)).exists(),
                    "shim {}.cmd should exist",
                    cmd
                );
            }
        } else {
            for cmd in SHIM_COMMANDS {
                let shim = shims_dir.join(cmd);
                assert!(shim.exists(), "shim {} should exist", cmd);
                // Check it's executable on Unix
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let perms = fs::metadata(&shim).unwrap().permissions().mode();
                    assert!(perms & 0o111 != 0, "shim {} should be executable", cmd);
                }
            }
        }
    }

    #[test]
    fn test_shims_exist_true_after_creation() {
        let _guard = setup_temp_nvm_dir();
        create_shims().expect("create shims");
        assert!(shims_exist());
    }

    #[test]
    fn test_shim_commands_includes_corepack_tools() {
        // Regression: pnpm/pnpx/yarn/yarnpkg were missing from SHIM_COMMANDS,
        // causing "command not found" even after `nvm corepack enable`.
        for cmd in &["pnpm", "pnpx", "yarn", "yarnpkg"] {
            assert!(
                SHIM_COMMANDS.contains(cmd),
                "SHIM_COMMANDS must include '{}'",
                cmd
            );
        }
    }

    #[test]
    fn test_unix_shim_script_checks_for_none_marker() {
        let script = unix_shim_script();
        assert!(
            script.contains("none"),
            "Unix shim script must check for 'none' marker"
        );
        assert!(
            script.contains("deactivated"),
            "Unix shim script must print deactivation message"
        );
    }

    #[test]
    fn test_windows_shim_script_checks_for_none_marker() {
        let script = windows_shim_script();
        assert!(
            script.contains("none"),
            "Windows shim script must check for 'none' marker"
        );
        assert!(
            script.contains("deactivated"),
            "Windows shim script must print deactivation message"
        );
    }

    #[test]
    fn test_unix_shim_script_rejects_path_traversal() {
        // P1-12: the Unix shim must reject ".." and "/" in CURRENT
        // to prevent path traversal via a poisoned current file.
        let script = unix_shim_script();
        assert!(
            script.contains("*..*"),
            "Unix shim must reject '..' in CURRENT"
        );
        assert!(
            script.contains("corrupted current file"),
            "Unix shim must print a clear error when CURRENT is invalid"
        );
    }

    #[test]
    fn test_windows_shim_script_uses_quoted_set() {
        // P1-13: the Windows shim must use `set "BIN=..."` (quoted)
        // to prevent cmd.exe metacharacter injection via %CURRENT%.
        let script = windows_shim_script();
        assert!(
            script.contains("set \"BIN="),
            "Windows shim must use quoted set syntax to prevent batch injection"
        );
        assert!(
            !script.contains("set BIN=%NVM_DIR%"),
            "Windows shim must NOT use unquoted set BIN="
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_active_symlink_create_and_update() {
        let _guard = setup_temp_nvm_dir();
        let nvm_dir = crate::system::get_nvm_dir();
        create_fake_version(&nvm_dir, "v20.0.0");

        // Create
        create_active_symlink(&nvm_dir, "v20.0.0").expect("create symlink");
        assert!(active_exists(&nvm_dir), "active should exist after create");

        // Verify it points to v20.0.0
        let target = fs::read_link(nvm_dir.join("active")).expect("read link");
        assert!(
            target.ends_with("v20.0.0"),
            "should point to v20.0.0, got {:?}",
            target
        );

        // Update to different version
        create_fake_version(&nvm_dir, "v22.0.0");
        update_active_symlink(&nvm_dir, "v22.0.0").expect("update symlink");
        let target = fs::read_link(nvm_dir.join("active")).expect("read link after update");
        assert!(
            target.ends_with("v22.0.0"),
            "should point to v22.0.0, got {:?}",
            target
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_active_symlink_remove() {
        let _guard = setup_temp_nvm_dir();
        let nvm_dir = crate::system::get_nvm_dir();
        create_fake_version(&nvm_dir, "v20.0.0");
        create_active_symlink(&nvm_dir, "v20.0.0").expect("create");
        assert!(active_exists(&nvm_dir));

        remove_active_symlink(&nvm_dir).expect("remove");
        assert!(
            !active_exists(&nvm_dir),
            "active should not exist after remove"
        );
    }

    #[test]
    fn test_migrate_to_full_shim_no_current() {
        let _guard = setup_temp_nvm_dir();
        let nvm_dir = crate::system::get_nvm_dir();
        // No current file → should return Ok(false)
        let migrated = migrate_to_full_shim(&nvm_dir).expect("migrate");
        assert!(!migrated, "should not migrate when no current version");
        assert!(!active_exists(&nvm_dir), "active should not exist");
    }

    #[test]
    fn test_migrate_to_full_shim_current_none() {
        let _guard = setup_temp_nvm_dir();
        let nvm_dir = crate::system::get_nvm_dir();
        crate::utils::atomic_write(&nvm_dir.join("current"), "none").unwrap();
        let migrated = migrate_to_full_shim(&nvm_dir).expect("migrate");
        assert!(!migrated, "should not migrate when current is 'none'");
    }

    #[cfg(unix)]
    #[test]
    fn test_migrate_to_full_shim_first_migration() {
        let _guard = setup_temp_nvm_dir();
        let nvm_dir = crate::system::get_nvm_dir();
        create_fake_version(&nvm_dir, "v20.0.0");
        crate::utils::atomic_write(&nvm_dir.join("current"), "v20.0.0").unwrap();

        let migrated = migrate_to_full_shim(&nvm_dir).expect("migrate");
        assert!(migrated, "should return true on first migration");
        assert!(
            active_exists(&nvm_dir),
            "active should exist after migration"
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_migrate_to_full_shim_already_migrated() {
        let _guard = setup_temp_nvm_dir();
        let nvm_dir = crate::system::get_nvm_dir();
        create_fake_version(&nvm_dir, "v20.0.0");
        crate::utils::atomic_write(&nvm_dir.join("current"), "v20.0.0").unwrap();

        // First migration
        migrate_to_full_shim(&nvm_dir).expect("first migrate");

        // Second call should return false (already migrated)
        let migrated = migrate_to_full_shim(&nvm_dir).expect("second migrate");
        assert!(!migrated, "should return false on subsequent calls");
    }
}
