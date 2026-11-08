//! Integration coverage for two audit gaps, both entirely local:
//!
//! - **U-10** — `nvm auto` reading `.node-version`. Every pre-existing test
//!   only wrote `.nvmrc`, so the second project-file format and the
//!   precedence between the two were untested.
//! - **U-12** — `nvm use --save`. The flag had no test at all; nothing pinned
//!   what it persists or where.
//!
//! Assertions are on exit status and on-disk state (`current`, `config.json`,
//! `alias.json`), never on message text: the language under test depends on the
//! runner's environment, so text assertions would be locale-fragile.

mod common;

use std::path::Path;
use std::process::Command;

use common::{combined_output, create_fake_version, isolated_command, nvm_bin, stdout};

/// A second `nvm` invocation reusing the isolation dirs from
/// [`isolated_command`], so a test can run one command and then assert on what
/// a *later* command sees.
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

// --- U-10: `.node-version` -------------------------------------------------

#[test]
fn auto_silent_reads_node_version_file() {
    let (mut cmd, nvm_dir, _home) = isolated_command(&["auto", "--silent"]);
    create_fake_version(nvm_dir.path(), "v20.0.0", true);
    std::fs::write(nvm_dir.path().join(".node-version"), "v20.0.0\n").expect("write .node-version");

    let out = cmd
        .current_dir(nvm_dir.path())
        .output()
        .expect("run nvm auto --silent");

    assert!(
        out.status.success(),
        "auto should resolve .node-version: {}",
        combined_output(&out)
    );
    assert_eq!(read_current(nvm_dir.path()), "v20.0.0");
    assert!(
        stdout(&out).is_empty(),
        "--silent must produce no stdout, got: {:?}",
        stdout(&out)
    );
}

#[test]
fn auto_prefers_nvmrc_over_node_version_in_the_same_dir() {
    // Both files are checked at the *same* directory level, `.nvmrc` first, so
    // a directory holding both must resolve to `.nvmrc`. Pinning this stops a
    // reorder from silently changing which file wins.
    let (mut cmd, nvm_dir, _home) = isolated_command(&["auto", "--silent"]);
    create_fake_version(nvm_dir.path(), "v20.0.0", true);
    create_fake_version(nvm_dir.path(), "v18.0.0", true);
    std::fs::write(nvm_dir.path().join(".nvmrc"), "v20.0.0\n").expect("write .nvmrc");
    std::fs::write(nvm_dir.path().join(".node-version"), "v18.0.0\n").expect("write .node-version");

    let out = cmd
        .current_dir(nvm_dir.path())
        .output()
        .expect("run nvm auto --silent");

    assert!(out.status.success(), "{}", combined_output(&out));
    assert_eq!(
        read_current(nvm_dir.path()),
        "v20.0.0",
        ".nvmrc must win over .node-version in the same directory"
    );
}

#[test]
fn auto_prefers_nearer_node_version_over_farther_nvmrc() {
    // The upward walk checks both formats at each level before moving up, so a
    // `.node-version` in the project directory beats an `.nvmrc` in a parent.
    // A per-file-type precedence (all `.nvmrc` levels first) would get this
    // backwards.
    let (mut cmd, nvm_dir, _home) = isolated_command(&["auto", "--silent"]);
    create_fake_version(nvm_dir.path(), "v20.0.0", true);
    create_fake_version(nvm_dir.path(), "v18.0.0", true);

    let project = nvm_dir.path().join("packages").join("app");
    std::fs::create_dir_all(&project).expect("create nested project dir");
    std::fs::write(nvm_dir.path().join(".nvmrc"), "v20.0.0\n").expect("write parent .nvmrc");
    std::fs::write(project.join(".node-version"), "v18.0.0\n").expect("write .node-version");

    let out = cmd
        .current_dir(&project)
        .output()
        .expect("run nvm auto --silent");

    assert!(out.status.success(), "{}", combined_output(&out));
    assert_eq!(
        read_current(nvm_dir.path()),
        "v18.0.0",
        "the nearer .node-version must beat the farther .nvmrc"
    );
}

#[test]
fn node_version_skips_comments_and_blank_lines() {
    // Same parsing rule as .nvmrc: first line that is non-blank after trim and
    // does not start with '#'.
    let (mut cmd, nvm_dir, _home) = isolated_command(&["auto", "--silent"]);
    create_fake_version(nvm_dir.path(), "v20.0.0", true);
    std::fs::write(
        nvm_dir.path().join(".node-version"),
        "# pinned by CI\n\n   \nv20.0.0\n",
    )
    .expect("write .node-version");

    let out = cmd
        .current_dir(nvm_dir.path())
        .output()
        .expect("run nvm auto --silent");

    assert!(out.status.success(), "{}", combined_output(&out));
    assert_eq!(read_current(nvm_dir.path()), "v20.0.0");
}

// --- U-12: `nvm use --save` ------------------------------------------------

#[test]
fn use_save_persists_default_version_to_config() {
    let (mut cmd, nvm_dir, _home) = isolated_command(&["use", "v20.0.0", "--save"]);
    create_fake_version(nvm_dir.path(), "v20.0.0", true);

    let out = cmd
        .current_dir(nvm_dir.path())
        .output()
        .expect("run nvm use --save");

    assert!(
        out.status.success(),
        "use --save should succeed: {}",
        combined_output(&out)
    );
    assert_eq!(read_current(nvm_dir.path()), "v20.0.0");

    let config: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(nvm_dir.path().join("config.json"))
            .expect("--save must write config.json"),
    )
    .expect("config.json must be valid JSON");
    assert_eq!(
        config["default_version"].as_str(),
        Some("v20.0.0"),
        "--save must persist default_version, got: {config}"
    );
}

#[test]
fn use_save_does_not_write_alias_file() {
    // `--save` writes `config.default_version`; it does NOT create a `default`
    // alias. `resolve_alias("default")` consults alias.json first and falls
    // back to the config field, so the two are distinct mechanisms. Pinning
    // this keeps the split from being accidentally merged.
    let (mut cmd, nvm_dir, home) = isolated_command(&["use", "v20.0.0", "--save"]);
    create_fake_version(nvm_dir.path(), "v20.0.0", true);

    let out = cmd
        .current_dir(nvm_dir.path())
        .output()
        .expect("run nvm use --save");
    assert!(out.status.success(), "{}", combined_output(&out));

    let alias_path = nvm_dir.path().join("alias.json");
    if alias_path.exists() {
        let aliases: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&alias_path).expect("read alias.json"))
                .expect("alias.json must be valid JSON");
        assert!(
            aliases["aliases"]["default"].is_null(),
            "--save must not create a `default` alias, got: {aliases}"
        );
    }
    // HOME is isolated, so nothing outside the tempdirs was touched either.
    assert!(
        !home.path().join("alias.json").exists(),
        "--save must not write into HOME"
    );
}

#[test]
fn saved_default_is_used_by_a_later_bare_use() {
    // The point of --save: a subsequent `nvm use` with no argument and no
    // project file resolves to the saved version instead of bailing.
    let (mut cmd, nvm_dir, home) = isolated_command(&["use", "v20.0.0", "--save"]);
    create_fake_version(nvm_dir.path(), "v20.0.0", true);
    create_fake_version(nvm_dir.path(), "v18.0.0", true);

    let out = cmd
        .current_dir(nvm_dir.path())
        .output()
        .expect("run nvm use --save");
    assert!(out.status.success(), "{}", combined_output(&out));

    // Switch away, then let the saved default bring it back.
    let away = command_in(nvm_dir.path(), home.path(), &["use", "v18.0.0"])
        .output()
        .expect("run nvm use v18.0.0");
    assert!(away.status.success(), "{}", combined_output(&away));
    assert_eq!(read_current(nvm_dir.path()), "v18.0.0");

    let back = command_in(nvm_dir.path(), home.path(), &["use"])
        .output()
        .expect("run bare nvm use");
    assert!(
        back.status.success(),
        "bare `nvm use` should fall back to the saved default: {}",
        combined_output(&back)
    );
    assert_eq!(
        read_current(nvm_dir.path()),
        "v20.0.0",
        "bare `nvm use` must resolve to the --save'd default"
    );
}

#[test]
fn explicit_default_alias_beats_saved_config() {
    // `resolve_alias("default")` checks alias.json before config.default_version,
    // so an explicit `nvm alias default` must win over a later `--save`.
    let (mut cmd, nvm_dir, home) = isolated_command(&["alias", "default", "v18.0.0"]);
    create_fake_version(nvm_dir.path(), "v20.0.0", true);
    create_fake_version(nvm_dir.path(), "v18.0.0", true);

    let alias_out = cmd
        .current_dir(nvm_dir.path())
        .output()
        .expect("run nvm alias default");
    assert!(
        alias_out.status.success(),
        "nvm alias default should succeed: {}",
        combined_output(&alias_out)
    );

    let save = command_in(nvm_dir.path(), home.path(), &["use", "v20.0.0", "--save"])
        .output()
        .expect("run nvm use --save");
    assert!(save.status.success(), "{}", combined_output(&save));

    let bare = command_in(nvm_dir.path(), home.path(), &["use"])
        .output()
        .expect("run bare nvm use");
    assert!(bare.status.success(), "{}", combined_output(&bare));
    assert_eq!(
        read_current(nvm_dir.path()),
        "v18.0.0",
        "an explicit `default` alias must take precedence over config.default_version"
    );
}
