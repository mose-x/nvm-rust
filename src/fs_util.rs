use std::fs;
use std::path::Path;

use anyhow::Result;

/// Atomically write `contents` to `path` using the temp-file-then-rename
/// pattern. On Unix, `fs::rename` is atomic — readers always see either the
/// old or the new file, never a half-written one. This prevents concurrent
/// `nvm use` invocations from interleaving writes (which would leave a
/// truncated `current`/`config.json` behind), and protects against a crash
/// mid-write corrupting the file.
///
/// The temp file is created in the same directory as the target (required for
/// rename to be atomic — cross-device rename is not). On failure the temp
/// file is removed by `NamedTempFile`'s Drop.
pub fn atomic_write(path: &Path, contents: &str) -> Result<()> {
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
    std::io::Write::write_all(&mut tmp, contents.as_bytes())?;
    tmp.persist(path).map_err(|e| anyhow::anyhow!(e.error))?;
    Ok(())
}

pub fn file_backup_path(path: &Path) -> std::path::PathBuf {
    let mut backup = path.to_path_buf();
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "backup".to_string());
    backup.set_file_name(format!("{}.bak", name));
    backup
}

pub fn backup_file(path: &Path) -> Result<(), std::io::Error> {
    // Copy directly and map NotFound → Ok(()) instead of `exists()` + `copy`.
    // The two-step form is a TOCTOU race: a concurrent process could remove
    // the file between the stat and the open, turning a "nothing to back up"
    // no-op into a confusing "No such file or directory" error. The single
    // read is also one syscall instead of two.
    match fs::copy(path, file_backup_path(path)) {
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_file_backup_path() {
        use std::path::PathBuf;
        let path = PathBuf::from("/tmp/test.txt");
        let backup = file_backup_path(&path);
        assert_eq!(backup, PathBuf::from("/tmp/test.txt.bak"));
    }

    #[test]
    fn test_backup_file_copies_existing_file() {
        use std::fs;
        use std::path::PathBuf;
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("a.txt");
        fs::write(&path, b"hello").expect("write");
        backup_file(&path).expect("backup_file should succeed for existing file");
        let backup = file_backup_path(&path);
        assert_eq!(backup, PathBuf::from(dir.path()).join("a.txt.bak"));
        assert_eq!(fs::read(&backup).expect("read backup"), b"hello");
    }

    #[test]
    fn test_backup_file_no_op_for_missing_file() {
        // For a non-existent path, backup_file is a no-op (Ok(())) and must
        // NOT create a .bak file.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("missing.txt");
        backup_file(&path).expect("backup_file should be Ok for missing file");
        assert!(!dir.path().join("missing.txt.bak").exists());
    }

    #[test]
    fn test_atomic_write_creates_new_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("current");
        atomic_write(&path, "v20.11.0").expect("atomic_write should succeed");
        assert_eq!(std::fs::read_to_string(&path).expect("read"), "v20.11.0");
    }

    #[test]
    fn test_atomic_write_overwrites_existing_file() {
        // The current-file save path relies on overwrite being atomic: a
        // concurrent reader must never see a half-written file. Verify the
        // final content is exactly the new content (not appended, not mixed).
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("current");
        std::fs::write(&path, "v18.0.0").expect("initial write");
        atomic_write(&path, "v22.22.2").expect("atomic_write overwrite");
        assert_eq!(std::fs::read_to_string(&path).expect("read"), "v22.22.2");
    }

    #[test]
    fn test_atomic_write_leaves_no_temp_files() {
        // NamedTempFile::persist renames the temp file; on success the temp
        // is gone and only the target remains. A leftover *.tmp or hidden
        // temp file would accumulate across `nvm use` invocations.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("current");
        atomic_write(&path, "v20.0.0").expect("atomic_write");
        let entries: Vec<_> = std::fs::read_dir(dir.path()).expect("read_dir").collect();
        assert_eq!(
            entries.len(),
            1,
            "exactly one file (the target) should exist"
        );
        assert_eq!(
            entries[0].as_ref().expect("entry").file_name(),
            std::ffi::OsStr::new("current")
        );
    }
}
