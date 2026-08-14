//! Integration tests for `nvm` version output.

mod common;
use common::{run, stdout};

#[test]
fn long_version_flag() {
    let out = run(&["--version"]);
    assert!(out.status.success(), "--version should exit 0");
    let s = stdout(&out);
    // clap reads the version from Cargo.toml; match whatever Cargo.toml says.
    let expected = env!("CARGO_PKG_VERSION");
    assert!(s.contains(expected), "expected {expected} in output: {s}");
}

#[test]
fn short_version_flag() {
    let out = run(&["-V"]);
    assert!(out.status.success(), "-V should exit 0");
    let s = stdout(&out);
    let expected = env!("CARGO_PKG_VERSION");
    assert!(s.contains(expected), "expected {expected} in output: {s}");
}

#[test]
fn lowercase_v_flag() {
    let out = run(&["-v"]);
    assert!(out.status.success(), "-v should exit 0");
    let s = stdout(&out);
    let expected = env!("CARGO_PKG_VERSION");
    assert!(s.contains(expected), "expected {expected} in output: {s}");
}

#[test]
fn version_subcommand_exits_zero() {
    // `nvm version` shows current node/npm info; with no node installed it
    // should still exit 0 (it prints a status, not an error).
    let out = run(&["version"]);
    assert!(out.status.success(), "version subcommand should exit 0");
}
