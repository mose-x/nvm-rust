//! Release API client for `nvm upgrade` — fetches the latest release tag
//! and asset list from GitHub or Gitee, with an HTML-page fallback when
//! the GitHub API is rate-limited.
//!
//! Extracted from `upgrade.rs` as part of P1-7 module refactoring. All
//! items are re-exported through `upgrade::*` so existing call sites are
//! unchanged.

use anyhow::{Context, Result};

use crate::i18n::{format_t, T};

/// GitHub repo coordinates. Hardcoded — this is nvm-rust's own self-update,
/// not a generic installer.
const REPO_OWNER: &str = "mose-x";
const REPO_NAME: &str = "nvm-rust";

/// Gitee mirror repo (must be kept in sync manually by the owner on each
/// release). `--from-gitee` switches both the API and the download URL.
const GITEE_OWNER: &str = "mose-x";
const GITEE_REPO: &str = "nvm-rust";

/// A release asset (name + download URL).
pub(crate) struct Asset {
    pub(crate) name: String,
    pub(crate) url: String,
}

/// Fetch the latest release tag and its asset list.
///
/// Returns `(tag, assets, source_label)` where `source_label` is a short
/// human-readable string for the "latest version" line ("GitHub" / "Gitee").
///
/// GitHub's unauthenticated API is limited to 60 requests/hour/IP. For
/// typical `nvm upgrade` usage this is plenty; users who hit the limit can
/// set `GITHUB_TOKEN` and we'll send it as a bearer token (5000/hour).
pub(crate) fn fetch_latest_release(
    client: &reqwest::blocking::Client,
    from_gitee: bool,
) -> Result<(String, Vec<Asset>, &'static str)> {
    if from_gitee {
        // Gitee API: GET /api/v5/repos/{owner}/{repo}/releases/latest
        // Returns a release object with `tag_name` and `assets[]`.
        let url = format!(
            "https://gitee.com/api/v5/repos/{}/{}/releases/latest",
            GITEE_OWNER, GITEE_REPO
        );
        let resp = client
            .get(&url)
            .send()
            .map_err(|e| anyhow::anyhow!("{}: {}", T("upgrade_fetch_failed"), e))?;
        if !resp.status().is_success() {
            anyhow::bail!(
                "{}",
                format_t(
                    "upgrade_fetch_http_failed",
                    std::slice::from_ref(&format!("{}", resp.status()))
                )
            );
        }
        let text = resp.text().context(T("upgrade_parse_failed"))?;
        let json: serde_json::Value =
            serde_json::from_str(&text).context(T("upgrade_parse_failed"))?;
        let tag = json
            .get("tag_name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("{}", T("upgrade_no_tag")))?
            .to_string();
        let assets = json
            .get("assets")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|a| {
                        let name = a.get("name")?.as_str()?.to_string();
                        let url = a.get("browser_download_url")?.as_str()?.to_string();
                        Some(Asset { name, url })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        return Ok((tag, assets, "Gitee"));
    }

    // GitHub API: GET /repos/{owner}/{repo}/releases/latest
    let url = format!(
        "https://api.github.com/repos/{}/{}/releases/latest",
        REPO_OWNER, REPO_NAME
    );
    // GitHub API rejects requests without a User-Agent with 403. Send one
    // identifying nvm-rust's self-updater.
    let mut req = client
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .header(
            "User-Agent",
            format!("nvm-rust/{}", env!("CARGO_PKG_VERSION")),
        );
    // Optional: send GITHUB_TOKEN to lift the 60/hour rate limit to 5000/hour.
    if let Ok(tok) = std::env::var("GITHUB_TOKEN") {
        if !tok.is_empty() {
            req = req.header("Authorization", format!("Bearer {}", tok));
        }
    }
    let resp = req
        .send()
        .map_err(|e| anyhow::anyhow!("{}: {}", T("upgrade_fetch_failed"), e))?;
    if !resp.status().is_success() {
        // On failure, surface GitHub's message body instead of just the status
        // code. GitHub returns JSON with a "message" field explaining the
        // reason (rate limit, repo not found, etc.); printing it tells the
        // user exactly what to fix. For 403 with rate-limit headers, append a
        // hint to set GITHUB_TOKEN (raises the anonymous 60/hour cap to
        // 5000/hour).
        let status = resp.status();
        let remaining = resp
            .headers()
            .get("x-ratelimit-remaining")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
            .unwrap_or_default();
        let body = resp.text().unwrap_or_default();
        let gh_msg = serde_json::from_str::<serde_json::Value>(&body)
            .ok()
            .and_then(|v| v.get("message").and_then(|m| m.as_str()).map(String::from))
            .unwrap_or_default();
        let rate_limited = status.as_u16() == 403 && remaining == "0";
        if rate_limited {
            // Auto-fallback: the GitHub API is rate-limited (60/hour per IP for
            // anonymous), but the releases HTML page (github.com/.../releases/latest)
            // is served by a different system and is NOT rate-limited. It 302-
            // redirects to releases/tag/<tag>, so we resolve the tag from the
            // redirect URL and construct asset download URLs from the tag.
            // This keeps `nvm upgrade` working for regular users on shared IPs
            // without requiring a GITHUB_TOKEN. Print a clear two-line notice:
            // what happened + what (if anything) the user can do about it.
            use colored::Colorize;
            eprintln!(
                "  {} {}",
                "ℹ".cyan().bold(),
                T("upgrade_fallback_to_html").cyan()
            );
            eprintln!("    {}", T("upgrade_fallback_to_html_detail").dimmed());
            return fetch_latest_release_via_html(client);
        } else if !gh_msg.is_empty() {
            anyhow::bail!(
                "{}\n  {}",
                format_t(
                    "upgrade_fetch_http_failed",
                    std::slice::from_ref(&format!("{}", status))
                ),
                gh_msg
            );
        } else {
            anyhow::bail!(
                "{}",
                format_t(
                    "upgrade_fetch_http_failed",
                    std::slice::from_ref(&format!("{}", status))
                )
            );
        }
    }
    let text = resp.text().context(T("upgrade_parse_failed"))?;
    let json: serde_json::Value = serde_json::from_str(&text).context(T("upgrade_parse_failed"))?;
    let tag = json
        .get("tag_name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("{}", T("upgrade_no_tag")))?
        .to_string();
    let assets = json
        .get("assets")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|a| {
                    let name = a.get("name")?.as_str()?.to_string();
                    let url = a.get("browser_download_url")?.as_str()?.to_string();
                    Some(Asset { name, url })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Ok((tag, assets, "GitHub"))
}

/// Extract the release tag from a `releases/tag/<tag>` redirect URL.
/// Returns `None` if the URL has no trailing path segment.
fn parse_tag_from_url(url: &str) -> Option<String> {
    let tag = url.rsplit('/').next()?;
    if tag.is_empty() {
        None
    } else {
        Some(tag.to_string())
    }
}

/// Construct the known asset list for a release tag, using the fixed
/// `nvm-<version>-<target>.<ext>` naming scheme produced by release.yml.
/// Includes `sha256sums.txt`. The tag keeps its leading `v` (used in the
/// download URL path); the version (leading `v` stripped) is used in asset
/// filenames.
pub(crate) fn build_assets_from_tag(tag: &str) -> Vec<Asset> {
    let version = tag.strip_prefix('v').unwrap_or(tag);
    let targets_exts: &[(&str, &str)] = &[
        ("linux-x64", "tar.gz"),
        ("linux-musl-x64", "tar.gz"),
        ("linux-arm64", "tar.gz"),
        ("linux-musl-arm64", "tar.gz"),
        ("macos-x64", "tar.gz"),
        ("macos-arm64", "tar.gz"),
        ("windows-x64", "zip"),
        ("windows-arm64", "zip"),
    ];
    let base = format!(
        "https://github.com/{}/{}/releases/download/{}",
        REPO_OWNER, REPO_NAME, tag
    );
    let mut assets: Vec<Asset> = targets_exts
        .iter()
        .map(|(target, ext)| {
            let name = format!("nvm-{}-{}.{}", version, target, ext);
            let url = format!("{}/{}", base, name);
            Asset { name, url }
        })
        .collect();
    assets.push(Asset {
        name: "sha256sums.txt".to_string(),
        url: format!("{}/sha256sums.txt", base),
    });
    assets
}

/// Fallback when the GitHub API is rate-limited: resolve the latest release
/// tag from the `releases/latest` HTML page, which 302-redirects to
/// `releases/tag/<tag>`. This endpoint is served by github.com (not
/// api.github.com) and is NOT subject to the 60/hour API rate limit, so it
/// keeps `nvm upgrade` working for regular users on shared IPs without a
/// GITHUB_TOKEN.
///
/// Since we cannot list assets without the API, we construct the known asset
/// set from the tag + the fixed release naming scheme
/// (`nvm-<version>-<target>.<ext>` across all supported platforms, plus
/// `sha256sums.txt`). The caller picks the matching asset by `<target>.<ext>`
/// suffix, same as the API path. If a platform's asset is absent from the
/// release, the download itself 404s with a clear error.
fn fetch_latest_release_via_html(
    client: &reqwest::blocking::Client,
) -> Result<(String, Vec<Asset>, &'static str)> {
    let url = format!(
        "https://github.com/{}/{}/releases/latest",
        REPO_OWNER, REPO_NAME
    );
    // reqwest follows the 302 by default; resp.url() is the final URL.
    let resp = client
        .get(&url)
        .header(
            "User-Agent",
            format!("nvm-rust/{}", env!("CARGO_PKG_VERSION")),
        )
        .send()
        .map_err(|e| anyhow::anyhow!("{}: {}", T("upgrade_fetch_failed"), e))?;
    if !resp.status().is_success() {
        // Both the API and the HTML fallback failed. Surface the status and
        // the GITHUB_TOKEN hint: the token is the only reliable way to bypass
        // IP-based rate limiting on shared networks.
        anyhow::bail!(
            "{}\n  {}",
            format_t(
                "upgrade_fetch_http_failed",
                std::slice::from_ref(&format!("{}", resp.status()))
            ),
            T("upgrade_rate_limited_hint")
        );
    }
    // Final URL looks like:
    //   https://github.com/<owner>/<repo>/releases/tag/v2.0.0
    let tag = parse_tag_from_url(resp.url().as_str())
        .ok_or_else(|| anyhow::anyhow!("{}", T("upgrade_no_tag")))?;
    let assets = build_assets_from_tag(&tag);
    Ok((tag, assets, "GitHub (HTML)"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_tag_from_url_normal() {
        let url = "https://github.com/mose-x/nvm-rust/releases/tag/v2.0.0";
        assert_eq!(parse_tag_from_url(url).as_deref(), Some("v2.0.0"));
    }

    #[test]
    fn test_parse_tag_from_url_no_trailing_segment() {
        // Trailing slash → empty segment → None
        let url = "https://github.com/mose-x/nvm-rust/releases/tag/";
        assert_eq!(parse_tag_from_url(url), None);
    }

    #[test]
    fn test_parse_tag_from_url_pre_release_tag() {
        let url = "https://github.com/mose-x/nvm-rust/releases/tag/v2.0.0-rc.1";
        assert_eq!(parse_tag_from_url(url).as_deref(), Some("v2.0.0-rc.1"));
    }

    #[test]
    fn test_build_assets_from_tag_v_prefixed() {
        let assets = build_assets_from_tag("v2.0.0");
        // 8 platform assets + sha256sums.txt
        assert_eq!(assets.len(), 9);
        // Asset filenames use the version WITHOUT the leading 'v'
        let linux_x64 = assets
            .iter()
            .find(|a| a.name == "nvm-2.0.0-linux-x64.tar.gz")
            .expect("linux-x64 asset must exist");
        assert!(linux_x64.url.starts_with("https://github.com/"));
        assert!(linux_x64
            .url
            .ends_with("/releases/download/v2.0.0/nvm-2.0.0-linux-x64.tar.gz"));
        // Download URL path keeps the leading 'v' (it's the tag name)
        assert!(assets
            .iter()
            .any(|a| a.name == "nvm-2.0.0-linux-musl-x64.tar.gz"));
        assert!(assets
            .iter()
            .any(|a| a.name == "nvm-2.0.0-linux-musl-arm64.tar.gz"));
        assert!(assets
            .iter()
            .any(|a| a.name == "nvm-2.0.0-macos-arm64.tar.gz"));
        assert!(assets.iter().any(|a| a.name == "nvm-2.0.0-windows-x64.zip"));
        let sums = assets
            .iter()
            .find(|a| a.name == "sha256sums.txt")
            .expect("sha256sums.txt must exist");
        assert!(sums
            .url
            .ends_with("/releases/download/v2.0.0/sha256sums.txt"));
    }

    #[test]
    fn test_build_assets_from_tag_no_v_prefix() {
        // Some users/CI may tag without 'v'; the version in filenames
        // equals the tag itself.
        let assets = build_assets_from_tag("2.0.0");
        assert!(assets
            .iter()
            .any(|a| a.name == "nvm-2.0.0-linux-x64.tar.gz"));
        assert!(assets.iter().any(|a| a
            .url
            .ends_with("/releases/download/2.0.0/nvm-2.0.0-linux-x64.tar.gz")));
    }

    #[test]
    fn test_build_assets_covers_all_release_yml_targets() {
        // Must match the 8 targets produced by release.yml exactly.
        let assets = build_assets_from_tag("v9.9.9");
        let names: Vec<&str> = assets.iter().map(|a| a.name.as_str()).collect();
        assert!(names.contains(&"nvm-9.9.9-linux-x64.tar.gz"));
        assert!(names.contains(&"nvm-9.9.9-linux-musl-x64.tar.gz"));
        assert!(names.contains(&"nvm-9.9.9-linux-arm64.tar.gz"));
        assert!(names.contains(&"nvm-9.9.9-linux-musl-arm64.tar.gz"));
        assert!(names.contains(&"nvm-9.9.9-macos-x64.tar.gz"));
        assert!(names.contains(&"nvm-9.9.9-macos-arm64.tar.gz"));
        assert!(names.contains(&"nvm-9.9.9-windows-x64.zip"));
        assert!(names.contains(&"nvm-9.9.9-windows-arm64.zip"));
        assert!(names.contains(&"sha256sums.txt"));
    }

    #[test]
    fn test_build_assets_includes_musl_arm64() {
        // Regression test: linux-musl-arm64 was missing from the asset list,
        // causing Alpine ARM64 users to download a glibc binary via nvm upgrade.
        let assets = build_assets_from_tag("v1.0.0");
        assert!(
            assets
                .iter()
                .any(|a| a.name == "nvm-1.0.0-linux-musl-arm64.tar.gz"),
            "linux-musl-arm64 asset must exist for Alpine ARM64 self-upgrade"
        );
    }
}
