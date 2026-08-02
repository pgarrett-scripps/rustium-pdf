//! Substitute faces for fonts a document does not embed.
//!
//! A PDF may reference Helvetica or Times and ship no program, on the understanding that the
//! viewer supplies one. Extraction does not care — the widths come from the metrics tables — but
//! rendering does: without a substitute, every glyph of a non-embedded font silently disappears
//! and the page comes out blank of text.
//!
//! Only the glyph *shapes* come from the substitute. Positions and advances always come from the
//! document, so a substitute with different metrics shifts nothing; it only draws letters that
//! are a little wide or narrow inside slots the PDF chose.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

/// The style slot a substitute is chosen for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Style {
    pub serif: bool,
    pub fixed_pitch: bool,
    pub bold: bool,
    pub italic: bool,
}

/// Candidate file names per slot, best first.
///
/// The Liberation family leads because it is metrically compatible with Arial, Times New Roman
/// and Courier New — the faces PDFs actually name — so substituted text occupies very nearly the
/// space the document laid out for it. DejaVu and Noto follow as near-universal fallbacks.
fn candidates(style: Style) -> Vec<String> {
    let (family, dejavu, noto) = if style.fixed_pitch {
        ("LiberationMono", "DejaVuSansMono", "NotoSansMono")
    } else if style.serif {
        ("LiberationSerif", "DejaVuSerif", "NotoSerif")
    } else {
        ("LiberationSans", "DejaVuSans", "NotoSans")
    };

    // Each family spells its styles differently: Liberation and Noto use `-Regular`/`-Italic`,
    // DejaVu uses a bare name and `-Oblique` for its sans faces.
    let liberation = match (style.bold, style.italic) {
        (true, true) => "-BoldItalic",
        (true, false) => "-Bold",
        (false, true) => "-Italic",
        (false, false) => "-Regular",
    };
    let dejavu_suffix = match (style.bold, style.italic) {
        (true, true) => {
            if style.serif {
                "-BoldItalic"
            } else {
                "-BoldOblique"
            }
        }
        (true, false) => "-Bold",
        (false, true) => {
            if style.serif {
                "-Italic"
            } else {
                "-Oblique"
            }
        }
        (false, false) => "",
    };

    let mut out = vec![
        format!("{family}{liberation}.ttf"),
        format!("{dejavu}{dejavu_suffix}.ttf"),
        format!("{noto}{liberation}.ttf"),
    ];
    // Falling back across styles beats drawing nothing at all.
    if style.bold || style.italic {
        out.push(format!("{family}-Regular.ttf"));
        out.push(format!("{dejavu}.ttf"));
        out.push(format!("{noto}-Regular.ttf"));
    }
    out
}

/// Directories searched for substitutes, in order.
fn font_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    // An explicit override wins, so a deployment can pin the faces it ships with.
    if let Ok(dir) = std::env::var("RUSTIUM_FONT_DIR") {
        dirs.extend(std::env::split_paths(&dir));
    }
    if let Ok(home) = std::env::var("HOME") {
        dirs.push(PathBuf::from(&home).join(".local/share/fonts"));
        dirs.push(PathBuf::from(&home).join(".fonts"));
    }
    dirs.extend(
        [
            "/usr/share/fonts",
            "/usr/local/share/fonts",
            "/Library/Fonts",
            "/System/Library/Fonts",
            "C:\\Windows\\Fonts",
        ]
        .iter()
        .map(PathBuf::from),
    );
    dirs
}

/// Every font file on the system, by file name. Built once per process.
///
/// The index holds paths, not contents: a font directory is a few hundred files and several
/// hundred megabytes, so the scan is cheap but loading it all would not be.
fn index() -> &'static Vec<(String, PathBuf)> {
    static INDEX: OnceLock<Vec<(String, PathBuf)>> = OnceLock::new();
    INDEX.get_or_init(|| {
        let mut out = Vec::new();
        for dir in font_dirs() {
            collect(&dir, 0, &mut out);
        }
        out
    })
}

fn collect(dir: &std::path::Path, depth: usize, out: &mut Vec<(String, PathBuf)>) {
    const MAX_DEPTH: usize = 4;
    const MAX_FILES: usize = 5_000;
    if depth > MAX_DEPTH || out.len() >= MAX_FILES {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, depth + 1, out);
        } else if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            let lower = name.to_ascii_lowercase();
            if lower.ends_with(".ttf") || lower.ends_with(".otf") || lower.ends_with(".ttc") {
                out.push((lower, path.clone()));
            }
        }
    }
}

/// Loads a substitute face for `style`, or `None` when the system has nothing usable.
///
/// Results are cached per style, so a document using four faces reads at most four files no
/// matter how many pages or glyphs it has.
pub fn substitute(style: Style) -> Option<Arc<[u8]>> {
    use std::collections::HashMap;
    use std::sync::Mutex;

    type Cache = HashMap<(bool, bool, bool, bool), Option<Arc<[u8]>>>;
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let key = (style.serif, style.fixed_pitch, style.bold, style.italic);

    if let Some(hit) = cache.lock().unwrap().get(&key) {
        return hit.clone();
    }

    let index = index();
    let mut found = None;
    'outer: for want in candidates(style) {
        let want = want.to_ascii_lowercase();
        for (name, path) in index {
            if *name == want {
                if let Ok(data) = std::fs::read(path) {
                    found = Some(Arc::from(data.as_slice()));
                    break 'outer;
                }
            }
        }
    }

    cache.lock().unwrap().insert(key, found.clone());
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidate_order_prefers_metric_compatible_faces() {
        let serif_bold = candidates(Style {
            serif: true,
            fixed_pitch: false,
            bold: true,
            italic: false,
        });
        assert_eq!(serif_bold[0], "LiberationSerif-Bold.ttf");
        assert!(serif_bold.iter().any(|c| c == "DejaVuSerif-Bold.ttf"));
        // A styled request falls back to the upright face rather than to nothing.
        assert!(serif_bold
            .iter()
            .any(|c| c == "LiberationSerif-Regular.ttf"));
    }

    #[test]
    fn dejavu_sans_uses_oblique_not_italic() {
        let sans_italic = candidates(Style {
            serif: false,
            fixed_pitch: false,
            bold: false,
            italic: true,
        });
        assert!(sans_italic.iter().any(|c| c == "DejaVuSans-Oblique.ttf"));
    }

    /// Substitution is best-effort by nature, so this asserts the lookup runs and caches rather
    /// than that any particular font is installed.
    #[test]
    fn lookup_is_stable_across_calls() {
        let style = Style {
            serif: false,
            fixed_pitch: false,
            bold: false,
            italic: false,
        };
        let a = substitute(style);
        let b = substitute(style);
        assert_eq!(a.is_some(), b.is_some());
        if let (Some(a), Some(b)) = (a, b) {
            assert!(Arc::ptr_eq(&a, &b), "second lookup should hit the cache");
        }
    }
}
