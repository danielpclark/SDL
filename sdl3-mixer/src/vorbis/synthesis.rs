// Rust translation of lib/synthesis.c and lib/registry.c from libvorbis
// 1.3.7 (https://xiph.org/vorbis/, at the revision SDL_mixer's
// external/vorbis pins).
// THE OggVorbis SOURCE CODE IS (C) COPYRIGHT 1994-2015
// by the Xiph.Org Foundation https://xiph.org/
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! function: single-block PCM synthesis
//!
//! registry.c's tables of backends (`_floor_P`, `_residue_P`,
//! `_mapping_P`) are the matches on the floor, residue and mapping types
//! where they're used.

use super::mapping0::mapping0_inverse;
use super::sharedbook::ov_ilog;
use super::*;
use crate::ogg::bitwise::*;

/// Translation of `vorbis_synthesis()`: `packet` is the packet's data,
/// and `granulepos`, `packetno` and `e_o_s` its fields.
pub(crate) fn vorbis_synthesis(
    vb: &mut VorbisBlock,
    vd: &mut VorbisDspState,
    vi: &VorbisInfo,
    packet: &[u8],
    granulepos: i64,
    packetno: i64,
    e_o_s: i64,
) -> i32 {
    let Some(b) = vd.backend_state.as_deref_mut() else {
        return OV_EBADPACKET;
    };
    let Some(ci) = vi.codec_setup.as_deref() else {
        return OV_EBADPACKET;
    };
    let mut opb = OggpackBuffer::default();
    let mode: i32;

    /* first things first.  Make sure decode is ready */
    /* (_vorbis_block_ripcord(vb): the block's storage is its vectors) */
    oggpack_readinit(&mut opb, packet, packet.len() as i32);

    /* Check the packet type */
    if oggpack_read(&mut opb, 1) != 0 {
        /* Oops.  This is not an audio data packet */
        return OV_ENOTAUDIO;
    }

    /* read our mode and pre/post windowsize */
    mode = oggpack_read(&mut opb, b.modebits) as i32;
    if mode == -1 {
        return OV_EBADPACKET;
    }

    vb.mode = mode;
    let Some(mode_param) = ci.mode_param[mode as usize] else {
        return OV_EBADPACKET;
    };

    vb.W = mode_param.blockflag as i64;
    if vb.W != 0 {
        /* this doesn;t get mapped through mode selection as it's used
        only for window selection */
        vb.lW = oggpack_read(&mut opb, 1);
        vb.nW = oggpack_read(&mut opb, 1);
        if vb.nW == -1 {
            return OV_EBADPACKET;
        }
    } else {
        vb.lW = 0;
        vb.nW = 0;
    }

    /* more setup */
    vb.granulepos = granulepos;
    vb.sequence = packetno;
    vb.eofflag = e_o_s as i32;

    /* alloc pcm passback storage */
    vb.pcmend = ci.blocksizes[vb.W as usize] as i32;
    vb.pcm_null = false;
    vb.pcm.resize(vi.channels as usize, Vec::new());
    for pcm in vb.pcm.iter_mut() {
        pcm.resize(vb.pcmend as usize, 0.0);
    }

    /* unpack_header enforces range checking */
    let mapping = mode_param.mapping as usize;
    let _type = ci.map_type[mapping];

    mapping0_inverse(
        vb,
        &mut opb,
        vi,
        b,
        ci.map_param[mapping].as_deref().unwrap(),
    )
}

/* used to track pcm position without actually performing decode.
Useful for sequential 'fast forward' */
/// Translation of `vorbis_synthesis_trackonly()`.
pub(crate) fn vorbis_synthesis_trackonly(
    vb: &mut VorbisBlock,
    vd: &VorbisDspState,
    vi: &VorbisInfo,
    packet: &[u8],
    granulepos: i64,
    packetno: i64,
    e_o_s: i64,
) -> i32 {
    let b = vd.backend_state.as_deref().unwrap();
    let ci = vi.codec_setup.as_deref().unwrap();
    let mut opb = OggpackBuffer::default();
    let mode: i32;

    /* first things first.  Make sure decode is ready */
    oggpack_readinit(&mut opb, packet, packet.len() as i32);

    /* Check the packet type */
    if oggpack_read(&mut opb, 1) != 0 {
        /* Oops.  This is not an audio data packet */
        return OV_ENOTAUDIO;
    }

    /* read our mode and pre/post windowsize */
    mode = oggpack_read(&mut opb, b.modebits) as i32;
    if mode == -1 {
        return OV_EBADPACKET;
    }

    vb.mode = mode;
    let Some(mode_param) = ci.mode_param[mode as usize] else {
        return OV_EBADPACKET;
    };

    vb.W = mode_param.blockflag as i64;
    if vb.W != 0 {
        vb.lW = oggpack_read(&mut opb, 1);
        vb.nW = oggpack_read(&mut opb, 1);
        if vb.nW == -1 {
            return OV_EBADPACKET;
        }
    } else {
        vb.lW = 0;
        vb.nW = 0;
    }

    /* more setup */
    vb.granulepos = granulepos;
    vb.sequence = packetno;
    vb.eofflag = e_o_s as i32;

    /* no pcm */
    vb.pcmend = 0;
    vb.pcm_null = true;

    0
}

/// Translation of `vorbis_packet_blocksize()`.
pub(crate) fn vorbis_packet_blocksize(vi: &VorbisInfo, packet: &[u8]) -> i64 {
    let Some(ci) = vi.codec_setup.as_deref() else {
        /* codec setup not properly intialized */
        return OV_EFAULT as i64;
    };
    if ci.modes <= 0 {
        /* codec setup not properly intialized */
        return OV_EFAULT as i64;
    }
    let mut opb = OggpackBuffer::default();
    let mode: i32;

    oggpack_readinit(&mut opb, packet, packet.len() as i32);

    /* Check the packet type */
    if oggpack_read(&mut opb, 1) != 0 {
        /* Oops.  This is not an audio data packet */
        return OV_ENOTAUDIO as i64;
    }

    /* read our mode and pre/post windowsize */
    mode = oggpack_read(&mut opb, ov_ilog((ci.modes - 1) as u32)) as i32;
    if mode == -1 {
        return OV_EBADPACKET as i64;
    }
    let Some(mode_param) = ci.mode_param[mode as usize] else {
        return OV_EBADPACKET as i64;
    };
    ci.blocksizes[mode_param.blockflag as usize]
}

/// Translation of `vorbis_synthesis_halfrate_p()`.
pub(crate) fn vorbis_synthesis_halfrate_p(vi: &VorbisInfo) -> i32 {
    /* (vorbisfile never asks a cleared info, which C would crash on) */
    vi.codec_setup.as_deref().map_or(0, |ci| ci.halfrate_flag)
}
