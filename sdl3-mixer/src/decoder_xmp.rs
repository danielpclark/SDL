// Rust translation of src/decoder_xmp.c from SDL_mixer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! This file supports playing tracker modules (MOD, XM, S3M, IT and the
//! other formats libxmp loads) with libxmp, which is translated in
//! `xmp/` (under its MIT license; see LICENSE.txt), as upstream builds it
//! when it is vendored.

use std::sync::Arc;

use sdl3::audio::{AudioFormat, AudioSpec, AudioStream};
use sdl3::error::{Error, Result};
use sdl3::io::{IoStream, IoWhence};
use sdl3::properties::Properties;

use crate::internal::{AudioData, Decoder, TrackData};
use crate::mixer::{frames_to_ms, ms_to_frames};
use crate::xmp::{self, cstr, XmpCallbacks, XmpContext, XmpFrameInfo};
use crate::{PROP_METADATA_COPYRIGHT_STRING, PROP_METADATA_TITLE_STRING};

/// `XMP_IoCallbacks`: `XMP_IoRead()`, `XMP_IoSeek()` and `XMP_IoTell()`
/// on the stream (its `close_func` is `NULL`).
struct XmpIoCallbacks<'s, 'a>(&'s mut IoStream<'a>);

impl XmpCallbacks for XmpIoCallbacks<'_, '_> {
    /// Translation of `XMP_IoRead()`.
    fn read_func(&mut self, dest: &mut [u8], len: usize, nmemb: usize) -> usize {
        if len > 0 && nmemb > 0 {
            self.0.read(&mut dest[..len * nmemb]) / len
        } else {
            0
        }
    }

    /// Translation of `XMP_IoSeek()`.
    fn seek_func(&mut self, offset: i64, whence: i32) -> i32 {
        let whence = match whence {
            0 => IoWhence::Set,
            1 => IoWhence::Cur,
            2 => IoWhence::End,
            _ => {
                // (SDL_SeekIO() refuses other values.)
                return -1;
            }
        };
        if self.0.seek(offset, whence).is_err() {
            -1
        } else {
            0
        }
    }

    /// Translation of `XMP_IoTell()`.
    fn tell_func(&mut self) -> i64 {
        self.0.tell().unwrap_or(-1)
    }
}

/// Translation of `SetLibXmpError()`.
fn set_lib_xmp_error(function: &str, error: i32) -> Error {
    let name = match -error {
        xmp::XMP_ERROR_INTERNAL => "XMP_ERROR_INTERNAL",
        xmp::XMP_ERROR_FORMAT => "XMP_ERROR_FORMAT",
        xmp::XMP_ERROR_LOAD => "XMP_ERROR_LOAD",
        xmp::XMP_ERROR_DEPACK => "XMP_ERROR_DEPACK",
        xmp::XMP_ERROR_SYSTEM => "XMP_ERROR_SYSTEM",
        xmp::XMP_ERROR_INVALID => "XMP_ERROR_INVALID",
        xmp::XMP_ERROR_STATE => "XMP_ERROR_STATE",
        _ => return Error::new(format!("{}: unknown error {}", function, error)),
    };
    Error::new(format!("{}: {}", function, name))
}

/// Translation of `XMP_TrackData`.
struct XmpTrackData<'a> {
    freq: i32,
    ctx: XmpContext,
    /// The track's stream (upstream loads the module from it and leaves it
    /// be).
    _io: IoStream<'a>,
}

/// The bytes of a C string as a property's string.
fn prop_string(s: &[u8]) -> String {
    String::from_utf8_lossy(cstr(s)).into_owned()
}

/// Translation of `XMP_init_audio()`.
fn xmp_init_audio(
    io: Option<&mut IoStream<'_>>,
    spec: &mut AudioSpec,
    props: &Properties,
    duration_frames: &mut i64,
) -> Result<Arc<dyn AudioData>> {
    let io = io.ok_or_else(|| Error::invalid_param("io"))?;

    if xmp::xmp_test_module_from_callbacks(&mut XmpIoCallbacks(io), None) < 0 {
        return Err(Error::new("Not a libxmp-supported format"));
    }

    // If we're still here, we either have valid data, or we're on libxmp < 4.5. In either case, just load the whole thing and see what happens.

    // Go back and do a proper load now to get metadata.
    io.seek(0, IoWhence::Set)?;

    let mut ctx = xmp::xmp_create_context();

    let err = xmp::xmp_load_module_from_callbacks(&mut ctx, &mut XmpIoCallbacks(io));
    if err != 0 {
        xmp::xmp_free_context(ctx);
        return Err(set_lib_xmp_error("xmp_load_module_from_memory", err));
    }

    if let Some(info) = xmp::xmp_get_module_info(&ctx) {
        if let Some(comment) = info.comment {
            if !cstr(comment).is_empty() {
                let comment = prop_string(comment);
                let _ = props.set("SDL_mixer.metadata.xmp.comment", comment.clone());
                if !props.contains(PROP_METADATA_COPYRIGHT_STRING) {
                    let _ = props.set(PROP_METADATA_COPYRIGHT_STRING, comment);
                }
            }
        }

        let mod_ = info.mod_;
        if mod_.name[0] != 0 {
            let name = prop_string(&mod_.name_cstr());
            let _ = props.set("SDL_mixer.metadata.xmp.name", name.clone());
            if !props.contains(PROP_METADATA_TITLE_STRING) {
                let _ = props.set(PROP_METADATA_TITLE_STRING, name);
            }
        }
        if mod_.type_[0] != 0 {
            let _ = props.set("SDL_mixer.metadata.xmp.type", prop_string(&mod_.type_));
        }
    }

    // !!! FIXME: is this necessary?
    let err = xmp::xmp_start_player(&mut ctx, spec.freq, 0); // start this running at device sample rate, so we can calculate total_time.
    if err != 0 {
        xmp::xmp_release_module(&mut ctx);
        xmp::xmp_free_context(ctx);
        return Err(set_lib_xmp_error("xmp_start_player", err));
    }

    let mut frame_info = XmpFrameInfo::default();
    xmp::xmp_get_frame_info(&ctx, &mut frame_info);

    *duration_frames = ms_to_frames(spec.freq, frame_info.total_time as i64).unwrap_or(-1); // closest we can get.
    if *duration_frames < 0 {
        *duration_frames = 0;
    }

    xmp::xmp_stop_module(&mut ctx);
    xmp::xmp_end_player(&mut ctx);
    xmp::xmp_release_module(&mut ctx);
    xmp::xmp_free_context(ctx);

    // libxmp prefers to generate Sint16, stereo data.
    spec.format = AudioFormat::S16;
    spec.channels = 2;
    // libxmp generates in whatever sample rate, so use the current device spec->freq.

    Ok(Arc::new(XmpAudioData)) // no state.
}

/// `XMP_init_audio()`'s audio userdata (none: `NULL`). Its `Drop` is
/// `XMP_quit_audio()`.
struct XmpAudioData;

impl AudioData for XmpAudioData {
    /// Translation of `XMP_init_track()`.
    fn init_track<'a>(
        self: Arc<Self>,
        io: Option<IoStream<'a>>,
        spec: &AudioSpec,
        _props: &Properties,
    ) -> Result<Box<dyn TrackData + 'a>> {
        let mut io = io.ok_or_else(|| Error::invalid_param("io"))?;

        let freq = spec.freq;

        let mut ctx = xmp::xmp_create_context();

        let err = xmp::xmp_load_module_from_callbacks(&mut ctx, &mut XmpIoCallbacks(&mut io));
        if err != 0 {
            xmp::xmp_free_context(ctx);
            return Err(set_lib_xmp_error("xmp_load_module_from_memory", err));
        }

        let err = xmp::xmp_start_player(&mut ctx, spec.freq, 0);
        if err != 0 {
            xmp::xmp_release_module(&mut ctx);
            xmp::xmp_free_context(ctx);
            return Err(set_lib_xmp_error("xmp_start_player", err));
        }

        Ok(Box::new(XmpTrackData { freq, ctx, _io: io }))
    }
}

impl TrackData for XmpTrackData<'_> {
    /// Translation of `XMP_decode()`.
    fn decode(&mut self, stream: &AudioStream) -> bool {
        if xmp::xmp_play_frame(&mut self.ctx) < 0 {
            return false; // either an error or EOF, either way we're done.
        }

        let mut info = XmpFrameInfo::default();
        xmp::xmp_get_frame_info(&self.ctx, &mut info);

        if info.loop_count > 0 {
            return false; // if we looped, we're at the EOF. !!! FIXME: do _all_ formats loop, or should we honor this?
        }

        let _ = stream.put_data(xmp::control::frame_buffer(&self.ctx, &info));

        true // had more data to decode.
    }

    /// Translation of `XMP_seek()`.
    fn seek(&mut self, frame: u64) -> Result<()> {
        let mut ms = frames_to_ms(self.freq, frame as i64).unwrap_or(-1);
        if ms < 0 {
            ms = 0;
        }
        let err = xmp::xmp_seek_time(&mut self.ctx, ms as i32);
        xmp::xmp_play_buffer(&mut self.ctx, None, 0, 0); // reset the internal state.
        if err < 0 {
            Err(set_lib_xmp_error("xmp_seek_time", err))
        } else {
            Ok(())
        }
    }

    /// Translation of `XMP_jump_to_order()`.
    fn jump_to_order(&mut self, order: i32) -> Result<()> {
        let err = xmp::xmp_set_position(&mut self.ctx, order);
        if err == -xmp::XMP_ERROR_STATE || err == -xmp::XMP_ERROR_INVALID {
            return Err(set_lib_xmp_error("xmp_set_position", err));
        }
        Ok(())
    }
}

impl Drop for XmpTrackData<'_> {
    /// Translation of `XMP_quit_track()`.
    fn drop(&mut self) {
        xmp::xmp_stop_module(&mut self.ctx);
        xmp::xmp_end_player(&mut self.ctx);
        xmp::xmp_release_module(&mut self.ctx);
        xmp::control::xmp_free_context_data(&mut self.ctx); // (and the context's Drop)
    }
}

/// Translation of `MIX_Decoder_XMP` (`XMP_init()` and `XMP_quit()` load
/// and unload libxmp, which is built in here).
pub(crate) static DECODER: Decoder = Decoder {
    name: "XMP",
    init: None,
    init_audio: xmp_init_audio,
    has_jump_to_order: true,
    quit: None,
};
