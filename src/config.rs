use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;

use crate::i18n::t_en;
use crate::system::{get_nvm_dir, ALIAS_FILE, CONFIG_FILE};
use crate::utils::atomic_write;

// Re-export the extracted modules so existing `crate::config::*` call sites
// (resolve_alias, handle_mirror, detect_shell_config, etc.) keep working
// without touching ~30 files.
pub use crate::alias::*;
pub use crate::mirror::*;
pub use crate::shell_config::*;

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
}
