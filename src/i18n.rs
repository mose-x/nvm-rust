use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::config::load_config;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    #[default]
    En,
    Cn,
}

impl Lang {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "en" | "english" => Some(Lang::En),
            "cn" | "zh" | "zh-cn" | "chinese" | "中文" => Some(Lang::Cn),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Cn => "cn",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Lang::En => "English",
            Lang::Cn => "中文",
        }
    }
}

pub fn get_language() -> Lang {
    load_config()
        .ok()
        .and_then(|c| c.language.clone())
        .and_then(|s| Lang::from_str(&s))
        .unwrap_or(Lang::En)
}

pub fn set_language(lang: Lang) -> anyhow::Result<()> {
    let mut config = load_config()?;
    config.language = Some(lang.as_str().to_string());
    crate::config::save_config(&config)?;
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
    match lang {
        Lang::En => en_str(key).to_string(),
        Lang::Cn => cn_str(key).to_string(),
    }
}

#[allow(dead_code)]
fn en_str(key: &str) -> &str {
    EN_STRINGS.get(key).map(|s| s.as_str()).unwrap_or(key)
}

#[allow(dead_code)]
fn cn_str(key: &str) -> &str {
    CN_STRINGS.get(key).map(|s| s.as_str()).unwrap_or_else(|| en_str(key))
}

// ---------------------------------------------------------------------------
// String tables — loaded from ../locales/{en,cn}.toml at compile time.
// The TOML files are embedded into the binary via `include_str!`, so the
// single-binary distribution model is preserved (no external locale files
// needed at runtime). To add a new language, drop a `xx.toml` into
// `locales/`, wire it up below, and add a `Lang` variant + `t()` arm.
// ---------------------------------------------------------------------------

lazy_static::lazy_static! {
    static ref EN_STRINGS: HashMap<String, String> = {
        let text = include_str!("../locales/en.toml");
        toml::from_str(text).expect("failed to parse locales/en.toml")
    };

    static ref CN_STRINGS: HashMap<String, String> = {
        let text = include_str!("../locales/cn.toml");
        toml::from_str(text).expect("failed to parse locales/cn.toml")
    };
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn test_lang_from_str() {
        assert_eq!(Lang::from_str("en"), Some(Lang::En));
        assert_eq!(Lang::from_str("EN"), Some(Lang::En));
        assert_eq!(Lang::from_str("english"), Some(Lang::En));
        assert_eq!(Lang::from_str("cn"), Some(Lang::Cn));
        assert_eq!(Lang::from_str("CN"), Some(Lang::Cn));
        assert_eq!(Lang::from_str("zh"), Some(Lang::Cn));
        assert_eq!(Lang::from_str("zh-cn"), Some(Lang::Cn));
        assert_eq!(Lang::from_str("chinese"), Some(Lang::Cn));
        assert_eq!(Lang::from_str("中文"), Some(Lang::Cn));
        assert_eq!(Lang::from_str("invalid"), None);
        assert_eq!(Lang::from_str(""), None);
    }

    #[test]
    fn test_lang_as_str() {
        assert_eq!(Lang::En.as_str(), "en");
        assert_eq!(Lang::Cn.as_str(), "cn");
    }

    #[test]
    fn test_lang_display_name() {
        assert_eq!(Lang::En.display_name(), "English");
        assert_eq!(Lang::Cn.display_name(), "中文");
    }

    #[test]
    fn test_lang_default() {
        assert_eq!(Lang::default(), Lang::En);
    }

    #[test]
    fn locales_en_cn_keys_match() {
        // Guards against drift: every key in en.toml must exist in cn.toml
        // and vice versa. Replaces the compile-time guarantee we had when
        // strings were inline Rust.
        let en_keys: BTreeSet<&String> = EN_STRINGS.keys().collect();
        let cn_keys: BTreeSet<&String> = CN_STRINGS.keys().collect();
        assert_eq!(en_keys, cn_keys, "en.toml and cn.toml key sets differ");
    }

    #[test]
    fn locales_load_and_resolve() {
        // Sanity-check that the TOML files parse and a known key resolves
        // to the expected value in both languages.
        assert_eq!(en_str("checking_lts"), "Checking latest LTS version...");
        assert_eq!(cn_str("checking_lts"), "正在检查最新 LTS 版本...");
        // CN falls back to EN for a missing key instead of returning the
        // raw key, so a missing CN entry is non-fatal (covered by the
        // keys-match test above instead).
        assert_eq!(cn_str("definitely_not_a_real_key"), "definitely_not_a_real_key");
    }
}
