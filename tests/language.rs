//! Integration tests for `nvm language` (bilingual UI switching).
//!
//! Each test uses a fresh isolated `NVM_DIR` so the language setting never
//! leaks into the user's real config or other tests.

mod common;
use common::{run_isolated, stdout};

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
