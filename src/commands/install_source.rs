//! Source-build install path (`nvm install --source`).
//!
//! `install_from_source` compiles Node.js from a source tarball using the
//! Unix toolchain (`./configure`, `make`). It is unreachable on Windows
//! (rejected up-front in `build_install_target` with `source_unsupported_windows`).
//! io.js source compilation is also rejected upstream.
//!
//! `SourceGuard`, `command_failed`, and `InstallTarget` live in
//! [`super::install`] because they are shared with the binary-install path
//! (`install_binary`) and the orchestrator (`install`).

use anyhow::{Context, Result};
use colored::Colorize;
use std::fs;
use std::path::Path;
use std::process::Command;

use super::version_resolve::get_source_url;
use crate::download::{copy_from_cache, download_to_cache, is_cached};
use crate::i18n::{format_t, T};

use super::install::{command_failed, InstallTarget, SourceGuard};
use super::package_upgrade::download_prebuilt_npm;

/// Compile and install Node.js from source. Used when `--source` is passed.
/// io.js source compilation is rejected upstream in `build_install_target`.
///
/// External toolchain required: a POSIX `sh`, `make`, a C compiler, and `tar`
/// supporting `--strip-components=1` (GNU tar; bsdtar on Windows 10 build
/// 17063+ behaves differently for that flag — `--source` is primarily a
/// Unix power-user feature, prebuilt binaries are the default install path).
pub(crate) fn install_from_source(
    target: &InstallTarget,
    base_url: &str,
    offline: bool,
    nvm_dir: &Path,
    version_dir: &Path,
) -> Result<()> {
    let ncpus = std::thread::available_parallelism()
        .map(|p| p.get())
        .unwrap_or(1);
    let source_url = get_source_url(&target.target_version, base_url)?;
    let archive_name = &target.archive_name;

    // Extract straight from the cache file. The previous code copied the
    // ~80MB source tarball from `cache/<name>` to `<nvm_dir>/<ver>.src.tmp`
    // and then had `tar` read that copy — a redundant full-file disk write +
    // read on every source install. We now point `tar` directly at the cache
    // file. `owns_src` tracks whether that file is our temp copy (offline
    // path, must be cleaned up) or the shared cache (online path, must stay).
    let temp_src = nvm_dir.join(format!("{}.src.tmp", target.target_version));
    let (src_path, owns_src) = if offline {
        if !is_cached(archive_name) {
            anyhow::bail!(
                "{}",
                format_t(
                    "offline_source_no_cache",
                    std::slice::from_ref(archive_name)
                )
            );
        }
        println!("  {} {}", "ℹ".cyan().bold(), T("using_cache").cyan());
        copy_from_cache(archive_name, &temp_src)?;
        (temp_src, true)
    } else {
        let cached_path = download_to_cache(&source_url, archive_name)?;
        (cached_path, false)
    };

    // RAII guard so the temp source copy is removed on every exit path
    // (including `?` early-returns from tar/configure/make failures), not
    // just the success path. A `let _ = fs::remove_file` at the end of the
    // function was skipped whenever an error bailed out, leaving the ~80MB
    // temp file behind on every failed source install.
    let src_guard = owns_src.then(|| SourceGuard::file(src_path.clone()));

    let build_dir = nvm_dir.join(format!("node-v{}.build", target.target_version));
    fs::create_dir_all(&build_dir)?;
    // Same RAII pattern for the build dir: `remove_dir_all` at the end was
    // skipped on every `?` failure, leaving the extracted source tree behind.
    let build_guard = SourceGuard::dir(build_dir.clone());

    println!("  {} {}", "›".dimmed(), T("source_extract"));
    let status = Command::new("tar")
        .arg("xf")
        .arg(&src_path)
        .arg("-C")
        .arg(&build_dir)
        .arg("--strip-components=1")
        .status()
        .context(T("tar_extract_failed"))?;
    if !status.success() {
        anyhow::bail!(command_failed("extract_source_failed", status));
    }
    // Source extracted into build_dir; the temp tarball copy is no longer
    // needed. Drop the guard early so it doesn't outlive its usefulness
    // (and so a later failure doesn't re-delete an already-removed file).
    drop(src_guard);

    println!(
        "  {} {}",
        "›".dimmed(),
        format_t("source_configure", &[version_dir.display().to_string()])
    );
    let cfg = Command::new("./configure")
        .arg(format!("--prefix={}", version_dir.display()))
        .current_dir(&build_dir)
        .status()
        .context(T("configure_spawn_failed"))?;
    if !cfg.success() {
        anyhow::bail!(command_failed("configure_failed", cfg));
    }

    println!(
        "  {} {}",
        "›".dimmed(),
        format_t("source_make", &[ncpus.to_string()])
    );
    let m = Command::new("make")
        .args(["-j", &ncpus.to_string()])
        .current_dir(&build_dir)
        .status()
        .context(T("make_failed"))?;
    if !m.success() {
        anyhow::bail!(command_failed("make_failed", m));
    }

    println!("  {} {}", "›".dimmed(), T("source_install"));
    let mi = Command::new("make")
        .arg("install")
        .current_dir(&build_dir)
        .status()
        .context(T("make_install_failed"))?;
    if !mi.success() {
        anyhow::bail!(command_failed("make_install_failed", mi));
    }

    // Install succeeded — clean up the build tree, matching the previous
    // `remove_dir_all(&build_dir).ok()` at the end of the happy path. The
    // guard's Drop would do this anyway, but doing it explicitly + disarming
    // avoids a redundant stat in Drop and makes the intent obvious.
    drop(build_guard);

    let npm_path = version_dir.join("bin").join("npm");
    if !npm_path.exists() {
        println!("  {} {}", "ℹ".cyan().bold(), T("source_npm_fetch"));
        download_prebuilt_npm(version_dir, &target.target_version)?;
    }

    println!();
    println!(
        "{} {} {}",
        "✓".green().bold(),
        target.product_name.green().bold(),
        format_t("compiled", std::slice::from_ref(&target.target_version))
            .white()
            .bold()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::ENV_TESTS_MUTEX;
    use crate::test_http::{Mode, RangeMode, TestServer};

    /// `install_from_source` is a pipeline of external commands (`tar`,
    /// `./configure`, `make`, `make install`). None of it was covered, and the
    /// comments above document two bugs that only appear on the *failure* paths
    /// — cleanup that a `?` early-return skipped, leaving an ~80MB temp tarball
    /// or a whole extracted source tree behind. Those paths need the real
    /// function to run, so the toolchain is stubbed instead.
    ///
    /// One script body per tool, with exit codes read from files in the stub
    /// directory: a test selects a scenario by writing a file rather than by
    /// generating a new script. Same pattern as the `gpg` stub in `system.rs`.
    struct ToolchainStub {
        dir: tempfile::TempDir,
        saved_path: Option<std::ffi::OsString>,
        saved_stub_dir: Option<std::ffi::OsString>,
    }

    impl ToolchainStub {
        /// `tar` records its arguments, then plants the `./configure` the real
        /// tarball would have provided, so the next step has something to run.
        #[cfg(unix)]
        const TAR_BODY: &str = r#"D="$NVM_SRC_STUB_DIR"
printf '%s\n' "tar $*" >> "$D/calls.log"
rc=$(cat "$D/tar.rc" 2>/dev/null || echo 0)
[ "$rc" = "0" ] || exit "$rc"
outdir=""
prev=""
for a in "$@"; do
  if [ "$prev" = "-C" ]; then outdir="$a"; fi
  prev="$a"
done
[ -n "$outdir" ] || exit 91
mkdir -p "$outdir" || exit 92
cp "$D/configure.stub" "$outdir/configure" || exit 93
chmod 755 "$outdir/configure" || exit 94
exit 0
"#;

        #[cfg(unix)]
        const CONFIGURE_BODY: &str = r#"D="$NVM_SRC_STUB_DIR"
printf '%s\n' "configure $*" >> "$D/calls.log"
for a in "$@"; do
  case "$a" in
    --prefix=*) printf '%s' "${a#--prefix=}" > "$D/prefix.txt" ;;
  esac
done
exit "$(cat "$D/configure.rc" 2>/dev/null || echo 0)"
"#;

        /// `make` is invoked twice — `make -j N` and `make install` — so the
        /// two have separate exit-code files. A successful install creates
        /// `<prefix>/bin/npm`; without it the caller would fall through to
        /// `download_prebuilt_npm`, which talks to the real npm registry.
        #[cfg(unix)]
        const MAKE_BODY: &str = r#"D="$NVM_SRC_STUB_DIR"
printf '%s\n' "make $*" >> "$D/calls.log"
if [ "$1" = "install" ]; then
  rc=$(cat "$D/make_install.rc" 2>/dev/null || echo 0)
  if [ "$rc" = "0" ]; then
    prefix=$(cat "$D/prefix.txt" 2>/dev/null)
    if [ -n "$prefix" ]; then
      mkdir -p "$prefix/bin"
      printf '#!/bin/sh\n' > "$prefix/bin/npm"
      chmod 755 "$prefix/bin/npm"
    fi
  fi
  exit "$rc"
fi
exit "$(cat "$D/make.rc" 2>/dev/null || echo 0)"
"#;

        /// `None` on Windows: `--source` is rejected there upstream, and a
        /// script stub could not shadow the real `tar`/`make` anyway — Rust
        /// walks `PATHEXT` outside `PATH`, so a real `.exe` further down `PATH`
        /// wins over a stub in the prepended directory (see `GpgStub` in
        /// `system.rs`).
        fn install() -> Option<Self> {
            if cfg!(windows) {
                return None;
            }
            let dir = tempfile::tempdir().expect("tempdir");

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                for (name, body) in [
                    ("tar", Self::TAR_BODY),
                    ("make", Self::MAKE_BODY),
                    // Not on PATH: the tar stub copies this into the build dir,
                    // where `install_from_source` runs it as `./configure`.
                    ("configure.stub", Self::CONFIGURE_BODY),
                ] {
                    let path = dir.path().join(name);
                    fs::write(&path, format!("#!/bin/sh\n{body}")).expect("write stub");
                    // Without 0755 the spawn fails, which would surface as a
                    // confusing tar/configure error rather than a skipped test.
                    let mut perms = fs::metadata(&path).expect("stub metadata").permissions();
                    perms.set_mode(0o755);
                    fs::set_permissions(&path, perms).expect("chmod stub");
                }
            }

            let saved_path = std::env::var_os("PATH");
            let saved_stub_dir = std::env::var_os("NVM_SRC_STUB_DIR");
            let joined = std::env::join_paths(
                std::iter::once(std::ffi::OsString::from(dir.path())).chain(
                    std::env::split_paths(&saved_path.clone().unwrap_or_default())
                        .map(std::ffi::OsString::from),
                ),
            )
            .expect("join PATH");
            std::env::set_var("PATH", joined);
            std::env::set_var("NVM_SRC_STUB_DIR", dir.path());

            Some(Self {
                dir,
                saved_path,
                saved_stub_dir,
            })
        }

        /// Make one tool fail. `name` is `tar`, `configure`, `make` or
        /// `make_install`.
        fn fail(&self, name: &str, code: u8) {
            fs::write(self.dir.path().join(format!("{name}.rc")), code.to_string())
                .expect("write rc");
        }

        /// Every tool invocation, in order, as `"tar <args>"` / `"make <args>"`.
        fn calls(&self) -> Vec<String> {
            fs::read_to_string(self.dir.path().join("calls.log"))
                .unwrap_or_default()
                .lines()
                .map(String::from)
                .collect()
        }
    }

    impl Drop for ToolchainStub {
        fn drop(&mut self) {
            match self.saved_path.take() {
                Some(v) => std::env::set_var("PATH", v),
                None => std::env::remove_var("PATH"),
            }
            match self.saved_stub_dir.take() {
                Some(v) => std::env::set_var("NVM_SRC_STUB_DIR", v),
                None => std::env::remove_var("NVM_SRC_STUB_DIR"),
            }
        }
    }

    /// An isolated `NVM_DIR`, a stubbed toolchain, and a ready-made
    /// `InstallTarget`.
    ///
    /// Declare after the `ENV_TESTS_MUTEX` lock: construction mutates env, and
    /// locals drop in reverse declaration order, so lock-first is what restores
    /// `NVM_DIR` and `PATH` while the lock is still held.
    struct SourceFixture {
        nvm_dir: std::path::PathBuf,
        version_dir: std::path::PathBuf,
        target: InstallTarget,
        stub: ToolchainStub,
        saved_nvm_dir: Option<std::ffi::OsString>,
        // Never read: held so its `Drop` does not delete the tree mid-test.
        _tmp: tempfile::TempDir,
    }

    impl SourceFixture {
        fn new() -> Option<Self> {
            let stub = ToolchainStub::install()?;
            let tmp = tempfile::tempdir().expect("tempdir");
            let nvm_dir = tmp.path().to_path_buf();
            // Pinned literally, not recomputed with the same `format!` as the
            // production code: the on-disk layout is part of the contract
            // (`cache clear`, crash recovery and the guards all name these),
            // so duplicating the expression would hide a change to it.
            let version_dir = nvm_dir.join("v20.0.0");
            let saved_nvm_dir = std::env::var_os("NVM_DIR");
            std::env::set_var("NVM_DIR", &nvm_dir);

            Some(Self {
                version_dir,
                target: InstallTarget {
                    target_version: "20.0.0".to_string(),
                    download_url: "https://example.invalid/unused.tar.gz".to_string(),
                    archive_name: "node-v20.0.0.tar.gz".to_string(),
                    product_name: "Node.js",
                    is_iojs: false,
                },
                nvm_dir,
                stub,
                saved_nvm_dir,
                _tmp: tmp,
            })
        }

        fn cache_dir(&self) -> std::path::PathBuf {
            self.nvm_dir.join("cache")
        }

        /// Put an archive in the cache so the offline path has something to use.
        fn seed_cache(&self, bytes: &[u8]) {
            fs::create_dir_all(self.cache_dir()).expect("create cache dir");
            fs::write(self.cache_dir().join("node-v20.0.0.tar.gz"), bytes).expect("seed cache");
        }

        fn build_dir(&self) -> std::path::PathBuf {
            self.nvm_dir.join("node-v20.0.0.build")
        }

        fn temp_src(&self) -> std::path::PathBuf {
            self.nvm_dir.join("20.0.0.src.tmp")
        }

        fn run(&self, offline: bool, base_url: &str) -> Result<()> {
            install_from_source(
                &self.target,
                base_url,
                offline,
                &self.nvm_dir,
                &self.version_dir,
            )
        }
    }

    impl Drop for SourceFixture {
        fn drop(&mut self) {
            match self.saved_nvm_dir.take() {
                Some(v) => std::env::set_var("NVM_DIR", v),
                None => std::env::remove_var("NVM_DIR"),
            }
        }
    }

    #[test]
    fn source_install_happy_path_assembles_the_expected_commands() {
        let _lock = ENV_TESTS_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let Some(fx) = SourceFixture::new() else {
            return;
        };
        fx.seed_cache(b"not-a-real-tarball");

        fx.run(true, "https://nodejs.org/dist").expect("install");

        let calls = fx.stub.calls();
        assert_eq!(
            calls.len(),
            4,
            "tar, configure, make, make install: {calls:?}"
        );

        // Offline copies the cached archive to a temp file and extracts that,
        // stripping the archive's top-level directory into the build dir.
        let expected_tar = format!(
            "tar xf {} -C {} --strip-components=1",
            fx.temp_src().display(),
            fx.build_dir().display()
        );
        assert_eq!(calls[0], expected_tar);
        // configure gets the install prefix, which is where Node will land.
        assert_eq!(
            calls[1],
            format!("configure --prefix={}", fx.version_dir.display())
        );
        // make is parallelised; the core count is machine-specific, so only the
        // shape is pinned.
        let make_args: Vec<&str> = calls[2].split_whitespace().collect();
        assert_eq!(make_args[0], "make");
        assert_eq!(make_args[1], "-j");
        assert!(
            make_args[2].parse::<u32>().is_ok_and(|n| n > 0),
            "make -j must get a positive core count, got {calls:?}"
        );
        assert_eq!(calls[3], "make install");

        // Both RAII guards fired on the success path: no temp tarball copy, no
        // extracted source tree left behind.
        assert!(!fx.temp_src().exists(), "temp source copy must be removed");
        assert!(!fx.build_dir().exists(), "build tree must be removed");
        // npm came from the stub install, so the registry fallback never ran.
        assert!(fx.version_dir.join("bin").join("npm").exists());
        // The shared cache entry must survive an offline install.
        assert!(
            fx.cache_dir().join("node-v20.0.0.tar.gz").exists(),
            "the cache is shared, not owned by this install"
        );
    }

    #[test]
    fn source_install_cleans_up_when_tar_fails() {
        let _lock = ENV_TESTS_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let Some(fx) = SourceFixture::new() else {
            return;
        };
        fx.seed_cache(b"not-a-real-tarball");
        fx.stub.fail("tar", 3);

        let err = fx
            .run(true, "https://nodejs.org/dist")
            .expect_err("tar fails");

        // `command_failed` renders "<message> (<exit code>)"; the code is not
        // translated, so it is the locale-safe part to assert on.
        assert!(
            format!("{err}").ends_with("(3)"),
            "the exit code should be surfaced, got: {err}"
        );
        assert_eq!(
            fx.stub.calls().len(),
            1,
            "nothing runs after tar: {:?}",
            fx.stub.calls()
        );
        // The bug the guard exists for: a `?` early-return used to skip cleanup.
        assert!(
            !fx.temp_src().exists(),
            "temp source copy must be removed on failure"
        );
        assert!(
            !fx.build_dir().exists(),
            "build tree must be removed on failure"
        );
    }

    #[test]
    fn source_install_cleans_up_when_configure_fails() {
        let _lock = ENV_TESTS_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let Some(fx) = SourceFixture::new() else {
            return;
        };
        fx.seed_cache(b"not-a-real-tarball");
        fx.stub.fail("configure", 4);

        let err = fx
            .run(true, "https://nodejs.org/dist")
            .expect_err("configure fails");

        assert!(
            format!("{err}").ends_with("(4)"),
            "the exit code should be surfaced, got: {err}"
        );
        assert_eq!(
            fx.stub.calls().len(),
            2,
            "make must not run: {:?}",
            fx.stub.calls()
        );
        assert!(!fx.temp_src().exists());
        assert!(
            !fx.build_dir().exists(),
            "build tree must be removed on failure"
        );
    }

    #[test]
    fn source_install_cleans_up_when_make_fails() {
        let _lock = ENV_TESTS_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let Some(fx) = SourceFixture::new() else {
            return;
        };
        fx.seed_cache(b"not-a-real-tarball");
        fx.stub.fail("make", 2);

        let err = fx
            .run(true, "https://nodejs.org/dist")
            .expect_err("make fails");

        assert!(
            format!("{err}").ends_with("(2)"),
            "the exit code should be surfaced, got: {err}"
        );
        assert_eq!(
            fx.stub.calls().len(),
            3,
            "make install must not run: {:?}",
            fx.stub.calls()
        );
        assert!(
            !fx.build_dir().exists(),
            "build tree must be removed on failure"
        );
    }

    #[test]
    fn source_install_cleans_up_when_make_install_fails() {
        let _lock = ENV_TESTS_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let Some(fx) = SourceFixture::new() else {
            return;
        };
        fx.seed_cache(b"not-a-real-tarball");
        fx.stub.fail("make_install", 5);

        let err = fx
            .run(true, "https://nodejs.org/dist")
            .expect_err("make install fails");

        assert!(
            format!("{err}").ends_with("(5)"),
            "the exit code should be surfaced, got: {err}"
        );
        assert_eq!(fx.stub.calls().len(), 4, "{:?}", fx.stub.calls());
        assert!(
            !fx.build_dir().exists(),
            "build tree must be removed on failure"
        );
        // The failure happened before npm was installed, so the registry
        // fallback must not have been reached either.
        assert!(!fx.version_dir.join("bin").join("npm").exists());
    }

    #[test]
    fn source_install_offline_without_cache_bails_before_spawning_anything() {
        let _lock = ENV_TESTS_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let Some(fx) = SourceFixture::new() else {
            return;
        };
        // No seed_cache: `--offline` has nothing to build from.

        let err = fx
            .run(true, "https://nodejs.org/dist")
            .expect_err("offline with no cache must fail");

        // The archive name is interpolated into the message, so it is stable
        // across locales; the surrounding wording is not.
        assert!(
            format!("{err}").contains("node-v20.0.0.tar.gz"),
            "the error should name the missing archive, got: {err}"
        );
        assert!(
            fx.stub.calls().is_empty(),
            "no tool may run without a source archive: {:?}",
            fx.stub.calls()
        );
        assert!(!fx.build_dir().exists());
        assert!(!fx.temp_src().exists());
    }

    #[test]
    fn source_install_online_composes_the_url_from_the_base() {
        let _lock = ENV_TESTS_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let Some(fx) = SourceFixture::new() else {
            return;
        };
        let server = TestServer::start(
            b"downloaded-archive",
            "\"src\"",
            Mode::Range(RangeMode::Honor),
        );

        // A trailing slash on the base must not produce `//v20.0.0/`.
        fx.run(false, &server.url("/")).expect("install");

        let seen = server.requests();
        assert_eq!(seen.len(), 1, "{seen:?}");
        assert_eq!(seen[0].path, "/v20.0.0/node-v20.0.0.tar.gz");

        // Online reads the cache file tar was given and keeps it for reuse.
        let tar_call = &fx.stub.calls()[0];
        assert!(
            tar_call.contains(
                &fx.cache_dir()
                    .join("node-v20.0.0.tar.gz")
                    .display()
                    .to_string()
            ),
            "online must extract straight from the cache, got: {tar_call}"
        );
        assert!(fx.cache_dir().join("node-v20.0.0.tar.gz").exists());
        assert!(!fx.temp_src().exists(), "online owns no temp copy");
        assert!(!fx.build_dir().exists());
    }
}
