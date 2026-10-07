/*

    TiMidity -- Experimental MIDI to WAVE converter
    Copyright (C) 1995 Tuukka Toivonen <toivonen@clinet.fi>

    This program is free software; you can redistribute it and/or modify
    it under the terms of the Perl Artistic License, available in COPYING.
*/
// Modified 2026-10-07: translated into Rust from timidity.h in SDL_mixer 3.3.0's TiMidity (see NOTICE).

//! # sdl3-mixer-timidity — SDL_mixer's TiMidity, translated to Rust
//!
//! A line-by-line translation of the copy of TiMidity 0.2i ("an
//! experimental MIDI to WAVE converter", by Tuukka Toivonen, as stripped
//! for SDL_sound and SDL_mixer) bundled with SDL_mixer 3.3.0, which
//! [`sdl3-mixer`](https://crates.io/crates/sdl3-mixer) uses to play MIDI
//! files. It renders Standard MIDI Files (and RIFF RMID files) to PCM
//! with Gravis Ultrasound patches (`.pat` files) or a SoundFont 1 or 2
//! (`.sbk`/`.sf2`), which are read from the file system as a
//! `timidity.cfg` configuration file says. None are bundled.
//!
//! Unlike the rest of this repository, which is under the zlib license,
//! TiMidity is under the Perl Artistic License or the GNU LGPL version
//! 2.1, at your choice (see `COPYING`, `COPYING.artistic`, `COPYING.LGPL`
//! and `NOTICE` in this crate). `sdl3-mixer` uses it through its
//! `timidity` Cargo feature, which is on by default; build `sdl3-mixer`
//! with `default-features = false` for a purely zlib-licensed build
//! without MIDI.
//!
//! The API is upstream's `Timidity_*` functions: [`init`] reads the
//! configuration (global, as upstream's is), [`MidiSong::load`] reads a
//! MIDI file and its instruments, and [`MidiSong::play_some`] renders it.
//! Every item names the C symbol it translates. Where corrupt input makes
//! upstream's C read or write out of bounds, divide by zero or spin
//! forever, the translation does something defined instead; those places
//! are marked `FIXME (upstream)`.

#![forbid(unsafe_code)]
#![warn(missing_debug_implementations)]
// The translation keeps upstream's code shape (its branches, constants,
// counters and arithmetic, including its quirks), which these lints would
// have rewritten.
#![allow(
    clippy::approx_constant,
    clippy::collapsible_else_if,
    clippy::collapsible_if,
    clippy::comparison_chain,
    clippy::excessive_precision,
    clippy::field_reassign_with_default,
    clippy::identity_op,
    clippy::if_same_then_else,
    clippy::manual_range_contains,
    clippy::needless_range_loop,
    clippy::too_many_arguments,
    clippy::unnecessary_cast
)]

mod common;
mod instrum;
mod mix;
mod options;
mod output;
mod playmidi;
mod readmidi;
mod readsbk;
mod resample;
mod sndfont;
mod tables;
mod timidity;

use std::fmt;
use std::sync::Arc;

pub use timidity::{exit, init, init_no_config, set_soundfont};

pub(crate) type SampleT = i16;
pub(crate) type FinalVolumeT = i32;

pub(crate) const VIBRATO_SAMPLE_INCREMENTS: usize = 32;

/* Maximum polyphony. */
/* #define MAX_VOICES	48 */
pub(crate) const MAX_VOICES: usize = 256;
pub(crate) const MAXCHAN: usize = 16;
/* #define MAXCHAN	64 */
pub(crate) const MAXBANK: usize = 128;

/// Translation of `Sample`.
#[derive(Clone, Default)]
pub(crate) struct Sample {
    pub loop_start: i32,
    pub loop_end: i32,
    pub data_length: i32,
    pub sample_rate: i32,
    pub low_freq: i32,
    pub high_freq: i32,
    pub root_freq: i32,
    #[allow(dead_code)]
    pub root_tune: i8,
    #[allow(dead_code)]
    pub fine_tune: i8, /* for soundfont support */
    pub envelope_rate: [i32; 6],
    pub envelope_offset: [i32; 6],
    pub volume: f32,
    /// The sample data, with the extra samples upstream allocates past the
    /// end (zeroed).
    pub data: Vec<SampleT>,
    pub tremolo_sweep_increment: i32,
    pub tremolo_phase_increment: i32,
    pub vibrato_sweep_increment: i32,
    pub vibrato_control_ratio: i32,
    pub tremolo_depth: u8,
    pub vibrato_depth: u8,
    pub modes: u8,
    pub panning: i8,
    pub note_to_use: i8,
    pub scale_tuning: i16, /* for soundfont support */
}

/// Translation of `Channel`.
#[derive(Clone, Copy, Default)]
pub(crate) struct Channel {
    pub bank: i32,
    pub program: i32,
    pub volume: i32,
    pub sustain: i32,
    pub panning: i32,
    pub pitchbend: i32,
    pub expression: i32,
    pub mono: i32, /* one note only on this channel -- not implemented yet */
    pub pitchsens: i32,
    /* chorus, reverb... Coming soon to a 300-MHz, eight-way superscalar
    processor near you */
    pub pitchfactor: f32, /* precomputed pitch bend factor to save some fdiv's */
}

/// Translation of `Voice`.
#[derive(Clone, Default)]
pub(crate) struct Voice {
    pub status: u8,
    pub channel: u8,
    pub note: u8,
    pub velocity: u8,
    pub sample: Option<Arc<Sample>>,
    pub orig_frequency: i32,
    pub frequency: i32,
    pub sample_offset: i32,
    pub sample_increment: i32,
    pub envelope_volume: i32,
    pub envelope_target: i32,
    pub envelope_increment: i32,
    pub tremolo_sweep: i32,
    pub tremolo_sweep_position: i32,
    pub tremolo_phase: i32,
    pub tremolo_phase_increment: i32,
    pub vibrato_sweep: i32,
    pub vibrato_sweep_position: i32,

    pub left_mix: FinalVolumeT,
    pub right_mix: FinalVolumeT,

    pub left_amp: f32,
    pub right_amp: f32,
    pub tremolo_volume: f32,
    pub vibrato_sample_increment: [i32; VIBRATO_SAMPLE_INCREMENTS],
    pub vibrato_phase: i32,
    pub vibrato_control_ratio: i32,
    pub vibrato_control_counter: i32,
    pub envelope_stage: i32,
    pub control_counter: i32,
    pub panning: i32,
    pub panned: i32,
}

pub(crate) const INST_GUS: i32 = 0;
pub(crate) const INST_SF2: i32 = 1;

/// Translation of `Instrument`.
pub(crate) struct Instrument {
    pub type_: i32,
    pub samples: i32,
    pub sample: Vec<Arc<Sample>>,
}

/* Shared data */
/// Translation of `ToneBankElement`.
#[derive(Clone, Default)]
pub(crate) struct ToneBankElement {
    pub name: Option<Vec<u8>>,
    pub note: i32,
    pub amp: i32,
    pub pan: i32,
    pub strip_loop: i32,
    pub strip_envelope: i32,
    pub strip_tail: i32,
}

/// An entry of `ToneBank`'s `instrument` array: `NULL`,
/// `MAGIC_LOAD_INSTRUMENT` or an instrument.
#[derive(Default)]
pub(crate) enum InstSlot {
    #[default]
    None,
    /* A hack to delay instrument loading until after reading the
    entire MIDI file. */
    MagicLoad,
    Loaded(Box<Instrument>),
}

impl InstSlot {
    /// `bank->instrument[i]` (when not `MAGIC_LOAD_INSTRUMENT`).
    pub fn get(&self) -> Option<&Instrument> {
        match self {
            InstSlot::Loaded(ip) => Some(ip),
            _ => None,
        }
    }
    /// `bank->instrument[i] != NULL`.
    pub fn is_some(&self) -> bool {
        !matches!(self, InstSlot::None)
    }
}

/// Where a song's `ToneBank` gets its `tone` array: upstream shares the
/// configuration's (`master_tonebank[i]->tone`), or allocates one (the
/// SoundFont loader, for banks the configuration doesn't have).
pub(crate) enum ToneSrc {
    Master,
    Own(Vec<ToneBankElement>),
}

/// Translation of `ToneBank`.
pub(crate) struct ToneBank {
    pub tone: ToneSrc,
    pub instrument: [InstSlot; 128],
}

impl ToneBank {
    pub fn new(tone: ToneSrc) -> Box<ToneBank> {
        Box::new(ToneBank {
            tone,
            instrument: std::array::from_fn(|_| InstSlot::None),
        })
    }
}

/// Translation of `MidiEvent`.
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct MidiEvent {
    pub time: i32,
    pub channel: u8,
    pub type_: u8,
    pub a: u8,
    pub b: u8,
}

/// Translation of `struct _MidiEventList` (the `next` pointer is an index
/// into [`MidiSong::evlist`]).
#[derive(Clone, Copy)]
pub(crate) struct MidiEventList {
    pub event: MidiEvent,
    pub next: Option<u32>,
}

/// How `Timidity_PlaySome()` writes the 32-bit mix (`song->write`).
#[derive(Clone, Copy)]
pub(crate) enum WriteFn {
    S8,
    U8,
    S16L,
    S16B,
    S32L,
    S32B,
    F32L,
    F32B,
}

/// A loaded MIDI file, ready to play. Translation of `MidiSong`.
///
/// Made by [`MidiSong::load`]; its `Drop` is `Timidity_FreeSong()`.
pub struct MidiSong {
    pub(crate) oom: i32, /* malloc() failed */
    pub(crate) playing: i32,
    pub(crate) rate: i32,
    pub(crate) encoding: i32,
    pub(crate) master_volume: f32,
    pub(crate) amplification: i32,
    pub(crate) tonebank: [Option<Box<ToneBank>>; MAXBANK],
    pub(crate) drumset: [Option<Box<ToneBank>>; MAXBANK],
    pub(crate) default_instrument: Option<Box<Instrument>>,
    pub(crate) default_program: i32,
    pub(crate) write: WriteFn,
    pub(crate) buffer_size: i32,
    pub(crate) resample_buffer: Vec<SampleT>,
    pub(crate) common_buffer: Vec<i32>,
    /// `buffer_pointer`, as an index into `common_buffer`.
    pub(crate) buffer_pointer: usize,
    /* These would both fit into 32 bits, but they are often added in
    large multiples, so it's simpler to have two roomy ints */
    /* samples per MIDI delta-t */
    pub(crate) sample_increment: i32,
    pub(crate) sample_correction: i32,
    pub(crate) channel: [Channel; MAXCHAN],
    pub(crate) voice: Vec<Voice>,
    pub(crate) voices: i32,
    pub(crate) drumchannels: i32,
    pub(crate) buffered_count: i32,
    pub(crate) control_ratio: i32,
    pub(crate) lost_notes: i32,
    pub(crate) cut_notes: i32,
    pub(crate) samples: i32,
    pub(crate) events: Vec<MidiEvent>,
    /// `current_event`, as an index into `events`.
    pub(crate) current_event: usize,
    pub(crate) evlist: Vec<MidiEventList>,
    pub(crate) current_sample: i32,
    pub(crate) event_count: i32,
    pub(crate) at: i32,
    pub(crate) groomed_event_count: i32,
}

impl fmt::Debug for MidiSong {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MidiSong")
            .field("rate", &self.rate)
            .field("playing", &self.playing)
            .field("events", &self.groomed_event_count)
            .field("current_sample", &self.current_sample)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests;
