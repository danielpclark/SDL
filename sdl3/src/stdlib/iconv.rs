// Rust translation of src/stdlib/SDL_iconv.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Character set conversion between ASCII, Latin-1, UTF-8, UTF-16, UTF-32,
//! UCS-2 and UCS-4. Translation of SDL's built-in `SDL_iconv` (the code C
//! SDL uses when the platform has no `iconv()`).
//!
//! Lots of useful information on Unicode at:
//! <http://www.cl.cam.ac.uk/~mgk25/unicode.html>

use std::fmt;

use crate::error::{Error, Result};

const UNICODE_BOM: u32 = 0xFEFF;

const UNKNOWN_ASCII: u8 = b'?';
const UNKNOWN_UNICODE: u32 = 0xFFFD;

/// The encodings SDL's iconv understands. Translation of the `ENCODING_*` enum.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Encoding {
    Ascii,
    Latin1,
    Utf8,
    /// Needs byte order marker
    Utf16,
    Utf16Be,
    Utf16Le,
    /// Needs byte order marker
    Utf32,
    Utf32Be,
    Utf32Le,
    Ucs2Be,
    Ucs2Le,
    Ucs4Be,
    Ucs4Le,
}

impl Encoding {
    /// `ENCODING_UTF16NATIVE`
    pub const UTF16_NATIVE: Encoding = if cfg!(target_endian = "big") {
        Encoding::Utf16Be
    } else {
        Encoding::Utf16Le
    };
    /// `ENCODING_UTF32NATIVE`
    pub const UTF32_NATIVE: Encoding = if cfg!(target_endian = "big") {
        Encoding::Utf32Be
    } else {
        Encoding::Utf32Le
    };
    /// `ENCODING_UCS2NATIVE`
    pub const UCS2_NATIVE: Encoding = if cfg!(target_endian = "big") {
        Encoding::Ucs2Be
    } else {
        Encoding::Ucs2Le
    };
    /// `ENCODING_UCS4NATIVE`
    pub const UCS4_NATIVE: Encoding = if cfg!(target_endian = "big") {
        Encoding::Ucs4Be
    } else {
        Encoding::Ucs4Le
    };

    /// Look an encoding up by name, case-insensitively (`"UTF-8"`, `"utf16le"`,
    /// `"ISO-8859-1"`, `"WCHAR_T"`, ...). Uses the `encodings[]` table.
    pub fn from_name(name: &str) -> Option<Encoding> {
        ENCODINGS
            .iter()
            .find(|(n, _)| super::string::strcasecmp(name, *n).is_eq())
            .map(|&(_, e)| e)
    }
}

/// Translation of the `encodings[]` table.
static ENCODINGS: [(&str, Encoding); 29] = [
    ("ASCII", Encoding::Ascii),
    ("US-ASCII", Encoding::Ascii),
    ("8859-1", Encoding::Latin1),
    ("ISO-8859-1", Encoding::Latin1),
    (
        "WCHAR_T",
        if cfg!(windows) {
            Encoding::Utf16Le
        } else {
            Encoding::UCS4_NATIVE
        },
    ),
    ("UTF8", Encoding::Utf8),
    ("UTF-8", Encoding::Utf8),
    ("UTF16", Encoding::Utf16),
    ("UTF-16", Encoding::Utf16),
    ("UTF16BE", Encoding::Utf16Be),
    ("UTF-16BE", Encoding::Utf16Be),
    ("UTF16LE", Encoding::Utf16Le),
    ("UTF-16LE", Encoding::Utf16Le),
    ("UTF32", Encoding::Utf32),
    ("UTF-32", Encoding::Utf32),
    ("UTF32BE", Encoding::Utf32Be),
    ("UTF-32BE", Encoding::Utf32Be),
    ("UTF32LE", Encoding::Utf32Le),
    ("UTF-32LE", Encoding::Utf32Le),
    ("UCS2", Encoding::Ucs2Be),
    ("UCS-2", Encoding::Ucs2Be),
    ("UCS-2LE", Encoding::Ucs2Le),
    ("UCS-2BE", Encoding::Ucs2Be),
    ("UCS-2-INTERNAL", Encoding::UCS2_NATIVE),
    ("UCS4", Encoding::Ucs4Be),
    ("UCS-4", Encoding::Ucs4Be),
    ("UCS-4LE", Encoding::Ucs4Le),
    ("UCS-4BE", Encoding::Ucs4Be),
    ("UCS-4-INTERNAL", Encoding::UCS4_NATIVE),
];

/// Why a conversion stopped. Translation of `SDL_ICONV_E2BIG`,
/// `SDL_ICONV_EILSEQ` and `SDL_ICONV_EINVAL`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum IconvError {
    /// Output buffer was too small.
    TooBig,
    /// Invalid input sequence was encountered.
    IllegalSequence,
    /// Incomplete input sequence was encountered.
    Incomplete,
}

impl fmt::Display for IconvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            IconvError::TooBig => "Output buffer was too small",
            IconvError::IllegalSequence => "Invalid input sequence was encountered",
            IconvError::Incomplete => "Incomplete input sequence was encountered",
        })
    }
}

impl std::error::Error for IconvError {}

/// The locale's character set from `LC_ALL`/`LC_CTYPE`/`LC_MESSAGES`/`LANG`
/// (`"en_US.UTF-8@euro"` gives `"UTF-8"`; unset or `"C"` gives `"ASCII"`).
/// Translation of the static `getlocale()`.
fn getlocale() -> String {
    let lang = super::getenv("LC_ALL")
        .or_else(|| super::getenv("LC_CTYPE"))
        .or_else(|| super::getenv("LC_MESSAGES"))
        .or_else(|| super::getenv("LANG"));
    let mut lang = match lang {
        Some(l) if !l.is_empty() && l != "C" => l,
        _ => "ASCII".to_owned(),
    };
    // We need to trim down strings like "en_US.UTF-8@blah" to "UTF-8"
    if let Some(dot) = lang.find('.') {
        lang = lang[dot + 1..].to_owned();
    }
    lang.truncate(63); // SDL_strlcpy() into a 64-byte buffer (bytes; keep char boundaries)
    while !lang.is_char_boundary(lang.len()) {
        lang.pop();
    }
    if let Some(at) = lang.find('@') {
        lang.truncate(at); // chop end of string.
    }
    lang
}

/// A conversion descriptor. Translation of `SDL_iconv_t` (`struct SDL_iconv_data_t`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Iconv {
    src_fmt: Encoding,
    dst_fmt: Encoding,
}

impl Iconv {
    /// Prepare a conversion from `fromcode` to `tocode`; an empty name means
    /// the locale's character set. Translation of `SDL_iconv_open()`.
    pub fn open(tocode: &str, fromcode: &str) -> Result<Iconv> {
        let fromcode = if fromcode.is_empty() {
            getlocale()
        } else {
            fromcode.to_owned()
        };
        let tocode = if tocode.is_empty() {
            getlocale()
        } else {
            tocode.to_owned()
        };
        match (Encoding::from_name(&fromcode), Encoding::from_name(&tocode)) {
            (Some(src_fmt), Some(dst_fmt)) => Ok(Iconv { src_fmt, dst_fmt }),
            (None, _) => Err(Error::new(format!("Unsupported encoding \"{fromcode}\""))),
            (_, None) => Err(Error::new(format!("Unsupported encoding \"{tocode}\""))),
        }
    }

    /// A descriptor for two known encodings.
    pub fn new(to: Encoding, from: Encoding) -> Iconv {
        Iconv {
            src_fmt: from,
            dst_fmt: to,
        }
    }

    /// The current source encoding (a `Utf16`/`Utf32` source becomes the
    /// detected byte order after the first [`convert`](Self::convert)).
    pub fn source(&self) -> Encoding {
        self.src_fmt
    }

    /// The current destination encoding.
    pub fn destination(&self) -> Encoding {
        self.dst_fmt
    }

    /// Convert as much of `inbuf` into `outbuf` as fits, advancing both slices
    /// past what was consumed and produced, and return the number of
    /// characters converted. Translation of `SDL_iconv()`.
    ///
    /// On error the slices still reflect everything converted up to the
    /// failing character. Invalid input is replaced (U+FFFD or `?`) rather
    /// than reported, as upstream does; only [`IconvError::TooBig`] and
    /// [`IconvError::Incomplete`] are ever returned.
    pub fn convert(
        &mut self,
        inbuf: &mut &[u8],
        outbuf: &mut &mut [u8],
    ) -> std::result::Result<usize, IconvError> {
        // For simplicity, we'll convert everything to and from UCS-4

        if outbuf.is_empty() {
            return Err(IconvError::TooBig);
        }
        let mut src: &[u8] = inbuf;
        let mut dst_pos = 0usize;

        match self.src_fmt {
            Encoding::Utf16 => {
                // Scan for a byte order marker
                let mut found = false;
                for p in src.chunks_exact(2) {
                    // (upstream maps FF FE to big-endian and FE FF to
                    // little-endian, the wrong way round; fixed here: FF FE
                    // is the little-endian BOM.)
                    if p[0] == 0xFF && p[1] == 0xFE {
                        self.src_fmt = Encoding::Utf16Le;
                        found = true;
                        break;
                    } else if p[0] == 0xFE && p[1] == 0xFF {
                        self.src_fmt = Encoding::Utf16Be;
                        found = true;
                        break;
                    }
                }
                if !found {
                    // We can't tell, default to host order
                    self.src_fmt = Encoding::UTF16_NATIVE;
                }
            }
            Encoding::Utf32 => {
                // Scan for a byte order marker
                let mut found = false;
                for p in src.chunks_exact(4) {
                    // (upstream has the byte orders inverted here too, as
                    // for UTF-16 above; fixed here.)
                    if p[0] == 0xFF && p[1] == 0xFE && p[2] == 0x00 && p[3] == 0x00 {
                        self.src_fmt = Encoding::Utf32Le;
                        found = true;
                        break;
                    } else if p[0] == 0x00 && p[1] == 0x00 && p[2] == 0xFE && p[3] == 0xFF {
                        self.src_fmt = Encoding::Utf32Be;
                        found = true;
                        break;
                    }
                }
                if !found {
                    // We can't tell, default to host order
                    self.src_fmt = Encoding::UTF32_NATIVE;
                }
            }
            _ => {}
        }

        {
            let dst: &mut [u8] = outbuf;
            match self.dst_fmt {
                Encoding::Utf16 => {
                    // Default to host order, need to add byte order marker
                    if dst.len() < 2 {
                        return Err(IconvError::TooBig);
                    }
                    dst[..2].copy_from_slice(&(UNICODE_BOM as u16).to_ne_bytes());
                    dst_pos += 2;
                    self.dst_fmt = Encoding::UTF16_NATIVE;
                }
                Encoding::Utf32 => {
                    // Default to host order, need to add byte order marker
                    if dst.len() < 4 {
                        return Err(IconvError::TooBig);
                    }
                    dst[..4].copy_from_slice(&UNICODE_BOM.to_ne_bytes());
                    dst_pos += 4;
                    self.dst_fmt = Encoding::UTF32_NATIVE;
                }
                _ => {}
            }
        }

        // (upstream advances `*outbuf` only after each converted character,
        // so a BOM written above is lost when the first character doesn't
        // fit: the next call, now in native order, overwrites it. Fixed here
        // by committing the BOM as soon as it is written.)
        let mut committed_dst = dst_pos;
        let mut total = 0;

        let result = loop {
            if src.is_empty() {
                break Ok(total);
            }
            let srclen = src.len();
            let mut ch: u32;
            // Decode a character
            match self.src_fmt {
                Encoding::Ascii => {
                    ch = (src[0] & 0x7F) as u32;
                    src = &src[1..];
                }
                Encoding::Latin1 => {
                    ch = src[0] as u32;
                    src = &src[1..];
                }
                Encoding::Utf8 => {
                    // RFC 3629
                    let p = src;
                    let mut left = 0usize;
                    let mut overlong = false;
                    if p[0] >= 0xF0 {
                        if (p[0] & 0xF8) != 0xF0 {
                            /* Skip illegal sequences
                              return SDL_ICONV_EILSEQ;
                            */
                            ch = UNKNOWN_UNICODE;
                        } else {
                            if p[0] == 0xF0 && srclen > 1 && (p[1] & 0xF0) == 0x80 {
                                overlong = true;
                            }
                            ch = (p[0] & 0x07) as u32;
                            left = 3;
                        }
                    } else if p[0] >= 0xE0 {
                        if (p[0] & 0xF0) != 0xE0 {
                            /* Skip illegal sequences
                              return SDL_ICONV_EILSEQ;
                            */
                            ch = UNKNOWN_UNICODE;
                        } else {
                            if p[0] == 0xE0 && srclen > 1 && (p[1] & 0xE0) == 0x80 {
                                overlong = true;
                            }
                            ch = (p[0] & 0x0F) as u32;
                            left = 2;
                        }
                    } else if p[0] >= 0xC0 {
                        if (p[0] & 0xE0) != 0xC0 {
                            /* Skip illegal sequences
                              return SDL_ICONV_EILSEQ;
                            */
                            ch = UNKNOWN_UNICODE;
                        } else {
                            if (p[0] & 0xDE) == 0xC0 {
                                overlong = true;
                            }
                            ch = (p[0] & 0x1F) as u32;
                            left = 1;
                        }
                    } else if (p[0] & 0x80) != 0 {
                        /* Skip illegal sequences
                          return SDL_ICONV_EILSEQ;
                        */
                        ch = UNKNOWN_UNICODE;
                    } else {
                        ch = p[0] as u32;
                    }
                    src = &src[1..];
                    if src.len() < left {
                        break Err(IconvError::Incomplete);
                    }
                    let mut i = 0;
                    while left > 0 {
                        left -= 1;
                        i += 1;
                        if (p[i] & 0xC0) != 0x80 {
                            /* Skip illegal sequences
                              return SDL_ICONV_EILSEQ;
                            */
                            ch = UNKNOWN_UNICODE;
                            break;
                        }
                        ch <<= 6;
                        ch |= (p[i] & 0x3F) as u32;
                        src = &src[1..];
                    }
                    if overlong {
                        /* Potential security risk
                          return SDL_ICONV_EILSEQ;
                        */
                        ch = UNKNOWN_UNICODE;
                    }
                    if (0xD800..=0xDFFF).contains(&ch)
                        || ch == 0xFFFE
                        || ch == 0xFFFF
                        || ch > 0x10FFFF
                    {
                        /* Skip illegal sequences
                          return SDL_ICONV_EILSEQ;
                        */
                        ch = UNKNOWN_UNICODE;
                    }
                }
                Encoding::Utf16Be | Encoding::Utf16Le => {
                    // RFC 2781
                    let be = self.src_fmt == Encoding::Utf16Be;
                    let word = |p: &[u8]| {
                        if be {
                            u16::from_be_bytes([p[0], p[1]])
                        } else {
                            u16::from_le_bytes([p[0], p[1]])
                        }
                    };
                    if src.len() < 2 {
                        break Err(IconvError::Incomplete);
                    }
                    let w1 = word(src);
                    src = &src[2..];
                    if !(0xD800..=0xDFFF).contains(&w1) {
                        ch = w1 as u32;
                    } else if w1 > 0xDBFF {
                        /* Skip illegal sequences
                          return SDL_ICONV_EILSEQ;
                        */
                        ch = UNKNOWN_UNICODE;
                    } else {
                        if src.len() < 2 {
                            break Err(IconvError::Incomplete);
                        }
                        let w2 = word(src);
                        src = &src[2..];
                        if !(0xDC00..=0xDFFF).contains(&w2) {
                            /* Skip illegal sequences
                              return SDL_ICONV_EILSEQ;
                            */
                            ch = UNKNOWN_UNICODE;
                        } else {
                            ch = ((((w1 & 0x3FF) as u32) << 10) | (w2 & 0x3FF) as u32) + 0x10000;
                        }
                    }
                }
                Encoding::Ucs2Le | Encoding::Ucs2Be => {
                    if src.len() < 2 {
                        break Err(IconvError::Incomplete);
                    }
                    ch = if self.src_fmt == Encoding::Ucs2Le {
                        u16::from_le_bytes([src[0], src[1]]) as u32
                    } else {
                        u16::from_be_bytes([src[0], src[1]]) as u32
                    };
                    src = &src[2..];
                }
                Encoding::Ucs4Be | Encoding::Utf32Be | Encoding::Ucs4Le | Encoding::Utf32Le => {
                    if src.len() < 4 {
                        break Err(IconvError::Incomplete);
                    }
                    let b = [src[0], src[1], src[2], src[3]];
                    ch = if matches!(self.src_fmt, Encoding::Ucs4Be | Encoding::Utf32Be) {
                        u32::from_be_bytes(b)
                    } else {
                        u32::from_le_bytes(b)
                    };
                    src = &src[4..];
                }
                // (Utf16/Utf32 sources were resolved to a byte order above)
                Encoding::Utf16 | Encoding::Utf32 => {
                    unreachable!("byte order resolved before decoding")
                }
            }

            // Encode a character
            let dst: &mut [u8] = &mut outbuf[dst_pos..];
            let dstlen = dst.len();
            let written = match self.dst_fmt {
                Encoding::Ascii => {
                    if dstlen < 1 {
                        break Err(IconvError::TooBig);
                    }
                    dst[0] = if ch > 0x7F { UNKNOWN_ASCII } else { ch as u8 };
                    1
                }
                Encoding::Latin1 => {
                    if dstlen < 1 {
                        break Err(IconvError::TooBig);
                    }
                    dst[0] = if ch > 0xFF { UNKNOWN_ASCII } else { ch as u8 };
                    1
                }
                Encoding::Utf8 => {
                    // RFC 3629
                    if ch > 0x10FFFF {
                        ch = UNKNOWN_UNICODE;
                    }
                    if ch <= 0x7F {
                        if dstlen < 1 {
                            break Err(IconvError::TooBig);
                        }
                        dst[0] = ch as u8;
                        1
                    } else if ch <= 0x7FF {
                        if dstlen < 2 {
                            break Err(IconvError::TooBig);
                        }
                        dst[0] = 0xC0 | ((ch >> 6) & 0x1F) as u8;
                        dst[1] = 0x80 | (ch & 0x3F) as u8;
                        2
                    } else if ch <= 0xFFFF {
                        if dstlen < 3 {
                            break Err(IconvError::TooBig);
                        }
                        dst[0] = 0xE0 | ((ch >> 12) & 0x0F) as u8;
                        dst[1] = 0x80 | ((ch >> 6) & 0x3F) as u8;
                        dst[2] = 0x80 | (ch & 0x3F) as u8;
                        3
                    } else {
                        if dstlen < 4 {
                            break Err(IconvError::TooBig);
                        }
                        dst[0] = 0xF0 | ((ch >> 18) & 0x07) as u8;
                        dst[1] = 0x80 | ((ch >> 12) & 0x3F) as u8;
                        dst[2] = 0x80 | ((ch >> 6) & 0x3F) as u8;
                        dst[3] = 0x80 | (ch & 0x3F) as u8;
                        4
                    }
                }
                Encoding::Utf16Be | Encoding::Utf16Le => {
                    // RFC 2781
                    let be = self.dst_fmt == Encoding::Utf16Be;
                    let put = |d: &mut [u8], w: u16| {
                        d.copy_from_slice(&if be { w.to_be_bytes() } else { w.to_le_bytes() })
                    };
                    if ch > 0x10FFFF {
                        ch = UNKNOWN_UNICODE;
                    }
                    if ch < 0x10000 {
                        if dstlen < 2 {
                            break Err(IconvError::TooBig);
                        }
                        put(&mut dst[0..2], ch as u16);
                        2
                    } else {
                        if dstlen < 4 {
                            break Err(IconvError::TooBig);
                        }
                        ch -= 0x10000;
                        let w1 = 0xD800 | ((ch >> 10) & 0x3FF) as u16;
                        let w2 = 0xDC00 | (ch & 0x3FF) as u16;
                        put(&mut dst[0..2], w1);
                        put(&mut dst[2..4], w2);
                        4
                    }
                }
                Encoding::Ucs2Be | Encoding::Ucs2Le => {
                    if ch > 0xFFFF {
                        ch = UNKNOWN_UNICODE;
                    }
                    if dstlen < 2 {
                        break Err(IconvError::TooBig);
                    }
                    let b = if self.dst_fmt == Encoding::Ucs2Be {
                        (ch as u16).to_be_bytes()
                    } else {
                        (ch as u16).to_le_bytes()
                    };
                    dst[..2].copy_from_slice(&b);
                    2
                }
                Encoding::Utf32Be | Encoding::Ucs4Be | Encoding::Utf32Le | Encoding::Ucs4Le => {
                    if matches!(self.dst_fmt, Encoding::Utf32Be | Encoding::Utf32Le)
                        && ch > 0x10FFFF
                    {
                        ch = UNKNOWN_UNICODE;
                    }
                    if ch > 0x7FFFFFFF {
                        ch = UNKNOWN_UNICODE;
                    }
                    if dstlen < 4 {
                        break Err(IconvError::TooBig);
                    }
                    let b = if matches!(self.dst_fmt, Encoding::Utf32Be | Encoding::Ucs4Be) {
                        ch.to_be_bytes()
                    } else {
                        ch.to_le_bytes()
                    };
                    dst[..4].copy_from_slice(&b);
                    4
                }
                Encoding::Utf16 | Encoding::Utf32 => {
                    unreachable!("byte order resolved before encoding")
                }
            };
            dst_pos += written;

            // Update state
            *inbuf = src;
            committed_dst = dst_pos;
            total += 1;
        };

        let out = std::mem::take(outbuf);
        *outbuf = &mut out[committed_dst..];
        result
    }
}

/// Convert a whole buffer from `fromcode` to `tocode` (empty names mean
/// UTF-8). Translation of `SDL_iconv_string()`; the result does not carry
/// upstream's four trailing zero bytes.
pub fn iconv_string(tocode: &str, fromcode: &str, inbuf: &[u8]) -> Result<Vec<u8>> {
    let tocode = if tocode.is_empty() { "UTF-8" } else { tocode };
    let fromcode = if fromcode.is_empty() {
        "UTF-8"
    } else {
        fromcode
    };
    let mut cd = Iconv::open(tocode, fromcode)?;

    let mut stringsize = inbuf.len();
    let mut string = vec![0u8; stringsize];
    let mut written = 0usize;
    let mut input = inbuf;

    while !input.is_empty() {
        let oldinbytesleft = input.len();
        let mut out: &mut [u8] = &mut string[written..];
        let before = out.len();
        let ret = cd.convert(&mut input, &mut out);
        written += before - out.len();
        match ret {
            Err(IconvError::TooBig) => {
                stringsize *= 2;
                string.resize(stringsize, 0);
                continue;
            }
            Err(IconvError::IllegalSequence) => {
                // Try skipping some input data - not perfect, but...
                input = &input[1..];
            }
            Err(IconvError::Incomplete) => {
                // We can't continue...
                input = &[];
            }
            Ok(_) => {}
        }
        // Avoid infinite loops when nothing gets converted
        if oldinbytesleft == input.len() {
            break;
        }
    }
    string.truncate(written);
    Ok(string)
}

/// UTF-8 to the locale's encoding. Translation of `SDL_iconv_utf8_locale()`.
pub fn utf8_to_locale(s: &str) -> Result<Vec<u8>> {
    iconv_string("", "UTF-8", s.as_bytes())
}

/// UTF-8 to UCS-2 (native byte order units). Translation of `SDL_iconv_utf8_ucs2()`.
pub fn utf8_to_ucs2(s: &str) -> Result<Vec<u16>> {
    let bytes = iconv_string("UCS-2-INTERNAL", "UTF-8", s.as_bytes())?;
    Ok(bytes
        .chunks_exact(2)
        .map(|c| u16::from_ne_bytes([c[0], c[1]]))
        .collect())
}

/// UTF-8 to UCS-4 (native byte order units). Translation of `SDL_iconv_utf8_ucs4()`.
pub fn utf8_to_ucs4(s: &str) -> Result<Vec<u32>> {
    let bytes = iconv_string("UCS-4-INTERNAL", "UTF-8", s.as_bytes())?;
    Ok(bytes
        .chunks_exact(4)
        .map(|c| u32::from_ne_bytes([c[0], c[1], c[2], c[3]]))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conv(to: &str, from: &str, input: &[u8]) -> Vec<u8> {
        iconv_string(to, from, input).unwrap()
    }

    #[test]
    fn names() {
        assert_eq!(Encoding::from_name("utf-8"), Some(Encoding::Utf8));
        assert_eq!(Encoding::from_name("Iso-8859-1"), Some(Encoding::Latin1));
        assert_eq!(Encoding::from_name("UCS-2"), Some(Encoding::Ucs2Be));
        assert_eq!(Encoding::from_name("EBCDIC"), None);
        assert!(Iconv::open("UTF-8", "KOI8-R").is_err());
    }

    #[test]
    fn round_trips() {
        let s = "Grüße, 世界 😀";
        let u16be = conv("UTF-16BE", "UTF-8", s.as_bytes());
        let expect: Vec<u8> = s.encode_utf16().flat_map(|w| w.to_be_bytes()).collect();
        assert_eq!(u16be, expect);
        assert_eq!(conv("UTF-8", "UTF-16BE", &u16be), s.as_bytes());

        let u32le = conv("UTF-32LE", "UTF-8", s.as_bytes());
        let expect: Vec<u8> = s.chars().flat_map(|c| (c as u32).to_le_bytes()).collect();
        assert_eq!(u32le, expect);
        assert_eq!(conv("UTF-8", "UTF-32LE", &u32le), s.as_bytes());

        assert_eq!(utf8_to_ucs4("a😀").unwrap(), vec!['a' as u32, 0x1F600]);
        assert_eq!(
            utf8_to_ucs2("a😀").unwrap(),
            vec!['a' as u16, 0xFFFD],
            "UCS-2 can't hold astral planes"
        );
    }

    #[test]
    fn replacement_rules() {
        // ASCII/Latin-1 outputs use '?'
        assert_eq!(conv("ASCII", "UTF-8", "é!".as_bytes()), b"?!");
        assert_eq!(conv("ISO-8859-1", "UTF-8", "é€".as_bytes()), [0xE9, b'?']);
        // ASCII input masks the high bit
        assert_eq!(conv("UTF-8", "ASCII", &[0xC1]), b"A");
        // Overlong and surrogate UTF-8 input become U+FFFD
        assert_eq!(conv("UTF-8", "UTF-8", &[0xC0, 0x80]), "\u{FFFD}".as_bytes());
        assert_eq!(
            conv("UTF-8", "UTF-8", &[0xED, 0xA0, 0x80]),
            "\u{FFFD}".as_bytes()
        );
        // A lone low surrogate in UTF-16
        assert_eq!(
            conv("UTF-8", "UTF-16LE", &[0x00, 0xDC]),
            "\u{FFFD}".as_bytes()
        );
    }

    #[test]
    fn source_byte_order_marks() {
        // The BOM picks the byte order (and, as upstream, is itself decoded).
        let want = "\u{FEFF}Aé".as_bytes();
        assert_eq!(
            conv("UTF-8", "UTF-16", &[0xFF, 0xFE, 0x41, 0x00, 0xE9, 0x00]),
            want
        );
        assert_eq!(
            conv("UTF-8", "UTF-16", &[0xFE, 0xFF, 0x00, 0x41, 0x00, 0xE9]),
            want
        );
        assert_eq!(
            conv(
                "UTF-8",
                "UTF-32",
                &[0xFF, 0xFE, 0, 0, 0x41, 0, 0, 0, 0xE9, 0, 0, 0]
            ),
            want
        );
        assert_eq!(
            conv(
                "UTF-8",
                "UTF-32",
                &[0, 0, 0xFE, 0xFF, 0, 0, 0, 0x41, 0, 0, 0, 0xE9]
            ),
            want
        );

        let mut cd = Iconv::open("UTF-8", "UTF-16").unwrap();
        let mut inbuf: &[u8] = &[0xFF, 0xFE, 0x41, 0x00];
        let mut storage = [0u8; 8];
        let mut outbuf: &mut [u8] = &mut storage;
        assert_eq!(cd.convert(&mut inbuf, &mut outbuf), Ok(2));
        assert_eq!(cd.source(), Encoding::Utf16Le);
    }

    #[test]
    fn errors_and_progress() {
        let mut cd = Iconv::new(Encoding::Utf8, Encoding::Utf16Le);
        let input = [0x41, 0x00, 0x3D, 0xD8]; // 'A' then a truncated surrogate pair
        let mut inbuf: &[u8] = &input;
        let mut storage = [0u8; 8];
        let mut outbuf: &mut [u8] = &mut storage;
        assert_eq!(
            cd.convert(&mut inbuf, &mut outbuf),
            Err(IconvError::Incomplete)
        );
        assert_eq!(inbuf, &[0x3D, 0xD8]);
        assert_eq!(outbuf.len(), 7);
        assert_eq!(storage[0], b'A');

        let mut cd = Iconv::new(Encoding::Utf8, Encoding::Latin1);
        let mut inbuf: &[u8] = &[0xE9, 0xE9];
        let mut storage = [0u8; 3];
        let mut outbuf: &mut [u8] = &mut storage;
        assert_eq!(cd.convert(&mut inbuf, &mut outbuf), Err(IconvError::TooBig));
        assert_eq!(inbuf, &[0xE9], "the first character went through");
        assert_eq!(outbuf.len(), 1);
        assert_eq!(&storage[..2], "é".as_bytes());
        let mut empty: &mut [u8] = &mut [];
        assert_eq!(cd.convert(&mut inbuf, &mut empty), Err(IconvError::TooBig));

        // UTF-16 output gets a native-order BOM when it fits with the first character...
        let mut cd = Iconv::open("UTF-16", "UTF-8").unwrap();
        let mut inbuf: &[u8] = b"AB";
        let mut storage = [0u8; 8];
        let mut outbuf: &mut [u8] = &mut storage;
        assert_eq!(cd.convert(&mut inbuf, &mut outbuf), Ok(2));
        assert_eq!(&storage[..2], &0xFEFFu16.to_ne_bytes());
        assert_eq!(&storage[2..4], &('A' as u16).to_ne_bytes());
        assert_eq!(cd.destination(), Encoding::UTF16_NATIVE);
        // ...and is kept when the first character doesn't fit (upstream drops
        // it, and SDL_iconv_string() hits that whenever its buffer first grows
        // past the BOM but not past the first character).
        let mut cd = Iconv::open("UTF-16", "UTF-8").unwrap();
        let mut inbuf: &[u8] = b"A";
        let mut storage = [0u8; 3];
        let mut outbuf: &mut [u8] = &mut storage;
        assert_eq!(cd.convert(&mut inbuf, &mut outbuf), Err(IconvError::TooBig));
        assert_eq!(outbuf.len(), 1, "the BOM was committed");
        assert_eq!(inbuf, b"A");
        assert_eq!(&storage[..2], &0xFEFFu16.to_ne_bytes());
        let out = conv("UTF-16", "UTF-8", b"A\0");
        assert_eq!(
            out,
            [
                &0xFEFFu16.to_ne_bytes()[..],
                &('A' as u16).to_ne_bytes()[..],
                &[0, 0][..]
            ]
            .concat()
        );
        let out = conv("UTF-32", "UTF-8", b"A");
        assert_eq!(
            out,
            [
                &0xFEFFu32.to_ne_bytes()[..],
                &('A' as u32).to_ne_bytes()[..]
            ]
            .concat()
        );
    }
}
