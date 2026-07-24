use anyhow::{Context, Result};
use indicatif::{ProgressBar, ProgressStyle};
use std::fs::{self, File, OpenOptions};
use std::io::{copy, Write};
use std::path::{Path, PathBuf};

use crate::i18n::{format_t, T};
use crate::proxy::build_http_client;
use crate::system::{ensure_cache_dir, get_cache_dir};

/// Suffix used for partial downloads. A file is only considered complete
/// (and therefore cache-hit eligible) once it has been renamed from
/// `<name>.part` to `<name>`. This is what makes resume safe: a truncated
/// or half-downloaded file never satisfies the `cache_path.exists()` check.
const PART_SUFFIX: &str = ".part";

/// Refuse to open `path` for writing if it is (or points through) a symlink.
///
/// The cache dir lives under the user's home, but any process that can write
/// there (another user on a shared box, a compromised helper, a malicious
/// npm postinstall script) could pre-create a symlink at
/// `<cache>/node-v20.tar.gz.part` → `~/.ssh/authorized_keys`. A plain
/// `File::create` / `OpenOptions::append` would follow that symlink and
/// clobber the target with download bytes.
///
/// This checks `symlink_metadata` (which does NOT follow the link) and bails
/// if the entry is a symlink. On Unix we additionally pass `O_NOFOLLOW` to
/// close the TOCTOU window between the metadata check and the `open(2)` call.
/// On Windows, symlink creation requires the SeCreateSymbolicLink privilege,
/// so the metadata check is sufficient in practice.
fn ensure_not_symlink(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(m) => {
            if m.file_type().is_symlink() {
                anyhow::bail!(
                    "{}",
                    format_t("part_refused_symlink", &[path.display().to_string()])
                );
            }
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(anyhow::anyhow!(e)),
    }
}

/// Open `path` for a fresh download (truncate + create), refusing symlinks
/// and using restrictive permissions on Unix (0600) so a cached partial
/// download is not world-readable.
fn create_part_file(path: &Path) -> Result<File> {
    ensure_not_symlink(path)?;
    let mut opts = OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // O_NOFOLLOW: if `path` is a symlink, the open fails with ELOOP
        // instead of following it. Closes the TOCTOU gap between
        // `ensure_not_symlink` and the actual open.
        opts.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    opts.open(path).map_err(|e| {
        // O_NOFOLLOW on a symlink fails with ELOOP. `ErrorKind::FilesystemLoop`
        // is unstable (issue #86442), so we match on the raw OS error instead.
        let is_symlink_rejection = e.raw_os_error() == Some(libc::ELOOP)
            || format!("{e}").contains("Too many levels of symbolic links");
        if is_symlink_rejection {
            anyhow::anyhow!(
                "{}",
                format_t("part_refused_symlink", &[path.display().to_string()])
            )
        } else {
            anyhow::anyhow!("{}: {e}", T("cannot_create_part"))
        }
    })
}

/// Open an existing `.part` for resume (append), after verifying it is a
/// regular file and not a symlink planted between runs.
fn open_part_for_resume(path: &Path) -> Result<File> {
    ensure_not_symlink(path)?;
    let mut opts = OpenOptions::new();
    opts.append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.custom_flags(libc::O_NOFOLLOW);
    }
    opts.open(path).map_err(|e| {
        let is_symlink_rejection = e.raw_os_error() == Some(libc::ELOOP)
            || format!("{e}").contains("Too many levels of symbolic links");
        if is_symlink_rejection {
            anyhow::anyhow!(
                "{}",
                format_t("part_refused_symlink", &[path.display().to_string()])
            )
        } else {
            anyhow::anyhow!("{}: {e}", T("cannot_open_part_append"))
        }
    })
}

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
        println!("  {}", T("cached_file"));
        return Ok(cache_path);
    }

    println!("{}", T("downloading"));

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

    let response = req.send().context(T("download_failed"))?;
    if !response.status().is_success() {
        anyhow::bail!(
            "{}",
            format_t("download_http_failed", &[response.status().to_string()])
        );
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
    // Both paths go through the symlink-safe helpers — a symlink planted
    // at the .part path would otherwise be followed and its target clobbered
    // with download bytes.
    let mut dest_file = if supports_resume {
        open_part_for_resume(&part_path)?
    } else {
        create_part_file(&part_path)?
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
    copy(&mut source, &mut dest_file).context(T("write_failed"))?;
    dest_file.flush().ok();

    pb.finish_with_message(T("progress_done"));

    // Atomically promote .part to the final name. On success the file becomes
    // visible to the cache-hit check above; on failure the .part is left in
    // place so the next attempt can resume again.
    //
    // On Unix `fs::rename` atomically overwrites an existing destination.
    // On Windows `fs::rename` fails with AccessDenied when the destination
    // already exists (e.g. a previous cache entry), so we remove it first
    // and retry. The remove-then-rename window is not atomic on Windows,
    // but the cache dir is single-writer (one nvm process at a time) and
    // a crash here just leaves the .part for the next resume attempt.
    fs::rename(&part_path, &cache_path)
        .or_else(|_| {
            let _ = fs::remove_file(&cache_path);
            fs::rename(&part_path, &cache_path)
        })
        .with_context(|| format_t("cannot_rename_part", &[cache_path.display().to_string()]))?;

    println!("  {}", T("cached_saved"));
    Ok(cache_path)
}

/// Copy a cached file to a destination path.
pub fn copy_from_cache(filename: &str, dest: &Path) -> Result<()> {
    let cache_path = get_cache_dir().join(filename);
    if !cache_path.exists() {
        anyhow::bail!("{}", format_t("file_not_in_cache", &[filename.to_string()]));
    }
    fs::copy(&cache_path, dest).context(T("copy_from_cache_failed"))?;
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
        // Use symlink_metadata so a symlink planted in the cache dir can't
        // trick us into deleting (or following) its target.
        let metadata = entry.path().symlink_metadata()?;
        if metadata.is_file() {
            cleared += metadata.len();
            fs::remove_file(entry.path())?;
        }
    }
    Ok(cleared)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn create_part_file_refuses_symlink() {
        // A symlink planted at the .part path must be rejected, not followed.
        // We point the symlink at /dev/null (harmless target) to prove the
        // open is refused before any write could happen.
        let tmp = tempfile::tempdir().expect("tempdir");
        let link = tmp.path().join("evil.part");
        std::os::unix::fs::symlink("/dev/null", &link).expect("symlink");

        let err = create_part_file(&link).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("symlink") || msg.contains("symbolic"),
            "expected symlink-rejection error, got: {msg}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn open_part_for_resume_refuses_symlink() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let link = tmp.path().join("resume.part");
        std::os::unix::fs::symlink("/dev/null", &link).expect("symlink");

        let err = open_part_for_resume(&link).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("symlink") || msg.contains("symbolic"),
            "expected symlink-rejection error, got: {msg}"
        );
    }

    #[test]
    fn create_part_file_writes_regular_file() {
        // Happy path: a regular (non-symlink) path is created and writable.
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("ok.part");
        {
            let mut f = create_part_file(&path).expect("create part");
            f.write_all(b"hello").expect("write");
        }
        assert_eq!(fs::read(&path).unwrap(), b"hello");
    }
}
