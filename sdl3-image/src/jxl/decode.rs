// Rust translation of the parts of lib/jxl/decode.cc (and
// lib/include/jxl/decode.h) from libjxl (https://github.com/libjxl/libjxl,
// at the revision SDL_image's external/libjxl pins: libjxl 0.7.3 with SDL's
// patches) that SDL_image's IMG_jxl.c uses.
// Copyright (c) the JPEG XL Project Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The JxlDecoder API state machine, for the calls SDL_image makes:
//! `JxlDecoderCreate`, `JxlDecoderSubscribeEvents`, `JxlDecoderSetInput`,
//! `JxlDecoderProcessInput`, `JxlDecoderGetBasicInfo`,
//! `JxlDecoderImageOutBufferSize` and `JxlDecoderSetImageOutBuffer`. Box
//! output, JPEG reconstruction, previews, frame skipping, progressive
//! events and callbacks are not translated (SDL_image never asks for them;
//! their decoder state stays at the defaults). There is no thread pool and
//! no memory limit (`memory_limit_base` is 0 outside fuzzing builds).

use std::rc::Rc;

use super::base::{div_ceil, load_be32, load_be64, StatusCode, K_BITS_PER_BYTE};
use super::dec_bit_reader::BitReader;
use super::dec_cache::PassesDecoderState;
use super::dec_external_image::convert_to_external;
use super::dec_frame::{FrameDecoder, SectionInfo, SectionStatus};
use super::fields::{bundle_can_read, bundle_read, Fields};
use super::frame_header::{FrameHeader, FrameType};
use super::headers::K_CODESTREAM_MARKER;
use super::icc_codec::read_icc;
use super::image_bundle::ImageBundle;
use super::image_metadata::{CodecMetadata, ExtraChannel, ImageMetadata, Orientation};

/// Translation of `JxlDecoderStatus`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum JxlDecoderStatus {
    Success,
    Error,
    NeedMoreInput,
    NeedImageOutBuffer,
    BasicInfo,
    ColorEncoding,
    Frame,
    FullImage,
}

impl JxlDecoderStatus {
    /// The numeric value of the status (as `IMG_jxl.c` prints it).
    pub(crate) fn value(self) -> i32 {
        match self {
            JxlDecoderStatus::Success => 0,
            JxlDecoderStatus::Error => 1,
            JxlDecoderStatus::NeedMoreInput => 2,
            JxlDecoderStatus::NeedImageOutBuffer => 5,
            JxlDecoderStatus::BasicInfo => 0x40,
            JxlDecoderStatus::ColorEncoding => 0x100,
            JxlDecoderStatus::Frame => 0x400,
            JxlDecoderStatus::FullImage => 0x1000,
        }
    }
}

pub(crate) const JXL_DEC_BASIC_INFO: i32 = 0x40;
pub(crate) const JXL_DEC_COLOR_ENCODING: i32 = 0x100;
pub(crate) const JXL_DEC_PREVIEW_IMAGE: i32 = 0x200;
pub(crate) const JXL_DEC_FRAME: i32 = 0x400;
pub(crate) const JXL_DEC_FULL_IMAGE: i32 = 0x1000;
pub(crate) const JXL_DEC_BOX: i32 = 0x4000;
pub(crate) const JXL_DEC_FRAME_PROGRESSION: i32 = 0x8000;

/// Translation of `JxlDataType`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[allow(dead_code)]
pub(crate) enum JxlDataType {
    Float,
    Uint8,
    Uint16,
    Float16,
}

/// Translation of `JxlEndianness`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[allow(dead_code)]
pub(crate) enum JxlEndianness {
    Native,
    Little,
    Big,
}

/// Translation of `JxlPixelFormat`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct JxlPixelFormat {
    pub num_channels: u32,
    pub data_type: JxlDataType,
    pub endianness: JxlEndianness,
    pub align: usize,
}

/// The fields of `JxlBasicInfo` SDL_image reads.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct JxlBasicInfo {
    pub have_container: bool,
    pub xsize: u32,
    pub ysize: u32,
    pub bits_per_sample: u32,
    pub exponent_bits_per_sample: u32,
    pub have_animation: bool,
    pub alpha_bits: u32,
    pub alpha_premultiplied: bool,
    pub num_color_channels: u32,
    pub num_extra_channels: u32,
}

// Checks if a + b > size, taking possible integer overflow into account.
fn out_of_bounds(a: usize, b: usize, size: usize) -> bool {
    let pos = a.wrapping_add(b);
    if pos > size {
        return true;
    }
    if pos < a {
        return true; // overflow happened
    }
    false
}

fn sum_overflows(a: usize, b: usize, c: usize) -> bool {
    let mut sum = a.wrapping_add(b);
    if sum < b {
        return true;
    }
    sum = sum.wrapping_add(c);
    if sum < c {
        return true;
    }
    false
}

fn initial_basic_info_size_hint() -> usize {
    // Amount of bytes before the start of the codestream in the container format,
    // assuming that the codestream is the first box after the signature and
    // filetype boxes. 12 bytes signature box + 20 bytes filetype box + 16 bytes
    // codestream box length + name + optional XLBox length.
    let container_header_size = 48;

    // Worst-case amount of bytes for basic info of the JPEG XL codestream header,
    // that is all information up to and including extra_channel_bits. Up to
    // around 2 bytes signature + 8 bytes SizeHeader + 31 bytes ColorEncoding + 4
    // bytes rest of ImageMetadata + 5 bytes part of ImageMetadata2.
    // TODO(lode): recompute and update this value when alpha_bits is moved to
    // extra channels info.
    let max_codestream_basic_info_size = 50;

    container_header_size + max_codestream_basic_info_size
}

/// Translation of `JxlSignature`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum JxlSignature {
    NotEnoughBytes,
    Invalid,
    Codestream,
    Container,
}

/// Translation of `ReadSignature()` / `JxlSignatureCheck()`.
fn jxl_signature_check(buf: &[u8]) -> JxlSignature {
    let len = buf.len();
    if len == 0 {
        return JxlSignature::NotEnoughBytes;
    }

    // JPEG XL codestream: 0xff 0x0a
    if buf[0] == 0xff {
        if len < 2 {
            return JxlSignature::NotEnoughBytes;
        } else if buf[1] == K_CODESTREAM_MARKER {
            return JxlSignature::Codestream;
        } else {
            return JxlSignature::Invalid;
        }
    }

    // JPEG XL container
    if buf[0] == 0 {
        if len < 12 {
            return JxlSignature::NotEnoughBytes;
        } else if buf[1] == 0
            && buf[2] == 0
            && buf[3] == 0xC
            && buf[4] == b'J'
            && buf[5] == b'X'
            && buf[6] == b'L'
            && buf[7] == b' '
            && buf[8] == 0xD
            && buf[9] == 0xA
            && buf[10] == 0x87
            && buf[11] == 0xA
        {
            return JxlSignature::Container;
        } else {
            return JxlSignature::Invalid;
        }
    }

    JxlSignature::Invalid
}

/// Translation of `BitsPerChannel()`.
fn bits_per_channel(data_type: JxlDataType) -> usize {
    match data_type {
        JxlDataType::Uint8 => 8,
        JxlDataType::Uint16 => 16,
        JxlDataType::Float => 32,
        JxlDataType::Float16 => 16,
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum DecoderStage {
    Inited,             // Decoder created, no JxlDecoderProcessInput called yet
    Started,            // Running JxlDecoderProcessInput calls
    CodestreamFinished, // Codestream done, but other boxes could still occur.
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum FrameStage {
    Header,     // Must parse frame header.
    Toc,        // Must parse TOC
    Full,       // Must parse full pixels
    FullOutput, // Must output full pixels
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum BoxStage {
    Header,            // Parsing box header of the next box, or start of non-container stream
    Ftyp,              // The ftyp box
    Skip,              // Box whose contents are skipped
    Codestream,        // Handling codestream box contents, or non-container stream
    PartialCodestream, // Handling the extra header of partial codestream box
}

/// Translation of `JxlDecoderStruct` (the decoder works on an input buffer
/// set once: SDL_image passes the whole file).
pub(crate) struct JxlDecoder<'a> {
    stage: DecoderStage,

    // Status of progression, internal.
    got_signature: bool,
    // Indicates we know that we've seen the last codestream box: either this
    // was a jxlc box, or a jxlp box that has its index indicated as last by
    // having its most significant bit set, or no boxes are used at all. This
    // does not indicate the full codestream has already been seen, only the
    // last box of it has been initiated.
    last_codestream_seen: bool,
    got_codestream_signature: bool,
    got_basic_info: bool,
    got_transform_data: bool, // To skip everything before ICC.
    got_all_headers: bool,    // Codestream metadata headers.
    post_headers: bool,       // Already decoding pixels.
    // This means either we actually got the preview image, or determined we
    // cannot get it or there is none.
    got_preview_image: bool,
    preview_frame: bool,

    // Position of next_in in the original file including box format if present
    // (as opposed to position in the codestream)
    file_pos: usize,

    box_contents_begin: usize,
    box_contents_end: usize,
    box_contents_size: usize,
    header_size: usize,
    // Either a final box that runs until EOF, or the case of no container format
    // at all.
    box_contents_unbounded: bool,

    box_type: [u8; 4],
    box_decoded_type: [u8; 4], // Underlying type for brob boxes

    // Settings
    keep_orientation: bool,
    unpremul_alpha: bool,
    render_spotcolors: bool,
    coalescing: bool,
    desired_intensity_target: f32,

    // Bitfield, for which informative events (JXL_DEC_BASIC_INFO, etc...) the
    // decoder returns a status. By default, do not return for any of the events,
    // only return when the decoder cannot continue because it needs more input or
    // output data.
    events_wanted: i32,
    orig_events_wanted: i32,

    // Fields for reading the basic info from the header.
    basic_info_size_hint: usize,
    have_container: bool,
    box_count: usize,

    // Set to true if either an image out buffer or an image out callback was set.
    image_out_buffer_set: bool,

    // Owned buffer for the full resolution image (upstream, the caller's).
    image_out_buffer: Option<Vec<u8>>,
    image_out_size: usize,
    image_out_format: JxlPixelFormat,

    metadata: Rc<CodecMetadata>,
    // Same as metadata.m, except for the color_encoding, which is set to the
    // output encoding.
    image_metadata: Rc<ImageMetadata>,
    ib: Option<ImageBundle>,

    passes_state: Option<Box<PassesDecoderState>>,
    frame_dec: Option<FrameDecoder>,
    next_section: usize,
    section_processed: Vec<u8>,
    // The FrameDecoder is initialized, and not yet finalized
    frame_dec_in_progress: bool,

    // headers and TOC for the current frame. When got_toc is true, this is
    // always the frame header of the last frame of the current still series,
    // that is, the displayed frame.
    frame_header: FrameHeader,

    remaining_frame_size: usize,
    frame_stage: FrameStage,
    // The currently processed frame is the last of the current composite still,
    // and so must be returned as pixels
    is_last_of_still: bool,
    // The currently processed frame is the last of the codestream
    is_last_total: bool,
    // Skipping the current frame (never: SDL_image does not skip frames).
    skipping_frame: bool,

    // Amount of internal frames and external frames started. External frames are
    // user-visible frames, internal frames includes all external frames and
    // also invisible frames such as patches, blending-only and dc_level frames.
    internal_frames: usize,
    external_frames: usize,

    // For each internal frame, which storage locations it references, and which
    // storage locations it is stored in, using the bit mask as defined in
    // FrameDecoder::References and FrameDecoder::SaveAs.
    frame_references: Vec<i32>,
    frame_saved_as: Vec<i32>,

    // Translates external frame index to internal frame index. The external
    // index is the index of user-visible frames. The internal index can be larger
    // since non-visible frames (such as frames with patches, ...) are included.
    frame_external_to_internal: Vec<usize>,

    // Codestream input data is copied here temporarily when the decoder needs
    // more input bytes to process the next part of the stream. We copy the input
    // data in order to be able to release it all through the API it when
    // returning JXL_DEC_NEED_MORE_INPUT.
    codestream_copy: Vec<u8>,
    // Number of bytes at the end of codestream_copy that were not yet consumed
    // by calling AdvanceInput().
    codestream_unconsumed: usize,
    // Position in the codestream_copy vector that the decoder already finished
    // processing. It can be greater than the current size of codestream_copy in
    // case where the decoder skips some parts of the frame that were not yet
    // provided.
    codestream_pos: usize,
    // Number of bits after codestream_pos that were already processed.
    codestream_bits_ahead: usize,

    box_stage: BoxStage,

    input: &'a [u8],
    next_in: usize,
    avail_in: usize,
    input_closed: bool,
}

/// The codestream bytes to read from: a part of the input or of the copy.
#[derive(Clone, Copy)]
enum Span {
    Input(usize, usize),
    Copy(usize, usize),
}

impl<'a> JxlDecoder<'a> {
    /// Translation of `JxlDecoderCreate()` (with `JxlDecoderReset()`).
    pub(crate) fn new() -> Self {
        let metadata = CodecMetadata::new();
        let image_metadata = Rc::new(metadata.m.clone());
        let metadata = Rc::new(metadata);
        JxlDecoder {
            stage: DecoderStage::Inited,
            got_signature: false,
            last_codestream_seen: false,
            got_codestream_signature: false,
            got_basic_info: false,
            got_transform_data: false,
            got_all_headers: false,
            post_headers: false,
            got_preview_image: false,
            preview_frame: false,
            file_pos: 0,
            box_contents_begin: 0,
            box_contents_end: 0,
            box_contents_size: 0,
            header_size: 0,
            box_contents_unbounded: false,
            box_type: [0; 4],
            box_decoded_type: [0; 4],
            keep_orientation: false,
            unpremul_alpha: false,
            render_spotcolors: true,
            coalescing: true,
            desired_intensity_target: 0.0,
            events_wanted: 0,
            orig_events_wanted: 0,
            basic_info_size_hint: initial_basic_info_size_hint(),
            have_container: false,
            box_count: 0,
            image_out_buffer_set: false,
            image_out_buffer: None,
            image_out_size: 0,
            image_out_format: JxlPixelFormat {
                num_channels: 0,
                data_type: JxlDataType::Float,
                endianness: JxlEndianness::Native,
                align: 0,
            },
            frame_header: FrameHeader::new(Some(metadata.clone())),
            metadata,
            image_metadata,
            ib: None,
            passes_state: None,
            frame_dec: None,
            next_section: 0,
            section_processed: Vec::new(),
            frame_dec_in_progress: false,
            remaining_frame_size: 0,
            frame_stage: FrameStage::Header,
            is_last_of_still: false,
            is_last_total: false,
            skipping_frame: false,
            internal_frames: 0,
            external_frames: 0,
            frame_references: Vec::new(),
            frame_saved_as: Vec::new(),
            frame_external_to_internal: Vec::new(),
            codestream_copy: Vec::new(),
            codestream_unconsumed: 0,
            codestream_pos: 0,
            codestream_bits_ahead: 0,
            box_stage: BoxStage::Header,
            input: &[],
            next_in: 0,
            avail_in: 0,
            input_closed: false,
        }
    }

    /// Translation of `JxlDecoderSubscribeEvents()`.
    pub(crate) fn subscribe_events(&mut self, events_wanted: i32) -> JxlDecoderStatus {
        if self.stage != DecoderStage::Inited {
            return JxlDecoderStatus::Error; // Cannot subscribe to events after having started.
        }
        if events_wanted & 63 != 0 {
            return JxlDecoderStatus::Error; // Can only subscribe to informative events.
        }
        self.events_wanted = events_wanted;
        self.orig_events_wanted = events_wanted;
        JxlDecoderStatus::Success
    }

    /// Translation of `JxlDecoderSetInput()`.
    pub(crate) fn set_input(&mut self, data: &'a [u8]) -> JxlDecoderStatus {
        if !self.input.is_empty() {
            // "already set input, use JxlDecoderReleaseInput first"
            return JxlDecoderStatus::Error;
        }
        if self.input_closed {
            return JxlDecoderStatus::Error; // "input already closed"
        }
        self.input = data;
        self.next_in = 0;
        self.avail_in = data.len();
        JxlDecoderStatus::Success
    }

    fn advance_input(&mut self, size: usize) {
        debug_assert!(self.avail_in >= size);
        self.next_in += size;
        self.avail_in -= size;
        self.file_pos += size;
    }

    fn available_codestream(&self) -> usize {
        let mut avail_codestream = self.avail_in;
        if !self.box_contents_unbounded {
            avail_codestream =
                avail_codestream.min(self.box_contents_end.wrapping_sub(self.file_pos));
        }
        avail_codestream
    }

    fn advance_codestream(&mut self, size: usize) {
        let avail_codestream = self.available_codestream();
        if self.codestream_copy.is_empty() {
            if size <= avail_codestream {
                self.advance_input(size);
            } else {
                self.codestream_pos = size - avail_codestream;
                self.advance_input(avail_codestream);
            }
        } else {
            self.codestream_pos += size;
            if self.codestream_pos + self.codestream_unconsumed >= self.codestream_copy.len() {
                let advance = self.codestream_unconsumed.min(
                    self.codestream_unconsumed + self.codestream_pos - self.codestream_copy.len(),
                );
                self.advance_input(advance);
                self.codestream_pos -= self.codestream_pos.min(self.codestream_copy.len());
                self.codestream_unconsumed = 0;
                self.codestream_copy.clear();
            }
        }
    }

    fn request_more_input(&mut self) -> JxlDecoderStatus {
        if self.codestream_copy.is_empty() {
            let avail_codestream = self.available_codestream();
            let s = &self.input[self.next_in..self.next_in + avail_codestream];
            self.codestream_copy.extend_from_slice(s);
            self.advance_input(avail_codestream);
        } else {
            self.advance_input(self.codestream_unconsumed);
            self.codestream_unconsumed = 0;
        }
        JxlDecoderStatus::NeedMoreInput
    }

    fn get_codestream_input(&mut self) -> Result<Span, JxlDecoderStatus> {
        if self.codestream_copy.is_empty() && self.codestream_pos > 0 {
            let avail_codestream = self.available_codestream();
            let skip = self.codestream_pos.min(avail_codestream);
            self.advance_input(skip);
            self.codestream_pos -= skip;
            if self.codestream_pos > 0 {
                return Err(self.request_more_input());
            }
        }
        debug_assert!(self.codestream_pos <= self.codestream_copy.len());
        debug_assert!(self.codestream_unconsumed <= self.codestream_copy.len());
        let avail_codestream = self.available_codestream();
        if self.codestream_copy.is_empty() {
            if avail_codestream == 0 {
                return Err(self.request_more_input());
            }
            Ok(Span::Input(self.next_in, avail_codestream))
        } else {
            let s = &self.input
                [self.next_in + self.codestream_unconsumed..self.next_in + avail_codestream];
            self.codestream_copy.extend_from_slice(s);
            self.codestream_unconsumed = avail_codestream;
            Ok(Span::Copy(
                self.codestream_pos,
                self.codestream_copy.len() - self.codestream_pos,
            ))
        }
    }

    /// The bytes of a codestream span.
    fn span_bytes(&self, span: Span) -> &[u8] {
        match span {
            Span::Input(start, len) => &self.input[start..start + len],
            Span::Copy(start, len) => &self.codestream_copy[start..start + len],
        }
    }

    /// A span's bytes, not borrowing the decoder (the bit readers must not
    /// borrow the decoder while it changes): the input itself, or a copy of
    /// the codestream copy.
    fn span_vec(&self, span: Span) -> std::borrow::Cow<'a, [u8]> {
        let input: &'a [u8] = self.input;
        match span {
            Span::Input(start, len) => std::borrow::Cow::Borrowed(&input[start..start + len]),
            Span::Copy(start, len) => {
                std::borrow::Cow::Owned(self.codestream_copy[start..start + len].to_vec())
            }
        }
    }

    /// Translation of `JxlDecoderReadBasicInfo()`.
    fn read_basic_info(&mut self) -> JxlDecoderStatus {
        if !self.got_codestream_signature {
            // Check and skip the codestream signature
            let span = match self.get_codestream_input() {
                Ok(s) => s,
                Err(e) => return e,
            };
            let bytes = self.span_bytes(span);
            if bytes.len() < 2 {
                return self.request_more_input();
            }
            if bytes[0] != 0xff || bytes[1] != K_CODESTREAM_MARKER {
                return JxlDecoderStatus::Error; // "invalid signature"
            }
            self.got_codestream_signature = true;
            self.advance_codestream(2);
        }

        let span = match self.get_codestream_input() {
            Ok(s) => s,
            Err(e) => return e,
        };
        let data = self.span_vec(span);
        let mut reader = BitReader::new(&data);
        let mut md = (*self.metadata).clone();
        let st = read_bundle(&data, &mut reader, &mut md.size);
        if st != JxlDecoderStatus::Success {
            return self.finish_reader(reader, st);
        }
        let st = read_bundle(&data, &mut reader, &mut md.m);
        if st != JxlDecoderStatus::Success {
            return self.finish_reader(reader, st);
        }
        let total_bits = reader.total_bits_consumed();
        close_reader(reader);
        self.metadata = Rc::new(md);
        self.advance_codestream(total_bits / K_BITS_PER_BYTE);
        self.codestream_bits_ahead = total_bits % K_BITS_PER_BYTE;
        self.got_basic_info = true;
        self.basic_info_size_hint = 0;
        self.image_metadata = Rc::new(self.metadata.m.clone());

        // (No size limit: memory_limit_base is 0.)
        JxlDecoderStatus::Success
    }

    /// Ends a `GetBitReader()` reader and maps `RequestMoreInput()`.
    fn finish_reader(&mut self, reader: BitReader<'_>, st: JxlDecoderStatus) -> JxlDecoderStatus {
        close_reader(reader);
        if st == JxlDecoderStatus::NeedMoreInput {
            return self.request_more_input();
        }
        st
    }

    /// Reads all codestream headers (but not frame headers). Translation of
    /// `JxlDecoderReadAllHeaders()`.
    fn read_all_headers(&mut self) -> JxlDecoderStatus {
        if !self.got_transform_data {
            let span = match self.get_codestream_input() {
                Ok(s) => s,
                Err(e) => return e,
            };
            let data = self.span_vec(span);
            let mut reader = BitReader::new(&data);
            reader.skip_bits(self.codestream_bits_ahead);
            let mut md = (*self.metadata).clone();
            md.transform_data.nonserialized_xyb_encoded = md.m.xyb_encoded;
            let st = read_bundle(&data, &mut reader, &mut md.transform_data);
            if st != JxlDecoderStatus::Success {
                return self.finish_reader(reader, st);
            }
            let total_bits = reader.total_bits_consumed();
            close_reader(reader);
            self.metadata = Rc::new(md);
            self.advance_codestream(total_bits / K_BITS_PER_BYTE);
            self.codestream_bits_ahead = total_bits % K_BITS_PER_BYTE;
            self.got_transform_data = true;
        }

        let span = match self.get_codestream_input() {
            Ok(s) => s,
            Err(e) => return e,
        };
        let data = self.span_vec(span);
        let mut reader = BitReader::new(&data);
        reader.skip_bits(self.codestream_bits_ahead);

        if self.metadata.m.color_encoding.want_icc() {
            let mut icc: Vec<u8> = Vec::new();
            let status = read_icc(&mut reader, 0, &mut icc);
            // Always check AllReadsWithinBounds, not all the C++ decoder implementation
            // handles reader out of bounds correctly  yet (e.g. context map). Not
            // checking AllReadsWithinBounds can cause reader->Close() to trigger an
            // assert, but we don't want library to quit program for invalid codestream.
            if !reader.all_reads_within_bounds() || status == Err(StatusCode::NotEnoughBytes) {
                close_reader(reader);
                return self.request_more_input();
            }
            if status.is_err() {
                // Other non-successful status is an error
                close_reader(reader);
                return JxlDecoderStatus::Error;
            }
            let mut md = (*self.metadata).clone();
            if md.m.color_encoding.set_icc_raw(icc).is_err() {
                close_reader(reader);
                return JxlDecoderStatus::Error;
            }
            self.metadata = Rc::new(md);
        }

        self.got_all_headers = true;
        if reader.jump_to_byte_boundary().is_err() {
            close_reader(reader);
            return JxlDecoderStatus::Error;
        }

        let consumed = reader.total_bits_consumed() / K_BITS_PER_BYTE;
        close_reader(reader);
        self.advance_codestream(consumed);
        self.codestream_bits_ahead = 0;

        if self.passes_state.is_none() {
            self.passes_state = Some(Box::new(PassesDecoderState::new()));
        }

        let md = self.metadata.clone();
        let Some(ps) = self.passes_state.as_mut() else {
            return JxlDecoderStatus::Error;
        };
        if ps.output_encoding_info.set_from_metadata(&md).is_err() {
            return JxlDecoderStatus::Error;
        }
        if self.desired_intensity_target > 0.0 {
            ps.output_encoding_info.desired_intensity_target = self.desired_intensity_target;
        }
        self.image_metadata = Rc::new(self.metadata.m.clone());

        JxlDecoderStatus::Success
    }

    /// Translation of `GetStride()`.
    fn get_stride(&self, format: &JxlPixelFormat) -> usize {
        let (xsize, _) = self.get_current_dimensions(true);
        let mut stride = xsize
            * (bits_per_channel(format.data_type) * format.num_channels as usize / K_BITS_PER_BYTE);
        if format.align > 1 {
            stride = div_ceil(stride, format.align) * format.align;
        }
        stride
    }

    /// Helper function to get the dimensions of the current image buffer.
    /// Translation of `GetCurrentDimensions()`.
    fn get_current_dimensions(&self, oriented: bool) -> (usize, usize) {
        if self.frame_header.nonserialized_is_preview {
            return (
                self.metadata.oriented_preview_xsize(self.keep_orientation),
                self.metadata.oriented_preview_ysize(self.keep_orientation),
            );
        }
        let mut xsize = self
            .metadata
            .oriented_xsize(self.keep_orientation || !oriented);
        let mut ysize = self
            .metadata
            .oriented_ysize(self.keep_orientation || !oriented);
        if !self.coalescing {
            let frame_dim = self.frame_header.to_frame_dimensions();
            xsize = frame_dim.xsize_upsampled;
            ysize = frame_dim.ysize_upsampled;
            if !self.keep_orientation && oriented && (self.metadata.m.get_orientation() as u32) > 4
            {
                std::mem::swap(&mut xsize, &mut ysize);
            }
        }
        (xsize, ysize)
    }

    /// Translation of `JxlDecoderProcessSections()`.
    fn process_sections(&mut self) -> JxlDecoderStatus {
        let span = match self.get_codestream_input() {
            Ok(s) => s,
            Err(e) => return e,
        };
        let data = self.span_vec(span);
        let Some(frame_dec) = self.frame_dec.as_mut() else {
            return JxlDecoderStatus::Error;
        };
        let toc = frame_dec.toc().clone();
        let mut pos = 0usize;
        let mut section_info: Vec<SectionInfo<'_>> = Vec::new();
        for i in self.next_section..toc.len() {
            if self.section_processed[i] != 0 {
                continue;
            }
            let id = toc[i].id;
            let size = toc[i].size;
            if out_of_bounds(pos, size, data.len()) {
                break;
            }
            let br = BitReader::new(&data[pos..pos + size]);
            section_info.push(SectionInfo { br, id });
            pos += size;
        }
        let mut section_status = vec![SectionStatus::Skipped; section_info.len()];
        let Some(dec_state) = self.passes_state.as_deref_mut() else {
            return JxlDecoderStatus::Error;
        };
        let status = frame_dec.process_sections(&mut section_info, &mut section_status, dec_state);
        let mut out_of_bounds_found = false;
        for info in section_info.iter_mut() {
            if !info.br.all_reads_within_bounds() {
                // Mark out of bounds section, but keep closing and deleting the next
                // ones as well.
                out_of_bounds_found = true;
            }
            // (JXL_ASSERT(info.br->Close()))
            let _ = info.br.close();
        }
        if out_of_bounds_found {
            // If any bit reader indicates out of bounds, it's an error, not just
            // needing more input, since we ensure only bit readers containing
            // a complete section are provided to the FrameDecoder.
            return JxlDecoderStatus::Error; // "frame out of bounds"
        }
        if status.is_err() {
            return JxlDecoderStatus::Error; // "frame processing failed"
        }
        let mut found_skipped_section = false;
        let mut num_done = 0usize;
        let mut processed_bytes = 0usize;
        for (i, &st) in section_status.iter().enumerate() {
            if st == SectionStatus::Done {
                if !found_skipped_section {
                    processed_bytes += toc[self.next_section + i].size;
                    num_done += 1;
                }
                self.section_processed[self.next_section + i] = 1;
            } else if st == SectionStatus::Skipped {
                found_skipped_section = true;
            } else {
                return JxlDecoderStatus::Error; // "unexpected section status"
            }
        }
        self.next_section += num_done;
        self.remaining_frame_size = self.remaining_frame_size.wrapping_sub(processed_bytes);
        self.advance_codestream(processed_bytes);
        JxlDecoderStatus::Success
    }

    /// Translation of `JxlDecoderProcessCodestream()`.
    fn process_codestream(&mut self) -> JxlDecoderStatus {
        // No matter what events are wanted, the basic info is always required.
        if !self.got_basic_info {
            let status = self.read_basic_info();
            if status != JxlDecoderStatus::Success {
                return status;
            }
        }

        if self.events_wanted & JXL_DEC_BASIC_INFO != 0 {
            self.events_wanted &= !JXL_DEC_BASIC_INFO;
            return JxlDecoderStatus::BasicInfo;
        }

        if self.events_wanted == 0 {
            self.stage = DecoderStage::CodestreamFinished;
            return JxlDecoderStatus::Success;
        }

        if !self.got_all_headers {
            let status = self.read_all_headers();
            if status != JxlDecoderStatus::Success {
                return status;
            }
        }

        if self.events_wanted & JXL_DEC_COLOR_ENCODING != 0 {
            self.events_wanted &= !JXL_DEC_COLOR_ENCODING;
            return JxlDecoderStatus::ColorEncoding;
        }

        if self.events_wanted == 0 {
            self.stage = DecoderStage::CodestreamFinished;
            return JxlDecoderStatus::Success;
        }

        self.post_headers = true;

        if !self.got_preview_image && self.metadata.m.have_preview {
            self.preview_frame = true;
        }

        // Handle frames
        loop {
            let parse_frames = self.events_wanted
                & (JXL_DEC_PREVIEW_IMAGE | JXL_DEC_FRAME | JXL_DEC_FULL_IMAGE)
                != 0;
            if !parse_frames {
                break;
            }
            if self.frame_stage == FrameStage::Header && self.is_last_total {
                break;
            }
            if self.frame_stage == FrameStage::Header {
                if self.ib.is_none() {
                    self.ib = Some(ImageBundle::new(Some(self.image_metadata.clone())));
                }
                // (No JPEG reconstruction.)

                let mut frame_dec = FrameDecoder::new(self.metadata.clone());
                self.frame_header = FrameHeader::new(Some(self.metadata.clone()));
                let span = match self.get_codestream_input() {
                    Ok(s) => s,
                    Err(e) => return e,
                };
                let data = self.span_vec(span);
                let mut reader = BitReader::new(&data);
                let output_needed = if self.preview_frame {
                    self.events_wanted & JXL_DEC_PREVIEW_IMAGE != 0
                } else {
                    self.events_wanted & JXL_DEC_FULL_IMAGE != 0
                };
                let Some(dec_state) = self.passes_state.as_deref_mut() else {
                    return JxlDecoderStatus::Error;
                };
                let ib = self
                    .ib
                    .take()
                    .unwrap_or_else(|| ImageBundle::new(Some(self.image_metadata.clone())));
                let status = frame_dec.init_frame(
                    &mut reader,
                    ib,
                    dec_state,
                    self.preview_frame,
                    output_needed,
                );
                if !reader.all_reads_within_bounds() || status == Err(StatusCode::NotEnoughBytes) {
                    close_reader(reader);
                    self.ib = Some(std::mem::replace(
                        &mut frame_dec.decoded,
                        ImageBundle::new(None),
                    ));
                    self.frame_dec = Some(frame_dec);
                    return self.request_more_input();
                } else if status.is_err() {
                    close_reader(reader);
                    self.frame_dec = Some(frame_dec);
                    return JxlDecoderStatus::Error; // "invalid frame header"
                }
                let consumed = reader.total_bits_consumed() / K_BITS_PER_BYTE;
                close_reader(reader);
                self.advance_codestream(consumed);
                self.frame_header = frame_dec.get_frame_header().clone();
                // (No size limits: memory_limit_base and cpu_limit_base are 0.)
                self.remaining_frame_size = frame_dec.sum_section_sizes() as usize;
                self.frame_dec = Some(frame_dec);

                self.frame_stage = FrameStage::Toc;
                if self.preview_frame {
                    if self.events_wanted & JXL_DEC_PREVIEW_IMAGE == 0 {
                        self.frame_stage = FrameStage::Header;
                        self.advance_codestream(self.remaining_frame_size);
                        self.got_preview_image = true;
                        self.preview_frame = false;
                    }
                    continue;
                }

                let saved_as = FrameDecoder::saved_as(&self.frame_header);
                // is last in entire codestream
                self.is_last_total = self.frame_header.is_last;
                // is last of current still
                self.is_last_of_still =
                    self.is_last_total || self.frame_header.animation_frame.duration > 0;
                // is kRegularFrame and coalescing is disabled
                self.is_last_of_still |=
                    !self.coalescing && self.frame_header.frame_type == FrameType::RegularFrame;
                let internal_frame_index = self.internal_frames;
                let external_frame_index = self.external_frames;
                if self.is_last_of_still {
                    self.external_frames += 1;
                }
                self.internal_frames += 1;

                // (skip_frames is 0.)
                self.skipping_frame = false;

                if external_frame_index >= self.frame_external_to_internal.len() {
                    self.frame_external_to_internal.push(internal_frame_index);
                }

                if internal_frame_index >= self.frame_saved_as.len() {
                    self.frame_saved_as.push(saved_as);

                    // add the value 0xff (which means all references) to new slots: we only
                    // know the references of the frame at FinalizeFrame, and fill in the
                    // correct values there. As long as this information is not known, the
                    // worst case where the frame depends on all storage slots is assumed.
                    self.frame_references.push(0xff);
                }

                if (self.events_wanted & JXL_DEC_FRAME) != 0 && self.is_last_of_still {
                    // Only return this for the last of a series of stills: patches frames
                    // etc... before this one do not contain the correct information such
                    // as animation timing, ...
                    if !self.skipping_frame {
                        return JxlDecoderStatus::Frame;
                    }
                }
            }

            if self.frame_stage == FrameStage::Toc {
                if let Some(fd) = self.frame_dec.as_mut() {
                    fd.set_render_spotcolors(self.render_spotcolors);
                    fd.set_coalescing(self.coalescing);
                }

                // (JXL_DEC_FRAME_PROGRESSION is not subscribed: the progressive
                // detail is kFrames.)

                self.next_section = 0;
                self.section_processed.clear();
                let n = self.frame_dec.as_ref().map_or(0, |f| f.toc().len());
                self.section_processed.resize(n, 0);

                // If we don't need pixels, we can skip actually decoding the frames
                // (kFull / kFullOut).
                if self.preview_frame || (self.events_wanted & JXL_DEC_FULL_IMAGE) != 0 {
                    self.frame_dec_in_progress = true;
                    self.frame_stage = FrameStage::Full;
                } else if !self.is_last_total {
                    self.frame_stage = FrameStage::Header;
                    self.advance_codestream(self.remaining_frame_size);
                    continue;
                } else {
                    break;
                }
            }

            let mut return_full_image = false;

            if self.frame_stage == FrameStage::Full {
                if self.preview_frame {
                    // (Previews are not translated: JXL_DEC_PREVIEW_IMAGE is
                    // not subscribed, so this is unreachable.)
                    return JxlDecoderStatus::Error;
                } else if (self.events_wanted & JXL_DEC_FULL_IMAGE) != 0
                    && !self.image_out_buffer_set
                    && self.is_last_of_still
                {
                    // TODO(lode): remove the dec->is_last_of_still condition if the
                    // frame decoder needs the image buffer as working space for decoding
                    // non-visible or blending frames too
                    if !self.skipping_frame {
                        return JxlDecoderStatus::NeedImageOutBuffer;
                    }
                }

                {
                    let (Some(fd), Some(ps)) =
                        (self.frame_dec.as_ref(), self.passes_state.as_deref_mut())
                    else {
                        return JxlDecoderStatus::Error;
                    };
                    fd.maybe_set_unpremultiply_alpha(self.unpremul_alpha, ps);
                }

                if !self.preview_frame
                    && self.image_out_buffer_set
                    && self.image_out_buffer.is_some()
                    && self.image_out_format.data_type == JxlDataType::Uint8
                    && self.image_out_format.num_channels >= 3
                {
                    let is_rgba = self.image_out_format.num_channels == 4;
                    let stride = self.get_stride(&self.image_out_format);
                    let keep_orientation = self.keep_orientation;
                    if let (Some(buf), Some(fd), Some(ps)) = (
                        self.image_out_buffer.take(),
                        self.frame_dec.as_ref(),
                        self.passes_state.as_deref_mut(),
                    ) {
                        self.image_out_buffer = fd.maybe_set_rgb8_output_buffer(
                            buf,
                            stride,
                            is_rgba,
                            !keep_orientation,
                            ps,
                        );
                    }
                }

                // (No float callback.)

                let status = self.process_sections();
                if status != JxlDecoderStatus::Success {
                    self.reclaim_rgb_output();
                    return status;
                }

                let all_sections_done =
                    self.frame_dec.as_ref().is_some_and(|f| f.has_decoded_all());

                if !all_sections_done {
                    // Not all sections have been processed yet
                    self.reclaim_rgb_output();
                    return self.request_more_input();
                }

                if !self.preview_frame {
                    let internal_index = self.internal_frames - 1;
                    debug_assert!(self.frame_references.len() > internal_index);
                    // Always fill this in, even if it was already written, it could be that
                    // this frame was skipped before and set to 255, while only now we know
                    // the true value.
                    if let (Some(fd), Some(ps)) =
                        (self.frame_dec.as_ref(), self.passes_state.as_deref())
                    {
                        self.frame_references[internal_index] = fd.references(ps);
                    }
                }

                let finalized = match (self.frame_dec.as_mut(), self.passes_state.as_deref_mut()) {
                    (Some(fd), Some(ps)) => fd.finalize_frame(ps),
                    _ => Err(StatusCode::GenericError),
                };
                if finalized.is_err() {
                    self.reclaim_rgb_output();
                    return JxlDecoderStatus::Error; // "decoding frame failed"
                }

                self.frame_dec_in_progress = false;
                self.frame_stage = FrameStage::FullOutput;
            }

            if self.frame_stage == FrameStage::FullOutput {
                let has_rgb_buffer = self
                    .passes_state
                    .as_deref()
                    .is_some_and(FrameDecoder::has_rgb_buffer);
                self.reclaim_rgb_output();
                if self.is_last_of_still {
                    if self.events_wanted & JXL_DEC_FULL_IMAGE != 0 {
                        self.events_wanted &= !JXL_DEC_FULL_IMAGE;
                        return_full_image = true;
                    }

                    // Frame finished, restore the events_wanted with the per-frame events
                    // from orig_events_wanted, in case there is a next frame.
                    self.events_wanted |= self.orig_events_wanted
                        & (JXL_DEC_FULL_IMAGE | JXL_DEC_FRAME | JXL_DEC_FRAME_PROGRESSION);

                    // If no output buffer was set, we merely return the JXL_DEC_FULL_IMAGE
                    // status without outputting pixels.
                    if return_full_image && self.image_out_buffer_set {
                        if !has_rgb_buffer {
                            // Copy pixels if desired.
                            let status = self.convert_image_internal();
                            if status != JxlDecoderStatus::Success {
                                return status;
                            }
                        }
                        self.image_out_buffer_set = false;
                        // (No extra channel output buffers.)
                    }
                }
            }

            self.frame_stage = FrameStage::Header;

            // The pixels have been output or are not needed, do not keep them in
            // memory here.
            self.ib = None;
            if let Some(fd) = self.frame_dec.as_mut() {
                fd.decoded = ImageBundle::new(None);
            }
            if return_full_image && !self.skipping_frame {
                return JxlDecoderStatus::FullImage;
            }
        }

        self.stage = DecoderStage::CodestreamFinished;
        // Return success, this means there is nothing more to do.
        JxlDecoderStatus::Success
    }

    /// Takes the RGB8 output buffer back from the frame's decoder state.
    fn reclaim_rgb_output(&mut self) {
        if let Some(ps) = self.passes_state.as_deref_mut() {
            if let Some(buf) = ps.rgb_output.take() {
                self.image_out_buffer = Some(buf);
            }
        }
    }

    /// Internal wrapper around jxl::ConvertToExternal which converts the
    /// stride, format and orientation. Translation of
    /// `ConvertImageInternal()` (for the color channels).
    fn convert_image_internal(&mut self) -> JxlDecoderStatus {
        // TODO(lode): handle mismatch of RGB/grayscale color profiles and pixel data
        // color/grayscale format
        let format = self.image_out_format;
        let stride = self.get_stride(&format);

        let float_format =
            format.data_type == JxlDataType::Float || format.data_type == JxlDataType::Float16;

        let undo_orientation = if self.keep_orientation {
            Orientation::Identity
        } else {
            self.metadata.m.get_orientation()
        };
        let little_endian = format.endianness != JxlEndianness::Big;
        let Some(frame) = self.frame_dec.as_ref().map(|f| &f.decoded) else {
            return JxlDecoderStatus::Error;
        };
        let Some(out) = self.image_out_buffer.as_mut() else {
            return JxlDecoderStatus::Error;
        };
        let size = self.image_out_size.min(out.len());
        let status = convert_to_external(
            frame,
            bits_per_channel(format.data_type),
            float_format,
            format.num_channels as usize,
            little_endian,
            stride,
            &mut out[..size],
            undo_orientation,
            self.unpremul_alpha,
        );
        if status.is_ok() {
            JxlDecoderStatus::Success
        } else {
            JxlDecoderStatus::Error
        }
    }

    /// Parses a box header. Translation of `ParseBoxHeader()`.
    fn parse_box_header(
        &self,
        box_size: &mut u64,
        header_size: &mut u64,
        box_type: &mut [u8; 4],
    ) -> JxlDecoderStatus {
        let input = &self.input[self.next_in..self.next_in + self.avail_in];
        let size = self.avail_in;
        let mut pos = 0usize;
        if out_of_bounds(pos, 8, size) {
            *header_size = 8;
            return JxlDecoderStatus::NeedMoreInput;
        }
        let box_start = pos;
        // Box size, including this header itself.
        *box_size = load_be32(&input[pos..]) as u64;
        pos += 4;
        if *box_size == 1 {
            *header_size = 16;
            if out_of_bounds(pos, 12, size) {
                return JxlDecoderStatus::NeedMoreInput;
            }
            *box_size = load_be64(&input[pos..]);
            pos += 8;
        }
        box_type.copy_from_slice(&input[pos..pos + 4]);
        pos += 4;
        *header_size = (pos - box_start) as u64;
        if *box_size > 0 && *box_size < *header_size {
            return JxlDecoderStatus::Error; // "invalid box size"
        }
        if sum_overflows(self.file_pos, pos, *box_size as usize) || *box_size > usize::MAX as u64 {
            return JxlDecoderStatus::Error; // "Box size overflow"
        }
        JxlDecoderStatus::Success
    }

    /// This includes handling the codestream if it is not a box-based jxl
    /// file. Translation of `HandleBoxes()`.
    fn handle_boxes(&mut self) -> JxlDecoderStatus {
        // Box handling loop
        loop {
            if self.box_stage != BoxStage::Header {
                self.advance_input(self.header_size);
                self.header_size = 0;
                // (No box output, no Exif/XMP storage for JPEG reconstruction.)
            }

            if self.box_stage == BoxStage::Header {
                if !self.have_container {
                    if self.stage == DecoderStage::CodestreamFinished {
                        return JxlDecoderStatus::Success;
                    }
                    self.box_stage = BoxStage::Codestream;
                    self.box_contents_unbounded = true;
                    continue;
                }
                if self.avail_in == 0 {
                    if self.stage != DecoderStage::CodestreamFinished {
                        // Not yet seen (all) codestream boxes.
                        return JxlDecoderStatus::NeedMoreInput;
                    }
                    if self.input_closed {
                        return JxlDecoderStatus::Success;
                    }
                    if self.events_wanted & JXL_DEC_BOX == 0 {
                        // All codestream and jbrd metadata boxes finished, and no individual
                        // boxes requested by user, so no need to request any more input.
                        // This returns success for backwards compatibility, when
                        // JxlDecoderCloseInput and JXL_DEC_BOX did not exist, as well
                        // as for efficiency.
                        return JxlDecoderStatus::Success;
                    }
                    // Even though we are exactly at a box end, there still may be more
                    // boxes. The user may call JxlDecoderCloseInput to indicate the input
                    // is finished and get success instead.
                    return JxlDecoderStatus::NeedMoreInput;
                }

                // (boxed_codestream_done needs JXL_DEC_BOX, never subscribed.)

                let mut box_size: u64 = 0;
                let mut header_size: u64 = 0;
                let mut box_type = [0u8; 4];
                let status = self.parse_box_header(&mut box_size, &mut header_size, &mut box_type);
                self.box_type = box_type;
                if status != JxlDecoderStatus::Success {
                    if status == JxlDecoderStatus::NeedMoreInput {
                        self.basic_info_size_hint = initial_basic_info_size_hint()
                            .wrapping_add(header_size as usize)
                            .wrapping_sub(self.file_pos);
                    }
                    return status;
                }
                if &self.box_type == b"brob" {
                    if self.avail_in < header_size as usize + 4 {
                        return JxlDecoderStatus::NeedMoreInput;
                    }
                    let s = self.next_in + header_size as usize;
                    self.box_decoded_type.copy_from_slice(&self.input[s..s + 4]);
                } else {
                    self.box_decoded_type = self.box_type;
                }

                // Box order validity checks
                // The signature box at box_count == 1 is not checked here since that's
                // already done at the beginning.
                self.box_count += 1;
                if self.box_count == 2 && &self.box_type != b"ftyp" {
                    return JxlDecoderStatus::Error; // "the second box must be the ftyp box"
                }
                if &self.box_type == b"ftyp" && self.box_count != 2 {
                    return JxlDecoderStatus::Error; // "the ftyp box must come second"
                }

                self.box_contents_unbounded = box_size == 0;
                self.box_contents_begin = self.file_pos + header_size as usize;
                self.box_contents_end = if self.box_contents_unbounded {
                    0
                } else {
                    self.file_pos + box_size as usize
                };
                self.box_contents_size = if self.box_contents_unbounded {
                    0
                } else {
                    (box_size - header_size) as usize
                };
                self.header_size = header_size as usize;

                if &self.box_type == b"ftyp" {
                    self.box_stage = BoxStage::Ftyp;
                } else if &self.box_type == b"jxlc" {
                    if self.last_codestream_seen {
                        return JxlDecoderStatus::Error; // "there can only be one jxlc box"
                    }
                    self.last_codestream_seen = true;
                    self.box_stage = BoxStage::Codestream;
                } else if &self.box_type == b"jxlp" {
                    self.box_stage = BoxStage::PartialCodestream;
                } else {
                    self.box_stage = BoxStage::Skip;
                }
            } else if self.box_stage == BoxStage::Ftyp {
                if self.box_contents_size < 12 {
                    return JxlDecoderStatus::Error; // "file type box too small"
                }
                if self.avail_in < 4 {
                    return JxlDecoderStatus::NeedMoreInput;
                }
                if &self.input[self.next_in..self.next_in + 4] != b"jxl " {
                    return JxlDecoderStatus::Error; // "file type box major brand must be \"jxl \""
                }
                self.advance_input(4);
                self.box_stage = BoxStage::Skip;
            } else if self.box_stage == BoxStage::PartialCodestream {
                if self.last_codestream_seen {
                    return JxlDecoderStatus::Error; // "cannot have jxlp box after last jxlp box"
                }
                // TODO(lode): error if box is unbounded but last bit not set
                if self.avail_in < 4 {
                    return JxlDecoderStatus::NeedMoreInput;
                }
                if !self.box_contents_unbounded && self.box_contents_size < 4 {
                    return JxlDecoderStatus::Error; // "jxlp box too small to contain index"
                }
                let jxlp_index = load_be32(&self.input[self.next_in..]);
                // The high bit of jxlp_index indicates whether this is the last
                // jxlp box.
                if jxlp_index & 0x80000000 != 0 {
                    self.last_codestream_seen = true;
                }
                self.advance_input(4);
                self.box_stage = BoxStage::Codestream;
            } else if self.box_stage == BoxStage::Codestream {
                let status = self.process_codestream();
                if status == JxlDecoderStatus::NeedMoreInput
                    && self.file_pos == self.box_contents_end
                    && !self.box_contents_unbounded
                {
                    self.box_stage = BoxStage::Header;
                    continue;
                }

                if status == JxlDecoderStatus::Success && self.box_contents_unbounded {
                    // Last box reached and codestream done, nothing more to do.
                    break;
                }
                return status;
            } else {
                // (BoxStage::Skip)
                if self.box_contents_unbounded {
                    if self.input_closed {
                        return JxlDecoderStatus::Success;
                    }
                    // An unbounded box is always the last box. Not requesting box data,
                    // so return success even if JxlDecoderCloseInput was not called for
                    // backwards compatibility as well as efficiency since this box is
                    // being skipped.
                    return JxlDecoderStatus::Success;
                }
                // Amount of remaining bytes in the box that is being skipped.
                let remaining = self.box_contents_end - self.file_pos;
                if self.avail_in < remaining {
                    // Indicate how many more bytes needed starting from next_in.
                    self.basic_info_size_hint =
                        initial_basic_info_size_hint() + self.box_contents_end - self.file_pos;
                    // Don't have the full box yet, skip all we have so far
                    self.advance_input(self.avail_in);
                    return JxlDecoderStatus::NeedMoreInput;
                } else {
                    // Full box available, skip all its remaining bytes
                    self.advance_input(remaining);
                    self.box_stage = BoxStage::Header;
                }
            }
        }

        JxlDecoderStatus::Success
    }

    /// Translation of `JxlDecoderProcessInput()`.
    pub(crate) fn process_input(&mut self) -> JxlDecoderStatus {
        if self.stage == DecoderStage::Inited {
            self.stage = DecoderStage::Started;
        }

        if !self.got_signature {
            let sig = jxl_signature_check(&self.input[self.next_in..self.next_in + self.avail_in]);
            if sig == JxlSignature::Invalid {
                return JxlDecoderStatus::Error; // "invalid signature"
            }
            if sig == JxlSignature::NotEnoughBytes {
                if self.input_closed {
                    return JxlDecoderStatus::Error; // "file too small for signature"
                }
                return JxlDecoderStatus::NeedMoreInput;
            }

            self.got_signature = true;

            if sig == JxlSignature::Container {
                self.have_container = true;
            } else {
                self.last_codestream_seen = true;
            }
        }

        let status = self.handle_boxes();

        if status == JxlDecoderStatus::NeedMoreInput && self.input_closed {
            return JxlDecoderStatus::Error; // "missing input"
        }

        // Even if the box handling returns success, certain types of
        // data may be missing.
        if status == JxlDecoderStatus::Success && self.stage != DecoderStage::CodestreamFinished {
            return JxlDecoderStatus::Error; // "codestream never finished"
        }

        status
    }

    /// Translation of `JxlDecoderGetBasicInfo()`.
    pub(crate) fn get_basic_info(&self, info: &mut JxlBasicInfo) -> JxlDecoderStatus {
        if !self.got_basic_info {
            return JxlDecoderStatus::NeedMoreInput;
        }

        *info = JxlBasicInfo::default();

        let meta = &self.metadata.m;

        info.have_container = self.have_container;
        info.xsize = self.metadata.size.xsize() as u32;
        info.ysize = self.metadata.size.ysize() as u32;

        info.bits_per_sample = meta.bit_depth.bits_per_sample;
        info.exponent_bits_per_sample = meta.bit_depth.exponent_bits_per_sample;

        info.have_animation = meta.have_animation;
        let orientation = meta.orientation;

        if !self.keep_orientation && orientation >= Orientation::Transpose as u32 {
            std::mem::swap(&mut info.xsize, &mut info.ysize);
        }

        if let Some(alpha) = meta.find(ExtraChannel::Alpha) {
            info.alpha_bits = alpha.bit_depth.bits_per_sample;
            info.alpha_premultiplied = alpha.alpha_associated;
        }

        info.num_color_channels = if meta.color_encoding.get_color_space()
            == super::color_encoding_internal::ColorSpace::Gray
        {
            1
        } else {
            3
        };

        info.num_extra_channels = meta.num_extra_channels;

        JxlDecoderStatus::Success
    }

    /// Translation of `PrepareSizeCheck()`.
    fn prepare_size_check(&self, format: &JxlPixelFormat, bits: &mut usize) -> JxlDecoderStatus {
        if !self.got_basic_info {
            // Don't know image dimensions yet, cannot check for valid size.
            return JxlDecoderStatus::NeedMoreInput;
        }
        if !self.coalescing && self.frame_stage == FrameStage::Header {
            return JxlDecoderStatus::Error; // "Don't know frame dimensions yet"
        }
        if format.num_channels > 4 {
            return JxlDecoderStatus::Error; // "More than 4 channels not supported"
        }

        *bits = bits_per_channel(format.data_type);

        JxlDecoderStatus::Success
    }

    /// Translation of `JxlDecoderImageOutBufferSize()`.
    pub(crate) fn image_out_buffer_size(
        &self,
        format: &JxlPixelFormat,
        size: &mut usize,
    ) -> JxlDecoderStatus {
        let mut bits = 0usize;
        let status = self.prepare_size_check(format, &mut bits);
        if status != JxlDecoderStatus::Success {
            return status;
        }
        if format.num_channels < 3 && !self.image_metadata.color_encoding.is_gray() {
            return JxlDecoderStatus::Error; // "Number of channels is too low for color output"
        }
        let (xsize, ysize) = self.get_current_dimensions(true);
        let mut row_size = div_ceil(xsize * format.num_channels as usize * bits, K_BITS_PER_BYTE);
        if format.align > 1 {
            row_size = div_ceil(row_size, format.align) * format.align;
        }
        *size = row_size * ysize;

        JxlDecoderStatus::Success
    }

    /// Translation of `JxlDecoderSetImageOutBuffer()`: the decoder takes the
    /// buffer, and gives it back with [`take_image_out_buffer`].
    pub(crate) fn set_image_out_buffer(
        &mut self,
        format: &JxlPixelFormat,
        buffer: Vec<u8>,
    ) -> JxlDecoderStatus {
        if !self.got_basic_info || (self.orig_events_wanted & JXL_DEC_FULL_IMAGE) == 0 {
            return JxlDecoderStatus::Error; // "No image out buffer needed at this time"
        }
        if format.num_channels < 3 && !self.image_metadata.color_encoding.is_gray() {
            return JxlDecoderStatus::Error; // "Number of channels is too low for color output"
        }
        let mut min_size = 0usize;
        // This also checks whether the format is valid and supported and basic info
        // is available.
        let status = self.image_out_buffer_size(format, &mut min_size);
        if status != JxlDecoderStatus::Success {
            return status;
        }

        if buffer.len() < min_size {
            return JxlDecoderStatus::Error;
        }

        self.image_out_buffer_set = true;
        self.image_out_size = buffer.len();
        self.image_out_buffer = Some(buffer);
        self.image_out_format = *format;

        JxlDecoderStatus::Success
    }

    /// The image out buffer (upstream, the caller keeps its pointer).
    pub(crate) fn take_image_out_buffer(&mut self) -> Option<Vec<u8>> {
        self.reclaim_rgb_output();
        self.image_out_buffer.take()
    }
}

/// Returns JXL_DEC_SUCCESS if the full bundle was successfully read, status
/// indicating either error or need more input otherwise. Translation of
/// `ReadBundle()` (with `CanRead()`; `NeedMoreInput` stands for
/// `RequestMoreInput()`).
fn read_bundle(data: &[u8], reader: &mut BitReader<'_>, t: &mut dyn Fields) -> JxlDecoderStatus {
    // Use a copy of the bit reader because CanRead advances bits.
    let mut reader2 = BitReader::new(data);
    reader2.skip_bits(reader.total_bits_consumed());
    let result = bundle_can_read(&mut reader2, t);
    let _ = reader2.close();
    if !result {
        return JxlDecoderStatus::NeedMoreInput;
    }
    if bundle_read(reader, t).is_err() {
        return JxlDecoderStatus::Error;
    }
    JxlDecoderStatus::Success
}

/// The deleter of `GetBitReader()`.
fn close_reader(mut reader: BitReader<'_>) {
    // We can't allow Close to abort the program if the reader is out of
    // bounds, or all return paths in the code, even those that already
    // return failure, would have to manually call AllReadsWithinBounds().
    // Invalid JXL codestream should not cause program to quit.
    let _ = reader.all_reads_within_bounds();
    let _ = reader.close();
}
