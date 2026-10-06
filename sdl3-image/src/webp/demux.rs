// Rust translation of src/demux/demux.c and src/webp/demux.h from libwebp
// (https://chromium.googlesource.com/webm/libwebp, as SDL_image's
// external/libwebp pins it).
// Copyright 2012 Google Inc. All Rights Reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! WebP container demux: the frames and chunks of a WebP file.
//!
//! The demuxer owns the data it parses here (upstream borrows the caller's
//! `WebPData`, which must outlive it), the frames and chunks lists are
//! vectors, and an iterator's frame or chunk is a range of the data (read
//! with [`WebPDemuxer::bytes`]). The iterators' `private_` demuxer pointer
//! is the demuxer passed to the iteration functions, with a flag for
//! whether the iterator was set up by them. The animation decoder
//! (`anim_decode.c`) is not translated: SDL_image composites the frames
//! itself.

#![allow(dead_code)] // (the whole demux API, of which SDL_image uses part)

use crate::webp::dec::webp_dec::webp_get_features;
use crate::webp::decode::{
    mkfourcc, VP8StatusCode, WebPBitstreamFeatures, WebPMuxAnimBlend, WebPMuxAnimDispose,
    ALL_VALID_FLAGS, ALPHA_FLAG, ANIMATION_FLAG, ANIM_CHUNK_SIZE, ANMF_CHUNK_SIZE,
    CHUNK_HEADER_SIZE, CHUNK_SIZE_BYTES, EXIF_FLAG, ICCP_FLAG, MAX_CHUNK_PAYLOAD,
    MAX_IMAGE_AREA, RIFF_HEADER_SIZE, TAG_SIZE, VP8X_CHUNK_SIZE, XMP_FLAG,
};
use crate::webp::utils::{get_le16, get_le24, get_le32};

const DMUX_MAJ_VERSION: i32 = 1;
const DMUX_MIN_VERSION: i32 = 3;
const DMUX_REV_VERSION: i32 = 2;

/// Translation of `MemBuffer` (`buf_` is the demuxer's data).
#[derive(Clone, Copy, Default)]
struct MemBuffer {
    /// start location of the data
    start: usize,
    /// end location
    end: usize,
    /// riff chunk end location, can be > end_.
    riff_end: usize,
    /// size of the buffer
    buf_size: usize,
}

/// Translation of `ChunkData`.
#[derive(Clone, Copy, Default)]
struct ChunkData {
    offset: usize,
    size: usize,
}

/// Translation of `struct Frame`.
#[derive(Clone, Copy, Default)]
struct Frame {
    x_offset: i32,
    y_offset: i32,
    width: i32,
    height: i32,
    has_alpha: bool,
    duration: i32,
    dispose_method: WebPMuxAnimDispose,
    blend_method: WebPMuxAnimBlend,
    frame_num: i32,
    /// img_components_ contains a full image.
    complete: bool,
    /// 0=VP8{,L} 1=ALPH
    img_components: [ChunkData; 2],
}

/// Translation of `struct Chunk`.
#[derive(Clone, Copy, Default)]
struct Chunk {
    data: ChunkData,
}

/// Translation of `WebPDemuxState`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum WebPDemuxState {
    /// An error occurred while parsing.
    ParseError = -1,
    /// Not enough data to parse full header.
    ParsingHeader = 0,
    /// Header parsing complete, data may be available.
    ParsedHeader = 1,
    /// Entire file has been parsed.
    Done = 2,
}

/// Translation of `WebPFormatFeature`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum WebPFormatFeature {
    /// bit-wise combination of WebPFeatureFlags corresponding to the 'VP8X'
    /// chunk (if present).
    FormatFlags,
    CanvasWidth,
    CanvasHeight,
    /// only relevant for animated file
    LoopCount,
    /// idem.
    BackgroundColor,
    /// Number of frames present in the demux object. In case of a partial
    /// demux, this is the number of frames seen so far, with the last frame
    /// possibly being partial.
    FrameCount,
}

/// Translation of `struct WebPDemuxer`.
pub(crate) struct WebPDemuxer {
    data: Vec<u8>,
    mem: MemBuffer,
    state: WebPDemuxState,
    is_ext_format: bool,
    feature_flags: u32,
    canvas_width: i32,
    canvas_height: i32,
    loop_count: i32,
    bgcolor: u32,
    num_frames: i32,
    frames: Vec<Frame>,
    /// non-image chunks
    chunks: Vec<Chunk>,
}

/// Translation of `ParseStatus`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ParseStatus {
    Ok,
    NeedMoreData,
    Error,
}

/// Iteration: translation of `struct WebPIterator`.
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct WebPIterator {
    pub(crate) frame_num: i32,
    /// equivalent to WEBP_FF_FRAME_COUNT.
    pub(crate) num_frames: i32,
    /// offset relative to the canvas.
    pub(crate) x_offset: i32,
    pub(crate) y_offset: i32,
    /// dimensions of this frame.
    pub(crate) width: i32,
    pub(crate) height: i32,
    /// display duration in milliseconds.
    pub(crate) duration: i32,
    /// dispose method for the frame.
    pub(crate) dispose_method: WebPMuxAnimDispose,
    /// true if 'fragment' contains a full frame. partial images may still be
    /// decoded with the WebP incremental decoder.
    pub(crate) complete: bool,
    /// The frame given by 'frame_num' (its offset and size in the data).
    /// Note for historical reasons this is called a fragment.
    pub(crate) fragment: (usize, usize),
    /// True if the frame contains transparency.
    pub(crate) has_alpha: bool,
    /// Blend operation for the frame.
    pub(crate) blend_method: WebPMuxAnimBlend,
    /// whether `private_` (the demuxer) is set
    private: bool,
}

/// Translation of `struct WebPChunkIterator`.
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct WebPChunkIterator {
    /// The current and total number of chunks with the fourcc given to
    /// WebPDemuxGetChunk().
    pub(crate) chunk_num: i32,
    pub(crate) num_chunks: i32,
    /// The payload of the chunk (its offset and size in the data).
    pub(crate) chunk: (usize, usize),
    /// whether `private_` (the demuxer) is set
    private: bool,
}

/// The master chunks: translation of `kMasterChunks[]` (their parse and
/// validity functions).
#[derive(Clone, Copy)]
enum MasterChunk {
    SingleImage,
    Vp8x,
}

static K_MASTER_CHUNKS: [(&[u8; 4], MasterChunk); 3] = [
    (b"VP8 ", MasterChunk::SingleImage),
    (b"VP8L", MasterChunk::SingleImage),
    (b"VP8X", MasterChunk::Vp8x),
];

//------------------------------------------------------------------------------

/// Translation of `WebPGetDemuxVersion()`.
pub(crate) fn webp_get_demux_version() -> i32 {
    (DMUX_MAJ_VERSION << 16) | (DMUX_MIN_VERSION << 8) | DMUX_REV_VERSION
}

// -----------------------------------------------------------------------------
// MemBuffer

/// Translation of `RemapMemBuffer()`.
fn remap_mem_buffer(mem: &mut MemBuffer, size: usize) -> bool {
    if size < mem.buf_size {
        return false; // can't remap to a shorter buffer!
    }

    mem.end = size;
    mem.buf_size = size;
    true
}

/// Translation of `InitMemBuffer()`.
fn init_mem_buffer(mem: &mut MemBuffer, size: usize) -> bool {
    *mem = MemBuffer::default();
    remap_mem_buffer(mem, size)
}

/// Return the remaining data size available in 'mem'. Translation of
/// `MemDataSize()`.
fn mem_data_size(mem: &MemBuffer) -> usize {
    mem.end - mem.start
}

/// Return true if 'size' exceeds the end of the RIFF chunk. Translation of
/// `SizeIsInvalid()`.
fn size_is_invalid(mem: &MemBuffer, size: usize) -> bool {
    size > mem.riff_end - mem.start
}

/// Translation of `Skip()`.
fn skip(mem: &mut MemBuffer, size: usize) {
    mem.start += size;
}

/// Translation of `Rewind()`.
fn rewind(mem: &mut MemBuffer, size: usize) {
    mem.start -= size;
}

impl WebPDemuxer {
    /// The bytes at `range` (an offset and a size) of the data: a frame's
    /// fragment, or a chunk's payload.
    pub(crate) fn bytes(&self, range: (usize, usize)) -> &[u8] {
        &self.data[range.0..range.0 + range.1]
    }

    /// The data, as `GetBuffer()` returns it.
    fn get_buffer(&self) -> &[u8] {
        &self.data[self.mem.start..]
    }

    /// Read from 'mem' and skip the read bytes. Translation of `ReadByte()`.
    fn read_byte(&mut self) -> u8 {
        let byte = self.data[self.mem.start];
        skip(&mut self.mem, 1);
        byte
    }

    /// Translation of `ReadLE16s()`.
    fn read_le16s(&mut self) -> i32 {
        let val = get_le16(&self.data[self.mem.start..]);
        skip(&mut self.mem, 2);
        val
    }

    /// Translation of `ReadLE24s()`.
    fn read_le24s(&mut self) -> i32 {
        let val = get_le24(&self.data[self.mem.start..]);
        skip(&mut self.mem, 3);
        val
    }

    /// Translation of `ReadLE32()`.
    fn read_le32(&mut self) -> u32 {
        let val = get_le32(&self.data[self.mem.start..]);
        skip(&mut self.mem, 4);
        val
    }
}

// -----------------------------------------------------------------------------
// Secondary chunk parsing

/// Translation of `AddChunk()`.
fn add_chunk(dmux: &mut WebPDemuxer, chunk: Chunk) {
    dmux.chunks.push(chunk);
}

/// Add a frame to the end of the list, ensuring the last frame is complete.
/// Returns true on success, false otherwise. Translation of `AddFrame()`.
fn add_frame(dmux: &mut WebPDemuxer, frame: Frame) -> bool {
    if let Some(last_frame) = dmux.frames.last() {
        if !last_frame.complete {
            return false;
        }
    }

    dmux.frames.push(frame);
    true
}

/// Translation of `SetFrameInfo()`.
fn set_frame_info(
    start_offset: usize,
    size: usize,
    frame_num: i32,
    complete: bool,
    features: &WebPBitstreamFeatures,
    frame: &mut Frame,
) {
    frame.img_components[0].offset = start_offset;
    frame.img_components[0].size = size;
    frame.width = features.width;
    frame.height = features.height;
    frame.has_alpha |= features.has_alpha;
    frame.frame_num = frame_num;
    frame.complete = complete;
}

/// Store image bearing chunks to 'frame'. 'min_size' is an optional size
/// requirement, it may be zero. Translation of `StoreFrame()`.
fn store_frame(dmux: &mut WebPDemuxer, frame_num: i32, min_size: u32, frame: &mut Frame) -> ParseStatus {
    let mut alpha_chunks = 0;
    let mut image_chunks = 0;
    let mut done = mem_data_size(&dmux.mem) < CHUNK_HEADER_SIZE
        || mem_data_size(&dmux.mem) < min_size as usize;
    let mut status = ParseStatus::Ok;

    if done {
        return ParseStatus::NeedMoreData;
    }

    loop {
        let chunk_start_offset = dmux.mem.start;
        let fourcc = dmux.read_le32();
        let payload_size = dmux.read_le32();

        if payload_size > MAX_CHUNK_PAYLOAD {
            return ParseStatus::Error;
        }

        let payload_size_padded = payload_size + (payload_size & 1);
        let payload_available = if payload_size_padded as usize > mem_data_size(&dmux.mem) {
            mem_data_size(&dmux.mem)
        } else {
            payload_size_padded as usize
        };
        let chunk_size = CHUNK_HEADER_SIZE + payload_available;
        if size_is_invalid(&dmux.mem, payload_size_padded as usize) {
            return ParseStatus::Error;
        }
        if payload_size_padded as usize > mem_data_size(&dmux.mem) {
            status = ParseStatus::NeedMoreData;
        }

        let mut goto_done = false;
        if fourcc == mkfourcc(b"ALPH") {
            if alpha_chunks == 0 {
                alpha_chunks += 1;
                frame.img_components[1].offset = chunk_start_offset;
                frame.img_components[1].size = chunk_size;
                frame.has_alpha = true;
                frame.frame_num = frame_num;
                skip(&mut dmux.mem, payload_available);
            } else {
                goto_done = true;
            }
        } else if fourcc == mkfourcc(b"VP8L") || fourcc == mkfourcc(b"VP8 ") {
            if fourcc == mkfourcc(b"VP8L") && alpha_chunks > 0 {
                return ParseStatus::Error; // VP8L has its own alpha
            }
            // fall through
            if image_chunks == 0 {
                // Extract the bitstream features, tolerating failures when the data
                // is incomplete.
                let mut features = WebPBitstreamFeatures::default();
                let vp8_status = webp_get_features(
                    &dmux.data[chunk_start_offset..chunk_start_offset + chunk_size],
                    &mut features,
                );
                if status == ParseStatus::NeedMoreData
                    && vp8_status == VP8StatusCode::NotEnoughData
                {
                    return ParseStatus::NeedMoreData;
                } else if vp8_status != VP8StatusCode::Ok {
                    // We have enough data, and yet WebPGetFeatures() failed.
                    return ParseStatus::Error;
                }
                image_chunks += 1;
                set_frame_info(
                    chunk_start_offset,
                    chunk_size,
                    frame_num,
                    status == ParseStatus::Ok,
                    &features,
                    frame,
                );
                skip(&mut dmux.mem, payload_available);
            } else {
                goto_done = true;
            }
        } else {
            goto_done = true;
        }
        if goto_done {
            // Done:
            // Restore fourcc/size when moving up one level in parsing.
            rewind(&mut dmux.mem, CHUNK_HEADER_SIZE);
            done = true;
        }

        if dmux.mem.start == dmux.mem.riff_end {
            done = true;
        } else if mem_data_size(&dmux.mem) < CHUNK_HEADER_SIZE {
            status = ParseStatus::NeedMoreData;
        }
        if done || status != ParseStatus::Ok {
            break;
        }
    }

    status
}

/// Creates a new Frame if 'actual_size' is within bounds and 'mem' contains
/// enough data ('min_size') to parse the payload.
/// Returns PARSE_OK on success with *frame pointing to the new Frame.
/// Returns PARSE_NEED_MORE_DATA with insufficient data, PARSE_ERROR
/// otherwise. Translation of `NewFrame()`.
fn new_frame(mem: &MemBuffer, min_size: u32, actual_size: u32) -> Result<Frame, ParseStatus> {
    if size_is_invalid(mem, min_size as usize) {
        return Err(ParseStatus::Error);
    }
    if actual_size < min_size {
        return Err(ParseStatus::Error);
    }
    if mem_data_size(mem) < min_size as usize {
        return Err(ParseStatus::NeedMoreData);
    }

    Ok(Frame::default())
}

/// Parse a 'ANMF' chunk and any image bearing chunks that immediately
/// follow. 'frame_chunk_size' is the previously validated, padded chunk
/// size. Translation of `ParseAnimationFrame()`.
fn parse_animation_frame(dmux: &mut WebPDemuxer, frame_chunk_size: u32) -> ParseStatus {
    let is_animation = dmux.feature_flags & ANIMATION_FLAG != 0;
    let anmf_payload_size = frame_chunk_size.wrapping_sub(ANMF_CHUNK_SIZE);
    let mut frame = match new_frame(&dmux.mem, ANMF_CHUNK_SIZE, frame_chunk_size) {
        Ok(frame) => frame,
        Err(status) => return status,
    };

    frame.x_offset = 2 * dmux.read_le24s();
    frame.y_offset = 2 * dmux.read_le24s();
    frame.width = 1 + dmux.read_le24s();
    frame.height = 1 + dmux.read_le24s();
    frame.duration = dmux.read_le24s();
    let bits = dmux.read_byte();
    frame.dispose_method = if bits & 1 != 0 {
        WebPMuxAnimDispose::Background
    } else {
        WebPMuxAnimDispose::None
    };
    frame.blend_method = if bits & 2 != 0 {
        WebPMuxAnimBlend::NoBlend
    } else {
        WebPMuxAnimBlend::Blend
    };
    if frame.width as u64 * frame.height as u64 >= MAX_IMAGE_AREA {
        return ParseStatus::Error;
    }

    // Store a frame only if the animation flag is set there is some data for
    // this frame is available.
    let start_offset = dmux.mem.start;
    let mut status = store_frame(dmux, dmux.num_frames + 1, anmf_payload_size, &mut frame);
    if status != ParseStatus::Error && dmux.mem.start - start_offset > anmf_payload_size as usize {
        status = ParseStatus::Error;
    }
    if status != ParseStatus::Error && is_animation && frame.frame_num > 0 {
        if add_frame(dmux, frame) {
            dmux.num_frames += 1;
        } else {
            status = ParseStatus::Error;
        }
    }

    status
}

/// General chunk storage, starting with the header at 'start_offset',
/// allowing the user to request the payload via a fourcc string. 'size'
/// includes the header and the unpadded payload size. Returns true on
/// success, false otherwise. Translation of `StoreChunk()`.
fn store_chunk(dmux: &mut WebPDemuxer, start_offset: usize, size: u32) -> bool {
    let chunk = Chunk {
        data: ChunkData {
            offset: start_offset,
            size: size as usize,
        },
    };
    add_chunk(dmux, chunk);
    true
}

// -----------------------------------------------------------------------------
// Primary chunk parsing

/// Translation of `ReadHeader()`.
fn read_header(data: &[u8], mem: &mut MemBuffer) -> ParseStatus {
    let min_size = RIFF_HEADER_SIZE + CHUNK_HEADER_SIZE;

    // Basic file level validation.
    if mem_data_size(mem) < min_size {
        return ParseStatus::NeedMoreData;
    }
    let buf = &data[mem.start..];
    if &buf[..CHUNK_SIZE_BYTES] != b"RIFF"
        || &buf[CHUNK_HEADER_SIZE..CHUNK_HEADER_SIZE + CHUNK_SIZE_BYTES] != b"WEBP"
    {
        return ParseStatus::Error;
    }

    let riff_size = get_le32(&buf[TAG_SIZE..]);
    if (riff_size as usize) < CHUNK_HEADER_SIZE {
        return ParseStatus::Error;
    }
    if riff_size > MAX_CHUNK_PAYLOAD {
        return ParseStatus::Error;
    }

    // There's no point in reading past the end of the RIFF chunk
    mem.riff_end = riff_size as usize + CHUNK_HEADER_SIZE;
    if mem.buf_size > mem.riff_end {
        mem.buf_size = mem.riff_end;
        mem.end = mem.riff_end;
    }

    skip(mem, RIFF_HEADER_SIZE);
    ParseStatus::Ok
}

/// Translation of `ParseSingleImage()`.
fn parse_single_image(dmux: &mut WebPDemuxer) -> ParseStatus {
    let min_size = CHUNK_HEADER_SIZE;

    if !dmux.frames.is_empty() {
        return ParseStatus::Error;
    }
    if size_is_invalid(&dmux.mem, min_size) {
        return ParseStatus::Error;
    }
    if mem_data_size(&dmux.mem) < min_size {
        return ParseStatus::NeedMoreData;
    }

    let mut frame = Frame::default();

    // For the single image case we allow parsing of a partial frame, so no
    // minimum size is imposed here.
    let mut status = store_frame(dmux, 1, 0, &mut frame);
    if status != ParseStatus::Error {
        let has_alpha = dmux.feature_flags & ALPHA_FLAG != 0;
        // Clear any alpha when the alpha flag is missing.
        if !has_alpha && frame.img_components[1].size > 0 {
            frame.img_components[1].offset = 0;
            frame.img_components[1].size = 0;
            frame.has_alpha = false;
        }

        // Use the frame width/height as the canvas values for non-vp8x files.
        // Also, set ALPHA_FLAG if this is a lossless image with alpha.
        if !dmux.is_ext_format && frame.width > 0 && frame.height > 0 {
            dmux.state = WebPDemuxState::ParsedHeader;
            dmux.canvas_width = frame.width;
            dmux.canvas_height = frame.height;
            dmux.feature_flags |= if frame.has_alpha { ALPHA_FLAG } else { 0 };
        }
        if !add_frame(dmux, frame) {
            status = ParseStatus::Error; // last frame was left incomplete
        } else {
            dmux.num_frames = 1;
        }
    }

    status
}

/// Translation of `ParseVP8XChunks()`.
fn parse_vp8x_chunks(dmux: &mut WebPDemuxer) -> ParseStatus {
    let is_animation = dmux.feature_flags & ANIMATION_FLAG != 0;
    let mut anim_chunks = 0;
    let mut status = ParseStatus::Ok;

    loop {
        let mut store = true;
        let chunk_start_offset = dmux.mem.start;
        let fourcc = dmux.read_le32();
        let chunk_size = dmux.read_le32();

        if chunk_size > MAX_CHUNK_PAYLOAD {
            return ParseStatus::Error;
        }

        let chunk_size_padded = chunk_size + (chunk_size & 1);
        if size_is_invalid(&dmux.mem, chunk_size_padded as usize) {
            return ParseStatus::Error;
        }

        let mut goto_skip = false;
        if fourcc == mkfourcc(b"VP8X") {
            return ParseStatus::Error;
        } else if fourcc == mkfourcc(b"ALPH")
            || fourcc == mkfourcc(b"VP8 ")
            || fourcc == mkfourcc(b"VP8L")
        {
            // check that this isn't an animation (all frames should be in an ANMF).
            if anim_chunks > 0 || is_animation {
                return ParseStatus::Error;
            }

            rewind(&mut dmux.mem, CHUNK_HEADER_SIZE);
            status = parse_single_image(dmux);
        } else if fourcc == mkfourcc(b"ANIM") {
            if chunk_size_padded < ANIM_CHUNK_SIZE {
                return ParseStatus::Error;
            }

            if mem_data_size(&dmux.mem) < chunk_size_padded as usize {
                status = ParseStatus::NeedMoreData;
            } else if anim_chunks == 0 {
                anim_chunks += 1;
                dmux.bgcolor = dmux.read_le32();
                dmux.loop_count = dmux.read_le16s();
                skip(&mut dmux.mem, (chunk_size_padded - ANIM_CHUNK_SIZE) as usize);
            } else {
                store = false;
                goto_skip = true;
            }
        } else if fourcc == mkfourcc(b"ANMF") {
            if anim_chunks == 0 {
                return ParseStatus::Error; // 'ANIM' precedes frames.
            }
            status = parse_animation_frame(dmux, chunk_size_padded);
        } else if fourcc == mkfourcc(b"ICCP") {
            store = dmux.feature_flags & ICCP_FLAG != 0;
            goto_skip = true;
        } else if fourcc == mkfourcc(b"EXIF") {
            store = dmux.feature_flags & EXIF_FLAG != 0;
            goto_skip = true;
        } else if fourcc == mkfourcc(b"XMP ") {
            store = dmux.feature_flags & XMP_FLAG != 0;
            goto_skip = true;
        } else {
            goto_skip = true;
        }
        if goto_skip {
            // Skip:
            if chunk_size_padded as usize <= mem_data_size(&dmux.mem) {
                if store {
                    // Store only the chunk header and unpadded size as only the payload
                    // will be returned to the user.
                    if !store_chunk(
                        dmux,
                        chunk_start_offset,
                        CHUNK_HEADER_SIZE as u32 + chunk_size,
                    ) {
                        return ParseStatus::Error;
                    }
                }
                skip(&mut dmux.mem, chunk_size_padded as usize);
            } else {
                status = ParseStatus::NeedMoreData;
            }
        }

        if dmux.mem.start == dmux.mem.riff_end {
            break;
        } else if mem_data_size(&dmux.mem) < CHUNK_HEADER_SIZE {
            status = ParseStatus::NeedMoreData;
        }
        if status != ParseStatus::Ok {
            break;
        }
    }

    status
}

/// Translation of `ParseVP8X()`.
fn parse_vp8x(dmux: &mut WebPDemuxer) -> ParseStatus {
    if mem_data_size(&dmux.mem) < CHUNK_HEADER_SIZE {
        return ParseStatus::NeedMoreData;
    }

    dmux.is_ext_format = true;
    skip(&mut dmux.mem, TAG_SIZE); // VP8X
    let mut vp8x_size = dmux.read_le32();
    if vp8x_size > MAX_CHUNK_PAYLOAD {
        return ParseStatus::Error;
    }
    if vp8x_size < VP8X_CHUNK_SIZE {
        return ParseStatus::Error;
    }
    vp8x_size += vp8x_size & 1;
    if size_is_invalid(&dmux.mem, vp8x_size as usize) {
        return ParseStatus::Error;
    }
    if mem_data_size(&dmux.mem) < vp8x_size as usize {
        return ParseStatus::NeedMoreData;
    }

    dmux.feature_flags = dmux.read_byte() as u32;
    skip(&mut dmux.mem, 3); // Reserved.
    dmux.canvas_width = 1 + dmux.read_le24s();
    dmux.canvas_height = 1 + dmux.read_le24s();
    if dmux.canvas_width as u64 * dmux.canvas_height as u64 >= MAX_IMAGE_AREA {
        return ParseStatus::Error; // image final dimension is too large
    }
    skip(&mut dmux.mem, (vp8x_size - VP8X_CHUNK_SIZE) as usize); // skip any trailing data.
    dmux.state = WebPDemuxState::ParsedHeader;

    if size_is_invalid(&dmux.mem, CHUNK_HEADER_SIZE) {
        return ParseStatus::Error;
    }
    if mem_data_size(&dmux.mem) < CHUNK_HEADER_SIZE {
        return ParseStatus::NeedMoreData;
    }

    parse_vp8x_chunks(dmux)
}

// -----------------------------------------------------------------------------
// Format validation

/// Translation of `IsValidSimpleFormat()`.
fn is_valid_simple_format(dmux: &WebPDemuxer) -> bool {
    let frame = dmux.frames.first();
    if dmux.state == WebPDemuxState::ParsingHeader {
        return true;
    }

    if dmux.canvas_width <= 0 || dmux.canvas_height <= 0 {
        return false;
    }
    if dmux.state == WebPDemuxState::Done && frame.is_none() {
        return false;
    }
    // (a header parsed without a frame can't happen here: ParseSingleImage()
    // sets the state only with the frame it adds)
    let Some(frame) = frame else {
        return false;
    };

    if frame.width <= 0 || frame.height <= 0 {
        return false;
    }
    true
}

/// If 'exact' is true, check that the image resolution matches the canvas.
/// If 'exact' is false, check that the x/y offsets do not exceed the
/// canvas. Translation of `CheckFrameBounds()`.
fn check_frame_bounds(frame: &Frame, exact: bool, canvas_width: i32, canvas_height: i32) -> bool {
    if exact {
        if frame.x_offset != 0 || frame.y_offset != 0 {
            return false;
        }
        if frame.width != canvas_width || frame.height != canvas_height {
            return false;
        }
    } else {
        if frame.x_offset < 0 || frame.y_offset < 0 {
            return false;
        }
        if frame.width + frame.x_offset > canvas_width {
            return false;
        }
        if frame.height + frame.y_offset > canvas_height {
            return false;
        }
    }
    true
}

/// Translation of `IsValidExtendedFormat()`.
fn is_valid_extended_format(dmux: &WebPDemuxer) -> bool {
    let is_animation = dmux.feature_flags & ANIMATION_FLAG != 0;

    if dmux.state == WebPDemuxState::ParsingHeader {
        return true;
    }

    if dmux.canvas_width <= 0 || dmux.canvas_height <= 0 {
        return false;
    }
    if dmux.loop_count < 0 {
        return false;
    }
    if dmux.state == WebPDemuxState::Done && dmux.frames.is_empty() {
        return false;
    }
    if dmux.feature_flags & !ALL_VALID_FLAGS != 0 {
        return false; // invalid bitstream
    }

    let mut i = 0;
    while i < dmux.frames.len() {
        let cur_frame_set = dmux.frames[i].frame_num;

        // Check frame properties.
        while i < dmux.frames.len() && dmux.frames[i].frame_num == cur_frame_set {
            let f = &dmux.frames[i];
            let image = &f.img_components[0];
            let alpha = &f.img_components[1];

            if !is_animation && f.frame_num > 1 {
                return false;
            }

            if f.complete {
                if alpha.size == 0 && image.size == 0 {
                    return false;
                }
                // Ensure alpha precedes image bitstream.
                if alpha.size > 0 && alpha.offset > image.offset {
                    return false;
                }

                if f.width <= 0 || f.height <= 0 {
                    return false;
                }
            } else {
                // There shouldn't be a partial frame in a complete file.
                if dmux.state == WebPDemuxState::Done {
                    return false;
                }

                // Ensure alpha precedes image bitstream.
                if alpha.size > 0 && image.size > 0 && alpha.offset > image.offset {
                    return false;
                }
                // There shouldn't be any frames after an incomplete one.
                if i + 1 < dmux.frames.len() {
                    return false;
                }
            }

            if f.width > 0
                && f.height > 0
                && !check_frame_bounds(f, !is_animation, dmux.canvas_width, dmux.canvas_height)
            {
                return false;
            }
            i += 1;
        }
    }
    true
}

// -----------------------------------------------------------------------------
// WebPDemuxer object

/// Translation of `InitDemux()`.
fn init_demux(data: Vec<u8>, mem: &MemBuffer) -> WebPDemuxer {
    WebPDemuxer {
        data,
        mem: *mem,
        state: WebPDemuxState::ParsingHeader,
        is_ext_format: false,
        feature_flags: 0,
        canvas_width: -1,
        canvas_height: -1,
        loop_count: 1,
        bgcolor: 0xFFFFFFFF, // White background by default.
        num_frames: 0,
        frames: Vec::new(),
        chunks: Vec::new(),
    }
}

/// Translation of `CreateRawImageDemuxer()`.
fn create_raw_image_demuxer(
    data: Vec<u8>,
    mem: &MemBuffer,
) -> Result<WebPDemuxer, (ParseStatus, Vec<u8>)> {
    let mut features = WebPBitstreamFeatures::default();
    let status = webp_get_features(&data[..mem.buf_size], &mut features);
    if status != VP8StatusCode::Ok {
        return Err((
            if status == VP8StatusCode::NotEnoughData {
                ParseStatus::NeedMoreData
            } else {
                ParseStatus::Error
            },
            data,
        ));
    }

    {
        let mut dmux = init_demux(data, mem);
        let mut frame = Frame::default();
        set_frame_info(0, mem.buf_size, 1 /*frame_num*/, true /*complete*/, &features, &mut frame);
        if !add_frame(&mut dmux, frame) {
            return Err((ParseStatus::Error, dmux.data));
        }
        dmux.state = WebPDemuxState::Done;
        dmux.canvas_width = frame.width;
        dmux.canvas_height = frame.height;
        dmux.feature_flags |= if frame.has_alpha { ALPHA_FLAG } else { 0 };
        dmux.num_frames = 1;
        debug_assert!(is_valid_simple_format(&dmux));
        Ok(dmux)
    }
}

/// Parses the full WebP file given by 'data' (which the demuxer keeps).
/// For single images the WebP file header alone or the file header and
/// the chunk header may be absent. Returns a WebPDemuxer object on
/// successful parse, `None` otherwise. With `allow_partial`, a partial
/// file (whose state, as `state` then reports, is
/// `WEBP_DEMUX_PARSING_HEADER` or `WEBP_DEMUX_PARSED_HEADER`) is
/// accepted. Translation of `WebPDemuxInternal()` (`WebPDemux()` and
/// `WebPDemuxPartial()`).
pub(crate) fn webp_demux_internal(
    data: Vec<u8>,
    allow_partial: bool,
    mut state: Option<&mut WebPDemuxState>,
) -> Option<WebPDemuxer> {
    let mut mem = MemBuffer::default();

    if let Some(state) = state.as_deref_mut() {
        *state = WebPDemuxState::ParseError;
    }

    if data.is_empty() {
        return None;
    }

    if !init_mem_buffer(&mut mem, data.len()) {
        return None;
    }
    let mut status = read_header(&data, &mut mem);
    if status != ParseStatus::Ok {
        // If parsing of the webp file header fails attempt to handle a raw
        // VP8/VP8L frame. Note 'allow_partial' is ignored in this case.
        if status == ParseStatus::Error {
            match create_raw_image_demuxer(data, &mem) {
                Ok(dmux) => {
                    if let Some(state) = state.as_deref_mut() {
                        *state = WebPDemuxState::Done;
                    }
                    return Some(dmux);
                }
                Err((s, _)) => status = s,
            }
        }
        if let Some(state) = state.as_deref_mut() {
            *state = if status == ParseStatus::NeedMoreData {
                WebPDemuxState::ParsingHeader
            } else {
                WebPDemuxState::ParseError
            };
        }
        return None;
    }

    let partial = mem.buf_size < mem.riff_end;
    if !allow_partial && partial {
        return None;
    }

    let mut dmux = init_demux(data, &mem);

    status = ParseStatus::Error;
    for &(id, parser) in &K_MASTER_CHUNKS {
        if &dmux.get_buffer()[..TAG_SIZE] == id {
            status = match parser {
                MasterChunk::SingleImage => parse_single_image(&mut dmux),
                MasterChunk::Vp8x => parse_vp8x(&mut dmux),
            };
            if status == ParseStatus::Ok {
                dmux.state = WebPDemuxState::Done;
            }
            if status == ParseStatus::NeedMoreData && !partial {
                status = ParseStatus::Error;
            }
            let valid = match parser {
                MasterChunk::SingleImage => is_valid_simple_format(&dmux),
                MasterChunk::Vp8x => is_valid_extended_format(&dmux),
            };
            if status != ParseStatus::Error && !valid {
                status = ParseStatus::Error;
            }
            if status == ParseStatus::Error {
                dmux.state = WebPDemuxState::ParseError;
            }
            break;
        }
    }
    if let Some(state) = state.as_deref_mut() {
        *state = dmux.state;
    }

    if status == ParseStatus::Error {
        return None;
    }
    Some(dmux)
}

/// Parses the full WebP file given by 'data'. Translation of
/// `WebPDemux()`.
pub(crate) fn webp_demux(data: Vec<u8>) -> Option<WebPDemuxer> {
    webp_demux_internal(data, false, None)
}

// -----------------------------------------------------------------------------

impl WebPDemuxer {
    /// Get the 'feature' value from the 'dmux'. NOTE: values are only valid
    /// if WebPDemux() was used or WebPDemuxPartial() returned a state >
    /// WEBP_DEMUX_PARSING_HEADER. Translation of `WebPDemuxGetI()`.
    pub(crate) fn get_i(&self, feature: WebPFormatFeature) -> u32 {
        match feature {
            WebPFormatFeature::FormatFlags => self.feature_flags,
            WebPFormatFeature::CanvasWidth => self.canvas_width as u32,
            WebPFormatFeature::CanvasHeight => self.canvas_height as u32,
            WebPFormatFeature::LoopCount => self.loop_count as u32,
            WebPFormatFeature::BackgroundColor => self.bgcolor,
            WebPFormatFeature::FrameCount => self.num_frames as u32,
        }
    }
}

// -----------------------------------------------------------------------------
// Frame iteration

/// Translation of `GetFrame()`.
fn get_frame(dmux: &WebPDemuxer, frame_num: i32) -> Option<&Frame> {
    dmux.frames.iter().find(|f| frame_num == f.frame_num)
}

/// Translation of `GetFramePayload()`: the payload's offset and size.
fn get_frame_payload(frame: &Frame) -> (usize, usize) {
    let image = &frame.img_components[0];
    let alpha = &frame.img_components[1];
    let mut start_offset = image.offset;
    let mut data_size = image.size;

    // if alpha exists it precedes image, update the size allowing for
    // intervening chunks.
    if alpha.size > 0 {
        let inter_size = if image.offset > 0 {
            image.offset - (alpha.offset + alpha.size)
        } else {
            0
        };
        start_offset = alpha.offset;
        data_size += alpha.size + inter_size;
    }
    (start_offset, data_size)
}

/// Create a whole 'frame' from VP8 (+ alpha) or lossless. Translation of
/// `SynthesizeFrame()`.
fn synthesize_frame(dmux: &WebPDemuxer, frame: &Frame, iter: &mut WebPIterator) -> bool {
    let payload = get_frame_payload(frame);

    iter.frame_num = frame.frame_num;
    iter.num_frames = dmux.num_frames;
    iter.x_offset = frame.x_offset;
    iter.y_offset = frame.y_offset;
    iter.width = frame.width;
    iter.height = frame.height;
    iter.has_alpha = frame.has_alpha;
    iter.duration = frame.duration;
    iter.dispose_method = frame.dispose_method;
    iter.blend_method = frame.blend_method;
    iter.complete = frame.complete;
    iter.fragment = payload;
    true
}

/// Translation of `SetFrame()`.
fn set_frame(dmux: &WebPDemuxer, mut frame_num: i32, iter: &mut WebPIterator) -> bool {
    if !iter.private || frame_num < 0 {
        return false;
    }
    if frame_num > dmux.num_frames {
        return false;
    }
    if frame_num == 0 {
        frame_num = dmux.num_frames;
    }

    let Some(frame) = get_frame(dmux, frame_num) else {
        return false;
    };

    synthesize_frame(dmux, frame, iter)
}

/// Retrieves frame 'frame_number' from 'dmux'. 'iter->fragment' points to
/// the frame on return from this function. Setting 'frame_number' equal
/// to 0 will return the last frame of the image. Returns false if 'dmux'
/// is NULL or frame 'frame_number' is not present. Translation of
/// `WebPDemuxGetFrame()`.
pub(crate) fn webp_demux_get_frame(dmux: &WebPDemuxer, frame: i32, iter: &mut WebPIterator) -> bool {
    *iter = WebPIterator::default();
    iter.private = true;
    set_frame(dmux, frame, iter)
}

/// Sets 'iter->fragment' to point to the next ('iter->frame_num' + 1)
/// frame. Returns true on success, false otherwise. Translation of
/// `WebPDemuxNextFrame()`.
pub(crate) fn webp_demux_next_frame(dmux: &WebPDemuxer, iter: &mut WebPIterator) -> bool {
    set_frame(dmux, iter.frame_num + 1, iter)
}

/// Sets 'iter->fragment' to point to the previous ('iter->frame_num' - 1)
/// frame. Translation of `WebPDemuxPrevFrame()`.
pub(crate) fn webp_demux_prev_frame(dmux: &WebPDemuxer, iter: &mut WebPIterator) -> bool {
    if iter.frame_num <= 1 {
        return false;
    }
    set_frame(dmux, iter.frame_num - 1, iter)
}

/// Releases any memory associated with 'iter'. Translation of
/// `WebPDemuxReleaseIterator()` (nothing to release).
pub(crate) fn webp_demux_release_iterator(_iter: &mut WebPIterator) {}

// -----------------------------------------------------------------------------
// Chunk iteration

/// Translation of `ChunkCount()`.
fn chunk_count(dmux: &WebPDemuxer, fourcc: &[u8; 4]) -> i32 {
    let mut count = 0;
    for c in &dmux.chunks {
        let header = &dmux.data[c.data.offset..c.data.offset + TAG_SIZE];
        if header == fourcc {
            count += 1;
        }
    }
    count
}

/// Translation of `GetChunk()`.
fn get_chunk<'d>(dmux: &'d WebPDemuxer, fourcc: &[u8; 4], chunk_num: i32) -> Option<&'d Chunk> {
    let mut count = 0;
    for c in &dmux.chunks {
        let header = &dmux.data[c.data.offset..c.data.offset + TAG_SIZE];
        if header == fourcc {
            count += 1;
        }
        if count == chunk_num {
            return Some(c);
        }
    }
    None
}

/// Translation of `SetChunk()`.
fn set_chunk(dmux: &WebPDemuxer, fourcc: &[u8; 4], mut chunk_num: i32, iter: &mut WebPChunkIterator) -> bool {
    if !iter.private || chunk_num < 0 {
        return false;
    }
    let count = chunk_count(dmux, fourcc);
    if count == 0 {
        return false;
    }
    if chunk_num == 0 {
        chunk_num = count;
    }

    if chunk_num <= count {
        let Some(chunk) = get_chunk(dmux, fourcc, chunk_num) else {
            return false;
        };
        iter.chunk = (
            chunk.data.offset + CHUNK_HEADER_SIZE,
            chunk.data.size - CHUNK_HEADER_SIZE,
        );
        iter.num_chunks = count;
        iter.chunk_num = chunk_num;
        return true;
    }
    false
}

/// Retrieves the 'chunk_number' instance of the chunk with id 'fourcc'
/// from 'dmux'. 'fourcc' is a character array containing the fourcc of the
/// chunk to return, e.g., "ICCP", "XMP ", "EXIF", etc. Setting
/// 'chunk_number' equal to 0 will return the last chunk in a set. Returns
/// true if the chunk is found, false otherwise. Translation of
/// `WebPDemuxGetChunk()`.
pub(crate) fn webp_demux_get_chunk(
    dmux: &WebPDemuxer,
    fourcc: &[u8; 4],
    chunk_num: i32,
    iter: &mut WebPChunkIterator,
) -> bool {
    *iter = WebPChunkIterator::default();
    iter.private = true;
    set_chunk(dmux, fourcc, chunk_num, iter)
}

/// The fourcc of the iterator's chunk: the header before its payload.
fn iter_fourcc(dmux: &WebPDemuxer, iter: &WebPChunkIterator) -> [u8; 4] {
    let start = iter.chunk.0 - CHUNK_HEADER_SIZE;
    let mut fourcc = [0u8; 4];
    fourcc.copy_from_slice(&dmux.data[start..start + TAG_SIZE]);
    fourcc
}

/// Sets 'iter->chunk' to point to the next ('iter->chunk_num' + 1) chunk.
/// Translation of `WebPDemuxNextChunk()`.
pub(crate) fn webp_demux_next_chunk(dmux: &WebPDemuxer, iter: &mut WebPChunkIterator) -> bool {
    if !iter.private {
        return false;
    }
    let fourcc = iter_fourcc(dmux, iter);
    set_chunk(dmux, &fourcc, iter.chunk_num + 1, iter)
}

/// Sets 'iter->chunk' to point to the previous ('iter->chunk_num' - 1)
/// chunk. Translation of `WebPDemuxPrevChunk()`.
pub(crate) fn webp_demux_prev_chunk(dmux: &WebPDemuxer, iter: &mut WebPChunkIterator) -> bool {
    if iter.private && iter.chunk_num > 1 {
        let fourcc = iter_fourcc(dmux, iter);
        return set_chunk(dmux, &fourcc, iter.chunk_num - 1, iter);
    }
    false
}

/// Releases any memory associated with 'iter'. Translation of
/// `WebPDemuxReleaseChunkIterator()` (nothing to release).
pub(crate) fn webp_demux_release_chunk_iterator(_iter: &mut WebPChunkIterator) {}
