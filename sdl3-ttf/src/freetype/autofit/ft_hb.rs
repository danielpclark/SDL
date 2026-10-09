// Rust translation of src/autofit/ft-hb.c and ft-hb.h from FreeType
// (2.13.2, as SDL_ttf's external/freetype pins it), with
// `FT_CONFIG_OPTION_USE_HARFBUZZ` defined.
// Copyright (C) 2009, 2023  Red Hat, Inc.
// Copyright (C) 2015  Google, Inc.
// This is an altered (translated) version of the original software. As
// FreeType's LICENSE.TXT notes, these files contain code taken almost
// verbatim from HarfBuzz's hb-ft.cc, under HarfBuzz's "Old MIT" license:
// this translation is used under that license (see HARFBUZZ-COPYING and
// LICENSE.txt).
//
// Red Hat Author(s): Behdad Esfahbod, Matthias Clasen
// Google Author(s): Behdad Esfahbod

//! FreeType's own HarfBuzz font creation (a copy of hb-ft's, without its
//! font functions: the font uses HarfBuzz's OpenType functions).
//!
//! Translation notes: the face's tables are loaded when it is created
//! (C's `hb_ft_reference_table_` loads them when HarfBuzz first asks);
//! FreeType's streams in SDL_ttf have a read function, so the C never
//! takes the memory-blob path.

use std::collections::HashMap;
use std::sync::Arc;

use super::super::base::ftobjs::*;
use super::super::fttypes::*;
use crate::harfbuzz::hb_common::HbTag;
use crate::harfbuzz::hb_face::{HbFace, HB_FACE_TABLES};
use crate::harfbuzz::hb_font::HbFontData;
use crate::harfbuzz::hb_ot_font::HB_OT_FONT_TABLES;

/// `hb_ft_reference_table_`
fn hb_ft_reference_table_(ft_face: &mut FtFace, tag: HbTag) -> Option<Vec<u8>> {
    let mut length: FtULong = 0;

    /* Note: FreeType like HarfBuzz uses the NONE tag for fetching the entire blob */

    if ft_load_sfnt_table(ft_face, tag as FtULong, 0, None, &mut length).is_err() {
        return None;
    }

    let mut buffer = Vec::new();
    if buffer.try_reserve_exact(length as usize).is_err() {
        return None;
    }
    buffer.resize(length as usize, 0);

    if ft_load_sfnt_table(ft_face, tag as FtULong, 0, Some(&mut buffer), &mut length).is_err() {
        return None;
    }

    Some(buffer)
}

/// `hb_ft_face_create_`
fn hb_ft_face_create_(ft_face: &mut FtFace) -> HbFace {
    /* (hb_face_create_for_tables (hb_ft_reference_table_, ...)) */
    let mut tables = HashMap::new();
    for &tag in HB_FACE_TABLES.iter().chain(HB_OT_FONT_TABLES.iter()) {
        if let Some(blob) = hb_ft_reference_table_(ft_face, tag) {
            tables.insert(tag, blob);
        }
    }

    /* hb_face_set_index (face, ft_face->face_index); */
    /* hb_face_set_upem (face, ft_face->units_per_EM); */
    HbFace::new_for_tables(
        tables,
        ft_face.face_index as u32,
        ft_face.units_per_EM as u32,
    )
}

/// `hb_ft_font_create_`
pub fn hb_ft_font_create_(ft_face: &mut FtFace) -> HbFontData {
    let face = hb_ft_face_create_(ft_face);
    HbFontData::hb_font_create(Arc::new(face))
}
