/// Tests for the root/sudo detection feature.
///
/// On Unix, nvm must refuse to run as root (euid==0) unless
/// NVM_ALLOW_ROOT=1 is set. This prevents config files from being
/// locked to root ownership.
use std::fs;

#[test]
fn test_main_rs_has_root_check_before_os_check() {
    let main_rs = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/main.rs");
    let content = fs::read_to_string(&main_rs).expect("main.rs must exist");

    // The root check must come BEFORE os_check()
    let root_check_pos = content.find("geteuid").unwrap_or(usize::MAX);
    let os_check_pos = content.find("os_check").unwrap_or(usize::MAX);
    assert!(
        root_check_pos < os_check_pos,
        "root check (geteuid) must appear before os_check in main.rs"
    );
}

#[test]
fn test_main_rs_checks_euid_zero() {
    let main_rs = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/main.rs");
    let content = fs::read_to_string(&main_rs).expect("main.rs must exist");

    assert!(
        content.contains("euid == 0"),
        "main.rs must check euid == 0 for root detection"
    );
}

#[test]
fn test_main_rs_has_allow_root_escape_hatch() {
    let main_rs = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/main.rs");
    let content = fs::read_to_string(&main_rs).expect("main.rs must exist");

    assert!(
        content.contains("NVM_ALLOW_ROOT"),
        "main.rs must check NVM_ALLOW_ROOT env var as escape hatch for Docker/CI"
    );
}

#[test]
fn test_main_rs_exits_on_root() {
    let main_rs = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/main.rs");
    let content = fs::read_to_string(&main_rs).expect("main.rs must exist");

    // Can't use find("}") to delimit the block because format strings
    // like "{} {}" contain } characters. Just check the whole file has
    // process::exit(1) and it's after the euid check.
    let euid_pos = content.find("euid == 0").unwrap_or(0);
    let after_euid = &content[euid_pos..];
    assert!(
        after_euid.contains("process::exit(1)"),
        "main.rs must exit(1) when running as root without NVM_ALLOW_ROOT"
    );
}

#[test]
fn test_main_rs_root_check_is_unix_only() {
    let main_rs = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/main.rs");
    let content = fs::read_to_string(&main_rs).expect("main.rs must exist");

    // The root check must be gated with #[cfg(unix)]
    let geteuid_pos = content.find("geteuid").unwrap_or(0);
    let before = &content[..geteuid_pos];
    assert!(
        before.rfind("#[cfg(unix)]").is_some(),
        "root check must be gated with #[cfg(unix)] — not needed on Windows"
    );
}

#[test]
fn test_locale_has_root_keys() {
    let en = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("locales/en.toml");
    let cn = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("locales/cn.toml");

    let en_content = fs::read_to_string(&en).expect("en.toml must exist");
    let cn_content = fs::read_to_string(&cn).expect("cn.toml must exist");

    for key in &["root_not_supported", "root_hint", "root_force_hint"] {
        assert!(en_content.contains(key), "en.toml must have key: {}", key);
        assert!(cn_content.contains(key), "cn.toml must have key: {}", key);
    }
}
