# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

[Unreleased]: https://github.com/pgarrett-scripps/rustium-pdf/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/pgarrett-scripps/rustium-pdf/releases/tag/v0.1.0
