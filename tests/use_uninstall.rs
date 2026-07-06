//! Integration tests for `nvm use` and `nvm uninstall` error paths and
//! flag dispatch.
//!
//! All offline: we exercise the failure paths (version not installed,
//! conflicting flags, empty install state) rather than real installs.

mod common;
use common::{combined_output, create_fake_version, run_isolated, stdout};

// --- `nvm use` error paths -------------------------------------------------

#[test]
fn use_no_arg_no_nvmrc_bails_with_specify_version() {
    // No version arg and no .nvmrc / .node-version / package.json in cwd
    // should bail with `specify_version`.
    let (out, _dir) = run_isolated(&["use"]);
    assert!(!out.status.success(), "use (no arg) should fail");
    let s = combined_output(&out);
    assert!(
        s.to_lowercase().contains("specify") || s.to_lowercase().contains("version"),
        "expected a 'specify version' hint, got: {s}"
    );
}

#[test]
fn use_nonexistent_version_bails_not_installed() {
    let (out, _dir) = run_isolated(&["use", "v99.99.99"]);
    assert!(!out.status.success(), "use v99.99.99 should fail");
    let s = combined_output(&out);
    assert!(
        s.to_lowercase().contains("not installed") || s.to_lowercase().contains("99.99.99"),
        "expected 'not installed' for v99.99.99, got: {s}"
    );
}

#[test]
fn use_succeeds_when_version_dir_exists() {
    // Create a fake v20.0.0 with a node binary so `use` switches to it.
    let (dir, nvm_dir) = common::isolated_nvm_dir();
    create_fake_version(dir.path(), "v20.0.0", true);

    let out = std::process::Command::new(common::nvm_bin())
        .arg("use")
        .arg("v20.0.0")
        .env("NVM_DIR", &nvm_dir)
        .output()
        .expect("run nvm use");
    assert!(out.status.success(), "use v20.0.0 should succeed: {}", stdout(&out));
}

// --- `nvm uninstall` error paths ------------------------------------------

#[test]
fn uninstall_nonexistent_version_bails_not_installed() {
    let (out, _dir) = run_isolated(&["uninstall", "v99.99.99"]);
    assert!(!out.status.success(), "uninstall v99.99.99 should fail");
    let s = combined_output(&out);
    assert!(
        s.to_lowercase().contains("not installed") || s.to_lowercase().contains("99.99.99"),
        "expected 'not installed' for uninstall v99.99.99, got: {s}"
    );
}

// --- `nvm uninstall --lts` / `--latest` empty state -----------------------

#[test]
fn uninstall_lts_empty_install_bails() {
    let (out, _dir) = run_isolated(&["uninstall", "--lts"]);
    assert!(!out.status.success(), "uninstall --lts on empty should fail");
}

#[test]
fn uninstall_latest_empty_install_bails() {
    let (out, _dir) = run_isolated(&["uninstall", "--latest"]);
    assert!(!out.status.success(), "uninstall --latest on empty should fail");
}

// --- `nvm uninstall` conflicting flag dispatch (main.rs match arm) --------
//
// main.rs:45-52 matches (version, lts, latest): only (Some, false, false),
// (None, true, false), (None, false, true) are valid. Any other combination
// (e.g. both --lts and --latest, or a version plus a flag) should bail with
// `specify_version_or_lts`.

#[test]
fn uninstall_both_lts_and_latest_bails() {
    let (out, _dir) = run_isolated(&["uninstall", "--lts", "--latest"]);
    assert!(!out.status.success(), "--lts --latest should fail");
}

#[test]
fn uninstall_version_and_lts_bails() {
    let (out, _dir) = run_isolated(&["uninstall", "v1.0.0", "--lts"]);
    assert!(!out.status.success(), "<ver> --lts should fail");
}

#[test]
fn uninstall_version_and_latest_bails() {
    let (out, _dir) = run_isolated(&["uninstall", "v1.0.0", "--latest"]);
    assert!(!out.status.success(), "<ver> --latest should fail");
}

// --- `nvm deactivate` empty state (no-op, should succeed) -----------------

#[test]
fn deactivate_with_no_current_succeeds() {
    // deactivate guards on `current` file existence, so it's a safe no-op
    // when nothing is active.
    let (out, _dir) = run_isolated(&["deactivate"]);
    assert!(out.status.success(), "deactivate with no current should succeed");
}
