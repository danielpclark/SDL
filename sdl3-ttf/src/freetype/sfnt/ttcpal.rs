// Rust translation of src/sfnt/ttcpal.c from FreeType (2.13.2, as SDL_ttf's
// external/freetype pins it).
// Copyright (C) 2018-2023 by David Turner, Robert Wilhelm, Dominik Röttsches,
// and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! TrueType and OpenType color palette support (body).
//!
//! `CPAL' table specification:
//!
//!   <https://www.microsoft.com/typography/otspec/cpal.htm>

use super::super::base::ftmemory::ft_new_array;
use super::super::base::ftstream::*;
use super::super::fttypes::*;
use super::super::tttables::*;
use super::super::tttypes::*;
use super::ttload::tt_face_goto_table;

/* NOTE: These are the table sizes calculated through the specs. */
const CPAL_V0_HEADER_BASE_SIZE: FtULong = 12;
const COLOR_SIZE: FtULong = 4;

/// `Cpal`: all data from `CPAL' not covered in FT_Palette_Data
#[derive(Debug, Default)]
pub struct Cpal {
    pub version: FtUShort,    /* Table version number (0 or 1 supported). */
    pub num_colors: FtUShort, /* Total number of color records, */
    /* combined for all palettes.     */
    /// offset of the RGBA array of colors in `table`
    pub colors: usize,
    /// offset of the index of each palette's first color record in the
    /// combined color record array
    pub color_indices: usize,

    /* The memory which backs up the `CPAL' table. */
    pub table: Vec<u8>,
    pub table_size: FtULong,
}

/// `tt_face_load_cpal`
pub fn tt_face_load_cpal(face: &mut TtFaceRec, stream: &mut FtStreamRec) -> FtResult<()> {
    let r = (|| -> FtResult<()> {
        let table_size = tt_face_goto_table(face, TTAG_CPAL as FtULong, stream)?;

        if table_size < CPAL_V0_HEADER_BASE_SIZE {
            return Err(FT_ERR_INVALID_TABLE);
        }

        let table = stream.extract_frame(table_size)?;
        let mut p = 0usize;

        let mut cpal = Box::<Cpal>::default();

        cpal.version = ft_next_ushort(&table, &mut p);
        if cpal.version > 1 {
            return Err(FT_ERR_INVALID_TABLE);
        }

        face.palette_data.num_palette_entries = ft_next_ushort(&table, &mut p);
        face.palette_data.num_palettes = ft_next_ushort(&table, &mut p);

        cpal.num_colors = ft_next_ushort(&table, &mut p);
        let colors_offset = ft_next_ulong(&table, &mut p) as FtULong;

        if CPAL_V0_HEADER_BASE_SIZE + face.palette_data.num_palettes as FtULong * 2 > table_size {
            return Err(FT_ERR_INVALID_TABLE);
        }

        if colors_offset >= table_size {
            return Err(FT_ERR_INVALID_TABLE);
        }
        if cpal.num_colors as FtULong * COLOR_SIZE > table_size - colors_offset {
            return Err(FT_ERR_INVALID_TABLE);
        }

        if face.palette_data.num_palette_entries > cpal.num_colors {
            return Err(FT_ERR_INVALID_TABLE);
        }

        cpal.color_indices = p;
        cpal.colors = colors_offset as usize;

        if cpal.version == 1 {
            if CPAL_V0_HEADER_BASE_SIZE + face.palette_data.num_palettes as FtULong * 2 + 3 * 4
                > table_size
            {
                return Err(FT_ERR_INVALID_TABLE);
            }

            p += face.palette_data.num_palettes as usize * 2;

            let type_offset = ft_next_ulong(&table, &mut p) as FtULong;
            let label_offset = ft_next_ulong(&table, &mut p) as FtULong;
            let entry_label_offset = ft_next_ulong(&table, &mut p) as FtULong;

            if type_offset != 0 {
                if type_offset >= table_size {
                    return Err(FT_ERR_INVALID_TABLE);
                }
                if face.palette_data.num_palettes as FtULong * 2 > table_size - type_offset {
                    return Err(FT_ERR_INVALID_TABLE);
                }

                let mut array: Vec<FtUShort> =
                    ft_new_array(face.palette_data.num_palettes as FtLong)?;

                let mut p = type_offset as usize;
                for q in array.iter_mut() {
                    *q = ft_next_ushort(&table, &mut p);
                }

                face.palette_data.palette_flags = Some(array);
            }

            if label_offset != 0 {
                if label_offset >= table_size {
                    return Err(FT_ERR_INVALID_TABLE);
                }
                if face.palette_data.num_palettes as FtULong * 2 > table_size - label_offset {
                    return Err(FT_ERR_INVALID_TABLE);
                }

                let mut array: Vec<FtUShort> =
                    ft_new_array(face.palette_data.num_palettes as FtLong)?;

                let mut p = label_offset as usize;
                for q in array.iter_mut() {
                    *q = ft_next_ushort(&table, &mut p);
                }

                face.palette_data.palette_name_ids = Some(array);
            }

            if entry_label_offset != 0 {
                if entry_label_offset >= table_size {
                    return Err(FT_ERR_INVALID_TABLE);
                }
                if face.palette_data.num_palette_entries as FtULong * 2
                    > table_size - entry_label_offset
                {
                    return Err(FT_ERR_INVALID_TABLE);
                }

                let mut array: Vec<FtUShort> =
                    ft_new_array(face.palette_data.num_palette_entries as FtLong)?;

                let mut p = entry_label_offset as usize;
                for q in array.iter_mut() {
                    *q = ft_next_ushort(&table, &mut p);
                }

                face.palette_data.palette_entry_name_ids = Some(array);
            }
        }

        cpal.table = table;
        cpal.table_size = table_size;

        face.cpal = Some(cpal);

        /* set up default palette */
        face.palette = ft_new_array(face.palette_data.num_palette_entries as FtLong)?;

        if tt_face_palette_set(face, 0).is_err() {
            return Err(FT_ERR_INVALID_TABLE);
        }

        Ok(())
    })();

    if r.is_err() {
        /* NoCpal: */
        face.cpal = None;

        /* arrays in `face->palette_data' and `face->palette' */
        /* are freed in `sfnt_done_face'                      */
    }

    r
}

/// `tt_face_free_cpal`
pub fn tt_face_free_cpal(face: &mut TtFaceRec) {
    face.cpal = None;
}

/// `tt_face_palette_set`
pub fn tt_face_palette_set(face: &mut TtFaceRec, palette_index: FtUInt) -> FtResult<()> {
    let cpal = match &face.cpal {
        Some(c) if palette_index < face.palette_data.num_palettes as FtUInt => c,
        _ => return Err(FT_ERR_INVALID_ARGUMENT),
    };

    let offset = cpal.color_indices + 2 * palette_index as usize;
    let color_index = ft_peek_ushort(&cpal.table, offset);

    if color_index as FtUInt + face.palette_data.num_palette_entries as FtUInt
        > cpal.num_colors as FtUInt
    {
        return Err(FT_ERR_INVALID_TABLE);
    }

    let mut p = cpal.colors + COLOR_SIZE as usize * color_index as usize;
    for q in face.palette.iter_mut() {
        q.blue = ft_next_byte(&cpal.table, &mut p);
        q.green = ft_next_byte(&cpal.table, &mut p);
        q.red = ft_next_byte(&cpal.table, &mut p);
        q.alpha = ft_next_byte(&cpal.table, &mut p);
    }

    Ok(())
}
