// Rust translation of src/pfr/pfrobjs.c and src/pfr/pfrobjs.h from
// FreeType (2.13.2, as SDL_ttf's external/freetype pins it).
// Copyright (C) 2002-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! FreeType PFR object methods (body).
//!
//! The glyph slot's `PFR_GlyphRec` (C's `PFR_SlotRec.glyph`) is the
//! face's `glyph`, which holds the slot's glyph loader while it loads a
//! glyph.

use super::super::base::ftcalc::{ft_msb, ft_mul_div, ft_mul_fix};
use super::super::base::ftmemory::ft_new_array;
use super::super::base::ftobjs::*;
use super::super::base::ftoutln::ft_outline_get_cbox;
use super::super::base::ftstream::{ft_next_ulong, ft_peek_short, FtStreamRec};
use super::super::fttypes::*;
use super::super::tttables::*;
use super::pfrcmap::*;
use super::pfrgload::*;
use super::pfrload::*;
use super::pfrsbit::pfr_slot_load_bitmap;
use super::pfrtypes::*;

/// `PFR_FaceRec`
#[derive(Debug, Default)]
pub struct PfrFaceRec {
    pub root: FtFaceRec,
    pub header: PfrHeaderRec,
    pub log_font: PfrLogFontRec,
    pub phy_font: PfrPhyFontRec,
    /// the glyph slot's glyph record (`PFR_SlotRec.glyph`)
    pub glyph: PfrGlyphRec,
}

/// A C string's bytes (up to its first null byte) as a string.
fn c_string(s: &[u8]) -> String {
    let len = s.iter().position(|&c| c == 0).unwrap_or(s.len());
    String::from_utf8_lossy(&s[..len]).into_owned()
}

/*************************************************************************/
/*************************************************************************/
/*****                                                               *****/
/*****                     FACE OBJECT METHODS                       *****/
/*****                                                               *****/
/*************************************************************************/
/*************************************************************************/

/// `pfr_face_done`
pub fn pfr_face_done(pfrface: &mut FtFace) {
    let FtFace::Pfr(face) = pfrface else {
        return;
    };

    /* we don't want dangling pointers */
    face.root.family_name = None;
    face.root.style_name = None;

    /* finalize the physical font record */
    pfr_phy_font_done(&mut face.phy_font);

    /* no need to finalize the logical font or the header */
    face.root.available_sizes = Vec::new();
}

/// `pfr_face_init`
pub fn pfr_face_init(
    pfrface: &mut FtFace,
    face_index: FtInt,
    _params: &[FtParameter],
) -> FtResult<()> {
    let FtFace::Pfr(face) = pfrface else {
        return Err(FT_ERR_INVALID_FACE_HANDLE);
    };
    let face = &mut **face;
    let Some(stream) = face.root.stream.as_mut() else {
        return Err(FT_ERR_INVALID_STREAM_HANDLE);
    };

    /* load the header and check it */
    if pfr_header_load(&mut face.header, stream).is_err() {
        return Err(FT_ERR_UNKNOWN_FILE_FORMAT);
    }

    if !pfr_header_check(&face.header) {
        return Err(FT_ERR_UNKNOWN_FILE_FORMAT);
    }

    /* check face index */
    {
        let mut num_faces: FtLong = 0;

        pfr_log_font_count(stream, face.header.log_dir_offset, &mut num_faces)?;

        face.root.num_faces = num_faces;
    }

    if face_index < 0 {
        return Ok(());
    }

    if (face_index & 0xFFFF) as FtLong >= face.root.num_faces {
        return Err(FT_ERR_INVALID_ARGUMENT);
    }

    /* load the face */
    pfr_log_font_load(
        &mut face.log_font,
        stream,
        (face_index & 0xFFFF) as FtUInt,
        face.header.log_dir_offset,
        face.header.phy_font_max_size_high != 0,
    )?;

    /* load the physical font descriptor */
    pfr_phy_font_load(
        &mut face.phy_font,
        stream,
        face.log_font.phys_offset,
        face.log_font.phys_size,
    )?;

    /* set up all root face fields */
    {
        let phy_font = &face.phy_font;
        let root = &mut face.root;

        root.face_index = (face_index & 0xFFFF) as FtLong;
        root.num_glyphs = phy_font.num_chars as FtLong + 1;

        root.face_flags |= FT_FACE_FLAG_SCALABLE;

        /* if gps_offset == 0 for all characters, we  */
        /* assume that the font only contains bitmaps */
        {
            let mut nn = 0;
            while nn < phy_font.num_chars as usize {
                if phy_font.chars[nn].gps_offset != 0 {
                    break;
                }
                nn += 1;
            }

            if nn == phy_font.num_chars as usize {
                if phy_font.num_strikes > 0 {
                    root.face_flags &= !FT_FACE_FLAG_SCALABLE;
                } else {
                    return Err(FT_ERR_INVALID_FILE_FORMAT);
                }
            }
        }

        if phy_font.flags & PFR_PHY_PROPORTIONAL == 0 {
            root.face_flags |= FT_FACE_FLAG_FIXED_WIDTH;
        }

        if phy_font.flags & PFR_PHY_VERTICAL != 0 {
            root.face_flags |= FT_FACE_FLAG_VERTICAL;
        } else {
            root.face_flags |= FT_FACE_FLAG_HORIZONTAL;
        }

        if phy_font.num_strikes > 0 {
            root.face_flags |= FT_FACE_FLAG_FIXED_SIZES;
        }

        if phy_font.num_kern_pairs > 0 {
            root.face_flags |= FT_FACE_FLAG_KERNING;
        }

        /* If no family name was found in the `undocumented' auxiliary
         * data, use the font ID instead.  This sucks but is better than
         * nothing.
         */
        root.family_name = phy_font
            .family_name
            .as_deref()
            .or(phy_font.font_id.as_deref())
            .map(c_string);

        /* note that the style name can be NULL in certain PFR fonts,
         * probably meaning `Regular'
         */
        root.style_name = phy_font.style_name.as_deref().map(c_string);

        root.num_fixed_sizes = 0;
        root.available_sizes = Vec::new();

        root.bbox = phy_font.bbox;
        root.units_per_EM = phy_font.outline_resolution as FtUShort;
        root.ascender = phy_font.bbox.yMax as FtShort;
        root.descender = phy_font.bbox.yMin as FtShort;

        root.height = ((root.units_per_EM as i32 * 12) / 10) as FtShort;
        if (root.height as i32) < root.ascender as i32 - root.descender as i32 {
            root.height = (root.ascender as i32 - root.descender as i32) as FtShort;
        }

        if phy_font.num_strikes > 0 {
            let count = phy_font.num_strikes;

            root.available_sizes = ft_new_array(count as FtLong)?;

            for (size, strike) in root.available_sizes.iter_mut().zip(&phy_font.strikes) {
                size.height = strike.y_ppm as FtShort;
                size.width = strike.x_ppm as FtShort;
                size.size = (strike.y_ppm << 6) as FtPos;
                size.x_ppem = (strike.x_ppm << 6) as FtPos;
                size.y_ppem = (strike.y_ppm << 6) as FtPos;
            }
            root.num_fixed_sizes = count as FtInt;
        }

        /* now compute maximum advance width */
        if (phy_font.flags & PFR_PHY_PROPORTIONAL) == 0 {
            root.max_advance_width = phy_font.standard_advance as FtShort;
        } else {
            let mut max: FtInt = 0;

            for gchar in &phy_font.chars[..phy_font.num_chars as usize] {
                if max < gchar.advance {
                    max = gchar.advance;
                }
            }

            root.max_advance_width = max as FtShort;
        }

        root.max_advance_height = root.height;

        root.underline_position = (-(root.units_per_EM as i32) / 10) as FtShort;
        root.underline_thickness = (root.units_per_EM as i32 / 30) as FtShort;
    }

    /* create charmap */
    {
        let charmap = FtCharMapRec {
            platform_id: TT_PLATFORM_MICROSOFT,
            encoding_id: TT_MS_ID_UNICODE_CS,
            encoding: FT_ENCODING_UNICODE,
        };

        let data = pfr_cmap_init(face)?;
        ft_cmap_new(&mut face.root, &PFR_CMAP_CLASS_REC, data, charmap)?;
    }

    /* check whether we have loaded any kerning pairs */
    if face.phy_font.num_kern_pairs != 0 {
        face.root.face_flags |= FT_FACE_FLAG_KERNING;
    }

    /* Exit: */
    Ok(())
}

/*************************************************************************/
/*************************************************************************/
/*****                                                               *****/
/*****                    SLOT OBJECT METHOD                         *****/
/*****                                                               *****/
/*************************************************************************/
/*************************************************************************/

/// `pfr_slot_init` (`pfr_glyph_init` on the face's glyph record, which
/// starts out zeroed, rewinds the slot's glyph loader)
pub fn pfr_slot_init(pfrslot: &mut FtGlyphSlotRec) -> FtResult<()> {
    if let Some(loader) = pfrslot.internal.loader.as_mut() {
        loader.rewind();
    }
    Ok(())
}

/// `pfr_slot_done` (the face's glyph record, `pfr_glyph_done`, goes with
/// the face)
pub fn pfr_slot_done(_pfrslot: &mut FtGlyphSlotRec) {}

/// `pfr_slot_load`
pub fn pfr_slot_load(pfrface: &mut FtFace, gindex: FtUInt, load_flags: FtInt32) -> FtResult<()> {
    let FtFace::Pfr(face) = pfrface else {
        return Err(FT_ERR_INVALID_ARGUMENT);
    };
    let face = &mut **face;
    let mut gindex = gindex;

    if gindex > 0 {
        gindex -= 1;
    }

    if gindex >= face.phy_font.num_chars {
        return Err(FT_ERR_INVALID_ARGUMENT);
    }

    /* try to load an embedded bitmap */
    if load_flags & (FT_LOAD_NO_SCALE | FT_LOAD_NO_BITMAP) == 0 {
        let error = pfr_slot_load_bitmap(
            face,
            gindex,
            (load_flags & FT_LOAD_BITMAP_METRICS_ONLY) != 0,
        );
        if error.is_ok() {
            return Ok(());
        }
    }

    if load_flags & FT_LOAD_SBITS_ONLY != 0 {
        return Err(FT_ERR_INVALID_ARGUMENT);
    }

    let gchar = face.phy_font.chars[gindex as usize];
    face.root.glyph.format = FT_GLYPH_FORMAT_OUTLINE;
    face.root.glyph.outline.n_points = 0;
    face.root.glyph.outline.n_contours = 0;
    let gps_offset = face.header.gps_section_offset as FtULong;

    /* load the glyph outline (FT_LOAD_NO_RECURSE isn't supported) */
    let Some(stream) = face.root.stream.as_mut() else {
        return Err(FT_ERR_INVALID_STREAM_HANDLE);
    };
    face.glyph.loader = face.root.glyph.internal.loader.take().unwrap_or_default();
    let error = pfr_glyph_load(
        &mut face.glyph,
        stream,
        gps_offset,
        gchar.gps_offset as FtULong,
        gchar.gps_size as FtULong,
    );

    if error.is_ok() {
        let pfrslot = &mut face.root.glyph;
        let size_metrics = face.root.size.metrics;

        let scaling = load_flags & FT_LOAD_NO_SCALE == 0;

        /* copy outline data */
        {
            let base = &face.glyph.loader.base.outline;
            let n = (base.n_points.max(0) as usize).min(base.points.len());
            let nc = (base.n_contours.max(0) as usize).min(base.contours.len());

            pfrslot.outline = FtOutline {
                n_contours: base.n_contours,
                n_points: base.n_points,
                points: base.points[..n].to_vec(),
                tags: base.tags[..n].to_vec(),
                contours: base.contours[..nc].to_vec(),
                flags: base.flags,
            };
        }
        let outline = &mut pfrslot.outline;

        outline.flags &= !FT_OUTLINE_OWNER;
        outline.flags |= FT_OUTLINE_REVERSE_FILL;

        if size_metrics.y_ppem < 24 {
            outline.flags |= FT_OUTLINE_HIGH_PRECISION;
        }

        let metrics = &mut pfrslot.metrics;

        /* compute the advance vector */
        metrics.horiAdvance = 0;
        metrics.vertAdvance = 0;

        let mut advance: FtPos = gchar.advance as FtPos;
        let em_metrics = face.phy_font.metrics_resolution;
        let em_outline = face.phy_font.outline_resolution;

        if em_metrics != em_outline {
            advance = ft_mul_div(advance, em_outline as FtLong, em_metrics as FtLong);
        }

        if face.phy_font.flags & PFR_PHY_VERTICAL != 0 {
            metrics.vertAdvance = advance;
        } else {
            metrics.horiAdvance = advance;
        }

        pfrslot.linearHoriAdvance = metrics.horiAdvance;
        pfrslot.linearVertAdvance = metrics.vertAdvance;

        /* make up vertical metrics(?) */
        metrics.vertBearingX = 0;
        metrics.vertBearingY = 0;

        /* (the `#if 0'ed font matrix: some fonts seem to be broken here) */

        /* scale when needed */
        if scaling {
            let x_scale = size_metrics.x_scale;
            let y_scale = size_metrics.y_scale;

            /* scale outline points */
            for vec in outline.points.iter_mut() {
                vec.x = ft_mul_fix(vec.x, x_scale);
                vec.y = ft_mul_fix(vec.y, y_scale);
            }

            /* scale the advance */
            metrics.horiAdvance = ft_mul_fix(metrics.horiAdvance, x_scale);
            metrics.vertAdvance = ft_mul_fix(metrics.vertAdvance, y_scale);
        }

        /* compute the rest of the metrics */
        let cbox = ft_outline_get_cbox(outline);

        metrics.width = cbox.xMax - cbox.xMin;
        metrics.height = cbox.yMax - cbox.yMin;
        metrics.horiBearingX = cbox.xMin;
        /* FIXME (upstream): this is the bottom (`cbox.yMin'), */
        /* not the top, of the glyph                          */
        metrics.horiBearingY = cbox.yMax - metrics.height;
    }

    face.root.glyph.internal.loader = Some(std::mem::take(&mut face.glyph.loader));

    /* Exit: */
    error
}

/*************************************************************************/
/*************************************************************************/
/*****                                                               *****/
/*****                      KERNING METHOD                           *****/
/*****                                                               *****/
/*************************************************************************/
/*************************************************************************/

/// `pfr_face_get_kerning`
pub fn pfr_face_get_kerning(
    face: &mut PfrFaceRec,
    glyph1: FtUInt,
    glyph2: FtUInt,
    kerning: &mut FtVector,
) -> FtResult<()> {
    let phy_font = &face.phy_font;

    kerning.x = 0;
    kerning.y = 0;

    /* PFR indexing skips .notdef, which becomes UINT_MAX */
    let glyph1 = glyph1.wrapping_sub(1);
    let glyph2 = glyph2.wrapping_sub(1);

    /* check the array bounds, .notdef is automatically out */
    if glyph1 >= phy_font.num_chars || glyph2 >= phy_font.num_chars {
        return Ok(());
    }

    /* convert glyph indices to character codes */
    let code1 = phy_font.chars[glyph1 as usize].char_code;
    let code2 = phy_font.chars[glyph2 as usize].char_code;
    let pair = pfr_kern_index(code1, code2);

    /* now search the list of kerning items */
    let Some(item) = phy_font
        .kern_items
        .iter()
        .find(|item| pair >= item.pair1 && pair <= item.pair2)
    else {
        return Ok(());
    };
    let Some(stream) = face.root.stream.as_mut() else {
        return Err(FT_ERR_INVALID_STREAM_HANDLE);
    };

    /* FoundPair: we found an item, now parse it and find the value if any */
    stream.seek(item.offset as FtULong)?;
    stream.enter_frame(item.pair_count as FtULong * item.pair_size as FtULong)?;

    kerning.x = pfr_kern_lookup(stream, item, pair);

    stream.exit_frame();

    /* Exit: */
    Ok(())
}

/// The search of `pfr_face_get_kerning` in the kerning item `item` of
/// the stream frame: the adjustment of `pair` (0 if not found).
fn pfr_kern_lookup(stream: &FtStreamRec, item: &PfrKernItemRec, pair: FtUInt32) -> FtPos {
    /* FIXME (upstream): when the first probe is below `pair', the search */
    /* goes on over `power' more pairs, one past the item's last (of a    */
    /* font whose pairs are not sorted); here a memory stream's bytes     */
    /* after the frame are those of the font, as C's, and a callback      */
    /* stream's zeros                                                     */
    let bytes: &[u8] = match stream.memory_base() {
        Some(mem) => mem.get(stream.frame_origin()..).unwrap_or(&[]),
        None => stream.frame_data(),
    };

    let count = item.pair_count as FtUInt;
    let size = item.pair_size;
    let power: FtUInt = 1 << ft_msb(count);
    let mut probe = power * size;
    let extra = count - power;
    let mut base = 0usize;
    let twobytes = item.flags & PFR_KERN_2BYTE_CHAR != 0;
    let twobyte_adj = item.flags & PFR_KERN_2BYTE_ADJ != 0;
    let mut p: usize;
    let mut cpair: FtUInt32;

    let next_pair = |p: &mut usize| {
        if twobytes {
            ft_next_ulong(bytes, p)
        } else {
            pfr_next_kpair(bytes, p)
        }
    };

    'found: {
        if extra > 0 {
            p = base + (extra * size) as usize;
            cpair = next_pair(&mut p);

            if cpair == pair {
                break 'found;
            }

            if cpair < pair {
                if twobyte_adj {
                    p += 2;
                } else {
                    p += 1;
                }
                base = p;
            }
        }

        while probe > size {
            probe >>= 1;
            p = base + probe as usize;
            cpair = next_pair(&mut p);

            if cpair == pair {
                break 'found;
            }

            if cpair < pair {
                base += probe as usize;
            }
        }

        p = base;
        cpair = next_pair(&mut p);

        if cpair != pair {
            return 0;
        }
    }

    /* Found: */
    let value: FtInt = if twobyte_adj {
        ft_peek_short(bytes, p) as FtInt
    } else {
        bytes.get(p).copied().unwrap_or(0) as FtInt
    };

    item.base_adj as FtPos + value as FtPos
}
