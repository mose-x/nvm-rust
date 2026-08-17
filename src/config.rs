use anyhow::{Context, Result};
use colored::Colorize;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::i18n::{format_t, t_en, T};
use crate::system::{get_nvm_dir, get_tags, ALIAS_FILE, CONFIG_FILE, URI};
use crate::utils::{atomic_write, backup_file};

// Compiled once: extracts the leading major from a `vX.Y.Z` tag, used by
// `find_latest_unstable` to pick the highest odd-major release. Cached so a
// repeated `nvm alias default unstable` doesn't recompile the regex.
lazy_static::lazy_static! {
    static ref UNSTABLE_MAJOR_RE: regex::Regex =
        regex::Regex::new(r"^v(\d+)\.").expect("unstable-major regex");
}

#[derive(Serialize, Deserialize, Default)]
pub struct Config {
    pub mirror: Option<String>,
    pub default_version: Option<String>,
    pub language: Option<String>,
    pub proxy: Option<bool>,
    pub use_on_cd: Option<bool>,
}

#[derive(Serialize, Deserialize, Default)]
pub struct Aliases {
    pub aliases: BTreeMap<String, String>,
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

pub fn load_config() -> Result<Config> {
    let config_file = get_nvm_dir().join(CONFIG_FILE);

    // Read directly and map NotFound → default, instead of `exists()` +
    // `read_to_string`. The two-step form is a TOCTOU race — the file could
    // be removed (or replaced) between the exists check and the read, and
    // a concurrent `save_config` running on another CPU could even swap a
    // fresh write into place between our stat and our open. A single read
    // that maps NotFound to the default is both faster (one syscall) and
    // race-free. Mirrors the pattern in `commands::get_current_version`.
    let content = match fs::read_to_string(&config_file) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Config::default()),
        Err(e) => return Err(e.into()),
    };
    // Surface parse errors instead of silently dropping all settings.
    // Returning default on a corrupt file would cause the next
    // save_config to overwrite it with an empty config, permanently
    // losing the user's mirror/aliases/language.
    //
    // Use `t_en` (not `T`) for the hint: `T()` → `get_language()` →
    // `load_config()`, so formatting this bail message with `T()` would
    // recurse infinitely on a corrupt config and abort with a stack
    // overflow. `t_en` resolves the English string directly.
    match serde_json::from_str::<Config>(&content) {
        Ok(c) => Ok(c),
        Err(e) => anyhow::bail!(
            "{}: {} ({})",
            config_file.display(),
            e,
            t_en("config_corrupt_hint")
        ),
    }
}

pub fn save_config(config: &Config) -> Result<()> {
    let config_file = get_nvm_dir().join(CONFIG_FILE);
    let content = serde_json::to_string_pretty(config)?;
    atomic_write(&config_file, &content)?;
    Ok(())
}

pub fn load_aliases() -> Result<Aliases> {
    let alias_file = get_nvm_dir().join(ALIAS_FILE);

    // Read directly and map NotFound → default (same race-free pattern as
    // `load_config` / `get_current_version`). The previous `exists()` +
    // `read_to_string` was a TOCTOU race: a concurrent `save_aliases` or
    // `uninstall` removing the file between the stat and the open would
    // surface as a confusing "No such file" error instead of the expected
    // "no aliases defined" default.
    let content = match fs::read_to_string(&alias_file) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Aliases::default()),
        Err(e) => return Err(e.into()),
    };
    match serde_json::from_str::<Aliases>(&content) {
        Ok(a) => Ok(a),
        Err(e) => anyhow::bail!(
            "{}: {} ({})",
            alias_file.display(),
            e,
            t_en("config_corrupt_hint")
        ),
    }
}

pub fn save_aliases(aliases: &Aliases) -> Result<()> {
    let alias_file = get_nvm_dir().join(ALIAS_FILE);
    let content = serde_json::to_string_pretty(aliases)?;
    atomic_write(&alias_file, &content)?;
    Ok(())
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

pub fn handle_mirror(mirror: Option<&str>) -> Result<()> {
    let mut config = load_config()?;
    let uri = crate::system::URI;
    let mirror_uri = crate::system::MIRROR_URI;

    match mirror {
        Some("taobao") | Some("npmmirror") => {
            config.mirror = Some(mirror_uri.to_string());
            save_config(&config)?;
            println!(
                "{}",
                format_t("mirror_set", &[mirror_uri.to_string()]).green()
            );
        }
        Some("official") | Some("nodejs") => {
            config.mirror = None;
            save_config(&config)?;
            println!(
                "{}",
                format_t("mirror_official", &[uri.to_string()]).green()
            );
        }
        Some(url) => {
            let normalized = normalize_mirror_url(url)?;
            config.mirror = Some(normalized.clone());
            save_config(&config)?;
            println!("{}", format_t("mirror_set", &[normalized]).green());
        }
        None => match &config.mirror {
            Some(url) => println!(
                "{} {} {}",
                "▶".cyan().bold(),
                T("current_mirror").cyan(),
                url.white().bold()
            ),
            None => println!(
                "{} {} {}",
                "▶".cyan().bold(),
                T("current_mirror").cyan(),
                format!("{} {}", uri, T("official_suffix")).white().bold()
            ),
        },
    }

    Ok(())
}

/// Normalise a user-supplied mirror URL and enforce HTTPS.
///
/// Security: Node.js tarballs are downloaded from this URL and verified only
/// by SHA-256 / GPG afterwards. A plain-HTTP mirror is vulnerable to a
/// network attacker swapping the tarball (and the SHASUMS256.txt fetched
/// from the same mirror) in transit, defeating both checks. We therefore:
///   - reject `http://` outright, and
///   - default a scheme-less URL to `https://` (with a notice) so users who
///     paste `registry.npmmirror.com/-/binary/node/` still get a secure URL.
///
/// Trailing slashes are NOT normalised here — `get_base_url` already joins
/// `{base}{version}/...`, so callers are expected to supply a trailing slash.
fn normalize_mirror_url(url: &str) -> Result<String> {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        anyhow::bail!("{}", T("mirror_url_empty"));
    }
    if trimmed.starts_with("http://") {
        anyhow::bail!(
            "{}",
            format_t("mirror_insecure_http", &[trimmed.to_string()])
        );
    }
    if trimmed.starts_with("https://") {
        return Ok(trimmed.to_string());
    }
    // No scheme: assume HTTPS (the only secure option) and inform the user.
    let upgraded = format!("https://{}", trimmed);
    println!(
        "{}",
        format_t("mirror_https_upgraded", std::slice::from_ref(&upgraded)).yellow()
    );
    Ok(upgraded)
}

fn detect_shell_config() -> Option<String> {
    let home = crate::system::get_home_dir();
    if home == "." {
        return None;
    }
    let home_path = PathBuf::from(&home);

    // On Windows the POSIX rc files (.bashrc/.zshrc/...) don't exist by
    // default — the shell integration target is the PowerShell profile.
    // Probe both PowerShell 7 (`Documents\PowerShell\`) and Windows
    // PowerShell 5.1 (`Documents\WindowsPowerShell\`) and prefer an existing
    // profile so we don't clobber one the user doesn't source. If neither
    // exists, fall back to the PS7 path (created on first write).
    if cfg!(windows) {
        let docs = home_path.join("Documents");
        let candidates = [
            docs.join("PowerShell")
                .join("Microsoft.PowerShell_profile.ps1"),
            docs.join("WindowsPowerShell")
                .join("Microsoft.PowerShell_profile.ps1"),
        ];
        for c in &candidates {
            if c.exists() {
                return Some(c.to_string_lossy().into_owned());
            }
        }
        return Some(candidates[0].to_string_lossy().into_owned());
    }

    // Use PathBuf::join (not `format!("{}/{}", ...)`) so paths stay canonical
    // — mixed `home/foo` separators would still work for `exists()` on Unix
    // but break string comparison against `PathBuf::display()` elsewhere.
    let fish_config = home_path.join(".config").join("fish").join("config.fish");
    let candidates: [PathBuf; 5] = [
        home_path.join(".zshrc"),
        home_path.join(".bashrc"),
        home_path.join(".bash_profile"),
        home_path.join(".profile"),
        fish_config,
    ];
    for c in &candidates {
        if c.exists() {
            return Some(c.to_string_lossy().into_owned());
        }
    }
    Some(home_path.join(".bashrc").to_string_lossy().into_owned())
}

/// Detect the shell type from the config file path.
fn detect_shell_type(config_path: &str) -> &'static str {
    if cfg!(windows) && (config_path.ends_with(".ps1") || config_path.contains("PowerShell")) {
        return "powershell";
    }
    if config_path.contains("config.fish") || config_path.contains("/fish/") {
        "fish"
    } else if config_path.ends_with(".zshrc") {
        "zsh"
    } else {
        "bash"
    }
}

/// Generate the cd hook shell code for the given shell type.
fn cd_hook_code(shell_type: &str) -> String {
    match shell_type {
        "zsh" => r#"
# NVM Rust - use-on-cd
autoload -Uz add-zsh-hook
__nvm_use_on_cd() {
    if [[ "$PWD" != "$__NVM_PREV_DIR" ]]; then
        __NVM_PREV_DIR="$PWD"
        nvm auto --silent 2>/dev/null
    fi
}
add-zsh-hook precmd __nvm_use_on_cd
"#
        .to_string(),
        "fish" => r#"
# NVM Rust - use-on-cd
function __nvm_use_on_cd --on-variable PWD
    nvm auto --silent 2>/dev/null
end
"#
        .to_string(),
        "powershell" => r#"
# NVM Rust - use-on-cd
# Wrap the existing prompt so `nvm auto` runs on directory change, mirroring
# bash's PROMPT_COMMAND. The guard prevents double-wrapping across reloads;
# `nvm unload` removes this block wholesale so the original prompt is restored.
if (-not (Test-Path Function:__NVM_ORIG_PROMPT)) {
    if (Test-Path Function:prompt) {
        Rename-Item Function:prompt __NVM_ORIG_PROMPT
    } else {
        function global:__NVM_ORIG_PROMPT { 'PS> ' }
    }
    function global:prompt {
        if ((-not (Test-Path Variable:__NVM_PREV_DIR)) -or ($PWD.Path -ne $__NVM_PREV_DIR)) {
            $global:__NVM_PREV_DIR = $PWD.Path
            nvm auto --silent 2>$null
        }
        __NVM_ORIG_PROMPT
    }
}
"#
        .to_string(),
        _ => r#"
# NVM Rust - use-on-cd
__nvm_use_on_cd() {
    if [[ "$PWD" != "$__NVM_PREV_DIR" ]]; then
        __NVM_PREV_DIR="$PWD"
        nvm auto --silent 2>/dev/null
    fi
}
PROMPT_COMMAND="__nvm_use_on_cd${PROMPT_COMMAND:+;$PROMPT_COMMAND}"
"#
        .to_string(),
    }
}

/// Remove all nvm-rust-managed lines from a shell config body:
/// the cd-hook blocks (for every shell type, so removal works even if the
/// user switched shells) and the `# NVM Rust` marker / `NVM_HOME=` /
/// `export PATH=...<nvm_dir>...` lines.
///
/// Extracted because both [`update_shell_config`] and
/// [`remove_from_shell_config`] ran the identical strip pass — keeping two
/// copies in sync was error-prone (the line-filter predicate was duplicated
/// verbatim, and a future addition to "what counts as an nvm line" would
/// have to be made in both places).
fn strip_nvm_lines(content: &str, nvm_dir_str: &str) -> String {
    // Remember whether the input ended with a newline so we can re-attach it.
    // `lines()` + `join("\n")` otherwise normalises the trailing newline away,
    // which would make `remove_from_shell_config` needlessly rewrite an
    // already-clean file (dropping its final newline) — breaking idempotency
    // and producing a spurious diff every time the command runs on a clean rc.
    let trailing_newline = content.ends_with('\n');
    let mut content = content.to_string();
    // Remove any previously-written cd hook block as an exact substring.
    // Try all shell types so removal still works if the user switched shells
    // (including bash↔powershell on a dual-boot / WSL-adjacent setup).
    for st in &["bash", "zsh", "fish", "powershell"] {
        let hook = cd_hook_code(st);
        if content.contains(&hook) {
            content = content.replace(&hook, "");
        }
    }
    // Remove marker / NVM_HOME / nvm.rust / PATH-export lines line-by-line.
    // Recognises both POSIX (`export NVM_HOME=`, `export PATH=`) and
    // PowerShell (`$env:NVM_HOME =`, `$env:PATH =`) forms so cleanup works
    // regardless of which shell wrote the lines.
    let mut out: String = content
        .lines()
        .filter(|line| {
            let l = line.trim();
            !(l.contains("NVM_HOME")
                || l.contains("nvm.rust")
                || l.contains(".nvm.rust")
                || l.contains("# NVM Rust")
                || (l.starts_with("export PATH=") && l.contains(nvm_dir_str))
                || (l.starts_with("$env:PATH") && l.contains(nvm_dir_str)))
        })
        .collect::<Vec<&str>>()
        .join("\n");
    // Re-attach the trailing newline only when there's remaining content —
    // an all-nvm file should become empty, not a lone "\n".
    if trailing_newline && !out.is_empty() {
        out.push('\n');
    }
    out
}

pub fn update_shell_config(version: &str, use_on_cd: bool) -> Result<()> {
    let nvm_dir = get_nvm_dir();
    let version_dir = nvm_dir.join(version);
    let bin_dir = crate::system::version_bin_dir(&version_dir);

    let shell_config = match detect_shell_config() {
        Some(p) => p,
        None => return Ok(()),
    };

    let config_path = Path::new(&shell_config);
    // Backup MUST succeed before we touch the user's shell config. The
    // previous `.ok()` silently dropped backup failures, and combined with
    // `read_to_string(...).unwrap_or_default()` below could destroy the
    // file: if both backup and read failed, we'd write a fresh config over
    // an unreadable-but-still-present original, losing the user's existing
    // rc content with no recovery copy.
    backup_file(config_path).context(T("shell_config_backup_failed"))?;

    let shell_type = detect_shell_type(&shell_config);

    // Emit shell-native env setup so the lines actually take effect in the
    // target shell (POSIX `export` vs PowerShell `$env:`). Both forms are
    // recognised by `strip_nvm_lines` for later cleanup.
    let (nvm_export, node_export) = if shell_type == "powershell" {
        (
            format!(r#"$env:NVM_HOME = "{}""#, nvm_dir.display()),
            format!(
                r#"$env:PATH = "{};{};" + $env:PATH"#,
                nvm_dir.join("shims").display(),
                bin_dir.display()
            ),
        )
    } else {
        (
            format!(r#"export NVM_HOME="{}""#, nvm_dir.display()),
            format!(
                r#"export PATH="{}:{}:$PATH""#,
                nvm_dir.join("shims").display(),
                bin_dir.display()
            ),
        )
    };

    // Read the existing config. A missing file is fine (first-time setup,
    // we'll create it), but a present file that fails to read must abort —
    // otherwise we'd overwrite content we couldn't see, with no safe way
    // back. The previous `unwrap_or_default()` collapsed both cases into
    // an empty string and proceeded to overwrite.
    //
    // Read directly and map NotFound → empty string instead of `exists()` +
    // `read_to_string`: the two-step form is a TOCTOU race (another process
    // could remove the rc file between the stat and the open), and a single
    // read is one syscall instead of two.
    let content = match fs::read_to_string(config_path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e).context(T("shell_config_read_failed")),
    };

    let nvm_dir_str = nvm_dir.display().to_string();
    let stripped = strip_nvm_lines(&content, &nvm_dir_str);

    let mut new_config = format!(
        "{}\n# NVM Rust\n{}\n{}\n",
        stripped, nvm_export, node_export
    );

    if use_on_cd {
        new_config.push_str(&cd_hook_code(shell_type));
    }

    // Atomic write (tempfile + rename): a crash mid-write on .bashrc/.zshrc
    // would corrupt the user's shell config. backup_file is a best-effort
    // safety net, but atomic_write prevents the corruption in the first
    // place and keeps this path consistent with config.json/alias.json saves.
    // On Windows the PowerShell profile directory (`Documents\PowerShell\`)
    // may not exist yet on a fresh install; atomic_write's temp file lives in
    // the parent dir, so create it first or the write fails with ENOENT.
    //
    // Propagate the create_dir_all error instead of `.ok()`-ing it: a
    // permission denial or read-only filesystem here would otherwise surface
    // as a confusing "cannot update shell config" from the atomic_write
    // below, hiding the real cause (the parent dir could not be created).
    if let Some(parent) = config_path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .with_context(|| format!("{}: {}", T("cannot_create_dir"), parent.display()))?;
        }
    }
    atomic_write(config_path, &new_config).context(T("cannot_update_shell_config"))?;

    Ok(())
}

pub fn remove_from_shell_config() -> Result<()> {
    let shell_config = match detect_shell_config() {
        Some(p) => p,
        None => return Ok(()),
    };

    let config_path = Path::new(&shell_config);

    let nvm_dir_str = get_nvm_dir().display().to_string();

    // Read directly and map NotFound → "nothing to clean" instead of
    // `exists()` + `read_to_string`: the two-step form is a TOCTOU race
    // (another process could remove the rc file between the stat and the
    // read), and a single read is one syscall instead of two.
    //
    // The previous `if let Ok(...) = read_to_string` silently returned
    // Ok(()) on read failure, masking permission/IO errors as "nothing
    // to remove" — the user's config would remain polluted with stale
    // NVM lines and they'd never know. Surface the read error instead.
    let content = match fs::read_to_string(config_path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e).context(T("shell_config_read_failed")),
    };
    // Backup MUST succeed before we overwrite, same rationale as
    // update_shell_config. At this point the file is known to exist (read
    // succeeded), so backup_file will actually copy it.
    backup_file(config_path).context(T("shell_config_backup_failed"))?;
    let stripped = strip_nvm_lines(&content, &nvm_dir_str);
    atomic_write(config_path, &stripped)?;
    println!("{}", crate::i18n::T("shell_config_removed").green());
    Ok(())
}

/// Rewrite the shell rc file from version-specific PATH format
/// (e.g. `export PATH="shims:v24.18.0/bin:$PATH"`) to the fixed
/// `active/bin` format (`export PATH="shims:active/bin:$PATH"`).
/// This is the core of the Full Shim mode migration.
pub fn migrate_rc_to_shim_mode() -> Result<()> {
    let shell_config = match detect_shell_config() {
        Some(p) => p,
        None => return Ok(()), // No rc file, nothing to migrate
    };

    let config_path = Path::new(&shell_config);
    let nvm_dir = get_nvm_dir();
    let nvm_dir_str = nvm_dir.display().to_string();

    let content = match fs::read_to_string(config_path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e).context(T("shell_config_read_failed")),
    };

    backup_file(config_path).context(T("shell_config_backup_failed"))?;

    let stripped = strip_nvm_lines(&content, &nvm_dir_str);

    let shims = nvm_dir.join("shims").display().to_string();
    let active_bin = nvm_dir.join("active").join("bin").display().to_string();

    let shell_type = detect_shell_type(&shell_config);
    let (nvm_export, path_export) = if shell_type == "powershell" {
        (
            format!(r#"$env:NVM_HOME = "{}""#, nvm_dir_str),
            format!(r#"$env:PATH = "{};{};" + $env:PATH"#, shims, active_bin),
        )
    } else {
        (
            format!(r#"export NVM_HOME="{}""#, nvm_dir_str),
            format!(r#"export PATH="{}:{}:$PATH""#, shims, active_bin),
        )
    };

    let new_config = format!(
        "{}\n# NVM Rust\n{}\n{}\n",
        stripped, nvm_export, path_export
    );

    crate::utils::atomic_write(config_path, &new_config)?;
    Ok(())
}

/// Check if the rc file contains version-specific PATH (old format)
/// rather than the fixed `active/bin` (new/shim format).
/// Returns true if migration is needed.
pub fn rc_has_version_specific_path() -> Result<bool> {
    let shell_config = match detect_shell_config() {
        Some(p) => p,
        None => return Ok(false),
    };

    let content = fs::read_to_string(&shell_config).unwrap_or_default();
    let has_nvm_path = content.contains("shims") && content.contains("bin");
    let has_active = content.contains("active");
    Ok(has_nvm_path && !has_active)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_default() {
        let config = Config::default();
        assert!(config.mirror.is_none());
        assert!(config.default_version.is_none());
        assert!(config.language.is_none());
        assert!(config.proxy.is_none());
        assert!(config.use_on_cd.is_none());
    }

    #[test]
    fn test_config_serialization() {
        let config = Config {
            mirror: Some("https://example.com".to_string()),
            default_version: Some("v20.0.0".to_string()),
            language: Some("cn".to_string()),
            proxy: Some(true),
            use_on_cd: Some(true),
        };

        let json = serde_json::to_string(&config).unwrap();
        let deserialized: Config = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.mirror, Some("https://example.com".to_string()));
        assert_eq!(deserialized.default_version, Some("v20.0.0".to_string()));
        assert_eq!(deserialized.language, Some("cn".to_string()));
        assert_eq!(deserialized.proxy, Some(true));
        assert_eq!(deserialized.use_on_cd, Some(true));
    }

    #[test]
    fn test_aliases_default() {
        let aliases = Aliases::default();
        assert!(aliases.aliases.is_empty());
    }

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
    fn test_detect_shell_config() {
        // Should return Some path even in test environment
        let result = detect_shell_config();
        assert!(result.is_some());
        // Should be a valid path string
        let path = result.unwrap();
        assert!(!path.is_empty());
    }

    #[test]
    fn test_strip_nvm_lines_removes_marker_and_exports() {
        let nvm_dir = "/home/u/.nvm.rust";
        let input = format!(
            "# my alias\n\
             alias ll='ls -l'\n\
             # NVM Rust\n\
             export NVM_HOME=\"{nvm_dir}\"\n\
             export PATH=\"/home/u/.nvm.rust/v20.0.0/bin:$PATH\"\n\
             export EDITOR=vim\n"
        );
        let out = strip_nvm_lines(&input, nvm_dir);
        // User lines survive.
        assert!(out.contains("alias ll='ls -l'"));
        assert!(out.contains("export EDITOR=vim"));
        // nvm lines are gone.
        assert!(!out.contains("NVM_HOME="));
        assert!(!out.contains("# NVM Rust"));
        assert!(!out.contains("nvm.rust"));
    }

    #[test]
    fn test_strip_nvm_lines_removes_cd_hook_all_shells() {
        let nvm_dir = "/home/u/.nvm.rust";
        // Each shell's hook block carries a distinctive marker we assert is
        // gone after stripping — `__nvm_use_on_cd` for POSIX/fish, and the
        // PowerShell prompt-wrapper symbol for powershell.
        for (st, marker) in &[
            ("bash", "__nvm_use_on_cd"),
            ("zsh", "__nvm_use_on_cd"),
            ("fish", "__nvm_use_on_cd"),
            ("powershell", "__NVM_ORIG_PROMPT"),
        ] {
            let hook = cd_hook_code(st);
            let input = format!("alias x='y'\n{hook}\nexport FOO=bar\n");
            let out = strip_nvm_lines(&input, nvm_dir);
            assert!(
                !out.contains(marker),
                "cd hook for {st} was not stripped: {out}"
            );
            assert!(out.contains("alias x='y'"));
            assert!(out.contains("export FOO=bar"));
        }
    }

    #[test]
    fn test_strip_nvm_lines_removes_powershell_env_exports() {
        // PowerShell-formatted env lines must be stripped just like POSIX
        // `export` lines, so `nvm unload` cleans up after a Windows install.
        let nvm_dir = "/home/u/.nvm.rust";
        let input = format!(
            "alias x='y'\n\
             # NVM Rust\n\
             $env:NVM_HOME = \"{nvm_dir}\"\n\
             $env:PATH = \"{nvm_dir}/v20.0.0/bin;\" + $env:PATH\n\
             export EDITOR=vim\n"
        );
        let out = strip_nvm_lines(&input, nvm_dir);
        assert!(out.contains("alias x='y'"));
        assert!(out.contains("export EDITOR=vim"));
        assert!(
            !out.contains("NVM_HOME"),
            "PowerShell NVM_HOME line kept: {out}"
        );
        assert!(
            !out.contains("$env:PATH"),
            "PowerShell PATH line kept: {out}"
        );
        assert!(!out.contains("# NVM Rust"));
    }

    #[test]
    fn test_strip_nvm_lines_preserves_unrelated_path_exports() {
        // An export PATH line that does NOT reference the nvm dir must be
        // kept — the filter must not be overzealous and drop user PATH setup.
        let nvm_dir = "/home/u/.nvm.rust";
        let input = "export PATH=/usr/local/bin:$PATH\nalias ll='ls -l'\n";
        let out = strip_nvm_lines(input, nvm_dir);
        assert!(out.contains("export PATH=/usr/local/bin:$PATH"));
    }

    #[test]
    fn test_strip_nvm_lines_idempotent() {
        // Stripping an already-clean body is a no-op (modulo trailing newline
        // joining), so re-running remove_from_shell_config is safe.
        let nvm_dir = "/home/u/.nvm.rust";
        let clean = "alias ll='ls -l'\nexport EDITOR=vim\n";
        let out = strip_nvm_lines(clean, nvm_dir);
        assert_eq!(out, clean);
    }

    #[test]
    fn test_normalize_mirror_url_accepts_https() {
        assert_eq!(
            normalize_mirror_url("https://example.com/node/").unwrap(),
            "https://example.com/node/"
        );
    }

    #[test]
    fn test_normalize_mirror_url_rejects_http() {
        // HTTP must be rejected outright to prevent MITM on tarball downloads.
        let err = normalize_mirror_url("http://example.com/node/").unwrap_err();
        assert!(format!("{err}").contains("HTTPS"));
    }

    #[test]
    fn test_normalize_mirror_url_rejects_empty() {
        let err = normalize_mirror_url("   ").unwrap_err();
        assert!(format!("{err}").contains("empty"));
    }

    #[test]
    fn test_normalize_mirror_url_upgrades_schemeless_to_https() {
        // A scheme-less URL is upgraded to https:// so users can paste a bare host.
        assert_eq!(
            normalize_mirror_url("registry.npmmirror.com/-/binary/node/").unwrap(),
            "https://registry.npmmirror.com/-/binary/node/"
        );
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

    // Tests that mutate NVM_DIR (and HOME) acquire the process-global
    // `ENV_TESTS_MUTEX`, serializing them against NVM_DIR-mutating tests in
    // download.rs / proxy.rs / utils.rs. The previous per-module
    // `SHELL_CFG_TESTS_MUTEX` only serialized within config.rs.
    use crate::system::ENV_TESTS_MUTEX;

    #[test]
    fn update_shell_config_surfaces_create_dir_all_failure() {
        let _guard = ENV_TESTS_MUTEX.lock().expect("ENV_TESTS_MUTEX poisoned");

        // Set up a HOME whose `.bashrc` parent cannot be created: place a
        // regular file where the parent directory would go. `create_dir_all`
        // then fails with NotADirectory (ENOTDIR), which must propagate
        // instead of being swallowed by the old `.ok()`.
        let tmp = tempfile::TempDir::new().expect("tempdir for HOME blocker");
        let blocker = tmp.path().join("blocker_file");
        std::fs::write(&blocker, b"not a directory").expect("write blocker file");

        let nvm_tmp = tempfile::TempDir::new().expect("tempdir for NVM_DIR");

        let old_home = std::env::var_os("HOME");
        let old_nvm_dir = std::env::var_os("NVM_DIR");
        std::env::set_var("HOME", &blocker);
        std::env::set_var("NVM_DIR", nvm_tmp.path());

        // Restore env even if assertions fail — leaking HOME=<blocker> would
        // break every subsequent test that touches the shell config.
        struct EnvGuard {
            old_home: Option<std::ffi::OsString>,
            old_nvm_dir: Option<std::ffi::OsString>,
        }
        impl Drop for EnvGuard {
            fn drop(&mut self) {
                match &self.old_home {
                    Some(v) => std::env::set_var("HOME", v),
                    None => std::env::remove_var("HOME"),
                }
                match &self.old_nvm_dir {
                    Some(v) => std::env::set_var("NVM_DIR", v),
                    None => std::env::remove_var("NVM_DIR"),
                }
            }
        }
        let _guard_env = EnvGuard {
            old_home,
            old_nvm_dir,
        };

        let result = update_shell_config("v20.0.0", false);

        let err = result
            .expect_err("update_shell_config should fail when the parent dir cannot be created");
        let msg = format!("{err:#}");
        // The error must mention directory creation (the real cause) rather
        // than the downstream "cannot update shell config" that the old
        // `.ok()` path would have produced instead.
        assert!(
            msg.to_lowercase().contains("directory") || msg.to_lowercase().contains("create"),
            "expected create_dir_all error context, got: {msg}"
        );
    }
}
