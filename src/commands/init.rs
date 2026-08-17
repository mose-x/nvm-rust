//! `nvm init` — one-time setup: shims, completions, shell config, active symlink.
//!
//! Run after install.sh or `nvm upgrade` to ensure everything is configured.
//! Idempotent — safe to run anytime.

use anyhow::{Context, Result};
use colored::Colorize;
use std::path::Path;

use crate::i18n::T;
use crate::system::get_nvm_dir;
use crate::utils::atomic_write;

/// Entry point: detect shell, create shims, generate completions, ensure shell config,
/// create active symlink if current version exists.
pub fn init(shell: Option<&str>) -> Result<()> {
    let shell_type = shell.map(|s| s.to_string()).unwrap_or_else(detect_shell);

    let nvm_dir = get_nvm_dir();

    // 1. Create shims
    crate::shim::create_shims()?;
    println!("  {} {}", "✓".green().bold(), T("init_shims_created"));

    // 2. Generate completions for detected shell
    crate::completions::generate_completions(Some(&shell_type))?;
    println!(
        "  {} {}",
        "✓".green().bold(),
        T("init_completions_generated")
    );

    // 3. Ensure shell config has nvm lines (with active/bin format)
    ensure_shell_config(&nvm_dir)?;

    // 4. Create active symlink if current version exists
    match crate::shim::migrate_to_full_shim(&nvm_dir) {
        Ok(true) => println!("  {} {}", "✓".green().bold(), T("shim_migrated")),
        Ok(false) => {}
        Err(e) => eprintln!("  {} active symlink: {}", "⚠".yellow().bold(), e),
    }

    println!();
    println!("  {}", T("init_restart_hint").dimmed());
    match shell_type.as_str() {
        "zsh" => println!("  exec zsh"),
        "bash" => println!("  exec bash"),
        "fish" => println!("  exec fish"),
        _ => println!("  Restart your terminal"),
    }
    Ok(())
}

/// Detect the current shell from $SHELL or platform.
fn detect_shell() -> String {
    if cfg!(windows) {
        return "powershell".to_string();
    }
    match std::env::var("SHELL") {
        Ok(s) => {
            if s.ends_with("zsh") {
                "zsh"
            } else if s.ends_with("bash") {
                "bash"
            } else if s.ends_with("fish") {
                "fish"
            } else {
                "bash"
            }
        }
        .to_string(),
        Err(_) => "bash".to_string(),
    }
}

/// Ensure the shell rc file has nvm source lines with active/bin PATH format.
/// If rc already has nvm lines, skip. If not, add them.
fn ensure_shell_config(nvm_dir: &Path) -> Result<()> {
    let shell_config = match crate::config::detect_shell_config() {
        Some(p) => p,
        None => {
            println!("  {} {}", "ℹ".cyan().bold(), T("init_no_rc"));
            return Ok(());
        }
    };

    let config_path = Path::new(&shell_config);
    let nvm_dir_str = nvm_dir.display().to_string();
    let content = match std::fs::read_to_string(config_path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e).context(T("shell_config_read_failed")),
    };

    // Check if rc already has nvm lines
    let has_nvm = content.contains("nvm.rust") || content.contains("nvm.sh");
    if has_nvm {
        // Already configured — ensure it's the new active/bin format
        if !content.contains("active") {
            crate::config::migrate_rc_to_shim_mode()?;
            println!("  {} {}", "✓".green().bold(), T("init_shell_migrated"));
        } else {
            println!("  {} {}", "✓".green().bold(), T("init_shell_configured"));
        }
        return Ok(());
    }

    // Add nvm lines with active/bin format
    crate::utils::backup_file(config_path).context(T("shell_config_backup_failed"))?;
    let shims = nvm_dir.join("shims").display().to_string();
    let active_bin = nvm_dir.join("active").join("bin").display().to_string();
    let nvm_bin = nvm_dir.join("bin").display().to_string();
    let nvm_export = format!(r#"export NVM_HOME="{}""#, nvm_dir_str);
    let path_export = format!(
        r#"export PATH="{}:{}:{}:$PATH""#,
        shims, active_bin, nvm_bin
    );
    let source_line = format!(r#"source "{}/bin/nvm.sh""#, nvm_dir_str);

    let new_line = format!(
        "\n# NVM Rust\n{}\n{}\n{}\n",
        nvm_export, path_export, source_line
    );
    let new_content = format!("{}{}", content, new_line);
    atomic_write(config_path, &new_content)?;
    println!(
        "  {} {} ({})",
        "✓".green().bold(),
        T("init_shell_configured"),
        shell_config
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_shell_returns_known_type() {
        let shell = detect_shell();
        assert!(
            shell == "bash" || shell == "zsh" || shell == "fish" || shell == "powershell",
            "detect_shell returned unknown: {shell}"
        );
    }
}
