// Rust translation of libtiff/tif_getimage.c from libtiff
// (https://gitlab.com/libtiff/libtiff, 4.7.2 as SDL_image's external/libtiff
// pins it).
// Copyright (c) 1991-1997 Sam Leffler
// Copyright (c) 1991-1997 Silicon Graphics, Inc.
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! TIFF Library
//!
//! Read and return a packed RGBA image.
//!
//! `TIFFReadRGBAImageOriented()` and what it uses (the strip and tile
//! readers `TIFFReadRGBAStrip()` and `TIFFReadRGBATile()` are left out:
//! SDL_image doesn't use them). The raster is the caller's buffer of
//! packed ABGR pixels (`uint32_t`s in host byte order) as bytes, a
//! [`Raster`]; the routines' pointers into it and into the decoded data
//! are indices, and a pixel or sample outside the buffers (which the C
//! would write or read out of bounds) is skipped or reads as 0. The
//! `BWmap` and `PALmap` tables of pointers are flat vectors of the
//! pixels each byte unpacks to.

#![allow(clippy::too_many_arguments)]

use super::tif_aux::{
    _tiff_multiply_ssize, tiff_get_field_defaulted, tiff_get_field_defaulted_u16,
    tiff_get_field_defaulted_u16_pair,
};
use super::tif_color::{
    tiff_cie_lab16_to_xyz, tiff_cie_lab_to_rgb_init, tiff_cie_lab_to_xyz, tiff_xyz_to_rgb,
    tiff_ycbcr_to_rgb, tiff_ycbcr_to_rgb_init,
};
use super::tif_dir::{tiff_get_field, tiff_get_field_int, tiff_set_field, Gv, Va};
use super::tif_error::{tiff_error_ext_r, tiff_warning_ext_r};
use super::tif_read::{
    _tiff_read_encoded_strip_and_alloc_buffer, _tiff_read_tile_and_alloc_buffer,
    tiff_read_encoded_strip, tiff_read_tile,
};
use super::tif_strip::{tiff_compute_strip, tiff_scanline_size, tiff_strip_size};
use super::tif_tile::{tiff_tile_row_size, tiff_tile_size};
use super::tiff::*;
use super::tiffio::{TIFFCIELabToRGB, TIFFDisplay, TIFFRGBValue, TIFFYCbCrToRGB};
use super::tiffiop::{try_vec, Tiff, TmSize, TIFF_TMSIZE_T_MAX};

const PHOTO_TAG: &str = "PhotometricInterpretation";

/*
 * Helper constants used in Orientation tag handling
 */
const FLIP_VERTICALLY: i32 = 0x01;
const FLIP_HORIZONTALLY: i32 = 0x02;

/*
 * Color conversion constants. We will define display types here.
 */

static DISPLAY_SRGB: TIFFDisplay = TIFFDisplay {
    d_mat: [
        /* XYZ -> luminance matrix */
        [3.2410f32, -1.5374f32, -0.4986f32],
        [-0.9692f32, 1.8760f32, 0.0416f32],
        [0.0556f32, -0.2040f32, 1.0570f32],
    ],
    d_YCR: 100.0f32,
    d_YCG: 100.0f32,
    d_YCB: 100.0f32, /* Light o/p for reference white */
    d_Vrwr: 255,
    d_Vrwg: 255,
    d_Vrwb: 255, /* Pixel values for ref. white */
    d_Y0R: 1.0f32,
    d_Y0G: 1.0f32,
    d_Y0B: 1.0f32, /* Residual light o/p for black pixel */
    d_gammaR: 2.4f32,
    d_gammaG: 2.4f32,
    d_gammaB: 2.4f32, /* Gamma values for the three guns */
};

/// The raster: packed ABGR pixels (`uint32_t`s in host byte order) in the
/// caller's bytes.
pub(crate) struct Raster<'r> {
    px: &'r mut [u8],
}

impl<'r> Raster<'r> {
    pub(crate) fn new(px: &'r mut [u8]) -> Self {
        Raster { px }
    }

    /// The bytes of pixel `i`, if it's in the raster.
    fn at(&self, i: isize) -> Option<std::ops::Range<usize>> {
        let start = usize::try_from(i).ok()?.checked_mul(4)?;
        let end = start.checked_add(4)?;
        (end <= self.px.len()).then_some(start..end)
    }

    /// `raster[i]`
    fn get(&self, i: isize) -> u32 {
        match self.at(i) {
            Some(r) => {
                let b = &self.px[r];
                u32::from_ne_bytes([b[0], b[1], b[2], b[3]])
            }
            None => 0,
        }
    }

    /// `raster[i] = v`
    fn set(&mut self, i: isize, v: u32) {
        if let Some(r) = self.at(i) {
            self.px[r].copy_from_slice(&v.to_ne_bytes());
        }
    }
}

/// The byte at `i` of the decoded data.
fn px(buf: &[u8], i: isize) -> u8 {
    match usize::try_from(i) {
        Ok(i) => buf.get(i).copied().unwrap_or(0),
        Err(_) => 0,
    }
}

/// The 16-bit sample (host byte order) at byte `i` of the decoded data.
fn px16(buf: &[u8], i: isize) -> u16 {
    u16::from_ne_bytes([px(buf, i), px(buf, i + 1)])
}

/// A table entry (`m[i]`), 0 outside the table.
fn tab<T: Copy + Default>(m: &[T], i: usize) -> T {
    m.get(i).copied().unwrap_or_default()
}

/// `tileContigRoutine`: the put routine for packed data. `cp` is the index
/// of the first pixel in the raster, `pp` of the first byte in `buf`.
pub(crate) type TileContigRoutine =
    fn(&TIFFRGBAImage, &mut Raster<'_>, isize, u32, u32, u32, u32, i32, i32, &[u8], isize);
/// `tileSeparateRoutine`: the put routine for unpacked data. `r`, `g`, `b`
/// and `a` are indices into `buf`.
pub(crate) type TileSeparateRoutine = fn(
    &TIFFRGBAImage,
    &mut Raster<'_>,
    isize,
    u32,
    u32,
    u32,
    u32,
    i32,
    i32,
    &[u8],
    isize,
    isize,
    isize,
    Option<isize>,
);
/// The get image data routine: the raster from pixel `base` on.
type GetRoutine = fn(&TIFFRGBAImage, &mut Tiff<'_>, &mut Raster<'_>, isize, u32, u32) -> i32;

/// The put decoded strip/tile routine (`put`'s union).
#[derive(Clone, Copy)]
pub(crate) enum Put {
    None,
    Contig(TileContigRoutine),
    Separate(TileSeparateRoutine),
}

/// Translation of `TIFFRGBAImage`: RGBA-reader state (the image handle is
/// passed alongside).
pub(crate) struct TIFFRGBAImage {
    stoponerr: i32,            /* stop on read error */
    is_contig: i32,            /* data is packed/separate */
    alpha: i32,                /* type of alpha data present */
    width: u32,                /* image width */
    height: u32,               /* image height */
    bitspersample: u16,        /* image bits/sample */
    samplesperpixel: u16,      /* image samples/pixel */
    orientation: u16,          /* image orientation */
    pub(crate) req_orientation: u16, /* requested orientation */
    photometric: u16,          /* image photometric interp */
    redcmap: Option<Vec<u16>>, /* colormap palette */
    greencmap: Option<Vec<u16>>,
    bluecmap: Option<Vec<u16>>,
    /* get image data routine */
    get: Option<GetRoutine>,
    /* put decoded strip/tile */
    put: Put,
    map: Option<Vec<TIFFRGBValue>>, /* sample mapping array */
    bwmap: Option<Vec<u32>>,        /* black&white map */
    palmap: Option<Vec<u32>>,       /* palette image map */
    /// the entries per byte of `bwmap` and `palmap`
    map_stride: usize,
    ycbcr: Option<Box<TIFFYCbCrToRGB>>, /* YCbCr conversion state */
    cielab: Option<Box<TIFFCIELabToRGB>>, /* CIE L*a*b conversion state */

    ua_to_aa: Option<Vec<u8>>, /* Unassociated alpha to associated alpha conversion LUT */
    bitdepth16_to_8: Option<Vec<u8>>, /* LUT for conversion from 16bit to 8bit values */

    row_offset: i32,
    col_offset: i32,
}

impl TIFFRGBAImage {
    fn new() -> Self {
        TIFFRGBAImage {
            stoponerr: 0,
            is_contig: 0,
            alpha: 0,
            width: 0,
            height: 0,
            bitspersample: 0,
            samplesperpixel: 0,
            orientation: 0,
            req_orientation: 0,
            photometric: 0,
            redcmap: None,
            greencmap: None,
            bluecmap: None,
            get: None,
            put: Put::None,
            map: None,
            bwmap: None,
            palmap: None,
            map_stride: 1,
            ycbcr: None,
            cielab: None,
            ua_to_aa: None,
            bitdepth16_to_8: None,
            row_offset: 0,
            col_offset: 0,
        }
    }

    fn map16(&self, v: u16) -> u32 {
        self.bitdepth16_to_8
            .as_deref()
            .map_or(0, |m| tab(m, v as usize) as u32)
    }

    fn ua(&self, a: u32, v: u32) -> u32 {
        self.ua_to_aa
            .as_deref()
            .map_or(0, |m| tab(m, ((a as usize) << 8) + v as usize) as u32)
    }
}

/// `TIFFGetField()` of a tag of one integer value.
fn get_int(tif: &mut Tiff<'_>, tag: u32) -> Option<u64> {
    tiff_get_field_int(tif, tag)
}

/// Check the image to see if TIFFReadRGBAImage can deal with it.
/// 1/0 is returned according to whether or not the image can
/// be handled.  If 0 is returned, emsg contains the reason
/// why it is being rejected.
/// Translation of `TIFFRGBAImageOK()`.
pub(crate) fn tiff_rgba_image_ok(tif: &mut Tiff<'_>, emsg: &mut String) -> i32 {
    if tif.tif_decodestatus == 0 {
        *emsg = "Sorry, requested compression method is not configured".to_string();
        return 0;
    }
    let td = &tif.tif_dir;
    match td.td_bitspersample {
        1 | 2 | 4 | 8 | 16 => {}
        _ => {
            *emsg = format!(
                "Sorry, can not handle images with {}-bit samples",
                td.td_bitspersample
            );
            return 0;
        }
    }
    if td.td_sampleformat == SAMPLEFORMAT_IEEEFP {
        *emsg = "Sorry, can not handle images with IEEE floating-point samples".to_string();
        return 0;
    }
    let colorchannels = td.td_samplesperpixel as i32 - td.td_extrasamples as i32;
    let photometric = match get_int(tif, TIFFTAG_PHOTOMETRIC) {
        Some(p) => p as u16,
        None => match colorchannels {
            1 => PHOTOMETRIC_MINISBLACK,
            3 => PHOTOMETRIC_RGB,
            _ => {
                *emsg = format!("Missing needed {} tag", PHOTO_TAG);
                return 0;
            }
        },
    };
    let td = &tif.tif_dir;
    match photometric {
        PHOTOMETRIC_MINISWHITE | PHOTOMETRIC_MINISBLACK | PHOTOMETRIC_PALETTE => {
            if td.td_planarconfig == PLANARCONFIG_CONTIG
                && td.td_samplesperpixel != 1
                && td.td_bitspersample < 8
            {
                *emsg = format!(
                    "Sorry, can not handle contiguous data with {}={}, and {}={} and Bits/Sample={}",
                    PHOTO_TAG,
                    photometric,
                    "Samples/pixel",
                    td.td_samplesperpixel,
                    td.td_bitspersample
                );
                return 0;
            }
            /*
             * We should likely validate that any extra samples are either
             * to be ignored, or are alpha, and if alpha we should try to use
             * them.  But for now we won't bother with this.
             */
        }
        PHOTOMETRIC_YCBCR => {
            /*
             * TODO: if at all meaningful and useful, make more complete
             * support check here, or better still, refactor to let supporting
             * code decide whether there is support and what meaningful
             * error to return
             */
        }
        PHOTOMETRIC_RGB => {
            if colorchannels < 3 {
                *emsg = format!(
                    "Sorry, can not handle RGB image with {}={}",
                    "Color channels", colorchannels
                );
                return 0;
            }
        }
        PHOTOMETRIC_SEPARATED => {
            let inkset = tiff_get_field_defaulted_u16(tif, TIFFTAG_INKSET).unwrap_or(0);
            if inkset != INKSET_CMYK {
                *emsg = format!(
                    "Sorry, can not handle separated image with {}={}",
                    "InkSet", inkset
                );
                return 0;
            }
            if tif.tif_dir.td_samplesperpixel < 4 {
                *emsg = format!(
                    "Sorry, can not handle separated image with {}={}",
                    "Samples/pixel", tif.tif_dir.td_samplesperpixel
                );
                return 0;
            }
        }
        PHOTOMETRIC_LOGL => {
            if td.td_compression != COMPRESSION_SGILOG {
                *emsg = format!(
                    "Sorry, LogL data must have {}={}",
                    "Compression", COMPRESSION_SGILOG
                );
                return 0;
            }
        }
        PHOTOMETRIC_LOGLUV => {
            if td.td_compression != COMPRESSION_SGILOG && td.td_compression != COMPRESSION_SGILOG24
            {
                *emsg = format!(
                    "Sorry, LogLuv data must have {}={} or {}",
                    "Compression", COMPRESSION_SGILOG, COMPRESSION_SGILOG24
                );
                return 0;
            }
            if td.td_planarconfig != PLANARCONFIG_CONTIG {
                *emsg = format!(
                    "Sorry, can not handle LogLuv images with {}={}",
                    "Planarconfiguration", td.td_planarconfig
                );
                return 0;
            }
            if td.td_samplesperpixel != 3 || colorchannels != 3 {
                *emsg = format!(
                    "Sorry, can not handle image with {}={}, {}={}",
                    "Samples/pixel", td.td_samplesperpixel, "colorchannels", colorchannels
                );
                return 0;
            }
        }
        PHOTOMETRIC_CIELAB => {
            if td.td_samplesperpixel != 3
                || colorchannels != 3
                || (td.td_bitspersample != 8 && td.td_bitspersample != 16)
            {
                *emsg = format!(
                    "Sorry, can not handle image with {}={}, {}={} and {}={}",
                    "Samples/pixel",
                    td.td_samplesperpixel,
                    "colorchannels",
                    colorchannels,
                    "Bits/sample",
                    td.td_bitspersample
                );
                return 0;
            }
        }
        _ => {
            *emsg = format!(
                "Sorry, can not handle image with {}={}",
                PHOTO_TAG, photometric
            );
            return 0;
        }
    }
    1
}

/// Translation of `TIFFRGBAImageEnd()`.
pub(crate) fn tiff_rgba_image_end(img: &mut TIFFRGBAImage) {
    img.map = None;
    img.bwmap = None;
    img.palmap = None;
    img.ycbcr = None;
    img.cielab = None;
    img.ua_to_aa = None;
    img.bitdepth16_to_8 = None;

    img.redcmap = None;
    img.greencmap = None;
    img.bluecmap = None;
}

/// Translation of `isCCITTCompression()`.
fn is_ccitt_compression(tif: &mut Tiff<'_>) -> bool {
    let compress = get_int(tif, TIFFTAG_COMPRESSION).unwrap_or(0) as u16;
    compress == COMPRESSION_CCITTFAX3
        || compress == COMPRESSION_CCITTFAX4
        || compress == COMPRESSION_CCITTRLE
        || compress == COMPRESSION_CCITTRLEW
}

/// A copy of the first `n` entries of a colormap (zeros past its end).
fn copy_cmap(src: Option<&Vec<u16>>, n: usize) -> Option<Vec<u16>> {
    let mut v: Vec<u16> = try_vec(n)?;
    if let Some(src) = src {
        let k = n.min(src.len());
        v[..k].copy_from_slice(&src[..k]);
    }
    Some(v)
}

/// Translation of `TIFFRGBAImageBegin()`.
pub(crate) fn tiff_rgba_image_begin(
    img: &mut TIFFRGBAImage,
    tif: &mut Tiff<'_>,
    stop: i32,
    emsg: &mut String,
) -> i32 {
    if tiff_rgba_image_ok(tif, emsg) == 0 {
        return 0;
    }

    /* Initialize to normal values */
    *img = TIFFRGBAImage::new();
    img.req_orientation = ORIENTATION_BOTLEFT; /* It is the default */

    img.stoponerr = stop;
    img.bitspersample = tiff_get_field_defaulted_u16(tif, TIFFTAG_BITSPERSAMPLE).unwrap_or(0);
    if begin_body(img, tif, emsg) {
        return 1;
    }
    // fail_return:
    tiff_rgba_image_end(img);
    0
}

/// The body of `TIFFRGBAImageBegin()` after its initializations: false
/// for its `goto fail_return`s.
fn begin_body(img: &mut TIFFRGBAImage, tif: &mut Tiff<'_>, emsg: &mut String) -> bool {
    match img.bitspersample {
        1 | 2 | 4 | 8 | 16 => {}
        _ => {
            *emsg = format!(
                "Sorry, can not handle images with {}-bit samples",
                img.bitspersample
            );
            return false;
        }
    }
    img.alpha = 0;
    img.samplesperpixel = tiff_get_field_defaulted_u16(tif, TIFFTAG_SAMPLESPERPIXEL).unwrap_or(0);
    let mut out = Vec::new();
    tiff_get_field_defaulted(tif, TIFFTAG_EXTRASAMPLES, &mut out);
    let mut extrasamples = out.first().and_then(Gv::as_u64).unwrap_or(0) as u16;
    let sampleinfo0 = out
        .get(1)
        .and_then(Gv::as_u16s)
        .and_then(|s| s.first().copied())
        .unwrap_or(0);
    if extrasamples >= 1 {
        match sampleinfo0 {
            EXTRASAMPLE_UNSPECIFIED => {
                /* Workaround for some images without */
                if img.samplesperpixel > 3 {
                    /* correct info about alpha channel */
                    img.alpha = EXTRASAMPLE_ASSOCALPHA as i32;
                }
            }
            EXTRASAMPLE_ASSOCALPHA /* data is pre-multiplied */
            | EXTRASAMPLE_UNASSALPHA /* data is not pre-multiplied */ => {
                img.alpha = sampleinfo0 as i32;
            }
            _ => {}
        }
    }

    // DEFAULT_EXTRASAMPLE_AS_ALPHA
    img.photometric = match get_int(tif, TIFFTAG_PHOTOMETRIC) {
        Some(p) => p as u16,
        None => PHOTOMETRIC_MINISWHITE,
    };

    if extrasamples == 0 && img.samplesperpixel == 4 && img.photometric == PHOTOMETRIC_RGB {
        img.alpha = EXTRASAMPLE_ASSOCALPHA as i32;
        extrasamples = 1;
    }

    let colorchannels = img.samplesperpixel as i32 - extrasamples as i32;
    let compress = tiff_get_field_defaulted_u16(tif, TIFFTAG_COMPRESSION).unwrap_or(0);
    let planarconfig = tiff_get_field_defaulted_u16(tif, TIFFTAG_PLANARCONFIG).unwrap_or(0);
    match get_int(tif, TIFFTAG_PHOTOMETRIC) {
        Some(p) => img.photometric = p as u16,
        None => match colorchannels {
            1 => {
                if is_ccitt_compression(tif) {
                    img.photometric = PHOTOMETRIC_MINISWHITE;
                } else {
                    img.photometric = PHOTOMETRIC_MINISBLACK;
                }
            }
            3 => img.photometric = PHOTOMETRIC_RGB,
            _ => {
                *emsg = format!("Missing needed {} tag", PHOTO_TAG);
                return false;
            }
        },
    }
    let mut palette_or_grey = false;
    match img.photometric {
        PHOTOMETRIC_PALETTE => {
            let mut out = Vec::new();
            if tiff_get_field(tif, TIFFTAG_COLORMAP, &mut out) == 0 {
                *emsg = "Missing required \"Colormap\" tag".to_string();
                return false;
            }

            /* copy the colormaps so we can modify them */
            let n_color = 1usize << img.bitspersample;
            let orig = |i: usize| match out.get(i) {
                Some(Gv::U16s(v)) => v.as_ref(),
                _ => None,
            };
            img.redcmap = copy_cmap(orig(0), n_color);
            img.greencmap = copy_cmap(orig(1), n_color);
            img.bluecmap = copy_cmap(orig(2), n_color);
            if img.redcmap.is_none() || img.greencmap.is_none() || img.bluecmap.is_none() {
                *emsg = "Out of memory for colormap copy".to_string();
                return false;
            }

            /* fall through... */
            palette_or_grey = true;
        }
        PHOTOMETRIC_MINISWHITE | PHOTOMETRIC_MINISBLACK => palette_or_grey = true,
        PHOTOMETRIC_YCBCR => {
            /* It would probably be nice to have a reality check here. */
            if planarconfig == PLANARCONFIG_CONTIG {
                /* can rely on libjpeg to convert to RGB */
                /* XXX should restore current state on exit */
                if compress == COMPRESSION_JPEG {
                    /*
                     * TODO: when complete tests verify complete
                     * desubsampling and YCbCr handling, remove use of
                     * TIFFTAG_JPEGCOLORMODE in favor of tif_getimage.c
                     * native handling
                     */
                    tiff_set_field(
                        tif,
                        TIFFTAG_JPEGCOLORMODE,
                        &[Va::Int(JPEGCOLORMODE_RGB as i64)],
                    );
                    img.photometric = PHOTOMETRIC_RGB;
                }
                /* default: do nothing */
            }
            /*
             * TODO: if at all meaningful and useful, make more complete
             * support check here, or better still, refactor to let supporting
             * code decide whether there is support and what meaningful
             * error to return
             */
        }
        PHOTOMETRIC_RGB => {
            if colorchannels < 3 {
                *emsg = format!(
                    "Sorry, can not handle RGB image with {}={}",
                    "Color channels", colorchannels
                );
                return false;
            }
        }
        PHOTOMETRIC_SEPARATED => {
            let inkset = tiff_get_field_defaulted_u16(tif, TIFFTAG_INKSET).unwrap_or(0);
            if inkset != INKSET_CMYK {
                *emsg = format!(
                    "Sorry, can not handle separated image with {}={}",
                    "InkSet", inkset
                );
                return false;
            }
            if img.samplesperpixel < 4 {
                *emsg = format!(
                    "Sorry, can not handle separated image with {}={}",
                    "Samples/pixel", img.samplesperpixel
                );
                return false;
            }
        }
        PHOTOMETRIC_LOGL => {
            if compress != COMPRESSION_SGILOG {
                *emsg = format!(
                    "Sorry, LogL data must have {}={}",
                    "Compression", COMPRESSION_SGILOG
                );
                return false;
            }
            tiff_set_field(
                tif,
                TIFFTAG_SGILOGDATAFMT,
                &[Va::Int(SGILOGDATAFMT_8BIT as i64)],
            );
            img.photometric = PHOTOMETRIC_MINISBLACK; /* little white lie */
            img.bitspersample = 8;
        }
        PHOTOMETRIC_LOGLUV => {
            if compress != COMPRESSION_SGILOG && compress != COMPRESSION_SGILOG24 {
                *emsg = format!(
                    "Sorry, LogLuv data must have {}={} or {}",
                    "Compression", COMPRESSION_SGILOG, COMPRESSION_SGILOG24
                );
                return false;
            }
            if planarconfig != PLANARCONFIG_CONTIG {
                *emsg = format!(
                    "Sorry, can not handle LogLuv images with {}={}",
                    "Planarconfiguration", planarconfig
                );
                // (the C returns here without freeing: nothing is allocated)
                return false;
            }
            tiff_set_field(
                tif,
                TIFFTAG_SGILOGDATAFMT,
                &[Va::Int(SGILOGDATAFMT_8BIT as i64)],
            );
            img.photometric = PHOTOMETRIC_RGB; /* little white lie */
            img.bitspersample = 8;
        }
        PHOTOMETRIC_CIELAB => {}
        _ => {
            *emsg = format!(
                "Sorry, can not handle image with {}={}",
                PHOTO_TAG, img.photometric
            );
            return false;
        }
    }
    if palette_or_grey
        && planarconfig == PLANARCONFIG_CONTIG
        && img.samplesperpixel != 1
        && img.bitspersample < 8
    {
        *emsg = format!(
            "Sorry, can not handle contiguous data with {}={}, and {}={} and Bits/Sample={}",
            PHOTO_TAG,
            img.photometric,
            "Samples/pixel",
            img.samplesperpixel,
            img.bitspersample
        );
        return false;
    }
    img.width = get_int(tif, TIFFTAG_IMAGEWIDTH).unwrap_or(0) as u32;
    img.height = get_int(tif, TIFFTAG_IMAGELENGTH).unwrap_or(0) as u32;
    img.orientation = tiff_get_field_defaulted_u16(tif, TIFFTAG_ORIENTATION).unwrap_or(0);
    img.is_contig =
        !(planarconfig == PLANARCONFIG_SEPARATE && img.samplesperpixel > 1) as i32;
    if img.is_contig != 0 {
        if pick_contig_case(img, tif) == 0 {
            *emsg = "Sorry, can not handle image".to_string();
            return false;
        }
    } else if pick_separate_case(img, tif) == 0 {
        *emsg = "Sorry, can not handle image".to_string();
        return false;
    }
    true
}

/// Translation of `TIFFRGBAImageGet()`.
pub(crate) fn tiff_rgba_image_get(
    img: &mut TIFFRGBAImage,
    tif: &mut Tiff<'_>,
    raster: &mut Raster<'_>,
    w: u32,
    mut h: u32,
) -> i32 {
    let Some(get) = img.get else {
        tiff_error_ext_r!(&tif.tif_name, "No \"get\" routine setup");
        return 0;
    };
    if let Put::None = img.put {
        tiff_error_ext_r!(
            &tif.tif_name,
            "No \"put\" routine setupl; probably can not handle image format"
        );
        return 0;
    }
    let mut base: isize = 0;
    /* Verify raster height against image height.
     * Width is checked in img->get() function individually. */
    if 0 <= img.row_offset && (img.row_offset as u32) < img.height {
        let hx = img.height - img.row_offset as u32;
        if h > hx {
            /* Adapt parameters to read only available lines and put image
             * at the bottom of the raster. */
            base += ((h - hx) as usize).wrapping_mul(w as usize) as isize;
            h = hx;
        }
    } else {
        tiff_error_ext_r!(
            &tif.tif_name,
            "Error in TIFFRGBAImageGet: row offset {} exceeds image height {}",
            img.row_offset,
            img.height
        );
        return 0;
    }
    get(img, tif, raster, base, w, h)
}

/// Read the specified image into an ABGR-format rastertaking in account
/// specified orientation.
/// Translation of `TIFFReadRGBAImageOriented()`: `raster` holds the
/// `rwidth` x `rheight` pixels.
pub(crate) fn tiff_read_rgba_image_oriented(
    tif: &mut Tiff<'_>,
    rwidth: u32,
    rheight: u32,
    raster: &mut [u8],
    orientation: i32,
    stop: i32,
) -> i32 {
    let mut emsg = String::new();
    let mut img = TIFFRGBAImage::new();

    let ok = if tiff_rgba_image_begin(&mut img, tif, stop, &mut emsg) != 0 {
        img.req_orientation = orientation as u16;
        let ok = tiff_rgba_image_get(&mut img, tif, &mut Raster::new(raster), rwidth, rheight);
        tiff_rgba_image_end(&mut img);
        ok
    } else {
        tiff_error_ext_r!(&tif.tif_name, "{}", emsg);
        0
    };
    ok
}

/// Translation of `setorientation()`.
fn setorientation(img: &TIFFRGBAImage) -> i32 {
    let req = img.req_orientation;
    let is = |a: u16, b: u16| req == a || req == b;
    match img.orientation {
        ORIENTATION_TOPLEFT | ORIENTATION_LEFTTOP => {
            if is(ORIENTATION_TOPRIGHT, ORIENTATION_RIGHTTOP) {
                FLIP_HORIZONTALLY
            } else if is(ORIENTATION_BOTRIGHT, ORIENTATION_RIGHTBOT) {
                FLIP_HORIZONTALLY | FLIP_VERTICALLY
            } else if is(ORIENTATION_BOTLEFT, ORIENTATION_LEFTBOT) {
                FLIP_VERTICALLY
            } else {
                0
            }
        }
        ORIENTATION_TOPRIGHT | ORIENTATION_RIGHTTOP => {
            if is(ORIENTATION_TOPLEFT, ORIENTATION_LEFTTOP) {
                FLIP_HORIZONTALLY
            } else if is(ORIENTATION_BOTRIGHT, ORIENTATION_RIGHTBOT) {
                FLIP_VERTICALLY
            } else if is(ORIENTATION_BOTLEFT, ORIENTATION_LEFTBOT) {
                FLIP_HORIZONTALLY | FLIP_VERTICALLY
            } else {
                0
            }
        }
        ORIENTATION_BOTRIGHT | ORIENTATION_RIGHTBOT => {
            if is(ORIENTATION_TOPLEFT, ORIENTATION_LEFTTOP) {
                FLIP_HORIZONTALLY | FLIP_VERTICALLY
            } else if is(ORIENTATION_TOPRIGHT, ORIENTATION_RIGHTTOP) {
                FLIP_VERTICALLY
            } else if is(ORIENTATION_BOTLEFT, ORIENTATION_LEFTBOT) {
                FLIP_HORIZONTALLY
            } else {
                0
            }
        }
        ORIENTATION_BOTLEFT | ORIENTATION_LEFTBOT => {
            if is(ORIENTATION_TOPLEFT, ORIENTATION_LEFTTOP) {
                FLIP_VERTICALLY
            } else if is(ORIENTATION_TOPRIGHT, ORIENTATION_RIGHTTOP) {
                FLIP_HORIZONTALLY | FLIP_VERTICALLY
            } else if is(ORIENTATION_BOTRIGHT, ORIENTATION_RIGHTBOT) {
                FLIP_HORIZONTALLY
            } else {
                0
            }
        }
        _ => 0, /* NOTREACHED */
    }
}

/// The horizontal flip at the end of the get routines: Use wmin to only
/// flip horizontally data in place and not complete raster-row.
fn flip_horizontally(raster: &mut Raster<'_>, base: isize, w: u32, h: u32, wmin: u32) {
    for line in 0..h {
        let mut left = base + (line as isize).wrapping_mul(w as isize);
        let mut right = left + wmin as isize - 1;

        while left < right {
            let temp = raster.get(left);
            let r = raster.get(right);
            raster.set(left, r);
            raster.set(right, temp);
            left += 1;
            right -= 1;
        }
    }
}

/// Get an tile-organized image that has
///  PlanarConfiguration contiguous if SamplesPerPixel > 1
/// or
///  SamplesPerPixel == 1
/// Translation of `gtTileContig()`.
fn gt_tile_contig(
    img: &TIFFRGBAImage,
    tif: &mut Tiff<'_>,
    raster: &mut Raster<'_>,
    base: isize,
    w: u32,
    h: u32,
) -> i32 {
    let Put::Contig(put) = img.put else {
        return 0;
    };
    let mut buf: Option<Vec<u8>> = None;
    let mut ret = 1;

    /* If the raster is smaller than the image,
     * or if there is a col_offset, adapt the samples to be copied per row. */
    let wmin = if 0 <= img.col_offset && (img.col_offset as u32) < img.width {
        w.min(img.width - img.col_offset as u32)
    } else {
        tiff_error_ext_r!(
            &tif.tif_name,
            "Error in gtTileContig: column offset {} exceeds image width {}",
            img.col_offset,
            img.width
        );
        return 0;
    };
    let bufsize = tiff_tile_size(tif);
    if bufsize == 0 {
        tiff_error_ext_r!(&tif.tif_name, "{}", "No space for tile buffer");
        return 0;
    }

    let tw = get_int(tif, TIFFTAG_TILEWIDTH).unwrap_or(0) as u32;
    let th = get_int(tif, TIFFTAG_TILELENGTH).unwrap_or(0) as u32;

    let flip = setorientation(img);
    let mut y: u32;
    let toskew: i32;
    if flip & FLIP_VERTICALLY != 0 {
        if (tw as i64 + w as i64) > i32::MAX as i64 {
            tiff_error_ext_r!(&tif.tif_name, "{}", "unsupported tile size (too wide)");
            return 0;
        }
        y = h.wrapping_sub(1);
        toskew = (tw.wrapping_add(w) as i32).wrapping_neg();
    } else {
        if tw as i64 > (i32::MAX as i64 + w as i64) || w as i64 > (i32::MAX as i64 + tw as i64) {
            tiff_error_ext_r!(&tif.tif_name, "{}", "unsupported tile size (too wide)");
            return 0;
        }
        y = 0;
        toskew = (tw.wrapping_sub(w) as i32).wrapping_neg();
    }

    if tw == 0 || th == 0 {
        tiff_error_ext_r!(&tif.tif_name, "tile width or height is zero");
        return 0;
    }

    /*
     *  Leftmost tile is clipped on left side if col_offset > 0.
     */
    let leftmost_fromskew = (img.col_offset as u32 % tw) as i32;
    let leftmost_tw = (tw as i32).wrapping_sub(leftmost_fromskew) as u32;
    let skew_i64 = toskew as i64 + leftmost_fromskew as i64;
    if skew_i64 > i32::MAX as i64 || skew_i64 < i32::MIN as i64 {
        tiff_error_ext_r!(&tif.tif_name, "{} {}", "Invalid skew", skew_i64);
        return 0;
    }
    let leftmost_toskew = skew_i64 as i32;
    let mut row: u32 = 0;
    while ret != 0 && row < h {
        let rowstoread = th - (row.wrapping_add(img.row_offset as u32)) % th;
        let nrow = if row.wrapping_add(rowstoread) > h {
            h - row
        } else {
            rowstoread
        };
        let mut fromskew = leftmost_fromskew;
        let mut this_tw = leftmost_tw;
        let mut this_toskew = leftmost_toskew;
        let mut tocol: u32 = 0;
        let mut col = img.col_offset as u32;
        /* wmin: only write imagewidth if raster is bigger. */
        while tocol < wmin {
            if _tiff_read_tile_and_alloc_buffer(
                tif,
                &mut buf,
                bufsize,
                col,
                row.wrapping_add(img.row_offset as u32),
                0,
                0,
            ) == -1
                && (buf.is_none() || img.stoponerr != 0)
            {
                ret = 0;
                break;
            }
            let pos = ((row.wrapping_add(img.row_offset as u32)) % th) as TmSize
                * tiff_tile_row_size(tif)
                + (fromskew as TmSize * img.samplesperpixel as TmSize);
            if tocol.wrapping_add(this_tw) > wmin {
                /*
                 * Rightmost tile is clipped on right side.
                 */
                fromskew = tw.wrapping_sub(wmin - tocol) as i32;
                this_tw = (tw as i32).wrapping_sub(fromskew) as u32;
                this_toskew = toskew.wrapping_add(fromskew as u32 as i32);
            }
            let roffset = (y as TmSize).wrapping_mul(w as TmSize) + tocol as TmSize;
            put(
                img,
                raster,
                base + roffset,
                tocol,
                y,
                this_tw,
                nrow,
                fromskew,
                this_toskew,
                buf.as_deref().unwrap_or(&[]),
                pos,
            );
            tocol = tocol.wrapping_add(this_tw);
            col = col.wrapping_add(this_tw);
            /*
             * After the leftmost tile, tiles are no longer clipped on left
             * side.
             */
            fromskew = 0;
            this_tw = tw;
            this_toskew = toskew;
        }

        y = y.wrapping_add(if flip & FLIP_VERTICALLY != 0 {
            (nrow as i32).wrapping_neg() as u32
        } else {
            nrow
        });
        row = row.wrapping_add(nrow);
    }
    drop(buf);

    if flip & FLIP_HORIZONTALLY != 0 {
        flip_horizontally(raster, base, w, h, wmin);
    }

    ret
}

/// Get an tile-organized image that has
///  SamplesPerPixel > 1
///  PlanarConfiguration separated
/// We assume that all such images are RGB.
/// Translation of `gtTileSeparate()`.
fn gt_tile_separate(
    img: &TIFFRGBAImage,
    tif: &mut Tiff<'_>,
    raster: &mut Raster<'_>,
    base: isize,
    w: u32,
    h: u32,
) -> i32 {
    let Put::Separate(put) = img.put else {
        return 0;
    };
    let mut buf: Option<Vec<u8>> = None;
    let (mut p0, mut p1, mut p2): (isize, isize, isize) = (0, 0, 0);
    let mut pa: Option<isize> = None;
    let alpha = img.alpha;
    let mut ret = 1;

    /* If the raster is smaller than the image,
     * or if there is a col_offset, adapt the samples to be copied per row. */
    let wmin = if 0 <= img.col_offset && (img.col_offset as u32) < img.width {
        w.min(img.width - img.col_offset as u32)
    } else {
        tiff_error_ext_r!(
            &tif.tif_name,
            "Error in gtTileSeparate: column offset {} exceeds image width {}",
            img.col_offset,
            img.width
        );
        return 0;
    };

    let tilesize = tiff_tile_size(tif);
    let bufsize = _tiff_multiply_ssize(
        Some(tif),
        if alpha != 0 { 4 } else { 3 },
        tilesize,
        Some("gtTileSeparate"),
    );
    if bufsize == 0 {
        return 0;
    }

    let tw = get_int(tif, TIFFTAG_TILEWIDTH).unwrap_or(0) as u32;
    let th = get_int(tif, TIFFTAG_TILELENGTH).unwrap_or(0) as u32;

    let flip = setorientation(img);
    let mut y: u32;
    let toskew: i32;
    if flip & FLIP_VERTICALLY != 0 {
        if (tw as i64 + w as i64) > i32::MAX as i64 {
            tiff_error_ext_r!(&tif.tif_name, "{}", "unsupported tile size (too wide)");
            return 0;
        }
        y = h.wrapping_sub(1);
        toskew = (tw.wrapping_add(w) as i32).wrapping_neg();
    } else {
        if tw as i64 > (i32::MAX as i64 + w as i64) || w as i64 > (i32::MAX as i64 + tw as i64) {
            tiff_error_ext_r!(&tif.tif_name, "{}", "unsupported tile size (too wide)");
            return 0;
        }
        y = 0;
        toskew = (tw.wrapping_sub(w) as i32).wrapping_neg();
    }

    let colorchannels: u16 = match img.photometric {
        PHOTOMETRIC_MINISWHITE | PHOTOMETRIC_MINISBLACK | PHOTOMETRIC_PALETTE => 1,
        _ => 3,
    };

    if tw == 0 || th == 0 {
        tiff_error_ext_r!(&tif.tif_name, "tile width or height is zero");
        return 0;
    }

    /*
     *  Leftmost tile is clipped on left side if col_offset > 0.
     */
    let leftmost_fromskew = (img.col_offset as u32 % tw) as i32;
    let leftmost_tw = (tw as i32).wrapping_sub(leftmost_fromskew) as u32;
    let skew_i64 = toskew as i64 + leftmost_fromskew as i64;
    if skew_i64 > i32::MAX as i64 || skew_i64 < i32::MIN as i64 {
        tiff_error_ext_r!(&tif.tif_name, "{} {}", "Invalid skew", skew_i64);
        return 0;
    }
    let leftmost_toskew = skew_i64 as i32;
    let mut row: u32 = 0;
    while ret != 0 && row < h {
        let rowstoread = th - (row.wrapping_add(img.row_offset as u32)) % th;
        let nrow = if row.wrapping_add(rowstoread) > h {
            h - row
        } else {
            rowstoread
        };
        let mut fromskew = leftmost_fromskew;
        let mut this_tw = leftmost_tw;
        let mut this_toskew = leftmost_toskew;
        let mut tocol: u32 = 0;
        let mut col = img.col_offset as u32;
        let trow = (row as i32).wrapping_add(img.row_offset) as u32;
        /* wmin: only write imagewidth if raster is bigger. */
        while tocol < wmin {
            if buf.is_none() {
                if _tiff_read_tile_and_alloc_buffer(tif, &mut buf, bufsize, col, trow, 0, 0) == -1
                    && (buf.is_none() || img.stoponerr != 0)
                {
                    ret = 0;
                    break;
                }
                p0 = 0;
                if colorchannels == 1 {
                    p1 = p0;
                    p2 = p0;
                    pa = if alpha != 0 {
                        Some(p0 + 3 * tilesize)
                    } else {
                        None
                    };
                } else {
                    p1 = p0 + tilesize;
                    p2 = p1 + tilesize;
                    pa = if alpha != 0 { Some(p2 + tilesize) } else { None };
                }
            } else if read_tile_at(tif, &mut buf, p0, col, trow, 0) == -1 && img.stoponerr != 0 {
                ret = 0;
                break;
            }
            if colorchannels > 1
                && read_tile_at(tif, &mut buf, p1, col, trow, 1) == -1
                && img.stoponerr != 0
            {
                ret = 0;
                break;
            }
            if colorchannels > 1
                && read_tile_at(tif, &mut buf, p2, col, trow, 2) == -1
                && img.stoponerr != 0
            {
                ret = 0;
                break;
            }
            if let Some(pa) = pa {
                if read_tile_at(tif, &mut buf, pa, col, trow, colorchannels) == -1
                    && img.stoponerr != 0
                {
                    ret = 0;
                    break;
                }
            }

            /* For SEPARATE the pos-offset is per sample and should not be
             * multiplied by img->samplesperpixel. */
            let pos = (trow % th) as TmSize * tiff_tile_row_size(tif) + fromskew as TmSize;
            if tocol.wrapping_add(this_tw) > wmin {
                /*
                 * Rightmost tile is clipped on right side.
                 */
                fromskew = tw.wrapping_sub(wmin - tocol) as i32;
                this_tw = (tw as i32).wrapping_sub(fromskew) as u32;
                this_toskew = toskew.wrapping_add(fromskew as u32 as i32);
            }
            let roffset = (y as TmSize).wrapping_mul(w as TmSize) + tocol as TmSize;
            put(
                img,
                raster,
                base + roffset,
                tocol,
                y,
                this_tw,
                nrow,
                fromskew,
                this_toskew,
                buf.as_deref().unwrap_or(&[]),
                p0 + pos,
                p1 + pos,
                p2 + pos,
                pa.map(|pa| pa + pos),
            );
            tocol = tocol.wrapping_add(this_tw);
            col = col.wrapping_add(this_tw);
            /*
             * After the leftmost tile, tiles are no longer clipped on left
             * side.
             */
            fromskew = 0;
            this_tw = tw;
            this_toskew = toskew;
        }

        y = y.wrapping_add(if flip & FLIP_VERTICALLY != 0 {
            (nrow as i32).wrapping_neg() as u32
        } else {
            nrow
        });
        row = row.wrapping_add(nrow);
    }

    if flip & FLIP_HORIZONTALLY != 0 {
        flip_horizontally(raster, base, w, h, wmin);
    }

    ret
}

/// `TIFFReadTile(tif, buf + at, x, y, 0, s)`.
fn read_tile_at(
    tif: &mut Tiff<'_>,
    buf: &mut Option<Vec<u8>>,
    at: isize,
    x: u32,
    y: u32,
    s: u16,
) -> TmSize {
    let Some(b) = buf.as_mut() else {
        return -1;
    };
    let slice = b.get_mut(at.max(0) as usize..).unwrap_or(&mut []);
    tiff_read_tile(tif, slice, x, y, 0, s)
}

/// `TIFFReadEncodedStrip(tif, strip, buf + at, size)`.
fn read_strip_at(
    tif: &mut Tiff<'_>,
    strip: u32,
    buf: &mut Option<Vec<u8>>,
    at: isize,
    size: TmSize,
) -> TmSize {
    let Some(b) = buf.as_mut() else {
        return -1;
    };
    let slice = b.get_mut(at.max(0) as usize..).unwrap_or(&mut []);
    tiff_read_encoded_strip(tif, strip, slice, size)
}

/// Get a strip-organized image that has
///  PlanarConfiguration contiguous if SamplesPerPixel > 1
/// or
///  SamplesPerPixel == 1
/// Translation of `gtStripContig()`.
fn gt_strip_contig(
    img: &TIFFRGBAImage,
    tif: &mut Tiff<'_>,
    raster: &mut Raster<'_>,
    base: isize,
    w: u32,
    h: u32,
) -> i32 {
    let Put::Contig(put) = img.put else {
        return 0;
    };
    let mut buf: Option<Vec<u8>> = None;
    let imagewidth = img.width;
    let mut ret = 1;

    /* If the raster is smaller than the image,
     * or if there is a col_offset, adapt the samples to be copied per row. */
    let wmin = if 0 <= img.col_offset && (img.col_offset as u32) < imagewidth {
        w.min(imagewidth - img.col_offset as u32)
    } else {
        tiff_error_ext_r!(
            &tif.tif_name,
            "Error in gtStripContig: column offset {} exceeds image width {}",
            img.col_offset,
            imagewidth
        );
        return 0;
    };

    let (_subsamplinghor, subsamplingver) =
        tiff_get_field_defaulted_u16_pair(tif, TIFFTAG_YCBCRSUBSAMPLING).unwrap_or((0, 0));
    if subsamplingver == 0 {
        tiff_error_ext_r!(&tif.tif_name, "Invalid vertical YCbCr subsampling");
        return 0;
    }

    let maxstripsize = tiff_strip_size(tif);

    let flip = setorientation(img);
    let mut y: u32;
    /* fromskew, toskew are the increments within the input image or the raster
     * from the end of a line to the start of the next line to read or write. */
    let toskew: i32;
    if flip & FLIP_VERTICALLY != 0 {
        if w > i32::MAX as u32 / 2 {
            tiff_error_ext_r!(&tif.tif_name, "Width overflow");
            return 0;
        }
        y = h.wrapping_sub(1);
        /* Skew back to the raster row before the currently written row
         * -> one raster width plus copied image pixels. */
        toskew = (w.wrapping_add(wmin) as i32).wrapping_neg();
    } else {
        y = 0;
        /* Skew forward to the end of the raster width of the row currently
         * copied. */
        toskew = w.wrapping_sub(wmin) as i32;
    }

    let rowsperstrip = get_defaulted_u32(tif, TIFFTAG_ROWSPERSTRIP);
    if rowsperstrip == 0 {
        tiff_error_ext_r!(&tif.tif_name, "rowsperstrip is zero");
        return 0;
    }

    let scanline = tiff_scanline_size(tif);
    let fromskew = if w < imagewidth { imagewidth - w } else { 0 } as i32;
    let mut row: u32 = 0;
    while row < h {
        let srow = (row as i32).wrapping_add(img.row_offset) as u32;
        let rowstoread = rowsperstrip - srow % rowsperstrip;
        let nrow = if row.wrapping_add(rowstoread) > h {
            h - row
        } else {
            rowstoread
        };
        let mut nrowsub = nrow;
        if (nrowsub % subsamplingver as u32) != 0 {
            nrowsub = nrowsub
                .wrapping_add(subsamplingver as u32 - nrowsub % subsamplingver as u32);
        }
        let temp = (srow % rowsperstrip).wrapping_add(nrowsub);
        if scanline > 0 && temp as usize > (TIFF_TMSIZE_T_MAX / scanline) as usize {
            tiff_error_ext_r!(&tif.tif_name, "Integer overflow in gtStripContig");
            return 0;
        }
        let strip = tiff_compute_strip(tif, srow, 0);
        if _tiff_read_encoded_strip_and_alloc_buffer(
            tif,
            strip,
            &mut buf,
            maxstripsize,
            (temp as usize).wrapping_mul(scanline as usize) as TmSize,
        ) == -1
            && (buf.is_none() || img.stoponerr != 0)
        {
            ret = 0;
            break;
        }

        let pos = (srow % rowsperstrip) as TmSize * scanline
            + (img.col_offset as TmSize * img.samplesperpixel as TmSize);
        let roffset = (y as TmSize).wrapping_mul(w as TmSize);
        put(
            img,
            raster,
            base + roffset,
            0,
            y,
            wmin,
            nrow,
            fromskew,
            toskew,
            buf.as_deref().unwrap_or(&[]),
            pos,
        );
        y = y.wrapping_add(if flip & FLIP_VERTICALLY != 0 {
            (nrow as i32).wrapping_neg() as u32
        } else {
            nrow
        });
        row = row.wrapping_add(nrow);
    }

    if flip & FLIP_HORIZONTALLY != 0 {
        /* Flips the complete raster matrix horizontally. If raster width is
         * larger than image width, data are moved horizontally to the right
         * side.
         * Use wmin to only flip data in place. */
        flip_horizontally(raster, base, w, h, wmin);
    }

    ret
}

/// `TIFFGetFieldDefaulted(tif, tag, &v)` for a field of one 32-bit value.
fn get_defaulted_u32(tif: &mut Tiff<'_>, tag: u32) -> u32 {
    let mut out = Vec::new();
    tiff_get_field_defaulted(tif, tag, &mut out);
    out.first().and_then(Gv::as_u64).unwrap_or(0) as u32
}

/// Get a strip-organized image with
///  SamplesPerPixel > 1
///  PlanarConfiguration separated
/// We assume that all such images are RGB.
/// Translation of `gtStripSeparate()`.
fn gt_strip_separate(
    img: &TIFFRGBAImage,
    tif: &mut Tiff<'_>,
    raster: &mut Raster<'_>,
    base: isize,
    w: u32,
    h: u32,
) -> i32 {
    let Put::Separate(put) = img.put else {
        return 0;
    };
    let mut buf: Option<Vec<u8>> = None;
    let (mut p0, mut p1, mut p2): (isize, isize, isize) = (0, 0, 0);
    let mut pa: Option<isize> = None;
    let imagewidth = img.width;
    let alpha = img.alpha;
    let mut ret = 1;

    /* If the raster is smaller than the image,
     * or if there is a col_offset, adapt the samples to be copied per row. */
    let wmin = if 0 <= img.col_offset && (img.col_offset as u32) < imagewidth {
        w.min(imagewidth - img.col_offset as u32)
    } else {
        tiff_error_ext_r!(
            &tif.tif_name,
            "Error in gtStripSeparate: column offset {} exceeds image width {}",
            img.col_offset,
            imagewidth
        );
        return 0;
    };

    let stripsize = tiff_strip_size(tif);
    let bufsize = _tiff_multiply_ssize(
        Some(tif),
        if alpha != 0 { 4 } else { 3 },
        stripsize,
        Some("gtStripSeparate"),
    );
    if bufsize == 0 {
        return 0;
    }

    let flip = setorientation(img);
    let mut y: u32;
    let toskew: i32;
    if flip & FLIP_VERTICALLY != 0 {
        if w > i32::MAX as u32 / 2 {
            tiff_error_ext_r!(&tif.tif_name, "Width overflow");
            return 0;
        }
        y = h.wrapping_sub(1);
        /* Skew back to the raster row before the currently written row
         * -> one raster width plus one image width. */
        toskew = (w.wrapping_add(wmin) as i32).wrapping_neg();
    } else {
        y = 0;
        /* Skew forward to the end of the raster width of the row currently
         * written. */
        toskew = w.wrapping_sub(wmin) as i32;
    }

    let colorchannels: u16 = match img.photometric {
        PHOTOMETRIC_MINISWHITE | PHOTOMETRIC_MINISBLACK | PHOTOMETRIC_PALETTE => 1,
        _ => 3,
    };

    let rowsperstrip = get_defaulted_u32(tif, TIFFTAG_ROWSPERSTRIP);
    if rowsperstrip == 0 {
        tiff_error_ext_r!(&tif.tif_name, "rowsperstrip is zero");
        return 0;
    }

    let scanline = tiff_scanline_size(tif);
    let fromskew = if w < imagewidth { imagewidth - w } else { 0 } as i32;
    let mut row: u32 = 0;
    while row < h {
        let srow = (row as i32).wrapping_add(img.row_offset) as u32;
        let rowstoread = rowsperstrip - srow % rowsperstrip;
        let nrow = if row.wrapping_add(rowstoread) > h {
            h - row
        } else {
            rowstoread
        };
        let offset_row = srow;
        let temp = (srow % rowsperstrip).wrapping_add(nrow);
        if scanline > 0 && temp as usize > (TIFF_TMSIZE_T_MAX / scanline) as usize {
            tiff_error_ext_r!(&tif.tif_name, "Integer overflow in gtStripSeparate");
            return 0;
        }
        let size = (temp as TmSize).wrapping_mul(scanline);
        if buf.is_none() {
            let strip = tiff_compute_strip(tif, offset_row, 0);
            if _tiff_read_encoded_strip_and_alloc_buffer(tif, strip, &mut buf, bufsize, size)
                == -1
                && (buf.is_none() || img.stoponerr != 0)
            {
                ret = 0;
                break;
            }
            p0 = 0;
            if colorchannels == 1 {
                p1 = p0;
                p2 = p0;
                pa = if alpha != 0 {
                    Some(p0 + 3 * stripsize)
                } else {
                    None
                };
            } else {
                p1 = p0 + stripsize;
                p2 = p1 + stripsize;
                pa = if alpha != 0 {
                    Some(p2 + stripsize)
                } else {
                    None
                };
            }
        } else {
            let strip = tiff_compute_strip(tif, offset_row, 0);
            if read_strip_at(tif, strip, &mut buf, p0, size) == -1 && img.stoponerr != 0 {
                ret = 0;
                break;
            }
        }
        if colorchannels > 1 {
            let strip = tiff_compute_strip(tif, offset_row, 1);
            if read_strip_at(tif, strip, &mut buf, p1, size) == -1 && img.stoponerr != 0 {
                ret = 0;
                break;
            }
        }
        if colorchannels > 1 {
            let strip = tiff_compute_strip(tif, offset_row, 2);
            if read_strip_at(tif, strip, &mut buf, p2, size) == -1 && img.stoponerr != 0 {
                ret = 0;
                break;
            }
        }
        if let Some(pa) = pa {
            let strip = tiff_compute_strip(tif, offset_row, colorchannels);
            if read_strip_at(tif, strip, &mut buf, pa, size) == -1 && img.stoponerr != 0 {
                ret = 0;
                break;
            }
        }

        /* For SEPARATE the pos-offset is per sample and should not be
         * multiplied by img->samplesperpixel. */
        let pos = (srow % rowsperstrip) as TmSize * scanline + img.col_offset as TmSize;
        let roffset = (y as TmSize).wrapping_mul(w as TmSize);
        put(
            img,
            raster,
            base + roffset,
            0,
            y,
            wmin,
            nrow,
            fromskew,
            toskew,
            buf.as_deref().unwrap_or(&[]),
            p0 + pos,
            p1 + pos,
            p2 + pos,
            pa.map(|pa| pa + pos),
        );
        y = y.wrapping_add(if flip & FLIP_VERTICALLY != 0 {
            (nrow as i32).wrapping_neg() as u32
        } else {
            nrow
        });
        row = row.wrapping_add(nrow);
    }

    if flip & FLIP_HORIZONTALLY != 0 {
        flip_horizontally(raster, base, w, h, wmin);
    }

    ret
}

/*
 * The following routines move decoded data returned
 * from the TIFF library into rasters filled with packed
 * ABGR pixels (i.e. suitable for passing to lrecwrite.)
 *
 * The routines have been created according to the most
 * important cases and optimized.  PickContigCase and
 * PickSeparateCase analyze the parameters and select
 * the appropriate "get" and "put" routine to use.
 *
 * (The C's REPEAT/CASE/UNROLL macros unroll loops: here they are the
 * loops.)
 */

const A1: u32 = 0xff << 24;
/// `PACK()`
fn pack(r: u32, g: u32, b: u32) -> u32 {
    r | (g << 8) | (b << 16) | A1
}
/// `PACK4()`
fn pack4(r: u32, g: u32, b: u32, a: u32) -> u32 {
    r | (g << 8) | (b << 16) | (a << 24)
}

/// `UNROLLn(w, bw = map[*pp++], *cp++ = *bw++)` for a map of `n` pixels
/// per byte.
fn unroll_map(
    raster: &mut Raster<'_>,
    cp: &mut isize,
    buf: &[u8],
    pp: &mut isize,
    map: &[u32],
    n: u32,
    w: u32,
) {
    let mut x = w;
    while x > 0 {
        let k = x.min(n);
        let bw = px(buf, *pp) as usize * n as usize;
        *pp += 1;
        for i in 0..k as usize {
            raster.set(*cp, tab(map, bw + i));
            *cp += 1;
        }
        x -= k;
    }
}

/// 8-bit palette => colormap/RGB
/// Translation of `put8bitcmaptile()`.
fn put8bitcmaptile(
    img: &TIFFRGBAImage,
    raster: &mut Raster<'_>,
    mut cp: isize,
    _x: u32,
    _y: u32,
    w: u32,
    h: u32,
    fromskew: i32,
    toskew: i32,
    buf: &[u8],
    mut pp: isize,
) {
    let palmap = img.palmap.as_deref().unwrap_or(&[]);
    let stride = img.map_stride;
    let samplesperpixel = img.samplesperpixel as isize;

    for _ in 0..h {
        for _ in 0..w {
            raster.set(cp, tab(palmap, px(buf, pp) as usize * stride));
            cp += 1;
            pp += samplesperpixel;
        }
        cp += toskew as isize;
        pp += fromskew as isize;
    }
}

/// 4-bit palette => colormap/RGB
/// Translation of `put4bitcmaptile()`.
fn put4bitcmaptile(
    img: &TIFFRGBAImage,
    raster: &mut Raster<'_>,
    mut cp: isize,
    _x: u32,
    _y: u32,
    w: u32,
    h: u32,
    mut fromskew: i32,
    toskew: i32,
    buf: &[u8],
    mut pp: isize,
) {
    let palmap = img.palmap.as_deref().unwrap_or(&[]);

    fromskew /= 2;
    for _ in 0..h {
        unroll_map(raster, &mut cp, buf, &mut pp, palmap, 2, w);
        cp += toskew as isize;
        pp += fromskew as isize;
    }
}

/// 2-bit palette => colormap/RGB
/// Translation of `put2bitcmaptile()`.
fn put2bitcmaptile(
    img: &TIFFRGBAImage,
    raster: &mut Raster<'_>,
    mut cp: isize,
    _x: u32,
    _y: u32,
    w: u32,
    h: u32,
    mut fromskew: i32,
    toskew: i32,
    buf: &[u8],
    mut pp: isize,
) {
    let palmap = img.palmap.as_deref().unwrap_or(&[]);

    fromskew /= 4;
    for _ in 0..h {
        unroll_map(raster, &mut cp, buf, &mut pp, palmap, 4, w);
        cp += toskew as isize;
        pp += fromskew as isize;
    }
}

/// 1-bit palette => colormap/RGB
/// Translation of `put1bitcmaptile()`.
fn put1bitcmaptile(
    img: &TIFFRGBAImage,
    raster: &mut Raster<'_>,
    mut cp: isize,
    _x: u32,
    _y: u32,
    w: u32,
    h: u32,
    mut fromskew: i32,
    toskew: i32,
    buf: &[u8],
    mut pp: isize,
) {
    let palmap = img.palmap.as_deref().unwrap_or(&[]);

    fromskew /= 8;
    for _ in 0..h {
        unroll_map(raster, &mut cp, buf, &mut pp, palmap, 8, w);
        cp += toskew as isize;
        pp += fromskew as isize;
    }
}

/// 8-bit greyscale => colormap/RGB
/// Translation of `putgreytile()`.
fn putgreytile(
    img: &TIFFRGBAImage,
    raster: &mut Raster<'_>,
    mut cp: isize,
    _x: u32,
    _y: u32,
    w: u32,
    h: u32,
    fromskew: i32,
    toskew: i32,
    buf: &[u8],
    mut pp: isize,
) {
    let samplesperpixel = img.samplesperpixel as isize;
    let bwmap = img.bwmap.as_deref().unwrap_or(&[]);
    let stride = img.map_stride;

    for _ in 0..h {
        for _ in 0..w {
            raster.set(cp, tab(bwmap, px(buf, pp) as usize * stride));
            cp += 1;
            pp += samplesperpixel;
        }
        cp += toskew as isize;
        pp += fromskew as isize;
    }
}

/// 8-bit greyscale with associated alpha => colormap/RGBA
/// Translation of `putagreytile()`.
fn putagreytile(
    img: &TIFFRGBAImage,
    raster: &mut Raster<'_>,
    mut cp: isize,
    _x: u32,
    _y: u32,
    w: u32,
    h: u32,
    fromskew: i32,
    toskew: i32,
    buf: &[u8],
    mut pp: isize,
) {
    let samplesperpixel = img.samplesperpixel as isize;
    let bwmap = img.bwmap.as_deref().unwrap_or(&[]);
    let stride = img.map_stride;

    for _ in 0..h {
        for _ in 0..w {
            raster.set(
                cp,
                tab(bwmap, px(buf, pp) as usize * stride)
                    & ((px(buf, pp + 1) as u32) << 24 | !A1),
            );
            cp += 1;
            pp += samplesperpixel;
        }
        cp += toskew as isize;
        pp += fromskew as isize;
    }
}

/// 16-bit greyscale => colormap/RGB
/// Translation of `put16bitbwtile()`.
fn put16bitbwtile(
    img: &TIFFRGBAImage,
    raster: &mut Raster<'_>,
    mut cp: isize,
    _x: u32,
    _y: u32,
    w: u32,
    h: u32,
    fromskew: i32,
    toskew: i32,
    buf: &[u8],
    mut pp: isize,
) {
    let samplesperpixel = img.samplesperpixel as isize;
    let bwmap = img.bwmap.as_deref().unwrap_or(&[]);
    let stride = img.map_stride;
    /* Convert pixel skew to byte skew (16-bit samples) */
    let fromskew_local = fromskew as TmSize * (2 * samplesperpixel);

    for _ in 0..h {
        let mut wp = pp;

        for _ in 0..w {
            /* use high order byte of 16bit value */

            raster.set(cp, tab(bwmap, (px16(buf, wp) >> 8) as usize * stride));
            cp += 1;
            pp += 2 * samplesperpixel;
            wp += 2 * samplesperpixel;
        }
        cp += toskew as isize;
        pp += fromskew_local;
    }
}

/// 1-bit bilevel => colormap/RGB
/// Translation of `put1bitbwtile()`.
fn put1bitbwtile(
    img: &TIFFRGBAImage,
    raster: &mut Raster<'_>,
    mut cp: isize,
    _x: u32,
    _y: u32,
    w: u32,
    h: u32,
    mut fromskew: i32,
    toskew: i32,
    buf: &[u8],
    mut pp: isize,
) {
    let bwmap = img.bwmap.as_deref().unwrap_or(&[]);

    fromskew /= 8;
    for _ in 0..h {
        unroll_map(raster, &mut cp, buf, &mut pp, bwmap, 8, w);
        cp += toskew as isize;
        pp += fromskew as isize;
    }
}

/// 2-bit greyscale => colormap/RGB
/// Translation of `put2bitbwtile()`.
fn put2bitbwtile(
    img: &TIFFRGBAImage,
    raster: &mut Raster<'_>,
    mut cp: isize,
    _x: u32,
    _y: u32,
    w: u32,
    h: u32,
    mut fromskew: i32,
    toskew: i32,
    buf: &[u8],
    mut pp: isize,
) {
    let bwmap = img.bwmap.as_deref().unwrap_or(&[]);

    fromskew /= 4;
    for _ in 0..h {
        unroll_map(raster, &mut cp, buf, &mut pp, bwmap, 4, w);
        cp += toskew as isize;
        pp += fromskew as isize;
    }
}

/// 4-bit greyscale => colormap/RGB
/// Translation of `put4bitbwtile()`.
fn put4bitbwtile(
    img: &TIFFRGBAImage,
    raster: &mut Raster<'_>,
    mut cp: isize,
    _x: u32,
    _y: u32,
    w: u32,
    h: u32,
    mut fromskew: i32,
    toskew: i32,
    buf: &[u8],
    mut pp: isize,
) {
    let bwmap = img.bwmap.as_deref().unwrap_or(&[]);

    fromskew /= 2;
    for _ in 0..h {
        unroll_map(raster, &mut cp, buf, &mut pp, bwmap, 2, w);
        cp += toskew as isize;
        pp += fromskew as isize;
    }
}

/// 8-bit packed samples, no Map => RGB
/// Translation of `putRGBcontig8bittile()`.
fn put_rgb_contig8bittile(
    img: &TIFFRGBAImage,
    raster: &mut Raster<'_>,
    mut cp: isize,
    _x: u32,
    _y: u32,
    w: u32,
    h: u32,
    fromskew: i32,
    toskew: i32,
    buf: &[u8],
    mut pp: isize,
) {
    let samplesperpixel = img.samplesperpixel as isize;

    let fromskew_local = fromskew as TmSize * samplesperpixel;
    for _ in 0..h {
        for _ in 0..w {
            raster.set(
                cp,
                pack(
                    px(buf, pp) as u32,
                    px(buf, pp + 1) as u32,
                    px(buf, pp + 2) as u32,
                ),
            );
            cp += 1;
            pp += samplesperpixel;
        }
        cp += toskew as isize;
        pp += fromskew_local;
    }
}

/// 8-bit packed samples => RGBA w/ associated alpha
/// (known to have Map == NULL)
/// Translation of `putRGBAAcontig8bittile()`.
fn put_rgbaa_contig8bittile(
    img: &TIFFRGBAImage,
    raster: &mut Raster<'_>,
    mut cp: isize,
    _x: u32,
    _y: u32,
    w: u32,
    h: u32,
    fromskew: i32,
    toskew: i32,
    buf: &[u8],
    mut pp: isize,
) {
    let samplesperpixel = img.samplesperpixel as isize;

    let fromskew_local = fromskew as TmSize * samplesperpixel;
    for _ in 0..h {
        for _ in 0..w {
            raster.set(
                cp,
                pack4(
                    px(buf, pp) as u32,
                    px(buf, pp + 1) as u32,
                    px(buf, pp + 2) as u32,
                    px(buf, pp + 3) as u32,
                ),
            );
            cp += 1;
            pp += samplesperpixel;
        }
        cp += toskew as isize;
        pp += fromskew_local;
    }
}

/// 8-bit packed samples => RGBA w/ unassociated alpha
/// (known to have Map == NULL)
/// Translation of `putRGBUAcontig8bittile()`.
fn put_rgbua_contig8bittile(
    img: &TIFFRGBAImage,
    raster: &mut Raster<'_>,
    mut cp: isize,
    _x: u32,
    _y: u32,
    w: u32,
    h: u32,
    fromskew: i32,
    toskew: i32,
    buf: &[u8],
    mut pp: isize,
) {
    let samplesperpixel = img.samplesperpixel as isize;
    let fromskew_local = fromskew as TmSize * samplesperpixel;
    for _ in 0..h {
        for _ in 0..w {
            let a = px(buf, pp + 3) as u32;
            let r = img.ua(a, px(buf, pp) as u32);
            let g = img.ua(a, px(buf, pp + 1) as u32);
            let b = img.ua(a, px(buf, pp + 2) as u32);
            raster.set(cp, pack4(r, g, b, a));
            cp += 1;
            pp += samplesperpixel;
        }
        cp += toskew as isize;
        pp += fromskew_local;
    }
}

/// 16-bit packed samples => RGB
/// Translation of `putRGBcontig16bittile()`.
fn put_rgb_contig16bittile(
    img: &TIFFRGBAImage,
    raster: &mut Raster<'_>,
    mut cp: isize,
    _x: u32,
    _y: u32,
    w: u32,
    h: u32,
    fromskew: i32,
    toskew: i32,
    buf: &[u8],
    pp: isize,
) {
    let samplesperpixel = img.samplesperpixel as isize;
    let mut wp = pp;
    let fromskew_local = fromskew as TmSize * samplesperpixel;
    for _ in 0..h {
        for _ in 0..w {
            raster.set(
                cp,
                pack(
                    img.map16(px16(buf, wp)),
                    img.map16(px16(buf, wp + 2)),
                    img.map16(px16(buf, wp + 4)),
                ),
            );
            cp += 1;
            wp += 2 * samplesperpixel;
        }
        cp += toskew as isize;
        wp += 2 * fromskew_local;
    }
}

/// 16-bit packed samples => RGBA w/ associated alpha
/// (known to have Map == NULL)
/// Translation of `putRGBAAcontig16bittile()`.
fn put_rgbaa_contig16bittile(
    img: &TIFFRGBAImage,
    raster: &mut Raster<'_>,
    mut cp: isize,
    _x: u32,
    _y: u32,
    w: u32,
    h: u32,
    fromskew: i32,
    toskew: i32,
    buf: &[u8],
    pp: isize,
) {
    let samplesperpixel = img.samplesperpixel as isize;
    let mut wp = pp;
    let fromskew_local = fromskew as TmSize * samplesperpixel;
    for _ in 0..h {
        for _ in 0..w {
            raster.set(
                cp,
                pack4(
                    img.map16(px16(buf, wp)),
                    img.map16(px16(buf, wp + 2)),
                    img.map16(px16(buf, wp + 4)),
                    img.map16(px16(buf, wp + 6)),
                ),
            );
            cp += 1;
            wp += 2 * samplesperpixel;
        }
        cp += toskew as isize;
        wp += 2 * fromskew_local;
    }
}

/// 16-bit packed samples => RGBA w/ unassociated alpha
/// (known to have Map == NULL)
/// Translation of `putRGBUAcontig16bittile()`.
fn put_rgbua_contig16bittile(
    img: &TIFFRGBAImage,
    raster: &mut Raster<'_>,
    mut cp: isize,
    _x: u32,
    _y: u32,
    w: u32,
    h: u32,
    fromskew: i32,
    toskew: i32,
    buf: &[u8],
    pp: isize,
) {
    let samplesperpixel = img.samplesperpixel as isize;
    let mut wp = pp;
    let fromskew_local = fromskew as TmSize * samplesperpixel;
    for _ in 0..h {
        for _ in 0..w {
            let a = img.map16(px16(buf, wp + 6));
            let r = img.ua(a, img.map16(px16(buf, wp)));
            let g = img.ua(a, img.map16(px16(buf, wp + 2)));
            let b = img.ua(a, img.map16(px16(buf, wp + 4)));
            raster.set(cp, pack4(r, g, b, a));
            cp += 1;
            wp += 2 * samplesperpixel;
        }
        cp += toskew as isize;
        wp += 2 * fromskew_local;
    }
}

/// 8-bit packed CMYK samples w/o Map => RGB
///
/// NB: The conversion of CMYK->RGB is *very* crude.
/// Translation of `putRGBcontig8bitCMYKtile()`.
fn put_rgb_contig8bit_cmyk_tile(
    img: &TIFFRGBAImage,
    raster: &mut Raster<'_>,
    mut cp: isize,
    _x: u32,
    _y: u32,
    w: u32,
    h: u32,
    fromskew: i32,
    toskew: i32,
    buf: &[u8],
    mut pp: isize,
) {
    let samplesperpixel = img.samplesperpixel as isize;

    let fromskew_local = fromskew as TmSize * samplesperpixel;
    for _ in 0..h {
        for _ in 0..w {
            let k = 255 - px(buf, pp + 3) as u32;
            let r = (k * (255 - px(buf, pp) as u32)) / 255;
            let g = (k * (255 - px(buf, pp + 1) as u32)) / 255;
            let b = (k * (255 - px(buf, pp + 2) as u32)) / 255;
            raster.set(cp, pack(r, g, b));
            cp += 1;
            pp += samplesperpixel;
        }
        cp += toskew as isize;
        pp += fromskew_local;
    }
}

/// 8-bit packed CMYK samples w/Map => RGB
///
/// NB: The conversion of CMYK->RGB is *very* crude.
/// Translation of `putRGBcontig8bitCMYKMaptile()`.
fn put_rgb_contig8bit_cmyk_map_tile(
    img: &TIFFRGBAImage,
    raster: &mut Raster<'_>,
    mut cp: isize,
    _x: u32,
    _y: u32,
    w: u32,
    h: u32,
    fromskew: i32,
    toskew: i32,
    buf: &[u8],
    mut pp: isize,
) {
    let samplesperpixel = img.samplesperpixel as isize;
    let map = img.map.as_deref().unwrap_or(&[]);

    let fromskew_local = fromskew as TmSize * samplesperpixel;
    for _ in 0..h {
        for _ in 0..w {
            let k = 255u32 - px(buf, pp + 3) as u32;
            let r = (k * (255u32 - px(buf, pp) as u32)) / 255u32;
            let g = (k * (255u32 - px(buf, pp + 1) as u32)) / 255u32;
            let b = (k * (255u32 - px(buf, pp + 2) as u32)) / 255u32;
            raster.set(
                cp,
                pack(
                    tab(map, r as usize) as u32,
                    tab(map, g as usize) as u32,
                    tab(map, b as usize) as u32,
                ),
            );
            cp += 1;
            pp += samplesperpixel;
        }
        pp += fromskew_local;
        cp += toskew as isize;
    }
}

/// 8-bit unpacked samples => RGB
/// Translation of `putRGBseparate8bittile()`.
fn put_rgb_separate8bittile(
    _img: &TIFFRGBAImage,
    raster: &mut Raster<'_>,
    mut cp: isize,
    _x: u32,
    _y: u32,
    w: u32,
    h: u32,
    fromskew: i32,
    toskew: i32,
    buf: &[u8],
    mut r: isize,
    mut g: isize,
    mut b: isize,
    _a: Option<isize>,
) {
    for _ in 0..h {
        for _ in 0..w {
            raster.set(
                cp,
                pack(px(buf, r) as u32, px(buf, g) as u32, px(buf, b) as u32),
            );
            cp += 1;
            r += 1;
            g += 1;
            b += 1;
        }
        r += fromskew as isize;
        g += fromskew as isize;
        b += fromskew as isize;
        cp += toskew as isize;
    }
}

/// 8-bit unpacked samples => RGBA w/ associated alpha
/// Translation of `putRGBAAseparate8bittile()`.
fn put_rgbaa_separate8bittile(
    _img: &TIFFRGBAImage,
    raster: &mut Raster<'_>,
    mut cp: isize,
    _x: u32,
    _y: u32,
    w: u32,
    h: u32,
    fromskew: i32,
    toskew: i32,
    buf: &[u8],
    mut r: isize,
    mut g: isize,
    mut b: isize,
    a: Option<isize>,
) {
    let Some(mut a) = a else {
        return;
    };
    for _ in 0..h {
        for _ in 0..w {
            raster.set(
                cp,
                pack4(
                    px(buf, r) as u32,
                    px(buf, g) as u32,
                    px(buf, b) as u32,
                    px(buf, a) as u32,
                ),
            );
            cp += 1;
            r += 1;
            g += 1;
            b += 1;
            a += 1;
        }
        r += fromskew as isize;
        g += fromskew as isize;
        b += fromskew as isize;
        a += fromskew as isize;
        cp += toskew as isize;
    }
}

/// 8-bit unpacked CMYK samples => RGBA
/// Translation of `putCMYKseparate8bittile()`.
fn put_cmyk_separate8bittile(
    _img: &TIFFRGBAImage,
    raster: &mut Raster<'_>,
    mut cp: isize,
    _x: u32,
    _y: u32,
    w: u32,
    h: u32,
    fromskew: i32,
    toskew: i32,
    buf: &[u8],
    mut r: isize,
    mut g: isize,
    mut b: isize,
    a: Option<isize>,
) {
    let Some(mut a) = a else {
        return;
    };
    for _ in 0..h {
        for _ in 0..w {
            let kv = 255u32 - px(buf, a) as u32;
            a += 1;
            let rv = (kv * (255u32 - px(buf, r) as u32)) / 255u32;
            r += 1;
            let gv = (kv * (255u32 - px(buf, g) as u32)) / 255u32;
            g += 1;
            let bv = (kv * (255u32 - px(buf, b) as u32)) / 255u32;
            b += 1;
            raster.set(cp, pack4(rv, gv, bv, 255));
            cp += 1;
        }
        r += fromskew as isize;
        g += fromskew as isize;
        b += fromskew as isize;
        a += fromskew as isize;
        cp += toskew as isize;
    }
}

/// 8-bit unpacked samples => RGBA w/ unassociated alpha
/// Translation of `putRGBUAseparate8bittile()`.
fn put_rgbua_separate8bittile(
    img: &TIFFRGBAImage,
    raster: &mut Raster<'_>,
    mut cp: isize,
    _x: u32,
    _y: u32,
    w: u32,
    h: u32,
    fromskew: i32,
    toskew: i32,
    buf: &[u8],
    mut r: isize,
    mut g: isize,
    mut b: isize,
    a: Option<isize>,
) {
    let Some(mut a) = a else {
        return;
    };
    for _ in 0..h {
        for _ in 0..w {
            let av = px(buf, a) as u32;
            a += 1;
            let rv = img.ua(av, px(buf, r) as u32);
            r += 1;
            let gv = img.ua(av, px(buf, g) as u32);
            g += 1;
            let bv = img.ua(av, px(buf, b) as u32);
            b += 1;
            raster.set(cp, pack4(rv, gv, bv, av));
            cp += 1;
        }
        r += fromskew as isize;
        g += fromskew as isize;
        b += fromskew as isize;
        a += fromskew as isize;
        cp += toskew as isize;
    }
}

/// 16-bit unpacked samples => RGB
/// Translation of `putRGBseparate16bittile()`.
fn put_rgb_separate16bittile(
    img: &TIFFRGBAImage,
    raster: &mut Raster<'_>,
    mut cp: isize,
    _x: u32,
    _y: u32,
    w: u32,
    h: u32,
    fromskew: i32,
    toskew: i32,
    buf: &[u8],
    r: isize,
    g: isize,
    b: isize,
    _a: Option<isize>,
) {
    let (mut wr, mut wg, mut wb) = (r, g, b);
    for _ in 0..h {
        for _ in 0..w {
            raster.set(
                cp,
                pack(
                    img.map16(px16(buf, wr)),
                    img.map16(px16(buf, wg)),
                    img.map16(px16(buf, wb)),
                ),
            );
            cp += 1;
            wr += 2;
            wg += 2;
            wb += 2;
        }
        wr += 2 * fromskew as isize;
        wg += 2 * fromskew as isize;
        wb += 2 * fromskew as isize;
        cp += toskew as isize;
    }
}

/// 16-bit unpacked samples => RGBA w/ associated alpha
/// Translation of `putRGBAAseparate16bittile()`.
fn put_rgbaa_separate16bittile(
    img: &TIFFRGBAImage,
    raster: &mut Raster<'_>,
    mut cp: isize,
    _x: u32,
    _y: u32,
    w: u32,
    h: u32,
    fromskew: i32,
    toskew: i32,
    buf: &[u8],
    r: isize,
    g: isize,
    b: isize,
    a: Option<isize>,
) {
    let Some(a) = a else {
        return;
    };
    let (mut wr, mut wg, mut wb, mut wa) = (r, g, b, a);
    for _ in 0..h {
        for _ in 0..w {
            raster.set(
                cp,
                pack4(
                    img.map16(px16(buf, wr)),
                    img.map16(px16(buf, wg)),
                    img.map16(px16(buf, wb)),
                    img.map16(px16(buf, wa)),
                ),
            );
            cp += 1;
            wr += 2;
            wg += 2;
            wb += 2;
            wa += 2;
        }
        wr += 2 * fromskew as isize;
        wg += 2 * fromskew as isize;
        wb += 2 * fromskew as isize;
        wa += 2 * fromskew as isize;
        cp += toskew as isize;
    }
}

/// 16-bit unpacked samples => RGBA w/ unassociated alpha
/// Translation of `putRGBUAseparate16bittile()`.
fn put_rgbua_separate16bittile(
    img: &TIFFRGBAImage,
    raster: &mut Raster<'_>,
    mut cp: isize,
    _x: u32,
    _y: u32,
    w: u32,
    h: u32,
    fromskew: i32,
    toskew: i32,
    buf: &[u8],
    r: isize,
    g: isize,
    b: isize,
    a: Option<isize>,
) {
    let Some(a) = a else {
        return;
    };
    let (mut wr, mut wg, mut wb, mut wa) = (r, g, b, a);
    for _ in 0..h {
        for _ in 0..w {
            let a2 = img.map16(px16(buf, wa));
            wa += 2;
            let r2 = img.ua(a2, img.map16(px16(buf, wr)));
            wr += 2;
            let g2 = img.ua(a2, img.map16(px16(buf, wg)));
            wg += 2;
            let b2 = img.ua(a2, img.map16(px16(buf, wb)));
            wb += 2;
            raster.set(cp, pack4(r2, g2, b2, a2));
            cp += 1;
        }
        wr += 2 * fromskew as isize;
        wg += 2 * fromskew as isize;
        wb += 2 * fromskew as isize;
        wa += 2 * fromskew as isize;
        cp += toskew as isize;
    }
}

/// 8-bit packed CIE L*a*b 1976 samples => RGB
/// Translation of `putcontig8bitCIELab8()`.
fn putcontig8bit_cielab8(
    img: &TIFFRGBAImage,
    raster: &mut Raster<'_>,
    mut cp: isize,
    _x: u32,
    _y: u32,
    w: u32,
    h: u32,
    fromskew: i32,
    toskew: i32,
    buf: &[u8],
    mut pp: isize,
) {
    let Some(cielab) = img.cielab.as_deref() else {
        return;
    };
    let fromskew_local = fromskew as TmSize * 3;
    for _ in 0..h {
        for _ in 0..w {
            let (x, y, z) = tiff_cie_lab_to_xyz(
                cielab,
                px(buf, pp) as u32,
                px(buf, pp + 1) as i8 as i32,
                px(buf, pp + 2) as i8 as i32,
            );
            let (r, g, b) = tiff_xyz_to_rgb(cielab, x, y, z);
            raster.set(cp, pack(r, g, b));
            cp += 1;
            pp += 3;
        }
        cp += toskew as isize;
        pp += fromskew_local;
    }
}

/// 16-bit packed CIE L*a*b 1976 samples => RGB
/// Translation of `putcontig8bitCIELab16()`.
fn putcontig8bit_cielab16(
    img: &TIFFRGBAImage,
    raster: &mut Raster<'_>,
    mut cp: isize,
    _x: u32,
    _y: u32,
    w: u32,
    h: u32,
    fromskew: i32,
    toskew: i32,
    buf: &[u8],
    pp: isize,
) {
    let Some(cielab) = img.cielab.as_deref() else {
        return;
    };
    let mut wp = pp;
    let fromskew_local = fromskew as TmSize * 3;
    for _ in 0..h {
        for _ in 0..w {
            let (x, y, z) = tiff_cie_lab16_to_xyz(
                cielab,
                px16(buf, wp) as u32,
                px16(buf, wp + 2) as i16 as i32,
                px16(buf, wp + 4) as i16 as i32,
            );
            let (r, g, b) = tiff_xyz_to_rgb(cielab, x, y, z);
            raster.set(cp, pack(r, g, b));
            cp += 1;
            wp += 2 * 3;
        }
        cp += toskew as isize;
        wp += 2 * fromskew_local;
    }
}

/*
 * YCbCr -> RGB conversion and packing routines.
 */

/// `YCbCrtoRGB(dst, Y)`: `raster[dst]` becomes the pixel.
fn ycbcr_to_rgb(img: &TIFFRGBAImage, raster: &mut Raster<'_>, dst: isize, y: u32, cb: i32, cr: i32) {
    let (r, g, b) = match img.ycbcr.as_deref() {
        Some(ycbcr) => tiff_ycbcr_to_rgb(ycbcr, y, cb, cr),
        None => (0, 0, 0),
    };
    raster.set(dst, pack(r, g, b));
}

/// One block of the 4,4 and 4,2 routines' general case: the first `nx`
/// columns of the first `nh` rows (`rows[]` the rows' raster indices), from
/// the block's luma samples (`bw` of them a row) and the chroma after
/// them.
fn ycbcr_block(
    img: &TIFFRGBAImage,
    raster: &mut Raster<'_>,
    rows: &[isize],
    buf: &[u8],
    pp: isize,
    bw: usize,
    nx: usize,
    nh: usize,
) {
    let nluma = (bw * rows.len()) as isize;
    let cb = px(buf, pp + nluma) as i32;
    let cr = px(buf, pp + nluma + 1) as i32;
    for dx in 0..nx {
        for (dy, &row) in rows.iter().enumerate().take(nh) {
            let y = px(buf, pp + (dy * bw + dx) as isize) as u32;
            ycbcr_to_rgb(img, raster, row + dx as isize, y, cb, cr);
        }
    }
}

/// 8-bit packed YCbCr samples w/ 4,4 subsampling => RGB
/// Translation of `putcontig8bitYCbCr44tile()`.
fn putcontig8bit_ycbcr44tile(
    img: &TIFFRGBAImage,
    raster: &mut Raster<'_>,
    mut cp: isize,
    _x: u32,
    _y: u32,
    w: u32,
    mut h: u32,
    fromskew: i32,
    toskew: i32,
    buf: &[u8],
    mut pp: isize,
) {
    let mut cp1 = cp + w as isize + toskew as isize;
    let mut cp2 = cp1 + w as isize + toskew as isize;
    let mut cp3 = cp2 + w as isize + toskew as isize;
    let incr = 3 * w as TmSize + 4 * toskew as TmSize;

    /* adjust fromskew */
    let fromskew_local = (fromskew / 4) as TmSize * (4 * 4 + 2);
    if (h & 3) == 0 && (w & 3) == 0 {
        while h >= 4 {
            let mut x = w >> 2;
            loop {
                ycbcr_block(img, raster, &[cp, cp1, cp2, cp3], buf, pp, 4, 4, 4);

                cp += 4;
                cp1 += 4;
                cp2 += 4;
                cp3 += 4;
                pp += 18;
                x = x.wrapping_sub(1);
                if x == 0 {
                    break;
                }
            }
            cp += incr;
            cp1 += incr;
            cp2 += incr;
            cp3 += incr;
            pp += fromskew_local;
            h -= 4;
        }
    } else {
        while h > 0 {
            let mut x = w;
            while x > 0 {
                ycbcr_block(
                    img,
                    raster,
                    &[cp, cp1, cp2, cp3],
                    buf,
                    pp,
                    4,
                    x.min(4) as usize,
                    h.min(4) as usize,
                );
                if x < 4 {
                    cp += x as isize;
                    cp1 += x as isize;
                    cp2 += x as isize;
                    cp3 += x as isize;
                    x = 0;
                } else {
                    cp += 4;
                    cp1 += 4;
                    cp2 += 4;
                    cp3 += 4;
                    x -= 4;
                }
                pp += 18;
            }
            if h <= 4 {
                break;
            }
            h -= 4;
            cp += incr;
            cp1 += incr;
            cp2 += incr;
            cp3 += incr;
            pp += fromskew_local;
        }
    }
}

/// 8-bit packed YCbCr samples w/ 4,2 subsampling => RGB
/// Translation of `putcontig8bitYCbCr42tile()`.
fn putcontig8bit_ycbcr42tile(
    img: &TIFFRGBAImage,
    raster: &mut Raster<'_>,
    mut cp: isize,
    _x: u32,
    _y: u32,
    w: u32,
    mut h: u32,
    fromskew: i32,
    toskew: i32,
    buf: &[u8],
    mut pp: isize,
) {
    let mut cp1 = cp + w as isize + toskew as isize;
    let incr = 2 * toskew as TmSize + w as TmSize;

    let fromskew_local = (fromskew / 4) as TmSize * (4 * 2 + 2);
    if (w & 3) == 0 && (h & 1) == 0 {
        while h >= 2 {
            let mut x = w >> 2;
            loop {
                ycbcr_block(img, raster, &[cp, cp1], buf, pp, 4, 4, 2);

                cp += 4;
                cp1 += 4;
                pp += 10;
                x = x.wrapping_sub(1);
                if x == 0 {
                    break;
                }
            }
            cp += incr;
            cp1 += incr;
            pp += fromskew_local;
            h -= 2;
        }
    } else {
        while h > 0 {
            let mut x = w;
            while x > 0 {
                ycbcr_block(
                    img,
                    raster,
                    &[cp, cp1],
                    buf,
                    pp,
                    4,
                    x.min(4) as usize,
                    h.min(2) as usize,
                );
                if x < 4 {
                    cp += x as isize;
                    cp1 += x as isize;
                    x = 0;
                } else {
                    cp += 4;
                    cp1 += 4;
                    x -= 4;
                }
                pp += 10;
            }
            if h <= 2 {
                break;
            }
            h -= 2;
            cp += incr;
            cp1 += incr;
            pp += fromskew_local;
        }
    }
}

/// 8-bit packed YCbCr samples w/ 4,1 subsampling => RGB
/// Translation of `putcontig8bitYCbCr41tile()`.
fn putcontig8bit_ycbcr41tile(
    img: &TIFFRGBAImage,
    raster: &mut Raster<'_>,
    mut cp: isize,
    _x: u32,
    _y: u32,
    w: u32,
    mut h: u32,
    fromskew: i32,
    toskew: i32,
    buf: &[u8],
    mut pp: isize,
) {
    let fromskew_local = (fromskew / 4) as TmSize * (4 + 2);
    loop {
        let mut x = w >> 2;
        while x > 0 {
            ycbcr_block(img, raster, &[cp], buf, pp, 4, 4, 1);

            cp += 4;
            pp += 6;
            x -= 1;
        }

        if (w & 3) != 0 {
            ycbcr_block(img, raster, &[cp], buf, pp, 4, (w & 3) as usize, 1);

            cp += (w & 3) as isize;
            pp += 6;
        }

        cp += toskew as isize;
        pp += fromskew_local;
        h = h.wrapping_sub(1);
        if h == 0 {
            break;
        }
    }
}

/// 8-bit packed YCbCr samples w/ 2,2 subsampling => RGB
/// Translation of `putcontig8bitYCbCr22tile()`.
fn putcontig8bit_ycbcr22tile(
    img: &TIFFRGBAImage,
    raster: &mut Raster<'_>,
    mut cp: isize,
    _x: u32,
    _y: u32,
    w: u32,
    mut h: u32,
    fromskew: i32,
    toskew: i32,
    buf: &[u8],
    mut pp: isize,
) {
    let incr = 2 * toskew as TmSize + w as TmSize;
    let fromskew_local = (fromskew / 2) as TmSize * (2 * 2 + 2);
    let mut cp2 = cp + w as isize + toskew as isize;
    while h >= 2 {
        let mut x = w;
        while x >= 2 {
            ycbcr_block(img, raster, &[cp, cp2], buf, pp, 2, 2, 2);
            cp += 2;
            cp2 += 2;
            pp += 6;
            x -= 2;
        }
        if x == 1 {
            ycbcr_block(img, raster, &[cp, cp2], buf, pp, 2, 1, 2);
            cp += 1;
            cp2 += 1;
            pp += 6;
        }
        cp += incr;
        cp2 += incr;
        pp += fromskew_local;
        h -= 2;
    }
    if h == 1 {
        let mut x = w;
        while x >= 2 {
            ycbcr_block(img, raster, &[cp, cp2], buf, pp, 2, 2, 1);
            cp += 2;
            cp2 += 2;
            pp += 6;
            x -= 2;
        }
        if x == 1 {
            ycbcr_block(img, raster, &[cp, cp2], buf, pp, 2, 1, 1);
        }
    }
}

/// 8-bit packed YCbCr samples w/ 2,1 subsampling => RGB
/// Translation of `putcontig8bitYCbCr21tile()`.
fn putcontig8bit_ycbcr21tile(
    img: &TIFFRGBAImage,
    raster: &mut Raster<'_>,
    mut cp: isize,
    _x: u32,
    _y: u32,
    w: u32,
    mut h: u32,
    fromskew: i32,
    toskew: i32,
    buf: &[u8],
    mut pp: isize,
) {
    let fromskew_local = (fromskew / 2) as TmSize * (2 + 2);
    loop {
        let mut x = w >> 1;
        while x > 0 {
            ycbcr_block(img, raster, &[cp], buf, pp, 2, 2, 1);

            cp += 2;
            pp += 4;
            x -= 1;
        }

        if (w & 1) != 0 {
            ycbcr_block(img, raster, &[cp], buf, pp, 2, 1, 1);

            cp += 1;
            pp += 4;
        }

        cp += toskew as isize;
        pp += fromskew_local;
        h = h.wrapping_sub(1);
        if h == 0 {
            break;
        }
    }
}

/// 8-bit packed YCbCr samples w/ 1,2 subsampling => RGB
/// Translation of `putcontig8bitYCbCr12tile()`.
fn putcontig8bit_ycbcr12tile(
    img: &TIFFRGBAImage,
    raster: &mut Raster<'_>,
    mut cp: isize,
    _x: u32,
    _y: u32,
    w: u32,
    mut h: u32,
    fromskew: i32,
    toskew: i32,
    buf: &[u8],
    mut pp: isize,
) {
    let incr = 2 * toskew as TmSize + w as TmSize;
    let fromskew_local = fromskew as TmSize * (2 + 2);
    let mut cp2 = cp + w as isize + toskew as isize;
    while h >= 2 {
        let mut x = w;
        loop {
            ycbcr_block(img, raster, &[cp, cp2], buf, pp, 1, 1, 2);
            cp += 1;
            cp2 += 1;
            pp += 4;
            x = x.wrapping_sub(1);
            if x == 0 {
                break;
            }
        }
        cp += incr;
        cp2 += incr;
        pp += fromskew_local;
        h -= 2;
    }
    if h == 1 {
        let mut x = w;
        loop {
            ycbcr_block(img, raster, &[cp, cp2], buf, pp, 1, 1, 1);
            cp += 1;
            pp += 4;
            x = x.wrapping_sub(1);
            if x == 0 {
                break;
            }
        }
    }
}

/// 8-bit packed YCbCr samples w/ no subsampling => RGB
/// Translation of `putcontig8bitYCbCr11tile()`.
fn putcontig8bit_ycbcr11tile(
    img: &TIFFRGBAImage,
    raster: &mut Raster<'_>,
    mut cp: isize,
    _x: u32,
    _y: u32,
    w: u32,
    mut h: u32,
    fromskew: i32,
    toskew: i32,
    buf: &[u8],
    mut pp: isize,
) {
    let fromskew_local = fromskew as TmSize * (1 + 2);
    loop {
        let mut x = w; /* was x = w>>1; patched 2000/09/25 warmerda@home.com */
        loop {
            ycbcr_block(img, raster, &[cp], buf, pp, 1, 1, 1);
            cp += 1;

            pp += 3;
            x = x.wrapping_sub(1);
            if x == 0 {
                break;
            }
        }
        cp += toskew as isize;
        pp += fromskew_local;
        h = h.wrapping_sub(1);
        if h == 0 {
            break;
        }
    }
}

/// 8-bit packed YCbCr samples w/ no subsampling => RGB
/// Translation of `putseparate8bitYCbCr11tile()`.
fn putseparate8bit_ycbcr11tile(
    img: &TIFFRGBAImage,
    raster: &mut Raster<'_>,
    mut cp: isize,
    _x: u32,
    _y: u32,
    w: u32,
    h: u32,
    fromskew: i32,
    toskew: i32,
    buf: &[u8],
    mut r: isize,
    mut g: isize,
    mut b: isize,
    _a: Option<isize>,
) {
    /* TODO: naming of input vars is still off, change obfuscating declaration
     * inside define, or resolve obfuscation */
    for _ in 0..h {
        let mut x = w;
        loop {
            ycbcr_to_rgb(
                img,
                raster,
                cp,
                px(buf, r) as u32,
                px(buf, g) as i32,
                px(buf, b) as i32,
            );
            cp += 1;
            r += 1;
            g += 1;
            b += 1;
            x = x.wrapping_sub(1);
            if x == 0 {
                break;
            }
        }
        r += fromskew as isize;
        g += fromskew as isize;
        b += fromskew as isize;
        cp += toskew as isize;
    }
}

/// Translation of `isInRefBlackWhiteRange()`.
fn is_in_ref_black_white_range(f: f32) -> bool {
    f > (-0x7FFFFFFF + 128) as f32 && f < 0x7FFFFFFF as f32
}

/// Translation of `initYCbCrConversion()`.
fn init_ycbcr_conversion(img: &mut TIFFRGBAImage, tif: &mut Tiff<'_>) -> i32 {
    const MODULE: &str = "initYCbCrConversion";

    if img.ycbcr.is_none() {
        img.ycbcr = Some(Box::default());
    }

    let mut out = Vec::new();
    tiff_get_field_defaulted(tif, TIFFTAG_YCBCRCOEFFICIENTS, &mut out);
    let luma_v = out.first().and_then(Gv::as_f32s).unwrap_or_default();
    let luma = [tab(&luma_v, 0), tab(&luma_v, 1), tab(&luma_v, 2)];
    let mut out = Vec::new();
    tiff_get_field_defaulted(tif, TIFFTAG_REFERENCEBLACKWHITE, &mut out);
    let rbw_v = out.first().and_then(Gv::as_f32s).unwrap_or_default();
    let mut ref_black_white = [0f32; 6];
    for (i, v) in ref_black_white.iter_mut().enumerate() {
        *v = tab(&rbw_v, i);
    }

    /* Do some validation to avoid later issues. Detect NaN for now */
    /* and also if lumaGreen is zero since we divide by it later */
    if luma[0].is_nan() || luma[1].is_nan() || !(luma[1].abs() > 0.0f32) || luma[2].is_nan() {
        tiff_error_ext_r!(MODULE, "Invalid values for YCbCrCoefficients tag");
        return 0;
    }

    if !ref_black_white.iter().all(|&f| is_in_ref_black_white_range(f)) {
        tiff_error_ext_r!(MODULE, "Invalid values for ReferenceBlackWhite tag");
        return 0;
    }

    let Some(ycbcr) = img.ycbcr.as_deref_mut() else {
        return 0;
    };
    if tiff_ycbcr_to_rgb_init(ycbcr, &luma, &ref_black_white) < 0 {
        return 0;
    }
    1
}

/// Translation of `initCIELabConversion()`.
fn init_cielab_conversion(img: &mut TIFFRGBAImage, tif: &mut Tiff<'_>) -> Option<TileContigRoutine> {
    const MODULE: &str = "initCIELabConversion";

    let mut out = Vec::new();
    tiff_get_field_defaulted(tif, TIFFTAG_WHITEPOINT, &mut out);
    let wp = out.first().and_then(Gv::as_f32s).unwrap_or_default();
    let white_point = [tab(&wp, 0), tab(&wp, 1)];
    if !(white_point[1].abs() > 0.0f32) {
        tiff_error_ext_r!(MODULE, "Invalid value for WhitePoint tag.");
        return None;
    }

    if img.cielab.is_none() {
        img.cielab = Some(Box::default());
    }

    let mut ref_white = [0f32; 3];
    ref_white[1] = 100.0f32;
    ref_white[0] = white_point[0] / white_point[1] * ref_white[1];
    ref_white[2] = (1.0f32 - white_point[0] - white_point[1]) / white_point[1] * ref_white[1];
    let cielab = img.cielab.as_deref_mut()?;
    if tiff_cie_lab_to_rgb_init(cielab, &DISPLAY_SRGB, &ref_white) < 0 {
        tiff_error_ext_r!(
            MODULE,
            "Failed to initialize CIE L*a*b*->RGB conversion state."
        );
        img.cielab = None;
        return None;
    }

    if img.bitspersample == 8 {
        return Some(putcontig8bit_cielab8);
    } else if img.bitspersample == 16 {
        return Some(putcontig8bit_cielab16);
    }
    None
}

/// Greyscale images with less than 8 bits/sample are handled
/// with a table to avoid lots of shifts and masks.  The table
/// is setup so that put*bwtile (below) can retrieve 8/bitspersample
/// pixel values simply by indexing into the table with one
/// number.
/// Translation of `makebwmap()`.
fn makebwmap(img: &mut TIFFRGBAImage, tif: &Tiff<'_>) -> i32 {
    let map = img.map.as_deref().unwrap_or(&[]);
    let bitspersample = img.bitspersample as i32;
    let mut nsamples = 8 / bitspersample;

    if nsamples == 0 {
        nsamples = 1;
    }

    let Some(mut bwmap) = try_vec::<u32>(256 * nsamples as usize) else {
        tiff_error_ext_r!(&tif.tif_name, "No space for B&W mapping table");
        return 0;
    };
    let mut p = 0usize;
    let mut grey = |x: usize| {
        let c = tab(map, x) as u32;
        bwmap[p] = pack(c, c, c);
        p += 1;
    };
    for i in 0..256usize {
        match bitspersample {
            1 => {
                grey(i >> 7);
                grey((i >> 6) & 1);
                grey((i >> 5) & 1);
                grey((i >> 4) & 1);
                grey((i >> 3) & 1);
                grey((i >> 2) & 1);
                grey((i >> 1) & 1);
                grey(i & 1);
            }
            2 => {
                grey(i >> 6);
                grey((i >> 4) & 3);
                grey((i >> 2) & 3);
                grey(i & 3);
            }
            4 => {
                grey(i >> 4);
                grey(i & 0xf);
            }
            8 | 16 => grey(i),
            _ => {}
        }
    }
    img.bwmap = Some(bwmap);
    img.map_stride = nsamples as usize;
    1
}

/// Construct a mapping table to convert from the range
/// of the data samples to [0,255] --for display.  This
/// process also handles inverting B&W images when needed.
/// Translation of `setupMap()`.
fn setup_map(img: &mut TIFFRGBAImage, tif: &Tiff<'_>) -> i32 {
    let mut range = ((1u32 << img.bitspersample) - 1) as i32;

    /* treat 16 bit the same as eight bit */
    if img.bitspersample == 16 {
        range = 255;
    }

    let Some(mut map) = try_vec::<TIFFRGBValue>((range + 1) as usize) else {
        tiff_error_ext_r!(&tif.tif_name, "No space for photometric conversion table");
        return 0;
    };
    if img.photometric == PHOTOMETRIC_MINISWHITE {
        for x in 0..=range {
            map[x as usize] = (((range - x) * 255) / range) as TIFFRGBValue;
        }
    } else {
        for x in 0..=range {
            map[x as usize] = ((x * 255) / range) as TIFFRGBValue;
        }
    }
    img.map = Some(map);
    if img.bitspersample <= 16
        && (img.photometric == PHOTOMETRIC_MINISBLACK
            || img.photometric == PHOTOMETRIC_MINISWHITE)
    {
        /*
         * Use photometric mapping table to construct
         * unpacking tables for samples <= 8 bits.
         */
        if makebwmap(img, tif) == 0 {
            return 0;
        }
        /* no longer need Map, free it */
        img.map = None;
    }
    1
}

/// Translation of `checkcmap()`.
fn checkcmap(img: &TIFFRGBAImage) -> i32 {
    let r = img.redcmap.as_deref().unwrap_or(&[]);
    let g = img.greencmap.as_deref().unwrap_or(&[]);
    let b = img.bluecmap.as_deref().unwrap_or(&[]);
    let n = 1usize << img.bitspersample;

    for i in 0..n {
        if tab(r, i) >= 256 || tab(g, i) >= 256 || tab(b, i) >= 256 {
            return 16;
        }
    }
    8
}

/// Translation of `cvtcmap()`.
fn cvtcmap(img: &mut TIFFRGBAImage) {
    let n = 1usize << img.bitspersample;
    for map in [&mut img.redcmap, &mut img.greencmap, &mut img.bluecmap] {
        if let Some(m) = map.as_deref_mut() {
            for v in m.iter_mut().take(n) {
                *v >>= 8;
            }
        }
    }
}

/// Palette images with <= 8 bits/sample are handled
/// with a table to avoid lots of shifts and masks.  The table
/// is setup so that put*cmaptile (below) can retrieve 8/bitspersample
/// pixel values simply by indexing into the table with one
/// number.
/// Translation of `makecmap()`.
fn makecmap(img: &mut TIFFRGBAImage, tif: &Tiff<'_>) -> i32 {
    let bitspersample = img.bitspersample as i32;
    let nsamples = (8 / bitspersample).max(1);
    let r = img.redcmap.as_deref().unwrap_or(&[]);
    let g = img.greencmap.as_deref().unwrap_or(&[]);
    let b = img.bluecmap.as_deref().unwrap_or(&[]);

    let Some(mut palmap) = try_vec::<u32>(256 * nsamples as usize) else {
        tiff_error_ext_r!(&tif.tif_name, "No space for Palette mapping table");
        return 0;
    };
    let mut p = 0usize;
    let mut cmap = |x: usize| {
        let c = x as TIFFRGBValue as usize;
        palmap[p] = pack(
            (tab(r, c) & 0xff) as u32,
            (tab(g, c) & 0xff) as u32,
            (tab(b, c) & 0xff) as u32,
        );
        p += 1;
    };
    for i in 0..256usize {
        match bitspersample {
            1 => {
                cmap(i >> 7);
                cmap((i >> 6) & 1);
                cmap((i >> 5) & 1);
                cmap((i >> 4) & 1);
                cmap((i >> 3) & 1);
                cmap((i >> 2) & 1);
                cmap((i >> 1) & 1);
                cmap(i & 1);
            }
            2 => {
                cmap(i >> 6);
                cmap((i >> 4) & 3);
                cmap((i >> 2) & 3);
                cmap(i & 3);
            }
            4 => {
                cmap(i >> 4);
                cmap(i & 0xf);
            }
            8 => cmap(i),
            _ => {}
        }
    }
    img.palmap = Some(palmap);
    img.map_stride = nsamples as usize;
    1
}

/// Construct any mapping table used
/// by the associated put routine.
/// Translation of `buildMap()`.
fn build_map(img: &mut TIFFRGBAImage, tif: &Tiff<'_>) -> i32 {
    match img.photometric {
        PHOTOMETRIC_RGB | PHOTOMETRIC_YCBCR | PHOTOMETRIC_SEPARATED
            if img.bitspersample == 8 => {}
        PHOTOMETRIC_RGB
        | PHOTOMETRIC_YCBCR
        | PHOTOMETRIC_SEPARATED
        | PHOTOMETRIC_MINISBLACK
        | PHOTOMETRIC_MINISWHITE => {
            if setup_map(img, tif) == 0 {
                return 0;
            }
        }
        PHOTOMETRIC_PALETTE => {
            /*
             * Convert 16-bit colormap to 8-bit (unless it looks
             * like an old-style 8-bit colormap).
             */
            if checkcmap(img) == 16 {
                cvtcmap(img);
            } else {
                tiff_warning_ext_r!(&tif.tif_name, "Assuming 8-bit colormap");
            }
            /*
             * Use mapping table and colormap to construct
             * unpacking tables for samples < 8 bits.
             */
            if img.bitspersample <= 8 && makecmap(img, tif) == 0 {
                return 0;
            }
        }
        _ => {}
    }
    1
}

/// Select the appropriate conversion routine for packed data.
/// Translation of `PickContigCase()`.
fn pick_contig_case(img: &mut TIFFRGBAImage, tif: &mut Tiff<'_>) -> i32 {
    img.get = Some(if tif.is_tiled() {
        gt_tile_contig
    } else {
        gt_strip_contig
    });
    img.put = Put::None;
    let mut put: Option<TileContigRoutine> = None;
    let assoc = img.alpha == EXTRASAMPLE_ASSOCALPHA as i32;
    let unass = img.alpha == EXTRASAMPLE_UNASSALPHA as i32;
    match img.photometric {
        PHOTOMETRIC_RGB => match img.bitspersample {
            8 => {
                if assoc && img.samplesperpixel >= 4 {
                    put = Some(put_rgbaa_contig8bittile);
                } else if unass && img.samplesperpixel >= 4 {
                    if build_map_ua_to_aa(img) != 0 {
                        put = Some(put_rgbua_contig8bittile);
                    }
                } else if img.samplesperpixel >= 3 {
                    put = Some(put_rgb_contig8bittile);
                }
            }
            16 => {
                if assoc && img.samplesperpixel >= 4 {
                    if build_map_bitdepth16_to_8(img) != 0 {
                        put = Some(put_rgbaa_contig16bittile);
                    }
                } else if unass && img.samplesperpixel >= 4 {
                    if build_map_bitdepth16_to_8(img) != 0 && build_map_ua_to_aa(img) != 0 {
                        put = Some(put_rgbua_contig16bittile);
                    }
                } else if img.samplesperpixel >= 3 && build_map_bitdepth16_to_8(img) != 0 {
                    put = Some(put_rgb_contig16bittile);
                }
            }
            _ => {}
        },
        PHOTOMETRIC_SEPARATED => {
            if img.samplesperpixel >= 4 && build_map(img, tif) != 0 && img.bitspersample == 8 {
                if img.map.is_none() {
                    put = Some(put_rgb_contig8bit_cmyk_tile);
                } else {
                    put = Some(put_rgb_contig8bit_cmyk_map_tile);
                }
            }
        }
        PHOTOMETRIC_PALETTE => {
            if build_map(img, tif) != 0 {
                put = match img.bitspersample {
                    8 => Some(put8bitcmaptile),
                    4 => Some(put4bitcmaptile),
                    2 => Some(put2bitcmaptile),
                    1 => Some(put1bitcmaptile),
                    _ => None,
                };
            }
        }
        PHOTOMETRIC_MINISWHITE | PHOTOMETRIC_MINISBLACK => {
            if build_map(img, tif) != 0 {
                put = match img.bitspersample {
                    16 => Some(put16bitbwtile),
                    8 => {
                        if img.alpha != 0 && img.samplesperpixel == 2 {
                            Some(putagreytile)
                        } else {
                            Some(putgreytile)
                        }
                    }
                    4 => Some(put4bitbwtile),
                    2 => Some(put2bitbwtile),
                    1 => Some(put1bitbwtile),
                    _ => None,
                };
            }
        }
        PHOTOMETRIC_YCBCR => {
            if img.bitspersample == 8
                && img.samplesperpixel == 3
                && init_ycbcr_conversion(img, tif) != 0
            {
                /*
                 * The 6.0 spec says that subsampling must be
                 * one of 1, 2, or 4, and that vertical subsampling
                 * must always be <= horizontal subsampling; so
                 * there are only a few possibilities and we just
                 * enumerate the cases.
                 * Joris: added support for the [1,2] case, nonetheless, to
                 * accommodate some OJPEG files
                 */
                let (subsampling_hor, subsampling_ver) =
                    tiff_get_field_defaulted_u16_pair(tif, TIFFTAG_YCBCRSUBSAMPLING)
                        .unwrap_or((0, 0));
                /* Validate that the image dimensions are compatible with
                the subsampling block. All putcontig8bitYCbCrXYtile routines
                assume width >= X and height >= Y. */
                if img.width < subsampling_hor as u32 || img.height < subsampling_ver as u32 {
                    tiff_error_ext_r!(
                        &tif.tif_name,
                        "YCbCr subsampling ({},{}) incompatible with image size {}x{}",
                        subsampling_hor,
                        subsampling_ver,
                        img.width,
                        img.height
                    );
                    return 0;
                }
                put = match ((subsampling_hor as u32) << 4) | subsampling_ver as u32 {
                    0x44 => Some(putcontig8bit_ycbcr44tile),
                    0x42 => Some(putcontig8bit_ycbcr42tile),
                    0x41 => Some(putcontig8bit_ycbcr41tile),
                    0x22 => Some(putcontig8bit_ycbcr22tile),
                    0x21 => Some(putcontig8bit_ycbcr21tile),
                    0x12 => Some(putcontig8bit_ycbcr12tile),
                    0x11 => Some(putcontig8bit_ycbcr11tile),
                    _ => None,
                };
            }
        }
        PHOTOMETRIC_CIELAB => {
            if img.samplesperpixel == 3
                && build_map(img, tif) != 0
                && (img.bitspersample == 8 || img.bitspersample == 16)
            {
                put = init_cielab_conversion(img, tif);
            }
        }
        _ => {}
    }
    if let Some(put) = put {
        img.put = Put::Contig(put);
    }
    (img.get.is_some() && put.is_some()) as i32
}

/// Select the appropriate conversion routine for unpacked data.
///
/// NB: we assume that unpacked single channel data is directed
///  to the "packed routines.
/// Translation of `PickSeparateCase()`.
fn pick_separate_case(img: &mut TIFFRGBAImage, tif: &mut Tiff<'_>) -> i32 {
    img.get = Some(if tif.is_tiled() {
        gt_tile_separate
    } else {
        gt_strip_separate
    });
    img.put = Put::None;
    let mut put: Option<TileSeparateRoutine> = None;
    let assoc = img.alpha == EXTRASAMPLE_ASSOCALPHA as i32;
    let unass = img.alpha == EXTRASAMPLE_UNASSALPHA as i32;
    match img.photometric {
        /* greyscale images processed pretty much as RGB by gtTileSeparate
         */
        PHOTOMETRIC_MINISWHITE | PHOTOMETRIC_MINISBLACK | PHOTOMETRIC_RGB => {
            match img.bitspersample {
                8 => {
                    if assoc {
                        put = Some(put_rgbaa_separate8bittile);
                    } else if unass {
                        if build_map_ua_to_aa(img) != 0 {
                            put = Some(put_rgbua_separate8bittile);
                        }
                    } else {
                        put = Some(put_rgb_separate8bittile);
                    }
                }
                16 => {
                    if assoc {
                        if build_map_bitdepth16_to_8(img) != 0 {
                            put = Some(put_rgbaa_separate16bittile);
                        }
                    } else if unass {
                        if build_map_bitdepth16_to_8(img) != 0 && build_map_ua_to_aa(img) != 0 {
                            put = Some(put_rgbua_separate16bittile);
                        }
                    } else if build_map_bitdepth16_to_8(img) != 0 {
                        put = Some(put_rgb_separate16bittile);
                    }
                }
                _ => {}
            }
        }
        PHOTOMETRIC_SEPARATED => {
            if img.bitspersample == 8 && img.samplesperpixel == 4 {
                /* Not alpha, but seems like the only way to get 4th band */
                img.alpha = 1;
                put = Some(put_cmyk_separate8bittile);
            }
        }
        PHOTOMETRIC_YCBCR => {
            if img.bitspersample == 8
                && img.samplesperpixel == 3
                && init_ycbcr_conversion(img, tif) != 0
            {
                let (hs, vs) = tiff_get_field_defaulted_u16_pair(tif, TIFFTAG_YCBCRSUBSAMPLING)
                    .unwrap_or((0, 0));
                if ((hs as u32) << 4) | vs as u32 == 0x11 {
                    put = Some(putseparate8bit_ycbcr11tile);
                }
                /* TODO: add other cases here */
            }
        }
        _ => {}
    }
    if let Some(put) = put {
        img.put = Put::Separate(put);
    }
    (img.get.is_some() && put.is_some()) as i32
}

/// Translation of `BuildMapUaToAa()`.
fn build_map_ua_to_aa(img: &mut TIFFRGBAImage) -> i32 {
    const MODULE: &str = "BuildMapUaToAa";
    let Some(mut m) = try_vec::<u8>(65536) else {
        tiff_error_ext_r!(MODULE, "Out of memory");
        return 0;
    };
    let mut i = 0;
    for na in 0..256u32 {
        for nv in 0..256u32 {
            m[i] = ((nv * na + 127) / 255) as u8;
            i += 1;
        }
    }
    img.ua_to_aa = Some(m);
    1
}

/// Translation of `BuildMapBitdepth16To8()`.
fn build_map_bitdepth16_to_8(img: &mut TIFFRGBAImage) -> i32 {
    const MODULE: &str = "BuildMapBitdepth16To8";
    let Some(mut m) = try_vec::<u8>(65536) else {
        tiff_error_ext_r!(MODULE, "Out of memory");
        return 0;
    };
    for (n, v) in m.iter_mut().enumerate() {
        *v = ((n as u32 + 128) / 257) as u8;
    }
    img.bitdepth16_to_8 = Some(m);
    1
}
