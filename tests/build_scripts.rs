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
    assert!(c.contains("package.sh"), "must list package script");
    assert!(
        c.contains("Prerequisites"),
        "must have prerequisites section"
    );
}

// --- package.sh ---

#[test]
fn test_package_sh_exists_and_has_usage() {
    let c = read_script("package.sh");
    assert!(c.contains("Usage:"), "must have usage instructions");
    assert!(
        c.contains("binary_path"),
        "usage must mention binary_path arg"
    );
    assert!(c.contains("version"), "usage must mention version arg");
    assert!(c.contains("os"), "usage must mention os arg");
    assert!(c.contains("arch"), "usage must mention arch arg");
}

#[test]
fn test_package_sh_handles_both_formats() {
    let c = read_script("package.sh");
    assert!(c.contains("tar.gz"), "must support tar.gz format");
    assert!(c.contains("zip"), "must support zip format");
}

#[test]
fn test_package_sh_includes_friendly_pack_files() {
    let c = read_script("package.sh");
    assert!(c.contains("README.md"), "must include README.md");
    assert!(c.contains("README.ZH_CN.md"), "must include Chinese README");
    assert!(c.contains("LICENSE"), "must include LICENSE");
    assert!(c.contains("install.sh"), "must include install.sh");
    assert!(c.contains("install.ps1"), "must include install.ps1");
    assert!(c.contains("shell/nvm.sh"), "must include nvm.sh");
    assert!(c.contains("shell/nvm.fish"), "must include nvm.fish");
    assert!(c.contains("shell/nvm.psm1"), "must include nvm.psm1");
}

#[test]
fn test_package_sh_detects_format_by_os() {
    let c = read_script("package.sh");
    assert!(
        c.contains("windows") && c.contains("zip"),
        "Windows must use zip"
    );
}

#[test]
fn test_package_sh_cleans_up_staging() {
    let c = read_script("package.sh");
    assert!(c.contains("rm -rf"), "must clean up staging directory");
    assert!(c.contains("STAGE"), "must use a staging directory");
}

#[test]
fn test_release_yml_calls_package_sh() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(".github")
        .join("workflows")
        .join("release.yml");
    let c = fs::read_to_string(&path).expect("release.yml must exist");

    assert!(
        c.contains("scripts/package.sh"),
        "release.yml must call scripts/package.sh for packaging (single source of truth)"
    );
    assert!(
        !c.contains("Stage friendly-pack files"),
        "release.yml must NOT have inline staging (moved to package.sh)"
    );
}
