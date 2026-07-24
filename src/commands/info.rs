use anyhow::{Context, Result};
use colored::Colorize;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::{compare_versions, get_base_url, get_codename, get_current_version};
use crate::config::{
    handle_mirror, list_all_aliases, load_config, remove_alias, remove_from_shell_config,
    resolve_alias, save_config, set_alias, update_shell_config,
};
use crate::i18n::{format_t, T};
use crate::system::{exe_path, get_nvm_dir, get_tags, prepend_to_path, version_bin_dir};
use crate::utils::{atomic_write, get_installed_versions, is_lts_version, pad_right};

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
    // the --save'd default_version) before bailing — this matches nvm-sh,
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
                format_t(
                    "not_installed_run_install",
                    &[resolved.clone(), resolved.clone()]
                )
            );
        }
    }

    let current_file = nvm_dir.join("current");
    atomic_write(&current_file, &resolved).context(T("cannot_write_current"))?;

    // Load config once for both the cd-hook flag and the --save default.
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

    // Skip rewriting the shell rc on cd-hook-triggered runs (silent=true):
    // the hook is already installed from the first `nvm use --use-on-cd`,
    // and rewriting it on every `cd` would read+backup+filter+write the
    // entire rc file each time — a visible stall on directory changes.
    if !silent {
        update_shell_config(&resolved, cd_hook)?;
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
        println!(
            "  {} {}",
            T("tip_label").dimmed(),
            T("tip_apply_shell").dimmed()
        );
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

pub fn deactivate() -> Result<()> {
    let nvm_dir = get_nvm_dir();
    let current_file = nvm_dir.join("current");
    if current_file.exists() {
        fs::remove_file(&current_file)?;
    }
    println!("{} {}", "✓".green().bold(), T("deactivated").green());
    Ok(())
}

pub fn unload() -> Result<()> {
    remove_from_shell_config()
}

pub fn run_version(version: &str, args: &[String]) -> Result<()> {
    let resolved = resolve_alias(version)?;
    let nvm_dir = get_nvm_dir();

    let node_path = if resolved.starts_with("system:") {
        PathBuf::from("node")
    } else {
        exe_path(&version_bin_dir(&nvm_dir.join(&resolved)), "node")
    };

    if !resolved.starts_with("system:") && !node_path.exists() {
        anyhow::bail!(
            "{}",
            format_t("not_installed", std::slice::from_ref(&resolved))
        );
    }

    let status = Command::new(&node_path)
        .args(args)
        .status()
        .context(T("execution_failed"))?;

    std::process::exit(status.code().unwrap_or(1));
}

pub fn exec_version(version: &str, args: &[String]) -> Result<()> {
    if args.is_empty() {
        anyhow::bail!("{}", T("specify_command"));
    }

    let resolved = resolve_alias(version)?;
    let nvm_dir = get_nvm_dir();

    let bin_dir = if resolved.starts_with("system:") {
        match crate::utils::find_system_node_path() {
            Some(node_path) => match node_path.parent() {
                Some(parent) => parent.to_path_buf(),
                None => anyhow::bail!("{}", T("system_node_not_found")),
            },
            None => anyhow::bail!("{}", T("system_node_not_found")),
        }
    } else {
        // Verify the requested version is actually installed, so we never
        // silently fall back to a system node found later on PATH.
        let version_dir = nvm_dir.join(&resolved);
        if !version_dir.exists() {
            anyhow::bail!(
                "{}",
                format_t(
                    "not_installed_run_install",
                    &[resolved.clone(), resolved.clone()]
                )
            );
        }
        version_bin_dir(&nvm_dir.join(&resolved))
    };

    let cmd = &args[0];
    let cmd_args = &args[1..];

    let new_path = prepend_to_path(&bin_dir);

    // `Command::new(cmd).status()` fails synchronously when `cmd` is not on
    // PATH (or is not an executable). The raw io::Error surfaces as
    // "No such file or directory (os error 2)", which is confusing because it
    // doesn't name the command the user typed. Detect that specific case and
    // bail with an i18n message that includes `cmd`.
    let status = Command::new(cmd)
        .args(cmd_args)
        .env("PATH", &new_path)
        .status()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                anyhow::anyhow!(
                    "{}",
                    format_t("exec_command_not_found", std::slice::from_ref(cmd))
                )
            } else {
                anyhow::Error::new(e).context(T("execution_failed"))
            }
        })?;

    std::process::exit(status.code().unwrap_or(1));
}

pub fn which_version(version: Option<&str>) -> Result<()> {
    let resolved = match version {
        Some(v) => resolve_alias(v)?,
        None => match get_current_version()? {
            Some(v) => v,
            None => anyhow::bail!("{}", T("no_current_version_set")),
        },
    };

    if resolved.starts_with("system:") {
        if let Some(node_path) = crate::utils::find_system_node_path() {
            println!("{}", node_path.display().to_string().white().bold());
            return Ok(());
        }
        anyhow::bail!("{}", T("system_node_not_found"));
    }

    let nvm_dir = get_nvm_dir();
    let node_path = exe_path(&version_bin_dir(&nvm_dir.join(&resolved)), "node");

    if !node_path.exists() {
        anyhow::bail!(
            "{}",
            format_t("not_installed", std::slice::from_ref(&resolved))
        );
    }

    println!("{}", node_path.display().to_string().white().bold());
    Ok(())
}

pub fn auto_switch(silent: bool) -> Result<()> {
    // `nvm auto` is now an alias for `nvm use` (no arg): both look up the
    // version from .nvmrc / .node-version / package.json and switch. Keep
    // the explicit entry point so existing shell hooks (`nvm auto --silent`)
    // keep working.
    use_version_silent(None, false, false, false, silent)
}

/// Recursively search for .nvmrc or .node-version file from current directory up to root
fn find_nvmrc_recursive(silent: bool) -> Result<Option<String>> {
    let current_dir = std::env::current_dir()?;
    let mut dir = current_dir.as_path();

    // Read the first non-comment, non-empty line from a .nvmrc /
    // .node-version file. nvm-sh itself only reads the first line, but many
    // real-world .nvmrc files start with a `# comment` (editor templates,
    // per-project docs) — without this filter the comment text would be
    // passed to resolve_alias and produce a confusing error like
    // "Version v# comment\nv18.20.4 is not installed".
    let read_first_version_line = |path: &Path| -> Option<String> {
        let content = fs::read_to_string(path).ok()?;
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if trimmed.starts_with('#') {
                continue;
            }
            return Some(trimmed.to_string());
        }
        None
    };

    loop {
        let nvmrc = dir.join(".nvmrc");
        if nvmrc.exists() {
            if let Some(version) = read_first_version_line(&nvmrc) {
                if !silent {
                    println!(
                        "{} {} {}",
                        "ℹ".cyan().bold(),
                        T("found_nvmrc").cyan(),
                        dir.display().to_string().dimmed()
                    );
                }
                return Ok(Some(version));
            }
        }

        let node_version = dir.join(".node-version");
        if node_version.exists() {
            if let Some(version) = read_first_version_line(&node_version) {
                if !silent {
                    println!(
                        "{} {} {}",
                        "ℹ".cyan().bold(),
                        T("found_node_version").cyan(),
                        dir.display().to_string().dimmed()
                    );
                }
                return Ok(Some(version));
            }
        }

        // Move to parent directory
        dir = match dir.parent() {
            Some(parent) => parent,
            None => break,
        };

        // Stop at filesystem root
        if dir.parent().is_none() {
            break;
        }
    }

    Ok(None)
}

/// Find Node.js version from package.json engines.node field.
///
/// `engines.node` may be:
/// - a bare version:        `"22.0.0"` or `"v22.0.0"`
/// - a range expression:    `">=18.0.0"`, `"^20.11.0"`, `"~22.0.0"`,
///   `"22.x"`, `"22 || 20"`, etc.
/// - the wildcard `"*"` / `"x"` / `""`  (no preference)
///
/// For ranges we pick the newest locally installed version that satisfies the
/// range. If none is installed we return the range expression itself verbatim,
/// so the caller can show a helpful "not installed, run nvm install <ver>"
/// message (matching the original behavior for bare versions).
fn find_package_json_node_version(silent: bool) -> Result<Option<String>> {
    let current_dir = std::env::current_dir()?;
    let package_json = current_dir.join("package.json");

    if !package_json.exists() {
        return Ok(None);
    }

    let content = fs::read_to_string(&package_json)?;
    // A malformed package.json shouldn't crash auto-detection — skip it and
    // fall through to the .nvmrc/.node-version lookup.
    let json: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return Ok(None),
    };

    let raw = match json
        .get("engines")
        .and_then(|e| e.get("node"))
        .and_then(|n| n.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    {
        Some(v) => v,
        None => return Ok(None),
    };

    // "lts/*", "lts", "node", "stable", "latest" — resolve as aliases against
    // the installed set: lts/* → newest LTS installed, node/stable/latest →
    // newest installed. Falls through to range parsing if not an alias.
    let installed = get_installed_versions();

    // Resolve alias-like expressions before range parsing so that "lts/*" /
    // "lts" don't get misinterpreted as version strings.
    let lower = raw.to_lowercase();
    if lower == "lts" || lower == "lts/*" || lower == "lts/-1" {
        let mut lts: Vec<String> = installed
            .iter()
            .filter(|v| is_lts_version(v))
            .cloned()
            .collect();
        lts.sort_by(|a, b| compare_versions(a, b));
        if let Some(chosen) = lts.last() {
            if !silent {
                println!(
                    "{} {} {} {}",
                    "ℹ".cyan().bold(),
                    T("found_engines_node").cyan(),
                    raw.white().bold(),
                    format!("→ {}", chosen).dimmed()
                );
            }
            return Ok(Some(chosen.clone()));
        }
        // No LTS installed — surface the alias so use_version reports it.
        return Ok(Some(raw));
    }
    if lower == "node" || lower == "stable" || lower == "latest" || lower == "*" || lower == "x" {
        if let Some(chosen) = installed
            .iter()
            .max_by(|a, b| compare_versions(a, b))
            .cloned()
        {
            if !silent {
                println!(
                    "{} {} {} {}",
                    "ℹ".cyan().bold(),
                    T("found_engines_node").cyan(),
                    raw.white().bold(),
                    format!("→ {}", chosen).dimmed()
                );
            }
            return Ok(Some(chosen));
        }
        return Ok(None);
    }

    // Try to satisfy as a range expression. This also handles bare versions
    // with wildcards ("22.x"), unions ("22 || 20"), compound (" >=20 <22 "),
    // caret/tilde, and operator-prefixed forms. If it resolves to an installed
    // version we return that; otherwise we fall back to the raw expression so
    // use_version prints the standard "not installed" hint.
    if let Some(chosen) = pick_version_for_range(&raw, &installed) {
        if !silent {
            println!(
                "{} {} {} {}",
                "ℹ".cyan().bold(),
                T("found_engines_node").cyan(),
                raw.white().bold(),
                format!("→ {}", chosen).dimmed()
            );
        }
        return Ok(Some(chosen));
    }

    // Plain bare version like "22.0.0" or "v22.0.0" — pass through verbatim.
    if raw.starts_with(|c: char| c.is_ascii_digit() || c == 'v') && !raw.contains(' ') {
        return Ok(Some(raw));
    }

    // Nothing installed satisfies the range and it isn't a bare version. Surface
    // the original constraint so the user sees what was requested.
    Ok(Some(raw))
}

/// Best-effort semver-ish range matcher. Supports `>=`, `>`, `<=`, `<`, `^`,
/// `~`, `x`/`*` wildcards, `||` unions, and space-separated compound ranges
/// (e.g. `>=20 <22` means both must hold). Picks the highest installed
/// version that satisfies the constraint.
fn pick_version_for_range(range: &str, installed: &[String]) -> Option<String> {
    if installed.is_empty() {
        return None;
    }

    // Union: "a || b"
    let ors: Vec<&str> = range.split("||").map(|s| s.trim()).collect();
    let mut candidates: Vec<String> = Vec::new();
    for part in &ors {
        // Within a union arm, space-separated tokens form an AND:
        // ">=20 <22" means both >=20 AND <22 must hold.
        let tokens: Vec<&str> = part.split_whitespace().collect();
        if tokens.is_empty() {
            continue;
        }
        if tokens.len() == 1 {
            if let Some(v) = pick_version_for_range_single(tokens[0], installed) {
                candidates.push(v);
            }
            continue;
        }
        // Compound AND: keep only installed versions satisfying every token.
        let mut matching: Vec<String> = installed
            .iter()
            .filter(|v| tokens.iter().all(|t| version_matches_simple(t, v)))
            .cloned()
            .collect();
        if !matching.is_empty() {
            matching.sort_by(|a, b| compare_versions(a, b));
            candidates.push(matching.pop().unwrap());
        }
    }
    candidates.into_iter().max_by(|a, b| compare_versions(a, b))
}

/// Lightweight single-token matcher used by the compound AND branch above.
/// `token` is one of `>=`, `>`, `<=`, `<`, `^`, `~`, `=`, or a bare version.
fn version_matches_simple(token: &str, version: &str) -> bool {
    let (op, rest) = if let Some(r) = token.strip_prefix(">=") {
        (">=", r)
    } else if let Some(r) = token.strip_prefix("<=") {
        ("<=", r)
    } else if let Some(r) = token.strip_prefix('>') {
        (">", r)
    } else if let Some(r) = token.strip_prefix('<') {
        ("<", r)
    } else if let Some(r) = token.strip_prefix('=') {
        ("=", r)
    } else if let Some(r) = token.strip_prefix('^') {
        ("^", r)
    } else if let Some(r) = token.strip_prefix('~') {
        ("~", r)
    } else {
        ("=", token)
    };
    let rest = rest.trim().trim_start_matches('v');
    let comps: Vec<&str> = rest.split('.').collect();
    let wild = comps.iter().any(|c| *c == "x" || *c == "X" || *c == "*");
    version_matches_op(version, op, rest, wild)
}

fn pick_version_for_range_single(expr: &str, installed: &[String]) -> Option<String> {
    let expr = expr.trim();
    if expr.is_empty() {
        return None;
    }

    // Parse operator + remainder
    let (op, rest) = if let Some(r) = expr.strip_prefix(">=") {
        (">=", r.trim_start())
    } else if let Some(r) = expr.strip_prefix("<=") {
        ("<=", r.trim_start())
    } else if let Some(r) = expr.strip_prefix('>') {
        (">", r.trim_start())
    } else if let Some(r) = expr.strip_prefix('<') {
        ("<", r.trim_start())
    } else if let Some(r) = expr.strip_prefix('=') {
        ("=", r.trim_start())
    } else if let Some(r) = expr.strip_prefix('^') {
        ("^", r.trim_start())
    } else if let Some(r) = expr.strip_prefix('~') {
        ("~", r.trim_start())
    } else {
        ("=", expr)
    };

    let rest = rest.trim().trim_start_matches('v').to_string();
    if rest.is_empty() || rest == "*" || rest == "x" || rest == "X" {
        // Match any — pick newest installed
        return installed
            .iter()
            .max_by(|a, b| compare_versions(a, b))
            .cloned();
    }

    // Detect wildcard in major.minor.patch, e.g. "22.x", "22.*", "20.11.x"
    let comps: Vec<&str> = rest.split('.').collect();
    let wild = comps.iter().any(|c| *c == "x" || *c == "X" || *c == "*");

    // A bare major like "22" (no dots) is shorthand for "22.x.x" — treat as
    // wildcard so `22 || 20` matches any installed 22.x or 20.x.
    let effective_wild = wild || (!rest.contains('.') && op == "=");
    let effective_rest = if effective_wild && !rest.contains('.') && op == "=" {
        format!("{}.x", rest)
    } else {
        rest
    };

    let mut matching: Vec<String> = installed
        .iter()
        .filter(|v| version_matches_op(v, op, &effective_rest, effective_wild))
        .cloned()
        .collect();

    if matching.is_empty() {
        return None;
    }
    matching.sort_by(|a, b| compare_versions(a, b));
    matching.pop() // newest
}

fn parse_v_tuple(v: &str) -> Option<(u64, u64, u64)> {
    let (maj, min, pat) = crate::utils::parse_version_parts(v)?;
    Some((maj as u64, min as u64, pat as u64))
}

fn version_matches_op(version: &str, op: &str, target: &str, wildcard: bool) -> bool {
    let (maj, min, pat) = match parse_v_tuple(version) {
        Some(t) => t,
        None => return false,
    };
    let comps: Vec<&str> = target.split('.').collect();
    let t_maj: u64 = comps.first().and_then(|s| s.parse().ok()).unwrap_or(0);
    let t_min: u64 = comps.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    let t_pat: u64 = comps.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);

    match op {
        ">=" => maj > t_maj || (maj == t_maj && (min > t_min || (min == t_min && pat >= t_pat))),
        ">" => maj > t_maj || (maj == t_maj && (min > t_min || (min == t_min && pat > t_pat))),
        "<=" => maj < t_maj || (maj == t_maj && (min < t_min || (min == t_min && pat <= t_pat))),
        "<" => maj < t_maj || (maj == t_maj && (min < t_min || (min == t_min && pat < t_pat))),
        "^" => {
            // Compatible with: same major, >= target
            if maj != t_maj {
                return false;
            }
            (min, pat) >= (t_min, t_pat)
        }
        "~" => {
            // Same major.minor, >= target patch
            if maj != t_maj || min != t_min {
                return false;
            }
            pat >= t_pat
        }
        _ => {
            // "=" — exact, or wildcard match
            if wildcard {
                if comps
                    .first()
                    .map(|s| *s == "x" || *s == "X" || *s == "*")
                    .unwrap_or(true)
                {
                    return false; // shouldn't happen — handled above
                }
                if maj != t_maj {
                    return false;
                }
                if comps.len() > 1 {
                    let m = comps[1];
                    if !(m == "x" || m == "X" || m == "*") {
                        let m: u64 = m.parse().unwrap_or(0);
                        if min != m {
                            return false;
                        }
                    }
                }
                if comps.len() > 2 {
                    let p = comps[2];
                    if !(p == "x" || p == "X" || p == "*") {
                        let p: u64 = p.parse().unwrap_or(0);
                        if pat != p {
                            return false;
                        }
                    }
                }
                true
            } else {
                (maj, min, pat) == (t_maj, t_min, t_pat)
            }
        }
    }
}

/// Probe node/npm/yarn/pnpm versions in a single `node -e` invocation.
/// Each tool is probed via `require.resolve`: if the package is installed
/// globally, resolve returns its path and we read the version from
/// `require().version`; otherwise we emit "none" so the caller can show an
/// install hint. Returns `None` if node itself is missing or the probe failed.
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
        "(process.versions.npm||'none'),",
        "v('yarn'),v('pnpm')].join('|')",
        "}()",
        ")"
    );
    let out = Command::new(node_bin)
        .arg("-e")
        .arg(probe_script)
        .output()
        .ok()?;
    let line = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let parts: Vec<&str> = line.split('|').collect();
    if parts.len() == 4 {
        Some([
            parts[0].to_string(),
            parts[1].to_string(),
            parts[2].to_string(),
            parts[3].to_string(),
        ])
    } else {
        None
    }
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

            // Single node invocation to get node + npm + yarn + pnpm versions.
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
    let tags = get_tags(base_url.to_string());

    let mut versions: Vec<String> = Vec::new();
    for tag in tags {
        if tag.starts_with("v") && tag.ends_with('/') {
            versions.push(tag.trim_end_matches('/').to_string());
        }
    }
    versions.sort_by(|a, b| compare_versions(b, a));

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
                println!(
                    "  {} {} {}",
                    "✓".green().bold(),
                    T("language_set_label").green(),
                    parsed.display_name().white().bold()
                );
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
                format_t("lang_usage", &[usage_hint.to_string()]).dimmed()
            );
            println!();
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Proxy management
// ---------------------------------------------------------------------------

pub fn cmd_proxy(action: Option<&str>) -> Result<()> {
    use crate::proxy::{get_system_proxy, proxy_status, set_proxy_enabled, test_connectivity};

    match action {
        Some("on") => {
            let sys_proxy = get_system_proxy();
            if sys_proxy.is_none() {
                println!();
                println!(
                    "  {} {}",
                    "⚠".yellow().bold(),
                    T("proxy_no_system_proxy").yellow()
                );
                println!("  {} {}", "→".dimmed(), T("proxy_set_env_vars").dimmed());
                println!();
                return Ok(());
            }

            // Enable proxy first, so the connectivity test routes through it.
            set_proxy_enabled(true)?;

            // Test connectivity via the now-enabled proxy.
            println!("  {} {}", "›".dimmed(), T("testing_connectivity"));
            let (baidu_ok, google_ok) = test_connectivity();

            if baidu_ok || google_ok {
                println!();
                println!(
                    "  {} {} {}",
                    "✓".green().bold(),
                    T("proxy_enabled").green(),
                    T("proxy_will_be_used").green()
                );
                print!("    ");
                if baidu_ok {
                    print!("{}  ", T("proxy_test_baidu_ok").green());
                } else {
                    print!("{}  ", T("proxy_test_baidu_fail").red());
                }
                if google_ok {
                    println!("{}", T("proxy_test_google_ok").green());
                } else {
                    println!("{}", T("proxy_test_google_fail").red());
                }
                println!();
            } else {
                // Proxy did not work; roll back so downloads do not hang.
                set_proxy_enabled(false)?;
                println!();
                println!(
                    "  {} {}",
                    "⚠".yellow().bold(),
                    T("neither_reachable").yellow()
                );
                println!("  {} {}", "→".dimmed(), T("check_proxy_settings").dimmed());
                println!();
            }
        }
        Some("off") => {
            set_proxy_enabled(false)?;
            println!();
            println!("  {} {}", "✓".green().bold(), T("proxy_disabled").green());
            println!();
        }
        Some(other) => {
            anyhow::bail!("{}", format_t("unknown_action", &[other.to_string()]));
        }
        None => {
            let status = proxy_status();
            let sys_proxy = status.system_proxy.clone();

            println!();
            println!("  {}", T("proxy_status_title").cyan().bold());
            println!();

            // NVM proxy toggle. `pad_right` correctly handles ANSI-coloured
            // labels (it strips escape codes when measuring width), so the
            // two rows line up regardless of which color the label uses.
            const STATUS_COL: usize = 10;
            let nvm_state = if status.nvm_proxy_enabled {
                T("proxy_state_on").green().bold().to_string()
            } else {
                T("proxy_state_off").red().bold().to_string()
            };
            println!(
                "    {} {}",
                pad_right(&"nvm:".dimmed().to_string(), STATUS_COL),
                nvm_state
            );

            // System proxy env
            let sys_state = match &sys_proxy {
                Some(p) => format!("{}", p.as_str().dimmed()),
                None => T("not_set").red().to_string(),
            };
            println!(
                "    {} {}",
                pad_right(&"system:".dimmed().to_string(), STATUS_COL),
                sys_state
            );

            println!();

            if status.nvm_proxy_enabled {
                if sys_proxy.is_some() {
                    println!("  {} {}", "✓".green().bold(), T("proxy_active").green());
                } else {
                    println!(
                        "  {} {}",
                        "⚠".yellow().bold(),
                        T("proxy_on_no_env").yellow()
                    );
                }
            } else {
                println!("  {} {}", "ℹ".cyan().bold(), T("proxy_off_direct").cyan());
            }

            println!();
            println!(
                "  {} {}",
                T("usage_label").dimmed(),
                T("proxy_usage_hint").yellow().bold()
            );
            println!();
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Unit tests for the hand-rolled semver range matcher
// (`pick_version_for_range`, `version_matches_simple`,
// `pick_version_for_range_single`, `version_matches_op`, `parse_v_tuple`).
// This is the highest-risk code in the project (no external semver crate),
// so the tests pin every operator and wildcard edge case.
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    fn installed() -> Vec<String> {
        vec![
            "v18.20.0".to_string(),
            "v20.11.0".to_string(),
            "v20.11.1".to_string(),
            "v22.5.0".to_string(),
        ]
    }

    // --- caret (^) ---------------------------------------------------------
    #[test]
    fn caret_picks_newest_in_same_major() {
        let r = pick_version_for_range("^20.10.0", &installed());
        assert_eq!(r.as_deref(), Some("v20.11.1"));
    }

    #[test]
    fn caret_rejects_lower_patch() {
        let r = pick_version_for_range("^20.11.5", &installed());
        assert_eq!(r, None);
    }

    #[test]
    fn caret_rejects_lower_minor_in_same_major() {
        // ^18.21.0 requires >=18.21.0 in major 18; v18.20.0 is too old.
        assert_eq!(pick_version_for_range("^18.21.0", &installed()), None);
    }

    // --- tilde (~) ---------------------------------------------------------
    #[test]
    fn tilde_locks_major_minor() {
        let r = pick_version_for_range("~20.11.0", &installed());
        assert_eq!(r.as_deref(), Some("v20.11.1"));
    }

    #[test]
    fn tilde_rejects_different_minor() {
        let r = pick_version_for_range("~20.12.0", &installed());
        assert_eq!(r, None);
    }

    // --- comparison operators ---------------------------------------------
    #[test]
    fn ge_picks_newest_satisfying() {
        let r = pick_version_for_range(">=20.0.0", &installed());
        assert_eq!(r.as_deref(), Some("v22.5.0"));
    }

    #[test]
    fn gt_strictly_greater() {
        let r = pick_version_for_range(">22.5.0", &installed());
        assert_eq!(r, None);
        let r2 = pick_version_for_range(">20.11.0", &installed());
        assert_eq!(r2.as_deref(), Some("v22.5.0"));
    }

    #[test]
    fn le_picks_newest_below_bound() {
        let r = pick_version_for_range("<=20.11.1", &installed());
        assert_eq!(r.as_deref(), Some("v20.11.1"));
    }

    #[test]
    fn lt_strictly_less() {
        let r = pick_version_for_range("<22.5.0", &installed());
        assert_eq!(r.as_deref(), Some("v20.11.1"));
    }

    // --- exact (=) ---------------------------------------------------------
    #[test]
    fn exact_match() {
        let r = pick_version_for_range("=20.11.0", &installed());
        assert_eq!(r.as_deref(), Some("v20.11.0"));
    }

    #[test]
    fn bare_version_is_exact() {
        let r = pick_version_for_range("20.11.1", &installed());
        assert_eq!(r.as_deref(), Some("v20.11.1"));
    }

    #[test]
    fn v_prefix_stripped() {
        let r = pick_version_for_range("v20.11.1", &installed());
        assert_eq!(r.as_deref(), Some("v20.11.1"));
    }

    // --- wildcards (x / *) -------------------------------------------------
    #[test]
    fn wildcard_major_matches_newest_of_major() {
        let r = pick_version_for_range("20.x", &installed());
        assert_eq!(r.as_deref(), Some("v20.11.1"));
    }

    #[test]
    fn wildcard_star_matches_newest_of_major() {
        let r = pick_version_for_range("20.*", &installed());
        assert_eq!(r.as_deref(), Some("v20.11.1"));
    }

    #[test]
    fn wildcard_minor_pin_patch() {
        // 20.11.x → both 20.11.0 and 20.11.1 match → newest
        let r = pick_version_for_range("20.11.x", &installed());
        assert_eq!(r.as_deref(), Some("v20.11.1"));
    }

    #[test]
    fn bare_major_is_wildcard() {
        // "22" → 22.x.x → matches v22.5.0
        let r = pick_version_for_range("22", &installed());
        assert_eq!(r.as_deref(), Some("v22.5.0"));
    }

    #[test]
    fn star_alone_matches_any() {
        let r = pick_version_for_range("*", &installed());
        assert_eq!(r.as_deref(), Some("v22.5.0"));
    }

    // --- union (||) --------------------------------------------------------
    #[test]
    fn union_picks_newest_across_arms() {
        let r = pick_version_for_range("^18 || ^22", &installed());
        assert_eq!(r.as_deref(), Some("v22.5.0"));
    }

    #[test]
    fn union_with_no_matching_arm() {
        let r = pick_version_for_range("^17 || ^19", &installed());
        assert_eq!(r, None);
    }

    // --- compound AND ------------------------------------------------------
    #[test]
    fn compound_and_intersection() {
        // >=20 AND <22 → both 20.x match → newest is v20.11.1
        let r = pick_version_for_range(">=20 <22", &installed());
        assert_eq!(r.as_deref(), Some("v20.11.1"));
    }

    #[test]
    fn compound_and_empty_intersection() {
        let r = pick_version_for_range(">=21 <22", &installed());
        assert_eq!(r, None);
    }

    // --- edge cases --------------------------------------------------------
    #[test]
    fn empty_installed_returns_none() {
        let empty: Vec<String> = Vec::new();
        assert_eq!(pick_version_for_range("^20", &empty), None);
        assert_eq!(pick_version_for_range("*", &empty), None);
    }

    #[test]
    fn no_match_returns_none() {
        assert_eq!(pick_version_for_range("^99", &installed()), None);
    }

    // --- parse_v_tuple -----------------------------------------------------
    #[test]
    fn parse_v_tuple_v_prefixed() {
        assert_eq!(parse_v_tuple("v20.11.1"), Some((20, 11, 1)));
    }

    #[test]
    fn parse_v_tuple_bare() {
        assert_eq!(parse_v_tuple("18.20.0"), Some((18, 20, 0)));
    }

    #[test]
    fn parse_v_tuple_iojs_prefix() {
        assert_eq!(parse_v_tuple("iojs-v3.3.1"), Some((3, 3, 1)));
    }

    #[test]
    fn parse_v_tuple_iojs_dot_prefix() {
        // Previously a bug: parse_v_tuple missed "io.js-v" / "io.js-" prefixes,
        // making io.js versions invisible to the engines.node range matcher.
        assert_eq!(parse_v_tuple("io.js-v3.3.1"), Some((3, 3, 1)));
        assert_eq!(parse_v_tuple("io.js-3.3.1"), Some((3, 3, 1)));
    }

    #[test]
    fn parse_v_tuple_trailing_suffix() {
        // "v20.11.1-rc.1" → (20, 11, 1)
        assert_eq!(parse_v_tuple("v20.11.1-rc.1"), Some((20, 11, 1)));
    }

    #[test]
    fn parse_v_tuple_missing_patch_defaults_zero() {
        assert_eq!(parse_v_tuple("v22"), Some((22, 0, 0)));
    }

    #[test]
    fn iojs_dot_prefix_matches_engines_range() {
        // Regression: an installed "io.js-3.3.1" used to be invisible to
        // `package.json#engines.node` range matching because parse_v_tuple
        // returned None for the "io.js-" prefix.
        let installed = vec!["io.js-3.3.1".to_string()];
        assert_eq!(
            pick_version_for_range(">=3.0.0", &installed),
            Some("io.js-3.3.1".to_string())
        );
    }
}
