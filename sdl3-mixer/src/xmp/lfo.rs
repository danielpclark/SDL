// Rust translation of src/lfo.c and src/lfo.h from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2024 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Low frequency oscillators: vibrato, tremolo, panbrello.

use super::common::*;
use super::rng::libxmp_get_random;

/// Translation of `struct lfo`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Lfo {
    pub type_: i32,
    pub rate: i32,
    pub depth: i32,
    pub phase: i32,
}

const WAVEFORM_SIZE: i32 = 64;

static SINE_WAVE: [i32; WAVEFORM_SIZE as usize] = [
    0, 24, 49, 74, 97, 120, 141, 161, 180, 197, 212, 224, 235, 244, 250, 253, 255, 253, 250, 244,
    235, 224, 212, 197, 180, 161, 141, 120, 97, 74, 49, 24, 0, -24, -49, -74, -97, -120, -141,
    -161, -180, -197, -212, -224, -235, -244, -250, -253, -255, -253, -250, -244, -235, -224, -212,
    -197, -180, -161, -141, -120, -97, -74, -49, -24,
];

/* LFO */

/// Translation of `get_lfo_mod()`.
fn get_lfo_mod(rng: &mut RngState, lfo: &Lfo) -> i32 {
    if lfo.rate == 0 {
        return 0;
    }

    let val = match lfo.type_ {
        0 => {
            /* sine */
            SINE_WAVE[lfo.phase as usize]
        }
        1 => {
            /* ramp down */
            255 - (lfo.phase << 3)
        }
        2 => {
            /* square */
            if lfo.phase < WAVEFORM_SIZE / 2 {
                255
            } else {
                -255
            }
        }
        3 => {
            /* random */
            libxmp_get_random(rng, 512) as i32 - 256
        }
        669 => {
            /* 669 vibrato */
            lfo.phase & 1
        }
        _ => return 0,
    };

    val.wrapping_mul(lfo.depth)
}

/// Translation of `get_lfo_st3()`.
fn get_lfo_st3(rng: &mut RngState, lfo: &Lfo) -> i32 {
    if lfo.rate == 0 {
        return 0;
    }

    /* S3M square */
    if lfo.type_ == 2 {
        let val = if lfo.phase < WAVEFORM_SIZE / 2 {
            255
        } else {
            0
        };
        return val * lfo.depth;
    }

    get_lfo_mod(rng, lfo)
}

/* From OpenMPT VibratoWaveforms.xm:
 * "Generally the vibrato and tremolo tables are identical to those that
 *  ProTracker uses, but the vibrato’s “ramp down” table is upside down."
 */
/// Translation of `get_lfo_ft2()`.
fn get_lfo_ft2(rng: &mut RngState, lfo: &Lfo) -> i32 {
    if lfo.rate == 0 {
        return 0;
    }

    /* FT2 ramp */
    if lfo.type_ == 1 {
        let phase = (lfo.phase + (WAVEFORM_SIZE >> 1)) % WAVEFORM_SIZE;
        let val = (phase << 3) - 255;
        return val * lfo.depth;
    }

    get_lfo_mod(rng, lfo)
}

/// Translation of `get_lfo_it()`.
fn get_lfo_it(rng: &mut RngState, lfo: &Lfo) -> i32 {
    if lfo.rate == 0 {
        return 0;
    }

    get_lfo_st3(rng, lfo)
}

/// Translation of `libxmp_lfo_get()` (its context is the module's
/// `read_event_type` and the random state).
pub(crate) fn libxmp_lfo_get(
    read_event_type: i32,
    rng: &mut RngState,
    lfo: &Lfo,
    is_vibrato: i32,
) -> i32 {
    match read_event_type {
        READ_EVENT_ST3 => get_lfo_st3(rng, lfo),
        READ_EVENT_FT2 => {
            if is_vibrato != 0 {
                get_lfo_ft2(rng, lfo)
            } else {
                get_lfo_mod(rng, lfo)
            }
        }
        READ_EVENT_IT => get_lfo_it(rng, lfo),
        _ => get_lfo_mod(rng, lfo),
    }
}

/// Translation of `libxmp_lfo_update()`.
pub(crate) fn libxmp_lfo_update(lfo: &mut Lfo) {
    lfo.phase = lfo.phase.wrapping_add(lfo.rate);
    lfo.phase &= WAVEFORM_SIZE - 1; /* Rate may be negative, don't %= */
}

/// Translation of `libxmp_lfo_set_phase()`.
pub(crate) fn libxmp_lfo_set_phase(lfo: &mut Lfo, phase: i32) {
    lfo.phase = phase;
}

/// Translation of `libxmp_lfo_set_depth()`.
pub(crate) fn libxmp_lfo_set_depth(lfo: &mut Lfo, depth: i32) {
    lfo.depth = depth;
}

/// Translation of `libxmp_lfo_set_rate()`.
pub(crate) fn libxmp_lfo_set_rate(lfo: &mut Lfo, rate: i32) {
    lfo.rate = rate;
}

/// Translation of `libxmp_lfo_set_waveform()`.
pub(crate) fn libxmp_lfo_set_waveform(lfo: &mut Lfo, type_: i32) {
    lfo.type_ = type_;
}
