use anyhow::{Context, Result};
use colored::Colorize;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::{get_base_url, get_codename, get_current_version};
use crate::config::{
    handle_mirror, list_all_aliases, load_config, remove_alias, remove_from_shell_config,
    resolve_alias, save_config, set_alias, update_shell_config,
};
use crate::i18n::{format_t, T};
use crate::system::{exe_path, get_nvm_dir, get_tags, prepend_to_path, version_bin_dir};
use crate::utils::{atomic_write, get_installed_versions, is_lts_version, pad_right};

/// Exit the process with the exit status of a child command.
///
/// When the child was terminated by a signal (e.g. SIGINT, SIGTERM), the
/// shell convention is to exit with `128 + signal_number`. The previous
/// `status.code().unwrap_or(1)` collapsed every signal death into exit
/// code `1`, so a script could not distinguish "command failed" from
/// "command was killed" (e.g. by Ctrl-C). On non-Unix targets there is no
/// signal information available, so we keep the legacy `1` fallback there.
fn exit_with_status(status: std::process::ExitStatus) -> ! {
    let code = status.code().unwrap_or_else(|| {
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            // `signal()` returns `None` only if the process exited normally,
            // which contradicts `code()` returning `None` — but fall back to
            // `1` defensively in case a platform reports neither.
            status.signal().map(|s| 128 + s).unwrap_or(1)
        }
        #[cfg(not(unix))]
        {
            1
        }
    });
    std::process::exit(code);
}

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
    // the `current` file write, AND the config read-modify-write above — all
    // the nvm-state mutations. Everything below (shell rc rewrite, success
    // messages) touches files outside nvm's own state or uses its own
    // atomic_write, so holding the lock through it only serializes
    // concurrent `nvm use`/`nvm install` callers during the slow shell-rc
    // rewrite (backup + read + filter + write). Drop the guard explicitly
    // to release contention early — AFTER config save, BEFORE shell rc.
    drop(_nvm_lock);

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
    // Remove directly and treat NotFound as success instead of `exists()` +
    // `remove_file`: the two-step form is a TOCTOU race (a concurrent
    // `nvm use`/`uninstall` could remove `current` between the stat and the
    // unlink, surfacing as a confusing error), and deactivation is a no-op
    // when nothing is active anyway.
    if let Err(e) = fs::remove_file(&current_file) {
        if e.kind() != std::io::ErrorKind::NotFound {
            return Err(e.into());
        }
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

    let (node_path, bin_dir) = if resolved.starts_with("system:") {
        (PathBuf::from("node"), None)
    } else {
        let bin = version_bin_dir(&nvm_dir.join(&resolved));
        (exe_path(&bin, "node"), Some(bin))
    };

    if !resolved.starts_with("system:") && !node_path.exists() {
        anyhow::bail!(
            "{}",
            format_t("not_installed", std::slice::from_ref(&resolved))
        );
    }

    // Prepend the version's bin dir to PATH so child processes spawned by the
    // script (e.g. `child_process.exec('npm install')`) resolve npm/npx from
    // THIS version, not the parent shell's PATH. Matches `exec_version` and
    // nvm-sh's `nvm run` semantics. Without this, `nvm run 20 app.js` that
    // shells out to `npm` would use a different npm (or none).
    let mut cmd = Command::new(&node_path);
    cmd.args(args);
    if let Some(bin) = bin_dir {
        // `prepend_to_path` always returns a usable PATH string (it falls
        // back to the current PATH when the env var is unset), so there is
        // no error case to guard here — just set it unconditionally.
        let new_path = prepend_to_path(&bin);
        cmd.env("PATH", new_path);
    }
    let status = cmd.status().context(T("execution_failed"))?;

    exit_with_status(status);
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

    exit_with_status(status);
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

/// Mask the `user:pass@` userinfo in a proxy URL before printing it.
///
/// Proxy URLs commonly embed credentials (`http://user:pass@host:port`).
/// Printing such a URL to stdout leaks the password into terminal scrollback,
/// CI logs, and screen recordings. We replace the userinfo segment with
/// `***@`, preserving the scheme/host/port for diagnostics while hiding the
/// secret. URLs without userinfo are returned unchanged.
fn redact_proxy_credentials(url: &str) -> String {
    // Match `scheme://[userinfo@]host[:port]/path?query#frag`. userinfo ends
    // at the LAST `@` before the first `/`/`?`/`#` (authority terminator).
    // Using the last `@` handles passwords that themselves contain `@`
    // (`http://user:p@ss@host` -> userinfo=`user:p@ss`). Restricting to the
    // authority segment avoids mis-treating `@` in path/query as userinfo
    // (`http://host/path@evil` must NOT be redacted).
    if let Some(scheme_end) = url.find("://") {
        let after_scheme = &url[scheme_end + 3..];
        // Find end of authority: first of `/`, `?`, `#`, or end of string.
        let auth_end = after_scheme
            .find(['/', '?', '#'])
            .unwrap_or(after_scheme.len());
        let authority = &after_scheme[..auth_end];
        if let Some(at) = authority.rfind('@') {
            let userinfo = &authority[..at];
            if !userinfo.is_empty() {
                let rest_after_userinfo = &after_scheme[at..]; // includes '@'
                return format!("{}{}{}", &url[..scheme_end + 3], "***", rest_after_userinfo);
            }
        }
    }
    url.to_string()
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
        // Distinguish "file not present" (expected, return None) from real
        // read errors (permission denied, I/O error). The previous `.ok()?`
        // lumped them together, so a `.nvmrc` that exists but is unreadable
        // was silently treated as absent — the user got "no .nvmrc found"
        // instead of a clear permission error.
        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
            Err(e) => {
                eprintln!(
                    "{} {} {}: {}",
                    "⚠".yellow().bold(),
                    path.display(),
                    T("nvmrc_read_failed"),
                    e
                );
                return None;
            }
        };
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
        // `read_first_version_line` already maps NotFound -> None, so a
        // pre-check with `.exists()` would be a redundant stat call and a
        // TOCTOU window (file could be created/removed between the check
        // and the read). Just try the read directly.
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

        let node_version = dir.join(".node-version");
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

        // Move to parent directory. `Path::parent()` returns `None` for the
        // filesystem root (`/` on Unix, `C:\` on Windows), which terminates
        // the walk.
        //
        // The previous code also had a redundant `if dir.parent().is_none()
        // { break; }` guard *after* this reassignment — that broke one step
        // early: after moving from `/a` to `/`, the guard fired and the loop
        // exited *before* the next iteration's body checked `/.nvmrc`, so a
        // version file at the filesystem root was silently ignored. The
        // `match ... None => break` here is sufficient and correct.
        dir = match dir.parent() {
            Some(parent) => parent,
            None => break,
        };
    }

    Ok(None)
}

/// Find Node.js version from the closest `package.json` with an `engines.node`
/// field, walking up from the current directory to the filesystem root.
///
/// Mirrors `find_nvmrc_recursive`'s walk-up semantics so `nvm use` from a
/// sub-directory of a project picks up the project-root `package.json`
/// constraint, just as it would for `.nvmrc` / `.node-version`. A
/// `package.json` that exists but has no `engines.node` does *not* terminate
/// the search — this is what makes the common monorepo layout (root
/// `package.json` declares `engines.node` for the whole repo, sub-packages
/// don't) work without requiring every sub-package to repeat the constraint.
///
/// The first `package.json` (closest to cwd) with a non-empty `engines.node`
/// wins; lower levels are not consulted.
fn find_package_json_node_version(silent: bool) -> Result<Option<String>> {
    let current_dir = std::env::current_dir()?;
    let mut dir = current_dir.as_path();

    loop {
        let package_json = dir.join("package.json");
        // Read the file directly without a pre-check `.exists()` — that would
        // be a redundant stat + a TOCTOU window. Distinguish NotFound (no
        // package.json at this level — keep walking up) from real read errors
        // (permission denied, I/O) so the user gets a warning instead of a
        // silent "no engines.node found".
        let content = match fs::read_to_string(&package_json) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                dir = match dir.parent() {
                    Some(parent) => parent,
                    None => break,
                };
                continue;
            }
            Err(e) => {
                eprintln!(
                    "{} {} {}: {}",
                    "⚠".yellow().bold(),
                    package_json.display(),
                    T("package_json_read_failed"),
                    e
                );
                dir = match dir.parent() {
                    Some(parent) => parent,
                    None => break,
                };
                continue;
            }
        };

        // A malformed package.json at this level shouldn't crash auto-
        // detection or shadow a valid one higher up — skip and try the parent.
        let json: serde_json::Value = match serde_json::from_str(&content) {
            Ok(v) => v,
            Err(_) => {
                dir = match dir.parent() {
                    Some(parent) => parent,
                    None => break,
                };
                continue;
            }
        };

        let raw = match json
            .get("engines")
            .and_then(|e| e.get("node"))
            .and_then(|n| n.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
        {
            Some(v) => v,
            None => {
                // package.json exists but has no engines.node — keep walking
                // up so a sub-package without engines.node doesn't shadow the
                // project root's engines.node constraint.
                dir = match dir.parent() {
                    Some(parent) => parent,
                    None => break,
                };
                continue;
            }
        };

        // Found a package.json with engines.node at this level — resolve it.
        return resolve_engines_node(&raw, silent, dir);
    }

    Ok(None)
}

/// Resolve an `engines.node` raw value into a concrete version, printing the
/// standard "Found engines.node in package.json:" notice (including the
/// directory where the package.json was located) unless `silent`.
///
/// `raw` may be:
/// - a bare version:        `"22.0.0"` or `"v22.0.0"`
/// - a range expression:    `">=18.0.0"`, `"^20.11.0"`, `"~22.0.0"`,
///   `"22.x"`, `"22 || 20"`, etc.
/// - the wildcard `"*"` / `"x"` / `""`  (no preference)
/// - an alias:              `"lts/*"`, `"lts"`, `"node"`, `"stable"`, `"latest"`
///
/// For ranges we pick the newest locally installed version that satisfies the
/// range. If none is installed we return the range expression itself verbatim,
/// so the caller can show a helpful "not installed, run nvm install <ver>"
/// message (matching the original behavior for bare versions).
fn resolve_engines_node(raw: &str, silent: bool, found_at: &Path) -> Result<Option<String>> {
    // "lts/*", "lts", "node", "stable", "latest" — resolve as aliases against
    // the installed set: lts/* → newest LTS installed, node/stable/latest →
    // newest installed. Falls through to range parsing if not an alias.
    let installed = get_installed_versions();

    // Single point for the informational message so every resolution branch
    // stays in sync. Includes the directory where the package.json was found,
    // mirroring the `Found .nvmrc in: <dir>` notice — useful now that the
    // search is recursive and the matched package.json may be several levels
    // above the current directory.
    let announce = |chosen: &str| {
        if !silent {
            println!(
                "{} {} {} {} {}",
                "ℹ".cyan().bold(),
                T("found_engines_node").cyan(),
                raw.white().bold(),
                format!("→ {}", chosen).dimmed(),
                format!("({})", found_at.display()).dimmed()
            );
        }
    };

    // Resolve alias-like expressions before range parsing so that "lts/*" /
    // "lts" don't get misinterpreted as version strings.
    let lower = raw.to_lowercase();
    if lower == "lts" || lower == "lts/*" || lower == "lts/-1" {
        let mut lts: Vec<String> = installed
            .iter()
            .filter(|v| is_lts_version(v))
            .cloned()
            .collect();
        lts.sort_by(|a, b| crate::utils::compare_semver(a, b));
        if let Some(chosen) = lts.last() {
            announce(chosen);
            return Ok(Some(chosen.clone()));
        }
        // No LTS installed — surface the alias so use_version reports it.
        return Ok(Some(raw.to_string()));
    }
    if lower == "node" || lower == "stable" || lower == "latest" || lower == "*" || lower == "x" {
        if let Some(chosen) = installed
            .iter()
            .max_by(|a, b| crate::utils::compare_semver(a, b))
            .cloned()
        {
            announce(&chosen);
            return Ok(Some(chosen));
        }
        return Ok(None);
    }

    // Try to satisfy as a range expression. This also handles bare versions
    // with wildcards ("22.x"), unions ("22 || 20"), compound (" >=20 <22 "),
    // caret/tilde, and operator-prefixed forms. If it resolves to an installed
    // version we return that; otherwise we fall back to the raw expression so
    // use_version prints the standard "not installed" hint.
    if let Some(chosen) = pick_version_for_range(raw, &installed) {
        announce(&chosen);
        return Ok(Some(chosen));
    }

    // Plain bare version like "22.0.0" or "v22.0.0" — pass through verbatim.
    if raw.starts_with(|c: char| c.is_ascii_digit() || c == 'v') && !raw.contains(' ') {
        return Ok(Some(raw.to_string()));
    }

    // Nothing installed satisfies the range and it isn't a bare version. Surface
    // the original constraint so the user sees what was requested.
    Ok(Some(raw.to_string()))
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
            matching.sort_by(|a, b| crate::utils::compare_semver(a, b));
            // `pop()` is safe because `!matching.is_empty()`, but use `if let`
            // to make the invariant explicit and avoid a panic-prone `unwrap()`
            // that would fire if a future refactor breaks the guard above.
            if let Some(latest) = matching.pop() {
                candidates.push(latest);
            }
        }
    }
    candidates
        .into_iter()
        .max_by(|a, b| crate::utils::compare_semver(a, b))
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
            .max_by(|a, b| crate::utils::compare_semver(a, b))
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
    matching.sort_by(|a, b| crate::utils::compare_semver(a, b));
    matching.pop() // newest
}

fn version_matches_op(version: &str, op: &str, target: &str, wildcard: bool) -> bool {
    // `parse_version_parts` already returns (u32, u32, u32); the previous
    // `parse_v_tuple` wrapper widened to u64, but Node.js version numbers
    // fit in u32 and the comparison semantics are identical.
    let (maj, min, pat) = match crate::utils::parse_version_parts(version) {
        Some(t) => t,
        None => return false,
    };
    let comps: Vec<&str> = target.split('.').collect();
    let t_maj: u32 = comps.first().and_then(|s| s.parse().ok()).unwrap_or(0);
    let t_min: u32 = comps.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    let t_pat: u32 = comps.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);

    match op {
        ">=" => maj > t_maj || (maj == t_maj && (min > t_min || (min == t_min && pat >= t_pat))),
        ">" => maj > t_maj || (maj == t_maj && (min > t_min || (min == t_min && pat > t_pat))),
        "<=" => maj < t_maj || (maj == t_maj && (min < t_min || (min == t_min && pat <= t_pat))),
        "<" => maj < t_maj || (maj == t_maj && (min < t_min || (min == t_min && pat < t_pat))),
        "^" => {
            // Caret: allow changes that do not modify the left-most non-zero
            // element of [major, minor, patch], per the npm semver spec.
            //   ^1.2.3 := >=1.2.3 <2.0.0  (major nonzero → fix major only)
            //   ^0.2.3 := >=0.2.3 <0.3.0  (minor nonzero → fix major.minor)
            //   ^0.0.3 := >=0.0.3 <0.0.4  (patch nonzero → fix all three)
            //   ^0.0.0 := >=0.0.0 <0.0.1  (all zero → exact)
            // Wildcard components widen the upper bound (treated as
            // "unspecified" rather than zero):
            //   ^0.x   := >=0.0.0 <1.0.0
            //   ^0.0.x := >=0.0.0 <0.1.0
            //   ^1.x   := >=1.0.0 <2.0.0
            //
            // The previous implementation only checked `maj == t_maj &&
            // (min, pat) >= (t_min, t_pat)`, which treats `^0.2.3` as
            // `>=0.2.3 <1.0.0` — incorrectly matching 0.3.0, 0.10.0, etc.
            if maj != t_maj {
                return false;
            }
            // Lower bound: version >= target.
            if (min, pat) < (t_min, t_pat) {
                return false;
            }
            // Determine how many components were explicitly specified,
            // treating a wildcard as "end of specified components" (a
            // wildcard in position i means positions i.. are unspecified).
            //   "0.2.3"  → n=3
            //   "0.2.x"  → n=2 (patch is wildcard → unspecified)
            //   "0.x"    → n=1
            //   "1"      → n=1
            let n_specified = comps
                .iter()
                .position(|c| *c == "x" || *c == "X" || *c == "*")
                .unwrap_or(comps.len().min(3));
            if n_specified == 0 {
                // Entirely wildcard (e.g. `^x`); already handled by the
                // early-return in pick_version_for_range_single, but guard
                // here too — match anything in the same major.
                return true;
            }
            // Find the left-most non-zero position among the specified
            // components. If all specified are zero, increment the LAST
            // specified component (this is what makes `^0.0.0` → `<0.0.1`
            // and `^0` → `<1.0.0`).
            let inc_pos: usize = if t_maj > 0 {
                0
            } else if n_specified >= 2 && t_min > 0 {
                1
            } else if n_specified >= 3 && t_pat > 0 {
                2
            } else {
                // All specified components are zero (or fewer specified).
                // Increment the last specified component.
                n_specified - 1
            };
            // Upper bound = (inc_pos component + 1, everything after = 0).
            let (u_maj, u_min, u_pat) = match inc_pos {
                0 => (t_maj + 1, 0u32, 0u32),
                1 => (0u32, t_min + 1, 0u32),
                _ => (0u32, 0u32, t_pat + 1),
            };
            // version < upper bound (strictly).
            (maj, min, pat) < (u_maj, u_min, u_pat)
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
                        let m: u32 = m.parse().unwrap_or(0);
                        if min != m {
                            return false;
                        }
                    }
                }
                if comps.len() > 2 {
                    let p = comps[2];
                    if !(p == "x" || p == "X" || p == "*") {
                        let p: u32 = p.parse().unwrap_or(0);
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
                format_t("lang_usage", std::slice::from_ref(&usage_hint)).dimmed()
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

            // System proxy env. Redact embedded credentials before printing:
            // `HTTPS_PROXY=http://user:pass@proxy:8080` is a common pattern,
            // and printing the raw URL to stdout would leak the password into
            // terminal scrollback, CI logs, and screen recordings.
            let sys_state = match &sys_proxy {
                Some(p) => format!("{}", redact_proxy_credentials(p).dimmed()),
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

    #[test]
    fn test_redact_proxy_credentials_masks_userinfo() {
        // user:pass@ must be replaced with ***@
        assert_eq!(
            redact_proxy_credentials("http://user:pass@proxy.corp:8080"),
            "http://***@proxy.corp:8080"
        );
        assert_eq!(
            redact_proxy_credentials("https://bob:s3cr3t@10.0.0.1:3128"),
            "https://***@10.0.0.1:3128"
        );
        // User-only (no password) is still masked.
        assert_eq!(
            redact_proxy_credentials("http://user@host:80"),
            "http://***@host:80"
        );
    }

    #[test]
    fn test_redact_proxy_credentials_preserves_no_creds() {
        // No userinfo → returned unchanged.
        assert_eq!(
            redact_proxy_credentials("http://127.0.0.1:8080"),
            "http://127.0.0.1:8080"
        );
        assert_eq!(
            redact_proxy_credentials("socks5://localhost:1080"),
            "socks5://localhost:1080"
        );
        // Not a URL at all.
        assert_eq!(redact_proxy_credentials("not set"), "not set");
    }

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

    // --- caret (^) with 0.x.y — the P1-12 regression tests ----------------
    //
    // Per the npm semver spec, the caret locks the left-most NON-ZERO
    // component of [major, minor, patch]:
    //   ^1.2.3  := >=1.2.3 <2.0.0   (major nonzero → fix major)
    //   ^0.2.3  := >=0.2.3 <0.3.0   (minor nonzero → fix major.minor)
    //   ^0.0.3  := >=0.0.3 <0.0.4   (patch nonzero → fix all three)
    //   ^0.0.0  := >=0.0.0 <0.0.1   (all zero → exact)
    // The previous implementation only checked `maj == t_maj && version >=
    // target`, treating ^0.2.3 as >=0.2.3 <1.0.0 — incorrectly matching
    // 0.3.0, 0.10.0, etc. These tests pin the correct behaviour.

    fn installed_with_zero_major() -> Vec<String> {
        vec![
            "v0.8.0".to_string(),
            "v0.10.0".to_string(),
            "v0.10.5".to_string(),
            "v0.11.0".to_string(),
            "v0.12.0".to_string(),
            "v0.0.3".to_string(),
            "v0.0.4".to_string(),
            "v0.1.0".to_string(),
            "v20.11.0".to_string(),
        ]
    }

    #[test]
    fn caret_zero_minor_locks_major_minor() {
        // ^0.10.0 := >=0.10.0 <0.11.0. Must match 0.10.5 (newest in 0.10.x)
        // and reject 0.11.0 / 0.12.0 (different minor in major 0).
        let r = pick_version_for_range("^0.10.0", &installed_with_zero_major());
        assert_eq!(r.as_deref(), Some("v0.10.5"));
    }

    #[test]
    fn caret_zero_minor_rejects_higher_minor() {
        // ^0.10.5 must NOT match 0.11.0 or 0.12.0 — the old code did.
        let r = pick_version_for_range("^0.10.5", &installed_with_zero_major());
        assert_eq!(r.as_deref(), Some("v0.10.5"));
        // And 0.11.0 is explicitly out of range:
        let only_0_11 = vec!["v0.11.0".to_string()];
        assert_eq!(pick_version_for_range("^0.10.5", &only_0_11), None);
    }

    #[test]
    fn caret_zero_zero_patch_is_exact() {
        // ^0.0.3 := >=0.0.3 <0.0.4 — only 0.0.3 matches (not 0.0.4).
        let r = pick_version_for_range("^0.0.3", &installed_with_zero_major());
        assert_eq!(r.as_deref(), Some("v0.0.3"));
        // A pool with only 0.0.4 must not satisfy ^0.0.3:
        let only_0_0_4 = vec!["v0.0.4".to_string()];
        assert_eq!(pick_version_for_range("^0.0.3", &only_0_0_4), None);
    }

    #[test]
    fn caret_zero_zero_zero_is_exact() {
        // ^0.0.0 := >=0.0.0 <0.0.1 — only 0.0.0 matches.
        let pool = vec!["v0.0.0".to_string(), "v0.0.1".to_string()];
        assert_eq!(
            pick_version_for_range("^0.0.0", &pool).as_deref(),
            Some("v0.0.0")
        );
    }

    #[test]
    fn caret_zero_wildcard_minor_matches_all_zero_x() {
        // ^0.x := >=0.0.0 <1.0.0 — any 0.x version matches. Picks newest.
        let r = pick_version_for_range("^0.x", &installed_with_zero_major());
        assert_eq!(r.as_deref(), Some("v0.12.0"));
    }

    #[test]
    fn caret_zero_zero_wildcard_patch_matches_only_zero_zero_x() {
        // ^0.0.x := >=0.0.0 <0.1.0 — matches 0.0.3 and 0.0.4, not 0.1.0.
        let r = pick_version_for_range("^0.0.x", &installed_with_zero_major());
        assert_eq!(r.as_deref(), Some("v0.0.4"));
    }

    #[test]
    fn caret_zero_bare_major_matches_all_zero_x() {
        // ^0 := >=0.0.0 <1.0.0 (same as ^0.x). Picks newest 0.x.
        let r = pick_version_for_range("^0", &installed_with_zero_major());
        assert_eq!(r.as_deref(), Some("v0.12.0"));
    }

    #[test]
    fn caret_nonzero_major_still_works() {
        // Regression guard: the fix must not break the existing ^1.x.y path.
        // ^20.10.0 := >=20.10.0 <21.0.0 — picks newest 20.x.
        let r = pick_version_for_range("^20.10.0", &installed_with_zero_major());
        assert_eq!(r.as_deref(), Some("v20.11.0"));
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

    // --- parse_version_parts (used by version_matches_op) ------------------
    #[test]
    fn parse_v_tuple_v_prefixed() {
        assert_eq!(
            crate::utils::parse_version_parts("v20.11.1"),
            Some((20, 11, 1))
        );
    }

    #[test]
    fn parse_v_tuple_bare() {
        assert_eq!(
            crate::utils::parse_version_parts("18.20.0"),
            Some((18, 20, 0))
        );
    }

    #[test]
    fn parse_v_tuple_iojs_prefix() {
        assert_eq!(
            crate::utils::parse_version_parts("iojs-v3.3.1"),
            Some((3, 3, 1))
        );
    }

    #[test]
    fn parse_v_tuple_iojs_dot_prefix() {
        // Previously a bug: parse missed "io.js-v" / "io.js-" prefixes,
        // making io.js versions invisible to the engines.node range matcher.
        assert_eq!(
            crate::utils::parse_version_parts("io.js-v3.3.1"),
            Some((3, 3, 1))
        );
        assert_eq!(
            crate::utils::parse_version_parts("io.js-3.3.1"),
            Some((3, 3, 1))
        );
    }

    #[test]
    fn parse_v_tuple_trailing_suffix() {
        // "v20.11.1-rc.1" → (20, 11, 1)
        assert_eq!(
            crate::utils::parse_version_parts("v20.11.1-rc.1"),
            Some((20, 11, 1))
        );
    }

    #[test]
    fn parse_v_tuple_missing_patch_defaults_zero() {
        assert_eq!(crate::utils::parse_version_parts("v22"), Some((22, 0, 0)));
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

    // --- find_package_json_node_version ------------------------------------
    //
    // Walks up from CWD looking for a package.json with an `engines.node`
    // field. The function reads `std::env::current_dir()`, so the tests
    // chdir into a tempdir and MUST be serialised — parallel chdir would
    // race against each other. A module-local Mutex serialises only these
    // tests; everything else still runs in parallel.

    /// Hold the CWD lock for the duration of a chdir-based test and restore
    /// the original working directory on Drop. The Mutex guard inside is
    /// what serialises the tests; storing it in the struct keeps it alive
    /// until `CwdGuard` itself is dropped.
    struct CwdGuard {
        original: std::path::PathBuf,
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl CwdGuard {
        fn enter(dir: &std::path::Path) -> Self {
            let lock = CWD_MUTEX.lock().expect("CWD_MUTEX poisoned");
            let original = std::env::current_dir().expect("current_dir");
            std::env::set_current_dir(dir).expect("set_current_dir");
            CwdGuard {
                original,
                _lock: lock,
            }
        }
    }

    impl Drop for CwdGuard {
        fn drop(&mut self) {
            let _ = std::env::set_current_dir(&self.original);
        }
    }

    static CWD_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn find_package_json_returns_none_when_no_package_json() {
        let dir = tempfile::tempdir().expect("tempdir");
        let _cwd = CwdGuard::enter(dir.path());
        let result = find_package_json_node_version(true).expect("no error");
        assert_eq!(result, None, "no package.json anywhere → None");
    }

    #[test]
    fn find_package_json_returns_none_when_engines_node_absent() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"name": "x", "version": "1.0.0"}"#,
        )
        .expect("write");
        let _cwd = CwdGuard::enter(dir.path());
        let result = find_package_json_node_version(true).expect("no error");
        assert_eq!(result, None, "no engines.node → None");
    }

    #[test]
    fn find_package_json_finds_engines_node_in_cwd() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"engines": {"node": ">=20.0.0"}}"#,
        )
        .expect("write");
        let _cwd = CwdGuard::enter(dir.path());
        let result = find_package_json_node_version(true).expect("no error");
        // The raw range is surfaced verbatim when no installed version matches;
        // we only assert that *something* was found (the range resolution
        // itself is exercised by the pick_version_for_range tests above).
        assert!(result.is_some(), "engines.node present → Some");
    }

    #[test]
    fn find_package_json_walks_up_to_parent() {
        // Subdir has no package.json; parent does with engines.node.
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"engines": {"node": "22.x"}}"#,
        )
        .expect("write");
        let nested = dir.path().join("packages").join("a");
        std::fs::create_dir_all(&nested).expect("mkdir");
        let _cwd = CwdGuard::enter(&nested);
        let result = find_package_json_node_version(true).expect("no error");
        assert!(
            result.is_some(),
            "should find parent package.json by walking up"
        );
    }

    #[test]
    fn find_package_json_skips_malformed_json_and_continues_up() {
        // A broken package.json at this level must not crash detection —
        // the search should continue to the parent.
        let dir = tempfile::tempdir().expect("tempdir");
        let nested = dir.path().join("sub");
        std::fs::create_dir_all(&nested).expect("mkdir");
        std::fs::write(nested.join("package.json"), "not valid json {{{").expect("write");
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"engines": {"node": "20.0.0"}}"#,
        )
        .expect("write");
        let _cwd = CwdGuard::enter(&nested);
        let result = find_package_json_node_version(true).expect("no error");
        assert!(
            result.is_some(),
            "malformed child json should fall through to parent"
        );
    }
}
