# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.3] - 2026-11-14

### Fixed

- **0.1.2 dropped the space after numbered reference labels**, and many other plain spaces, in
  Springer-typeset PDFs: "1. Lahiry" read as "1.<tab>Lahiry" and "et al." as "et<nbsp>al.".
  That typesetter tags ordinary space glyphs with an `/ActualText` saying how the line was laid
  out — a tab after a list label, a no-break or thin space where a line must not break, a
  control character at a column gap — and 0.1.2 passed those through as the text. Any
  whitespace or control character in a replacement now reads as a space, as a line feed already
  did.
- The same typesetter's zero-width marks in a replacement (zero-width space, word joiner,
  left-to-right and right-to-left marks) are dropped as the soft hyphen is, so a URL or DOI it
  allowed to break reads unbroken ("https://doi.org/10.1186/…" rather than
  "https:// doi. org/ 10. 1186/ …"), and a non-breaking hyphen reads as a hyphen.
- `Page::text` no longer starts a line twice when the first glyph on it has no text.

## [0.1.2] - 2026-11-14

### Fixed

- **`/ActualText` is honoured.** Inside a marked-content sequence that carries it, the
  replacement is the text and the glyphs keep only their geometry: the first glyph of the
  sequence holds the whole replacement, the rest hold nothing. Chromium-printed PDFs draw
  ligatures and some digits as glyphs with no `/ToUnicode` entry and name them only this way,
  so they previously extracted as "signicant" with every 6 and 8 missing from the reference
  numbers. Nested sequences defer to the outermost replacement, an empty one silences its
  glyphs, the value is decoded as a text string (UTF-16 behind a byte-order mark,
  PDFDocEncoding otherwise), properties are followed through the `/Properties` resources, and a
  form XObject drawn inside a sequence is part of it. A line break in the replacement reads as
  a space: Chromium tags spaces with a line feed at HTML block boundaries, which are often
  mid-line, and line breaks remain a matter of geometry.

### Added

- Committed regression fixtures for hostile input (`tests/hostile_input.rs`) and a bounded
  `cargo fuzz` target, neither of which ships in the crate.

## [0.1.1] - 2026-08-13

The reader survives hostile input. No API change.

### Fixed

- **Deeply nested objects no longer overflow the stack.** The object grammar recursed once per
  `[` and once per `{`, so a hundred thousand delimiter bytes aborted the process. Nesting now
  stops at a depth of 150, and braces and stray delimiters are skipped without recursion.
- **Decompression is bounded.** Flate and LZW stop at 256 MB of output and keep the prefix, the
  same way a corrupt tail is handled, so a kilobyte of stream cannot ask for gigabytes.
- **CMYK image colour.** Samples were inverted twice, which rendered pale cyan as dark red.
  They now convert the way vector colours do.
- An empty RC4 key returns the data unchanged instead of indexing an empty slice.
- Object numbers taken from xref subsection headers and `/W` arrays are formed with checked
  arithmetic; entries that run off the end of the number space are skipped.

## [0.1.0] - 2026-08-02

First release.

### Added

- **File structure** — cross-reference tables and streams, hybrid `/XRefStm`, object streams,
  incremental updates, and a brute-force recovery scan for files whose xref is unusable.
- **Encryption** — RC4 and AES under the standard security handler, revisions 2 through 6, with
  the supplied password tried as both the user and the owner password. Certificate-based files
  are refused.
- **Filters** — Flate, LZW, ASCIIHex, ASCII85 and RunLength, with PNG and TIFF predictors.
  Image codecs are reported by name and passed through.
- **Content interpretation** — the graphics and text state machines, path construction and
  painting, form XObjects with recursion guards, inline images, and Type3 glyph procedures.
  Primitives carry a shared paint-order stamp so z-order survives the split into per-type
  vectors.
- **Fonts** — Simple, Type0/CID and Type3; base encodings with `/Differences`; `/ToUnicode`
  CMaps; CID `/W` arrays; built-in metrics for the standard 14 fonts; and typeface flags
  corroborated from the descriptor, weight, italic angle and name. A symbolic font that names no
  encoding is read through the builtin `/Encoding` array of its embedded Type1 program, which
  TeX's Computer Modern needs to produce any text at all.
- **Glyph names to text** — the Adobe Glyph List subset that scientific documents use, plus TeX
  size variants (`summationdisplay`, `parenleftBig`) and extensible delimiters built from stacked
  pieces, each resolved to a single character. Microsoft symbol-font `/ToUnicode` maps that hand
  back `U+F0xx` private-use code points are rewritten as the symbols they stand for.
- **Glyph outlines** — from embedded TrueType, OpenType and bare CFF programs, cached per glyph.
- **Font substitution** — documents that embed no program fall back to a style-matched system
  face, preferring the metrically compatible Liberation family. Positions always come from the
  document, so substitution never shifts layout. `RUSTIUM_FONT_DIR` pins the search path.
- **Rendering** — a tiny-skia rasteriser replaying primitives in content-stream paint order,
  with region cropping, configurable resolution, per-type suppression and PNG output.
- `Document` is `Send + Sync`, asserted at compile time.

### Known limitations

- Type1 `/FontFile` programs yield metrics and text but no outlines.
- A bare CFF program's own builtin encoding is not read; its `/Encoding` or `/ToUnicode` is.
- Predefined non-Identity CJK CMaps fall back to the identity mapping.
- `JPXDecode`, `CCITTFaxDecode` and `JBIG2Decode` images are not decoded and are skipped when
  rendering. `DCTDecode` (JPEG) is handled.
- Clipping is tracked as a bounding rectangle rather than an arbitrary path.

[Unreleased]: https://github.com/pgarrett-scripps/rustium-pdf/compare/v0.1.3...HEAD
[0.1.3]: https://github.com/pgarrett-scripps/rustium-pdf/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/pgarrett-scripps/rustium-pdf/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/pgarrett-scripps/rustium-pdf/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/pgarrett-scripps/rustium-pdf/releases/tag/v0.1.0
