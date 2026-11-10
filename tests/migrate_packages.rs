//! Integration tests for `nvm migrate`, `nvm install-npm/yarn/pnpm`,
//! and `nvm reinstall-packages`.
//!
//! All cover offline error paths: source not found, no current version,
//! version not installed. The real install path requires network and a working
//! node/npm; the real migration path is covered end to end in
//! `tests/migrate_nvm_sh.rs`.
//!
//! Every test pins the UI language to English (`run_isolated*_en`) so the
//! message assertions below are exact. They previously used disjunctions like
//! `contains("not found") || contains("migrate") || contains("source")`, which
//! any output merely mentioning the command name would satisfy — a two-letter
//! `contains("no")` matched "node", "not" and "non-default" alike, so those
//! assertions passed on the wrong error and on each other's output.

mod common;
use common::{combined_output, run_isolated_en, run_isolated_with_home, run_isolated_with_home_en};

// --- `nvm migrate` --------------------------------------------------------

#[test]
fn migrate_nvm_source_not_found_bails() {
    // With HOME pointed at an empty tempdir, ~/.nvm/versions/node does not
    // exist, so `migrate nvm` should bail `migrate_source_not_found`.
    let (out, _nvm, _home) = run_isolated_with_home_en(&["migrate", "nvm"]);
    assert!(!out.status.success(), "migrate nvm (no source) should fail");
    let s = combined_output(&out);
    // `NVM_SH_HOME` is not translated, so it anchors the message independently
    // of wording — and it is the actionable part of the hint.
    assert!(
        s.contains("Migration source 'nvm' not found") && s.contains("NVM_SH_HOME"),
        "expected the migrate_source_not_found hint, got: {s}"
    );
}

#[test]
fn migrate_unknown_source_bails() {
    let (out, _nvm, _home) = run_isolated_with_home(&["migrate", "badsource"]);
    assert!(!out.status.success(), "migrate badsource should fail");
}

// --- `nvm install-npm/yarn/pnpm` -----------------------------------
//
// All three share resolve_install_target(), which bails
// `no_current_version_set` when there's no current and no default. We test
// npm explicitly and also loop the same assertion over yarn/pnpm to lock
// the shared behavior for each command.

#[test]
fn install_latest_npm_no_current_bails() {
    let (out, _dir) = run_isolated_en(&["install-npm"]);
    assert!(
        !out.status.success(),
        "install-npm (no current) should fail"
    );
    let s = combined_output(&out);
    assert!(
        s.contains("No current version set"),
        "expected no_current_version_set, got: {s}"
    );
}

#[test]
fn install_latest_yarn_no_current_bails() {
    let (out, _dir) = run_isolated_en(&["install-yarn"]);
    assert!(
        !out.status.success(),
        "install-yarn (no current) should fail"
    );
    assert!(
        combined_output(&out).contains("No current version set"),
        "yarn shares resolve_install_target, so it must report the same error"
    );
}

#[test]
fn install_latest_pnpm_no_current_bails() {
    let (out, _dir) = run_isolated_en(&["install-pnpm"]);
    assert!(
        !out.status.success(),
        "install-pnpm (no current) should fail"
    );
    assert!(
        combined_output(&out).contains("No current version set"),
        "pnpm shares resolve_install_target, so it must report the same error"
    );
}

#[test]
fn install_latest_npm_uninstalled_version_bails() {
    let (out, _dir) = run_isolated_en(&["install-npm", "v99.99.99"]);
    assert!(!out.status.success(), "install-npm v99.99.99 should fail");
    let s = combined_output(&out);
    // Distinguished from `reinstall-packages` below, which reports
    // "Source version …": both name the version, only one says "Source".
    assert!(
        s.contains("Version v99.99.99 is not installed") && !s.contains("Source version"),
        "expected not_installed for the requested version, got: {s}"
    );
}

// --- `nvm reinstall-packages` ---------------------------------------------

#[test]
fn reinstall_packages_source_not_installed_bails() {
    // The source version directory doesn't exist → bail
    // `source_not_installed` (checked before current).
    let (out, _dir) = run_isolated_en(&["reinstall-packages", "v99.99.99"]);
    assert!(
        !out.status.success(),
        "reinstall-packages v99.99.99 should fail"
    );
    let s = combined_output(&out);
    assert!(
        s.contains("Source version v99.99.99 is not installed"),
        "expected source_not_installed, got: {s}"
    );
}
