// Rust translation of src/winfonts/winfnt.c and src/winfonts/winfnt.h from
// FreeType (2.13.2, as SDL_ttf's external/freetype pins it).
// Copyright (C) 1996-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// Copyright 2003 Huw D M Davies for Codeweavers
// Copyright 2007 Dmitry Timoshkov for Codeweavers
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! FreeType font driver for Windows FNT/FON files.
//!
//! The `FT_FRAME_*` field descriptors become the reads they describe; the
//! font's frame (`fnt_frame`) is an owned copy of the FNT data.

use super::super::base::ftcalc::ft_mul_div;
use super::super::base::ftmemory::ft_qalloc;
use super::super::base::ftobjs::*;
use super::super::base::ftstream::{ft_peek_ulong_le, ft_peek_ushort_le, FtStreamRec};
use super::super::base::ftwinfnt::*;
use super::super::fttypes::*;
use super::super::tttables::*;

/* winfnt.h */

/// `WinMZ_HeaderRec`
#[derive(Debug, Clone, Copy, Default)]
pub struct WinMzHeaderRec {
    pub magic: FtUShort,
    /* skipped content */
    pub lfanew: FtUShort,
}

/// `WinNE_HeaderRec`
#[derive(Debug, Clone, Copy, Default)]
pub struct WinNeHeaderRec {
    pub magic: FtUShort,
    /* skipped content */
    pub resource_tab_offset: FtUShort,
    pub rname_tab_offset: FtUShort,
}

/// `WinPE32_HeaderRec`
#[derive(Debug, Clone, Copy, Default)]
pub struct WinPe32HeaderRec {
    pub magic: FtULong,
    pub machine: FtUShort,
    pub number_of_sections: FtUShort,
    /* skipped content */
    pub size_of_optional_header: FtUShort,
    /* skipped content */
    pub magic32: FtUShort,
    /* skipped content */
    pub rsrc_virtual_address: FtULong,
    pub rsrc_size: FtULong,
    /* skipped content */
}

/// `WinPE32_SectionRec`
#[derive(Debug, Clone, Copy, Default)]
pub struct WinPe32SectionRec {
    pub name: [FtByte; 8],
    /* skipped content */
    pub virtual_address: FtULong,
    pub size_of_raw_data: FtULong,
    pub pointer_to_raw_data: FtULong,
    /* skipped content */
}

/// `WinPE_RsrcDirRec`
#[derive(Debug, Clone, Copy, Default)]
pub struct WinPeRsrcDirRec {
    pub characteristics: FtULong,
    pub time_date_stamp: FtULong,
    pub major_version: FtUShort,
    pub minor_version: FtUShort,
    pub number_of_named_entries: FtUShort,
    pub number_of_id_entries: FtUShort,
}

/// `WinPE_RsrcDirEntryRec`
#[derive(Debug, Clone, Copy, Default)]
pub struct WinPeRsrcDirEntryRec {
    pub name: FtULong,
    pub offset: FtULong,
}

/// `WinPE_RsrcDataEntryRec`
#[derive(Debug, Clone, Copy, Default)]
pub struct WinPeRsrcDataEntryRec {
    pub offset_to_data: FtULong,
    pub size: FtULong,
    pub code_page: FtULong,
    pub reserved: FtULong,
}

/// `WinNameInfoRec`
#[derive(Debug, Clone, Copy, Default)]
pub struct WinNameInfoRec {
    pub offset: FtUShort,
    pub length: FtUShort,
    pub flags: FtUShort,
    pub id: FtUShort,
    pub handle: FtUShort,
    pub usage: FtUShort,
}

/// `WinResourceInfoRec`
#[derive(Debug, Clone, Copy, Default)]
pub struct WinResourceInfoRec {
    pub type_id: FtUShort,
    pub count: FtUShort,
}

pub const WINFNT_MZ_MAGIC: FtUShort = 0x5A4D;
pub const WINFNT_NE_MAGIC: FtUShort = 0x454E;
pub const WINFNT_PE_MAGIC: FtUShort = 0x4550;

/// `FNT_FontRec`
#[derive(Debug, Clone, Default)]
pub struct FntFontRec {
    pub offset: FtULong,

    pub header: FtWinFntHeaderRec,

    pub fnt_frame: Vec<FtByte>,
    pub fnt_size: FtULong,
    pub family_name: Option<Vec<u8>>,
}

/// `FNT_FaceRec`
#[derive(Debug, Default)]
pub struct FntFaceRec {
    pub root: FtFaceRec,
    pub font: Option<Box<FntFontRec>>,
}

/* winfnt.c */

/// `winmz_header_fields`
fn read_winmz_header(stream: &mut FtStreamRec) -> FtResult<WinMzHeaderRec> {
    stream.enter_frame(64)?;
    let mut h = WinMzHeaderRec {
        magic: stream.get_ushort_le(),
        ..Default::default()
    };
    let _ = stream.skip_bytes(29 * 2);
    /* (`lfanew' is an FT_UShort read as an unsigned long: its low bits) */
    h.lfanew = stream.get_ulong_le() as FtUShort;
    stream.exit_frame();
    Ok(h)
}

/// `winne_header_fields`
fn read_winne_header(stream: &mut FtStreamRec) -> FtResult<WinNeHeaderRec> {
    stream.enter_frame(40)?;
    let mut h = WinNeHeaderRec {
        magic: stream.get_ushort_le(),
        ..Default::default()
    };
    let _ = stream.skip_bytes(34);
    h.resource_tab_offset = stream.get_ushort_le();
    h.rname_tab_offset = stream.get_ushort_le();
    stream.exit_frame();
    Ok(h)
}

/// `winpe32_header_fields`
fn read_winpe32_header(stream: &mut FtStreamRec) -> FtResult<WinPe32HeaderRec> {
    stream.enter_frame(248)?;
    let mut h = WinPe32HeaderRec {
        magic: stream.get_ulong_le() as FtULong, /* PE00 */
        machine: stream.get_ushort_le(),         /* 0x014C - i386 */
        number_of_sections: stream.get_ushort_le(),
        ..Default::default()
    };
    let _ = stream.skip_bytes(12);
    h.size_of_optional_header = stream.get_ushort_le();
    let _ = stream.skip_bytes(2);
    h.magic32 = stream.get_ushort_le(); /* 0x10B */
    let _ = stream.skip_bytes(110);
    h.rsrc_virtual_address = stream.get_ulong_le() as FtULong;
    h.rsrc_size = stream.get_ulong_le() as FtULong;
    let _ = stream.skip_bytes(104);
    stream.exit_frame();
    Ok(h)
}

/// `winpe32_section_fields`
fn read_winpe32_section(stream: &mut FtStreamRec) -> FtResult<WinPe32SectionRec> {
    stream.enter_frame(40)?;
    let mut s = WinPe32SectionRec::default();
    let _ = stream.get_bytes(&mut s.name);
    let _ = stream.skip_bytes(4);
    s.virtual_address = stream.get_ulong_le() as FtULong;
    s.size_of_raw_data = stream.get_ulong_le() as FtULong;
    s.pointer_to_raw_data = stream.get_ulong_le() as FtULong;
    let _ = stream.skip_bytes(16);
    stream.exit_frame();
    Ok(s)
}

/// `winpe_rsrc_dir_fields`
fn read_winpe_rsrc_dir(stream: &mut FtStreamRec) -> FtResult<WinPeRsrcDirRec> {
    stream.enter_frame(16)?;
    let d = WinPeRsrcDirRec {
        characteristics: stream.get_ulong_le() as FtULong,
        time_date_stamp: stream.get_ulong_le() as FtULong,
        major_version: stream.get_ushort_le(),
        minor_version: stream.get_ushort_le(),
        number_of_named_entries: stream.get_ushort_le(),
        number_of_id_entries: stream.get_ushort_le(),
    };
    stream.exit_frame();
    Ok(d)
}

/// `winpe_rsrc_dir_entry_fields`
fn read_winpe_rsrc_dir_entry(stream: &mut FtStreamRec) -> FtResult<WinPeRsrcDirEntryRec> {
    stream.enter_frame(8)?;
    let e = WinPeRsrcDirEntryRec {
        name: stream.get_ulong_le() as FtULong,
        offset: stream.get_ulong_le() as FtULong,
    };
    stream.exit_frame();
    Ok(e)
}

/// `winpe_rsrc_data_entry_fields`
fn read_winpe_rsrc_data_entry(stream: &mut FtStreamRec) -> FtResult<WinPeRsrcDataEntryRec> {
    stream.enter_frame(16)?;
    let e = WinPeRsrcDataEntryRec {
        offset_to_data: stream.get_ulong_le() as FtULong,
        size: stream.get_ulong_le() as FtULong,
        code_page: stream.get_ulong_le() as FtULong,
        reserved: stream.get_ulong_le() as FtULong,
    };
    stream.exit_frame();
    Ok(e)
}

/// `winfnt_header_fields`
fn read_winfnt_header(stream: &mut FtStreamRec, header: &mut FtWinFntHeaderRec) -> FtResult<()> {
    stream.enter_frame(148)?;
    header.version = stream.get_ushort_le();
    header.file_size = stream.get_ulong_le() as FtULong;
    let _ = stream.get_bytes(&mut header.copyright);
    header.file_type = stream.get_ushort_le();
    header.nominal_point_size = stream.get_ushort_le();
    header.vertical_resolution = stream.get_ushort_le();
    header.horizontal_resolution = stream.get_ushort_le();
    header.ascent = stream.get_ushort_le();
    header.internal_leading = stream.get_ushort_le();
    header.external_leading = stream.get_ushort_le();
    header.italic = stream.get_byte();
    header.underline = stream.get_byte();
    header.strike_out = stream.get_byte();
    header.weight = stream.get_ushort_le();
    header.charset = stream.get_byte();
    header.pixel_width = stream.get_ushort_le();
    header.pixel_height = stream.get_ushort_le();
    header.pitch_and_family = stream.get_byte();
    header.avg_width = stream.get_ushort_le();
    header.max_width = stream.get_ushort_le();
    header.first_char = stream.get_byte();
    header.last_char = stream.get_byte();
    header.default_char = stream.get_byte();
    header.break_char = stream.get_byte();
    header.bytes_per_row = stream.get_ushort_le();
    header.device_offset = stream.get_ulong_le() as FtULong;
    header.face_name_offset = stream.get_ulong_le() as FtULong;
    header.bits_pointer = stream.get_ulong_le() as FtULong;
    header.bits_offset = stream.get_ulong_le() as FtULong;
    header.reserved = stream.get_byte();
    header.flags = stream.get_ulong_le() as FtULong;
    header.A_space = stream.get_ushort_le();
    header.B_space = stream.get_ushort_le();
    header.C_space = stream.get_ushort_le();
    /* (the field is an FT_UShort read as an unsigned long: its low bits) */
    header.color_table_offset = stream.get_ulong_le() as FtUShort;
    /* (16 bytes copied over the first of the FT_ULong array's) */
    let mut reserved1 = [0u8; 16];
    let _ = stream.get_bytes(&mut reserved1);
    for (i, r) in header.reserved1.iter_mut().take(2).enumerate() {
        let mut b = [0u8; 8];
        b.copy_from_slice(&reserved1[i * 8..i * 8 + 8]);
        *r = FtULong::from_le_bytes(b);
    }
    stream.exit_frame();
    Ok(())
}

/// `fnt_font_done`
fn fnt_font_done(face: &mut FntFaceRec) {
    /* (the frame and the family name go with the font) */
    face.font = None;
}

/// `fnt_font_load`
fn fnt_font_load(font: &mut FntFontRec, stream: &mut FtStreamRec) -> FtResult<()> {
    let header = &mut font.header;

    /* first of all, read the FNT header */
    if stream.seek(font.offset).is_err() || read_winfnt_header(stream, header).is_err() {
        return Err(FT_ERR_UNKNOWN_FILE_FORMAT);
    }

    /* check header */
    if header.version != 0x200 && header.version != 0x300 {
        return Err(FT_ERR_UNKNOWN_FILE_FORMAT);
    }

    let new_format = font.header.version == 0x300;
    let size: FtULong = if new_format { 148 } else { 118 };

    let header = &mut font.header;
    if header.file_size < size {
        return Err(FT_ERR_UNKNOWN_FILE_FORMAT);
    }

    /* Version 2 doesn't have these fields */
    if header.version == 0x200 {
        header.flags = 0;
        header.A_space = 0;
        header.B_space = 0;
        header.C_space = 0;

        header.color_table_offset = 0;
    }

    if header.file_type & 1 != 0 {
        /* [can't handle vector FNT fonts] */
        return Err(FT_ERR_UNKNOWN_FILE_FORMAT);
    }

    /* this is a FNT file/table; extract its frame */
    stream.seek(font.offset)?;
    font.fnt_frame = stream.extract_frame(header.file_size)?;

    Ok(())
}

/// `fnt_face_get_dll_font`
fn fnt_face_get_dll_font(face: &mut FntFaceRec, face_instance_index: FtInt) -> FtResult<()> {
    let mut error: FtError;
    let Some(stream) = face.root.stream.as_mut() else {
        return Err(FT_ERR_INVALID_STREAM_HANDLE);
    };
    let stream: &mut FtStreamRec = stream;

    face.font = None;

    let face_index: FtLong = ((face_instance_index as FtLong).abs()) & 0xFFFF;

    /* does it begin with an MZ header? */
    let mz_header = match stream.seek(0).and_then(|_| read_winmz_header(stream)) {
        Ok(h) => h,
        Err(_) => return Err(FT_ERR_UNKNOWN_FILE_FORMAT),
    };

    error = FT_ERR_UNKNOWN_FILE_FORMAT;
    'exit: {
        'fail: {
            if mz_header.magic == WINFNT_MZ_MAGIC {
                /* yes, now look for an NE header in the file */

                if let Err(e) = stream.seek(mz_header.lfanew as FtULong) {
                    error = e;
                    break 'exit;
                }
                let ne_header = match read_winne_header(stream) {
                    Ok(h) => h,
                    Err(e) => {
                        error = e;
                        break 'exit;
                    }
                };

                error = FT_ERR_UNKNOWN_FILE_FORMAT;
                if ne_header.magic == WINFNT_NE_MAGIC {
                    /* good, now look into the resource table for each FNT resource */
                    let res_offset: FtULong =
                        mz_header.lfanew as FtULong + ne_header.resource_tab_offset as FtULong;
                    let mut font_count: FtUShort = 0;
                    let mut font_offset: FtULong = 0;

                    if let Err(e) = stream.seek(res_offset) {
                        error = e;
                        break 'exit;
                    }
                    if let Err(e) = stream.enter_frame(
                        (ne_header.rname_tab_offset as i32 - ne_header.resource_tab_offset as i32)
                            as i64 as FtULong,
                    ) {
                        error = e;
                        break 'exit;
                    }

                    /* (the seek and the frame set `error' when they succeed) */
                    error = FT_ERR_OK;

                    let size_shift: FtUShort = stream.get_ushort_le();

                    /* Microsoft's specification of the executable-file header format */
                    /* for `New Executable' (NE) doesn't give a limit for the         */
                    /* alignment shift count; however, in 1985, the year of the       */
                    /* specification release, only 32bit values were supported, thus  */
                    /* anything larger than 16 doesn't make sense in general, given   */
                    /* that file offsets are 16bit values, shifted by the alignment   */
                    /* shift count                                                    */
                    if size_shift > 16 {
                        /* invalid alignment shift count for resource data */
                        /* Exit1: */
                        stream.exit_frame();
                        error = FT_ERR_INVALID_FILE_FORMAT;
                        break 'exit;
                    }

                    loop {
                        let type_id: FtUShort = stream.get_ushort_le();
                        if type_id == 0 {
                            break;
                        }

                        let count: FtUShort = stream.get_ushort_le();

                        if type_id == 0x8008 {
                            font_count = count;
                            font_offset = stream.pos().wrapping_add(4).wrapping_add(
                                (stream.cursor() as i64 - stream.limit() as i64) as FtULong,
                            );
                            break;
                        }

                        let c = stream.cursor();
                        stream.set_cursor(c + 4 + count as usize * 12);
                    }

                    stream.exit_frame();

                    if font_count == 0 || font_offset == 0 {
                        /* this file doesn't contain any FNT resources */
                        error = FT_ERR_INVALID_FILE_FORMAT;
                        break 'exit;
                    }

                    /* loading `winfnt_header_fields' needs at least 118 bytes;    */
                    /* use this as a rough measure to check the expected font size */
                    if font_count as FtULong * 118 > stream.size {
                        /* invalid number of faces */
                        error = FT_ERR_INVALID_FILE_FORMAT;
                        break 'exit;
                    }

                    face.root.num_faces = font_count as FtLong;

                    if face_instance_index < 0 {
                        break 'exit;
                    }

                    if face_index >= font_count as FtLong {
                        error = FT_ERR_INVALID_ARGUMENT;
                        break 'exit;
                    }

                    face.font = Some(Box::default());
                    let font = face.font.as_mut().unwrap();

                    if let Err(e) = stream
                        .seek(font_offset.wrapping_add(face_index as FtULong * 12))
                        .and_then(|_| stream.enter_frame(12))
                    {
                        error = e;
                        break 'fail;
                    }

                    font.offset = (stream.get_ushort_le() as FtULong) << size_shift;
                    font.fnt_size = (stream.get_ushort_le() as FtULong) << size_shift;

                    let c = stream.cursor();
                    stream.set_cursor(c + 8);

                    stream.exit_frame();

                    error = match fnt_font_load(font, stream) {
                        Ok(()) => FT_ERR_OK,
                        Err(e) => e,
                    };
                } else if ne_header.magic == WINFNT_PE_MAGIC {
                    if let Err(e) = stream.seek(mz_header.lfanew as FtULong) {
                        error = e;
                        break 'exit;
                    }
                    let pe32_header = match read_winpe32_header(stream) {
                        Ok(h) => h,
                        Err(e) => {
                            error = e;
                            break 'exit;
                        }
                    };

                    if pe32_header.magic != WINFNT_PE_MAGIC as FtULong /* check full signature */
                        || pe32_header.machine != 0x014C /* i386 */
                        || pe32_header.size_of_optional_header != 0xE0 /* FIXME */
                        || pe32_header.magic32 != 0x10B
                    {
                        /* this file has an invalid PE header */
                        error = FT_ERR_INVALID_FILE_FORMAT;
                        break 'exit;
                    }

                    /* (the reads set `error' when they succeed) */
                    error = FT_ERR_OK;

                    face.root.num_faces = 0;

                    let mut pe32_section = WinPe32SectionRec::default();
                    let mut found = false;
                    for _ in 0..pe32_header.number_of_sections {
                        pe32_section = match read_winpe32_section(stream) {
                            Ok(s) => s,
                            Err(e) => {
                                error = e;
                                break 'exit;
                            }
                        };

                        if pe32_header.rsrc_virtual_address == pe32_section.virtual_address {
                            found = true;
                            break;
                        }
                    }

                    if !found {
                        /* this file doesn't contain any resources */
                        error = FT_ERR_INVALID_FILE_FORMAT;
                        break 'exit;
                    }

                    /* Found_rsrc_section: */
                    if let Err(e) = stream.seek(pe32_section.pointer_to_raw_data) {
                        error = e;
                        break 'exit;
                    }
                    let root_dir = match read_winpe_rsrc_dir(stream) {
                        Ok(d) => d,
                        Err(e) => {
                            error = e;
                            break 'exit;
                        }
                    };

                    let root_dir_offset: FtULong = pe32_section.pointer_to_raw_data;

                    /* FIXME (upstream): the FT_UShort counters wrap around (and  */
                    /* the loops do not end) with more than 65535 entries whose */
                    /* reads all succeed                                        */
                    let n1 = root_dir.number_of_named_entries as u32
                        + root_dir.number_of_id_entries as u32;
                    let mut i: FtUShort = 0;
                    while (i as u32) < n1 {
                        let mut dir_entry1 = match stream
                            .seek(root_dir_offset.wrapping_add(16 + i as FtULong * 8))
                            .and_then(|_| read_winpe_rsrc_dir_entry(stream))
                        {
                            Ok(e) => e,
                            Err(e) => {
                                error = e;
                                break 'exit;
                            }
                        };

                        if dir_entry1.offset & 0x80000000 == 0
                        /* DataIsDirectory */
                        {
                            error = FT_ERR_INVALID_FILE_FORMAT;
                            break 'exit;
                        }

                        dir_entry1.offset &= !0x80000000;

                        let name_dir_offset: FtULong = pe32_section
                            .pointer_to_raw_data
                            .wrapping_add(dir_entry1.offset);

                        let name_dir = match stream
                            .seek(
                                pe32_section
                                    .pointer_to_raw_data
                                    .wrapping_add(dir_entry1.offset),
                            )
                            .and_then(|_| read_winpe_rsrc_dir(stream))
                        {
                            Ok(d) => d,
                            Err(e) => {
                                error = e;
                                break 'exit;
                            }
                        };

                        let n2 = name_dir.number_of_named_entries as u32
                            + name_dir.number_of_id_entries as u32;
                        let mut j: FtUShort = 0;
                        while (j as u32) < n2 {
                            let mut dir_entry2 = match stream
                                .seek(name_dir_offset.wrapping_add(16 + j as FtULong * 8))
                                .and_then(|_| read_winpe_rsrc_dir_entry(stream))
                            {
                                Ok(e) => e,
                                Err(e) => {
                                    error = e;
                                    break 'exit;
                                }
                            };

                            if dir_entry2.offset & 0x80000000 == 0
                            /* DataIsDirectory */
                            {
                                error = FT_ERR_INVALID_FILE_FORMAT;
                                break 'exit;
                            }

                            dir_entry2.offset &= !0x80000000;

                            let lang_dir_offset: FtULong = pe32_section
                                .pointer_to_raw_data
                                .wrapping_add(dir_entry2.offset);

                            let lang_dir = match stream
                                .seek(
                                    pe32_section
                                        .pointer_to_raw_data
                                        .wrapping_add(dir_entry2.offset),
                                )
                                .and_then(|_| read_winpe_rsrc_dir(stream))
                            {
                                Ok(d) => d,
                                Err(e) => {
                                    error = e;
                                    break 'exit;
                                }
                            };

                            let n3 = lang_dir.number_of_named_entries as u32
                                + lang_dir.number_of_id_entries as u32;
                            let mut k: FtUShort = 0;
                            while (k as u32) < n3 {
                                let dir_entry3 = match stream
                                    .seek(lang_dir_offset.wrapping_add(16 + k as FtULong * 8))
                                    .and_then(|_| read_winpe_rsrc_dir_entry(stream))
                                {
                                    Ok(e) => e,
                                    Err(e) => {
                                        error = e;
                                        break 'exit;
                                    }
                                };

                                /* FIXME (upstream): this tests `dir_entry2', */
                                /* whose flag was just cleared, instead of    */
                                /* `dir_entry3'                               */
                                if dir_entry2.offset & 0x80000000 != 0
                                /* DataIsDirectory */
                                {
                                    error = FT_ERR_INVALID_FILE_FORMAT;
                                    break 'exit;
                                }

                                if dir_entry1.name == 8
                                /* RT_FONT */
                                {
                                    let data_entry = match stream
                                        .seek(root_dir_offset.wrapping_add(dir_entry3.offset))
                                        .and_then(|_| read_winpe_rsrc_data_entry(stream))
                                    {
                                        Ok(e) => e,
                                        Err(e) => {
                                            error = e;
                                            break 'exit;
                                        }
                                    };

                                    if face_index == face.root.num_faces {
                                        let mut font = Box::<FntFontRec>::default();

                                        font.offset = pe32_section
                                            .pointer_to_raw_data
                                            .wrapping_add(data_entry.offset_to_data)
                                            .wrapping_sub(pe32_section.virtual_address);
                                        font.fnt_size = data_entry.size;

                                        let r = fnt_font_load(&mut font, stream);
                                        face.font = Some(font);
                                        if let Err(e) = r {
                                            error = e;
                                            break 'fail;
                                        }
                                    }

                                    face.root.num_faces += 1;
                                }
                                k = k.wrapping_add(1);
                            }
                            j = j.wrapping_add(1);
                        }
                        i = i.wrapping_add(1);
                    }
                }

                if face.root.num_faces == 0 {
                    /* this file doesn't contain any RT_FONT resources */
                    error = FT_ERR_INVALID_FILE_FORMAT;
                    break 'exit;
                }

                if face_index >= face.root.num_faces {
                    error = FT_ERR_INVALID_ARGUMENT;
                    break 'exit;
                }
            }
        }

        /* Fail: */
        if error != 0 {
            fnt_font_done(face);
        }
    }

    /* Exit: */
    if error != 0 {
        Err(error)
    } else {
        Ok(())
    }
}

/// `FNT_CMapRec` (the data of a charmap object)
#[derive(Debug, Clone, Copy, Default)]
pub struct FntCMapRec {
    pub first: FtUInt32,
    pub count: FtUInt32,
}

/// `fnt_cmap_init`
fn fnt_cmap_init(face: &FntFaceRec) -> FtCMapData {
    let Some(font) = face.font.as_ref() else {
        return FtCMapData::Fnt(FntCMapRec::default());
    };

    let first = font.header.first_char as FtUInt32;
    let count = (font.header.last_char as FtUInt32)
        .wrapping_sub(first)
        .wrapping_add(1);

    FtCMapData::Fnt(FntCMapRec { first, count })
}

fn fntcmap(cmap: &FtCMapRec) -> FntCMapRec {
    match cmap.data {
        FtCMapData::Fnt(c) => c,
        _ => FntCMapRec::default(),
    }
}

/// `fnt_cmap_char_index`
fn fnt_cmap_char_index(cmap: &FtCMapRec, char_code: FtUInt32) -> FtUInt {
    let fntcmap = fntcmap(cmap);
    let mut gindex: FtUInt = 0;

    let char_code = char_code.wrapping_sub(fntcmap.first);
    if char_code < fntcmap.count {
        /* we artificially increase the glyph index; */
        /* FNT_Load_Glyph reverts to the right one   */
        gindex = char_code.wrapping_add(1) as FtUInt;
    }
    gindex
}

/// `fnt_cmap_char_next`
fn fnt_cmap_char_next(cmap: &mut FtCMapRec, pchar_code: &mut FtUInt32) -> FtUInt {
    let fntcmap = fntcmap(cmap);
    let mut gindex: FtUInt = 0;
    let mut result: FtUInt32 = 0;
    let mut char_code: FtUInt32 = pchar_code.wrapping_add(1);

    if char_code <= fntcmap.first {
        result = fntcmap.first;
        gindex = 1;
    } else {
        char_code -= fntcmap.first;
        if char_code < fntcmap.count {
            result = fntcmap.first + char_code;
            gindex = char_code.wrapping_add(1) as FtUInt;
        }
    }

    *pchar_code = result;
    gindex
}

/// `fnt_cmap_class_rec`
pub static FNT_CMAP_CLASS_REC: FtCMapClassRec = FtCMapClassRec {
    char_index: fnt_cmap_char_index,
    char_next: fnt_cmap_char_next,

    char_var_index: None,
    char_var_default: None,
    variant_list: None,
    charvariant_list: None,
    variantchar_list: None,
};

/// `FNT_Face_Done`
fn fnt_face_done(fntface: &mut FtFace) {
    let FtFace::Fnt(face) = fntface else {
        return;
    };

    fnt_font_done(face);

    face.root.available_sizes = Vec::new();
    face.root.num_fixed_sizes = 0;
}

/// `FNT_Face_Init`
fn fnt_face_init(
    fntface: &mut FtFace,
    face_instance_index: FtInt,
    _params: &[FtParameter],
) -> FtResult<()> {
    let FtFace::Fnt(face) = fntface else {
        return Err(FT_ERR_INVALID_FACE_HANDLE);
    };

    let face_index: FtInt = (face_instance_index.wrapping_abs()) & 0xFFFF;

    let r = fnt_face_init_body(face, face_instance_index, face_index);
    match r {
        /* Exit: */
        Ok(()) => Ok(()),
        Err((e, true)) => {
            /* Fail: */
            fnt_face_done(fntface);
            Err(e)
        }
        Err((e, false)) => Err(e),
    }
}

/// The body of `FNT_Face_Init` (errors say whether to go to `Fail`).
fn fnt_face_init_body(
    face: &mut FntFaceRec,
    face_instance_index: FtInt,
    face_index: FtInt,
) -> Result<(), (FtError, bool)> {
    /* try to load font from a DLL */
    let mut error = fnt_face_get_dll_font(face, face_instance_index);
    if error.is_ok() && face_instance_index < 0 {
        return Ok(());
    }

    if matches!(error, Err(e) if ft_err_eq(e, FT_ERR_UNKNOWN_FILE_FORMAT)) {
        /* this didn't work; try to load a single FNT font */
        face.root.num_faces = 1;

        let Some(stream) = face.root.stream.as_mut() else {
            return Err((FT_ERR_INVALID_STREAM_HANDLE, true));
        };
        let mut font = Box::new(FntFontRec {
            offset: 0,
            fnt_size: stream.size,
            ..Default::default()
        });

        error = fnt_font_load(&mut font, stream);
        face.font = Some(font);

        if error.is_ok() {
            if face_instance_index < 0 {
                return Ok(());
            }

            if face_index > 0 {
                error = Err(FT_ERR_INVALID_ARGUMENT);
            }
        }
    }

    if let Err(e) = error {
        return Err((e, true));
    }

    let Some(font) = face.font.as_ref() else {
        return Err((FT_ERR_INVALID_FILE_FORMAT, true));
    };

    /* sanity check */
    if font.header.pixel_height == 0 {
        /* invalid pixel height */
        return Err((FT_ERR_INVALID_FILE_FORMAT, true));
    }

    /* we now need to fill the root FT_Face fields */
    /* with relevant information                   */
    {
        let root = &mut face.root;

        root.face_index = face_index as FtLong;

        root.face_flags |= FT_FACE_FLAG_FIXED_SIZES | FT_FACE_FLAG_HORIZONTAL;

        if font.header.avg_width == font.header.max_width {
            root.face_flags |= FT_FACE_FLAG_FIXED_WIDTH;
        }

        if font.header.italic != 0 {
            root.style_flags |= FT_STYLE_FLAG_ITALIC;
        }

        if font.header.weight >= 800 {
            root.style_flags |= FT_STYLE_FLAG_BOLD;
        }

        /* set up the `fixed_sizes' array */
        let mut bsize = FtBitmapSize::default();

        root.num_fixed_sizes = 1;

        {
            bsize.width = font.header.avg_width as FtShort;
            bsize.height =
                (font.header.pixel_height as i32 + font.header.external_leading as i32) as FtShort;
            bsize.size = (font.header.nominal_point_size as FtPos) << 6;

            let mut x_res: FtUShort = font.header.horizontal_resolution;
            if x_res == 0 {
                x_res = 72;
            }

            let mut y_res: FtUShort = font.header.vertical_resolution;
            if y_res == 0 {
                y_res = 72;
            }

            bsize.y_ppem = ft_mul_div(bsize.size, y_res as FtLong, 72);
            bsize.y_ppem = ft_pix_round(bsize.y_ppem);

            /*
             * this reads:
             *
             * the nominal height is larger than the bbox's height
             *
             * => nominal_point_size contains incorrect value;
             *    use pixel_height as the nominal height
             */
            if bsize.y_ppem > ((font.header.pixel_height as FtPos) << 6) {
                /* use pixel_height as the nominal height */
                bsize.y_ppem = (font.header.pixel_height as FtPos) << 6;
                bsize.size = ft_mul_div(bsize.y_ppem, 72, y_res as FtLong);
            }

            bsize.x_ppem = ft_mul_div(bsize.size, x_res as FtLong, 72);
            bsize.x_ppem = ft_pix_round(bsize.x_ppem);
        }
        root.available_sizes = vec![bsize];
    }

    {
        let mut charmap = FtCharMapRec {
            encoding: FT_ENCODING_NONE,
            /* initial platform/encoding should indicate unset status? */
            platform_id: TT_PLATFORM_APPLE_UNICODE,
            encoding_id: TT_APPLE_ID_DEFAULT,
        };

        if font.header.charset == FT_WinFNT_ID_MAC {
            charmap.encoding = FT_ENCODING_APPLE_ROMAN;
            charmap.platform_id = TT_PLATFORM_MACINTOSH;
            /*        charmap.encoding_id = TT_MAC_ID_ROMAN; */
        }

        let data = fnt_cmap_init(face);
        if let Err(e) = ft_cmap_new(&mut face.root, &FNT_CMAP_CLASS_REC, data, charmap) {
            return Err((e, true));
        }
    }

    let Some(font) = face.font.as_ref() else {
        return Err((FT_ERR_INVALID_FILE_FORMAT, true));
    };
    let root = &mut face.root;

    /* set up remaining flags */

    if font.header.last_char < font.header.first_char {
        /* invalid number of glyphs */
        return Err((FT_ERR_INVALID_FILE_FORMAT, true));
    }

    /* reserve one slot for the .notdef glyph at index 0 */
    root.num_glyphs = font.header.last_char as FtLong - font.header.first_char as FtLong + 1 + 1;

    if font.header.face_name_offset >= font.header.file_size {
        /* invalid family name offset */
        return Err((FT_ERR_INVALID_FILE_FORMAT, true));
    }
    let family_size: FtULong = font.header.file_size - font.header.face_name_offset;
    /* Some broken fonts don't delimit the face name with a final */
    /* null byte -- the frame is erroneously one byte too small.  */
    /* We thus allocate one more byte, setting it explicitly to   */
    /* zero.                                                      */
    let mut family_name = match ft_qalloc(family_size as FtLong + 1) {
        Ok(v) => v,
        Err(e) => return Err((e, true)),
    };

    let start = font.header.face_name_offset as usize;
    let avail = font
        .fnt_frame
        .len()
        .saturating_sub(start)
        .min(family_size as usize);
    family_name[..avail].copy_from_slice(&font.fnt_frame[start..start + avail]);

    family_name[family_size as usize] = b'\0';

    /* shrink it to the actual length */
    let len = family_name
        .iter()
        .position(|&c| c == 0)
        .unwrap_or(family_name.len());
    family_name.truncate(len);

    root.family_name = Some(String::from_utf8_lossy(&family_name).into_owned());
    root.style_name = Some("Regular".into());

    if root.style_flags & FT_STYLE_FLAG_BOLD != 0 {
        if root.style_flags & FT_STYLE_FLAG_ITALIC != 0 {
            root.style_name = Some("Bold Italic".into());
        } else {
            root.style_name = Some("Bold".into());
        }
    } else if root.style_flags & FT_STYLE_FLAG_ITALIC != 0 {
        root.style_name = Some("Italic".into());
    }

    if let Some(font) = face.font.as_mut() {
        font.family_name = Some(family_name);
    }

    Ok(())
}

/// `FNT_Size_Select`
fn fnt_size_select(fntface: &mut FtFace, _strike_index: FtULong) -> FtResult<()> {
    let FtFace::Fnt(face) = fntface else {
        return Err(FT_ERR_INVALID_FACE_HANDLE);
    };
    let Some(font) = face.font.as_ref() else {
        return Err(FT_ERR_INVALID_FACE_HANDLE);
    };
    let header = font.header;

    ft_select_metrics(&mut face.root, 0);

    let metrics = &mut face.root.size.metrics;
    metrics.ascender = header.ascent as FtPos * 64;
    metrics.descender = -(header.pixel_height as FtPos - header.ascent as FtPos) * 64;
    metrics.max_advance = header.max_width as FtPos * 64;

    Ok(())
}

/// `FNT_Size_Request`
fn fnt_size_request(fntface: &mut FtFace, req: &FtSizeRequestRec) -> FtResult<()> {
    let FtFace::Fnt(face) = &*fntface else {
        return Err(FT_ERR_INVALID_FACE_HANDLE);
    };
    let Some(font) = face.font.as_ref() else {
        return Err(FT_ERR_INVALID_FACE_HANDLE);
    };
    let header = &font.header;
    let bsize = face
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
            if height == header.pixel_height as FtLong {
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
        fnt_size_select(fntface, 0)
    }
}

/// `FNT_Load_Glyph`
fn fnt_load_glyph(fntface: &mut FtFace, glyph_index: FtUInt, load_flags: FtInt32) -> FtResult<()> {
    let FtFace::Fnt(face) = fntface else {
        return Err(FT_ERR_INVALID_FACE_HANDLE);
    };
    let mut glyph_index = glyph_index;

    let num_glyphs = face.root.num_glyphs as FtUInt;
    let font = match face.font.as_ref() {
        Some(font) if glyph_index < num_glyphs => font,
        _ => return Err(FT_ERR_INVALID_ARGUMENT),
    };
    let slot = &mut face.root.glyph;
    let bitmap = &mut slot.bitmap;

    if glyph_index > 0 {
        glyph_index -= 1; /* revert to real index */
    } else {
        glyph_index = font.header.default_char as FtUInt; /* the `.notdef' glyph  */
    }

    let new_format = font.header.version == 0x300;
    let len: FtUInt = if new_format { 6 } else { 4 };

    /* get glyph width and offset */
    let mut offset: FtULong =
        (if new_format { 148 } else { 118 }) + len as FtULong * glyph_index as FtULong;

    if offset
        >= font
            .header
            .file_size
            .wrapping_sub(2)
            .wrapping_sub(if new_format { 4 } else { 2 })
    {
        /* invalid FNT offset */
        return Err(FT_ERR_INVALID_FILE_FORMAT);
    }

    let frame = &font.fnt_frame;
    let p = offset as usize;

    bitmap.width = ft_peek_ushort_le(frame, p) as u32;

    /* jump to glyph entry */
    if new_format {
        offset = ft_peek_ulong_le(frame, p + 2) as FtULong;
    } else {
        offset = ft_peek_ushort_le(frame, p + 2) as FtULong;
    }

    if offset >= font.header.file_size {
        /* invalid FNT offset */
        return Err(FT_ERR_INVALID_FILE_FORMAT);
    }

    bitmap.rows = font.header.pixel_height as u32;
    bitmap.pixel_mode = FT_PIXEL_MODE_MONO;

    slot.bitmap_left = 0;
    slot.bitmap_top = font.header.ascent as FtInt;
    slot.format = FT_GLYPH_FORMAT_BITMAP;

    /* now set up metrics */
    slot.metrics.width = (bitmap.width << 6) as FtPos;
    slot.metrics.height = (bitmap.rows << 6) as FtPos;
    slot.metrics.horiAdvance = (bitmap.width << 6) as FtPos;
    slot.metrics.horiBearingX = 0;
    slot.metrics.horiBearingY = (slot.bitmap_top << 6) as FtPos;

    ft_synthesize_vertical_metrics(&mut slot.metrics, (bitmap.rows << 6) as FtPos);

    if load_flags & FT_LOAD_BITMAP_METRICS_ONLY != 0 {
        return Ok(());
    }

    /* jump to glyph data */
    let mut p = /* font->header.bits_offset */ offset as usize;

    /* allocate and build bitmap */
    {
        let mut pitch: FtUInt = (bitmap.width + 7) >> 3;

        bitmap.pitch = pitch as i32;
        if pitch == 0 || offset as u64 + pitch as u64 * bitmap.rows as u64 > font.header.file_size {
            /* invalid bitmap width */
            return Err(FT_ERR_INVALID_FILE_FORMAT);
        }

        /* note: since glyphs are stored in columns and not in rows we */
        /*       can't use ft_glyphslot_set_bitmap                     */
        bitmap.buffer =
            super::super::base::ftmemory::ft_alloc_mult(bitmap.rows as FtLong, pitch as FtLong)?;

        let rows = bitmap.rows as usize;
        let bpitch = bitmap.pitch as usize;
        let mut column = 0usize;

        while pitch > 0 {
            let limit = p + rows;

            let mut write = column;
            while p < limit {
                bitmap.buffer[write] = frame.get(p).copied().unwrap_or(0);
                p += 1;
                write += bpitch;
            }
            pitch -= 1;
            column += 1;
        }

        slot.internal.flags = FT_GLYPH_OWN_BITMAP;
    }

    Ok(())
}

/// `winfnt_get_header`
fn winfnt_get_header(face: &FtFace) -> FtResult<FtWinFntHeaderRec> {
    match face {
        FtFace::Fnt(face) => match face.font.as_ref() {
            Some(font) => Ok(font.header),
            None => Err(FT_ERR_INVALID_ARGUMENT),
        },
        _ => Err(FT_ERR_INVALID_ARGUMENT),
    }
}

/// `winfnt_service_rec`
static WINFNT_SERVICE_REC: FtServiceWinFntRec = FtServiceWinFntRec {
    get_header: winfnt_get_header, /* get_header */
};

/// `FT_FONT_FORMAT_WINFNT`
pub const FT_FONT_FORMAT_WINFNT: &str = "Windows FNT";

/*
 * SERVICE LIST
 *
 */

/// `winfnt_services`
static WINFNT_SERVICES: [(&str, FtService); 2] = [
    (
        FT_SERVICE_ID_FONT_FORMAT,
        FtService::FontFormat(FT_FONT_FORMAT_WINFNT),
    ),
    (FT_SERVICE_ID_WINFNT, FtService::WinFnt(&WINFNT_SERVICE_REC)),
];

/// `winfnt_get_service`
fn winfnt_get_service(_module: &FtModuleRec, service_id: &str) -> Option<FtService> {
    WINFNT_SERVICES
        .iter()
        .find(|(id, _)| *id == service_id)
        .map(|(_, s)| *s)
}

fn fnt_new_face(root: FtFaceRec) -> FtFace {
    FtFace::Fnt(Box::new(FntFaceRec { root, font: None }))
}

/// `winfnt_driver_class`
pub static WINFNT_DRIVER_CLASS: FtDriverClassRec = FtDriverClassRec {
    root: FtModuleClass {
        module_flags: FT_MODULE_FONT_DRIVER | FT_MODULE_DRIVER_NO_OUTLINES,

        module_name: "winfonts",
        module_version: 0x10000,
        module_requires: 0x20000,

        module_interface: FtModuleInterface::None, /* module-specific interface */

        module_init: None, /* FT_Module_Constructor  module_init   */
        module_done: None, /* FT_Module_Destructor   module_done   */
        get_interface: Some(winfnt_get_service), /* FT_Module_Requester    get_interface */
    },

    new_face: fnt_new_face,

    init_face: Some(fnt_face_init), /* FT_Face_InitFunc  init_face */
    done_face: Some(fnt_face_done), /* FT_Face_DoneFunc  done_face */
    init_size: None,                /* FT_Size_InitFunc  init_size */
    done_size: None,                /* FT_Size_DoneFunc  done_size */
    init_slot: None,                /* FT_Slot_InitFunc  init_slot */
    done_slot: None,                /* FT_Slot_DoneFunc  done_slot */

    load_glyph: Some(fnt_load_glyph), /* FT_Slot_LoadFunc  load_glyph */

    get_kerning: None,  /* FT_Face_GetKerningFunc   get_kerning  */
    attach_file: None,  /* FT_Face_AttachFunc       attach_file  */
    get_advances: None, /* FT_Face_GetAdvancesFunc  get_advances */

    request_size: Some(fnt_size_request), /* FT_Size_RequestFunc  request_size */
    select_size: Some(fnt_size_select),   /* FT_Size_SelectFunc   select_size  */
};
