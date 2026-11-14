//! `/ActualText` through the public API, on synthetic documents shaped like the real defect.
//!
//! Chromium's PDF backend (Skia) writes some glyphs — ligatures, and on some pages plain digits —
//! with no usable `/ToUnicode` entry, and says what they are only by wrapping each in
//! `/Span <</ActualText ...>> BDC ... EMC`. A reader that ignores the span loses the characters:
//! "significant" becomes "signicant" and every reference number with a 6 or an 8 in it changes.

use rustium_pdf::Document;

/// A font whose codes 1..=3 name glyphs no encoding knows, so they carry no text of their own,
/// while the rest of the code space is ordinary Helvetica.
const MUTE_FONT: &str = "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica \
     /Encoding << /Type /Encoding /Differences [1 /g1 /g2 /g3] >> >>";

/// Assembles a one-page PDF with a correct xref. Object 4 is the page content; `extra` objects
/// are numbered from 6 and `resources` is spliced into the page's resource dictionary.
fn pdf(content: &str, resources: &str, extra: &[String]) -> Vec<u8> {
    let stream = |dict: &str, body: &str| {
        format!(
            "<< {dict} /Length {} >>\nstream\n{body}\nendstream",
            body.len()
        )
    };
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R \
             /Resources << /Font << /F1 5 0 R >> {resources} >> >>"
        ),
        stream("", content),
        MUTE_FONT.to_string(),
    ];
    objects.extend(extra.iter().cloned());

    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
    }
    let xref_at = out.len();
    let size = objects.len() + 1;
    out.extend_from_slice(format!("xref\n0 {size}\n0000000000 65535 f \n").as_bytes());
    for offset in offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!("trailer\n<< /Size {size} /Root 1 0 R >>\nstartxref\n{xref_at}\n%%EOF\n")
            .as_bytes(),
    );
    out
}

fn form(body: &str) -> String {
    format!(
        "<< /Type /XObject /Subtype /Form /BBox [0 0 612 792] \
         /Resources << /Font << /F1 5 0 R >> >> /Length {} >>\nstream\n{body}\nendstream",
        body.len()
    )
}

fn text(content: &str) -> String {
    text_with(content, "", &[])
}

fn text_with(content: &str, resources: &str, extra: &[String]) -> String {
    let doc = Document::from_bytes(pdf(content, resources, extra), None).expect("fixture parses");
    doc.page(0).expect("page parses").text()
}

/// The content Chromium writes for "significant [68]": one glyph per `Tj`, each placed by its
/// own `Td`, with the ligature and the two digits as mute glyphs inside spans.
const CHROMIUM_LINE: &str = "BT /F1 16 Tf 1 0 0 1 72 720 Tm \
     (s) Tj 8 0 Td (i) Tj 4 0 Td (g) Tj 9 0 Td (n) Tj 9 0 Td (i) Tj \
     /Span <</ActualText <FEFF00660069>>> BDC 4 0 Td <01> Tj EMC \
     8 0 Td (c) Tj 8 0 Td (a) Tj 9 0 Td (n) Tj 9 0 Td (t) Tj \
     4 0 Td ( ) Tj 4 0 Td ([) Tj \
     /Span <</ActualText <FEFF0036>>> BDC 5 0 Td <02> Tj EMC \
     /Span <</ActualText <FEFF0038>>> BDC 9 0 Td <03> Tj EMC \
     9 0 Td (]) Tj ET";

#[test]
fn the_fixture_font_really_is_mute() {
    // Without the spans the damage is exactly the one reported.
    let stripped = CHROMIUM_LINE
        .replace("/Span <</ActualText <FEFF00660069>>> BDC", "")
        .replace("/Span <</ActualText <FEFF0036>>> BDC", "")
        .replace("/Span <</ActualText <FEFF0038>>> BDC", "")
        .replace("EMC", "");
    assert_eq!(text(&stripped), "signicant []");
}

#[test]
fn chromium_spans_restore_ligatures_and_digits() {
    let doc = Document::from_bytes(pdf(CHROMIUM_LINE, "", &[]), None).unwrap();
    let page = doc.page(0).unwrap();
    assert_eq!(page.text(), "significant [68]");

    // The ligature is one glyph reading as two characters, where the glyph was drawn.
    let fi = page.glyphs.iter().find(|g| g.text == "fi").expect("fi");
    assert_eq!(fi.code(), 1);
    assert!((fi.origin.x - (72.0 + 8.0 + 4.0 + 9.0 + 9.0 + 4.0)).abs() < 0.01);
    assert!((fi.origin.y - 720.0).abs() < 0.01);
    assert!((fi.font_size - 16.0).abs() < 0.01);
}

#[test]
fn a_form_drawn_inside_a_span_belongs_to_it() {
    // The span's glyphs live in a form XObject; the replacement still lands on them, once.
    // The `b` that follows is drawn a word gap away, which is the one space in the result.
    let inner = form("BT /F1 12 Tf 80 720 Td <0102> Tj ET");
    assert_eq!(
        text_with(
            "BT /F1 12 Tf 72 720 Td (a) Tj ET \
             /Span <</ActualText (68)>> BDC /Fm1 Do EMC \
             BT /F1 12 Tf 100 720 Td (b) Tj ET",
            "/XObject << /Fm1 6 0 R >>",
            &[inner],
        ),
        "a68 b"
    );
}

/// The head of a Springer reference list, operator for operator: the space before the label and
/// the one after it are each tagged with a tab, and the space in "et al." with a no-break space.
const SPRINGER_REFERENCE: &str = "BT /F1 1 Tf 8 0 0 8 56.69 700 Tm \
     /Span<</ActualText<FEFF0009>>>BDC 0 -1.2 Td ( )Tj EMC \
     .133 0 Td (1.)Tj \
     /Span<</ActualText<FEFF0009>>>BDC ( )Tj EMC \
     1.867 0 Td [(Lahir)-30(y P)138(, et)]TJ \
     /Span<</ActualText<FEFF00A0>>>BDC ( )Tj EMC \
     [(al)11(. K)-15(inase mutations)]TJ ET";

#[test]
fn a_space_tagged_as_a_tab_or_no_break_space_is_still_a_space() {
    // 0.1.2 passed the tags through, so the label read "1.\tLahiry" and anything splitting on
    // spaces saw it fused to the entry.
    assert_eq!(
        text(SPRINGER_REFERENCE),
        " 1. Lahiry P, et al. Kinase mutations"
    );
}

#[test]
fn layout_marks_in_actual_text_are_not_content() {
    let tagged = |value: &str| {
        text(&format!(
            "BT /F1 12 Tf 72 720 Td (a) Tj /Span<</ActualText {value}>>BDC ( ) Tj EMC (b) Tj ET"
        ))
    };
    // Every kind of space, and the control characters Springer uses as column gaps.
    for space in ["0009", "00A0", "2002", "2009", "202F", "0007", "0008"] {
        assert_eq!(tagged(&format!("<FEFF{space}>")), "a b", "U+{space}");
    }
    // Break opportunities and direction marks have no width and no text: the URL they were
    // typeset into reads unbroken.
    for mark in ["200B", "200E", "200F", "2060", "00AD"] {
        assert_eq!(tagged(&format!("<FEFF{mark}>")), "ab", "U+{mark}");
    }
    // A non-breaking hyphen is a hyphen, and real replacement text around a mark survives.
    assert_eq!(tagged("<FEFF2011>"), "a-b");
    assert_eq!(tagged("<FEFF002F200B>"), "a/b");
}

#[test]
fn a_silenced_glyph_at_the_head_of_a_line_does_not_break_it_twice() {
    assert_eq!(
        text(
            "BT /F1 12 Tf 72 720 Td (one) Tj 0 -14 Td \
             /Span<</ActualText<FEFF200B>>>BDC ( ) Tj EMC (two) Tj ET"
        ),
        "one\ntwo"
    );
}

#[test]
fn a_form_cannot_close_or_leak_into_its_callers_marked_content() {
    // The form closes a sequence it never opened and leaves one of its own open. Neither may
    // reach the page: the page's span survives the stray EMC, and the form's unclosed span
    // does not swallow the text drawn after the form returns.
    let inner = form("EMC BT /F1 12 Tf 80 720 Td <01> Tj ET /Span <</ActualText (LEAK)>> BDC");
    assert_eq!(
        text_with(
            "/Span <</ActualText (6)>> BDC /Fm1 Do BT /F1 12 Tf 90 720 Td <02> Tj ET EMC \
             /Fm1 Do BT /F1 12 Tf 100 720 Td (b) Tj ET",
            "/XObject << /Fm1 6 0 R >>",
            &[inner],
        ),
        "6b"
    );
}
