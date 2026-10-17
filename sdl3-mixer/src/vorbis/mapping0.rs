// Rust translation of the decoding half of lib/mapping0.c from libvorbis
// 1.3.7 (https://xiph.org/vorbis/, at the revision SDL_mixer's
// external/vorbis pins).
// THE OggVorbis SOURCE CODE IS (C) COPYRIGHT 1994-2010
// by the Xiph.Org Foundation https://xiph.org/
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! function: channel mapping 0 implementation
//!
//! `mapping0_pack()` and `mapping0_forward()` are the encoder's and
//! aren't translated.

/* simplistic, wasteful way of doing this (unique lookup for each
mode/submapping); there should be a central repository for
identical lookups.  That will require minor work, so I'm putting it
off as low priority.

Why a lookup for each backend in a given mode?  Because the
blocksize is set by the mode, and low backend lookups may require
parameters from other areas of the mode/mapping */

use super::floor0::{floor0_inverse1, floor0_inverse2};
use super::floor1::{floor1_inverse1, floor1_inverse2};
use super::mdct::mdct_backward;
use super::res0::{res0_inverse, res1_inverse, res2_inverse};
use super::sharedbook::ov_ilog;
use super::*;
use crate::ogg::bitwise::*;

/// Translation of `vorbis_info_mapping0` (backends.h).
#[derive(Clone, Copy, Debug)]
pub(crate) struct VorbisInfoMapping0 {
    pub submaps: i32,          /* <= 16 */
    pub chmuxlist: [i32; 256], /* up to 256 channels in a Vorbis stream */

    pub floorsubmap: [i32; 16],   /* [mux] submap to floors */
    pub residuesubmap: [i32; 16], /* [mux] submap to residue */

    pub coupling_steps: i32,
    pub coupling_mag: [i32; 256],
    pub coupling_ang: [i32; 256],
}

impl Default for VorbisInfoMapping0 {
    fn default() -> Self {
        VorbisInfoMapping0 {
            submaps: 0,
            chmuxlist: [0; 256],
            floorsubmap: [0; 16],
            residuesubmap: [0; 16],
            coupling_steps: 0,
            coupling_mag: [0; 256],
            coupling_ang: [0; 256],
        }
    }
}

/* also responsible for range checking */
/// Translation of `mapping0_unpack()` (`channels` is the stream's
/// `vi->channels`).
pub(crate) fn mapping0_unpack(
    channels: i32,
    ci: &CodecSetupInfo,
    opb: &mut OggpackBuffer<'_>,
) -> Option<Box<VorbisInfoMapping0>> {
    let mut b: i32;
    let mut info = Box::<VorbisInfoMapping0>::default();
    if channels <= 0 {
        return None;
    }

    b = oggpack_read(opb, 1) as i32;
    if b < 0 {
        return None;
    }
    if b != 0 {
        info.submaps = oggpack_read(opb, 4) as i32 + 1;
        if info.submaps <= 0 {
            return None;
        }
    } else {
        info.submaps = 1;
    }

    b = oggpack_read(opb, 1) as i32;
    if b < 0 {
        return None;
    }
    if b != 0 {
        info.coupling_steps = oggpack_read(opb, 8) as i32 + 1;
        if info.coupling_steps <= 0 {
            return None;
        }
        for i in 0..info.coupling_steps as usize {
            /* vi->channels > 0 is enforced in the caller */
            let testM = oggpack_read(opb, ov_ilog((channels - 1) as u32)) as i32;
            info.coupling_mag[i] = testM;
            let testA = oggpack_read(opb, ov_ilog((channels - 1) as u32)) as i32;
            info.coupling_ang[i] = testA;

            if testM < 0 || testA < 0 || testM == testA || testM >= channels || testA >= channels {
                return None;
            }
        }
    }

    if oggpack_read(opb, 2) != 0 {
        return None; /* 2,3:reserved */
    }

    if info.submaps > 1 {
        for i in 0..channels as usize {
            info.chmuxlist[i] = oggpack_read(opb, 4) as i32;
            if info.chmuxlist[i] >= info.submaps || info.chmuxlist[i] < 0 {
                return None;
            }
        }
    }
    for i in 0..info.submaps as usize {
        oggpack_read(opb, 8); /* time submap unused */
        info.floorsubmap[i] = oggpack_read(opb, 8) as i32;
        if info.floorsubmap[i] >= ci.floors || info.floorsubmap[i] < 0 {
            return None;
        }
        info.residuesubmap[i] = oggpack_read(opb, 8) as i32;
        if info.residuesubmap[i] >= ci.residues || info.residuesubmap[i] < 0 {
            return None;
        }
    }

    Some(info)
}

/// Translation of `mapping0_inverse()`.
pub(crate) fn mapping0_inverse(
    vb: &mut VorbisBlock,
    opb: &mut OggpackBuffer<'_>,
    vi: &VorbisInfo,
    b: &mut PrivateState,
    info: &VorbisInfoMapping0,
) -> i32 {
    let ci = vi.codec_setup.as_deref().unwrap();
    let fullbooks = ci.fullbooks.as_deref().unwrap();

    let channels = vi.channels as usize;
    vb.pcmend = ci.blocksizes[vb.W as usize] as i32;
    let n = vb.pcmend as i64;

    let mut pcmbundle: Vec<usize> = vec![0; channels];
    let mut zerobundle: Vec<i32> = vec![0; channels];

    let mut nonzero: Vec<i32> = vec![0; channels];
    let mut floormemo: Vec<Option<FloorMemo>> = (0..channels).map(|_| None).collect();

    /* recover the spectral envelope; store it in the PCM vector for now */
    for i in 0..channels {
        let submap = info.chmuxlist[i] as usize;
        let floor = info.floorsubmap[submap] as usize;
        floormemo[i] = match &b.flr[floor] {
            VorbisLookFloor::Floor0(look) => floor0_inverse1(opb, ci, look),
            VorbisLookFloor::Floor1(look) => floor1_inverse1(opb, ci, look),
        };
        if floormemo[i].is_some() {
            nonzero[i] = 1;
        } else {
            nonzero[i] = 0;
        }
        vb.pcm[i][..(n / 2) as usize].fill(0.0);
    }

    /* channel coupling can 'dirty' the nonzero listing */
    for i in 0..info.coupling_steps as usize {
        if nonzero[info.coupling_mag[i] as usize] != 0
            || nonzero[info.coupling_ang[i] as usize] != 0
        {
            nonzero[info.coupling_mag[i] as usize] = 1;
            nonzero[info.coupling_ang[i] as usize] = 1;
        }
    }

    /* recover the residue into our working vectors */
    for i in 0..info.submaps {
        let mut ch_in_bundle = 0usize;
        for j in 0..channels {
            if info.chmuxlist[j] == i {
                if nonzero[j] != 0 {
                    zerobundle[ch_in_bundle] = 1;
                } else {
                    zerobundle[ch_in_bundle] = 0;
                }
                pcmbundle[ch_in_bundle] = j;
                ch_in_bundle += 1;
            }
        }

        let r = info.residuesubmap[i as usize] as usize;
        let inverse = match ci.residue_type[r] {
            0 => res0_inverse,
            1 => res1_inverse,
            _ => res2_inverse,
        };
        inverse(
            vb,
            opb,
            fullbooks,
            &b.residue[r],
            &pcmbundle[..ch_in_bundle],
            &zerobundle[..ch_in_bundle],
            ch_in_bundle as i32,
        );
    }

    /* channel coupling */
    for i in (0..info.coupling_steps as usize).rev() {
        let m = info.coupling_mag[i] as usize;
        let a = info.coupling_ang[i] as usize;

        for j in 0..(n / 2) as usize {
            let mag = vb.pcm[m][j];
            let ang = vb.pcm[a][j];

            if mag > 0.0 {
                if ang > 0.0 {
                    vb.pcm[m][j] = mag;
                    vb.pcm[a][j] = mag - ang;
                } else {
                    vb.pcm[a][j] = mag;
                    vb.pcm[m][j] = mag + ang;
                }
            } else if ang > 0.0 {
                vb.pcm[m][j] = mag;
                vb.pcm[a][j] = mag + ang;
            } else {
                vb.pcm[a][j] = mag;
                vb.pcm[m][j] = mag - ang;
            }
        }
    }

    /* compute and apply spectral envelope */
    for i in 0..channels {
        let submap = info.chmuxlist[i] as usize;
        let floor = info.floorsubmap[submap] as usize;
        let mut pcm = std::mem::take(&mut vb.pcm[i]);
        match (&mut b.flr[floor], &mut floormemo[i]) {
            (VorbisLookFloor::Floor0(look), memo) => {
                let memo = match memo {
                    Some(FloorMemo::Floor0(lsp)) => Some(&mut lsp[..]),
                    _ => None,
                };
                floor0_inverse2(vb, ci, look, memo, &mut pcm);
            }
            (VorbisLookFloor::Floor1(look), memo) => {
                let memo = match memo {
                    Some(FloorMemo::Floor1(fit_value)) => Some(&fit_value[..]),
                    _ => None,
                };
                floor1_inverse2(vb, ci, look, memo, &mut pcm);
            }
        }
        vb.pcm[i] = pcm;
    }

    /* transform the PCM data; takes PCM vector, vb; modifies PCM vector */
    /* only MDCT right now.... */
    for i in 0..channels {
        mdct_backward(&b.transform[vb.W as usize], &mut vb.pcm[i]);
    }

    /* all done! */
    0
}
