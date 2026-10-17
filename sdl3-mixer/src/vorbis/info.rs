// Rust translation of lib/info.c from libvorbis 1.3.7
// (https://xiph.org/vorbis/, at the revision SDL_mixer's external/vorbis
// pins).
// THE OggVorbis SOURCE CODE IS (C) COPYRIGHT 1994-2015
// by the Xiph.Org Foundation https://xiph.org/
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! function: maintain the info structure, info <-> header packets
//!
//! General handling of the header and the vorbis_info structure (and
//! substructures). The packing side (`_vorbis_pack_*()`,
//! `vorbis_commentheader_out()`, `vorbis_analysis_headerout()`), the
//! comment editing and queries (`vorbis_comment_add*()`,
//! `vorbis_comment_query*()`), `vorbis_granule_time()` and
//! `vorbis_version_string()` aren't translated: decoding doesn't use them.

use super::floor0::floor0_unpack;
use super::floor1::floor1_unpack;
use super::mapping0::mapping0_unpack;
use super::res0::res0_unpack;
use super::sharedbook::vorbis_staticbook_unpack;
use super::*;
use crate::ogg::bitwise::*;

/* helpers */
/// Translation of `_v_readstring()`.
fn _v_readstring(o: &mut OggpackBuffer<'_>, buf: &mut [u8], bytes: i32) {
    for b in buf.iter_mut().take(bytes.max(0) as usize) {
        *b = oggpack_read(o, 8) as u8;
    }
}

/// Translation of `vorbis_comment_init()`.
pub(crate) fn vorbis_comment_init(vc: &mut VorbisComment) {
    *vc = VorbisComment::default();
}

/// Translation of `vorbis_comment_clear()`.
pub(crate) fn vorbis_comment_clear(vc: &mut VorbisComment) {
    *vc = VorbisComment::default();
}

/* blocksize 0 is guaranteed to be short, 1 is guaranteed to be long.
They may be equal, but short will never ge greater than long */
/// Translation of `vorbis_info_blocksize()`.
pub(crate) fn vorbis_info_blocksize(vi: &VorbisInfo, zo: i32) -> i32 {
    match &vi.codec_setup {
        Some(ci) => ci.blocksizes[zo as usize] as i32,
        None => -1,
    }
}

/* used by synthesis, which has a full, alloced vi */
/// Translation of `vorbis_info_init()`.
pub(crate) fn vorbis_info_init(vi: &mut VorbisInfo) {
    *vi = VorbisInfo::default();
    vi.codec_setup = Some(Box::default());
}

/// Translation of `vorbis_info_clear()`: the setup's modes, mappings,
/// floors, residues and books go with it.
pub(crate) fn vorbis_info_clear(vi: &mut VorbisInfo) {
    *vi = VorbisInfo::default();
}

/* Header packing/unpacking ********************************************/

/// Translation of `_vorbis_unpack_info()`.
fn _vorbis_unpack_info(vi: &mut VorbisInfo, opb: &mut OggpackBuffer<'_>) -> i32 {
    let mut bs: i32;
    if vi.codec_setup.is_none() {
        return OV_EFAULT;
    }

    'err_out: {
        vi.version = oggpack_read(opb, 32) as i32;
        if vi.version != 0 {
            return OV_EVERSION;
        }

        vi.channels = oggpack_read(opb, 8) as i32;
        vi.rate = oggpack_read(opb, 32);

        vi.bitrate_upper = oggpack_read(opb, 32) as i32 as i64;
        vi.bitrate_nominal = oggpack_read(opb, 32) as i32 as i64;
        vi.bitrate_lower = oggpack_read(opb, 32) as i32 as i64;

        let ci = vi.codec_setup.as_mut().unwrap();
        bs = oggpack_read(opb, 4) as i32;
        if bs < 0 {
            break 'err_out;
        }
        ci.blocksizes[0] = 1 << bs;
        bs = oggpack_read(opb, 4) as i32;
        if bs < 0 {
            break 'err_out;
        }
        ci.blocksizes[1] = 1 << bs;

        if vi.rate < 1 {
            break 'err_out;
        }
        if vi.channels < 1 {
            break 'err_out;
        }
        if ci.blocksizes[0] < 64 {
            break 'err_out;
        }
        if ci.blocksizes[1] < ci.blocksizes[0] {
            break 'err_out;
        }
        if ci.blocksizes[1] > 8192 {
            break 'err_out;
        }

        if oggpack_read(opb, 1) != 1 {
            break 'err_out; /* EOP check */
        }

        return 0;
    }
    vorbis_info_clear(vi);
    OV_EBADHEADER
}

/// Translation of `_vorbis_unpack_comment()`.
fn _vorbis_unpack_comment(vc: &mut VorbisComment, opb: &mut OggpackBuffer<'_>) -> i32 {
    'err_out: {
        let vendorlen = oggpack_read(opb, 32) as i32;
        if vendorlen < 0 {
            break 'err_out;
        }
        if vendorlen as i64 > opb.storage - 8 {
            break 'err_out;
        }
        let Some(mut vendor) = try_vec::<u8>(vendorlen as usize) else {
            break 'err_out; /* (C would crash on the NULL) */
        };
        _v_readstring(opb, &mut vendor, vendorlen);
        vc.vendor = vendor;
        vc.vendor_set = true;
        let mut i = oggpack_read(opb, 32) as i32;
        if i < 0 {
            break 'err_out;
        }
        if i as i64 > ((opb.storage - oggpack_bytes(opb)) >> 2) {
            break 'err_out;
        }
        vc.comments = i;
        vc.user_comments = Vec::new();
        if vc.user_comments.try_reserve(vc.comments as usize).is_err() {
            break 'err_out;
        }

        i = 0;
        while i < vc.comments {
            let len = oggpack_read(opb, 32) as i32;
            if len < 0 {
                break 'err_out;
            }
            if len as i64 > opb.storage - oggpack_bytes(opb) {
                break 'err_out;
            }
            let Some(mut comment) = try_vec::<u8>(len as usize) else {
                break 'err_out;
            };
            _v_readstring(opb, &mut comment, len);
            vc.user_comments.push(comment);
            i += 1;
        }
        if oggpack_read(opb, 1) != 1 {
            break 'err_out; /* EOP check */
        }

        return 0;
    }
    vorbis_comment_clear(vc);
    OV_EBADHEADER
}

/* all of the real encoding details are here.  The modes, books,
everything */
/// Translation of `_vorbis_unpack_books()`.
fn _vorbis_unpack_books(vi: &mut VorbisInfo, opb: &mut OggpackBuffer<'_>) -> i32 {
    let channels = vi.channels;
    'err_out: {
        let ci = vi.codec_setup.as_mut().unwrap();

        /* codebooks */
        ci.books = oggpack_read(opb, 8) as i32 + 1;
        if ci.books <= 0 {
            break 'err_out;
        }
        for i in 0..ci.books as usize {
            ci.book_param[i] = vorbis_staticbook_unpack(opb);
            if ci.book_param[i].is_none() {
                break 'err_out;
            }
        }

        /* time backend settings; hooks are unused */
        {
            let times = oggpack_read(opb, 6) as i32 + 1;
            if times <= 0 {
                break 'err_out;
            }
            for _ in 0..times {
                let test = oggpack_read(opb, 16) as i32;
                if test < 0 || test >= VI_TIMEB {
                    break 'err_out;
                }
            }
        }

        /* floor backend settings */
        ci.floors = oggpack_read(opb, 6) as i32 + 1;
        if ci.floors <= 0 {
            break 'err_out;
        }
        for i in 0..ci.floors as usize {
            ci.floor_type[i] = oggpack_read(opb, 16) as i32;
            if ci.floor_type[i] < 0 || ci.floor_type[i] >= VI_FLOORB {
                break 'err_out;
            }
            ci.floor_param[i] = match ci.floor_type[i] {
                0 => floor0_unpack(ci, opb).map(VorbisInfoFloor::Floor0),
                _ => floor1_unpack(ci, opb).map(VorbisInfoFloor::Floor1),
            };
            if ci.floor_param[i].is_none() {
                break 'err_out;
            }
        }

        /* residue backend settings */
        ci.residues = oggpack_read(opb, 6) as i32 + 1;
        if ci.residues <= 0 {
            break 'err_out;
        }
        for i in 0..ci.residues as usize {
            ci.residue_type[i] = oggpack_read(opb, 16) as i32;
            if ci.residue_type[i] < 0 || ci.residue_type[i] >= VI_RESB {
                break 'err_out;
            }
            ci.residue_param[i] = res0_unpack(ci, opb);
            if ci.residue_param[i].is_none() {
                break 'err_out;
            }
        }

        /* map backend settings */
        ci.maps = oggpack_read(opb, 6) as i32 + 1;
        if ci.maps <= 0 {
            break 'err_out;
        }
        for i in 0..ci.maps as usize {
            ci.map_type[i] = oggpack_read(opb, 16) as i32;
            if ci.map_type[i] < 0 || ci.map_type[i] >= VI_MAPB {
                break 'err_out;
            }
            ci.map_param[i] = mapping0_unpack(channels, ci, opb);
            if ci.map_param[i].is_none() {
                break 'err_out;
            }
        }

        /* mode settings */
        ci.modes = oggpack_read(opb, 6) as i32 + 1;
        if ci.modes <= 0 {
            break 'err_out;
        }
        for i in 0..ci.modes as usize {
            let mode = VorbisInfoMode {
                blockflag: oggpack_read(opb, 1) as i32,
                windowtype: oggpack_read(opb, 16) as i32,
                transformtype: oggpack_read(opb, 16) as i32,
                mapping: oggpack_read(opb, 8) as i32,
            };
            ci.mode_param[i] = Some(mode);

            if mode.windowtype >= VI_WINDOWB {
                break 'err_out;
            }
            if mode.transformtype >= VI_WINDOWB {
                break 'err_out;
            }
            if mode.mapping >= ci.maps {
                break 'err_out;
            }
            if mode.mapping < 0 {
                break 'err_out;
            }
        }

        if oggpack_read(opb, 1) != 1 {
            break 'err_out; /* top level EOP check */
        }

        return 0;
    }
    vorbis_info_clear(vi);
    OV_EBADHEADER
}

/* Is this packet a vorbis ID header? */
/// Translation of `vorbis_synthesis_idheader()` (`op` is the packet's
/// data and its `b_o_s` flag).
pub(crate) fn vorbis_synthesis_idheader(packet: &[u8], b_o_s: i64) -> i32 {
    let mut opb = OggpackBuffer::default();
    let mut buffer = [0u8; 6];

    oggpack_readinit(&mut opb, packet, packet.len() as i32);

    if b_o_s == 0 {
        return 0; /* Not the initial packet */
    }

    if oggpack_read(&mut opb, 8) != 1 {
        return 0; /* not an ID header */
    }

    _v_readstring(&mut opb, &mut buffer, 6);
    if &buffer != b"vorbis" {
        return 0; /* not vorbis */
    }

    1
}

/* The Vorbis header is in three packets; the initial small packet in
the first page that identifies basic parameters, a second packet
with bitstream comments and a third packet that holds the
codebook. */

/// Translation of `vorbis_synthesis_headerin()` (`op` is the packet's
/// data and its `b_o_s` flag).
pub(crate) fn vorbis_synthesis_headerin(
    vi: &mut VorbisInfo,
    vc: &mut VorbisComment,
    packet: &[u8],
    b_o_s: i64,
) -> i32 {
    let mut opb = OggpackBuffer::default();

    oggpack_readinit(&mut opb, packet, packet.len() as i32);

    /* Which of the three types of header is this? */
    /* Also verify header-ness, vorbis */
    {
        let mut buffer = [0u8; 6];
        let packtype = oggpack_read(&mut opb, 8) as i32;
        _v_readstring(&mut opb, &mut buffer, 6);
        if &buffer != b"vorbis" {
            /* not a vorbis header */
            return OV_ENOTVORBIS;
        }
        match packtype {
            0x01 => {
                /* least significant *bit* is read first */
                if b_o_s == 0 {
                    /* Not the initial packet */
                    return OV_EBADHEADER;
                }
                if vi.rate != 0 {
                    /* previously initialized info header */
                    return OV_EBADHEADER;
                }

                _vorbis_unpack_info(vi, &mut opb)
            }
            0x03 => {
                /* least significant *bit* is read first */
                if vi.rate == 0 {
                    /* um... we didn't get the initial header */
                    return OV_EBADHEADER;
                }
                if vc.vendor_set {
                    /* previously initialized comment header */
                    return OV_EBADHEADER;
                }

                _vorbis_unpack_comment(vc, &mut opb)
            }
            0x05 => {
                /* least significant *bit* is read first */
                if vi.rate == 0 || !vc.vendor_set {
                    /* um... we didn;t get the initial header or comments yet */
                    return OV_EBADHEADER;
                }
                let Some(ci) = &vi.codec_setup else {
                    /* improperly initialized vorbis_info */
                    return OV_EFAULT;
                };
                if ci.books > 0 {
                    /* previously initialized setup header */
                    return OV_EBADHEADER;
                }

                _vorbis_unpack_books(vi, &mut opb)
            }
            _ => {
                /* Not a valid vorbis header type */
                OV_EBADHEADER
            }
        }
    }
}
