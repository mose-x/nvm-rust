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

    // 5. Migrate binary to system path (EDR-safe layout).
    // If the binary is a real file in the user dir (old layout), try to
    // move it to /usr/local/bin (Unix) and replace with a symlink. This
    // is a no-op if the binary is already in a system path or is already
    // a symlink.
    migrate_binary_to_system_path(&nvm_dir);

    // 6. Download latest nvm.sh (release archive doesn't include it)
    refresh_nvm_sh(&nvm_dir)?;

    // 7. Fix unguarded `source` lines in shell config (auto-repair for
    // users who upgraded — pre-fix versions wrote `source "..."` without
    // `[ -f ]` guard, causing .zshrc errors when nvm.sh is missing).
    fix_rc_source_guard(&nvm_dir);

    // 8. Windows: repair PowerShell integration. Pre-2.4.0 installers
    // injected `Import-Module nvm.psm1` into the profile; that module
    // shadowed nvm.exe and broke `nvm -v`/exit codes. `nvm upgrade` execs
    // this same `refresh` on the NEW binary, so updating self-heals.
    #[cfg(windows)]
    repair_powershell(&nvm_dir);

    // 9. zsh cache tip
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

/// Migrate the nvm binary from the old layout (real file in user dir) to
/// the new EDR-safe layout (real binary in system path, symlink in user dir).
///
/// Old layout (EDR-risky):
/// ```text
/// ~/.nvm.rust/bin/nvm          ← real binary (EDR kills)
/// /usr/local/bin/nvm           ← symlink → ~/.nvm.rust/bin/nvm
/// ```
///
/// New layout (EDR-safe):
/// ```text
/// /usr/local/bin/nvm           ← real binary (EDR trusts)
/// ~/.nvm.rust/bin/nvm          ← symlink → /usr/local/bin/nvm
/// ```
///
/// This function tries `/usr/local/bin` directly: if the directory
/// exists and is writable, it copies the real binary there and creates the
/// reverse symlink. No probe, no auto-sudo — if the directory is not
/// writable, it prints an explicit message telling the user how to do it
/// manually.
///
/// Safe to call on all platforms. On Windows, prints a warning suggesting
/// admin install. No-op if the binary is already a symlink or doesn't exist.
fn migrate_binary_to_system_path(nvm_dir: &Path) {
    let bin_name = if cfg!(windows) { "nvm.exe" } else { "nvm" };
    let user_bin = nvm_dir.join("bin").join(bin_name);

    // Nothing to migrate if the user-path binary doesn't exist.
    if !user_bin.exists() {
        return;
    }

    // Use symlink_metadata to check if it's a symlink WITHOUT following it.
    // If it's already a symlink, the migration is already done.
    match std::fs::symlink_metadata(&user_bin) {
        Ok(meta) => {
            if meta.file_type().is_symlink() {
                return; // already migrated
            }
        }
        Err(_) => return, // can't stat, bail
    }

    #[cfg(unix)]
    {
        eprintln!("  {} {}", "ℹ".cyan().bold(), T("refresh_binary_migrating"));

        // Simple writability check: try /usr/local/bin directly. If it exists
        // and is writable, copy the real binary there and create a reverse
        // symlink (user dir → system path). No probe, no auto-sudo.
        let system_dir = std::path::Path::new("/usr/local/bin");
        if system_dir.is_dir() && crate::commands::binary_swap::is_dir_writable(system_dir) {
            let system_bin = system_dir.join(bin_name);
            if std::fs::copy(&user_bin, &system_bin).is_ok() {
                use std::os::unix::fs::PermissionsExt;
                let _ =
                    std::fs::set_permissions(&system_bin, std::fs::Permissions::from_mode(0o755));
                // Remove the real file and create a symlink → system path.
                if std::fs::remove_file(&user_bin).is_ok() {
                    if std::os::unix::fs::symlink(&system_bin, &user_bin).is_ok() {
                        println!("  {} {}", "✓".green().bold(), T("refresh_binary_migrated"));
                        return;
                    } else {
                        // Restore the real binary so the user isn't left
                        // without a working nvm.
                        let _ = std::fs::copy(&system_bin, &user_bin);
                        eprintln!(
                            "  {} {}",
                            "⚠".yellow().bold(),
                            T("refresh_binary_symlink_failed")
                        );
                        return;
                    }
                } else {
                    eprintln!(
                        "  {} {}",
                        "⚠".yellow().bold(),
                        T("refresh_binary_remove_failed")
                    );
                    return;
                }
            }
        }

        // System path not writable or copy failed — print explicit manual
        // instructions (no auto-sudo).
        eprintln!(
            "  {} {}",
            "⚠".yellow().bold(),
            format_t(
                "refresh_binary_edr_risk_manual",
                &[
                    user_bin.display().to_string(),
                    "/usr/local/bin/nvm".to_string()
                ]
            )
        );
    }
    #[cfg(windows)]
    {
        // Windows has no /usr/local/bin equivalent that EDR trusts. The
        // closest is %ProgramFiles%\nvm-rust\. Suggest running
        // install.ps1 as admin for the system-path install.
        let system_dir = std::env::var("ProgramFiles")
            .map(|pf| format!("{}\\nvm-rust", pf))
            .unwrap_or_else(|_| "C:\\Program Files\\nvm-rust".to_string());
        eprintln!(
            "  {} {} (system path candidate: {})",
            "⚠".yellow().bold(),
            T("refresh_binary_edr_risk_windows"),
            system_dir
        );
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = nvm_dir;
    }
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
    // Fix any source line that uses literal paths instead of $NVM_HOME,
    // or is missing the [ -f ] guard. Replace with the $NVM_HOME guarded version.
    if !content.contains("source \"") || !content.contains("nvm.sh") {
        return;
    }
    // Already uses $NVM_HOME with guard — nothing to do
    if content.contains("$NVM_HOME/bin/nvm.sh") {
        return;
    }
    let nvm_dir_str = nvm_dir.display().to_string();
    let new_source = r#"[ -f "$NVM_HOME/bin/nvm.sh" ] && source "$NVM_HOME/bin/nvm.sh""#;
    // Replace both unguarded and guarded-but-literal versions
    let old_unguarded = format!(r#"source "{}/bin/nvm.sh""#, nvm_dir_str);
    let old_guarded = format!(
        r#"[ -f "{}/bin/nvm.sh" ] && source "{}/bin/nvm.sh""#,
        nvm_dir_str, nvm_dir_str
    );
    let fixed = content
        .replace(&old_unguarded, new_source)
        .replace(&old_guarded, new_source);
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

/// Windows-only: rewrite the fixed nvm.psm1 and strip the legacy
/// `Import-Module` profile injection left by pre-2.4.0 installers.
/// Never fails the refresh — a repair error only warns.
#[cfg(windows)]
fn repair_powershell(nvm_dir: &Path) {
    match crate::ps_repair::repair(nvm_dir) {
        Ok(report) => {
            if report.psm1_written || report.profiles_cleaned > 0 {
                println!("  {} {}", "✓".green().bold(), T("refresh_ps_repaired"));
            } else {
                println!("  {} {}", "✓".green().bold(), T("refresh_ps_clean"));
            }
        }
        Err(e) => eprintln!(
            "  {} {}",
            "⚠".yellow().bold(),
            format_t("refresh_ps_failed", std::slice::from_ref(&e.to_string()))
        ),
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
