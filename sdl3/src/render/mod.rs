// Rust translation of src/render/ from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! 2D rendering.
//!
//! So far: the software renderer's drawing primitives.

pub(crate) mod software;

/// How texture coordinates outside [0, 1] are handled.
/// Translation of `SDL_TextureAddressMode`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum TextureAddressMode {
    /// Not a valid mode
    Invalid = -1,
    /// Wrapping is enabled if texture coordinates are outside [0, 1], this is the default
    #[default]
    Auto,
    /// Texture coordinates are clamped to the [0, 1] range
    Clamp,
    /// The texture is repeated (tiled)
    Wrap,
}
