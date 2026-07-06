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

/// Run `nvm` inheriting the current process's environment (used for pure
/// help/version checks that never touch NVM_DIR).
pub fn run(args: &[&str]) -> Output {
    Command::new(nvm_bin())
        .args(args)
        .output()
        .expect("failed to run nvm binary")
}

/// Decode a `Output`'s stdout as UTF-8 (lossy), for substring assertions.
pub fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).to_string()
}

/// Decode a `Output`'s stderr as UTF-8 (lossy).
pub fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).to_string()
}
