// Rust translation of include/SDL3_image/SDL_image.h from SDL_image.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! # sdl3-image — SDL_image, translated to Rust
//!
//! The pure-Rust translation of [SDL_image](https://github.com/libsdl-org/SDL_image)
//! 3, "an example image loading library for use with SDL": image files in
//! many formats loaded as [`Surface`]s (or straight into renderer
//! [`Texture`](sdl3::render::Texture)s), and surfaces saved back to files.
//! It is a separate crate over [`sdl3`], as SDL_image is a separate library
//! over SDL.
//!
//! * Loading: [`load`], [`load_io`] and [`load_typed_io`] detect the format
//!   (or are told it, for the magicless TGA) and call the format's loader;
//!   [`load_texture`] and friends upload the result to a renderer.
//! * Detection: an `is_*` function for every format SDL_image knows, each
//!   leaving the stream where it was.
//! * Decoders: BMP, ICO and CUR, GIF (still images), JPEG and PNG (through
//!   the stb_image translation in `sdl3`, as upstream's stb backend), LBM
//!   (IFF PBM and ILBM, EHB and HAM), PCX, PNM (PBM/PGM/PPM), QOI, TGA,
//!   XCF (GIMP), XPM (also from arrays of strings) and XV thumbnails.
//! * Savers: [`save`] and [`save_typed_io`] pick the format from a file
//!   extension; BMP, ICO, CUR, JPEG (tiny_jpeg), PNG (miniz, in `sdl3`) and
//!   TGA.
//!
//! Not translated yet: the SVG, WebP, AVIF, TIFF and JPEG XL decoders, GIF
//! saving and the animation API. Their detectors are
//! here; [`load_io`] reports them as an unsupported image format, like an
//! upstream build without them.
//!
//! As in the [`sdl3`] crate, the implementation is a line-by-line
//! translation and the API is designed for Rust: the `closeio` flags are
//! gone (the caller owns the stream and drops it), errors are
//! [`Result`](sdl3::Result)s, and every item names the C symbol it
//! translates.
//!
//! [`Surface`]: sdl3::video::Surface

#![forbid(unsafe_code)]
#![warn(missing_debug_implementations)]

mod ani;
mod avif;
mod bmp;
mod gif;
mod img;
mod jpg;
mod jxl;
mod lbm;
mod pcx;
mod png;
mod pnm;
mod qoi;
mod stb;
mod svg;
mod tga;
mod tif;
mod tiny_jpeg;
mod util;
mod webp;
mod xcf;
mod xpm;
mod xv;

pub use ani::is_ani;
pub use avif::is_avif;
pub use bmp::{
    is_bmp, is_cur, is_ico, load_bmp_io, load_cur_io, load_ico_io, save_bmp, save_bmp_io, save_cur,
    save_cur_io, save_ico, save_ico_io,
};
pub use gif::{is_gif, load_gif_io};
pub use img::{
    clipboard_image, load, load_io, load_texture, load_texture_io, load_texture_typed_io,
    load_typed_io, save, save_typed_io, version,
};
pub use jpg::{is_jpg, load_jpg_io, save_jpg, save_jpg_io};
pub use jxl::is_jxl;
pub use lbm::{is_lbm, load_lbm_io};
pub use pcx::{is_pcx, load_pcx_io};
pub use png::{is_png, load_png_io, save_png, save_png_io};
pub use pnm::{is_pnm, load_pnm_io};
pub use qoi::{is_qoi, load_qoi_io};
pub use svg::is_svg;
pub use tga::{load_tga_io, save_tga, save_tga_io};
pub use tif::is_tif;
pub use webp::is_webp;
pub use xcf::{is_xcf, load_xcf_io};
pub use xpm::{is_xpm, load_xpm_io, read_xpm_from_array, read_xpm_from_array_to_rgb888};
pub use xv::{is_xv, load_xv_io};

/// The major version of SDL_image this crate translates.
/// Translation of `SDL_IMAGE_MAJOR_VERSION`.
pub const MAJOR_VERSION: u16 = 3;
/// The minor version. Translation of `SDL_IMAGE_MINOR_VERSION`.
pub const MINOR_VERSION: u16 = 5;
/// The micro (patch) version. Translation of `SDL_IMAGE_MICRO_VERSION`.
pub const MICRO_VERSION: u16 = 0;

/// The SDL_image version this crate translates. Translation of
/// `SDL_IMAGE_VERSION` (and, with [`Version::at_least`](sdl3::Version::at_least),
/// of `SDL_IMAGE_VERSION_ATLEAST()`).
pub const VERSION: sdl3::Version = sdl3::Version::new(MAJOR_VERSION, MINOR_VERSION, MICRO_VERSION);

/// The upstream SDL_image revision this translation was made from.
pub const REVISION: &str = "SDL_image-3.5.0-f7ec8b3631f5eeb4497bf4a581547409dfaa55c4";

#[cfg(test)]
mod tests;
