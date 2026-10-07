/*

    TiMidity -- Experimental MIDI to WAVE converter
    Copyright (C) 1995 Tuukka Toivonen <toivonen@clinet.fi>

    This program is free software; you can redistribute it and/or modify
    it under the terms of the Perl Artistic License, available in COPYING.

    resample.c
*/
// Modified 2026-10-07: translated into Rust from resample.c and resample.h in SDL_mixer 3.3.0's TiMidity (see NOTICE).

//! Resampling the voices' samples to the output rate, and pre-resampling
//! fixed-pitch instruments at load time. Translation of `resample.c` (and
//! `resample.h`).
//!
//! Corrupt patches (loop points outside the data, loops that end before
//! they start, sample increments of 0) make upstream read outside the
//! sample data, divide by zero, spin forever or write past its buffer;
//! here reads outside the data are 0, and the loops below stop where
//! upstream would never finish (the `FIXME (upstream)` notes).

use std::sync::Arc;

use crate::common::c_f64_to_i32;
use crate::instrum::*;
use crate::options::*;
use crate::playmidi::*;
use crate::tables::{timi_sine, BEND_COARSE, BEND_FINE, FREQ_TABLE, SINE_CYCLE_LENGTH};
use crate::{MidiSong, Sample, SampleT, Voice, VIBRATO_SAMPLE_INCREMENTS};

/// What `resample_voice()` returns: `song->resample_buffer`, or (for a
/// pre-resampled sample) a position in the sample's data.
pub(crate) enum Resampled {
    Buffer,
    Data(Arc<Sample>, usize),
}

/// `PRECALC_LOOP_COUNT(start, end, incr)`:
/// `(((end) - (start) + (incr) - 1) / (incr))`.
#[inline]
fn precalc_loop_count(start: i32, end: i32, incr: i32) -> i32 {
    if incr == 0 {
        // FIXME (upstream): a sample increment of 0 (a corrupt patch's
        // frequencies) divides by zero. The position never moves: go on
        // as long as asked.
        return i32::MAX;
    }
    end.wrapping_sub(start)
        .wrapping_add(incr)
        .wrapping_sub(1)
        .wrapping_div(incr)
}

/// `src[i]`, 0 outside the data.
#[inline]
fn at(src: &[SampleT], i: i32) -> SampleT {
    src.get(i as usize).copied().unwrap_or(0)
}

/// `v1 + (((v2 - v1) * (ofs & FRACTION_MASK)) >> FRACTION_BITS)`, with
/// `v1 = src[ofs >> FRACTION_BITS]` and `v2` the next sample, as stored in
/// a `sample_t`.
#[inline]
fn interpolate(src: &[SampleT], ofs: i32) -> SampleT {
    let v1 = at(src, ofs >> FRACTION_BITS) as i32;
    let v2 = at(src, (ofs >> FRACTION_BITS) + 1) as i32;
    // (upstream computes the product and shift in unsigned arithmetic,
    // which gives the same low 16 bits.)
    (v1 + (((v2 - v1) * (ofs as u32 & FRACTION_MASK) as i32) >> FRACTION_BITS)) as SampleT
}

/// `*dest++ = x` into the resample buffer.
#[inline]
fn put(dest: &mut [SampleT], d: &mut usize, x: SampleT) {
    if let Some(p) = dest.get_mut(*d) {
        *p = x;
    }
    *d += 1;
}

/// Where upstream's loops below would never finish, or would write far
/// past its buffer, with corrupt patches: count the passes that make no
/// progress, and the samples written.
struct Guard {
    stalls: i32,
    limit: usize,
}

impl Guard {
    /// The number of passes in a row that may make no progress.
    const MAX_STALLS: i32 = 64;

    fn new(dest: &[SampleT]) -> Guard {
        Guard {
            stalls: 0,
            limit: dest.len().saturating_mul(4),
        }
    }

    /// Whether to give up, after a pass that resamples `i` samples
    /// (`d` so far).
    fn give_up(&mut self, i: i32, d: usize) -> bool {
        if i <= 0 {
            self.stalls += 1;
        } else {
            self.stalls = 0;
        }
        self.stalls > Self::MAX_STALLS || d > self.limit
    }
}

/// Zero the rest of the buffer when a loop gives up.
fn give_up(dest: &mut [SampleT], d: usize, count: i32) {
    let end = d.saturating_add(count.max(0) as usize).min(dest.len());
    if d < end {
        dest[d..end].fill(0);
    }
}

/*************** resampling with fixed increment *****************/

fn rs_plain(dest: &mut [SampleT], vp: &mut Voice, countptr: &mut i32) {
    /* Play sample until end, then free the voice. */

    let Some(sample) = vp.sample.clone() else {
        return;
    };
    let src = &sample.data[..];
    let mut d = 0usize;
    let mut ofs = vp.sample_offset;
    let mut incr = vp.sample_increment;
    let le = sample.data_length;
    let mut count = *countptr;

    if incr < 0 {
        incr = incr.wrapping_neg(); /* In case we're coming out of a bidir loop */
    }

    /* Precalc how many times we should go through the loop.
    NOTE: Assumes that incr > 0 and that ofs <= le */
    let mut i = precalc_loop_count(ofs, le, incr);

    if i > count {
        i = count;
        count = 0;
    } else {
        count = count.wrapping_sub(i);
    }

    for _ in 0..i {
        put(dest, &mut d, interpolate(src, ofs));
        ofs = ofs.wrapping_add(incr);
    }

    if ofs >= le {
        if ofs == le {
            put(dest, &mut d, at(src, (ofs >> FRACTION_BITS) - 1) / 2);
        }
        vp.status = VOICE_FREE;
        *countptr = countptr.wrapping_sub(count.wrapping_add(1));
    }

    vp.sample_offset = ofs; /* Update offset */
}

fn rs_loop(dest: &mut [SampleT], vp: &mut Voice, mut count: i32) {
    /* Play sample until end-of-loop, skip back and continue. */

    let Some(sample) = vp.sample.clone() else {
        return;
    };
    let src = &sample.data[..];
    let mut d = 0usize;
    let mut ofs = vp.sample_offset;
    let incr = vp.sample_increment;
    let le = sample.loop_end;
    let ll = le.wrapping_sub(sample.loop_start);
    let mut guard = Guard::new(dest);

    while count != 0 {
        if ofs >= le && ll <= 0 {
            // FIXME (upstream): a loop that ends before it starts never
            // gets back into it upstream.
            give_up(dest, d, count);
            break;
        }
        while ofs >= le {
            ofs = ofs.wrapping_sub(ll);
        }
        /* Precalc how many times we should go through the loop */
        let mut i = precalc_loop_count(ofs, le, incr);
        // FIXME (upstream): with a negative increment, this can make no
        // progress (or grow the count past the buffer) for ever.
        if guard.give_up(i, d) {
            give_up(dest, d, count);
            break;
        }
        if i > count {
            i = count;
            count = 0;
        } else {
            count = count.wrapping_sub(i);
        }
        for _ in 0..i {
            put(dest, &mut d, interpolate(src, ofs));
            ofs = ofs.wrapping_add(incr);
        }
    }

    vp.sample_offset = ofs; /* Update offset */
}

fn rs_bidir(dest: &mut [SampleT], vp: &mut Voice, mut count: i32) {
    let Some(sample) = vp.sample.clone() else {
        return;
    };
    let src = &sample.data[..];
    let mut d = 0usize;
    let mut ofs = vp.sample_offset;
    let mut incr = vp.sample_increment;
    let le = sample.loop_end;
    let ls = sample.loop_start;
    let le2 = le.wrapping_shl(1);
    let ls2 = ls.wrapping_shl(1);
    let mut guard = Guard::new(dest);
    /* Play normally until inside the loop region */

    if incr > 0 && ofs < ls {
        /* NOTE: Assumes that incr > 0, which is NOT always the case
        when doing bidirectional looping.  I have yet to see a case
        where both ofs <= ls AND incr < 0, however. */
        let mut i = precalc_loop_count(ofs, ls, incr);
        if i > count {
            i = count;
            count = 0;
        } else {
            count = count.wrapping_sub(i);
        }
        for _ in 0..i {
            put(dest, &mut d, interpolate(src, ofs));
            ofs = ofs.wrapping_add(incr);
        }
    }

    /* Then do the bidirectional looping */
    while count != 0 {
        /* Precalc how many times we should go through the loop */
        let mut i = precalc_loop_count(ofs, if incr > 0 { le } else { ls }, incr);
        // FIXME (upstream): an increment longer than the loop can
        // overshoot it by more than its length, which grows the count, and
        // upstream writes past its buffer (here the extra samples are
        // dropped); and a loop that ends before it starts can make no
        // progress for ever.
        if guard.give_up(i, d) {
            give_up(dest, d, count);
            break;
        }
        if i > count {
            i = count;
            count = 0;
        } else {
            count = count.wrapping_sub(i);
        }
        for _ in 0..i {
            put(dest, &mut d, interpolate(src, ofs));
            ofs = ofs.wrapping_add(incr);
        }
        if ofs >= le {
            /* fold the overshoot back in */
            ofs = le2.wrapping_sub(ofs);
            incr = incr.wrapping_neg();
        } else if ofs <= ls {
            ofs = ls2.wrapping_sub(ofs);
            incr = incr.wrapping_neg();
        }
    }

    vp.sample_increment = incr;
    vp.sample_offset = ofs; /* Update offset */
}

/*********************** vibrato versions ***************************/

/* We only need to compute one half of the vibrato sine cycle */
fn vib_phase_to_inc_ptr(phase: i32) -> i32 {
    const VSI: i32 = VIBRATO_SAMPLE_INCREMENTS as i32;
    if phase < VSI / 2 {
        VSI / 2 - 1 - phase
    } else if phase >= 3 * VSI / 2 {
        5 * VSI / 2 - 1 - phase
    } else {
        phase - VSI / 2
    }
}

fn update_vibrato(rate: i32, vp: &mut Voice, sign: bool) -> i32 {
    let Some(sample) = vp.sample.clone() else {
        return 0;
    };

    let old = vp.vibrato_phase;
    vp.vibrato_phase += 1;
    if old >= 2 * VIBRATO_SAMPLE_INCREMENTS as i32 - 1 {
        vp.vibrato_phase = 0;
    }
    let phase =
        (vib_phase_to_inc_ptr(vp.vibrato_phase) as usize) & (VIBRATO_SAMPLE_INCREMENTS - 1);

    if vp.vibrato_sample_increment[phase] != 0 {
        if sign {
            return vp.vibrato_sample_increment[phase].wrapping_neg();
        } else {
            return vp.vibrato_sample_increment[phase];
        }
    }

    /* Need to compute this sample increment. */
    let mut depth: i32 = (sample.vibrato_depth as i32) << 7;

    if vp.vibrato_sweep != 0 {
        /* Need to update sweep */
        vp.vibrato_sweep_position = vp.vibrato_sweep_position.wrapping_add(vp.vibrato_sweep);
        if vp.vibrato_sweep_position >= (1 << SWEEP_SHIFT) {
            vp.vibrato_sweep = 0;
        } else {
            /* Adjust depth */
            depth = depth.wrapping_mul(vp.vibrato_sweep_position);
            depth >>= SWEEP_SHIFT;
        }
    }

    let mut a = tim_fscale(
        ((sample.sample_rate as f64) * (vp.frequency as f64))
            / ((sample.root_freq as f64) * (rate as f64)),
        FRACTION_BITS,
    ) as f64;

    let mut pb = c_f64_to_i32(
        timi_sine(
            (vp.vibrato_phase * (SINE_CYCLE_LENGTH / (2 * VIBRATO_SAMPLE_INCREMENTS as i32)))
                as f64,
        ) * (depth as f64)
            * VIBRATO_AMPLITUDE_TUNING,
    );

    // (|pb| is at most 255 << 7, unless a sweep position went negative.)
    if pb < 0 {
        pb = pb.wrapping_neg();
        a /= BEND_FINE[((pb >> 5) & 0xFF) as usize] * bend_coarse(pb >> 13);
    } else {
        a *= BEND_FINE[((pb >> 5) & 0xFF) as usize] * bend_coarse(pb >> 13);
    }

    /* If the sweep's over, we can store the newly computed sample_increment */
    if vp.vibrato_sweep == 0 {
        vp.vibrato_sample_increment[phase] = c_f64_to_i32(a);
    }

    if sign {
        a = -a; /* need to preserve the loop direction */
    }

    c_f64_to_i32(a)
}

/// `bend_coarse[i]`.
fn bend_coarse(i: i32) -> f64 {
    // FIXME (upstream): a corrupt patch's sweep can make the depth huge,
    // and the index outside bend_coarse; use 1.
    BEND_COARSE.get(i as usize).copied().unwrap_or(1.0)
}

fn rs_vib_plain(rate: i32, dest: &mut [SampleT], vp: &mut Voice, countptr: &mut i32) {
    /* Play sample until end, then free the voice. */

    let Some(sample) = vp.sample.clone() else {
        return;
    };
    let src = &sample.data[..];
    let mut d = 0usize;
    let le = sample.data_length;
    let mut ofs = vp.sample_offset;
    let mut incr = vp.sample_increment;
    let mut count = *countptr;
    let mut cc = vp.vibrato_control_counter;

    /* This has never been tested */

    if incr < 0 {
        incr = incr.wrapping_neg(); /* In case we're coming out of a bidir loop */
    }

    while count > 0 {
        count -= 1;
        let was = cc;
        cc = cc.wrapping_sub(1);
        if was == 0 {
            cc = vp.vibrato_control_ratio;
            incr = update_vibrato(rate, vp, false);
        }
        put(dest, &mut d, interpolate(src, ofs));
        ofs = ofs.wrapping_add(incr);
        if ofs >= le {
            if ofs == le {
                put(dest, &mut d, at(src, (ofs >> FRACTION_BITS) - 1) / 2);
            }
            vp.status = VOICE_FREE;
            *countptr = countptr.wrapping_sub(count + 1);
            break;
        }
    }

    vp.vibrato_control_counter = cc;
    vp.sample_increment = incr;
    vp.sample_offset = ofs; /* Update offset */
}

fn rs_vib_loop(rate: i32, dest: &mut [SampleT], vp: &mut Voice, mut count: i32) {
    /* Play sample until end-of-loop, skip back and continue. */

    let Some(sample) = vp.sample.clone() else {
        return;
    };
    let src = &sample.data[..];
    let mut d = 0usize;
    let mut ofs = vp.sample_offset;
    let mut incr = vp.sample_increment;
    let le = sample.loop_end;
    let ll = le.wrapping_sub(sample.loop_start);
    let mut cc = vp.vibrato_control_counter;
    let mut vibflag = false;
    let mut guard = Guard::new(dest);

    while count != 0 {
        if ofs >= le && ll <= 0 {
            // FIXME (upstream): see rs_loop().
            give_up(dest, d, count);
            break;
        }
        /* Hopefully the loop is longer than an increment */
        while ofs >= le {
            ofs = ofs.wrapping_sub(ll);
        }
        /* Precalc how many times to go through the loop, taking
        the vibrato control ratio into account this time. */
        let mut i = precalc_loop_count(ofs, le, incr);
        if i > count {
            i = count;
        }
        if i > cc {
            i = cc;
            vibflag = true;
        } else {
            cc = cc.wrapping_sub(i);
        }
        // FIXME (upstream): see rs_loop().
        if guard.give_up(i, d) {
            give_up(dest, d, count);
            break;
        }
        count = count.wrapping_sub(i);
        for _ in 0..i {
            put(dest, &mut d, interpolate(src, ofs));
            ofs = ofs.wrapping_add(incr);
        }
        if vibflag {
            cc = vp.vibrato_control_ratio;
            incr = update_vibrato(rate, vp, false);
            vibflag = false;
        }
    }

    vp.vibrato_control_counter = cc;
    vp.sample_increment = incr;
    vp.sample_offset = ofs; /* Update offset */
}

fn rs_vib_bidir(rate: i32, dest: &mut [SampleT], vp: &mut Voice, mut count: i32) {
    let Some(sample) = vp.sample.clone() else {
        return;
    };
    let src = &sample.data[..];
    let mut d = 0usize;
    let mut ofs = vp.sample_offset;
    let mut incr = vp.sample_increment;
    let le = sample.loop_end;
    let ls = sample.loop_start;
    let mut cc = vp.vibrato_control_counter;
    let le2 = le.wrapping_shl(1);
    let ls2 = ls.wrapping_shl(1);
    let mut vibflag = false;
    let mut guard = Guard::new(dest);

    /* Play normally until inside the loop region */
    while count != 0 && incr > 0 && ofs < ls {
        let mut i = precalc_loop_count(ofs, ls, incr);
        if i > count {
            i = count;
        }
        if i > cc {
            i = cc;
            vibflag = true;
        } else {
            cc = cc.wrapping_sub(i);
        }
        // FIXME (upstream): see rs_bidir().
        if guard.give_up(i, d) {
            give_up(dest, d, count);
            count = 0;
            break;
        }
        count = count.wrapping_sub(i);
        for _ in 0..i {
            put(dest, &mut d, interpolate(src, ofs));
            ofs = ofs.wrapping_add(incr);
        }
        if vibflag {
            cc = vp.vibrato_control_ratio;
            incr = update_vibrato(rate, vp, false);
            vibflag = false;
        }
    }

    /* Then do the bidirectional looping */
    while count != 0 {
        /* Precalc how many times we should go through the loop */
        let mut i = precalc_loop_count(ofs, if incr > 0 { le } else { ls }, incr);
        if i > count {
            i = count;
        }
        if i > cc {
            i = cc;
            vibflag = true;
        } else {
            cc = cc.wrapping_sub(i);
        }
        // FIXME (upstream): see rs_bidir().
        if guard.give_up(i, d) {
            give_up(dest, d, count);
            break;
        }
        count = count.wrapping_sub(i);
        for _ in 0..i.max(0) {
            put(dest, &mut d, interpolate(src, ofs));
            ofs = ofs.wrapping_add(incr);
        }
        if vibflag {
            cc = vp.vibrato_control_ratio;
            incr = update_vibrato(rate, vp, incr < 0);
            vibflag = false;
        }
        if ofs >= le {
            /* fold the overshoot back in */
            ofs = le2.wrapping_sub(ofs);
            incr = incr.wrapping_neg();
        } else if ofs <= ls {
            ofs = ls2.wrapping_sub(ofs);
            incr = incr.wrapping_neg();
        }
    }

    vp.vibrato_control_counter = cc;
    vp.sample_increment = incr;
    vp.sample_offset = ofs; /* Update offset */
}

/// Translation of `resample_voice()`.
pub(crate) fn resample_voice(
    song: &mut MidiSong,
    dest: &mut [SampleT],
    v: usize,
    countptr: &mut i32,
) -> Resampled {
    let rate = song.rate;
    let vp = &mut song.voice[v];
    let Some(sample) = vp.sample.clone() else {
        *countptr = 0;
        return Resampled::Buffer;
    };

    if sample.sample_rate == 0 {
        /* Pre-resampled data -- just update the offset and check if
        we're out of data. */
        let ofs = vp.sample_offset >> FRACTION_BITS; /* Kind of silly to use
                                                     FRACTION_BITS here... */
        let left = (sample.data_length >> FRACTION_BITS).wrapping_sub(ofs);
        if *countptr >= left {
            /* Note finished. Free the voice. */
            vp.status = VOICE_FREE;

            /* Let the caller know how much data we had left */
            *countptr = left;
        } else {
            vp.sample_offset = vp
                .sample_offset
                .wrapping_add(countptr.wrapping_shl(FRACTION_BITS as u32));
        }

        // (a negative offset, from corrupt data, reads before the data
        // upstream; here it reads nothing.)
        let ofs = if ofs < 0 { usize::MAX } else { ofs as usize };
        return Resampled::Data(sample, ofs);
    }

    /* Need to resample. Use the proper function. */
    let modes = sample.modes;

    if vp.vibrato_control_ratio != 0 {
        if (modes & MODES_LOOPING != 0)
            && ((modes & MODES_ENVELOPE != 0)
                || (vp.status == VOICE_ON || vp.status == VOICE_SUSTAINED))
        {
            if modes & MODES_PINGPONG != 0 {
                rs_vib_bidir(rate, dest, vp, *countptr);
            } else {
                rs_vib_loop(rate, dest, vp, *countptr);
            }
        } else {
            rs_vib_plain(rate, dest, vp, countptr);
        }
    } else {
        if (modes & MODES_LOOPING != 0)
            && ((modes & MODES_ENVELOPE != 0)
                || (vp.status == VOICE_ON || vp.status == VOICE_SUSTAINED))
        {
            if modes & MODES_PINGPONG != 0 {
                rs_bidir(dest, vp, *countptr);
            } else {
                rs_loop(dest, vp, *countptr);
            }
        } else {
            rs_plain(dest, vp, countptr);
        }
    }
    Resampled::Buffer
}

/// Translation of `pre_resample()`.
pub(crate) fn pre_resample(song: &mut MidiSong, sp: &mut Sample) {
    let src = &sp.data[..];

    crate::snddbg!(" * pre-resampling for note {}\n", sp.note_to_use);

    // FIXME (upstream): a SoundFont's drum keynote over 127 is negative as
    // a Sint8, and indexes before freq_table; use its low 7 bits.
    let a = ((sp.root_freq as f64) * song.rate as f64)
        / ((sp.sample_rate as f64) * FREQ_TABLE[(sp.note_to_use as i32 & 0x7F) as usize] as f64);
    if sp.data_length as f64 * a >= 0x7fffffff as f64 {
        /* Too large to compute */
        crate::snddbg!(" *** Can't pre-resampling for note {}\n", sp.note_to_use);
        return;
    }

    let newlen = c_f64_to_i32(sp.data_length as f64 * a);
    let mut count = (newlen >> FRACTION_BITS) - 1;
    if count == 0 {
        // FIXME (upstream): a sample that resamples to one or two samples
        // divides by zero here. Leave it as it is.
        return;
    }
    let incr = (sp.data_length.wrapping_sub(1 << FRACTION_BITS)).wrapping_div(count);
    let mut ofs = incr;

    if newlen as f64 + incr as f64 >= 0x7fffffff as f64 {
        /* Too large to compute */
        crate::snddbg!(" *** Can't pre-resampling for note {}\n", sp.note_to_use);
        return;
    }

    // SDL_malloc((newlen >> (FRACTION_BITS - 1)) + 2): a negative length
    // fails, as does an allocation too large. FIXME (upstream): a sample
    // that resamples to less than two samples gets a buffer too small for
    // the four written below; the ones past it are dropped here.
    let bytes = (newlen >> (FRACTION_BITS - 1)).wrapping_add(2);
    let mut newdata: Vec<SampleT> = Vec::new();
    if bytes < 0 || newdata.try_reserve_exact(bytes as usize / 2).is_err() {
        song.oom = 1;
        return;
    }
    newdata.resize(bytes as usize / 2, 0);
    let mut d = 0usize;

    count -= 1;
    if count != 0 {
        put(&mut newdata, &mut d, at(src, 0));
    }

    /* Since we're pre-processing and this doesn't have to be done in
    real-time, we go ahead and do the full sliding cubic interpolation. */
    count -= 1;
    for _ in 0..count.max(0) {
        let vptr = ofs >> FRACTION_BITS;
        let v1: i32 = if vptr >= 1 { at(src, vptr - 1) as i32 } else { 0 };
        let v2: i32 = at(src, vptr) as i32;
        let v3: i32 = at(src, vptr + 1) as i32;
        let v4: i32 = at(src, vptr + 2) as i32;
        let v5: i32 = v2 - v3;
        let xdiff = tim_fscaleneg((ofs as u32 & FRACTION_MASK) as f64, FRACTION_BITS) as f64;
        let v = c_f64_to_i32(
            v2 as f64
                + xdiff
                    * (1.0 / 6.0)
                    * ((3 * (v3 - v5) - 2 * v1 - v4) as f64
                        + xdiff
                            * ((3 * (v1 - v2 - v5)) as f64 + xdiff * ((3 * v5 + v4 - v1) as f64))),
        );
        put(
            &mut newdata,
            &mut d,
            (if v > 32767 {
                32767
            } else if v < -32768 {
                -32768
            } else {
                v
            }) as SampleT,
        );
        ofs = ofs.wrapping_add(incr);
    }

    if ofs as u32 & FRACTION_MASK != 0 {
        let v1: i32 = at(src, ofs >> FRACTION_BITS) as i32;
        let v2: i32 = at(src, (ofs >> FRACTION_BITS) + 1) as i32;
        put(
            &mut newdata,
            &mut d,
            (v1 + (((v2 - v1) * (ofs as u32 & FRACTION_MASK) as i32) >> FRACTION_BITS))
                as SampleT,
        );
    } else {
        put(&mut newdata, &mut d, at(src, ofs >> FRACTION_BITS));
    }

    let last = if d >= 1 { newdata.get(d - 1).copied().unwrap_or(0) } else { 0 };
    put(&mut newdata, &mut d, last / 2);
    let last = newdata.get(d - 1).copied().unwrap_or(0);
    put(&mut newdata, &mut d, last / 2);

    sp.data_length = newlen;
    sp.loop_start = c_f64_to_i32(sp.loop_start as f64 * a);
    sp.loop_end = c_f64_to_i32(sp.loop_end as f64 * a);
    sp.data = newdata;
    sp.sample_rate = 0;
}
