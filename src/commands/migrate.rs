use anyhow::{Context, Result};
use colored::Colorize;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use crate::config::{load_config, save_config};
use crate::i18n::{format_t, T};
use crate::system::{ensure_nvm_dir, get_nvm_dir};

/// Locate the source versions directory for a given migration source.
fn resolve_migration_source(source: &str) -> Option<PathBuf> {
    let home = crate::system::get_home_dir();
    match source.to_lowercase().as_str() {
        "nvm" | "nvm-sh" | "nvm_sh" => {
            // nvm-sh stores versions under ~/.nvm/versions/node/. We deliberately do NOT
            // honor NVM_DIR here because that variable is also what nvm-rust itself uses
            // for its own install dir — honoring it would make source and destination
            // point at the same place and silently skip everything.
            // The legacy NVM_SH_HOME override lets tests (and advanced users) point at
            // a non-default nvm-sh install location.
            let nvm_sh_root = env::var("NVM_SH_HOME").unwrap_or_else(|_| home.clone());
            let versions_dir = PathBuf::from(&nvm_sh_root)
                .join(".nvm")
                .join("versions")
                .join("node");
            if versions_dir.is_dir() {
                Some(versions_dir)
            } else {
                None
            }
        }
        "nvm-windows" | "nvm_windows" | "nvmwindows" => {
            // nvm-windows stores versions under $NVM_HOME or $NVM_SYMLINK root.
            // These env vars are nvm-windows specific and do not conflict with nvm-rust.
            let root = env::var("NVM_HOME")
                .or_else(|_| env::var("NVM_SYMLINK"))
                .unwrap_or_else(|_| {
                    // Fallback when neither env var is set. Use `PathBuf::join`
                    // so the path separator is correct for the *current* host:
                    // the previous `format!("{}\\nvm4w", home)` baked a literal
                    // backslash into the path, which on a non-Windows host (e.g.
                    // running `nvm migrate nvm-windows` from WSL or a Linux box
                    // that mounted a Windows drive) produced a malformed path
                    // like `/home/user\nvm4w` — a single component containing a
                    // backslash rather than `home` + `nvm4w`.
                    PathBuf::from(&home)
                        .join("nvm4w")
                        .to_string_lossy()
                        .into_owned()
                });
            let p = PathBuf::from(&root);
            if p.is_dir() {
                Some(p)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Copy a source version directory into the nvm-rust store.
/// Returns true if the version was newly imported, false if already present.
///
/// Always performs a deep copy (never a symlink). A symlinked import would
/// dangle as soon as the user removes the source tree — e.g. after running
/// `rm -rf ~/.nvm` to clean up nvm-sh, every imported version would turn
/// into a broken symlink and `nvm use <version>` would fail with
/// "No such file or directory" instead of the expected "version not
/// installed". Copying makes the import self-contained, matching nvm-sh's
/// own `nvm install` semantics where the version lives entirely under
/// `NVM_DIR`.
fn import_version(src: &Path, dest: &Path) -> Result<bool> {
    if dest.exists() {
        return Ok(false);
    }

    copy_dir_recursive(src, dest).context(T("copy_version_dir_failed"))?;
    Ok(true)
}

/// Recursively copy a directory tree, preserving symlinks.
///
/// nvm-sh's version directories contain symlinks: `bin/npm` and `bin/npx`
/// point at `../lib/node_modules/npm/bin/*-cli.js`. The previous
/// implementation used `path.is_dir()` (which follows symlinks) and
/// `fs::copy` (which also follows symlinks and writes the target's bytes
/// as a regular file). That had two consequences: the link structure was
/// lost, and the copied file inherited the target's permissions (typically
/// 0644 for the .js source, not executable), so `nvm use <migrated>; npm
/// install` failed because `bin/npm` was no longer executable.
///
/// We now use `symlink_metadata` to detect symlinks without following them
/// and recreate the link at the destination. If link recreation fails
/// (e.g. Windows without SeCreateSymbolicLinkPrivilege, or a cross-device
/// absolute target that would dangle), we fall back to copying the target
/// contents and restoring the executable bit so the result at least runs.
fn copy_dir_recursive(src: &Path, dest: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dest)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        let target = dest.join(&name);

        let meta = fs::symlink_metadata(&path)?;
        let ft = meta.file_type();

        if ft.is_symlink() {
            let link_target = fs::read_link(&path)?;
            let link_result = create_symlink(&link_target, &target);
            if link_result.is_err() {
                // Cannot create the symlink (Windows without privilege,
                // or filesystem that doesn't support links). Fall back to
                // copying the resolved contents. For the npm/npx case the
                // target is a .js file; copy it and mark executable so
                // `bin/npm` still runs via the shebang.
                copy_resolved(&path, &target)?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let mut perms = fs::metadata(&target)?.permissions();
                    perms.set_mode(0o755);
                    fs::set_permissions(&target, perms)?;
                }
            }
        } else if ft.is_dir() {
            copy_dir_recursive(&path, &target)?;
        } else {
            fs::copy(&path, &target)?;
        }
    }
    Ok(())
}

/// Copy the resolved target of a symlink to `dest` (follows one level).
fn copy_resolved(link_path: &Path, dest: &Path) -> std::io::Result<()> {
    match fs::metadata(link_path) {
        Ok(m) if m.is_dir() => copy_dir_recursive(
            &fs::canonicalize(link_path).unwrap_or_else(|_| link_path.to_path_buf()),
            dest,
        ),
        _ => fs::copy(link_path, dest).map(|_| ()),
    }
}

/// Create a symlink at `link` pointing to `target`, using the correct
/// platform-specific std API. Returns Err if symlinks are not supported
/// (Windows without SeCreateSymbolicLinkPrivilege, FAT, etc.) so the
/// caller can fall back to copying.
fn create_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link)
    }
    #[cfg(windows)]
    {
        // Windows distinguishes file vs dir symlinks. Decide by inspecting
        // the target (follow the link we just read_link'd).
        if fs::metadata(target).map(|m| m.is_dir()).unwrap_or(false) {
            std::os::windows::fs::symlink_dir(target, link)
        } else {
            std::os::windows::fs::symlink_file(target, link)
        }
    }
}

/// Migrate installed Node.js versions from nvm-sh or nvm-windows.
///
/// Versions are deep-copied into the nvm-rust store (see `import_version`) so
/// the import is self-contained and survives deletion of the source tree.
/// Already-present versions are skipped. The `default` alias from nvm-sh is
/// also carried over when present.
pub fn cmd_migrate(source: &str) -> Result<()> {
    let src_dir = resolve_migration_source(source).ok_or_else(|| {
        anyhow::anyhow!(
            "{}",
            format_t("migrate_source_not_found", &[source.to_string()])
        )
    })?;

    // Compute the nvm-sh install root (the dir that *contains* `.nvm/`),
    // mirroring the logic in `resolve_migration_source` for "nvm"/"nvm-sh".
    // `detect_nvm_sh_default` needs the same root it used to find versions,
    // otherwise it would read the alias from `NVM_DIR` (which is nvm-rust's
    // own store, not the nvm-sh install we just migrated from).
    let nvm_sh_root: Option<PathBuf> = match source.to_lowercase().as_str() {
        "nvm" | "nvm-sh" | "nvm_sh" => {
            let home = crate::system::get_home_dir();
            Some(PathBuf::from(env::var("NVM_SH_HOME").unwrap_or(home)))
        }
        _ => None, // nvm-windows: no `~/.nvm/alias/default` concept.
    };

    let nvm_dir = get_nvm_dir();
    ensure_nvm_dir().context(T("cannot_create_nvm_dir"))?;

    // Serialize against concurrent install/uninstall/use. `import_version`
    // does a `dest.exists()` check then `copy_dir_recursive` — without the
    // lock, two concurrent `nvm migrate` (or a migrate racing an install of
    // the same version) would both pass the exists check and then clobber
    // each other's copy, producing a corrupted version directory. Same race
    // class as the one `install`/`uninstall` already guard against.
    let _nvm_lock = crate::utils::acquire_nvm_lock(&nvm_dir)?;

    println!();
    println!(
        "  {} {}",
        "▶".cyan().bold(),
        format_t("migrate_scanning", &[src_dir.display().to_string()]).cyan()
    );
    println!();

    let mut imported = 0usize;
    let mut skipped = 0usize;

    // Enumerate version directories. nvm-sh uses "vX.Y.Z", nvm-windows too,
    // and io.js installs use "iojs-vX.Y.Z" (or the "io.js-" spelling some
    // nvm forks write). Reuse `is_iojs_version` so every io.js prefix variant
    // is recognised — the previous `starts_with("iojs-")` check missed the
    // `io.js-*` spellings and silently skipped those version directories.
    let mut entries: Vec<PathBuf> = Vec::new();
    // Surface read_dir errors (permission denied, I/O) instead of silently
    // treating them as "no versions". The previous `if let Ok(rd)` arm
    // returned an empty `entries` vec, which printed "no versions found"
    // even when the real problem was e.g. an unreadable source dir.
    let rd = fs::read_dir(&src_dir)
        .with_context(|| format_t("migrate_scan_failed", &[src_dir.display().to_string()]))?;
    for entry in rd.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name.starts_with('v') || crate::utils::is_iojs_version(name) {
                    entries.push(path);
                }
            }
        }
    }
    // Sort semantically (major.minor.patch) rather than lexicographically.
    // `PathBuf::sort()` orders by OS string, so `v9.0.0` would sort *after*
    // `v20.11.0` ('9' > '2'), printing versions in the wrong order during
    // migration. Compare by the file-name component using `compare_semver`.
    entries.sort_by(|a, b| {
        let an = a.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let bn = b.file_name().and_then(|n| n.to_str()).unwrap_or("");
        crate::utils::compare_semver(an, bn)
    });

    if entries.is_empty() {
        println!(
            "  {} {}",
            "⚠".yellow().bold(),
            T("migrate_no_versions").yellow()
        );
        println!();
        return Ok(());
    }

    println!(
        "  {} {}",
        "ℹ".cyan().bold(),
        format_t("migrate_found", &[entries.len().to_string()])
    );
    println!();

    for src_version_dir in &entries {
        let version_name = src_version_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");
        if version_name.is_empty() {
            continue;
        }
        let dest_version_dir = nvm_dir.join(version_name);

        match import_version(src_version_dir, &dest_version_dir) {
            Ok(true) => {
                println!(
                    "  {} {}",
                    "✓".green().bold(),
                    format_t("migrate_imported", &[version_name.to_string()]).green()
                );
                imported += 1;
            }
            Ok(false) => {
                println!(
                    "  {} {}",
                    "·".dimmed(),
                    format_t("migrate_skipped", &[version_name.to_string()]).dimmed()
                );
                skipped += 1;
            }
            Err(e) => {
                println!(
                    "  {} {} ({})",
                    "✗".red().bold(),
                    format_t("migrate_failed", &[version_name.to_string()]).red(),
                    e
                );
            }
        }
    }

    // Carry over the `default` alias from nvm-sh if we touched anything at
    // all (imported OR skipped — already-imported versions still mean the
    // nvm-sh tree was found and is the source of truth for "default").
    if imported + skipped > 0 {
        if let Some(root) = nvm_sh_root {
            if let Some(default_ver) = detect_nvm_sh_default(&root) {
                // Defense-in-depth: `detect_nvm_sh_default` reads
                // `~/.nvm/alias/default` from the source nvm-sh install.
                // A hand-edited or malicious default file could contain a
                // path-traversal payload like `v../../etc/passwd`. We are
                // about to `nvm_dir.join(&default_ver)` and write it into
                // `config.default_version`, so validate FIRST. Only concrete
                // version forms (`v20.11.0`, `iojs-v3.3.1`) can pass the
                // subsequent `dest.exists()` check anyway — alias forms
                // like `lts/*` would never match an installed version dir —
                // so rejecting traversal here cannot break the happy path.
                if crate::utils::validate_version_name(&default_ver).is_err() {
                    eprintln!(
                        "{} {}: {}",
                        "⚠".yellow().bold(),
                        crate::i18n::T("invalid_version_name"),
                        default_ver
                    );
                } else {
                    let dest = nvm_dir.join(&default_ver);
                    if dest.exists() {
                        let mut config = load_config()?;
                        config.default_version = Some(default_ver.clone());
                        save_config(&config)?;
                        // Also overwrite `aliases.aliases["default"]` if it exists:
                        // `resolve_alias("default")` checks user-defined aliases
                        // FIRST and only falls back to `config.default_version`,
                        // so writing config alone is not enough — a pre-existing
                        // `default` alias (e.g. from an earlier `nvm alias default
                        // lts`) would shadow the migrated value and `nvm use
                        // default` would resolve to the old alias instead.
                        if let Ok(mut aliases) = crate::config::load_aliases() {
                            if aliases.aliases.contains_key("default") {
                                aliases
                                    .aliases
                                    .insert("default".to_string(), default_ver.clone());
                                crate::config::save_aliases(&aliases)?;
                            }
                        }
                        println!();
                        println!(
                            "  {} {}",
                            "✓".green().bold(),
                            format_t("migrate_default_set", &[default_ver]).green()
                        );
                    }
                }
            }
        }
    }

    println!();
    println!(
        "  {} {}",
        "✓".green().bold(),
        format_t(
            "migrate_summary",
            &[imported.to_string(), skipped.to_string()]
        )
        .green()
    );
    println!();
    Ok(())
}

/// Read the nvm-sh default alias from <nvm_sh_root>/.nvm/alias/default.
/// `nvm_sh_root` is the directory that contains the `.nvm` subdir of the
/// nvm-sh install being migrated from (i.e. the same root
/// `resolve_migration_source` derives its `versions/node` path from). We MUST
/// NOT consult `NVM_DIR` here: that variable is what nvm-rust itself uses for
/// its own store, and could point at a different location than the nvm-sh
/// install we just migrated from — reading the alias from there would either
/// find a stale/empty file or the wrong default.
fn detect_nvm_sh_default(nvm_sh_root: &Path) -> Option<String> {
    let default_file = nvm_sh_root.join(".nvm").join("alias").join("default");
    // Distinguish "file not present" (expected, return None) from real read
    // errors (permission denied, IO error). The previous `.ok()?` lumped
    // them together, so an unreadable default file was silently treated as
    // "no default" instead of surfacing the permission problem.
    let raw = match fs::read_to_string(&default_file) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => {
            eprintln!("{} {}: {}", "⚠".yellow().bold(), default_file.display(), e);
            return None;
        }
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    // Already a fully-qualified version: keep as-is. Use the shared io.js
    // detector so ALL four spellings (iojs-v / iojs- / io.js-v / io.js-) are
    // accepted, matching the rest of the codebase. The previous
    // `starts_with("iojs-")` only recognized one spelling, so a default file
    // containing `io.js-v3.3.1` fell through to bare-major parsing where
    // `"io".parse::<u32>()` fails and the alias resolved to "latest of
    // everything" — silently wrong.
    if trimmed.starts_with('v') || crate::utils::is_iojs_version(trimmed) {
        return Some(trimmed.to_string());
    }
    // Full version without "v" prefix (e.g. "20.11.0", "22.5.1"): add prefix.
    // We detect "full" as exactly two dots among digits.
    let dots = trimmed.matches('.').count();
    if dots == 2 && trimmed.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return Some(format!("v{}", trimmed));
    }
    // Bare major ("20"), bare major.minor ("20.5"), "node", "stable",
    // "lts/*", "lts/iron", etc. — resolve against the SOURCE nvm-sh install
    // so "20" maps to the latest v20.x.y that nvm-sh actually has installed.
    let versions_root = nvm_sh_root.join(".nvm").join("versions").join("node");
    let mut candidates: Vec<String> = Vec::new();
    // Surface read_dir errors instead of silently treating them as "empty".
    // The previous `if let Ok(rd)` swallowed permission/IO errors, so a
    // versions dir that exists but is unreadable resolved every alias to
    // None — the user saw "no default to migrate" instead of the real error.
    match fs::read_dir(&versions_root) {
        Ok(rd) => {
            for entry in rd.flatten() {
                if let Some(name) = entry.file_name().to_str() {
                    // Collect both Node.js (`vX.Y.Z`) and io.js (`iojs-*` /
                    // `io.js-*`) installs. The previous `starts_with('v')` filter
                    // silently dropped io.js versions, so a `node`/`stable`
                    // alias on a host with only io.js installed resolved to
                    // `None` and the default was silently not migrated.
                    if name.starts_with('v') || crate::utils::is_iojs_version(name) {
                        candidates.push(name.to_string());
                    }
                }
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // No versions dir on disk: leave candidates empty and fall through
            // to `None`. This is expected when nvm-sh is installed but has no
            // versions yet, so we do not warn.
        }
        Err(e) => {
            // Surface permission/IO errors instead of silently treating them
            // as "no versions". The previous `if let Ok(rd)` swallowed these,
            // so an unreadable versions dir resolved every alias to None and
            // the user saw "no default to migrate" instead of the real error.
            eprintln!("{} {}: {}", "⚠".yellow().bold(), versions_root.display(), e);
            return None;
        }
    }
    // `lts/*` (and `lts/<codename>`) must restrict to LTS versions only.
    // Without this, the bare-major branch below doesn't match (`"lts/*"`
    // doesn't parse as u32) and the function falls through to "latest of
    // everything", picking a non-LTS Current release as the default —
    // silently wrong.
    if trimmed == "lts/*" || trimmed.starts_with("lts/") {
        candidates.retain(|v| crate::utils::is_lts_version(v));

        // `lts/-N` → Nth-previous LTS *line* relative to the newest LTS line
        // present in the source install (e.g. lts/-1 = the LTS line before
        // the newest). Without this, lts/-1 fell through to "latest of all
        // LTS versions" (i.e. the newest LTS line), silently wrong —
        // nvm-sh's lts/-1 means the LTS line BEFORE the newest. This
        // mirrors `config::resolve_lts_relative` but operates on the
        // SOURCE nvm-sh install's versions dir instead of nvm-rust's.
        if let Some(offset_str) = trimmed.strip_prefix("lts/-") {
            if let Ok(offset) = offset_str.parse::<usize>() {
                if offset > 0 {
                    // Unique LTS majors present in the source install, ascending.
                    let mut majors: Vec<u32> = candidates
                        .iter()
                        .filter_map(|v| crate::utils::parse_major(v))
                        .collect();
                    majors.sort_unstable();
                    majors.dedup();
                    // offset=1 → second-newest (index len-2), offset=2 → len-3.
                    match majors
                        .len()
                        .checked_sub(1 + offset)
                        .and_then(|i| majors.get(i))
                    {
                        Some(&major) => {
                            let prefix = format!("v{}.", major);
                            candidates.retain(|v| v.starts_with(&prefix));
                        }
                        None => return None, // offset out of range: no such LTS line
                    }
                }
                // offset == 0 (lts/-0) is nonsensical; fall through to lts/*
                // behavior (return latest LTS).
            }
            // Non-numeric suffix (e.g. "lts/-foo") falls through to lts/*
            // behavior too — matching the lenient handling elsewhere.
        }
    }
    // For a bare major like "20", restrict to matching "v20.*". For generic
    // aliases ("node", "stable") we take the latest of everything.
    if let Some(major) = trimmed
        .split('.')
        .next()
        .and_then(|s| s.parse::<u32>().ok())
    {
        let prefix = format!("v{}.", major);
        candidates.retain(|v| v.starts_with(&prefix));
    }
    // Sort semantically — alphabetical sort would put `v20.5.0` after
    // `v20.20.2` ('5' > '2') and pick the older version as "latest".
    candidates.sort_by(|a, b| crate::utils::compare_semver(a, b));
    candidates.last().cloned()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a fake nvm-sh root with `<root>/.nvm/alias/default` containing
    /// `contents`, and (optionally) a `versions/node/` dir populated with the
    /// given version dir names. Returns the root path.
    fn fake_nvm_sh_root(default_contents: &str, versions: &[&str]) -> tempfile::TempDir {
        let tmp = tempfile::tempdir().expect("tempdir");
        let alias_dir = tmp.path().join(".nvm").join("alias");
        fs::create_dir_all(&alias_dir).expect("create alias dir");
        fs::write(alias_dir.join("default"), default_contents).expect("write default");
        if !versions.is_empty() {
            let versions_root = tmp.path().join(".nvm").join("versions").join("node");
            fs::create_dir_all(&versions_root).expect("create versions root");
            for v in versions {
                fs::create_dir_all(versions_root.join(v)).expect("create version dir");
            }
        }
        tmp
    }

    #[test]
    fn detect_nvm_sh_default_passes_through_v_prefixed_version() {
        // Happy path: a fully-qualified `vX.Y.Z` default file is returned
        // verbatim (the common case for nvm-sh users who set a specific
        // version as default).
        let root = fake_nvm_sh_root("v20.11.0", &[]);
        assert_eq!(
            detect_nvm_sh_default(root.path()),
            Some("v20.11.0".to_string())
        );
    }

    #[test]
    fn detect_nvm_sh_default_passes_through_iojs_version() {
        // All four io.js spellings should be accepted by the shared
        // `is_iojs_version` detector used in the v-prefix branch.
        for s in ["iojs-v3.3.1", "iojs-3.3.1", "io.js-v3.3.1", "io.js-3.3.1"] {
            let root = fake_nvm_sh_root(s, &[]);
            assert_eq!(
                detect_nvm_sh_default(root.path()),
                Some(s.to_string()),
                "io.js spelling {s:?} should pass through"
            );
        }
    }

    #[test]
    fn detect_nvm_sh_default_adds_v_prefix_to_bare_full_version() {
        // "20.11.0" (no leading v) gets a `v` prepended.
        let root = fake_nvm_sh_root("20.11.0", &[]);
        assert_eq!(
            detect_nvm_sh_default(root.path()),
            Some("v20.11.0".to_string())
        );
    }

    #[test]
    fn detect_nvm_sh_default_returns_none_for_missing_file() {
        // No default file → None (not an error). This is the expected case
        // when nvm-sh is installed but no default alias was ever set.
        let tmp = tempfile::tempdir().expect("tempdir");
        fs::create_dir_all(tmp.path().join(".nvm").join("alias")).expect("create alias dir");
        assert_eq!(detect_nvm_sh_default(tmp.path()), None);
    }

    #[test]
    fn detect_nvm_sh_default_returns_none_for_empty_file() {
        // An empty/whitespace default file is treated as "no default".
        let root = fake_nvm_sh_root("   \n  ", &[]);
        assert_eq!(detect_nvm_sh_default(root.path()), None);
    }

    #[test]
    fn detect_nvm_sh_default_lts_star_returns_newest_lts() {
        // `lts/*` must resolve to the newest LTS version present in the
        // source install. Here v22.11.0 > v22.5.1 > v20.17.0 > v18.20.0,
        // so the answer is v22.11.0.
        let root = fake_nvm_sh_root(
            "lts/*",
            &["v22.5.1", "v22.11.0", "v20.10.0", "v20.17.0", "v18.20.0"],
        );
        assert_eq!(
            detect_nvm_sh_default(root.path()),
            Some("v22.11.0".to_string()),
            "lts/* must pick the newest LTS version"
        );
    }

    #[test]
    fn detect_nvm_sh_default_lts_minus_one_returns_previous_lts_line() {
        // Regression for the lts/-N bug: previously `lts/-1` fell through to
        // "latest of all LTS" (i.e. v22.11.0, the newest LTS line), silently
        // wrong — nvm-sh's `lts/-1` means the LTS line BEFORE the newest.
        // With v22/v20/v18 LTS lines installed, lts/-1 must pick the newest
        // version on the v20 line (v20.17.0), NOT v22.11.0.
        let root = fake_nvm_sh_root(
            "lts/-1",
            &["v22.5.1", "v22.11.0", "v20.10.0", "v20.17.0", "v18.20.0"],
        );
        assert_eq!(
            detect_nvm_sh_default(root.path()),
            Some("v20.17.0".to_string()),
            "lts/-1 must pick the newest version on the previous LTS line"
        );
    }

    #[test]
    fn detect_nvm_sh_default_lts_minus_two_returns_two_lines_back() {
        // lts/-2 → two LTS lines back from the newest. With v22/v20/v18
        // installed, that's the v18 line → v18.20.0.
        let root = fake_nvm_sh_root(
            "lts/-2",
            &["v22.5.1", "v22.11.0", "v20.10.0", "v20.17.0", "v18.20.0"],
        );
        assert_eq!(
            detect_nvm_sh_default(root.path()),
            Some("v18.20.0".to_string()),
            "lts/-2 must pick the newest version on the 2nd-previous LTS line"
        );
    }

    #[test]
    fn detect_nvm_sh_default_lts_offset_out_of_range_returns_none() {
        // lts/-999 → no LTS line that far back. Must return None (not fall
        // through to "latest LTS" as the old code did).
        let root = fake_nvm_sh_root("lts/-999", &["v22.11.0", "v20.17.0", "v18.20.0"]);
        assert_eq!(
            detect_nvm_sh_default(root.path()),
            None,
            "out-of-range lts/-N must return None, not latest LTS"
        );
    }

    #[test]
    fn detect_nvm_sh_default_lts_minus_zero_falls_back_to_latest() {
        // lts/-0 is nonsensical; we treat it like lts/* (latest LTS) rather
        // than erroring, matching the lenient handling in
        // `config::resolve_lts_relative` for offset == 0.
        let root = fake_nvm_sh_root("lts/-0", &["v22.11.0", "v20.17.0"]);
        assert_eq!(
            detect_nvm_sh_default(root.path()),
            Some("v22.11.0".to_string()),
            "lts/-0 should behave like lts/*"
        );
    }

    #[test]
    fn detect_nvm_sh_default_passes_through_traversal_payload_unvalidated() {
        // SECURITY CONTRACT TEST:
        //
        // `detect_nvm_sh_default` is a pure parser — it does NOT validate the
        // contents of the default file. A hand-edited or malicious file
        // containing `v../../etc/passwd` is returned verbatim because it
        // starts with `v`. This is intentional: the validation gate lives
        // in `cmd_migrate` (the only caller), which calls
        // `validate_version_name` before `nvm_dir.join(&default_ver)` and
        // before writing to config.default_version.
        //
        // This test locks the contract: if someone moves validation INTO
        // `detect_nvm_sh_default` they must update this test; if someone
        // REMOVES validation from `cmd_migrate`, this test still passes but
        // the safety gap reopens — the `validate_version_name` test below
        // is the second layer of the contract.
        let root = fake_nvm_sh_root("v../../etc/passwd", &[]);
        assert_eq!(
            detect_nvm_sh_default(root.path()),
            Some("v../../etc/passwd".to_string()),
            "detect_nvm_sh_default must return the raw value; cmd_migrate validates"
        );
        // And the validator MUST reject this payload — this is the gate
        // cmd_migrate relies on. If validate_version_name ever started
        // accepting `..` or `/`, cmd_migrate's defense would be defeated.
        assert!(
            crate::utils::validate_version_name("v../../etc/passwd").is_err(),
            "validate_version_name must reject path-traversal payloads"
        );
    }
}
