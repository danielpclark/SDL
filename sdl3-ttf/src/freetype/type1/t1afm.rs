// Rust translation of src/type1/t1afm.c and src/type1/t1afm.h from
// FreeType (2.13.2, as SDL_ttf's external/freetype pins it).
// Copyright (C) 1996-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! AFM support for Type 1 fonts (body).
//!
//! `T1_CONFIG_OPTION_NO_AFM` is undefined. The attached metrics are the
//! frame of the attached stream; the PFM kerning pairs are sorted stably
//! (C's `ft_qsort`; pairs are unique in valid files).

use super::super::base::ftcalc::ft_mul_div;
use super::super::base::ftmemory::ft_new_array;
use super::super::base::ftobjs::*;
use super::super::base::ftstream::{
    ft_peek_short_le, ft_peek_ulong_le, ft_peek_ushort_le, FtStreamRec,
};
use super::super::fttypes::*;
use super::super::psaux::afmparse::*;
use super::super::t1types::*;

/// `T1_Done_Metrics`
pub fn t1_done_metrics(fi: Box<AfmFontInfoRec>) {
    /* (the kerning pairs and track kerns go with it) */
    drop(fi);
}

/// `t1_get_index`: read a glyph name and return the equivalent glyph
/// index
fn t1_get_index(name: &[u8], type1: &T1FontRec) -> FtInt {
    let len = name.len();

    /* PS string/name length must be < 16-bit */
    if len > 0xFFFF {
        return 0;
    }

    for n in 0..type1.num_glyphs.max(0) as usize {
        if let Some(gname) = type1.glyph_names.name(n) {
            if gname.first() == name.first() && gname.len() == len && gname == name {
                return n as FtInt;
            }
        }
    }

    0
}

/// `KERN_INDEX`
#[inline]
fn kern_index(g1: FtUInt, g2: FtUInt) -> FtULong {
    ((g1 as FtULong) << 16) | g2 as FtULong
}

/// `compare_kern_pairs`: compare two kerning pairs
fn compare_kern_pairs(pair1: &AfmKernPairRec, pair2: &AfmKernPairRec) -> std::cmp::Ordering {
    let index1 = kern_index(pair1.index1, pair1.index2);
    let index2 = kern_index(pair2.index1, pair2.index2);

    index1.cmp(&index2)
}

/// `T1_Read_PFM`: parse a PFM file -- for now, only read the kerning
/// pairs (`frame` is the stream's frame: C's `stream->cursor` to
/// `stream->limit`)
fn t1_read_pfm(t1_face: &mut FtFaceRec, frame: &[u8], fi: &mut AfmFontInfoRec) -> FtResult<()> {
    let start: u64 = 0;
    let mut limit: u64 = frame.len() as u64;

    let error = (|| -> FtResult<()> {
        /* Figure out how long the width table is.          */
        /* This info is a little-endian short at offset 99. */
        let mut p: u64 = start + 99;
        if p + 2 > limit {
            return Err(FT_ERR_UNKNOWN_FILE_FORMAT);
        }
        let width_table_length = ft_peek_ushort_le(frame, p as usize) as FtInt;

        p += 18 + width_table_length as u64;
        if p + 0x12 > limit || ft_peek_ushort_le(frame, p as usize) < 0x12 {
            /* extension table is probably optional */
            return Ok(());
        }

        /* Kerning offset is 14 bytes from start of extensions table. */
        p += 14;
        p = start + ft_peek_ulong_le(frame, p as usize) as u64;

        if p == start {
            /* zero offset means no table */
            return Ok(());
        }

        if p + 2 > limit {
            return Err(FT_ERR_UNKNOWN_FILE_FORMAT);
        }

        fi.NumKernPair = ft_peek_ushort_le(frame, p as usize) as FtUInt;
        p += 2;
        if p + 4 * fi.NumKernPair as u64 > limit {
            return Err(FT_ERR_UNKNOWN_FILE_FORMAT);
        }

        /* Actually, kerning pairs are simply optional! */
        if fi.NumKernPair == 0 {
            return Ok(());
        }

        /* allocate the pairs */
        fi.KernPairs = ft_new_array(fi.NumKernPair as FtLong)?;

        /* now, read each kern pair */
        let mut kp = 0usize;
        limit = p + 4 * fi.NumKernPair as u64;

        /* PFM kerning data are stored by encoding rather than glyph index, */
        /* so find the PostScript charmap of this font and install it       */
        /* temporarily.  If we find no PostScript charmap, then just use    */
        /* the default and hope it is the right one.                        */
        let oldcharmap = t1_face.charmap;

        for n in 0..t1_face.num_charmaps.max(0) as usize {
            let charmap = &t1_face.charmaps[n];
            /* check against PostScript pseudo platform */
            if charmap.charmap.platform_id == 7 {
                t1_face.charmap = Some(n);
                break;
            }
        }

        /* Kerning info is stored as:             */
        /*                                        */
        /*   encoding of first glyph (1 byte)     */
        /*   encoding of second glyph (1 byte)    */
        /*   offset (little-endian short)         */
        while p < limit {
            let i = p as usize;
            let pair = &mut fi.KernPairs[kp];
            pair.index1 = ft_get_char_index(t1_face, frame[i] as FtULong);
            pair.index2 = ft_get_char_index(t1_face, frame[i + 1] as FtULong);

            pair.x = ft_peek_short_le(frame, i + 2) as FtInt;
            pair.y = 0;

            kp += 1;
            p += 4;
        }

        t1_face.charmap = oldcharmap;

        /* now, sort the kern pairs according to their glyph indices */
        fi.KernPairs.sort_by(compare_kern_pairs);

        Ok(())
    })();

    /* Exit: */
    if error.is_err() {
        fi.KernPairs = Vec::new();
        fi.NumKernPair = 0;
    }

    error
}

/// `T1_Read_Metrics`: parse a metrics file -- either AFM or PFM
/// depending on what it turns out to be
pub fn t1_read_metrics(t1_face: &mut FtFace, stream: &mut FtStreamRec) -> FtResult<()> {
    let FtFace::T1(face) = t1_face else {
        return Err(FT_ERR_INVALID_FACE_HANDLE);
    };
    let mut error: FtResult<()> = Err(FT_ERR_UNKNOWN_FILE_FORMAT);

    if let Some(afm_data) = face.afm_data.take() {
        t1_done_metrics(afm_data);
    }

    let mut fi = Box::<AfmFontInfoRec>::default();
    stream.enter_frame(stream.size)?;

    {
        let frame = stream.frame_data();
        let t1_font = &face.type1;

        fi.FontBBox = t1_font.font_bbox;
        fi.Ascender = t1_font.font_bbox.yMax;
        fi.Descender = t1_font.font_bbox.yMin;

        /* (the `psaux' module's AFM parser functions are called directly) */
        if let Ok(mut parser) = afm_parser_init(frame, 0, frame.len()) {
            let get_index = |name: &[u8]| t1_get_index(name, t1_font);

            parser.font_info = Some(&mut fi);
            parser.get_index = Some(&get_index);

            error = afm_parser_parse(&mut parser);
            afm_parser_done(&mut parser);
        }

        if error == Err(FT_ERR_UNKNOWN_FILE_FORMAT) {
            let start = frame;

            /* MS Windows allows versions up to 0x3FF without complaining */
            if stream.size > 6
                && start[1] < 4
                && ft_peek_ulong_le(start, 2) as FtULong == stream.size
            {
                error = t1_read_pfm(&mut face.root, frame, &mut fi);
            }
        }
    }

    if error.is_ok() {
        let t1_font = &mut face.type1;
        let root = &mut face.root;

        t1_font.font_bbox = fi.FontBBox;

        root.bbox.xMin = fi.FontBBox.xMin >> 16;
        root.bbox.yMin = fi.FontBBox.yMin >> 16;
        /* no `U' suffix here to 0xFFFF! */
        root.bbox.xMax = (fi.FontBBox.xMax + 0xFFFF) >> 16;
        root.bbox.yMax = (fi.FontBBox.yMax + 0xFFFF) >> 16;

        /* ascender and descender are optional and could both be zero */
        /* check if values are meaningful before overriding defaults  */
        if fi.Ascender > fi.Descender {
            /* no `U' suffix here to 0x8000! */
            root.ascender = ((fi.Ascender + 0x8000) >> 16) as FtShort;
            root.descender = ((fi.Descender + 0x8000) >> 16) as FtShort;
        }

        if fi.NumKernPair != 0 {
            root.face_flags |= FT_FACE_FLAG_KERNING;
            face.afm_data = Some(fi);
        }
    }

    stream.exit_frame();

    /* Exit: */
    /* (an unattached `fi' goes) */

    error
}

/// `T1_Get_Kerning`: find the kerning for a given glyph pair
pub fn t1_get_kerning(fi: &AfmFontInfoRec, glyph1: FtUInt, glyph2: FtUInt, kerning: &mut FtVector) {
    let idx: FtULong = kern_index(glyph1, glyph2);
    let pairs = &fi.KernPairs[..(fi.NumKernPair as usize).min(fi.KernPairs.len())];

    /* simple binary search */
    let mut min: isize = 0;
    let mut max: isize = pairs.len() as isize - 1;

    while min <= max {
        let mid = min + (max - min) / 2;
        let pair = &pairs[mid as usize];
        let midi: FtULong = kern_index(pair.index1, pair.index2);

        if midi == idx {
            kerning.x = pair.x as FtPos;
            kerning.y = pair.y as FtPos;

            return;
        }

        if midi < idx {
            min = mid + 1;
        } else {
            max = mid - 1;
        }
    }

    kerning.x = 0;
    kerning.y = 0;
}

/// `T1_Get_Track_Kerning`
pub fn t1_get_track_kerning(
    face: &FtFace,
    ptsize: FtFixed,
    degree: FtInt,
    kerning: &mut FtFixed,
) -> FtResult<()> {
    let FtFace::T1(face) = face else {
        return Err(FT_ERR_INVALID_ARGUMENT);
    };
    let Some(fi) = face.afm_data.as_deref() else {
        return Err(FT_ERR_INVALID_ARGUMENT);
    };

    for tk in fi.TrackKerns.iter().take(fi.NumTrackKern as usize) {
        if tk.degree != degree {
            continue;
        }

        if ptsize < tk.min_ptsize {
            *kerning = tk.min_kern;
        } else if ptsize > tk.max_ptsize {
            *kerning = tk.max_kern;
        } else {
            *kerning = ft_mul_div(
                ptsize - tk.min_ptsize,
                tk.max_kern - tk.min_kern,
                tk.max_ptsize - tk.min_ptsize,
            ) + tk.min_kern;
        }
    }

    Ok(())
}
