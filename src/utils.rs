use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::Result;
use indicatif::ProgressStyle;

use crate::i18n::{format_t, T};
use crate::system::get_nvm_dir;

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

// Lazy-built, process-wide cache of the LTS codename → major table.
//
// `is_lts_version` is called once per version in `nvm ls-remote` (~600
// iterations), and `get_codename` similarly. Rebuilding the 11-entry
// `BTreeMap` on every call allocated 600 maps per listing for nothing —
// the table is immutable for the process lifetime. `lazy_static` builds it
// once on first access; subsequent callers get a `&'static` reference.
lazy_static::lazy_static! {
    static ref LTS_CODENAME_TO_MAJOR: BTreeMap<&'static str, u32> = {
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
        m.insert("jodhpur", 22);
        m.insert("krypton", 24);
        m
    };
}

/// The LTS codename → major table, built once and reused for the process
/// lifetime (see [`LTS_CODENAME_TO_MAJOR`]). Returns a `&'static` reference
/// so hot callers like `is_lts_version` (called ~600× per `nvm ls-remote`)
/// pay zero allocation.
pub fn lts_codename_to_major() -> &'static BTreeMap<&'static str, u32> {
    &LTS_CODENAME_TO_MAJOR
}

/// Hardcoded LTS codename → major fallback used when the network is
/// unavailable or `index.json` can't be parsed. This is the `&'static str`
/// view; `lts_codename_to_major_with_remote` merges dynamic entries over it.
fn lts_codename_to_major_fallback() -> BTreeMap<String, u32> {
    lts_codename_to_major()
        .iter()
        .map(|(k, v)| (k.to_string(), *v))
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
    // Count dots without allocating a Vec: LTS check needs the major and
    // requires a full vX.Y.Z (>= 2 dots). `split('.').next()` gives the
    // major without collecting the rest.
    if v.matches('.').count() < 2 {
        return false;
    }
    if let Some(first) = v.split('.').next() {
        if let Ok(major) = first.parse::<u32>() {
            // A version is LTS only if its major has a registered LTS codename.
            // The old "even major >= 4" heuristic was wrong: it marked v26.x.x
            // (and any future even Current line) as LTS before that line actually
            // enters LTS, producing a bogus "✓ LTS" badge with codename "-" in
            // `nvm ls-remote` / `nvm ls`.
            let codename_map = lts_codename_to_major();
            return codename_map.values().any(|&m| m == major);
        }
    }
    false
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
    Ok(())
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
pub fn atomic_write(path: &Path, contents: &str) -> Result<()> {
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
    std::io::Write::write_all(&mut tmp, contents.as_bytes())?;
    tmp.persist(path).map_err(|e| anyhow::anyhow!(e.error))?;
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
///
/// Returns a borrowed `Cow` when `s` is already at least `width` columns
/// (no padding needed), avoiding an allocation in the common case where the
/// input already fits — e.g. `render_table` calls this per cell.
pub fn pad_right(s: &str, width: usize) -> std::borrow::Cow<'_, str> {
    let w = display_width(s);
    if w >= width {
        std::borrow::Cow::Borrowed(s)
    } else {
        std::borrow::Cow::Owned(format!("{}{}", s, " ".repeat(width - w)))
    }
}

/// Right-align `s` to `width` columns, padding with spaces on the left.
pub fn pad_left(s: &str, width: usize) -> std::borrow::Cow<'_, str> {
    let w = display_width(s);
    if w >= width {
        std::borrow::Cow::Borrowed(s)
    } else {
        std::borrow::Cow::Owned(format!("{}{}", " ".repeat(width - w), s))
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
    // Copy directly and map NotFound → Ok(()) instead of `exists()` + `copy`.
    // The two-step form is a TOCTOU race: a concurrent process could remove
    // the file between the stat and the open, turning a "nothing to back up"
    // no-op into a confusing "No such file or directory" error. The single
    // read is also one syscall instead of two.
    match fs::copy(path, file_backup_path(path)) {
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
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

// ---------------------------------------------------------------------------
// Concurrency: nvm-wide advisory lock
// ---------------------------------------------------------------------------

/// Process-local flag for re-entrancy. `nvm use --install` calls `install`
/// internally, and both want to hold the nvm lock; a second `flock(LOCK_EX)`
/// on the same file from the *same* process can self-deadlock on some
/// platforms, so we track ownership per-process and hand out a no-op guard
/// when the lock is already held. Cross-process contention is still
/// serialised by the OS lock itself.
static NVM_LOCK_HELD: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// RAII guard holding an exclusive OS advisory lock on the nvm directory.
///
/// Prevents two `nvm` processes from racing on mutating operations
/// (install / uninstall / use) against the same `NVM_DIR`. The lock is an
/// OS-level advisory lock (`flock` on Unix, `LockFileEx` on Windows) on the
/// `.nvm.lock` file, which the kernel releases automatically when the
/// holding process exits — so a crashed/killed `nvm` never leaves a stale
/// lock behind (the previous PID-based lock-file approach had both a
/// reclaim race and stale-lock leaks on crash).
///
/// The `Option<File>` is `None` for a re-entrant acquire (the current
/// process already holds the lock via an outer call); dropping a re-entrant
/// guard does not release the lock.
///
/// Acquired via [`acquire_nvm_lock`]; released on `Drop`.
pub struct NvmLock(Option<std::fs::File>);

impl Drop for NvmLock {
    fn drop(&mut self) {
        if let Some(file) = self.0.take() {
            // `fs4::fs_std::FileExt::unlock` is brought into scope by the
            // `use` inside `acquire_nvm_lock` for the acquire path; for the
            // drop path we reference it fully-qualified to avoid a stale
            // module-level import.
            //
            // ORDER MATTERS: clear the process-local flag BEFORE releasing the
            // OS lock. If we unlock first, there is a window where the OS lock
            // is free but `NVM_LOCK_HELD` is still `true` — another thread in
            // THIS process calling `acquire_nvm_lock` would then see `swap`
            // return `true`, take the re-entrant no-op branch, and execute its
            // critical section with NO OS lock while a different process has
            // already grabbed the now-free OS lock. That breaks mutual
            // exclusion silently.
            //
            // Clearing the flag first means a same-process contender sees
            // `false`, tries `swap(true)` → `false`, and goes for the OS lock
            // (still held by us) → blocks until we unlock. Correct.
            //
            // If `unlock` itself fails (extremely rare: invalid fd, kernel
            // error), the OS lock stays held with `NVM_LOCK_HELD=false`. The
            // next same-process acquire will then block on `lock_exclusive`
            // rather than silently bypass — a safer failure mode than the
            // silent-bypass window above. We still surface the failure as a
            // warning so it is diagnosable.
            NVM_LOCK_HELD.store(false, std::sync::atomic::Ordering::Release);
            if let Err(e) = fs4::fs_std::FileExt::unlock(&file) {
                eprintln!(
                    "{} {}: {}",
                    "⚠".yellow().bold(),
                    crate::i18n::T("lock_release_failed"),
                    e
                );
            }
        }
        // Re-entrant guard (None): nothing to release; the outer guard still
        // owns the OS lock and the `NVM_LOCK_HELD` flag.
    }
}

/// RAII guard that removes a file when dropped, unless disarmed.
///
/// Used by `download_prebuilt_npm` to ensure the npm tarball is cleaned up
/// on EVERY exit path (download `io::copy` failure, truncation, integrity
/// mismatch, tar extraction failure, symlink failure), not just the success
/// path. Previously only the truncation/integrity branches and the final
/// success line cleaned up; an `io::copy` `?` left a half-written
/// `npm-v*.tgz` that the next run's `exists()` cache-hit check treated as
/// complete, silently skipping re-download and then failing at extraction
/// with a confusing "unexpected EOF".
///
/// On the success path the caller removes the file explicitly (so a failure
/// between staging and disarm still triggers cleanup via `Drop`) and then
/// calls `disarm()` so `Drop` does not issue a redundant `remove_file`.
pub struct FileGuard {
    path: std::path::PathBuf,
    armed: bool,
}

impl FileGuard {
    /// Create an armed guard for `path`. The file at `path` need not exist
    /// yet (e.g. it is about to be created by a download); `Drop` will
    /// silently tolerate a missing file via `remove_file`'s `Err` being
    /// ignored.
    pub fn new(path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
            armed: true,
        }
    }

    /// Disarm the guard so `Drop` does not remove the file. Call this only
    /// after the file has been successfully consumed (extracted + wired up)
    /// AND explicitly removed by the caller, so a failure between staging
    /// and disarm still triggers cleanup.
    pub fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for FileGuard {
    fn drop(&mut self) {
        if self.armed {
            // Best-effort: a missing file (e.g. download never started, or
            // caller already removed it) is not an error. Any other I/O
            // error (permission denied, etc.) is swallowed because we are
            // on an unwind/early-return path where surfacing it would mask
            // the real error in flight.
            let _ = fs::remove_file(&self.path);
        }
    }
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

/// Acquire an exclusive lock on the nvm directory, blocking until any other
/// `nvm` process releases it.
///
/// The lock file lives at `<nvm_dir>/.nvm.lock`. We open it `create(true)`
/// so the first-ever invocation creates it; subsequent invocations reuse the
/// same file and contend on the OS lock, not on file creation (which would
/// be racy).
///
/// **Re-entrant within a process**: if the current process already holds the
/// lock (e.g. `nvm use --install` → `install`), this returns a no-op guard
/// instead of deadlocking on a second `flock(LOCK_EX)` on the same file.
///
/// Returns an [`NvmLock`] whose `Drop` releases the lock. Hold it for the
/// duration of the mutating operation (install / uninstall / use).
pub fn acquire_nvm_lock(nvm_dir: &Path) -> Result<NvmLock> {
    use fs4::fs_std::FileExt;

    // Re-entrancy: already held in this process → hand out a no-op guard.
    // `swap` returns the previous value; if it was already `true`, another
    // frame in this process owns the real lock, so we don't touch the OS
    // lock and we must NOT flip the flag back (the outer owner will).
    if NVM_LOCK_HELD.swap(true, std::sync::atomic::Ordering::AcqRel) {
        return Ok(NvmLock(None));
    }

    // Ensure the EXACT dir we were passed exists -- not a re-derived
    // `get_nvm_dir()` path. If `NVM_DIR` changed between the caller's
    // capture and here (e.g. a parallel test doing `set_var("NVM_DIR", ..)`),
    // `ensure_nvm_dir()` would create a *different* dir and the
    // `open(nvm_dir.join(".nvm.lock"))` below would hit ENOENT. Using the
    // parameter closes that race and is more correct: we lock the dir the
    // caller asked us to lock.
    //
    // The flag was already flipped to `true` above, so on failure we MUST
    // roll it back — otherwise every subsequent `acquire_nvm_lock` in this
    // process takes the re-entrant branch and returns a no-op guard,
    // silently bypassing the OS lock and breaking mutual exclusion.
    fs::create_dir_all(nvm_dir).inspect_err(|_| {
        NVM_LOCK_HELD.store(false, std::sync::atomic::Ordering::Release);
    })?;
    let lock_path = nvm_dir.join(".nvm.lock");
    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)
        .map_err(|e| {
            // Roll back the flag so a later retry can actually acquire.
            NVM_LOCK_HELD.store(false, std::sync::atomic::Ordering::Release);
            anyhow::anyhow!("{}: {e}", crate::i18n::T("lock_open_failed"))
        })?;

    // Fast path: try a non-blocking acquire so the common single-invocation
    // case returns instantly. If another nvm holds the lock, fall back to a
    // blocking acquire (with a notice) so we wait for it instead of erroring
    // out — concurrent `nvm install` should serialize, not fail.
    match file.try_lock_exclusive() {
        Ok(()) => Ok(NvmLock(Some(file))),
        Err(_) => {
            eprintln!("  {} {}", "⏳".cyan(), crate::i18n::T("lock_wait_another"));
            match file.lock_exclusive() {
                Ok(()) => Ok(NvmLock(Some(file))),
                Err(e) => {
                    NVM_LOCK_HELD.store(false, std::sync::atomic::Ordering::Release);
                    Err(anyhow::anyhow!(
                        "{}: {e}",
                        crate::i18n::T("lock_acquire_failed")
                    ))
                }
            }
        }
    }
}

// `colored::Colorize` is needed for the cyan() call in `acquire_nvm_lock`'s
// blocking path. It's already a dependency; bring it into scope here so the
// macro resolves without forcing every caller of utils to import it.
use colored::Colorize as _;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // Both lock tests mutate the process-global `NVM_LOCK_HELD` flag and
    // acquire the OS lock on the real nvm dir (resolved via `get_nvm_dir`).
    // Running them in parallel (cargo test's default) would let one test's
    // acquire race with the other's drop, producing flaky flag assertions
    // and self-deadlock on `flock(LOCK_EX)`. They also READ NVM_DIR via
    // `get_nvm_dir()`, so a parallel test in another module doing
    // `set_var("NVM_DIR", ..)` could point them at a temp dir that gets
    // restored mid-acquire. The process-global `ENV_TESTS_MUTEX` closes both
    // gaps: it serializes the lock tests against each other AND against
    // every other NVM_DIR-touching test across the crate.
    use crate::system::ENV_TESTS_MUTEX;

    #[test]
    fn acquire_nvm_lock_is_reentrant_in_same_process() {
        let _guard = ENV_TESTS_MUTEX.lock().expect("ENV_TESTS_MUTEX poisoned");
        // Two nested acquires in the SAME process must not deadlock: the
        // inner one returns a no-op guard (re-entrant) because the outer
        // already holds the OS lock. This is the `nvm use --install` →
        // `install` path.
        let nvm_dir = crate::system::get_nvm_dir();
        let outer = acquire_nvm_lock(&nvm_dir).expect("outer acquire");
        // Inner acquire should succeed instantly (no-op guard), not block.
        let inner = acquire_nvm_lock(&nvm_dir).expect("inner re-entrant acquire");
        drop(inner);
        drop(outer);
        // After both drop, the flag must be cleared so a subsequent real
        // acquire works again.
        assert!(!NVM_LOCK_HELD.load(std::sync::atomic::Ordering::Acquire));
    }

    #[test]
    fn acquire_nvm_lock_can_be_reacquired_after_drop() {
        let _guard = ENV_TESTS_MUTEX.lock().expect("ENV_TESTS_MUTEX poisoned");
        // Regression for the drop-order bug: previously `drop` released the
        // OS lock FIRST and only then cleared `NVM_LOCK_HELD`. If anything
        // went wrong between those two steps (panic, reentrant re-acquire
        // from another thread), the flag could be left `true` with the OS
        // lock free, making every subsequent same-process acquire take the
        // re-entrant no-op branch and silently bypass mutual exclusion.
        //
        // Here we exercise the acquire → drop → acquire → drop cycle several
        // times. After each drop the flag MUST be `false` (proving the flag
        // was cleared, not left dangling), and the next acquire MUST succeed
        // as a REAL acquire (proving the OS lock was actually released, not
        // leaked). If the OS lock were leaked, the second acquire would
        // self-deadlock on `flock(LOCK_EX)` and hang the test.
        let nvm_dir = crate::system::get_nvm_dir();
        for i in 0..5 {
            let guard = acquire_nvm_lock(&nvm_dir)
                .unwrap_or_else(|e| panic!("iteration {i}: acquire failed: {e}"));
            // While held, the flag must be true.
            assert!(
                NVM_LOCK_HELD.load(std::sync::atomic::Ordering::Acquire),
                "iteration {i}: flag not set after acquire"
            );
            drop(guard);
            // After drop, the flag must be cleared before any further acquire.
            assert!(
                !NVM_LOCK_HELD.load(std::sync::atomic::Ordering::Acquire),
                "iteration {i}: flag not cleared after drop — drop order regression"
            );
        }
    }

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
    fn test_lts_codename_to_major() {
        let map = lts_codename_to_major();
        assert_eq!(map.get("argon"), Some(&4));
        assert_eq!(map.get("boron"), Some(&6));
        assert_eq!(map.get("iron"), Some(&20));
        assert_eq!(map.get("jodhpur"), Some(&22));
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
