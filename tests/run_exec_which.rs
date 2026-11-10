//! Integration tests for `nvm run`, `nvm exec`, and `nvm which`.
//!
//! Covers offline error paths: version not installed, exec with no args,
//! exec with a nonexistent command (against a fake version dir), and
//! `which` with/without a version argument.
//!
//! Message assertions pin the UI language to English first, so they can compare
//! against the exact `locales/en.toml` text. They previously used disjunctions
//! such as `contains("current") || contains("no") || contains("version")`,
//! where the two-letter `contains("no")` matched "node", "not" and
//! "non-default" alike — the assertion passed on the wrong error and on other
//! tests' output too.

mod common;
use common::{combined_output, create_fake_version, run_isolated_en, stdout};

// --- `nvm run` ------------------------------------------------------------

#[test]
fn run_nonexistent_version_bails_not_installed() {
    let (out, _dir) = run_isolated_en(&["run", "v99.99.99"]);
    assert!(!out.status.success(), "run v99.99.99 should fail");
    let s = combined_output(&out);
    assert!(
        s.contains("Version v99.99.99 is not installed"),
        "expected not_installed naming the version, got: {s}"
    );
}

// --- `nvm exec` -----------------------------------------------------------

#[test]
fn exec_without_args_bails_specify_command() {
    // exec_version checks `args.is_empty()` before resolving the version, so
    // this reports the missing command rather than the uninstalled version.
    // That ordering is the point of the assertion: were the version resolved
    // first, the message would be "Version v99.99.99 is not installed".
    let (out, _dir) = run_isolated_en(&["exec", "v99.99.99"]);
    assert!(!out.status.success(), "exec with no args should fail");
    let s = combined_output(&out);
    assert!(
        s.contains("Specify a command to run"),
        "the empty-args check must run before version resolution, got: {s}"
    );
    assert!(
        !s.contains("is not installed"),
        "version resolution must not happen first, got: {s}"
    );
}

#[test]
fn exec_nonexistent_version_bails_not_installed() {
    let (out, _dir) = run_isolated_en(&["exec", "v99.99.99", "echo", "hi"]);
    assert!(!out.status.success(), "exec v99.99.99 should fail");
    let s = combined_output(&out);
    // `not_installed_run_install`, which additionally tells the user the exact
    // command to run. Plain `not_installed` (used by `run`/`which`) does not.
    assert!(
        s.contains("Version v99.99.99 is not installed. Run 'nvm install v99.99.99' first."),
        "expected the not_installed_run_install hint, got: {s}"
    );
}

// --- `nvm which` ----------------------------------------------------------

#[test]
fn which_no_version_no_current_bails() {
    // No version arg and no current version → bail `no_current_version_set`.
    let (out, _dir) = run_isolated_en(&["which"]);
    assert!(!out.status.success(), "which with no current should fail");
    let s = combined_output(&out);
    assert!(
        s.contains("No current version set"),
        "expected no_current_version_set, got: {s}"
    );
}

#[test]
fn which_nonexistent_version_bails_not_installed() {
    let (out, _dir) = run_isolated_en(&["which", "v99.99.99"]);
    assert!(!out.status.success(), "which v99.99.99 should fail");
    let s = combined_output(&out);
    assert!(
        s.contains("Version v99.99.99 is not installed"),
        "expected not_installed naming the version, got: {s}"
    );
}

#[test]
fn which_succeeds_when_version_installed() {
    // Create a fake v20.0.0 with a node binary; `which v20.0.0` should
    // print its bin/node path and exit 0.
    let (mut cmd, nvm_dir, _home) = common::isolated_command(&["which", "v20.0.0"]);
    create_fake_version(nvm_dir.path(), "v20.0.0", true);

    let out = cmd.output().expect("run nvm which");
    assert!(
        out.status.success(),
        "which v20.0.0 should succeed: {}",
        stdout(&out)
    );
    let s = combined_output(&out);
    assert!(
        s.contains("v20.0.0") && s.contains("node"),
        "expected node path under v20.0.0, got: {s}"
    );
}

// --- `nvm auto --silent` --------------------------------------------------

#[test]
fn auto_silent_no_nvmrc_bails() {
    // `auto` forwards to `use_version_silent(None, ...)`; with no .nvmrc /
    // .node-version / package.json it bails `specify_version`.
    let (out, _dir) = run_isolated_en(&["auto", "--silent"]);
    assert!(
        !out.status.success(),
        "auto --silent with no .nvmrc should fail"
    );
    let s = combined_output(&out);
    // All three project files are named, so a future detection source added
    // without updating the message shows up here.
    assert!(
        s.contains(".nvmrc") && s.contains(".node-version") && s.contains("package.json"),
        "the hint should name every file auto looks for, got: {s}"
    );
}
