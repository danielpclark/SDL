// Rust translation of src/hb-ot-cmap-table.hh from HarfBuzz (8.5.0, as
// SDL_ttf's external/harfbuzz pins it): the subtables' lookups, the
// subtable selection and the accelerator.
// Copyright © 2014  Google, Inc.
// This is an altered (translated) version of the original software; see
// LICENSE.txt (HarfBuzz's "Old MIT" license).
//
// Google Author(s): Behdad Esfahbod

//! cmap -- Character to Glyph Index Mapping
//! <https://docs.microsoft.com/en-us/typography/opentype/spec/cmap>
//!
//! Translation notes: the table is read in place from its sanitized
//! bytes, a subtable being its offset in them. The subsetter's parts
//! (serializing, collecting unicodes and mappings) are not translated, nor
//! is the lookup cache (it only remembers results).

use super::hb_common::*;
use super::hb_face::{HbFace, FONT_PAGE_NONE, FONT_PAGE_SIMP_ARABIC, FONT_PAGE_TRAD_ARABIC};
use super::hb_open_type::*;
use super::hb_ot_shaper_arabic_pua::{_hb_arabic_pua_simp_map, _hb_arabic_pua_trad_map};
use super::hb_sanitize::{hb_sanitize_blob, HbSanitizeContext};

/// `HB_OT_TAG_cmap`
pub(crate) const HB_OT_TAG_CMAP: HbTag = hb_tag(b'c', b'm', b'a', b'p');

/// `unicode_to_macroman_t`'s `mapping`
static MAPPING: [(u16, u8); 128] = [
    (0x00A0, 0xCA),
    (0x00A1, 0xC1),
    (0x00A2, 0xA2),
    (0x00A3, 0xA3),
    (0x00A5, 0xB4),
    (0x00A7, 0xA4),
    (0x00A8, 0xAC),
    (0x00A9, 0xA9),
    (0x00AA, 0xBB),
    (0x00AB, 0xC7),
    (0x00AC, 0xC2),
    (0x00AE, 0xA8),
    (0x00AF, 0xF8),
    (0x00B0, 0xA1),
    (0x00B1, 0xB1),
    (0x00B4, 0xAB),
    (0x00B5, 0xB5),
    (0x00B6, 0xA6),
    (0x00B7, 0xE1),
    (0x00B8, 0xFC),
    (0x00BA, 0xBC),
    (0x00BB, 0xC8),
    (0x00BF, 0xC0),
    (0x00C0, 0xCB),
    (0x00C1, 0xE7),
    (0x00C2, 0xE5),
    (0x00C3, 0xCC),
    (0x00C4, 0x80),
    (0x00C5, 0x81),
    (0x00C6, 0xAE),
    (0x00C7, 0x82),
    (0x00C8, 0xE9),
    (0x00C9, 0x83),
    (0x00CA, 0xE6),
    (0x00CB, 0xE8),
    (0x00CC, 0xED),
    (0x00CD, 0xEA),
    (0x00CE, 0xEB),
    (0x00CF, 0xEC),
    (0x00D1, 0x84),
    (0x00D2, 0xF1),
    (0x00D3, 0xEE),
    (0x00D4, 0xEF),
    (0x00D5, 0xCD),
    (0x00D6, 0x85),
    (0x00D8, 0xAF),
    (0x00D9, 0xF4),
    (0x00DA, 0xF2),
    (0x00DB, 0xF3),
    (0x00DC, 0x86),
    (0x00DF, 0xA7),
    (0x00E0, 0x88),
    (0x00E1, 0x87),
    (0x00E2, 0x89),
    (0x00E3, 0x8B),
    (0x00E4, 0x8A),
    (0x00E5, 0x8C),
    (0x00E6, 0xBE),
    (0x00E7, 0x8D),
    (0x00E8, 0x8F),
    (0x00E9, 0x8E),
    (0x00EA, 0x90),
    (0x00EB, 0x91),
    (0x00EC, 0x93),
    (0x00ED, 0x92),
    (0x00EE, 0x94),
    (0x00EF, 0x95),
    (0x00F1, 0x96),
    (0x00F2, 0x98),
    (0x00F3, 0x97),
    (0x00F4, 0x99),
    (0x00F5, 0x9B),
    (0x00F6, 0x9A),
    (0x00F7, 0xD6),
    (0x00F8, 0xBF),
    (0x00F9, 0x9D),
    (0x00FA, 0x9C),
    (0x00FB, 0x9E),
    (0x00FC, 0x9F),
    (0x00FF, 0xD8),
    (0x0131, 0xF5),
    (0x0152, 0xCE),
    (0x0153, 0xCF),
    (0x0178, 0xD9),
    (0x0192, 0xC4),
    (0x02C6, 0xF6),
    (0x02C7, 0xFF),
    (0x02D8, 0xF9),
    (0x02D9, 0xFA),
    (0x02DA, 0xFB),
    (0x02DB, 0xFE),
    (0x02DC, 0xF7),
    (0x02DD, 0xFD),
    (0x03A9, 0xBD),
    (0x03C0, 0xB9),
    (0x2013, 0xD0),
    (0x2014, 0xD1),
    (0x2018, 0xD4),
    (0x2019, 0xD5),
    (0x201A, 0xE2),
    (0x201C, 0xD2),
    (0x201D, 0xD3),
    (0x201E, 0xE3),
    (0x2020, 0xA0),
    (0x2021, 0xE0),
    (0x2022, 0xA5),
    (0x2026, 0xC9),
    (0x2030, 0xE4),
    (0x2039, 0xDC),
    (0x203A, 0xDD),
    (0x2044, 0xDA),
    (0x20AC, 0xDB),
    (0x2122, 0xAA),
    (0x2202, 0xB6),
    (0x2206, 0xC6),
    (0x220F, 0xB8),
    (0x2211, 0xB7),
    (0x221A, 0xC3),
    (0x221E, 0xB0),
    (0x222B, 0xBA),
    (0x2248, 0xC5),
    (0x2260, 0xAD),
    (0x2264, 0xB2),
    (0x2265, 0xB3),
    (0x25CA, 0xD7),
    (0xF8FF, 0xF0),
    (0xFB01, 0xDE),
    (0xFB02, 0xDF),
];

/// `unicode_to_macroman`
fn unicode_to_macroman(u: HbCodepoint) -> u8 {
    /* (hb_bsearch with _hb_cmp_operator<uint16_t, uint16_t>: the key is
     * converted to uint16_t) */
    let key = u as u16;
    match hb_bsearch_impl(MAPPING.len(), |mid| {
        let v = MAPPING[mid].0;
        if key < v {
            -1
        } else if key > v {
            1
        } else {
            0
        }
    }) {
        Ok(i) => MAPPING[i as usize].1,
        Err(_) => 0,
    }
}

/// `CmapSubtableFormat0::get_glyph`
fn format0_get_glyph(t: &[u8], st: usize, codepoint: HbCodepoint, glyph: &mut HbCodepoint) -> bool {
    let gid = if codepoint < 256 {
        u8_at(t, st + 6 + codepoint as usize) as HbCodepoint
    } else {
        0
    };
    if gid == 0 {
        return false;
    }
    *glyph = gid;
    true
}

/// `CmapSubtableFormat4::accelerator_t` (positions in the table)
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Format4Accel {
    end_count: usize,
    start_count: usize,
    id_delta: usize,
    id_range_offset: usize,
    glyph_id_array: usize,
    seg_count: u32,
    glyph_id_array_length: u32,
}

impl Format4Accel {
    /// `init`
    fn init(t: &[u8], st: usize) -> Format4Accel {
        let seg_count = u16_at(t, st + 6) as u32 / 2;
        let end_count = st + 14;
        let start_count = end_count + 2 * (seg_count as usize + 1);
        let id_delta = start_count + 2 * seg_count as usize;
        let id_range_offset = id_delta + 2 * seg_count as usize;
        let glyph_id_array = id_range_offset + 2 * seg_count as usize;
        let length = u16_at(t, st + 2) as u32;
        Format4Accel {
            end_count,
            start_count,
            id_delta,
            id_range_offset,
            glyph_id_array,
            seg_count,
            glyph_id_array_length: length.wrapping_sub(16).wrapping_sub(8 * seg_count) / 2,
        }
    }

    /// `get_glyph`
    fn get_glyph(&self, t: &[u8], codepoint: HbCodepoint, glyph: &mut HbCodepoint) -> bool {
        /* CustomRange::cmp: the key against the segment's last and first
         * (`(&last)[distance]`, its startCount) */
        let distance = self.seg_count as usize + 1;
        let found = hb_bsearch_impl(self.seg_count as usize, |mid| {
            let last = u16_at(t, self.end_count + 2 * mid) as HbCodepoint;
            if codepoint > last {
                return 1;
            }
            let first = u16_at(t, self.end_count + 2 * (mid + distance)) as HbCodepoint;
            if codepoint < first {
                return -1;
            }
            0
        });
        let Ok(i) = found else {
            return false;
        };
        let i = i as usize;

        let mut gid: HbCodepoint;
        let range_offset = u16_at(t, self.id_range_offset + 2 * i) as u32;
        let id_delta = u16_at(t, self.id_delta + 2 * i) as u32;
        if range_offset == 0 {
            gid = codepoint.wrapping_add(id_delta);
        } else {
            /* Somebody has been smoking... */
            let start = u16_at(t, self.start_count + 2 * i) as u32;
            let index = (range_offset / 2)
                .wrapping_add(codepoint.wrapping_sub(start))
                .wrapping_add(i as u32)
                .wrapping_sub(self.seg_count);
            if index >= self.glyph_id_array_length {
                return false;
            }
            gid = u16_at(t, self.glyph_id_array + 2 * index as usize) as u32;
            if gid == 0 {
                return false;
            }
            gid = gid.wrapping_add(id_delta);
        }
        gid &= 0xFFFF;
        if gid == 0 {
            return false;
        }
        *glyph = gid;
        true
    }
}

/// `CmapSubtableFormat4::sanitize`
fn format4_sanitize(c: &mut HbSanitizeContext<'_>, p: usize) -> bool {
    if !c.check_struct(p, 14) {
        return false;
    }

    let length = c.u16(p + 2) as u32;
    if !c.check_range(p, length) {
        /* Some broken fonts have too long of a "length" value.
         * If that is the case, just change the value to truncate
         * the subtable at the end of the blob. */
        let new_length = 65535.min(c.end - p) as u16;
        if !c.try_set_u16(p + 2, new_length) {
            return false;
        }
    }

    16 + 4 * (c.u16(p + 6) as u32) <= c.u16(p + 2) as u32
}

/// `CmapSubtableTrimmed<UINT>::get_glyph` (`wide`: format 10's 32-bit
/// fields)
fn trimmed_get_glyph(
    t: &[u8],
    st: usize,
    wide: bool,
    codepoint: HbCodepoint,
    glyph: &mut HbCodepoint,
) -> bool {
    /* Rely on our implicit array bound-checking. */
    let (start_char_code, len, items) = if wide {
        (u32_at(t, st + 12), u32_at(t, st + 16), st + 20)
    } else {
        (u16_at(t, st + 6) as u32, u16_at(t, st + 8) as u32, st + 10)
    };
    let i = codepoint.wrapping_sub(start_char_code);
    let gid = if i < len {
        u16_at(t, items + 2 * i as usize) as HbCodepoint
    } else {
        0
    };
    if gid == 0 {
        return false;
    }
    *glyph = gid;
    true
}

/// `CmapSubtableLongSegmented<T>::get_glyph` (`format12`: format 12's
/// `group_get_glyph`, else format 13's)
fn long_segmented_get_glyph(
    t: &[u8],
    st: usize,
    format12: bool,
    codepoint: HbCodepoint,
    glyph: &mut HbCodepoint,
) -> bool {
    let len = u32_at(t, st + 12);
    let groups = st + 16;
    /* groups.bsearch (codepoint), with CmapSubtableLongGroup::cmp; the Null
     * group is { 1, 0, 0 } */
    let (start, end, glyph_id) = match hb_bsearch_impl(len as usize, |mid| {
        let g = groups + 12 * mid;
        if codepoint < u32_at(t, g) {
            return -1;
        }
        if codepoint > u32_at(t, g + 4) {
            return 1;
        }
        0
    }) {
        Ok(i) => {
            let g = groups + 12 * i as usize;
            (u32_at(t, g), u32_at(t, g + 4), u32_at(t, g + 8))
        }
        Err(_) => (1, 0, 0),
    };
    let gid = if format12 {
        /* CmapSubtableFormat12::group_get_glyph */
        if start <= end {
            glyph_id.wrapping_add(codepoint.wrapping_sub(start))
        } else {
            0
        }
    } else {
        /* CmapSubtableFormat13::group_get_glyph */
        glyph_id
    };
    if gid == 0 {
        return false;
    }
    *glyph = gid;
    true
}

/// `glyph_variant_t`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GlyphVariant {
    NotFound,
    Found,
    UseDefault,
}

/// `CmapSubtableFormat14::get_glyph_variant`
fn format14_get_glyph_variant(
    t: &[u8],
    st: usize,
    codepoint: HbCodepoint,
    variation_selector: HbCodepoint,
    glyph: &mut HbCodepoint,
) -> GlyphVariant {
    /* record.bsearch (variation_selector) */
    let len = u32_at(t, st + 6);
    let records = st + 10;
    let Ok(i) = hb_bsearch_impl(len as usize, |mid| {
        cmp_wide(variation_selector, u24_at(t, records + 11 * mid))
    }) else {
        /* (the Null record: no default and no non-default UVS) */
        return GlyphVariant::NotFound;
    };
    let record = records + 11 * i as usize;

    /* VariationSelectorRecord::get_glyph */
    let default_uvs = u32_at(t, record + 3) as usize;
    if default_uvs != 0 {
        let d = st + default_uvs;
        let n = u32_at(t, d);
        /* bfind, with UnicodeValueRange::cmp */
        if hb_bsearch_impl(n as usize, |mid| {
            let r = d + 4 + 4 * mid;
            let start = u24_at(t, r);
            if codepoint < start {
                return -1;
            }
            if codepoint > start + u8_at(t, r + 3) as u32 {
                return 1;
            }
            0
        })
        .is_ok()
        {
            return GlyphVariant::UseDefault;
        }
    }
    let non_default_uvs = u32_at(t, record + 7) as usize;
    if non_default_uvs != 0 {
        let d = st + non_default_uvs;
        let n = u32_at(t, d);
        /* bsearch, with UVSMapping::cmp */
        if let Ok(j) = hb_bsearch_impl(n as usize, |mid| {
            cmp_wide(codepoint, u24_at(t, d + 4 + 5 * mid))
        }) {
            let glyph_id = u16_at(t, d + 4 + 5 * j as usize + 3) as HbCodepoint;
            if glyph_id != 0 {
                *glyph = glyph_id;
                return GlyphVariant::Found;
            }
        }
    }
    GlyphVariant::NotFound
}

/// `CmapSubtable::get_glyph`
fn subtable_get_glyph(
    t: &[u8],
    st: usize,
    codepoint: HbCodepoint,
    glyph: &mut HbCodepoint,
) -> bool {
    match u16_at(t, st) {
        0 => format0_get_glyph(t, st, codepoint, glyph),
        4 => Format4Accel::init(t, st).get_glyph(t, codepoint, glyph),
        6 => trimmed_get_glyph(t, st, false, codepoint, glyph),
        10 => trimmed_get_glyph(t, st, true, codepoint, glyph),
        12 => long_segmented_get_glyph(t, st, true, codepoint, glyph),
        13 => long_segmented_get_glyph(t, st, false, codepoint, glyph),
        /* 14, and formats 2 and 8, intentionally not implemented */
        _ => false,
    }
}

/// `CmapSubtable::sanitize`
fn subtable_sanitize(c: &mut HbSanitizeContext<'_>, p: usize) -> bool {
    if !c.check_struct(p, 2) {
        return false;
    }
    match c.u16(p) {
        0 => c.check_struct(p, 6 + 256),
        4 => format4_sanitize(c, p),
        6 => c.check_struct(p, 10) && c.sanitize_array16_shallow(p + 8, 2),
        10 => c.check_struct(p, 20) && c.sanitize_array32_shallow(p + 16, 2),
        12 | 13 => c.check_struct(p, 16) && c.sanitize_array32_shallow(p + 12, 12),
        14 => {
            c.check_struct(p, 10)
                && c.sanitize_array32(p + 6, 11, |c, q| {
                    /* VariationSelectorRecord::sanitize (c, base) */
                    c.check_struct(q, 11)
                        && c.sanitize_offset32(q + 3, p, |c, d| c.sanitize_array32_shallow(d, 4))
                        && c.sanitize_offset32(q + 7, p, |c, d| c.sanitize_array32_shallow(d, 5))
                })
        }
        _ => true,
    }
}

/// `cmap::sanitize`
fn cmap_sanitize(c: &mut HbSanitizeContext<'_>) -> bool {
    c.check_struct(0, 4)
        && c.u16(0) == 0
        && c.sanitize_array16(2, 8, |c, q| {
            /* EncodingRecord::sanitize (c, base) */
            c.check_struct(q, 8) && c.sanitize_offset32(q + 4, 0, subtable_sanitize)
        })
}

/// `cmap::find_subtable`: the subtable's position (`None` for a null
/// one)
fn find_subtable(t: &[u8], platform_id: u32, encoding_id: u32) -> Option<usize> {
    let (count, records) = (u16_at(t, 2) as usize, 4usize);
    /* encodingRecord.bsearch (key), with EncodingRecord::cmp */
    let i = hb_bsearch_impl(count, |mid| {
        let r = records + 8 * mid;
        let ret = cmp_small(platform_id, u16_at(t, r) as u32);
        if ret != 0 {
            return ret;
        }
        if encoding_id != 0xFFFF {
            let ret = cmp_small(encoding_id, u16_at(t, r + 2) as u32);
            if ret != 0 {
                return ret;
            }
        }
        0
    })
    .ok()?;
    let subtable = u32_at(t, records + 8 * i as usize + 4) as usize;
    if subtable == 0 {
        return None;
    }
    Some(subtable)
}

/// `cmap::find_best_subtable`: the subtable (`None`: the Null subtable)
/// and whether it is a symbol, Mac and MacRoman one
fn find_best_subtable(t: &[u8]) -> (Option<usize>, bool, bool, bool) {
    /* Symbol subtable.
     * Prefer symbol if available.
     * https://github.com/harfbuzz/harfbuzz/issues/1918 */
    if let Some(st) = find_subtable(t, 3, 0) {
        return (Some(st), true, false, false);
    }

    /* 32-bit subtables. */
    for (p, e) in [(3, 10), (0, 6), (0, 4)] {
        if let Some(st) = find_subtable(t, p, e) {
            return (Some(st), false, false, false);
        }
    }

    /* 16-bit subtables. */
    for (p, e) in [(3, 1), (0, 3), (0, 2), (0, 1), (0, 0)] {
        if let Some(st) = find_subtable(t, p, e) {
            return (Some(st), false, false, false);
        }
    }

    /* MacRoman subtable. */
    if let Some(st) = find_subtable(t, 1, 0) {
        return (Some(st), false, true, true);
    }
    /* Any other Mac subtable; we just map ASCII for these. */
    if let Some(st) = find_subtable(t, 1, 0xFFFF) {
        return (Some(st), false, true, false);
    }

    /* Meh. */
    (None, false, false, false)
}

/// `_hb_symbol_pua_map`
fn _hb_symbol_pua_map(codepoint: u32) -> u32 {
    if codepoint <= 0x00FF {
        /* For symbol-encoded OpenType fonts, we duplicate the
         * U+F000..F0FF range at U+0000..U+00FF.  That's what
         * Windows seems to do, and that's hinted about at:
         * https://docs.microsoft.com/en-us/typography/opentype/spec/recom
         * under "Non-Standard (Symbol) Fonts". */
        return 0xF000 + codepoint;
    }
    0
}

/// `hb_pua_remap_func_t`s
#[derive(Debug, Clone, Copy)]
enum PuaRemap {
    Symbol,
    ArabicSimp,
    ArabicTrad,
}

impl PuaRemap {
    fn remap(self, codepoint: u32) -> u32 {
        match self {
            PuaRemap::Symbol => _hb_symbol_pua_map(codepoint),
            PuaRemap::ArabicSimp => _hb_arabic_pua_simp_map(codepoint) as u32,
            PuaRemap::ArabicTrad => _hb_arabic_pua_trad_map(codepoint) as u32,
        }
    }
}

/// `get_glyph_funcZ`
#[derive(Debug, Clone, Copy)]
enum GetGlyphFunc {
    /// `get_glyph_from<CmapSubtable>` (and `<CmapSubtableFormat12>`)
    From,
    /// `format4_accel.get_glyph_func`
    Format4(Format4Accel),
    /// `get_glyph_from_symbol<CmapSubtable, remap>`
    FromSymbol(PuaRemap),
    /// `get_glyph_from_macroman<CmapSubtable>`
    FromMacroman,
    /// `get_glyph_from_ascii<CmapSubtable>`
    FromAscii,
}

/// `cmap::accelerator_t`
#[derive(Debug)]
pub(crate) struct CmapAccel {
    /// `table` (the sanitized table)
    table: Vec<u8>,
    subtable: Option<usize>,
    subtable_uvs: Option<usize>,
    get_glyph_func: GetGlyphFunc,
}

impl CmapAccel {
    /// `accelerator_t (face)`
    pub(crate) fn new(face: &HbFace) -> CmapAccel {
        let table = hb_sanitize_blob(
            face.reference_table(HB_OT_TAG_CMAP),
            Some(face.get_num_glyphs()),
            cmap_sanitize,
        );
        let (subtable, symbol, mac, macroman) = find_best_subtable(&table);
        let mut subtable_uvs = None;
        {
            if let Some(st) = find_subtable(&table, 0, 5) {
                if u16_at(&table, st) == 14 {
                    subtable_uvs = Some(st);
                }
            }
        }

        let get_glyph_func = if symbol {
            match face.os2_get_font_page() {
                FONT_PAGE_NONE => GetGlyphFunc::FromSymbol(PuaRemap::Symbol),
                FONT_PAGE_SIMP_ARABIC => GetGlyphFunc::FromSymbol(PuaRemap::ArabicSimp),
                FONT_PAGE_TRAD_ARABIC => GetGlyphFunc::FromSymbol(PuaRemap::ArabicTrad),
                _ => GetGlyphFunc::From,
            }
        } else if macroman {
            GetGlyphFunc::FromMacroman
        } else if mac {
            GetGlyphFunc::FromAscii
        } else {
            match subtable.map(|st| u16_at(&table, st)) {
                /* Accelerate format 4 and format 12. */
                Some(4) => GetGlyphFunc::Format4(Format4Accel::init(&table, subtable.unwrap())),
                _ => GetGlyphFunc::From,
            }
        };

        CmapAccel {
            table,
            subtable,
            subtable_uvs,
            get_glyph_func,
        }
    }

    /// The subtable's `get_glyph` (the Null subtable maps nothing).
    fn subtable_get_glyph(&self, codepoint: HbCodepoint, glyph: &mut HbCodepoint) -> bool {
        match self.subtable {
            Some(st) => subtable_get_glyph(&self.table, st, codepoint, glyph),
            None => false,
        }
    }

    /// `_cached_get` (without the cache)
    fn get(&self, unicode: HbCodepoint, glyph: &mut HbCodepoint) -> bool {
        match self.get_glyph_func {
            GetGlyphFunc::From => self.subtable_get_glyph(unicode, glyph),
            GetGlyphFunc::Format4(accel) => accel.get_glyph(&self.table, unicode, glyph),
            GetGlyphFunc::FromSymbol(remap) => {
                if self.subtable_get_glyph(unicode, glyph) {
                    return true;
                }

                let c = remap.remap(unicode);
                if c != 0 {
                    return self.subtable_get_glyph(c, glyph);
                }

                false
            }
            GetGlyphFunc::FromAscii => unicode < 0x80 && self.subtable_get_glyph(unicode, glyph),
            GetGlyphFunc::FromMacroman => {
                if unicode < 0x80 && self.subtable_get_glyph(unicode, glyph) {
                    return true;
                }

                let c = unicode_to_macroman(unicode) as HbCodepoint;
                c != 0 && self.subtable_get_glyph(c, glyph)
            }
        }
    }

    /// `get_nominal_glyph`
    pub(crate) fn get_nominal_glyph(&self, unicode: HbCodepoint, glyph: &mut HbCodepoint) -> bool {
        self.get(unicode, glyph)
    }

    /// `get_nominal_glyphs`
    pub(crate) fn get_nominal_glyphs(
        &self,
        unicodes: &[HbCodepoint],
        glyphs: &mut [HbCodepoint],
    ) -> u32 {
        let mut done = 0;
        while (done as usize) < unicodes.len()
            && self.get(unicodes[done as usize], &mut glyphs[done as usize])
        {
            done += 1;
        }
        done
    }

    /// `get_variation_glyph`
    pub(crate) fn get_variation_glyph(
        &self,
        unicode: HbCodepoint,
        variation_selector: HbCodepoint,
        glyph: &mut HbCodepoint,
    ) -> bool {
        let variant = match self.subtable_uvs {
            Some(st) => {
                format14_get_glyph_variant(&self.table, st, unicode, variation_selector, glyph)
            }
            /* (the Null format 14 subtable) */
            None => GlyphVariant::NotFound,
        };
        match variant {
            GlyphVariant::NotFound => return false,
            GlyphVariant::Found => return true,
            GlyphVariant::UseDefault => {}
        }

        self.get_nominal_glyph(unicode, glyph)
    }
}
