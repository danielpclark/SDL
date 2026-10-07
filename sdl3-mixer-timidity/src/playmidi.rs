/*
    TiMidity -- Experimental MIDI to WAVE converter
    Copyright (C) 1995 Tuukka Toivonen <toivonen@clinet.fi>

    This program is free software; you can redistribute it and/or modify
    it under the terms of the Perl Artistic License, available in COPYING.

    playmidi.c -- random stuff in need of rearrangement
*/
// Modified 2026-10-07: translated into Rust from playmidi.c and playmidi.h in SDL_mixer 3.3.0's TiMidity (see NOTICE).

//! Playing the events: voices, channels, and the `Timidity_*` playback
//! functions. Translation of `playmidi.c` (and `playmidi.h`).

use crate::common::c_f64_to_i32;
use crate::instrum::*;
use crate::mix::{apply_envelope_to_amp, mix_voice, recompute_envelope};
use crate::options::*;
use crate::output::{write, PE_16BIT, PE_32BIT, PE_MONO};
use crate::tables::{BEND_COARSE, BEND_FINE, FREQ_TABLE};
use crate::{Instrument, MidiSong, INST_GUS, MAXCHAN, MAX_VOICES, VIBRATO_SAMPLE_INCREMENTS};

/* Midi events */
pub(crate) const ME_NONE: i32 = 0;
pub(crate) const ME_NOTEON: i32 = 1;
pub(crate) const ME_NOTEOFF: i32 = 2;
pub(crate) const ME_KEYPRESSURE: i32 = 3;
pub(crate) const ME_MAINVOLUME: i32 = 4;
pub(crate) const ME_PAN: i32 = 5;
pub(crate) const ME_SUSTAIN: i32 = 6;
pub(crate) const ME_EXPRESSION: i32 = 7;
pub(crate) const ME_PITCHWHEEL: i32 = 8;
pub(crate) const ME_PROGRAM: i32 = 9;
pub(crate) const ME_TEMPO: i32 = 10;
pub(crate) const ME_PITCH_SENS: i32 = 11;

pub(crate) const ME_ALL_SOUNDS_OFF: i32 = 12;
pub(crate) const ME_RESET_CONTROLLERS: i32 = 13;
pub(crate) const ME_ALL_NOTES_OFF: i32 = 14;
pub(crate) const ME_TONE_BANK: i32 = 15;

pub(crate) const ME_LYRIC: i32 = 16;

pub(crate) const ME_EOT: i32 = 99;

/* Causes the instrument's default panning to be used. */
pub(crate) const NO_PANNING: i32 = -1;

/* Voice status options: */
pub(crate) const VOICE_FREE: u8 = 0;
pub(crate) const VOICE_ON: u8 = 1;
pub(crate) const VOICE_SUSTAINED: u8 = 2;
pub(crate) const VOICE_OFF: u8 = 3;
pub(crate) const VOICE_DIE: u8 = 4;

/* Voice panned options: */
pub(crate) const PANNED_MYSTERY: i32 = 0;
pub(crate) const PANNED_LEFT: i32 = 1;
pub(crate) const PANNED_RIGHT: i32 = 2;
pub(crate) const PANNED_CENTER: i32 = 3;
/* Anything but PANNED_MYSTERY only uses the left volume */

/// `ISDRUMCHANNEL(s, c)`.
#[inline]
pub(crate) fn isdrumchannel(song: &MidiSong, c: usize) -> bool {
    (song.drumchannels & (1 << c)) != 0
}

fn adjust_amplification(song: &mut MidiSong) {
    song.master_volume = song.amplification as f32 / 100.0f32;
}

fn reset_voices(song: &mut MidiSong) {
    for i in 0..MAX_VOICES {
        song.voice[i].status = VOICE_FREE;
    }
}

/* Process the Reset All Controllers event */
fn reset_controllers(song: &mut MidiSong, c: usize) {
    song.channel[c].volume = 90; /* Some standard says, although the SCC docs say 0. */
    song.channel[c].expression = 127; /* SCC-1 does this. */
    song.channel[c].sustain = 0;
    song.channel[c].pitchbend = 0x2000;
    song.channel[c].pitchfactor = 0.0; /* to be computed */
}

fn reset_midi(song: &mut MidiSong) {
    for i in 0..MAXCHAN {
        reset_controllers(song, i);
        /* The rest of these are unaffected by the Reset All Controllers event */
        song.channel[i].program = song.default_program;
        song.channel[i].panning = NO_PANNING;
        song.channel[i].pitchsens = 2;
        song.channel[i].bank = 0; /* tone bank or drum set */
    }
    reset_voices(song);
}

fn recompute_freq(song: &mut MidiSong, v: usize) {
    let sign = song.voice[v].sample_increment < 0; /* for bidirectional loops */
    let ch = song.voice[v].channel as usize;
    let mut pb = song.channel[ch].pitchbend;

    let Some(sample) = song.voice[v].sample.clone() else {
        return;
    };
    if sample.sample_rate == 0 {
        return;
    }

    if song.voice[v].vibrato_control_ratio != 0 {
        /* This instrument has vibrato. Invalidate any precomputed
        sample_increments. */

        for i in (0..VIBRATO_SAMPLE_INCREMENTS).rev() {
            song.voice[v].vibrato_sample_increment[i] = 0;
        }
    }

    if pb == 0x2000 || pb < 0 || pb > 0x3FFF {
        song.voice[v].frequency = song.voice[v].orig_frequency;
    } else {
        pb -= 0x2000;
        if song.channel[ch].pitchfactor == 0.0 {
            /* Damn. Somebody bent the pitch. */
            let mut i: i32 = pb * song.channel[ch].pitchsens;
            if pb < 0 {
                i = -i;
            }
            // (pitchsens is at most 127, so i >> 13 is too.)
            song.channel[ch].pitchfactor = (BEND_FINE[((i >> 5) & 0xFF) as usize]
                * BEND_COARSE[(i >> 13) as usize & 0x7F])
                as f32;
        }
        if pb > 0 {
            song.voice[v].frequency = c_f64_to_i32(
                song.channel[ch].pitchfactor as f64 * (song.voice[v].orig_frequency as f64),
            );
        } else {
            song.voice[v].frequency = c_f64_to_i32(
                (song.voice[v].orig_frequency as f64) / song.channel[ch].pitchfactor as f64,
            );
        }
    }

    let mut a = tim_fscale(
        ((sample.sample_rate as f64) * (song.voice[v].frequency as f64))
            / ((sample.root_freq as f64) * (song.rate as f64)),
        FRACTION_BITS,
    ) as f64;

    if sign {
        a = -a; /* need to preserve the loop direction */
    }

    song.voice[v].sample_increment = c_f64_to_i32(a);
}

fn recompute_amp(song: &mut MidiSong, v: usize) {
    let ch = song.voice[v].channel as usize;
    let Some(sample) = song.voice[v].sample.as_ref() else {
        return;
    };
    let volume = sample.volume as f64;
    let vp = &mut song.voice[v];

    /* TODO: use fscale */

    let tempamp: i32 = vp.velocity as i32 * song.channel[ch].volume * song.channel[ch].expression; /* 21 bits */

    let master_volume = song.master_volume as f64;
    if song.encoding & PE_MONO == 0 {
        if vp.panning > 60 && vp.panning < 68 {
            vp.panned = PANNED_CENTER;

            vp.left_amp = tim_fscaleneg((tempamp as f64) * volume * master_volume, 21);
        } else if vp.panning < 5 {
            vp.panned = PANNED_LEFT;

            vp.left_amp = tim_fscaleneg((tempamp as f64) * volume * master_volume, 20);
        } else if vp.panning > 123 {
            vp.panned = PANNED_RIGHT;

            vp.left_amp = /* left_amp will be used */
                tim_fscaleneg((tempamp as f64) * volume * master_volume, 20);
        } else {
            vp.panned = PANNED_MYSTERY;

            vp.left_amp = tim_fscaleneg((tempamp as f64) * volume * master_volume, 27);
            vp.right_amp = vp.left_amp * (vp.panning as f32);
            vp.left_amp *= (127 - vp.panning) as f32;
        }
    } else {
        vp.panned = PANNED_CENTER;

        vp.left_amp = tim_fscaleneg((tempamp as f64) * volume * master_volume, 21);
    }
}

/// The instrument `find_samples()` plays: `NULL` (`None`) when there is none.
fn find_instrument<'a>(song: &'a MidiSong, e: &crate::MidiEvent) -> Option<&'a Instrument> {
    let ch = e.channel as usize;
    let a = (e.a & 0x7f) as usize;
    if isdrumchannel(song, ch) {
        let bank = (song.channel[ch].bank as usize).min(127);
        match song.drumset[bank].as_ref().and_then(|b| b.instrument[a].get()) {
            Some(ip) => {
                if ip.type_ == INST_GUS && ip.samples != 1 {
                    crate::snddbg!("Strange: percussion instrument with {} samples!", ip.samples);
                }
                Some(ip)
            }
            None => song.drumset[0]
                .as_ref()
                .and_then(|b| b.instrument[a].get()), /* No instrument? Then we can't play. */
        }
    } else if song.channel[ch].program == SPECIAL_PROGRAM {
        song.default_instrument.as_deref()
    } else {
        let bank = (song.channel[ch].bank as usize).min(127);
        let program = (song.channel[ch].program as usize) & 0x7f;
        match song.tonebank[bank]
            .as_ref()
            .and_then(|b| b.instrument[program].get())
        {
            Some(ip) => Some(ip),
            None => song.tonebank[0]
                .as_ref()
                .and_then(|b| b.instrument[program].get()), /* No instrument? Then we can't play. */
        }
    }
}

fn find_samples(song: &mut MidiSong, e: &crate::MidiEvent, vlist: &mut [usize; 32]) -> usize {
    let Some(ip) = find_instrument(song, e) else {
        return 0; /* No instrument? Then we can't play. */
    };
    // FIXME (upstream): an instrument without samples (a patch that says
    // it has none) is read past its (empty) array; it can't play.
    if ip.sample.is_empty() || ip.samples <= 0 {
        return 0;
    }
    let ip_type = ip.type_;
    let samples: Vec<_> = ip
        .sample
        .iter()
        .take(ip.samples as usize)
        .cloned()
        .collect();

    let note = if samples[0].note_to_use != 0 {
        samples[0].note_to_use as i32
    } else {
        (e.a & 0x7f) as i32
    };
    // FIXME (upstream): note_to_use is 0 to 127 for patches, but a
    // SoundFont's drum keynote can be more (negative as a Sint8), which
    // reads outside freq_table; use 0.
    let f = FREQ_TABLE.get(note as usize).copied().unwrap_or(0);

    let mut nv = 0;
    let maxnv = if ip_type != INST_GUS { 32 } else { 1 }; /* GUS: emulate timidity-0.2i start_note() */
    for sp in &samples {
        if sp.low_freq <= f && sp.high_freq >= f {
            vlist[nv] = find_voice(song, e);
            song.voice[vlist[nv]].orig_frequency = f;
            song.voice[vlist[nv]].sample = Some(sp.clone());
            nv += 1;
            if nv == maxnv {
                break;
            }
        }
    }

    if nv == 0 {
        let mut cdiff: i32 = 0x7FFFFFFF;
        let mut closest = &samples[0];
        for sp in &samples {
            let mut diff = sp.root_freq.wrapping_sub(f);
            if diff < 0 {
                diff = diff.wrapping_neg();
            }
            if diff < cdiff {
                cdiff = diff;
                closest = sp;
            }
        }
        vlist[nv] = find_voice(song, e);
        song.voice[vlist[nv]].orig_frequency = f;
        song.voice[vlist[nv]].sample = Some(closest.clone());
        nv += 1;
    }

    nv
}

fn start_note(song: &mut MidiSong, e: &crate::MidiEvent, i: usize) {
    let Some(sample) = song.voice[i].sample.clone() else {
        return;
    };
    {
        let vp = &mut song.voice[i];
        vp.status = VOICE_ON;
        vp.channel = e.channel;
        vp.note = e.a;
        vp.velocity = e.b;
        vp.sample_offset = 0;
        vp.sample_increment = 0; /* make sure it isn't negative */

        vp.tremolo_phase = 0;
        vp.tremolo_phase_increment = sample.tremolo_phase_increment;
        vp.tremolo_sweep = sample.tremolo_sweep_increment;
        vp.tremolo_sweep_position = 0;

        vp.vibrato_sweep = sample.vibrato_sweep_increment;
        vp.vibrato_sweep_position = 0;
        vp.vibrato_control_ratio = sample.vibrato_control_ratio;
        vp.vibrato_control_counter = 0;
        vp.vibrato_phase = 0;
        for j in 0..VIBRATO_SAMPLE_INCREMENTS {
            vp.vibrato_sample_increment[j] = 0;
        }
    }

    if song.channel[e.channel as usize].panning != NO_PANNING {
        song.voice[i].panning = song.channel[e.channel as usize].panning;
    } else {
        song.voice[i].panning = sample.panning as i32;
    }

    recompute_freq(song, i);
    recompute_amp(song, i);
    if sample.modes & MODES_ENVELOPE != 0 {
        /* Ramp up from 0 */
        song.voice[i].envelope_stage = 0;
        song.voice[i].envelope_volume = 0;
        song.voice[i].control_counter = 0;
        recompute_envelope(song, i);
        apply_envelope_to_amp(song, i);
    } else {
        song.voice[i].envelope_increment = 0;
        apply_envelope_to_amp(song, i);
    }
}

fn kill_note(song: &mut MidiSong, i: usize) {
    song.voice[i].status = VOICE_DIE;
}

/* Only one instance of a note can be playing on a single channel. */
fn find_voice(song: &mut MidiSong, e: &crate::MidiEvent) -> usize {
    let mut lowest: i32 = -1;
    let mut lv: i32 = 0x7FFFFFFF;

    for i in (0..song.voices as usize).rev() {
        if song.voice[i].status == VOICE_FREE {
            lowest = i as i32; /* Can't get a lower volume than silence */
        } else if song.voice[i].channel == e.channel
            && (song.voice[i].note == e.a
                || song.channel[song.voice[i].channel as usize].mono != 0)
        {
            kill_note(song, i);
        }
    }

    if lowest != -1 {
        /* Found a free voice. */
        return lowest as usize;
    }

    /* Look for the decaying note with the lowest volume */
    for i in (0..song.voices as usize).rev() {
        if (song.voice[i].status != VOICE_ON) && (song.voice[i].status != VOICE_DIE) {
            let mut v = song.voice[i].left_mix;
            if (song.voice[i].panned == PANNED_MYSTERY) && (song.voice[i].right_mix > v) {
                v = song.voice[i].right_mix;
            }
            if v < lv {
                lv = v;
                lowest = i as i32;
            }
        }
    }

    if lowest != -1 {
        /* This can still cause a click, but if we had a free voice to
        spare for ramping down this note, we wouldn't need to kill it
        in the first place... Still, this needs to be fixed. Perhaps
        we could use a reserve of voices to play dying notes only. */

        song.cut_notes += 1;
        song.voice[lowest as usize].status = VOICE_FREE;
        return lowest as usize;
    } else {
        song.lost_notes += 1;
    }
    0
}

fn note_on(song: &mut MidiSong) {
    let e = song.events[song.current_event];
    let mut vlist = [0usize; 32];

    let nv = find_samples(song, &e, &mut vlist);
    for &v in &vlist[..nv] {
        start_note(song, &e, v);
    }
}

fn finish_note(song: &mut MidiSong, i: usize) {
    let modes = song.voice[i].sample.as_ref().map_or(0, |s| s.modes);
    if modes & MODES_ENVELOPE != 0 {
        /* We need to get the envelope out of Sustain stage */
        song.voice[i].envelope_stage = 3;
        song.voice[i].status = VOICE_OFF;
        recompute_envelope(song, i);
        apply_envelope_to_amp(song, i);
    } else {
        /* Set status to OFF so resample_voice() will let this voice out
        of its loop, if any. In any case, this voice dies when it
        hits the end of its data (ofs>=data_length). */
        song.voice[i].status = VOICE_OFF;
    }
}

fn note_off(song: &mut MidiSong) {
    let e = song.events[song.current_event];

    for i in (0..song.voices as usize).rev() {
        if song.voice[i].status == VOICE_ON
            && song.voice[i].channel == e.channel
            && song.voice[i].note == e.a
        {
            if song.channel[e.channel as usize].sustain != 0 {
                song.voice[i].status = VOICE_SUSTAINED;
            } else {
                finish_note(song, i);
            }
        }
    }
}

/* Process the All Notes Off event */
fn all_notes_off(song: &mut MidiSong) {
    let c = song.events[song.current_event].channel;

    crate::snddbg!("All notes off on channel {}", c);
    for i in (0..song.voices as usize).rev() {
        if song.voice[i].status == VOICE_ON && song.voice[i].channel == c {
            if song.channel[c as usize].sustain != 0 {
                song.voice[i].status = VOICE_SUSTAINED;
            } else {
                finish_note(song, i);
            }
        }
    }
}

/* Process the All Sounds Off event */
fn all_sounds_off(song: &mut MidiSong) {
    let c = song.events[song.current_event].channel;

    for i in (0..song.voices as usize).rev() {
        if song.voice[i].channel == c
            && song.voice[i].status != VOICE_FREE
            && song.voice[i].status != VOICE_DIE
        {
            kill_note(song, i);
        }
    }
}

fn adjust_pressure(song: &mut MidiSong) {
    let e = song.events[song.current_event];

    for i in (0..song.voices as usize).rev() {
        if song.voice[i].status == VOICE_ON
            && song.voice[i].channel == e.channel
            && song.voice[i].note == e.a
        {
            song.voice[i].velocity = e.b;
            recompute_amp(song, i);
            apply_envelope_to_amp(song, i);
            return;
        }
    }
}

fn drop_sustain(song: &mut MidiSong) {
    let c = song.events[song.current_event].channel;

    for i in (0..song.voices as usize).rev() {
        if song.voice[i].status == VOICE_SUSTAINED && song.voice[i].channel == c {
            finish_note(song, i);
        }
    }
}

fn adjust_pitchbend(song: &mut MidiSong) {
    let c = song.events[song.current_event].channel;

    for i in (0..song.voices as usize).rev() {
        if song.voice[i].status != VOICE_FREE && song.voice[i].channel == c {
            recompute_freq(song, i);
        }
    }
}

fn adjust_volume(song: &mut MidiSong) {
    let c = song.events[song.current_event].channel;

    for i in (0..song.voices as usize).rev() {
        if song.voice[i].channel == c
            && (song.voice[i].status == VOICE_ON || song.voice[i].status == VOICE_SUSTAINED)
        {
            recompute_amp(song, i);
            apply_envelope_to_amp(song, i);
        }
    }
}

fn seek_forward(song: &mut MidiSong, until_time: i32) {
    reset_voices(song);
    while song.events[song.current_event].time < until_time {
        let e = song.events[song.current_event];
        let ch = e.channel as usize;
        match e.type_ as i32 {
            /* All notes stay off. Just handle the parameter changes. */
            ME_PITCH_SENS => {
                song.channel[ch].pitchsens = e.a as i32;
                song.channel[ch].pitchfactor = 0.0;
            }

            ME_PITCHWHEEL => {
                song.channel[ch].pitchbend = e.a as i32 + e.b as i32 * 128;
                song.channel[ch].pitchfactor = 0.0;
            }

            ME_MAINVOLUME => {
                song.channel[ch].volume = e.a as i32;
            }

            ME_PAN => {
                song.channel[ch].panning = e.a as i32;
            }

            ME_EXPRESSION => {
                song.channel[ch].expression = e.a as i32;
            }

            ME_PROGRAM => {
                if isdrumchannel(song, ch) {
                    /* Change drum set */
                    song.channel[ch].bank = e.a as i32;
                } else {
                    song.channel[ch].program = e.a as i32;
                }
            }

            ME_SUSTAIN => {
                song.channel[ch].sustain = e.a as i32;
            }

            ME_RESET_CONTROLLERS => {
                reset_controllers(song, ch);
            }

            ME_TONE_BANK => {
                song.channel[ch].bank = e.a as i32;
            }

            ME_EOT => {
                song.current_sample = e.time;
                return;
            }

            _ => {}
        }
        song.current_event += 1;
    }
    /*song->current_sample=song->current_event->time;*/
    if song.current_event != 0 {
        song.current_event -= 1;
    }
    song.current_sample = until_time;
}

fn skip_to(song: &mut MidiSong, until_time: i32) {
    if song.current_sample > until_time {
        song.current_sample = 0;
    }

    reset_midi(song);
    song.buffered_count = 0;
    song.buffer_pointer = 0;
    song.current_event = 0;

    if until_time != 0 {
        seek_forward(song, until_time);
    }
}

fn do_compute_data(song: &mut MidiSong, count: i32) {
    let n = if song.encoding & PE_MONO != 0 {
        count
    } else {
        count * 2
    };
    let start = song.buffer_pointer;
    let mut buf = std::mem::take(&mut song.common_buffer);
    let end = (start + n.max(0) as usize).min(buf.len());
    buf[start.min(end)..end].fill(0);
    for i in 0..song.voices as usize {
        if song.voice[i].status != VOICE_FREE {
            mix_voice(song, &mut buf[start.min(end)..end], i, count);
        }
    }
    song.common_buffer = buf;
    song.current_sample = song.current_sample.wrapping_add(count);
}

/* count=0 means flush remaining buffered data to output device, then
flush the device itself */
fn compute_data(song: &mut MidiSong, stream: &mut [u8], mut count: i32) {
    let channels = if song.encoding & PE_MONO != 0 { 1 } else { 2 };

    if count == 0 {
        if song.buffered_count != 0 {
            write(
                song.write,
                stream,
                &song.common_buffer,
                channels * song.buffered_count,
            );
        }
        song.buffer_pointer = 0;
        song.buffered_count = 0;
        return;
    }

    while (count + song.buffered_count) >= song.buffer_size {
        do_compute_data(song, song.buffer_size - song.buffered_count);
        count -= song.buffer_size - song.buffered_count;
        // FIXME (upstream): every buffer goes to the start of `stream`
        // (the caller's sole buffer when it asks for buffer_size samples,
        // as SDL_mixer does).
        write(
            song.write,
            stream,
            &song.common_buffer,
            channels * song.buffer_size,
        );
        song.buffer_pointer = 0;
        song.buffered_count = 0;
    }
    if count > 0 {
        do_compute_data(song, count);
        song.buffered_count += count;
        song.buffer_pointer += if song.encoding & PE_MONO != 0 {
            count as usize
        } else {
            count as usize * 2
        };
    }
}

impl MidiSong {
    /// Start playing from the beginning. Translation of `Timidity_Start()`.
    pub fn start(&mut self) {
        self.playing = 1;
        adjust_amplification(self);
        skip_to(self, 0);
    }

    /// Stop playing. Translation of `Timidity_Stop()`.
    pub fn stop(&mut self) {
        self.playing = 0;
    }

    /// Whether the song is playing (started, and not at its end).
    /// Translation of `Timidity_IsActive()`.
    pub fn is_active(&self) -> bool {
        self.playing != 0
    }

    /// Go to a time, in milliseconds. Translation of `Timidity_Seek()`.
    ///
    /// As upstream's, this doesn't restart a song that played to its end.
    pub fn seek(&mut self, ms: u32) {
        let rate = self.rate;
        skip_to(
            self,
            (ms.wrapping_mul((rate / 100) as u32) / 10) as i32,
        );
    }

    /// The length of the song, in milliseconds. Translation of
    /// `Timidity_GetSongLength()`.
    pub fn song_length(&self) -> u32 {
        let last_event = &self.events[self.groomed_event_count as usize - 1];
        /* We want last_event->time * 1000 / song->rate */
        let mut retvalue = (last_event.time.wrapping_div(self.rate)).wrapping_mul(1000) as u32;
        retvalue = retvalue.wrapping_add(
            ((last_event.time.wrapping_rem(self.rate)).wrapping_mul(1000) / self.rate) as u32,
        );
        retvalue
    }

    /// The time played so far, in milliseconds. Translation of
    /// `Timidity_GetSongTime()`.
    pub fn song_time(&self) -> u32 {
        let mut retvalue = (self.current_sample.wrapping_div(self.rate)).wrapping_mul(1000) as u32;
        retvalue = retvalue.wrapping_add(
            ((self.current_sample.wrapping_rem(self.rate)).wrapping_mul(1000) / self.rate) as u32,
        );
        retvalue
    }

    /// Render the next `stream.len()` bytes of audio (in the format the
    /// song was loaded for), returning how many bytes were rendered: fewer
    /// at the end of the song, and 0 when it isn't playing. Translation of
    /// `Timidity_PlaySome()`.
    ///
    /// As upstream, audio is written to `stream` a buffer (the `samples`
    /// given to [`MidiSong::load`]) at a time, each to the start of
    /// `stream`: ask for exactly a buffer at a time. At the end of the
    /// song, the audio rendered since the last full buffer isn't written.
    pub fn play_some(&mut self, stream: &mut [u8]) -> i32 {
        let len = stream.len().min(i32::MAX as usize) as i32;

        if self.playing == 0 {
            return 0;
        }

        let mut bytes_per_sample = 1;
        bytes_per_sample *= if self.encoding & PE_32BIT != 0 {
            4
        } else if self.encoding & PE_16BIT != 0 {
            2
        } else {
            1
        };
        bytes_per_sample *= if self.encoding & PE_MONO != 0 { 1 } else { 2 };
        let samples = len / bytes_per_sample;

        let start_sample = self.current_sample;
        let end_sample = self.current_sample.wrapping_add(samples);
        while self.current_sample < end_sample {
            /* Handle all events that should happen at this time */
            while self.events[self.current_event].time <= self.current_sample {
                let e = self.events[self.current_event];
                let ch = e.channel as usize;
                match e.type_ as i32 {
                    /* Effects affecting a single note */
                    ME_NOTEON => {
                        if e.b == 0 {
                            /* Velocity 0? */
                            note_off(self);
                        } else {
                            note_on(self);
                        }
                    }

                    ME_NOTEOFF => note_off(self),

                    ME_KEYPRESSURE => adjust_pressure(self),

                    /* Effects affecting a single channel */
                    ME_PITCH_SENS => {
                        self.channel[ch].pitchsens = e.a as i32;
                        self.channel[ch].pitchfactor = 0.0;
                    }

                    ME_PITCHWHEEL => {
                        self.channel[ch].pitchbend = e.a as i32 + e.b as i32 * 128;
                        self.channel[ch].pitchfactor = 0.0;
                        /* Adjust pitch for notes already playing */
                        adjust_pitchbend(self);
                    }

                    ME_MAINVOLUME => {
                        self.channel[ch].volume = e.a as i32;
                        adjust_volume(self);
                    }

                    ME_PAN => {
                        self.channel[ch].panning = e.a as i32;
                    }

                    ME_EXPRESSION => {
                        self.channel[ch].expression = e.a as i32;
                        adjust_volume(self);
                    }

                    ME_PROGRAM => {
                        if isdrumchannel(self, ch) {
                            /* Change drum set */
                            self.channel[ch].bank = e.a as i32;
                        } else {
                            self.channel[ch].program = e.a as i32;
                        }
                    }

                    ME_SUSTAIN => {
                        self.channel[ch].sustain = e.a as i32;
                        if e.a == 0 {
                            drop_sustain(self);
                        }
                    }

                    ME_RESET_CONTROLLERS => reset_controllers(self, ch),

                    ME_ALL_NOTES_OFF => all_notes_off(self),

                    ME_ALL_SOUNDS_OFF => all_sounds_off(self),

                    ME_TONE_BANK => {
                        self.channel[ch].bank = e.a as i32;
                    }

                    ME_EOT => {
                        /* Give the last notes a couple of seconds to decay  */
                        crate::snddbg!(
                            "Playing time: ~{} seconds\n",
                            self.current_sample / self.rate + 2
                        );
                        crate::snddbg!("Notes cut: {}\n", self.cut_notes);
                        crate::snddbg!("Notes lost totally: {}\n", self.lost_notes);
                        self.playing = 0;
                        return self
                            .current_sample
                            .wrapping_sub(start_sample)
                            .wrapping_mul(bytes_per_sample);
                    }

                    _ => {}
                }
                self.current_event += 1;
            }
            let next_time = self.events[self.current_event].time;
            if next_time > end_sample {
                compute_data(self, stream, end_sample - self.current_sample);
            } else {
                compute_data(self, stream, next_time - self.current_sample);
            }
        }
        samples * bytes_per_sample
    }

    /// Set the volume (the amplification, in percent: 0 to 800).
    /// Translation of `Timidity_SetVolume()`.
    pub fn set_volume(&mut self, volume: i32) {
        if volume > MAX_AMPLIFICATION {
            self.amplification = MAX_AMPLIFICATION;
        } else if volume < 0 {
            self.amplification = 0;
        } else {
            self.amplification = volume;
        }
        adjust_amplification(self);
        for i in 0..self.voices as usize {
            if self.voice[i].status != VOICE_FREE {
                recompute_amp(self, i);
                apply_envelope_to_amp(self, i);
            }
        }
    }
}

#[allow(dead_code)]
const _: i32 = ME_NONE + ME_LYRIC + PANNED_LEFT;
