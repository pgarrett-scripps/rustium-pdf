//! # rustium
//!
//! Pure-Rust extraction of PDF page primitives — glyphs with geometry, vector paths, images —
//! plus page rendering. A thread-safe replacement for the slice of pdfium that
//! `rustypdf2markdown` uses, with the same observable semantics where downstream code depends
//! on them (generated space glyphs, soft-hyphen stripping, y-down page space helpers).
//!
//! ```no_run
//! # fn main() -> rustium::Result<()> {
//! let doc = rustium::Document::open("paper.pdf")?;
//! let page = doc.page(0)?;
//!
//! // Primitives are in user space (y-up); `page_matrix` converts to y-down device space.
//! for glyph in page.glyphs.iter().filter(|g| g.is_visible()) {
//!     println!("{:?} at {:?} {}pt bold={}",
//!         glyph.text, glyph.origin, glyph.font_size, glyph.flags.is_bold());
//! }
//!
//! let png = page.render(&doc, rustium::RenderOptions::at_dpi(150.0))?.to_png()?;
//! # Ok(())
//! # }
//! ```
//!
//! ## Threading
//!
//! [`Document`] is `Send + Sync` with no global state, so pages can be extracted and rendered
//! concurrently from one open document — the property pdfium's process-wide, single-threaded
//! design cannot offer.
//!
//! ## Fonts without embedded programs
//!
//! A document may name Helvetica or Times and embed nothing. Extraction is unaffected: advances
//! come from built-in metrics for the standard 14. Rendering substitutes a system face, chosen
//! by style and preferring the metrically compatible Liberation family. Set `RUSTIUM_FONT_DIR`
//! to pin the search to fonts you ship. Positions always come from the document, never the
//! substitute, so a missing font shifts nothing.
//!
//! ## Known gaps
//!
//! * Type1 `/FontFile` programs yield metrics and text but no outlines; CFF and TrueType do.
//! * Predefined non-Identity CJK CMaps fall back to the identity mapping.
//! * `JPXDecode`, `CCITTFaxDecode` and `JBIG2Decode` images are passed through undecoded and
//!   are skipped when rendering; `DCTDecode` (JPEG) and the byte filters are handled.
//! * Clipping is tracked as a bounding rectangle, not an arbitrary path.

pub mod content;
pub mod crypt;
pub mod document;
pub mod error;
pub mod filters;
pub mod font;
pub mod geom;
pub mod lexer;
pub mod object;
pub mod page;
pub mod parser;
pub mod render;

pub use document::Document;
pub use error::{Error, Result};
pub use font::{Font, FontFlags};
pub use geom::{Matrix, Point, Rect};
pub use object::{Dict, Object};
pub use page::{Glyph, ImageData, ImageItem, Page, PathItem};
pub use render::{RenderOptions, Rendered};
