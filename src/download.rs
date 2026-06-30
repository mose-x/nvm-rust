use anyhow::{Context, Result};
use indicatif::{ProgressBar, ProgressStyle};
use std::fs::{self, File};
use std::io::copy;
use std::path::Path;

use crate::proxy::build_http_client;
use crate::system::{ensure_cache_dir, get_cache_dir};

/// Download a file to cache dir, returning the local cache path.
/// If already cached, just returns the cached path.
pub fn download_to_cache(url: &str, filename: &str) -> Result<std::path::PathBuf> {
    let cache_dir = get_cache_dir();
    ensure_cache_dir()?;
    let cache_path = cache_dir.join(filename);

    if cache_path.exists() {
        println!("  [cache] using cached file");
        return Ok(cache_path);
    }

    println!("Downloading...");

    let client = build_http_client();
    let response = client.get(url).send().context("Download failed")?;
    if !response.status().is_success() {
        anyhow::bail!("Download failed: HTTP {}", response.status());
    }

    let total_size = response.content_length().unwrap_or(0);
    let pb = ProgressBar::new(total_size);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta})")
            .unwrap()
            .progress_chars("#>-"),
    );

    let mut source = pb.wrap_read(response);
    let mut dest_file = File::create(&cache_path).context("Cannot create cache file")?;
    copy(&mut source, &mut dest_file).context("Write failed")?;

    pb.finish_with_message("Done");

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
