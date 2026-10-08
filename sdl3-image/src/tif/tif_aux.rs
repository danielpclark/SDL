// Rust translation of libtiff/tif_aux.c from libtiff
// (https://gitlab.com/libtiff/libtiff, 4.7.2 as SDL_image's external/libtiff
// pins it).
// Copyright (c) 1991-1997 Sam Leffler
// Copyright (c) 1991-1997 Silicon Graphics, Inc.
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Auxiliary Support Routines.
//!
//! The `where`/`module` arguments that the C passes as `NULL` to silence
//! the overflow messages are `None` here.

use super::tif_dir::{tiff_vget_field, Gv, TIFFDirectory};
use super::tif_error::tiff_error_ext_r;
use super::tiff::*;
use super::tiffio::{D50_X0, D50_Y0, D50_Z0};
use super::tiffiop::{
    tiff_howmany8_64, try_vec, TifData, Tiff, TmSize, SEEK_SET, TIFF_TMSIZE_T_MAX,
};

/// Translation of `_TIFFMultiply32()`.
pub(crate) fn _tiff_multiply32(first: u32, second: u32, where_: &str) -> u32 {
    if second != 0 && first > u32::MAX / second {
        tiff_error_ext_r!(where_, "Integer overflow in {}", where_);
        return 0;
    }

    first.wrapping_mul(second)
}

/// Translation of `_TIFFMultiply64()`.
pub(crate) fn _tiff_multiply64(first: u64, second: u64, where_: &str) -> u64 {
    if second != 0 && first > u64::MAX / second {
        tiff_error_ext_r!(where_, "Integer overflow in {}", where_);
        return 0;
    }

    first.wrapping_mul(second)
}

/// Translation of `_TIFFAdd64()`.
pub(crate) fn _tiff_add64(first: u64, second: u64, where_: Option<&str>) -> u64 {
    if first > u64::MAX - second {
        if let Some(where_) = where_ {
            tiff_error_ext_r!(where_, "Integer overflow in {}", where_);
        }
        return 0;
    }

    first + second
}

/// Translation of `_TIFFMultiplySSize()` (the handle is `Some` when the C
/// passes one, which with `where` makes the failures reported).
pub(crate) fn _tiff_multiply_ssize(
    tif: Option<&Tiff<'_>>,
    first: TmSize,
    second: TmSize,
    where_: Option<&str>,
) -> TmSize {
    if first <= 0 || second <= 0 {
        if let (Some(_), Some(where_)) = (tif, where_) {
            tiff_error_ext_r!(
                where_,
                "Invalid argument to _TIFFMultiplySSize() in {}",
                where_
            );
        }
        return 0;
    }

    if first > TIFF_TMSIZE_T_MAX / second {
        if let (Some(_), Some(where_)) = (tif, where_) {
            tiff_error_ext_r!(where_, "Integer overflow in {}", where_);
        }
        return 0;
    }
    first * second
}

/// Translation of `_TIFFCastUInt64ToSSize()`.
pub(crate) fn _tiff_cast_uint64_to_ssize(val: u64, module: Option<&str>) -> TmSize {
    if val > TIFF_TMSIZE_T_MAX as u64 {
        if let Some(module) = module {
            tiff_error_ext_r!(module, "Integer overflow");
        }
        return 0;
    }
    val as TmSize
}

/// Translation of `_TIFFCastUInt64ToUInt32()`.
pub(crate) fn _tiff_cast_uint64_to_uint32(val: u64, module: &str) -> u32 {
    if val > u32::MAX as u64 {
        tiff_error_ext_r!(module, "Integer overflow");
        return 0;
    }
    val as u32
}

/// Translation of `_TIFFComputeRowSize64()`: Returns 0 on overflow or
/// invalid zero-sized row inputs. Callers that intentionally allow empty
/// rows should not use this helper directly.
pub(crate) fn _tiff_compute_row_size64(width: u32, spp: u16, bps: u16, where_: &str) -> u64 {
    let samples = _tiff_multiply64(width as u64, spp as u64, where_);
    if samples == 0 {
        return 0;
    }

    let bits = _tiff_multiply64(samples, bps as u64, where_);
    if bits == 0 {
        return 0;
    }

    tiff_howmany8_64(bits)
}

/// Translation of `_TIFFCheckRealloc()`/`_TIFFCheckMalloc()` for a fresh
/// buffer: `nmemb` zeroed elements of `elem_size` bytes (as a vector of
/// `T`s, the element size's), or `None` (reported) when the size
/// overflows, is zero, or can't be allocated.
pub(crate) fn _tiff_check_malloc_vec<T: Clone + Default>(
    tif: &Tiff<'_>,
    nmemb: TmSize,
    elem_size: TmSize,
    what: &str,
) -> Option<Vec<T>> {
    let count = _tiff_multiply_ssize(None, nmemb, elem_size, None);
    /*
     * Check for integer overflow.
     */
    let cp = if count != 0 {
        let unit = std::mem::size_of::<T>().max(1) as TmSize;
        try_vec::<T>((count / unit) as usize)
    } else {
        None
    };

    if cp.is_none() {
        tiff_error_ext_r!(
            tif.tif_name,
            "Failed to allocate memory for {} ({} elements of {} bytes each)",
            what,
            nmemb,
            elem_size
        );
    }

    cp
}

/// Translation of `TIFFDefaultTransferFunction()`.
fn tiff_default_transfer_function(td: &mut TIFFDirectory) -> i32 {
    td.td_transferfunction = [None, None, None];
    // Do not try to generate a default TransferFunction beyond 24 bits.
    // This otherwise leads to insane amounts, resulting in denial of service
    // See https://github.com/OSGeo/gdal/issues/10875
    if td.td_bitspersample > 24 {
        return 0;
    }

    let n = 1usize << td.td_bitspersample;
    let Some(mut tf0) = try_vec::<u16>(n) else {
        return 0;
    };
    tf0[0] = 0;
    for (i, e) in tf0.iter_mut().enumerate().skip(1) {
        let t = i as f64 / (n as f64 - 1.0);
        *e = (65535.0 * t.powf(2.2) + 0.5).floor() as u16;
    }

    if td.td_samplesperpixel as i32 - td.td_extrasamples as i32 > 1 {
        let Some(tf1) = super::tiffiop::try_copy(&tf0) else {
            return 0;
        };
        let Some(tf2) = super::tiffiop::try_copy(&tf0) else {
            return 0;
        };
        td.td_transferfunction = [Some(tf0), Some(tf1), Some(tf2)];
    } else {
        td.td_transferfunction[0] = Some(tf0);
    }
    1
}

/// Translation of `TIFFDefaultRefBlackWhite()`.
fn tiff_default_ref_black_white(td: &mut TIFFDirectory) -> i32 {
    let Some(mut rbw) = try_vec::<f32>(6) else {
        return 0;
    };
    if td.td_photometric == PHOTOMETRIC_YCBCR {
        /*
         * YCbCr (Class Y) images must have the ReferenceBlackWhite
         * tag set. Fix the broken images, which lacks that tag.
         */
        rbw[0] = 0.0;
        rbw[1] = 255.0;
        rbw[3] = 255.0;
        rbw[5] = 255.0;
        rbw[2] = 128.0;
        rbw[4] = 128.0;
    } else {
        /*
         * Assume RGB (Class R)
         */
        for i in 0..3 {
            rbw[2 * i] = 0.0;
            if td.td_bitspersample < 64 {
                rbw[2 * i + 1] = ((1u64 << td.td_bitspersample) - 1) as f32;
            } else {
                rbw[2 * i + 1] = u64::MAX as f32;
            }
        }
    }
    td.td_refblackwhite = Some(rbw);
    1
}

/// Translation of `TIFFVGetFieldDefaulted()`: Like TIFFGetField, but
/// return any default value if the tag is not present in the directory.
///
/// NB: We use the value in the directory, rather than
///     explicit values so that defaults exist only one
///     place in the library -- in TIFFDefaultDirectory.
pub(crate) fn tiff_vget_field_defaulted(tif: &mut Tiff<'_>, tag: u32, ap: &mut Vec<Gv>) -> i32 {
    if tiff_vget_field(tif, tag, ap) != 0 {
        return 1;
    }
    let td = &mut tif.tif_dir;
    match tag {
        TIFFTAG_SUBFILETYPE => {
            ap.push(Gv::U32(td.td_subfiletype));
            1
        }
        TIFFTAG_BITSPERSAMPLE => {
            ap.push(Gv::U16(td.td_bitspersample));
            1
        }
        TIFFTAG_THRESHHOLDING => {
            ap.push(Gv::U16(td.td_threshholding));
            1
        }
        TIFFTAG_FILLORDER => {
            ap.push(Gv::U16(td.td_fillorder));
            1
        }
        TIFFTAG_ORIENTATION => {
            ap.push(Gv::U16(td.td_orientation));
            1
        }
        TIFFTAG_SAMPLESPERPIXEL => {
            ap.push(Gv::U16(td.td_samplesperpixel));
            1
        }
        TIFFTAG_ROWSPERSTRIP => {
            ap.push(Gv::U32(td.td_rowsperstrip));
            1
        }
        TIFFTAG_MINSAMPLEVALUE => {
            ap.push(Gv::U16(td.td_minsamplevalue));
            1
        }
        TIFFTAG_MAXSAMPLEVALUE => {
            /* td_bitspersample=1 is always set in TIFFDefaultDirectory().
             * Therefore, td_maxsamplevalue has to be re-calculated in
             * TIFFGetFieldDefaulted(). */
            let maxsamplevalue: u16 = if td.td_bitspersample > 0 {
                /* This shift operation into a uint16_t limits the value to
                 * 65535 even if td_bitspersamle is > 16 */
                if td.td_bitspersample <= 16 {
                    ((1u32 << td.td_bitspersample) - 1) as u16 /* 2**(BitsPerSample) - 1 */
                } else {
                    65535
                }
            } else {
                0
            };
            ap.push(Gv::U16(maxsamplevalue));
            1
        }
        TIFFTAG_PLANARCONFIG => {
            ap.push(Gv::U16(td.td_planarconfig));
            1
        }
        TIFFTAG_RESOLUTIONUNIT => {
            ap.push(Gv::U16(td.td_resolutionunit));
            1
        }
        TIFFTAG_PREDICTOR => {
            let predictor = match &tif.tif_data {
                TifData::Lzw(sp) => Some(sp.predict.predictor),
                _ => None,
            };
            match predictor {
                None => {
                    tiff_error_ext_r!(
                        tif.tif_name,
                        "Cannot get \"Predictor\" tag as plugin is not configured"
                    );
                    ap.push(Gv::U16(0));
                    0
                }
                Some(p) => {
                    ap.push(Gv::U16(p as u16));
                    1
                }
            }
        }
        TIFFTAG_DOTRANGE => {
            ap.push(Gv::U16(0));
            if td.td_bitspersample <= 16 {
                ap.push(Gv::U16(((1u32 << td.td_bitspersample) - 1) as u16));
            } else {
                ap.push(Gv::U16(65535));
            }
            1
        }
        TIFFTAG_INKSET => {
            ap.push(Gv::U16(INKSET_CMYK));
            1
        }
        TIFFTAG_NUMBEROFINKS => {
            ap.push(Gv::U16(4));
            1
        }
        TIFFTAG_EXTRASAMPLES => {
            ap.push(Gv::U16(td.td_extrasamples));
            ap.push(Gv::U16s(td.td_sampleinfo.clone()));
            1
        }
        TIFFTAG_MATTEING => {
            ap.push(Gv::U16(
                (td.td_extrasamples == 1
                    && td
                        .td_sampleinfo
                        .as_ref()
                        .is_some_and(|s| s.first() == Some(&EXTRASAMPLE_ASSOCALPHA)))
                    as u16,
            ));
            1
        }
        TIFFTAG_TILEDEPTH => {
            ap.push(Gv::U32(td.td_tiledepth));
            1
        }
        TIFFTAG_DATATYPE => {
            ap.push(Gv::U16(td.td_sampleformat.wrapping_sub(1)));
            1
        }
        TIFFTAG_SAMPLEFORMAT => {
            ap.push(Gv::U16(td.td_sampleformat));
            1
        }
        TIFFTAG_IMAGEDEPTH => {
            ap.push(Gv::U32(td.td_imagedepth));
            1
        }
        TIFFTAG_YCBCRCOEFFICIENTS => {
            /* defaults are from CCIR Recommendation 601-1 */
            ap.push(Gv::F32s(Some(vec![0.299f32, 0.587f32, 0.114f32])));
            1
        }
        TIFFTAG_YCBCRSUBSAMPLING => {
            ap.push(Gv::U16(td.td_ycbcrsubsampling[0]));
            ap.push(Gv::U16(td.td_ycbcrsubsampling[1]));
            1
        }
        TIFFTAG_YCBCRPOSITIONING => {
            ap.push(Gv::U16(td.td_ycbcrpositioning));
            1
        }
        TIFFTAG_WHITEPOINT => {
            /* TIFF 6.0 specification tells that it is no default
            value for the WhitePoint, but AdobePhotoshop TIFF
            Technical Note tells that it should be CIE D50. */
            ap.push(Gv::F32s(Some(vec![
                D50_X0 / (D50_X0 + D50_Y0 + D50_Z0),
                D50_Y0 / (D50_X0 + D50_Y0 + D50_Z0),
            ])));
            1
        }
        TIFFTAG_TRANSFERFUNCTION => {
            if td.td_transferfunction[0].is_none() && tiff_default_transfer_function(td) == 0 {
                tiff_error_ext_r!(tif.tif_name, "No space for \"TransferFunction\" tag");
                return 0;
            }
            ap.push(Gv::U16s(td.td_transferfunction[0].clone()));
            if td.td_samplesperpixel as i32 - td.td_extrasamples as i32 > 1 {
                ap.push(Gv::U16s(td.td_transferfunction[1].clone()));
                ap.push(Gv::U16s(td.td_transferfunction[2].clone()));
            }
            1
        }
        TIFFTAG_REFERENCEBLACKWHITE => {
            if td.td_refblackwhite.is_none() && tiff_default_ref_black_white(td) == 0 {
                return 0;
            }
            ap.push(Gv::F32s(td.td_refblackwhite.clone()));
            1
        }
        _ => 0,
    }
}

/// Translation of `TIFFGetFieldDefaulted()`: Like TIFFGetField, but
/// return any default value if the tag is not present in the directory.
pub(crate) fn tiff_get_field_defaulted(tif: &mut Tiff<'_>, tag: u32, out: &mut Vec<Gv>) -> i32 {
    tiff_vget_field_defaulted(tif, tag, out)
}

/// `TIFFGetFieldDefaulted(tif, tag, &v)` for a field of one 16-bit value.
pub(crate) fn tiff_get_field_defaulted_u16(tif: &mut Tiff<'_>, tag: u32) -> Option<u16> {
    let mut out = Vec::new();
    if tiff_get_field_defaulted(tif, tag, &mut out) == 0 {
        return None;
    }
    out.first().and_then(Gv::as_u64).map(|v| v as u16)
}

/// `TIFFGetFieldDefaulted(tif, tag, &a, &b)` for a field of two 16-bit
/// values.
pub(crate) fn tiff_get_field_defaulted_u16_pair(
    tif: &mut Tiff<'_>,
    tag: u32,
) -> Option<(u16, u16)> {
    let mut out = Vec::new();
    if tiff_get_field_defaulted(tif, tag, &mut out) == 0 {
        return None;
    }
    let a = out.first().and_then(Gv::as_u64)? as u16;
    let b = out.get(1).and_then(Gv::as_u64)? as u16;
    Some((a, b))
}

/// Translation of `_TIFFClampDoubleToFloat()`.
pub(crate) fn _tiff_clamp_double_to_float(val: f64) -> f32 {
    if val > f32::MAX as f64 {
        return f32::MAX;
    }
    if val < -(f32::MAX as f64) {
        return -f32::MAX;
    }
    val as f32
}

/// Translation of `_TIFFClampDoubleToUInt32()`.
pub(crate) fn _tiff_clamp_double_to_uint32(val: f64) -> u32 {
    if val < 0.0 {
        return 0;
    }
    if val > 0xFFFFFFFFu32 as f64 || val.is_nan() {
        return 0xFFFFFFFF;
    }
    val as u32
}

/// Translation of `_TIFFSeekOK()`.
pub(crate) fn _tiff_seek_ok(tif: &mut Tiff<'_>, off: u64) -> bool {
    /* Huge offsets, especially -1 / UINT64_MAX, can cause issues */
    /* See http://bugzilla.maptools.org/show_bug.cgi?id=2726 */
    off <= (!0u64) / 2 && tif.seek_file(off, SEEK_SET) == off
}
