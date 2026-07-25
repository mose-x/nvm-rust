//! Integration tests for install-time integrity verification (checksum / GPG).
//!
//! These exercise the *dispatch* of the integrity checks (which product gets
//! which check) without performing a real download, by planting a fake cached
//! archive and running `nvm install ... --offline`.

mod common;
use common::{isolated_nvm_dir, nvm_bin, stdout};
use std::process::Command;

/// Mirror `system::os_suffix()` for the host platform so the planted cache
/// file name matches what `build_install_target` computes for io.js. The
/// archive_name is `iojs-v<ver>-<os_suffix>`; the cache lookup is keyed on
/// that exact name, so a mismatch would 404 the cache and the install would
/// bail at `offline_no_cache` before reaching the checksum label.
fn host_os_suffix() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        "win-x64.7z"
    }
    #[cfg(target_os = "linux")]
    {
        if cfg!(target_arch = "aarch64") {
            "linux-arm64.tar.xz"
        } else {
            "linux-x64.tar.xz"
        }
    }
    #[cfg(target_os = "macos")]
    {
        if cfg!(target_arch = "aarch64") {
            "darwin-arm64.tar.xz"
        } else {
            "darwin-x64.tar.xz"
        }
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
    {
        ""
    }
}

/// Regression for P1-14: io.js installs used to skip checksum verification
/// entirely (the whole integrity block was gated on `!target.is_iojs`).
///
/// After the fix the SHA-256 checksum check runs for io.js too — only GPG is
/// node-only. We verify this behaviorally: plant a fake cached io.js archive,
/// run `nvm install iojs-3.3.1 --offline`, and assert the "Checksum:" label
/// (and the "skipped (offline)" notice) appear in stdout.
///
/// The install itself fails at extraction (the planted bytes are not a real
/// tarball), but that's fine — the checksum label is printed *before*
/// extraction, so it's in stdout regardless of the final exit status. Before
/// the fix, io.js install output contained no "Checksum:" line at all.
#[test]
fn iojs_offline_install_prints_checksum_label() {
    let (dir, nvm_dir) = isolated_nvm_dir();

    // Plant a fake cached io.js archive so the offline path reaches the
    // integrity-check block instead of bailing at `offline_no_cache`.
    let cache = dir.path().join("cache");
    std::fs::create_dir_all(&cache).expect("create cache dir");
    let archive_name = format!("iojs-v3.3.1-{}", host_os_suffix());
    std::fs::write(cache.join(&archive_name), b"not a real tarball")
        .expect("write fake cached io.js archive");

    let out = Command::new(nvm_bin())
        .args(["install", "iojs-3.3.1", "--offline"])
        .env("NVM_DIR", &nvm_dir)
        .output()
        .expect("run nvm install iojs-3.3.1 --offline");

    let s = stdout(&out);
    // The checksum label must now appear for io.js (previously skipped).
    assert!(
        s.contains("Checksum:"),
        "io.js install should print the Checksum label after P1-14 fix, got: {s}"
    );
    // Offline mode prints the "skipped (offline)" notice for the checksum.
    assert!(
        s.to_lowercase().contains("offline"),
        "io.js offline install should note checksum skipped (offline), got: {s}"
    );
}
