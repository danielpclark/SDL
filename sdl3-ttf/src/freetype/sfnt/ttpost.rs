// Rust translation of src/sfnt/ttpost.c from FreeType (2.13.2, as SDL_ttf's
// external/freetype pins it).
// Copyright (C) 1996-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! PostScript name table processing for TrueType and OpenType fonts
//! (body).
//!
//! The post table is not completely loaded by the core engine.  This
//! file loads the missing PS glyph names and implements an API to access
//! them.
//!
//! `FT_CONFIG_OPTION_POSTSCRIPT_NAMES` is defined: we rely on the `psnames'
//! module to grab the glyph names. The names are returned as byte strings
//! (without their terminating NUL).

use super::super::base::ftmemory::{ft_new_array, ft_qalloc};
use super::super::base::ftstream::*;
use super::super::fttypes::*;
use super::super::psnames::psmodule::ps_get_macintosh_name;
use super::super::tttables::*;
use super::super::tttypes::*;
use super::ttload::tt_face_goto_table;

/// `MAC_NAME`
fn mac_name(x: FtUInt) -> Vec<u8> {
    ps_get_macintosh_name(x).to_vec()
}

/// `load_format_20`
fn load_format_20(
    names: &mut TtPostNamesRec,
    stream: &mut FtStreamRec,
    num_glyphs: FtUShort,
    mut post_len: FtULong,
) -> FtResult<()> {
    let mut num_names: FtUShort = 0;
    let mut name_strings: Vec<Vec<u8>> = Vec::new();

    if num_glyphs as FtULong * 2 > post_len {
        return Err(FT_ERR_INVALID_FILE_FORMAT);
    }

    /* load the indices and note their maximum */
    let mut glyph_indices: Vec<FtUShort> = ft_new_array(num_glyphs as FtLong)?;
    stream.enter_frame(num_glyphs as FtULong * 2)?;

    {
        let q = stream.frame_data();
        let mut p = 0;

        for gi in glyph_indices.iter_mut() {
            let idx = ft_next_ushort(q, &mut p);

            if idx > num_names {
                num_names = idx;
            }

            *gi = idx;
        }
    }

    stream.exit_frame();

    /* compute number of names stored in the table */
    num_names = if num_names > 257 { num_names - 257 } else { 0 };

    /* now load the name strings */
    if num_names != 0 {
        post_len -= num_glyphs as FtULong * 2;

        let mut strings = ft_qalloc((post_len + 1) as FtLong)?;
        if name_strings.try_reserve_exact(num_names as usize).is_err() {
            return Err(FT_ERR_OUT_OF_MEMORY);
        }

        stream.read(&mut strings[..post_len as usize])?;

        /* convert from Pascal- to C-strings and set pointers */
        let mut starts: Vec<usize> = Vec::with_capacity(num_names as usize);
        let mut p: FtULong = 0;
        let mut n: FtUShort = 0;
        while p < post_len && n < num_names {
            let len = strings[p as usize] as FtUInt;

            /* all names in Adobe Glyph List are shorter than 40 characters */

            strings[p as usize] = 0;

            starts.push((p + 1) as usize);
            p += len as FtULong + 1;
            n += 1;
        }
        strings[post_len as usize] = 0;

        /* deal with missing or insufficient string data */
        while n < num_names {
            starts.push(post_len as usize);
            n += 1;
        }

        for s in starts {
            let rest = strings.get(s..).unwrap_or(&[]);
            let end = rest.iter().position(|&c| c == 0).unwrap_or(rest.len());
            name_strings.push(rest[..end].to_vec());
        }
    }

    /* all right, set table fields and exit successfully */
    names.num_glyphs = num_glyphs;
    names.num_names = num_names;
    names.glyph_indices = glyph_indices;
    names.glyph_names = name_strings;

    Ok(())
}

/// `load_format_25`
fn load_format_25(
    names: &mut TtPostNamesRec,
    stream: &mut FtStreamRec,
    num_glyphs: FtUShort,
    post_len: FtULong,
) -> FtResult<()> {
    /* check the number of glyphs, including the theoretical limit */
    if num_glyphs as FtULong > post_len || num_glyphs > 258 + 128 {
        return Err(FT_ERR_INVALID_FILE_FORMAT);
    }

    /* load the indices and check their Mac range */
    let mut glyph_indices: Vec<FtUShort> = ft_new_array(num_glyphs as FtLong)?;
    stream.enter_frame(num_glyphs as FtULong)?;

    {
        let q = stream.frame_data();
        let mut p = 0;

        for (n, gi) in glyph_indices.iter_mut().enumerate() {
            let mut idx = n as FtInt + ft_next_char(q, &mut p) as FtInt;

            if !(0..=257).contains(&idx) {
                idx = 0;
            }

            *gi = idx as FtUShort;
        }
    }

    stream.exit_frame();

    /* OK, set table fields and exit successfully */
    names.num_glyphs = num_glyphs;
    names.glyph_indices = glyph_indices;

    Ok(())
}

/// `load_post_names`
fn load_post_names(face: &mut TtFaceRec, stream: &mut FtStreamRec) -> FtResult<()> {
    let format = face.postscript.FormatType;

    let r = (|| -> FtResult<()> {
        /* seek to the beginning of the PS names table */
        let post_len = tt_face_goto_table(face, TTAG_post as FtULong, stream)?;

        /* UNDOCUMENTED!  The number of glyphs in this table can be smaller */
        /* than the value in the maxp table (cf. cyberbit.ttf).             */
        if post_len < 34 || stream.skip(32).is_err() {
            return Ok(());
        }
        let num_glyphs = match stream.read_ushort() {
            Ok(n) => n,
            Err(_) => return Ok(()),
        };
        if num_glyphs > face.max_profile.numGlyphs || num_glyphs == 0 {
            return Ok(());
        }

        /* now read postscript names data */
        if format == 0x00020000 {
            load_format_20(
                &mut face.postscript_names,
                stream,
                num_glyphs,
                post_len - 34,
            )
        } else if format == 0x00025000 {
            load_format_25(
                &mut face.postscript_names,
                stream,
                num_glyphs,
                post_len - 34,
            )
        } else {
            Ok(())
        }
    })();

    /* Exit: */
    face.postscript_names.loaded = true; /* even if failed */

    r
}

/// `tt_face_free_ps_names`
pub fn tt_face_free_ps_names(face: &mut TtFaceRec) {
    let names = &mut face.postscript_names;

    if names.num_glyphs != 0 {
        names.glyph_indices = Vec::new();
        names.num_glyphs = 0;
    }

    if names.num_names != 0 {
        names.glyph_names = Vec::new();
        names.num_names = 0;
    }

    names.loaded = false;
}

/// `tt_face_get_ps_name`: Get the PostScript glyph name of a glyph.
pub fn tt_face_get_ps_name(
    face: &mut TtFaceRec,
    stream: &mut FtStreamRec,
    idx: FtUInt,
) -> FtResult<Vec<u8>> {
    if idx >= face.max_profile.numGlyphs as FtUInt {
        return Err(FT_ERR_INVALID_GLYPH_INDEX);
    }

    /* `.notdef' by default */
    let mut psname = mac_name(0);

    let format = face.postscript.FormatType;

    if format == 0x00010000 {
        if idx < 258 {
            /* paranoid checking */
            psname = mac_name(idx);
        }
    } else if format == 0x00020000 || format == 0x00025000 {
        'end: {
            if !face.postscript_names.loaded && load_post_names(face, stream).is_err() {
                break 'end;
            }

            let names = &face.postscript_names;

            if idx < names.num_glyphs as FtUInt {
                let name_index = names.glyph_indices[idx as usize];

                if name_index < 258 {
                    psname = mac_name(name_index as FtUInt);
                } else {
                    /* only for version 2.0 */
                    psname = names
                        .glyph_names
                        .get((name_index - 258) as usize)
                        .cloned()
                        .unwrap_or_default();
                }
            }
        }
    }

    /* nothing to do for format == 0x00030000L */

    /* End: */
    /* post format errors ignored */
    Ok(psname)
}
