// Rust translation of src/audio/SDL_audioresample.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! SDL's resampler uses a "bandlimited interpolation" algorithm:
//!     <https://ccrma.stanford.edu/~jos/resample/>
//!
//! The resampler uses 32:32 fixed-point arithmetic to track its position.
//!
//! Upstream uses 6 zero crossings and the SSE (or NEON) frame resampler
//! whenever SIMD intrinsics are compiled in, which is every x86-64 and
//! AArch64 build; only builds without SIMD use 5 zero crossings and the
//! scalar `ResampleFrame_*` functions. This translation follows the SIMD
//! configuration, evaluating the SSE routine's lane arithmetic in portable
//! code, so its output is bit-identical to SDL on x86-64. (NEON sums the
//! final lanes in a different order, so AArch64 builds of C SDL can differ
//! in the last bit.)

use std::sync::OnceLock;

/// Translation of `RESAMPLER_ZERO_CROSSINGS` (SIMD builds).
const RESAMPLER_ZERO_CROSSINGS: usize = 6;

/// Translation of `RESAMPLER_SAMPLES_PER_FRAME`.
const RESAMPLER_SAMPLES_PER_FRAME: usize = RESAMPLER_ZERO_CROSSINGS * 2;

/// For a given srcpos, `srcpos + frame` are sampled, where
/// `-RESAMPLER_ZERO_CROSSINGS < frame <= RESAMPLER_ZERO_CROSSINGS`. Note,
/// when upsampling, it is also possible to start sampling from `srcpos = -1`.
/// Translation of `RESAMPLER_MAX_PADDING_FRAMES`.
const RESAMPLER_MAX_PADDING_FRAMES: i32 = RESAMPLER_ZERO_CROSSINGS as i32 + 1;

// More bits gives more precision, at the cost of a larger table.
const RESAMPLER_BITS_PER_ZERO_CROSSING: u32 = 3;
const RESAMPLER_SAMPLES_PER_ZERO_CROSSING: usize = 1 << RESAMPLER_BITS_PER_ZERO_CROSSING;
const RESAMPLER_FILTER_INTERP_BITS: u32 = 32 - RESAMPLER_BITS_PER_ZERO_CROSSING;
const RESAMPLER_FILTER_INTERP_RANGE: u32 = 1 << RESAMPLER_FILTER_INTERP_BITS;

/// Cubic Polynomial. Translation of `Cubic`.
type Cubic = [f32; 4];

/// The filter table, transposed in groups of four like the SIMD paths
/// expect (`SetupAudioResampler()` with `transpose = true`).
type FilterTable = [[Cubic; RESAMPLER_SAMPLES_PER_FRAME]; RESAMPLER_SAMPLES_PER_ZERO_CROSSING];

/// Translation of `ResamplerFilter`.
static RESAMPLER_FILTER: OnceLock<FilterTable> = OnceLock::new();

/// A 4-lane float vector, evaluated lane by lane in SSE order.
type V4 = [f32; 4];

#[inline(always)]
fn add(a: V4, b: V4) -> V4 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2], a[3] + b[3]]
}

#[inline(always)]
fn mul(a: V4, b: V4) -> V4 {
    [a[0] * b[0], a[1] * b[1], a[2] * b[2], a[3] * b[3]]
}

/// `sdl_madd_ps(a, b, c)`: `a + b * c` ("Not-so-fused multiply-add").
#[inline(always)]
fn madd(a: V4, b: V4, c: V4) -> V4 {
    add(a, mul(b, c))
}

#[inline(always)]
fn splat(x: f32) -> V4 {
    [x; 4]
}

/// `_mm_add_ss(out, _mm_movehl_ps(shuf, out))` after `out += shuf(2,3,0,1)`:
/// `(o0 + o1) + (o2 + o3)`.
#[inline(always)]
fn hsum(o: V4) -> f32 {
    (o[0] + o[1]) + (o[2] + o[3])
}

/// Translation of `ResampleFrame_Generic_SSE()`.
///
/// `src(i)` reads the input sample `i` floats after the frame start.
#[inline(always)]
fn resample_frame(
    src: impl Fn(usize) -> f32,
    dst: &mut [f32],
    filter: &[Cubic; RESAMPLER_SAMPLES_PER_FRAME],
    frac: f32,
    chans: usize,
) {
    let ld = |i: usize| -> V4 { [src(i), src(i + 1), src(i + 2), src(i + 3)] };

    let (f0, f1, f2) = {
        let frac1 = splat(frac);
        let frac2 = mul(frac1, frac1);
        let frac3 = mul(frac1, frac2);

        // Transposed in SetupAudioResampler
        let x = |g: usize| -> V4 {
            let mut out = filter[g * 4];
            out = madd(out, frac1, filter[g * 4 + 1]);
            out = madd(out, frac2, filter[g * 4 + 2]);
            out = madd(out, frac3, filter[g * 4 + 3]);
            out
        };
        (x(0), x(1), x(2))
    };

    if chans == 2 {
        let lo = |f: V4| [f[0], f[0], f[1], f[1]];
        let hi = |f: V4| [f[2], f[2], f[3], f[3]];
        // Duplicate each of the filter elements and multiply by the input
        // Use two accumulators to improve throughput
        let mut out0 = mul(ld(0), lo(f0));
        let mut out1 = mul(ld(4), hi(f0));
        out0 = madd(out0, ld(8), lo(f1));
        out1 = madd(out1, ld(12), hi(f1));
        out0 = madd(out0, ld(16), lo(f2));
        out1 = madd(out1, ld(20), hi(f2));

        // Add the accumulators together
        let out = add(out0, out1);

        // Add the lower and upper pairs together
        dst[0] = out[0] + out[2];
        dst[1] = out[1] + out[3];
        return;
    }

    if chans == 1 {
        // Multiply the filter by the input
        let mut out = mul(f0, ld(0));
        out = madd(out, f1, ld(4));
        out = madd(out, f2, ld(8));

        // Horizontal sum
        dst[0] = hsum(out);
        return;
    }

    let mut chan = 0;

    // Process 4 channels at once
    while chan + 4 <= chans {
        let mut inp = chan;
        let mut out0 = splat(0.0);
        let mut out1 = splat(0.0);

        for f in [f0, f1, f2] {
            for (b, acc) in [(0, 0), (1, 1), (2, 0), (3, 1)] {
                let v = madd(if acc == 0 { out0 } else { out1 }, ld(inp), splat(f[b]));
                if acc == 0 {
                    out0 = v;
                } else {
                    out1 = v;
                }
                inp += chans;
            }
        }

        // Add the accumulators together
        let out = add(out0, out1);
        dst[chan..chan + 4].copy_from_slice(&out);
        chan += 4;
    }

    // Process the remaining channels one at a time.
    // Channel counts 1,2,4,8 are already handled above, leaving 3,5,6,7 to deal with (looping 3,1,2,3 times).
    // Without vgatherdps (AVX2), this gets quite messy.
    while chan < chans {
        let gather = |base: usize| -> V4 {
            [
                src(base),
                src(base + chans),
                src(base + 2 * chans),
                src(base + 3 * chans),
            ]
        };
        let v0 = gather(chan);
        let v1 = gather(chan + 4 * chans);
        let v2 = gather(chan + 8 * chans);

        let mut out = mul(f0, v0);
        out = madd(out, f1, v1);
        out = madd(out, f2, v2);

        // Horizontal sum
        dst[chan] = hsum(out);
        chan += 1;
    }
}

/// Calculate the cubic equation which passes through all four points.
/// Translation of `CubicLeastSquares()`.
///
/// <https://en.wikipedia.org/wiki/Ordinary_least_squares>
/// <https://en.wikipedia.org/wiki/Polynomial_regression>
fn cubic_least_squares(y0: f32, y1: f32, y2: f32, y3: f32) -> Cubic {
    // Least squares matrix for xs = [0, 1/3, 2/3, 1]
    // [  1.0   0.0   0.0  0.0 ]
    // [ -5.5   9.0  -4.5  1.0 ]
    // [  9.0 -22.5  18.0 -4.5 ]
    // [ -4.5  13.5 -13.5  4.5 ]
    [
        y0,
        -5.5 * y0 + 9.0 * y1 - 4.5 * y2 + y3,
        9.0 * y0 - 22.5 * y1 + 18.0 * y2 - 4.5 * y3,
        -4.5 * y0 + 13.5 * y1 - 13.5 * y2 + 4.5 * y3,
    ]
}

/// Zeroth-order modified Bessel function of the first kind.
/// Translation of `BesselI0()`.
///
/// <https://mathworld.wolfram.com/ModifiedBesselFunctionoftheFirstKind.html>
#[allow(clippy::misrefactored_assign_op)] // `x *= x * 0.25` squares and quarters x, as upstream
fn bessel_i0(mut x: f32) -> f32 {
    let mut sum = 0.0f32;
    let mut i = 1.0f32;
    let mut t = 1.0f32;
    x *= x * 0.25;

    while t >= sum * f32::EPSILON {
        sum += t;
        t *= x / (i * i);
        i += 1.0;
    }

    sum
}

/// Pre-calculate 180 degrees of sin(pi * x) / pi. Translation of `SincTable()`.
///
/// The speedup from this isn't huge, but it also avoids precision issues.
/// If sinf isn't available, SDL_sinf just calls SDL_sin.
/// Know what SDL_sin(SDL_PI_F) equals? Not quite zero.
fn sinc_table(table: &mut [f32]) {
    let len = table.len();
    for (i, t) in table.iter_mut().enumerate() {
        *t = crate::stdlib::math::sinf(i as f32 * (crate::stdlib::math::PI_F / len as f32))
            / crate::stdlib::math::PI_F;
    }
}

/// Calculate Sinc(x/y), using a lookup table. Translation of `Sinc()`.
fn sinc(table: &[f32], x: i32, y: i32) -> f32 {
    let mut s = table[(x % y) as usize];
    s = if (x / y) & 1 != 0 { -s } else { s };
    (s * y as f32) / x as f32
}

/// Translation of `GenerateResamplerFilter()` followed by the transpose of
/// `SetupAudioResampler()`.
fn generate_resampler_filter() -> FilterTable {
    // Generate samples at 3x the target resolution, so that we have samples at [0, 1/3, 2/3, 1] of each position
    const TABLE_SAMPLES_PER_ZERO_CROSSING: usize = RESAMPLER_SAMPLES_PER_ZERO_CROSSING * 3;
    const TABLE_SIZE: usize = RESAMPLER_ZERO_CROSSINGS * TABLE_SAMPLES_PER_ZERO_CROSSING;

    // if dB > 50, beta=(0.1102 * (dB - 8.7)), according to Matlab.
    let db = 80.0f32;
    let beta = 0.1102f32 * (db - 8.7);
    let bessel_beta = bessel_i0(beta);
    let lensqr = (TABLE_SIZE * TABLE_SIZE) as f32;

    let mut sinc_tab = [0.0f32; TABLE_SAMPLES_PER_ZERO_CROSSING];
    sinc_table(&mut sinc_tab);

    // Generate one wing of the filter
    // https://en.wikipedia.org/wiki/Kaiser_window
    // https://en.wikipedia.org/wiki/Whittaker%E2%80%93Shannon_interpolation_formula
    let mut filter = [0.0f32; TABLE_SIZE + 1];
    filter[0] = 1.0;

    for (i, f) in filter.iter_mut().enumerate().skip(1) {
        let b = bessel_i0(beta * ((lensqr - (i * i) as f32) / lensqr).sqrt()) / bessel_beta;
        let s = sinc(&sinc_tab, i as i32, TABLE_SAMPLES_PER_ZERO_CROSSING as i32);
        *f = b * s;
    }

    // Generate the coefficients for each point
    // When interpolating, the fraction represents how far we are between input samples,
    // so we need to align the filter by "moving" it to the right.
    //
    // For the left wing, this means interpolating "forwards" (away from the center)
    // For the right wing, this means interpolating "backwards" (towards the center)
    //
    // The center of the filter is at the end of the left wing (RESAMPLER_ZERO_CROSSINGS - 1)
    // The left wing is the filter, but reversed
    // The right wing is the filter, but offset by 1
    //
    // Since the right wing is offset by 1, this just means we interpolate backwards
    // between the same points, instead of forwards
    // interp(p[n], p[n+1], t) = interp(p[n+1], p[n+1-1], 1 - t) = interp(p[n+1], p[n], 1 - t)
    let mut table: FilterTable =
        [[[0.0; 4]; RESAMPLER_SAMPLES_PER_FRAME]; RESAMPLER_SAMPLES_PER_ZERO_CROSSING];
    for i in 0..RESAMPLER_SAMPLES_PER_ZERO_CROSSING {
        for j in 0..RESAMPLER_ZERO_CROSSINGS {
            let o = ((j * RESAMPLER_SAMPLES_PER_ZERO_CROSSING) + i) * 3;
            let ys = &filter[o..o + 4];

            // Calculate the cubic equation of the 4 points
            table[i][RESAMPLER_ZERO_CROSSINGS - j - 1] =
                cubic_least_squares(ys[0], ys[1], ys[2], ys[3]);
            table[RESAMPLER_SAMPLES_PER_ZERO_CROSSING - i - 1][RESAMPLER_ZERO_CROSSINGS + j] =
                cubic_least_squares(ys[3], ys[2], ys[1], ys[0]);
        }
    }

    // Transpose each set of 4 coefficients, to reduce work when resampling
    // (Transpose4x4())
    for row in table.iter_mut() {
        for group in row.chunks_exact_mut(4) {
            let temp = [group[0], group[1], group[2], group[3]];
            for (i, g) in group.iter_mut().enumerate() {
                for (j, t) in temp.iter().enumerate() {
                    g[j] = t[i];
                }
            }
        }
    }

    table
}

/// Must be called at least once before resampling. Translation of `SDL_SetupAudioResampler()`.
pub(crate) fn setup_audio_resampler() -> &'static FilterTable {
    RESAMPLER_FILTER.get_or_init(generate_resampler_filter)
}

/// The resample rate (32:32 fixed point step per output frame).
/// Translation of `SDL_GetResampleRate()`.
pub(crate) fn get_resample_rate(src_rate: i32, dst_rate: i32) -> i64 {
    crate::sdl_assert!(src_rate > 0);
    crate::sdl_assert!(dst_rate > 0);

    let numerator = (src_rate as i64) << 32;
    let denominator = dst_rate as i64;

    // Generally it's expected that `dst_frames = (src_frames * dst_rate) / src_rate`
    // To match this as closely as possible without infinite precision, always round up the resample rate.
    // For example, without rounding up, a sample ratio of 2:3 would have `sample_rate = 0xAAAAAAAA`
    // After 3 frames, the position would be 0x1.FFFFFFFE, meaning we haven't fully consumed the second input frame.
    // By rounding up to 0xAAAAAAAB, we would instead reach 0x2.00000001, fulling consuming the second frame.
    // Technically you could say this is kicking the can 0x100000000 steps down the road, but I'm fine with that :)
    // sample_rate = div_ceil(numerator, denominator)
    let sample_rate = ((numerator - 1) / denominator) + 1;

    crate::sdl_assert!(sample_rate > 0);

    sample_rate
}

/// Translation of `SDL_GetResamplerHistoryFrames()`.
pub(crate) fn get_resampler_history_frames() -> i32 {
    // Even if we aren't currently resampling, make sure to keep enough history in case we need to later.

    RESAMPLER_MAX_PADDING_FRAMES
}

/// Translation of `SDL_GetResamplerPaddingFrames()`.
pub(crate) fn get_resampler_padding_frames(resample_rate: i64) -> i32 {
    // This must always be <= SDL_GetResamplerHistoryFrames()

    if resample_rate != 0 {
        RESAMPLER_MAX_PADDING_FRAMES
    } else {
        0
    }
}

// These are not general purpose. They do not check for all possible underflow/overflow
/// Translation of `ResamplerAdd()`.
#[inline(always)]
fn resampler_add(a: i64, b: i64) -> Option<i64> {
    if b > 0 && a > i64::MAX - b {
        return None;
    }
    Some(a.wrapping_add(b))
}

/// Translation of `ResamplerMul()`.
#[inline(always)]
fn resampler_mul(a: i64, b: i64) -> Option<i64> {
    if b > 0 && a > i64::MAX / b {
        return None;
    }
    Some(a.wrapping_mul(b))
}

/// Translation of `SDL_GetResamplerInputFrames()`.
pub(crate) fn get_resampler_input_frames(
    output_frames: i64,
    resample_rate: i64,
    resample_offset: i64,
) -> i64 {
    // Calculate the index of the last input frame, then add 1.
    // ((((output_frames - 1) * resample_rate) + resample_offset) >> 32) + 1

    let output_offset = resampler_mul(output_frames, resample_rate)
        .and_then(|o| resampler_add(o, -resample_rate + resample_offset + 0x1_0000_0000))
        .unwrap_or(i64::MAX);

    let input_frames = ((output_offset >> 32) as i32) as i64;
    input_frames.max(0)
}

/// Translation of `SDL_GetResamplerOutputFrames()`.
pub(crate) fn get_resampler_output_frames(
    input_frames: i64,
    resample_rate: i64,
    inout_resample_offset: &mut i64,
) -> i64 {
    let resample_offset = *inout_resample_offset;

    // input_offset = (input_frames << 32) - resample_offset;
    let input_offset = resampler_mul(input_frames, 0x1_0000_0000)
        .and_then(|o| resampler_add(o, -resample_offset))
        .unwrap_or(i64::MAX);

    // output_frames = div_ceil(input_offset, resample_rate)
    let output_frames = if input_offset > 0 {
        ((input_offset - 1) / resample_rate) + 1
    } else {
        0
    };

    *inout_resample_offset = (output_frames * resample_rate) - input_offset;

    output_frames
}

#[inline(always)]
fn read_f32(buf: &[u8], i: usize) -> f32 {
    f32::from_ne_bytes([buf[i * 4], buf[i * 4 + 1], buf[i * 4 + 2], buf[i * 4 + 3]])
}

/// Resample some audio. Translation of `SDL_ResampleAudio()`.
///
/// The `inframes` input frames of `chans` native `f32` samples start at byte
/// offset `src_offset` of `src`.
///
/// REQUIRES: `inframes >= get_resampler_input_frames(outframes)`
/// REQUIRES: At least `get_resampler_padding_frames(...)` extra frames to the
/// left of the input, and right of input+inframes.
#[allow(clippy::too_many_arguments)]
pub(crate) fn resample_audio(
    chans: usize,
    src: &[u8],
    src_offset: usize,
    inframes: i32,
    dst: &mut [u8],
    outframes: usize,
    resample_rate: i64,
    inout_resample_offset: &mut i64,
) {
    let filters = setup_audio_resampler();
    let mut srcpos = *inout_resample_offset;

    crate::sdl_assert!(resample_rate > 0);

    // (src -= (RESAMPLER_ZERO_CROSSINGS - 1) * chans, in samples)
    let src_base = (src_offset / 4) as isize - ((RESAMPLER_ZERO_CROSSINGS - 1) * chans) as isize;

    let mut out = [0.0f32; 8];
    for i in 0..outframes {
        let srcindex = (srcpos >> 32) as i32;
        let srcfraction = (srcpos & 0xFFFF_FFFF) as u32;
        srcpos += resample_rate;

        crate::sdl_assert!(srcindex >= -1 && srcindex < inframes);

        let filter = &filters[(srcfraction >> RESAMPLER_FILTER_INTERP_BITS) as usize];
        let frac = (srcfraction & (RESAMPLER_FILTER_INTERP_RANGE - 1)) as f32
            * (1.0 / RESAMPLER_FILTER_INTERP_RANGE as f32);

        let frame = (src_base + srcindex as isize * chans as isize) as usize;
        resample_frame(
            |k| read_f32(src, frame + k),
            &mut out[..chans],
            filter,
            frac,
            chans,
        );

        let d = i * chans * 4;
        for (c, v) in out[..chans].iter().enumerate() {
            dst[d + c * 4..d + c * 4 + 4].copy_from_slice(&v.to_ne_bytes());
        }
    }

    *inout_resample_offset = srcpos - ((inframes as i64) << 32);
}
