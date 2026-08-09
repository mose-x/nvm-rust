use anyhow::{Context, Result};
use std::fs;

use crate::system::get_nvm_dir;

/// Commands that get a shim. These are the binaries that ship with every
/// Node.js distribution (bin/ directory). yarn/pnpm are handled by corepack
/// — they live inside the version's bin/ after `nvm corepack enable`, so the
/// same shim script covers them automatically (basename "$0" resolves to
/// "yarn" or "pnpm" at runtime).
pub const SHIM_COMMANDS: &[&str] = &["node", "npm", "npx", "corepack"];

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
    if exist "%NVM_DIR%\%CURRENT%\bin\%CMD%.exe" set BIN=%NVM_DIR%\%CURRENT%\bin\%CMD%.exe
    if not defined BIN if exist "%NVM_DIR%\%CURRENT%\bin\%CMD%.cmd" set BIN=%NVM_DIR%\%CURRENT%\bin\%CMD%.cmd
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
}
