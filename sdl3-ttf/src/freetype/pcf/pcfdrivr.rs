// Rust translation of src/pcf/pcfdrivr.c and src/pcf/pcfdrivr.h from
// FreeType (2.13.2, as SDL_ttf's external/freetype pins it).
//
// FreeType font driver for pcf files
//
// Copyright (C) 2000-2004, 2006-2011, 2013, 2014 by
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

//! FreeType font driver for pcf fonts.
//!
//! As SDL_ttf's bundled build configures it, `FT_CONFIG_OPTION_USE_ZLIB`
//! and `FT_CONFIG_OPTION_USE_LZW` are defined and
//! `FT_CONFIG_OPTION_USE_BZIP2` and `PCF_CONFIG_OPTION_LONG_FAMILY_NAMES`
//! are not: the driver reads gzip- and LZW-compressed fonts, not bzip2
//! ones. The charmap's data is a copy of the face's encodings. The BDF
//! service (`pcf_get_bdf_property`, `pcf_get_charset_id`) has no service
//! record, as in the `sfnt` module: nothing in SDL_ttf queries BDF
//! properties.

use std::sync::{Arc, Mutex};

use super::super::base::ftobjs::*;
use super::super::base::ftstream::FtSharedStream;
use super::super::fttypes::*;
use super::super::gzip::ftgzip::ft_stream_open_gzip;
use super::super::lzw::ftlzw::ft_stream_open_lzw;
use super::super::sfnt::ttbdf::BdfPropertyRec;
use super::super::tttables::*;
use super::pcf::*;
use super::pcfread::*;
use super::pcfutil::*;

/*
 * This file uses X11 terminology for PCF data; an `encoding' in X11 speak
 * is the same as a `character code' in FreeType speak.
 */

/// `PCF_CMapRec` (the data of a charmap object: the face's encodings)
#[derive(Debug, Clone, Default)]
pub struct PcfCMapRec {
    pub enc: PcfEncRec,
}

/// `pcf_cmap_init`
fn pcf_cmap_init(face: &PcfFaceRec) -> FtCMapData {
    FtCMapData::Pcf(PcfCMapRec {
        enc: face.enc.clone(),
    })
}

/// `pcf_cmap_done`
pub fn pcf_cmap_done(cmap: &mut FtCMapRec) {
    cmap.data = FtCMapData::None;
}

fn pcfcmap(cmap: &FtCMapRec) -> Option<&PcfEncRec> {
    match &cmap.data {
        FtCMapData::Pcf(c) => Some(&c.enc),
        _ => None,
    }
}

/// `pcf_cmap_char_index`
fn pcf_cmap_char_index(cmap: &FtCMapRec, charcode: FtUInt32) -> FtUInt {
    let Some(enc) = pcfcmap(cmap) else {
        return 0;
    };

    let i: FtUInt32 = (charcode >> 8).wrapping_sub(enc.firstRow as FtUInt32);
    let j: FtUInt32 = (charcode & 0xFF).wrapping_sub(enc.firstCol as FtUInt32);
    let h: FtUInt32 = (enc.lastRow as FtUInt32)
        .wrapping_sub(enc.firstRow as FtUInt32)
        .wrapping_add(1);
    let w: FtUInt32 = (enc.lastCol as FtUInt32)
        .wrapping_sub(enc.firstCol as FtUInt32)
        .wrapping_add(1);

    /* wrapped around "negative" values are also rejected */
    if i >= h || j >= w {
        return 0;
    }

    enc.offset.get((i * w + j) as usize).copied().unwrap_or(0) as FtUInt
}

/// `pcf_cmap_char_next`
fn pcf_cmap_char_next(cmap: &mut FtCMapRec, acharcode: &mut FtUInt32) -> FtUInt {
    let Some(enc) = pcfcmap(cmap) else {
        return 0;
    };
    let charcode: FtUInt32 = acharcode.wrapping_add(1);

    let mut i: FtUInt32 = (charcode >> 8).wrapping_sub(enc.firstRow as FtUInt32);
    let mut j: FtUInt32 = (charcode & 0xFF).wrapping_sub(enc.firstCol as FtUInt32);
    let h: FtUInt32 = (enc.lastRow as FtUInt32)
        .wrapping_sub(enc.firstRow as FtUInt32)
        .wrapping_add(1);
    let w: FtUInt32 = (enc.lastCol as FtUInt32)
        .wrapping_sub(enc.firstCol as FtUInt32)
        .wrapping_add(1);

    let mut result: FtUInt = 0;

    /* adjust wrapped around "negative" values */
    if (i as FtInt32) < 0 {
        i = 0;
    }
    if (j as FtInt32) < 0 {
        j = 0;
    }

    'exit: {
        while i < h {
            while j < w {
                result = enc.offset.get((i * w + j) as usize).copied().unwrap_or(0) as FtUInt;
                if result != 0xFFFF {
                    break 'exit;
                }
                j += 1;
            }
            i += 1;
            j = 0;
        }
    }

    /* Exit: */
    *acharcode =
        (i.wrapping_add(enc.firstRow as FtUInt32) << 8) | j.wrapping_add(enc.firstCol as FtUInt32);

    result
}

/// `pcf_cmap_class`
pub static PCF_CMAP_CLASS: FtCMapClassRec = FtCMapClassRec {
    char_index: pcf_cmap_char_index,
    char_next: pcf_cmap_char_next,

    char_var_index: None,
    char_var_default: None,
    variant_list: None,
    charvariant_list: None,
    variantchar_list: None,
};

/// `PCF_Face_Done`
fn pcf_face_done_rec(pcfface: &mut PcfFaceRec) {
    pcfface.metrics = Vec::new();
    pcfface.enc.offset = Arc::from(Vec::new());

    /* free properties */
    pcfface.properties = Vec::new();

    pcfface.toc.tables = Vec::new();
    pcfface.root.family_name = None;
    pcfface.root.style_name = None;
    pcfface.root.available_sizes = Vec::new();
    pcfface.charset_encoding = None;
    pcfface.charset_registry = None;

    /* close compressed stream if any */
    if let Some(source) = pcfface.comp_source.take() {
        /* (the compressed stream's callback holds the source too) */
        pcfface.root.stream = None;
        pcfface.root.stream = match Arc::try_unwrap(source) {
            Ok(m) => Some(m.into_inner().unwrap_or_else(|e| e.into_inner())),
            Err(_) => None,
        };
    }
}

fn pcf_face_done(face: &mut FtFace) {
    if let FtFace::Pcf(pcfface) = face {
        pcf_face_done_rec(pcfface);
    }
}

/// `PCF_Face_Init`
fn pcf_face_init(face: &mut FtFace, face_index: FtInt, _params: &[FtParameter]) -> FtResult<()> {
    let FtFace::Pcf(pcfface) = face else {
        return Err(FT_ERR_INVALID_FACE_HANDLE);
    };

    let r = pcf_face_init_rec(pcfface, face_index);
    match r {
        Ok(()) => Ok(()),
        Err(PcfInit::Exit(e)) => Err(e),
        Err(PcfInit::Fail) => {
            /* Fail: */
            pcf_face_done_rec(pcfface);
            Err(FT_ERR_UNKNOWN_FILE_FORMAT) /* error */
        }
    }
}

/// How `PCF_Face_Init` ends when it fails.
enum PcfInit {
    Exit(FtError),
    Fail,
}

/// Runs `pcf_load_font` on the face's stream.
fn load_font(face: &mut PcfFaceRec, face_index: FtLong) -> FtResult<()> {
    let Some(mut stream) = face.root.stream.take() else {
        return Err(FT_ERR_INVALID_STREAM_HANDLE);
    };
    let r = pcf_load_font(&mut stream, face, face_index);
    face.root.stream = Some(stream);
    r
}

fn pcf_face_init_rec(pcfface: &mut PcfFaceRec, face_index: FtInt) -> Result<(), PcfInit> {
    let error = load_font(pcfface, face_index as FtLong);
    if error.is_err() {
        pcf_face_done_rec(pcfface);

        /* FT_CONFIG_OPTION_USE_ZLIB || FT_CONFIG_OPTION_USE_LZW */
        let Some(stream) = pcfface.root.stream.take() else {
            return Err(PcfInit::Fail);
        };
        let source: FtSharedStream = Arc::new(Mutex::new(stream));

        /* FT_CONFIG_OPTION_USE_ZLIB */
        let mut comp_stream = {
            /* this didn't work, try gzip support! */
            let error2 = ft_stream_open_gzip(&source);
            if matches!(error2, Err(e) if ft_err_eq(e, FT_ERR_UNIMPLEMENTED_FEATURE)) {
                pcfface.root.stream = unshare(source);
                return Err(PcfInit::Fail);
            }

            error2
        };

        /* FT_CONFIG_OPTION_USE_LZW */
        if comp_stream.is_err() {
            /* this didn't work, try LZW support! */
            let error3 = ft_stream_open_lzw(&source);
            if matches!(error3, Err(e) if ft_err_eq(e, FT_ERR_UNIMPLEMENTED_FEATURE)) {
                pcfface.root.stream = unshare(source);
                return Err(PcfInit::Fail);
            }

            comp_stream = error3;
        }

        /* (!FT_CONFIG_OPTION_USE_BZIP2) */

        let comp_stream = match comp_stream {
            Ok(s) => s,
            Err(_) => {
                pcfface.root.stream = unshare(source);
                return Err(PcfInit::Fail);
            }
        };

        pcfface.comp_source = Some(source);
        pcfface.root.stream = Some(comp_stream);

        if load_font(pcfface, face_index as FtLong).is_err() {
            return Err(PcfInit::Fail);
        }
    }

    /* PCF cannot have multiple faces in a single font file.
     * XXX: A non-zero face_index is already an invalid argument, but
     *      Type1, Type42 drivers have a convention to return
     *      an invalid argument error when the font could be
     *      opened by the specified driver.
     */
    if face_index < 0 {
        return Ok(());
    } else if face_index > 0 && (face_index & 0xFFFF) > 0 {
        pcf_face_done_rec(pcfface);
        return Err(PcfInit::Exit(FT_ERR_INVALID_ARGUMENT));
    }

    /* set up charmap */
    {
        let mut unicode_charmap = false;

        if let (Some(s), Some(enc)) = (
            pcfface.charset_registry.as_deref(),
            pcfface.charset_encoding.as_deref(),
        ) {
            let c = |i: usize| s.get(i).copied().unwrap_or(0);

            /* Uh, oh, compare first letters manually to avoid dependency
            on locales. */
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

            let data = pcf_cmap_init(pcfface);
            if let Err(e) = ft_cmap_new(&mut pcfface.root, &PCF_CMAP_CLASS, data, charmap) {
                return Err(PcfInit::Exit(e));
            }
        }
    }

    /* Exit: */
    Ok(())
}

/// The shared source stream, back as the face's own.
fn unshare(source: FtSharedStream) -> Option<super::super::base::ftstream::FtStream> {
    match Arc::try_unwrap(source) {
        Ok(m) => Some(m.into_inner().unwrap_or_else(|e| e.into_inner())),
        Err(_) => None,
    }
}

/// `PCF_Size_Select`
fn pcf_size_select(face: &mut FtFace, strike_index: FtULong) -> FtResult<()> {
    let FtFace::Pcf(pcfface) = face else {
        return Err(FT_ERR_INVALID_FACE_HANDLE);
    };
    let accel = pcfface.accel;

    ft_select_metrics(&mut pcfface.root, strike_index);

    let metrics = &mut pcfface.root.size.metrics;
    metrics.ascender = accel.fontAscent * 64;
    metrics.descender = -accel.fontDescent * 64;
    metrics.max_advance = accel.maxbounds.characterWidth as FtPos * 64;

    Ok(())
}

/// `PCF_Size_Request`
fn pcf_size_request(face: &mut FtFace, req: &FtSizeRequestRec) -> FtResult<()> {
    let FtFace::Pcf(pcfface) = &*face else {
        return Err(FT_ERR_INVALID_FACE_HANDLE);
    };
    let bsize = pcfface
        .root
        .available_sizes
        .first()
        .copied()
        .unwrap_or_default();
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
            if height == (pcfface.accel.fontAscent + pcfface.accel.fontDescent) {
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
        pcf_size_select(face, 0)
    }
}

/// `PCF_Glyph_Load`
fn pcf_glyph_load(face: &mut FtFace, glyph_index: FtUInt, load_flags: FtInt32) -> FtResult<()> {
    let FtFace::Pcf(pcfface) = face else {
        return Err(FT_ERR_INVALID_FACE_HANDLE);
    };

    if glyph_index >= pcfface.root.num_glyphs as FtUInt {
        return Err(FT_ERR_INVALID_ARGUMENT);
    }

    let metric = pcfface
        .metrics
        .get(glyph_index as usize)
        .copied()
        .unwrap_or_default();
    let bitmaps_format = pcfface.bitmapsFormat;
    let accel = pcfface.accel;

    let slot = &mut pcfface.root.glyph;
    let bitmap = &mut slot.bitmap;

    bitmap.rows = (metric.ascent as i32 + metric.descent as i32) as u32;
    bitmap.width = (metric.rightSideBearing as i32 - metric.leftSideBearing as i32) as u32;
    bitmap.num_grays = 1;
    bitmap.pixel_mode = FT_PIXEL_MODE_MONO;

    match pcf_glyph_pad(bitmaps_format) {
        1 => bitmap.pitch = ((bitmap.width + 7) >> 3) as i32,

        2 => bitmap.pitch = (((bitmap.width + 15) >> 4) << 1) as i32,

        4 => bitmap.pitch = (((bitmap.width + 31) >> 5) << 2) as i32,

        8 => bitmap.pitch = (((bitmap.width + 63) >> 6) << 3) as i32,

        _ => return Err(FT_ERR_INVALID_FILE_FORMAT),
    }

    slot.format = FT_GLYPH_FORMAT_BITMAP;
    slot.bitmap_left = metric.leftSideBearing as FtInt;
    slot.bitmap_top = metric.ascent as FtInt;

    slot.metrics.horiAdvance = (metric.characterWidth as i32 * 64) as FtPos;
    slot.metrics.horiBearingX = (metric.leftSideBearing as i32 * 64) as FtPos;
    slot.metrics.horiBearingY = (metric.ascent as i32 * 64) as FtPos;
    slot.metrics.width =
        ((metric.rightSideBearing as i32 - metric.leftSideBearing as i32) * 64) as FtPos;
    slot.metrics.height = slot.bitmap.rows.wrapping_mul(64) as FtPos;

    ft_synthesize_vertical_metrics(
        &mut slot.metrics,
        ((accel.fontAscent + accel.fontDescent) * 64) as FtPos,
    );

    if load_flags & FT_LOAD_BITMAP_METRICS_ONLY != 0 {
        return Ok(());
    }

    /* XXX: to do: are there cases that need repadding the bitmap? */
    let bytes: FtULong = slot.bitmap.pitch as u32 as FtULong * slot.bitmap.rows as FtULong;

    ft_glyphslot_alloc_bitmap(slot, bytes)?;

    let Some(stream) = pcfface.root.stream.as_mut() else {
        return Err(FT_ERR_INVALID_STREAM_HANDLE);
    };
    let slot = &mut pcfface.root.glyph;
    stream.seek(metric.bits)?;
    stream.read(&mut slot.bitmap.buffer[..bytes as usize])?;

    let buffer = &mut slot.bitmap.buffer[..bytes as usize];
    if pcf_bit_order(bitmaps_format) != MSBFirst {
        bit_order_invert(buffer);
    }

    if pcf_byte_order(bitmaps_format) != pcf_bit_order(bitmaps_format) {
        match pcf_scan_unit(bitmaps_format) {
            1 => {}

            2 => two_byte_swap(buffer),

            4 => four_byte_swap(buffer),

            _ => {}
        }
    }

    Ok(())
}

/*
 *
 * BDF SERVICE
 *
 */

/// `pcf_get_bdf_property`
pub fn pcf_get_bdf_property(face: &FtFace, prop_name: &[u8]) -> FtResult<BdfPropertyRec> {
    let FtFace::Pcf(pcfface) = face else {
        return Err(FT_ERR_INVALID_ARGUMENT);
    };

    if let Some(prop) = pcf_find_property(pcfface, prop_name) {
        if prop.isString != 0 {
            return Ok(BdfPropertyRec::Atom(prop.atom.clone().unwrap_or_default()));
        } else {
            /*
             * The PCF driver loads all properties as signed integers.
             * This really doesn't seem to be a problem, because this is
             * sufficient for any meaningful values.
             */
            return Ok(BdfPropertyRec::Integer(prop.l as FtInt32));
        }
    }

    Err(FT_ERR_INVALID_ARGUMENT)
}

/// `pcf_get_charset_id`: the charset encoding and registry
pub fn pcf_get_charset_id(face: &FtFace) -> FtResult<(Option<Vec<u8>>, Option<Vec<u8>>)> {
    let FtFace::Pcf(pcfface) = face else {
        return Err(FT_ERR_INVALID_ARGUMENT);
    };

    Ok((
        pcfface.charset_encoding.clone(),
        pcfface.charset_registry.clone(),
    ))
}

/*
 * PROPERTY SERVICE
 *
 */

/// `pcf_property_set` (`!PCF_CONFIG_OPTION_LONG_FAMILY_NAMES`)
fn pcf_property_set(
    _module: &FtModuleRec, /* PCF_Driver */
    _property_name: &str,
    _value: &FtPropertyValue,
    _value_is_string: bool,
) -> FtResult<()> {
    Err(FT_ERR_MISSING_PROPERTY)
}

/// `pcf_property_get` (`!PCF_CONFIG_OPTION_LONG_FAMILY_NAMES`)
fn pcf_property_get(
    _module: &FtModuleRec, /* PCF_Driver */
    _property_name: &str,
) -> FtResult<FtPropertyValue> {
    Err(FT_ERR_MISSING_PROPERTY)
}

/// `pcf_service_properties`
static PCF_SERVICE_PROPERTIES: FtServicePropertiesRec = FtServicePropertiesRec {
    set_property: pcf_property_set, /* set_property */
    get_property: pcf_property_get, /* get_property */
};

/// `FT_FONT_FORMAT_PCF`
pub const FT_FONT_FORMAT_PCF: &str = "PCF";

/*
 *
 * SERVICE LIST
 *
 */

/// `pcf_services` (the BDF service is left out; see the module
/// documentation)
static PCF_SERVICES: [(&str, FtService); 2] = [
    (
        FT_SERVICE_ID_FONT_FORMAT,
        FtService::FontFormat(FT_FONT_FORMAT_PCF),
    ),
    (
        FT_SERVICE_ID_PROPERTIES,
        FtService::Properties(&PCF_SERVICE_PROPERTIES),
    ),
];

/// `pcf_driver_requester`
fn pcf_driver_requester(_module: &FtModuleRec, name: &str) -> Option<FtService> {
    PCF_SERVICES
        .iter()
        .find(|(id, _)| *id == name)
        .map(|(_, s)| *s)
}

/// `pcf_driver_init` (`!PCF_CONFIG_OPTION_LONG_FAMILY_NAMES`)
fn pcf_driver_init(_library: &mut FtLibraryRec, _module: usize) -> FtResult<()> {
    Ok(())
}

/// `pcf_driver_done`
fn pcf_driver_done(_module: &FtModuleRec) {}

fn pcf_new_face(root: FtFaceRec) -> FtFace {
    FtFace::Pcf(Box::new(PcfFaceRec {
        root,
        ..Default::default()
    }))
}

/// `pcf_driver_class`
pub static PCF_DRIVER_CLASS: FtDriverClassRec = FtDriverClassRec {
    root: FtModuleClass {
        module_flags: FT_MODULE_FONT_DRIVER | FT_MODULE_DRIVER_NO_OUTLINES,

        module_name: "pcf",
        module_version: 0x10000,
        module_requires: 0x20000,

        module_interface: FtModuleInterface::None, /* module-specific interface */

        module_init: Some(pcf_driver_init), /* FT_Module_Constructor  module_init   */
        module_done: Some(pcf_driver_done), /* FT_Module_Destructor   module_done   */
        get_interface: Some(pcf_driver_requester), /* FT_Module_Requester    get_interface */
    },

    new_face: pcf_new_face,

    init_face: Some(pcf_face_init), /* FT_Face_InitFunc  init_face */
    done_face: Some(pcf_face_done), /* FT_Face_DoneFunc  done_face */
    init_size: None,                /* FT_Size_InitFunc  init_size */
    done_size: None,                /* FT_Size_DoneFunc  done_size */
    init_slot: None,                /* FT_Slot_InitFunc  init_slot */
    done_slot: None,                /* FT_Slot_DoneFunc  done_slot */

    load_glyph: Some(pcf_glyph_load), /* FT_Slot_LoadFunc  load_glyph */

    get_kerning: None,  /* FT_Face_GetKerningFunc   get_kerning  */
    attach_file: None,  /* FT_Face_AttachFunc       attach_file  */
    get_advances: None, /* FT_Face_GetAdvancesFunc  get_advances */

    request_size: Some(pcf_size_request), /* FT_Size_RequestFunc  request_size */
    select_size: Some(pcf_size_select),   /* FT_Size_SelectFunc   select_size  */
};
