// Rust translation of src/libFLAC/stream_decoder.c (and the declarations
// of include/FLAC/stream_decoder.h and
// src/libFLAC/include/protected/stream_decoder.h) from libFLAC 1.3.4
// (https://xiph.org/flac/, at the revision SDL_mixer's external/flac pins).
// Copyright (C) 2000-2009  Josh Coalson
// Copyright (C) 2011-2016  Xiph.Org Foundation
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The stream decoder, as upstream's build has it (`FLAC__HAS_OGG` 0, the
//! plain C restore functions, no MD5 checking; see the module's
//! documentation). C's callbacks and `client_data` are the
//! [`StreamDecoderClient`] passed to each call that may run them; a write
//! callback gets the decoder itself, which C's gets as its first argument
//! (SDL_mixer's seeks from there), and reads the decoded signal through
//! [`StreamDecoder::output()`] from the offset it is given (C's `buffer`
//! pointers). The decoder object is always fully allocated (Rust's
//! allocation of it doesn't fail), so `FLAC__stream_decoder_new()` can't
//! return NULL; the decoder's buffers are allocated fallibly, as upstream's
//! `malloc()`s may fail.

use super::bitmath::FLAC__bitmath_ilog2;
use super::bitreader::{BitReader, BitReaderReadCallback};
use super::crc::{FLAC__crc8, FLAC__CRC16_UPDATE};
use super::fixed::FLAC__fixed_restore_signal;
use super::format::*;
use super::lpc::{FLAC__lpc_restore_signal, FLAC__lpc_restore_signal_wide};
use super::*;

/***********************************************************************
 *
 * Private static data
 *
 ***********************************************************************/

const ID3V2_TAG_: [u8; 3] = *b"ID3";

/// State values for a [`StreamDecoder`] (`FLAC__StreamDecoderState`).
///
/// The decoder's state can be obtained by calling
/// [`StreamDecoder::get_state()`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[allow(dead_code)] // (the Ogg state isn't reached without Ogg FLAC)
pub(crate) enum StreamDecoderState {
    /// The decoder is ready to search for metadata.
    SearchForMetadata = 0,
    /// The decoder is ready to or is in the process of reading metadata.
    ReadMetadata,
    /// The decoder is ready to or is in the process of searching for the
    /// frame sync code.
    SearchForFrameSync,
    /// The decoder is ready to or is in the process of reading a frame.
    ReadFrame,
    /// The decoder has reached the end of the stream.
    EndOfStream,
    /// An error occurred in the underlying Ogg layer.
    OggError,
    /// An error occurred while seeking.  The decoder must be flushed
    /// with `FLAC__stream_decoder_flush()` or reset with
    /// `FLAC__stream_decoder_reset()` before decoding can continue.
    SeekError,
    /// The decoder was aborted by the read or write callback.
    Aborted,
    /// An error occurred allocating memory.  The decoder is in an invalid
    /// state and can no longer be used.
    MemoryAllocationError,
    /// The decoder is in the uninitialized state; one of the
    /// `FLAC__stream_decoder_init_*()` functions must be called before
    /// samples can be processed.
    Uninitialized,
}

/// Possible return values for the `FLAC__stream_decoder_init_*()` functions
/// (`FLAC__StreamDecoderInitStatus`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[allow(dead_code)] // (the stdio and invalid-callback statuses aren't reached)
pub(crate) enum StreamDecoderInitStatus {
    /// Initialization was successful.
    Ok = 0,
    /// The library was not compiled with support for the given container
    /// format.
    UnsupportedContainer,
    /// A required callback was not supplied.
    InvalidCallbacks,
    /// An error occurred allocating memory.
    MemoryAllocationError,
    /// fopen() failed in `FLAC__stream_decoder_init_file()` or
    /// `FLAC__stream_decoder_init_ogg_file()`.
    ErrorOpeningFile,
    /// `FLAC__stream_decoder_init_*()` was called when the decoder was
    /// already initialized, usually because
    /// `FLAC__stream_decoder_finish()` was not called.
    AlreadyInitialized,
}

/// Return values for the read callback (`FLAC__StreamDecoderReadStatus`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum StreamDecoderReadStatus {
    /// The read was OK and decoding can continue.
    Continue,
    /// The read was attempted while at the end of the stream.  Note that
    /// the client must only return this value when the read callback was
    /// called when already at the end of the stream.  Otherwise, if the read
    /// itself moves to the end of the stream, the client should still return
    /// the data and `Continue`, and then on the next read callback it should
    /// return `EndOfStream` with a byte count of `0`.
    EndOfStream,
    /// An unrecoverable error occurred.  The decoder will return from the
    /// process call.
    Abort,
}

/// Return values for the seek callback (`FLAC__StreamDecoderSeekStatus`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[allow(dead_code)] // (SDL_mixer's streams can always seek)
pub(crate) enum StreamDecoderSeekStatus {
    /// The seek was OK and decoding can continue.
    Ok,
    /// An unrecoverable error occurred.  The decoder will return from the
    /// process call.
    Error,
    /// Client does not support seeking.
    Unsupported,
}

/// Return values for the tell callback (`FLAC__StreamDecoderTellStatus`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[allow(dead_code)] // (likewise)
pub(crate) enum StreamDecoderTellStatus {
    /// The tell was OK and decoding can continue.
    Ok,
    /// An unrecoverable error occurred.  The decoder will return from the
    /// process call.
    Error,
    /// Client does not support telling the position.
    Unsupported,
}

/// Return values for the length callback
/// (`FLAC__StreamDecoderLengthStatus`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[allow(dead_code)] // (likewise)
pub(crate) enum StreamDecoderLengthStatus {
    /// The length call was OK and decoding can continue.
    Ok,
    /// An unrecoverable error occurred.  The decoder will return from the
    /// process call.
    Error,
    /// Client does not support reporting the length.
    Unsupported,
}

/// Return values for the write callback (`FLAC__StreamDecoderWriteStatus`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum StreamDecoderWriteStatus {
    /// The write was OK and decoding can continue.
    Continue,
    /// An unrecoverable error occurred.  The decoder will return from the
    /// process call.
    Abort,
}

/// Possible values passed back to the error callback
/// (`FLAC__StreamDecoderErrorStatus`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum StreamDecoderErrorStatus {
    /// An error in the stream caused the decoder to lose synchronization.
    LostSync,
    /// The decoder encountered a corrupted frame header.
    BadHeader,
    /// The frame's data did not match the CRC in the footer.
    FrameCrcMismatch,
    /// The decoder encountered reserved fields in use in the stream.
    UnparseableStream,
}

/// The callbacks given to `FLAC__stream_decoder_init_stream()` and their
/// `client_data` (all of them: SDL_mixer gives them all, so the decoder's
/// checks for missing ones are left out).
pub(crate) trait StreamDecoderClient {
    /// `FLAC__StreamDecoderReadCallback`: read up to `*bytes` bytes into
    /// `buffer`, setting `*bytes` to the number read.
    fn read(&mut self, buffer: &mut [u8], bytes: &mut usize) -> StreamDecoderReadStatus;
    /// `FLAC__StreamDecoderSeekCallback`.
    fn seek(&mut self, absolute_byte_offset: u64) -> StreamDecoderSeekStatus;
    /// `FLAC__StreamDecoderTellCallback`.
    fn tell(&mut self, absolute_byte_offset: &mut u64) -> StreamDecoderTellStatus;
    /// `FLAC__StreamDecoderLengthCallback`.
    fn length(&mut self, stream_length: &mut u64) -> StreamDecoderLengthStatus;
    /// `FLAC__StreamDecoderEofCallback`.
    fn eof(&mut self) -> bool;
    /// `FLAC__StreamDecoderWriteCallback`: the decoded frame is
    /// `decoder.output()`'s channels from `offset` on, `frame.blocksize`
    /// samples each.
    fn write(
        &mut self,
        decoder: &mut StreamDecoder,
        frame: &FrameHeader,
        offset: usize,
    ) -> StreamDecoderWriteStatus;
    /// `FLAC__StreamDecoderMetadataCallback`.
    fn metadata(&mut self, metadata: &StreamMetadata<'_>);
    /// `FLAC__StreamDecoderErrorCallback`.
    fn error(&mut self, status: StreamDecoderErrorStatus);
}

/// `FLAC__StreamDecoder` (its `protected_` and `private_` parts as one;
/// the fields of the Ogg, stdio, CPU-detection and MD5 parts upstream's
/// build doesn't use are left out).
pub(crate) struct StreamDecoder {
    /* protected_ */
    state: StreamDecoderState,
    channels: u32,
    channel_assignment: ChannelAssignment,
    bits_per_sample: u32,
    sample_rate: u32,   /* in Hz */
    blocksize: u32,     /* in samples (per channel) */
    md5_checking: bool, /* if true, generate MD5 signature of decoded data and compare against signature in the STREAMINFO metadata block */
    /* private_ */
    input: BitReader,
    output: [Vec<i32>; FLAC__MAX_CHANNELS],
    residual: [Vec<i32>; FLAC__MAX_CHANNELS],
    partitioned_rice_contents: [PartitionedRiceContents; FLAC__MAX_CHANNELS],
    output_capacity: u32,
    output_channels: u32,
    fixed_block_size: u32,
    next_fixed_block_size: u32,
    samples_decoded: u64,
    has_stream_info: bool,
    has_seek_table: bool,
    /// `stream_info` (a `FLAC__StreamMetadata` upstream: its data, and
    /// its `is_last` and `length` below)
    stream_info: StreamInfo,
    stream_info_is_last: bool,
    stream_info_length: u32,
    /// `seek_table` (likewise)
    seek_table: SeekTable,
    seek_table_is_last: bool,
    seek_table_length: u32,
    metadata_filter: [bool; 128], /* MAGIC number 128 == total number of metadata block types == 1 << 7 */
    metadata_filter_ids: Vec<[u8; 4]>,
    frame: Frame,
    cached: bool,              /* true if there is a byte in lookahead */
    header_warmup: [u8; 2],    /* contains the sync code and reserved bits */
    lookahead: u8,             /* temp storage when we need to look ahead one byte in the stream */
    do_md5_checking: bool, /* initially gets protected_->md5_checking but is turned off after a seek or if the metadata has a zero MD5 */
    internal_reset_hack: bool, /* used only during init() so we can call reset to set up the decoder without rewinding the input */
    is_seeking: bool,
    /* (the rest of these are only used for seeking) */
    last_frame: FrameHeader, /* holds the info of the last frame we seeked to */
    first_frame_offset: u64, /* hint to the seek routine of where in the stream the first audio frame starts */
    target_sample: u64,
    unparseable_frame_count: u32, /* used to tell whether we're decoding a future version of FLAC or just got a bad sync */
}

impl std::fmt::Debug for StreamDecoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StreamDecoder")
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

/// `read_callback_()` with what it uses of the decoder: the bit reader's
/// read callback, built for each bit reader call (see [`cb!`]).
struct ReadCallback<'a> {
    state: &'a mut StreamDecoderState,
    is_seeking: bool,
    unparseable_frame_count: u32,
    client: &'a mut dyn StreamDecoderClient,
}

/// The bit reader's callback for a decoder method's bit reader call:
/// `cb!(self, client)`.
macro_rules! cb {
    ($d:expr, $client:expr) => {
        &mut ReadCallback {
            state: &mut $d.state,
            is_seeking: $d.is_seeking,
            unparseable_frame_count: $d.unparseable_frame_count,
            client: &mut *$client,
        }
    };
}

/// A double converted to a 64-bit integer as x86-64's `cvttsd2si` does
/// (C's cast, undefined when out of range): `INT64_MIN` when it doesn't
/// fit.
fn c_i64(x: f64) -> i64 {
    if x >= -9_223_372_036_854_775_808.0 && x < 9_223_372_036_854_775_808.0 {
        x as i64
    } else {
        i64::MIN
    }
}

/// A zeroed `len`-element vector, `None` when it can't be allocated
/// (C's `malloc()` failing).
fn try_vec<T: Clone + Default>(len: usize) -> Option<Vec<T>> {
    crate::internal::try_alloc(len).ok()
}

/***********************************************************************
 *
 * Class constructor/destructor
 *
 ***********************************************************************/
impl StreamDecoder {
    /// `FLAC__stream_decoder_new()`.
    pub(crate) fn new() -> Box<StreamDecoder> {
        let mut decoder = Box::new(StreamDecoder {
            state: StreamDecoderState::Uninitialized,
            channels: 0,
            channel_assignment: ChannelAssignment::Independent,
            bits_per_sample: 0,
            sample_rate: 0,
            blocksize: 0,
            md5_checking: false,
            input: BitReader::new(),
            output: Default::default(),
            residual: Default::default(),
            partitioned_rice_contents: Default::default(),
            output_capacity: 0,
            output_channels: 0,
            fixed_block_size: 0,
            next_fixed_block_size: 0,
            samples_decoded: 0,
            has_stream_info: false,
            has_seek_table: false,
            stream_info: StreamInfo::default(),
            stream_info_is_last: false,
            stream_info_length: 0,
            seek_table: SeekTable::default(),
            seek_table_is_last: false,
            seek_table_length: 0,
            metadata_filter: [false; 128],
            metadata_filter_ids: Vec::new(),
            frame: Frame::default(),
            cached: false,
            header_warmup: [0; 2],
            lookahead: 0,
            do_md5_checking: false,
            internal_reset_hack: false,
            is_seeking: false,
            last_frame: FrameHeader::default(),
            first_frame_offset: 0,
            target_sample: 0,
            unparseable_frame_count: 0,
        });

        for i in 0..FLAC__MAX_CHANNELS {
            FLAC__format_entropy_coding_method_partitioned_rice_contents_init(
                &mut decoder.partitioned_rice_contents[i],
            );
        }

        decoder.set_defaults_();

        decoder.state = StreamDecoderState::Uninitialized;

        decoder
    }

    /// `FLAC__stream_decoder_delete()`.
    pub(crate) fn delete(mut self: Box<Self>) {
        let _ = self.finish();

        for i in 0..FLAC__MAX_CHANNELS {
            FLAC__format_entropy_coding_method_partitioned_rice_contents_clear(
                &mut self.partitioned_rice_contents[i],
            );
        }
    }

    /***********************************************************************
     *
     * Public class methods
     *
     ***********************************************************************/

    fn init_stream_internal_(&mut self, is_ogg: bool) -> StreamDecoderInitStatus {
        if self.state != StreamDecoderState::Uninitialized {
            return StreamDecoderInitStatus::AlreadyInitialized;
        }

        if is_ogg {
            // (FLAC__HAS_OGG == 0)
            return StreamDecoderInitStatus::UnsupportedContainer;
        }

        // (the callbacks are all there: a StreamDecoderClient has them.)

        /*
         * get the CPU info and set the function pointers
         */
        // (the non-asm routines are the only ones translated, and the
        // only ones upstream picks on x86-64: "No useful SSE optimizations
        // yet".)

        /* from here on, errors are fatal */

        if !self.input.init() {
            self.state = StreamDecoderState::MemoryAllocationError;
            return StreamDecoderInitStatus::MemoryAllocationError;
        }

        self.fixed_block_size = 0;
        self.next_fixed_block_size = 0;
        self.samples_decoded = 0;
        self.has_stream_info = false;
        self.cached = false;

        self.do_md5_checking = self.md5_checking;
        self.is_seeking = false;

        self.internal_reset_hack = true; /* so the following reset does not try to rewind the input */
        if !self.reset(None) {
            /* above call sets the state for us */
            return StreamDecoderInitStatus::MemoryAllocationError;
        }

        StreamDecoderInitStatus::Ok
    }

    /// `FLAC__stream_decoder_init_stream()`. The callbacks are the
    /// [`StreamDecoderClient`] given to the calls that process the stream.
    pub(crate) fn init_stream(&mut self) -> StreamDecoderInitStatus {
        self.init_stream_internal_(/*is_ogg=*/ false)
    }

    /// `FLAC__stream_decoder_init_ogg_stream()`: Ogg FLAC isn't in the
    /// build, so this is `FLAC__STREAM_DECODER_INIT_STATUS_UNSUPPORTED_CONTAINER`.
    pub(crate) fn init_ogg_stream(&mut self) -> StreamDecoderInitStatus {
        self.init_stream_internal_(/*is_ogg=*/ true)
    }

    /// `FLAC__stream_decoder_finish()`.
    pub(crate) fn finish(&mut self) -> bool {
        let md5_failed = false;

        if self.state == StreamDecoderState::Uninitialized {
            return true;
        }

        /* see the comment in FLAC__stream_decoder_reset() as to why we
         * always call FLAC__MD5Final()
         */
        // (no MD5 here)

        self.seek_table.points = None;
        self.has_seek_table = false;

        self.input.free();
        for i in 0..FLAC__MAX_CHANNELS {
            self.output[i] = Vec::new();
            self.residual[i] = Vec::new();
        }
        self.output_capacity = 0;
        self.output_channels = 0;

        // (do_md5_checking is never on: SDL_mixer doesn't ask for it.)
        self.is_seeking = false;

        self.set_defaults_();

        self.state = StreamDecoderState::Uninitialized;

        !md5_failed
    }

    /// `FLAC__stream_decoder_set_metadata_respond()`.
    pub(crate) fn set_metadata_respond(&mut self, type_: u32) -> bool {
        /* double protection */
        if type_ > FLAC__MAX_METADATA_TYPE_CODE {
            return false;
        }
        if self.state != StreamDecoderState::Uninitialized {
            return false;
        }
        self.metadata_filter[type_ as usize] = true;
        if type_ == FLAC__METADATA_TYPE_APPLICATION {
            self.metadata_filter_ids.clear();
        }
        true
    }

    /// `FLAC__stream_decoder_get_state()`.
    pub(crate) fn get_state(&self) -> StreamDecoderState {
        self.state
    }

    /// `FLAC__stream_decoder_get_total_samples()`.
    pub(crate) fn get_total_samples(&self) -> u64 {
        if self.has_stream_info {
            self.stream_info.total_samples
        } else {
            0
        }
    }

    /// `FLAC__stream_decoder_get_channels()`.
    pub(crate) fn get_channels(&self) -> u32 {
        self.channels
    }

    /// `FLAC__stream_decoder_get_bits_per_sample()`.
    pub(crate) fn get_bits_per_sample(&self) -> u32 {
        self.bits_per_sample
    }

    /// The decoded signal (C's `output` arrays, which the write callback
    /// gets as `buffer`).
    pub(crate) fn output(&self) -> &[Vec<i32>; FLAC__MAX_CHANNELS] {
        &self.output
    }

    /// `FLAC__stream_decoder_get_decode_position()`.
    pub(crate) fn get_decode_position(
        &mut self,
        client: &mut dyn StreamDecoderClient,
        position: &mut u64,
    ) -> bool {
        // (no Ogg FLAC, and the client can tell)
        if client.tell(position) != StreamDecoderTellStatus::Ok {
            return false;
        }
        /* should never happen since all FLAC frames and metadata blocks are byte aligned, but check just in case */
        if !self.input.is_consumed_byte_aligned() {
            return false;
        }
        *position = position.wrapping_sub(self.get_input_bytes_unconsumed() as u64);
        true
    }

    /// `FLAC__stream_decoder_flush()`.
    pub(crate) fn flush(&mut self) -> bool {
        if !self.internal_reset_hack && self.state == StreamDecoderState::Uninitialized {
            return false;
        }

        self.samples_decoded = 0;
        self.do_md5_checking = false;

        if !self.input.clear() {
            self.state = StreamDecoderState::MemoryAllocationError;
            return false;
        }
        self.state = StreamDecoderState::SearchForFrameSync;

        true
    }

    /// `FLAC__stream_decoder_reset()`: `client` is the one to rewind the
    /// input with (`None` only for `init()`'s reset, which doesn't).
    pub(crate) fn reset(&mut self, client: Option<&mut dyn StreamDecoderClient>) -> bool {
        if !self.flush() {
            /* above call sets the state for us */
            return false;
        }

        /* Rewind if necessary.  If FLAC__stream_decoder_init() is calling us,
         * (internal_reset_hack) don't try to rewind since we are already at
         * the beginning of the stream and don't want to fail if the input is
         * not seekable.
         */
        if !self.internal_reset_hack {
            if let Some(client) = client {
                if client.seek(0) == StreamDecoderSeekStatus::Error {
                    return false; /* seekable and seek fails, reset fails */
                }
            }
        } else {
            self.internal_reset_hack = false;
        }

        self.state = StreamDecoderState::SearchForMetadata;

        self.has_stream_info = false;

        self.seek_table.points = None;
        self.has_seek_table = false;

        self.do_md5_checking = self.md5_checking;
        /*
         * This goes in reset() and not flush() because according to the spec, a
         * fixed-blocksize stream must stay that way through the whole stream.
         */
        self.fixed_block_size = 0;
        self.next_fixed_block_size = 0;

        /* We initialize the FLAC__MD5Context even though we may never use it.  This
         * is because md5 checking may be turned on to start and then turned off if
         * a seek occurs.  So we init the context here and finalize it in
         * FLAC__stream_decoder_finish() to make sure things are always cleaned up
         * properly.
         */
        // (no MD5 here)

        self.first_frame_offset = 0;
        self.unparseable_frame_count = 0;

        true
    }

    /// `FLAC__stream_decoder_process_single()`.
    pub(crate) fn process_single(&mut self, client: &mut dyn StreamDecoderClient) -> bool {
        let mut got_a_frame = false;

        loop {
            match self.state {
                StreamDecoderState::SearchForMetadata => {
                    if !self.find_metadata_(client) {
                        return false; /* above function sets the status for us */
                    }
                }
                StreamDecoderState::ReadMetadata => {
                    if !self.read_metadata_(client) {
                        return false; /* above function sets the status for us */
                    } else {
                        return true;
                    }
                }
                StreamDecoderState::SearchForFrameSync => {
                    if !self.frame_sync_(client) {
                        return true; /* above function sets the status for us */
                    }
                }
                StreamDecoderState::ReadFrame => {
                    if !self.read_frame_(client, &mut got_a_frame, /*do_full_decode=*/ true) {
                        return false; /* above function sets the status for us */
                    }
                    if got_a_frame {
                        return true; /* above function sets the status for us */
                    }
                }
                StreamDecoderState::EndOfStream | StreamDecoderState::Aborted => {
                    return true;
                }
                _ => {
                    return false;
                }
            }
        }
    }

    /// `FLAC__stream_decoder_process_until_end_of_metadata()`.
    pub(crate) fn process_until_end_of_metadata(
        &mut self,
        client: &mut dyn StreamDecoderClient,
    ) -> bool {
        loop {
            match self.state {
                StreamDecoderState::SearchForMetadata => {
                    if !self.find_metadata_(client) {
                        return false; /* above function sets the status for us */
                    }
                }
                StreamDecoderState::ReadMetadata => {
                    if !self.read_metadata_(client) {
                        return false; /* above function sets the status for us */
                    }
                }
                StreamDecoderState::SearchForFrameSync
                | StreamDecoderState::ReadFrame
                | StreamDecoderState::EndOfStream
                | StreamDecoderState::Aborted => {
                    return true;
                }
                _ => {
                    return false;
                }
            }
        }
    }

    /// `FLAC__stream_decoder_seek_absolute()`.
    pub(crate) fn seek_absolute(
        &mut self,
        client: &mut dyn StreamDecoderClient,
        sample: u64,
    ) -> bool {
        let mut length: u64 = 0;

        if self.state != StreamDecoderState::SearchForMetadata
            && self.state != StreamDecoderState::ReadMetadata
            && self.state != StreamDecoderState::SearchForFrameSync
            && self.state != StreamDecoderState::ReadFrame
            && self.state != StreamDecoderState::EndOfStream
        {
            return false;
        }

        // (the client can seek)

        if self.get_total_samples() > 0 && sample >= self.get_total_samples() {
            return false;
        }

        self.is_seeking = true;

        /* turn off md5 checking if a seek is attempted */
        self.do_md5_checking = false;

        /* get the file length (currently our algorithm needs to know the length so it's also an error to get FLAC__STREAM_DECODER_LENGTH_STATUS_UNSUPPORTED) */
        if client.length(&mut length) != StreamDecoderLengthStatus::Ok {
            self.is_seeking = false;
            return false;
        }

        /* if we haven't finished processing the metadata yet, do that so we have the STREAMINFO, SEEK_TABLE, and first_frame_offset */
        if self.state == StreamDecoderState::SearchForMetadata
            || self.state == StreamDecoderState::ReadMetadata
        {
            if !self.process_until_end_of_metadata(client) {
                /* above call sets the state for us */
                self.is_seeking = false;
                return false;
            }
            /* check this again in case we didn't know total_samples the first time */
            if self.get_total_samples() > 0 && sample >= self.get_total_samples() {
                self.is_seeking = false;
                return false;
            }
        }

        {
            let ok = self.seek_to_absolute_sample_(client, length, sample);
            self.is_seeking = false;
            ok
        }
    }

    /***********************************************************************
     *
     * Protected class methods
     *
     ***********************************************************************/

    /// `FLAC__stream_decoder_get_input_bytes_unconsumed()`.
    fn get_input_bytes_unconsumed(&self) -> u32 {
        self.input.get_input_bits_unconsumed() / 8
    }

    /***********************************************************************
     *
     * Private class methods
     *
     ***********************************************************************/

    fn set_defaults_(&mut self) {
        // (the callbacks are the client's)

        self.metadata_filter = [false; 128];
        self.metadata_filter[FLAC__METADATA_TYPE_STREAMINFO as usize] = true;
        self.metadata_filter_ids.clear();

        self.md5_checking = false;
    }

    /// `allocate_output_()`.
    fn allocate_output_(&mut self, size: u32, channels: u32) -> bool {
        if size <= self.output_capacity && channels <= self.output_channels {
            return true;
        }

        /* simply using realloc() is not practical because the number of channels may change mid-stream */

        for i in 0..FLAC__MAX_CHANNELS {
            self.output[i] = Vec::new();
            self.residual[i] = Vec::new();
        }

        for i in 0..channels as usize {
            /* WATCHOUT:
             * FLAC__lpc_restore_signal_asm_ia32_mmx() and ..._intrin_sseN()
             * require that the output arrays have a buffer of up to 3 zeroes
             * in front (at negative indices) for alignment purposes;
             * we use 4 to keep the data well-aligned.
             */
            // (those versions aren't translated, so there are no zeroes in front.)
            let (Some(output), Some(residual)) = (try_vec(size as usize), try_vec(size as usize))
            else {
                self.state = StreamDecoderState::MemoryAllocationError;
                // (upstream keeps the old capacity here, with the arrays
                // freed; with none, a later call allocates them again.)
                self.output_capacity = 0;
                self.output_channels = 0;
                return false;
            };
            self.output[i] = output;
            self.residual[i] = residual;
        }

        self.output_capacity = size;
        self.output_channels = channels;

        true
    }

    fn has_id_filtered_(&self, id: &[u8; 4]) -> bool {
        self.metadata_filter_ids.iter().any(|f| f == id)
    }

    fn find_metadata_(&mut self, client: &mut dyn StreamDecoderClient) -> bool {
        let mut x: u32 = 0;
        let mut i: u32 = 0;
        let mut id: u32 = 0;
        let mut first = true;

        while i < 4 {
            if self.cached {
                x = self.lookahead as u32;
                self.cached = false;
            } else {
                if !self.input.read_raw_uint32(&mut x, 8, cb!(self, client)) {
                    return false; /* read_callback_ sets the state for us */
                }
            }
            if x == FLAC__STREAM_SYNC_STRING[i as usize] as u32 {
                first = true;
                i += 1;
                id = 0;
                continue;
            }

            if id >= 3 {
                return false;
            }

            if x == ID3V2_TAG_[id as usize] as u32 {
                id += 1;
                i = 0;
                if id == 3 {
                    if !self.skip_id3v2_tag_(client) {
                        return false; /* skip_id3v2_tag_ sets the state for us */
                    }
                }
                continue;
            }
            id = 0;
            if x == 0xff {
                /* MAGIC NUMBER for the first 8 frame sync bits */
                self.header_warmup[0] = x as u8;
                if !self.input.read_raw_uint32(&mut x, 8, cb!(self, client)) {
                    return false; /* read_callback_ sets the state for us */
                }

                /* we have to check if we just read two 0xff's in a row; the second may actually be the beginning of the sync code */
                /* else we have to check if the second byte is the end of a sync code */
                if x == 0xff {
                    /* MAGIC NUMBER for the first 8 frame sync bits */
                    self.lookahead = x as u8;
                    self.cached = true;
                } else if x >> 1 == 0x7c {
                    /* MAGIC NUMBER for the last 6 sync bits and reserved 7th bit */
                    self.header_warmup[1] = x as u8;
                    self.state = StreamDecoderState::ReadFrame;
                    return true;
                }
            }
            i = 0;
            if first {
                self.send_error_to_client_(client, StreamDecoderErrorStatus::LostSync);
                first = false;
            }
        }

        self.state = StreamDecoderState::ReadMetadata;
        true
    }

    fn read_metadata_(&mut self, client: &mut dyn StreamDecoderClient) -> bool {
        let mut x: u32 = 0;
        let mut type_: u32 = 0;
        let mut length: u32 = 0;

        if !self
            .input
            .read_raw_uint32(&mut x, FLAC__STREAM_METADATA_IS_LAST_LEN, cb!(self, client))
        {
            return false; /* read_callback_ sets the state for us */
        }
        let is_last = x != 0;

        if !self.input.read_raw_uint32(
            &mut type_,
            FLAC__STREAM_METADATA_TYPE_LEN,
            cb!(self, client),
        ) {
            return false; /* read_callback_ sets the state for us */
        }

        if !self.input.read_raw_uint32(
            &mut length,
            FLAC__STREAM_METADATA_LENGTH_LEN,
            cb!(self, client),
        ) {
            return false; /* read_callback_ sets the state for us */
        }

        if type_ == FLAC__METADATA_TYPE_STREAMINFO {
            if !self.read_metadata_streaminfo_(client, is_last, length) {
                return false;
            }

            self.has_stream_info = true;
            if self.stream_info.md5sum == [0u8; 16] {
                self.do_md5_checking = false;
            }
            if !self.is_seeking && self.metadata_filter[FLAC__METADATA_TYPE_STREAMINFO as usize] {
                client.metadata(&StreamMetadata {
                    type_: FLAC__METADATA_TYPE_STREAMINFO,
                    is_last: self.stream_info_is_last,
                    length: self.stream_info_length,
                    data: StreamMetadataData::StreamInfo(&self.stream_info),
                });
            }
        } else if type_ == FLAC__METADATA_TYPE_SEEKTABLE {
            /* just in case we already have a seek table, and reading the next one fails: */
            self.has_seek_table = false;

            if !self.read_metadata_seektable_(client, is_last, length) {
                return false;
            }

            self.has_seek_table = true;
            if !self.is_seeking && self.metadata_filter[FLAC__METADATA_TYPE_SEEKTABLE as usize] {
                client.metadata(&StreamMetadata {
                    type_: FLAC__METADATA_TYPE_SEEKTABLE,
                    is_last: self.seek_table_is_last,
                    length: self.seek_table_length,
                    data: StreamMetadataData::SeekTable(&self.seek_table),
                });
            }
        } else {
            let mut skip_it = !self.metadata_filter[type_ as usize];
            let mut real_length = length;
            let mut application_id = [0u8; 4];

            if type_ == FLAC__METADATA_TYPE_APPLICATION {
                if !self
                    .input
                    .read_byte_block_aligned_no_crc(&mut application_id, cb!(self, client))
                {
                    return false; /* read_callback_ sets the state for us */
                }

                if real_length < FLAC__STREAM_METADATA_APPLICATION_ID_LEN / 8 {
                    /* underflow check */
                    self.state = StreamDecoderState::MemoryAllocationError; /*@@@@@@ maybe wrong error? need to resync?*/
                    return false;
                }

                real_length -= FLAC__STREAM_METADATA_APPLICATION_ID_LEN / 8;

                if !self.metadata_filter_ids.is_empty() && self.has_id_filtered_(&application_id) {
                    skip_it = !skip_it;
                }
            }

            if skip_it {
                if !self
                    .input
                    .skip_byte_block_aligned_no_crc(real_length, cb!(self, client))
                {
                    return false; /* read_callback_ sets the state for us */
                }
            } else {
                let mut ok = true;
                // (the block's data, freed after the callback as upstream frees it)
                let mut data: Option<Vec<u8>> = None;
                let mut vorbis_comment = VorbisComment::default();
                match type_ {
                    FLAC__METADATA_TYPE_PADDING => {
                        /* skip the padding bytes */
                        if !self
                            .input
                            .skip_byte_block_aligned_no_crc(real_length, cb!(self, client))
                        {
                            ok = false; /* read_callback_ sets the state for us */
                        }
                    }
                    FLAC__METADATA_TYPE_VORBIS_COMMENT => {
                        if !self.read_metadata_vorbiscomment_(
                            client,
                            &mut vorbis_comment,
                            real_length,
                        ) {
                            ok = false;
                        }
                    }
                    FLAC__METADATA_TYPE_CUESHEET | FLAC__METADATA_TYPE_PICTURE => {
                        // (read_metadata_cuesheet_() and read_metadata_picture_()
                        // aren't translated: SDL_mixer only asks for
                        // STREAMINFO and VORBIS_COMMENT blocks, so these are
                        // skipped above.)
                        unreachable!("CUESHEET and PICTURE blocks are never asked for");
                    }
                    _ => {
                        /* remember, we read the ID already */
                        // (APPLICATION and unknown blocks: the data)
                        if real_length > 0 {
                            match try_vec::<u8>(real_length as usize) {
                                None => {
                                    self.state = StreamDecoderState::MemoryAllocationError;
                                    ok = false;
                                }
                                Some(mut d) => {
                                    if !self
                                        .input
                                        .read_byte_block_aligned_no_crc(&mut d, cb!(self, client))
                                    {
                                        ok = false; /* read_callback_ sets the state for us */
                                    }
                                    data = Some(d);
                                }
                            }
                        }
                    }
                }
                if ok && !self.is_seeking {
                    let data = match type_ {
                        FLAC__METADATA_TYPE_PADDING => StreamMetadataData::Padding,
                        FLAC__METADATA_TYPE_APPLICATION => StreamMetadataData::Application {
                            id: application_id,
                            data: data.as_deref(),
                        },
                        FLAC__METADATA_TYPE_VORBIS_COMMENT => {
                            StreamMetadataData::VorbisComment(&vorbis_comment)
                        }
                        _ => StreamMetadataData::Unknown {
                            data: data.as_deref(),
                        },
                    };
                    client.metadata(&StreamMetadata {
                        type_,
                        is_last,
                        length,
                        data,
                    });
                }

                /* now we have to free any malloc()ed data in the block */
                // (dropping it)

                if !ok {
                    /* anything that unsets "ok" should also make sure decoder->protected_->state is updated */
                    return false;
                }
            }
        }

        if is_last {
            /* if this fails, it's OK, it's just a hint for the seek routine */
            let mut first_frame_offset = self.first_frame_offset;
            if !self.get_decode_position(client, &mut first_frame_offset) {
                first_frame_offset = 0;
            }
            self.first_frame_offset = first_frame_offset;
            self.state = StreamDecoderState::SearchForFrameSync;
        }

        true
    }

    fn read_metadata_streaminfo_(
        &mut self,
        client: &mut dyn StreamDecoderClient,
        is_last: bool,
        mut length: u32,
    ) -> bool {
        let mut x: u32 = 0;
        let mut bits: u32;
        let mut used_bits: u32 = 0;

        self.stream_info_is_last = is_last;
        self.stream_info_length = length;

        bits = FLAC__STREAM_METADATA_STREAMINFO_MIN_BLOCK_SIZE_LEN;
        if !self.input.read_raw_uint32(&mut x, bits, cb!(self, client)) {
            return false; /* read_callback_ sets the state for us */
        }
        self.stream_info.min_blocksize = x;
        used_bits += bits;

        bits = FLAC__STREAM_METADATA_STREAMINFO_MAX_BLOCK_SIZE_LEN;
        if !self.input.read_raw_uint32(
            &mut x,
            FLAC__STREAM_METADATA_STREAMINFO_MAX_BLOCK_SIZE_LEN,
            cb!(self, client),
        ) {
            return false; /* read_callback_ sets the state for us */
        }
        self.stream_info.max_blocksize = x;
        used_bits += bits;

        bits = FLAC__STREAM_METADATA_STREAMINFO_MIN_FRAME_SIZE_LEN;
        if !self.input.read_raw_uint32(
            &mut x,
            FLAC__STREAM_METADATA_STREAMINFO_MIN_FRAME_SIZE_LEN,
            cb!(self, client),
        ) {
            return false; /* read_callback_ sets the state for us */
        }
        self.stream_info.min_framesize = x;
        used_bits += bits;

        bits = FLAC__STREAM_METADATA_STREAMINFO_MAX_FRAME_SIZE_LEN;
        if !self.input.read_raw_uint32(
            &mut x,
            FLAC__STREAM_METADATA_STREAMINFO_MAX_FRAME_SIZE_LEN,
            cb!(self, client),
        ) {
            return false; /* read_callback_ sets the state for us */
        }
        self.stream_info.max_framesize = x;
        used_bits += bits;

        bits = FLAC__STREAM_METADATA_STREAMINFO_SAMPLE_RATE_LEN;
        if !self.input.read_raw_uint32(
            &mut x,
            FLAC__STREAM_METADATA_STREAMINFO_SAMPLE_RATE_LEN,
            cb!(self, client),
        ) {
            return false; /* read_callback_ sets the state for us */
        }
        self.stream_info.sample_rate = x;
        used_bits += bits;

        bits = FLAC__STREAM_METADATA_STREAMINFO_CHANNELS_LEN;
        if !self.input.read_raw_uint32(
            &mut x,
            FLAC__STREAM_METADATA_STREAMINFO_CHANNELS_LEN,
            cb!(self, client),
        ) {
            return false; /* read_callback_ sets the state for us */
        }
        self.stream_info.channels = x + 1;
        used_bits += bits;

        bits = FLAC__STREAM_METADATA_STREAMINFO_BITS_PER_SAMPLE_LEN;
        if !self.input.read_raw_uint32(
            &mut x,
            FLAC__STREAM_METADATA_STREAMINFO_BITS_PER_SAMPLE_LEN,
            cb!(self, client),
        ) {
            return false; /* read_callback_ sets the state for us */
        }
        self.stream_info.bits_per_sample = x + 1;
        used_bits += bits;

        bits = FLAC__STREAM_METADATA_STREAMINFO_TOTAL_SAMPLES_LEN;
        if !self.input.read_raw_uint64(
            &mut self.stream_info.total_samples,
            FLAC__STREAM_METADATA_STREAMINFO_TOTAL_SAMPLES_LEN,
            cb!(self, client),
        ) {
            return false; /* read_callback_ sets the state for us */
        }
        used_bits += bits;

        if !self
            .input
            .read_byte_block_aligned_no_crc(&mut self.stream_info.md5sum, cb!(self, client))
        {
            return false; /* read_callback_ sets the state for us */
        }
        used_bits += 16 * 8;

        /* skip the rest of the block */
        if length < (used_bits / 8) {
            return false; /* read_callback_ sets the state for us */
        }
        length -= used_bits / 8;
        if !self
            .input
            .skip_byte_block_aligned_no_crc(length, cb!(self, client))
        {
            return false; /* read_callback_ sets the state for us */
        }

        true
    }

    fn read_metadata_seektable_(
        &mut self,
        client: &mut dyn StreamDecoderClient,
        is_last: bool,
        mut length: u32,
    ) -> bool {
        let mut x: u32 = 0;
        let mut xx: u64 = 0;

        self.seek_table_is_last = is_last;
        self.seek_table_length = length;

        self.seek_table.num_points = length / FLAC__STREAM_METADATA_SEEKPOINT_LENGTH;

        /* use realloc since we may pass through here several times (e.g. after seeking) */
        let num_points = self.seek_table.num_points as usize;
        if num_points == 0 {
            // (safe_realloc_mul_2op_() is then realloc(points, 0), which on
            // glibc frees an existing array and returns NULL, the error
            // below, and gives malloc(0)'s pointer for a NULL one.)
            if self.seek_table.points.take().is_some() {
                self.state = StreamDecoderState::MemoryAllocationError;
                return false;
            }
            self.seek_table.points = Some(Vec::new());
        } else {
            let mut points = self.seek_table.points.take().unwrap_or_default();
            if points
                .try_reserve_exact(num_points.saturating_sub(points.len()))
                .is_err()
            {
                // (safe_realloc_() frees the old array when it fails.)
                self.state = StreamDecoderState::MemoryAllocationError;
                return false;
            }
            points.resize(num_points, SeekPoint::default());
            self.seek_table.points = Some(points);
        }
        for i in 0..num_points {
            if !self.input.read_raw_uint64(
                &mut xx,
                FLAC__STREAM_METADATA_SEEKPOINT_SAMPLE_NUMBER_LEN,
                cb!(self, client),
            ) {
                return false; /* read_callback_ sets the state for us */
            }
            if let Some(points) = self.seek_table.points.as_mut() {
                points[i].sample_number = xx;
            }

            if !self.input.read_raw_uint64(
                &mut xx,
                FLAC__STREAM_METADATA_SEEKPOINT_STREAM_OFFSET_LEN,
                cb!(self, client),
            ) {
                return false; /* read_callback_ sets the state for us */
            }
            if let Some(points) = self.seek_table.points.as_mut() {
                points[i].stream_offset = xx;
            }

            if !self.input.read_raw_uint32(
                &mut x,
                FLAC__STREAM_METADATA_SEEKPOINT_FRAME_SAMPLES_LEN,
                cb!(self, client),
            ) {
                return false; /* read_callback_ sets the state for us */
            }
            if let Some(points) = self.seek_table.points.as_mut() {
                points[i].frame_samples = x;
            }
        }
        length -= self.seek_table.num_points * FLAC__STREAM_METADATA_SEEKPOINT_LENGTH;
        /* if there is a partial point left, skip over it */
        if length > 0 {
            /*@@@ do a send_error_to_client_() here?  there's an argument for either way */
            if !self
                .input
                .skip_byte_block_aligned_no_crc(length, cb!(self, client))
            {
                return false; /* read_callback_ sets the state for us */
            }
        }

        true
    }

    fn read_metadata_vorbiscomment_(
        &mut self,
        client: &mut dyn StreamDecoderClient,
        obj: &mut VorbisComment,
        mut length: u32,
    ) -> bool {
        'skip: {
            /* read vendor string */
            if length >= 8 {
                length -= 8; /* vendor string length + num comments entries alone take 8 bytes */
                if !self
                    .input
                    .read_uint32_little_endian(&mut obj.vendor_string.length, cb!(self, client))
                {
                    return false; /* read_callback_ sets the state for us */
                }
                if obj.vendor_string.length > 0 {
                    if length < obj.vendor_string.length {
                        obj.vendor_string.length = 0;
                        obj.vendor_string.entry = None;
                        break 'skip;
                    } else {
                        length -= obj.vendor_string.length;
                    }
                    let n = obj.vendor_string.length as usize;
                    let Some(mut entry) = try_vec::<u8>(n + 1) else {
                        self.state = StreamDecoderState::MemoryAllocationError;
                        return false;
                    };
                    let ok = self
                        .input
                        .read_byte_block_aligned_no_crc(&mut entry[..n], cb!(self, client));
                    obj.vendor_string.entry = Some(entry);
                    if !ok {
                        return false; /* read_callback_ sets the state for us */
                    }
                    // (the NUL after it is already there)
                } else {
                    obj.vendor_string.entry = None;
                }

                /* read num comments */
                if !self
                    .input
                    .read_uint32_little_endian(&mut obj.num_comments, cb!(self, client))
                {
                    return false; /* read_callback_ sets the state for us */
                }

                /* read comments */
                if obj.num_comments > 100000 {
                    /* Possibly malicious file. */
                    obj.num_comments = 0;
                    return false;
                }
                if obj.num_comments > 0 {
                    let Some(comments) = try_vec::<VorbisCommentEntry>(obj.num_comments as usize)
                    else {
                        obj.num_comments = 0;
                        self.state = StreamDecoderState::MemoryAllocationError;
                        return false;
                    };
                    let comments = obj.comments.insert(comments);
                    for i in 0..obj.num_comments as usize {
                        /* Initialize here just to make sure. */
                        comments[i].length = 0;
                        comments[i].entry = None;

                        if length < 4 {
                            obj.num_comments = i as u32;
                            break 'skip;
                        } else {
                            length -= 4;
                        }
                        if !self
                            .input
                            .read_uint32_little_endian(&mut comments[i].length, cb!(self, client))
                        {
                            obj.num_comments = i as u32;
                            return false; /* read_callback_ sets the state for us */
                        }
                        if comments[i].length > 0 {
                            if length < comments[i].length {
                                obj.num_comments = i as u32;
                                break 'skip;
                            } else {
                                length -= comments[i].length;
                            }
                            let n = comments[i].length as usize;
                            let Some(mut entry) = try_vec::<u8>(n + 1) else {
                                self.state = StreamDecoderState::MemoryAllocationError;
                                obj.num_comments = i as u32;
                                return false;
                            };
                            if !self
                                .input
                                .read_byte_block_aligned_no_crc(&mut entry[..n], cb!(self, client))
                            {
                                /* Current i-th entry is bad, so we delete it. */
                                comments[i].entry = None;
                                obj.num_comments = i as u32;
                                break 'skip;
                            }
                            comments[i].entry = Some(entry); // (with its NUL)
                        } else {
                            comments[i].entry = None;
                        }
                    }
                }
            }
        }

        // skip:
        if length > 0 {
            /* length > 0 can only happen on files with invalid data in comments */
            if obj.num_comments < 1 {
                obj.comments = None;
            }
            if !self
                .input
                .skip_byte_block_aligned_no_crc(length, cb!(self, client))
            {
                return false; /* read_callback_ sets the state for us */
            }
        }

        true
    }

    fn skip_id3v2_tag_(&mut self, client: &mut dyn StreamDecoderClient) -> bool {
        let mut x: u32 = 0;

        /* skip the version and flags bytes */
        if !self.input.read_raw_uint32(&mut x, 24, cb!(self, client)) {
            return false; /* read_callback_ sets the state for us */
        }
        /* get the size (in bytes) to skip */
        let mut skip: u32 = 0;
        for _ in 0..4 {
            if !self.input.read_raw_uint32(&mut x, 8, cb!(self, client)) {
                return false; /* read_callback_ sets the state for us */
            }
            skip <<= 7;
            skip |= x & 0x7f;
        }
        /* skip the rest of the tag */
        if !self
            .input
            .skip_byte_block_aligned_no_crc(skip, cb!(self, client))
        {
            return false; /* read_callback_ sets the state for us */
        }
        true
    }

    fn frame_sync_(&mut self, client: &mut dyn StreamDecoderClient) -> bool {
        let mut x: u32 = 0;
        let mut first = true;

        /* If we know the total number of samples in the stream, stop if we've read that many. */
        /* This will stop us, for example, from wasting time trying to sync on an ID3V1 tag. */
        if self.get_total_samples() > 0 {
            if self.samples_decoded >= self.get_total_samples() {
                self.state = StreamDecoderState::EndOfStream;
                return true;
            }
        }

        /* make sure we're byte aligned */
        if !self.input.is_consumed_byte_aligned() {
            let bits = self.input.bits_left_for_byte_alignment();
            if !self.input.read_raw_uint32(&mut x, bits, cb!(self, client)) {
                return false; /* read_callback_ sets the state for us */
            }
        }

        loop {
            if self.cached {
                x = self.lookahead as u32;
                self.cached = false;
            } else {
                if !self.input.read_raw_uint32(&mut x, 8, cb!(self, client)) {
                    return false; /* read_callback_ sets the state for us */
                }
            }
            if x == 0xff {
                /* MAGIC NUMBER for the first 8 frame sync bits */
                self.header_warmup[0] = x as u8;
                if !self.input.read_raw_uint32(&mut x, 8, cb!(self, client)) {
                    return false; /* read_callback_ sets the state for us */
                }

                /* we have to check if we just read two 0xff's in a row; the second may actually be the beginning of the sync code */
                /* else we have to check if the second byte is the end of a sync code */
                if x == 0xff {
                    /* MAGIC NUMBER for the first 8 frame sync bits */
                    self.lookahead = x as u8;
                    self.cached = true;
                } else if x >> 1 == 0x7c {
                    /* MAGIC NUMBER for the last 6 sync bits and reserved 7th bit */
                    self.header_warmup[1] = x as u8;
                    self.state = StreamDecoderState::ReadFrame;
                    return true;
                }
            }
            if first {
                self.send_error_to_client_(client, StreamDecoderErrorStatus::LostSync);
                first = false;
            }
        }
    }

    fn read_frame_(
        &mut self,
        client: &mut dyn StreamDecoderClient,
        got_a_frame: &mut bool,
        do_full_decode: bool,
    ) -> bool {
        let mut x: u32 = 0;

        *got_a_frame = false;

        /* init the CRC */
        let mut frame_crc: u32 = 0; /* the one we calculate from the input stream */
        frame_crc = FLAC__CRC16_UPDATE(self.header_warmup[0] as u32, frame_crc);
        frame_crc = FLAC__CRC16_UPDATE(self.header_warmup[1] as u32, frame_crc);
        self.input.reset_read_crc16(frame_crc as u16);

        if !self.read_frame_header_(client) {
            return false;
        }
        if self.state == StreamDecoderState::SearchForFrameSync {
            /* means we didn't sync on a valid header */
            return true;
        }
        if !self.allocate_output_(self.frame.header.blocksize, self.frame.header.channels) {
            return false;
        }
        for channel in 0..self.frame.header.channels {
            /*
             * first figure the correct bits-per-sample of the subframe
             */
            let mut bps = self.frame.header.bits_per_sample;
            match self.frame.header.channel_assignment {
                ChannelAssignment::Independent => { /* no adjustment needed */ }
                ChannelAssignment::LeftSide => {
                    if channel == 1 {
                        bps += 1;
                    }
                }
                ChannelAssignment::RightSide => {
                    if channel == 0 {
                        bps += 1;
                    }
                }
                ChannelAssignment::MidSide => {
                    if channel == 1 {
                        bps += 1;
                    }
                }
            }
            /*
             * now read it
             */
            if !self.read_subframe_(client, channel as usize, bps, do_full_decode) {
                return false;
            }
            if self.state == StreamDecoderState::SearchForFrameSync {
                /* means bad sync or got corruption */
                return true;
            }
        }
        if !self.read_zero_padding_(client) {
            return false;
        }
        if self.state == StreamDecoderState::SearchForFrameSync {
            /* means bad sync or got corruption (i.e. "zero bits" were not all zeroes) */
            return true;
        }

        /*
         * Read the frame CRC-16 from the footer and check
         */
        frame_crc = self.input.get_read_crc16() as u32;
        if !self
            .input
            .read_raw_uint32(&mut x, FLAC__FRAME_FOOTER_CRC_LEN, cb!(self, client))
        {
            return false; /* read_callback_ sets the state for us */
        }
        if frame_crc == x {
            if do_full_decode {
                /* Undo any special channel coding */
                let blocksize = self.frame.header.blocksize as usize;
                let (ch0, rest) = self.output.split_at_mut(1);
                let (out0, out1) = (&mut ch0[0], &mut rest[0]);
                match self.frame.header.channel_assignment {
                    ChannelAssignment::Independent => { /* do nothing */ }
                    ChannelAssignment::LeftSide => {
                        for i in 0..blocksize {
                            out1[i] = out0[i].wrapping_sub(out1[i]);
                        }
                    }
                    ChannelAssignment::RightSide => {
                        for i in 0..blocksize {
                            out0[i] = out0[i].wrapping_add(out1[i]);
                        }
                    }
                    ChannelAssignment::MidSide => {
                        for i in 0..blocksize {
                            let mut mid = out0[i];
                            let side = out1[i];
                            mid = ((mid as u32) << 1) as i32;
                            mid |= side & 1; /* i.e. if 'side' is odd... */
                            out0[i] = mid.wrapping_add(side) >> 1;
                            out1[i] = mid.wrapping_sub(side) >> 1;
                        }
                    }
                }
            }
        } else {
            /* Bad frame, emit error and zero the output signal */
            self.send_error_to_client_(client, StreamDecoderErrorStatus::FrameCrcMismatch);
            if do_full_decode {
                for channel in 0..self.frame.header.channels as usize {
                    self.output[channel][..self.frame.header.blocksize as usize].fill(0);
                }
            }
        }

        *got_a_frame = true;

        /* we wait to update fixed_block_size until here, when we're sure we've got a proper frame and hence a correct blocksize */
        if self.next_fixed_block_size != 0 {
            self.fixed_block_size = self.next_fixed_block_size;
        }

        /* put the latest values into the public section of the decoder instance */
        self.channels = self.frame.header.channels;
        self.channel_assignment = self.frame.header.channel_assignment;
        self.bits_per_sample = self.frame.header.bits_per_sample;
        self.sample_rate = self.frame.header.sample_rate;
        self.blocksize = self.frame.header.blocksize;

        self.samples_decoded = self
            .frame
            .header
            .sample_number()
            .wrapping_add(self.frame.header.blocksize as u64);

        /* write it */
        if do_full_decode {
            if self.write_audio_frame_to_client_(client) != StreamDecoderWriteStatus::Continue {
                self.state = StreamDecoderState::Aborted;
                return false;
            }
        }

        self.state = StreamDecoderState::SearchForFrameSync;
        true
    }

    fn read_frame_header_(&mut self, client: &mut dyn StreamDecoderClient) -> bool {
        let mut x: u32 = 0;
        let mut xx: u64 = 0;
        let mut blocksize_hint: u32 = 0;
        let mut sample_rate_hint: u32 = 0;
        let mut raw_header = [0u8; 16]; /* MAGIC NUMBER based on the maximum frame header size, including CRC */
        let mut raw_header_len: u32;
        let mut is_unparseable = false;

        /* init the raw header with the saved bits from synchronization */
        raw_header[0] = self.header_warmup[0];
        raw_header[1] = self.header_warmup[1];
        raw_header_len = 2;

        /* check to make sure that reserved bit is 0 */
        if raw_header[1] & 0x02 != 0 {
            /* MAGIC NUMBER */
            is_unparseable = true;
        }

        /*
         * Note that along the way as we read the header, we look for a sync
         * code inside.  If we find one it would indicate that our original
         * sync was bad since there cannot be a sync code in a valid header.
         *
         * Three kinds of things can go wrong when reading the frame header:
         *  1) We may have sync'ed incorrectly and not landed on a frame header.
         *     If we don't find a sync code, it can end up looking like we read
         *     a valid but unparseable header, until getting to the frame header
         *     CRC.  Even then we could get a false positive on the CRC.
         *  2) We may have sync'ed correctly but on an unparseable frame (from a
         *     future encoder).
         *  3) We may be on a damaged frame which appears valid but unparseable.
         *
         * For all these reasons, we try and read a complete frame header as
         * long as it seems valid, even if unparseable, up until the frame
         * header CRC.
         */

        /*
         * read in the raw header as bytes so we can CRC it, and parse it on the way
         */
        for _ in 0..2 {
            if !self.input.read_raw_uint32(&mut x, 8, cb!(self, client)) {
                return false; /* read_callback_ sets the state for us */
            }
            if x == 0xff {
                /* MAGIC NUMBER for the first 8 frame sync bits */
                /* if we get here it means our original sync was erroneous since the sync code cannot appear in the header */
                self.lookahead = x as u8;
                self.cached = true;
                self.send_error_to_client_(client, StreamDecoderErrorStatus::BadHeader);
                self.state = StreamDecoderState::SearchForFrameSync;
                return true;
            }
            raw_header[raw_header_len as usize] = x as u8;
            raw_header_len += 1;
        }

        x = (raw_header[2] >> 4) as u32;
        match x {
            0 => {
                is_unparseable = true;
            }
            1 => {
                self.frame.header.blocksize = 192;
            }
            2..=5 => {
                self.frame.header.blocksize = 576 << (x - 2);
            }
            6 | 7 => {
                blocksize_hint = x;
            }
            _ => {
                // 8..=15
                self.frame.header.blocksize = 256 << (x - 8);
            }
        }

        x = (raw_header[2] & 0x0f) as u32;
        match x {
            0 => {
                if self.has_stream_info {
                    self.frame.header.sample_rate = self.stream_info.sample_rate;
                } else {
                    is_unparseable = true;
                }
            }
            1 => self.frame.header.sample_rate = 88200,
            2 => self.frame.header.sample_rate = 176400,
            3 => self.frame.header.sample_rate = 192000,
            4 => self.frame.header.sample_rate = 8000,
            5 => self.frame.header.sample_rate = 16000,
            6 => self.frame.header.sample_rate = 22050,
            7 => self.frame.header.sample_rate = 24000,
            8 => self.frame.header.sample_rate = 32000,
            9 => self.frame.header.sample_rate = 44100,
            10 => self.frame.header.sample_rate = 48000,
            11 => self.frame.header.sample_rate = 96000,
            12..=14 => {
                sample_rate_hint = x;
            }
            _ => {
                // 15
                self.send_error_to_client_(client, StreamDecoderErrorStatus::BadHeader);
                self.state = StreamDecoderState::SearchForFrameSync;
                return true;
            }
        }

        x = (raw_header[3] >> 4) as u32;
        if x & 8 != 0 {
            self.frame.header.channels = 2;
            match x & 7 {
                0 => self.frame.header.channel_assignment = ChannelAssignment::LeftSide,
                1 => self.frame.header.channel_assignment = ChannelAssignment::RightSide,
                2 => self.frame.header.channel_assignment = ChannelAssignment::MidSide,
                _ => {
                    is_unparseable = true;
                }
            }
        } else {
            self.frame.header.channels = x + 1;
            self.frame.header.channel_assignment = ChannelAssignment::Independent;
        }

        x = ((raw_header[3] & 0x0e) >> 1) as u32;
        match x {
            0 => {
                if self.has_stream_info {
                    self.frame.header.bits_per_sample = self.stream_info.bits_per_sample;
                } else {
                    is_unparseable = true;
                }
            }
            1 => self.frame.header.bits_per_sample = 8,
            2 => self.frame.header.bits_per_sample = 12,
            4 => self.frame.header.bits_per_sample = 16,
            5 => self.frame.header.bits_per_sample = 20,
            6 => self.frame.header.bits_per_sample = 24,
            _ => {
                // 3, 7
                is_unparseable = true;
            }
        }

        if self.frame.header.bits_per_sample == 32
            && self.frame.header.channel_assignment != ChannelAssignment::Independent
        {
            /* Decoder isn't equipped for 33-bit side frame */
            is_unparseable = true;
        }

        /* check to make sure that reserved bit is 0 */
        if raw_header[3] & 0x01 != 0 {
            /* MAGIC NUMBER */
            is_unparseable = true;
        }

        /* read the frame's starting sample number (or frame number as the case may be) */
        if raw_header[1] & 0x01 != 0 ||
            /*@@@ this clause is a concession to the old way of doing variable blocksize; the only known implementation is flake and can probably be removed without inconveniencing anyone */
            (self.has_stream_info && self.stream_info.min_blocksize != self.stream_info.max_blocksize)
        {
            /* variable blocksize */
            if !self.input.read_utf8_uint64(
                &mut xx,
                &mut raw_header,
                &mut raw_header_len,
                cb!(self, client),
            ) {
                return false; /* read_callback_ sets the state for us */
            }
            if xx == 0xffff_ffff_ffff_ffff {
                /* i.e. non-UTF8 code... */
                self.lookahead = raw_header[raw_header_len as usize - 1]; /* back up as much as we can */
                self.cached = true;
                self.send_error_to_client_(client, StreamDecoderErrorStatus::BadHeader);
                self.state = StreamDecoderState::SearchForFrameSync;
                return true;
            }
            self.frame.header.number_type = FrameNumberType::SampleNumber;
            self.frame.header.number = xx;
        } else {
            /* fixed blocksize */
            if !self.input.read_utf8_uint32(
                &mut x,
                &mut raw_header,
                &mut raw_header_len,
                cb!(self, client),
            ) {
                return false; /* read_callback_ sets the state for us */
            }
            if x == 0xffffffff {
                /* i.e. non-UTF8 code... */
                self.lookahead = raw_header[raw_header_len as usize - 1]; /* back up as much as we can */
                self.cached = true;
                self.send_error_to_client_(client, StreamDecoderErrorStatus::BadHeader);
                self.state = StreamDecoderState::SearchForFrameSync;
                return true;
            }
            self.frame.header.number_type = FrameNumberType::FrameNumber;
            self.frame.header.set_frame_number(x);
        }

        if blocksize_hint != 0 {
            if !self.input.read_raw_uint32(&mut x, 8, cb!(self, client)) {
                return false; /* read_callback_ sets the state for us */
            }
            raw_header[raw_header_len as usize] = x as u8;
            raw_header_len += 1;
            if blocksize_hint == 7 {
                let mut _x: u32 = 0;
                if !self.input.read_raw_uint32(&mut _x, 8, cb!(self, client)) {
                    return false; /* read_callback_ sets the state for us */
                }
                raw_header[raw_header_len as usize] = _x as u8;
                raw_header_len += 1;
                x = (x << 8) | _x;
            }
            self.frame.header.blocksize = x + 1;
        }

        if sample_rate_hint != 0 {
            if !self.input.read_raw_uint32(&mut x, 8, cb!(self, client)) {
                return false; /* read_callback_ sets the state for us */
            }
            raw_header[raw_header_len as usize] = x as u8;
            raw_header_len += 1;
            if sample_rate_hint != 12 {
                let mut _x: u32 = 0;
                if !self.input.read_raw_uint32(&mut _x, 8, cb!(self, client)) {
                    return false; /* read_callback_ sets the state for us */
                }
                raw_header[raw_header_len as usize] = _x as u8;
                raw_header_len += 1;
                x = (x << 8) | _x;
            }
            if sample_rate_hint == 12 {
                self.frame.header.sample_rate = x * 1000;
            } else if sample_rate_hint == 13 {
                self.frame.header.sample_rate = x;
            } else {
                self.frame.header.sample_rate = x * 10;
            }
        }

        /* read the CRC-8 byte */
        if !self.input.read_raw_uint32(&mut x, 8, cb!(self, client)) {
            return false; /* read_callback_ sets the state for us */
        }
        let crc8 = x as u8;

        if FLAC__crc8(&raw_header[..raw_header_len as usize]) != crc8 {
            self.send_error_to_client_(client, StreamDecoderErrorStatus::BadHeader);
            self.state = StreamDecoderState::SearchForFrameSync;
            return true;
        }

        /* calculate the sample number from the frame number if needed */
        self.next_fixed_block_size = 0;
        if self.frame.header.number_type == FrameNumberType::FrameNumber {
            x = self.frame.header.frame_number();
            self.frame.header.number_type = FrameNumberType::SampleNumber;
            if self.fixed_block_size != 0 {
                self.frame.header.number = self.fixed_block_size as u64 * x as u64;
            } else if self.has_stream_info {
                if self.stream_info.min_blocksize == self.stream_info.max_blocksize {
                    self.frame.header.number = self.stream_info.min_blocksize as u64 * x as u64;
                    self.next_fixed_block_size = self.stream_info.max_blocksize;
                } else {
                    is_unparseable = true;
                }
            } else if x == 0 {
                self.frame.header.number = 0;
                self.next_fixed_block_size = self.frame.header.blocksize;
            } else {
                /* can only get here if the stream has invalid frame numbering and no STREAMINFO, so assume it's not the last (possibly short) frame */
                self.frame.header.number = self.frame.header.blocksize as u64 * x as u64;
            }
        }

        if is_unparseable {
            self.send_error_to_client_(client, StreamDecoderErrorStatus::UnparseableStream);
            self.state = StreamDecoderState::SearchForFrameSync;
            return true;
        }

        true
    }

    fn read_subframe_(
        &mut self,
        client: &mut dyn StreamDecoderClient,
        channel: usize,
        mut bps: u32,
        do_full_decode: bool,
    ) -> bool {
        let mut x: u32 = 0;

        if !self.input.read_raw_uint32(&mut x, 8, cb!(self, client)) {
            /* MAGIC NUMBER */
            return false; /* read_callback_ sets the state for us */
        }

        let wasted_bits = (x & 1) != 0;
        x &= 0xfe;

        if wasted_bits {
            let mut u: u32 = 0;
            if !self.input.read_unary_unsigned(&mut u, cb!(self, client)) {
                return false; /* read_callback_ sets the state for us */
            }
            self.frame.subframes[channel].wasted_bits = u.wrapping_add(1);
            if self.frame.subframes[channel].wasted_bits >= bps {
                return false;
            }
            bps -= self.frame.subframes[channel].wasted_bits;
        } else {
            self.frame.subframes[channel].wasted_bits = 0;
        }

        /*
         * Lots of magic numbers here
         */
        if x & 0x80 != 0 {
            self.send_error_to_client_(client, StreamDecoderErrorStatus::LostSync);
            self.state = StreamDecoderState::SearchForFrameSync;
            return true;
        } else if x == 0 {
            if !self.read_subframe_constant_(client, channel, bps, do_full_decode) {
                return false;
            }
        } else if x == 2 {
            if !self.read_subframe_verbatim_(client, channel, bps, do_full_decode) {
                return false;
            }
        } else if x < 16 {
            self.send_error_to_client_(client, StreamDecoderErrorStatus::UnparseableStream);
            self.state = StreamDecoderState::SearchForFrameSync;
            return true;
        } else if x <= 24 {
            let predictor_order = (x >> 1) & 7;
            if self.frame.header.bits_per_sample > 24 {
                /* Decoder isn't equipped for fixed subframes with more than 24 bps */
                self.send_error_to_client_(client, StreamDecoderErrorStatus::UnparseableStream);
                self.state = StreamDecoderState::SearchForFrameSync;
                return true;
            }
            if self.frame.header.blocksize <= predictor_order {
                self.send_error_to_client_(client, StreamDecoderErrorStatus::LostSync);
                self.state = StreamDecoderState::SearchForFrameSync;
                return true;
            }
            if !self.read_subframe_fixed_(client, channel, bps, predictor_order, do_full_decode) {
                return false;
            }
            if self.state == StreamDecoderState::SearchForFrameSync {
                /* means bad sync or got corruption */
                return true;
            }
        } else if x < 64 {
            self.send_error_to_client_(client, StreamDecoderErrorStatus::UnparseableStream);
            self.state = StreamDecoderState::SearchForFrameSync;
            return true;
        } else {
            let predictor_order = ((x >> 1) & 31) + 1;
            if self.frame.header.blocksize <= predictor_order {
                self.send_error_to_client_(client, StreamDecoderErrorStatus::LostSync);
                self.state = StreamDecoderState::SearchForFrameSync;
                return true;
            }
            if !self.read_subframe_lpc_(client, channel, bps, predictor_order, do_full_decode) {
                return false;
            }
            if self.state == StreamDecoderState::SearchForFrameSync {
                /* means bad sync or got corruption */
                return true;
            }
        }

        if wasted_bits && do_full_decode {
            x = self.frame.subframes[channel].wasted_bits;
            let output = &mut self.output[channel];
            for i in 0..self.frame.header.blocksize as usize {
                let val = output[i] as u32;
                output[i] = (val << x) as i32;
            }
        }

        true
    }

    fn read_subframe_constant_(
        &mut self,
        client: &mut dyn StreamDecoderClient,
        channel: usize,
        bps: u32,
        do_full_decode: bool,
    ) -> bool {
        let mut x: i32 = 0;

        self.frame.subframes[channel].type_ = SubframeType::Constant;

        if !self.input.read_raw_int32(&mut x, bps, cb!(self, client)) {
            return false; /* read_callback_ sets the state for us */
        }

        self.frame.subframes[channel].constant_value = x;

        /* decode the subframe */
        if do_full_decode {
            self.output[channel][..self.frame.header.blocksize as usize].fill(x);
        }

        true
    }

    fn read_subframe_fixed_(
        &mut self,
        client: &mut dyn StreamDecoderClient,
        channel: usize,
        bps: u32,
        order: u32,
        do_full_decode: bool,
    ) -> bool {
        let mut i32_: i32 = 0;
        let mut u32_: u32 = 0;

        self.frame.subframes[channel].type_ = SubframeType::Fixed;

        self.frame.subframes[channel].fixed.order = order;

        /* read warm-up samples */
        for u in 0..order as usize {
            if !self.input.read_raw_int32(&mut i32_, bps, cb!(self, client)) {
                return false; /* read_callback_ sets the state for us */
            }
            self.frame.subframes[channel].fixed.warmup[u] = i32_;
        }

        /* read entropy coding method info */
        if !self.input.read_raw_uint32(
            &mut u32_,
            FLAC__ENTROPY_CODING_METHOD_TYPE_LEN,
            cb!(self, client),
        ) {
            return false; /* read_callback_ sets the state for us */
        }
        self.frame.subframes[channel]
            .fixed
            .entropy_coding_method
            .type_ = u32_;
        match u32_ {
            FLAC__ENTROPY_CODING_METHOD_PARTITIONED_RICE
            | FLAC__ENTROPY_CODING_METHOD_PARTITIONED_RICE2 => {
                if !self.input.read_raw_uint32(
                    &mut u32_,
                    FLAC__ENTROPY_CODING_METHOD_PARTITIONED_RICE_ORDER_LEN,
                    cb!(self, client),
                ) {
                    return false; /* read_callback_ sets the state for us */
                }
                if (self.frame.header.blocksize >> u32_ < order)
                    || (self.frame.header.blocksize % (1 << u32_) > 0)
                {
                    self.send_error_to_client_(client, StreamDecoderErrorStatus::LostSync);
                    self.state = StreamDecoderState::SearchForFrameSync;
                    return true;
                }
                self.frame.subframes[channel]
                    .fixed
                    .entropy_coding_method
                    .order = u32_;
            }
            _ => {
                self.send_error_to_client_(client, StreamDecoderErrorStatus::UnparseableStream);
                self.state = StreamDecoderState::SearchForFrameSync;
                return true;
            }
        }

        /* read residual */
        let method = self.frame.subframes[channel].fixed.entropy_coding_method;
        if !self.read_residual_partitioned_rice_(
            client,
            channel,
            order,
            method.order,
            /*is_extended=*/ method.type_ == FLAC__ENTROPY_CODING_METHOD_PARTITIONED_RICE2,
        ) {
            return false;
        }

        /* decode the subframe */
        if do_full_decode {
            let order_ = order as usize;
            self.output[channel][..order_]
                .copy_from_slice(&self.frame.subframes[channel].fixed.warmup[..order_]);
            FLAC__fixed_restore_signal(
                &self.residual[channel],
                self.frame.header.blocksize - order,
                order,
                &mut self.output[channel],
                order_,
            );
        }

        true
    }

    fn read_subframe_lpc_(
        &mut self,
        client: &mut dyn StreamDecoderClient,
        channel: usize,
        bps: u32,
        order: u32,
        do_full_decode: bool,
    ) -> bool {
        let mut i32_: i32 = 0;
        let mut u32_: u32 = 0;

        self.frame.subframes[channel].type_ = SubframeType::Lpc;

        self.frame.subframes[channel].lpc.order = order;

        /* read warm-up samples */
        for u in 0..order as usize {
            if !self.input.read_raw_int32(&mut i32_, bps, cb!(self, client)) {
                return false; /* read_callback_ sets the state for us */
            }
            self.frame.subframes[channel].lpc.warmup[u] = i32_;
        }

        /* read qlp coeff precision */
        if !self.input.read_raw_uint32(
            &mut u32_,
            FLAC__SUBFRAME_LPC_QLP_COEFF_PRECISION_LEN,
            cb!(self, client),
        ) {
            return false; /* read_callback_ sets the state for us */
        }
        if u32_ == (1u32 << FLAC__SUBFRAME_LPC_QLP_COEFF_PRECISION_LEN) - 1 {
            self.send_error_to_client_(client, StreamDecoderErrorStatus::LostSync);
            self.state = StreamDecoderState::SearchForFrameSync;
            return true;
        }
        self.frame.subframes[channel].lpc.qlp_coeff_precision = u32_ + 1;

        /* read qlp shift */
        if !self.input.read_raw_int32(
            &mut i32_,
            FLAC__SUBFRAME_LPC_QLP_SHIFT_LEN,
            cb!(self, client),
        ) {
            return false; /* read_callback_ sets the state for us */
        }
        if i32_ < 0 {
            self.send_error_to_client_(client, StreamDecoderErrorStatus::LostSync);
            self.state = StreamDecoderState::SearchForFrameSync;
            return true;
        }
        self.frame.subframes[channel].lpc.quantization_level = i32_;

        /* read quantized lp coefficiencts */
        for u in 0..order as usize {
            let precision = self.frame.subframes[channel].lpc.qlp_coeff_precision;
            if !self
                .input
                .read_raw_int32(&mut i32_, precision, cb!(self, client))
            {
                return false; /* read_callback_ sets the state for us */
            }
            self.frame.subframes[channel].lpc.qlp_coeff[u] = i32_;
        }

        /* read entropy coding method info */
        if !self.input.read_raw_uint32(
            &mut u32_,
            FLAC__ENTROPY_CODING_METHOD_TYPE_LEN,
            cb!(self, client),
        ) {
            return false; /* read_callback_ sets the state for us */
        }
        self.frame.subframes[channel]
            .lpc
            .entropy_coding_method
            .type_ = u32_;
        match u32_ {
            FLAC__ENTROPY_CODING_METHOD_PARTITIONED_RICE
            | FLAC__ENTROPY_CODING_METHOD_PARTITIONED_RICE2 => {
                if !self.input.read_raw_uint32(
                    &mut u32_,
                    FLAC__ENTROPY_CODING_METHOD_PARTITIONED_RICE_ORDER_LEN,
                    cb!(self, client),
                ) {
                    return false; /* read_callback_ sets the state for us */
                }
                if (self.frame.header.blocksize >> u32_ < order)
                    || (self.frame.header.blocksize % (1 << u32_) > 0)
                {
                    self.send_error_to_client_(client, StreamDecoderErrorStatus::LostSync);
                    self.state = StreamDecoderState::SearchForFrameSync;
                    return true;
                }
                self.frame.subframes[channel]
                    .lpc
                    .entropy_coding_method
                    .order = u32_;
            }
            _ => {
                self.send_error_to_client_(client, StreamDecoderErrorStatus::UnparseableStream);
                self.state = StreamDecoderState::SearchForFrameSync;
                return true;
            }
        }

        /* read residual */
        let method = self.frame.subframes[channel].lpc.entropy_coding_method;
        if !self.read_residual_partitioned_rice_(
            client,
            channel,
            order,
            method.order,
            /*is_extended=*/ method.type_ == FLAC__ENTROPY_CODING_METHOD_PARTITIONED_RICE2,
        ) {
            return false;
        }

        /* decode the subframe */
        if do_full_decode {
            let lpc = self.frame.subframes[channel].lpc;
            let order_ = order as usize;
            self.output[channel][..order_].copy_from_slice(&lpc.warmup[..order_]);
            let data_len = self.frame.header.blocksize - order;
            if bps + lpc.qlp_coeff_precision + FLAC__bitmath_ilog2(order) <= 32 {
                if bps <= 16 && lpc.qlp_coeff_precision <= 16 {
                    // (local_lpc_restore_signal_16bit, which is
                    // FLAC__lpc_restore_signal() on x86-64)
                    FLAC__lpc_restore_signal(
                        &self.residual[channel],
                        data_len,
                        &lpc.qlp_coeff,
                        order,
                        lpc.quantization_level,
                        &mut self.output[channel],
                        order_,
                    );
                } else {
                    FLAC__lpc_restore_signal(
                        &self.residual[channel],
                        data_len,
                        &lpc.qlp_coeff,
                        order,
                        lpc.quantization_level,
                        &mut self.output[channel],
                        order_,
                    );
                }
            } else {
                FLAC__lpc_restore_signal_wide(
                    &self.residual[channel],
                    data_len,
                    &lpc.qlp_coeff,
                    order,
                    lpc.quantization_level,
                    &mut self.output[channel],
                    order_,
                );
            }
        }

        true
    }

    fn read_subframe_verbatim_(
        &mut self,
        client: &mut dyn StreamDecoderClient,
        channel: usize,
        bps: u32,
        do_full_decode: bool,
    ) -> bool {
        let mut x: i32 = 0;

        self.frame.subframes[channel].type_ = SubframeType::Verbatim;

        for i in 0..self.frame.header.blocksize as usize {
            if !self.input.read_raw_int32(&mut x, bps, cb!(self, client)) {
                return false; /* read_callback_ sets the state for us */
            }
            self.residual[channel][i] = x;
        }

        /* decode the subframe */
        if do_full_decode {
            let n = self.frame.header.blocksize as usize;
            let (output, residual) = (&mut self.output[channel], &self.residual[channel]);
            output[..n].copy_from_slice(&residual[..n]);
        }

        true
    }

    fn read_residual_partitioned_rice_(
        &mut self,
        client: &mut dyn StreamDecoderClient,
        channel: usize,
        predictor_order: u32,
        partition_order: u32,
        is_extended: bool,
    ) -> bool {
        let mut rice_parameter: u32 = 0;
        let mut i: i32 = 0;
        let partitions: u32 = 1u32 << partition_order;
        let partition_samples: u32 = self.frame.header.blocksize >> partition_order;
        let plen = if is_extended {
            FLAC__ENTROPY_CODING_METHOD_PARTITIONED_RICE2_PARAMETER_LEN
        } else {
            FLAC__ENTROPY_CODING_METHOD_PARTITIONED_RICE_PARAMETER_LEN
        };
        let pesc = if is_extended {
            FLAC__ENTROPY_CODING_METHOD_PARTITIONED_RICE2_ESCAPE_PARAMETER
        } else {
            FLAC__ENTROPY_CODING_METHOD_PARTITIONED_RICE_ESCAPE_PARAMETER
        };

        /* invalid predictor and partition orders mush be handled in the callers */

        if !FLAC__format_entropy_coding_method_partitioned_rice_contents_ensure_size(
            &mut self.partitioned_rice_contents[channel],
            6u32.max(partition_order),
        ) {
            self.state = StreamDecoderState::MemoryAllocationError;
            return false;
        }

        let mut sample: u32 = 0;
        for partition in 0..partitions as usize {
            if !self
                .input
                .read_raw_uint32(&mut rice_parameter, plen, cb!(self, client))
            {
                return false; /* read_callback_ sets the state for us */
            }
            self.partitioned_rice_contents[channel].parameters[partition] = rice_parameter;
            if rice_parameter < pesc {
                self.partitioned_rice_contents[channel].raw_bits[partition] = 0;
                let u = if partition == 0 {
                    partition_samples - predictor_order
                } else {
                    partition_samples
                };
                if !self.input.read_rice_signed_block(
                    &mut self.residual[channel][sample as usize..],
                    u as usize,
                    rice_parameter,
                    cb!(self, client),
                ) {
                    return false; /* read_callback_ sets the state for us */
                }
                sample += u;
            } else {
                if !self.input.read_raw_uint32(
                    &mut rice_parameter,
                    FLAC__ENTROPY_CODING_METHOD_PARTITIONED_RICE_RAW_LEN,
                    cb!(self, client),
                ) {
                    return false; /* read_callback_ sets the state for us */
                }
                self.partitioned_rice_contents[channel].raw_bits[partition] = rice_parameter;
                let mut u = if partition == 0 { predictor_order } else { 0 };
                while u < partition_samples {
                    if !self
                        .input
                        .read_raw_int32(&mut i, rice_parameter, cb!(self, client))
                    {
                        return false; /* read_callback_ sets the state for us */
                    }
                    self.residual[channel][sample as usize] = i;
                    u += 1;
                    sample += 1;
                }
            }
        }

        true
    }

    fn read_zero_padding_(&mut self, client: &mut dyn StreamDecoderClient) -> bool {
        if !self.input.is_consumed_byte_aligned() {
            let mut zero: u32 = 0;
            let bits = self.input.bits_left_for_byte_alignment();
            if !self
                .input
                .read_raw_uint32(&mut zero, bits, cb!(self, client))
            {
                return false; /* read_callback_ sets the state for us */
            }
            if zero != 0 {
                self.send_error_to_client_(client, StreamDecoderErrorStatus::LostSync);
                self.state = StreamDecoderState::SearchForFrameSync;
            }
        }
        true
    }

    fn write_audio_frame_to_client_(
        &mut self,
        client: &mut dyn StreamDecoderClient,
    ) -> StreamDecoderWriteStatus {
        let frame = self.frame.header;
        if self.is_seeking {
            let this_frame_sample = frame.sample_number();
            let next_frame_sample = this_frame_sample.wrapping_add(frame.blocksize as u64);
            let target_sample = self.target_sample;

            self.last_frame = frame; /* save the frame */
            if this_frame_sample <= target_sample && target_sample < next_frame_sample {
                /* we hit our target frame */
                let delta = (target_sample - this_frame_sample) as u32;
                /* kick out of seek mode */
                self.is_seeking = false;
                /* shift out the samples before target_sample */
                if delta > 0 {
                    self.last_frame.blocksize -= delta;
                    self.last_frame.number = self.last_frame.number.wrapping_add(delta as u64);
                    /* write the relevant samples */
                    let last_frame = self.last_frame;
                    client.write(self, &last_frame, delta as usize)
                } else {
                    /* write the relevant samples */
                    client.write(self, &frame, 0)
                }
            } else {
                StreamDecoderWriteStatus::Continue
            }
        } else {
            /*
             * If we never got STREAMINFO, turn off MD5 checking to save
             * cycles since we don't have a sum to compare to anyway
             */
            if !self.has_stream_info {
                self.do_md5_checking = false;
            }
            // (do_md5_checking is never on here)
            client.write(self, &frame, 0)
        }
    }

    fn send_error_to_client_(
        &mut self,
        client: &mut dyn StreamDecoderClient,
        status: StreamDecoderErrorStatus,
    ) {
        if !self.is_seeking {
            client.error(status);
        } else if status == StreamDecoderErrorStatus::UnparseableStream {
            self.unparseable_frame_count += 1;
        }
    }

    fn seek_to_absolute_sample_(
        &mut self,
        client: &mut dyn StreamDecoderClient,
        stream_length: u64,
        target_sample: u64,
    ) -> bool {
        let first_frame_offset = self.first_frame_offset;
        let mut lower_bound: u64;
        let mut upper_bound: u64;
        let mut lower_bound_sample: u64;
        let mut upper_bound_sample: u64;
        let mut this_frame_sample: u64;
        let mut pos: i64;
        let mut approx_bytes_per_frame: u32;
        let mut first_seek = true;
        let total_samples = self.get_total_samples();
        let min_blocksize = self.stream_info.min_blocksize;
        let max_blocksize = self.stream_info.max_blocksize;
        let max_framesize = self.stream_info.max_framesize;
        let min_framesize = self.stream_info.min_framesize;
        /* take these from the current frame in case they've changed mid-stream */
        let mut channels = self.get_channels();
        let mut bps = self.get_bits_per_sample();
        let has_seek_table = self.has_seek_table;

        /* use values from stream info if we didn't decode a frame */
        if channels == 0 {
            channels = self.stream_info.channels;
        }
        if bps == 0 {
            bps = self.stream_info.bits_per_sample;
        }

        /* we are just guessing here */
        if max_framesize > 0 {
            approx_bytes_per_frame = (max_framesize + min_framesize) / 2 + 1;
        }
        /*
         * Check if it's a known fixed-blocksize stream.  Note that though
         * the spec doesn't allow zeroes in the STREAMINFO block, we may
         * never get a STREAMINFO block when decoding so the value of
         * min_blocksize might be zero.
         */
        else if min_blocksize == max_blocksize && min_blocksize > 0 {
            /* note there are no () around 'bps/8' to keep precision up since it's an integer calculation */
            approx_bytes_per_frame =
                min_blocksize.wrapping_mul(channels).wrapping_mul(bps) / 8 + 64;
        } else {
            approx_bytes_per_frame = 4096u32.wrapping_mul(channels).wrapping_mul(bps) / 8 + 64;
        }

        /*
         * First, we set an upper and lower bound on where in the
         * stream we will search.  For now we take the current position
         * as one bound and, depending on where the target position lies,
         * the beginning of the first frame or the end of the stream as
         * the other bound.
         */
        lower_bound = first_frame_offset;
        lower_bound_sample = 0;
        upper_bound = stream_length;
        upper_bound_sample = if total_samples > 0 {
            total_samples
        } else {
            target_sample /*estimate it*/
        };

        if self.state == StreamDecoderState::SearchForFrameSync && self.samples_decoded != 0 {
            if target_sample < self.samples_decoded {
                if self.get_decode_position(client, &mut upper_bound) {
                    upper_bound_sample = self.samples_decoded;
                }
            } else {
                if self.get_decode_position(client, &mut lower_bound) {
                    lower_bound_sample = self.samples_decoded;
                }
            }
        }

        /*
         * Now we refine the bounds if we have a seektable with
         * suitable points.  Note that according to the spec they
         * must be ordered by ascending sample number.
         *
         * Note: to protect against invalid seek tables we will ignore points
         * that have frame_samples==0 or sample_number>=total_samples
         */
        if has_seek_table {
            let mut new_lower_bound = lower_bound;
            let mut new_upper_bound = upper_bound;
            let mut new_lower_bound_sample = lower_bound_sample;
            let mut new_upper_bound_sample = upper_bound_sample;
            let num_points = self.seek_table.num_points as i32;
            let points: &[SeekPoint] = self.seek_table.points.as_deref().unwrap_or(&[]);
            let suitable = |p: &SeekPoint| {
                p.sample_number != FLAC__STREAM_METADATA_SEEKPOINT_PLACEHOLDER
                    && p.frame_samples > 0 /* defense against bad seekpoints */
                    && (total_samples == 0 || p.sample_number < total_samples) /* defense against bad seekpoints */
            };

            /* find the closest seek point <= target_sample, if it exists */
            let mut i = num_points - 1;
            while i >= 0 {
                let p = &points[i as usize];
                if suitable(p) && p.sample_number <= target_sample {
                    break;
                }
                i -= 1;
            }
            if i >= 0 {
                /* i.e. we found a suitable seek point... */
                new_lower_bound = first_frame_offset.wrapping_add(points[i as usize].stream_offset);
                new_lower_bound_sample = points[i as usize].sample_number;
            }

            /* find the closest seek point > target_sample, if it exists */
            i = 0;
            while i < num_points {
                let p = &points[i as usize];
                if suitable(p) && p.sample_number > target_sample {
                    break;
                }
                i += 1;
            }
            if i < num_points {
                /* i.e. we found a suitable seek point... */
                new_upper_bound = first_frame_offset.wrapping_add(points[i as usize].stream_offset);
                new_upper_bound_sample = points[i as usize].sample_number;
            }
            /* final protection against unsorted seek tables; keep original values if bogus */
            if new_upper_bound >= new_lower_bound {
                lower_bound = new_lower_bound;
                upper_bound = new_upper_bound;
                lower_bound_sample = new_lower_bound_sample;
                upper_bound_sample = new_upper_bound_sample;
            }
        }

        /* there are 2 insidious ways that the following equality occurs, which
         * we need to fix:
         *  1) total_samples is 0 (unknown) and target_sample is 0
         *  2) total_samples is 0 (unknown) and target_sample happens to be
         *     exactly equal to the last seek point in the seek table; this
         *     means there is no seek point above it, and upper_bound_samples
         *     remains equal to the estimate (of target_samples) we made above
         * in either case it does not hurt to move upper_bound_sample up by 1
         */
        if upper_bound_sample == lower_bound_sample {
            upper_bound_sample = upper_bound_sample.wrapping_add(1);
        }

        self.target_sample = target_sample;
        loop {
            /* check if the bounds are still ok */
            if lower_bound_sample >= upper_bound_sample || lower_bound > upper_bound {
                self.state = StreamDecoderState::SeekError;
                return false;
            }
            pos = (lower_bound as i64)
                .wrapping_add(c_i64(
                    target_sample.wrapping_sub(lower_bound_sample) as f64
                        / upper_bound_sample.wrapping_sub(lower_bound_sample) as f64
                        * upper_bound.wrapping_sub(lower_bound) as f64,
                ))
                .wrapping_sub(approx_bytes_per_frame as i64);
            if pos >= upper_bound as i64 {
                pos = (upper_bound as i64).wrapping_sub(1);
            }
            if pos < lower_bound as i64 {
                pos = lower_bound as i64;
            }
            if client.seek(pos as u64) != StreamDecoderSeekStatus::Ok {
                self.state = StreamDecoderState::SeekError;
                return false;
            }
            if !self.flush() {
                /* above call sets the state for us */
                return false;
            }
            /* Now we need to get a frame.  First we need to reset our
             * unparseable_frame_count; if we get too many unparseable
             * frames in a row, the read callback will return
             * FLAC__STREAM_DECODER_READ_STATUS_ABORT, causing
             * FLAC__stream_decoder_process_single() to return false.
             */
            self.unparseable_frame_count = 0;
            if !self.process_single(client) || self.state == StreamDecoderState::Aborted {
                self.state = StreamDecoderState::SeekError;
                return false;
            }
            /* our write callback will change the state when it gets to the target frame */
            /* actually, we could have got_a_frame if our decoder is at FLAC__STREAM_DECODER_END_OF_STREAM so we need to check for that also */
            if !self.is_seeking {
                break;
            }

            this_frame_sample = self.last_frame.sample_number();

            if 0 == self.samples_decoded
                || (this_frame_sample.wrapping_add(self.last_frame.blocksize as u64)
                    >= upper_bound_sample
                    && !first_seek)
            {
                if pos == lower_bound as i64 {
                    /* can't move back any more than the first frame, something is fatally wrong */
                    self.state = StreamDecoderState::SeekError;
                    return false;
                }
                /* our last move backwards wasn't big enough, try again */
                approx_bytes_per_frame = if approx_bytes_per_frame != 0 {
                    approx_bytes_per_frame.wrapping_mul(2)
                } else {
                    16
                };
                continue;
            }
            /* allow one seek over upper bound, so we can get a correct upper_bound_sample for streams with unknown total_samples */
            first_seek = false;

            /* make sure we are not seeking in corrupted stream */
            if this_frame_sample < lower_bound_sample {
                self.state = StreamDecoderState::SeekError;
                return false;
            }

            /* we need to narrow the search */
            if target_sample < this_frame_sample {
                upper_bound_sample =
                    this_frame_sample.wrapping_add(self.last_frame.blocksize as u64);
                /*@@@@@@ what will decode position be if at end of stream? */
                if !self.get_decode_position(client, &mut upper_bound) {
                    self.state = StreamDecoderState::SeekError;
                    return false;
                }
                approx_bytes_per_frame =
                    (2u64.wrapping_mul(upper_bound.wrapping_sub(pos as u64)) / 3 + 16) as u32;
            } else {
                /* target_sample >= this_frame_sample + this frame's blocksize */
                lower_bound_sample =
                    this_frame_sample.wrapping_add(self.last_frame.blocksize as u64);
                if !self.get_decode_position(client, &mut lower_bound) {
                    self.state = StreamDecoderState::SeekError;
                    return false;
                }
                approx_bytes_per_frame =
                    (2u64.wrapping_mul(lower_bound.wrapping_sub(pos as u64)) / 3 + 16) as u32;
            }
        }

        true
    }
}

impl BitReaderReadCallback for ReadCallback<'_> {
    /// `read_callback_()`.
    fn read(&mut self, buffer: &mut [u8], bytes: &mut usize) -> bool {
        if self.client.eof() {
            *bytes = 0;
            *self.state = StreamDecoderState::EndOfStream;
            false
        } else if *bytes > 0 {
            /* While seeking, it is possible for our seek to land in the
             * middle of audio data that looks exactly like a frame header
             * from a future version of an encoder.  When that happens, our
             * error callback will get an
             * FLAC__STREAM_DECODER_UNPARSEABLE_STREAM and increment its
             * unparseable_frame_count.  But there is a remote possibility
             * that it is properly synced at such a "future-codec frame",
             * so to make sure, we wait to see many "unparseable" errors in
             * a row before bailing out.
             */
            if self.is_seeking && self.unparseable_frame_count > 20 {
                *self.state = StreamDecoderState::Aborted;
                false
            } else {
                let status = self.client.read(buffer, bytes);
                if status == StreamDecoderReadStatus::Abort {
                    *self.state = StreamDecoderState::Aborted;
                    false
                } else if *bytes == 0 {
                    if status == StreamDecoderReadStatus::EndOfStream || self.client.eof() {
                        *self.state = StreamDecoderState::EndOfStream;
                        false
                    } else {
                        true
                    }
                } else {
                    true
                }
            }
        } else {
            /* abort to avoid a deadlock */
            *self.state = StreamDecoderState::Aborted;
            false
        }
    }
}
