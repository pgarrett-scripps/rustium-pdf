//! CMap parsing: ToUnicode maps and CID encodings.
//!
//! CMap files are PostScript-flavoured, but their payload lines are plain COS objects between
//! `begin.../end...` keywords, so the content-stream tokenizer parses them directly: operands
//! accumulate and arrive attached to the `end...` operator.

use std::collections::HashMap;

use crate::content::ContentParser;
use crate::object::Object;

/// A code space: how many bytes one character code occupies, by range.
#[derive(Debug, Default, Clone)]
pub struct CodeSpace {
    /// `(low, high, byte length)`, where low/high are the range ends read as big-endian.
    ranges: Vec<(u32, u32, u8)>,
}

impl CodeSpace {
    pub fn identity_two_byte() -> Self {
        Self {
            ranges: vec![(0, 0xFFFF, 2)],
        }
    }

    fn push(&mut self, lo: &[u8], hi: &[u8]) {
        let n = lo.len().clamp(1, 4) as u8;
        self.ranges.push((be(lo), be(hi), n));
    }

    /// Splits `bytes` into character codes. Greedy: at each position the matching range wins;
    /// with no match, one byte is consumed so malformed strings cannot stall.
    pub fn decode(&self, bytes: &[u8]) -> Vec<u32> {
        if self.ranges.is_empty() {
            return bytes.iter().map(|&b| b as u32).collect();
        }
        let mut out = Vec::with_capacity(bytes.len());
        let mut i = 0;
        'outer: while i < bytes.len() {
            for len in 1..=4usize {
                if i + len > bytes.len() {
                    break;
                }
                let code = be(&bytes[i..i + len]);
                if self
                    .ranges
                    .iter()
                    .any(|&(lo, hi, n)| n as usize == len && (lo..=hi).contains(&code))
                {
                    out.push(code);
                    i += len;
                    continue 'outer;
                }
            }
            // No range matched: consume the shortest declared length so we stay aligned.
            let len = self.ranges.iter().map(|r| r.2 as usize).min().unwrap_or(1);
            let len = len.min(bytes.len() - i);
            out.push(be(&bytes[i..i + len]));
            i += len;
        }
        out
    }
}

fn be(bytes: &[u8]) -> u32 {
    bytes
        .iter()
        .take(4)
        .fold(0u32, |acc, &b| (acc << 8) | b as u32)
}

/// A parsed CMap: code → CID and/or code → Unicode.
#[derive(Debug, Default, Clone)]
pub struct CMap {
    pub codespace: CodeSpace,
    cid_singles: HashMap<u32, u32>,
    cid_ranges: Vec<(u32, u32, u32)>,
    uni_singles: HashMap<u32, String>,
    /// `(lo, hi, base)`: base's last UTF-16 unit increments with the code.
    uni_ranges: Vec<(u32, u32, Vec<u16>)>,
    /// `(lo, [dst...])` from the array form of bfrange.
    uni_arrays: Vec<(u32, Vec<String>)>,
    /// Whether the map is (or uses) the identity CID mapping.
    identity_cid: bool,
}

impl CMap {
    pub fn identity() -> Self {
        Self {
            codespace: CodeSpace::identity_two_byte(),
            identity_cid: true,
            ..Default::default()
        }
    }

    pub fn parse(data: &[u8]) -> Self {
        let mut map = CMap::default();
        for op in ContentParser::new(data) {
            match op.operator.as_str() {
                "endcodespacerange" => {
                    for pair in op.operands.chunks(2) {
                        if let [Object::String(lo), Object::String(hi)] = pair {
                            map.codespace.push(lo, hi);
                        }
                    }
                }
                "endbfchar" => {
                    for pair in op.operands.chunks(2) {
                        if let [Object::String(src), dst] = pair {
                            if let Some(s) = unicode_target(dst) {
                                map.uni_singles.insert(be(src), s);
                            }
                        }
                    }
                }
                "endbfrange" => {
                    for triple in op.operands.chunks(3) {
                        let [Object::String(lo), Object::String(hi), dst] = triple else {
                            continue;
                        };
                        let (lo, hi) = (be(lo), be(hi));
                        match dst {
                            Object::String(base) => {
                                map.uni_ranges.push((lo, hi, utf16_units(base)));
                            }
                            Object::Array(items) => {
                                let dsts: Vec<String> =
                                    items.iter().filter_map(unicode_target).collect();
                                map.uni_arrays.push((lo, dsts));
                            }
                            _ => {}
                        }
                    }
                }
                "endcidchar" => {
                    for pair in op.operands.chunks(2) {
                        if let [Object::String(src), Object::Int(cid)] = pair {
                            map.cid_singles.insert(be(src), *cid as u32);
                        }
                    }
                }
                "endcidrange" => {
                    for triple in op.operands.chunks(3) {
                        if let [Object::String(lo), Object::String(hi), Object::Int(cid)] = triple {
                            map.cid_ranges.push((be(lo), be(hi), *cid as u32));
                        }
                    }
                }
                // Only the identity parents matter in practice.
                "usecmap"
                    if op
                        .operands
                        .iter()
                        .any(|o| o.as_name().is_some_and(|n| n.starts_with("Identity"))) =>
                {
                    map.identity_cid = true;
                }
                _ => {}
            }
        }
        if map.codespace.ranges.is_empty() {
            // A ToUnicode map without codespace: infer byte length from the widest key.
            let wide = map.uni_singles.keys().any(|&k| k > 0xFF)
                || map.uni_ranges.iter().any(|&(_, hi, _)| hi > 0xFF);
            map.codespace
                .ranges
                .push(if wide { (0, 0xFFFF, 2) } else { (0, 0xFF, 1) });
        }
        map
    }

    pub fn cid(&self, code: u32) -> u32 {
        if let Some(&cid) = self.cid_singles.get(&code) {
            return cid;
        }
        for &(lo, hi, base) in &self.cid_ranges {
            if (lo..=hi).contains(&code) {
                return base + (code - lo);
            }
        }
        if self.identity_cid {
            code
        } else {
            0
        }
    }

    pub fn unicode(&self, code: u32) -> Option<String> {
        if let Some(s) = self.uni_singles.get(&code) {
            return non_empty(s.clone());
        }
        for (lo, dsts) in &self.uni_arrays {
            if code >= *lo && ((code - lo) as usize) < dsts.len() {
                return non_empty(dsts[(code - lo) as usize].clone());
            }
        }
        for (lo, hi, base) in &self.uni_ranges {
            if (*lo..=*hi).contains(&code) {
                let mut units = base.clone();
                // An empty destination has no trailing unit to increment, so there is no
                // mapping to build.
                let last = units.last_mut()?;
                *last = last.wrapping_add((code - lo) as u16);
                return non_empty(String::from_utf16_lossy(&units));
            }
        }
        None
    }

    pub fn has_unicode(&self) -> bool {
        !self.uni_singles.is_empty() || !self.uni_ranges.is_empty() || !self.uni_arrays.is_empty()
    }
}

fn non_empty(s: String) -> Option<String> {
    // Some producers map unmappable glyphs to U+0000 or the empty string; that is "no mapping".
    let s: String = s
        .chars()
        .filter(|&c| c != '\0' && c != '\u{FFFD}')
        .collect();
    (!s.is_empty()).then_some(s)
}

fn utf16_units(bytes: &[u8]) -> Vec<u16> {
    bytes
        .chunks(2)
        .map(|c| {
            if c.len() == 2 {
                u16::from_be_bytes([c[0], c[1]])
            } else {
                c[0] as u16
            }
        })
        .collect()
}

fn unicode_target(obj: &Object) -> Option<String> {
    match obj {
        Object::String(bytes) => Some(String::from_utf16_lossy(&utf16_units(bytes))),
        Object::Name(n) => crate::font::encoding::glyph_name_to_unicode(n),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOUNICODE: &[u8] = br#"
/CIDInit /ProcSet findresource begin
12 dict begin
begincmap
1 begincodespacerange
<0000> <FFFF>
endcodespacerange
2 beginbfchar
<0003> <0020>
<001B> <FB00>
endbfchar
2 beginbfrange
<000F> <0018> <0041>
<0020> <0021> [<0058> <00590059>]
endbfrange
endcmap
CMapName currentdict /CMap defineresource pop
end end
"#;

    #[test]
    fn tounicode_parses() {
        let m = CMap::parse(TOUNICODE);
        assert_eq!(m.unicode(0x03).as_deref(), Some(" "));
        assert_eq!(m.unicode(0x1B).as_deref(), Some("ﬀ"));
        assert_eq!(m.unicode(0x0F).as_deref(), Some("A"));
        assert_eq!(m.unicode(0x12).as_deref(), Some("D"));
        assert_eq!(m.unicode(0x20).as_deref(), Some("X"));
        assert_eq!(m.unicode(0x21).as_deref(), Some("YY"));
        assert_eq!(m.unicode(0x99), None);
    }

    #[test]
    fn codespace_decodes_two_byte() {
        let m = CMap::parse(TOUNICODE);
        assert_eq!(
            m.codespace.decode(&[0x00, 0x0F, 0x00, 0x12]),
            vec![0x0F, 0x12]
        );
    }

    #[test]
    fn mixed_width_codespace() {
        let mut cs = CodeSpace::default();
        cs.push(&[0x00], &[0x80]);
        cs.push(&[0x81, 0x40], &[0xFE, 0xFE]);
        assert_eq!(
            cs.decode(&[0x41, 0x81, 0x41, 0x42]),
            vec![0x41, 0x8141, 0x42]
        );
    }

    #[test]
    fn cid_ranges() {
        let data = br#"
1 begincodespacerange
<0000> <FFFF>
endcodespacerange
1 begincidrange
<0010> <001F> 100
endcidrange
1 begincidchar
<0005> 7
endcidchar
"#;
        let m = CMap::parse(data);
        assert_eq!(m.cid(0x15), 105);
        assert_eq!(m.cid(0x05), 7);
        assert_eq!(m.cid(0x99), 0);
    }

    #[test]
    fn identity() {
        let m = CMap::identity();
        assert_eq!(m.cid(0x1234), 0x1234);
        assert_eq!(
            m.codespace.decode(&[0x12, 0x34, 0x00, 0x41]),
            vec![0x1234, 0x41]
        );
    }
}
