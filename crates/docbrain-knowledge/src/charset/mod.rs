// SPDX-License-Identifier: MIT
//! The hidden-text rule (`SEMANTICS.md` §Hidden text), on the Unicode 17.0
//! tables generated into `tables.rs`. `std`'s Unicode tables are never used here.

pub mod tables;

/// Whether `c` lies in one of `table`'s sorted inclusive ranges.
pub fn in_table(table: &[(u32, u32)], c: u32) -> bool {
    table
        .binary_search_by(|&(lo, hi)| {
            if hi < c {
                std::cmp::Ordering::Less
            } else if lo > c {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Equal
            }
        })
        .is_ok()
}

const TAB: u32 = 0x09;
const LF: u32 = 0x0A;
const CR: u32 = 0x0D;
const ZWNJ: u32 = 0x200C;
const ZWJ: u32 = 0x200D;
/// The implicit direction marks: they reorder nothing (`SEMANTICS.md` §Hidden text).
const DIRECTION_MARKS: [u32; 3] = [0x200E, 0x200F, 0x061C];
/// The variation selectors that are hidden even alone (`SEMANTICS.md` §Hidden text).
const SUPPLEMENTARY_VS: (u32, u32) = (0xE0100, 0xE01EF);

/// A non-ASCII letter, mark or emoji: the neighbours a joiner may sit between.
fn joinable(c: Option<u32>) -> bool {
    matches!(c, Some(c) if c > 0x7F
        && (in_table(tables::LETTER, c) || in_table(tables::MARK, c) || in_table(tables::EMOJI, c)))
}

/// The first hidden-text character in `text`, as (line, column, code point). Lines and columns count
/// from 1; a column counts Unicode scalar values; a BOM at the very start is allowed and is not a column.
pub fn hidden_text(text: &str) -> Option<(u32, u32, u32)> {
    let body = text.strip_prefix('\u{FEFF}').unwrap_or(text);
    let cs: Vec<u32> = body.chars().map(u32::from).collect();
    let (mut line, mut col) = (1u32, 0u32);
    for (i, &c) in cs.iter().enumerate() {
        let prev = i.checked_sub(1).map(|j| cs[j]);
        let next = cs.get(i + 1).copied();
        col += 1;
        let hidden = if in_table(tables::CC, c) {
            !(c == TAB || c == LF || (c == CR && next == Some(LF)))
        } else if in_table(tables::CF, c) {
            let joiner_ok = (c == ZWNJ || c == ZWJ) && joinable(prev) && joinable(next);
            !(joiner_ok || DIRECTION_MARKS.contains(&c))
        } else if (SUPPLEMENTARY_VS.0..=SUPPLEMENTARY_VS.1).contains(&c) {
            true
        } else {
            in_table(tables::VARIATION_SELECTOR, c) && prev.is_some_and(|p| in_table(tables::VARIATION_SELECTOR, p))
        };
        if hidden {
            return Some((line, col, c));
        }
        if c == LF {
            line += 1;
            col = 0;
        }
    }
    // A BOM anywhere but the very start is a Cf character and is caught above.
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bidi_controls_are_hidden() {
        for c in ['\u{202A}', '\u{202E}', '\u{2066}', '\u{2069}'] {
            assert!(hidden_text(&format!("a{c}b")).is_some(), "{:X}", c as u32);
        }
    }

    #[test]
    fn tags_zero_width_soft_hyphen_hidden() {
        for c in ['\u{E0041}', '\u{200B}', '\u{00AD}', '\u{2060}'] {
            assert!(hidden_text(&format!("x{c}y")).is_some(), "{:X}", c as u32);
        }
    }

    #[test]
    fn controls_except_tab_lf_crlf() {
        assert!(hidden_text("a\tb\nc\r\nd").is_none());
        assert!(hidden_text("a\rb").is_some());
        assert!(hidden_text("a\r").is_some());
        assert!(hidden_text("a\u{1B}[31m").is_some());
        assert!(hidden_text("a\u{85}").is_some());
        assert!(hidden_text("a\u{0}").is_some());
        assert!(hidden_text("a\u{7F}").is_some());
        assert!(hidden_text("a\u{0B}").is_some()); // vertical tab
    }

    #[test]
    fn bom_only_at_start() {
        assert!(hidden_text("\u{FEFF}x").is_none());
        assert!(hidden_text("x\u{FEFF}").is_some());
        assert!(hidden_text("\u{FEFF}\u{FEFF}x").is_some());
    }

    #[test]
    fn direction_marks_allowed() {
        assert!(hidden_text("מחיר\u{200F} 5").is_none());
        assert!(hidden_text("\u{061C}٣").is_none());
        assert!(hidden_text("a\u{200E}b").is_none());
    }

    #[test]
    fn zwj_between_letters_marks_emoji_only() {
        assert!(hidden_text("می\u{200C}خواهم").is_none()); // Persian ZWNJ
        assert!(hidden_text("क्\u{200D}ष").is_none()); // Indic
        assert!(hidden_text("👨\u{200D}👩\u{200D}👧").is_none());
        assert!(hidden_text("❤\u{FE0F}\u{200D}🔥").is_none()); // after VS16, a mark
        assert!(hidden_text("ab\u{200D}cd").is_some()); // ASCII neighbours
        assert!(hidden_text("\u{200D}م").is_some()); // no left neighbour
        assert!(hidden_text("م\u{200D}").is_some()); // no right neighbour
        assert!(hidden_text("1\u{200D}2").is_some()); // ASCII digits are Emoji=Yes but ASCII
        assert!(hidden_text("م\u{200D}\u{200D}م").is_some()); // a joiner is not a letter
    }

    #[test]
    fn variation_selectors() {
        assert!(hidden_text("❤\u{FE0F}").is_none());
        assert!(hidden_text("a\u{E0100}").is_some());
        assert!(hidden_text("❤\u{FE0F}\u{FE0F}").is_some());
        assert!(hidden_text("ᠠ\u{180B}\u{FE0F}").is_some()); // a run across the two blocks
    }

    #[test]
    fn position_is_line_and_scalar_column() {
        assert_eq!(hidden_text("ok\nαβ\u{202E}"), Some((2, 3, 0x202E)));
        assert_eq!(hidden_text("\u{FEFF}a\u{200B}"), Some((1, 2, 0x200B))); // the leading BOM is not a column
        assert_eq!(hidden_text("a\r\nb\u{200B}"), Some((2, 2, 0x200B)));
        assert_eq!(hidden_text("❤\u{FE0F}\u{FE0F}"), Some((1, 3, 0xFE0F))); // the second of the run
    }

    #[test]
    fn ordinary_text_in_any_language_passes() {
        for t in ["Production API runs at least two replicas", "日本語のテキスト", "Ελληνικά", "emoji 🔥 ok", "e\u{301}"] {
            assert!(hidden_text(t).is_none(), "{t}");
        }
    }

    #[test]
    fn tables_pin_unicode_17() {
        assert_eq!(tables::UNICODE_VERSION, "17.0.0");
        assert!(in_table(tables::CF, 0x200B));
        assert!(in_table(tables::CC, 0x85));
        assert!(in_table(tables::LETTER, 'a' as u32));
        assert!(in_table(tables::MARK, 0x0301));
        assert!(in_table(tables::VARIATION_SELECTOR, 0xFE0F));
        assert!(in_table(tables::EMOJI, 0x1F525));
        assert!(!in_table(tables::LETTER, '1' as u32));
        for t in [tables::CF, tables::CC, tables::LETTER, tables::MARK, tables::VARIATION_SELECTOR, tables::EMOJI] {
            assert!(t.windows(2).all(|w| w[0].1 + 1 < w[1].0), "sorted, disjoint, not touching");
            assert!(t.iter().all(|(lo, hi)| lo <= hi));
        }
    }
}
