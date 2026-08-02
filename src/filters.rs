//! Stream filters.
//!
//! Content and font streams decode fully here. Image-only filters (DCT, JPX, CCITT, JBIG2) are
//! passed through untouched with the filter name reported, because the pixel decoders live in
//! the renderer and callers extracting structure never need the pixels.

use crate::error::{Error, Result};
use crate::object::{Dict, Object};

/// The result of running a stream's filter chain.
pub struct Decoded {
    pub data: Vec<u8>,
    /// `Some(name)` when the chain ended at an image codec the byte-level filters do not
    /// decode; `data` is then that codec's compressed payload.
    pub image_filter: Option<String>,
}

/// Applies a stream's `/Filter` chain to `data`.
///
/// `resolve` maps indirect objects to direct ones, since `/Filter`, `/DecodeParms` and their
/// members may all be references.
pub fn decode(dict: &Dict, data: &[u8], resolve: &dyn Fn(&Object) -> Object) -> Result<Decoded> {
    let filters = filter_names(dict, resolve);
    let parms = decode_parms(dict, filters.len(), resolve);

    let mut data = data.to_vec();
    for (i, name) in filters.iter().enumerate() {
        let parm = parms.get(i).cloned().unwrap_or_default();
        match name.as_str() {
            "FlateDecode" | "Fl" => {
                data = apply_predictor(&parm, inflate(&data)?, resolve)?;
            }
            "LZWDecode" | "LZW" => {
                let early = parm
                    .get("EarlyChange")
                    .map(resolve)
                    .and_then(|o| o.as_int())
                    .unwrap_or(1)
                    != 0;
                data = apply_predictor(&parm, lzw_decode(&data, early), resolve)?;
            }
            "ASCIIHexDecode" | "AHx" => data = ascii_hex_decode(&data),
            "ASCII85Decode" | "A85" => data = ascii85_decode(&data),
            "RunLengthDecode" | "RL" => data = run_length_decode(&data),
            "Crypt" => {
                // Identity crypt filters appear on already-decrypted streams; anything else was
                // handled at the document layer before filters ran.
            }
            "DCTDecode" | "DCT" | "JPXDecode" | "CCITTFaxDecode" | "CCF" | "JBIG2Decode" => {
                return Ok(Decoded {
                    data,
                    image_filter: Some(name.clone()),
                });
            }
            other => {
                return Err(Error::Parse(format!("unknown filter {other}")));
            }
        }
    }
    Ok(Decoded {
        data,
        image_filter: None,
    })
}

fn filter_names(dict: &Dict, resolve: &dyn Fn(&Object) -> Object) -> Vec<String> {
    let entry = dict
        .get("Filter")
        .or_else(|| dict.get("F").filter(|o| !matches!(o, Object::String(_))));
    let Some(entry) = entry else {
        return Vec::new();
    };
    match resolve(entry) {
        Object::Name(n) => vec![n],
        Object::Array(items) => items
            .iter()
            .filter_map(|o| resolve(o).as_name().map(str::to_owned))
            .collect(),
        _ => Vec::new(),
    }
}

fn decode_parms(dict: &Dict, n: usize, resolve: &dyn Fn(&Object) -> Object) -> Vec<Dict> {
    let entry = dict.get("DecodeParms").or_else(|| dict.get("DP"));
    let Some(entry) = entry else {
        return vec![Dict::new(); n];
    };
    match resolve(entry) {
        Object::Dict(d) => {
            let mut v = vec![Dict::new(); n];
            if let Some(slot) = v.first_mut() {
                *slot = d;
            }
            v
        }
        Object::Array(items) => (0..n)
            .map(|i| {
                items
                    .get(i)
                    .map(resolve)
                    .and_then(|o| o.as_dict().cloned())
                    .unwrap_or_default()
            })
            .collect(),
        _ => vec![Dict::new(); n],
    }
}

/// Inflates zlib or raw deflate data, keeping whatever decoded cleanly when the tail is corrupt.
///
/// Truncated and trailing-garbage Flate streams are common enough in the wild that failing hard
/// would reject otherwise fine documents.
fn inflate(data: &[u8]) -> Result<Vec<u8>> {
    use flate2::{Decompress, FlushDecompress, Status};

    // Skip leading whitespace some producers leave before the zlib header.
    let start = data
        .iter()
        .position(|&b| !b.is_ascii_whitespace())
        .unwrap_or(0);
    let data = &data[start..];

    let zlib = data.first().is_some_and(|&b| b & 0x0f == 8);
    let mut inflater = Decompress::new(zlib);
    let mut out = Vec::with_capacity(data.len().saturating_mul(4).min(1 << 20));
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let before_in = inflater.total_in();
        let before_out = inflater.total_out();
        let status = inflater.decompress(
            &data[inflater.total_in() as usize..],
            &mut buf,
            FlushDecompress::None,
        );
        let produced = (inflater.total_out() - before_out) as usize;
        out.extend_from_slice(&buf[..produced]);
        match status {
            Ok(Status::StreamEnd) => break,
            Ok(_) => {
                let consumed = inflater.total_in() - before_in;
                if consumed == 0 && produced == 0 {
                    break; // No progress: truncated stream.
                }
            }
            Err(_) => break, // Corrupt tail: keep what we have.
        }
    }
    if out.is_empty() && !data.is_empty() {
        return Err(Error::Parse("flate stream decoded to nothing".into()));
    }
    Ok(out)
}

fn apply_predictor(
    parm: &Dict,
    data: Vec<u8>,
    resolve: &dyn Fn(&Object) -> Object,
) -> Result<Vec<u8>> {
    let get = |k: &str, default: i64| {
        parm.get(k)
            .map(resolve)
            .and_then(|o| o.as_int())
            .unwrap_or(default)
    };
    let predictor = get("Predictor", 1);
    if predictor <= 1 {
        return Ok(data);
    }
    let colors = get("Colors", 1).max(1) as usize;
    let bpc = get("BitsPerComponent", 8).max(1) as usize;
    let columns = get("Columns", 1).max(1) as usize;
    let bpp = (colors * bpc).div_ceil(8); // bytes per pixel, min 1
    let row_len = (colors * bpc * columns).div_ceil(8);

    if predictor == 2 {
        return Ok(tiff_predictor(data, colors, bpc, columns));
    }

    // PNG predictors: each row is prefixed with its filter type.
    let stride = row_len + 1;
    let rows = data.len() / stride;
    let mut out = Vec::with_capacity(rows * row_len);
    let mut prev = vec![0u8; row_len];
    for r in 0..rows {
        let row = &data[r * stride..(r + 1) * stride];
        let ft = row[0];
        let mut cur = row[1..].to_vec();
        match ft {
            0 => {}
            1 => {
                for i in bpp..row_len {
                    cur[i] = cur[i].wrapping_add(cur[i - bpp]);
                }
            }
            2 => {
                for i in 0..row_len {
                    cur[i] = cur[i].wrapping_add(prev[i]);
                }
            }
            3 => {
                for i in 0..row_len {
                    let left = if i >= bpp { cur[i - bpp] as u32 } else { 0 };
                    cur[i] = cur[i].wrapping_add(((left + prev[i] as u32) / 2) as u8);
                }
            }
            4 => {
                for i in 0..row_len {
                    let a = if i >= bpp { cur[i - bpp] as i32 } else { 0 };
                    let b = prev[i] as i32;
                    let c = if i >= bpp { prev[i - bpp] as i32 } else { 0 };
                    let p = a + b - c;
                    let (pa, pb, pc) = ((p - a).abs(), (p - b).abs(), (p - c).abs());
                    let pred = if pa <= pb && pa <= pc {
                        a
                    } else if pb <= pc {
                        b
                    } else {
                        c
                    };
                    cur[i] = cur[i].wrapping_add(pred as u8);
                }
            }
            _ => return Err(Error::Parse(format!("png predictor row type {ft}"))),
        }
        out.extend_from_slice(&cur);
        prev = cur;
    }
    Ok(out)
}

fn tiff_predictor(mut data: Vec<u8>, colors: usize, bpc: usize, columns: usize) -> Vec<u8> {
    if bpc != 8 {
        // Sub-byte TIFF prediction is vanishingly rare; return as-is rather than corrupt.
        return data;
    }
    let row_len = colors * columns;
    for row in data.chunks_mut(row_len) {
        for i in colors..row.len() {
            row[i] = row[i].wrapping_add(row[i - colors]);
        }
    }
    data
}

fn lzw_decode(data: &[u8], early_change: bool) -> Vec<u8> {
    const CLEAR: u16 = 256;
    const EOD: u16 = 257;

    let mut out = Vec::with_capacity(data.len() * 3);
    let mut table: Vec<Vec<u8>> = Vec::new();
    let reset = |table: &mut Vec<Vec<u8>>| {
        table.clear();
        for b in 0u16..256 {
            table.push(vec![b as u8]);
        }
        table.push(Vec::new()); // 256: clear
        table.push(Vec::new()); // 257: eod
    };
    reset(&mut table);

    let mut bits = 9usize;
    let mut acc = 0u32;
    let mut nbits = 0usize;
    let mut prev: Option<u16> = None;

    for &byte in data {
        acc = (acc << 8) | byte as u32;
        nbits += 8;
        while nbits >= bits {
            let code = ((acc >> (nbits - bits)) & ((1 << bits) - 1)) as u16;
            nbits -= bits;

            if code == CLEAR {
                reset(&mut table);
                bits = 9;
                prev = None;
                continue;
            }
            if code == EOD {
                return out;
            }
            let entry: Vec<u8> = if (code as usize) < table.len() {
                table[code as usize].clone()
            } else if let Some(p) = prev {
                // KwKwK case.
                let mut e = table[p as usize].clone();
                if let Some(&f) = table[p as usize].first() {
                    e.push(f);
                }
                e
            } else {
                return out; // Corrupt.
            };
            out.extend_from_slice(&entry);
            if let Some(p) = prev {
                let mut ne = table[p as usize].clone();
                if let Some(&f) = entry.first() {
                    ne.push(f);
                }
                table.push(ne);
            }
            prev = Some(code);

            let limit = if early_change { 1 } else { 0 };
            if table.len() + limit >= (1 << bits) && bits < 12 {
                bits += 1;
            }
        }
    }
    out
}

fn ascii_hex_decode(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() / 2);
    let mut hi: Option<u8> = None;
    for &b in data {
        if b == b'>' {
            break;
        }
        let Some(v) = crate::lexer::hex_val(b) else {
            continue;
        };
        match hi.take() {
            Some(h) => out.push(h * 16 + v),
            None => hi = Some(v),
        }
    }
    if let Some(h) = hi {
        out.push(h * 16);
    }
    out
}

fn ascii85_decode(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() * 4 / 5);
    let mut group = [0u8; 5];
    let mut n = 0usize;
    let mut i = 0usize;
    // Optional <~ prefix.
    if data.len() >= 2 && &data[0..2] == b"<~" {
        i = 2;
    }
    while i < data.len() {
        let b = data[i];
        i += 1;
        match b {
            b'~' => break,
            b'z' if n == 0 => out.extend_from_slice(&[0, 0, 0, 0]),
            b'!'..=b'u' => {
                group[n] = b - b'!';
                n += 1;
                if n == 5 {
                    let mut v = 0u32;
                    for &g in &group {
                        v = v.wrapping_mul(85).wrapping_add(g as u32);
                    }
                    out.extend_from_slice(&v.to_be_bytes());
                    n = 0;
                }
            }
            _ => {} // Whitespace and junk.
        }
    }
    if n > 0 {
        // Pad with 'u' (84) and keep n-1 bytes.
        for slot in group.iter_mut().skip(n) {
            *slot = 84;
        }
        let mut v = 0u32;
        for &g in &group {
            v = v.wrapping_mul(85).wrapping_add(g as u32);
        }
        out.extend_from_slice(&v.to_be_bytes()[..n - 1]);
    }
    out
}

fn run_length_decode(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() * 2);
    let mut i = 0usize;
    while i < data.len() {
        let len = data[i];
        i += 1;
        match len {
            0..=127 => {
                let take = (len as usize + 1).min(data.len() - i);
                out.extend_from_slice(&data[i..i + take]);
                i += take;
            }
            128 => break,
            _ => {
                if i < data.len() {
                    out.extend(std::iter::repeat_n(data[i], 257 - len as usize));
                    i += 1;
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_resolve(o: &Object) -> Object {
        o.clone()
    }

    #[test]
    fn flate_round_trip() {
        use flate2::write::ZlibEncoder;
        use flate2::Compression;
        use std::io::Write;
        let payload = b"BT /F1 12 Tf (Hello) Tj ET".repeat(100);
        let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
        enc.write_all(&payload).unwrap();
        let compressed = enc.finish().unwrap();

        let mut dict = Dict::new();
        dict.insert("Filter", Object::Name("FlateDecode".into()));
        let d = decode(&dict, &compressed, &no_resolve).unwrap();
        assert_eq!(d.data, payload);
        assert!(d.image_filter.is_none());
    }

    #[test]
    fn truncated_flate_keeps_prefix() {
        use flate2::write::ZlibEncoder;
        use flate2::Compression;
        use std::io::Write;
        let payload = vec![7u8; 100_000];
        let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
        enc.write_all(&payload).unwrap();
        let mut compressed = enc.finish().unwrap();
        compressed.truncate(compressed.len() / 2);

        let mut dict = Dict::new();
        dict.insert("Filter", Object::Name("FlateDecode".into()));
        let d = decode(&dict, &compressed, &no_resolve).unwrap();
        assert!(!d.data.is_empty());
        assert!(d.data.iter().all(|&b| b == 7));
    }

    #[test]
    fn ascii_filters() {
        assert_eq!(ascii_hex_decode(b"48656C6C6F>"), b"Hello");
        assert_eq!(ascii85_decode(b"87cURD_*#T~>"), b"Hello, w");
        assert_eq!(ascii85_decode(b"z~>"), [0, 0, 0, 0]);
    }

    #[test]
    fn run_length() {
        // 2 literal bytes, then 0xFF repeated 4 times (257-253), then EOD.
        assert_eq!(
            run_length_decode(&[1, b'a', b'b', 253, 0xFF, 128]),
            b"ab\xff\xff\xff\xff"
        );
    }

    #[test]
    fn lzw_known_vector() {
        // The example from the PDF spec, 7.4.4.2: decodes to 45 45 45 45 45 65 45 45 45 66.
        let encoded = [0x80, 0x0B, 0x60, 0x50, 0x22, 0x0C, 0x0C, 0x85, 0x01];
        assert_eq!(
            lzw_decode(&encoded, true),
            [45, 45, 45, 45, 45, 65, 45, 45, 45, 66]
        );
    }

    #[test]
    fn png_up_predictor() {
        // Two rows of 3 bytes, filter type 2 (Up).
        let mut dict = Dict::new();
        dict.insert("Filter", Object::Name("FlateDecode".into()));
        let mut parms = Dict::new();
        parms.insert("Predictor", Object::Int(12));
        parms.insert("Columns", Object::Int(3));
        dict.insert("DecodeParms", Object::Dict(parms));

        use flate2::write::ZlibEncoder;
        use flate2::Compression;
        use std::io::Write;
        let raw = [2u8, 1, 2, 3, 2, 1, 1, 1];
        let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
        enc.write_all(&raw).unwrap();
        let compressed = enc.finish().unwrap();

        let d = decode(&dict, &compressed, &no_resolve).unwrap();
        assert_eq!(d.data, [1, 2, 3, 2, 3, 4]);
    }

    #[test]
    fn dct_passes_through() {
        let mut dict = Dict::new();
        dict.insert("Filter", Object::Name("DCTDecode".into()));
        let d = decode(&dict, b"\xff\xd8jpegdata", &no_resolve).unwrap();
        assert_eq!(d.image_filter.as_deref(), Some("DCTDecode"));
        assert_eq!(d.data, b"\xff\xd8jpegdata");
    }
}
