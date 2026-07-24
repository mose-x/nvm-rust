use anyhow::Result;
use colored::Colorize;
use std::fs;

use crate::config::Config;
use crate::system::{get_nvm_dir, URI};
use crate::utils::{display_width, lts_codename_to_major, pad_left, pad_right, parse_major};

mod info;
mod install;
mod listing;
mod migrate;
mod version_resolve;

pub use {info::*, install::*, listing::*, migrate::*};

// Compiled-once regexes used in version resolution. `Regex::new` is not free
// (~microseconds each); the `node-...` pattern is reused in 4 resolution
// helpers that can all fire during a single `nvm install`, so caching them
// avoids recompiling the same regex 4+ times per invocation.
lazy_static::lazy_static! {
    pub(crate) static ref NODE_VERSION_RE: regex::Regex =
        regex::Regex::new(r"node-(v[\d.]+)-").expect("node-version regex");
    pub(crate) static ref IOJS_VERSION_RE: regex::Regex =
        regex::Regex::new(r"iojs-(v[\d.]+)-").expect("iojs-version regex");
}

pub(crate) fn get_codename(version: &str) -> String {
    let map = lts_codename_to_major();
    get_codename_from_map(version, map)
}

/// Look up the LTS codename for `version` against a pre-built codename→major
/// map. Centralises the linear scan so callers that already hold a map
/// (e.g. `remote_versions` fetched one up-front for the whole 600+ version
/// loop) can reuse it instead of re-fetching `index.json` per version.
///
/// Generic over the key type so both the shipped `BTreeMap<&'static str, u32>`
/// (from `lts_codename_to_major`) and the remote-augmented
/// `BTreeMap<String, u32>` (from `lts_codename_to_major_with_remote`) work
/// without forcing the caller to reallocate keys.
pub(crate) fn get_codename_from_map<K: AsRef<str>>(
    version: &str,
    map: &std::collections::BTreeMap<K, u32>,
) -> String {
    parse_major(version)
        .and_then(|m| {
            for (name, major) in map {
                if *major == m {
                    return Some(name.as_ref().to_string());
                }
            }
            None
        })
        .unwrap_or_else(|| "-".to_string())
}

/// Render a beautiful bordered table.
/// columns: (header_text, alignment) where alignment is 0=left, 1=right, 2=center
/// header can be "" to indicate no header text for that column
pub(crate) fn render_table(title: &str, columns: &[(&str, u8)], rows: &[Vec<String>]) {
    let n_cols = columns.len();
    let padding = 1;

    // Calculate column widths based on content
    let mut col_widths: Vec<usize> = columns
        .iter()
        .map(|(h, _)| if h.is_empty() { 0 } else { display_width(h) })
        .collect();
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            if i < n_cols {
                let w = display_width(cell);
                if w > col_widths[i] {
                    col_widths[i] = w;
                }
            }
        }
    }
    // Ensure minimum width for marker column
    for w in col_widths.iter_mut() {
        if *w == 0 {
            *w = 1;
        }
    }

    let inner_total: usize = col_widths.iter().sum::<usize>() + padding * 2 * n_cols + (n_cols - 1);
    let title_width = display_width(title);
    let total_width = inner_total.max(title_width + padding * 2);

    // Build a separator line with custom corner/join chars
    let sep_line = |left: char, mid: char, right: char, fill: char| -> String {
        let mut s = String::new();
        s.push(left);
        for (i, w) in col_widths.iter().enumerate() {
            s.push_str(&fill.to_string().repeat(w + padding * 2));
            if i < n_cols - 1 {
                s.push(mid);
            }
        }
        s.push(right);
        s
    };

    // Top border
    println!("  ╭{}╮", "─".repeat(total_width));

    // Title row
    if !title.is_empty() {
        let title_str = format!(" {} ", title.bold().cyan());
        let tw = display_width(&title_str);
        let right_pad = total_width.saturating_sub(tw);
        println!("  │{}{}│", title_str, " ".repeat(right_pad));
        println!("  {}", sep_line('├', '┼', '┤', '─'));
    }

    // Header row (skip empty header columns)
    let has_any_header = columns.iter().any(|(h, _)| !h.is_empty());
    if has_any_header {
        let mut header = String::from("  │");
        for (i, (h, _)) in columns.iter().enumerate() {
            let content = if h.is_empty() {
                " ".repeat(col_widths[i])
            } else {
                h.bold().cyan().to_string()
            };
            let padded = format!("{}{}{}", " ".repeat(padding), content, " ".repeat(padding));
            let padded = pad_right(&padded, col_widths[i] + padding * 2);
            header.push_str(&padded);
            if i < n_cols - 1 {
                header.push('│');
            }
        }
        header.push('│');
        println!("{}", header);
        println!("  {}", sep_line('├', '┼', '┤', '─'));
    }

    // Data rows
    for row in rows {
        let mut line = String::from("  │");
        for (i, cell) in row.iter().enumerate() {
            let align = columns.get(i).map(|(_, a)| *a).unwrap_or(0);
            let content = match align {
                1 => pad_left(cell, col_widths[i]),
                2 => {
                    let w = col_widths[i];
                    let dw = display_width(cell);
                    let lp = (w - dw) / 2;
                    let rp = w - dw - lp;
                    format!("{}{}{}", " ".repeat(lp), cell, " ".repeat(rp))
                }
                _ => pad_right(cell, col_widths[i]),
            };
            let padded = format!("{}{}{}", " ".repeat(padding), content, " ".repeat(padding));
            let padded = pad_right(&padded, col_widths[i] + padding * 2);
            line.push_str(&padded);
            if i < n_cols - 1 {
                line.push('│');
            }
        }
        line.push('│');
        println!("{}", line);
    }

    // Bottom border
    println!("  ╰{}╯", "─".repeat(total_width));
}

/// Compare two version strings by semantic version (major.minor.patch).
/// Returns greater if a is newer than b. Delegates to `utils::compare_semver`
/// so all version comparisons share one implementation.
pub(crate) fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    crate::utils::compare_semver(a, b)
}

pub(crate) fn get_current_version() -> Result<Option<String>> {
    let nvm_dir = get_nvm_dir();
    let current_file = nvm_dir.join("current");

    // Read directly and handle NotFound rather than `exists()` + `read_to_string`:
    // the two-step form is a TOCTOU race — the file could be removed (or
    // replaced by a non-file) between the exists check and the read. A single
    // read that maps NotFound to None is both faster and race-free.
    match fs::read_to_string(&current_file) {
        Ok(content) => {
            let version = content.trim();
            if version.is_empty() {
                Ok(None)
            } else {
                Ok(Some(version.to_string()))
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub(crate) fn get_base_url(config: &Config) -> &str {
    config.mirror.as_deref().unwrap_or(URI)
}
