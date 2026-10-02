// Rust translation of include/SDL3/SDL_version.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The SDL version this crate translates.

use std::fmt;

/// An SDL version number. Translation of `SDL_VERSIONNUM()` and friends.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Version {
    pub major: u16,
    pub minor: u16,
    pub micro: u16,
}

impl Version {
    /// The version of SDL this crate translates (`SDL_MAJOR_VERSION` etc.).
    pub const CURRENT: Version = Version {
        major: 3,
        minor: 5,
        micro: 0,
    };

    /// The upstream git revision this translation was made from (`SDL_REVISION`).
    pub const REVISION: &'static str = "SDL-3.5.0-ea7a2dabfd1ab6b0a5720568ffd207f5825e47f1";

    pub const fn new(major: u16, minor: u16, micro: u16) -> Self {
        Version {
            major,
            minor,
            micro,
        }
    }

    /// Pack into SDL's single-integer form: `1.2.3` → `1002003`. Translation of `SDL_VERSIONNUM()`.
    pub const fn to_number(self) -> i32 {
        self.major as i32 * 1_000_000 + self.minor as i32 * 1000 + self.micro as i32
    }

    /// Unpack SDL's single-integer form. Translation of `SDL_VERSIONNUM_MAJOR/MINOR/MICRO()`.
    pub const fn from_number(version: i32) -> Self {
        Version {
            major: (version / 1_000_000) as u16,
            minor: ((version / 1000) % 1000) as u16,
            micro: (version % 1000) as u16,
        }
    }

    /// Translation of `SDL_VERSION_ATLEAST()`.
    pub const fn at_least(self, major: u16, minor: u16, micro: u16) -> bool {
        self.to_number() >= Version::new(major, minor, micro).to_number()
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.micro)
    }
}

/// The version of SDL this crate translates. Translation of `SDL_GetVersion()`.
pub const fn version() -> Version {
    Version::CURRENT
}

/// The upstream revision this translation was made from. Translation of `SDL_GetRevision()`.
pub const fn revision() -> &'static str {
    Version::REVISION
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_compare() {
        let v = Version::new(3, 5, 0);
        assert_eq!(v.to_number(), 3_005_000);
        assert_eq!(Version::from_number(3_005_000), v);
        assert!(v.at_least(3, 0, 0));
        assert!(!v.at_least(4, 0, 0));
        assert!(Version::new(3, 4, 9) < v);
        assert_eq!(v.to_string(), "3.5.0");
        assert_eq!(version(), Version::CURRENT);
        assert!(revision().starts_with("SDL-3.5.0"));
    }
}
