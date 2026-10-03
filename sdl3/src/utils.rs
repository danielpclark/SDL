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
}
