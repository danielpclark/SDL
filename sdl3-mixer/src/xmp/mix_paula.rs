// Rust translation of src/mix_paula.c and src/paula.h from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2025 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Amiga Paula simulator (`LIBXMP_PAULA_SIMULATOR`, for Amiga modules
//! played with `XMP_FLAGS_A500`).
//!
//! Based on Antti S. Lankila's reference code, modified for libxmp
//! by Claudio Matsuoka.

use super::common::*;
use super::mix_all::MixerFp;
use super::mixer::{MixerVoice, SMIX_MASK, SMIX_SHIFT};
use super::precomp_blep::WINSINC_INTEGRAL;
use super::SAMPLE_GUARD;

/* 131072 to 0, 2048 entries */
pub(crate) const PAULA_HZ: i32 = 3546895;
pub(crate) const MINIMUM_INTERVAL: i32 = 16;
pub(crate) const BLEP_SCALE: i32 = 17;
pub(crate) const BLEP_SIZE: i32 = 2048;
pub(crate) const MAX_BLEPS: usize = (BLEP_SIZE / MINIMUM_INTERVAL) as usize;

/* the structure that holds data of bleps */
/// Translation of `struct blep_state`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct BlepState {
    pub level: i16,
    pub age: i16,
}

/// Translation of `struct paula_state`.
#[derive(Clone, Debug)]
pub(crate) struct PaulaState {
    /* the instantaneous value of Paula output */
    pub global_output_level: i16,

    /* count of simultaneous bleps to keep track of */
    pub active_bleps: u32,

    /* place to keep our bleps in. MAX_BLEPS should be
     * defined as a BLEP_SIZE / MINIMUM_EVENT_INTERVAL.
     * For Paula, minimum event interval could be even 1, but it makes
     * sense to limit it to some higher value such as 16. */
    pub blepstate: [BlepState; MAX_BLEPS],

    pub remainder: f64,
    pub fdiv: f64,
}

impl Default for PaulaState {
    fn default() -> Self {
        PaulaState {
            global_output_level: 0,
            active_bleps: 0,
            blepstate: [BlepState::default(); MAX_BLEPS],
            remainder: 0.0,
            fdiv: 0.0,
        }
    }
}

/// Translation of `libxmp_paula_init()`.
pub(crate) fn libxmp_paula_init(ctx: &ContextData, paula: &mut PaulaState) {
    let s = &ctx.s;

    paula.global_output_level = 0;
    paula.active_bleps = 0;
    paula.fdiv = PAULA_HZ as f64 / s.freq as f64;
    paula.remainder = paula.fdiv;
}

/* return output simulated as series of bleps */
/// Translation of `output_sample()`.
fn output_sample(paula: &PaulaState, tabnum: usize) -> i16 {
    let mut output: i32 = (paula.global_output_level as i32) << BLEP_SCALE;
    for i in 0..paula.active_bleps as usize {
        let age = paula.blepstate[i].age as i32;
        let level = paula.blepstate[i].level as i32;
        output = output.wrapping_sub(WINSINC_INTEGRAL[tabnum][age as usize].wrapping_mul(level));
    }
    output >>= BLEP_SCALE;

    if output < -32768 {
        output = -32768;
    } else if output > 32767 {
        output = 32767;
    }

    output as i16
}

/// Translation of `input_sample()`.
fn input_sample(paula: &mut PaulaState, sample: i16) {
    if sample != paula.global_output_level {
        /* Start a new blep: level is the difference, age (or phase) is 0 clocks. */
        if paula.active_bleps > MAX_BLEPS as u32 - 1 {
            paula.active_bleps = MAX_BLEPS as u32 - 1;
        }

        /* Make room for new blep */
        let n = paula.active_bleps as usize;
        paula.blepstate.copy_within(0..n, 1);

        /* Update state to account for the new blep */
        paula.active_bleps += 1;
        paula.blepstate[0].age = 0;
        paula.blepstate[0].level = (sample as i32 - paula.global_output_level as i32) as i16;
        paula.global_output_level = sample;
    }
}

/// Translation of `do_clock()`.
fn do_clock(paula: &mut PaulaState, cycles: i32) {
    if cycles <= 0 {
        return;
    }

    for i in 0..paula.active_bleps as usize {
        paula.blepstate[i].age = (paula.blepstate[i].age as i32 + cycles) as i16;
        if paula.blepstate[i].age as i32 >= BLEP_SIZE {
            paula.active_bleps = i as u32;
            break;
        }
    }
}

/// The `MIXER()` functions: `libxmp_mix_{monoout,stereoout}_mono_a500[_filter]()`.
fn mix_a500<const STEREOOUT: bool, const TAB: usize>(
    vi: &mut MixerVoice,
    buffer: &mut [i32],
    sptr: &mut [u8],
    mut count: i32,
    mut vl: i32,
    mut vr: i32,
    step: i32,
    _ramp: i32,
    _delta_l: i32,
    _delta_r: i32,
) {
    // VAR_NORM(int8)
    let mut pos: u32 = vi.pos as u32;
    let mut frac: i32 = ((1 << SMIX_SHIFT) as f64 * (vi.pos - (vi.pos as i32) as f64)) as i32;
    // VAR_PAULA_MONO / VAR_PAULA
    vl <<= 8;
    vr <<= 8;
    let mut b = 0usize;
    let paula = vi.paula.as_mut().unwrap();
    /* Note (upstream): the steps between the output samples can reach
     * past the end of the sample's data (and its guard bytes), where
     * upstream reads out of bounds; here that reads 0. */
    let smp_at = |pos: u32| {
        sptr.get(SAMPLE_GUARD + pos as usize)
            .map_or(0, |&b| b as i8 as i16)
    };

    while count != 0 {
        // PAULA_SIMULATION(x)
        let num_in = (paula.remainder / MINIMUM_INTERVAL as f64) as i32;
        let ministep = step / num_in;

        /* input is always sampled at a higher rate than output */
        for _ in 0..num_in - 1 {
            input_sample(paula, smp_at(pos));
            do_clock(paula, MINIMUM_INTERVAL);
            // UPDATE_POS(ministep)
            frac += ministep;
            pos = pos.wrapping_add((frac >> SMIX_SHIFT) as u32);
            frac &= SMIX_MASK;
        }
        input_sample(paula, smp_at(pos));
        paula.remainder -= (num_in * MINIMUM_INTERVAL) as f64;

        do_clock(paula, paula.remainder as i32);
        let smp_in = output_sample(paula, TAB) as i32;
        do_clock(paula, MINIMUM_INTERVAL - paula.remainder as i32);
        // UPDATE_POS(step - (num_in - 1) * ministep)
        frac += step - (num_in - 1) * ministep;
        pos = pos.wrapping_add((frac >> SMIX_SHIFT) as u32);
        frac &= SMIX_MASK;

        paula.remainder += paula.fdiv;

        if STEREOOUT {
            // MIX_STEREO()
            buffer[b] = buffer[b].wrapping_add(smp_in.wrapping_mul(vl));
            b += 1;
            buffer[b] = buffer[b].wrapping_add(smp_in.wrapping_mul(vr));
            b += 1;
        } else {
            // MIX_MONO()
            buffer[b] = buffer[b].wrapping_add(smp_in.wrapping_mul(vl));
            b += 1;
        }
        count -= 1;
    }
}

/// `LIST_MIX_FUNCTIONS_PAULA(type)`
const fn list_paula(monoout: MixerFp, stereoout: MixerFp) -> [Option<MixerFp>; 16] {
    let mut out: [Option<MixerFp>; 16] = [None; 16];
    out[0] = Some(monoout);
    out[4] = Some(stereoout);
    out
}

/// Translation of `libxmp_a500_mixers[]`.
pub(crate) static LIBXMP_A500_MIXERS: [Option<MixerFp>; 16] =
    list_paula(mix_a500::<false, 0>, mix_a500::<true, 0>);

/// Translation of `libxmp_a500led_mixers[]`.
pub(crate) static LIBXMP_A500LED_MIXERS: [Option<MixerFp>; 16] =
    list_paula(mix_a500::<false, 1>, mix_a500::<true, 1>);
