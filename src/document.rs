//! The document: catalog, page tree, and per-page access.

use std::collections::HashSet;
use std::path::Path;

use crate::error::{Error, Result};
use crate::geom::Rect;
use crate::object::{Dict, Object};
use crate::page::Page;
use crate::parser::PdfFile;

/// Attributes a page inherits from its ancestors when it lacks its own.
#[derive(Clone, Default)]
struct Inherited {
    resources: Option<Object>,
    media_box: Option<Rect>,
    crop_box: Option<Rect>,
    rotate: Option<i64>,
}

/// One leaf of the page tree, fully resolved.
pub(crate) struct PageNode {
    pub dict: Dict,
    pub resources: Dict,
    pub media_box: Rect,
    pub crop_box: Rect,
    /// Clockwise display rotation in degrees: 0, 90, 180 or 270.
    pub rotation: i32,
}

/// An open PDF document.
///
/// `Document` is `Send + Sync`: internal caches are lock-guarded and there is no global state,
/// so separate documents can be processed on separate threads freely — and a single document
/// can be shared across threads too.
pub struct Document {
    pub(crate) file: PdfFile,
    pages: Vec<PageNode>,
}

impl Document {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_with_password(path, None)
    }

    pub fn open_with_password(path: impl AsRef<Path>, password: Option<&str>) -> Result<Self> {
        let data = std::fs::read(path)?;
        Self::from_bytes(data, password)
    }

    pub fn from_bytes(data: Vec<u8>, password: Option<&str>) -> Result<Self> {
        let file = PdfFile::load(data, password)?;
        let pages = collect_pages(&file)?;
        Ok(Self { file, pages })
    }

    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    /// Loads one page and interprets its content streams into primitives.
    pub fn page(&self, index: usize) -> Result<Page> {
        let node = self
            .pages
            .get(index)
            .ok_or(Error::PageOutOfRange(index))?;
        Page::build(self, node, index)
    }

    pub(crate) fn resolve(&self, obj: &Object) -> Object {
        self.file.resolve(obj)
    }

    /// Resolved value of `key` in `dict`.
    pub(crate) fn dict_get(&self, dict: &Dict, key: &str) -> Option<Object> {
        dict.get(key).map(|o| self.resolve(o)).filter(|o| !o.is_null())
    }
}

fn rect_from(obj: &Object) -> Option<Rect> {
    let a = obj.as_array()?;
    let v: Vec<f32> = a.iter().filter_map(|o| o.as_f32()).collect();
    if v.len() < 4 {
        return None;
    }
    let r = Rect::from_corners(v[0], v[1], v[2], v[3]);
    (!r.is_empty()).then_some(r)
}

fn collect_pages(file: &PdfFile) -> Result<Vec<PageNode>> {
    const MAX_PAGES: usize = 100_000;
    const MAX_DEPTH: usize = 64;

    let root = file
        .trailer
        .get("Root")
        .map(|o| file.resolve(o))
        .and_then(|o| o.as_dict().cloned())
        .ok_or_else(|| Error::Parse("catalog missing".into()))?;
    let pages_obj = root
        .get("Pages")
        .map(|o| file.resolve(o))
        .ok_or_else(|| Error::Parse("catalog has no /Pages".into()))?;

    let mut out = Vec::new();
    let mut visited: HashSet<usize> = HashSet::new();

    // Stack of (node, inherited, depth). Kids are pushed in reverse so page order is preserved.
    let mut stack: Vec<(Object, Inherited, usize)> = vec![(pages_obj, Inherited::default(), 0)];
    while let Some((node_ref, inherited, depth)) = stack.pop() {
        if out.len() >= MAX_PAGES || depth > MAX_DEPTH {
            break;
        }
        // Cycle guard keyed on object identity where available.
        if let Object::Ref(r) = &node_ref {
            if !visited.insert(r.num as usize) {
                continue;
            }
        }
        let node = file.resolve(&node_ref);
        let Some(dict) = node.as_dict() else { continue };

        let mut inh = inherited.clone();
        if let Some(r) = dict.get("Resources") {
            inh.resources = Some(r.clone());
        }
        if let Some(r) = dict.get("MediaBox").map(|o| file.resolve(o)) {
            if let Some(rect) = rect_from(&r) {
                inh.media_box = Some(rect);
            }
        }
        if let Some(r) = dict.get("CropBox").map(|o| file.resolve(o)) {
            if let Some(rect) = rect_from(&r) {
                inh.crop_box = Some(rect);
            }
        }
        if let Some(r) = dict.get("Rotate").map(|o| file.resolve(o)).and_then(|o| o.as_int()) {
            inh.rotate = Some(r);
        }

        let ty = dict.get("Type").and_then(|o| o.as_name());
        let kids = dict.get("Kids").map(|o| file.resolve(o));
        match (ty, &kids) {
            // Treat anything with /Kids as an internal node; some producers mislabel /Type.
            (Some("Pages"), _) | (_, Some(Object::Array(_))) => {
                if let Some(Object::Array(kids)) = kids {
                    for kid in kids.iter().rev() {
                        stack.push((kid.clone(), inh.clone(), depth + 1));
                    }
                }
            }
            _ => {
                // A leaf. US Letter is the fallback of last resort for a missing MediaBox.
                let media_box = inh
                    .media_box
                    .unwrap_or(Rect::from_corners(0.0, 0.0, 612.0, 792.0));
                let crop_box = inh
                    .crop_box
                    .map(|c| {
                        let i = c.intersect(&media_box);
                        if i.is_empty() {
                            media_box
                        } else {
                            i
                        }
                    })
                    .unwrap_or(media_box);
                let resources = inh
                    .resources
                    .as_ref()
                    .map(|o| file.resolve(o))
                    .and_then(|o| o.as_dict().cloned())
                    .unwrap_or_default();
                let rotation = inh.rotate.unwrap_or(0).rem_euclid(360) as i32;
                let rotation = (rotation / 90) * 90; // Anything non-multiple is treated as 0/90/180/270 floor.
                out.push(PageNode {
                    dict: dict.clone(),
                    resources,
                    media_box,
                    crop_box,
                    rotation,
                });
            }
        }
    }
    Ok(out)
}

/// Concatenates a page's content streams, newline-separated, decoded.
pub(crate) fn page_content(doc: &Document, page: &Dict) -> Vec<u8> {
    let Some(contents) = doc.dict_get(page, "Contents") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut push_stream = |obj: &Object| {
        if let Some(stream) = doc.resolve(obj).as_stream() {
            if let Ok(decoded) = doc.file.decode_stream(stream) {
                if !out.is_empty() {
                    out.push(b'\n');
                }
                out.extend_from_slice(&decoded.data);
            }
        }
    };
    match &contents {
        Object::Array(items) => {
            for item in items {
                push_stream(item);
            }
        }
        other => push_stream(other),
    }
    out
}
