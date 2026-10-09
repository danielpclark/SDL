// Rust translation of src/psaux/cffdecode.c, src/psaux/cffdecode.h and
// the CFF decoder record of include/freetype/internal/psaux.h from
// FreeType (2.13.2, as SDL_ttf's external/freetype pins it).
// Copyright (C) 2017-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! PostScript CFF (Type 2) decoding routines (body).
//!
//! `CFF_CONFIG_OPTION_OLD_ENGINE` is undefined, so the old Type 2
//! charstring interpreter (`cff_decoder_parse_charstrings` and its
//! helpers) is not compiled; what remains sets the decoder up for the
//! Adobe engine (`psft`). The glyph data callbacks are the CFF driver's
//! `cff_get_glyph_data`, called directly.

use std::sync::Arc;

use super::super::base::ftstream::FtStreamRec;
use super::super::cff::cffload::{cff_fd_select_get, cff_get_standard_encoding};
use super::super::cfftypes::*;
use super::super::fttypes::*;
use super::psobjs::{cff_builder_init, CffBuilder, CffBuilderFace, CffGlyph};

pub const CFF_MAX_OPERANDS: usize = 48;
pub const CFF_MAX_SUBRS_CALLS: usize = 16; /* maximum subroutine nesting;         */
/* only 10 are allowed but there exist */
/* fonts like `HiraKakuProN-W3.ttf'    */
/* (Hiragino Kaku Gothic ProN W3;      */
/* 8.2d6e1; 2014-12-19) that exceed    */
/* this limit                          */
pub const CFF_MAX_TRANS_ELEMENTS: usize = 32;

/// `CFF_Decoder` (the old engine's operand stack, zones and flex state
/// are left out with it). The subroutine tables are offsets into their
/// index's bytes (`locals_bytes`, `globals_bytes`); `stream` is the face's.
#[derive(Debug)]
pub struct CffDecoder<'a> {
    pub builder: CffBuilder<'a>,
    pub cff: &'a mut CffFontRec,
    pub stream: &'a mut FtStreamRec,

    pub glyph_width: FtPos,
    pub nominal_width: FtPos,

    pub read_width: bool,
    pub width_only: bool,
    pub num_hints: FtInt,

    pub num_locals: FtUInt,
    pub num_globals: FtUInt,

    pub locals_bias: FtInt,
    pub globals_bias: FtInt,

    pub locals: Option<Arc<[usize]>>,
    pub locals_bytes: Option<Arc<[u8]>>,
    pub globals: Option<Arc<[usize]>>,
    pub globals_bytes: Option<Arc<[u8]>>,

    pub num_glyphs: FtUInt, /* number of glyphs in font */

    pub hint_mode: FtRenderMode,

    pub seac: bool,

    pub current_subfont: CffSubFontId, /* for current glyph_index */
}

/*************************************************************************/
/*************************************************************************/
/*************************************************************************/
/**********                                                      *********/
/**********                                                      *********/
/**********             GENERIC CHARSTRING PARSING               *********/
/**********                                                      *********/
/**********                                                      *********/
/*************************************************************************/
/*************************************************************************/
/*************************************************************************/

/// `cff_compute_bias`: computes the bias value in dependence of the
/// number of glyph subroutines.
///
/// * `in_charstring_type`: the `CharstringType' value of the top DICT
///   dictionary.
/// * `num_subrs`: the number of glyph subroutines.
fn cff_compute_bias(in_charstring_type: FtInt, num_subrs: FtUInt) -> FtInt {
    if in_charstring_type == 1 {
        0
    } else if num_subrs < 1240 {
        107
    } else if num_subrs < 33900 {
        1131
    } else {
        32768
    }
}

/// `cff_lookup_glyph_by_stdcharcode`
pub fn cff_lookup_glyph_by_stdcharcode(cff: &CffFontRec, charcode: FtInt) -> FtInt {
    /* CID-keyed fonts don't have glyph names */
    let Some(sids) = cff.charset.sids.as_ref() else {
        return -1;
    };

    /* check range of standard char code */
    if !(0..=255).contains(&charcode) {
        return -1;
    }

    /* Get code to SID mapping from `cff_standard_encoding'. */
    let glyph_sid = cff_get_standard_encoding(charcode as FtUInt);

    for n in 0..cff.num_glyphs as usize {
        if sids.get(n).copied() == Some(glyph_sid) {
            return n as FtInt;
        }
    }

    -1
}

/// `cff_decoder_init`: initializes a given glyph decoder.
///
/// * `face`: the current face object's data (and the face's CFF font and
///   stream).
/// * `size`: whether there is a size, and whether its `internal->module_data`
///   is set.
/// * `slot`: the current glyph object.
/// * `hinting`: whether hinting is active.
/// * `hint_mode`: the hinting mode.
#[allow(clippy::too_many_arguments)]
pub fn cff_decoder_init<'a>(
    face: CffBuilderFace,
    cff: &'a mut CffFontRec,
    stream: &'a mut FtStreamRec,
    size: Option<bool>,
    slot: Option<CffGlyph<'a>>,
    hinting: bool,
    hint_mode: FtRenderMode,
) -> CffDecoder<'a> {
    /* clear everything */
    /* initialize builder */
    let builder = cff_builder_init(face, size, slot, hinting);

    /* initialize Type2 decoder */
    let num_globals = cff.global_subrs_index.count;
    let globals = cff.global_subrs.clone();
    let globals_bytes = cff.global_subrs_index.bytes.clone();
    let globals_bias = cff_compute_bias(cff.top_font.font_dict.charstring_type, num_globals);

    CffDecoder {
        builder,
        cff,
        stream,
        glyph_width: 0,
        nominal_width: 0,
        read_width: false,
        width_only: false,
        num_hints: 0,
        num_locals: 0,
        num_globals,
        locals_bias: 0,
        globals_bias,
        locals: None,
        locals_bytes: None,
        globals,
        globals_bytes,
        num_glyphs: 0,
        hint_mode,
        seac: false,
        current_subfont: CffSubFontId::Top,
    }
}

/// `cff_decoder_prepare`: this function is used to select the subfont and
/// the locals subrs array
pub fn cff_decoder_prepare(
    decoder: &mut CffDecoder<'_>,
    size: Option<bool>,
    glyph_index: FtUInt,
) -> FtResult<()> {
    let cff = &mut *decoder.cff;
    let mut sub = CffSubFontId::Top;

    /* manage CID fonts */
    if cff.num_subfonts != 0 {
        let fd_index: FtByte = cff_fd_select_get(&mut cff.fd_select, glyph_index);

        if fd_index as FtUInt >= cff.num_subfonts {
            return Err(FT_ERR_INVALID_FILE_FORMAT);
        }

        sub = CffSubFontId::Sub(fd_index as usize);

        if decoder.builder.hints_funcs && size.is_some() {
            /* for CFFs without subfonts, this value has already been set */
            decoder.builder.hints_globals = Some(CffSubFontId::Sub(fd_index as usize));
        }
    }

    let subfont = cff.subfont(sub);
    decoder.num_locals = subfont.local_subrs_index.count;
    decoder.locals = subfont.local_subrs.clone();
    decoder.locals_bytes = subfont.local_subrs_index.bytes.clone();
    decoder.locals_bias =
        cff_compute_bias(cff.top_font.font_dict.charstring_type, decoder.num_locals);

    decoder.glyph_width = subfont.private_dict.default_width;
    decoder.nominal_width = subfont.private_dict.nominal_width;

    decoder.current_subfont = sub;

    Ok(())
}
