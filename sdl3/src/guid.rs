// Rust translation of src/SDL_guid.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! 128-bit globally unique identifiers, with SDL's hex text form.

use std::fmt;
use std::str::FromStr;

/// A 128-bit identifier for an input device that identifies that device
/// across runs of SDL programs on the same platform. Translation of `SDL_GUID`.
///
/// `Display` renders the 32-character lowercase hex form (`SDL_GUIDToString`).
/// [`FromStr`] parses it strictly; [`Guid::parse_lossy`] has SDL's forgiving
/// behaviour (`SDL_StringToGUID`).
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Guid(pub [u8; 16]);

/// Error returned by [`Guid::from_str`] for malformed text.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ParseGuidError;

impl fmt::Display for ParseGuidError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("GUID must be 32 hexadecimal digits")
    }
}

impl std::error::Error for ParseGuidError {}

/// Returns the 4 bit nibble for a hex character (0 for anything else).
fn nibble(c: u8) -> u8 {
    match c {
        b'0'..=b'9' => c - b'0',
        b'A'..=b'F' => c - b'A' + 0x0a,
        b'a'..=b'f' => c - b'a' + 0x0a,
        // received an invalid character, and no real way to return an error
        _ => 0,
    }
}

impl Guid {
    /// The all-zero GUID.
    pub const ZERO: Guid = Guid([0; 16]);

    /// Parse the way `SDL_StringToGUID()` does: no error checking, pairs of
    /// hex digits are consumed left to right, invalid digits read as 0, an
    /// odd trailing digit is ignored, extra digits are ignored, and a short
    /// string leaves the remaining bytes zero.
    pub fn parse_lossy(text: &str) -> Guid {
        let bytes = text.as_bytes();
        let len = bytes.len() & !0x1; // Make sure it's even
        let mut guid = Guid::ZERO;
        for (out, pair) in guid.0.iter_mut().zip(bytes[..len].chunks_exact(2)) {
            *out = (nibble(pair[0]) << 4) | nibble(pair[1]);
        }
        guid
    }

    /// The raw bytes.
    pub const fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
}

impl fmt::Display for Guid {
    /// Translation of `SDL_GUIDToString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for b in self.0 {
            write!(f, "{b:02x}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for Guid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Guid({self})")
    }
}

impl FromStr for Guid {
    type Err = ParseGuidError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.len() != 32 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(ParseGuidError);
        }
        Ok(Guid::parse_lossy(s))
    }
}

impl From<[u8; 16]> for Guid {
    fn from(data: [u8; 16]) -> Self {
        Guid(data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let g = Guid([
            0x03, 0x00, 0x5e, 0x04, 0x4c, 0x05, 0x00, 0x00, 0xcc, 0x09, 0x00, 0x00, 0x11, 0x81,
            0x00, 0x00,
        ]);
        let s = g.to_string();
        assert_eq!(s, "03005e044c050000cc09000011810000");
        assert_eq!(s.parse::<Guid>().unwrap(), g);
        assert_eq!(s.to_uppercase().parse::<Guid>().unwrap(), g);
        assert_eq!(format!("{g:?}"), format!("Guid({s})"));
    }

    #[test]
    fn lossy_vs_strict() {
        let g = Guid::parse_lossy("abc");
        assert_eq!(g.0[0], 0xab);
        assert_eq!(g.0[1], 0);
        assert_eq!(Guid::parse_lossy(&"ff".repeat(20)).0, [0xff; 16]);
        assert_eq!(Guid::parse_lossy("zz").0[0], 0);
        assert_eq!("abc".parse::<Guid>(), Err(ParseGuidError));
        assert_eq!("zz".repeat(16).parse::<Guid>(), Err(ParseGuidError));
    }
}
