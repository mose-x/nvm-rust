use anyhow::Result;

use super::{IOJS_VERSION_RE, NODE_VERSION_RE};
use crate::i18n::{format_t, T};
use crate::system::{get_tags, os_suffix};
use crate::utils::{iojs_version_number, normalize_iojs_version, validate_version_name};

pub(crate) fn resolve_version(input: &str, base_url: &str) -> Result<String> {
    // Fully-specified version "vX.Y.Z" / "X.Y.Z" with two dots: use as-is.
    if input.starts_with('v') && input.matches('.').count() >= 2 {
        validate_version_name(input)?;
        return Ok(input.to_string());
    }
    if input.matches('.').count() >= 2 && input.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        let v = format!("v{}", input);
        validate_version_name(&v)?;
        return Ok(v);
    }

    // `lts`, `lts/*`, `lts/-1` (bare) → newest LTS across all lines.
    let lower = input.to_lowercase();
    if lower == "lts" || lower == "lts/*" || lower == "lts/-1" {
        return get_latest_lts_version(base_url);
    }

    // `lts/krypton`, `lts/iron`, ... → newest release in that LTS line.
    // nodejs.org exposes `latest-v{major}.x/` for every major, so we can
    // resolve any LTS codename (or any bare major) by listing that dir.
    // Use the remote-augmented alias table so a brand-new LTS line is
    // resolvable even before this binary's hardcoded table is updated.
    if lower.starts_with("lts/") {
        let aliases = crate::config::named_lts_aliases_with_remote(base_url);
        if let Some(prefix) = aliases.get(&lower) {
            let major = prefix.trim_start_matches('v');
            return get_latest_version_in_major(major, base_url);
        }
        anyhow::bail!("{}", format_t("unknown_lts_alias", &[input.to_string()]));
    }

    // Bare major ("20") or major.minor ("20.5") → newest release in that
    // major line. Matches nvm-sh: `nvm install 20` installs the latest v20.x.x.
    if let Some(major) = bare_major_for_install(input) {
        return get_latest_version_in_major(&major, base_url);
    }

    // Fall back to listing `{base_url}v{input}/` — used for partial versions
    // like "20.11" that should match a specific patch release directory.
    let version_num = input.trim_start_matches('v');
    let tags = get_tags(format!("{}v{}/", base_url, version_num));

    if tags.is_empty() {
        anyhow::bail!("{}", T("cannot_fetch_versions"));
    }

    let suffix = os_suffix();
    for tag in tags.iter().rev() {
        if tag.ends_with(suffix) {
            if let Some(caps) = NODE_VERSION_RE.captures(tag) {
                return Ok(caps[1].to_string());
            }
        }
    }

    anyhow::bail!("{}", format_t("cannot_resolve", &[input.to_string()]))
}

/// If `input` is a bare major ("22") or major.minor ("22.5"), optionally with
/// a leading `v` ("v22", "v22.5"), return the major as a string so we can
/// fetch the `latest-v{major}.x/` directory. Returns `None` for full versions
/// ("22.5.1"), aliases, io.js names, `system`, etc.
pub(crate) fn bare_major_for_install(input: &str) -> Option<String> {
    let s = crate::utils::validate_bare_major(input)?;
    s.split('.').next().map(|m| m.to_string())
}

/// Fetch the newest release in a major line by listing nodejs.org's
/// `latest-v{major}.x/` directory. Works for both LTS and non-LTS lines.
pub(crate) fn get_latest_version_in_major(major: &str, base_url: &str) -> Result<String> {
    let dir = format!("latest-v{}.x/", major);
    let tags = get_tags(format!("{}{}", base_url, dir));
    if tags.is_empty() {
        anyhow::bail!("{}", T("cannot_fetch_versions"));
    }
    let suffix = os_suffix();
    for tag in tags.iter().rev() {
        if tag.ends_with(suffix) {
            if let Some(caps) = NODE_VERSION_RE.captures(tag) {
                return Ok(caps[1].to_string());
            }
        }
    }
    anyhow::bail!("{}", format_t("cannot_resolve", &[format!("v{}.", major)]))
}

pub(crate) fn get_latest_lts_version(base_url: &str) -> Result<String> {
    // Prefer the official `index.json` manifest — it explicitly tags each
    // release with its LTS codename, which is far more reliable than scraping
    // the `latest-vXX.x/` directory links (those exist for every major,
    // including non-LTS odd ones, and the "highest even" heuristic breaks
    // when a newer non-LTS even major ships before the LTS line bumps).
    //
    // Any failure — network error, non-200, malformed JSON, no LTS entry —
    // maps to the same "cannot determine LTS" bail, matching the previous
    // fall-through behaviour but without 5 levels of nested `if let`.
    match latest_lts_from_index(base_url) {
        Some(v) => Ok(v),
        None => anyhow::bail!("{}", T("cannot_determine_lts")),
    }
}

/// Walk `index.json` (newest-first) and return the first release tagged with
/// an LTS codename. Returns `None` on any network/parse failure or when no
/// LTS release is present — the caller decides how to surface that.
fn latest_lts_from_index(base_url: &str) -> Option<String> {
    let index_url = format!("{}index.json", base_url);
    let client = crate::proxy::build_http_client();
    let resp = client.get(&index_url).send().ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let text = resp.text().ok()?;
    let json: serde_json::Value = serde_json::from_str(&text).ok()?;
    let arr = json.as_array()?;
    for entry in arr {
        // Each entry: { "version": "v24.18.0", "lts": "Krypton", ... }
        // Non-LTS releases have `"lts": false`.
        let is_lts = entry.get("lts").and_then(|v| v.as_str()).is_some();
        if is_lts {
            if let Some(ver) = entry.get("version").and_then(|v| v.as_str()) {
                return Some(ver.to_string());
            }
        }
    }
    None
}

pub(crate) fn get_latest_version(base_url: &str) -> Result<String> {
    let tags = get_tags(base_url.to_string());
    for tag in tags {
        if tag == "latest/" {
            let sub_tags = get_tags(format!("{}{}", base_url, tag));
            let suffix = os_suffix();
            for sub_tag in sub_tags {
                if sub_tag.ends_with(suffix) {
                    if let Some(caps) = NODE_VERSION_RE.captures(&sub_tag) {
                        return Ok(caps[1].to_string());
                    }
                }
            }
        }
    }
    anyhow::bail!("{}", T("cannot_determine_latest"))
}

pub(crate) fn get_download_url(version: &str, base_url: &str) -> Result<String> {
    let suffix = os_suffix();
    let version_dir = format!("{}{}/", base_url, version);

    let tags = get_tags(version_dir.clone());
    for tag in tags {
        if tag.ends_with(suffix) {
            return Ok(format!("{}{}", version_dir, tag));
        }
    }

    anyhow::bail!("{}", format_t("cannot_find_url", &[version.to_string()]))
}

/// Build source tarball URL: {base_url}/v{version}/node-v{version}.tar.gz
pub(crate) fn get_source_url(version: &str, base_url: &str) -> Result<String> {
    let url = format!(
        "{}/v{}/node-v{}.tar.gz",
        base_url.trim_end_matches('/'),
        version,
        version
    );
    Ok(url)
}

/// Resolve an io.js version string (e.g., "iojs-3.3.1", "io.js-v2.5.0", "1.0.0")
/// Returns canonical "iojs-vX.Y.Z"
pub(crate) fn resolve_iojs_version(input: &str, iojs_base_url: &str) -> Result<String> {
    let mut ver = input.trim().to_lowercase();

    // Normalize prefix variations — only at the start of the string. The
    // previous `ver.replace("io.js", "iojs")` was global and would also
    // rewrite any later "io.js" substring (e.g. inside a pre-release tag),
    // so strip the prefix explicitly instead of replacing across the whole
    // string.
    if let Some(rest) = ver.strip_prefix("io.js-v") {
        ver = format!("iojs-v{}", rest);
    } else if let Some(rest) = ver.strip_prefix("io.js-") {
        ver = format!("iojs-{}", rest);
    } else if let Some(rest) = ver.strip_prefix("io.js") {
        ver = format!("iojs{}", rest);
    }

    if !ver.starts_with("iojs") {
        ver = format!("iojs-v{}", ver);
    }
    if !ver.starts_with("iojs-v") {
        ver = ver.replace("iojs-", "iojs-v");
    }

    // If already fully specified (three parts), use as-is
    if ver.matches('.').count() >= 2 {
        let normalized = normalize_iojs_version(&ver);
        validate_version_name(&normalized)?;
        return Ok(normalized);
    }

    // Partial version, fetch from remote
    let v_num = ver.trim_start_matches("iojs-v").trim_start_matches("iojs-");
    let tags = get_tags(format!("{}v{}/", iojs_base_url, v_num));
    if tags.is_empty() {
        anyhow::bail!("{}", format_t("no_iojs_match", &[input.to_string()]));
    }

    let suffix = os_suffix();
    for tag in tags.iter().rev() {
        if tag.ends_with(&suffix) {
            if let Some(caps) = IOJS_VERSION_RE.captures(tag) {
                return Ok(format!("iojs-{}", &caps[1]));
            }
        }
    }

    anyhow::bail!("{}", format_t("cannot_resolve_iojs", &[input.to_string()]))
}

/// Build the download URL for an io.js binary tarball.
pub(crate) fn get_iojs_download_url(version: &str, iojs_base_url: &str) -> Result<String> {
    let ver_num = iojs_version_number(version).unwrap_or_else(|| version.to_string());
    let suffix = os_suffix();
    let version_dir = format!("{}v{}/", iojs_base_url, ver_num);
    let filename = format!("iojs-v{}-{}", ver_num, suffix);
    Ok(format!("{}{}", version_dir, filename))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // Fully-specified versions (>= 2 dots) short-circuit before any network
    // fetch, so resolve_iojs_version is deterministic for these inputs and
    // the `base_url` argument is never used.
    #[test]
    fn resolve_iojs_version_normalizes_prefixes() {
        let base = "https://example.com/";
        assert_eq!(
            resolve_iojs_version("io.js-v3.3.1", base).unwrap(),
            "iojs-v3.3.1"
        );
        assert_eq!(
            resolve_iojs_version("io.js-3.3.1", base).unwrap(),
            "iojs-v3.3.1"
        );
        assert_eq!(
            resolve_iojs_version("iojs-3.3.1", base).unwrap(),
            "iojs-v3.3.1"
        );
        assert_eq!(
            resolve_iojs_version("iojs-v3.3.1", base).unwrap(),
            "iojs-v3.3.1"
        );
        assert_eq!(resolve_iojs_version("3.3.1", base).unwrap(), "iojs-v3.3.1");
    }

    #[test]
    fn resolve_iojs_version_only_rewrites_leading_iojs_prefix() {
        // Regression: the old `ver.replace("io.js", "iojs")` was global and
        // would rewrite a non-prefix "io.js" substring. Only the leading
        // prefix should be normalized; a trailing "io.js" (e.g. in a
        // pre-release tag) must be preserved.
        let base = "https://example.com/";
        assert_eq!(
            resolve_iojs_version("iojs-3.3.1-io.js", base).unwrap(),
            "iojs-v3.3.1-io.js"
        );
    }
}
