// Rust translation of src/pfr/pfrtypes.h from FreeType (2.13.2, as
// SDL_ttf's external/freetype pins it).
// Copyright (C) 2002-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! FreeType PFR data structures (specification only).
//!
//! The C records' pointers into each other become owned vectors: the
//! physical font's kerning items are a vector in list order, and its stem
//! snap tables one vector (`stem_snaps`), C's `vertical.stem_snaps`
//! block, that the dimensions' `stem_snaps` index.

use super::super::base::ftgloadr::FtGlyphLoaderRec;
use super::super::fttypes::*;

/************************************************************************/

/// `PFR_HeaderRec`: the PFR Header structure
#[derive(Debug, Clone, Copy, Default)]
pub struct PfrHeaderRec {
    pub signature: FtUInt32,
    pub version: FtUInt,
    pub signature2: FtUInt,
    pub header_size: FtUInt,

    pub log_dir_size: FtUInt,
    pub log_dir_offset: FtUInt,

    pub log_font_max_size: FtUInt,
    pub log_font_section_size: FtUInt32,
    pub log_font_section_offset: FtUInt32,

    pub phy_font_max_size: FtUInt32,
    pub phy_font_section_size: FtUInt32,
    pub phy_font_section_offset: FtUInt32,

    pub gps_max_size: FtUInt,
    pub gps_section_size: FtUInt32,
    pub gps_section_offset: FtUInt32,

    pub max_blue_values: FtUInt,
    pub max_x_orus: FtUInt,
    pub max_y_orus: FtUInt,

    pub phy_font_max_size_high: FtUInt,
    pub color_flags: FtUInt,

    pub bct_max_size: FtUInt32,
    pub bct_set_max_size: FtUInt32,
    pub phy_bct_set_max_size: FtUInt32,

    pub num_phy_fonts: FtUInt,
    pub max_vert_stem_snap: FtUInt,
    pub max_horz_stem_snap: FtUInt,
    pub max_chars: FtUInt,
}

/* used in `color_flags' field of the PFR_Header */
pub const PFR_FLAG_BLACK_PIXEL: FtUInt = 0x01;
pub const PFR_FLAG_INVERT_BITMAP: FtUInt = 0x02;

/************************************************************************/

/// `PFR_LogFontRec`
#[derive(Debug, Clone, Copy, Default)]
pub struct PfrLogFontRec {
    pub size: FtUInt32,
    pub offset: FtUInt32,

    pub matrix: [FtInt32; 4],
    pub stroke_flags: FtUInt,
    pub stroke_thickness: FtInt,
    pub bold_thickness: FtInt,
    pub miter_limit: FtInt32,

    pub phys_size: FtUInt32,
    pub phys_offset: FtUInt32,
}

pub const PFR_LINE_JOIN_MITER: FtUInt = 0x00;
pub const PFR_LINE_JOIN_ROUND: FtUInt = 0x01;
pub const PFR_LINE_JOIN_BEVEL: FtUInt = 0x02;
pub const PFR_LINE_JOIN_MASK: FtUInt = PFR_LINE_JOIN_ROUND | PFR_LINE_JOIN_BEVEL;

pub const PFR_LOG_STROKE: FtUInt = 0x04;
pub const PFR_LOG_2BYTE_STROKE: FtUInt = 0x08;
pub const PFR_LOG_BOLD: FtUInt = 0x10;
pub const PFR_LOG_2BYTE_BOLD: FtUInt = 0x20;
pub const PFR_LOG_EXTRA_ITEMS: FtUInt = 0x40;

/************************************************************************/

pub const PFR_BITMAP_2BYTE_CHARCODE: FtUInt = 0x01;
pub const PFR_BITMAP_2BYTE_SIZE: FtUInt = 0x02;
pub const PFR_BITMAP_3BYTE_OFFSET: FtUInt = 0x04;

/*not part of the specification but used for implementation */
pub const PFR_BITMAP_CHARCODES_VALIDATED: FtUInt = 0x40;
pub const PFR_BITMAP_VALID_CHARCODES: FtUInt = 0x80;

/// `PFR_BitmapCharRec`
#[derive(Debug, Clone, Copy, Default)]
pub struct PfrBitmapCharRec {
    pub char_code: FtUInt,
    pub gps_size: FtUInt,
    pub gps_offset: FtUInt32,
}

pub const PFR_STRIKE_2BYTE_XPPM: FtUInt = 0x01;
pub const PFR_STRIKE_2BYTE_YPPM: FtUInt = 0x02;
pub const PFR_STRIKE_3BYTE_SIZE: FtUInt = 0x04;
pub const PFR_STRIKE_3BYTE_OFFSET: FtUInt = 0x08;
pub const PFR_STRIKE_2BYTE_COUNT: FtUInt = 0x10;

/// `PFR_StrikeRec`
#[derive(Debug, Clone, Default)]
pub struct PfrStrikeRec {
    pub x_ppm: FtUInt,
    pub y_ppm: FtUInt,
    pub flags: FtUInt,

    pub gps_size: FtUInt32,
    pub gps_offset: FtUInt32,

    pub bct_size: FtUInt32,
    pub bct_offset: FtUInt32,

    /* optional */
    pub num_bitmaps: FtUInt,
    pub bitmaps: Vec<PfrBitmapCharRec>,
}

/************************************************************************/

/// `PFR_CharRec`
#[derive(Debug, Clone, Copy, Default)]
pub struct PfrCharRec {
    pub char_code: FtUInt,
    pub advance: FtInt,
    pub gps_size: FtUInt,
    pub gps_offset: FtUInt32,
}

/************************************************************************/

/// `PFR_DimensionRec`
#[derive(Debug, Clone, Copy, Default)]
pub struct PfrDimensionRec {
    pub standard: FtUInt,
    pub num_stem_snaps: FtUInt,
    /// `stem_snaps`: the dimension's first value in the physical font's
    /// `stem_snaps`
    pub stem_snaps: Option<usize>,
}

/************************************************************************/

/// `PFR_KernItemRec` (`next` is the next item of the physical font's
/// `kern_items`)
#[derive(Debug, Clone, Copy, Default)]
pub struct PfrKernItemRec {
    pub pair_count: FtByte,
    pub flags: FtByte,
    pub base_adj: FtShort,
    pub pair_size: FtUInt,
    pub offset: usize, /* FT_Offset */
    pub pair1: FtUInt32,
    pub pair2: FtUInt32,
}

/// `PFR_KERN_INDEX`
#[inline]
pub fn pfr_kern_index(g1: FtUInt, g2: FtUInt) -> FtUInt32 {
    (g1 << 16) | (g2 as FtUInt16 as FtUInt32)
}

/// `PFR_NEXT_KPAIR`
#[inline]
pub fn pfr_next_kpair(base: &[u8], p: &mut usize) -> FtUInt32 {
    *p += 2;
    ((byte_at(base, *p - 2) as FtUInt32) << 16) | byte_at(base, *p - 1) as FtUInt32
}

fn byte_at(p: &[u8], i: usize) -> u8 {
    p.get(i).copied().unwrap_or(0)
}

/************************************************************************/

/// `PFR_PhyFontRec`
#[derive(Debug, Clone, Default)]
pub struct PfrPhyFontRec {
    pub offset: FtUInt32,

    pub font_ref_number: FtUInt,
    pub outline_resolution: FtUInt,
    pub metrics_resolution: FtUInt,
    pub bbox: FtBBox,
    pub flags: FtUInt,
    pub standard_advance: FtInt,

    pub ascent: FtInt,  /* optional, bbox.yMax if not present */
    pub descent: FtInt, /* optional, bbox.yMin if not present */
    pub leading: FtInt, /* optional, 0 if not present         */

    pub horizontal: PfrDimensionRec,
    pub vertical: PfrDimensionRec,
    /// the stem snap tables (C's `vertical.stem_snaps` block)
    pub stem_snaps: Option<Vec<FtInt>>,

    pub font_id: Option<Vec<u8>>,
    pub family_name: Option<Vec<u8>>,
    pub style_name: Option<Vec<u8>>,

    pub num_strikes: FtUInt,
    pub max_strikes: FtUInt,
    pub strikes: Vec<PfrStrikeRec>,

    pub num_blue_values: FtUInt,
    pub blue_values: Vec<FtInt>,
    pub blue_fuzz: FtUInt,
    pub blue_scale: FtUInt,

    pub num_chars: FtUInt,
    pub chars_offset: usize, /* FT_Offset */
    pub chars: Vec<PfrCharRec>,

    pub num_kern_pairs: FtUInt,
    pub kern_items: Vec<PfrKernItemRec>,

    /* not part of the spec, but used during load */
    pub bct_offset: FtULong,
}

pub const PFR_PHY_VERTICAL: FtUInt = 0x01;
pub const PFR_PHY_2BYTE_CHARCODE: FtUInt = 0x02;
pub const PFR_PHY_PROPORTIONAL: FtUInt = 0x04;
pub const PFR_PHY_ASCII_CODE: FtUInt = 0x08;
pub const PFR_PHY_2BYTE_GPS_SIZE: FtUInt = 0x10;
pub const PFR_PHY_3BYTE_GPS_OFFSET: FtUInt = 0x20;
pub const PFR_PHY_EXTRA_ITEMS: FtUInt = 0x80;

pub const PFR_KERN_2BYTE_CHAR: FtByte = 0x01;
pub const PFR_KERN_2BYTE_ADJ: FtByte = 0x02;

/************************************************************************/

pub const PFR_GLYPH_YCOUNT: FtUInt = 0x01;
pub const PFR_GLYPH_XCOUNT: FtUInt = 0x02;
pub const PFR_GLYPH_1BYTE_XYCOUNT: FtUInt = 0x04;

pub const PFR_GLYPH_SINGLE_EXTRA_ITEMS: FtUInt = 0x08;
pub const PFR_GLYPH_COMPOUND_EXTRA_ITEMS: FtUInt = 0x40;
pub const PFR_GLYPH_IS_COMPOUND: FtUInt = 0x80;

/// `PFR_CoordRec`: controlled coordinate
#[derive(Debug, Clone, Copy, Default)]
pub struct PfrCoordRec {
    pub org: FtUInt,
    pub cur: FtUInt,
}

/// `PFR_SubGlyphRec`
#[derive(Debug, Clone, Copy, Default)]
pub struct PfrSubGlyphRec {
    pub x_scale: FtFixed,
    pub y_scale: FtFixed,
    pub x_delta: FtInt,
    pub y_delta: FtInt,
    pub gps_offset: FtUInt32,
    pub gps_size: FtUInt,
}

pub const PFR_SUBGLYPH_XSCALE: FtUInt = 0x10;
pub const PFR_SUBGLYPH_YSCALE: FtUInt = 0x20;
pub const PFR_SUBGLYPH_2BYTE_SIZE: FtUInt = 0x40;
pub const PFR_SUBGLYPH_3BYTE_OFFSET: FtUInt = 0x80;

/// `PFR_GlyphRec` (its `loader` is the glyph slot's, which it holds while
/// it loads a glyph)
#[derive(Debug, Clone, Default)]
pub struct PfrGlyphRec {
    pub format: FtByte,

    /* (`#if 0'ed num_x_control and num_y_control) */
    pub max_xy_control: FtUInt,
    /// `x_control` (`max_xy_control` values, the y ones after the x ones)
    pub x_control: Vec<FtPos>,
    /// `y_control`: the first y control value in `x_control`
    pub y_control: usize,

    pub num_subs: FtUInt,
    pub max_subs: FtUInt,
    pub subs: Vec<PfrSubGlyphRec>,

    pub loader: FtGlyphLoaderRec,
    pub path_begun: bool,
}
