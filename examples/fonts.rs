//! Per-font glyph coverage: `cargo run --example fonts -- file.pdf`.
//!
//! Reports, for every font on the page, how many of its glyphs produced no text at all. A glyph
//! with no text is invisible to every downstream consumer, so a font with a high unmapped rate
//! is the single most useful thing to know when text is missing — and mathematics fonts are
//! where it usually happens, because TeX's are symbolic, often carry no `/ToUnicode`, and use
//! glyph names no standard encoding lists.

use std::collections::BTreeMap;

#[derive(Default)]
struct Tally {
    total: usize,
    unmapped: usize,
    flags: u32,
    sample: Vec<u32>,
}

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(path) = args.next() else {
        eprintln!("usage: fonts <file.pdf>");
        std::process::exit(2);
    };
    let doc = match rustium_pdf::Document::open(&path) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("{path}: {e}");
            std::process::exit(1);
        }
    };

    let mut fonts: BTreeMap<String, Tally> = BTreeMap::new();
    for i in 0..doc.page_count() {
        let Ok(page) = doc.page(i) else { continue };
        for g in page.glyphs.iter().filter(|g| !g.is_generated_space) {
            let t = fonts.entry(g.base_font.clone()).or_default();
            t.total += 1;
            t.flags = g.flags.0;
            if g.text.is_empty() {
                t.unmapped += 1;
                if t.sample.len() < 12 {
                    t.sample.push(g.code());
                }
            }
        }
    }

    let (mut total, mut unmapped) = (0usize, 0usize);
    println!(
        "{:<40} {:>8} {:>9}  unmapped codes",
        "font", "glyphs", "unmapped"
    );
    println!("{}", "-".repeat(96));
    for (name, t) in &fonts {
        total += t.total;
        unmapped += t.unmapped;
        let pct = if t.total > 0 {
            t.unmapped as f32 * 100.0 / t.total as f32
        } else {
            0.0
        };
        let codes: Vec<String> = t.sample.iter().map(|c| format!("{c:#04x}")).collect();
        let marker = if pct > 5.0 { " <<<" } else { "" };
        println!(
            "{:<40} {:>8} {:>7.1}%  {}{}",
            name.chars().take(40).collect::<String>(),
            t.total,
            pct,
            codes.join(" "),
            marker
        );
    }
    println!("{}", "-".repeat(96));
    let pct = if total > 0 {
        unmapped as f32 * 100.0 / total as f32
    } else {
        0.0
    };
    println!("{:<40} {:>8} {:>7.1}%", "TOTAL", total, pct);
}
