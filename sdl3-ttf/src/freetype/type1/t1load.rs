// Rust translation of src/type1/t1load.c, src/type1/t1load.h and
// src/type1/t1tokens.h from FreeType (2.13.2, as SDL_ttf's
// external/freetype pins it).
// Copyright (C) 1996-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! Type 1 font loader (body).
//!
//! This is the new and improved Type 1 data loader for FreeType 2.  The
//! old loader has several problems: it is slow, complex, difficult to
//! maintain, and contains incredible hacks to make it accept some
//! ill-formed Type 1 fonts without hiccup-ing.  Moreover, about 5% of
//! the Type 1 fonts on my machine still aren't loaded correctly by it.
//!
//! This version is much simpler, much faster and also easier to read and
//! maintain by a great order of magnitude.  The idea behind it is to
//! _not_ try to read the Type 1 token stream with a state machine (i.e.
//! a Postscript-like interpreter) but rather to perform simple pattern
//! matching.
//!
//! Indeed, nearly all data definitions follow a simple pattern like
//!
//! `... /Field <data> ...`
//!
//! where `<data>` can be a number, a boolean, a string, or an array of
//! numbers.  There are a few exceptions, namely the encoding, font name,
//! charstrings, and subrs; they are handled with a special pattern
//! matching routine.
//!
//! All other common cases are handled very simply.  The matching rules
//! are defined in the file `t1tokens.h' through the use of several
//! macros calls PARSE_XXX.  This file is included twice here; the first
//! time to generate parsing callback functions, the second time to
//! generate a table of keywords (with pointers to the associated
//! callback functions).
//!
//! The function `parse_dict' simply scans *linearly* a given dictionary
//! (either the top-level or private one) and calls the appropriate
//! callback when it encounters an immediate keyword.
//!
//! This is by far the fastest way one can find to parse and read all
//! data.
//!
//! This led to tremendous code size reduction.  Note that later, the
//! glyph loader will also be _greatly_ simplified, and the automatic
//! hinter will replace the clumsy `t1hinter'.
//!
//! `FT_CONFIG_OPTION_INCREMENTAL` is defined, but no incremental
//! interface is ever set (`IS_INCREMENTAL` is false). The keyword table
//! (`t1_keywords`, with `t1tokens.h`) is [`T1_KEYWORDS`]. The blend
//! record's `design_pos` is its one block; the `font_infos`, `privates`
//! and `bboxes` of the designs after the first (the face's own records)
//! are its vectors.

use std::collections::HashMap;

use super::super::base::ftcalc::*;
use super::super::base::ftmemory::{ft_new_array, ft_qalloc};
use super::super::base::ftstream::FtStreamRec;
use super::super::fttypes::*;
use super::super::psaux::psconv::*;
use super::super::psaux::psobjs::*;
use super::super::t1tables::*;
use super::super::t1types::*;
use super::super::truetype::ttgxvar::{FtMMVar, FtVarAxis};
use super::t1parse::*;
use crate::{t1_callback_field, t1_field, t1_table_field, t1_table_field2};

/// `T1_LoaderRec`
#[derive(Debug, Default)]
pub struct T1LoaderRec {
    pub parser: T1ParserRec, /* parser used to read the stream */

    pub num_chars: FtInt,           /* number of characters in encoding */
    pub encoding_table: PsTableRec, /* PS_Table used to store the       */
    /* encoding character names         */
    pub num_glyphs: FtInt,
    pub glyph_names: PsTableRec,
    pub charstrings: PsTableRec,
    pub swap_table: PsTableRec, /* For moving .notdef glyph to index 0. */

    pub num_subrs: FtInt,
    pub subrs: PsTableRec,
    pub subrs_hash: Option<HashMap<FtInt, usize>>,
    pub fontdata: bool,

    pub keywords_encountered: FtUInt, /* T1_LOADER_ENCOUNTERED_XXX */
}

/* treatment of some keywords differs depending on whether */
/* they precede or follow certain other keywords           */

pub const T1_PRIVATE: FtUInt = 1 << 0;
pub const T1_FONTDIR_AFTER_PRIVATE: FtUInt = 1 << 1;

/* ftmm.h */

/// `FT_MM_Axis`
#[derive(Debug, Clone, Default)]
pub struct FtMMAxis {
    pub name: Option<Vec<u8>>,
    pub minimum: FtLong,
    pub maximum: FtLong,
}

/// `FT_Multi_Master`
#[derive(Debug, Clone, Default)]
pub struct FtMultiMaster {
    pub num_axis: FtUInt,
    pub num_designs: FtUInt,
    pub axis: [FtMMAxis; T1_MAX_MM_AXIS],
}

/// The type of the keyword callbacks (`T1_Field_ParseFunc`).
pub type T1FieldReader = fn(face: &mut T1FaceRec, loader: &mut T1LoaderRec);

/// A keyword of the Type 1 loader.
pub type T1Field = T1FieldRec<T1FieldReader>;

/// The parser's error as a result.
fn parser_result(error: FtError) -> FtResult<()> {
    if error == 0 {
        Ok(())
    } else {
        Err(error)
    }
}

/// An error as the parser's error code.
fn error_code(r: FtResult<()>) -> FtError {
    match r {
        Ok(()) => FT_ERR_OK,
        Err(e) => e,
    }
}

/*************************************************************************/
/*************************************************************************/
/*****                                                               *****/
/*****                    MULTIPLE MASTERS SUPPORT                   *****/
/*****                                                               *****/
/*************************************************************************/
/*************************************************************************/

/// `t1_allocate_blend`
fn t1_allocate_blend(face: &mut T1FaceRec, num_designs: FtUInt, num_axis: FtUInt) -> FtResult<()> {
    if face.blend.is_none() {
        let mut blend = Box::<PsBlendRec>::default();

        blend.num_default_design_vector = 0;
        blend.weight_vector = None;
        blend.default_weight_vector = Vec::new();
        blend.design_pos = None;

        face.blend = Some(blend);
    }
    let blend = face.blend.as_mut().unwrap();

    /* allocate design data if needed */
    if num_designs > 0 {
        if blend.num_designs == 0 {
            /* allocate the blend `private' and `font_info' dictionaries */
            /* (the first design's are the face's own)                   */
            blend.font_infos = ft_new_array(num_designs as FtLong)?;
            blend.privates = ft_new_array(num_designs as FtLong)?;
            blend.bboxes = ft_new_array(num_designs as FtLong)?;

            blend.num_designs = num_designs;
        } else if blend.num_designs != num_designs {
            return Err(FT_ERR_INVALID_FILE_FORMAT);
        }
    }

    /* allocate axis data if needed */
    if num_axis > 0 {
        if blend.num_axis != 0 && blend.num_axis != num_axis {
            return Err(FT_ERR_INVALID_FILE_FORMAT);
        }

        blend.num_axis = num_axis;
    }

    /* Exit: */
    Ok(())
}

/// `T1_Get_Multi_Master`
pub fn t1_get_multi_master(face: &T1FaceRec, master: &mut FtMultiMaster) -> FtResult<()> {
    let Some(blend) = face.blend.as_deref() else {
        return Err(FT_ERR_INVALID_ARGUMENT);
    };

    master.num_axis = blend.num_axis;
    master.num_designs = blend.num_designs;

    for n in 0..(blend.num_axis as usize).min(T1_MAX_MM_AXIS) {
        let axis = &mut master.axis[n];
        let map = &blend.design_map[n];

        axis.name = blend.axis_names[n].clone();
        axis.minimum = map.design_points.first().copied().unwrap_or(0);
        axis.maximum = map
            .design_points
            .get((map.num_points as usize).wrapping_sub(1))
            .copied()
            .unwrap_or(0);
    }

    Ok(())
}

/// `mm_axis_unmap`: given a normalized (blend) coordinate, figure out
/// the design coordinate appropriate for that value.
fn mm_axis_unmap(axismap: &PsDesignMapRec, ncv: FtFixed) -> FtFixed {
    let design = |j: usize| axismap.design_points.get(j).copied().unwrap_or(0);
    let blend = |j: usize| axismap.blend_points.get(j).copied().unwrap_or(0);

    if ncv <= blend(0) {
        return int_to_fixed(design(0));
    }

    for j in 1..axismap.num_points as usize {
        if ncv <= blend(j) {
            return int_to_fixed(
                design(j - 1)
                    + ft_mul_div(
                        ncv - blend(j - 1),
                        design(j) - design(j - 1),
                        blend(j) - blend(j - 1),
                    ),
            );
        }
    }

    int_to_fixed(design((axismap.num_points as usize).wrapping_sub(1)))
}

/// `mm_weights_unmap`: given a vector of weights, one for each design,
/// figure out the normalized axis coordinates which gave rise to those
/// weights.
fn mm_weights_unmap(weights: &[FtFixed], axiscoords: &mut [FtFixed; 4], axis_count: FtUInt) {
    /* (the face's blend has `2^^axis_count' designs) */
    let w = |i: usize| weights.get(i).copied().unwrap_or(0);

    if axis_count == 1 {
        axiscoords[0] = w(1);
    } else if axis_count == 2 {
        axiscoords[0] = w(3) + w(1);
        axiscoords[1] = w(3) + w(2);
    } else if axis_count == 3 {
        axiscoords[0] = w(7) + w(5) + w(3) + w(1);
        axiscoords[1] = w(7) + w(6) + w(3) + w(2);
        axiscoords[2] = w(7) + w(6) + w(5) + w(4);
    } else {
        axiscoords[0] = w(15) + w(13) + w(11) + w(9) + w(7) + w(5) + w(3) + w(1);
        axiscoords[1] = w(15) + w(14) + w(11) + w(10) + w(7) + w(6) + w(3) + w(2);
        axiscoords[2] = w(15) + w(14) + w(13) + w(12) + w(7) + w(6) + w(5) + w(4);
        axiscoords[3] = w(15) + w(14) + w(13) + w(12) + w(11) + w(10) + w(9) + w(8);
    }
}

/// `T1_Get_MM_Var`: just a wrapper around `T1_Get_Multi_Master` to
/// support the different arguments needed by the GX var distortable
/// fonts.
pub fn t1_get_mm_var(face: &T1FaceRec) -> FtResult<FtMMVar> {
    let mut mmaster = FtMultiMaster::default();
    let mut axiscoords: [FtFixed; T1_MAX_MM_AXIS] = [0; T1_MAX_MM_AXIS];

    t1_get_multi_master(face, &mut mmaster)?;
    let blend = face.blend.as_deref().ok_or(FT_ERR_INVALID_ARGUMENT)?;

    /* (the axes and their flags are the record's vectors) */
    let num_axis = mmaster.num_axis as usize;
    let mut mmvar = FtMMVar {
        num_axis: mmaster.num_axis,
        num_designs: mmaster.num_designs,
        num_namedstyles: 0, /* Not supported */
        ..Default::default()
    };

    /* while axis flags are meaningless here, we have to provide the array */
    /* to make `FT_Get_Var_Axis_Flags' work: the function expects that the */
    /* values directly follow the data of `FT_MM_Var'                      */
    mmvar.axis_flags = ft_new_array(num_axis as FtLong)?;

    mmvar.axis = ft_new_array::<FtVarAxis>(num_axis as FtLong)?;

    for i in 0..num_axis {
        let axis = &mut mmvar.axis[i];
        let name = mmaster.axis[i].name.as_deref();

        /* (a NULL name is an empty string) */
        axis.name = String::from_utf8_lossy(name.unwrap_or(b"")).into_owned();
        axis.minimum = int_to_fixed(mmaster.axis[i].minimum);
        axis.maximum = int_to_fixed(mmaster.axis[i].maximum);
        axis.strid = !0; /* Does not apply */
        axis.tag = !0u32 as FtULong; /* Does not apply */

        let Some(name) = name else {
            continue;
        };

        if name == b"Weight" {
            axis.tag = ft_make_tag(b'w', b'g', b'h', b't') as FtULong;
        } else if name == b"Width" {
            axis.tag = ft_make_tag(b'w', b'd', b't', b'h') as FtULong;
        } else if name == b"OpticalSize" {
            axis.tag = ft_make_tag(b'o', b'p', b's', b'z') as FtULong;
        } else if name == b"Slant" {
            axis.tag = ft_make_tag(b's', b'l', b'n', b't') as FtULong;
        } else if name == b"Italic" {
            axis.tag = ft_make_tag(b'i', b't', b'a', b'l') as FtULong;
        }
    }

    mm_weights_unmap(
        &blend.default_weight_vector,
        &mut axiscoords,
        blend.num_axis,
    );

    for i in 0..num_axis {
        mmvar.axis[i].def = mm_axis_unmap(&blend.design_map[i], axiscoords[i]);
    }

    /* Exit: */
    Ok(mmvar)
}

/// `t1_set_mm_blend` (`Err(-1)` is C's `no change')
fn t1_set_mm_blend(face: &mut T1FaceRec, num_coords: FtUInt, coords: &[FtFixed]) -> FtResult<()> {
    let Some(blend) = face.blend.as_deref_mut() else {
        return Err(FT_ERR_INVALID_ARGUMENT);
    };

    let mut have_diff = false;

    let num_coords = num_coords.min(blend.num_axis);

    /* recompute the weight vector from the blend coordinates */
    for n in 0..blend.num_designs {
        let mut result: FtFixed = 0x10000; /* 1.0 fixed */
        let mut factor: FtFixed;

        for m in 0..blend.num_axis {
            /* use a default value if we don't have a coordinate */
            if m >= num_coords {
                result >>= 1;
                continue;
            }

            /* get current blend axis position */
            factor = coords.get(m as usize).copied().unwrap_or(0);
            if (n & (1 << m)) == 0 {
                factor = 0x10000 - factor;
            }

            if factor <= 0 {
                result = 0;
                break;
            } else if factor >= 0x10000 {
                continue;
            }

            result = ft_mul_fix(result, factor);
        }

        if let Some(w) = blend
            .weight_vector
            .as_mut()
            .and_then(|w| w.get_mut(n as usize))
        {
            if *w != result {
                *w = result;
                have_diff = true;
            }
        }
    }

    /* return value -1 indicates `no change' */
    if have_diff {
        Ok(())
    } else {
        Err(-1)
    }
}

/// `T1_Set_MM_Blend`
pub fn t1_set_mm_blend_svc(
    face: &mut T1FaceRec,
    num_coords: FtUInt,
    coords: &[FtFixed],
) -> FtResult<()> {
    t1_set_mm_blend(face, num_coords, coords)
}

/// `T1_Get_MM_Blend` (`coords.len ()` is `num_coords`)
pub fn t1_get_mm_blend(face: &T1FaceRec, coords: &mut [FtFixed]) -> FtResult<()> {
    let Some(blend) = face.blend.as_deref() else {
        return Err(FT_ERR_INVALID_ARGUMENT);
    };

    let mut axiscoords: [FtFixed; 4] = [0; 4];

    mm_weights_unmap(
        blend.weight_vector.as_deref().unwrap_or(&[]),
        &mut axiscoords,
        blend.num_axis,
    );

    let num_coords = coords.len();
    let mut nc = num_coords;
    if num_coords > blend.num_axis as usize {
        nc = blend.num_axis as usize;
    }

    let mut i = 0;
    while i < nc {
        coords[i] = axiscoords[i];
        i += 1;
    }
    while i < num_coords {
        coords[i] = 0x8000;
        i += 1;
    }

    Ok(())
}

/// `T1_Set_MM_WeightVector` (`None` is C's NULL `weightvector`)
pub fn t1_set_mm_weight_vector(
    face: &mut T1FaceRec,
    len: FtUInt,
    weightvector: Option<&[FtFixed]>,
) -> FtResult<()> {
    let Some(blend) = face.blend.as_deref_mut() else {
        return Err(FT_ERR_INVALID_ARGUMENT);
    };
    let num_designs = blend.num_designs as usize;

    if len == 0 && weightvector.is_none() {
        let default = blend.default_weight_vector.clone();
        if let Some(w) = blend.weight_vector.as_mut() {
            for i in 0..num_designs {
                w[i] = default.get(i).copied().unwrap_or(0);
            }
        }
    } else {
        let Some(weightvector) = weightvector else {
            return Err(FT_ERR_INVALID_ARGUMENT);
        };

        let n = (len as usize).min(num_designs);

        if let Some(w) = blend.weight_vector.as_mut() {
            let mut i = 0;
            while i < n {
                w[i] = weightvector.get(i).copied().unwrap_or(0);
                i += 1;
            }

            while i < num_designs {
                w[i] = 0;
                i += 1;
            }
        }
    }

    Ok(())
}

/// `T1_Get_MM_WeightVector` (`weightvector.len ()` is `*len`)
pub fn t1_get_mm_weight_vector(
    face: &T1FaceRec,
    len: &mut FtUInt,
    weightvector: &mut [FtFixed],
) -> FtResult<()> {
    let Some(blend) = face.blend.as_deref() else {
        return Err(FT_ERR_INVALID_ARGUMENT);
    };

    if *len < blend.num_designs {
        *len = blend.num_designs;
        return Err(FT_ERR_INVALID_ARGUMENT);
    }

    let w = blend.weight_vector.as_deref().unwrap_or(&[]);
    let mut i = 0usize;
    while i < blend.num_designs as usize {
        weightvector[i] = w.get(i).copied().unwrap_or(0);
        i += 1;
    }
    while i < *len as usize {
        weightvector[i] = 0;
        i += 1;
    }

    *len = blend.num_designs;

    Ok(())
}

/// `T1_Set_MM_Design`
pub fn t1_set_mm_design(
    face: &mut T1FaceRec,
    num_coords: FtUInt,
    coords: &[FtLong],
) -> FtResult<()> {
    let Some(blend) = face.blend.as_deref() else {
        return Err(FT_ERR_INVALID_ARGUMENT);
    };
    let mut final_blends: [FtFixed; T1_MAX_MM_DESIGNS] = [0; T1_MAX_MM_DESIGNS];

    let num_coords = num_coords.min(blend.num_axis);

    /* compute the blend coordinates through the blend design map */

    for n in 0..(blend.num_axis as usize).min(T1_MAX_MM_AXIS) {
        let design: FtLong;
        let the_blend: FtFixed;
        let map = &blend.design_map[n];
        let designs = |p: usize| map.design_points.get(p).copied().unwrap_or(0);
        let blends = |p: usize| map.blend_points.get(p).copied().unwrap_or(0);
        let mut before: FtInt = -1;
        let mut after: FtInt = -1;
        let last = (map.num_points as usize).wrapping_sub(1);

        /* use a default value if we don't have a coordinate */
        if (n as FtUInt) < num_coords {
            design = coords.get(n).copied().unwrap_or(0);
        } else {
            design = (designs(last) - designs(0)) / 2;
        }

        'found: {
            for p in 0..map.num_points as usize {
                let p_design = designs(p);

                /* exact match? */
                if design == p_design {
                    the_blend = blends(p);
                    break 'found;
                }

                if design < p_design {
                    after = p as FtInt;
                    break;
                }

                before = p as FtInt;
            }

            /* now interpolate if necessary */
            if before < 0 {
                the_blend = blends(0);
            } else if after < 0 {
                the_blend = blends(last);
            } else {
                let (b, a) = (before as usize, after as usize);
                the_blend = ft_mul_div(
                    design - designs(b),
                    blends(a) - blends(b),
                    designs(a) - designs(b),
                );
            }
        }

        /* Found: */
        final_blends[n] = the_blend;
    }

    let num_axis = blend.num_axis;
    t1_set_mm_blend(face, num_axis, &final_blends)?;

    Ok(())
}

/* MM fonts don't have named instances, so only the design is reset */

/// `T1_Reset_MM_Blend`
pub fn t1_reset_mm_blend(face: &mut T1FaceRec, _instance_index: FtUInt) -> FtResult<()> {
    t1_set_mm_blend_svc(face, 0, &[])
}

/// `T1_Set_Var_Design`: just a wrapper around `T1_Set_MM_Design` to
/// support the different arguments needed by the GX var distortable
/// fonts.
pub fn t1_set_var_design(face: &mut T1FaceRec, coords: &[FtFixed]) -> FtResult<()> {
    let mut lcoords: [FtLong; T1_MAX_MM_AXIS] = [0; T1_MAX_MM_AXIS];

    let num_coords = coords.len().min(T1_MAX_MM_AXIS);

    for i in 0..num_coords {
        lcoords[i] = fixed_to_int(coords[i]);
    }

    t1_set_mm_design(face, num_coords as FtUInt, &lcoords)
}

/// `T1_Get_Var_Design` (`coords.len ()` is `num_coords`)
pub fn t1_get_var_design(face: &T1FaceRec, coords: &mut [FtFixed]) -> FtResult<()> {
    let Some(blend) = face.blend.as_deref() else {
        return Err(FT_ERR_INVALID_ARGUMENT);
    };

    let mut axiscoords: [FtFixed; 4] = [0; 4];

    mm_weights_unmap(
        blend.weight_vector.as_deref().unwrap_or(&[]),
        &mut axiscoords,
        blend.num_axis,
    );

    let num_coords = coords.len();
    let mut nc = num_coords;
    if num_coords > blend.num_axis as usize {
        nc = blend.num_axis as usize;
    }

    let mut i = 0;
    while i < nc {
        coords[i] = mm_axis_unmap(&blend.design_map[i], axiscoords[i]);
        i += 1;
    }
    while i < num_coords {
        coords[i] = 0;
        i += 1;
    }

    Ok(())
}

/// `T1_Done_Blend`
pub fn t1_done_blend(face: &mut T1FaceRec) {
    /* (the blend's tables, names and maps go with it) */
    face.blend = None;
}

/// `parse_blend_axis_types`
fn parse_blend_axis_types(face: &mut T1FaceRec, loader: &mut T1LoaderRec) {
    let mut axis_tokens = [T1TokenRec::default(); T1_MAX_MM_AXIS];
    let mut num_axis: FtInt = 0;

    let error = (|| -> FtResult<()> {
        /* take an array of objects */
        t1_to_token_array(
            &mut loader.parser,
            Some(&mut axis_tokens),
            T1_MAX_MM_AXIS as FtUInt,
            &mut num_axis,
        );
        if num_axis < 0 {
            return Err(FT_ERR_IGNORE);
        }
        if num_axis == 0 || num_axis as usize > T1_MAX_MM_AXIS {
            return Err(FT_ERR_INVALID_FILE_FORMAT);
        }

        /* allocate blend if necessary */
        t1_allocate_blend(face, 0, num_axis as FtUInt)?;

        let blend = face.blend.as_mut().unwrap();
        let b = loader.parser.bytes();

        /* each token is an immediate containing the name of the axis */
        for n in 0..num_axis as usize {
            let token = &mut axis_tokens[n];

            /* skip first slash, if any */
            if at(b, token.start()) == b'/' {
                token.start = Some(token.start() + 1);
            }

            let len = token.limit().wrapping_sub(token.start()) as FtUInt;
            if len == 0 {
                return Err(FT_ERR_INVALID_FILE_FORMAT);
            }

            if blend.axis_names[n].is_some() {
                blend.axis_names[n] = None;
            }

            let mut name = ft_qalloc(len as FtLong + 1)?;
            let src = b.get(token.start()..token.limit()).unwrap_or(&[]);
            name[..src.len()].copy_from_slice(src);
            name.truncate(len as usize);
            blend.axis_names[n] = Some(name);
        }

        Ok(())
    })();

    /* Exit: */
    loader.parser.root.error = error_code(error);
}

/// `parse_blend_design_positions`
fn parse_blend_design_positions(face: &mut T1FaceRec, loader: &mut T1LoaderRec) {
    let mut design_tokens = [T1TokenRec::default(); T1_MAX_MM_DESIGNS];
    let mut num_designs: FtInt = 0;
    let mut num_axis: FtInt = 0; /* make compiler happy */
    let parser = &mut loader.parser;
    let mut design_pos: Option<Vec<FtFixed>> = None;

    let error = (|| -> FtResult<()> {
        /* get the array of design tokens -- compute number of designs */
        t1_to_token_array(
            parser,
            Some(&mut design_tokens),
            T1_MAX_MM_DESIGNS as FtUInt,
            &mut num_designs,
        );
        if num_designs < 0 {
            return Err(FT_ERR_IGNORE);
        }
        if num_designs == 0 || num_designs as usize > T1_MAX_MM_DESIGNS {
            return Err(FT_ERR_INVALID_FILE_FORMAT);
        }

        {
            let old_cursor = parser.root.cursor;
            let old_limit = parser.root.limit;

            for n in 0..num_designs as usize {
                let mut axis_tokens = [T1TokenRec::default(); T1_MAX_MM_AXIS];
                let mut n_axis: FtInt = 0;

                /* read axis/coordinates tokens */
                let token = design_tokens[n];
                parser.root.cursor = token.start();
                parser.root.limit = token.limit();
                t1_to_token_array(
                    parser,
                    Some(&mut axis_tokens),
                    T1_MAX_MM_AXIS as FtUInt,
                    &mut n_axis,
                );

                if n == 0 {
                    if n_axis <= 0 || n_axis as usize > T1_MAX_MM_AXIS {
                        return Err(FT_ERR_INVALID_FILE_FORMAT);
                    }

                    num_axis = n_axis;
                    t1_allocate_blend(face, num_designs as FtUInt, num_axis as FtUInt)?;

                    /* allocate a blend design pos table */
                    design_pos = Some(ft_new_array(num_designs as FtLong * num_axis as FtLong)?);
                } else if n_axis != num_axis {
                    return Err(FT_ERR_INVALID_FILE_FORMAT);
                }

                /* now read each axis token into the design position */
                for axis in 0..n_axis as usize {
                    let token2 = axis_tokens[axis];

                    parser.root.cursor = token2.start();
                    parser.root.limit = token2.limit();
                    let v = t1_to_fixed(parser, 0);
                    if let Some(pos) = design_pos.as_mut() {
                        pos[n * num_axis as usize + axis] = v;
                    }
                }
            }

            loader_cursor_restore(parser, old_cursor, old_limit);

            /* a valid BlendDesignPosition has been parsed */
            let blend = face.blend.as_mut().unwrap();
            blend.design_pos = design_pos.take();
        }

        Ok(())
    })();

    /* Exit: */
    loader.parser.root.error = error_code(error);
}

/// Restores the parser's cursor and limit.
fn loader_cursor_restore(parser: &mut T1ParserRec, cursor: usize, limit: usize) {
    parser.root.cursor = cursor;
    parser.root.limit = limit;
}

/// `parse_blend_design_map`
fn parse_blend_design_map(face: &mut T1FaceRec, loader: &mut T1LoaderRec) {
    let parser = &mut loader.parser;
    let mut axis_tokens = [T1TokenRec::default(); T1_MAX_MM_AXIS];
    let mut num_axis: FtInt = 0;

    let error = (|| -> FtResult<()> {
        t1_to_token_array(
            parser,
            Some(&mut axis_tokens),
            T1_MAX_MM_AXIS as FtUInt,
            &mut num_axis,
        );
        if num_axis < 0 {
            return Err(FT_ERR_IGNORE);
        }
        if num_axis == 0 || num_axis as usize > T1_MAX_MM_AXIS {
            return Err(FT_ERR_INVALID_FILE_FORMAT);
        }

        let old_cursor = parser.root.cursor;
        let old_limit = parser.root.limit;

        t1_allocate_blend(face, 0, num_axis as FtUInt)?;
        let blend = face.blend.as_mut().unwrap();

        /* now read each axis design map */
        for n in 0..num_axis as usize {
            let map = &mut blend.design_map[n];
            let mut point_tokens = [T1TokenRec::default(); T1_MAX_MM_MAP_POINTS];
            let mut num_points: FtInt = 0;

            let axis_token = axis_tokens[n];

            parser.root.cursor = axis_token.start();
            parser.root.limit = axis_token.limit();
            t1_to_token_array(
                parser,
                Some(&mut point_tokens),
                T1_MAX_MM_MAP_POINTS as FtUInt,
                &mut num_points,
            );

            if num_points <= 0 || num_points as usize > T1_MAX_MM_MAP_POINTS {
                return Err(FT_ERR_INVALID_FILE_FORMAT);
            }

            if !map.design_points.is_empty() {
                return Err(FT_ERR_INVALID_FILE_FORMAT);
            }

            /* allocate design map data */
            map.design_points = ft_new_array(num_points as FtLong)?;
            map.blend_points = ft_new_array(num_points as FtLong)?;
            map.num_points = num_points as FtByte;

            for p in 0..num_points as usize {
                let point_token = point_tokens[p];

                /* don't include delimiting brackets */
                parser.root.cursor = point_token.start().wrapping_add(1);
                parser.root.limit = point_token.limit().wrapping_sub(1);

                map.design_points[p] = t1_to_int(parser);
                map.blend_points[p] = t1_to_fixed(parser, 0);
            }
        }

        parser.root.cursor = old_cursor;
        parser.root.limit = old_limit;

        Ok(())
    })();

    /* Exit: */
    loader.parser.root.error = error_code(error);
}

/// `parse_weight_vector`
fn parse_weight_vector(face: &mut T1FaceRec, loader: &mut T1LoaderRec) {
    let mut design_tokens = [T1TokenRec::default(); T1_MAX_MM_DESIGNS];
    let mut num_designs: FtInt = 0;
    let parser = &mut loader.parser;

    let error = (|| -> FtResult<()> {
        t1_to_token_array(
            parser,
            Some(&mut design_tokens),
            T1_MAX_MM_DESIGNS as FtUInt,
            &mut num_designs,
        );
        if num_designs < 0 {
            return Err(FT_ERR_IGNORE);
        }
        if num_designs == 0 || num_designs as usize > T1_MAX_MM_DESIGNS {
            return Err(FT_ERR_INVALID_FILE_FORMAT);
        }

        if face.blend.as_ref().is_none_or(|b| b.num_designs == 0) {
            t1_allocate_blend(face, num_designs as FtUInt, 0)?;
        } else if face.blend.as_ref().unwrap().num_designs != num_designs as FtUInt {
            return Err(FT_ERR_INVALID_FILE_FORMAT);
        }
        let blend = face.blend.as_mut().unwrap();

        /* (C's one block of both vectors) */
        if blend.weight_vector.is_none() {
            let weight_vector = ft_new_array(num_designs as FtLong)?;
            blend.default_weight_vector = ft_new_array(num_designs as FtLong)?;
            blend.weight_vector = Some(weight_vector);
        }

        let old_cursor = parser.root.cursor;
        let old_limit = parser.root.limit;

        for n in 0..num_designs as usize {
            let token = design_tokens[n];
            parser.root.cursor = token.start();
            parser.root.limit = token.limit();

            let v = t1_to_fixed(parser, 0);
            blend.default_weight_vector[n] = v;
            blend.weight_vector.as_mut().unwrap()[n] = v;
        }

        parser.root.cursor = old_cursor;
        parser.root.limit = old_limit;

        Ok(())
    })();

    /* Exit: */
    loader.parser.root.error = error_code(error);
}

/// `parse_buildchar`: e.g., `/BuildCharArray [0 0 0 0 0 0 0 0] def`;
/// we're only interested in the number of array elements
fn parse_buildchar(face: &mut T1FaceRec, loader: &mut T1LoaderRec) {
    face.len_buildchar = t1_to_fixed_array(&mut loader.parser, 0, None, 0) as FtUInt;
}

/*************************************************************************/
/*************************************************************************/
/*****                                                               *****/
/*****                      TYPE 1 SYMBOL PARSING                    *****/
/*****                                                               *****/
/*************************************************************************/
/*************************************************************************/

/// `t1_load_keyword`
fn t1_load_keyword(
    face: &mut T1FaceRec,
    loader: &mut T1LoaderRec,
    field: &T1Field,
) -> FtResult<()> {
    let has_blend = face.blend.as_ref().is_some_and(|b| b.num_designs != 0);

    /* if the keyword has a dedicated callback, call it */
    if field.type_ == T1_FIELD_TYPE_CALLBACK {
        if let Some(reader) = field.reader {
            reader(face, loader);
        }
        return parser_result(loader.parser.root.error);
    }

    /* now, the keyword is either a simple field, or a table of fields; */
    /* we are now going to take care of it                              */
    let T1FaceRec {
        type1,
        blend,
        indices,
        ..
    } = face;

    /* (C's `objects': the records, the first one C's `dummy_object') */
    let mut objects: Vec<T1Object<'_>> = Vec::new();
    let mut max_objects: FtUInt = 0;

    match field.location {
        T1_FIELD_LOCATION_FONT_INFO => {
            objects.push(T1Object::FontInfo(&mut type1.font_info));

            if has_blend {
                let blend = blend.as_mut().unwrap();
                max_objects = blend.num_designs;
                objects.extend(blend.font_infos.iter_mut().map(T1Object::FontInfo));
            }
        }

        T1_FIELD_LOCATION_FONT_EXTRA => {
            objects.push(T1Object::FontExtra(&mut type1.font_extra));
        }

        T1_FIELD_LOCATION_PRIVATE => {
            objects.push(T1Object::Private(&mut type1.private_dict));

            if has_blend {
                let blend = blend.as_mut().unwrap();
                max_objects = blend.num_designs;
                objects.extend(blend.privates.iter_mut().map(T1Object::Private));
            }
        }

        T1_FIELD_LOCATION_BBOX => {
            objects.push(T1Object::BBox(&mut type1.font_bbox));

            if has_blend {
                let blend = blend.as_mut().unwrap();
                max_objects = blend.num_designs;
                objects.extend(blend.bboxes.iter_mut().map(T1Object::BBox));
            }
        }

        T1_FIELD_LOCATION_LOADER => { /* (no Type 1 keyword is a loader field) */ }

        T1_FIELD_LOCATION_FACE => {
            objects.push(T1Object::T1Face(indices));
        }

        T1_FIELD_LOCATION_BLEND => {
            /* (C's `dummy_object' is the face's blend, NULL without one) */
            if let Some(blend) = blend.as_deref_mut() {
                objects.push(T1Object::Blend(blend));
            }
        }

        _ => {
            objects.push(T1Object::Font(type1));
        }
    }

    if !objects.is_empty() {
        if field.type_ == T1_FIELD_TYPE_INTEGER_ARRAY || field.type_ == T1_FIELD_TYPE_FIXED_ARRAY {
            t1_load_field_table(&mut loader.parser, field, &mut objects, max_objects)
        } else {
            t1_load_field(&mut loader.parser, field, &mut objects, max_objects)
        }
    } else {
        Ok(())
    }

    /* Exit: */
}

/// `parse_private`
fn parse_private(_face: &mut T1FaceRec, loader: &mut T1LoaderRec) {
    loader.keywords_encountered |= T1_PRIVATE;
}

/// `read_binary_data` (`base` is an offset into the parsed bytes);
/// returns 1 in case of success
fn read_binary_data(
    parser: &mut T1ParserRec,
    size: &mut FtULong,
    base: &mut usize,
    incremental: bool,
) -> bool {
    let limit = parser.root.limit;

    /* the binary data has one of the following formats */
    /*                                                  */
    /*   `size' [white*] RD white ....... ND            */
    /*   `size' [white*] -| white ....... |-            */
    /*                                                  */

    t1_skip_spaces(parser);

    let cur = parser.root.cursor;

    if cur < limit && at(parser.bytes(), cur).is_ascii_digit() {
        let s: FtLong = t1_to_int(parser);

        t1_skip_ps_token(parser); /* `RD' or `-|' or something else */

        /* there is only one whitespace char after the */
        /* `RD' or `-|' token                          */
        *base = parser.root.cursor + 1;

        if s >= 0 && s < limit as FtLong - *base as FtLong {
            parser.root.cursor += s as usize + 1;
            *size = s as FtULong;
            return parser.root.error == 0;
        }
    }

    if !incremental {
        parser.root.error = FT_ERR_INVALID_FILE_FORMAT;
    }

    false
}

/* We now define the routines to handle the `/Encoding', `/Subrs', */
/* and `/CharStrings' dictionaries.                                */

/// `t1_parse_font_matrix`
fn t1_parse_font_matrix(face: &mut T1FaceRec, loader: &mut T1LoaderRec) {
    let parser = &mut loader.parser;
    let mut temp: [FtFixed; 6] = [0; 6];

    /* input is scaled by 1000 to accommodate default FontMatrix */
    let result = t1_to_fixed_array(parser, 6, Some(&mut temp), 3);

    if result < 6 {
        parser.root.error = FT_ERR_INVALID_FILE_FORMAT;
        return;
    }

    let temp_scale: FtFixed = temp[3].abs();

    if temp_scale == 0 {
        parser.root.error = FT_ERR_INVALID_FILE_FORMAT;
        return;
    }

    /* atypical case */
    if temp_scale != 0x10000 {
        /* set units per EM based on FontMatrix values */
        face.root.units_per_EM = ft_div_fix(1000, temp_scale) as FtUShort;

        temp[0] = ft_div_fix(temp[0], temp_scale);
        temp[1] = ft_div_fix(temp[1], temp_scale);
        temp[2] = ft_div_fix(temp[2], temp_scale);
        temp[4] = ft_div_fix(temp[4], temp_scale);
        temp[5] = ft_div_fix(temp[5], temp_scale);
        temp[3] = if temp[3] < 0 { -0x10000 } else { 0x10000 };
    }

    let matrix = &mut face.type1.font_matrix;
    matrix.xx = temp[0];
    matrix.yx = temp[1];
    matrix.xy = temp[2];
    matrix.yy = temp[3];

    if !ft_matrix_check(matrix) {
        parser.root.error = FT_ERR_INVALID_FILE_FORMAT;
        return;
    }

    /* note that the offsets must be expressed in integer font units */
    let offset = &mut face.type1.font_offset;
    offset.x = temp[4] >> 16;
    offset.y = temp[5] >> 16;
}

/// `parse_encoding`
fn parse_encoding(face: &mut T1FaceRec, loader: &mut T1LoaderRec) {
    let limit = loader.parser.root.limit;

    t1_skip_spaces(&mut loader.parser);
    let mut cur = loader.parser.root.cursor;
    if cur >= limit {
        loader.parser.root.error = FT_ERR_INVALID_FILE_FORMAT;
        return;
    }

    let c = at(loader.parser.bytes(), cur);

    /* if we have a number or `[', the encoding is an array, */
    /* and we must load it now                               */
    if c.is_ascii_digit() || c == b'[' {
        let encode = &mut face.type1.encoding;
        let char_table = &mut loader.encoding_table;
        let parser = &mut loader.parser;
        let count: FtInt;
        let mut array_size: FtInt;
        let mut only_immediates = false;

        /* read the number of entries in the encoding; should be 256 */
        if c == b'[' {
            count = 256;
            only_immediates = true;
            parser.root.cursor += 1;
        } else {
            count = t1_to_int(parser) as FtInt;
        }

        array_size = count;
        if count > 256 {
            array_size = 256;
        }

        t1_skip_spaces(parser);
        if parser.root.cursor >= limit {
            return;
        }

        /* PostScript happily allows overwriting of encoding arrays */
        if !encode.char_index.is_empty() {
            encode.char_index = Vec::new();
            encode.char_name = Vec::new();
            t1_release_table(char_table);
        }

        /* we use a T1_Table to store our charnames */
        loader.num_chars = array_size;
        encode.num_chars = array_size;
        let r = (|| -> FtResult<()> {
            encode.char_index = ft_new_array(array_size as FtLong)?;
            encode.char_name = ft_new_array(array_size as FtLong)?;
            ps_table_new(char_table, array_size)
        })();
        if let Err(error) = r {
            parser.root.error = error;
            return;
        }

        /* We need to `zero' out encoding_table.elements */
        for n in 0..array_size {
            let _ = t1_add_table(char_table, n, b".notdef\0", 8);
        }

        /* Now we need to read records of the form                */
        /*                                                        */
        /*   ... charcode /charname ...                           */
        /*                                                        */
        /* for each entry in our table.                           */
        /*                                                        */
        /* We simply look for a number followed by an immediate   */
        /* name.  Note that this ignores correctly the sequence   */
        /* that is often seen in type1 fonts:                     */
        /*                                                        */
        /*   0 1 255 { 1 index exch /.notdef put } for dup        */
        /*                                                        */
        /* used to clean the encoding array before anything else. */
        /*                                                        */
        /* Alternatively, if the array is directly given as       */
        /*                                                        */
        /*   /Encoding [ ... ]                                    */
        /*                                                        */
        /* we only read immediates.                               */

        let mut n: FtInt = 0;
        t1_skip_spaces(parser);

        while parser.root.cursor < limit {
            cur = parser.root.cursor;
            let b = parser.bytes();

            /* we stop when we encounter a `def' or `]' */
            if at(b, cur) == b'd'
                && cur + 3 < limit
                && at(b, cur + 1) == b'e'
                && at(b, cur + 2) == b'f'
                && is_ps_delim(at(b, cur + 3))
            {
                cur += 3;
                break;
            }
            if at(b, cur) == b']' {
                cur += 1;
                break;
            }

            /* check whether we've found an entry */
            if at(b, cur).is_ascii_digit() || only_immediates {
                let charcode: FtInt;

                if only_immediates {
                    charcode = n;
                } else {
                    charcode = t1_to_int(parser) as FtInt;
                    t1_skip_spaces(parser);

                    /* protect against invalid charcode */
                    if cur == parser.root.cursor {
                        parser.root.error = FT_ERR_UNKNOWN_FILE_FORMAT;
                        return;
                    }
                }

                cur = parser.root.cursor;

                if cur + 2 < limit && at(parser.bytes(), cur) == b'/' && n < count {
                    cur += 1;

                    parser.root.cursor = cur;
                    t1_skip_ps_token(parser);
                    if parser.root.cursor >= limit {
                        return;
                    }
                    if parser.root.error != 0 {
                        return;
                    }

                    let len = parser.root.cursor - cur;

                    if n < array_size {
                        let b = parser.bytes();
                        let src = b.get(cur..(cur + len + 1).min(b.len())).unwrap_or(&[]);
                        if let Err(error) =
                            t1_add_table(char_table, charcode, src, len as FtUInt + 1)
                        {
                            parser.root.error = error;
                            return;
                        }
                        if let Some(start) = char_table.elements[charcode as usize] {
                            char_table.block[start + len] = b'\0';
                        }
                    }

                    n += 1;
                } else if only_immediates {
                    /* Since the current position is not updated for           */
                    /* immediates-only mode we would get an infinite loop if   */
                    /* we don't do anything here.                              */
                    /*                                                         */
                    /* This encoding array is not valid according to the type1 */
                    /* specification (it might be an encoding for a CID type1  */
                    /* font, however), so we conclude that this font is NOT a  */
                    /* type1 font.                                             */
                    parser.root.error = FT_ERR_UNKNOWN_FILE_FORMAT;
                    return;
                }
            } else {
                t1_skip_ps_token(parser);
                if parser.root.error != 0 {
                    return;
                }
            }

            t1_skip_spaces(parser);
        }

        face.type1.encoding_type = T1_ENCODING_TYPE_ARRAY;
        parser.root.cursor = cur;
    }
    /* Otherwise, we should have either `StandardEncoding', */
    /* `ExpertEncoding', or `ISOLatin1Encoding'             */
    else {
        let b = loader.parser.bytes();
        let starts = |s: &[u8]| b.get(cur..cur + s.len()) == Some(s);

        if cur + 17 < limit && starts(b"StandardEncoding") {
            face.type1.encoding_type = T1_ENCODING_TYPE_STANDARD;
        } else if cur + 15 < limit && starts(b"ExpertEncoding") {
            face.type1.encoding_type = T1_ENCODING_TYPE_EXPERT;
        } else if cur + 18 < limit && starts(b"ISOLatin1Encoding") {
            face.type1.encoding_type = T1_ENCODING_TYPE_ISOLATIN1;
        } else {
            loader.parser.root.error = FT_ERR_IGNORE;
        }
    }
}

/// `parse_subrs`
fn parse_subrs(face: &mut T1FaceRec, loader: &mut T1LoaderRec) {
    let error = parse_subrs_body(face, loader);

    if let Err(error) = error {
        /* Fail: */
        loader.parser.root.error = error;
    }
}

fn parse_subrs_body(face: &mut T1FaceRec, loader: &mut T1LoaderRec) -> FtResult<()> {
    let parser = &mut loader.parser;
    let table = &mut loader.subrs;

    t1_skip_spaces(parser);

    /* test for empty array */
    if parser.root.cursor < parser.root.limit && at(parser.bytes(), parser.root.cursor) == b'[' {
        t1_skip_ps_token(parser);
        t1_skip_spaces(parser);
        if parser.root.cursor >= parser.root.limit || at(parser.bytes(), parser.root.cursor) != b']'
        {
            parser.root.error = FT_ERR_INVALID_FILE_FORMAT;
        }
        return Ok(());
    }

    let mut num_subrs: FtInt = t1_to_int(parser) as FtInt;
    if num_subrs < 0 {
        parser.root.error = FT_ERR_INVALID_FILE_FORMAT;
        return Ok(());
    }

    /* we certainly need more than 8 bytes per subroutine */
    if parser.root.limit >= parser.root.cursor
        && num_subrs as FtLong > ((parser.root.limit - parser.root.cursor) >> 3) as FtLong
    {
        /*
         * There are two possibilities.  Either the font contains an invalid
         * value for `num_subrs', or we have a subsetted font where the
         * subroutine indices are not adjusted, e.g.
         *
         *   /Subrs 812 array
         *     dup 0 { ... } NP
         *     dup 51 { ... } NP
         *     dup 681 { ... } NP
         *   ND
         *
         * In both cases, we use a number hash that maps from subr indices to
         * actual array elements.
         */

        num_subrs = ((parser.root.limit - parser.root.cursor) >> 3) as FtInt;

        if loader.subrs_hash.is_none() {
            loader.subrs_hash = Some(HashMap::new());
        }
    }

    /* position the parser right before the `dup' of the first subr */
    t1_skip_ps_token(parser); /* `array' */
    if parser.root.error != 0 {
        return Ok(());
    }
    t1_skip_spaces(parser);

    /* initialize subrs array -- with synthetic fonts it is possible */
    /* we get here twice                                             */
    if loader.num_subrs == 0 {
        ps_table_new(table, num_subrs)?;
    }

    /* the format is simple:   */
    /*                         */
    /*   `index' + binary data */
    /*                         */
    let mut count: FtUInt = 0;
    loop {
        let mut idx: FtLong;
        let mut size: FtULong = 0;
        let mut base: usize = 0;

        /* If we are out of data, or if the next token isn't `dup', */
        /* we are done.                                             */
        if parser.root.cursor + 4 >= parser.root.limit
            || parser
                .bytes()
                .get(parser.root.cursor..parser.root.cursor + 3)
                != Some(b"dup")
        {
            break;
        }

        t1_skip_ps_token(parser); /* `dup' */

        idx = t1_to_int(parser);

        if !read_binary_data(parser, &mut size, &mut base, false) {
            return Ok(());
        }

        /* The binary string is followed by one token, e.g. `NP' */
        /* (bound to `noaccess put') or by two separate tokens:  */
        /* `noaccess' & `put'.  We position the parser right     */
        /* before the next `dup', if any.                        */
        t1_skip_ps_token(parser); /* `NP' or `|' or `noaccess' */
        if parser.root.error != 0 {
            return Ok(());
        }
        t1_skip_spaces(parser);

        if parser.root.cursor + 4 < parser.root.limit
            && parser
                .bytes()
                .get(parser.root.cursor..parser.root.cursor + 3)
                == Some(b"put")
        {
            t1_skip_ps_token(parser); /* skip `put' */
            t1_skip_spaces(parser);
        }

        /* if we use a hash, the subrs index is the key, and a running */
        /* counter specified for `T1_Add_Table' acts as the value      */
        if let Some(hash) = loader.subrs_hash.as_mut() {
            /* (an insertion that fails to allocate is dropped, as C */
            /* ignores `ft_hash_num_insert''s error)                 */
            if hash.try_reserve(1).is_ok() {
                hash.insert(idx as FtInt, count as usize);
            }
            idx = count as FtLong;
        }

        count += 1;

        /* with synthetic fonts it is possible we get here twice */
        if loader.num_subrs != 0 {
            continue;
        }

        /* some fonts use a value of -1 for lenIV to indicate that */
        /* the charstrings are unencoded                           */
        /*                                                         */
        /* thanks to Tom Kacvinsky for pointing this out           */
        /*                                                         */
        let len_iv = face.type1.private_dict.lenIV;
        let b = parser.bytes();
        let data = b.get(base..base + size as usize).unwrap_or(&[]);
        if len_iv >= 0 {
            /* some fonts define empty subr records -- this is not totally */
            /* compliant to the specification (which says they should at   */
            /* least contain a `return'), but we support them anyway       */
            if size < len_iv as FtULong {
                return Err(FT_ERR_INVALID_FILE_FORMAT);
            }

            /* t1_decrypt() shouldn't write to base -- make temporary copy */
            let mut temp = ft_qalloc(size as FtLong)?;
            temp.copy_from_slice(data);
            t1_decrypt(&mut temp, size as usize, 4330);
            size -= len_iv as FtULong;
            t1_add_table(
                table,
                idx as FtInt,
                &temp[len_iv as usize..],
                size as FtUInt,
            )?;
        } else {
            t1_add_table(table, idx as FtInt, data, size as FtUInt)?;
        }
    }

    if loader.num_subrs == 0 {
        loader.num_subrs = num_subrs;
    }

    Ok(())
}

const TABLE_EXTEND: FtInt = 5;

/// `parse_charstrings`
fn parse_charstrings(face: &mut T1FaceRec, loader: &mut T1LoaderRec) {
    if let Err(error) = parse_charstrings_body(face, loader) {
        /* Fail: */
        loader.parser.root.error = error;
    }
}

fn parse_charstrings_body(face: &mut T1FaceRec, loader: &mut T1LoaderRec) -> FtResult<()> {
    let T1LoaderRec {
        parser,
        charstrings: code_table,
        glyph_names: name_table,
        swap_table,
        num_glyphs: loader_num_glyphs,
        ..
    } = loader;

    let mut cur = parser.root.cursor;
    let limit = parser.root.limit;
    let mut notdef_index: FtInt = 0;
    let mut notdef_found = false;

    let mut num_glyphs: FtInt = t1_to_int(parser) as FtInt;
    if num_glyphs < 0 {
        return Err(FT_ERR_INVALID_FILE_FORMAT);
    }

    /* we certainly need more than 8 bytes per glyph */
    if num_glyphs as FtLong > (limit as FtLong - cur as FtLong) >> 3 {
        num_glyphs = ((limit as FtLong - cur as FtLong) >> 3) as FtInt;
    }

    /* some fonts like Optima-Oblique not only define the /CharStrings */
    /* array but access it also                                        */
    if num_glyphs == 0 || parser.root.error != 0 {
        return Ok(());
    }

    /* initialize tables, leaving space for addition of .notdef, */
    /* if necessary, and a few other glyphs to handle buggy      */
    /* fonts which have more glyphs than specified.              */

    /* for some non-standard fonts like `Optima' which provides  */
    /* different outlines depending on the resolution it is      */
    /* possible to get here twice                                */
    if *loader_num_glyphs == 0 {
        ps_table_new(code_table, num_glyphs + 1 + TABLE_EXTEND)?;

        ps_table_new(name_table, num_glyphs + 1 + TABLE_EXTEND)?;

        /* Initialize table for swapping index notdef_index and */
        /* index 0 names and codes (if necessary).              */

        ps_table_new(swap_table, 4)?;
    }

    let mut n: FtInt = 0;

    loop {
        let mut size: FtULong = 0;
        let mut base: usize = 0;

        /* the format is simple:        */
        /*   `/glyphname' + binary data */

        t1_skip_spaces(parser);

        cur = parser.root.cursor;
        if cur >= limit {
            break;
        }

        let b = parser.bytes();

        /* we stop when we find a `def' or `end' keyword */
        if cur + 3 < limit && is_ps_delim(at(b, cur + 3)) {
            if at(b, cur) == b'd' && at(b, cur + 1) == b'e' && at(b, cur + 2) == b'f' {
                /* There are fonts which have this: */
                /*                                  */
                /*   /CharStrings 118 dict def      */
                /*   Private begin                  */
                /*   CharStrings begin              */
                /*   ...                            */
                /*                                  */
                /* To catch this we ignore `def' if */
                /* no charstring has actually been  */
                /* seen.                            */
                if n != 0 {
                    break;
                }
            }

            if at(b, cur) == b'e' && at(b, cur + 1) == b'n' && at(b, cur + 2) == b'd' {
                break;
            }
        }

        t1_skip_ps_token(parser);
        if parser.root.cursor >= limit {
            return Err(FT_ERR_INVALID_FILE_FORMAT);
        }
        if parser.root.error != 0 {
            return Ok(());
        }

        if at(parser.bytes(), cur) == b'/' {
            if cur + 2 >= limit {
                return Err(FT_ERR_INVALID_FILE_FORMAT);
            }

            cur += 1; /* skip `/' */
            let len = parser.root.cursor - cur;

            if !read_binary_data(parser, &mut size, &mut base, false) {
                return Ok(());
            }

            /* for some non-standard fonts like `Optima' which provides */
            /* different outlines depending on the resolution it is     */
            /* possible to get here twice                               */
            if *loader_num_glyphs != 0 {
                continue;
            }

            let b = parser.bytes();
            let name = b.get(cur..(cur + len + 1).min(b.len())).unwrap_or(&[]);
            t1_add_table(name_table, n, name, len as FtUInt + 1)?;

            /* add a trailing zero to the name table */
            if let Some(start) = name_table.elements[n as usize] {
                name_table.block[start + len] = b'\0';
            }

            /* record index of /.notdef */
            if at(b, cur) == b'.' && name_table.element(n as usize).map(c_str) == Some(b".notdef") {
                notdef_index = n;
                notdef_found = true;
            }

            let len_iv = face.type1.private_dict.lenIV;
            let data = b.get(base..base + size as usize).unwrap_or(&[]);
            if len_iv >= 0 && n < num_glyphs + TABLE_EXTEND {
                if size <= len_iv as FtULong {
                    return Err(FT_ERR_INVALID_FILE_FORMAT);
                }

                /* t1_decrypt() shouldn't write to base -- make temporary copy */
                let mut temp = ft_qalloc(size as FtLong)?;
                temp.copy_from_slice(data);
                t1_decrypt(&mut temp, size as usize, 4330);
                size -= len_iv as FtULong;
                t1_add_table(code_table, n, &temp[len_iv as usize..], size as FtUInt)?;
            } else {
                t1_add_table(code_table, n, data, size as FtUInt)?;
            }

            n += 1;
        }
    }

    if n == 0 {
        return Err(FT_ERR_INVALID_FILE_FORMAT);
    }

    *loader_num_glyphs = n;

    /* the element `i' of a table and its length */
    fn elem(table: &PsTableRec, i: usize) -> (&[u8], FtUInt) {
        (
            table.element(i).unwrap_or(&[]),
            table.lengths.get(i).copied().unwrap_or(0),
        )
    }

    /* if /.notdef is found but does not occupy index 0, do our magic. */
    if notdef_found && name_table.element(0).map(c_str) != Some(b".notdef") {
        /* Swap glyph in index 0 with /.notdef glyph.  First, add index 0  */
        /* name and code entries to swap_table.  Then place notdef_index   */
        /* name and code entries into swap_table.  Then swap name and code */
        /* entries at indices notdef_index and 0 using values stored in    */
        /* swap_table.                                                     */

        /* Index 0 name */
        let (o, l) = elem(name_table, 0);
        t1_add_table(swap_table, 0, o, l)?;

        /* Index 0 code */
        let (o, l) = elem(code_table, 0);
        t1_add_table(swap_table, 1, o, l)?;

        /* Index notdef_index name */
        let (o, l) = elem(name_table, notdef_index as usize);
        t1_add_table(swap_table, 2, o, l)?;

        /* Index notdef_index code */
        let (o, l) = elem(code_table, notdef_index as usize);
        t1_add_table(swap_table, 3, o, l)?;

        let (o, l) = elem(swap_table, 0);
        t1_add_table(name_table, notdef_index, o, l)?;

        let (o, l) = elem(swap_table, 1);
        t1_add_table(code_table, notdef_index, o, l)?;

        let (o, l) = elem(swap_table, 2);
        t1_add_table(name_table, 0, o, l)?;

        let (o, l) = elem(swap_table, 3);
        t1_add_table(code_table, 0, o, l)?;
    } else if !notdef_found {
        /* notdef_index is already 0, or /.notdef is undefined in   */
        /* charstrings dictionary.  Worry about /.notdef undefined. */
        /* We take index 0 and add it to the end of the table(s)    */
        /* and add our own /.notdef glyph to index 0.               */

        /* 0 333 hsbw endchar */
        let notdef_glyph: [FtByte; 5] = [0x8B, 0xF7, 0xE1, 0x0D, 0x0E];

        let (o, l) = elem(name_table, 0);
        t1_add_table(swap_table, 0, o, l)?;

        let (o, l) = elem(code_table, 0);
        t1_add_table(swap_table, 1, o, l)?;

        t1_add_table(name_table, 0, b".notdef\0", 8)?;

        t1_add_table(code_table, 0, &notdef_glyph, 5)?;

        let (o, l) = elem(swap_table, 0);
        t1_add_table(name_table, n, o, l)?;

        let (o, l) = elem(swap_table, 1);
        t1_add_table(code_table, n, o, l)?;

        /* we added a glyph. */
        *loader_num_glyphs += 1;
    }

    Ok(())
}

/// The C string at the start of `s` (up to its first null byte).
fn c_str(s: &[u8]) -> &[u8] {
    let len = s.iter().position(|&c| c == 0).unwrap_or(s.len());
    &s[..len]
}

/// The bounding box field (`T1_FIELD_BBOX`: the whole record, at the
/// offset of its `xMin`).
macro_rules! t1_bbox_field {
    ($ident:expr, $dict:expr) => {
        T1FieldRec {
            ident: $ident,
            location: T1_FIELD_LOCATION_BBOX,
            type_: T1_FIELD_TYPE_BBOX,
            reader: None,
            access: Some({
                fn access<'a, 'b>(o: &'a mut T1Object<'b>) -> T1FieldRef<'a> {
                    match o {
                        T1Object::BBox(r) => T1FieldRef::BBox(r),
                        _ => T1FieldRef::None,
                    }
                }
                access
            }),
            array_max: 0,
            dict: $dict,
        }
    };
}

/// `t1_keywords`: the token field static variables (`t1tokens.h` and
/// the special functions).
pub static T1_KEYWORDS: [T1Field; 47] = [
    /* t1tokens.h */

    /* FT_STRUCTURE PS_FontInfoRec, T1CODE T1_FIELD_LOCATION_FONT_INFO */
    t1_field!(
        "version",
        T1_FIELD_LOCATION_FONT_INFO,
        T1_FIELD_TYPE_STRING,
        FontInfo,
        version,
        String,
        0,
        T1_FIELD_DICT_FONTDICT
    ),
    t1_field!(
        "Notice",
        T1_FIELD_LOCATION_FONT_INFO,
        T1_FIELD_TYPE_STRING,
        FontInfo,
        notice,
        String,
        0,
        T1_FIELD_DICT_FONTDICT
    ),
    t1_field!(
        "FullName",
        T1_FIELD_LOCATION_FONT_INFO,
        T1_FIELD_TYPE_STRING,
        FontInfo,
        full_name,
        String,
        0,
        T1_FIELD_DICT_FONTDICT
    ),
    t1_field!(
        "FamilyName",
        T1_FIELD_LOCATION_FONT_INFO,
        T1_FIELD_TYPE_STRING,
        FontInfo,
        family_name,
        String,
        0,
        T1_FIELD_DICT_FONTDICT
    ),
    t1_field!(
        "Weight",
        T1_FIELD_LOCATION_FONT_INFO,
        T1_FIELD_TYPE_STRING,
        FontInfo,
        weight,
        String,
        0,
        T1_FIELD_DICT_FONTDICT
    ),
    /* we use pointers to detect modifications made by synthetic fonts */
    t1_field!(
        "ItalicAngle",
        T1_FIELD_LOCATION_FONT_INFO,
        T1_FIELD_TYPE_INTEGER,
        FontInfo,
        italic_angle,
        Long,
        0,
        T1_FIELD_DICT_FONTDICT
    ),
    t1_field!(
        "isFixedPitch",
        T1_FIELD_LOCATION_FONT_INFO,
        T1_FIELD_TYPE_BOOL,
        FontInfo,
        is_fixed_pitch,
        Byte,
        0,
        T1_FIELD_DICT_FONTDICT
    ),
    t1_field!(
        "UnderlinePosition",
        T1_FIELD_LOCATION_FONT_INFO,
        T1_FIELD_TYPE_INTEGER,
        FontInfo,
        underline_position,
        Short,
        0,
        T1_FIELD_DICT_FONTDICT
    ),
    t1_field!(
        "UnderlineThickness",
        T1_FIELD_LOCATION_FONT_INFO,
        T1_FIELD_TYPE_INTEGER,
        FontInfo,
        underline_thickness,
        UShort,
        0,
        T1_FIELD_DICT_FONTDICT
    ),
    /* FT_STRUCTURE PS_FontExtraRec, T1CODE T1_FIELD_LOCATION_FONT_EXTRA */
    t1_field!(
        "FSType",
        T1_FIELD_LOCATION_FONT_EXTRA,
        T1_FIELD_TYPE_INTEGER,
        FontExtra,
        fs_type,
        UShort,
        0,
        T1_FIELD_DICT_FONTDICT
    ),
    /* FT_STRUCTURE PS_PrivateRec, T1CODE T1_FIELD_LOCATION_PRIVATE */
    t1_field!(
        "UniqueID",
        T1_FIELD_LOCATION_PRIVATE,
        T1_FIELD_TYPE_INTEGER,
        Private,
        unique_id,
        Int,
        0,
        T1_FIELD_DICT_FONTDICT | T1_FIELD_DICT_PRIVATE
    ),
    t1_field!(
        "lenIV",
        T1_FIELD_LOCATION_PRIVATE,
        T1_FIELD_TYPE_INTEGER,
        Private,
        lenIV,
        Int,
        0,
        T1_FIELD_DICT_PRIVATE
    ),
    t1_field!(
        "LanguageGroup",
        T1_FIELD_LOCATION_PRIVATE,
        T1_FIELD_TYPE_INTEGER,
        Private,
        language_group,
        Long,
        0,
        T1_FIELD_DICT_PRIVATE
    ),
    t1_field!(
        "password",
        T1_FIELD_LOCATION_PRIVATE,
        T1_FIELD_TYPE_INTEGER,
        Private,
        password,
        Long,
        0,
        T1_FIELD_DICT_PRIVATE
    ),
    t1_field!(
        "BlueScale",
        T1_FIELD_LOCATION_PRIVATE,
        T1_FIELD_TYPE_FIXED_1000,
        Private,
        blue_scale,
        Long,
        0,
        T1_FIELD_DICT_PRIVATE
    ),
    t1_field!(
        "BlueShift",
        T1_FIELD_LOCATION_PRIVATE,
        T1_FIELD_TYPE_INTEGER,
        Private,
        blue_shift,
        Int,
        0,
        T1_FIELD_DICT_PRIVATE
    ),
    t1_field!(
        "BlueFuzz",
        T1_FIELD_LOCATION_PRIVATE,
        T1_FIELD_TYPE_INTEGER,
        Private,
        blue_fuzz,
        Int,
        0,
        T1_FIELD_DICT_PRIVATE
    ),
    t1_table_field!(
        "BlueValues",
        T1_FIELD_LOCATION_PRIVATE,
        T1_FIELD_TYPE_INTEGER_ARRAY,
        Private,
        blue_values,
        Shorts,
        num_blue_values,
        Byte,
        14,
        T1_FIELD_DICT_PRIVATE
    ),
    t1_table_field!(
        "OtherBlues",
        T1_FIELD_LOCATION_PRIVATE,
        T1_FIELD_TYPE_INTEGER_ARRAY,
        Private,
        other_blues,
        Shorts,
        num_other_blues,
        Byte,
        10,
        T1_FIELD_DICT_PRIVATE
    ),
    t1_table_field!(
        "FamilyBlues",
        T1_FIELD_LOCATION_PRIVATE,
        T1_FIELD_TYPE_INTEGER_ARRAY,
        Private,
        family_blues,
        Shorts,
        num_family_blues,
        Byte,
        14,
        T1_FIELD_DICT_PRIVATE
    ),
    t1_table_field!(
        "FamilyOtherBlues",
        T1_FIELD_LOCATION_PRIVATE,
        T1_FIELD_TYPE_INTEGER_ARRAY,
        Private,
        family_other_blues,
        Shorts,
        num_family_other_blues,
        Byte,
        10,
        T1_FIELD_DICT_PRIVATE
    ),
    t1_table_field2!(
        "StdHW",
        T1_FIELD_LOCATION_PRIVATE,
        T1_FIELD_TYPE_INTEGER_ARRAY,
        Private,
        standard_width,
        UShorts,
        1,
        T1_FIELD_DICT_PRIVATE
    ),
    t1_table_field2!(
        "StdVW",
        T1_FIELD_LOCATION_PRIVATE,
        T1_FIELD_TYPE_INTEGER_ARRAY,
        Private,
        standard_height,
        UShorts,
        1,
        T1_FIELD_DICT_PRIVATE
    ),
    t1_table_field2!(
        "MinFeature",
        T1_FIELD_LOCATION_PRIVATE,
        T1_FIELD_TYPE_INTEGER_ARRAY,
        Private,
        min_feature,
        Shorts,
        2,
        T1_FIELD_DICT_PRIVATE
    ),
    t1_table_field!(
        "StemSnapH",
        T1_FIELD_LOCATION_PRIVATE,
        T1_FIELD_TYPE_INTEGER_ARRAY,
        Private,
        snap_widths,
        Shorts,
        num_snap_widths,
        Byte,
        12,
        T1_FIELD_DICT_PRIVATE
    ),
    t1_table_field!(
        "StemSnapV",
        T1_FIELD_LOCATION_PRIVATE,
        T1_FIELD_TYPE_INTEGER_ARRAY,
        Private,
        snap_heights,
        Shorts,
        num_snap_heights,
        Byte,
        12,
        T1_FIELD_DICT_PRIVATE
    ),
    t1_field!(
        "ExpansionFactor",
        T1_FIELD_LOCATION_PRIVATE,
        T1_FIELD_TYPE_FIXED,
        Private,
        expansion_factor,
        Long,
        0,
        T1_FIELD_DICT_PRIVATE
    ),
    t1_field!(
        "ForceBold",
        T1_FIELD_LOCATION_PRIVATE,
        T1_FIELD_TYPE_BOOL,
        Private,
        force_bold,
        Byte,
        0,
        T1_FIELD_DICT_PRIVATE
    ),
    /* FT_STRUCTURE T1_FontRec, T1CODE T1_FIELD_LOCATION_FONT_DICT */
    t1_field!(
        "FontName",
        T1_FIELD_LOCATION_FONT_DICT,
        T1_FIELD_TYPE_KEY,
        Font,
        font_name,
        String,
        0,
        T1_FIELD_DICT_FONTDICT
    ),
    t1_field!(
        "PaintType",
        T1_FIELD_LOCATION_FONT_DICT,
        T1_FIELD_TYPE_INTEGER,
        Font,
        paint_type,
        Byte,
        0,
        T1_FIELD_DICT_FONTDICT
    ),
    t1_field!(
        "FontType",
        T1_FIELD_LOCATION_FONT_DICT,
        T1_FIELD_TYPE_INTEGER,
        Font,
        font_type,
        Byte,
        0,
        T1_FIELD_DICT_FONTDICT
    ),
    t1_field!(
        "StrokeWidth",
        T1_FIELD_LOCATION_FONT_DICT,
        T1_FIELD_TYPE_FIXED,
        Font,
        stroke_width,
        Long,
        0,
        T1_FIELD_DICT_FONTDICT
    ),
    /* FT_STRUCTURE FT_BBox, T1CODE T1_FIELD_LOCATION_BBOX */
    t1_bbox_field!("FontBBox", T1_FIELD_DICT_FONTDICT),
    /* (T1_CONFIG_OPTION_NO_MM_SUPPORT is undefined) */

    /* FT_STRUCTURE T1_FaceRec, T1CODE T1_FIELD_LOCATION_FACE */
    t1_field!(
        "NDV",
        T1_FIELD_LOCATION_FACE,
        T1_FIELD_TYPE_INTEGER,
        T1Face,
        ndv_idx,
        Int,
        0,
        T1_FIELD_DICT_PRIVATE
    ),
    t1_field!(
        "CDV",
        T1_FIELD_LOCATION_FACE,
        T1_FIELD_TYPE_INTEGER,
        T1Face,
        cdv_idx,
        Int,
        0,
        T1_FIELD_DICT_PRIVATE
    ),
    /* FT_STRUCTURE PS_BlendRec, T1CODE T1_FIELD_LOCATION_BLEND */
    t1_table_field!(
        "DesignVector",
        T1_FIELD_LOCATION_BLEND,
        T1_FIELD_TYPE_INTEGER_ARRAY,
        Blend,
        default_design_vector,
        UInts,
        num_default_design_vector,
        UInt,
        T1_MAX_MM_DESIGNS as FtUInt,
        T1_FIELD_DICT_FONTDICT
    ),
    /* now add the special functions... */
    t1_callback_field!(
        "FontMatrix",
        t1_parse_font_matrix as T1FieldReader,
        T1_FIELD_DICT_FONTDICT
    ),
    t1_callback_field!(
        "Encoding",
        parse_encoding as T1FieldReader,
        T1_FIELD_DICT_FONTDICT
    ),
    t1_callback_field!("Subrs", parse_subrs as T1FieldReader, T1_FIELD_DICT_PRIVATE),
    t1_callback_field!(
        "CharStrings",
        parse_charstrings as T1FieldReader,
        T1_FIELD_DICT_PRIVATE
    ),
    t1_callback_field!(
        "Private",
        parse_private as T1FieldReader,
        T1_FIELD_DICT_FONTDICT
    ),
    t1_callback_field!(
        "BlendDesignPositions",
        parse_blend_design_positions as T1FieldReader,
        T1_FIELD_DICT_FONTDICT
    ),
    t1_callback_field!(
        "BlendDesignMap",
        parse_blend_design_map as T1FieldReader,
        T1_FIELD_DICT_FONTDICT
    ),
    t1_callback_field!(
        "BlendAxisTypes",
        parse_blend_axis_types as T1FieldReader,
        T1_FIELD_DICT_FONTDICT
    ),
    t1_callback_field!(
        "WeightVector",
        parse_weight_vector as T1FieldReader,
        T1_FIELD_DICT_FONTDICT
    ),
    t1_callback_field!(
        "BuildCharArray",
        parse_buildchar as T1FieldReader,
        T1_FIELD_DICT_PRIVATE
    ),
    /* (the terminating NULL entry) */
    t1_end_field(),
];

/// The table's terminating entry (C's NULL `ident` is empty).
const fn t1_end_field() -> T1Field {
    T1FieldRec {
        ident: "",
        location: T1_FIELD_LOCATION_CID_INFO,
        type_: T1_FIELD_TYPE_NONE,
        reader: None,
        access: None,
        array_max: 0,
        dict: 0,
    }
}

/// `parse_dict` (`dict` is the dictionary, of `size` bytes)
fn parse_dict(
    face: &mut T1FaceRec,
    loader: &mut T1LoaderRec,
    dict: T1Dict,
    size: FtULong,
) -> FtResult<()> {
    let mut start_binary: usize = 0;
    let mut have_integer = false;

    loader.parser.dict = dict;
    loader.parser.root.cursor = 0;
    loader.parser.root.limit = size as usize;
    loader.parser.root.error = FT_ERR_OK;

    let limit = loader.parser.root.limit;

    t1_skip_spaces(&mut loader.parser);

    'exit: {
        while loader.parser.root.cursor < limit {
            let mut cur = loader.parser.root.cursor;
            let b = loader.parser.bytes();

            /* look for `eexec' */
            if is_ps_token(b, cur, limit, b"eexec") {
                break;
            }
            /* look for `closefile' which ends the eexec section */
            else if is_ps_token(b, cur, limit, b"closefile") {
                break;
            }
            /* in a synthetic font the base font starts after a           */
            /* `FontDictionary' token that is placed after a Private dict */
            else if is_ps_token(b, cur, limit, b"FontDirectory") {
                if loader.keywords_encountered & T1_PRIVATE != 0 {
                    loader.keywords_encountered |= T1_FONTDIR_AFTER_PRIVATE;
                }
                loader.parser.root.cursor += 13;
            }
            /* check whether we have an integer */
            else if at(b, cur).is_ascii_digit() {
                start_binary = cur;
                t1_skip_ps_token(&mut loader.parser);
                if loader.parser.root.error != 0 {
                    break 'exit;
                }
                have_integer = true;
            }
            /* in valid Type 1 fonts we don't see `RD' or `-|' directly */
            /* since those tokens are handled by parse_subrs and        */
            /* parse_charstrings                                        */
            else if at(b, cur) == b'R'
                && cur + 6 < limit
                && at(b, cur + 1) == b'D'
                && have_integer
            {
                let mut s: FtULong = 0;
                let mut bb: usize = 0;

                loader.parser.root.cursor = start_binary;
                if !read_binary_data(&mut loader.parser, &mut s, &mut bb, false) {
                    return Err(FT_ERR_INVALID_FILE_FORMAT);
                }
                have_integer = false;
            } else if at(b, cur) == b'-'
                && cur + 6 < limit
                && at(b, cur + 1) == b'|'
                && have_integer
            {
                let mut s: FtULong = 0;
                let mut bb: usize = 0;

                loader.parser.root.cursor = start_binary;
                if !read_binary_data(&mut loader.parser, &mut s, &mut bb, false) {
                    return Err(FT_ERR_INVALID_FILE_FORMAT);
                }
                have_integer = false;
            }
            /* look for immediates */
            else if at(b, cur) == b'/' && cur + 2 < limit {
                cur += 1;

                loader.parser.root.cursor = cur;
                t1_skip_ps_token(&mut loader.parser);
                if loader.parser.root.error != 0 {
                    break 'exit;
                }

                let len = loader.parser.root.cursor - cur;

                if len > 0 && len < 22 && loader.parser.root.cursor < limit {
                    /* now compare the immediate name to the keyword table */
                    let name_bytes = &loader.parser.bytes()[cur..cur + len];

                    for keyword in T1_KEYWORDS.iter() {
                        let name = keyword.ident.as_bytes();
                        if name.is_empty() {
                            break;
                        }

                        if name_bytes[0] == name[0] && len == name.len() && name_bytes == name {
                            /* We found it -- run the parsing callback!     */
                            /* We record every instance of every field      */
                            /* (until we reach the base font of a           */
                            /* synthetic font) to deal adequately with      */
                            /* multiple master fonts; this is also          */
                            /* necessary because later PostScript           */
                            /* definitions override earlier ones.           */

                            /* Once we encounter `FontDirectory' after      */
                            /* `/Private', we know that this is a synthetic */
                            /* font; except for `/CharStrings' we are not   */
                            /* interested in anything that follows this     */
                            /* `FontDirectory'.                             */

                            /* MM fonts have more than one /Private token at */
                            /* the top level; let's hope that all the junk   */
                            /* that follows the first /Private token is not  */
                            /* interesting to us.                            */

                            /* According to Adobe Tech Note #5175 (CID-Keyed */
                            /* Font Installation for ATM Software) a `begin' */
                            /* must be followed by exactly one `end', and    */
                            /* `begin' -- `end' pairs must be accurately     */
                            /* paired.  We could use this to distinguish     */
                            /* between the global Private and the Private    */
                            /* dict that is a member of the Blend dict.      */

                            let dict: FtUInt = if loader.keywords_encountered & T1_PRIVATE != 0 {
                                T1_FIELD_DICT_PRIVATE
                            } else {
                                T1_FIELD_DICT_FONTDICT
                            };

                            if dict & keyword.dict == 0 {
                                break;
                            }

                            if loader.keywords_encountered & T1_FONTDIR_AFTER_PRIVATE == 0
                                || keyword.ident == "CharStrings"
                            {
                                loader.parser.root.error =
                                    error_code(t1_load_keyword(face, loader, keyword));
                                if loader.parser.root.error != 0 {
                                    if loader.parser.root.error == FT_ERR_IGNORE {
                                        loader.parser.root.error = FT_ERR_OK;
                                    } else {
                                        return Err(loader.parser.root.error);
                                    }
                                }
                            }
                            break;
                        }
                    }
                }

                have_integer = false;
            } else {
                t1_skip_ps_token(&mut loader.parser);
                if loader.parser.root.error != 0 {
                    break 'exit;
                }
                have_integer = false;
            }

            t1_skip_spaces(&mut loader.parser);
        }
    }

    /* Exit: */
    parser_result(loader.parser.root.error)
}

/// `t1_init_loader`
fn t1_init_loader(loader: &mut T1LoaderRec, _face: &T1FaceRec) {
    *loader = T1LoaderRec::default();
}

/// `t1_done_loader`
fn t1_done_loader(loader: &mut T1LoaderRec) {
    /* finalize tables */
    t1_release_table(&mut loader.encoding_table);
    t1_release_table(&mut loader.charstrings);
    t1_release_table(&mut loader.glyph_names);
    t1_release_table(&mut loader.swap_table);
    t1_release_table(&mut loader.subrs);

    /* finalize hash */
    loader.subrs_hash = None;

    /* finalize parser */
    t1_finalize_parser(&mut loader.parser);
}

/// `T1_Open_Face` (the face's stream is `stream`)
pub fn t1_open_face(face: &mut T1FaceRec, stream: &mut FtStreamRec) -> FtResult<()> {
    let mut loader = T1LoaderRec::default();

    t1_init_loader(&mut loader, face);

    let error = t1_open_face_loader(face, stream, &mut loader);

    /* Exit: */
    t1_done_loader(&mut loader);
    error
}

fn t1_open_face_loader(
    face: &mut T1FaceRec,
    stream: &mut FtStreamRec,
    loader: &mut T1LoaderRec,
) -> FtResult<()> {
    let mut error: FtResult<()> = Ok(());

    /* default values */
    face.indices.ndv_idx = -1;
    face.indices.cdv_idx = -1;
    face.len_buildchar = 0;

    {
        let priv_ = &mut face.type1.private_dict;
        priv_.blue_shift = 7;
        priv_.blue_fuzz = 1;
        priv_.lenIV = 4;
        priv_.expansion_factor = (0.06 * 0x10000 as f64) as FtFixed;
        priv_.blue_scale = (0.039625 * 0x10000 as f64 * 1000.0) as FtFixed;
    }

    t1_new_parser(&mut loader.parser, stream)?;

    let base_len = loader.parser.base_len;
    parse_dict(face, loader, T1Dict::Base, base_len)?;

    t1_get_private_dict(&mut loader.parser, stream)?;

    let private_len = loader.parser.private_len;
    parse_dict(face, loader, T1Dict::Private, private_len)?;

    /* ensure even-ness of `num_blue_values' */
    face.type1.private_dict.num_blue_values &= !1;

    /* (T1_CONFIG_OPTION_NO_MM_SUPPORT is undefined) */

    /* we don't support Multiple Master fonts with intermediate designs; */
    /* this implies that `num_designs' must be equal to `2^^num_axis'    */
    if let Some(blend) = face.blend.as_deref() {
        /* (`num_axis' is at most `T1_MAX_MM_AXIS') */
        if blend.num_designs != (1u32 << blend.num_axis) {
            t1_done_blend(face);
        }
    }

    if let Some(blend) = face.blend.as_deref_mut() {
        if blend.num_default_design_vector != 0 && blend.num_default_design_vector != blend.num_axis
        {
            /* we don't use it currently so just warn, reset, and ignore */
            blend.num_default_design_vector = 0;
        }
    }

    /* the following can happen for MM instances; we then treat the */
    /* font as a normal PS font                                     */
    if face
        .blend
        .as_deref()
        .is_some_and(|blend| blend.num_designs == 0 || blend.num_axis == 0)
    {
        t1_done_blend(face);
    }

    /* the font may have no valid WeightVector */
    if face
        .blend
        .as_deref()
        .is_some_and(|blend| blend.weight_vector.is_none())
    {
        t1_done_blend(face);
    }

    /* the font may have no valid BlendDesignPositions */
    if face
        .blend
        .as_deref()
        .is_some_and(|blend| blend.design_pos.is_none())
    {
        t1_done_blend(face);
    }

    /* the font may have no valid BlendDesignMap */
    if let Some(blend) = face.blend.as_deref() {
        for i in 0..blend.num_axis as usize {
            if blend
                .design_map
                .get(i)
                .is_none_or(|map| map.num_points == 0)
            {
                t1_done_blend(face);
                break;
            }
        }
    }

    if face.blend.is_some() {
        if face.len_buildchar > 0 {
            match ft_new_array(face.len_buildchar as FtLong) {
                Ok(buildchar) => face.buildchar = buildchar,
                Err(e) => {
                    face.len_buildchar = 0;
                    return Err(e);
                }
            }
        }
    } else {
        face.len_buildchar = 0;
    }

    /* now, propagate the subrs, charstrings, and glyphnames tables */
    /* to the Type1 data                                            */
    let type1 = &mut face.type1;
    type1.num_glyphs = loader.num_glyphs;

    if loader.subrs.init != 0 {
        type1.num_subrs = loader.num_subrs;
        type1.subrs = Some(std::mem::take(&mut loader.subrs).freeze());
        type1.subrs_hash = loader.subrs_hash.take();

        /* prevent `t1_done_loader' from freeing the propagated data */
        loader.subrs.init = 0;
    }

    /* (no incremental interface) */
    if loader.charstrings.init == 0 {
        error = Err(FT_ERR_INVALID_FILE_FORMAT);
    }

    loader.charstrings.init = 0;
    type1.charstrings = std::mem::take(&mut loader.charstrings).freeze();

    /* we copy the glyph names `block' and `elements' fields; */
    /* the `lengths' field must be released later             */
    type1.glyph_names = std::mem::take(&mut loader.glyph_names).freeze();

    /* we must now build type1.encoding when we have a custom array */
    if type1.encoding_type == T1_ENCODING_TYPE_ARRAY {
        /* OK, we do the following: for each element in the encoding  */
        /* table, look up the index of the glyph having the same name */
        /* the index is then stored in type1.encoding.char_index, and */
        /* the name to type1.encoding.char_name                       */

        let mut min_char: FtInt = 0;
        let mut max_char: FtInt = 0;

        for charcode in 0..loader.encoding_table.max_elems.max(0) as usize {
            let char_name = loader.encoding_table.element(charcode).map(c_str);

            type1.encoding.char_index[charcode] = 0;
            type1.encoding.char_name[charcode] = None; /* ".notdef" */

            if let Some(char_name) = char_name {
                for idx in 0..type1.num_glyphs.max(0) as usize {
                    let glyph_name = type1.glyph_names.name(idx).unwrap_or(b"");

                    if char_name == glyph_name {
                        type1.encoding.char_index[charcode] = idx as FtUShort;
                        type1.encoding.char_name[charcode] = Some(idx);

                        /* Change min/max encoded char only if glyph name is */
                        /* not /.notdef                                      */
                        if glyph_name != b".notdef" {
                            if (charcode as FtInt) < min_char {
                                min_char = charcode as FtInt;
                            }
                            if charcode as FtInt >= max_char {
                                max_char = charcode as FtInt + 1;
                            }
                        }
                        break;
                    }
                }
            }
        }

        type1.encoding.code_first = min_char;
        type1.encoding.code_last = max_char;
        type1.encoding.num_chars = loader.num_chars;
    }

    /* some sanitizing to avoid overflows later on; */
    /* the upper limits are ad-hoc values           */
    let priv_ = &mut type1.private_dict;
    if priv_.blue_shift > 1000 || priv_.blue_shift < 0 {
        priv_.blue_shift = 7;
    }

    if priv_.blue_fuzz > 1000 || priv_.blue_fuzz < 0 {
        priv_.blue_fuzz = 1;
    }

    error
}
