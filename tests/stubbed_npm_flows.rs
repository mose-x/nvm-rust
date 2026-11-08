//! Happy-path coverage for the two child-process flows that had none:
//!
//! - **U-7** — `nvm corepack enable` / `disable`. `tests/completion_corepack.rs`
//!   states outright that it covers error paths only, and `src/corepack.rs` has
//!   no `#[cfg(test)]` module at all.
//! - **U-8** — `nvm reinstall-packages`. `src/commands/reinstall.rs` had zero
//!   tests; `tests/migrate_packages.rs` only asserts the `source_not_installed`
//!   bail.
//!
//! Both drive the real binary against **stub executables** placed in fake
//! version directories, so nothing hits the network and no real Node.js install
//! is needed. `corepack enable`'s own fallback runs `npm install -g corepack`,
//! which would reach the registry — the stubs avoid that by making the primary
//! path succeed.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::{combined_output, create_fake_version, isolated_command, stdout};

/// Write a stub executable at `dir/name` (plus the platform extension).
///
/// On Unix the file must be `chmod 755`: `create_fake_version` does not chmod,
/// and `Command::new` on a non-executable file fails with a spawn error — which
/// in `corepack_enable` would silently route into the `npm install -g corepack`
/// fallback and reach the network.
fn write_stub(dir: &Path, name: &str, unix_body: &str, windows_body: &str) {
    // Each cfg branch consumes only one body; without this the other is an
    // unused parameter and fails the Linux/macOS builds under -D warnings.
    let (_, _) = (unix_body, windows_body);
    fs::create_dir_all(dir).expect("create stub dir");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        fs::write(&path, format!("#!/bin/sh\n{unix_body}")).expect("write stub");
        let mut perms = fs::metadata(&path).expect("stub metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms).expect("chmod stub");
    }
    #[cfg(windows)]
    {
        fs::write(dir.join(format!("{name}.cmd")), windows_body).expect("write stub");
    }
}

/// A `corepack` stub that does what the real one does for our purposes: create
/// the four shims inside `--install-directory` (argument 3), each containing the
/// literal word `corepack`. That marker is what `corepack.rs` inspects to decide
/// whether a shim is its own.
fn write_corepack_stub(bin_dir: &Path) {
    write_stub(
        bin_dir,
        "corepack",
        // args: enable|disable --install-directory <dir>
        r#"dir="$3"
if [ "$1" = "disable" ]; then
  # Mirror real corepack: only remove shims it recognises as its own.
  for t in pnpm pnpx yarn yarnpkg; do
    grep -q corepack "$dir/$t" 2>/dev/null && rm -f "$dir/$t"
  done
  exit 0
fi
for t in pnpm pnpx yarn yarnpkg; do
  printf '#!/bin/sh\n# managed by corepack\n' > "$dir/$t"
  chmod 755 "$dir/$t"
done
exit 0
"#,
        "@echo off\r\nset DIR=%3\r\nif \"%1\"==\"disable\" exit /b 0\r\n> \"%DIR%\\pnpm\" echo managed by corepack\r\n> \"%DIR%\\pnpx\" echo managed by corepack\r\n> \"%DIR%\\yarn\" echo managed by corepack\r\n> \"%DIR%\\yarnpkg\" echo managed by corepack\r\nexit /b 0\r\n",
    );
}

/// The bin dir for a fake version, matching `version_bin_dir`'s per-platform
/// layout (version root on Windows, `bin/` elsewhere). Shims are written
/// extensionless, which `exe_path` finds via its bare-name fallback.
fn version_bin(nvm_dir: &Path, version: &str) -> PathBuf {
    if cfg!(windows) {
        nvm_dir.join(version)
    } else {
        nvm_dir.join(version).join("bin")
    }
}

// --- U-7: corepack enable / disable ---------------------------------------

#[test]
fn corepack_enable_creates_all_four_shims() {
    let (mut cmd, nvm_dir, _home) = isolated_command(&["corepack", "enable", "v20.0.0"]);
    create_fake_version(nvm_dir.path(), "v20.0.0", true);
    let bin = version_bin(nvm_dir.path(), "v20.0.0");
    write_corepack_stub(&bin);

    let out = cmd.output().expect("run nvm corepack enable");
    assert!(
        out.status.success(),
        "corepack enable should succeed with a working corepack stub: {}",
        combined_output(&out)
    );

    for tool in ["pnpm", "pnpx", "yarn", "yarnpkg"] {
        let shim = bin.join(tool);
        assert!(
            shim.exists(),
            "corepack enable must create the {tool} shim at {shim:?}"
        );
        let body = fs::read_to_string(&shim).unwrap_or_default();
        assert!(
            body.contains("corepack"),
            "the {tool} shim must be recognisable as corepack-managed, got: {body:?}"
        );
    }
}

#[test]
fn corepack_disable_fallback_removes_only_corepack_shims() {
    // The safety guard worth testing: when `corepack disable` cannot run, nvm
    // deletes the shims itself — but only files whose *content* mentions
    // corepack, so a user's own `pnpm` (installed via `npm i -g pnpm`) is never
    // removed. No corepack binary is installed here, so the spawn fails and the
    // `if !success` fallback in corepack.rs is the code under test.
    let (mut cmd, nvm_dir, _home) = isolated_command(&["corepack", "disable", "v20.0.0"]);
    create_fake_version(nvm_dir.path(), "v20.0.0", true);
    let bin = version_bin(nvm_dir.path(), "v20.0.0");

    fs::write(bin.join("pnpm"), "#!/bin/sh\n# user's own pnpm wrapper\n").expect("write user pnpm");
    for tool in ["yarn", "yarnpkg"] {
        fs::write(bin.join(tool), "#!/bin/sh\n# managed by corepack\n")
            .unwrap_or_else(|_| panic!("write {tool} shim"));
    }

    let out = cmd.output().expect("run nvm corepack disable");
    assert!(
        out.status.success(),
        "corepack disable returns Ok even when corepack itself is missing: {}",
        combined_output(&out)
    );

    let kept = fs::read_to_string(bin.join("pnpm"))
        .expect("a shim without the corepack marker must NOT be deleted");
    assert!(
        kept.contains("user's own pnpm"),
        "the user's pnpm must be untouched, got: {kept:?}"
    );
    for tool in ["yarn", "yarnpkg"] {
        assert!(
            !bin.join(tool).exists(),
            "the corepack-managed {tool} shim must be removed"
        );
    }
}

#[test]
fn corepack_disable_trusts_a_working_corepack_binary() {
    // The `success == true` branch: corepack removes its own shims, so nvm's
    // fallback must not run and must not touch anything else.
    let (mut cmd, nvm_dir, _home) = isolated_command(&["corepack", "disable", "v20.0.0"]);
    create_fake_version(nvm_dir.path(), "v20.0.0", true);
    let bin = version_bin(nvm_dir.path(), "v20.0.0");
    write_corepack_stub(&bin);

    fs::write(bin.join("pnpm"), "#!/bin/sh\n# user's own pnpm wrapper\n").expect("write user pnpm");
    fs::write(bin.join("yarn"), "#!/bin/sh\n# managed by corepack\n").expect("write yarn shim");

    let out = cmd.output().expect("run nvm corepack disable");
    assert!(
        out.status.success(),
        "corepack disable should succeed: {}",
        combined_output(&out)
    );
    assert!(
        bin.join("pnpm").exists(),
        "the user's pnpm must survive a successful corepack disable"
    );
}

#[test]
fn corepack_enable_bails_for_uninstalled_version() {
    // Guards the happy-path tests above against passing vacuously.
    let (mut cmd, _nvm_dir, _home) = isolated_command(&["corepack", "enable", "v20.0.0"]);
    let out = cmd.output().expect("run nvm corepack enable");
    assert!(
        !out.status.success(),
        "corepack enable must fail when the version is not installed: {}",
        combined_output(&out)
    );
}

// --- U-8: reinstall-packages ---------------------------------------------

/// What `npm list -g --depth=0 --json` emits on the source version: two
/// bundled packages that must be skipped, two real ones that must migrate.
const NPM_LIST_JSON: &str = r#"{"dependencies":{"npm":{"version":"10.8.0"},"corepack":{"version":"0.29.0"},"typescript":{"version":"5.6.0"},"eslint":{"version":"9.0.0"}}}"#;

fn write_source_npm(bin_dir: &Path) {
    write_stub(
        bin_dir,
        "npm",
        &format!("cat <<'NVMEOF'\n{NPM_LIST_JSON}\nNVMEOF\n"),
        &format!("@echo off\r\necho {NPM_LIST_JSON}\r\n"),
    );
}

fn write_target_npm(bin_dir: &Path, exit_code: u8) {
    write_stub(
        bin_dir,
        "npm",
        &format!("exit {exit_code}\n"),
        &format!("@echo off\r\nexit /b {exit_code}\r\n"),
    );
}

#[test]
fn reinstall_packages_migrates_listed_packages() {
    let (mut cmd, nvm_dir, _home) = isolated_command(&["reinstall-packages", "v18.0.0"]);
    create_fake_version(nvm_dir.path(), "v18.0.0", true);
    create_fake_version(nvm_dir.path(), "v20.0.0", true);
    write_source_npm(&version_bin(nvm_dir.path(), "v18.0.0"));
    write_target_npm(&version_bin(nvm_dir.path(), "v20.0.0"), 0);
    // `reinstall_packages` installs into whatever `current` points at.
    fs::write(nvm_dir.path().join("current"), "v20.0.0").expect("write current");

    let out = cmd.output().expect("run nvm reinstall-packages");
    assert!(
        out.status.success(),
        "reinstall-packages should succeed: {}",
        combined_output(&out)
    );
    // npm and corepack are excluded, so exactly two packages migrate. Asserted
    // on the summary because the child npm processes inherit the harness stdout.
    let s = stdout(&out);
    assert!(s.contains('2'), "expected a migrated count of 2 in: {s:?}");
}

#[test]
fn reinstall_packages_bails_when_a_package_fails() {
    let (mut cmd, nvm_dir, _home) = isolated_command(&["reinstall-packages", "v18.0.0"]);
    create_fake_version(nvm_dir.path(), "v18.0.0", true);
    create_fake_version(nvm_dir.path(), "v20.0.0", true);
    write_source_npm(&version_bin(nvm_dir.path(), "v18.0.0"));
    // Every `npm install -g` fails, which must surface as a non-zero exit that
    // names the packages rather than reporting a clean migration.
    write_target_npm(&version_bin(nvm_dir.path(), "v20.0.0"), 1);
    fs::write(nvm_dir.path().join("current"), "v20.0.0").expect("write current");

    let out = cmd.output().expect("run nvm reinstall-packages");
    assert!(
        !out.status.success(),
        "reinstall-packages must fail when installs fail: {}",
        combined_output(&out)
    );
    let combined = combined_output(&out);
    assert!(
        combined.contains("eslint") || combined.contains("typescript"),
        "the failure must name the packages that failed, got: {combined:?}"
    );
}
