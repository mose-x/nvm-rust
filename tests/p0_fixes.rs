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

/// P0-3 regression: _nvm_prepend_path must prepend bin FIRST, then shims,
/// so the final PATH order is shims:bin:<rest> (shims take precedence).
/// If shims is prepended first and bin second, the order would be bin:shims
/// which is wrong — legacy bin/ binaries would shadow shims.
#[test]
fn p0_3_shims_prepend_order_is_shims_before_bin() {
    let shell_nvm_sh = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("shell")
        .join("nvm.sh");
    let content = fs::read_to_string(&shell_nvm_sh).expect("shell/nvm.sh must exist");

    // Find _nvm_prepend_path function body. Can't use find("}") because
    // ${NVM_RUST_BIN} contains a } character. Instead, find the } that
    // appears at the start of a line (the function's closing brace).
    let func_start = content.find("_nvm_prepend_path()").unwrap_or(0);
    let lines: Vec<&str> = content[func_start..].lines().collect();
    let func_line_count = lines
        .iter()
        .position(|line| line.trim_start().starts_with('}'))
        .unwrap_or(lines.len());
    let func_body: String = lines[..func_line_count].join("\n");

    let bin_pos = func_body.find("NVM_RUST_BIN").unwrap_or(usize::MAX);
    let shims_pos = func_body.find("NVM_RUST_SHIMS").unwrap_or(usize::MAX);
    assert!(
        bin_pos < shims_pos,
        "_nvm_prepend_path must prepend BIN first, then SHIMS (so shims ends up at front). \
         BIN at pos {}, SHIMS at pos {} — BIN should come first in the function body.",
        bin_pos,
        shims_pos
    );
}

/// P0-3 regression: deactivate and unload must strip BOTH NVM_RUST_SHIMS
/// and NVM_RUST_BIN from PATH. The implementation now uses _nvm_strip_path
/// which calls _nvm_path_remove for each entry — verify the function is called.
#[test]
fn p0_3_deactivate_and_unload_strip_shims_from_path() {
    let shell_nvm_sh = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("shell")
        .join("nvm.sh");
    let content = fs::read_to_string(&shell_nvm_sh).expect("shell/nvm.sh must exist");

    // Verify _nvm_strip_path removes NVM_RUST_SHIMS
    assert!(
        content.contains("_nvm_path_remove \"${NVM_RUST_SHIMS}\""),
        "_nvm_strip_path must remove NVM_RUST_SHIMS from PATH"
    );

    // Verify deactivate calls _nvm_strip_path
    let deactivate_start = content.find("deactivate)").unwrap_or(0);
    let deactivate_section = &content[deactivate_start..];
    let deactivate_end = deactivate_section
        .find(";;")
        .map(|i| deactivate_start + i)
        .unwrap_or(content.len());
    let deactivate_body = &content[deactivate_start..deactivate_end];
    assert!(
        deactivate_body.contains("_nvm_strip_path"),
        "deactivate must call _nvm_strip_path to remove nvm entries from PATH"
    );

    // Verify unload calls _nvm_strip_path
    let unload_start = content.find("unload)").unwrap_or(0);
    let unload_section = &content[unload_start..];
    let unload_end = unload_section
        .find(";;")
        .map(|i| unload_start + i)
        .unwrap_or(content.len());
    let unload_body = &content[unload_start..unload_end];
    assert!(
        unload_body.contains("_nvm_strip_path"),
        "unload must call _nvm_strip_path to remove nvm entries from PATH"
    );
}

/// install.sh must support --uninstall and --uninstall --self
#[test]
fn install_sh_supports_uninstall() {
    let install_sh = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("install.sh");
    let content = fs::read_to_string(&install_sh).expect("install.sh must exist");
    assert!(
        content.contains("--uninstall"),
        "install.sh must detect --uninstall flag"
    );
    assert!(
        content.contains("uninstall_self"),
        "install.sh must have uninstall_self function"
    );
    assert!(
        content.contains("uninstall_all"),
        "install.sh must have uninstall_all function"
    );
    assert!(
        content.contains("--self"),
        "install.sh must support --uninstall --self"
    );
    assert!(
        content.contains("clean_shell_config"),
        "install.sh must clean shell config during uninstall"
    );
    assert!(
        content.contains("[y/N]"),
        "install.sh uninstall must require y/N confirmation"
    );
}

/// install.ps1 must support -Uninstall and -Self
#[test]
fn install_ps1_supports_uninstall() {
    let install_ps1 = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("install.ps1");
    let content = fs::read_to_string(&install_ps1).expect("install.ps1 must exist");
    assert!(
        content.contains("Uninstall"),
        "install.ps1 must have -Uninstall parameter"
    );
    assert!(
        content.contains("Self"),
        "install.ps1 must have -Self parameter"
    );
    assert!(
        content.contains("Uninstall-Self") || content.contains("Uninstall_Self"),
        "install.ps1 must have Uninstall-Self function"
    );
    assert!(
        content.contains("Uninstall-All") || content.contains("Uninstall_All"),
        "install.ps1 must have Uninstall-All function"
    );
}

/// P1-4: install.sh clean_shell_config must use `grep -Ev` (extended regex)
/// instead of `grep -v` with `\|` alternation — BSD grep (macOS default) does
/// not support `\|` in BRE, so the filter would match nothing on macOS.
#[test]
fn install_sh_grep_uses_extended_regex() {
    let install_sh = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("install.sh");
    let content = fs::read_to_string(&install_sh).expect("install.sh must exist");
    assert!(
        content.contains("grep -Ev"),
        "install.sh must use grep -Ev (extended regex) for BSD grep compatibility"
    );
    assert!(
        !content.contains("grep -v \"nvm.rust\\|"),
        "install.sh must not use grep -v with \\| (broken on macOS BSD grep)"
    );
}

/// P1-5: install.sh must guard against empty BASH_SOURCE[0] when piped via
/// `curl | bash`. Without this, SCRIPT_DIR falls back to CWD, and a malicious
/// `nvm` binary in the current directory would be used instead of downloading.
#[test]
fn install_sh_guards_empty_bash_source() {
    let install_sh = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("install.sh");
    let content = fs::read_to_string(&install_sh).expect("install.sh must exist");
    assert!(
        content.contains("[ -n \"${BASH_SOURCE[0]:-}\" ]"),
        "install.sh must guard against empty BASH_SOURCE[0] (binary planting risk)"
    );
}

/// P0-3: install.ps1 must NOT define a custom `Write-Error` function that
/// shadows PowerShell's built-in cmdlet. The custom version used Write-Host
/// (console only) and didn't throw, breaking $ErrorActionPreference="Stop".
#[test]
fn install_ps1_no_write_error_shadow() {
    let install_ps1 = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("install.ps1");
    let content = fs::read_to_string(&install_ps1).expect("install.ps1 must exist");
    assert!(
        !content.contains("function Write-Error"),
        "install.ps1 must not define function Write-Error (shadows built-in cmdlet)"
    );
    assert!(
        content.contains("function Write-Err"),
        "install.ps1 must use Write-Err instead of Write-Error"
    );
}

/// P1-6: install.ps1 PATH manipulation must use element-by-element comparison
/// (-contains) instead of substring match (-notlike) to avoid false matches
/// like "shims-old" matching "*shims*".
#[test]
fn install_ps1_path_uses_element_comparison() {
    let install_ps1 = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("install.ps1");
    let content = fs::read_to_string(&install_ps1).expect("install.ps1 must exist");
    assert!(
        content.contains("-contains $shimsDir"),
        "install.ps1 must use -contains for PATH element comparison"
    );
    assert!(
        !content.contains("-notlike \"*$shimsDir*\""),
        "install.ps1 must not use -notlike substring match for PATH"
    );
}

/// P1-7: install.ps1 must warn before silently changing the PowerShell
/// execution policy, so users in compliance-restricted environments are informed.
#[test]
fn install_ps1_warns_before_exec_policy_change() {
    let install_ps1 = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("install.ps1");
    let content = fs::read_to_string(&install_ps1).expect("install.ps1 must exist");
    let policy_pos = content.find("Set-ExecutionPolicy");
    assert!(
        policy_pos.is_some(),
        "install.ps1 must have Set-ExecutionPolicy"
    );
    let before_policy = &content[..policy_pos.unwrap()];
    assert!(
        before_policy.contains("Write-Warn") || before_policy.rfind("Write-Warn").is_some(),
        "install.ps1 must warn before changing execution policy"
    );
}

/// P2-10: install.sh "already configured" check must use "# nvm-rs" marker
/// (not loose "nvm.sh" grep that matches comments about the old nvm-sh project).
#[test]
fn install_sh_already_configured_uses_precise_marker() {
    let install_sh = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("install.sh");
    let content = fs::read_to_string(&install_sh).expect("install.sh must exist");
    assert!(
        content.contains("grep -qF \"# nvm-rs\""),
        "install.sh must use '# nvm-rs' marker for already-configured check"
    );
}

/// P2-11/P1-2: install.sh uninstall functions must use the consistent $NVM_DIR
/// variable (derived from NVM_INSTALL_DIR at top of script) instead of
/// hardcoding $HOME/.nvm.rust.
#[test]
fn install_sh_uninstall_uses_nvm_dir() {
    let install_sh = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("install.sh");
    let content = fs::read_to_string(&install_sh).expect("install.sh must exist");
    // Verify NVM_DIR is defined at the top
    assert!(
        content.contains("NVM_DIR=\"$(dirname \"$INSTALL_DIR\")\""),
        "install.sh must define NVM_DIR derived from INSTALL_DIR"
    );
    // Both uninstall functions should use $NVM_DIR
    let uninstall_self_pos = content.find("uninstall_self()").unwrap_or(0);
    let uninstall_all_pos = content.find("uninstall_all()").unwrap_or(0);
    let self_section = &content[uninstall_self_pos..uninstall_all_pos];
    let all_section = &content[uninstall_all_pos..];
    assert!(
        self_section.contains("$NVM_DIR"),
        "install.sh uninstall_self must use $NVM_DIR"
    );
    assert!(
        all_section.contains("$NVM_DIR"),
        "install.sh uninstall_all must use $NVM_DIR"
    );
}

/// P2-13: install.ps1 must respect $env:NVM_DIR instead of hardcoding the path.
#[test]
fn install_ps1_respects_nvm_dir() {
    let install_ps1 = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("install.ps1");
    let content = fs::read_to_string(&install_ps1).expect("install.ps1 must exist");
    assert!(
        content.contains("$env:NVM_DIR"),
        "install.ps1 must respect $env:NVM_DIR"
    );
}

/// P2-15: install.ps1 shell script download must not use -ErrorAction SilentlyContinue
/// (which silently swallows download failures and prints false success).
/// Other uses (cleanup, file reads) are acceptable.
#[test]
fn install_ps1_no_silently_continue_on_download() {
    let install_ps1 = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("install.ps1");
    let content = fs::read_to_string(&install_ps1).expect("install.ps1 must exist");
    assert!(
        !content.contains("OutFile $shellDest -UseBasicParsing -ErrorAction SilentlyContinue"),
        "install.ps1 shell script download must not use -ErrorAction SilentlyContinue"
    );
}

/// P1-3: Windows shim script must have path traversal defense-in-depth
/// (findstr ".." guard), matching the Unix shim's case guard.
#[test]
fn windows_shim_has_path_traversal_guard() {
    let shim_rs = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/shim.rs");
    let content = fs::read_to_string(&shim_rs).expect("shim.rs must exist");
    assert!(
        content.contains("findstr /C:\"..\""),
        "Windows shim script must use findstr /C:\"..\" (literal match, not regex)"
    );
}

/// P3-2: install.sh grep pattern must escape dots (in ERE, . matches any char).
#[test]
fn install_sh_grep_escapes_dots() {
    let install_sh = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("install.sh");
    let content = fs::read_to_string(&install_sh).expect("install.sh must exist");
    assert!(
        content.contains("nvm\\.rust"),
        "install.sh grep pattern must escape dots (nvm\\.rust)"
    );
}

/// P1-4: install.sh clean_shell_config must remove the `# nvm-rs` marker
/// (contains dash, not dot) so reinstall doesn't skip PATH configuration.
#[test]
fn install_sh_clean_removes_nvm_rs_marker() {
    let install_sh = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("install.sh");
    let content = fs::read_to_string(&install_sh).expect("install.sh must exist");
    assert!(
        content.contains("nvm-rs|NVM_HOME"),
        "install.sh clean_shell_config must match 'nvm-rs' (dash) to remove the marker"
    );
}

/// P1-6: install.ps1 shim script must respect existing NVM_DIR env var
/// (use 'if not defined' instead of unconditional 'set').
#[test]
fn install_ps1_shim_respects_nvm_dir() {
    let install_ps1 = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("install.ps1");
    let content = fs::read_to_string(&install_ps1).expect("install.ps1 must exist");
    assert!(
        content.contains("if not defined NVM_DIR"),
        "install.ps1 shim script must respect existing NVM_DIR (if not defined)"
    );
}

/// Issue 1: scripts/devbuild.sh and scripts/devbuild.bat must exist for auto-copy.
#[test]
fn devbuild_scripts_exist() {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let sh = manifest.join("scripts/devbuild.sh");
    let bat = manifest.join("scripts/devbuild.bat");
    assert!(
        sh.exists(),
        "scripts/devbuild.sh must exist (Unix auto-copy)"
    );
    assert!(
        bat.exists(),
        "scripts/devbuild.bat must exist (Windows auto-copy)"
    );
    // Verify Unix script copies nvm binary
    let sh_content = fs::read_to_string(&sh).expect("devbuild.sh must be readable");
    assert!(
        sh_content.contains("cp target/debug/nvm"),
        "devbuild.sh must copy target/debug/nvm to ~/.nvm.rust/bin/"
    );
    // Verify Windows script copies nvm.exe
    let bat_content = fs::read_to_string(&bat).expect("devbuild.bat must be readable");
    assert!(
        bat_content.contains("target\\debug\\nvm.exe"),
        "devbuild.bat must copy target/debug/nvm.exe to ~/.nvm.rust/bin/"
    );
}

/// Issue 3: package_upgrade.rs must set COREPACK_ENABLE_DOWNLOAD_PROMPT=0
/// to suppress the interactive "Do you want to continue?" prompt.
#[test]
fn package_upgrade_sets_download_prompt_off() {
    let src =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/commands/package_upgrade.rs");
    let content = fs::read_to_string(&src).expect("package_upgrade.rs must exist");
    assert!(
        content.contains("COREPACK_ENABLE_DOWNLOAD_PROMPT"),
        "package_upgrade.rs must set COREPACK_ENABLE_DOWNLOAD_PROMPT"
    );
}

/// Issue 4: info.rs must have probe_tool_version for corepack shim detection.
#[test]
fn info_has_probe_tool_version() {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/commands/info.rs");
    let content = fs::read_to_string(&src).expect("info.rs must exist");
    assert!(
        content.contains("fn probe_tool_version"),
        "info.rs must have probe_tool_version for corepack shim fallback"
    );
}

/// Issue 5: shell_config.rs source line must have [ -f ] guard.
#[test]
fn shell_config_source_has_file_guard() {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/shell_config.rs");
    let content = fs::read_to_string(&src).expect("shell_config.rs must exist");
    assert!(
        content.contains("[ -f "),
        "shell_config.rs source line must have [ -f ] guard"
    );
}

/// Issue 5: refresh.rs must NOT skip download when nvm.sh is missing.
#[test]
fn refresh_does_not_skip_missing_nvm_sh() {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/commands/refresh.rs");
    let content = fs::read_to_string(&src).expect("refresh.rs must exist");
    assert!(
        !content.contains("if !nvm_sh_path.exists()"),
        "refresh.rs must not skip when nvm.sh is missing (should download/create)"
    );
}

/// Issue 6: corepack.rs must verify shim content contains "corepack"
/// before reporting "already enabled" (prevents false positive on npm shims).
#[test]
fn corepack_verifies_shim_content() {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/corepack.rs");
    let content = fs::read_to_string(&src).expect("corepack.rs must exist");
    assert!(
        content.contains("is_corepack_shim"),
        "corepack.rs must verify shim content contains 'corepack' (not just file existence)"
    );
}

/// Issue 5 (init.rs): init.rs source line must have [ -f ] guard
/// (matching shell_config.rs — prevents .zshrc error when nvm.sh missing).
#[test]
fn init_rs_source_has_file_guard() {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/commands/init.rs");
    let content = fs::read_to_string(&src).expect("init.rs must exist");
    assert!(
        content.contains("[ -f "),
        "init.rs source line must have [ -f ] guard (matching shell_config.rs)"
    );
}

/// Issue 5 (init.rs migration): init.rs must auto-fix existing unguarded
/// source lines when user runs `nvm init` after upgrading.
#[test]
fn init_rs_has_source_migration_logic() {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/commands/init.rs");
    let content = fs::read_to_string(&src).expect("init.rs must exist");
    assert!(
        content.contains("unguarded") || content.contains("pre-fix"),
        "init.rs must have migration logic to detect and fix unguarded source lines"
    );
}

/// Issue 5 (refresh): refresh.rs must also auto-fix unguarded source lines
/// so users who upgrade and run `nvm refresh` get the repair without `nvm init`.
#[test]
fn refresh_has_source_guard_fix() {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/commands/refresh.rs");
    let content = fs::read_to_string(&src).expect("refresh.rs must exist");
    assert!(
        content.contains("fn fix_rc_source_guard"),
        "refresh.rs must have fix_rc_source_guard function for auto-repair"
    );
}

/// NVM_HOME usage: shell_config.rs must use $NVM_HOME in PATH and source lines.
#[test]
fn shell_config_uses_nvm_home_variable() {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/shell_config.rs");
    let content = fs::read_to_string(&src).expect("shell_config.rs must exist");
    assert!(
        content.contains("$NVM_HOME/shims"),
        "shell_config.rs must use $NVM_HOME in PATH export"
    );
    assert!(
        content.contains("$NVM_HOME/bin/nvm.sh"),
        "shell_config.rs must use $NVM_HOME in source line"
    );
}

/// NVM_HOME usage: init.rs must also use $NVM_HOME (matching shell_config.rs).
#[test]
fn init_rs_uses_nvm_home_variable() {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/commands/init.rs");
    let content = fs::read_to_string(&src).expect("init.rs must exist");
    assert!(
        content.contains("$NVM_HOME/shims"),
        "init.rs must use $NVM_HOME in PATH export"
    );
    assert!(
        content.contains("$NVM_HOME/bin/nvm.sh"),
        "init.rs must use $NVM_HOME in source line"
    );
}

/// Build scripts must auto-copy binary after build.
#[test]
fn build_scripts_have_auto_copy() {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    for script in &["scripts/build-linux.sh", "scripts/build-macos.sh"] {
        let path = manifest.join(script);
        let content = fs::read_to_string(&path).unwrap_or_else(|_| panic!("{} must exist", script));
        assert!(
            content.contains("cp ") && content.contains(".nvm.rust/bin"),
            "{} must auto-copy binary to ~/.nvm.rust/bin/",
            script
        );
    }
    let bat = manifest.join("scripts/build-windows.bat");
    let bat_content = fs::read_to_string(&bat).expect("build-windows.bat must exist");
    assert!(
        bat_content.contains("copy /Y") && bat_content.contains(".nvm.rust\\bin"),
        "build-windows.bat must auto-copy binary to .nvm.rust\\bin\\"
    );
}

/// EDR-safe layout: install.sh must try /usr/local/bin first (system path,
/// EDR-safe), then create a symlink at ~/.nvm.rust/bin/nvm → /usr/local/bin/nvm.
/// Fallback to old behavior (real file in user dir) with warning.
#[test]
fn install_sh_installs_to_system_path_first() {
    let install_sh = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("install.sh");
    let content = fs::read_to_string(&install_sh).expect("install.sh must exist");
    // Must try /usr/local/bin first
    assert!(
        content.contains("/usr/local/bin") && content.contains("[ -w \"/usr/local/bin\""),
        "install.sh must check if /usr/local/bin is writable for EDR-safe install"
    );
    // Must create symlink from user dir → system path
    assert!(
        content.contains("ln -sf \"$BIN_LINK\" \"${INSTALL_DIR}/${BINARY_NAME}\""),
        "install.sh must create symlink from user dir to system path (EDR-safe layout)"
    );
    // Must have sudo fallback for system path
    assert!(
        content.contains("sudo cp -f"),
        "install.sh must use sudo cp as fallback for system path install"
    );
    // Must have EDR warning for fallback to user dir
    assert!(
        content.contains("EDR may block"),
        "install.sh must warn about EDR risk when falling back to user dir"
    );
    // Uninstall must use sudo rm for system-path binary
    assert!(
        content.contains("sudo rm -f \"$BIN_LINK\""),
        "install.sh uninstall must use sudo rm for system-path binary (may be root-owned)"
    );
}

/// EDR-safe layout: binary_swap.rs must have sudo fallback logic for
/// system paths that are not user-writable.
#[test]
fn binary_swap_has_sudo_fallback() {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/commands/binary_swap.rs");
    let content = fs::read_to_string(&src).expect("binary_swap.rs must exist");
    // Must check if dir is writable
    assert!(
        content.contains("is_dir_writable"),
        "binary_swap.rs must have is_dir_writable function to detect system paths"
    );
    // Must have sudo cp fallback on Unix
    assert!(
        content.contains("sudo") && content.contains("cp"),
        "binary_swap.rs must have sudo cp fallback for non-writable system paths"
    );
    // Marker must be in user bin dir (always user-writable).
    // cargo fmt may split the chained .join() across lines, so check
    // for the components separately.
    assert!(
        content.contains("get_nvm_dir()")
            && content.contains(".join(\"bin\")")
            && content.contains(".join(\".swap-pending\")"),
        "binary_swap.rs must write .swap-pending marker to user bin dir (always user-writable)"
    );
    // check_swap_recovery must look in user bin dir
    assert!(
        content.contains("user_bin_dir"),
        "binary_swap.rs check_swap_recovery must look in user bin dir for marker and recovery files"
    );
}

/// EDR-safe layout: refresh.rs must have migrate_binary_to_system_path
/// function that auto-migrates old-layout binaries to the new EDR-safe layout.
#[test]
fn refresh_has_migrate_binary_to_system_path() {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/commands/refresh.rs");
    let content = fs::read_to_string(&src).expect("refresh.rs must exist");
    // Must have the migration function
    assert!(
        content.contains("fn migrate_binary_to_system_path"),
        "refresh.rs must have migrate_binary_to_system_path function"
    );
    // Must check if binary is a symlink (already migrated) using symlink_metadata
    assert!(
        content.contains("symlink_metadata"),
        "refresh.rs must use symlink_metadata to check if binary is already a symlink"
    );
    // Must try sudo cp on Unix
    assert!(
        content.contains("sudo") && content.contains("/usr/local/bin/nvm"),
        "refresh.rs must try sudo cp to /usr/local/bin/nvm for migration"
    );
    // Must create symlink after migration
    assert!(
        content.contains("std::os::unix::fs::symlink"),
        "refresh.rs must create symlink from user dir to system path after migration"
    );
    // Must call it in refresh()
    assert!(
        content.contains("migrate_binary_to_system_path(&nvm_dir)"),
        "refresh.rs must call migrate_binary_to_system_path in refresh()"
    );
}

/// EDR-safe layout: doctor.rs must check if binary is in user dir (EDR risk)
/// and suggest running 'nvm refresh' to migrate.
#[test]
fn doctor_has_edr_risk_check() {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/commands/doctor.rs");
    let content = fs::read_to_string(&src).expect("doctor.rs must exist");
    // Must check if path contains .nvm.rust (user path = EDR risk)
    assert!(
        content.contains(".nvm.rust"),
        "doctor.rs must check if binary path contains .nvm.rust (EDR risk)"
    );
    // Must print EDR risk warning
    assert!(
        content.contains("doctor_binary_edr_risk"),
        "doctor.rs must use doctor_binary_edr_risk i18n key for EDR risk warning"
    );
    // Must suggest nvm refresh
    assert!(
        content.contains("nvm refresh") || content.contains("doctor_binary_edr_risk"),
        "doctor.rs must suggest running nvm refresh to migrate"
    );
}
