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
//!   [`load_texture`] and friends upload the result to a renderer, and
//!   [`load_gpu_texture`] and friends to a GPU texture through a copy pass.
//! * Detection: an `is_*` function for every format SDL_image knows, each
//!   leaving the stream where it was.
//! * Decoders: BMP, ICO and CUR, GIF (still images), JPEG and PNG (through
//!   the stb_image translation in `sdl3`, as upstream's stb backend), LBM
//!   (IFF PBM and ILBM, EHB and HAM), PCX, PNM (PBM/PGM/PPM), QOI, SVG
//!   (through translations of the bundled NanoSVG parser and rasterizer,
//!   also at a chosen size with [`load_sized_svg_io`]), TGA, XCF (GIMP),
//!   XPM (also from arrays of strings) and XV thumbnails.
//! * Savers: [`save`] and [`save_typed_io`] pick the format from a file
//!   extension; BMP, ICO, CUR, GIF, JPEG (tiny_jpeg), PNG (miniz, in
//!   `sdl3`) and TGA.
//! * Animations: [`load_animation`] and friends read whole [`Animation`]s
//!   (GIF and ANI cursors, or any still image as one frame), and
//!   [`save_animation`] writes them; [`AnimationDecoder`] and
//!   [`AnimationEncoder`] work frame by frame with timebases and metadata,
//!   and [`create_animated_cursor`] makes a cursor from an animation.
//!
//! Not translated yet: the WebP, AVIF, TIFF and JPEG XL decoders, and the
//! APNG, animated WebP and AVIF animation decoders and encoders. Their
//! detectors are here; [`load_io`] reports them as an unsupported image
//! format, and the animation API with upstream's messages for a build
//! without them.
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
mod anim_decoder;
mod anim_encoder;
mod avif;
mod bmp;
mod gif;
mod gpu;
mod img;
mod jpg;
mod jxl;
mod lbm;
mod nanosvg;
mod nanosvgrast;
mod pcx;
mod png;
mod pnm;
mod qoi;
mod qsort;
mod stb;
mod svg;
mod tga;
mod tif;
mod tiny_jpeg;
mod util;
mod webp;
mod xcf;
mod xmlman;
mod xpm;
mod xv;

pub use ani::is_ani;
pub use anim_decoder::{
    load_ani_animation_io, load_apng_animation_io, load_avif_animation_io, load_gif_animation_io,
    load_webp_animation_io, AnimationDecoder, AnimationDecoderStatus,
    PROP_ANIMATION_DECODER_CREATE_AVIF_ALLOW_INCREMENTAL_BOOLEAN,
    PROP_ANIMATION_DECODER_CREATE_AVIF_ALLOW_PROGRESSIVE_BOOLEAN,
    PROP_ANIMATION_DECODER_CREATE_AVIF_MAX_THREADS_NUMBER,
    PROP_ANIMATION_DECODER_CREATE_FILENAME_STRING,
    PROP_ANIMATION_DECODER_CREATE_GIF_NUM_COLORS_NUMBER,
    PROP_ANIMATION_DECODER_CREATE_GIF_TRANSPARENT_COLOR_INDEX_NUMBER,
    PROP_ANIMATION_DECODER_CREATE_IOSTREAM_AUTOCLOSE_BOOLEAN,
    PROP_ANIMATION_DECODER_CREATE_IOSTREAM_POINTER,
    PROP_ANIMATION_DECODER_CREATE_TIMEBASE_DENOMINATOR_NUMBER,
    PROP_ANIMATION_DECODER_CREATE_TIMEBASE_NUMERATOR_NUMBER,
    PROP_ANIMATION_DECODER_CREATE_TYPE_STRING, PROP_METADATA_AUTHOR_STRING,
    PROP_METADATA_COPYRIGHT_STRING, PROP_METADATA_CREATION_TIME_STRING,
    PROP_METADATA_DESCRIPTION_STRING, PROP_METADATA_FRAME_COUNT_NUMBER,
    PROP_METADATA_IGNORE_PROPS_BOOLEAN, PROP_METADATA_LOOP_COUNT_NUMBER,
    PROP_METADATA_TITLE_STRING,
};
pub use anim_encoder::{
    save_ani_animation_io, save_apng_animation_io, save_avif_animation_io, save_gif_animation_io,
    save_webp_animation_io, AnimationEncoder,
    PROP_ANIMATION_ENCODER_CREATE_AVIF_KEYFRAME_INTERVAL_NUMBER,
    PROP_ANIMATION_ENCODER_CREATE_AVIF_MAX_THREADS_NUMBER,
    PROP_ANIMATION_ENCODER_CREATE_FILENAME_STRING,
    PROP_ANIMATION_ENCODER_CREATE_GIF_USE_LUT_BOOLEAN,
    PROP_ANIMATION_ENCODER_CREATE_IOSTREAM_AUTOCLOSE_BOOLEAN,
    PROP_ANIMATION_ENCODER_CREATE_IOSTREAM_POINTER, PROP_ANIMATION_ENCODER_CREATE_QUALITY_NUMBER,
    PROP_ANIMATION_ENCODER_CREATE_TIMEBASE_DENOMINATOR_NUMBER,
    PROP_ANIMATION_ENCODER_CREATE_TIMEBASE_NUMERATOR_NUMBER,
    PROP_ANIMATION_ENCODER_CREATE_TYPE_STRING,
};
pub use avif::is_avif;
pub use bmp::{
    is_bmp, is_cur, is_ico, load_bmp_io, load_cur_io, load_ico_io, save_bmp, save_bmp_io, save_cur,
    save_cur_io, save_ico, save_ico_io,
};
pub use gif::{is_gif, load_gif_io, save_gif, save_gif_io};
pub use gpu::{load_gpu_texture, load_gpu_texture_io, load_gpu_texture_typed_io};
pub use img::{
    clipboard_image, create_animated_cursor, load, load_animation, load_animation_io,
    load_animation_typed_io, load_io, load_texture, load_texture_io, load_texture_typed_io,
    load_typed_io, save, save_animation, save_animation_typed_io, save_typed_io, version,
    Animation,
};
pub use jpg::{is_jpg, load_jpg_io, save_jpg, save_jpg_io};
pub use jxl::is_jxl;
pub use lbm::{is_lbm, load_lbm_io};
pub use pcx::{is_pcx, load_pcx_io};
pub use png::{is_png, load_png_io, save_png, save_png_io};
pub use pnm::{is_pnm, load_pnm_io};
pub use qoi::{is_qoi, load_qoi_io};
pub use svg::{is_svg, load_sized_svg_io, load_svg_io};
pub use tga::{load_tga_io, save_tga, save_tga_io};
pub use tif::is_tif;
pub use webp::{is_webp, load_webp_io};
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
