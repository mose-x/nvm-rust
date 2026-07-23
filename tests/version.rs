//! Integration tests for `nvm` version output.

mod common;
use common::{run, stdout};

#[test]
fn long_version_flag() {
    let out = run(&["--version"]);
    assert!(out.status.success(), "--version should exit 0");
    let s = stdout(&out);
    // clap reads the version from Cargo.toml; it should print "1.0.0".
    assert!(s.contains("1.0.0"), "expected 1.0.0 in output: {s}");
}

#[test]
fn short_version_flag() {
    let out = run(&["-V"]);
    assert!(out.status.success(), "-V should exit 0");
    let s = stdout(&out);
    assert!(s.contains("1.0.0"), "expected 1.0.0 in output: {s}");
}

#[test]
fn version_subcommand_exits_zero() {
    // `nvm version` shows current node/npm info; with no node installed it
    // should still exit 0 (it prints a status, not an error).
    let out = run(&["version"]);
    assert!(out.status.success(), "version subcommand should exit 0");
}
