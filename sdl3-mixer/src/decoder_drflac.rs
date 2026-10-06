// Rust translation of src/decoder_drflac.c from SDL_mixer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! FLAC decoder, using dr_flac (see `dr_flac.rs`).

use std::sync::Arc;

use sdl3::audio::{AudioFormat, AudioSpec, AudioStream};
use sdl3::error::{Error, Result};
use sdl3::io::{IoStream, IoWhence};
use sdl3::properties::Properties;

use crate::dr_flac::{
    Drflac, VorbisComment, VorbisCommentIterator, DRFLAC_METADATA_BLOCK_TYPE_VORBIS_COMMENT,
};
use crate::internal::{borrow_io, put_f32, AudioData, Decoder, OggLoop, TrackData};
use crate::metadata_tags::parse_ogg_comments;
use crate::{DURATION_INFINITE, DURATION_UNKNOWN, PROP_AUDIO_LOAD_IGNORE_LOOPS_BOOLEAN};

/// Translation of `DRFLAC_AudioData`.
struct DrflacAudioData {
    framesize: usize,
    loop_: OggLoop,
}

/// Translation of `DRFLAC_TrackData`.
struct DrflacTrackData<'a> {
    adata: Arc<DrflacAudioData>,
    decoder: Drflac<'a>,
    current_iteration: i64,
    current_iteration_frames: i64,
}

/// Translation of `DRFLAC_Metadata`.
#[derive(Default)]
struct DrflacMetadata {
    vendor: Option<Vec<u8>>,
    comments: Vec<Vec<u8>>,
}

/// Translation of `DRFLAC_OnMetadata()`.
fn on_metadata(metadata: &mut DrflacMetadata, block_type: u8, data: Option<&VorbisComment<'_>>) {
    if block_type == DRFLAC_METADATA_BLOCK_TYPE_VORBIS_COMMENT {
        let Some(data) = data else {
            return;
        };

        if metadata.vendor.is_none() {
            metadata.vendor = Some(data.vendor.to_vec());
        }

        let mut iter = VorbisCommentIterator::new(data.comment_count, data.comments);
        while let Some(comment) = iter.next_comment() {
            metadata.comments.push(comment.to_vec());
        }
    }
}

/// Translation of `DRFLAC_init_audio()`.
fn drflac_init_audio(
    io: Option<&mut IoStream<'_>>,
    spec: &mut AudioSpec,
    props: &Properties,
    duration_frames: &mut i64,
) -> Result<Arc<dyn AudioData>> {
    let io = io.ok_or_else(|| Error::invalid_param("io"))?;

    // just load the bare minimum from the IOStream to verify it's a FLAC file (if it's an Ogg stream, we'll let libFLAC try to parse it out).
    //bool is_ogg_stream = false;
    let mut magic = [0u8; 4];
    if io.read(&mut magic) != 4 {
        return Err(Error::new("DRFLAC: Not a FLAC audio stream")); // (upstream sets no error here.)
    } else if &magic == b"OggS" {
        //is_ogg_stream = true;  // MAYBE flac, might be vorbis, etc.
    } else if &magic != b"fLaC" {
        return Err(Error::new("Not a FLAC audio stream"));
    }

    // Go back and do a proper load now to get metadata.
    // Note (upstream): this passes SDL_IO_SEEK_SET as the offset and 0 as
    // the whence, which happen to be the same thing.
    io.seek(0, IoWhence::Set)?;

    // open upfront to make sure data is usable and pull in metadata.
    let mut metadata = DrflacMetadata::default();
    let decoder = Drflac::open_with_metadata(borrow_io(io), &mut |block_type, data| {
        on_metadata(&mut metadata, block_type, data)
    });
    let Some(decoder) = decoder else {
        return Err(Error::new("DRFLAC: Not a FLAC audio stream")); // probably not a FLAC file. (upstream sets no error here.)
    };

    let mut loop_ = OggLoop::default();
    let comments: Vec<&[u8]> = metadata.comments.iter().map(|c| c.as_slice()).collect();
    parse_ogg_comments(
        props,
        decoder.sample_rate as i32,
        metadata.vendor.as_deref(),
        &comments,
        &mut loop_,
    );

    spec.format = AudioFormat::F32;
    spec.channels = decoder.channels as i32;
    spec.freq = decoder.sample_rate as i32;

    let framesize = spec.format.bytesize() as usize * spec.channels as usize;

    let ignore_loops = props
        .get_bool(PROP_AUDIO_LOAD_IGNORE_LOOPS_BOOLEAN)
        .unwrap_or(false);
    if ignore_loops || (loop_.end > decoder.total_pcm_frame_count as i64) {
        loop_.active = false;
    }

    if decoder.total_pcm_frame_count == 0 {
        *duration_frames = DURATION_UNKNOWN;
    } else if loop_.active {
        *duration_frames = if loop_.count < 0 {
            DURATION_INFINITE
        } else {
            (decoder.total_pcm_frame_count as i64).wrapping_mul(loop_.count)
        };
    } else {
        *duration_frames = decoder.total_pcm_frame_count as i64;
    }

    Ok(Arc::new(DrflacAudioData { framesize, loop_ }))
}

impl AudioData for DrflacAudioData {
    /// Translation of `DRFLAC_init_track()`.
    fn init_track<'a>(
        self: Arc<Self>,
        io: Option<IoStream<'a>>,
        _spec: &AudioSpec,
        _props: &Properties,
    ) -> Result<Box<dyn TrackData + 'a>> {
        let io = io.ok_or_else(|| Error::invalid_param("io"))?;
        let Some(decoder) = Drflac::open(io) else {
            return Err(Error::new("DRFLAC: Not a FLAC audio stream")); // (upstream sets no error here.)
        };

        Ok(Box::new(DrflacTrackData {
            adata: self,
            decoder,
            current_iteration: -1,
            current_iteration_frames: 0,
        }))
    }
}

impl TrackData for DrflacTrackData<'_> {
    /// Translation of `DRFLAC_decode()`.
    fn decode(&mut self, stream: &AudioStream) -> bool {
        let framesize = self.adata.framesize;
        let mut samples = [0.0f32; 256];
        let mut amount = self.decoder.read_pcm_frames_f32(
            (std::mem::size_of_val(&samples) / framesize) as u64,
            Some(&mut samples),
        );
        if amount == 0 {
            return false; // done decoding.
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

        if self.current_iteration >= 0 {
            debug_assert!(loop_.active);
            let available = loop_.len.wrapping_sub(self.current_iteration_frames);
            if amount as i64 > available {
                amount = available as u64;
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
            let n = (amount as usize).wrapping_mul(framesize) / 4;
            let _ = put_f32(stream, &samples[..n.min(samples.len())]);
            self.current_iteration_frames =
                self.current_iteration_frames.wrapping_add(amount as i64);
        }

        true // had more data to decode.
    }

    /// Translation of `DRFLAC_seek()`.
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

        if !self.decoder.seek_to_pcm_frame(frame) {
            return Err(Error::new("DRFLAC: seek failed")); // (upstream sets no error here.)
        }

        self.current_iteration = final_iteration;
        self.current_iteration_frames = final_iteration_frames;

        Ok(())
    }
}

/// Translation of `MIX_Decoder_DRFLAC`.
pub(crate) static DECODER: Decoder = Decoder {
    name: "DRFLAC",
    init: None,
    init_audio: drflac_init_audio,
    has_jump_to_order: false,
    quit: None,
};
