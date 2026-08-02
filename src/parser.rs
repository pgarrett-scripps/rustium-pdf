//! File structure: header, cross-reference tables and streams, object streams, trailer chain,
//! and a brute-force recovery scan for files whose xref is wrong — which is common enough that
//! every serious reader has one.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use crate::crypt::Decryptor;
use crate::error::{Error, Result};
use crate::filters;
use crate::lexer::{find, rfind, Cursor};
use crate::object::{Dict, Object, ObjRef, Stream};

#[derive(Debug, Clone, Copy)]
enum XrefEntry {
    /// Byte offset of `n g obj` from the start of the (header-adjusted) buffer.
    Offset { offset: usize, gen: u16 },
    /// Object lives inside an object stream.
    InStream { stream_num: u32, index: u32 },
}

/// The parsed file: bytes plus the object map. All access is by object number.
///
/// Interior caches are mutex-guarded, so `&PdfFile` is usable from multiple threads — the
/// property pdfium never had.
pub struct PdfFile {
    data: Vec<u8>,
    xref: HashMap<u32, XrefEntry>,
    pub trailer: Dict,
    pub decryptor: Option<Decryptor>,
    /// The /Encrypt dictionary's own ref: its strings are not encrypted.
    encrypt_ref: Option<ObjRef>,
    cache: Mutex<HashMap<u32, Arc<Object>>>,
    /// Decoded object streams, keyed by stream object number.
    objstm_cache: Mutex<HashMap<u32, Arc<Vec<(u32, usize)>>>>,
    objstm_data: Mutex<HashMap<u32, Arc<[u8]>>>,
}

impl PdfFile {
    pub fn load(mut data: Vec<u8>, password: Option<&str>) -> Result<Self> {
        // The header may be preceded by junk; all offsets are then relative to `%PDF`.
        let header = find(&data, 0, b"%PDF-").ok_or_else(|| Error::Parse("no %PDF header".into()))?;
        if header > 0 {
            data.drain(..header);
        }

        let (xref, trailer) = match Self::read_xref_chain(&data) {
            Ok(ok) if ok.1.contains("Root") => ok,
            _ => Self::rebuild_by_scan(&data)?,
        };

        let mut file = Self {
            data,
            xref,
            trailer,
            decryptor: None,
            encrypt_ref: None,
            cache: Mutex::new(HashMap::new()),
            objstm_cache: Mutex::new(HashMap::new()),
            objstm_data: Mutex::new(HashMap::new()),
        };

        // A rebuilt xref can still miss /Root if the catalog hides in an object stream that the
        // scan indexed; that resolves through the normal path below, so only a missing entry is
        // fatal here.
        if !file.trailer.contains("Root") {
            return Err(Error::Parse("trailer has no /Root".into()));
        }

        if let Some(enc) = file.trailer.get("Encrypt").cloned() {
            file.encrypt_ref = enc.as_ref_id();
            let enc_dict = file
                .resolve(&enc)
                .as_dict()
                .cloned()
                .ok_or_else(|| Error::Parse("encrypt entry is not a dictionary".into()))?;
            let ids = file
                .trailer
                .get("ID")
                .map(|o| file.resolve(o))
                .and_then(|o| o.as_array().and_then(|a| a.first().cloned()))
                .and_then(|o| o.as_string().map(<[u8]>::to_vec))
                .unwrap_or_default();
            let resolve = |o: &Object| file.resolve(o);
            file.decryptor = Some(Decryptor::new(&enc_dict, &ids, password, &resolve)?);
            // The cache may hold objects loaded before the decryptor existed (the encrypt dict
            // itself); drop them so nothing encrypted is served stale.
            file.cache.lock().unwrap().clear();
        }

        Ok(file)
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }

    // ---- xref reading ----------------------------------------------------------------------

    fn read_xref_chain(data: &[u8]) -> Result<(HashMap<u32, XrefEntry>, Dict)> {
        let tail_start = data.len().saturating_sub(2048);
        let sx = rfind(&data[tail_start..], b"startxref")
            .map(|p| p + tail_start)
            .ok_or_else(|| Error::Parse("no startxref".into()))?;
        let mut c = Cursor::at(data, sx + b"startxref".len());
        c.skip_ws();
        let offset = match Cursor::parse_number(c.read_regular()) {
            Some(Object::Int(i)) if i >= 0 => i as usize,
            _ => return Err(Error::Parse("bad startxref offset".into())),
        };

        let mut xref = HashMap::new();
        let mut trailer = Dict::new();
        let mut visited = HashSet::new();
        let mut queue = vec![offset];

        while let Some(off) = queue.pop() {
            if off >= data.len() || !visited.insert(off) {
                continue;
            }
            let mut c = Cursor::at(data, off);
            c.skip_ws();
            if c.eat_keyword(b"xref") {
                let section_trailer = Self::read_xref_table(&mut c, &mut xref)?;
                // Hybrid files put newer entries in an /XRefStm; those must win over the table
                // they accompany but lose to entries already seen, which insert-if-absent gives
                // us as long as the stream is processed before /Prev. Push order: Prev first.
                if let Some(prev) = section_trailer.get("Prev").and_then(|o| o.as_int()) {
                    queue.insert(0, prev as usize);
                }
                if let Some(x) = section_trailer.get("XRefStm").and_then(|o| o.as_int()) {
                    queue.push(x as usize);
                }
                merge_trailer(&mut trailer, section_trailer);
            } else {
                let dict = Self::read_xref_stream(data, off, &mut xref)?;
                if let Some(prev) = dict.get("Prev").and_then(|o| o.as_int()) {
                    queue.insert(0, prev as usize);
                }
                merge_trailer(&mut trailer, dict);
            }
        }
        Ok((xref, trailer))
    }

    fn read_xref_table(c: &mut Cursor, xref: &mut HashMap<u32, XrefEntry>) -> Result<Dict> {
        loop {
            c.skip_ws();
            if c.eat_keyword(b"trailer") {
                c.skip_ws();
                let t = c
                    .read_object()
                    .and_then(|o| o.as_dict().cloned())
                    .ok_or_else(|| Error::Parse("xref table without trailer dict".into()))?;
                return Ok(t);
            }
            // Subsection header: `start count`.
            let start = match Cursor::parse_number(c.read_regular()) {
                Some(Object::Int(i)) if i >= 0 => i as u32,
                _ => return Err(Error::Parse("bad xref subsection".into())),
            };
            c.skip_ws();
            let count = match Cursor::parse_number(c.read_regular()) {
                Some(Object::Int(i)) if i >= 0 => i as u32,
                _ => return Err(Error::Parse("bad xref subsection count".into())),
            };
            for i in 0..count {
                c.skip_ws();
                let f1 = Cursor::parse_number(c.read_regular()).and_then(|o| o.as_int());
                c.skip_ws();
                let f2 = Cursor::parse_number(c.read_regular()).and_then(|o| o.as_int());
                c.skip_ws();
                let kind = c.read_regular();
                let (Some(offset), Some(gen)) = (f1, f2) else {
                    return Err(Error::Parse("bad xref entry".into()));
                };
                if kind == b"n" {
                    xref.entry(start + i).or_insert(XrefEntry::Offset {
                        offset: offset as usize,
                        gen: gen.clamp(0, u16::MAX as i64) as u16,
                    });
                }
            }
        }
    }

    fn read_xref_stream(
        data: &[u8],
        off: usize,
        xref: &mut HashMap<u32, XrefEntry>,
    ) -> Result<Dict> {
        let mut c = Cursor::at(data, off);
        c.skip_ws();
        // `n g obj`
        let _num = c.read_regular();
        c.skip_ws();
        let _gen = c.read_regular();
        c.skip_ws();
        if !c.eat_keyword(b"obj") {
            return Err(Error::Parse("startxref points at neither table nor stream".into()));
        }
        let obj = c
            .read_object()
            .ok_or_else(|| Error::Parse("empty xref stream object".into()))?;
        let Object::Dict(dict) = obj else {
            return Err(Error::Parse("xref stream object is not a stream".into()));
        };
        c.skip_ws();
        if !c.eat_keyword(b"stream") {
            return Err(Error::Parse("xref stream without stream body".into()));
        }
        let length = dict
            .get("Length")
            .and_then(|o| o.as_int())
            .map(|l| l.max(0) as usize);
        let Object::Stream(stream) = c.read_stream_data(dict, length) else {
            unreachable!()
        };

        // Xref streams are never encrypted, so plain filter decode is correct here.
        let no_resolve = |o: &Object| o.clone();
        let decoded = filters::decode(&stream.dict, &stream.raw, &no_resolve)?;
        let content = decoded.data;

        let w: Vec<usize> = stream
            .dict
            .get("W")
            .and_then(|o| o.as_array())
            .map(|a| a.iter().filter_map(|o| o.as_int()).map(|i| i.max(0) as usize).collect())
            .unwrap_or_default();
        if w.len() < 3 {
            return Err(Error::Parse("xref stream missing /W".into()));
        }
        let row = w.iter().sum::<usize>();
        if row == 0 {
            return Err(Error::Parse("xref stream /W is all zero".into()));
        }
        let size = stream.dict.get("Size").and_then(|o| o.as_int()).unwrap_or(0);
        let index: Vec<i64> = stream
            .dict
            .get("Index")
            .and_then(|o| o.as_array())
            .map(|a| a.iter().filter_map(|o| o.as_int()).collect())
            .unwrap_or_else(|| vec![0, size]);

        let read_field = |bytes: &[u8]| -> u64 {
            bytes.iter().fold(0u64, |acc, &b| (acc << 8) | b as u64)
        };

        let mut rows = content.chunks_exact(row);
        for pair in index.chunks(2) {
            let [start, count] = pair else { break };
            for i in 0..*count {
                let Some(r) = rows.next() else { break };
                let (t, rest) = r.split_at(w[0]);
                let (f2, f3) = rest.split_at(w[1]);
                // A zero-width type field defaults to type 1.
                let t = if w[0] == 0 { 1 } else { read_field(t) };
                let f2 = read_field(f2);
                let f3 = read_field(&f3[..w[2]]);
                let num = (*start + i) as u32;
                match t {
                    1 => {
                        xref.entry(num).or_insert(XrefEntry::Offset {
                            offset: f2 as usize,
                            gen: f3.min(u16::MAX as u64) as u16,
                        });
                    }
                    2 => {
                        xref.entry(num).or_insert(XrefEntry::InStream {
                            stream_num: f2 as u32,
                            index: f3 as u32,
                        });
                    }
                    _ => {} // 0 = free; anything else per spec is a null reference.
                }
            }
        }
        Ok(stream.dict)
    }

    /// Rebuilds the object map by scanning the whole file for `n g obj`, taking the *last*
    /// definition of each object (later definitions supersede in incremental updates), and
    /// hunting for a trailer or catalog.
    fn rebuild_by_scan(data: &[u8]) -> Result<(HashMap<u32, XrefEntry>, Dict)> {
        let mut xref = HashMap::new();
        let mut objstms = Vec::new();
        let mut pos = 0usize;
        while let Some(hit) = find(data, pos, b"obj") {
            pos = hit + 3;
            // `obj` must be a standalone keyword.
            if data.get(hit + 3).is_some_and(|&b| crate::lexer::is_regular(b)) {
                continue;
            }
            // Walk backwards over ws, generation digits, ws, object digits.
            let mut i = hit;
            let step_back_digits = |mut i: usize| -> Option<(usize, u64)> {
                while i > 0 && crate::lexer::is_whitespace(data[i - 1]) {
                    i -= 1;
                }
                let end = i;
                while i > 0 && data[i - 1].is_ascii_digit() {
                    i -= 1;
                }
                if i == end {
                    return None;
                }
                let v = std::str::from_utf8(&data[i..end]).ok()?.parse::<u64>().ok()?;
                Some((i, v))
            };
            let Some((gi, gen)) = step_back_digits(i) else { continue };
            i = gi;
            let Some((ni, num)) = step_back_digits(i) else { continue };
            if num > u32::MAX as u64 || gen > u16::MAX as u64 {
                continue;
            }
            xref.insert(
                num as u32,
                XrefEntry::Offset {
                    offset: ni,
                    gen: gen as u16,
                },
            );
            // Remember object streams so their members can be indexed afterwards.
            let mut c = Cursor::at(data, hit + 3);
            c.skip_ws();
            if let Some(Object::Dict(d)) = c.clone().read_object() {
                if d.get("Type").and_then(|o| o.as_name()) == Some("ObjStm") {
                    objstms.push(num as u32);
                }
            }
        }
        if xref.is_empty() {
            return Err(Error::Parse("no objects found in scan".into()));
        }

        // Index object-stream members: any member not already defined at top level.
        let probe = PdfFile {
            data: data.to_vec(),
            xref: xref.clone(),
            trailer: Dict::new(),
            decryptor: None,
            encrypt_ref: None,
            cache: Mutex::new(HashMap::new()),
            objstm_cache: Mutex::new(HashMap::new()),
            objstm_data: Mutex::new(HashMap::new()),
        };
        for sn in objstms {
            if let Ok(members) = probe.objstm_index(sn) {
                for (i, (num, _)) in members.iter().enumerate() {
                    xref.entry(*num).or_insert(XrefEntry::InStream {
                        stream_num: sn,
                        index: i as u32,
                    });
                }
            }
        }

        // Prefer the last real trailer dict; otherwise find a /Type /Catalog object.
        let mut trailer = Dict::new();
        let mut tpos = 0usize;
        let mut last_trailer = None;
        while let Some(hit) = find(data, tpos, b"trailer") {
            tpos = hit + 7;
            let mut c = Cursor::at(data, tpos);
            c.skip_ws();
            if let Some(Object::Dict(d)) = c.read_object() {
                last_trailer = Some(d);
            }
        }
        if let Some(t) = last_trailer {
            merge_trailer(&mut trailer, t);
        }
        if !trailer.contains("Root") {
            let probe = PdfFile {
                xref: xref.clone(),
                ..probe
            };
            let mut nums: Vec<u32> = xref.keys().copied().collect();
            nums.sort_unstable();
            for num in nums {
                if let Object::Dict(d) | Object::Stream(Stream { dict: d, .. }) =
                    probe.get(num).as_ref()
                {
                    if d.get("Type").and_then(|o| o.as_name()) == Some("Catalog") {
                        trailer.insert("Root", Object::Ref(ObjRef::new(num, 0)));
                        break;
                    }
                }
            }
        }
        Ok((xref, trailer))
    }

    // ---- object access ---------------------------------------------------------------------

    /// Fetches object `num`, from cache when possible. Returns `Object::Null` for anything
    /// missing or malformed: a dangling reference is data-level damage, not a hard error.
    pub fn get(&self, num: u32) -> Arc<Object> {
        if let Some(hit) = self.cache.lock().unwrap().get(&num) {
            return hit.clone();
        }
        let obj = Arc::new(self.load_object(num, &mut HashSet::new()));
        self.cache.lock().unwrap().insert(num, obj.clone());
        obj
    }

    /// Resolves an object one level: a `Ref` becomes its target, everything else is cloned.
    pub fn resolve(&self, obj: &Object) -> Object {
        let mut current = obj.clone();
        let mut hops = 0;
        while let Object::Ref(r) = current {
            if hops > 32 {
                return Object::Null;
            }
            hops += 1;
            current = (*self.get(r.num)).clone();
        }
        current
    }

    fn load_object(&self, num: u32, in_flight: &mut HashSet<u32>) -> Object {
        if !in_flight.insert(num) {
            return Object::Null; // Reference cycle in /Length or an object stream.
        }
        let entry = match self.xref.get(&num) {
            Some(e) => *e,
            None => return Object::Null,
        };
        let obj = match entry {
            XrefEntry::Offset { offset, gen } => self.load_at_offset(num, gen, offset, in_flight),
            XrefEntry::InStream { stream_num, index } => {
                self.load_from_objstm(stream_num, index, num, in_flight)
            }
        };
        in_flight.remove(&num);
        obj
    }

    fn load_at_offset(
        &self,
        num: u32,
        gen: u16,
        offset: usize,
        in_flight: &mut HashSet<u32>,
    ) -> Object {
        if offset >= self.data.len() {
            return Object::Null;
        }
        let mut c = Cursor::at(&self.data, offset);
        c.skip_ws();
        let n_tok = c.read_regular().to_vec();
        c.skip_ws();
        let _g = c.read_regular();
        c.skip_ws();
        if !c.eat_keyword(b"obj") {
            return Object::Null;
        }
        // An xref that points at the wrong object is a damaged file; serving the wrong object
        // corrupts everything downstream, so verify the number.
        match Cursor::parse_number(&n_tok) {
            Some(Object::Int(n)) if n == num as i64 => {}
            _ => return Object::Null,
        }

        let Some(mut obj) = c.read_object() else {
            return Object::Null;
        };

        if let Object::Dict(dict) = obj {
            c.skip_ws();
            if c.eat_keyword(b"stream") {
                let length = dict.get("Length").and_then(|len| match len {
                    Object::Int(i) => Some((*i).max(0) as usize),
                    Object::Ref(r) => self
                        .load_object(r.num, in_flight)
                        .as_int()
                        .map(|i| i.max(0) as usize),
                    _ => None,
                });
                obj = c.read_stream_data(dict, length);
            } else {
                obj = Object::Dict(dict);
            }
        }

        self.decrypt_in_place(&mut obj, num, gen);
        obj
    }

    fn decrypt_in_place(&self, obj: &mut Object, num: u32, gen: u16) {
        let Some(dec) = &self.decryptor else { return };
        if self.encrypt_ref.is_some_and(|r| r.num == num) {
            return;
        }
        fn walk(obj: &mut Object, dec: &Decryptor, num: u32, gen: u16) {
            match obj {
                Object::String(s) => *s = dec.decrypt_string(num, gen, s),
                Object::Array(items) => {
                    for i in items {
                        walk(i, dec, num, gen);
                    }
                }
                Object::Dict(d) => {
                    for v in d.0.values_mut() {
                        walk(v, dec, num, gen);
                    }
                }
                Object::Stream(s) => {
                    for v in s.dict.0.values_mut() {
                        walk(v, dec, num, gen);
                    }
                    // XRef streams are never encrypted; /Metadata is exempt when
                    // EncryptMetadata is false, which the decryptor knows.
                    let ty = s.dict.get("Type").and_then(|o| o.as_name());
                    if ty != Some("XRef") && dec.stream_needs_decrypt(ty) {
                        s.raw = dec.decrypt_stream(num, gen, &s.raw).into();
                    }
                }
                _ => {}
            }
        }
        walk(obj, dec, num, gen);
    }

    // ---- object streams --------------------------------------------------------------------

    /// The `(object number, byte offset)` index of an object stream's members.
    fn objstm_index(&self, stream_num: u32) -> Result<Arc<Vec<(u32, usize)>>> {
        if let Some(hit) = self.objstm_cache.lock().unwrap().get(&stream_num) {
            return Ok(hit.clone());
        }
        let container = self.get(stream_num);
        let Object::Stream(stream) = container.as_ref() else {
            return Err(Error::Parse(format!("object stream {stream_num} missing")));
        };
        let resolve = |o: &Object| self.resolve(o);
        let decoded = filters::decode(&stream.dict, &stream.raw, &resolve)?;
        let data: Arc<[u8]> = decoded.data.into();

        let n = stream
            .dict
            .get("N")
            .map(|o| self.resolve(o))
            .and_then(|o| o.as_int())
            .unwrap_or(0)
            .max(0) as usize;
        let first = stream
            .dict
            .get("First")
            .map(|o| self.resolve(o))
            .and_then(|o| o.as_int())
            .unwrap_or(0)
            .max(0) as usize;

        let mut pairs = Vec::with_capacity(n);
        let mut c = Cursor::new(&data);
        for _ in 0..n {
            c.skip_ws();
            let num = Cursor::parse_number(c.read_regular()).and_then(|o| o.as_int());
            c.skip_ws();
            let off = Cursor::parse_number(c.read_regular()).and_then(|o| o.as_int());
            let (Some(num), Some(off)) = (num, off) else { break };
            if num >= 0 && off >= 0 {
                pairs.push((num as u32, first + off as usize));
            }
        }

        let pairs = Arc::new(pairs);
        self.objstm_cache
            .lock()
            .unwrap()
            .insert(stream_num, pairs.clone());
        self.objstm_data.lock().unwrap().insert(stream_num, data);
        Ok(pairs)
    }

    fn load_from_objstm(
        &self,
        stream_num: u32,
        index: u32,
        expect_num: u32,
        _in_flight: &mut HashSet<u32>,
    ) -> Object {
        let Ok(pairs) = self.objstm_index(stream_num) else {
            return Object::Null;
        };
        // The xref's index is authoritative, but scan-recovered files may have it wrong, so
        // fall back to searching by object number.
        let slot = pairs
            .get(index as usize)
            .filter(|(n, _)| *n == expect_num)
            .or_else(|| pairs.iter().find(|(n, _)| *n == expect_num));
        let Some(&(_, offset)) = slot else {
            return Object::Null;
        };
        let data = self.objstm_data.lock().unwrap().get(&stream_num).cloned();
        let Some(data) = data else {
            return Object::Null;
        };
        let mut c = Cursor::at(&data, offset);
        // Members of an object stream are never streams themselves and are not individually
        // encrypted: the container already was.
        c.read_object().unwrap_or(Object::Null)
    }

    /// Decodes a stream's filter chain (after any decryption, which happened at load).
    pub fn decode_stream(&self, stream: &Stream) -> Result<filters::Decoded> {
        let resolve = |o: &Object| self.resolve(o);
        filters::decode(&stream.dict, &stream.raw, &resolve)
    }
}

/// First trailer wins for the keys that matter; the chain is walked newest-first.
fn merge_trailer(into: &mut Dict, from: Dict) {
    for (k, v) in from.0 {
        into.0.entry(k).or_insert(v);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A minimal one-page PDF with a classic xref table, built by hand.
    pub fn tiny_pdf() -> Vec<u8> {
        let mut out: Vec<u8> = Vec::new();
        let mut offsets = [0usize; 6];
        out.extend_from_slice(b"%PDF-1.4\n");
        let content = b"BT /F1 12 Tf 72 720 Td (Hi) Tj ET";
        let objs: Vec<(usize, String)> = vec![
            (1, "<< /Type /Catalog /Pages 2 0 R >>".into()),
            (2, "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into()),
            (
                3,
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R \
                 /Resources << /Font << /F1 5 0 R >> >> >>"
                    .into(),
            ),
            (
                4,
                format!(
                    "<< /Length {} >>\nstream\n{}\nendstream",
                    content.len(),
                    std::str::from_utf8(content).unwrap()
                ),
            ),
            (
                5,
                "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
            ),
        ];
        for (num, body) in &objs {
            offsets[*num] = out.len();
            out.extend_from_slice(format!("{num} 0 obj\n{body}\nendobj\n").as_bytes());
        }
        let xref_at = out.len();
        out.extend_from_slice(b"xref\n0 6\n0000000000 65535 f \n");
        for num in 1..6 {
            out.extend_from_slice(format!("{:010} 00000 n \n", offsets[num]).as_bytes());
        }
        out.extend_from_slice(b"trailer\n<< /Size 6 /Root 1 0 R >>\n");
        out.extend_from_slice(format!("startxref\n{xref_at}\n%%EOF\n").as_bytes());
        out
    }

    #[test]
    fn loads_classic_xref() {
        let file = PdfFile::load(tiny_pdf(), None).unwrap();
        let root = file.trailer.get("Root").unwrap().as_ref_id().unwrap();
        let cat = file.get(root.num);
        assert_eq!(
            cat.as_dict().unwrap().get("Type").unwrap().as_name(),
            Some("Catalog")
        );
        let content = file.get(4);
        let s = content.as_stream().unwrap();
        let d = file.decode_stream(s).unwrap();
        assert!(d.data.starts_with(b"BT"));
    }

    #[test]
    fn survives_broken_startxref() {
        let mut pdf = tiny_pdf();
        // Corrupt the startxref offset.
        let sx = rfind(&pdf, b"startxref").unwrap();
        pdf[sx + 10] = b'9';
        pdf[sx + 11] = b'9';
        let file = PdfFile::load(pdf, None).unwrap();
        assert!(file.trailer.contains("Root"));
        assert!(file.get(3).as_dict().is_some());
    }

    #[test]
    fn survives_junk_before_header() {
        let mut pdf = b"GARBAGE BYTES ".to_vec();
        pdf.extend_from_slice(&tiny_pdf());
        let file = PdfFile::load(pdf, None).unwrap();
        assert!(file.get(1).as_dict().is_some());
    }

    #[test]
    fn wrong_length_falls_back_to_endstream_scan() {
        let pdf = tiny_pdf();
        let s = String::from_utf8(pdf).unwrap().replace("/Length 33", "/Length 4");
        let file = PdfFile::load(s.into_bytes(), None).unwrap();
        let content = file.get(4);
        let stream = content.as_stream().unwrap();
        assert!(stream.raw.starts_with(b"BT"));
        assert!(stream.raw.ends_with(b"ET"));
    }

    #[test]
    fn missing_object_is_null() {
        let file = PdfFile::load(tiny_pdf(), None).unwrap();
        assert!(file.get(99).is_null());
        assert_eq!(file.resolve(&Object::Ref(ObjRef::new(99, 0))), Object::Null);
    }
}
