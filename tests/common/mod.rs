//! Shared helpers for integration tests.
//!
//! Each integration test runs the freshly built `nvm` binary as a subprocess
//! with `NVM_DIR` pointed at a throwaway tempdir, so tests never touch the
//! user's real `~/.nvm` / `~/.nvm.rust` state.
//!
//! `#![allow(dead_code)]` is needed because each integration test file
//! compiles this module independently and not every helper is used by every
//! test file.

#![allow(dead_code)]

use std::env;
use std::ffi::OsString;
use std::path::Path;
use std::process::{Command, Output};
use tempfile::TempDir;

/// Path to the `nvm` binary built by `cargo test`.
///
/// `CARGO_BIN_EXE_<name>` is set automatically by cargo (Rust 1.43+) for
/// integration tests and points at the compiled binary under
/// `target/debug` (or `target/release` with `--release`).
pub fn nvm_bin() -> &'static str {
    env!("CARGO_BIN_EXE_nvm")
}

/// A scratch `NVM_DIR` together with a guard that keeps the tempdir alive.
///
/// Drop the returned `TempDir` to clean up.
pub fn isolated_nvm_dir() -> (TempDir, OsString) {
    let dir = TempDir::new().expect("failed to create tempdir for NVM_DIR");
    let path = dir.path().as_os_str().to_os_string();
    (dir, path)
}

/// Run `nvm` with the given args and an isolated `NVM_DIR`.
///
/// The returned `TempDir` owns the scratch directory; keep it alive for the
/// duration of the assertions (binding it to `_dir` is enough).
pub fn run_isolated(args: &[&str]) -> (Output, TempDir) {
    let (dir, nvm_dir) = isolated_nvm_dir();
    let output = Command::new(nvm_bin())
        .args(args)
        .env("NVM_DIR", &nvm_dir)
        .output()
        .expect("failed to run nvm binary");
    (output, dir)
}

/// Run `nvm` with both `NVM_DIR` and `HOME` pointed at isolated tempdirs.
///
/// Used by tests that must also isolate `~/.nvm` lookups (e.g. `migrate`,
/// which resolves the nvm-sh source under `~/.nvm/versions/node` via
/// `get_home_dir()`). Returns both guards so the caller can keep them alive.
pub fn run_isolated_with_home(args: &[&str]) -> (Output, TempDir, TempDir) {
    let nvm = TempDir::new().expect("tempdir for NVM_DIR");
    let home = TempDir::new().expect("tempdir for HOME");
    let output = Command::new(nvm_bin())
        .args(args)
        .env("NVM_DIR", nvm.path())
        .env("HOME", home.path())
        .output()
        .expect("failed to run nvm binary");
    (output, nvm, home)
}

/// Run `nvm` inheriting the current process's environment (used for pure
/// help/version checks that never touch NVM_DIR).
pub fn run(args: &[&str]) -> Output {
    Command::new(nvm_bin())
        .args(args)
        .output()
        .expect("failed to run nvm binary")
}

/// Create a fake installed version directory (e.g. `NVM_DIR/v20.0.0/bin/`)
/// so commands that check `version_dir.exists()` succeed without a real
/// download. Optionally place a (empty) `node` binary under `bin/`.
pub fn create_fake_version(nvm_dir: &Path, version: &str, with_node: bool) {
    let bin = nvm_dir.join(version).join("bin");
    std::fs::create_dir_all(&bin).expect("create fake version bin/");
    if with_node {
        // `exe_path` on Windows looks for `node.exe` first, then `node.cmd`,
        // then a bare `node`. The previous test only wrote a bare `node`,
        // which `exe_path` falls back to — but `nvm which` then reported the
        // path and downstream existence checks behaved inconsistently, so
        // `which_succeeds_when_version_installed` failed on windows-latest.
        // Write the platform-appropriate filename so the lookup resolves on
        // the first candidate everywhere.
        if cfg!(windows) {
            std::fs::write(bin.join("node.exe"), b"fake").expect("write fake node.exe");
        } else {
            std::fs::write(bin.join("node"), b"#!/bin/sh\nexit 0\n").expect("write fake node");
        }
    }
}

/// Decode a `Output`'s stdout as UTF-8 (lossy), for substring assertions.
pub fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).to_string()
}

/// Decode a `Output`'s stderr as UTF-8 (lossy).
pub fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).to_string()
}

/// Decode stdout + stderr concatenated (lossy). Use this when the test
/// only cares that some message appeared somewhere, because anyhow's
/// `Error: ...` report is written to stderr while normal `println!` output
/// goes to stdout — assertions that scan only one of the two are brittle.
pub fn combined_output(out: &Output) -> String {
    let mut s = String::from_utf8_lossy(&out.stdout).to_string();
    s.push('\n');
    s.push_str(&String::from_utf8_lossy(&out.stderr));
    s
}
