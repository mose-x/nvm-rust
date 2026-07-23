use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use crate::system::get_nvm_dir;

/// Strip all known version prefixes (Node.js `v`, io.js `iojs-v` / `iojs-` /
/// `io.js-v` / `io.js-`) and parse the major.minor.patch tuple. Pre-release
/// suffixes (e.g. `-rc.1`) are discarded via `split('-')`.
///
/// This is the single source of truth for version parsing — shared by
/// `compare_semver` and `commands::info::parse_v_tuple` so the two can never
/// drift apart on which prefixes they handle (a previous bug where
/// `parse_v_tuple` missed `io.js-v` / `io.js-` caused io.js versions to be
/// invisible to the `package.json#engines.node` range matcher).
pub fn parse_version_parts(v: &str) -> Option<(u32, u32, u32)> {
    let s = v
        .trim_start_matches("iojs-v")
        .trim_start_matches("io.js-v")
        .trim_start_matches("iojs-")
        .trim_start_matches("io.js-")
        .trim_start_matches('v');
    let parts: Vec<&str> = s.split('-').next().unwrap_or("").split('.').collect();
    Some((
        parts.first().and_then(|s| s.parse().ok())?,
        parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0),
        parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(0),
    ))
}

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
        let (maj, min, pat) = parse_version_parts(v).unwrap_or((0, 0, 0));
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
    version.starts_with("iojs-v")
        || version.starts_with("io.js-v")
        || version.starts_with("iojs-")
        || version.starts_with("io.js-")
}

/// Normalize an io.js version name to canonical "iojs-vX.Y.Z"
pub fn normalize_iojs_version(version: &str) -> String {
    let v = version
        .trim_start_matches("io.js-")
        .trim_start_matches("iojs-")
        .trim_start_matches('v');
    format!("iojs-v{}", v)
}

/// Extract the version number from an io.js version (returns "X.Y.Z")
pub fn iojs_version_number(version: &str) -> Option<String> {
    if is_iojs_version(version) {
        let v = version
            .trim_start_matches("io.js-")
            .trim_start_matches("iojs-")
            .trim_start_matches('v');
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

/// Hardcoded LTS codename → major fallback used when the network is
/// unavailable or `index.json` can't be parsed. This is the `&'static str`
/// view; `lts_codename_to_major_with_remote` merges dynamic entries over it.
fn lts_codename_to_major_fallback() -> BTreeMap<String, u32> {
    lts_codename_to_major()
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect()
}

/// Return the codename → major map, merging the hardcoded fallback with a
/// live `index.json` fetch. Dynamic entries override fallback entries with
/// the same key (so a bumped codename wins), and new codenames from the
/// manifest are added. On any network/parse failure the fallback table is
/// returned unchanged — the caller never has to handle an error.
///
/// Use this in code paths that already do network work (install, listing,
/// alias resolution with a config). The no-arg `lts_codename_to_major`
/// stays available for hot/synchronous paths like `is_lts_version` where a
/// network round-trip would be unacceptable; it always reflects the shipped
/// table, which is correct for every past LTS line.
pub fn lts_codename_to_major_with_remote(base_url: &str) -> BTreeMap<String, u32> {
    let mut m = lts_codename_to_major_fallback();
    let remote = crate::system::fetch_lts_codename_map(base_url);
    for (k, v) in remote {
        m.insert(k, v);
    }
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

/// Validate that `input` is a bare-major or major.minor shorthand —
/// `"22"`, `"22.5"`, optionally with a leading `v` (`"v22"`, `"v22.5"`).
/// On success returns the inner string (prefix-stripped). Returns `None`
/// for full versions (`"22.5.1"`, more than one dot), aliases, io.js
/// names, `system`, empty input, or anything containing non-digit/non-dot
/// characters.
///
/// This is the shared core of `version_resolve::bare_major_for_install`
/// and `config::bare_major_prefix`, which previously each carried their
/// own copy of the same strip-v / count-dots / all-digit validation.
pub(crate) fn validate_bare_major(input: &str) -> Option<&str> {
    let s = input.strip_prefix('v').unwrap_or(input);
    if s.matches('.').count() > 1 {
        return None;
    }
    if s.is_empty() || !s.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return None;
    }
    // Reject pure "." (no digits at all).
    if s.chars().all(|c| !c.is_ascii_digit()) {
        return None;
    }
    Some(s)
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

/// Atomically write `contents` to `path` using the temp-file-then-rename
/// pattern. On Unix, `fs::rename` is atomic — readers always see either the
/// old or the new file, never a half-written one. This prevents concurrent
/// `nvm use` invocations from interleaving writes (which would leave a
/// truncated `current`/`config.json` behind), and protects against a crash
/// mid-write corrupting the file.
///
/// The temp file is created in the same directory as the target (required for
/// rename to be atomic — cross-device rename is not). On failure the temp
/// file is removed by `NamedTempFile`'s Drop.
pub fn atomic_write(path: &Path, contents: &str) -> Result<(), std::io::Error> {
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
    std::io::Write::write_all(&mut tmp, contents.as_bytes())?;
    tmp.persist(path).map_err(|e| e.error)?;
    Ok(())
}

/// Calculate display width of a string, ignoring ANSI color escape codes
/// and counting CJK / wide characters as 2 columns. Used for aligning
/// table columns and help-text option columns in both `commands.rs`
/// (version listings, proxy status) and `cli.rs` (per-command help).
pub fn display_width(s: &str) -> usize {
    let mut width = 0;
    let mut in_escape = false;
    for c in s.chars() {
        if in_escape {
            if c == 'm' {
                in_escape = false;
            }
        } else if c == '\x1b' {
            in_escape = true;
        } else {
            let cp = c as u32;
            // Approximate: CJK characters and wide symbols take 2 columns
            let w = if (0x1100..=0x115F).contains(&cp)    // Hangul Jamo
                || (0x2E80..=0x303E).contains(&cp)     // CJK Radicals etc.
                || (0x3041..=0x33FF).contains(&cp)     // Hiragana etc.
                || (0x3400..=0x4DBF).contains(&cp)     // CJK Ext A
                || (0x4E00..=0x9FFF).contains(&cp)     // CJK Unified
                || (0xA000..=0xA4CF).contains(&cp)     // Yi Syllables
                || (0xAC00..=0xD7A3).contains(&cp)     // Hangul
                || (0xF900..=0xFAFF).contains(&cp)     // CJK Compat
                || (0xFE30..=0xFE4F).contains(&cp)     // CJK Compat Forms
                || (0xFF00..=0xFF60).contains(&cp)     // Fullwidth Forms
                || (0xFFE0..=0xFFE6).contains(&cp)     // Fullwidth Forms
                || (0x20000..=0x2FFFD).contains(&cp)   // CJK Ext B-D
                || (0x30000..=0x3FFFD).contains(&cp)
            {
                2
            } else {
                1
            };
            width += w;
        }
    }
    width
}

/// Left-align `s` to `width` columns, padding with spaces on the right.
/// Uses `display_width` so ANSI-coloured and CJK strings pad correctly.
pub fn pad_right(s: &str, width: usize) -> String {
    let w = display_width(s);
    if w >= width {
        s.to_string()
    } else {
        format!("{}{}", s, " ".repeat(width - w))
    }
}

/// Right-align `s` to `width` columns, padding with spaces on the left.
pub fn pad_left(s: &str, width: usize) -> String {
    let w = display_width(s);
    if w >= width {
        s.to_string()
    } else {
        format!("{}{}", " ".repeat(width - w), s)
    }
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

/// Locate the system Node.js binary on PATH. Uses `which` on Unix and
/// `where` on Windows (Windows has no `which`). Returns the first match
/// trimmed of whitespace, or `None` if the lookup command is missing /
/// reports nothing. This is the cross-platform replacement for the
/// `Command::new("which").arg("node")` pattern that silently failed on
/// Windows.
pub fn find_system_node_path() -> Option<std::path::PathBuf> {
    use std::process::Command;
    let output = if cfg!(unix) {
        Command::new("which").arg("node").output().ok()?
    } else if cfg!(windows) {
        Command::new("where").arg("node").output().ok()?
    } else {
        return None;
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    // `where` on Windows prints one path per line; `which` prints one line.
    // Take the first non-empty trimmed line in both cases.
    let first = stdout.lines().map(|l| l.trim()).find(|l| !l.is_empty())?;
    Some(std::path::PathBuf::from(first))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_bare_major() {
        // Bare major / major.minor, with optional v prefix.
        assert_eq!(validate_bare_major("22"), Some("22"));
        assert_eq!(validate_bare_major("22.5"), Some("22.5"));
        assert_eq!(validate_bare_major("v22"), Some("22"));
        assert_eq!(validate_bare_major("v22.5"), Some("22.5"));

        // Full versions (more than one dot) are rejected.
        assert_eq!(validate_bare_major("22.5.1"), None);
        assert_eq!(validate_bare_major("v22.5.1"), None);

        // Non-numeric / aliases / system / io.js are rejected.
        assert_eq!(validate_bare_major("lts/iron"), None);
        assert_eq!(validate_bare_major("system"), None);
        assert_eq!(validate_bare_major("iojs-3.3.1"), None);
        assert_eq!(validate_bare_major(""), None);
        assert_eq!(validate_bare_major("."), None);
        assert_eq!(validate_bare_major("22a"), None);
    }

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
    fn test_parse_version_parts() {
        assert_eq!(parse_version_parts("v20.11.0"), Some((20, 11, 0)));
        assert_eq!(parse_version_parts("20.11.0"), Some((20, 11, 0)));
        assert_eq!(parse_version_parts("iojs-v3.3.1"), Some((3, 3, 1)));
        assert_eq!(parse_version_parts("io.js-v3.3.1"), Some((3, 3, 1)));
        assert_eq!(parse_version_parts("io.js-3.3.1"), Some((3, 3, 1)));
        assert_eq!(parse_version_parts("v22"), Some((22, 0, 0)));
        // Pre-release suffix stripped.
        assert_eq!(parse_version_parts("v20.11.1-rc.1"), Some((20, 11, 1)));
        // Unparseable major.
        assert_eq!(parse_version_parts("node"), None);
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
        assert_eq!(
            iojs_version_number("iojs-v3.3.1"),
            Some("3.3.1".to_string())
        );
        assert_eq!(
            iojs_version_number("io.js-v2.5.0"),
            Some("2.5.0".to_string())
        );
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
        assert!(!is_lts_version("v3.0.0")); // major < 4
        assert!(!is_lts_version("v5.0.0")); // odd major
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
        assert!(!is_version_dir_name("versions")); // nvm-sh nested dir
        assert!(!is_version_dir_name("v8-flags")); // starts with v but not version
        assert!(!is_version_dir_name("current")); // symlink
        assert!(!is_version_dir_name("")); // empty
        assert!(!is_version_dir_name("node")); // not a version
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
        assert_eq!(
            compare_semver("v100.100.100", "v99.99.99"),
            Ordering::Greater
        );
    }

    #[test]
    fn test_compare_semver_iojs() {
        use std::cmp::Ordering;
        // Different io.js releases compare numerically.
        assert_eq!(compare_semver("iojs-v2.5.0", "iojs-v3.3.1"), Ordering::Less);
        assert_eq!(
            compare_semver("iojs-v3.3.1", "iojs-v2.5.0"),
            Ordering::Greater
        );
        // Prefix variations are equivalent.
        assert_eq!(
            compare_semver("iojs-v3.3.1", "io.js-v3.3.1"),
            Ordering::Equal
        );
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

    #[test]
    fn test_atomic_write_creates_new_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("current");
        atomic_write(&path, "v20.11.0").expect("atomic_write should succeed");
        assert_eq!(std::fs::read_to_string(&path).expect("read"), "v20.11.0");
    }

    #[test]
    fn test_atomic_write_overwrites_existing_file() {
        // The current-file save path relies on overwrite being atomic: a
        // concurrent reader must never see a half-written file. Verify the
        // final content is exactly the new content (not appended, not mixed).
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("current");
        std::fs::write(&path, "v18.0.0").expect("initial write");
        atomic_write(&path, "v22.22.2").expect("atomic_write overwrite");
        assert_eq!(std::fs::read_to_string(&path).expect("read"), "v22.22.2");
    }

    #[test]
    fn test_atomic_write_leaves_no_temp_files() {
        // NamedTempFile::persist renames the temp file; on success the temp
        // is gone and only the target remains. A leftover *.tmp or hidden
        // temp file would accumulate across `nvm use` invocations.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("current");
        atomic_write(&path, "v20.0.0").expect("atomic_write");
        let entries: Vec<_> = std::fs::read_dir(dir.path()).expect("read_dir").collect();
        assert_eq!(
            entries.len(),
            1,
            "exactly one file (the target) should exist"
        );
        assert_eq!(
            entries[0].as_ref().expect("entry").file_name(),
            std::ffi::OsStr::new("current")
        );
    }

    #[test]
    fn test_display_width_ascii() {
        assert_eq!(display_width(""), 0);
        assert_eq!(display_width("abc"), 3);
        assert_eq!(display_width("nvm use"), 7);
    }

    #[test]
    fn test_display_width_cjk_counts_as_two() {
        // CJK characters occupy 2 terminal columns; the width math in
        // render_table and print_cmd_section depends on this.
        assert_eq!(display_width("中"), 2);
        assert_eq!(display_width("中文"), 4);
        assert_eq!(display_width("v20.11.0 (中文)"), 15); // 8 + 1 + 1 + 4 + 1
    }

    #[test]
    fn test_display_width_ignores_ansi_escapes() {
        // Colored output from the `colored` crate wraps text in `\x1b[...m`
        // escape sequences that occupy 0 columns. If these were counted,
        // column alignment would break whenever any cell is colored.
        assert_eq!(display_width("\x1b[32mabc\x1b[0m"), 3);
        assert_eq!(display_width("\x1b[1;31merror\x1b[0m"), 5);
    }

    #[test]
    fn test_pad_right_aligns_ascii() {
        assert_eq!(pad_right("abc", 5), "abc  ");
        assert_eq!(pad_right("abc", 3), "abc");
        // Already wider than target → returned unchanged (no truncation).
        assert_eq!(pad_right("abcdef", 3), "abcdef");
    }

    #[test]
    fn test_pad_right_counts_cjk_as_two_columns() {
        // A single CJK char needs 1 space to reach width 3.
        assert_eq!(pad_right("中", 3), "中 ");
        assert_eq!(pad_right("中文", 6), "中文  ");
    }

    #[test]
    fn test_pad_left_right_aligns() {
        assert_eq!(pad_left("abc", 5), "  abc");
        assert_eq!(pad_left("abc", 3), "abc");
        assert_eq!(pad_left("中", 4), "  中");
    }
}
