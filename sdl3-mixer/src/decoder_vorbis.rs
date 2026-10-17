// Rust translation of src/decoder_vorbis.c from SDL_mixer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Ogg Vorbis decoder, using libvorbisfile over libvorbis (the `vorbis`
//! module), both translated here.

// This file supports Ogg Vorbis audio streams using libvorbisfile (or the integer-only implementation, "tremor").

// (The tremor build, VORBIS_USE_TREMOR, isn't translated: upstream's
// default build uses libvorbisfile, and vendors no tremor.)

use std::sync::Arc;

use sdl3::audio::{AudioFormat, AudioSpec, AudioStream};
use sdl3::error::{Error, Result};
use sdl3::io::{IoStream, IoWhence};
use sdl3::properties::Properties;

use crate::internal::{borrow_io, AudioData, Decoder, OggLoop, TrackData};
use crate::metadata_tags::parse_ogg_comments;
use crate::vorbis::vorbisfile::{
    ov_clear, ov_comment, ov_info, ov_open_callbacks, ov_pcm_seek, ov_pcm_total, ov_raw_seek,
    ov_read_float, ov_test_callbacks, OggVorbisFile,
};
use crate::vorbis::*;
use crate::{DURATION_INFINITE, PROP_AUDIO_LOAD_IGNORE_LOOPS_BOOLEAN};

const VORBIS_AUDIO_FORMAT: AudioFormat = AudioFormat::F32;

/// Translation of `VORBIS_AudioData`.
struct VorbisAudioData {
    #[allow(dead_code)]
    framesize: usize,
    loop_: OggLoop,
}

/// Translation of `VORBIS_TrackData`.
struct VorbisTrackData<'a> {
    adata: Arc<VorbisAudioData>,
    vf: Box<OggVorbisFile<'a>>,
    current_channels: i32,
    current_freq: i32,
    current_bitstream: i32,
    current_iteration: i64,
    current_iteration_frames: i64,
}

/// Translation of `SetOggVorbisError()`.
fn set_ogg_vorbis_error(function: &str, error: i32) -> Error {
    let name = match error {
        OV_FALSE => "OV_FALSE",
        OV_EOF => "OV_EOF",
        OV_HOLE => "OV_HOLE",
        OV_EREAD => "OV_EREAD",
        OV_EFAULT => "OV_EFAULT",
        OV_EIMPL => "OV_EIMPL",
        OV_EINVAL => "OV_EINVAL",
        OV_ENOTVORBIS => "OV_ENOTVORBIS",
        OV_EBADHEADER => "OV_EBADHEADER",
        OV_EVERSION => "OV_EVERSION",
        OV_ENOTAUDIO => "OV_ENOTAUDIO",
        OV_EBADPACKET => "OV_EBADPACKET",
        OV_EBADLINK => "OV_EBADLINK",
        OV_ENOSEEK => "OV_ENOSEEK",
        _ => return Error::new(format!("{function}: unknown error {error}")),
    };
    Error::new(format!("{function}: {name}"))
}

/// Translation of `VORBIS_init_audio()`.
fn vorbis_init_audio(
    io: Option<&mut IoStream<'_>>,
    spec: &mut AudioSpec,
    props: &Properties,
    duration_frames: &mut i64,
) -> Result<Arc<dyn AudioData>> {
    let io = io.ok_or_else(|| Error::invalid_param("io"))?;

    let mut buffer = [0u8; 17];
    if io.read(&mut buffer) != buffer.len() {
        return Err(Error::new("Not an Ogg Vorbis audio stream")); // (upstream sets no error here.)
    } else if &buffer == b"Extended Module: " {
        // this is an XM file that _might_ be OXM and we would incorrectly claim it. Drop out now.
        return Err(Error::new("Not an Ogg Vorbis audio stream"));
    }

    // Go back and let ov_test_callbacks() take a run at it, too.
    io.seek(0, IoWhence::Set)?;

    // just load the bare minimum from the IOStream to verify it's an Ogg Vorbis file.
    // !!! FIXME: is ov_open_callbacks going to return more slowly if this isn't a Vorbis file? It's probably better to just do the full open.
    match ov_test_callbacks(borrow_io(io)) {
        Ok(mut vf) => {
            ov_clear(&mut vf);
        }
        Err(_) => return Err(Error::new("Not an Ogg Vorbis audio stream")),
    }

    // Go back and do a proper load now to get metadata.
    io.seek(0, IoWhence::Set)?;

    let mut loop_ = OggLoop::default();

    // now open the stream for serious processing.
    let mut vf = ov_open_callbacks(borrow_io(io))
        .map_err(|rc| set_ogg_vorbis_error("ov_open_callbacks", rc))?;

    let Some(vi) = ov_info(&vf, -1) else {
        ov_clear(&mut vf);
        return Err(Error::new("Couldn't get Ogg Vorbis info; corrupt data?"));
    };

    spec.format = VORBIS_AUDIO_FORMAT;
    spec.channels = vi.channels;
    spec.freq = vi.rate as i32;

    let framesize = spec.frame_size();

    if let Some(vc) = ov_comment(&vf, -1) {
        let comments: Vec<&[u8]> = vc.user_comments[..vc.comments as usize]
            .iter()
            .map(|c| c.as_slice())
            .collect();
        parse_ogg_comments(
            props,
            spec.freq,
            vc.vendor_set.then_some(&vc.vendor[..]),
            &comments,
            &mut loop_,
        );
    }

    ov_raw_seek(&mut vf, 0); // !!! FIXME: it's not clear if this seek is necessary, but https://stackoverflow.com/a/72482773 suggests it might be, at least on older libvorbisfile releases...
    let full_length = ov_pcm_total(&vf, -1);

    let ignore_loops = props
        .get_bool(PROP_AUDIO_LOAD_IGNORE_LOOPS_BOOLEAN)
        .unwrap_or(false);
    if ignore_loops || (loop_.end > full_length) {
        loop_.active = false;
    }

    ov_clear(&mut vf); // done with this instance. Tracks will maintain their own OggVorbis_File object.

    if loop_.active {
        *duration_frames = if loop_.count < 0 {
            DURATION_INFINITE
        } else {
            full_length.wrapping_mul(loop_.count)
        };
    } else {
        *duration_frames = full_length;
    }

    Ok(Arc::new(VorbisAudioData { framesize, loop_ }))
}

impl AudioData for VorbisAudioData {
    /// Translation of `VORBIS_init_track()`.
    fn init_track<'a>(
        self: Arc<Self>,
        io: Option<IoStream<'a>>,
        spec: &AudioSpec,
        _props: &Properties,
    ) -> Result<Box<dyn TrackData + 'a>> {
        let io = io.ok_or_else(|| Error::invalid_param("io"))?;

        // now open the stream for serious processing.
        let vf =
            ov_open_callbacks(io).map_err(|rc| set_ogg_vorbis_error("ov_open_callbacks", rc))?;

        Ok(Box::new(VorbisTrackData {
            adata: self,
            vf,
            current_channels: spec.channels,
            current_freq: spec.freq,
            current_bitstream: -1,
            current_iteration: -1,
            current_iteration_frames: 0,
        }))
    }
}

/// Translation of `UpdateVorbisStreamFormat()`.
fn update_vorbis_stream_format(stream: &AudioStream, spec: &AudioSpec) {
    static VORBIS_CHMAP_3: [i32; 6] = [0, 2, 1, -1, -1, -1]; // SDL=FL,FR,FC,LFE,RL,RR  VORBIS=FL,FC,FR
    static VORBIS_CHMAP_5: [i32; 6] = [0, 2, 1, 4, 5, -1]; // SDL=FL,FR,FC,LFE,RL,RR  VORBIS=FL,FC,FR,RL,RR
    static VORBIS_CHMAP_6: [i32; 6] = [0, 2, 1, 4, 5, 3]; // SDL=FL,FR,FC,LFE,RL,RR  VORBIS=FL,FC,FR,RL,RR,LFE
    static VORBIS_CHMAP_7: [i32; 7] = [0, 2, 1, 5, 6, 4, 3]; // SDL=FL,FR,FC,LFE,RC,SL,SR  VORBIS=FL,FC,FR,SL,SR,RC,LFE
    static VORBIS_CHMAP_8: [i32; 8] = [0, 2, 1, 6, 7, 4, 5, 3]; // SDL=FL,FR,FC,LFE,RL,RR,SL,SR  VORBIS=FL,FC,FR,SL,SR,RL,RR,LFE

    let mut bumped_spec = *spec;
    let mut chmap: Option<&[i32]> = None;
    match spec.channels {
        // 3 and 5 channel audio offers a Front Center channel, which SDL doesn't have until 5.1 (6 channels), so bump the spec
        //  to 5.1 and provide NULL planar channels where appropriate.
        // !!! FIXME: Since Tremor doesn't provide planar data, we just pass this through in the wrong channel order there,
        // !!! FIXME: which is dumb but who is using these formats in the first place, let alone with low-powered Tremor?
        3 => {
            bumped_spec.channels = 6;
            chmap = Some(&VORBIS_CHMAP_3);
        }
        5 => {
            bumped_spec.channels = 6;
            chmap = Some(&VORBIS_CHMAP_5);
        }

        6 => chmap = Some(&VORBIS_CHMAP_6),
        7 => chmap = Some(&VORBIS_CHMAP_7),
        8 => chmap = Some(&VORBIS_CHMAP_8),
        _ => {
            // no special mapping, pass through as-is.
        }
    }

    let _ = stream.set_format(Some(&bumped_spec), None);
    let _ = stream.set_input_channel_map(chmap);
}

impl TrackData for VorbisTrackData<'_> {
    /// Translation of `VORBIS_decode()`.
    fn decode(&mut self, stream: &AudioStream) -> bool {
        let mut bitstream = self.current_bitstream;

        let mut pcm_channels = 0usize;
        let mut amount = ov_read_float(
            &mut self.vf,
            Some(&mut pcm_channels),
            256,
            Some(&mut bitstream),
        ) as i32;
        if amount < 0 {
            let _ = set_ogg_vorbis_error("ov_read_float", amount); // (upstream sets the error; decode() can only report false)
            return false;
        }

        if bitstream != self.current_bitstream {
            // FIXME (upstream): only a link whose channel count or rate
            // differs from the last one's sets the stream's format and
            // channel map, so the first link's 3- and 5-channel audio
            // isn't padded out to 5.1, and its 6- to 8-channel audio isn't
            // reordered to SDL's channel order.
            if let Some(vi) = ov_info(&self.vf, -1) {
                // this _shouldn't_ be NULL, but if it is, we're just going on without it and hoping the stream format didn't change.
                if (self.current_channels != vi.channels) || (self.current_freq != vi.rate as i32) {
                    let spec = AudioSpec {
                        format: VORBIS_AUDIO_FORMAT,
                        channels: vi.channels,
                        freq: vi.rate as i32,
                    };
                    self.current_channels = vi.channels;
                    self.current_freq = vi.rate as i32;
                    update_vorbis_stream_format(stream, &spec);
                }
            }
            self.current_bitstream = bitstream;
        }

        if amount == 0 {
            return false; // EOF
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
            // FIXME (upstream): SDL_assert(current_iteration_frames <= loop->len)
            // here and below fails on damaged files (where a looping seek
            // doesn't land where it asked), so it's left out.
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
                    // FIXME (upstream): with current_iteration_frames back
                    // at 0, the check above sees the loop start again
                    // `loop.start` frames later, so a loop with a
                    // LOOPCOUNT plays forever.
                    self.current_iteration = -1;
                }
                self.current_iteration_frames = 0;
            }
        }

        if amount > 0 {
            // Note (upstream): the planar pointers point into libvorbis's
            // PCM buffers, which a looping seek above decodes into, so the
            // samples output when looping are from that seek's decoding
            // (and, if that seek went to another link, from freed memory:
            // FIXME (upstream); here they're from the new link's buffers).
            let n = amount as usize;
            let pcm = &self.vf.vd.pcm;
            let planes: Vec<Vec<u8>> = (0..self.current_channels.max(0) as usize)
                .map(|i| {
                    let Some(buf) = pcm.get(i) else {
                        return Vec::new();
                    };
                    let start = pcm_channels.min(buf.len());
                    let end = pcm_channels.saturating_add(n).min(buf.len());
                    buf[start..end]
                        .iter()
                        .flat_map(|s| s.to_ne_bytes())
                        .collect()
                })
                .collect();
            let bufs: Vec<Option<&[u8]>> = planes
                .iter()
                .map(|p| (p.len() == n * 4).then_some(&p[..]))
                .collect();
            let _ = stream.put_planar_data(&bufs, n);
            self.current_iteration_frames =
                self.current_iteration_frames.wrapping_add(amount as i64);
        }

        true // had more data to decode.
    }

    /// Translation of `VORBIS_seek()`.
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
                    frame.checked_div(loop_.len as u64).unwrap_or(0) as i64
                }; // decide what iteration of the loop we're on (stays at zero for infinite loops).
                frame = frame.checked_rem(loop_.len as u64).unwrap_or(frame); // drop iterations so we're an offset into the loop.
                final_iteration_frames = frame as i64;
                frame = frame.wrapping_add(loop_.start as u64); // convert back into physical frame index.
            } else {
                // past the loop point?
                debug_assert!(loop_.count > 0); // can't be infinite loop if we passed it.
                frame = frame.wrapping_sub(loop_.len.wrapping_mul(loop_.count) as u64);
                // drop the iterations to get the physical frame index.
            }
        }

        // !!! FIXME: I assume ov_raw_seek is faster if we're seeking to start, but I could be wrong.
        let rc = if frame == 0 {
            ov_raw_seek(&mut self.vf, 0)
        } else {
            ov_pcm_seek(&mut self.vf, frame as i64)
        };
        if rc != 0 {
            return Err(set_ogg_vorbis_error("ov_pcm_seek", rc));
        }

        self.current_iteration = final_iteration;
        self.current_iteration_frames = final_iteration_frames;

        Ok(())
    }
}

/// Translation of `MIX_Decoder_VORBIS`.
pub(crate) static DECODER: Decoder = Decoder {
    name: "VORBIS",
    init: None,
    init_audio: vorbis_init_audio,
    has_jump_to_order: false,
    quit: None,
};
