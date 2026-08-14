//! Integration tests for `nvm` help output.
//!
//! These cover the four help entry points (`--help`, `-h`, `help`, no args).
//! They do not touch `NVM_DIR`, so no isolation is needed.

mod common;
use common::{combined_output, run, stdout};

#[test]
fn no_args_prints_help_and_exits_zero() {
    let out = run(&[]);
    assert!(out.status.success(), "no-args should exit 0");
    let s = stdout(&out);
    assert!(s.contains("Node Version Manager"), "title missing: {s}");
    assert!(s.contains("Usage"), "usage line missing: {s}");
}

#[test]
fn long_help_flag() {
    let out = run(&["--help"]);
    assert!(out.status.success(), "--help should exit 0");
    let s = stdout(&out);
    assert!(s.contains("Node Version Manager"));
    assert!(s.contains("install"));
    assert!(s.contains("uninstall"));
    assert!(s.contains("version"));
}

#[test]
fn short_help_flag() {
    let out = run(&["-h"]);
    assert!(out.status.success(), "-h should exit 0");
    let s = stdout(&out);
    assert!(s.contains("Node Version Manager"));
}

#[test]
fn help_subcommand() {
    let out = run(&["help"]);
    assert!(out.status.success(), "help should exit 0");
    let s = stdout(&out);
    assert!(s.contains("Node Version Manager"));
    assert!(s.contains("Usage"));
}

#[test]
fn help_subcommand_for_install_shows_install_flags() {
    let out = run(&["help", "install"]);
    assert!(out.status.success(), "help install should exit 0");
    let s = stdout(&out);
    // Should mention at least one install flag.
    assert!(
        s.contains("--lts") || s.contains("--latest") || s.contains("--source"),
        "install help missing flags: {s}"
    );
}

#[test]
fn unknown_command_prints_i18n_error() {
    let out = run(&["foo"]);
    assert!(
        !out.status.success(),
        "unknown command should exit non-zero"
    );
    let s = combined_output(&out);
    assert!(
        s.contains("nvm help") || s.contains("Unknown") || s.contains("未知"),
        "should mention 'nvm help' or 'unknown' in error: {s}"
    );
    // Should NOT print clap's English "error:" prefix
    assert!(
        !s.contains("error: The subcommand"),
        "should not print clap's English error: {s}"
    );
}

#[test]
fn unknown_flag_prints_i18n_error() {
    let out = run(&["-p"]);
    assert!(!out.status.success(), "unknown flag should exit non-zero");
    let s = combined_output(&out);
    assert!(
        s.contains("nvm help") || s.contains("Unknown") || s.contains("未知"),
        "should mention 'nvm help' or 'unknown' in error: {s}"
    );
    assert!(
        !s.contains("error: Found argument"),
        "should not print clap's English error: {s}"
    );
}
