//! Page primitives: interpreting content streams into glyphs, paths and images.
//!
//! The interpreter walks a page's operators maintaining the graphics and text state the PDF
//! specification defines, and emits flat primitives in **user space** — PDF's y-up coordinate
//! system with the origin at the media box's lower-left corner. [`Page::page_matrix`] converts
//! to the y-down device space a rasteriser or a layout consumer wants, applying `/Rotate` and
//! the crop box in one step.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::content::ContentParser;
use crate::document::{page_content, Document, PageNode};
use crate::error::{Error, Result};
use crate::font::glyph::PathCmd;
use crate::font::{CodeItem, Font, FontFlags};
use crate::geom::{Matrix, Point, Rect};
use crate::object::{Dict, ObjRef, Object};

/// Nesting limit for form XObjects and Type3 glyph procedures.
const MAX_DEPTH: usize = 12;
/// A horizontal gap this many ems wide, on an unchanged baseline, reads as a word break.
const SPACE_GAP_EM: f32 = 0.12;
/// Baselines within this fraction of an em count as the same line for gap detection.
const BASELINE_EPSILON_EM: f32 = 0.10;

/// One glyph placed on the page.
#[derive(Debug, Clone)]
pub struct Glyph {
    /// The text this glyph contributes. Usually one character; ligatures expand to several.
    pub text: String,
    /// The baseline origin, in user space.
    pub origin: Point,
    /// The glyph's bounding box in user space: tight around the outline when the font program
    /// is available, and a nominal em box derived from the advance when it is not.
    pub bbox: Rect,
    /// The advance from this glyph's origin to the next, in user space units.
    pub advance: f32,
    /// The effective on-page font size: `/Tf` size scaled by the text and current matrices.
    pub font_size: f32,
    /// The resource name the font was reached by (`/F1`).
    pub font_name: String,
    /// The font's `/BaseFont`.
    pub base_font: String,
    /// Typeface properties of the font this glyph came from.
    pub flags: FontFlags,
    /// Baseline rotation in radians, counter-clockwise from the positive x-axis.
    pub rotation: f32,
    /// True when this glyph was synthesized from a positioning gap rather than read from a
    /// string, matching the word breaks pdfium reports.
    pub is_generated_space: bool,
    /// The text rendering mode in force (`/Tr`). Mode 3 and 7 are invisible.
    pub render_mode: i32,
    /// Fill colour as linear RGB in 0..=1.
    pub color: [f32; 3],
    /// Constant fill alpha from the graphics state (`/ca`), in 0..=1.
    pub alpha: f32,
    /// Paint order across every primitive on the page. Sorting glyphs, paths and images by
    /// this reproduces the order the content stream drew them in, which is what a renderer
    /// needs and what the separate per-type vectors would otherwise lose.
    pub order: u32,

    /// The font and code this glyph came from, kept so [`Glyph::outline`] can resolve the
    /// shape on demand. Storing the outline itself would mean tens of thousands of copies of
    /// paths the font already caches once each.
    font: Option<Arc<Font>>,
    code: CodeItem,
    /// Maps glyph-space outline coordinates to user space.
    glyph_matrix: Matrix,
}

impl Glyph {
    /// Whether the glyph paints anything. Invisible text is what scanned pages put under their
    /// page image, so extraction keeps it and rendering skips it.
    pub fn is_visible(&self) -> bool {
        self.render_mode != 3 && self.render_mode != 7
    }

    /// The glyph's outline in glyph space, resolved through the font's cache.
    ///
    /// `None` for a generated space, a Type3 glyph (whose marks the interpreter already emitted
    /// as paths) or a font whose program is absent or uninterpretable.
    pub fn outline(&self) -> Option<Arc<crate::font::glyph::Outline>> {
        if self.is_generated_space {
            return None;
        }
        self.font.as_ref()?.outline(&self.code)
    }

    /// Maps this glyph's outline coordinates into user space.
    pub fn outline_matrix(&self) -> Matrix {
        self.glyph_matrix
    }
}

/// A filled and/or stroked path.
#[derive(Debug, Clone)]
pub struct PathItem {
    /// Path commands in user space.
    pub cmds: Vec<PathCmd>,
    pub bbox: Rect,
    pub fill: Option<[f32; 3]>,
    pub stroke: Option<[f32; 3]>,
    /// Stroke width in user space units.
    pub line_width: f32,
    /// True when the fill uses the even-odd rule rather than nonzero winding.
    pub even_odd: bool,
    /// Constant alphas from the graphics state (`/ca` and `/CA`), in 0..=1.
    pub fill_alpha: f32,
    pub stroke_alpha: f32,
    /// Paint order; see [`Glyph::order`].
    pub order: u32,
}

/// The pixel payload of an image, as far as the byte-level filters could take it.
#[derive(Debug, Clone)]
pub enum ImageData {
    /// Fully decoded samples, row-major, `bits_per_component` per component.
    Raw {
        bits_per_component: u8,
        components: u8,
        data: Vec<u8>,
    },
    /// A codec payload the byte filters do not decode (`DCTDecode`, `JPXDecode`, the fax and
    /// JBIG2 filters); `data` is that codec's own bitstream.
    Encoded { filter: String, data: Vec<u8> },
}

/// An image drawn onto the page.
#[derive(Debug, Clone)]
pub struct ImageItem {
    /// The XObject resource name, or `"inline"` for a `BI ... EI` image.
    pub name: String,
    /// Maps the image's unit square onto the page, in user space. The image's first row is at
    /// the top of the unit square, so this matrix already includes the y flip.
    pub ctm: Matrix,
    pub bbox: Rect,
    pub width: u32,
    pub height: u32,
    /// True for a stencil mask (`/ImageMask`), which paints the fill colour through 1-bit data.
    pub is_mask: bool,
    /// Bits per component and component count, known without touching the pixels.
    pub bits_per_component: u8,
    pub components: u8,
    /// The colour a stencil mask paints through, and the constant alpha in force.
    pub fill: [f32; 3],
    pub alpha: f32,
    /// Paint order; see [`Glyph::order`].
    pub order: u32,
    source: ImageSource,
}

/// Where an image's pixels come from.
///
/// Image XObjects are referenced rather than carried: a page of photographs holds tens of
/// megabytes of samples, and a layout pass that only needs each image's box should not pay for
/// them. Inline images are already bounded by the content stream, so they are kept as decoded.
#[derive(Debug, Clone)]
enum ImageSource {
    Object(ObjRef),
    Inline(Arc<ImageData>),
}

impl ImageItem {
    /// Decodes this image's pixels, reading them from the document if they were not retained.
    pub fn decode(&self, doc: &Document) -> Result<ImageData> {
        match &self.source {
            ImageSource::Inline(data) => Ok((**data).clone()),
            ImageSource::Object(r) => {
                let obj = doc.resolve(&Object::Ref(*r));
                let stream = obj.as_stream().ok_or_else(|| {
                    Error::Parse(format!("image object {} is not a stream", r.num))
                })?;
                let decoded = doc.file.decode_stream(stream)?;
                Ok(match decoded.image_filter {
                    Some(filter) => ImageData::Encoded {
                        filter,
                        data: decoded.data,
                    },
                    None => ImageData::Raw {
                        bits_per_component: self.bits_per_component,
                        components: self.components,
                        data: decoded.data,
                    },
                })
            }
        }
    }
}

/// A page's primitives, with the geometry needed to place them.
pub struct Page {
    pub index: usize,
    pub media_box: Rect,
    pub crop_box: Rect,
    /// Clockwise display rotation in degrees: 0, 90, 180 or 270.
    pub rotation: i32,
    pub glyphs: Vec<Glyph>,
    pub paths: Vec<PathItem>,
    pub images: Vec<ImageItem>,
}

impl Page {
    pub(crate) fn build(doc: &Document, node: &PageNode, index: usize) -> Result<Page> {
        let content = page_content(doc, &node.dict);
        let mut interp = Interpreter::new(doc);
        interp.run(&content, &node.resources, State::new(node.crop_box), 0);

        Ok(Page {
            index,
            media_box: node.media_box,
            crop_box: node.crop_box,
            rotation: node.rotation,
            glyphs: interp.glyphs,
            paths: interp.paths,
            images: interp.images,
        })
    }

    /// The page's displayed width in points, after rotation.
    pub fn width(&self) -> f32 {
        if self.rotation % 180 == 0 {
            self.crop_box.width()
        } else {
            self.crop_box.height()
        }
    }

    /// The page's displayed height in points, after rotation.
    pub fn height(&self) -> f32 {
        if self.rotation % 180 == 0 {
            self.crop_box.height()
        } else {
            self.crop_box.width()
        }
    }

    /// Maps user space to y-down device space at `scale` pixels per point, with the origin at
    /// the top-left of the displayed page and `/Rotate` applied.
    pub fn page_matrix(&self, scale: f32) -> Matrix {
        // Shift the crop box to the origin, flip y, then rotate about the resulting page.
        let to_origin = Matrix::translate(-self.crop_box.x0, -self.crop_box.y0);
        let w = self.crop_box.width();
        let h = self.crop_box.height();
        let flip = Matrix::new(1.0, 0.0, 0.0, -1.0, 0.0, h);
        let rotate = match self.rotation.rem_euclid(360) {
            90 => Matrix::new(0.0, 1.0, -1.0, 0.0, h, 0.0),
            180 => Matrix::new(-1.0, 0.0, 0.0, -1.0, w, h),
            270 => Matrix::new(0.0, -1.0, 1.0, 0.0, 0.0, w),
            _ => Matrix::IDENTITY,
        };
        to_origin
            .concat(&flip)
            .concat(&rotate)
            .concat(&Matrix::scale(scale, scale))
    }

    /// The page's text in content-stream order, with soft hyphens removed.
    ///
    /// Content order is the order operators appear, which is not always reading order; callers
    /// that need reading order sort [`Page::glyphs`] by geometry themselves.
    pub fn text(&self) -> String {
        let mut out = String::new();
        let mut prev: Option<&Glyph> = None;
        for g in &self.glyphs {
            // A content stream carries no line breaks: a new line is just a glyph placed at a
            // different baseline. Concatenating blindly fuses the last word of one line onto
            // the first of the next — "of" + "the" becomes "ofthe" — which silently destroys
            // the commonest words in the document.
            if let Some(p) = prev {
                if starts_new_line(p, g) {
                    out.push('\n');
                }
            }
            out.push_str(&g.text);
            if !g.text.is_empty() {
                prev = Some(g);
            }
        }
        out
    }
}

/// Whether `g` begins a new line relative to the glyph before it.
///
/// Judged geometrically rather than from the operators, because `Td`, `TD`, `T*`, `Tm` and a
/// fresh `BT` can all start a line, and plenty of producers use `Tm` for every single line.
fn starts_new_line(prev: &Glyph, g: &Glyph) -> bool {
    let size = prev.font_size.max(g.font_size);
    if size <= 0.0 {
        return false;
    }
    // Rotated runs are compared along their own baseline direction, so a rotated column does
    // not read as one break per glyph.
    if (prev.rotation - g.rotation).abs() > 0.01 {
        return true;
    }
    let (dx, dy) = (g.origin.x - prev.origin.x, g.origin.y - prev.origin.y);
    let (cos, sin) = (prev.rotation.cos(), prev.rotation.sin());
    let along = dx * cos + dy * sin;
    let across = -dx * sin + dy * cos;

    // A baseline shift of a third of an em is a new line; so is any backward jump larger than
    // one em, which is how a wrapped line returns to the left margin.
    across.abs() > size * 0.33 || along < -size
}

// ---- interpreter ----------------------------------------------------------------------------

/// The graphics state, saved and restored by `q`/`Q`.
#[derive(Clone)]
struct State {
    ctm: Matrix,
    clip: Rect,
    fill: [f32; 3],
    stroke: [f32; 3],
    line_width: f32,
    /// Number of components in the current fill colour space, to interpret `sc`/`scn`.
    fill_components: usize,
    stroke_components: usize,
    /// Constant alphas from `/ca` and `/CA`.
    fill_alpha: f32,
    stroke_alpha: f32,
    // Text state persists across BT/ET, so it lives here rather than in the text object.
    font: Option<Arc<Font>>,
    font_name: String,
    font_size: f32,
    char_spacing: f32,
    word_spacing: f32,
    /// `/Tz` as a plain factor, not a percentage.
    horizontal_scale: f32,
    leading: f32,
    rise: f32,
    render_mode: i32,
}

impl State {
    fn new(clip: Rect) -> State {
        State {
            ctm: Matrix::IDENTITY,
            clip,
            fill: [0.0; 3],
            stroke: [0.0; 3],
            line_width: 1.0,
            fill_components: 1,
            stroke_components: 1,
            fill_alpha: 1.0,
            stroke_alpha: 1.0,
            font: None,
            font_name: String::new(),
            font_size: 0.0,
            char_spacing: 0.0,
            word_spacing: 0.0,
            horizontal_scale: 1.0,
            leading: 0.0,
            rise: 0.0,
            render_mode: 0,
        }
    }
}

/// Where the previous glyph ended, for synthesizing word breaks.
#[derive(Clone, Copy)]
struct PenTrail {
    end: Point,
    baseline_y: f32,
    size: f32,
}

struct Interpreter<'a> {
    doc: &'a Document,
    glyphs: Vec<Glyph>,
    paths: Vec<PathItem>,
    images: Vec<ImageItem>,
    /// Fonts keyed by object number, so a font shared by many pages parses once per page build.
    font_cache: HashMap<u32, Arc<Font>>,
    /// Form XObjects currently being executed, to stop a self-referential form.
    active_forms: HashSet<u32>,
    trail: Option<PenTrail>,
    /// Monotonic paint-order counter, shared by every primitive type.
    order: u32,
}

/// The state of the current text object, reset by `BT`.
struct TextState {
    matrix: Matrix,
    line_matrix: Matrix,
}

impl<'a> Interpreter<'a> {
    fn new(doc: &'a Document) -> Self {
        Interpreter {
            doc,
            glyphs: Vec::new(),
            paths: Vec::new(),
            images: Vec::new(),
            font_cache: HashMap::new(),
            active_forms: HashSet::new(),
            trail: None,
            order: 0,
        }
    }

    fn run(&mut self, content: &[u8], resources: &Dict, initial: State, depth: usize) {
        if depth > MAX_DEPTH {
            return;
        }
        let mut stack: Vec<State> = Vec::new();
        let mut gs = initial;
        let mut text = TextState {
            matrix: Matrix::IDENTITY,
            line_matrix: Matrix::IDENTITY,
        };
        // The path under construction, in user space, plus the point a `h` closes back to.
        let mut path: Vec<PathCmd> = Vec::new();
        let mut subpath_start = Point::default();
        let mut current = Point::default();
        let mut pending_clip: Option<bool> = None;

        for op in ContentParser::new(content) {
            let n = |i: usize| op.operands.get(i).and_then(|o| o.as_f32());
            match op.operator.as_str() {
                // ---- graphics state ----
                "q" => stack.push(gs.clone()),
                "Q" => {
                    if let Some(prev) = stack.pop() {
                        gs = prev;
                    }
                }
                "cm" => {
                    if let (Some(a), Some(b), Some(c), Some(d), Some(e), Some(f)) =
                        (n(0), n(1), n(2), n(3), n(4), n(5))
                    {
                        gs.ctm = Matrix::new(a, b, c, d, e, f).concat(&gs.ctm);
                    }
                }
                "w" => {
                    if let Some(v) = n(0) {
                        gs.line_width = v;
                    }
                }
                "gs" => {
                    if let Some(name) = op.operands.first().and_then(|o| o.as_name()) {
                        self.apply_ext_gstate(resources, name, &mut gs);
                    }
                }

                // ---- colour ----
                "g" | "G" => self.set_color(&mut gs, op.operator == "g", &op.operands, 1),
                "rg" | "RG" => self.set_color(&mut gs, op.operator == "rg", &op.operands, 3),
                "k" | "K" => self.set_color(&mut gs, op.operator == "k", &op.operands, 4),
                "cs" | "CS" => {
                    let comps = self.colorspace_components(resources, &op.operands);
                    if op.operator == "cs" {
                        gs.fill_components = comps;
                        gs.fill = [0.0; 3];
                    } else {
                        gs.stroke_components = comps;
                        gs.stroke = [0.0; 3];
                    }
                }
                "sc" | "scn" | "SC" | "SCN" => {
                    let is_fill = op.operator.starts_with('s');
                    // A pattern operand carries a name and no usable colour; leave the previous
                    // one in place rather than painting black.
                    let numeric = op.operands.iter().filter_map(|o| o.as_f32()).count();
                    if numeric > 0 {
                        // The space declared by `cs`/`CS` says how to read the operands. A
                        // producer that disagrees with its own declaration is trusted on the
                        // operands it actually wrote, since that is what it meant to paint.
                        let declared = if is_fill {
                            gs.fill_components
                        } else {
                            gs.stroke_components
                        };
                        let components = if (1..=numeric).contains(&declared) {
                            declared
                        } else {
                            numeric
                        };
                        self.set_color(&mut gs, is_fill, &op.operands, components);
                    }
                }

                // ---- path construction ----
                "m" => {
                    if let (Some(x), Some(y)) = (n(0), n(1)) {
                        current = gs.ctm.apply(Point::new(x, y));
                        subpath_start = current;
                        path.push(PathCmd::MoveTo(current));
                    }
                }
                "l" => {
                    if let (Some(x), Some(y)) = (n(0), n(1)) {
                        current = gs.ctm.apply(Point::new(x, y));
                        path.push(PathCmd::LineTo(current));
                    }
                }
                "c" => {
                    if let (Some(a), Some(b), Some(c), Some(d), Some(e), Some(f)) =
                        (n(0), n(1), n(2), n(3), n(4), n(5))
                    {
                        let c1 = gs.ctm.apply(Point::new(a, b));
                        let c2 = gs.ctm.apply(Point::new(c, d));
                        current = gs.ctm.apply(Point::new(e, f));
                        path.push(PathCmd::CurveTo(c1, c2, current));
                    }
                }
                "v" => {
                    // The first control point is the current point.
                    if let (Some(c), Some(d), Some(e), Some(f)) = (n(0), n(1), n(2), n(3)) {
                        let c2 = gs.ctm.apply(Point::new(c, d));
                        let end = gs.ctm.apply(Point::new(e, f));
                        path.push(PathCmd::CurveTo(current, c2, end));
                        current = end;
                    }
                }
                "y" => {
                    // The second control point is the endpoint.
                    if let (Some(a), Some(b), Some(e), Some(f)) = (n(0), n(1), n(2), n(3)) {
                        let c1 = gs.ctm.apply(Point::new(a, b));
                        let end = gs.ctm.apply(Point::new(e, f));
                        path.push(PathCmd::CurveTo(c1, end, end));
                        current = end;
                    }
                }
                "h" => {
                    path.push(PathCmd::Close);
                    current = subpath_start;
                }
                "re" => {
                    if let (Some(x), Some(y), Some(w), Some(h)) = (n(0), n(1), n(2), n(3)) {
                        let p0 = gs.ctm.apply(Point::new(x, y));
                        let p1 = gs.ctm.apply(Point::new(x + w, y));
                        let p2 = gs.ctm.apply(Point::new(x + w, y + h));
                        let p3 = gs.ctm.apply(Point::new(x, y + h));
                        path.extend([
                            PathCmd::MoveTo(p0),
                            PathCmd::LineTo(p1),
                            PathCmd::LineTo(p2),
                            PathCmd::LineTo(p3),
                            PathCmd::Close,
                        ]);
                        subpath_start = p0;
                        current = p0;
                    }
                }

                // ---- path painting ----
                "n" | "f" | "F" | "f*" | "S" | "s" | "B" | "B*" | "b" | "b*" => {
                    let o = op.operator.as_str();
                    if matches!(o, "s" | "b" | "b*") {
                        path.push(PathCmd::Close);
                    }
                    let even_odd = o.ends_with('*');
                    let fills = matches!(o, "f" | "F" | "f*" | "B" | "B*" | "b" | "b*");
                    let strokes = matches!(o, "S" | "s" | "B" | "B*" | "b" | "b*");
                    // `W` names the path just painted, so the clip region must be measured
                    // before the path is moved into the emitted item.
                    let region = pending_clip.take().map(|_| bounds(&path));
                    if !path.is_empty() && (fills || strokes) {
                        let bbox = bounds(&path);
                        let order = self.next_order();
                        self.paths.push(PathItem {
                            cmds: std::mem::take(&mut path),
                            bbox,
                            fill: fills.then_some(gs.fill),
                            stroke: strokes.then_some(gs.stroke),
                            // Line width is a user-space quantity scaled by the CTM.
                            line_width: gs.line_width * gs.ctm.x_scale().max(gs.ctm.y_scale()),
                            even_odd,
                            fill_alpha: gs.fill_alpha,
                            stroke_alpha: gs.stroke_alpha,
                            order,
                        });
                    }
                    // `W` takes effect only once the path is painted or discarded. The clip is
                    // tracked as a bounding rectangle, which over-covers a non-rectangular
                    // region — callers use it to reject content, never to include it.
                    if let Some(region) = region {
                        if !region.is_empty() {
                            gs.clip = gs.clip.intersect(&region);
                        }
                    }
                    path.clear();
                }
                "W" => pending_clip = Some(false),
                "W*" => pending_clip = Some(true),

                // ---- text objects ----
                "BT" => {
                    text.matrix = Matrix::IDENTITY;
                    text.line_matrix = Matrix::IDENTITY;
                    self.trail = None;
                }
                "ET" => self.trail = None,
                "Tc" => gs.char_spacing = n(0).unwrap_or(0.0),
                "Tw" => gs.word_spacing = n(0).unwrap_or(0.0),
                "Tz" => gs.horizontal_scale = n(0).unwrap_or(100.0) / 100.0,
                "TL" => gs.leading = n(0).unwrap_or(0.0),
                "Ts" => gs.rise = n(0).unwrap_or(0.0),
                "Tr" => {
                    gs.render_mode =
                        op.operands.first().and_then(|o| o.as_int()).unwrap_or(0) as i32
                }
                "Tf" => {
                    gs.font_size = n(1).unwrap_or(0.0);
                    if let Some(name) = op.operands.first().and_then(|o| o.as_name()) {
                        gs.font_name = name.to_string();
                        gs.font = self.font_for(resources, name);
                    }
                }
                "Td" => {
                    if let (Some(tx), Some(ty)) = (n(0), n(1)) {
                        text.line_matrix = Matrix::translate(tx, ty).concat(&text.line_matrix);
                        text.matrix = text.line_matrix;
                    }
                }
                "TD" => {
                    if let (Some(tx), Some(ty)) = (n(0), n(1)) {
                        gs.leading = -ty;
                        text.line_matrix = Matrix::translate(tx, ty).concat(&text.line_matrix);
                        text.matrix = text.line_matrix;
                    }
                }
                "Tm" => {
                    if let (Some(a), Some(b), Some(c), Some(d), Some(e), Some(f)) =
                        (n(0), n(1), n(2), n(3), n(4), n(5))
                    {
                        text.line_matrix = Matrix::new(a, b, c, d, e, f);
                        text.matrix = text.line_matrix;
                    }
                }
                "T*" => next_line(&mut text, gs.leading),
                "Tj" => {
                    if let Some(Object::String(s)) = op.operands.first() {
                        let s = s.clone();
                        self.show(&s, &gs, &mut text, resources, depth);
                    }
                }
                "'" => {
                    next_line(&mut text, gs.leading);
                    if let Some(Object::String(s)) = op.operands.first() {
                        let s = s.clone();
                        self.show(&s, &gs, &mut text, resources, depth);
                    }
                }
                "\"" => {
                    // `aw ac string "`: sets word and char spacing, then shows on a new line.
                    gs.word_spacing = n(0).unwrap_or(gs.word_spacing);
                    gs.char_spacing = n(1).unwrap_or(gs.char_spacing);
                    next_line(&mut text, gs.leading);
                    if let Some(Object::String(s)) = op.operands.get(2) {
                        let s = s.clone();
                        self.show(&s, &gs, &mut text, resources, depth);
                    }
                }
                "TJ" => {
                    let Some(Object::Array(items)) = op.operands.first().cloned() else {
                        continue;
                    };
                    for item in items {
                        match item {
                            Object::String(s) => self.show(&s, &gs, &mut text, resources, depth),
                            other => {
                                if let Some(adj) = other.as_f32() {
                                    // A positive number moves left, hence the negation.
                                    let tx = -adj / 1000.0 * gs.font_size * gs.horizontal_scale;
                                    text.matrix = Matrix::translate(tx, 0.0).concat(&text.matrix);
                                }
                            }
                        }
                    }
                }

                // ---- XObjects and inline images ----
                "Do" => {
                    if let Some(name) = op.operands.first().and_then(|o| o.as_name()) {
                        let name = name.to_string();
                        self.do_xobject(&name, resources, &gs, depth);
                    }
                }
                "BI" => {
                    if let (Some(dict), Some(data)) = (
                        op.operands.first().and_then(|o| o.as_dict()),
                        op.inline_data.as_ref(),
                    ) {
                        let dict = dict.clone();
                        let data = data.clone();
                        self.record_inline_image(&dict, &data, &gs, resources);
                    }
                }
                _ => {}
            }
        }
    }

    // ---- text ---------------------------------------------------------------------------

    /// Shows one string, emitting a glyph per character code and advancing the text matrix.
    fn show(
        &mut self,
        bytes: &[u8],
        gs: &State,
        text: &mut TextState,
        resources: &Dict,
        depth: usize,
    ) {
        let Some(font) = gs.font.clone() else {
            return;
        };
        for item in font.decode(bytes) {
            // The text rendering matrix places glyph space on the page:
            // [size*Th, 0, 0, size, 0, rise] x Tm x CTM
            let param = Matrix::new(
                gs.font_size * gs.horizontal_scale,
                0.0,
                0.0,
                gs.font_size,
                0.0,
                gs.rise,
            );
            let trm = param.concat(&text.matrix).concat(&gs.ctm);

            let advance_text = (item.width / 1000.0 * gs.font_size
                + gs.char_spacing
                + if item.is_word_space {
                    gs.word_spacing
                } else {
                    0.0
                })
                * gs.horizontal_scale;

            // The advance is a text-space vector, so it reaches user space through Tm and the
            // CTM — but not through `param`, which would apply the font size a second time.
            let text_to_user = text.matrix.concat(&gs.ctm);
            self.emit_glyph(&font, &item, &trm, &text_to_user, gs, advance_text);

            if font.is_type3 {
                self.draw_type3_glyph(&font, &item, &trm, gs, resources, depth);
            }
            text.matrix = Matrix::translate(advance_text, 0.0).concat(&text.matrix);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn emit_glyph(
        &mut self,
        font: &Font,
        item: &CodeItem,
        trm: &Matrix,
        text_to_user: &Matrix,
        gs: &State,
        advance_text: f32,
    ) {
        // Soft hyphens mark optional break points and are not content; producers emit them at
        // line ends where they would otherwise appear as stray hyphens in extracted text.
        let text: String = item
            .unicode
            .as_deref()
            .unwrap_or("")
            .chars()
            .filter(|&c| c != '\u{00AD}')
            .collect();

        let origin = trm.apply(Point::default());
        // The rendering matrix already folds in `/Tf` size, `Tm` and the CTM, so its vertical
        // scale is the size the glyph actually appears at.
        let effective_size = trm.y_scale();
        // Transforming the advance as a vector keeps rotated and skewed text correct, and is
        // exactly the displacement the interpreter applies to the text matrix after this glyph.
        let (advance_dx, advance_dy) = text_to_user.apply_vector(advance_text, 0.0);
        let advance_user = advance_dx.hypot(advance_dy);

        // A gap on an unchanged baseline is a word break the string itself does not contain.
        if !text.is_empty() {
            if let Some(prev) = self.trail {
                let same_line =
                    (origin.y - prev.baseline_y).abs() <= BASELINE_EPSILON_EM * prev.size;
                let gap = origin.x - prev.end.x;
                if same_line && prev.size > 0.0 && gap > SPACE_GAP_EM * prev.size {
                    let already_space = self
                        .glyphs
                        .last()
                        .is_some_and(|g| g.text.chars().all(char::is_whitespace));
                    if !already_space && !text.starts_with(char::is_whitespace) {
                        let order = self.next_order();
                        self.glyphs.push(Glyph {
                            text: " ".into(),
                            origin: prev.end,
                            bbox: Rect::from_corners(
                                prev.end.x,
                                prev.baseline_y,
                                origin.x,
                                prev.baseline_y + prev.size,
                            ),
                            advance: gap,
                            font_size: prev.size,
                            font_name: gs.font_name.clone(),
                            base_font: font.base_font.clone(),
                            flags: font.flags,
                            rotation: trm.rotation(),
                            is_generated_space: true,
                            render_mode: gs.render_mode,
                            color: gs.fill,
                            alpha: gs.fill_alpha,
                            order,
                            font: None,
                            code: item.clone(),
                            glyph_matrix: *trm,
                        });
                    }
                }
            }
        }

        let (bbox, glyph_matrix) = self.glyph_bbox(font, item, trm);
        let order = self.next_order();
        self.glyphs.push(Glyph {
            text,
            origin,
            bbox,
            advance: advance_user,
            font_size: effective_size,
            font_name: gs.font_name.clone(),
            base_font: font.base_font.clone(),
            flags: font.flags,
            rotation: trm.rotation(),
            is_generated_space: false,
            render_mode: gs.render_mode,
            color: gs.fill,
            alpha: gs.fill_alpha,
            order,
            font: gs.font.clone(),
            code: item.clone(),
            glyph_matrix,
        });

        self.trail = Some(PenTrail {
            end: Point::new(origin.x + advance_dx, origin.y + advance_dy),
            baseline_y: origin.y,
            size: effective_size,
        });
    }

    /// A glyph's box: tight around the outline when the font program yields one, otherwise a
    /// nominal em box from the advance, which is what a metrics-only font can support.
    ///
    /// A substituted outline is deliberately not trusted horizontally. Its letters are another
    /// typeface's, so their ink widths are not this font's — and horizontally the document has
    /// already given the exact answer in the advance. Taking the substitute's widths made the
    /// gap between two glyphs depend on which fonts happen to be installed, which is what turned
    /// a letterspaced `REFERENCES` into `REFEREN CES`. Vertically the substitute is kept: cap
    /// height, x-height and descender depth carry across text faces well enough to be worth far
    /// more than one nominal height applied to every glyph alike.
    fn glyph_bbox(&self, font: &Font, item: &CodeItem, trm: &Matrix) -> (Rect, Matrix) {
        // Type3 glyph space is arbitrary, so the font matrix rather than /1000 applies.
        let to_text = if font.is_type3 {
            font.font_matrix
        } else {
            Matrix::scale(0.001, 0.001)
        };
        let m = to_text.concat(trm);
        let advance = item.width.max(1.0);
        if let Some(outline) = font.outline(item) {
            if let Some(b) = outline.bbox() {
                let b = if outline.is_substitute {
                    Rect {
                        x0: 0.0,
                        x1: advance,
                        ..b
                    }
                } else {
                    b
                };
                return (m.apply_rect(&b), m);
            }
        }
        // Nominal box: baseline to ascender, descender below, spanning the advance.
        let nominal = Rect::from_corners(0.0, -200.0, advance, 800.0);
        (m.apply_rect(&nominal), m)
    }

    /// Runs a Type3 glyph's procedure so the marks it makes become page primitives.
    fn draw_type3_glyph(
        &mut self,
        font: &Font,
        item: &CodeItem,
        trm: &Matrix,
        gs: &State,
        resources: &Dict,
        depth: usize,
    ) {
        if depth >= MAX_DEPTH {
            return;
        }
        let Some(proc_obj) = font.type3_proc(self.doc, item.code) else {
            return;
        };
        let Some(stream) = proc_obj.as_stream() else {
            return;
        };
        let Ok(decoded) = self.doc.file.decode_stream(stream) else {
            return;
        };
        let mut inner = gs.clone();
        // Inside the procedure, glyph space maps to the page through the font matrix.
        inner.ctm = font.font_matrix.concat(trm);
        inner.font = None;
        self.run(&decoded.data, resources, inner, depth + 1);
    }

    // ---- resources ----------------------------------------------------------------------

    fn font_for(&mut self, resources: &Dict, name: &str) -> Option<Arc<Font>> {
        let fonts = self.doc.dict_get(resources, "Font")?;
        let entry = fonts.as_dict()?.get(name)?.clone();
        if let Object::Ref(r) = &entry {
            if let Some(hit) = self.font_cache.get(&r.num) {
                return Some(hit.clone());
            }
        }
        let dict = self.doc.resolve(&entry).as_dict()?.clone();
        let font = Arc::new(Font::load(self.doc, &dict));
        if let Object::Ref(r) = &entry {
            self.font_cache.insert(r.num, font.clone());
        }
        Some(font)
    }

    /// `/ExtGState` carries a handful of parameters that change geometry or visibility.
    fn apply_ext_gstate(&self, resources: &Dict, name: &str, gs: &mut State) {
        let Some(ext) = self
            .doc
            .dict_get(resources, "ExtGState")
            .and_then(|o| o.as_dict().and_then(|d| d.get(name).cloned()))
        else {
            return;
        };
        let Some(dict) = self.doc.resolve(&ext).as_dict().cloned() else {
            return;
        };
        if let Some(lw) = self.doc.dict_get(&dict, "LW").and_then(|o| o.as_f32()) {
            gs.line_width = lw;
        }
        // Constant alpha. Watermarks and highlight boxes are drawn with these, so a consumer
        // that ignores them cannot tell a faint overlay from solid page content.
        if let Some(ca) = self.doc.dict_get(&dict, "ca").and_then(|o| o.as_f32()) {
            gs.fill_alpha = ca.clamp(0.0, 1.0);
        }
        if let Some(ca) = self.doc.dict_get(&dict, "CA").and_then(|o| o.as_f32()) {
            gs.stroke_alpha = ca.clamp(0.0, 1.0);
        }
        // `/Font` in an ExtGState is `[fontRef size]`.
        if let Some(Object::Array(items)) = self.doc.dict_get(&dict, "Font") {
            if let Some(size) = items.get(1).and_then(|o| o.as_f32()) {
                gs.font_size = size;
            }
        }
    }

    /// How many components a named colour space has, so `scn` operands can be read.
    fn colorspace_components(&self, resources: &Dict, operands: &[Object]) -> usize {
        let Some(name) = operands.first().and_then(|o| o.as_name()) else {
            return 1;
        };
        match name {
            "DeviceGray" | "CalGray" | "G" => return 1,
            "DeviceRGB" | "CalRGB" | "Lab" | "RGB" => return 3,
            "DeviceCMYK" | "CMYK" => return 4,
            "Pattern" => return 0,
            _ => {}
        }
        // A named space resolves through the resource dictionary.
        let Some(space) = self
            .doc
            .dict_get(resources, "ColorSpace")
            .and_then(|o| o.as_dict().and_then(|d| d.get(name).cloned()))
            .map(|o| self.doc.resolve(&o))
        else {
            return 1;
        };
        self.components_of(&space, 0)
    }

    fn components_of(&self, space: &Object, depth: usize) -> usize {
        if depth > 4 {
            return 1;
        }
        match space {
            Object::Name(n) => match n.as_str() {
                "DeviceRGB" | "CalRGB" | "Lab" => 3,
                "DeviceCMYK" => 4,
                _ => 1,
            },
            Object::Array(items) => {
                let head = items.first().and_then(|o| o.as_name()).unwrap_or("");
                match head {
                    // Indexed and separation spaces take a single index or tint value.
                    "Indexed" | "I" | "Separation" => 1,
                    "DeviceN" => items
                        .get(1)
                        .map(|o| self.doc.resolve(o))
                        .and_then(|o| o.as_array().map(<[Object]>::len))
                        .unwrap_or(1),
                    "ICCBased" => items
                        .get(1)
                        .map(|o| self.doc.resolve(o))
                        .and_then(|o| o.as_dict().and_then(|d| d.get("N").cloned()))
                        .and_then(|o| o.as_int())
                        .unwrap_or(3)
                        .clamp(1, 4) as usize,
                    "CalRGB" | "Lab" => 3,
                    "CalGray" => 1,
                    "Pattern" => 0,
                    _ => self
                        .components_of(&items.first().cloned().unwrap_or(Object::Null), depth + 1),
                }
            }
            _ => 1,
        }
    }

    fn set_color(&self, gs: &mut State, is_fill: bool, operands: &[Object], components: usize) {
        let v: Vec<f32> = operands.iter().filter_map(|o| o.as_f32()).collect();
        let rgb = match components.min(v.len()) {
            1 => {
                let g = v[0].clamp(0.0, 1.0);
                [g, g, g]
            }
            3 => [
                v[0].clamp(0.0, 1.0),
                v[1].clamp(0.0, 1.0),
                v[2].clamp(0.0, 1.0),
            ],
            4 => {
                // The standard naive CMYK conversion; a colour-managed one needs the ICC profile.
                let k = v[3].clamp(0.0, 1.0);
                [
                    (1.0 - v[0].clamp(0.0, 1.0)) * (1.0 - k),
                    (1.0 - v[1].clamp(0.0, 1.0)) * (1.0 - k),
                    (1.0 - v[2].clamp(0.0, 1.0)) * (1.0 - k),
                ]
            }
            _ => return,
        };
        if is_fill {
            gs.fill = rgb;
            gs.fill_components = components;
        } else {
            gs.stroke = rgb;
            gs.stroke_components = components;
        }
    }

    // ---- XObjects -----------------------------------------------------------------------

    fn do_xobject(&mut self, name: &str, resources: &Dict, gs: &State, depth: usize) {
        let Some(xobjects) = self.doc.dict_get(resources, "XObject") else {
            return;
        };
        let Some(entry) = xobjects.as_dict().and_then(|d| d.get(name).cloned()) else {
            return;
        };
        let obj_num = entry.as_ref_id().map(|r| r.num);
        let resolved = self.doc.resolve(&entry);
        let Some(stream) = resolved.as_stream() else {
            return;
        };
        let subtype = stream.dict.get("Subtype").and_then(|o| o.as_name());

        match subtype {
            Some("Image") => self.record_image(name, stream, entry.as_ref_id(), gs),
            Some("Form") => {
                // A form that draws itself would recurse forever.
                if let Some(num) = obj_num {
                    if !self.active_forms.insert(num) {
                        return;
                    }
                }
                let Ok(decoded) = self.doc.file.decode_stream(stream) else {
                    if let Some(num) = obj_num {
                        self.active_forms.remove(&num);
                    }
                    return;
                };
                let mut inner = gs.clone();
                if let Some(m) = stream
                    .dict
                    .get("Matrix")
                    .map(|o| self.doc.resolve(o))
                    .as_ref()
                    .and_then(|o| o.as_array())
                {
                    let v: Vec<f32> = m.iter().filter_map(|o| o.as_f32()).collect();
                    if v.len() >= 6 {
                        inner.ctm = Matrix::new(v[0], v[1], v[2], v[3], v[4], v[5]).concat(&gs.ctm);
                    }
                }
                if let Some(bbox) = stream
                    .dict
                    .get("BBox")
                    .map(|o| self.doc.resolve(o))
                    .as_ref()
                    .and_then(|o| o.as_array())
                {
                    let v: Vec<f32> = bbox.iter().filter_map(|o| o.as_f32()).collect();
                    if v.len() >= 4 {
                        let r = inner
                            .ctm
                            .apply_rect(&Rect::from_corners(v[0], v[1], v[2], v[3]));
                        if !r.is_empty() {
                            inner.clip = inner.clip.intersect(&r);
                        }
                    }
                }
                // A form without its own /Resources inherits the page's.
                let inner_resources = self
                    .doc
                    .dict_get(&stream.dict, "Resources")
                    .and_then(|o| o.as_dict().cloned())
                    .unwrap_or_else(|| resources.clone());
                let data = decoded.data;
                self.run(&data, &inner_resources, inner, depth + 1);
                if let Some(num) = obj_num {
                    self.active_forms.remove(&num);
                }
            }
            _ => {}
        }
    }

    fn record_image(
        &mut self,
        name: &str,
        stream: &crate::object::Stream,
        obj_ref: Option<ObjRef>,
        gs: &State,
    ) {
        let dict = &stream.dict;
        let width = self
            .doc
            .dict_get(dict, "Width")
            .and_then(|o| o.as_int())
            .unwrap_or(0)
            .max(0) as u32;
        let height = self
            .doc
            .dict_get(dict, "Height")
            .and_then(|o| o.as_int())
            .unwrap_or(0)
            .max(0) as u32;
        if width == 0 || height == 0 {
            return;
        }
        let is_mask = self
            .doc
            .dict_get(dict, "ImageMask")
            .and_then(|o| o.as_bool())
            .unwrap_or(false);
        let bpc = self
            .doc
            .dict_get(dict, "BitsPerComponent")
            .and_then(|o| o.as_int())
            .unwrap_or(if is_mask { 1 } else { 8 })
            .clamp(1, 16) as u8;
        let components = if is_mask {
            1
        } else {
            let space = self
                .doc
                .dict_get(dict, "ColorSpace")
                .unwrap_or(Object::Null);
            self.components_of(&space, 0).max(1) as u8
        };

        // The pixels stay in the file until someone asks for them. When the image is not
        // reachable by reference the samples have to be carried, but a direct image XObject is
        // rare enough that the cost does not matter.
        let source = match obj_ref {
            Some(r) => ImageSource::Object(r),
            None => {
                let Ok(decoded) = self.doc.file.decode_stream(stream) else {
                    return;
                };
                ImageSource::Inline(Arc::new(match decoded.image_filter {
                    Some(filter) => ImageData::Encoded {
                        filter,
                        data: decoded.data,
                    },
                    None => ImageData::Raw {
                        bits_per_component: bpc,
                        components,
                        data: decoded.data,
                    },
                }))
            }
        };
        self.push_image(name, gs, width, height, is_mask, bpc, components, source);
    }

    fn record_inline_image(&mut self, dict: &Dict, raw: &[u8], gs: &State, resources: &Dict) {
        let width = self
            .doc
            .dict_get(dict, "Width")
            .and_then(|o| o.as_int())
            .unwrap_or(0)
            .max(0) as u32;
        let height = self
            .doc
            .dict_get(dict, "Height")
            .and_then(|o| o.as_int())
            .unwrap_or(0)
            .max(0) as u32;
        if width == 0 || height == 0 {
            return;
        }
        let is_mask = self
            .doc
            .dict_get(dict, "ImageMask")
            .and_then(|o| o.as_bool())
            .unwrap_or(false);
        let bpc = self
            .doc
            .dict_get(dict, "BitsPerComponent")
            .and_then(|o| o.as_int())
            .unwrap_or(if is_mask { 1 } else { 8 })
            .clamp(1, 16) as u8;
        // An inline image's colour space may name an entry in the page's /ColorSpace resources.
        let components = if is_mask {
            1
        } else {
            match self.doc.dict_get(dict, "ColorSpace") {
                Some(Object::Name(n)) => {
                    self.colorspace_components(resources, &[Object::Name(n)]) as u8
                }
                Some(other) => self.components_of(&other, 0) as u8,
                None => 1,
            }
            .max(1)
        };

        let resolve = |o: &Object| self.doc.resolve(o);
        let data = match crate::filters::decode(dict, raw, &resolve) {
            Ok(d) => match d.image_filter {
                Some(filter) => ImageData::Encoded {
                    filter,
                    data: d.data,
                },
                None => ImageData::Raw {
                    bits_per_component: bpc,
                    components,
                    data: d.data,
                },
            },
            Err(_) => return,
        };
        let source = ImageSource::Inline(Arc::new(data));
        self.push_image(
            "inline", gs, width, height, is_mask, bpc, components, source,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn push_image(
        &mut self,
        name: &str,
        gs: &State,
        width: u32,
        height: u32,
        is_mask: bool,
        bits_per_component: u8,
        components: u8,
        source: ImageSource,
    ) {
        // Image space has its first row at the top of the unit square, so flip y into the CTM.
        let ctm = Matrix::new(1.0, 0.0, 0.0, -1.0, 0.0, 1.0).concat(&gs.ctm);
        let bbox = gs.ctm.apply_rect(&Rect::from_corners(0.0, 0.0, 1.0, 1.0));
        let order = self.next_order();
        self.images.push(ImageItem {
            name: name.to_string(),
            ctm,
            bbox,
            width,
            height,
            is_mask,
            bits_per_component,
            components,
            fill: gs.fill,
            alpha: gs.fill_alpha,
            order,
            source,
        });
    }

    /// Allocates the next paint-order stamp.
    fn next_order(&mut self) -> u32 {
        let n = self.order;
        self.order = self.order.saturating_add(1);
        n
    }
}

fn next_line(text: &mut TextState, leading: f32) {
    text.line_matrix = Matrix::translate(0.0, -leading).concat(&text.line_matrix);
    text.matrix = text.line_matrix;
}

/// The bounding box of a path, control points included.
fn bounds(cmds: &[PathCmd]) -> Rect {
    let mut out: Option<Rect> = None;
    let mut add = |p: Point| {
        out = Some(match out {
            None => Rect {
                x0: p.x,
                y0: p.y,
                x1: p.x,
                y1: p.y,
            },
            Some(r) => Rect {
                x0: r.x0.min(p.x),
                y0: r.y0.min(p.y),
                x1: r.x1.max(p.x),
                y1: r.y1.max(p.y),
            },
        });
    };
    for cmd in cmds {
        match *cmd {
            PathCmd::MoveTo(p) | PathCmd::LineTo(p) => add(p),
            PathCmd::CurveTo(a, b, c) => {
                add(a);
                add(b);
                add(c);
            }
            PathCmd::Close => {}
        }
    }
    out.unwrap_or(Rect::EMPTY)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::parser::tests::tiny_pdf;

    /// Builds a one-page document whose content stream is `content`, using Helvetica as /F1.
    pub(crate) fn doc_with(content: &str) -> Document {
        let base = String::from_utf8(tiny_pdf()).unwrap();
        let old = "BT /F1 12 Tf 72 720 Td (Hi) Tj ET";
        // The /Length is wrong after substitution, which the endstream scan recovers from — the
        // same path a real producer's off-by-one length takes.
        Document::from_bytes(base.replace(old, content).into_bytes(), None).unwrap()
    }

    #[test]
    fn extracts_text_with_positions() {
        let doc = doc_with("BT /F1 12 Tf 72 720 Td (Hi) Tj ET");
        let page = doc.page(0).unwrap();
        assert_eq!(page.text(), "Hi");
        assert_eq!(page.glyphs.len(), 2);

        let h = &page.glyphs[0];
        assert_eq!(h.text, "H");
        assert_eq!(h.base_font, "Helvetica");
        assert!((h.origin.x - 72.0).abs() < 0.01);
        assert!((h.origin.y - 720.0).abs() < 0.01);
        assert!((h.font_size - 12.0).abs() < 0.01);
        // Helvetica's H advances 722/1000 em.
        assert!((h.advance - 722.0 / 1000.0 * 12.0).abs() < 0.01);
        // The next glyph starts exactly one advance along.
        assert!((page.glyphs[1].origin.x - (72.0 + 8.664)).abs() < 0.01);
    }

    #[test]
    fn page_geometry_matches_media_box() {
        let doc = doc_with("BT /F1 12 Tf 72 720 Td (Hi) Tj ET");
        let page = doc.page(0).unwrap();
        assert_eq!((page.width(), page.height()), (612.0, 792.0));
        // User-space y-up maps to device y-down: the baseline at y=720 lands 72pt from the top.
        let m = page.page_matrix(1.0);
        let p = m.apply(Point::new(72.0, 720.0));
        assert!((p.x - 72.0).abs() < 0.01 && (p.y - 72.0).abs() < 0.01);
    }

    #[test]
    fn tj_offsets_shift_the_pen_and_generate_spaces() {
        // -500 thousandths at 12pt is a 6pt gap, far over the word-break threshold.
        let doc = doc_with("BT /F1 12 Tf 72 720 Td [(A) -500 (B)] TJ ET");
        let page = doc.page(0).unwrap();
        assert_eq!(page.text(), "A B");
        assert!(page.glyphs[1].is_generated_space);

        // A small adjustment is kerning, not a word break.
        let doc = doc_with("BT /F1 12 Tf 72 720 Td [(A) -20 (B)] TJ ET");
        assert_eq!(doc.page(0).unwrap().text(), "AB");
    }

    /// Justified text squeezes its word spaces, and they still have to register as spaces.
    ///
    /// A real line of ICLR body text was found setting them at 0.19 em — under the 0.20 em the
    /// threshold started at, so *every* word gap on that line was missed and the line extracted
    /// as one run-on word. The threshold has to sit below what justification compresses a space
    /// to, and above what kerning ever opens up.
    #[test]
    fn a_compressed_word_space_is_still_a_word_space() {
        let doc = doc_with("BT /F1 12 Tf 72 720 Td [(A) -190 (B)] TJ ET");
        assert_eq!(
            doc.page(0).unwrap().text(),
            "A B",
            "0.19 em is a squeezed space"
        );

        // Well below any space, and comfortably above ordinary kerning: still one word.
        let doc = doc_with("BT /F1 12 Tf 72 720 Td [(A) -80 (B)] TJ ET");
        assert_eq!(doc.page(0).unwrap().text(), "AB", "0.08 em is not a space");
    }

    /// A glyph drawn from a substitute face must report the document's advance, not the
    /// substitute's ink width.
    ///
    /// Helvetica is one of the standard 14 and embeds nothing, so its outlines come from
    /// whatever system face is installed. Letting that face's ink decide the box made the gap
    /// between two glyphs depend on which fonts the machine happened to have, and a letterspaced
    /// heading then picked up a word break that is not in the document (`REFEREN CES`).
    #[test]
    fn a_substituted_glyph_reports_the_documents_advance() {
        let doc = doc_with("BT /F1 12 Tf 72 720 Td (Hi) Tj ET");
        let page = doc.page(0).unwrap();

        for glyph in &page.glyphs {
            let width = glyph.bbox.x1 - glyph.bbox.x0;
            assert!(
                (width - glyph.advance).abs() < 0.01,
                "{:?}: box {width} should span the advance {}",
                glyph.text,
                glyph.advance
            );
        }

        // Consecutive boxes therefore abut exactly, leaving no gap to mistake for a space.
        let gap = page.glyphs[1].bbox.x0 - page.glyphs[0].bbox.x1;
        assert!(
            gap.abs() < 0.01,
            "adjacent letters must not show an ink gap"
        );
    }

    #[test]
    fn text_state_operators_apply() {
        // Tz halves the advance; the second glyph must land half as far along.
        let doc = doc_with("BT /F1 12 Tf 50 Tz 72 720 Td (Hi) Tj ET");
        let page = doc.page(0).unwrap();
        assert!((page.glyphs[1].origin.x - (72.0 + 8.664 / 2.0)).abs() < 0.01);

        // TL + T* moves down one leading.
        let doc = doc_with("BT /F1 12 Tf 14 TL 72 720 Td (A) Tj T* (B) Tj ET");
        let page = doc.page(0).unwrap();
        let b = page.glyphs.iter().find(|g| g.text == "B").unwrap();
        assert!((b.origin.y - 706.0).abs() < 0.01);
    }

    #[test]
    fn a_scaled_text_matrix_does_not_manufacture_spaces() {
        // `Tm` carries its own scale, so the pen advance must travel through Tm as well as the
        // CTM. Scaling by the CTM alone under-counts it, and the shortfall reads as a gap wide
        // enough to be a word break — a space between every single character.
        let doc = doc_with("BT /F1 12 Tf 2 0 0 2 72 720 Tm (Hi) Tj ET");
        let page = doc.page(0).unwrap();
        assert_eq!(page.text(), "Hi");
        assert!(!page.glyphs.iter().any(|g| g.is_generated_space));
        // Helvetica's H advances 722/1000 em, doubled by the text matrix.
        assert!((page.glyphs[1].origin.x - (72.0 + 0.722 * 12.0 * 2.0)).abs() < 0.05);
    }

    #[test]
    fn an_oversized_advance_does_not_swallow_word_breaks() {
        // The mirror of the case above: over-counting the advance pushes the pen past the next
        // glyph, every gap computes as negative, and no word break is ever emitted.
        let doc = doc_with("BT /F1 12 Tf 0.5 0 0 0.5 72 720 Tm (a) Tj 40 0 Td (b) Tj ET");
        let page = doc.page(0).unwrap();
        assert!(
            page.glyphs.iter().any(|g| g.is_generated_space),
            "a 20pt gap at 6pt effective size is a word break: {:?}",
            page.text()
        );
    }

    #[test]
    fn text_breaks_lines_instead_of_fusing_words() {
        // Content streams have no newlines; a new line is a glyph at another baseline. Without
        // a break the last word of one line fuses onto the first of the next.
        let doc = doc_with("BT /F1 12 Tf 72 720 Td (of) Tj 0 -14 Td (the) Tj ET");
        assert_eq!(doc.page(0).unwrap().text(), "of\nthe");
    }

    #[test]
    fn a_wrapped_line_returning_to_the_margin_breaks() {
        // Some producers set an explicit `Tm` per line rather than using `Td`/`T*`.
        let doc =
            doc_with("BT /F1 12 Tf 1 0 0 1 300 720 Tm (end) Tj 1 0 0 1 72 720 Tm (start) Tj ET");
        assert_eq!(doc.page(0).unwrap().text(), "end\nstart");
    }

    #[test]
    fn ordinary_spacing_within_a_line_is_not_a_break() {
        let doc = doc_with("BT /F1 12 Tf 72 720 Td (a) Tj (b) Tj ET");
        assert_eq!(doc.page(0).unwrap().text(), "ab");
    }

    #[test]
    fn invisible_text_is_kept_but_flagged() {
        let doc = doc_with("BT /F1 12 Tf 3 Tr 72 720 Td (Hi) Tj ET");
        let page = doc.page(0).unwrap();
        assert_eq!(page.text(), "Hi");
        assert!(!page.glyphs[0].is_visible());
    }

    #[test]
    fn paths_are_recorded_in_user_space() {
        let doc = doc_with("0 0 1 rg 10 20 100 50 re f");
        let page = doc.page(0).unwrap();
        assert_eq!(page.paths.len(), 1);
        let p = &page.paths[0];
        assert_eq!(p.bbox, Rect::from_corners(10.0, 20.0, 110.0, 70.0));
        assert_eq!(p.fill, Some([0.0, 0.0, 1.0]));
        assert_eq!(p.stroke, None);
    }

    #[test]
    fn cm_transforms_geometry_and_q_restores_it() {
        let doc = doc_with("q 2 0 0 2 5 5 cm 0 0 10 10 re f Q 0 0 10 10 re S");
        let page = doc.page(0).unwrap();
        assert_eq!(page.paths.len(), 2);
        // Scaled by 2 and shifted by 5.
        assert_eq!(page.paths[0].bbox, Rect::from_corners(5.0, 5.0, 25.0, 25.0));
        // After Q the CTM is back to identity.
        assert_eq!(page.paths[1].bbox, Rect::from_corners(0.0, 0.0, 10.0, 10.0));
        assert!(page.paths[1].fill.is_none() && page.paths[1].stroke.is_some());
    }

    #[test]
    fn cmyk_and_gray_colors_convert() {
        let doc = doc_with("0.5 g 0 0 1 1 re f 0 1 1 0 k 0 0 1 1 re f");
        let page = doc.page(0).unwrap();
        assert_eq!(page.paths[0].fill, Some([0.5, 0.5, 0.5]));
        // Cyan=0, magenta=1, yellow=1, black=0 is red.
        assert_eq!(page.paths[1].fill, Some([1.0, 0.0, 0.0]));
    }

    #[test]
    fn inline_images_are_recorded() {
        let doc =
            doc_with("q 100 0 0 50 10 10 cm BI /W 2 /H 2 /BPC 8 /CS /G ID \x01\x02\x03\x04 EI Q");
        let page = doc.page(0).unwrap();
        assert_eq!(page.images.len(), 1);
        let img = &page.images[0];
        assert_eq!((img.width, img.height), (2, 2));
        assert_eq!(img.bbox, Rect::from_corners(10.0, 10.0, 110.0, 60.0));
        assert_eq!((img.components, img.bits_per_component), (1, 8));
        // Pixels are fetched on demand rather than carried on the item.
        match img.decode(&doc).unwrap() {
            ImageData::Raw { data, .. } => assert_eq!(data.len(), 4),
            other => panic!("expected raw samples, got {other:?}"),
        }
    }

    #[test]
    fn malformed_content_does_not_panic() {
        for junk in [
            "BT (unterminated",
            "q q q q 1 0 0 1 cm",
            "BT /Missing 12 Tf (x) Tj ET",
            "[(A) (B)] TJ",
            ") ] >> garbage f",
            "BT /F1 0 Tf 72 720 Td (Hi) Tj ET",
        ] {
            let page = doc_with(junk).page(0).unwrap();
            let _ = page.text();
        }
    }
}

#[cfg(test)]
mod thread_safety {
    /// The crate's premise is that a document is usable from several threads at once, which
    /// pdfium's global state prevents. Assert it at compile time so a stray `Rc` or `Cell`
    /// cannot silently take it away.
    const _: fn() = || {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<crate::Document>();
        assert_send_sync::<crate::Font>();
        assert_send_sync::<crate::Page>();
    };
}

#[cfg(test)]
mod colorspace_tests {
    use super::tests::doc_with;

    #[test]
    fn scn_reads_operands_through_the_declared_space() {
        // An ICCBased 3-component space makes `scn` an RGB triple.
        let doc = doc_with("/DeviceRGB cs 1 0 0 scn 0 0 1 1 re f");
        assert_eq!(doc.page(0).unwrap().paths[0].fill, Some([1.0, 0.0, 0.0]));

        // A Separation space takes one tint, read as gray.
        let doc = doc_with("/DeviceGray cs 0.25 scn 0 0 1 1 re f");
        assert_eq!(doc.page(0).unwrap().paths[0].fill, Some([0.25, 0.25, 0.25]));
    }

    #[test]
    fn pattern_name_operand_leaves_the_color_alone() {
        // `/P0 scn` names a pattern and carries no numeric operands. There is no colour to
        // read, so the current one must survive rather than collapsing to black.
        let doc = doc_with("1 0 0 rg /P0 scn 0 0 1 1 re f");
        assert_eq!(doc.page(0).unwrap().paths[0].fill, Some([1.0, 0.0, 0.0]));
    }

    #[test]
    fn cs_resets_to_the_spaces_initial_color() {
        // Selecting a colour space resets the current colour to that space's initial value,
        // which for the device spaces is black.
        let doc = doc_with("1 0 0 rg /DeviceRGB cs 0 0 1 1 re f");
        assert_eq!(doc.page(0).unwrap().paths[0].fill, Some([0.0, 0.0, 0.0]));
    }
}
