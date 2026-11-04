// Rust translation of src/events/default_cursor.h from Simple DirectMedia
// Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Default cursor - it happens to be the Mac cursor, but could be anything
//! (the KMS/DRM driver's: `KMSDRM_CreateDefaultCursor()`).
//!
//! Added a real MacOS cursor, at the request of Luc-Olivier de Charrière
//! (`USE_MACOS_CURSOR`, which upstream defines: its other bitmap isn't
//! translated).

pub(crate) const DEFAULT_CWIDTH: i32 = 16;
pub(crate) const DEFAULT_CHEIGHT: i32 = 16;
pub(crate) const DEFAULT_CHOTX: i32 = 0;
pub(crate) const DEFAULT_CHOTY: i32 = 0;

#[rustfmt::skip]
pub(crate) const DEFAULT_CDATA: [u8; 32] = [
    0x00, 0x00,
    0x40, 0x00,
    0x60, 0x00,
    0x70, 0x00,
    0x78, 0x00,
    0x7C, 0x00,
    0x7E, 0x00,
    0x7F, 0x00,
    0x7F, 0x80,
    0x7C, 0x00,
    0x6C, 0x00,
    0x46, 0x00,
    0x06, 0x00,
    0x03, 0x00,
    0x03, 0x00,
    0x00, 0x00,
];

#[rustfmt::skip]
pub(crate) const DEFAULT_CMASK: [u8; 32] = [
    0xC0, 0x00,
    0xE0, 0x00,
    0xF0, 0x00,
    0xF8, 0x00,
    0xFC, 0x00,
    0xFE, 0x00,
    0xFF, 0x00,
    0xFF, 0x80,
    0xFF, 0xC0,
    0xFF, 0xE0,
    0xFE, 0x00,
    0xEF, 0x00,
    0xCF, 0x00,
    0x87, 0x80,
    0x07, 0x80,
    0x03, 0x00,
];
