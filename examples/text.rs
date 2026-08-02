//! Extracts a document's text: `cargo run --example text -- file.pdf [page]`.
//!
//! Pages are separated by a form feed, matching what `pdftotext` emits, so the output can be
//! diffed against other extractors directly.

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(path) = args.next() else {
        eprintln!("usage: text <file.pdf> [page-index]");
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

    let pages: Vec<usize> = match only {
        Some(i) => vec![i],
        None => (0..doc.page_count()).collect(),
    };
    for (n, i) in pages.iter().enumerate() {
        if n > 0 {
            print!("\u{000C}");
        }
        match doc.page(*i) {
            Ok(page) => println!("{}", page.text()),
            Err(e) => eprintln!("page {i}: {e}"),
        }
    }
}
