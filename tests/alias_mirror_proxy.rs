//! Integration tests for `nvm alias`, `nvm mirror`, and `nvm proxy`.
//!
//! These cover the full dispatch of each command's sub-actions:
//! - alias: set / lookup-nonexistent / remove-nonexistent / list-empty
//! - mirror: taobao / official / custom URL / empty value / status
//! - proxy: off (writes config) / invalid action (bails). `on` is network-
//!   dependent and intentionally not tested here.

mod common;
use common::{combined_output, create_fake_version, run_isolated, stdout};

// --- `nvm alias` ----------------------------------------------------------

#[test]
fn alias_set_nonexistent_version_bails() {
    // Setting an alias to a version that isn't installed should fail
    // (config.rs set_alias checks version_dir.exists()).
    let (out, _dir) = run_isolated(&["alias", "foo", "v99.99.99"]);
    assert!(!out.status.success(), "alias foo v99.99.99 should fail");
    let s = combined_output(&out);
    assert!(
        s.to_lowercase().contains("not installed") || s.to_lowercase().contains("99.99.99"),
        "expected 'not installed', got: {s}"
    );
}

#[test]
fn alias_lookup_nonexistent_prints_not_found_but_exits_zero() {
    // Looking up an alias that doesn't exist prints "alias not found" but
    // does NOT bail (it's a query, not a mutation).
    let (out, _dir) = run_isolated(&["alias", "nope"]);
    assert!(out.status.success(), "alias nope should exit 0");
    let s = stdout(&out);
    assert!(
        s.to_lowercase().contains("nope") && (s.to_lowercase().contains("not found") || s.to_lowercase().contains("does not exist")),
        "expected 'alias not found', got: {s}"
    );
}

#[test]
fn alias_unalias_nonexistent_bails() {
    // Removing a nonexistent alias bails (config.rs remove_alias).
    let (out, _dir) = run_isolated(&["unalias", "ghost"]);
    assert!(!out.status.success(), "unalias ghost should fail");
    let s = combined_output(&out);
    assert!(
        s.to_lowercase().contains("ghost") && (s.to_lowercase().contains("not found") || s.to_lowercase().contains("does not exist")),
        "expected 'alias not found', got: {s}"
    );
}

#[test]
fn alias_set_then_unalias_roundtrip() {
    // Create a fake installed version, set an alias to it, then remove it.
    let (dir, nvm_dir) = common::isolated_nvm_dir();
    create_fake_version(dir.path(), "v20.0.0", false);

    let set = std::process::Command::new(common::nvm_bin())
        .args(["alias", "myalias", "v20.0.0"])
        .env("NVM_DIR", &nvm_dir)
        .output()
        .expect("run nvm alias set");
    assert!(set.status.success(), "alias set should succeed: {}", stdout(&set));

    let rm = std::process::Command::new(common::nvm_bin())
        .args(["unalias", "myalias"])
        .env("NVM_DIR", &nvm_dir)
        .output()
        .expect("run nvm unalias");
    assert!(rm.status.success(), "unalias should succeed: {}", stdout(&rm));
}

// --- `nvm mirror` ---------------------------------------------------------

#[test]
fn mirror_set_taobao_then_status_shows_it() {
    let (dir, nvm_dir) = common::isolated_nvm_dir();

    let set = std::process::Command::new(common::nvm_bin())
        .args(["mirror", "taobao"])
        .env("NVM_DIR", &nvm_dir)
        .output()
        .expect("run nvm mirror taobao");
    assert!(set.status.success(), "mirror taobao should succeed: {}", stdout(&set));

    let status = std::process::Command::new(common::nvm_bin())
        .args(["mirror"])
        .env("NVM_DIR", dir.path())
        .output()
        .expect("run nvm mirror");
    assert!(status.status.success(), "mirror status should succeed");
    let s = stdout(&status);
    assert!(
        s.contains("npmmirror") || s.contains("taobao"),
        "expected mirror URL in status, got: {s}"
    );
}

#[test]
fn mirror_set_official_clears_mirror() {
    let (dir, nvm_dir) = common::isolated_nvm_dir();

    // Set taobao first, then reset to official.
    let _ = std::process::Command::new(common::nvm_bin())
        .args(["mirror", "taobao"])
        .env("NVM_DIR", &nvm_dir)
        .output()
        .expect("mirror taobao");

    let official = std::process::Command::new(common::nvm_bin())
        .args(["mirror", "official"])
        .env("NVM_DIR", &nvm_dir)
        .output()
        .expect("mirror official");
    assert!(official.status.success(), "mirror official should succeed: {}", stdout(&official));

    let status = std::process::Command::new(common::nvm_bin())
        .args(["mirror"])
        .env("NVM_DIR", dir.path())
        .output()
        .expect("mirror status");
    let s = stdout(&status);
    assert!(
        s.contains("nodejs.org") || s.contains("official"),
        "expected official URL after reset, got: {s}"
    );
}

#[test]
fn mirror_set_custom_url_persists() {
    let (dir, nvm_dir) = common::isolated_nvm_dir();
    let custom = "https://my-mirror.example.com/node/";

    let set = std::process::Command::new(common::nvm_bin())
        .args(["mirror", custom])
        .env("NVM_DIR", &nvm_dir)
        .output()
        .expect("mirror custom");
    assert!(set.status.success(), "mirror custom should succeed: {}", stdout(&set));

    let status = std::process::Command::new(common::nvm_bin())
        .args(["mirror"])
        .env("NVM_DIR", dir.path())
        .output()
        .expect("mirror status");
    let s = stdout(&status);
    assert!(
        s.contains("my-mirror.example.com"),
        "expected custom URL in status, got: {s}"
    );
}

#[test]
fn mirror_empty_value_bails() {
    // An empty mirror value should bail with `mirror_url_empty`.
    let (out, _dir) = run_isolated(&["mirror", "   "]);
    assert!(!out.status.success(), "mirror with empty value should fail");
}

// --- `nvm proxy` ----------------------------------------------------------

#[test]
fn proxy_off_succeeds_and_writes_config() {
    // `proxy off` just sets proxy_enabled=false in config; no network.
    let (out, _dir) = run_isolated(&["proxy", "off"]);
    assert!(out.status.success(), "proxy off should succeed");
    let s = stdout(&out);
    assert!(
        s.to_lowercase().contains("off") || s.to_lowercase().contains("direct") || s.to_lowercase().contains("disabled"),
        "expected 'off/disabled' message, got: {s}"
    );
}

#[test]
fn proxy_invalid_action_bails() {
    // An unknown action should bail with `unknown_action`.
    let (out, _dir) = run_isolated(&["proxy", "bogus"]);
    assert!(!out.status.success(), "proxy bogus should fail");
    let s = combined_output(&out);
    assert!(
        s.to_lowercase().contains("unknown") || s.to_lowercase().contains("bogus"),
        "expected 'unknown action', got: {s}"
    );
}
