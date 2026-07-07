use anyhow::{Context, Result};
use std::fs::{self, File};
use std::path::Path;
use tar::Archive;
use xz2::read::XzDecoder;

use crate::system::os_type_name;

pub fn extract_archive(archive_path: &Path, dest_dir: &Path, version: &str) -> Result<()> {
    println!("Extracting...");

    fs::create_dir_all(dest_dir).context("Cannot create directory")?;

    let file = File::open(archive_path).context("Cannot open archive")?;

    #[cfg(target_os = "windows")]
    {
        let _ = file; // .7z path reads via path, not the opened File handle.
        // Pure-Rust 7z decompression — no external 7z.exe needed.
        sevenz_rust::decompress_file(archive_path, dest_dir)
            .map_err(|e| anyhow::anyhow!("Extraction failed: {e}"))?;

        let extracted = dest_dir.join(format!("node-{}-win-x64", version));
        if extracted.exists() {
            flatten_dir(&extracted, dest_dir)?;
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        let decoder = XzDecoder::new(file);
        let mut archive = Archive::new(decoder);
        archive.unpack(dest_dir).context("Extraction failed")?;

        let extracted = dest_dir.join(format!("node-{}-{}-x64", version, os_type_name()));
        if extracted.exists() {
            flatten_dir(&extracted, dest_dir)?;
        }
    }

    Ok(())
}

/// Extract an io.js tarball. io.js archives use "iojs-vX.Y.Z-platform-arch" prefix.
pub fn extract_iojs_archive(archive_path: &Path, dest_dir: &Path, version: &str) -> Result<()> {
    println!("Extracting...");

    fs::create_dir_all(dest_dir).context("Cannot create directory")?;

    let ver_num = version
        .trim_start_matches("iojs-v")
        .trim_start_matches("io.js-v")
        .trim_start_matches('v');

    let file = File::open(archive_path).context("Cannot open archive")?;

    #[cfg(target_os = "windows")]
    {
        let _ = file; // .7z path reads via path, not the opened File handle.
        // Pure-Rust 7z decompression — no external 7z.exe needed.
        sevenz_rust::decompress_file(archive_path, dest_dir)
            .map_err(|e| anyhow::anyhow!("Extraction failed: {e}"))?;

        let extracted = dest_dir.join(format!("iojs-v{}-win-x64", ver_num));
        if extracted.exists() {
            flatten_dir(&extracted, dest_dir)?;
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        let decoder = XzDecoder::new(file);
        let mut archive = Archive::new(decoder);
        archive.unpack(dest_dir).context("Extraction failed")?;

        let extracted = dest_dir.join(format!("iojs-v{}-{}-x64", ver_num, os_type_name()));
        if extracted.exists() {
            flatten_dir(&extracted, dest_dir)?;
        }
    }

    Ok(())
}

fn flatten_dir(src: &Path, dest: &Path) -> Result<()> {
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let target = dest.join(entry.file_name());
        fs::rename(entry.path(), target)?;
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
}
