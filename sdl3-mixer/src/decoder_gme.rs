// Rust translation of src/decoder_gme.c from SDL_mixer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! This file supports the audio of classic video game consoles with
//! game-music-emu (the `sdl3-mixer-gme` crate, which is under the GNU LGPL
//! 2.1 or later; see its NOTICE).

use std::sync::Arc;

use sdl3::audio::{AudioFormat, AudioSpec, AudioStream};
use sdl3::error::{Error, Result};
use sdl3::io::{IoStream, IoWhence};
use sdl3::properties::Properties;
use sdl3_mixer_gme::{self as gme, MusicEmu};

use crate::internal::{AudioData, Decoder, TrackData};
use crate::mixer::ms_to_frames;
use crate::{
    DURATION_INFINITE, PROP_METADATA_ALBUM_STRING, PROP_METADATA_ARTIST_STRING,
    PROP_METADATA_COPYRIGHT_STRING, PROP_METADATA_TITLE_STRING,
};

/// Translation of `MIX_SlurpConstIO()`: the whole stream, from its start
/// (upstream takes a memory stream's memory as it is, and reads any other
/// stream from its start; a stream here doesn't expose its memory, so this
/// always reads it from its start, which gives the same bytes).
fn slurp_const_io(io: &mut IoStream<'_>) -> Result<Vec<u8>> {
    io.seek(0, IoWhence::Set)?;
    io.load_all()
}

/// The bytes of a track information string as a property's string.
fn prop_string(s: &[u8]) -> String {
    String::from_utf8_lossy(s).into_owned()
}

/// Translation of `GME_init_audio()`.
fn gme_init_audio(
    io: Option<&mut IoStream<'_>>,
    spec: &mut AudioSpec,
    props: &Properties,
    duration_frames: &mut i64,
) -> Result<Arc<dyn AudioData>> {
    let io = io.ok_or_else(|| Error::invalid_param("io"))?;

    // just load the bare minimum from the IOStream to verify it's a supported file.
    let mut magic = [0u8; 4];
    if io.read(&mut magic) != 4 {
        // (upstream sets no error of its own.)
        return Err(io
            .last_error()
            .cloned()
            .unwrap_or_else(|| Error::new("Not a libgme-supported audio stream")));
    }

    let ext = gme::gme_identify_header(&magic);
    if ext.is_empty() {
        return Err(Error::new("Not a libgme-supported audio stream"));
    }

    // Go back and do a proper load now to get metadata.
    let data = slurp_const_io(io)?;

    let emu = gme::gme_open_data(&data, gme::GME_INFO_ONLY)
        .map_err(|err| Error::new(format!("gme_open_data failed: {err}")))?;
    drop(data);

    *duration_frames = -1;

    // if this fails, oh well.
    if let Ok(info) = gme::gme_track_info(emu.as_ref(), 0) {
        let set_gme_metadata = |name: &str, s: &[u8], mixerprop: Option<&str>| {
            if !s.is_empty() {
                let s = prop_string(s);
                let _ = props.set(&format!("SDL_mixer.metadata.gme.{name}"), s.clone());
                if let Some(mixerprop) = mixerprop {
                    let _ = props.set(mixerprop, s);
                }
            }
        };
        set_gme_metadata("song", &info.song, Some(PROP_METADATA_TITLE_STRING));
        set_gme_metadata("author", &info.author, Some(PROP_METADATA_ARTIST_STRING));
        set_gme_metadata("game", &info.game, Some(PROP_METADATA_ALBUM_STRING));
        set_gme_metadata(
            "copyright",
            &info.copyright,
            Some(PROP_METADATA_COPYRIGHT_STRING),
        );
        set_gme_metadata("system", &info.system, None);
        set_gme_metadata("comment", &info.comment, None);
        set_gme_metadata("dumper", &info.dumper, None);

        if (info.intro_length >= 0) && (info.loop_length > 0) {
            *duration_frames = DURATION_INFINITE;
        } else if info.length >= 0 {
            *duration_frames = ms_to_frames(spec.freq, info.length as i64).unwrap_or(-1);
            if *duration_frames < 0 {
                *duration_frames = 0;
            }
        }
    }

    drop(emu); // gme_delete()

    // libgme only outputs Sint16 stereo data.
    spec.format = AudioFormat::S16;
    spec.channels = 2;
    // libgme generates in whatever sample rate, so use the current device spec->freq.

    Ok(Arc::new(GmeAudioData)) // no state.
}

/// `GME_init_audio()`'s audio userdata (none: `NULL`). Its `Drop` is
/// `GME_quit_audio()`.
struct GmeAudioData;

/// The track userdata: the emulator (upstream's `Music_Emu*`), whose `Drop`
/// is `GME_quit_track()`'s `gme_delete()`.
struct GmeTrackData<'a> {
    emu: Box<dyn MusicEmu>,
    /// The track's stream (upstream reads the file from it and leaves it be).
    _io: IoStream<'a>,
}

impl AudioData for GmeAudioData {
    /// Translation of `GME_init_track()`.
    fn init_track<'a>(
        self: Arc<Self>,
        io: Option<IoStream<'a>>,
        spec: &AudioSpec,
        _props: &Properties,
    ) -> Result<Box<dyn TrackData + 'a>> {
        let mut io = io.ok_or_else(|| Error::invalid_param("io"))?;

        let data = slurp_const_io(&mut io)?;

        let mut emu = gme::gme_open_data(&data, spec.freq)
            .map_err(|err| Error::new(format!("gme_open_data failed: {err}")))?;
        drop(data);

        // Set this flag BEFORE calling the gme_start_track() to fix an inability to loop forever
        gme::gme_set_autoload_playback_limit(emu.as_mut(), 0);

        if let Err(err) = gme::gme_start_track(emu.as_mut(), 0) {
            return Err(Error::new(format!("gme_start_track failed: {err}")));
        }

        Ok(Box::new(GmeTrackData { emu, _io: io }))
    }
}

impl TrackData for GmeTrackData<'_> {
    /// Translation of `GME_decode()`.
    fn decode(&mut self, stream: &AudioStream) -> bool {
        let emu = self.emu.as_mut();
        if gme::gme_track_ended(emu) != 0 {
            return false; // all done.
        }

        // FIXME (upstream): left uninitialized, and when the emulator stops
        // with an error (as the SPC emulator does at a STOP instruction),
        // libgme leaves the rest of the samples unwritten, and they are
        // played; they start zeroed here, and upstream's build for the
        // tests is patched the same.
        let mut samples = [0i16; 256];
        if gme::gme_play(emu, &mut samples).is_err() {
            return false; // i guess we're done. (upstream sets the error)
        }

        let mut bytes = [0u8; 512];
        for (b, s) in bytes.chunks_exact_mut(2).zip(samples) {
            b.copy_from_slice(&s.to_ne_bytes());
        }
        let _ = stream.put_data(&bytes);

        true // had more data to decode.
    }

    /// Translation of `GME_seek()`. Note (upstream): the frame is passed
    /// as a count of samples, of which there are two per (stereo) frame,
    /// so this seeks to half the frame asked for.
    fn seek(&mut self, frame: u64) -> Result<()> {
        // FIXME (upstream): an odd count of samples never finishes
        // (Game_Music_Emu plays whole stereo frames, so its Classic_Emu
        // play_() loops forever for the half frame left, running its
        // emulator past its buffers): this rounds it down to an even one.
        gme::gme_seek_samples(self.emu.as_mut(), frame as i32 & !1)
            .map_err(|err| Error::new(format!("gme_seek_samples failed: {err}")))
    }

    /// Translation of `GME_jump_to_order()`.
    fn jump_to_order(&mut self, order: i32) -> Result<()> {
        gme::gme_start_track(self.emu.as_mut(), order)
            .map_err(|err| Error::new(format!("gme_start_track failed: {err}")))
    }
}

/// Translation of `MIX_Decoder_GME` (`GME_init()` and `GME_quit()` load
/// and unload libgme, which is built in here).
pub(crate) static DECODER: Decoder = Decoder {
    name: "GME",
    init: None,
    init_audio: gme_init_audio,
    has_jump_to_order: true,
    quit: None,
};
