//! Committed parser regressions, independent of downloaded corpora.

use rustium_pdf::{Document, Rect, RenderOptions};

const VALID: &[u8] = include_bytes!("fixtures/minimal.pdf");
const CYCLIC: &[u8] = include_bytes!("fixtures/cyclic-pages.pdf");
const OVERSIZED: &[u8] = include_bytes!("fixtures/oversized-page.pdf");

#[test]
fn an_oversized_page_can_be_rendered_within_a_bounded_viewport() {
    let document = Document::from_bytes(OVERSIZED.to_vec(), None).expect("catalog parses");
    let page = document.page(0).expect("page parses");
    assert!(page.width() > 2304.0 || page.height() > 2304.0);
    let region = Rect::from_corners(0.0, 0.0, 2304.0, 2304.0);
    let raster = page
        .render_region(&document, Some(region), RenderOptions::at_dpi(8.0))
        .expect("bounded viewport renders");
    assert!(raster.width <= 256 && raster.height <= 256);
    assert!(raster.rgba.len() <= 256 * 256 * 4);
}

#[test]
fn a_cyclic_page_tree_terminates() {
    let document = Document::from_bytes(CYCLIC.to_vec(), None).expect("catalog parses");
    assert_eq!(document.page_count(), 0);
}

#[test]
fn truncation_at_every_offset_does_not_panic() {
    for end in 0..VALID.len() {
        if let Ok(document) = Document::from_bytes(VALID[..end].to_vec(), None) {
            for index in 0..document.page_count().min(4) {
                let _ = document.page(index);
            }
        }
    }
}

#[test]
fn mutations_of_the_object_structure_do_not_panic() {
    for offset in 0..VALID.len() {
        for byte in [0, b'[', b'/', 255] {
            let mut bytes = VALID.to_vec();
            bytes[offset] = byte;
            if let Ok(document) = Document::from_bytes(bytes, None) {
                let _ = document.page(0);
            }
        }
    }
}
