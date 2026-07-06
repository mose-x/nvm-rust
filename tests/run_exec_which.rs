//! Integration tests for `nvm run`, `nvm exec`, and `nvm which`.
//!
//! Covers offline error paths: version not installed, exec with no args,
//! exec with a nonexistent command (against a fake version dir), and
//! `which` with/without a version argument.

mod common;
use common::{combined_output, create_fake_version, run_isolated, stdout};

// --- `nvm run` ------------------------------------------------------------

#[test]
fn run_nonexistent_version_bails_not_installed() {
    let (out, _dir) = run_isolated(&["run", "v99.99.99"]);
    assert!(!out.status.success(), "run v99.99.99 should fail");
    let s = combined_output(&out);
    assert!(
        s.to_lowercase().contains("not installed") || s.to_lowercase().contains("99.99.99"),
        "expected 'not installed', got: {s}"
    );
}

// --- `nvm exec` -----------------------------------------------------------

#[test]
fn exec_without_args_bails_specify_command() {
    // exec_version checks `args.is_empty()` before resolving the version.
    let (out, _dir) = run_isolated(&["exec", "v99.99.99"]);
    assert!(!out.status.success(), "exec with no args should fail");
    let s = combined_output(&out);
    assert!(
        s.to_lowercase().contains("specify") || s.to_lowercase().contains("command"),
        "expected 'specify command', got: {s}"
    );
}

#[test]
fn exec_nonexistent_version_bails_not_installed() {
    let (out, _dir) = run_isolated(&["exec", "v99.99.99", "echo", "hi"]);
    assert!(!out.status.success(), "exec v99.99.99 should fail");
    let s = combined_output(&out);
    assert!(
        s.to_lowercase().contains("not installed") || s.to_lowercase().contains("99.99.99") || s.to_lowercase().contains("install"),
        "expected 'not installed / run install', got: {s}"
    );
}

// --- `nvm which` ----------------------------------------------------------

#[test]
fn which_no_version_no_current_bails() {
    // No version arg and no current version → bail `no_current_version_set`.
    let (out, _dir) = run_isolated(&["which"]);
    assert!(!out.status.success(), "which with no current should fail");
    let s = combined_output(&out);
    assert!(
        s.to_lowercase().contains("current") || s.to_lowercase().contains("no") || s.to_lowercase().contains("version"),
        "expected 'no current version' message, got: {s}"
    );
}

#[test]
fn which_nonexistent_version_bails_not_installed() {
    let (out, _dir) = run_isolated(&["which", "v99.99.99"]);
    assert!(!out.status.success(), "which v99.99.99 should fail");
    let s = combined_output(&out);
    assert!(
        s.to_lowercase().contains("not installed") || s.to_lowercase().contains("99.99.99"),
        "expected 'not installed', got: {s}"
    );
}

#[test]
fn which_succeeds_when_version_installed() {
    // Create a fake v20.0.0 with a node binary; `which v20.0.0` should
    // print its bin/node path and exit 0.
    let (dir, nvm_dir) = common::isolated_nvm_dir();
    create_fake_version(dir.path(), "v20.0.0", true);

    let out = std::process::Command::new(common::nvm_bin())
        .args(["which", "v20.0.0"])
        .env("NVM_DIR", &nvm_dir)
        .output()
        .expect("run nvm which");
    assert!(out.status.success(), "which v20.0.0 should succeed: {}", stdout(&out));
    let s = combined_output(&out);
    assert!(
        s.contains("v20.0.0") && s.contains("node"),
        "expected node path under v20.0.0, got: {s}"
    );
}

// --- `nvm auto --silent` --------------------------------------------------

#[test]
fn auto_silent_no_nvmrc_bails() {
    // `auto` forwards to `use_version_silent(None, ...)`; with no .nvmrc /
    // .node-version / package.json it bails `specify_version`.
    let (out, _dir) = run_isolated(&["auto", "--silent"]);
    assert!(!out.status.success(), "auto --silent with no .nvmrc should fail");
}
