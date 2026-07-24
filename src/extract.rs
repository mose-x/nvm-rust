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
    println!("{}", T("extracting"));

    fs::create_dir_all(dest_dir).context(T("cannot_create_dir"))?;

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

        let extracted = dest_dir.join(format!("node-{}-win-x64", version));
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
        // `node-vX.Y.Z-<platform>-<arch>` (e.g. `node-v20.0.0-darwin-arm64`).
        // Derive that name from `os_suffix()` via `extracted_dir_name` so it
        // always matches the real dir. The previous inline code appended a
        // literal `-x64`, which on ARM64 produced
        // `node-v20.0.0-darwin-arm64-x64` (a path that never exists), so
        // `flatten_dir` was skipped and the version dir stayed nested one
        // level deep — breaking `nvm use/which/run`.
        let extracted = dest_dir.join(extracted_dir_name(&format!("node-{}", version)));
        if extracted.exists() {
            flatten_dir(&extracted, dest_dir)?;
        }
    }

    Ok(())
}

/// Extract an io.js tarball. io.js archives use "iojs-vX.Y.Z-platform-arch" prefix.
pub fn extract_iojs_archive(archive_path: &Path, dest_dir: &Path, version: &str) -> Result<()> {
    println!("{}", T("extracting"));

    fs::create_dir_all(dest_dir).context(T("cannot_create_dir"))?;

    let ver_num = crate::utils::strip_iojs_prefix(version)
        .unwrap_or(version)
        .trim_start_matches('v');

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

        let extracted = dest_dir.join(format!("iojs-v{}-win-x64", ver_num));
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

        // Mirror the node-archive fix: io.js tarballs expand to
        // `iojs-vX.Y.Z-<platform>-<arch>`, so derive the dir name from
        // `os_suffix()` via `extracted_dir_name` instead of appending a
        // literal `-x64` (which broke ARM64 hosts the same way as above).
        let extracted = dest_dir.join(extracted_dir_name(&format!("iojs-v{}", ver_num)));
        if extracted.exists() {
            flatten_dir(&extracted, dest_dir)?;
        }
    }

    Ok(())
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
}
