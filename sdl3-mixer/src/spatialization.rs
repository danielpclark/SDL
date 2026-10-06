// Rust translation of src/SDL_mixer_spatialization.c from SDL_mixer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

// Most of this code was originally from MojoAL (https://github.com/icculus/mojoAL),
// either written by me under the zlib license, or offered by others in the public domain.

//! 3D positioning: Vector Based Amplitude Panning for surround sound, and
//! constant power panning for stereo.
//!
//! Upstream has scalar, SSE and NEON versions of the vector math, picked at
//! build time; x86 builds always take the SSE one, whose dot product adds
//! the lanes in a different order than the scalar code. This translation
//! computes the SSE version's arithmetic in portable code, so its panning
//! matches upstream's x86 builds bit for bit everywhere.

use sdl3::stdlib::math::{acosf, cosf, floorf, sinf, sqrtf, PI_F};

// Vector Based Amplitude Panning stuff, for surround sound positional audio.
// VBAP code originally from https://github.com/drbafflegab/vbap/ ... CC0 license (public domain).
pub(crate) const VBAP2D_MAX_RESOLUTION: i32 = 3600;
pub(crate) const VBAP2D_MAX_SPEAKER_COUNT: usize = 8; // original code had 64, assumed you'd use less, but we're hardcoding our current maximum.
pub(crate) const VBAP2D_RESOLUTION: i32 = 36; // 10 degrees per division

/// Translation of `MIX_VBAP2D_Bucket`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Vbap2dBucket {
    speaker_pair: u8,
}

/// Translation of `MIX_VBAP2D_Matrix`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Vbap2dMatrix {
    a00: f32,
    a01: f32,
    a10: f32,
    a11: f32,
}

/// Translation of `MIX_VBAP2D`.
#[derive(Clone, Debug)]
pub(crate) struct Vbap2d {
    speaker_count: i32,
    buckets: [Vbap2dBucket; VBAP2D_RESOLUTION as usize],
    matrices: [Vbap2dMatrix; VBAP2D_MAX_SPEAKER_COUNT - 1], // the upper ones all have an LFE channel, which we don't track here, so minus one.
}

impl Default for Vbap2d {
    fn default() -> Self {
        Vbap2d {
            speaker_count: 0,
            buckets: [Vbap2dBucket::default(); VBAP2D_RESOLUTION as usize],
            matrices: [Vbap2dMatrix::default(); VBAP2D_MAX_SPEAKER_COUNT - 1],
        }
    }
}

/// Translation of `MIX_VBAP2D_division_to_angle()`.
fn vbap2d_division_to_angle(division: i32) -> f32 {
    division as f32 * (2.0 * PI_F) / VBAP2D_RESOLUTION as f32
}

/// Translation of `MIX_VBAP2D_angle_to_span()`.
fn vbap2d_angle_to_span(angle: f32) -> i32 {
    floorf(angle * VBAP2D_RESOLUTION as f32 / (2.0 * PI_F)) as i32
}

/// Translation of `MIX_VBAP2D_contains()`.
fn vbap2d_contains(division: i32, last_division: i32, next_division: i32) -> bool {
    if last_division < next_division {
        last_division <= division && division < next_division
    } else {
        let cond_a = 0 <= division && division < next_division;
        let cond_b = last_division <= division && division < VBAP2D_RESOLUTION;
        cond_a || cond_b
    }
}

/// Translation of `MIX_VBAP2D_unpack_speaker_pair()`.
fn vbap2d_unpack_speaker_pair(speaker_pair: i32, speaker_count: i32) -> [i32; 2] {
    [
        (if speaker_pair == 0 {
            speaker_count
        } else {
            speaker_pair
        }) - 1,
        speaker_pair,
    ]
}

/// Translation of `MIX_VBAP2D_SpeakerPosition`.
#[derive(Clone, Copy)]
struct Vbap2dSpeakerPosition {
    division: u8, // this is in degrees--positive to the left--divided by the resolution. RESOLUTION MUST BE AT LEAST 2 TO FIT IN UINT8!
    sdl_channel: u8, // the channel in SDL's layout (in stereo: {left=0, right=1}...etc).
}

/// Translation of `MIX_VBAP2D_SpeakerLayout`.
struct Vbap2dSpeakerLayout {
    positions: &'static [Vbap2dSpeakerPosition],
    lfe_channel: i32,
}

// these have to go from smallest to largest angle, I think...
const fn p(angle: f64, sdl_channel: u8) -> Vbap2dSpeakerPosition {
    Vbap2dSpeakerPosition {
        division: ((angle / 360.0) * VBAP2D_RESOLUTION as f64) as u8,
        sdl_channel,
    }
}
static SPEAKER_POSITIONS_QUAD: [Vbap2dSpeakerPosition; 4] =
    [p(45.0, 1), p(135.0, 0), p(225.0, 2), p(315.0, 3)];
static SPEAKER_POSITIONS_4_1: [Vbap2dSpeakerPosition; 4] =
    [p(45.0, 1), p(135.0, 0), p(225.0, 3), p(315.0, 4)];
static SPEAKER_POSITIONS_5_1: [Vbap2dSpeakerPosition; 5] = [
    p(60.0, 1),
    p(90.0, 2),
    p(120.0, 0),
    p(240.0, 4),
    p(300.0, 5),
];
static SPEAKER_POSITIONS_6_1: [Vbap2dSpeakerPosition; 6] = [
    p(60.0, 1),
    p(90.0, 2),
    p(120.0, 0),
    p(190.0, 5),
    p(270.0, 4),
    p(350.0, 6),
];
static SPEAKER_POSITIONS_7_1: [Vbap2dSpeakerPosition; 7] = [
    p(0.0, 7),
    p(60.0, 1),
    p(90.0, 2),
    p(120.0, 0),
    p(200.0, 6),
    p(240.0, 4),
    p(300.0, 5),
];
static SPEAKER_LAYOUTS: [Vbap2dSpeakerLayout; VBAP2D_MAX_SPEAKER_COUNT - 3] = [
    // -3 to skip mono/stereo/2.1
    Vbap2dSpeakerLayout {
        positions: &SPEAKER_POSITIONS_QUAD,
        lfe_channel: -1,
    },
    Vbap2dSpeakerLayout {
        positions: &SPEAKER_POSITIONS_4_1,
        lfe_channel: 2,
    },
    Vbap2dSpeakerLayout {
        positions: &SPEAKER_POSITIONS_5_1,
        lfe_channel: 3,
    },
    Vbap2dSpeakerLayout {
        positions: &SPEAKER_POSITIONS_6_1,
        lfe_channel: 3,
    },
    Vbap2dSpeakerLayout {
        positions: &SPEAKER_POSITIONS_7_1,
        lfe_channel: 3,
    },
];

impl Vbap2d {
    /// Translation of `MIX_VBAP2D_Init()`.
    pub(crate) fn init(&mut self, mut speaker_count: i32) {
        debug_assert!(speaker_count > 0);
        debug_assert!(speaker_count <= VBAP2D_MAX_SPEAKER_COUNT as i32);
        const _: () = assert!(VBAP2D_RESOLUTION <= VBAP2D_MAX_RESOLUTION);

        self.speaker_count = speaker_count;

        if speaker_count < 4 {
            return; // no VBAP for mono, stereo, or 2.1.
        }
        // (more channels than SDL's layouts don't get VBAP either, where
        // upstream would index past its table.)
        let Some(speaker_layout) = SPEAKER_LAYOUTS.get((speaker_count - 4) as usize) else {
            return;
        };
        let speaker_positions = speaker_layout.positions;

        if speaker_layout.lfe_channel >= 0 {
            speaker_count -= 1; // for our purposes, collapse out the subwoofer channel
        }

        let mut speaker_pair = 0;
        for division in 0..VBAP2D_RESOLUTION {
            let speakers = vbap2d_unpack_speaker_pair(speaker_pair, speaker_count);
            let last_division = speaker_positions[speakers[0] as usize].division as i32;
            let next_division = speaker_positions[speakers[1] as usize].division as i32;

            if !vbap2d_contains(division, last_division, next_division) {
                speaker_pair = (speaker_pair + 1) % speaker_count;
            }

            self.buckets[division as usize].speaker_pair = speaker_pair as u8;
        }

        for speaker_pair in 0..speaker_count {
            let speakers = vbap2d_unpack_speaker_pair(speaker_pair, speaker_count);
            let last_division = speaker_positions[speakers[0] as usize].division as i32;
            let next_division = speaker_positions[speakers[1] as usize].division as i32;
            let last_angle = vbap2d_division_to_angle(last_division);
            let next_angle = vbap2d_division_to_angle(next_division);
            let a00 = cosf(last_angle);
            let a01 = cosf(next_angle);
            let a10 = sinf(last_angle);
            let a11 = sinf(next_angle);
            let det = 1.0 / (a00 * a11 - a01 * a10);

            let m = &mut self.matrices[speaker_pair as usize];
            m.a00 = a11 * det;
            m.a01 = -a01 * det;
            m.a10 = -a10 * det;
            m.a11 = a00 * det;
        }
    }

    /// Translation of `MIX_VBAP2D_CalculateGains()`.
    fn calculate_gains(
        &self,
        mut source_angle: f32,
        gains: &mut [f32; 2],
        speakers: &mut [i32; 2],
    ) {
        let mut speaker_count = self.speaker_count;
        debug_assert!(speaker_count >= 4);

        let Some(speaker_layout) = SPEAKER_LAYOUTS.get((speaker_count - 4) as usize) else {
            return;
        }; // offset to zero, skip mono/stereo/2.1

        if speaker_layout.lfe_channel >= 0 {
            speaker_count -= 1; // for our purposes, collapse out the subwoofer channel
        }

        // shift so angle 0 is due east instead of due north, and normalize it to the 0 to 2pi range.
        source_angle += PI_F / 2.0;

        while source_angle < 0.0 {
            source_angle += 2.0 * PI_F;
        }
        while source_angle > (2.0 * PI_F) {
            source_angle -= 2.0 * PI_F;
        }

        let source_x = cosf(source_angle);
        let source_y = sinf(source_angle);
        // (an angle of exactly 2pi lands one past the last bucket; upstream
        // reads past its array there. Wrap it to the first.)
        let span = vbap2d_angle_to_span(source_angle).rem_euclid(VBAP2D_RESOLUTION);
        let speaker_pair = self.buckets[span as usize].speaker_pair as i32;

        let vbap_speakers = vbap2d_unpack_speaker_pair(speaker_pair, speaker_count);

        let matrix = &self.matrices[speaker_pair as usize];
        let gain_a = source_x * matrix.a00 + source_y * matrix.a01;
        let gain_b = source_x * matrix.a10 + source_y * matrix.a11;

        let scale = 1.0 / sqrtf(gain_a * gain_a + gain_b * gain_b);

        let gain_a_normalized = gain_a * scale;
        let gain_b_normalized = gain_b * scale;

        speakers[0] = speaker_layout.positions[vbap_speakers[0] as usize].sdl_channel as i32;
        speakers[1] = speaker_layout.positions[vbap_speakers[1] as usize].sdl_channel as i32;
        gains[0] = gain_a_normalized;
        gains[1] = gain_b_normalized;
    }

    pub(crate) fn speaker_count(&self) -> i32 {
        self.speaker_count
    }
}

// end VBAP code.

// All the 3D math here is way overcommented because I HAVE NO IDEA WHAT I'M
//  DOING and had to research the hell out of what are probably pretty simple
//  concepts. Pay attention in math class, kids.

// The scalar versions have explanitory comments and links. The SIMD versions don't.

/// Translation of `calculate_distance_attenuation()`.
fn calculate_distance_attenuation(distance: f32) -> f32 {
    // we use the OpenAL default distance model (AL_INVERSE_DISTANCE_CLAMPED), with a reference distance and rolloff factor of 1.0f (the defaults).
    // this collapses a ton of work out of this code that MojoAL had to do.
    1.0 / (1.0 + (distance.max(1.0) - 1.0))
}

static LISTENER_AT: [f32; 4] = [0.0, 0.0, -1.0, 0.0]; // default "at" for OpenAL listener orientation matrix.
static LISTENER_UP: [f32; 4] = [0.0, 1.0, 0.0, 0.0]; // default "up" for OpenAL listener orientation matrix.

//  XYZZY!! https://en.wikipedia.org/wiki/Cross_product#Mnemonic
//
//  Calculates cross product. https://en.wikipedia.org/wiki/Cross_product
//  Basically takes two vectors and gives you a vector that's perpendicular
//  to both.
/// Translation of `xyzzy_sse()` (the same arithmetic as the scalar
/// `xyzzy()`: `_mm_sub_ps(_mm_mul_ps(a, b.yzx), _mm_mul_ps(b, a.yzx)).yzx`).
fn xyzzy(a: &[f32; 4], b: &[f32; 4]) -> [f32; 4] {
    // http://fastcpp.blogspot.com/2011/04/vector-cross-product-using-sse-code.html
    //    this is the "three shuffle" version in the comments, plus the variables swapped around for handedness in the later comment.
    let v = [
        a[0] * b[1] - b[0] * a[1],
        a[1] * b[2] - b[1] * a[2],
        a[2] * b[0] - b[2] * a[0],
        a[3] * b[3] - b[3] * a[3],
    ];
    [v[1], v[2], v[0], v[3]]
}

// calculate dot product (multiply each element of two vectors, sum them)
/// Translation of `dotproduct_sse()`: the lanes are summed as
/// `(p3 + p1) + (p2 + p0)`, not `(p0 + p1) + p2` as in the scalar
/// `dotproduct()`.
fn dotproduct(a: &[f32; 4], b: &[f32; 4]) -> f32 {
    let prod = [a[0] * b[0], a[1] * b[1], a[2] * b[2], a[3] * b[3]];
    // sum1 = prod + prod.zwxy
    let sum1 = [
        prod[0] + prod[2],
        prod[1] + prod[3],
        prod[2] + prod[0],
        prod[3] + prod[1],
    ];
    // sum2 = sum1 + sum1.xxzz; lane 3 is the result.
    // FIXME: this can use _mm_hadd_ps in SSE3, or _mm_dp_ps in SSE4.1
    sum1[3] + sum1[2]
}

// calculate distance ("magnitude") in 3D space:
//  https://math.stackexchange.com/questions/42640/calculate-distance-in-3d-space
//  assumes vector starts at (0,0,0).
/// Translation of `magnitude_sse()`.
fn magnitude(v: &[f32; 4]) -> f32 {
    // technically, the inital part on this is just a dot product of itself.
    sqrtf(dotproduct(v, v))
}

/// Translation of `calculate_distance_attenuation_and_angle_sse()` (see
/// the module docs), with the scalar version's comments.
fn calculate_distance_attenuation_and_angle(position: &[f32; 4]) -> (f32, f32) {
    // this goes through most of the steps the AL spec dictates for gain and distance attenuation...

    // Remove upwards component so it lies completely within the horizontal plane.
    let a = dotproduct(position, &LISTENER_UP);
    let v = [
        position[0] - (a * LISTENER_UP[0]),
        position[1] - (a * LISTENER_UP[1]),
        position[2] - (a * LISTENER_UP[2]),
        position[3] - (a * LISTENER_UP[3]),
    ];

    // Calculate angle
    let mags = magnitude(&LISTENER_AT) * magnitude(&v);
    let mut radians = if mags == 0.0 {
        0.0
    } else {
        let cosangle = dotproduct(&LISTENER_AT, &v) / mags;
        acosf(cosangle.clamp(-1.0, 1.0))
    };

    // Get "right" vector
    let r = xyzzy(&LISTENER_AT, &LISTENER_UP);

    // make it negative to the left, positive to the right.
    if dotproduct(&r, &v) < 0.0 {
        radians = -radians;
    }

    (calculate_distance_attenuation(magnitude(position)), radians)
}

// Get the sin(angle) and cos(angle) at the same time. Ideally, with one
//  instruction, like what is offered on the x86.
//  angle is in radians, not degrees.
/// Translation of `calculate_sincos()`.
fn calculate_sincos(angle: f32) -> (f32, f32) {
    // (of course, FSINCOS uses the floating point registers, so we're
    //  currently opting to favor portability by using the SDL_* functions.)
    (sinf(angle), cosf(angle))
}

/// Translation of `MIX_Spatialize()`: fill in `panning` and `speakers`, the
/// two speakers to write to and at what gain. `position` holds X, Y, Z
/// and a zero.
pub(crate) fn spatialize(
    vbap2d: &Vbap2d,
    position: &[f32; 4],
    panning: &mut [f32; 2],
    speakers: &mut [i32; 2],
) {
    let output_channels = vbap2d.speaker_count;

    debug_assert!(output_channels > 0);

    let (gain, radians) = calculate_distance_attenuation_and_angle(position);

    if output_channels == 1 {
        // no positioning for mono output, just distance attenuation.
        speakers[0] = 0;
        speakers[1] = 0;
        panning[0] = gain;
        panning[1] = 0.0;
    } else if (output_channels == 2) || (output_channels == 3) {
        // stereo (and 2.1) output uses Constant Power Panning.
        speakers[0] = 0;
        speakers[1] = 1;

        // here comes the Constant Power Panning magic...
        const SQRT2_DIV2: f32 = 0.7071067812; // sqrt(2.0) / 2.0 ...

        // this might be a terrible idea, which is totally my own doing here,
        // but here you go: Constant Power Panning only works from -45 to 45
        // degrees in front of the listener. So we split this into 4 quadrants.

        //   - from -45 to 45: standard panning.
        //   - from 45 to 135: pan full right.
        //   - from 135 to 225: flip angle so it works like standard panning.
        //   - from 225 to -45: pan full left.

        const RADIANS_45_DEGREES: f32 = 0.7853981634;
        const RADIANS_135_DEGREES: f32 = 2.3561944902;
        if (-RADIANS_45_DEGREES..=RADIANS_45_DEGREES).contains(&radians) {
            let (sine, cosine) = calculate_sincos(radians);
            panning[0] = SQRT2_DIV2 * (cosine - sine);
            panning[1] = SQRT2_DIV2 * (cosine + sine);
        } else if (RADIANS_45_DEGREES..=RADIANS_135_DEGREES).contains(&radians) {
            panning[0] = 0.0;
            panning[1] = 1.0;
        } else if (-RADIANS_135_DEGREES..=-RADIANS_45_DEGREES).contains(&radians) {
            panning[0] = 1.0;
            panning[1] = 0.0;
        } else if radians < 0.0 {
            // back left
            let (sine, cosine) = calculate_sincos(-(radians + PI_F));
            panning[0] = SQRT2_DIV2 * (cosine - sine);
            panning[1] = SQRT2_DIV2 * (cosine + sine);
        } else {
            // back right
            let (sine, cosine) = calculate_sincos(-(radians - PI_F));
            panning[0] = SQRT2_DIV2 * (cosine - sine);
            panning[1] = SQRT2_DIV2 * (cosine + sine);
        }

        // apply distance attenuation and gain to positioning.
        panning[0] *= gain;
        panning[1] *= gain;
    } else {
        // surround-sound (output_channels >= 4)
        // we're going negative to the _right_ here, at the moment, so negative radians.
        vbap2d.calculate_gains(-radians, panning, speakers);

        // apply distance attenuation and gain to positioning.
        panning[0] *= gain;
        panning[1] *= gain;
    }
}
