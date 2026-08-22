//! Windows PowerShell integration repair.
//!
//! Pre-2.4.0 installers injected `Import-Module ...shell\nvm.psm1` into the
//! PowerShell profile. That module exported an `nvm` function which shadowed
//! `nvm.exe` (PowerShell resolves functions before external commands), so:
//!
//! - bare `nvm` printed the module's hardcoded help instead of running the
//!   binary;
//! - dash flags (`nvm -v`, `--version`, `-h`) were bound as PowerShell
//!   parameters instead of reaching the binary;
//! - exit codes were dropped;
//! - the help text mojibake'd on GBK locales (UTF-8 without BOM).
//!
//! The Full Shim architecture (persistent user PATH + active junction) makes
//! the module unnecessary for normal use, so the repair:
//!
//! 1. rewrites `%NVM_DIR%\shell\nvm.psm1` with the fixed pass-through module
//!    (for users who opt into the cd auto-switch hook);
//! 2. strips the legacy `Import-Module` line from PowerShell profiles,
//!    backing each profile up to `<profile>.bak-nvm` first.
//!
//! Wired into `nvm refresh` (which `nvm upgrade` execs automatically, so
//! updating self-heals) and `nvm doctor` (detection + `--fix`).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The fixed module source, embedded at compile time. Written with a UTF-8
/// BOM so Windows PowerShell 5.1 decodes it correctly under any locale.
pub const PSM1_CONTENT: &str = include_str!("../shell/nvm.psm1");

const UTF8_BOM: &[u8] = b"\xEF\xBB\xBF";
const BACKUP_SUFFIX: &str = ".bak-nvm";

/// PowerShell profile candidates: PowerShell 7 first, then Windows
/// PowerShell 5.1. Both can coexist (different hosts, different profiles).
pub fn ps_profile_candidates() -> Vec<PathBuf> {
    let home = crate::system::get_home_dir();
    vec![
        Path::new(&home).join("Documents/PowerShell/Microsoft.PowerShell_profile.ps1"),
        Path::new(&home).join("Documents/WindowsPowerShell/Microsoft.PowerShell_profile.ps1"),
    ]
}

/// True if the line is a legacy `Import-Module ...nvm.psm1` injection.
fn is_import_line(line: &str) -> bool {
    let t = line.trim();
    t.starts_with("Import-Module") && t.contains("nvm.psm1")
}

/// True if the content contains the legacy installer injection: an
/// `Import-Module ...nvm.psm1` line directly preceded by the `# nvm-rs`
/// marker comment. Matches `strip_module_import` semantics exactly, so
/// detection (doctor) and repair can never disagree.
pub fn has_legacy_injection(content: &str) -> bool {
    let lines: Vec<&str> = content.lines().collect();
    lines
        .iter()
        .enumerate()
        .any(|(i, line)| i > 0 && lines[i - 1].trim() == "# nvm-rs" && is_import_line(line))
}

/// Strip the legacy installer injection from one profile: an
/// `Import-Module ...nvm.psm1` line is removed ONLY when directly preceded
/// by the `# nvm-rs` marker comment the old installer wrote. Bare imports
/// (added by the user to opt into the cd auto-switch hook) are kept — the
/// repaired module on disk is a harmless pass-through. Creates
/// `<profile>.bak-nvm` before the first modification. Returns true if the
/// file changed. Preserves original line endings.
pub fn strip_module_import(profile: &Path) -> io::Result<bool> {
    let content = match fs::read_to_string(profile) {
        Ok(c) => c,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e),
    };
    if !content.contains("nvm.psm1") {
        return Ok(false);
    }

    let lines: Vec<&str> = content.split_inclusive('\n').collect();
    let mut drop = vec![false; lines.len()];
    for (i, line) in lines.iter().enumerate() {
        if i > 0 && lines[i - 1].trim() == "# nvm-rs" && is_import_line(line) {
            drop[i] = true;
            drop[i - 1] = true;
        }
    }
    if !drop.iter().any(|&d| d) {
        return Ok(false);
    }

    // Backup before touching the user's profile. Overwrites an earlier
    // `.bak-nvm` — it was created by this same repair.
    let backup = profile.with_extension(format!(
        "{}{}",
        profile
            .extension()
            .map(|e| format!("{}.", e.to_string_lossy()))
            .unwrap_or_default(),
        BACKUP_SUFFIX.trim_start_matches('.')
    ));
    fs::copy(profile, &backup)?;

    let cleaned: String = lines
        .iter()
        .zip(drop.iter())
        .filter(|(_, &d)| !d)
        .map(|(l, _)| *l)
        .collect();
    fs::write(profile, cleaned)?;
    Ok(true)
}

/// Rewrite the on-disk module with the fixed content, prefixed with a UTF-8
/// BOM so Windows PowerShell 5.1 (which assumes ANSI for BOM-less scripts)
/// decodes it correctly on every locale.
pub fn write_psm1(path: &Path) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut bytes = Vec::with_capacity(UTF8_BOM.len() + PSM1_CONTENT.len());
    bytes.extend_from_slice(UTF8_BOM);
    bytes.extend_from_slice(PSM1_CONTENT.as_bytes());
    fs::write(path, &bytes)
}

/// True if the on-disk module matches the embedded fixed content (BOM
/// optional). Missing file counts as stale so callers can (re)create it.
pub fn psm1_is_current(path: &Path) -> bool {
    match fs::read(path) {
        Ok(bytes) => {
            let body = bytes.strip_prefix(UTF8_BOM).unwrap_or(&bytes);
            body == PSM1_CONTENT.as_bytes()
        }
        Err(_) => false,
    }
}

/// Outcome of a full repair run.
#[derive(Debug, Default)]
pub struct RepairReport {
    pub psm1_written: bool,
    pub profiles_cleaned: usize,
}

/// Full repair: refresh the module file + strip legacy profile injections.
pub fn repair(nvm_dir: &Path) -> io::Result<RepairReport> {
    let mut report = RepairReport::default();

    let psm1_path = nvm_dir.join("shell").join("nvm.psm1");
    if !psm1_is_current(&psm1_path) {
        write_psm1(&psm1_path)?;
        report.psm1_written = true;
    }

    for profile in ps_profile_candidates() {
        if strip_module_import(&profile)? {
            report.profiles_cleaned += 1;
        }
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn psm1_content_is_ascii_and_passthrough() {
        assert!(
            PSM1_CONTENT.is_ascii(),
            "nvm.psm1 must stay ASCII-only so PS 5.1 cannot mojibake it"
        );
        assert!(
            PSM1_CONTENT.contains("& $NvmExe @args"),
            "nvm function must be a pure pass-through"
        );
        assert!(
            !PSM1_CONTENT.contains("ValidateSet"),
            "command whitelist breaks -v/--version and rots with new subcommands"
        );
        assert!(
            !PSM1_CONTENT.contains("Show-NvmHelp"),
            "hardcoded help shadowed the binary's real help"
        );
        assert!(
            PSM1_CONTENT.contains("Microsoft.PowerShell.Management\\Set-Location"),
            "cd hook must delegate to the original Set-Location"
        );
    }

    #[test]
    fn write_psm1_prepends_bom() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shell").join("nvm.psm1");
        write_psm1(&path).unwrap();
        let bytes = fs::read(&path).unwrap();
        assert!(
            bytes.starts_with(UTF8_BOM),
            "written file must start with BOM"
        );
        assert_eq!(&bytes[UTF8_BOM.len()..], PSM1_CONTENT.as_bytes());
        assert!(psm1_is_current(&path));
    }

    #[test]
    fn psm1_is_current_tolerates_missing_bom() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nvm.psm1");
        fs::write(&path, PSM1_CONTENT.as_bytes()).unwrap();
        assert!(
            psm1_is_current(&path),
            "BOM-less but correct content is current"
        );
        fs::write(&path, "old content").unwrap();
        assert!(!psm1_is_current(&path));
        assert!(!psm1_is_current(&dir.path().join("missing.psm1")));
    }

    #[test]
    fn strip_removes_import_and_marker_with_backup() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("profile.ps1");
        let original = "# user stuff\r\n\r\n# nvm-rs\r\nImport-Module \"C:\\Users\\u\\.nvm.rust\\shell\\nvm.psm1\"\r\nmore stuff\r\n";
        fs::write(&profile, original).unwrap();

        assert!(strip_module_import(&profile).unwrap());

        let cleaned = fs::read_to_string(&profile).unwrap();
        assert!(!cleaned.contains("nvm.psm1"));
        assert!(!cleaned.contains("# nvm-rs"));
        assert!(cleaned.contains("# user stuff"));
        assert!(cleaned.contains("more stuff"));

        let backup = dir.path().join("profile.ps1.bak-nvm");
        assert_eq!(fs::read_to_string(&backup).unwrap(), original);
    }

    #[test]
    fn strip_is_idempotent_and_skips_unrelated() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("profile.ps1");
        fs::write(&profile, "# nvm-rs\r\nImport-Module \"x\\nvm.psm1\"\r\n").unwrap();
        assert!(strip_module_import(&profile).unwrap());
        assert!(
            !strip_module_import(&profile).unwrap(),
            "second run is a no-op"
        );

        let unrelated = dir.path().join("other.ps1");
        fs::write(&unrelated, "Import-Module SomeOtherModule\r\n").unwrap();
        assert!(!strip_module_import(&unrelated).unwrap());

        assert!(!strip_module_import(&dir.path().join("missing.ps1")).unwrap());
    }

    #[test]
    fn strip_keeps_user_written_import_without_marker() {
        // A bare import (user opted into the cd hook manually) must survive:
        // only the installer's marker+import pair is legacy.
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("profile.ps1");
        let content =
            "Import-Module \"$env:USERPROFILE\\.nvm.rust\\shell\\nvm.psm1\"\n$env:FOO = 1\n";
        fs::write(&profile, content).unwrap();
        assert!(!strip_module_import(&profile).unwrap());
        assert_eq!(fs::read_to_string(&profile).unwrap(), content);
        assert!(!has_legacy_injection(content));
    }

    #[test]
    fn has_legacy_injection_detects_marker_pair_only() {
        assert!(has_legacy_injection(
            "# nvm-rs\nImport-Module \"C:\\x\\nvm.psm1\"\n"
        ));
        assert!(has_legacy_injection(
            "other\r\n# nvm-rs\r\nImport-Module 'nvm.psm1'\r\n"
        ));
        assert!(!has_legacy_injection("Import-Module \"nvm.psm1\"\n"));
        assert!(!has_legacy_injection("# nvm-rs\n$env:X = 1\n"));
    }

    #[test]
    fn repair_writes_module_and_cleans_profiles() {
        let dir = tempfile::tempdir().unwrap();
        // repair() reads real home for profile candidates; here we only
        // assert the module half, which is NVM_DIR-relative.
        let report = repair(dir.path()).unwrap();
        assert!(report.psm1_written);
        let psm1 = dir.path().join("shell").join("nvm.psm1");
        assert!(psm1_is_current(&psm1));
        // Second run: nothing to do.
        let report2 = repair(dir.path()).unwrap();
        assert!(!report2.psm1_written);
    }
}
