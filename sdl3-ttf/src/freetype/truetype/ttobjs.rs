// Rust translation of src/truetype/ttobjs.c (and ttobjs.h) from FreeType
// (2.13.2, as SDL_ttf's external/freetype pins it).
// Copyright (C) 1996-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! Objects manager (body).
//!
//! The TrueType size object lives in [`TtFaceRec::size`] (a face has
//! exactly one size here); its `root` is the face's `FtSizeRec`.  The
//! driver object's only field, the interpreter version, is kept in the
//! `truetype` module's properties ([`TtDriverRec`]).

use std::sync::Arc;

use super::super::base::ftcalc::*;
use super::super::base::ftobjs::*;
use super::super::base::ftstream::FtStreamRec;
use super::super::fttypes::*;
use super::super::sfnt::sfobjs::{sfnt_done_face, sfnt_init_face, sfnt_load_face};
use super::super::sfnt::ttload::{tt_face_goto_table, with_stream};
use super::super::sfnt::ttpost::tt_face_get_ps_name;
use super::super::tttables::*;
use super::super::tttypes::*;
use super::ttgxvar::{tt_done_blend, tt_set_named_instance_ftmm};
use super::ttinterp::*;
use super::ttpload::*;

/* ftdriver.h */
pub const TT_INTERPRETER_VERSION_35: FtUInt = 35;
pub const TT_INTERPRETER_VERSION_38: FtUInt = 38;
pub const TT_INTERPRETER_VERSION_40: FtUInt = 40;

/// `TT_DriverRec` (the part of the driver object beyond `FT_DriverRec`;
/// `zone`, the glyph loader points zone, is unused)
#[derive(Debug, Clone, Copy)]
pub struct TtDriverRec {
    pub interpreter_version: FtUInt,
}

/// The interpreter version of the face's driver
/// (`((TT_Driver)FT_FACE_DRIVER( face ))->interpreter_version`).
pub fn tt_driver_interpreter_version(face: &TtFaceRec) -> FtUInt {
    face.root
        .driver_module()
        .with_props(|d: &mut TtDriverRec| d.interpreter_version)
        .unwrap_or(TT_INTERPRETER_VERSION_40)
}

/// `TT_GraphicsState`: The TrueType graphics state used during bytecode
/// interpretation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TtGraphicsState {
    pub rp0: FtUShort,
    pub rp1: FtUShort,
    pub rp2: FtUShort,

    pub dual_vector: FtUnitVector,
    pub proj_vector: FtUnitVector,
    pub free_vector: FtUnitVector,

    /// `loop`
    pub loop_: FtLong,
    pub minimum_distance: FtF26Dot6,
    pub round_state: FtInt,

    pub auto_flip: bool,
    pub control_value_cutin: FtF26Dot6,
    pub single_width_cutin: FtF26Dot6,
    pub single_width_value: FtF26Dot6,
    pub delta_base: FtUShort,
    pub delta_shift: FtUShort,

    pub instruct_control: FtByte,
    /* According to Greg Hitchcock from Microsoft, the `scan_control'     */
    /* variable as documented in the TrueType specification is a 32-bit   */
    /* integer; the high-word part holds the SCANTYPE value, the low-word */
    /* part the SCANCTRL value.  We separate it into two fields.          */
    pub scan_control: bool,
    pub scan_type: FtInt,

    pub gep0: FtUShort,
    pub gep1: FtUShort,
    pub gep2: FtUShort,
}

/*
 *
 * EXECUTION SUBTABLES
 *
 * These sub-tables relate to instruction execution.
 *
 */

pub const TT_MAX_CODE_RANGES: usize = 3;

/*
 * There can only be 3 active code ranges at once:
 *   - the Font Program
 *   - the CVT Program
 *   - a glyph's instructions set
 */
/// `TT_CodeRange_Tag`
pub const TT_CODERANGE_NONE: FtInt = 0;
pub const TT_CODERANGE_FONT: FtInt = 1;
pub const TT_CODERANGE_CVT: FtInt = 2;
pub const TT_CODERANGE_GLYPH: FtInt = 3;

/// `TT_CodeRange`
#[derive(Debug, Clone, Default)]
pub struct TtCodeRange {
    pub base: Option<Arc<[u8]>>,
    pub size: FtLong,
}

/// `TT_CodeRangeTable`
pub type TtCodeRangeTable = [TtCodeRange; TT_MAX_CODE_RANGES];

/// `TT_DefRecord`: Defines a function/instruction definition record.
#[derive(Debug, Clone, Copy, Default)]
pub struct TtDefRecord {
    pub range: FtInt,  /* in which code range is it located?     */
    pub start: FtLong, /* where does it start?                   */
    pub end: FtLong,   /* where does it end?                     */
    pub opc: FtUInt,   /* function #, or instruction code        */
    pub active: bool,  /* is it active?                          */
}

/// `TT_Transform`: Subglyph transformation record.
#[derive(Debug, Clone, Copy, Default)]
pub struct TtTransform {
    pub xx: FtFixed,
    pub xy: FtFixed, /* transformation matrix coefficients */
    pub yx: FtFixed,
    pub yy: FtFixed,
    pub ox: FtF26Dot6,
    pub oy: FtF26Dot6, /* offsets                            */
}

/*
 *
 * A note regarding non-squared pixels:
 *
 * (This text will probably go into some docs at some time; for now, it
 * is kept here to explain some definitions in the TT_Size_Metrics
 * record).
 *
 * The CVT is a one-dimensional array containing values that control
 * certain important characteristics in a font, like the height of all
 * capitals, all lowercase letter, default spacing or stem width/height.
 *
 * These values are found in FUnits in the font file, and must be scaled
 * to pixel coordinates before being used by the CVT and glyph programs.
 * Unfortunately, when using distinct x and y resolutions (or distinct x
 * and y pointsizes), there are two possible scalings.
 *
 * A first try was to implement a `lazy' scheme where all values were
 * scaled when first used.  However, while some values are always used
 * in the same direction, some others are used under many different
 * circumstances and orientations.
 *
 * I have found a simpler way to do the same, and it even seems to work
 * in most of the cases:
 *
 * - All CVT values are scaled to the maximum ppem size.
 *
 * - When performing a read or write in the CVT, a ratio factor is used
 *   to perform adequate scaling.  Example:
 *
 *     x_ppem = 14
 *     y_ppem = 10
 *
 *   We choose ppem = x_ppem = 14 as the CVT scaling size.  All cvt
 *   entries are scaled to it.
 *
 *     x_ratio = 1.0
 *     y_ratio = y_ppem/ppem (< 1.0)
 *
 *   We compute the current ratio like:
 *
 *   - If projVector is horizontal,
 *       ratio = x_ratio = 1.0
 *
 *   - if projVector is vertical,
 *       ratio = y_ratio
 *
 *   - else,
 *       ratio = sqrt( (proj.x * x_ratio) ^ 2 + (proj.y * y_ratio) ^ 2 )
 *
 *   Reading a cvt value returns
 *     ratio * cvt[index]
 *
 *   Writing a cvt value in pixels:
 *     cvt[index] / ratio
 *
 *   The current ppem is simply
 *     ratio * ppem
 *
 */

/// `TT_Size_Metrics`: Metrics used by the TrueType size and context
/// objects.
#[derive(Debug, Clone, Copy, Default)]
pub struct TtSizeMetrics {
    /* for non-square pixels */
    pub x_ratio: FtLong,
    pub y_ratio: FtLong,

    pub ppem: FtUShort, /* maximum ppem size              */
    pub ratio: FtLong,  /* current ratio                  */
    pub scale: FtFixed,

    pub compensations: [FtF26Dot6; 4], /* device-specific compensations  */

    pub valid: bool,

    pub rotated: bool,   /* `is the glyph rotated?'-flag   */
    pub stretched: bool, /* `is the glyph stretched?'-flag */
}

/// `TT_SizeRec`: TrueType size class (the `root` is the face's
/// `FtSizeRec`).
#[derive(Debug, Default)]
pub struct TtSizeRec {
    /* we have our own copy of metrics so that we can modify */
    /* it without affecting auto-hinting (when used)         */
    /// `metrics`: whether it points to `root.metrics` (`true`) or to
    /// `hinted_metrics` (`false`); for the current rendering mode
    pub metrics_root: bool,
    pub hinted_metrics: FtSizeMetrics, /* for the hinted rendering mode  */

    pub ttmetrics: TtSizeMetrics,

    /// `widthp`: glyph widths from the hdmx table (an offset into
    /// `hdmx_table`)
    pub widthp: Option<usize>,

    pub strike_index: FtULong, /* 0xFFFFFFFF to indicate invalid */

    /* TT_USE_BYTECODE_INTERPRETER */
    pub point_size: FtLong, /* for the `MPS' bytecode instruction */

    pub num_function_defs: FtUInt, /* number of function definitions */
    pub max_function_defs: FtUInt,
    pub function_defs: Vec<TtDefRecord>, /* table of function definitions  */

    pub num_instruction_defs: FtUInt, /* number of ins. definitions */
    pub max_instruction_defs: FtUInt,
    pub instruction_defs: Vec<TtDefRecord>, /* table of ins. definitions  */

    pub max_func: FtUInt,
    pub max_ins: FtUInt,

    pub code_range_table: TtCodeRangeTable,

    /// `GS`
    pub gs: TtGraphicsState,

    pub cvt_size: FtULong, /* the scaled control value table */
    pub cvt: Vec<FtLong>,

    pub storage_size: FtUShort, /* The storage area is now part of */
    pub storage: Vec<FtLong>,   /* the instance                    */

    pub twilight: TtGlyphZoneRec, /* The instance's twilight zone    */

    pub context: Option<Box<TtExecContextRec>>,

    /* if negative, `fpgm' (resp. `prep'), wasn't executed yet; */
    /* otherwise it is the returned error code                  */
    pub bytecode_ready: FtError,
    pub cvt_ready: FtError,
}

impl TtFaceRec {
    /// `size->metrics` (the TrueType size's current metrics)
    pub fn size_metrics(&self) -> &FtSizeMetrics {
        if self.size.metrics_root {
            &self.root.size.metrics
        } else {
            &self.size.hinted_metrics
        }
    }
}

/* TT_USE_BYTECODE_INTERPRETER */

/*
 *
 *                      GLYPH ZONE FUNCTIONS
 *
 */

/// `tt_glyphzone_done`: Deallocate a glyph zone.
pub fn tt_glyphzone_done(zone: &mut TtGlyphZoneRec) {
    zone.contours = Vec::new();
    zone.tags = Vec::new();
    zone.cur = Vec::new();
    zone.org = Vec::new();
    zone.orus = Vec::new();

    zone.max_points = 0;
    zone.n_points = 0;
    zone.max_contours = 0;
    zone.n_contours = 0;
}

/// `tt_glyphzone_new`: Allocate a new glyph zone.
pub fn tt_glyphzone_new(
    max_points: FtUShort,
    max_contours: FtShort,
    zone: &mut TtGlyphZoneRec,
) -> FtResult<()> {
    use super::super::base::ftmemory::ft_new_array;

    *zone = TtGlyphZoneRec::default();

    let r = (|| -> FtResult<()> {
        zone.org = ft_new_array(max_points as FtLong)?;
        zone.cur = ft_new_array(max_points as FtLong)?;
        zone.orus = ft_new_array(max_points as FtLong)?;
        zone.tags = ft_new_array(max_points as FtLong)?;
        zone.contours = ft_new_array(max_contours as FtLong)?;
        Ok(())
    })();

    match r {
        Err(e) => {
            tt_glyphzone_done(zone);
            Err(e)
        }
        Ok(()) => {
            zone.max_points = max_points;
            zone.max_contours = max_contours;
            Ok(())
        }
    }
}

/*
 * Fonts embedded in PDFs are made unique by prepending randomization
 * prefixes to their names: as defined in Section 5.5.3, 'Font Subsets',
 * of the PDF Reference, they consist of 6 uppercase letters followed by
 * the `+` sign.  For safety, we do not skip prefixes violating this rule.
 */
/// `tt_skip_pdffont_random_tag`
fn tt_skip_pdffont_random_tag(name: &[u8]) -> &[u8] {
    if name.len() < 8 || name[6] != b'+' {
        return name;
    }

    for &c in &name[..6] {
        if !ft_isupper(c) {
            return name;
        }
    }

    &name[7..]
}

/// `tt_check_trickyness_family`: Compare the face with a list of
/// well-known `tricky' fonts.  This list shall be expanded as we find
/// more of them.
fn tt_check_trickyness_family(name: &str) -> bool {
    const TRICK_NAMES_COUNT: usize = 20;

    static TRICK_NAMES: [&str; TRICK_NAMES_COUNT] = [
        /*
          PostScript names are given in brackets if they differ from the
          family name.  The version numbers, together with the copyright or
          release year data, are taken from fonts available to the
          developers.

          Note that later versions of the fonts might be no longer tricky;
          for example, `MingLiU' version 7.00 (file `mingliu.ttc' from
          Windows 7) is an ordinary TTC with non-tricky subfonts.
        */
        "cpop",             /* dftt-p7.ttf; version 1.00, 1992 [DLJGyShoMedium] */
        "DFGirl-W6-WIN-BF", /* dftt-h6.ttf; version 1.00, 1993 */
        "DFGothic-EB",      /* DynaLab Inc. 1992-1995 */
        "DFGyoSho-Lt",      /* DynaLab Inc. 1992-1995 */
        "DFHei",            /* DynaLab Inc. 1992-1995 [DFHei-Bd-WIN-HK-BF] */
        /* covers "DFHei-Md-HK-BF", maybe DynaLab Inc. */
        "DFHSGothic-W5", /* DynaLab Inc. 1992-1995 */
        "DFHSMincho-W3", /* DynaLab Inc. 1992-1995 */
        "DFHSMincho-W7", /* DynaLab Inc. 1992-1995 */
        "DFKaiSho-SB",   /* dfkaisb.ttf */
        "DFKaiShu",      /* covers "DFKaiShu-Md-HK-BF", maybe DynaLab Inc. */
        "DFKai-SB",      /* kaiu.ttf; version 3.00, 1998 [DFKaiShu-SB-Estd-BF] */
        "DFMing",        /* DynaLab Inc. 1992-1995 [DFMing-Md-WIN-HK-BF] */
        /* covers "DFMing-Bd-HK-BF", maybe DynaLab Inc. */
        "DLC", /* dftt-m7.ttf; version 1.00, 1993 [DLCMingBold] */
        /* dftt-f5.ttf; version 1.00, 1993 [DLCFongSung] */
        /* covers following */
        /* "DLCHayMedium", dftt-b5.ttf; version 1.00, 1993 */
        /* "DLCHayBold",   dftt-b7.ttf; version 1.00, 1993 */
        /* "DLCKaiMedium", dftt-k5.ttf; version 1.00, 1992 */
        /* "DLCLiShu",     dftt-l5.ttf; version 1.00, 1992 */
        /* "DLCRoundBold", dftt-r7.ttf; version 1.00, 1993 */
        "HuaTianKaiTi?",      /* htkt2.ttf */
        "HuaTianSongTi?",     /* htst3.ttf */
        "Ming(for ISO10646)", /* hkscsiic.ttf; version 0.12, 2007 [Ming] */
        /* iicore.ttf; version 0.07, 2007 [Ming] */
        "MingLiU", /* mingliu.ttf */
        /* mingliu.ttc; version 3.21, 2001 */
        "MingMedium", /* dftt-m5.ttf; version 1.00, 1993 [DLCMingMedium] */
        "PMingLiU",   /* mingliu.ttc; version 3.21, 2001 */
        "MingLi43",   /* mingli.ttf; version 1.00, 1992 */
    ];

    let name_without_tag = tt_skip_pdffont_random_tag(name.as_bytes());

    for trick in TRICK_NAMES.iter() {
        let t = trick.as_bytes();
        if name_without_tag.windows(t.len()).any(|w| w == t) {
            return true;
        }
    }

    false
}

/* XXX: This function should be in the `sfnt' module. */

/* Some PDF generators clear the checksums in the TrueType header table. */
/* For example, Quartz ContextPDF clears all entries, or Bullzip PDF     */
/* Printer clears the entries for subsetted subtables.  We thus have to  */
/* recalculate the checksums  where necessary.                           */

/// `tt_synth_sfnt_checksum`
fn tt_synth_sfnt_checksum(stream: &mut FtStreamRec, mut length: FtULong) -> FtUInt32 {
    let mut checksum: FtUInt32 = 0;

    if stream.enter_frame(length).is_err() {
        return 0;
    }

    while length > 3 {
        checksum = checksum.wrapping_add(stream.get_ulong());
        length -= 4;
    }

    let mut i = 3;
    while length > 0 {
        checksum = checksum.wrapping_add((stream.get_byte() as FtUInt32) << (i * 8));
        length -= 1;
        i -= 1;
    }

    stream.exit_frame();

    checksum
}

/* XXX: This function should be in the `sfnt' module. */

/// `tt_get_sfnt_checksum`
fn tt_get_sfnt_checksum(face: &TtFaceRec, stream: &mut FtStreamRec, i: FtUShort) -> FtULong {
    if tt_face_goto_table(face, face.dir_tables[i as usize].Tag, stream).is_err() {
        return 0;
    }

    tt_synth_sfnt_checksum(stream, face.dir_tables[i as usize].Length) as FtULong
}

/// `tt_sfnt_id_rec`
#[derive(Clone, Copy)]
struct TtSfntIdRec {
    check_sum: FtULong,
    length: FtULong,
}

/// `tt_check_trickyness_sfnt_ids`
fn tt_check_trickyness_sfnt_ids(face: &TtFaceRec, stream: &mut FtStreamRec) -> bool {
    const TRICK_SFNT_IDS_PER_FACE: i32 = 3;
    const TRICK_SFNT_IDS_NUM_FACES: usize = 31;

    const fn id(check_sum: FtULong, length: FtULong) -> TtSfntIdRec {
        TtSfntIdRec { check_sum, length }
    }

    static SFNT_ID: [[TtSfntIdRec; TRICK_SFNT_IDS_PER_FACE as usize]; TRICK_SFNT_IDS_NUM_FACES] = [
        [
            /* MingLiU 1995 */
            id(0x05BCF058, 0x000002E4), /* cvt  */
            id(0x28233BF1, 0x000087C4), /* fpgm */
            id(0xA344A1EA, 0x000001E1), /* prep */
        ],
        [
            /* MingLiU 1996- */
            id(0x05BCF058, 0x000002E4), /* cvt  */
            id(0x28233BF1, 0x000087C4), /* fpgm */
            id(0xA344A1EB, 0x000001E1), /* prep */
        ],
        [
            /* DFGothic-EB */
            id(0x12C3EBB2, 0x00000350), /* cvt  */
            id(0xB680EE64, 0x000087A7), /* fpgm */
            id(0xCE939563, 0x00000758), /* prep */
        ],
        [
            /* DFGyoSho-Lt */
            id(0x11E5EAD4, 0x00000350), /* cvt  */
            id(0xCE5956E9, 0x0000BC85), /* fpgm */
            id(0x8272F416, 0x00000045), /* prep */
        ],
        [
            /* DFHei-Md-HK-BF */
            id(0x1257EB46, 0x00000350), /* cvt  */
            id(0xF699D160, 0x0000715F), /* fpgm */
            id(0xD222F568, 0x000003BC), /* prep */
        ],
        [
            /* DFHSGothic-W5 */
            id(0x1262EB4E, 0x00000350), /* cvt  */
            id(0xE86A5D64, 0x00007940), /* fpgm */
            id(0x7850F729, 0x000005FF), /* prep */
        ],
        [
            /* DFHSMincho-W3 */
            id(0x122DEB0A, 0x00000350), /* cvt  */
            id(0x3D16328A, 0x0000859B), /* fpgm */
            id(0xA93FC33B, 0x000002CB), /* prep */
        ],
        [
            /* DFHSMincho-W7 */
            id(0x125FEB26, 0x00000350), /* cvt  */
            id(0xA5ACC982, 0x00007EE1), /* fpgm */
            id(0x90999196, 0x0000041F), /* prep */
        ],
        [
            /* DFKaiShu */
            id(0x11E5EAD4, 0x00000350), /* cvt  */
            id(0x5A30CA3B, 0x00009063), /* fpgm */
            id(0x13A42602, 0x0000007E), /* prep */
        ],
        [
            /* DFKaiShu, variant */
            id(0x11E5EAD4, 0x00000350), /* cvt  */
            id(0xA6E78C01, 0x00008998), /* fpgm */
            id(0x13A42602, 0x0000007E), /* prep */
        ],
        [
            /* DFKaiShu-Md-HK-BF */
            id(0x11E5EAD4, 0x00000360), /* cvt  */
            id(0x9DB282B2, 0x0000C06E), /* fpgm */
            id(0x53E6D7CA, 0x00000082), /* prep */
        ],
        [
            /* DFMing-Bd-HK-BF */
            id(0x1243EB18, 0x00000350), /* cvt  */
            id(0xBA0A8C30, 0x000074AD), /* fpgm */
            id(0xF3D83409, 0x0000037B), /* prep */
        ],
        [
            /* DLCLiShu */
            id(0x07DCF546, 0x00000308), /* cvt  */
            id(0x40FE7C90, 0x00008E2A), /* fpgm */
            id(0x608174B5, 0x0000007A), /* prep */
        ],
        [
            /* DLCHayBold */
            id(0xEB891238, 0x00000308), /* cvt  */
            id(0xD2E4DCD4, 0x0000676F), /* fpgm */
            id(0x8EA5F293, 0x000003B8), /* prep */
        ],
        [
            /* HuaTianKaiTi */
            id(0xFFFBFFFC, 0x00000008), /* cvt  */
            id(0x9C9E48B8, 0x0000BEA2), /* fpgm */
            id(0x70020112, 0x00000008), /* prep */
        ],
        [
            /* HuaTianSongTi */
            id(0xFFFBFFFC, 0x00000008), /* cvt  */
            id(0x0A5A0483, 0x00017C39), /* fpgm */
            id(0x70020112, 0x00000008), /* prep */
        ],
        [
            /* NEC fadpop7.ttf */
            id(0x00000000, 0x00000000), /* cvt  */
            id(0x40C92555, 0x000000E5), /* fpgm */
            id(0xA39B58E3, 0x0000117C), /* prep */
        ],
        [
            /* NEC fadrei5.ttf */
            id(0x00000000, 0x00000000), /* cvt  */
            id(0x33C41652, 0x000000E5), /* fpgm */
            id(0x26D6C52A, 0x00000F6A), /* prep */
        ],
        [
            /* NEC fangot7.ttf */
            id(0x00000000, 0x00000000), /* cvt  */
            id(0x6DB1651D, 0x0000019D), /* fpgm */
            id(0x6C6E4B03, 0x00002492), /* prep */
        ],
        [
            /* NEC fangyo5.ttf */
            id(0x00000000, 0x00000000), /* cvt  */
            id(0x40C92555, 0x000000E5), /* fpgm */
            id(0xDE51FAD0, 0x0000117C), /* prep */
        ],
        [
            /* NEC fankyo5.ttf */
            id(0x00000000, 0x00000000), /* cvt  */
            id(0x85E47664, 0x000000E5), /* fpgm */
            id(0xA6C62831, 0x00001CAA), /* prep */
        ],
        [
            /* NEC fanrgo5.ttf */
            id(0x00000000, 0x00000000), /* cvt  */
            id(0x2D891CFD, 0x0000019D), /* fpgm */
            id(0xA0604633, 0x00001DE8), /* prep */
        ],
        [
            /* NEC fangot5.ttc */
            id(0x00000000, 0x00000000), /* cvt  */
            id(0x40AA774C, 0x000001CB), /* fpgm */
            id(0x9B5CAA96, 0x00001F9A), /* prep */
        ],
        [
            /* NEC fanmin3.ttc */
            id(0x00000000, 0x00000000), /* cvt  */
            id(0x0D3DE9CB, 0x00000141), /* fpgm */
            id(0xD4127766, 0x00002280), /* prep */
        ],
        [
            /* NEC FA-Gothic, 1996 */
            id(0x00000000, 0x00000000), /* cvt  */
            id(0x4A692698, 0x000001F0), /* fpgm */
            id(0x340D4346, 0x00001FCA), /* prep */
        ],
        [
            /* NEC FA-Minchou, 1996 */
            id(0x00000000, 0x00000000), /* cvt  */
            id(0xCD34C604, 0x00000166), /* fpgm */
            id(0x6CF31046, 0x000022B0), /* prep */
        ],
        [
            /* NEC FA-RoundGothicB, 1996 */
            id(0x00000000, 0x00000000), /* cvt  */
            id(0x5DA75315, 0x0000019D), /* fpgm */
            id(0x40745A5F, 0x000022E0), /* prep */
        ],
        [
            /* NEC FA-RoundGothicM, 1996 */
            id(0x00000000, 0x00000000), /* cvt  */
            id(0xF055FC48, 0x000001C2), /* fpgm */
            id(0x3900DED3, 0x00001E18), /* prep */
        ],
        [
            /* MINGLI.TTF, 1992 */
            id(0x00170003, 0x00000060), /* cvt  */
            id(0xDBB4306E, 0x000058AA), /* fpgm */
            id(0xD643482A, 0x00000035), /* prep */
        ],
        [
            /* DFHei-Bd-WIN-HK-BF, issue #1087 */
            id(0x1269EB58, 0x00000350), /* cvt  */
            id(0x5CD5957A, 0x00006A4E), /* fpgm */
            id(0xF758323A, 0x00000380), /* prep */
        ],
        [
            /* DFMing-Md-WIN-HK-BF, issue #1087 */
            id(0x122FEB0B, 0x00000350), /* cvt  */
            id(0x7F10919A, 0x000070A9), /* fpgm */
            id(0x7CD7E7B7, 0x0000025C), /* prep */
        ],
    ];

    const TRICK_SFNT_ID_CVT: usize = 0;
    const TRICK_SFNT_ID_FPGM: usize = 1;
    const TRICK_SFNT_ID_PREP: usize = 2;

    let mut num_matched_ids = [0i32; TRICK_SFNT_IDS_NUM_FACES];
    let mut has_cvt = false;
    let mut has_fpgm = false;
    let mut has_prep = false;

    for i in 0..face.num_tables {
        let mut checksum: FtULong = 0;
        let k;

        match face.dir_tables[i as usize].Tag as u32 {
            TTAG_cvt => {
                k = TRICK_SFNT_ID_CVT;
                has_cvt = true;
            }

            TTAG_fpgm => {
                k = TRICK_SFNT_ID_FPGM;
                has_fpgm = true;
            }

            TTAG_prep => {
                k = TRICK_SFNT_ID_PREP;
                has_prep = true;
            }

            _ => continue,
        }

        for j in 0..TRICK_SFNT_IDS_NUM_FACES {
            if face.dir_tables[i as usize].Length == SFNT_ID[j][k].length {
                if checksum == 0 {
                    checksum = tt_get_sfnt_checksum(face, stream, i);
                }

                if SFNT_ID[j][k].check_sum == checksum {
                    num_matched_ids[j] += 1;
                }

                if num_matched_ids[j] == TRICK_SFNT_IDS_PER_FACE {
                    return true;
                }
            }
        }
    }

    for j in 0..TRICK_SFNT_IDS_NUM_FACES {
        if !has_cvt && SFNT_ID[j][TRICK_SFNT_ID_CVT].length == 0 {
            num_matched_ids[j] += 1;
        }
        if !has_fpgm && SFNT_ID[j][TRICK_SFNT_ID_FPGM].length == 0 {
            num_matched_ids[j] += 1;
        }
        if !has_prep && SFNT_ID[j][TRICK_SFNT_ID_PREP].length == 0 {
            num_matched_ids[j] += 1;
        }
        if num_matched_ids[j] == TRICK_SFNT_IDS_PER_FACE {
            return true;
        }
    }

    false
}

/// `tt_check_trickyness`
fn tt_check_trickyness(face: &TtFaceRec, stream: &mut FtStreamRec) -> bool {
    /* For first, check the face name for quick check. */
    if let Some(family_name) = &face.root.family_name {
        if tt_check_trickyness_family(family_name) {
            return true;
        }
    }

    /* Type42 fonts may lack `name' tables, we thus try to identify */
    /* tricky fonts by checking the checksums of Type42-persistent  */
    /* sfnt tables (`cvt', `fpgm', and `prep').                     */
    if tt_check_trickyness_sfnt_ids(face, stream) {
        return true;
    }

    false
}

/* TT_USE_BYTECODE_INTERPRETER */

/// `tt_check_single_notdef`: Check whether `.notdef' is the only glyph in
/// the `loca' table.
fn tt_check_single_notdef(face: &mut TtFaceRec, stream: &mut FtStreamRec) -> bool {
    let mut result = false;

    let mut glyph_index: FtULong = 0;
    let mut count: FtUInt = 0;

    for i in 0..face.num_locations {
        let (_, asize) = tt_face_get_location(face, i as FtUInt);
        if asize > 0 {
            count += 1;
            if count > 1 {
                break;
            }
            glyph_index = i;
        }
    }

    /* Only have a single outline. */
    if count == 1 {
        if glyph_index == 0 {
            result = true;
        } else {
            /* FIXME: Need to test glyphname == .notdef ? */

            /* FT_Get_Glyph_Name( ttface, glyph_index, buf, 8 ) */
            if (glyph_index as FtLong) < face.root.num_glyphs && ft_has_glyph_names(&face.root) {
                if let Ok(name) = tt_face_get_ps_name(face, stream, glyph_index as FtUInt) {
                    /* (FT_STRCPYN truncates the name to 7 characters) */
                    let buf = &name[..name.len().min(7)];
                    if buf.first() == Some(&b'.') && buf == b".notdef" {
                        result = true;
                    }
                }
            }
        }
    }

    result
}

/// `tt_face_init`: Initialize a given TrueType face object.
///
/// `face_index` is the index of the TrueType font, if we are opening a
/// collection, in bits 0-15.  The numbered instance index~+~1 of a GX
/// (sub)font, if applicable, in bits 16-30.
pub fn tt_face_init(
    face: &mut TtFaceRec,
    face_index: FtInt,
    params: &[FtParameter],
) -> FtResult<()> {
    /* (the `sfnt' module is called directly) */

    /* create input stream from resource */
    face.root.stream().seek(0)?;

    /* check that we have a valid TrueType file */
    sfnt_init_face(face, face_index, params)?;

    /* Stream may have changed. */

    /* We must also be able to accept Mac/GX fonts, as well as OT ones. */
    /* The 0x00020000 tag is completely undocumented; some fonts from   */
    /* Arphic made for Chinese Windows 3.1 have this.                   */
    if face.format_tag != 0x00010000 /* MS fonts                             */
        && face.format_tag != 0x00020000 /* CJK fonts for Win 3.1                */
        && face.format_tag != TTAG_true as FtULong /* Mac fonts                            */
        && face.format_tag != TTAG_0xA5kbd as FtULong /* `Keyboard.dfont' (legacy Mac OS X)   */
        && face.format_tag != TTAG_0xA5lst as FtULong
    /* `LastResort.dfont' (legacy Mac OS X) */
    {
        /* Bad_Format: */
        return Err(FT_ERR_UNKNOWN_FILE_FORMAT);
    }

    /* TT_USE_BYTECODE_INTERPRETER */
    face.root.face_flags |= FT_FACE_FLAG_HINTER;

    /* If we are performing a simple font format check, exit immediately. */
    if face_index < 0 {
        return Ok(());
    }

    /* Load font directory */
    sfnt_load_face(face, face_index, params)?;

    with_stream(face, |face, stream| -> FtResult<()> {
        /* TT_USE_BYTECODE_INTERPRETER */
        if tt_check_trickyness(face, stream) {
            face.root.face_flags |= FT_FACE_FLAG_TRICKY;
        }

        tt_face_load_hdmx(face, stream)?;

        if ft_is_scalable(&face.root) || ft_has_sbix(&face.root) {
            /* (FT_CONFIG_OPTION_INCREMENTAL: no incremental interface is set) */
            {
                let error = tt_face_load_loca(face, stream);

                /* having a (non-zero) `glyf' table without */
                /* a `loca' table is not valid              */
                error?;
            }

            /* `fpgm', `cvt', and `prep' are optional */
            if let Err(e) = tt_face_load_cvt(face, stream) {
                if ft_err_neq(e, FT_ERR_TABLE_MISSING) {
                    return Err(e);
                }
            }

            if let Err(e) = tt_face_load_fpgm(face, stream) {
                if ft_err_neq(e, FT_ERR_TABLE_MISSING) {
                    return Err(e);
                }
            }

            if let Err(e) = tt_face_load_prep(face, stream) {
                if ft_err_neq(e, FT_ERR_TABLE_MISSING) {
                    return Err(e);
                }
            }

            /* Check the scalable flag based on `loca'. */
            if face.root.num_fixed_sizes != 0
                && !face.glyph_locations.is_empty()
                && tt_check_single_notdef(face, stream)
            {
                face.root.face_flags &= !FT_FACE_FLAG_SCALABLE;
            }
        }

        Ok(())
    })?;

    /* TT_CONFIG_OPTION_GX_VAR_SUPPORT */
    {
        let instance_index = (face_index as FtUInt) >> 16;

        if ft_has_multiple_masters(&face.root) && instance_index > 0 {
            tt_set_named_instance_ftmm(face, instance_index)?;
        }
    }

    /* initialize standard glyph loading routines */
    /* (TT_Init_Glyph_Loading: the glyph loader is called directly) */

    Ok(())
}

/// `tt_face_done`: Finalize a given face object.
pub fn tt_face_done(face: &mut TtFaceRec) {
    /* for `extended TrueType formats' (i.e. compressed versions) */
    /* (no finalizer is ever set) */

    sfnt_done_face(face);

    /* freeing the locations table */
    tt_face_done_loca(face);

    tt_face_free_hdmx(face);

    /* freeing the CVT */
    face.cvt = Vec::new();
    face.cvt_size = 0;

    /* freeing the programs */
    face.font_program = Arc::from(Vec::new());
    face.cvt_program = Arc::from(Vec::new());
    face.font_program_size = 0;
    face.cvt_program_size = 0;

    /* TT_CONFIG_OPTION_GX_VAR_SUPPORT */
    tt_done_blend(face);
    face.blend = None;
}

/*
 *
 *                          SIZE  FUNCTIONS
 *
 */

/* TT_USE_BYTECODE_INTERPRETER */

/// `tt_size_run_fpgm`: Run the font program.
pub fn tt_size_run_fpgm(face: &mut TtFaceRec, pedantic: bool) -> FtResult<()> {
    let mut exec = match face.size.context.take() {
        Some(e) => e,
        None => return Err(FT_ERR_COULD_NOT_FIND_CONTEXT),
    };
    let interpreter_version = tt_driver_interpreter_version(face);

    let r = tt_size_run_fpgm_exec(face, &mut exec, pedantic, interpreter_version);

    tt_unload_context(&mut exec, &mut face.size);
    face.size.context = Some(exec);
    r
}

fn tt_size_run_fpgm_exec(
    face: &mut TtFaceRec,
    exec: &mut TtExecContextRec,
    pedantic: bool,
    interpreter_version: FtUInt,
) -> FtResult<()> {
    tt_load_context(exec, face, interpreter_version)?;

    exec.call_top = 0;
    exec.top = 0;

    exec.period = 64;
    exec.phase = 0;
    exec.threshold = 0;

    exec.instruction_trap = false;
    exec.f_dot_p = 0x4000;

    exec.pedantic_hinting = pedantic;

    {
        let size_metrics = &mut exec.metrics;
        let tt_metrics = &mut exec.tt_metrics;

        size_metrics.x_ppem = 0;
        size_metrics.y_ppem = 0;
        size_metrics.x_scale = 0;
        size_metrics.y_scale = 0;

        tt_metrics.ppem = 0;
        tt_metrics.scale = 0;
        tt_metrics.ratio = 0x10000;
    }

    /* allow font program execution */
    tt_set_code_range(
        exec,
        TT_CODERANGE_FONT,
        face.font_program.clone(),
        face.font_program_size as FtLong,
    );

    /* disable CVT and glyph programs coderange */
    tt_clear_code_range(exec, TT_CODERANGE_CVT);
    tt_clear_code_range(exec, TT_CODERANGE_GLYPH);

    let error = if face.font_program_size > 0 {
        tt_goto_code_range(exec, TT_CODERANGE_FONT, 0);

        tt_run_ins(exec)
    } else {
        Ok(())
    };

    face.size.bytecode_ready = match error {
        Ok(()) => 0,
        Err(e) => e,
    };

    if error.is_ok() {
        tt_save_context(exec, &mut face.size);
    }

    error
}

/// `tt_size_run_prep`: Run the control value program.
pub fn tt_size_run_prep(face: &mut TtFaceRec, pedantic: bool) -> FtResult<()> {
    /* unscaled CVT values are already stored in 26.6 format */
    let scale = face.size.ttmetrics.scale >> 6;

    /* Scale the cvt values to the new ppem.            */
    /* By default, we use the y ppem value for scaling. */
    for i in 0..face.size.cvt_size as usize {
        face.size.cvt[i] = ft_mul_fix(face.cvt[i] as FtLong, scale);
    }

    let mut exec = match face.size.context.take() {
        Some(e) => e,
        None => return Err(FT_ERR_COULD_NOT_FIND_CONTEXT),
    };
    let interpreter_version = tt_driver_interpreter_version(face);

    let r = tt_size_run_prep_exec(face, &mut exec, pedantic, interpreter_version);

    tt_unload_context(&mut exec, &mut face.size);
    face.size.context = Some(exec);
    r
}

fn tt_size_run_prep_exec(
    face: &mut TtFaceRec,
    exec: &mut TtExecContextRec,
    pedantic: bool,
    interpreter_version: FtUInt,
) -> FtResult<()> {
    tt_load_context(exec, face, interpreter_version)?;

    exec.call_top = 0;
    exec.top = 0;

    exec.instruction_trap = false;

    exec.pedantic_hinting = pedantic;

    tt_set_code_range(
        exec,
        TT_CODERANGE_CVT,
        face.cvt_program.clone(),
        face.cvt_program_size as FtLong,
    );

    tt_clear_code_range(exec, TT_CODERANGE_GLYPH);

    let error = if face.cvt_program_size > 0 {
        tt_goto_code_range(exec, TT_CODERANGE_CVT, 0);

        tt_run_ins(exec)
    } else {
        Ok(())
    };

    face.size.cvt_ready = match error {
        Ok(()) => 0,
        Err(e) => e,
    };

    /* UNDOCUMENTED!  The MS rasterizer doesn't allow the following */
    /* graphics state variables to be modified by the CVT program.  */

    exec.gs.dual_vector.x = 0x4000;
    exec.gs.dual_vector.y = 0;
    exec.gs.proj_vector.x = 0x4000;
    exec.gs.proj_vector.y = 0x0;
    exec.gs.free_vector.x = 0x4000;
    exec.gs.free_vector.y = 0x0;

    exec.gs.rp0 = 0;
    exec.gs.rp1 = 0;
    exec.gs.rp2 = 0;

    exec.gs.gep0 = 1;
    exec.gs.gep1 = 1;
    exec.gs.gep2 = 1;

    exec.gs.loop_ = 1;

    /* save as default graphics state */
    face.size.gs = exec.gs;

    tt_save_context(exec, &mut face.size);

    error
}

/// `tt_size_done_bytecode`
fn tt_size_done_bytecode(face: &mut TtFaceRec) {
    let size = &mut face.size;

    if let Some(c) = size.context.take() {
        tt_done_context(c);
    }

    size.cvt = Vec::new();
    size.cvt_size = 0;

    /* free storage area */
    size.storage = Vec::new();
    size.storage_size = 0;

    /* twilight zone */
    tt_glyphzone_done(&mut size.twilight);

    size.function_defs = Vec::new();
    size.instruction_defs = Vec::new();

    size.num_function_defs = 0;
    size.max_function_defs = 0;
    size.num_instruction_defs = 0;
    size.max_instruction_defs = 0;

    size.max_func = 0;
    size.max_ins = 0;

    size.bytecode_ready = -1;
    size.cvt_ready = -1;
}

/// `tt_size_init_bytecode`: Initialize bytecode-related fields in the size
/// object.  We do this only if bytecode interpretation is really needed.
fn tt_size_init_bytecode(face: &mut TtFaceRec, pedantic: bool) -> FtResult<()> {
    use super::super::base::ftmemory::ft_new_array;

    let maxp = face.max_profile;
    let cvt_size = face.cvt_size;

    {
        let size = &mut face.size;

        /* clean up bytecode related data */
        size.function_defs = Vec::new();
        size.instruction_defs = Vec::new();
        size.cvt = Vec::new();
        size.storage = Vec::new();

        if let Some(c) = size.context.take() {
            tt_done_context(c);
        }
        tt_glyphzone_done(&mut size.twilight);

        size.bytecode_ready = -1;
        size.cvt_ready = -1;

        size.context = tt_new_context();

        size.max_function_defs = maxp.maxFunctionDefs as FtUInt;
        size.max_instruction_defs = maxp.maxInstructionDefs as FtUInt;

        size.num_function_defs = 0;
        size.num_instruction_defs = 0;

        size.max_func = 0;
        size.max_ins = 0;

        size.cvt_size = cvt_size;
        size.storage_size = maxp.maxStorage;

        /* Set default metrics */
        {
            let tt_metrics = &mut size.ttmetrics;

            tt_metrics.rotated = false;
            tt_metrics.stretched = false;

            /* Set default engine compensation.  Value 3 is not described */
            /* in the OpenType specification (as of Mai 2019), but Greg   */
            /* says that MS handles it the same as `gray'.                */
            /*                                                            */
            /* The Apple specification says that the compensation for     */
            /* `gray' is always zero.  FreeType doesn't do any            */
            /* compensation at all.                                       */
            tt_metrics.compensations[0] = 0; /* gray  */
            tt_metrics.compensations[1] = 0; /* black */
            tt_metrics.compensations[2] = 0; /* white */
            tt_metrics.compensations[3] = 0; /* zero  */
        }
    }

    let r = (|| -> FtResult<()> {
        let size = &mut face.size;

        /* allocate function defs, instruction defs, cvt, and storage area */
        size.function_defs = ft_new_array(size.max_function_defs as FtLong)?;
        size.instruction_defs = ft_new_array(size.max_instruction_defs as FtLong)?;
        size.cvt = ft_new_array(size.cvt_size as FtLong)?;
        size.storage = ft_new_array(size.storage_size as FtLong)?;

        /* reserve twilight zone */
        let mut n_twilight = maxp.maxTwilightPoints;

        /* there are 4 phantom points (do we need this?) */
        n_twilight = n_twilight.wrapping_add(4);

        tt_glyphzone_new(n_twilight, 0, &mut size.twilight)?;

        size.twilight.n_points = n_twilight;

        size.gs = TT_DEFAULT_GRAPHICS_STATE;

        /* set `face->interpreter' according to the debug hook present */
        /* (always TT_RunIns) */

        Ok(())
    })();

    if let Err(e) = r {
        /* Exit: */
        tt_size_done_bytecode(face);
        return Err(e);
    }

    /* Fine, now run the font program! */

    /* In case of an error while executing `fpgm', we intentionally don't */
    /* clean up immediately – bugs in the `fpgm' are so fundamental that  */
    /* all following hinting calls should fail.  Additionally, `fpgm' is  */
    /* to be executed just once; calling it again is completely useless   */
    /* and might even lead to extremely slow behaviour if it is malformed */
    /* (containing an infinite loop, for example).                        */
    tt_size_run_fpgm(face, pedantic)
}

/// `tt_size_ready_bytecode`
pub fn tt_size_ready_bytecode(face: &mut TtFaceRec, pedantic: bool) -> FtResult<()> {
    if face.size.bytecode_ready < 0 {
        tt_size_init_bytecode(face, pedantic)?;
    } else if face.size.bytecode_ready != 0 {
        return Err(face.size.bytecode_ready);
    }

    /* rescale CVT when needed */
    if face.size.cvt_ready < 0 {
        let size = &mut face.size;

        /* all twilight points are originally zero */
        for i in 0..size.twilight.n_points as usize {
            size.twilight.org[i].x = 0;
            size.twilight.org[i].y = 0;
            size.twilight.cur[i].x = 0;
            size.twilight.cur[i].y = 0;
        }

        /* clear storage area */
        for i in 0..size.storage_size as usize {
            size.storage[i] = 0;
        }

        size.gs = TT_DEFAULT_GRAPHICS_STATE;

        tt_size_run_prep(face, pedantic)
    } else if face.size.cvt_ready != 0 {
        Err(face.size.cvt_ready)
    } else {
        Ok(())
    }
}

/// `tt_size_init`: Initialize a new TrueType size object.
pub fn tt_size_init(face: &mut TtFaceRec) -> FtResult<()> {
    let size = &mut face.size;

    /* TT_USE_BYTECODE_INTERPRETER */
    size.bytecode_ready = -1;
    size.cvt_ready = -1;

    size.ttmetrics.valid = false;
    size.strike_index = 0xFFFFFFFF;

    Ok(())
}

/// `tt_size_done`: The TrueType size object finalizer.
pub fn tt_size_done(face: &mut TtFaceRec) {
    /* TT_USE_BYTECODE_INTERPRETER */
    tt_size_done_bytecode(face);

    face.size.ttmetrics.valid = false;
}

/// `tt_size_reset_height`: Recompute a TrueType size's ascender,
/// descender, and height when resolutions and character dimensions have
/// been changed.  Used for variation fonts as an iterator function.
pub fn tt_size_reset_height(face: &mut TtFaceRec) -> FtResult<()> {
    face.size.ttmetrics.valid = false;

    /* copy the result from base layer */
    face.size.hinted_metrics = face.root.size.metrics;
    let size_metrics = &mut face.size.hinted_metrics;

    if size_metrics.x_ppem < 1 || size_metrics.y_ppem < 1 {
        return Err(FT_ERR_INVALID_PPEM);
    }

    /* This bit flag, if set, indicates that the ppems must be       */
    /* rounded to integers.  Nearly all TrueType fonts have this bit */
    /* set, as hinting won't work really well otherwise.             */
    /*                                                               */
    if face.header.Flags & 8 != 0 {
        /* the TT spec always asks for ROUND, not FLOOR or CEIL */
        size_metrics.ascender = ft_pix_round(ft_mul_fix(
            face.root.ascender as FtLong,
            size_metrics.y_scale,
        ));
        size_metrics.descender = ft_pix_round(ft_mul_fix(
            face.root.descender as FtLong,
            size_metrics.y_scale,
        ));
        size_metrics.height =
            ft_pix_round(ft_mul_fix(face.root.height as FtLong, size_metrics.y_scale));
    }

    face.size.ttmetrics.valid = true;

    Ok(())
}

/// `tt_size_reset`: Reset a TrueType size when resolutions and character
/// dimensions have been changed.
pub fn tt_size_reset(face: &mut TtFaceRec) -> FtResult<()> {
    tt_size_reset_height(face)?;

    let units_per_em = face.root.units_per_EM as FtLong;
    let max_advance_width = face.root.max_advance_width as FtLong;
    let size_metrics = &mut face.size.hinted_metrics;

    if face.header.Flags & 8 != 0 {
        /* base scaling values on integer ppem values, */
        /* as mandated by the TrueType specification   */
        size_metrics.x_scale = ft_div_fix((size_metrics.x_ppem as FtLong) << 6, units_per_em);
        size_metrics.y_scale = ft_div_fix((size_metrics.y_ppem as FtLong) << 6, units_per_em);

        size_metrics.max_advance =
            ft_pix_round(ft_mul_fix(max_advance_width, size_metrics.x_scale));
    }

    let size_metrics = *size_metrics;
    let size = &mut face.size;

    /* compute new transformation */
    if size_metrics.x_ppem >= size_metrics.y_ppem {
        size.ttmetrics.scale = size_metrics.x_scale;
        size.ttmetrics.ppem = size_metrics.x_ppem;
        size.ttmetrics.x_ratio = 0x10000;
        size.ttmetrics.y_ratio =
            ft_div_fix(size_metrics.y_ppem as FtLong, size_metrics.x_ppem as FtLong);
    } else {
        size.ttmetrics.scale = size_metrics.y_scale;
        size.ttmetrics.ppem = size_metrics.y_ppem;
        size.ttmetrics.x_ratio =
            ft_div_fix(size_metrics.x_ppem as FtLong, size_metrics.y_ppem as FtLong);
        size.ttmetrics.y_ratio = 0x10000;
    }

    face.size.widthp = tt_face_get_device_metrics(face, size_metrics.x_ppem as FtUInt, 0);

    face.size.metrics_root = false;

    /* TT_USE_BYTECODE_INTERPRETER */
    face.size.cvt_ready = -1;

    Ok(())
}

/// `tt_driver_init`: Initialize a given TrueType driver object.
pub fn tt_driver_init(library: &mut FtLibraryRec, module: usize) -> FtResult<()> {
    /* TT_USE_BYTECODE_INTERPRETER */
    let mut driver = TtDriverRec {
        interpreter_version: TT_INTERPRETER_VERSION_35,
    };

    /* TT_SUPPORT_SUBPIXEL_HINTING_MINIMAL */
    driver.interpreter_version = TT_INTERPRETER_VERSION_40;

    let m = &library.modules[module];
    *m.props.lock().unwrap_or_else(|e| e.into_inner()) = Some(Box::new(driver));

    Ok(())
}

/// `tt_driver_done`: Finalize a given TrueType driver.
pub fn tt_driver_done(_ttdriver: &FtModuleRec) {}

/// `tt_slot_init`: Initialize a new slot object.
pub fn tt_slot_init(slot: &mut FtGlyphSlotRec) -> FtResult<()> {
    match slot.internal.loader.as_mut() {
        Some(loader) => loader.create_extra(),
        None => Ok(()),
    }
}
