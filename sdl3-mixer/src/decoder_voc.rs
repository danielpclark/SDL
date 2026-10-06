// Rust translation of src/decoder_voc.c from SDL_mixer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Creative Voice (.VOC) files.

// https://moddingwiki.shikadi.net/wiki/VOC_Format
// https://www.manualslib.com/manual/547219/Creative-Sb0350.html?page=125#manual

use std::sync::Arc;

use sdl3::audio::{AudioFormat, AudioSpec, AudioStream};
use sdl3::error::{Error, Result};
use sdl3::io::{IoStream, IoWhence};
use sdl3::properties::Properties;
use sdl3::stdlib::iconv::iconv_string;

use crate::internal::{try_alloc, AudioData, Decoder, TrackData};
use crate::metadata_tags::cstring;
use crate::PROP_AUDIO_LOAD_IGNORE_LOOPS_BOOLEAN;

const VOC_TERM: u8 = 0;
const VOC_DATA: u8 = 1;
const VOC_CONT: u8 = 2;
const VOC_SILENCE: u8 = 3;
#[allow(dead_code)]
const VOC_MARKER: u8 = 4;
const VOC_TEXT: u8 = 5;
const VOC_LOOP: u8 = 6;
const VOC_LOOPEND: u8 = 7;
const VOC_EXTENDED: u8 = 8;
const VOC_DATA_16: u8 = 9;

/// Translation of `VOC_Block`.
#[derive(Clone, Copy, Default, Debug)]
struct VocBlock {
    loop_count: i32, // 0=data or silence block, >0=loop block of X loops, -1=infinite loop block, -2=endloop block.
    iopos: i64, // byte position in i/o stream of this block's data. Might be 0 for things like loop and silence blocks.
    spec: AudioSpec,
    frames: u64,
}

/// Translation of `VOC_AudioData`.
#[derive(Default)]
struct VocAudioData {
    blocks: Vec<VocBlock>,
}

/// Translation of `VOC_TrackData`.
struct VocTrackData<'a> {
    adata: Arc<VocAudioData>,
    io: IoStream<'a>,
    spec: AudioSpec,
    current_block: usize,
    frame_pos: u64,
    loop_start: Option<usize>,
    loop_count: i32,
    /// Frames pushed since the current loop started (to stop at an empty
    /// infinite loop; see `decode`).
    frames_in_loop: u64,
}

/// Translation of `AddVocBlock()`.
fn add_voc_block(adata: &mut VocAudioData) -> Option<&mut VocBlock> {
    if adata.blocks.try_reserve(1).is_err() {
        return None;
    }
    adata.blocks.push(VocBlock::default());
    adata.blocks.last_mut()
}

/// Translation of `AddVocDataBlock()`.
fn add_voc_data_block(adata: &mut VocAudioData, iopos: i64, spec: &AudioSpec, frames: u64) -> bool {
    if iopos < 0 {
        // SDL_TellIO failed?
        return false;
    }

    match add_voc_block(adata) {
        Some(block) => {
            block.iopos = iopos;
            block.loop_count = 0;
            block.spec = *spec;
            block.frames = frames;
            true
        }
        None => false,
    }
}

/// Translation of `AddVocLoopBlock()`.
fn add_voc_loop_block(adata: &mut VocAudioData, loop_count: i32) -> Option<usize> {
    let block = add_voc_block(adata)?;
    block.loop_count = loop_count;
    Some(adata.blocks.len() - 1)
}

fn voc_error() -> Error {
    Error::new("VOC: Couldn't read from datastream")
}

/// Translation of `ParseVocFile()`.
// this runs during VOC_audio_init to walk the whole .VOC for metadata and sanity checks.
fn parse_voc_file(
    io: &mut IoStream<'_>,
    adata: &mut VocAudioData,
    props: &Properties,
    spec: &mut AudioSpec,
    duration_frames: &mut i64,
) -> Result<()> {
    let ignore_loops = props
        .get_bool(PROP_AUDIO_LOAD_IGNORE_LOOPS_BOOLEAN)
        .unwrap_or(false);
    let mut total_frames: i64 = 0;
    let mut loop_start: Option<usize> = None;
    let mut loop_start_loop_count: i32 = 0;
    let mut loop_frames: i64 = 0;
    let mut text_count = 0;

    let mut pos = io.tell()?;

    let original_spec = *spec;
    let mut current_spec = *spec;
    spec.format = AudioFormat::UNKNOWN;

    loop {
        let mut block = [0u8; 1];
        let mut blen: u32 = 0;
        if io.read(&mut block) != 1 {
            break; // assume that's the end of the file.
        }
        let block = block[0];
        if block == VOC_TERM {
            break; // that's the (optional) end.
        } else if block != VOC_LOOPEND {
            // TERM and LOOPEND don't have a size field.
            let mut bits24 = [0u8; 3];
            if io.read(&mut bits24) != bits24.len() {
                return Err(voc_error());
            }
            // Size is a 24-bit value. Ugh.
            blen = (bits24[0] as u32) | ((bits24[1] as u32) << 8) | ((bits24[2] as u32) << 16);
        }

        pos += 1;
        if (block != VOC_TERM) && (block != VOC_LOOPEND) {
            pos += 3; // size fields on everything but VOC_TERM and VOC_LOOPEND.
        }

        match block {
            VOC_DATA => {
                let mut rateu8 = [0u8; 1];
                let mut codec = [0u8; 1];
                if io.read(&mut rateu8) != 1 {
                    return Err(voc_error());
                } else if io.read(&mut codec) != 1 {
                    return Err(voc_error());
                }
                let (rateu8, codec) = (rateu8[0], codec[0]);

                if (codec != 0) && (codec != 4) {
                    // !!! FIXME: there are other formats (adpcm, etc), but we don't support them at the moment.
                    return Err(Error::new("Unsupported VOC data format"));
                }

                current_spec.freq = 1000000 / (256 - rateu8 as i32);
                current_spec.channels = 1;
                current_spec.format = if codec == 0 {
                    AudioFormat::U8
                } else {
                    AudioFormat::S16LE
                };

                let framelen = current_spec.frame_size() as u32;
                let frames = (blen.wrapping_sub(2) / framelen) as u64;
                if !add_voc_data_block(adata, io.tell().unwrap_or(-1), &current_spec, frames) {
                    return Err(Error::new("VOC: Couldn't add a data block"));
                }

                if spec.format == AudioFormat::UNKNOWN {
                    // take the first thing as the "official" spec, but this can change later.
                    *spec = current_spec;
                }

                if total_frames != -1 {
                    loop_frames += frames as i64;
                }
            }

            VOC_DATA_16 => {
                let rate32 = io.read_u32_le()?;
                if rate32 == 0 {
                    return Err(Error::new("VOC sample rate is zero?"));
                }

                let mut bits = [0u8; 1];
                let mut channels = [0u8; 1];
                let mut codec = [0u8; 1];
                if io.read(&mut bits) != 1 {
                    return Err(voc_error());
                } else if (bits[0] != 8) && (bits[0] != 16) {
                    return Err(Error::new("Unsupported VOC data format"));
                } else if io.read(&mut channels) != 1 {
                    // I assume you have mono or stereo, but we'll let you go wild with whatever.
                    return Err(voc_error());
                } else if io.read(&mut codec) != 1 {
                    return Err(voc_error());
                }
                let (bits, channels, codec) = (bits[0], channels[0], codec[0]);
                if (codec != 0) && (codec != 4) {
                    // !!! FIXME: there are other formats (adpcm, etc), but we don't support them at the moment.
                    return Err(Error::new("Unsupported VOC data format"));
                } else if ((codec == 0) && (bits != 8)) || ((codec == 4) && (bits != 16)) {
                    return Err(Error::new("Corrupt VOC data"));
                }
                // FIXME (upstream): zero channels divide by zero (and crash) below.
                if channels == 0 {
                    return Err(Error::new("Corrupt VOC data"));
                }

                current_spec.freq = rate32 as i32;
                current_spec.channels = channels as i32;
                current_spec.format = if codec == 0 {
                    AudioFormat::U8
                } else {
                    AudioFormat::S16LE
                };

                let framelen = current_spec.frame_size() as u32;
                let frames = (blen.wrapping_sub(12) / framelen) as u64;
                if !add_voc_data_block(adata, io.tell().unwrap_or(-1), &current_spec, frames) {
                    return Err(Error::new("VOC: Couldn't add a data block"));
                }

                if spec.format == AudioFormat::UNKNOWN {
                    // take the first thing as the "official" spec, but this can change later.
                    *spec = current_spec;
                }

                if total_frames != -1 {
                    loop_frames += frames as i64;
                }
            }

            VOC_CONT => {
                if spec.format == AudioFormat::UNKNOWN {
                    return Err(Error::new(
                        "VOC continuation block before a data type is set.",
                    ));
                }

                let framelen = (current_spec.frame_size() as u32).max(1);
                let frames = (blen / framelen) as u64;
                if !add_voc_data_block(adata, io.tell().unwrap_or(-1), &current_spec, frames) {
                    return Err(Error::new("VOC: Couldn't add a data block"));
                }

                if total_frames != -1 {
                    loop_frames += frames as i64;
                }
            }

            VOC_SILENCE => {
                // it's okay if we haven't set an initial format yet, we'll know it when we get there later.
                // technically we could try to add this to an existing silence block, but in practice these are rare, and duplicate blocks probably more so.
                let frames = io.read_u16_le()?;
                if !add_voc_data_block(adata, 0, &current_spec, frames as u64 + 1) {
                    return Err(Error::new("VOC: Couldn't add a silence block"));
                }

                if total_frames != -1 {
                    loop_frames += frames as i64 + 1;
                }
            }

            VOC_LOOP => 'block: {
                if ignore_loops {
                    break 'block; // technically this can make a file with bogus loops load where it wouldn't otherwise, but good enough.
                }

                // !!! FIXME: can LOOP/LOOPEND sections nest? https://moddingwiki.shikadi.net/wiki/VOC_Format suggests no, saying LOOPEND goes back to _most recent_ LOOP start.
                if loop_start.is_some() {
                    return Err(Error::new("VOC has nested loop"));
                }
                let iterations = io.read_u16_le()?;

                let mut loop_count = -1;
                if iterations == 0xFFFF {
                    total_frames = -1; // it's infinite.
                } else {
                    loop_count = iterations as i32 + 1;
                }

                if total_frames != -1 {
                    total_frames += loop_frames; // add in anything that's accumulated until now.
                }

                loop_frames = 0;

                let Some(index) = add_voc_loop_block(adata, loop_count) else {
                    return Err(Error::new("VOC: Couldn't add a loop block"));
                };
                loop_start = Some(index);
                loop_start_loop_count = adata.blocks[index].loop_count;
            }

            VOC_LOOPEND => 'block: {
                if ignore_loops {
                    break 'block; // technically this can make a file with bogus loops load where it wouldn't otherwise, but good enough.
                }

                if loop_start.is_none() {
                    return Err(Error::new("VOC has a LOOPEND without a matching LOOP"));
                }

                if add_voc_loop_block(adata, -2).is_none() {
                    return Err(Error::new("VOC: Couldn't add a loop block"));
                }

                if total_frames != -1 {
                    total_frames += loop_frames * loop_start_loop_count as i64;
                }

                loop_start = None;
                loop_start_loop_count = 0;
                loop_frames = 0;
            }

            VOC_EXTENDED => {
                let rateu16 = io.read_u16_le()?;
                let mut codec = [0u8; 1];
                let mut channelsu8 = [0u8; 1];
                if io.read(&mut codec) != 1 {
                    return Err(voc_error());
                } else if (codec[0] != 0) && (codec[0] != 4) {
                    // !!! FIXME: there are other formats (adpcm, etc), but we don't support them at the moment.
                    return Err(Error::new("Unsupported VOC data format"));
                } else if io.read(&mut channelsu8) != 1 {
                    return Err(voc_error());
                }

                let channels = channelsu8[0] as i32 + 1;
                current_spec.freq = 256000000 / (channels * (65536 - rateu16 as i32));
                current_spec.channels = channels;
                current_spec.format = if codec[0] == 0 {
                    AudioFormat::U8
                } else {
                    AudioFormat::S16LE
                };

                if spec.format == AudioFormat::UNKNOWN {
                    *spec = current_spec;
                }
            }

            VOC_TEXT => {
                if let Ok(mut value) = try_alloc::<u8>(blen as usize) {
                    // oh well if we ran out of memory.
                    if io.read(&mut value) != blen as usize {
                        return Err(voc_error());
                    }
                    if let Ok(utf8) = iconv_string("UTF-8", "ISO-8859-1", &value) {
                        let key = format!("SDL_mixer.metadata.voc.text{text_count}");
                        let _ = props.set(&key, cstring(&utf8));
                        text_count += 1;
                    }
                }
            }

            // just skip these.
            //case VOC_MARKER:
            _ => {}
        }

        pos += blen as i64;
        io.seek(pos, IoWhence::Set)?;
    }

    if loop_start.is_some() {
        // !!! FIXME: should we just treat EOF as the loop end in this case?
        return Err(Error::new("VOC has a LOOP without a matching LOOPEND"));
    }

    if spec.format == AudioFormat::UNKNOWN {
        // theoretically this can happen if you only have VOC_SILENCE blocks. Set it to the original device format.
        *spec = original_spec;
    }

    if total_frames != -1 {
        total_frames += loop_frames;
    }

    *duration_frames = total_frames;

    Ok(())
}

/// Translation of `VOC_init_audio()`.
fn voc_init_audio(
    io: Option<&mut IoStream<'_>>,
    spec: &mut AudioSpec,
    props: &Properties,
    duration_frames: &mut i64,
) -> Result<Arc<dyn AudioData>> {
    let io = io.ok_or_else(|| Error::invalid_param("io"))?;

    // just load the bare minimum from the IOStream to verify it's a .VOC file.
    let mut magic = [0u8; 20];
    if io.read(&mut magic) != 20 {
        return Err(voc_error());
    } else if &magic != b"Creative Voice File\x1a" {
        return Err(Error::new("Not a VOC audio stream"));
    }

    // now jump to the data and prepare to parse everything out.
    let datablockpos = io.read_u16_le()?;
    io.seek(datablockpos as i64, IoWhence::Set)?;

    let mut adata = VocAudioData::default();
    parse_voc_file(io, &mut adata, props, spec, duration_frames)?;

    Ok(Arc::new(adata))
}

impl AudioData for VocAudioData {
    /// Translation of `VOC_init_track()`.
    fn init_track<'a>(
        self: Arc<Self>,
        io: Option<IoStream<'a>>,
        spec: &AudioSpec,
        _props: &Properties,
    ) -> Result<Box<dyn TrackData + 'a>> {
        let io = io.ok_or_else(|| Error::invalid_param("io"))?;
        Ok(Box::new(VocTrackData {
            adata: self,
            io,
            spec: *spec,
            current_block: 0,
            frame_pos: 0,
            loop_start: None,
            loop_count: 0,
            frames_in_loop: 0,
        }))
    }
}

impl VocTrackData<'_> {
    /// The endloop block's work, shared by `VOC_decode()` and `VOC_seek()`.
    fn end_loop(&mut self) {
        debug_assert!(self.loop_start.is_some());
        let start_block = self.loop_start.unwrap_or(self.current_block) + 1;
        if self.loop_count < 0 {
            // infinite loop
            self.current_block = start_block;
        } else if self.loop_count == 0 {
            // last iteration
            self.current_block += 1;
            self.loop_start = None;
        } else {
            self.loop_count -= 1;
            self.current_block = start_block;
        }
    }
}

impl TrackData for VocTrackData<'_> {
    /// Translation of `VOC_decode()`.
    fn decode(&mut self, stream: &AudioStream) -> bool {
        if self.current_block >= self.adata.blocks.len() {
            return false; // EOF.
        }

        let block = self.adata.blocks[self.current_block];

        if self.frame_pos == 0 {
            // starting a new block, see what we're doing...
            if block.loop_count == 0 {
                // it's data.
                if block.iopos != 0 {
                    // have to read actual file data from this block.
                    if self.io.seek(block.iopos, IoWhence::Set).ok() != Some(block.iopos) {
                        return false; // uhoh.
                    }
                }
                if self.spec != block.spec {
                    self.spec.format = AudioFormat::UNKNOWN; // we'll set it later.
                }
            } else if block.loop_count == -2 {
                // it's an endloop block.
                // FIXME (upstream): an infinite loop with no audio in it
                // makes the decoder return true forever without decoding
                // anything, hanging the caller; that's the end of the data
                // here (and in the patched C reference harness).
                if self.loop_count < 0 && self.frames_in_loop == 0 {
                    return false;
                }
                self.frames_in_loop = 0;
                self.end_loop();
                return true; // try again on new block.
            } else {
                // it's a loop block.
                debug_assert!(self.loop_start.is_none());
                self.loop_start = Some(self.current_block);
                self.loop_count = block.loop_count;
                self.frames_in_loop = 0;
                self.current_block += 1;
                return true; // try again on new block.
            }
        }

        // still here? Feed data.
        debug_assert!(self.frame_pos <= block.frames);
        let available = block.frames - self.frame_pos;
        let mut buffer = [0u8; 512 * 4];
        let framesize = block.spec.frame_size().max(1);
        let frames = available.min((buffer.len() / framesize) as u64);

        if frames == 0 {
            // finished this block.
            self.frame_pos = 0;
            self.current_block += 1;
            return true; // try again, there might be more data available.
        }

        if self.spec.format == AudioFormat::UNKNOWN {
            self.spec = block.spec;
            let _ = stream.set_format(Some(&self.spec), None);
        }

        if block.iopos == 0 {
            // zero position means write silence (you can't have a data block at position 0 because of headers, etc).
            // FIXME (upstream): this pushes one NULL channel buffer as
            // planar data, which pushes silence only for multichannel
            // audio: mono audio (all of SDL's own VOC data blocks) gets
            // nothing, as SDL_PutAudioStreamPlanarData passes a single
            // channel's NULL buffer to SDL_PutAudioStreamData, which fails.
            if stream.put_planar_data(&[None], frames as usize).is_ok() {
                // push silence to the stream.
                self.frames_in_loop += 1;
            }
            self.frame_pos += frames;
        } else {
            let total = frames as usize * framesize;
            let br = self.io.read(&mut buffer[..total]);
            let frames_read = br / framesize;
            if frames_read == 0 {
                return false; // uhoh.
            }
            let _ = stream.put_data(&buffer[..frames_read * framesize]);
            self.frame_pos += frames_read as u64;
            self.frames_in_loop += 1;
        }

        true
    }

    /// Translation of `VOC_seek()`.
    fn seek(&mut self, frame: u64) -> Result<()> {
        self.loop_start = None;
        self.loop_count = 0;
        self.frame_pos = 0;
        self.current_block = 0;
        self.frames_in_loop = 0;

        if frame == 0 {
            self.spec.format = AudioFormat::UNKNOWN; // we'll set it later.
            return Ok(()); // easy seek to start.
        }

        let num_blocks = self.adata.blocks.len();
        let mut framepos: u64 = 0;
        let mut loop_start_framepos: u64 = 0;
        while self.current_block < num_blocks {
            let block = self.adata.blocks[self.current_block];
            if block.loop_count == 0 {
                // it's data.
                let framesize = block.spec.frame_size();
                if frame < (framepos + block.frames) {
                    debug_assert!(frame >= framepos);
                    self.frame_pos = frame - framepos;
                    if block.iopos != 0 {
                        // have to read actual file data from this block.
                        self.io.seek(
                            block.iopos + (self.frame_pos as i64 * framesize as i64),
                            IoWhence::Set,
                        )?;
                    }
                    if self.spec != block.spec {
                        self.spec.format = AudioFormat::UNKNOWN; // we'll set it later.
                    }
                    return Ok(()); // we're ready!
                } else {
                    framepos += block.frames; // not there yet, keep walking through blocks.
                    self.current_block += 1;
                }
            } else if block.loop_count == -2 {
                // it's an endloop block.
                // (an infinite loop with no audio in it would walk forever; see decode.)
                if self.loop_count < 0 && framepos == loop_start_framepos {
                    break;
                }
                self.end_loop();
                loop_start_framepos = framepos;
                // try again on new block.
            } else {
                // it's a loop block.
                debug_assert!(self.loop_start.is_none());
                self.loop_start = Some(self.current_block);
                self.loop_count = block.loop_count;
                self.current_block += 1;
                loop_start_framepos = framepos;
                // try again on new block.
            }
        }

        // still here? Seek was past EOF.
        Err(Error::new("VOC: seek past end of data"))
    }
}

/// Translation of `MIX_Decoder_VOC`.
pub(crate) static DECODER: Decoder = Decoder {
    name: "VOC",
    init: None,
    init_audio: voc_init_audio,
    has_jump_to_order: false,
    quit: None,
};
