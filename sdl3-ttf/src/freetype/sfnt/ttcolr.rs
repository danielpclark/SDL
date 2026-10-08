// Rust translation of src/sfnt/ttcolr.c from FreeType (2.13.2, as SDL_ttf's
// external/freetype pins it).
// Copyright (C) 2018-2023 by David Turner, Robert Wilhelm, Dominik Röttsches,
// and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! TrueType and OpenType colored glyph layer support (body).
//!
//! `COLR' table specification:
//!
//!   <https://www.microsoft.com/typography/otspec/colr.htm>
//!
//! The `COLR' v1 paint graph accessors (`tt_face_get_colr_glyph_paint`,
//! `tt_face_get_color_glyph_clipbox`, `tt_face_get_paint_layers`,
//! `tt_face_get_colorline_stops`, `tt_face_get_paint`, and their helpers
//! `read_color_line`, `get_child_table_pointer`,
//! `get_deltas_for_var_index_base`, `read_paint`,
//! `find_base_glyph_v1_record`) are not translated yet: they only back the
//! public `FT_Get_Color_Glyph_Paint` family of functions, which SDL_ttf
//! never calls.  The table loader below validates the v1 header exactly as
//! C does, since that decides whether a face has color glyphs.

use super::super::base::ftobjs::*;
use super::super::base::ftstream::*;
use super::super::fttypes::*;
use super::super::truetype::ttgxvar::{
    tt_get_mm_var, tt_var_done_delta_set_index_map, tt_var_done_item_variation_store,
    tt_var_load_delta_set_index_mapping, tt_var_load_item_variation_store, GxDeltaSetIdxMapRec,
    GxItemVarStoreRec,
};
use super::super::tttables::*;
use super::super::tttypes::*;
use super::ttload::tt_face_goto_table;

/* NOTE: These are the table sizes calculated through the specs. */
const BASE_GLYPH_SIZE: FtULong = 6;
const BASE_GLYPH_PAINT_RECORD_SIZE: FtULong = 6;
const LAYER_V1_LIST_PAINT_OFFSET_SIZE: FtULong = 4;
#[allow(dead_code)]
const LAYER_V1_LIST_NUM_LAYERS_SIZE: FtULong = 4;
#[allow(dead_code)]
const COLOR_STOP_SIZE: FtULong = 6;
#[allow(dead_code)]
const VAR_IDX_BASE_SIZE: FtULong = 4;
const LAYER_SIZE: FtULong = 4;
/* https://docs.microsoft.com/en-us/typography/opentype/spec/colr#colr-header */
/* 3 * uint16 + 2 * Offset32 */
const COLRV0_HEADER_SIZE: FtULong = 14;
/* COLRV0_HEADER_SIZE + 5 * Offset32 */
const COLRV1_HEADER_SIZE: FtULong = 34;

/// `BaseGlyphRecord`
#[derive(Debug, Clone, Copy, Default)]
struct BaseGlyphRecord {
    gid: FtUShort,
    first_layer_index: FtUShort,
    num_layers: FtUShort,
}

/// `Colr`; the pointers into the table are offsets into `table`.
#[derive(Debug, Default)]
pub struct Colr {
    pub version: FtUShort,
    pub num_base_glyphs: FtUShort,
    pub num_layers: FtUShort,

    pub base_glyphs: usize,
    pub layers: usize,

    pub num_base_glyphs_v1: FtULong,
    /* Points at beginning of BaseGlyphV1List. */
    pub base_glyphs_v1: usize,

    pub num_layers_v1: FtULong,
    pub layers_v1: usize,

    pub clip_list: usize,

    /*
     * Paint tables start at the minimum of the end of the LayerList and the
     * end of the BaseGlyphList.  Record this location in a field here for
     * safety checks when accessing paint tables.
     */
    pub paints_start_v1: usize,

    /* TT_CONFIG_OPTION_GX_VAR_SUPPORT */
    /* Item Variation Store for variable 'COLR' v1. */
    pub var_store: GxItemVarStoreRec,
    pub delta_set_idx_map: GxDeltaSetIdxMapRec,

    /* The memory that backs up the `COLR' table. */
    pub table: Vec<u8>,
    pub table_size: FtULong,
}

/// `tt_face_load_colr`
pub fn tt_face_load_colr(face: &mut TtFaceRec, stream: &mut FtStreamRec) -> FtResult<()> {
    /* `COLR' always needs `CPAL' */
    if face.cpal.is_none() {
        return Err(FT_ERR_INVALID_FILE_FORMAT);
    }

    let table_size = tt_face_goto_table(face, TTAG_COLR as FtULong, stream)?;

    /* TT_CONFIG_OPTION_GX_VAR_SUPPORT */
    let colr_offset_in_stream = stream.pos();

    if table_size < COLRV0_HEADER_SIZE {
        /* NoColr: (with `error' still zero) */
        return Ok(());
    }

    let table = stream.extract_frame(table_size)?;
    let t = &table[..];
    let mut p = 0usize;

    let mut colr = Box::<Colr>::default();

    let r = (|| -> FtResult<()> {
        colr.version = ft_next_ushort(t, &mut p);
        if colr.version != 0 && colr.version != 1 {
            return Err(FT_ERR_INVALID_TABLE);
        }

        colr.num_base_glyphs = ft_next_ushort(t, &mut p);
        let base_glyph_offset = ft_next_ulong(t, &mut p) as FtULong;

        if base_glyph_offset >= table_size {
            return Err(FT_ERR_INVALID_TABLE);
        }
        if colr.num_base_glyphs as FtULong * BASE_GLYPH_SIZE > table_size - base_glyph_offset {
            return Err(FT_ERR_INVALID_TABLE);
        }

        let layer_offset = ft_next_ulong(t, &mut p) as FtULong;
        colr.num_layers = ft_next_ushort(t, &mut p);

        if layer_offset >= table_size {
            return Err(FT_ERR_INVALID_TABLE);
        }
        if colr.num_layers as FtULong * LAYER_SIZE > table_size - layer_offset {
            return Err(FT_ERR_INVALID_TABLE);
        }

        if colr.version == 1 {
            if table_size < COLRV1_HEADER_SIZE {
                return Err(FT_ERR_INVALID_TABLE);
            }

            let base_glyphs_offset_v1 = ft_next_ulong(t, &mut p) as FtULong;

            if base_glyphs_offset_v1 >= table_size - 4 {
                return Err(FT_ERR_INVALID_TABLE);
            }

            let p1 = base_glyphs_offset_v1 as usize;
            let num_base_glyphs_v1 = ft_peek_ulong(t, p1) as FtULong;

            if num_base_glyphs_v1 * BASE_GLYPH_PAINT_RECORD_SIZE
                > table_size - base_glyphs_offset_v1
            {
                return Err(FT_ERR_INVALID_TABLE);
            }

            colr.num_base_glyphs_v1 = num_base_glyphs_v1;
            colr.base_glyphs_v1 = p1;

            let layer_offset_v1 = ft_next_ulong(t, &mut p) as FtULong;

            if layer_offset_v1 >= table_size {
                return Err(FT_ERR_INVALID_TABLE);
            }

            if layer_offset_v1 != 0 {
                if layer_offset_v1 >= table_size - 4 {
                    return Err(FT_ERR_INVALID_TABLE);
                }

                let p1 = layer_offset_v1 as usize;
                let num_layers_v1 = ft_peek_ulong(t, p1) as FtULong;

                if num_layers_v1 * LAYER_V1_LIST_PAINT_OFFSET_SIZE > table_size - layer_offset_v1 {
                    return Err(FT_ERR_INVALID_TABLE);
                }

                colr.num_layers_v1 = num_layers_v1;
                colr.layers_v1 = p1;

                colr.paints_start_v1 = std::cmp::min(
                    colr.base_glyphs_v1
                        + (colr.num_base_glyphs_v1 * BASE_GLYPH_PAINT_RECORD_SIZE) as usize,
                    colr.layers_v1
                        + (colr.num_layers_v1 * LAYER_V1_LIST_PAINT_OFFSET_SIZE) as usize,
                );
            } else {
                colr.num_layers_v1 = 0;
                colr.layers_v1 = 0;
                colr.paints_start_v1 = colr.base_glyphs_v1
                    + (colr.num_base_glyphs_v1 * BASE_GLYPH_PAINT_RECORD_SIZE) as usize;
            }

            let clip_list_offset = ft_next_ulong(t, &mut p) as FtULong;

            if clip_list_offset >= table_size {
                return Err(FT_ERR_INVALID_TABLE);
            }

            colr.clip_list = clip_list_offset as usize;

            /* TT_CONFIG_OPTION_GX_VAR_SUPPORT */
            colr.var_store = GxItemVarStoreRec::default();
            colr.delta_set_idx_map = GxDeltaSetIdxMapRec::default();

            if face.variation_support & TT_FACE_FLAG_VAR_FVAR != 0 {
                let var_idx_map_offset = ft_next_ulong(t, &mut p) as FtULong;

                if var_idx_map_offset >= table_size {
                    return Err(FT_ERR_INVALID_TABLE);
                }

                let var_store_offset = ft_next_ulong(t, &mut p) as FtULong;
                if var_store_offset >= table_size {
                    return Err(FT_ERR_INVALID_TABLE);
                }

                if var_store_offset != 0 {
                    /* If variation info has not been initialized yet, try doing so, */
                    /* otherwise loading the variation store will fail as it         */
                    /* requires access to `blend` for checking the number of axes.   */
                    if face.blend.is_none() && tt_get_mm_var(face, stream, false).is_err() {
                        return Err(FT_ERR_INVALID_TABLE);
                    }

                    /* Try loading `VarIdxMap` and `VarStore`. */
                    if tt_var_load_item_variation_store(
                        face,
                        stream,
                        colr_offset_in_stream + var_store_offset,
                        &mut colr.var_store,
                    )
                    .is_err()
                    {
                        return Err(FT_ERR_INVALID_TABLE);
                    }
                }

                if colr.var_store.axisCount != 0
                    && var_idx_map_offset != 0
                    && tt_var_load_delta_set_index_mapping(
                        face,
                        stream,
                        colr_offset_in_stream + var_idx_map_offset,
                        &mut colr.delta_set_idx_map,
                        &colr.var_store,
                        table_size,
                    )
                    .is_err()
                {
                    return Err(FT_ERR_INVALID_TABLE);
                }
            }
        }

        colr.base_glyphs = base_glyph_offset as usize;
        colr.layers = layer_offset as usize;

        Ok(())
    })();

    match r {
        Ok(()) => {
            colr.table = table;
            colr.table_size = table_size;

            face.colr = Some(colr);

            Ok(())
        }
        Err(e) => {
            /* InvalidTable: */
            /* TT_CONFIG_OPTION_GX_VAR_SUPPORT */
            tt_var_done_delta_set_index_map(&mut colr.delta_set_idx_map);
            tt_var_done_item_variation_store(&mut colr.var_store);

            /* NoColr: */
            Err(e)
        }
    }
}

/// `tt_face_free_colr`
pub fn tt_face_free_colr(face: &mut TtFaceRec) {
    if let Some(mut colr) = face.colr.take() {
        /* TT_CONFIG_OPTION_GX_VAR_SUPPORT */
        tt_var_done_delta_set_index_map(&mut colr.delta_set_idx_map);
        tt_var_done_item_variation_store(&mut colr.var_store);
    }
}

/// `find_base_glyph_record`
fn find_base_glyph_record(
    table: &[u8],
    base_glyph_begin: usize,
    num_base_glyph: FtUInt,
    glyph_id: FtUInt,
    record: &mut BaseGlyphRecord,
) -> bool {
    let mut min: FtUInt = 0;
    let mut max = num_base_glyph;

    while min < max {
        let mid = min + (max - min) / 2;
        let mut p = base_glyph_begin + (mid as usize) * BASE_GLYPH_SIZE as usize;

        let gid = ft_next_ushort(table, &mut p);

        if (gid as FtUInt) < glyph_id {
            min = mid + 1;
        } else if gid as FtUInt > glyph_id {
            max = mid;
        } else {
            record.gid = gid;
            record.first_layer_index = ft_next_ushort(table, &mut p);
            record.num_layers = ft_next_ushort(table, &mut p);

            return true;
        }
    }

    false
}

/// `tt_face_get_colr_layer`
pub fn tt_face_get_colr_layer(
    face: &TtFaceRec,
    base_glyph: FtUInt,
    aglyph_index: &mut FtUInt,
    acolor_index: &mut FtUInt,
    iterator: &mut FtLayerIterator,
) -> bool {
    let colr = match &face.colr {
        Some(c) => c,
        None => return false,
    };

    if iterator.p.is_none() {
        let mut glyph_record = BaseGlyphRecord::default();

        /* first call to function */
        iterator.layer = 0;

        if !find_base_glyph_record(
            &colr.table,
            colr.base_glyphs,
            colr.num_base_glyphs as FtUInt,
            base_glyph,
            &mut glyph_record,
        ) {
            return false;
        }

        if glyph_record.num_layers != 0 {
            iterator.num_layers = glyph_record.num_layers as FtUInt;
        } else {
            return false;
        }

        let offset = LAYER_SIZE * glyph_record.first_layer_index as FtULong;
        if offset + LAYER_SIZE * glyph_record.num_layers as FtULong > colr.table_size {
            return false;
        }

        iterator.p = Some(colr.layers + offset as usize);
    }

    let mut p = iterator.p.unwrap_or(0);
    if iterator.layer >= iterator.num_layers || p < colr.layers || p >= colr.table_size as usize {
        return false;
    }

    *aglyph_index = ft_next_ushort(&colr.table, &mut p) as FtUInt;
    *acolor_index = ft_next_ushort(&colr.table, &mut p) as FtUInt;
    iterator.p = Some(p);

    if *aglyph_index >= face.root.num_glyphs as FtUInt
        || (*acolor_index != 0xFFFF
            && *acolor_index >= face.palette_data.num_palette_entries as FtUInt)
    {
        return false;
    }

    iterator.layer += 1;

    true
}

/// `tt_face_colr_blend_layer`
pub fn tt_face_colr_blend_layer(
    face: &TtFaceRec,
    color_index: FtUInt,
    dst_slot: &mut FtGlyphSlotRec,
    src_slot: &FtGlyphSlotRec,
) -> FtResult<()> {
    let (b, g, r, alpha): (FtByte, FtByte, FtByte, FtByte);

    if dst_slot.bitmap.buffer.is_empty() {
        /* Initialize destination of color bitmap */
        /* with the size of first component.      */
        dst_slot.bitmap_left = src_slot.bitmap_left;
        dst_slot.bitmap_top = src_slot.bitmap_top;

        dst_slot.bitmap.width = src_slot.bitmap.width;
        dst_slot.bitmap.rows = src_slot.bitmap.rows;
        dst_slot.bitmap.pixel_mode = FT_PIXEL_MODE_BGRA;
        dst_slot.bitmap.pitch = (dst_slot.bitmap.width as i32).wrapping_mul(4);
        dst_slot.bitmap.num_grays = 256;

        let size = (dst_slot.bitmap.rows).wrapping_mul(dst_slot.bitmap.pitch as u32) as FtULong;

        ft_glyphslot_alloc_bitmap(dst_slot, size)?;

        dst_slot.bitmap.buffer.fill(0);
    } else {
        /* Resize destination if needed such that new component fits. */
        let x_min = std::cmp::min(dst_slot.bitmap_left, src_slot.bitmap_left);
        let x_max = std::cmp::max(
            dst_slot.bitmap_left + dst_slot.bitmap.width as FtInt,
            src_slot.bitmap_left + src_slot.bitmap.width as FtInt,
        );

        let y_min = std::cmp::min(
            dst_slot.bitmap_top - dst_slot.bitmap.rows as FtInt,
            src_slot.bitmap_top - src_slot.bitmap.rows as FtInt,
        );
        let y_max = std::cmp::max(dst_slot.bitmap_top, src_slot.bitmap_top);

        if x_min != dst_slot.bitmap_left
            || x_max != dst_slot.bitmap_left + dst_slot.bitmap.width as FtInt
            || y_min != dst_slot.bitmap_top - dst_slot.bitmap.rows as FtInt
            || y_max != dst_slot.bitmap_top
        {
            let width = (x_max - x_min) as FtUInt;
            let rows = (y_max - y_min) as FtUInt;
            let pitch = width.wrapping_mul(4);

            let size = rows.wrapping_mul(pitch) as FtULong;
            let mut buf = super::super::base::ftmemory::ft_alloc(size as FtLong)?;

            let mut p = 0usize;
            let mut q = (pitch as FtInt * (y_max - dst_slot.bitmap_top)
                + 4 * (dst_slot.bitmap_left - x_min)) as usize;

            let w4 = dst_slot.bitmap.width as usize * 4;
            for _ in 0..dst_slot.bitmap.rows {
                buf[q..q + w4].copy_from_slice(&dst_slot.bitmap.buffer[p..p + w4]);

                p = (p as isize + dst_slot.bitmap.pitch as isize) as usize;
                q += pitch as usize;
            }

            ft_glyphslot_set_bitmap(dst_slot, buf);

            dst_slot.bitmap_top = y_max;
            dst_slot.bitmap_left = x_min;

            dst_slot.bitmap.width = width;
            dst_slot.bitmap.rows = rows;
            dst_slot.bitmap.pitch = pitch as i32;

            dst_slot.internal.flags |= FT_GLYPH_OWN_BITMAP;
            dst_slot.format = FT_GLYPH_FORMAT_BITMAP;
        }
    }

    if color_index == 0xFFFF {
        if face.have_foreground_color {
            b = face.foreground_color.blue;
            g = face.foreground_color.green;
            r = face.foreground_color.red;
            alpha = face.foreground_color.alpha;
        } else if face
            .palette_data
            .palette_flags
            .as_ref()
            .is_some_and(|f| f[face.palette_index as usize] & FT_PALETTE_FOR_DARK_BACKGROUND != 0)
        {
            /* white opaque */
            b = 0xFF;
            g = 0xFF;
            r = 0xFF;
            alpha = 0xFF;
        } else {
            /* black opaque */
            b = 0x00;
            g = 0x00;
            r = 0x00;
            alpha = 0xFF;
        }
    } else {
        let c = face.palette[color_index as usize];
        b = c.blue;
        g = c.green;
        r = c.red;
        alpha = c.alpha;
    }

    /* XXX Convert if srcSlot.bitmap is not grey? */
    let src_buf = &src_slot.bitmap.buffer;
    let mut src = 0isize;
    let mut dst = dst_slot.bitmap.pitch as isize
        * (dst_slot.bitmap_top - src_slot.bitmap_top) as isize
        + 4 * (src_slot.bitmap_left - dst_slot.bitmap_left) as isize;
    let dst_buf = &mut dst_slot.bitmap.buffer;

    for _ in 0..src_slot.bitmap.rows {
        for x in 0..src_slot.bitmap.width as usize {
            let aa = src_buf[src as usize + x] as i32;
            let fa = alpha as i32 * aa / 255;

            let fb = b as i32 * fa / 255;
            let fg = g as i32 * fa / 255;
            let fr = r as i32 * fa / 255;

            let ba2 = 255 - fa;

            let d = dst as usize + 4 * x;
            let bb = dst_buf[d] as i32;
            let bg = dst_buf[d + 1] as i32;
            let br = dst_buf[d + 2] as i32;
            let ba = dst_buf[d + 3] as i32;

            dst_buf[d] = (bb * ba2 / 255 + fb) as FtByte;
            dst_buf[d + 1] = (bg * ba2 / 255 + fg) as FtByte;
            dst_buf[d + 2] = (br * ba2 / 255 + fr) as FtByte;
            dst_buf[d + 3] = (ba * ba2 / 255 + fa) as FtByte;
        }

        src += src_slot.bitmap.pitch as isize;
        dst += dst_slot.bitmap.pitch as isize;
    }

    Ok(())
}
