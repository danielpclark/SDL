// Rust translation of src/truetype/ttgxvar.c (and ttgxvar.h, with the types
// of include/freetype/internal/ftmmtypes.h and ftmm.h) from FreeType (2.13.2,
// as SDL_ttf's external/freetype pins it).
// Copyright (C) 2004-2023 by David Turner, Robert Wilhelm, Werner Lemberg,
// and George Williams.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! TrueType GX Font Variation loader.
//!
//! Apple documents the `fvar', `gvar', `cvar', and `avar' tables at
//!
//!   <https://developer.apple.com/fonts/TrueType-Reference-Manual/RM06/Chap6[fgca]var.html>
//!
//! The documentation for `gvar' is not intelligible; `cvar' refers you
//! to `gvar' and is thus also incomprehensible.
//!
//! The documentation for `avar' appears correct, but Apple has no fonts
//! with an `avar' table, so it is hard to test.
//!
//! Many thanks to John Jenkins (at Apple) in figuring this out.
//!
//! Apple's `kern' table has some references to tuple indices, but as
//! there is no indication where these indices are defined, nor how to
//! interpolate the kerning values (different tuples have different
//! classes) this issue is ignored.
//!
//! Translation notes: `FT_MM_Var` is a plain struct here (C packs it into
//! a single allocation of `mmvar_len` bytes); the multiple masters and
//! metrics variations services are called directly; and only the parts
//! of `FT_Set_Named_Instance` (ftmm.c) that `tt_face_init` needs are in
//! [`tt_set_named_instance_ftmm`].

use super::super::base::ftcalc::*;
use super::super::base::ftmemory::ft_new_array;
use super::super::base::ftobjs::*;
use super::super::base::ftstream::*;
use super::super::fttypes::*;
use super::super::sfnt::sfdriver::sfnt_get_name_id;
use super::super::sfnt::sfobjs::tt_face_get_name;
use super::super::sfnt::ttload::{tt_face_goto_table, with_stream};
use super::super::tttables::*;
use super::super::tttypes::*;
use super::ttobjs::tt_size_reset_height;
use super::ttpload::tt_face_load_cvt;

/* ftmmtypes.h */

/// `FT_ItemVarDelta`
pub type FtItemVarDelta = FtInt32;

/// `GX_ItemVarDataRec`
#[derive(Debug, Clone, Default)]
pub struct GxItemVarDataRec {
    pub itemCount: FtUInt,          /* Number of delta sets per item.   */
    pub regionIdxCount: FtUInt,     /* Number of region indices.        */
    pub regionIndices: Vec<FtUInt>, /* Array of `regionCount` indices;  */
    /* these index `varRegionList`.     */
    pub deltaSet: Vec<u8>, /* Array of `itemCount` deltas;     */
    /* use `innerIndex` for this array. */
    pub wordDeltaCount: FtUShort, /* Number of the first 32-bit ints  */
    /* or 16-bit ints of `deltaSet`     */
    /* depending on `longWords`.        */
    pub longWords: bool, /* If true, `deltaSet` is a 32-bit  */
                         /* array followed by a 16-bit       */
                         /* array, otherwise a 16-bit array  */
                         /* followed by an 8-bit array.      */
}

/// `GX_AxisCoordsRec`: contribution of one axis to a region
#[derive(Debug, Clone, Copy, Default)]
pub struct GxAxisCoordsRec {
    pub startCoord: FtFixed,
    pub peakCoord: FtFixed, /* zero means no effect (factor = 1) */
    pub endCoord: FtFixed,
}

/// `GX_VarRegionRec`
#[derive(Debug, Clone, Default)]
pub struct GxVarRegionRec {
    pub axisList: Vec<GxAxisCoordsRec>, /* array of axisCount records */
}

/// `GX_ItemVarStoreRec`: item variation store
#[derive(Debug, Clone, Default)]
pub struct GxItemVarStoreRec {
    pub dataCount: FtUInt,
    pub varData: Vec<GxItemVarDataRec>, /* array of dataCount records;     */
    /* use `outerIndex' for this array */
    pub axisCount: FtUShort,
    pub regionCount: FtUInt, /* total number of regions defined */
    pub varRegionList: Vec<GxVarRegionRec>,
}

/// `GX_DeltaSetIdxMapRec`
#[derive(Debug, Clone, Default)]
pub struct GxDeltaSetIdxMapRec {
    pub mapCount: FtULong,
    pub outerIndex: Vec<FtUInt>, /* indices to item var data */
    pub innerIndex: Vec<FtUInt>, /* indices to delta set     */
}

/* ftmm.h */

/// `FT_Var_Axis`
#[derive(Debug, Clone, Default)]
pub struct FtVarAxis {
    /// `name` (the axis tag as a string, as `TT_Get_MM_Var` sets it up)
    pub name: String,

    pub minimum: FtFixed,
    pub def: FtFixed,
    pub maximum: FtFixed,

    pub tag: FtULong,
    pub strid: FtUInt,
}

/// `FT_Var_Named_Style`
#[derive(Debug, Clone, Default)]
pub struct FtVarNamedStyle {
    pub coords: Vec<FtFixed>,
    pub strid: FtUInt,
    pub psid: FtUInt, /* since 2.7.1 */
}

/// `FT_MM_Var`
#[derive(Debug, Clone, Default)]
pub struct FtMMVar {
    pub num_axis: FtUInt,
    pub num_designs: FtUInt,
    pub num_namedstyles: FtUInt,
    pub axis: Vec<FtVarAxis>,
    pub namedstyle: Vec<FtVarNamedStyle>,
    /// the axis flags (C keeps them right after the `FT_MM_Var` struct:
    /// `alas, no public field in FT_Var_Axis for axis flags')
    pub axis_flags: Vec<FtUShort>,
}

/* ttgxvar.h */

/// `GX_AVarCorrespondenceRec`: A data structure representing
/// `shortFracCorrespondence' in `avar' table according to the
/// specifications from Apple.
#[derive(Debug, Clone, Copy, Default)]
pub struct GxAVarCorrespondenceRec {
    pub fromCoord: FtFixed,
    pub toCoord: FtFixed,
}

/// `GX_AVarSegmentRec`: Data from the segment field of `avar' table.
/// There is one of these for each axis.
#[derive(Debug, Clone, Default)]
pub struct GxAVarSegmentRec {
    pub pairCount: FtUShort,
    pub correspondence: Vec<GxAVarCorrespondenceRec>, /* array with pairCount entries */
}

/// `GX_AVarTableRec`: Data from the `avar' table.
#[derive(Debug, Clone, Default)]
pub struct GxAVarTableRec {
    /// `avar_segment[num_axis]` (`None` is C's NULL)
    pub avar_segment: Option<Vec<GxAVarSegmentRec>>,
    pub itemStore: GxItemVarStoreRec, /* Item Variation Store   */
    pub axisMap: GxDeltaSetIdxMapRec, /* Axis Mapping           */
}

/// `GX_HVVarTableRec`: Data from either the `HVAR' or `VVAR' table.
#[derive(Debug, Clone, Default)]
pub struct GxHVVarTableRec {
    pub itemStore: GxItemVarStoreRec, /* Item Variation Store  */
    pub widthMap: GxDeltaSetIdxMapRec, /* Advance Width Mapping */
                                      /* (lsbMap, rsbMap, tsbMap, bsbMap, vorgMap: not implemented) */
}

const MVAR_TAG_GASP_0: FtULong = ft_make_tag(b'g', b's', b'p', b'0') as FtULong;
const MVAR_TAG_GASP_1: FtULong = ft_make_tag(b'g', b's', b'p', b'1') as FtULong;
const MVAR_TAG_GASP_2: FtULong = ft_make_tag(b'g', b's', b'p', b'2') as FtULong;
const MVAR_TAG_GASP_3: FtULong = ft_make_tag(b'g', b's', b'p', b'3') as FtULong;
const MVAR_TAG_GASP_4: FtULong = ft_make_tag(b'g', b's', b'p', b'4') as FtULong;
const MVAR_TAG_GASP_5: FtULong = ft_make_tag(b'g', b's', b'p', b'5') as FtULong;
const MVAR_TAG_GASP_6: FtULong = ft_make_tag(b'g', b's', b'p', b'6') as FtULong;
const MVAR_TAG_GASP_7: FtULong = ft_make_tag(b'g', b's', b'p', b'7') as FtULong;
const MVAR_TAG_GASP_8: FtULong = ft_make_tag(b'g', b's', b'p', b'8') as FtULong;
const MVAR_TAG_GASP_9: FtULong = ft_make_tag(b'g', b's', b'p', b'9') as FtULong;

const MVAR_TAG_CPHT: FtULong = ft_make_tag(b'c', b'p', b'h', b't') as FtULong;
const MVAR_TAG_HASC: FtULong = ft_make_tag(b'h', b'a', b's', b'c') as FtULong;
const MVAR_TAG_HCLA: FtULong = ft_make_tag(b'h', b'c', b'l', b'a') as FtULong;
const MVAR_TAG_HCLD: FtULong = ft_make_tag(b'h', b'c', b'l', b'd') as FtULong;
const MVAR_TAG_HCOF: FtULong = ft_make_tag(b'h', b'c', b'o', b'f') as FtULong;
const MVAR_TAG_HCRN: FtULong = ft_make_tag(b'h', b'c', b'r', b'n') as FtULong;
const MVAR_TAG_HCRS: FtULong = ft_make_tag(b'h', b'c', b'r', b's') as FtULong;
const MVAR_TAG_HDSC: FtULong = ft_make_tag(b'h', b'd', b's', b'c') as FtULong;
const MVAR_TAG_HLGP: FtULong = ft_make_tag(b'h', b'l', b'g', b'p') as FtULong;
const MVAR_TAG_SBXO: FtULong = ft_make_tag(b's', b'b', b'x', b'o') as FtULong;
const MVAR_TAG_SBXS: FtULong = ft_make_tag(b's', b'b', b'x', b's') as FtULong;
const MVAR_TAG_SBYO: FtULong = ft_make_tag(b's', b'b', b'y', b'o') as FtULong;
const MVAR_TAG_SBYS: FtULong = ft_make_tag(b's', b'b', b'y', b's') as FtULong;
const MVAR_TAG_SPXO: FtULong = ft_make_tag(b's', b'p', b'x', b'o') as FtULong;
const MVAR_TAG_SPXS: FtULong = ft_make_tag(b's', b'p', b'x', b's') as FtULong;
const MVAR_TAG_SPYO: FtULong = ft_make_tag(b's', b'p', b'y', b'o') as FtULong;
const MVAR_TAG_SPYS: FtULong = ft_make_tag(b's', b'p', b'y', b's') as FtULong;
const MVAR_TAG_STRO: FtULong = ft_make_tag(b's', b't', b'r', b'o') as FtULong;
const MVAR_TAG_STRS: FtULong = ft_make_tag(b's', b't', b'r', b's') as FtULong;
const MVAR_TAG_UNDO: FtULong = ft_make_tag(b'u', b'n', b'd', b'o') as FtULong;
const MVAR_TAG_UNDS: FtULong = ft_make_tag(b'u', b'n', b'd', b's') as FtULong;
const MVAR_TAG_VASC: FtULong = ft_make_tag(b'v', b'a', b's', b'c') as FtULong;
const MVAR_TAG_VCOF: FtULong = ft_make_tag(b'v', b'c', b'o', b'f') as FtULong;
const MVAR_TAG_VCRN: FtULong = ft_make_tag(b'v', b'c', b'r', b'n') as FtULong;
const MVAR_TAG_VCRS: FtULong = ft_make_tag(b'v', b'c', b'r', b's') as FtULong;
const MVAR_TAG_VDSC: FtULong = ft_make_tag(b'v', b'd', b's', b'c') as FtULong;
const MVAR_TAG_VLGP: FtULong = ft_make_tag(b'v', b'l', b'g', b'p') as FtULong;
const MVAR_TAG_XHGT: FtULong = ft_make_tag(b'x', b'h', b'g', b't') as FtULong;

/// `GX_ValueRec`
#[derive(Debug, Clone, Copy, Default)]
pub struct GxValueRec {
    pub tag: FtULong,
    pub outerIndex: FtUShort,
    pub innerIndex: FtUShort,

    pub unmodified: FtShort, /* values are either FT_Short or FT_UShort */
}

/// `GX_MVarTableRec`: Data from the `MVAR' table.
#[derive(Debug, Clone, Default)]
pub struct GxMVarTableRec {
    pub valueCount: FtUShort,

    pub itemStore: GxItemVarStoreRec, /* Item Variation Store  */
    pub values: Vec<GxValueRec>,      /* Value Records         */
}

/// `GX_BlendRec`: Data for interpolating a font from a distortable font
/// specified by the GX *var tables (\[fgcahvm\]var).
#[derive(Debug, Clone, Default)]
pub struct GxBlendRec {
    pub num_axis: FtUInt,
    /// `coords` (`None` is C's NULL)
    pub coords: Option<Vec<FtFixed>>,
    /// `normalizedcoords` (`None` is C's NULL)
    pub normalizedcoords: Option<Vec<FtFixed>>,

    pub mmvar: Option<Box<FtMMVar>>,
    pub mmvar_len: FtULong,

    /// `normalized_stylecoords[num_namedstyles][num_axis]`
    pub normalized_stylecoords: Vec<FtFixed>,

    pub avar_loaded: bool,
    pub avar_table: Option<Box<GxAVarTableRec>>,

    pub hvar_loaded: bool,
    pub hvar_checked: bool,
    pub hvar_error: FtError,
    pub hvar_table: Option<Box<GxHVVarTableRec>>,

    pub vvar_loaded: bool,
    pub vvar_checked: bool,
    pub vvar_error: FtError,
    pub vvar_table: Option<Box<GxHVVarTableRec>>,

    pub mvar_table: Option<Box<GxMVarTableRec>>,

    pub tuplecount: FtUInt,
    /// `tuplecoords[tuplecount][num_axis]` (`None` is C's NULL)
    pub tuplecoords: Option<Vec<FtFixed>>,

    pub gv_glyphcnt: FtUInt,
    /// `glyphoffsets[gv_glyphcnt + 1]` (`None` is C's NULL)
    pub glyphoffsets: Option<Vec<FtULong>>,

    pub gvar_size: FtULong,
}

/// `GX_TupleCountFlags`: Flags used within the `TupleCount' field of the
/// `gvar' table.
pub const GX_TC_TUPLES_SHARE_POINT_NUMBERS: FtUInt = 0x8000;
pub const GX_TC_RESERVED_TUPLE_FLAGS: FtUInt = 0x7000;
pub const GX_TC_TUPLE_COUNT_MASK: FtUInt = 0x0FFF;

/// `GX_TupleIndexFlags`: Flags used within the `TupleIndex' field of the
/// `gvar' and `cvar' tables.
pub const GX_TI_EMBEDDED_TUPLE_COORD: FtUInt = 0x8000;
pub const GX_TI_INTERMEDIATE_TUPLE: FtUInt = 0x4000;
pub const GX_TI_PRIVATE_POINT_NUMBERS: FtUInt = 0x2000;
pub const GX_TI_RESERVED_TUPLE_FLAG: FtUInt = 0x1000;
pub const GX_TI_TUPLE_INDEX_MASK: FtUInt = 0x0FFF;

const TTAG_WGHT: FtULong = ft_make_tag(b'w', b'g', b'h', b't') as FtULong;
const TTAG_WDTH: FtULong = ft_make_tag(b'w', b'd', b't', b'h') as FtULong;
const TTAG_OPSZ: FtULong = ft_make_tag(b'o', b'p', b's', b'z') as FtULong;
const TTAG_SLNT: FtULong = ft_make_tag(b's', b'l', b'n', b't') as FtULong;
const TTAG_ITAL: FtULong = ft_make_tag(b'i', b't', b'a', b'l') as FtULong;

/// The outline `TT_Vary_Apply_Glyph_Deltas` changes (C passes an
/// `FT_Outline` with appended phantom points).
#[derive(Debug)]
pub struct GxOutlineView<'a> {
    /// the number of points without the four phantom points
    pub n_points: FtShort,
    pub n_contours: FtShort,
    /// the points, including the phantom points
    pub points: &'a mut [FtVector],
    pub tags: &'a [u8],
    pub contours: &'a [FtShort],
}

/// The loader fields `TT_Vary_Apply_Glyph_Deltas` updates.
#[derive(Debug, Clone, Copy, Default)]
pub struct GxVaryPhantoms {
    pub pp1: FtVector,
    pub pp2: FtVector,
    pub pp3: FtVector,
    pub pp4: FtVector,
    pub linear: FtInt,
    pub vadvance: FtInt,
}

/// `FT_Stream_FTell`: the cursor's position relative to `stream->base`
/// (the memory of a memory-based stream, or the frame otherwise)
fn ft_stream_ftell(stream: &FtStreamRec) -> FtULong {
    (stream.frame_origin() + stream.cursor()) as FtULong
}

/// `FT_Stream_SeekSet`
fn ft_stream_seek_set(stream: &mut FtStreamRec, off: FtULong) {
    let origin = stream.frame_origin() as FtULong;
    let limit = stream.limit();
    if off < origin + limit as FtULong {
        stream.set_cursor(off.wrapping_sub(origin) as usize);
    } else {
        stream.set_cursor(limit);
    }
}

/* some macros we need */

/// `FT_fdot14ToFixed`
#[inline]
fn ft_fdot14_to_fixed(x: FtShort) -> FtFixed {
    ((x as FtLong as FtULong) << 2) as FtFixed
}

/// `FT_intToFixed`
#[inline]
fn ft_int_to_fixed(i: FtLong) -> FtFixed {
    ((i as FtULong) << 16) as FtFixed
}

/// `FT_fdot6ToFixed`
#[allow(dead_code)]
#[inline]
fn ft_fdot6_to_fixed(i: FtLong) -> FtFixed {
    ((i as FtULong) << 10) as FtFixed
}

/// `FT_fixedToInt`
#[inline]
fn ft_fixed_to_int(x: FtFixed) -> FtShort {
    ((x.wrapping_add(0x8000)) >> 16) as FtShort
}

/// `FT_fixedToFdot6`
#[inline]
fn ft_fixed_to_fdot6(x: FtFixed) -> FtPos {
    (x.wrapping_add(0x200)) >> 10
}

/*************************************************************************/
/*************************************************************************/
/*****                                                               *****/
/*****                       Internal Routines                       *****/
/*****                                                               *****/
/*************************************************************************/
/*************************************************************************/

/// The point numbers `ft_var_readpackedpoints` returns: C's `ALL_POINTS`
/// (a delta for every point without needing to enumerate all of them),
/// or the points.
#[derive(Debug)]
enum PackedPoints {
    All,
    Points(Vec<FtUShort>),
}

const GX_PT_POINTS_ARE_WORDS: FtUInt = 0x80;
const GX_PT_POINT_RUN_COUNT_MASK: FtUInt = 0x7F;

/// `ft_var_readpackedpoints`: Read a set of points to which the following
/// deltas will apply.  Points are packed with a run length encoding.
///
/// Returns the points (or `PackedPoints::All`) and the number of points
/// read; `None` is C's NULL.
fn ft_var_readpackedpoints(
    stream: &mut FtStreamRec,
    size: FtULong,
    point_cnt: &mut FtUInt,
) -> Option<PackedPoints> {
    *point_cnt = 0;

    let mut n = stream.get_byte() as FtUInt;
    if n == 0 {
        return Some(PackedPoints::All);
    }

    if n & GX_PT_POINTS_ARE_WORDS != 0 {
        n &= GX_PT_POINT_RUN_COUNT_MASK;
        n <<= 8;
        n |= stream.get_byte() as FtUInt;
    }

    if n as FtULong > size {
        return None;
    }

    /* in the nested loops below we increase `i' twice; */
    /* it is faster to simply allocate one more slot    */
    /* than to add another test within the loop         */
    let mut points: Vec<FtUShort> = ft_new_array(n as FtLong + 1).ok()?;

    *point_cnt = n;

    let mut first: FtUShort = 0;
    let mut i: FtUInt = 0;
    while i < n {
        let mut runcnt = stream.get_byte() as FtUInt;
        if runcnt & GX_PT_POINTS_ARE_WORDS != 0 {
            runcnt &= GX_PT_POINT_RUN_COUNT_MASK;
            first = first.wrapping_add(stream.get_ushort());
            points[i as usize] = first;
            i += 1;

            /* first point not included in run count */
            for _ in 0..runcnt {
                first = first.wrapping_add(stream.get_ushort());
                points[i as usize] = first;
                i += 1;
                if i >= n {
                    break;
                }
            }
        } else {
            first = first.wrapping_add(stream.get_byte() as FtUShort);
            points[i as usize] = first;
            i += 1;

            for _ in 0..runcnt {
                first = first.wrapping_add(stream.get_byte() as FtUShort);
                points[i as usize] = first;
                i += 1;
                if i >= n {
                    break;
                }
            }
        }
    }

    Some(PackedPoints::Points(points))
}

const GX_DT_DELTAS_ARE_ZERO: FtUInt = 0x80;
const GX_DT_DELTAS_ARE_WORDS: FtUInt = 0x40;
const GX_DT_DELTA_RUN_COUNT_MASK: FtUInt = 0x3F;

/// `ft_var_readpackeddeltas`: Read a set of deltas.  These are packed
/// slightly differently than points.  In particular there is no overall
/// count.
///
/// We use FT_Fixed to avoid accumulation errors while summing up all
/// deltas (the rounding to integer values happens as the very last
/// step).
fn ft_var_readpackeddeltas(
    stream: &mut FtStreamRec,
    size: FtULong,
    delta_cnt: FtUInt,
) -> Option<Vec<FtFixed>> {
    let mut deltas: Vec<FtFixed> = ft_new_array(delta_cnt as FtLong).ok()?;

    let mut i: FtUInt = 0;
    let mut bytes_used: FtUInt = 0;

    while i < delta_cnt && (bytes_used as FtULong) < size {
        let runcnt = stream.get_byte() as FtUInt;
        let cnt = runcnt & GX_DT_DELTA_RUN_COUNT_MASK;

        bytes_used += 1;

        let mut j: FtUInt = 0;
        if runcnt & GX_DT_DELTAS_ARE_ZERO != 0 {
            /* `cnt` + 1 zeroes get added */
            while j <= cnt && i < delta_cnt {
                deltas[i as usize] = 0;
                i += 1;
                j += 1;
            }
        } else if runcnt & GX_DT_DELTAS_ARE_WORDS != 0 {
            /* `cnt` + 1 shorts from the stack */
            bytes_used += 2 * (cnt + 1);
            if bytes_used as FtULong > size {
                return None;
            }

            while j <= cnt && i < delta_cnt {
                deltas[i as usize] = ft_int_to_fixed(stream.get_short() as FtLong);
                i += 1;
                j += 1;
            }
        } else {
            /* `cnt` + 1 signed bytes from the stack */
            bytes_used += cnt + 1;
            if bytes_used as FtULong > size {
                return None;
            }

            while j <= cnt && i < delta_cnt {
                deltas[i as usize] = ft_int_to_fixed(stream.get_char() as FtLong);
                i += 1;
                j += 1;
            }
        }

        if j <= cnt {
            return None;
        }
    }

    if i < delta_cnt {
        return None;
    }

    Some(deltas)
}

/// `ft_var_load_avar`: Parse the `avar' table if present.  It need not
/// be, so we return nothing.
fn ft_var_load_avar(face: &mut TtFaceRec, stream: &mut FtStreamRec) {
    let blend = face.blend.as_mut().expect("no blend");
    blend.avar_loaded = true;

    let table_len = match tt_face_goto_table(face, TTAG_avar as FtULong, stream) {
        Ok(l) => l,
        Err(_) => return,
    };

    /* !TT_CONFIG_OPTION_NO_BORING_EXPANSION */
    let table_offset = stream.pos();

    if stream.enter_frame(table_len).is_err() {
        return;
    }

    (|| {
        let version = stream.get_long() as FtLong;
        let axis_count = stream.get_long() as FtLong;

        if version != 0x00010000
            /* !TT_CONFIG_OPTION_NO_BORING_EXPANSION */
            && version != 0x00020000
        {
            return;
        }

        let num_axis = face
            .blend
            .as_ref()
            .unwrap()
            .mmvar
            .as_ref()
            .unwrap()
            .num_axis;
        if axis_count != num_axis as FtLong {
            return;
        }

        face.blend.as_mut().unwrap().avar_table = Some(Box::default());

        let mut segments: Vec<GxAVarSegmentRec> = match ft_new_array(axis_count) {
            Ok(s) => s,
            Err(_) => return,
        };

        for i in 0..axis_count as usize {
            let segment = &mut segments[i];
            segment.pairCount = stream.get_ushort();
            if segment.pairCount as FtULong * 4 > table_len {
                /* Failure.  Free everything we have done so far.  We must do */
                /* it right now since loading the `avar' table is optional.   */
                return;
            }
            segment.correspondence = match ft_new_array(segment.pairCount as FtLong) {
                Ok(c) => c,
                Err(_) => return,
            };

            for j in 0..segment.pairCount as usize {
                segment.correspondence[j].fromCoord = ft_fdot14_to_fixed(stream.get_short());
                segment.correspondence[j].toCoord = ft_fdot14_to_fixed(stream.get_short());
            }
        }

        face.blend
            .as_mut()
            .unwrap()
            .avar_table
            .as_mut()
            .unwrap()
            .avar_segment = Some(segments);

        /* !TT_CONFIG_OPTION_NO_BORING_EXPANSION */
        if version < 0x00020000 {
            return;
        }

        let axis_map_offset = stream.get_ulong() as FtULong;
        let store_offset = stream.get_ulong() as FtULong;

        /* (the frame stays entered while the item variation store and the */
        /* delta set index mapping are read, as in C)                       */
        let mut table = face.blend.as_mut().unwrap().avar_table.take().unwrap();

        let r = (|| -> FtResult<()> {
            if store_offset != 0 {
                tt_var_load_item_variation_store_in(
                    face,
                    stream,
                    table_offset + store_offset,
                    &mut table.itemStore,
                )?;
            }

            if axis_map_offset != 0 {
                tt_var_load_delta_set_index_mapping_in(
                    stream,
                    table_offset + axis_map_offset,
                    &mut table.axisMap,
                    &table.itemStore,
                    table_len,
                )?;
            }
            Ok(())
        })();
        let _ = r;

        face.blend.as_mut().unwrap().avar_table = Some(table);
    })();

    /* Exit: */
    stream.exit_frame();
}

/// `tt_var_load_item_variation_store`
pub fn tt_var_load_item_variation_store(
    face: &TtFaceRec,
    stream: &mut FtStreamRec,
    offset: FtULong,
    item_store: &mut GxItemVarStoreRec,
) -> FtResult<()> {
    tt_var_load_item_variation_store_in(face, stream, offset, item_store)
}

fn tt_var_load_item_variation_store_in(
    face: &TtFaceRec,
    stream: &mut FtStreamRec,
    offset: FtULong,
    item_store: &mut GxItemVarStoreRec,
) -> FtResult<()> {
    let blend = face.blend.as_ref().expect("no blend");

    /* (the stream may be inside a frame, as in `ft_var_load_avar'; the */
    /* reads then happen through the frame-independent stream methods)  */
    stream.seek(offset)?;
    let format = stream.read_ushort()?;

    if format != 1 {
        return Err(FT_ERR_INVALID_TABLE);
    }

    /* read top level fields */
    let region_offset = stream.read_ulong()? as FtULong;
    let data_count = stream.read_ushort()? as FtUInt;

    /* we need at least one entry in `itemStore->varData' */
    if data_count == 0 {
        return Err(FT_ERR_INVALID_TABLE);
    }

    /* make temporary copy of item variation data offsets; */
    /* we will parse region list first, then come back     */
    let mut data_offset_array: Vec<FtULong> = ft_new_array(data_count as FtLong)?;

    for d in data_offset_array.iter_mut() {
        *d = stream.read_ulong()? as FtULong;
    }

    /* parse array of region records (region list) */
    stream.seek(offset + region_offset)?;

    let axis_count = stream.read_ushort()?;
    let region_count = stream.read_ushort()? as FtUInt;

    if axis_count as FtLong != blend.mmvar.as_ref().map_or(0, |m| m.num_axis) as FtLong {
        return Err(FT_ERR_INVALID_TABLE);
    }
    item_store.axisCount = axis_count;

    /* new constraint in OpenType 1.8.4 */
    if region_count >= 32768 {
        return Err(FT_ERR_INVALID_TABLE);
    }

    item_store.varRegionList = ft_new_array(region_count as FtLong)?;
    item_store.regionCount = region_count;

    for i in 0..item_store.regionCount as usize {
        item_store.varRegionList[i].axisList = ft_new_array(axis_count as FtLong)?;

        for j in 0..item_store.axisCount as usize {
            let start = stream.read_short()?;
            let peak = stream.read_short()?;
            let end = stream.read_short()?;

            let axis_coords = &mut item_store.varRegionList[i].axisList[j];
            axis_coords.startCoord = ft_fdot14_to_fixed(start);
            axis_coords.peakCoord = ft_fdot14_to_fixed(peak);
            axis_coords.endCoord = ft_fdot14_to_fixed(end);
        }
    }

    /* end of region list parse */

    /* use dataOffsetArray now to parse varData items */
    item_store.varData = ft_new_array(data_count as FtLong)?;
    item_store.dataCount = data_count;

    for i in 0..data_count as usize {
        stream.seek(offset + data_offset_array[i])?;

        let item_count = stream.read_ushort()? as FtUInt;
        let mut word_delta_count = stream.read_ushort()?;
        let region_idx_count = stream.read_ushort()? as FtUInt;

        let long_words = (word_delta_count & 0x8000) != 0;
        word_delta_count &= 0x7FFF;

        /* check some data consistency */
        if word_delta_count as FtUInt > region_idx_count {
            return Err(FT_ERR_INVALID_TABLE);
        }

        if region_idx_count > item_store.regionCount {
            return Err(FT_ERR_INVALID_TABLE);
        }

        /* parse region indices */
        let region_count = item_store.regionCount;
        let var_data = &mut item_store.varData[i];
        var_data.regionIndices = ft_new_array(region_idx_count as FtLong)?;
        var_data.regionIdxCount = region_idx_count;
        var_data.wordDeltaCount = word_delta_count;
        var_data.longWords = long_words;

        for j in 0..var_data.regionIdxCount as usize {
            var_data.regionIndices[j] = stream.read_ushort()? as FtUInt;

            if var_data.regionIndices[j] >= region_count {
                return Err(FT_ERR_INVALID_TABLE);
            }
        }

        let mut per_region_size = word_delta_count as FtUInt + region_idx_count;
        if long_words {
            per_region_size *= 2;
        }

        var_data.deltaSet = ft_new_array((per_region_size * item_count) as FtLong)?;
        if stream.read(&mut var_data.deltaSet).is_err() {
            return Err(FT_ERR_INVALID_TABLE);
        }

        var_data.itemCount = item_count;
    }

    Ok(())
}

/// `tt_var_load_delta_set_index_mapping`
pub fn tt_var_load_delta_set_index_mapping(
    _face: &TtFaceRec,
    stream: &mut FtStreamRec,
    offset: FtULong,
    map: &mut GxDeltaSetIdxMapRec,
    item_store: &GxItemVarStoreRec,
    table_len: FtULong,
) -> FtResult<()> {
    tt_var_load_delta_set_index_mapping_in(stream, offset, map, item_store, table_len)
}

fn tt_var_load_delta_set_index_mapping_in(
    stream: &mut FtStreamRec,
    offset: FtULong,
    map: &mut GxDeltaSetIdxMapRec,
    item_store: &GxItemVarStoreRec,
    table_len: FtULong,
) -> FtResult<()> {
    stream.seek(offset)?;
    let format = stream.read_byte()?;
    let entry_format = stream.read_byte()?;

    if format == 0 {
        map.mapCount = stream.read_ushort()? as FtULong;
    } else if format == 1 {
        /* new in OpenType 1.9 */
        map.mapCount = stream.read_ulong()? as FtULong;
    } else {
        return Err(FT_ERR_INVALID_TABLE);
    }

    if entry_format & 0xC0 != 0 {
        return Err(FT_ERR_INVALID_TABLE);
    }

    /* bytes per entry: 1, 2, 3, or 4 */
    let entry_size = (((entry_format & 0x30) >> 4) + 1) as FtUInt;
    let inner_bit_count = ((entry_format & 0x0F) + 1) as FtUInt;
    let inner_index_mask: FtUInt = (1u32 << inner_bit_count).wrapping_sub(1);

    /* rough sanity check */
    if map.mapCount * entry_size as FtULong > table_len {
        return Err(FT_ERR_INVALID_TABLE);
    }

    map.innerIndex = ft_new_array(map.mapCount as FtLong)?;
    map.outerIndex = ft_new_array(map.mapCount as FtLong)?;

    for i in 0..map.mapCount as usize {
        let mut map_data: FtUInt = 0;

        /* read map data one unsigned byte at a time, big endian */
        for _ in 0..entry_size {
            let data = stream.read_byte()?;
            map_data = (map_data << 8) | data as FtUInt;
        }

        /* new in OpenType 1.8.4 */
        if map_data == 0xFFFFFFFF {
            /* no variation data for this item */
            map.outerIndex[i] = 0xFFFF;
            map.innerIndex[i] = 0xFFFF;

            continue;
        }

        let outer_index = map_data >> inner_bit_count;

        if outer_index >= item_store.dataCount {
            return Err(FT_ERR_INVALID_TABLE);
        }

        map.outerIndex[i] = outer_index;

        let inner_index = map_data & inner_index_mask;

        if inner_index >= item_store.varData[outer_index as usize].itemCount {
            return Err(FT_ERR_INVALID_TABLE);
        }

        map.innerIndex[i] = inner_index;
    }

    Ok(())
}

/// `ft_var_load_hvvar`: If `vertical' is zero, parse the `HVAR' table and
/// set `blend->hvar_loaded' to TRUE.  On success, `blend->hvar_checked'
/// is set to TRUE.
///
/// If `vertical' is not zero, parse the `VVAR' table and set
/// `blend->vvar_loaded' to TRUE.  On success, `blend->vvar_checked' is
/// set to TRUE.
///
/// Some memory may remain allocated on error; it is always freed in
/// `tt_done_blend', however.
fn ft_var_load_hvvar(
    face: &mut TtFaceRec,
    stream: &mut FtStreamRec,
    vertical: bool,
) -> FtResult<()> {
    let r = (|| -> FtResult<()> {
        let tag = if vertical {
            face.blend.as_mut().unwrap().vvar_loaded = true;
            TTAG_VVAR
        } else {
            face.blend.as_mut().unwrap().hvar_loaded = true;
            TTAG_HVAR
        };
        let table_len = tt_face_goto_table(face, tag as FtULong, stream)?;

        let table_offset = stream.pos();

        /* skip minor version */
        let major_version = stream.read_ushort()?;
        stream.skip(2)?;

        if major_version != 1 {
            return Err(FT_ERR_INVALID_TABLE);
        }

        let store_offset = stream.read_ulong()? as FtULong;
        let width_map_offset = stream.read_ulong()? as FtULong;

        let mut table = Box::<GxHVVarTableRec>::default();

        let r = (|| -> FtResult<()> {
            tt_var_load_item_variation_store_in(
                face,
                stream,
                table_offset + store_offset,
                &mut table.itemStore,
            )?;

            if width_map_offset != 0 {
                tt_var_load_delta_set_index_mapping_in(
                    stream,
                    table_offset + width_map_offset,
                    &mut table.widthMap,
                    &table.itemStore,
                    table_len,
                )?;
            }
            Ok(())
        })();

        let blend = face.blend.as_mut().unwrap();
        if vertical {
            blend.vvar_table = Some(table);
        } else {
            blend.hvar_table = Some(table);
        }

        r
    })();

    /* Exit: */
    if r.is_ok() {
        let blend = face.blend.as_mut().unwrap();
        if vertical {
            blend.vvar_checked = true;

            /* FreeType doesn't provide functions to quickly retrieve    */
            /* TSB, BSB, or VORG values; we thus don't have to implement */
            /* support for those three item variation stores.            */

            face.variation_support |= TT_FACE_FLAG_VAR_VADVANCE;
        } else {
            blend.hvar_checked = true;

            /* FreeType doesn't provide functions to quickly retrieve */
            /* LSB or RSB values; we thus don't have to implement     */
            /* support for those two item variation stores.           */

            face.variation_support |= TT_FACE_FLAG_VAR_HADVANCE;
        }
    }

    r
}

/// `tt_var_get_item_delta`
pub fn tt_var_get_item_delta(
    face: &TtFaceRec,
    item_store: &GxItemVarStoreRec,
    outer_index: FtUInt,
    inner_index: FtUInt,
) -> FtItemVarDelta {
    match face
        .blend
        .as_ref()
        .and_then(|b| b.normalizedcoords.as_deref())
    {
        Some(coords) => tt_var_get_item_delta_coords(coords, item_store, outer_index, inner_index),
        None => 0,
    }
}

/// `tt_var_get_item_delta` with the blend's normalized coordinates
/// (`ft_var_to_normalized` temporarily installs other ones).
fn tt_var_get_item_delta_coords(
    normalizedcoords: &[FtFixed],
    item_store: &GxItemVarStoreRec,
    outer_index: FtUInt,
    inner_index: FtUInt,
) -> FtItemVarDelta {
    /* OpenType 1.8.4+: No variation data for this item */
    /* as indices have special value 0xFFFF.            */
    if outer_index == 0xFFFF && inner_index == 0xFFFF {
        return 0;
    }

    /* See pseudo code from `Font Variations Overview' */
    /* in the OpenType specification.                  */

    if outer_index >= item_store.dataCount {
        return 0; /* Out of range. */
    }

    let var_data = &item_store.varData[outer_index as usize];

    if inner_index >= var_data.itemCount {
        return 0; /* Out of range. */
    }

    let n = var_data.regionIdxCount as usize;
    let mut delta_set: Vec<FtItemVarDelta> = Vec::new();
    let mut scalars: Vec<FtFixed> = Vec::new();
    if delta_set.try_reserve_exact(n).is_err() || scalars.try_reserve_exact(n).is_err() {
        return 0;
    }
    delta_set.resize(n, 0);
    scalars.resize(n, 0);

    /* Parse delta set.                                            */
    /*                                                             */
    /* Deltas are (word_delta_count + region_idx_count) bytes each */
    /* if `longWords` isn't set, and twice as much otherwise.      */
    let mut per_region_size = var_data.wordDeltaCount as FtUInt + var_data.regionIdxCount;
    if var_data.longWords {
        per_region_size *= 2;
    }

    let bytes = &var_data.deltaSet[..];
    let mut p = (per_region_size * inner_index) as usize;

    let mut master = 0usize;
    if var_data.longWords {
        while master < var_data.wordDeltaCount as usize {
            delta_set[master] = ft_next_long(bytes, &mut p);
            master += 1;
        }
        while master < n {
            delta_set[master] = ft_next_short(bytes, &mut p) as FtItemVarDelta;
            master += 1;
        }
    } else {
        while master < var_data.wordDeltaCount as usize {
            delta_set[master] = ft_next_short(bytes, &mut p) as FtItemVarDelta;
            master += 1;
        }
        while master < n {
            delta_set[master] = ft_next_char(bytes, &mut p) as FtItemVarDelta;
            master += 1;
        }
    }

    /* outer loop steps through master designs to be blended */
    for master in 0..n {
        let mut scalar: FtFixed = 0x10000;
        let region_index = var_data.regionIndices[master] as usize;

        let axis_list = &item_store.varRegionList[region_index].axisList;

        /* inner loop steps through axes in this region */
        for j in 0..item_store.axisCount as usize {
            let axis = &axis_list[j];
            let nc = normalizedcoords[j];

            /* compute the scalar contribution of this axis; */
            /* ignore invalid ranges                         */
            if axis.startCoord > axis.peakCoord || axis.peakCoord > axis.endCoord {
                continue;
            } else if axis.startCoord < 0 && axis.endCoord > 0 && axis.peakCoord != 0 {
                continue;
            }
            /* peak of 0 means ignore this axis */
            else if axis.peakCoord == 0 {
                continue;
            } else if nc == axis.peakCoord {
                continue;
            }
            /* ignore this region if coords are out of range */
            else if nc <= axis.startCoord || nc >= axis.endCoord {
                scalar = 0;
                break;
            }
            /* cumulative product of all the axis scalars */
            else if nc < axis.peakCoord {
                scalar = ft_mul_div(
                    scalar,
                    nc - axis.startCoord,
                    axis.peakCoord - axis.startCoord,
                );
            } else {
                scalar = ft_mul_div(scalar, axis.endCoord - nc, axis.endCoord - axis.peakCoord);
            }
        } /* per-axis loop */

        scalars[master] = scalar;
    } /* per-region loop */

    /* Compute the scaled delta for this region.
     *
     * From: https://docs.microsoft.com/en-us/typography/opentype/spec/otvarcommonformats#item-variation-store-header-and-item-variation-data-subtables:
     *
     *   `Fixed` is a 32-bit (16.16) type and, in the general case, requires
     *   32-bit deltas.  As described above, the `DeltaSet` record can
     *   accommodate deltas that are, logically, either 16-bit or 32-bit.
     *   When scaled deltas are applied to `Fixed` values, the `Fixed` value
     *   is treated like a 32-bit integer.
     *
     * `FT_MulAddFix` internally uses 64-bit precision; it thus can handle
     * deltas ranging from small 8-bit to large 32-bit values that are
     * applied to 16.16 `FT_Fixed` / OpenType `Fixed` values.
     */
    ft_mul_add_fix(&scalars, &delta_set, n)
}

/// `tt_hvadvance_adjust`: Apply `HVAR' advance width or `VVAR' advance
/// height adjustment of a given glyph.
fn tt_hvadvance_adjust(
    face: &mut TtFaceRec,
    stream: &mut FtStreamRec,
    gindex: FtUInt,
    avalue: &mut FtInt,
    vertical: bool,
) -> FtResult<()> {
    if !face.doblend || face.blend.is_none() {
        return Ok(());
    }

    if vertical {
        if !face.blend.as_ref().unwrap().vvar_loaded {
            /* initialize vvar table */
            let e = ft_var_load_hvvar(face, stream, true).err().unwrap_or(0);
            face.blend.as_mut().unwrap().vvar_error = e;
        }

        let blend = face.blend.as_ref().unwrap();
        if !blend.vvar_checked {
            return if blend.vvar_error != 0 {
                Err(blend.vvar_error)
            } else {
                Ok(())
            };
        }
    } else {
        if !face.blend.as_ref().unwrap().hvar_loaded {
            /* initialize hvar table */
            let e = ft_var_load_hvvar(face, stream, false).err().unwrap_or(0);
            face.blend.as_mut().unwrap().hvar_error = e;
        }

        let blend = face.blend.as_ref().unwrap();
        if !blend.hvar_checked {
            return if blend.hvar_error != 0 {
                Err(blend.hvar_error)
            } else {
                Ok(())
            };
        }
    }

    let blend = face.blend.as_ref().unwrap();
    let table = if vertical {
        blend.vvar_table.as_ref().unwrap()
    } else {
        blend.hvar_table.as_ref().unwrap()
    };

    /* advance width or height adjustments are always present in an */
    /* `HVAR' or `VVAR' table; no need to test for this capability  */

    let (outer_index, inner_index);
    if !table.widthMap.innerIndex.is_empty() {
        let mut idx = gindex as FtULong;

        if idx >= table.widthMap.mapCount {
            idx = table.widthMap.mapCount - 1;
        }

        /* trust that HVAR parser has checked indices */
        outer_index = table.widthMap.outerIndex[idx as usize];
        inner_index = table.widthMap.innerIndex[idx as usize];
    } else {
        /* no widthMap data */
        outer_index = 0;
        inner_index = gindex;
    }

    let delta = tt_var_get_item_delta(face, &table.itemStore, outer_index, inner_index);

    if delta != 0 {
        *avalue = add_int(*avalue, delta);
    }

    Ok(())
}

/// `tt_hadvance_adjust`
pub fn tt_hadvance_adjust(
    face: &mut TtFaceRec,
    stream: &mut FtStreamRec,
    gindex: FtUInt,
    avalue: &mut FtInt,
) -> FtResult<()> {
    tt_hvadvance_adjust(face, stream, gindex, avalue, false)
}

/// `tt_vadvance_adjust`
pub fn tt_vadvance_adjust(
    face: &mut TtFaceRec,
    stream: &mut FtStreamRec,
    gindex: FtUInt,
    avalue: &mut FtInt,
) -> FtResult<()> {
    tt_hvadvance_adjust(face, stream, gindex, avalue, true)
}

const GX_VALUE_SIZE: FtULong = 8;

/* all values are FT_Short or FT_UShort entities; */
/* we treat them consistently as FT_Short         */

/// `ft_var_get_value_pointer`: reads (with `set` being `None`) or writes
/// the value an MVAR tag refers to; `None` is C's NULL pointer.
fn ft_var_value(face: &mut TtFaceRec, mvar_tag: FtULong, set: Option<FtShort>) -> Option<FtShort> {
    macro_rules! value_case {
        ($field:expr) => {{
            let p = &mut $field;
            if let Some(v) = set {
                *p = v as _;
            }
            Some(*p as FtShort)
        }};
    }

    macro_rules! gasp_case {
        ($idx:expr) => {{
            if ($idx as FtInt) < face.gasp.numRanges as FtInt - 1 {
                value_case!(face.gasp.gaspRanges[$idx].maxPPEM)
            } else {
                None
            }
        }};
    }

    match mvar_tag {
        MVAR_TAG_GASP_0 => gasp_case!(0),
        MVAR_TAG_GASP_1 => gasp_case!(1),
        MVAR_TAG_GASP_2 => gasp_case!(2),
        MVAR_TAG_GASP_3 => gasp_case!(3),
        MVAR_TAG_GASP_4 => gasp_case!(4),
        MVAR_TAG_GASP_5 => gasp_case!(5),
        MVAR_TAG_GASP_6 => gasp_case!(6),
        MVAR_TAG_GASP_7 => gasp_case!(7),
        MVAR_TAG_GASP_8 => gasp_case!(8),
        MVAR_TAG_GASP_9 => gasp_case!(9),

        MVAR_TAG_CPHT => value_case!(face.os2.sCapHeight),
        MVAR_TAG_HASC => value_case!(face.os2.sTypoAscender),
        MVAR_TAG_HCLA => value_case!(face.os2.usWinAscent),
        MVAR_TAG_HCLD => value_case!(face.os2.usWinDescent),
        MVAR_TAG_HCOF => value_case!(face.horizontal.caret_Offset),
        MVAR_TAG_HCRN => value_case!(face.horizontal.caret_Slope_Run),
        MVAR_TAG_HCRS => value_case!(face.horizontal.caret_Slope_Rise),
        MVAR_TAG_HDSC => value_case!(face.os2.sTypoDescender),
        MVAR_TAG_HLGP => value_case!(face.os2.sTypoLineGap),
        MVAR_TAG_SBXO => value_case!(face.os2.ySubscriptXOffset),
        MVAR_TAG_SBXS => value_case!(face.os2.ySubscriptXSize),
        MVAR_TAG_SBYO => value_case!(face.os2.ySubscriptYOffset),
        MVAR_TAG_SBYS => value_case!(face.os2.ySubscriptYSize),
        MVAR_TAG_SPXO => value_case!(face.os2.ySuperscriptXOffset),
        MVAR_TAG_SPXS => value_case!(face.os2.ySuperscriptXSize),
        MVAR_TAG_SPYO => value_case!(face.os2.ySuperscriptYOffset),
        MVAR_TAG_SPYS => value_case!(face.os2.ySuperscriptYSize),
        MVAR_TAG_STRO => value_case!(face.os2.yStrikeoutPosition),
        MVAR_TAG_STRS => value_case!(face.os2.yStrikeoutSize),
        MVAR_TAG_UNDO => value_case!(face.postscript.underlinePosition),
        MVAR_TAG_UNDS => value_case!(face.postscript.underlineThickness),
        MVAR_TAG_VASC => value_case!(face.vertical.Ascender),
        MVAR_TAG_VCOF => value_case!(face.vertical.caret_Offset),
        MVAR_TAG_VCRN => value_case!(face.vertical.caret_Slope_Run),
        MVAR_TAG_VCRS => value_case!(face.vertical.caret_Slope_Rise),
        MVAR_TAG_VDSC => value_case!(face.vertical.Descender),
        MVAR_TAG_VLGP => value_case!(face.vertical.Line_Gap),
        MVAR_TAG_XHGT => value_case!(face.os2.sxHeight),

        _ => {
            /* ignore unknown tag */
            None
        }
    }
}

/// `ft_var_load_mvar`: Parse the `MVAR' table.
///
/// Some memory may remain allocated on error; it is always freed in
/// `tt_done_blend', however.
fn ft_var_load_mvar(face: &mut TtFaceRec, stream: &mut FtStreamRec) {
    let _table_len = match tt_face_goto_table(face, TTAG_MVAR as FtULong, stream) {
        Ok(l) => l,
        Err(_) => return,
    };

    let table_offset = stream.pos();

    /* skip minor version */
    let major_version = match stream.read_ushort() {
        Ok(v) => v,
        Err(_) => return,
    };
    if stream.skip(2).is_err() {
        return;
    }

    if major_version != 1 {
        return;
    }

    let mut mvar = Box::<GxMVarTableRec>::default();

    let r = (|| -> FtResult<FtULong> {
        /* skip reserved entry and value record size */
        stream.skip(4)?;
        mvar.valueCount = stream.read_ushort()?;
        let store_offset = stream.read_ushort()? as FtULong;

        let records_offset = stream.pos();

        tt_var_load_item_variation_store_in(
            face,
            stream,
            table_offset + store_offset,
            &mut mvar.itemStore,
        )?;

        mvar.values = ft_new_array(mvar.valueCount as FtLong)?;

        stream.seek(records_offset)?;
        stream.enter_frame(mvar.valueCount as FtULong * GX_VALUE_SIZE)?;

        Ok(records_offset)
    })();

    if r.is_err() {
        face.blend.as_mut().unwrap().mvar_table = Some(mvar);
        return;
    }

    let mut error = Ok(());
    {
        let item_store = &mvar.itemStore;
        for value in mvar.values.iter_mut() {
            value.tag = stream.get_ulong() as FtULong;
            value.outerIndex = stream.get_ushort();
            value.innerIndex = stream.get_ushort();

            /* new in OpenType 1.8.4 */
            if value.outerIndex == 0xFFFF && value.innerIndex == 0xFFFF {
                /* no variation data for this item */
                continue;
            }

            if value.outerIndex as FtUInt >= item_store.dataCount
                || value.innerIndex as FtUInt
                    >= item_store.varData[value.outerIndex as usize].itemCount
            {
                error = Err(FT_ERR_INVALID_TABLE);
                break;
            }
        }
    }

    stream.exit_frame();

    if error.is_err() {
        face.blend.as_mut().unwrap().mvar_table = Some(mvar);
        return;
    }

    /* save original values of the data MVAR is going to modify */
    for value in mvar.values.iter_mut() {
        if let Some(p) = ft_var_value(face, value.tag, None) {
            value.unmodified = p;
        }
    }

    face.blend.as_mut().unwrap().mvar_table = Some(mvar);

    face.variation_support |= TT_FACE_FLAG_VAR_MVAR;
}

/// `tt_apply_mvar`: Apply `MVAR' table adjustments.
pub fn tt_apply_mvar(face: &mut TtFaceRec) {
    let mut mvar_hasc_delta: FtShort = 0;
    let mut mvar_hdsc_delta: FtShort = 0;
    let mut mvar_hlgp_delta: FtShort = 0;

    if face.variation_support & TT_FACE_FLAG_VAR_MVAR == 0 {
        return;
    }

    let mvar = face.blend.as_mut().unwrap().mvar_table.take().unwrap();

    for value in mvar.values.iter() {
        let delta = tt_var_get_item_delta(
            face,
            &mvar.itemStore,
            value.outerIndex as FtUInt,
            value.innerIndex as FtUInt,
        );

        if delta != 0 && ft_var_value(face, value.tag, None).is_some() {
            /* since we handle both signed and unsigned values as FT_Short, */
            /* ensure proper overflow arithmetic                            */
            ft_var_value(
                face,
                value.tag,
                Some((value.unmodified as i32 + delta as FtShort as i32) as FtShort),
            );

            /* Treat hasc, hdsc and hlgp specially, see below. */
            if value.tag == MVAR_TAG_HASC {
                mvar_hasc_delta = delta as FtShort;
            } else if value.tag == MVAR_TAG_HDSC {
                mvar_hdsc_delta = delta as FtShort;
            } else if value.tag == MVAR_TAG_HLGP {
                mvar_hlgp_delta = delta as FtShort;
            }
        }
    }

    face.blend.as_mut().unwrap().mvar_table = Some(mvar);

    /* adjust all derived values */
    {
        /*
         * Apply the deltas of hasc, hdsc and hlgp to the FT_Face's ascender,
         * descender and height attributes, no matter how they were originally
         * computed.
         *
         * (Code that ignores those and accesses the font's metrics values
         * directly is already served by the delta application code above.)
         *
         * The MVAR table supports variations for both typo and win metrics.
         * According to Behdad Esfahbod, the thinking of the working group was
         * that no one uses win metrics anymore for setting line metrics (the
         * specification even calls these metrics "horizontal clipping
         * ascent/descent", probably for their role on the Windows platform in
         * computing clipping boxes), and new fonts should use typo metrics, so
         * typo deltas should be applied to whatever sfnt_load_face decided the
         * line metrics should be.
         *
         * Before, the following led to different line metrics between default
         * outline and instances, visible when e.g. the default outlines were
         * used as the regular face and instances for everything else:
         *
         * 1. sfnt_load_face applied the hhea metrics by default.
         * 2. This code later applied the typo metrics by default, regardless of
         *    whether they were actually changed or the font had the OS/2 table's
         *    fsSelection's bit 7 (USE_TYPO_METRICS) set.
         */
        let root = &mut face.root;
        let current_line_gap =
            (root.height as i32 - root.ascender as i32 + root.descender as i32) as FtShort;

        root.ascender = (root.ascender as i32 + mvar_hasc_delta as i32) as FtShort;
        root.descender = (root.descender as i32 + mvar_hdsc_delta as i32) as FtShort;
        root.height = (root.ascender as i32 - root.descender as i32
            + current_line_gap as i32
            + mvar_hlgp_delta as i32) as FtShort;

        root.underline_position = (face.postscript.underlinePosition as i32
            - face.postscript.underlineThickness as i32 / 2)
            as FtShort;
        root.underline_thickness = face.postscript.underlineThickness;

        /* iterate over all FT_Size objects and call `var->size_reset' */
        /* to propagate the metrics changes                            */
        if face.root.has_slot_and_size {
            let _ = tt_size_reset_height(face);
        }
    }
}

/// `GX_GVar_Head`
#[derive(Debug, Clone, Copy, Default)]
struct GxGVarHead {
    version: FtLong,
    axis_count: FtUShort,
    global_coord_count: FtUShort,
    offset_to_coord: FtULong,
    glyph_count: FtUShort,
    flags: FtUShort,
    offset_to_data: FtULong,
}

/// `ft_var_load_gvar`: Parse the `gvar' table if present.  If `fvar' is
/// there, `gvar' had better be there too.
fn ft_var_load_gvar(face: &mut TtFaceRec, stream: &mut FtStreamRec) -> FtResult<()> {
    let table_len = tt_face_goto_table(face, TTAG_gvar as FtULong, stream)?;

    let gvar_start = stream.pos();

    stream.enter_frame(20)?;
    let gvar_head = GxGVarHead {
        version: stream.get_long() as FtLong,
        axis_count: stream.get_ushort(),
        global_coord_count: stream.get_ushort(),
        offset_to_coord: stream.get_ulong() as FtULong,
        glyph_count: stream.get_ushort(),
        flags: stream.get_ushort(),
        offset_to_data: stream.get_ulong() as FtULong,
    };
    stream.exit_frame();

    if gvar_head.version != 0x00010000 {
        return Err(FT_ERR_INVALID_TABLE);
    }

    let blend = face.blend.as_mut().unwrap();
    if gvar_head.axis_count != blend.mmvar.as_ref().unwrap().num_axis as FtUShort {
        return Err(FT_ERR_INVALID_TABLE);
    }

    /* rough sanity check, ignoring offsets */
    if gvar_head.global_coord_count as FtULong * gvar_head.axis_count as FtULong > table_len / 2 {
        return Err(FT_ERR_INVALID_TABLE);
    }

    /* offsets can be either 2 or 4 bytes                  */
    /* (one more offset than glyphs, to mark size of last) */
    let offsets_len =
        (gvar_head.glyph_count as FtULong + 1) * if gvar_head.flags & 1 != 0 { 4 } else { 2 };

    /* rough sanity check */
    if offsets_len > table_len {
        return Err(FT_ERR_INVALID_TABLE);
    }

    blend.gvar_size = table_len;
    let offset_to_data = gvar_start + gvar_head.offset_to_data;

    stream.enter_frame(offsets_len)?;

    let fail = |blend: &mut GxBlendRec| {
        /* Fail: */
        blend.glyphoffsets = None;
        blend.gv_glyphcnt = 0;
    };

    /* offsets (one more offset than glyphs, to mark size of last) */
    let mut glyphoffsets: Vec<FtULong> = match ft_new_array(gvar_head.glyph_count as FtLong + 1) {
        Ok(g) => g,
        Err(e) => {
            /* Fail2: */
            stream.exit_frame();
            fail(blend);
            return Err(e);
        }
    };

    {
        let limit = gvar_start + table_len;
        let mut max_offset: FtULong = 0;

        for i in 0..=gvar_head.glyph_count as usize {
            glyphoffsets[i] = if gvar_head.flags & 1 != 0 {
                offset_to_data + stream.get_ulong() as FtULong
            } else {
                offset_to_data + stream.get_ushort() as FtULong * 2
            };

            if max_offset <= glyphoffsets[i] {
                max_offset = glyphoffsets[i];
            } else {
                glyphoffsets[i] = max_offset;
            }

            /* use `<', not `<=' */
            if limit < glyphoffsets[i] {
                glyphoffsets[i] = limit;
            }
        }
    }

    blend.glyphoffsets = Some(glyphoffsets);
    blend.gv_glyphcnt = gvar_head.glyph_count as FtUInt;

    stream.exit_frame();

    if gvar_head.global_coord_count != 0 {
        if stream.seek(gvar_start + gvar_head.offset_to_coord).is_err()
            || stream
                .enter_frame(
                    gvar_head.global_coord_count as FtULong * gvar_head.axis_count as FtULong * 2,
                )
                .is_err()
        {
            /* Fail: (with the error of the seek or frame) */
            let e = match stream.seek(gvar_start + gvar_head.offset_to_coord) {
                Err(e) => e,
                Ok(()) => stream
                    .enter_frame(
                        gvar_head.global_coord_count as FtULong
                            * gvar_head.axis_count as FtULong
                            * 2,
                    )
                    .err()
                    .unwrap_or(FT_ERR_INVALID_STREAM_OPERATION),
            };
            fail(blend);
            return Err(e);
        }

        let n = gvar_head.axis_count as FtLong * gvar_head.global_coord_count as FtLong;
        let mut tuplecoords: Vec<FtFixed> = match ft_new_array(n) {
            Ok(t) => t,
            Err(e) => {
                /* Fail2: */
                stream.exit_frame();
                fail(blend);
                return Err(e);
            }
        };

        for i in 0..gvar_head.global_coord_count as usize {
            for j in 0..gvar_head.axis_count as usize {
                tuplecoords[i * gvar_head.axis_count as usize + j] =
                    ft_fdot14_to_fixed(stream.get_short());
            }
        }

        blend.tuplecoords = Some(tuplecoords);
        blend.tuplecount = gvar_head.global_coord_count as FtUInt;

        stream.exit_frame();
    }

    Ok(())
}

/// `ft_var_apply_tuple`: Figure out whether a given tuple (design) applies
/// to the current blend, and if so, what is the scaling factor.
fn ft_var_apply_tuple(
    blend: &GxBlendRec,
    tuple_index: FtUShort,
    tuple_coords: &[FtFixed],
    im_start_coords: &[FtFixed],
    im_end_coords: &[FtFixed],
) -> FtFixed {
    let mut apply: FtFixed = 0x10000;
    let nc = blend.normalizedcoords.as_deref().unwrap_or(&[]);

    for i in 0..blend.num_axis as usize {
        /* It's not clear why (for intermediate tuples) we don't need     */
        /* to check against start/end -- the documentation says we don't. */
        /* Similarly, it's unclear why we don't need to scale along the   */
        /* axis.                                                          */

        if tuple_coords[i] == 0 {
            continue;
        }

        if nc[i] == 0 {
            apply = 0;
            break;
        }

        if nc[i] == tuple_coords[i] {
            /* `apply' does not change */
            continue;
        }

        if (tuple_index as FtUInt & GX_TI_INTERMEDIATE_TUPLE) == 0 {
            /* not an intermediate tuple */

            if nc[i] < std::cmp::min(0, tuple_coords[i])
                || nc[i] > std::cmp::max(0, tuple_coords[i])
            {
                apply = 0;
                break;
            }

            apply = ft_mul_div(apply, nc[i], tuple_coords[i]);
        } else {
            /* intermediate tuple */

            if nc[i] <= im_start_coords[i] || nc[i] >= im_end_coords[i] {
                apply = 0;
                break;
            }

            if nc[i] < tuple_coords[i] {
                apply = ft_mul_div(
                    apply,
                    nc[i] - im_start_coords[i],
                    tuple_coords[i] - im_start_coords[i],
                );
            } else {
                apply = ft_mul_div(
                    apply,
                    im_end_coords[i] - nc[i],
                    im_end_coords[i] - tuple_coords[i],
                );
            }
        }
    }

    apply
}

/// `ft_var_to_normalized`: convert from design coordinates to normalized
/// coordinates
fn ft_var_to_normalized(
    face: &TtFaceRec,
    num_coords: FtUInt,
    coords: &[FtFixed],
    normalized: &mut [FtFixed],
) {
    let blend = face.blend.as_ref().unwrap();
    let mmvar = blend.mmvar.as_ref().unwrap();

    let mut num_coords = num_coords;
    if num_coords > mmvar.num_axis {
        num_coords = mmvar.num_axis;
    }

    /* Axis normalization is a two-stage process.  First we normalize */
    /* based on the [min,def,max] values for the axis to be [-1,0,1]. */
    /* Then, if there's an `avar' table, we renormalize this range.   */

    let mut i = 0usize;
    while i < num_coords as usize {
        let a = &mmvar.axis[i];
        let coord = coords[i];

        if coord > a.def {
            normalized[i] = if coord >= a.maximum {
                0x10000
            } else {
                ft_div_fix(sub_long(coord, a.def), sub_long(a.maximum, a.def))
            };
        } else if coord < a.def {
            normalized[i] = if coord <= a.minimum {
                -0x10000
            } else {
                ft_div_fix(sub_long(coord, a.def), sub_long(a.def, a.minimum))
            };
        } else {
            normalized[i] = 0;
        }
        i += 1;
    }

    while i < mmvar.num_axis as usize {
        normalized[i] = 0;
        i += 1;
    }

    if let Some(table) = blend.avar_table.as_ref() {
        if let Some(segments) = table.avar_segment.as_ref() {
            for i in 0..mmvar.num_axis as usize {
                let av = &segments[i];
                for j in 1..av.pairCount as usize {
                    if normalized[i] < av.correspondence[j].fromCoord {
                        normalized[i] = ft_mul_div(
                            normalized[i] - av.correspondence[j - 1].fromCoord,
                            av.correspondence[j].toCoord - av.correspondence[j - 1].toCoord,
                            av.correspondence[j].fromCoord - av.correspondence[j - 1].fromCoord,
                        ) + av.correspondence[j - 1].toCoord;
                        break;
                    }
                }
            }
        }

        if !table.itemStore.varData.is_empty() {
            let mut new_normalized: Vec<FtFixed> = match ft_new_array(mmvar.num_axis as FtLong) {
                Ok(v) => v,
                Err(_) => return,
            };

            /* Install our half-normalized coordinates for the next */
            /* Item Variation Store to work with.                   */
            /* (they are passed to `tt_var_get_item_delta_coords') */
            for i in 0..mmvar.num_axis as usize {
                let mut v = normalized[i];
                let mut inner_index = i as FtUInt;
                let mut outer_index: FtUInt = 0;

                if !table.axisMap.innerIndex.is_empty() {
                    let mut idx = i as FtULong;

                    if idx >= table.axisMap.mapCount {
                        idx = table.axisMap.mapCount - 1;
                    }

                    outer_index = table.axisMap.outerIndex[idx as usize];
                    inner_index = table.axisMap.innerIndex[idx as usize];
                }

                let delta = tt_var_get_item_delta_coords(
                    normalized,
                    &table.itemStore,
                    outer_index,
                    inner_index,
                );

                /* Convert to 16.16 format before adding. */
                v += delta.wrapping_mul(4) as FtFixed;

                /* Clamp value range. */
                v = if v >= 0x10000 { 0x10000 } else { v };
                v = if v <= -0x10000 { -0x10000 } else { v };

                new_normalized[i] = v;
            }

            normalized[..mmvar.num_axis as usize].copy_from_slice(&new_normalized);
        }
    }
}

/// `ft_var_to_design`: convert from normalized coordinates to design
/// coordinates
fn ft_var_to_design(
    blend: &GxBlendRec,
    num_coords: FtUInt,
    coords: &[FtFixed],
    design: &mut [FtFixed],
) {
    let mut nc = num_coords;
    if num_coords > blend.num_axis {
        nc = blend.num_axis;
    }

    let mut i = 0usize;
    while i < nc as usize {
        design[i] = coords[i];
        i += 1;
    }

    while i < num_coords as usize {
        design[i] = 0;
        i += 1;
    }

    if let Some(segments) = blend
        .avar_table
        .as_ref()
        .and_then(|t| t.avar_segment.as_ref())
    {
        for i in 0..nc as usize {
            let av = &segments[i];
            for j in 1..av.pairCount as usize {
                if design[i] < av.correspondence[j].toCoord {
                    design[i] = ft_mul_div(
                        design[i] - av.correspondence[j - 1].toCoord,
                        av.correspondence[j].fromCoord - av.correspondence[j - 1].fromCoord,
                        av.correspondence[j].toCoord - av.correspondence[j - 1].toCoord,
                    ) + av.correspondence[j - 1].fromCoord;
                    break;
                }
            }
        }
    }

    let mmvar = blend.mmvar.as_ref().unwrap();
    for i in 0..nc as usize {
        let a = &mmvar.axis[i];
        if design[i] < 0 {
            design[i] = a.def + ft_mul_fix(design[i], a.def - a.minimum);
        } else if design[i] > 0 {
            design[i] = a.def + ft_mul_fix(design[i], a.maximum - a.def);
        } else {
            design[i] = a.def;
        }
    }
}

/*************************************************************************/
/*************************************************************************/
/*****                                                               *****/
/*****               MULTIPLE MASTERS SERVICE FUNCTIONS              *****/
/*****                                                               *****/
/*************************************************************************/
/*************************************************************************/

/// `GX_FVar_Head`
#[derive(Debug, Clone, Copy, Default)]
struct GxFVarHead {
    version: FtLong,
    offset_to_data: FtUShort,
    axis_count: FtUShort,
    axis_size: FtUShort,
    instance_count: FtUShort,
    instance_size: FtUShort,
}

/// `GX_FVar_Axis`
#[derive(Debug, Clone, Copy, Default)]
struct GxFVarAxis {
    axis_tag: FtULong,
    min_value: FtFixed,
    default_value: FtFixed,
    max_value: FtFixed,
    flags: FtUShort,
    name_id: FtUShort,
}

/// The axis name C's `TT_Get_MM_Var` stores (the four tag bytes, up to a
/// NUL byte).
fn axis_tag_name(tag: FtULong) -> String {
    let bytes = [
        (tag >> 24) as u8,
        ((tag >> 16) & 0xFF) as u8,
        ((tag >> 8) & 0xFF) as u8,
        (tag & 0xFF) as u8,
    ];
    bytes
        .iter()
        .take_while(|&&b| b != 0)
        .map(|&b| b as char)
        .collect()
}

/// `TT_Get_MM_Var`: Check that the font's `fvar' table is valid, parse it,
/// and return those data.  It also loads (and parses) the `MVAR' table,
/// if possible.
///
/// `face` (InOut): The font face.  TT_Get_MM_Var initializes the blend
/// structure.
///
/// Returns the `fvar' data if `want_master` is set (C's `master` output,
/// which can be NULL, which makes this function simply load MM support).
pub fn tt_get_mm_var(
    face: &mut TtFaceRec,
    stream: &mut FtStreamRec,
    want_master: bool,
) -> FtResult<Option<FtMMVar>> {
    let mut fvar_head = GxFVarHead::default();

    /* `num_instances` holds the number of all named instances including  */
    /* the default instance, which might be missing in the table of named */
    /* instances (in 'fvar').  This value is validated in `sfobjs.c` and  */
    /* may be reset to 0 if consistency checks fail.                      */
    let num_instances = (face.root.style_flags as FtUInt) >> 16;

    /* read the font data and set up the internal representation */
    /* if not already done                                       */
    let need_init = face.blend.is_none();

    let num_axes: FtUInt;
    let mut fvar_start: FtULong = 0;
    let mut use_ps_name = false;

    if need_init {
        tt_face_goto_table(face, TTAG_fvar as FtULong, stream)?;

        fvar_start = stream.pos();

        /* the validity of the `fvar' header data was already checked */
        /* in function `sfnt_init_face'                               */
        stream.enter_frame(16)?;
        fvar_head.version = stream.get_long() as FtLong;
        fvar_head.offset_to_data = stream.get_ushort();
        let _ = stream.get_ushort();
        fvar_head.axis_count = stream.get_ushort();
        fvar_head.axis_size = stream.get_ushort();
        fvar_head.instance_count = stream.get_ushort();
        fvar_head.instance_size = stream.get_ushort();
        stream.exit_frame();

        /* If `num_instances` is larger, synthetization of the default  */
        /* instance is required.  If `num_instances` is smaller,        */
        /* however, the value has been reset to 0 in `sfnt_init_face`   */
        /* (in `sfobjs.c`); in this case we have underallocated `mmvar` */
        /* structs.                                                     */
        if num_instances < fvar_head.instance_count as FtUInt {
            return Err(FT_ERR_INVALID_TABLE);
        }

        use_ps_name = fvar_head.instance_size as FtUInt == 6 + 4 * fvar_head.axis_count as FtUInt;

        face.blend = Some(Box::default());

        num_axes = fvar_head.axis_count as FtUInt;
        face.blend.as_mut().unwrap().num_axis = num_axes;
    } else {
        num_axes = face.blend.as_ref().unwrap().num_axis;
    }

    /* prepare storage area for MM data; this cannot overflow   */
    /* 32-bit arithmetic because of the size limits used in the */
    /* `fvar' table validity check in `sfnt_init_face'          */

    /* (the separately allocated vectors of `FT_MM_Var' replace the */
    /* single allocation; `mmvar_len' is computed as in C)          */
    let align_size =
        |n: usize| (n + std::mem::size_of::<usize>() - 1) & !(std::mem::size_of::<usize>() - 1);
    let mmvar_size = align_size(7 * std::mem::size_of::<usize>());
    let axis_flags_size = align_size(num_axes as usize * 2);
    let axis_size = align_size(num_axes as usize * 7 * std::mem::size_of::<usize>());
    let namedstyle_size = align_size(num_instances as usize * 2 * std::mem::size_of::<usize>());
    let next_coords_size =
        align_size(num_instances as usize * num_axes as usize * std::mem::size_of::<FtFixed>());
    let next_name_size = num_axes as usize * 5;

    if need_init {
        face.blend.as_mut().unwrap().mmvar_len = (mmvar_size
            + axis_flags_size
            + axis_size
            + namedstyle_size
            + next_coords_size
            + next_name_size) as FtULong;

        let mut mmvar = Box::<FtMMVar>::default();

        /* set up pointers and offsets into the `mmvar' array; */
        /* the data gets filled in later on                    */

        mmvar.num_axis = num_axes;
        mmvar.num_designs = !0u32; /* meaningless in this context; each glyph */
        /* may have a different number of designs  */
        /* (or tuples, as called by Apple)         */
        mmvar.num_namedstyles = num_instances;

        /* alas, no public field in `FT_Var_Axis' for axis flags */
        mmvar.axis_flags = ft_new_array(num_axes as FtLong)?;
        mmvar.axis = ft_new_array(num_axes as FtLong)?;
        mmvar.namedstyle = ft_new_array(num_instances as FtLong)?;

        for i in 0..num_instances as usize {
            mmvar.namedstyle[i].coords = ft_new_array(num_axes as FtLong)?;
        }

        face.blend.as_mut().unwrap().mmvar = Some(mmvar);

        /* now fill in the data */

        stream.seek(fvar_start + fvar_head.offset_to_data as FtULong)?;

        for i in 0..num_axes as usize {
            stream.enter_frame(20)?;
            let axis_rec = GxFVarAxis {
                axis_tag: stream.get_ulong() as FtULong,
                min_value: stream.get_long() as FtFixed,
                default_value: stream.get_long() as FtFixed,
                max_value: stream.get_long() as FtFixed,
                flags: stream.get_ushort(),
                name_id: stream.get_ushort(),
            };
            stream.exit_frame();

            let mmvar = face.blend.as_mut().unwrap().mmvar.as_mut().unwrap();
            let a = &mut mmvar.axis[i];
            a.tag = axis_rec.axis_tag;
            a.minimum = axis_rec.min_value;
            a.def = axis_rec.default_value;
            a.maximum = axis_rec.max_value;
            a.strid = axis_rec.name_id as FtUInt;

            a.name = axis_tag_name(a.tag);

            mmvar.axis_flags[i] = axis_rec.flags;

            let a = &mut mmvar.axis[i];
            if a.minimum > a.def || a.def > a.maximum {
                a.minimum = a.def;
                a.maximum = a.def;
            }
        }

        /* named instance coordinates are stored as design coordinates; */
        /* we have to convert them to normalized coordinates also       */
        face.blend.as_mut().unwrap().normalized_stylecoords =
            ft_new_array(num_axes as FtLong * num_instances as FtLong)?;

        if fvar_head.instance_count != 0 && !face.blend.as_ref().unwrap().avar_loaded {
            let offset = stream.pos();

            ft_var_load_avar(face, stream);

            stream.seek(offset)?;
        }

        let mut nsc_off = 0usize;
        for i in 0..fvar_head.instance_count as usize {
            /* PostScript names add 2 bytes to the instance record size */
            stream.enter_frame((if use_ps_name { 6 } else { 4 }) + 4 * num_axes as FtULong)?;

            let mut coords: Vec<FtFixed> = std::mem::take(
                &mut face
                    .blend
                    .as_mut()
                    .unwrap()
                    .mmvar
                    .as_mut()
                    .unwrap()
                    .namedstyle[i]
                    .coords,
            );
            let strid = stream.get_ushort() as FtUInt;
            let _ = /* flags = */ stream.get_ushort();

            for c in coords.iter_mut() {
                *c = stream.get_long() as FtFixed;
            }

            /* valid psid values are 6, [256;32767], and 0xFFFF */
            let psid = if use_ps_name {
                stream.get_ushort() as FtUInt
            } else {
                0xFFFF
            };

            let mut nsc = std::mem::take(&mut face.blend.as_mut().unwrap().normalized_stylecoords);
            ft_var_to_normalized(face, num_axes, &coords, &mut nsc[nsc_off..]);
            nsc_off += num_axes as usize;
            face.blend.as_mut().unwrap().normalized_stylecoords = nsc;

            let ns = &mut face
                .blend
                .as_mut()
                .unwrap()
                .mmvar
                .as_mut()
                .unwrap()
                .namedstyle[i];
            ns.strid = strid;
            ns.psid = psid;
            ns.coords = coords;

            stream.exit_frame();
        }

        if num_instances != fvar_head.instance_count as FtUInt {
            let mut dummy1: FtInt = 0;
            let mut dummy2: FtInt = 0;
            let mut strid: FtUInt = !0u32;

            /* The default instance is missing in array the    */
            /* of named instances; try to synthesize an entry. */
            /* If this fails, `default_named_instance` remains */
            /* at value zero, which doesn't do any harm.       */
            let mut found = sfnt_get_name_id(
                face,
                TT_NAME_ID_TYPOGRAPHIC_SUBFAMILY as FtUShort,
                &mut dummy1,
                &mut dummy2,
            );
            if found {
                strid = TT_NAME_ID_TYPOGRAPHIC_SUBFAMILY as FtUInt;
            } else {
                found = sfnt_get_name_id(
                    face,
                    TT_NAME_ID_FONT_SUBFAMILY as FtUShort,
                    &mut dummy1,
                    &mut dummy2,
                );
                if found {
                    strid = TT_NAME_ID_FONT_SUBFAMILY as FtUInt;
                }
            }

            if found {
                found = sfnt_get_name_id(
                    face,
                    TT_NAME_ID_PS_NAME as FtUShort,
                    &mut dummy1,
                    &mut dummy2,
                );
                if found {
                    /* named instance indices start with value 1 */
                    face.var_default_named_instance = num_instances;

                    let mmvar = face.blend.as_mut().unwrap().mmvar.as_mut().unwrap();
                    let defs: Vec<FtFixed> = mmvar.axis.iter().map(|a| a.def).collect();
                    let ns = &mut mmvar.namedstyle[fvar_head.instance_count as usize];
                    ns.strid = strid;
                    ns.psid = TT_NAME_ID_PS_NAME as FtUInt;

                    ns.coords.copy_from_slice(&defs);
                }
            }
        }

        ft_var_load_mvar(face, stream);
    }

    /* fill the output array if requested */
    if want_master {
        let mut mmvar: FtMMVar = (**face.blend.as_ref().unwrap().mmvar.as_ref().unwrap()).clone();

        for a in mmvar.axis.iter_mut() {
            /* standard PostScript names for some standard apple tags */
            if a.tag == TTAG_WGHT {
                a.name = "Weight".into();
            } else if a.tag == TTAG_WDTH {
                a.name = "Width".into();
            } else if a.tag == TTAG_OPSZ {
                a.name = "OpticalSize".into();
            } else if a.tag == TTAG_SLNT {
                a.name = "Slant".into();
            } else if a.tag == TTAG_ITAL {
                a.name = "Italic".into();
            }
        }

        return Ok(Some(mmvar));
    }

    Ok(None)
}

/// The internal error code -1 of the multiple masters functions: success
/// and unchanged axis values.
pub const GX_ERR_NO_CHANGE: FtError = -1;

/// `tt_set_mm_blend`
fn tt_set_mm_blend(
    face: &mut TtFaceRec,
    stream: &mut FtStreamRec,
    num_coords: FtUInt,
    coords: Option<&[FtFixed]>,
    set_design_coords: bool,
) -> FtResult<()> {
    #[derive(PartialEq)]
    enum ManageCvt {
        Retain,
        Modify,
        Load,
    }

    let mut all_design_coords = false;
    let manage_cvt;

    face.doblend = false;

    if face.blend.is_none() {
        tt_get_mm_var(face, stream, false)?;
    }

    let mut num_coords = num_coords;
    let num_axis = face
        .blend
        .as_ref()
        .unwrap()
        .mmvar
        .as_ref()
        .unwrap()
        .num_axis;

    if num_coords > num_axis {
        num_coords = num_axis;
    }

    if let Some(coords) = coords {
        for &c in &coords[..num_coords as usize] {
            if !(-0x00010000..=0x00010000).contains(&c) {
                return Err(FT_ERR_INVALID_ARGUMENT);
            }
        }
    }

    if !face.is_cff2 && face.blend.as_ref().unwrap().glyphoffsets.is_none() {
        /* While a missing 'gvar' table is acceptable, for example for */
        /* fonts that only vary metrics information or 'COLR' v1       */
        /* `PaintVar*` tables, an incorrect SFNT table offset or size  */
        /* for 'gvar', or an inconsistent 'gvar' table is not.         */
        match ft_var_load_gvar(face, stream) {
            Ok(()) => {}
            Err(e) if e == FT_ERR_TABLE_MISSING => {}
            Err(e) => return Err(e),
        }
    }

    let blend = face.blend.as_mut().unwrap();

    if blend.coords.is_none() {
        blend.coords = Some(ft_new_array(num_axis as FtLong)?);

        /* the first time we have to compute all design coordinates */
        all_design_coords = true;
    }

    if blend.normalizedcoords.is_none() {
        blend.normalizedcoords = Some(ft_new_array(num_axis as FtLong)?);

        manage_cvt = ManageCvt::Modify;

        /* If we have not set the blend coordinates before this, then the  */
        /* cvt table will still be what we read from the `cvt ' table and  */
        /* we don't need to reload it.  We may need to change it though... */
    } else {
        let mut have_diff = false;
        let mut mc = ManageCvt::Retain;

        let nc = blend.normalizedcoords.as_ref().unwrap();
        let mut i = 0usize;
        while i < num_coords as usize {
            /* (C reads `coords[i]` here even if `coords` is NULL, */
            /* but `num_coords` is then 0)                         */
            if nc[i] != coords.unwrap()[i] {
                mc = ManageCvt::Load;
                have_diff = true;
                break;
            }
            i += 1;
        }

        if !have_diff {
            if ft_is_named_instance(&face.root) {
                let instance_index = (face.root.face_index as FtUInt) >> 16;

                let n = &blend.normalized_stylecoords
                    [(instance_index as usize - 1) * num_axis as usize..];
                for j in i..num_axis as usize {
                    if nc[j] != n[j] {
                        have_diff = true;
                    }
                }
            } else {
                for &c in &nc[i..num_axis as usize] {
                    if c != 0 {
                        have_diff = true;
                    }
                }
            }
        }

        /* return value -1 indicates `no change' */
        if !have_diff {
            face.doblend = true;

            return Err(GX_ERR_NO_CHANGE);
        }

        while i < num_axis as usize {
            if nc[i] != 0 {
                mc = ManageCvt::Load;
                break;
            }
            i += 1;
        }

        /* If we don't change the blend coords then we don't need to do  */
        /* anything to the cvt table.  It will be correct.  Otherwise we */
        /* no longer have the original cvt (it was modified when we set  */
        /* the blend last time), so we must reload and then modify it.   */
        manage_cvt = mc;
    }

    blend.num_axis = num_axis;
    if let Some(coords) = coords {
        blend.normalizedcoords.as_mut().unwrap()[..num_coords as usize]
            .copy_from_slice(&coords[..num_coords as usize]);
    }

    if set_design_coords {
        let n = if all_design_coords {
            blend.num_axis
        } else {
            num_coords
        };
        let normalized = blend.normalizedcoords.clone().unwrap();
        let mut design = blend.coords.take().unwrap();
        ft_var_to_design(blend, n, &normalized, &mut design);
        blend.coords = Some(design);
    }

    face.doblend = true;

    if !face.cvt.is_empty() {
        match manage_cvt {
            ManageCvt::Load => {
                /* The cvt table has been loaded already; every time we change the */
                /* blend we may need to reload and remodify the cvt table.         */
                face.cvt = Vec::new();

                tt_face_load_cvt(face, stream)?;
            }

            ManageCvt::Modify => {
                /* The original cvt table is in memory.  All we need to do is */
                /* apply the `cvar' table (if any).                           */
                tt_face_vary_cvt(face, stream)?;
            }

            ManageCvt::Retain => { /* The cvt table is correct for this set of coordinates. */ }
        }
    }

    Ok(())
}

/// `TT_Set_MM_Blend`: Set the blend (normalized) coordinates for this
/// instance of the font.  Check that the `gvar' table is reasonable and
/// does some initial preparation.
///
/// `num_coords`: The number of available coordinates.  If it is larger
/// than the number of axes, ignore the excess values.  If it is smaller
/// than the number of axes, use the default value (0) for the remaining
/// axes.
///
/// `coords`: An array of `num_coords', each between [-1,1].
///
/// Returns `Err(GX_ERR_NO_CHANGE)` (-1) for success and unchanged axis
/// values.
pub fn tt_set_mm_blend_pub(
    face: &mut TtFaceRec,
    stream: &mut FtStreamRec,
    num_coords: FtUInt,
    coords: &[FtFixed],
) -> FtResult<()> {
    tt_set_mm_blend(face, stream, num_coords, Some(coords), true)
}

/// `TT_Get_MM_Blend`: Get the blend (normalized) coordinates for this
/// instance of the font.
pub fn tt_get_mm_blend(
    face: &mut TtFaceRec,
    stream: &mut FtStreamRec,
    coords: &mut [FtFixed],
) -> FtResult<()> {
    let num_coords = coords.len() as FtUInt;

    if face.blend.is_none() {
        tt_get_mm_var(face, stream, false)?;
    }

    if face.blend.as_ref().unwrap().coords.is_none() {
        /* select default instance coordinates */
        /* if no instance is selected yet      */
        tt_set_mm_blend(face, stream, 0, None, true)?;
    }

    let blend = face.blend.as_ref().unwrap();
    let mut nc = num_coords;
    if num_coords > blend.num_axis {
        nc = blend.num_axis;
    }

    let mut i = 0usize;
    if face.doblend {
        while i < nc as usize {
            coords[i] = blend.normalizedcoords.as_ref().unwrap()[i];
            i += 1;
        }
    } else {
        while i < nc as usize {
            coords[i] = 0;
            i += 1;
        }
    }

    while i < num_coords as usize {
        coords[i] = 0;
        i += 1;
    }

    Ok(())
}

/// `TT_Set_Var_Design`: Set the coordinates for the instance, measured in
/// the user coordinate system.  Parse the `avar' table (if present) to
/// convert from user to normalized coordinates.
///
/// `num_coords`: The number of available coordinates.  If it is larger
/// than the number of axes, ignore the excess values.  If it is smaller
/// than the number of axes, use the default values for the remaining
/// axes.
///
/// Returns `Err(GX_ERR_NO_CHANGE)` (-1) for success and unchanged axis
/// values.
pub fn tt_set_var_design(
    face: &mut TtFaceRec,
    stream: &mut FtStreamRec,
    num_coords: FtUInt,
    coords: &[FtFixed],
) -> FtResult<()> {
    let mut have_diff = false;

    if face.blend.is_none() {
        tt_get_mm_var(face, stream, false)?;
    }

    let is_named_instance = ft_is_named_instance(&face.root);
    let face_index = face.root.face_index;

    let blend = face.blend.as_mut().unwrap();
    let num_axis = blend.mmvar.as_ref().unwrap().num_axis;

    let mut num_coords = num_coords;
    if num_coords > num_axis {
        num_coords = num_axis;
    }

    if blend.coords.is_none() {
        blend.coords = Some(ft_new_array(num_axis as FtLong)?);
    }

    {
        let mmvar = blend.mmvar.as_ref().unwrap();
        let c = blend.coords.as_mut().unwrap();
        let mut i = 0usize;
        while i < num_coords as usize {
            if c[i] != coords[i] {
                c[i] = coords[i];
                have_diff = true;
            }
            i += 1;
        }

        if is_named_instance {
            let instance_index = (face_index as FtUInt) >> 16;
            let named_style = &mmvar.namedstyle[instance_index as usize - 1];

            while i < num_axis as usize {
                let n = named_style.coords[i];
                if c[i] != n {
                    c[i] = n;
                    have_diff = true;
                }
                i += 1;
            }
        } else {
            while i < num_axis as usize {
                let a = &mmvar.axis[i];
                if c[i] != a.def {
                    c[i] = a.def;
                    have_diff = true;
                }
                i += 1;
            }
        }
    }

    /* return value -1 indicates `no change';                      */
    /* we can exit early if `normalizedcoords' is already computed */
    if blend.normalizedcoords.is_some() && !have_diff {
        return Err(GX_ERR_NO_CHANGE);
    }

    let mut normalized: Vec<FtFixed> = ft_new_array(num_axis as FtLong)?;

    if !blend.avar_loaded {
        ft_var_load_avar(face, stream);
    }

    let blend_coords = face.blend.as_ref().unwrap().coords.clone().unwrap();
    ft_var_to_normalized(face, num_coords, &blend_coords, &mut normalized);

    tt_set_mm_blend(face, stream, num_axis, Some(&normalized), false)
}

/// `TT_Get_Var_Design`: Get the design coordinates of the currently
/// selected interpolated font.
pub fn tt_get_var_design(
    face: &mut TtFaceRec,
    stream: &mut FtStreamRec,
    coords: &mut [FtFixed],
) -> FtResult<()> {
    let num_coords = coords.len() as FtUInt;

    if face.blend.is_none() {
        tt_get_mm_var(face, stream, false)?;
    }

    if face.blend.as_ref().unwrap().coords.is_none() {
        /* select default instance coordinates */
        /* if no instance is selected yet      */
        tt_set_mm_blend(face, stream, 0, None, true)?;
    }

    let blend = face.blend.as_ref().unwrap();
    let mut nc = num_coords;
    if num_coords > blend.num_axis {
        nc = blend.num_axis;
    }

    let mut i = 0usize;
    if face.doblend {
        while i < nc as usize {
            coords[i] = blend.coords.as_ref().unwrap()[i];
            i += 1;
        }
    } else {
        while i < nc as usize {
            coords[i] = 0;
            i += 1;
        }
    }

    while i < num_coords as usize {
        coords[i] = 0;
        i += 1;
    }

    Ok(())
}

/// `TT_Set_Named_Instance`: Set the given named instance, also resetting
/// any further variation.
///
/// `instance_index`: The instance index, starting with value 1.  Value 0
/// indicates to not use an instance.
///
/// Returns `Err(GX_ERR_NO_CHANGE)` (-1) for success and unchanged axis
/// values.
pub fn tt_set_named_instance(
    face: &mut TtFaceRec,
    stream: &mut FtStreamRec,
    instance_index: FtUInt,
) -> FtResult<()> {
    if face.blend.is_none() {
        tt_get_mm_var(face, stream, false)?;
    }

    let num_instances = (face.root.style_flags as FtUInt) >> 16;

    /* `instance_index' starts with value 1, thus `>' */
    if instance_index > num_instances {
        return Err(FT_ERR_INVALID_ARGUMENT);
    }

    if instance_index > 0 {
        let mmvar = face.blend.as_ref().unwrap().mmvar.as_ref().unwrap();
        let named_style = &mmvar.namedstyle[instance_index as usize - 1];
        let strid = named_style.strid;
        let num_axis = mmvar.num_axis;
        let named_coords = named_style.coords.clone();

        let style_name = tt_face_get_name(face, stream, strid as FtUShort)?;

        /* set (or replace) style name */
        face.root.style_name = style_name;

        /* finally, select the named instance */
        tt_set_var_design(face, stream, num_axis, &named_coords)
    } else {
        /* restore non-VF style name */
        face.root.style_name = face.non_var_style_name.clone();

        tt_set_var_design(face, stream, 0, &[])
    }
}

/// `TT_Get_Default_Named_Instance`: Get the default named instance.
pub fn tt_get_default_named_instance(
    face: &mut TtFaceRec,
    stream: &mut FtStreamRec,
) -> FtResult<FtUInt> {
    if face.blend.is_none() {
        tt_get_mm_var(face, stream, false)?;
    }

    Ok(face.var_default_named_instance)
}

/// `tt_construct_ps_name`: This function triggers (lazy) recomputation of
/// the `postscript_name` field in `TT_Face`.
pub fn tt_construct_ps_name(face: &mut TtFaceRec) {
    face.postscript_name = None;
}

/// The part of `FT_Set_Named_Instance` (src/base/ftmm.c) that
/// `tt_face_init` calls, with the TrueType driver's multiple masters and
/// metrics variations services.
pub fn tt_set_named_instance_ftmm(face: &mut TtFaceRec, instance_index: FtUInt) -> FtResult<()> {
    /* check of `face' delayed to `ft_face_get_mm_service' */
    if !ft_has_multiple_masters(&face.root) {
        return Err(FT_ERR_INVALID_ARGUMENT);
    }

    let error = with_stream(face, |face, stream| {
        tt_set_named_instance(face, stream, instance_index)
    });

    if error.is_ok() || error == Err(GX_ERR_NO_CHANGE) {
        let is_variation_old = ft_is_variation(&face.root);

        face.root.face_flags &= !FT_FACE_FLAG_VARIATION;
        face.root.face_index = ((instance_index as FtLong) << 16) | (face.root.face_index & 0xFFFF);

        tt_construct_ps_name(face);

        /* internal error code -1 means `no change'; we can exit immediately */
        if error == Err(GX_ERR_NO_CHANGE) && is_variation_old == ft_is_variation(&face.root) {
            return Ok(());
        }
    }

    if error.is_ok() {
        tt_apply_mvar(face);
    }

    /* enforce recomputation of auto-hinting data */
    if error.is_ok() && face.root.autohint.is_some() {
        face.root.autohint = None;
    }

    error
}

/*************************************************************************/
/*************************************************************************/
/*****                                                               *****/
/*****                     GX VAR PARSING ROUTINES                   *****/
/*****                                                               *****/
/*************************************************************************/
/*************************************************************************/

/// `tt_face_vary_cvt`: Modify the loaded cvt table according to the
/// `cvar' table and the font's blend.
///
/// Most errors are ignored.  It is perfectly valid not to have a `cvar'
/// table even if there is a `gvar' and `fvar' table.
pub fn tt_face_vary_cvt(face: &mut TtFaceRec, stream: &mut FtStreamRec) -> FtResult<()> {
    let error = tt_face_vary_cvt_body(face, stream);

    /* iterate over all FT_Size objects and set `cvt_ready' to -1 */
    /* to trigger rescaling of all CVT values                     */
    if face.root.has_slot_and_size {
        face.size.cvt_ready = -1;
    }

    error
}

fn tt_face_vary_cvt_body(face: &mut TtFaceRec, stream: &mut FtStreamRec) -> FtResult<()> {
    if face.blend.is_none() {
        return Ok(());
    }

    if face.cvt.is_empty() {
        return Ok(());
    }

    let table_len = match tt_face_goto_table(face, TTAG_cvar as FtULong, stream) {
        Ok(l) => l,
        Err(_) => return Ok(()),
    };

    if stream.enter_frame(table_len).is_err() {
        return Ok(());
    }

    let r = tt_face_vary_cvt_frame(face, stream, table_len);

    /* FExit: */
    stream.exit_frame();

    r
}

fn tt_face_vary_cvt_frame(
    face: &mut TtFaceRec,
    stream: &mut FtStreamRec,
    table_len: FtULong,
) -> FtResult<()> {
    let blend = face.blend.as_ref().unwrap();
    let num_axis = blend.num_axis as usize;

    let table_start = ft_stream_ftell(stream);
    if stream.get_long() != 0x00010000 {
        return Ok(());
    }

    let mut tuple_coords: Vec<FtFixed> = ft_new_array(num_axis as FtLong)?;
    let mut im_start_coords: Vec<FtFixed> = ft_new_array(num_axis as FtLong)?;
    let mut im_end_coords: Vec<FtFixed> = ft_new_array(num_axis as FtLong)?;

    let tuple_count = stream.get_ushort() as FtUInt;
    let mut offset_to_data = stream.get_ushort() as FtULong;

    /* rough sanity test */
    if offset_to_data + (tuple_count & GX_TC_TUPLE_COUNT_MASK) as FtULong * 4 > table_len {
        return Err(FT_ERR_INVALID_TABLE);
    }

    offset_to_data += table_start;

    let mut sharedpoints: Option<PackedPoints> = None;
    let mut spoint_count: FtUInt = 0;

    if tuple_count & GX_TC_TUPLES_SHARE_POINT_NUMBERS != 0 {
        let here = ft_stream_ftell(stream);

        ft_stream_seek_set(stream, offset_to_data);

        sharedpoints = ft_var_readpackedpoints(stream, table_len, &mut spoint_count);

        offset_to_data = ft_stream_ftell(stream);

        ft_stream_seek_set(stream, here);
    }

    let cvt_size = face.cvt_size;
    let mut cvt_deltas: Vec<FtFixed> = ft_new_array(cvt_size as FtLong)?;

    for _ in 0..(tuple_count & GX_TC_TUPLE_COUNT_MASK) {
        let tuple_data_size = stream.get_ushort() as FtULong;
        let tuple_index = stream.get_ushort() as FtUInt;

        if tuple_index & GX_TI_EMBEDDED_TUPLE_COORD != 0 {
            for c in tuple_coords.iter_mut() {
                *c = ft_fdot14_to_fixed(stream.get_short());
            }
        } else if (tuple_index & GX_TI_TUPLE_INDEX_MASK) >= blend.tuplecount {
            return Err(FT_ERR_INVALID_TABLE);
        } else {
            let Some(tc) = blend.tuplecoords.as_ref() else {
                return Err(FT_ERR_INVALID_TABLE);
            };

            let start = (tuple_index & GX_TI_TUPLE_INDEX_MASK) as usize * num_axis;
            tuple_coords.copy_from_slice(&tc[start..start + num_axis]);
        }

        if tuple_index & GX_TI_INTERMEDIATE_TUPLE != 0 {
            for c in im_start_coords.iter_mut() {
                *c = ft_fdot14_to_fixed(stream.get_short());
            }
            for c in im_end_coords.iter_mut() {
                *c = ft_fdot14_to_fixed(stream.get_short());
            }
        }

        let apply = ft_var_apply_tuple(
            blend,
            tuple_index as FtUShort,
            &tuple_coords,
            &im_start_coords,
            &im_end_coords,
        );

        if apply == 0 {
            /* tuple isn't active for our blend */
            offset_to_data += tuple_data_size;
            continue;
        }

        let here = ft_stream_ftell(stream);

        ft_stream_seek_set(stream, offset_to_data);

        let mut point_count: FtUInt = 0;
        let localpoints: Option<PackedPoints>;
        let private_points = tuple_index & GX_TI_PRIVATE_POINT_NUMBERS != 0;
        if private_points {
            localpoints = ft_var_readpackedpoints(stream, table_len, &mut point_count);
        } else {
            localpoints = None;
            point_count = spoint_count;
        }
        let points = if private_points {
            localpoints.as_ref()
        } else {
            sharedpoints.as_ref()
        };

        let deltas = ft_var_readpackeddeltas(
            stream,
            table_len,
            if point_count == 0 {
                cvt_size as FtUInt
            } else {
                point_count
            },
        );

        match (points, deltas) {
            (None, _) | (_, None) => { /* failure, ignore it */ }
            (Some(points), Some(deltas)) => {
                if matches!(localpoints, Some(PackedPoints::All)) {
                    /* this means that there are deltas for every entry in cvt */
                    for j in 0..cvt_size as usize {
                        let old_cvt_delta = cvt_deltas[j];
                        cvt_deltas[j] = old_cvt_delta.wrapping_add(ft_mul_fix(deltas[j], apply));
                    }
                } else {
                    /* FIXME (upstream): the test above is `localpoints ==   */
                    /* ALL_POINTS', so shared `ALL_POINTS' point numbers end */
                    /* up here with a point count of zero and are ignored.   */
                    if let PackedPoints::Points(points) = points {
                        for j in 0..point_count as usize {
                            let pindex = points[j] as usize;

                            if pindex as FtULong >= cvt_size {
                                continue;
                            }

                            let old_cvt_delta = cvt_deltas[pindex];
                            cvt_deltas[pindex] =
                                old_cvt_delta.wrapping_add(ft_mul_fix(deltas[j], apply));
                        }
                    }
                }
            }
        }

        offset_to_data += tuple_data_size;

        ft_stream_seek_set(stream, here);
    }

    for (cvt, &d) in face.cvt.iter_mut().zip(cvt_deltas.iter()) {
        *cvt = cvt.wrapping_add(ft_fixed_to_fdot6(d) as FtInt32);
    }

    Ok(())
}

/// `tt_delta_shift`: Shift the original coordinates of all points between
/// indices `p1' and `p2', using the same difference as given by index
/// `ref'.
///
/// modeled after `af_iup_shift'
fn tt_delta_shift(
    p1: usize,
    p2: usize,
    ref_: usize,
    in_points: &[FtVector],
    out_points: &mut [FtVector],
) {
    let delta = FtVector {
        x: out_points[ref_].x - in_points[ref_].x,
        y: out_points[ref_].y - in_points[ref_].y,
    };

    if delta.x == 0 && delta.y == 0 {
        return;
    }

    for p in p1..ref_ {
        out_points[p].x += delta.x;
        out_points[p].y += delta.y;
    }

    for p in ref_ + 1..=p2 {
        out_points[p].x += delta.x;
        out_points[p].y += delta.y;
    }
}

/// `tt_delta_interpolate`: Interpolate the original coordinates of all
/// points with indices between `p1' and `p2', using `ref1' and `ref2' as
/// the reference point indices.
///
/// modeled after `af_iup_interp', `_iup_worker_interpolate', and
/// `Ins_IUP' with spec differences in handling ill-defined cases.
fn tt_delta_interpolate(
    p1: i32,
    p2: i32,
    ref1: i32,
    ref2: i32,
    in_points: &[FtVector],
    out_points: &mut [FtVector],
) {
    let (mut ref1, mut ref2) = (ref1 as usize, ref2 as usize);

    if p1 > p2 {
        return;
    }

    /* handle both horizontal and vertical coordinates */
    for i in 0..=1 {
        /* (C shifts the array pointers so that it can access `foo.y' as */
        /* `foo.x'; the coordinate is selected here)                     */
        let get = |v: &FtVector| if i == 0 { v.x } else { v.y };

        if get(&in_points[ref1]) > get(&in_points[ref2]) {
            std::mem::swap(&mut ref1, &mut ref2);
        }

        let in1 = get(&in_points[ref1]);
        let in2 = get(&in_points[ref2]);
        let out1 = get(&out_points[ref1]);
        let out2 = get(&out_points[ref2]);
        let d1 = out1 - in1;
        let d2 = out2 - in2;

        /* If the reference points have the same coordinate but different */
        /* delta, inferred delta is zero.  Otherwise interpolate.         */
        if in1 != in2 || out1 == out2 {
            let scale = if in1 != in2 {
                ft_div_fix(out2 - out1, in2 - in1)
            } else {
                0
            };

            for p in p1 as usize..=p2 as usize {
                let mut out = get(&in_points[p]);

                if out <= in1 {
                    out += d1;
                } else if out >= in2 {
                    out += d2;
                } else {
                    out = out1 + ft_mul_fix(out - in1, scale);
                }

                if i == 0 {
                    out_points[p].x = out;
                } else {
                    out_points[p].y = out;
                }
            }
        }
    }
}

/// `tt_interpolate_deltas`: Interpolate points without delta values,
/// similar to the `IUP' hinting instruction.
///
/// modeled after `Ins_IUP
fn tt_interpolate_deltas(
    outline: &GxOutlineView<'_>,
    out_points: &mut [FtVector],
    in_points: &[FtVector],
    has_delta: &[bool],
) {
    /* ignore empty outlines */
    if outline.n_contours == 0 {
        return;
    }

    let mut contour: FtShort = 0;
    let mut point: FtInt = 0;

    loop {
        let end_point = outline.contours[contour as usize] as FtInt;
        let first_point = point;

        /* search first point that has a delta */
        while point <= end_point && !has_delta[point as usize] {
            point += 1;
        }

        if point <= end_point {
            let first_delta = point;
            let mut cur_delta = point;

            point += 1;

            while point <= end_point {
                /* search next point that has a delta  */
                /* and interpolate intermediate points */
                if has_delta[point as usize] {
                    tt_delta_interpolate(
                        cur_delta + 1,
                        point - 1,
                        cur_delta,
                        point,
                        in_points,
                        out_points,
                    );
                    cur_delta = point;
                }

                point += 1;
            }

            /* shift contour if we only have a single delta */
            if cur_delta == first_delta {
                tt_delta_shift(
                    first_point as usize,
                    end_point as usize,
                    cur_delta as usize,
                    in_points,
                    out_points,
                );
            } else {
                /* otherwise handle remaining points       */
                /* at the end and beginning of the contour */
                tt_delta_interpolate(
                    cur_delta + 1,
                    end_point,
                    cur_delta,
                    first_delta,
                    in_points,
                    out_points,
                );

                if first_delta > 0 {
                    tt_delta_interpolate(
                        first_point,
                        first_delta - 1,
                        cur_delta,
                        first_delta,
                        in_points,
                        out_points,
                    );
                }
            }
        }
        contour += 1;

        if contour >= outline.n_contours {
            break;
        }
    }
}

/// `TT_Vary_Apply_Glyph_Deltas`: Apply the appropriate deltas to the
/// current glyph.
///
/// `outline` (InOut): The outline to change, with appended phantom
/// points.
///
/// `unrounded` (Output): An array with `n_points' elements that is filled
/// with unrounded point coordinates (in 26.6 format).
///
/// The loader's phantom points, `linear`, and `vadvance` are in
/// `phantoms`.
pub fn tt_vary_apply_glyph_deltas(
    face: &mut TtFaceRec,
    stream: &mut FtStreamRec,
    glyph_index: FtUInt,
    outline: &mut GxOutlineView<'_>,
    unrounded: &mut [FtVector],
    phantoms: &mut GxVaryPhantoms,
) -> FtResult<()> {
    let n_points = outline.n_points as FtUInt + 4;

    if !face.doblend || face.blend.is_none() {
        return Err(FT_ERR_INVALID_ARGUMENT);
    }

    for i in 0..n_points as usize {
        unrounded[i].x = int_to_f26dot6(outline.points[i].x);
        unrounded[i].y = int_to_f26dot6(outline.points[i].y);
    }

    let blend = face.blend.as_ref().unwrap();
    let glyphoffsets = blend.glyphoffsets.as_deref().unwrap_or(&[]);

    if glyph_index >= blend.gv_glyphcnt
        || glyphoffsets[glyph_index as usize] == glyphoffsets[glyph_index as usize + 1]
    {
        return Ok(());
    }

    let mut points_org: Vec<FtVector> = ft_new_array(n_points as FtLong)?; /* coordinates in 16.16 format */
    let mut points_out: Vec<FtVector> = ft_new_array(n_points as FtLong)?; /* coordinates in 16.16 format */
    let mut has_delta: Vec<bool> = ft_new_array(n_points as FtLong)?;

    let data_size = glyphoffsets[glyph_index as usize + 1] - glyphoffsets[glyph_index as usize];

    stream.seek(glyphoffsets[glyph_index as usize])?;
    stream.enter_frame(data_size)?;

    let r = tt_vary_apply_glyph_deltas_frame(
        face,
        stream,
        outline,
        unrounded,
        phantoms,
        data_size,
        &mut points_org,
        &mut points_out,
        &mut has_delta,
    );

    /* Fail2: */
    stream.exit_frame();

    r
}

#[allow(clippy::too_many_arguments)]
fn tt_vary_apply_glyph_deltas_frame(
    face: &TtFaceRec,
    stream: &mut FtStreamRec,
    outline: &mut GxOutlineView<'_>,
    unrounded: &mut [FtVector],
    phantoms: &mut GxVaryPhantoms,
    data_size: FtULong,
    points_org: &mut [FtVector],
    points_out: &mut [FtVector],
    has_delta: &mut [bool],
) -> FtResult<()> {
    let blend = face.blend.as_ref().unwrap();
    let n_points = outline.n_points as FtUInt + 4;
    let num_axis = blend.num_axis as usize;

    let glyph_start = ft_stream_ftell(stream);

    /* each set of glyph variation data is formatted similarly to `cvar' */

    let mut tuple_coords: Vec<FtFixed> = ft_new_array(num_axis as FtLong)?;
    let mut im_start_coords: Vec<FtFixed> = ft_new_array(num_axis as FtLong)?;
    let mut im_end_coords: Vec<FtFixed> = ft_new_array(num_axis as FtLong)?;

    let tuple_count = stream.get_ushort() as FtUInt;
    let mut offset_to_data = stream.get_ushort() as FtULong;

    /* rough sanity test */
    if offset_to_data > data_size
        || (tuple_count & GX_TC_TUPLE_COUNT_MASK) as FtULong * 4 > data_size
    {
        return Err(FT_ERR_INVALID_TABLE);
    }

    offset_to_data += glyph_start;

    let mut sharedpoints: Option<PackedPoints> = None;
    let mut spoint_count: FtUInt = 0;

    if tuple_count & GX_TC_TUPLES_SHARE_POINT_NUMBERS != 0 {
        let here = ft_stream_ftell(stream);

        ft_stream_seek_set(stream, offset_to_data);

        sharedpoints = ft_var_readpackedpoints(stream, blend.gvar_size, &mut spoint_count);

        offset_to_data = ft_stream_ftell(stream);

        ft_stream_seek_set(stream, here);
    }

    let mut point_deltas_x: Vec<FtFixed> = ft_new_array(n_points as FtLong)?;
    let mut point_deltas_y: Vec<FtFixed> = ft_new_array(n_points as FtLong)?;

    for j in 0..n_points as usize {
        points_org[j].x = ft_int_to_fixed(outline.points[j].x);
        points_org[j].y = ft_int_to_fixed(outline.points[j].y);
    }

    for _ in 0..(tuple_count & GX_TC_TUPLE_COUNT_MASK) {
        let tuple_data_size = stream.get_ushort() as FtULong;
        let tuple_index = stream.get_ushort() as FtUInt;

        if tuple_index & GX_TI_EMBEDDED_TUPLE_COORD != 0 {
            for c in tuple_coords.iter_mut() {
                *c = ft_fdot14_to_fixed(stream.get_short());
            }
        } else if (tuple_index & GX_TI_TUPLE_INDEX_MASK) >= blend.tuplecount {
            return Err(FT_ERR_INVALID_TABLE);
        } else {
            let tc = blend.tuplecoords.as_deref().unwrap_or(&[]);
            let start = (tuple_index & GX_TI_TUPLE_INDEX_MASK) as usize * num_axis;
            tuple_coords.copy_from_slice(&tc[start..start + num_axis]);
        }

        if tuple_index & GX_TI_INTERMEDIATE_TUPLE != 0 {
            for c in im_start_coords.iter_mut() {
                *c = ft_fdot14_to_fixed(stream.get_short());
            }
            for c in im_end_coords.iter_mut() {
                *c = ft_fdot14_to_fixed(stream.get_short());
            }
        }

        let apply = ft_var_apply_tuple(
            blend,
            tuple_index as FtUShort,
            &tuple_coords,
            &im_start_coords,
            &im_end_coords,
        );

        if apply == 0 {
            /* tuple isn't active for our blend */
            offset_to_data += tuple_data_size;
            continue;
        }

        let here = ft_stream_ftell(stream);

        ft_stream_seek_set(stream, offset_to_data);

        let mut point_count: FtUInt = 0;
        let localpoints: Option<PackedPoints>;
        let points = if tuple_index & GX_TI_PRIVATE_POINT_NUMBERS != 0 {
            localpoints = ft_var_readpackedpoints(stream, blend.gvar_size, &mut point_count);
            localpoints.as_ref()
        } else {
            point_count = spoint_count;
            sharedpoints.as_ref()
        };

        let cnt = if point_count == 0 {
            n_points
        } else {
            point_count
        };
        let deltas_x = ft_var_readpackeddeltas(stream, blend.gvar_size, cnt);
        let deltas_y = ft_var_readpackeddeltas(stream, blend.gvar_size, cnt);

        match (points, deltas_y, deltas_x) {
            (Some(points), Some(deltas_y), Some(deltas_x)) => match points {
                PackedPoints::All => {
                    /* this means that there are deltas for every point in the glyph */
                    for j in 0..n_points as usize {
                        let old_point_delta_x = point_deltas_x[j];
                        let old_point_delta_y = point_deltas_y[j];

                        let point_delta_x = ft_mul_fix(deltas_x[j], apply);
                        let point_delta_y = ft_mul_fix(deltas_y[j], apply);

                        point_deltas_x[j] = old_point_delta_x.wrapping_add(point_delta_x);
                        point_deltas_y[j] = old_point_delta_y.wrapping_add(point_delta_y);
                    }
                }
                PackedPoints::Points(points) => {
                    /* we have to interpolate the missing deltas similar to the */
                    /* IUP bytecode instruction                                 */
                    for j in 0..n_points as usize {
                        has_delta[j] = false;
                        points_out[j] = points_org[j];
                    }

                    for j in 0..point_count as usize {
                        let idx = points[j];

                        if idx as FtUInt >= n_points {
                            continue;
                        }

                        has_delta[idx as usize] = true;

                        points_out[idx as usize].x = points_out[idx as usize]
                            .x
                            .wrapping_add(ft_mul_fix(deltas_x[j], apply));
                        points_out[idx as usize].y = points_out[idx as usize]
                            .y
                            .wrapping_add(ft_mul_fix(deltas_y[j], apply));
                    }

                    /* no need to handle phantom points here,      */
                    /* since solitary points can't be interpolated */
                    tt_interpolate_deltas(outline, points_out, points_org, has_delta);

                    for j in 0..n_points as usize {
                        let old_point_delta_x = point_deltas_x[j];
                        let old_point_delta_y = point_deltas_y[j];

                        let point_delta_x = points_out[j].x.wrapping_sub(points_org[j].x);
                        let point_delta_y = points_out[j].y.wrapping_sub(points_org[j].y);

                        point_deltas_x[j] = old_point_delta_x.wrapping_add(point_delta_x);
                        point_deltas_y[j] = old_point_delta_y.wrapping_add(point_delta_y);
                    }
                }
            },
            _ => { /* failure, ignore it */ }
        }

        offset_to_data += tuple_data_size;

        ft_stream_seek_set(stream, here);
    }

    /* To avoid double adjustment of advance width or height, */
    /* do not move phantom points if there is HVAR or VVAR    */
    /* support, respectively.                                 */
    let n = n_points as usize;
    if face.variation_support & TT_FACE_FLAG_VAR_HADVANCE != 0 {
        point_deltas_x[n - 4] = 0;
        point_deltas_y[n - 4] = 0;
        point_deltas_x[n - 3] = 0;
        point_deltas_y[n - 3] = 0;
    }
    if face.variation_support & TT_FACE_FLAG_VAR_VADVANCE != 0 {
        point_deltas_x[n - 2] = 0;
        point_deltas_y[n - 2] = 0;
        point_deltas_x[n - 1] = 0;
        point_deltas_y[n - 1] = 0;
    }

    for i in 0..n {
        unrounded[i].x = unrounded[i]
            .x
            .wrapping_add(ft_fixed_to_fdot6(point_deltas_x[i]));
        unrounded[i].y = unrounded[i]
            .y
            .wrapping_add(ft_fixed_to_fdot6(point_deltas_y[i]));

        outline.points[i].x = outline.points[i]
            .x
            .wrapping_add(ft_fixed_to_int(point_deltas_x[i]) as FtPos);
        outline.points[i].y = outline.points[i]
            .y
            .wrapping_add(ft_fixed_to_int(point_deltas_y[i]) as FtPos);
    }

    /* To avoid double adjustment of advance width or height, */
    /* adjust phantom points only if there is no HVAR or VVAR */
    /* support, respectively.                                 */
    if face.variation_support & TT_FACE_FLAG_VAR_HADVANCE == 0 {
        phantoms.pp1 = outline.points[n - 4];
        phantoms.pp2 = outline.points[n - 3];
        phantoms.linear =
            (ft_pix_round(unrounded[n - 3].x.wrapping_sub(unrounded[n - 4].x)) / 64) as FtInt;
    }

    if face.variation_support & TT_FACE_FLAG_VAR_VADVANCE == 0 {
        phantoms.pp3 = outline.points[n - 2];
        phantoms.pp4 = outline.points[n - 1];
        phantoms.vadvance =
            (ft_pix_round(unrounded[n - 1].y.wrapping_sub(unrounded[n - 2].y)) / 64) as FtInt;
    }

    Ok(())
}

/// `tt_get_var_blend`: An extended internal version of `TT_Get_MM_Blend'
/// that returns pointers instead of copying data, without any
/// initialization of the MM machinery in case it isn't loaded yet.
///
/// Returns `(num_coords, coords, normalizedcoords, mm_var)`.
#[allow(clippy::type_complexity)]
pub fn tt_get_var_blend(
    face: &TtFaceRec,
) -> (
    FtUInt,
    Option<&[FtFixed]>,
    Option<&[FtFixed]>,
    Option<&FtMMVar>,
) {
    match face.blend.as_ref() {
        Some(blend) => (
            blend.num_axis,
            blend.coords.as_deref(),
            blend.normalizedcoords.as_deref(),
            blend.mmvar.as_deref(),
        ),
        /* (C leaves `normalizedcoords' untouched here) */
        None => (0, None, None, None),
    }
}

/// `tt_var_done_item_variation_store`
pub fn tt_var_done_item_variation_store(item_store: &mut GxItemVarStoreRec) {
    item_store.varData = Vec::new();
    item_store.varRegionList = Vec::new();
}

/// `tt_var_done_delta_set_index_map`
pub fn tt_var_done_delta_set_index_map(delta_set_idx_map: &mut GxDeltaSetIdxMapRec) {
    delta_set_idx_map.innerIndex = Vec::new();
    delta_set_idx_map.outerIndex = Vec::new();
}

/// `tt_done_blend`: Free the blend internal data structure.
pub fn tt_done_blend(face: &mut TtFaceRec) {
    if let Some(mut blend) = face.blend.take() {
        if let Some(table) = blend.avar_table.as_mut() {
            table.avar_segment = None;

            tt_var_done_item_variation_store(&mut table.itemStore);
            tt_var_done_delta_set_index_map(&mut table.axisMap);
        }

        if let Some(table) = blend.hvar_table.as_mut() {
            tt_var_done_item_variation_store(&mut table.itemStore);
            tt_var_done_delta_set_index_map(&mut table.widthMap);
        }

        if let Some(table) = blend.vvar_table.as_mut() {
            tt_var_done_item_variation_store(&mut table.itemStore);
            tt_var_done_delta_set_index_map(&mut table.widthMap);
        }

        if let Some(table) = blend.mvar_table.as_mut() {
            tt_var_done_item_variation_store(&mut table.itemStore);
        }

        /* (the rest is freed when `blend' is dropped) */
    }
}
