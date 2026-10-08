// Rust translation of libtiff/tif_strip.c from libtiff
// (https://gitlab.com/libtiff/libtiff, 4.7.2 as SDL_image's external/libtiff
// pins it).
// Copyright (c) 1991-1997 Sam Leffler
// Copyright (c) 1991-1997 Silicon Graphics, Inc.
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Strip-organized Image Support Routines.

use super::tif_aux::{
    _tiff_add64, _tiff_cast_uint64_to_ssize, _tiff_cast_uint64_to_uint32, _tiff_multiply32,
    _tiff_multiply64, tiff_get_field_defaulted_u16_pair,
};
use super::tif_error::tiff_error_ext_r;
use super::tif_tile::tiff_tile_row_size64;
use super::tiff::*;
use super::tiffiop::{tiff_howmany8_64, tiff_howmany_32, tiff_howmany_64, Tiff, TmSize};

/// Translation of `TIFFComputeStrip()`: Compute which strip a
/// (row,sample) value is in.
pub(crate) fn tiff_compute_strip(tif: &mut Tiff<'_>, row: u32, sample: u16) -> u32 {
    const MODULE: &str = "TIFFComputeStrip";
    let td = &tif.tif_dir;

    if td.td_rowsperstrip == 0 {
        tiff_error_ext_r!(MODULE, "Cannot compute strip: RowsPerStrip is zero");
        return 0;
    }
    let mut strip = row / td.td_rowsperstrip;
    if td.td_planarconfig == PLANARCONFIG_SEPARATE {
        if sample >= td.td_samplesperpixel {
            tiff_error_ext_r!(
                MODULE,
                "{}: Sample out of range, max {}",
                sample as u64,
                td.td_samplesperpixel as u64
            );
            return 0;
        }
        let sample_offset = _tiff_multiply64(
            sample as u64,
            td.td_stripsperimage as u64,
            "TIFFComputeStrip",
        );
        if sample_offset == 0 && sample != 0 && td.td_stripsperimage != 0 {
            return 0;
        }
        let strip64 = _tiff_add64(sample_offset, strip as u64, Some("TIFFComputeStrip"));
        if strip64 == 0 && (sample_offset != 0 || strip != 0) {
            return 0;
        }
        strip = _tiff_cast_uint64_to_uint32(strip64, "TIFFComputeStrip");
        if strip == 0 && strip64 != 0 {
            return 0;
        }
    }
    strip
}

/// Translation of `TIFFNumberOfStrips()`: Compute how many strips are in
/// an image.
pub(crate) fn tiff_number_of_strips(tif: &Tiff<'_>) -> u32 {
    let td = &tif.tif_dir;

    if td.td_rowsperstrip == 0 {
        super::tif_error::tiff_warning_ext_r!("TIFFNumberOfStrips", "RowsPerStrip is zero");
        return 0;
    }
    let mut nstrips = if td.td_rowsperstrip == u32::MAX {
        1
    } else {
        tiff_howmany_32(td.td_imagelength, td.td_rowsperstrip)
    };
    if td.td_planarconfig == PLANARCONFIG_SEPARATE {
        nstrips = _tiff_multiply32(nstrips, td.td_samplesperpixel as u32, "TIFFNumberOfStrips");
    }
    nstrips
}

/// Translation of `_TIFFStrileSize64()`: Compute the # bytes in a
/// variable height, row-aligned strip if isStrip is TRUE, or in a tile if
/// isStrip is FALSE
pub(crate) fn _tiff_strile_size64(tif: &mut Tiff<'_>, mut nrows: u32, is_strip: bool) -> u64 {
    const MODULE: &str = "_TIFFStrileSize64";
    if is_strip {
        if nrows == u32::MAX {
            nrows = tif.tif_dir.td_imagelength;
        }
    } else {
        let td = &tif.tif_dir;
        if td.td_tilelength == 0 || td.td_tilewidth == 0 || td.td_tiledepth == 0 {
            return 0;
        }
    }
    if (tif.tif_dir.td_planarconfig == PLANARCONFIG_CONTIG)
        && (tif.tif_dir.td_photometric == PHOTOMETRIC_YCBCR)
        && (!tif.is_up_sampled())
    {
        /*
         * Packed YCbCr data contain one Cb+Cr for every
         * HorizontalSampling*VerticalSampling Y values.
         * Must also roundup width and height when calculating
         * since images that are not a multiple of the
         * horizontal/vertical subsampling area include
         * YCbCr data for the extended image.
         */
        if tif.tif_dir.td_samplesperpixel != 3 {
            tiff_error_ext_r!(MODULE, "Invalid td_samplesperpixel value");
            return 0;
        }
        let (ss0, ss1) =
            tiff_get_field_defaulted_u16_pair(tif, TIFFTAG_YCBCRSUBSAMPLING).unwrap_or((0, 0));
        if (ss0 != 1 && ss0 != 2 && ss0 != 4)
            || (ss1 != 1 && ss1 != 2 && ss1 != 4)
            || (ss0 == 0 || ss1 == 0)
        {
            tiff_error_ext_r!(MODULE, "Invalid YCbCr subsampling ({}x{})", ss0, ss1);
            return 0;
        }
        let td = &tif.tif_dir;
        let samplingblock_samples = ss0 * ss1 + 2;
        let width = if is_strip {
            td.td_imagewidth
        } else {
            td.td_tilewidth
        };
        let samplingblocks_hor = tiff_howmany_32(width, ss0 as u32);
        let samplingblocks_ver = tiff_howmany_32(nrows, ss1 as u32);
        let samplingrow_samples = _tiff_multiply64(
            samplingblocks_hor as u64,
            samplingblock_samples as u64,
            MODULE,
        );
        let samplingrow_size = tiff_howmany8_64(_tiff_multiply64(
            samplingrow_samples,
            td.td_bitspersample as u64,
            MODULE,
        ));
        _tiff_multiply64(samplingrow_size, samplingblocks_ver as u64, MODULE)
    } else {
        let size = if is_strip {
            tiff_scanline_size64(tif)
        } else {
            tiff_tile_row_size64(tif)
        };
        _tiff_multiply64(nrows as u64, size, MODULE)
    }
}

/// Translation of `TIFFVStripSize64()`: Compute the # bytes in a variable
/// height, row-aligned strip.
pub(crate) fn tiff_vstrip_size64(tif: &mut Tiff<'_>, nrows: u32) -> u64 {
    _tiff_strile_size64(tif, nrows, /* isStrip = */ true)
}

/// Translation of `TIFFVStripSize()`.
pub(crate) fn tiff_vstrip_size(tif: &mut Tiff<'_>, nrows: u32) -> TmSize {
    const MODULE: &str = "TIFFVStripSize";
    let m = tiff_vstrip_size64(tif, nrows);
    _tiff_cast_uint64_to_ssize(m, Some(MODULE))
}

/// Translation of `TIFFStripSize64()`: Compute the # bytes in a
/// (row-aligned) strip.
///
/// Note that if RowsPerStrip is larger than the recorded ImageLength,
/// then the strip size is truncated to reflect the actual space required
/// to hold the strip.
pub(crate) fn tiff_strip_size64(tif: &mut Tiff<'_>) -> u64 {
    let mut rps = tif.tif_dir.td_rowsperstrip;
    if rps > tif.tif_dir.td_imagelength {
        rps = tif.tif_dir.td_imagelength;
    }
    tiff_vstrip_size64(tif, rps)
}

/// Translation of `TIFFStripSize()`.
pub(crate) fn tiff_strip_size(tif: &mut Tiff<'_>) -> TmSize {
    const MODULE: &str = "TIFFStripSize";
    let m = tiff_strip_size64(tif);
    _tiff_cast_uint64_to_ssize(m, Some(MODULE))
}

/// Translation of `TIFFScanlineSize64()`: Return the number of bytes to
/// read/write in a call to one of the scanline-oriented i/o routines.
/// Note that this number may be 1/samples-per-pixel if data is stored as
/// separate planes. The ScanlineSize in case of YCbCrSubsampling is
/// defined as the strip size divided by the strip height, i.e. the size of
/// a pack of vertical subsampling lines divided by vertical subsampling.
/// It should thus make sense when multiplied by a multiple of vertical
/// subsampling.
pub(crate) fn tiff_scanline_size64(tif: &mut Tiff<'_>) -> u64 {
    const MODULE: &str = "TIFFScanlineSize64";
    let scanline_size: u64;
    if tif.tif_dir.td_planarconfig == PLANARCONFIG_CONTIG {
        if (tif.tif_dir.td_photometric == PHOTOMETRIC_YCBCR)
            && (tif.tif_dir.td_samplesperpixel == 3)
            && (!tif.is_up_sampled())
        {
            let (ss0, ss1) =
                tiff_get_field_defaulted_u16_pair(tif, TIFFTAG_YCBCRSUBSAMPLING).unwrap_or((0, 0));
            if ((ss0 != 1) && (ss0 != 2) && (ss0 != 4))
                || ((ss1 != 1) && (ss1 != 2) && (ss1 != 4))
                || ((ss0 == 0) || (ss1 == 0))
            {
                tiff_error_ext_r!(MODULE, "Invalid YCbCr subsampling");
                return 0;
            }
            let td = &tif.tif_dir;
            let samplingblock_samples = ss0 * ss1 + 2;
            let samplingblocks_hor = tiff_howmany_32(td.td_imagewidth, ss0 as u32);
            let samplingrow_samples = _tiff_multiply64(
                samplingblocks_hor as u64,
                samplingblock_samples as u64,
                MODULE,
            );
            let samplingrow_size = tiff_howmany_64(
                _tiff_multiply64(samplingrow_samples, td.td_bitspersample as u64, MODULE),
                8,
            );
            scanline_size = samplingrow_size / ss1 as u64;
        } else {
            let td = &tif.tif_dir;
            let scanline_width = td.td_imagewidth;

            let scanline_samples =
                _tiff_multiply64(scanline_width as u64, td.td_samplesperpixel as u64, MODULE);
            scanline_size = tiff_howmany_64(
                _tiff_multiply64(scanline_samples, td.td_bitspersample as u64, MODULE),
                8,
            );
        }
    } else {
        let td = &tif.tif_dir;
        scanline_size = tiff_howmany_64(
            _tiff_multiply64(td.td_imagewidth as u64, td.td_bitspersample as u64, MODULE),
            8,
        );
    }
    if scanline_size == 0 {
        tiff_error_ext_r!(MODULE, "Computed scanline size is zero");
        return 0;
    }
    scanline_size
}

/// Translation of `TIFFScanlineSize()`.
pub(crate) fn tiff_scanline_size(tif: &mut Tiff<'_>) -> TmSize {
    const MODULE: &str = "TIFFScanlineSize";
    let m = tiff_scanline_size64(tif);
    _tiff_cast_uint64_to_ssize(m, Some(MODULE))
}
