//! Integration coverage for **U-5**: `nvm migrate nvm` end to end.
//!
//! `migrate.rs` has 19 unit tests, but every one of them targets
//! `detect_nvm_sh_default*` — the alias parser. `cmd_migrate` itself, the
//! function users actually run, had no coverage beyond two error paths in
//! `tests/migrate_packages.rs`. That left untested: which directories are
//! recognised as versions, the deep-copy semantics, the skip-if-present
//! behaviour, and the carry-over of the `default` alias into both
//! `config.json` and `alias.json`.
//!
//! Everything here is local. `resolve_migration_source` honours `NVM_SH_HOME`
//! specifically so a test can point at a fabricated nvm-sh tree, so no real
//! `~/.nvm` is touched and there is no network.
//!
//! Assertions are on exit status and on-disk state, never on message text: the
//! runner's locale is not under our control.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use common::{combined_output, nvm_bin};

/// A fabricated nvm-sh install, an isolated nvm-rust store, and a throwaway
/// HOME — all inside one temp root.
///
/// One root rather than three separate `TempDir`s because the path-traversal
/// test needs to plant a directory *outside* the store but still inside
/// something that gets cleaned up.
struct Fixture {
    root: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().expect("tempdir");
        for sub in ["store", "home", "sh"] {
            fs::create_dir_all(root.path().join(sub)).expect("create fixture subdir");
        }
        let p = Paths::new(root.path().to_path_buf());
        fs::create_dir_all(p.versions_dir()).expect("create versions dir");
        Self { root }
    }

    fn paths(&self) -> Paths {
        Paths::new(self.root.path().to_path_buf())
    }

    /// Run `nvm migrate nvm` against this fixture.
    fn migrate(&self) -> Output {
        let p = self.paths();
        let mut cmd = Command::new(nvm_bin());
        cmd.arg("migrate")
            .arg("nvm")
            .env("NVM_DIR", p.store())
            .env("HOME", p.home())
            .env("NVM_SH_HOME", p.sh_root())
            .current_dir(p.store());
        #[cfg(windows)]
        cmd.env("USERPROFILE", p.home());
        cmd.output().expect("run nvm migrate nvm")
    }
}

/// Path accessors derived from the fixture root.
struct Paths {
    root: PathBuf,
}

impl Paths {
    fn new(root: PathBuf) -> Self {
        Self { root }
    }
    fn store(&self) -> PathBuf {
        self.root.join("store")
    }
    fn home(&self) -> PathBuf {
        self.root.join("home")
    }
    /// The directory that stands in for the user's `$HOME` when nvm-sh looks
    /// for `~/.nvm`.
    fn sh_root(&self) -> PathBuf {
        self.root.join("sh")
    }
    fn versions_dir(&self) -> PathBuf {
        self.sh_root().join(".nvm").join("versions").join("node")
    }
    fn default_alias_file(&self) -> PathBuf {
        self.sh_root().join(".nvm").join("alias").join("default")
    }
    fn config(&self) -> PathBuf {
        self.store().join("config.json")
    }
    fn aliases(&self) -> PathBuf {
        self.store().join("alias.json")
    }

    /// Create a plausible nvm-sh version directory: a `bin/node` executable and
    /// an npm package under `lib/node_modules`.
    fn add_version(&self, version: &str, node_body: &[u8]) -> PathBuf {
        let dir = self.versions_dir().join(version);
        fs::create_dir_all(dir.join("bin")).expect("create bin");
        fs::create_dir_all(dir.join("lib/node_modules/npm/bin")).expect("create npm dir");
        fs::write(dir.join("bin/node"), node_body).expect("write node");
        fs::write(
            dir.join("lib/node_modules/npm/bin/npm-cli.js"),
            b"#!/usr/bin/env node\nconsole.log('npm');\n",
        )
        .expect("write npm-cli.js");
        dir
    }

    fn set_default_alias(&self, value: &str) {
        let f = self.default_alias_file();
        fs::create_dir_all(f.parent().expect("alias dir")).expect("create alias dir");
        fs::write(f, format!("{value}\n")).expect("write default alias");
    }

    fn read_json(&self, path: &Path) -> serde_json::Value {
        let raw = fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("{} must be readable: {e}", path.display()));
        serde_json::from_str(&raw)
            .unwrap_or_else(|e| panic!("{} must be valid JSON: {e}", path.display()))
    }

    fn aliases_json(&self) -> serde_json::Value {
        self.read_json(&self.aliases())
    }

    /// `config.default_version`, or `None` when it is unset *or* when
    /// `config.json` was never written. Several paths legitimately produce no
    /// file at all: an empty source tree returns before the alias carry-over,
    /// and a rejected traversal alias skips the save.
    fn default_version(&self) -> Option<String> {
        let raw = match fs::read_to_string(self.config()) {
            Ok(r) => r,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
            Err(e) => panic!("config.json must be readable: {e}"),
        };
        let json: serde_json::Value =
            serde_json::from_str(&raw).expect("config.json must be valid JSON");
        json.get("default_version")
            .and_then(|v| v.as_str())
            .map(String::from)
    }
}

fn assert_migrate_ok(out: &Output) {
    assert!(
        out.status.success(),
        "migrate should succeed: {}",
        combined_output(out)
    );
}

#[test]
fn migrate_imports_every_version_directory() {
    let fx = Fixture::new();
    let p = fx.paths();
    p.add_version("v18.20.0", b"node-18");
    p.add_version("v20.11.0", b"node-20");

    assert_migrate_ok(&fx.migrate());

    for (version, body) in [("v18.20.0", b"node-18"), ("v20.11.0", b"node-20")] {
        let node = p.store().join(version).join("bin/node");
        assert!(node.exists(), "{version} should have been imported");
        assert_eq!(
            fs::read(&node).expect("read imported node"),
            body,
            "{version} must be a deep copy with its contents intact"
        );
        // Nested files matter too: a shallow copy of just `bin/` would pass the
        // assertion above while leaving npm behind.
        assert!(
            p.store()
                .join(version)
                .join("lib/node_modules/npm/bin/npm-cli.js")
                .exists(),
            "{version}: nested lib/ tree must be copied"
        );
    }
}

#[test]
fn migrate_skips_a_version_already_in_the_store() {
    let fx = Fixture::new();
    let p = fx.paths();
    p.add_version("v18.20.0", b"node-18");
    p.add_version("v20.11.0", b"source-node-20");

    // Already present in the nvm-rust store, with contents that differ from the
    // source. `import_version` must short-circuit rather than clobber.
    let existing = p.store().join("v20.11.0/bin");
    fs::create_dir_all(&existing).expect("create existing version");
    fs::write(existing.join("node"), b"PRE-EXISTING").expect("write marker");

    assert_migrate_ok(&fx.migrate());

    assert_eq!(
        fs::read(p.store().join("v20.11.0/bin/node")).expect("read"),
        b"PRE-EXISTING",
        "an already-imported version must be left untouched, not overwritten"
    );
    assert!(
        p.store().join("v18.20.0/bin/node").exists(),
        "the other version must still be imported"
    );
}

#[test]
fn migrate_ignores_entries_that_are_not_versions() {
    let fx = Fixture::new();
    let p = fx.paths();
    p.add_version("v20.11.0", b"node-20");

    // nvm-sh keeps plenty of non-version entries alongside `versions/node`; the
    // filter is `starts_with('v')` or an io.js prefix.
    let versions = p.versions_dir();
    fs::create_dir_all(versions.join("alias")).expect("mkdir alias");
    fs::create_dir_all(versions.join("packages")).expect("mkdir packages");
    fs::create_dir_all(versions.join(".cache")).expect("mkdir .cache");
    fs::write(versions.join("README.md"), b"not a version").expect("write README");
    // Starts with 'v' but is not a version — the filter is deliberately lexical,
    // so this one IS picked up; pinning that keeps the behaviour honest rather
    // than pretending it is smarter than it is.
    fs::create_dir_all(versions.join("v8-flags")).expect("mkdir v8-flags");

    assert_migrate_ok(&fx.migrate());

    for skipped in ["alias", "packages", ".cache", "README.md"] {
        assert!(
            !p.store().join(skipped).exists(),
            "{skipped} is not a version directory and must not be imported"
        );
    }
    assert!(p.store().join("v20.11.0/bin/node").exists());
}

#[test]
fn migrate_imports_every_iojs_prefix_spelling() {
    let fx = Fixture::new();
    let p = fx.paths();
    // The old filter was `starts_with("iojs-")`, which silently skipped the
    // `io.js-*` spellings some nvm forks write.
    p.add_version("iojs-v3.3.1", b"iojs-3");
    p.add_version("io.js-v2.5.0", b"iojs-2");

    assert_migrate_ok(&fx.migrate());

    for version in ["iojs-v3.3.1", "io.js-v2.5.0"] {
        assert!(
            p.store().join(version).join("bin/node").exists(),
            "{version} must be recognised as an io.js version directory"
        );
    }
}

#[test]
fn migrate_carries_over_the_default_alias() {
    let fx = Fixture::new();
    let p = fx.paths();
    p.add_version("v18.20.0", b"node-18");
    p.add_version("v20.11.0", b"node-20");
    p.set_default_alias("v20.11.0");

    assert_migrate_ok(&fx.migrate());

    assert_eq!(
        p.default_version().as_deref(),
        Some("v20.11.0"),
        "the nvm-sh default must land in config.default_version"
    );
}

#[test]
fn migrate_default_overrides_a_pre_existing_default_alias() {
    let fx = Fixture::new();
    let p = fx.paths();
    p.add_version("v18.20.0", b"node-18");
    p.add_version("v20.11.0", b"node-20");
    p.set_default_alias("v20.11.0");

    // `resolve_alias("default")` consults user aliases FIRST and only falls
    // back to `config.default_version`. Writing the config alone would let a
    // pre-existing `default` alias shadow the migrated value, so `alias.json`
    // has to be updated too.
    fs::write(p.aliases(), r#"{"aliases":{"default":"v18.20.0"}}"#).expect("seed alias.json");

    assert_migrate_ok(&fx.migrate());

    assert_eq!(p.default_version().as_deref(), Some("v20.11.0"));
    assert_eq!(
        p.aliases_json()
            .get("aliases")
            .and_then(|a| a.get("default"))
            .and_then(|v| v.as_str()),
        Some("v20.11.0"),
        "a pre-existing `default` alias must be overwritten, not left to shadow"
    );
}

#[test]
fn migrate_leaves_the_default_alone_when_the_alias_is_absent() {
    let fx = Fixture::new();
    let p = fx.paths();
    p.add_version("v20.11.0", b"node-20");
    // No `.nvm/alias/default` at all.

    assert_migrate_ok(&fx.migrate());

    assert!(p.store().join("v20.11.0/bin/node").exists());
    assert_eq!(
        p.default_version(),
        None,
        "no source default means no default_version is invented"
    );
}

#[test]
fn migrate_rejects_a_path_traversal_default_alias() {
    let fx = Fixture::new();
    let p = fx.paths();
    p.add_version("v20.11.0", b"node-20");

    // A hand-edited or hostile `default` file. `cmd_migrate` joins this onto
    // NVM_DIR, so without `validate_version_name` it would escape the store.
    // The traversal target is planted and made to EXIST: `cmd_migrate` only
    // writes the config when `dest.exists()`, so without this the test would
    // pass even if the validation were deleted.
    p.set_default_alias("v20.11.0/../../evil");
    fs::create_dir_all(p.root.join("evil")).expect("plant traversal target");

    assert_migrate_ok(&fx.migrate());

    // The version itself still imports; only the alias is refused.
    assert!(p.store().join("v20.11.0/bin/node").exists());
    assert_ne!(
        p.default_version().as_deref(),
        Some("v20.11.0/../../evil"),
        "a traversal payload must never be written into config.default_version"
    );
    assert_eq!(
        p.default_version(),
        None,
        "a rejected default must leave default_version unset"
    );
}

#[test]
fn migrate_with_no_versions_succeeds_without_writing_a_default() {
    let fx = Fixture::new();
    let p = fx.paths();
    // `versions/node` exists but is empty.

    assert_migrate_ok(&fx.migrate());

    assert_eq!(
        p.default_version(),
        None,
        "nothing was imported, so no default should be carried over"
    );
}

/// The import must be self-contained: `import_version` deep-copies precisely so
/// that `rm -rf ~/.nvm` afterwards cannot leave the migrated versions dangling.
#[cfg(unix)]
#[test]
fn migrate_produces_a_tree_that_survives_deleting_the_source() {
    let fx = Fixture::new();
    let p = fx.paths();
    let src = p.add_version("v20.11.0", b"node-20");

    // nvm-sh lays `bin/npm` out as a relative symlink into `lib/node_modules`.
    // `copy_dir_recursive` recreates the link rather than flattening it, so the
    // interesting property is that the *target* was copied too.
    std::os::unix::fs::symlink(
        "../lib/node_modules/npm/bin/npm-cli.js",
        src.join("bin/npm"),
    )
    .expect("create npm symlink");

    assert_migrate_ok(&fx.migrate());

    let migrated = p.store().join("v20.11.0");
    let link = migrated.join("bin/npm");
    assert!(
        fs::symlink_metadata(&link)
            .expect("stat link")
            .file_type()
            .is_symlink(),
        "the relative symlink should be recreated as a symlink"
    );
    assert_eq!(
        fs::read_link(&link).expect("read link"),
        Path::new("../lib/node_modules/npm/bin/npm-cli.js"),
        "the link target must be preserved verbatim"
    );

    // The real assertion: remove the entire source tree, then the link must
    // still resolve to the npm entry point.
    fs::remove_dir_all(p.sh_root()).expect("delete the source nvm-sh tree");
    let resolved =
        fs::read(&link).expect("bin/npm must still resolve after the source tree is deleted");
    assert!(
        String::from_utf8_lossy(&resolved).contains("npm"),
        "the symlink target must have been copied too, got {} bytes",
        resolved.len()
    );
}
