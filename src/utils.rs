use std::collections::BTreeMap;
use std::cmp::Ordering;
use std::fs;
use std::path::Path;

use crate::system::get_nvm_dir;

/// Compare two version strings semantically (major.minor.patch), returning
/// `Greater` if `a` is newer than `b`. Handles both Node.js (`v20.11.0`) and
/// io.js (`iojs-v3.3.1`, `io.js-v2.5.0`) forms.
///
/// This MUST be used instead of `String::cmp` / `Vec::sort()` when picking the
/// "latest" of a set of installed versions: alphabetical sort puts `v20.5.0`
/// after `v20.20.2` (because '5' > '2' as chars), which is the wrong answer.
pub fn compare_semver(a: &str, b: &str) -> Ordering {
    let parse_v = |v: &str| -> (bool, u32, u32, u32) {
        let is_iojs = v.starts_with("iojs-") || v.starts_with("io.js-");
        let s = v
            .trim_start_matches("iojs-v")
            .trim_start_matches("io.js-v")
            .trim_start_matches("iojs-")
            .trim_start_matches("io.js-")
            .trim_start_matches('v');
        let parts: Vec<&str> = s.split('.').collect();
        let maj = parts.first().and_then(|s| s.parse().ok()).unwrap_or(0);
        let min = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
        let pat = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
        (is_iojs, maj, min, pat)
    };
    let (ai, amj, ami, apa) = parse_v(a);
    let (bi, bmj, bmi, bpa) = parse_v(b);
    // Sort by (major, minor, patch) numerically, then break ties by treating
    // io.js as newer than Node.js for the same version (matches the legacy
    // behavior of `compare_versions` in commands.rs).
    (amj, ami, apa, ai).cmp(&(bmj, bmi, bpa, bi))
}

/// Check if a version string is an io.js version (prefixes "iojs-" or "io.js-v")
pub fn is_iojs_version(version: &str) -> bool {
    version.starts_with("iojs-v") || version.starts_with("io.js-v") ||
    version.starts_with("iojs-") || version.starts_with("io.js-")
}

/// Normalize an io.js version name to canonical "iojs-vX.Y.Z"
pub fn normalize_iojs_version(version: &str) -> String {
    let v = version.trim_start_matches("io.js-").trim_start_matches("iojs-").trim_start_matches('v');
    format!("iojs-v{}", v)
}

/// Extract the version number from an io.js version (returns "X.Y.Z")
pub fn iojs_version_number(version: &str) -> Option<String> {
    if is_iojs_version(version) {
        let v = version.trim_start_matches("io.js-").trim_start_matches("iojs-").trim_start_matches('v');
        if v.matches('.').count() >= 2 {
            return Some(v.to_string());
        }
    }
    None
}

pub fn lts_codename_to_major() -> BTreeMap<&'static str, u32> {
    let mut m = BTreeMap::new();
    m.insert("argon", 4);
    m.insert("boron", 6);
    m.insert("carbon", 8);
    m.insert("dubnium", 10);
    m.insert("erbium", 12);
    m.insert("fermium", 14);
    m.insert("gallium", 16);
    m.insert("hydrogen", 18);
    m.insert("iron", 20);
    m.insert("jod", 22);
    m.insert("krypton", 24);
    m
}

pub fn is_lts_version(version: &str) -> bool {
    let v = version.trim_start_matches('v');
    let parts: Vec<&str> = v.split('.').collect();
    if parts.len() < 3 {
        return false;
    }
    if let Ok(major) = parts[0].parse::<u32>() {
        // A version is LTS only if its major has a registered LTS codename.
        // The old "even major >= 4" heuristic was wrong: it marked v26.x.x
        // (and any future even Current line) as LTS before that line actually
        // enters LTS, producing a bogus "✓ LTS" badge with codename "-" in
        // `nvm ls-remote` / `nvm ls`.
        let codename_map = lts_codename_to_major();
        return codename_map.values().any(|&m| m == major);
    }
    false
}

pub fn parse_major(version: &str) -> Option<u32> {
    let v = version.trim_start_matches('v');
    let parts: Vec<&str> = v.split('.').collect();
    parts.first().and_then(|p| p.parse::<u32>().ok())
}

/// Check if a directory name is a valid version directory. Accepts:
/// - `vX.Y.Z` (digit must immediately follow `v`, so `versions` is rejected)
/// - `iojs-vX.Y.Z` / `iojs-X.Y.Z`
/// - `io.js-vX.Y.Z` / `io.js-X.Y.Z`
pub fn is_version_dir_name(name: &str) -> bool {
    if let Some(rest) = name.strip_prefix("iojs-v") {
        return !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit() || c == '.');
    }
    if let Some(rest) = name.strip_prefix("iojs-") {
        return !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit() || c == '.');
    }
    if let Some(rest) = name.strip_prefix("io.js-v") {
        return !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit() || c == '.');
    }
    if let Some(rest) = name.strip_prefix("io.js-") {
        return !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit() || c == '.');
    }
    if let Some(rest) = name.strip_prefix('v') {
        return !rest.is_empty()
            && rest.chars().all(|c| c.is_ascii_digit() || c == '.')
            && rest.chars().any(|c| c.is_ascii_digit());
    }
    false
}

pub fn get_installed_versions() -> Vec<String> {
    let nvm_dir = get_nvm_dir();
    let mut versions: Vec<String> = Vec::new();
    if let Ok(rd) = fs::read_dir(&nvm_dir) {
        for entry in rd.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Some(name) = path.file_name() {
                    let name = name.to_string_lossy().to_string();
                    // Accept "vX.Y.Z" (digit must follow the `v`), "iojs-vX.Y.Z",
                    // "io.js-vX.Y.Z". Rejects "current", "versions" (nvm-sh's
                    // nested dir), "v8-flags" and any other non-version `v*`.
                    if name != "current" && is_version_dir_name(&name) {
                        versions.push(name);
                    }
                }
            }
        }
    }
    versions.sort();
    versions.reverse();
    versions
}

pub fn file_backup_path(path: &Path) -> std::path::PathBuf {
    let mut backup = path.to_path_buf();
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "backup".to_string());
    backup.set_file_name(format!("{}.bak", name));
    backup
}

pub fn backup_file(path: &Path) -> Result<(), std::io::Error> {
    if path.exists() {
        let backup = file_backup_path(path);
        fs::copy(path, backup)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_iojs_version() {
        assert!(is_iojs_version("iojs-v3.3.1"));
        assert!(is_iojs_version("io.js-v2.5.0"));
        assert!(is_iojs_version("iojs-1.0.0"));
        assert!(is_iojs_version("io.js-1.0.0"));
        assert!(!is_iojs_version("v20.0.0"));
        assert!(!is_iojs_version("20.0.0"));
        assert!(!is_iojs_version("node"));
        assert!(!is_iojs_version(""));
    }

    #[test]
    fn test_normalize_iojs_version() {
        assert_eq!(normalize_iojs_version("iojs-v3.3.1"), "iojs-v3.3.1");
        assert_eq!(normalize_iojs_version("io.js-v2.5.0"), "iojs-v2.5.0");
        assert_eq!(normalize_iojs_version("iojs-1.0.0"), "iojs-v1.0.0");
        assert_eq!(normalize_iojs_version("v3.3.1"), "iojs-v3.3.1");
    }

    #[test]
    fn test_iojs_version_number() {
        assert_eq!(iojs_version_number("iojs-v3.3.1"), Some("3.3.1".to_string()));
        assert_eq!(iojs_version_number("io.js-v2.5.0"), Some("2.5.0".to_string()));
        assert_eq!(iojs_version_number("v20.0.0"), None);
        assert_eq!(iojs_version_number("iojs-v1"), None); // only one dot
        assert_eq!(iojs_version_number("invalid"), None);
    }

    #[test]
    fn test_lts_codename_to_major() {
        let map = lts_codename_to_major();
        assert_eq!(map.get("argon"), Some(&4));
        assert_eq!(map.get("boron"), Some(&6));
        assert_eq!(map.get("iron"), Some(&20));
        assert_eq!(map.get("jod"), Some(&22));
        assert_eq!(map.get("krypton"), Some(&24));
        assert_eq!(map.get("non-existent"), None);
    }

    #[test]
    fn test_is_lts_version() {
        assert!(is_lts_version("v4.4.0"));
        assert!(is_lts_version("v6.0.0"));
        assert!(is_lts_version("v20.11.0"));
        assert!(is_lts_version("v24.18.0"));
        assert!(!is_lts_version("v3.0.0"));  // major < 4
        assert!(!is_lts_version("v5.0.0"));  // odd major
        assert!(!is_lts_version("v21.0.0")); // odd major
        assert!(!is_lts_version("v26.0.0")); // even but not LTS (no codename)
        assert!(!is_lts_version("v0.12.0")); // pre-LTS
    }

    #[test]
    fn test_is_version_dir_name() {
        // Standard Node.js versions
        assert!(is_version_dir_name("v20.20.2"));
        assert!(is_version_dir_name("v22.22.2"));
        assert!(is_version_dir_name("v0.12.18"));
        // io.js variants
        assert!(is_version_dir_name("iojs-v3.3.1"));
        assert!(is_version_dir_name("io.js-v2.5.0"));
        // Non-version directories that start with 'v' must be rejected
        assert!(!is_version_dir_name("versions"));   // nvm-sh nested dir
        assert!(!is_version_dir_name("v8-flags"));   // starts with v but not version
        assert!(!is_version_dir_name("current"));    // symlink
        assert!(!is_version_dir_name(""));           // empty
        assert!(!is_version_dir_name("node"));       // not a version
    }

    #[test]
    fn test_parse_major() {
        assert_eq!(parse_major("v20.0.0"), Some(20));
        assert_eq!(parse_major("v18.19.0"), Some(18));
        assert_eq!(parse_major("20.0.0"), Some(20));
        assert_eq!(parse_major("invalid"), None);
        assert_eq!(parse_major(""), None);
    }

    #[test]
    fn test_file_backup_path() {
        use std::path::PathBuf;
        let path = PathBuf::from("/tmp/test.txt");
        let backup = file_backup_path(&path);
        assert_eq!(backup, PathBuf::from("/tmp/test.txt.bak"));
    }

    #[test]
    fn test_compare_semver_basic() {
        use std::cmp::Ordering;
        assert_eq!(compare_semver("v20.5.0", "v20.20.2"), Ordering::Less);
        assert_eq!(compare_semver("v20.20.2", "v20.5.0"), Ordering::Greater);
        assert_eq!(compare_semver("v20.20.2", "v20.20.2"), Ordering::Equal);
        assert_eq!(compare_semver("v18.20.8", "v22.22.2"), Ordering::Less);
        assert_eq!(compare_semver("v22.22.2", "v18.20.8"), Ordering::Greater);
    }

    #[test]
    fn test_compare_semver_major_digits() {
        use std::cmp::Ordering;
        // Regression: alphabetical sort returns v99 as newer than v100.
        assert_eq!(compare_semver("v99.99.99", "v100.100.100"), Ordering::Less);
        assert_eq!(compare_semver("v100.100.100", "v99.99.99"), Ordering::Greater);
    }

    #[test]
    fn test_compare_semver_iojs() {
        use std::cmp::Ordering;
        // Different io.js releases compare numerically.
        assert_eq!(compare_semver("iojs-v2.5.0", "iojs-v3.3.1"), Ordering::Less);
        assert_eq!(compare_semver("iojs-v3.3.1", "iojs-v2.5.0"), Ordering::Greater);
        // Prefix variations are equivalent.
        assert_eq!(compare_semver("iojs-v3.3.1", "io.js-v3.3.1"), Ordering::Equal);
    }

    #[test]
    fn test_compare_semver_bare_versions() {
        use std::cmp::Ordering;
        // No "v" prefix should still work.
        assert_eq!(compare_semver("20.5.0", "20.20.2"), Ordering::Less);
        assert_eq!(compare_semver("20.20.2", "20.5.0"), Ordering::Greater);
    }

    #[test]
    fn test_compare_semver_iojs_vs_node_tiebreak() {
        use std::cmp::Ordering;
        // Documented contract: for the same major.minor.patch, io.js is
        // treated as newer than Node.js (mirrors compare_versions legacy
        // behavior). Lock this so a refactor doesn't silently flip it.
        assert_eq!(compare_semver("v3.3.1", "iojs-v3.3.1"), Ordering::Less);
        assert_eq!(compare_semver("iojs-v3.3.1", "v3.3.1"), Ordering::Greater);
        // io.js prefix variants compare equal to each other for the same
        // version numbers.
        assert_eq!(compare_semver("iojs-3.3.1", "io.js-3.3.1"), Ordering::Equal);
    }

    #[test]
    fn test_compare_semver_malformed_input_silently_zeros() {
        use std::cmp::Ordering;
        // Malformed inputs must not panic: unparseable numeric parts fall
        // back to 0 (parse_v uses `.unwrap_or(0)`). Lock this behavior so
        // a future strict-parse change is a conscious decision.
        assert_eq!(compare_semver("", "v1.0.0"), Ordering::Less);
        assert_eq!(compare_semver("v", "v1.0.0"), Ordering::Less);
        assert_eq!(compare_semver("abc", "v1.0.0"), Ordering::Less);
        // Two malformed inputs are Equal (both parse to all-zeros, non-iojs).
        assert_eq!(compare_semver("", ""), Ordering::Equal);
        assert_eq!(compare_semver("garbage", "v"), Ordering::Equal);
    }

    #[test]
    fn test_is_version_dir_name_iojs_without_v_prefix() {
        // The function explicitly accepts `iojs-1.0.0` and `io.js-1.0.0`
        // (no `v` after the dash) — cover those branches directly.
        assert!(is_version_dir_name("iojs-1.0.0"));
        assert!(is_version_dir_name("io.js-1.0.0"));
    }

    #[test]
    fn test_backup_file_copies_existing_file() {
        use std::fs;
        use std::path::PathBuf;
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("a.txt");
        fs::write(&path, b"hello").expect("write");
        backup_file(&path).expect("backup_file should succeed for existing file");
        let backup = file_backup_path(&path);
        assert_eq!(backup, PathBuf::from(dir.path()).join("a.txt.bak"));
        assert_eq!(fs::read(&backup).expect("read backup"), b"hello");
    }

    #[test]
    fn test_backup_file_no_op_for_missing_file() {
        // For a non-existent path, backup_file is a no-op (Ok(())) and must
        // NOT create a .bak file.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("missing.txt");
        backup_file(&path).expect("backup_file should be Ok for missing file");
        assert!(!dir.path().join("missing.txt.bak").exists());
    }
}
