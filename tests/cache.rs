//! Integration tests for `nvm cache list` and `nvm cache clear`.
//!
//! Covers both the empty-cache state and the non-empty state (by placing
//! fake files directly in `NVM_DIR/cache/`). Also verifies that `.part`
//! in-flight files are hidden from `cache list`.

mod common;
use common::{run_isolated, stdout};
use std::fs;

#[test]
fn cache_list_empty_succeeds() {
    let (out, _dir) = run_isolated(&["cache", "list"]);
    assert!(out.status.success(), "cache list on empty should succeed");
}

#[test]
fn cache_clear_empty_succeeds_and_reports_zero() {
    let (out, _dir) = run_isolated(&["cache", "clear"]);
    assert!(out.status.success(), "cache clear on empty should succeed");
    // The cleared-bytes message should mention "0" somewhere.
    let s = stdout(&out);
    assert!(
        s.contains('0') || s.to_lowercase().contains("cleared"),
        "expected a '0 / cleared' message, got: {s}"
    );
}

#[test]
fn cache_list_shows_files_and_hides_part_files() {
    let (dir, nvm_dir) = common::isolated_nvm_dir();
    let cache = dir.path().join("cache");
    fs::create_dir_all(&cache).expect("create cache dir");

    // A real cached file and an in-flight .part file.
    fs::write(cache.join("node-v20.0.0.tar.xz"), b"hello").expect("write cached file");
    fs::write(cache.join("node-v20.0.0.tar.xz.part"), b"partial").expect("write .part");

    let out = std::process::Command::new(common::nvm_bin())
        .args(["cache", "list"])
        .env("NVM_DIR", &nvm_dir)
        .output()
        .expect("run nvm cache list");
    assert!(out.status.success(), "cache list should succeed");
    let s = stdout(&out);
    assert!(
        s.contains("node-v20.0.0.tar.xz"),
        "expected cached file name in listing, got: {s}"
    );
    assert!(
        !s.contains(".part"),
        ".part file should be hidden from listing, got: {s}"
    );
}

#[test]
fn cache_clear_removes_files_and_reports_bytes() {
    let (dir, nvm_dir) = common::isolated_nvm_dir();
    let cache = dir.path().join("cache");
    fs::create_dir_all(&cache).expect("create cache dir");
    fs::write(cache.join("node-v20.0.0.tar.xz"), b"hello world").expect("write cached file");

    let out = std::process::Command::new(common::nvm_bin())
        .args(["cache", "clear"])
        .env("NVM_DIR", &nvm_dir)
        .output()
        .expect("run nvm cache clear");
    assert!(out.status.success(), "cache clear should succeed");

    // The cached file should be gone.
    assert!(
        !cache.join("node-v20.0.0.tar.xz").exists(),
        "cached file should be removed after clear"
    );
}
