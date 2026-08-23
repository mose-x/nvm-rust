//! P0 regression tests: Windows shim dispatch must find executables at the
//! version dir ROOT (Windows Node archives have node.exe/npm.cmd there — no
//! bin/ subdir). Before the fix, the generated `shims\node.cmd` dispatcher
//! only looked in `%NVM_DIR%\%CURRENT%\bin\`, so every `node`/`npm` call
//! through PATH failed with "nvm: node not found" while the `active`
//! junction sat unused behind it.

mod common;

/// The installer ships its own copy of the dispatcher (written to
/// `shims\*.cmd` on install); it must carry the same root-first fix as the
/// binary-embedded one in shim.rs.
#[test]
fn install_ps1_shim_resolves_root_before_bin() {
    let content = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("install.ps1"),
    )
    .expect("install.ps1 must exist");

    assert!(
        content.contains(r#"if exist "%NVM_DIR%\%CURRENT%\%CMD%.exe""#),
        "install.ps1 shim must check the version dir root (Windows node layout)"
    );
    assert!(
        content.contains(r#"if exist "%NVM_DIR%\%CURRENT%\bin\%CMD%.exe""#),
        "install.ps1 shim must keep the bin\\ fallback"
    );
}

/// End-to-end: generate real shims via `nvm refresh`, then dispatch through
/// `shims\node.cmd` against a version dir whose executable lives at the ROOT
/// (the actual Windows layout). Before the fix this exited 1 with
/// "nvm: node not found".
#[cfg(windows)]
#[test]
fn windows_shim_dispatches_root_layout_executables() {
    use std::process::Command;

    let (dir, nvm_dir) = common::isolated_nvm_dir();
    let nvm_dir_path = dir.path();

    // Fake installed version: executable at the version dir root, exactly
    // like the official Windows node archive. The fake "node" is the nvm
    // binary itself, so a successful dispatch prints "nvm <version>".
    let version_dir = nvm_dir_path.join("v20.0.0");
    std::fs::create_dir_all(&version_dir).expect("create version dir");
    std::fs::copy(common::nvm_bin(), version_dir.join("node.exe")).expect("copy fake node");
    // Real installs also have bin\ (nvm.exe lives there; refresh writes
    // bin\nvm.sh into it).
    let bin_dir = nvm_dir_path.join("bin");
    std::fs::create_dir_all(&bin_dir).expect("create bin dir");
    std::fs::copy(common::nvm_bin(), bin_dir.join("nvm.exe")).expect("copy nvm to bin");
    std::fs::write(nvm_dir_path.join("current"), "v20.0.0").expect("write current");

    // Generate the shims through the real binary.
    let refresh = Command::new(common::nvm_bin())
        .arg("refresh")
        .env("NVM_DIR", &nvm_dir)
        .env("HOME", nvm_dir_path)
        .env("USERPROFILE", nvm_dir_path)
        .output()
        .expect("run nvm refresh");
    assert!(
        refresh.status.success(),
        "refresh failed: {}",
        String::from_utf8_lossy(&refresh.stderr)
    );

    let shim = nvm_dir_path.join("shims").join("node.cmd");
    assert!(shim.exists(), "refresh must create shims\\node.cmd");

    // Dispatch through the shim. NVM_DIR is passed explicitly: the script
    // must respect it (regression guard for the clobber bug).
    let out = Command::new("cmd")
        .args(["/C", shim.to_str().unwrap(), "--version"])
        .env("NVM_DIR", &nvm_dir)
        .env("USERPROFILE", nvm_dir_path)
        .output()
        .expect("run shim");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "dispatch failed (exit {:?}): {}",
        out.status.code(),
        stdout
    );
    assert!(
        stdout.contains("nvm"),
        "the fake node (= nvm binary) must have been dispatched: {stdout}"
    );
}
