// Rust translation of src/decoder_mpg123.c from SDL_mixer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! MP3 decoder (and MPEG audio layers I and II), using libmpg123 (the
//! `sdl3-mixer-mpg123` crate, which is under the LGPL, version 2.1; see
//! its NOTICE).

// This file supports MP3 audio streams using libmpg123.

// (The loader of the dynamically loaded library, MPG123_DYNAMIC, isn't
// translated: the library is a crate here, always there when the `mpg123`
// feature is on. It is libmpg123 1.33's API 49, so only the
// `MPG123_API_VERSION >= 49` paths are, and DEBUG_MPG123 is off.)

use std::cmp::Ordering;
use std::sync::Arc;

use sdl3::audio::{AudioFormat, AudioSpec, AudioStream};
use sdl3::error::{Error, Result};
use sdl3::io::{IoStatus, IoStream, IoWhence};
use sdl3::properties::Properties;
use sdl3::stdlib::string::strcasecmp;
use sdl3_mixer_mpg123::mpg123::*;
use sdl3_mixer_mpg123::{
    mpg123_exit, mpg123_init, mpg123_plain_strerror, mpg123_rates, Mpg123Handle, Mpg123Io,
    SEEK_CUR, SEEK_END, SEEK_SET,
};

use crate::internal::{borrow_io, AudioData, Decoder, TrackData};
use crate::PROP_AUDIO_DECODER_STRING;

/// Translation of `MPG123_init()`.
fn mpg123_decoder_init() -> bool {
    // (LoadModule_mpg123() always succeeds: the library is linked in.)

    mpg123_init();

    true
}

/// Translation of `MPG123_quit()`.
fn mpg123_decoder_quit() {
    mpg123_exit();
    // (UnloadModule_mpg123() has nothing to do.)
}

/// The I/O handle `mpg123_open_handle64()` gets: the stream, read and
/// sought by `MPG123_IoRead()` and `MPG123_IoSeek()`. Its `Drop` is
/// `MPG123_IoClose()`, which does nothing (we will free the file later):
/// the stream is borrowed, or the track's, which owns it.
struct Mpg123IoStream<'a>(IoStream<'a>);

impl Mpg123Io for Mpg123IoStream<'_> {
    /// Translation of `MPG123_IoRead()`.
    fn read(&mut self, dst: &mut [u8], b: &mut usize) -> i32 {
        let io = &mut self.0;
        *b = io.read(dst);
        if *b == 0 && io.status() != IoStatus::Eof {
            return -1;
        }
        0
    }

    /// Translation of `MPG123_IoSeek()`.
    fn lseek(&mut self, offset: i64, whence: i32) -> i64 {
        // (SDL's whence values are the C library's.)
        let whence = match whence {
            SEEK_SET => IoWhence::Set,
            SEEK_CUR => IoWhence::Cur,
            SEEK_END => IoWhence::End,
            _ => return -1,
        };
        self.0.seek(offset, whence).unwrap_or(-1)
    }
}

/// Translation of `FormatMPG123ToSDL()`.
fn format_mpg123_to_sdl(fmt: i32) -> AudioFormat {
    match fmt {
        MPG123_ENC_SIGNED_8 => AudioFormat::S8,
        MPG123_ENC_UNSIGNED_8 => AudioFormat::U8,
        MPG123_ENC_SIGNED_16 => AudioFormat::S16,
        MPG123_ENC_SIGNED_32 => AudioFormat::S32,
        MPG123_ENC_FLOAT_32 => AudioFormat::F32,
        _ => AudioFormat::UNKNOWN,
    }
}

/// Translation of `mpg_err()`.
fn mpg_err(mpg: Option<&Mpg123Handle<'_>>, result: i32) -> &'static str {
    match mpg {
        Some(mpg) if result == MPG123_ERR => mpg.strerror(),
        _ => mpg123_plain_strerror(result),
    }
}

// stolen from dr_mp3's drmp3_hdr_valid() function.
/// Translation of `IsMp3Header()`.
fn is_mp3_header(h: &[u8; 4]) -> bool {
    let mp3_hdr_get_layer = (h[1] >> 1) & 3;
    let mp3_hdr_get_bitrate = h[2] >> 4;
    let mp3_hdr_get_sample_rate = (h[2] >> 2) & 3;
    h[0] == 0xff
        && ((h[1] & 0xF0) == 0xf0 || (h[1] & 0xFE) == 0xe2)
        && (mp3_hdr_get_layer != 0)
        && (mp3_hdr_get_bitrate != 15)
        && (mp3_hdr_get_sample_rate != 3)
}

/// The formats `MPG123_init_audio()` and `MPG123_init_track()` enable:
/// every rate libmpg123 knows, in mono and stereo, in S8, U8, S16, S32
/// and F32.
fn enable_formats(handle: &mut Mpg123Handle<'_>) {
    for &rate in mpg123_rates() {
        let channels = MPG123_MONO | MPG123_STEREO;
        let formats = MPG123_ENC_SIGNED_8
            | MPG123_ENC_UNSIGNED_8
            | MPG123_ENC_SIGNED_16
            | MPG123_ENC_SIGNED_32
            | MPG123_ENC_FLOAT_32;
        handle.format(rate, channels, formats);
    }
}

/// Translation of `MPG123_init_audio()`.
fn mpg123_init_audio(
    io: Option<&mut IoStream<'_>>,
    spec: &mut AudioSpec,
    props: &Properties,
    duration_frames: &mut i64,
) -> Result<Arc<dyn AudioData>> {
    let io = io.ok_or_else(|| Error::invalid_param("io"))?;

    // libmpg123 will accept almost any binary data and try to treat it as MPEG audio, which causes
    //  problems. Sometimes it will reject invalid data, but it appears to accept lots of things that
    //  are definitely not MPEG audio.
    // To limit it from accepting things that aren't MP3s, we do an extremely simple check to see if we
    //  see something kinda like a valid MPEG frame header at the start.
    // If this decoder was explicitly requested, we skip this check and let libmpg123 run with anything it
    //  thinks it can (overconfidently) handle.
    let decoder_name = props.get_string(PROP_AUDIO_DECODER_STRING);
    let accept_anything = decoder_name
        .as_deref()
        .is_some_and(|name| strcasecmp(name, "MPG123") == Ordering::Equal);
    if !accept_anything {
        // if it has an ID3 tag, we'll assume it's worth trying without further tests.
        if !props.contains("SDL_mixer.metadata.id3v1.title")
            && !props.contains("SDL_mixer.metadata.id3v2.TIT2")
            && !props.contains("SDL_mixer.metadata.id3v2.TT2")
        {
            let mut maybe_frame_header = [0u8; 4];
            if io.read(&mut maybe_frame_header) != maybe_frame_header.len() {
                // (upstream sets no error of its own.)
                return Err(io
                    .last_error()
                    .cloned()
                    .unwrap_or_else(|| Error::new("mpg123: (Probably) not MPEG audio data")));
            } else if !is_mp3_header(&maybe_frame_header) {
                return Err(Error::new("mpg123: (Probably) not MPEG audio data"));
            }
            // roll back and let mpg123 have it all.
            io.seek(0, IoWhence::Set)?;
        }
    }

    let mut encoding = 0;
    let mut rate = 0;

    let Ok(mut handle) = Mpg123Handle::new(None) else {
        return Err(Error::new("mpg123_new failed"));
    };

    // (On failure, upstream closes and deletes the handle after setting the
    // error: `failed`, which dropping it is.)
    handle.param2(MPG123_ADD_FLAGS, MPG123_QUIET, 0.0); // don't log errors to stderr.
    let result = handle.reader64(true);
    if result != MPG123_OK {
        return Err(Error::new(format!(
            "mpg123_replace_reader_handle: {}",
            mpg_err(Some(&handle), result)
        )));
    }

    let result = handle.format_none();
    if result != MPG123_OK {
        return Err(Error::new(format!(
            "mpg123_format_none: {}",
            mpg_err(Some(&handle), result)
        )));
    }

    enable_formats(&mut handle);

    let result = handle.open_handle64(Box::new(Mpg123IoStream(borrow_io(io))));
    if result != MPG123_OK {
        return Err(Error::new(format!(
            "mpg123_open_handle: {}",
            mpg_err(Some(&handle), result)
        )));
    }

    let result = handle.getformat(
        Some(&mut rate),
        Some(&mut spec.channels),
        Some(&mut encoding),
    );
    if result != MPG123_OK {
        return Err(Error::new(format!(
            "mpg123_getformat: {}",
            mpg_err(Some(&handle), result)
        )));
    }

    spec.format = format_mpg123_to_sdl(encoding);
    debug_assert!(spec.format != AudioFormat::UNKNOWN);
    spec.freq = rate as i32;

    let result = handle.scan(); // parse through whole file; it makes mpg123_length() accurate even if MP3 metadata is missing.
    if result != MPG123_OK {
        return Err(Error::new(format!(
            "mpg123_scan: {}",
            mpg_err(Some(&handle), result)
        )));
    }

    // mpg123_length() returns sample frames, or MPG123_ERR, which happens to be -1, which we use for "don't know" here.
    *duration_frames = handle.length64();

    handle.close();
    drop(handle);

    Ok(Arc::new(Mpg123AudioData)) // no state.
}

/// `MPG123_init_audio()`'s audio userdata (none: `NULL`). Its `Drop` is
/// `MPG123_quit_audio()`, which has nothing to do.
struct Mpg123AudioData;

impl AudioData for Mpg123AudioData {
    /// Translation of `MPG123_init_track()`.
    fn init_track<'a>(
        self: Arc<Self>,
        io: Option<IoStream<'a>>,
        _spec: &AudioSpec,
        _props: &Properties,
    ) -> Result<Box<dyn TrackData + 'a>> {
        let io = io.ok_or_else(|| Error::invalid_param("io"))?;

        let Ok(mut handle) = Mpg123Handle::new(None) else {
            return Err(Error::new("mpg123_new failed"));
        };

        // (Each failure deletes the handle: drops it.)
        handle.param2(MPG123_ADD_FLAGS, MPG123_QUIET, 0.0); // don't log errors to stderr.
        let result = handle.reader64(true);
        if result != MPG123_OK {
            return Err(Error::new(format!(
                "mpg123_replace_reader_handle: {}",
                mpg_err(Some(&handle), result)
            )));
        }

        // !!! FIXME: can I just force the `spec` format here, or does that cause problems if we have a "frankenstein" mp3 that changes formats later?
        let result = handle.format_none();
        if result != MPG123_OK {
            return Err(Error::new(format!(
                "mpg123_format_none: {}",
                mpg_err(Some(&handle), result)
            )));
        }

        enable_formats(&mut handle);

        let result = handle.open_handle64(Box::new(Mpg123IoStream(io)));
        if result != MPG123_OK {
            return Err(Error::new(format!(
                "mpg123_open_handle: {}",
                mpg_err(Some(&handle), result)
            )));
        }

        // !!! FIXME: should we do this? It makes seeking better, maybe?  mpg123.mpg123_scan(handle);

        Ok(Box::new(Mpg123TrackData { handle }))
    }
}

/// `MPG123_init_track()`'s track userdata: the handle. Its `Drop`
/// (`mpg123_delete()`, which closes it) is `MPG123_quit_track()`.
struct Mpg123TrackData<'a> {
    handle: Box<Mpg123Handle<'a>>,
}

impl TrackData for Mpg123TrackData<'_> {
    /// Translation of `MPG123_decode()`.
    fn decode(&mut self, stream: &AudioStream) -> bool {
        let handle = &mut self.handle;
        let mut amount = 0;
        let mut rate = 0;
        let mut channels = 0;
        let mut encoding = 0;
        let mut buffer = [0u8; 1024];

        let mut result = handle.read(&mut buffer, &mut amount);

        if result == MPG123_NEW_FORMAT {
            result = handle.getformat(Some(&mut rate), Some(&mut channels), Some(&mut encoding));
            if result != MPG123_OK {
                // (upstream sets the error; decode() can only report false)
                let _ = Error::new(format!(
                    "mpg123_getformat: {}",
                    mpg_err(Some(handle), result)
                ));
                return false;
            }

            let spec = AudioSpec {
                format: format_mpg123_to_sdl(encoding),
                channels,
                freq: rate as i32,
            };
            debug_assert!(spec.format != AudioFormat::UNKNOWN);

            let _ = stream.set_format(Some(&spec), None);
        }

        if amount > 0 {
            let _ = stream.put_data(&buffer[..amount]);
        }

        if (result != MPG123_OK) && (result != MPG123_DONE) {
            // (upstream sets the error; decode() can only report false)
            let _ = Error::new(format!("mpg123_read: {}", mpg_err(Some(handle), result)));
        }

        result == MPG123_OK
    }

    /// Translation of `MPG123_seek()`.
    fn seek(&mut self, frame: u64) -> Result<()> {
        let handle = &mut self.handle;
        let rc = handle.seek64(frame as i64, SEEK_SET);
        if rc < 0 {
            Err(Error::new(format!(
                "mpg123_seek:{}",
                mpg_err(Some(handle), rc as i32)
            )))
        } else {
            Ok(())
        }
    }
}

/// Translation of `MIX_Decoder_MPG123`.
pub(crate) static DECODER: Decoder = Decoder {
    name: "MPG123",
    init: Some(mpg123_decoder_init),
    init_audio: mpg123_init_audio,
    has_jump_to_order: false,
    quit: Some(mpg123_decoder_quit),
};
