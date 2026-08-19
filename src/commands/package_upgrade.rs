//! Package-manager upgrade commands: `nvm install-npm`, `install-yarn`,
//! `install-pnpm`, plus the shared `install_latest_package_inner` helper
//! used by the `--latest-npm`/`--latest-yarn` post-install hooks.
//!
//! Also contains `download_prebuilt_npm` / `verify_npm_integrity`, used by
//! the source-build path ([`super::install_source`]) to fetch a bundled npm
//! tarball when the compiled Node.js lacks one.

use anyhow::{Context, Result};
use colored::Colorize;
use indicatif::ProgressBar;
use std::io::Write;
use std::path::Path;
use std::process::Command;

/// Sanitize proxy environment variables for corepack child processes.
///
/// Corepack (via undici) rejects proxy URLs that don't start with `http:` or
/// `https:` — including empty strings, bare `host:port`, and `socks5://`.
/// This function checks each proxy env var and returns overrides that either
/// fix the scheme (prepend `http://` if missing) or clear the var entirely
/// (empty/socks/unparseable). Returns a Vec suitable for `Command::envs()`.
fn sanitize_proxy_env_for_corepack() -> Vec<(String, String)> {
    let mut overrides = Vec::new();
    for key in &[
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "http_proxy",
        "https_proxy",
        "ALL_PROXY",
        "all_proxy",
    ] {
        if let Ok(val) = std::env::var(key) {
            if val.is_empty() {
                // Empty string → clear it (undici rejects empty proxy URLs)
                overrides.push((key.to_string(), String::new()));
            } else if val.starts_with("http://") || val.starts_with("https://") {
                // Valid HTTP(S) proxy — keep as-is
            } else if val.starts_with("socks") {
                // SOCKS proxy → undici can't use it, clear to avoid error
                overrides.push((key.to_string(), String::new()));
            } else if val.contains("://") {
                // Other scheme (ftp://, etc.) → clear
                overrides.push((key.to_string(), String::new()));
            } else {
                // Bare host:port → prepend http://
                overrides.push((key.to_string(), format!("http://{}", val)));
            }
        }
    }
    overrides
}

use crate::config::load_config;
use crate::i18n::{format_t, T};
use crate::system::{exe_path, get_nvm_dir, prepend_to_path, version_bin_dir, NPM_REGISTRY};
use crate::utils::bytes_progress_style;

use super::install::{command_failed, get_installed_version};

/// Upgrade a globally-installed package manager (`npm`, `yarn`, or `pnpm`)
/// to its latest release, using the bundled npm in `version`'s bin dir as the
/// installer.
///
/// The flow mirrors `nvm install-npm`:
///   1. Resolve + validate the target version (must be installed, must ship npm).
///   2. Print an "Upgrading X for vX.Y.Z" banner.
///   3. Run `npm install -g <package>@latest` with that version's bin on PATH.
///   4. On failure for `npm` only: retry via `npm exec --yes npm@latest --`
///      to dodge npm 10.x's self-upgrade bug. yarn/pnpm don't have this bug
///      (they install into their own dirs, npm doesn't replace itself), so
///      their first-attempt failure is a real failure and we bail.
pub(crate) fn install_latest_package_inner(version: &str, package: &str) -> Result<()> {
    let nvm_dir = get_nvm_dir();
    let resolved = version.to_string();
    let version_dir = nvm_dir.join(&resolved);
    if !version_dir.exists() {
        anyhow::bail!(
            "{}",
            format_t("not_installed", std::slice::from_ref(&resolved))
        );
    }
    let npm_path = exe_path(&version_bin_dir(&version_dir), "npm");
    if !npm_path.exists() {
        anyhow::bail!(
            "{}",
            format_t("version_no_npm", std::slice::from_ref(&resolved))
        );
    }
    // Per-package i18n keys so each tool reports its own name in messages.
    let (upgrading_key, upgraded_key, failed_key) = match package {
        "yarn" => ("upgrading_yarn", "yarn_upgraded", "yarn_upgrade_failed"),
        "pnpm" => ("upgrading_pnpm", "pnpm_upgraded", "pnpm_upgrade_failed"),
        _ => ("upgrading_npm", "npm_upgraded", "npm_upgrade_failed"),
    };
    println!(
        "  {} {}",
        "▶".cyan().bold(),
        format_t(upgrading_key, std::slice::from_ref(&resolved)).cyan()
    );
    let path_env = prepend_to_path(&version_bin_dir(&version_dir));
    // First attempt: plain `npm install -g <package>@latest`. Works for
    // yarn/pnpm (they don't replace themselves) and for npm 11+ (whose
    // reify no longer moves its own deps out from under itself).
    let pkg_spec = format!("{}@latest", package);
    let status = Command::new(&npm_path)
        .args(["install", "-g", &pkg_spec])
        .env("PATH", &path_env)
        .status()
        .context(format_t(
            "package_upgrade_spawn_failed",
            &[package.to_string()],
        ))?;
    if status.success() {
        println!("    {} {}", "✓".green().bold(), T(upgraded_key).green());
        return Ok(());
    }
    // npm-specific retry: npm 10.x has a self-upgrade bug (reify moves its
    // own node_modules, then crashes with "Cannot find module
    // 'promise-retry'" when creating bin links). Retry via `npm exec --yes
    // npm@latest --` which downloads a fresh npm to a temp dir and runs it
    // from there. yarn/pnpm don't have this bug, so we bail immediately.
    if package == "npm" {
        eprintln!(
            "  {} {}",
            "↻".yellow().bold(),
            T("npm_upgrade_retry_npx").yellow()
        );
        let status = Command::new(&npm_path)
            .args([
                "exec",
                "--yes",
                "npm@latest",
                "--",
                "install",
                "-g",
                "npm@latest",
                "--prefix",
            ])
            .arg(version_dir.display().to_string())
            .env("PATH", &path_env)
            .status()
            .context(T("npm_upgrade_failed"))?;
        if status.success() {
            println!("    {} {}", "✓".green().bold(), T("npm_upgraded").green());
            return Ok(());
        }
    }
    anyhow::bail!(command_failed(failed_key, status));
}

/// Download a prebuilt npm tarball and install it into the version's lib/node_modules.
///
/// The npm registry (npmjs.org) is distinct from the Node.js binary mirror
/// (`config.mirror`): `config.mirror` only mirrors `nodejs.org/dist/`, while
/// npm tarballs live on the npm registry. We therefore always hit
/// `registry.npmjs.org` here — the user's npm CLI itself uses the same
/// registry by default (configurable via `~/.npmrc`).
///
/// For tamper resistance we fetch the per-version registry metadata
/// (`/npm/{version}`) and verify the downloaded tarball's SHA-512 against the
/// `dist.integrity` field. If the metadata fetch fails (e.g. offline), we
/// fall back to the hardcoded tarball URL and skip verification — same
/// "Skipped vs Failed" pattern as GPG verification.
pub(crate) fn download_prebuilt_npm(version_dir: &Path, version: &str) -> Result<()> {
    let ver_num = version.trim_start_matches('v');
    let npm_tarball = format!("npm-v{}.tgz", ver_num);
    let fallback_url = format!("{}/npm/-/npm-{}.tgz", NPM_REGISTRY, ver_num);
    let npm_tar_path = get_nvm_dir().join(&npm_tarball);
    // RAII guard: removes `npm_tar_path` on drop, covering EVERY exit path
    // (download `io::copy` failure, truncation, integrity mismatch, tar
    // extraction failure, symlink failure, AND the normal success path).
    // Previously only the truncation/integrity branches and the final
    // success line cleaned up; an `io::copy` `?` left a half-written
    // `npm-v*.tgz` that the next run's `exists()` cache-hit check treated as
    // complete, silently skipping re-download and then failing at extraction
    // with a confusing "unexpected EOF". `disarm()` is called only after the
    // tarball has been successfully extracted AND wired up, so a failure
    // between staging and disarm still triggers cleanup.
    let mut tar_guard = crate::utils::FileGuard::new(&npm_tar_path);

    if !npm_tar_path.exists() {
        println!("  {} {}", "›".dimmed(), T("downloading_npm"));
        let client = crate::proxy::build_http_client();

        // Fetch registry metadata for the canonical tarball URL + integrity
        // hash. On any failure we fall back to the hardcoded URL and skip
        // integrity verification (with a warning), so a transient registry
        // outage doesn't block source-build npm installs.
        let meta_url = format!("{}/npm/{}", NPM_REGISTRY, ver_num);
        let registry_result: Option<(String, Option<String>)> = (|| {
            let resp = client.get(&meta_url).send().ok()?;
            if !resp.status().is_success() {
                return None;
            }
            let body = resp.text().ok()?;
            let json: serde_json::Value = serde_json::from_str(&body).ok()?;
            let dist = json.get("dist")?;
            let tarball = dist.get("tarball")?.as_str()?.to_string();
            let integrity = dist
                .get("integrity")
                .and_then(|i| i.as_str())
                .map(|s| s.to_string());
            Some((tarball, integrity))
        })();
        let (npm_url, expected_integrity) = match registry_result {
            Some((url, Some(int))) => (url, Some(int)),
            Some((url, None)) => {
                eprintln!(
                    "  {} {}",
                    "⚠".yellow().bold(),
                    T("npm_integrity_skipped").yellow()
                );
                (url, None)
            }
            None => (fallback_url, None),
        };

        let response = client
            .get(&npm_url)
            .send()
            .context(T("npm_tarball_download_failed"))?;
        if !response.status().is_success() {
            anyhow::bail!(
                "{}",
                format_t("npm_download_failed", std::slice::from_ref(&npm_url))
            );
        }
        let total = response.content_length().unwrap_or(0);
        let pb = ProgressBar::new(total);
        pb.set_style(bytes_progress_style());
        let mut src = pb.wrap_read(response);
        let mut dest = std::fs::File::create(&npm_tar_path)?;
        let bytes_copied =
            std::io::copy(&mut src, &mut dest).context(T("npm_tarball_write_failed"))?;
        pb.finish_with_message(T("progress_done"));

        // Detect truncated downloads: if the server advertised a content
        // length and we got fewer bytes, the connection dropped mid-transfer.
        // Without this check, `tar xzf` below would fail with a confusing
        // "unexpected EOF" instead of a clear "truncated" message.
        if total > 0 && bytes_copied < total {
            anyhow::bail!("{}", T("npm_download_truncated"));
        }
        // When the server omits Content-Length (chunked-only responses, some
        // proxies), the above length check is a no-op. If we also have no
        // registry integrity hash to verify against, a truncated tarball would
        // only surface later as a cryptic `tar` error. Warn so the user can
        // distinguish a download glitch from a real extraction failure.
        if total == 0 && expected_integrity.is_none() {
            eprintln!(
                "{} {}",
                "⚠".yellow().bold(),
                T("npm_no_length_no_integrity").yellow()
            );
        }

        // Verify SHA-512 integrity against the registry's `dist.integrity`.
        // This catches a compromised CDN cache serving a tampered tarball at
        // the legitimate URL — TLS alone doesn't protect against that.
        if let Some(integrity) = expected_integrity {
            if verify_npm_integrity(&npm_tar_path, &integrity).is_err() {
                anyhow::bail!("{}", T("npm_integrity_failed"));
            }
        }
    }

    // Extract npm tarball into lib/node_modules.
    // Requires `tar` with `--strip-components=1` (GNU tar; Windows 10 build
    // 17063+ ships bsdtar which supports this flag). On older Windows the
    // prebuilt-binary install path is used instead (npm ships bundled).
    let node_modules = version_dir.join("lib").join("node_modules");
    std::fs::create_dir_all(&node_modules)?;

    let status = Command::new("tar")
        .arg("xzf")
        .arg(&npm_tar_path)
        .arg("-C")
        .arg(&node_modules)
        .arg("--strip-components=1")
        .status()
        .context(T("npm_extract_failed"))?;
    if !status.success() {
        anyhow::bail!("{}", T("npm_extract_failed"));
    }

    // Wire up the `npm` executable so it lands on PATH. The tarball ships
    // `bin/npm` (a JS launcher); we symlink (Unix) or copy (Windows) it into
    // the version's `bin/` dir alongside `node`, so `nvm use <ver>` exposes
    // npm immediately.
    //
    // These used to be `.ok()` — silently swallowing a read-only `bin/`,
    // a Windows AV file lock, or a full disk. The result was "npm installed!"
    // with no npm on PATH. Propagate the error instead so the failure is
    // visible and the install is reported as failed.
    let npm_bin_src = node_modules.join("bin").join("npm");
    let npm_bin_dst = version_dir.join("bin").join("npm");
    let npm_bin_dst_parent = version_dir.join("bin");
    std::fs::create_dir_all(&npm_bin_dst_parent)?;
    #[cfg(unix)]
    std::os::unix::fs::symlink(&npm_bin_src, &npm_bin_dst)
        .with_context(|| format!("failed to symlink npm bin at {}", npm_bin_dst.display()))?;
    #[cfg(windows)]
    std::fs::copy(&npm_bin_src, &npm_bin_dst)
        .map(|_| ())
        .with_context(|| format!("failed to copy npm bin at {}", npm_bin_dst.display()))?;

    // Best-effort cleanup of the downloaded tarball; a failure here doesn't
    // invalidate the install, so don't surface it as an error.
    let _ = std::fs::remove_file(&npm_tar_path);
    // The tarball has been extracted and npm wired up — disarm the guard so
    // its `Drop` doesn't issue a redundant `remove_file` on an already-removed
    // path. Any `?` early-return above leaves the guard armed, so a partial
    // download / extraction failure still cleans up.
    tar_guard.disarm();
    Ok(())
}

/// Verify a downloaded npm tarball against the registry's `dist.integrity`
/// field. The field is in the Subresource Integrity format: `<algo>-<b64>`.
/// We support `sha512` (the algorithm npm uses for all current releases).
pub(crate) fn verify_npm_integrity(file_path: &Path, integrity: &str) -> Result<()> {
    let (algo, expected_b64) = integrity
        .split_once('-')
        .ok_or_else(|| anyhow::anyhow!("malformed integrity field"))?;
    if algo != "sha512" {
        anyhow::bail!("unsupported integrity algorithm: {} (only sha512)", algo);
    }
    use base64::Engine;
    let expected = base64::engine::general_purpose::STANDARD
        .decode(expected_b64)
        .map_err(|e| anyhow::anyhow!("invalid base64 in integrity field: {}", e))?;

    use sha2::Digest;
    let mut file = std::fs::File::open(file_path)?;
    let mut hasher = sha2::Sha512::new();
    std::io::copy(&mut file, &mut hasher)?;
    let actual = hasher.finalize();

    if actual.as_slice() != expected.as_slice() {
        anyhow::bail!("sha512 mismatch");
    }
    Ok(())
}

/// Resolve the target version for a package-upgrade command. With no
/// argument, act on the *current* version (matches nvm-sh behavior). Only
/// fall back to `default` when there's no current set — `nvm install-latest-<pkg>`
/// with no current and no default is a setup error.
fn resolve_install_target(version: Option<&str>) -> Result<String> {
    let target = match version {
        Some(v) => v.to_string(),
        None => match super::get_current_version()? {
            Some(v) => v,
            None => {
                let config = load_config()?;
                match config.default_version {
                    Some(v) => v,
                    None => anyhow::bail!("{}", T("no_current_version_set")),
                }
            }
        },
    };
    get_installed_version(&target)
}

pub fn install_latest_npm(version: Option<&str>) -> Result<()> {
    let resolved = resolve_install_target(version)?;
    install_latest_package_inner(&resolved, "npm")
}

pub fn install_latest_yarn(version: Option<&str>) -> Result<()> {
    let resolved = resolve_install_target(version)?;
    install_latest_package_inner(&resolved, "yarn")
}

/// Install the latest pnpm for a version via corepack.
///
/// Uses `corepack prepare pnpm@latest --activate` instead of
/// `npm install -g pnpm@latest` to avoid pnpm 10+'s
/// `@pnpm/exe` native binary verification error. Falls back to
/// `install_latest_package_inner` (npm install) if corepack is not
/// bundled with the version or `corepack prepare` fails.
pub(crate) fn install_latest_pnpm_via_corepack(version: &str) -> Result<()> {
    let nvm_dir = get_nvm_dir();
    let resolved = version.to_string();
    let version_dir = nvm_dir.join(&resolved);
    if !version_dir.exists() {
        anyhow::bail!(
            "{}",
            format_t("not_installed", std::slice::from_ref(&resolved))
        );
    }
    let bin_dir = version_bin_dir(&version_dir);
    let corepack_path = exe_path(&bin_dir, "corepack");
    if !corepack_path.exists() {
        return install_latest_package_inner(version, "pnpm");
    }

    println!(
        "  {} {}",
        "▶".cyan().bold(),
        format_t("upgrading_pnpm", std::slice::from_ref(&resolved)).cyan()
    );

    let path_env = prepend_to_path(&bin_dir);
    // Sanitize proxy env vars for corepack: undici's ProxyAgent rejects
    // empty/malformed values (e.g. `https_proxy=""` or `socks5://...`).
    // If nvm's proxy is off, clear them entirely; if on, ensure http:// prefix.
    let proxy_env_overrides = sanitize_proxy_env_for_corepack();
    if let Err(e) = crate::corepack::corepack_enable(Some(version)) {
        eprintln!(
            "  {} corepack enable failed: {} — continuing with prepare",
            "⚠".yellow().bold(),
            e
        );
    }

    let status = Command::new(&corepack_path)
        .args(["prepare", "pnpm@latest", "--activate"])
        .env("PATH", &path_env)
        .env("COREPACK_ENABLE_DOWNLOAD_PROMPT", "0")
        .envs(proxy_env_overrides)
        .status()
        .context(format_t(
            "package_upgrade_spawn_failed",
            &["pnpm".to_string()],
        ))?;

    if status.success() {
        // Pre-trigger corepack's lazy download so the user isn't prompted
        // ("Corepack is about to download ... [Y/n]") on first `pnpm` use.
        // Pipe "Y\n" to stdin to auto-confirm.
        if let Ok(mut child) = Command::new(&corepack_path)
            .args(["pnpm", "--version"])
            .env("PATH", &path_env)
            .env("COREPACK_ENABLE_DOWNLOAD_PROMPT", "0")
            .envs(sanitize_proxy_env_for_corepack())
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            if let Some(stdin) = child.stdin.as_mut() {
                let _ = stdin.write_all(b"Y\n");
            }
            let _ = child.wait();
        }
        println!("    {} {}", "✓".green().bold(), T("pnpm_upgraded").green());
        return Ok(());
    }

    // Fallback to npm install if corepack prepare fails
    eprintln!(
        "  {} {}",
        "↻".yellow().bold(),
        T("pnpm_corepack_fallback").yellow()
    );
    install_latest_package_inner(version, "pnpm")
}

pub fn install_latest_pnpm(version: Option<&str>) -> Result<()> {
    let resolved = resolve_install_target(version)?;
    install_latest_pnpm_via_corepack(&resolved)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_proxy_clears_empty_string() {
        let _guard = crate::system::ENV_TESTS_MUTEX
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        std::env::set_var("http_proxy", "");
        let overrides = sanitize_proxy_env_for_corepack();
        let http_proxy = overrides
            .iter()
            .find(|(k, _)| k == "http_proxy")
            .map(|(_, v)| v.as_str())
            .unwrap_or("UNSET");
        assert_eq!(
            http_proxy, "",
            "empty proxy should be cleared to empty string"
        );
        std::env::remove_var("http_proxy");
    }

    #[test]
    fn test_sanitize_proxy_clears_socks() {
        let _guard = crate::system::ENV_TESTS_MUTEX
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        std::env::set_var("https_proxy", "socks5://127.0.0.1:7890");
        let overrides = sanitize_proxy_env_for_corepack();
        let https = overrides
            .iter()
            .find(|(k, _)| k == "https_proxy")
            .map(|(_, v)| v.as_str())
            .unwrap_or("UNSET");
        assert_eq!(https, "", "socks5 proxy should be cleared");
        std::env::remove_var("https_proxy");
    }

    #[test]
    fn test_sanitize_proxy_prepends_http_to_bare_host() {
        let _guard = crate::system::ENV_TESTS_MUTEX
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        std::env::set_var("HTTP_PROXY", "127.0.0.1:7890");
        let overrides = sanitize_proxy_env_for_corepack();
        let http = overrides
            .iter()
            .find(|(k, _)| k == "HTTP_PROXY")
            .map(|(_, v)| v.as_str())
            .unwrap_or("UNSET");
        assert_eq!(
            http, "http://127.0.0.1:7890",
            "bare host should get http:// prefix"
        );
        std::env::remove_var("HTTP_PROXY");
    }

    #[test]
    fn test_sanitize_proxy_keeps_valid_http() {
        let _guard = crate::system::ENV_TESTS_MUTEX
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        std::env::set_var("HTTPS_PROXY", "http://127.0.0.1:7890");
        let overrides = sanitize_proxy_env_for_corepack();
        // Valid http:// proxy should NOT appear in overrides (kept as-is, no override needed)
        let has_override = overrides.iter().any(|(k, _)| k == "HTTPS_PROXY");
        assert!(
            !has_override,
            "valid http:// proxy should not need override"
        );
        std::env::remove_var("HTTPS_PROXY");
    }

    #[test]
    fn test_sanitize_proxy_no_env_returns_empty() {
        let _guard = crate::system::ENV_TESTS_MUTEX
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        std::env::remove_var("HTTP_PROXY");
        std::env::remove_var("HTTPS_PROXY");
        std::env::remove_var("http_proxy");
        std::env::remove_var("https_proxy");
        std::env::remove_var("ALL_PROXY");
        std::env::remove_var("all_proxy");
        let overrides = sanitize_proxy_env_for_corepack();
        assert!(
            overrides.is_empty(),
            "no proxy env vars should return empty overrides"
        );
    }

    /// Build a real SRI `sha512-<base64>` integrity string for `contents`
    /// so the verify_npm_integrity tests don't depend on a hardcoded hash.
    fn make_integrity(contents: &str) -> String {
        use base64::Engine;
        use sha2::Digest;
        let mut hasher = sha2::Sha512::new();
        hasher.update(contents.as_bytes());
        let digest = hasher.finalize();
        format!(
            "sha512-{}",
            base64::engine::general_purpose::STANDARD.encode(digest)
        )
    }

    #[test]
    fn test_verify_npm_integrity_accepts_correct_hash() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("npm.tgz");
        let contents = "fake npm tarball contents";
        std::fs::write(&file, contents).expect("write");
        let integrity = make_integrity(contents);
        verify_npm_integrity(&file, &integrity).expect("matching integrity should verify");
    }

    #[test]
    fn test_verify_npm_integrity_rejects_tampered_file() {
        // The download flow computes the hash over the bytes on disk. If the
        // tarball was truncated or replaced after the metadata fetch, the
        // hash must not match — this is the tamper-detection guarantee.
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("npm.tgz");
        std::fs::write(&file, "original contents").expect("write");
        let integrity = make_integrity("tampered contents");
        let err = verify_npm_integrity(&file, &integrity).expect_err("mismatched hash should fail");
        assert!(
            err.to_string().contains("mismatch"),
            "expected mismatch error, got: {}",
            err
        );
    }

    #[test]
    fn test_verify_npm_integrity_rejects_wrong_algorithm() {
        // npm registry only ships sha512 SRI; sha256 or others must be
        // rejected so we never silently skip verification for an algo we
        // can't check.
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("npm.tgz");
        std::fs::write(&file, "x").expect("write");
        let err =
            verify_npm_integrity(&file, "sha256-AAAA").expect_err("non-sha512 algo should fail");
        assert!(err.to_string().contains("sha512"));
    }

    #[test]
    fn test_verify_npm_integrity_rejects_malformed_field() {
        // No `-` separator → can't split algo from hash. Must fail rather
        // than panic on a None unwrap.
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("npm.tgz");
        std::fs::write(&file, "x").expect("write");
        verify_npm_integrity(&file, "noseparator").expect_err("malformed integrity should fail");
    }

    #[test]
    fn test_verify_npm_integrity_rejects_invalid_base64() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("npm.tgz");
        std::fs::write(&file, "x").expect("write");
        verify_npm_integrity(&file, "sha512-!!!not-base64!!!")
            .expect_err("invalid base64 should fail");
    }
}
