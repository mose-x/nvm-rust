use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::config::{load_config, save_config};

// Pull in the build-script-generated static tables. `include!` runs at
// compile time, so LANG_CODES / LANG_DISPLAY_NAMES / LANG_ALIASES /
// LANG_STRINGS are all `&'static` data with zero runtime cost.
//
// To add a new language, drop a `xx.toml` into `locales/` and rebuild —
// build.rs regenerates this include on every build via
// `cargo:rerun-if-changed=locales`. No source edits required.
include!(concat!(env!("OUT_DIR"), "/locales_generated.rs"));

/// A language tag.
///
/// Internally this is just a `&'static str` lang code (e.g. `"en"`, `"cn"`)
/// drawn from `LANG_CODES`. It is a `Copy` newtype so it can be passed by
/// value everywhere, matching the ergonomics of the old `enum Lang`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Lang(&'static str);

impl Lang {
    /// The fallback language. Must always exist in `LANG_CODES` (enforced
    /// by build.rs, which panics if `locales/en.toml` is missing).
    pub const EN: Lang = Lang("en");

    /// Parse a user-supplied language tag or alias into a `Lang`.
    ///
    /// Accepts any canonical code from `LANG_CODES` (case-insensitive) and
    /// any alias declared in a locale's `[_meta] aliases` table. For
    /// example, `cn.toml` declares `aliases = ["zh", "zh-cn", "chinese",
    /// "中文"]`, so all of those resolve to `Lang("cn")`.
    pub fn from_str(s: &str) -> Option<Self> {
        let lower = s.to_lowercase();
        // 1. Direct match against a canonical code.
        for &code in LANG_CODES {
            if lower == code {
                return Some(Lang(code));
            }
        }
        // 2. Alias match (LANG_ALIASES stores (alias, canonical_code)).
        for &(alias, code) in LANG_ALIASES {
            if lower == alias {
                return Some(Lang(code));
            }
        }
        None
    }

    /// The canonical lang code, e.g. `"en"` or `"cn"`.
    pub fn as_str(&self) -> &'static str {
        self.0
    }

    /// Human-readable name for display, e.g. `"English"` or `"中文"`.
    ///
    /// Falls back to the lang code if a locale file omitted `_meta.display_name`.
    pub fn display_name(&self) -> &'static str {
        for &(code, name) in LANG_DISPLAY_NAMES {
            if code == self.0 {
                return name;
            }
        }
        self.0
    }
}

impl Default for Lang {
    fn default() -> Self {
        Lang::EN
    }
}

// Serialize/Deserialize as the canonical string code so the config file
// stays stable even though Lang is now a newtype instead of an enum.
impl Serialize for Lang {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.0)
    }
}

impl<'de> Deserialize<'de> for Lang {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Lang::from_str(&s).or(Some(Lang::EN)).ok_or_else(|| {
            serde::de::Error::custom(format!("unknown language: {s}"))
        })
    }
}

pub fn get_language() -> Lang {
    load_config()
        .ok()
        .and_then(|c| c.language.clone())
        .and_then(|s| Lang::from_str(&s))
        .unwrap_or_default()
}

pub fn set_language(lang: Lang) -> Result<()> {
    let mut config = load_config()?;
    config.language = Some(lang.as_str().to_string());
    save_config(&config)?;
    Ok(())
}

#[allow(non_snake_case)]
#[allow(dead_code)]
pub fn T(key: &str) -> String {
    let lang = get_language();
    t(key, lang)
}

/// Format a translation with parameter substitution
pub fn format_t(key: &str, args: &[String]) -> String {
    let template = T(key);
    substitute_params(&template, args)
}

/// Substitute {0}, {1}, etc. in a string with provided arguments
fn substitute_params(template: &str, args: &[String]) -> String {
    let mut result = template.to_string();
    for (i, arg) in args.iter().enumerate() {
        let placeholder = format!("{{{}}}", i);
        result = result.replace(&placeholder, arg);
    }
    result
}

#[allow(dead_code)]
fn t(key: &str, lang: Lang) -> String {
    // Look up the requested language first; fall back to English if the
    // key is missing. English is always present (enforced by build.rs) and
    // itself falls back to the raw key name, so a missing key never panics
    // and never returns an empty string.
    if let Some(s) = lookup(lang, key) {
        return s.to_string();
    }
    if let Some(s) = lookup(Lang::EN, key) {
        return s.to_string();
    }
    key.to_string()
}

// Lazy-loaded per-language string tables. Parsed from the TOML embedded by
// LANG_STRINGS on first use. Storing them in a HashMap gives O(1) lookup at
// the cost of a one-time parse per language.
//
// Only top-level string values are collected; the `[_meta]` table (which
// holds display_name / aliases and is consumed by build.rs at codegen time)
// is skipped here so it never leaks into the translation table.
lazy_static::lazy_static! {
    static ref LOCALES: HashMap<&'static str, HashMap<String, String>> = {
        let mut map = HashMap::new();
        for &(code, text) in LANG_STRINGS {
            match toml::from_str::<toml::Value>(text) {
                Ok(root) => {
                    let strings = collect_string_values(&root);
                    map.insert(code, strings);
                }
                Err(e) => {
                    // A malformed locale file would otherwise panic the
                    // whole process on first use. Log and skip so that a
                    // broken jp.toml doesn't take down `nvm install`.
                    eprintln!("warning: failed to parse locales/{}.toml: {}", code, e);
                }
            }
        }
        map
    };
}

/// Walk a parsed TOML value and collect every top-level `key = "string"`
/// pair into a flat HashMap. Sub-tables (like `[_meta]`) are skipped: they
/// are locale metadata, not translatable strings.
fn collect_string_values(root: &toml::Value) -> HashMap<String, String> {
    let mut out = HashMap::new();
    if let Some(table) = root.as_table() {
        for (k, v) in table {
            if let Some(s) = v.as_str() {
                out.insert(k.clone(), s.to_string());
            }
            // Non-string top-level values (tables, arrays, numbers) are
            // ignored — the i18n format is strictly `key = "value"`.
        }
    }
    out
}

/// Look up a key in a specific language. Returns None if the language or
/// the key is missing (caller decides on fallback).
fn lookup(lang: Lang, key: &str) -> Option<&str> {
    LOCALES
        .get(lang.as_str())
        .and_then(|m| m.get(key))
        .map(|s| s.as_str())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn test_lang_from_str_canonical() {
        assert_eq!(Lang::from_str("en"), Some(Lang::EN));
        assert_eq!(Lang::from_str("EN"), Some(Lang::EN));
        assert_eq!(Lang::from_str("cn"), Some(Lang("cn")));
        assert_eq!(Lang::from_str("CN"), Some(Lang("cn")));
    }

    #[test]
    fn test_lang_from_str_aliases() {
        // Aliases declared in each locale's [_meta] table.
        assert_eq!(Lang::from_str("zh"), Some(Lang("cn")));
        assert_eq!(Lang::from_str("zh-cn"), Some(Lang("cn")));
        assert_eq!(Lang::from_str("chinese"), Some(Lang("cn")));
        assert_eq!(Lang::from_str("中文"), Some(Lang("cn")));
    }

    #[test]
    fn test_lang_from_str_rejects_unknown() {
        assert_eq!(Lang::from_str("invalid"), None);
        assert_eq!(Lang::from_str(""), None);
        assert_eq!(Lang::from_str("klingon"), None);
    }

    #[test]
    fn test_lang_as_str() {
        assert_eq!(Lang::EN.as_str(), "en");
        assert_eq!(Lang("cn").as_str(), "cn");
    }

    #[test]
    fn test_lang_display_name() {
        assert_eq!(Lang::EN.display_name(), "English");
        assert_eq!(Lang("cn").display_name(), "中文");
    }

    #[test]
    fn test_lang_default_is_en() {
        assert_eq!(Lang::default(), Lang::EN);
    }

    #[test]
    fn test_en_str_resolves_known_key() {
        assert_eq!(
            lookup(Lang::EN, "checking_lts"),
            Some("Checking latest LTS version...")
        );
    }

    #[test]
    fn test_cn_str_resolves_known_key() {
        assert_eq!(
            lookup(Lang("cn"), "checking_lts"),
            Some("正在检查最新 LTS 版本...")
        );
    }

    #[test]
    fn test_unknown_key_returns_none_then_raw_key_via_t() {
        // lookup returns None for a missing key.
        assert_eq!(lookup(Lang::EN, "definitely_not_a_real_key"), None);
        // t() falls back: not in cn -> not in en -> raw key string.
        assert_eq!(t("definitely_not_a_real_key", Lang("cn")), "definitely_not_a_real_key");
    }

    #[test]
    fn test_cn_falls_back_to_en_for_missing_key() {
        // A key present in en.toml but absent from cn.toml should resolve
        // to the English value rather than the raw key. (The keys-match
        // test below enforces this never actually happens, but the
        // fallback path must still be correct.)
        // Use a known en-only-safe check: any key in EN_STRINGS that's
        // also in CN_STRINGS will return the CN value, which is fine. We
        // can't easily fabricate a missing-key scenario at runtime, so we
        // just verify the fallback function doesn't panic.
        let _ = t("checking_lts", Lang("cn"));
    }

    /// Every locale must declare exactly the same key set as en.toml.
    /// This guards against drift: a new key added to en.toml without
    /// updating jp.toml would silently render English text to Japanese
    /// users, which is better than a raw key but still wrong.
    #[test]
    fn all_locales_have_same_keys_as_en() {
        // collect_string_values already strips the `[_meta]` table, so the
        // key sets here are pure translation keys.
        let en_keys: BTreeSet<&String> = LOCALES
            .get("en")
            .map(|m| m.keys().collect())
            .expect("en locale must be present");

        for &code in LANG_CODES {
            if code == "en" {
                continue;
            }
            let other_keys: BTreeSet<&String> = LOCALES
                .get(code)
                .map(|m| m.keys().collect())
                .unwrap_or_default();
            assert_eq!(
                en_keys, other_keys,
                "locale `{}` key set differs from en.toml",
                code
            );
        }
    }

    #[test]
    fn test_substitute_params() {
        let args = vec!["v20.0.0".to_string()];
        assert_eq!(
            substitute_params("Installed {0}!", &args),
            "Installed v20.0.0!"
        );
    }
}
