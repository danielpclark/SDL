// Rust translation of the resource fork directory access of
// src/base/ftrfork.c from FreeType (2.13.2, as SDL_ttf's external/freetype
// pins it).
// Copyright (C) 2004-2023 by Masatake YAMATO and Redhat K.K.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! Embedded resource forks accessor (body).
//!
//! Development of the code in this file is support of
//! Information-technology Promotion Agency, Japan.
//!
//! Only the resource fork directory access is translated: the guessing
//! functions (`FT_Raccess_Guess` and its rules) look for resource forks in
//! other files next to a font opened by path name, which the streams here
//! (opened from memory or an `SDL_IOStream`, as SDL_ttf opens fonts) never
//! are.

use super::super::fttypes::*;
use super::ftmemory::ft_new_array;
use super::ftstream::FtStreamRec;

/// `FT_MAC_RFORK_MAX_LEN`
pub const FT_MAC_RFORK_MAX_LEN: FtULong = 0x00FFFFFF;

/// `FT_RFork_Ref`
#[derive(Debug, Clone, Copy, Default)]
struct FtRForkRef {
    res_id: FtShort,
    offset: FtLong,
}

/*************************************************************************/
/****                                                                 ****/
/****               Resource fork directory access                    ****/
/****                                                                 ****/
/*************************************************************************/

/// `FT_Raccess_Get_HeaderInfo`: returns `(map_offset, rdata_pos)`
pub fn ft_raccess_get_header_info(
    stream: &mut FtStreamRec,
    rfork_offset: FtLong,
) -> FtResult<(FtLong, FtLong)> {
    let mut head = [0u8; 16];
    let mut head2 = [0u8; 16];

    stream.seek(rfork_offset as FtULong)?;

    stream.read(&mut head)?;

    /* ensure positive values */
    if head[0] >= 0x80 || head[4] >= 0x80 || head[8] >= 0x80 || head[12] >= 0x80 {
        return Err(FT_ERR_UNKNOWN_FILE_FORMAT);
    }

    let be = |i: usize| -> FtLong {
        ((head[i] as FtLong) << 24)
            | ((head[i + 1] as FtLong) << 16)
            | ((head[i + 2] as FtLong) << 8)
            | head[i + 3] as FtLong
    };
    let mut rdata_pos = be(0);
    let mut map_pos = be(4);
    let rdata_len = be(8);
    let map_len = be(12);

    /* the map must not be empty */
    if map_pos == 0 {
        return Err(FT_ERR_UNKNOWN_FILE_FORMAT);
    }

    /* check whether rdata and map overlap */
    if rdata_pos < map_pos {
        if rdata_pos > map_pos - rdata_len {
            return Err(FT_ERR_UNKNOWN_FILE_FORMAT);
        }
    } else if map_pos > rdata_pos - map_len {
        return Err(FT_ERR_UNKNOWN_FILE_FORMAT);
    }

    /* check whether end of rdata or map exceeds stream size */
    if FtLong::MAX - rdata_len < rdata_pos
        || FtLong::MAX - map_len < map_pos
        || FtLong::MAX - (rdata_pos + rdata_len) < rfork_offset
        || FtLong::MAX - (map_pos + map_len) < rfork_offset
        || (rfork_offset + rdata_pos + rdata_len) as FtULong > stream.size
        || (rfork_offset + map_pos + map_len) as FtULong > stream.size
    {
        return Err(FT_ERR_UNKNOWN_FILE_FORMAT);
    }

    rdata_pos += rfork_offset;
    map_pos += rfork_offset;

    stream.seek(map_pos as FtULong)?;

    head2[15] = head[15].wrapping_add(1); /* make it be different */

    stream.read(&mut head2)?;

    let mut allzeros = true;
    let mut allmatch = true;
    for i in 0..16 {
        if head2[i] != 0 {
            allzeros = false;
        }
        if head2[i] != head[i] {
            allmatch = false;
        }
    }
    if !allzeros && !allmatch {
        return Err(FT_ERR_UNKNOWN_FILE_FORMAT);
    }

    /* If we have reached this point then it is probably a mac resource */
    /* file.  Now, does it contain any interesting resources?           */

    let _ = stream.skip(
        4 /* skip handle to next resource map */
            + 2 /* skip file resource number */
            + 2, /* skip attributes */
    );

    let type_list = stream.read_short()? as FtLong;
    if type_list < 0 {
        return Err(FT_ERR_UNKNOWN_FILE_FORMAT);
    }

    stream.seek((map_pos + type_list) as FtULong)?;

    Ok((map_pos + type_list, rdata_pos))
}

/// `FT_Raccess_Get_DataOffsets`
pub fn ft_raccess_get_data_offsets(
    stream: &mut FtStreamRec,
    map_offset: FtLong,
    rdata_pos: FtLong,
    tag: FtLong,
    sort_by_res_id: bool,
) -> FtResult<Vec<FtLong>> {
    stream.seek(map_offset as FtULong)?;

    let mut cnt = stream.read_short()? as i32;
    cnt += 1;

    /* `rpos' is a signed 16bit integer offset to resource records; the    */
    /* size of a resource record is 12 bytes.  The map header is 28 bytes, */
    /* and a type list needs 10 bytes or more.  If we assume that the name */
    /* list is empty and we have only a single entry in the type list,     */
    /* there can be at most                                                */
    /*                                                                     */
    /*   (32768 - 28 - 10) / 12 = 2727                                     */
    /*                                                                     */
    /* resources.                                                          */
    /*                                                                     */
    /* A type list starts with a two-byte counter, followed by 10-byte     */
    /* type records.  Assuming that there are no resources, the number of  */
    /* type records can be at most                                         */
    /*                                                                     */
    /*   (32768 - 28 - 2) / 8 = 4079                                       */
    /*                                                                     */
    if cnt > 4079 {
        return Err(FT_ERR_INVALID_TABLE);
    }

    for _ in 0..cnt {
        let tag_internal = stream.read_long()? as FtLong;
        let subcnt = stream.read_short()? as i32;
        let mut rpos = stream.read_short()? as FtLong;

        if tag_internal == tag {
            let count = subcnt as FtLong + 1;
            rpos += map_offset;

            /* a zero count might be valid in the resource specification, */
            /* however, it is completely useless to us                    */
            if !(1..=2727).contains(&count) {
                return Err(FT_ERR_INVALID_TABLE);
            }

            stream.seek(rpos as FtULong)?;

            let mut refs: Vec<FtRForkRef> = ft_new_array(count)?;

            for r in refs.iter_mut() {
                r.res_id = stream.read_short()?;
                stream.skip(2)?; /* resource name offset */
                let temp = stream.read_long()? as FtLong; /* attributes (8bit), offset (24bit) */
                stream.skip(4)?; /* mbz */

                /*
                 * According to Inside Macintosh: More Macintosh Toolbox,
                 * "Resource IDs" (1-46), there are some reserved IDs.
                 * However, FreeType2 is not a font synthesizer, no need
                 * to check the acceptable resource ID.
                 */
                if temp < 0 {
                    return Err(FT_ERR_INVALID_TABLE);
                }

                r.offset = temp & 0xFFFFFF;
            }

            if sort_by_res_id {
                /* (`ft_qsort`; glibc's is a merge sort, which is stable) */
                refs.sort_by_key(|r| r.res_id);
            }

            /* XXX: duplicated reference ID,
             *      gap between reference IDs are acceptable?
             *      further investigation on Apple implementation is needed.
             */
            let mut offsets_internal: Vec<FtLong> = ft_new_array(count)?;
            for (o, r) in offsets_internal.iter_mut().zip(refs.iter()) {
                *o = rdata_pos + r.offset;
            }

            return Ok(offsets_internal);
        }
    }

    Err(FT_ERR_CANNOT_OPEN_RESOURCE)
}
