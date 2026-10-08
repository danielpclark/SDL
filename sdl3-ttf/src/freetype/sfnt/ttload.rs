// Rust translation of src/sfnt/ttload.c from FreeType (2.13.2, as SDL_ttf's
// external/freetype pins it).
// Copyright (C) 1996-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! Load the basic TrueType tables, i.e., tables that can be either in
//! TTF or OTF fonts (body).
//!
//! As in C, the loaders take the face and its stream (`face->root.stream`)
//! separately; see [`with_stream`].

use std::sync::Arc;

use super::super::base::ftmemory::ft_new_array;
use super::super::base::ftstream::FtStreamRec;
use super::super::fttypes::*;
use super::super::tttables::*;
use super::super::tttypes::*;

/// Runs `f` on the face and its stream, taken out of the face for the
/// call (C passes `face->root.stream` alongside the face).
pub fn with_stream<R>(
    face: &mut TtFaceRec,
    f: impl FnOnce(&mut TtFaceRec, &mut FtStreamRec) -> R,
) -> R {
    let mut stream = face.root.stream.take().expect("face without a stream");
    let r = f(face, &mut stream);
    if face.root.stream.is_none() {
        face.root.stream = Some(stream);
    }
    r
}

/// `tt_face_lookup_table`: Looks for a TrueType table by name; returns the
/// table directory entry, if any.
pub fn tt_face_lookup_table(face: &TtFaceRec, tag: FtULong) -> Option<TtTableRec> {
    for entry in &face.dir_tables[..face.num_tables as usize] {
        /* For compatibility with Windows, we consider    */
        /* zero-length tables the same as missing tables. */
        if entry.Tag == tag && entry.Length != 0 {
            return Some(*entry);
        }
    }

    None
}

/// `tt_face_goto_table`: Looks for a TrueType table by name, then seek a
/// stream to it; returns the length of the table.
pub fn tt_face_goto_table(
    face: &TtFaceRec,
    tag: FtULong,
    stream: &mut FtStreamRec,
) -> FtResult<FtULong> {
    match tt_face_lookup_table(face, tag) {
        Some(table) => {
            stream.seek(table.Offset)?;
            Ok(table.Length)
        }
        None => Err(FT_ERR_TABLE_MISSING),
    }
}

/* Here, we                                                         */
/*                                                                  */
/* - check that `num_tables' is valid (and adjust it if necessary); */
/*   also return the number of valid table entries                  */
/*                                                                  */
/* - look for a `head' table, check its size, and parse it to check */
/*   whether its `magic' field is correctly set                     */
/*                                                                  */
/* - errors (except errors returned by stream handling)             */
/*                                                                  */
/*     SFNT_Err_Unknown_File_Format:                                */
/*       no table is defined in directory, it is not sfnt-wrapped   */
/*       data                                                       */
/*     SFNT_Err_Table_Missing:                                      */
/*       table directory is valid, but essential tables             */
/*       (head/bhed/SING) are missing                               */
/*                                                                  */
fn check_table_dir(sfnt: &mut SfntHeaderRec, stream: &mut FtStreamRec) -> FtResult<FtUShort> {
    let mut valid_entries: FtUShort = 0;
    let mut has_head = false;
    let mut has_sing = false;
    let mut has_meta = false;
    let offset = sfnt.offset + 12;

    stream.seek(offset)?;

    let mut nn: FtUShort = 0;
    while nn < sfnt.num_tables {
        if stream.enter_frame(16).is_err() {
            sfnt.num_tables = nn;
            break;
        }
        let table = TtTableRec {
            Tag: stream.get_ulong() as FtULong,
            CheckSum: stream.get_ulong() as FtULong,
            Offset: stream.get_ulong() as FtULong,
            Length: stream.get_ulong() as FtULong,
        };
        stream.exit_frame();

        /* we ignore invalid tables */

        if table.Offset > stream.size {
            nn += 1;
            continue;
        } else if table.Length > stream.size - table.Offset {
            /* Some tables have such a simple structure that clipping its     */
            /* contents is harmless.  This also makes FreeType less sensitive */
            /* to invalid table lengths (which programs like Acroread seem to */
            /* ignore in general).                                            */

            if table.Tag == TTAG_hmtx as FtULong || table.Tag == TTAG_vmtx as FtULong {
                valid_entries += 1;
            } else {
                nn += 1;
                continue;
            }
        } else {
            valid_entries += 1;
        }

        if table.Tag == TTAG_head as FtULong || table.Tag == TTAG_bhed as FtULong {
            has_head = true;

            /*
             * The table length should be 0x36, but certain font tools make it
             * 0x38, so we will just check that it is greater.
             *
             * Note that according to the specification, the table must be
             * padded to 32-bit lengths, but this doesn't apply to the value of
             * its `Length' field!
             *
             */
            if table.Length < 0x36 {
                return Err(FT_ERR_TABLE_MISSING);
            }

            stream.seek(table.Offset + 12)?;
            let _magic = stream.read_ulong()?;

            stream.seek(offset + (nn as FtULong + 1) * 16)?;
        } else if table.Tag == TTAG_SING as FtULong {
            has_sing = true;
        } else if table.Tag == TTAG_META as FtULong {
            has_meta = true;
        }

        nn += 1;
    }

    if valid_entries == 0 {
        return Err(FT_ERR_UNKNOWN_FILE_FORMAT);
    }

    /* if `sing' and `meta' tables are present, there is no `head' table */
    if has_head || (has_sing && has_meta) {
        Ok(valid_entries)
    } else {
        Err(FT_ERR_TABLE_MISSING)
    }
}

/// `tt_face_load_font_dir`: Loads the header of a SFNT font file.
///
/// The stream cursor must be at the beginning of the font directory.
pub fn tt_face_load_font_dir(face: &mut TtFaceRec, stream: &mut FtStreamRec) -> FtResult<()> {
    let mut sfnt = SfntHeaderRec::default();
    let mut valid_entries: FtUShort;

    /* read the offset table */

    sfnt.offset = stream.pos();

    sfnt.format_tag = stream.read_ulong()? as FtULong;
    stream.enter_frame(8)?;
    sfnt.num_tables = stream.get_ushort();
    sfnt.search_range = stream.get_ushort();
    sfnt.entry_selector = stream.get_ushort();
    sfnt.range_shift = stream.get_ushort();
    stream.exit_frame();

    /* many fonts don't have these fields set correctly */

    /* load the table directory */

    if sfnt.format_tag != TTAG_OTTO as FtULong {
        /* check first */
        valid_entries = check_table_dir(&mut sfnt, stream)?;
    } else {
        valid_entries = sfnt.num_tables;
        if valid_entries == 0 {
            return Err(FT_ERR_UNKNOWN_FILE_FORMAT);
        }
    }

    face.num_tables = valid_entries;
    face.format_tag = sfnt.format_tag;

    face.dir_tables = ft_new_array(face.num_tables as FtLong)?;

    stream.seek(sfnt.offset + 12)?;
    stream.enter_frame(sfnt.num_tables as FtULong * 16)?;

    valid_entries = 0;
    for _ in 0..sfnt.num_tables {
        let mut entry = TtTableRec {
            Tag: stream.get_ulong() as FtULong,
            CheckSum: stream.get_ulong() as FtULong,
            Offset: stream.get_ulong() as FtULong,
            Length: stream.get_ulong() as FtULong,
        };

        /* ignore invalid tables that can't be sanitized */

        if entry.Offset > stream.size {
            continue;
        } else if entry.Length > stream.size - entry.Offset {
            if entry.Tag == TTAG_hmtx as FtULong || entry.Tag == TTAG_vmtx as FtULong {
                /* make metrics table length a multiple of 4 */
                entry.Length = (stream.size - entry.Offset) & !3;
            } else {
                continue;
            }
        }

        /* ignore duplicate tables -- the first one wins */
        let duplicate = face.dir_tables[..valid_entries as usize]
            .iter()
            .any(|t| t.Tag == entry.Tag);
        if duplicate {
            continue;
        } else {
            /* we finally have a valid entry */
            face.dir_tables[valid_entries as usize] = entry;
            valid_entries += 1;
        }
    }

    /* final adjustment to number of tables */
    face.num_tables = valid_entries;

    stream.exit_frame();

    if valid_entries == 0 {
        return Err(FT_ERR_UNKNOWN_FILE_FORMAT);
    }

    Ok(())
}

/// `tt_face_load_any`: Loads any font table into client memory.
///
/// `tag` 0 means the whole font file; with `*length == 0` the function
/// only returns the length of the table or file.
pub fn tt_face_load_any(
    face: &TtFaceRec,
    stream: &mut FtStreamRec,
    tag: FtULong,
    mut offset: FtLong,
    buffer: Option<&mut [u8]>,
    length: Option<&mut FtULong>,
) -> FtResult<()> {
    let mut size: FtULong;

    if tag != 0 {
        /* look for tag in font directory */
        let table = match tt_face_lookup_table(face, tag) {
            Some(t) => t,
            None => return Err(FT_ERR_TABLE_MISSING),
        };

        offset = offset.wrapping_add(table.Offset as FtLong);
        size = table.Length;
    } else {
        /* tag == 0 -- the user wants to access the font file directly */
        size = stream.size;
    }

    if let Some(length) = length {
        if *length == 0 {
            *length = size;

            return Ok(());
        }
        size = *length;
    }

    let buffer = buffer.ok_or(FT_ERR_INVALID_ARGUMENT)?;
    let size = (size as usize).min(buffer.len());
    stream.read_at(offset as FtULong, &mut buffer[..size])
}

/// `tt_face_load_generic_header`: Loads the TrueType table `head' or
/// `bhed'.
fn tt_face_load_generic_header(
    face: &mut TtFaceRec,
    stream: &mut FtStreamRec,
    tag: FtULong,
) -> FtResult<()> {
    tt_face_goto_table(face, tag, stream)?;

    let header = &mut face.header;

    stream.enter_frame(54)?;
    header.Table_Version = stream.get_ulong() as FtFixed;
    header.Font_Revision = stream.get_ulong() as FtFixed;
    header.CheckSum_Adjust = stream.get_long() as FtLong;
    header.Magic_Number = stream.get_long() as FtLong;
    header.Flags = stream.get_ushort();
    header.Units_Per_EM = stream.get_ushort();
    header.Created[0] = stream.get_ulong() as FtULong;
    header.Created[1] = stream.get_ulong() as FtULong;
    header.Modified[0] = stream.get_ulong() as FtULong;
    header.Modified[1] = stream.get_ulong() as FtULong;
    header.xMin = stream.get_short();
    header.yMin = stream.get_short();
    header.xMax = stream.get_short();
    header.yMax = stream.get_short();
    header.Mac_Style = stream.get_ushort();
    header.Lowest_Rec_PPEM = stream.get_ushort();
    header.Font_Direction = stream.get_short();
    header.Index_To_Loc_Format = stream.get_short();
    header.Glyph_Data_Format = stream.get_short();
    stream.exit_frame();

    Ok(())
}

/// `tt_face_load_head`
pub fn tt_face_load_head(face: &mut TtFaceRec, stream: &mut FtStreamRec) -> FtResult<()> {
    tt_face_load_generic_header(face, stream, TTAG_head as FtULong)
}

/// `tt_face_load_bhed`
pub fn tt_face_load_bhed(face: &mut TtFaceRec, stream: &mut FtStreamRec) -> FtResult<()> {
    tt_face_load_generic_header(face, stream, TTAG_bhed as FtULong)
}

/// `tt_face_load_maxp`: Loads the maximum profile into a face object.
pub fn tt_face_load_maxp(face: &mut TtFaceRec, stream: &mut FtStreamRec) -> FtResult<()> {
    tt_face_goto_table(face, TTAG_maxp as FtULong, stream)?;

    let max_profile = &mut face.max_profile;

    stream.enter_frame(6)?;
    max_profile.version = stream.get_long() as FtFixed;
    max_profile.numGlyphs = stream.get_ushort();
    stream.exit_frame();

    max_profile.maxPoints = 0;
    max_profile.maxContours = 0;
    max_profile.maxCompositePoints = 0;
    max_profile.maxCompositeContours = 0;
    max_profile.maxZones = 0;
    max_profile.maxTwilightPoints = 0;
    max_profile.maxStorage = 0;
    max_profile.maxFunctionDefs = 0;
    max_profile.maxInstructionDefs = 0;
    max_profile.maxStackElements = 0;
    max_profile.maxSizeOfInstructions = 0;
    max_profile.maxComponentElements = 0;
    max_profile.maxComponentDepth = 0;

    if max_profile.version >= 0x10000 {
        stream.enter_frame(26)?;
        max_profile.maxPoints = stream.get_ushort();
        max_profile.maxContours = stream.get_ushort();
        max_profile.maxCompositePoints = stream.get_ushort();
        max_profile.maxCompositeContours = stream.get_ushort();
        max_profile.maxZones = stream.get_ushort();
        max_profile.maxTwilightPoints = stream.get_ushort();
        max_profile.maxStorage = stream.get_ushort();
        max_profile.maxFunctionDefs = stream.get_ushort();
        max_profile.maxInstructionDefs = stream.get_ushort();
        max_profile.maxStackElements = stream.get_ushort();
        max_profile.maxSizeOfInstructions = stream.get_ushort();
        max_profile.maxComponentElements = stream.get_ushort();
        max_profile.maxComponentDepth = stream.get_ushort();
        stream.exit_frame();

        /* XXX: an adjustment that is necessary to load certain */
        /*      broken fonts like `Keystrokes MT' :-(           */
        /*                                                      */
        /*   We allocate 64 function entries by default when    */
        /*   the maxFunctionDefs value is smaller.              */

        if max_profile.maxFunctionDefs < 64 {
            max_profile.maxFunctionDefs = 64;
        }

        /* we add 4 phantom points later */
        if max_profile.maxTwilightPoints > (0xFFFF - 4) {
            max_profile.maxTwilightPoints = 0xFFFF - 4;
        }
    }

    Ok(())
}

/// `tt_face_load_name`: Loads the name records.
pub fn tt_face_load_name(face: &mut TtFaceRec, stream: &mut FtStreamRec) -> FtResult<()> {
    let table_len = tt_face_goto_table(face, TTAG_name as FtULong, stream)?;

    let table = &mut face.name_table;

    let table_pos = stream.pos();

    stream.enter_frame(6)?;
    table.format = stream.get_ushort();
    table.numNameRecords = stream.get_ushort() as FtUInt;
    table.storageOffset = stream.get_ushort() as FtUInt;
    stream.exit_frame();

    /* Some popular Asian fonts have an invalid `storageOffset' value (it */
    /* should be at least `6 + 12*numNameRecords').  However, the string  */
    /* offsets, computed as `storageOffset + entry->stringOffset', are    */
    /* valid pointers within the name table...                            */
    /*                                                                    */
    /* We thus can't check `storageOffset' right now.                     */
    /*                                                                    */
    let mut storage_start = table_pos + 6 + 12 * table.numNameRecords as FtULong;
    let storage_limit = table_pos + table_len;

    if storage_start > storage_limit {
        return Err(FT_ERR_NAME_TABLE_MISSING);
    }

    /* `name' format 1 contains additional language tag records, */
    /* which we load first                                       */
    if table.format == 1 {
        stream.seek(storage_start)?;
        table.numLangTagRecords = stream.read_ushort()? as FtUInt;

        storage_start += 2 + 4 * table.numLangTagRecords as FtULong;

        /* allocate language tag records array */
        let mut lang_tags: Vec<TtLangTagRec> = ft_new_array(table.numLangTagRecords as FtLong)?;
        stream.enter_frame(table.numLangTagRecords as FtULong * 4)?;

        /* load language tags */
        for entry in lang_tags.iter_mut() {
            entry.stringLength = stream.get_ushort();
            entry.stringOffset = stream.get_ushort() as FtULong;

            /* check that the langTag string is within the table */
            entry.stringOffset += table_pos + table.storageOffset as FtULong;
            if entry.stringOffset < storage_start
                || entry.stringOffset + entry.stringLength as FtULong > storage_limit
            {
                /* invalid entry; ignore it */
                entry.stringLength = 0;
            }

            /* mark the string as not yet loaded */
            entry.string = None;
        }

        table.langTags = lang_tags;

        stream.exit_frame();

        let _ = stream.seek(table_pos + 6);
    }

    /* allocate name records array */
    let mut names: Vec<TtNameRec> = ft_new_array(table.numNameRecords as FtLong)?;
    stream.enter_frame(table.numNameRecords as FtULong * 12)?;

    /* load name records */
    {
        let mut valid = 0;

        for _ in 0..table.numNameRecords {
            let entry = &mut names[valid];
            entry.platformID = stream.get_ushort();
            entry.encodingID = stream.get_ushort();
            entry.languageID = stream.get_ushort();
            entry.nameID = stream.get_ushort();
            entry.stringLength = stream.get_ushort();
            entry.stringOffset = stream.get_ushort() as FtULong;

            /* check that the name is not empty */
            if entry.stringLength == 0 {
                continue;
            }

            /* check that the name string is within the table */
            entry.stringOffset += table_pos + table.storageOffset as FtULong;
            if entry.stringOffset < storage_start
                || entry.stringOffset + entry.stringLength as FtULong > storage_limit
            {
                /* invalid entry; ignore it */
                continue;
            }

            /* assure that we have a valid language tag ID, and   */
            /* that the corresponding langTag entry is valid, too */
            if table.format == 1 && entry.languageID >= 0x8000 {
                let li = (entry.languageID - 0x8000) as usize;
                if li as FtUInt >= table.numLangTagRecords || table.langTags[li].stringLength == 0 {
                    /* invalid entry; ignore it */
                    continue;
                }
            }

            /* mark the string as not yet converted */
            entry.string = None;

            valid += 1;
        }

        /* reduce array size to the actually used elements */
        names.truncate(valid);
        table.names = names;
        table.numNameRecords = valid as FtUInt;
    }

    stream.exit_frame();

    /* everything went well, update face->num_names */
    face.num_names = table.numNameRecords as FtUShort;

    Ok(())
}

/// `tt_face_free_name`: Frees the name records.
pub fn tt_face_free_name(face: &mut TtFaceRec) {
    let table = &mut face.name_table;

    table.names = Vec::new();
    table.langTags = Vec::new();

    table.numNameRecords = 0;
    table.numLangTagRecords = 0;
    table.format = 0;
    table.storageOffset = 0;
}

/// `tt_face_load_cmap`: Loads the cmap directory in a face object.  The
/// cmaps themselves are loaded on demand in the `ttcmap.c' module.
pub fn tt_face_load_cmap(face: &mut TtFaceRec, stream: &mut FtStreamRec) -> FtResult<()> {
    face.cmap_size = tt_face_goto_table(face, TTAG_cmap as FtULong, stream)?;

    match stream.extract_frame(face.cmap_size) {
        Ok(bytes) => face.cmap_table = Some(Arc::from(bytes)),
        Err(e) => {
            face.cmap_size = 0;
            return Err(e);
        }
    }

    Ok(())
}

/// `tt_face_load_os2`: Loads the OS2 table.
pub fn tt_face_load_os2(face: &mut TtFaceRec, stream: &mut FtStreamRec) -> FtResult<()> {
    /* We now support old Mac fonts where the OS/2 table doesn't  */
    /* exist.  Simply put, we set the `version' field to 0xFFFF   */
    /* and test this value each time we need to access the table. */
    tt_face_goto_table(face, TTAG_OS2 as FtULong, stream)?;

    let os2 = &mut face.os2;

    stream.enter_frame(78)?;
    os2.version = stream.get_ushort();
    os2.xAvgCharWidth = stream.get_short();
    os2.usWeightClass = stream.get_ushort();
    os2.usWidthClass = stream.get_ushort();
    os2.fsType = stream.get_short() as FtUShort;
    os2.ySubscriptXSize = stream.get_short();
    os2.ySubscriptYSize = stream.get_short();
    os2.ySubscriptXOffset = stream.get_short();
    os2.ySubscriptYOffset = stream.get_short();
    os2.ySuperscriptXSize = stream.get_short();
    os2.ySuperscriptYSize = stream.get_short();
    os2.ySuperscriptXOffset = stream.get_short();
    os2.ySuperscriptYOffset = stream.get_short();
    os2.yStrikeoutSize = stream.get_short();
    os2.yStrikeoutPosition = stream.get_short();
    os2.sFamilyClass = stream.get_short();
    for i in 0..10 {
        os2.panose[i] = stream.get_byte();
    }
    os2.ulUnicodeRange1 = stream.get_ulong() as FtULong;
    os2.ulUnicodeRange2 = stream.get_ulong() as FtULong;
    os2.ulUnicodeRange3 = stream.get_ulong() as FtULong;
    os2.ulUnicodeRange4 = stream.get_ulong() as FtULong;
    for i in 0..4 {
        os2.achVendID[i] = stream.get_byte() as FtChar;
    }
    os2.fsSelection = stream.get_ushort();
    os2.usFirstCharIndex = stream.get_ushort();
    os2.usLastCharIndex = stream.get_ushort();
    os2.sTypoAscender = stream.get_short();
    os2.sTypoDescender = stream.get_short();
    os2.sTypoLineGap = stream.get_short();
    os2.usWinAscent = stream.get_ushort();
    os2.usWinDescent = stream.get_ushort();
    stream.exit_frame();

    os2.ulCodePageRange1 = 0;
    os2.ulCodePageRange2 = 0;
    os2.sxHeight = 0;
    os2.sCapHeight = 0;
    os2.usDefaultChar = 0;
    os2.usBreakChar = 0;
    os2.usMaxContext = 0;
    os2.usLowerOpticalPointSize = 0;
    os2.usUpperOpticalPointSize = 0xFFFF;

    if os2.version >= 0x0001 {
        /* only version 1 tables */
        stream.enter_frame(8)?;
        os2.ulCodePageRange1 = stream.get_ulong() as FtULong;
        os2.ulCodePageRange2 = stream.get_ulong() as FtULong;
        stream.exit_frame();

        if os2.version >= 0x0002 {
            /* only version 2 tables */
            stream.enter_frame(10)?;
            os2.sxHeight = stream.get_short();
            os2.sCapHeight = stream.get_short();
            os2.usDefaultChar = stream.get_ushort();
            os2.usBreakChar = stream.get_ushort();
            os2.usMaxContext = stream.get_ushort();
            stream.exit_frame();

            if os2.version >= 0x0005 {
                /* only version 5 tables */
                stream.enter_frame(4)?;
                os2.usLowerOpticalPointSize = stream.get_ushort();
                os2.usUpperOpticalPointSize = stream.get_ushort();
                stream.exit_frame();
            }
        }
    }

    Ok(())
}

/// `tt_face_load_post`: Loads the Postscript table.
pub fn tt_face_load_post(face: &mut TtFaceRec, stream: &mut FtStreamRec) -> FtResult<()> {
    tt_face_goto_table(face, TTAG_post as FtULong, stream)?;

    let post = &mut face.postscript;

    stream.enter_frame(32)?;
    post.FormatType = stream.get_long() as FtFixed;
    post.italicAngle = stream.get_long() as FtFixed;
    post.underlinePosition = stream.get_short();
    post.underlineThickness = stream.get_short();
    post.isFixedPitch = stream.get_ulong() as FtULong;
    post.minMemType42 = stream.get_ulong() as FtULong;
    post.maxMemType42 = stream.get_ulong() as FtULong;
    post.minMemType1 = stream.get_ulong() as FtULong;
    post.maxMemType1 = stream.get_ulong() as FtULong;
    stream.exit_frame();

    if post.FormatType != 0x00030000
        && post.FormatType != 0x00025000
        && post.FormatType != 0x00020000
        && post.FormatType != 0x00010000
    {
        return Err(FT_ERR_INVALID_POST_TABLE_FORMAT);
    }

    /* we don't load the glyph names, we do that in another */
    /* module (ttpost).                                     */

    Ok(())
}

/// `tt_face_load_pclt`: Loads the PCL 5 Table.
pub fn tt_face_load_pclt(face: &mut TtFaceRec, stream: &mut FtStreamRec) -> FtResult<()> {
    /* optional table */
    tt_face_goto_table(face, TTAG_PCLT as FtULong, stream)?;

    let pclt = &mut face.pclt;

    stream.enter_frame(54)?;
    pclt.Version = stream.get_ulong() as FtFixed;
    pclt.FontNumber = stream.get_ulong() as FtULong;
    pclt.Pitch = stream.get_ushort();
    pclt.xHeight = stream.get_ushort();
    pclt.Style = stream.get_ushort();
    pclt.TypeFamily = stream.get_ushort();
    pclt.CapHeight = stream.get_ushort();
    pclt.SymbolSet = stream.get_ushort();
    let mut buf = [0u8; 16];
    let r = stream.get_bytes(&mut buf);
    if r.is_ok() {
        for i in 0..16 {
            pclt.TypeFace[i] = buf[i] as FtChar;
        }
    }
    let r = r.and_then(|_| {
        let mut buf = [0u8; 8];
        stream.get_bytes(&mut buf)?;
        for i in 0..8 {
            pclt.CharacterComplement[i] = buf[i] as FtChar;
        }
        let mut buf = [0u8; 6];
        stream.get_bytes(&mut buf)?;
        for i in 0..6 {
            pclt.FileName[i] = buf[i] as FtChar;
        }
        Ok(())
    });
    if r.is_ok() {
        pclt.StrokeWeight = stream.get_char();
        pclt.WidthType = stream.get_char();
        pclt.SerifStyle = stream.get_byte();
        pclt.Reserved = stream.get_byte();
    }
    stream.exit_frame();

    r
}

/// `tt_face_load_gasp`: Loads the `gasp' table into a face object.
pub fn tt_face_load_gasp(face: &mut TtFaceRec, stream: &mut FtStreamRec) -> FtResult<()> {
    /* the gasp table is optional */
    tt_face_goto_table(face, TTAG_gasp as FtULong, stream)?;

    stream.enter_frame(4)?;

    face.gasp.version = stream.get_ushort();
    let num_ranges = stream.get_ushort();

    stream.exit_frame();

    /* only support versions 0 and 1 of the table */
    if face.gasp.version >= 2 {
        face.gasp.numRanges = 0;
        return Err(FT_ERR_INVALID_TABLE);
    }

    let mut gasp_ranges: Vec<TtGaspRangeRec> = ft_new_array(num_ranges as FtLong)?;
    stream.enter_frame(num_ranges as FtULong * 4)?;

    for r in gasp_ranges.iter_mut() {
        r.maxPPEM = stream.get_ushort();
        r.gaspFlag = stream.get_ushort();
    }

    face.gasp.gaspRanges = gasp_ranges;
    face.gasp.numRanges = num_ranges;

    stream.exit_frame();

    Ok(())
}
