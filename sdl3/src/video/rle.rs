// Rust translation of src/video/SDL_RLEaccel.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! RLE acceleration of colorkeyed and alpha-blended surfaces.
//!
//! Not translated yet: this is the `SDL_HAVE_RLE`-disabled configuration,
//! where `SDL_SetSurfaceRLE()` only records the request and blits never
//! encode. Only the decoding half that the rest of the code calls is here.

use crate::video::blit::{COPY_RLE_ALPHAKEY, COPY_RLE_COLORKEY};
use crate::video::surface::{Pixels, Surface, SurfaceFlags, INTERNAL_SURFACE_RLEACCEL};

/// Translation of `SDL_UnRLESurface()`.
pub(crate) fn un_rle_surface(surface: &mut Surface<'_>) {
    if surface.internal_flags & INTERNAL_SURFACE_RLEACCEL != 0 {
        surface.internal_flags &= !INTERNAL_SURFACE_RLEACCEL;

        surface.map.flags &= !(COPY_RLE_COLORKEY | COPY_RLE_ALPHAKEY);

        if !surface.flags.contains(SurfaceFlags::PREALLOCATED) {
            surface.pixels = std::mem::replace(&mut surface.saved_pixels, Pixels::None);
        }

        surface.map.invalidate();

        surface.update_lock_flag();
    }
}
