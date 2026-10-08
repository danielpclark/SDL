// Rust translation of libtiff/tif_tile.c from libtiff
// (https://gitlab.com/libtiff/libtiff, 4.7.2 as SDL_image's external/libtiff
// pins it).
// Copyright (c) 1991-1997 Sam Leffler
// Copyright (c) 1991-1997 Silicon Graphics, Inc.
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Tiled Image Support Routines.

use super::tif_aux::{
    _tiff_add64, _tiff_cast_uint64_to_ssize, _tiff_cast_uint64_to_uint32, _tiff_multiply32,
    _tiff_multiply64,
};
use super::tif_error::tiff_error_ext_r;
use super::tif_strip::_tiff_strile_size64;
use super::tiff::*;
use super::tiffiop::{tiff_howmany8_64, tiff_howmany_32, Tiff, TmSize};

/// Translation of `TIFFComputeTile()`: Compute which tile an (x,y,z,s)
/// value is in.
pub(crate) fn tiff_compute_tile(tif: &Tiff<'_>, x: u32, y: u32, mut z: u32, s: u16) -> u32 {
    let td = &tif.tif_dir;
    let mut dx = td.td_tilewidth;
    let mut dy = td.td_tilelength;
    let mut dz = td.td_tiledepth;
    let mut tile: u32 = 1;

    if td.td_imagedepth == 1 {
        z = 0;
    }
    if dx == u32::MAX {
        dx = td.td_imagewidth;
    }
    if dy == u32::MAX {
        dy = td.td_imagelength;
    }
    if dz == u32::MAX {
        dz = td.td_imagedepth;
    }
    if dx != 0 && dy != 0 && dz != 0 {
        let xpt = tiff_howmany_32(td.td_imagewidth, dx);
        let ypt = tiff_howmany_32(td.td_imagelength, dy);
        let zpt = tiff_howmany_32(td.td_imagedepth, dz);
        let xpt_ypt = _tiff_multiply32(xpt, ypt, "TIFFComputeTile");
        let xpt_ypt_zpt = _tiff_multiply32(xpt_ypt, zpt, "TIFFComputeTile");

        if (xpt_ypt == 0 && xpt != 0 && ypt != 0) || (xpt_ypt_zpt == 0 && xpt_ypt != 0 && zpt != 0)
        {
            return 0;
        }

        let z_offset = _tiff_multiply64(xpt_ypt as u64, (z / dz) as u64, "TIFFComputeTile");
        let y_offset = _tiff_multiply64(xpt as u64, (y / dy) as u64, "TIFFComputeTile");
        if (z_offset == 0 && xpt_ypt != 0 && (z / dz) != 0)
            || (y_offset == 0 && xpt != 0 && (y / dy) != 0)
        {
            return 0;
        }
        let mut tile64 = _tiff_add64(z_offset, y_offset, Some("TIFFComputeTile"));
        if tile64 == 0 && (z_offset != 0 || y_offset != 0) {
            return 0;
        }
        tile64 = _tiff_add64(tile64, (x / dx) as u64, Some("TIFFComputeTile"));
        if tile64 == 0 && (z_offset != 0 || y_offset != 0 || (x / dx) != 0) {
            return 0;
        }
        if td.td_planarconfig == PLANARCONFIG_SEPARATE {
            if s >= td.td_samplesperpixel {
                tiff_error_ext_r!(
                    "TIFFComputeTile",
                    "{}: Sample out of range, max {}",
                    s as u64,
                    td.td_samplesperpixel as u64
                );
                return 0;
            }
            let sample_offset = _tiff_multiply64(xpt_ypt_zpt as u64, s as u64, "TIFFComputeTile");
            if sample_offset == 0 && xpt_ypt_zpt != 0 && s != 0 {
                return 0;
            }
            tile64 = _tiff_add64(sample_offset, tile64, Some("TIFFComputeTile"));
            if tile64 == 0
                && (sample_offset != 0 || z_offset != 0 || y_offset != 0 || (x / dx) != 0)
            {
                return 0;
            }
        }
        tile = _tiff_cast_uint64_to_uint32(tile64, "TIFFComputeTile");
        if tile == 0 && tile64 != 0 {
            return 0;
        }
    }
    tile
}

/// Translation of `TIFFCheckTile()`: Check an (x,y,z,s) coordinate
/// against the image bounds.
pub(crate) fn tiff_check_tile(tif: &Tiff<'_>, x: u32, y: u32, z: u32, s: u16) -> i32 {
    let td = &tif.tif_dir;

    if x >= td.td_imagewidth {
        tiff_error_ext_r!(
            tif.tif_name,
            "{}: Col out of range, max {}",
            x as u64,
            td.td_imagewidth.wrapping_sub(1) as u64
        );
        return 0;
    }
    if y >= td.td_imagelength {
        tiff_error_ext_r!(
            tif.tif_name,
            "{}: Row out of range, max {}",
            y as u64,
            td.td_imagelength.wrapping_sub(1) as u64
        );
        return 0;
    }
    if z >= td.td_imagedepth {
        tiff_error_ext_r!(
            tif.tif_name,
            "{}: Depth out of range, max {}",
            z as u64,
            td.td_imagedepth.wrapping_sub(1) as u64
        );
        return 0;
    }
    if td.td_planarconfig == PLANARCONFIG_SEPARATE && s >= td.td_samplesperpixel {
        tiff_error_ext_r!(
            tif.tif_name,
            "{}: Sample out of range, max {}",
            s as u64,
            td.td_samplesperpixel.wrapping_sub(1) as u64
        );
        return 0;
    }
    1
}

/// Translation of `TIFFNumberOfTiles()`: Compute how many tiles are in an
/// image.
pub(crate) fn tiff_number_of_tiles(tif: &Tiff<'_>) -> u32 {
    let td = &tif.tif_dir;
    let mut dx = td.td_tilewidth;
    let mut dy = td.td_tilelength;
    let mut dz = td.td_tiledepth;

    if dx == u32::MAX {
        dx = td.td_imagewidth;
    }
    if dy == u32::MAX {
        dy = td.td_imagelength;
    }
    if dz == u32::MAX {
        dz = td.td_imagedepth;
    }
    let mut ntiles = if dx == 0 || dy == 0 || dz == 0 {
        0
    } else {
        _tiff_multiply32(
            _tiff_multiply32(
                tiff_howmany_32(td.td_imagewidth, dx),
                tiff_howmany_32(td.td_imagelength, dy),
                "TIFFNumberOfTiles",
            ),
            tiff_howmany_32(td.td_imagedepth, dz),
            "TIFFNumberOfTiles",
        )
    };
    if td.td_planarconfig == PLANARCONFIG_SEPARATE {
        ntiles = _tiff_multiply32(ntiles, td.td_samplesperpixel as u32, "TIFFNumberOfTiles");
    }
    ntiles
}

/// Translation of `TIFFTileRowSize64()`: Compute the # bytes in each row
/// of a tile.
pub(crate) fn tiff_tile_row_size64(tif: &Tiff<'_>) -> u64 {
    const MODULE: &str = "TIFFTileRowSize64";
    let td = &tif.tif_dir;

    if td.td_tilelength == 0 {
        tiff_error_ext_r!(MODULE, "Tile length is zero");
        return 0;
    }
    if td.td_tilewidth == 0 {
        tiff_error_ext_r!(MODULE, "Tile width is zero");
        return 0;
    }
    let mut rowsize = _tiff_multiply64(
        td.td_bitspersample as u64,
        td.td_tilewidth as u64,
        "TIFFTileRowSize",
    );
    if td.td_planarconfig == PLANARCONFIG_CONTIG {
        if td.td_samplesperpixel == 0 {
            tiff_error_ext_r!(MODULE, "Samples per pixel is zero");
            return 0;
        }
        rowsize = _tiff_multiply64(rowsize, td.td_samplesperpixel as u64, "TIFFTileRowSize");
    }
    let tilerowsize = tiff_howmany8_64(rowsize);
    if tilerowsize == 0 {
        tiff_error_ext_r!(MODULE, "Computed tile row size is zero");
        return 0;
    }
    tilerowsize
}

/// Translation of `TIFFTileRowSize()`.
pub(crate) fn tiff_tile_row_size(tif: &Tiff<'_>) -> TmSize {
    const MODULE: &str = "TIFFTileRowSize";
    let m = tiff_tile_row_size64(tif);
    _tiff_cast_uint64_to_ssize(m, Some(MODULE))
}

/// Translation of `TIFFVTileSize64()`: Compute the # bytes in a variable
/// length, row-aligned tile.
pub(crate) fn tiff_vtile_size64(tif: &mut Tiff<'_>, nrows: u32) -> u64 {
    _tiff_strile_size64(tif, nrows, /* isStrip = */ false)
}

/// Translation of `TIFFTileSize64()`: Compute the # bytes in a
/// row-aligned tile.
pub(crate) fn tiff_tile_size64(tif: &mut Tiff<'_>) -> u64 {
    let nrows = tif.tif_dir.td_tilelength;
    tiff_vtile_size64(tif, nrows)
}

/// Translation of `TIFFTileSize()`.
pub(crate) fn tiff_tile_size(tif: &mut Tiff<'_>) -> TmSize {
    const MODULE: &str = "TIFFTileSize";
    let m = tiff_tile_size64(tif);
    _tiff_cast_uint64_to_ssize(m, Some(MODULE))
}
