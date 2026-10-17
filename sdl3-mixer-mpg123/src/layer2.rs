/*
    layer2.c: the layer 2 decoder, root of mpg123

    copyright 1994-2021 by the mpg123 project - free software under the terms of the LGPL 2.1
    see COPYING and AUTHORS files in distribution or http://mpg123.org
    initially written by Michael Hipp

    mpg123 started as mp2 decoder a long time ago...
    part of this file is required for layer 1, too.
*/
// Modified 2026-10-17: translated into Rust from layer2.c in SDL_mixer 3.3.0's libmpg123 1.33.7 (see NOTICE).

use crate::frame::*;
use crate::getbits::{getbits, getbits_fast};
use crate::l12tabs::{GRP_3TAB, GRP_5TAB, GRP_9TAB, LAYER12_TABLE};
use crate::l2tables::{ALLOC_0, ALLOC_1, ALLOC_2, ALLOC_3, ALLOC_4};

/* Stuff  needed for layer I and II. */

// The layer12_table is already in real format (fixed or float), just needs
// a little scaling in the MMX/SSE case.

/// Translation of `INT123_init_layer12_stuff()`.
pub(crate) fn init_layer12_stuff(
    fr: &mut Mpg123Handle<'_>,
    init_table: fn(&mut Mpg123Handle<'_>, usize, usize) -> usize,
) {
    for k in 0..27 {
        // (muls[k] is at GAINPOW2_SIZE + k*64 in the shared array)
        let table = init_table(fr, GAINPOW2_SIZE + k * 64, k);
        fr.gainpow2_muls[table] = 0.0;
    }
}

/// Translation of `INT123_init_layer12_table()`: fills the 63 reals from
/// `table` on, returns the position after them.
pub(crate) fn init_layer12_table(fr: &mut Mpg123Handle<'_>, mut table: usize, m: usize) -> usize {
    for i in 0..63 {
        fr.gainpow2_muls[table] = LAYER12_TABLE[m][i];
        table += 1;
    }
    table
}

/* The rest is the actual decoding of layer II data. */

fn ii_step_one(
    bit_alloc: &mut [u32; 64],
    scale: &mut [i32; 192],
    fr: &mut Mpg123Handle<'_>,
) -> i32 {
    let stereo = fr.stereo - 1;
    let sblimit = fr.ii_sblimit;
    let jsbound = fr.jsbound;
    let sblimit2 = fr.ii_sblimit << stereo;
    let alloc = fr.alloc;
    let mut alloc1 = 0usize;
    let mut scfsi_buf = [0u32; 64];
    let mut step;
    /* Count the bits needed for getbits_fast(). */
    let mut needbits: u32 = 0;
    let scale_bits: [u32; 4] = [18, 12, 6, 12];

    let mut bita = 0usize;
    let mut scfsi = 0usize;
    if stereo != 0 {
        for _i in 0..jsbound {
            step = alloc[alloc1].bits as i32;
            bit_alloc[bita] = getbits(fr, step) as u8 as u32;
            bit_alloc[bita + 1] = getbits(fr, step) as u8 as u32;
            needbits += ((if bit_alloc[bita] != 0 { 1 } else { 0 })
                + (if bit_alloc[bita + 1] != 0 { 1 } else { 0 }))
                * 2;
            bita += 2;
            alloc1 += 1 << step;
        }
        for _i in 0..(sblimit - jsbound) {
            step = alloc[alloc1].bits as i32;
            bit_alloc[bita] = getbits(fr, step) as u8 as u32;
            bit_alloc[bita + 1] = bit_alloc[bita];
            needbits += (if bit_alloc[bita] != 0 { 1 } else { 0 }) * 2 * 2;
            bita += 2;
            alloc1 += 1 << step;
        }
        bita = 0;

        if fr.bits_avail < needbits as i64 {
            // need %u bits, have %li
            return -1;
        }
        for _i in 0..sblimit2 {
            let b = bit_alloc[bita];
            bita += 1;
            if b != 0 {
                scfsi_buf[scfsi] = getbits_fast(fr, 2) as u8 as u32;
                scfsi += 1;
            }
        }
    } else
    /* mono */
    {
        for _i in 0..sblimit {
            step = alloc[alloc1].bits as i32;
            bit_alloc[bita] = getbits(fr, step) as u8 as u32;
            if bit_alloc[bita] != 0 {
                needbits += 2;
            }
            bita += 1;
            alloc1 += 1 << step;
        }
        bita = 0;
        if fr.bits_avail < needbits as i64 {
            return -1;
        }
        for _i in 0..sblimit {
            let b = bit_alloc[bita];
            bita += 1;
            if b != 0 {
                scfsi_buf[scfsi] = getbits_fast(fr, 2) as u8 as u32;
                scfsi += 1;
            }
        }
    }

    needbits = 0;
    bita = 0;
    scfsi = 0;
    for _i in 0..sblimit2 {
        let b = bit_alloc[bita];
        bita += 1;
        if b != 0 {
            needbits += scale_bits[scfsi_buf[scfsi] as usize];
            scfsi += 1;
        }
    }
    if fr.bits_avail < needbits as i64 {
        return -1;
    }

    bita = 0;
    scfsi = 0;
    let mut sc_at = 0usize;
    for _i in 0..sblimit2 {
        let b = bit_alloc[bita];
        bita += 1;
        if b != 0 {
            let s = scfsi_buf[scfsi];
            scfsi += 1;
            match s {
                0 => {
                    scale[sc_at] = getbits_fast(fr, 6) as i32;
                    scale[sc_at + 1] = getbits_fast(fr, 6) as i32;
                    scale[sc_at + 2] = getbits_fast(fr, 6) as i32;
                }
                1 => {
                    let sc = getbits_fast(fr, 6) as i32;
                    scale[sc_at] = sc;
                    scale[sc_at + 1] = sc;
                    scale[sc_at + 2] = getbits_fast(fr, 6) as i32;
                }
                2 => {
                    let sc = getbits_fast(fr, 6) as i32;
                    scale[sc_at] = sc;
                    scale[sc_at + 1] = sc;
                    scale[sc_at + 2] = sc;
                }
                _ => {
                    /* case 3 */
                    scale[sc_at] = getbits_fast(fr, 6) as i32;
                    let sc = getbits_fast(fr, 6) as i32;
                    scale[sc_at + 1] = sc;
                    scale[sc_at + 2] = sc;
                }
            }
            sc_at += 3;
        }
    }

    0
}

/// The grouped sample tables: `table[d1]` of `II_step_two()`.
fn grp_table(d1: i32) -> &'static [u8] {
    match d1 {
        3 => &GRP_3TAB,
        5 => &GRP_5TAB,
        _ => &GRP_9TAB, // (9)
    }
}

fn ii_step_two(
    bit_alloc: &[u32; 64],
    fraction: &mut [[[f32; SBLIMIT]; 4]; 2],
    scale: &[i32; 192],
    fr: &mut Mpg123Handle<'_>,
    x1: usize,
) {
    let stereo = fr.stereo as usize;
    let mut sblimit = fr.ii_sblimit as usize;
    let jsbound = fr.jsbound as usize;
    let alloc = fr.alloc;
    let mut alloc1 = 0usize;
    let mut bita = 0usize;
    let mut sc = 0usize;

    for i in 0..jsbound {
        let step = alloc[alloc1].bits as i32;
        for j in 0..stereo {
            let ba = bit_alloc[bita] as usize;
            bita += 1;
            if ba != 0 {
                let alloc2 = alloc[alloc1 + ba];
                let k = alloc2.bits as i32;
                let d1 = alloc2.d as i32;
                if d1 < 0 {
                    let cm = fr.muls(k as usize, scale[sc + x1] as usize);
                    fraction[j][0][i] = ((getbits(fr, k) as i32 + d1) as f32) * cm;
                    fraction[j][1][i] = ((getbits(fr, k) as i32 + d1) as f32) * cm;
                    fraction[j][2][i] = ((getbits(fr, k) as i32 + d1) as f32) * cm;
                } else {
                    let m = scale[sc + x1] as usize;
                    let idx = getbits(fr, k) as usize;
                    let tab = &grp_table(d1)[idx + idx + idx..];
                    fraction[j][0][i] = fr.muls(tab[0] as usize, m);
                    fraction[j][1][i] = fr.muls(tab[1] as usize, m);
                    fraction[j][2][i] = fr.muls(tab[2] as usize, m);
                }
                sc += 3;
            } else {
                fraction[j][0][i] = 0.0;
                fraction[j][1][i] = 0.0;
                fraction[j][2][i] = 0.0;
            }
            if fr.bits_avail < 0 {
                return; /* Caller checks that again. */
            }
        }
        alloc1 += 1 << step;
    }

    for i in jsbound..sblimit {
        let step = alloc[alloc1].bits as i32;
        bita += 1; /* channel 1 and channel 2 bitalloc are the same */
        let ba = bit_alloc[bita] as usize;
        bita += 1;
        if ba != 0 {
            let alloc2 = alloc[alloc1 + ba];
            let k = alloc2.bits as i32;
            let d1 = alloc2.d as i32;
            if d1 < 0 {
                let mut cm = fr.muls(k as usize, scale[sc + x1 + 3] as usize);
                fraction[0][0][i] = (getbits(fr, k) as i32 + d1) as f32;
                fraction[0][1][i] = (getbits(fr, k) as i32 + d1) as f32;
                fraction[0][2][i] = (getbits(fr, k) as i32 + d1) as f32;
                fraction[1][0][i] = fraction[0][0][i] * cm;
                fraction[1][1][i] = fraction[0][1][i] * cm;
                fraction[1][2][i] = fraction[0][2][i] * cm;
                cm = fr.muls(k as usize, scale[sc + x1] as usize);
                fraction[0][0][i] *= cm;
                fraction[0][1][i] *= cm;
                fraction[0][2][i] *= cm;
            } else {
                let m1 = scale[sc + x1] as usize;
                let m2 = scale[sc + x1 + 3] as usize;
                let idx = getbits(fr, k) as usize;
                let tab = &grp_table(d1)[idx + idx + idx..];
                fraction[0][0][i] = fr.muls(tab[0] as usize, m1);
                fraction[1][0][i] = fr.muls(tab[0] as usize, m2);
                fraction[0][1][i] = fr.muls(tab[1] as usize, m1);
                fraction[1][1][i] = fr.muls(tab[1] as usize, m2);
                fraction[0][2][i] = fr.muls(tab[2] as usize, m1);
                fraction[1][2][i] = fr.muls(tab[2] as usize, m2);
            }
            sc += 6;
            if fr.bits_avail < 0 {
                return; /* Caller checks that again. */
            }
        } else {
            fraction[0][0][i] = 0.0;
            fraction[0][1][i] = 0.0;
            fraction[0][2][i] = 0.0;
            fraction[1][0][i] = 0.0;
            fraction[1][1][i] = 0.0;
            fraction[1][2][i] = 0.0;
        }
        /*
            Historic comment...
            should we use individual scalefac for channel 2 or
            is the current way the right one , where we just copy channel 1 to
            channel 2 ??
            The current 'strange' thing is, that we throw away the scalefac
            values for the second channel ...!!
            -> changed .. now we use the scalefac values of channel one !!
        */
        alloc1 += 1 << step;
    }

    if sblimit > fr.down_sample_sblimit as usize {
        sblimit = fr.down_sample_sblimit as usize;
    }

    for i in sblimit..SBLIMIT {
        for j in 0..stereo {
            fraction[j][0][i] = 0.0;
            fraction[j][1][i] = 0.0;
            fraction[j][2][i] = 0.0;
        }
    }
}

fn ii_select_table(fr: &mut Mpg123Handle<'_>) {
    const TRANSLATE: [[[usize; 16]; 2]; 3] = [
        [
            [0, 2, 2, 2, 2, 2, 2, 0, 0, 0, 1, 1, 1, 1, 1, 0],
            [0, 2, 2, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0],
        ],
        [
            [0, 2, 2, 2, 2, 2, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            [0, 2, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        ],
        [
            [0, 3, 3, 3, 3, 3, 3, 0, 0, 0, 1, 1, 1, 1, 1, 0],
            [0, 3, 3, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0],
        ],
    ];

    let tables: [&'static [AlTable]; 5] = [&ALLOC_0, &ALLOC_1, &ALLOC_2, &ALLOC_3, &ALLOC_4];
    const SBLIMS: [i32; 5] = [27, 30, 8, 12, 30];

    let table = if fr.hdr.sampling_frequency >= 3 {
        /* Or equivalent: (fr->lsf == 1) */
        4
    } else {
        TRANSLATE[fr.hdr.sampling_frequency as usize][(2 - fr.stereo) as usize]
            [fr.hdr.bitrate_index as usize]
    };

    let sblim = SBLIMS[table];
    fr.alloc = tables[table];
    fr.ii_sblimit = sblim;
}

/// Translation of `INT123_do_layer2()`.
pub(crate) fn do_layer2(fr: &mut Mpg123Handle<'_>) -> i32 {
    let Some(mut scratch) = fr.layerscratch.take() else {
        return 0;
    };
    let clip = do_layer2_on(fr, &mut scratch.layer2_fraction);
    fr.layerscratch = Some(scratch);
    clip
}

fn do_layer2_on(fr: &mut Mpg123Handle<'_>, fraction: &mut [[[f32; SBLIMIT]; 4]; 2]) -> i32 {
    let mut clip = 0;
    let stereo = fr.stereo;
    /* pick_table clears unused subbands */
    /* replacement for real fraction[2][4][SBLIMIT], needs alignment. */
    let mut bit_alloc = [0u32; 64];
    let mut scale = [0i32; 192];
    let mut single = fr.single;

    ii_select_table(fr);
    fr.jsbound = if fr.hdr.mode == MPG_MD_JOINT_STEREO {
        (fr.hdr.mode_ext << 2) + 4
    } else {
        fr.ii_sblimit
    };

    if fr.jsbound > fr.ii_sblimit {
        // Truncating stereo boundary to sideband limit.
        fr.jsbound = fr.ii_sblimit;
    }

    /* TODO: What happens with mono mixing, actually? */
    if stereo == 1 || single == SINGLE_MIX {
        /* also, mix not really handled */
        single = SINGLE_LEFT;
    }

    // FIXME (upstream): mono flags other than MPG123_MONO_LEFT, _RIGHT or _MIX
    // alone (MPG123_FORCE_MONO, the test mask, say) make single 2, 4, 5 or 6,
    // and upstream reads the fractions of that "channel", past the buffer;
    // this decodes the left channel instead, as layer III does then.
    if single > SINGLE_RIGHT {
        single = SINGLE_LEFT;
    }

    if ii_step_one(&mut bit_alloc, &mut scale, fr) != 0 {
        // first step of layer I decoding failed
        return clip;
    }

    for i in 0..SCALE_BLOCK {
        ii_step_two(&bit_alloc, fraction, &scale, fr, i >> 2);
        if fr.bits_avail < 0 {
            // missing bits in layer II step two
            return clip;
        }
        for j in 0..3 {
            if single != SINGLE_STEREO {
                let f = fr.synth_mono.expect("synth set up");
                clip += f(&mut fraction[single as usize][j], fr);
            } else {
                let f = fr.synth_stereo.expect("synth set up");
                let (l, r) = fraction.split_at_mut(1);
                clip += f(&mut l[0][j], &mut r[0][j], fr);
            }
        }
    }

    clip
}
