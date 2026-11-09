//! Integration coverage for **U-11**: `nvm auto` resolving a version from
//! `package.json`'s `engines.node`.
//!
//! `find_package_json_node_version` and `resolve_engines_node` have unit tests,
//! but nothing exercised the path a user actually hits: running the binary in a
//! project directory and having the detected constraint decide which version
//! becomes active. That leaves the wiring untested — the precedence against
//! `.nvmrc`/`.node-version`, the walk-up from a sub-directory, and whether a
//! resolved range survives the trip through `resolve_alias` into `current`.
//!
//! Everything here is local: versions are faked on disk, so no download, no
//! network, no registry.
//!
//! Assertions are on exit status and on-disk state (`current`), never on
//! message text — the runner's locale is not under our control.

mod common;

use std::path::Path;
use std::process::Command;

use common::{combined_output, create_fake_version, isolated_command, nvm_bin};

/// A further `nvm` invocation reusing the isolation dirs, so a test can assert
/// on what a later command sees.
fn command_in(nvm_dir: &Path, home: &Path, args: &[&str]) -> Command {
    let mut cmd = Command::new(nvm_bin());
    cmd.args(args)
        .env("NVM_DIR", nvm_dir)
        .env("HOME", home)
        .current_dir(nvm_dir);
    #[cfg(windows)]
    cmd.env("USERPROFILE", home);
    cmd
}

fn read_current(nvm_dir: &Path) -> String {
    std::fs::read_to_string(nvm_dir.join("current"))
        .expect("`current` should exist after a successful switch")
        .trim()
        .to_string()
}

fn write_package_json(dir: &Path, engines_node: Option<&str>) {
    std::fs::create_dir_all(dir).expect("create project dir");
    let body = match engines_node {
        Some(v) => {
            format!(r#"{{"name": "fixture", "version": "1.0.0", "engines": {{"node": "{v}"}}}}"#)
        }
        None => r#"{"name": "fixture", "version": "1.0.0"}"#.to_string(),
    };
    std::fs::write(dir.join("package.json"), body).expect("write package.json");
}

/// Run `nvm auto --silent` with `cwd` set to `project`, which may be a
/// sub-directory of the isolated `NVM_DIR`.
fn auto_in(cmd: &mut Command, project: &Path) -> std::process::Output {
    cmd.current_dir(project)
        .output()
        .expect("run nvm auto --silent")
}

#[test]
fn auto_resolves_a_bare_version_from_package_json() {
    let (mut cmd, nvm_dir, _home) = isolated_command(&["auto", "--silent"]);
    create_fake_version(nvm_dir.path(), "v20.0.0", true);
    write_package_json(nvm_dir.path(), Some("20.0.0"));

    let out = auto_in(&mut cmd, nvm_dir.path());

    assert!(
        out.status.success(),
        "auto should resolve engines.node: {}",
        combined_output(&out)
    );
    assert_eq!(read_current(nvm_dir.path()), "v20.0.0");
}

#[test]
fn auto_resolves_an_engines_range_to_an_installed_version() {
    let (mut cmd, nvm_dir, _home) = isolated_command(&["auto", "--silent"]);
    for v in ["v18.20.0", "v20.11.0", "v22.1.0"] {
        create_fake_version(nvm_dir.path(), v, true);
    }
    // `^20.0.0` is >=20.0.0 <21.0.0. The newest installed (v22.1.0) does not
    // satisfy it, so a pass here means the range was really evaluated rather
    // than "pick the newest" or "pass the range through".
    write_package_json(nvm_dir.path(), Some("^20.0.0"));

    let out = auto_in(&mut cmd, nvm_dir.path());

    assert!(
        out.status.success(),
        "auto should resolve the range: {}",
        combined_output(&out)
    );
    assert_eq!(read_current(nvm_dir.path()), "v20.11.0");
}

#[test]
fn auto_prefers_nvmrc_over_package_json() {
    let (mut cmd, nvm_dir, _home) = isolated_command(&["auto", "--silent"]);
    create_fake_version(nvm_dir.path(), "v18.0.0", true);
    create_fake_version(nvm_dir.path(), "v20.0.0", true);
    std::fs::write(nvm_dir.path().join(".nvmrc"), "v18.0.0\n").expect("write .nvmrc");
    write_package_json(nvm_dir.path(), Some("20.0.0"));

    let out = auto_in(&mut cmd, nvm_dir.path());

    assert!(out.status.success(), "{}", combined_output(&out));
    assert_eq!(
        read_current(nvm_dir.path()),
        "v18.0.0",
        ".nvmrc is nvm-native and deliberately outranks a project-wide engines.node"
    );
}

#[test]
fn auto_prefers_node_version_over_package_json() {
    let (mut cmd, nvm_dir, _home) = isolated_command(&["auto", "--silent"]);
    create_fake_version(nvm_dir.path(), "v18.0.0", true);
    create_fake_version(nvm_dir.path(), "v20.0.0", true);
    std::fs::write(nvm_dir.path().join(".node-version"), "v18.0.0\n").expect("write .node-version");
    write_package_json(nvm_dir.path(), Some("20.0.0"));

    let out = auto_in(&mut cmd, nvm_dir.path());

    assert!(out.status.success(), "{}", combined_output(&out));
    assert_eq!(read_current(nvm_dir.path()), "v18.0.0");
}

#[test]
fn auto_walks_up_to_a_parent_package_json() {
    let (mut cmd, nvm_dir, _home) = isolated_command(&["auto", "--silent"]);
    create_fake_version(nvm_dir.path(), "v20.0.0", true);
    write_package_json(nvm_dir.path(), Some("20.0.0"));
    // The monorepo layout: the command runs from a nested package that has no
    // version file of its own.
    let sub = nvm_dir.path().join("packages").join("app");
    std::fs::create_dir_all(&sub).expect("create sub package");

    let out = auto_in(&mut cmd, &sub);

    assert!(
        out.status.success(),
        "auto should find the root package.json from a sub-directory: {}",
        combined_output(&out)
    );
    assert_eq!(read_current(nvm_dir.path()), "v20.0.0");
}

#[test]
fn auto_uses_the_root_engines_when_a_sub_package_declares_none() {
    let (mut cmd, nvm_dir, _home) = isolated_command(&["auto", "--silent"]);
    create_fake_version(nvm_dir.path(), "v20.0.0", true);
    create_fake_version(nvm_dir.path(), "v22.0.0", true);
    write_package_json(nvm_dir.path(), Some("20.0.0"));
    let sub = nvm_dir.path().join("packages").join("app");
    // A package.json that exists but declares no engines.node must NOT stop the
    // walk — otherwise every sub-package in a monorepo would have to repeat the
    // root's constraint.
    write_package_json(&sub, None);

    let out = auto_in(&mut cmd, &sub);

    assert!(out.status.success(), "{}", combined_output(&out));
    assert_eq!(
        read_current(nvm_dir.path()),
        "v20.0.0",
        "the nearest package.json without engines.node must not shadow the root's"
    );
}

#[test]
fn auto_survives_a_malformed_package_json_and_uses_the_parent() {
    let (mut cmd, nvm_dir, _home) = isolated_command(&["auto", "--silent"]);
    create_fake_version(nvm_dir.path(), "v20.0.0", true);
    write_package_json(nvm_dir.path(), Some("20.0.0"));
    let sub = nvm_dir.path().join("packages").join("app");
    std::fs::create_dir_all(&sub).expect("create sub package");
    std::fs::write(sub.join("package.json"), "not valid json {{{").expect("write broken json");

    let out = auto_in(&mut cmd, &sub);

    assert!(
        out.status.success(),
        "a broken package.json must not abort detection: {}",
        combined_output(&out)
    );
    assert_eq!(read_current(nvm_dir.path()), "v20.0.0");
}

/// A `package.json` with no `engines.node` anywhere means there is no
/// project-level constraint, so `auto` must fall back to the saved default
/// rather than failing or picking an arbitrary installed version.
#[test]
fn auto_falls_back_to_the_default_alias_without_engines_node() {
    let (mut cmd, nvm_dir, home) = isolated_command(&["auto", "--silent"]);
    create_fake_version(nvm_dir.path(), "v18.0.0", true);
    create_fake_version(nvm_dir.path(), "v20.0.0", true);
    write_package_json(nvm_dir.path(), None);

    // Save a default the fallback can find.
    let save = command_in(nvm_dir.path(), home.path(), &["use", "v18.0.0", "--save"])
        .current_dir(nvm_dir.path())
        .output()
        .expect("run nvm use --save");
    assert!(
        save.status.success(),
        "seeding the default should succeed: {}",
        combined_output(&save)
    );

    let out = auto_in(&mut cmd, nvm_dir.path());

    assert!(out.status.success(), "{}", combined_output(&out));
    assert_eq!(
        read_current(nvm_dir.path()),
        "v18.0.0",
        "with no engines.node, auto must use the saved default"
    );
}
