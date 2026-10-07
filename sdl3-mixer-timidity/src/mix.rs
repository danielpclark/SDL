/*
    TiMidity -- Experimental MIDI to WAVE converter
    Copyright (C) 1995 Tuukka Toivonen <toivonen@clinet.fi>

    This program is free software; you can redistribute it and/or modify
    it under the terms of the Perl Artistic License, available in COPYING.

    mix.c */
// Modified 2026-10-07: translated into Rust from mix.c and mix.h in SDL_mixer 3.3.0's TiMidity (see NOTICE).

//! Envelopes, tremolo, and mixing the voices into the 32-bit buffer.
//! Translation of `mix.c` (and `mix.h`).
//!
//! The mixing functions take the voice's (resampled) samples as a slice,
//! reading 0 past its end, where corrupt patches make upstream read out
//! of bounds.

use crate::common::c_f32_to_i32;
use crate::instrum::MODES_ENVELOPE;
use crate::options::*;
use crate::output::PE_MONO;
use crate::playmidi::*;
use crate::resample::{resample_voice, Resampled};
use crate::tables::{timi_sine, VOL_TABLE};
use crate::{FinalVolumeT, MidiSong, SampleT};

/// `vol_table[x]`.
fn vol_table(x: i32) -> f64 {
    // FIXME (upstream): an envelope volume that overflowed (with a corrupt
    // patch's rates) indexes outside vol_table; use 0.
    VOL_TABLE.get(x as usize).copied().unwrap_or(0.0)
}

/// The voice's sample's `modes`.
fn sample_modes(song: &MidiSong, v: usize) -> u8 {
    song.voice[v].sample.as_ref().map_or(0, |s| s.modes)
}

/* Returns 1 if envelope runs out */
/// Translation of `recompute_envelope()`.
pub(crate) fn recompute_envelope(song: &mut MidiSong, v: usize) -> i32 {
    let modes = sample_modes(song, v);
    let (offsets, rates) = match song.voice[v].sample.as_ref() {
        Some(s) => (s.envelope_offset, s.envelope_rate),
        None => ([0; 6], [0; 6]),
    };
    let vp = &mut song.voice[v];
    loop {
        let stage = vp.envelope_stage;

        if stage > 5 || stage < 0 {
            /* Envelope ran out. */
            vp.status = VOICE_FREE;
            return 1;
        }

        if modes & MODES_ENVELOPE != 0 {
            if vp.status == VOICE_ON || vp.status == VOICE_SUSTAINED {
                if stage > 2 {
                    /* Freeze envelope until note turns off. Trumpets want this. */
                    vp.envelope_increment = 0;
                    return 0;
                }
            }
        }
        vp.envelope_stage = stage + 1;

        let stage = stage as usize;
        if vp.envelope_volume == offsets[stage]
            || (stage > 2 && vp.envelope_volume < offsets[stage])
        {
            continue; // return recompute_envelope(song, v);
        }
        vp.envelope_target = offsets[stage];
        vp.envelope_increment = rates[stage];
        if vp.envelope_target < vp.envelope_volume {
            vp.envelope_increment = vp.envelope_increment.wrapping_neg();
        }
        return 0;
    }
}

/// Translation of `apply_envelope_to_amp()`.
pub(crate) fn apply_envelope_to_amp(song: &mut MidiSong, v: usize) {
    let modes = sample_modes(song, v);
    let vp = &mut song.voice[v];
    let mut lamp = vp.left_amp;
    let mut ramp;
    let mut la: i32;
    let mut ra: i32;
    if vp.panned == PANNED_MYSTERY {
        ramp = vp.right_amp;
        if vp.tremolo_phase_increment != 0 {
            lamp *= vp.tremolo_volume;
            ramp *= vp.tremolo_volume;
        }
        if modes & MODES_ENVELOPE != 0 {
            lamp *= vol_table(vp.envelope_volume >> 23) as f32;
            ramp *= vol_table(vp.envelope_volume >> 23) as f32;
        }

        la = c_f32_to_i32(tim_fscale(lamp as f64, AMP_BITS));

        if la > MAX_AMP_VALUE {
            la = MAX_AMP_VALUE;
        }

        ra = c_f32_to_i32(tim_fscale(ramp as f64, AMP_BITS));
        if ra > MAX_AMP_VALUE {
            ra = MAX_AMP_VALUE;
        }

        vp.left_mix = la;
        vp.right_mix = ra;
    } else {
        if vp.tremolo_phase_increment != 0 {
            lamp *= vp.tremolo_volume;
        }
        if modes & MODES_ENVELOPE != 0 {
            lamp *= vol_table(vp.envelope_volume >> 23) as f32;
        }

        la = c_f32_to_i32(tim_fscale(lamp as f64, AMP_BITS));

        if la > MAX_AMP_VALUE {
            la = MAX_AMP_VALUE;
        }

        vp.left_mix = la;
    }
}

fn update_envelope(song: &mut MidiSong, v: usize) -> i32 {
    let vp = &mut song.voice[v];
    vp.envelope_volume = vp.envelope_volume.wrapping_add(vp.envelope_increment);
    /* Why is there no ^^ operator?? */
    if ((vp.envelope_increment < 0) && (vp.envelope_volume <= vp.envelope_target))
        || ((vp.envelope_increment > 0) && (vp.envelope_volume >= vp.envelope_target))
    {
        vp.envelope_volume = vp.envelope_target;
        if recompute_envelope(song, v) != 0 {
            return 1;
        }
    }
    0
}

fn update_tremolo(song: &mut MidiSong, v: usize) {
    let tremolo_depth = song.voice[v].sample.as_ref().map_or(0, |s| s.tremolo_depth);
    let vp = &mut song.voice[v];
    let mut depth: i32 = (tremolo_depth as i32) << 7;

    if vp.tremolo_sweep != 0 {
        /* Update sweep position */

        vp.tremolo_sweep_position = vp.tremolo_sweep_position.wrapping_add(vp.tremolo_sweep);
        if vp.tremolo_sweep_position >= (1 << SWEEP_SHIFT) {
            vp.tremolo_sweep = 0; /* Swept to max amplitude */
        } else {
            /* Need to adjust depth */
            depth = depth.wrapping_mul(vp.tremolo_sweep_position);
            depth >>= SWEEP_SHIFT;
        }
    }

    vp.tremolo_phase = vp.tremolo_phase.wrapping_add(vp.tremolo_phase_increment);

    /* if (song->voice[v].tremolo_phase >= (SINE_CYCLE_LENGTH<<RATE_SHIFT))
    song->voice[v].tremolo_phase -= SINE_CYCLE_LENGTH<<RATE_SHIFT;  */

    vp.tremolo_volume = (1.0
        - tim_fscaleneg(
            (timi_sine((vp.tremolo_phase >> RATE_SHIFT) as f64) + 1.0)
                * depth as f64
                * TREMOLO_AMPLITUDE_TUNING,
            17,
        ) as f64) as f32;

    /* I'm not sure about the +1.0 there -- it makes tremoloed voices'
    volumes on average the lower the higher the tremolo amplitude. */
}

/* Returns 1 if the note died */
fn update_signal(song: &mut MidiSong, v: usize) -> i32 {
    if song.voice[v].envelope_increment != 0 && update_envelope(song, v) != 0 {
        return 1;
    }

    if song.voice[v].tremolo_phase_increment != 0 {
        update_tremolo(song, v);
    }

    apply_envelope_to_amp(song, v);
    0
}

/// The samples the mixing functions read (`sample_t *sp`), reading 0
/// past the end.
struct Src<'a> {
    data: &'a [SampleT],
    pos: usize,
}

impl Src<'_> {
    /// `*sp++`.
    #[inline]
    fn next(&mut self) -> SampleT {
        let s = self.data.get(self.pos).copied().unwrap_or(0);
        self.pos += 1;
        s
    }
}

/// The mix buffer (`Sint32 *lp`).
struct Dst<'a> {
    buf: &'a mut [i32],
    pos: usize,
}

impl Dst<'_> {
    /// `MIXATION(a)`: `*lp++ += (a)*s;`
    #[inline]
    fn mixation(&mut self, a: FinalVolumeT, s: SampleT) {
        if let Some(d) = self.buf.get_mut(self.pos) {
            *d = d.wrapping_add(a.wrapping_mul(s as i32));
        }
        self.pos += 1;
    }
    /// `lp++`.
    #[inline]
    fn skip(&mut self) {
        self.pos += 1;
    }
}

/// How each of the `mix_*_signal()` functions mixes a sample.
#[derive(Clone, Copy, PartialEq)]
enum Shape {
    Mystery,
    Center,
    Single,
    Mono,
}

fn mix_one(shape: Shape, lp: &mut Dst<'_>, left: FinalVolumeT, right: FinalVolumeT, s: SampleT) {
    match shape {
        Shape::Mystery => {
            lp.mixation(left, s);
            lp.mixation(right, s);
        }
        Shape::Center => {
            lp.mixation(left, s);
            lp.mixation(left, s);
        }
        Shape::Single => {
            lp.mixation(left, s);
            lp.skip();
        }
        Shape::Mono => {
            lp.mixation(left, s);
        }
    }
}

/// `mix_mystery_signal()`, `mix_center_signal()`, `mix_single_signal()`
/// and `mix_mono_signal()`, which differ only in how they mix a sample
/// (and in `right`, which only the first uses).
fn mix_signal(
    shape: Shape,
    song: &mut MidiSong,
    sp: &mut Src<'_>,
    lp: &mut Dst<'_>,
    v: usize,
    mut count: i32,
) {
    let mut left: FinalVolumeT = song.voice[v].left_mix;
    let mut right: FinalVolumeT = song.voice[v].right_mix;
    let mut cc: i32;

    cc = song.voice[v].control_counter;
    if cc == 0 {
        cc = song.control_ratio;
        if update_signal(song, v) != 0 {
            return; /* Envelope ran out */
        }
        left = song.voice[v].left_mix;
        right = song.voice[v].right_mix;
    }

    while count != 0 {
        if cc < count {
            count -= cc;
            while cc > 0 {
                cc -= 1;
                let s = sp.next();
                mix_one(shape, lp, left, right, s);
            }
            cc = song.control_ratio;
            if update_signal(song, v) != 0 {
                return; /* Envelope ran out */
            }
            left = song.voice[v].left_mix;
            right = song.voice[v].right_mix;
        } else {
            song.voice[v].control_counter = cc - count;
            while count > 0 {
                count -= 1;
                let s = sp.next();
                mix_one(shape, lp, left, right, s);
            }
            return;
        }
    }
}

fn mix_mystery_signal(song: &mut MidiSong, sp: &mut Src<'_>, lp: &mut Dst<'_>, v: usize, count: i32) {
    mix_signal(Shape::Mystery, song, sp, lp, v, count);
}

fn mix_center_signal(song: &mut MidiSong, sp: &mut Src<'_>, lp: &mut Dst<'_>, v: usize, count: i32) {
    mix_signal(Shape::Center, song, sp, lp, v, count);
}

fn mix_single_signal(song: &mut MidiSong, sp: &mut Src<'_>, lp: &mut Dst<'_>, v: usize, count: i32) {
    mix_signal(Shape::Single, song, sp, lp, v, count);
}

fn mix_mono_signal(song: &mut MidiSong, sp: &mut Src<'_>, lp: &mut Dst<'_>, v: usize, count: i32) {
    mix_signal(Shape::Mono, song, sp, lp, v, count);
}

/// `mix_mystery()`, `mix_center()`, `mix_single()` and `mix_mono()`,
/// which differ in the same way.
fn mix_plain(shape: Shape, song: &MidiSong, sp: &mut Src<'_>, lp: &mut Dst<'_>, v: usize, count: i32) {
    let left: FinalVolumeT = song.voice[v].left_mix;
    let right: FinalVolumeT = song.voice[v].right_mix;

    for _ in 0..count {
        let s = sp.next();
        mix_one(shape, lp, left, right, s);
    }
}

fn mix_mystery(song: &MidiSong, sp: &mut Src<'_>, lp: &mut Dst<'_>, v: usize, count: i32) {
    mix_plain(Shape::Mystery, song, sp, lp, v, count);
}

fn mix_center(song: &MidiSong, sp: &mut Src<'_>, lp: &mut Dst<'_>, v: usize, count: i32) {
    mix_plain(Shape::Center, song, sp, lp, v, count);
}

fn mix_single(song: &MidiSong, sp: &mut Src<'_>, lp: &mut Dst<'_>, v: usize, count: i32) {
    mix_plain(Shape::Single, song, sp, lp, v, count);
}

fn mix_mono(song: &MidiSong, sp: &mut Src<'_>, lp: &mut Dst<'_>, v: usize, count: i32) {
    mix_plain(Shape::Mono, song, sp, lp, v, count);
}

/* Ramp a note out in c samples */
fn ramp_out(song: &MidiSong, sp: &mut Src<'_>, lp: &mut Dst<'_>, v: usize, mut c: i32) {
    /* should be final_volume_t, but Uint8 gives trouble. */
    let mut left: i32;
    let mut right: i32;
    let mut li: i32;
    let ri: i32;

    let mut s: SampleT; /* silly warning about uninitialized s */

    left = song.voice[v].left_mix;
    li = (left.wrapping_div(c)).wrapping_neg();
    if li == 0 {
        li = -1;
    }

    /* printf("Ramping out: left=%d, c=%d, li=%d\n", left, c, li); */

    if song.encoding & PE_MONO == 0 {
        if song.voice[v].panned == PANNED_MYSTERY {
            right = song.voice[v].right_mix;
            ri = (right.wrapping_div(c)).wrapping_neg();
            while c > 0 {
                c -= 1;
                left = left.wrapping_add(li);
                if left < 0 {
                    left = 0;
                }
                right = right.wrapping_add(ri);
                if right < 0 {
                    right = 0;
                }
                s = sp.next();
                lp.mixation(left, s);
                lp.mixation(right, s);
            }
        } else if song.voice[v].panned == PANNED_CENTER {
            while c > 0 {
                c -= 1;
                left = left.wrapping_add(li);
                if left < 0 {
                    return;
                }
                s = sp.next();
                lp.mixation(left, s);
                lp.mixation(left, s);
            }
        } else if song.voice[v].panned == PANNED_LEFT {
            while c > 0 {
                c -= 1;
                left = left.wrapping_add(li);
                if left < 0 {
                    return;
                }
                s = sp.next();
                lp.mixation(left, s);
                lp.skip();
            }
        } else if song.voice[v].panned == PANNED_RIGHT {
            while c > 0 {
                c -= 1;
                left = left.wrapping_add(li);
                if left < 0 {
                    return;
                }
                s = sp.next();
                lp.skip();
                lp.mixation(left, s);
            }
        }
    } else {
        /* Mono output.  */
        while c > 0 {
            c -= 1;
            left = left.wrapping_add(li);
            if left < 0 {
                return;
            }
            s = sp.next();
            lp.mixation(left, s);
        }
    }
}

/**************** interface function ******************/

/// Translation of `mix_voice()`.
pub(crate) fn mix_voice(song: &mut MidiSong, buf: &mut [i32], v: usize, mut c: i32) {
    let mut rbuf = std::mem::take(&mut song.resample_buffer);
    if song.voice[v].status == VOICE_DIE {
        if c >= MAX_DIE_TIME {
            c = MAX_DIE_TIME;
        }
        let res = resample_voice(song, &mut rbuf, v, &mut c);
        if c > 0 {
            let data: &[SampleT] = match &res {
                Resampled::Buffer => &rbuf,
                Resampled::Data(sample, ofs) => sample.data.get(*ofs..).unwrap_or(&[]),
            };
            ramp_out(song, &mut Src { data, pos: 0 }, &mut Dst { buf, pos: 0 }, v, c);
        }
        song.voice[v].status = VOICE_FREE;
    } else {
        let res = resample_voice(song, &mut rbuf, v, &mut c);
        // FIXME (upstream): a voice whose sample ends where it starts (an
        // empty one, say) is left with a count of -1, which upstream mixes
        // as some four billion samples. Mix none.
        if c < 0 {
            c = 0;
        }
        let data: &[SampleT] = match &res {
            Resampled::Buffer => &rbuf,
            Resampled::Data(sample, ofs) => sample.data.get(*ofs..).unwrap_or(&[]),
        };
        let sp = &mut Src { data, pos: 0 };
        let lp = &mut Dst { buf, pos: 0 };
        let signal =
            song.voice[v].envelope_increment != 0 || song.voice[v].tremolo_phase_increment != 0;
        if song.encoding & PE_MONO != 0 {
            /* Mono output. */
            if signal {
                mix_mono_signal(song, sp, lp, v, c);
            } else {
                mix_mono(song, sp, lp, v, c);
            }
        } else if song.voice[v].panned == PANNED_MYSTERY {
            if signal {
                mix_mystery_signal(song, sp, lp, v, c);
            } else {
                mix_mystery(song, sp, lp, v, c);
            }
        } else if song.voice[v].panned == PANNED_CENTER {
            if signal {
                mix_center_signal(song, sp, lp, v, c);
            } else {
                mix_center(song, sp, lp, v, c);
            }
        } else {
            /* It's either full left or full right. In either case,
            every other sample is 0. Just get the offset right: */
            if song.voice[v].panned == PANNED_RIGHT {
                lp.skip();
            }

            if signal {
                mix_single_signal(song, sp, lp, v, c);
            } else {
                mix_single(song, sp, lp, v, c);
            }
        }
    }
    song.resample_buffer = rbuf;
}
