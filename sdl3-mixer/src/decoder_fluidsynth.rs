// Rust translation of src/decoder_fluidsynth.c from SDL_mixer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! This file supports playing MIDI files with FluidSynth (the
//! `sdl3-mixer-fluidsynth` crate, which is under FluidSynth's LGPL; see
//! its NOTICE).

// SDL2_mixer's FluidSynth code was written by James Le Cuirot; this was rewritten using that work as a reference.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use sdl3::audio::{AudioFormat, AudioSpec, AudioStream};
use sdl3::error::{Error, Result};
use sdl3::io::{IoInterface, IoStop, IoStream, IoWhence};
use sdl3::properties::{Properties, ValueKind};
use sdl3_mixer_fluidsynth::{
    self as fluidsynth, Player, Settings, SfFile, SfLoader, Synth, FLUID_FAILED, FLUID_OK,
    FLUID_PLAYER_PLAYING, SEEK_CUR, SEEK_END, SEEK_SET,
};

use crate::internal::{
    put_f32, AudioData, Decoder, TrackData, PROP_DECODER_FLUIDSYNTH_PROPS_NUMBER,
    PROP_DECODER_FLUIDSYNTH_SOUNDFONT_IOSTREAM_POINTER,
    PROP_DECODER_FLUIDSYNTH_SOUNDFONT_PATH_STRING,
};

/// Translation of `FLUIDSYNTH_AudioData`.
struct FluidsynthAudioData {
    sfdata: Option<Arc<[u8]>>,
    fluidsynth_props: Option<Properties>,
}

/// Translation of `FLUIDSYNTH_TrackData` (`adata` is not kept: nothing
/// reads it after `FLUIDSYNTH_init_track()`). The fields drop in
/// `FLUIDSYNTH_quit_track()`'s order.
struct FluidsynthTrackData<'a> {
    player: Player,
    synth: Synth,
    _settings: Settings,
    #[allow(dead_code)] // (upstream keeps it, and reads it nowhere)
    freq: i32,
    current_frame: u64,
    /// The track's stream (which upstream's track keeps until it ends).
    _io: IoStream<'a>,
}

/// Translation of `FLUIDSYNTH_init()`.
fn fluidsynth_init() -> bool {
    // don't let FluidSynth touch hardware directly under any circumstances.
    let no_drivers: &[&str] = &[];
    fluidsynth::audio_driver_register(Some(no_drivers));
    true
}

/// Translation of `FLUIDSYNTH_quit()`.
fn fluidsynth_quit() {}

/// Translation of `FLUIDSYNTH_init_audio()`.
fn fluidsynth_init_audio(
    io: Option<&mut IoStream<'_>>,
    spec: &mut AudioSpec,
    props: &Properties,
    duration_frames: &mut i64,
) -> Result<Arc<dyn AudioData>> {
    let io = io.ok_or_else(|| Error::invalid_param("io"))?;

    // Try to load a soundfont file if we can.
    //  If the app provided an IOStream for it, use it. If not, see if they provided a path.
    // (Here the app's stream is read from where it is and never closed, as
    // the caller owns it.)
    let app_sfio = props
        .get_any::<Mutex<IoStream<'static>>>(PROP_DECODER_FLUIDSYNTH_SOUNDFONT_IOSTREAM_POINTER);
    let mut sfio: Option<IoStream<'static>> = None;
    if app_sfio.is_none() {
        // Try to load a correction file if available.
        if let Some(sfpath) = props.get_string(PROP_DECODER_FLUIDSYNTH_SOUNDFONT_PATH_STRING) {
            sfio = IoStream::from_file(&sfpath, "rb").ok();
        }

        // (SDL_PLATFORM_UNIX: not Apple's platforms)
        #[cfg(all(unix, not(target_vendor = "apple")))]
        {
            if sfio.is_none() {
                // this happens to be where Ubuntu stores a usable soundfont, at least on my laptop. Try it if nothing else worked out.
                sfio = IoStream::from_file("/usr/share/sounds/sf2/default-GM.sf2", "rb").ok();
            }
            if sfio.is_none() {
                // Fedora is storing it here.
                sfio = IoStream::from_file("/usr/share/soundfonts/default.sf2", "rb").ok();
            }
            if sfio.is_none() {
                // Fedora is _also_ storing it here. Both are symlinks to the same thing elsewhere, but try both in case one was an older path.
                sfio = IoStream::from_file("/usr/share/sounds/sf2/default.sf2", "rb").ok();
            }
        }
    }

    // !!! FIXME: should we fail if there's no soundfont? It's not going to generate sound without one, unless there's a system-provided one in some cases...?

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

    let mut sfdata: Option<Arc<[u8]>> = None;
    if let Some(app_sfio) = app_sfio {
        let mut sfio = app_sfio.lock().unwrap_or_else(|e| e.into_inner());
        sfdata = Some(sfio.load_all()?.into());
    } else if let Some(mut sfio) = sfio {
        sfdata = Some(sfio.load_all()?.into());
        // (closesfio: dropping it closes it.)
    }

    spec.format = AudioFormat::F32;
    spec.channels = 2;
    // Use the device's current sample rate, already set in spec->freq

    *duration_frames = -1; // !!! FIXME: fluid_player_get_total_ticks can give us a time duration, but we don't have a player until we set up the track later.

    // (the app's FluidSynth settings: a Properties handle, set with
    // `Properties::set_any`, as upstream's number is an SDL_PropertiesID.)
    let mut fluidsynth_props = None;
    if let Some(src) = props.get_any::<Properties>(PROP_DECODER_FLUIDSYNTH_PROPS_NUMBER) {
        let copy = Properties::new();
        copy.copy_from(&src)?;
        fluidsynth_props = Some(copy);
    }

    Ok(Arc::new(FluidsynthAudioData {
        sfdata,
        fluidsynth_props,
    }))
}

/// `SDL_IOFromConstMem()` over the SoundFont data the audio keeps (as an
/// `IoInterface`, so the stream can outlive the borrow).
struct SoundFontMem {
    data: Arc<[u8]>,
    here: usize,
}

impl IoInterface for SoundFontMem {
    fn has_size(&self) -> bool {
        true
    }
    fn size(&mut self) -> Result<i64> {
        Ok(self.data.len() as i64)
    }
    /// Translation of `mem_seek()`.
    fn seek(&mut self, offset: i64, whence: IoWhence) -> Result<i64> {
        let base = match whence {
            IoWhence::Set => 0i64,
            IoWhence::Cur => self.here as i64,
            IoWhence::End => self.data.len() as i64,
        };
        self.here = base.saturating_add(offset).clamp(0, self.data.len() as i64) as usize;
        Ok(self.here as i64)
    }
    fn can_read(&self) -> bool {
        true
    }
    /// Translation of `mem_read()`.
    fn read(&mut self, buf: &mut [u8]) -> std::result::Result<usize, IoStop> {
        let n = buf.len().min(self.data.len() - self.here);
        buf[..n].copy_from_slice(&self.data[self.here..self.here + n]);
        self.here += n;
        if n < buf.len() {
            return Err(IoStop::status(n, sdl3::io::IoStatus::Eof));
        }
        Ok(n)
    }
}

/// The SoundFont stream FluidSynth's loader reads: `SoundFontRead()`,
/// `SoundFontSeek()`, `SoundFontTell()` and `SoundFontClose()` over it.
struct SoundFontFile(IoStream<'static>);

impl SfFile for SoundFontFile {
    /// Translation of `SoundFontRead()`.
    fn read(&mut self, buf: &mut [u8]) -> i32 {
        // (count can't be negative here, nor more than memory holds.)
        if self.0.read(buf) == buf.len() {
            FLUID_OK
        } else {
            FLUID_FAILED
        }
    }

    /// Translation of `SoundFontSeek()`.
    fn seek(&mut self, offset: i64, origin: i32) -> i32 {
        let whence = match origin {
            SEEK_SET => IoWhence::Set,
            SEEK_CUR => IoWhence::Cur,
            SEEK_END => IoWhence::End,
            _ => return FLUID_FAILED,
        };
        match self.0.seek(offset, whence) {
            Ok(pos) if pos >= 0 => FLUID_OK,
            _ => FLUID_FAILED,
        }
    }

    /// Translation of `SoundFontTell()`.
    fn tell(&mut self) -> i64 {
        self.0.tell().unwrap_or(-1)
    }

    /// Translation of `SoundFontClose()`.
    fn close(self: Box<Self>) -> i32 {
        let _ = self.0.close();
        FLUID_OK
    }
}

/// Translation of `SetCustomFluidsynthProperties()`.
fn set_custom_fluidsynth_properties(settings: &Settings, props: &Properties, name: &str) {
    match props.kind(name) {
        Some(ValueKind::Number) => {
            settings.setint(name, props.get_number(name).unwrap_or(0) as i32);
        }
        Some(ValueKind::Float) => {
            settings.setnum(name, props.get_float(name).unwrap_or(0.0) as f64);
        }
        Some(ValueKind::String) => {
            settings.setstr(name, Some(&props.get_string(name).unwrap_or_default()));
        }
        _ => {} // oh well.
    }
}

/// The address `"&FAKESFNAME&%p"` prints: a new stream's. Here a number
/// that is different for each one.
fn fake_soundfont_address() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(0x1000);
    NEXT.fetch_add(0x100, Ordering::Relaxed)
}

impl AudioData for FluidsynthAudioData {
    /// Translation of `FLUIDSYNTH_init_track()`.
    fn init_track<'a>(
        self: Arc<Self>,
        io: Option<IoStream<'a>>,
        spec: &AudioSpec,
        _props: &Properties,
    ) -> Result<Box<dyn TrackData + 'a>> {
        let mut io = io.ok_or_else(|| Error::invalid_param("io"))?;
        let adata = &*self;
        let mut samplerate = 0.0;

        let settings = Settings::new();

        //settings.setint("synth.cpu-cores", 2);
        settings.setnum("synth.gain", 1.0);
        settings.setnum("synth.sample-rate", spec.freq as f64);
        settings.getnum("synth.sample-rate", &mut samplerate);

        // these settings were recommended as good defaults in https://github.com/libsdl-org/SDL_mixer/issues/284
        settings.setnum("synth.chorus.depth", 5.0);
        settings.setnum("synth.chorus.level", 0.35);
        settings.setnum("synth.reverb.damp", 0.4);
        settings.setnum("synth.reverb.level", 0.15);
        settings.setnum("synth.reverb.width", 4.0);
        settings.setnum("synth.reverb.room-size", 0.6);

        // let custom properties override anything we already set internally. You break it, you buy it!
        if let Some(fprops) = &adata.fluidsynth_props {
            for name in fprops.names() {
                set_custom_fluidsynth_properties(&settings, fprops, &name);
            }
        }

        let Some(synth) = Synth::new(&settings) else {
            return Err(Error::new("Failed to create FluidSynth synthesizer"));
        };

        if let Some(sfdata) = &adata.sfdata {
            let mut sfloader = SfLoader::new_defsfloader(&settings);

            // (the stream the fake file name points to; FluidSynth opens
            // it once, and closes it.)
            let fakefname = format!("&FAKESFNAME&0x{:x}", fake_soundfont_address());
            let stream = Mutex::new(Some(IoStream::open(SoundFontMem {
                data: sfdata.clone(),
                here: 0,
            })));
            let expected = fakefname.clone();
            // Translation of `SoundFontOpen()` (which parses the stream's
            // address back out of the name).
            sfloader.set_callbacks(Arc::new(move |filename: &str| -> Option<Box<dyn SfFile>> {
                // Note (upstream): opened again, upstream hands out the
                // stream it already closed; here it fails.
                if filename != expected {
                    return None;
                }
                let io = stream.lock().unwrap_or_else(|e| e.into_inner()).take()?;
                Some(Box::new(SoundFontFile(io)))
            }));
            synth.add_sfloader(sfloader);
            if synth.sfload(&fakefname, true) == FLUID_FAILED {
                return Err(Error::new("Failed to load FluidSynth soundfont"));
            }
        }

        // libfluidsynth doesn't have an abstract loader mechanism, you have to either give it a file path or the whole thing
        //  in one memory buffer upfront. Fortunately MIDI files tend to be extremely small, so just slurp it all in.
        // (and, if `io` appears to be a memory-based SDL_IOStream, just hand it the memory buffer directly.)
        io.seek(0, IoWhence::Set)?;
        let buffer = io.load_all()?;

        let Some(player) = Player::new(&synth) else {
            return Err(Error::new("Failed to create FluidSynth player"));
        };

        let okay = player.add_mem(&buffer) == FLUID_OK;
        drop(buffer);

        if !okay {
            player.stop();
            return Err(Error::new("FluidSynth failed to load in-memory song"));
        }

        if player.play() != FLUID_OK {
            player.stop();
            return Err(Error::new("Failed to start FluidSynth player"));
        }

        Ok(Box::new(FluidsynthTrackData {
            player,
            synth,
            _settings: settings,
            freq: samplerate as i32,
            current_frame: 0,
            _io: io,
        }))
    }
}

impl TrackData for FluidsynthTrackData<'_> {
    /// Translation of `FLUIDSYNTH_decode()`.
    fn decode(&mut self, stream: &AudioStream) -> bool {
        if self.player.get_status() != FLUID_PLAYER_PLAYING {
            return false;
        }

        let mut samples = [0f32; 256];
        if self
            .synth
            .write_float(samples.len() as i32 / 2, &mut samples, 0, 2, None, 1, 2)
            != FLUID_OK
        {
            return false; // maybe EOF...?
        }

        let _ = put_f32(stream, &samples);
        self.current_frame += samples.len() as u64 / 2;
        true
    }

    /// Translation of `FLUIDSYNTH_seek()`.
    fn seek(&mut self, frame: u64) -> Result<()> {
        // This is expensive, but it's not trivial to seek to a specific frame in fluidsynth for various reasons.
        //  (see some explanations in https://github.com/libsdl-org/SDL_mixer/issues/519)
        if self.current_frame > frame {
            if self.player.seek(0) != FLUID_OK {
                return Err(Error::new("Couldn't rewind MIDI track"));
            }
            self.current_frame = 0;

            if self.player.get_status() != FLUID_PLAYER_PLAYING && self.player.play() != FLUID_OK {
                return Err(Error::new("Failed to restart FluidSynth player"));
            }
        }

        let mut remaining_frames = frame - self.current_frame;
        while remaining_frames > 0 {
            if self.player.get_status() != FLUID_PLAYER_PLAYING {
                return Err(Error::new("Seek past end of MIDI file"));
            }

            let mut samples = [0f32; 512];
            let write_frames = (samples.len() as u64 / 2).min(remaining_frames);
            if self
                .synth
                .write_float(write_frames as i32, &mut samples, 0, 2, None, 1, 2)
                != FLUID_OK
            {
                return Err(Error::new("Seek past end of MIDI file")); // maybe EOF...?
            }
            self.current_frame += write_frames;
            remaining_frames -= write_frames;
        }

        Ok(())
    }
}

/// Translation of `FLUIDSYNTH_quit_track()` (then the player, the synth
/// and the settings drop, in that order).
impl Drop for FluidsynthTrackData<'_> {
    fn drop(&mut self) {
        self.player.stop();
    }
}

// (FLUIDSYNTH_quit_audio() is FluidsynthAudioData's Drop.)

/// Translation of `MIX_Decoder_FLUIDSYNTH`.
pub(crate) static DECODER: Decoder = Decoder {
    name: "FLUIDSYNTH",
    init: Some(fluidsynth_init),
    init_audio: fluidsynth_init_audio,
    has_jump_to_order: false,
    quit: Some(fluidsynth_quit),
};
