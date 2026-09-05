#![no_main]

use libfuzzer_sys::fuzz_target;
use rustium_pdf::{Document, RenderOptions};

fuzz_target!(|data: &[u8]| {
    if data.len() > 262_144 {
        return;
    }
    let Ok(document) = Document::from_bytes(data.to_vec(), None) else {
        return;
    };
    for index in 0..document.page_count().min(4) {
        if let Ok(page) = document.page(index) {
            let _ = page.text();
            // The fuzzer has an RSS limit. The renderer must reject invalid geometry.
            let _ = page.render(&document, RenderOptions::at_dpi(8.0));
        }
    }
});
