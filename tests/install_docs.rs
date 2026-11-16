//! Release archives shipped both READMEs but no install guide, so the one
//! thing a user who downloaded `nvm-<version>-<os>-<arch>.tar.gz` actually
//! needed — how to get from that file to a working `nvm` — was only readable
//! on the GitHub blob view, buried under 27 KB of everything else.
//!
//! `INSTALL.md` / `INSTALL.ZH_CN.md` close that gap. `scripts/package.sh`
//! stages them into every archive, and they walk through extract → offline
//! `./install.sh` / `.\install.ps1` → Gatekeeper → first commands → upgrade →
//! uninstall, in both languages to match the READMEs they sit next to.
//!
//! These tests pin the three ways the guides can silently rot: they stop being
//! packaged (the archive loses its only instructions again), their steps stop
//! matching the installers they document, or their links start pointing at
//! README headings that no longer exist.

use std::fs;
use std::path::Path;

/// Each guide paired with the README it defers the long-form docs to.
const GUIDES: [(&str, &str); 2] = [
    ("INSTALL.md", "README.md"),
    ("INSTALL.ZH_CN.md", "README.ZH_CN.md"),
];

/// Steps every guide has to spell out, whatever language it is written in.
/// Each one is a command or a literal the installers print, so an assertion
/// cannot be satisfied by prose that merely mentions the topic.
const REQUIRED_STEPS: [&str; 14] = [
    "tar -xzf",
    "Expand-Archive",
    "./install.sh",
    "install.ps1",
    "Found bundled binary",
    "offline install",
    "NVM_INSTALL_DIR",
    "xattr -dr com.apple.quarantine",
    "Unblock-File",
    "Set-ExecutionPolicy",
    "nvm init",
    "nvm doctor",
    "nvm upgrade",
    "nvm uninstall --self",
];

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn read_root_file(name: &str) -> String {
    let path = root().join(name);
    fs::read_to_string(&path).unwrap_or_else(|_| panic!("{} must exist", name))
}

/// `content` from the `start` heading up to (excluding) the `end` heading.
fn section<'a>(content: &'a str, start: &str, end: &str) -> &'a str {
    let from = content
        .find(start)
        .unwrap_or_else(|| panic!("missing section heading {:?}", start));
    let to = content[from..]
        .find(end)
        .map(|offset| from + offset)
        .unwrap_or(content.len());
    &content[from..to]
}

/// GitHub's heading-to-anchor rule, as far as these READMEs need it: lowercase,
/// drop punctuation, spaces to hyphens. Fenced blocks are skipped so a shell
/// comment cannot masquerade as a heading.
fn anchors(content: &str) -> Vec<String> {
    let mut in_fence = false;
    let mut found = Vec::new();
    for line in content.lines() {
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence || !line.starts_with('#') {
            continue;
        }
        let text = line.trim_start_matches('#').trim();
        if text.is_empty() {
            continue;
        }
        let mut anchor = String::new();
        for ch in text.to_lowercase().chars() {
            if ch.is_alphanumeric() || ch == '-' || ch == '_' {
                anchor.push(ch);
            } else if ch == ' ' {
                anchor.push('-');
            }
        }
        found.push(anchor);
    }
    found
}

/// Every `](<target>#<anchor>)` link in `content` that points into `target`.
fn links_to(content: &str, target: &str) -> Vec<String> {
    let needle = format!("]({}#", target);
    let mut found = Vec::new();
    let mut rest = content;
    while let Some(offset) = rest.find(&needle) {
        rest = &rest[offset + needle.len()..];
        let end = rest.find(')').expect("markdown link must be closed");
        found.push(rest[..end].to_string());
        rest = &rest[end..];
    }
    found
}

#[test]
fn both_guides_exist_and_cross_link_each_other() {
    for (guide, _) in GUIDES {
        let content = read_root_file(guide);
        assert!(
            content.starts_with("# "),
            "{} must open with a title, not front matter",
            guide
        );
        assert!(
            content.lines().count() > 100,
            "{} is {} lines — too short to be a step-by-step guide",
            guide,
            content.lines().count()
        );
    }

    // A reader who landed in the wrong language has to be able to switch.
    assert!(
        read_root_file("INSTALL.md").contains("](INSTALL.ZH_CN.md)"),
        "INSTALL.md must link the Chinese guide"
    );
    assert!(
        read_root_file("INSTALL.ZH_CN.md").contains("](INSTALL.md)"),
        "INSTALL.ZH_CN.md must link the English guide"
    );
}

#[test]
fn guides_cover_every_step_from_archive_to_working_nvm() {
    for (guide, _) in GUIDES {
        let content = read_root_file(guide);
        for step in REQUIRED_STEPS {
            assert!(
                content.contains(step),
                "{} must document {:?} — archive users hit this step",
                guide,
                step
            );
        }
    }

    // The archive has no top-level folder; extracting in place scatters files
    // over the current directory. Both guides have to warn about it.
    assert!(
        read_root_file("INSTALL.md").contains("no top-level folder"),
        "INSTALL.md must warn that the archive has no top-level folder"
    );
    assert!(
        read_root_file("INSTALL.ZH_CN.md").contains("没有顶层目录"),
        "INSTALL.ZH_CN.md must warn that the archive has no top-level folder"
    );
}

#[test]
fn package_sh_stages_both_guides_into_every_archive() {
    let script = read_root_file("scripts/package.sh");
    for (guide, _) in GUIDES {
        let staged = format!("cp \"$PROJECT_ROOT/{0}\" \"$STAGE/{0}\"", guide);
        assert!(
            script.contains(&staged),
            "scripts/package.sh must stage {} or archives lose their install guide: missing {:?}",
            guide,
            staged
        );
    }

    let docs = read_root_file("scripts/README.md");
    for (guide, _) in GUIDES {
        assert!(
            docs.contains(guide),
            "scripts/README.md lists the archive contents and must mention {}",
            guide
        );
    }
}

#[test]
fn guides_only_link_readme_headings_that_exist() {
    for (guide, readme) in GUIDES {
        let content = read_root_file(guide);
        let target = read_root_file(readme);
        let links = links_to(&content, readme);
        assert!(
            !links.is_empty(),
            "{} must send readers to {} for the long-form docs",
            guide,
            readme
        );
        let available = anchors(&target);
        for anchor in links {
            assert!(
                available.contains(&anchor),
                "{} links to {}#{} which is not a heading in {}",
                guide,
                readme,
                anchor,
                readme
            );
        }
    }
}

#[test]
fn readmes_link_the_guide_where_downloads_are_documented() {
    let english_readme = read_root_file("README.md");
    let english = section(
        &english_readme,
        "### Download manually",
        "### Installation options",
    );
    assert!(
        english.contains("](INSTALL.md)"),
        "the manual-download section of README.md must link INSTALL.md"
    );

    let chinese_readme = read_root_file("README.ZH_CN.md");
    let chinese = section(&chinese_readme, "### 手动下载", "### 安装选项");
    assert!(
        chinese.contains("](INSTALL.ZH_CN.md)"),
        "the manual-download section of README.ZH_CN.md must link INSTALL.ZH_CN.md"
    );
}
