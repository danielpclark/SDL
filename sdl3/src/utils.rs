// Rust translation of src/SDL_utils.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Small helpers shared across subsystems.

use std::sync::atomic::{AtomicU32, Ordering};

/// Translation of `SDL_CalculateGCD()`.
pub fn gcd(a: u32, b: u32) -> u32 {
    if b == 0 {
        return a;
    }
    gcd(b, a % b)
}

/// Best rational approximation of `x` with numerator and denominator ≤ 1000,
/// returned as `(numerator, denominator)`.
///
/// Algorithm adapted with thanks from John Cook's blog post:
/// <http://www.johndcook.com/blog/2010/10/20/best-rational-approximation>
///
/// Translation of `SDL_CalculateFraction()`. SDL uses this to express
/// refresh rates and pixel densities as fractions.
pub fn approximate_fraction(x: f32) -> (i32, i32) {
    const N: i32 = 1000;
    let (mut a, mut b) = (0i32, 1i32);
    let (mut c, mut d) = (1i32, 0i32);

    while b <= N && d <= N {
        let mediant = (a + c) as f32 / (b + d) as f32;
        if x == mediant {
            if b + d <= N {
                return (a + c, b + d);
            } else if d > b {
                return (c, d);
            } else {
                return (a, b);
            }
        } else if x > mediant {
            a += c;
            b += d;
        } else {
            c += a;
            d += b;
        }
    }
    if b > N {
        (c, d)
    } else {
        (a, b)
    }
}

static LAST_OBJECT_ID: AtomicU32 = AtomicU32::new(0);

/// Next unique, non-zero object id (translation of `SDL_GetNextObjectID()`).
pub(crate) fn next_object_id() -> u32 {
    let mut id = LAST_OBJECT_ID
        .fetch_add(1, Ordering::Relaxed)
        .wrapping_add(1);
    if id == 0 {
        id = LAST_OBJECT_ID
            .fetch_add(1, Ordering::Relaxed)
            .wrapping_add(1);
    }
    id
}

/// Translation of `SDL_startswith()`.
#[allow(dead_code)] // (used by the platform layers)
pub(crate) fn startswith(string: Option<&str>, prefix: Option<&str>) -> bool {
    matches!((string, prefix), (Some(s), Some(p)) if s.starts_with(p))
}

/// Translation of `SDL_endswith()`.
#[allow(dead_code)] // (used by the platform layers)
pub(crate) fn endswith(string: Option<&str>, suffix: Option<&str>) -> bool {
    let suffix = suffix.unwrap_or("");
    !suffix.is_empty() && string.unwrap_or("").ends_with(suffix)
}

/// Translation of `PrefixMatch()`.
#[allow(dead_code)] // (used by the platform drivers)
fn prefix_match(a: &[u8], b: &[u8]) -> usize {
    // Fixes the "HORI HORl Taiko No Tatsujin Drum Controller"
    if a.starts_with(b"HORI ") && b.starts_with(b"HORl ") {
        return 5;
    }
    a.iter()
        .zip(b)
        .take_while(|(x, y)| x.eq_ignore_ascii_case(y))
        .count()
}

/// A standardized name for a device, from its USB IDs and the names the
/// system reports, or `default_name`. Translation of `SDL_CreateDeviceName()`.
#[allow(dead_code)] // (used by the platform drivers)
pub(crate) fn create_device_name(
    vendor: u16,
    product: u16,
    vendor_name: Option<&str>,
    product_name: Option<&str>,
    default_name: Option<&str>,
) -> Option<String> {
    use crate::joystick::gamepad::GamepadType;

    const REPLACEMENTS: [(&str, &str); 15] = [
        ("(Standard system devices) ", ""),
        ("8BitDo Tech Ltd", "8BitDo"),
        ("ASTRO Gaming", "ASTRO"),
        ("Bensussen Deutsch & Associates,Inc.(BDA)", "BDA"),
        (
            "Guangzhou Chicken Run Network Technology Co., Ltd.",
            "GameSir",
        ),
        ("HORI CO.,LTD.", "HORI"),
        ("HORI CO.,LTD", "HORI"),
        ("Mad Catz Inc.", "Mad Catz"),
        ("Nintendo Co., Ltd.", "Nintendo"),
        ("NVIDIA Corporation ", ""),
        ("Performance Designed Products", "PDP"),
        ("QANBA USA, LLC", "Qanba"),
        ("QANBA USA,LLC", "Qanba"),
        ("Voyetra Turtle Beach,Inc.", "Turtle Beach"),
        ("Unknown ", ""),
    ];

    let vendor_name = vendor_name.unwrap_or("").trim_start_matches(' ');
    let product_name = product_name.unwrap_or("").trim_start_matches(' ');

    let name = if !vendor_name.is_empty() && !product_name.is_empty() {
        format!("{vendor_name} {product_name}")
    } else if !product_name.is_empty() {
        product_name.to_string()
    } else if vendor != 0 || product != 0 {
        // Couldn't find a controller name, try to give it one based on device type
        match crate::joystick::gamepad_type_from_vidpid(vendor, product, None, true) {
            GamepadType::Xbox360 => "Xbox 360 Controller".to_string(),
            GamepadType::XboxOne => "Xbox One Controller".to_string(),
            GamepadType::Ps3 => "PS3 Controller".to_string(),
            GamepadType::Ps4 => "PS4 Controller".to_string(),
            GamepadType::Ps5 => "DualSense Wireless Controller".to_string(),
            GamepadType::NintendoSwitchPro => "Nintendo Switch Pro Controller".to_string(),
            GamepadType::Steam => "Steam Controller".to_string(),
            _ => format!("0x{vendor:04x}/0x{product:04x}"),
        }
    } else {
        default_name?.to_string()
    };
    let mut name = name.into_bytes();

    // Trim trailing whitespace
    while name.last() == Some(&b' ') {
        name.pop();
    }

    // Compress duplicate spaces
    let mut i = 0;
    while i + 1 < name.len() {
        if name[i] == b' ' && name[i + 1] == b' ' {
            name.remove(i);
        } else {
            i += 1;
        }
    }

    // Perform any manufacturer replacements
    for (prefix, replacement) in REPLACEMENTS {
        let prefixlen = prefix.len();
        if crate::stdlib::string::strncasecmp(&name, prefix, prefixlen) == std::cmp::Ordering::Equal
        {
            if replacement.len() <= prefixlen {
                name.splice(..prefixlen.min(name.len()), replacement.bytes());
            } else {
                // FIXME: Need to handle the expand case by reallocating the string
            }
            break;
        }
    }

    /* Remove duplicate manufacturer or product in the name
     * e.g. Razer Razer Raiju Tournament Edition Wired
     */
    let len = name.len();
    for i in 1..len.saturating_sub(1) {
        let mut matchlen = prefix_match(&name, &name[i..]);
        while matchlen > 0 {
            if name[matchlen] == b' ' || name[matchlen] == b'-' {
                name.drain(..=matchlen);
                break;
            }
            matchlen -= 1;
        }
        if matchlen > 0 {
            // We matched the manufacturer's name and removed it
            break;
        }
    }

    Some(String::from_utf8_lossy(&name).into_owned())
}

/// Decode URI escape sequences (`%XX`) in the first `len` bytes of `src`
/// (all of it when `len` is 0). An invalid escape is copied through as is.
/// Translation of `SDL_URIDecode()`.
// (used by the D-Bus code)
#[cfg_attr(not(unix), allow(dead_code))]
fn uri_decode(src: &[u8], len: usize) -> Vec<u8> {
    let len = if len == 0 {
        src.len()
    } else {
        len.min(src.len())
    };
    let mut dst = Vec::with_capacity(len);
    let mut decode: u8 = 0;
    let mut di = 0usize;
    let mut ri = 0usize;
    while ri < len && dst.len() < len {
        let c = src[ri];
        if di == 0 {
            // start decoding
            if c == b'%' {
                decode = 0;
                di += 1;
                ri += 1;
                continue;
            }
            // normal write
            dst.push(c);
        } else {
            let digit = match c {
                b'0'..=b'9' => c - b'0',
                b'a'..=b'f' => c - b'a' + 10,
                b'A'..=b'F' => c - b'A' + 10,
                _ => {
                    // not a hexadecimal
                    dst.extend_from_slice(&src[ri - di..=ri]);
                    di = 0;
                    ri += 1;
                    continue;
                }
            };
            // itsy bitsy magicsy
            decode |= digit << ((2 - di) * 4);
            if di == 2 {
                dst.push(decode);
                di = 0;
            } else {
                di += 1;
            }
        }
        ri += 1;
    }
    dst
}

/// The local path of a `file:` URI (or of a path with no scheme), with
/// escapes decoded, or `None` for another scheme or a remote host.
/// `file:///p`, `file:/p`, `file://localhost/p` and
/// `file://<this host>/p` are local. Translation of `SDL_URIToLocal()`.
// (used by the D-Bus code)
#[cfg_attr(not(unix), allow(dead_code))]
pub(crate) fn uri_to_local(uri: &str) -> Option<Vec<u8>> {
    let mut src = uri.as_bytes();
    let had_file_scheme = src.starts_with(b"file:/");
    if had_file_scheme {
        src = &src[6..]; // local file?
    } else if uri.contains(":/") {
        return None; // wrong scheme
    }
    let at = |s: &[u8], i: usize| s.get(i).copied().unwrap_or(0);

    let mut local = at(src, 0) != b'/' || (at(src, 0) != 0 && at(src, 1) == b'/');

    // Check the hostname, if present. RFC 3986 states that the hostname component of a URI is not case-sensitive.
    if !local && at(src, 0) == b'/' && at(src, 2) != b'/' {
        if let Some(end) = src[1..].iter().position(|&c| c == b'/').map(|p| p + 1) {
            let host = &src[1..end];
            #[cfg(unix)]
            {
                let mut hostname = [0u8; 257];
                // SAFETY: the buffer holds 255 bytes plus a terminator.
                if unsafe { libc::gethostname(hostname.as_mut_ptr().cast(), 255) } == 0 {
                    hostname[256] = 0;
                    let n = hostname.iter().position(|&c| c == 0).unwrap_or(256);
                    if host.eq_ignore_ascii_case(&hostname[..n]) {
                        src = &src[end + 1..];
                        local = true;
                    }
                }
            }
            if !local && host.eq_ignore_ascii_case(b"localhost") {
                src = &src[end + 1..];
                local = true;
            }
            if local {
                // (the slash after the host was skipped; decode from it)
                return Some(uri_decode(&uri.as_bytes()[uri.len() - src.len() - 1..], 0));
            }
        }
    }

    if local {
        // Convert URI escape sequences to real characters
        if at(src, 0) == b'/' {
            return Some(uri_decode(&src[1..], 0));
        }
        // FIXME (upstream): for a path without a "file:/" scheme, upstream
        // steps `src` back one byte before the start of the string and
        // decodes from there. Only the "file:" case has a byte to step back
        // to (the scheme's slash).
        if !had_file_scheme {
            return None;
        }
        return Some(uri_decode(&uri.as_bytes()[uri.len() - src.len() - 1..], 0));
    }
    None
}

/// Whether `uri` starts with a valid scheme: a letter, then letters,
/// digits, `+`, `-` or `.`, then `:`. Translation of `SDL_IsURI()`.
// (used by the D-Bus code)
#[cfg_attr(not(unix), allow(dead_code))]
pub(crate) fn is_uri(uri: &str) -> bool {
    /* A valid URI begins with a letter and is followed by any sequence of
     * letters, digits, '+', '.', or '-'.
     */
    let b = uri.as_bytes();

    // The first character of the scheme must be a letter.
    if !b.first().is_some_and(u8::is_ascii_alphabetic) {
        return false;
    }

    /* If the colon is found before encountering the end of the string or
     * any invalid characters, the scheme can be considered valid.
     */
    for (i, &c) in b.iter().enumerate() {
        if !(c.is_ascii_alphanumeric() || c == b'+' || c == b'-' || c == b'.') {
            return false;
        }
        if b.get(i + 1) == Some(&b':') {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_names() {
        let name = |v, p, vn, pn| create_device_name(v, p, vn, pn, Some("Controller"));
        assert_eq!(
            name(0, 0, Some("Razer"), Some("Razer Raiju Tournament Edition")).as_deref(),
            Some("Razer Raiju Tournament Edition")
        );
        assert_eq!(
            name(0, 0, Some("HORI CO.,LTD."), Some("Fighting  Stick ")).as_deref(),
            Some("HORI Fighting Stick")
        );
        assert_eq!(
            name(0x1234, 0x5678, None, None).as_deref(),
            Some("0x1234/0x5678")
        );
        assert_eq!(name(0, 0, None, None).as_deref(), Some("Controller"));
        assert_eq!(create_device_name(0, 0, None, None, None), None);
        assert_eq!(
            name(0, 0, Some(" Unknown "), Some("Pad")).as_deref(),
            Some("Pad")
        );
    }

    #[test]
    fn gcd_and_fraction() {
        assert_eq!(gcd(1_000_000_000, 1000), 1000);
        assert_eq!(gcd(12, 18), 6);
        assert_eq!(approximate_fraction(0.5), (1, 2));
        assert_eq!(approximate_fraction(0.75), (3, 4));
        let (n, d) = approximate_fraction(59.94);
        assert!(((n as f32 / d as f32) - 59.94).abs() < 0.01);
    }

    #[test]
    fn ids_unique_nonzero() {
        let a = next_object_id();
        let b = next_object_id();
        assert_ne!(a, 0);
        assert_ne!(a, b);
    }

    #[test]
    fn uris() {
        assert!(is_uri("https://libsdl.org"));
        assert!(is_uri("x-scheme+1.2:rest"));
        assert!(!is_uri("1http://x"));
        assert!(!is_uri("/tmp/file"));
        assert!(!is_uri("no_colon"));
        assert!(!is_uri("bad char:x"));

        assert_eq!(uri_decode(b"a%20b%2Fc", 0), b"a b/c");
        assert_eq!(
            uri_decode(b"100%zz", 0),
            b"100%zz",
            "invalid escapes pass through"
        );
        assert_eq!(uri_decode(b"%41%4a%4A", 0), b"AJJ");

        let local = |u: &str| uri_to_local(u).map(|v| String::from_utf8(v).unwrap());
        assert_eq!(local("file:///tmp/a%20b").as_deref(), Some("/tmp/a b"));
        assert_eq!(local("file:/tmp/x").as_deref(), Some("/tmp/x"));
        assert_eq!(local("file://localhost/tmp/x").as_deref(), Some("/tmp/x"));
        assert_eq!(local("file://LOCALHOST/tmp/x").as_deref(), Some("/tmp/x"));
        assert_eq!(local("file://elsewhere.example/tmp/x"), None);
        assert_eq!(local("https://libsdl.org/"), None);
        assert_eq!(local("relative/path"), None);
    }
}
