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
