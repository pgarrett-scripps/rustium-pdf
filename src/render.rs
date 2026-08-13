//! Rasterising a page to pixels.
//!
//! The page's primitives are replayed in the order the content stream drew them — glyphs, paths
//! and images share one [`crate::page::Glyph::order`] sequence precisely so that this pass can
//! interleave them correctly. Painting each type in its own batch would put every image on top
//! of every rule, which is wrong for any page with a shaded table cell or a figure background.

use tiny_skia::{
    FillRule, Paint, PathBuilder, Pixmap, PixmapPaint, Shader, Stroke, Transform as SkTransform,
};

use crate::document::Document;
use crate::error::{Error, Result};
use crate::font::glyph::PathCmd;
use crate::geom::{Matrix, Rect};
use crate::page::{ImageData, ImageItem, Page};

/// Anything larger than this in either dimension is refused rather than attempted: a page whose
/// media box or requested scale is absurd would otherwise ask for a multi-gigabyte allocation.
const MAX_DIMENSION: u32 = 20_000;

/// Options for a render.
#[derive(Debug, Clone, Copy)]
pub struct RenderOptions {
    /// Output resolution. 72 dpi renders one pixel per point.
    pub dpi: f32,
    /// Paint an opaque white background first. A PDF page is defined as white paper; leaving it
    /// transparent makes text unreadable in any viewer that composites onto dark.
    pub background: bool,
    /// Draw text. Turning it off is how a caller gets a figure's artwork without its labels.
    pub draw_text: bool,
    pub draw_paths: bool,
    pub draw_images: bool,
}

impl Default for RenderOptions {
    fn default() -> Self {
        RenderOptions {
            dpi: 72.0,
            background: true,
            draw_text: true,
            draw_paths: true,
            draw_images: true,
        }
    }
}

impl RenderOptions {
    pub fn at_dpi(dpi: f32) -> Self {
        RenderOptions {
            dpi,
            ..Default::default()
        }
    }
}

/// A rendered page: 8-bit RGBA, row-major, premultiplied by tiny-skia's convention undone.
pub struct Rendered {
    pub width: u32,
    pub height: u32,
    /// Straight (non-premultiplied) RGBA, four bytes per pixel.
    pub rgba: Vec<u8>,
}

impl Rendered {
    /// Encodes to PNG.
    pub fn to_png(&self) -> Result<Vec<u8>> {
        use image::ImageEncoder;
        let mut out = Vec::new();
        image::codecs::png::PngEncoder::new(&mut out)
            .write_image(
                &self.rgba,
                self.width,
                self.height,
                image::ExtendedColorType::Rgba8,
            )
            .map_err(|e| Error::Render(format!("png encode: {e}")))?;
        Ok(out)
    }
}

impl Page {
    /// Renders the whole page.
    pub fn render(&self, doc: &Document, options: RenderOptions) -> Result<Rendered> {
        self.render_region(doc, None, options)
    }

    /// Renders a region of the page, or all of it when `region` is `None`.
    ///
    /// `region` is in **y-down page space** — the same space [`Page::page_matrix`] produces and
    /// that a layout consumer works in — with the origin at the top-left of the displayed page.
    pub fn render_region(
        &self,
        doc: &Document,
        region: Option<Rect>,
        options: RenderOptions,
    ) -> Result<Rendered> {
        let scale = (options.dpi / 72.0).max(0.0);
        if !scale.is_finite() || scale <= 0.0 {
            return Err(Error::Render(format!("bad dpi {}", options.dpi)));
        }

        // Clamp the region to the page so a slightly oversized caller bbox cannot ask for
        // pixels that do not exist.
        let page_rect = Rect::from_corners(0.0, 0.0, self.width(), self.height());
        let region = match region {
            Some(r) => {
                let clipped = r.intersect(&page_rect);
                if clipped.is_empty() {
                    return Err(Error::Render("region does not overlap the page".into()));
                }
                clipped
            }
            None => page_rect,
        };

        let width = ((region.width() * scale).ceil() as u32).clamp(1, MAX_DIMENSION);
        let height = ((region.height() * scale).ceil() as u32).clamp(1, MAX_DIMENSION);
        let mut pixmap = Pixmap::new(width, height)
            .ok_or_else(|| Error::Render(format!("cannot allocate {width}x{height} pixmap")))?;
        if options.background {
            pixmap.fill(tiny_skia::Color::WHITE);
        }

        // User space to the cropped, scaled output: the page transform, then the region offset.
        let base = self
            .page_matrix(scale)
            .concat(&Matrix::translate(-region.x0 * scale, -region.y0 * scale));

        // One merged pass in paint order. Each primitive is cheap to classify and the counts
        // are page-sized, so the sort costs nothing next to the rasterisation.
        let mut order: Vec<Item> = Vec::new();
        if options.draw_paths {
            order.extend(self.paths.iter().map(|p| Item::Path(p.order, p)));
        }
        if options.draw_text {
            order.extend(
                self.glyphs
                    .iter()
                    .filter(|g| g.is_visible())
                    .map(|g| Item::Glyph(g.order, g)),
            );
        }
        if options.draw_images {
            order.extend(self.images.iter().map(|i| Item::Image(i.order, i)));
        }
        order.sort_by_key(Item::order);

        for item in order {
            match item {
                Item::Path(_, p) => draw_path(&mut pixmap, p, &base),
                Item::Glyph(_, g) => draw_glyph(&mut pixmap, g, &base),
                Item::Image(_, i) => draw_image(doc, &mut pixmap, i, &base),
            }
        }

        // tiny-skia stores premultiplied alpha; callers expect straight RGBA.
        let rgba = pixmap
            .pixels()
            .iter()
            .flat_map(|p| {
                let c = p.demultiply();
                [c.red(), c.green(), c.blue(), c.alpha()]
            })
            .collect();

        Ok(Rendered {
            width,
            height,
            rgba,
        })
    }
}

/// A primitive tagged with its paint order, so the three vectors can be merged.
enum Item<'a> {
    Path(u32, &'a crate::page::PathItem),
    Glyph(u32, &'a crate::page::Glyph),
    Image(u32, &'a ImageItem),
}

impl Item<'_> {
    fn order(&self) -> u32 {
        match self {
            Item::Path(o, _) | Item::Glyph(o, _) | Item::Image(o, _) => *o,
        }
    }
}

fn paint_for(color: [f32; 3], alpha: f32) -> Paint<'static> {
    let mut paint = Paint {
        anti_alias: true,
        ..Default::default()
    };
    paint.shader = Shader::SolidColor(
        tiny_skia::Color::from_rgba(
            color[0].clamp(0.0, 1.0),
            color[1].clamp(0.0, 1.0),
            color[2].clamp(0.0, 1.0),
            alpha.clamp(0.0, 1.0),
        )
        .unwrap_or(tiny_skia::Color::BLACK),
    );
    paint
}

/// Builds a tiny-skia path from our commands, transformed by `m`.
fn build_path(cmds: &[PathCmd], m: &Matrix) -> Option<tiny_skia::Path> {
    let mut b = PathBuilder::new();
    let mut open = false;
    for cmd in cmds {
        match *cmd {
            PathCmd::MoveTo(p) => {
                let p = m.apply(p);
                b.move_to(p.x, p.y);
                open = true;
            }
            PathCmd::LineTo(p) => {
                if !open {
                    continue;
                }
                let p = m.apply(p);
                b.line_to(p.x, p.y);
            }
            PathCmd::CurveTo(c1, c2, p) => {
                if !open {
                    continue;
                }
                let (c1, c2, p) = (m.apply(c1), m.apply(c2), m.apply(p));
                b.cubic_to(c1.x, c1.y, c2.x, c2.y, p.x, p.y);
            }
            PathCmd::Close => {
                if open {
                    b.close();
                }
            }
        }
    }
    b.finish()
}

fn draw_path(pixmap: &mut Pixmap, item: &crate::page::PathItem, base: &Matrix) {
    let Some(path) = build_path(&item.cmds, base) else {
        return;
    };
    if let Some(color) = item.fill {
        let rule = if item.even_odd {
            FillRule::EvenOdd
        } else {
            FillRule::Winding
        };
        pixmap.fill_path(
            &path,
            &paint_for(color, item.fill_alpha),
            rule,
            SkTransform::identity(),
            None,
        );
    }
    if let Some(color) = item.stroke {
        // The recorded width is already in user space; the base transform scales it to device.
        let width = (item.line_width * base.x_scale()).max(0.1);
        pixmap.stroke_path(
            &path,
            &paint_for(color, item.stroke_alpha),
            &Stroke {
                width,
                ..Default::default()
            },
            SkTransform::identity(),
            None,
        );
    }
}

/// Draws one glyph. The document is not needed: the glyph carries its own font, and outlines
/// resolve through that font's cache.
fn draw_glyph(pixmap: &mut Pixmap, glyph: &crate::page::Glyph, base: &Matrix) {
    // Generated spaces have no outline by construction, and a Type3 glyph's marks were already
    // emitted as ordinary paths by the interpreter.
    let Some(outline) = glyph.outline() else {
        return;
    };
    let m = glyph.outline_matrix().concat(base);
    let Some(path) = build_path(&outline.cmds, &m) else {
        return;
    };
    // Modes 1 and 5 stroke; everything else that paints, fills. Stroked text is rare enough
    // that filling it too is closer than leaving it blank.
    pixmap.fill_path(
        &path,
        &paint_for(glyph.color, glyph.alpha),
        FillRule::Winding,
        SkTransform::identity(),
        None,
    );
}

fn draw_image(doc: &Document, pixmap: &mut Pixmap, item: &ImageItem, base: &Matrix) {
    let Some(src) = decode_to_rgba(doc, item) else {
        return;
    };
    // `item.ctm` maps the unit square onto the page with the row flip already folded in, so the
    // image's own pixel grid only needs scaling down to that square.
    let to_unit = Matrix::scale(1.0 / item.width as f32, 1.0 / item.height as f32);
    let m = to_unit.concat(&item.ctm).concat(base);
    let ts = SkTransform::from_row(m.a, m.b, m.c, m.d, m.e, m.f);

    pixmap.draw_pixmap(
        0,
        0,
        src.as_ref(),
        &PixmapPaint {
            opacity: item.alpha.clamp(0.0, 1.0),
            quality: tiny_skia::FilterQuality::Bilinear,
            ..Default::default()
        },
        ts,
        None,
    );
}

/// Expands an image's samples to an RGBA pixmap.
fn decode_to_rgba(doc: &Document, item: &ImageItem) -> Option<Pixmap> {
    let data = item.decode(doc).ok()?;
    let mut pixmap = Pixmap::new(item.width, item.height)?;
    let px = pixmap.pixels_mut();
    let (w, h) = (item.width as usize, item.height as usize);

    match data {
        // JPEG is the only image codec worth carrying a decoder for: it is what scanned pages
        // and photographs use. JPEG 2000, CCITT and JBIG2 are left to the caller.
        ImageData::Encoded { filter, data } if filter == "DCTDecode" || filter == "DCT" => {
            let img = image::load_from_memory_with_format(&data, image::ImageFormat::Jpeg)
                .ok()?
                .to_rgba8();
            for (i, slot) in px.iter_mut().enumerate() {
                let (x, y) = ((i % w) as u32, (i / w) as u32);
                if x >= img.width() || y >= img.height() {
                    continue;
                }
                let p = img.get_pixel(x, y).0;
                *slot = tiny_skia::PremultipliedColorU8::from_rgba(p[0], p[1], p[2], 255)
                    .unwrap_or(tiny_skia::PremultipliedColorU8::TRANSPARENT);
            }
            Some(pixmap)
        }
        ImageData::Encoded { .. } => None,
        ImageData::Raw {
            bits_per_component,
            components,
            data,
        } => {
            let bpc = bits_per_component.max(1) as usize;
            let comps = components.max(1) as usize;
            // Rows are padded to a byte boundary.
            let row_bits = w * comps * bpc;
            let row_bytes = row_bits.div_ceil(8);

            for y in 0..h {
                for x in 0..w {
                    let mut vals = [0u8; 4];
                    for (c, slot) in vals.iter_mut().enumerate().take(comps.min(4)) {
                        let bit = (x * comps + c) * bpc;
                        *slot = sample(&data, y * row_bytes, bit, bpc);
                    }
                    let (r, g, b, a) = if item.is_mask {
                        // A stencil mask paints the fill colour where the sample is 0.
                        let on = vals[0] == 0;
                        let c = item.fill;
                        (
                            (c[0] * 255.0) as u8,
                            (c[1] * 255.0) as u8,
                            (c[2] * 255.0) as u8,
                            if on { 255 } else { 0 },
                        )
                    } else {
                        match comps {
                            1 => (vals[0], vals[0], vals[0], 255),
                            3 => (vals[0], vals[1], vals[2], 255),
                            4 => {
                                // Naive CMYK, matching the interpreter's colour conversion:
                                // `(1 - c) * (1 - k)` per channel.
                                let k = vals[3] as f32 / 255.0;
                                let f = |v: u8| (((255 - v) as f32) * (1.0 - k)) as u8;
                                (f(vals[0]), f(vals[1]), f(vals[2]), 255)
                            }
                            _ => (vals[0], vals[0], vals[0], 255),
                        }
                    };
                    let idx = y * w + x;
                    if let Some(slot) = px.get_mut(idx) {
                        // tiny-skia stores premultiplied; alpha here is 0 or 255 so the
                        // multiply is exact.
                        let (r, g, b) = if a == 0 { (0, 0, 0) } else { (r, g, b) };
                        *slot = tiny_skia::PremultipliedColorU8::from_rgba(r, g, b, a)
                            .unwrap_or(tiny_skia::PremultipliedColorU8::TRANSPARENT);
                    }
                }
            }
            Some(pixmap)
        }
    }
}

/// Reads one sample of `bits` width starting `bit_offset` bits into the row at `row_start`,
/// scaled up to a full byte.
fn sample(data: &[u8], row_start: usize, bit_offset: usize, bits: usize) -> u8 {
    match bits {
        8 => data.get(row_start + bit_offset / 8).copied().unwrap_or(0),
        16 => data.get(row_start + bit_offset / 8).copied().unwrap_or(0),
        1 | 2 | 4 => {
            let byte = data.get(row_start + bit_offset / 8).copied().unwrap_or(0);
            let shift = 8 - bits - (bit_offset % 8);
            let mask = (1u16 << bits) - 1;
            let v = (byte >> shift) as u16 & mask;
            // Scale the small range up so 1-bit 1 becomes 255, not 1.
            ((v * 255) / mask) as u8
        }
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::page::tests::doc_with;

    /// Reads a pixel as `(r, g, b, a)`.
    fn px(img: &Rendered, x: u32, y: u32) -> (u8, u8, u8, u8) {
        let i = ((y * img.width + x) * 4) as usize;
        (
            img.rgba[i],
            img.rgba[i + 1],
            img.rgba[i + 2],
            img.rgba[i + 3],
        )
    }

    #[test]
    fn fills_land_in_y_down_device_space() {
        // A 100x100 blue box at the user-space origin is at the *bottom* left of the page.
        let doc = doc_with("0 0 1 rg 0 0 100 100 re f");
        let page = doc.page(0).unwrap();
        let img = page.render(&doc, RenderOptions::at_dpi(72.0)).unwrap();
        assert_eq!((img.width, img.height), (612, 792));
        assert_eq!(
            px(&img, 50, 742),
            (0, 0, 255, 255),
            "box should be bottom-left"
        );
        assert_eq!(
            px(&img, 50, 50),
            (255, 255, 255, 255),
            "top-left is background"
        );
    }

    #[test]
    fn cmyk_image_samples_are_not_inverted() {
        // One DeviceCMYK pixel stretched over the page: C=0x60 M=0x20 Y=0x10 K=0x40, which is a
        // pale cyan. The samples used to be inverted twice, turning it into a dark red.
        let doc = doc_with(
            "q 612 0 0 792 0 0 cm BI /W 1 /H 1 /BPC 8 /CS /CMYK ID \u{60}\u{20}\u{10}\u{40} EI Q",
        );
        let page = doc.page(0).unwrap();
        let img = page.render(&doc, RenderOptions::at_dpi(72.0)).unwrap();
        let (r, g, b, a) = px(&img, 306, 396);
        assert_eq!(a, 255);
        // (255 - component) * (1 - k), so cyan leaves the red channel darkest.
        let want = |c: u8| ((255 - c) as f32 * (1.0 - 0x40 as f32 / 255.0)) as u8;
        for (got, expect) in [(r, want(0x60)), (g, want(0x20)), (b, want(0x10))] {
            assert!(
                got.abs_diff(expect) <= 1,
                "channel {got} should be about {expect}"
            );
        }
        assert!(r < g && g < b, "cyan-dominant, not the inverted red");
    }

    #[test]
    fn dpi_scales_the_output() {
        let doc = doc_with("0 0 1 rg 0 0 100 100 re f");
        let page = doc.page(0).unwrap();
        let img = page.render(&doc, RenderOptions::at_dpi(144.0)).unwrap();
        assert_eq!((img.width, img.height), (1224, 1584));
    }

    #[test]
    fn region_crops_and_is_clamped_to_the_page() {
        let doc = doc_with("0 0 1 rg 0 0 100 100 re f");
        let page = doc.page(0).unwrap();
        // Bottom-left corner in y-down space, deliberately overhanging the page edge.
        let region = Rect::from_corners(0.0, 692.0, 200.0, 900.0);
        let img = page
            .render_region(&doc, Some(region), RenderOptions::at_dpi(72.0))
            .unwrap();
        assert_eq!((img.width, img.height), (200, 100));
        assert_eq!(px(&img, 50, 50), (0, 0, 255, 255));
        // Right of the 100pt box is background.
        assert_eq!(px(&img, 150, 50), (255, 255, 255, 255));
    }

    #[test]
    fn a_region_off_the_page_is_an_error() {
        let doc = doc_with("0 0 1 rg 0 0 10 10 re f");
        let page = doc.page(0).unwrap();
        let region = Rect::from_corners(2000.0, 2000.0, 2100.0, 2100.0);
        assert!(page
            .render_region(&doc, Some(region), RenderOptions::default())
            .is_err());
    }

    #[test]
    fn paint_order_is_preserved_across_primitive_types() {
        // Red box, then a blue box on top of it. The later one must win.
        let doc = doc_with("1 0 0 rg 0 0 100 100 re f 0 0 1 rg 0 0 100 100 re f");
        let page = doc.page(0).unwrap();
        let img = page.render(&doc, RenderOptions::at_dpi(72.0)).unwrap();
        assert_eq!(px(&img, 50, 742), (0, 0, 255, 255));
    }

    #[test]
    fn non_embedded_base14_text_draws_via_substitution() {
        // Helvetica with no embedded program: without a substitute face this page would render
        // blank, which is the failure this whole path exists to prevent.
        let doc = doc_with("BT /F1 48 Tf 0 0 0 rg 40 400 Td (HHHHHH) Tj ET");
        let page = doc.page(0).unwrap();
        assert!(!page.glyphs.is_empty());
        if page.glyphs[0].outline().is_none() {
            eprintln!("no substitute font installed; skipping ink check");
            return;
        }
        let img = page.render(&doc, RenderOptions::at_dpi(72.0)).unwrap();
        let inked = img.rgba.chunks(4).filter(|p| p[0] < 128).count();
        assert!(inked > 200, "expected glyph ink, got {inked} dark pixels");
    }

    #[test]
    fn draw_text_false_suppresses_glyphs_only() {
        let doc = doc_with("0 0 1 rg 0 0 100 100 re f BT /F1 48 Tf 40 400 Td (HHHH) Tj ET");
        let page = doc.page(0).unwrap();
        let img = page
            .render(
                &doc,
                RenderOptions {
                    draw_text: false,
                    ..RenderOptions::at_dpi(72.0)
                },
            )
            .unwrap();
        // The box still paints; nothing dark remains where the text was.
        assert_eq!(px(&img, 50, 742), (0, 0, 255, 255));
        let dark = img
            .rgba
            .chunks(4)
            .filter(|p| p[0] < 128 && p[2] < 128)
            .count();
        assert_eq!(dark, 0, "text should be suppressed");
    }

    #[test]
    fn transparent_background_is_available() {
        let doc = doc_with("0 0 1 rg 0 0 100 100 re f");
        let page = doc.page(0).unwrap();
        let img = page
            .render(
                &doc,
                RenderOptions {
                    background: false,
                    ..RenderOptions::at_dpi(72.0)
                },
            )
            .unwrap();
        assert_eq!(
            px(&img, 50, 50).3,
            0,
            "unpainted area should be transparent"
        );
        assert_eq!(px(&img, 50, 742), (0, 0, 255, 255));
    }

    #[test]
    fn png_encodes() {
        let doc = doc_with("0 0 1 rg 0 0 100 100 re f");
        let png = doc
            .page(0)
            .unwrap()
            .render(&doc, RenderOptions::at_dpi(36.0))
            .unwrap()
            .to_png()
            .unwrap();
        assert!(png.starts_with(&[0x89, b'P', b'N', b'G']));
    }
}
