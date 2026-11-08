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

/// Global packages to migrate out of `npm list -g --depth=0 --json` output:
/// the top-level `dependencies` keys minus `npm` and `corepack`, which ship
/// with Node and are not user-installed.
///
/// Returns `None` when npm reported no `dependencies` object at all, which the
/// caller treats as "nothing to do" and stays silent about. That is distinct
/// from `Some(vec![])` — an empty list still prints a zero count — so the
/// `Option` preserves the existing behaviour rather than collapsing two cases.
///
/// Extracted from `reinstall_packages_inner`, where the same filter ran twice
/// (once for the count, once for the install loop) and the two could disagree.
/// This is also the only part of the flow testable without spawning npm.
pub(crate) fn packages_to_migrate(json: &serde_json::Value) -> Option<Vec<String>> {
    const BUNDLED_WITH_NODE: [&str; 2] = ["npm", "corepack"];
    let deps = json.get("dependencies")?.as_object()?;
    let mut packages: Vec<String> = deps
        .keys()
        .filter(|name| !BUNDLED_WITH_NODE.contains(&name.as_str()))
        .cloned()
        .collect();
    // serde_json's Map is a BTreeMap unless the `preserve_order` feature is on
    // (it is not, and no dependency enables it), so keys are already sorted.
    // Sorting explicitly keeps migration order — and the tests — independent
    // of that feature set.
    packages.sort();
    Some(packages)
}

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
    if let Some(packages) = packages_to_migrate(&json) {
        let new_path = prepend_to_path(&version_bin_dir(&to_dir));
        println!(
            "  {} {}",
            "ℹ".cyan().bold(),
            format_t("reinstall_count", &[packages.len().to_string()])
        );
        let mut migrated = 0usize;
        let mut failed: Vec<String> = Vec::new();
        for pkg in &packages {
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

#[cfg(test)]
mod tests {
    use super::packages_to_migrate;
    use serde_json::json;

    #[test]
    fn excludes_packages_bundled_with_node() {
        let json = json!({
            "dependencies": {
                "npm": {"version": "10.8.0"},
                "corepack": {"version": "0.29.0"},
                "typescript": {"version": "5.6.0"},
                "eslint": {"version": "9.0.0"}
            }
        });
        assert_eq!(
            packages_to_migrate(&json).expect("dependencies object is present"),
            vec!["eslint".to_string(), "typescript".to_string()]
        );
    }

    #[test]
    fn exact_name_match_only() {
        // A user package whose name merely contains "npm" or "corepack" must
        // still be migrated; a `starts_with`/`contains` filter would drop it.
        let json = json!({
            "dependencies": {
                "npm": {},
                "npm-check-updates": {},
                "corepack": {},
                "corepack-foo": {}
            }
        });
        assert_eq!(
            packages_to_migrate(&json).unwrap(),
            vec!["corepack-foo".to_string(), "npm-check-updates".to_string()]
        );
    }

    #[test]
    fn only_bundled_packages_is_empty_list_not_none() {
        // Distinct from a missing `dependencies` object: this still prints a
        // zero count, the missing case stays silent.
        let json = json!({"dependencies": {"npm": {}, "corepack": {}}});
        assert_eq!(packages_to_migrate(&json), Some(vec![]));
    }

    #[test]
    fn missing_or_malformed_dependencies_is_none() {
        // Every shape npm could produce that isn't a dependencies object must
        // collapse to None so the caller keeps its "nothing to do" silence
        // instead of printing a bogus count.
        for json in [
            json!({}),
            json!({"dependencies": null}),
            json!({"dependencies": "nope"}),
            json!({"dependencies": []}),
            json!({"dependencies": 42}),
        ] {
            assert_eq!(packages_to_migrate(&json), None, "input was {json}");
        }
    }

    #[test]
    fn ignores_nested_and_sibling_sections() {
        // `npm list -g --depth=0` only emits top-level deps, but a deeper
        // listing must not cause transitive packages to be reinstalled as
        // globals, and `devDependencies` is not a global-install concept.
        let json = json!({
            "name": "probe",
            "dependencies": {
                "eslint": {"version": "9.0.0", "dependencies": {"chalk": {"version": "5.0.0"}}}
            },
            "devDependencies": {"vitest": {"version": "2.0.0"}}
        });
        assert_eq!(
            packages_to_migrate(&json).unwrap(),
            vec!["eslint".to_string()]
        );
    }

    #[test]
    fn output_order_is_deterministic() {
        // Migration runs `npm install -g` in this order, so a nondeterministic
        // ordering would make failures irreproducible.
        let json = json!({
            "dependencies": {"zod": {}, "axios": {}, "mid": {}}
        });
        let packages = packages_to_migrate(&json).unwrap();
        assert_eq!(
            packages,
            vec!["axios".to_string(), "mid".to_string(), "zod".to_string()]
        );
    }
}
