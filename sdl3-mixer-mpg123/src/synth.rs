/*
    synth.c: The functions for synthesizing samples, at the end of decoding.

    copyright 1995-2008 by the mpg123 project - free software under the terms of the LGPL 2.1
    see COPYING and AUTHORS files in distribution or http://mpg123.org
    initially written by Michael Hipp, heavily dissected and rearranged by Thomas Orgis
*/
// Modified 2026-10-17: translated into Rust from synth.c, synth_8bit.c, synth_real.c, synth_s32.c, synth.h, synth_mono.h, equalizer.c and sample.h in SDL_mixer 3.3.0's libmpg123 1.33.7 (see NOTICE).

//! The 1:1 synth functions of the generic decoder: `INT123_synth_1to1()`
//! and its mono and mono-to-stereo wrappers, for the 16-bit, 8-bit, float
//! and 32-bit outputs (each a `synth.h`/`synth_mono.h` instance in
//! upstream's C, a [`Sample`] type here), with `ACCURATE_ROUNDING` and
//! `IEEE_FLOAT` as SDL_mixer's build has them.

use crate::dct64::dct64;
use crate::frame::{Mpg123Handle, RealBuffs, AUSHIFT, S32_RESCALE, SHORT_SCALE};

/*
    sample.h: The conversion from internal data to output samples of differing formats.

    copyright 2007-9 by the mpg123 project - free software under the terms of the LGPL 2.1
    see COPYING and AUTHORS files in distribution or http://mpg123.org
    initially written by Thomas Orgis, taking WRITE_SAMPLE from decode.c
    Later added the end-conversion specific macros here, too.
*/

/* This function is only available for IEEE754 single-precision values
This is nearly identical to proper rounding, just -+0.5 is rounded to 0 */
#[inline]
pub(crate) fn ftoi16(x: f32) -> i16 {
    let f = x + 12582912.0f32; /* Magic Number: 2^23 + 2^22 */
    f.to_bits() as i32 as i16
}

/// `REAL_TO_SHORT()` (`REAL_TO_SHORT_ACCURATE()`, as `ACCURATE_ROUNDING`
/// is on).
#[inline]
fn real_to_short(x: f32) -> i16 {
    ftoi16(x)
}

/// `REAL_TO_S32()` with `ACCURATE_ROUNDING`.
#[inline]
fn real_to_s32(x: f32) -> i32 {
    let x = x as f64;
    (if x > 0.0 { x + 0.5 } else { x - 0.5 }) as i32
}

const REAL_PLUS_32767: f64 = 32767.0;
const REAL_MINUS_32768: f64 = -32768.0;
const REAL_PLUS_S32: f64 = 2147483647.0;
const REAL_MINUS_S32: f64 = -2147483648.0;

/// A synth output sample type (`SAMPLE_T` with its `WRITE_SAMPLE`).
pub(crate) trait Sample {
    /// `sizeof(SAMPLE_T)`
    const SIZE: usize;
    /// `WRITE_SAMPLE(samples, sum, clip)` at byte offset `at` of `out`.
    fn write(out: &mut [u8], at: usize, sum: f32, clip: &mut i32, conv16to8: Option<&[u8; 8192]>);
}

/// 16 bit signed (`short`, `WRITE_SHORT_SAMPLE`).
pub(crate) struct S16;
/// 8 bit (`unsigned char`, `WRITE_8BIT_SAMPLE`), via the 16 to 8 bit table.
pub(crate) struct U8Table;
/// `real` (`WRITE_REAL_SAMPLE`).
pub(crate) struct Real;
/// 32 bit signed (`int32_t`, `WRITE_S32_SAMPLE`).
pub(crate) struct S32;

impl Sample for S16 {
    const SIZE: usize = 2;
    #[inline]
    fn write(out: &mut [u8], at: usize, sum: f32, clip: &mut i32, _: Option<&[u8; 8192]>) {
        /* Macro to produce a short (signed 16bit) output sample from internal representation,
        which may be float, double or indeed some integer for fixed point handling. */
        let v: i16 = if (sum as f64) > REAL_PLUS_32767 {
            *clip += 1;
            0x7fff
        } else if (sum as f64) < REAL_MINUS_32768 {
            *clip += 1;
            -0x8000
        } else {
            real_to_short(sum)
        };
        out[at..at + 2].copy_from_slice(&v.to_ne_bytes());
    }
}

impl Sample for U8Table {
    const SIZE: usize = 1;
    #[inline]
    fn write(out: &mut [u8], at: usize, sum: f32, clip: &mut i32, conv16to8: Option<&[u8; 8192]>) {
        /* Produce an 8bit sample, via 16bit intermediate. */
        let write_8bit_tmp: i16 = if (sum as f64) > REAL_PLUS_32767 {
            *clip += 1;
            0x7fff
        } else if (sum as f64) < REAL_MINUS_32768 {
            *clip += 1;
            -0x8000
        } else {
            real_to_short(sum)
        };
        // (fr->conv16to8 points to the middle of conv16to8_buf)
        let conv = conv16to8.expect("conv16to8 table");
        out[at] = conv[(4096 + ((write_8bit_tmp as i32) >> AUSHIFT)) as usize];
    }
}

impl Sample for Real {
    const SIZE: usize = 4;
    #[inline]
    fn write(out: &mut [u8], at: usize, sum: f32, _clip: &mut i32, _: Option<&[u8; 8192]>) {
        let v = (1.0f32 / SHORT_SCALE as f32) * sum;
        out[at..at + 4].copy_from_slice(&v.to_ne_bytes());
    }
}

impl Sample for S32 {
    const SIZE: usize = 4;
    #[inline]
    fn write(out: &mut [u8], at: usize, sum: f32, clip: &mut i32, _: Option<&[u8; 8192]>) {
        /*
        32bit signed
        We do clipping with the same old borders... but different conversion.
        We see here that we need extra work for non-16bit output... we optimized for 16bit.
        -0x7fffffff-1 is the minimum 32 bit signed integer value expressed so that MSVC
        does not give a compile time warning.
        */
        let tmpsum = sum * S32_RESCALE as f32;
        let v: i32 = if (tmpsum as f64) > REAL_PLUS_S32 {
            *clip += 1;
            0x7fffffff
        } else if (tmpsum as f64) < REAL_MINUS_S32 {
            *clip += 1;
            -0x7fffffff - 1
        } else {
            real_to_s32(tmpsum)
        };
        out[at..at + 4].copy_from_slice(&v.to_ne_bytes());
    }
}

/*
    equalizer.c: equalizer settings

    copyright ?-2006 by the mpg123 project - free software under the terms of the LGPL 2.1
    see COPYING and AUTHORS files in distribution or http://mpg123.org
    initially written by Michael Hipp
*/

/// Translation of `INT123_do_equalizer()`.
fn do_equalizer(bandptr: &mut [f32], channel: i32, equalizer: &[[f32; 32]; 2]) {
    for i in 0..32 {
        bandptr[i] *= equalizer[channel as usize][i];
    }
}

/* One decoding block is 64 samples. */
const BLOCK: usize = 0x40;

/*
    synth.h: generic synth functions

    copyright 1995-2008 by the mpg123 project - free software under the terms of the LGPL 2.1
    see COPYING and AUTHORS files in distribution or http://mpg123.org
    initially written by Michael Hipp, generalized by Thomas Orgis

    This header is used multiple times to create different variants of these functions.
    See decode.c and friends.
    Hint: BLOCK, MONO_NAME, MONO2STEREO_NAME, SYNTH_NAME and SAMPLE_T as well as WRITE_SAMPLE do vary.

    Thomas looked closely at the decode_1to1, decode_2to1 and decode_4to1 contents, seeing that they are too similar to be separate files.
    This is what resulted...

    Basically, you need one set of these functions for each output sample type.
    That currently means signed short, 8bit or float/double; though unsigned short may come, too.

    Define NO_AUTOINCREMENT i386 code that shall not rely on autoincrement.
    Actual benefit of this has to be examined; may apply to specific (old) compilers, only.
*/

/// The body of `SYNTH_NAME()` (synth.h), writing the samples into `out`
/// from byte offset `pos` on (upstream's `fr->buffer.data +
/// fr->buffer.fill`).
#[allow(clippy::too_many_arguments)]
fn synth_core<S: Sample>(
    bandptr: &mut [f32],
    channel: i32,
    bo: &mut i32,
    buffs: &mut RealBuffs,
    decwin: &[f32; 512 + 32],
    equalizer: Option<&[[f32; 32]; 2]>,
    conv16to8: Option<&[u8; 8192]>,
    out: &mut [u8],
    pos: usize,
) -> i32 {
    /* We use autoincrement and thus need this re-adjustment for window/b0. */
    const BACKPEDAL: isize = 0x10;
    const STEP: usize = 2;
    let mut samples = pos;

    let mut clip = 0;
    if let Some(eq) = equalizer {
        do_equalizer(bandptr, channel, eq);
    }
    let buf = if channel == 0 {
        *bo -= 1;
        *bo &= 0xf;
        &mut buffs[0]
    } else {
        samples += S::SIZE;
        &mut buffs[1]
    };

    let bo1;
    let b0: &[f32; 0x110];
    if *bo & 0x1 != 0 {
        bo1 = *bo;
        let (b_0, b_1) = buf.split_at_mut(1);
        dct64(
            &mut b_1[0][((*bo + 1) & 0xf) as usize..],
            &mut b_0[0][*bo as usize..],
            bandptr,
        );
        b0 = &buf[0];
    } else {
        bo1 = *bo + 1;
        let (b_0, b_1) = buf.split_at_mut(1);
        dct64(
            &mut b_0[0][*bo as usize..],
            &mut b_1[0][(*bo + 1) as usize..],
            bandptr,
        );
        b0 = &buf[1];
    }

    {
        let mut b0i: isize = 0;
        let mut window: isize = 16 - bo1 as isize;
        let w = |i: isize| decwin[i as usize];
        let b = |i: isize| b0[i as usize];

        for _j in (1..=(BLOCK / 4)).rev() {
            let mut sum;
            sum = w(window) * b(b0i);
            sum -= w(window + 1) * b(b0i + 1);
            sum += w(window + 2) * b(b0i + 2);
            sum -= w(window + 3) * b(b0i + 3);
            sum += w(window + 4) * b(b0i + 4);
            sum -= w(window + 5) * b(b0i + 5);
            sum += w(window + 6) * b(b0i + 6);
            sum -= w(window + 7) * b(b0i + 7);
            sum += w(window + 8) * b(b0i + 8);
            sum -= w(window + 9) * b(b0i + 9);
            sum += w(window + 10) * b(b0i + 10);
            sum -= w(window + 11) * b(b0i + 11);
            sum += w(window + 12) * b(b0i + 12);
            sum -= w(window + 13) * b(b0i + 13);
            sum += w(window + 14) * b(b0i + 14);
            sum -= w(window + 15) * b(b0i + 15);
            window += 16;
            b0i += 16;

            S::write(out, samples, sum, &mut clip, conv16to8);
            b0i += 0x400 / BLOCK as isize - BACKPEDAL;
            window += 0x800 / BLOCK as isize - BACKPEDAL;
            samples += STEP * S::SIZE;
        }

        {
            let mut sum;
            sum = w(window) * b(b0i);
            sum += w(window + 0x2) * b(b0i + 0x2);
            sum += w(window + 0x4) * b(b0i + 0x4);
            sum += w(window + 0x6) * b(b0i + 0x6);
            sum += w(window + 0x8) * b(b0i + 0x8);
            sum += w(window + 0xA) * b(b0i + 0xA);
            sum += w(window + 0xC) * b(b0i + 0xC);
            sum += w(window + 0xE) * b(b0i + 0xE);

            S::write(out, samples, sum, &mut clip, conv16to8);
            samples += STEP * S::SIZE;
            b0i -= 0x400 / BLOCK as isize;
            window -= 0x800 / BLOCK as isize;
        }
        window += (bo1 as isize) << 1;

        for _j in (1..=(BLOCK / 4 - 1)).rev() {
            let mut sum;
            window -= 1;
            sum = -(w(window) * b(b0i));
            for k in 1..16 {
                window -= 1;
                sum -= w(window) * b(b0i + k);
            }
            b0i += 16;
            S::write(out, samples, sum, &mut clip, conv16to8);
            b0i -= 0x400 / BLOCK as isize + BACKPEDAL;
            window -= 0x800 / BLOCK as isize - BACKPEDAL;
            samples += STEP * S::SIZE;
        }
    }

    clip
}

/// `SYNTH_NAME()` for one sample type: `INT123_synth_1to1()` and friends.
fn synth_plain<S: Sample>(
    bandptr: &mut [f32],
    channel: i32,
    fr: &mut Mpg123Handle<'_>,
    final_: i32,
) -> i32 {
    let equalizer = if fr.have_eq_settings != 0 {
        Some(&fr.equalizer)
    } else {
        None
    };
    let (Some(buffs), Some(decwin)) = (fr.real_buffs.as_deref_mut(), fr.decwin.as_deref()) else {
        return 0;
    };
    let pos = fr.buffer.fill;
    let clip = synth_core::<S>(
        bandptr,
        channel,
        &mut fr.bo,
        buffs,
        decwin,
        equalizer,
        fr.conv16to8_buf.as_deref(),
        &mut fr.buffer.data,
        pos,
    );

    if final_ != 0 {
        fr.buffer.fill += BLOCK * S::SIZE;
    }

    clip
}

/*
    monosynth.h: generic mono related synth functions

    copyright 1995-2008 by the mpg123 project - free software under the terms of the LGPL 2.1
    see COPYING and AUTHORS files in distribution or http://mpg123.org
    initially written by Michael Hipp, generalized by Thomas Orgis

    This header is used multiple times to create different variants of these functions.
    See decode.c and synth.h .
    Hint: BLOCK, MONO_NAME, MONO2STEREO_NAME, SYNTH_NAME and SAMPLE_T do vary.

    Thomas looked closely at the decode_1to1, decode_2to1 and decode_4to1 contents, seeing that they are too similar to be separate files.
    This is what resulted...

    Reason to separate this from synth.h:
    There are decoders that have a special INT123_synth_1to1 but still can use these generic derivations for the mono stuff.
    It generally makes a good deal of sense to set SYNTH_NAME to opt_synth_1to1(fr) (or opt_synth_2to1(fr), etc.).
*/

/// `MONO_NAME()`: mono synth, wrapping over `SYNTH_NAME` (the plain synth
/// of this sample type, which is always the generic one here).
fn synth_mono<S: Sample>(bandptr: &mut [f32], fr: &mut Mpg123Handle<'_>) -> i32 {
    let mut samples_tmp = [0u8; BLOCK * 4];

    /* save buffer stuff, trick samples_tmp into there, decode, restore */
    let pnt = fr.buffer.fill;
    let equalizer = if fr.have_eq_settings != 0 {
        Some(&fr.equalizer)
    } else {
        None
    };
    let (Some(buffs), Some(decwin)) = (fr.real_buffs.as_deref_mut(), fr.decwin.as_deref()) else {
        return 0;
    };
    let ret = synth_core::<S>(
        bandptr,
        0,
        &mut fr.bo,
        buffs,
        decwin,
        equalizer,
        fr.conv16to8_buf.as_deref(),
        &mut samples_tmp,
        0,
    ); /* decode into samples_tmp */

    /* now append samples from samples_tmp */
    let mut samples = pnt; /* just the next mem in frame buffer */
    let mut tmp1 = 0;
    for _i in 0..(BLOCK / 2) {
        fr.buffer.data[samples..samples + S::SIZE]
            .copy_from_slice(&samples_tmp[tmp1..tmp1 + S::SIZE]);
        samples += S::SIZE;
        tmp1 += 2 * S::SIZE;
    }
    fr.buffer.fill = pnt + (BLOCK / 2) * S::SIZE;

    ret
}

/// `MONO2STEREO_NAME()`: mono to stereo synth, wrapping over `SYNTH_NAME`.
fn synth_m2s<S: Sample>(bandptr: &mut [f32], fr: &mut Mpg123Handle<'_>) -> i32 {
    let ret = synth_plain::<S>(bandptr, 0, fr, 1);
    let mut samples = fr.buffer.fill - BLOCK * S::SIZE;

    for _i in 0..(BLOCK / 2) {
        let (a, b) = fr.buffer.data.split_at_mut(samples + S::SIZE);
        b[..S::SIZE].copy_from_slice(&a[samples..samples + S::SIZE]);
        samples += 2 * S::SIZE;
    }

    ret
}

/// Translation of `INT123_synth_1to1()`.
pub(crate) fn synth_1to1(
    bandptr: &mut [f32],
    channel: i32,
    fr: &mut Mpg123Handle<'_>,
    final_: i32,
) -> i32 {
    synth_plain::<S16>(bandptr, channel, fr, final_)
}

/// Translation of `INT123_synth_1to1_mono()`.
pub(crate) fn synth_1to1_mono(bandptr: &mut [f32], fr: &mut Mpg123Handle<'_>) -> i32 {
    synth_mono::<S16>(bandptr, fr)
}

/// Translation of `INT123_synth_1to1_m2s()`.
pub(crate) fn synth_1to1_m2s(bandptr: &mut [f32], fr: &mut Mpg123Handle<'_>) -> i32 {
    synth_m2s::<S16>(bandptr, fr)
}

/// Translation of `INT123_synth_1to1_8bit()`.
pub(crate) fn synth_1to1_8bit(
    bandptr: &mut [f32],
    channel: i32,
    fr: &mut Mpg123Handle<'_>,
    final_: i32,
) -> i32 {
    synth_plain::<U8Table>(bandptr, channel, fr, final_)
}

/// Translation of `INT123_synth_1to1_8bit_mono()`.
pub(crate) fn synth_1to1_8bit_mono(bandptr: &mut [f32], fr: &mut Mpg123Handle<'_>) -> i32 {
    synth_mono::<U8Table>(bandptr, fr)
}

/// Translation of `INT123_synth_1to1_8bit_m2s()`.
pub(crate) fn synth_1to1_8bit_m2s(bandptr: &mut [f32], fr: &mut Mpg123Handle<'_>) -> i32 {
    synth_m2s::<U8Table>(bandptr, fr)
}

/// Translation of `INT123_synth_1to1_real()`.
pub(crate) fn synth_1to1_real(
    bandptr: &mut [f32],
    channel: i32,
    fr: &mut Mpg123Handle<'_>,
    final_: i32,
) -> i32 {
    synth_plain::<Real>(bandptr, channel, fr, final_)
}

/// Translation of `INT123_synth_1to1_real_mono()`.
pub(crate) fn synth_1to1_real_mono(bandptr: &mut [f32], fr: &mut Mpg123Handle<'_>) -> i32 {
    synth_mono::<Real>(bandptr, fr)
}

/// Translation of `INT123_synth_1to1_real_m2s()`.
pub(crate) fn synth_1to1_real_m2s(bandptr: &mut [f32], fr: &mut Mpg123Handle<'_>) -> i32 {
    synth_m2s::<Real>(bandptr, fr)
}

/// Translation of `INT123_synth_1to1_s32()`.
pub(crate) fn synth_1to1_s32(
    bandptr: &mut [f32],
    channel: i32,
    fr: &mut Mpg123Handle<'_>,
    final_: i32,
) -> i32 {
    synth_plain::<S32>(bandptr, channel, fr, final_)
}

/// Translation of `INT123_synth_1to1_s32_mono()`.
pub(crate) fn synth_1to1_s32_mono(bandptr: &mut [f32], fr: &mut Mpg123Handle<'_>) -> i32 {
    synth_mono::<S32>(bandptr, fr)
}

/// Translation of `INT123_synth_1to1_s32_m2s()`.
pub(crate) fn synth_1to1_s32_m2s(bandptr: &mut [f32], fr: &mut Mpg123Handle<'_>) -> i32 {
    synth_m2s::<S32>(bandptr, fr)
}
