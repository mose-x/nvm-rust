use anyhow::{Context, Result};
use std::fs::{self, File};
use std::path::Path;
#[cfg(target_os = "windows")]
use std::process::Command;
use tar::Archive;
use xz2::read::XzDecoder;

use crate::system::os_type_name;

pub fn extract_archive(archive_path: &Path, dest_dir: &Path, version: &str) -> Result<()> {
    println!("Extracting...");

    fs::create_dir_all(dest_dir).context("Cannot create directory")?;

    let file = File::open(archive_path).context("Cannot open archive")?;

    #[cfg(target_os = "windows")]
    {
        let status = Command::new("7z")
            .arg("x")
            .arg("-y")
            .arg(format!("-o{}", dest_dir.display()))
            .arg(archive_path)
            .status()
            .context("Extraction failed, ensure 7z is installed")?;

        if !status.success() {
            anyhow::bail!("Extraction failed");
        }

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
        let status = Command::new("7z")
            .arg("x")
            .arg("-y")
            .arg(format!("-o{}", dest_dir.display()))
            .arg(archive_path)
            .status()
            .context("Extraction failed, ensure 7z is installed")?;

        if !status.success() {
            anyhow::bail!("Extraction failed");
        }

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
