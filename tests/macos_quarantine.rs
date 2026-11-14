//! P2-7: macOS Gatekeeper quarantine handling.
//!
//! Measured on a real Mac against the v2.3.6 release assets before choosing
//! where the fix belongs:
//!
//! - `nvm install --lts` produced 4753 files with **zero**
//!   `com.apple.quarantine` attributes. `reqwest` downloads over plain
//!   TCP/TLS and never goes through LaunchServices, so nothing tags the
//!   Node.js tarball or what gets extracted from it.
//! - The `tar` crate used by `src/extract.rs` defaults to
//!   `unpack_xattrs: false`, so xattrs stored *inside* an archive are not
//!   written out either. Both controls (`tar --xattrs` and
//!   `set_unpack_xattrs(true)`) did write them, so the negative result is a
//!   real measurement and not a vacuous one.
//! - A release archive downloaded through a browser *is* quarantined, Archive
//!   Utility propagates the attribute to every extracted file, `cp -f`
//!   preserves it, and the unsigned `nvm` binary is then blocked by Gatekeeper
//!   (`spctl -a -t execute` → `rejected / no usable signature`).
//!
//! So the defect lives in `install.sh`, which copies a possibly-quarantined
//! binary into place, plus the README guidance for users who extract by hand.
//! `extract.rs` never sees a quarantined file and must stay untouched.

use std::fs;
use std::path::Path;

fn read_root_file(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(name);
    fs::read_to_string(&path).unwrap_or_else(|_| panic!("{} must exist", name))
}

/// The `clear_quarantine()` body on its own, so an assertion can never be
/// satisfied by an unrelated occurrence elsewhere in the script.
fn quarantine_helper() -> String {
    let script = read_root_file("install.sh");
    let start = script
        .find("clear_quarantine() {")
        .expect("install.sh must define clear_quarantine()");
    let body = &script[start..];
    // The loop's closing brace is indented, so a brace at column 0 ends the fn.
    let end = body.find("\n}").expect("clear_quarantine() must be closed");
    body[..end].to_string()
}

/// The `clear_quarantine` invocation inside `main()`.
fn quarantine_call() -> String {
    let script = read_root_file("install.sh");
    let start = script
        .find("clear_quarantine \"/usr/local/bin/")
        .expect("main() must call clear_quarantine on the installed paths");
    let tail = &script[start..];
    let end = tail.find('\n').expect("call must be terminated");
    tail[..end].to_string()
}

/// A slice of `content` from the `start` heading up to the `end` heading, so
/// an assertion can only be satisfied inside the section it is about.
fn section<'a>(content: &'a str, start: &str, end: &str) -> &'a str {
    let from = content
        .find(start)
        .unwrap_or_else(|| panic!("missing section heading {:?}", start));
    let to = content[from..]
        .find(end)
        .map(|offset| from + offset)
        .unwrap_or(content.len());
    &content[from..to]
}

#[test]
fn install_sh_clears_only_the_quarantine_attribute() {
    let helper = quarantine_helper();
    assert!(
        // Match the invocation (`"$target"`), not the warn text (`${target}`),
        // so deleting the command cannot hide behind the message that quotes it.
        helper.contains("xattr -dr com.apple.quarantine \"$target\""),
        "must delete the quarantine key recursively: {}",
        helper
    );
    // `-c` wipes every attribute (com.apple.provenance, com.apple.metadata,
    // com.apple.quarantine, ...). Only the Gatekeeper key is in the way.
    assert!(
        !helper.contains("xattr -cr") && !helper.contains("xattr -c "),
        "must not use the blanket `xattr -c` form: {}",
        helper
    );
}

#[test]
fn install_sh_guards_the_helper_to_macos_with_xattr() {
    let helper = quarantine_helper();
    assert!(
        helper.contains("[ \"$(uname -s)\" = \"Darwin\" ] || return 0"),
        "quarantine is a macOS concept; other platforms must be a no-op: {}",
        helper
    );
    assert!(
        helper.contains("command -v xattr"),
        "must tolerate a missing xattr binary: {}",
        helper
    );
    assert!(
        helper.contains("[ -e \"$target\" ] || continue"),
        "must skip paths that were not installed (e.g. no /usr/local/bin write): {}",
        helper
    );
}

#[test]
fn install_sh_warns_instead_of_aborting_when_clearing_fails() {
    let script = read_root_file("install.sh");
    assert!(
        script.contains("set -euo pipefail"),
        "install.sh runs with errexit, so an unguarded failure would abort"
    );
    let helper = quarantine_helper();
    assert!(
        helper.contains("2>/dev/null ||"),
        "a failed clear must not abort the install: {}",
        helper
    );
    assert!(
        helper.contains("warn \"Could not clear com.apple.quarantine"),
        "a failed clear must tell the user the manual command: {}",
        helper
    );
}

#[test]
fn install_sh_clears_quarantine_after_installing_every_file() {
    let script = read_root_file("install.sh");
    let installed_at = script
        .find("Installed to /usr/local/bin")
        .expect("install.sh must report the installed binary");
    let cleared_at = script
        .find("clear_quarantine \"/usr/local/bin/")
        .expect("install.sh must clear quarantine on the installed paths");
    assert!(
        cleared_at > installed_at,
        "clearing before the binary is copied would leave it quarantined"
    );

    // Every path that receives a `cp -f` from the (possibly quarantined)
    // release archive has to be covered.
    let call = quarantine_call();
    assert!(
        call.contains("\"${INSTALL_DIR}\""),
        "must cover the user install dir (bin/nvm, bin/nvm.sh): {}",
        call
    );
    assert!(
        call.contains("\"${shell_dir}\""),
        "must cover the copied shell integration scripts: {}",
        call
    );
}

#[test]
fn extract_rs_stays_free_of_quarantine_handling() {
    // Regression guard for the audit's original — and measured-wrong — P2-7
    // location. Node.js tarballs are fetched by reqwest and unpacked by the
    // `tar` crate with `unpack_xattrs` defaulted to false, so nothing in this
    // path can produce a quarantined file. Adding an xattr call here would be
    // dead weight on every install; see the module docs for the measurements.
    let source = read_root_file("src/extract.rs");
    assert!(
        !source.contains("xattr"),
        "src/extract.rs must not grow quarantine handling"
    );
    assert!(
        !source.contains("quarantine"),
        "src/extract.rs must not grow quarantine handling"
    );
}

#[test]
fn readmes_document_the_gatekeeper_symptom_and_remedy() {
    let english = read_root_file("README.md");
    let manual = section(
        &english,
        "### Download manually",
        "### Installation options",
    );
    assert!(
        manual.contains("xattr -dr com.apple.quarantine"),
        "README.md must give the manual remedy for hand-extracted releases"
    );
    let troubleshooting = section(&english, "### Troubleshooting", "## Shell Integration");
    assert!(
        troubleshooting.contains("cannot be opened because the developer cannot be verified"),
        "README.md troubleshooting must name the symptom users actually see"
    );
    assert!(
        troubleshooting.contains("xattr -dr com.apple.quarantine"),
        "README.md troubleshooting must give the remedy next to the symptom"
    );

    let chinese = read_root_file("README.ZH_CN.md");
    let manual_zh = section(&chinese, "### 手动下载", "### 安装选项");
    assert!(
        manual_zh.contains("xattr -dr com.apple.quarantine"),
        "README.ZH_CN.md must give the manual remedy for hand-extracted releases"
    );
    let troubleshooting_zh = section(&chinese, "### 安装排错", "## Shell 集成");
    assert!(
        troubleshooting_zh.contains("身份不明的开发者"),
        "README.ZH_CN.md troubleshooting must name the symptom users actually see"
    );
    assert!(
        troubleshooting_zh.contains("xattr -dr com.apple.quarantine"),
        "README.ZH_CN.md troubleshooting must give the remedy next to the symptom"
    );
}

// --- Behavioural checks for the remedy the script relies on ---------------
//
// `xattr -w` needs no privileges, so the quarantined state is synthesised
// rather than mocked. These assert the two facts `install.sh` depends on:
// `cp` carries the attribute over, and `xattr -dr` takes it back off.

#[cfg(target_os = "macos")]
mod macos_behavior {
    use std::fs;
    use std::path::Path;
    use std::process::Command;
    use tempfile::TempDir;

    /// The value a browser writes: `<flags>;<timestamp hex>;<agent>;<uuid>`.
    const BROWSER_VALUE: &str = "0083;6af700f3;Chrome;3B1ACED4-DBAF-471B-8C5F-8C5A2C6D4C7F";

    fn xattr(args: &[&str]) -> std::process::Output {
        Command::new("xattr")
            .args(args)
            .output()
            .expect("xattr ships with macOS")
    }

    fn path_arg(path: &Path) -> &str {
        path.to_str().expect("temp paths are valid UTF-8")
    }

    fn has_quarantine(path: &Path) -> bool {
        xattr(&["-p", "com.apple.quarantine", path_arg(path)])
            .status
            .success()
    }

    /// Stamp the attribute and prove the fixture really is quarantined, so a
    /// later "attribute is gone" assertion cannot pass vacuously.
    fn stamp_quarantine(path: &Path) {
        let out = xattr(&["-w", "com.apple.quarantine", BROWSER_VALUE, path_arg(path)]);
        assert!(out.status.success(), "xattr -w must succeed");
        assert!(
            has_quarantine(path),
            "fixture must be quarantined before the test acts on it"
        );
    }

    /// Exactly what `clear_quarantine()` in install.sh runs.
    fn clear_quarantine(target: &Path) -> bool {
        xattr(&["-dr", "com.apple.quarantine", path_arg(target)])
            .status
            .success()
    }

    #[test]
    fn cp_preserves_quarantine_which_is_why_install_sh_must_clear_it() {
        let dir = TempDir::new().expect("tempdir");
        let source = dir.path().join("nvm");
        fs::write(&source, b"binary").expect("write");
        stamp_quarantine(&source);

        // install.sh installs with `cp -f`; the copy keeps the attribute.
        let installed = dir.path().join("installed-nvm");
        let status = Command::new("cp")
            .args(["-f", path_arg(&source), path_arg(&installed)])
            .status()
            .expect("cp ships with macOS");
        assert!(status.success(), "cp -f must succeed");
        assert!(
            has_quarantine(&installed),
            "cp -f must carry quarantine over, otherwise no fix would be needed"
        );

        assert!(clear_quarantine(&installed), "clearing must succeed");
        assert!(
            !has_quarantine(&installed),
            "install.sh's remedy must leave the installed binary unquarantined"
        );
    }

    #[test]
    fn clear_is_recursive_so_a_whole_install_dir_can_be_passed() {
        let dir = TempDir::new().expect("tempdir");
        let nested = dir.path().join("shell").join("nested");
        fs::create_dir_all(&nested).expect("mkdir -p");
        let top = dir.path().join("nvm.sh");
        let deep = nested.join("nvm.fish");
        fs::write(&top, b"sh").expect("write");
        fs::write(&deep, b"fish").expect("write");
        stamp_quarantine(&top);
        stamp_quarantine(&deep);

        assert!(clear_quarantine(dir.path()), "clearing a dir must succeed");
        assert!(!has_quarantine(&top), "top-level file must be cleared");
        assert!(
            !has_quarantine(&deep),
            "files below the target dir must be cleared too"
        );
    }

    #[test]
    fn clear_leaves_unrelated_attributes_alone() {
        let dir = TempDir::new().expect("tempdir");
        let target = dir.path().join("nvm");
        fs::write(&target, b"binary").expect("write");
        stamp_quarantine(&target);
        assert!(
            xattr(&["-w", "com.nvmrust.keepme", "v1", path_arg(&target)])
                .status
                .success(),
            "writing an unrelated attribute must succeed"
        );

        assert!(clear_quarantine(&target), "clearing must succeed");
        assert!(!has_quarantine(&target), "quarantine must be gone");
        assert!(
            xattr(&["-p", "com.nvmrust.keepme", path_arg(&target)])
                .status
                .success(),
            "unrelated attributes must survive — this is why the fix uses -d, not -c"
        );
    }

    #[test]
    fn clear_on_an_unquarantined_target_is_a_no_op() {
        // The `curl | bash` path never sets quarantine, so the helper runs on
        // clean files for most installs. That must stay silent and successful
        // under `set -euo pipefail`.
        let dir = TempDir::new().expect("tempdir");
        let target = dir.path().join("nvm");
        fs::write(&target, b"binary").expect("write");
        assert!(!has_quarantine(&target), "fixture starts clean");

        assert!(
            clear_quarantine(&target),
            "clearing a clean file must exit 0"
        );
        assert!(
            clear_quarantine(&target),
            "clearing twice must stay exit 0 (idempotent)"
        );
    }
}
