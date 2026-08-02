//! Renders a page to PNG: `cargo run --example render -- file.pdf [page] [dpi] [out.png]`.

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(path) = args.next() else {
        eprintln!("usage: render <file.pdf> [page] [dpi] [out.png]");
        std::process::exit(2);
    };
    let index: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let dpi: f32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(150.0);
    let out = args.next().unwrap_or_else(|| "page.png".into());

    let doc = rustium_pdf::Document::open(&path).expect("open");
    let page = doc.page(index).expect("page");
    let started = std::time::Instant::now();
    let image = page
        .render(&doc, rustium_pdf::RenderOptions::at_dpi(dpi))
        .expect("render");
    let elapsed = started.elapsed();

    std::fs::write(&out, image.to_png().expect("encode")).expect("write");
    println!(
        "{out}: {}x{} at {dpi}dpi in {:.0}ms ({} glyphs, {} paths, {} images)",
        image.width,
        image.height,
        elapsed.as_secs_f32() * 1000.0,
        page.glyphs.len(),
        page.paths.len(),
        page.images.len()
    );
}
