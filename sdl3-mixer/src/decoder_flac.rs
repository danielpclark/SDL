// Rust translation of src/decoder_flac.c from SDL_mixer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! FLAC decoder, using libFLAC's stream decoder (the `flac` module,
//! translated here). Its callbacks are the [`StreamDecoderClient`]s
//! below: [`FlacProbe`] while `FLAC_init_audio()` reads the metadata and
//! [`FlacClient`] for a track (C's `FLAC_TrackData` is its `client_data`
//! for both). libFLAC is linked in, so the dynamic loading
//! (`FLAC_DYNAMIC`, `LoadModule_flac()`) isn't translated.

use std::sync::Arc;

use sdl3::audio::{AudioFormat, AudioSpec, AudioStream};
use sdl3::error::{Error, Result};
use sdl3::io::{IoStatus, IoStream, IoWhence};
use sdl3::properties::Properties;

use crate::flac::*;
use crate::internal::{borrow_io, io_size, AudioData, Decoder, OggLoop, TrackData};
use crate::metadata_tags::parse_ogg_comments;
use crate::{DURATION_INFINITE, PROP_AUDIO_LOAD_IGNORE_LOOPS_BOOLEAN};

/// Translation of `FLAC_AudioData` (`props` is only used while
/// `FLAC_init_audio()` runs, so it is [`FlacProbe`]'s).
struct FlacAudioData {
    is_ogg_stream: bool,
    loop_: OggLoop,
}

/// Translation of `FLAC_TrackData` for a track. `stream` is the stream
/// the last `FLAC_decode()` got (`tdata->stream`), which the write
/// callback puts the audio into, also when a seek decodes; `None` before
/// the first one (C's NULL).
struct FlacClient<'a, 's> {
    t: &'a mut FlacTrack<'s>,
    stream: Option<&'a AudioStream>,
}

/// The rest of `FLAC_TrackData` for a track.
struct FlacTrack<'s> {
    adata: Arc<FlacAudioData>,
    io: IoStream<'s>,
    spec: AudioSpec,
    bits_per_sample: i32,
    cvtbuf: Vec<u8>,
    current_iteration: i64,
    current_iteration_frames: i64,
    /// (whether `FLAC_decode()` has set `tdata->stream` yet)
    has_stream: bool,
    /// (whether `FLAC_IoWrite()` is in its looping seek; see the FIXME
    /// there)
    in_loop_seek: bool,
}

/// Translation of `FLAC_TrackData` while `FLAC_init_audio()` reads the
/// metadata.
struct FlacProbe<'s, 'p> {
    io: IoStream<'s>,
    spec: AudioSpec,
    bits_per_sample: i32,
    /// (`adata->props`)
    props: &'p Properties,
    /// (`adata->loop`)
    loop_: OggLoop,
}

/// The track data the mixer holds: the decoder and its client data.
struct FlacTrackData<'s> {
    decoder: Box<StreamDecoder>,
    t: FlacTrack<'s>,
}

/// Translation of `FLAC_IoRead()`.
fn flac_io_read(
    io: &mut IoStream<'_>,
    buffer: &mut [u8],
    bytes: &mut usize,
) -> StreamDecoderReadStatus {
    if *bytes > 0 {
        *bytes = io.read(&mut buffer[..*bytes]);
        let status = io.status();
        if status == IoStatus::Ready || status == IoStatus::NotReady {
            return StreamDecoderReadStatus::Continue;
        } else if status == IoStatus::Eof {
            return StreamDecoderReadStatus::EndOfStream;
        }
    }
    StreamDecoderReadStatus::Abort
}

/// Translation of `FLAC_IoSeek()`.
fn flac_io_seek(io: &mut IoStream<'_>, absolute_byte_offset: u64) -> StreamDecoderSeekStatus {
    if io.seek(absolute_byte_offset as i64, IoWhence::Set).is_err() {
        return StreamDecoderSeekStatus::Error;
    }
    StreamDecoderSeekStatus::Ok
}

/// Translation of `FLAC_IoTell()`.
fn flac_io_tell(io: &mut IoStream<'_>, absolute_byte_offset: &mut u64) -> StreamDecoderTellStatus {
    let pos = io.tell().unwrap_or(-1);
    if pos < 0 {
        return StreamDecoderTellStatus::Error;
    }
    *absolute_byte_offset = pos as u64;
    StreamDecoderTellStatus::Ok
}

/// Translation of `FLAC_IoLength()`.
fn flac_io_length(io: &mut IoStream<'_>, stream_length: &mut u64) -> StreamDecoderLengthStatus {
    let iolen = io_size(io);
    if iolen < 0 {
        return StreamDecoderLengthStatus::Error;
    }
    *stream_length = iolen as u64;
    StreamDecoderLengthStatus::Ok
}

/// Translation of `FLAC_IoEOF()`.
fn flac_io_eof(io: &mut IoStream<'_>) -> bool {
    io.tell().unwrap_or(-1) >= io_size(io)
}

/// `bits_per_sample`'s format: decoded FLAC data is always int, from 4 to
/// 32 bits, apparently.
fn flac_format(spec: &mut AudioSpec, bits_per_sample: i32) {
    if bits_per_sample <= 8 {
        spec.format = AudioFormat::S8;
    } else if bits_per_sample <= 16 {
        spec.format = AudioFormat::S16;
    } else if bits_per_sample <= 32 {
        spec.format = AudioFormat::S32;
    }
}

impl StreamDecoderClient for FlacProbe<'_, '_> {
    fn read(&mut self, buffer: &mut [u8], bytes: &mut usize) -> StreamDecoderReadStatus {
        flac_io_read(&mut self.io, buffer, bytes)
    }

    fn seek(&mut self, absolute_byte_offset: u64) -> StreamDecoderSeekStatus {
        flac_io_seek(&mut self.io, absolute_byte_offset)
    }

    fn tell(&mut self, absolute_byte_offset: &mut u64) -> StreamDecoderTellStatus {
        flac_io_tell(&mut self.io, absolute_byte_offset)
    }

    fn length(&mut self, stream_length: &mut u64) -> StreamDecoderLengthStatus {
        flac_io_length(&mut self.io, stream_length)
    }

    fn eof(&mut self) -> bool {
        flac_io_eof(&mut self.io)
    }

    /// Translation of `FLAC_IoWriteNoOp()`.
    fn write(
        &mut self,
        _decoder: &mut StreamDecoder,
        _frame: &FrameHeader,
        _offset: usize,
    ) -> StreamDecoderWriteStatus {
        StreamDecoderWriteStatus::Continue // we don't need this data at this moment.
    }

    /// Translation of `FLAC_IoMetadata()`.
    fn metadata(&mut self, metadata: &StreamMetadata<'_>) {
        match metadata.data {
            StreamMetadataData::StreamInfo(stream_info) => {
                self.spec.freq = stream_info.sample_rate as i32;
                self.spec.channels = stream_info.channels as i32; // (if we need the 3-channel map magic, it'll notice spec.channels is wrong when we get to FLAC_IoWrite and set it up.)
                self.bits_per_sample = stream_info.bits_per_sample as i32;

                // decoded FLAC data is always int, from 4 to 32 bits, apparently.
                flac_format(&mut self.spec, self.bits_per_sample);
            }
            StreamMetadataData::VorbisComment(vc) => {
                let num_comments = vc.num_comments as i32;
                // FIXME (upstream): a comment of no bytes has a NULL entry,
                // which MIX_ParseOggComments() passes to SDL_strdup(), and
                // crashes; it is an empty comment here (and in the patched
                // C reference harness).
                let comments: Vec<&[u8]> = (0..num_comments.max(0) as usize)
                    .map(|i| {
                        vc.comments
                            .as_ref()
                            .and_then(|c| c[i].entry.as_deref())
                            .unwrap_or(b"")
                    })
                    .collect();
                parse_ogg_comments(
                    self.props,
                    self.spec.freq,
                    vc.vendor_string.entry.as_deref(),
                    &comments,
                    &mut self.loop_,
                );
            }
            _ => {}
        }
    }

    /// Translation of `FLAC_IoError()`.
    fn error(&mut self, _status: StreamDecoderErrorStatus) {
        // (currently) don't care.
    }
}

impl StreamDecoderClient for FlacClient<'_, '_> {
    fn read(&mut self, buffer: &mut [u8], bytes: &mut usize) -> StreamDecoderReadStatus {
        flac_io_read(&mut self.t.io, buffer, bytes)
    }

    fn seek(&mut self, absolute_byte_offset: u64) -> StreamDecoderSeekStatus {
        flac_io_seek(&mut self.t.io, absolute_byte_offset)
    }

    fn tell(&mut self, absolute_byte_offset: &mut u64) -> StreamDecoderTellStatus {
        flac_io_tell(&mut self.t.io, absolute_byte_offset)
    }

    fn length(&mut self, stream_length: &mut u64) -> StreamDecoderLengthStatus {
        flac_io_length(&mut self.t.io, stream_length)
    }

    fn eof(&mut self) -> bool {
        flac_io_eof(&mut self.t.io)
    }

    /// Translation of `FLAC_IoWrite()`.
    fn write(
        &mut self,
        decoder: &mut StreamDecoder,
        frame: &FrameHeader,
        offset: usize,
    ) -> StreamDecoderWriteStatus {
        let stream = self.stream;
        let channels = frame.channels as i32;

        // !!! FIXME: this is kinda gross, but FLAC 3-channel is FL, FR, FC, whereas SDL is FL, FR, LFE...we don't have a "front center" channel until 5.1 output.  :/  The channel mask thing Sam wants in SDL3 would fix this.
        let sdlchannels = if channels == 3 { 6 } else { channels };

        // change the stream format if we're suddenly getting data in a different format. I assume this can happen if you chain FLAC files together.
        let t = &mut *self.t;
        if (t.spec.freq != frame.sample_rate as i32)
            || (t.spec.channels != sdlchannels)
            || (t.bits_per_sample != frame.bits_per_sample as i32)
        {
            t.bits_per_sample = frame.bits_per_sample as i32;
            t.spec.freq = frame.sample_rate as i32;
            t.spec.channels = sdlchannels;
            // decoded FLAC data is always int, from 4 to 32 bits, apparently.
            flac_format(&mut t.spec, t.bits_per_sample);
            if let Some(stream) = stream {
                let _ = stream.set_format(Some(&t.spec), None);
            }
        }

        let mut amount = frame.blocksize as usize;
        let onebuflen = t.spec.format.bytesize() as usize * amount;
        let buflen = onebuflen * channels as usize;
        if t.cvtbuf.len() < buflen {
            if t.cvtbuf.try_reserve_exact(buflen - t.cvtbuf.len()).is_err() {
                return StreamDecoderWriteStatus::Abort;
            }
            t.cvtbuf.resize(buflen, 0);
        }

        let loop_ = t.adata.loop_;
        if t.current_iteration < 0
            && loop_.active
            && (t.current_iteration_frames.wrapping_add(amount as i64) >= loop_.start)
        {
            t.current_iteration = 0; // we've hit the start of the loop point.
            t.current_iteration_frames = t.current_iteration_frames.wrapping_sub(loop_.start);
            // so adding `amount` corrects this later.
        }

        if t.current_iteration >= 0 {
            debug_assert!(loop_.active);
            // FIXME (upstream): SDL_assert(current_iteration_frames <= loop->len)
            // here and below fails on damaged files (where a looping seek
            // doesn't land where it asked), so it's left out.
            let available = loop_.len.wrapping_sub(t.current_iteration_frames);
            if amount as i64 > available {
                // (available is never negative; if it were, C's size_t
                // would put far more than the frame.)
                amount = available.max(0) as usize;
            }

            // FIXME (upstream): the looping seek below decodes the loop's
            // first frame, and libFLAC hands it to this callback before the
            // seek returns, with current_iteration_frames not yet reset; so
            // that call loops again, and so on: an infinite loop (or one
            // with many iterations) recurses until the stack overflows.
            // Here (and in the patched C reference harness) the callback
            // that comes from the looping seek doesn't loop again. Also,
            // `amount` frames are then put from the decoder's output, which
            // that seek has decoded the loop's first frame into, rather than
            // this frame's last ones.
            if !t.in_loop_seek
                && (t.current_iteration_frames.wrapping_add(amount as i64) >= loop_.len)
            {
                // time to loop?
                let mut should_loop = false;
                if loop_.count < 0 {
                    // negative==infinite loop
                    t.current_iteration = 0;
                    should_loop = true;
                } else {
                    t.current_iteration += 1;
                    if t.current_iteration < loop_.count {
                        should_loop = true;
                    }
                }

                if should_loop {
                    let nextframe = (loop_.start as u64)
                        .wrapping_add((loop_.len as u64).wrapping_mul(t.current_iteration as u64));
                    t.in_loop_seek = true;
                    let ok = flac_seek(decoder, self, nextframe).is_ok();
                    self.t.in_loop_seek = false;
                    if !ok {
                        // FIXME (upstream): this returns `false`, which is
                        // FLAC__STREAM_DECODER_WRITE_STATUS_CONTINUE, so
                        // decoding goes on after a failed looping seek.
                        return StreamDecoderWriteStatus::Continue;
                    }
                } else {
                    self.t.current_iteration = -1;
                }
                self.t.current_iteration_frames = 0;
            }
        }

        let t = &mut *self.t;
        if amount > 0 {
            // (If we padded out to 5.1 to get a front-center channel, the unnecessary channels happen
            //   to be at the end, so it'll assume those padded channels are silent.)
            let shift = t.spec.format.bitsize() as i32 - t.bits_per_sample;
            let bytesize = t.spec.format.bytesize() as usize;
            if !matches!(
                t.spec.format,
                AudioFormat::S8 | AudioFormat::S16 | AudioFormat::S32
            ) {
                debug_assert!(false, "Unexpected audio data type");
                return StreamDecoderWriteStatus::Abort;
            }
            let output = decoder.output();
            for (channel, out) in output.iter().enumerate().take(channels as usize) {
                let src = &out[offset..];
                let dst = &mut t.cvtbuf[channel * onebuflen..];
                for i in 0..amount {
                    let v = (src[i] as u32).wrapping_shl(shift as u32);
                    match bytesize {
                        1 => dst[i] = v as i8 as u8,
                        2 => dst[i * 2..i * 2 + 2].copy_from_slice(&(v as i16).to_ne_bytes()),
                        4 => dst[i * 4..i * 4 + 4].copy_from_slice(&(v as i32).to_ne_bytes()),
                        _ => {}
                    }
                }
            }

            if let Some(stream) = stream {
                let channel_arrays: Vec<Option<&[u8]>> = (0..channels as usize)
                    .map(|i| Some(&t.cvtbuf[i * onebuflen..i * onebuflen + bytesize * amount]))
                    .collect();
                let _ = stream.put_planar_data(&channel_arrays, amount);
            }
            t.current_iteration_frames = t.current_iteration_frames.wrapping_add(amount as i64);
        }

        StreamDecoderWriteStatus::Continue
    }

    /// Translation of `FLAC_IoMetadataNoOp()`.
    fn metadata(&mut self, _metadata: &StreamMetadata<'_>) {
        // don't care about metadata at this point.
    }

    /// Translation of `FLAC_IoError()`.
    fn error(&mut self, _status: StreamDecoderErrorStatus) {
        // (currently) don't care.
    }
}

/// Translation of `FLAC_init_audio()`.
fn flac_init_audio(
    io: Option<&mut IoStream<'_>>,
    spec: &mut AudioSpec,
    props: &Properties,
    duration_frames: &mut i64,
) -> Result<Arc<dyn AudioData>> {
    let io = io.ok_or_else(|| Error::invalid_param("io"))?;

    // just load the bare minimum from the IOStream to verify it's a FLAC file (if it's an Ogg stream, we'll let libFLAC try to parse it out).
    let mut is_ogg_stream = false;

    let mut magic = [0u8; 4];
    if io.read(&mut magic) != 4 {
        return Err(Error::new("Not a FLAC audio stream")); // (upstream sets no error here.)
    } else if &magic == b"OggS" {
        is_ogg_stream = true; // MAYBE flac, might be vorbis, etc.
    } else if &magic != b"fLaC" {
        return Err(Error::new("Not a FLAC audio stream"));
    } else if io.seek(0, IoWhence::Set).is_err() {
        // rewind, let libFLAC process through the metadata, so we know it's definitely a FLAC file and we have the props.
        return Err(Error::new("Not a FLAC audio stream")); // (upstream keeps SDL_SeekIO()'s error.)
    }

    let mut tdata = FlacProbe {
        io: borrow_io(io),
        spec: AudioSpec::default(),
        bits_per_sample: 0,
        props,
        loop_: OggLoop::default(),
    };

    let mut decoder = StreamDecoder::new();

    decoder.set_metadata_respond(FLAC__METADATA_TYPE_VORBIS_COMMENT);

    let ret = if is_ogg_stream {
        decoder.init_ogg_stream()
    } else {
        decoder.init_stream()
    };

    if ret != StreamDecoderInitStatus::Ok {
        decoder.delete();
        return Err(Error::new("FLAC__stream_decoder_init_stream() failed"));
    }

    let rc = decoder.process_until_end_of_metadata(&mut tdata);
    let total_frames = if rc {
        decoder.get_total_samples() as i64
    } else {
        -1
    };

    // Dump this stream no matter what. Either we failed, or we know it's a real FLAC and have its metadata pushed to `props`.
    decoder.finish();
    decoder.delete();

    if !rc {
        return Err(Error::new(
            "FLAC__stream_decoder_process_until_end_of_metadata() failed",
        ));
    }

    // metadata processing should have filled in several things in `adata`.

    *spec = tdata.spec;
    let mut loop_ = tdata.loop_;

    let ignore_loops = props
        .get_bool(PROP_AUDIO_LOAD_IGNORE_LOOPS_BOOLEAN)
        .unwrap_or(false);
    if ignore_loops || (loop_.end > total_frames) {
        loop_.active = false;
    }

    if loop_.active {
        *duration_frames = if loop_.count < 0 {
            DURATION_INFINITE
        } else {
            total_frames.wrapping_mul(loop_.count)
        };
    } else {
        *duration_frames = total_frames;
    }

    Ok(Arc::new(FlacAudioData {
        is_ogg_stream,
        loop_,
    }))
}

impl AudioData for FlacAudioData {
    /// Translation of `FLAC_init_track()`.
    fn init_track<'a>(
        self: Arc<Self>,
        io: Option<IoStream<'a>>,
        spec: &AudioSpec,
        _props: &Properties,
    ) -> Result<Box<dyn TrackData + 'a>> {
        let io = io.ok_or_else(|| Error::invalid_param("io"))?;

        let is_ogg_stream = self.is_ogg_stream;
        let mut t = FlacTrack {
            adata: self,
            io,
            spec: AudioSpec::default(),
            bits_per_sample: 0,
            cvtbuf: Vec::new(),
            current_iteration: -1,
            current_iteration_frames: 0,
            has_stream: false,
            in_loop_seek: false,
        };
        let mut decoder = StreamDecoder::new();

        let ret = if is_ogg_stream {
            decoder.init_ogg_stream()
        } else {
            decoder.init_stream()
        };

        if ret != StreamDecoderInitStatus::Ok {
            decoder.delete();
            return Err(Error::new("FLAC__stream_decoder_init_stream() failed"));
        }

        if !decoder.process_until_end_of_metadata(&mut FlacClient {
            t: &mut t,
            stream: None,
        }) {
            decoder.finish();
            decoder.delete();
            // FIXME (upstream): this sets the error but goes on to return
            // true with the track data it just freed; it fails here (and
            // in the patched C reference harness).
            return Err(Error::new(
                "FLAC__stream_decoder_process_until_end_of_metadata() failed",
            ));
        }

        t.spec = *spec;

        Ok(Box::new(FlacTrackData { decoder, t }))
    }
}

/// Translation of `FLAC_seek()`, with the client (`tdata`) the decoder
/// calls back.
fn flac_seek(
    decoder: &mut StreamDecoder,
    client: &mut FlacClient<'_, '_>,
    mut frame: u64,
) -> Result<()> {
    let loop_ = client.t.adata.loop_;
    let mut final_iteration: i64 = -1;
    let mut final_iteration_frames: i64 = 0;

    // frame has hit the loop point?
    if loop_.active && (frame as i64 >= loop_.start) {
        // figure out the _actual_ frame in the vorbis file we're aiming for.
        if (loop_.count < 0) || ((frame as i64) < (loop_.len.wrapping_mul(loop_.count))) {
            // literally in the loop right now.
            frame = frame.wrapping_sub(loop_.start as u64); // make logical frame index relative to start of loop.
            final_iteration = if loop_.count < 0 {
                0
            } else {
                (frame / loop_.len as u64) as i64
            }; // decide what iteration of the loop we're on (stays at zero for infinite loops).
            frame %= loop_.len as u64; // drop iterations so we're an offset into the loop.
            final_iteration_frames = frame as i64;
            frame = frame.wrapping_add(loop_.start as u64); // convert back into physical frame index.
        } else {
            // past the loop point?
            debug_assert!(loop_.count > 0); // can't be infinite loop if we passed it.
            frame = frame.wrapping_sub(loop_.len.wrapping_mul(loop_.count) as u64);
            // drop the iterations to get the physical frame index.
        }
    }

    if !decoder.seek_absolute(client, frame) {
        if decoder.get_state() == StreamDecoderState::SeekError {
            decoder.flush();
        }
        return Err(Error::new(
            "Seeking of FLAC stream failed: libFLAC seek failed.",
        ));
    }

    client.t.current_iteration = final_iteration;
    client.t.current_iteration_frames = final_iteration_frames;

    Ok(())
}

impl FlacTrackData<'_> {
    /// `FLAC_seek()` on the track, the write callback putting what the
    /// seek decodes into `stream` (`tdata->stream`).
    ///
    /// FIXME (upstream): libFLAC's seek decodes the frame it lands in and
    /// hands it to the write callback, which puts it into `tdata->stream`;
    /// before the first `FLAC_decode()` that is NULL, so that frame's audio
    /// goes nowhere: predecoding (which seeks to 0 first) loses the first
    /// frame. A seek that the mixer follows with clearing the stream (a
    /// new playback position) loses the rest of that frame, too. Kept.
    fn seek_into(&mut self, frame: u64, stream: Option<&AudioStream>) -> Result<()> {
        let stream = if self.t.has_stream { stream } else { None };
        flac_seek(
            &mut self.decoder,
            &mut FlacClient {
                t: &mut self.t,
                stream,
            },
            frame,
        )
    }
}

impl TrackData for FlacTrackData<'_> {
    /// Translation of `FLAC_decode()`.
    fn decode(&mut self, stream: &AudioStream) -> bool {
        self.t.has_stream = true;

        let mut client = FlacClient {
            t: &mut self.t,
            stream: Some(stream),
        };
        if !self.decoder.process_single(&mut client) {
            // write callback will fill in stream. Might fill 0 if it hit a metadata block, but the higher level loops to get what it needs.
            return false;
        } else if self.decoder.get_state() == StreamDecoderState::EndOfStream {
            return false; // we're done.
        }

        true
    }

    /// Translation of `FLAC_seek()`, before any `FLAC_decode()` gave the
    /// track a stream (or where the mixer has none to give).
    fn seek(&mut self, frame: u64) -> Result<()> {
        self.seek_into(frame, None)
    }

    /// Translation of `FLAC_seek()`: what libFLAC decodes for the seek goes
    /// to the stream the decoding goes to.
    fn seek_with_stream(&mut self, frame: u64, stream: Option<&AudioStream>) -> Result<()> {
        self.seek_into(frame, stream)
    }
}

impl Drop for FlacTrackData<'_> {
    /// Translation of `FLAC_quit_track()`.
    fn drop(&mut self) {
        self.decoder.finish(); // (and FLAC__stream_decoder_delete() is dropping it)
    }
}

/// Translation of `MIX_Decoder_FLAC`.
pub(crate) static DECODER: Decoder = Decoder {
    name: "FLAC",
    init: None,
    init_audio: flac_init_audio,
    has_jump_to_order: false,
    quit: None,
};
