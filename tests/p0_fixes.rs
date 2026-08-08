use std::fs;

/// P0-1: Root nvm.sh must not exist — it conflicts with shell/nvm.sh.
/// The root nvm.sh was a pre-shim-era script with different deactivate/unload/use
/// semantics. Users could source the wrong one and get broken behavior.
#[test]
fn p0_1_root_nvm_sh_does_not_exist() {
    let root_nvm_sh = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("nvm.sh");
    assert!(
        !root_nvm_sh.exists(),
        "Root nvm.sh must not exist — it conflicts with shell/nvm.sh. \
         Delete it; only shell/nvm.sh should be shipped."
    );
}

/// P0-2: install.sh must use $path_line (not the undefined $source_line)
/// in the fresh-install branch. Without this fix, a brand-new machine gets
/// an empty PATH export line, and nvm is not on PATH after install.
#[test]
fn p0_2_install_sh_uses_path_line_not_source_line() {
    let install_sh = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("install.sh");
    let content = fs::read_to_string(&install_sh).expect("install.sh must exist");

    // The fresh-install branch should use $path_line, not $source_line.
    // Check that $source_line is NOT used in the fresh-install context.
    // The variable $path_line is defined earlier in the same function scope.
    assert!(
        content.contains("echo \"$path_line\" >> \"$shell_profile\""),
        "install.sh fresh-install branch must use $path_line, not $source_line"
    );
    assert!(
        !content.contains("echo \"$source_line\" >> \"$shell_profile\""),
        "install.sh must NOT use $source_line — it's undefined in the main() scope"
    );
}

/// P0-3: shell/nvm.sh must add the shims directory to PATH.
/// Without shims in PATH, the shim architecture doesn't work through this script
/// — node/npm/npx won't resolve via the `current` file.
#[test]
fn p0_3_shell_nvm_sh_adds_shims_to_path() {
    let shell_nvm_sh = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("shell")
        .join("nvm.sh");
    let content = fs::read_to_string(&shell_nvm_sh).expect("shell/nvm.sh must exist");

    // Must define the shims directory variable.
    assert!(
        content.contains("NVM_RUST_SHIMS"),
        "shell/nvm.sh must define NVM_RUST_SHIMS variable"
    );
    // Must prepend shims to PATH.
    assert!(
        content.contains("${NVM_RUST_SHIMS}:${PATH}"),
        "shell/nvm.sh must prepend NVM_RUST_SHIMS to PATH in _nvm_prepend_path"
    );
}

/// P0-1 (companion): shell/nvm.sh must still exist and be functional.
#[test]
fn p0_1_shell_nvm_sh_exists_and_is_valid() {
    let shell_nvm_sh = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("shell")
        .join("nvm.sh");
    assert!(shell_nvm_sh.exists(), "shell/nvm.sh must exist");

    let content = fs::read_to_string(&shell_nvm_sh).expect("shell/nvm.sh must be readable");
    assert!(
        content.contains("NVM_RUST_DIR") && content.contains("_nvm_prepend_path"),
        "shell/nvm.sh must contain core variable and function definitions"
    );
}

/// P0-2 (companion): install.sh must define $path_line before using it.
#[test]
fn p0_2_install_sh_defines_path_line() {
    let install_sh = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("install.sh");
    let content = fs::read_to_string(&install_sh).expect("install.sh must exist");

    assert!(
        content.contains("local path_line="),
        "install.sh must define $path_line before using it"
    );
    assert!(
        content.contains("local fish_path_line="),
        "install.sh must define $fish_path_line for fish shells"
    );
}
