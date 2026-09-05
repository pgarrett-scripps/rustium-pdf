#![no_main]

use libfuzzer_sys::fuzz_target;
use rustium_pdf::{Document, Rect, RenderOptions};

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
            // Bound the requested raster independently of attacker-controlled page size.
            // At 8 dpi this viewport is at most 256 by 256 pixels.
            let region = Rect::from_corners(0.0, 0.0, 2304.0, 2304.0);
            let _ = page.render_region(&document, Some(region), RenderOptions::at_dpi(8.0));
        }
    }
});
