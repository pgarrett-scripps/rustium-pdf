//! Fonts: mapping character codes to advances, Unicode and glyph outlines.
//!
//! Three shapes of font reach this module and they differ in how a string becomes glyphs:
//!
//! * **Simple** (Type1, TrueType, MMType1) — one byte per code, and an encoding table maps each
//!   code to a glyph *name*.
//! * **Type0** (composite) — a CMap splits the string into codes of one to four bytes and maps
//!   each to a CID, which the descendant font turns into a glyph.
//! * **Type3** — glyphs are content streams; the page interpreter draws them, so this module
//!   only supplies the encoding, the font matrix and the procedure streams.

pub mod cmap;
pub mod encoding;
pub mod glyph;
mod metrics;
pub mod system;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::document::Document;
use crate::geom::Matrix;
use crate::object::{Dict, Object};

use cmap::CMap;
use encoding::{base_encoding_name, glyph_name_to_unicode, Encoding};
use glyph::{Outline, OutlineBuilder};

/// Typeface properties, from the font descriptor's `/Flags` plus the corroborating entries.
///
/// A descriptor's flags alone are unreliable — publisher toolchains routinely ship bold faces
/// with the bold bit clear and italic faces with a zero `/ItalicAngle` — so each accessor
/// corroborates the bit with the font's weight, angle or name before answering.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FontFlags(pub u32);

impl FontFlags {
    pub const FIXED_PITCH: u32 = 1 << 0;
    pub const SERIF: u32 = 1 << 1;
    pub const SYMBOLIC: u32 = 1 << 2;
    pub const CURSIVE: u32 = 1 << 3;
    pub const ITALIC: u32 = 1 << 4;
    pub const ALL_CAPS: u32 = 1 << 5;
    pub const SMALL_CAPS: u32 = 1 << 6;
    pub const BOLD: u32 = 1 << 7;
    /// Set when the descriptor's nonsymbolic bit is set or the serif bit is clear; a font is
    /// never both this and [`FontFlags::SERIF`].
    pub const SANS_SERIF: u32 = 1 << 8;

    pub fn has(self, flag: u32) -> bool {
        self.0 & flag != 0
    }

    fn set(&mut self, flag: u32, on: bool) {
        if on {
            self.0 |= flag;
        } else {
            self.0 &= !flag;
        }
    }

    pub fn is_bold(self) -> bool {
        self.has(Self::BOLD)
    }

    pub fn is_italic(self) -> bool {
        self.has(Self::ITALIC)
    }

    pub fn is_serif(self) -> bool {
        self.has(Self::SERIF)
    }

    pub fn is_fixed_pitch(self) -> bool {
        self.has(Self::FIXED_PITCH)
    }

    pub fn is_symbolic(self) -> bool {
        self.has(Self::SYMBOLIC)
    }
}

/// Bit positions in a font descriptor's `/Flags`, per the PDF specification's table.
mod desc_flag {
    pub const FIXED_PITCH: i64 = 1 << 0;
    pub const SERIF: i64 = 1 << 1;
    pub const SYMBOLIC: i64 = 1 << 2;
    pub const SCRIPT: i64 = 1 << 3;
    pub const NONSYMBOLIC: i64 = 1 << 5;
    pub const ITALIC: i64 = 1 << 6;
    pub const ALL_CAP: i64 = 1 << 16;
    pub const SMALL_CAP: i64 = 1 << 17;
    pub const FORCE_BOLD: i64 = 1 << 18;
}

/// Derives typeface properties from a descriptor and the `/BaseFont` name.
fn derive_flags(doc: &Document, descriptor: Option<&Dict>, base_font: &str) -> FontFlags {
    let num = |key: &str| {
        descriptor
            .and_then(|d| doc.dict_get(d, key))
            .and_then(|o| o.as_f32())
    };
    let raw = descriptor
        .and_then(|d| doc.dict_get(d, "Flags"))
        .and_then(|o| o.as_int())
        .unwrap_or(0);

    // The style suffix on a subset name (`ABCDEF+Times-BoldItalic`) is often the only honest
    // record of the face, so it corroborates every heuristic below.
    let name = base_font
        .split('+')
        .next_back()
        .unwrap_or(base_font)
        .to_ascii_lowercase();

    let mut flags = FontFlags::default();

    // A font is symbolic when it says so and does not also claim to be nonsymbolic; producers
    // that set both mean nonsymbolic, which is the more conservative reading.
    let symbolic = raw & desc_flag::SYMBOLIC != 0 && raw & desc_flag::NONSYMBOLIC == 0;
    flags.set(FontFlags::SYMBOLIC, symbolic);

    let serif = raw & desc_flag::SERIF != 0
        || [
            "times", "roman", "georgia", "garamond", "minion", "cambria", "book",
        ]
        .iter()
        .any(|f| name.contains(f));
    flags.set(FontFlags::SERIF, serif);
    flags.set(FontFlags::SANS_SERIF, !serif && !symbolic);

    flags.set(
        FontFlags::FIXED_PITCH,
        raw & desc_flag::FIXED_PITCH != 0 || name.contains("courier") || name.contains("mono"),
    );

    // `/ItalicAngle` is the strongest signal: an upright face has angle zero.
    let italic = raw & desc_flag::ITALIC != 0
        || num("ItalicAngle").is_some_and(|a| a.abs() > 0.5)
        || name.contains("italic")
        || name.contains("oblique");
    flags.set(FontFlags::ITALIC, italic);

    // The descriptor's bold bit alone misses faces that are bold by weight but not by flag,
    // which is common in publisher-typeset PDFs. `/StemV` above ~120 is a bold stem width.
    let bold = raw & desc_flag::FORCE_BOLD != 0
        || num("FontWeight").is_some_and(|w| w >= 600.0)
        || num("StemV").is_some_and(|s| s >= 120.0)
        || name.contains("bold")
        || name.contains("black")
        || name.contains("heavy");
    flags.set(FontFlags::BOLD, bold);

    flags.set(
        FontFlags::CURSIVE,
        raw & desc_flag::SCRIPT != 0 || name.contains("script"),
    );
    flags.set(FontFlags::ALL_CAPS, raw & desc_flag::ALL_CAP != 0);
    flags.set(FontFlags::SMALL_CAPS, raw & desc_flag::SMALL_CAP != 0);
    flags
}

/// One character code decoded from a PDF string.
#[derive(Debug, Clone)]
pub struct CodeItem {
    /// The raw character code, as assembled from one or more bytes.
    pub code: u32,
    /// The glyph selector: the code itself for simple fonts, the CID for composite ones.
    pub cid: u32,
    /// The text this code contributes, when it can be determined.
    pub unicode: Option<String>,
    /// Horizontal advance in glyph space, where 1000 units is one em.
    pub width: f32,
    /// True only for a single-byte code 32, which is what `Tw` word spacing applies to.
    pub is_word_space: bool,
}

/// An embedded font program.
enum Program {
    /// TrueType or OpenType — anything with an sfnt table directory.
    Sfnt(Arc<[u8]>),
    /// A bare CFF program (`/FontFile3` with subtype `Type1C` or `CIDFontType0C`).
    Cff(Arc<[u8]>),
    /// No program, or one in a format whose charstrings are not interpreted (Type1 `/FontFile`).
    None,
}

/// Advance widths, in the form the font type declares them.
enum Widths {
    /// Simple fonts: an array indexed from `first`, with a fallback for codes outside it.
    Simple {
        first: u32,
        widths: Vec<f32>,
        missing: Option<f32>,
    },
    /// CID fonts: sparse per-CID widths over a default.
    Cid {
        default: f32,
        map: HashMap<u32, f32>,
    },
}

/// The parts specific to a composite (Type0) font.
struct Composite {
    /// Maps character codes to CIDs and defines the codespace.
    cmap: CMap,
    /// `/CIDToGIDMap` as a stream: two big-endian bytes of glyph id per CID.
    cid_to_gid: Option<Arc<[u8]>>,
}

/// A loaded font, ready to decode strings and produce outlines.
///
/// Outlines are cached behind a mutex, so a `Font` shared across threads stays correct and only
/// pays for each glyph once.
pub struct Font {
    pub base_font: String,
    /// Typeface properties, corroborated from the descriptor, weight, angle and name.
    pub flags: FontFlags,
    pub is_type3: bool,
    /// Glyph space to text space. `0.001` scale for every font type except Type3, which
    /// declares its own.
    pub font_matrix: Matrix,
    /// Code to glyph name, 256 entries, for simple and Type3 fonts. Empty for Type0.
    names: Vec<String>,
    /// A symbolic font's built-in encoding takes precedence over any standard one.
    symbolic: bool,
    composite: Option<Composite>,
    to_unicode: Option<CMap>,
    widths: Widths,
    program: Program,
    /// Type3 `/CharProcs`.
    char_procs: Dict,
    outline_cache: Mutex<HashMap<u32, Option<Arc<Outline>>>>,
    /// The substitute face, resolved on the first glyph that needs one.
    substitute: std::sync::OnceLock<Option<Arc<[u8]>>>,
}

impl std::fmt::Debug for Font {
    /// Deliberately shallow: the caches and program bytes are noise in a glyph dump, and a
    /// `Glyph` holding an `Arc<Font>` is printed once per character.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Font")
            .field("base_font", &self.base_font)
            .field("flags", &self.flags)
            .field("is_type3", &self.is_type3)
            .finish_non_exhaustive()
    }
}

impl Font {
    /// Builds a font from a `/Font` resource dictionary.
    ///
    /// Malformed entries degrade rather than fail: a font that cannot be understood still
    /// decodes codes and reports advances, because dropping the text entirely is worse than
    /// reporting it with approximate metrics.
    pub fn load(doc: &Document, dict: &Dict) -> Font {
        let subtype = dict.get("Subtype").and_then(|o| o.as_name()).unwrap_or("");
        let base_font = doc
            .dict_get(dict, "BaseFont")
            .as_ref()
            .and_then(|o| o.as_name())
            .unwrap_or("")
            .to_string();

        match subtype {
            "Type0" => Self::load_type0(doc, dict, base_font),
            "Type3" => Self::load_type3(doc, dict, base_font),
            _ => Self::load_simple(doc, dict, base_font),
        }
    }

    // ---- constructors per font type ---------------------------------------------------------

    fn load_simple(doc: &Document, dict: &Dict, base_font: String) -> Font {
        let descriptor = doc
            .dict_get(dict, "FontDescriptor")
            .and_then(|o| o.as_dict().cloned());
        let flags = derive_flags(doc, descriptor.as_ref(), &base_font);
        let symbolic = flags.is_symbolic();

        let names = build_encoding(doc, dict, &base_font, symbolic);
        let widths = simple_widths(doc, dict, descriptor.as_ref());
        let program = load_program(doc, descriptor.as_ref());

        Font {
            base_font,
            flags,
            is_type3: false,
            font_matrix: Matrix::scale(0.001, 0.001),
            names,
            symbolic,
            composite: None,
            to_unicode: load_to_unicode(doc, dict),
            widths,
            program,
            char_procs: Dict::new(),
            outline_cache: Mutex::new(HashMap::new()),
            substitute: std::sync::OnceLock::new(),
        }
    }

    fn load_type0(doc: &Document, dict: &Dict, base_font: String) -> Font {
        // The encoding CMap turns bytes into CIDs.
        let cmap = match doc.dict_get(dict, "Encoding") {
            Some(Object::Name(n)) => {
                if n.starts_with("Identity") {
                    CMap::identity()
                } else {
                    // A predefined non-identity CMap needs Adobe's CMap files, which are not
                    // bundled; identity keeps the codespace right for the two-byte majority.
                    CMap::identity()
                }
            }
            Some(obj) => obj
                .as_stream()
                .and_then(|s| doc.file.decode_stream(s).ok())
                .map(|d| CMap::parse(&d.data))
                .unwrap_or_else(CMap::identity),
            None => CMap::identity(),
        };

        // Metrics and the glyph program live on the descendant.
        let descendant = doc
            .dict_get(dict, "DescendantFonts")
            .and_then(|o| match o {
                Object::Array(items) => items.first().map(|i| doc.resolve(i)),
                other => Some(other),
            })
            .and_then(|o| o.as_dict().cloned())
            .unwrap_or_default();
        let descriptor = doc
            .dict_get(&descendant, "FontDescriptor")
            .and_then(|o| o.as_dict().cloned());

        let cid_to_gid = match doc.dict_get(&descendant, "CIDToGIDMap") {
            Some(Object::Stream(s)) => doc
                .file
                .decode_stream(&s)
                .ok()
                .map(|d| Arc::from(d.data.as_slice())),
            // `/Identity` (or absent) means the CID is the glyph id.
            _ => None,
        };

        Font {
            flags: derive_flags(doc, descriptor.as_ref(), &base_font),
            base_font,
            is_type3: false,
            font_matrix: Matrix::scale(0.001, 0.001),
            names: Vec::new(),
            symbolic: true,
            composite: Some(Composite { cmap, cid_to_gid }),
            to_unicode: load_to_unicode(doc, dict),
            widths: cid_widths(doc, &descendant),
            program: load_program(doc, descriptor.as_ref()),
            char_procs: Dict::new(),
            outline_cache: Mutex::new(HashMap::new()),
            substitute: std::sync::OnceLock::new(),
        }
    }

    fn load_type3(doc: &Document, dict: &Dict, base_font: String) -> Font {
        let font_matrix = doc
            .dict_get(dict, "FontMatrix")
            .as_ref()
            .and_then(|o| o.as_array())
            .map(|a| {
                let v: Vec<f32> = a.iter().filter_map(|o| o.as_f32()).collect();
                if v.len() >= 6 {
                    Matrix::new(v[0], v[1], v[2], v[3], v[4], v[5])
                } else {
                    Matrix::scale(0.001, 0.001)
                }
            })
            .unwrap_or_else(|| Matrix::scale(0.001, 0.001));

        Font {
            // A Type3 font has no program and no descriptor worth trusting; its glyphs are
            // arbitrary procedures, which is exactly what the symbolic flag means.
            flags: FontFlags(FontFlags::SYMBOLIC),
            base_font,
            is_type3: true,
            font_matrix,
            names: build_encoding(doc, dict, "", true),
            symbolic: true,
            composite: None,
            to_unicode: load_to_unicode(doc, dict),
            widths: simple_widths(doc, dict, None),
            program: Program::None,
            char_procs: doc
                .dict_get(dict, "CharProcs")
                .and_then(|o| o.as_dict().cloned())
                .unwrap_or_default(),
            outline_cache: Mutex::new(HashMap::new()),
            substitute: std::sync::OnceLock::new(),
        }
    }

    // ---- decoding ---------------------------------------------------------------------------

    /// Splits a PDF string into character codes with their advances and text.
    pub fn decode(&self, bytes: &[u8]) -> Vec<CodeItem> {
        match &self.composite {
            Some(comp) => comp
                .cmap
                .codespace
                .decode(bytes)
                .into_iter()
                .map(|code| {
                    let cid = comp.cmap.cid(code);
                    CodeItem {
                        code,
                        cid,
                        unicode: self.unicode_for(code),
                        width: self.width_for(code, cid),
                        // Word spacing is defined only for single-byte code 32, which a
                        // composite font's codespace does not produce in practice.
                        is_word_space: false,
                    }
                })
                .collect(),
            None => bytes
                .iter()
                .map(|&b| {
                    let code = u32::from(b);
                    CodeItem {
                        code,
                        cid: code,
                        unicode: self.unicode_for(code),
                        width: self.width_for(code, code),
                        is_word_space: b == 32,
                    }
                })
                .collect(),
        }
    }

    /// The text a code contributes: `/ToUnicode` first, then the glyph name it is encoded to.
    fn unicode_for(&self, code: u32) -> Option<String> {
        if let Some(map) = &self.to_unicode {
            if let Some(s) = map.unicode(code) {
                return Some(s);
            }
        }
        glyph_name_to_unicode(self.glyph_name(code))
    }

    /// The glyph name a simple font encodes `code` to, or `""`.
    fn glyph_name(&self, code: u32) -> &str {
        self.names
            .get(code as usize)
            .map(String::as_str)
            .unwrap_or("")
    }

    fn width_for(&self, code: u32, cid: u32) -> f32 {
        match &self.widths {
            Widths::Simple {
                first,
                widths,
                missing,
            } => {
                if let Some(w) = code
                    .checked_sub(*first)
                    .and_then(|i| widths.get(i as usize))
                    .copied()
                {
                    // A zero width in the array is legitimate (combining marks), so it is only
                    // overridden when the array is absent entirely.
                    return w;
                }
                // For one of the 14 standard fonts the built-in metrics are authoritative. For
                // any other font `/MissingWidth` is the descriptor's own answer and outranks a
                // guess made from a look-alike standard face.
                let standard = metrics::is_standard_font(&self.base_font);
                if standard {
                    if let Some(w) = metrics::base14_width(&self.base_font, self.glyph_name(code)) {
                        return w;
                    }
                }
                missing
                    .or_else(|| metrics::base14_width(&self.base_font, self.glyph_name(code)))
                    .unwrap_or(500.0)
            }
            Widths::Cid { default, map } => map.get(&cid).copied().unwrap_or(*default),
        }
    }

    // ---- outlines ---------------------------------------------------------------------------

    /// The outline for a decoded code, in glyph space (1000 units per em).
    ///
    /// `None` when the font has no interpretable program or the glyph is absent — the caller
    /// still has geometry and text, which is what extraction needs; only rendering needs this.
    pub fn outline(&self, item: &CodeItem) -> Option<Arc<Outline>> {
        if let Some(hit) = self.outline_cache.lock().unwrap().get(&item.cid) {
            return hit.clone();
        }
        let built = self.build_outline(item).map(Arc::new);
        self.outline_cache
            .lock()
            .unwrap()
            .insert(item.cid, built.clone());
        built
    }

    fn build_outline(&self, item: &CodeItem) -> Option<Outline> {
        self.embedded_outline(item)
            .or_else(|| self.substitute_outline(item))
    }

    /// Draws the glyph from a substitute system face, keeping the document's own advance.
    ///
    /// Reached when a font embeds no program — the standard 14 typically do not — or embeds one
    /// this crate cannot interpret. Without it those pages render with no text at all.
    fn substitute_outline(&self, item: &CodeItem) -> Option<Outline> {
        if self.is_type3 {
            return None;
        }
        let data = self.substitute.get_or_init(|| {
            system::substitute(system::Style {
                serif: self.flags.is_serif(),
                fixed_pitch: self.flags.is_fixed_pitch(),
                bold: self.flags.is_bold(),
                italic: self.flags.is_italic(),
            })
        });
        let face = ttf_parser::Face::parse(data.as_ref()?, 0).ok()?;
        // A substitute has its own glyph order, so the only durable key into it is Unicode.
        let ch = item.unicode.as_deref()?.chars().next()?;
        let gid = face.glyph_index(ch)?;
        let mut sink = Sink::default();
        face.outline_glyph(gid, &mut sink)?;
        let upem = f32::from(face.units_per_em()).max(1.0);
        Some(sink.finish(1000.0 / upem, item.width))
    }

    fn embedded_outline(&self, item: &CodeItem) -> Option<Outline> {
        match &self.program {
            Program::Sfnt(data) => {
                let face = ttf_parser::Face::parse(data, 0).ok()?;
                let gid = self.sfnt_glyph_id(&face, item)?;
                let mut sink = Sink::default();
                face.outline_glyph(gid, &mut sink)?;
                // Font units per em vary (1000 for CFF-flavoured, 2048 for most TrueType);
                // normalise so callers only ever see glyph space.
                let upem = f32::from(face.units_per_em()).max(1.0);
                Some(sink.finish(1000.0 / upem, item.width))
            }
            Program::Cff(data) => {
                let table = ttf_parser::cff::Table::parse(data)?;
                let gid = self.cff_glyph_id(&table, item)?;
                let mut sink = Sink::default();
                table.outline(gid, &mut sink).ok()?;
                // A CFF font matrix is usually 1/1000, but subsetters do emit others.
                let scale = table.matrix().sx * 1000.0;
                Some(sink.finish(if scale.abs() < 1e-6 { 1.0 } else { scale }, item.width))
            }
            Program::None => None,
        }
    }

    /// Resolves a code to a glyph id in an sfnt program.
    fn sfnt_glyph_id(
        &self,
        face: &ttf_parser::Face,
        item: &CodeItem,
    ) -> Option<ttf_parser::GlyphId> {
        if self.composite.is_some() {
            return Some(ttf_parser::GlyphId(self.composite_gid(item.cid)));
        }
        let name = self.glyph_name(item.code);

        // A symbolic font's own cmap is authoritative and is keyed by the raw code — often in
        // the (3,0) subtable at 0xF000 + code, which is where symbol fonts hide.
        if self.symbolic {
            if let Some(gid) = symbol_cmap_lookup(face, item.code) {
                return Some(gid);
            }
        }
        if !name.is_empty() {
            if let Some(gid) = face.glyph_index_by_name(name) {
                return Some(gid);
            }
            if let Some(ch) = glyph_name_to_unicode(name).and_then(|s| s.chars().next()) {
                if let Some(gid) = face.glyph_index(ch) {
                    return Some(gid);
                }
            }
        }
        if !self.symbolic {
            if let Some(gid) = symbol_cmap_lookup(face, item.code) {
                return Some(gid);
            }
        }
        // Subset fonts with no usable cmap are conventionally indexed by code.
        Some(ttf_parser::GlyphId(item.code as u16))
    }

    /// Resolves a code to a glyph id in a bare CFF program.
    fn cff_glyph_id(
        &self,
        table: &ttf_parser::cff::Table,
        item: &CodeItem,
    ) -> Option<ttf_parser::GlyphId> {
        if self.composite.is_some() {
            return Some(ttf_parser::GlyphId(self.composite_gid(item.cid)));
        }
        let name = self.glyph_name(item.code);
        if !name.is_empty() {
            if let Some(gid) = table.glyph_index_by_name(name) {
                return Some(gid);
            }
        }
        if let Ok(code) = u8::try_from(item.code) {
            if let Some(gid) = table.glyph_index(code) {
                return Some(gid);
            }
        }
        Some(ttf_parser::GlyphId(item.cid as u16))
    }

    /// A composite font's CID to glyph id, through `/CIDToGIDMap` when one is present.
    fn composite_gid(&self, cid: u32) -> u16 {
        let Some(map) = self.composite.as_ref().and_then(|c| c.cid_to_gid.as_ref()) else {
            return cid as u16;
        };
        let i = cid as usize * 2;
        match (map.get(i), map.get(i + 1)) {
            (Some(&hi), Some(&lo)) => u16::from_be_bytes([hi, lo]),
            // Past the end of the map means unmapped, which is glyph 0.
            _ => 0,
        }
    }

    // ---- Type3 ------------------------------------------------------------------------------

    /// The content stream that draws `code`, for a Type3 font.
    pub fn type3_proc(&self, doc: &Document, code: u32) -> Option<Object> {
        let name = self.glyph_name(code);
        if name.is_empty() {
            return None;
        }
        doc.dict_get(&self.char_procs, name)
    }
}

/// Looks a raw code up in the cmap subtables symbol fonts use.
fn symbol_cmap_lookup(face: &ttf_parser::Face, code: u32) -> Option<ttf_parser::GlyphId> {
    use ttf_parser::PlatformId;
    let cmap = face.tables().cmap?;
    for sub in cmap.subtables {
        match sub.platform_id {
            // (3,0) Windows Symbol: codes live in the 0xF000 private-use page, but broken
            // producers write the bare code too.
            PlatformId::Windows if sub.encoding_id == 0 => {
                for candidate in [0xF000 + (code & 0xFF), code] {
                    if let Some(gid) = sub.glyph_index(candidate) {
                        return Some(gid);
                    }
                }
            }
            // (1,0) Macintosh Roman: a straight byte lookup.
            PlatformId::Macintosh if sub.encoding_id == 0 => {
                if let Some(gid) = sub.glyph_index(code) {
                    return Some(gid);
                }
            }
            _ => {}
        }
    }
    None
}

/// Bridges `ttf-parser`'s outline callbacks into our own path builder.
#[derive(Default)]
struct Sink {
    builder: OutlineBuilder,
}

impl Sink {
    fn finish(self, scale: f32, advance: f32) -> Outline {
        let mut cmds = self.builder.finish();
        if (scale - 1.0).abs() > 1e-6 {
            for cmd in &mut cmds {
                scale_cmd(cmd, scale);
            }
        }
        Outline { cmds, advance }
    }
}

fn scale_cmd(cmd: &mut glyph::PathCmd, s: f32) {
    use glyph::PathCmd::*;
    let scale_point = |p: &mut crate::geom::Point| {
        p.x *= s;
        p.y *= s;
    };
    match cmd {
        MoveTo(p) | LineTo(p) => scale_point(p),
        CurveTo(a, b, c) => {
            scale_point(a);
            scale_point(b);
            scale_point(c);
        }
        Close => {}
    }
}

impl ttf_parser::OutlineBuilder for Sink {
    fn move_to(&mut self, x: f32, y: f32) {
        self.builder.move_to(x, y);
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.builder.line_to(x, y);
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        // Elevate the quadratic to a cubic so the path type stays uniform: the cubic controls
        // sit two thirds of the way from each endpoint toward the quadratic control point.
        let (x0, y0) = (self.builder.x, self.builder.y);
        self.builder.curve_to(
            x0 + 2.0 / 3.0 * (x1 - x0),
            y0 + 2.0 / 3.0 * (y1 - y0),
            x + 2.0 / 3.0 * (x1 - x),
            y + 2.0 / 3.0 * (y1 - y),
            x,
            y,
        );
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.builder.curve_to(x1, y1, x2, y2, x, y);
    }

    fn close(&mut self) {
        self.builder.close();
    }
}

// ---- shared loading helpers -----------------------------------------------------------------

fn load_to_unicode(doc: &Document, dict: &Dict) -> Option<CMap> {
    let stream = doc.dict_get(dict, "ToUnicode")?;
    let decoded = doc.file.decode_stream(stream.as_stream()?).ok()?;
    let map = CMap::parse(&decoded.data);
    map.has_unicode().then_some(map)
}

/// Picks the embedded program out of a font descriptor.
fn load_program(doc: &Document, descriptor: Option<&Dict>) -> Program {
    let Some(desc) = descriptor else {
        return Program::None;
    };
    // `/FontFile` is Type1, whose eexec-encrypted charstrings are not interpreted here; its
    // metrics still come from `/Widths`, so text extraction is unaffected.
    for key in ["FontFile2", "FontFile3", "FontFile"] {
        let Some(obj) = doc.dict_get(desc, key) else {
            continue;
        };
        let Some(stream) = obj.as_stream() else {
            continue;
        };
        let Ok(decoded) = doc.file.decode_stream(stream) else {
            continue;
        };
        let data: Arc<[u8]> = Arc::from(decoded.data.as_slice());
        let subtype = stream.dict.get("Subtype").and_then(|o| o.as_name());
        return match key {
            "FontFile2" => Program::Sfnt(data),
            "FontFile3" => match subtype {
                // An OpenType wrapper is a full sfnt; the bare CFF subtypes are not.
                Some("OpenType") => Program::Sfnt(data),
                _ => Program::Cff(data),
            },
            _ => Program::None,
        };
    }
    Program::None
}

/// Builds the 256-entry code to glyph-name table for a simple or Type3 font.
fn build_encoding(doc: &Document, dict: &Dict, base_font: &str, symbolic: bool) -> Vec<String> {
    let encoding_obj = doc.dict_get(dict, "Encoding");

    let named = |n: &str| match n {
        "WinAnsiEncoding" => Some(Encoding::WinAnsi),
        "MacRomanEncoding" => Some(Encoding::MacRoman),
        "StandardEncoding" | "MacExpertEncoding" => Some(Encoding::Standard),
        _ => None,
    };

    let mut base = match &encoding_obj {
        Some(Object::Name(n)) => named(n),
        Some(obj) => obj
            .as_dict()
            .and_then(|d| d.get("BaseEncoding"))
            .and_then(|o| o.as_name())
            .and_then(named),
        None => None,
    };
    if base.is_none() {
        let lower = base_font.to_ascii_lowercase();
        if lower.contains("symbol") {
            base = Some(Encoding::Symbol);
        } else if !symbolic {
            // A nonsymbolic font with no stated encoding uses the standard one.
            base = Some(Encoding::Standard);
        }
    }

    let mut names = vec![String::new(); 256];
    if let Some(enc) = base {
        for (code, slot) in names.iter_mut().enumerate() {
            *slot = base_encoding_name(enc, code as u8).to_string();
        }
    }

    // `/Differences` is a flat array where each integer restarts the code counter and each name
    // assigns the current code, then advances it.
    if let Some(diffs) = encoding_obj
        .as_ref()
        .and_then(|o| o.as_dict())
        .and_then(|d| doc.dict_get(d, "Differences"))
        .as_ref()
        .and_then(|o| o.as_array())
    {
        let mut code: i64 = 0;
        for item in diffs {
            match doc.resolve(item) {
                Object::Int(i) => code = i,
                Object::Real(r) => code = r as i64,
                Object::Name(n) => {
                    if let Some(slot) = usize::try_from(code).ok().and_then(|c| names.get_mut(c)) {
                        *slot = n;
                    }
                    code += 1;
                }
                _ => {}
            }
        }
    }
    names
}

fn simple_widths(doc: &Document, dict: &Dict, descriptor: Option<&Dict>) -> Widths {
    let first = doc
        .dict_get(dict, "FirstChar")
        .and_then(|o| o.as_int())
        .unwrap_or(0)
        .clamp(0, u32::MAX as i64) as u32;
    let widths: Vec<f32> = doc
        .dict_get(dict, "Widths")
        .as_ref()
        .and_then(|o| o.as_array())
        .map(|a| {
            a.iter()
                .map(|o| doc.resolve(o).as_f32().unwrap_or(0.0))
                .collect()
        })
        .unwrap_or_default();
    let missing = descriptor
        .and_then(|d| doc.dict_get(d, "MissingWidth"))
        .and_then(|o| o.as_f32());
    Widths::Simple {
        first,
        widths,
        missing,
    }
}

/// Parses a CID font's `/W` array: alternating `code [w w ...]` runs and `first last w` ranges.
fn cid_widths(doc: &Document, descendant: &Dict) -> Widths {
    const MAX_RANGE: u32 = 65_536;

    let default = doc
        .dict_get(descendant, "DW")
        .and_then(|o| o.as_f32())
        .unwrap_or(1000.0);
    let mut map = HashMap::new();

    if let Some(w) = doc
        .dict_get(descendant, "W")
        .as_ref()
        .and_then(|o| o.as_array())
    {
        let items: Vec<Object> = w.iter().map(|o| doc.resolve(o)).collect();
        let mut i = 0;
        while i < items.len() {
            let Some(start) = items[i].as_f32().map(|f| f.max(0.0) as u32) else {
                i += 1;
                continue;
            };
            match items.get(i + 1) {
                Some(Object::Array(list)) => {
                    for (k, item) in list.iter().enumerate() {
                        if let Some(width) = doc.resolve(item).as_f32() {
                            map.insert(start + k as u32, width);
                        }
                    }
                    i += 2;
                }
                Some(end_obj) => {
                    let end = end_obj.as_f32().map(|f| f.max(0.0) as u32).unwrap_or(start);
                    let width = items.get(i + 2).and_then(|o| o.as_f32()).unwrap_or(default);
                    // A corrupt range can span the whole CID space; cap the work it can cause.
                    for cid in start..=end.min(start.saturating_add(MAX_RANGE)) {
                        map.insert(cid, width);
                    }
                    i += 3;
                }
                None => break,
            }
        }
    }
    Widths::Cid { default, map }
}
