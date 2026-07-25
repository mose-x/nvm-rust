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

/// Sidecar suffix storing the `If-Range` validator (ETag or Last-Modified)
/// captured when a fresh download started, so a resumed request can detect
/// an upstream file swap. See `ifrange_sidecar_path` / `download_to_cache`.
const IFRANGE_SUFFIX: &str = ".part.ifrange";

/// A download is in-flight if its cache entry is a `.part` (partial body) or
/// the matching `.part.ifrange` sidecar. Used by `list_cached_files` (hide
/// them) and `clear_cache` (preserve them — a concurrent install may be
/// actively writing them).
fn is_inflight_cache_file(name: &str) -> bool {
    name.ends_with(PART_SUFFIX) || name.ends_with(IFRANGE_SUFFIX)
}

/// Path of the sidecar that stores the `If-Range` validator (ETag or
/// Last-Modified) captured when a fresh download started, so a resumed
/// request can detect an upstream file swap of the same length.
fn ifrange_sidecar_path(cache_dir: &Path, filename: &str) -> PathBuf {
    cache_dir.join(format!("{}{}", filename, IFRANGE_SUFFIX))
}

/// Capture the strongest `If-Range` validator present on `response` and
/// persist it to the sidecar. ETag is preferred over Last-Modified (more
/// precise — a date has 1-second granularity). Writes are best-effort: a
/// missing validator yields an empty sidecar, and write failures are silently
/// ignored (we just lose swap protection for this download, which is strictly
/// better than failing the install).
fn save_ifrange_validator(sidecar_path: &Path, response: &reqwest::blocking::Response) {
    // ETag is the stronger validator (RFC 9110 §8.8.3); prefer it when present.
    let value = response
        .headers()
        .get("ETag")
        .or_else(|| response.headers().get("Last-Modified"))
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    // Best-effort write; missing validator → empty sidecar → load returns None.
    let _ = fs::write(sidecar_path, value);
}

/// Read the `If-Range` validator previously captured for this `.part`.
/// Returns None if there is no sidecar or it's empty (server didn't send
/// ETag/Last-Modified, so we can't defend against same-length swaps — resume
/// falls back to plain Range, matching the pre-fix behaviour).
fn load_ifrange_validator(sidecar_path: &Path) -> Option<String> {
    let s = fs::read_to_string(sidecar_path).ok()?;
    let s = s.trim();
    if s.is_empty() {
        None
    } else {
        Some(s.to_string())
    }
}

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
    let ifrange_path = ifrange_sidecar_path(&cache_dir, filename);

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
            let _ = fs::remove_file(&ifrange_path);
        }
    }

    // Build the request. Send a Range header when resuming so the server can
    // return 206 Partial Content with the remaining bytes. Some servers
    // return 200 with the full body even when Range is requested; we detect
    // that and fall back to a fresh download below.
    let mut req = client.get(url);
    if start_offset > 0 {
        req = req.header("Range", format!("bytes={}-", start_offset));
        // If-Range: defend against same-length upstream file swaps. Without
        // this header, a Range request for the remaining bytes of a file that
        // was replaced server-side with a *different* file of the same length
        // would happily append the new file's tail to the old file's head,
        // silently corrupting the archive. With If-Range, the server returns
        // 200 OK (full body) instead of 206 Partial Content when its current
        // representation doesn't match the validator we captured at the start
        // of the original download — we then discard the stale .part and
        // start over. Falls back to plain Range when no validator was
        // captured (server sent neither ETag nor Last-Modified), matching the
        // pre-fix behaviour.
        if let Some(v) = load_ifrange_validator(&ifrange_path) {
            req = req.header("If-Range", v);
        }
    }

    // `mut` because the 416-retry path below rebinds this to a fresh
    // response (re-requested without the Range header).
    let mut response = req.send().context(T("download_failed"))?;

    // HTTP 416 "Range Not Satisfiable" means our `.part` resume offset is
    // past the end of the server's current copy — usually the upstream file
    // was replaced with a shorter one, or the `.part` is corrupt/larger than
    // the real file. Without this handling the request falls into the generic
    // non-2xx branch below and bails, leaving the user stuck with a `.part`
    // they can never resume past. Delete the stale `.part` (and its sidecar)
    // and retry once from byte 0 without the Range header.
    if start_offset > 0 && response.status() == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
        let _ = fs::remove_file(&part_path);
        let _ = fs::remove_file(&ifrange_path);
        start_offset = 0;
        response = client.get(url).send().context(T("download_failed"))?;
    }

    if !response.status().is_success() {
        anyhow::bail!(
            "{}",
            format_t("download_http_failed", &[response.status().to_string()])
        );
    }

    let supports_resume =
        start_offset > 0 && response.status() == reqwest::StatusCode::PARTIAL_CONTENT;
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
        // 200 OK: full body. Two possible causes:
        //   (a) fresh download (start_offset was 0),
        //   (b) resume attempt where If-Range mismatched (upstream file
        //       swapped) OR the server ignored Range/If-Range entirely.
        // Either way, the existing .part (if any) is now stale — discard it
        // and the sidecar, then capture a fresh validator for THIS
        // representation so a future resume can detect a subsequent swap.
        if start_offset > 0 {
            start_offset = 0;
            let _ = fs::remove_file(&part_path);
        }
        save_ifrange_validator(&ifrange_path, &response);
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
    // Propagate flush failures instead of `.ok()`-ing them. A failed flush
    // (disk full, quota, network FS) leaves the `.part` with buffered bytes
    // not yet on disk; the subsequent `fs::rename` would then promote a
    // truncated file into the cache, where the checksum check in
    // `install_binary` would catch it — but only for the Node.js path.
    // io.js and source installs skip the checksum, so a silent flush failure
    // there could install a truncated archive. Surfacing the error here is
    // strictly better.
    dest_file.flush().context(T("write_failed"))?;

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

    // The .part has been promoted to the final cache file; drop the validator
    // sidecar so it doesn't linger as a stale in-flight marker (which would
    // make `cache list` hide a non-existent .part and `cache clear` skip a
    // non-existent file). Best-effort: a failure here leaves a 0-byte stray
    // sidecar, which `is_inflight_cache_file` already handles gracefully.
    let _ = fs::remove_file(&ifrange_path);

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
    // Read directly and map NotFound → empty instead of `exists()` +
    // `read_dir`: the two-step form is a TOCTOU race (another process could
    // remove the cache dir between the stat and the open), and a single
    // read_dir is one syscall instead of two.
    let rd = match fs::read_dir(&cache_dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    let mut files: Vec<(String, u64)> = Vec::new();
    for entry in rd {
        let entry = entry?;
        // Use symlink_metadata (not entry.metadata()) so a dangling symlink
        // in the cache dir doesn't propagate an error and abort the whole
        // listing. Matches clear_cache's behaviour. is_file() on symlink
        // metadata is true only for real files (symlinks themselves are
        // is_symlink()), which is what we want to list.
        let metadata = entry.path().symlink_metadata()?;
        if metadata.is_file() {
            if let Some(name) = entry.file_name().to_str() {
                // Hide in-flight files from the listing: a `.part` is a
                // partial body, a `.part.ifrange` is its validator sidecar.
                // Neither is usable as a cache hit, so showing them would
                // be noise (and would leak the existence of a download that
                // may still be in progress).
                if is_inflight_cache_file(name) {
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
    // Read directly and map NotFound → 0 instead of `exists()` + `read_dir`
    // (same race-free pattern as `list_cached_files`).
    let rd = match fs::read_dir(&cache_dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e.into()),
    };
    let mut cleared: u64 = 0;
    for entry in rd {
        let entry = match entry {
            Ok(e) => e,
            // Another process removed the entry between read_dir and the
            // implicit stat — skip it instead of aborting the whole clear
            // (the user would lose all progress made on earlier entries).
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e.into()),
        };
        // Use symlink_metadata so a symlink planted in the cache dir can't
        // trick us into deleting (or following) its target.
        let metadata = match entry.path().symlink_metadata() {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e.into()),
        };
        if metadata.is_file() {
            // Don't delete in-flight files: a concurrent `nvm install` may
            // be actively writing to a `.part`, and the matching
            // `.part.ifrange` sidecar holds the validator a future resume
            // of that `.part` needs to detect an upstream swap. Deleting
            // either out from under the running download causes the next
            // write to fail with a confusing "No such file" error, or
            // silently downgrades the next resume to plain Range (losing
            // swap protection). `list_cached_files` already hides both
            // suffixes from the listing; `clear_cache` must match that
            // behaviour so the user's "clear the cache" intent (completed
            // downloads) doesn't nuke a partial download still in progress.
            if entry
                .file_name()
                .to_str()
                .is_some_and(is_inflight_cache_file)
            {
                continue;
            }
            cleared += metadata.len();
            // Best-effort remove: a concurrent process may have already
            // deleted the file between our metadata stat and the remove.
            // NotFound here is fine, just skip; other errors still propagate.
            if let Err(e) = fs::remove_file(entry.path()) {
                if e.kind() == std::io::ErrorKind::NotFound {
                    continue;
                }
                return Err(e.into());
            }
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

    // --- P1-15: If-Range sidecar helpers --------------------------------------

    #[test]
    fn is_inflight_cache_file_matches_both_suffixes() {
        // The helper is what `list_cached_files`/`clear_cache` use to decide
        // what to hide/preserve. A regression that drops either suffix from
        // the match would either leak the sidecar into `cache list` output
        // or let `cache clear` delete a validator a concurrent resume needs.
        assert!(is_inflight_cache_file("node-v20.tar.xz.part"));
        assert!(is_inflight_cache_file("node-v20.tar.xz.part.ifrange"));
        // Completed cache files and unrelated files must NOT match.
        assert!(!is_inflight_cache_file("node-v20.tar.xz"));
        assert!(!is_inflight_cache_file("node-v20.tar.xz.part.ifrange.bak"));
        assert!(!is_inflight_cache_file("README.md"));
        assert!(!is_inflight_cache_file(""));
    }

    #[test]
    fn ifrange_sidecar_path_appends_suffix() {
        let dir = Path::new("/tmp/nvm-cache");
        let p = ifrange_sidecar_path(dir, "node-v20.tar.xz");
        assert_eq!(p, Path::new("/tmp/nvm-cache/node-v20.tar.xz.part.ifrange"));
    }

    #[test]
    fn load_ifrange_validator_handles_missing_empty_and_present() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let sidecar = tmp.path().join("node-v20.tar.xz.part.ifrange");

        // Missing sidecar → None (server never sent a validator, or this is
        // a pre-P1-15 .part left over from an older nvm-rust).
        assert!(load_ifrange_validator(&sidecar).is_none());

        // Empty sidecar → None (server's response had neither ETag nor
        // Last-Modified; `save_ifrange_validator` writes an empty file).
        fs::write(&sidecar, "").expect("write empty");
        assert!(load_ifrange_validator(&sidecar).is_none());

        // Whitespace-only sidecar → None (treated as empty after trim).
        fs::write(&sidecar, "   \n\t ").expect("write whitespace");
        assert!(load_ifrange_validator(&sidecar).is_none());

        // ETag value (with surrounding quotes, as servers send them) → Some.
        let etag = "\"deadbeef-1234\"";
        fs::write(&sidecar, etag).expect("write etag");
        assert_eq!(load_ifrange_validator(&sidecar).as_deref(), Some(etag));

        // HTTP-date (Last-Modified) → Some. Trailing newline is trimmed so
        // `save_ifrange_validator`'s `to_string()` (which has no trailing
        // newline) and a hand-edited file with a trailing newline both work.
        let date = "Wed, 21 Oct 2015 07:28:00 GMT";
        fs::write(&sidecar, format!("{date}\n")).expect("write date");
        assert_eq!(load_ifrange_validator(&sidecar).as_deref(), Some(date));
    }

    // Serializes tests that mutate the process-global NVM_DIR env var and
    // operate on the real cache dir. Without this, parallel cargo test runs
    // would race on NVM_DIR and stomp each other's cache directories.
    static CACHE_TESTS_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn clear_cache_skips_inflight_part_files() {
        // Regression for the concurrent-download bug: clear_cache previously
        // deleted EVERY file in the cache dir, including in-flight `.part`
        // files that a concurrent `nvm install` might be actively writing
        // to. The next write to the deleted .part would fail with a
        // confusing "No such file" error. list_cached_files already hid
        // .part from listings; clear_cache must match that behavior.
        let _guard = CACHE_TESTS_MUTEX
            .lock()
            .expect("CACHE_TESTS_MUTEX poisoned");
        // Save and override NVM_DIR so get_cache_dir() points at our tempdir.
        let saved_nvm_dir = std::env::var_os("NVM_DIR");
        let tmp = tempfile::tempdir().expect("tempdir");
        std::env::set_var("NVM_DIR", tmp.path());
        let cache_dir = get_cache_dir();
        fs::create_dir_all(&cache_dir).expect("create cache dir");

        // A completed (cached) download: 11 bytes. Should be deleted.
        fs::write(cache_dir.join("node-v99.9.9.tar.xz"), b"hello world").expect("write cached");
        // An in-flight partial download: 7 bytes. Must be preserved.
        fs::write(cache_dir.join("node-v99.9.9.tar.xz.part"), b"partial").expect("write part");
        // P1-15: the If-Range validator sidecar for the in-flight .part.
        // Must also be preserved — deleting it would silently downgrade the
        // next resume of this .part to plain Range, losing same-length swap
        // protection. 20 bytes of ETag text.
        fs::write(
            cache_dir.join("node-v99.9.9.tar.xz.part.ifrange"),
            "\"deadbeef-1234-abcd\"",
        )
        .expect("write ifrange sidecar");

        let cleared = clear_cache().expect("clear_cache should succeed");

        // The completed file is gone, both in-flight files survive.
        assert!(
            !cache_dir.join("node-v99.9.9.tar.xz").exists(),
            "completed cache file should be deleted"
        );
        assert!(
            cache_dir.join("node-v99.9.9.tar.xz.part").exists(),
            "in-flight .part file must NOT be deleted (concurrent download protection)"
        );
        assert!(
            cache_dir.join("node-v99.9.9.tar.xz.part.ifrange").exists(),
            "in-flight .part.ifrange sidecar must NOT be deleted (swap-detection protection)"
        );
        // Cleared byte count reflects only the completed file (11 bytes),
        // NOT the .part (7 bytes) or the sidecar (20 bytes) — the user asked
        // to clear completed cache, not in-flight downloads.
        assert_eq!(
            cleared, 11,
            "cleared bytes should count only completed files, not .part / .part.ifrange"
        );

        // Restore NVM_DIR so we don't poison other tests.
        match saved_nvm_dir {
            Some(v) => std::env::set_var("NVM_DIR", v),
            None => std::env::remove_var("NVM_DIR"),
        }
    }

    #[test]
    fn list_cached_files_hides_inflight_sidecars() {
        // P1-15: `cache list` must hide BOTH in-flight files (the .part body
        // and its .part.ifrange validator sidecar), not just the .part.
        // Showing the sidecar would be noise (it's not a usable cache hit)
        // and would leak the existence of a download still in progress.
        let _guard = CACHE_TESTS_MUTEX
            .lock()
            .expect("CACHE_TESTS_MUTEX poisoned");
        let saved_nvm_dir = std::env::var_os("NVM_DIR");
        let tmp = tempfile::tempdir().expect("tempdir");
        std::env::set_var("NVM_DIR", tmp.path());
        let cache_dir = get_cache_dir();
        fs::create_dir_all(&cache_dir).expect("create cache dir");

        fs::write(cache_dir.join("node-v20.0.0.tar.xz"), b"hello").expect("write cached");
        fs::write(cache_dir.join("node-v20.0.0.tar.xz.part"), b"partial").expect("write part");
        fs::write(
            cache_dir.join("node-v20.0.0.tar.xz.part.ifrange"),
            "\"etag-xyz\"",
        )
        .expect("write ifrange sidecar");

        let files = list_cached_files().expect("list_cached_files should succeed");

        // Only the completed file is listed.
        assert_eq!(
            files.len(),
            1,
            "only the completed cache file should be listed, got: {files:?}"
        );
        assert_eq!(files[0].0, "node-v20.0.0.tar.xz");
        assert_eq!(files[0].1, 5); // "hello" is 5 bytes

        match saved_nvm_dir {
            Some(v) => std::env::set_var("NVM_DIR", v),
            None => std::env::remove_var("NVM_DIR"),
        }
    }
}
