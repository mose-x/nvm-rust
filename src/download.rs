use anyhow::{Context, Result};
use indicatif::{ProgressBar, ProgressStyle};
use std::fs::{self, File, OpenOptions};
use std::io::{copy, Write};
use std::path::{Path, PathBuf};

use crate::proxy::build_http_client;
use crate::system::{ensure_cache_dir, get_cache_dir};

/// Suffix used for partial downloads. A file is only considered complete
/// (and therefore cache-hit eligible) once it has been renamed from
/// `<name>.part` to `<name>`. This is what makes resume safe: a truncated
/// or half-downloaded file never satisfies the `cache_path.exists()` check.
const PART_SUFFIX: &str = ".part";

/// Download a file to cache dir, returning the local cache path.
/// If already cached (complete), just returns the cached path.
///
/// Supports resume: if a `<filename>.part` exists, an HTTP Range request is
/// sent to continue from the existing byte offset. Servers that do not honor
/// Range requests cause a transparent fallback to a full re-download. The
/// final file is only visible to callers once the download finishes and the
/// `.part` file is atomically renamed to its final name.
pub fn download_to_cache(url: &str, filename: &str) -> Result<PathBuf> {
    let cache_dir = get_cache_dir();
    ensure_cache_dir()?;
    let cache_path = cache_dir.join(filename);
    let part_path = cache_dir.join(format!("{}{}", filename, PART_SUFFIX));

    if cache_path.exists() {
        println!("  [cache] using cached file");
        return Ok(cache_path);
    }

    println!("Downloading...");

    let client = build_http_client();

    // Determine the byte offset we can resume from (0 = fresh download).
    let mut start_offset: u64 = 0;
    if part_path.exists() {
        start_offset = match fs::metadata(&part_path) {
            Ok(m) => m.len(),
            Err(_) => 0,
        };
        // A zero-byte .part offers nothing to resume; treat as fresh.
        if start_offset == 0 {
            let _ = fs::remove_file(&part_path);
        }
    }

    // Build the request. Send a Range header when resuming so the server can
    // return 206 Partial Content with the remaining bytes. Some servers
    // return 200 with the full body even when Range is requested; we detect
    // that and fall back to a fresh download below.
    let mut req = client.get(url);
    if start_offset > 0 {
        req = req.header("Range", format!("bytes={}-", start_offset));
    }

    let response = req.send().context("Download failed")?;
    if !response.status().is_success() {
        anyhow::bail!("Download failed: HTTP {}", response.status());
    }

    let supports_resume = start_offset > 0 && response.status().as_u16() == 206;
    let total_size: u64 = if supports_resume {
        // Content-Range header looks like "bytes 100-999/1000".
        response
            .headers()
            .get("Content-Range")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.split('/').nth(1))
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0)
    } else {
        // 200 OK: full body. Reset offset and discard any stale .part.
        if start_offset > 0 {
            start_offset = 0;
            let _ = fs::remove_file(&part_path);
        }
        response.content_length().unwrap_or(0)
    };

    // Open the .part file: append when resuming, truncate when fresh.
    let mut dest_file = if supports_resume {
        OpenOptions::new()
            .append(true)
            .open(&part_path)
            .context("Cannot open .part for append")?
    } else {
        File::create(&part_path).context("Cannot create .part file")?
    };

    // Progress bar starts at the resume offset so the user sees it continue.
    let pb = ProgressBar::new(total_size);
    pb.set_position(start_offset);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta})")
            .unwrap()
            .progress_chars("#>-"),
    );

    let mut source = pb.wrap_read(response);
    copy(&mut source, &mut dest_file).context("Write failed")?;
    dest_file.flush().ok();

    pb.finish_with_message("Done");

    // Atomically promote .part to the final name. On success the file becomes
    // visible to the cache-hit check above; on failure the .part is left in
    // place so the next attempt can resume again.
    fs::rename(&part_path, &cache_path)
        .with_context(|| format!("Cannot rename .part to {}", cache_path.display()))?;

    println!("  [cache] saved to cache");
    Ok(cache_path)
}

/// Copy a cached file to a destination path.
pub fn copy_from_cache(filename: &str, dest: &Path) -> Result<()> {
    let cache_path = get_cache_dir().join(filename);
    if !cache_path.exists() {
        anyhow::bail!("File not found in cache: {}", filename);
    }
    fs::copy(&cache_path, dest).context("Failed to copy from cache")?;
    Ok(())
}

/// Check if a file exists in cache.
pub fn is_cached(filename: &str) -> bool {
    get_cache_dir().join(filename).exists()
}

/// List all cached files (name, size_bytes).
pub fn list_cached_files() -> Result<Vec<(String, u64)>> {
    let cache_dir = get_cache_dir();
    if !cache_dir.exists() {
        return Ok(Vec::new());
    }
    let mut files: Vec<(String, u64)> = Vec::new();
    for entry in fs::read_dir(&cache_dir)? {
        let entry = entry?;
        let metadata = entry.metadata()?;
        if metadata.is_file() {
            if let Some(name) = entry.file_name().to_str() {
                // Hide .part files from the listing: they are in-flight and
                // not usable as a cache hit, so showing them would be noise.
                if name.ends_with(PART_SUFFIX) {
                    continue;
                }
                files.push((name.to_string(), metadata.len()));
            }
        }
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(files)
}

/// Clear all cached files, returns total bytes cleared.
pub fn clear_cache() -> Result<u64> {
    let cache_dir = get_cache_dir();
    if !cache_dir.exists() {
        return Ok(0);
    }
    let mut cleared: u64 = 0;
    for entry in fs::read_dir(&cache_dir)? {
        let entry = entry?;
        let metadata = entry.metadata()?;
        if metadata.is_file() {
            cleared += metadata.len();
            fs::remove_file(entry.path())?;
        }
    }
    Ok(cleared)
}

// NOTE: download_file is kept for potential future use (e.g., checksum-only downloads)
#[allow(dead_code)]
pub fn download_file(_url: &str, _dest: &Path) -> Result<()> {
    anyhow::bail!("download_file is deprecated, use download_to_cache instead")
}
