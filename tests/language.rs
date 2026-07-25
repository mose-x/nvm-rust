//! Integration tests for `nvm language` (bilingual UI switching).
//!
//! Each test uses a fresh isolated `NVM_DIR` so the language setting never
//! leaks into the user's real config or other tests.

mod common;
use common::{isolated_nvm_dir, nvm_bin, run_isolated, stderr, stdout};
use std::process::Command;

#[test]
fn language_status_succeeds() {
    let (out, _dir) = run_isolated(&["language"]);
    assert!(out.status.success(), "language status should exit 0");
    let s = stdout(&out);
    // Default language is English, so the status line should mention English
    // or the Chinese label — either is fine, we just want non-empty output.
    assert!(!s.trim().is_empty(), "language status output empty");
}

#[test]
fn language_set_cn_then_en_roundtrip() {
    // Set Chinese.
    let (out_cn, _dir_cn) = run_isolated(&["language", "cn"]);
    assert!(out_cn.status.success(), "language cn should exit 0");
    let s_cn = stdout(&out_cn);
    assert!(
        s_cn.contains("中文") || s_cn.contains("cn") || s_cn.contains("语言"),
        "language cn output unexpected: {s_cn}"
    );

    // Set English.
    let (out_en, _dir_en) = run_isolated(&["language", "en"]);
    assert!(out_en.status.success(), "language en should exit 0");
    let s_en = stdout(&out_en);
    assert!(
        s_en.contains("English") || s_en.contains("en") || s_en.contains("Language"),
        "language en output unexpected: {s_en}"
    );
}

#[test]
fn language_alias_zh_sets_chinese() {
    // `zh` is an accepted alias for Chinese (see Lang::from_str).
    let (out, _dir) = run_isolated(&["language", "zh"]);
    assert!(out.status.success(), "language zh should exit 0");
}

#[test]
fn language_invalid_value_exits_nonzero() {
    // An unknown language code should fail (non-zero exit), not silently
    // succeed and corrupt the config.
    let (out, _dir) = run_isolated(&["language", "klingon"]);
    assert!(
        !out.status.success(),
        "language klingon should exit non-zero"
    );
}

/// Regression for the once-per-process corruption-warning gate (P1-13).
///
/// Before the fix, `get_language()` printed "Failed to read language from
/// config" on EVERY `T()` call when `config.json` was malformed. `nvm help`
/// renders dozens of translated strings, so a single corrupt config would
/// flood stderr with the same warning line dozens of times. After the fix,
/// the warning is gated by an `AtomicBool` and appears exactly once.
///
/// We target `nvm help` because `print_help` is pure `T()` output — it never
/// calls `load_config()` itself, so the command does not bail on the corrupt
/// config and actually reaches the many `T()` calls that exercise the gate.
/// A command like `nvm ls` would hit `load_config()` directly, bail with
/// `config_corrupt_hint`, and never reach the spam path.
#[test]
fn corrupt_config_warns_once_across_t_calls() {
    let (dir, nvm_dir) = isolated_nvm_dir();
    // Write a malformed config.json: present on disk but invalid JSON, so
    // `load_config()` returns Err and `get_language()` hits the warning path.
    std::fs::write(dir.path().join("config.json"), "{ not valid json").unwrap();

    let out = Command::new(nvm_bin())
        .arg("help")
        .env("NVM_DIR", &nvm_dir)
        .output()
        .expect("run nvm help");

    // `nvm help` should still succeed (help rendering doesn't need config).
    assert!(
        out.status.success(),
        "nvm help should exit 0 even with corrupt config"
    );

    let err = stderr(&out);
    let count = err.matches("Failed to read language from config").count();
    assert_eq!(
        count, 1,
        "corruption warning should appear exactly once, got {count}:\n{err}"
    );
}
