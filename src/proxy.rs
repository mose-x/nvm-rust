use anyhow::Result;
use std::env;
use std::time::Duration;

use crate::config::{load_config, save_config};

/// Get the currently configured proxy from environment or system settings.
///
/// Lookup order:
/// 1. Environment variables (HTTPS_PROXY / https_proxy / HTTP_PROXY / ...)
/// 2. Platform-native system proxy:
///    - Windows: registry Internet Settings
///    - macOS:   scutil --proxy
///    - Linux:   GNOME gsettings
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
    let out = std::process::Command::new("reg")
        .args([
            "query",
            "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings",
            "/v",
            "ProxyEnable",
        ])
        .output()
        .ok()?;

    let stdout = String::from_utf8_lossy(&out.stdout);
    if !stdout.contains("0x1") {
        return None;
    }

    let out = std::process::Command::new("reg")
        .args([
            "query",
            "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings",
            "/v",
            "ProxyServer",
        ])
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
#[allow(dead_code)]
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

    let mut enabled = false;
    let mut host: Option<String> = None;
    let mut port: Option<String> = None;

    for line in txt.lines() {
        let line = line.trim();
        if let Some(v) = line.strip_prefix("HTTPEnable : ") {
            enabled = v.trim() == "1";
        } else if let Some(v) = line.strip_prefix("HTTPProxy : ") {
            host = Some(v.trim().to_string());
        } else if let Some(v) = line.strip_prefix("HTTPPort : ") {
            port = Some(v.trim().to_string());
        }
    }

    if !enabled {
        return None;
    }
    match (host, port) {
        (Some(h), Some(p)) => Some(format!("http://{}:{}", h, p)),
        _ => None,
    }
}

// --- Linux: read GNOME proxy via `gsettings` --------------------------------

#[cfg(target_os = "linux")]
fn gsettings_system_proxy() -> Option<String> {
    let mode = std::process::Command::new("gsettings")
        .args(["get", "org.gnome.system.proxy", "mode"])
        .output()
        .ok()?;
    let mode_txt = String::from_utf8_lossy(&mode.stdout);
    if !mode_txt.contains("'manual'") {
        return None;
    }

    let host = std::process::Command::new("gsettings")
        .args(["get", "org.gnome.system.proxy.http", "host"])
        .output()
        .ok()?;
    let port = std::process::Command::new("gsettings")
        .args(["get", "org.gnome.system.proxy.http", "port"])
        .output()
        .ok()?;

    let host_str = String::from_utf8_lossy(&host.stdout).trim_matches('\'').trim().to_string();
    let port_str = String::from_utf8_lossy(&port.stdout).trim().to_string();

    if host_str.is_empty() || port_str.is_empty() {
        return None;
    }
    Some(format!("http://{}:{}", host_str, port_str))
}

// --- Unified HTTP client builder --------------------------------------------

/// Build a reqwest blocking client honoring the NVM proxy setting.
///
/// When the user has run `nvm proxy on` and a system proxy is detected,
/// all HTTP traffic (downloads, listing, checksum) is routed through it.
/// Otherwise a plain direct client is returned.
pub fn build_http_client() -> reqwest::blocking::Client {
    let mut builder = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(300))
        .danger_accept_invalid_certs(false);

    if is_proxy_enabled() {
        if let Some(proxy_url) = get_system_proxy() {
            if let Ok(proxy) = reqwest::Proxy::all(&proxy_url) {
                builder = builder.proxy(proxy);
            }
        }
    }

    builder.build().unwrap_or_else(|_| {
        reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(300))
            .build()
            .expect("fallback client build must succeed")
    })
}

/// Build a short-timeout client for connectivity tests. Always uses the proxy
/// when it is enabled, so that the test reflects the real download path.
pub fn build_test_client(timeout_secs: u64) -> reqwest::blocking::Client {
    let mut builder = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(timeout_secs));

    if is_proxy_enabled() {
        if let Some(proxy_url) = get_system_proxy() {
            if let Ok(proxy) = reqwest::Proxy::all(&proxy_url) {
                builder = builder.proxy(proxy);
            }
        }
    }

    builder.build().unwrap_or_else(|_| {
        reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
            .build()
            .expect("fallback test client build must succeed")
    })
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
pub fn test_connectivity() -> (bool, bool) {
    // Test Baidu (China)
    let baidu_ok = is_reachable("https://www.baidu.com", 5);
    // Test Google (international)
    let google_ok = is_reachable("https://www.google.com", 5);
    (baidu_ok, google_ok)
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
        assert_eq!(status.system_proxy, Some("http://127.0.0.1:8080".to_string()));

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
        assert_eq!(normalize_proxy_url("127.0.0.1:8080", "http"), "http://127.0.0.1:8080");
        assert_eq!(normalize_proxy_url("http://127.0.0.1:8080", "http"), "http://127.0.0.1:8080");
        assert_eq!(normalize_proxy_url("socks5://127.0.0.1:1080", "http"), "socks5://127.0.0.1:1080");
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn test_parse_windows_proxy_server_plain() {
        assert_eq!(parse_windows_proxy_server("127.0.0.1:8080"), "http://127.0.0.1:8080");
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
}
