/// Tests for the local build scripts (one per platform).
///
/// These verify that the scripts exist and contain the expected commands.
/// Content-verification tests are the right approach for shell/batch
/// scripts (same pattern as tests/p0_fixes.rs).
use std::fs;
use std::path::Path;

fn read_script(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("scripts")
        .join(name);
    fs::read_to_string(&path).unwrap_or_else(|_| panic!("scripts/{} must exist", name))
}

// --- Linux ---

#[test]
fn test_linux_script_exists_and_has_modes() {
    let c = read_script("build-linux.sh");
    assert!(c.contains("release"), "must support release mode");
    assert!(c.contains("check"), "must support check mode");
    assert!(c.contains("cargo build"), "must call cargo build");
    assert!(c.contains("cargo fmt"), "check must run fmt");
    assert!(c.contains("cargo clippy"), "check must run clippy");
    assert!(c.contains("cargo test"), "check must run tests");
}

#[test]
fn test_linux_script_detects_musl() {
    let c = read_script("build-linux.sh");
    assert!(c.contains("musl"), "must detect musl libc for Alpine");
}

#[test]
fn test_linux_script_no_msvc_references() {
    let c = read_script("build-linux.sh");
    assert!(
        !c.contains("vcvars64"),
        "Linux script must not reference MSVC"
    );
    assert!(
        !c.contains("cl.exe"),
        "Linux script must not reference cl.exe"
    );
}

// --- macOS ---

#[test]
fn test_macos_script_exists_and_has_modes() {
    let c = read_script("build-macos.sh");
    assert!(c.contains("release"), "must support release mode");
    assert!(c.contains("check"), "must support check mode");
    assert!(c.contains("cargo build"), "must call cargo build");
    assert!(c.contains("cargo fmt"), "check must run fmt");
    assert!(c.contains("cargo clippy"), "check must run clippy");
    assert!(c.contains("cargo test"), "check must run tests");
}

#[test]
fn test_macos_script_checks_xcode_clt() {
    let c = read_script("build-macos.sh");
    assert!(
        c.contains("xcode-select"),
        "must check for Xcode Command Line Tools"
    );
}

#[test]
fn test_macos_script_no_msvc_references() {
    let c = read_script("build-macos.sh");
    assert!(
        !c.contains("vcvars64"),
        "macOS script must not reference MSVC"
    );
    assert!(
        !c.contains("cl.exe"),
        "macOS script must not reference cl.exe"
    );
}

// --- Windows ---

#[test]
fn test_windows_script_exists_and_has_modes() {
    let c = read_script("build-windows.bat");
    assert!(c.contains("release"), "must support release mode");
    assert!(c.contains("check"), "must support check mode");
    assert!(c.contains("cargo build"), "must call cargo build");
    assert!(c.contains("cargo fmt"), "check must run fmt");
    assert!(c.contains("cargo clippy"), "check must run clippy");
    assert!(c.contains("cargo test"), "check must run tests");
}

#[test]
fn test_windows_script_detects_msvc() {
    let c = read_script("build-windows.bat");
    assert!(
        c.contains("vcvars64"),
        "must detect and load MSVC Build Tools"
    );
    assert!(c.contains("cl.exe"), "must check if cl.exe is on PATH");
}

#[test]
fn test_windows_script_detects_svc_rust() {
    let c = read_script("build-windows.bat");
    assert!(c.contains(".svc"), "must detect .svc Rust installation");
    assert!(c.contains("rust"), "must reference rust path");
}

#[test]
fn test_windows_script_detects_nodejs() {
    let c = read_script("build-windows.bat");
    assert!(
        c.contains("node.exe"),
        "must detect Node.js for corepack test"
    );
}

#[test]
fn test_windows_script_sets_nvm_dir() {
    let c = read_script("build-windows.bat");
    assert!(c.contains("NVM_DIR"), "must set NVM_DIR");
    assert!(c.contains("TEMP"), "must set NVM_DIR to temp dir");
}

#[test]
fn test_windows_script_no_unix_commands() {
    let c = read_script("build-windows.bat");
    assert!(!c.contains("mktemp"), "Windows script must not use mktemp");
    assert!(!c.contains("uname"), "Windows script must not use uname");
}

// --- README ---

#[test]
fn test_scripts_readme_exists_and_lists_all_platforms() {
    let c = read_script("README.md");
    assert!(c.contains("build-linux.sh"), "must list Linux script");
    assert!(c.contains("build-macos.sh"), "must list macOS script");
    assert!(c.contains("build-windows.bat"), "must list Windows script");
    assert!(
        c.contains("Prerequisites"),
        "must have prerequisites section"
    );
}
