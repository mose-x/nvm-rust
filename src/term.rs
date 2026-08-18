use std::borrow::Cow;

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
                || (0x2E80..0x303E).contains(&cp)     // CJK Radicals etc.
                || (0x3041..0x33FF).contains(&cp)     // Hiragana etc.
                || (0x3400..0x4DBF).contains(&cp)     // CJK Ext A
                || (0x4E00..0x9FFF).contains(&cp)     // CJK Unified
                || (0xA000..0xA4CF).contains(&cp)     // Yi Syllables
                || (0xAC00..0xD7A3).contains(&cp)     // Hangul
                || (0xF900..0xFAFF).contains(&cp)     // CJK Compat
                || (0xFE30..0xFE4F).contains(&cp)     // CJK Compat Forms
                || (0xFF00..0xFF60).contains(&cp)     // Fullwidth Forms
                || (0xFFE0..0xFFE6).contains(&cp)     // Fullwidth Forms
                || (0x20000..0x2FFFD).contains(&cp)   // CJK Ext B-D
                || (0x30000..0x3FFFD).contains(&cp)
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
pub fn pad_right(s: &str, width: usize) -> Cow<'_, str> {
    let w = display_width(s);
    if w >= width {
        Cow::Borrowed(s)
    } else {
        Cow::Owned(format!("{}{}", s, " ".repeat(width - w)))
    }
}

/// Right-align `s` to `width` columns, padding with spaces on the left.
pub fn pad_left(s: &str, width: usize) -> Cow<'_, str> {
    let w = display_width(s);
    if w >= width {
        Cow::Borrowed(s)
    } else {
        Cow::Owned(format!("{}{}", " ".repeat(width - w), s))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

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
