//! Content-verification tests for the PowerShell hijack fix.
//!
//! Pre-2.4.0, install.ps1 injected `Import-Module shell\nvm.psm1` into the
//! PowerShell profile, and the module's `nvm` function shadowed nvm.exe:
//! bare `nvm` printed hardcoded help, `-v`/`--version` never reached the
//! binary, exit codes were dropped, and UTF-8-without-BOM text mojibake'd
//! under GBK locales. These tests lock in the fix:
//!
//! - the module is a pure pass-through (no whitelist, no hardcoded help);
//! - the module file is ASCII-only (mojibake-proof regardless of BOM);
//! - install.ps1 no longer injects the module and cleans legacy injections;
//! - release archives keep shipping shell/nvm.psm1.

use std::fs;
use std::path::PathBuf;

fn repo_file(rel: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("{} must exist: {e}", rel))
}

#[test]
fn psm1_nvm_function_is_pure_passthrough() {
    let content = repo_file("shell/nvm.psm1");
    assert!(
        content.contains("& $NvmExe @args"),
        "the nvm function must forward everything to nvm.exe verbatim"
    );
    assert!(
        !content.contains("ValidateSet"),
        "a command whitelist rejects new subcommands and breaks dash flags"
    );
    assert!(
        !content.contains("Show-NvmHelp"),
        "hardcoded help shadowed the binary's real help output"
    );
}

#[test]
fn psm1_is_ascii_only() {
    let bytes = fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("shell/nvm.psm1"))
        .expect("shell/nvm.psm1 must exist");
    assert!(
        bytes.is_ascii(),
        "nvm.psm1 must stay ASCII-only: Windows PowerShell 5.1 decodes \
         BOM-less scripts as ANSI and mojibakes non-ASCII characters"
    );
}

#[test]
fn psm1_cd_hook_forwards_all_parameters() {
    let content = repo_file("shell/nvm.psm1");
    assert!(
        content.contains("Microsoft.PowerShell.Management\\Set-Location @PSBoundParameters"),
        "the Set-Location override must forward every parameter to the real cmdlet"
    );
}

#[test]
fn install_ps1_never_injects_module_and_cleans_legacy() {
    let content = repo_file("install.ps1");
    assert!(
        !content.contains("Added PowerShell module to profile"),
        "install.ps1 must not inject Import-Module into the profile anymore"
    );
    assert!(
        !content.contains("Created PowerShell profile with nvm module"),
        "install.ps1 must not create a profile just to load the module"
    );
    assert!(
        content.contains("Removed legacy nvm.psm1 import from profile"),
        "install.ps1 must clean up the legacy injection on re-install"
    );
}

#[test]
fn package_sh_still_ships_psm1() {
    let content = repo_file("scripts/package.sh");
    assert!(
        content.contains("shell/nvm.psm1"),
        "release archives must keep bundling shell/nvm.psm1 for opt-in cd hooks"
    );
}
