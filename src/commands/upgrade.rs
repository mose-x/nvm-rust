//! `nvm upgrade` — self-update nvm-rust from a GitHub/Gitee release.
//!
//! Flow:
//!   1. Query the latest release tag from the GitHub API (or Gitee API).
//!   2. Compare against `CARGO_PKG_VERSION` (compiled-in current version).
//!   3. Pick the asset matching the host target triple
//!      (`nvm-<target>.tar.gz` / `nvm-<target>.zip`).
//!   4. Download + verify SHA256 against `sha256sums.txt`.
//!   5. Extract the new binary to a temp file, swap it into place, and
//!      keep the previous binary as `nvm.bak` for `--rollback`.
//!
//! The binary lives at `~/.nvm.rust/bin/nvm` (per install.sh); replacing
//! it does NOT require sudo. `/usr/local/bin/nvm` is a symlink install.sh
//! optionally creates, and points at the user-dir binary, so the swap is
//! transparent to that link.

use anyhow::{Context, Result};
use colored::Colorize;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use crate::i18n::{format_t, T};
use crate::proxy::build_http_client;
use crate::system::{get_home_dir, R_NVM_PATH};

/// GitHub repo coordinates. Hardcoded — this is nvm-rust's own self-update,
/// not a generic installer.
const REPO_OWNER: &str = "mose-x";
const REPO_NAME: &str = "nvm-rust";

/// Gitee mirror repo (must be kept in sync manually by the owner on each
/// release). `--from-gitee` switches both the API and the download URL.
const GITEE_OWNER: &str = "mose-x";
const GITEE_REPO: &str = "nvm-rust";

/// Backup file written next to the live binary before each swap. `--rollback`
/// restores from this. Overwritten on every upgrade, so it always holds the
/// immediately previous version.
const BAK_SUFFIX: &str = ".bak";

/// Entry point invoked from `main.rs` dispatch.
///
/// `check`    — only print "newer version available" (or "up to date"), exit 0.
/// `force`    — reinstall even if the latest tag equals the current version.
/// `from_gitee` — use the Gitee mirror for both API and download.
/// `from_mirror` — use a custom mirror URL prefix (e.g. `https://ghproxy.com/`).
///                  Applied to GitHub download URLs only (the API still hits
///                  github.com so the version check works without a mirror).
/// `rollback` — restore `nvm.bak` over the live binary and exit.
pub fn upgrade(
    check: bool,
    force: bool,
    from_gitee: bool,
    from_mirror: Option<String>,
    rollback: bool,
) -> Result<()> {
    let bin_path = current_binary_path()?;

    if rollback {
        return rollback_binary(&bin_path);
    }

    let client = build_http_client();

    // 1. Resolve the latest release tag + asset list.
    let (latest_tag, assets, api_source) = fetch_latest_release(&client, from_gitee)?;

    let current = env!("CARGO_PKG_VERSION");
    println!(
        "{}  {} {}",
        "ℹ".cyan().bold(),
        T("upgrade_current").cyan(),
        current.white().bold()
    );
    println!(
        "  {} {} ({})",
        T("upgrade_latest_label").dimmed(),
        latest_tag.white().bold(),
        api_source.dimmed()
    );

    // 2. Compare versions. `compare_semver` returns Greater when a > b,
    //    so latest > current means an upgrade is available.
    let newer = crate::utils::compare_semver(&latest_tag, &format!("v{}", current))
        == std::cmp::Ordering::Greater;

    if !newer && !force {
        println!(
            "{}  {}",
            "✓".green().bold(),
            T("upgrade_up_to_date").green()
        );
        return Ok(());
    }

    if check {
        if newer {
            println!(
                "{}  {}",
                "↑".yellow().bold(),
                format_t("upgrade_available", std::slice::from_ref(&latest_tag)).yellow()
            );
        } else {
            println!(
                "{}  {}",
                "✓".green().bold(),
                T("upgrade_up_to_date").green()
            );
        }
        return Ok(());
    }

    if !newer && force {
        println!(
            "{}  {}",
            "ℹ".cyan().bold(),
            T("upgrade_force_reinstall").cyan()
        );
    }

    // 3. Pick the asset matching the host target.
    //    Asset naming: `nvm-<version>-<os>-<arch>[<variant>].<ext>`
    //    e.g. nvm-2.0.0-linux-x64.tar.gz, nvm-2.0.0-linux-musl-x64.tar.gz,
    //         nvm-2.0.0-macos-arm64.tar.gz, nvm-2.0.0-windows-arm64.zip
    //    `version` is the latest release tag (without leading `v`); we
    //    don't need it for matching — we look up the asset by the
    //    `<os>-<arch>.<ext>` suffix, which is unique per release.
    let target = host_target()?;
    let ext = if cfg!(windows) { "zip" } else { "tar.gz" };
    let suffix = format!("-{}.{}", target, ext);
    let asset = assets
        .iter()
        .find(|a| a.name.ends_with(&suffix) && a.name.starts_with("nvm-"))
        .ok_or_else(|| {
            anyhow::anyhow!(
                "{}",
                format_t(
                    "upgrade_no_asset",
                    std::slice::from_ref(&format!("nvm-*{}-{}.{}", target, target, ext))
                )
            )
        })?;
    let asset_name = asset.name.clone();
    let asset_url = asset.url.clone();

    // Apply a custom mirror prefix to the GitHub download URL.
    // `--from-mirror https://ghproxy.com/` rewrites
    //   https://github.com/.../nvm-2.0.0-linux-x64.tar.gz
    // to
    //   https://ghproxy.com/https://github.com/.../nvm-...tar.gz
    // The API (version check) still hits github.com directly — mirrors
    // usually only proxy raw download URLs, not the API. `--from-gitee`
    // already gave us gitee URLs from the Gitee API; mirroring those is
    // pointless, so we skip the rewrite in that case.
    let asset_url = if let Some(prefix) = from_mirror.as_deref() {
        if !from_gitee && asset_url.starts_with("https://github.com/") {
            let trimmed = prefix.trim_end_matches('/');
            format!("{}/{}", trimmed, asset_url)
        } else {
            asset_url
        }
    } else {
        asset_url
    };

    println!("  {} {}", T("url_label").dimmed(), asset_url);

    // 4. Download to a temp file (NOT the cache: the cache lives under
    //    ~/.nvm.rust/cache and is keyed by node version names; nvm's own
    //    binary has a different lifecycle and shouldn't pollute it).
    let tmp_dir = tempfile::tempdir().context(T("upgrade_tmp_dir_failed"))?;
    let archive_path = tmp_dir.path().join(&asset_name);
    download_file(&client, &asset_url, &archive_path)?;

    // 5. Verify SHA256 against sha256sums.txt from the same release.
    //    Defends against CDN tampering and mirror corruption.
    //    Apply the same mirror rewrite as the asset URL.
    let checksums_url = assets
        .iter()
        .find(|a| a.name == "sha256sums.txt")
        .map(|a| a.url.clone())
        .map(|url| {
            if let Some(prefix) = from_mirror.as_deref() {
                if !from_gitee && url.starts_with("https://github.com/") {
                    let trimmed = prefix.trim_end_matches('/');
                    return format!("{}/{}", trimmed, url);
                }
            }
            url
        });
    if let Some(url) = checksums_url {
        verify_sha256(&client, &url, &asset_name, &archive_path)?;
    } else {
        eprintln!(
            "{}  {}",
            "⚠".yellow().bold(),
            T("upgrade_no_checksums").yellow()
        );
    }

    // 6. Extract the binary from the archive. The archive layout is just
    //    `nvm` (or `nvm.exe`) at the root — release.yml packages with
    //    `tar -czf ... -C target/.../release nvm`.
    let extracted_bin = extract_binary(&archive_path, tmp_dir.path())?;

    // 7. Swap into place: backup current → move new into bin_path.
    //    On Unix, renaming over a running binary is fine (the kernel keeps
    //    the old inode alive for the running process). On Windows the exe
    //    is locked, so we rename the old one to .bak first, then write new.
    swap_binary(&bin_path, &extracted_bin)?;

    println!(
        "{}  {} ({} → {})",
        "✓".green().bold(),
        T("upgrade_done").green().bold(),
        format!("v{}", current).dimmed(),
        latest_tag.white().bold()
    );
    println!(
        "  {} {}",
        T("tip_label").dimmed(),
        T("upgrade_restart_hint").dimmed()
    );

    Ok(())
}

/// Where the currently-running nvm binary lives on disk.
/// `std::env::current_exe` follows symlinks, so if the user installed via
/// install.sh's `/usr/local/bin/nvm` symlink, this returns the real path
/// under `~/.nvm.rust/bin/nvm` — which is what we want to replace.
fn current_binary_path() -> Result<PathBuf> {
    let exe = std::env::current_exe().context(T("upgrade_find_exe_failed"))?;
    // Canonicalize to resolve any symlinks in the parent path. The binary
    // itself may be the target of a symlink (e.g. /usr/local/bin/nvm →
    // ~/.nvm.rust/bin/nvm); current_exe already resolves that, but the
    // parent dir might have its own symlinks.
    let exe = exe.canonicalize().unwrap_or(exe);
    Ok(exe)
}

/// Host platform identifier used in release asset names.
/// Returns the `<os>-<arch>[<variant>]` portion of the asset name, e.g.
/// `linux-x64`, `linux-arm64`, `linux-musl-x64`, `macos-arm64`,
/// `windows-x64`, `windows-arm64`.
///
/// We use the user-facing `os-arch` form (matching Node.js's own naming)
/// rather than the verbose Rust target triple (`x86_64-unknown-linux-gnu`)
/// so users can tell at a glance which asset to download from the release
/// page. `std::env::consts` reports the host we're running on, so this
/// correctly handles cross-arch situations (e.g. x64 nvm running under
/// Rosetta on an Apple Silicon Mac still reports `x86_64`).
///
/// On Linux we distinguish musl from gnu via `/proc/self/cputype`-free
/// heuristic: we check `ldd --version` output for "musl". This catches
/// Alpine/distroless users who want the fully-static binary; glibc users
/// (the overwhelming majority) get the gnu build.
///
/// Unknown platforms bail instead of silently falling back to x86_64-linux,
/// which would download an unrunnable binary and brick the next `nvm` call.
fn host_target() -> Result<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => {
            // Detect musl libc (Alpine, distroless). `ldd --version` on musl
            // systems prints "musl libc"; on glibc it prints "ldd (GNU libc)".
            // Failure to detect → fall back to gnu (the common case).
            if is_musl_libc() {
                Ok("linux-musl-x64")
            } else {
                Ok("linux-x64")
            }
        }
        ("linux", "aarch64") => Ok("linux-arm64"),
        ("macos", "x86_64") => Ok("macos-x64"),
        ("macos", "aarch64") => Ok("macos-arm64"),
        ("windows", "x86_64") => Ok("windows-x64"),
        ("windows", "aarch64") => Ok("windows-arm64"),
        (os, arch) => anyhow::bail!("{}: {}-{}", T("upgrade_unsupported_platform"), os, arch),
    }
}

/// Detect musl libc on Linux by inspecting `ldd --version` output.
/// Returns `true` if the system uses musl, `false` for glibc or unknown.
fn is_musl_libc() -> bool {
    std::process::Command::new("ldd")
        .arg("--version")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.contains("musl"))
        .unwrap_or(false)
}

/// A release asset (name + download URL).
struct Asset {
    name: String,
    url: String,
}

/// Fetch the latest release tag and its asset list.
///
/// Returns `(tag, assets, source_label)` where `source_label` is a short
/// human-readable string for the "latest version" line ("GitHub" / "Gitee").
///
/// GitHub's unauthenticated API is limited to 60 requests/hour/IP. For
/// typical `nvm upgrade` usage this is plenty; users who hit the limit can
/// set `GITHUB_TOKEN` and we'll send it as a bearer token (5000/hour).
fn fetch_latest_release(
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
    let final_url = resp.url().as_str();
    let tag = final_url
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("{}", T("upgrade_no_tag")))?
        .to_string();
    // Strip a leading 'v' to get the bare version used in asset names.
    let version = tag.strip_prefix('v').unwrap_or(&tag);
    // Known asset naming: covers every platform produced by release.yml.
    // The caller matches by `-<target>.<ext>` suffix, so listing all platforms
    // here is safe -- only the host's match is downloaded.
    let targets_exts: &[(&str, &str)] = &[
        ("linux-x64", "tar.gz"),
        ("linux-musl-x64", "tar.gz"),
        ("linux-arm64", "tar.gz"),
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
    Ok((tag, assets, "GitHub (HTML)"))
}

/// Download a file, streaming to disk to avoid loading it all in memory.
/// The progress bar uses bytes because release binaries are ~5-15 MB.
fn download_file(client: &reqwest::blocking::Client, url: &str, dest: &Path) -> Result<()> {
    use indicatif::{ProgressBar, ProgressStyle};
    let resp = client
        .get(url)
        .send()
        .map_err(|e| anyhow::anyhow!("{}: {}", T("download_failed"), e))?;
    if !resp.status().is_success() {
        anyhow::bail!(
            "{}",
            format_t(
                "download_http_failed",
                std::slice::from_ref(&format!("{}", resp.status()))
            )
        );
    }
    let total = resp.content_length().unwrap_or(0);
    let pb = ProgressBar::new(total);
    pb.set_style(
        ProgressStyle::with_template("{bar:40} {bytes}/{total_bytes} ({bytes_per_sec}, {eta})")
            .unwrap_or_else(|_| ProgressStyle::default_bar())
            .progress_chars("=>-"),
    );
    let mut file = fs::File::create(dest)
        .with_context(|| format!("{}: {}", T("write_failed"), dest.display()))?;
    let mut reader = pb.wrap_read(resp);
    std::io::copy(&mut reader, &mut file).context(T("write_failed"))?;
    file.flush().context(T("write_failed"))?;
    pb.finish_with_message(T("progress_done"));
    Ok(())
}

/// Verify `archive_path`'s SHA256 against the entry for `asset_name` in
/// `sha256sums.txt`. The file format is `<hex>  <filename>` per line
/// (sha256sum's default output). Fails closed: a missing entry rejects.
fn verify_sha256(
    client: &reqwest::blocking::Client,
    checksums_url: &str,
    asset_name: &str,
    archive_path: &Path,
) -> Result<()> {
    let resp = client
        .get(checksums_url)
        .send()
        .map_err(|e| anyhow::anyhow!("{}: {}", T("upgrade_checksum_fetch_failed"), e))?;
    if !resp.status().is_success() {
        anyhow::bail!(
            "{}",
            format_t(
                "upgrade_checksum_fetch_failed",
                std::slice::from_ref(&format!("HTTP {}", resp.status()))
            )
        );
    }
    let text = resp.text().context(T("upgrade_checksum_fetch_failed"))?;
    // Find the line matching our asset. sha256sum writes `<hash>  <name>`
    // (two spaces). split_whitespace tolerates any run of whitespace.
    let expected: String = text
        .lines()
        .find_map(|line| {
            let mut parts = line.split_whitespace();
            let hash = parts.next()?;
            let name = parts.next()?;
            if name == asset_name {
                Some(hash.to_lowercase())
            } else {
                None
            }
        })
        .ok_or_else(|| {
            anyhow::anyhow!(
                "{}",
                format_t("upgrade_checksum_no_entry", &[asset_name.to_string()])
            )
        })?;

    // Compute the file's SHA256.
    let mut file = fs::File::open(archive_path)
        .with_context(|| format!("{}: {}", T("write_failed"), archive_path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = file.read(&mut buf).context(T("write_failed"))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let actual = format!("{:x}", hasher.finalize());

    if actual != expected {
        anyhow::bail!(
            "{}",
            format_t("upgrade_checksum_mismatch", &[expected.clone(), actual])
        );
    }
    println!("{}  {}", "✓".green().bold(), T("checksum_verified").green());
    Ok(())
}

/// Extract the `nvm` (or `nvm.exe`) binary from the release archive.
/// The archive contains just the binary at the root (release.yml packages
/// with `tar -C target/.../release nvm`), so we extract into `dest_dir`
/// and return the path to the binary inside it.
fn extract_binary(archive_path: &Path, dest_dir: &Path) -> Result<PathBuf> {
    fs::create_dir_all(dest_dir).context(T("cannot_create_dir"))?;
    let bin_name = if cfg!(windows) { "nvm.exe" } else { "nvm" };

    #[cfg(not(windows))]
    {
        // tar.gz extraction. The release archive is gzip-compressed tar.
        let f = fs::File::open(archive_path)
            .with_context(|| format!("{}: {}", T("write_failed"), archive_path.display()))?;
        let gz = flate2::read::GzDecoder::new(f);
        let mut tar = tar::Archive::new(gz);
        tar.unpack(dest_dir).context(T("extract_failed"))?;
    }
    #[cfg(windows)]
    {
        // zip extraction. `zip` crate is already a dependency (used by extract.rs
        // for Windows node archives). Extract just the binary entry.
        let f = fs::File::open(archive_path)
            .with_context(|| format!("{}: {}", T("write_failed"), archive_path.display()))?;
        let mut za = zip::ZipArchive::new(f).context(T("extract_failed"))?;
        for i in 0..za.len() {
            let mut entry = za.by_index(i).context(T("extract_failed"))?;
            let name = entry.name().to_string();
            if name == bin_name || name.ends_with(&format!("/{}", bin_name)) {
                let out = dest_dir.join(bin_name);
                let mut out_file = fs::File::create(&out)
                    .with_context(|| format!("{}: {}", T("write_failed"), out.display()))?;
                std::io::copy(&mut entry, &mut out_file).context(T("write_failed"))?;
                break;
            }
        }
    }

    let bin = dest_dir.join(bin_name);
    if !bin.exists() {
        anyhow::bail!(
            "{}",
            format_t("upgrade_no_binary_in_archive", &[bin_name.to_string()])
        );
    }

    // chmod +x on Unix so the swapped-in binary is executable.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&bin)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&bin, perms)?;
    }

    Ok(bin)
}

/// Swap `new_bin` into `bin_path`, backing up the current binary to
/// `bin_path.bak` first.
///
/// On Unix, `rename` over an existing file atomically replaces it, and the
/// kernel keeps the old inode alive for the running process — so the
/// currently-executing nvm keeps running until the user starts a new shell.
///
/// On Windows, the running exe is locked and cannot be overwritten. We
/// rename the current binary to `.bak` first (Windows allows renaming a
/// locked file as long as we don't delete it), then move the new one in.
fn swap_binary(bin_path: &Path, new_bin: &Path) -> Result<()> {
    // Append `.bak` to the full filename (e.g. `nvm` → `nvm.bak`,
    // `nvm.exe` → `nvm.exe.bak`). Using `with_extension` would mangle the
    // `.exe` on Windows, so we append to the file name directly.
    let bak = bin_path.parent().unwrap_or(Path::new(".")).join(format!(
        "{}{}",
        bin_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        BAK_SUFFIX
    ));

    println!(
        "  {} {} → {}",
        T("upgrade_backing_up").dimmed(),
        bin_path.display(),
        bak.display()
    );

    #[cfg(unix)]
    {
        // Rename old → bak (overwrites previous .bak). Then rename new → bin_path.
        // Both are atomic on Unix. If the old binary doesn't exist (first
        // upgrade, binary moved manually), the rename fails — skip it.
        if bin_path.exists() {
            fs::rename(bin_path, &bak)
                .with_context(|| format!("{}: {}", T("upgrade_backup_failed"), bak.display()))?;
        }
        fs::rename(new_bin, bin_path)
            .with_context(|| format!("{}: {}", T("upgrade_swap_failed"), bin_path.display()))?;
    }
    #[cfg(windows)]
    {
        // Windows: rename the locked exe to .bak (allowed), then move new in.
        if bin_path.exists() {
            // If a previous .bak exists, remove it first (rename won't overwrite).
            let _ = fs::remove_file(&bak);
            fs::rename(bin_path, &bak)
                .with_context(|| format!("{}: {}", T("upgrade_backup_failed"), bak.display()))?;
        }
        // On Windows, `fs::rename` across the same volume works for files.
        // If the new binary is on a different volume, fall back to copy+delete.
        if fs::rename(new_bin, bin_path).is_err() {
            fs::copy(new_bin, bin_path)
                .with_context(|| format!("{}: {}", T("upgrade_swap_failed"), bin_path.display()))?;
            let _ = fs::remove_file(new_bin);
        }
    }

    Ok(())
}

/// Restore `nvm.bak` over the live binary.
///
/// Fails with a clear message if no backup exists (e.g. this is the first
/// install, or the user deleted `.bak`). The restored binary becomes the
/// new live binary; the current live binary is NOT saved as a new `.bak`
/// (rollback is one-shot — repeated rollback would just toggle back to the
/// version the user just escaped from).
fn rollback_binary(bin_path: &Path) -> Result<()> {
    let bak = bin_path.parent().unwrap_or(Path::new(".")).join(format!(
        "{}{}",
        bin_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        BAK_SUFFIX
    ));

    if !bak.exists() {
        anyhow::bail!(
            "{}",
            format_t(
                "upgrade_no_backup",
                std::slice::from_ref(&bak.display().to_string())
            )
        );
    }

    println!(
        "  {} {} → {}",
        T("upgrade_restoring").dimmed(),
        bak.display(),
        bin_path.display()
    );

    #[cfg(unix)]
    {
        // Overwrite the live binary with the backup. Atomic on Unix.
        fs::rename(&bak, bin_path)
            .with_context(|| format!("{}: {}", T("upgrade_swap_failed"), bin_path.display()))?;
    }
    #[cfg(windows)]
    {
        // On Windows the running exe is locked; rename it out of the way first.
        if bin_path.exists() {
            let _ = fs::remove_file(bin_path);
        }
        fs::rename(&bak, bin_path)
            .with_context(|| format!("{}: {}", T("upgrade_swap_failed"), bin_path.display()))?;
    }

    println!(
        "{}  {}",
        "✓".green().bold(),
        T("upgrade_rollback_done").green().bold()
    );
    println!(
        "  {} {}",
        T("tip_label").dimmed(),
        T("upgrade_restart_hint").dimmed()
    );

    Ok(())
}

/// Resolve the install dir from `NVM_INSTALL_DIR` or default to
/// `~/.nvm.rust/bin`. Currently only used for the README/help text; the
/// actual swap uses `current_binary_path()` so upgrades work even if the
/// user installed to a non-default location.
#[allow(dead_code)]
fn install_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("NVM_INSTALL_DIR") {
        if !dir.is_empty() {
            return PathBuf::from(dir);
        }
    }
    PathBuf::from(get_home_dir()).join(R_NVM_PATH).join("bin")
}
