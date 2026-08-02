//! Reading a Type1 font program's builtin encoding.
//!
//! A `/FontFile` program is PostScript, and only its charstrings are encrypted: the `/Encoding`
//! array sits in the cleartext header ahead of `eexec`, as plain
//!
//! ```text
//! /Encoding 256 array
//! 0 1 255 {1 index exch /.notdef put} for
//! dup 65 /A put
//! dup 97 /a put
//! readonly def
//! ```
//!
//! That matters because a symbolic simple font is entitled to omit `/Encoding` from its PDF font
//! dictionary entirely, and TeX's Computer Modern fonts do exactly that — `/Flags 4`, no
//! `/Encoding`, no `/ToUnicode`. The program's own table is then the *only* record of what each
//! code means, and without it every glyph on the page extracts as nothing at all.

/// The encoding a Type1 program declares for itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Builtin {
    /// `/Encoding StandardEncoding def` — defer to the standard table.
    Standard,
    /// An explicit array: 256 glyph names, empty where the slot is `.notdef`.
    Custom(Vec<String>),
}

/// The cleartext portion of a Type1 program: everything before `eexec`.
///
/// Also steps over a PFB segment header if one is present. A PDF `/FontFile` is raw PostScript,
/// but the same bytes reach this code from a `.pfb` on disk, where each segment is preceded by
/// `0x80 <type> <little-endian length>`.
fn cleartext(data: &[u8]) -> &[u8] {
    let body = match data {
        [0x80, _, _, _, _, _, rest @ ..] => rest,
        _ => data,
    };
    match find(body, b"eexec") {
        Some(end) => &body[..end],
        None => body,
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn is_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\r' | b'\n' | b'\x0c' | b'\0')
}

/// A PostScript name ends at whitespace or at any delimiter.
fn is_name_end(b: u8) -> bool {
    is_space(b)
        || matches!(
            b,
            b'/' | b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'%'
        )
}

/// Extracts the encoding a Type1 program declares, if it declares one.
pub fn builtin_encoding(data: &[u8]) -> Option<Builtin> {
    let clear = cleartext(data);
    let at = find(clear, b"/Encoding")?;
    let rest = &clear[at + b"/Encoding".len()..];

    let head = rest.iter().position(|b| !is_space(*b))?;
    if rest[head..].starts_with(b"StandardEncoding") {
        return Some(Builtin::Standard);
    }

    // The array ends at the `def` that binds it. Bounding the scan matters: without it a font
    // whose `/Encoding` is `.notdef`-only would go on to collect `dup ... put` pairs out of the
    // Subrs and CharStrings that follow.
    let end = find(rest, b"readonly def")
        .or_else(|| find(rest, b" def"))
        .unwrap_or(rest.len());
    let body = &rest[..end];

    let mut names = vec![String::new(); 256];
    let mut found = false;
    let mut i = 0usize;

    while let Some(offset) = find(&body[i..], b"dup") {
        i += offset + 3;

        // `dup <code> /<name> put`. Anything that does not match that shape is not an encoding
        // entry — the `{1 index exch /.notdef put} for` initialiser has no `dup` and so never
        // reaches here, but a malformed program still must not derail the scan.
        let Some((code, next)) = parse_int(body, i) else {
            continue;
        };
        i = next;
        while i < body.len() && is_space(body[i]) {
            i += 1;
        }
        if i >= body.len() || body[i] != b'/' {
            continue;
        }
        i += 1;
        let start = i;
        while i < body.len() && !is_name_end(body[i]) {
            i += 1;
        }
        let name = &body[start..i];
        while i < body.len() && is_space(body[i]) {
            i += 1;
        }
        if !body[i..].starts_with(b"put") {
            continue;
        }

        if (0..256).contains(&code) && name != b".notdef" && !name.is_empty() {
            if let Ok(name) = std::str::from_utf8(name) {
                names[code as usize] = name.to_owned();
                found = true;
            }
        }
    }

    found.then_some(Builtin::Custom(names))
}

/// Parses a non-negative integer starting at or after `i`, returning it and the position after.
fn parse_int(body: &[u8], mut i: usize) -> Option<(i32, usize)> {
    while i < body.len() && is_space(body[i]) {
        i += 1;
    }
    let start = i;
    while i < body.len() && body[i].is_ascii_digit() {
        i += 1;
    }
    if i == start {
        return None;
    }
    std::str::from_utf8(&body[start..i])
        .ok()?
        .parse()
        .ok()
        .map(|n| (n, i))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The header Computer Modern actually ships, trimmed.
    const CMBX10: &[u8] = b"%!PS-AdobeFont-1.0: CMBX10 003.002\n\
        /FontType 1 def\n\
        /FontName /HELIDL+CMBX10 def\n\
        /Encoding 256 array\n\
        0 1 255 {1 index exch /.notdef put} for\n\
        dup 65 /A put\n\
        dup 97 /a put\n\
        dup 49 /one put\n\
        dup 46 /period put\n\
        readonly def\n\
        currentdict end\n\
        currentfile eexec\n\
        \xcd\xa1\x63\x0b";

    #[test]
    fn reads_the_computer_modern_encoding() {
        let Some(Builtin::Custom(names)) = builtin_encoding(CMBX10) else {
            panic!("expected a custom encoding");
        };
        assert_eq!(names[65], "A");
        assert_eq!(names[97], "a");
        assert_eq!(names[49], "one");
        assert_eq!(names[46], "period");
        // Every slot the program did not name stays empty, including `.notdef` ones.
        assert_eq!(names[66], "");
        assert_eq!(names.len(), 256);
    }

    #[test]
    fn recognises_a_deferral_to_the_standard_table() {
        let src = b"/FontType 1 def\n/Encoding StandardEncoding def\ncurrentfile eexec\n";
        assert_eq!(builtin_encoding(src), Some(Builtin::Standard));
    }

    #[test]
    fn a_program_without_an_encoding_yields_nothing() {
        let src = b"%!PS-AdobeFont-1.0\n/FontType 1 def\ncurrentfile eexec\nbinary";
        assert_eq!(builtin_encoding(src), None);
        // An `/Encoding` naming only `.notdef` carries no information either.
        let empty = b"/Encoding 256 array\n0 1 255 {1 index exch /.notdef put} for\nreadonly def\n";
        assert_eq!(builtin_encoding(empty), None);
    }

    #[test]
    fn the_scan_stops_at_the_binding_def() {
        // `dup 3 /other put` sits past `readonly def`, in what would be the private dictionary.
        let src = b"/Encoding 256 array\ndup 65 /A put\nreadonly def\n\
                    /Private 15 dict dup begin\ndup 3 /other put\n";
        let Some(Builtin::Custom(names)) = builtin_encoding(src) else {
            panic!("expected a custom encoding");
        };
        assert_eq!(names[65], "A");
        assert_eq!(
            names[3], "",
            "entries after the binding def must not be read"
        );
    }

    #[test]
    fn only_the_cleartext_header_is_searched() {
        // An `/Encoding` appearing in the encrypted body is binary noise, not a declaration.
        let src = b"/FontType 1 def\ncurrentfile eexec\n/Encoding 256 array\ndup 65 /A put\n";
        assert_eq!(builtin_encoding(src), None);
    }

    #[test]
    fn a_pfb_segment_header_is_stepped_over() {
        let mut src = vec![0x80, 0x01, 0x00, 0x01, 0x00, 0x00];
        src.extend_from_slice(b"/Encoding 256 array\ndup 65 /A put\nreadonly def\neexec");
        let Some(Builtin::Custom(names)) = builtin_encoding(&src) else {
            panic!("expected a custom encoding");
        };
        assert_eq!(names[65], "A");
    }

    #[test]
    fn out_of_range_and_malformed_entries_are_skipped() {
        let src = b"/Encoding 256 array\n\
                    dup 300 /TooBig put\n\
                    dup /NoCode put\n\
                    dup 66 NotAName put\n\
                    dup 67 /C put\n\
                    readonly def\n";
        let Some(Builtin::Custom(names)) = builtin_encoding(src) else {
            panic!("expected a custom encoding");
        };
        assert_eq!(names[67], "C");
        assert_eq!(names[66], "");
    }
}
