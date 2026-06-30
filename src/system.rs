use anyhow::Result;
use scraper::{Html, Selector};
use sha2::Digest;
use std::env;
use std::fs;
use std::path::PathBuf;
use sysinfo::System;

use crate::proxy::build_http_client;

pub const URI: &str = "https://nodejs.org/dist/";
pub const MIRROR_URI: &str = "https://registry.npmmirror.com/-/binary/node/";
pub const IOJS_URI: &str = "https://iojs.org/dist/";
pub const R_NVM_PATH: &str = ".nvm.rust";
pub const CONFIG_FILE: &str = "config.json";
pub const ALIAS_FILE: &str = "alias.json";
pub const CACHE_DIR: &str = "cache";

/// Get the user home directory cross-platform.
///
/// Windows does not set `HOME`; it uses `USERPROFILE` instead (e.g. `C:\Users\name`).
/// Returns "." as a last resort so callers never panic.
pub fn get_home_dir() -> String {
    for var in &["HOME", "USERPROFILE"] {
        if let Ok(val) = env::var(var) {
            if !val.is_empty() {
                return val;
            }
        }
    }
    ".".to_string()
}

pub fn os_check() {
    let os_name = System::name().unwrap_or_default();

    match os_name.as_str() {
        "Windows" | "Linux" | "Ubuntu" | "Darwin" => {}
        _ => {
            eprintln!("Unsupported operating system: {}", os_name);
            std::process::exit(1);
        }
    }
}

/// Get NVM directory, NVM_DIR env var takes priority
pub fn get_nvm_dir() -> PathBuf {
    if let Ok(dir) = env::var("NVM_DIR") {
        if !dir.is_empty() {
            return PathBuf::from(dir);
        }
    }
    PathBuf::from(get_home_dir()).join(R_NVM_PATH)
}

pub fn ensure_nvm_dir() -> Result<()> {
    let nvm_dir = get_nvm_dir();
    if !nvm_dir.exists() {
        fs::create_dir_all(&nvm_dir)?;
    }
    Ok(())
}

pub fn get_cache_dir() -> PathBuf {
    get_nvm_dir().join(CACHE_DIR)
}

pub fn ensure_cache_dir() -> Result<()> {
    let cache_dir = get_cache_dir();
    if !cache_dir.exists() {
        fs::create_dir_all(&cache_dir)?;
    }
    Ok(())
}

pub fn get_tags(u: String) -> Vec<String> {
    let client = build_http_client();
    let response = match client.get(&u).send() {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };

    if !response.status().is_success() {
        return Vec::new();
    }

    let body = match response.text_with_charset("utf-8") {
        Ok(t) => t,
        Err(_) => return Vec::new(),
    };

    let fragment = Html::parse_document(&body);
    let selector = match Selector::parse("body pre a") {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };

    fragment
        .select(&selector)
        .map(|element| element.inner_html())
        .collect()
}

/// Verify downloaded file against SHASUMS256.txt
pub fn verify_checksum(file_path: &std::path::Path, archive_name: &str, base_url: &str, version: &str) -> Result<bool> {
    let sums_url = format!("{}{}/SHASUMS256.txt", base_url, version);
    let client = build_http_client();
    let response = match client.get(&sums_url).send() {
        Ok(r) => r,
        Err(_) => return Ok(false),
    };

    if !response.status().is_success() {
        return Ok(false);
    }

    let body = response.text().unwrap_or_default();

    for line in body.lines() {
        if line.contains(archive_name) {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 && parts[1] == archive_name {
                let expected = parts[0];
                let mut file = fs::File::open(file_path)?;
                let mut hasher = sha2::Sha256::new();
                std::io::copy(&mut file, &mut hasher)?;
                let actual = format!("{:x}", hasher.finalize());
                return Ok(actual == expected);
            }
        }
    }

    Ok(false)
}

#[cfg(target_os = "windows")]
pub fn os_suffix() -> &'static str {
    "win-x64.7z"
}

#[cfg(target_os = "linux")]
pub fn os_suffix() -> &'static str {
    if cfg!(target_arch = "aarch64") {
        "linux-arm64.tar.xz"
    } else {
        "linux-x64.tar.xz"
    }
}

#[cfg(target_os = "macos")]
pub fn os_suffix() -> &'static str {
    if cfg!(target_arch = "aarch64") {
        "darwin-arm64.tar.xz"
    } else {
        "darwin-x64.tar.xz"
    }
}

#[cfg(target_os = "windows")]
pub fn os_type_name() -> &'static str {
    "win"
}

#[cfg(target_os = "linux")]
pub fn os_type_name() -> &'static str {
    if cfg!(target_arch = "aarch64") {
        "linux-arm64"
    } else {
        "linux"
    }
}

#[cfg(target_os = "macos")]
pub fn os_type_name() -> &'static str {
    if cfg!(target_arch = "aarch64") {
        "darwin-arm64"
    } else {
        "darwin"
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_uris_defined() {
        assert!(!URI.is_empty());
        assert!(!MIRROR_URI.is_empty());
        assert!(!IOJS_URI.is_empty());
        assert!(URI.starts_with("https://"));
        assert!(IOJS_URI.starts_with("https://"));
    }

    #[test]
    fn test_os_suffix_not_empty() {
        let suffix = os_suffix();
        assert!(!suffix.is_empty());
        assert!(suffix.contains("linux") || suffix.contains("darwin") || suffix.contains("win"));
    }

    #[test]
    fn test_os_type_name_not_empty() {
        let name = os_type_name();
        assert!(!name.is_empty());
        assert!(name.contains("linux") || name.contains("darwin") || name.contains("win"));
    }

    #[test]
    fn test_cache_dir() {
        let cache = get_cache_dir();
        // Should end with cache dir name
        assert!(cache.to_string_lossy().ends_with("cache"));
    }
}
