// Rust translation of src/decoder_timidity.c from SDL_mixer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! This file supports playing MIDI files with timidity (the
//! `sdl3-mixer-timidity` crate, which is under TiMidity's own license;
//! see its NOTICE).

use std::sync::Arc;

use sdl3::audio::{AudioFormat, AudioSpec, AudioStream};
use sdl3::error::{Error, Result};
use sdl3::io::{IoStream, IoWhence};
use sdl3::properties::Properties;
use sdl3::stdlib::getenv;
use sdl3_mixer_timidity::{self as timidity, MidiSong};

use crate::internal::{AudioData, Decoder, TrackData};
use crate::mixer::{frames_to_ms, ms_to_frames};

const SAMPLES_PER_DECODE: i32 = 256;

// Config file should contain any other directory that needs
//  to be added to the search path. The library adds the path
//  of the config file to its search path, too.
#[cfg(windows)]
static TIMIDITY_CFGS: &[&str] = &["C:\\TIMIDITY\\TIMIDITY.CFG"];
#[cfg(not(windows))] // unix:
static TIMIDITY_CFGS: &[&str] = &[
    "/etc/timidity.cfg",
    "/etc/timidity/timidity.cfg",
    "/etc/timidity/freepats.cfg",
];

/// Translation of `TIMIDITY_TrackData`.
struct TimidityTrackData<'a> {
    song: MidiSong,
    freq: i32,
    /// The track's stream (upstream's song keeps a pointer to it).
    _io: IoStream<'a>,
}

/// Translation of `TIMIDITY_init()`.
fn timidity_init() -> bool {
    if let Some(sf2) = getenv("TIMIDITY_SOUNDFONT") {
        if timidity::set_soundfont(Some(&sf2)).is_err() {
            // user override, no cfg
            return false;
        }
    }

    if let Some(cfg) = getenv("TIMIDITY_CFG") {
        // see if the user had one.
        return timidity::init(Some(&cfg)).is_ok(); // env or user override: no other tries
    }

    for cfg in TIMIDITY_CFGS {
        if timidity::init(Some(cfg)).is_ok() {
            return true;
        }
    }

    timidity::init(None).is_ok() // library's default cfg.
}

/// Translation of `TIMIDITY_quit()`.
fn timidity_quit() {
    timidity::exit();
}

/// Translation of `TIMIDITY_init_audio()`.
fn timidity_init_audio(
    io: Option<&mut IoStream<'_>>,
    spec: &mut AudioSpec,
    _props: &Properties,
    duration_frames: &mut i64,
) -> Result<Arc<dyn AudioData>> {
    let io = io.ok_or_else(|| Error::invalid_param("io"))?;

    // just load the bare minimum from the IOStream to verify it's a MIDI file.
    let mut magic = [0u8; 4];
    if io.read(&mut magic) != 4 {
        // (upstream sets no error of its own.)
        return Err(io
            .last_error()
            .cloned()
            .unwrap_or_else(|| Error::new("Not a MIDI audio stream")));
    } else if &magic != b"MThd" {
        return Err(Error::new("Not a MIDI audio stream"));
    }

    // Go back and do a proper load now to get metadata.
    io.seek(0, IoWhence::Set)?;

    spec.format = AudioFormat::S32; // timidity wants to do Sint32, and converts to other formats internally from Sint32.
    spec.channels = 2;
    // Use the device's current sample rate, already set in spec->freq

    let song = MidiSong::load(io, spec, SAMPLES_PER_DECODE)?;

    let mut song_length_in_frames =
        ms_to_frames(spec.freq, song.song_length() as i64).unwrap_or(-1);
    if song_length_in_frames < 0 {
        song_length_in_frames = 0;
    }
    drop(song);

    *duration_frames = song_length_in_frames;

    Ok(Arc::new(TimidityAudioData)) // no state.
}

/// `TIMIDITY_init_audio()`'s audio userdata (none: `NULL`). Its `Drop`
/// is `TIMIDITY_quit_audio()`.
struct TimidityAudioData;

impl AudioData for TimidityAudioData {
    /// Translation of `TIMIDITY_init_track()`.
    fn init_track<'a>(
        self: Arc<Self>,
        io: Option<IoStream<'a>>,
        spec: &AudioSpec,
        _props: &Properties,
    ) -> Result<Box<dyn TrackData + 'a>> {
        let mut io = io.ok_or_else(|| Error::invalid_param("io"))?;

        let Ok(mut song) = MidiSong::load(&mut io, spec, SAMPLES_PER_DECODE) else {
            return Err(Error::new("Timidity_LoadSong failed"));
        };

        song.set_volume(800); // !!! FIXME: maybe my test patches are really quiet?
        song.start();

        Ok(Box::new(TimidityTrackData {
            song,
            freq: spec.freq,
            _io: io,
        }))
    }
}

impl TrackData for TimidityTrackData<'_> {
    /// Translation of `TIMIDITY_decode()`.
    fn decode(&mut self, stream: &AudioStream) -> bool {
        // (Sint32 samples[SAMPLES_PER_DECODE * 2/*channels*/], as bytes.)
        // FIXME (upstream): at the end of the song, Timidity_PlaySome()
        // reports the samples it rendered since its last full buffer
        // without writing them, so upstream pushes whatever its
        // uninitialized buffer held; here, silence.
        let mut samples = [0u8; SAMPLES_PER_DECODE as usize * 2 * 4];
        let amount = self.song.play_some(&mut samples);
        if amount <= 0 {
            return false; // EOF or error, we're done either way.
        }

        let _ = stream.put_data(&samples[..(amount as usize).min(samples.len())]);
        true
    }

    /// Translation of `TIMIDITY_seek()`.
    fn seek(&mut self, frame: u64) -> Result<()> {
        let mut ticks = frames_to_ms(self.freq, frame as i64).unwrap_or(-1);
        if ticks < 0 {
            ticks = 0;
        }
        self.song.seek(ticks as u32); // !!! FIXME: this returns void, what happens if we seek past EOF?
        Ok(())
    }
}

// (TIMIDITY_quit_track() is TimidityTrackData's Drop: Timidity_Stop() and
// Timidity_FreeSong(), which is MidiSong's Drop.)

/// Translation of `MIX_Decoder_TIMIDITY`.
pub(crate) static DECODER: Decoder = Decoder {
    name: "TIMIDITY",
    init: Some(timidity_init),
    init_audio: timidity_init_audio,
    has_jump_to_order: false,
    quit: Some(timidity_quit),
};
