//! Built-in metrics for the 14 standard fonts.
//!
//! A PDF may reference Helvetica, Times, Courier, Symbol or ZapfDingbats with no `/Widths` array
//! and no embedded program, on the understanding that every viewer knows their metrics. Only the
//! ASCII range is tabulated here — it is where essentially all text lives, and the alternative to
//! a partial table is a wrong advance for every glyph, which shifts an entire line.
//!
//! Column order matches [`Family`]. Courier is monospaced at 600 and needs no table.

use super::encoding::glyph_name_to_unicode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Family {
    Helvetica = 0,
    HelveticaBold = 1,
    TimesRoman = 2,
    TimesBold = 3,
    TimesItalic = 4,
    TimesBoldItalic = 5,
    Courier,
    /// Symbol and ZapfDingbats: no tabulated metrics, but they are never monospaced either.
    Other,
}

/// Widths for codes 32..=126, one column per [`Family`] discriminant 0..=5.
#[rustfmt::skip]
const ASCII: [[u16; 6]; 95] = [
    /* space      */ [278, 278, 250, 250, 250, 250],
    /* exclam     */ [278, 333, 333, 333, 333, 389],
    /* quotedbl   */ [355, 474, 408, 555, 420, 555],
    /* numbersign */ [556, 556, 500, 500, 500, 500],
    /* dollar     */ [556, 556, 500, 500, 500, 500],
    /* percent    */ [889, 889, 833, 1000, 833, 833],
    /* ampersand  */ [667, 722, 778, 833, 778, 778],
    /* quotesingle*/ [191, 238, 180, 278, 214, 278],
    /* parenleft  */ [333, 333, 333, 333, 333, 333],
    /* parenright */ [333, 333, 333, 333, 333, 333],
    /* asterisk   */ [389, 389, 500, 500, 500, 500],
    /* plus       */ [584, 584, 564, 570, 675, 570],
    /* comma      */ [278, 278, 250, 250, 250, 250],
    /* hyphen     */ [333, 333, 333, 333, 333, 333],
    /* period     */ [278, 278, 250, 250, 250, 250],
    /* slash      */ [278, 278, 278, 278, 278, 278],
    /* zero       */ [556, 556, 500, 500, 500, 500],
    /* one        */ [556, 556, 500, 500, 500, 500],
    /* two        */ [556, 556, 500, 500, 500, 500],
    /* three      */ [556, 556, 500, 500, 500, 500],
    /* four       */ [556, 556, 500, 500, 500, 500],
    /* five       */ [556, 556, 500, 500, 500, 500],
    /* six        */ [556, 556, 500, 500, 500, 500],
    /* seven      */ [556, 556, 500, 500, 500, 500],
    /* eight      */ [556, 556, 500, 500, 500, 500],
    /* nine       */ [556, 556, 500, 500, 500, 500],
    /* colon      */ [278, 333, 278, 333, 333, 333],
    /* semicolon  */ [278, 333, 278, 333, 333, 333],
    /* less       */ [584, 584, 564, 570, 675, 570],
    /* equal      */ [584, 584, 564, 570, 675, 570],
    /* greater    */ [584, 584, 564, 570, 675, 570],
    /* question   */ [556, 611, 444, 500, 500, 500],
    /* at         */ [1015, 975, 921, 930, 920, 832],
    /* A          */ [667, 722, 722, 722, 611, 667],
    /* B          */ [667, 722, 667, 667, 611, 667],
    /* C          */ [722, 722, 667, 722, 667, 667],
    /* D          */ [722, 722, 722, 722, 722, 722],
    /* E          */ [667, 667, 611, 667, 611, 667],
    /* F          */ [611, 611, 556, 611, 611, 667],
    /* G          */ [778, 778, 722, 778, 722, 722],
    /* H          */ [722, 722, 722, 778, 722, 778],
    /* I          */ [278, 278, 333, 389, 333, 389],
    /* J          */ [500, 556, 389, 500, 444, 500],
    /* K          */ [667, 722, 722, 778, 667, 667],
    /* L          */ [556, 611, 611, 667, 556, 611],
    /* M          */ [833, 833, 889, 944, 833, 889],
    /* N          */ [722, 722, 722, 722, 667, 722],
    /* O          */ [778, 778, 722, 778, 722, 722],
    /* P          */ [667, 667, 556, 611, 611, 611],
    /* Q          */ [778, 778, 722, 778, 722, 722],
    /* R          */ [722, 722, 667, 722, 611, 667],
    /* S          */ [667, 667, 556, 556, 500, 556],
    /* T          */ [611, 611, 611, 667, 556, 611],
    /* U          */ [722, 722, 722, 722, 722, 722],
    /* V          */ [667, 667, 722, 722, 611, 667],
    /* W          */ [944, 944, 944, 1000, 833, 889],
    /* X          */ [667, 667, 722, 722, 611, 667],
    /* Y          */ [667, 667, 722, 722, 556, 611],
    /* Z          */ [611, 611, 611, 667, 556, 611],
    /* bracketleft*/ [278, 333, 333, 333, 389, 333],
    /* backslash  */ [278, 278, 278, 278, 278, 278],
    /* bracketrgt */ [278, 333, 333, 333, 389, 333],
    /* asciicircum*/ [469, 584, 469, 581, 422, 570],
    /* underscore */ [556, 556, 500, 500, 500, 500],
    /* grave      */ [333, 333, 333, 333, 333, 333],
    /* a          */ [556, 556, 444, 500, 500, 500],
    /* b          */ [556, 611, 500, 556, 500, 500],
    /* c          */ [500, 556, 444, 444, 444, 444],
    /* d          */ [556, 611, 500, 556, 500, 500],
    /* e          */ [556, 556, 444, 444, 444, 444],
    /* f          */ [278, 333, 333, 333, 278, 333],
    /* g          */ [556, 611, 500, 500, 500, 500],
    /* h          */ [556, 611, 500, 556, 500, 556],
    /* i          */ [222, 278, 278, 278, 278, 278],
    /* j          */ [222, 278, 278, 333, 278, 278],
    /* k          */ [500, 556, 500, 556, 444, 500],
    /* l          */ [222, 278, 278, 278, 278, 278],
    /* m          */ [833, 889, 778, 833, 722, 778],
    /* n          */ [556, 611, 500, 556, 500, 556],
    /* o          */ [556, 611, 500, 500, 500, 500],
    /* p          */ [556, 611, 500, 556, 500, 500],
    /* q          */ [556, 611, 500, 556, 500, 500],
    /* r          */ [333, 389, 333, 444, 389, 389],
    /* s          */ [500, 556, 389, 389, 389, 389],
    /* t          */ [278, 333, 278, 333, 278, 278],
    /* u          */ [556, 611, 500, 556, 500, 556],
    /* v          */ [500, 556, 500, 500, 444, 444],
    /* w          */ [722, 778, 722, 722, 667, 667],
    /* x          */ [500, 556, 500, 500, 444, 500],
    /* y          */ [500, 556, 500, 500, 444, 444],
    /* z          */ [500, 500, 444, 444, 389, 389],
    /* braceleft  */ [334, 389, 480, 394, 400, 348],
    /* bar        */ [260, 280, 200, 220, 275, 220],
    /* braceright */ [334, 389, 480, 394, 400, 348],
    /* asciitilde */ [584, 584, 541, 520, 541, 570],
];

/// Classifies a `/BaseFont` name, tolerating subset prefixes (`ABCDEF+Helvetica`) and the
/// PostScript style suffixes producers use interchangeably (`,Bold` and `-Bold`).
fn family_of(base_font: &str) -> Family {
    let name = base_font.split('+').next_back().unwrap_or(base_font);
    let lower = name.to_ascii_lowercase();
    // Oblique and Italic share metrics with the upright face in the Helvetica and Courier
    // families, but not in Times, so the checks below are ordered per family.
    let bold = lower.contains("bold") || lower.contains("black") || lower.contains("heavy");
    let italic = lower.contains("italic") || lower.contains("oblique");

    if lower.contains("courier") || lower.contains("mono") {
        Family::Courier
    } else if lower.contains("times") || lower.contains("serif") || lower.contains("roman") {
        match (bold, italic) {
            (true, true) => Family::TimesBoldItalic,
            (true, false) => Family::TimesBold,
            (false, true) => Family::TimesItalic,
            (false, false) => Family::TimesRoman,
        }
    } else if lower.contains("symbol") || lower.contains("dingbat") || lower.contains("zapf") {
        Family::Other
    } else {
        // Helvetica, Arial and anything unrecognised: Helvetica is the closest common default.
        if bold {
            Family::HelveticaBold
        } else {
            Family::Helvetica
        }
    }
}

/// The standard-font advance for `glyph_name`, in glyph space (1000 units per em).
///
/// Returns `None` outside the tabulated ASCII range so callers can fall back to a font program's
/// own advance or a `/MissingWidth` before resorting to a guess.
pub fn base14_width(base_font: &str, glyph_name: &str) -> Option<f32> {
    let family = family_of(base_font);
    if family == Family::Courier {
        return Some(600.0);
    }
    let column = match family {
        Family::Other => return None,
        Family::Courier => unreachable!("handled above"),
        f => f as usize,
    };
    // Glyph names reach the table through Unicode so that `space`, `uni0041` and `A.sc` all land
    // on the same row without a second name table.
    let text = glyph_name_to_unicode(glyph_name)?;
    let mut chars = text.chars();
    let ch = chars.next()?;
    if chars.next().is_some() {
        return None; // A ligature or multi-character mapping has no single-glyph width.
    }
    let code = u32::from(ch);
    if !(32..=126).contains(&code) {
        return None;
    }
    Some(f32::from(ASCII[code as usize - 32][column]))
}

/// Whether `base_font` names one of the 14 standard fonts, which may legally omit `/Widths`.
pub fn is_standard_font(base_font: &str) -> bool {
    let name = base_font.split('+').next_back().unwrap_or(base_font);
    let lower = name.to_ascii_lowercase();
    [
        "helvetica", "courier", "times", "symbol", "zapfdingbats", "arial",
    ]
    .iter()
    .any(|f| lower.contains(f))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helvetica_ascii_widths() {
        assert_eq!(base14_width("Helvetica", "space"), Some(278.0));
        assert_eq!(base14_width("Helvetica", "A"), Some(667.0));
        assert_eq!(base14_width("Helvetica", "i"), Some(222.0));
        assert_eq!(base14_width("Helvetica", "W"), Some(944.0));
    }

    #[test]
    fn bold_and_italic_select_columns() {
        assert_eq!(base14_width("Helvetica-Bold", "A"), Some(722.0));
        assert_eq!(base14_width("Times-Roman", "A"), Some(722.0));
        assert_eq!(base14_width("Times-Italic", "A"), Some(611.0));
        assert_eq!(base14_width("Times-BoldItalic", "A"), Some(667.0));
        // Oblique is Helvetica's italic, and shares the upright metrics.
        assert_eq!(
            base14_width("Helvetica-Oblique", "A"),
            base14_width("Helvetica", "A")
        );
    }

    #[test]
    fn courier_is_monospaced() {
        assert_eq!(base14_width("Courier", "A"), Some(600.0));
        assert_eq!(base14_width("Courier-BoldOblique", "i"), Some(600.0));
    }

    #[test]
    fn subset_prefixes_and_comma_styles_resolve() {
        assert_eq!(base14_width("ABCDEF+Times-Bold", "A"), Some(722.0));
        assert_eq!(base14_width("Arial,Bold", "A"), Some(722.0));
    }

    #[test]
    fn untabulated_names_report_nothing() {
        assert_eq!(base14_width("Helvetica", "eacute"), None);
        assert_eq!(base14_width("Helvetica", "fi"), None);
        assert_eq!(base14_width("Symbol", "alpha"), None);
    }

    #[test]
    fn standard_font_detection() {
        assert!(is_standard_font("ABCDEF+Helvetica-Bold"));
        assert!(is_standard_font("Times-Roman"));
        assert!(!is_standard_font("MinionPro-Regular"));
    }
}
