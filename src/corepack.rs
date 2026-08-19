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

/// Print corepack status for a system-installed Node.js.
///
/// `resolved` is the canonical `system:vX.Y.Z` form returned by
/// `resolve_alias("system")`. There is no version-scoped `bin/` dir to probe
/// (system Node lives outside `NVM_DIR`), so we just report the active system
/// node version and probe for a system-wide `corepack` binary.
///
/// Shared by `corepack_status(Some("system"))`, `corepack_status(None)` when
/// `current` is `system:…`, and previously inlined as the fall-through block
/// of the `None` arm — extracting it ensures all three entry points print the
/// same output for the same state instead of diverging by accident.
fn corepack_system_status(resolved: &str) -> anyhow::Result<()> {
    let system_ver = resolved.trim_start_matches("system:");
    if !system_ver.is_empty() {
        println!(
            "{} {} {} ({})",
            "ℹ".cyan().bold(),
            T("system_node").cyan(),
            "node".white().bold(),
            system_ver.dimmed()
        );
    }
    // Probe the system-wide `corepack` binary. `Command::new("corepack")`
    // succeeds if the spawn worked, but the child may still exit non-zero
    // (broken install, permission error) — guard both cases so we don't
    // print "System corepack: <empty>" for a broken install. The previous
    // form only checked the spawn and printed whatever was on stdout,
    // including an empty string on non-zero exit.
    match Command::new("corepack").arg("--version").output() {
        Ok(o) if o.status.success() => {
            let version_str = String::from_utf8_lossy(&o.stdout).trim().to_string();
            println!(
                "{} {} {}",
                "ℹ".cyan().bold(),
                T("system_corepack").cyan(),
                version_str.white().bold()
            );
        }
        _ => {
            println!("{} {}", "ℹ".cyan().bold(), T("corepack_no_version").cyan());
        }
    }
    Ok(())
}

pub fn corepack_status(version: Option<&str>) -> anyhow::Result<()> {
    let nvm_dir = get_nvm_dir();

    match version {
        Some(ver) => {
            let resolved = crate::config::resolve_alias(ver)?;

            // `nvm corepack status system` (or an alias resolving to system):
            // there is no version-scoped bin dir to probe, so fall through to
            // the system-wide corepack check. Without this branch the code
            // below would join `nvm_dir/system:v20.0.0`, find no `node`
            // binary there, and bail with `not_installed` — masking the fact
            // that the user explicitly asked about the system install.
            if resolved.starts_with("system:") {
                return corepack_system_status(&resolved);
            }

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
                if current.starts_with("system:") {
                    // Current is the system Node.js — probe system-wide
                    // corepack instead of looking for a version-scoped bin.
                    return corepack_system_status(&current);
                }
                return corepack_status(Some(&current));
            }

            // No current version set: still report whether corepack is
            // available system-wide so the user knows what they have.
            corepack_system_status("system:")?;
        }
    }

    Ok(())
}

/// Resolved target for a corepack enable/disable operation.
///
/// Built by `resolve_corepack_target` and consumed by both `corepack_enable`
/// and `corepack_disable` so the version-resolution + node-presence-check +
/// bin/corepack_path/path_env computation lives in exactly one place. The
/// previous enable/disable pair each duplicated this ~25-line block; keeping
/// them in sync by hand was fragile (an enable-only fix to `path_env` would
/// silently miss disable, and vice versa).
struct CorepackTarget {
    resolved: String,
    version_bin: std::path::PathBuf,
    corepack_path: std::path::PathBuf,
    /// PATH with this version's bin dir prepended. `corepack` is a JS file run
    /// via `#!/usr/bin/env node`, so without the version's `bin/` on PATH the
    /// spawn silently fails with exit 127 ("node: No such file or directory")
    /// and the command falls through to its fallback path.
    path_env: String,
}

/// Resolve `version` (or fall back to the `current` symlink) and compute the
/// corepack target paths. Bails with `not_installed` if the resolved version's
/// `node` binary is missing, with `no_version_no_current` if no version was
/// given and `current` is unset, and with `corepack_system_not_supported` if
/// the resolved version is the system Node.js — `corepack enable`/`disable`
/// write shims into a version-scoped bin dir that does not exist for system
/// installs, and running them with the default system-wide target would leak
/// shims outside nvm's management (they would persist after `nvm deactivate`
/// and survive an uninstall).
fn resolve_corepack_target(
    nvm_dir: &std::path::Path,
    version: Option<&str>,
) -> anyhow::Result<CorepackTarget> {
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

    if resolved.starts_with("system:") {
        anyhow::bail!(
            "{}",
            format_t(
                "corepack_system_not_supported",
                std::slice::from_ref(&resolved)
            )
        );
    }

    let version_bin = version_bin_dir(&nvm_dir.join(&resolved));
    let node_path = exe_path(&version_bin, "node");
    if !node_path.exists() {
        anyhow::bail!(
            "{}",
            format_t("not_installed", std::slice::from_ref(&resolved))
        );
    }
    let corepack_path = exe_path(&version_bin, "corepack");
    let path_env = prepend_to_path(&version_bin);
    Ok(CorepackTarget {
        resolved,
        version_bin,
        corepack_path,
        path_env,
    })
}

pub fn corepack_enable(version: Option<&str>) -> anyhow::Result<()> {
    let nvm_dir = get_nvm_dir();
    let CorepackTarget {
        resolved,
        version_bin,
        corepack_path,
        path_env,
    } = resolve_corepack_target(&nvm_dir, version)?;

    // `corepack enable` writes pnpm/yarn/... shims. By default it targets a
    // system-wide bin directory, which is wrong for an nvm-managed install —
    // the shims must live inside this version's bin dir so they disappear when
    // the version is uninstalled. Scope the install directory explicitly.
    let bin_arg = version_bin.display().to_string();

    let mut success = false;
    if corepack_path.exists() {
        match Command::new(&corepack_path)
            .args(["enable", "--install-directory", &bin_arg])
            .env("PATH", &path_env)
            .output()
        {
            Ok(o) => {
                success = o.status.success();
                if !success {
                    // Surface corepack's own stderr so the user can see why
                    // enable refused (e.g. EPERM on the shim dir, conflicting
                    // global install). The previous code discarded both the
                    // non-zero exit reason and any spawn error silently.
                    let stderr = String::from_utf8_lossy(&o.stderr);
                    if !stderr.trim().is_empty() {
                        eprintln!("{} corepack enable: {}", "⚠".yellow().bold(), stderr.trim());
                    }
                }
            }
            Err(e) => {
                // Spawn failure (e.g. node missing despite the PATH patch
                // above, exec format error). Don't silently fall through to
                // the npm re-install fallback — that masks the real cause
                // and re-downloads corepack for nothing.
                eprintln!(
                    "{} corepack enable (spawn failed): {}",
                    "⚠".yellow().bold(),
                    e
                );
            }
        }
    }

    // Fallback: corepack not bundled with this version. Install it via npm,
    // then re-run enable with the scoped install directory.
    if !success {
        let npm_path = exe_path(&version_bin, "npm");
        if npm_path.exists() {
            match Command::new(&npm_path)
                .args(["install", "-g", "corepack"])
                .env("PATH", &path_env)
                .output()
            {
                Ok(o) => {
                    if o.status.success() && corepack_path.exists() {
                        match Command::new(&corepack_path)
                            .args(["enable", "--install-directory", &bin_arg])
                            .env("PATH", &path_env)
                            .output()
                        {
                            Ok(o) => success = o.status.success(),
                            Err(e) => eprintln!("{} corepack enable: {}", "⚠".yellow().bold(), e),
                        }
                    } else if !o.status.success() {
                        let stderr = String::from_utf8_lossy(&o.stderr);
                        eprintln!(
                            "{} npm install -g corepack: {}",
                            "⚠".yellow().bold(),
                            stderr.trim()
                        );
                    }
                }
                Err(e) => eprintln!(
                    "{} npm install -g corepack (spawn failed): {}",
                    "⚠".yellow().bold(),
                    e
                ),
            }
        } else {
            anyhow::bail!(
                "{}",
                format_t("npm_not_found", std::slice::from_ref(&resolved))
            );
        }
    }

    // Trust the on-disk state, not the exit code: a successful exit doesn't
    // guarantee shims were actually written into the version's bin. Use the
    // shared `COREPACK_SHIMS` list (the file-top constant) rather than a
    // hand-picked subset — the previous `["pnpm", "yarn"]` diverged from the
    // "single source" promise in the constant's doc comment and would silently
    // miss a future shim added there.
    let shims_present = COREPACK_SHIMS
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
        // Shims exist, but verify they're actually corepack-managed (not
        // leftover npm-installed shims from a previous fallback).
        let is_corepack_shim = COREPACK_SHIMS.iter().any(|t| {
            let p = exe_path(&version_bin, t);
            p.exists()
                && std::fs::read_to_string(&p)
                    .map(|c| c.contains("corepack"))
                    .unwrap_or(false)
        });
        if is_corepack_shim {
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
    let CorepackTarget {
        resolved,
        version_bin,
        corepack_path,
        path_env,
    } = resolve_corepack_target(&nvm_dir, version)?;

    // First try the official `corepack disable` with an install-directory scoped
    // to this version's bin dir, so we only remove the shim entries created for
    // this version (and never touch a system-wide install).

    let output = Command::new(&corepack_path)
        .args([
            "disable",
            "--install-directory",
            &version_bin.display().to_string(),
        ])
        .env("PATH", &path_env)
        .output();

    let mut success = false;
    match output {
        Ok(out) => {
            success = out.status.success();
            if !success {
                // Surface corepack's stderr so the user can see why disable
                // refused, instead of silently falling back to manual shim
                // removal (which skips corepack's own bookkeeping).
                let stderr = String::from_utf8_lossy(&out.stderr);
                if !stderr.trim().is_empty() {
                    eprintln!(
                        "{} corepack disable: {}",
                        "⚠".yellow().bold(),
                        stderr.trim()
                    );
                }
            }
        }
        Err(e) => {
            // Spawn failure — corepack missing or node not on PATH despite
            // the patch above. The manual fallback below still removes the
            // shims, but log the cause so the user can diagnose.
            eprintln!(
                "{} corepack disable (spawn failed): {}",
                "⚠".yellow().bold(),
                e
            );
        }
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
