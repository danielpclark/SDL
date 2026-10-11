// Rust translation of src/bdf/bdfdrivr.c and src/bdf/bdfdrivr.h from
// FreeType (2.13.2, as SDL_ttf's external/freetype pins it).
//
// FreeType font driver for bdf files
//
// Copyright (C) 2001-2008, 2011, 2013, 2014 by
// Francesco Zappa Nardelli
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT.  IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
// THE SOFTWARE.
//
// This is an altered (translated) version of the original software.

//! FreeType font driver for bdf fonts.
//!
//! The charmap's data is a copy of the face's encoding table (C points to
//! it). The BDF service (`bdf_get_bdf_property`, `bdf_get_charset_id`)
//! has no service record, as in the `sfnt` module: nothing in SDL_ttf
//! queries BDF properties.

use std::sync::Arc;

use super::super::base::ftcalc::ft_mul_div;
use super::super::base::ftmemory::ft_new_array;
use super::super::base::ftobjs::*;
use super::super::fttypes::*;
use super::super::sfnt::ttbdf::BdfPropertyRec;
use super::super::tttables::*;
use super::bdflib::*;

/* bdfdrivr.h */

/// `BDF_encoding_el`
#[derive(Debug, Clone, Copy, Default)]
pub struct BdfEncodingEl {
    pub enc: FtULong,
    pub glyph: FtUShort,
}

/// `BDF_FaceRec`
#[derive(Debug, Default)]
pub struct BdfFaceRec {
    pub root: FtFaceRec,

    pub charset_encoding: Option<Vec<u8>>,
    pub charset_registry: Option<Vec<u8>>,

    pub bdffont: Option<Box<BdfFont>>,

    pub en_table: Vec<BdfEncodingEl>,

    pub default_glyph: FtUInt,
}

/* bdfdrivr.c */

/// `BDF_CMapRec` (the data of a charmap object)
#[derive(Debug, Clone, Default)]
pub struct BdfCMapRec {
    pub num_encodings: FtULong, /* ftobjs.h: FT_CMap->clazz->size */
    pub encodings: Arc<[BdfEncodingEl]>,
}

/// `bdf_cmap_init`
fn bdf_cmap_init(face: &BdfFaceRec) -> FtCMapData {
    let num_encodings = face.bdffont.as_ref().map(|f| f.glyphs_used).unwrap_or(0);
    let n = (num_encodings as usize).min(face.en_table.len());

    FtCMapData::Bdf(BdfCMapRec {
        num_encodings,
        encodings: Arc::from(&face.en_table[..n]),
    })
}

/// `bdf_cmap_done`
pub fn bdf_cmap_done(cmap: &mut FtCMapRec) {
    cmap.data = FtCMapData::None;
}

fn bdfcmap(cmap: &FtCMapRec) -> Option<&BdfCMapRec> {
    match &cmap.data {
        FtCMapData::Bdf(c) => Some(c),
        _ => None,
    }
}

/// `encodings[mid]` (zero past the table, as the C's zeroed tail)
fn enc_at(encodings: &[BdfEncodingEl], i: FtULong) -> BdfEncodingEl {
    encodings.get(i as usize).copied().unwrap_or_default()
}

/// `bdf_cmap_char_index`
fn bdf_cmap_char_index(bdfcmap_: &FtCMapRec, charcode: FtUInt32) -> FtUInt {
    let Some(cmap) = bdfcmap(bdfcmap_) else {
        return 0;
    };
    let encodings = &cmap.encodings;
    let mut result: FtUShort = 0; /* encodings->glyph */

    let charcode = charcode as FtULong;
    let mut min: FtULong = 0;
    let mut max: FtULong = cmap.num_encodings;
    let mut mid: FtULong = (min + max) >> 1;

    while min < max {
        let code: FtULong = enc_at(encodings, mid).enc;

        if charcode == code {
            /* increase glyph index by 1 --              */
            /* we reserve slot 0 for the undefined glyph */
            result = enc_at(encodings, mid).glyph.wrapping_add(1);
            break;
        }

        if charcode < code {
            max = mid;
        } else {
            min = mid + 1;
        }

        /* reasonable prediction in a continuous block */
        mid = mid.wrapping_add(charcode.wrapping_sub(code));
        if mid >= max || mid < min {
            mid = (min + max) >> 1;
        }
    }

    result as FtUInt
}

/// `bdf_cmap_char_next`
fn bdf_cmap_char_next(bdfcmap_: &mut FtCMapRec, acharcode: &mut FtUInt32) -> FtUInt {
    let Some(cmap) = bdfcmap(bdfcmap_) else {
        return 0;
    };
    let encodings = &cmap.encodings;
    let mut result: FtUShort = 0; /* encodings->glyph */
    let mut charcode: FtULong = (*acharcode as FtULong) + 1;

    let mut min: FtULong = 0;
    let mut max: FtULong = cmap.num_encodings;
    let mut mid: FtULong = (min + max) >> 1;

    'exit: {
        while min < max {
            let code: FtULong = enc_at(encodings, mid).enc;

            if charcode == code {
                /* increase glyph index by 1 --              */
                /* we reserve slot 0 for the undefined glyph */
                result = enc_at(encodings, mid).glyph.wrapping_add(1);
                break 'exit;
            }

            if charcode < code {
                max = mid;
            } else {
                min = mid + 1;
            }

            /* prediction in a continuous block */
            mid = mid.wrapping_add(charcode.wrapping_sub(code));
            if mid >= max || mid < min {
                mid = (min + max) >> 1;
            }
        }

        charcode = 0;
        if min < cmap.num_encodings {
            charcode = enc_at(encodings, min).enc;
            result = enc_at(encodings, min).glyph.wrapping_add(1);
        }
    }

    /* Exit: */
    if charcode > 0xFFFFFFFF {
        *acharcode = 0;
        /* XXX: result should be changed to indicate an overflow error */
    } else {
        *acharcode = charcode as FtUInt32;
    }
    result as FtUInt
}

/// `bdf_cmap_class`
pub static BDF_CMAP_CLASS: FtCMapClassRec = FtCMapClassRec {
    char_index: bdf_cmap_char_index,
    char_next: bdf_cmap_char_next,

    char_var_index: None,
    char_var_default: None,
    variant_list: None,
    charvariant_list: None,
    variantchar_list: None,
};

/// The atom of a property, if it is one with a value (`prop &&
/// prop->format == BDF_ATOM && prop->value.atom`).
fn atom_of<'a>(font: &'a BdfFont, name: &[u8]) -> Option<&'a [u8]> {
    match bdf_get_font_property(font, name) {
        Some(prop) if prop.format == BDF_ATOM => prop.atom.as_deref(),
        _ => None,
    }
}

/// `bdf_interpret_style`
fn bdf_interpret_style(bdf: &mut BdfFaceRec) -> FtResult<()> {
    let face = &mut bdf.root;
    let Some(font) = bdf.bdffont.as_ref() else {
        return Ok(());
    };

    let mut strings: [Option<Vec<u8>>; 4] = [None, None, None, None];
    let mut lengths: [usize; 4] = [0; 4];

    face.style_flags = 0;

    if let Some(atom) = atom_of(font, b"SLANT") {
        let c = atom.first().copied().unwrap_or(0);
        if c == b'O' || c == b'o' || c == b'I' || c == b'i' {
            face.style_flags |= FT_STYLE_FLAG_ITALIC;
            strings[2] = Some(if c == b'O' || c == b'o' {
                b"Oblique".to_vec()
            } else {
                b"Italic".to_vec()
            });
        }
    }

    if let Some(atom) = atom_of(font, b"WEIGHT_NAME") {
        let c = atom.first().copied().unwrap_or(0);
        if c == b'B' || c == b'b' {
            face.style_flags |= FT_STYLE_FLAG_BOLD;
            strings[1] = Some(b"Bold".to_vec());
        }
    }

    if let Some(atom) = atom_of(font, b"SETWIDTH_NAME") {
        let c = atom.first().copied().unwrap_or(0);
        if c != 0 && !(c == b'N' || c == b'n') {
            strings[3] = Some(atom.to_vec());
        }
    }

    if let Some(atom) = atom_of(font, b"ADD_STYLE_NAME") {
        let c = atom.first().copied().unwrap_or(0);
        if c != 0 && !(c == b'N' || c == b'n') {
            strings[0] = Some(atom.to_vec());
        }
    }

    let mut len = 0usize;
    for nn in 0..4 {
        lengths[nn] = 0;
        if let Some(s) = &strings[nn] {
            lengths[nn] = s.iter().position(|&c| c == 0).unwrap_or(s.len());
            len += lengths[nn] + 1;
        }
    }

    if len == 0 {
        strings[0] = Some(b"Regular".to_vec());
        lengths[0] = 7;
    }

    {
        let mut s: Vec<u8> = Vec::new();

        for nn in 0..4 {
            let Some(src) = &strings[nn] else {
                continue;
            };

            let len = lengths[nn];

            /* separate elements with a space */
            if !s.is_empty() {
                s.push(b' ');
            }

            let start = s.len();
            s.extend_from_slice(&src[..len]);

            /* need to convert spaces to dashes for */
            /* add_style_name and setwidth_name     */
            if nn == 0 || nn == 3 {
                for c in s[start..].iter_mut() {
                    if *c == b' ' {
                        *c = b'-';
                    }
                }
            }
        }
        face.style_name = Some(String::from_utf8_lossy(&s).into_owned());
    }

    Ok(())
}

/// `BDF_Face_Done`
fn bdf_face_done(face: &mut FtFace) {
    let FtFace::Bdf(bdfface) = face else {
        return;
    };

    bdf_free_font(bdfface.bdffont.take());

    bdfface.en_table = Vec::new();

    bdfface.charset_encoding = None;
    bdfface.charset_registry = None;
    bdfface.root.family_name = None;
    bdfface.root.style_name = None;

    bdfface.root.available_sizes = Vec::new();
}

/// `BDF_Face_Init`
fn bdf_face_init(face: &mut FtFace, face_index: FtInt, _params: &[FtParameter]) -> FtResult<()> {
    let FtFace::Bdf(bdfface) = face else {
        return Err(FT_ERR_INVALID_FACE_HANDLE);
    };

    let Some(stream) = bdfface.root.stream.as_mut() else {
        return Err(FT_ERR_INVALID_STREAM_HANDLE);
    };

    stream.seek(0)?;

    let options = BdfOptions {
        correct_metrics: 1, /* FZ XXX: options semantics */
        keep_unencoded: 1,
        keep_comments: 0,
        font_spacing: BDF_PROPORTIONAL,
    };

    let font = match bdf_load_font(stream, Some(&options)) {
        Ok(font) => font,
        Err(e) if ft_err_eq(e, FT_ERR_MISSING_STARTFONT_FIELD) => {
            /* Fail: */
            bdf_face_done(face);
            return Err(FT_ERR_UNKNOWN_FILE_FORMAT);
        }
        Err(e) => return Err(e),
    };

    /* we have a bdf font: let's construct the face object */
    bdfface.bdffont = Some(font);

    /* BDF cannot have multiple faces in a single font file.
     * XXX: non-zero face_index is already invalid argument, but
     *      Type1, Type42 driver has a convention to return
     *      an invalid argument error when the font could be
     *      opened by the specified driver.
     */
    if face_index > 0 && (face_index & 0xFFFF) > 0 {
        bdf_face_done(face);
        return Err(FT_ERR_INVALID_ARGUMENT);
    }

    let Some(font) = bdfface.bdffont.as_mut() else {
        return Err(FT_ERR_INVALID_FILE_FORMAT);
    };
    let face = &mut bdfface.root;

    face.num_faces = 1;
    face.face_index = 0;

    face.face_flags |= FT_FACE_FLAG_FIXED_SIZES | FT_FACE_FLAG_HORIZONTAL;

    if let Some(atom) = atom_of(font, b"SPACING") {
        let c = atom.first().copied().unwrap_or(0);
        if c == b'M' || c == b'm' || c == b'C' || c == b'c' {
            face.face_flags |= FT_FACE_FLAG_FIXED_WIDTH;
        }
    }

    /* FZ XXX: TO DO: FT_FACE_FLAGS_VERTICAL   */
    /* FZ XXX: I need a font to implement this */

    match bdf_get_font_property(font, b"FAMILY_NAME").and_then(|p| p.atom.as_ref()) {
        Some(atom) => face.family_name = Some(String::from_utf8_lossy(atom).into_owned()),
        None => face.family_name = None,
    }

    bdf_interpret_style(bdfface)?;

    let Some(font) = bdfface.bdffont.as_mut() else {
        return Err(FT_ERR_INVALID_FILE_FORMAT);
    };
    let face = &mut bdfface.root;

    /* the number of glyphs (with one slot for the undefined glyph */
    /* at position 0 and all unencoded glyphs)                     */
    face.num_glyphs = font.glyphs_size.wrapping_add(1) as FtLong;

    face.num_fixed_sizes = 1;

    {
        let mut bsize = FtBitmapSize::default();
        let mut resolution_x: FtShort = 0;
        let mut resolution_y: FtShort = 0;
        let mut value: i64;

        /* sanity checks */
        if font.font_ascent > 0x7FFF || font.font_ascent < -0x7FFF {
            font.font_ascent = if font.font_ascent < 0 {
                -0x7FFF
            } else {
                0x7FFF
            };
        }
        if font.font_descent > 0x7FFF || font.font_descent < -0x7FFF {
            font.font_descent = if font.font_descent < 0 {
                -0x7FFF
            } else {
                0x7FFF
            };
        }

        bsize.height = (font.font_ascent + font.font_descent) as FtShort;

        if let Some(prop) = bdf_get_font_property(font, b"AVERAGE_WIDTH") {
            if prop.value_l() > 0x7FFF * 10 - 5 || prop.value_l() < -(0x7FFF * 10 - 5) {
                bsize.width = 0x7FFF;
            } else {
                bsize.width = (((prop.value_l() + 5) / 10) as FtShort).wrapping_abs();
            }
        } else {
            /* this is a heuristical value */
            bsize.width = ((bsize.height as i32 * 2 + 1) / 3) as FtShort;
        }

        if let Some(prop) = bdf_get_font_property(font, b"POINT_SIZE") {
            /* convert from 722.7 decipoints to 72 points per inch */
            if prop.value_l() > 0x504C2 /* 0x7FFF * 72270/7200 */ || prop.value_l() < -0x504C2 {
                bsize.size = 0x7FFF;
            } else {
                bsize.size = ft_mul_div(prop.value_l().wrapping_abs(), 64 * 7200, 72270);
            }
        } else if font.point_size != 0 {
            if font.point_size > 0x7FFF {
                bsize.size = 0x7FFF;
            } else {
                bsize.size = (font.point_size as FtPos) << 6;
            }
        } else {
            /* this is a heuristical value */
            bsize.size = bsize.width as FtPos * 64;
        }

        if let Some(prop) = bdf_get_font_property(font, b"PIXEL_SIZE") {
            if prop.value_l() > 0x7FFF || prop.value_l() < -0x7FFF {
                bsize.y_ppem = 0x7FFF << 6;
            } else {
                bsize.y_ppem = ((prop.value_l() as FtShort).wrapping_abs() as FtPos) << 6;
            }
        }

        value = match bdf_get_font_property(font, b"RESOLUTION_X") {
            Some(prop) => prop.value_l(),
            None => font.resolution_x as i64,
        };
        if value != 0 {
            if value > 0x7FFF || value < -0x7FFF {
                resolution_x = 0x7FFF;
            } else {
                resolution_x = (value as FtShort).wrapping_abs();
            }
        }

        value = match bdf_get_font_property(font, b"RESOLUTION_Y") {
            Some(prop) => prop.value_l(),
            None => font.resolution_y as i64,
        };
        if value != 0 {
            if value > 0x7FFF || value < -0x7FFF {
                resolution_y = 0x7FFF;
            } else {
                resolution_y = (value as FtShort).wrapping_abs();
            }
        }

        if bsize.y_ppem == 0 {
            bsize.y_ppem = bsize.size;
            if resolution_y != 0 {
                bsize.y_ppem = ft_mul_div(bsize.y_ppem, resolution_y as FtLong, 72);
            }
        }
        if resolution_x != 0 && resolution_y != 0 {
            bsize.x_ppem = ft_mul_div(bsize.y_ppem, resolution_x as FtLong, resolution_y as FtLong);
        } else {
            bsize.x_ppem = bsize.y_ppem;
        }

        face.available_sizes = vec![bsize];
    }

    /* encoding table */
    {
        let cur = &font.glyphs;

        bdfface.en_table = ft_new_array(font.glyphs_size as FtLong)?;

        bdfface.default_glyph = 0;
        for n in 0..font.glyphs_size as usize {
            let encoding = cur.get(n).map(|g| g.encoding).unwrap_or(0);
            bdfface.en_table[n].enc = encoding;
            bdfface.en_table[n].glyph = n as FtUShort;

            if encoding == font.default_char {
                if (n as u64) < FtUInt::MAX as u64 {
                    bdfface.default_glyph = n as FtUInt;
                }
            }
        }
    }

    /* charmaps */
    {
        let mut unicode_charmap = false;

        let charset_registry = bdf_get_font_property(font, b"CHARSET_REGISTRY");
        let charset_encoding = bdf_get_font_property(font, b"CHARSET_ENCODING");
        if let (Some(charset_registry), Some(charset_encoding)) =
            (charset_registry, charset_encoding)
        {
            if charset_registry.format == BDF_ATOM && charset_encoding.format == BDF_ATOM {
                if let (Some(reg), Some(enc)) = (
                    charset_registry.atom.as_ref(),
                    charset_encoding.atom.as_ref(),
                ) {
                    bdfface.charset_encoding = Some(enc.clone());
                    bdfface.charset_registry = Some(reg.clone());

                    /* Uh, oh, compare first letters manually to avoid dependency */
                    /* on locales.                                                */
                    let s = reg.as_slice();
                    let enc = enc.as_slice();
                    let c = |i: usize| s.get(i).copied().unwrap_or(0);
                    if (c(0) == b'i' || c(0) == b'I')
                        && (c(1) == b's' || c(1) == b'S')
                        && (c(2) == b'o' || c(2) == b'O')
                    {
                        let s = &s[3..];
                        if s == b"10646" || (s == b"8859" && enc == b"1") {
                            unicode_charmap = true;
                        }
                        /* another name for ASCII */
                        else if s == b"646.1991" && enc == b"IRV" {
                            unicode_charmap = true;
                        }
                    }

                    {
                        let mut charmap = FtCharMapRec {
                            encoding: FT_ENCODING_NONE,
                            /* initial platform/encoding should indicate unset status? */
                            platform_id: TT_PLATFORM_APPLE_UNICODE,
                            encoding_id: TT_APPLE_ID_DEFAULT,
                        };

                        if unicode_charmap {
                            charmap.encoding = FT_ENCODING_UNICODE;
                            charmap.platform_id = TT_PLATFORM_MICROSOFT;
                            charmap.encoding_id = TT_MS_ID_UNICODE_CS;
                        }

                        let data = bdf_cmap_init(bdfface);
                        ft_cmap_new(&mut bdfface.root, &BDF_CMAP_CLASS, data, charmap)?;
                    }

                    return Ok(());
                }
            }
        }

        /* otherwise assume Adobe standard encoding */

        {
            let charmap = FtCharMapRec {
                encoding: FT_ENCODING_ADOBE_STANDARD,
                platform_id: TT_PLATFORM_ADOBE,
                encoding_id: TT_ADOBE_ID_STANDARD,
            };

            let data = bdf_cmap_init(bdfface);
            let error = ft_cmap_new(&mut bdfface.root, &BDF_CMAP_CLASS, data, charmap);

            /* Select default charmap */
            if bdfface.root.num_charmaps != 0 {
                bdfface.root.charmap = Some(0);
            }

            error?;
        }
    }

    Ok(())
}

/// `BDF_Size_Select`
fn bdf_size_select(face: &mut FtFace, strike_index: FtULong) -> FtResult<()> {
    let FtFace::Bdf(bdfface) = face else {
        return Err(FT_ERR_INVALID_FACE_HANDLE);
    };

    ft_select_metrics(&mut bdfface.root, strike_index);

    let Some(bdffont) = bdfface.bdffont.as_ref() else {
        return Err(FT_ERR_INVALID_FACE_HANDLE);
    };
    let metrics = &mut bdfface.root.size.metrics;
    metrics.ascender = bdffont.font_ascent * 64;
    metrics.descender = -bdffont.font_descent * 64;
    metrics.max_advance = bdffont.bbx.width as FtPos * 64;

    Ok(())
}

/// `BDF_Size_Request`
fn bdf_size_request(face: &mut FtFace, req: &FtSizeRequestRec) -> FtResult<()> {
    let FtFace::Bdf(bdfface) = &*face else {
        return Err(FT_ERR_INVALID_FACE_HANDLE);
    };
    let bsize = bdfface
        .root
        .available_sizes
        .first()
        .copied()
        .unwrap_or_default();
    let Some(bdffont) = bdfface.bdffont.as_ref() else {
        return Err(FT_ERR_INVALID_FACE_HANDLE);
    };
    let mut error = FT_ERR_INVALID_PIXEL_SIZE;

    let mut height = ft_request_height(req);
    height = (height + 32) >> 6;

    match req.type_ {
        FT_SIZE_REQUEST_TYPE_NOMINAL => {
            if height == ((bsize.y_ppem + 32) >> 6) {
                error = FT_ERR_OK;
            }
        }

        FT_SIZE_REQUEST_TYPE_REAL_DIM => {
            if height == (bdffont.font_ascent + bdffont.font_descent) {
                error = FT_ERR_OK;
            }
        }

        _ => {
            error = FT_ERR_UNIMPLEMENTED_FEATURE;
        }
    }

    if error != 0 {
        Err(error)
    } else {
        bdf_size_select(face, 0)
    }
}

/// `BDF_Glyph_Load`
fn bdf_glyph_load(face: &mut FtFace, glyph_index: FtUInt, _load_flags: FtInt32) -> FtResult<()> {
    let FtFace::Bdf(bdf) = face else {
        return Err(FT_ERR_INVALID_FACE_HANDLE);
    };
    let mut glyph_index = glyph_index;
    let Some(bdffont) = bdf.bdffont.as_ref() else {
        return Err(FT_ERR_INVALID_FACE_HANDLE);
    };
    let bpp = bdffont.bpp;

    if glyph_index >= bdf.root.num_glyphs as FtUInt {
        return Err(FT_ERR_INVALID_ARGUMENT);
    }

    /* index 0 is the undefined glyph */
    if glyph_index == 0 {
        glyph_index = bdf.default_glyph;
    } else {
        glyph_index -= 1;
    }

    /* slot, bitmap => freetype, glyph => bdflib */
    let empty = BdfGlyph::default();
    let glyph = bdffont.glyphs.get(glyph_index as usize).unwrap_or(&empty);

    let slot = &mut bdf.root.glyph;

    slot.bitmap.rows = glyph.bbx.height as u32;
    slot.bitmap.width = glyph.bbx.width as u32;
    slot.bitmap.pitch = glyph.bpr as i32; /* same as FT_Bitmap.pitch */

    /* note: we don't allocate a new array to hold the bitmap; */
    /*       we can simply point to it                         */
    /* (a copy of it) */
    ft_glyphslot_set_bitmap(slot, glyph.bitmap.clone());

    let bitmap = &mut slot.bitmap;
    match bpp {
        1 => bitmap.pixel_mode = FT_PIXEL_MODE_MONO,
        2 => bitmap.pixel_mode = FT_PIXEL_MODE_GRAY2,
        4 => bitmap.pixel_mode = FT_PIXEL_MODE_GRAY4,
        8 => {
            bitmap.pixel_mode = FT_PIXEL_MODE_GRAY;
            bitmap.num_grays = 256;
        }
        _ => {}
    }

    slot.format = FT_GLYPH_FORMAT_BITMAP;
    slot.bitmap_left = glyph.bbx.x_offset as FtInt;
    slot.bitmap_top = glyph.bbx.ascent as FtInt;

    slot.metrics.horiAdvance = (glyph.dwidth as i32 * 64) as FtPos;
    slot.metrics.horiBearingX = (glyph.bbx.x_offset as i32 * 64) as FtPos;
    slot.metrics.horiBearingY = (glyph.bbx.ascent as i32 * 64) as FtPos;
    slot.metrics.width = (slot.bitmap.width * 64) as FtPos;
    slot.metrics.height = (slot.bitmap.rows * 64) as FtPos;

    /*
     * XXX DWIDTH1 and VVECTOR should be parsed and
     * used here, provided such fonts do exist.
     */
    ft_synthesize_vertical_metrics(&mut slot.metrics, (bdffont.bbx.height as i32 * 64) as FtPos);

    Ok(())
}

/*
 *
 * BDF SERVICE
 *
 */

/// `bdf_get_bdf_property`
pub fn bdf_get_bdf_property(face: &FtFace, prop_name: &[u8]) -> FtResult<BdfPropertyRec> {
    let FtFace::Bdf(bdfface) = face else {
        return Err(FT_ERR_INVALID_ARGUMENT);
    };
    let Some(font) = bdfface.bdffont.as_ref() else {
        return Err(FT_ERR_INVALID_ARGUMENT);
    };

    if let Some(prop) = bdf_get_font_property(font, prop_name) {
        match prop.format {
            BDF_ATOM => {
                return Ok(match &prop.atom {
                    Some(a) => BdfPropertyRec::Atom(a.clone()),
                    /* (a NULL atom) */
                    None => BdfPropertyRec::Atom(Vec::new()),
                });
            }

            BDF_INTEGER => {
                return Ok(BdfPropertyRec::Integer(prop.value_l() as FtInt32));
            }

            BDF_CARDINAL => {
                return Ok(BdfPropertyRec::Cardinal(prop.value_ul() as FtUInt32));
            }

            _ => {}
        }
    }

    /* Fail: */
    Err(FT_ERR_INVALID_ARGUMENT)
}

/// `bdf_get_charset_id`: the charset encoding and registry
pub fn bdf_get_charset_id(face: &FtFace) -> FtResult<(Option<Vec<u8>>, Option<Vec<u8>>)> {
    let FtFace::Bdf(bdfface) = face else {
        return Err(FT_ERR_INVALID_ARGUMENT);
    };

    Ok((
        bdfface.charset_encoding.clone(),
        bdfface.charset_registry.clone(),
    ))
}

/// `FT_FONT_FORMAT_BDF`
pub const FT_FONT_FORMAT_BDF: &str = "BDF";

/*
 *
 * SERVICES LIST
 *
 */

/// `bdf_services` (the BDF service is left out; see the module
/// documentation)
static BDF_SERVICES: [(&str, FtService); 1] = [(
    FT_SERVICE_ID_FONT_FORMAT,
    FtService::FontFormat(FT_FONT_FORMAT_BDF),
)];

/// `bdf_driver_requester`
fn bdf_driver_requester(_module: &FtModuleRec, name: &str) -> Option<FtService> {
    BDF_SERVICES
        .iter()
        .find(|(id, _)| *id == name)
        .map(|(_, s)| *s)
}

fn bdf_new_face(root: FtFaceRec) -> FtFace {
    FtFace::Bdf(Box::new(BdfFaceRec {
        root,
        ..Default::default()
    }))
}

/// `bdf_driver_class`
pub static BDF_DRIVER_CLASS: FtDriverClassRec = FtDriverClassRec {
    root: FtModuleClass {
        module_flags: FT_MODULE_FONT_DRIVER | FT_MODULE_DRIVER_NO_OUTLINES,

        module_name: "bdf",
        module_version: 0x10000,
        module_requires: 0x20000,

        module_interface: FtModuleInterface::None, /* module-specific interface */

        module_init: None, /* FT_Module_Constructor  module_init   */
        module_done: None, /* FT_Module_Destructor   module_done   */
        get_interface: Some(bdf_driver_requester), /* FT_Module_Requester    get_interface */
    },

    new_face: bdf_new_face,

    init_face: Some(bdf_face_init), /* FT_Face_InitFunc  init_face */
    done_face: Some(bdf_face_done), /* FT_Face_DoneFunc  done_face */
    init_size: None,                /* FT_Size_InitFunc  init_size */
    done_size: None,                /* FT_Size_DoneFunc  done_size */
    init_slot: None,                /* FT_Slot_InitFunc  init_slot */
    done_slot: None,                /* FT_Slot_DoneFunc  done_slot */

    load_glyph: Some(bdf_glyph_load), /* FT_Slot_LoadFunc  load_glyph */

    get_kerning: None,  /* FT_Face_GetKerningFunc   get_kerning  */
    attach_file: None,  /* FT_Face_AttachFunc       attach_file  */
    get_advances: None, /* FT_Face_GetAdvancesFunc  get_advances */

    request_size: Some(bdf_size_request), /* FT_Size_RequestFunc  request_size */
    select_size: Some(bdf_size_select),   /* FT_Size_SelectFunc   select_size  */
};
