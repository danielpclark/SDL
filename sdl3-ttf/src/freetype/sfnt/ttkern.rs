// Rust translation of src/sfnt/ttkern.c from FreeType (2.13.2, as SDL_ttf's
// external/freetype pins it).
// Copyright (C) 1996-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! Load the basic TrueType kerning table.  This doesn't handle
//! kerning data within the GPOS table at the moment.
//!
//! Its secondary purpose is to load the kerning data from the
//! `kern' table (TrueType and OpenType).

use super::super::base::ftstream::*;
use super::super::fttypes::*;
use super::super::tttables::*;
use super::super::tttypes::*;
use super::ttload::tt_face_goto_table;

/// `TT_KERN_INDEX`
const fn tt_kern_index(g1: FtUInt, g2: FtUInt) -> FtULong {
    ((g1 as FtULong) << 16) | g2 as FtULong
}

/// `tt_face_load_kern`: Load the first kerning table with format 0 in the
/// font.  Only accepts the first horizontal kerning table.  Developers
/// should use the `ftxkern' extension to access other kerning tables in
/// the font file, if they really want to.
pub fn tt_face_load_kern(face: &mut TtFaceRec, stream: &mut FtStreamRec) -> FtResult<()> {
    let mut avail: FtUInt32 = 0;
    let mut ordered: FtUInt32 = 0;

    /* the kern table is optional; exit silently if it is missing */
    let table_size = tt_face_goto_table(face, TTAG_kern as FtULong, stream)?;

    if table_size < 4 {
        /* the case of a malformed table */
        return Err(FT_ERR_TABLE_MISSING);
    }

    face.kern_table = stream.extract_frame(table_size)?;

    face.kern_table_size = table_size;

    let t = &face.kern_table;
    let mut p: usize = 0;
    let p_limit = table_size as usize;

    p += 2; /* skip version */
    let mut num_tables = ft_next_ushort(t, &mut p) as FtUInt;

    if num_tables > 32 {
        /* we only support up to 32 sub-tables */
        num_tables = 32;
    }

    let mut nn: FtUInt = 0;
    while nn < num_tables {
        let mask: FtUInt32 = 1 << nn;

        if p + 6 > p_limit {
            break;
        }

        let mut p_next = p;

        p += 2; /* skip version */
        let length = ft_next_ushort(t, &mut p) as usize;
        let coverage = ft_next_ushort(t, &mut p) as FtUInt;

        if length <= 6 + 8 {
            break;
        }

        p_next += length;

        if p_next > p_limit {
            /* handle broken table */
            p_next = p_limit;
        }

        let format = coverage >> 8;

        'next_table: {
            /* we currently only support format 0 kerning tables */
            if format != 0 {
                break 'next_table;
            }

            /* only use horizontal kerning tables */
            if (coverage & 3) != 0x0001 || p + 8 > p_next {
                break 'next_table;
            }

            let mut num_pairs = ft_next_ushort(t, &mut p) as FtUInt;
            p += 6;

            if ((p_next - p) as i64) < 6 * num_pairs as i64 {
                /* handle broken count */
                num_pairs = (p_next.saturating_sub(p) / 6) as FtUInt;
            }

            avail |= mask;

            /*
             * Now check whether the pairs in this table are ordered.
             * We then can use binary search.
             */
            if num_pairs > 0 {
                let mut old_pair = ft_next_ulong(t, &mut p);
                p += 2;

                let mut count = num_pairs - 1;
                while count > 0 {
                    let cur_pair = ft_next_ulong(t, &mut p);
                    if cur_pair < old_pair {
                        break;
                    }

                    p += 2;
                    old_pair = cur_pair;
                    count -= 1;
                }

                if count == 0 {
                    ordered |= mask;
                }
            }
        }

        /* NextTable: */
        p = p_next;
        nn += 1;
    }

    face.num_kern_tables = nn;
    face.kern_avail_bits = avail;
    face.kern_order_bits = ordered;

    Ok(())
}

/// `tt_face_done_kern`
pub fn tt_face_done_kern(face: &mut TtFaceRec) {
    face.kern_table = Vec::new();
    face.kern_table_size = 0;
    face.num_kern_tables = 0;
    face.kern_avail_bits = 0;
    face.kern_order_bits = 0;
}

/// `tt_face_get_kerning`: Return the horizontal kerning value between two
/// glyphs.
pub fn tt_face_get_kerning(face: &TtFaceRec, left_glyph: FtUInt, right_glyph: FtUInt) -> FtInt {
    let mut result: FtInt = 0;

    if face.kern_table.is_empty() {
        return result;
    }

    let t = &face.kern_table;
    let mut p: usize = 0;
    let p_limit = face.kern_table_size as usize;

    p += 4;
    let mut mask: FtUInt = 0x0001;

    let mut count = face.num_kern_tables;
    while count > 0 && p + 6 <= p_limit {
        let base = p;
        let _version = ft_next_ushort(t, &mut p);
        let length = ft_next_ushort(t, &mut p) as usize;
        let coverage = ft_next_ushort(t, &mut p) as FtUInt;
        let mut value: FtInt = 0;

        let mut next = base + length;

        if next > p_limit {
            /* handle broken table */
            next = p_limit;
        }

        'next_table: {
            if (face.kern_avail_bits & mask) == 0 {
                break 'next_table;
            }

            let mut num_pairs = ft_next_ushort(t, &mut p) as FtUInt;
            p += 6;

            if ((next as i64) - (p as i64)) < 6 * num_pairs as i64 {
                /* handle broken count  */
                num_pairs = (next.saturating_sub(p) / 6) as FtUInt;
            }

            let found = match coverage >> 8 {
                0 => {
                    let key0 = tt_kern_index(left_glyph, right_glyph);
                    let mut found = false;

                    if face.kern_order_bits & mask != 0 {
                        /* binary search */
                        let mut min: FtUInt = 0;
                        let mut max: FtUInt = num_pairs;

                        while min < max {
                            let mid = (min + max) >> 1;
                            let mut q = p + 6 * mid as usize;

                            let key = ft_next_ulong(t, &mut q) as FtULong;

                            if key == key0 {
                                value = ft_peek_short(t, q) as FtInt;
                                found = true;
                                break;
                            }
                            if key < key0 {
                                min = mid + 1;
                            } else {
                                max = mid;
                            }
                        }
                    } else {
                        /* linear search */
                        for _ in 0..num_pairs {
                            let key = ft_next_ulong(t, &mut p) as FtULong;

                            if key == key0 {
                                value = ft_peek_short(t, p) as FtInt;
                                found = true;
                                break;
                            }
                            p += 2;
                        }
                    }
                    found
                }

                /*
                 * We don't support format 2 because we haven't seen a single font
                 * using it in real life...
                 */
                _ => false,
            };

            if !found {
                break 'next_table;
            }

            /* Found: */
            if coverage & 8 != 0 {
                /* override or add */
                result = value;
            } else {
                result = result.wrapping_add(value);
            }
        }

        /* NextTable: */
        p = next;
        count -= 1;
        mask <<= 1;
    }

    result
}
