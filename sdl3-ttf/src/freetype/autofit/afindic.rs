// Rust translation of src/autofit/afindic.c and afindic.h from FreeType
// (2.13.2, as SDL_ttf's external/freetype pins it), with
// `AF_CONFIG_OPTION_INDIC` defined.
// Copyright (C) 2007-2023 by Rahul Bhalerao <rahul.bhalerao@redhat.com>,
// <b.rahul.pm@gmail.com>.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! Auto-fitter hinting routines for Indic writing system.

use super::super::base::ftobjs::*;
use super::super::fttypes::*;
use super::afcjk::*;
use super::afglobal::AfFaceGlobalsRec;
use super::afhints::*;
use super::aftypes::*;

/// `af_indic_metrics_init`
fn af_indic_metrics_init(
    metrics: &mut AfStyleMetricsRec, /* AF_CJKMetrics */
    face: &mut FtFace,
    globals: &mut AfFaceGlobalsRec,
) -> FtResult<()> {
    /* skip blue zone init in CJK routines */
    let oldmap = face.charmap;

    metrics.cjk_mut().units_per_em = face.units_per_EM as FtUInt;

    if ft_select_charmap(face, FT_ENCODING_UNICODE).is_err() {
        face.charmap = None;
    } else {
        af_cjk_metrics_init_widths(metrics, face, globals);
        /* either need indic specific blue_chars[] or just skip blue zones */
        af_cjk_metrics_check_digits(metrics, face, globals);
    }

    face.charmap = oldmap;
    Ok(())
}

/// `af_indic_metrics_scale`
fn af_indic_metrics_scale(
    metrics: &mut AfStyleMetricsRec,
    scaler: &AfScalerRec,
    globals: &AfFaceGlobalsRec,
) {
    /* use CJK routines */
    af_cjk_metrics_scale(metrics, scaler, globals);
}

/// `af_indic_hints_init`
fn af_indic_hints_init(hints: &mut AfGlyphHintsRec, metrics: &AfStyleMetricsRec) -> FtResult<()> {
    /* use CJK routines */
    af_cjk_hints_init(hints, metrics)
}

/// `af_indic_hints_apply`
fn af_indic_hints_apply(
    glyph_index: FtUInt,
    hints: &mut AfGlyphHintsRec,
    outline: &mut FtOutline,
    metrics: &AfStyleMetricsRec,
    globals: &AfFaceGlobalsRec,
) -> FtResult<()> {
    /* use CJK routines */
    af_cjk_hints_apply(glyph_index, hints, outline, metrics, globals)
}

/* Extract standard_width from writing system/script specific */
/* metrics class.                                             */

/// `af_indic_get_standard_widths`
fn af_indic_get_standard_widths(
    metrics: &AfStyleMetricsRec, /* AF_CJKMetrics */
) -> (FtPos, FtPos) {
    let cjk = metrics.cjk();
    (
        cjk.axis[AF_DIMENSION_VERT].standard_width,
        cjk.axis[AF_DIMENSION_HORZ].standard_width,
    )
}

/*************************************************************************/
/*************************************************************************/
/*****                                                               *****/
/*****                I N D I C   S C R I P T   C L A S S            *****/
/*****                                                               *****/
/*************************************************************************/
/*************************************************************************/

/// `af_indic_writing_system_class`
pub static AF_INDIC_WRITING_SYSTEM_CLASS: AfWritingSystemClassRec = AfWritingSystemClassRec {
    writing_system: AF_WRITING_SYSTEM_INDIC,

    /* sizeof ( AF_CJKMetricsRec ) */
    style_metrics_init: Some(af_indic_metrics_init), /* style_metrics_init    */
    style_metrics_scale: Some(af_indic_metrics_scale), /* style_metrics_scale   */
    style_metrics_done: None,                        /* style_metrics_done    */
    style_metrics_getstdw: Some(af_indic_get_standard_widths), /* style_metrics_getstdw */

    style_hints_init: Some(af_indic_hints_init), /* style_hints_init      */
    style_hints_apply: Some(af_indic_hints_apply), /* style_hints_apply     */
};
