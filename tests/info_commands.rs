//! Integration tests for read-only info commands.
//!
//! All run with an isolated `NVM_DIR` so they never touch the user's real
//! `~/.nvm.rust`. These commands should succeed (exit 0) even with an empty
//! install directory.

mod common;
use common::{run_isolated, stdout};

#[test]
fn dir_command_succeeds() {
    let (out, _dir) = run_isolated(&["dir"]);
    assert!(out.status.success(), "dir should exit 0");
    let s = stdout(&out);
    assert!(
        s.contains("NVM") || s.contains("nvm"),
        "dir output empty: {s}"
    );
}

#[test]
fn list_command_succeeds_on_empty_install() {
    let (out, _dir) = run_isolated(&["list"]);
    assert!(out.status.success(), "list should exit 0 on empty install");
}

#[test]
fn current_command_succeeds_with_no_version() {
    // With no version installed, `current` should still exit 0 (prints a
    // "no current version" message rather than erroring).
    let (out, _dir) = run_isolated(&["current"]);
    assert!(out.status.success(), "current should exit 0");
}

#[test]
fn cache_dir_command_succeeds() {
    let (out, _dir) = run_isolated(&["cache", "dir"]);
    assert!(out.status.success(), "cache dir should exit 0");
    let s = stdout(&out);
    assert!(s.contains("cache"), "cache dir output missing 'cache': {s}");
}

#[test]
fn proxy_status_command_succeeds() {
    let (out, _dir) = run_isolated(&["proxy"]);
    assert!(out.status.success(), "proxy status should exit 0");
}

#[test]
fn mirror_status_command_succeeds() {
    let (out, _dir) = run_isolated(&["mirror"]);
    assert!(out.status.success(), "mirror status should exit 0");
}

#[test]
fn alias_list_command_succeeds_on_empty() {
    let (out, _dir) = run_isolated(&["alias"]);
    assert!(out.status.success(), "alias (list) should exit 0 on empty");
}
