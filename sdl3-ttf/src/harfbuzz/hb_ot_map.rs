// Rust translation of src/hb-ot-map.hh and src/hb-ot-map.cc from HarfBuzz
// (8.5.0, as SDL_ttf's external/harfbuzz pins it).
// Copyright © 2009,2010  Red Hat, Inc.
// Copyright © 2010,2011,2013  Google, Inc.
// This is an altered (translated) version of the original software; see
// LICENSE.txt (HarfBuzz's "Old MIT" license).
//
// Red Hat Author(s): Behdad Esfahbod
// Google Author(s): Behdad Esfahbod

//! The map of features to masks and lookups.

pub(crate) const HB_OT_MAP_MAX_BITS: u32 = 8;
pub(crate) const HB_OT_MAP_MAX_VALUE: u32 = (1 << HB_OT_MAP_MAX_BITS) - 1;
