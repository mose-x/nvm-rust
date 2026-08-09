use std::fs;

/// P1-1: unload() must warn on shim removal errors, not silently swallow them.
#[test]
fn p1_1_unload_warns_on_shim_removal_error() {
    let info_rs = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/commands/info.rs");
    let content = fs::read_to_string(&info_rs).expect("info.rs must exist");

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
    let info_rs = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/commands/info.rs");
    let content = fs::read_to_string(&info_rs).expect("info.rs must exist");

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
        content.contains("set -q NVM_DIR"),
        "nvm.fish must check if NVM_DIR is set (set -q NVM_DIR)"
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

/// P1-6: upgrade.rs must have swap_binary function (used by rollback).
#[test]
fn p1_6_upgrade_has_swap_binary() {
    let upgrade_rs =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/commands/upgrade.rs");
    let content = fs::read_to_string(&upgrade_rs).expect("upgrade.rs must exist");

    assert!(
        content.contains("fn swap_binary"),
        "upgrade.rs must have swap_binary function for rollback"
    );
    assert!(
        content.contains("BAK_SUFFIX"),
        "upgrade.rs must have BAK_SUFFIX constant for backup file naming"
    );
}
