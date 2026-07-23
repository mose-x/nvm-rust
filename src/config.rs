use anyhow::{Context, Result};
use colored::Colorize;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::Command;

use crate::i18n::{format_t, T};
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

pub fn named_lts_aliases() -> BTreeMap<&'static str, &'static str> {
    let mut m = BTreeMap::new();
    m.insert("lts/argon", "v4");
    m.insert("lts/boron", "v6");
    m.insert("lts/carbon", "v8");
    m.insert("lts/dubnium", "v10");
    m.insert("lts/erbium", "v12");
    m.insert("lts/fermium", "v14");
    m.insert("lts/gallium", "v16");
    m.insert("lts/hydrogen", "v18");
    m.insert("lts/iron", "v20");
    m.insert("lts/jod", "v22");
    m.insert("lts/krypton", "v24");
    m
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
    let mut m: BTreeMap<String, String> = named_lts_aliases()
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let remote = crate::system::fetch_lts_codename_map(base_url);
    for (codename, major) in remote {
        let alias = format!("lts/{}", codename);
        m.insert(alias, format!("v{}", major));
    }
    m
}

pub fn load_config() -> Result<Config> {
    let config_file = get_nvm_dir().join(CONFIG_FILE);

    if config_file.exists() {
        let content = fs::read_to_string(&config_file)?;
        // Surface parse errors instead of silently dropping all settings.
        // Returning default on a corrupt file would cause the next
        // save_config to overwrite it with an empty config, permanently
        // losing the user's mirror/aliases/language.
        match serde_json::from_str::<Config>(&content) {
            Ok(c) => Ok(c),
            Err(e) => anyhow::bail!(
                "{}: {} ({})",
                config_file.display(),
                e,
                T("config_corrupt_hint")
            ),
        }
    } else {
        Ok(Config::default())
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

    if alias_file.exists() {
        let content = fs::read_to_string(&alias_file)?;
        match serde_json::from_str::<Aliases>(&content) {
            Ok(a) => Ok(a),
            Err(e) => anyhow::bail!(
                "{}: {} ({})",
                alias_file.display(),
                e,
                T("config_corrupt_hint")
            ),
        }
    } else {
        Ok(Aliases::default())
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
    let mut aliases = load_aliases()?;

    match version {
        Some(v) => {
            let resolved = resolve_alias(v)?;
            let version_dir = get_nvm_dir().join(&resolved);
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

    for (name, prefix) in named_lts_aliases() {
        let mut installed: Vec<String> = Vec::new();
        if let Ok(rd) = fs::read_dir(&nvm_dir) {
            for entry in rd.flatten() {
                if let Some(s) = entry.file_name().to_str() {
                    // Strict match: must be a valid version directory AND its
                    // major must equal the alias's target major. Without the
                    // major check, `lts/argon` (prefix "v4") would also match
                    // "v40.0.0" because "v40.0.0".starts_with("v4") is true.
                    if crate::utils::is_version_dir_name(s) {
                        if let Some(major) = crate::utils::parse_major(s) {
                            let prefix_major = prefix.trim_start_matches('v');
                            if prefix_major == major.to_string() {
                                installed.push(s.to_string());
                            }
                        }
                    }
                }
            }
        }
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
                return Ok(v.clone());
            }
        }
        let config = load_config()?;
        if let Some(v) = config.default_version {
            return Ok(v);
        }
        anyhow::bail!("{}", T("no_default_version"));
    }

    // "current" resolves to whatever version is active right now (the
    // contents of the `current` file). Enables `nvm which current`,
    // `nvm use current`, `nvm exec current ...`, etc.
    if name == "current" {
        let current_file = get_nvm_dir().join("current");
        if current_file.exists() {
            if let Ok(content) = fs::read_to_string(&current_file) {
                let v = content.trim();
                if !v.is_empty() {
                    return Ok(v.to_string());
                }
            }
        }
        anyhow::bail!("{}", T("no_current_version_set"));
    }

    if name == "system" {
        if crate::utils::find_system_node_path().is_some() {
            if let Ok(v) = Command::new("node").arg("--version").output() {
                let v = String::from_utf8_lossy(&v.stdout).trim().to_string();
                if !v.is_empty() {
                    return Ok(format!("system:{}", v));
                }
            }
        }
        anyhow::bail!("{}", T("system_node_not_found"));
    }

    if name.starts_with("lts/") {
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
        return Ok(v.clone());
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

fn find_latest_installed(prefix: &str) -> Result<String> {
    let nvm_dir = get_nvm_dir();
    let mut versions: Vec<String> = Vec::new();
    if let Ok(rd) = fs::read_dir(&nvm_dir) {
        for entry in rd.flatten() {
            if let Some(s) = entry.file_name().to_str() {
                // Only consider real version directories — `versions` (nvm-sh's
                // nested dir) starts with "v" but isn't a version, and would
                // otherwise pollute `use lts` / `use node` lookups.
                if !s.starts_with(prefix) || !crate::utils::is_version_dir_name(s) {
                    continue;
                }
                // Strict major match when prefix is `vN` (no dot). Without
                // this, `lts/hydrogen` (prefix "v18") would also match a
                // hypothetical "v180.0.0" install. The `v22.` form returned
                // by `bare_major_prefix` already encodes the dot so the
                // starts_with check above is sufficient there; this branch
                // only adds the major equality for the bare `vN` aliases.
                if !prefix.contains('.') && prefix.len() > 1 {
                    let prefix_major = prefix.trim_start_matches('v');
                    if let Some(major) = crate::utils::parse_major(s) {
                        if prefix_major != major.to_string() {
                            continue;
                        }
                    } else {
                        continue;
                    }
                }
                versions.push(s.to_string());
            }
        }
    }
    if versions.is_empty() {
        anyhow::bail!("{}", format_t("no_matching_version", &[prefix.to_string()]));
    }
    // Sort semantically (numeric major.minor.patch), not alphabetically:
    // alphabetical sort would put `v20.5.0` after `v20.20.2` ('5' > '2'),
    // returning the older version as "latest".
    versions.sort_by(|a, b| crate::utils::compare_semver(a, b));
    Ok(versions.last().unwrap().clone())
}

fn find_latest_installed_lts() -> Result<String> {
    let nvm_dir = get_nvm_dir();
    let mut versions: Vec<String> = Vec::new();
    if let Ok(rd) = fs::read_dir(&nvm_dir) {
        for entry in rd.flatten() {
            if let Some(s) = entry.file_name().to_str() {
                if crate::utils::is_version_dir_name(s) && crate::utils::is_lts_version(s) {
                    versions.push(s.to_string());
                }
            }
        }
    }
    if versions.is_empty() {
        anyhow::bail!("{}", T("no_installed_lts"));
    }
    versions.sort_by(|a, b| crate::utils::compare_semver(a, b));
    Ok(versions.last().unwrap().clone())
}

fn find_latest_unstable() -> Result<String> {
    let tags = get_tags(URI.to_string());
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
            let trimmed = url.trim();
            if trimmed.is_empty() {
                anyhow::bail!("{}", T("mirror_url_empty"));
            }
            config.mirror = Some(trimmed.to_string());
            save_config(&config)?;
            println!("{}", format_t("mirror_set", &[trimmed.to_string()]).green());
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

fn detect_shell_config() -> Option<String> {
    let home = crate::system::get_home_dir();
    if home == "." {
        return None;
    }
    // Fish config lives in a different location
    let fish_config = format!("{}/.config/fish/config.fish", home);
    let candidates = vec![
        ".zshrc",
        ".bashrc",
        ".bash_profile",
        ".profile",
        &fish_config,
    ];
    for c in candidates {
        let p = if c.starts_with('/') {
            c.to_string()
        } else {
            format!("{}/{}", home, c)
        };
        if Path::new(&p).exists() {
            return Some(p);
        }
    }
    Some(format!("{}/.bashrc", home))
}

/// Detect the shell type from the config file path.
fn detect_shell_type(config_path: &str) -> &'static str {
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

pub fn update_shell_config(version: &str, use_on_cd: bool) -> Result<()> {
    let nvm_dir = get_nvm_dir();
    let version_dir = nvm_dir.join(version);
    let bin_dir = version_dir.join("bin");

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

    let nvm_export = format!(r#"export NVM_HOME="{}""#, nvm_dir.display());
    let node_export = format!(r#"export PATH="{}:$PATH""#, bin_dir.display());

    // Read the existing config. A missing file is fine (first-time setup,
    // we'll create it), but a present file that fails to read must abort —
    // otherwise we'd overwrite content we couldn't see, with no safe way
    // back. The previous `unwrap_or_default()` collapsed both cases into
    // an empty string and proceeded to overwrite.
    let mut content = if config_path.exists() {
        fs::read_to_string(config_path).context(T("shell_config_read_failed"))?
    } else {
        String::new()
    };

    // Remove any previously-written cd hook block as an exact substring.
    // We try all shell types so removal still works if the user switched shells.
    for st in &["bash", "zsh", "fish"] {
        let hook = cd_hook_code(st);
        if content.contains(&hook) {
            content = content.replace(&hook, "");
        }
    }

    // Remove old PATH export lines and markers line-by-line.
    let nvm_dir_str = nvm_dir.display().to_string();
    let lines: Vec<&str> = content
        .lines()
        .filter(|line| {
            let l = line.trim();
            !(l.contains("NVM_HOME=")
                || l.contains("nvm.rust")
                || l.contains(".nvm.rust")
                || l.contains("# NVM Rust")
                || (l.starts_with("export PATH=") && l.contains(&nvm_dir_str)))
        })
        .collect();

    let mut new_config = format!(
        "{}\n# NVM Rust\n{}\n{}\n",
        lines.join("\n"),
        nvm_export,
        node_export
    );

    if use_on_cd {
        new_config.push_str(&cd_hook_code(shell_type));
    }

    // Atomic write (tempfile + rename): a crash mid-write on .bashrc/.zshrc
    // would corrupt the user's shell config. backup_file is a best-effort
    // safety net, but atomic_write prevents the corruption in the first
    // place and keeps this path consistent with config.json/alias.json saves.
    atomic_write(config_path, &new_config).context(T("cannot_update_shell_config"))?;

    Ok(())
}

pub fn remove_from_shell_config() -> Result<()> {
    let shell_config = match detect_shell_config() {
        Some(p) => p,
        None => return Ok(()),
    };

    let config_path = Path::new(&shell_config);
    // Nothing to clean if the rc file isn't there. But if it IS there,
    // backup must succeed before we overwrite — same rationale as
    // update_shell_config.
    if !config_path.exists() {
        return Ok(());
    }
    backup_file(config_path).context(T("shell_config_backup_failed"))?;

    let nvm_dir_str = get_nvm_dir().display().to_string();

    // The previous `if let Ok(...) = read_to_string` silently returned
    // Ok(()) on read failure, masking permission/IO errors as "nothing
    // to remove" — the user's config would remain polluted with stale
    // NVM lines and they'd never know. Surface the read error instead.
    let mut content = fs::read_to_string(config_path).context(T("shell_config_read_failed"))?;

    // Remove any cd hook block (any shell type) as an exact substring.
    for st in &["bash", "zsh", "fish"] {
        let hook = cd_hook_code(st);
        if content.contains(&hook) {
            content = content.replace(&hook, "");
        }
    }
    // Remove PATH export lines and markers line-by-line.
    let lines: Vec<&str> = content
        .lines()
        .filter(|line| {
            let l = line.trim();
            !(l.contains("NVM_HOME=")
                || l.contains("nvm.rust")
                || l.contains(".nvm.rust")
                || l.contains("# NVM Rust")
                || (l.starts_with("export PATH=") && l.contains(&nvm_dir_str)))
        })
        .collect();
    atomic_write(config_path, &lines.join("\n"))?;
    println!("{}", crate::i18n::T("shell_config_removed").green());
    Ok(())
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
        assert_eq!(aliases.get("lts/argon"), Some(&"v4"));
        assert_eq!(aliases.get("lts/iron"), Some(&"v20"));
        assert_eq!(aliases.get("lts/jod"), Some(&"v22"));
        assert_eq!(aliases.get("lts/krypton"), Some(&"v24"));
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
}
