// Rust translation of src/decoder_stb_vorbis.c from SDL_mixer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Ogg Vorbis decoder, using stb_vorbis (see `stb_vorbis.rs`).

// This file supports Ogg Vorbis audio streams using the public-domain, header-only library, stb_vorbis.

use std::sync::Arc;

use sdl3::audio::{AudioFormat, AudioSpec, AudioStream};
use sdl3::error::{Error, Result};
use sdl3::io::{IoStream, IoWhence};
use sdl3::properties::Properties;

use crate::internal::{borrow_io, AudioData, Decoder, OggLoop, TrackData};
use crate::metadata_tags::parse_ogg_comments;
use crate::stb_vorbis::*;
use crate::DURATION_INFINITE;

/// Translation of `SetStbVorbisError()`.
fn set_stb_vorbis_error(function: &str, error: StbVorbisError) -> Error {
    let name = match error {
        VORBIS_NEED_MORE_DATA => "VORBIS_need_more_data",
        VORBIS_INVALID_API_MIXING => "VORBIS_invalid_api_mixing",
        VORBIS_OUTOFMEM => "VORBIS_outofmem",
        VORBIS_FEATURE_NOT_SUPPORTED => "VORBIS_feature_not_supported",
        VORBIS_TOO_MANY_CHANNELS => "VORBIS_too_many_channels",
        VORBIS_FILE_OPEN_FAILURE => "VORBIS_file_open_failure",
        VORBIS_SEEK_WITHOUT_LENGTH => "VORBIS_seek_without_length",
        VORBIS_UNEXPECTED_EOF => "VORBIS_unexpected_eof",
        VORBIS_SEEK_INVALID => "VORBIS_seek_invalid",
        VORBIS_INVALID_SETUP => "VORBIS_invalid_setup",
        VORBIS_INVALID_STREAM => "VORBIS_invalid_stream",
        VORBIS_MISSING_CAPTURE_PATTERN => "VORBIS_missing_capture_pattern",
        VORBIS_INVALID_STREAM_STRUCTURE_VERSION => "VORBIS_invalid_stream_structure_version",
        VORBIS_CONTINUED_PACKET_FLAG_INVALID => "VORBIS_continued_packet_flag_invalid",
        VORBIS_INCORRECT_STREAM_SERIAL_NUMBER => "VORBIS_incorrect_stream_serial_number",
        VORBIS_INVALID_FIRST_PAGE => "VORBIS_invalid_first_page",
        VORBIS_BAD_PACKET_TYPE => "VORBIS_bad_packet_type",
        VORBIS_CANT_FIND_LAST_PAGE => "VORBIS_cant_find_last_page",
        VORBIS_SEEK_FAILED => "VORBIS_seek_failed",
        VORBIS_OGG_SKELETON_NOT_SUPPORTED => "VORBIS_ogg_skeleton_not_supported",
        _ => return Error::new(format!("{function}: unknown error {error}\n")),
    };
    Error::new(format!("{function}: {name}"))
}

/// Translation of `STBVORBIS_AudioData`.
struct StbVorbisAudioData {
    loop_: OggLoop,
}

/// Translation of `STBVORBIS_TrackData`.
struct StbVorbisTrackData<'a> {
    adata: Arc<StbVorbisAudioData>,
    vorbis: StbVorbis<'a>,
    skip_samples: u32,
    current_iteration: i64,
    current_iteration_frames: i64,
}

/// Translation of `STBVORBIS_init_audio()`.
fn stbvorbis_init_audio(
    io: Option<&mut IoStream<'_>>,
    spec: &mut AudioSpec,
    props: &Properties,
    duration_frames: &mut i64,
) -> Result<Arc<dyn AudioData>> {
    let io = io.ok_or_else(|| Error::invalid_param("io"))?;

    // just load the bare minimum from the IOStream to verify it's an Ogg Vorbis file.
    let mut buffer = [0u8; 35]; // this is just enough to see "OggS" at the start and "vorbis" at the end.

    // the initial Ogg Page should catch this in 35 bytes; no matter how large the page might be,
    // the initial portion will still start with OggS and have a Vorbis header at the same place,
    // knock on wood.
    if io.read(&mut buffer) != buffer.len() {
        return Err(Error::new("Not an Ogg Vorbis audio stream")); // (upstream sets no error here.)
    } else if &buffer[..4] != b"OggS" {
        return Err(Error::new("Not an Ogg Vorbis audio stream"));
    } else if &buffer[29..35] != b"vorbis" {
        return Err(Error::new("Not an Ogg Vorbis audio stream"));
    }

    // Go back and do a proper load now to get metadata.
    io.seek(0, IoWhence::Set)?;

    // now open the stream for serious processing.
    let mut vorbis = StbVorbis::open_io(borrow_io(io))
        .map_err(|error| set_stb_vorbis_error("stb_vorbis_open_memory", error))?;

    let vi = vorbis.get_info();
    spec.format = AudioFormat::F32;
    spec.channels = vi.channels;
    spec.freq = vi.sample_rate as i32;

    let mut loop_ = OggLoop::default();
    {
        let (vendor, comment_list) = vorbis.get_comment();
        let comments: Vec<&[u8]> = comment_list.iter().map(|c| c.as_slice()).collect();
        parse_ogg_comments(props, spec.freq, Some(vendor), &comments, &mut loop_);
    }

    let full_length = vorbis.stream_length_in_samples() as i64;
    if loop_.end > full_length {
        loop_.active = false;
    }
    drop(vorbis); // done with this instance. Tracks will maintain their own stb_vorbis object.

    if loop_.active {
        *duration_frames = if loop_.count < 0 {
            DURATION_INFINITE
        } else {
            full_length.wrapping_mul(loop_.count)
        };
    } else {
        *duration_frames = full_length;
    }

    Ok(Arc::new(StbVorbisAudioData { loop_ }))
}

impl AudioData for StbVorbisAudioData {
    /// Translation of `STBVORBIS_init_track()`.
    fn init_track<'a>(
        self: Arc<Self>,
        io: Option<IoStream<'a>>,
        _spec: &AudioSpec,
        _props: &Properties,
    ) -> Result<Box<dyn TrackData + 'a>> {
        let io = io.ok_or_else(|| Error::invalid_param("io"))?;
        let vorbis = StbVorbis::open_io(io)
            .map_err(|error| set_stb_vorbis_error("stb_vorbis_open_io", error))?;

        Ok(Box::new(StbVorbisTrackData {
            adata: self,
            vorbis,
            skip_samples: 0,
            current_iteration: -1,
            current_iteration_frames: 0,
        }))
    }
}

impl TrackData for StbVorbisTrackData<'_> {
    /// Translation of `STBVORBIS_decode()`.
    fn decode(&mut self, stream: &AudioStream) -> bool {
        // Note that stb_vorbis does not currently handle the bitstream id
        // changing--a "chained" ogg file, or perhaps a "frankenstein" file, as
        // mpg123 calls it--where two unrelated .ogg files, possibly with
        // different audio specs, are cat'd together. libvorbisfile can handle
        // this, but at the moment stb_vorbis will call it EOF at the end of the
        // current bitstream. So we don't have all the decoder_vorbis.c code to
        // change audio specs mid-file here.

        let mut amount;
        loop {
            let has_deferred = self.vorbis.discard_samples_deferred > 0;
            amount = self.vorbis.get_frame_float();
            if !((amount == 0) && has_deferred) {
                break;
            }
        } // if it's still flushing out garbage at the start of the stream, keep trying.

        if amount <= 0 {
            return false; // EOF
        }

        // did we just seek and need to throw away some samples at the start of the frame to reach the exact seek point?
        let mut skip = 0usize;
        if self.skip_samples != 0 {
            let s = self.skip_samples;
            if s >= amount as u32 {
                self.skip_samples -= amount as u32;
                return true; // throw this all away; just try again next iteration.
            }
            skip = s as usize;
            self.skip_samples = 0;
            amount = (amount as u32 - s) as i32;
        }

        let loop_ = self.adata.loop_;
        if self.current_iteration < 0
            && loop_.active
            && (self.current_iteration_frames.wrapping_add(amount as i64) >= loop_.start)
        {
            self.current_iteration = 0; // we've hit the start of the loop point.
            self.current_iteration_frames = self.current_iteration_frames.wrapping_sub(loop_.start);
            // so adding `amount` corrects this later.
        }

        // Note (upstream): the output pointers point into the decoder's
        // channel buffers, which a looping seek below decodes into, so the
        // samples output when looping are from that seek's decoding.
        let output_start = self.vorbis.output_start() + skip;

        if self.current_iteration >= 0 {
            debug_assert!(loop_.active);
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
                    self.current_iteration = -1;
                }
                self.current_iteration_frames = 0;
            }
        }

        if amount > 0 {
            let n = amount as usize;
            let num_channels = self.vorbis.get_info().channels.max(0) as usize;
            let planes: Vec<Vec<u8>> = (0..num_channels)
                .map(|i| {
                    let buf = self.vorbis.channel_buffer(i);
                    let start = output_start.min(buf.len());
                    let end = output_start.saturating_add(n).min(buf.len());
                    buf[start..end]
                        .iter()
                        .flat_map(|s| s.to_ne_bytes())
                        .collect()
                })
                .collect();
            let bufs: Vec<Option<&[u8]>> = planes
                .iter()
                .map(|p| Some(&p[..(n * 4).min(p.len())]))
                .collect();
            let _ = stream.put_planar_data(&bufs, n);
            self.current_iteration_frames =
                self.current_iteration_frames.wrapping_add(amount as i64);
        }

        true // had more data to decode.
    }

    /// Translation of `STBVORBIS_seek()`.
    fn seek(&mut self, mut frame: u64) -> Result<()> {
        let loop_ = self.adata.loop_;
        let mut final_iteration: i64 = -1;
        let mut final_iteration_frames: i64 = 0;

        // frame has hit the loop point?
        if loop_.active && (frame as i64 >= loop_.start) {
            // figure out the _actual_ frame in the vorbis file we're aiming for.
            if (loop_.count < 0) || ((frame as i64) < loop_.len.wrapping_mul(loop_.count)) {
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

        if !self.vorbis.seek_frame(frame as u32) {
            return Err(set_stb_vorbis_error(
                "stb_vorbis_seek",
                self.vorbis.get_error(),
            ));
        }

        self.skip_samples = frame.wrapping_sub(self.vorbis.current_loc as u64) as u32;
        self.current_iteration = final_iteration;
        self.current_iteration_frames = final_iteration_frames;

        Ok(())
    }
}

/// Translation of `MIX_Decoder_STBVORBIS`.
pub(crate) static DECODER: Decoder = Decoder {
    name: "STBVORBIS",
    init: None,
    init_audio: stbvorbis_init_audio,
    has_jump_to_order: false,
    quit: None,
};
