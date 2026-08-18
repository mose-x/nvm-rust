use std::fs;

/// P1-1: unload() must warn on shim removal errors, not silently swallow them.
#[test]
fn p1_1_unload_warns_on_shim_removal_error() {
    let lifecycle_rs =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/commands/lifecycle.rs");
    let content = fs::read_to_string(&lifecycle_rs).expect("lifecycle.rs must exist");

    // The unload function should use if-let-Err instead of let _ =
    let unload_start = content.find("pub fn unload()").unwrap_or(0);
    let unload_section = &content[unload_start..];
    let unload_end = unload_section
        .find("\n}")
        .map(|i| unload_start + i)
        .unwrap_or(content.len());
    let unload_body = &content[unload_start..unload_end];

    assert!(
        !unload_body.contains("let _ = crate::shim::remove_shims()"),
        "unload must NOT use 'let _ =' for shim removal — errors must be surfaced"
    );
    assert!(
        unload_body.contains("if let Err(e) = crate::shim::remove_shims()"),
        "unload must use 'if let Err(e)' to surface shim removal errors"
    );
}

/// P1-2: deactivate() must write "none" marker instead of deleting current file.
#[test]
fn p1_2_deactivate_writes_none_marker() {
    let lifecycle_rs =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/commands/lifecycle.rs");
    let content = fs::read_to_string(&lifecycle_rs).expect("lifecycle.rs must exist");

    let deactivate_start = content.find("pub fn deactivate()").unwrap_or(0);
    let deactivate_section = &content[deactivate_start..];
    let deactivate_end = deactivate_section
        .find("\n}")
        .map(|i| deactivate_start + i)
        .unwrap_or(content.len());
    let deactivate_body = &content[deactivate_start..deactivate_end];

    assert!(
        deactivate_body.contains("\"none\""),
        "deactivate must write 'none' marker to current file instead of deleting it"
    );
    assert!(
        !deactivate_body.contains("remove_file(&current_file)"),
        "deactivate must NOT delete current file — write 'none' instead"
    );
}

/// P1-2: shim scripts must check for "none" marker.
#[test]
fn p1_2_shim_scripts_check_none_marker() {
    let shim_rs = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/shim.rs");
    let content = fs::read_to_string(&shim_rs).expect("shim.rs must exist");

    // Unix shim script
    assert!(
        content.contains(r#"if [ "$CURRENT" = "none" ]"#),
        "Unix shim script must check for 'none' marker"
    );
    // Windows shim script
    assert!(
        content.contains(r#"if "%CURRENT%"=="none""#),
        "Windows shim script must check for 'none' marker"
    );
}

/// P1-2: install.sh shim script must also check for "none" marker.
#[test]
fn p1_2_install_sh_checks_none_marker() {
    let install_sh = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("install.sh");
    let content = fs::read_to_string(&install_sh).expect("install.sh must exist");

    assert!(
        content.contains(r#"[ "$CURRENT" = "none" ]"#),
        "install.sh shim script must check for 'none' marker"
    );
}

/// P1-2: install.ps1 shim script must also check for "none" marker.
#[test]
fn p1_2_install_ps1_checks_none_marker() {
    let install_ps1 = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("install.ps1");
    let content = fs::read_to_string(&install_ps1).expect("install.ps1 must exist");

    assert!(
        content.contains(r#""%CURRENT%"=="none""#),
        "install.ps1 shim script must check for 'none' marker"
    );
}

/// P1-8: nvm.fish must respect $NVM_DIR environment variable.
#[test]
fn p1_8_nvm_fish_respects_nvm_dir() {
    let nvm_fish = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("shell")
        .join("nvm.fish");
    let content = fs::read_to_string(&nvm_fish).expect("nvm.fish must exist");

    assert!(
        content.contains("test -n \"$NVM_DIR\""),
        "nvm.fish must check if NVM_DIR is set and non-empty (test -n)"
    );
    assert!(
        !content.contains(r#"set -g NVM_RUST_DIR "$HOME/.nvm.rust""#),
        "nvm.fish must NOT hardcode $HOME/.nvm.rust"
    );
}

/// P1-8: nvm.fish must add shims to PATH.
#[test]
fn p1_8_nvm_fish_adds_shims_to_path() {
    let nvm_fish = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("shell")
        .join("nvm.fish");
    let content = fs::read_to_string(&nvm_fish).expect("nvm.fish must exist");

    assert!(
        content.contains("NVM_RUST_SHIMS"),
        "nvm.fish must define NVM_RUST_SHIMS"
    );
    assert!(
        content.contains("$NVM_RUST_SHIMS"),
        "nvm.fish must add NVM_RUST_SHIMS to PATH"
    );
}

/// P1-9: nvm.psm1 Set-Location must guard for null $Path.
#[test]
fn p1_9_psm1_set_location_guards_null_path() {
    let nvm_psm1 = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("shell")
        .join("nvm.psm1");
    let content = fs::read_to_string(&nvm_psm1).expect("nvm.psm1 must exist");

    assert!(
        content.contains("if (-not $Path)"),
        "Set-Location must guard for null/empty $Path"
    );
}

/// P1-10: nvm.psm1 ValidateSet must include new commands.
#[test]
fn p1_10_psm1_validateset_includes_new_commands() {
    let nvm_psm1 = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("shell")
        .join("nvm.psm1");
    let content = fs::read_to_string(&nvm_psm1).expect("nvm.psm1 must exist");

    for cmd in &["install-yarn", "install-pnpm", "upgrade", "migrate"] {
        assert!(
            content.contains(&format!("'{}'", cmd)),
            "nvm.psm1 ValidateSet must include '{}'",
            cmd
        );
    }
}

/// P1-11: nvm.fish help text must include new commands.
#[test]
fn p1_11_nvm_fish_help_includes_new_commands() {
    let nvm_fish = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("shell")
        .join("nvm.fish");
    let content = fs::read_to_string(&nvm_fish).expect("nvm.fish must exist");

    for cmd in &[
        "install-npm",
        "install-yarn",
        "install-pnpm",
        "upgrade",
        "migrate",
    ] {
        assert!(
            content.contains(cmd),
            "nvm.fish help text must include '{}'",
            cmd
        );
    }
}

/// P1-3: CI must include cargo audit step.
#[test]
fn p1_3_ci_includes_cargo_audit() {
    let ci_yml = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(".github")
        .join("workflows")
        .join("ci.yml");
    let content = fs::read_to_string(&ci_yml).expect("ci.yml must exist");

    assert!(
        content.contains("audit") || content.contains("rustsec"),
        "CI must include a cargo audit / security scan step"
    );
}

/// P1-5: listing.rs uninstall must have auto-switch logic.
#[test]
fn p1_5_uninstall_has_auto_switch_logic() {
    let listing_rs =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/commands/listing.rs");
    let content = fs::read_to_string(&listing_rs).expect("listing.rs must exist");

    assert!(
        content.contains("next_available_version"),
        "uninstall must call next_available_version for auto-switch"
    );
    assert!(
        content.contains("atomic_write(&current_file, &next)"),
        "uninstall must write the next version to current file"
    );
}

/// P1-6: swap_binary function and BAK_SUFFIX constant must exist for rollback.
/// After P1-7 refactoring these moved from upgrade.rs to binary_swap.rs,
/// re-exported through upgrade::* so `commands::swap_binary` still works.
#[test]
fn p1_6_upgrade_has_swap_binary() {
    let binary_swap_rs =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/commands/binary_swap.rs");
    let content = fs::read_to_string(&binary_swap_rs).expect("binary_swap.rs must exist");

    assert!(
        content.contains("fn swap_binary"),
        "binary_swap.rs must have swap_binary function for rollback"
    );
    assert!(
        content.contains("BAK_SUFFIX"),
        "binary_swap.rs must have BAK_SUFFIX constant for backup file naming"
    );

    // Verify upgrade.rs re-exports binary_swap so callers are unaffected.
    let upgrade_rs =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/commands/upgrade.rs");
    let upgrade_content = fs::read_to_string(&upgrade_rs).expect("upgrade.rs must exist");
    assert!(
        upgrade_content.contains("binary_swap::*"),
        "upgrade.rs must re-export binary_swap module"
    );
}

/// Regression: get_current_version() must filter "none" marker.
/// When deactivate() writes "none" to the current file, get_current_version()
/// should return None (not Some("none")), so callers don't treat "none" as
/// a valid version directory path.
#[test]
fn regression_get_current_version_filters_none_marker() {
    let mod_rs = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/commands/mod.rs");
    let content = fs::read_to_string(&mod_rs).expect("mod.rs must exist");

    // The function must check for "none" alongside the empty check.
    let func_start = content.find("fn get_current_version()").unwrap_or(0);
    let func_section = &content[func_start..];
    let func_end = func_section
        .find("\n}")
        .map(|i| func_start + i + 2)
        .unwrap_or(content.len());
    let func_body = &content[func_start..func_end];

    assert!(
        func_body.contains("\"none\""),
        "get_current_version() must filter 'none' marker — when current file contains 'none', \
         it should return None, not Some(\"none\")"
    );
}

/// Regression: nvm.fish PATH order must be shims:bin:rest (shims first).
/// BIN must be prepended first, then SHIMS, so SHIMS ends up at front.
#[test]
fn regression_fish_path_order_is_shims_before_bin() {
    let nvm_fish = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("shell")
        .join("nvm.fish");
    let content = fs::read_to_string(&nvm_fish).expect("nvm.fish must exist");

    // Find the PATH prepend section.
    // BIN should be prepended first (appears first in the code),
    // then SHIMS (appears second) — so SHIMS ends up at the front.
    let bin_pos = content.find("NVM_RUST_BIN").unwrap_or(usize::MAX);
    let shims_pos = content.find("NVM_RUST_SHIMS").unwrap_or(usize::MAX);
    assert!(
        bin_pos < shims_pos,
        "nvm.fish must prepend BIN first, then SHIMS (so SHIMS ends up at front). \
         BIN at pos {}, SHIMS at pos {}",
        bin_pos,
        shims_pos
    );
}

/// P0-1: nvm.psm1 must export Set-Location so the auto-switch-on-cd override
/// actually replaces the global `cd`/`Set-Location` in the user's session.
#[test]
fn p0_1_psm1_exports_set_location() {
    let nvm_psm1 = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("shell")
        .join("nvm.psm1");
    let content = fs::read_to_string(&nvm_psm1).expect("nvm.psm1 must exist");
    assert!(
        content.contains("Export-ModuleMember") && content.contains("Set-Location"),
        "nvm.psm1 must export Set-Location so auto-switch-on-cd works"
    );
}

/// P0-2: nvm.psm1 must respect $env:NVM_DIR instead of hardcoding the path.
#[test]
fn p0_2_psm1_respects_nvm_dir() {
    let nvm_psm1 = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("shell")
        .join("nvm.psm1");
    let content = fs::read_to_string(&nvm_psm1).expect("nvm.psm1 must exist");
    assert!(
        content.contains("$env:NVM_DIR"),
        "nvm.psm1 must respect $env:NVM_DIR (not hardcode the path)"
    );
}

/// P1-1: All three shell wrappers must forward extra args (e.g. --silent) to
/// `nvm auto`, not pass a literal "auto" that drops the flag.
#[test]
fn p1_1_wrappers_forward_auto_args() {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));

    // nvm.sh: auto case must use "$@" not literal "auto"
    let nvm_sh = manifest.join("shell/nvm.sh");
    let sh = fs::read_to_string(&nvm_sh).expect("nvm.sh must exist");
    assert!(
        !sh.contains("\"${NVM_RUST_BIN}/nvm\" auto\n"),
        "nvm.sh must not pass literal 'auto' to nvm binary (drops --silent)"
    );

    // nvm.fish: auto case must use $argv not literal "auto"
    let nvm_fish = manifest.join("shell/nvm.fish");
    let fish = fs::read_to_string(&nvm_fish).expect("nvm.fish must exist");
    assert!(
        !fish.contains("\"$NVM_RUST_BIN/nvm\" auto\n"),
        "nvm.fish must not pass literal 'auto' to nvm binary (drops --silent)"
    );

    // nvm.psm1: auto case must pass $Arguments
    let nvm_psm1 = manifest.join("shell/nvm.psm1");
    let psm1 = fs::read_to_string(&nvm_psm1).expect("nvm.psm1 must exist");
    assert!(
        psm1.contains("& $NvmExe auto $Arguments"),
        "nvm.psm1 must pass $Arguments to nvm auto (forwards --silent)"
    );
}

/// P1-2: All three shell wrappers must accept `nvm use` with no args and let
/// the binary handle the default-version fallback (not reject it early).
#[test]
fn p1_2_wrappers_accept_use_no_args() {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));

    // nvm.sh: must not have the "$# -lt 2" check for use
    let nvm_sh = manifest.join("shell/nvm.sh");
    let sh = fs::read_to_string(&nvm_sh).expect("nvm.sh must exist");
    assert!(
        !sh.contains("Usage: nvm use <version>"),
        "nvm.sh must not reject 'nvm use' with no args (binary supports default fallback)"
    );

    // nvm.fish: must not have the "test -z \"$ver\"" check for use
    let nvm_fish = manifest.join("shell/nvm.fish");
    let fish = fs::read_to_string(&nvm_fish).expect("nvm.fish must exist");
    assert!(
        !fish.contains("Usage: nvm use <version>"),
        "nvm.fish must not reject 'nvm use' with no args"
    );

    // nvm.psm1: must not have the "-not $Arguments" check for use
    let nvm_psm1 = manifest.join("shell/nvm.psm1");
    let psm1 = fs::read_to_string(&nvm_psm1).expect("nvm.psm1 must exist");
    assert!(
        !psm1.contains("Usage: nvm use <version>"),
        "nvm.psm1 must not reject 'nvm use' with no args"
    );
}

/// P1-3: nvm.fish must have a __nvm_strip_path function and call it in
/// deactivate and unload (matching nvm.sh's _nvm_strip_path behavior).
#[test]
fn p1_3_fish_has_strip_path() {
    let nvm_fish = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("shell")
        .join("nvm.fish");
    let content = fs::read_to_string(&nvm_fish).expect("nvm.fish must exist");
    assert!(
        content.contains("function __nvm_strip_path"),
        "nvm.fish must define __nvm_strip_path function"
    );
    // Verify it's called in deactivate and unload
    let deactivate_pos = content.find("case deactivate");
    let unload_pos = content.find("case unload");
    assert!(
        deactivate_pos.is_some(),
        "nvm.fish must have deactivate case"
    );
    assert!(unload_pos.is_some(), "nvm.fish must have unload case");
    let deactivate_section = &content[deactivate_pos.unwrap()..unload_pos.unwrap()];
    assert!(
        deactivate_section.contains("__nvm_strip_path"),
        "nvm.fish deactivate must call __nvm_strip_path"
    );
    let unload_section = &content[unload_pos.unwrap()..];
    assert!(
        unload_section.contains("__nvm_strip_path"),
        "nvm.fish unload must call __nvm_strip_path"
    );
}

/// P1-8: nvm.psm1 ValidateSet must include version/help flags so they pass
/// through to the binary instead of being rejected by validation.
#[test]
fn p1_8_psm1_validateset_includes_version_flags() {
    let nvm_psm1 = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("shell")
        .join("nvm.psm1");
    let content = fs::read_to_string(&nvm_psm1).expect("nvm.psm1 must exist");
    for flag in &["--version", "-V", "-v", "--help", "-h"] {
        assert!(
            content.contains(&format!("'{}'", flag)),
            "nvm.psm1 ValidateSet must include '{}'",
            flag
        );
    }
}

/// P1-9: nvm.psm1 Remove-NvmFromPath must remove entries from the CURRENT
/// PATH, not restore a stale snapshot from module load ($script:OriginalPath).
#[test]
fn p1_9_psm1_remove_from_path_uses_current() {
    let nvm_psm1 = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("shell")
        .join("nvm.psm1");
    let content = fs::read_to_string(&nvm_psm1).expect("nvm.psm1 must exist");
    assert!(
        !content.contains("$env:Path = $script:OriginalPath"),
        "nvm.psm1 Remove-NvmFromPath must not restore stale OriginalPath snapshot"
    );
    assert!(
        content.contains("Where-Object"),
        "nvm.psm1 Remove-NvmFromPath must filter current PATH elements"
    );
}

/// P2-5: nvm.sh deactivate must not blindly swallow all stderr with 2>/dev/null.
/// Should check the exit code and report warnings to the user.
#[test]
fn p2_5_nvm_sh_deactivate_checks_exit_code() {
    let nvm_sh = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("shell")
        .join("nvm.sh");
    let content = fs::read_to_string(&nvm_sh).expect("nvm.sh must exist");
    // The old code had `deactivate 2>/dev/null` which swallowed all errors.
    // The fix uses an if/else to check the exit code.
    assert!(
        !content.contains("deactivate 2>/dev/null"),
        "nvm.sh deactivate must not swallow all stderr with 2>/dev/null"
    );
}

/// P2-6: nvm.sh unload must unset ALL NVM_RUST_* variables, not just
/// NVM_RUST_SOURCED and NVM_RUST_AUTO_SWITCH_DONE.
#[test]
fn p2_6_nvm_sh_unload_unsets_all_vars() {
    let nvm_sh = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("shell")
        .join("nvm.sh");
    let content = fs::read_to_string(&nvm_sh).expect("nvm.sh must exist");
    // Check each variable appears on an `unset` line (bash allows multiple
    // vars per unset: `unset VAR1 VAR2 VAR3`).
    for var in &[
        "NVM_RUST_DIR",
        "NVM_RUST_BIN",
        "NVM_RUST_SHIMS",
        "NVM_RUST_ACTIVE",
    ] {
        let found = content
            .lines()
            .any(|line| line.contains("unset") && line.contains(var));
        assert!(found, "nvm.sh unload must unset {}", var);
    }
}

/// P2-8: nvm.sh must bootstrap compinit for zsh so completions actually load
/// even if the user's zshrc hasn't called compinit.
#[test]
fn p2_8_nvm_sh_bootstraps_compinit() {
    let nvm_sh = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("shell")
        .join("nvm.sh");
    let content = fs::read_to_string(&nvm_sh).expect("nvm.sh must exist");
    assert!(
        content.contains("compinit"),
        "nvm.sh must bootstrap compinit for zsh completions"
    );
}

/// P2-9: nvm.sh auto-switch hook must fall back to `uname` when OSTYPE is
/// unset (plain sh / some non-bash shells), matching the PATH setup pattern.
#[test]
fn p2_9_nvm_sh_auto_switch_has_uname_fallback() {
    let nvm_sh = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("shell")
        .join("nvm.sh");
    let content = fs::read_to_string(&nvm_sh).expect("nvm.sh must exist");
    assert!(
        content.contains("uname -s") && content.contains("_ostype"),
        "nvm.sh auto-switch hook must fall back to uname when OSTYPE is unset"
    );
}

/// P2-16: nvm.fish must use `test -n` (not `set -q`) for NVM_DIR so an
/// empty-string NVM_DIR doesn't break all paths.
#[test]
fn p2_16_nvm_fish_nvm_dir_empty_guard() {
    let nvm_fish = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("shell")
        .join("nvm.fish");
    let content = fs::read_to_string(&nvm_fish).expect("nvm.fish must exist");
    assert!(
        content.contains("test -n \"$NVM_DIR\""),
        "nvm.fish must use test -n (not set -q) to guard against empty NVM_DIR"
    );
}

/// P1-1: nvm.fish unload must erase __nvm_auto_switch to prevent zombie hook
/// firing on every cd after unload.
#[test]
fn p1_1_fish_unload_erases_auto_switch() {
    let nvm_fish = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("shell")
        .join("nvm.fish");
    let content = fs::read_to_string(&nvm_fish).expect("nvm.fish must exist");
    let unload_pos = content.find("case unload").expect("must have unload case");
    let unload_section = &content[unload_pos..];
    assert!(
        unload_section.contains("functions -e __nvm_auto_switch"),
        "nvm.fish unload must erase __nvm_auto_switch (zombie hook prevention)"
    );
}

/// P2-1: nvm.fish .nvmrc parsing must extract first token (using awk),
/// not read the entire file. Handles comments and multi-line .nvmrc.
#[test]
fn p2_1_fish_nvmrc_uses_awk() {
    let nvm_fish = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("shell")
        .join("nvm.fish");
    let content = fs::read_to_string(&nvm_fish).expect("nvm.fish must exist");
    assert!(
        content.contains("head -1 .nvmrc | awk"),
        "nvm.fish must use head -1 + awk to parse .nvmrc (handles comments)"
    );
}

/// P2-2: nvm.psm1 Initialize-NvmPath must use element comparison (-notcontains),
/// not substring match (-notlike which falsely matches "bin-old" etc.).
#[test]
fn p2_2_psm1_init_path_uses_element_comparison() {
    let nvm_psm1 = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("shell")
        .join("nvm.psm1");
    let content = fs::read_to_string(&nvm_psm1).expect("nvm.psm1 must exist");
    assert!(
        content.contains("-notcontains $NvmBin"),
        "nvm.psm1 Initialize-NvmPath must use -notcontains (element comparison)"
    );
}

/// P2-3: nvm.psm1 auto case must call Initialize-NvmPath after binary,
/// matching nvm.sh's _nvm_prepend_path call after auto.
#[test]
fn p2_3_psm1_auto_calls_init_path() {
    let nvm_psm1 = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("shell")
        .join("nvm.psm1");
    let content = fs::read_to_string(&nvm_psm1).expect("nvm.psm1 must exist");
    let auto_pos = content.find("'auto' {").expect("must have auto case");
    let auto_section = &content[auto_pos..];
    let auto_end = auto_section.find('}').unwrap_or(50);
    let auto_body = &auto_section[..auto_end + 1];
    assert!(
        auto_body.contains("Initialize-NvmPath"),
        "nvm.psm1 auto case must call Initialize-NvmPath"
    );
}

/// P2-4: nvm.psm1 upgrade/refresh must re-import the module so updated
/// function definitions take effect (matches nvm.sh's re-source behavior).
#[test]
fn p2_4_psm1_upgrade_reimports_module() {
    let nvm_psm1 = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("shell")
        .join("nvm.psm1");
    let content = fs::read_to_string(&nvm_psm1).expect("nvm.psm1 must exist");
    assert!(
        content.contains("Import-Module") && content.contains("Remove-Module"),
        "nvm.psm1 upgrade/refresh must re-import module (Remove+Import)"
    );
}

/// P2-8: nvm.psm1 must not have dead $script:OriginalPath variable
/// (was only used by the old Remove-NvmFromPath which now filters current PATH).
#[test]
fn p2_8_psm1_no_dead_original_path() {
    let nvm_psm1 = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("shell")
        .join("nvm.psm1");
    let content = fs::read_to_string(&nvm_psm1).expect("nvm.psm1 must exist");
    assert!(
        !content.contains("$script:OriginalPath"),
        "nvm.psm1 must not have dead $script:OriginalPath variable"
    );
}
