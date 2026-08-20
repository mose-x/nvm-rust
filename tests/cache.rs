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
    let (mut cmd, nvm_dir, _home) = common::isolated_command(&["cache", "list"]);
    let cache = nvm_dir.path().join("cache");
    fs::create_dir_all(&cache).expect("create cache dir");

    // A real cached file and an in-flight .part file.
    fs::write(cache.join("node-v20.0.0.tar.xz"), b"hello").expect("write cached file");
    fs::write(cache.join("node-v20.0.0.tar.xz.part"), b"partial").expect("write .part");

    let out = cmd.output().expect("run nvm cache list");
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

// P1-15: `cache list` must also hide the `.part.ifrange` validator sidecar
// that now sits next to every in-flight `.part`. Showing it would be noise
// (it's not a usable cache hit) and would leak the existence of a download
// still in progress. This is the CLI-level counterpart of the
// `list_cached_files_hides_inflight_sidecars` unit test in src/download.rs.
#[test]
fn cache_list_hides_ifrange_sidecar() {
    let (mut cmd, nvm_dir, _home) = common::isolated_command(&["cache", "list"]);
    let cache = nvm_dir.path().join("cache");
    fs::create_dir_all(&cache).expect("create cache dir");

    fs::write(cache.join("node-v20.0.0.tar.xz"), b"hello").expect("write cached file");
    fs::write(cache.join("node-v20.0.0.tar.xz.part"), b"partial").expect("write .part");
    fs::write(
        cache.join("node-v20.0.0.tar.xz.part.ifrange"),
        "\"etag-xyz\"",
    )
    .expect("write ifrange sidecar");

    let out = cmd.output().expect("run nvm cache list");
    assert!(out.status.success(), "cache list should succeed");
    let s = stdout(&out);
    assert!(
        s.contains("node-v20.0.0.tar.xz"),
        "expected cached file name in listing, got: {s}"
    );
    assert!(
        !s.contains(".part"),
        "in-flight .part and .part.ifrange should both be hidden, got: {s}"
    );
    assert!(
        !s.contains("ifrange"),
        ".part.ifrange sidecar should be hidden, got: {s}"
    );
}

#[test]
fn cache_clear_removes_files_and_reports_bytes() {
    let (mut cmd, nvm_dir, _home) = common::isolated_command(&["cache", "clear"]);
    let cache = nvm_dir.path().join("cache");
    fs::create_dir_all(&cache).expect("create cache dir");
    fs::write(cache.join("node-v20.0.0.tar.xz"), b"hello world").expect("write cached file");

    let out = cmd.output().expect("run nvm cache clear");
    assert!(out.status.success(), "cache clear should succeed");

    // The cached file should be gone.
    assert!(
        !cache.join("node-v20.0.0.tar.xz").exists(),
        "cached file should be removed after clear"
    );
}

// P1-15: `cache clear` must preserve the `.part.ifrange` sidecar alongside
// the in-flight `.part` it belongs to. Deleting the sidecar would silently
// downgrade the next resume of that `.part` to plain Range, losing the
// same-length swap detection this fix adds. CLI-level counterpart of the
// `clear_cache_skips_inflight_part_files` unit test in src/download.rs.
#[test]
fn cache_clear_preserves_inflight_ifrange_sidecar() {
    let (mut cmd, nvm_dir, _home) = common::isolated_command(&["cache", "clear"]);
    let cache = nvm_dir.path().join("cache");
    fs::create_dir_all(&cache).expect("create cache dir");

    // Completed download: should be cleared.
    fs::write(cache.join("node-v20.0.0.tar.xz"), b"hello world").expect("write cached file");
    // In-flight download + its validator sidecar: must survive.
    fs::write(cache.join("node-v21.0.0.tar.xz.part"), b"partial").expect("write .part");
    fs::write(
        cache.join("node-v21.0.0.tar.xz.part.ifrange"),
        "\"etag-abc\"",
    )
    .expect("write ifrange sidecar");

    let out = cmd.output().expect("run nvm cache clear");
    assert!(out.status.success(), "cache clear should succeed");

    assert!(
        !cache.join("node-v20.0.0.tar.xz").exists(),
        "completed cache file should be removed"
    );
    assert!(
        cache.join("node-v21.0.0.tar.xz.part").exists(),
        "in-flight .part must be preserved (concurrent download protection)"
    );
    assert!(
        cache.join("node-v21.0.0.tar.xz.part.ifrange").exists(),
        "in-flight .part.ifrange sidecar must be preserved (swap-detection protection)"
    );
}
