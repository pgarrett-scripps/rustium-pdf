# rustium-pdf

Pure-Rust extraction of PDF page primitives — glyphs with geometry, vector paths, images — plus
page rendering. A thread-safe replacement for the slice of [pdfium](https://pdfium.googlesource.com/pdfium/)
that [`rustypdf2markdown`](https://github.com/pgarrett-scripps/rustypdf2markdown) uses, with the
same observable semantics where downstream code depends on them: generated space glyphs,
soft-hyphen stripping, and y-down page-space helpers.

No C library, no FFI, no global state.

## Install

```toml
[dependencies]
rustium-pdf = "0.1"
```

The package is `rustium-pdf`; the crate imports as `rustium_pdf`.

Minimum supported Rust version is **1.88**, set by the `image` dependency and verified in CI.

## Usage

```rust
let doc = rustium_pdf::Document::open("paper.pdf")?;
let page = doc.page(0)?;

// Primitives are in user space (y-up); `page_matrix` converts to y-down device space.
for glyph in page.glyphs.iter().filter(|g| g.is_visible()) {
    println!("{:?} at {:?} {}pt bold={}",
        glyph.text, glyph.origin, glyph.font_size, glyph.flags.is_bold());
}

let png = page.render(&doc, rustium_pdf::RenderOptions::at_dpi(150.0))?.to_png()?;
```

Two examples are included:

```sh
cargo run --example dump   -- file.pdf [page]                 # primitives and text
cargo run --example render -- file.pdf [page] [dpi] [out.png] # rasterise
```

## What it does

| Area | Coverage |
| --- | --- |
| File structure | xref tables and streams, hybrid `/XRefStm`, object streams, incremental updates, brute-force recovery for damaged files |
| Encryption | RC4 and AES, standard security handler |
| Filters | Flate, LZW, ASCIIHex, ASCII85, RunLength, with PNG/TIFF predictors |
| Content | Full graphics and text state machine, form XObjects, inline images, Type3 glyph procedures |
| Fonts | Simple, Type0/CID and Type3; encodings with `/Differences`; `/ToUnicode`; CID `/W` arrays; built-in metrics for the standard 14 |
| Outlines | TrueType, OpenType and bare CFF via `ttf-parser` |
| Rendering | tiny-skia rasteriser honouring content-stream paint order, with region cropping and PNG output |

## Threading

`Document` is `Send + Sync` with no global state, so pages can be extracted and rendered
concurrently from a single open document — the property pdfium's process-wide, single-threaded
design cannot offer. A compile-time assertion in `page.rs` keeps it that way.

## Fonts without embedded programs

A document may name Helvetica or Times and embed nothing. Extraction is unaffected: advances come
from built-in metrics for the standard 14. Rendering substitutes a system face chosen by style,
preferring the metrically compatible Liberation family, then DejaVu, then Noto. Set
`RUSTIUM_FONT_DIR` to pin the search to fonts you ship.

Positions and advances always come from the document, never the substitute, so a missing font
shifts nothing — it only draws letters slightly wide or narrow inside slots the PDF chose.

## Known gaps

- Type1 `/FontFile` programs yield metrics and text but no outlines; CFF and TrueType do.
- Predefined non-Identity CJK CMaps fall back to the identity mapping.
- `JPXDecode`, `CCITTFaxDecode` and `JBIG2Decode` images are passed through undecoded and skipped
  when rendering. `DCTDecode` (JPEG) and all byte-level filters are handled.
- Clipping is tracked as a bounding rectangle rather than an arbitrary path. It over-covers, so
  it is safe to reject content with and unsafe to include content by.

## Status

67 tests. Verified against real-world documents at 100% glyph-outline resolution and roughly
17 ms/page rendering at 110 dpi.

## License

MIT OR Apache-2.0
