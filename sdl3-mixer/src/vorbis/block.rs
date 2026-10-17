// Rust translation of the synthesis half of lib/block.c from libvorbis
// 1.3.7 (https://xiph.org/vorbis/, at the revision SDL_mixer's
// external/vorbis pins).
// THE OggVorbis SOURCE CODE IS (C) COPYRIGHT 1994-2015
// by the Xiph.Org Foundation https://xiph.org/
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! function: PCM data vector blocking, windowing and dis/reassembly
//!
//! Handle windowing, overlap-add, etc of the PCM vectors.  This is made
//! more amusing by Vorbis' current two allowed block sizes.
//!
//! The analysis side (`vorbis_analysis_init()`, `vorbis_analysis_buffer()`,
//! `vorbis_analysis_wrote()`, `vorbis_analysis_blockout()`,
//! `_preextrapolate_helper()`, and the encoder's half of
//! `_vds_shared_init()`), the block's local storage allocator
//! (`_vorbis_block_alloc()`, `_vorbis_block_ripcord()`: the block's
//! vectors are ordinary ones here), and `vorbis_synthesis_lapout()` and
//! `vorbis_window()`, which only vorbisfile's lapped seeks and crosslap
//! use, aren't translated.

/* pcm accumulator examples (not exhaustive):

 <-------------- lW ---------------->
                   <--------------- W ---------------->
:            .....|.....       _______________         |
:        .'''     |     '''_---      |       |\        |
:.....'''         |_____--- '''......|       | \_______|
:.................|__________________|_______|__|______|
                  |<------ Sl ------>|      > Sr <     |endW
                  |beginSl           |endSl  |  |endSr
                  |beginW            |endlW  |beginSr


                      |< lW >|
                   <--------------- W ---------------->
                  |   |  ..  ______________            |
                  |   | '  `/        |     ---_        |
                  |___.'___/`.       |         ---_____|
                  |_______|__|_______|_________________|
                  |      >|Sl|<      |<------ Sr ----->|endW
                  |       |  |endSl  |beginSr          |endSr
                  |beginW |  |endlW
                  mult[0] |beginSl                     mult[n]

 <-------------- lW ----------------->
                          |<--W-->|
:            ..............  ___  |   |
:        .'''             |`/   \ |   |
:.....'''                 |/`....\|...|
:.........................|___|___|___|
                          |Sl |Sr |endW
                          |   |   |endSr
                          |   |beginSr
                          |   |endSl
                          |beginSl
                          |beginW
*/

use super::floor0::floor0_look;
use super::floor1::floor1_look;
use super::mdct::mdct_init;
use super::res0::res0_look;
use super::sharedbook::{ov_ilog, vorbis_book_clear, vorbis_book_init_decode};
use super::window::_vorbis_window_get;
use super::*;

/* block abstraction setup *********************************************/

/// Translation of `vorbis_block_init()` (synthesis).
pub(crate) fn vorbis_block_init(vb: &mut VorbisBlock) -> i32 {
    *vb = VorbisBlock::default();
    0
}

/// Translation of `vorbis_block_clear()`.
pub(crate) fn vorbis_block_clear(vb: &mut VorbisBlock) -> i32 {
    *vb = VorbisBlock::default();
    0
}

/* Analysis side code, but directly related to blocking.  Thus it's
here and not in analysis.c (which is for analysis transforms only).
The init is here because some of it is shared */

/// Translation of `_vds_shared_init()` for decoding (`encp` is 0).
///
/// FIXME (upstream): when a codebook fails to set up, the static books are
/// freed but the half set-up `fullbooks` stay in the `codec_setup_info`,
/// so the next `vorbis_synthesis_init()` (vorbisfile tries again on the
/// next read or seek) skips the book setup and decodes with the zeroed
/// books, which hangs floor 0 (a book of no dimensions never advances its
/// loop) and divides by zero in residue 2. Here (and in the reference C,
/// patched the same way) the `fullbooks` go too, so that every later try
/// fails as the first did.
fn _vds_shared_init(v: &mut VorbisDspState, vi: &mut VorbisInfo, vi_index: usize) -> i32 {
    let channels = vi.channels;
    let Some(ci) = vi.codec_setup.as_deref_mut() else {
        return 1;
    };
    if ci.modes <= 0 || ci.blocksizes[0] < 64 || ci.blocksizes[1] < ci.blocksizes[0] {
        return 1;
    }
    let hs = ci.halfrate_flag;

    *v = VorbisDspState::default();
    let mut b = Box::<PrivateState>::default();

    v.vi = Some(vi_index);
    b.modebits = ov_ilog((ci.modes - 1) as u32);

    /* MDCT is tranform 0 */

    mdct_init(&mut b.transform[0], (ci.blocksizes[0] >> hs) as i32);
    mdct_init(&mut b.transform[1], (ci.blocksizes[1] >> hs) as i32);

    /* Vorbis I uses only window type 0 */
    /* note that the correct computation below is technically:
         b->window[0]=ov_ilog(ci->blocksizes[0]-1)-6;
         b->window[1]=ov_ilog(ci->blocksizes[1]-1)-6;
      but since blocksizes are always powers of two,
      the below is equivalent.
    */
    b.window[0] = ov_ilog(ci.blocksizes[0] as u32) - 7;
    b.window[1] = ov_ilog(ci.blocksizes[1] as u32) - 7;

    v.backend_state = Some(b);

    'abort_books: {
        /* finish the codebooks */
        if ci.fullbooks.is_none() {
            let mut fullbooks: Vec<Codebook> = vec![Codebook::default(); ci.books.max(0) as usize];
            let mut failed = false;
            for i in 0..ci.books as usize {
                let Some(book) = ci.book_param[i].as_deref() else {
                    failed = true;
                    break;
                };
                if vorbis_book_init_decode(&mut fullbooks[i], book) != 0 {
                    failed = true;
                    break;
                }
                /* decode codebooks are now standalone after init */
                ci.book_param[i] = None;
            }
            if failed {
                /* (the fullbooks don't stay: see above) */
                for book in fullbooks.iter_mut() {
                    vorbis_book_clear(book);
                }
                break 'abort_books;
            }
            ci.fullbooks = Some(fullbooks);
        }

        /* initialize the storage vectors. blocksize[1] is small for encode,
        but the correct size for decode */
        v.pcm_storage = ci.blocksizes[1] as i32;
        v.pcm = (0..channels)
            .map(|_| vec![0f32; v.pcm_storage as usize])
            .collect();

        /* all 1 (large block) or 0 (small block) */
        /* explicitly set for the sake of clarity */
        v.lW = 0; /* previous window size */
        v.W = 0; /* current window size */

        /* all vector indexes */
        v.centerW = ci.blocksizes[1] / 2;

        v.pcm_current = v.centerW as i32;

        /* initialize all the backend lookups */
        let b = v.backend_state.as_mut().unwrap();
        b.flr = Vec::with_capacity(ci.floors as usize);
        b.residue = Vec::with_capacity(ci.residues as usize);

        for i in 0..ci.floors as usize {
            b.flr.push(match ci.floor_param[i].as_ref().unwrap() {
                VorbisInfoFloor::Floor0(info) => VorbisLookFloor::Floor0(floor0_look(info)),
                VorbisInfoFloor::Floor1(info) => VorbisLookFloor::Floor1(floor1_look(info)),
            });
        }

        for i in 0..ci.residues as usize {
            let Some(look) = res0_look(ci, ci.residue_param[i].as_ref().unwrap()) else {
                /* (C would crash on the failed allocation) */
                vorbis_dsp_clear(v);
                return -1;
            };
            b.residue.push(look);
        }

        return 0;
    }
    for i in 0..ci.books as usize {
        if ci.book_param[i].is_some() {
            ci.book_param[i] = None;
        }
    }
    vorbis_dsp_clear(v);
    -1
}

/// Translation of `vorbis_dsp_clear()`.
pub(crate) fn vorbis_dsp_clear(v: &mut VorbisDspState) {
    *v = VorbisDspState::default();
}

/// Translation of `vorbis_synthesis_restart()` (`vi` is the state's
/// `v->vi`, if it has one).
pub(crate) fn vorbis_synthesis_restart(v: &mut VorbisDspState, vi: Option<&VorbisInfo>) -> i32 {
    if v.backend_state.is_none() {
        return -1;
    }
    let Some(vi) = vi else {
        return -1;
    };
    let Some(ci) = vi.codec_setup.as_deref() else {
        return -1;
    };
    let hs = ci.halfrate_flag;

    v.centerW = ci.blocksizes[1] >> (hs + 1);
    v.pcm_current = (v.centerW >> hs) as i32;

    v.pcm_returned = -1;
    v.granulepos = -1;
    v.sequence = -1;
    v.eofflag = 0;
    v.backend_state.as_mut().unwrap().sample_count = -1;

    0
}

/// Translation of `vorbis_synthesis_init()`: `vi_index` is what the state
/// records as its `vi` (see [`VorbisDspState`]).
pub(crate) fn vorbis_synthesis_init(
    v: &mut VorbisDspState,
    vi: &mut VorbisInfo,
    vi_index: usize,
) -> i32 {
    if _vds_shared_init(v, vi, vi_index) != 0 {
        vorbis_dsp_clear(v);
        return 1;
    }
    vorbis_synthesis_restart(v, Some(vi));
    0
}

/* Unlike in analysis, the window is only partially applied for each
block.  The time domain envelope is not yet handled at the point of
calling (as it relies on the previous block). */

/// Translation of `vorbis_synthesis_blockin()`.
pub(crate) fn vorbis_synthesis_blockin(
    v: &mut VorbisDspState,
    vi: &VorbisInfo,
    vb: &VorbisBlock,
) -> i32 {
    let ci = vi.codec_setup.as_deref().unwrap();
    let hs = ci.halfrate_flag;

    if v.pcm_current > v.pcm_returned && v.pcm_returned != -1 {
        return OV_EINVAL;
    }

    v.lW = v.W;
    v.W = vb.W;
    v.nW = -1;

    if (v.sequence == -1) || (v.sequence.wrapping_add(1) != vb.sequence) {
        v.granulepos = -1; /* out of sequence; lose count */
        v.backend_state.as_mut().unwrap().sample_count = -1;
    }

    v.sequence = vb.sequence;

    if !vb.pcm_null {
        /* no pcm to process if vorbis_synthesis_trackonly
        was called on block */
        let n = (ci.blocksizes[v.W as usize] >> (hs + 1)) as usize;
        let n0 = (ci.blocksizes[0] >> (hs + 1)) as usize;
        let n1 = (ci.blocksizes[1] >> (hs + 1)) as usize;

        let thisCenter: usize;
        let prevCenter: usize;

        v.glue_bits += vb.glue_bits;
        v.time_bits += vb.time_bits;
        v.floor_bits += vb.floor_bits;
        v.res_bits += vb.res_bits;

        if v.centerW != 0 {
            thisCenter = n1;
            prevCenter = 0;
        } else {
            thisCenter = 0;
            prevCenter = n1;
        }

        /* v->pcm is now used like a two-stage double buffer.  We don't want
        to have to constantly shift *or* adjust memory usage.  Don't
        accept a new block until the old is shifted out */

        let window = v.backend_state.as_ref().unwrap().window;
        for j in 0..vi.channels as usize {
            let pcmj = &mut v.pcm[j];
            let p = &vb.pcm[j];
            /* the overlap/add section */
            if v.lW != 0 {
                if v.W != 0 {
                    /* large/large */
                    let w = _vorbis_window_get(window[1] - hs);
                    let pcm = &mut pcmj[prevCenter..];
                    for i in 0..n1 {
                        pcm[i] = pcm[i] * w[n1 - i - 1] + p[i] * w[i];
                    }
                } else {
                    /* large/small */
                    let w = _vorbis_window_get(window[0] - hs);
                    let pcm = &mut pcmj[prevCenter + n1 / 2 - n0 / 2..];
                    for i in 0..n0 {
                        pcm[i] = pcm[i] * w[n0 - i - 1] + p[i] * w[i];
                    }
                }
            } else if v.W != 0 {
                /* small/large */
                let w = _vorbis_window_get(window[0] - hs);
                let pcm = &mut pcmj[prevCenter..];
                let p = &p[n1 / 2 - n0 / 2..];
                let mut i = 0;
                while i < n0 {
                    pcm[i] = pcm[i] * w[n0 - i - 1] + p[i] * w[i];
                    i += 1;
                }
                while i < n1 / 2 + n0 / 2 {
                    pcm[i] = p[i];
                    i += 1;
                }
            } else {
                /* small/small */
                let w = _vorbis_window_get(window[0] - hs);
                let pcm = &mut pcmj[prevCenter..];
                for i in 0..n0 {
                    pcm[i] = pcm[i] * w[n0 - i - 1] + p[i] * w[i];
                }
            }

            /* the copy section */
            {
                let pcm = &mut pcmj[thisCenter..];
                let p = &p[n..];
                pcm[..n].copy_from_slice(&p[..n]);
            }
        }

        if v.centerW != 0 {
            v.centerW = 0;
        } else {
            v.centerW = n1 as i64;
        }

        /* deal with initial packet state; we do this using the explicit
        pcm_returned==-1 flag otherwise we're sensitive to first block
        being short or long */

        if v.pcm_returned == -1 {
            v.pcm_returned = thisCenter as i32;
            v.pcm_current = thisCenter as i32;
        } else {
            v.pcm_returned = prevCenter as i32;
            v.pcm_current = prevCenter as i32
                + ((ci.blocksizes[v.lW as usize] / 4 + ci.blocksizes[v.W as usize] / 4) >> hs)
                    as i32;
        }
    }

    /* track the frame number... This is for convenience, but also
    making sure our last packet doesn't end with added padding.  If
    the last packet is partial, the number of samples we'll have to
    return will be past the vb->granulepos.

    This is not foolproof!  It will be confused if we begin
    decoding at the last page after a seek or hole.  In that case,
    we don't have a starting point to judge where the last frame
    is.  For this reason, vorbisfile will always try to make sure
    it reads the last two marked pages in proper sequence */

    let b = v.backend_state.as_mut().unwrap();
    if b.sample_count == -1 {
        b.sample_count = 0;
    } else {
        b.sample_count = b
            .sample_count
            .wrapping_add(ci.blocksizes[v.lW as usize] / 4 + ci.blocksizes[v.W as usize] / 4);
    }

    if v.granulepos == -1 {
        if vb.granulepos != -1 {
            /* only set if we have a position to set to */

            v.granulepos = vb.granulepos;

            /* is this a short page? */
            if b.sample_count > v.granulepos {
                /* corner case; if this is both the first and last audio page,
                then spec says the end is cut, not beginning */
                let mut extra: i64 = b.sample_count.wrapping_sub(vb.granulepos);

                /* we use ogg_int64_t for granule positions because a
                uint64 isn't universally available.  Unfortunately,
                that means granposes can be 'negative' and result in
                extra being negative */
                if extra < 0 {
                    extra = 0;
                }

                if vb.eofflag != 0 {
                    /* trim the end */
                    /* no preceding granulepos; assume we started at zero (we'd
                    have to in a short single-page stream) */
                    /* granulepos could be -1 due to a seek, but that would result
                    in a long count, not short count */

                    /* Guard against corrupt/malicious frames that set EOP and
                    a backdated granpos; don't rewind more samples than we
                    actually have */
                    if extra > ((v.pcm_current - v.pcm_returned) << hs) as i64 {
                        extra = ((v.pcm_current - v.pcm_returned) << hs) as i64;
                    }

                    v.pcm_current = (v.pcm_current as i64 - (extra >> hs)) as i32;
                } else {
                    /* trim the beginning */
                    v.pcm_returned = (v.pcm_returned as i64 + (extra >> hs)) as i32;
                    if v.pcm_returned > v.pcm_current {
                        v.pcm_returned = v.pcm_current;
                    }
                }
            }
        }
    } else {
        v.granulepos = v
            .granulepos
            .wrapping_add(ci.blocksizes[v.lW as usize] / 4 + ci.blocksizes[v.W as usize] / 4);
        if vb.granulepos != -1 && v.granulepos != vb.granulepos {
            if v.granulepos > vb.granulepos {
                let mut extra: i64 = v.granulepos.wrapping_sub(vb.granulepos);

                if extra != 0 && vb.eofflag != 0 {
                    /* partial last frame.  Strip the extra samples off */

                    /* Guard against corrupt/malicious frames that set EOP and
                    a backdated granpos; don't rewind more samples than we
                    actually have */
                    if extra > ((v.pcm_current - v.pcm_returned) << hs) as i64 {
                        extra = ((v.pcm_current - v.pcm_returned) << hs) as i64;
                    }

                    /* we use ogg_int64_t for granule positions because a
                    uint64 isn't universally available.  Unfortunately,
                    that means granposes can be 'negative' and result in
                    extra being negative */
                    if extra < 0 {
                        extra = 0;
                    }

                    v.pcm_current = (v.pcm_current as i64 - (extra >> hs)) as i32;
                } /* else {Shouldn't happen *unless* the bitstream is out of
                  spec.  Either way, believe the bitstream } */
            } /* else {Shouldn't happen *unless* the bitstream is out of
              spec.  Either way, believe the bitstream } */
            v.granulepos = vb.granulepos;
        }
    }

    /* Update, cleanup */

    if vb.eofflag != 0 {
        v.eofflag = 1;
    }
    0
}

/* pcm==NULL indicates we just want the pending samples, no more */
/// Translation of `vorbis_synthesis_pcmout()`: `pcm`, when given, is
/// set to where the samples start in each of the state's `pcm` vectors.
pub(crate) fn vorbis_synthesis_pcmout(v: &VorbisDspState, pcm: Option<&mut usize>) -> i32 {
    if v.pcm_returned > -1 && v.pcm_returned < v.pcm_current {
        if let Some(pcm) = pcm {
            *pcm = v.pcm_returned as usize;
        }
        return v.pcm_current - v.pcm_returned;
    }
    0
}

/// Translation of `vorbis_synthesis_read()`.
pub(crate) fn vorbis_synthesis_read(v: &mut VorbisDspState, n: i32) -> i32 {
    if n != 0 && v.pcm_returned + n > v.pcm_current {
        return OV_EINVAL;
    }
    v.pcm_returned += n;
    0
}
