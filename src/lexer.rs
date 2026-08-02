//! A byte cursor and recursive-descent reader for COS syntax.
//!
//! Used in two places: the file body parser (which additionally understands `obj`, `stream` and
//! cross-reference keywords) and the content-stream tokenizer (which additionally understands
//! operators). Both share the object grammar, which lives here.

use crate::object::{Dict, Object, ObjRef, Stream};

pub fn is_whitespace(b: u8) -> bool {
    matches!(b, b'\0' | b'\t' | b'\n' | b'\x0c' | b'\r' | b' ')
}

pub fn is_delimiter(b: u8) -> bool {
    matches!(b, b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%')
}

pub fn is_regular(b: u8) -> bool {
    !is_whitespace(b) && !is_delimiter(b)
}

#[derive(Clone)]
pub struct Cursor<'a> {
    pub data: &'a [u8],
    pub pos: usize,
}

impl<'a> Cursor<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    pub fn at(data: &'a [u8], pos: usize) -> Self {
        Self {
            data,
            pos: pos.min(data.len()),
        }
    }

    pub fn peek(&self) -> Option<u8> {
        self.data.get(self.pos).copied()
    }

    pub fn bump(&mut self) -> Option<u8> {
        let b = self.peek()?;
        self.pos += 1;
        Some(b)
    }

    pub fn eof(&self) -> bool {
        self.pos >= self.data.len()
    }

    /// Skips whitespace and `%` comments.
    pub fn skip_ws(&mut self) {
        while let Some(b) = self.peek() {
            if is_whitespace(b) {
                self.pos += 1;
            } else if b == b'%' {
                while let Some(b) = self.peek() {
                    if b == b'\n' || b == b'\r' {
                        break;
                    }
                    self.pos += 1;
                }
            } else {
                break;
            }
        }
    }

    /// Reads a run of regular characters (a keyword, operator or number).
    pub fn read_regular(&mut self) -> &'a [u8] {
        let start = self.pos;
        while let Some(b) = self.peek() {
            if is_regular(b) {
                self.pos += 1;
            } else {
                break;
            }
        }
        &self.data[start..self.pos]
    }

    /// True and consumed if the next bytes are exactly `kw` followed by a non-regular byte.
    pub fn eat_keyword(&mut self, kw: &[u8]) -> bool {
        let end = self.pos + kw.len();
        if end <= self.data.len()
            && &self.data[self.pos..end] == kw
            && self.data.get(end).is_none_or(|&b| !is_regular(b))
        {
            self.pos = end;
            true
        } else {
            false
        }
    }

    /// Parses a name, positioned after the `/`.
    pub fn read_name(&mut self) -> String {
        let mut out = Vec::new();
        while let Some(b) = self.peek() {
            if !is_regular(b) {
                break;
            }
            self.pos += 1;
            if b == b'#' {
                let hi = self.peek().and_then(hex_val);
                if let Some(hi) = hi {
                    self.pos += 1;
                    let lo = self.peek().and_then(hex_val);
                    if let Some(lo) = lo {
                        self.pos += 1;
                        out.push(hi * 16 + lo);
                        continue;
                    }
                }
                // A `#` not followed by two hex digits is kept literally, matching viewers.
                out.push(b'#');
            } else {
                out.push(b);
            }
        }
        String::from_utf8_lossy(&out).into_owned()
    }

    /// Parses a literal string, positioned after the `(`.
    pub fn read_literal_string(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        let mut depth = 1usize;
        while let Some(b) = self.bump() {
            match b {
                b'(' => {
                    depth += 1;
                    out.push(b);
                }
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                    out.push(b);
                }
                b'\\' => {
                    let Some(esc) = self.bump() else { break };
                    match esc {
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'(' | b')' | b'\\' => out.push(esc),
                        b'\r' => {
                            // Line continuation; \r\n counts as one break.
                            if self.peek() == Some(b'\n') {
                                self.pos += 1;
                            }
                        }
                        b'\n' => {}
                        b'0'..=b'7' => {
                            let mut v = (esc - b'0') as u32;
                            for _ in 0..2 {
                                match self.peek() {
                                    Some(d @ b'0'..=b'7') => {
                                        v = v * 8 + (d - b'0') as u32;
                                        self.pos += 1;
                                    }
                                    _ => break,
                                }
                            }
                            out.push((v & 0xff) as u8);
                        }
                        other => out.push(other),
                    }
                }
                _ => out.push(b),
            }
        }
        out
    }

    /// Parses a hex string, positioned after the `<`.
    pub fn read_hex_string(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        let mut hi: Option<u8> = None;
        while let Some(b) = self.bump() {
            if b == b'>' {
                break;
            }
            let Some(v) = hex_val(b) else { continue };
            match hi.take() {
                Some(h) => out.push(h * 16 + v),
                None => hi = Some(v),
            }
        }
        // An odd final digit is padded with zero.
        if let Some(h) = hi {
            out.push(h * 16);
        }
        out
    }

    /// Parses a number token from raw bytes.
    pub fn parse_number(tok: &[u8]) -> Option<Object> {
        let s = std::str::from_utf8(tok).ok()?;
        if s.is_empty() {
            return None;
        }
        if !s.contains(['.', 'e', 'E']) {
            if let Ok(i) = s.parse::<i64>() {
                return Some(Object::Int(i));
            }
        }
        // Real numbers in the wild include forms like `.5`, `4.`, `-.002` and `--0` (broken
        // producers); parse leniently.
        parse_real(s).map(Object::Real)
    }

    /// Parses one object. Returns `None` at a delimiter that cannot start an object (`]`, `>>`,
    /// `)`, `}`) or at EOF, without consuming it.
    ///
    /// Indirect references (`n g R`) are recognised here with backtracking, so callers see a
    /// single [`Object::Ref`].
    pub fn read_object(&mut self) -> Option<Object> {
        self.skip_ws();
        let b = self.peek()?;
        match b {
            b'/' => {
                self.pos += 1;
                Some(Object::Name(self.read_name()))
            }
            b'(' => {
                self.pos += 1;
                Some(Object::String(self.read_literal_string()))
            }
            b'<' => {
                if self.data.get(self.pos + 1) == Some(&b'<') {
                    self.pos += 2;
                    self.read_dict_body()
                } else {
                    self.pos += 1;
                    Some(Object::String(self.read_hex_string()))
                }
            }
            b'[' => {
                self.pos += 1;
                let mut items = Vec::new();
                loop {
                    self.skip_ws();
                    if self.peek() == Some(b']') {
                        self.pos += 1;
                        break;
                    }
                    match self.read_object() {
                        Some(o) => items.push(o),
                        // Malformed: skip one byte so a bad token cannot loop forever.
                        None => {
                            if self.bump().is_none() {
                                break;
                            }
                        }
                    }
                }
                Some(Object::Array(items))
            }
            b']' | b'>' | b')' | b'}' => None,
            b'{' => {
                // PostScript-function procedure braces; not valid at object level. Skip.
                self.pos += 1;
                self.read_object()
            }
            _ => {
                let start = self.pos;
                let tok = self.read_regular();
                if tok.is_empty() {
                    // A stray delimiter we do not understand; consume so callers make progress.
                    self.pos += 1;
                    return self.read_object();
                }
                match tok {
                    b"true" => Some(Object::Bool(true)),
                    b"false" => Some(Object::Bool(false)),
                    b"null" => Some(Object::Null),
                    _ => {
                        if let Some(num) = Self::parse_number(tok) {
                            if let Object::Int(n) = num {
                                // Try `n g R`.
                                let mark = self.pos;
                                self.skip_ws();
                                let gen_tok = self.read_regular().to_vec();
                                if let Some(Object::Int(g)) = Self::parse_number(&gen_tok) {
                                    self.skip_ws();
                                    if self.eat_keyword(b"R")
                                        && n >= 0
                                        && (0..=u32::MAX as i64).contains(&n)
                                        && (0..=u16::MAX as i64).contains(&g)
                                    {
                                        return Some(Object::Ref(ObjRef::new(n as u32, g as u16)));
                                    }
                                }
                                self.pos = mark;
                            }
                            Some(num)
                        } else {
                            // An unknown keyword: report as a name-like token so the caller can
                            // decide (content streams treat these as operators). At object level
                            // it is malformed; rewind so `eat_keyword` callers can see it.
                            self.pos = start;
                            None
                        }
                    }
                }
            }
        }
    }

    /// Parses dictionary entries, positioned after `<<`. Consumes the closing `>>`.
    fn read_dict_body(&mut self) -> Option<Object> {
        let mut dict = Dict::new();
        loop {
            self.skip_ws();
            match self.peek() {
                Some(b'>') => {
                    if self.data.get(self.pos + 1) == Some(&b'>') {
                        self.pos += 2;
                    } else {
                        self.pos += 1;
                    }
                    break;
                }
                Some(b'/') => {
                    self.pos += 1;
                    let key = self.read_name();
                    match self.read_object() {
                        Some(value) => dict.insert(key, value),
                        None => {
                            // `/Key >>`: a key with no value; drop the key.
                        }
                    }
                }
                Some(_) => {
                    // Garbage between entries; skip a byte to make progress.
                    self.pos += 1;
                }
                None => break,
            }
        }
        Some(Object::Dict(dict))
    }

    /// Parses `<< ... >> stream ... endstream` bodies given a just-parsed dict, using `length`
    /// when it is a plain integer and scanning for `endstream` otherwise.
    pub fn read_stream_data(&mut self, dict: Dict, length: Option<usize>) -> Object {
        // `stream` must be followed by CRLF or LF.
        match self.peek() {
            Some(b'\r') => {
                self.pos += 1;
                if self.peek() == Some(b'\n') {
                    self.pos += 1;
                }
            }
            Some(b'\n') => self.pos += 1,
            _ => {}
        }
        let start = self.pos;

        let end = match length {
            Some(len) if start + len <= self.data.len() && {
                // Trust /Length only when `endstream` actually follows it (after optional EOL);
                // broken producers write lengths that are off by the EOL convention or worse.
                let mut c = Cursor::at(self.data, start + len);
                c.skip_ws();
                c.eat_keyword(b"endstream")
            } =>
            {
                start + len
            }
            _ => {
                // Scan for the first `endstream`, backing off trailing EOL bytes.
                match find(self.data, start, b"endstream") {
                    Some(mut e) => {
                        if e > start && self.data[e - 1] == b'\n' {
                            e -= 1;
                        }
                        if e > start && self.data[e - 1] == b'\r' {
                            e -= 1;
                        }
                        e
                    }
                    None => self.data.len(),
                }
            }
        };

        let raw: std::sync::Arc<[u8]> = self.data[start..end].into();
        // Position after `endstream` for the caller.
        let mut c = Cursor::at(self.data, end);
        c.skip_ws();
        c.eat_keyword(b"endstream");
        self.pos = c.pos;
        Object::Stream(Stream { dict, raw })
    }
}

pub fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Lenient real-number parse: tolerates multiple signs and a lone `.`.
fn parse_real(s: &str) -> Option<f64> {
    if let Ok(v) = s.parse::<f64>() {
        return v.is_finite().then_some(v);
    }
    let cleaned: String = {
        let mut out = String::new();
        let mut seen_digit_or_dot = false;
        for (i, ch) in s.chars().enumerate() {
            match ch {
                '-' | '+' if i == 0 => out.push(ch),
                '-' | '+' if !seen_digit_or_dot => {} // `--0.5`: drop extra signs
                '0'..='9' | '.' => {
                    seen_digit_or_dot = true;
                    out.push(ch);
                }
                _ => return None,
            }
        }
        out
    };
    match cleaned.as_str() {
        "" | "-" | "+" | "." | "-." | "+." => Some(0.0),
        c => c.parse::<f64>().ok().filter(|v| v.is_finite()),
    }
}

/// Finds `needle` in `data` at or after `from`.
pub fn find(data: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || from >= data.len() {
        return None;
    }
    data[from..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| p + from)
}

/// Finds the last occurrence of `needle` in `data`.
pub fn rfind(data: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > data.len() {
        return None;
    }
    data.windows(needle.len()).rposition(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Object {
        Cursor::new(s.as_bytes()).read_object().unwrap()
    }

    #[test]
    fn scalars() {
        assert_eq!(parse("true"), Object::Bool(true));
        assert_eq!(parse("null"), Object::Null);
        assert_eq!(parse("42"), Object::Int(42));
        assert_eq!(parse("-3"), Object::Int(-3));
        assert_eq!(parse("+17"), Object::Int(17));
        assert_eq!(parse(".5"), Object::Real(0.5));
        assert_eq!(parse("4."), Object::Real(4.0));
        // Broken double-sign forms keep the leading sign and drop the rest.
        assert_eq!(parse("--0.5"), Object::Real(-0.5));
    }

    #[test]
    fn names_with_escapes() {
        assert_eq!(parse("/Name1"), Object::Name("Name1".into()));
        assert_eq!(parse("/A#20B"), Object::Name("A B".into()));
        assert_eq!(parse("/"), Object::Name(String::new()));
    }

    #[test]
    fn strings() {
        assert_eq!(parse("(hello)"), Object::String(b"hello".to_vec()));
        assert_eq!(parse("(a(b)c)"), Object::String(b"a(b)c".to_vec()));
        assert_eq!(parse(r"(a\)b)"), Object::String(b"a)b".to_vec()));
        assert_eq!(parse(r"(\101\12)"), Object::String(b"A\n".to_vec()));
        assert_eq!(parse("<48656C6C6F>"), Object::String(b"Hello".to_vec()));
        assert_eq!(parse("<48 65 6c>"), Object::String(b"Hel".to_vec()));
        assert_eq!(parse("<484>"), Object::String(b"H@".to_vec()));
    }

    #[test]
    fn refs_and_arrays() {
        assert_eq!(parse("12 0 R"), Object::Ref(ObjRef::new(12, 0)));
        let arr = parse("[1 2 0 R /X (s)]");
        let Object::Array(items) = arr else { panic!() };
        assert_eq!(items.len(), 4);
        assert_eq!(items[1], Object::Ref(ObjRef::new(2, 0)));
        // `1 2` followed by a name is two integers, not a ref.
        assert_eq!(items[0], Object::Int(1));
    }

    #[test]
    fn dicts_nest() {
        let o = parse("<< /A 1 /B << /C (x) >> /D [1 2] >>");
        let d = o.as_dict().unwrap();
        assert_eq!(d.get("A"), Some(&Object::Int(1)));
        let b = d.get("B").unwrap().as_dict().unwrap();
        assert_eq!(b.get("C"), Some(&Object::String(b"x".to_vec())));
    }

    #[test]
    fn ref_backtracking_does_not_eat_numbers() {
        let mut c = Cursor::new(b"1 2 3");
        assert_eq!(c.read_object(), Some(Object::Int(1)));
        assert_eq!(c.read_object(), Some(Object::Int(2)));
        assert_eq!(c.read_object(), Some(Object::Int(3)));
    }
}
