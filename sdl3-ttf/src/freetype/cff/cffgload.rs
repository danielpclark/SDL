// Rust translation of src/cff/cffgload.c and src/cff/cffgload.h from
// FreeType (2.13.2, as SDL_ttf's external/freetype pins it).
// Copyright (C) 1996-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! OpenType Glyph Loader (body).
//!
//! The glyph slot is the face's (with the CFF driver's additions in
//! `cff_slot`) and so is the size; `size` says whether there is one (C's
//! non-NULL `CFF_Size`). `FT_CONFIG_OPTION_INCREMENTAL` is defined, but no
//! incremental interface is ever set, so its branches are left out.

use std::sync::Arc;

use super::super::base::ftcalc::*;
use super::super::base::ftobjs::*;
use super::super::base::ftoutln::*;
use super::super::base::ftstream::FtStreamRec;
use super::super::cfftypes::*;
use super::super::fttypes::*;
use super::super::psaux::cffdecode::*;
use super::super::psaux::psft::cf2_decoder_parse_charstrings;
use super::super::psaux::psobjs::*;
use super::super::psaux::PsDriverRec;
use super::super::sfnt::ttload::with_stream;
use super::super::sfnt::ttmtx::tt_face_get_metrics;
use super::super::sfnt::ttsbit::tt_face_load_sbit_image;
use super::super::sfnt::ttsvg::tt_face_load_svg_doc;
use super::super::tttypes::*;
use super::cffload::*;

/* TT_CONFIG_OPTION_GX_VAR_SUPPORT */
fn is_default_instance(face: &FtFaceRec) -> bool {
    !(ft_is_named_instance(face) || ft_is_variation(face))
}

/// `cff_get_glyph_data`
pub fn cff_get_glyph_data(
    cff: &CffFontRec,
    stream: &mut FtStreamRec,
    glyph_index: FtUInt,
) -> FtResult<Vec<u8>> {
    /* (no incremental interface is set) */
    cff_index_access_element(&cff.charstrings_index, stream, glyph_index)
}

/// `cff_free_glyph_data`
pub fn cff_free_glyph_data(cff: &CffFontRec, pointer: Vec<u8>) {
    cff_index_forget_element(&cff.charstrings_index, pointer);
}

/*************************************************************************/
/*************************************************************************/
/*************************************************************************/
/**********                                                      *********/
/**********                                                      *********/
/**********            COMPUTE THE MAXIMUM ADVANCE WIDTH         *********/
/**********                                                      *********/
/**********    The following code is in charge of computing      *********/
/**********    the maximum advance width of the font.  It        *********/
/**********    quickly processes each glyph charstring to        *********/
/**********    extract the value from either a `sbw' or `seac'   *********/
/**********    operator.                                         *********/
/**********                                                      *********/
/*************************************************************************/
/*************************************************************************/
/*************************************************************************/

/* (`cff_compute_max_advance' is `#if 0' in C: unused until pure CFF fonts */
/* are supported)                                                          */

/// The face data the builders and the Adobe engine read through the face.
fn cff_builder_face(face: &TtFaceRec) -> CffBuilderFace {
    let driver = face
        .root
        .library
        .as_ref()
        .map(|l| {
            l.modules[face.root.driver]
                .with_props(|d: &mut PsDriverRec| *d)
                .unwrap_or_default()
        })
        .unwrap_or_default();

    let (num_coords, normalizedcoords) = cff_get_var_blend(face);

    CffBuilderFace {
        units_per_EM: face.root.units_per_EM,
        y_ppem: face.root.size.metrics.y_ppem,
        no_stem_darkening: face.root.internal.no_stem_darkening,
        is_cff2: face.is_cff2,
        driver,
        num_coords,
        normalizedcoords: normalizedcoords.map(|v| v.to_vec()),
    }
}

/// `cff_slot_load`
pub fn cff_slot_load(
    face: &mut TtFaceRec,
    size: bool,
    glyph_index: FtUInt,
    load_flags: FtInt32,
) -> FtResult<()> {
    with_stream(face, |face, stream| {
        cff_slot_load_stream(face, stream, size, glyph_index, load_flags)
    })
}

fn cff_slot_load_stream(
    face: &mut TtFaceRec,
    stream: &mut FtStreamRec,
    size: bool,
    mut glyph_index: FtUInt,
    mut load_flags: FtInt32,
) -> FtResult<()> {
    let mut error: FtResult<()>;
    let mut hinting: bool;
    let scaled: bool;
    let mut force_scaling: bool;

    let font_matrix: FtMatrix;
    let font_offset: FtVector;

    force_scaling = false;

    {
        let cff = face.cff.as_ref().ok_or(FT_ERR_INVALID_ARGUMENT)?;

        /* in a CID-keyed font, consider `glyph_index' as a CID and map */
        /* it immediately to the real glyph_index -- if it isn't a      */
        /* subsetted font, glyph_indices and CIDs are identical, though */
        if cff.top_font.font_dict.cid_registry != 0xFFFF && cff.charset.cids.is_some() {
            /* don't handle CID 0 (.notdef) which is directly mapped to GID 0 */
            if glyph_index != 0 {
                glyph_index = cff_charset_cid_to_gindex(&cff.charset, glyph_index);
                if glyph_index == 0 {
                    return Err(FT_ERR_INVALID_ARGUMENT);
                }
            }
        } else if glyph_index >= cff.num_glyphs {
            return Err(FT_ERR_INVALID_ARGUMENT);
        }
    }

    if load_flags & FT_LOAD_NO_RECURSE != 0 {
        load_flags |= FT_LOAD_NO_SCALE | FT_LOAD_NO_HINTING;
    }

    face.cff_slot.x_scale = 0x10000;
    face.cff_slot.y_scale = 0x10000;
    if size {
        face.cff_slot.x_scale = face.root.size.metrics.x_scale;
        face.cff_slot.y_scale = face.root.size.metrics.y_scale;
    }

    /* TT_CONFIG_OPTION_EMBEDDED_BITMAPS */

    /* try to load embedded bitmap if any              */
    /*                                                 */
    /* XXX: The convention should be emphasized in     */
    /*      the documents because it can be confusing. */
    if size {
        if face.cff_size.strike_index != 0xFFFFFFFF
            && (load_flags & FT_LOAD_NO_BITMAP) == 0
            && is_default_instance(&face.root)
        {
            let mut metrics = TtSBitMetricsRec::default();

            let strike_index = face.cff_size.strike_index;
            error = tt_face_load_sbit_image(
                face,
                stream,
                strike_index,
                glyph_index,
                load_flags as FtUInt,
                &mut metrics,
            );

            if error.is_ok() {
                {
                    let glyph = &mut face.root.glyph;

                    glyph.outline.n_points = 0;
                    glyph.outline.n_contours = 0;

                    glyph.metrics.width = metrics.width as FtPos * 64;
                    glyph.metrics.height = metrics.height as FtPos * 64;

                    glyph.metrics.horiBearingX = metrics.horiBearingX as FtPos * 64;
                    glyph.metrics.horiBearingY = metrics.horiBearingY as FtPos * 64;
                    glyph.metrics.horiAdvance = metrics.horiAdvance as FtPos * 64;

                    glyph.metrics.vertBearingX = metrics.vertBearingX as FtPos * 64;
                    glyph.metrics.vertBearingY = metrics.vertBearingY as FtPos * 64;
                    glyph.metrics.vertAdvance = metrics.vertAdvance as FtPos * 64;

                    glyph.format = FT_GLYPH_FORMAT_BITMAP;

                    if load_flags & FT_LOAD_VERTICAL_LAYOUT != 0 {
                        glyph.bitmap_left = metrics.vertBearingX as FtInt;
                        glyph.bitmap_top = metrics.vertBearingY as FtInt;
                    } else {
                        glyph.bitmap_left = metrics.horiBearingX as FtInt;
                        glyph.bitmap_top = metrics.horiBearingY as FtInt;
                    }
                }

                /* compute linear advance widths */

                let (_dummy, advance) = tt_face_get_metrics(face, stream, false, glyph_index);
                face.root.glyph.linearHoriAdvance = advance as FtFixed;

                let has_vertical_info = face.vertical_info && face.vertical.number_Of_VMetrics > 0;

                /* get the vertical metrics from the vmtx table if we have one */
                if has_vertical_info {
                    let (_dummy, advance) = tt_face_get_metrics(face, stream, true, glyph_index);
                    face.root.glyph.linearVertAdvance = advance as FtFixed;
                } else {
                    /* make up vertical ones */
                    if face.os2.version != 0xFFFF {
                        face.root.glyph.linearVertAdvance = (face.os2.sTypoAscender as FtPos
                            - face.os2.sTypoDescender as FtPos)
                            as FtFixed;
                    } else {
                        face.root.glyph.linearVertAdvance = (face.horizontal.Ascender as FtPos
                            - face.horizontal.Descender as FtPos)
                            as FtFixed;
                    }
                }

                return error;
            }
        }
    }

    /* return immediately if we only want the embedded bitmaps */
    if load_flags & FT_LOAD_SBITS_ONLY != 0 {
        return Err(FT_ERR_INVALID_ARGUMENT);
    }

    /* FT_CONFIG_OPTION_SVG */
    /* check for OT-SVG */
    if (load_flags & FT_LOAD_NO_SVG) == 0 && (load_flags & FT_LOAD_COLOR) != 0 && face.svg.is_some()
    {
        /*
         * We load the SVG document and try to grab the advances from the
         * table.  For the bearings we rely on the presetting hook to do that.
         */

        if size && (face.root.size.metrics.x_ppem < 1 || face.root.size.metrics.y_ppem < 1) {
            return Err(FT_ERR_INVALID_SIZE_HANDLE);
        }

        if tt_face_load_svg_doc(face, glyph_index).is_ok() {
            let x_scale = face.root.size.metrics.x_scale;
            let y_scale = face.root.size.metrics.y_scale;

            face.root.glyph.format = FT_GLYPH_FORMAT_SVG;

            /*
             * If horizontal or vertical advances are not present in the table,
             * this is a problem with the font since the standard requires them.
             * However, we are graceful and calculate the values by ourselves
             * for the vertical case.
             */
            let (_dummy, advance_x) = tt_face_get_metrics(face, stream, false, glyph_index);
            let (_dummy, advance_y) = tt_face_get_metrics(face, stream, true, glyph_index);

            let glyph = &mut face.root.glyph;
            glyph.linearHoriAdvance = advance_x as FtFixed;
            glyph.linearVertAdvance = advance_y as FtFixed;

            glyph.metrics.horiAdvance = ft_mul_fix(advance_x as FtLong, x_scale);
            glyph.metrics.vertAdvance = ft_mul_fix(advance_y as FtLong, y_scale);

            return Ok(());
        }
    }

    /* if we have a CID subfont, use its matrix (which has already */
    /* been multiplied with the root matrix)                       */

    /* this scaling is only relevant if the PS hinter isn't active */
    {
        let cff = face.cff.as_mut().unwrap();
        if cff.num_subfonts != 0 {
            let mut fd_index: FtByte = cff_fd_select_get(&mut cff.fd_select, glyph_index);

            if fd_index as FtUInt >= cff.num_subfonts {
                fd_index = (cff.num_subfonts as FtByte).wrapping_sub(1);
            }

            let top_upm: FtLong = cff.top_font.font_dict.units_per_em as FtLong;
            let sub = &cff.subfonts[fd_index as usize];
            let sub_upm: FtLong = sub.font_dict.units_per_em as FtLong;

            font_matrix = sub.font_dict.font_matrix;
            font_offset = sub.font_dict.font_offset;

            if top_upm != sub_upm {
                face.cff_slot.x_scale = ft_mul_div(face.cff_slot.x_scale, top_upm, sub_upm);
                face.cff_slot.y_scale = ft_mul_div(face.cff_slot.y_scale, top_upm, sub_upm);

                force_scaling = true;
            }
        } else {
            font_matrix = cff.top_font.font_dict.font_matrix;
            font_offset = cff.top_font.font_dict.font_offset;
        }
    }

    face.root.glyph.outline.n_points = 0;
    face.root.glyph.outline.n_contours = 0;

    /* top-level code ensures that FT_LOAD_NO_HINTING is set */
    /* if FT_LOAD_NO_SCALE is active                         */
    hinting = (load_flags & FT_LOAD_NO_HINTING) == 0;
    scaled = (load_flags & FT_LOAD_NO_SCALE) == 0;

    face.cff_slot.hint = hinting;
    face.cff_slot.scaled = scaled;
    face.root.glyph.format = FT_GLYPH_FORMAT_OUTLINE; /* by default */

    let left_bearing: FtVector;
    let glyph_width: FtPos;
    let hints_funcs: bool;
    {
        let builder_face = cff_builder_face(face);
        let size_internal = if size {
            Some(face.cff_size.module_data.is_some())
        } else {
            None
        };

        let TtFaceRec {
            root,
            cff,
            cff_slot,
            ..
        } = face;
        let cff = cff.as_mut().unwrap();

        let mut decoder = cff_decoder_init(
            builder_face,
            cff,
            stream,
            size_internal,
            Some(CffGlyph {
                root: &mut root.glyph,
                cff: cff_slot,
            }),
            hinting,
            ((load_flags >> 16) & 15) as FtRenderMode, /* FT_LOAD_TARGET_MODE */
        );

        /* this is for pure CFFs */
        if load_flags & FT_LOAD_ADVANCE_ONLY != 0 {
            decoder.width_only = true;
        }

        decoder.builder.no_recurse = load_flags & FT_LOAD_NO_RECURSE != 0;

        error = (|| -> FtResult<()> {
            /* this function also checks for a valid subfont index */
            cff_decoder_prepare(&mut decoder, size_internal, glyph_index)?;

            /* now load the unscaled outline */
            let charstring = cff_get_glyph_data(decoder.cff, decoder.stream, glyph_index)?;
            let charstring_len = charstring.len() as FtULong;
            let charstring: Arc<[u8]> = Arc::from(charstring);

            /* (the old engine is not compiled) */
            let error = {
                let mut psdecoder = ps_decoder_init(&mut decoder, false);

                let mut error =
                    cf2_decoder_parse_charstrings(&mut psdecoder, &charstring, charstring_len);

                /* Adobe's engine uses 16.16 numbers everywhere;              */
                /* as a consequence, glyphs larger than 2000ppem get rejected */
                if error == Err(FT_ERR_GLYPH_TOO_BIG) {
                    /* this time, we retry unhinted and scale up the glyph later on */
                    /* (the engine uses and sets the hardcoded value 0x10000 / 64 = */
                    /* 0x400 for both `x_scale' and `y_scale' in this case)         */
                    hinting = false;
                    force_scaling = true;
                    if let Some(glyph) = psdecoder.builder.builder.glyph.as_mut() {
                        glyph.cff.hint = hinting;
                    }

                    error =
                        cf2_decoder_parse_charstrings(&mut psdecoder, &charstring, charstring_len);
                }
                error
            };

            cff_free_glyph_data(decoder.cff, Vec::new());

            error?;

            /* We set control_data and control_len if charstrings is loaded. */
            /* See how charstring loads at cff_index_access_element() in     */
            /* cffload.c.                                                    */
            {
                let csindex = &decoder.cff.charstrings_index;

                if let (Some(offsets), Some(bytes)) = (&csindex.offsets, &csindex.bytes) {
                    let start = (offsets[glyph_index as usize] as usize).wrapping_sub(1);
                    if let Some(glyph) = decoder.builder.glyph.as_mut() {
                        glyph.root.control_data = bytes
                            .get(start..start + charstring_len as usize)
                            .map(|b| b.to_vec())
                            .unwrap_or_default();
                        glyph.root.control_len = charstring_len as i64;
                    }
                }
            }

            Ok(())
        })();

        /* Glyph_Build_Finished: */
        /* save new glyph tables, if no error */
        if error.is_ok() {
            cff_builder_done(&mut decoder.builder);
        }
        /* XXX: anything to do for broken glyph entry? */

        left_bearing = decoder.builder.left_bearing;
        glyph_width = decoder.glyph_width;
        hints_funcs = decoder.builder.hints_funcs;
    }

    if error.is_ok() {
        /* Now, set the metrics -- this is rather simple, as   */
        /* the left side bearing is the xMin, and the top side */
        /* bearing the yMax.                                   */

        /* For composite glyphs, return only left side bearing and */
        /* advance width.                                          */
        if load_flags & FT_LOAD_NO_RECURSE != 0 {
            let glyph = &mut face.root.glyph;
            let internal = &mut glyph.internal;

            glyph.metrics.horiBearingX = left_bearing.x;
            glyph.metrics.horiAdvance = glyph_width;
            internal.glyph_matrix = font_matrix;
            internal.glyph_delta = font_offset;
            internal.glyph_transformed = true;
        } else {
            if face.horizontal.number_Of_HMetrics != 0 {
                let (hori_bearing_x, hori_advance) =
                    tt_face_get_metrics(face, stream, false, glyph_index);
                let metrics = &mut face.root.glyph.metrics;
                metrics.horiAdvance = hori_advance as FtPos;
                metrics.horiBearingX = hori_bearing_x as FtPos;
                face.root.glyph.linearHoriAdvance = hori_advance as FtFixed;
            } else {
                /* copy the _unscaled_ advance width */
                face.root.glyph.metrics.horiAdvance = glyph_width;
                face.root.glyph.linearHoriAdvance = glyph_width;
            }

            face.root.glyph.internal.glyph_transformed = false;

            let has_vertical_info = face.vertical_info && face.vertical.number_Of_VMetrics > 0;

            /* get the vertical metrics from the vmtx table if we have one */
            if has_vertical_info {
                let (vert_bearing_y, vert_advance) =
                    tt_face_get_metrics(face, stream, true, glyph_index);
                let metrics = &mut face.root.glyph.metrics;
                metrics.vertBearingY = vert_bearing_y as FtPos;
                metrics.vertAdvance = vert_advance as FtPos;
            } else {
                /* make up vertical ones */
                if face.os2.version != 0xFFFF {
                    face.root.glyph.metrics.vertAdvance =
                        face.os2.sTypoAscender as FtPos - face.os2.sTypoDescender as FtPos;
                } else {
                    face.root.glyph.metrics.vertAdvance =
                        face.horizontal.Ascender as FtPos - face.horizontal.Descender as FtPos;
                }
            }

            let y_ppem = face.root.size.metrics.y_ppem;
            let (x_scale, y_scale) = (face.cff_slot.x_scale, face.cff_slot.y_scale);
            let glyph = &mut face.root.glyph;

            glyph.linearVertAdvance = glyph.metrics.vertAdvance;

            glyph.format = FT_GLYPH_FORMAT_OUTLINE;

            glyph.outline.flags = 0;
            if size && y_ppem < 24 {
                glyph.outline.flags |= FT_OUTLINE_HIGH_PRECISION;
            }

            glyph.outline.flags |= FT_OUTLINE_REVERSE_FILL;

            /* apply the font matrix, if any */
            if font_matrix.xx != 0x10000
                || font_matrix.yy != 0x10000
                || font_matrix.xy != 0
                || font_matrix.yx != 0
            {
                ft_outline_transform(&mut glyph.outline, &font_matrix);

                glyph.metrics.horiAdvance = ft_mul_fix(glyph.metrics.horiAdvance, font_matrix.xx);
                glyph.metrics.vertAdvance = ft_mul_fix(glyph.metrics.vertAdvance, font_matrix.yy);
            }

            if font_offset.x != 0 || font_offset.y != 0 {
                ft_outline_translate(&mut glyph.outline, font_offset.x, font_offset.y);

                glyph.metrics.horiAdvance += font_offset.x;
                glyph.metrics.vertAdvance += font_offset.y;
            }

            if (load_flags & FT_LOAD_NO_SCALE) == 0 || force_scaling {
                /* scale the outline and the metrics */
                let cur = &mut glyph.outline;

                /* First of all, scale the points */
                if !hinting || !hints_funcs {
                    let n = (cur.n_points.max(0) as usize).min(cur.points.len());
                    for vec in cur.points[..n].iter_mut() {
                        vec.x = ft_mul_fix(vec.x, x_scale);
                        vec.y = ft_mul_fix(vec.y, y_scale);
                    }
                }

                /* Then scale the metrics */
                glyph.metrics.horiAdvance = ft_mul_fix(glyph.metrics.horiAdvance, x_scale);
                glyph.metrics.vertAdvance = ft_mul_fix(glyph.metrics.vertAdvance, y_scale);
            }

            /* compute the other metrics */
            let cbox = ft_outline_get_cbox(&glyph.outline);

            let metrics = &mut glyph.metrics;
            metrics.width = cbox.xMax - cbox.xMin;
            metrics.height = cbox.yMax - cbox.yMin;

            metrics.horiBearingX = cbox.xMin;
            metrics.horiBearingY = cbox.yMax;

            if has_vertical_info {
                metrics.vertBearingX = metrics.horiBearingX - metrics.horiAdvance / 2;
                metrics.vertBearingY = ft_mul_fix(metrics.vertBearingY, y_scale);
            } else if load_flags & FT_LOAD_VERTICAL_LAYOUT != 0 {
                let va = metrics.vertAdvance;
                ft_synthesize_vertical_metrics(metrics, va);
            }
        }
    }

    error
}
