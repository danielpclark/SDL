// Rust translation of src/decoder_opus.c from SDL_mixer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Ogg Opus decoder, using opusfile (see the `opusfile` module) over the
//! libopus decoder (the `opus` module), both translated here.

// This file supports Ogg Opus audio streams using libopusfile.

use std::sync::Arc;

use sdl3::audio::{AudioFormat, AudioSpec, AudioStream};
use sdl3::error::{Error, Result};
use sdl3::io::{IoStream, IoWhence};
use sdl3::properties::Properties;

use crate::internal::{borrow_io, put_f32, AudioData, Decoder, OggLoop, TrackData};
use crate::metadata_tags::parse_ogg_comments;
use crate::opusfile::opusfile::{
    op_head, op_open_callbacks, op_pcm_seek, op_pcm_total, op_raw_seek, op_read_float, op_tags,
    op_test_callbacks, OggOpusFile,
};
use crate::opusfile::*;
use crate::{DURATION_INFINITE, PROP_AUDIO_LOAD_IGNORE_LOOPS_BOOLEAN};

/// Translation of `OPUS_AudioData`.
struct OpusAudioData {
    loop_: OggLoop,
}

/// Translation of `OPUS_TrackData`.
struct OpusTrackData<'a> {
    adata: Arc<OpusAudioData>,
    of: Box<OggOpusFile<'a>>,
    current_channels: i32,
    current_bitstream: i32,
    current_iteration: i64,
    current_iteration_frames: i64,
}

/// Translation of `set_op_error()`.
fn set_op_error(function: &str, error: i32) -> Error {
    let name = match error {
        OP_FALSE => "OP_FALSE",
        OP_EOF => "OP_EOF",
        OP_HOLE => "OP_HOLE",
        OP_EREAD => "OP_EREAD",
        OP_EFAULT => "OP_EFAULT",
        OP_EIMPL => "OP_EIMPL",
        OP_EINVAL => "OP_EINVAL",
        OP_ENOTFORMAT => "OP_ENOTFORMAT",
        OP_EBADHEADER => "OP_EBADHEADER",
        OP_EVERSION => "OP_EVERSION",
        OP_ENOTAUDIO => "OP_ENOTAUDIO",
        OP_EBADPACKET => "OP_EBADPACKET",
        OP_EBADLINK => "OP_EBADLINK",
        OP_ENOSEEK => "OP_ENOSEEK",
        OP_EBADTIMESTAMP => "OP_EBADTIMESTAMP",
        _ => return Error::new(format!("{function}: unknown error {error}")),
    };
    Error::new(format!("{function}: {name}"))
}

/// Translation of `OPUS_init_audio()`.
fn opus_init_audio(
    io: Option<&mut IoStream<'_>>,
    spec: &mut AudioSpec,
    props: &Properties,
    duration_frames: &mut i64,
) -> Result<Arc<dyn AudioData>> {
    let io = io.ok_or_else(|| Error::invalid_param("io"))?;

    // just load the bare minimum from the IOStream to verify it's an Opus file.
    // !!! FIXME: is op_open_callbacks going to return more slowly if this isn't an Opus file? It's probably better to just do the full open.
    match op_test_callbacks(borrow_io(io), &[]) {
        Ok(of) => drop(of),
        Err(_) => return Err(Error::new("Not an Opus audio stream")),
    }

    // Go back and do a proper load now to get metadata.
    io.seek(0, IoWhence::Set)?;

    let mut loop_ = OggLoop::default();

    // now open the stream for serious processing.
    let mut of = op_open_callbacks(borrow_io(io), &[])
        .map_err(|rc| set_op_error("ov_open_callbacks", rc))?;

    // (op_head() can't return NULL here, so there's no "Couldn't get Opus info; corrupt data?" error.)
    let info = op_head(&of, -1);

    spec.format = AudioFormat::F32;
    spec.channels = info.channel_count;
    spec.freq = 48000; // libopus only outputs at 48kHz.

    if let Some(tags) = op_tags(&of, -1) {
        let comments: Vec<&[u8]> = tags.user_comments[..tags.comments as usize]
            .iter()
            .map(|c| c.as_slice())
            .collect();
        parse_ogg_comments(props, spec.freq, Some(&tags.vendor), &comments, &mut loop_);
    }

    op_raw_seek(&mut of, 0); // !!! FIXME: it's not clear if this seek is necessary, but https://stackoverflow.com/a/72482773 suggests it might be, at least on older libvorbisfile releases...
    let full_length = op_pcm_total(&of, -1);

    let ignore_loops = props
        .get_bool(PROP_AUDIO_LOAD_IGNORE_LOOPS_BOOLEAN)
        .unwrap_or(false);
    if ignore_loops || (loop_.end > full_length) {
        loop_.active = false;
    }

    drop(of); // done with this instance. Tracks will maintain their own OggOpusFile object.

    if loop_.active {
        *duration_frames = if loop_.count < 0 {
            DURATION_INFINITE
        } else {
            // Note (upstream): this is the whole file `count` times there, where
            // only the loop plays `count` times; the translation counts the rest
            // of the file once.
            full_length.wrapping_add(loop_.len.wrapping_mul(loop_.count - 1))
        };
    } else {
        *duration_frames = full_length;
    }

    Ok(Arc::new(OpusAudioData { loop_ }))
}

impl AudioData for OpusAudioData {
    /// Translation of `OPUS_init_track()`.
    fn init_track<'a>(
        self: Arc<Self>,
        io: Option<IoStream<'a>>,
        spec: &AudioSpec,
        _props: &Properties,
    ) -> Result<Box<dyn TrackData + 'a>> {
        let io = io.ok_or_else(|| Error::invalid_param("io"))?;

        // now open the stream for serious processing.
        let of = op_open_callbacks(io, &[]).map_err(|rc| set_op_error("op_open_callbacks", rc))?;

        Ok(Box::new(OpusTrackData {
            adata: self,
            of,
            current_channels: spec.channels,
            current_bitstream: -1,
            current_iteration: -1,
            current_iteration_frames: 0,
        }))
    }
}

impl TrackData for OpusTrackData<'_> {
    /// Translation of `OPUS_decode()`.
    fn decode(&mut self, stream: &AudioStream) -> bool {
        let mut bitstream = self.current_bitstream;
        let mut samples = [0.0f32; 256];

        let channels = self.current_channels;
        let mut amount = op_read_float(&mut self.of, &mut samples, 256, Some(&mut bitstream));
        if amount < 0 {
            let _ = set_op_error("op_read_float", amount); // (upstream sets the error; decode() can only report false)
            return false;
        } else if amount == 0 {
            return false; // EOF
        }

        // Note (upstream): SDL_assert((amount * channels) <= SDL_arraysize(samples)),
        // but `channels` is the previous link's channel count, so a chained
        // file whose next link has fewer channels fails it (op_read_float()
        // never returns more than fits); it's left out.
        let _ = channels;

        if bitstream != self.current_bitstream {
            let info = op_head(&self.of, -1);
            // (this _shouldn't_ be NULL, and in Rust it can't be.)
            if self.current_channels != info.channel_count {
                let spec = AudioSpec {
                    format: AudioFormat::F32,
                    channels: info.channel_count,
                    freq: 48000,
                };
                let _ = stream.set_format(Some(&spec), None);
                self.current_channels = info.channel_count;
            }
            self.current_bitstream = bitstream;
        }

        let loop_ = self.adata.loop_;
        if self.current_iteration == -1
            && loop_.active
            && (self.current_iteration_frames.wrapping_add(amount as i64) >= loop_.start)
        {
            self.current_iteration = 0; // we've hit the start of the loop point.
            self.current_iteration_frames = self.current_iteration_frames.wrapping_sub(loop_.start);
            // so adding `amount` corrects this later.
        }

        if self.current_iteration >= 0 {
            debug_assert!(loop_.active);
            // Note (upstream): SDL_assert(current_iteration_frames <= loop->len)
            // here and below fails on damaged files (where a looping seek
            // doesn't land where it asked), so it's left out.
            let full_amount = amount;
            let available = loop_.len.wrapping_sub(self.current_iteration_frames);
            if amount as i64 > available {
                amount = available as i32;
            }

            if self.current_iteration_frames.wrapping_add(amount as i64) >= loop_.len {
                // time to loop?
                let mut should_loop = false;
                if loop_.count < 0 {
                    // negative==infinite loop
                    self.current_iteration = 0;
                    should_loop = true;
                } else {
                    self.current_iteration += 1;
                    debug_assert!(self.current_iteration <= loop_.count);
                    if self.current_iteration < loop_.count {
                        should_loop = true;
                    }
                }

                if should_loop {
                    let nextframe = (loop_.start as u64).wrapping_add(
                        (loop_.len as u64).wrapping_mul(self.current_iteration as u64),
                    );
                    if self.seek(nextframe).is_err() {
                        return false;
                    }
                } else {
                    // Note (upstream): this sets current_iteration back to -1
                    // (before the loop) there, so with current_iteration_frames
                    // back at 0, the check above saw the loop start again
                    // `loop.start` frames later and a loop with a LOOPCOUNT
                    // played forever; the translation's -2 means it's done. And
                    // the rest of this chunk, past the loop's end, was cut off
                    // there; here it plays on.
                    self.current_iteration = -2; // done looping.
                    amount = full_amount;
                }
                self.current_iteration_frames = 0;
            }
        }

        if amount > 0 {
            let n = (amount * self.current_channels).clamp(0, samples.len() as i32) as usize;
            let _ = put_f32(stream, &samples[..n]);
            self.current_iteration_frames =
                self.current_iteration_frames.wrapping_add(amount as i64);
        }

        true // had more data to decode.
    }

    /// Translation of `OPUS_seek()`.
    fn seek(&mut self, mut frame: u64) -> Result<()> {
        let loop_ = self.adata.loop_;
        let mut final_iteration: i64 = -1;
        // Note (upstream): this starts at 0 there, so a frame before the loop
        // left current_iteration_frames at 0, where it means `frame` (and the
        // loop started that many frames late); the translation starts it at
        // `frame`.
        let mut final_iteration_frames: i64 = frame as i64;

        // Note (upstream): for a loop with a LOOPCOUNT, the test below leaves
        // out the loop's start there (`frame < len * count`), and the frames
        // past the loop land a loop's length early (`len * count` comes off,
        // where the loop's first iteration is the file's own), so seeks into
        // the loop's last `start` frames or past the loop went wrong (and so
        // did the looping seek to the last iteration of a loop that starts
        // no sooner than its length); the translation tests `frame < start +
        // len * count` and takes `len * (count - 1)` off.
        // frame has hit the loop point?
        if loop_.active && (frame as i64 >= loop_.start) {
            // figure out the _actual_ frame in the vorbis file we're aiming for.
            if (loop_.count < 0)
                || ((frame as i64)
                    < loop_
                        .start
                        .wrapping_add(loop_.len.wrapping_mul(loop_.count)))
            {
                // literally in the loop right now.
                frame = frame.wrapping_sub(loop_.start as u64); // make logical frame index relative to start of loop.
                final_iteration = if loop_.count < 0 {
                    0
                } else {
                    frame.checked_div(loop_.len as u64).unwrap_or(0) as i64
                }; // decide what iteration of the loop we're on (stays at zero for infinite loops).
                frame = frame.checked_rem(loop_.len as u64).unwrap_or(frame); // drop iterations so we're an offset into the loop.
                final_iteration_frames = frame as i64;
                frame = frame.wrapping_add(loop_.start as u64); // convert back into physical frame index.
            } else {
                // past the loop point?
                debug_assert!(loop_.count > 0); // can't be infinite loop if we passed it.
                frame = frame.wrapping_sub(loop_.len.wrapping_mul(loop_.count - 1) as u64);
                // drop the iterations to get the physical frame index.
                // Note (upstream): -1 there, which loops again.
                final_iteration = -2; // done looping.
            }
        }

        // !!! FIXME: I assume op_raw_seek is faster if we're seeking to start, but I could be wrong.
        let rc = if frame == 0 {
            op_raw_seek(&mut self.of, 0)
        } else {
            op_pcm_seek(&mut self.of, frame as i64)
        };
        if rc != 0 {
            return Err(set_op_error("op_pcm_seek", rc));
        }

        self.current_iteration = final_iteration;
        self.current_iteration_frames = final_iteration_frames;

        Ok(())
    }
}

/// Translation of `MIX_Decoder_OPUS`.
pub(crate) static DECODER: Decoder = Decoder {
    name: "OPUS",
    init: None,
    init_audio: opus_init_audio,
    has_jump_to_order: false,
    quit: None,
};
