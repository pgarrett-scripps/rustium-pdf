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
