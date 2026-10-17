// Rust translation of lib/floor0.c from libvorbis 1.3.7
// (https://xiph.org/vorbis/, at the revision SDL_mixer's external/vorbis
// pins).
// THE OggVorbis SOURCE CODE IS (C) COPYRIGHT 1994-2015
// by the Xiph.Org Foundation https://xiph.org/
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! function: floor backend 0 implementation
//!
//! The look keeps a copy of its setup (C points at it in the
//! `codec_setup_info`).

use super::codebook::vorbis_book_decodev_set;
use super::lsp::vorbis_lsp_to_curve;
use super::sharedbook::ov_ilog;
use super::*;
use crate::ogg::bitwise::*;

/// Translation of `vorbis_info_floor0` (backends.h).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct VorbisInfoFloor0 {
    pub order: i32,
    pub rate: i64,
    pub barkmap: i64,

    pub ampbits: i32,
    pub ampdB: i32,

    pub numbooks: i32, /* <= 16 */
    pub books: [i32; 16],
    /* (lessthan and greaterthan are encode-only config setting hacks for libvorbis) */
}

/// Translation of `vorbis_look_floor0`.
#[derive(Debug, Default)]
pub(crate) struct VorbisLookFloor0 {
    pub ln: i32,
    pub m: i32,
    pub linearmap: [Option<Vec<i32>>; 2],
    pub n: [i32; 2],

    pub vi: VorbisInfoFloor0,
    /* (bits and frames are never counted) */
}

/***********************************************/

/// Translation of `floor0_unpack()`.
pub(crate) fn floor0_unpack(
    ci: &CodecSetupInfo,
    opb: &mut OggpackBuffer<'_>,
) -> Option<Box<VorbisInfoFloor0>> {
    let mut info = Box::<VorbisInfoFloor0>::default();
    info.order = oggpack_read(opb, 8) as i32;
    info.rate = oggpack_read(opb, 16);
    info.barkmap = oggpack_read(opb, 16);
    info.ampbits = oggpack_read(opb, 6) as i32;
    info.ampdB = oggpack_read(opb, 8) as i32;
    info.numbooks = oggpack_read(opb, 4) as i32 + 1;

    if info.order < 1 {
        return None;
    }
    if info.rate < 1 {
        return None;
    }
    if info.barkmap < 1 {
        return None;
    }
    if info.numbooks < 1 {
        return None;
    }

    for j in 0..info.numbooks as usize {
        info.books[j] = oggpack_read(opb, 8) as i32;
        if info.books[j] < 0 || info.books[j] >= ci.books {
            return None;
        }
        let book = ci.book_param[info.books[j] as usize].as_ref()?;
        if book.maptype == 0 {
            return None;
        }
        if book.dim < 1 {
            return None;
        }
    }
    Some(info)
}

/* initialize Bark scale and normalization lookups.  We could do this
with static tables, but Vorbis allows a number of possible
combinations, so it's best to do it computationally.

The below is authoritative in terms of defining scale mapping.
Note that the scale depends on the sampling rate as well as the
linear block and mapping sizes */

/// Translation of `floor0_map_lazy_init()`.
fn floor0_map_lazy_init(
    vb: &VorbisBlock,
    ci: &CodecSetupInfo,
    info: &VorbisInfoFloor0,
    look: &mut VorbisLookFloor0,
) {
    if look.linearmap[vb.W as usize].is_none() {
        let W = vb.W as usize;
        let n = (ci.blocksizes[W] / 2) as i32;

        /* we choose a scaling constant so that:
        floor(bark(rate/2-1)*C)=mapped-1
        floor(bark(rate/2)*C)=mapped */
        let scale: f32 = (look.ln as f64 / toBARK(info.rate as f32 / 2.0)) as f32;

        /* the mapping from a linear scale to a smaller bark scale is
        straightforward.  We do *not* make sure that the linear mapping
        does not skip bark-scale bins; the decoder simply skips them and
        the encoder may do what it wishes in filling them.  They're
        necessary in some mapping combinations to keep the scale spacing
        accurate */
        let mut linearmap = vec![0i32; n as usize + 1];
        let mut j = 0usize;
        while j < n as usize {
            let mut val = c_int(sdl3::stdlib::math::floor(
                toBARK((info.rate as f32 / 2.0) / n as f32 * j as f32) * scale as f64,
            )); /* bark numbers represent band edges */
            if val >= look.ln {
                val = look.ln - 1; /* guard against the approximation */
            }
            linearmap[j] = val;
            j += 1;
        }
        linearmap[j] = -1;
        look.linearmap[W] = Some(linearmap);
        look.n[W] = n;
    }
}

/// Translation of `floor0_look()`.
pub(crate) fn floor0_look(info: &VorbisInfoFloor0) -> Box<VorbisLookFloor0> {
    Box::new(VorbisLookFloor0 {
        m: info.order,
        ln: info.barkmap as i32,
        vi: *info,
        linearmap: [None, None],
        n: [0; 2],
    })
}

/// Translation of `floor0_inverse1()`.
pub(crate) fn floor0_inverse1(
    opb: &mut OggpackBuffer<'_>,
    ci: &CodecSetupInfo,
    look: &VorbisLookFloor0,
) -> Option<FloorMemo> {
    let info = &look.vi;

    let ampraw = oggpack_read(opb, info.ampbits) as i32;
    if ampraw > 0 {
        /* also handles the -1 out of data case */
        let maxval: i64 = 1i32.wrapping_shl(info.ampbits as u32).wrapping_sub(1) as i64; /* (x86's int shift) */
        let amp: f32 = ampraw as f32 / maxval as f32 * info.ampdB as f32;
        let booknum = oggpack_read(opb, ov_ilog(info.numbooks as u32)) as i32;

        if booknum != -1 && booknum < info.numbooks {
            /* be paranoid */
            let b = &ci.fullbooks.as_deref().unwrap()[info.books[booknum as usize] as usize];
            let mut last: f32 = 0.0;

            /* the additional b->dim is a guard against any possible stack
            smash; b->dim is provably more than we can overflow the
            vector */
            let mut lsp = vec![0f32; (look.m as i64 + b.dim + 1) as usize];

            if vorbis_book_decodev_set(b, &mut lsp, opb, look.m) == -1 {
                return None;
            }
            let mut j = 0usize;
            while j < look.m as usize {
                let mut k = 0i64;
                while j < look.m as usize && k < b.dim {
                    lsp[j] += last;
                    k += 1;
                    j += 1;
                }
                last = lsp[j - 1];
            }

            lsp[look.m as usize] = amp;
            return Some(FloorMemo::Floor0(lsp));
        }
    }
    /* eop: */
    None
}

/// Translation of `floor0_inverse2()`.
pub(crate) fn floor0_inverse2(
    vb: &VorbisBlock,
    ci: &CodecSetupInfo,
    look: &mut VorbisLookFloor0,
    memo: Option<&mut [f32]>,
    out: &mut [f32],
) -> i32 {
    let info = look.vi;

    floor0_map_lazy_init(vb, ci, &info, look);

    let W = vb.W as usize;
    if let Some(lsp) = memo {
        let amp = lsp[look.m as usize];

        /* take the coefficients back to a spectral envelope curve */
        vorbis_lsp_to_curve(
            out,
            look.linearmap[W].as_deref().unwrap(),
            look.n[W],
            look.ln,
            lsp,
            look.m,
            amp,
            info.ampdB as f32,
        );
        return 1;
    }
    for v in out.iter_mut().take(look.n[W] as usize) {
        *v = 0.0;
    }
    0
}
