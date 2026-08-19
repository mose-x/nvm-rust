//! Project-level Node.js version discovery.
//!
//! Walks up from the current directory looking for `.nvmrc`, `.node-version`,
//! or `package.json` with an `engines.node` field, then resolves the discovered
//! version/range against the locally installed set.

use anyhow::Result;
use colored::Colorize;
use std::fs;
use std::path::Path;

use crate::i18n::T;
use crate::utils::{compare_semver, get_installed_versions, is_lts_version};

/// Recursively search for .nvmrc or .node-version file from current directory
/// up to root.
pub fn find_nvmrc_recursive(silent: bool) -> Result<Option<String>> {
    let current_dir = std::env::current_dir()?;
    let mut dir = current_dir.as_path();

    // Read the first non-comment, non-empty line from a .nvmrc /
    // .node-version file. nvm-sh itself only reads the first line, but many
    // real-world .nvmrc files start with a `# comment` (editor templates,
    // per-project docs) -- without this filter the comment text would be
    // passed to resolve_alias and produce a confusing error like
    // "Version v# comment\nv18.20.4 is not installed".
    let read_first_version_line = |path: &Path| -> Option<String> {
        // Distinguish "file not present" (expected, return None) from real
        // read errors (permission denied, I/O error). The previous `.ok()?`
        // lumped them together, so a `.nvmrc` that exists but is unreadable
        // was silently treated as absent -- the user got "no .nvmrc found"
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
        // { break; }` guard *after* this reassignment -- that broke one step
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
/// the search -- this is what makes the common monorepo layout (root
/// `package.json` declares `engines.node` for the whole repo, sub-packages
/// don't) work without requiring every sub-package to repeat the constraint.
///
/// The first `package.json` (closest to cwd) with a non-empty `engines.node`
/// wins; lower levels are not consulted.
pub fn find_package_json_node_version(silent: bool) -> Result<Option<String>> {
    let current_dir = std::env::current_dir()?;
    let mut dir = current_dir.as_path();

    loop {
        let package_json = dir.join("package.json");
        // Read the file directly without a pre-check `.exists()` -- that would
        // be a redundant stat + a TOCTOU window. Distinguish NotFound (no
        // package.json at this level -- keep walking up) from real read errors
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
        // detection or shadow a valid one higher up -- skip and try the parent.
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
                // package.json exists but has no engines.node -- keep walking
                // up so a sub-package without engines.node doesn't shadow the
                // project root's engines.node constraint.
                dir = match dir.parent() {
                    Some(parent) => parent,
                    None => break,
                };
                continue;
            }
        };

        // Found a package.json with engines.node at this level -- resolve it.
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
pub fn resolve_engines_node(raw: &str, silent: bool, found_at: &Path) -> Result<Option<String>> {
    // "lts/*", "lts", "node", "stable", "latest" -- resolve as aliases against
    // the installed set: lts/* -> newest LTS installed, node/stable/latest ->
    // newest installed. Falls through to range parsing if not an alias.
    let installed = get_installed_versions();

    // Single point for the informational message so every resolution branch
    // stays in sync. Includes the directory where the package.json was found,
    // mirroring the `Found .nvmrc in: <dir>` notice -- useful now that the
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
        lts.sort_by(|a, b| compare_semver(a, b));
        if let Some(chosen) = lts.last() {
            announce(chosen);
            return Ok(Some(chosen.clone()));
        }
        // No LTS installed -- surface the alias so use_version reports it.
        return Ok(Some(raw.to_string()));
    }
    if lower == "node" || lower == "stable" || lower == "latest" || lower == "*" || lower == "x" {
        if let Some(chosen) = installed
            .iter()
            .max_by(|a, b| compare_semver(a, b))
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
    if let Some(chosen) = crate::semver_range::pick_version_for_range(raw, &installed) {
        announce(&chosen);
        return Ok(Some(chosen));
    }

    // Plain bare version like "22.0.0" or "v22.0.0" -- pass through verbatim.
    if raw.starts_with(|c: char| c.is_ascii_digit() || c == 'v') && !raw.contains(' ') {
        return Ok(Some(raw.to_string()));
    }

    // Nothing installed satisfies the range and it isn't a bare version. Surface
    // the original constraint so the user sees what was requested.
    Ok(Some(raw.to_string()))
}

// ---------------------------------------------------------------------------
// Unit tests for find_package_json_node_version.
//
// Walks up from CWD looking for a package.json with an `engines.node`
// field. The function reads `std::env::current_dir()`, so the tests
// chdir into a tempdir and MUST be serialised -- parallel chdir would
// race against each other. A module-local Mutex serialises only these
// tests; everything else still runs in parallel.
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

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
            let lock = CWD_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
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
        assert_eq!(result, None, "no package.json anywhere -> None");
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
        assert_eq!(result, None, "no engines.node -> None");
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
        assert!(result.is_some(), "engines.node present -> Some");
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
        // A broken package.json at this level must not crash detection --
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
