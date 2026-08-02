//! Base encodings and glyph-name → Unicode mapping.
//!
//! The glyph list is a working subset of the Adobe Glyph List: full Latin-1 coverage, the
//! ligatures, Greek, and the mathematical symbols that appear in scientific documents. Names
//! not present fall through to the `uniXXXX`/`uXXXX(XX)` conventions, then to suffix stripping
//! (`a.sc` → `a`), then to nothing.

/// ASCII slots common to Standard, WinAnsi and MacRoman, code 32..=126.
const ASCII_NAMES: [&str; 95] = [
    "space",
    "exclam",
    "quotedbl",
    "numbersign",
    "dollar",
    "percent",
    "ampersand",
    "quotesingle",
    "parenleft",
    "parenright",
    "asterisk",
    "plus",
    "comma",
    "hyphen",
    "period",
    "slash",
    "zero",
    "one",
    "two",
    "three",
    "four",
    "five",
    "six",
    "seven",
    "eight",
    "nine",
    "colon",
    "semicolon",
    "less",
    "equal",
    "greater",
    "question",
    "at",
    "A",
    "B",
    "C",
    "D",
    "E",
    "F",
    "G",
    "H",
    "I",
    "J",
    "K",
    "L",
    "M",
    "N",
    "O",
    "P",
    "Q",
    "R",
    "S",
    "T",
    "U",
    "V",
    "W",
    "X",
    "Y",
    "Z",
    "bracketleft",
    "backslash",
    "bracketright",
    "asciicircum",
    "underscore",
    "grave",
    "a",
    "b",
    "c",
    "d",
    "e",
    "f",
    "g",
    "h",
    "i",
    "j",
    "k",
    "l",
    "m",
    "n",
    "o",
    "p",
    "q",
    "r",
    "s",
    "t",
    "u",
    "v",
    "w",
    "x",
    "y",
    "z",
    "braceleft",
    "bar",
    "braceright",
    "asciitilde",
];

/// Returns the glyph name for `code` under the named base encoding, or `""`.
pub fn base_encoding_name(encoding: Encoding, code: u8) -> &'static str {
    // Symbol reassigns the ASCII range to Greek and mathematics, so it must be consulted before
    // the shared-ASCII shortcut below rather than after it.
    if encoding == Encoding::Symbol {
        return symbol_name(code);
    }
    if (32..=126).contains(&code) {
        let name = ASCII_NAMES[code as usize - 32];
        // The two slots where StandardEncoding disagrees with the others.
        if encoding == Encoding::Standard {
            if code == 0x27 {
                return "quoteright";
            }
            if code == 0x60 {
                return "quoteleft";
            }
        }
        return name;
    }
    let table: &[(u8, &str)] = match encoding {
        Encoding::Standard => STANDARD_HIGH,
        Encoding::WinAnsi => WINANSI_HIGH,
        Encoding::MacRoman => MACROMAN_HIGH,
        Encoding::Symbol => return symbol_name(code),
    };
    table
        .iter()
        .find(|(c, _)| *c == code)
        .map(|(_, n)| *n)
        .unwrap_or("")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    Standard,
    WinAnsi,
    MacRoman,
    Symbol,
}

const STANDARD_HIGH: &[(u8, &str)] = &[
    (0xA1, "exclamdown"),
    (0xA2, "cent"),
    (0xA3, "sterling"),
    (0xA4, "fraction"),
    (0xA5, "yen"),
    (0xA6, "florin"),
    (0xA7, "section"),
    (0xA8, "currency"),
    (0xA9, "quotesingle"),
    (0xAA, "quotedblleft"),
    (0xAB, "guillemotleft"),
    (0xAC, "guilsinglleft"),
    (0xAD, "guilsinglright"),
    (0xAE, "fi"),
    (0xAF, "fl"),
    (0xB1, "endash"),
    (0xB2, "dagger"),
    (0xB3, "daggerdbl"),
    (0xB4, "periodcentered"),
    (0xB6, "paragraph"),
    (0xB7, "bullet"),
    (0xB8, "quotesinglbase"),
    (0xB9, "quotedblbase"),
    (0xBA, "quotedblright"),
    (0xBB, "guillemotright"),
    (0xBC, "ellipsis"),
    (0xBD, "perthousand"),
    (0xBF, "questiondown"),
    (0xC1, "grave"),
    (0xC2, "acute"),
    (0xC3, "circumflex"),
    (0xC4, "tilde"),
    (0xC5, "macron"),
    (0xC6, "breve"),
    (0xC7, "dotaccent"),
    (0xC8, "dieresis"),
    (0xCA, "ring"),
    (0xCB, "cedilla"),
    (0xCD, "hungarumlaut"),
    (0xCE, "ogonek"),
    (0xCF, "caron"),
    (0xD0, "emdash"),
    (0xE1, "AE"),
    (0xE3, "ordfeminine"),
    (0xF1, "ae"),
    (0xF5, "dotlessi"),
];

const WINANSI_HIGH: &[(u8, &str)] = &[
    (0x80, "Euro"),
    (0x82, "quotesinglbase"),
    (0x83, "florin"),
    (0x84, "quotedblbase"),
    (0x85, "ellipsis"),
    (0x86, "dagger"),
    (0x87, "daggerdbl"),
    (0x88, "circumflex"),
    (0x89, "perthousand"),
    (0x8A, "Scaron"),
    (0x8B, "guilsinglleft"),
    (0x8C, "OE"),
    (0x8E, "Zcaron"),
    (0x91, "quoteleft"),
    (0x92, "quoteright"),
    (0x93, "quotedblleft"),
    (0x94, "quotedblright"),
    (0x95, "bullet"),
    (0x96, "endash"),
    (0x97, "emdash"),
    (0x98, "tilde"),
    (0x99, "trademark"),
    (0x9A, "scaron"),
    (0x9B, "guilsinglright"),
    (0x9C, "oe"),
    (0x9E, "zcaron"),
    (0x9F, "Ydieresis"),
    (0xA1, "exclamdown"),
    (0xA2, "cent"),
    (0xA3, "sterling"),
    (0xA4, "currency"),
    (0xA5, "yen"),
    (0xA6, "brokenbar"),
    (0xA7, "section"),
    (0xA8, "dieresis"),
    (0xA9, "copyright"),
    (0xAA, "ordfeminine"),
    (0xAB, "guillemotleft"),
    (0xAC, "logicalnot"),
    (0xAD, "hyphen"),
    (0xAE, "registered"),
    (0xAF, "macron"),
    (0xB0, "degree"),
    (0xB1, "plusminus"),
    (0xB2, "twosuperior"),
    (0xB3, "threesuperior"),
    (0xB4, "acute"),
    (0xB5, "mu"),
    (0xB6, "paragraph"),
    (0xB7, "periodcentered"),
    (0xB8, "cedilla"),
    (0xB9, "onesuperior"),
    (0xBA, "ordmasculine"),
    (0xBB, "guillemotright"),
    (0xBC, "onequarter"),
    (0xBD, "onehalf"),
    (0xBE, "threequarters"),
    (0xBF, "questiondown"),
    (0xC0, "Agrave"),
    (0xC1, "Aacute"),
    (0xC2, "Acircumflex"),
    (0xC3, "Atilde"),
    (0xC4, "Adieresis"),
    (0xC5, "Aring"),
    (0xC6, "AE"),
    (0xC7, "Ccedilla"),
    (0xC8, "Egrave"),
    (0xC9, "Eacute"),
    (0xCA, "Ecircumflex"),
    (0xCB, "Edieresis"),
    (0xCC, "Igrave"),
    (0xCD, "Iacute"),
    (0xCE, "Icircumflex"),
    (0xCF, "Idieresis"),
    (0xD0, "Eth"),
    (0xD1, "Ntilde"),
    (0xD2, "Ograve"),
    (0xD3, "Oacute"),
    (0xD4, "Ocircumflex"),
    (0xD5, "Otilde"),
    (0xD6, "Odieresis"),
    (0xD7, "multiply"),
    (0xD8, "Oslash"),
    (0xD9, "Ugrave"),
    (0xDA, "Uacute"),
    (0xDB, "Ucircumflex"),
    (0xDC, "Udieresis"),
    (0xDD, "Yacute"),
    (0xDE, "Thorn"),
    (0xDF, "germandbls"),
    (0xE0, "agrave"),
    (0xE1, "aacute"),
    (0xE2, "acircumflex"),
    (0xE3, "atilde"),
    (0xE4, "adieresis"),
    (0xE5, "aring"),
    (0xE6, "ae"),
    (0xE7, "ccedilla"),
    (0xE8, "egrave"),
    (0xE9, "eacute"),
    (0xEA, "ecircumflex"),
    (0xEB, "edieresis"),
    (0xEC, "igrave"),
    (0xED, "iacute"),
    (0xEE, "icircumflex"),
    (0xEF, "idieresis"),
    (0xF0, "eth"),
    (0xF1, "ntilde"),
    (0xF2, "ograve"),
    (0xF3, "oacute"),
    (0xF4, "ocircumflex"),
    (0xF5, "otilde"),
    (0xF6, "odieresis"),
    (0xF7, "divide"),
    (0xF8, "oslash"),
    (0xF9, "ugrave"),
    (0xFA, "uacute"),
    (0xFB, "ucircumflex"),
    (0xFC, "udieresis"),
    (0xFD, "yacute"),
    (0xFE, "thorn"),
    (0xFF, "ydieresis"),
];

const MACROMAN_HIGH: &[(u8, &str)] = &[
    (0x80, "Adieresis"),
    (0x81, "Aring"),
    (0x82, "Ccedilla"),
    (0x83, "Eacute"),
    (0x84, "Ntilde"),
    (0x85, "Odieresis"),
    (0x86, "Udieresis"),
    (0x87, "aacute"),
    (0x88, "agrave"),
    (0x89, "acircumflex"),
    (0x8A, "adieresis"),
    (0x8B, "atilde"),
    (0x8C, "aring"),
    (0x8D, "ccedilla"),
    (0x8E, "eacute"),
    (0x8F, "egrave"),
    (0x90, "ecircumflex"),
    (0x91, "edieresis"),
    (0x92, "iacute"),
    (0x93, "igrave"),
    (0x94, "icircumflex"),
    (0x95, "idieresis"),
    (0x96, "ntilde"),
    (0x97, "oacute"),
    (0x98, "ograve"),
    (0x99, "ocircumflex"),
    (0x9A, "odieresis"),
    (0x9B, "otilde"),
    (0x9C, "uacute"),
    (0x9D, "ugrave"),
    (0x9E, "ucircumflex"),
    (0x9F, "udieresis"),
    (0xA0, "dagger"),
    (0xA1, "degree"),
    (0xA2, "cent"),
    (0xA3, "sterling"),
    (0xA4, "section"),
    (0xA5, "bullet"),
    (0xA6, "paragraph"),
    (0xA7, "germandbls"),
    (0xA8, "registered"),
    (0xA9, "copyright"),
    (0xAA, "trademark"),
    (0xAB, "acute"),
    (0xAC, "dieresis"),
    (0xAE, "AE"),
    (0xAF, "Oslash"),
    (0xB0, "infinity"),
    (0xB1, "plusminus"),
    (0xB2, "lessequal"),
    (0xB3, "greaterequal"),
    (0xB4, "yen"),
    (0xB5, "mu"),
    (0xB6, "partialdiff"),
    (0xB7, "summation"),
    (0xB8, "product"),
    (0xB9, "pi"),
    (0xBA, "integral"),
    (0xBB, "ordfeminine"),
    (0xBC, "ordmasculine"),
    (0xBE, "ae"),
    (0xBF, "oslash"),
    (0xC0, "questiondown"),
    (0xC1, "exclamdown"),
    (0xC2, "logicalnot"),
    (0xC3, "radical"),
    (0xC4, "florin"),
    (0xC5, "approxequal"),
    (0xC6, "Delta"),
    (0xC7, "guillemotleft"),
    (0xC8, "guillemotright"),
    (0xC9, "ellipsis"),
    (0xCA, "space"),
    (0xCB, "Agrave"),
    (0xCC, "Atilde"),
    (0xCD, "Otilde"),
    (0xCE, "OE"),
    (0xCF, "oe"),
    (0xD0, "endash"),
    (0xD1, "emdash"),
    (0xD2, "quotedblleft"),
    (0xD3, "quotedblright"),
    (0xD4, "quoteleft"),
    (0xD5, "quoteright"),
    (0xD6, "divide"),
    (0xD7, "lozenge"),
    (0xD8, "ydieresis"),
    (0xD9, "Ydieresis"),
    (0xDA, "fraction"),
    (0xDB, "currency"),
    (0xDC, "guilsinglleft"),
    (0xDD, "guilsinglright"),
    (0xDE, "fi"),
    (0xDF, "fl"),
    (0xE0, "daggerdbl"),
    (0xE1, "periodcentered"),
    (0xE2, "quotesinglbase"),
    (0xE3, "quotedblbase"),
    (0xE4, "perthousand"),
    (0xE5, "Acircumflex"),
    (0xE6, "Ecircumflex"),
    (0xE7, "Aacute"),
    (0xE8, "Edieresis"),
    (0xE9, "Egrave"),
    (0xEA, "Iacute"),
    (0xEB, "Icircumflex"),
    (0xEC, "Idieresis"),
    (0xED, "Igrave"),
    (0xEE, "Oacute"),
    (0xEF, "Ocircumflex"),
    (0xF1, "Ograve"),
    (0xF2, "Uacute"),
    (0xF3, "Ucircumflex"),
    (0xF4, "Ugrave"),
    (0xF5, "dotlessi"),
    (0xF6, "circumflex"),
    (0xF7, "tilde"),
    (0xF8, "macron"),
    (0xF9, "breve"),
    (0xFA, "dotaccent"),
    (0xFB, "ring"),
    (0xFC, "cedilla"),
    (0xFD, "hungarumlaut"),
    (0xFE, "ogonek"),
    (0xFF, "caron"),
];

/// The Symbol font's built-in encoding, mapped straight to Unicode-bearing names.
fn symbol_name(code: u8) -> &'static str {
    match code {
        0x22 => "universal",
        0x24 => "existential",
        0x27 => "suchthat",
        0x40 => "congruent",
        0x41 => "Alpha",
        0x42 => "Beta",
        0x43 => "Chi",
        0x44 => "Delta",
        0x45 => "Epsilon",
        0x46 => "Phi",
        0x47 => "Gamma",
        0x48 => "Eta",
        0x49 => "Iota",
        0x4A => "theta1",
        0x4B => "Kappa",
        0x4C => "Lambda",
        0x4D => "Mu",
        0x4E => "Nu",
        0x4F => "Omicron",
        0x50 => "Pi",
        0x51 => "Theta",
        0x52 => "Rho",
        0x53 => "Sigma",
        0x54 => "Tau",
        0x55 => "Upsilon",
        0x56 => "sigma1",
        0x57 => "Omega",
        0x58 => "Xi",
        0x59 => "Psi",
        0x5A => "Zeta",
        0x5C => "therefore",
        0x5E => "perpendicular",
        0x60 => "radicalex",
        0x61 => "alpha",
        0x62 => "beta",
        0x63 => "chi",
        0x64 => "delta",
        0x65 => "epsilon",
        0x66 => "phi",
        0x67 => "gamma",
        0x68 => "eta",
        0x69 => "iota",
        0x6A => "phi1",
        0x6B => "kappa",
        0x6C => "lambda",
        0x6D => "mu",
        0x6E => "nu",
        0x6F => "omicron",
        0x70 => "pi",
        0x71 => "theta",
        0x72 => "rho",
        0x73 => "sigma",
        0x74 => "tau",
        0x75 => "upsilon",
        0x76 => "omega1",
        0x77 => "omega",
        0x78 => "xi",
        0x79 => "psi",
        0x7A => "zeta",
        0x7E => "similar",
        0xA2 => "minute",
        0xA3 => "lessequal",
        0xA4 => "fraction",
        0xA5 => "infinity",
        0xA6 => "florin",
        0xA8 => "diamond",
        0xA9 => "heart",
        0xAA => "spade",
        0xAB => "arrowboth",
        0xAC => "arrowleft",
        0xAD => "arrowup",
        0xAE => "arrowright",
        0xAF => "arrowdown",
        0xB0 => "degree",
        0xB1 => "plusminus",
        0xB2 => "second",
        0xB3 => "greaterequal",
        0xB4 => "multiply",
        0xB5 => "proportional",
        0xB6 => "partialdiff",
        0xB7 => "bullet",
        0xB8 => "divide",
        0xB9 => "notequal",
        0xBA => "equivalence",
        0xBB => "approxequal",
        0xBC => "ellipsis",
        0xC0 => "aleph",
        0xC1 => "Ifraktur",
        0xC2 => "Rfraktur",
        0xC3 => "weierstrass",
        0xC4 => "circlemultiply",
        0xC5 => "circleplus",
        0xC6 => "emptyset",
        0xC7 => "intersection",
        0xC8 => "union",
        0xC9 => "propersuperset",
        0xCA => "reflexsuperset",
        0xCB => "notsubset",
        0xCC => "propersubset",
        0xCD => "reflexsubset",
        0xCE => "element",
        0xCF => "notelement",
        0xD0 => "angle",
        0xD1 => "gradient",
        0xD5 => "product",
        0xD6 => "radical",
        0xD7 => "dotmath",
        0xD8 => "logicalnot",
        0xD9 => "logicaland",
        0xDA => "logicalor",
        0xDB => "arrowdblboth",
        0xDC => "arrowdblleft",
        0xDD => "arrowdblup",
        0xDE => "arrowdblright",
        0xDF => "arrowdbldown",
        0xE5 => "summation",
        0xF2 => "integral",
        // ASCII-identical slots.
        0x20..=0x7F => ASCII_NAMES.get(code as usize - 32).copied().unwrap_or(""),
        _ => "",
    }
}

/// Adobe Glyph List subset: everything the encoding tables above name, plus Greek, ligatures
/// and the mathematical symbols that turn up in scientific typesetting.
///
/// Sorted by name for binary search; the test enforces it.
#[rustfmt::skip]
static AGL: &[(&str, char)] = &[
    ("A", 'A'), ("AE", 'Æ'), ("Aacute", 'Á'), ("Acircumflex", 'Â'), ("Adieresis", 'Ä'),
    ("Agrave", 'À'), ("Alpha", 'Α'), ("Aring", 'Å'), ("Atilde", 'Ã'), ("B", 'B'),
    ("Beta", 'Β'), ("C", 'C'), ("Ccedilla", 'Ç'), ("Chi", 'Χ'), ("D", 'D'),
    ("Delta", 'Δ'), ("E", 'E'), ("Eacute", 'É'), ("Ecircumflex", 'Ê'), ("Edieresis", 'Ë'),
    ("Egrave", 'È'), ("Epsilon", 'Ε'), ("Eta", 'Η'), ("Eth", 'Ð'), ("Euro", '€'),
    ("F", 'F'), ("G", 'G'), ("Gamma", 'Γ'), ("H", 'H'), ("I", 'I'),
    ("Iacute", 'Í'), ("Icircumflex", 'Î'), ("Idieresis", 'Ï'), ("Ifraktur", 'ℑ'),
    ("Igrave", 'Ì'), ("Iota", 'Ι'), ("J", 'J'), ("K", 'K'), ("Kappa", 'Κ'),
    ("L", 'L'), ("Lambda", 'Λ'), ("Lslash", 'Ł'), ("M", 'M'), ("Mu", 'Μ'),
    ("N", 'N'), ("Ntilde", 'Ñ'), ("Nu", 'Ν'), ("O", 'O'), ("OE", 'Œ'),
    ("Oacute", 'Ó'), ("Ocircumflex", 'Ô'), ("Odieresis", 'Ö'), ("Ograve", 'Ò'),
    ("Omega", 'Ω'), ("Omicron", 'Ο'), ("Oslash", 'Ø'), ("Otilde", 'Õ'), ("P", 'P'),
    ("Phi", 'Φ'), ("Pi", 'Π'), ("Psi", 'Ψ'), ("Q", 'Q'), ("R", 'R'),
    ("Rfraktur", 'ℜ'), ("Rho", 'Ρ'), ("S", 'S'), ("Scaron", 'Š'), ("Sigma", 'Σ'),
    ("T", 'T'), ("Tau", 'Τ'), ("Theta", 'Θ'), ("Thorn", 'Þ'), ("U", 'U'),
    ("Uacute", 'Ú'), ("Ucircumflex", 'Û'), ("Udieresis", 'Ü'), ("Ugrave", 'Ù'),
    ("Upsilon", 'Υ'), ("V", 'V'), ("W", 'W'), ("X", 'X'), ("Xi", 'Ξ'),
    ("Y", 'Y'), ("Yacute", 'Ý'), ("Ydieresis", 'Ÿ'), ("Z", 'Z'), ("Zcaron", 'Ž'),
    ("Zeta", 'Ζ'), ("a", 'a'), ("aacute", 'á'), ("acircumflex", 'â'), ("acute", '´'),
    ("adieresis", 'ä'), ("ae", 'æ'), ("agrave", 'à'), ("aleph", 'ℵ'), ("alpha", 'α'),
    ("ampersand", '&'), ("angle", '∠'), ("angleleft", '⟨'), ("angleright", '⟩'),
    ("approxequal", '≈'), ("aring", 'å'), ("arrowboth", '↔'), ("arrowdblboth", '⇔'),
    ("arrowdbldown", '⇓'), ("arrowdblleft", '⇐'), ("arrowdblright", '⇒'), ("arrowdblup", '⇑'),
    ("arrowdown", '↓'), ("arrowleft", '←'), ("arrowright", '→'), ("arrowup", '↑'),
    ("asciicircum", '^'), ("asciitilde", '~'), ("asterisk", '*'), ("asteriskmath", '∗'),
    ("at", '@'), ("atilde", 'ã'), ("b", 'b'), ("backslash", '\\'), ("bar", '|'),
    ("beta", 'β'), ("braceleft", '{'), ("braceright", '}'), ("bracketleft", '['),
    ("bracketright", ']'), ("breve", '˘'), ("brokenbar", '¦'), ("bullet", '•'),
    ("c", 'c'), ("caron", 'ˇ'), ("ccedilla", 'ç'), ("cedilla", '¸'), ("cent", '¢'),
    ("chi", 'χ'), ("circlemultiply", '⊗'), ("circleplus", '⊕'), ("circumflex", 'ˆ'),
    ("club", '♣'), ("colon", ':'), ("comma", ','), ("congruent", '≅'), ("copyright", '©'),
    ("currency", '¤'), ("d", 'd'), ("dagger", '†'), ("daggerdbl", '‡'), ("degree", '°'),
    ("delta", 'δ'), ("diamond", '♦'), ("dieresis", '¨'), ("divide", '÷'), ("dollar", '$'),
    ("dotaccent", '˙'), ("dotlessi", 'ı'), ("dotlessj", 'ȷ'), ("dotmath", '⋅'),
    ("e", 'e'), ("eacute", 'é'), ("ecircumflex", 'ê'), ("edieresis", 'ë'), ("egrave", 'è'),
    ("eight", '8'), ("element", '∈'), ("ellipsis", '…'), ("emdash", '—'), ("emptyset", '∅'),
    ("endash", '–'), ("epsilon", 'ε'), ("equal", '='), ("equivalence", '≡'), ("eta", 'η'),
    ("eth", 'ð'), ("exclam", '!'), ("exclamdown", '¡'), ("existential", '∃'), ("f", 'f'),
    ("ff", 'ﬀ'), ("ffi", 'ﬃ'), ("ffl", 'ﬄ'), ("fi", 'ﬁ'), ("five", '5'),
    ("fl", 'ﬂ'), ("florin", 'ƒ'), ("four", '4'), ("fraction", '⁄'), ("g", 'g'),
    ("gamma", 'γ'), ("germandbls", 'ß'), ("gradient", '∇'), ("grave", '`'), ("greater", '>'),
    ("greaterequal", '≥'), ("guillemotleft", '«'), ("guillemotright", '»'),
    ("guilsinglleft", '‹'), ("guilsinglright", '›'), ("h", 'h'), ("heart", '♥'),
    ("hungarumlaut", '˝'), ("hyphen", '-'), ("i", 'i'), ("iacute", 'í'),
    ("icircumflex", 'î'), ("idieresis", 'ï'), ("igrave", 'ì'), ("infinity", '∞'),
    ("integral", '∫'), ("intersection", '∩'), ("iota", 'ι'), ("j", 'j'), ("k", 'k'),
    ("kappa", 'κ'), ("l", 'l'), ("lambda", 'λ'), ("less", '<'), ("lessequal", '≤'),
    ("logicaland", '∧'), ("logicalnot", '¬'), ("logicalor", '∨'), ("lozenge", '◊'),
    ("lslash", 'ł'), ("m", 'm'), ("macron", '¯'), ("minus", '−'), ("minute", '′'),
    ("mu", 'µ'), ("multiply", '×'), ("n", 'n'), ("nine", '9'), ("notelement", '∉'),
    ("notequal", '≠'), ("notsubset", '⊄'), ("ntilde", 'ñ'), ("nu", 'ν'),
    ("numbersign", '#'), ("o", 'o'), ("oacute", 'ó'), ("ocircumflex", 'ô'),
    ("odieresis", 'ö'), ("oe", 'œ'), ("ogonek", '˛'), ("ograve", 'ò'), ("omega", 'ω'),
    ("omega1", 'ϖ'), ("omicron", 'ο'), ("one", '1'), ("onehalf", '½'), ("onequarter", '¼'),
    ("onesuperior", '¹'), ("ordfeminine", 'ª'), ("ordmasculine", 'º'), ("oslash", 'ø'),
    ("otilde", 'õ'), ("p", 'p'), ("paragraph", '¶'), ("parenleft", '('),
    ("parenright", ')'), ("partialdiff", '∂'), ("percent", '%'), ("period", '.'),
    ("periodcentered", '·'), ("perpendicular", '⊥'), ("perthousand", '‰'), ("phi", 'φ'),
    ("phi1", 'ϕ'), ("pi", 'π'), ("plus", '+'), ("plusminus", '±'), ("product", '∏'),
    ("propersubset", '⊂'), ("propersuperset", '⊃'), ("proportional", '∝'), ("psi", 'ψ'),
    ("q", 'q'), ("question", '?'), ("questiondown", '¿'), ("quotedbl", '"'),
    ("quotedblbase", '„'), ("quotedblleft", '“'), ("quotedblright", '”'),
    ("quoteleft", '‘'), ("quoteright", '’'), ("quotesinglbase", '‚'), ("quotesingle", '\''),
    ("r", 'r'), ("radical", '√'), ("radicalex", '‾'), ("reflexsubset", '⊆'),
    ("reflexsuperset", '⊇'), ("registered", '®'), ("rho", 'ρ'), ("ring", '˚'),
    ("s", 's'), ("scaron", 'š'), ("second", '″'), ("section", '§'), ("semicolon", ';'),
    ("seven", '7'), ("sigma", 'σ'), ("sigma1", 'ς'), ("similar", '∼'), ("six", '6'),
    ("slash", '/'), ("space", ' '), ("spade", '♠'), ("sterling", '£'), ("suchthat", '∋'),
    ("summation", '∑'), ("t", 't'), ("tau", 'τ'), ("theta", 'θ'), ("theta1", 'ϑ'),
    ("thorn", 'þ'), ("three", '3'), ("threequarters", '¾'), ("threesuperior", '³'),
    ("tilde", '˜'), ("trademark", '™'), ("two", '2'), ("twosuperior", '²'),
    ("u", 'u'), ("uacute", 'ú'), ("ucircumflex", 'û'), ("udieresis", 'ü'),
    ("ugrave", 'ù'), ("underscore", '_'), ("union", '∪'), ("universal", '∀'),
    ("upsilon", 'υ'), ("v", 'v'), ("w", 'w'), ("weierstrass", '℘'), ("x", 'x'),
    ("xi", 'ξ'), ("y", 'y'), ("yacute", 'ý'), ("ydieresis", 'ÿ'), ("yen", '¥'),
    ("z", 'z'), ("zcaron", 'ž'), ("zero", '0'), ("zeta", 'ζ'),
];

/// Names used by TeX's mathematics fonts that no standard glyph list carries.
///
/// The Adobe list covers text. TeX's `CMSY`, `CMEX` and the AMS fonts name symbols Adobe never
/// had to, and they name delimiters by construction — a tall bracket is three glyphs, not one.
#[rustfmt::skip]
static MATH_NAMES: &[(&str, &str)] = &[
    ("angbracketleft", "\u{27E8}"), ("angbracketright", "\u{27E9}"),
    ("arrowhookleft", "\u{21A9}"), ("arrowhookright", "\u{21AA}"),
    ("arrowlefttophalf", "\u{21BC}"), ("arrowrighttophalf", "\u{21C0}"),
    ("bardbl", "\u{2016}"), ("braceleft", "{"), ("braceright", "}"),
    ("ceilingleft", "\u{2308}"), ("ceilingright", "\u{2309}"),
    ("circlecopyrt", "\u{00A9}"), ("circledivide", "\u{2298}"),
    ("circledot", "\u{2299}"), ("circleminus", "\u{2296}"),
    ("contintegral", "\u{222E}"), ("coproduct", "\u{2210}"),
    ("floorleft", "\u{230A}"), ("floorright", "\u{230B}"),
    ("intersection", "\u{2229}"), ("logicaland", "\u{2227}"), ("logicalor", "\u{2228}"),
    ("negationslash", "\u{2044}"), ("owner", "\u{220B}"),
    ("summation", "\u{2211}"), ("union", "\u{222A}"), ("unionmulti", "\u{228E}"),
    ("uniondbl", "\u{228C}"), ("vextenddouble", "\u{2016}"), ("vextendsingle", "|"),
];

/// Size variants TeX appends to a delimiter or operator name. A `\\big(` is a different glyph
/// from `(` but the same character.
const MATH_SIZES: [&str; 8] = ["Bigg", "bigg", "Big", "big", "display", "text", "Ex", "ex"];

/// Resolves a TeX mathematics glyph name.
///
/// Two conventions have to be undone. Size variants (`summationdisplay`, `parenleftBig`) are the
/// same character drawn larger, so the suffix is stripped. Extensible delimiters are *built* from
/// pieces — `parenlefttp`, `parenleftex`, `parenleftbt` stack into one tall bracket — so the top
/// piece stands for the delimiter and the middle and bottom pieces yield nothing, which is what
/// makes a three-glyph bracket extract as a single character rather than three.
fn math_name_to_unicode(name: &str) -> Option<String> {
    // Piece suffixes first: `parenlefttp` also ends in no size suffix, and stripping in the
    // other order would turn `bracketrightbt` into `bracketrightb`.
    for piece in ["tp", "bt", "mid", "ex"] {
        if let Some(base) = name.strip_suffix(piece) {
            if base.len() >= 3
                && (base.starts_with("paren")
                    || base.starts_with("bracket")
                    || base.starts_with("brace")
                    || base.starts_with("angbracket")
                    || base.starts_with("arrow")
                    || base.starts_with("radical")
                    || base.starts_with("vert")
                    || base.starts_with("bracehtip"))
            {
                return match piece {
                    // Only the top piece carries the character; the rest continue the same one.
                    "tp" => lookup_math(base).or_else(|| plain_lookup(base)),
                    _ => Some(String::new()),
                };
            }
        }
    }
    for size in MATH_SIZES {
        if let Some(base) = name.strip_suffix(size) {
            if base.len() >= 3 {
                if let Some(found) = lookup_math(base).or_else(|| plain_lookup(base)) {
                    return Some(found);
                }
            }
        }
    }
    lookup_math(name)
}

fn lookup_math(name: &str) -> Option<String> {
    MATH_NAMES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, v)| (*v).to_string())
}

/// The Adobe list only, so that the math resolver cannot recurse into itself.
fn plain_lookup(name: &str) -> Option<String> {
    AGL.binary_search_by(|(n, _)| n.cmp(&name))
        .ok()
        .map(|i| AGL[i].1.to_string())
}

/// Maps a glyph name to its Unicode string.
///
/// Resolution order mirrors the AGL specification: the list itself, `uniXXXX[XXXX...]`,
/// `uXXXX`–`uXXXXXX`, then TeX's mathematics conventions, then a suffix-stripped retry
/// (`one.oldstyle` → `one`).
pub fn glyph_name_to_unicode(name: &str) -> Option<String> {
    if name.is_empty() || name == ".notdef" {
        return None;
    }
    if let Ok(i) = AGL.binary_search_by(|(n, _)| n.cmp(&name)) {
        return Some(AGL[i].1.to_string());
    }
    if let Some(hex) = name.strip_prefix("uni") {
        if hex.len() >= 4 && hex.len() % 4 == 0 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
            let units: Vec<u16> = hex
                .as_bytes()
                .chunks(4)
                .filter_map(|c| u16::from_str_radix(std::str::from_utf8(c).ok()?, 16).ok())
                .collect();
            let s = String::from_utf16_lossy(&units);
            if !s.is_empty() {
                return Some(s);
            }
        }
    }
    if let Some(hex) = name.strip_prefix("u") {
        if (4..=6).contains(&hex.len()) && hex.chars().all(|c| c.is_ascii_hexdigit()) {
            if let Some(ch) = u32::from_str_radix(hex, 16).ok().and_then(char::from_u32) {
                return Some(ch.to_string());
            }
        }
    }
    // TeX mathematics names: size variants and the pieces of an extensible delimiter.
    if let Some(found) = math_name_to_unicode(name) {
        return Some(found);
    }
    // `g.alt`, `a.sc`, `T1`-style subset suffixes.
    if let Some(base) = name
        .split('.')
        .next()
        .filter(|b| *b != name && !b.is_empty())
    {
        return glyph_name_to_unicode(base);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agl_is_sorted_for_binary_search() {
        for w in AGL.windows(2) {
            assert!(w[0].0 < w[1].0, "AGL out of order at {:?}", w[1].0);
        }
    }

    #[test]
    fn name_lookups() {
        assert_eq!(glyph_name_to_unicode("A").as_deref(), Some("A"));
        assert_eq!(glyph_name_to_unicode("alpha").as_deref(), Some("α"));
        assert_eq!(glyph_name_to_unicode("ffi").as_deref(), Some("ﬃ"));
        assert_eq!(glyph_name_to_unicode("uni0041").as_deref(), Some("A"));
        assert_eq!(glyph_name_to_unicode("uni00660066").as_deref(), Some("ff"));
        assert_eq!(glyph_name_to_unicode("u1D400").as_deref(), Some("𝐀"));
        assert_eq!(glyph_name_to_unicode("one.oldstyle").as_deref(), Some("1"));
        assert_eq!(glyph_name_to_unicode(".notdef"), None);
        assert_eq!(glyph_name_to_unicode("madeupname"), None);
    }

    #[test]
    fn tex_size_variants_resolve_to_one_character() {
        // `\\big(` through `\\Bigg(` are four glyphs and one character.
        for n in [
            "parenleftbig",
            "parenleftBig",
            "parenleftbigg",
            "parenleftBigg",
        ] {
            assert_eq!(glyph_name_to_unicode(n).as_deref(), Some("("), "{n}");
        }
        assert_eq!(
            glyph_name_to_unicode("bracketrightBig").as_deref(),
            Some("]")
        );
        assert_eq!(glyph_name_to_unicode("radicalbig").as_deref(), Some("√"));
    }

    #[test]
    fn display_and_text_operators_resolve() {
        assert_eq!(
            glyph_name_to_unicode("summationdisplay").as_deref(),
            Some("∑")
        );
        assert_eq!(glyph_name_to_unicode("summationtext").as_deref(), Some("∑"));
        assert_eq!(
            glyph_name_to_unicode("integraldisplay").as_deref(),
            Some("∫")
        );
        assert_eq!(
            glyph_name_to_unicode("productdisplay").as_deref(),
            Some("∏")
        );
        assert_eq!(glyph_name_to_unicode("uniondisplay").as_deref(), Some("∪"));
        assert_eq!(
            glyph_name_to_unicode("contintegraldisplay").as_deref(),
            Some("∮")
        );
    }

    #[test]
    fn an_extensible_delimiter_yields_exactly_one_character() {
        // A tall bracket is stacked from three glyphs. Emitting a character per piece would
        // triple every delimiter in a displayed matrix.
        assert_eq!(glyph_name_to_unicode("parenlefttp").as_deref(), Some("("));
        assert_eq!(glyph_name_to_unicode("parenleftex").as_deref(), Some(""));
        assert_eq!(glyph_name_to_unicode("parenleftbt").as_deref(), Some(""));
        assert_eq!(
            glyph_name_to_unicode("bracketrighttp").as_deref(),
            Some("]")
        );
        assert_eq!(glyph_name_to_unicode("bracketrightex").as_deref(), Some(""));
    }

    #[test]
    fn math_names_absent_from_the_adobe_list_resolve() {
        assert_eq!(
            glyph_name_to_unicode("angbracketleft").as_deref(),
            Some("⟨")
        );
        assert_eq!(glyph_name_to_unicode("floorleft").as_deref(), Some("⌊"));
        assert_eq!(glyph_name_to_unicode("ceilingright").as_deref(), Some("⌉"));
        assert_eq!(glyph_name_to_unicode("bardbl").as_deref(), Some("‖"));
        assert_eq!(glyph_name_to_unicode("coproduct").as_deref(), Some("∐"));
    }

    #[test]
    fn ordinary_names_are_not_mangled_by_the_math_rules() {
        // `big`, `text` and `ex` are suffixes only in TeX's mathematics fonts; a text glyph
        // whose name merely ends that way must survive untouched.
        assert_eq!(glyph_name_to_unicode("a").as_deref(), Some("a"));
        assert_eq!(glyph_name_to_unicode("six").as_deref(), Some("6"));
        assert_eq!(glyph_name_to_unicode("bullet").as_deref(), Some("•"));
        assert_eq!(glyph_name_to_unicode("nosuchglyphbig"), None);
    }

    #[test]
    fn encodings_disagree_where_documented() {
        assert_eq!(base_encoding_name(Encoding::Standard, 0x27), "quoteright");
        assert_eq!(base_encoding_name(Encoding::WinAnsi, 0x27), "quotesingle");
        assert_eq!(base_encoding_name(Encoding::WinAnsi, 0xE9), "eacute");
        assert_eq!(base_encoding_name(Encoding::MacRoman, 0x8E), "eacute");
        assert_eq!(base_encoding_name(Encoding::Symbol, 0x61), "alpha");
        assert_eq!(base_encoding_name(Encoding::Standard, 0xAE), "fi");
    }
}
