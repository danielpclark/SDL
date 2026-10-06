// Rust translation of src/IMG.c from SDL_image.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! A simple library to load images of various formats as SDL surfaces: the
//! format table, loading by detection or by type, textures, and saving by
//! file extension.

use std::path::Path;

use sdl3::error::{Error, Result};
use sdl3::io::{IoStream, IoWhence};
use sdl3::render::{Renderer, Texture};
use sdl3::stdlib::string::strcasecmp;
use sdl3::video::{FlipMode, Surface};

use crate::{MAJOR_VERSION, MICRO_VERSION, MINOR_VERSION};

/* Limited by its encoding in SDL_VERSIONNUM */
#[allow(clippy::absurd_extreme_comparisons)]
const _: () = assert!(MAJOR_VERSION <= 10);
#[allow(clippy::absurd_extreme_comparisons)]
const _: () = assert!(MINOR_VERSION <= 999);
#[allow(clippy::absurd_extreme_comparisons)]
const _: () = assert!(MICRO_VERSION <= 999);

/// A format's detection function. `None` marks a magicless format, which
/// is only loaded when asked for by type.
type IsFn = fn(&mut IoStream<'_>) -> bool;
/// A format's loading function.
type LoadFn = fn(&mut IoStream<'_>) -> Result<Surface<'static>>;

/* Table of image detection and loading functions */
// (the formats this crate translates; the others are left out, as in an
// upstream build without them)
static SUPPORTED: &[(&str, Option<IsFn>, LoadFn)] = &[
    /* keep magicless formats first */
    ("TGA", None, crate::tga::load_tga_io),
    ("CUR", Some(crate::bmp::is_cur), crate::bmp::load_cur_io),
    ("ICO", Some(crate::bmp::is_ico), crate::bmp::load_ico_io),
    ("BMP", Some(crate::bmp::is_bmp), crate::bmp::load_bmp_io),
    ("GIF", Some(crate::gif::is_gif), crate::gif::load_gif_io),
    ("JPG", Some(crate::jpg::is_jpg), crate::jpg::load_jpg_io),
    ("PCX", Some(crate::pcx::is_pcx), crate::pcx::load_pcx_io),
    ("PNG", Some(crate::png::is_png), crate::png::load_png_io),
    ("PNM", Some(crate::pnm::is_pnm), crate::pnm::load_pnm_io), /* P[BGP]M share code */
    ("QOI", Some(crate::qoi::is_qoi), crate::qoi::load_qoi_io),
];

/// The version of SDL_image. Translation of `IMG_Version()`.
pub const fn version() -> sdl3::Version {
    crate::VERSION
}

/// The extension of a file name: what follows its last `.` (upstream's
/// `SDL_strrchr(file, '.') + 1`, so a dot in a directory name counts too).
fn file_extension(file: &Path) -> Option<String> {
    let name = file.to_string_lossy();
    name.rfind('.').map(|i| name[i + 1..].to_owned())
}

/// Load an image from a file. The format is detected from the contents,
/// or for a magicless format (TGA) taken from the file's extension.
/// Translation of `IMG_Load()`.
pub fn load(file: impl AsRef<Path>) -> Result<Surface<'static>> {
    let file = file.as_ref();
    let mut src = IoStream::from_file(file, "rb")?;
    /* The error message has been set in SDL_IOFromFile */

    let ext = file_extension(file);
    load_typed_io(&mut src, ext.as_deref())
}

/// Load an image from a data source, detecting its format.
/// Translation of `IMG_Load_IO()` (for compatibility).
pub fn load_io(src: &mut IoStream<'_>) -> Result<Surface<'static>> {
    load_typed_io(src, None)
}

/// Load an image from a data source, optionally specifying the type (a
/// file extension such as `"TGA"`, compared without case). Formats with a
/// magic number are detected whatever `type_` says; `type_` only selects
/// the magicless ones. Translation of `IMG_LoadTyped_IO()`.
pub fn load_typed_io(src: &mut IoStream<'_>, type_: Option<&str>) -> Result<Surface<'static>> {
    /* See whether or not this data source can handle seeking */
    if src.seek(0, IoWhence::Cur).is_err() {
        return Err(Error::new("Can't seek in this data source"));
    }

    /* Detect the type of image being loaded */
    for &(name, is, load) in SUPPORTED {
        if let Some(is) = is {
            if !is(src) {
                continue;
            }
        } else {
            /* magicless format */
            match type_ {
                Some(t) if strcasecmp(t, name).is_eq() => {}
                _ => continue,
            }
        }
        return load(src);
    }

    Err(Error::new("Unsupported image format"))
}

/// Load an image from a file into a texture of `renderer`.
/// Translation of `IMG_LoadTexture()`.
pub fn load_texture(renderer: &mut Renderer, file: impl AsRef<Path>) -> Result<Texture> {
    let mut surface = load(file)?;
    renderer.create_texture_from_surface(&mut surface)
}

/// Load an image from a data source into a texture of `renderer`.
/// Translation of `IMG_LoadTexture_IO()`.
pub fn load_texture_io(renderer: &mut Renderer, src: &mut IoStream<'_>) -> Result<Texture> {
    let mut surface = load_io(src)?;
    renderer.create_texture_from_surface(&mut surface)
}

/// Load an image of an optionally specified type from a data source into a
/// texture of `renderer`. Translation of `IMG_LoadTextureTyped_IO()`.
pub fn load_texture_typed_io(
    renderer: &mut Renderer,
    src: &mut IoStream<'_>,
    type_: Option<&str>,
) -> Result<Texture> {
    let mut surface = load_typed_io(src, type_)?;
    renderer.create_texture_from_surface(&mut surface)
}

/// Check that a surface can be saved: an indexed surface needs a palette.
/// Translation of `IMG_VerifyCanSaveSurface()`.
pub(crate) fn verify_can_save_surface(surface: &Surface<'_>) -> Result<()> {
    if surface.format().is_indexed() && surface.palette().is_none() {
        return Err(Error::new("Indexed surfaces must have a palette"));
    }
    Ok(())
}

/// Save a surface to a file, in the format named by the file's extension
/// (see [`save_typed_io`]). Translation of `IMG_Save()`.
pub fn save(surface: &mut Surface<'_>, file: impl AsRef<Path>) -> Result<()> {
    verify_can_save_surface(surface)?;

    let file = file.as_ref();
    if file.as_os_str().is_empty() {
        return Err(Error::invalid_param("file"));
    }

    let Some(type_) = file_extension(file) else {
        return Err(Error::new("Couldn't determine file type"));
    };
    // Skip the '.' in the file extension

    let mut dst = IoStream::from_file(file, "wb")?;

    let result = save_typed_io(surface, &mut dst, &type_);
    let closed = dst.close();
    result.and(closed)
}

/// Save a surface to a data source in the format named by `type_`, a file
/// extension compared without case: `"bmp"`, `"cur"`, `"ico"`, `"jpg"` or
/// `"jpeg"` (at quality 90), `"png"` or `"tga"`. AVIF, GIF and WebP saving
/// are not translated yet and report so. Translation of `IMG_SaveTyped_IO()`.
pub fn save_typed_io(surface: &mut Surface<'_>, dst: &mut IoStream<'_>, type_: &str) -> Result<()> {
    if type_.is_empty() {
        return Err(Error::invalid_param("type"));
    }

    let is = |name: &str| strcasecmp(type_, name).is_eq();
    if is("avif") {
        Err(Error::new("SDL_image built without AVIF save support"))
    } else if is("bmp") {
        crate::bmp::save_bmp_io(surface, dst)
    } else if is("cur") {
        crate::bmp::save_cur_io(surface, dst)
    } else if is("gif") {
        Err(Error::new("SDL_image built without GIF save support"))
    } else if is("ico") {
        crate::bmp::save_ico_io(surface, dst)
    } else if is("jpg") || is("jpeg") {
        crate::jpg::save_jpg_io(surface, dst, 90)
    } else if is("png") {
        crate::png::save_png_io(surface, dst)
    } else if is("tga") {
        crate::tga::save_tga_io(surface, dst)
    } else if is("webp") {
        Err(Error::new("SDL_image built without WEBP save support"))
    } else {
        Err(Error::new("Unsupported image format"))
    }
}

/// Load the first image on the clipboard: the data of the first `image/*`
/// MIME type that decodes. Translation of `IMG_GetClipboardImage()`.
pub fn clipboard_image() -> Result<Surface<'static>> {
    let mut surface = None;

    if let Ok(mime_types) = sdl3::video::clipboard::clipboard_mime_types() {
        for mime_type in &mime_types {
            if surface.is_some() {
                break;
            }
            if mime_type.starts_with("image/") {
                if let Ok(Some(data)) = sdl3::video::clipboard::clipboard_data(mime_type) {
                    let mut src = IoStream::from_const_mem(&data);
                    surface = load_io(&mut src).ok();
                }
            }
        }
    }
    surface.ok_or_else(|| Error::new("No clipboard image available"))
}

/// Convert a duration between time bases, rounding the start and end
/// points so that consecutive durations add up. Translation of
/// `IMG_TimebaseDuration()`.
#[allow(dead_code)] // for the animation API
pub(crate) fn timebase_duration(
    pts: u64,
    duration: u64,
    src_numerator: u64,
    src_denominator: u64,
    dst_numerator: u64,
    dst_denominator: u64,
) -> u64 {
    let a = ((((pts.wrapping_add(duration)).wrapping_mul(2)).wrapping_add(1))
        .wrapping_mul(src_numerator)
        .wrapping_mul(dst_denominator))
        / (2u64
            .wrapping_mul(src_denominator)
            .wrapping_mul(dst_numerator));
    let b = (((pts.wrapping_mul(2)).wrapping_add(1))
        .wrapping_mul(src_numerator)
        .wrapping_mul(dst_denominator))
        / (2u64
            .wrapping_mul(src_denominator)
            .wrapping_mul(dst_numerator));
    a.wrapping_sub(b)
}

/// Apply an EXIF orientation (1 to 8) to a decoded image: flip, then
/// rotate clockwise. Translation of `IMG_ApplyOrientation()` (as built
/// without `ORIENTATION_USES_PROPERTIES`).
#[allow(dead_code)] // for the TIFF, AVIF and JPEG XL decoders still to come
pub(crate) fn apply_orientation(
    mut surface: Surface<'static>,
    orientation: i32,
) -> Result<Surface<'static>> {
    let mut rotation = 0.0f32;
    let mut flip = FlipMode::None;
    match orientation {
        1 => {
            // Normal (no rotation required)
        }
        2 => {
            // Mirror horizontal
            flip = FlipMode::Horizontal;
        }
        3 => {
            // Rotate 180
            rotation = 180.0;
        }
        4 => {
            // Mirror vertical
            flip = FlipMode::Vertical;
        }
        5 => {
            // Mirror horizontal and rotate 270 CW
            flip = FlipMode::Horizontal;
            rotation = 270.0;
        }
        6 => {
            // Rotate 90 CW
            rotation = 90.0;
        }
        7 => {
            // Mirror horizontal and rotate 90 CW
            flip = FlipMode::Horizontal;
            rotation = 90.0;
        }
        8 => {
            // Rotate 270 CW
            rotation = 270.0;
        }
        _ => {}
    }

    if flip != FlipMode::None {
        surface.flip(flip)?;
    }
    if rotation != 0.0 {
        surface = surface.rotate(rotation)?;
    }
    Ok(surface)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_matches_the_header() {
        assert_eq!(version().to_number(), 3_005_000);
        assert!(crate::VERSION.at_least(3, 0, 0));
        assert!(!crate::VERSION.at_least(3, 5, 1));
    }

    #[test]
    fn timebase_duration_rounds_consistently() {
        // 1/1000 s ticks to 1/100 s ticks, accumulating
        let mut pts = 0;
        let mut total = 0;
        for d in [33, 33, 34, 15, 85] {
            total += timebase_duration(pts, d, 1, 1000, 1, 100);
            pts += d;
        }
        assert_eq!(total, timebase_duration(0, pts, 1, 1000, 1, 100));
        assert_eq!(timebase_duration(0, 100, 1, 100, 1, 1000), 1000);
        assert_eq!(timebase_duration(0, 10, 1, 100, 1, 1000), 100);
    }

    #[test]
    fn file_extension_is_after_the_last_dot() {
        assert_eq!(file_extension(Path::new("a/b.tga")).as_deref(), Some("tga"));
        assert_eq!(
            file_extension(Path::new("a.dir/b")).as_deref(),
            Some("dir/b")
        );
        assert_eq!(file_extension(Path::new("noext")), None);
        assert_eq!(file_extension(Path::new("x.")).as_deref(), Some(""));
    }
}
