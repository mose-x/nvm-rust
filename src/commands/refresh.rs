//! `nvm refresh` — refresh shims, completions, validate state, download nvm.sh.
//!
//! Called after `nvm upgrade` (via exec) or manually when things feel stale.
//! Idempotent: safe to run anytime.

use anyhow::Result;
use colored::Colorize;
use std::fs;
use std::path::Path;

use crate::i18n::{format_t, T};
use crate::shim::{self, SHIM_COMMANDS};
use crate::system::get_nvm_dir;
use crate::utils::atomic_write;

/// Entry point: refresh shims, completions, validate current, download nvm.sh.
pub fn refresh() -> Result<()> {
    let nvm_dir = get_nvm_dir();

    // Acquire lock to prevent races with concurrent nvm use/uninstall.
    // Re-entrant: if refresh is called from nvm upgrade (which already holds
    // no lock), this is the only lock. If called standalone, this serializes.
    let _nvm_lock = crate::utils::acquire_nvm_lock(&nvm_dir)?;

    // 1. Re-create shim scripts (overwrite with latest content)
    crate::shim::create_shims()?;
    println!(
        "  {} {} ({} commands)",
        "✓".green().bold(),
        T("refresh_shims"),
        SHIM_COMMANDS.len()
    );

    // 2. Regenerate completion scripts (only if already installed)
    crate::completions::regenerate_completions_if_installed()?;
    println!("  {} {}", "✓".green().bold(), T("refresh_completions"));

    // 3. Validate and fix `current` file
    let current_file = nvm_dir.join("current");
    match validate_current(&nvm_dir) {
        CurrentStatus::Valid(v) => {
            println!(
                "  {} {}",
                "✓".green().bold(),
                format_t("refresh_current_valid", std::slice::from_ref(&v))
            );
        }
        CurrentStatus::Invalid(old) => {
            if let Some(latest) = shim::next_available_version("") {
                atomic_write(&current_file, &latest)?;
                println!(
                    "  {} {}",
                    "⚠".yellow().bold(),
                    format_t("refresh_current_fixed", &[old, latest])
                );
            } else {
                println!("  {} {}", "✗".red().bold(), T("refresh_no_versions"));
            }
        }
        CurrentStatus::Missing => {
            if let Some(latest) = shim::next_available_version("") {
                atomic_write(&current_file, &latest)?;
                println!(
                    "  {} {}",
                    "⚠".yellow().bold(),
                    format_t("refresh_current_set", std::slice::from_ref(&latest))
                );
            } else {
                println!("  {} {}", "ℹ".cyan().bold(), T("refresh_current_missing"));
            }
        }
        CurrentStatus::None => {
            println!("  {} {}", "ℹ".cyan().bold(), T("refresh_current_none"));
        }
    }

    // 4. Migrate to Full Shim mode (create active symlink + migrate rc)
    match crate::shim::migrate_to_full_shim(&nvm_dir) {
        Ok(true) => println!("  {} {}", "✓".green().bold(), T("shim_migrated")),
        Ok(false) => {} // already migrated or no active version
        Err(e) => eprintln!("  {} shim migration failed: {}", "⚠".yellow().bold(), e),
    }

    // 5. Download latest nvm.sh (release archive doesn't include it)
    refresh_nvm_sh(&nvm_dir)?;

    // 6. Fix unguarded `source` lines in shell config (auto-repair for
    // users who upgraded — pre-fix versions wrote `source "..."` without
    // `[ -f ]` guard, causing .zshrc errors when nvm.sh is missing).
    fix_rc_source_guard(&nvm_dir);

    // 7. zsh cache tip
    if std::env::var("SHELL")
        .map(|s| s.ends_with("zsh"))
        .unwrap_or(false)
    {
        println!();
        println!(
            "  {} {}",
            T("tip_label").dimmed(),
            T("completion_cache_tip").dimmed()
        );
    }

    println!();
    println!("  {}", T("refresh_complete").green().bold());
    Ok(())
}

/// Fix unguarded `source` lines in the shell config file.
///
/// Pre-fix versions wrote `source "/path/nvm.sh"` without a `[ -f ]` guard,
/// causing `.zshrc` to error on every shell startup when nvm.sh is missing.
/// This function detects and replaces those lines with the guarded version.
/// Safe to call on PowerShell profiles (they use `Import-Module`, not `source`).
fn fix_rc_source_guard(nvm_dir: &Path) {
    let shell_config = match crate::config::detect_shell_config() {
        Some(p) => p,
        None => return,
    };
    let config_path = Path::new(&shell_config);
    if config_path.extension().is_some_and(|ext| ext == "ps1") {
        return;
    }
    let content = match fs::read_to_string(config_path) {
        Ok(c) => c,
        Err(_) => return,
    };
    // Only fix if there's a `source` line with nvm.sh but no `[ -f ]` guard
    if !content.contains("source \"") || !content.contains("nvm.sh") || content.contains("[ -f ") {
        return;
    }
    let nvm_dir_str = nvm_dir.display().to_string();
    let old_source = format!(r#"source "{}/bin/nvm.sh""#, nvm_dir_str);
    let new_source = format!(
        r#"[ -f "{}/bin/nvm.sh" ] && source "{}/bin/nvm.sh""#,
        nvm_dir_str, nvm_dir_str
    );
    let fixed = content.replace(&old_source, &new_source);
    if fixed != content {
        if let Err(e) = atomic_write(config_path, &fixed) {
            eprintln!(
                "  {} Failed to fix shell config: {}",
                "⚠".yellow().bold(),
                e
            );
            return;
        }
        println!(
            "  {} Fixed: nvm.sh source line now has [ -f ] guard",
            "✓".green().bold()
        );
    }
}

/// Status of the `current` file.
enum CurrentStatus {
    Valid(String),
    Invalid(String),
    Missing,
    None,
}

fn validate_current(nvm_dir: &Path) -> CurrentStatus {
    let current_file = nvm_dir.join("current");
    match fs::read_to_string(&current_file) {
        Ok(content) => {
            let v = content.trim();
            if v.is_empty() || v == "none" {
                return CurrentStatus::None;
            }
            if nvm_dir.join(v).is_dir() {
                CurrentStatus::Valid(v.to_string())
            } else {
                CurrentStatus::Invalid(v.to_string())
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => CurrentStatus::Missing,
        Err(_) => CurrentStatus::Missing,
    }
}

/// Download the latest nvm.sh from GitHub raw, overwriting the local copy.
/// Creates the file if it doesn't exist (e.g. user installed via cargo
/// install, not install.sh — nvm.sh was never copied to bin/).
fn refresh_nvm_sh(nvm_dir: &Path) -> Result<()> {
    let nvm_sh_path = nvm_dir.join("bin").join("nvm.sh");

    let url = "https://raw.githubusercontent.com/mose-x/nvm-rust/main/shell/nvm.sh";
    let client = crate::proxy::build_http_client();
    let resp = match client.get(url).send() {
        Ok(r) => r,
        Err(e) => {
            let msg = e.to_string();
            eprintln!(
                "  {} {}",
                "⚠".yellow().bold(),
                format_t("refresh_nvm_sh_failed", std::slice::from_ref(&msg))
            );
            return Ok(());
        }
    };

    if !resp.status().is_success() {
        let msg = format!("HTTP {}", resp.status());
        eprintln!(
            "  {} {}",
            "⚠".yellow().bold(),
            format_t("refresh_nvm_sh_failed", std::slice::from_ref(&msg))
        );
        return Ok(());
    }

    let content = resp.text()?;
    atomic_write(&nvm_sh_path, &content)?;
    println!("  {} {}", "✓".green().bold(), T("refresh_nvm_sh"));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::ENV_TESTS_MUTEX;
    use std::env;
    use std::fs;
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

    fn setup_temp_nvm_dir() -> NvmDirGuard {
        let mutex = ENV_TESTS_MUTEX.lock().expect("mutex");
        let old_value = env::var("NVM_DIR").ok();
        let dir = tempfile::tempdir().expect("tempdir");
        env::set_var("NVM_DIR", dir.path());
        NvmDirGuard {
            old_value,
            _dir: dir,
            _mutex: mutex,
        }
    }

    fn create_fake_version(nvm_dir: &Path, version: &str) {
        let version_dir = nvm_dir.join(version);
        fs::create_dir_all(&version_dir).expect("create version dir");
        let bin_dir = if cfg!(windows) {
            version_dir.clone()
        } else {
            version_dir.join("bin")
        };
        fs::create_dir_all(&bin_dir).expect("create bin dir");
        if cfg!(windows) {
            fs::write(bin_dir.join("node.exe"), b"fake").expect("write node");
        } else {
            fs::write(bin_dir.join("node"), b"#!/bin/sh\nexit 0\n").expect("write node");
        }
    }

    #[test]
    fn test_validate_current_valid() {
        let _guard = setup_temp_nvm_dir();
        let nvm_dir = crate::system::get_nvm_dir();
        create_fake_version(&nvm_dir, "v20.0.0");
        crate::utils::atomic_write(&nvm_dir.join("current"), "v20.0.0").unwrap();
        match validate_current(&nvm_dir) {
            CurrentStatus::Valid(v) => assert_eq!(v, "v20.0.0"),
            _ => panic!("expected Valid"),
        }
    }

    #[test]
    fn test_validate_current_invalid() {
        let _guard = setup_temp_nvm_dir();
        let nvm_dir = crate::system::get_nvm_dir();
        crate::utils::atomic_write(&nvm_dir.join("current"), "v99.99.99").unwrap();
        match validate_current(&nvm_dir) {
            CurrentStatus::Invalid(v) => assert_eq!(v, "v99.99.99"),
            _ => panic!("expected Invalid"),
        }
    }

    #[test]
    fn test_validate_current_missing() {
        let _guard = setup_temp_nvm_dir();
        let nvm_dir = crate::system::get_nvm_dir();
        match validate_current(&nvm_dir) {
            CurrentStatus::Missing => {}
            _ => panic!("expected Missing"),
        }
    }

    #[test]
    fn test_validate_current_none() {
        let _guard = setup_temp_nvm_dir();
        let nvm_dir = crate::system::get_nvm_dir();
        crate::utils::atomic_write(&nvm_dir.join("current"), "none").unwrap();
        match validate_current(&nvm_dir) {
            CurrentStatus::None => {}
            _ => panic!("expected None"),
        }
    }
}
