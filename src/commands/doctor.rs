//! `nvm doctor` — diagnostics + optional auto-fix.
//!
//! Checks: binary, shims, current, active symlink, shell config, completions,
//! corepack, pnpm source, PATH conflicts, network (optional).

use anyhow::Result;
use colored::Colorize;
use std::path::Path;

use crate::config::load_config;
use crate::shim::SHIM_COMMANDS;
use crate::system::{exe_path, get_nvm_dir, version_bin_dir, URI};

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
        println!(
            "  {} Run 'nvm doctor' again to verify fixes, or restart your shell.",
            "ℹ".cyan().bold()
        );
    }

    Ok(())
}

fn check_binary() {
    let exe = std::env::current_exe().ok();
    let ver = env!("CARGO_PKG_VERSION");
    match exe {
        Some(path) => println!(
            "  {} binary       v{} at {}",
            "✓".green().bold(),
            ver,
            path.display()
        ),
        None => println!("  {} binary       cannot find executable", "✗".red().bold()),
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
            "  {} shims        {}/{} present",
            "✓".green().bold(),
            SHIM_COMMANDS.len(),
            SHIM_COMMANDS.len()
        );
    } else if fix {
        match crate::shim::create_shims() {
            Ok(()) => println!(
                "  {} shims        {} missing, fixed",
                "⚠".yellow().bold(),
                missing.len()
            ),
            Err(e) => println!(
                "  {} shims        {} missing, fix FAILED: {}",
                "✗".red().bold(),
                missing.len(),
                e
            ),
        }
    } else {
        println!(
            "  {} shims        {} missing — run 'nvm refresh'",
            "✗".red().bold(),
            missing.len()
        );
    }
}

fn check_current(nvm_dir: &Path, fix: bool) {
    let current_file = nvm_dir.join("current");
    match std::fs::read_to_string(&current_file) {
        Ok(content) => {
            let v = content.trim();
            if v.is_empty() || v == "none" {
                println!("  {} current      deactivated", "ℹ".cyan().bold());
            } else if nvm_dir.join(v).is_dir() {
                println!("  {} current      {}", "✓".green().bold(), v);
            } else if fix {
                if let Some(latest) = crate::shim::next_available_version("") {
                    match crate::utils::atomic_write(&current_file, &latest) {
                        Ok(()) => println!(
                            "  {} current      {} → {} (fixed)",
                            "⚠".yellow().bold(),
                            v,
                            latest
                        ),
                        Err(e) => println!("  {} current      fix FAILED: {}", "✗".red().bold(), e),
                    }
                } else {
                    println!("  {} current      no installed versions", "✗".red().bold());
                }
            } else {
                println!("  {} current      {} not installed", "✗".red().bold(), v);
            }
        }
        Err(_) if !current_file.exists() => {
            if fix {
                if let Some(latest) = crate::shim::next_available_version("") {
                    match crate::utils::atomic_write(&current_file, &latest) {
                        Ok(()) => {
                            println!("  {} current      set to {}", "⚠".yellow().bold(), latest)
                        }
                        Err(e) => println!("  {} current      fix FAILED: {}", "✗".red().bold(), e),
                    }
                } else {
                    println!("  {} current      not set", "ℹ".cyan().bold());
                }
            } else {
                println!(
                    "  {} current      not set — run 'nvm use <version>'",
                    "ℹ".cyan().bold()
                );
            }
        }
        Err(_) => println!("  {} current      unreadable", "✗".red().bold()),
    }
}

fn check_shim_mode(nvm_dir: &Path) {
    if crate::shim::active_exists(nvm_dir) {
        println!("  {} shim mode    active (full shim)", "✓".green().bold());
    } else {
        println!(
            "  {} shim mode    legacy — run 'nvm refresh' to migrate",
            "ℹ".cyan().bold()
        );
    }
}

fn check_shell_config(_nvm_dir: &Path) {
    // Read the rc file directly — check common paths including fish.
    let home = crate::system::get_home_dir();
    let candidates = [
        format!("{}/.zshrc", home),
        format!("{}/.bashrc", home),
        format!("{}/.bash_profile", home),
        format!("{}/.profile", home),
        format!("{}/.config/fish/config.fish", home),
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
        // Check for "active/bin" specifically, not just "active" which could
        // appear in unrelated comments or variable names.
        let has_active_bin = content.contains("active/bin") || content.contains("active\\bin");
        if has_nvm && has_active_bin {
            println!(
                "  {} shell config {} (full shim format)",
                "✓".green().bold(),
                rc_path
            );
            return;
        } else if has_nvm {
            println!(
                "  {} shell config {} (legacy format — run 'nvm refresh')",
                "⚠".yellow().bold(),
                rc_path
            );
            return;
        }
    }
    println!(
        "  {} shell config not configured — run 'nvm init'",
        "⚠".yellow().bold()
    );
}

fn check_completions(nvm_dir: &Path, fix: bool) {
    let comp_dir = nvm_dir.join("completions");
    if !comp_dir.exists() {
        println!(
            "  {} completions  not installed — run 'nvm completion <shell>'",
            "ℹ".cyan().bold()
        );
        return;
    }
    let has_any = comp_dir
        .read_dir()
        .map(|mut d| d.next().is_some())
        .unwrap_or(false);
    if has_any {
        println!("  {} completions  installed", "✓".green().bold());
    } else if fix {
        match crate::completions::regenerate_completions_if_installed() {
            Ok(_) => println!("  {} completions  regenerated", "⚠".yellow().bold()),
            Err(e) => println!("  {} completions  fix FAILED: {}", "✗".red().bold(), e),
        }
    } else {
        println!(
            "  {} completions  empty — run 'nvm completion <shell>'",
            "⚠".yellow().bold()
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
                println!("  {} corepack     enabled for {}", "✓".green().bold(), v);
            } else if fix {
                match crate::corepack::corepack_enable(Some(&v)) {
                    Ok(()) => println!(
                        "  {} corepack     enabled for {} (fixed)",
                        "⚠".yellow().bold(),
                        v
                    ),
                    Err(e) => println!("  {} corepack     fix FAILED: {}", "✗".red().bold(), e),
                }
            } else {
                println!(
                    "  {} corepack     not enabled — run 'nvm corepack enable'",
                    "ℹ".cyan().bold()
                );
            }
        }
        _ => println!("  {} corepack     no current version", "ℹ".cyan().bold()),
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
                println!("  {} pnpm         not installed", "ℹ".cyan().bold());
            } else {
                let content = std::fs::read_to_string(&pnpm_path).unwrap_or_default();
                if content.contains("corepack") {
                    println!("  {} pnpm         managed by corepack", "✓".green().bold());
                } else {
                    println!(
                        "  {} pnpm         installed via npm — run 'nvm install-pnpm'",
                        "⚠".yellow().bold()
                    );
                }
            }
        }
        _ => println!("  {} pnpm         no current version", "ℹ".cyan().bold()),
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
            println!("  {} PATH         nvm's node is first", "✓".green().bold());
        }
        Some(p) => {
            println!(
                "  {} PATH         system node at {} shadows nvm",
                "⚠".yellow().bold(),
                p.display()
            );
        }
        None => println!("  {} PATH         no node found", "ℹ".cyan().bold()),
    }
}

fn check_network() {
    let config = load_config().unwrap_or_default();
    let base_url = config.mirror.as_deref().unwrap_or(URI);
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
                "  {} network      {} reachable ({}ms)",
                "✓".green().bold(),
                base_url,
                ms
            );
        }
        Ok(resp) => println!(
            "  {} network      {} HTTP {}",
            "✗".red().bold(),
            base_url,
            resp.status()
        ),
        Err(e) => println!(
            "  {} network      {} unreachable: {}",
            "✗".red().bold(),
            base_url,
            e
        ),
    }
}
