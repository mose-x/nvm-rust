use anyhow::Result;
use colored::Colorize;
use std::collections::BTreeMap;
use std::fs;
use std::process::Command;

use crate::config::{load_aliases, load_config, save_aliases};
use crate::i18n::{format_t, T};
use crate::system::{get_nvm_dir, get_tags, URI};

// Compiled once: extracts the leading major from a `vX.Y.Z` tag, used by
// `find_latest_unstable` to pick the highest odd-major release. Cached so a
// repeated `nvm alias default unstable` doesn't recompile the regex.
lazy_static::lazy_static! {
    static ref UNSTABLE_MAJOR_RE: regex::Regex =
        regex::Regex::new(r"^v(\d+)\.").expect("unstable-major regex");
}

/// `lts/<codename>` → `v<major>` aliases, derived from the single source of
/// truth in [`crate::utils::lts_codename_to_major`].
///
/// Both this map and [`crate::utils::lts_codename_to_major`] previously held
/// their own hardcoded copy of the codename→major table, which had to be
/// kept in sync by hand — forgetting one half meant `nvm use lts/argon`
/// could resolve while `is_lts_version("v4.0.0")` returned false (or vice
/// versa). Deriving here keeps one table (`utils`) as the authority.
pub fn named_lts_aliases() -> BTreeMap<String, String> {
    crate::utils::lts_codename_to_major()
        .iter()
        .map(|(codename, major)| (format!("lts/{}", codename), format!("v{}", major)))
        .collect()
}

/// Return `lts/<codename>` → `v<major>` aliases, merging the hardcoded
/// fallback with a live `index.json` fetch. Dynamic entries override
/// fallback entries (so a codename that moved majors wins) and add any
/// new codename not yet shipped in the table. On network/parse failure the
/// fallback table is returned unchanged.
///
/// Use this in network-capable code paths (install, `nvm use lts/<name>`).
/// The no-arg `named_lts_aliases` stays for synchronous paths.
pub fn named_lts_aliases_with_remote(base_url: &str) -> BTreeMap<String, String> {
    let mut m = named_lts_aliases();
    let remote = crate::system::fetch_lts_codename_map(base_url);
    for (codename, major) in remote {
        let alias = format!("lts/{}", codename);
        m.insert(alias, format!("v{}", major));
    }
    m
}

pub fn set_alias(name: &str, version: Option<&str>) -> Result<()> {
    if name.is_empty() {
        anyhow::bail!("{}", T("alias_name_empty"));
    }
    // Hold the nvm lock across the load→modify→save so two concurrent
    // `nvm alias` calls don't lose updates: both would otherwise load the
    // same alias.json, each insert into its in-memory copy, and the second
    // save would silently overwrite the first (lost update). `atomic_write`
    // only guarantees a single write is atomic, not the whole transaction.
    // Re-entrant: an outer caller already holding the lock gets a no-op
    // guard instead of self-deadlocking.
    let nvm_dir = get_nvm_dir();
    let _nvm_lock = crate::utils::acquire_nvm_lock(&nvm_dir)?;
    let mut aliases = load_aliases()?;

    match version {
        Some(v) => {
            let resolved = resolve_alias(v)?;
            let version_dir = nvm_dir.join(&resolved);
            if !version_dir.exists() {
                anyhow::bail!(
                    "{}",
                    format_t("not_installed", std::slice::from_ref(&resolved))
                );
            }

            aliases.aliases.insert(name.to_string(), resolved.clone());
            println!(
                "{}",
                format_t("alias_set", &[name.to_string(), resolved.clone()]).green()
            );
            save_aliases(&aliases)?;
        }
        None => {
            if let Some(v) = aliases.aliases.get(name) {
                println!(
                    "{} {} {}",
                    name.cyan().bold(),
                    "→".dimmed(),
                    v.white().bold()
                );
            } else {
                println!(
                    "{} {}",
                    "✗".red().bold(),
                    format_t("alias_not_found", &[name.to_string()]).red()
                );
            }
        }
    }

    Ok(())
}

pub fn remove_alias(name: &str) -> Result<()> {
    // Same read-modify-write transaction as `set_alias`: hold the nvm lock
    // across load→remove→save to prevent a concurrent `set_alias`/
    // `remove_alias` from overwriting this removal (or vice versa).
    let nvm_dir = get_nvm_dir();
    let _nvm_lock = crate::utils::acquire_nvm_lock(&nvm_dir)?;
    let mut aliases = load_aliases()?;

    if aliases.aliases.remove(name).is_some() {
        save_aliases(&aliases)?;
        println!("{}", format_t("alias_removed", &[name.to_string()]).green());
        Ok(())
    } else {
        anyhow::bail!("{}", format_t("alias_not_found", &[name.to_string()]));
    }
}

pub fn list_all_aliases() -> Result<()> {
    let aliases = load_aliases()?;
    let nvm_dir = get_nvm_dir();
    let mut entries: Vec<(String, String, bool)> = Vec::new();

    // Read the nvm dir ONCE and collect (name, major) for every installed
    // version directory. The previous loop called fs::read_dir once per LTS
    // alias (11 directory scans) and re-parsed every entry each time, even
    // though the listing is identical across iterations.
    //
    // `NotFound` is legitimate on a fresh install (no versions downloaded
    // yet) and is treated as an empty list silently. Any other read_dir
    // failure (permission denied, I/O error, ...) was previously folded
    // into an empty list by `.unwrap_or_default()`, which made the alias
    // listing look empty without any hint that something was wrong.
    let installed_majors: Vec<(String, u32)> = match fs::read_dir(&nvm_dir) {
        Ok(rd) => rd
            .flatten()
            .filter_map(|entry| {
                let s = entry.file_name().to_str()?.to_string();
                if crate::utils::is_version_dir_name(&s) {
                    crate::utils::parse_major(&s).map(|m| (s, m))
                } else {
                    None
                }
            })
            .collect(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => {
            // Surface the real cause but keep the command usable: custom
            // aliases (from the alias file) can still be listed below.
            eprintln!(
                "{} Failed to list installed versions in {}: {}",
                "⚠".yellow().bold(),
                nvm_dir.display(),
                e
            );
            Vec::new()
        }
    };

    for (name, prefix) in named_lts_aliases() {
        // Strict match: the version's major must equal the alias's target
        // major. Without this, `lts/argon` (prefix "v4") would also match
        // "v40.0.0" because "v40.0.0".starts_with("v4") is true.
        let prefix_major: u32 = prefix.trim_start_matches('v').parse().unwrap_or(0);
        let mut installed: Vec<String> = installed_majors
            .iter()
            .filter_map(|(s, major)| {
                if *major == prefix_major {
                    Some(s.clone())
                } else {
                    None
                }
            })
            .collect();
        installed.sort();
        if let Some(latest) = installed.last() {
            entries.push((name.to_string(), latest.clone(), true));
        }
    }

    for (k, v) in &aliases.aliases {
        entries.push((k.clone(), v.clone(), false));
    }

    if entries.is_empty() {
        println!("{} {}", "ℹ".cyan().bold(), T("no_aliases").cyan());
        return Ok(());
    }

    entries.sort_by(|a, b| a.0.cmp(&b.0));

    println!("{}", crate::i18n::T("aliases_title").cyan().bold());
    for (k, v, is_lts) in entries {
        let tag = if is_lts {
            " LTS".green().to_string()
        } else {
            "".to_string()
        };
        println!(
            "  {} {} {} {}{}",
            "•".cyan(),
            k.bold(),
            "→".dimmed(),
            v.white(),
            tag
        );
    }

    Ok(())
}

/// Validate a resolved alias/version string before returning it from
/// `resolve_alias`. This is the defense-in-depth gate for values sourced
/// from on-disk JSON (`aliases.json`, `config.json`) or the `current` file:
/// `set_alias` validates on write, but a user hand-editing the JSON (or an
/// attacker with write access to `~/.nvm`) could inject path-traversal
/// payloads like `v1.0.0/../../etc/passwd` that would later escape
/// `nvm_dir` via `nvm_dir.join(&version)`. Reject such payloads here.
///
/// `lts/*`, `lts/-N` and `lts/<codename>` are alias forms that legitimately
/// contain a slash; they will be re-resolved recursively by the caller and
/// hit the terminal fallback's `validate_version_name`. For those we only
/// reject traversal markers (`..`, NUL, control chars). Every other value
/// goes through the full `validate_version_name` (which forbids `/`, `\`,
/// `..`, control chars, spaces) — accepting `v20.0.0`, `iojs-v3.3.1`,
/// `system:v20.0.0`, and alias-of-alias names like `lts` / `default`.
fn validated(v: &str) -> Result<String> {
    if v.starts_with("lts/") {
        if v.contains("..") || v.contains('\0') || v.chars().any(|c| c.is_control()) {
            anyhow::bail!("{}", format_t("invalid_version_name", &[v.to_string()]));
        }
        return Ok(v.to_string());
    }
    crate::utils::validate_version_name(v)?;
    Ok(v.to_string())
}

pub fn resolve_alias(name: &str) -> Result<String> {
    // Reject empty / whitespace-only input early. Without this, `nvm use ""`
    // would fall through to `resolve_version`, which prepends "v" to the
    // empty string and produces the confusing "Version v is not installed"
    // instead of a clear "specify a version" message. Trim once so every
    // comparison below uses the cleaned form.
    let name = name.trim();
    if name.is_empty() {
        anyhow::bail!("{}", T("alias_name_empty"));
    }
    // A user-defined alias named "default" (via `nvm alias default X`) takes
    // precedence over the --save'd default_version, so the alias isn't dead.
    if name == "default" {
        if let Ok(aliases) = load_aliases() {
            if let Some(v) = aliases.aliases.get(name) {
                return validated(v);
            }
        }
        let config = load_config()?;
        if let Some(v) = config.default_version {
            return validated(&v);
        }
        anyhow::bail!("{}", T("no_default_version"));
    }

    // "current" resolves to whatever version is active right now (the
    // contents of the `current` file). Enables `nvm which current`,
    // `nvm use current`, `nvm exec current ...`, etc.
    if name == "current" {
        let current_file = get_nvm_dir().join("current");
        // Read directly and map NotFound → "no current version set" instead
        // of `exists()` + `read_to_string`. The two-step form is a TOCTOU
        // race (file could be removed between stat and open) and the
        // previous `if let Ok(content)` silently swallowed real read errors
        // (permission denied, I/O) as "no current version set", hiding the
        // actual cause from the user.
        match fs::read_to_string(&current_file) {
            Ok(content) => {
                let v = content.trim();
                if !v.is_empty() {
                    return validated(v);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                eprintln!("{} {}: {}", "⚠".yellow().bold(), current_file.display(), e);
            }
        }
        anyhow::bail!("{}", T("no_current_version_set"));
    }

    if name == "system" {
        // Use the path resolved by `find_system_node_path` (which runs
        // `which`/`where`) directly, instead of re-resolving `node` through
        // PATH via `Command::new("node")`. The two lookups can disagree:
        // `which` may follow shell hash tables/aliases, or PATH may have
        // changed between calls, so `Command::new("node")` could execute a
        // different binary (e.g. the nvm-activated version) and report the
        // wrong version. Feeding the resolved PathBuf to Command::new
        // guarantees we probe the same node that `which`/`where` found.
        if let Some(node_path) = crate::utils::find_system_node_path() {
            if let Ok(v) = Command::new(&node_path).arg("--version").output() {
                let v = String::from_utf8_lossy(&v.stdout).trim().to_string();
                if !v.is_empty() {
                    return validated(&format!("system:{}", v));
                }
            }
        }
        anyhow::bail!("{}", T("system_node_not_found"));
    }

    if name.starts_with("lts/") {
        // `lts/*` → newest installed LTS version (any line). Mirrors nvm-sh's
        // `nvm alias default lts/*` / `nvm use lts/*`.
        if name == "lts/*" {
            return find_latest_installed_lts();
        }

        // `lts/-N` (N >= 1) → the Nth-previous LTS *line* relative to the
        // newest known LTS line, then the newest installed version on that
        // line. e.g. if the newest LTS line is v24 (krypton):
        //   lts/-1 → v22 (jodhpur), lts/-2 → v20 (iron), ...
        // This is nvm-sh's `lts/-1` / `lts/-2` shorthand for "the LTS before
        // the latest". We resolve against the known LTS table (not just
        // installed versions) so `lts/-1` is stable even if the newest line
        // isn't installed locally.
        if let Some(offset_str) = name.strip_prefix("lts/-") {
            if let Ok(offset) = offset_str.parse::<usize>() {
                if offset == 0 {
                    // lts/-0 is nonsensical; treat like lts/* for safety.
                    return find_latest_installed_lts();
                }
                return resolve_lts_relative(offset);
            }
            // Non-numeric suffix (e.g. "lts/-foo") falls through to the
            // codename lookup below, which will bail with unknown_lts_alias.
        }

        let aliases = named_lts_aliases();
        if let Some(prefix) = aliases.get(name) {
            return find_latest_installed(prefix);
        }
        anyhow::bail!("{}", format_t("unknown_lts_alias", &[name.to_string()]));
    }

    if name == "lts" {
        // `use lts` / `nvm alias default lts` must resolve to the latest
        // installed LTS version, NOT just the latest installed version.
        // Without the LTS filter, `use lts` would happily return a non-LTS
        // build (e.g. v26.x.x installed via `nvm install --latest`).
        return find_latest_installed_lts();
    }

    if name == "node" || name == "stable" {
        return find_latest_installed("v");
    }

    if name == "unstable" {
        return find_latest_unstable();
    }

    let aliases = load_aliases()?;
    if let Some(v) = aliases.aliases.get(name) {
        return validated(v);
    }

    // Bare major / major.minor shorthand (e.g. "22", "22.5", "v22.5"):
    // resolve to the *latest installed* version that matches, so commands like
    // `nvm use 22`, `nvm which 22`, `nvm exec 22 ...` pick v22.22.2 if that's
    // what's installed (matches nvm-sh behavior). If nothing is installed we
    // fall through to "v22" so the caller can produce its usual
    // "not installed, run nvm install" message instead of a confusing bare
    // number.
    if let Some(prefix) = bare_major_prefix(name) {
        if let Ok(latest) = find_latest_installed(&prefix) {
            return Ok(latest);
        }
    }

    let mut version = name.to_string();
    // Don't prepend "v" to io.js versions ("iojs-...", "io.js-...") or to
    // already-prefixed/system versions; otherwise "iojs-v3.3.1" would become
    // the nonsensical "viojs-v3.3.1".
    if !version.starts_with('v')
        && !version.starts_with("system:")
        && !version.starts_with("iojs")
        && !version.starts_with("io.js")
    {
        version = format!("v{}", version);
    }
    // Reject path-traversal payloads (`v1.0.0/../../etc`) before they reach
    // any `nvm_dir.join(&version)` / `fs::remove_dir_all` caller. This is
    // the terminal fallback for unknown inputs, so a malicious `.nvmrc`
    // line or `nvm use "v1/../../x"` both stop here.
    crate::utils::validate_version_name(&version)?;
    Ok(version)
}

/// If `name` is a bare major ("22") or major.minor ("22.5") shorthand,
/// optionally with a leading `v` ("v22", "v22.5"), return the versioned
/// prefix to look up among installed versions ("v22."). Returns `None` for
/// fully-specified versions ("22.5.1"), aliases ("lts/iron"), io.js names,
/// `system`, etc. — those have their own resolution paths.
fn bare_major_prefix(name: &str) -> Option<String> {
    let s = crate::utils::validate_bare_major(name)?;
    Some(format!("v{}.", s))
}

/// Walk the nvm directory and collect version directory names that satisfy
/// `predicate`. Shared by [`find_latest_installed`] and
/// [`find_latest_installed_lts`] so the read_dir + sort logic lives in one
/// place. Returns the names unsorted; callers sort by semver.
fn collect_installed_versions(predicate: impl Fn(&str) -> bool) -> Vec<String> {
    let nvm_dir = get_nvm_dir();
    let mut versions: Vec<String> = Vec::new();
    // Distinguish NotFound (fresh install — no versions yet) from real
    // read_dir failures. The previous `if let Ok(rd)` folded both into an
    // empty list, so an unreadable nvm_dir made every alias resolution
    // (`lts/*`, `node`, `stable`, bare major) silently return "not found"
    // instead of surfacing the permission/IO error. Mirrors
    // `list_all_aliases` and `get_installed_versions`.
    let rd = match fs::read_dir(&nvm_dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return versions,
        Err(e) => {
            eprintln!(
                "{} Failed to list installed versions in {}: {}",
                "⚠".yellow().bold(),
                nvm_dir.display(),
                e
            );
            return versions;
        }
    };
    for entry in rd.flatten() {
        if let Some(s) = entry.file_name().to_str() {
            if crate::utils::is_version_dir_name(s) && predicate(s) {
                versions.push(s.to_string());
            }
        }
    }
    versions
}

/// Pick the newest entry from `versions` by semver. Returns `Err` when empty
/// rather than `unwrap()`-ing on the implicit non-empty contract — a future
/// refactor that moves the empty-check could otherwise trigger a panic.
fn pick_latest(versions: Vec<String>) -> Result<String> {
    let mut v = versions;
    v.sort_by(|a, b| crate::utils::compare_semver(a, b));
    v.last()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("no versions collected"))
}

fn find_latest_installed(prefix: &str) -> Result<String> {
    // Strict major match when prefix is `vN` (no dot). Without
    // this, `lts/hydrogen` (prefix "v18") would also match a
    // hypothetical "v180.0.0" install. The `v22.` form returned
    // by `bare_major_prefix` already encodes the dot so the
    // starts_with check above is sufficient there; this branch
    // only adds the major equality for the bare `vN` aliases.
    let prefix_major: Option<String> = if !prefix.contains('.') && prefix.len() > 1 {
        Some(prefix.trim_start_matches('v').to_string())
    } else {
        None
    };
    let prefix_owned = prefix.to_string();
    let versions = collect_installed_versions(|s| {
        if !s.starts_with(&prefix_owned) {
            return false;
        }
        if let Some(ref want_major) = prefix_major {
            match crate::utils::parse_major(s) {
                Some(m) => m.to_string() == *want_major,
                None => false,
            }
        } else {
            true
        }
    });
    if versions.is_empty() {
        anyhow::bail!("{}", format_t("no_matching_version", &[prefix.to_string()]));
    }
    // Sort semantically (numeric major.minor.patch), not alphabetically:
    // alphabetical sort would put `v20.5.0` after `v20.20.2` ('5' > '2'),
    // returning the older version as "latest".
    pick_latest(versions)
}

fn find_latest_installed_lts() -> Result<String> {
    let versions = collect_installed_versions(crate::utils::is_lts_version);
    if versions.is_empty() {
        anyhow::bail!("{}", T("no_installed_lts"));
    }
    pick_latest(versions)
}

/// Resolve `lts/-N`: pick the LTS *line* that is `offset` lines older than
/// the newest known LTS line, then return the newest installed version on
/// that line.
///
/// LTS lines are taken from [`named_lts_aliases`] and sorted by major version
/// (the codenames happen to sort alphabetically == by major, but we sort
/// numerically to be robust against future non-alphabetical codenames).
/// `offset == 1` → the line immediately before the newest; `offset == 2` →
/// two lines before, etc.
///
/// Bails if `offset` is larger than the number of known LTS lines minus one
/// (i.e. there is no line that far back), or if no version is installed on
/// the selected line.
fn resolve_lts_relative(offset: usize) -> Result<String> {
    // Collect (major) for every known LTS codename, sorted ascending.
    let mut majors: Vec<u32> = named_lts_aliases()
        .values()
        .filter_map(|prefix| prefix.trim_start_matches('v').parse::<u32>().ok())
        .collect();
    majors.sort_unstable();
    if majors.is_empty() {
        anyhow::bail!("{}", T("no_installed_lts"));
    }

    // Index from the newest (last) backwards. offset=1 → second-newest.
    // saturating_sub guards against offset > len, mapped to an explicit bail.
    let idx = majors.len().checked_sub(1 + offset);
    let Some(&major) = idx.and_then(|i| majors.get(i)) else {
        anyhow::bail!(
            "{}",
            format_t("lts_offset_out_of_range", &[offset.to_string()])
        );
    };
    find_latest_installed(&format!("v{}", major))
}

fn find_latest_unstable() -> Result<String> {
    // Resolve the configured mirror (if any) so `nvm use unstable` /
    // `nvm alias default unstable` honours `nvm mirror taobao` instead of
    // always hitting nodejs.org. Previously this hardcoded `URI`, which
    // silently broke the alias behind the GFW / on offline mirrors even
    // though every other version-resolution path already accepted a
    // `base_url`. Reading the config here (rather than threading a
    // `base_url` param through `resolve_alias`) avoids a 30-call-site
    // signature change for a rarely-used alias.
    let base_url = load_config()
        .map(|c| c.mirror.unwrap_or_else(|| URI.to_string()))
        .unwrap_or_else(|_| URI.to_string());
    let tags = get_tags(&base_url)?;
    let mut odd_max: Option<(u32, String)> = None;
    for tag in tags {
        let v = tag.trim_end_matches('/');
        if v.starts_with('v') {
            if let Some(caps) = UNSTABLE_MAJOR_RE.captures(v) {
                if let Ok(major) = caps[1].parse::<u32>() {
                    if major % 2 == 1 {
                        let version = v.to_string();
                        if odd_max.as_ref().is_none_or(|(m, _)| major >= *m) {
                            odd_max = Some((major, version));
                        }
                    }
                }
            }
        }
    }
    if let Some((_, v)) = odd_max {
        return Ok(v);
    }
    anyhow::bail!("{}", T("no_unstable"))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_named_lts_aliases() {
        let aliases = named_lts_aliases();
        assert_eq!(aliases.len(), 11);
        assert_eq!(aliases.get("lts/argon"), Some(&"v4".to_string()));
        assert_eq!(aliases.get("lts/iron"), Some(&"v20".to_string()));
        assert_eq!(aliases.get("lts/jodhpur"), Some(&"v22".to_string()));
        assert_eq!(aliases.get("lts/krypton"), Some(&"v24".to_string()));
        assert_eq!(aliases.get("lts/unknown"), None);
    }

    #[test]
    fn test_resolve_lts_relative_out_of_range_bails() {
        // There are 11 known LTS lines (v4..v24). An offset far beyond that
        // must bail with the out-of-range message rather than panic on
        // underflow / index out of bounds.
        let err = resolve_lts_relative(9999).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("out of range") || msg.contains("超出范围"));
    }

    #[test]
    fn test_resolve_alias_lts_offset_within_table_does_not_panic() {
        // lts/-1 targets the second-newest LTS line (v22 if v24 is newest).
        // It may bail with "no matching installed version" if v22 isn't
        // installed, but must NOT panic or return a success with garbage.
        let res = resolve_alias("lts/-1");
        match res {
            Ok(v) => assert!(
                v.starts_with("v22") || v.starts_with("v20"),
                "lts/-1 should target v22 or v20, got {v}"
            ),
            Err(e) => {
                let m = format!("{e}");
                assert!(
                    m.contains("matching") || m.contains("匹配"),
                    "lts/-1 error should be 'no matching version', got: {m}"
                );
            }
        }
    }

    // ---- resolve_alias: pure (non-filesystem) paths ----
    //
    // `resolve_alias` has several branches that never touch the nvm dir:
    // empty-input rejection, the `lts/<codename>` table lookup (bails before
    // scanning installed versions when the codename is unknown), and the
    // terminal fallback that normalises the version string and runs it
    // through `validate_version_name`. These paths are the security boundary
    // (path-traversal rejection) and the most common resolution shape, so
    // they deserve deterministic unit tests that don't depend on whatever
    // versions happen to be installed in the test env.

    #[test]
    fn test_resolve_alias_rejects_empty() {
        // Empty input must bail early with alias_name_empty rather than
        // falling through to produce the confusing "Version v is not
        // installed".
        assert!(resolve_alias("").is_err());
    }

    #[test]
    fn test_resolve_alias_rejects_whitespace_only() {
        // Whitespace is trimmed before the empty check, so "   " is treated
        // exactly like "".
        assert!(resolve_alias("   ").is_err());
        assert!(resolve_alias("\t\n").is_err());
    }

    #[test]
    fn test_resolve_alias_rejects_unknown_lts_codename() {
        // An unknown `lts/<name>` must bail with unknown_lts_alias (which
        // interpolates the input) BEFORE attempting any installed-version
        // scan — so this is deterministic regardless of what's installed.
        let err = resolve_alias("lts/doesnotexist").unwrap_err();
        let m = format!("{err}");
        assert!(
            m.contains("lts/doesnotexist"),
            "error should name the alias: {m}"
        );
    }

    #[test]
    fn test_resolve_alias_rejects_non_numeric_lts_offset() {
        // `lts/-foo`: the non-numeric suffix fails `parse::<usize>`, falls
        // through to the codename lookup (which won't match `lts/-foo`) and
        // bails with unknown_lts_alias. Must not panic on the parse.
        let err = resolve_alias("lts/-foo").unwrap_err();
        let m = format!("{err}");
        assert!(m.contains("lts/-foo"), "error should name the alias: {m}");
    }

    #[test]
    fn test_resolve_alias_rejects_path_traversal() {
        // The terminal fallback runs every unknown input through
        // `validate_version_name`. A slash-bearing payload must be rejected
        // here so a malicious `.nvmrc` / `nvm use "v1/../../etc"` can't
        // escape nvm_dir via a later `nvm_dir.join(&version)`.
        let err = resolve_alias("v1.0.0/../../etc").unwrap_err();
        let m = format!("{err}");
        assert!(
            m.contains("v1.0.0/../../etc"),
            "path-traversal must be rejected with the offending name: {m}"
        );
    }

    #[test]
    fn test_resolve_alias_rejects_backslash_traversal() {
        // Windows-style traversal must also be rejected — `validate_version_name`
        // forbids backslashes on every platform so a payload crafted for one
        // OS can't slip through on the other.
        assert!(resolve_alias("v1\\..\\x").is_err());
    }

    #[test]
    fn test_resolve_alias_rejects_parent_dir_token() {
        // A bare ".." token (no slash) is still rejected by validate_version_name,
        // blocking e.g. `nvm uninstall ".."` from resolving to a parent dir.
        assert!(resolve_alias("v1..2").is_err());
    }

    #[test]
    fn test_resolve_alias_rejects_null_byte() {
        // Control characters (incl. NUL) are forbidden so they can't be used
        // to truncate the version string mid-path on C-based path APIs.
        assert!(resolve_alias("v1\0x").is_err());
    }

    #[test]
    fn test_resolve_alias_passes_through_v_prefixed_version() {
        // A fully-specified `vX.Y.Z` is not a bare-major shorthand (it has 2
        // dots, so `bare_major_prefix` returns None) and reaches the terminal
        // fallback, which leaves the `v` prefix intact.
        assert_eq!(resolve_alias("v22.5.1").unwrap(), "v22.5.1");
    }

    #[test]
    fn test_resolve_alias_prepends_v_to_bare_version() {
        // A bare `X.Y.Z` (no leading v) gets a `v` prepended so downstream
        // code always sees the canonical `vX.Y.Z` form. io.js / system: forms
        // are excluded from this prepend (see tests below).
        assert_eq!(resolve_alias("22.5.1").unwrap(), "v22.5.1");
    }

    #[test]
    fn test_resolve_alias_passes_through_iojs_version() {
        // io.js names must NOT get a `v` prepended — otherwise "iojs-v3.3.1"
        // would become the nonsensical "viojs-v3.3.1". The terminal fallback
        // recognises the "iojs" prefix and skips the prepend.
        assert_eq!(resolve_alias("iojs-v3.3.1").unwrap(), "iojs-v3.3.1");
    }

    #[test]
    fn test_resolve_alias_passes_through_iojs_dot_version() {
        // The "io.js-" spelling must also skip the v-prepend.
        assert_eq!(resolve_alias("io.js-v3.3.1").unwrap(), "io.js-v3.3.1");
    }

    #[test]
    fn validated_accepts_legitimate_values() {
        // Every form that can legitimately come from aliases.json /
        // config.json / the current file must pass the defense-in-depth
        // gate. If any of these started being rejected, `nvm use default` /
        // `nvm use current` / alias chains would break.
        assert_eq!(validated("v20.11.0").unwrap(), "v20.11.0");
        assert_eq!(validated("20.11.0").unwrap(), "20.11.0");
        assert_eq!(validated("iojs-v3.3.1").unwrap(), "iojs-v3.3.1");
        assert_eq!(validated("io.js-v2.5.0").unwrap(), "io.js-v2.5.0");
        assert_eq!(validated("system:v20.0.0").unwrap(), "system:v20.0.0");
        // Aliases can point at other aliases (alias-of-alias chains).
        assert_eq!(validated("lts").unwrap(), "lts");
        assert_eq!(validated("lts/*").unwrap(), "lts/*");
        assert_eq!(validated("lts/-1").unwrap(), "lts/-1");
        assert_eq!(validated("lts/iron").unwrap(), "lts/iron");
        assert_eq!(validated("default").unwrap(), "default");
        // Bare major / major.minor shorthand.
        assert_eq!(validated("22").unwrap(), "22");
        assert_eq!(validated("v22.5").unwrap(), "v22.5");
    }

    #[test]
    fn validated_rejects_path_traversal_from_disk() {
        // Regression for the defense-in-depth gap: previously the `default`,
        // `current`, and user-alias branches of `resolve_alias` returned
        // values read straight from on-disk JSON / the current file WITHOUT
        // calling `validate_version_name`. A user hand-editing
        // `~/.nvm/alias/default` to `v1.0.0/../../etc/passwd` would have
        // escaped nvm_dir via `nvm_dir.join(&version)` on the next
        // `nvm use default`. `validated` must reject every traversal shape.
        assert!(validated("v1.0.0/../../etc").is_err());
        assert!(validated("v1.0.0\\..\\etc").is_err());
        assert!(validated("..").is_err());
        assert!(validated("v1..2").is_err());
        assert!(validated("v1\0x").is_err());
        assert!(validated("v1.0.0 ../etc").is_err());
        assert!(validated("").is_err());
        // A `system:` prefix with traversal in the version part must also be
        // rejected — `system:v1/../../x` would escape just as easily.
        assert!(validated("system:v1/../../etc").is_err());
        // An `lts/` prefix hiding traversal must be rejected too — the
        // lts/ fast-path only allows the alias forms (lts/*, lts/-N,
        // lts/<codename>), not `lts/../../etc`.
        assert!(validated("lts/../../etc").is_err());
        assert!(validated("lts/\0x").is_err());
    }
}
