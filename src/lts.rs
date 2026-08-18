use std::collections::BTreeMap;

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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

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
}
