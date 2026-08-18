//! `nvm reinstall-packages` command and its `reinstall_packages_inner` helper
//! (also invoked by the `--reinstall-packages-from` post-install hook in
//! [`super::install`]).
//!
//! Reads the global package list from the source version's npm, then runs
//! `npm install -g <pkg>` against the target version's npm for each entry,
//! skipping the bundled `npm`/`corepack` packages.

use anyhow::{Context, Result};
use colored::Colorize;
use std::io::{self, Write};
use std::process::Command;

use crate::i18n::{format_t, T};
use crate::system::{exe_path, get_nvm_dir, prepend_to_path, version_bin_dir};

pub(crate) fn reinstall_packages_inner(from: &str, to: &str) -> Result<()> {
    let nvm_dir = get_nvm_dir();
    let from_dir = nvm_dir.join(from);
    if !from_dir.exists() {
        anyhow::bail!("{}", format_t("source_not_installed", &[from.to_string()]));
    }
    let to_dir = nvm_dir.join(to);
    if !to_dir.exists() {
        anyhow::bail!("{}", format_t("target_not_installed", &[to.to_string()]));
    }
    let from_npm = exe_path(&version_bin_dir(&from_dir), "npm");
    let to_npm = exe_path(&version_bin_dir(&to_dir), "npm");

    let output = Command::new(&from_npm)
        .arg("list")
        .arg("-g")
        .arg("--depth=0")
        .arg("--json")
        .env("PATH", prepend_to_path(&version_bin_dir(&from_dir)))
        .output()
        .context(T("list_global_packages_failed"))?;

    // `npm list --json` only writes the dependency tree to stdout on exit
    // success. On a non-zero exit (broken install, corrupt node_modules, npm
    // crash) stdout is empty or an error blob, so the previous
    // `from_str(...).unwrap_or_default()` silently produced `Null` and
    // `reinstall-packages` reported "0 packages migrated" instead of failing.
    // Bail explicitly so the user sees the real cause.
    if !output.status.success() {
        let code = output.status.code().unwrap_or(-1);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = stderr.trim();
        let msg = format_t("npm_list_failed_code", &[code.to_string()]);
        if detail.is_empty() {
            anyhow::bail!("{}", msg);
        } else {
            anyhow::bail!("{}: {}", msg, detail);
        }
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    // Surface JSON parse errors instead of silently treating them as "no
    // dependencies" (which produced a misleading "0 packages migrated"
    // success). A non-zero exit code is already handled above; reaching
    // here means npm exited 0 but emitted something we couldn't parse —
    // typically a truncated pipe or a corrupted/different npm binary.
    let json: serde_json::Value = serde_json::from_str(&stdout).map_err(|e| {
        anyhow::anyhow!(
            "{}: {}",
            format_t("npm_list_parse_failed", &[e.to_string()]),
            T("npm_list_parse_failed_hint")
        )
    })?;
    if let Some(deps) = json.get("dependencies").and_then(|d| d.as_object()) {
        let new_path = prepend_to_path(&version_bin_dir(&to_dir));
        // Exclude npm/corepack from the count: they are bundled, not migrated.
        let pkg_count = deps
            .keys()
            .filter(|k| *k != "npm" && *k != "corepack")
            .count();
        println!(
            "  {} {}",
            "ℹ".cyan().bold(),
            format_t("reinstall_count", &[pkg_count.to_string()])
        );
        let mut migrated = 0usize;
        let mut failed: Vec<String> = Vec::new();
        for pkg in deps.keys() {
            if pkg == "npm" || pkg == "corepack" {
                continue;
            }
            print!("    {} {}... ", "•".cyan(), pkg);
            io::stdout().flush().ok();
            let status = match Command::new(&to_npm)
                .arg("install")
                .arg("-g")
                .arg(pkg)
                .env("PATH", &new_path)
                .status()
            {
                Ok(s) => s,
                Err(e) => {
                    // Surface the spawn error instead of a bare ✗. A
                    // non-zero npm exit prints its own code below; a spawn
                    // failure (e.g. `npm` not executable, EPERM, disk full
                    // fork) previously printed just "✗" with no reason,
                    // leaving the user unable to tell why the package was
                    // skipped.
                    println!(
                        "{} {}",
                        "✗".red().bold(),
                        format_t("package_failed_spawn", &[e.to_string()]).red()
                    );
                    failed.push(pkg.clone());
                    continue;
                }
            };
            if status.success() {
                println!("{}", "✓".green().bold());
                migrated += 1;
            } else {
                println!(
                    "{} {}",
                    "✗".red().bold(),
                    format_t(
                        "package_failed_code",
                        &[status.code().unwrap_or(-1).to_string()]
                    )
                    .red()
                );
                failed.push(pkg.clone());
            }
        }
        println!(
            "    {} {} {} {}",
            "✓".green().bold(),
            format_t("packages_migrated", &[migrated.to_string()]).green(),
            "→".dimmed(),
            to.white().bold()
        );
        if !failed.is_empty() {
            anyhow::bail!(
                "{}",
                format_t("reinstall_failed_list", &[failed.join(", ")])
            );
        }
    }
    Ok(())
}

pub fn reinstall_packages(from_version: &str) -> Result<()> {
    // Resolve aliases (default, lts/iron, bare "22.22.2", etc.) so the
    // user can pass the same kind of identifier they would to `nvm use`.
    let resolved_from = crate::config::resolve_alias(from_version)?;
    // Validate the source version *before* requiring a current version: the
    // user-facing input is "from_version", and a missing current is a setup
    // problem that should be reported only if the source is otherwise valid.
    let nvm_dir = get_nvm_dir();
    let from_dir = nvm_dir.join(&resolved_from);
    if !from_dir.exists() {
        anyhow::bail!(
            "{}",
            format_t("source_not_installed", std::slice::from_ref(&resolved_from))
        );
    }
    let current = super::get_current_version()?
        .ok_or_else(|| anyhow::anyhow!("{}", T("no_current_version_set")))?;
    reinstall_packages_inner(&resolved_from, &current)
}
