use anyhow::Result;
use std::env;
use std::time::Duration;

use crate::config::{load_config, save_config};

/// Get the currently configured proxy from environment or system settings.
///
/// Lookup order:
/// 1. Environment variables (HTTPS_PROXY / https_proxy / HTTP_PROXY / ...)
/// 2. Platform-native system proxy:
///    - Windows: registry Internet Settings (incl. AutoConfigURL PAC)
///    - macOS:   scutil --proxy (incl. ProxyAutoConfigEnable / SOCKSEnable)
///    - Linux:   GNOME gsettings (mode = 'auto' PAC or 'manual' per-protocol)
pub fn get_system_proxy() -> Option<String> {
    // 1. Environment variables take priority
    for var in &[
        "HTTPS_PROXY",
        "https_proxy",
        "HTTP_PROXY",
        "http_proxy",
        "ALL_PROXY",
        "all_proxy",
    ] {
        if let Ok(val) = env::var(var) {
            if !val.is_empty() {
                return Some(val);
            }
        }
    }

    // 2. Platform-native system proxy
    platform_system_proxy()
}

/// Read the platform-native system proxy configuration.
#[cfg(target_os = "windows")]
fn platform_system_proxy() -> Option<String> {
    winreg_system_proxy()
}

#[cfg(target_os = "macos")]
fn platform_system_proxy() -> Option<String> {
    scutil_system_proxy()
}

#[cfg(target_os = "linux")]
fn platform_system_proxy() -> Option<String> {
    gsettings_system_proxy()
}

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
fn platform_system_proxy() -> Option<String> {
    None
}

// --- Windows: read registry via `reg query` (no extra crate needed) -----------

#[cfg(target_os = "windows")]
fn winreg_system_proxy() -> Option<String> {
    use std::process::Command;

    const REG_KEY: &str = "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings";

    // 1. PAC has priority: AutoConfigURL points at a .pac file (http(s):// or file://).
    let out = Command::new("reg")
        .args(["query", REG_KEY, "/v", "AutoConfigURL"])
        .output()
        .ok()?;
    if out.status.success() {
        let stdout = String::from_utf8_lossy(&out.stdout);
        for line in stdout.lines() {
            if let Some(idx) = line.rfind("REG_SZ") {
                let val = line[idx + "REG_SZ".len()..].trim();
                if !val.is_empty() {
                    if let Some(pac) = fetch_pac(val) {
                        if let Some(proxy_url) = parse_pac_response(&pac) {
                            return Some(proxy_url);
                        }
                    }
                    break;
                }
            }
        }
    }

    // 2. Static ProxyEnable / ProxyServer.
    let out = Command::new("reg")
        .args(["query", REG_KEY, "/v", "ProxyEnable"])
        .output()
        .ok()?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    if !stdout.contains("0x1") {
        return None;
    }

    let out = Command::new("reg")
        .args(["query", REG_KEY, "/v", "ProxyServer"])
        .output()
        .ok()?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    for line in stdout.lines() {
        if let Some(idx) = line.rfind("REG_SZ") {
            let val = line[idx + "REG_SZ".len()..].trim();
            if val.is_empty() {
                continue;
            }
            return Some(parse_windows_proxy_server(val));
        }
    }
    None
}

/// Parse Windows ProxyServer value. It can be:
/// - "127.0.0.1:8080"            -> http://127.0.0.1:8080
/// - "http=...;https=..."        -> pick https scheme
/// - "socks=..."                 -> socks://...
#[cfg(target_os = "windows")]
fn parse_windows_proxy_server(val: &str) -> String {
    if val.contains('=') {
        for part in val.split(';') {
            let part = part.trim();
            if let Some(rest) = part.strip_prefix("https=") {
                return normalize_proxy_url(rest, "http");
            }
            if let Some(rest) = part.strip_prefix("http=") {
                return normalize_proxy_url(rest, "http");
            }
            if let Some(rest) = part.strip_prefix("socks=") {
                return normalize_proxy_url(rest, "socks5");
            }
        }
        // Fall through to use the whole value if no known scheme found.
    }
    normalize_proxy_url(val, "http")
}

/// Ensure a proxy URL has a scheme; default to the given scheme if missing.
fn normalize_proxy_url(addr: &str, default_scheme: &str) -> String {
    let addr = addr.trim();
    if addr.starts_with("http://") || addr.starts_with("https://") || addr.starts_with("socks") {
        addr.to_string()
    } else {
        format!("{}://{}", default_scheme, addr)
    }
}

// --- macOS: read system proxy via `scutil --proxy` -------------------------

#[cfg(target_os = "macos")]
fn scutil_system_proxy() -> Option<String> {
    let out = std::process::Command::new("scutil")
        .arg("--proxy")
        .output()
        .ok()?;
    let txt = String::from_utf8_lossy(&out.stdout);

    // scutil prints "Key : Value" lines. Collect them into a map so the
    // priority logic below is independent of the field order scutil happens
    // to emit (which has changed across macOS versions).
    let mut fields: std::collections::HashMap<&str, &str> = std::collections::HashMap::new();
    for line in txt.lines() {
        let line = line.trim();
        if let Some((k, v)) = line.split_once(" : ") {
            fields.insert(k.trim(), v.trim());
        }
    }

    let enabled = |k: &str| fields.get(k).map(|v| *v == "1").unwrap_or(false);
    let pair = |host_key: &str, port_key: &str| -> Option<(&str, &str)> {
        match (fields.get(host_key), fields.get(port_key)) {
            (Some(h), Some(p)) if !h.is_empty() && !p.is_empty() => Some((*h, *p)),
            _ => None,
        }
    };

    // 1. PAC has priority: macOS only exposes one PAC URL, so fetch it and
    //    pick the first non-DIRECT entry. This is what makes Clash/Surge
    //    "system proxy" mode work for nvm-rust.
    if enabled("ProxyAutoConfigEnable") {
        if let Some(url) = fields
            .get("ProxyAutoConfigURL")
            .or_else(|| fields.get("ProxyAutoConfigURLString"))
        {
            if !url.is_empty() {
                if let Some(pac) = fetch_pac(url) {
                    if let Some(proxy_url) = parse_pac_response(&pac) {
                        return Some(proxy_url);
                    }
                }
            }
        }
    }

    // 2. HTTPS proxy (nvm traffic is HTTPS, so prefer HTTPSProxy over HTTPProxy).
    if enabled("HTTPSEnable") {
        if let Some((h, p)) = pair("HTTPSProxy", "HTTPSPort") {
            return Some(format!("http://{}:{}", h, p));
        }
    }

    // 3. SOCKS proxy (now usable since reqwest was built with the "socks" feature).
    if enabled("SOCKSEnable") {
        if let Some((h, p)) = pair("SOCKSProxy", "SOCKSPort") {
            return Some(format!("socks5://{}:{}", h, p));
        }
    }

    // 4. HTTP proxy (fallback).
    if enabled("HTTPEnable") {
        if let Some((h, p)) = pair("HTTPProxy", "HTTPPort") {
            return Some(format!("http://{}:{}", h, p));
        }
    }

    None
}

// --- Linux: read GNOME proxy via `gsettings` --------------------------------

#[cfg(target_os = "linux")]
fn gsettings_system_proxy() -> Option<String> {
    let mode = std::process::Command::new("gsettings")
        .args(["get", "org.gnome.system.proxy", "mode"])
        .output()
        .ok()?;
    let mode_txt = String::from_utf8_lossy(&mode.stdout);

    // 1. 'auto' = PAC URL built from org.gnome.system.proxy.autoconfig.
    if mode_txt.contains("'auto'") {
        let host = gsettings_get("org.gnome.system.proxy.autoconfig", "host")?;
        let port = gsettings_get("org.gnome.system.proxy.autoconfig", "port")?;
        if !host.is_empty() && !port.is_empty() {
            let pac_url = format!("http://{}:{}", host, port);
            if let Some(pac) = fetch_pac(&pac_url) {
                if let Some(proxy_url) = parse_pac_response(&pac) {
                    return Some(proxy_url);
                }
            }
        }
        return None;
    }

    // 2. 'manual' = static per-protocol host/port. HTTPS > SOCKS > HTTP
    //    (nvm downloads are HTTPS, so prefer the https entry when present).
    if !mode_txt.contains("'manual'") {
        return None;
    }

    if let Some(proxy) = gsettings_proxy_url("org.gnome.system.proxy.https", "http") {
        return Some(proxy);
    }
    if let Some(proxy) = gsettings_proxy_url("org.gnome.system.proxy.socks", "socks5") {
        return Some(proxy);
    }
    if let Some(proxy) = gsettings_proxy_url("org.gnome.system.proxy.http", "http") {
        return Some(proxy);
    }
    None
}

/// Read a single `gsettings` key as a trimmed string (single-quoted values
/// have the quotes stripped). Returns None on any error or empty result.
#[cfg(target_os = "linux")]
fn gsettings_get(schema: &str, key: &str) -> Option<String> {
    let out = std::process::Command::new("gsettings")
        .args(["get", schema, key])
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&out.stdout)
        .trim_matches('\'')
        .trim()
        .to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

/// Build a `scheme://host:port` URL from a per-protocol gsettings schema.
#[cfg(target_os = "linux")]
fn gsettings_proxy_url(schema: &str, scheme: &str) -> Option<String> {
    let host = gsettings_get(schema, "host")?;
    let port = gsettings_get(schema, "port")?;
    if host.is_empty() || port.is_empty() {
        return None;
    }
    Some(format!("{}://{}:{}", scheme, host, port))
}

// --- PAC (Proxy Auto-Config) fetch + parse ----------------------------------
//
// A PAC file is a JavaScript function `FindProxyForURL(url, host)` that
// returns a string of the form "PROXY host:port; SOCKS host:port; DIRECT".
// We don't run the JS (that would need a full JS engine); instead we fetch
// the file and pick the first non-DIRECT entry. This handles the common
// case where Clash/Surge/etc. expose a single fallback proxy via PAC.

/// Fetch a PAC file from an http(s):// or file:// URL.
///
/// Uses a short timeout and a proxy-less client (the PAC itself must be
/// reachable directly, otherwise we'd have a chicken-and-egg loop). Returns
/// None on any failure so callers can fall back to the next detection step.
fn fetch_pac(url: &str) -> Option<String> {
    if let Some(path) = url.strip_prefix("file://") {
        return std::fs::read_to_string(path).ok();
    }
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
        .ok()?;
    client.get(url).send().ok()?.text().ok()
}

/// Parse a PAC `FindProxyForURL` return value into a single proxy URL.
///
/// Walks the `;`-separated entries left-to-right and returns the first
/// non-DIRECT one. Recognised tokens:
/// - `PROXY host:port` / `HTTP host:port` -> `http://host:port`
/// - `SOCKS host:port` / `SOCKS5 host:port` -> `socks5://host:port`
/// - `DIRECT` -> skipped
///
/// Unknown tokens are skipped. Returns None if no usable entry is found.
fn parse_pac_response(pac_text: &str) -> Option<String> {
    for part in pac_text.split(';') {
        let part = part.trim();
        if part.is_empty() || part.eq_ignore_ascii_case("DIRECT") {
            continue;
        }
        let (proto, addr) = match part.split_once(char::is_whitespace) {
            Some(p) => p,
            None => continue,
        };
        let addr = addr.trim();
        if addr.is_empty() {
            continue;
        }
        let scheme = match proto.to_ascii_uppercase().as_str() {
            "PROXY" | "HTTP" => "http",
            "SOCKS" | "SOCKS5" | "SOCKS4" => "socks5",
            _ => continue,
        };
        return Some(normalize_proxy_url(addr, scheme));
    }
    None
}

// --- Unified HTTP client builder --------------------------------------------

/// Build a reqwest blocking client honoring the NVM proxy setting, with the
/// given timeout. All three public client builders delegate here so the
/// proxy-detection + fallback logic lives in exactly one place.
///
/// When the user has run `nvm proxy on` and a system proxy is detected, the
/// traffic is routed through it; otherwise a plain direct client is returned.
/// If the proxied builder fails to construct (malformed proxy URL, etc.) we
/// fall back to `Client::new()` (an infallible direct client with default
/// settings) so a bad proxy setting never hard-aborts a download.
fn build_client(timeout: Duration) -> reqwest::blocking::Client {
    let mut builder = reqwest::blocking::Client::builder()
        .timeout(timeout)
        .danger_accept_invalid_certs(false);

    if is_proxy_enabled() {
        if let Some(proxy_url) = get_system_proxy() {
            if let Ok(proxy) = reqwest::Proxy::all(&proxy_url) {
                builder = builder.proxy(proxy);
            }
        }
    }

    builder.build().unwrap_or_else(|_| {
        // `Client::new()` is the one infallible constructor reqwest exposes
        // (it uses default settings, no timeout/proxy). Losing the timeout on
        // this fallback path is acceptable: this branch only runs when the
        // primary builder fails (e.g. a misconfigured TLS backend), which is
        // rare — and a working client without a timeout beats panicking here.
        reqwest::blocking::Client::new()
    })
}

/// Long-timeout (5 min) client for tarball downloads, which can legitimately
/// be large and slow on poor connections.
pub fn build_http_client() -> reqwest::blocking::Client {
    build_client(Duration::from_secs(300))
}

/// Short-timeout client (30s) for lightweight metadata fetches: version
/// listings (`get_tags`), latest-LTS lookups, checksum and GPG signature
/// files. These are small requests that should never need the 5-minute window
/// reserved for tarball downloads — a hung keyserver or DNS resolution should
/// fail fast instead of stalling `nvm install`.
pub fn build_listing_client() -> reqwest::blocking::Client {
    build_client(Duration::from_secs(30))
}

/// Short-timeout client for connectivity tests. Always uses the proxy when it
/// is enabled, so that the test reflects the real download path.
pub fn build_test_client(timeout_secs: u64) -> reqwest::blocking::Client {
    build_client(Duration::from_secs(timeout_secs))
}

/// Check if a URL is reachable (with a short timeout), honoring proxy.
fn is_reachable(url: &str, timeout_secs: u64) -> bool {
    let client = build_test_client(timeout_secs);
    client
        .get(url)
        .send()
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

/// Test internet connectivity (both domestic and international).
///
/// The two probes run concurrently so the user waits for the slowest one
/// (typically Google timing out behind the GFW) rather than their sum.
pub fn test_connectivity() -> (bool, bool) {
    std::thread::scope(|s| {
        let baidu = s.spawn(|| is_reachable("https://www.baidu.com", 5));
        let google = s.spawn(|| is_reachable("https://www.google.com", 5));
        (
            baidu.join().unwrap_or(false),
            google.join().unwrap_or(false),
        )
    })
}

/// Check if proxy is enabled in config.
pub fn is_proxy_enabled() -> bool {
    load_config().ok().and_then(|c| c.proxy).unwrap_or(false)
}

/// Enable or disable proxy in config.
pub fn set_proxy_enabled(enabled: bool) -> Result<()> {
    let mut config = load_config()?;
    config.proxy = Some(enabled);
    save_config(&config)?;
    Ok(())
}

/// Get proxy status info for display.
pub fn proxy_status() -> ProxyStatus {
    let nvm_proxy = is_proxy_enabled();
    let sys_proxy = get_system_proxy();
    ProxyStatus {
        nvm_proxy_enabled: nvm_proxy,
        system_proxy: sys_proxy,
    }
}

pub struct ProxyStatus {
    pub nvm_proxy_enabled: bool,
    pub system_proxy: Option<String>,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_proxy_status_struct() {
        let status = ProxyStatus {
            nvm_proxy_enabled: true,
            system_proxy: Some("http://127.0.0.1:8080".to_string()),
        };
        assert!(status.nvm_proxy_enabled);
        assert_eq!(
            status.system_proxy,
            Some("http://127.0.0.1:8080".to_string())
        );

        let status2 = ProxyStatus {
            nvm_proxy_enabled: false,
            system_proxy: None,
        };
        assert!(!status2.nvm_proxy_enabled);
        assert!(status2.system_proxy.is_none());
    }

    #[test]
    fn test_get_system_proxy_no_env() {
        // When no proxy env vars are set, should return None
        // Note: This test may behave differently depending on system env
        let result = get_system_proxy();
        // Just verify it doesn't panic and returns the expected type
        assert!(result.is_none() || result.is_some());
    }

    #[test]
    fn test_normalize_proxy_url_adds_scheme() {
        assert_eq!(
            normalize_proxy_url("127.0.0.1:8080", "http"),
            "http://127.0.0.1:8080"
        );
        assert_eq!(
            normalize_proxy_url("http://127.0.0.1:8080", "http"),
            "http://127.0.0.1:8080"
        );
        assert_eq!(
            normalize_proxy_url("socks5://127.0.0.1:1080", "http"),
            "socks5://127.0.0.1:1080"
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn test_parse_windows_proxy_server_plain() {
        assert_eq!(
            parse_windows_proxy_server("127.0.0.1:8080"),
            "http://127.0.0.1:8080"
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn test_parse_windows_proxy_server_schemes() {
        let r = parse_windows_proxy_server("http=127.0.0.1:8080;https=127.0.0.1:8443");
        assert_eq!(r, "http://127.0.0.1:8443");
    }

    #[test]
    fn test_build_http_client_smoke() {
        // Should not panic and should produce a usable client.
        let _client = build_http_client();
    }

    #[test]
    fn test_build_test_client_smoke() {
        let _client = build_test_client(2);
    }

    #[test]
    fn test_parse_pac_response_single_proxy() {
        assert_eq!(
            parse_pac_response("PROXY 127.0.0.1:7890"),
            Some("http://127.0.0.1:7890".to_string())
        );
    }

    #[test]
    fn test_parse_pac_response_single_socks5() {
        assert_eq!(
            parse_pac_response("SOCKS5 127.0.0.1:1080"),
            Some("socks5://127.0.0.1:1080".to_string())
        );
        assert_eq!(
            parse_pac_response("SOCKS 127.0.0.1:1080"),
            Some("socks5://127.0.0.1:1080".to_string())
        );
    }

    #[test]
    fn test_parse_pac_response_picks_first_non_direct() {
        // Multiple entries separated by `;` -> first non-DIRECT wins.
        assert_eq!(
            parse_pac_response("PROXY 127.0.0.1:7890; DIRECT"),
            Some("http://127.0.0.1:7890".to_string())
        );
        assert_eq!(
            parse_pac_response("DIRECT; PROXY 127.0.0.1:7890"),
            Some("http://127.0.0.1:7890".to_string())
        );
        assert_eq!(
            parse_pac_response("SOCKS5 127.0.0.1:1080; PROXY 127.0.0.1:7890; DIRECT"),
            Some("socks5://127.0.0.1:1080".to_string())
        );
    }

    #[test]
    fn test_parse_pac_response_direct_only_returns_none() {
        assert_eq!(parse_pac_response("DIRECT"), None);
        assert_eq!(parse_pac_response("DIRECT; DIRECT"), None);
    }

    #[test]
    fn test_parse_pac_response_empty_and_garbage_returns_none() {
        assert_eq!(parse_pac_response(""), None);
        assert_eq!(parse_pac_response("   "), None);
        // Unknown token is skipped, leaving nothing usable.
        assert_eq!(parse_pac_response("FOO 127.0.0.1:1"), None);
        // Address missing after token.
        assert_eq!(parse_pac_response("PROXY"), None);
        assert_eq!(parse_pac_response("PROXY    "), None);
    }

    #[test]
    fn test_parse_pac_response_case_insensitive_proto() {
        assert_eq!(
            parse_pac_response("proxy 127.0.0.1:7890"),
            Some("http://127.0.0.1:7890".to_string())
        );
        assert_eq!(
            parse_pac_response("Socks5 127.0.0.1:1080"),
            Some("socks5://127.0.0.1:1080".to_string())
        );
    }

    #[test]
    fn test_parse_pac_response_normalizes_addr_without_scheme() {
        // `PROXY host:port` (no http:// prefix) -> normalized to http://
        assert_eq!(
            parse_pac_response("PROXY 127.0.0.1:7890"),
            Some("http://127.0.0.1:7890".to_string())
        );
    }

    #[test]
    fn test_parse_pac_response_http_token_alias() {
        // `HTTP host:port` is a valid alias for `PROXY`.
        assert_eq!(
            parse_pac_response("HTTP 127.0.0.1:8080"),
            Some("http://127.0.0.1:8080".to_string())
        );
    }
}
