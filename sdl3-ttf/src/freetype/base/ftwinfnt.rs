// Rust translation of src/base/ftwinfnt.c, include/freetype/ftwinfnt.h and
// include/freetype/internal/services/svwinfnt.h from FreeType (2.13.2, as
// SDL_ttf's external/freetype pins it).
// Copyright (C) 2003-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! FreeType API for accessing Windows FNT specific info (body).

use super::super::fttypes::*;
use super::ftobjs::*;

/* @enum: FT_WinFNT_ID_XXX */

pub const FT_WinFNT_ID_CP1252: FtByte = 0;
pub const FT_WinFNT_ID_DEFAULT: FtByte = 1;
pub const FT_WinFNT_ID_SYMBOL: FtByte = 2;
pub const FT_WinFNT_ID_MAC: FtByte = 77;
pub const FT_WinFNT_ID_CP932: FtByte = 128;
pub const FT_WinFNT_ID_CP949: FtByte = 129;
pub const FT_WinFNT_ID_CP1361: FtByte = 130;
pub const FT_WinFNT_ID_CP936: FtByte = 134;
pub const FT_WinFNT_ID_CP950: FtByte = 136;
pub const FT_WinFNT_ID_CP1253: FtByte = 161;
pub const FT_WinFNT_ID_CP1254: FtByte = 162;
pub const FT_WinFNT_ID_CP1258: FtByte = 163;
pub const FT_WinFNT_ID_CP1255: FtByte = 177;
pub const FT_WinFNT_ID_CP1256: FtByte = 178;
pub const FT_WinFNT_ID_CP1257: FtByte = 186;
pub const FT_WinFNT_ID_CP1251: FtByte = 204;
pub const FT_WinFNT_ID_CP874: FtByte = 222;
pub const FT_WinFNT_ID_CP1250: FtByte = 238;
pub const FT_WinFNT_ID_OEM: FtByte = 255;

/// `FT_WinFNT_HeaderRec`: Windows FNT Header info.
#[derive(Debug, Clone, Copy)]
#[allow(non_snake_case)]
pub struct FtWinFntHeaderRec {
    pub version: FtUShort,
    pub file_size: FtULong,
    pub copyright: [FtByte; 60],
    pub file_type: FtUShort,
    pub nominal_point_size: FtUShort,
    pub vertical_resolution: FtUShort,
    pub horizontal_resolution: FtUShort,
    pub ascent: FtUShort,
    pub internal_leading: FtUShort,
    pub external_leading: FtUShort,
    pub italic: FtByte,
    pub underline: FtByte,
    pub strike_out: FtByte,
    pub weight: FtUShort,
    pub charset: FtByte,
    pub pixel_width: FtUShort,
    pub pixel_height: FtUShort,
    pub pitch_and_family: FtByte,
    pub avg_width: FtUShort,
    pub max_width: FtUShort,
    pub first_char: FtByte,
    pub last_char: FtByte,
    pub default_char: FtByte,
    pub break_char: FtByte,
    pub bytes_per_row: FtUShort,
    pub device_offset: FtULong,
    pub face_name_offset: FtULong,
    pub bits_pointer: FtULong,
    pub bits_offset: FtULong,
    pub reserved: FtByte,
    pub flags: FtULong,
    pub A_space: FtUShort,
    pub B_space: FtUShort,
    pub C_space: FtUShort,
    pub color_table_offset: FtUShort,
    pub reserved1: [FtULong; 4],
}

impl Default for FtWinFntHeaderRec {
    fn default() -> Self {
        FtWinFntHeaderRec {
            version: 0,
            file_size: 0,
            copyright: [0; 60],
            file_type: 0,
            nominal_point_size: 0,
            vertical_resolution: 0,
            horizontal_resolution: 0,
            ascent: 0,
            internal_leading: 0,
            external_leading: 0,
            italic: 0,
            underline: 0,
            strike_out: 0,
            weight: 0,
            charset: 0,
            pixel_width: 0,
            pixel_height: 0,
            pitch_and_family: 0,
            avg_width: 0,
            max_width: 0,
            first_char: 0,
            last_char: 0,
            default_char: 0,
            break_char: 0,
            bytes_per_row: 0,
            device_offset: 0,
            face_name_offset: 0,
            bits_pointer: 0,
            bits_offset: 0,
            reserved: 0,
            flags: 0,
            A_space: 0,
            B_space: 0,
            C_space: 0,
            color_table_offset: 0,
            reserved1: [0; 4],
        }
    }
}

/// `FT_Service_WinFntRec`
#[derive(Debug)]
pub struct FtServiceWinFntRec {
    pub get_header: fn(face: &FtFace) -> FtResult<FtWinFntHeaderRec>,
}

pub const FT_SERVICE_ID_WINFNT: &str = "winfonts";

/// `FT_Get_WinFNT_Header`: Retrieve a Windows FNT font info header.
///
/// This function only works with Windows FNT faces, returning an error
/// otherwise.
pub fn ft_get_winfnt_header(face: &FtFace) -> FtResult<FtWinFntHeaderRec> {
    match ft_face_find_service(face, FT_SERVICE_ID_WINFNT) {
        Some(FtService::WinFnt(service)) => (service.get_header)(face),
        _ => Err(FT_ERR_INVALID_ARGUMENT),
    }
}
