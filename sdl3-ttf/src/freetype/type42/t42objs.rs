// Rust translation of src/type42/t42objs.c, src/type42/t42objs.h and
// src/type42/t42types.h from FreeType (2.13.2, as SDL_ttf's
// external/freetype pins it).
// Copyright (C) 2002-2023 by Roberto Alameda.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! Type 42 objects manager (body).
//!
//! The embedded TrueType font is its own face (`ttf_face`), opened with
//! the TrueType driver on the font's `sfnts` data; its single size is
//! the Type 42 size's (C's `ttsize`) and its glyph slot the Type 42
//! slot's (`ttslot`), which shares the Type 42 slot's internal record
//! (the glyph loader) while it loads a glyph. The root face's family and
//! style names are copies of the font info's strings.

use std::sync::Arc;

use super::super::base::ftobjs::*;
use super::super::fttypes::*;
use super::super::psaux::t1cmap::*;
use super::super::psnames::psmodule::PsUnicodesRec;
use super::super::t1tables::*;
use super::super::t1types::*;
use super::super::tttables::*;
use super::t42parse::*;

/// `T42_FaceRec`
#[derive(Debug, Default)]
pub struct T42FaceRec {
    pub root: FtFaceRec,
    pub type1: T1FontRec,
    /// (`psnames`: whether the `psnames' module is there)
    pub psnames: bool,
    /// (`psaux`: whether the `psaux' module is there)
    pub psaux: bool,
    /* (the `#if 0'ed `afm_data') */
    pub ttf_data: Vec<u8>,
    pub ttf_size: FtLong,
    pub ttf_face: Option<FtFace>,
    /* (C's unused `charmaprecs' and `charmaps') */
    pub unicode_map: PsUnicodesRec,
}

/// `ft_strtol( s, NULL, 10 )` of a C string (`strtol`: leading white
/// space, a sign, decimal digits; saturated on overflow).
fn ft_strtol(s: &[u8]) -> FtLong {
    let mut i = 0;
    while i < s.len() && matches!(s[i], b' ' | b'\t' | b'\n' | 0x0B | 0x0C | b'\r') {
        i += 1;
    }
    let mut negative = false;
    if i < s.len() && (s[i] == b'+' || s[i] == b'-') {
        negative = s[i] == b'-';
        i += 1;
    }
    let mut v: i128 = 0;
    while i < s.len() && s[i].is_ascii_digit() {
        v = (v * 10 + (s[i] - b'0') as i128).min(i64::MAX as i128 + 1);
        i += 1;
    }
    if negative {
        v = -v;
    }
    v.clamp(i64::MIN as i128, i64::MAX as i128) as FtLong
}

/// The TrueType glyph index of the Type 42 glyph `glyph_index` (its
/// charstring, a decimal number).
pub fn t42_ttf_glyph_index(type1: &T1FontRec, glyph_index: usize) -> FtUInt {
    /* FIXME (upstream): C reads `charstrings[glyph_index]' without */
    /* checking the index, which FT_Load_Glyph does not check       */
    /* either; a missing charstring is glyph 0 here                 */
    ft_strtol(type1.charstrings.name(glyph_index).unwrap_or(b"0")) as FtUInt
}

/// `T42_Open_Face`
fn t42_open_face(face: &mut T42FaceRec) -> FtResult<()> {
    let mut loader = T42LoaderRec::default();

    t42_loader_init(&mut loader, face);

    face.ttf_data = Vec::new();
    face.ttf_size = 0;

    let error = t42_open_face_loader(face, &mut loader);

    /* Exit: */
    t42_loader_done(&mut loader);

    if error.is_err() {
        face.ttf_data = Vec::new();
        face.ttf_size = 0;
    }

    error
}

fn t42_open_face_loader(face: &mut T42FaceRec, loader: &mut T42LoaderRec) -> FtResult<()> {
    let mut error: FtResult<()> = Ok(());

    {
        let Some(stream) = face.root.stream.as_mut() else {
            return Err(FT_ERR_INVALID_STREAM_HANDLE);
        };
        t42_parser_init(&mut loader.parser, stream)?;
    }

    let base_len = loader.parser.base_len;
    t42_parse_dict(face, loader, base_len)?;

    let type1 = &mut face.type1;

    if type1.font_type != 42 {
        return Err(FT_ERR_UNKNOWN_FILE_FORMAT);
    }

    /* now, propagate the charstrings and glyphnames tables */
    /* to the Type1 data                                    */
    type1.num_glyphs = loader.num_glyphs;

    if loader.charstrings.init == 0 {
        error = Err(FT_ERR_INVALID_FILE_FORMAT);
    }

    loader.charstrings.init = 0;
    type1.charstrings = std::mem::take(&mut loader.charstrings).freeze();

    /* we copy the glyph names `block' and `elements' fields; */
    /* the `lengths' field must be released later             */
    type1.glyph_names = std::mem::take(&mut loader.glyph_names).freeze();

    /* we must now build type1.encoding when we have a custom array */
    if type1.encoding_type == T1_ENCODING_TYPE_ARRAY {
        /* OK, we do the following: for each element in the encoding   */
        /* table, look up the index of the glyph having the same name  */
        /* as defined in the CharStrings array.                        */
        /* The index is then stored in type1.encoding.char_index, and  */
        /* the name in type1.encoding.char_name                        */

        let mut min_char: FtInt = 0;
        let mut max_char: FtInt = 0;

        for charcode in 0..loader.encoding_table.max_elems.max(0) as usize {
            let char_name = loader.encoding_table.element(charcode).map(|e| {
                let len = e.iter().position(|&c| c == 0).unwrap_or(e.len());
                &e[..len]
            });

            type1.encoding.char_index[charcode] = 0;
            type1.encoding.char_name[charcode] = None; /* ".notdef" */

            if let Some(char_name) = char_name {
                for idx in 0..type1.num_glyphs.max(0) as usize {
                    let glyph_name = type1.glyph_names.name(idx).unwrap_or(b"");

                    if char_name == glyph_name {
                        type1.encoding.char_index[charcode] = idx as FtUShort;
                        type1.encoding.char_name[charcode] = Some(idx);

                        /* Change min/max encoded char only if glyph name is */
                        /* not /.notdef                                      */
                        if glyph_name != b".notdef" {
                            if (charcode as FtInt) < min_char {
                                min_char = charcode as FtInt;
                            }
                            if charcode as FtInt >= max_char {
                                max_char = charcode as FtInt + 1;
                            }
                        }
                        break;
                    }
                }
            }
        }

        type1.encoding.code_first = min_char;
        type1.encoding.code_last = max_char;
        type1.encoding.num_chars = loader.num_chars;
    }

    error
}

/***************** Driver Functions *************/

/// A C string's bytes (up to its first null byte).
fn c_str(s: &[u8]) -> &[u8] {
    let len = s.iter().position(|&c| c == 0).unwrap_or(s.len());
    &s[..len]
}

/// A C string's bytes as a string.
fn name_string(bytes: &[u8]) -> String {
    String::from_utf8_lossy(c_str(bytes)).into_owned()
}

/// `T42_Face_Init` (the stream is the face's)
pub fn t42_face_init(
    face: &mut T42FaceRec,
    face_index: FtInt,
    params: &[FtParameter],
) -> FtResult<()> {
    face.ttf_face = None;
    face.root.num_faces = 1;

    let library = face
        .root
        .library
        .clone()
        .ok_or(FT_ERR_INVALID_LIBRARY_HANDLE)?;

    /* FT_FACE_FIND_GLOBAL_SERVICE( face, psnames, POSTSCRIPT_CMAPS ) */
    let psnames = library.get_module("psnames").is_some();
    face.psnames = psnames;

    face.psaux = library.get_module("psaux").is_some_and(|m| {
        matches!(
            library.modules[m].clazz.root().module_interface,
            FtModuleInterface::PsAux
        )
    });
    if !face.psaux {
        return Err(FT_ERR_MISSING_MODULE);
    }

    /* open the tokenizer, this will also check the font format */
    t42_open_face(face)?;

    /* if we just wanted to check the format, leave successfully now */
    if face_index < 0 {
        return Ok(());
    }

    /* check the face index */
    if (face_index & 0xFFFF) > 0 {
        return Err(FT_ERR_INVALID_ARGUMENT);
    }

    /* Now load the font program into the face object */

    /* Init the face object fields */
    /* Now set up root face fields */
    {
        let type1 = &face.type1;
        let info = &type1.font_info;
        let root = &mut face.root;

        root.num_glyphs = type1.num_glyphs as FtLong;
        root.num_charmaps = 0;
        root.face_index = 0;

        root.face_flags |=
            FT_FACE_FLAG_SCALABLE | FT_FACE_FLAG_HORIZONTAL | FT_FACE_FLAG_GLYPH_NAMES;

        if info.is_fixed_pitch != 0 {
            root.face_flags |= FT_FACE_FLAG_FIXED_WIDTH;
        }

        /* (TT_CONFIG_OPTION_BYTECODE_INTERPRETER is defined) */
        root.face_flags |= FT_FACE_FLAG_HINTER;

        /* XXX: TODO -- add kerning with .afm support */

        /* get style name -- be careful, some broken fonts only */
        /* have a `/FontName' dictionary entry!                 */
        let mut family_name: Option<&[u8]> = info.family_name.as_deref();

        /* assume "Regular" style if we don't know better */
        let mut style_name: &[u8] = b"Regular";
        if let Some(family) = family_name {
            if let Some(full) = info.full_name.as_deref() {
                let at = |s: &[u8], i: usize| s.get(i).copied().unwrap_or(0);
                let (mut f, mut g) = (0usize, 0usize);

                while at(full, f) != 0 {
                    if at(full, f) == at(family, g) {
                        g += 1;
                        f += 1;
                    } else if at(full, f) == b' ' || at(full, f) == b'-' {
                        f += 1;
                    } else if at(family, g) == b' ' || at(family, g) == b'-' {
                        g += 1;
                    } else {
                        if at(family, g) == 0 {
                            style_name = &full[f..];
                        }
                        break;
                    }
                }
            }
        } else {
            /* do we have a `/FontName'? */
            if let Some(name) = type1.font_name.as_deref() {
                family_name = Some(name);
            }
        }

        root.family_name = family_name.map(name_string);
        root.style_name = Some(name_string(style_name));

        /* no embedded bitmap support */
        root.num_fixed_sizes = 0;
        root.available_sizes = Vec::new();
    }

    /* Load the TTF font embedded in the T42 font */
    {
        let mut flags = FT_OPEN_MEMORY | FT_OPEN_DRIVER;
        let driver = library.get_module("truetype");
        let data = face
            .ttf_data
            .get(..face.ttf_size.max(0) as usize)
            .unwrap_or(&[]);

        if !params.is_empty() {
            flags |= FT_OPEN_PARAMS;
        }

        let args = FtOpenArgs {
            flags,
            memory_base: Some(Arc::from(data)),
            driver,
            params: params.to_vec(),
            ..Default::default()
        };

        face.ttf_face = Some(ft_open_face(&library, args, 0)?);
    }

    let ttf_face = face.ttf_face.as_mut().unwrap();
    ft_done_size(ttf_face);

    let ttf: &FtFaceRec = ttf_face;
    let info = &face.type1.font_info;
    let root = &mut face.root;

    /* Ignore info in FontInfo dictionary and use the info from the  */
    /* loaded TTF font.  The PostScript interpreter also ignores it. */
    root.bbox = ttf.bbox;
    root.units_per_EM = ttf.units_per_EM;

    root.ascender = ttf.ascender;
    root.descender = ttf.descender;
    root.height = ttf.height;

    root.max_advance_width = ttf.max_advance_width;
    root.max_advance_height = ttf.max_advance_height;

    root.underline_position = info.underline_position;
    root.underline_thickness = info.underline_thickness as FtShort;

    /* compute style flags */
    root.style_flags = 0;
    if info.italic_angle != 0 {
        root.style_flags |= FT_STYLE_FLAG_ITALIC;
    }

    if ttf.style_flags & FT_STYLE_FLAG_BOLD != 0 {
        root.style_flags |= FT_STYLE_FLAG_BOLD;
    }

    if ttf.face_flags & FT_FACE_FLAG_VERTICAL != 0 {
        root.face_flags |= FT_FACE_FLAG_VERTICAL;
    }

    if psnames {
        /* (the `psaux' module's charmap classes) */
        let mut charmap = FtCharMapRec {
            /* first of all, try to synthesize a Unicode charmap */
            platform_id: TT_PLATFORM_MICROSOFT,
            encoding_id: TT_MS_ID_UNICODE_CS,
            encoding: FT_ENCODING_UNICODE,
        };

        match t1_cmap_unicode_init(&face.type1) {
            Ok(data) => {
                ft_cmap_new(&mut face.root, &T1_CMAP_UNICODE_CLASS_REC, data, charmap)?;
            }
            Err(error)
                if error != FT_ERR_NO_UNICODE_GLYPH_NAME
                    && error != FT_ERR_UNIMPLEMENTED_FEATURE =>
            {
                return Err(error);
            }
            Err(_) => {}
        }

        /* now, generate an Adobe Standard encoding when appropriate */
        charmap.platform_id = TT_PLATFORM_ADOBE;
        let mut clazz: Option<&'static FtCMapClassRec> = None;
        let mut init: Option<fn(&T1FontRec) -> FtResult<FtCMapData>> = None;

        match face.type1.encoding_type {
            T1_ENCODING_TYPE_STANDARD => {
                charmap.encoding = FT_ENCODING_ADOBE_STANDARD;
                charmap.encoding_id = TT_ADOBE_ID_STANDARD;
                clazz = Some(&T1_CMAP_STANDARD_CLASS_REC);
                init = Some(t1_cmap_standard_init);
            }

            T1_ENCODING_TYPE_EXPERT => {
                charmap.encoding = FT_ENCODING_ADOBE_EXPERT;
                charmap.encoding_id = TT_ADOBE_ID_EXPERT;
                clazz = Some(&T1_CMAP_EXPERT_CLASS_REC);
                init = Some(t1_cmap_expert_init);
            }

            T1_ENCODING_TYPE_ARRAY => {
                charmap.encoding = FT_ENCODING_ADOBE_CUSTOM;
                charmap.encoding_id = TT_ADOBE_ID_CUSTOM;
                clazz = Some(&T1_CMAP_CUSTOM_CLASS_REC);
                init = Some(t1_cmap_custom_init);
            }

            T1_ENCODING_TYPE_ISOLATIN1 => {
                charmap.encoding = FT_ENCODING_ADOBE_LATIN_1;
                charmap.encoding_id = TT_ADOBE_ID_LATIN_1;
                clazz = Some(&T1_CMAP_UNICODE_CLASS_REC);
                init = Some(t1_cmap_unicode_init);
            }

            _ => {}
        }

        if let (Some(clazz), Some(init)) = (clazz, init) {
            let data = init(&face.type1)?;
            ft_cmap_new(&mut face.root, clazz, data, charmap)?;
        }
    }

    /* Exit: */
    Ok(())
}

/// `T42_Face_Done`
pub fn t42_face_done(face: &mut T42FaceRec) {
    /* delete internal ttf face prior to freeing face->ttf_data */
    if let Some(ttf_face) = face.ttf_face.take() {
        ft_done_face(ttf_face);
    }

    let type1 = &mut face.type1;

    /* release font info strings */
    let info = &mut type1.font_info;
    info.version = None;
    info.notice = None;
    info.full_name = None;
    info.family_name = None;
    info.weight = None;

    /* release top dictionary */
    type1.charstrings = Default::default();
    type1.glyph_names = Default::default();

    type1.encoding.char_index = Vec::new();
    type1.encoding.char_name = Vec::new();
    type1.font_name = None;

    face.ttf_data = Vec::new();

    /* (the `#if 0'ed release of the afm data) */

    /* release unicode map, if any */
    face.unicode_map = PsUnicodesRec::default();

    face.root.family_name = None;
    face.root.style_name = None;
}

/// `T42_Driver_Init`: initializes a given Type 42 driver object (C's
/// `ttclazz`, the TrueType driver's class, is the library's module's).
pub fn t42_driver_init(library: &mut FtLibraryRec, _module: usize) -> FtResult<()> {
    if library.get_module("truetype").is_none() {
        return Err(FT_ERR_MISSING_MODULE);
    }

    Ok(())
}

/// `T42_Driver_Done`
pub fn t42_driver_done(_module: &FtModuleRec) {}

/// `T42_Size_Init`
pub fn t42_size_init(face: &mut T42FaceRec) -> FtResult<()> {
    let Some(ttf_face) = face.ttf_face.as_mut() else {
        return Err(FT_ERR_INVALID_FACE_HANDLE);
    };

    /* (FT_New_Size and FT_Activate_Size on the TrueType face's single size) */
    ft_new_size(ttf_face)
}

/// `T42_Size_Request`
pub fn t42_size_request(face: &mut T42FaceRec, req: &FtSizeRequestRec) -> FtResult<()> {
    let Some(ttf_face) = face.ttf_face.as_mut() else {
        return Err(FT_ERR_INVALID_FACE_HANDLE);
    };

    /* (FT_Activate_Size: the TrueType face's single size) */
    ft_request_size(ttf_face, req)?;
    face.root.size.metrics = ttf_face.size.metrics;

    Ok(())
}

/// `T42_Size_Select`
pub fn t42_size_select(face: &mut T42FaceRec, strike_index: FtULong) -> FtResult<()> {
    let Some(ttf_face) = face.ttf_face.as_mut() else {
        return Err(FT_ERR_INVALID_FACE_HANDLE);
    };

    /* (FT_Activate_Size: the TrueType face's single size) */
    ft_select_size(ttf_face, strike_index as FtInt)?;
    face.root.size.metrics = ttf_face.size.metrics;

    Ok(())
}

/// `T42_Size_Done`
pub fn t42_size_done(face: &mut T42FaceRec) {
    /* (the TrueType face's single size) */
    if let Some(ttf_face) = face.ttf_face.as_mut() {
        ft_done_size(ttf_face);
    }
}

/// `T42_GlyphSlot_Init` (the slot's `ttslot` is the TrueType face's slot,
/// whose internal record is the Type 42 slot's while it loads a glyph:
/// C's sharing of the loader so that the autohinter can see it)
pub fn t42_glyph_slot_init(_slot: &mut FtGlyphSlotRec) -> FtResult<()> {
    Ok(())
}

/// `T42_GlyphSlot_Done` (the TrueType face's slot goes with its face)
pub fn t42_glyph_slot_done(_slot: &mut FtGlyphSlotRec) {}

/// `t42_glyphslot_clear`
fn t42_glyphslot_clear(slot: &mut FtGlyphSlotRec) {
    /* free bitmap if needed */
    ft_glyphslot_free_bitmap(slot);

    /* clear all public fields in the glyph slot */
    slot.metrics = FtGlyphMetrics::default();
    slot.outline = FtOutline::default();
    slot.bitmap = FtBitmap::default();

    slot.bitmap_left = 0;
    slot.bitmap_top = 0;
    slot.num_subglyphs = 0;
    slot.subglyphs = Vec::new();
    slot.control_data = Vec::new();
    slot.control_len = 0;
    slot.other = None;
    slot.format = FT_GLYPH_FORMAT_NONE;

    slot.linearHoriAdvance = 0;
    slot.linearVertAdvance = 0;
}

/// `T42_GlyphSlot_Load` (into the face's glyph slot, at its size)
pub fn t42_glyph_slot_load(
    face: &mut FtFace,
    glyph_index: FtUInt,
    load_flags: FtInt32,
) -> FtResult<()> {
    let FtFace::T42(t42face) = face else {
        return Err(FT_ERR_INVALID_FACE_HANDLE);
    };
    let T42FaceRec {
        root,
        type1,
        ttf_face,
        ..
    } = &mut **t42face;
    let Some(ttf_face) = ttf_face.as_mut() else {
        return Err(FT_ERR_INVALID_FACE_HANDLE);
    };
    let ttclazz = ttf_face.driver_class();

    /* map T42 glyph index to embedded TTF's glyph index */
    let glyph_index = t42_ttf_glyph_index(type1, glyph_index as usize);

    t42_glyphslot_clear(&mut ttf_face.glyph);

    /* (the slots share the Type 42 slot's internal record) */
    std::mem::swap(&mut root.glyph.internal, &mut ttf_face.glyph.internal);
    let error = match ttclazz.load_glyph {
        Some(load_glyph) => load_glyph(ttf_face, glyph_index, load_flags | FT_LOAD_NO_BITMAP),
        None => Err(FT_ERR_UNIMPLEMENTED_FEATURE),
    };
    std::mem::swap(&mut root.glyph.internal, &mut ttf_face.glyph.internal);

    if error.is_ok() {
        let ttslot = &ttf_face.glyph;
        let glyph = &mut root.glyph;

        glyph.metrics = ttslot.metrics;
        glyph.linearHoriAdvance = ttslot.linearHoriAdvance;
        glyph.linearVertAdvance = ttslot.linearVertAdvance;
        glyph.format = ttslot.format;
        glyph.outline = ttslot.outline.clone();

        glyph.bitmap = ttslot.bitmap.clone();
        glyph.bitmap_left = ttslot.bitmap_left;
        glyph.bitmap_top = ttslot.bitmap_top;

        glyph.num_subglyphs = ttslot.num_subglyphs;
        glyph.subglyphs = ttslot.subglyphs.clone();
        glyph.control_data = ttslot.control_data.clone();
        glyph.control_len = ttslot.control_len;
    }

    error
}
