// Rust translation of src/SDL_mixer_internal.h from SDL_mixer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The decoder interface, the I/O clamp, and the other pieces the mixer
//! and its decoders share.
//!
//! The SIMD selection at the top of the header (`SDL_MIXER_NEED_SCALAR_FALLBACK`,
//! `MIX_HasSSE`, `MIX_HasNEON`) only picks between implementations of the
//! 3D math in `spatialization.rs`, which computes the SSE results in
//! portable code (see there). `SDL_mixer_loader.h`, which loads external
//! decoder libraries, isn't needed: every decoder here is translated.

use std::sync::Arc;

use sdl3::audio::{AudioSpec, AudioStream};
use sdl3::error::{Error, Result};
use sdl3::io::{IoInterface, IoStatus, IoStop, IoStream, IoWhence};
use sdl3::properties::Properties;

// these are not (currently) available in the public API, and may change names or functionality, or be removed.
#[allow(dead_code)]
pub(crate) const PROP_DECODER_NAME_STRING: &str = "SDL_mixer.decoder.name";
pub(crate) const PROP_DECODER_FORMAT_NUMBER: &str = "SDL_mixer.decoder.format";
pub(crate) const PROP_DECODER_CHANNELS_NUMBER: &str = "SDL_mixer.decoder.channels";
pub(crate) const PROP_DECODER_FREQ_NUMBER: &str = "SDL_mixer.decoder.freq";
pub(crate) const PROP_DECODER_SINEWAVE_HZ_NUMBER: &str = "SDL_mixer.decoder.sinewave.hz";
pub(crate) const PROP_DECODER_SINEWAVE_AMPLITUDE_FLOAT: &str =
    "SDL_mixer.decoder.sinewave.amplitude";
pub(crate) const PROP_DECODER_SINEWAVE_MS_NUMBER: &str = "SDL_mixer.decoder.sinewave.ms";
pub(crate) const PROP_AUDIO_LOAD_PATH_STRING: &str = "SDL_mixer.audio.load.path";
pub(crate) const PROP_AUDIO_LOAD_ONDEMAND_BOOLEAN: &str = "SDL_mixer.audio.load.ondemand";

// Clamp an IOStream to a subset of its available data...this is used to cut ID3 (etc) tags off
//  both ends of an audio file, making it look like the file just doesn't have those bytes.

/// Translation of `MIX_IoClamp`, with the `SDL_IOStreamInterface` that
/// `MIX_OpenIoClamp` opens over it (wrap it in [`IoStream::open`] for
/// that). The metadata tag parser reads through it directly, since it moves
/// `start` and `length` while it reads.
pub(crate) struct IoClamp<'a> {
    pub io: IoStream<'a>,
    pub start: i64,
    pub length: i64,
    pub pos: i64,
    /// The last error a seek here failed with: upstream's callers that fail
    /// without setting an error of their own report whatever error was set
    /// last (`SDL_GetError()`), which is this one.
    pub last_error: Option<Error>,
}

impl<'a> IoClamp<'a> {
    /// Translation of `MIX_OpenIoClamp()`.
    pub fn open(mut io: IoStream<'a>) -> Result<IoClamp<'a>> {
        /* Don't use SDL_GetIOSize() here -- see SDL bug #4026 */
        let start = io.tell().unwrap_or(-1);
        let length = io.seek(0, IoWhence::End).unwrap_or(-1) - start;
        if start < 0 || length < 0 || io.seek(start, IoWhence::Set).is_err() {
            return Err(Error::new("Error seeking in datastream"));
        }
        Ok(IoClamp {
            io,
            start,
            length,
            pos: 0,
            last_error: None,
        })
    }

    /// Translation of `MIX_IoClamp_size()`.
    pub fn clamp_size(&self) -> i64 {
        self.length
    }

    /// Translation of `MIX_IoClamp_seek()`.
    pub fn clamp_seek(&mut self, mut offset: i64, whence: IoWhence) -> Result<i64> {
        if whence == IoWhence::Cur {
            offset += self.pos;
        } else if whence == IoWhence::End {
            offset += self.length;
        }

        if offset < 0 {
            let e = Error::new("Seek before start of data");
            self.last_error = Some(e.clone());
            return Err(e);
        } else if offset > self.length {
            offset = self.length;
        }

        if self.pos != offset {
            if let Err(e) = self.io.seek(self.start + offset, IoWhence::Set) {
                self.last_error = Some(e.clone());
                return Err(e);
            }
            self.pos = offset;
        }

        Ok(offset)
    }

    /// Translation of `MIX_IoClamp_read()`.
    pub fn clamp_read(&mut self, buf: &mut [u8]) -> std::result::Result<usize, IoStop> {
        let size = buf.len();
        // (a clamp that was moved past its end has a negative size left,
        // which, as a size_t, doesn't limit the read.)
        let remaining = (self.length - self.pos) as u64 as usize;
        let ret = self.io.read(&mut buf[..size.min(remaining)]);
        self.pos += ret as i64;
        if ret < size {
            let status = if ret == remaining {
                IoStatus::Eof
            } else {
                self.io.status()
            };
            if status != IoStatus::Ready {
                return Err(IoStop {
                    bytes: ret,
                    status,
                    error: self.io.last_error().cloned(),
                });
            }
        }
        Ok(ret)
    }

    /// `SDL_ReadIO()` on the clamp's stream.
    pub fn read_io(&mut self, buf: &mut [u8]) -> usize {
        if buf.is_empty() {
            return 0;
        }
        match self.clamp_read(buf) {
            Ok(n) => n,
            Err(stop) => stop.bytes,
        }
    }

    /// `SDL_SeekIO()` on the clamp's stream (-1 on failure).
    pub fn seek_io(&mut self, offset: i64, whence: IoWhence) -> i64 {
        self.clamp_seek(offset, whence).unwrap_or(-1)
    }

    /// `SDL_TellIO()` on the clamp's stream.
    pub fn tell_io(&mut self) -> i64 {
        self.seek_io(0, IoWhence::Cur)
    }
}

impl IoInterface for IoClamp<'_> {
    fn has_size(&self) -> bool {
        true
    }
    fn size(&mut self) -> Result<i64> {
        Ok(self.clamp_size())
    }
    fn seek(&mut self, offset: i64, whence: IoWhence) -> Result<i64> {
        self.clamp_seek(offset, whence)
    }
    fn can_read(&self) -> bool {
        true
    }
    fn read(&mut self, buf: &mut [u8]) -> std::result::Result<usize, IoStop> {
        self.clamp_read(buf)
    }
}

/// A stream over a borrowed one, so code that owns its stream (a decoder's
/// track data) can work on the caller's. Closing it leaves the borrowed
/// stream open, as upstream's callers that pass `closeio=false` expect.
struct BorrowedIo<'a, 'b>(&'a mut IoStream<'b>);

impl IoInterface for BorrowedIo<'_, '_> {
    fn has_size(&self) -> bool {
        true
    }
    fn size(&mut self) -> Result<i64> {
        self.0.size()
    }
    fn seek(&mut self, offset: i64, whence: IoWhence) -> Result<i64> {
        self.0.seek(offset, whence)
    }
    fn can_read(&self) -> bool {
        true
    }
    fn read(&mut self, buf: &mut [u8]) -> std::result::Result<usize, IoStop> {
        let n = self.0.read(buf);
        let status = self.0.status();
        if n < buf.len() && status != IoStatus::Ready {
            return Err(IoStop {
                bytes: n,
                status,
                error: self.0.last_error().cloned(),
            });
        }
        Ok(n)
    }
}

/// Use a borrowed stream where an owned one is needed.
pub(crate) fn borrow_io<'a>(io: &'a mut IoStream<'_>) -> IoStream<'a> {
    IoStream::open(BorrowedIo(io))
}

/// Audio data shared between tracks: what a [`MIX_Audio`](crate::Audio)
/// keeps in RAM (`precache`). It might be the app's own memory, from
/// [`Audio::load_no_copy`](crate::Audio::load_no_copy).
pub(crate) type Precache = Arc<dyn AsRef<[u8]> + Send + Sync>;

/// `SDL_IOFromConstMem()` over a [`Precache`], so the stream can outlive
/// the call that made it (as a track's input does).
pub(crate) struct PrecacheIo {
    data: Precache,
    here: usize,
}

impl PrecacheIo {
    pub fn open(data: Precache) -> IoStream<'static> {
        IoStream::open(PrecacheIo { data, here: 0 })
    }
}

impl IoInterface for PrecacheIo {
    fn has_size(&self) -> bool {
        true
    }
    fn size(&mut self) -> Result<i64> {
        Ok((*self.data).as_ref().len() as i64)
    }
    /// Translation of `mem_seek()`.
    fn seek(&mut self, offset: i64, whence: IoWhence) -> Result<i64> {
        let stop = (*self.data).as_ref().len() as i64;
        let base = match whence {
            IoWhence::Set => 0i64,
            IoWhence::Cur => self.here as i64,
            IoWhence::End => stop,
        };
        self.here = base.saturating_add(offset).clamp(0, stop) as usize;
        Ok(self.here as i64)
    }
    fn can_read(&self) -> bool {
        true
    }
    /// Translation of `mem_read()`.
    fn read(&mut self, buf: &mut [u8]) -> std::result::Result<usize, IoStop> {
        let mem = (*self.data).as_ref();
        let n = buf.len().min(mem.len() - self.here);
        buf[..n].copy_from_slice(&mem[self.here..self.here + n]);
        self.here += n;
        if n < buf.len() && self.here == mem.len() {
            return Err(IoStop::status(n, IoStatus::Eof));
        }
        Ok(n)
    }
}

/// What a decoder's `init_audio` finds out about the audio: its static
/// userdata, shared by all the tracks that play it. Its `Drop` is the
/// decoder's `quit_audio`.
pub(crate) trait AudioData: Send + Sync {
    /// Init decoder instance data for a single track (`init_track`). The
    /// track owns `io` from now on (closing it is `quit_track`'s job,
    /// which is the returned value's `Drop`).
    fn init_track<'a>(
        self: Arc<Self>,
        io: Option<IoStream<'a>>,
        spec: &AudioSpec,
        props: &Properties,
    ) -> Result<Box<dyn TrackData + 'a>>;
}

/// A decoder instance for a single track.
pub(crate) trait TrackData: Send {
    /// Decode a little more into `stream`; `false` at the end of the data
    /// (or on failure).
    fn decode(&mut self, stream: &AudioStream) -> bool;
    /// Seek to a sample frame.
    fn seek(&mut self, frame: u64) -> Result<()>;
    /// Jump to an order in a module (none of the decoders translated here
    /// support it; see [`Decoder::has_jump_to_order`]).
    fn jump_to_order(&mut self, _order: i32) -> Result<()> {
        Err(Error::unsupported())
    }
}

/// A decoder's `init_audio`: see if it's a supported format, init spec,
/// set metadata in props, allocate static userdata and payload.
pub(crate) type InitAudioFn = fn(
    io: Option<&mut IoStream<'_>>,
    spec: &mut AudioSpec,
    props: &Properties,
    duration_frames: &mut i64,
) -> Result<Arc<dyn AudioData>>;

/// Translation of `MIX_Decoder`. The `init_track`, `decode`, `seek`,
/// `jump_to_order`, `quit_track` and `quit_audio` entry points are the
/// methods and `Drop`s of [`AudioData`] and [`TrackData`].
pub(crate) struct Decoder {
    pub name: &'static str,
    /// initialize the decoder (load external libraries, etc).
    pub init: Option<fn() -> bool>,
    pub init_audio: InitAudioFn,
    /// `jump_to_order != NULL`
    pub has_jump_to_order: bool,
    /// deinitialize the decoder (unload external libraries, etc).
    pub quit: Option<fn()>,
}

// Various Ogg-based decoders use this (Vorbis, FLAC, Opus, etc).
/// Translation of `MIX_OggLoop`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct OggLoop {
    pub start: i64,
    pub end: i64,
    pub len: i64,
    pub count: i64,
    pub active: bool,
}

/// `SDL_ReadIO()` until `buf` is full or the stream stops; the number of
/// bytes read (upstream's single read call does the same on the streams
/// SDL provides).
pub(crate) fn read_io(io: &mut IoStream<'_>, buf: &mut [u8]) -> usize {
    io.read(buf)
}

/// `SDL_TellIO()`, -1 on failure.
pub(crate) fn tell_io(io: &mut IoStream<'_>) -> i64 {
    io.tell().unwrap_or(-1)
}

/// `SDL_SeekIO()`, -1 on failure.
pub(crate) fn seek_io(io: &mut IoStream<'_>, offset: i64, whence: IoWhence) -> i64 {
    io.seek(offset, whence).unwrap_or(-1)
}

/// `SDL_GetIOSize()`, -1 on failure.
pub(crate) fn io_size(io: &mut IoStream<'_>) -> i64 {
    io.size().unwrap_or(-1)
}

/// Allocate a zeroed buffer whose size came from a file, failing instead
/// of aborting when it can't be had (upstream's `SDL_malloc` returning
/// `NULL`).
pub(crate) fn try_alloc<T: Clone + Default>(len: usize) -> Result<Vec<T>> {
    let mut v = Vec::new();
    v.try_reserve_exact(len)
        .map_err(|_| Error::out_of_memory())?;
    v.resize(len, T::default());
    Ok(v)
}

/// Push interleaved float samples to a stream (`SDL_PutAudioStreamData()`
/// with a `float *`).
pub(crate) fn put_f32(stream: &AudioStream, samples: &[f32]) -> Result<()> {
    let mut bytes = Vec::with_capacity(samples.len() * 4);
    for s in samples {
        bytes.extend_from_slice(&s.to_ne_bytes());
    }
    stream.put_data(&bytes)
}
