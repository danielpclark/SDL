// Rust translation of src/test/SDL_test_internal.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! What the test library's files share: the colors of the log messages.

use std::sync::atomic::{AtomicBool, Ordering};

/// Whether log messages are colored. Translation of `SDLTest_Color`, which
/// upstream defines in `SDL_test_common.c` (on unless the `NO_COLOR`
/// environment variable is set, off with `--no-color`).
static COLOR: AtomicBool = AtomicBool::new(true);

pub(crate) fn color() -> bool {
    COLOR.load(Ordering::Relaxed)
}

pub(crate) fn set_color(enabled: bool) {
    COLOR.store(enabled, Ordering::Relaxed);
}

pub(crate) const COLOR_RAW_RED: &str = "\x1b[0;31m";
pub(crate) const COLOR_RAW_GREEN: &str = "\x1b[0;32m";
pub(crate) const COLOR_RAW_YELLOW: &str = "\x1b[0;93m";
pub(crate) const COLOR_RAW_BLUE: &str = "\x1b[0;94m";
pub(crate) const COLOR_RAW_END: &str = "\x1b[0m";

/// Translation of `COLOR_RED`.
pub(crate) fn color_red() -> &'static str {
    if color() {
        COLOR_RAW_RED
    } else {
        ""
    }
}

/// Translation of `COLOR_GREEN`.
pub(crate) fn color_green() -> &'static str {
    if color() {
        COLOR_RAW_GREEN
    } else {
        ""
    }
}

/// Translation of `COLOR_YELLOW`.
pub(crate) fn color_yellow() -> &'static str {
    if color() {
        COLOR_RAW_YELLOW
    } else {
        ""
    }
}

/// Translation of `COLOR_BLUE`.
pub(crate) fn color_blue() -> &'static str {
    if color() {
        COLOR_RAW_BLUE
    } else {
        ""
    }
}

/// Translation of `COLOR_END`.
pub(crate) fn color_end() -> &'static str {
    if color() {
        COLOR_RAW_END
    } else {
        ""
    }
}

/// `SDL_isprint()` for the bytes the C code passes it.
pub(crate) fn isprint(c: u8) -> bool {
    (b' '..0x7F).contains(&c)
}

/// The text `SDL_vsnprintf()` leaves in a buffer of `size` bytes: at most
/// `size - 1` bytes (cut at a character boundary here, where C would cut
/// a UTF-8 sequence in two).
pub(crate) fn truncate(mut text: String, size: usize) -> String {
    let max = size.saturating_sub(1);
    if text.len() > max {
        let mut end = max;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    text
}

/// C's `%g` for a value: 6 significant digits, in fixed or exponent
/// notation by its exponent, without trailing zeros.
pub(crate) fn fmt_g(value: f64) -> String {
    const PRECISION: i32 = 6;
    if value.is_nan() {
        return if value.is_sign_negative() {
            "-nan"
        } else {
            "nan"
        }
        .to_owned();
    }
    if value.is_infinite() {
        return if value < 0.0 { "-inf" } else { "inf" }.to_owned();
    }
    if value == 0.0 {
        return if value.is_sign_negative() { "-0" } else { "0" }.to_owned();
    }
    // The exponent of the value rounded to the precision, as %e has it.
    let e = format!("{:.*e}", (PRECISION - 1) as usize, value);
    let (mantissa, exponent) = e.split_once('e').unwrap_or((&e, "0"));
    let exponent: i32 = exponent.parse().unwrap_or(0);
    let strip = |s: &str| -> String {
        if s.contains('.') {
            s.trim_end_matches('0').trim_end_matches('.').to_owned()
        } else {
            s.to_owned()
        }
    };
    if !(-4..PRECISION).contains(&exponent) {
        let sign = if exponent < 0 { '-' } else { '+' };
        format!("{}e{sign}{:02}", strip(mantissa), exponent.abs())
    } else {
        let decimals = (PRECISION - 1 - exponent) as usize;
        strip(&format!("{value:.decimals$}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn printf_g() {
        // What glibc's printf("%g") prints.
        for (value, expected) in [
            (0.0, "0"),
            (-0.0, "-0"),
            (1.0, "1"),
            (1.5, "1.5"),
            (100.0, "100"),
            (123456.0, "123456"),
            (1234567.0, "1.23457e+06"),
            (0.0001, "0.0001"),
            (0.00001, "1e-05"),
            (0.1 + 0.2, "0.3"),
            (59.94, "59.94"),
            (-2.5, "-2.5"),
            (999999.5, "1e+06"),
            (1e100, "1e+100"),
            (f64::INFINITY, "inf"),
        ] {
            assert_eq!(fmt_g(value), expected, "{value}");
        }
        assert_eq!(fmt_g(f32::MAX as f64), "3.40282e+38");
        assert_eq!(fmt_g(0.1f32 as f64), "0.1");
    }

    #[test]
    fn cut_at_char_boundary() {
        assert_eq!(truncate("abcdef".to_owned(), 4), "abc");
        assert_eq!(truncate("aé".to_owned(), 3), "a");
        assert_eq!(truncate("ab".to_owned(), 10), "ab");
        assert!(isprint(b' ') && isprint(b'~') && !isprint(0x7f) && !isprint(b'\n'));
    }
}
