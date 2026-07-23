// Build script: scan `locales/*.toml` at compile time and emit a generated
// Rust source file that the i18n module includes via `include!`.
//
// This is what makes the i18n system "convention over configuration": to
// add a new language, drop a `xx.toml` into `locales/` and rebuild. No
// source edits required.
//
// TOML convention
// ---------------
// Each locale file is a flat `key = "value"` table of translation strings,
// plus an optional `[_meta]` table with two fields:
//
//   [_meta]
//   display_name = "日本語"        # shown in `nvm language` listing
//   aliases      = ["ja", "jpn"]   # accepted by `nvm language <alias>`
//
// The file's stem (e.g. `jp` from `jp.toml`) is the canonical lang code.
// If `_meta` is missing, display_name defaults to the code and aliases is
// empty. `en.toml` is always present and is the fallback baseline.

use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    let locales_dir = Path::new("locales");
    println!("cargo:rerun-if-changed=locales");

    // Discover every `*.toml` in locales/, sorted for deterministic output.
    let mut files: Vec<PathBuf> = match fs::read_dir(locales_dir) {
        Ok(rd) => rd.filter_map(|e| e.ok().map(|e| e.path())).collect(),
        Err(_) => Vec::new(),
    };
    files.sort();

    // Parse each file once to pull out _meta (display_name + aliases).
    // The string table itself is embedded with include_str! at use sites,
    // not duplicated here — we only need metadata for codegen.
    let mut entries: Vec<LocaleEntry> = Vec::new();
    for path in &files {
        if path.extension().and_then(|s| s.to_str()) != Some("toml") {
            continue;
        }
        let code = match path.file_stem().and_then(|s| s.to_str()) {
            Some(s) => s.to_string(),
            None => continue,
        };
        let text = match fs::read_to_string(path) {
            Ok(t) => t,
            Err(_) => continue,
        };
        let meta = parse_meta(&text, &code);
        // `include_str!` resolves paths relative to the file that contains
        // the macro invocation — which is `OUT_DIR/locales_generated.rs`,
        // not the crate root. Emit absolute paths so the include works no
        // matter where the generated file lands.
        let abs = fs::canonicalize(path).unwrap_or_else(|_| path.clone());
        entries.push(LocaleEntry {
            code,
            display_name: meta.display_name,
            aliases: meta.aliases,
            abs_path: abs.to_string_lossy().replace('\\', "/"),
        });
    }

    if !entries.iter().any(|e| e.code == "en") {
        panic!(
            "build.rs: locales/en.toml is required as the fallback baseline, \
             but it was not found"
        );
    }

    let generated = generate_source(&entries);
    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR not set by cargo");
    let out_path = Path::new(&out_dir).join("locales_generated.rs");
    fs::write(&out_path, generated).expect("write locales_generated.rs");
}

struct LocaleEntry {
    code: String,
    display_name: String,
    aliases: Vec<String>,
    abs_path: String,
}

struct Meta {
    display_name: String,
    aliases: Vec<String>,
}

/// Parse the `[_meta]` table from a locale TOML without pulling in a full
/// TOML dependency in build.rs (we keep build-deps minimal). Only the two
/// fields we care about are read; everything else is ignored.
fn parse_meta(text: &str, fallback_code: &str) -> Meta {
    let mut display_name = fallback_code.to_string();
    let mut aliases: Vec<String> = Vec::new();

    let mut in_meta = false;
    for raw in text.lines() {
        let line = raw.trim();
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        if line.starts_with('[') {
            in_meta = line == "[_meta]";
            continue;
        }
        if !in_meta {
            continue;
        }
        if let Some(rest) = line.strip_prefix("display_name") {
            if let Some(v) = extract_string_value(rest) {
                display_name = v;
            }
        } else if let Some(rest) = line.strip_prefix("aliases") {
            aliases = extract_string_array(rest);
        }
    }

    Meta {
        display_name,
        aliases,
    }
}

/// Given the part of a line after the key (e.g. `= "English"`), extract the
/// quoted string value. Returns None if it cannot be parsed.
fn extract_string_value(rest: &str) -> Option<String> {
    let rest = rest.trim_start();
    let rest = rest.strip_prefix('=')?.trim();
    let rest = rest.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

/// Given the part of a line after `aliases` (e.g. `= ["ja", "jpn"]`),
/// extract every quoted string inside the brackets.
fn extract_string_array(rest: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = rest.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'"' {
            if let Some(end) = rest[i + 1..].find('"') {
                out.push(rest[i + 1..i + 1 + end].to_string());
                i = i + 1 + end + 1;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// Emit a Rust source file containing:
///   - `LANG_CODES: &[&str]` — every lang code, en first
///   - `LANG_DISPLAY_NAMES: &[(&str, &str)]` — (code, display_name)
///   - `LANG_ALIASES: &[(&str, &str)]` — (alias, canonical_code)
///   - `LANG_STRINGS: &[(&str, &str)]` — (code, include_str!("locales/xx.toml"))
fn generate_source(entries: &[LocaleEntry]) -> String {
    let mut out = String::new();
    out.push_str("// AUTO-GENERATED by build.rs. Do not edit by hand.\n");
    out.push_str("// Re-generated on every build via `cargo:rerun-if-changed=locales`.\n\n");

    // en first, then the rest alphabetically for deterministic ordering.
    let mut ordered: Vec<&LocaleEntry> = entries.iter().collect();
    ordered.sort_by(|a, b| {
        if a.code == "en" {
            std::cmp::Ordering::Less
        } else if b.code == "en" {
            std::cmp::Ordering::Greater
        } else {
            a.code.cmp(&b.code)
        }
    });

    // LANG_CODES
    out.push_str("pub static LANG_CODES: &[&str] = &[\n");
    for e in &ordered {
        out.push_str(&format!("    \"{}\",\n", escape_rust_str(&e.code)));
    }
    out.push_str("];\n\n");

    // LANG_DISPLAY_NAMES
    out.push_str("pub static LANG_DISPLAY_NAMES: &[(&str, &str)] = &[\n");
    for e in &ordered {
        out.push_str(&format!(
            "    (\"{}\", \"{}\"),\n",
            escape_rust_str(&e.code),
            escape_rust_str(&e.display_name)
        ));
    }
    out.push_str("];\n\n");

    // LANG_ALIASES
    out.push_str("pub static LANG_ALIASES: &[(&str, &str)] = &[\n");
    for e in &ordered {
        for alias in &e.aliases {
            out.push_str(&format!(
                "    (\"{}\", \"{}\"),\n",
                escape_rust_str(alias),
                escape_rust_str(&e.code)
            ));
        }
    }
    out.push_str("];\n\n");

    // LANG_STRINGS — embed the TOML file contents at compile time.
    out.push_str("pub static LANG_STRINGS: &[(&str, &str)] = &[\n");
    for e in &ordered {
        out.push_str(&format!(
            "    (\"{}\", include_str!(\"{}\")),\n",
            escape_rust_str(&e.code),
            escape_rust_str(&e.abs_path)
        ));
    }
    out.push_str("];\n");

    out
}

/// Escape a string for safe embedding inside a Rust `"..."` literal.
fn escape_rust_str(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}
