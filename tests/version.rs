//! Integration tests for `nvm` version output.
//!
//! Uses isolated NVM_DIR + HOME so the real user's config.json doesn't
//! affect locale.

mod common;
use common::stdout;

/// Run `nvm` with isolated NVM_DIR + HOME (no real config visible).
fn run_version(args: &[&str]) -> std::process::Output {
    use std::process::Command;
    use tempfile::TempDir;
    let dir = TempDir::new().expect("tempdir");
    let path = dir.path().as_os_str();
    let mut cmd = Command::new(common::nvm_bin());
    cmd.args(args).env("NVM_DIR", path).env("HOME", path);
    #[cfg(windows)]
    {
        cmd.env("USERPROFILE", path);
    }
    let output = cmd.output().expect("failed to run nvm binary");
    drop(dir);
    output
}

#[test]
fn long_version_flag() {
    let out = run_version(&["--version"]);
    assert!(out.status.success(), "--version should exit 0");
    let s = stdout(&out);
    // clap reads the version from Cargo.toml; match whatever Cargo.toml says.
    let expected = env!("CARGO_PKG_VERSION");
    assert!(s.contains(expected), "expected {expected} in output: {s}");
}

#[test]
fn short_version_flag() {
    let out = run_version(&["-V"]);
    assert!(out.status.success(), "-V should exit 0");
    let s = stdout(&out);
    let expected = env!("CARGO_PKG_VERSION");
    assert!(s.contains(expected), "expected {expected} in output: {s}");
}

#[test]
fn lowercase_v_flag() {
    let out = run_version(&["-v"]);
    assert!(out.status.success(), "-v should exit 0");
    let s = stdout(&out);
    let expected = env!("CARGO_PKG_VERSION");
    assert!(s.contains(expected), "expected {expected} in output: {s}");
}

#[test]
fn version_subcommand_exits_zero() {
    // `nvm version` shows current node/npm info; with no node installed it
    // should still exit 0 (it prints a status, not an error).
    let out = run_version(&["version"]);
    assert!(out.status.success(), "version subcommand should exit 0");
}
