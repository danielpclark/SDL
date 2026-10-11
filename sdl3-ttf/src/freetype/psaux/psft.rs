// Rust translation of src/psaux/psft.c and src/psaux/psft.h from FreeType
// (2.13.2, as SDL_ttf's external/freetype pins it).
// Copyright 2013-2014 Adobe Systems Incorporated.
//
// This software, and all works of authorship, whether in source or
// object code form as indicated by the copyright notice(s) included
// herein (collectively, the "Work") is made available, and may only be
// used, modified, and distributed under the FreeType Project License,
// LICENSE.TXT.  Additionally, subject to the terms and conditions of the
// FreeType Project License, each contributor to the Work hereby grants
// to any individual or legal entity exercising permissions granted by
// the FreeType Project License and this section (hereafter, "You" or
// "Your") a perpetual, worldwide, non-exclusive, no-charge,
// royalty-free, irrevocable (except as stated in this section) patent
// license to make, have made, use, offer to sell, sell, import, and
// otherwise transfer the Work, where such license applies only to those
// patent claims licensable by such contributor that are necessarily
// infringed by their contribution(s) alone or by combination of their
// contribution(s) with the Work to which such contribution(s) was
// submitted.  If You institute patent litigation against any entity
// (including a cross-claim or counterclaim in a lawsuit) alleging that
// the Work or a contribution incorporated within the Work constitutes
// direct or contributory patent infringement, then any patent licenses
// granted to You under this License for that Work shall terminate as of
// the date such litigation is filed.
//
// By using, modifying, or distributing the Work you indicate that you
// have read and understood the terms and conditions of the
// FreeType Project License as well as those provided in this section,
// and you accept them fully.
//
// This is an altered (translated) version of the original software; the
// FreeType Project License is in FTL.TXT (see also LICENSE.txt).

//! FreeType Glue Component to Adobe's Interpreter (body).
//!
//! The decoder's font instance (the CFF font record's `cf2_instance`, or
//! the Type 1 decoder's) is taken out of it while a glyph is rendered.
//! The glyph data callbacks are the CFF driver's `cff_get_glyph_data`
//! (an element of the charstrings index), called directly; a Type 1
//! seac component's data is the Type 1 font's charstring (the face's,
//! without an incremental interface).

use std::sync::Arc;

use super::super::base::ftcalc::*;
use super::super::cff::cffload::cff_index_access_element;
use super::super::cfftypes::*;
use super::super::fttypes::*;
use super::cffdecode::cff_lookup_glyph_by_stdcharcode;
use super::psfixed::*;
use super::psfont::*;
use super::psglue::*;
use super::psobjs::*;
use super::psread::Cf2BufferRec;

/// `CF2_MAX_SIZE` (max ppem)
const CF2_MAX_SIZE: Cf2Fixed = cf2_int_to_fixed(2000);

/*
 * This check should avoid most internal overflow cases.  Clients should
 * generally respond to `Glyph_Too_Big' by getting a glyph outline
 * at EM size, scaling it and filling it as a graphics operation.
 *
 */
fn cf2_check_transform(transform: &Cf2Matrix, units_per_em: Cf2Int) -> FtResult<()> {
    if transform.a <= 0 || transform.d <= 0 {
        return Err(FT_ERR_INVALID_SIZE_HANDLE);
    }

    if units_per_em > 0x7FFF {
        return Err(FT_ERR_GLYPH_TOO_BIG);
    }

    let max_scale: Cf2Fixed = ft_div_fix(
        CF2_MAX_SIZE as FtLong,
        cf2_int_to_fixed(units_per_em) as FtLong,
    ) as Cf2Fixed;

    if transform.a > max_scale || transform.d > max_scale {
        return Err(FT_ERR_GLYPH_TOO_BIG);
    }

    Ok(())
}

fn cf2_set_glyph_width(decoder: &mut PsDecoder<'_, '_>, width: Cf2Fixed) {
    if !decoder.builder.is_t1 {
        if let PsDecoderFont::Cff { glyph_width, .. } = &mut decoder.font {
            **glyph_width = cf2_fixed_to_int(width) as FtPos;
        }
    }
}

/// `cf2_free_instance`: clean up font instance (its blend vectors go
/// with it).
fn cf2_free_instance(_font: Option<Box<Cf2FontRec>>) {}

/*********************************************
 *
 * functions for handling client outline;
 * FreeType uses coordinates in 26.6 format
 *
 */

/// `cf2_builder_moveTo`
pub fn cf2_builder_move_to(decoder: &mut PsDecoder<'_, '_>, _params: &Cf2CallbackParamsRec) {
    let builder = &mut decoder.builder;

    /* note: two successive moves simply close the contour twice */
    ps_builder_close_contour(builder);
    builder.path_begun = false;
}

/// `cf2_builder_lineTo` (`error` is the callbacks' error, the font
/// instance's)
pub fn cf2_builder_line_to(
    decoder: &mut PsDecoder<'_, '_>,
    error: &mut FtError,
    params: &Cf2CallbackParamsRec,
) {
    let builder = &mut decoder.builder;

    if !builder.path_begun {
        /* record the move before the line; also check points and set */
        /* `path_begun'                                               */
        if let Err(e) = ps_builder_start_point(builder, params.pt0.x, params.pt0.y) {
            if *error == 0 {
                *error = e;
            }
            return;
        }
    }

    /* `ps_builder_add_point1' includes a check_points call for one point */
    if let Err(e) = ps_builder_add_point1(builder, params.pt1.x, params.pt1.y) {
        if *error == 0 {
            *error = e;
        }
    }
}

/// `cf2_builder_cubeTo` (`error` is the callbacks' error, the font
/// instance's)
pub fn cf2_builder_cube_to(
    decoder: &mut PsDecoder<'_, '_>,
    error: &mut FtError,
    params: &Cf2CallbackParamsRec,
) {
    let builder = &mut decoder.builder;

    if !builder.path_begun {
        /* record the move before the line; also check points and set */
        /* `path_begun'                                               */
        if let Err(e) = ps_builder_start_point(builder, params.pt0.x, params.pt0.y) {
            if *error == 0 {
                *error = e;
            }
            return;
        }
    }

    /* prepare room for 3 points: 2 off-curve, 1 on-curve */
    if let Err(e) = ps_builder_check_points(builder, 3) {
        if *error == 0 {
            *error = e;
        }
        return;
    }

    ps_builder_add_point(builder, params.pt1.x, params.pt1.y, 0);
    ps_builder_add_point(builder, params.pt2.x, params.pt2.y, 0);
    ps_builder_add_point(builder, params.pt3.x, params.pt3.y, 1);
}

fn cf2_outline_init(outline: &mut Cf2OutlineCallbacksRec) {
    *outline = Cf2OutlineCallbacksRec::default();
}

/* get scaling and hint flag from GlyphSlot */
fn cf2_get_scale_and_hint_flag(
    decoder: &PsDecoder<'_, '_>,
    x_scale: &mut Cf2Fixed,
    y_scale: &mut Cf2Fixed,
    hinted: &mut bool,
    scaled: &mut bool,
) {
    let glyph = decoder.builder.builder.glyph.as_ref();
    let ext = glyph.map(|g| *g.cff).unwrap_or_default();

    /* note: FreeType scale includes a factor of 64 */
    *hinted = ext.hint;
    *scaled = ext.scaled;

    if *hinted {
        *x_scale = add_int32(ext.x_scale as i32, 32) / 64;
        *y_scale = add_int32(ext.y_scale as i32, 32) / 64;
    } else {
        /* for unhinted outlines, `cff_slot_load' does the scaling, */
        /* thus render at `unity' scale                             */

        *x_scale = 0x0400; /* 1/64 as 16.16 */
        *y_scale = 0x0400;
    }
}

/* get units per em from `FT_Face' */
/* TODO: should handle font matrix concatenation? */
fn cf2_get_units_per_em(decoder: &PsDecoder<'_, '_>) -> FtUShort {
    decoder.builder.builder.face.units_per_EM
}

/// `cf2_decoder_parse_charstrings`: main entry point: render one glyph.
pub fn cf2_decoder_parse_charstrings(
    decoder: &mut PsDecoder<'_, '_>,
    charstring_base: &Arc<[u8]>,
    charstring_len: FtULong,
) -> FtResult<()> {
    let is_t1 = decoder.builder.is_t1;

    /* (a Type 1 decoder always has the subfont `t1_make_subfont' made) */

    /* CF2 data is saved here across glyphs */
    let mut font = match decoder.cf2_instance().take() {
        Some(font) => font,
        None => {
            /* on first glyph, allocate instance structure */
            let mut font = Box::new(Cf2FontRec::default());

            if !is_t1 {
                if let Some(cff) = decoder.cff() {
                    font.cffload = cff.cffload;
                }
            }

            /* initialize a client outline, to be shared by each glyph rendered */
            cf2_outline_init(&mut font.outline);
            font
        }
    };

    /* save decoder; it is a stack variable and will be different on each */
    /* call                                                               */
    /* (the decoder is given to the functions using it) */

    let r = cf2_decoder_parse_charstrings_font(&mut font, decoder, charstring_base, charstring_len);

    *decoder.cf2_instance() = Some(font);

    r
}

fn cf2_decoder_parse_charstrings_font(
    font: &mut Cf2FontRec,
    decoder: &mut PsDecoder<'_, '_>,
    charstring_base: &Arc<[u8]>,
    charstring_len: FtULong,
) -> FtResult<()> {
    let is_t1 = decoder.builder.is_t1;

    /* build parameters for Adobe engine */

    let driver = decoder.builder.builder.face.driver;
    let no_stem_darkening_driver: bool = driver.no_stem_darkening;
    let no_stem_darkening_font: FtChar = decoder.builder.builder.face.no_stem_darkening;

    let mut transform = Cf2Matrix::default();
    let mut glyph_width: Cf2F16Dot16 = 0;

    let mut hinted = false;
    let mut scaled = false;

    /* FreeType has already looked up the GID; convert to         */
    /* `RegionBuffer', assuming that the input has been validated */
    let len = (charstring_len as usize).min(charstring_base.len());
    let buf = Cf2BufferRec {
        bytes: charstring_base.clone(),
        start: 0,
        ptr: 0,
        end: len,
    };

    cf2_get_scale_and_hint_flag(
        decoder,
        &mut transform.a,
        &mut transform.d,
        &mut hinted,
        &mut scaled,
    );

    if is_t1 {
        font.isCFF2 = false;
    } else {
        /* copy isCFF2 boolean from TT_Face to CF2_Font */
        font.isCFF2 = decoder.builder.builder.face.is_cff2;
    }
    font.isT1 = is_t1;

    font.renderingFlags = 0;
    if hinted {
        font.renderingFlags |= CF2_FLAGS_HINTED;
    }
    if scaled
        && (no_stem_darkening_font == 0
            || (no_stem_darkening_font < 0 && !no_stem_darkening_driver))
    {
        font.renderingFlags |= CF2_FLAGS_DARKENED;
    }

    font.darkenParams = driver.darken_params;

    /* now get an outline for this glyph;      */
    /* also get units per em to validate scale */
    font.unitsPerEm = cf2_get_units_per_em(decoder) as Cf2Int;

    if scaled {
        cf2_check_transform(&transform, font.unitsPerEm)?;
    }

    if cf2_get_glyph_outline(font, decoder, &buf, &transform, &mut glyph_width).is_err() {
        return Err(FT_ERR_INVALID_FILE_FORMAT);
    }

    cf2_set_glyph_width(decoder, glyph_width);

    Ok(())
}

/// `cf2_getSubfont`: get pointer to current FreeType subfont (based on
/// current glyphID)
pub fn cf2_get_subfont(decoder: &PsDecoder<'_, '_>) -> CffSubFontId {
    decoder.subfont_id()
}

/// The current subfont's record.
fn current_subfont<'d>(decoder: &'d PsDecoder<'_, '_>) -> &'d CffSubFontRec {
    decoder.subfont()
}

/// `cf2_getVStore`: get pointer to VStore structure (CFF mode)
pub fn cf2_get_vstore<'d>(decoder: &'d PsDecoder<'_, '_>) -> &'d CffVStoreRec {
    static EMPTY: std::sync::OnceLock<CffVStoreRec> = std::sync::OnceLock::new();

    match decoder.cff() {
        Some(cff) => &cff.vstore,
        None => EMPTY.get_or_init(CffVStoreRec::default),
    }
}

/// `cf2_getMaxstack`: get maxstack value from CFF2 Top DICT (CFF mode)
pub fn cf2_get_maxstack(decoder: &PsDecoder<'_, '_>) -> FtUInt {
    decoder
        .cff()
        .map_or(0, |cff| cff.top_font.font_dict.maxstack)
}

/// `cf2_getNormalizedVector`: get normalized design vector for current
/// render request; return pointer and length.
///
/// Note: Uses FT_Fixed not CF2_Fixed for the vector.
#[allow(clippy::type_complexity)]
pub fn cf2_get_normalized_vector(
    decoder: &PsDecoder<'_, '_>,
) -> FtResult<(Cf2UInt, Option<Vec<FtFixed>>)> {
    let face = &decoder.builder.builder.face;
    Ok((face.num_coords, face.normalizedcoords.clone()))
}

/// `cf2_getPpemY`: get `y_ppem' from `CFF_Size'
pub fn cf2_get_ppem_y(decoder: &PsDecoder<'_, '_>) -> Cf2Fixed {
    /*
     * Note that `y_ppem' can be zero if there wasn't a call to
     * `FT_Set_Char_Size' or something similar.  However, this isn't a
     * problem since we come to this place in the code only if
     * FT_LOAD_NO_SCALE is set (the other case gets caught by
     * `cf2_checkTransform').  The ppem value is needed to compute the stem
     * darkening, which is disabled for getting the unscaled outline.
     *
     */
    cf2_int_to_fixed(decoder.builder.builder.face.y_ppem as i32)
}

/* get standard stem widths for the current subfont; */
/* FreeType stores these as integer font units       */
/* (note: variable names seem swapped)               */

/// `cf2_getStdVW`
pub fn cf2_get_std_vw(decoder: &PsDecoder<'_, '_>) -> Cf2Fixed {
    cf2_int_to_fixed(current_subfont(decoder).private_dict.standard_height as i32)
}

/// `cf2_getStdHW`
pub fn cf2_get_std_hw(decoder: &PsDecoder<'_, '_>) -> Cf2Fixed {
    cf2_int_to_fixed(current_subfont(decoder).private_dict.standard_width as i32)
}

/// `cf2_getBlueMetrics`: note: FreeType stores 1000 times the actual value
/// for `BlueScale'
pub fn cf2_get_blue_metrics(
    decoder: &PsDecoder<'_, '_>,
    blue_scale: &mut Cf2Fixed,
    blue_shift: &mut Cf2Fixed,
    blue_fuzz: &mut Cf2Fixed,
) {
    let p = &current_subfont(decoder).private_dict;

    *blue_scale = ft_div_fix(p.blue_scale, cf2_int_to_fixed(1000) as FtLong) as Cf2Fixed;
    *blue_shift = cf2_int_to_fixed(p.blue_shift as i32);
    *blue_fuzz = cf2_int_to_fixed(p.blue_fuzz as i32);
}

/* get blue values counts and arrays; the FreeType parser has validated */
/* the counts and verified that each is an even number                  */

/// `cf2_getBlueValues`
pub fn cf2_get_blue_values<'d>(decoder: &'d PsDecoder<'_, '_>) -> (usize, &'d [FtPos]) {
    let p = &current_subfont(decoder).private_dict;
    (p.num_blue_values as usize, &p.blue_values[..])
}

/// `cf2_getOtherBlues`
pub fn cf2_get_other_blues<'d>(decoder: &'d PsDecoder<'_, '_>) -> (usize, &'d [FtPos]) {
    let p = &current_subfont(decoder).private_dict;
    (p.num_other_blues as usize, &p.other_blues[..])
}

/// `cf2_getFamilyBlues`
pub fn cf2_get_family_blues<'d>(decoder: &'d PsDecoder<'_, '_>) -> (usize, &'d [FtPos]) {
    let p = &current_subfont(decoder).private_dict;
    (p.num_family_blues as usize, &p.family_blues[..])
}

/// `cf2_getFamilyOtherBlues`
pub fn cf2_get_family_other_blues<'d>(decoder: &'d PsDecoder<'_, '_>) -> (usize, &'d [FtPos]) {
    let p = &current_subfont(decoder).private_dict;
    (p.num_family_other_blues as usize, &p.family_other_blues[..])
}

/// `cf2_getLanguageGroup`
pub fn cf2_get_language_group(decoder: &PsDecoder<'_, '_>) -> Cf2Int {
    current_subfont(decoder).private_dict.language_group
}

/// `cf2_initGlobalRegionBuffer`: convert unbiased subroutine index to
/// `CF2_Buffer' and return 0 on success
pub fn cf2_init_global_region_buffer(
    decoder: &PsDecoder<'_, '_>,
    subr_num: Cf2Int,
    buf: &mut Cf2BufferRec,
) -> bool {
    *buf = Cf2BufferRec::default();

    let idx: Cf2UInt = subr_num.wrapping_add(decoder.globals_bias) as Cf2UInt;
    if idx >= decoder.num_globals {
        return true; /* error */
    }

    let (Some(globals), Some(bytes)) = (decoder.globals.as_ref(), decoder.globals_bytes.as_ref())
    else {
        return true;
    };
    buf.bytes = bytes.clone();
    buf.start = globals[idx as usize];
    buf.ptr = buf.start;
    buf.end = globals[idx as usize + 1];

    false /* success */
}

/// `cf2_getSeacComponent`: convert AdobeStandardEncoding code to
/// CF2_Buffer; used for seac component
pub fn cf2_get_seac_component(
    decoder: &mut PsDecoder<'_, '_>,
    code: Cf2Int,
    buf: &mut Cf2BufferRec,
) -> FtResult<()> {
    *buf = Cf2BufferRec::default();

    let PsDecoderFont::Cff { cff, stream, .. } = &mut decoder.font else {
        return Err(FT_ERR_INVALID_GLYPH_FORMAT);
    };

    let gid: Cf2Int = cff_lookup_glyph_by_stdcharcode(cff, code);
    if gid < 0 {
        return Err(FT_ERR_INVALID_GLYPH_FORMAT);
    }

    /* `get_glyph_callback' (`cff_get_glyph_data') */
    /* TODO: for now, just pass the FreeType error through */
    let charstring = cff_index_access_element(&cff.charstrings_index, stream, gid as Cf2UInt)?;

    /* assume input has been validated */
    let len = charstring.len();
    buf.bytes = Arc::from(charstring);
    buf.start = 0;
    buf.end = len;
    buf.ptr = buf.start;

    Ok(())
}

/// `cf2_freeSeacComponent` (`free_glyph_callback`: the bytes go with the
/// buffer)
pub fn cf2_free_seac_component(_decoder: &mut PsDecoder<'_, '_>, buf: &mut Cf2BufferRec) {
    *buf = Cf2BufferRec::default();
}

/// `cf2_getT1SeacComponent`
pub fn cf2_get_t1_seac_component(
    decoder: &mut PsDecoder<'_, '_>,
    glyph_index: FtUInt,
    buf: &mut Cf2BufferRec,
) -> FtResult<()> {
    /* (FT_CONFIG_OPTION_INCREMENTAL: no incremental interface) */

    /* For ordinary fonts get the character data stored in the face record. */
    let Some(charstrings) = decoder.t1().and_then(|t1| t1.charstrings.as_ref()) else {
        return Err(FT_ERR_INVALID_GLYPH_FORMAT);
    };

    let start = charstrings
        .elements
        .get(glyph_index as usize)
        .copied()
        .flatten()
        .unwrap_or(0);
    let charstring_len = charstrings.length(glyph_index as usize) as usize;

    *buf = Cf2BufferRec {
        bytes: charstrings.block.clone(),
        start,
        ptr: start,
        end: start + charstring_len,
    };

    Ok(())
}

/// `cf2_freeT1SeacComponent` (no incremental interface)
pub fn cf2_free_t1_seac_component(_decoder: &mut PsDecoder<'_, '_>, _buf: &mut Cf2BufferRec) {}

/// `cf2_initLocalRegionBuffer`
pub fn cf2_init_local_region_buffer(
    decoder: &PsDecoder<'_, '_>,
    subr_num: Cf2Int,
    buf: &mut Cf2BufferRec,
) -> bool {
    *buf = Cf2BufferRec::default();

    let idx: Cf2UInt = subr_num.wrapping_add(decoder.locals_bias) as Cf2UInt;
    if idx >= decoder.num_locals {
        return true; /* error */
    }

    if decoder.builder.is_t1 {
        let Some(t1) = decoder.t1() else {
            return true;
        };

        /* The Type 1 driver stores subroutines without the seed bytes. */
        /* The CID driver stores subroutines with seed bytes.  This     */
        /* case is taken care of when decoder->subrs_len == 0.          */
        if let Some(locals) = t1.locals_t1.as_ref() {
            match locals.elements.get(idx as usize).copied().flatten() {
                Some(start) => {
                    buf.bytes = locals.block.clone();
                    buf.start = start;
                    buf.end = start + locals.length(idx as usize) as usize;
                }
                None => { /* (a NULL subroutine is an empty buffer) */ }
            }
        } else {
            let (Some(locals), Some(bytes)) =
                (decoder.locals.as_ref(), decoder.locals_bytes.as_ref())
            else {
                return true;
            };

            /* We are using subroutines from a CID font.  We must adjust */
            /* for the seed bytes.                                       */
            buf.bytes = bytes.clone();
            buf.start = locals[idx as usize] + if t1.lenIV >= 0 { t1.lenIV as usize } else { 0 };
            buf.end = locals[idx as usize + 1];
        }
    } else {
        let (Some(locals), Some(bytes)) = (decoder.locals.as_ref(), decoder.locals_bytes.as_ref())
        else {
            return true;
        };
        buf.bytes = bytes.clone();
        buf.start = locals[idx as usize];
        buf.end = locals[idx as usize + 1];
    }

    buf.ptr = buf.start;

    false /* success */
}

/// `cf2_getDefaultWidthX`
pub fn cf2_get_default_width_x(decoder: &PsDecoder<'_, '_>) -> Cf2Fixed {
    cf2_int_to_fixed(current_subfont(decoder).private_dict.default_width as i32)
}

/// `cf2_getNominalWidthX`
pub fn cf2_get_nominal_width_x(decoder: &PsDecoder<'_, '_>) -> Cf2Fixed {
    cf2_int_to_fixed(current_subfont(decoder).private_dict.nominal_width as i32)
}

/// `cf2_outline_reset`
pub fn cf2_outline_reset(font: &mut Cf2FontRec, decoder: &mut PsDecoder<'_, '_>) {
    font.outline.windingMomentum = 0;

    if let Some(loader) = decoder.builder.builder.loader() {
        loader.rewind();
    }
}

/// `cf2_outline_close`
pub fn cf2_outline_close(decoder: &mut PsDecoder<'_, '_>) {
    ps_builder_close_contour(&mut decoder.builder);
    if let Some(loader) = decoder.builder.builder.loader() {
        loader.add();
    }
}
