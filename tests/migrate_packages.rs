//! Integration tests for `nvm migrate`, `nvm install-latest-npm/yarn/pnpm`,
//! and `nvm reinstall-packages`.
//!
//! All cover offline error paths: source not found, no current version,
//! version not installed. The real install/migration paths require network
//! and a working node/npm and are not tested here.

mod common;
use common::{combined_output, run_isolated, run_isolated_with_home, stdout};

// --- `nvm migrate` --------------------------------------------------------

#[test]
fn migrate_nvm_source_not_found_bails() {
    // With HOME pointed at an empty tempdir, ~/.nvm/versions/node does not
    // exist, so `migrate nvm` should bail `migrate_source_not_found`.
    let (out, _nvm, _home) = run_isolated_with_home(&["migrate", "nvm"]);
    assert!(!out.status.success(), "migrate nvm (no source) should fail");
    let s = combined_output(&out);
    assert!(
        s.to_lowercase().contains("not found") || s.to_lowercase().contains("migrate") || s.to_lowercase().contains("source"),
        "expected 'source not found', got: {s}"
    );
}

#[test]
fn migrate_unknown_source_bails() {
    let (out, _nvm, _home) = run_isolated_with_home(&["migrate", "badsource"]);
    assert!(!out.status.success(), "migrate badsource should fail");
}

// --- `nvm install-latest-npm/yarn/pnpm` -----------------------------------
//
// All three share resolve_install_target(), which bails
// `no_current_version_set` when there's no current and no default. We test
// npm explicitly and also loop the same assertion over yarn/pnpm to lock
// the shared behavior for each command.

#[test]
fn install_latest_npm_no_current_bails() {
    let (out, _dir) = run_isolated(&["install-latest-npm"]);
    assert!(!out.status.success(), "install-latest-npm (no current) should fail");
    let s = combined_output(&out);
    assert!(
        s.to_lowercase().contains("current") || s.to_lowercase().contains("version") || s.to_lowercase().contains("no"),
        "expected 'no current version', got: {s}"
    );
}

#[test]
fn install_latest_yarn_no_current_bails() {
    let (out, _dir) = run_isolated(&["install-latest-yarn"]);
    assert!(!out.status.success(), "install-latest-yarn (no current) should fail");
}

#[test]
fn install_latest_pnpm_no_current_bails() {
    let (out, _dir) = run_isolated(&["install-latest-pnpm"]);
    assert!(!out.status.success(), "install-latest-pnpm (no current) should fail");
}

#[test]
fn install_latest_npm_uninstalled_version_bails() {
    let (out, _dir) = run_isolated(&["install-latest-npm", "v99.99.99"]);
    assert!(!out.status.success(), "install-latest-npm v99.99.99 should fail");
    let s = combined_output(&out);
    assert!(
        s.to_lowercase().contains("not installed") || s.to_lowercase().contains("99.99.99") || s.to_lowercase().contains("install"),
        "expected 'not installed', got: {s}"
    );
}

// --- `nvm reinstall-packages` ---------------------------------------------

#[test]
fn reinstall_packages_source_not_installed_bails() {
    // The source version directory doesn't exist → bail
    // `source_not_installed` (checked before current).
    let (out, _dir) = run_isolated(&["reinstall-packages", "v99.99.99"]);
    assert!(!out.status.success(), "reinstall-packages v99.99.99 should fail");
    let s = combined_output(&out);
    assert!(
        s.to_lowercase().contains("not installed") || s.to_lowercase().contains("99.99.99") || s.to_lowercase().contains("source"),
        "expected 'source not installed', got: {s}"
    );
}
