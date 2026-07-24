use std::fs;
use std::process::Command;

use colored::Colorize;

use crate::i18n::{format_t, T};
use crate::system::{exe_path, get_nvm_dir, prepend_to_path, version_bin_dir};

/// Tool shims that `corepack enable` writes into a version's `bin/` dir.
///
/// Single source for both the "is corepack enabled?" probe and the
/// "remove corepack shims" fallback — previously the array was duplicated
/// at the two call sites, which had to be kept in sync by hand.
const COREPACK_SHIMS: &[&str] = &["pnpm", "pnpx", "yarn", "yarnpkg"];

pub fn corepack_status(version: Option<&str>) -> anyhow::Result<()> {
    let nvm_dir = get_nvm_dir();

    match version {
        Some(ver) => {
            let resolved = crate::config::resolve_alias(ver)?;
            let version_bin = version_bin_dir(&nvm_dir.join(&resolved));
            let node_path = exe_path(&version_bin, "node");

            if !node_path.exists() {
                anyhow::bail!(
                    "{}",
                    format_t("not_installed", std::slice::from_ref(&resolved))
                );
            }

            let corepack_path = exe_path(&version_bin, "corepack");

            if !corepack_path.exists() {
                println!(
                    "{} {} {}",
                    "✗".red().bold(),
                    T("corepack_not_found_for").red(),
                    resolved.white().bold()
                );
                println!();
                println!(
                    "  {}: {}",
                    T("tip_label").dimmed(),
                    format_t("corepack_install_tip", std::slice::from_ref(&resolved))
                );
                return Ok(());
            }

            // Corepack "enabled" means the tool shims (pnpm/yarn/...) have been
            // written into the version's bin directory by `corepack enable`.
            // We must NOT probe by running `corepack <tool> --version`, because
            // corepack will happily download and run the tool on first call even
            // when it has not been enabled — that would falsely report "enabled".
            let activated: Vec<&str> = COREPACK_SHIMS
                .iter()
                .copied()
                .filter(|t| exe_path(&version_bin, t).exists())
                .collect();

            if activated.is_empty() {
                println!(
                    "{} {} {}",
                    "○".yellow().bold(),
                    T("corepack_disabled_for").yellow(),
                    resolved.white().bold()
                );
                println!();
                println!(
                    "  {} {}",
                    T("tip_label").dimmed(),
                    format_t("corepack_install_tip", std::slice::from_ref(&resolved)).dimmed()
                );
            } else {
                println!(
                    "{} {} {}",
                    "✓".green().bold(),
                    T("corepack_enabled_for").green(),
                    resolved.white().bold()
                );
                println!();
                for tool in activated {
                    // Probe the shim directly (not via `corepack <tool>`) so we
                    // only print a version when the shim is actually installed.
                    let ver = Command::new(exe_path(&version_bin, tool))
                        .arg("--version")
                        .output()
                        .ok()
                        .and_then(|o| {
                            if o.status.success() {
                                Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
                            } else {
                                None
                            }
                        })
                        .unwrap_or_default();
                    println!("  {} {}", tool.cyan(), ver.dimmed());
                }
            }
        }
        None => {
            // Show status for current version
            let current_file = nvm_dir.join("current");
            if current_file.exists() {
                let current = fs::read_to_string(&current_file)?.trim().to_string();
                if !current.starts_with("system:") {
                    return corepack_status(Some(&current));
                }
            }

            // Check if corepack is available system-wide
            if let Ok(output) = Command::new("corepack").arg("--version").output() {
                let version_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
                println!(
                    "{} {} {}",
                    "ℹ".cyan().bold(),
                    T("system_corepack").cyan(),
                    version_str.white().bold()
                );
            } else {
                println!("{} {}", "ℹ".cyan().bold(), T("corepack_no_version").cyan());
            }
        }
    }

    Ok(())
}

pub fn corepack_enable(version: Option<&str>) -> anyhow::Result<()> {
    let nvm_dir = get_nvm_dir();
    let resolved = match version {
        Some(ver) => crate::config::resolve_alias(ver)?,
        None => {
            let current_file = nvm_dir.join("current");
            if current_file.exists() {
                fs::read_to_string(&current_file)?.trim().to_string()
            } else {
                anyhow::bail!("{}", T("no_version_no_current"));
            }
        }
    };

    let version_bin = version_bin_dir(&nvm_dir.join(&resolved));
    let node_path = exe_path(&version_bin, "node");
    if !node_path.exists() {
        anyhow::bail!(
            "{}",
            format_t("not_installed", std::slice::from_ref(&resolved))
        );
    }

    // `corepack enable` writes pnpm/yarn/... shims. By default it targets a
    // system-wide bin directory, which is wrong for an nvm-managed install —
    // the shims must live inside this version's bin dir so they disappear when
    // the version is uninstalled. Scope the install directory explicitly.
    let corepack_path = exe_path(&version_bin, "corepack");
    let bin_arg = version_bin.display().to_string();
    // The corepack binary is a JS file run via `#!/usr/bin/env node`. Without
    // the version's `bin/` on PATH, `env node` won't find node and the spawn
    // fails with "node: No such file or directory" — silently, because
    // `.output()` swallows the lookup error. Same applies to the npm fallback
    // below.
    let path_env = prepend_to_path(&version_bin);

    let mut success = false;
    if corepack_path.exists() {
        let out = Command::new(&corepack_path)
            .args(["enable", "--install-directory", &bin_arg])
            .env("PATH", &path_env)
            .output();
        if let Ok(o) = out {
            success = o.status.success();
        }
    }

    // Fallback: corepack not bundled with this version. Install it via npm,
    // then re-run enable with the scoped install directory.
    if !success {
        let npm_path = exe_path(&version_bin, "npm");
        if npm_path.exists() {
            let npm_out = Command::new(&npm_path)
                .args(["install", "-g", "corepack"])
                .env("PATH", &path_env)
                .output();
            if let Ok(o) = npm_out {
                if o.status.success() && corepack_path.exists() {
                    let out = Command::new(&corepack_path)
                        .args(["enable", "--install-directory", &bin_arg])
                        .env("PATH", &path_env)
                        .output();
                    if let Ok(o) = out {
                        success = o.status.success();
                    }
                }
            }
        } else {
            anyhow::bail!(
                "{}",
                format_t("npm_not_found", std::slice::from_ref(&resolved))
            );
        }
    }

    // Trust the on-disk state, not the exit code: a successful exit doesn't
    // guarantee shims were actually written into the version's bin.
    let shims_present = ["pnpm", "yarn"]
        .iter()
        .any(|t| exe_path(&version_bin, t).exists());

    if success && shims_present {
        println!(
            "{} {} {}",
            "✓".green().bold(),
            T("corepack_enabled_for").green(),
            resolved.white().bold()
        );
    } else if shims_present {
        // Already enabled (e.g. shims pre-existed from a previous run).
        println!(
            "{} {} {}",
            "✓".green().bold(),
            T("corepack_enabled_for").green(),
            resolved.white().bold()
        );
    } else {
        anyhow::bail!(
            "{}",
            format_t("corepack_enable_failed", std::slice::from_ref(&resolved))
        );
    }

    Ok(())
}

pub fn corepack_disable(version: Option<&str>) -> anyhow::Result<()> {
    let nvm_dir = get_nvm_dir();
    let resolved = match version {
        Some(ver) => crate::config::resolve_alias(ver)?,
        None => {
            let current_file = nvm_dir.join("current");
            if current_file.exists() {
                fs::read_to_string(&current_file)?.trim().to_string()
            } else {
                anyhow::bail!("{}", T("no_version_no_current"));
            }
        }
    };

    let node_path = exe_path(&version_bin_dir(&nvm_dir.join(&resolved)), "node");
    if !node_path.exists() {
        anyhow::bail!(
            "{}",
            format_t("not_installed", std::slice::from_ref(&resolved))
        );
    }

    // First try the official `corepack disable` with an install-directory scoped
    // to this version's bin dir, so we only remove the shim entries created for
    // this version (and never touch a system-wide install).
    let version_bin = version_bin_dir(&nvm_dir.join(&resolved));
    let corepack_path = exe_path(&version_bin, "corepack");
    // `corepack disable` runs corepack (a JS file), which needs `node` on
    // PATH. Without this, the spawn silently fails with exit code 127 and we
    // fall through to the manual shim-removal fallback (which still works,
    // but loses corepack's own bookkeeping).
    let path_env = prepend_to_path(&version_bin);

    let output = Command::new(&corepack_path)
        .args([
            "disable",
            "--install-directory",
            &version_bin.display().to_string(),
        ])
        .env("PATH", &path_env)
        .output();

    let mut success = false;
    if let Ok(out) = output {
        success = out.status.success();
    }

    // Fallback: directly remove the well-known corepack-managed shims so the
    // version's bin dir no longer advertises pnpm/yarn. This mirrors what
    // `corepack disable` does on disk.
    if !success {
        let mut remove_failed = false;
        for tool in COREPACK_SHIMS {
            let shim = exe_path(&version_bin, tool);
            if !shim.exists() {
                continue;
            }
            // Only remove files that are actually corepack-managed shims. A
            // user-installed `pnpm`/`yarn` (e.g. via `npm i -g pnpm`) is a
            // real binary and must not be deleted. Corepack shims are tiny
            // JS wrappers that reference the `corepack` binary, so require
            // that marker in the file content before removing.
            let is_corepack_shim = fs::read_to_string(&shim)
                .map(|c| c.contains("corepack"))
                .unwrap_or(false);
            if !is_corepack_shim {
                continue;
            }
            if let Err(e) = fs::remove_file(&shim) {
                // Previously this was `let _ = fs::remove_file(...)`, which
                // swallowed file-lock/permission errors and then set
                // `success = true` unconditionally — reporting "Corepack
                // disabled" even when the shim was still on disk. Surface
                // the failure instead.
                eprintln!(
                    "{} {}",
                    "⚠".yellow().bold(),
                    format_t("corepack_remove_failed", &[tool.to_string(), e.to_string()])
                );
                remove_failed = true;
            }
        }
        // Only claim success when every corepack shim was actually removed.
        success = !remove_failed;
    }

    if success {
        println!(
            "{} {} {}",
            "✓".green().bold(),
            T("corepack_disabled_for").green(),
            resolved.white().bold()
        );
    } else {
        // Reached only when a shim could not be removed (the `corepack
        // disable` command failed and the manual fallback hit a
        // permission/lock error on at least one shim).
        println!(
            "{} {} {}",
            "ℹ".cyan().bold(),
            T("corepack_disable_partial").cyan(),
            resolved.white().bold()
        );
    }

    Ok(())
}

pub fn handle_corepack(action: Option<&str>, version: Option<&str>) -> anyhow::Result<()> {
    match action {
        Some("enable") => corepack_enable(version),
        Some("disable") => corepack_disable(version),
        Some("status") | None => corepack_status(version),
        _ => {
            println!("{} {}", "ℹ".cyan().bold(), T("corepack_usage").cyan());
            Ok(())
        }
    }
}
