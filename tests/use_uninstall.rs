//! Integration tests for `nvm use` and `nvm uninstall` error paths and
//! flag dispatch.
//!
//! All offline: we exercise the failure paths (version not installed,
//! conflicting flags, empty install state) rather than real installs.

mod common;
use common::{combined_output, create_fake_version, run_isolated, stdout};

// --- `nvm use` error paths -------------------------------------------------

#[test]
fn use_no_arg_no_nvmrc_bails_with_specify_version() {
    // No version arg and no .nvmrc / .node-version / package.json in cwd
    // should bail with `specify_version`.
    let (out, _dir) = run_isolated(&["use"]);
    assert!(!out.status.success(), "use (no arg) should fail");
    let s = combined_output(&out);
    assert!(
        s.to_lowercase().contains("specify") || s.to_lowercase().contains("version"),
        "expected a 'specify version' hint, got: {s}"
    );
}

#[test]
fn use_nonexistent_version_bails_not_installed() {
    let (out, _dir) = run_isolated(&["use", "v99.99.99"]);
    assert!(!out.status.success(), "use v99.99.99 should fail");
    let s = combined_output(&out);
    assert!(
        s.to_lowercase().contains("not installed") || s.to_lowercase().contains("99.99.99"),
        "expected 'not installed' for v99.99.99, got: {s}"
    );
}

#[test]
fn use_succeeds_when_version_dir_exists() {
    // Create a fake v20.0.0 with a node binary so `use` switches to it.
    let (mut cmd, nvm_dir, _home) = common::isolated_command(&["use", "v20.0.0"]);
    create_fake_version(nvm_dir.path(), "v20.0.0", true);

    let out = cmd.output().expect("run nvm use");
    assert!(
        out.status.success(),
        "use v20.0.0 should succeed: {}",
        stdout(&out)
    );
}

#[test]
fn use_no_arg_falls_back_to_default_when_nvmrc_absent() {
    // P1-9: `nvm use` with no arg and no .nvmrc should switch to the
    // `default` version (config.default_version) instead of bailing. We seed
    // a fake v18.0.0 install + a config.json defaulting to it, then `use`.
    let (mut cmd, nvm_dir, _home) = common::isolated_command(&["use"]);
    create_fake_version(nvm_dir.path(), "v18.0.0", true);
    std::fs::write(
        nvm_dir.path().join("config.json"),
        r#"{"default_version":"v18.0.0"}"#,
    )
    .expect("write config.json");

    let out = cmd.output().expect("run nvm use");
    assert!(
        out.status.success(),
        "use (no arg) with default set should succeed: {}",
        combined_output(&out)
    );
    let s = combined_output(&out);
    assert!(
        s.contains("v18.0.0"),
        "expected default v18.0.0 to be used, got: {s}"
    );
}

// --- `nvm uninstall` error paths ------------------------------------------

#[test]
fn uninstall_nonexistent_version_bails_not_installed() {
    let (out, _dir) = run_isolated(&["uninstall", "v99.99.99"]);
    assert!(!out.status.success(), "uninstall v99.99.99 should fail");
    let s = combined_output(&out);
    assert!(
        s.to_lowercase().contains("not installed") || s.to_lowercase().contains("99.99.99"),
        "expected 'not installed' for uninstall v99.99.99, got: {s}"
    );
}

// --- `nvm uninstall --lts` / `--latest` empty state -----------------------

#[test]
fn uninstall_lts_empty_install_bails() {
    let (out, _dir) = run_isolated(&["uninstall", "--lts"]);
    assert!(
        !out.status.success(),
        "uninstall --lts on empty should fail"
    );
}

#[test]
fn uninstall_latest_empty_install_bails() {
    let (out, _dir) = run_isolated(&["uninstall", "--latest"]);
    assert!(
        !out.status.success(),
        "uninstall --latest on empty should fail"
    );
}

// --- `nvm uninstall` conflicting flag dispatch (main.rs match arm) --------
//
// main.rs:45-52 matches (version, lts, latest): only (Some, false, false),
// (None, true, false), (None, false, true) are valid. Any other combination
// (e.g. both --lts and --latest, or a version plus a flag) should bail with
// `specify_version_or_lts`.

#[test]
fn uninstall_both_lts_and_latest_bails() {
    let (out, _dir) = run_isolated(&["uninstall", "--lts", "--latest"]);
    assert!(!out.status.success(), "--lts --latest should fail");
}

#[test]
fn uninstall_version_and_lts_bails() {
    let (out, _dir) = run_isolated(&["uninstall", "v1.0.0", "--lts"]);
    assert!(!out.status.success(), "<ver> --lts should fail");
}

#[test]
fn uninstall_version_and_latest_bails() {
    let (out, _dir) = run_isolated(&["uninstall", "v1.0.0", "--latest"]);
    assert!(!out.status.success(), "<ver> --latest should fail");
}

// --- `nvm deactivate` empty state (no-op, should succeed) -----------------

#[test]
fn deactivate_with_no_current_succeeds() {
    // deactivate guards on `current` file existence, so it's a safe no-op
    // when nothing is active.
    let (out, _dir) = run_isolated(&["deactivate"]);
    assert!(
        out.status.success(),
        "deactivate with no current should succeed"
    );
}

// --- `nvm use --use-on-cd` and the `nvm auto --silent` cd-hook path -------
//
// `--use-on-cd` is on `nvm use`; `--silent` is on `nvm auto`. The real-world
// flow is: the user runs `nvm use <ver> --use-on-cd` once to install the
// shell cd hook, and thereafter every `cd` into a directory with a .nvmrc
// fires `nvm auto --silent` to switch versions without flooding the
// terminal. These tests pin both halves of that contract:
//   1. `--use-on-cd` persists `use_on_cd: true` in config.json AND writes
//      the `current` file (the version switch still happens).
//   2. `nvm auto --silent` switches the version (writes `current`) while
//      producing NO stdout — the cd hook must be invisible to the user.
//   3. Combined: after `--use-on-cd` is enabled, a subsequent
//      `nvm auto --silent` against a different .nvmrc switches silently.

#[test]
fn use_with_use_on_cd_persists_config_flag_and_writes_current() {
    // `nvm use v20.0.0 --use-on-cd` must:
    //   - succeed (the version is fake-installed)
    //   - write `current` with "v20.0.0" (the actual version switch)
    //   - persist `"use_on_cd":true` in config.json (the flag's whole job)
    //   - print SOMETHING (silent=false on `nvm use`, so the success + cd-hook
    //     notice must appear — guards against a regression that silently
    //     flipped the silent flag on the `use` path).
    //
    // HOME isolation is mandatory here: `nvm use` (non-silent) calls
    // `update_shell_config`, which probes HOME for `.bashrc`/`.zshrc` and
    // rewrites the first one it finds. Without isolating HOME the test
    // would clobber the developer's real shell rc.
    let (mut cmd, nvm_dir, _home) = common::isolated_command(&["use", "v20.0.0", "--use-on-cd"]);
    create_fake_version(nvm_dir.path(), "v20.0.0", true);

    let out = cmd.output().expect("run nvm use --use-on-cd");
    assert!(
        out.status.success(),
        "use v20.0.0 --use-on-cd should succeed: {}",
        combined_output(&out)
    );

    // `current` must reflect the switched version.
    let current = std::fs::read_to_string(nvm_dir.path().join("current"))
        .expect("current file should exist after use");
    assert_eq!(
        current.trim(),
        "v20.0.0",
        "current file must contain v20.0.0 after use --use-on-cd"
    );

    // config.json must persist `use_on_cd: true` — this is the flag's
    // entire purpose, and the cd hook installer checks this to decide
    // whether to fire `nvm auto` on cd.
    let config = std::fs::read_to_string(nvm_dir.path().join("config.json"))
        .expect("config.json should exist after use --use-on-cd");
    assert!(
        config.contains("\"use_on_cd\":true") || config.contains("\"use_on_cd\": true"),
        "config.json must persist use_on_cd:true, got: {config}"
    );

    // `nvm use` is non-silent, so the success path MUST print something
    // (the "now using" line and the "use_on_cd_enabled" notice). An empty
    // stdout here would mean the silent flag was wrongly applied to the
    // `use` path, breaking user feedback.
    let out_str = stdout(&out);
    assert!(
        !out_str.trim().is_empty(),
        "non-silent `nvm use --use-on-cd` must produce stdout, got empty output"
    );
}

#[test]
fn auto_silent_switches_version_and_produces_no_stdout() {
    // `nvm auto --silent` is the cd-hook invocation. It must:
    //   - switch to the version named in .nvmrc (write `current`)
    //   - produce NO stdout (the whole point of --silent is that `cd` does
    //     not flood the terminal)
    //   - NOT rewrite the shell rc (silent=true skips update_shell_config,
    //     avoiding a backup+read+filter+write of the rc on every cd)
    //
    // HOME is isolated defensively even though silent=true skips the shell
    // rc rewrite — `nvm auto` resolves the version via `get_home_dir()` in
    // some paths, and we don't want a future code change to silently start
    // touching the real HOME.
    let (mut cmd, nvm_dir, _home) = common::isolated_command(&["auto", "--silent"]);
    create_fake_version(nvm_dir.path(), "v20.0.0", true);
    // .nvmrc lives in CWD; `nvm auto` reads it via find_nvmrc_recursive.
    std::fs::write(nvm_dir.path().join(".nvmrc"), "v20.0.0\n").expect("write .nvmrc");

    let out = cmd
        .current_dir(nvm_dir.path())
        .output()
        .expect("run nvm auto --silent");
    assert!(
        out.status.success(),
        "nvm auto --silent should succeed: {}",
        combined_output(&out)
    );

    // The version switch must still happen despite silence.
    let current = std::fs::read_to_string(nvm_dir.path().join("current"))
        .expect("current file should exist after auto --silent");
    assert_eq!(
        current.trim(),
        "v20.0.0",
        "current file must contain v20.0.0 after auto --silent"
    );

    // The cd hook must be invisible: stdout must be empty. (stderr may
    // still carry warnings on real errors, but the happy path is silent.)
    let out_str = stdout(&out);
    assert!(
        out_str.trim().is_empty(),
        "nvm auto --silent must produce no stdout, got: {out_str:?}"
    );
}

#[test]
fn use_on_cd_then_auto_silent_switches_silently() {
    // Full cd-hook lifecycle:
    //   1. `nvm use v20.0.0 --use-on-cd` enables the hook + sets current=v20.0.0
    //   2. A subdirectory has a .nvmrc pinning v18.0.0
    //   3. `nvm auto --silent` (simulating the cd hook firing) must switch
    //      `current` to v18.0.0 with no stdout.
    //
    // This locks the end-to-end contract: enabling --use-on-cd does not
    // interfere with a later silent switch to a DIFFERENT version. A
    // regression that, e.g., cached the first version or skipped the
    // `current` write on silent would be caught here.
    //
    // HOME is isolated because step 1 (`nvm use --use-on-cd`, non-silent)
    // rewrites the shell rc — without isolation it would clobber the real
    // `~/.bashrc`.
    let (dir, nvm_dir) = common::isolated_nvm_dir();
    let home = tempfile::tempdir().expect("tempdir for HOME");
    create_fake_version(dir.path(), "v20.0.0", true);
    create_fake_version(dir.path(), "v18.0.0", true);

    // Step 1: enable the cd hook and switch to v20.0.0.
    let out = std::process::Command::new(common::nvm_bin())
        .arg("use")
        .arg("v20.0.0")
        .arg("--use-on-cd")
        .env("NVM_DIR", &nvm_dir)
        .env("HOME", home.path())
        .output()
        .expect("run nvm use --use-on-cd");
    assert!(
        out.status.success(),
        "use v20.0.0 --use-on-cd should succeed: {}",
        combined_output(&out)
    );

    // Step 2: a subdirectory pins v18.0.0 via .nvmrc.
    let subdir = dir.path().join("project");
    std::fs::create_dir_all(&subdir).expect("mkdir project");
    std::fs::write(subdir.join(".nvmrc"), "v18.0.0\n").expect("write .nvmrc");

    // Step 3: simulate the cd hook firing silently.
    let out = std::process::Command::new(common::nvm_bin())
        .arg("auto")
        .arg("--silent")
        .env("NVM_DIR", &nvm_dir)
        .env("HOME", home.path())
        .current_dir(&subdir)
        .output()
        .expect("run nvm auto --silent");
    assert!(
        out.status.success(),
        "nvm auto --silent should succeed: {}",
        combined_output(&out)
    );

    // The silent hook must have switched `current` to v18.0.0.
    let current =
        std::fs::read_to_string(dir.path().join("current")).expect("current file should exist");
    assert_eq!(
        current.trim(),
        "v18.0.0",
        "current must be v18.0.0 after silent cd-hook switch, got: {current}"
    );

    // And the switch must have been silent.
    let out_str = stdout(&out);
    assert!(
        out_str.trim().is_empty(),
        "cd-hook `nvm auto --silent` must produce no stdout, got: {out_str:?}"
    );
}

// --- `nvm uninstall --all` / `--self` ---------------------------------------

#[test]
fn uninstall_no_args_shows_hint() {
    let (out, _dir) = run_isolated(&["uninstall"]);
    assert!(
        !out.status.success(),
        "bare uninstall should fail with hint"
    );
    let s = combined_output(&out);
    assert!(
        s.contains("--all") || s.contains("--self") || s.contains("版本"),
        "should mention --all/--self in hint: {s}"
    );
}

#[test]
fn uninstall_all_cancelled_by_default() {
    // No 'y' on stdin → should cancel
    let (mut cmd, nvm_dir, _home) = common::isolated_command(&["uninstall", "--all"]);
    create_fake_version(nvm_dir.path(), "v20.0.0", true);

    let out = cmd
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn")
        .wait_with_output()
        .expect("wait");

    assert!(out.status.success(), "cancelled uninstall should exit 0");
    assert!(
        nvm_dir.path().exists(),
        "nvm dir should still exist after cancellation"
    );
    let s = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        s.contains("Cancel") || s.contains("取消"),
        "should print cancelled message: {s}"
    );
}

#[test]
fn uninstall_all_with_y_removes_everything() {
    let (mut cmd, nvm_dir, _home) = common::isolated_command(&["uninstall", "--all"]);
    create_fake_version(nvm_dir.path(), "v20.0.0", true);
    create_fake_version(nvm_dir.path(), "v22.0.0", true);

    let mut child = cmd
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn");

    use std::io::Write;
    if let Some(stdin) = child.stdin.as_mut() {
        let _ = stdin.write_all(b"y\n");
    }
    let out = child.wait_with_output().expect("wait");

    assert!(
        out.status.success(),
        "uninstall --all with y should succeed"
    );
    assert!(
        !nvm_dir.path().exists(),
        "nvm dir should be removed after --all"
    );
}

#[test]
fn uninstall_self_preserves_node_versions() {
    let (mut cmd, nvm_dir, _home) = common::isolated_command(&["uninstall", "--self"]);
    create_fake_version(nvm_dir.path(), "v20.0.0", true);

    let mut child = cmd
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn");

    use std::io::Write;
    if let Some(stdin) = child.stdin.as_mut() {
        let _ = stdin.write_all(b"y\n");
    }
    let out = child.wait_with_output().expect("wait");

    assert!(
        out.status.success(),
        "uninstall --self with y should succeed"
    );
    assert!(
        nvm_dir.path().join("v20.0.0").exists(),
        "Node version should be preserved after --self"
    );
    assert!(
        !nvm_dir.path().join("shims").exists(),
        "shims should be removed after --self"
    );
}
