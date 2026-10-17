// Rust translation of src/opusfile.c (and the OggOpusLink/OggOpusFile
// structures of src/internal.h) from opusfile 0.12
// (https://opus-codec.org, at the revision SDL_mixer's external/opusfile
// pins).
// THE libopusfile SOURCE CODE IS (C) COPYRIGHT 1994-2020
// by the Xiph.Org Foundation and contributors https://xiph.org/
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! function: stdio-based convenience library for opening/seeking/decoding
//! last mod: $Id: vorbisfile.c 17573 2010-10-27 14:53:59Z xiphmont $
//!
//! The stream is an [`IoStream`] used through SDL_mixer's
//! `OPUS_IoCallbacks` (decoder_opus.c): `read` is `SDL_ReadIO()`, `seek`
//! is `SDL_SeekIO()` (0, or -1 on failure), `tell` is `SDL_TellIO()` and
//! `close` does nothing. `op_free()` is dropping the [`OggOpusFile`].

use sdl3::io::{IoStream, IoWhence};

use super::info::{
    opus_head_parse, opus_tags_clear, opus_tags_get_album_gain, opus_tags_get_track_gain,
    opus_tags_parse,
};
use super::internal::*;
use super::*;
use crate::ogg::framing::*;
use crate::opus::multistream::{opus_multistream_decode_float, OpusMSDecoder};
use crate::opus::opus_decoder::opus_packet_get_nb_frames;
use crate::opus::packet::opus_packet_get_samples_per_frame;

const SEEK_SET: i32 = 0;
const SEEK_CUR: i32 = 1;
const SEEK_END: i32 = 2;

/*Information cached for a single link in a chained Ogg Opus file.
We choose the first Opus stream encountered in each link to play back (and
require at least one).*/
/// Translation of `struct OggOpusLink`.
#[derive(Clone, Debug, Default)]
pub(crate) struct OggOpusLink {
    /*The byte offset of the first header page in this link.*/
    offset: i64,
    /*The byte offset of the first data page from the chosen Opus stream in this
    link (after the headers).*/
    data_offset: i64,
    /*The byte offset of the last page from the chosen Opus stream in this link.
    This is used when seeking to ensure we find a page before the last one, so
    that end-trimming calculations work properly.
    This is only valid for seekable sources.*/
    end_offset: i64,
    /*The total duration of all prior links.
    This is always zero for non-seekable sources.*/
    pcm_file_offset: i64,
    /*The granule position of the last sample.
    This is only valid for seekable sources.*/
    pcm_end: i64,
    /*The granule position before the first sample.*/
    pcm_start: i64,
    /*The serial number.*/
    serialno: u32,
    /*The contents of the info header.*/
    head: OpusHead,
    /*The contents of the comment header.*/
    tags: OpusTags,
}

/// Translation of `struct OggOpusFile`.
pub(crate) struct OggOpusFile<'a> {
    /*The callbacks used to access the stream.*/
    /*A FILE *, memory buffer, etc.*/
    stream: IoStream<'a>,
    /*Whether or not we can seek with this stream.*/
    seekable: i32,
    /*The number of links in this chained Ogg Opus file.*/
    nlinks: i32,
    /*The cached information from each link in a chained Ogg Opus file.
    If stream isn't seekable (e.g., it's a pipe), only the current link
    appears.*/
    /* (its length is C's allocated link count) */
    links: Vec<OggOpusLink>,
    /*Storage for the list of serial numbers from a single link.
    This is a scratch buffer used when scanning the BOS pages at the start of
    each link.*/
    /* (nserialnos is its length) */
    serialnos: Vec<u32>,
    /*This is the current offset of the data processed by the ogg_sync_state.
    After a seek, this should be set to the target offset so that we can track
    the byte offsets of subsequent pages.
    After a call to op_get_next_page(), this will point to the first byte after
    that page.*/
    offset: i64,
    /*The total size of this stream, or -1 if it's unseekable.*/
    end: i64,
    /*Used to locate pages in the stream.*/
    oy: OggSyncState,
    /*One of OP_NOTOPEN, OP_PARTOPEN, OP_OPENED, OP_STREAMSET, OP_INITSET.*/
    ready_state: i32,
    /*The current link being played back.*/
    cur_link: i32,
    /*The number of decoded samples to discard from the start of decoding.*/
    cur_discard_count: i32,
    /*The granule position of the previous packet (current packet start time).*/
    prev_packet_gp: i64,
    /*The stream offset of the most recent page with completed packets, or -1.
    This is only needed to recover continued packet data in the seeking logic,
    when we use the current position as one of our bounds, only to later
    discover it was the correct starting point.*/
    prev_page_offset: i64,
    /*The number of bytes read since the last bitrate query, including framing.*/
    #[allow(dead_code)] /* (op_bitrate_instant() isn't translated) */ bytes_tracked: i64,
    /*The number of samples decoded since the last bitrate query.*/
    #[allow(dead_code)]
    samples_tracked: i64,
    /*Takes physical pages and welds them into a logical stream of packets.*/
    os: OggStreamState,
    /*Re-timestamped packets from a single page.
    Buffering these relies on the undocumented libogg behavior that ogg_packet
    pointers remain valid until the next page is submitted to the
    ogg_stream_state they came from.*/
    op: [OggPacket; 255],
    /*The index of the next packet to return.*/
    op_pos: i32,
    /*The total number of packets available.*/
    op_count: i32,
    /*Central working state for the packet-to-PCM decoder.*/
    od: Option<OpusMSDecoder>,
    /*The stream count used to initialize the decoder.*/
    od_stream_count: i32,
    /*The coupled stream count used to initialize the decoder.*/
    od_coupled_count: i32,
    /*The channel count used to initialize the decoder.*/
    od_channel_count: i32,
    /*The channel mapping used to initialize the decoder.*/
    od_mapping: [u8; OP_NCHANNELS_MAX],
    /*The buffered data for one decoded packet.*/
    /* (empty is C's NULL) */
    od_buffer: Vec<f32>,
    /*The current position in the decoded buffer.*/
    od_buffer_pos: i32,
    /*The number of valid samples in the decoded buffer.*/
    od_buffer_size: i32,
    /*The type of gain offset to apply.
    One of OP_HEADER_GAIN, OP_ALBUM_GAIN, OP_TRACK_GAIN, or OP_ABSOLUTE_GAIN.*/
    gain_type: i32,
    /*The offset to apply to the gain.*/
    gain_offset_q8: i32,
    /* (the float->short dithering and soft clipping state is only used by the
    int16 API, which isn't translated) */
}

impl std::fmt::Debug for OggOpusFile<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OggOpusFile")
            .field("nlinks", &self.nlinks)
            .finish_non_exhaustive()
    }
}

impl OggOpusFile<'_> {
    /* SDL_mixer's OPUS_IoCallbacks */

    /// `OPUS_IoRead()`.
    fn cb_read(stream: &mut IoStream<'_>, buf: &mut [u8]) -> i32 {
        stream.read(buf) as i32
    }

    /// `OPUS_IoSeek()`.
    fn cb_seek(&mut self, offset: i64, whence: i32) -> i32 {
        let whence = match whence {
            SEEK_SET => IoWhence::Set,
            SEEK_CUR => IoWhence::Cur,
            _ => IoWhence::End,
        };
        if self.stream.seek(offset, whence).is_err() {
            -1
        } else {
            0
        }
    }

    /// `OPUS_IoTell()`.
    fn cb_tell(&mut self) -> i64 {
        self.stream.tell().unwrap_or(-1)
    }
}

/*This implementation is largely based off of libvorbisfile.
All of the Ogg bits work roughly the same, though I have made some
"improvements" that have not been folded back there, yet.*/

/*A 'chained bitstream' is an Ogg Opus bitstream that contains more than one
logical bitstream arranged end to end (the only form of Ogg multiplexing
supported by this library.
Grouping (parallel multiplexing) is not supported, except to the extent that
if there are multiple logical Ogg streams in a single link of the chain, we
will ignore all but the first Opus stream we find.*/

/*An Ogg Opus file can be played beginning to end (streamed) without worrying
ahead of time about chaining (see opusdec from the opus-tools package).
If we have the whole file, however, and want random access
(seeking/scrubbing) or desire to know the total length/time of a file, we
need to account for the possibility of chaining.*/

/*We can handle things a number of ways.
We can determine the entire bitstream structure right off the bat, or find
pieces on demand.
This library determines and caches structure for the entire bitstream, but
builds a virtual decoder on the fly when moving between links in the chain.*/

/*There are also different ways to implement seeking.
Enough information exists in an Ogg bitstream to seek to sample-granularity
positions in the output.
Or, one can seek by picking some portion of the stream roughly in the desired
area if we only want coarse navigation through the stream.
We implement and expose both strategies.*/

/*The maximum number of bytes in a page (including the page headers).*/
const OP_PAGE_SIZE_MAX: i64 = 65307;
/*The default amount to seek backwards per step when trying to find the
previous page.
This must be at least as large as the maximum size of a page.*/
const OP_CHUNK_SIZE: i32 = 65536;
/*The maximum amount to seek backwards per step when trying to find the
previous page.*/
const OP_CHUNK_SIZE_MAX: i32 = 1024 * 1024;
/*A smaller read size is needed for low-rate streaming.*/
const OP_READ_SIZE: i32 = 2048;

/*Many, many internal helpers.
The intention is not to be confusing.
Rampant duplication and monolithic function implementation (though we do have
some large, omnibus functions still) would be harder to understand anyway.
The high level functions are last.
Begin grokking near the end of the file if you prefer to read things
top-down.*/

/*The read/seek functions track absolute position within the stream.*/

/*Read a little more data from the file/pipe into the ogg_sync framer.
_nbytes: The maximum number of bytes to read.
Return: A positive number of bytes read on success, 0 on end-of-file, or a
negative value on failure.*/
fn op_get_data(of: &mut OggOpusFile<'_>, nbytes: i32) -> i32 {
    debug_assert!(nbytes > 0);
    let Some(buffer) = ogg_sync_buffer(&mut of.oy, nbytes as i64) else {
        return OP_EFAULT;
    };
    let nbytes_read = OggOpusFile::cb_read(&mut of.stream, &mut buffer[..nbytes as usize]);
    debug_assert!(nbytes_read <= nbytes);
    if nbytes_read > 0 {
        ogg_sync_wrote(&mut of.oy, nbytes_read as i64);
    }
    nbytes_read
}

/*Save a tiny smidge of verbosity to make the code more readable.*/
fn op_seek_helper(of: &mut OggOpusFile<'_>, offset: i64) -> i32 {
    if offset == of.offset {
        return 0;
    }
    if of.cb_seek(offset, SEEK_SET) != 0 {
        return OP_EREAD;
    }
    of.offset = offset;
    ogg_sync_reset(&mut of.oy);
    0
}

/*Get the current position indicator of the underlying stream.
This should be the same as the value reported by tell().*/
fn op_position(of: &OggOpusFile<'_>) -> i64 {
    /*The current position indicator is _not_ simply offset.
    We may also have unprocessed, buffered data in the sync state.*/
    of.offset + of.oy_fill() as i64 - of.oy_returned() as i64
}

impl OggOpusFile<'_> {
    fn oy_fill(&self) -> i32 {
        ogg_sync_fill(&self.oy)
    }
    fn oy_returned(&self) -> i32 {
        ogg_sync_returned(&self.oy)
    }
}

/*From the head of the stream, get the next page.
_boundary specifies if the function is allowed to fetch more data from the
stream (and how much) or only use internally buffered data.
_boundary: -1: Unbounded search.
            0: Read no additional data.
               Use only cached data.
            n: Search for the start of a new page up to file position n.
Return: n>=0:       Found a page at absolute offset n.
        OP_FALSE:   Hit the _boundary limit.
        OP_EREAD:   An underlying read operation failed.
        OP_BADLINK: We hit end-of-file before reaching _boundary.*/
fn op_get_next_page(of: &mut OggOpusFile<'_>, og: &mut OggPage, boundary: i64) -> i64 {
    while boundary <= 0 || of.offset < boundary {
        let more = ogg_sync_pageseek(&mut of.oy, Some(&mut *og));
        /*Skipped (-more) bytes.*/
        if more < 0 {
            of.offset -= more;
        } else if more == 0 {
            /*Send more paramedics.*/
            if boundary == 0 {
                return OP_FALSE as i64;
            }
            let read_nbytes = if boundary < 0 {
                OP_READ_SIZE
            } else {
                let position = op_position(of);
                if position >= boundary {
                    return OP_FALSE as i64;
                }
                (boundary - position).min(OP_READ_SIZE as i64) as i32
            };
            let ret = op_get_data(of, read_nbytes);
            if ret < 0 {
                return OP_EREAD as i64;
            }
            if ret == 0 {
                /*Only fail cleanly on EOF if we didn't have a known boundary.
                Otherwise, we should have been able to reach that boundary, and this
                is a fatal error.*/
                return if boundary < 0 { OP_FALSE } else { OP_EBADLINK } as i64;
            }
        } else {
            /*Got a page.
            Return the page start offset and advance the internal offset past the
            page end.*/
            let page_offset = of.offset;
            of.offset += more;
            debug_assert!(page_offset >= 0);
            return page_offset;
        }
    }
    OP_FALSE as i64
}

fn op_add_serialno(og: &OggPage, serialnos: &mut Vec<u32>) -> i32 {
    let s = ogg_page_serialno(og) as u32;
    if serialnos.len() >= (i32::MAX as usize / 4 - 1) >> 1 || serialnos.try_reserve(1).is_err() {
        return OP_EFAULT;
    }
    serialnos.push(s);
    0
}

/*Returns nonzero if found.*/
fn op_lookup_serialno(s: u32, serialnos: &[u32]) -> bool {
    serialnos.contains(&s)
}

fn op_lookup_page_serialno(og: &OggPage, serialnos: &[u32]) -> bool {
    op_lookup_serialno(ogg_page_serialno(og) as u32, serialnos)
}

/*We use this to remember the pages we found while enumerating the links of a
chained stream.
We keep track of the starting and ending offsets, as well as the point we
started searching from, so we know where to bisect.
We also keep the serial number, so we can tell if the page belonged to the
current link or not, as well as the granule position, to aid in estimating
the start of the link.*/
/// Translation of `struct OpusSeekRecord`.
#[derive(Clone, Copy, Debug, Default)]
struct OpusSeekRecord {
    /*The earliest byte we know of such that reading forward from it causes
    capture to be regained at this page.*/
    search_start: i64,
    /*The offset of this page.*/
    offset: i64,
    /*The size of this page.*/
    size: i32,
    /*The serial number of this page.*/
    serialno: u32,
    /*The granule position of this page.*/
    gp: i64,
}

/*Find the last page beginning before _offset with a valid granule position.
There is no '_boundary' parameter as it will always have to read more data.
This is much dirtier than the above, as Ogg doesn't have any backward search
linkage.
This search prefers pages of the specified serial number.
If a page of the specified serial number is spotted during the
seek-back-and-read-forward, it will return the info of last page of the
matching serial number, instead of the very last page, unless the very last
page belongs to a different link than preferred serial number.
If no page of the specified serial number is seen, it will return the info of
the last page.
[out] _sr:   Returns information about the page that was found on success.
_offset:     The _offset before which to find a page.
             Any page returned will consist of data entirely before _offset.
_serialno:   The preferred serial number.
             If a page with this serial number is found, it will be returned
              even if another page in the same link is found closer to
              _offset.
             This is purely opportunistic: there is no guarantee such a page
              will be found if it exists.
_serialnos:  The list of serial numbers in the link that contains the
              preferred serial number.
_nserialnos: The number of serial numbers in the current link.
Return: 0 on success, or a negative value on failure.
        OP_EREAD:    Failed to read more data (error or EOF).
        OP_EBADLINK: We couldn't find a page even after seeking back to the
                      start of the stream.*/
fn op_get_prev_page_serial(
    of: &mut OggOpusFile<'_>,
    sr: &mut OpusSeekRecord,
    mut offset: i64,
    serialno: u32,
    serialnos: &[u32],
) -> i32 {
    let mut preferred_sr = OpusSeekRecord::default();
    let mut og = OggPage::default();
    let original_end = offset;
    let mut end = offset;
    let mut begin = offset;
    let mut preferred_found = false;
    offset = -1;
    let mut chunk_size = OP_CHUNK_SIZE;
    loop {
        debug_assert!(chunk_size as i64 >= OP_PAGE_SIZE_MAX);
        begin = (begin - chunk_size as i64).max(0);
        let ret = op_seek_helper(of, begin);
        if ret < 0 {
            return ret;
        }
        let mut search_start = begin;
        while of.offset < end {
            let llret = op_get_next_page(of, &mut og, end);
            if llret < OP_FALSE as i64 {
                return llret as i32;
            } else if llret == OP_FALSE as i64 {
                break;
            }
            let page_serialno = ogg_page_serialno(&og) as u32;
            /*Save the information for this page.
            We're not interested in the page itself... just the serial number, byte
            offset, page size, and granule position.*/
            sr.search_start = search_start;
            offset = llret;
            sr.offset = offset;
            sr.serialno = page_serialno;
            debug_assert!(of.offset - offset >= 0);
            debug_assert!(of.offset - offset <= OP_PAGE_SIZE_MAX);
            sr.size = (of.offset - offset) as i32;
            sr.gp = ogg_page_granulepos(&og);
            /*If this page is from the stream we're looking for, remember it.*/
            if page_serialno == serialno {
                preferred_found = true;
                preferred_sr = *sr;
            }
            if !op_lookup_serialno(page_serialno, serialnos) {
                /*We fell off the end of the link, which means we seeked back too far
                and shouldn't have been looking in that link to begin with.
                If we found the preferred serial number, forget that we saw it.*/
                preferred_found = false;
            }
            search_start = llret + 1;
        }
        /*We started from the beginning of the stream and found nothing.
        This should be impossible unless the contents of the stream changed out
        from under us after we read from it.*/
        if begin == 0 && offset < 0 {
            return OP_EBADLINK;
        }
        /*Bump up the chunk size.
        This is mildly helpful when seeks are very expensive (http).*/
        chunk_size = (2 * chunk_size).min(OP_CHUNK_SIZE_MAX);
        /*Avoid quadratic complexity if we hit an invalid patch of the file.*/
        end = (begin + OP_PAGE_SIZE_MAX - 1).min(original_end);
        if offset >= 0 {
            break;
        }
    }
    if preferred_found {
        *sr = preferred_sr;
    }
    0
}

/*Find the last page beginning before _offset with the given serial number and
a valid granule position.
Unlike the above search, this continues until it finds such a page, but does
not stray outside the current link.
We could implement it (inefficiently) by calling op_get_prev_page_serial()
repeatedly until it returned a page that had both our preferred serial
number and a valid granule position, but doing it with a separate function
allows us to avoid repeatedly re-scanning valid pages from other streams as
we seek-back-and-read-forward.
[out] _gp:   Returns the granule position of the page that was found on
              success.
_offset:     The _offset before which to find a page.
             Any page returned will consist of data entirely before _offset.
_serialno:   The target serial number.
_serialnos:  The list of serial numbers in the link that contains the
              preferred serial number.
_nserialnos: The number of serial numbers in the current link.
Return: The offset of the page on success, or a negative value on failure.
        OP_EREAD:    Failed to read more data (error or EOF).
        OP_EBADLINK: We couldn't find a page even after seeking back past the
                      beginning of the link.*/
fn op_get_last_page(
    of: &mut OggOpusFile<'_>,
    gp_out: &mut i64,
    mut offset: i64,
    serialno: u32,
    serialnos: &[u32],
) -> i64 {
    let mut og = OggPage::default();
    /*The target serial number must belong to the current link.*/
    debug_assert!(op_lookup_serialno(serialno, serialnos));
    let original_end = offset;
    let mut end = offset;
    let mut begin = offset;
    offset = -1;
    /*We shouldn't have to initialize gp, but gcc is too dumb to figure out that
    ret>=0 implies we entered the if(page_gp!=-1) block at least once.*/
    let mut gp: i64 = -1;
    let mut chunk_size = OP_CHUNK_SIZE;
    loop {
        debug_assert!(chunk_size as i64 >= OP_PAGE_SIZE_MAX);
        begin = (begin - chunk_size as i64).max(0);
        let ret = op_seek_helper(of, begin);
        if ret < 0 {
            return ret as i64;
        }
        let mut left_link = false;
        while of.offset < end {
            let llret = op_get_next_page(of, &mut og, end);
            if llret < OP_FALSE as i64 {
                return llret;
            } else if llret == OP_FALSE as i64 {
                break;
            }
            let page_serialno = ogg_page_serialno(&og) as u32;
            if page_serialno == serialno {
                /*The page is from the right stream...*/
                let page_gp = ogg_page_granulepos(&og);
                if page_gp != -1 {
                    /*And has a valid granule position.
                    Let's remember it.*/
                    offset = llret;
                    gp = page_gp;
                }
            } else if !op_lookup_serialno(page_serialno, serialnos) {
                /*We fell off the start of the link, which means we don't need to keep
                seeking any farther back.*/
                left_link = true;
            }
        }
        /*We started from at or before the beginning of the link and found nothing.
        This should be impossible unless the contents of the stream changed out
        from under us after we read from it.*/
        if (left_link || begin == 0) && offset < 0 {
            return OP_EBADLINK as i64;
        }
        /*Bump up the chunk size.
        This is mildly helpful when seeks are very expensive (http).*/
        chunk_size = (2 * chunk_size).min(OP_CHUNK_SIZE_MAX);
        /*Avoid quadratic complexity if we hit an invalid patch of the file.*/
        end = (begin + OP_PAGE_SIZE_MAX - 1).min(original_end);
        if offset >= 0 {
            break;
        }
    }
    *gp_out = gp;
    offset
}

/*Uses the local ogg_stream storage in _of.
This is important for non-streaming input sources.*/
/// Translation of `op_fetch_headers_impl()`: the headers go to link `li`,
/// and `collect_serialnos` is whether C's `_serialnos` (`&_of->serialnos`)
/// is given.
fn op_fetch_headers_impl(
    of: &mut OggOpusFile<'_>,
    li: usize,
    collect_serialnos: bool,
    og: &mut OggPage,
) -> i32 {
    let mut op = OggPacket::default();
    if collect_serialnos {
        of.serialnos.clear();
    }
    /*Extract the serialnos of all BOS pages plus the first set of Opus headers
    we see in the link.*/
    while ogg_page_bos(og) != 0 {
        if collect_serialnos {
            if op_lookup_page_serialno(og, &of.serialnos) {
                /*A dupe serialnumber in an initial header packet set==invalid stream.*/
                return OP_EBADHEADER;
            }
            let ret = op_add_serialno(og, &mut of.serialnos);
            if ret < 0 {
                return ret;
            }
        }
        if of.ready_state < OP_STREAMSET {
            /*We don't have an Opus stream in this link yet, so begin prospective
            stream setup.
            We need a stream to get packets.*/
            ogg_stream_reset_serialno(&mut of.os, ogg_page_serialno(og));
            ogg_stream_pagein(&mut of.os, og);
            if ogg_stream_packetout(&mut of.os, Some(&mut op)) > 0 {
                let ret = opus_head_parse(Some(&mut of.links[li].head), of.os.packet_data(&op));
                /*Found a valid Opus header.
                Continue setup.*/
                if ret >= 0 {
                    of.ready_state = OP_STREAMSET;
                }
                /*If it's just a stream type we don't recognize, ignore it.
                Everything else is fatal.*/
                else if ret != OP_ENOTFORMAT {
                    return ret;
                }
            }
            /*TODO: Should a BOS page with no packets be an error?*/
        }
        /*Get the next page.
        No need to clamp the boundary offset against _of->end, as all errors
        become OP_ENOTFORMAT or OP_EBADHEADER.*/
        let boundary = op_adv_offset(of.offset, OP_CHUNK_SIZE as i64);
        if op_get_next_page(of, og, boundary) < 0 {
            return if of.ready_state < OP_STREAMSET {
                OP_ENOTFORMAT
            } else {
                OP_EBADHEADER
            };
        }
    }
    if of.ready_state != OP_STREAMSET {
        return OP_ENOTFORMAT;
    }
    /*If the first non-header page belonged to our Opus stream, submit it.*/
    if of.os.serialno == ogg_page_serialno(og) as i64 {
        ogg_stream_pagein(&mut of.os, og);
    }
    /*Loop getting packets.*/
    loop {
        match ogg_stream_packetout(&mut of.os, Some(&mut op)) {
            0 => {
                /*Loop getting pages.*/
                loop {
                    /*No need to clamp the boundary offset against _of->end, as all
                    errors become OP_EBADHEADER.*/
                    let boundary = op_adv_offset(of.offset, OP_CHUNK_SIZE as i64);
                    if op_get_next_page(of, og, boundary) < 0 {
                        return OP_EBADHEADER;
                    }
                    /*If this page belongs to the correct stream, go parse it.*/
                    if of.os.serialno == ogg_page_serialno(og) as i64 {
                        ogg_stream_pagein(&mut of.os, og);
                        break;
                    }
                    /*If the link ends before we see the Opus comment header, abort.*/
                    if ogg_page_bos(og) != 0 {
                        return OP_EBADHEADER;
                    }
                    /*Otherwise, keep looking.*/
                }
            }
            /*We shouldn't get a hole in the headers!*/
            -1 => return OP_EBADHEADER,
            _ => {
                /*Got a packet.
                It should be the comment header.*/
                let ret = opus_tags_parse(Some(&mut of.links[li].tags), of.os.packet_data(&op));
                if ret < 0 {
                    return ret;
                }
                /*Make sure the page terminated at the end of the comment header.
                If there is another packet on the page, or part of a packet, then
                reject the stream.
                Otherwise seekable sources won't be able to seek back to the start
                properly.*/
                let ret = ogg_stream_packetout(&mut of.os, Some(&mut op));
                if ret != 0 || og.header[og.header.len() - 1] == 255 {
                    /*If we fail, the caller assumes our tags are uninitialized.*/
                    opus_tags_clear(&mut of.links[li].tags);
                    return OP_EBADHEADER;
                }
                return 0;
            }
        }
    }
}

/// Translation of `op_fetch_headers()` (see [`op_fetch_headers_impl`]);
/// `og` is `None` for C's NULL.
fn op_fetch_headers(
    of: &mut OggOpusFile<'_>,
    li: usize,
    collect_serialnos: bool,
    og: Option<&mut OggPage>,
) -> i32 {
    let mut og_local = OggPage::default();
    let og = match og {
        Some(og) => og,
        None => {
            /*No need to clamp the boundary offset against _of->end, as all errors
            become OP_ENOTFORMAT.*/
            let boundary = op_adv_offset(of.offset, OP_CHUNK_SIZE as i64);
            if op_get_next_page(of, &mut og_local, boundary) < 0 {
                return OP_ENOTFORMAT;
            }
            &mut og_local
        }
    };
    of.ready_state = OP_OPENED;
    let ret = op_fetch_headers_impl(of, li, collect_serialnos, og);
    /*Revert back from OP_STREAMSET to OP_OPENED on failure, to prevent
    double-free of the tags in an unseekable stream.*/
    if ret < 0 {
        of.ready_state = OP_OPENED;
    }
    ret
}

/*Granule position manipulation routines.
A granule position is defined to be an unsigned 64-bit integer, with the
special value -1 in two's complement indicating an unset or invalid granule
position.
We are not guaranteed to have an unsigned 64-bit type, so we construct the
following routines that
a) Properly order negative numbers as larger than positive numbers, and
b) Check for underflow or overflow past the special -1 value.
This lets us operate on the full, valid range of granule positions in a
consistent and safe manner.
This full range is organized into distinct regions:
[ -1 (invalid) ][ 0 ... OP_INT64_MAX ][ OP_INT64_MIN ... -2 ][-1 (invalid) ]

No one should actually use granule positions so large that they're negative,
even if they are technically valid, as very little software handles them
correctly (including most of Xiph.Org's).
This library also refuses to support durations so large they won't fit in a
signed 64-bit integer (to avoid exposing this mess to the application, and
to simplify a good deal of internal arithmetic), so the only way to use them
successfully is if pcm_start is very large.
This means there isn't anything you can do with negative granule positions
that you couldn't have done with purely non-negative ones.
The main purpose of these routines is to allow us to think very explicitly
about the possible failure cases of all granule position manipulations.*/

/*Safely adds a small signed integer to a valid (not -1) granule position.
The result can use the full 64-bit range of values (both positive and
negative), but will fail on overflow (wrapping past -1; wrapping past
OP_INT64_MAX is explicitly okay).
[out] _dst_gp: The resulting granule position.
               Only modified on success.
_src_gp:       The granule position to add to.
               This must not be -1.
_delta:        The amount to add.
               This is allowed to be up to 32 bits to support the maximum
                duration of a single Ogg page (255 packets * 120 ms per
                packet == 1,468,800 samples at 48 kHz).
Return: 0 on success, or OP_EINVAL if the result would wrap around past -1.*/
fn op_granpos_add(dst_gp: &mut i64, mut src_gp: i64, mut delta: i32) -> i32 {
    /*The code below handles this case correctly, but there's no reason we
    should ever be called with these values, so make sure we aren't.*/
    debug_assert!(src_gp != -1);
    if delta > 0 {
        /*Adding this amount to the granule position would overflow its 64-bit
        range.*/
        if src_gp < 0 && src_gp >= -1 - delta as i64 {
            return OP_EINVAL;
        }
        if src_gp > OP_INT64_MAX - delta as i64 {
            /*Adding this amount to the granule position would overflow the positive
            half of its 64-bit range.
            Since signed overflow is undefined in C, do it in a way the compiler
            isn't allowed to screw up.*/
            delta -= (OP_INT64_MAX - src_gp) as i32 + 1;
            src_gp = OP_INT64_MIN;
        }
    } else if delta < 0 {
        /*Subtracting this amount from the granule position would underflow its
        64-bit range.*/
        if src_gp >= 0 && src_gp < -(delta as i64) {
            return OP_EINVAL;
        }
        if src_gp < OP_INT64_MIN - delta as i64 {
            /*Subtracting this amount from the granule position would underflow the
            negative half of its 64-bit range.
            Since signed underflow is undefined in C, do it in a way the compiler
            isn't allowed to screw up.*/
            delta += (src_gp - OP_INT64_MIN) as i32 + 1;
            src_gp = OP_INT64_MAX;
        }
    }
    *dst_gp = src_gp + delta as i64;
    0
}

/*Safely computes the difference between two granule positions.
The difference must fit in a signed 64-bit integer, or the function fails.
It correctly handles the case where the granule position has wrapped around
from positive values to negative ones.
[out] _delta: The difference between the granule positions.
              Only modified on success.
_gp_a:        The granule position to subtract from.
              This must not be -1.
_gp_b:        The granule position to subtract.
              This must not be -1.
Return: 0 on success, or OP_EINVAL if the result would not fit in a signed
64-bit integer.*/
fn op_granpos_diff(delta: &mut i64, gp_a: i64, gp_b: i64) -> i32 {
    /*The code below handles these cases correctly, but there's no reason we
    should ever be called with these values, so make sure we aren't.*/
    debug_assert!(gp_a != -1);
    debug_assert!(gp_b != -1);
    let gp_a_negative = gp_a < 0;
    let gp_b_negative = gp_b < 0;
    if gp_a_negative ^ gp_b_negative {
        if gp_a_negative {
            /*_gp_a has wrapped to a negative value but _gp_b hasn't: the difference
            should be positive.*/
            /*Step 1: Handle wrapping.*/
            /*_gp_a < 0 => da < 0.*/
            let da = (OP_INT64_MIN - gp_a) - 1;
            /*_gp_b >= 0  => db >= 0.*/
            let db = OP_INT64_MAX - gp_b;
            /*Step 2: Check for overflow.*/
            if OP_INT64_MAX + da < db {
                return OP_EINVAL;
            }
            *delta = db - da;
        } else {
            /*_gp_b has wrapped to a negative value but _gp_a hasn't: the difference
            should be negative.*/
            /*Step 1: Handle wrapping.*/
            /*_gp_a >= 0 => da <= 0*/
            let da = gp_a + OP_INT64_MIN;
            /*_gp_b < 0 => db <= 0*/
            let db = OP_INT64_MIN - gp_b;
            /*Step 2: Check for overflow.*/
            if da < OP_INT64_MIN - db {
                return OP_EINVAL;
            }
            *delta = da + db;
        }
    } else {
        *delta = gp_a - gp_b;
    }
    0
}

/// `op_granpos_add(&gp,gp,delta)`.
fn op_granpos_add_self(gp: &mut i64, delta: i32) -> i32 {
    let src_gp = *gp;
    op_granpos_add(gp, src_gp, delta)
}

fn op_granpos_cmp(gp_a: i64, gp_b: i64) -> i32 {
    /*The invalid granule position -1 should behave like NaN: neither greater
    than nor less than any other granule position, nor equal to any other
    granule position, including itself.
    However, that means there isn't anything we could sensibly return from this
    function for it.*/
    debug_assert!(gp_a != -1);
    debug_assert!(gp_b != -1);
    /*Handle the wrapping cases.*/
    if gp_a < 0 {
        if gp_b >= 0 {
            return 1;
        }
        /*Else fall through.*/
    } else if gp_b < 0 {
        return -1;
    }
    /*No wrapping case.*/
    (gp_a > gp_b) as i32 - (gp_b > gp_a) as i32
}

/*Returns the duration of the packet (in samples at 48 kHz), or a negative
value on error.*/
fn op_get_packet_duration(data: &[u8], len: i32) -> i32 {
    let nframes = opus_packet_get_nb_frames(data, len);
    if nframes < 0 {
        return OP_EBADPACKET;
    }
    let frame_size = opus_packet_get_samples_per_frame(data, 48000);
    let nsamples = nframes * frame_size;
    if nsamples > 120 * 48 {
        return OP_EBADPACKET;
    }
    nsamples
}

/*Grab all the packets currently in the stream state, and compute their
durations.
_of->op_count is set to the number of packets collected.
[out] _durations: Returns the durations of the individual packets.
Return: The total duration of all packets, or OP_HOLE if there was a hole.*/
fn op_collect_audio_packets(of: &mut OggOpusFile<'_>, durations: &mut [i32; 255]) -> i32 {
    /*Count the durations of all packets in the page.*/
    let mut op_count: usize = 0;
    let mut total_duration: i32 = 0;
    loop {
        /*This takes advantage of undocumented libogg behavior that returned
        ogg_packet buffers are valid at least until the next page is
        submitted.
        Relying on this is not too terrible, as _none_ of the Ogg memory
        ownership/lifetime rules are well-documented.
        But I can read its code and know this will work.*/
        /* (a page has at most 255 segments, so at most 255 packets; this
        never stops early, but rules out writing past op[]) */
        if op_count >= 255 {
            break;
        }
        let slot = op_count;
        let ret = ogg_stream_packetout(&mut of.os, Some(&mut of.op[slot]));
        if ret == 0 {
            break;
        }
        if ret < 0 {
            /*We shouldn't get holes in the middle of pages.*/
            debug_assert!(op_count == 0);
            /*Set the return value and break out of the loop.
            We want to make sure op_count gets set to 0, because we've ingested a
            page, so any previously loaded packets are now invalid.*/
            total_duration = OP_HOLE;
            break;
        }
        /*Unless libogg is broken, we can't get more than 255 packets from a
        single page.*/
        debug_assert!(op_count < 255);
        let op = of.op[slot];
        durations[slot] = op_get_packet_duration(of.os.packet_data(&op), op.bytes as i32);
        if durations[slot] > 0 {
            /*With at most 255 packets on a page, this can't overflow.*/
            total_duration += durations[slot];
            op_count += 1;
        }
        /*Ignore packets with an invalid TOC sequence.*/
        else if op_count > 0 {
            /*But save the granule position, if there was one.*/
            of.op[op_count - 1].granulepos = of.op[slot].granulepos;
        }
    }
    of.op_pos = 0;
    of.op_count = op_count as i32;
    total_duration
}

/*Starting from current cursor position, get the initial PCM offset of the next
page.
This also validates the granule position on the first page with a completed
audio data packet, as required by the spec.
If this link is completely empty (no pages with completed packets), then this
function sets pcm_start=pcm_end=0 and returns the BOS page of the next link
(if any).
In the seekable case, we initialize pcm_end=-1 before calling this function,
so that later we can detect that the link was empty before calling
op_find_final_pcm_offset().
[inout] _link: The link for which to find pcm_start.
[out] _og:     Returns the BOS page of the next link if this link was empty.
               In the unseekable case, we can then feed this to
                op_fetch_headers() to start the next link.
               The caller may pass NULL (e.g., for seekable streams), in
                which case this page will be discarded.
Return: 0 on success, 1 if there is a buffered BOS page available, or a
negative value on unrecoverable error.*/
fn op_find_initial_pcm_offset(
    of: &mut OggOpusFile<'_>,
    li: usize,
    og: Option<&mut OggPage>,
) -> i32 {
    let mut og_local = OggPage::default();
    let og = match og {
        Some(og) => og,
        None => &mut og_local,
    };
    let mut page_offset: i64;
    let mut pcm_start: i64 = 0;
    let mut durations = [0i32; 255];
    let serialno = of.os.serialno as u32;
    let mut op_count;
    /*We shouldn't have to initialize total_duration, but gcc is too dumb to
    figure out that op_count>0 implies we've been through the whole loop at
    least once.*/
    let mut total_duration: i32;
    loop {
        page_offset = op_get_next_page(of, og, of.end);
        /*We should get a page unless the file is truncated or mangled.
        Otherwise there are no audio data packets in the whole logical stream.*/
        if page_offset < 0 {
            /*Fail if there was a read error.*/
            if page_offset < OP_FALSE as i64 {
                return page_offset as i32;
            }
            let link = &mut of.links[li];
            /*Fail if the pre-skip is non-zero, since it's asking us to skip more
            samples than exist.*/
            if link.head.pre_skip > 0 {
                return OP_EBADTIMESTAMP;
            }
            link.pcm_file_offset = 0;
            /*Set pcm_end and end_offset so we can skip the call to
            op_find_final_pcm_offset().*/
            link.pcm_start = 0;
            link.pcm_end = 0;
            link.end_offset = link.data_offset;
            return 0;
        }
        /*Similarly, if we hit the next link in the chain, we've gone too far.*/
        if ogg_page_bos(og) != 0 {
            let link = &mut of.links[li];
            if link.head.pre_skip > 0 {
                return OP_EBADTIMESTAMP;
            }
            /*Set pcm_end and end_offset so we can skip the call to
            op_find_final_pcm_offset().*/
            link.pcm_file_offset = 0;
            link.pcm_start = 0;
            link.pcm_end = 0;
            link.end_offset = link.data_offset;
            /*Tell the caller we've got a buffered page for them.*/
            return 1;
        }
        /*Ignore pages from other streams (not strictly necessary, because of the
        checks in ogg_stream_pagein(), but saves some work).*/
        if serialno != ogg_page_serialno(og) as u32 {
            continue;
        }
        ogg_stream_pagein(&mut of.os, og);
        /*Bitrate tracking: add the header's bytes here.
        The body bytes are counted when we consume the packets.*/
        of.bytes_tracked += og.header.len() as i64;
        /*Count the durations of all packets in the page.*/
        loop {
            total_duration = op_collect_audio_packets(of, &mut durations);
            /*Ignore holes.*/
            if total_duration >= 0 {
                break;
            }
        }
        op_count = of.op_count;
        if op_count > 0 {
            break;
        }
    }
    let op_count = op_count as usize;
    /*We found the first page with a completed audio data packet: actually look
    at the granule position.
    RFC 3533 says, "A special value of -1 (in two's complement) indicates that
    no packets finish on this page," which does not say that a granule
    position that is NOT -1 indicates that some packets DO finish on that page
    (even though this was the intention, libogg itself violated this intention
    for years before we fixed it).
    The Ogg Opus specification only imposes its start-time requirements
    on the granule position of the first page with completed packets,
    so we ignore any set granule positions until then.*/
    let cur_page_gp = of.op[op_count - 1].granulepos;
    /*But getting a packet without a valid granule position on the page is not
    okay.*/
    if cur_page_gp == -1 {
        return OP_EBADTIMESTAMP;
    }
    let cur_page_eos = of.op[op_count - 1].e_o_s;
    if cur_page_eos == 0 {
        /*The EOS flag wasn't set.
        Work backwards from the provided granule position to get the starting PCM
        offset.*/
        if op_granpos_add(&mut pcm_start, cur_page_gp, -total_duration) < 0 {
            /*The starting granule position MUST not be smaller than the amount of
            audio on the first page with completed packets.*/
            return OP_EBADTIMESTAMP;
        }
    } else {
        /*The first page with completed packets was also the last.*/
        if op_granpos_add(&mut pcm_start, cur_page_gp, -total_duration) < 0 {
            /*If there's less audio on the page than indicated by the granule
            position, then we're doing end-trimming, and the starting PCM offset
            is zero by spec mandate.*/
            pcm_start = 0;
            /*However, the end-trimming MUST not ask us to trim more samples than
            exist after applying the pre-skip.*/
            if op_granpos_cmp(cur_page_gp, of.links[li].head.pre_skip as i64) < 0 {
                return OP_EBADTIMESTAMP;
            }
        }
    }
    /*Timestamp the individual packets.*/
    let mut prev_packet_gp = pcm_start;
    let mut pi = 0;
    while pi < op_count {
        if cur_page_eos != 0 {
            let mut diff: i64 = 0;
            op_granpos_diff(&mut diff, cur_page_gp, prev_packet_gp);
            diff = durations[pi] as i64 - diff;
            /*If we have samples to trim...*/
            if diff > 0 {
                /*If we trimmed the entire packet, stop (the spec says encoders
                shouldn't do this, but we support it anyway).*/
                if diff > durations[pi] as i64 {
                    break;
                }
                prev_packet_gp = cur_page_gp;
                of.op[pi].granulepos = cur_page_gp;
                /*Move the EOS flag to this packet, if necessary, so we'll trim the
                samples.*/
                of.op[pi].e_o_s = 1;
                pi += 1;
                continue;
            }
        }
        /*Update the granule position as normal.*/
        op_granpos_add(&mut of.op[pi].granulepos, prev_packet_gp, durations[pi]);
        prev_packet_gp = of.op[pi].granulepos;
        pi += 1;
    }
    /*Update the packet count after end-trimming.*/
    of.op_count = pi as i32;
    of.cur_discard_count = of.links[li].head.pre_skip as i32;
    of.links[li].pcm_file_offset = 0;
    of.links[li].pcm_start = pcm_start;
    of.prev_packet_gp = pcm_start;
    of.prev_page_offset = page_offset;
    0
}

/*Starting from current cursor position, get the final PCM offset of the
previous page.
This also validates the duration of the link, which, while not strictly
required by the spec, we need to ensure duration calculations don't
overflow.
This is only done for seekable sources.
We must validate that op_find_initial_pcm_offset() succeeded for this link
before calling this function, otherwise it will scan the entire stream
backwards until it reaches the start, and then fail.*/
#[allow(clippy::too_many_arguments)]
fn op_find_final_pcm_offset(
    of: &mut OggOpusFile<'_>,
    serialnos: &[u32],
    li: usize,
    mut offset: i64,
    end_serialno: u32,
    mut end_gp: i64,
    total_duration_out: &mut i64,
) -> i32 {
    let mut duration: i64 = 0;
    /*For the time being, fetch end PCM offset the simple way.*/
    let cur_serialno = of.links[li].serialno;
    if end_serialno != cur_serialno || end_gp == -1 {
        offset = op_get_last_page(of, &mut end_gp, offset, cur_serialno, serialnos);
        if offset < 0 {
            return offset as i32;
        }
    }
    let link = &mut of.links[li];
    /*At worst we should have found the first page with completed packets.*/
    if offset < link.data_offset {
        return OP_EBADLINK;
    }
    /*This implementation requires that the difference between the first and last
    granule positions in each link be representable in a signed, 64-bit
    number, and that each link also have at least as many samples as the
    pre-skip requires.*/
    if op_granpos_diff(&mut duration, end_gp, link.pcm_start) < 0
        || duration < link.head.pre_skip as i64
    {
        return OP_EBADTIMESTAMP;
    }
    /*We also require that the total duration be representable in a signed,
    64-bit number.*/
    duration -= link.head.pre_skip as i64;
    let total_duration = *total_duration_out;
    if OP_INT64_MAX - duration < total_duration {
        return OP_EBADTIMESTAMP;
    }
    *total_duration_out = total_duration + duration;
    link.pcm_end = end_gp;
    link.end_offset = offset;
    0
}

/*Rescale the number _x from the range [0,_from] to [0,_to].
_from and _to must be positive.*/
fn op_rescale64(mut x: i64, from: i64, to: i64) -> i64 {
    if x >= from {
        return to;
    }
    if x <= 0 {
        return 0;
    }
    let mut frac: i64 = 0;
    for _ in 0..63 {
        frac <<= 1;
        debug_assert!(x <= from);
        if x >= from >> 1 {
            x -= from - x;
            frac |= 1;
        } else {
            x <<= 1;
        }
    }
    let mut ret: i64 = 0;
    for _ in 0..63 {
        if frac & 1 != 0 {
            ret = (ret & to & 1) + (ret >> 1) + (to >> 1);
        } else {
            ret >>= 1;
        }
        frac >>= 1;
    }
    ret
}

/*The minimum granule position spacing allowed for making predictions.
This corresponds to about 1 second of audio at 48 kHz for both Opus and
Vorbis, or one keyframe interval in Theora with the default keyframe spacing
of 256.*/
const OP_GP_SPACING_MIN: i64 = 48000;

/*Try to estimate the location of the next link using the current seek
records, assuming the initial granule position of any streams we've found is
0.*/
fn op_predict_link_start(
    sr: &[OpusSeekRecord],
    searched: i64,
    mut end_searched: i64,
    bias: i32,
) -> i64 {
    /*Require that we be at least OP_CHUNK_SIZE from the end.
    We don't require that we be at least OP_CHUNK_SIZE from the beginning,
    because if we are we'll just scan forward without seeking.*/
    end_searched -= OP_CHUNK_SIZE as i64;
    if searched >= end_searched {
        return -1;
    }
    let mut bisect = end_searched;
    for sri in 0..sr.len() {
        /*If the granule position is negative, either it's invalid or we'd cause
        overflow.
        If it is larger than OP_INT64_MAX-OP_GP_SPACING_MIN, then no positive
        granule position would satisfy our minimum spacing requirements below.*/
        let gp1 = sr[sri].gp;
        if gp1 < 0 || gp1 > OP_INT64_MAX - OP_GP_SPACING_MIN {
            continue;
        }
        /*We require some minimum distance between granule positions to make an
        estimate.
        We don't actually know what granule position scheme is being used,
        because we have no idea what kind of stream these came from.
        Therefore we require a minimum spacing between them, with the
        expectation that while bitrates and granule position increments might
        vary locally in quite complex ways, they are globally smooth.*/
        let gp2_min = gp1 + OP_GP_SPACING_MIN;
        let offset1 = sr[sri].offset;
        let serialno1 = sr[sri].serialno;
        for srj in (0..sri).rev() {
            let mut gp2 = sr[srj].gp;
            if gp2 < gp2_min {
                continue;
            }
            /*Oh, and also make sure these came from the same stream.*/
            if sr[srj].serialno != serialno1 {
                continue;
            }
            let mut offset2 = sr[srj].offset;
            /*For once, we can subtract with impunity.*/
            let den = gp2 - gp1;
            let ipart = gp2 / den;
            let num = offset2 - offset1;
            debug_assert!(num > 0);
            if ipart > 0 && (offset2 - searched) / ipart < num {
                continue;
            }
            offset2 = offset2.wrapping_sub(ipart.wrapping_mul(num));
            gp2 -= ipart * den;
            offset2 = offset2.wrapping_sub(op_rescale64(gp2, den, num) - bias as i64);
            if offset2 < searched {
                continue;
            }
            bisect = bisect.min(offset2);
            break;
        }
    }
    if bisect >= end_searched {
        -1
    } else {
        bisect
    }
}

/*Finds each bitstream link, one at a time, using a bisection search.
This has to begin by knowing the offset of the first link's initial page.*/
fn op_bisect_forward_serialno(
    of: &mut OggOpusFile<'_>,
    mut searched: i64,
    sr: &mut [OpusSeekRecord; 64],
) -> i32 {
    let csr = sr.len();
    let mut og = OggPage::default();
    let mut nlinks = of.nlinks as usize;
    let mut total_duration: i64 = 0;
    /*We start with one seek record, for the last page in the file.
    We build up a list of records for places we seek to during link
    enumeration.
    This list is kept sorted in reverse order.
    We only care about seek locations that were _not_ in the current link,
    therefore we can add them one at a time to the end of the list as we
    improve the lower bound on the location where the next link starts.*/
    let mut nsr: usize = 1;
    loop {
        let mut end_offset: i64 = 0;
        let serialnos = of.serialnos.clone();
        if nlinks >= of.links.len() {
            let clinks = of.links.len();
            if clinks > (i32::MAX as usize - 1) >> 1 {
                return OP_EFAULT;
            }
            let clinks = 2 * clinks + 1;
            debug_assert!(nlinks < clinks);
            if of.links.try_reserve_exact(clinks - of.links.len()).is_err() {
                return OP_EFAULT;
            }
            of.links.resize(clinks, OggOpusLink::default());
        }
        /*Invariants:
        We have the headers and serial numbers for the link beginning at 'begin'.
        We have the offset and granule position of the last page in the file
        (potentially not a page we care about).*/
        /*Scan the seek records we already have to save us some bisection.*/
        let mut sri = 0;
        while sri < nsr {
            if op_lookup_serialno(sr[sri].serialno, &serialnos) {
                break;
            }
            sri += 1;
        }
        /*Is the last page in our current list of serial numbers?*/
        if sri == 0 {
            break;
        }
        /*Last page wasn't found.
        We have at least one more link.*/
        let mut last: i64 = -1;
        let mut end_searched = sr[sri - 1].search_start;
        let mut next = sr[sri - 1].offset;
        let mut end_gp: i64 = -1;
        if sri < nsr {
            searched = sr[sri].offset + sr[sri].size as i64;
            if sr[sri].serialno == of.links[nlinks - 1].serialno {
                end_gp = sr[sri].gp;
                end_offset = sr[sri].offset;
            }
        }
        nsr = sri;
        let mut bisect: i64 = -1;
        /*If we've already found the end of at least one link, try to pick the
        first bisection point at twice the average link size.
        This is a good choice for files with lots of links that are all about the
        same size.*/
        if nlinks > 1 {
            let last_offset = of.links[nlinks - 1].offset;
            let avg_link_size = last_offset / (nlinks as i64 - 1);
            let upper_limit = end_searched - OP_CHUNK_SIZE as i64 - avg_link_size;
            if last_offset > searched - avg_link_size && last_offset < upper_limit {
                bisect = last_offset + avg_link_size;
                if bisect < upper_limit {
                    bisect += avg_link_size;
                }
            }
        }
        /*We guard against garbage separating the last and first pages of two
        links below.*/
        while searched < end_searched {
            /*If we don't have a better estimate, use simple bisection.*/
            if bisect == -1 {
                bisect = searched + ((end_searched - searched) >> 1);
            }
            /*If we're within OP_CHUNK_SIZE of the start, scan forward.*/
            if bisect - searched < OP_CHUNK_SIZE as i64 {
                bisect = searched;
            }
            /*Otherwise we're skipping data.
            Forget the end page, if we saw one, as we might miss a later one.*/
            else {
                end_gp = -1;
            }
            let ret = op_seek_helper(of, bisect);
            if ret < 0 {
                return ret;
            }
            /*If there is a large region of invalid data in the middle of the file,
            avoid scanning it repeatedly.
            Because of the bisection, doing that would only be O(n*log(n)), not
            quadratic like op_get_last_page(), but still good to avoid.*/
            debug_assert!(end_searched <= sr[nsr - 1].search_start);
            let boundary = sr[nsr - 1]
                .offset
                .min(op_adv_offset(end_searched, OP_PAGE_SIZE_MAX - 1));
            last = op_get_next_page(of, &mut og, boundary);
            if last < OP_FALSE as i64 {
                return last as i32;
            }
            let mut next_bias = 0;
            if last == OP_FALSE as i64 {
                end_searched = bisect;
            } else {
                let serialno = ogg_page_serialno(&og) as u32;
                let gp = ogg_page_granulepos(&og);
                if !op_lookup_serialno(serialno, &serialnos) {
                    end_searched = bisect;
                    next = last;
                    /*In reality we should always have enough room, but be paranoid.*/
                    if nsr < csr {
                        sr[nsr].search_start = bisect;
                        sr[nsr].offset = last;
                        debug_assert!(of.offset - last >= 0);
                        debug_assert!(of.offset - last <= OP_PAGE_SIZE_MAX);
                        sr[nsr].size = (of.offset - last) as i32;
                        sr[nsr].serialno = serialno;
                        sr[nsr].gp = gp;
                        nsr += 1;
                    }
                } else {
                    searched = of.offset;
                    next_bias = OP_CHUNK_SIZE;
                    if serialno == of.links[nlinks - 1].serialno {
                        /*This page was from the stream we want, remember it.
                        If it's the last such page in the link, we won't have to go back
                        looking for it later.*/
                        end_gp = gp;
                        end_offset = last;
                    }
                }
            }
            bisect = op_predict_link_start(&sr[..nsr], searched, end_searched, next_bias);
        }
        /*Bisection point found.
        Get the final granule position of the previous link, assuming
        op_find_initial_pcm_offset() didn't already determine the link was
        empty.*/
        if of.links[nlinks - 1].pcm_end == -1 {
            if end_gp == -1 {
                /*If we don't know where the end page is, we'll have to seek back and
                look for it, starting from the end of the link.*/
                end_offset = next;
                /*Also forget the last page we read.
                It won't be available after the seek.*/
                last = -1;
            }
            let end_serialno = of.links[nlinks - 1].serialno;
            let ret = op_find_final_pcm_offset(
                of,
                &serialnos,
                nlinks - 1,
                end_offset,
                end_serialno,
                end_gp,
                &mut total_duration,
            );
            if ret < 0 {
                return ret;
            }
        }
        if last != next {
            /*The last page we read was not the first page the next link.
            Move the cursor position to the offset of that first page.
            This only performs an actual seek if the first page of the next link
            does not start at the end of the last page from the current Opus
            stream with a valid granule position.*/
            let ret = op_seek_helper(of, next);
            if ret < 0 {
                return ret;
            }
        }
        let ret = op_fetch_headers(
            of,
            nlinks,
            true,
            if last != next { None } else { Some(&mut og) },
        );
        if ret < 0 {
            return ret;
        }
        /*Mark the current link count so it can be cleaned up on error.*/
        of.nlinks = nlinks as i32 + 1;
        of.links[nlinks].offset = next;
        of.links[nlinks].data_offset = of.offset;
        of.links[nlinks].serialno = of.os.serialno as u32;
        of.links[nlinks].pcm_end = -1;
        /*This might consume a page from the next link, however the next bisection
        always starts with a seek.*/
        let ret = op_find_initial_pcm_offset(of, nlinks, None);
        if ret < 0 {
            return ret;
        }
        of.links[nlinks].pcm_file_offset = total_duration;
        searched = of.offset;
        nlinks += 1;
    }
    /*Last page is in the starting serialno list, so we've reached the last link.
    Now find the last granule position for it (if we didn't the first time we
    looked at the end of the stream, and if op_find_initial_pcm_offset()
    didn't already determine the link was empty).*/
    if of.links[nlinks - 1].pcm_end == -1 {
        let serialnos = of.serialnos.clone();
        let ret = op_find_final_pcm_offset(
            of,
            &serialnos,
            nlinks - 1,
            sr[0].offset,
            sr[0].serialno,
            sr[0].gp,
            &mut total_duration,
        );
        if ret < 0 {
            return ret;
        }
    }
    /*Trim back the links array if necessary.*/
    of.links.truncate(nlinks);
    /*We also don't need these anymore.*/
    of.serialnos = Vec::new();
    0
}

fn op_update_gain(of: &mut OggOpusFile<'_>) {
    /*If decode isn't ready, then we'll apply the gain when we initialize the
    decoder.*/
    if of.ready_state < OP_INITSET {
        return;
    }
    let mut gain_q8 = of.gain_offset_q8;
    let li = if of.seekable != 0 {
        of.cur_link as usize
    } else {
        0
    };
    let head = &of.links[li].head;
    /*We don't have to worry about overflow here because the header gain and
    track gain must lie in the range [-32768,32767], and the user-supplied
    offset has been pre-clamped to [-98302,98303].*/
    match of.gain_type {
        OP_ALBUM_GAIN => {
            let mut album_gain_q8 = 0;
            opus_tags_get_album_gain(&of.links[li].tags, &mut album_gain_q8);
            gain_q8 += album_gain_q8;
            gain_q8 += head.output_gain;
        }
        OP_TRACK_GAIN => {
            let mut track_gain_q8 = 0;
            opus_tags_get_track_gain(&of.links[li].tags, &mut track_gain_q8);
            gain_q8 += track_gain_q8;
            gain_q8 += head.output_gain;
        }
        OP_HEADER_GAIN => gain_q8 += head.output_gain,
        OP_ABSOLUTE_GAIN => {}
        _ => debug_assert!(false),
    }
    gain_q8 = gain_q8.clamp(-32768, 32767);
    debug_assert!(of.od.is_some());
    if let Some(od) = of.od.as_mut() {
        od.set_gain(gain_q8);
    }
}

fn op_make_decode_ready(of: &mut OggOpusFile<'_>) -> i32 {
    if of.ready_state > OP_STREAMSET {
        return 0;
    }
    if of.ready_state < OP_STREAMSET {
        return OP_EFAULT;
    }
    let li = if of.seekable != 0 {
        of.cur_link as usize
    } else {
        0
    };
    let head = &of.links[li].head;
    let stream_count = head.stream_count;
    let coupled_count = head.coupled_count;
    let channel_count = head.channel_count;
    let cc = channel_count as usize;
    /*Check to see if the current decoder is compatible with the current link.*/
    if of.od.is_some()
        && of.od_stream_count == stream_count
        && of.od_coupled_count == coupled_count
        && of.od_channel_count == channel_count
        && of.od_mapping[..cc] == head.mapping[..cc]
    {
        if let Some(od) = of.od.as_mut() {
            od.reset_state();
        }
    } else {
        of.od = None;
        match OpusMSDecoder::create(
            48000,
            channel_count,
            stream_count,
            coupled_count,
            &head.mapping,
        ) {
            Ok(od) => of.od = Some(od),
            Err(_) => return OP_EFAULT,
        }
        of.od_stream_count = stream_count;
        of.od_coupled_count = coupled_count;
        of.od_channel_count = channel_count;
        of.od_mapping[..cc].copy_from_slice(&of.links[li].head.mapping[..cc]);
    }
    of.ready_state = OP_INITSET;
    of.bytes_tracked = 0;
    of.samples_tracked = 0;
    /* (state_channel_count and dither_seed, the int16 API's, aren't kept) */
    op_update_gain(of);
    0
}

fn op_open_seekable2_impl(of: &mut OggOpusFile<'_>) -> i32 {
    /*64 seek records should be enough for anybody.
    Actually, with a bisection search in a 63-bit range down to OP_CHUNK_SIZE
    granularity, much more than enough.*/
    let mut sr = [OpusSeekRecord::default(); 64];
    /*We can seek, so set out learning all about this file.*/
    of.cb_seek(0, SEEK_END);
    of.end = of.cb_tell();
    of.offset = of.end;
    if of.end < 0 {
        return OP_EREAD;
    }
    let data_offset = of.links[0].data_offset;
    if of.end < data_offset {
        return OP_EBADLINK;
    }
    /*Get the offset of the last page of the physical bitstream, or, if we're
    lucky, the last Opus page of the first link, as most Ogg Opus files will
    contain a single logical bitstream.*/
    let serialnos = of.serialnos.clone();
    let (end, serialno) = (of.end, of.links[0].serialno);
    let ret = op_get_prev_page_serial(of, &mut sr[0], end, serialno, &serialnos);
    if ret < 0 {
        return ret;
    }
    /*If there's any trailing junk, forget about it.*/
    of.end = sr[0].offset + sr[0].size as i64;
    if of.end < data_offset {
        return OP_EBADLINK;
    }
    /*Now enumerate the bitstream structure.*/
    op_bisect_forward_serialno(of, data_offset, &mut sr)
}

fn op_open_seekable2(of: &mut OggOpusFile<'_>) -> i32 {
    /*We're partially open and have a first link header state in storage in _of.
    Save off that stream state so we can come back to it.
    It would be simpler to just dump all this state and seek back to
    links[0].data_offset when we're done.
    But we do the extra work to allow us to seek back to _exactly_ the same
    stream position we're at now.
    This allows, e.g., the HTTP backend to continue reading from the original
    connection (if it's still available), instead of opening a new one.
    This means we can open and start playing a normal Opus file with a single
    link and reasonable packet sizes using only two HTTP requests.*/
    let start_op_count = of.op_count;
    /*This is a bit too large to put on the stack unconditionally.*/
    let op_start = of.op;
    let oy_start = std::mem::take(&mut of.oy);
    let os_start = std::mem::take(&mut of.os);
    let prev_page_offset = of.prev_page_offset;
    let start_offset = of.offset;
    /* (C asserts here that tell() matches op_position()) */
    ogg_sync_init(&mut of.oy);
    ogg_stream_init(&mut of.os, -1);
    let ret = op_open_seekable2_impl(of);
    /*Restore the old stream state.*/
    ogg_stream_clear(&mut of.os);
    ogg_sync_clear(&mut of.oy);
    of.oy = oy_start;
    of.os = os_start;
    of.offset = start_offset;
    of.op_count = start_op_count;
    of.op = op_start;
    of.prev_packet_gp = of.links[0].pcm_start;
    of.prev_page_offset = prev_page_offset;
    of.cur_discard_count = of.links[0].head.pre_skip as i32;
    if ret < 0 {
        return ret;
    }
    /*And restore the position indicator.*/
    let pos = op_position(of);
    let ret = of.cb_seek(pos, SEEK_SET);
    if ret < 0 {
        OP_EREAD
    } else {
        0
    }
}

/*Clear out the current logical bitstream decoder.*/
fn op_decode_clear(of: &mut OggOpusFile<'_>) {
    /*We don't actually free the decoder.
    We might be able to re-use it for the next link.*/
    of.op_count = 0;
    of.od_buffer_size = 0;
    of.prev_packet_gp = -1;
    of.prev_page_offset = -1;
    if of.seekable == 0 {
        debug_assert!(of.ready_state >= OP_INITSET);
        opus_tags_clear(&mut of.links[0].tags);
    }
    of.ready_state = OP_OPENED;
}

/* (op_clear() is dropping the OggOpusFile: the decoder, links, buffers and
framing states are freed, and SDL_mixer's close callback does nothing) */

fn op_open1<'a>(stream: IoStream<'a>, initial_data: &[u8]) -> Result<Box<OggOpusFile<'a>>, i32> {
    let mut og = OggPage::default();
    let mut pog = false;
    let mut of = Box::new(OggOpusFile {
        stream,
        seekable: 0,
        nlinks: 0,
        links: Vec::new(),
        serialnos: Vec::new(),
        offset: 0,
        end: -1,
        oy: OggSyncState::default(),
        ready_state: OP_NOTOPEN,
        cur_link: 0,
        cur_discard_count: 0,
        prev_packet_gp: 0,
        prev_page_offset: 0,
        bytes_tracked: 0,
        samples_tracked: 0,
        os: OggStreamState::default(),
        op: [OggPacket::default(); 255],
        op_pos: 0,
        op_count: 0,
        od: None,
        od_stream_count: 0,
        od_coupled_count: 0,
        od_channel_count: 0,
        od_mapping: [0; OP_NCHANNELS_MAX],
        od_buffer: Vec::new(),
        od_buffer_pos: 0,
        od_buffer_size: 0,
        gain_type: OP_HEADER_GAIN,
        gain_offset_q8: 0,
    });
    /*At a minimum, we need to be able to read data.*/
    /*Initialize the framing state.*/
    ogg_sync_init(&mut of.oy);
    /*Perhaps some data was previously read into a buffer for testing against
    other stream types.
    Allow initialization from this previously read data (especially as we may
    be reading from a non-seekable stream).
    This requires copying it into a buffer allocated by ogg_sync_buffer() and
    doesn't support seeking, so this is not a good mechanism to use for
    decoding entire files from RAM.*/
    if !initial_data.is_empty() {
        let Some(buffer) = ogg_sync_buffer(&mut of.oy, initial_data.len() as i64) else {
            return Err(OP_EFAULT);
        };
        buffer[..initial_data.len()].copy_from_slice(initial_data);
        ogg_sync_wrote(&mut of.oy, initial_data.len() as i64);
    }
    /*Can we seek?
    Stevens suggests the seek test is portable.
    It's actually not for files on win32, but we address that by fixing it in
    our callback implementation (see stream.c).*/
    let seekable = of.cb_seek(0, SEEK_CUR) != -1;
    /*If seek is implemented, tell must also be implemented.*/
    if seekable {
        let pos = of.cb_tell();
        /*If the current position is not equal to the initial bytes consumed,
        absolute seeking will not work.*/
        if pos != initial_data.len() as i64 {
            return Err(OP_EINVAL);
        }
    }
    of.seekable = seekable as i32;
    /*Don't seek yet.
    Set up a 'single' (current) logical bitstream entry for partial open.*/
    if of.links.try_reserve_exact(1).is_err() {
        return Err(OP_EFAULT);
    }
    of.links.push(OggOpusLink::default());
    /*The serialno gets filled in later by op_fetch_headers().*/
    ogg_stream_init(&mut of.os, -1);
    let mut ret;
    loop {
        /*Fetch all BOS pages, store the Opus header and all seen serial numbers,
        and load subsequent Opus setup headers.*/
        ret = op_fetch_headers(&mut of, 0, true, if pog { Some(&mut og) } else { None });
        if ret < 0 {
            break;
        }
        of.nlinks = 1;
        of.links[0].offset = 0;
        of.links[0].data_offset = of.offset;
        of.links[0].pcm_end = -1;
        of.links[0].serialno = of.os.serialno as u32;
        /*Fetch the initial PCM offset.*/
        ret = op_find_initial_pcm_offset(&mut of, 0, Some(&mut og));
        if seekable || ret <= 0 {
            break;
        }
        /*This link was empty, but we already have the BOS page for the next one in
        og.
        We can't seek, so start processing the next link right now.*/
        opus_tags_clear(&mut of.links[0].tags);
        of.nlinks = 0;
        if !seekable {
            of.cur_link += 1;
        }
        pog = true;
    }
    if ret >= 0 {
        of.ready_state = OP_PARTOPEN;
        Ok(of)
    } else {
        Err(ret)
    }
}

fn op_open2(of: &mut OggOpusFile<'_>) -> i32 {
    debug_assert!(of.ready_state == OP_PARTOPEN);
    let mut ret = if of.seekable != 0 {
        of.ready_state = OP_OPENED;
        op_open_seekable2(of)
    } else {
        0
    };
    if ret >= 0 {
        /*We have buffered packets from op_find_initial_pcm_offset().
        Move to OP_INITSET so we can use them.*/
        of.ready_state = OP_STREAMSET;
        ret = op_make_decode_ready(of);
        if ret >= 0 {
            return 0;
        }
    }
    /*Don't auto-close the stream on failure.*/
    /* (the caller drops the OggOpusFile: op_clear()) */
    ret
}

/// Translation of `op_test_callbacks()`, with SDL_mixer's callbacks over
/// `stream`.
pub(crate) fn op_test_callbacks<'a>(
    stream: IoStream<'a>,
    initial_data: &[u8],
) -> Result<Box<OggOpusFile<'a>>, i32> {
    /* (on failure, dropping the half-open file is C's op_clear()) */
    op_open1(stream, initial_data)
}

/// Translation of `op_open_callbacks()`, with SDL_mixer's callbacks over
/// `stream`.
pub(crate) fn op_open_callbacks<'a>(
    stream: IoStream<'a>,
    initial_data: &[u8],
) -> Result<Box<OggOpusFile<'a>>, i32> {
    let mut of = op_test_callbacks(stream, initial_data)?;
    let ret = op_open2(&mut of);
    if ret >= 0 {
        return Ok(of);
    }
    Err(ret)
}

/// Translation of `op_seekable()`.
#[allow(dead_code)] /* (SDL_mixer loads it, but doesn't call it) */
pub(crate) fn op_seekable(of: &OggOpusFile<'_>) -> i32 {
    of.seekable
}

/// Translation of `op_pcm_total()`.
pub(crate) fn op_pcm_total(of: &OggOpusFile<'_>, mut li: i32) -> i64 {
    let nlinks = of.nlinks;
    if of.ready_state < OP_OPENED || of.seekable == 0 || li >= nlinks {
        return OP_EINVAL as i64;
    }
    let links = &of.links;
    /*We verify that the granule position differences are larger than the
    pre-skip and that the total duration does not overflow during link
    enumeration, so we don't have to check here.*/
    let mut pcm_total: i64 = 0;
    if li < 0 {
        pcm_total = links[nlinks as usize - 1].pcm_file_offset;
        li = nlinks - 1;
    }
    let link = &links[li as usize];
    let mut diff: i64 = 0;
    op_granpos_diff(&mut diff, link.pcm_end, link.pcm_start);
    pcm_total + (diff - link.head.pre_skip as i64)
}

/// Translation of `op_head()`.
pub(crate) fn op_head<'b>(of: &'b OggOpusFile<'_>, mut li: i32) -> &'b OpusHead {
    if li >= of.nlinks {
        li = of.nlinks - 1;
    }
    if of.seekable == 0 {
        li = 0;
    }
    &of.links[if li < 0 { of.cur_link } else { li } as usize].head
}

/// Translation of `op_tags()`.
pub(crate) fn op_tags<'b>(of: &'b OggOpusFile<'_>, mut li: i32) -> Option<&'b OpusTags> {
    if li >= of.nlinks {
        li = of.nlinks - 1;
    }
    if of.seekable == 0 {
        if of.ready_state < OP_STREAMSET && of.ready_state != OP_PARTOPEN {
            return None;
        }
        li = 0;
    } else if li < 0 {
        li = if of.ready_state >= OP_STREAMSET {
            of.cur_link
        } else {
            0
        };
    }
    Some(&of.links[li as usize].tags)
}

/*Given a serialno, find a link with a corresponding Opus stream, if it exists.
Return: The index of the link to which the page belongs, or a negative number
if it was not a desired Opus bitstream section.*/
fn op_get_link_from_serialno(
    of: &OggOpusFile<'_>,
    mut cur_link: i32,
    page_offset: i64,
    serialno: u32,
) -> i32 {
    debug_assert!(of.seekable != 0);
    let links = &of.links;
    let nlinks = of.nlinks;
    let mut li_lo = 0;
    /*Start off by guessing we're just a multiplexed page in the current link.*/
    let mut li_hi = if cur_link + 1 < nlinks && page_offset < links[cur_link as usize + 1].offset {
        cur_link + 1
    } else {
        nlinks
    };
    loop {
        if page_offset >= links[cur_link as usize].offset {
            li_lo = cur_link;
        } else {
            li_hi = cur_link;
        }
        cur_link = li_lo + ((li_hi - li_lo) >> 1);
        if li_hi - li_lo <= 1 {
            break;
        }
    }
    /*We've identified the link that should contain this page.
    Make sure it's a page we care about.*/
    if links[cur_link as usize].serialno != serialno {
        return OP_FALSE;
    }
    cur_link
}

/*Fetch and process a page.
This handles the case where we're at a bitstream boundary and dumps the
decoding machine.
If the decoding machine is unloaded, it loads it.
It also keeps prev_packet_gp up to date (seek and read both use this).
Return: <0) Error, OP_HOLE (lost packet), or OP_EOF.
         0) Got at least one audio data packet.*/
fn op_fetch_and_process_page(
    of: &mut OggOpusFile<'_>,
    mut og_in: Option<OggPage>,
    mut page_offset: i64,
    spanp: bool,
    mut ignore_holes: bool,
) -> i32 {
    /*We shouldn't get here if we have unprocessed packets.*/
    debug_assert!(of.ready_state < OP_INITSET || of.op_pos >= of.op_count);
    let seekable = of.seekable != 0;
    let mut cur_link = if seekable { of.cur_link } else { 0 };
    let mut cur_serialno = of.links[cur_link as usize].serialno;
    /*Handle one page.*/
    loop {
        let mut og;
        debug_assert!(of.ready_state >= OP_OPENED);
        /*If we were given a page to use, use it.*/
        if let Some(given) = og_in.take() {
            og = given;
        }
        /*Keep reading until we get a page with the correct serialno.*/
        else {
            og = OggPage::default();
            page_offset = op_get_next_page(of, &mut og, of.end);
        }
        /*EOF: Leave uninitialized.*/
        if page_offset < 0 {
            return if page_offset < OP_FALSE as i64 {
                page_offset as i32
            } else {
                OP_EOF
            };
        }
        if of.ready_state >= OP_STREAMSET && cur_serialno != ogg_page_serialno(&og) as u32 {
            /*Two possibilities:
            1) Another stream is multiplexed into this logical section, or*/
            if ogg_page_bos(&og) == 0 {
                continue;
            }
            /* 2) Our decoding just traversed a bitstream boundary.*/
            if !spanp {
                return OP_EOF;
            }
            if of.ready_state >= OP_INITSET {
                op_decode_clear(of);
            }
        }
        /*Bitrate tracking: add the header's bytes here.
        The body bytes are counted when we consume the packets.*/
        else {
            of.bytes_tracked += og.header.len() as i64;
        }
        /*Do we need to load a new machine before submitting the page?
        This is different in the seekable and non-seekable cases.
        In the seekable case, we already have all the header information loaded
        and cached.
        We just initialize the machine with it and continue on our merry way.
        In the non-seekable (streaming) case, we'll only be at a boundary if we
        just left the previous logical bitstream, and we're now nominally at the
        header of the next bitstream.*/
        if of.ready_state < OP_STREAMSET {
            if seekable {
                let serialno = ogg_page_serialno(&og) as u32;
                /*Match the serialno to bitstream section.*/
                debug_assert!(cur_link >= 0 && cur_link < of.nlinks);
                if of.links[cur_link as usize].serialno != serialno {
                    /*It wasn't a page from the current link.
                    Is it from the next one?*/
                    if cur_link + 1 < of.nlinks
                        && of.links[cur_link as usize + 1].serialno == serialno
                    {
                        cur_link += 1;
                    } else {
                        let new_link =
                            op_get_link_from_serialno(of, cur_link, page_offset, serialno);
                        /*Not a desired Opus bitstream section.
                        Keep trying.*/
                        if new_link < 0 {
                            continue;
                        }
                        cur_link = new_link;
                    }
                }
                cur_serialno = serialno;
                of.cur_link = cur_link;
                ogg_stream_reset_serialno(&mut of.os, serialno as i32);
                of.ready_state = OP_STREAMSET;
                /*If we're at the start of this link, initialize the granule position
                and pre-skip tracking.*/
                let link = &of.links[cur_link as usize];
                if page_offset <= link.data_offset {
                    of.prev_packet_gp = link.pcm_start;
                    of.prev_page_offset = -1;
                    of.cur_discard_count = link.head.pre_skip as i32;
                    /*Ignore a hole at the start of a new link (this is common for
                    streams joined in the middle) or after seeking.*/
                    ignore_holes = true;
                }
            } else {
                loop {
                    /*We're streaming.
                    Fetch the two header packets, build the info struct.*/
                    let ret = op_fetch_headers(of, 0, false, Some(&mut og));
                    if ret < 0 {
                        return ret;
                    }
                    /*op_find_initial_pcm_offset() will suppress any initial hole for us,
                    so no need to set _ignore_holes.*/
                    let ret = op_find_initial_pcm_offset(of, 0, Some(&mut og));
                    if ret < 0 {
                        return ret;
                    }
                    cur_serialno = of.os.serialno as u32;
                    of.links[0].serialno = cur_serialno;
                    of.cur_link += 1;
                    /*If the link was empty, keep going, because we already have the
                    BOS page of the next one in og.*/
                    if ret <= 0 {
                        break;
                    }
                }
                /*If we didn't get any packets out of op_find_initial_pcm_offset(),
                keep going (this is possible if end-trimming trimmed them all).*/
                if of.op_count <= 0 {
                    continue;
                }
                /*Otherwise, we're done.
                TODO: This resets bytes_tracked, which misses the header bytes
                already processed by op_find_initial_pcm_offset().*/
                let ret = op_make_decode_ready(of);
                if ret < 0 {
                    return ret;
                }
                return 0;
            }
        }
        /*The buffered page is the data we want, and we're ready for it.
        Add it to the stream state.*/
        if of.ready_state == OP_STREAMSET {
            let ret = op_make_decode_ready(of);
            if ret < 0 {
                return ret;
            }
        }
        /*Extract all the packets from the current page.*/
        ogg_stream_pagein(&mut of.os, &og);
        if of.ready_state >= OP_INITSET {
            let mut durations = [0i32; 255];
            let mut report_hole = false;
            let mut total_duration = op_collect_audio_packets(of, &mut durations);
            if total_duration < 0 {
                /*libogg reported a hole (a gap in the page sequence numbers).
                Drain the packets from the page anyway.
                If we don't, they'll still be there when we fetch the next page.
                Then, when we go to pull out packets, we might get more than 255,
                which would overrun our packet buffer.
                We repeat this call until we get any actual packets, since we might
                have buffered multiple out-of-sequence pages with no packets on
                them.*/
                loop {
                    total_duration = op_collect_audio_packets(of, &mut durations);
                    if total_duration >= 0 {
                        break;
                    }
                }
                if !ignore_holes {
                    /*Report the hole to the caller after we finish timestamping the
                    packets.*/
                    report_hole = true;
                    /*We had lost or damaged pages, so reset our granule position
                    tracking.
                    This makes holes behave the same as a small raw seek.
                    If the next page is the EOS page, we'll discard it (because we
                    can't perform end trimming properly), and we'll always discard at
                    least 80 ms of audio (to allow decoder state to re-converge).
                    We could try to fill in the gap with PLC by looking at timestamps
                    in the non-EOS case, but that's complicated and error prone and we
                    can't rely on the timestamps being valid.*/
                    of.prev_packet_gp = -1;
                }
            }
            let mut op_count = of.op_count as usize;
            /*If we found at least one audio data packet, compute per-packet granule
            positions for them.*/
            if op_count > 0 {
                let mut diff: i64 = 0;
                let mut cur_packet_gp: i64 = 0;
                let mut cur_page_gp = of.op[op_count - 1].granulepos;
                let cur_page_eos = of.op[op_count - 1].e_o_s;
                let mut prev_packet_gp = of.prev_packet_gp;
                let link_pcm_start = of.links[cur_link as usize].pcm_start;
                if prev_packet_gp == -1 {
                    /*This is the first call after a raw seek.
                    Try to reconstruct prev_packet_gp from scratch.*/
                    debug_assert!(seekable);
                    if cur_page_eos != 0 {
                        /*If the first page we hit after our seek was the EOS page, and
                        we didn't start from data_offset or before, we don't have
                        enough information to do end-trimming.
                        Proceed to the next link, rather than risk playing back some
                        samples that shouldn't have been played.*/
                        of.op_count = 0;
                        if report_hole {
                            return OP_HOLE;
                        }
                        continue;
                    }
                    /*By default discard 80 ms of data after a seek, unless we seek
                    into the pre-skip region.*/
                    let mut cur_discard_count = 80 * 48;
                    cur_page_gp = of.op[op_count - 1].granulepos;
                    /*Try to initialize prev_packet_gp.
                    If the current page had packets but didn't have a granule
                    position, or the granule position it had was too small (both
                    illegal), just use the starting granule position for the link.*/
                    prev_packet_gp = link_pcm_start;
                    if cur_page_gp != -1 {
                        op_granpos_add(&mut prev_packet_gp, cur_page_gp, -total_duration);
                    }
                    if op_granpos_diff(&mut diff, prev_packet_gp, link_pcm_start) == 0 {
                        /*If we start at the beginning of the pre-skip region, or we're
                        at least 80 ms from the end of the pre-skip region, we discard
                        to the end of the pre-skip region.
                        Otherwise, we still use the 80 ms default, which will discard
                        past the end of the pre-skip region.*/
                        let pre_skip = of.links[cur_link as usize].head.pre_skip as i32;
                        if diff >= 0 && diff <= 0.max(pre_skip - 80 * 48) as i64 {
                            cur_discard_count = pre_skip - diff as i32;
                        }
                    }
                    of.cur_discard_count = cur_discard_count;
                }
                if cur_page_gp == -1 {
                    /*This page had completed packets but didn't have a valid granule
                    position.
                    This is illegal, but we'll try to handle it by continuing to count
                    forwards from the previous page.*/
                    if op_granpos_add(&mut cur_page_gp, prev_packet_gp, total_duration) < 0 {
                        /*The timestamp for this page overflowed.*/
                        cur_page_gp = of.links[cur_link as usize].pcm_end;
                    }
                }
                /*If we hit the last page, handle end-trimming.*/
                let mut pi = 0;
                if cur_page_eos != 0
                    && op_granpos_diff(&mut diff, cur_page_gp, prev_packet_gp) == 0
                    && diff < total_duration as i64
                {
                    cur_packet_gp = prev_packet_gp;
                    while pi < op_count {
                        /*Check for overflow.*/
                        if diff < 0 && OP_INT64_MAX + diff < durations[pi] as i64 {
                            diff = durations[pi] as i64 + 1;
                        } else {
                            diff = durations[pi] as i64 - diff;
                        }
                        /*If we have samples to trim...*/
                        if diff > 0 {
                            /*If we trimmed the entire packet, stop (the spec says encoders
                            shouldn't do this, but we support it anyway).*/
                            if diff > durations[pi] as i64 {
                                break;
                            }
                            cur_packet_gp = cur_page_gp;
                            /*Move the EOS flag to this packet, if necessary, so we'll trim
                            the samples during decode.*/
                            of.op[pi].e_o_s = 1;
                        } else {
                            /*Update the granule position as normal.*/
                            op_granpos_add_self(&mut cur_packet_gp, durations[pi]);
                        }
                        of.op[pi].granulepos = cur_packet_gp;
                        op_granpos_diff(&mut diff, cur_page_gp, cur_packet_gp);
                        pi += 1;
                    }
                } else {
                    /*Propagate timestamps to earlier packets.
                    op_granpos_add(&prev_packet_gp,prev_packet_gp,total_duration)
                    should succeed and give prev_packet_gp==cur_page_gp.
                    But we don't bother to check that, as there isn't much we can do
                    if it's not true, and it actually will not be true on the first
                    page after a seek, if there was a continued packet.
                    The only thing we guarantee is that the start and end granule
                    positions of the packets are valid, and that they are monotonic
                    within a page.
                    They might be completely out of range for this link (we'll check
                    that elsewhere), or non-monotonic between pages.*/
                    if op_granpos_add(&mut prev_packet_gp, cur_page_gp, -total_duration) < 0 {
                        /*The starting timestamp for the first packet on this page
                        underflowed.
                        This is illegal, but we ignore it.*/
                        prev_packet_gp = 0;
                    }
                    while pi < op_count {
                        if op_granpos_add(&mut cur_packet_gp, cur_page_gp, -total_duration) < 0 {
                            /*The start timestamp for this packet underflowed.
                            This is illegal, but we ignore it.*/
                            cur_packet_gp = 0;
                        }
                        total_duration -= durations[pi];
                        debug_assert!(total_duration >= 0);
                        op_granpos_add_self(&mut cur_packet_gp, durations[pi]);
                        of.op[pi].granulepos = cur_packet_gp;
                        pi += 1;
                    }
                    debug_assert!(total_duration == 0);
                }
                of.prev_packet_gp = prev_packet_gp;
                of.prev_page_offset = page_offset;
                op_count = pi;
                of.op_count = pi as i32;
            }
            if report_hole {
                return OP_HOLE;
            }
            /*If end-trimming didn't trim all the packets, we're done.*/
            if op_count > 0 {
                return 0;
            }
        }
    }
}

/// Translation of `op_raw_seek()`.
pub(crate) fn op_raw_seek(of: &mut OggOpusFile<'_>, pos: i64) -> i32 {
    if of.ready_state < OP_OPENED {
        return OP_EINVAL;
    }
    /*Don't dump the decoder state if we can't seek.*/
    if of.seekable == 0 {
        return OP_ENOSEEK;
    }
    if pos < 0 || pos > of.end {
        return OP_EINVAL;
    }
    /*Clear out any buffered, decoded data.*/
    op_decode_clear(of);
    of.bytes_tracked = 0;
    of.samples_tracked = 0;
    let ret = op_seek_helper(of, pos);
    if ret < 0 {
        return OP_EREAD;
    }
    let mut ret = op_fetch_and_process_page(of, None, -1, true, true);
    /*If we hit EOF, op_fetch_and_process_page() leaves us uninitialized.
    Instead, jump to the end.*/
    if ret == OP_EOF {
        op_decode_clear(of);
        let cur_link = of.nlinks - 1;
        of.cur_link = cur_link;
        of.prev_packet_gp = of.links[cur_link as usize].pcm_end;
        of.cur_discard_count = 0;
        ret = 0;
    }
    ret
}

/*Convert a PCM offset relative to the start of the whole stream to a granule
position in an individual link.*/
fn op_get_granulepos(of: &OggOpusFile<'_>, mut pcm_offset: i64, li_out: &mut i32) -> i64 {
    debug_assert!(pcm_offset >= 0);
    let nlinks = of.nlinks;
    let links = &of.links;
    let mut li_lo = 0;
    let mut li_hi = nlinks;
    loop {
        let li = li_lo + ((li_hi - li_lo) >> 1);
        if links[li as usize].pcm_file_offset <= pcm_offset {
            li_lo = li;
        } else {
            li_hi = li;
        }
        if li_hi - li_lo <= 1 {
            break;
        }
    }
    let link = &links[li_lo as usize];
    pcm_offset -= link.pcm_file_offset;
    let mut pcm_start = link.pcm_start;
    let pre_skip = link.head.pre_skip as i32;
    let mut duration: i64 = 0;
    op_granpos_diff(&mut duration, link.pcm_end, pcm_start);
    duration -= pre_skip as i64;
    if pcm_offset >= duration {
        return -1;
    }
    pcm_offset += pre_skip as i64;
    if pcm_start > OP_INT64_MAX - pcm_offset {
        /*Adding this amount to the granule position would overflow the positive
        half of its 64-bit range.
        Since signed overflow is undefined in C, do it in a way the compiler
        isn't allowed to screw up.*/
        pcm_offset -= OP_INT64_MAX - pcm_start + 1;
        pcm_start = OP_INT64_MIN;
    }
    pcm_start += pcm_offset;
    *li_out = li_lo;
    pcm_start
}

/*A small helper to determine if an Ogg page contains data that continues onto
a subsequent page.*/
fn op_page_continues(og: &OggPage) -> bool {
    debug_assert!(og.header.len() >= 27);
    let nlacing = og.header[26] as usize;
    debug_assert!(og.header.len() >= 27 + nlacing);
    /*This also correctly handles the (unlikely) case of nlacing==0, because
    0!=255.*/
    og.header[(27 + nlacing).wrapping_sub(1)] == 255
}

/*A small helper to buffer the continued packet data from a page.*/
fn op_buffer_continued_data(of: &mut OggOpusFile<'_>, og: &OggPage) {
    let mut op = OggPacket::default();
    ogg_stream_pagein(&mut of.os, og);
    /*Drain any packets that did end on this page (and ignore holes).
    We only care about the continued packet data.*/
    while ogg_stream_packetout(&mut of.os, Some(&mut op)) != 0 {}
}

/*This controls how close the target has to be to use the current stream
position to subdivide the initial range.
Two minutes seems to be a good default.*/
const OP_CUR_TIME_THRESH: i64 = 120 * 48 * 1000;

/*Note: The OP_SMALL_FOOTPRINT #define doesn't (currently) save much code size,
but it's meant to serve as documentation for portions of the seeking
algorithm that are purely optional, to aid others learning from/porting this
code to other contexts.*/
/*#define OP_SMALL_FOOTPRINT (1)*/

/*Search within link _li for the page with the highest granule position
preceding (or equal to) _target_gp.
There is a danger here: missing pages or incorrect frame number information
in the bitstream could make our task impossible.
Account for that (and report it as an error condition).*/
fn op_pcm_seek_page(of: &mut OggOpusFile<'_>, mut target_gp: i64, li: i32) -> i32 {
    let mut og = OggPage::default();
    let mut pcm_pre_skip: i64 = 0;
    let mut diff: i64 = 0;
    of.bytes_tracked = 0;
    of.samples_tracked = 0;
    let link = &of.links[li as usize];
    let link_end_offset = link.end_offset;
    let mut pcm_start = link.pcm_start;
    let mut best_gp = pcm_start;
    let mut pcm_end = link.pcm_end;
    let serialno = link.serialno;
    let mut begin = link.data_offset;
    let mut best = begin;
    let mut best_start = begin;
    let mut page_offset: i64 = -1;
    let mut buffering = false;
    let mut end: i64;
    let mut boundary: i64;
    /*We discard the first 80 ms of data after a seek, so seek back that much
    farther.
    If we can't, simply seek to the beginning of the link.*/
    if op_granpos_add_self(&mut target_gp, -80 * 48) < 0 || op_granpos_cmp(target_gp, pcm_start) < 0
    {
        target_gp = pcm_start;
    }
    /*Special case seeking to the start of the link.*/
    let pre_skip = link.head.pre_skip as i32;
    op_granpos_add(&mut pcm_pre_skip, pcm_start, pre_skip);
    if op_granpos_cmp(target_gp, pcm_pre_skip) < 0 {
        end = begin;
        boundary = begin;
    } else {
        end = link_end_offset;
        boundary = end;
        /*If we were decoding from this link, we can narrow the range a bit.*/
        if li == of.cur_link && of.ready_state >= OP_INITSET {
            let op_count = of.op_count as usize;
            /*The offset can be out of range if we were reading through the stream
            and encountered a page with the granule position for another link
            outside of the data range identified during link enumeration when we
            were opening the file.
            We will just ignore the current position in that case.
            The only way the offset can be valid _and_ we can fail the granule
            position checks below is if someone changed the contents of the last
            page since we read it.
            We'd be within our rights to just return OP_EBADLINK, but instead we'll
            simply ignore the current position in that case, too.*/
            let offset = of.offset;
            if op_count > 0 && begin <= offset && offset <= end {
                /*Make sure the timestamp is valid.
                The granule position might be -1 if we collected the packets from a
                page without a granule position after reporting a hole.*/
                let gp = of.op[op_count - 1].granulepos;
                if gp != -1 && op_granpos_cmp(pcm_start, gp) < 0 && op_granpos_cmp(pcm_end, gp) > 0
                {
                    op_granpos_diff(&mut diff, gp, target_gp);
                    /*We only actually use the current time if either
                    a) We can cut off at least half the range, or
                    b) We're seeking sufficiently close to the current position that
                    it's likely to be informative.
                    Otherwise it appears using the whole link range to estimate the
                    first seek location gives better results, on average.*/
                    if diff < 0 {
                        if offset - begin >= (end - begin) >> 1 || diff > -OP_CUR_TIME_THRESH {
                            begin = offset;
                            best = offset;
                            pcm_start = gp;
                            best_gp = gp;
                            /*If we have buffered data from a continued packet, remember the
                            offset of the previous page's start, so that if we do wind up
                            having to seek back here later, we can prime the stream with
                            the continued packet data.
                            With no continued packet, we remember the end of the page.*/
                            best_start = if ogg_stream_body_returned(&of.os)
                                < ogg_stream_body_fill(&of.os)
                            {
                                of.prev_page_offset
                            } else {
                                best
                            };
                            /*If there's completed packets and data in the stream state,
                            prev_page_offset should always be set.*/
                            debug_assert!(best_start >= 0);
                            /*Buffer any continued packet data starting from here.*/
                            buffering = true;
                        }
                    } else {
                        let mut prev_page_gp: i64 = 0;
                        /*We might get lucky and already have the packet with the target
                        buffered.
                        Worth checking.
                        For very small files (with all of the data in a single page,
                        generally 1 second or less), we can loop them continuously
                        without seeking at all.*/
                        let op0 = of.op[0];
                        if op_granpos_add(
                            &mut prev_page_gp,
                            op0.granulepos,
                            -op_get_packet_duration(of.os.packet_data(&op0), op0.bytes as i32),
                        ) < 0
                        {
                            /*We validate/sanitize the per-packet timestamps, so the only way
                            we should fail to calculate a granule position for the
                            previous page is if the first page with completed packets in
                            the stream is also the last, and end-trimming causes the
                            apparent granule position preceding the first sample in the
                            first packet to underflow.
                            The starting PCM offset is then 0 by spec mandate (see also:
                            op_find_initial_pcm_offset()).*/
                            debug_assert!(op0.e_o_s != 0);
                            prev_page_gp = 0;
                        }
                        if op_granpos_cmp(prev_page_gp, target_gp) <= 0 {
                            /*Don't call op_decode_clear(), because it will dump our
                            packets.*/
                            of.op_pos = 0;
                            of.od_buffer_size = 0;
                            of.prev_packet_gp = prev_page_gp;
                            /*_of->prev_page_offset already points to the right place.*/
                            of.ready_state = OP_STREAMSET;
                            return op_make_decode_ready(of);
                        }
                        /*No such luck.
                        Check if we can cut off at least half the range, though.*/
                        if offset - begin <= (end - begin) >> 1 || diff < OP_CUR_TIME_THRESH {
                            /*We really want the page start here, but this will do.*/
                            end = offset;
                            boundary = offset;
                            pcm_end = gp;
                        }
                    }
                }
            }
        }
    }
    /*This code was originally based on the "new search algorithm by HB (Nicholas
    Vinen)" from libvorbisfile.
    It has been modified substantially since.*/
    op_decode_clear(of);
    if !buffering {
        ogg_stream_reset_serialno(&mut of.os, serialno as i32);
    }
    of.cur_link = li;
    of.ready_state = OP_STREAMSET;
    /*Initialize the interval size history.*/
    let mut d0 = end - begin;
    let mut d1 = d0;
    let mut d2 = d0;
    let mut force_bisect = false;
    while begin < end {
        let mut bisect: i64;
        if end - begin < OP_CHUNK_SIZE as i64 {
            bisect = begin;
        } else {
            /*Update the interval size history.*/
            d0 = d1 >> 1;
            d1 = d2 >> 1;
            d2 = (end - begin) >> 1;
            if force_bisect {
                bisect = begin + ((end - begin) >> 1);
            } else {
                let mut diff2: i64 = 0;
                op_granpos_diff(&mut diff, target_gp, pcm_start);
                op_granpos_diff(&mut diff2, pcm_end, pcm_start);
                /*Take a (pretty decent) guess.*/
                bisect = begin + op_rescale64(diff, diff2, end - begin) - OP_CHUNK_SIZE as i64;
            }
            if bisect - (OP_CHUNK_SIZE as i64) < begin {
                bisect = begin;
            }
            force_bisect = false;
        }
        if bisect != of.offset {
            /*Discard any buffered continued packet data.*/
            if buffering {
                ogg_stream_reset(&mut of.os);
            }
            buffering = false;
            page_offset = -1;
            let ret = op_seek_helper(of, bisect);
            if ret < 0 {
                return ret;
            }
        }
        let mut chunk_size = OP_CHUNK_SIZE;
        let mut next_boundary = boundary;
        /*Now scan forward and figure out where we landed.
        In the ideal case, we will see a page with a granule position at or
        before our target, followed by a page with a granule position after our
        target (or the end of the search interval).
        Then we can just drop out and will have all of the data we need with no
        additional seeking.
        If we landed too far before, or after, we'll break out and do another
        bisection.*/
        while begin < end {
            page_offset = op_get_next_page(of, &mut og, boundary);
            if page_offset < 0 {
                if page_offset < OP_FALSE as i64 {
                    return page_offset as i32;
                }
                /*There are no more pages in our interval from our stream with a valid
                timestamp that start at position bisect or later.*/
                /*If we scanned the whole interval, we're done.*/
                if bisect <= begin + 1 {
                    end = begin;
                } else {
                    /*Otherwise, back up one chunk.
                    First, discard any data from a continued packet.*/
                    if buffering {
                        ogg_stream_reset(&mut of.os);
                    }
                    buffering = false;
                    bisect = (bisect - chunk_size as i64).max(begin);
                    let ret = op_seek_helper(of, bisect);
                    if ret < 0 {
                        return ret;
                    }
                    /*Bump up the chunk size.*/
                    chunk_size = (2 * chunk_size).min(OP_CHUNK_SIZE_MAX);
                    /*If we did find a page from another stream or without a timestamp,
                    don't read past it.*/
                    boundary = next_boundary;
                }
            } else {
                /*Save the offset of the first page we found after the seek, regardless
                of the stream it came from or whether or not it has a timestamp.*/
                next_boundary = page_offset.min(next_boundary);
                if serialno != ogg_page_serialno(&og) as u32 {
                    continue;
                }
                let has_packets = ogg_page_packets(&og) > 0;
                /*Force the gp to -1 (as it should be per spec) if no packets end on
                this page.
                Otherwise we might get confused when we try to pull out a packet
                with that timestamp and can't find it.*/
                let gp = if has_packets {
                    ogg_page_granulepos(&og)
                } else {
                    -1
                };
                if gp == -1 {
                    if buffering {
                        if !has_packets {
                            ogg_stream_pagein(&mut of.os, &og);
                        } else {
                            /*If packets did end on this page, but we still didn't have a
                            valid granule position (in violation of the spec!), stop
                            buffering continued packet data.
                            Otherwise we might continue past the packet we actually
                            wanted.*/
                            ogg_stream_reset(&mut of.os);
                            buffering = false;
                        }
                    }
                    continue;
                }
                if op_granpos_cmp(gp, target_gp) < 0 {
                    /*We found a page that ends before our target.
                    Advance to the raw offset of the next page.*/
                    begin = of.offset;
                    if op_granpos_cmp(pcm_start, gp) > 0 || op_granpos_cmp(pcm_end, gp) < 0 {
                        /*Don't let pcm_start get out of range!
                        That could happen with an invalid timestamp.*/
                        break;
                    }
                    /*Save the byte offset of the end of the page with this granule
                    position.*/
                    best = begin;
                    best_start = begin;
                    /*Buffer any data from a continued packet, if necessary.
                    This avoids the need to seek back here if the next timestamp we
                    encounter while scanning forward lies after our target.*/
                    if buffering {
                        ogg_stream_reset(&mut of.os);
                    }
                    if op_page_continues(&og) {
                        op_buffer_continued_data(of, &og);
                        /*If we have a continued packet, remember the offset of this
                        page's start, so that if we do wind up having to seek back here
                        later, we can prime the stream with the continued packet data.
                        With no continued packet, we remember the end of the page.*/
                        best_start = page_offset;
                    }
                    /*Then force buffering on, so that if a packet starts (but does not
                    end) on the next page, we still avoid the extra seek back.*/
                    buffering = true;
                    best_gp = gp;
                    pcm_start = gp;
                    op_granpos_diff(&mut diff, target_gp, pcm_start);
                    /*If we're more than a second away from our target, break out and
                    do another bisection.*/
                    if diff > 48000 {
                        break;
                    }
                    /*Otherwise, keep scanning forward (do NOT use begin+1).*/
                    bisect = begin;
                } else {
                    /*We found a page that ends after our target.*/
                    /*If we scanned the whole interval before we found it, we're done.*/
                    if bisect <= begin + 1 {
                        end = begin;
                    } else {
                        end = bisect;
                        /*In later iterations, don't read past the first page we found.*/
                        boundary = next_boundary;
                        /*If we're not making much progress shrinking the interval size,
                        start forcing straight bisection to limit the worst case.*/
                        force_bisect = end - begin > d0 * 2;
                        /*Don't let pcm_end get out of range!
                        That could happen with an invalid timestamp.*/
                        if op_granpos_cmp(pcm_end, gp) > 0 && op_granpos_cmp(pcm_start, gp) <= 0 {
                            pcm_end = gp;
                        }
                        break;
                    }
                }
            }
        }
    }
    /*Found our page.*/
    debug_assert!(op_granpos_cmp(best_gp, pcm_start) >= 0);
    /*Seek, if necessary.
    If we were buffering data from a continued packet, we should be able to
    continue to scan forward to get the rest of the data (even if
    page_offset==-1).
    Otherwise, we need to seek back to best_start.*/
    if !buffering {
        if best_start != page_offset {
            page_offset = -1;
            let ret = op_seek_helper(of, best_start);
            if ret < 0 {
                return ret;
            }
        }
        if best_start < best {
            /*Retrieve the page at best_start, if we do not already have it.*/
            if page_offset < 0 {
                page_offset = op_get_next_page(of, &mut og, link_end_offset);
                if page_offset < OP_FALSE as i64 {
                    return page_offset as i32;
                }
                if page_offset != best_start {
                    return OP_EBADLINK;
                }
            }
            op_buffer_continued_data(of, &og);
            page_offset = -1;
        }
    }
    /*Update prev_packet_gp to allow per-packet granule position assignment.*/
    of.prev_packet_gp = best_gp;
    of.prev_page_offset = best_start;
    let ret = op_fetch_and_process_page(
        of,
        if page_offset < 0 { None } else { Some(og) },
        page_offset,
        false,
        true,
    );
    if ret < 0 {
        return OP_EBADLINK;
    }
    /*Verify result.*/
    if op_granpos_cmp(of.prev_packet_gp, target_gp) > 0 {
        return OP_EBADLINK;
    }
    /*Our caller will set cur_discard_count to handle pre-roll.*/
    0
}

/// Translation of `op_pcm_seek()`.
pub(crate) fn op_pcm_seek(of: &mut OggOpusFile<'_>, mut pcm_offset: i64) -> i32 {
    let mut diff: i64 = 0;
    let mut li: i32 = 0;
    if of.ready_state < OP_OPENED {
        return OP_EINVAL;
    }
    if of.seekable == 0 {
        return OP_ENOSEEK;
    }
    if pcm_offset < 0 {
        return OP_EINVAL;
    }
    let target_gp = op_get_granulepos(of, pcm_offset, &mut li);
    if target_gp == -1 {
        return OP_EINVAL;
    }
    let pcm_start = of.links[li as usize].pcm_start;
    let link_pre_skip = of.links[li as usize].head.pre_skip as i64;
    op_granpos_diff(&mut pcm_offset, target_gp, pcm_start);
    /*For small (90 ms or less) forward seeks within the same link, just decode
    forward.
    This also optimizes the case of seeking to the current position.*/
    if li == of.cur_link && of.ready_state >= OP_INITSET {
        let mut gp = of.prev_packet_gp;
        if gp != -1 {
            let mut discard_count: i64 = 0;
            let nbuffered = (of.od_buffer_size - of.od_buffer_pos).max(0);
            op_granpos_add_self(&mut gp, -nbuffered);
            /*We do _not_ add cur_discard_count to gp.
            Otherwise the total amount to discard could grow without bound, and it
            would be better just to do a full seek.*/
            if op_granpos_diff(&mut discard_count, target_gp, gp) == 0 {
                /*We use a threshold of 90 ms instead of 80, since 80 ms is the
                _minimum_ we would have discarded after a full seek.
                Assuming 20 ms frames (the default), we'd discard 90 ms on average.*/
                if discard_count >= 0 && discard_count < 90 * 48 {
                    of.cur_discard_count = discard_count as i32;
                    return 0;
                }
            }
        }
    }
    let ret = op_pcm_seek_page(of, target_gp, li);
    if ret < 0 {
        return ret;
    }
    /*Now skip samples until we actually get to our target.*/
    /*Figure out where we should skip to.*/
    let skip = if pcm_offset <= link_pre_skip {
        0
    } else {
        (pcm_offset - 80 * 48).max(0)
    };
    debug_assert!(pcm_offset - skip >= 0);
    debug_assert!(pcm_offset - skip < (OP_INT32_MAX - 120 * 48) as i64);
    /*Skip packets until we find one with samples past our skip target.*/
    let mut prev_packet_gp;
    loop {
        let op_count = of.op_count;
        prev_packet_gp = of.prev_packet_gp;
        let mut op_pos = of.op_pos;
        while op_pos < op_count {
            let cur_packet_gp = of.op[op_pos as usize].granulepos;
            if op_granpos_diff(&mut diff, cur_packet_gp, pcm_start) == 0 && diff > skip {
                break;
            }
            prev_packet_gp = cur_packet_gp;
            op_pos += 1;
        }
        of.prev_packet_gp = prev_packet_gp;
        of.op_pos = op_pos;
        if op_pos < op_count {
            break;
        }
        /*We skipped all the packets on this page.
        Fetch another.*/
        let ret = op_fetch_and_process_page(of, None, -1, false, true);
        if ret < 0 {
            return OP_EBADLINK;
        }
    }
    /*We skipped too far, or couldn't get within 2 billion samples of the target.
    Either the timestamps were illegal or there was a hole in the data.*/
    if op_granpos_diff(&mut diff, prev_packet_gp, pcm_start) != 0
        || diff > skip
        || pcm_offset - diff >= OP_INT32_MAX as i64
    {
        return OP_EBADLINK;
    }
    /*TODO: If there are further holes/illegal timestamps, we still won't decode
    to the correct sample.
    However, at least op_pcm_tell() will report the correct value immediately
    after returning.*/
    of.cur_discard_count = (pcm_offset - diff) as i32;
    0
}

/*Allocate the decoder scratch buffer.
This is done lazily, since if the user provides large enough buffers, we'll
never need it.*/
fn op_init_buffer(of: &mut OggOpusFile<'_>) -> i32 {
    let nchannels_max = if of.seekable != 0 {
        let mut nchannels_max = 1;
        for link in of.links[..of.nlinks as usize].iter() {
            nchannels_max = nchannels_max.max(link.head.channel_count);
        }
        nchannels_max
    } else {
        OP_NCHANNELS_MAX as i32
    };
    match crate::internal::try_alloc::<f32>(nchannels_max as usize * 120 * 48) {
        Ok(buf) => of.od_buffer = buf,
        Err(_) => return OP_EFAULT,
    }
    0
}

/*Decode a single packet into the target buffer.*/
fn op_decode(
    of: &mut OggOpusFile<'_>,
    pcm: &mut [f32],
    op: &OggPacket,
    nsamples: i32,
    _nchannels: i32,
) -> i32 {
    /*First we try using the application-provided decode callback.*/
    /* (SDL_mixer sets none: OP_DEC_USE_DEFAULT) */
    /*If the application didn't want to handle decoding, do it ourselves.*/
    let Some(od) = of.od.as_mut() else {
        return OP_EFAULT;
    };
    let ret =
        opus_multistream_decode_float(od, of.os.packet_data(op), op.bytes as i32, pcm, nsamples, 0);
    debug_assert!(ret < 0 || ret == nsamples);
    if ret < 0 {
        return OP_EBADPACKET;
    }
    ret
}

/*Read more samples from the stream, using the same API as op_read() or
op_read_float().*/
fn op_read_native(
    of: &mut OggOpusFile<'_>,
    pcm: &mut [f32],
    buf_size: i32,
    li_out: Option<&mut i32>,
) -> i32 {
    if of.ready_state < OP_OPENED {
        return OP_EINVAL;
    }
    loop {
        if of.ready_state >= OP_INITSET {
            let nchannels = of.links[if of.seekable != 0 {
                of.cur_link as usize
            } else {
                0
            }]
            .head
            .channel_count;
            let mut od_buffer_pos = of.od_buffer_pos;
            let mut nsamples = of.od_buffer_size - od_buffer_pos;
            /*If we have buffered samples, return them.*/
            if nsamples > 0 {
                if nsamples * nchannels > buf_size {
                    nsamples = buf_size / nchannels;
                }
                /*Check nsamples again so we don't pass NULL to memcpy() if _buf_size
                is zero.
                That would technically be undefined behavior, even if the number of
                bytes to copy were zero.*/
                if nsamples > 0 {
                    let n = (nchannels * nsamples) as usize;
                    let o = (nchannels * od_buffer_pos) as usize;
                    pcm[..n].copy_from_slice(&of.od_buffer[o..o + n]);
                    od_buffer_pos += nsamples;
                    of.od_buffer_pos = od_buffer_pos;
                }
                if let Some(li) = li_out {
                    *li = of.cur_link;
                }
                return nsamples;
            }
            /*If we have buffered packets, decode one.*/
            let mut op_pos = of.op_pos;
            if op_pos < of.op_count {
                let pop = of.op[op_pos as usize];
                op_pos += 1;
                of.op_pos = op_pos;
                let mut cur_discard_count = of.cur_discard_count;
                let duration = op_get_packet_duration(of.os.packet_data(&pop), pop.bytes as i32);
                /*We don't buffer packets with an invalid TOC sequence.*/
                debug_assert!(duration > 0);
                let mut trimmed_duration = duration;
                /*Perform end-trimming.*/
                if pop.e_o_s != 0 {
                    let mut diff: i64 = 0;
                    if op_granpos_cmp(pop.granulepos, of.prev_packet_gp) <= 0 {
                        trimmed_duration = 0;
                    } else if op_granpos_diff(&mut diff, pop.granulepos, of.prev_packet_gp) == 0 {
                        trimmed_duration = diff.min(trimmed_duration as i64) as i32;
                    }
                }
                of.prev_packet_gp = pop.granulepos;
                if duration * nchannels > buf_size {
                    /*If the user's buffer is too small, decode into a scratch buffer.*/
                    if of.od_buffer.is_empty() {
                        let ret = op_init_buffer(of);
                        if ret < 0 {
                            return ret;
                        }
                    }
                    let mut buf = std::mem::take(&mut of.od_buffer);
                    let ret = op_decode(of, &mut buf, &pop, duration, nchannels);
                    of.od_buffer = buf;
                    if ret < 0 {
                        return ret;
                    }
                    /*Perform pre-skip/pre-roll.*/
                    od_buffer_pos = trimmed_duration.min(cur_discard_count);
                    cur_discard_count -= od_buffer_pos;
                    of.cur_discard_count = cur_discard_count;
                    of.od_buffer_pos = od_buffer_pos;
                    of.od_buffer_size = trimmed_duration;
                    /*Update bitrate tracking based on the actual samples we used from
                    what was decoded.*/
                    of.bytes_tracked += pop.bytes;
                    of.samples_tracked += (trimmed_duration - od_buffer_pos) as i64;
                } else {
                    /*Otherwise decode directly into the user's buffer.*/
                    let ret = op_decode(of, pcm, &pop, duration, nchannels);
                    if ret < 0 {
                        return ret;
                    }
                    if trimmed_duration > 0 {
                        /*Perform pre-skip/pre-roll.*/
                        od_buffer_pos = trimmed_duration.min(cur_discard_count);
                        cur_discard_count -= od_buffer_pos;
                        of.cur_discard_count = cur_discard_count;
                        trimmed_duration -= od_buffer_pos;
                        if trimmed_duration > 0 && od_buffer_pos > 0 {
                            let o = (od_buffer_pos * nchannels) as usize;
                            pcm.copy_within(o..o + (trimmed_duration * nchannels) as usize, 0);
                        }
                        /*Update bitrate tracking based on the actual samples we used from
                        what was decoded.*/
                        of.bytes_tracked += pop.bytes;
                        of.samples_tracked += trimmed_duration as i64;
                        if trimmed_duration > 0 {
                            if let Some(li) = li_out {
                                *li = of.cur_link;
                            }
                            return trimmed_duration;
                        }
                    }
                }
                /*Don't grab another page yet.
                This one might have more packets, or might have buffered data now.*/
                continue;
            }
        }
        /*Suck in another page.*/
        let ret = op_fetch_and_process_page(of, None, -1, true, false);
        if ret == OP_EOF {
            if let Some(li) = li_out {
                *li = of.cur_link;
            }
            return 0;
        }
        if ret < 0 {
            return ret;
        }
    }
}

/// Translation of `op_read_float()` (float build).
pub(crate) fn op_read_float(
    of: &mut OggOpusFile<'_>,
    pcm: &mut [f32],
    buf_size: i32,
    li: Option<&mut i32>,
) -> i32 {
    /* (state_channel_count, which C zeroes here, belongs to the int16 API) */
    op_read_native(of, pcm, buf_size, li)
}
