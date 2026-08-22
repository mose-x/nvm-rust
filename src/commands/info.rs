//! Version switching (`use`, `auto`), version info display, aliases, mirror,
//! and language commands.
//!
//! The semver range engine, project-detection helpers, proxy commands,
//! lifecycle commands (deactivate/unload/uninstall), and run/exec/which
//! commands have been extracted into their own modules. This file re-exports
//! them so all existing `commands::*` call sites keep working.

use anyhow::{Context, Result};
use colored::Colorize;
use std::path::Path;
use std::process::Command;

use super::{get_base_url, get_codename, get_current_version};
use crate::config::{
    handle_mirror, list_all_aliases, load_config, remove_alias, resolve_alias, save_config,
    set_alias, update_shell_config,
};
use crate::i18n::{format_t, T};
use crate::system::{exe_path, get_nvm_dir, get_tags, version_bin_dir};
use crate::utils::{atomic_write, is_lts_version};

// Re-export extracted modules so `commands::*` call sites remain unchanged.
// `project_detect` functions are called from `use_version_silent` below;
// `lifecycle`/`run`/`proxy_cmd` are re-exported for backward compatibility.
pub use super::{lifecycle::*, proxy_cmd::*, run::*};
pub use crate::project_detect::*;

pub fn use_version(
    version: Option<&str>,
    install_if_missing: bool,
    save: bool,
    use_on_cd: bool,
) -> Result<()> {
    use_version_silent(version, install_if_missing, save, use_on_cd, false)
}

/// Same as use_version but optionally suppresses all human-facing output.
/// Used by `nvm auto --silent` (the cd hook) so switching versions on every
/// directory change does not flood the terminal.
pub fn use_version_silent(
    version: Option<&str>,
    install_if_missing: bool,
    save: bool,
    use_on_cd: bool,
    silent: bool,
) -> Result<()> {
    // `nvm use` (no arg): fall back to .nvmrc / .node-version /
    // package.json#engines.node lookup, mirroring nvm-sh. If none of those
    // are found, fall back to the `default` alias (a user-defined alias or
    // the --save'd default_version) before bailing -- this matches nvm-sh,
    // where bare `nvm use` switches to the default version when no .nvmrc
    // is present.
    //
    // Lookup priority matches nvm-sh user expectations: an explicit .nvmrc
    // wins over a package.json engines.node range, because .nvmrc is the
    // nvm-native config file and the user put it there on purpose. Only
    // when neither .nvmrc nor .node-version is present do we consult
    // package.json (a project-wide constraint, not a per-developer choice).
    let version = match version {
        Some(v) => v.to_string(),
        None => match find_nvmrc_recursive(silent)? {
            Some(v) => v,
            None => match find_package_json_node_version(silent)? {
                Some(v) => v,
                None => {
                    // No project-local version file: try the `default` alias
                    // (user `nvm alias default X` or `nvm use X --save`).
                    // `resolve_alias("default")` already bails with a clear
                    // "no default version set" if neither exists, so we only
                    // add the informational notice here.
                    match resolve_alias("default") {
                        Ok(v) => {
                            if !silent {
                                println!(
                                    "{} {} {}",
                                    "ℹ".cyan().bold(),
                                    T("no_nvmrc_using_default").cyan(),
                                    v.white()
                                );
                            }
                            v
                        }
                        Err(_) => {
                            if !silent {
                                println!("{} {}", "ℹ".cyan().bold(), T("no_nvmrc_found").cyan());
                            }
                            anyhow::bail!("{}", T("specify_version"));
                        }
                    }
                }
            },
        },
    };
    let resolved = resolve_alias(&version)?;
    let nvm_dir = get_nvm_dir();

    // Serialize the `current` write + optional install against concurrent
    // install/uninstall. Re-entrant: the inner `install` call below will
    // get a no-op guard instead of self-deadlocking.
    let _nvm_lock = crate::utils::acquire_nvm_lock(&nvm_dir)?;

    if resolved.starts_with("system:") {
        let current_file = nvm_dir.join("current");
        atomic_write(&current_file, &resolved).context(T("cannot_write_current"))?;
        // Remove active symlink so active/bin stops resolving to the old version.
        // Without this, current says "system:vX" but active/bin still serves
        // the previous version -- inconsistent state.
        if let Err(e) = crate::shim::remove_active_symlink(&nvm_dir) {
            eprintln!(
                "  {} failed to remove active symlink: {}",
                "⚠".yellow().bold(),
                e
            );
        }
        if !silent {
            println!(
                "{} {} {}",
                "✓".green().bold(),
                T("now_using").green().bold(),
                T("system_node").white().bold()
            );
        }
        return Ok(());
    }

    let version_dir = nvm_dir.join(&resolved);
    if !version_dir.exists() {
        if install_if_missing {
            if !silent {
                println!(
                    "{} {} {}",
                    "ℹ".cyan().bold(),
                    T("version").cyan(),
                    format_t(
                        "version_not_installed_installing",
                        std::slice::from_ref(&resolved)
                    )
                    .cyan()
                );
            }
            // Install the version
            super::install(super::InstallConfig {
                version: Some(resolved.clone()),
                lts: false,
                latest: false,
                lts_newer: false,
                offline: false,
                reinstall_packages_from: None,
                latest_npm: false,
                latest_yarn: false,
                latest_pnpm: false,
                source: false,
                no_gpg_verify: false,
            })?;
            // Check if installation succeeded
            if !nvm_dir.join(&resolved).exists() {
                anyhow::bail!(
                    "{}",
                    format_t("install_failed", std::slice::from_ref(&resolved))
                );
            }
        } else {
            anyhow::bail!(
                "{}",
                format_t("not_installed_run_install", std::slice::from_ref(&resolved))
            );
        }
    }

    let current_file = nvm_dir.join("current");
    atomic_write(&current_file, &resolved).context(T("cannot_write_current"))?;

    // Load config once for both the cd-hook flag and the --save default.
    // This load-modify-save MUST stay inside the nvm lock: two concurrent
    // `nvm use --save` calls both loading config, modifying different
    // fields, and saving would otherwise produce a lost update (the second
    // save overwrites the first, silently dropping one caller's change).
    // `atomic_write` only guarantees a single write is atomic, not the
    // whole read-modify-write transaction.
    let mut config = load_config()?;
    let cd_hook = if use_on_cd {
        config.use_on_cd = Some(true);
        true
    } else {
        config.use_on_cd.unwrap_or(false)
    };

    // Persist config only when something actually changed.
    if use_on_cd || save {
        if save {
            config.default_version = Some(resolved.clone());
        }
        save_config(&config)?;
    }

    // The lock guards the version-dir existence check, the optional install,
    // Drop the guard explicitly to release contention early -- but ONLY in
    // legacy mode where update_shell_config does the slow shell-rc rewrite.
    // In Full Shim mode (active symlink exists), the symlink update is instant
    // and must be protected by the lock to prevent races with concurrent
    // nvm use (current file and active symlink must stay consistent).
    let nvm_dir = crate::system::get_nvm_dir();
    let is_full_shim = crate::shim::active_exists(&nvm_dir);
    if !is_full_shim {
        drop(_nvm_lock);
    }

    // Skip rewriting the shell rc on cd-hook-triggered runs (silent=true):
    // the hook is already installed from the first `nvm use --use-on-cd`,
    // and rewriting it on every `cd` would read+backup+filter+write the
    // entire rc file each time -- a visible stall on directory changes.
    if !silent {
        // Full Shim mode: if active symlink exists, just update it.
        // The rc file has a fixed PATH (shims:active/bin:$PATH) that never
        // changes -- no rc rewrite needed, no "source" prompt.
        if is_full_shim {
            // Update symlink to new version (instant, no rc rewrite)
            if let Err(e) = crate::shim::update_active_symlink(&nvm_dir, &resolved) {
                eprintln!(
                    "  {} failed to update active symlink: {}",
                    "⚠".yellow().bold(),
                    e
                );
            }
            // Check if rc was reverted to old format by an old nvm version
            if let Ok(true) = crate::config::rc_has_version_specific_path() {
                if let Err(e) = crate::config::migrate_rc_to_shim_mode_with_dir(&nvm_dir) {
                    eprintln!("  {} failed to re-migrate rc: {}", "⚠".yellow().bold(), e);
                }
            }
            // No update_shell_config call, no "source" prompt
        } else {
            // Legacy mode: try lazy migration first
            match crate::shim::migrate_to_full_shim(&nvm_dir) {
                Ok(true) => {
                    // First migration succeeded -- print notice
                    println!("  {} {}", "✓".green().bold(), T("shim_migrated"));
                    if !silent {
                        println!(
                            "  {} {}",
                            T("tip_label").dimmed(),
                            T("shim_migrate_restart").dimmed()
                        );
                    }
                }
                _ => {
                    // Migration not applicable (no active version, or already migrated)
                    // Fall back to old behavior: rewrite rc + source prompt
                    update_shell_config(&resolved, cd_hook)?;
                }
            }
        }
    }

    // --save: report the persisted default.
    if save && !silent {
        println!(
            "  {} {}",
            "✓".green().bold(),
            format_t("default_saved", std::slice::from_ref(&resolved)).green()
        );
    }

    if use_on_cd && !silent {
        println!(
            "  {} {}",
            "✓".green().bold(),
            T("use_on_cd_enabled").green()
        );
    }

    if !silent {
        // Use the correct product name in the success message: io.js versions
        // (iojs-vX.Y.Z / io.js-vX.Y.Z) should say "io.js" rather than
        // "Node.js" so the output matches what `nvm install` prints.
        let product_msg = if crate::utils::is_iojs_version(&resolved) {
            T("now_using_iojs")
        } else {
            T("now_using_node")
        };
        println!(
            "{} {} {}",
            "✓".green().bold(),
            product_msg.green().bold(),
            resolved.white().bold()
        );
        // Only show "source" prompt in legacy mode (no active symlink).
        // In Full Shim mode, the active symlink routes global packages instantly.
        let nvm_dir = crate::system::get_nvm_dir();
        if !crate::shim::active_exists(&nvm_dir) {
            println!(
                "  {} {}",
                T("tip_label").dimmed(),
                T("tip_apply_shell").dimmed()
            );
        }
    }

    Ok(())
}

pub fn current_version() -> Result<()> {
    match get_current_version()? {
        Some(version) => {
            let resolved = version;
            if resolved.starts_with("system:") {
                println!(
                    "{} {}",
                    "system".cyan().bold(),
                    format!("({})", resolved.trim_start_matches("system:")).dimmed()
                );
            } else {
                let nvm_dir = get_nvm_dir();
                let node_path = exe_path(&version_bin_dir(&nvm_dir.join(&resolved)), "node");

                println!("{}", resolved.green().bold());

                // Single node invocation for node + npm (mirrors
                // show_version_info's probe, avoiding a second spawn).
                if let Some(parts) = probe_versions(&node_path) {
                    println!("  {} {}", T("node_label").dimmed(), parts[0].white());
                    if parts[1] != "none" {
                        println!("  {} {}", T("npm_label").dimmed(), parts[1].white());
                    }
                }
            }
        }
        None => println!("{} {}", "✗".red().bold(), T("no_active_use").red()),
    }

    Ok(())
}

pub fn auto_switch(silent: bool) -> Result<()> {
    // `nvm auto` is now an alias for `nvm use` (no arg): both look up the
    // version from .nvmrc / .node-version / package.json and switch. Keep
    // the explicit entry point so existing shell hooks (`nvm auto --silent`)
    // keep working.
    use_version_silent(None, false, false, false, silent)
}

/// Probe node/npm/yarn/pnpm versions in a single `node -p` invocation.
/// `-p` (not `-e`) is required: the probe script is an IIFE whose value must
/// be printed; `node -e` evaluates but discards the result, leaving stdout
/// empty and making every probe look like a failure.
/// Each tool is probed via `require.resolve`: if the package is installed
/// globally, resolve returns its path and we read the version from
/// `require().version`; otherwise we emit "none" so the caller can show an
/// install hint. The `-p` script resolves modules from the CWD, so bundled
/// packages (npm on Windows ships in the node dir, not the CWD) often fail
/// to resolve — any "none" gets a second chance via `probe_tool_version`,
/// which runs the tool binary directly (`npm.cmd --version` etc.).
/// Returns `None` if node itself is missing or the probe failed.
fn probe_versions(node_bin: &Path) -> Option<[String; 4]> {
    let probe_script = concat!(
        "(",
        "function(){",
        "function v(name){",
        "try{var p=require.resolve(name+'/package.json');",
        "return require(p).version||'none';",
        "}catch(e){return 'none'}",
        "}",
        "return [process.version,",
        "v('npm'),v('yarn'),v('pnpm')].join('|')",
        "}()",
        ")"
    );
    let out = Command::new(node_bin)
        .arg("-p")
        .arg(probe_script)
        .output()
        .ok()?;
    let line = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let parts: Vec<&str> = line.split('|').collect();
    if parts.len() != 4 {
        return None;
    }
    let mut result = [
        parts[0].to_string(),
        parts[1].to_string(),
        parts[2].to_string(),
        parts[3].to_string(),
    ];
    // Fallback for packages `require.resolve` can't see from the CWD:
    // npm on Windows (bundled in the node dir) and corepack-managed
    // pnpm/yarn (shims in bin/, not packages in node_modules/). Run the
    // binary directly with --version.
    for (slot, tool) in [(1, "npm"), (2, "yarn"), (3, "pnpm")] {
        if result[slot] == "none" {
            if let Some(ver) = probe_tool_version(node_bin, tool) {
                result[slot] = ver;
            }
        }
    }
    Some(result)
}

/// Probe a package manager's version by running its binary directly.
/// Fallback for tools `require.resolve` can't find from the CWD: npm
/// bundled in the Windows node dir, and corepack-managed pnpm/yarn (shims
/// in bin/, not packages in node_modules/).
fn probe_tool_version(node_bin: &Path, tool: &str) -> Option<String> {
    let bin_dir = node_bin.parent()?;
    let tool_path = exe_path(bin_dir, tool);
    if !tool_path.exists() {
        return None;
    }
    let path_env = bin_dir.display().to_string();
    let out = Command::new(&tool_path)
        .arg("--version")
        .env("PATH", &path_env)
        .output()
        .ok()?;
    if out.status.success() {
        let ver = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !ver.is_empty() {
            return Some(ver);
        }
    }
    None
}

pub fn show_version_info() -> Result<()> {
    match get_current_version()? {
        Some(v) if v.starts_with("system:") => {
            println!(
                "{} {}",
                T("system_node_label").cyan().bold(),
                v.trim_start_matches("system:").white()
            );
        }
        Some(v) => {
            let nvm_dir = get_nvm_dir();
            let bin = version_bin_dir(&nvm_dir.join(&v));
            let node_bin = exe_path(&bin, "node");
            println!(
                "{} {}",
                T("active_node_label").green().bold(),
                v.white().bold()
            );

            // LTS badge + codename (cheap, no spawn).
            if is_lts_version(&v) {
                let codename = get_codename(&v);
                let codename_str = if codename == "-" {
                    String::new()
                } else {
                    format!(
                        "  {} {}",
                        T("version_codename_label").dimmed(),
                        codename.magenta().bold()
                    )
                };
                println!("  {}{}", T("lts_badge").green(), codename_str);
            }

            // Single node invocation to get node + npm + yarn + pnpm versions
            // (probe_versions applies the binary --version fallback internally).
            if let Some(parts) = probe_versions(&node_bin) {
                // node
                println!("  {} {}", T("node_label").dimmed(), parts[0].white());
                // npm
                if parts[1] != "none" {
                    println!("  {} {}", T("npm_label").dimmed(), parts[1].white());
                }
                // yarn
                if parts[2] != "none" {
                    println!("  {} {}", T("yarn_label").dimmed(), parts[2].white());
                } else {
                    println!(
                        "  {} {} {}",
                        T("yarn_label").dimmed(),
                        T("version_not_installed").yellow(),
                        T("version_install_hint_yarn").dimmed()
                    );
                }
                // pnpm
                if parts[3] != "none" {
                    println!("  {} {}", T("pnpm_label").dimmed(), parts[3].white());
                } else {
                    println!(
                        "  {} {} {}",
                        T("pnpm_label").dimmed(),
                        T("version_not_installed").yellow(),
                        T("version_install_hint_pnpm").dimmed()
                    );
                }
            }

            // Binary-path (reuse which-style output, no extra spawn).
            if node_bin.exists() {
                println!(
                    "  {} {}",
                    T("version_path_label").dimmed(),
                    node_bin.display().to_string().white()
                );
            }
        }
        None => println!("{} {}", "✗".red().bold(), T("no_active_version_set").red()),
    }
    Ok(())
}

pub fn show_remote_version_info() -> Result<()> {
    let config = load_config()?;
    let base_url = get_base_url(&config);
    let tags = get_tags(base_url)?;

    let mut versions: Vec<String> = Vec::new();
    for tag in tags {
        if tag.starts_with("v") && tag.ends_with('/') {
            versions.push(tag.trim_end_matches('/').to_string());
        }
    }
    versions.sort_by(|a, b| crate::utils::compare_semver(b, a));

    println!();
    print!("  ");
    print!("{}", T("latest_remote_versions").cyan().bold());
    print!("  ");
    print!(
        "{}",
        format_t("remote_total_count", &[versions.len().to_string()]).dimmed()
    );
    println!();

    for v in versions.iter().take(5) {
        let is_lts = is_lts_version(v);
        let lts_mark = if is_lts {
            format!("  {} ", T("lts_badge").green())
        } else {
            "       ".to_string()
        };
        let codename = get_codename(v);
        let codename_str = if codename == "-" {
            "".to_string()
        } else {
            format!("  {}", codename.magenta())
        };
        println!(
            "    {}  {}{}{}",
            "│".dimmed(),
            v.white().bold(),
            lts_mark,
            codename_str
        );
    }
    println!();

    Ok(())
}

pub fn cmd_set_alias(name: &str, version: Option<&str>) -> Result<()> {
    set_alias(name, version)
}

pub fn cmd_remove_alias(name: &str) -> Result<()> {
    remove_alias(name)
}

pub fn cmd_list_aliases() -> Result<()> {
    list_all_aliases()
}

pub fn cmd_mirror(mirror: Option<&str>) -> Result<()> {
    handle_mirror(mirror)
}

// ---------------------------------------------------------------------------
// Language / i18n
// ---------------------------------------------------------------------------

pub fn cmd_language(lang: Option<&str>) -> Result<()> {
    use crate::i18n::{available_lang_codes, get_language, set_language, Lang};

    // Build the `<en|cn|jp>` hint and the `en, cn, jp` list once; both are
    // rendered from LANG_CODES so they stay in sync with whatever locale
    // files are present at build time.
    let codes = available_lang_codes();
    let usage_hint = codes.join("|");
    let available_list = codes.join(", ");

    match lang {
        Some(l) => {
            if let Some(parsed) = Lang::from_str(l) {
                set_language(parsed)?;
                // After switching the language, silently regenerate the
                // installed zsh/fish completions so their descriptions follow
                // the new language. The user no longer needs to run
                // `nvm completion` manually. Errors are swallowed: a failure
                // to update completions must not block the language switch.
                // Returns zsh_regenerated: zsh caches the completion function
                // in the shell process memory, so editing the file does not
                // take effect in the current shell -- the user must be prompted
                // to refresh manually.
                let zsh_regenerated =
                    crate::completions::regenerate_completions_if_installed().unwrap_or(false);
                println!(
                    "  {} {} {}",
                    "✓".green().bold(),
                    T("language_set_label").green(),
                    parsed.display_name().white().bold()
                );
                // Only prompt when the zsh completion was actually rewritten.
                // bash/fish/powershell re-read the file on every completion, so
                // no prompt is needed; if zsh completion is not installed we
                // also stay silent to avoid misleading the user.
                if zsh_regenerated {
                    println!();
                    println!(
                        "  {} {}",
                        "ℹ".cyan().bold(),
                        T("lang_switched_zsh_reload_note")
                    );
                    println!(
                        "    {}",
                        "unfunction _nvm 2>/dev/null; autoload -Uz _nvm"
                            .yellow()
                            .bold()
                    );
                    println!("  {}", T("lang_switched_or_new_shell"));
                }
            } else {
                anyhow::bail!(
                    "{}",
                    format_t("lang_unknown", &[l.to_string(), available_list.to_string()])
                );
            }
        }
        None => {
            let current = get_language();
            println!();
            println!(
                "  {} {} {}",
                "▶".cyan().bold(),
                T("current_language_label").cyan(),
                current.display_name().white().bold()
            );
            println!(
                "  {} {}",
                "→".dimmed(),
                format_t("lang_usage", std::slice::from_ref(&usage_hint)).dimmed()
            );
            println!();
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The probe script is an IIFE: only `node -p` prints its return value.
    /// The fake node below emits nothing for `-e` and the version string for
    /// `-p`, so reverting the flag to `-e` makes this test fail.
    #[cfg(unix)]
    #[test]
    fn probe_versions_requires_print_flag() {
        let dir = std::env::temp_dir().join(format!("nvm_probe_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let node = dir.join("node");
        std::fs::write(
            &node,
            "#!/bin/sh\n\
             if [ \"$1\" = \"-p\" ]; then\n\
             echo 'v20.0.0|10.0.0|none|none'\n\
             fi\n",
        )
        .unwrap();
        std::process::Command::new("chmod")
            .arg("755")
            .arg(&node)
            .status()
            .unwrap();

        let parts = probe_versions(&node).expect("probe_versions should parse the -p output");
        assert_eq!(parts[0], "v20.0.0");
        assert_eq!(parts[1], "10.0.0");
        assert_eq!(parts[2], "none");
        assert_eq!(parts[3], "none");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[cfg(windows)]
    #[test]
    fn probe_versions_parses_pipe_separated_output() {
        let dir = std::env::temp_dir().join(format!("nvm_probe_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let node = dir.join("node.cmd");
        // Only print under -p, like the unix fake: reverting the flag to -e
        // leaves stdout empty and fails this test on Windows CI too.
        std::fs::write(
            &node,
            "@echo off\r\nif /i not \"%~1\"==\"-p\" exit /b 0\r\necho v20.0.0^|10.0.0^|none^|none\r\n",
        )
        .unwrap();

        let parts = probe_versions(&node).expect("probe_versions should parse the output");
        assert_eq!(parts[0], "v20.0.0");
        assert_eq!(parts[1], "10.0.0");
        assert_eq!(parts[2], "none");
        assert_eq!(parts[3], "none");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// Malformed output (not 4 pipe-separated fields) must yield None, not a
    /// partially filled or panicking result.
    #[cfg(unix)]
    #[test]
    fn probe_versions_garbage_output_returns_none() {
        let dir = std::env::temp_dir().join(format!("nvm_probe_garbage_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let node = dir.join("node");
        std::fs::write(&node, "#!/bin/sh\necho 'not|enough'\n").unwrap();
        std::process::Command::new("chmod")
            .arg("755")
            .arg(&node)
            .status()
            .unwrap();

        assert!(probe_versions(&node).is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[cfg(windows)]
    #[test]
    fn probe_versions_garbage_output_returns_none() {
        let dir = std::env::temp_dir().join(format!("nvm_probe_garbage_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let node = dir.join("node.cmd");
        std::fs::write(&node, "@echo off\r\necho not^|enough\r\n").unwrap();

        assert!(probe_versions(&node).is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn probe_versions_missing_node_returns_none() {
        let missing = std::env::temp_dir().join(format!(
            "nvm_probe_missing_{}_{}/node",
            std::process::id(),
            u32::MAX
        ));
        assert!(probe_versions(&missing).is_none());
    }

    /// The `-p` script resolves modules from the CWD, so bundled npm
    /// (Windows: shipped inside the node dir) fails require.resolve and
    /// reports "none". probe_versions must then fall back to running
    /// `npm --version` directly — without this, `nvm version` silently
    /// dropped the npm line while yarn/pnpm (which had the fallback) showed.
    #[cfg(unix)]
    #[test]
    fn probe_versions_npm_fallback_to_binary() {
        let dir = std::env::temp_dir().join(format!("nvm_probe_npmfb_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let node = dir.join("node");
        std::fs::write(
            &node,
            "#!/bin/sh\n\
             if [ \"$1\" = \"-p\" ]; then\n\
             echo 'v20.0.0|none|none|none'\n\
             fi\n",
        )
        .unwrap();
        let npm = dir.join("npm");
        std::fs::write(&npm, "#!/bin/sh\necho 10.9.0\n").unwrap();
        std::process::Command::new("chmod")
            .arg("755")
            .arg(&node)
            .status()
            .unwrap();
        std::process::Command::new("chmod")
            .arg("755")
            .arg(&npm)
            .status()
            .unwrap();

        let parts = probe_versions(&node).expect("probe should succeed");
        assert_eq!(parts[1], "10.9.0", "npm must fall back to the binary");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[cfg(windows)]
    #[test]
    fn probe_versions_npm_fallback_to_binary() {
        let dir = std::env::temp_dir().join(format!("nvm_probe_npmfb_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let node = dir.join("node.cmd");
        std::fs::write(
            &node,
            "@echo off\r\nif /i not \"%~1\"==\"-p\" exit /b 0\r\necho v20.0.0^|none^|none^|none\r\n",
        )
        .unwrap();
        let npm = dir.join("npm.cmd");
        std::fs::write(&npm, "@echo off\r\necho 10.9.0\r\n").unwrap();

        let parts = probe_versions(&node).expect("probe should succeed");
        assert_eq!(parts[1], "10.9.0", "npm must fall back to npm.cmd");

        std::fs::remove_dir_all(&dir).ok();
    }
}
