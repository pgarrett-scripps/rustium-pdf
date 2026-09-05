//! Committed parser regressions, independent of downloaded corpora.

use rustium_pdf::Document;

const VALID: &[u8] = include_bytes!("fixtures/minimal.pdf");
const CYCLIC: &[u8] = include_bytes!("fixtures/cyclic-pages.pdf");

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
