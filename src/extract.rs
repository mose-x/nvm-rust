use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

use crate::i18n::T;

// These imports are only used by the `#[cfg(not(target_os = "windows"))]`
// extraction path. On Windows the code path uses `sevenz_rust::decompress_file`
// instead, so leaving these unconditionally imported produces "unused import"
// errors under `-D warnings`. Gate them to match their usage.
#[cfg(not(target_os = "windows"))]
use std::fs::File;
#[cfg(not(target_os = "windows"))]
use tar::Archive;
#[cfg(not(target_os = "windows"))]
use xz2::read::XzDecoder;

#[cfg(not(target_os = "windows"))]
use crate::system::os_suffix;

pub fn extract_archive(archive_path: &Path, dest_dir: &Path, version: &str) -> Result<()> {
    // `node-vX.Y.Z` for both Windows (node-vX.Y.Z-win-x64) and Unix
    // (node-vX.Y.Z-<platform>-<arch>) extracted dir names.
    extract_inner(archive_path, dest_dir, &format!("node-{}", version))
}

/// Extract an io.js tarball. io.js archives use "iojs-vX.Y.Z-platform-arch" prefix.
pub fn extract_iojs_archive(archive_path: &Path, dest_dir: &Path, version: &str) -> Result<()> {
    let ver_num = crate::utils::strip_iojs_prefix(version)
        .unwrap_or(version)
        .trim_start_matches('v');
    // `iojs-vX.Y.Z` for both Windows (iojs-vX.Y.Z-win-x64) and Unix
    // (iojs-vX.Y.Z-<platform>-<arch>) extracted dir names.
    extract_inner(archive_path, dest_dir, &format!("iojs-v{}", ver_num))
}

/// Shared extraction core for both Node.js and io.js tarballs. `label` is the
/// version-prefixed head of the extracted dir name (`node-v20.0.0` or
/// `iojs-v3.3.1`); the platform/arch tail is derived from `os_suffix()` on
/// Unix or hardcoded `win-x64` on Windows.
fn extract_inner(archive_path: &Path, dest_dir: &Path, label: &str) -> Result<()> {
    fs::create_dir_all(dest_dir).context(T("cannot_create_dir"))?;
    // RAII guard: if extraction fails midway, dest_dir will contain partial
    // files. Without cleanup, the next `nvm install <same version>` sees a
    // non-empty version_dir and bails "already installed", leaving the user
    // stuck on a corrupted directory (the soft-lock bug). The guard removes
    // dest_dir on every error path; disarmed on success.
    //
    // CONTRACT: caller must ensure dest_dir does not exist or is empty
    // (install.rs enforces this for the binary install path). The guard
    // removes the whole dir on failure, so a pre-existing non-empty dest
    // would be lost — that is the caller's contract violation, not a bug
    // here.
    let mut guard = DirGuard::new(dest_dir.to_path_buf());

    #[cfg(target_os = "windows")]
    {
        // Pure-Rust 7z decompression — reads via path, no external 7z.exe.
        // Don't `File::open` here: on Windows an AV-locked tarball would
        // fail the open even though decompress_file would have worked.
        sevenz_rust::decompress_file(archive_path, dest_dir).map_err(|e| {
            anyhow::anyhow!(
                "{}",
                crate::i18n::format_t("extraction_failed", &[e.to_string()])
            )
        })?;

        let extracted = dest_dir.join(format!("{}-win-x64", label));
        if extracted.exists() {
            flatten_dir(&extracted, dest_dir)?;
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        let file = File::open(archive_path).context(T("cannot_open_archive"))?;
        let decoder = XzDecoder::new(file);
        let mut archive = Archive::new(decoder);
        archive
            .unpack(dest_dir)
            .context(T("extraction_failed_short"))?;

        // The tarball expands to a single top-level dir named
        // `<label>-<platform>-<arch>` (e.g. `node-v20.0.0-darwin-arm64`).
        // Derive that name from `os_suffix()` via `extracted_dir_name` so it
        // always matches the real dir. The previous inline code appended a
        // literal `-x64`, which on ARM64 produced
        // `node-v20.0.0-darwin-arm64-x64` (a path that never exists), so
        // `flatten_dir` was skipped and the version dir stayed nested one
        // level deep — breaking `nvm use/which/run`.
        let extracted = dest_dir.join(extracted_dir_name(label));
        if extracted.exists() {
            flatten_dir(&extracted, dest_dir)?;
        }
    }

    guard.disarm();
    Ok(())
}

/// RAII guard that removes a directory when dropped, unless disarmed.
/// Used by `extract_inner` to clean up a partially-populated dest_dir on
/// extraction failure, preventing the "already installed" soft-lock.
struct DirGuard {
    path: std::path::PathBuf,
    armed: bool,
}

impl DirGuard {
    fn new(path: std::path::PathBuf) -> Self {
        Self { path, armed: true }
    }
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for DirGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

/// Build the top-level directory name a Node.js/io.js tarball expands to on
/// the current non-Windows host, e.g. `node-v20.0.0-linux-arm64` or
/// `iojs-v3.3.1-darwin-x64`.
///
/// `label` is the version-prefixed head (`node-v20.0.0` / `iojs-v3.3.1`); the
/// `<platform>-<arch>` tail is derived from `os_suffix()` with the `.tar.xz`
/// extension stripped, so it always matches the directory inside the tarball.
///
/// This is the single source of truth for the extracted-dir name. The
/// previous inline `format!("node-{}-{}-x64", …)` appended a literal `-x64`
/// and silently broke ARM64 hosts: the looked-up path never existed, so
/// `flatten_dir` was skipped and the version dir stayed nested one level
/// deep. (Windows uses a 7z archive and hardcodes `win-x64`, so it does not
/// go through here.)
#[cfg(not(target_os = "windows"))]
fn extracted_dir_name(label: &str) -> String {
    let suffix = os_suffix().trim_end_matches(".tar.xz");
    format!("{}-{}", label, suffix)
}

fn flatten_dir(src: &Path, dest: &Path) -> Result<()> {
    // Collect the entry list BEFORE renaming anything. `read_dir` yields a
    // live directory stream, and `fs::rename` inside the loop mutates the
    // very directory being iterated. On Linux `readdir(3)` is allowed to
    // skip or revisit entries when the directory changes mid-scan — the
    // previous loop could then leave some entries behind in `src` (silently
    // skipped) and move others twice (revisited), producing a half-empty
    // version dir with no error surfaced. Snapshotting first makes the move
    // deterministic and complete.
    let entries: Vec<fs::DirEntry> = fs::read_dir(src)?.collect::<Result<_, _>>()?;

    for entry in entries {
        let target = dest.join(entry.file_name());
        // Bail on name conflict instead of letting `fs::rename` silently
        // overwrite the existing target (Unix) or fail with a platform-
        // specific error (Windows). The caller's contract is that `dest` is
        // empty; a conflict means either a contract violation, a tarball
        // with duplicate top-level entries, or a concurrent writer — all
        // bugs we want to surface rather than paper over with silent data
        // loss.
        if target.exists() {
            anyhow::bail!(
                "{}",
                crate::i18n::format_t(
                    "flatten_name_conflict",
                    &[
                        entry.file_name().to_string_lossy().to_string(),
                        dest.display().to_string(),
                    ]
                )
            );
        }
        fs::rename(entry.path(), &target)?;
    }
    fs::remove_dir(src)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flatten_dir_moves_contents_and_removes_source() {
        // Build a src dir with two entries and flatten it into dest.
        let tmp = tempfile::tempdir().expect("tempdir");
        let src = tmp.path().join("src");
        let dest = tmp.path().join("dest");
        std::fs::create_dir_all(&src).expect("create src");
        std::fs::create_dir_all(&dest).expect("create dest");
        std::fs::write(src.join("a.txt"), b"a").expect("write a");
        std::fs::write(src.join("b.txt"), b"b").expect("write b");

        flatten_dir(&src, &dest).expect("flatten_dir should succeed");

        // Contents moved into dest.
        assert!(dest.join("a.txt").exists(), "a.txt should be in dest");
        assert!(dest.join("b.txt").exists(), "b.txt should be in dest");
        // Source directory removed.
        assert!(!src.exists(), "src should be removed after flatten");
    }

    #[test]
    fn flatten_dir_empty_source_is_removed() {
        // An empty source dir should still be removed (read_dir yields none,
        // then remove_dir runs unconditionally).
        let tmp = tempfile::tempdir().expect("tempdir");
        let src = tmp.path().join("empty_src");
        let dest = tmp.path().join("dest");
        std::fs::create_dir_all(&src).expect("create empty src");
        std::fs::create_dir_all(&dest).expect("create dest");

        flatten_dir(&src, &dest).expect("flatten_dir on empty src should succeed");
        assert!(!src.exists(), "empty src should still be removed");
    }

    #[test]
    fn flatten_dir_bails_on_name_conflict() {
        // Regression for the silent-overwrite bug: if `dest` already has an
        // entry with the same name as one in `src`, `flatten_dir` must bail
        // with a clear error instead of letting `fs::rename` silently
        // clobber the existing file (Unix) or fail with a platform-specific
        // error (Windows).
        let tmp = tempfile::tempdir().expect("tempdir");
        let src = tmp.path().join("src");
        let dest = tmp.path().join("dest");
        std::fs::create_dir_all(&src).expect("create src");
        std::fs::create_dir_all(&dest).expect("create dest");
        // Pre-existing file in dest with the same name as a src entry.
        std::fs::write(dest.join("conflict.txt"), b"original").expect("write dest conflict");
        std::fs::write(src.join("conflict.txt"), b"new").expect("write src conflict");
        std::fs::write(src.join("unique.txt"), b"u").expect("write src unique");

        let result = flatten_dir(&src, &dest);
        assert!(
            result.is_err(),
            "flatten_dir should bail on name conflict, got: {:?}",
            result
        );
        let msg = format!("{}", result.unwrap_err());
        assert!(
            msg.contains("conflict.txt") || msg.to_lowercase().contains("conflict"),
            "error should name the conflicting entry, got: {msg}"
        );
        // The pre-existing file must NOT have been overwritten — the bail
        // happens before the rename of the conflicting entry.
        assert_eq!(
            std::fs::read_to_string(dest.join("conflict.txt")).unwrap(),
            "original",
            "original dest file must not be clobbered on conflict"
        );
    }

    #[test]
    fn flatten_dir_moves_all_entries_despite_iteration_mutation() {
        // Regression for the rename-during-iteration bug: previously the
        // loop called `fs::rename` while iterating `read_dir(src)`, which on
        // some filesystems causes `readdir` to skip entries. With many
        // entries the move could silently drop some. Snapshot-first must
        // move every entry.
        //
        // We create enough entries that a buggy iterator would be unlikely
        // to move all of them by luck.
        let tmp = tempfile::tempdir().expect("tempdir");
        let src = tmp.path().join("src");
        let dest = tmp.path().join("dest");
        std::fs::create_dir_all(&src).expect("create src");
        std::fs::create_dir_all(&dest).expect("create dest");

        let n = 50;
        for i in 0..n {
            std::fs::write(src.join(format!("file_{i:03}.txt")), [i as u8])
                .expect("write src file");
        }

        flatten_dir(&src, &dest).expect("flatten_dir should succeed");

        // Every entry must be present in dest — none silently skipped.
        for i in 0..n {
            let p = dest.join(format!("file_{i:03}.txt"));
            assert!(p.exists(), "file {i:03} should have been moved to dest");
            assert_eq!(
                std::fs::read(&p).unwrap(),
                [i as u8],
                "file {i:03} content should match"
            );
        }
        assert!(!src.exists(), "src should be removed after flatten");
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn extracted_dir_name_matches_tarball_layout() {
        // Regression for the ARM64 bug (extract.rs:42/89): the looked-up dir
        // must be `<label>-<os_suffix without .tar.xz>`. The old inline code
        // appended a literal `-x64`, which on ARM64 produced
        // `node-vX.Y.Z-darwin-arm64-x64` (a path that never exists), silently
        // skipping `flatten_dir`. Lock the formula and explicitly forbid the
        // `-arm64-x64` / `-x64-x64` patterns the bug introduced.
        for label in ["node-v20.0.0", "iojs-v3.3.1"] {
            let name = extracted_dir_name(label);
            let suffix = os_suffix().trim_end_matches(".tar.xz");
            assert_eq!(name, format!("{}-{}", label, suffix));
            assert!(
                name.starts_with(&format!("{label}-")),
                "{name} should start with {label}-"
            );
            assert!(!name.ends_with("-arm64-x64"));
            assert!(!name.ends_with("-x64-x64"));
        }
    }

    #[test]
    fn extract_archive_cleans_dest_dir_on_failure() {
        // Regression for the soft-lock bug: if extraction fails midway,
        // dest_dir must be removed so the next `nvm install <same version>`
        // does not bail "already installed" on a corrupted partial dir.
        // Here we feed a non-existent archive so `File::open` (Unix) or
        // `decompress_file` (Windows) fails immediately after create_dir_all.
        let tmp = tempfile::tempdir().expect("tempdir");
        let dest = tmp.path().join("dest");
        let bogus_archive = tmp.path().join("does-not-exist.tar.xz");

        let result = extract_archive(&bogus_archive, &dest, "v99.99.99");
        assert!(result.is_err(), "extract should fail on missing archive");
        assert!(
            !dest.exists(),
            "dest_dir must be cleaned up on extract failure to prevent soft-lock"
        );
    }

    #[test]
    fn dir_guard_removes_dir_when_armed() {
        // DirGuard armed (default) → Drop removes the directory.
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = tmp.path().join("guarded");
        std::fs::create_dir_all(&dir).expect("create dir");
        std::fs::write(dir.join("partial.txt"), b"x").expect("write partial");

        {
            let _guard = DirGuard::new(dir.clone());
        } // guard dropped here

        assert!(!dir.exists(), "armed DirGuard should remove dir on drop");
    }

    #[test]
    fn dir_guard_keeps_dir_when_disarmed() {
        // DirGuard disarmed → Drop is a no-op, dir survives.
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = tmp.path().join("kept");
        std::fs::create_dir_all(&dir).expect("create dir");
        std::fs::write(dir.join("good.txt"), b"y").expect("write good");

        {
            let mut guard = DirGuard::new(dir.clone());
            guard.disarm();
        } // guard dropped here, but disarmed

        assert!(dir.exists(), "disarmed DirGuard should keep dir");
        assert!(dir.join("good.txt").exists(), "dir contents should survive");
    }
}
