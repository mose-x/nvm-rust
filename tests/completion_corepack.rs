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
    assert!(
        out.status.success(),
        "completion bash should succeed: {}",
        stdout(&out)
    );
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
    assert!(
        out.status.success(),
        "completion zsh should succeed: {}",
        stdout(&out)
    );
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
    assert!(
        out.status.success(),
        "completion fish should succeed: {}",
        stdout(&out)
    );
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
    assert!(
        out.status.success(),
        "completion powershell should succeed: {}",
        stdout(&out)
    );
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
        err.to_lowercase().contains("unsupported")
            || err.to_lowercase().contains("tcsh")
            || err.to_lowercase().contains("shell"),
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
    assert!(
        !out.status.success(),
        "corepack status v99.99.99 should fail"
    );
    let s = combined_output(&out);
    assert!(
        s.to_lowercase().contains("not installed") || s.to_lowercase().contains("99.99.99"),
        "expected 'not installed', got: {s}"
    );
}

#[test]
fn corepack_enable_uninstalled_version_bails() {
    let (out, _dir) = run_isolated(&["corepack", "enable", "v99.99.99"]);
    assert!(
        !out.status.success(),
        "corepack enable v99.99.99 should fail"
    );
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
    assert!(
        !out.status.success(),
        "corepack enable (no current) should fail"
    );
}

// --- `nvm corepack` with `system:` current (P1-8) ------------------------
//
// When `nvm use system` writes `system:v20.0.0` to the `current` file, the
// corepack subcommands used to bail with `not_installed` because they joined
// `nvm_dir/system:v20.0.0` and found no `node` binary there. These tests
// pin the fixed behaviour: status reports the system install, while
// enable/disable refuse to mutate system-wide state.
//
// We write `system:v20.0.0` to `current` directly (bypassing resolve_alias)
// so the tests do not depend on a real system Node.js being on PATH — the
// fix under test is the `system:` prefix handling in corepack.rs, not the
// alias resolution.

/// Helper: create an isolated NVM_DIR with `current` set to `system:v20.0.0`.
fn isolated_with_system_current() -> (std::process::Output, tempfile::TempDir) {
    let (dir, nvm_dir) = common::isolated_nvm_dir();
    let current_file = dir.path().join("current");
    std::fs::write(&current_file, "system:v20.0.0").expect("write current file");
    let out = std::process::Command::new(common::nvm_bin())
        .args(["corepack", "status"])
        .env("NVM_DIR", &nvm_dir)
        .output()
        .expect("run nvm corepack status");
    (out, dir)
}

#[test]
fn corepack_status_no_arg_with_system_current_succeeds() {
    let (out, _dir) = isolated_with_system_current();
    assert!(
        out.status.success(),
        "corepack status (current=system:) should succeed, got: {}",
        combined_output(&out)
    );
    // The system status output mentions either the system corepack version
    // or the 'no version selected' fallback — both are acceptable. What is
    // NOT acceptable is the old `not_installed` bail.
    let s = combined_output(&out);
    assert!(
        !s.to_lowercase().contains("not installed"),
        "should not bail with 'not installed' for system current, got: {s}"
    );
    // Should mention corepack somewhere in the output.
    assert!(
        s.to_lowercase().contains("corepack"),
        "expected 'corepack' in output, got: {s}"
    );
}

#[test]
fn corepack_enable_no_arg_with_system_current_bails_with_clear_message() {
    let (dir, nvm_dir) = common::isolated_nvm_dir();
    let current_file = dir.path().join("current");
    std::fs::write(&current_file, "system:v20.0.0").expect("write current file");
    let out = std::process::Command::new(common::nvm_bin())
        .args(["corepack", "enable"])
        .env("NVM_DIR", &nvm_dir)
        .output()
        .expect("run nvm corepack enable");
    assert!(
        !out.status.success(),
        "corepack enable (current=system:) should refuse, got: {}",
        combined_output(&out)
    );
    let s = combined_output(&out);
    // The clear error message must mention 'system' and read as a refusal
    // (cannot / 无法), not the old generic 'not installed'.
    assert!(
        s.to_lowercase().contains("system"),
        "expected 'system' in refusal message, got: {s}"
    );
    assert!(
        !s.to_lowercase().contains("not installed"),
        "should not surface the old 'not installed' error for system current, got: {s}"
    );
}

#[test]
fn corepack_disable_no_arg_with_system_current_bails_with_clear_message() {
    let (dir, nvm_dir) = common::isolated_nvm_dir();
    let current_file = dir.path().join("current");
    std::fs::write(&current_file, "system:v20.0.0").expect("write current file");
    let out = std::process::Command::new(common::nvm_bin())
        .args(["corepack", "disable"])
        .env("NVM_DIR", &nvm_dir)
        .output()
        .expect("run nvm corepack disable");
    assert!(
        !out.status.success(),
        "corepack disable (current=system:) should refuse, got: {}",
        combined_output(&out)
    );
    let s = combined_output(&out);
    assert!(
        s.to_lowercase().contains("system"),
        "expected 'system' in refusal message, got: {s}"
    );
    assert!(
        !s.to_lowercase().contains("not installed"),
        "should not surface the old 'not installed' error for system current, got: {s}"
    );
}

/// `nvm corepack status system` (explicit arg) should also work when a
/// system Node.js is detectable on PATH. This test is skipped when `which
/// node` finds nothing (e.g. minimal CI without Node) because resolve_alias
/// would then bail with `system_node_not_found` before reaching the fix.
#[test]
fn corepack_status_system_arg_succeeds_when_node_on_path() {
    let node_on_path = std::process::Command::new(if cfg!(windows) { "where" } else { "which" })
        .arg("node")
        .output()
        .map(|o| o.status.success() && !String::from_utf8_lossy(&o.stdout).trim().is_empty())
        .unwrap_or(false);
    if !node_on_path {
        eprintln!("skipping: no system node on PATH");
        return;
    }

    let (out, _dir) = run_isolated(&["corepack", "status", "system"]);
    assert!(
        out.status.success(),
        "corepack status system should succeed when node is on PATH, got: {}",
        combined_output(&out)
    );
    let s = combined_output(&out);
    assert!(
        !s.to_lowercase().contains("not installed"),
        "should not bail with 'not installed' for explicit system arg, got: {s}"
    );
}

/// `nvm corepack enable system` must refuse (we don't mutate system state).
/// Also skipped when no system node is on PATH, for the same reason as above.
#[test]
fn corepack_enable_system_arg_refuses_when_node_on_path() {
    let node_on_path = std::process::Command::new(if cfg!(windows) { "where" } else { "which" })
        .arg("node")
        .output()
        .map(|o| o.status.success() && !String::from_utf8_lossy(&o.stdout).trim().is_empty())
        .unwrap_or(false);
    if !node_on_path {
        eprintln!("skipping: no system node on PATH");
        return;
    }

    let (out, _dir) = run_isolated(&["corepack", "enable", "system"]);
    assert!(
        !out.status.success(),
        "corepack enable system should refuse even when node is on PATH, got: {}",
        combined_output(&out)
    );
    let s = combined_output(&out);
    assert!(
        s.to_lowercase().contains("system"),
        "expected 'system' in refusal message, got: {s}"
    );
    assert!(
        !s.to_lowercase().contains("not installed"),
        "should not surface the old 'not installed' error for explicit system arg, got: {s}"
    );
}
