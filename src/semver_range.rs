//! Hand-rolled semver-ish range matcher for Node.js version selection.
//!
//! This module implements a best-effort semver range matching engine without
//! pulling in an external semver crate. It supports `>=`, `>`, `<=`, `<`, `^`,
//! `~`, `x`/`*` wildcards, `||` unions, and space-separated compound ranges
//! (e.g. `>=20 <22` means both must hold). Picks the highest installed version
//! that satisfies the constraint.

/// Best-effort semver-ish range matcher. Supports `>=`, `>`, `<=`, `<`, `^`,
/// `~`, `x`/`*` wildcards, `||` unions, and space-separated compound ranges
/// (e.g. `>=20 <22` means both must hold). Picks the highest installed
/// version that satisfies the constraint.
pub fn pick_version_for_range(range: &str, installed: &[String]) -> Option<String> {
    if installed.is_empty() {
        return None;
    }

    // Union: "a || b"
    let ors: Vec<&str> = range.split("||").map(|s| s.trim()).collect();
    let mut candidates: Vec<String> = Vec::new();
    for part in &ors {
        // Within a union arm, space-separated tokens form an AND:
        // ">=20 <22" means both >=20 AND <22 must hold.
        let tokens: Vec<&str> = part.split_whitespace().collect();
        if tokens.is_empty() {
            continue;
        }
        if tokens.len() == 1 {
            if let Some(v) = pick_version_for_range_single(tokens[0], installed) {
                candidates.push(v);
            }
            continue;
        }
        // Compound AND: keep only installed versions satisfying every token.
        let mut matching: Vec<String> = installed
            .iter()
            .filter(|v| tokens.iter().all(|t| version_matches_simple(t, v)))
            .cloned()
            .collect();
        if !matching.is_empty() {
            matching.sort_by(|a, b| crate::utils::compare_semver(a, b));
            // `pop()` is safe because `!matching.is_empty()`, but use `if let`
            // to make the invariant explicit and avoid a panic-prone `unwrap()`
            // that would fire if a future refactor breaks the guard above.
            if let Some(latest) = matching.pop() {
                candidates.push(latest);
            }
        }
    }
    candidates
        .into_iter()
        .max_by(|a, b| crate::utils::compare_semver(a, b))
}

/// Lightweight single-token matcher used by the compound AND branch above.
/// `token` is one of `>=`, `>`, `<=`, `<`, `^`, `~`, `=`, or a bare version.
pub fn version_matches_simple(token: &str, version: &str) -> bool {
    let (op, rest) = if let Some(r) = token.strip_prefix(">=") {
        (">=", r)
    } else if let Some(r) = token.strip_prefix("<=") {
        ("<=", r)
    } else if let Some(r) = token.strip_prefix('>') {
        (">", r)
    } else if let Some(r) = token.strip_prefix('<') {
        ("<", r)
    } else if let Some(r) = token.strip_prefix('=') {
        ("=", r)
    } else if let Some(r) = token.strip_prefix('^') {
        ("^", r)
    } else if let Some(r) = token.strip_prefix('~') {
        ("~", r)
    } else {
        ("=", token)
    };
    let rest = rest.trim().trim_start_matches('v');
    let comps: Vec<&str> = rest.split('.').collect();
    let wild = comps.iter().any(|c| *c == "x" || *c == "X" || *c == "*");
    version_matches_op(version, op, rest, wild)
}

pub fn pick_version_for_range_single(expr: &str, installed: &[String]) -> Option<String> {
    let expr = expr.trim();
    if expr.is_empty() {
        return None;
    }

    // Parse operator + remainder
    let (op, rest) = if let Some(r) = expr.strip_prefix(">=") {
        (">=", r.trim_start())
    } else if let Some(r) = expr.strip_prefix("<=") {
        ("<=", r.trim_start())
    } else if let Some(r) = expr.strip_prefix('>') {
        (">", r.trim_start())
    } else if let Some(r) = expr.strip_prefix('<') {
        ("<", r.trim_start())
    } else if let Some(r) = expr.strip_prefix('=') {
        ("=", r.trim_start())
    } else if let Some(r) = expr.strip_prefix('^') {
        ("^", r.trim_start())
    } else if let Some(r) = expr.strip_prefix('~') {
        ("~", r.trim_start())
    } else {
        ("=", expr)
    };

    let rest = rest.trim().trim_start_matches('v').to_string();
    if rest.is_empty() || rest == "*" || rest == "x" || rest == "X" {
        // Match any — pick newest installed
        return installed
            .iter()
            .max_by(|a, b| crate::utils::compare_semver(a, b))
            .cloned();
    }

    // Detect wildcard in major.minor.patch, e.g. "22.x", "22.*", "20.11.x"
    let comps: Vec<&str> = rest.split('.').collect();
    let wild = comps.iter().any(|c| *c == "x" || *c == "X" || *c == "*");

    // A bare major like "22" (no dots) is shorthand for "22.x.x" — treat as
    // wildcard so `22 || 20` matches any installed 22.x or 20.x.
    let effective_wild = wild || (!rest.contains('.') && op == "=");
    let effective_rest = if effective_wild && !rest.contains('.') && op == "=" {
        format!("{}.x", rest)
    } else {
        rest
    };

    let mut matching: Vec<String> = installed
        .iter()
        .filter(|v| version_matches_op(v, op, &effective_rest, effective_wild))
        .cloned()
        .collect();

    if matching.is_empty() {
        return None;
    }
    matching.sort_by(|a, b| crate::utils::compare_semver(a, b));
    matching.pop() // newest
}

pub fn version_matches_op(version: &str, op: &str, target: &str, wildcard: bool) -> bool {
    // `parse_version_parts` already returns (u32, u32, u32); the previous
    // `parse_v_tuple` wrapper widened to u64, but Node.js version numbers
    // fit in u32 and the comparison semantics are identical.
    let (maj, min, pat) = match crate::utils::parse_version_parts(version) {
        Some(t) => t,
        None => return false,
    };
    let comps: Vec<&str> = target.split('.').collect();
    let t_maj: u32 = comps.first().and_then(|s| s.parse().ok()).unwrap_or(0);
    let t_min: u32 = comps.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    let t_pat: u32 = comps.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);

    match op {
        ">=" => maj > t_maj || (maj == t_maj && (min > t_min || (min == t_min && pat >= t_pat))),
        ">" => maj > t_maj || (maj == t_maj && (min > t_min || (min == t_min && pat > t_pat))),
        "<=" => maj < t_maj || (maj == t_maj && (min < t_min || (min == t_min && pat <= t_pat))),
        "<" => maj < t_maj || (maj == t_maj && (min < t_min || (min == t_min && pat < t_pat))),
        "^" => {
            // Caret: allow changes that do not modify the left-most non-zero
            // element of [major, minor, patch], per the npm semver spec.
            //   ^1.2.3 := >=1.2.3 <2.0.0  (major nonzero -> fix major only)
            //   ^0.2.3 := >=0.2.3 <0.3.0  (minor nonzero -> fix major.minor)
            //   ^0.0.3 := >=0.0.3 <0.0.4  (patch nonzero -> fix all three)
            //   ^0.0.0 := >=0.0.0 <0.0.1  (all zero -> exact)
            // Wildcard components widen the upper bound (treated as
            // "unspecified" rather than zero):
            //   ^0.x   := >=0.0.0 <1.0.0
            //   ^0.0.x := >=0.0.0 <0.1.0
            //   ^1.x   := >=1.0.0 <2.0.0
            //
            // The previous implementation only checked `maj == t_maj &&
            // (min, pat) >= (t_min, t_pat)`, which treats `^0.2.3` as
            // `>=0.2.3 <1.0.0` -- incorrectly matching 0.3.0, 0.10.0, etc.
            if maj != t_maj {
                return false;
            }
            // Lower bound: version >= target.
            if (min, pat) < (t_min, t_pat) {
                return false;
            }
            // Determine how many components were explicitly specified,
            // treating a wildcard as "end of specified components" (a
            // wildcard in position i means positions i.. are unspecified).
            //   "0.2.3"  -> n=3
            //   "0.2.x"  -> n=2 (patch is wildcard -> unspecified)
            //   "0.x"    -> n=1
            //   "1"      -> n=1
            let n_specified = comps
                .iter()
                .position(|c| *c == "x" || *c == "X" || *c == "*")
                .unwrap_or(comps.len().min(3));
            if n_specified == 0 {
                // Entirely wildcard (e.g. `^x`); already handled by the
                // early-return in pick_version_for_range_single, but guard
                // here too -- match anything in the same major.
                return true;
            }
            // Find the left-most non-zero position among the specified
            // components. If all specified are zero, increment the LAST
            // specified component (this is what makes `^0.0.0` -> `<0.0.1`
            // and `^0` -> `<1.0.0`).
            let inc_pos: usize = if t_maj > 0 {
                0
            } else if n_specified >= 2 && t_min > 0 {
                1
            } else if n_specified >= 3 && t_pat > 0 {
                2
            } else {
                // All specified components are zero (or fewer specified).
                // Increment the last specified component.
                n_specified - 1
            };
            // Upper bound = (inc_pos component + 1, everything after = 0).
            let (u_maj, u_min, u_pat) = match inc_pos {
                0 => (t_maj + 1, 0u32, 0u32),
                1 => (0u32, t_min + 1, 0u32),
                _ => (0u32, 0u32, t_pat + 1),
            };
            // version < upper bound (strictly).
            (maj, min, pat) < (u_maj, u_min, u_pat)
        }
        "~" => {
            // Same major.minor, >= target patch
            if maj != t_maj || min != t_min {
                return false;
            }
            pat >= t_pat
        }
        _ => {
            // "=" -- exact, or wildcard match
            if wildcard {
                if comps
                    .first()
                    .map(|s| *s == "x" || *s == "X" || *s == "*")
                    .unwrap_or(true)
                {
                    return false; // shouldn't happen -- handled above
                }
                if maj != t_maj {
                    return false;
                }
                if comps.len() > 1 {
                    let m = comps[1];
                    if !(m == "x" || m == "X" || m == "*") {
                        let m: u32 = m.parse().unwrap_or(0);
                        if min != m {
                            return false;
                        }
                    }
                }
                if comps.len() > 2 {
                    let p = comps[2];
                    if !(p == "x" || p == "X" || p == "*") {
                        let p: u32 = p.parse().unwrap_or(0);
                        if pat != p {
                            return false;
                        }
                    }
                }
                true
            } else {
                (maj, min, pat) == (t_maj, t_min, t_pat)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Unit tests for the hand-rolled semver range matcher
// (`pick_version_for_range`, `version_matches_simple`,
// `pick_version_for_range_single`, `version_matches_op`).
// This is the highest-risk code in the project (no external semver crate),
// so the tests pin every operator and wildcard edge case.
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    fn installed() -> Vec<String> {
        vec![
            "v18.20.0".to_string(),
            "v20.11.0".to_string(),
            "v20.11.1".to_string(),
            "v22.5.0".to_string(),
        ]
    }

    // --- caret (^) ---------------------------------------------------------
    #[test]
    fn caret_picks_newest_in_same_major() {
        let r = pick_version_for_range("^20.10.0", &installed());
        assert_eq!(r.as_deref(), Some("v20.11.1"));
    }

    #[test]
    fn caret_rejects_lower_patch() {
        let r = pick_version_for_range("^20.11.5", &installed());
        assert_eq!(r, None);
    }

    #[test]
    fn caret_rejects_lower_minor_in_same_major() {
        // ^18.21.0 requires >=18.21.0 in major 18; v18.20.0 is too old.
        assert_eq!(pick_version_for_range("^18.21.0", &installed()), None);
    }

    // --- caret (^) with 0.x.y -- the P1-12 regression tests ----------------
    //
    // Per the npm semver spec, the caret locks the left-most NON-ZERO
    // component of [major, minor, patch]:
    //   ^1.2.3  := >=1.2.3 <2.0.0   (major nonzero -> fix major)
    //   ^0.2.3  := >=0.2.3 <0.3.0   (minor nonzero -> fix major.minor)
    //   ^0.0.3  := >=0.0.3 <0.0.4   (patch nonzero -> fix all three)
    //   ^0.0.0  := >=0.0.0 <0.0.1   (all zero -> exact)
    // The previous implementation only checked `maj == t_maj && version >=
    // target`, treating ^0.2.3 as >=0.2.3 <1.0.0 -- incorrectly matching
    // 0.3.0, 0.10.0, etc. These tests pin the correct behaviour.

    fn installed_with_zero_major() -> Vec<String> {
        vec![
            "v0.8.0".to_string(),
            "v0.10.0".to_string(),
            "v0.10.5".to_string(),
            "v0.11.0".to_string(),
            "v0.12.0".to_string(),
            "v0.0.3".to_string(),
            "v0.0.4".to_string(),
            "v0.1.0".to_string(),
            "v20.11.0".to_string(),
        ]
    }

    #[test]
    fn caret_zero_minor_locks_major_minor() {
        // ^0.10.0 := >=0.10.0 <0.11.0. Must match 0.10.5 (newest in 0.10.x)
        // and reject 0.11.0 / 0.12.0 (different minor in major 0).
        let r = pick_version_for_range("^0.10.0", &installed_with_zero_major());
        assert_eq!(r.as_deref(), Some("v0.10.5"));
    }

    #[test]
    fn caret_zero_minor_rejects_higher_minor() {
        // ^0.10.5 must NOT match 0.11.0 or 0.12.0 -- the old code did.
        let r = pick_version_for_range("^0.10.5", &installed_with_zero_major());
        assert_eq!(r.as_deref(), Some("v0.10.5"));
        // And 0.11.0 is explicitly out of range:
        let only_0_11 = vec!["v0.11.0".to_string()];
        assert_eq!(pick_version_for_range("^0.10.5", &only_0_11), None);
    }

    #[test]
    fn caret_zero_zero_patch_is_exact() {
        // ^0.0.3 := >=0.0.3 <0.0.4 -- only 0.0.3 matches (not 0.0.4).
        let r = pick_version_for_range("^0.0.3", &installed_with_zero_major());
        assert_eq!(r.as_deref(), Some("v0.0.3"));
        // A pool with only 0.0.4 must not satisfy ^0.0.3:
        let only_0_0_4 = vec!["v0.0.4".to_string()];
        assert_eq!(pick_version_for_range("^0.0.3", &only_0_0_4), None);
    }

    #[test]
    fn caret_zero_zero_zero_is_exact() {
        // ^0.0.0 := >=0.0.0 <0.0.1 -- only 0.0.0 matches.
        let pool = vec!["v0.0.0".to_string(), "v0.0.1".to_string()];
        assert_eq!(
            pick_version_for_range("^0.0.0", &pool).as_deref(),
            Some("v0.0.0")
        );
    }

    #[test]
    fn caret_zero_wildcard_minor_matches_all_zero_x() {
        // ^0.x := >=0.0.0 <1.0.0 -- any 0.x version matches. Picks newest.
        let r = pick_version_for_range("^0.x", &installed_with_zero_major());
        assert_eq!(r.as_deref(), Some("v0.12.0"));
    }

    #[test]
    fn caret_zero_zero_wildcard_patch_matches_only_zero_zero_x() {
        // ^0.0.x := >=0.0.0 <0.1.0 -- matches 0.0.3 and 0.0.4, not 0.1.0.
        let r = pick_version_for_range("^0.0.x", &installed_with_zero_major());
        assert_eq!(r.as_deref(), Some("v0.0.4"));
    }

    #[test]
    fn caret_zero_bare_major_matches_all_zero_x() {
        // ^0 := >=0.0.0 <1.0.0 (same as ^0.x). Picks newest 0.x.
        let r = pick_version_for_range("^0", &installed_with_zero_major());
        assert_eq!(r.as_deref(), Some("v0.12.0"));
    }

    #[test]
    fn caret_nonzero_major_still_works() {
        // Regression guard: the fix must not break the existing ^1.x.y path.
        // ^20.10.0 := >=20.10.0 <21.0.0 -- picks newest 20.x.
        let r = pick_version_for_range("^20.10.0", &installed_with_zero_major());
        assert_eq!(r.as_deref(), Some("v20.11.0"));
    }

    // --- tilde (~) ---------------------------------------------------------
    #[test]
    fn tilde_locks_major_minor() {
        let r = pick_version_for_range("~20.11.0", &installed());
        assert_eq!(r.as_deref(), Some("v20.11.1"));
    }

    #[test]
    fn tilde_rejects_different_minor() {
        let r = pick_version_for_range("~20.12.0", &installed());
        assert_eq!(r, None);
    }

    // --- comparison operators ---------------------------------------------
    #[test]
    fn ge_picks_newest_satisfying() {
        let r = pick_version_for_range(">=20.0.0", &installed());
        assert_eq!(r.as_deref(), Some("v22.5.0"));
    }

    #[test]
    fn gt_strictly_greater() {
        let r = pick_version_for_range(">22.5.0", &installed());
        assert_eq!(r, None);
        let r2 = pick_version_for_range(">20.11.0", &installed());
        assert_eq!(r2.as_deref(), Some("v22.5.0"));
    }

    #[test]
    fn le_picks_newest_below_bound() {
        let r = pick_version_for_range("<=20.11.1", &installed());
        assert_eq!(r.as_deref(), Some("v20.11.1"));
    }

    #[test]
    fn lt_strictly_less() {
        let r = pick_version_for_range("<22.5.0", &installed());
        assert_eq!(r.as_deref(), Some("v20.11.1"));
    }

    // --- exact (=) ---------------------------------------------------------
    #[test]
    fn exact_match() {
        let r = pick_version_for_range("=20.11.0", &installed());
        assert_eq!(r.as_deref(), Some("v20.11.0"));
    }

    #[test]
    fn bare_version_is_exact() {
        let r = pick_version_for_range("20.11.1", &installed());
        assert_eq!(r.as_deref(), Some("v20.11.1"));
    }

    #[test]
    fn v_prefix_stripped() {
        let r = pick_version_for_range("v20.11.1", &installed());
        assert_eq!(r.as_deref(), Some("v20.11.1"));
    }

    // --- wildcards (x / *) -------------------------------------------------
    #[test]
    fn wildcard_major_matches_newest_of_major() {
        let r = pick_version_for_range("20.x", &installed());
        assert_eq!(r.as_deref(), Some("v20.11.1"));
    }

    #[test]
    fn wildcard_star_matches_newest_of_major() {
        let r = pick_version_for_range("20.*", &installed());
        assert_eq!(r.as_deref(), Some("v20.11.1"));
    }

    #[test]
    fn wildcard_minor_pin_patch() {
        // 20.11.x -> both 20.11.0 and 20.11.1 match -> newest
        let r = pick_version_for_range("20.11.x", &installed());
        assert_eq!(r.as_deref(), Some("v20.11.1"));
    }

    #[test]
    fn bare_major_is_wildcard() {
        // "22" -> 22.x.x -> matches v22.5.0
        let r = pick_version_for_range("22", &installed());
        assert_eq!(r.as_deref(), Some("v22.5.0"));
    }

    #[test]
    fn star_alone_matches_any() {
        let r = pick_version_for_range("*", &installed());
        assert_eq!(r.as_deref(), Some("v22.5.0"));
    }

    // --- union (||) --------------------------------------------------------
    #[test]
    fn union_picks_newest_across_arms() {
        let r = pick_version_for_range("^18 || ^22", &installed());
        assert_eq!(r.as_deref(), Some("v22.5.0"));
    }

    #[test]
    fn union_with_no_matching_arm() {
        let r = pick_version_for_range("^17 || ^19", &installed());
        assert_eq!(r, None);
    }

    // --- compound AND ------------------------------------------------------
    #[test]
    fn compound_and_intersection() {
        // >=20 AND <22 -> both 20.x match -> newest is v20.11.1
        let r = pick_version_for_range(">=20 <22", &installed());
        assert_eq!(r.as_deref(), Some("v20.11.1"));
    }

    #[test]
    fn compound_and_empty_intersection() {
        let r = pick_version_for_range(">=21 <22", &installed());
        assert_eq!(r, None);
    }

    #[test]
    fn compound_and_closed_interval_inclusive_both_ends() {
        // Both bounds inclusive: v20.11.0 and v20.11.1 satisfy
        // >=20.11.0 AND <=20.11.1. Newest is v20.11.1.
        let r = pick_version_for_range(">=20.11.0 <=20.11.1", &installed());
        assert_eq!(r.as_deref(), Some("v20.11.1"));
    }

    #[test]
    fn compound_and_closed_interval_single_match() {
        // Tight closed interval pinning exactly one version:
        // >=20.11.1 AND <=20.11.1 -> only v20.11.1.
        let r = pick_version_for_range(">=20.11.1 <=20.11.1", &installed());
        assert_eq!(r.as_deref(), Some("v20.11.1"));
    }

    #[test]
    fn compound_and_strict_lower_excludes_floor() {
        // >20.11.0 (strict) excludes v20.11.0; <22 excludes v22.5.0.
        // Only v20.11.1 survives both -> v20.11.1. This locks the
        // semantic difference between `>` and `>=` inside an AND.
        let r = pick_version_for_range(">20.11.0 <22", &installed());
        assert_eq!(r.as_deref(), Some("v20.11.1"));
    }

    #[test]
    fn compound_and_three_tokens_intersects_correctly() {
        // Three-token AND: >=18 (includes all) AND >20.11.0 (excludes
        // v18.20.0 and v20.11.0) AND <22 (excludes v22.5.0). Only
        // v20.11.1 satisfies all three. Exercises the
        // `tokens.iter().all(...)` filter with len > 2.
        let r = pick_version_for_range(">=18 >20.11.0 <22", &installed());
        assert_eq!(r.as_deref(), Some("v20.11.1"));
    }

    #[test]
    fn compound_and_caret_with_lower_bound() {
        // ^20.11.0 := >=20.11.0 <21.0.0; intersected with >=20.11.1
        // leaves only v20.11.1. Locks the caret upper-bound semantics
        // inside the compound AND path (which uses
        // `version_matches_simple` rather than the single-token
        // resolver).
        let r = pick_version_for_range("^20.11.0 >=20.11.1", &installed());
        assert_eq!(r.as_deref(), Some("v20.11.1"));
    }

    #[test]
    fn compound_and_tilde_with_lower_bound() {
        // ~20.11.0 := >=20.11.0 <20.12.0; intersected with >=20.11.1
        // leaves only v20.11.1. Locks the tilde upper-bound semantics
        // inside the compound AND path.
        let r = pick_version_for_range("~20.11.0 >=20.11.1", &installed());
        assert_eq!(r.as_deref(), Some("v20.11.1"));
    }

    #[test]
    fn compound_and_picks_newest_when_multiple_match() {
        // >=20 AND <23 matches v20.11.0, v20.11.1, AND v22.5.0
        // (`<=22` would NOT match v22.5.0 -- in semver `<=22` means
        // `<=22.0.0`, so a major-bounded upper limit must use `<next`
        // to include patch releases). The picker must return the NEWEST
        // (v22.5.0), not the first match in iteration order -- guards
        // against a regression that returned the first filter hit
        // without sorting.
        let r = pick_version_for_range(">=20 <23", &installed());
        assert_eq!(r.as_deref(), Some("v22.5.0"));
    }

    #[test]
    fn compound_and_inside_union_first_arm_wins_on_value() {
        // Union of two AND arms: (>=20 <22) picks v20.11.1; (^18) picks
        // v18.20.0. The overall result is the MAX across arms, so
        // v20.11.1 wins. Locks the `candidates.max_by` at the end of
        // `pick_version_for_range`.
        let r = pick_version_for_range(">=20 <22 || ^18", &installed());
        assert_eq!(r.as_deref(), Some("v20.11.1"));
    }

    #[test]
    fn compound_and_inside_union_second_arm_can_win() {
        // (>=22 <23) picks v22.5.0; (^18) picks v18.20.0. v22.5.0 is
        // newer, so the first arm wins -- but if the first arm had no
        // match, the second arm alone must still produce a result.
        // Here we verify the AND arm (first) wins over the single-token
        // arm (second) when both have matches.
        let r = pick_version_for_range(">=22 <23 || ^18", &installed());
        assert_eq!(r.as_deref(), Some("v22.5.0"));
    }

    #[test]
    fn compound_and_inside_union_and_arm_no_match_falls_through() {
        // First AND arm (>=21 <22) matches nothing; the union must
        // fall through to the second arm (^18), which picks v18.20.0.
        // Guards against an early-return on the first arm that would
        // miss the union semantics.
        let r = pick_version_for_range(">=21 <22 || ^18", &installed());
        assert_eq!(r.as_deref(), Some("v18.20.0"));
    }

    // --- edge cases --------------------------------------------------------
    #[test]
    fn empty_installed_returns_none() {
        let empty: Vec<String> = Vec::new();
        assert_eq!(pick_version_for_range("^20", &empty), None);
        assert_eq!(pick_version_for_range("*", &empty), None);
    }

    #[test]
    fn no_match_returns_none() {
        assert_eq!(pick_version_for_range("^99", &installed()), None);
    }

    // --- parse_version_parts (used by version_matches_op) ------------------
    #[test]
    fn parse_v_tuple_v_prefixed() {
        assert_eq!(
            crate::utils::parse_version_parts("v20.11.1"),
            Some((20, 11, 1))
        );
    }

    #[test]
    fn parse_v_tuple_bare() {
        assert_eq!(
            crate::utils::parse_version_parts("18.20.0"),
            Some((18, 20, 0))
        );
    }

    #[test]
    fn parse_v_tuple_iojs_prefix() {
        assert_eq!(
            crate::utils::parse_version_parts("iojs-v3.3.1"),
            Some((3, 3, 1))
        );
    }

    #[test]
    fn parse_v_tuple_iojs_dot_prefix() {
        // Previously a bug: parse missed "io.js-v" / "io.js-" prefixes,
        // making io.js versions invisible to the engines.node range matcher.
        assert_eq!(
            crate::utils::parse_version_parts("io.js-v3.3.1"),
            Some((3, 3, 1))
        );
        assert_eq!(
            crate::utils::parse_version_parts("io.js-3.3.1"),
            Some((3, 3, 1))
        );
    }

    #[test]
    fn parse_v_tuple_trailing_suffix() {
        // "v20.11.1-rc.1" -> (20, 11, 1)
        assert_eq!(
            crate::utils::parse_version_parts("v20.11.1-rc.1"),
            Some((20, 11, 1))
        );
    }

    #[test]
    fn parse_v_tuple_missing_patch_defaults_zero() {
        assert_eq!(crate::utils::parse_version_parts("v22"), Some((22, 0, 0)));
    }

    #[test]
    fn iojs_dot_prefix_matches_engines_range() {
        // Regression: an installed "io.js-3.3.1" used to be invisible to
        // `package.json#engines.node` range matching because parse_v_tuple
        // returned None for the "io.js-" prefix.
        let installed = vec!["io.js-3.3.1".to_string()];
        assert_eq!(
            pick_version_for_range(">=3.0.0", &installed),
            Some("io.js-3.3.1".to_string())
        );
    }
}
