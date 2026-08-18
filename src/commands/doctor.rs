//! `nvm doctor` — diagnostics + optional auto-fix.
//!
//! Checks: binary, shims, current, active symlink, shell config, completions,
//! corepack, pnpm source, PATH conflicts, network (optional).

use anyhow::Result;
use colored::Colorize;
use std::path::Path;

use crate::config::load_config;
use crate::i18n::{format_t, T};
use crate::shim::SHIM_COMMANDS;
use crate::system::{exe_path, get_nvm_dir, version_bin_dir};

/// Entry point: run all checks, print results, optionally fix.
pub fn doctor(fix: bool, network: bool) -> Result<()> {
    let nvm_dir = get_nvm_dir();

    println!("{}", "nvm doctor".cyan().bold());
    println!();

    check_binary();
    check_shims(&nvm_dir, fix);
    check_current(&nvm_dir, fix);
    check_shim_mode(&nvm_dir);
    check_shell_config(&nvm_dir);
    check_completions(&nvm_dir, fix);
    check_corepack(&nvm_dir, fix);
    check_pnpm_source(&nvm_dir);
    check_path_conflicts(&nvm_dir);

    if network {
        check_network();
    }

    if fix {
        println!();
        println!("  {} {}", "ℹ".cyan().bold(), T("doctor_fix_summary"));
    }

    Ok(())
}

fn check_binary() {
    let exe = std::env::current_exe().ok();
    let ver = env!("CARGO_PKG_VERSION");
    match exe {
        Some(path) => println!(
            "  {} {}",
            "✓".green().bold(),
            format_t(
                "doctor_binary_ok",
                &[ver.to_string(), path.display().to_string()]
            )
        ),
        None => println!("  {} {}", "✗".red().bold(), T("doctor_binary_fail")),
    }
}

fn check_shims(nvm_dir: &Path, fix: bool) {
    let shims_dir = nvm_dir.join("shims");
    let missing: Vec<&str> = SHIM_COMMANDS
        .iter()
        .filter(|cmd| {
            let shim = if cfg!(windows) {
                shims_dir.join(format!("{}.cmd", cmd))
            } else {
                shims_dir.join(cmd)
            };
            !shim.exists()
        })
        .copied()
        .collect();

    if missing.is_empty() {
        println!(
            "  {} {}",
            "✓".green().bold(),
            format_t(
                "doctor_shims_ok",
                &[
                    SHIM_COMMANDS.len().to_string(),
                    SHIM_COMMANDS.len().to_string()
                ]
            )
        );
    } else if fix {
        match crate::shim::create_shims() {
            Ok(()) => println!(
                "  {} {}",
                "⚠".yellow().bold(),
                format_t("doctor_shims_fixed", &[missing.len().to_string()])
            ),
            Err(e) => println!(
                "  {} {}",
                "✗".red().bold(),
                format_t(
                    "doctor_shims_fix_failed",
                    &[missing.len().to_string(), e.to_string()]
                )
            ),
        }
    } else {
        println!(
            "  {} {}",
            "✗".red().bold(),
            format_t("doctor_shims_missing", &[missing.len().to_string()])
        );
    }
}

fn check_current(nvm_dir: &Path, fix: bool) {
    let current_file = nvm_dir.join("current");
    match std::fs::read_to_string(&current_file) {
        Ok(content) => {
            let v = content.trim();
            if v.is_empty() || v == "none" {
                println!(
                    "  {} {}",
                    "ℹ".cyan().bold(),
                    T("doctor_current_deactivated")
                );
            } else if nvm_dir.join(v).is_dir() {
                println!(
                    "  {} {}",
                    "✓".green().bold(),
                    format_t("doctor_current_ok", &[v.to_string()])
                );
            } else if fix {
                if let Some(latest) = crate::shim::next_available_version("") {
                    match crate::utils::atomic_write(&current_file, &latest) {
                        Ok(()) => println!(
                            "  {} {}",
                            "⚠".yellow().bold(),
                            format_t("doctor_current_fixed", &[v.to_string(), latest])
                        ),
                        Err(e) => println!(
                            "  {} {}",
                            "✗".red().bold(),
                            format_t("doctor_current_fix_failed", &[e.to_string()])
                        ),
                    }
                } else {
                    println!("  {} {}", "✗".red().bold(), T("doctor_current_no_versions"));
                }
            } else {
                println!(
                    "  {} {}",
                    "✗".red().bold(),
                    format_t("doctor_current_not_installed", &[v.to_string()])
                );
            }
        }
        Err(_) if !current_file.exists() => {
            if fix {
                if let Some(latest) = crate::shim::next_available_version("") {
                    match crate::utils::atomic_write(&current_file, &latest) {
                        Ok(()) => println!(
                            "  {} {}",
                            "⚠".yellow().bold(),
                            format_t("doctor_current_set", &[latest])
                        ),
                        Err(e) => println!(
                            "  {} {}",
                            "✗".red().bold(),
                            format_t("doctor_current_fix_failed", &[e.to_string()])
                        ),
                    }
                } else {
                    println!("  {} {}", "ℹ".cyan().bold(), T("doctor_current_not_set"));
                }
            } else {
                println!(
                    "  {} {}",
                    "ℹ".cyan().bold(),
                    T("doctor_current_not_set_hint")
                );
            }
        }
        Err(_) => println!("  {} {}", "✗".red().bold(), T("doctor_current_unreadable")),
    }
}

fn check_shim_mode(nvm_dir: &Path) {
    if crate::shim::active_exists(nvm_dir) {
        println!("  {} {}", "✓".green().bold(), T("doctor_shim_mode_active"));
    } else {
        println!("  {} {}", "ℹ".cyan().bold(), T("doctor_shim_mode_legacy"));
    }
}

fn check_shell_config(_nvm_dir: &Path) {
    // Read the rc file directly — check common paths including fish and PowerShell.
    let home = crate::system::get_home_dir();
    let candidates = [
        format!("{}/.zshrc", home),
        format!("{}/.bashrc", home),
        format!("{}/.bash_profile", home),
        format!("{}/.profile", home),
        format!("{}/.config/fish/config.fish", home),
        format!(
            "{}/Documents/PowerShell/Microsoft.PowerShell_profile.ps1",
            home
        ),
        format!(
            "{}/Documents/WindowsPowerShell/Microsoft.PowerShell_profile.ps1",
            home
        ),
    ];

    for rc_path in &candidates {
        let path = Path::new(rc_path);
        if !path.exists() {
            continue;
        }
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let has_nvm = content.contains("nvm.rust") || content.contains("nvm.sh");
        // Check for "active" in a PATH context — Unix uses active/bin,
        // PowerShell uses active; (semicolon separator) or active" (end of string).
        let has_active = content.contains("active/bin")
            || content.contains("active\\bin")
            || content.contains("active;")
            || content.contains("active\"");
        if has_nvm && has_active {
            println!(
                "  {} {}",
                "✓".green().bold(),
                format_t("doctor_shell_config_ok", &[rc_path.to_string()])
            );
            return;
        } else if has_nvm {
            println!(
                "  {} {}",
                "⚠".yellow().bold(),
                format_t("doctor_shell_config_legacy", &[rc_path.to_string()])
            );
            return;
        }
    }
    println!(
        "  {} {}",
        "⚠".yellow().bold(),
        T("doctor_shell_config_not_configured")
    );
}

fn check_completions(nvm_dir: &Path, fix: bool) {
    let comp_dir = nvm_dir.join("completions");
    if !comp_dir.exists() {
        println!(
            "  {} {}",
            "ℹ".cyan().bold(),
            T("doctor_completions_not_installed")
        );
        return;
    }
    let has_any = comp_dir
        .read_dir()
        .map(|mut d| d.next().is_some())
        .unwrap_or(false);
    if has_any {
        println!("  {} {}", "✓".green().bold(), T("doctor_completions_ok"));
    } else if fix {
        match crate::completions::regenerate_completions_if_installed() {
            Ok(_) => println!(
                "  {} {}",
                "⚠".yellow().bold(),
                T("doctor_completions_regenerated")
            ),
            Err(e) => println!(
                "  {} {}",
                "✗".red().bold(),
                format_t("doctor_completions_fix_failed", &[e.to_string()])
            ),
        }
    } else {
        println!(
            "  {} {}",
            "⚠".yellow().bold(),
            T("doctor_completions_empty")
        );
    }
}

fn check_corepack(nvm_dir: &Path, fix: bool) {
    // Read current version
    let current_file = nvm_dir.join("current");
    let current = std::fs::read_to_string(&current_file)
        .ok()
        .map(|c| c.trim().to_string())
        .filter(|v| !v.is_empty() && v != "none");

    match current {
        Some(v) if nvm_dir.join(&v).is_dir() => {
            let bin = version_bin_dir(&nvm_dir.join(&v));
            let has_pnpm = exe_path(&bin, "pnpm").exists();
            let has_yarn = exe_path(&bin, "yarn").exists();
            if has_pnpm || has_yarn {
                println!(
                    "  {} {}",
                    "✓".green().bold(),
                    format_t("doctor_corepack_ok", &[v])
                );
            } else if fix {
                match crate::corepack::corepack_enable(Some(&v)) {
                    Ok(()) => println!(
                        "  {} {}",
                        "⚠".yellow().bold(),
                        format_t("doctor_corepack_fixed", &[v])
                    ),
                    Err(e) => println!(
                        "  {} {}",
                        "✗".red().bold(),
                        format_t("doctor_corepack_fix_failed", &[e.to_string()])
                    ),
                }
            } else {
                println!(
                    "  {} {}",
                    "ℹ".cyan().bold(),
                    T("doctor_corepack_not_enabled")
                );
            }
        }
        _ => println!(
            "  {} {}",
            "ℹ".cyan().bold(),
            T("doctor_corepack_no_current")
        ),
    }
}

fn check_pnpm_source(nvm_dir: &Path) {
    let current_file = nvm_dir.join("current");
    let current = std::fs::read_to_string(&current_file)
        .ok()
        .map(|c| c.trim().to_string())
        .filter(|v| !v.is_empty() && v != "none");

    match current {
        Some(v) if nvm_dir.join(&v).is_dir() => {
            let pnpm_path = exe_path(&version_bin_dir(&nvm_dir.join(&v)), "pnpm");
            if !pnpm_path.exists() {
                println!("  {} {}", "ℹ".cyan().bold(), T("doctor_pnpm_not_installed"));
            } else {
                let content = std::fs::read_to_string(&pnpm_path).unwrap_or_default();
                if content.contains("corepack") {
                    println!("  {} {}", "✓".green().bold(), T("doctor_pnpm_corepack"));
                } else {
                    println!("  {} {}", "⚠".yellow().bold(), T("doctor_pnpm_via_npm"));
                }
            }
        }
        _ => println!("  {} {}", "ℹ".cyan().bold(), T("doctor_pnpm_no_current")),
    }
}

fn check_path_conflicts(nvm_dir: &Path) {
    let shims_dir = nvm_dir.join("shims");
    let path = std::env::var("PATH").unwrap_or_default();
    let sep = if cfg!(windows) { ";" } else { ":" };
    let node_name = if cfg!(windows) { "node.exe" } else { "node" };

    let first_node = path.split(sep).find_map(|dir| {
        let p = Path::new(dir).join(node_name);
        if p.exists() {
            Some(p)
        } else {
            None
        }
    });

    match first_node {
        Some(p) if p.starts_with(&shims_dir) || p.starts_with(nvm_dir) => {
            println!("  {} {}", "✓".green().bold(), T("doctor_path_ok"));
        }
        Some(p) => {
            println!(
                "  {} {}",
                "⚠".yellow().bold(),
                format_t("doctor_path_conflict", &[p.display().to_string()])
            );
        }
        None => println!("  {} {}", "ℹ".cyan().bold(), T("doctor_path_no_node")),
    }
}

fn check_network() {
    let config = load_config().unwrap_or_default();
    let base_url = super::get_base_url(&config);
    let client = crate::proxy::build_http_client();
    let start = std::time::Instant::now();
    match client
        .get(base_url)
        .timeout(std::time::Duration::from_secs(10))
        .send()
    {
        Ok(resp) if resp.status().is_success() => {
            let ms = start.elapsed().as_millis();
            println!(
                "  {} {}",
                "✓".green().bold(),
                format_t("doctor_network_ok", &[base_url.to_string(), ms.to_string()])
            );
        }
        Ok(resp) => println!(
            "  {} {}",
            "✗".red().bold(),
            format_t(
                "doctor_network_http_error",
                &[base_url.to_string(), resp.status().to_string()]
            )
        ),
        Err(e) => println!(
            "  {} {}",
            "✗".red().bold(),
            format_t(
                "doctor_network_unreachable",
                &[base_url.to_string(), e.to_string()]
            )
        ),
    }
}
