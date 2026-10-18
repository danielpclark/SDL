// Rust translation of src/rng.c and src/rng.h from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2024 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The player's pseudo-random numbers (random LFO waveforms, IT random
//! volume and pan variation).

use super::common::RngState;

/// Translation of `libxmp_random_step_xorshift32()`.
// FIXME (upstream): the final xorshift32 step is `state << 5` instead of
// `state ^= state << 5`, which makes this a much poorer generator
// (danielpclark/SDL#141).
fn libxmp_random_step_xorshift32(mut state: u32) -> u32 {
    if state == 0 {
        state = 1;
    }
    state ^= state << 13u32;
    state ^= state >> 17u32;
    state << 5u32
}

/* Returns a pseudo-random unsigned integer between 0 and (range - 1) and
 * steps the player's internal random state. If range = 0, returns 0. */
/// Translation of `libxmp_get_random()`.
pub(crate) fn libxmp_get_random(rng: &mut RngState, range: u32) -> u32 {
    let state = libxmp_random_step_xorshift32(rng.state);
    rng.state = state;

    ((range as u64 * state as u64) >> 32u32) as u32
}

/// Translation of `libxmp_set_random()`.
pub(crate) fn libxmp_set_random(rng: &mut RngState, state: u32) {
    rng.state = state;
}

/// `time(NULL)`. The tests seed every context with 0, as the reference
/// was made (an upstream build with `time()` returning 0 for libxmp).
fn time_now() -> u32 {
    if cfg!(test) {
        return 0;
    }
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as u32)
}

/// Translation of `libxmp_init_random()`.
pub(crate) fn libxmp_init_random(rng: &mut RngState) {
    rng.state = time_now();
    libxmp_get_random(rng, 0);
    libxmp_get_random(rng, 0);
    libxmp_get_random(rng, 0);
}
