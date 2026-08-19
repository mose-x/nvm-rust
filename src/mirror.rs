use anyhow::Result;
use colored::Colorize;

use crate::config::{load_config, save_config};
use crate::i18n::{format_t, T};

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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

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
        let _guard = crate::system::ENV_TESTS_MUTEX
            .lock()
            .expect("ENV_TESTS_MUTEX poisoned");
        std::env::set_var("NVM_LANG", "en");
        let err = normalize_mirror_url("   ").unwrap_err();
        assert!(format!("{err}").contains("empty"));
        std::env::remove_var("NVM_LANG");
    }

    #[test]
    fn test_normalize_mirror_url_upgrades_schemeless_to_https() {
        // A scheme-less URL is upgraded to https:// so users can paste a bare host.
        assert_eq!(
            normalize_mirror_url("registry.npmmirror.com/-/binary/node/").unwrap(),
            "https://registry.npmmirror.com/-/binary/node/"
        );
    }
}
