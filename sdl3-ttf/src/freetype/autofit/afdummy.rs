// Rust translation of src/autofit/afdummy.c and afdummy.h from FreeType
// (2.13.2, as SDL_ttf's external/freetype pins it).
// Copyright (C) 2003-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! Auto-fitter dummy routines to be used if no hinting should be
//! performed.

use super::super::fttypes::*;
use super::afglobal::AfFaceGlobalsRec;
use super::afhints::*;
use super::aftypes::*;

/// `af_dummy_hints_init`
fn af_dummy_hints_init(hints: &mut AfGlyphHintsRec, metrics: &AfStyleMetricsRec) -> FtResult<()> {
    af_glyph_hints_rescale(hints, metrics);

    hints.x_scale = metrics.scaler.x_scale;
    hints.y_scale = metrics.scaler.y_scale;
    hints.x_delta = metrics.scaler.x_delta;
    hints.y_delta = metrics.scaler.y_delta;

    Ok(())
}

/// `af_dummy_hints_apply`
fn af_dummy_hints_apply(
    _glyph_index: FtUInt,
    hints: &mut AfGlyphHintsRec,
    outline: &mut FtOutline,
    _metrics: &AfStyleMetricsRec,
    _globals: &AfFaceGlobalsRec,
) -> FtResult<()> {
    let error = af_glyph_hints_reload(hints, outline);
    if error.is_ok() {
        af_glyph_hints_save(hints, outline);
    }

    error
}

/// `af_dummy_writing_system_class`
pub static AF_DUMMY_WRITING_SYSTEM_CLASS: AfWritingSystemClassRec = AfWritingSystemClassRec {
    writing_system: AF_WRITING_SYSTEM_DUMMY,

    /* sizeof ( AF_StyleMetricsRec ) */
    style_metrics_init: None,    /* style_metrics_init    */
    style_metrics_scale: None,   /* style_metrics_scale   */
    style_metrics_done: None,    /* style_metrics_done    */
    style_metrics_getstdw: None, /* style_metrics_getstdw */

    style_hints_init: Some(af_dummy_hints_init), /* style_hints_init      */
    style_hints_apply: Some(af_dummy_hints_apply), /* style_hints_apply     */
};
