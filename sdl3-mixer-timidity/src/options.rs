/*
    TiMidity -- Experimental MIDI to WAVE converter
    Copyright (C) 1995 Tuukka Toivonen <toivonen@clinet.fi>

    This program is free software; you can redistribute it and/or modify
    it under the terms of the Perl Artistic License, available in COPYING.
*/
// Modified 2026-10-07: translated into Rust from options.h in SDL_mixer 3.3.0's TiMidity (see NOTICE).

//! Compile-time options. Translation of `options.h`.

#![allow(dead_code)]

/* When a patch file can't be opened, one of these extensions is
  appended to the filename and the open is tried again.
*/
pub(crate) const PATCH_EXT_LIST: &[&str] = &[".pat"];

/* Acoustic Grand Piano seems to be the usual default instrument. */
pub(crate) const DEFAULT_PROGRAM: i32 = 0;

/* 9 here is MIDI channel 10, which is the standard percussion channel.
Some files (notably C:\WINDOWS\CANYON.MID) think that 16 is one too.
On the other hand, some files know that 16 is not a drum channel and
try to play music on it. This is now a runtime option, so this isn't
a critical choice anymore. */
pub(crate) const DEFAULT_DRUMCHANNELS: i32 = 1 << 9;

/* In percent. */
pub(crate) const DEFAULT_AMPLIFICATION: i32 = 70;

/* Default polyphony */
/* #define DEFAULT_VOICES	32 */
pub(crate) const DEFAULT_VOICES: i32 = 256;

/* 1000 here will give a control ratio of 22:1 with 22 kHz output.
Higher CONTROLS_PER_SECOND values allow more accurate rendering
of envelopes and tremolo. The cost is CPU time. */
pub(crate) const CONTROLS_PER_SECOND: i32 = 1000;

/* Make envelopes twice as fast. Saves ~20% CPU time (notes decay
faster) and sounds more like a GUS. */
pub(crate) const FAST_DECAY: bool = true;

/* A somewhat arbitrary output frequency range. */
pub(crate) const MIN_OUTPUT_RATE: i32 = 4000;
pub(crate) const MAX_OUTPUT_RATE: i32 = 256000;

/* How many bits to use for the fractional part of sample positions.
This affects tonal accuracy. The entire position counter must fit
in 32 bits, so with FRACTION_BITS equal to 12, the maximum size of
a sample is 1048576 samples (2 megabytes in memory). The GUS gets
by with just 9 bits and a little help from its friends...
"The GUS does not SUCK!!!" -- a happy user :) */
pub(crate) const FRACTION_BITS: i32 = 12;

/* For some reason the sample volume is always set to maximum in all
patch files. Define this for a crude adjustment that may help
equalize instrument volumes. */
pub(crate) const ADJUST_SAMPLE_VOLUMES: bool = true;

/* The number of samples to use for ramping out a dying note. Affects
click removal. */
pub(crate) const MAX_DIE_TIME: i32 = 20;

/**************************************************************************/
/* Anything below this shouldn't need to be changed unless you're porting
to a new machine with other than 32-bit, big-endian words. */
/**************************************************************************/

/* change FRACTION_BITS above, not these */
pub(crate) const INTEGER_BITS: i32 = 32 - FRACTION_BITS;
// (unsigned in C: `ofs & FRACTION_MASK` is an unsigned value, 0 to 0xFFF.)
pub(crate) const INTEGER_MASK: u32 = 0xFFFFFFFF << FRACTION_BITS;
pub(crate) const FRACTION_MASK: u32 = !INTEGER_MASK;

/* This is enforced by some computations that must fit in an int */
pub(crate) const MAX_CONTROL_RATIO: i32 = 255;

pub(crate) const MAX_AMPLIFICATION: i32 = 800;

/* The TiMidity configuration file */
pub(crate) const TIMIDITY_CFG: &str = "timidity.cfg";

/* These affect general volume */
pub(crate) const GUARD_BITS: i32 = 3;
pub(crate) const AMP_BITS: i32 = 15 - GUARD_BITS;

pub(crate) const MAX_AMP_VALUE: i32 = (1 << (AMP_BITS + 1)) - 1;

/// `TIM_FSCALE(a,b)`: `(float)((a) * (double)(1<<(b)))`.
#[inline]
pub(crate) fn tim_fscale(a: f64, b: i32) -> f32 {
    (a * (1i32 << b) as f64) as f32
}

/// `TIM_FSCALENEG(a,b)`: `(float)((a) * (1.0L / (double)(1<<(b))))`.
///
/// (The `long double` product is exact, being a scaling by a power of two,
/// so computing it in `double` rounds to the same `float`.)
#[inline]
pub(crate) fn tim_fscaleneg(a: f64, b: i32) -> f32 {
    (a * (1.0 / (1i32 << b) as f64)) as f32
}

/* Vibrato and tremolo Choices of the Day */
pub(crate) const SWEEP_TUNING: i32 = 38;
pub(crate) const VIBRATO_AMPLITUDE_TUNING: f64 = 1.0;
pub(crate) const VIBRATO_RATE_TUNING: i32 = 38;
pub(crate) const TREMOLO_AMPLITUDE_TUNING: f64 = 1.0;
pub(crate) const TREMOLO_RATE_TUNING: i32 = 38;

pub(crate) const SWEEP_SHIFT: i32 = 16;
pub(crate) const RATE_SHIFT: i32 = 5;

pub(crate) const PI: f64 = 3.14159265358979323846;
