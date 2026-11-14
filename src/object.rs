//! The COS object model: everything a PDF body is made of.

use std::collections::HashMap;
use std::sync::Arc;

/// An indirect object reference: object number and generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ObjRef {
    pub num: u32,
    pub gen: u16,
}

impl ObjRef {
    pub const fn new(num: u32, gen: u16) -> Self {
        Self { num, gen }
    }
}

/// A dictionary. Keys are names with the leading `/` and `#xx` escapes already resolved.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Dict(pub HashMap<String, Object>);

impl Dict {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, key: &str) -> Option<&Object> {
        self.0.get(key)
    }

    pub fn insert(&mut self, key: impl Into<String>, value: Object) {
        self.0.insert(key.into(), value);
    }

    pub fn contains(&self, key: &str) -> bool {
        self.0.contains_key(key)
    }
}

/// A stream: its dictionary plus the raw (still filtered, possibly encrypted) bytes.
///
/// The bytes are behind an `Arc` so cloning the containing [`Object`] does not copy stream
/// payloads; document loading hands out clones freely.
#[derive(Debug, Clone, PartialEq)]
pub struct Stream {
    pub dict: Dict,
    pub raw: Arc<[u8]>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub enum Object {
    #[default]
    Null,
    Bool(bool),
    Int(i64),
    Real(f64),
    /// A string's decoded bytes. Interpretation (PDFDocEncoding, UTF-16BE) is the caller's.
    String(Vec<u8>),
    Name(String),
    Array(Vec<Object>),
    Dict(Dict),
    Stream(Stream),
    Ref(ObjRef),
}

impl Object {
    pub fn as_int(&self) -> Option<i64> {
        match self {
            Object::Int(i) => Some(*i),
            Object::Real(r) => Some(*r as i64),
            _ => None,
        }
    }

    pub fn as_f32(&self) -> Option<f32> {
        match self {
            Object::Int(i) => Some(*i as f32),
            Object::Real(r) => Some(*r as f32),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Object::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_name(&self) -> Option<&str> {
        match self {
            Object::Name(n) => Some(n),
            _ => None,
        }
    }

    pub fn as_string(&self) -> Option<&[u8]> {
        match self {
            Object::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Object]> {
        match self {
            Object::Array(a) => Some(a),
            _ => None,
        }
    }

    /// The dictionary of a dict *or* a stream: many entries (`/Resources`, `/Font`) may be
    /// either, and callers almost never care which.
    pub fn as_dict(&self) -> Option<&Dict> {
        match self {
            Object::Dict(d) => Some(d),
            Object::Stream(s) => Some(&s.dict),
            _ => None,
        }
    }

    pub fn as_stream(&self) -> Option<&Stream> {
        match self {
            Object::Stream(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_ref_id(&self) -> Option<ObjRef> {
        match self {
            Object::Ref(r) => Some(*r),
            _ => None,
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Object::Null)
    }
}

/// Decodes a PDF *text string* (ISO 32000 §7.9.2.2): UTF-16 when it opens with a byte-order
/// mark, UTF-8 when it opens with that encoding's mark (PDF 2.0), PDFDocEncoding otherwise.
///
/// Bytes PDFDocEncoding leaves undefined are dropped rather than guessed at.
pub(crate) fn text_string(bytes: &[u8]) -> String {
    let utf16 = |rest: &[u8], big_endian: bool| {
        // A dangling odd byte is left in the remainder and ignored.
        let (pairs, _) = rest.as_chunks::<2>();
        let units: Vec<u16> = pairs
            .iter()
            .map(|&pair| {
                if big_endian {
                    u16::from_be_bytes(pair)
                } else {
                    u16::from_le_bytes(pair)
                }
            })
            .collect();
        String::from_utf16_lossy(&units)
    };
    match bytes {
        [0xFE, 0xFF, rest @ ..] => utf16(rest, true),
        // Not in the standard, but written by enough producers that every reader accepts it.
        [0xFF, 0xFE, rest @ ..] => utf16(rest, false),
        [0xEF, 0xBB, 0xBF, rest @ ..] => String::from_utf8_lossy(rest).into_owned(),
        _ => bytes.iter().filter_map(|&b| pdf_doc_char(b)).collect(),
    }
}

/// One PDFDocEncoding byte as a character (ISO 32000 Annex D.3).
fn pdf_doc_char(byte: u8) -> Option<char> {
    /// Codes 0x18..=0x1F: the spacing diacritics.
    const ACCENTS: [char; 8] = [
        '\u{02D8}', '\u{02C7}', '\u{02C6}', '\u{02D9}', '\u{02DD}', '\u{02DB}', '\u{02DA}',
        '\u{02DC}',
    ];
    /// Codes 0x80..=0x9E: typographic punctuation, ligatures and a few Latin Extended letters.
    const HIGH: [char; 31] = [
        '\u{2022}', '\u{2020}', '\u{2021}', '\u{2026}', '\u{2014}', '\u{2013}', '\u{0192}',
        '\u{2044}', '\u{2039}', '\u{203A}', '\u{2212}', '\u{2030}', '\u{201E}', '\u{201C}',
        '\u{201D}', '\u{2018}', '\u{2019}', '\u{201A}', '\u{2122}', '\u{FB01}', '\u{FB02}',
        '\u{0141}', '\u{0152}', '\u{0160}', '\u{0178}', '\u{017D}', '\u{0131}', '\u{0142}',
        '\u{0153}', '\u{0161}', '\u{017E}',
    ];
    match byte {
        0x09 | 0x0A | 0x0D | 0x20..=0x7E => Some(byte as char),
        0x18..=0x1F => Some(ACCENTS[(byte - 0x18) as usize]),
        0x80..=0x9E => Some(HIGH[(byte - 0x80) as usize]),
        0xA0 => Some('\u{20AC}'),
        // The rest of the upper half is Latin-1, less the soft hyphen's slot.
        0xA1..=0xAC | 0xAE..=0xFF => Some(byte as char),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::text_string;

    #[test]
    fn a_text_string_is_utf16_only_behind_a_byte_order_mark() {
        assert_eq!(text_string(b"\xFE\xFF\x00f\x00i"), "fi");
        assert_eq!(text_string(b"\xFF\xFEf\x00i\x00"), "fi");
        // A surrogate pair, and a dangling odd byte that is ignored rather than misread.
        assert_eq!(text_string(b"\xFE\xFF\xD8\x35\xDC\x00\x00"), "\u{1D400}");
        assert_eq!(text_string(b"\xEF\xBB\xBF\xC3\xA9"), "\u{e9}");
        assert_eq!(text_string(b""), "");
    }

    #[test]
    fn a_text_string_without_a_mark_is_pdf_doc_encoding() {
        assert_eq!(text_string(b"fi 68"), "fi 68");
        // Where PDFDocEncoding parts from Latin-1: 0x93 is the fi ligature, 0x85 an en dash,
        // 0xA0 the euro sign. 0xE9 is where the two agree.
        assert_eq!(
            text_string(b"\x93\x85\xA0\xE9"),
            "\u{FB01}\u{2013}\u{20AC}\u{e9}"
        );
        // Undefined codes vanish instead of becoming control characters.
        assert_eq!(text_string(b"a\x7F\x9F\xADb\x01"), "ab");
    }
}
