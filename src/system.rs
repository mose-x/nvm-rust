use anyhow::Result;
use scraper::{Html, Selector};
use sha2::Digest;
use std::env;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use sysinfo::System;

use crate::i18n::format_t;
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
            eprintln!("{}", format_t("unsupported_os", std::slice::from_ref(&os_name)));
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

/// Node.js release team GPG key IDs used to verify `SHASUMS256.txt.sig`.
/// Mirrors the list nvm-sh imports; any present key is sufficient to verify
/// a release signature. The list is long on purpose so that older and newer
/// releases alike can be verified without a keyserver round-trip per release.
const NODEJS_RELEASE_KEY_IDS: &[&str] = &[
    "94AE36675C464D64BAFA68DD7434390BDBE9B9C5",
    "74F12602B6F1C4E913FAA37AD3A89613643B6201",
    "71DCFD284A79C3B38668286BC97EC7A07EDE3FC1",
    "8FCCA13FEF1D0C2E91008E09770F7A9A5AE15600",
    "C4F0DFFF4E3C283FDFCDFB08576E6C61A1A1B1FE",
    "DD8F2338BAE7501E3DD5AC78C273792F7D83545D",
    "B9AE9905FFD7803F25714661B63B535A4C206CA9",
    "77984A986EBC2AA786BC0F66B01FBB92821C587A",
    "890C08DB8579162FEE0DF9DB8BEAB4DFCF555EF4",
    "C82FA3AE1CBEDC6BE46B9360C43CEC45C17AB93C",
];

/// Keyservers tried (in order) when importing the Node.js release keys.
const NODEJS_KEYSERVERS: &[&str] = &[
    "hkp://keyserver.ubuntu.com:80",
    "hkp://keyserver.ubuntu.com:443",
    "hkp://keyserver.pgp.com:80",
];

/// Outcome of a GPG signature verification attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpgStatus {
    Verified,
    SkippedNoGpg,
    SkippedOffline,
    SkippedDisabled,
    SkippedNoSig,
    SkippedKeyImport,
    Failed,
}

/// Check whether the `gpg` binary is available on PATH and functional.
fn gpg_available() -> bool {
    Command::new("gpg")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Import the Node.js release signing keys into the user's keyring.
/// Returns `true` if the import command reported success (which is also the
/// case when the keys are already present). Best-effort: callers treat a
/// `false` result as "try verifying anyway, the keys may already be there".
fn import_nodejs_release_keys() -> bool {
    for keyserver in NODEJS_KEYSERVERS {
        let status = Command::new("gpg")
            .arg("--batch")
            .arg("--keyserver")
            .arg(keyserver)
            .arg("--recv-keys")
            .args(NODEJS_RELEASE_KEY_IDS)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        if let Ok(s) = status {
            if s.success() {
                return true;
            }
        }
    }
    false
}

/// Verify the GPG signature of `SHASUMS256.txt` for a Node.js release.
///
/// Downloads `SHASUMS256.txt.sig` alongside `SHASUMS256.txt`, imports the
/// Node.js release team's public key on demand, and runs `gpg --verify`.
/// This is an additional trust layer on top of the SHA-256 checksum: it
/// defeats an attacker who replaces both the tarball and `SHASUMS256.txt`.
///
/// The function never aborts an install. On any non-success it returns a
/// `Skipped*` / `Failed` status so the caller can print a warning and
/// continue — preserving the existing "checksum skipped" behavior while
/// adding signature verification when gpg is available.
pub fn verify_gpg_signature(
    base_url: &str,
    version: &str,
    no_gpg_verify: bool,
    offline: bool,
) -> Result<GpgStatus> {
    if no_gpg_verify {
        return Ok(GpgStatus::SkippedDisabled);
    }
    if offline {
        return Ok(GpgStatus::SkippedOffline);
    }
    if !gpg_available() {
        return Ok(GpgStatus::SkippedNoGpg);
    }

    let sig_url = format!("{}{}/SHASUMS256.txt.sig", base_url, version);
    let sums_url = format!("{}{}/SHASUMS256.txt", base_url, version);
    let client = build_http_client();

    // Download the detached signature. Mirrors occasionally omit the .sig
    // file, in which case we skip rather than fail the install.
    let sig_resp = match client.get(&sig_url).send() {
        Ok(r) => r,
        Err(_) => return Ok(GpgStatus::SkippedNoSig),
    };
    if !sig_resp.status().is_success() {
        return Ok(GpgStatus::SkippedNoSig);
    }
    let sig_bytes = match sig_resp.bytes() {
        Ok(b) => b.to_vec(),
        Err(_) => return Ok(GpgStatus::SkippedNoSig),
    };

    // Download a fresh copy of SHASUMS256.txt that exactly matches the .sig
    // (mirrors may reformat the text, which would invalidate the signature).
    let sums_resp = match client.get(&sums_url).send() {
        Ok(r) => r,
        Err(_) => return Ok(GpgStatus::SkippedNoSig),
    };
    if !sums_resp.status().is_success() {
        return Ok(GpgStatus::SkippedNoSig);
    }
    let sums_bytes = match sums_resp.bytes() {
        Ok(b) => b.to_vec(),
        Err(_) => return Ok(GpgStatus::SkippedNoSig),
    };

    // Write both to temp files named per-process to avoid collisions with
    // concurrent installs.
    let tmp = std::env::temp_dir();
    let pid = std::process::id();
    let sig_path = tmp.join(format!("nvm-rs-{}.SHASUMS256.txt.sig", pid));
    let sums_path = tmp.join(format!("nvm-rs-{}.SHASUMS256.txt", pid));
    {
        let mut f = fs::File::create(&sig_path)?;
        f.write_all(&sig_bytes)?;
        let mut f = fs::File::create(&sums_path)?;
        f.write_all(&sums_bytes)?;
    }

    let sig_str = sig_path.to_string_lossy().to_string();
    let sums_str = sums_path.to_string_lossy().to_string();
    let run_verify = || {
        Command::new("gpg")
            .arg("--batch")
            .arg("--verify")
            .arg(&sig_str)
            .arg(&sums_str)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
    };

    // First attempt: keys may already be present in the keyring from a
    // previous run, avoiding a keyserver round-trip entirely.
    let mut output = match run_verify() {
        Ok(o) if o.status.success() => {
            fs::remove_file(&sig_path).ok();
            fs::remove_file(&sums_path).ok();
            return Ok(GpgStatus::Verified);
        }
        Ok(o) => o,
        Err(_) => {
            fs::remove_file(&sig_path).ok();
            fs::remove_file(&sums_path).ok();
            return Ok(GpgStatus::SkippedNoGpg);
        }
    };

    // If verification failed purely because the public key is missing, try
    // importing the release keys once and retry. Distinguish a missing-key
    // failure from a genuine bad-signature failure so we don't report a
    // security failure for what is really a keyserver/network problem.
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let needs_keys =
        stderr.contains("No public key") || stderr.contains("public key not found");
    if needs_keys
        && import_nodejs_release_keys() {
            output = match run_verify() {
                Ok(o) => o,
                Err(_) => {
                    fs::remove_file(&sig_path).ok();
                    fs::remove_file(&sums_path).ok();
                    return Ok(GpgStatus::SkippedNoGpg);
                }
            };
        }

    fs::remove_file(&sig_path).ok();
    fs::remove_file(&sums_path).ok();

    if output.status.success() {
        Ok(GpgStatus::Verified)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        if stderr.contains("No public key") || stderr.contains("public key not found") {
            Ok(GpgStatus::SkippedKeyImport)
        } else {
            Ok(GpgStatus::Failed)
        }
    }
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
