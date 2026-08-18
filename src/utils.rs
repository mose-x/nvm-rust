use std::cmp::Ordering;
use std::fs;

use anyhow::Result;
use indicatif::ProgressStyle;

use crate::i18n::{format_t, T};
use crate::system::get_nvm_dir;

// Re-export the extracted submodules so existing `crate::utils::*` callers
// keep resolving without touching every call site.
pub use crate::fs_util::*;
pub use crate::lock::*;
pub use crate::lts::*;
pub use crate::term::*;

// `colored::Colorize` is needed for the `.yellow().bold()` call in
// `get_installed_versions`. It's already a dependency; bring it into scope
// here so the macro resolves without forcing every caller of utils to
// import it.
use colored::Colorize as _;

/// The four accepted io.js version prefixes, ordered longest-distinctive
/// first so `strip_prefix` matches `iojs-v` before `iojs-` (otherwise
/// `iojs-v3.3.1` would strip to `v3.3.1` and leave a stray `v`).
///
/// Single source of truth for the io.js spelling — previously each consumer
/// carried its own `trim_start_matches` / `strip_prefix` chain, and an early
/// copy that missed `io.js-v` made those versions invisible to the
/// `package.json#engines.node` matcher.
const IOJS_PREFIXES: &[&str] = &["iojs-v", "io.js-v", "iojs-", "io.js-"];

/// Strip any known io.js prefix from `v`, returning the remainder (without
/// the leading `v`). Returns `None` if `v` does not start with an io.js
/// prefix.
pub fn strip_iojs_prefix(v: &str) -> Option<&str> {
    IOJS_PREFIXES.iter().find_map(|p| v.strip_prefix(p))
}

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
    let s = strip_iojs_prefix(v).unwrap_or(v).trim_start_matches('v');
    let parts: Vec<&str> = s.split('-').next().unwrap_or("").split('.').collect();
    Some((
        parts.first().and_then(|s| s.parse().ok())?,
        parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0),
        parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(0),
    ))
}

/// Parse a version string into (is_iojs, major, minor, patch, pre_release)
/// for `compare_semver`. The pre-release is the substring after the first
/// `-` in the prefix-stripped form (e.g. `v2.0.0-rc.1` → `Some("rc.1")`).
fn parse_v_for_compare(v: &str) -> (bool, u32, u32, u32, Option<&str>) {
    let is_iojs = is_iojs_version(v);
    let (maj, min, pat) = parse_version_parts(v).unwrap_or((0, 0, 0));
    let s = strip_iojs_prefix(v).unwrap_or(v).trim_start_matches('v');
    let pre = s.split_once('-').map(|(_, pre)| pre);
    (is_iojs, maj, min, pat, pre)
}

/// Compare two version strings semantically (major.minor.patch), returning
/// `Greater` if `a` is newer than `b`. Handles both Node.js (`v20.11.0`) and
/// io.js (`iojs-v3.3.1`, `io.js-v2.5.0`) forms.
///
/// Pre-release suffixes (e.g. `-rc.1`, `-beta.2`) are compared per semver:
/// for equal `major.minor.patch`, a version WITHOUT a pre-release is newer
/// than one WITH a pre-release, so `v2.0.0` > `v2.0.0-rc.1`. Among two
/// pre-releases, compare the suffix lexicographically (`rc.1` < `rc.2`).
///
/// This MUST be used instead of `String::cmp` / `Vec::sort()` when picking the
/// "latest" of a set of installed versions: alphabetical sort puts `v20.5.0`
/// after `v20.20.2` (because '5' > '2' as chars), which is the wrong answer.
pub fn compare_semver(a: &str, b: &str) -> Ordering {
    let (ai, amj, ami, apa, apre) = parse_v_for_compare(a);
    let (bi, bmj, bmi, bpa, bpre) = parse_v_for_compare(b);
    // Sort by (major, minor, patch) numerically first.
    match (amj, ami, apa).cmp(&(bmj, bmi, bpa)) {
        std::cmp::Ordering::Equal => {}
        ord => return ord,
    }
    // Equal X.Y.Z: per semver, no pre-release > has pre-release.
    // None sorts as Greater than Some (no pre-release is newer).
    match (apre, bpre) {
        (None, None) => std::cmp::Ordering::Equal,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (Some(_), None) => std::cmp::Ordering::Less,
        (Some(a), Some(b)) => a.cmp(b),
    }
    .then(ai.cmp(&bi)) // break ties: io.js newer than Node.js for same version
}

/// Check if a version string is an io.js version (prefixes "iojs-" or "io.js-v")
pub fn is_iojs_version(version: &str) -> bool {
    strip_iojs_prefix(version).is_some()
}

/// Strip the io.js prefix and leading `v`, returning the bare "X.Y.Z" part.
/// Shared by [`normalize_iojs_version`] and [`iojs_version_number`] so the
/// prefix-stripping logic lives in one place.
fn iojs_bare_version(version: &str) -> &str {
    strip_iojs_prefix(version)
        .unwrap_or(version)
        .trim_start_matches('v')
}

/// Normalize an io.js version name to canonical "iojs-vX.Y.Z"
pub fn normalize_iojs_version(version: &str) -> String {
    format!("iojs-v{}", iojs_bare_version(version))
}

/// Extract the version number from an io.js version (returns "X.Y.Z")
pub fn iojs_version_number(version: &str) -> Option<String> {
    if is_iojs_version(version) {
        let v = iojs_bare_version(version);
        if v.matches('.').count() >= 2 {
            return Some(v.to_string());
        }
    }
    None
}

pub fn parse_major(version: &str) -> Option<u32> {
    let v = version.trim_start_matches('v');
    v.split('.').next()?.parse::<u32>().ok()
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
    // io.js variants all share the same body check via strip_iojs_prefix,
    // so a new spelling only needs adding to IOJS_PREFIXES.
    if let Some(rest) = strip_iojs_prefix(name) {
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
    // Distinguish NotFound (legitimate on a fresh install — no versions
    // downloaded yet) from real read_dir failures (permission denied, I/O
    // error). The previous `if let Ok(rd)` folded both into an empty list,
    // so an unreadable nvm_dir silently looked like "no versions installed"
    // and the user got confusing "version not installed" errors downstream
    // instead of a clear permission warning. Mirrors `list_all_aliases`.
    let rd = match fs::read_dir(&nvm_dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return versions,
        Err(e) => {
            eprintln!(
                "{} Failed to list installed versions in {}: {}",
                "⚠".yellow().bold(),
                nvm_dir.display(),
                e
            );
            return versions;
        }
    };
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
    versions.sort();
    versions.reverse();
    versions
}

/// Reject version strings that could escape `nvm_dir` via path traversal.
///
/// Version names are used directly in `nvm_dir.join(&version)` and similar
/// path constructions across install/uninstall/use/exec/reinstall-packages.
/// Without this guard, an input like `v1.0.0/../../etc` would let
/// `nvm uninstall "v1.0.0/../../etc"` execute `fs::remove_dir_all` on a
/// path outside `nvm_dir`, and `nvm install "v1.0.0/../../tmp/x"` would
/// write outside `nvm_dir`. The same risk applies to malicious `.nvmrc`
/// content.
///
/// This is the single source of truth for version-name safety. Every entry
/// point that accepts user input (`resolve_version`, `resolve_iojs_version`,
/// `resolve_alias`) MUST route the final resolved string through this before
/// returning it to a path-constructing caller. Aliases like `lts/*`,
/// `lts/iron`, `default`, `current`, `system`, `node` are excluded — they
/// are matched by name and never reach the path-construction branch.
pub fn validate_version_name(version: &str) -> Result<()> {
    if version.is_empty() {
        anyhow::bail!("{}", T("alias_name_empty"));
    }
    if version.contains('/')
        || version.contains('\\')
        || version.contains('\0')
        || version.contains("..")
        || version.chars().any(|c| c.is_control() || c == ' ')
    {
        anyhow::bail!(
            "{}",
            format_t("invalid_version_name", &[version.to_string()])
        );
    }
    // Windows: reject cmd.exe metacharacters that could be used for batch
    // injection via the unquoted `%CURRENT%` expansion in the Windows shim
    // script (`set BIN=%NVM_DIR%\%CURRENT%\...`). On Unix the shim uses
    // `"$CURRENT"` (double-quoted), so these are harmless.
    #[cfg(windows)]
    if version
        .chars()
        .any(|c| matches!(c, '&' | '|' | ';' | '(' | ')' | '%' | '^' | '<' | '>' | '"'))
    {
        anyhow::bail!(
            "{}",
            format_t("invalid_version_name", &[version.to_string()])
        );
    }
    Ok(())
}

/// Locate the system Node.js binary on PATH. Uses `which` on Unix and
/// `where` on Windows (Windows has no `which`). Returns the first match
/// trimmed of whitespace, or `None` if the lookup command is missing /
/// reports nothing. This is the cross-platform replacement for the
/// `Command::new("which").arg("node")` pattern that silently failed on
/// Windows.
///
/// Result is cached for the process lifetime: a single `nvm` invocation may
/// resolve `system` in multiple places (`resolve_alias`, `exec_version`,
/// `which_version`), each previously spawning a fresh `which`/`where`
/// subprocess. The system Node path cannot change mid-process, so we memoize
/// the first lookup.
pub fn find_system_node_path() -> Option<std::path::PathBuf> {
    use std::sync::OnceLock;
    static CACHE: OnceLock<Option<std::path::PathBuf>> = OnceLock::new();
    CACHE
        .get_or_init(|| {
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
        })
        .clone()
}

/// Build the shared progress-bar style used by every byte-stream download
/// (`download_to_cache`, `download_prebuilt_npm`). Centralising the template
/// here means a single `expect` covers all call sites — `indicatif`'s
/// `template()` returns `Result` because the template string is parsed at
/// runtime, but this is a known-valid static literal. `expect` (not `unwrap`)
/// makes the invariant explicit and gives a useful panic message if a future
/// `indicatif` upgrade changes the template grammar.
pub fn bytes_progress_style() -> indicatif::ProgressStyle {
    ProgressStyle::default_bar()
        .template(
            "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta})",
        )
        .expect("progress template is a known-valid static literal")
        .progress_chars("#>-")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_version_name_accepts_normal_versions() {
        assert!(validate_version_name("v20.11.0").is_ok());
        assert!(validate_version_name("20.11.0").is_ok());
        assert!(validate_version_name("iojs-v3.3.1").is_ok());
        assert!(validate_version_name("v22").is_ok());
    }

    #[test]
    fn validate_version_name_rejects_path_traversal() {
        // The CVE-shaped inputs that MUST be rejected before any
        // `nvm_dir.join(&version)` / `fs::remove_dir_all` caller sees them.
        assert!(validate_version_name("v1.0.0/../../etc").is_err());
        assert!(validate_version_name("v1.0.0\\..\\etc").is_err());
        assert!(validate_version_name("..").is_err());
        assert!(validate_version_name("v1.0.0/..").is_err());
        assert!(validate_version_name("../etc").is_err());
        assert!(validate_version_name("v1\0").is_err());
        assert!(validate_version_name("v1.0.0 ../etc").is_err());
        assert!(validate_version_name("").is_err());
    }

    #[cfg(windows)]
    #[test]
    fn validate_version_name_rejects_cmd_metacharacters() {
        // P1-13: cmd.exe metacharacters must be rejected to prevent
        // batch injection via the Windows shim's %CURRENT% expansion.
        assert!(validate_version_name("v20.0.0&calc").is_err());
        assert!(validate_version_name("v20.0.0|evil").is_err());
        assert!(validate_version_name("v20.0.0;cmd").is_err());
        assert!(validate_version_name("v20.0.0(erlang").is_err());
        assert!(validate_version_name("v20.0.0%PATH%").is_err());
        assert!(validate_version_name("v20.0.0^echo").is_err());
        assert!(validate_version_name("v20.0.0<x").is_err());
        assert!(validate_version_name("v20.0.0>x").is_err());
        assert!(validate_version_name("v20.0.0\"x").is_err());
        // Normal version names should still pass.
        assert!(validate_version_name("v20.0.0").is_ok());
        assert!(validate_version_name("iojs-v3.3.1").is_ok());
    }

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
    fn test_strip_iojs_prefix() {
        // All four spellings strip to the bare version (no leading `v`).
        assert_eq!(strip_iojs_prefix("iojs-v3.3.1"), Some("3.3.1"));
        assert_eq!(strip_iojs_prefix("io.js-v2.5.0"), Some("2.5.0"));
        assert_eq!(strip_iojs_prefix("iojs-1.0.0"), Some("1.0.0"));
        assert_eq!(strip_iojs_prefix("io.js-1.0.0"), Some("1.0.0"));
        // Non-io.js strings are not stripped.
        assert_eq!(strip_iojs_prefix("v20.0.0"), None);
        assert_eq!(strip_iojs_prefix("20.0.0"), None);
        assert_eq!(strip_iojs_prefix(""), None);
        // Order matters: `iojs-v` must win over `iojs-` so the `v` is consumed.
        assert_eq!(strip_iojs_prefix("iojs-v1.2.3"), Some("1.2.3"));
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
    fn test_compare_semver_prerelease_lower_than_release() {
        use std::cmp::Ordering;
        // Per semver: a version WITHOUT a pre-release is newer than one WITH.
        // This is the upgrade.rs bug fix — previously both parsed to (2,0,0)
        // and compared Equal, so `nvm upgrade` would skip a real release
        // when the latest tag was a pre-release.
        assert_eq!(compare_semver("v2.0.0", "v2.0.0-rc.1"), Ordering::Greater);
        assert_eq!(compare_semver("v2.0.0-rc.1", "v2.0.0"), Ordering::Less);
        assert_eq!(compare_semver("v2.0.0-beta", "v2.0.0"), Ordering::Less);
        assert_eq!(compare_semver("v2.0.0-alpha.2", "v2.0.0"), Ordering::Less);
    }

    #[test]
    fn test_compare_semver_two_prereleases_lexicographic() {
        use std::cmp::Ordering;
        // Two pre-releases of the same X.Y.Z compare lexicographically by
        // the pre-release string. `rc.1` < `rc.2`.
        assert_eq!(compare_semver("v2.0.0-rc.1", "v2.0.0-rc.2"), Ordering::Less);
        assert_eq!(
            compare_semver("v2.0.0-rc.2", "v2.0.0-rc.1"),
            Ordering::Greater
        );
        assert_eq!(
            compare_semver("v2.0.0-rc.1", "v2.0.0-rc.1"),
            Ordering::Equal
        );
    }

    #[test]
    fn test_compare_semver_prerelease_does_not_affect_different_xyz() {
        use std::cmp::Ordering;
        // When X.Y.Z differs, the pre-release is irrelevant.
        assert_eq!(compare_semver("v2.1.0-rc.1", "v2.0.0"), Ordering::Greater);
        assert_eq!(compare_semver("v1.9.0-rc.1", "v2.0.0"), Ordering::Less);
    }

    #[test]
    fn test_is_version_dir_name_iojs_without_v_prefix() {
        // The function explicitly accepts `iojs-1.0.0` and `io.js-1.0.0`
        // (no `v` after the dash) — cover those branches directly.
        assert!(is_version_dir_name("iojs-1.0.0"));
        assert!(is_version_dir_name("io.js-1.0.0"));
    }
}
