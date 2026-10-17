// Rust translation of src/mix_all.c and src/mix_all.h from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2026 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The software mixer's inner loops: nearest neighbor, linear and cubic
//! spline interpolation, with and without the IT lowpass filter, for 8 and
//! 16 bit, mono and stereo samples, to mono and stereo output.
//!
//! Upstream expands its macros into one function per combination; here
//! they are one generic function, [`mix`], instantiated for each (the
//! tables at the end list them in upstream's order).

use super::common::xmp_asl;
use super::mixer::{MixerVoice, FILTER_SHIFT, SMIX_MASK, SMIX_SHIFT};
use super::precomp_lut::*;
use super::SAMPLE_GUARD;

/* Mixers array index:
 *
 * bit 0: 0=8 bit sample, 1=16 bit sample
 * bit 1: 0=mono sample, 1=stereo sample
 * bit 2: 0=mono output, 1=stereo output
 * bit 3: 0=unfiltered, 1=filtered
 */

/// Translation of `MIXER_FP`: `vi`, `buffer` (from the position to mix
/// at), the voice's sample data (with its guard bytes), `count`, `vl`,
/// `vr`, `step`, `ramp`, `delta_l`, `delta_r`.
pub(crate) type MixerFp =
    fn(&mut MixerVoice, &mut [i32], &mut [u8], i32, i32, i32, i32, i32, i32, i32);

/* Mixers
 *
 * To increase performance eight mixers are defined, one for each
 * combination of the following parameters: interpolation, resolution
 * and number of channels.
 */

/// `sptr[i]` for an 8-bit sample.
#[inline(always)]
fn s8(sptr: &[u8], i: i32) -> i32 {
    sptr[(SAMPLE_GUARD as isize + i as isize) as usize] as i8 as i32
}

/// `sptr[i]` for a 16-bit sample.
#[inline(always)]
fn s16(sptr: &[u8], i: i32) -> i32 {
    let o = (SAMPLE_GUARD as isize + 2 * i as isize) as usize;
    i16::from_ne_bytes([sptr[o], sptr[o + 1]]) as i32
}

/* The following lut settings are PRECOMPUTED. If you plan on changing these
 * settings, you MUST also regenerate the arrays.
 */
/* number of bits used to scale spline coefs */
const SPLINE_QUANTBITS: i32 = 14;
const SPLINE_SHIFT: i32 = SPLINE_QUANTBITS;

/* log2(number) of precalculated splines (range is [4..14]) */
const SPLINE_FRACBITS: i32 = 10;
const SPLINE_LUTLEN: i32 = 1 << SPLINE_FRACBITS;

const SPLINE_FRACSHIFT: i32 = (16 - SPLINE_FRACBITS) - 2;
const SPLINE_FRACMASK: i32 = ((1 << (16 - SPLINE_FRACSHIFT)) - 1) & !3;

/* Sample pre-amplification is required to fix filter rounding errors
 * at high sample rates. The non-filtered mixers do not need this. */
const PREAMP_BITS: u32 = 15;

/* IT's WAV output driver uses a clamp that seems to roughly match this:
 * compare the WAV output of OpenMPT env-flt-max.it and filter-reset.it */
const FILTER_MIN: i64 = -65536 * (1 << PREAMP_BITS);
const FILTER_MAX: i64 = 65535 * (1 << PREAMP_BITS);

/// `MIX_FILTER_CLAMP(a)`
#[inline(always)]
fn mix_filter_clamp(a: i64) -> i64 {
    if a < FILTER_MIN {
        FILTER_MIN
    } else if a > FILTER_MAX {
        FILTER_MAX
    } else {
        a
    }
}

const NEAREST: u8 = 0;
const LINEAR: u8 = 1;
const SPLINE: u8 = 2;

/// The interpolated sample at `pos + off` (`NEAREST_8BIT()`,
/// `LINEAR_16BIT()`, `SPLINE_8BIT()` and the like).
#[inline(always)]
fn interpolate<const INTERP: u8, const B16: bool>(
    sptr: &[u8],
    pos: i32,
    off: i32,
    chn: i32,
    frac: i32,
) -> i32 {
    let get = |i: i32| if B16 { s16(sptr, i) } else { s8(sptr, i) };
    match INTERP {
        NEAREST => {
            if B16 {
                get(pos + off)
            } else {
                xmp_asl(get(pos + off), 8)
            }
        }
        LINEAR => {
            let (smp_l1, smp_dt);
            if B16 {
                smp_l1 = get(pos + off);
                smp_dt = get(pos + off + chn) - smp_l1;
            } else {
                smp_l1 = xmp_asl(get(pos + off), 8);
                smp_dt = xmp_asl(get(pos + off + chn), 8) - smp_l1;
            }
            smp_l1 + (((frac >> 1) * smp_dt) >> (SMIX_SHIFT - 1))
        }
        _ => {
            let f = (frac >> 6) as usize;
            let v = CUBIC_SPLINE_LUT0[f] as i32 * get(pos + off - chn)
                + CUBIC_SPLINE_LUT1[f] as i32 * get(pos + off)
                + CUBIC_SPLINE_LUT3[f] as i32 * get(pos + off + (chn << 1))
                + CUBIC_SPLINE_LUT2[f] as i32 * get(pos + off + chn);
            if B16 {
                v >> SPLINE_SHIFT
            } else {
                v >> (SPLINE_SHIFT - 8)
            }
        }
    }
}

/// The filter state of a voice (`VAR_FILTER_MONO`, `VAR_FILTER_STEREO`).
struct Filter {
    fl1: i32,
    fl2: i32,
    fr1: i32,
    fr2: i32,
    a0: i64,
    b0: i64,
    b1: i64,
}

impl Filter {
    /// `FILTER_LEFT(smp_in_l)`
    #[inline(always)]
    fn left(&mut self, smp_in_l: i32) -> i32 {
        let sl64 = (self.a0 * xmp_asl(smp_in_l, PREAMP_BITS) as i64
            + self.b0 * self.fl1 as i64
            + self.b1 * self.fl2 as i64)
            >> FILTER_SHIFT;
        let sl = mix_filter_clamp(sl64) as i32;
        self.fl2 = self.fl1;
        self.fl1 = sl;
        sl >> PREAMP_BITS
    }

    /// `FILTER_RIGHT(smp_in_r)`
    #[inline(always)]
    fn right(&mut self, smp_in_r: i32) -> i32 {
        let sr64 = (self.a0 * xmp_asl(smp_in_r, PREAMP_BITS) as i64
            + self.b0 * self.fr1 as i64
            + self.b1 * self.fr2 as i64)
            >> FILTER_SHIFT;
        let sr = mix_filter_clamp(sr64) as i32;
        self.fr2 = self.fr1;
        self.fr1 = sr;
        sr >> PREAMP_BITS
    }
}

/// `MIX_OUT(out_sample, out_level)`
#[inline(always)]
fn mix_out(buffer: &mut [i32], b: &mut usize, out_sample: i32, out_level: i32) {
    buffer[*b] = buffer[*b].wrapping_add(out_sample.wrapping_mul(out_level));
    *b += 1;
}

/// Translation of the `MIXER()` functions of every combination: the
/// `libxmp_mix_{monoout,stereoout}_{mono,stereo}_{8bit,16bit}_{nearest,linear,spline}[_filter]()`
/// functions.
#[inline(always)]
fn mix<
    const INTERP: u8,
    const B16: bool,
    const STEREO: bool,
    const STEREOOUT: bool,
    const FILTER: bool,
>(
    vi: &mut MixerVoice,
    buffer: &mut [i32],
    sptr: &mut [u8],
    mut count: i32,
    vl: i32,
    vr: i32,
    step: i32,
    ramp: i32,
    delta_l: i32,
    delta_r: i32,
) {
    // VAR_MONO / VAR_STEREO, VAR_NORM
    let chn: i32 = if STEREO { 2 } else { 1 };
    let mut pos: i32 = (vi.pos as i32).wrapping_mul(chn);
    let mut frac: i32 = ((1 << SMIX_SHIFT) as f64 * (vi.pos - (vi.pos as i32) as f64)) as i32;
    // VAR_MONOOUT, VAR_STEREOOUT
    let mut old_vl = vi.old_vl;
    let mut old_vr = vi.old_vr;
    // VAR_FILTER_MONO, VAR_FILTER_STEREO
    let mut flt = Filter {
        fl1: vi.filter.l1,
        fl2: vi.filter.l2,
        fr1: vi.filter.r1,
        fr2: vi.filter.r2,
        a0: vi.filter.a0 as i64,
        b0: vi.filter.b0 as i64,
        b1: vi.filter.b1 as i64,
    };
    let mut b = 0usize;

    if INTERP == NEAREST {
        // NEAREST_ROUND()
        /* For "nearest" to be nearest neighbor (instead of floor), the position needs
         * to be rounded. This only needs to be done once at the start of mixing, and
         * is required for reverse samples to round the same as forward samples.
         */
        frac += 1 << (SMIX_SHIFT - 1);
        pos += (frac >> SMIX_SHIFT) * chn;
        frac &= SMIX_MASK;
    }

    // The nearest neighbor mixers have no volume ramp (no LOOP_AC).
    let ramp_loop = INTERP != NEAREST;

    loop {
        let ac;
        if ramp_loop && count > ramp {
            ac = true; /* LOOP_AC */
        } else if count != 0 {
            ac = false; /* LOOP */
        } else {
            break;
        }

        let mut smpl = interpolate::<INTERP, B16>(sptr, pos, 0, chn, frac);
        let mut smpr = 0;
        if STEREO {
            smpr = interpolate::<INTERP, B16>(sptr, pos, 1, chn, frac);
        }
        if FILTER {
            smpl = flt.left(smpl);
            if STEREO {
                smpr = flt.right(smpr);
            }
        }

        if !STEREOOUT {
            /* MIX_MONO, MIX_MONO_AVG */
            let smp = if STEREO { (smpl + smpr) >> 1 } else { smpl };
            if ac {
                mix_out(buffer, &mut b, smp, old_vl >> 8);
                old_vl = old_vl.wrapping_add(delta_l);
            } else {
                mix_out(buffer, &mut b, smp, vl);
            }
        } else {
            /* MIX_STEREO */
            let r = if STEREO { smpr } else { smpl };
            if ac {
                mix_out(buffer, &mut b, smpl, old_vl >> 8);
                mix_out(buffer, &mut b, r, old_vr >> 8);
                old_vl = old_vl.wrapping_add(delta_l);
                old_vr = old_vr.wrapping_add(delta_r);
            } else {
                mix_out(buffer, &mut b, smpl, vl);
                mix_out(buffer, &mut b, r, vr);
            }
        }

        // UPDATE_POS()
        frac = frac.wrapping_add(step);
        pos = pos.wrapping_add((frac >> SMIX_SHIFT) * chn);
        frac &= SMIX_MASK;

        count -= 1;
    }

    if FILTER {
        /* Note: copying the left to the right here is for just in case these
         * values don't get cleared between playing stereo/mono samples. */
        // SAVE_FILTER_MONO(), SAVE_FILTER_STEREO()
        vi.filter.l1 = flt.fl1;
        vi.filter.l2 = flt.fl2;
        vi.filter.r1 = flt.fl1;
        vi.filter.r2 = flt.fl2;
        if STEREO {
            vi.filter.r1 = flt.fr1;
            vi.filter.r2 = flt.fr2;
        }
    }
}

/// `LIST_MIX_FUNCTIONS(type)`, with the filter variant's flag.
macro_rules! list_mix_functions {
    ($interp:expr, $filter:expr) => {
        [
            mix::<{ $interp }, false, false, false, { $filter }> as MixerFp,
            mix::<{ $interp }, true, false, false, { $filter }>,
            mix::<{ $interp }, false, true, false, { $filter }>,
            mix::<{ $interp }, true, true, false, { $filter }>,
            mix::<{ $interp }, false, false, true, { $filter }>,
            mix::<{ $interp }, true, false, true, { $filter }>,
            mix::<{ $interp }, false, true, true, { $filter }>,
            mix::<{ $interp }, true, true, true, { $filter }>,
        ]
    };
}

/// The two halves of a mixer table.
const fn cat(a: [MixerFp; 8], b: [MixerFp; 8]) -> [Option<MixerFp>; 16] {
    let mut out: [Option<MixerFp>; 16] = [None; 16];
    let mut i = 0;
    while i < 8 {
        out[i] = Some(a[i]);
        out[i + 8] = Some(b[i]);
        i += 1;
    }
    out
}

/// Translation of `libxmp_nearest_mixers[]`.
pub(crate) static LIBXMP_NEAREST_MIXERS: [Option<MixerFp>; 16] = cat(
    list_mix_functions!(NEAREST, false),
    list_mix_functions!(NEAREST, false),
);

/// Translation of `libxmp_linear_mixers[]`.
pub(crate) static LIBXMP_LINEAR_MIXERS: [Option<MixerFp>; 16] = cat(
    list_mix_functions!(LINEAR, false),
    list_mix_functions!(LINEAR, true),
);

/// Translation of `libxmp_spline_mixers[]`.
pub(crate) static LIBXMP_SPLINE_MIXERS: [Option<MixerFp>; 16] = cat(
    list_mix_functions!(SPLINE, false),
    list_mix_functions!(SPLINE, true),
);
