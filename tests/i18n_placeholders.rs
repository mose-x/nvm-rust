//! Static guards on i18n placeholder usage.
//!
//! `all_locales_have_same_keys_as_en` (in `src/i18n.rs`) checks that every
//! locale declares the same *key set*. It cannot see whether a call site
//! actually substituted the placeholders those keys contain — which is exactly
//! the bug behind `refresh_shims`: both locales carry a `{0}`, the call site
//! used `T()` instead of `format_t()`, and every `nvm upgrade` printed
//!
//! ```text
//! ✓ Shims refreshed ({0} commands) (8 commands)
//! ```
//!
//! In Chinese it was worse, because the hardcoded suffix stayed English:
//! `✓ Shim 已刷新（{0} 个命令） (8 commands)`.
//!
//! These tests close that gap.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Placeholder indices used by a translation value, e.g. `"a {0} b {1}"` -> `[0, 1]`.
///
/// Hand-rolled rather than a regex so the definition of "placeholder" matches
/// `i18n::substitute_params` exactly: `{`, one or more ASCII digits, `}`.
fn placeholders(value: &str) -> BTreeSet<usize> {
    let bytes = value.as_bytes();
    let mut found = BTreeSet::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'{' {
            let mut j = i + 1;
            let mut n = 0usize;
            let mut digits = 0;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                n = n * 10 + (bytes[j] - b'0') as usize;
                digits += 1;
                j += 1;
            }
            if digits > 0 && j < bytes.len() && bytes[j] == b'}' {
                found.insert(n);
                i = j + 1;
                continue;
            }
        }
        i += 1;
    }
    found
}

/// Every `.rs` file under `dir`, recursively.
fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display())) {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// Flat `key = "value"` map for one locale, mirroring `i18n::collect_string_values`.
///
/// Taking only string values drops the `[_meta]` table for free, since a table
/// is not a string.
fn load_locale(code: &str) -> BTreeMap<String, String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("locales")
        .join(format!("{code}.toml"));
    let text =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let root: toml::Value =
        toml::from_str(&text).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()));
    let mut out = BTreeMap::new();
    for (k, v) in root.as_table().expect("locale must be a TOML table") {
        if let Some(s) = v.as_str() {
            out.insert(k.clone(), s.to_string());
        }
    }
    out
}

/// Every line of every `.rs` file under `src/` as (file, 1-based line number,
/// text), so a failure points somewhere actionable.
fn src_lines() -> Vec<(String, usize, String)> {
    let mut files = Vec::new();
    rust_files(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut files,
    );
    assert!(!files.is_empty(), "no .rs files found under src/");

    let mut out = Vec::new();
    for file in files {
        let rel = file
            .strip_prefix(env!("CARGO_MANIFEST_DIR"))
            .unwrap_or(&file)
            .display()
            .to_string();
        let text = std::fs::read_to_string(&file)
            .unwrap_or_else(|e| panic!("read {}: {e}", file.display()));
        out.extend(
            text.lines()
                .enumerate()
                .map(|(i, l)| (rel.clone(), i + 1, l.to_string())),
        );
    }
    out
}

/// A translation containing `{n}` must be read with `format_t`, never `T`.
///
/// `T()` returns the template verbatim, so the placeholder reaches the user.
/// The match is on `T("<key>")` with a capital `T`; `format_t("<key>"` ends in
/// a lowercase `t` and so cannot be caught by accident.
#[test]
fn placeholder_keys_are_never_consumed_with_bare_t() {
    let en = load_locale("en");
    let cn = load_locale("cn");

    let mut keys: BTreeSet<&String> = BTreeSet::new();
    for (k, v) in en.iter().chain(cn.iter()) {
        if !placeholders(v).is_empty() {
            keys.insert(k);
        }
    }
    assert!(
        keys.len() > 20,
        "expected a meaningful number of parameterised keys, found {}",
        keys.len()
    );

    let lines = src_lines();
    let mut violations = Vec::new();
    for key in keys {
        let bare = format!("T(\"{key}\")");
        for (file, lineno, text) in &lines {
            if text.contains(&bare) {
                violations.push(format!("{file}:{lineno}: {text}"));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "{} translation key(s) contain {{n}} placeholders but are read with T() \
         instead of format_t(), so the placeholder leaks into user output:\n  {}",
        violations.len(),
        violations.join("\n  ")
    );
}

/// en and cn must parameterise a key identically.
///
/// Same key set is already enforced by `all_locales_have_same_keys_as_en`, but
/// a cn translation that drops `{0}` would render a sentence with a hole in it
/// while `format_t` silently discards the argument.
#[test]
fn placeholder_indices_match_across_locales() {
    let en = load_locale("en");
    let cn = load_locale("cn");

    let mut mismatched = Vec::new();
    for (key, en_value) in &en {
        let Some(cn_value) = cn.get(key) else {
            continue; // key-set parity is the other test's job
        };
        let (en_ph, cn_ph) = (placeholders(en_value), placeholders(cn_value));
        if en_ph != cn_ph {
            mismatched.push(format!(
                "{key}: en={en_ph:?} cn={cn_ph:?}\n    en = {en_value:?}\n    cn = {cn_value:?}"
            ));
        }
    }

    assert!(
        mismatched.is_empty(),
        "{} key(s) use different placeholders per locale:\n  {}",
        mismatched.len(),
        mismatched.join("\n  ")
    );
}

#[test]
fn placeholder_parser_handles_edge_cases() {
    // Guards the guards: if `placeholders` drifts from `substitute_params`,
    // both tests above go quietly vacuous.
    assert_eq!(
        placeholders("Shims refreshed ({0} commands)"),
        BTreeSet::from([0])
    );
    assert_eq!(placeholders("{0} and {1} and {0}"), BTreeSet::from([0, 1]));
    assert_eq!(placeholders("{12}"), BTreeSet::from([12]));
    assert!(placeholders("no placeholders here").is_empty());
    // Not placeholders: no digits, unterminated, or braces used as prose.
    assert!(placeholders("{}").is_empty());
    assert!(placeholders("{0").is_empty());
    assert!(placeholders("{ }").is_empty());
}

/// Error-context strings deliberately left in English (P2-4).
///
/// Each is either an internal invariant a user cannot act on, or detail that
/// never reaches a terminal:
///
/// - `no versions collected` (`alias.rs`) — unreachable unless a code path
///   forgets to push into the result vector.
/// - `io.js install requested but no version provided` (`install.rs`) — a
///   `Config` field invariant, not user input.
/// - `invalid selector for ...` (`system.rs`) — the selector is a hardcoded
///   literal in the same expression, so failure means a programming error.
/// - the four `verify_npm_integrity` messages (`package_upgrade.rs`) — its only
///   caller tests `.is_err()` and re-raises the already-translated
///   `npm_integrity_failed`, so these are developer-facing detail. Its unit
///   tests also assert on the text.
const ENGLISH_ERROR_ALLOWLIST: &[&str] = &[
    "no versions collected",
    "io.js install requested but no version provided",
    "invalid selector for",
    "malformed integrity field",
    "unsupported integrity algorithm",
    "invalid base64 in integrity field",
    "sha512 mismatch",
];

/// Whether `haystack` contains `needle` followed immediately by an ASCII letter,
/// i.e. a prose literal rather than a `{}`-led format string.
fn has_prose_literal(haystack: &str, needle: &str) -> bool {
    match haystack.find(needle) {
        Some(i) => {
            let rest = &haystack[i + needle.len()..];
            rest.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        }
        None => false,
    }
}

/// No new hardcoded-English error context (P2-4 regression guard).
///
/// Line-based, so a message split across lines would slip through; every
/// current call site keeps the literal on one line.
#[test]
fn error_context_is_not_hardcoded_english() {
    const NEEDLES: [&str; 4] = [
        ".context(\"",
        "bail!(\"",
        "anyhow!(\"",
        "with_context(|| format!(\"",
    ];

    let offenders: Vec<String> = src_lines()
        .iter()
        .filter(|(_, _, text)| {
            let t = text.trim_start();
            // Comments legitimately quote these patterns.
            if t.starts_with("//") {
                return false;
            }
            // Already routed through i18n.
            if t.contains("T(") || t.contains("format_t(") {
                return false;
            }
            if ENGLISH_ERROR_ALLOWLIST.iter().any(|a| t.contains(a)) {
                return false;
            }
            NEEDLES.iter().any(|n| has_prose_literal(t, n))
        })
        .map(|(f, l, t)| format!("{f}:{l}: {}", t.trim()))
        .collect();

    assert!(
        offenders.is_empty(),
        "{} error-context string(s) are hardcoded English. Route them through \
         T()/format_t with a key in BOTH locales, or add them to \
         ENGLISH_ERROR_ALLOWLIST above with a reason:\n  {}",
        offenders.len(),
        offenders.join("\n  ")
    );
}
