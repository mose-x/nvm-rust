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
if [ -z "$CURRENT" ] || [ ! -x "$NVM_DIR/$CURRENT/bin/$CMD" ]; then
    "$NVM_DIR/bin/nvm" auto --silent 2>/dev/null
    CURRENT=$(read_current)
fi
if [ -z "$CURRENT" ] || [ ! -x "$NVM_DIR/$CURRENT/bin/$CMD" ]; then
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
