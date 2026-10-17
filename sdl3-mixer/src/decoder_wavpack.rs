// Rust translation of src/decoder_wavpack.c from SDL_mixer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! WavPack decoder, using libwavpack (see the `wavpack` module), with
//! correction (.wvc) files for hybrid lossless audio.
//!
//! As in upstream's build, `MUSIC_WAVPACK_DSD` isn't defined (the CMake
//! option is `SDLMIXER_WAVPACK_DSD`, which defines `DECODER_WAVPACK_DSD`
//! instead), so the DSD decimation code is left out and files are opened
//! without `OPEN_DSD_AS_PCM`.

// This file supports Wavpack audio streams using libwavpack.

use std::sync::{Arc, Mutex};

use sdl3::audio::{AudioFormat, AudioSpec, AudioStream};
use sdl3::error::{Error, Result};
use sdl3::io::{IoStream, IoWhence};
use sdl3::properties::Properties;

use crate::internal::{
    borrow_io, io_size, try_alloc, AudioData, Decoder, PrecacheIo, TrackData,
    PROP_AUDIO_LOAD_PATH_STRING, PROP_DECODER_WAVPACK_WVC_IOSTREAM_POINTER,
    PROP_DECODER_WAVPACK_WVC_PATH_STRING,
};
use crate::wavpack::common_utils::{
    wavpack_get_bytes_per_sample, wavpack_get_error_message, wavpack_get_mode,
    wavpack_get_num_channels, wavpack_get_num_samples64, wavpack_get_sample_rate,
};
use crate::wavpack::open_utils::wavpack_open_file_input_ex64;
use crate::wavpack::unpack_seek::wavpack_seek_sample64;
use crate::wavpack::unpack_utils::wavpack_unpack_samples;
use crate::wavpack::{
    WavpackContext, WavpackStreamReader64, MODE_FLOAT, OPEN_NORMALIZE, OPEN_TAGS, SEEK_CUR,
    SEEK_END, SEEK_SET,
};

const DECODE_FRAMES: usize = 4096;

// i/o callbacks ...

/// SDL_mixer's `WAVPACK_IoReader64` callbacks, on a stream.
struct WavpackIo<'a>(IoStream<'a>);

impl WavpackStreamReader64 for WavpackIo<'_> {
    /// Translation of `WAVPACK_IoReadBytes()`.
    fn read_bytes(&mut self, data: &mut [u8]) -> i32 {
        self.0.read(data) as i32
    }

    /// Translation of `WAVPACK_IoGetPos64()`.
    fn get_pos(&mut self) -> i64 {
        self.0.tell().unwrap_or(-1)
    }

    /// Translation of `WAVPACK_IoSetPosAbs64()`.
    fn set_pos_abs(&mut self, pos: i64) -> i32 {
        if self.0.seek(pos, IoWhence::Set).is_err() {
            -1
        } else {
            0
        }
    }

    /// Translation of `WAVPACK_IoSetPosRel64()`.
    fn set_pos_rel(&mut self, delta: i64, mode: i32) -> i32 {
        let whence = match mode {
            // just in case SDL_IO doesn't match stdio..
            SEEK_SET => IoWhence::Set,
            SEEK_CUR => IoWhence::Cur,
            SEEK_END => IoWhence::End,
            _ => return -1,
        };
        if self.0.seek(delta, whence).is_err() {
            -1
        } else {
            0
        }
    }

    /// Translation of `WAVPACK_IoPushbackByte()`.
    fn push_back_byte(&mut self, _c: i32) -> i32 {
        // libwavpack calls ungetc(), but doesn't really modify buffer.
        if self.0.seek(-1, IoWhence::Cur).is_err() {
            -1
        } else {
            0
        }
    }

    /// Translation of `WAVPACK_IoGetLength64()`.
    fn get_length(&mut self) -> i64 {
        io_size(&mut self.0)
    }

    /// Translation of `WAVPACK_IoCanSeek()`.
    fn can_seek(&mut self) -> i32 {
        if self.0.seek(0, IoWhence::Cur).is_err() {
            0
        } else {
            1
        }
    }
}

const FLAGS_DSD: i32 = 0;

/// Translation of `WAVPACK_AudioData`.
struct WavpackAudioData {
    wvcdata: Option<Arc<Vec<u8>>>,
    #[allow(dead_code)]
    numsamples: i64,
    #[allow(dead_code)]
    samplerate: u32,
    bps: i32,
    channels: i32,
    #[allow(dead_code)]
    mode: i32,
    decimation: i32,
    format: AudioFormat,
}

/// Translation of `WAVPACK_TrackData` (the stream and the correction
/// data's stream are the context's readers).
struct WavpackTrackData<'a> {
    adata: Arc<WavpackAudioData>,
    ctx: WavpackContext<'a>,
    decode_buffer: Vec<i32>,
    /// The samples in the audio format, for `SDL_PutAudioStreamData()`
    /// (upstream converts them in `decode_buffer`).
    bytes: Vec<u8>,
}

/// Open a context over `io` and the correction data `wvcdata`, as both of
/// SDL_mixer's entry points do (with `WavpackOpenFileInputEx64()`).
fn open_context<'a>(
    io: IoStream<'a>,
    wvcdata: Option<&Arc<Vec<u8>>>,
) -> Result<WavpackContext<'a>> {
    let wvcio = wvcdata.map(|data| {
        Box::new(WavpackIo(PrecacheIo::open(data.clone())))
            as Box<dyn WavpackStreamReader64 + Send + 'a>
    });
    wavpack_open_file_input_ex64(
        Box::new(WavpackIo(io)),
        wvcio,
        OPEN_NORMALIZE | OPEN_TAGS | FLAGS_DSD,
        0,
    )
    .map_err(Error::new)
}

/// Translation of `WAVPACK_init_audio()`.
fn wavpack_init_audio(
    io: Option<&mut IoStream<'_>>,
    spec: &mut AudioSpec,
    props: &Properties,
    duration_frames: &mut i64,
) -> Result<Arc<dyn AudioData>> {
    let io = io.ok_or_else(|| Error::invalid_param("io"))?;

    // Try to load a correction file if we can. This turns a lossy file into a lossless one.
    //  If the app provided an IOStream for it, use it. If not, see if they provided a path. If not, see if we can load a ".wvc" version of the original path.
    // (Here the app's stream is read from where it is and never closed, as
    // the caller owns it.)
    let app_wvcio =
        props.get_any::<Mutex<IoStream<'static>>>(PROP_DECODER_WAVPACK_WVC_IOSTREAM_POINTER);
    let mut wvcio: Option<IoStream<'static>> = None;
    if app_wvcio.is_none() {
        // Try to load a correction file if available.
        let mut wvcpath = props.get_string(PROP_DECODER_WAVPACK_WVC_PATH_STRING);
        if wvcpath.is_none() {
            if let Some(origpath) = props.get_string(PROP_AUDIO_LOAD_PATH_STRING) {
                let s = format!("{origpath}c");
                let slen = s.len();
                if (slen > 4) && s.as_bytes()[slen - 4..].eq_ignore_ascii_case(b".wvc") {
                    // only accept it if the original file ended in ".wv" (so `str` is ".wvc").
                    wvcpath = Some(s);
                }
            }
        }
        if let Some(wvcpath) = wvcpath {
            wvcio = IoStream::from_file(&wvcpath, "rb").ok();
        }
    }

    let mut err = [0u8; 4];
    if io.read(&mut err) == 0 {
        return Err(Error::new("Not a WavPack file.")); // (upstream sets no error of its own here.)
    } else if &err != b"wvpk" {
        // just a quick check before we load the whole thing into RAM.
        return Err(Error::new("Not a WavPack file."));
    }

    // Go back and do a proper load now to get metadata.
    io.seek(0, IoWhence::Set)?;

    // !!! FIXME: it would be better to not pull this all in upfront; move the thing where we decide how to access the data to a common place, make an IOStream per-track.
    let mut wvcdata: Option<Arc<Vec<u8>>> = None;
    if let Some(app_wvcio) = app_wvcio {
        let mut wvcio = app_wvcio.lock().unwrap_or_else(|e| e.into_inner());
        wvcdata = Some(Arc::new(wvcio.load_all()?));
    } else if let Some(mut wvcio) = wvcio {
        wvcdata = Some(Arc::new(wvcio.load_all()?));
    }

    // now open the memory buffers for serious processing.
    let ctx = open_context(borrow_io(io), wvcdata.as_ref())?;

    let numsamples = wavpack_get_num_samples64(&ctx);
    let bps = wavpack_get_bytes_per_sample(&ctx) << 3;
    let mode = wavpack_get_mode(&ctx);
    let channels = wavpack_get_num_channels(&ctx);
    let samplerate = wavpack_get_sample_rate(&ctx);
    let decimation = 1;

    // library returns the samples in 8, 16, 24, or 32 bit depth, but
    // always in an int32_t[] buffer, in signed host-endian format.
    let format = match bps {
        8 => AudioFormat::S8,
        16 => AudioFormat::S16,
        24 => AudioFormat::S32,
        32 => {
            if (mode & MODE_FLOAT) != 0 {
                AudioFormat::F32
            } else {
                AudioFormat::S32
            }
        }
        _ => return Err(Error::new("Unsupported WavPack bitdepth")), // uhoh.
    };

    spec.format = format;
    spec.freq = (samplerate as i32) / decimation;
    spec.channels = channels;

    // WavPack files use other tag standards; their docs seem to favor APEv2 tags, but apparently
    // ID3 can be used as well. Since we handle those elsewhere, I've removed the WavpackGetTagItem
    // calls from in here.

    drop(ctx);

    *duration_frames = numsamples;

    Ok(Arc::new(WavpackAudioData {
        wvcdata,
        numsamples,
        samplerate,
        bps,
        channels,
        mode,
        decimation,
        format,
    }))
}

impl AudioData for WavpackAudioData {
    /// Translation of `WAVPACK_init_track()`.
    fn init_track<'a>(
        self: Arc<Self>,
        io: Option<IoStream<'a>>,
        spec: &AudioSpec,
        _props: &Properties,
    ) -> Result<Box<dyn TrackData + 'a>> {
        let io = io.ok_or_else(|| Error::invalid_param("io"))?;

        let frames = DECODE_FRAMES * spec.channels.max(0) as usize * self.decimation as usize;
        let decode_buffer = try_alloc::<i32>(frames)?;
        let bytes = try_alloc::<u8>(frames * 4)?;

        // now open the memory buffers for serious processing.
        // (upstream fails without an error of its own here.)
        let ctx = open_context(io, self.wvcdata.as_ref())?;

        Ok(Box::new(WavpackTrackData {
            adata: self,
            ctx,
            decode_buffer,
            bytes,
        }))
    }
}

impl TrackData for WavpackTrackData<'_> {
    /// Translation of `WAVPACK_decode()`.
    fn decode(&mut self, stream: &AudioStream) -> bool {
        let adata = &self.adata;

        let mut amount = wavpack_unpack_samples(
            &mut self.ctx,
            &mut self.decode_buffer,
            (DECODE_FRAMES as i32 * adata.decimation) as u32,
        ) as usize;

        if amount == 0 {
            return false; // EOF.
        }

        amount *= adata.channels as usize; // move from sample frames to samples.
        let src = &self.decode_buffer[..amount];

        // library returns the samples in 8, 16, 24, or 32 bit depth, but
        // always in an int32_t[] buffer, in signed host-endian format.
        let format = adata.format;
        let bytes = if format == AudioFormat::S8 {
            for (dst, &s) in self.bytes.iter_mut().zip(src) {
                *dst = s as i8 as u8; // data is 8-bit audio in an int32 array, shrink out unused bits in-place.
            }
            &self.bytes[..amount]
        } else if format == AudioFormat::S16 {
            for (dst, &s) in self.bytes.chunks_exact_mut(2).zip(src) {
                dst.copy_from_slice(&(s as i16).to_ne_bytes()); // data is 16-bit audio in an int32 array, shrink out unused bits in-place.
            }
            &self.bytes[..amount * 2]
        } else if adata.bps == 24 {
            debug_assert!(format == AudioFormat::S32);
            for (dst, &s) in self.bytes.chunks_exact_mut(4).zip(src) {
                dst.copy_from_slice(&(s << 8).to_ne_bytes()); // data is 24-bit audio in an int32 array, slide bits over so most significant bits scale up to full 32-bit range.
            }
            &self.bytes[..amount * 4]
        } else {
            debug_assert!((format == AudioFormat::F32) || (format == AudioFormat::S32)); // these just copy through as-is.
            for (dst, &s) in self.bytes.chunks_exact_mut(4).zip(src) {
                dst.copy_from_slice(&s.to_ne_bytes());
            }
            &self.bytes[..amount * 4]
        };

        let _ = stream.put_data(bytes);
        true
    }

    /// Translation of `WAVPACK_seek()`.
    fn seek(&mut self, frame: u64) -> Result<()> {
        let success = wavpack_seek_sample64(&mut self.ctx, frame as i64);
        if success == 0 {
            return Err(Error::new(wavpack_get_error_message(&self.ctx).to_string()));
        }

        Ok(())
    }
}

/// Translation of `MIX_Decoder_WAVPACK`.
pub(crate) static DECODER: Decoder = Decoder {
    name: "WAVPACK",
    init: None,
    init_audio: wavpack_init_audio,
    has_jump_to_order: false,
    quit: None,
};
