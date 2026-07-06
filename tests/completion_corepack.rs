//! Integration tests for `nvm completion` and `nvm corepack`.
//!
//! - completion: all four shells write a non-empty file under
//!   `NVM_DIR/completions/`; an unknown shell prints a hint to stderr.
//! - corepack: error paths only (unknown action, status/enable on an
//!   uninstalled version, enable with no current). The real enable/disable
//!   requires a working node+corepack and is not tested here.

mod common;
use common::{combined_output, run_isolated, stderr, stdout};
use std::fs;

// --- `nvm completion` -----------------------------------------------------

#[test]
fn completion_bash_writes_file() {
    let (dir, nvm_dir) = common::isolated_nvm_dir();
    let out = std::process::Command::new(common::nvm_bin())
        .args(["completion", "bash"])
        .env("NVM_DIR", &nvm_dir)
        .output()
        .expect("run nvm completion bash");
    assert!(out.status.success(), "completion bash should succeed: {}", stdout(&out));
    let file = dir.path().join("completions").join("nvm.bash");
    assert!(file.exists(), "nvm.bash should exist");
    let content = fs::read_to_string(&file).expect("read nvm.bash");
    assert!(!content.trim().is_empty(), "nvm.bash should be non-empty");
}

#[test]
fn completion_zsh_writes_file() {
    let (dir, nvm_dir) = common::isolated_nvm_dir();
    let out = std::process::Command::new(common::nvm_bin())
        .args(["completion", "zsh"])
        .env("NVM_DIR", &nvm_dir)
        .output()
        .expect("run nvm completion zsh");
    assert!(out.status.success(), "completion zsh should succeed: {}", stdout(&out));
    let file = dir.path().join("completions").join("_nvm");
    assert!(file.exists(), "_nvm should exist");
    let content = fs::read_to_string(&file).expect("read _nvm");
    assert!(!content.trim().is_empty(), "_nvm should be non-empty");
}

#[test]
fn completion_fish_writes_file() {
    let (dir, nvm_dir) = common::isolated_nvm_dir();
    let out = std::process::Command::new(common::nvm_bin())
        .args(["completion", "fish"])
        .env("NVM_DIR", &nvm_dir)
        .output()
        .expect("run nvm completion fish");
    assert!(out.status.success(), "completion fish should succeed: {}", stdout(&out));
    let file = dir.path().join("completions").join("nvm.fish");
    assert!(file.exists(), "nvm.fish should exist");
    let content = fs::read_to_string(&file).expect("read nvm.fish");
    assert!(!content.trim().is_empty(), "nvm.fish should be non-empty");
}

#[test]
fn completion_powershell_writes_file() {
    let (dir, nvm_dir) = common::isolated_nvm_dir();
    let out = std::process::Command::new(common::nvm_bin())
        .args(["completion", "powershell"])
        .env("NVM_DIR", &nvm_dir)
        .output()
        .expect("run nvm completion powershell");
    assert!(out.status.success(), "completion powershell should succeed: {}", stdout(&out));
    let file = dir.path().join("completions").join("nvm.ps1");
    assert!(file.exists(), "nvm.ps1 should exist");
    let content = fs::read_to_string(&file).expect("read nvm.ps1");
    assert!(!content.trim().is_empty(), "nvm.ps1 should be non-empty");
}

#[test]
fn completion_unknown_shell_prints_hint_exits_zero() {
    // Unknown shell prints an "unsupported shell" hint to stderr and
    // returns Ok(()) (exit 0) — it's a usage hint, not a hard error.
    let (out, _dir) = run_isolated(&["completion", "tcsh"]);
    assert!(out.status.success(), "completion <unknown> should exit 0");
    let err = stderr(&out);
    assert!(
        err.to_lowercase().contains("unsupported") || err.to_lowercase().contains("tcsh") || err.to_lowercase().contains("shell"),
        "expected 'unsupported shell' hint on stderr, got: {err}"
    );
}

// --- `nvm corepack` error paths -------------------------------------------

#[test]
fn corepack_unknown_action_prints_usage_exits_zero() {
    // Unknown action prints `corepack_usage` and returns Ok(()) (exit 0).
    let (out, _dir) = run_isolated(&["corepack", "bogus"]);
    assert!(out.status.success(), "corepack <unknown> should exit 0");
    let s = combined_output(&out);
    assert!(
        !s.trim().is_empty(),
        "corepack usage should be non-empty, got: {s}"
    );
}

#[test]
fn corepack_status_uninstalled_version_bails() {
    let (out, _dir) = run_isolated(&["corepack", "status", "v99.99.99"]);
    assert!(!out.status.success(), "corepack status v99.99.99 should fail");
    let s = combined_output(&out);
    assert!(
        s.to_lowercase().contains("not installed") || s.to_lowercase().contains("99.99.99"),
        "expected 'not installed', got: {s}"
    );
}

#[test]
fn corepack_enable_uninstalled_version_bails() {
    let (out, _dir) = run_isolated(&["corepack", "enable", "v99.99.99"]);
    assert!(!out.status.success(), "corepack enable v99.99.99 should fail");
    let s = combined_output(&out);
    assert!(
        s.to_lowercase().contains("not installed") || s.to_lowercase().contains("99.99.99"),
        "expected 'not installed', got: {s}"
    );
}

#[test]
fn corepack_enable_no_version_no_current_bails() {
    // With no explicit version and no current, corepack_enable bails
    // `no_version_no_current`.
    let (out, _dir) = run_isolated(&["corepack", "enable"]);
    assert!(!out.status.success(), "corepack enable (no current) should fail");
}
