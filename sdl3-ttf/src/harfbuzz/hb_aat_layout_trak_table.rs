// Rust translation of src/hb-aat-layout-trak-table.hh from HarfBuzz
// (8.5.0, as SDL_ttf's external/harfbuzz pins it).
// Copyright © 2018  Ebrahim Byagowi
// Copyright © 2018  Google, Inc.
// This is an altered (translated) version of the original software; see
// LICENSE.txt (HarfBuzz's "Old MIT" license).
//
// Google Author(s): Behdad Esfahbod

//! trak -- Tracking
//! <https://developer.apple.com/fonts/TrueType-Reference-Manual/RM06/Chap6trak.html>
//!
//! Translation notes: the table is the slice of its (sanitized) data;
//! the structures are positions in it.

use super::hb_aat_layout_common::HbAatApplyContext;
use super::hb_algs::hb_roundf;
use super::hb_common::*;
use super::hb_open_type::*;
use super::hb_ot_layout::foreach_grapheme;
use super::hb_sanitize::HbSanitizeContext;

/// `HB_AAT_TAG_trak`
pub(crate) const HB_AAT_TAG_TRAK: HbTag = hb_tag(b't', b'r', b'a', b'k');

/// `F16DOT16::to_float`
#[inline]
fn f16dot16_to_float(v: u32) -> f32 {
    (v as i32 as f32 + 0.0) / 65536.0
}

/* TrackTableEntry: track (F16DOT16, track value for this record),
 * trackNameID (NameID, the 'name' table index for this track (a short
 * word or phrase like "loose" or "very tight")), valuesZ
 * (NNOffset16To<UnsizedArrayOf<FWORD>>, offset from start of tracking
 * table to per-size tracking values for this track): 8 bytes. */

/// `TrackTableEntry::get_track_value`
#[inline]
fn track_table_entry_get_track_value(d: &[u8], e: usize) -> f32 {
    f16dot16_to_float(u32_at(d, e))
}

/// `TrackTableEntry::get_value`: `(base+valuesZ).as_array (table_size)[index]`
#[inline]
fn track_table_entry_get_value(d: &[u8], e: usize, index: u32, table_size: u32) -> i32 {
    if index >= table_size {
        return 0; /* (hb_array_t's Null) */
    }
    i16_at(d, u16_at(d, e + 6) as usize + index as usize * 2) as i32
}

/// `TrackTableEntry::sanitize (c, base, table_size)`
fn track_table_entry_sanitize(
    c: &mut HbSanitizeContext,
    e: usize,
    base: usize,
    table_size: u32,
) -> bool {
    c.check_struct(e, 8)
        && c.sanitize_nn_offset16(e + 6, base, |c, values| {
            c.check_array(values, table_size, 2)
        })
}

/* TrackData: nTracks (HBUINT16, number of separate tracks included in
 * this table), nSizes (HBUINT16, number of point sizes included in this
 * table), sizeTable (NNOffset32To<UnsizedArrayOf<F16DOT16>>, offset from
 * start of the tracking table to Array[nSizes] of size values..),
 * trackTable (UnsizedArrayOf<TrackTableEntry>, Array[nTracks] of
 * TrackTableEntry records). */

/// `TrackData::interpolate_at`
fn track_data_interpolate_at(
    d: &[u8],
    td: usize,
    idx: u32,
    target_size: f32,
    track_table_entry: usize,
) -> f32 {
    let sizes = u16_at(d, td + 2) as u32;
    let size_table = u32_at(d, td + 4) as usize;
    /* (hb_array_t's Null past the end) */
    let size_at = |i: u32| -> f32 {
        if i < sizes {
            f16dot16_to_float(u32_at(d, size_table + i as usize * 4))
        } else {
            f16dot16_to_float(0)
        }
    };

    let s0 = size_at(idx);
    let s1 = size_at(idx + 1);
    let t = if s0 == s1 {
        0.0
    } else {
        (target_size - s0) / (s1 - s0)
    };
    t * track_table_entry_get_value(d, track_table_entry, idx + 1, sizes) as f32
        + (1.0 - t) * track_table_entry_get_value(d, track_table_entry, idx, sizes) as f32
}

/// `TrackData::get_tracking`: the track data at `td` (None: the Null
/// one, which has no tracks) of the table `d` (its `base`).
fn track_data_get_tracking(d: &[u8], td: Option<usize>, ptem: f32) -> i32 {
    let Some(td) = td else {
        return 0;
    };
    /*
     * Choose track.
     */
    let mut track_table_entry = None;
    let count = u16_at(d, td) as usize;
    for i in 0..count {
        /* Note: Seems like the track entries are sorted by values.  But the
         * spec doesn't explicitly say that.  It just mentions it in the example. */

        /* For now we only seek for track entries with zero tracking value */

        let e = td + 8 + i * 8;
        if track_table_entry_get_track_value(d, e) == 0.0 {
            track_table_entry = Some(e);
            break;
        }
    }
    let Some(track_table_entry) = track_table_entry else {
        return 0;
    };

    /*
     * Choose size.
     */
    let sizes = u16_at(d, td + 2) as u32;
    if sizes == 0 {
        return 0;
    }
    if sizes == 1 {
        return track_table_entry_get_value(d, track_table_entry, 0, sizes);
    }

    let size_table = u32_at(d, td + 4) as usize;
    let mut size_index = 0;
    while size_index < sizes - 1 {
        if f16dot16_to_float(u32_at(d, size_table + size_index as usize * 4)) >= ptem {
            break;
        }
        size_index += 1;
    }

    hb_roundf(track_data_interpolate_at(
        d,
        td,
        if size_index != 0 { size_index - 1 } else { 0 },
        ptem,
        track_table_entry,
    )) as i32
}

/// `TrackData::sanitize (c, base)`
fn track_data_sanitize(c: &mut HbSanitizeContext, td: usize, base: usize) -> bool {
    c.check_struct(td, 8) && {
        let n_tracks = c.u16(td) as u32;
        let n_sizes = c.u16(td + 2) as u32;
        /* sizeTable.sanitize (c, base, nSizes) */
        c.check_struct(td + 4, 4)
            && c.check_array(base + c.u32(td + 4) as usize, n_sizes, 4)
            /* trackTable.sanitize (c, nTracks, base, nSizes) */
            && c.check_array(td + 8, n_tracks, 8)
            && (0..n_tracks as usize)
                .all(|i| track_table_entry_sanitize(c, td + 8 + i * 8, base, n_sizes))
    }
}

/// The position of an `Offset16To<TrackData>`'s target (None: null).
#[inline]
fn offset16(d: &[u8], at: usize) -> Option<usize> {
    match u16_at(d, at) {
        0 => None,
        off => Some(off as usize),
    }
}

/// `trak::has_data`
pub(crate) fn trak_has_data(d: &[u8]) -> bool {
    /* version.to_int () */
    u32_at(d, 0) != 0
}

/// `trak::apply`
pub(crate) fn trak_apply(d: &[u8], c: &mut HbAatApplyContext) -> bool {
    let trak_mask = c.plan.trak_mask;

    let ptem = c.font.p.ptem;
    if ptem <= 0.0 {
        return false;
    }

    let buffer = &mut *c.buffer;
    if hb_direction_is_horizontal(buffer.props.direction) {
        let tracking = track_data_get_tracking(d, offset16(d, 6) /* this+horizData */, ptem);
        let offset_to_add = c.font.em_scalef_x((tracking / 2) as f32);
        let advance_to_add = c.font.em_scalef_x(tracking as f32);
        for (start, _end) in foreach_grapheme(buffer) {
            if buffer.info[start as usize].mask & trak_mask == 0 {
                continue;
            }
            let pos = &mut buffer.pos[start as usize];
            pos.x_advance = pos.x_advance.wrapping_add(advance_to_add);
            pos.x_offset = pos.x_offset.wrapping_add(offset_to_add);
        }
    } else {
        let tracking = track_data_get_tracking(d, offset16(d, 8) /* this+vertData */, ptem);
        let offset_to_add = c.font.em_scalef_y((tracking / 2) as f32);
        let advance_to_add = c.font.em_scalef_y(tracking as f32);
        for (start, _end) in foreach_grapheme(buffer) {
            if buffer.info[start as usize].mask & trak_mask == 0 {
                continue;
            }
            let pos = &mut buffer.pos[start as usize];
            pos.y_advance = pos.y_advance.wrapping_add(advance_to_add);
            pos.y_offset = pos.y_offset.wrapping_add(offset_to_add);
        }
    }

    true
}

/// `trak::sanitize`
pub(crate) fn trak_sanitize(c: &mut HbSanitizeContext) -> bool {
    c.check_struct(0, 12)
        && c.u16(0) == 1 /* version.major */
        && c.sanitize_offset16(6, 0, |c, td| track_data_sanitize(c, td, 0))
        && c.sanitize_offset16(8, 0, |c, td| track_data_sanitize(c, td, 0))
}
