# rustium-pdf

Pure-Rust extraction of PDF page primitives — glyphs with geometry, vector paths, images — plus
page rendering.

No C library, no FFI, no global state.

This is the default PDF backend of
[`rustypaper`](https://github.com/pgarrett-scripps/rustypaper), where it took over the slice of
[pdfium](https://pdfium.googlesource.com/pdfium/) that project used to require; pdfium remains
there as an opt-in feature. Observable semantics match pdfium's where downstream code depends on
them: generated space glyphs, soft-hyphen stripping, and y-down page-space helpers.

## Install

```toml
[dependencies]
rustium-pdf = "0.1"
```

The package is `rustium-pdf`; the crate imports as `rustium_pdf`. API documentation is at
[docs.rs/rustium-pdf](https://docs.rs/rustium-pdf).

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

`Document::from_bytes` opens a document already in memory instead of a path.

Four examples are included:

```sh
cargo run --example dump   -- file.pdf [page]                 # primitives with geometry
cargo run --example text   -- file.pdf [page]                 # text, form feed between pages
cargo run --example render -- file.pdf [page] [dpi] [out.png] # rasterise
cargo run --example fonts  -- file.pdf                        # per-font unmapped-glyph rates
```

## What it does

| Area | Coverage |
| --- | --- |
| File structure | xref tables and streams, hybrid `/XRefStm`, object streams, incremental updates, brute-force recovery for damaged files |
| Encryption | RC4 and AES under the standard security handler, revisions 2 through 6 |
| Filters | Flate, LZW, ASCIIHex, ASCII85, RunLength, with PNG/TIFF predictors |
| Content | Full graphics and text state machine, form XObjects, inline images, Type3 glyph procedures |
| Fonts | Simple, Type0/CID and Type3; encodings with `/Differences`; `/ToUnicode`; the builtin `/Encoding` of an embedded Type1 program; CID `/W` arrays; built-in metrics for the standard 14; Microsoft symbol-font `U+F0xx` code points rewritten as the symbols they stand for |
| Mathematics | TeX size variants (`summationdisplay`, `parenleftBig`) and extensible delimiters built from stacked pieces, both resolved to single characters |
| Outlines | TrueType, OpenType and bare CFF via `ttf-parser` |
| Rendering | tiny-skia rasteriser honouring content-stream paint order, with region cropping and PNG output |

## Threading

`Document` is `Send + Sync` with no global state, so pages can be extracted and rendered
concurrently from a single open document — the property pdfium's process-wide, single-threaded
design cannot offer. A compile-time assertion in the test build keeps it that way, so a stray
`Rc` or `Cell` cannot take it away unnoticed.

## Fonts without embedded programs

A document may name Helvetica or Times and embed nothing. Extraction is unaffected: advances come
from built-in metrics for the standard 14. Rendering substitutes a system face chosen by style,
preferring the metrically compatible Liberation family, then DejaVu, then Noto. Set
`RUSTIUM_FONT_DIR` to pin the search to fonts you ship.

Positions and advances always come from the document, never the substitute, so a missing font
shifts nothing — it only draws letters slightly wide or narrow inside slots the PDF chose.

## Symbolic fonts that name no encoding

A simple font marked symbolic may omit `/Encoding` entirely, and TeX's Computer Modern does —
`/Flags 4`, no `/Encoding`, no `/ToUnicode`. Its embedded Type1 program is then the only record
of what each code means, so the builtin `/Encoding` array is read out of the program's cleartext
header, ahead of `eexec`. Skipping that step is not a partial loss: every glyph on such a page
lands in exactly the right place carrying no text at all.

## Scope

This is built for **academic paper extraction** — preprints and journal PDFs with a real text
layer. That focus is deliberate, and the following are explicit non-goals:

- **Scanned and image-only pages.** Reading them needs OCR. `Page::is_likely_scanned` reports
  the case so a caller can say why a document yielded nothing, rather than returning an empty
  page as though it had succeeded. A page carrying an invisible OCR text layer is ordinary text
  and extracts normally.
- **Reading order.** Primitives come out in content-stream order with their geometry attached.
  Ordering a two-column page is layout, and belongs to the consumer.
- **Forms, annotations, tagged-PDF structure and JavaScript.**

Encrypted documents *are* handled — publisher-typeset PDFs are routinely encrypted with an empty
user password, so refusing them would refuse ordinary papers. `Document::open` tries the empty
password; `Document::open_with_password` takes one. Certificate-based encryption is refused.

## Known gaps

- Type1 `/FontFile` programs yield metrics, encodings and text, but no outlines — their
  charstrings are eexec-encrypted and are not interpreted. CFF and TrueType do yield outlines.
- A bare CFF program's own builtin encoding is not read; its `/Encoding` or `/ToUnicode` is.
- A glyph drawn from a substitute face reports the document's advance as its horizontal extent
  rather than the substitute's ink, since that ink is another typeface's. Vertical extents are
  the substitute's.
- Predefined non-Identity CJK CMaps fall back to the identity mapping.
- `JPXDecode`, `CCITTFaxDecode` and `JBIG2Decode` images are passed through undecoded and skipped
  when rendering. `DCTDecode` (JPEG) and all byte-level filters are handled.
- Clipping is tracked as a bounding rectangle rather than an arbitrary path. It over-covers, so
  it is safe to reject content with and unsafe to include content by.

## Status

95 tests. Verified against real-world documents at 100% glyph-outline resolution and roughly
17 ms/page rendering at 110 dpi.

Parsing is measured by agreement with poppler over two corpora — ten arXiv papers, all from TeX,
and eight bioRxiv preprints from five unrelated producers. Mean word-bigram recall **0.974** and
word-set Jaccard **0.960** on the arXiv set, **0.995** and **0.992** on bioRxiv. `corpus_diverse/`
holds that harness, and says what it does and does not tell you.

How well those characters then *convert* to Markdown is a separate question, belonging to the
consumer rather than to a parser. Measured as the backend for `rustypaper` across its
ten-paper corpus, against that project's recorded pdfium baseline: prose bigram recall **0.891**
(pdfium 0.894), equation recall **0.370** (pdfium 0.375), equation fidelity **0.547** (pdfium
0.557). That corpus passes all 31 of its integration tests on either backend, and rustium-pdf
converts it in 2.06 s against pdfium's 1.94 s while holding 63 MB of resident memory against
pdfium's 95 MB. Those figures are quoted only to show this crate is not the limiting factor:
equation recall landing within 0.005 of pdfium places that ceiling in the consumer's equation
detection, not in either engine's extraction.

## License

MIT OR Apache-2.0
