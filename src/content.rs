//! Content-stream tokenization: a flat sequence of `operands... operator`, with inline images
//! (`BI ... ID ... EI`) folded into single operations.

use crate::lexer::{is_regular, is_whitespace, Cursor};
use crate::object::{Dict, Object};

pub struct Operation {
    pub operator: String,
    pub operands: Vec<Object>,
    /// Payload of an inline image (`ID` data); present only when `operator == "BI"`, in which
    /// case `operands` holds a single [`Object::Dict`] with the image parameters.
    pub inline_data: Option<Vec<u8>>,
}

pub struct ContentParser<'a> {
    cursor: Cursor<'a>,
    pending: Vec<Object>,
}

impl<'a> ContentParser<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self {
            cursor: Cursor::new(data),
            pending: Vec::new(),
        }
    }

    /// Parses an inline image, positioned just after `BI`.
    fn read_inline_image(&mut self) -> Operation {
        let mut dict = Dict::new();
        loop {
            self.cursor.skip_ws();
            match self.cursor.peek() {
                Some(b'/') => {
                    self.cursor.pos += 1;
                    let key = self.cursor.read_name();
                    let value = self.cursor.read_object().unwrap_or(Object::Null);
                    dict.insert(expand_inline_key(&key), value);
                }
                Some(_) => {
                    if self.cursor.eat_keyword(b"ID") {
                        break;
                    }
                    // Junk; skip a byte.
                    self.cursor.pos += 1;
                }
                None => break,
            }
        }
        // One whitespace byte separates `ID` from the data.
        if self.cursor.peek().is_some_and(is_whitespace) {
            self.cursor.pos += 1;
        }
        let start = self.cursor.pos;
        let data = self.cursor.data;

        // /L (length) makes the end exact; otherwise scan for whitespace-delimited `EI`.
        let end = dict
            .get("Length")
            .or_else(|| dict.get("L"))
            .and_then(|o| o.as_int())
            .map(|l| (start + l.max(0) as usize).min(data.len()))
            .filter(|&e| {
                let mut c = Cursor::at(data, e);
                c.skip_ws();
                c.eat_keyword(b"EI")
            })
            .unwrap_or_else(|| {
                let mut i = start;
                loop {
                    match crate::lexer::find(data, i, b"EI") {
                        Some(p) => {
                            let before_ok = p == 0 || is_whitespace(data[p - 1]);
                            let after_ok = data.get(p + 2).is_none_or(|&b| !is_regular(b));
                            if before_ok && after_ok {
                                break p.saturating_sub(1).max(start);
                            }
                            i = p + 2;
                        }
                        None => break data.len(),
                    }
                }
            });

        let payload = data[start..end].to_vec();
        let mut c = Cursor::at(data, end);
        c.skip_ws();
        c.eat_keyword(b"EI");
        self.cursor.pos = c.pos;

        Operation {
            operator: "BI".into(),
            operands: vec![Object::Dict(dict)],
            inline_data: Some(payload),
        }
    }
}

impl Iterator for ContentParser<'_> {
    type Item = Operation;

    fn next(&mut self) -> Option<Operation> {
        loop {
            self.cursor.skip_ws();
            if self.cursor.eof() {
                return None;
            }
            // Try an object first; anything the object grammar rejects is an operator keyword.
            let mark = self.cursor.pos;
            if let Some(obj) = self.cursor.read_object() {
                self.pending.push(obj);
                continue;
            }
            self.cursor.pos = mark;
            let tok = self.cursor.read_regular();
            if tok.is_empty() {
                // A stray delimiter (e.g. an unmatched `)`); skip it.
                self.cursor.pos += 1;
                continue;
            }
            if tok == b"BI" {
                self.pending.clear();
                return Some(self.read_inline_image());
            }
            let operator = String::from_utf8_lossy(tok).into_owned();
            let operands = std::mem::take(&mut self.pending);
            return Some(Operation {
                operator,
                operands,
                inline_data: None,
            });
        }
    }
}

/// Inline images abbreviate their dictionary keys; expand to the canonical names.
fn expand_inline_key(key: &str) -> String {
    match key {
        "BPC" => "BitsPerComponent",
        "CS" => "ColorSpace",
        "D" => "Decode",
        "DP" => "DecodeParms",
        "F" => "Filter",
        "H" => "Height",
        "IM" => "ImageMask",
        "I" => "Interpolate",
        "W" => "Width",
        other => other,
    }
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ops(data: &[u8]) -> Vec<Operation> {
        ContentParser::new(data).collect()
    }

    #[test]
    fn simple_text_ops() {
        let v = ops(b"BT /F1 12 Tf 72 720 Td (Hi) Tj ET");
        let names: Vec<&str> = v.iter().map(|o| o.operator.as_str()).collect();
        assert_eq!(names, ["BT", "Tf", "Td", "Tj", "ET"]);
        assert_eq!(v[1].operands[0].as_name(), Some("F1"));
        assert_eq!(v[1].operands[1].as_int(), Some(12));
        assert_eq!(v[3].operands[0].as_string(), Some(&b"Hi"[..]));
    }

    #[test]
    fn tj_array_and_negative_numbers() {
        let v = ops(b"[(A) -120 (B)] TJ");
        assert_eq!(v[0].operator, "TJ");
        let arr = v[0].operands[0].as_array().unwrap();
        assert_eq!(arr.len(), 3);
        assert_eq!(arr[1].as_int(), Some(-120));
    }

    #[test]
    fn quote_operators() {
        let v = ops(b"(x) ' (y) 1 2 \" ");
        assert_eq!(v[0].operator, "'");
        assert_eq!(v[1].operator, "\"");
    }

    #[test]
    fn inline_image() {
        let v = ops(b"q BI /W 2 /H 2 /BPC 8 /CS /G ID \x00\x01\x02\x03 EI Q");
        assert_eq!(v.len(), 3);
        assert_eq!(v[1].operator, "BI");
        let dict = v[1].operands[0].as_dict().unwrap();
        assert_eq!(dict.get("Width").unwrap().as_int(), Some(2));
        assert_eq!(v[1].inline_data.as_deref(), Some(&[0u8, 1, 2, 3][..]));
        assert_eq!(v[2].operator, "Q");
    }

    #[test]
    fn inline_image_with_ei_bytes_in_data() {
        // Data contains `EI` not surrounded by whitespace: must not end there.
        let v = ops(b"BI /W 1 /H 1 /BPC 8 /CS /G ID xEIy EI Q");
        assert_eq!(v[0].inline_data.as_deref(), Some(&b"xEIy"[..]));
    }

    #[test]
    fn operators_survive_junk() {
        let v = ops(b") q 1 0 0 1 5 5 cm Q");
        let names: Vec<&str> = v.iter().map(|o| o.operator.as_str()).collect();
        assert_eq!(names, ["q", "cm", "Q"]);
    }
}
