//! Dumps a PDF's extracted primitives: `cargo run --example dump -- file.pdf [page]`.

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(path) = args.next() else {
        eprintln!("usage: dump <file.pdf> [page-index]");
        std::process::exit(2);
    };
    let only: Option<usize> = args.next().and_then(|s| s.parse().ok());

    let doc = match rustium_pdf::Document::open(&path) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("{path}: {e}");
            std::process::exit(1);
        }
    };
    println!("{path}: {} pages", doc.page_count());

    let range: Vec<usize> = match only {
        Some(i) => vec![i],
        None => (0..doc.page_count()).collect(),
    };
    let (mut glyphs, mut paths, mut images) = (0usize, 0usize, 0usize);
    let mut fonts: std::collections::BTreeSet<String> = Default::default();

    for i in range {
        let page = match doc.page(i) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("page {i}: {e}");
                continue;
            }
        };
        glyphs += page.glyphs.len();
        paths += page.paths.len();
        images += page.images.len();
        for g in &page.glyphs {
            if !g.base_font.is_empty() {
                fonts.insert(g.base_font.clone());
            }
        }
        let text = page.text();
        println!(
            "\n--- page {i} ({:.0}x{:.0}, rot {}) {} glyphs, {} paths, {} images ---",
            page.width(),
            page.height(),
            page.rotation,
            page.glyphs.len(),
            page.paths.len(),
            page.images.len()
        );
        println!("{}", text.chars().take(600).collect::<String>());
    }

    println!("\n=== totals: {glyphs} glyphs, {paths} paths, {images} images ===");
    println!("fonts: {fonts:?}");
}
