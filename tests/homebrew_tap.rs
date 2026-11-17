//! The Homebrew formula lives in `mose-x/homebrew-tap`, not in this crate.
//!
//! Two copies used to sit in `homebrew/`, and both drifted four minor versions
//! behind the crate (still pointing at v2.0.0 while the release was v2.3.6);
//! the source one even shipped `sha256 "TO_BE_FILLED"`, which makes
//! `brew install` fail outright. No test in this crate can keep such a copy
//! current, because the checksums only exist once the release assets are
//! published — a copy checked in here is stale by construction. The tap removes
//! the root cause instead: a scheduled workflow reads the latest release,
//! rewrites the urls and recomputes every `sha256` from the published assets.
//!
//! These tests pin that decision down. They fail if a formula copy comes back,
//! and if either README stops documenting an install path that actually works:
//! `brew trust mose-x/tap` first, then `brew tap`, then the fully qualified
//! `mose-x/tap/nvm-rust`. Homebrew 7 refuses every entry of an untrusted tap,
//! the fully qualified name included, so trusting is no longer just a hint for
//! short-name users and its position in the block matters.

use std::fs;
use std::path::Path;

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

/// The Homebrew section of each README, paired with the file it came from.
fn homebrew_sections() -> [(&'static str, String); 2] {
    [
        (
            "README.md",
            section(
                &read_root_file("README.md"),
                "### Homebrew",
                "### Build from source",
            )
            .to_string(),
        ),
        (
            "README.ZH_CN.md",
            section(
                &read_root_file("README.ZH_CN.md"),
                "### Homebrew",
                "### 从源码编译",
            )
            .to_string(),
        ),
    ]
}

/// The first fenced ```bash block of a section, i.e. the commands a user copies.
fn bash_block(name: &str, section: &str) -> String {
    let after_fence = section
        .split("```bash")
        .nth(1)
        .unwrap_or_else(|| panic!("{name}: the Homebrew section has no ```bash block"));
    after_fence
        .split("```")
        .next()
        .unwrap_or_else(|| panic!("{name}: unterminated ```bash block"))
        .to_string()
}

#[test]
fn no_formula_copies_live_in_this_crate() {
    for name in [
        "homebrew",
        "homebrew/nvm-rust.rb",
        "homebrew/nvm-rust-binary.rb",
    ] {
        assert!(
            !root().join(name).exists(),
            "{name} must not come back: the formula lives in mose-x/homebrew-tap, \
             which recomputes the version and every sha256 from the published \
             release assets. A copy in this repo can only go stale."
        );
    }
}

#[test]
fn readmes_install_from_the_tap_by_its_full_name() {
    for (name, section) in homebrew_sections() {
        let block = bash_block(name, &section);
        assert!(
            block.contains("brew tap mose-x/tap"),
            "{name} must tap mose-x/tap before installing: {block}"
        );
        assert!(
            block.contains("brew install mose-x/tap/nvm-rust"),
            "{name} must install by the fully qualified name, which stays \
             unambiguous once the tap is trusted: {block}"
        );
        // The short name only resolves once the tap is both tapped and trusted,
        // and stays ambiguous against any other tap shipping an `nvm-rust`. The
        // fully qualified form has neither problem, so that is what gets
        // documented.
        for line in block.lines() {
            assert_ne!(
                line.trim(),
                "brew install nvm-rust",
                "{name} documents the short name, which needs a trusted and \
                 tapped mose-x/tap and is ambiguous besides"
            );
        }
    }
}

#[test]
fn readmes_explain_the_homebrew_trust_requirement() {
    for (name, section) in homebrew_sections() {
        assert!(
            section.contains("brew trust mose-x/tap"),
            "{name} must tell readers to run `brew trust mose-x/tap`"
        );
        assert!(
            section.contains("brew untrust mose-x/tap"),
            "{name} must say how to undo the trust"
        );
        assert!(
            section.contains("mose-x/homebrew-tap"),
            "{name} must link the tap repo that now owns the formula"
        );
        // Homebrew 7 requires trust for the fully qualified name too, and the
        // record is per user rather than per tap checkout. Both facts have to be
        // spelled out, or readers assume one `brew trust` covers every install
        // form and every machine.
        for fact in ["Homebrew 7", "trust.json"] {
            assert!(
                section.contains(fact),
                "{name} must mention {fact:?} so the trust requirement and its \
                 per-user scope stay explicit"
            );
        }
    }
}

/// `brew trust` is idempotent and does not need the tap to exist yet, so trusting
/// first is the only ordering that is right both on a machine that never saw
/// mose-x/tap and on one that already tapped it.
#[test]
fn readmes_trust_the_tap_before_installing() {
    for (name, section) in homebrew_sections() {
        let block = bash_block(name, &section);
        let install_line = block
            .lines()
            .position(|line| line.trim() == "brew install mose-x/tap/nvm-rust");
        let trust_line = block
            .lines()
            .position(|line| line.trim() == "brew trust mose-x/tap")
            .unwrap_or_else(|| {
                panic!("{name} must run `brew trust mose-x/tap` in its install block: {block}")
            });
        if let Some(install_line) = install_line {
            assert!(
                trust_line < install_line,
                "{name} must run `brew trust mose-x/tap` before installing: {block}"
            );
        }
    }
}

/// The archive install guides name Homebrew as the no-archive alternative, so
/// they inherit the trap the READMEs document: under Homebrew 7 a bare
/// `brew install mose-x/tap/nvm-rust` is refused until the tap is trusted.
///
/// The anchors are literal because GitHub slugs the two headings differently —
/// it drops the full-width parentheses of the Chinese one without leaving a
/// hyphen behind, so `homebrew-macos--linux` and `homebrewmacos--linux` are
/// both correct and neither is guessable from the other. `install_docs.rs`
/// re-derives every guide link from the README headings, so a renamed heading
/// fails there.
#[test]
fn install_guides_trust_the_tap_too() {
    for (guide, readme, anchor) in [
        ("INSTALL.md", "README.md", "homebrew-macos--linux"),
        (
            "INSTALL.ZH_CN.md",
            "README.ZH_CN.md",
            "homebrewmacos--linux",
        ),
    ] {
        let content = read_root_file(guide);
        let trust = content.find("brew trust mose-x/tap").unwrap_or_else(|| {
            panic!("{guide} offers Homebrew as an alternative, so it must give the trust step")
        });
        let install = content
            .find("brew install mose-x/tap/nvm-rust")
            .unwrap_or_else(|| panic!("{guide} must install by the fully qualified name"));
        assert!(
            trust < install,
            "{guide} must run `brew trust mose-x/tap` before installing"
        );
        assert!(
            content.contains(&format!("]({readme}#{anchor})")),
            "{guide} must point at the Homebrew section of {readme}, which is \
             where the trust requirement and its per-user scope are explained"
        );
    }
}
