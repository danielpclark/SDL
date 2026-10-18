/*
    layer1.c: the layer 1 decoder

    copyright 1995-2009 by the mpg123 project - free software under the terms of the LGPL 2.1
    see COPYING and AUTHORS files in distribution or http://mpg123.org
    initially written by Michael Hipp

    may have a few bugs after last optimization ...
*/
// Modified 2026-10-17: translated into Rust from layer1.c in SDL_mixer 3.3.0's libmpg123 1.33.7 (see NOTICE).

use crate::frame::*;
use crate::getbits::{getbits, getbits_fast};

/*
    Allocation value is not allowed to be 15. Initially, libmad showed me the
    error that mpg123 used to ignore. Then, I found a quote on that in
    Shlien, S. (1994): Guide to MPEG-1 Audio Standard.
    IEEE Transactions on Broadcasting 40, 4

    "To avoid conflicts with the synchronization code, code '1111' is defined
    to be illegal."
*/
fn check_balloc(balloc: &[u32]) -> i32 {
    for &ba in balloc {
        if ba == 15 {
            // Illegal bit allocation value.
            return -1;
        }
    }

    0
}

/// `NEED_BITS(fr, num)`: whether that many bits are there.
#[inline]
fn need_bits(fr: &Mpg123Handle<'_>, num: u32) -> bool {
    fr.bits_avail >= num as i64
}

fn i_step_one(
    balloc: &mut [u32; 2 * SBLIMIT],
    scale_index: &mut [u32; 2 * SBLIMIT],
    fr: &mut Mpg123Handle<'_>,
) -> i32 {
    let mut ba = 0usize;
    let mut sca = 0usize;

    if fr.stereo == 2 {
        let jsbound = fr.jsbound as usize;
        let mut needbits = (jsbound * 2 * 4 + (SBLIMIT - jsbound) * 4) as u32;

        if !need_bits(fr, needbits) {
            // %u bits needed, %li available
            return -1;
        }
        needbits = 0;
        for _i in 0..jsbound {
            balloc[ba] = getbits_fast(fr, 4);
            balloc[ba + 1] = getbits_fast(fr, 4);
            needbits += ((if balloc[ba] != 0 { 1 } else { 0 })
                + (if balloc[ba + 1] != 0 { 1 } else { 0 }))
                * 6;
            ba += 2;
        }
        for _i in jsbound..SBLIMIT {
            balloc[ba] = getbits_fast(fr, 4);
            needbits += (if balloc[ba] != 0 { 1 } else { 0 }) * 12;
            ba += 1;
        }

        if check_balloc(&balloc[..ba]) != 0 {
            return -1;
        }

        ba = 0;
        if !need_bits(fr, needbits) {
            return -1;
        }
        for _i in 0..jsbound {
            if balloc[ba] != 0 {
                scale_index[sca] = getbits_fast(fr, 6);
                sca += 1;
            }
            ba += 1;
            if balloc[ba] != 0 {
                scale_index[sca] = getbits_fast(fr, 6);
                sca += 1;
            }
            ba += 1;
        }
        for _i in jsbound..SBLIMIT {
            if balloc[ba] != 0 {
                scale_index[sca] = getbits_fast(fr, 6);
                sca += 1;
                scale_index[sca] = getbits_fast(fr, 6);
                sca += 1;
            }
            ba += 1;
        }
    } else {
        let mut needbits = (SBLIMIT * 4) as u32;

        if !need_bits(fr, needbits) {
            return -1;
        }
        needbits = 0;
        for _i in 0..SBLIMIT {
            balloc[ba] = getbits_fast(fr, 4);
            needbits += (if balloc[ba] != 0 { 1 } else { 0 }) * 6;
            ba += 1;
        }

        if check_balloc(&balloc[..ba]) != 0 {
            return -1;
        }

        ba = 0;
        if !need_bits(fr, needbits) {
            return -1;
        }
        for _i in 0..SBLIMIT {
            if balloc[ba] != 0 {
                scale_index[sca] = getbits_fast(fr, 6);
                sca += 1;
            }
            ba += 1;
        }
    }

    0
}

/* Something sane in place of undefined (-1)<<n. Well, not really. */
#[inline]
fn minus_shift(n: u32) -> i32 {
    (u32::MAX << n) as i32
}

fn i_step_two(
    fraction: &mut [[f32; SBLIMIT]; 2],
    balloc: &[u32; 2 * SBLIMIT],
    scale_index: &[u32; 2 * SBLIMIT],
    fr: &mut Mpg123Handle<'_>,
) -> i32 {
    let mut smpb = [0i32; 2 * SBLIMIT]; /* values: 0-65535 */
    let mut sca = 0usize;

    if fr.stereo == 2 {
        let mut needbits: u32 = 0;
        let jsbound = fr.jsbound as usize;

        let mut ba = 0usize;
        for _i in 0..jsbound {
            let n = balloc[ba];
            ba += 1;
            if n != 0 {
                needbits += n + 1;
            }
            let n = balloc[ba];
            ba += 1;
            if n != 0 {
                needbits += n + 1;
            }
        }
        for _i in jsbound..SBLIMIT {
            let n = balloc[ba];
            ba += 1;
            if n != 0 {
                needbits += n + 1;
            }
        }
        if !need_bits(fr, needbits) {
            return -1;
        }

        ba = 0;
        let mut sample = 0usize;
        for _i in 0..jsbound {
            let n = balloc[ba];
            ba += 1;
            if n != 0 {
                smpb[sample] = getbits(fr, (n + 1) as i32) as i32;
                sample += 1;
            }

            let n = balloc[ba];
            ba += 1;
            if n != 0 {
                smpb[sample] = getbits(fr, (n + 1) as i32) as i32;
                sample += 1;
            }
        }
        for _i in jsbound..SBLIMIT {
            let n = balloc[ba];
            ba += 1;
            if n != 0 {
                smpb[sample] = getbits(fr, (n + 1) as i32) as i32;
                sample += 1;
            }
        }

        ba = 0;
        sample = 0;
        let mut f0 = 0usize;
        let mut f1 = 0usize;
        for _i in 0..jsbound {
            let n = balloc[ba];
            ba += 1;
            if n != 0 {
                let v = (minus_shift(n) + smpb[sample] + 1) as f32;
                sample += 1;
                fraction[0][f0] = v * fr.muls((n + 1) as usize, scale_index[sca] as usize);
                sca += 1;
            } else {
                fraction[0][f0] = 0.0;
            }
            f0 += 1;

            let n = balloc[ba];
            ba += 1;
            if n != 0 {
                let v = (minus_shift(n) + smpb[sample] + 1) as f32;
                sample += 1;
                fraction[1][f1] = v * fr.muls((n + 1) as usize, scale_index[sca] as usize);
                sca += 1;
            } else {
                fraction[1][f1] = 0.0;
            }
            f1 += 1;
        }
        for _i in jsbound..SBLIMIT {
            let n = balloc[ba];
            ba += 1;
            if n != 0 {
                let samp = (minus_shift(n) + smpb[sample] + 1) as f32;
                sample += 1;
                fraction[0][f0] = samp * fr.muls((n + 1) as usize, scale_index[sca] as usize);
                sca += 1;
                fraction[1][f1] = samp * fr.muls((n + 1) as usize, scale_index[sca] as usize);
                sca += 1;
            } else {
                fraction[0][f0] = 0.0;
                fraction[1][f1] = 0.0;
            }
            f0 += 1;
            f1 += 1;
        }
        for i in fr.down_sample_sblimit.max(0) as usize..32 {
            fraction[0][i] = 0.0;
            fraction[1][i] = 0.0;
        }
    } else {
        let mut needbits: u32 = 0;

        let mut ba = 0usize;
        for _i in 0..SBLIMIT {
            let n = balloc[ba];
            ba += 1;
            if n != 0 {
                needbits += n + 1;
            }
        }
        if !need_bits(fr, needbits) {
            return -1;
        }

        ba = 0;
        let mut sample = 0usize;
        for _i in 0..SBLIMIT {
            let n = balloc[ba];
            ba += 1;
            if n != 0 {
                smpb[sample] = getbits(fr, (n + 1) as i32) as i32;
                sample += 1;
            }
        }

        ba = 0;
        sample = 0;
        for f0 in 0..SBLIMIT {
            let n = balloc[ba];
            ba += 1;
            if n != 0 {
                let v = (minus_shift(n) + smpb[sample] + 1) as f32;
                sample += 1;
                fraction[0][f0] = v * fr.muls((n + 1) as usize, scale_index[sca] as usize);
                sca += 1;
            } else {
                fraction[0][f0] = 0.0;
            }
        }
        for i in fr.down_sample_sblimit.max(0) as usize..32 {
            fraction[0][i] = 0.0;
        }
    }
    0
}

/// Translation of `INT123_do_layer1()`.
pub(crate) fn do_layer1(fr: &mut Mpg123Handle<'_>) -> i32 {
    let Some(mut scratch) = fr.layerscratch.take() else {
        return 0;
    };
    let clip = do_layer1_on(fr, &mut scratch.layer1_fraction);
    fr.layerscratch = Some(scratch);
    clip
}

fn do_layer1_on(fr: &mut Mpg123Handle<'_>, fraction: &mut [[f32; SBLIMIT]; 2]) -> i32 {
    let mut clip = 0;
    let stereo = fr.stereo;
    let mut balloc = [0u32; 2 * SBLIMIT];
    let mut scale_index = [0u32; 2 * SBLIMIT];
    let mut single = fr.single;

    fr.jsbound = if fr.hdr.mode == MPG_MD_JOINT_STEREO {
        (fr.hdr.mode_ext << 2) + 4
    } else {
        32
    };

    if stereo == 1 || single == SINGLE_MIX {
        /* I don't see mixing handled here */
        single = SINGLE_LEFT;
    }

    // Note (upstream): mono flags other than MPG123_MONO_LEFT, _RIGHT or _MIX
    // alone (MPG123_FORCE_MONO, the test mask, say) make single 2, 4, 5 or 6,
    // and upstream reads the fractions of that "channel", past the buffer;
    // this decodes the left channel instead, as layer III does then.
    if single > SINGLE_RIGHT {
        single = SINGLE_LEFT;
    }

    if i_step_one(&mut balloc, &mut scale_index, fr) != 0 {
        // Aborting layer I decoding after step one.
        return clip;
    }

    for _i in 0..SCALE_BLOCK {
        if i_step_two(fraction, &balloc, &scale_index, fr) != 0 {
            // Aborting layer I decoding after step two.
            return clip;
        }

        if single != SINGLE_STEREO {
            let f = fr.synth_mono.expect("synth set up");
            clip += f(&mut fraction[single as usize], fr);
        } else {
            let f = fr.synth_stereo.expect("synth set up");
            let (l, r) = fraction.split_at_mut(1);
            clip += f(&mut l[0], &mut r[0], fr);
        }
    }

    clip
}
