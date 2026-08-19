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

    // Remove nvm binary
    let bin_name = if cfg!(windows) { "nvm.exe" } else { "nvm" };
    let nvm_bin = nvm_dir.join("bin").join(bin_name);
    let _ = fs::remove_file(&nvm_bin);

    // Remove /usr/local/bin/nvm — REAL binary in EDR-safe layout.
    // May be root-owned if installed via sudo. Try regular rm, suggest
    // sudo if it still exists (zero auto-sudo).
    #[cfg(unix)]
    {
        let system_bin = std::path::Path::new("/usr/local/bin/nvm");
        if system_bin.exists() {
            if fs::remove_file(system_bin).is_err() {
                eprintln!(
                    "  {} /usr/local/bin/nvm may be root-owned. Remove: sudo rm -f /usr/local/bin/nvm",
                    "⚠".yellow().bold()
                );
            }
        }
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

    // Remove the entire ~/.nvm.rust/ directory
    // This removes: binary, nvm.sh, shims, all v* version dirs, config.json,
    // alias.json, cache/, completions/, current, .nvm.lock
    if nvm_dir.exists() {
        fs::remove_dir_all(&nvm_dir).context("failed to remove nvm directory")?;
    }

    // Remove /usr/local/bin/nvm — REAL binary, may be root-owned.
    #[cfg(unix)]
    {
        let system_bin = std::path::Path::new("/usr/local/bin/nvm");
        if system_bin.exists() {
            if fs::remove_file(system_bin).is_err() {
                eprintln!(
                    "  {} /usr/local/bin/nvm may be root-owned. Remove: sudo rm -f /usr/local/bin/nvm",
                    "⚠".yellow().bold()
                );
            }
        }
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
