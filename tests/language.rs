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
    // ONE NVM_DIR for both steps. This previously called `run_isolated` twice,
    // which mints a fresh tempdir per call, so the second command could never
    // see what the first had written — it tested two independent sets and
    // called the result a round-trip.
    let (dir, nvm_dir) = isolated_nvm_dir();
    let config = dir.path().join("config.json");

    let set = |lang: &str| {
        let mut cmd = Command::new(nvm_bin());
        cmd.args(["language", lang])
            .env("NVM_DIR", &nvm_dir)
            .env("HOME", &nvm_dir);
        #[cfg(windows)]
        cmd.env("USERPROFILE", &nvm_dir);
        cmd.output()
            .unwrap_or_else(|e| panic!("run `nvm language {lang}`: {e}"))
    };
    let stored_language = || -> String {
        let raw = std::fs::read_to_string(&config)
            .expect("config.json must exist after `nvm language <x>`");
        serde_json::from_str::<serde_json::Value>(&raw)
            .expect("config.json must be valid JSON")
            .get("language")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string()
    };

    // The persisted value is the contract; the printed confirmation is rendered
    // in the language just selected, so it also proves the switch took effect
    // in-process rather than only on disk.
    let out_cn = set("cn");
    assert!(out_cn.status.success(), "language cn should exit 0");
    assert_eq!(stored_language(), "cn", "config.json must record `cn`");
    assert!(
        stdout(&out_cn).contains("中文"),
        "the confirmation should be rendered in Chinese: {}",
        stdout(&out_cn)
    );

    let out_en = set("en");
    assert!(out_en.status.success(), "language en should exit 0");
    assert_eq!(
        stored_language(),
        "en",
        "the second set must overwrite the first in the same config"
    );
    assert!(
        stdout(&out_en).contains("English"),
        "the confirmation should switch back to English: {}",
        stdout(&out_en)
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
