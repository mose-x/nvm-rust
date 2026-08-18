//! Download + SHA256 verification helpers for `nvm upgrade`.
//!
//! Extracted from `upgrade.rs` as part of P1-7 module refactoring. All
//! items are re-exported through `upgrade::*` so existing call sites are
//! unchanged.

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{Read, Write};
use std::path::Path;

use crate::i18n::{format_t, T};

/// Download a file, streaming to disk to avoid loading it all in memory.
/// The progress bar uses bytes because release binaries are ~5-15 MB.
pub(crate) fn download_file(
    client: &reqwest::blocking::Client,
    url: &str,
    dest: &Path,
) -> Result<()> {
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

/// Find the expected SHA256 hash for `asset_name` in a `sha256sums.txt` body.
/// Format: `<hex>  <filename>` per line. Returns `None` if no entry matches.
fn find_checksum_for_asset(text: &str, asset_name: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let mut parts = line.split_whitespace();
        let hash = parts.next()?;
        let name = parts.next()?;
        if name == asset_name {
            Some(hash.to_lowercase())
        } else {
            None
        }
    })
}

/// Verify `archive_path`'s SHA256 against the entry for `asset_name` in
/// `sha256sums.txt`. The file format is `<hex>  <filename>` per line
/// (sha256sum's default output). Fails closed: a missing entry rejects.
pub(crate) fn verify_sha256(
    client: &reqwest::blocking::Client,
    checksums_url: &str,
    asset_name: &str,
    archive_path: &Path,
) -> Result<()> {
    use colored::Colorize;
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
    // sha256sum writes `<hash>  <name>` per line; `find_checksum_for_asset`
    // tolerates any whitespace run between them.
    let expected: String = find_checksum_for_asset(&text, asset_name).ok_or_else(|| {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_checksum_for_asset_normal() {
        let text = "\
abc123  nvm-2.0.0-linux-x64.tar.gz
def456  nvm-2.0.0-macos-arm64.tar.gz
789abc  sha256sums.txt
";
        assert_eq!(
            find_checksum_for_asset(text, "nvm-2.0.0-linux-x64.tar.gz").as_deref(),
            Some("abc123")
        );
        assert_eq!(
            find_checksum_for_asset(text, "nvm-2.0.0-macos-arm64.tar.gz").as_deref(),
            Some("def456")
        );
    }

    #[test]
    fn test_find_checksum_for_asset_normalizes_case() {
        // sha256sum prints lowercase, but be tolerant of uppercase input.
        let text = "ABCDEF  nvm-2.0.0-linux-x64.tar.gz\n";
        assert_eq!(
            find_checksum_for_asset(text, "nvm-2.0.0-linux-x64.tar.gz").as_deref(),
            Some("abcdef")
        );
    }

    #[test]
    fn test_find_checksum_for_asset_missing_entry() {
        let text = "abc123  some-other-file.tar.gz\n";
        assert_eq!(
            find_checksum_for_asset(text, "nvm-2.0.0-linux-x64.tar.gz"),
            None
        );
    }

    #[test]
    fn test_find_checksum_for_asset_tolerates_extra_whitespace() {
        // sha256sum uses two spaces; some tools may use tabs or more spaces.
        let text = "abc123\t\tnvm-2.0.0-linux-x64.tar.gz\n";
        assert_eq!(
            find_checksum_for_asset(text, "nvm-2.0.0-linux-x64.tar.gz").as_deref(),
            Some("abc123")
        );
    }

    #[test]
    fn test_find_checksum_for_asset_empty_text() {
        assert_eq!(
            find_checksum_for_asset("", "nvm-2.0.0-linux-x64.tar.gz"),
            None
        );
    }

    #[test]
    fn test_find_checksum_for_asset_skips_malformed_lines() {
        // Lines without a hash+name pair are ignored, not panicked.
        let text = "\
garbage line with no hash
abc123  nvm-2.0.0-linux-x64.tar.gz
justoneword
";
        assert_eq!(
            find_checksum_for_asset(text, "nvm-2.0.0-linux-x64.tar.gz").as_deref(),
            Some("abc123")
        );
    }
}
