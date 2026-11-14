//! Content checks for the two Homebrew formulas in `homebrew/`.
//!
//! Both formulas drifted four minor versions behind the crate (they still
//! pointed at v2.0.0 while the release was v2.3.6), and the source formula
//! shipped `sha256 "TO_BE_FILLED"`, which makes `brew install` fail outright.
//! Nothing caught either problem because no test read these files.
//!
//! Deliberately *not* asserted: that the formula version equals the version in
//! `Cargo.toml`. The binary formula's checksums can only be computed from the
//! published release assets, so the formulas are necessarily updated in a
//! commit *after* the version bump and the tag. Pinning them to the manifest
//! would make that ordering impossible. What is asserted instead is internal
//! consistency — every URL must agree with the version the same file declares
//! — plus digest-shaped checksums, which is what actually broke.
//!
//! The consequence is worth stating plainly: a formula that *lags* the crate
//! (the "v2.0.0 while the release was v2.3.6" half of the bug above) cannot be
//! caught offline, because nothing in the working tree records which releases
//! exist. A formula that *leads* the crate can be, and is — see
//! `formula_version_never_leads_the_crate_version`. Keeping the lagging half
//! from recurring is a release-checklist item, not a test.

use std::fs;
use std::path::Path;

const SOURCE_FORMULA: &str = "homebrew/nvm-rust.rb";
const BINARY_FORMULA: &str = "homebrew/nvm-rust-binary.rb";

fn read_formula(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(name);
    fs::read_to_string(&path).unwrap_or_else(|_| panic!("{name} must exist"))
}

/// The value of the formula-level `version "x.y.z"` declaration.
fn declared_version(content: &str) -> String {
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("version \"") {
            return rest
                .strip_suffix('"')
                .expect("version declaration must be closed")
                .to_string();
        }
    }
    panic!("formula must declare a version");
}

/// Every `key "value"` pair for `key`, in file order.
fn quoted_values(content: &str, key: &str) -> Vec<String> {
    let prefix = format!("{key} \"");
    content
        .lines()
        .filter_map(|line| line.trim().strip_prefix(prefix.as_str()))
        .filter_map(|rest| rest.split('"').next())
        .map(str::to_string)
        .collect()
}

fn is_lowercase_hex_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
}

/// The crate version from `Cargo.toml`, as three numeric components.
fn crate_version() -> (u64, u64, u64) {
    let manifest = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"))
        .expect("Cargo.toml must exist");
    let raw = manifest
        .lines()
        .filter_map(|line| line.trim().strip_prefix("version = \""))
        .next()
        .expect("Cargo.toml must declare a package version")
        .trim_end_matches('"')
        .to_string();
    split_version(&raw)
}

fn split_version(version: &str) -> (u64, u64, u64) {
    let parts: Vec<u64> = version
        .split('.')
        .map(|part| {
            part.parse()
                .unwrap_or_else(|_| panic!("version {version:?} must be numeric x.y.z"))
        })
        .collect();
    assert_eq!(parts.len(), 3, "version {version:?} must be x.y.z");
    (parts[0], parts[1], parts[2])
}

#[test]
fn formula_version_never_leads_the_crate_version() {
    // A formula may lag (its checksums need the published release), but it must
    // never advertise a version the crate has not reached: that download URL
    // 404s, and it means someone bumped the formula without bumping — or before
    // bumping — the manifest.
    let manifest = crate_version();
    for name in [SOURCE_FORMULA, BINARY_FORMULA] {
        let version = declared_version(&read_formula(name));
        assert!(
            split_version(&version) <= manifest,
            "{name} declares v{version}, ahead of the crate's v{manifest:?}"
        );
    }
}

#[test]
fn both_formulas_declare_a_plain_semver_version() {
    for name in [SOURCE_FORMULA, BINARY_FORMULA] {
        let version = declared_version(&read_formula(name));
        let parts: Vec<&str> = version.split('.').collect();
        assert_eq!(
            parts.len(),
            3,
            "{name}: version must be x.y.z, got {version:?}"
        );
        for part in parts {
            assert!(
                !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()),
                "{name}: version component {part:?} must be numeric in {version:?}"
            );
        }
    }
}

#[test]
fn every_sha256_is_a_real_digest_not_a_placeholder() {
    for name in [SOURCE_FORMULA, BINARY_FORMULA] {
        let digests = quoted_values(&read_formula(name), "sha256");
        assert!(
            !digests.is_empty(),
            "{name} must declare at least one sha256"
        );
        for digest in &digests {
            assert!(
                is_lowercase_hex_digest(digest),
                "{name}: sha256 {digest:?} is not a 64-char lowercase hex digest \
                 (a placeholder here breaks `brew install`)"
            );
        }
    }
}

#[test]
fn every_download_url_matches_the_declared_version() {
    for name in [SOURCE_FORMULA, BINARY_FORMULA] {
        let content = read_formula(name);
        let version = declared_version(&content);
        let urls = quoted_values(&content, "url");
        let versioned: Vec<&String> = urls
            .iter()
            .filter(|url| url.contains("releases/download") || url.contains("archive/refs/tags"))
            .collect();
        assert!(
            !versioned.is_empty(),
            "{name} must declare at least one versioned download URL"
        );
        for url in versioned {
            assert!(
                url.contains(&format!("/v{version}")),
                "{name}: URL {url} does not point at the declared version v{version}"
            );
        }
    }
}

#[test]
fn binary_formula_covers_every_supported_target_with_its_own_digest() {
    let content = read_formula(BINARY_FORMULA);
    let version = declared_version(&content);
    let urls = quoted_values(&content, "url");
    let digests = quoted_values(&content, "sha256");

    // macOS and Linux, each on Intel and ARM. Windows is served by the
    // release page directly — Homebrew does not install .zip binaries here.
    for target in ["macos-x64", "macos-arm64", "linux-x64", "linux-arm64"] {
        let asset = format!("nvm-{version}-{target}.tar.gz");
        assert!(
            urls.iter().any(|url| url.ends_with(&asset)),
            "binary formula must download {asset}; got {urls:?}"
        );
    }
    assert_eq!(
        urls.len(),
        4,
        "expected exactly one URL per target: {urls:?}"
    );
    assert_eq!(
        digests.len(),
        4,
        "each target needs its own sha256, got {digests:?}"
    );

    // A shared digest across targets would mean one copy/paste overwrote the
    // per-arch values, which installs the wrong binary on one of the arches.
    let unique: std::collections::HashSet<&String> = digests.iter().collect();
    assert_eq!(
        unique.len(),
        4,
        "sha256 values must differ per target: {digests:?}"
    );
}

#[test]
fn source_formula_builds_from_a_tag_and_keeps_a_head() {
    let content = read_formula(SOURCE_FORMULA);
    let version = declared_version(&content);
    assert!(
        content.contains(&format!("archive/refs/tags/v{version}.tar.gz")),
        "stable URL must be the v{version} source archive"
    );
    assert!(
        content.contains("branch: \"main\""),
        "head must keep tracking main so `brew install --HEAD` works"
    );
    assert!(
        content.contains("depends_on \"rust\" => :build"),
        "source formula must declare the Rust build dependency"
    );
}
