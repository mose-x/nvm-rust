use anyhow::Result;
use colored::Colorize;
use scraper::{Html, Selector};
use sha2::Digest;
use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use sysinfo::System;

use crate::i18n::{format_t, T};
use crate::proxy::build_listing_client;

pub const URI: &str = "https://nodejs.org/dist/";
pub const MIRROR_URI: &str = "https://registry.npmmirror.com/-/binary/node/";
pub const IOJS_URI: &str = "https://iojs.org/dist/";
/// The npm registry base URL. npm tarballs live here, distinct from the
/// Node.js binary mirror (`config.mirror` only mirrors `nodejs.org/dist/`).
/// The user's npm CLI uses the same registry by default (configurable via
/// `~/.npmrc`); nvm always hits this for the source-build npm fetch.
pub const NPM_REGISTRY: &str = "https://registry.npmjs.org";
pub const R_NVM_PATH: &str = ".nvm.rust";
pub const CONFIG_FILE: &str = "config.json";
pub const ALIAS_FILE: &str = "alias.json";
pub const CACHE_DIR: &str = "cache";

/// The platform PATH separator (`:` on Unix, `;` on Windows).
///
/// Centralised here so all PATH assembly goes through one constant —
/// when Windows support lands, flipping this is a single change.
pub const PATH_SEP: &str = if cfg!(windows) { ";" } else { ":" };

/// Prepend a bin directory to the current `PATH`, returning the new value.
///
/// This replaces the `format!("{}:{}", dir, env::var("PATH"))` pattern that
/// was duplicated across commands.rs and corepack.rs. Centralising it also
/// makes the eventual Windows `;` fix a one-line change.
pub fn prepend_to_path(bin_dir: &std::path::Path) -> String {
    format!(
        "{}{}{}",
        bin_dir.display(),
        PATH_SEP,
        env::var("PATH").unwrap_or_default()
    )
}

/// Resolve the directory holding a version's executables.
///
/// On Unix, Node.js tarballs extract `node`/`npm`/... into a `bin/`
/// subdirectory of the version dir. On Windows, the `.7z`/`.zip` archive
/// extracts `node.exe`/`npm.cmd`/... directly into the version dir root
/// (there is no `bin/` subdirectory). Routing every `version_dir.join("bin")`
/// through this helper makes the same code path work on both layouts —
/// previously ~20 call sites hardcoded `join("bin")`, which silently pointed
/// at a non-existent directory on Windows and broke every `nvm use`/`exec`/
///`which`/`corepack`/`reinstall-packages` invocation.
pub fn version_bin_dir(version_dir: &Path) -> PathBuf {
    if cfg!(windows) {
        version_dir.to_path_buf()
    } else {
        version_dir.join("bin")
    }
}

/// Resolve the on-disk path of an executable named `name` inside `bin_dir`,
/// accounting for Windows executable extensions.
///
/// On Unix this is simply `bin_dir.join(name)`. On Windows, Node.js ships
/// `node` as `node.exe` and `npm`/`npx`/`corepack` (and the corepack shims
/// pnpm/yarn/...) as `.cmd` wrappers — newer Node also ships `corepack.exe`.
/// Because the exact extension varies by tool and Node version, we probe the
/// candidate extensions in order (`.exe`, `.cmd`, then bare) and return the
/// first that exists on disk. If none exists we fall back to `.cmd` for the
/// known shim tools and `.exe` otherwise (the canonical Windows Node layout),
/// so both `Command::new(exe_path(...))` and `exe_path(...).exists()` behave
/// correctly whether the tool is installed or not.
pub fn exe_path(bin_dir: &Path, name: &str) -> PathBuf {
    if cfg!(not(windows)) {
        return bin_dir.join(name);
    }
    // First pass: return the first existing candidate. `.exe` is preferred
    // (the real binary) over `.cmd` (a shim that re-invokes node).
    for ext in &["exe", "cmd"] {
        let p = bin_dir.join(name).with_extension(ext);
        if p.exists() {
            return p;
        }
    }
    let bare = bin_dir.join(name);
    if bare.exists() {
        return bare;
    }
    // Nothing on disk: default to the canonical Windows extension so a
    // not-yet-installed path still carries a sensible filename in errors.
    let is_shim = matches!(
        name,
        "npm" | "npx" | "pnpm" | "pnpx" | "yarn" | "yarnpkg" | "corepack"
    );
    if is_shim {
        bin_dir.join(name).with_extension("cmd")
    } else {
        bin_dir.join(name).with_extension("exe")
    }
}

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
    // `std::env::consts::OS` is a compile-time constant ("linux", "windows",
    // "macos") and is stable across distros — unlike `System::name()` which
    // in sysinfo 0.32 returns the distro name ("Ubuntu", "Debian", …) read
    // from /etc/os-release, not the OS family. Use the constant for the
    // support check and `System::name()` only for the error message.
    match std::env::consts::OS {
        "linux" | "windows" | "macos" => {}
        _ => {
            let os_name = System::name().unwrap_or_default();
            eprintln!(
                "{}",
                format_t("unsupported_os", std::slice::from_ref(&os_name))
            );
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
    // `create_dir_all` is idempotent: it treats "already exists" as success.
    // The previous `if !dir.exists() { create_dir_all }` guard was both
    // redundant and racy (TOCTOU) — another process could remove the
    // directory between the `exists()` check and the create, or create it
    // between the check succeeding and the create being skipped (harmless).
    // Calling `create_dir_all` unconditionally closes that window.
    fs::create_dir_all(get_nvm_dir())?;
    Ok(())
}

pub fn get_cache_dir() -> PathBuf {
    get_nvm_dir().join(CACHE_DIR)
}

pub fn ensure_cache_dir() -> Result<()> {
    // See `ensure_nvm_dir` for why we don't pre-check `exists()`.
    fs::create_dir_all(get_cache_dir())?;
    Ok(())
}

pub fn get_tags(u: String) -> Vec<String> {
    let client = build_listing_client();
    let response = match client.get(&u).send() {
        Ok(r) => r,
        Err(e) => {
            eprintln!(
                "{} {}",
                "⚠".yellow().bold(),
                format_t("fetch_versions_failed", &[format!("{} ({})", u, e)])
            );
            return Vec::new();
        }
    };

    if !response.status().is_success() {
        eprintln!(
            "{} {}",
            "⚠".yellow().bold(),
            format_t(
                "fetch_versions_failed",
                &[format!("{} (HTTP {})", u, response.status())]
            )
        );
        return Vec::new();
    }

    let body = match response.text_with_charset("utf-8") {
        Ok(t) => t,
        Err(e) => {
            eprintln!(
                "{} {}",
                "⚠".yellow().bold(),
                format_t("fetch_versions_failed", &[format!("{} ({})", u, e)])
            );
            return Vec::new();
        }
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

/// Fetch `index.json` from the Node.js mirror and extract the LTS
/// codename → major version map.
///
/// `index.json` is an array of release objects; each LTS release has
/// `"lts": "<Codename>"` (capitalised, e.g. `"Krypton"`) while non-LTS
/// releases have `"lts": false`. We walk the array once, recording the
/// highest major seen for each codename (a codename spans a whole major
/// line, so every release in that line reports the same codename; taking
/// the max is defensive in case of malformed entries).
///
/// Returns an empty map on any network/parse failure — callers merge this
/// with the hardcoded fallback in `utils`/`config`, so a fetch error just
/// means "use the shipped table" (which is always correct for past LTS
/// lines and only lags behind a brand-new line until the next release).
pub fn fetch_lts_codename_map(base_url: &str) -> std::collections::BTreeMap<String, u32> {
    let index_url = format!("{}index.json", base_url);
    let client = build_listing_client();
    let mut map = std::collections::BTreeMap::new();

    let resp = match client.get(&index_url).send() {
        Ok(r) if r.status().is_success() => r,
        _ => return map,
    };
    let text = match resp.text() {
        Ok(t) => t,
        Err(_) => return map,
    };
    let json = match serde_json::from_str::<serde_json::Value>(&text) {
        Ok(v) => v,
        Err(_) => return map,
    };
    let arr = match json.as_array() {
        Some(a) => a,
        None => return map,
    };

    for entry in arr {
        // `"lts": false` (bool) for non-LTS; `"lts": "Krypton"` (string) for LTS.
        let codename = match entry.get("lts").and_then(|v| v.as_str()) {
            Some(c) => c,
            None => continue, // false or missing
        };
        let ver = match entry.get("version").and_then(|v| v.as_str()) {
            Some(v) => v,
            None => continue,
        };
        // "v24.18.0" → major 24
        let major = ver
            .trim_start_matches('v')
            .split('.')
            .next()
            .and_then(|m| m.parse::<u32>().ok());
        if let Some(maj) = major {
            let key = codename.to_lowercase();
            let entry_major = map.entry(key).or_insert(0);
            if maj > *entry_major {
                *entry_major = maj;
            }
        }
    }
    map
}

/// Fetch the `SHASUMS256.txt` for `version` from `base_url` exactly once as
/// raw bytes.
///
/// Both [`verify_checksum`] and [`verify_gpg_signature`] need this file: the
/// checksum step parses it to find the expected hash, and the GPG step writes
/// it to a temp file to check the detached `.sig` against it. Previously each
/// function fetched its own copy, so a single `nvm install` downloaded
/// `SHASUMS256.txt` twice. Fetching once here and passing the bytes to both
/// halves the metadata requests and guarantees both checks run against the
/// *same* bytes (so a mirror reformatting the file between requests can't
/// cause a checksum-pass / signature-fail mismatch).
///
/// Uses raw `bytes()` (not `text()`) so the GPG path gets the exact octets
/// the `.sig` was produced over — `text()` would UTF-8 decode and could
/// normalise line endings, invalidating the signature.
pub fn fetch_shasums(base_url: &str, version: &str) -> Result<Vec<u8>> {
    let sums_url = format!("{}{}/SHASUMS256.txt", base_url, version);
    let client = build_listing_client();
    let response = client
        .get(&sums_url)
        .send()
        .map_err(|e| anyhow::anyhow!("{}: {}", T("checksum_failed_abort"), e))?;
    if !response.status().is_success() {
        anyhow::bail!("{}: HTTP {}", T("checksum_failed_abort"), response.status());
    }
    response
        .bytes()
        .map_err(|e| anyhow::anyhow!("{}: {}", T("checksum_failed_abort"), e))
        .map(|b| b.to_vec())
}

/// Verify downloaded file against a pre-fetched `SHASUMS256.txt` body.
///
/// `sums_bytes` is the raw body returned by [`fetch_shasums`]; it is shared
/// with [`verify_gpg_signature`] so the file is downloaded only once per
/// install.
///
/// Returns `Ok(())` only when `archive_name` is listed in the sums AND its
/// SHA-256 matches the local file. Any other outcome — malformed body,
/// archive not listed, hash mismatch — is an `Err`. This is a hard security
/// boundary: a previous version returned `Ok(false)` for all of these and
/// the caller merely printed "skipped" before extracting the tarball, which
/// let a MITM drop the SHASUMS256.txt request (404) and ship a tampered
/// tarball unchallenged. Callers that genuinely want to skip must do so
/// explicitly (e.g. `--offline`).
pub fn verify_checksum(
    file_path: &std::path::Path,
    archive_name: &str,
    sums_bytes: &[u8],
) -> Result<()> {
    // SHASUMS256.txt is ASCII; from_utf8 is lossless for well-formed sums
    // files. If a mirror somehow served non-UTF-8, that's itself a red flag
    // we want to surface as an error rather than silently mis-parsing.
    let body = std::str::from_utf8(sums_bytes).map_err(|e| {
        anyhow::anyhow!(
            "{}: SHASUMS256.txt is not valid UTF-8: {e}",
            T("checksum_failed_abort")
        )
    })?;

    for line in body.lines() {
        if line.contains(archive_name) {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 && parts[1] == archive_name {
                let expected = parts[0];
                let mut file = fs::File::open(file_path)?;
                let mut hasher = sha2::Sha256::new();
                std::io::copy(&mut file, &mut hasher)?;
                let actual = format!("{:x}", hasher.finalize());
                if actual != expected {
                    anyhow::bail!(
                        "{}: {} (expected {}, got {})",
                        T("checksum_failed_abort"),
                        archive_name,
                        expected,
                        actual
                    );
                }
                return Ok(());
            }
        }
    }

    anyhow::bail!(
        "{}: {} not listed in SHASUMS256.txt",
        T("checksum_failed_abort"),
        archive_name
    );
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
///
/// Uses `hkps://` (OpenPGP over HTTPS, port 443) rather than `hkp://`
/// (plaintext, port 80/11371). A plaintext keyserver fetch lets a network
/// attacker substitute a different key for the Node.js release key, which
/// would then "verify" a tampered SHASUMS256.txt — defeating the whole point
/// of the GPG check. `hkps://` authenticates the keyserver via TLS so the
/// key bytes can't be swapped in transit. Port 443 is also more
/// firewall-friendly than the classic HKP port 11371.
const NODEJS_KEYSERVERS: &[&str] = &[
    "hkps://keyserver.ubuntu.com",
    "hkps://keys.openpgp.org",
    "hkps://keyserver.pgp.com",
];

/// Outcome of a GPG signature verification attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpgStatus {
    Verified,
    SkippedNoGpg,
    SkippedOffline,
    SkippedDisabled,
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
            // Cap each keyserver operation at 30s so an unresponsive
            // keyserver doesn't stall `nvm install` for minutes.
            .arg("--keyserver-options")
            .arg("timeout=30")
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
/// Downloads only `SHASUMS256.txt.sig` — the `SHASUMS256.txt` body itself
/// is passed in via `sums_bytes` (already fetched once by [`fetch_shasums`]
/// and shared with [`verify_checksum`], so a single install no longer
/// downloads the sums file twice). Imports the Node.js release team's
/// public key on demand and runs `gpg --verify`.
///
/// This is an additional trust layer on top of the SHA-256 checksum: it
/// defeats an attacker who replaces both the tarball and `SHASUMS256.txt`.
///
/// Returns `Ok(SkippedNoGpg)` only when the `gpg` binary is missing — the
/// caller may continue in that case (checksum verification still ran). A
/// `SkippedKeyImport` status means keyserver import failed: the signature
/// was NOT verified, so the caller should abort. `Failed` means gpg ran and
/// explicitly rejected the signature. Any network/HTTP failure fetching
/// `.sig` is an `Err` — a previous version returned `SkippedNoSig` for
/// those, which let a MITM drop the `.sig` request and ship an unsigned
/// (potentially tampered) `SHASUMS256.txt` unchallenged.
pub fn verify_gpg_signature(
    base_url: &str,
    version: &str,
    sums_bytes: &[u8],
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
    let client = build_listing_client();

    // Download the detached signature. A missing/unreachable `.sig` is a
    // hard failure — without it we cannot verify the authenticity of
    // SHASUMS256.txt, and silently continuing would let a MITM ship a
    // tampered sums file. Callers must pass `--no-gpg-verify` to bypass.
    let sig_resp = client
        .get(&sig_url)
        .send()
        .map_err(|e| anyhow::anyhow!("{}: {}", T("gpg_failed_abort"), e))?;
    if !sig_resp.status().is_success() {
        anyhow::bail!("{}: .sig HTTP {}", T("gpg_failed_abort"), sig_resp.status());
    }
    let sig_bytes = sig_resp
        .bytes()
        .map_err(|e| anyhow::anyhow!("{}: {}", T("gpg_failed_abort"), e))?
        .to_vec();

    // `sums_bytes` was already fetched once (shared with verify_checksum) —
    // reuse it instead of downloading SHASUMS256.txt a second time. Using
    // the same bytes for both checks also guarantees the checksum that
    // passed matches the file the signature covers.

    // Write both to randomly-named temp files. Using NamedTempFile (rather
    // than a predictable `nvm-rs-{pid}` name) prevents symlink attacks where
    // an attacker pre-creates a symlink at the predictable path pointing at a
    // victim file; our write would otherwise follow the symlink and clobber
    // it. NamedTempFile also removes the file on drop, so we don't need
    // manual cleanup.
    let tmp = std::env::temp_dir();
    let mut sig_file = tempfile::NamedTempFile::new_in(&tmp)?;
    sig_file.write_all(&sig_bytes)?;
    let mut sums_file = tempfile::NamedTempFile::new_in(&tmp)?;
    sums_file.write_all(sums_bytes)?;

    // Keep the temp files alive for the duration of `run_verify` (they're
    // moved in by closure capture and dropped — and thus removed — when
    // `run_verify` goes out of scope below).
    //
    // Pass the paths as `PathBuf` (AsRef<OsStr>) instead of
    // `to_string_lossy().to_string()`. `to_string_lossy` replaces non-UTF-8
    // bytes with U+FFFD, so a `TMPDIR` (or username inside it) containing
    // non-UTF-8 bytes on macOS/Linux would cause `gpg` to be invoked with a
    // wrong path and fail opaquely. `Command::arg` accepts `OsStr` natively.
    let sig_path = sig_file.path().to_path_buf();
    let sums_path = sums_file.path().to_path_buf();
    let run_verify = || {
        Command::new("gpg")
            .arg("--batch")
            .arg("--verify")
            .arg(&sig_path)
            .arg(&sums_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
    };

    // First attempt: keys may already be present in the keyring from a
    // previous run, avoiding a keyserver round-trip entirely.
    //
    // Spawn errors are propagated as `Err` rather than mapped to
    // `SkippedNoGpg`: we already passed the `gpg_available()` gate above, so
    // a spawn failure here means gpg vanished between the two calls, hit a
    // fork/resource limit, or otherwise failed in a way the user needs to
    // see — silently treating that as "gpg not installed" would mask real
    // failures and could let a tampered tarball through if the caller
    // ignored the returned status.
    let mut output = match run_verify() {
        Ok(o) if o.status.success() => return Ok(GpgStatus::Verified),
        Ok(o) => o,
        Err(e) => anyhow::bail!("{}: {}", T("gpg_failed_abort"), e),
    };

    // If verification failed purely because the public key is missing, try
    // importing the release keys once and retry. Distinguish a missing-key
    // failure from a genuine bad-signature failure so we don't report a
    // security failure for what is really a keyserver/network problem.
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let needs_keys = stderr.contains("No public key") || stderr.contains("public key not found");
    if needs_keys && import_nodejs_release_keys() {
        output = match run_verify() {
            Ok(o) => o,
            Err(e) => anyhow::bail!("{}: {}", T("gpg_failed_abort"), e),
        };
    }

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
    fn test_cache_dir() {
        let cache = get_cache_dir();
        // Should end with cache dir name
        assert!(cache.to_string_lossy().ends_with("cache"));
    }

    #[test]
    fn test_version_bin_dir_platform_layout() {
        // Locks the per-platform Node.js install layout: `bin/` subdir on
        // Unix, version dir root on Windows. A regression here would silently
        // point every Command::new(node_path) at a non-existent directory on
        // the wrong platform.
        let version_dir = Path::new("/tmp/nvm/v20.0.0");
        let bin = version_bin_dir(version_dir);
        if cfg!(windows) {
            assert_eq!(bin, version_dir);
        } else {
            assert_eq!(bin, version_dir.join("bin"));
        }
    }

    #[test]
    fn test_exe_path_unix_is_bare_join() {
        // On Unix, exe_path must return exactly bin_dir.join(name) — no
        // extension probing. This is the contract every Unix call site
        // relied on before the Windows helper was introduced.
        if cfg!(not(windows)) {
            let bin = Path::new("/tmp/nvm/v20.0.0/bin");
            assert_eq!(exe_path(bin, "node"), bin.join("node"));
            assert_eq!(exe_path(bin, "npm"), bin.join("npm"));
            assert_eq!(exe_path(bin, "corepack"), bin.join("corepack"));
        }
    }

    #[test]
    fn test_exe_path_windows_fallback_extensions() {
        // On Windows, when nothing exists on disk, exe_path must default to
        // `.cmd` for the known shim tools and `.exe` otherwise — so error
        // messages and existence checks carry a sensible filename.
        if cfg!(windows) {
            let bin = Path::new("C:\\nonexistent\\nvm\\v20.0.0");
            assert_eq!(exe_path(bin, "npm").file_name().unwrap(), "npm.cmd");
            assert_eq!(exe_path(bin, "pnpm").file_name().unwrap(), "pnpm.cmd");
            assert_eq!(exe_path(bin, "node").file_name().unwrap(), "node.exe");
        }
    }
}
