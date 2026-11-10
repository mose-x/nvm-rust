//! Integration tests for `nvm` help output.
//!
//! These cover the four help entry points (`--help`, `-h`, `help`, no args).
//! Uses isolated NVM_DIR + HOME so the real user's config.json (which may
//! have `language: cn`) doesn't affect the output locale.

mod common;
use common::{combined_output, pin_language, stdout};

/// Run `nvm` with isolated NVM_DIR and HOME (no real config.json visible) and
/// the UI language pinned to English, so the assertions below can compare
/// against exact `locales/en.toml` text.
///
/// This used to set `NVM_LANG=en`, which the binary has never read — `grep`
/// finds the name only in this file. The output was English anyway because
/// `Lang::default()` is `EN`, so the dead variable looked like it was working.
fn run_help(args: &[&str]) -> std::process::Output {
    use std::process::Command;
    use tempfile::TempDir;
    let dir = TempDir::new().expect("tempdir");
    pin_language(dir.path(), "en");
    let path = dir.path().as_os_str();
    let mut cmd = Command::new(common::nvm_bin());
    cmd.args(args).env("NVM_DIR", path).env("HOME", path);
    #[cfg(windows)]
    {
        cmd.env("USERPROFILE", path);
    }
    // Keep dir alive for the duration of the call
    let output = cmd.output().expect("failed to run nvm binary");
    drop(dir);
    output
}

#[test]
fn no_args_prints_help_and_exits_zero() {
    let out = run_help(&[]);
    assert!(out.status.success(), "no-args should exit 0");
    let s = stdout(&out);
    assert!(s.contains("Node Version Manager"), "title missing: {s}");
    assert!(s.contains("Usage"), "usage line missing: {s}");
}

#[test]
fn long_help_flag() {
    let out = run_help(&["--help"]);
    assert!(out.status.success(), "--help should exit 0");
    let s = stdout(&out);
    assert!(s.contains("Node Version Manager"));
    assert!(s.contains("install"));
    assert!(s.contains("uninstall"));
    assert!(s.contains("version"));
}

#[test]
fn short_help_flag() {
    let out = run_help(&["-h"]);
    assert!(out.status.success(), "-h should exit 0");
    let s = stdout(&out);
    assert!(s.contains("Node Version Manager"));
}

#[test]
fn help_subcommand() {
    let out = run_help(&["help"]);
    assert!(out.status.success(), "help should exit 0");
    let s = stdout(&out);
    assert!(s.contains("Node Version Manager"));
    assert!(s.contains("Usage"));
}

#[test]
fn help_subcommand_for_install_shows_install_flags() {
    let out = run_help(&["help", "install"]);
    assert!(out.status.success(), "help install should exit 0");
    let s = stdout(&out);
    // All three, not "at least one": a help page that silently dropped `--lts`
    // would still satisfy a disjunction, and the test name promises flags.
    for flag in ["--lts", "--latest", "--source"] {
        assert!(s.contains(flag), "install help is missing {flag}: {s}");
    }
}

#[test]
fn unknown_command_prints_i18n_error() {
    let out = run_help(&["foo"]);
    assert!(
        !out.status.success(),
        "unknown command should exit non-zero"
    );
    let s = combined_output(&out);
    assert!(
        s.contains("Unknown command or argument. Run 'nvm help'"),
        "should print the intercepted unknown-argument message: {s}"
    );
    // Should NOT print clap's English "error:" prefix
    assert!(
        !s.contains("error: The subcommand"),
        "should not print clap's English error: {s}"
    );
}

#[test]
fn unknown_flag_prints_i18n_error() {
    let out = run_help(&["-p"]);
    assert!(!out.status.success(), "unknown flag should exit non-zero");
    let s = combined_output(&out);
    assert!(
        s.contains("Unknown command or argument. Run 'nvm help'"),
        "should print the intercepted unknown-argument message: {s}"
    );
    assert!(
        !s.contains("error: Found argument"),
        "should not print clap's English error: {s}"
    );
}
