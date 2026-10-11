// Rust translation of src/cid/cidgload.c and src/cid/cidgload.h from
// FreeType (2.13.2, as SDL_ttf's external/freetype pins it).
// Copyright (C) 1996-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! CID-keyed Type1 Glyph Loader (body).
//!
//! The glyph slot and size are the face's; the decoder's parse callback
//! (`cid_load_glyph`) is called directly, with the face's records the
//! decoder does not hold. `FT_CONFIG_OPTION_INCREMENTAL` is defined, but
//! no incremental interface is ever set, so its branches are left out;
//! `T1_CONFIG_OPTION_OLD_ENGINE` is undefined. (The `#if 0`ed
//! `cid_face_compute_max_advance` is not translated.)

use std::sync::Arc;

use super::super::base::ftcalc::{fixed_to_int, ft_mul_fix};
use super::super::base::ftmemory::ft_qalloc;
use super::super::base::ftobjs::*;
use super::super::base::ftoutln::*;
use super::super::base::ftstream::FtStreamRec;
use super::super::cfftypes::CffSubFontRec;
use super::super::fttypes::*;
use super::super::psaux::psft::cf2_decoder_parse_charstrings_at;
use super::super::psaux::psobjs::*;
use super::super::psaux::t1decode::*;
use super::super::t1tables::CidFaceInfoRec;
use super::super::t1types::*;
use super::super::type1::t1gload::t1_builder_face;
use super::cidload::cid_get_offset;

/// `cid_compute_fd_and_offsets`: a helper function to compute FD number
/// (`fd_select`), the offset to the head of the glyph data (`off1`), and
/// the offset to the and of the glyph data (`off2`) (`stream` is the
/// face's `cid_stream`).
///
/// The number how many times `cid_get_offset` is invoked can be
/// controlled by the number of non-NULL arguments.  If `fd_select` is
/// non-NULL but `off1` and `off2` are NULL, `cid_get_offset` is invoked
/// only for `fd_select`; `off1` and `off2` are not validated.
pub fn cid_compute_fd_and_offsets(
    cid: &CidFaceInfoRec,
    stream: &mut FtStreamRec,
    glyph_index: FtUInt,
    fd_select_p: Option<&mut FtULong>,
    off1_p: Option<&mut FtULong>,
    off2_p: Option<&mut FtULong>,
) -> FtResult<()> {
    let entry_len: FtUInt = cid.fd_bytes.wrapping_add(cid.gd_bytes);

    /* For ordinary fonts, read the CID font dictionary index */
    /* and charstring offset from the CIDMap.                 */

    stream.seek(
        cid.data_offset
            .wrapping_add(cid.cidmap_offset)
            .wrapping_add(glyph_index.wrapping_mul(entry_len) as FtULong),
    )?;
    stream.enter_frame(entry_len.wrapping_mul(2) as FtULong)?;

    let error = {
        let b = stream.frame_data();
        let mut p = 0usize;
        let fd_select = cid_get_offset(b, &mut p, cid.fd_bytes);
        let off1 = cid_get_offset(b, &mut p, cid.gd_bytes);

        p += cid.fd_bytes as usize;
        let off2 = cid_get_offset(b, &mut p, cid.gd_bytes);

        if let Some(f) = fd_select_p {
            *f = fd_select;
        }
        if let Some(o) = off1_p {
            *o = off1;
        }
        if let Some(o) = off2_p {
            *o = off2;
        }

        if fd_select >= cid.num_dicts as FtULong {
            /*
             * fd_select == 0xFF is often used to indicate that the CID
             * has no charstring to be rendered, similar to GID = 0xFFFF
             * in TrueType fonts.
             */
            Err(FT_ERR_INVALID_OFFSET)
        } else if off2 > stream.size {
            Err(FT_ERR_INVALID_OFFSET)
        } else if off1 > off2 {
            Err(FT_ERR_INVALID_OFFSET)
        } else {
            Ok(())
        }
    };

    /* Exit: */
    stream.exit_frame();

    error
}

/// `cid_load_glyph` (the face's records the decoder does not hold:
/// `cid`, `subrs`, its `cid_stream` and `random_seed`)
pub fn cid_load_glyph(
    decoder: &mut T1DecoderRec<'_>,
    cid: &CidFaceInfoRec,
    subrs: &[CidSubrsRec],
    stream: &mut FtStreamRec,
    random_seed: &mut FtInt32,
    glyph_index: FtUInt,
) -> FtResult<()> {
    let mut fd_select: FtULong = 0;
    let mut force_scaling = false;

    let error = (|| -> FtResult<()> {
        /* (no incremental interface) */
        let mut off1: FtULong = 0;
        let mut off2: FtULong = 0;

        cid_compute_fd_and_offsets(
            cid,
            stream,
            glyph_index,
            Some(&mut fd_select),
            Some(&mut off1),
            Some(&mut off2),
        )?;

        let glyph_length: FtULong = off2 - off1;

        if glyph_length == 0 {
            return Ok(());
        }
        let mut charstring = ft_qalloc(glyph_length as FtLong)?;
        stream.read_at(cid.data_offset.wrapping_add(off1), &mut charstring)?;

        /* Now set up the subrs array and parse the charstrings. */
        {
            let cid_subrs = &subrs[fd_select as usize];

            /* Set up subrs */
            decoder.num_subrs = cid_subrs.num_subrs;
            decoder.cid_subrs = cid_subrs.code.clone();
            decoder.cid_subrs_bytes = cid_subrs.code_bytes.clone();
            decoder.subrs = None;
            decoder.subrs_hash = None;

            /* Set up font matrix */
            let dict = &cid.font_dicts[fd_select as usize];

            decoder.font_matrix = dict.font_matrix;
            decoder.font_offset = dict.font_offset;
            decoder.lenIV = dict.private_dict.lenIV;

            /* Decode the charstring. */

            /* Adjustment for seed bytes. */
            let cs_offset: FtUInt = if decoder.lenIV >= 0 {
                decoder.lenIV as FtUInt
            } else {
                0
            };
            if cs_offset as FtULong > glyph_length {
                return Err(FT_ERR_INVALID_OFFSET);
            }

            /* Decrypt only if lenIV >= 0. */
            if decoder.lenIV >= 0 {
                t1_decrypt(&mut charstring, glyph_length as usize, 4330);
            }

            /* choose which renderer to use */
            /* (T1_CONFIG_OPTION_OLD_ENGINE is undefined) */
            if decoder.builder.builder.metrics_only {
                t1_decoder_parse_metrics(
                    decoder,
                    &charstring[cs_offset as usize..],
                    (glyph_length - cs_offset as FtULong) as FtUInt,
                )?;
            } else {
                let mut subfont = CffSubFontRec::default();

                t1_make_subfont(random_seed, &dict.private_dict, &mut subfont);

                let charstring: Arc<[u8]> = Arc::from(charstring);
                let mut psdecoder = ps_decoder_init_t1(decoder, &mut subfont);

                let mut error = cf2_decoder_parse_charstrings_at(
                    &mut psdecoder,
                    &charstring,
                    cs_offset as usize,
                    glyph_length - cs_offset as FtULong,
                );

                /* Adobe's engine uses 16.16 numbers everywhere;              */
                /* as a consequence, glyphs larger than 2000ppem get rejected */
                if error == Err(FT_ERR_GLYPH_TOO_BIG) {
                    /* this time, we retry unhinted and scale up the glyph later on */
                    /* (the engine uses and sets the hardcoded value 0x10000 / 64 = */
                    /* 0x400 for both `x_scale' and `y_scale' in this case)         */
                    if let Some(glyph) = psdecoder.builder.builder.glyph.as_mut() {
                        glyph.cff.hint = false;
                    }

                    force_scaling = true;

                    error = cf2_decoder_parse_charstrings_at(
                        &mut psdecoder,
                        &charstring,
                        cs_offset as usize,
                        glyph_length - cs_offset as FtULong,
                    );
                }
                error?;
            }
        }

        /* (no incremental interface overrides the metrics) */

        Ok(())
    })();

    /* Exit: */
    if let Some(glyph) = decoder.builder.builder.glyph.as_mut() {
        glyph.cff.scaled = force_scaling;
    }

    error
}

/// `cid_slot_load_glyph` (into the face's glyph slot, at its size)
pub fn cid_slot_load_glyph(
    cidface: &mut FtFace,
    glyph_index: FtUInt,
    mut load_flags: FtInt32,
) -> FtResult<()> {
    let FtFace::Cid(face) = cidface else {
        return Err(FT_ERR_INVALID_FACE_HANDLE);
    };

    /* (the face's slot and size exist together) */
    if !face.root.has_slot_and_size {
        return Err(FT_ERR_INVALID_SLOT_HANDLE);
    }

    if glyph_index >= face.root.num_glyphs as FtUInt {
        return Err(FT_ERR_INVALID_ARGUMENT);
    }

    if load_flags & FT_LOAD_NO_RECURSE != 0 {
        load_flags |= FT_LOAD_NO_SCALE | FT_LOAD_NO_HINTING;
    }

    face.glyph.x_scale = face.root.size.metrics.x_scale;
    face.glyph.y_scale = face.root.size.metrics.y_scale;

    face.root.glyph.outline.n_points = 0;
    face.root.glyph.outline.n_contours = 0;

    let mut hinting =
        (load_flags & FT_LOAD_NO_SCALE) == 0 && (load_flags & FT_LOAD_NO_HINTING) == 0;
    let mut scaled = (load_flags & FT_LOAD_NO_SCALE) == 0;

    face.glyph.hint = hinting;
    face.glyph.scaled = scaled;
    face.root.glyph.format = FT_GLYPH_FORMAT_OUTLINE;

    let builder_face = t1_builder_face(&face.root);
    let size_internal = Some(face.size_module_data.is_some());

    let left_bearing: FtVector;
    let advance: FtVector;
    let font_matrix: FtMatrix;
    let font_offset: FtVector;
    let hints_funcs: bool;
    {
        let CidFaceRec {
            root,
            psnames,
            cid,
            subrs,
            cid_stream,
            glyph,
            ..
        } = &mut **face;
        let FtFaceRec {
            glyph: root_glyph,
            internal,
            num_glyphs,
            stream: root_stream,
            ..
        } = root;

        let mut buildchar: [FtLong; 0] = [];
        let mut decoder = t1_decoder_init(
            *psnames,
            *num_glyphs,
            builder_face,
            size_internal,
            Some(CffGlyph {
                root: root_glyph,
                cff: glyph,
            }),
            None, /* glyph names -- XXX */
            None, /* blend == 0 */
            hinting,
            ((load_flags >> 16) & 15) as FtRenderMode, /* FT_LOAD_TARGET_MODE */
            &mut buildchar,
        )?;

        /* TODO: initialize decoder.len_buildchar and decoder.buildchar */
        /*       if we ever support CID-keyed multiple master fonts     */

        /* set up the decoder */
        decoder.builder.builder.no_recurse = load_flags & FT_LOAD_NO_RECURSE != 0;

        let Some(stream) = cid_stream.as_deref_mut().or(root_stream.as_deref_mut()) else {
            return Err(FT_ERR_INVALID_STREAM_HANDLE);
        };

        let error = cid_load_glyph(
            &mut decoder,
            cid,
            subrs,
            stream,
            &mut internal.random_seed,
            glyph_index,
        );

        /* save new glyph tables (on an error, too: `Exit') */
        t1_decoder_done(&mut decoder);

        error?;

        /* copy flags back for forced scaling */
        if let Some(g) = decoder.builder.builder.glyph.as_ref() {
            hinting = g.cff.hint;
            scaled = g.cff.scaled;
        }

        font_matrix = decoder.font_matrix;
        font_offset = decoder.font_offset;

        left_bearing = decoder.builder.builder.left_bearing;
        advance = decoder.builder.builder.advance;
        hints_funcs = decoder.builder.builder.hints_funcs;
    }

    /* now set the metrics -- this is rather simple, as    */
    /* the left side bearing is the xMin, and the top side */
    /* bearing the yMax                                    */
    let font_bbox = face.cid.font_bbox;
    let (x_scale, y_scale) = (face.glyph.x_scale, face.glyph.y_scale);
    let y_ppem = face.root.size.metrics.y_ppem;
    let cidglyph = &mut face.root.glyph;

    cidglyph.outline.flags &= FT_OUTLINE_OWNER;
    cidglyph.outline.flags |= FT_OUTLINE_REVERSE_FILL;

    /* for composite glyphs, return only left side bearing and */
    /* advance width                                           */
    if load_flags & FT_LOAD_NO_RECURSE != 0 {
        let internal = &mut cidglyph.internal;

        cidglyph.metrics.horiBearingX = fixed_to_int(left_bearing.x);
        cidglyph.metrics.horiAdvance = fixed_to_int(advance.x);

        internal.glyph_matrix = font_matrix;
        internal.glyph_delta = font_offset;
        internal.glyph_transformed = true;
    } else {
        /* copy the _unscaled_ advance width */
        cidglyph.metrics.horiAdvance = fixed_to_int(advance.x);
        cidglyph.linearHoriAdvance = fixed_to_int(advance.x);
        cidglyph.internal.glyph_transformed = false;

        /* make up vertical ones */
        cidglyph.metrics.vertAdvance = (font_bbox.yMax - font_bbox.yMin) >> 16;
        cidglyph.linearVertAdvance = cidglyph.metrics.vertAdvance;

        cidglyph.format = FT_GLYPH_FORMAT_OUTLINE;

        if y_ppem < 24 {
            cidglyph.outline.flags |= FT_OUTLINE_HIGH_PRECISION;
        }

        /* apply the font matrix, if any */
        if font_matrix.xx != 0x10000
            || font_matrix.yy != 0x10000
            || font_matrix.xy != 0
            || font_matrix.yx != 0
        {
            ft_outline_transform(&mut cidglyph.outline, &font_matrix);

            cidglyph.metrics.horiAdvance = ft_mul_fix(cidglyph.metrics.horiAdvance, font_matrix.xx);
            cidglyph.metrics.vertAdvance = ft_mul_fix(cidglyph.metrics.vertAdvance, font_matrix.yy);
        }

        if font_offset.x != 0 || font_offset.y != 0 {
            ft_outline_translate(&mut cidglyph.outline, font_offset.x, font_offset.y);

            cidglyph.metrics.horiAdvance += font_offset.x;
            cidglyph.metrics.vertAdvance += font_offset.y;
        }

        if (load_flags & FT_LOAD_NO_SCALE) == 0 || scaled {
            /* scale the outline and the metrics */
            /* (the builder's base outline is the slot's) */
            let cur = &mut cidglyph.outline;

            /* First of all, scale the points */
            if !hinting || !hints_funcs {
                let n = (cur.n_points.max(0) as usize).min(cur.points.len());
                for vec in cur.points[..n].iter_mut() {
                    vec.x = ft_mul_fix(vec.x, x_scale);
                    vec.y = ft_mul_fix(vec.y, y_scale);
                }
            }

            /* Then scale the metrics */
            cidglyph.metrics.horiAdvance = ft_mul_fix(cidglyph.metrics.horiAdvance, x_scale);
            cidglyph.metrics.vertAdvance = ft_mul_fix(cidglyph.metrics.vertAdvance, y_scale);
        }

        /* compute the other metrics */
        let cbox = ft_outline_get_cbox(&cidglyph.outline);

        let metrics = &mut cidglyph.metrics;
        metrics.width = cbox.xMax - cbox.xMin;
        metrics.height = cbox.yMax - cbox.yMin;

        metrics.horiBearingX = cbox.xMin;
        metrics.horiBearingY = cbox.yMax;

        if load_flags & FT_LOAD_VERTICAL_LAYOUT != 0 {
            /* make up vertical ones */
            let va = metrics.vertAdvance;
            ft_synthesize_vertical_metrics(metrics, va);
        }
    }

    /* Exit: */
    Ok(())
}
