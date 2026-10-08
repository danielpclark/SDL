// Rust translation of src/sfnt/sfobjs.c from FreeType (2.13.2, as SDL_ttf's
// external/freetype pins it).
// Copyright (C) 1996-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! SFNT object management (base).
//!
//! The `SFNT_Service` interface of C is replaced by direct calls into the
//! `sfnt` module's loaders; the `psnames`, `mm`, and `tt_var` services are
//! likewise called directly.  WOFF2 needs Brotli, which SDL_ttf's bundled
//! FreeType build does not enable (`FT_CONFIG_OPTION_USE_BROTLI` is
//! undefined), so only WOFF (zlib) is handled.

use super::super::base::ftobjs::*;
use super::super::base::ftstream::FtStreamRec;
use super::super::fttypes::*;
use super::super::tttables::*;
use super::super::tttypes::*;
use super::sfwoff::woff_open_font;
use super::ttbdf::tt_face_free_bdf_props;
use super::ttcmap::{tt_cmap_unicode_init, tt_face_build_cmaps, TT_CMAP_UNICODE_CLASS_REC};
use super::ttcolr::{tt_face_free_colr, tt_face_load_colr};
use super::ttcpal::{tt_face_free_cpal, tt_face_load_cpal};
use super::ttkern::{tt_face_done_kern, tt_face_load_kern};
use super::ttload::*;
use super::ttmtx::{tt_face_load_hhea, tt_face_load_hmtx};
use super::ttpost::tt_face_free_ps_names;
use super::ttsbit::{tt_face_free_sbit, tt_face_load_sbit, tt_face_load_strike_metrics};
use super::ttsvg::{tt_face_free_svg, tt_face_load_svg};

/// `tt_name_ascii_from_utf16`: convert a UTF-16 name entry to ASCII
fn tt_name_ascii_from_utf16(entry: &TtNameRec) -> Option<String> {
    let read = entry.string.as_deref().unwrap_or(&[]);

    let len = entry.stringLength as FtUInt / 2;
    let mut string = String::new();
    if string.try_reserve_exact(len as usize + 1).is_err() {
        return None;
    }

    let mut p = 0;
    for _ in 0..len {
        let mut code = super::super::base::ftstream::ft_next_ushort(read, &mut p) as FtUInt;

        if code == 0 {
            break;
        }

        if !(32..=127).contains(&code) {
            code = b'?' as FtUInt;
        }

        string.push(code as u8 as char);
    }

    Some(string)
}

/// `tt_name_ascii_from_other`: convert an Apple Roman or symbol name entry
/// to ASCII
fn tt_name_ascii_from_other(entry: &TtNameRec) -> Option<String> {
    let read = entry.string.as_deref().unwrap_or(&[]);

    let len = entry.stringLength as FtUInt;
    let mut string = String::new();
    if string.try_reserve_exact(len as usize + 1).is_err() {
        return None;
    }

    for n in 0..len as usize {
        let mut code = read.get(n).copied().unwrap_or(0) as FtUInt;

        if code == 0 {
            break;
        }

        if !(32..=127).contains(&code) {
            code = b'?' as FtUInt;
        }

        string.push(code as u8 as char);
    }

    Some(string)
}

/// `TT_Name_ConvertFunc`
type TtNameConvertFunc = fn(&TtNameRec) -> Option<String>;

/// `tt_face_get_name` (documentation is in sfnt.h)
pub fn tt_face_get_name(
    face: &mut TtFaceRec,
    stream: &mut FtStreamRec,
    nameid: FtUShort,
) -> FtResult<Option<String>> {
    let mut result = None;

    let mut found_apple: FtInt;
    let mut found_apple_roman: FtInt = -1;
    let mut found_apple_english: FtInt = -1;
    let mut found_win: FtInt = -1;
    let mut found_unicode: FtInt = -1;

    let mut is_english = false;

    for n in 0..face.num_names as usize {
        let rec = &face.name_table.names[n];

        /* According to the OpenType 1.3 specification, only Microsoft or  */
        /* Apple platform IDs might be used in the `name' table.  The      */
        /* `Unicode' platform is reserved for the `cmap' table, and the    */
        /* `ISO' one is deprecated.                                        */
        /*                                                                 */
        /* However, the Apple TrueType specification doesn't say the same  */
        /* thing and goes to suggest that all Unicode `name' table entries */
        /* should be coded in UTF-16 (in big-endian format I suppose).     */
        /*                                                                 */
        if rec.nameID == nameid && rec.stringLength > 0 {
            match rec.platformID {
                TT_PLATFORM_APPLE_UNICODE | TT_PLATFORM_ISO => {
                    /* there is `languageID' to check there.  We should use this */
                    /* field only as a last solution when nothing else is        */
                    /* available.                                                */
                    /*                                                           */
                    found_unicode = n as FtInt;
                }

                TT_PLATFORM_MACINTOSH => {
                    /* This is a bit special because some fonts will use either    */
                    /* an English language id, or a Roman encoding id, to indicate */
                    /* the English version of its font name.                       */
                    /*                                                             */
                    if rec.languageID == TT_MAC_LANGID_ENGLISH {
                        found_apple_english = n as FtInt;
                    } else if rec.encodingID == TT_MAC_ID_ROMAN {
                        found_apple_roman = n as FtInt;
                    }
                }

                TT_PLATFORM_MICROSOFT => {
                    /* we only take a non-English name when there is nothing */
                    /* else available in the font                            */
                    /*                                                       */
                    if found_win == -1 || (rec.languageID & 0x3FF) == 0x009 {
                        match rec.encodingID {
                            TT_MS_ID_SYMBOL_CS | TT_MS_ID_UNICODE_CS | TT_MS_ID_UCS_4 => {
                                is_english = (rec.languageID & 0x3FF) == 0x009;
                                found_win = n as FtInt;
                            }
                            _ => {}
                        }
                    }
                }

                _ => {}
            }
        }
    }

    found_apple = found_apple_roman;
    if found_apple_english >= 0 {
        found_apple = found_apple_english;
    }

    /* some fonts contain invalid Unicode or Macintosh formatted entries; */
    /* we will thus favor names encoded in Windows formats if available   */
    /* (provided it is an English name)                                   */
    /*                                                                    */
    let mut convert: Option<TtNameConvertFunc> = None;
    let mut rec: Option<usize> = None;
    if found_win >= 0 && !(found_apple >= 0 && !is_english) {
        rec = Some(found_win as usize);
        match face.name_table.names[found_win as usize].encodingID {
            /* all Unicode strings are encoded using UTF-16BE */
            TT_MS_ID_UNICODE_CS | TT_MS_ID_SYMBOL_CS => {
                convert = Some(tt_name_ascii_from_utf16);
            }

            TT_MS_ID_UCS_4 => {
                /* Apparently, if this value is found in a name table entry, it is */
                /* documented as `full Unicode repertoire'.  Experience with the   */
                /* MsGothic font shipped with Windows Vista shows that this really */
                /* means UTF-16 encoded names (UCS-4 values are only used within   */
                /* charmaps).                                                      */
                convert = Some(tt_name_ascii_from_utf16);
            }

            _ => {}
        }
    } else if found_apple >= 0 {
        rec = Some(found_apple as usize);
        convert = Some(tt_name_ascii_from_other);
    } else if found_unicode >= 0 {
        rec = Some(found_unicode as usize);
        convert = Some(tt_name_ascii_from_utf16);
    }

    if let (Some(rec), Some(convert)) = (rec, convert) {
        let rec = &mut face.name_table.names[rec];

        if rec.string.is_none() {
            let r = (|| -> FtResult<Vec<u8>> {
                let mut s = super::super::base::ftmemory::ft_qalloc(rec.stringLength as FtLong)?;
                stream.seek(rec.stringOffset)?;
                stream.read(&mut s)?;
                Ok(s)
            })();
            match r {
                Ok(s) => rec.string = Some(s),
                Err(e) => {
                    rec.string = None;
                    rec.stringLength = 0;
                    return Err(e);
                }
            }
        }

        result = convert(rec);
    }

    /* Exit: */
    Ok(result)
}

/// `sfnt_find_encoding`
fn sfnt_find_encoding(platform_id: FtInt, encoding_id: FtInt) -> FtEncoding {
    /* TEncoding */
    struct TEncoding {
        platform_id: FtInt,
        encoding_id: FtInt,
        encoding: FtEncoding,
    }

    const fn te(platform_id: u16, encoding_id: i32, encoding: FtEncoding) -> TEncoding {
        TEncoding {
            platform_id: platform_id as FtInt,
            encoding_id,
            encoding,
        }
    }

    static TT_ENCODINGS: [TEncoding; 11] = [
        te(TT_PLATFORM_ISO, -1, FT_ENCODING_UNICODE),
        te(TT_PLATFORM_APPLE_UNICODE, -1, FT_ENCODING_UNICODE),
        te(
            TT_PLATFORM_MACINTOSH,
            TT_MAC_ID_ROMAN as i32,
            FT_ENCODING_APPLE_ROMAN,
        ),
        te(
            TT_PLATFORM_MICROSOFT,
            TT_MS_ID_SYMBOL_CS as i32,
            FT_ENCODING_MS_SYMBOL,
        ),
        te(
            TT_PLATFORM_MICROSOFT,
            TT_MS_ID_UCS_4 as i32,
            FT_ENCODING_UNICODE,
        ),
        te(
            TT_PLATFORM_MICROSOFT,
            TT_MS_ID_UNICODE_CS as i32,
            FT_ENCODING_UNICODE,
        ),
        te(
            TT_PLATFORM_MICROSOFT,
            TT_MS_ID_SJIS as i32,
            FT_ENCODING_SJIS,
        ),
        te(TT_PLATFORM_MICROSOFT, TT_MS_ID_PRC as i32, FT_ENCODING_PRC),
        te(
            TT_PLATFORM_MICROSOFT,
            TT_MS_ID_BIG_5 as i32,
            FT_ENCODING_BIG5,
        ),
        te(
            TT_PLATFORM_MICROSOFT,
            TT_MS_ID_WANSUNG as i32,
            FT_ENCODING_WANSUNG,
        ),
        te(
            TT_PLATFORM_MICROSOFT,
            TT_MS_ID_JOHAB as i32,
            FT_ENCODING_JOHAB,
        ),
    ];

    for cur in TT_ENCODINGS.iter() {
        if cur.platform_id == platform_id
            && (cur.encoding_id == encoding_id || cur.encoding_id == -1)
        {
            return cur.encoding;
        }
    }

    FT_ENCODING_NONE
}

/// `sfnt_open_font`: Fill in face->ttc_header.  If the font is not a TTC,
/// it is synthesized into a TTC with one offset table.
///
/// For a WOFF, `stream` is replaced by the synthesized SFNT stream.
fn sfnt_open_font(stream: &mut FtStreamRec, face: &mut TtFaceRec) -> FtResult<()> {
    face.ttc_header.tag = 0;
    face.ttc_header.version = 0;
    face.ttc_header.count = 0;

    let (offset, tag) = loop {
        /* retry: */
        let offset = stream.pos();

        let tag = stream.read_ulong()? as FtULong;

        if tag == TTAG_wOFF as FtULong {
            stream.seek(offset)?;

            let sfnt_stream = woff_open_font(stream, face)?;

            /* Swap out stream and retry! */
            *stream = *sfnt_stream;
            face.root.face_flags &= !FT_FACE_FLAG_EXTERNAL_STREAM;
            continue;
        }

        break (offset, tag);
    };

    if tag != 0x00010000
        && tag != TTAG_ttcf as FtULong
        && tag != TTAG_OTTO as FtULong
        && tag != TTAG_true as FtULong
        && tag != TTAG_typ1 as FtULong
        && tag != TTAG_0xA5kbd as FtULong
        && tag != TTAG_0xA5lst as FtULong
        && tag != 0x00020000
    {
        return Err(FT_ERR_UNKNOWN_FILE_FORMAT);
    }

    face.ttc_header.tag = TTAG_ttcf as FtULong;

    if tag == TTAG_ttcf as FtULong {
        stream.enter_frame(8)?;
        face.ttc_header.version = stream.get_long() as FtFixed;
        face.ttc_header.count = stream.get_long() as FtLong; /* this is ULong in the specs */
        stream.exit_frame();

        if face.ttc_header.count == 0 {
            return Err(FT_ERR_INVALID_TABLE);
        }

        /* a rough size estimate: let's conservatively assume that there   */
        /* is just a single table info in each subfont header (12 + 16*1 = */
        /* 28 bytes), thus we have (at least) `12 + 4*count' bytes for the */
        /* size of the TTC header plus `28*count' bytes for all subfont    */
        /* headers                                                         */
        if face.ttc_header.count as FtULong > stream.size / (28 + 4) {
            return Err(FT_ERR_ARRAY_TOO_LARGE);
        }

        /* now read the offsets of each font in the file */
        face.ttc_header.offsets =
            super::super::base::ftmemory::ft_new_array(face.ttc_header.count)?;

        stream.enter_frame(face.ttc_header.count as FtULong * 4)?;

        for n in 0..face.ttc_header.count as usize {
            face.ttc_header.offsets[n] = stream.get_ulong() as FtULong;
        }

        stream.exit_frame();
    } else {
        face.ttc_header.version = 1 << 16;
        face.ttc_header.count = 1;

        face.ttc_header.offsets = vec![offset];
    }

    Ok(())
}

/// `sfnt_init_face`
pub fn sfnt_init_face(
    face: &mut TtFaceRec,
    face_instance_index: FtInt,
    _params: &[FtParameter],
) -> FtResult<()> {
    /* for now, parameters are unused */

    with_stream(face, |face, stream| -> FtResult<()> {
        sfnt_open_font(stream, face)?;

        /* Stream may have changed in sfnt_open_font. */

        let mut face_index = face_instance_index.wrapping_abs() & 0xFFFF;

        /* value -(N+1) requests information on index N */
        if face_instance_index < 0 && face_index > 0 {
            face_index -= 1;
        }

        if face_index as FtLong >= face.ttc_header.count {
            if face_instance_index >= 0 {
                return Err(FT_ERR_INVALID_ARGUMENT);
            } else {
                face_index = 0;
            }
        }

        stream.seek(face.ttc_header.offsets[face_index as usize])?;

        /* check whether we have a valid TrueType file */
        tt_face_load_font_dir(face, stream)?;

        /* TT_CONFIG_OPTION_GX_VAR_SUPPORT */
        {
            let mut version: FtULong = 0;
            let mut offset: FtULong = 0;
            let mut num_axes: FtUShort = 0;
            let mut axis_size: FtUShort = 0;
            let mut num_instances: FtUShort = 0;
            let mut instance_size: FtUShort = 0;

            let instance_index = face_instance_index.wrapping_abs() >> 16;

            /* test whether current face is a GX font with named instances */
            let mut fvar_len: FtULong = 0;
            let r = (|| -> FtResult<()> {
                fvar_len = tt_face_goto_table(face, TTAG_fvar as FtULong, stream)?;
                if fvar_len < 20 {
                    return Err(FT_ERR_INVALID_TABLE);
                }
                version = stream.read_ulong()? as FtULong;
                offset = stream.read_ushort()? as FtULong;
                stream.skip(2)?; /* reserved */
                num_axes = stream.read_ushort()?;
                axis_size = stream.read_ushort()?;
                num_instances = stream.read_ushort()?;
                instance_size = stream.read_ushort()?;
                Ok(())
            })();
            if r.is_err() {
                version = 0;
                offset = 0;
                num_axes = 0;
                axis_size = 0;
                num_instances = 0;
                instance_size = 0;
            }

            /* check that the data is bound by the table length */
            if version != 0x00010000
                || axis_size != 20
                || num_axes == 0
                /* `num_axes' limit implied by 16-bit `instance_size' */
                || num_axes > 0x3FFE
                || !(instance_size as FtUInt == 4 + 4 * num_axes as FtUInt
                    || instance_size as FtUInt == 6 + 4 * num_axes as FtUInt)
                /* `num_instances' limit implied by limited range of name IDs */
                || num_instances > 0x7EFF
                || offset
                    + axis_size as FtULong * num_axes as FtULong
                    + instance_size as FtULong * num_instances as FtULong
                    > fvar_len
            {
                num_instances = 0;
            } else {
                face.variation_support |= TT_FACE_FLAG_VAR_FVAR;
            }

            /*
             * As documented in the OpenType specification, an entry for the
             * default instance may be omitted in the named instance table.  In
             * particular this means that even if there is no named instance
             * table in the font we actually do have a named instance, namely the
             * default instance.
             *
             * For consistency, we always want the default instance in our list
             * of named instances.  If it is missing, we try to synthesize it
             * later on.  Here, we have to adjust `num_instances' accordingly.
             */

            if face.variation_support & TT_FACE_FLAG_VAR_FVAR != 0 {
                if let (Ok(mut default_values), Ok(mut instance_values)) = (
                    super::super::base::ftmemory::ft_alloc(num_axes as FtLong * 4),
                    super::super::base::ftmemory::ft_alloc(num_axes as FtLong * 4),
                ) {
                    /* the current stream position is 16 bytes after the table start */
                    let array_start = stream.pos() - 16 + offset;

                    let mut default_value_offset = array_start + 8;
                    for i in 0..num_axes as usize {
                        /* Note (upstream): C reads into uninitialized memory */
                        /* and ignores read errors; the buffers are zeroed here. */
                        let _ = stream
                            .read_at(default_value_offset, &mut default_values[i * 4..i * 4 + 4]);
                        default_value_offset += axis_size as FtULong;
                    }

                    let mut instance_offset =
                        array_start + axis_size as FtULong * num_axes as FtULong + 4;

                    let mut i: FtUInt = 0;
                    while i < num_instances as FtUInt {
                        let _ = stream.read_at(instance_offset, &mut instance_values);

                        if default_values == instance_values {
                            break;
                        }

                        instance_offset += instance_size as FtULong;
                        i += 1;
                    }

                    /* named instance indices start with value 1 */
                    face.var_default_named_instance = i + 1;

                    if i == num_instances as FtUInt {
                        /* no default instance in named instance table; */
                        /* we thus have to synthesize it                */
                        num_instances += 1;
                    }
                }
            }

            /* we don't support Multiple Master CFFs yet; */
            /* note that `glyf' or `CFF2' have precedence */
            if tt_face_goto_table(face, TTAG_glyf as FtULong, stream).is_err()
                && tt_face_goto_table(face, TTAG_CFF2 as FtULong, stream).is_err()
                && tt_face_goto_table(face, TTAG_CFF as FtULong, stream).is_ok()
            {
                num_instances = 0;
            }

            /* instance indices in `face_instance_index' start with index 1, */
            /* thus `>' and not `>='                                         */
            if instance_index > num_instances as FtInt {
                if face_instance_index >= 0 {
                    return Err(FT_ERR_INVALID_ARGUMENT);
                } else {
                    num_instances = 0;
                }
            }

            face.root.style_flags = (num_instances as FtLong) << 16;
        }

        face.root.num_faces = face.ttc_header.count;
        face.root.face_index = face_instance_index as FtLong;

        Ok(())
    })
}

/// `sfnt_load_face`
pub fn sfnt_load_face(
    face: &mut TtFaceRec,
    _face_instance_index: FtInt,
    params: &[FtParameter],
) -> FtResult<()> {
    with_stream(face, |face, stream| {
        sfnt_load_face_stream(face, stream, params)
    })
}

fn sfnt_load_face_stream(
    face: &mut TtFaceRec,
    stream: &mut FtStreamRec,
    params: &[FtParameter],
) -> FtResult<()> {
    let mut ignore_typographic_family = false;
    let mut ignore_typographic_subfamily = false;
    let mut ignore_sbix = false;

    /* Check parameters */
    for p in params {
        if p.tag == FT_PARAM_TAG_IGNORE_TYPOGRAPHIC_FAMILY {
            ignore_typographic_family = true;
        } else if p.tag == FT_PARAM_TAG_IGNORE_TYPOGRAPHIC_SUBFAMILY {
            ignore_typographic_subfamily = true;
        } else if p.tag == FT_PARAM_TAG_IGNORE_SBIX {
            ignore_sbix = true;
        }
    }

    /* Load tables */

    /* We now support two SFNT-based bitmapped font formats.  They */
    /* are recognized easily as they do not include a `glyf'       */
    /* table.                                                      */
    /*                                                             */
    /* The first format comes from Apple, and uses a table named   */
    /* `bhed' instead of `head' to store the font header (using    */
    /* the same format).  It also doesn't include horizontal and   */
    /* vertical metrics tables (i.e. `hhea' and `vhea' tables are  */
    /* missing).                                                   */
    /*                                                             */
    /* The other format comes from Microsoft, and is used with     */
    /* WinCE/PocketPC.  It looks like a standard TTF, except that  */
    /* it doesn't contain outlines.                                */
    /*                                                             */

    /* do we have outlines in there? */
    /* (FT_CONFIG_OPTION_INCREMENTAL: no incremental interface is ever set) */
    let mut has_outline = tt_face_lookup_table(face, TTAG_glyf as FtULong).is_some()
        || tt_face_lookup_table(face, TTAG_CFF as FtULong).is_some()
        || tt_face_lookup_table(face, TTAG_CFF2 as FtULong).is_some();

    /* check which sbit formats are present */
    let has_cblc = tt_face_goto_table(face, TTAG_CBLC as FtULong, stream).is_ok();
    let has_cbdt = tt_face_goto_table(face, TTAG_CBDT as FtULong, stream).is_ok();
    let has_eblc = tt_face_goto_table(face, TTAG_EBLC as FtULong, stream).is_ok();
    let has_bloc = tt_face_goto_table(face, TTAG_bloc as FtULong, stream).is_ok();
    let mut has_sbix = tt_face_goto_table(face, TTAG_sbix as FtULong, stream).is_ok();

    let mut is_apple_sbit = false;

    if ignore_sbix {
        has_sbix = false;
    }

    /* if this font doesn't contain outlines, we try to load */
    /* a `bhed' table                                        */
    if !has_outline {
        is_apple_sbit = tt_face_load_bhed(face, stream).is_ok();
    }

    /* load the font header (`head' table) if this isn't an Apple */
    /* sbit font file                                             */
    if !is_apple_sbit || has_sbix {
        tt_face_load_head(face, stream)?;
    }

    /* Ignore outlines for CBLC/CBDT fonts. */
    if has_cblc || has_cbdt {
        has_outline = false;
    }

    /* OpenType 1.8.2 introduced limits to this value;    */
    /* however, they make sense for older SFNT fonts also */
    if face.header.Units_Per_EM < 16 || face.header.Units_Per_EM > 16384 {
        return Err(FT_ERR_INVALID_TABLE);
    }

    /* the following tables are often not present in embedded TrueType */
    /* fonts within PDF documents, so don't check for them.            */
    let _ = tt_face_load_maxp(face, stream);
    let _ = tt_face_load_cmap(face, stream);

    /* the following tables are optional in PCL fonts -- */
    /* don't check for errors                            */
    let _ = tt_face_load_name(face, stream);
    let psnames_error = tt_face_load_post(face, stream).err();

    /* do not load the metrics headers and tables if this is an Apple */
    /* sbit font file                                                 */
    if !is_apple_sbit {
        /* load the `hhea' and `hmtx' tables */
        let mut error = tt_face_load_hhea(face, stream, false);
        match error {
            Ok(()) => {
                error = tt_face_load_hmtx(face, stream, false);
                if let Err(e) = error {
                    if ft_err_eq(e, FT_ERR_TABLE_MISSING) {
                        error = Err(FT_ERR_HMTX_TABLE_MISSING);
                    }
                }
            }
            Err(e) if ft_err_eq(e, FT_ERR_TABLE_MISSING) => {
                /* No `hhea' table necessary for SFNT Mac fonts. */
                if face.format_tag == TTAG_true as FtULong {
                    has_outline = false;
                    error = Ok(());
                } else {
                    error = Err(FT_ERR_HORIZ_HEADER_MISSING);
                }
            }
            Err(_) => {}
        }

        error?;

        /* try to load the `vhea' and `vmtx' tables */
        let mut error = tt_face_load_hhea(face, stream, true);
        if error.is_ok() {
            error = tt_face_load_hmtx(face, stream, true);
            if error.is_ok() {
                face.vertical_info = true;
            }
        }

        if let Err(e) = error {
            if ft_err_neq(e, FT_ERR_TABLE_MISSING) {
                return Err(e);
            }
        }

        if tt_face_load_os2(face, stream).is_err() {
            /* we treat the table as missing if there are any errors */
            face.os2.version = 0xFFFF;
        }
    }

    /* the optional tables */

    /* embedded bitmap support */
    /* TODO: Replace this clumsy check for all possible sbit tables     */
    /*       with something better (for example, by passing a parameter */
    /*       to suppress 'sbix' loading).                               */
    if has_cblc || has_eblc || has_bloc || has_sbix {
        let _ = tt_face_load_sbit(face, stream);
    }

    /* colored glyph support */
    let _ = tt_face_load_cpal(face, stream);
    let _ = tt_face_load_colr(face, stream);

    /* OpenType-SVG glyph support */
    let _ = tt_face_load_svg(face, stream);

    /* consider the pclt, kerning, and gasp tables as optional */
    let _ = tt_face_load_pclt(face, stream);
    let _ = tt_face_load_gasp(face, stream);
    let _ = tt_face_load_kern(face, stream);

    face.root.num_glyphs = face.max_profile.numGlyphs as FtLong;

    /* Bit 8 of the `fsSelection' field in the `OS/2' table denotes  */
    /* a WWS-only font face.  `WWS' stands for `weight', width', and */
    /* `slope', a term used by Microsoft's Windows Presentation      */
    /* Foundation (WPF).  This flag has been introduced in version   */
    /* 1.5 of the OpenType specification (May 2008).                 */

    face.root.family_name = None;
    face.root.style_name = None;
    if face.os2.version != 0xFFFF && face.os2.fsSelection & 256 != 0 {
        if !ignore_typographic_family {
            face.root.family_name = tt_face_get_name(face, stream, TT_NAME_ID_TYPOGRAPHIC_FAMILY)?;
        }
        if face.root.family_name.is_none() {
            face.root.family_name = tt_face_get_name(face, stream, TT_NAME_ID_FONT_FAMILY)?;
        }

        if !ignore_typographic_subfamily {
            face.root.style_name =
                tt_face_get_name(face, stream, TT_NAME_ID_TYPOGRAPHIC_SUBFAMILY)?;
        }
        if face.root.style_name.is_none() {
            face.root.style_name = tt_face_get_name(face, stream, TT_NAME_ID_FONT_SUBFAMILY)?;
        }
    } else {
        face.root.family_name = tt_face_get_name(face, stream, TT_NAME_ID_WWS_FAMILY)?;
        if face.root.family_name.is_none() && !ignore_typographic_family {
            face.root.family_name = tt_face_get_name(face, stream, TT_NAME_ID_TYPOGRAPHIC_FAMILY)?;
        }
        if face.root.family_name.is_none() {
            face.root.family_name = tt_face_get_name(face, stream, TT_NAME_ID_FONT_FAMILY)?;
        }

        face.root.style_name = tt_face_get_name(face, stream, TT_NAME_ID_WWS_SUBFAMILY)?;
        if face.root.style_name.is_none() && !ignore_typographic_subfamily {
            face.root.style_name =
                tt_face_get_name(face, stream, TT_NAME_ID_TYPOGRAPHIC_SUBFAMILY)?;
        }
        if face.root.style_name.is_none() {
            face.root.style_name = tt_face_get_name(face, stream, TT_NAME_ID_FONT_SUBFAMILY)?;
        }
    }

    /* TT_CONFIG_OPTION_GX_VAR_SUPPORT */
    face.non_var_style_name = face.root.style_name.clone();

    /* now set up root fields */
    {
        let mut flags = face.root.face_flags;

        /*
         * Compute face flags.
         */
        if face.sbit_table_type == TT_SBIT_TABLE_TYPE_CBLC
            || face.sbit_table_type == TT_SBIT_TABLE_TYPE_SBIX
            || face.colr.is_some()
            || face.svg.is_some()
        {
            flags |= FT_FACE_FLAG_COLOR; /* color glyphs */
        }

        if has_outline {
            /* by default (and for backward compatibility) we handle */
            /* fonts with an 'sbix' table as bitmap-only             */
            if has_sbix {
                flags |= FT_FACE_FLAG_SBIX; /* with 'sbix' bitmaps */
            } else {
                flags |= FT_FACE_FLAG_SCALABLE; /* scalable outlines */
            }
        }

        /* The sfnt driver only supports bitmap fonts natively, thus we */
        /* don't set FT_FACE_FLAG_HINTER.                               */
        flags |= FT_FACE_FLAG_SFNT |  /* SFNT file format  */
                 FT_FACE_FLAG_HORIZONTAL; /* horizontal data   */

        /* TT_CONFIG_OPTION_POSTSCRIPT_NAMES */
        if psnames_error.is_none() && face.postscript.FormatType != 0x00030000 {
            flags |= FT_FACE_FLAG_GLYPH_NAMES;
        }

        /* fixed width font? */
        if face.postscript.isFixedPitch != 0 {
            flags |= FT_FACE_FLAG_FIXED_WIDTH;
        }

        /* vertical information? */
        if face.vertical_info {
            flags |= FT_FACE_FLAG_VERTICAL;
        }

        /* kerning available ? */
        if face.kern_avail_bits != 0 {
            flags |= FT_FACE_FLAG_KERNING;
        }

        /* TT_CONFIG_OPTION_GX_VAR_SUPPORT */
        /* Don't bother to load the tables unless somebody asks for them. */
        /* No need to do work which will (probably) not be used.          */
        if face.variation_support & TT_FACE_FLAG_VAR_FVAR != 0 {
            flags |= FT_FACE_FLAG_MULTIPLE_MASTERS;
        }

        face.root.face_flags = flags;

        /*
         * Compute style flags.
         */

        let mut flags: FtLong = 0;
        if has_outline && face.os2.version != 0xFFFF {
            /* We have an OS/2 table; use the `fsSelection' field.  Bit 9 */
            /* indicates an oblique font face.  This flag has been        */
            /* introduced in version 1.5 of the OpenType specification.   */

            if face.os2.fsSelection & 512 != 0 {
                /* bit 9 */
                flags |= FT_STYLE_FLAG_ITALIC;
            } else if face.os2.fsSelection & 1 != 0 {
                /* bit 0 */
                flags |= FT_STYLE_FLAG_ITALIC;
            }

            if face.os2.fsSelection & 32 != 0 {
                /* bit 5 */
                flags |= FT_STYLE_FLAG_BOLD;
            }
        } else {
            /* this is an old Mac font, use the header field */

            if face.header.Mac_Style & 1 != 0 {
                flags |= FT_STYLE_FLAG_BOLD;
            }

            if face.header.Mac_Style & 2 != 0 {
                flags |= FT_STYLE_FLAG_ITALIC;
            }
        }

        face.root.style_flags |= flags;

        /*
         * Polish the charmaps.
         *
         *   Try to set the charmap encoding according to the platform &
         *   encoding ID of each charmap.  Emulate Unicode charmap if one
         *   is missing.
         */

        let _ = tt_face_build_cmaps(face); /* ignore errors */

        /* set the encoding fields */
        {
            let mut has_unicode = false;

            for charmap in face.root.charmaps.iter_mut() {
                charmap.charmap.encoding = sfnt_find_encoding(
                    charmap.charmap.platform_id as FtInt,
                    charmap.charmap.encoding_id as FtInt,
                );

                if charmap.charmap.encoding == FT_ENCODING_UNICODE
                    || charmap.charmap.encoding == FT_ENCODING_MS_SYMBOL
                {
                    /* PUA */
                    has_unicode = true;
                }
            }

            /* synthesize Unicode charmap if one is missing */
            if !has_unicode && face.root.face_flags & FT_FACE_FLAG_GLYPH_NAMES != 0 {
                let cmaprec = FtCharMapRec {
                    platform_id: TT_PLATFORM_MICROSOFT,
                    encoding_id: TT_MS_ID_UNICODE_CS,
                    encoding: FT_ENCODING_UNICODE,
                };

                let error = tt_cmap_unicode_init(face, stream).and_then(|unicodes| {
                    ft_cmap_new(
                        &mut face.root,
                        &TT_CMAP_UNICODE_CLASS_REC,
                        FtCMapData::PsUnicodes(unicodes),
                        cmaprec,
                    )
                });
                if let Err(e) = error {
                    if ft_err_neq(e, FT_ERR_NO_UNICODE_GLYPH_NAME)
                        && ft_err_neq(e, FT_ERR_UNIMPLEMENTED_FEATURE)
                    {
                        return Err(e);
                    }
                }
            }
        }

        /* TT_CONFIG_OPTION_EMBEDDED_BITMAPS */
        /*
         * Now allocate the root array of FT_Bitmap_Size records and
         * populate them.  Unfortunately, it isn't possible to indicate bit
         * depths in the FT_Bitmap_Size record.  This is a design error.
         */
        {
            let count = face.sbit_num_strikes;

            if count > 0 {
                let mut em_size = face.header.Units_Per_EM as FtInt;
                let mut avgwidth = face.os2.xAvgCharWidth as FtInt;
                let mut metrics = FtSizeMetrics::default();

                if em_size == 0 || face.os2.version == 0xFFFF {
                    avgwidth = 1;
                    em_size = 1;
                }

                /* to avoid invalid strike data in the `available_sizes' field */
                /* of `FT_Face', we map `available_sizes' indices to strike    */
                /* indices                                                     */
                face.root.available_sizes =
                    super::super::base::ftmemory::ft_new_array(count as FtLong)?;
                let mut sbit_strike_map: Vec<FtUInt> =
                    super::super::base::ftmemory::ft_new_array(count as FtLong)?;

                let mut bsize_idx = 0usize;
                for strike_idx in 0..count {
                    if tt_face_load_strike_metrics(
                        face,
                        stream,
                        strike_idx as FtULong,
                        &mut metrics,
                    )
                    .is_err()
                    {
                        continue;
                    }

                    let bsize = &mut face.root.available_sizes[bsize_idx];

                    bsize.height = (metrics.height >> 6) as FtShort;
                    bsize.width =
                        ((avgwidth * metrics.x_ppem as FtInt + em_size / 2) / em_size) as FtShort;

                    bsize.x_ppem = (metrics.x_ppem as FtPos) << 6;
                    bsize.y_ppem = (metrics.y_ppem as FtPos) << 6;

                    /* assume 72dpi */
                    bsize.size = (metrics.y_ppem as FtPos) << 6;

                    /* only use strikes with valid PPEM values */
                    if bsize.x_ppem != 0 && bsize.y_ppem != 0 {
                        sbit_strike_map[bsize_idx] = strike_idx;
                        bsize_idx += 1;
                    }
                }

                /* reduce array size to the actually used elements */
                sbit_strike_map.truncate(bsize_idx);

                /* from now on, all strike indices are mapped */
                /* using `sbit_strike_map'                    */
                if bsize_idx != 0 {
                    face.sbit_strike_map = sbit_strike_map;

                    face.root.face_flags |= FT_FACE_FLAG_FIXED_SIZES;
                    face.root.num_fixed_sizes = bsize_idx as FtInt;
                }
            }
        }

        /* a font with no bitmaps and no outlines is scalable; */
        /* it has only empty glyphs then                       */
        if !ft_has_fixed_sizes(&face.root) && !ft_is_scalable(&face.root) {
            face.root.face_flags |= FT_FACE_FLAG_SCALABLE;
        }

        /*
         * Set up metrics.
         */
        if ft_is_scalable(&face.root) || ft_has_sbix(&face.root) {
            let root = &mut face.root;

            /* XXX What about if outline header is missing */
            /*     (e.g. sfnt wrapped bitmap)?             */
            root.bbox.xMin = face.header.xMin as FtPos;
            root.bbox.yMin = face.header.yMin as FtPos;
            root.bbox.xMax = face.header.xMax as FtPos;
            root.bbox.yMax = face.header.yMax as FtPos;
            root.units_per_EM = face.header.Units_Per_EM;

            /*
             * Computing the ascender/descender/height is tricky.
             *
             * The OpenType specification v1.8.3 says:
             *
             *   [OS/2's] sTypoAscender, sTypoDescender and sTypoLineGap fields
             *   are intended to allow applications to lay out documents in a
             *   typographically-correct and portable fashion.
             *
             * This is somewhat at odds with the decades of backwards
             * compatibility, operating systems and applications doing whatever
             * they want, not to mention broken fonts.
             *
             * Not all fonts have an OS/2 table; in this case, we take the values
             * in the horizontal header, although there is nothing stopping the
             * values from being unreliable. Even with a OS/2 table, certain fonts
             * set the sTypoAscender, sTypoDescender and sTypoLineGap fields to 0
             * and instead correctly set usWinAscent and usWinDescent.
             *
             * As an example, Arial Narrow is shipped as four files ARIALN.TTF,
             * ARIALNI.TTF, ARIALNB.TTF and ARIALNBI.TTF. Strangely, all fonts have
             * the same values in their sTypo* fields, except ARIALNB.ttf which
             * sets them to 0. All of them have different usWinAscent/Descent
             * values. The OS/2 table therefore cannot be trusted for computing the
             * text height reliably.
             *
             * As a compromise, do the following:
             *
             * 1. If the OS/2 table exists and the fsSelection bit 7 is set
             *    (USE_TYPO_METRICS), trust the font and use the sTypo* metrics.
             * 2. Otherwise, use the `hhea' table's metrics.
             * 3. If they are zero and the OS/2 table exists,
             *    1. use the OS/2 table's sTypo* metrics if they are non-zero.
             *    2. Otherwise, use the OS/2 table's usWin* metrics.
             */

            /* (the sums are computed in `int' and truncated to FT_Short) */
            if face.os2.version != 0xFFFF && face.os2.fsSelection & 128 != 0 {
                root.ascender = face.os2.sTypoAscender;
                root.descender = face.os2.sTypoDescender;
                root.height = (root.ascender as FtInt - root.descender as FtInt
                    + face.os2.sTypoLineGap as FtInt) as FtShort;
            } else {
                root.ascender = face.horizontal.Ascender;
                root.descender = face.horizontal.Descender;
                root.height = (root.ascender as FtInt - root.descender as FtInt
                    + face.horizontal.Line_Gap as FtInt) as FtShort;

                if !(root.ascender != 0 || root.descender != 0) && face.os2.version != 0xFFFF {
                    if face.os2.sTypoAscender != 0 || face.os2.sTypoDescender != 0 {
                        root.ascender = face.os2.sTypoAscender;
                        root.descender = face.os2.sTypoDescender;
                        root.height = (root.ascender as FtInt - root.descender as FtInt
                            + face.os2.sTypoLineGap as FtInt)
                            as FtShort;
                    } else {
                        root.ascender = face.os2.usWinAscent as FtShort;
                        root.descender = (face.os2.usWinDescent as FtShort).wrapping_neg();
                        root.height = (root.ascender as FtInt - root.descender as FtInt) as FtShort;
                    }
                }
            }

            root.max_advance_width = face.horizontal.advance_Width_Max as FtShort;
            root.max_advance_height = if face.vertical_info {
                face.vertical.advance_Height_Max as FtShort
            } else {
                root.height
            };

            /* See https://www.microsoft.com/typography/otspec/post.htm -- */
            /* Adjust underline position from top edge to centre of        */
            /* stroke to convert TrueType meaning to FreeType meaning.     */
            root.underline_position = (face.postscript.underlinePosition as FtInt
                - face.postscript.underlineThickness as FtInt / 2)
                as FtShort;
            root.underline_thickness = face.postscript.underlineThickness;
        }
    }

    /* Exit: */
    Ok(())
}

/// `sfnt_done_face`
pub fn sfnt_done_face(face: &mut TtFaceRec) {
    /* destroy the postscript names table if it is loaded */
    tt_face_free_ps_names(face);

    /* destroy the embedded bitmaps table if it is loaded */
    tt_face_free_sbit(face);

    /* destroy color table data if it is loaded */
    tt_face_free_cpal(face);
    tt_face_free_colr(face);

    /* free SVG data */
    tt_face_free_svg(face);

    /* freeing the embedded BDF properties */
    tt_face_free_bdf_props(face);

    /* freeing the kerning table */
    tt_face_done_kern(face);

    /* freeing the collection table */
    face.ttc_header.offsets = Vec::new();
    face.ttc_header.count = 0;

    /* freeing table directory */
    face.dir_tables = Vec::new();
    face.num_tables = 0;

    /* simply release the 'cmap' table frame */
    face.cmap_table = None;
    face.cmap_size = 0;

    face.horz_metrics_size = 0;
    face.vert_metrics_size = 0;

    /* freeing vertical metrics, if any */
    if face.vertical_info {
        face.vertical_info = false;
    }

    /* freeing the gasp table */
    face.gasp.gaspRanges = Vec::new();
    face.gasp.numRanges = 0;

    /* freeing the name table */
    tt_face_free_name(face);

    /* freeing family and style name */
    face.root.family_name = None;
    face.root.style_name = None;

    /* freeing sbit size table */
    face.root.available_sizes = Vec::new();
    face.sbit_strike_map = Vec::new();
    face.root.num_fixed_sizes = 0;

    face.postscript_name = None;

    /* TT_CONFIG_OPTION_GX_VAR_SUPPORT */
    face.var_postscript_prefix = None;
    face.non_var_style_name = None;

    /* freeing glyph color palette data */
    face.palette_data = FtPaletteData::default();
    face.palette = Vec::new();
}
