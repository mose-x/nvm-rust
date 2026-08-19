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
            crate::config::migrate_rc_to_shim_mode_with_dir(nvm_dir)?;
            println!("  {} {}", "✓".green().bold(), T("init_shell_migrated"));
        } else {
            // Already has active format, but check for old-style source line
            // (pre-fix versions wrote `source "..."` without `[ -f ]` guard,
            // or used literal paths instead of $NVM_HOME). Replace with the
            // guarded $NVM_HOME version so .zshrc doesn't error when nvm.sh
            // is missing and works regardless of install path.
            if !shell_config.ends_with(".ps1")
                && content.contains("source \"")
                && content.contains("nvm.sh")
            {
                let old_unguarded = format!(r#"source "{}/bin/nvm.sh""#, nvm_dir_str);
                let old_guarded = format!(
                    r#"[ -f "{}/bin/nvm.sh" ] && source "{}/bin/nvm.sh""#,
                    nvm_dir_str, nvm_dir_str
                );
                let new_source =
                    r#"[ -f "$NVM_HOME/bin/nvm.sh" ] && source "$NVM_HOME/bin/nvm.sh""#;
                let fixed = content
                    .replace(&old_unguarded, new_source)
                    .replace(&old_guarded, new_source);
                if fixed != content {
                    crate::utils::atomic_write(config_path, &fixed)
                        .context(T("shell_config_write_failed"))?;
                    println!(
                        "  {} Fixed: nvm.sh source line now uses $NVM_HOME with [ -f ] guard",
                        "✓".green().bold()
                    );
                }
            }
            println!("  {} {}", "✓".green().bold(), T("init_shell_configured"));
        }
        return Ok(());
    }

    // Add nvm lines with active/bin format
    crate::utils::backup_file(config_path).context(T("shell_config_backup_failed"))?;

    let is_powershell = shell_config.ends_with(".ps1");
    let (nvm_export, path_export, source_line) = if is_powershell {
        (
            format!(r#"$env:NVM_HOME = "{}""#, nvm_dir_str),
            r#"$env:PATH = "$env:NVM_HOME\shims;$env:NVM_HOME\active;$env:NVM_HOME\bin;" + $env:PATH"#
                .to_string(),
            r#"Import-Module "$env:NVM_HOME\shell\nvm.psm1""#.to_string(),
        )
    } else {
        (
            format!(r#"export NVM_HOME="{}""#, nvm_dir_str),
            r#"export PATH="$NVM_HOME/shims:$NVM_HOME/active/bin:$NVM_HOME/bin:$PATH""#.to_string(),
            r#"[ -f "$NVM_HOME/bin/nvm.sh" ] && source "$NVM_HOME/bin/nvm.sh""#.to_string(),
        )
    };

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
