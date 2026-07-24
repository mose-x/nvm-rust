use anyhow::{Context, Result};
use colored::Colorize;
use std::fs;

use super::{
    compare_versions, get_base_url, get_codename, get_codename_from_map, get_current_version,
    render_table,
};
use crate::config::{load_config, resolve_alias};
use crate::i18n::{format_t, T};
use crate::system::{get_cache_dir, get_nvm_dir, get_tags};
use crate::utils::{get_installed_versions, is_lts_version, parse_major};

fn version_codename_colored(version: &str) -> String {
    let code = get_codename(version);
    if code == "-" {
        "-".dimmed().to_string()
    } else {
        code.magenta().bold().to_string()
    }
}

fn version_lts_colored(version: &str) -> String {
    if is_lts_version(version) {
        T("lts_badge").green().to_string()
    } else {
        "".to_string()
    }
}

pub fn uninstall(version: &str) -> Result<()> {
    let resolved = resolve_alias(version)?;
    let nvm_dir = get_nvm_dir();
    let version_dir = nvm_dir.join(&resolved);

    // Serialize against concurrent install/uninstall: a concurrent install
    // could repopulate `version_dir` between our exists-check and
    // `remove_dir_all`, or a concurrent uninstall could remove it first and
    // leave us operating on a stale path. The lock is held across the whole
    // removal + `current` cleanup.
    let _nvm_lock = crate::utils::acquire_nvm_lock(&nvm_dir)?;

    if !version_dir.exists() {
        anyhow::bail!(
            "{}",
            format_t("not_installed", std::slice::from_ref(&resolved))
        );
    }

    let is_current_active = match get_current_version()? {
        Some(current) if current == resolved => {
            println!(
                "{} {}",
                "⚠".yellow().bold(),
                T("uninstall_warning").yellow()
            );
            true
        }
        _ => false,
    };

    print!(
        "{} {} ",
        "▶".red().bold(),
        T("uninstalling_label").red().bold()
    );
    print!("{}", resolved.white().bold());
    fs::remove_dir_all(&version_dir).context(T("uninstall_failed"))?;
    println!(" {}", "✓".green().bold());

    // Clear `current` if we just removed the active version, so subsequent
    // `nvm current` / `nvm ls` don't point at a deleted directory.
    if is_current_active {
        let current_file = nvm_dir.join("current");
        let _ = fs::remove_file(&current_file);
    }

    Ok(())
}

pub fn list_versions() -> Result<()> {
    let current = get_current_version()?;
    let mut versions = get_installed_versions();

    if versions.is_empty() {
        println!();
        println!(
            "  {} {}",
            "ℹ".cyan().bold(),
            T("no_installed_versions").cyan()
        );
        println!(
            "  {} {}",
            "→".dimmed(),
            format_t("run_get_started", &["nvm install <version>".to_string()]).dimmed()
        );
        println!();
        return Ok(());
    }

    // Sort descending (newest first) using semantic version comparison
    versions.sort_by(|a, b| compare_versions(b, a));

    let has_iojs = versions.iter().any(|v| v.starts_with("iojs-"));

    let columns: &[(&str, u8)] = &[
        (&T("version"), 0),
        (&T("type_col"), 0),
        (&T("lts_col"), 0),
        (&T("codename"), 0),
    ];

    let mut rows: Vec<Vec<String>> = Vec::new();
    for v in versions.iter() {
        let is_current = current.as_ref().map(|c| c == v).unwrap_or(false);
        let is_iojs = v.starts_with("iojs-");
        let version_str = if is_current {
            format!("{} {}", "●".green().bold(), v.green().bold())
        } else {
            format!("  {}", v.white())
        };
        let type_str = if is_iojs {
            "io.js".blue().to_string()
        } else {
            "node".dimmed().to_string()
        };
        let lts_str = if is_iojs {
            "-".dimmed().to_string()
        } else {
            version_lts_colored(v)
        };
        let codename_str = if is_iojs {
            "-".dimmed().to_string()
        } else {
            version_codename_colored(v)
        };

        rows.push(vec![version_str, type_str, lts_str, codename_str]);
    }

    let title = if has_iojs {
        T("installed_all_title")
    } else {
        T("installed_versions_title")
    };

    println!();
    render_table(&title, columns, &rows);
    println!();

    // Summary
    print!("  ");
    print!(
        "{}",
        format_t("installed", &[versions.len().to_string()])
            .white()
            .bold()
    );
    print!("    ");
    if let Some(curr) = &current {
        print!("{}", T("active").replace("{0}", curr).green().bold());
    } else {
        print!("{}", T("no_active").dimmed());
    }
    println!();
    println!();

    Ok(())
}

pub fn remote_versions(
    lts_only: bool,
    lts_old: bool,
    filter: Option<&str>,
    sort: Option<&str>,
    page: Option<usize>,
) -> Result<()> {
    let config = load_config()?;
    let base_url = get_base_url(&config);

    print!("  {} {}", "⟳".cyan().bold(), T("fetching_remote").cyan());
    let tags = get_tags(base_url.to_string());
    println!(" {}", "✓".green().bold());

    let mut all_versions: Vec<(String, bool, String)> = Vec::new();

    // Fetch the codename→major map ONCE for the whole version list. The
    // previous loop body called `get_codename_with_remote` per version,
    // which re-fetched `index.json` and rebuilt the BTreeMap on every
    // iteration — ~600 HTTP GETs + ~600 BTreeMap allocations for a single
    // `nvm ls-remote`. The map is read-only and identical for every
    // version on the same mirror, so one fetch covers all of them.
    let codename_map = crate::utils::lts_codename_to_major_with_remote(base_url);

    for tag in tags {
        if tag.starts_with('v') && tag.ends_with('/') {
            let version = tag.trim_end_matches('/').to_string();
            let is_lts = is_lts_version(&version);
            let codename = get_codename_from_map(&version, &codename_map);
            all_versions.push((version, is_lts, codename));
        }
    }

    // Sort descending by semantic version (newest first) or ascending if requested
    let ascending = sort.map(|s| s.to_lowercase() == "asc").unwrap_or(false);
    if ascending {
        all_versions.sort_by(|a, b| compare_versions(&a.0, &b.0));
    } else {
        all_versions.sort_by(|a, b| compare_versions(&b.0, &a.0));
    }

    // Apply filters. `into_iter` moves matching tuples out of `all_versions`
    // instead of cloning every (version, lts, codename) triple — the no-filter
    // branch just moves the whole vec with zero copying. `all_versions` is not
    // used after this point.
    let mut filtered: Vec<(String, bool, String)> = if lts_only {
        all_versions
            .into_iter()
            .filter(|(_, lts, _)| *lts)
            .collect()
    } else if lts_old {
        // Older LTS lines (major <= 18): v4 argon, v6 boron, v8 carbon,
        // v10 dubnium, v12 erbium, v14 fermium, v16 gallium, v18 hydrogen.
        all_versions
            .into_iter()
            .filter(|(_, lts, _)| *lts)
            .filter(|(v, _, _)| parse_major(v).is_some_and(|m| m <= 18))
            .collect()
    } else {
        all_versions
    };

    // Apply version filter if specified
    if let Some(f) = filter {
        let f_lower = f.to_lowercase();
        filtered.retain(|(v, _, _)| v.to_lowercase().contains(&f_lower));
    }

    if filtered.is_empty() {
        println!();
        println!("  {} {}", "ℹ".cyan().bold(), T("no_versions_found").cyan());
        return Ok(());
    }

    let page_size = 20;
    let total_count = filtered.len();
    let total_pages = total_count.div_ceil(page_size);
    let page_num = page.unwrap_or(1).max(1).min(total_pages.max(1));
    let start = (page_num - 1) * page_size;
    let end = (start + page_size).min(total_count);

    let page_items = &filtered[start..end];

    // Build table data
    let columns: &[(&str, u8)] = &[
        ("#", 1),
        (&T("version"), 0),
        (&T("lts_col"), 0),
        (&T("codename"), 0),
    ];

    let mut rows: Vec<Vec<String>> = Vec::new();
    for (i, (v, _is_lts, _codename)) in page_items.iter().enumerate() {
        let idx = (start + i + 1).to_string();
        let idx_str = idx.dimmed().to_string();
        let version_str = v.white().to_string();
        let lts_str = version_lts_colored(v);
        let codename_str = version_codename_colored(v);
        rows.push(vec![idx_str, version_str, lts_str, codename_str]);
    }

    let title = if lts_only {
        T("remote_lts_title")
    } else if lts_old {
        T("remote_lts_old_title")
    } else {
        T("remote_title")
    };

    println!();
    render_table(&title, columns, &rows);
    println!();

    // Pagination info
    print!("  ");
    print!(
        "{}",
        format_t(
            "page_info",
            &[
                page_num.to_string(),
                total_pages.max(1).to_string(),
                (start + 1).to_string(),
                end.to_string(),
                total_count.to_string(),
            ]
        )
        .cyan()
    );
    println!();

    // Navigation hints
    let mut nav_parts: Vec<String> = Vec::new();
    if page_num > 1 {
        nav_parts.push(
            format_t("prev_page", &[(page_num - 1).to_string()])
                .yellow()
                .to_string(),
        );
    }
    if page_num < total_pages {
        nav_parts.push(
            format_t("next_page", &[(page_num + 1).to_string()])
                .yellow()
                .to_string(),
        );
    }
    if !nav_parts.is_empty() {
        println!("  {}", nav_parts.join("    "));
    }
    println!();

    Ok(())
}

pub fn cache_dir() -> Result<()> {
    let cache_dir = get_cache_dir();
    println!("{}", cache_dir.display().to_string().white().bold());
    Ok(())
}

pub fn cmd_dir() -> Result<()> {
    let nvm_dir = get_nvm_dir();
    let home = crate::system::get_home_dir();
    let dot_nvm = std::path::PathBuf::from(&home).join(".nvm.rust");

    println!("{}", T("nvm_dir_title").cyan().bold());
    println!(
        "  {} {}",
        T("nvm_dir_path").white(),
        nvm_dir.display().to_string().green()
    );
    println!();
    println!("{}", T("nvm_home_title").cyan().bold());
    println!(
        "  {} {}",
        T("nvm_home_path").white(),
        dot_nvm.display().to_string().green()
    );
    Ok(())
}

pub fn cache_list() -> Result<()> {
    use crate::download::list_cached_files;

    let files = list_cached_files()?;

    if files.is_empty() {
        println!();
        println!("  {} {}", "ℹ".cyan().bold(), T("cache_empty").cyan());
        println!();
        return Ok(());
    }

    let total_bytes: u64 = files.iter().map(|(_, s)| s).sum();

    let columns: &[(&str, u8)] = &[(&T("file_col"), 0), (&T("size_col"), 1)];

    let mut rows: Vec<Vec<String>> = Vec::new();
    for (name, size) in &files {
        rows.push(vec![name.clone(), format_size(*size)]);
    }

    println!();
    let cache_title = T("cache_title");
    render_table(&cache_title, columns, &rows);
    println!();
    print!("  ");
    print!(
        "{}",
        format_t("cache_files", &[files.len().to_string()]).dimmed()
    );
    print!("    ");
    print!(
        "{}",
        T("cache_total")
            .replace("{0}", &format_size(total_bytes))
            .cyan()
    );
    println!();
    println!();

    Ok(())
}

pub fn cache_clear() -> Result<()> {
    use crate::download::clear_cache;

    let cleared = clear_cache()?;
    println!();
    println!(
        "  {} {}",
        "✓".green().bold(),
        format_t("cache_cleared", &[format_size(cleared)]).green()
    );
    println!();
    Ok(())
}

// ---------------------------------------------------------------------------
// Uninstall LTS (P3-7)
// ---------------------------------------------------------------------------

pub fn uninstall_latest_lts() -> Result<()> {
    // Pick the newest LTS that is *actually installed locally*, rather than the
    // latest LTS reported by the remote registry. This works offline and matches
    // user expectation: "uninstall the latest LTS I have".
    let versions = get_installed_versions();
    if versions.is_empty() {
        anyhow::bail!("{}", T("no_installed_versions"));
    }

    // Reuse the existing LTS detection (utils::is_lts_version) so the notion
    // of "LTS" stays consistent across the codebase.
    let mut lts: Vec<String> = versions.into_iter().filter(|v| is_lts_version(v)).collect();
    if lts.is_empty() {
        anyhow::bail!("{}", T("no_installed_lts"));
    }
    lts.sort_by(|a, b| compare_versions(b, a));
    uninstall(&lts[0])
}

/// Uninstall the newest installed version (any kind, including io.js).
/// Mirrors `uninstall --lts` but without the LTS filter.
pub fn uninstall_latest() -> Result<()> {
    let versions = get_installed_versions();
    if versions.is_empty() {
        anyhow::bail!("{}", T("no_installed_versions"));
    }
    let mut all: Vec<String> = versions;
    all.sort_by(|a, b| compare_versions(b, a));
    uninstall(&all[0])
}

// ---------------------------------------------------------------------------
// Helper
// ---------------------------------------------------------------------------

fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}
