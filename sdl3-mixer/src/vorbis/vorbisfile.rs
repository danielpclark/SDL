// Rust translation of lib/vorbisfile.c (and the OggVorbis_File structure
// of include/vorbis/vorbisfile.h) from libvorbis 1.3.7
// (https://xiph.org/vorbis/, at the revision SDL_mixer's external/vorbis
// pins).
// THE OggVorbis SOURCE CODE IS (C) COPYRIGHT 1994-2015
// by the Xiph.Org Foundation https://xiph.org/
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! function: stdio-based convenience library for opening/seeking/decoding
//!
//! The stream is an [`IoStream`] used through SDL_mixer's
//! `VORBIS_IoCallbacks` (decoder_vorbis.c): `read_func` is `SDL_ReadIO()`
//! (C's `errno` test after an empty read is the stream's error status),
//! `seek_func` is `SDL_SeekIO()` (0, or -1 on failure), `tell_func` is
//! `SDL_TellIO()` and `close_func` does nothing; no initial data is ever
//! given (`initial` is NULL). `ov_clear()` is also dropping the
//! [`OggVorbisFile`]. Only what SDL_mixer's VORBIS decoder calls is
//! translated (see the module's documentation for what isn't); C's
//! `op_in` parameter of `_fetch_and_process_packet()`, always NULL here,
//! isn't kept.

/* A 'chained bitstream' is a Vorbis bitstream that contains more than
one logical bitstream arranged end to end (the only form of Ogg
multiplexing allowed in a Vorbis bitstream; grouping [parallel
multiplexing] is not allowed in Vorbis) */

/* A Vorbis file can be played beginning to end (streamed) without
worrying ahead of time about chaining (see decoder_example.c).  If
we have the whole file, however, and want random access
(seeking/scrubbing) or desire to know the total length/time of a
file, we need to account for the possibility of chaining. */

/* We can handle things a number of ways; we can determine the entire
bitstream structure right off the bat, or find pieces on demand.
This example determines and caches structure for the entire
bitstream, but builds a virtual decoder on the fly when moving
between links in the chain. */

/* There are also different ways to implement seeking.  Enough
information exists in an Ogg bitstream to seek to
sample-granularity positions in the output.  Or, one can seek by
picking some portion of the stream roughly in the desired area if
we only want coarse navigation through the stream. */

/*************************************************************************
 * Many, many internal helpers.  The intention is not to be confusing;
 * rampant duplication and monolithic function implementation would be
 * harder to understand anyway.  The high level functions are last.  Begin
 * grokking near the end of the file */

use sdl3::io::{IoStatus, IoStream, IoWhence};

use super::block::*;
use super::info::*;
use super::synthesis::*;
use super::*;
use crate::ogg::framing::*;

const SEEK_SET: i32 = 0;
const SEEK_CUR: i32 = 1;
const SEEK_END: i32 = 2;

pub(crate) const NOTOPEN: i32 = 0;
pub(crate) const PARTOPEN: i32 = 1;
pub(crate) const OPENED: i32 = 2;
pub(crate) const STREAMSET: i32 = 3;
pub(crate) const INITSET: i32 = 4;

/// Translation of `OggVorbis_File`. C's `datasource` is the stream
/// (`None` where C sets it to NULL); its `callbacks` are SDL_mixer's.
pub(crate) struct OggVorbisFile<'a> {
    datasource: Option<IoStream<'a>>, /* Pointer to a FILE *, etc. */
    pub seekable: i32,
    pub offset: i64,
    pub end: i64,
    pub oy: OggSyncState,

    /* If the FILE handle isn't seekable (eg, a pipe), only the current
    stream appears */
    pub links: i32,
    pub offsets: Vec<i64>,
    pub dataoffsets: Vec<i64>,
    pub serialnos: Vec<i64>,
    pub pcmlengths: Vec<i64>, /* overloaded to maintain binary
                              compatibility; x2 size, stores both
                              beginning and end values */
    pub vi: Vec<VorbisInfo>,
    pub vc: Vec<VorbisComment>,

    /* Decoding working state local storage */
    pub pcm_offset: i64,
    pub ready_state: i32,
    pub current_serialno: i64,
    pub current_link: i32,

    pub bittrack: f64,
    pub samptrack: f64,

    pub os: OggStreamState, /* take physical pages, weld into a logical
                            stream of packets */
    pub vd: VorbisDspState, /* central working state for the packet->PCM decoder */
    pub vb: VorbisBlock,    /* local working space for packet->PCM decode */
}

impl std::fmt::Debug for OggVorbisFile<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OggVorbisFile")
            .field("links", &self.links)
            .field("ready_state", &self.ready_state)
            .field("pcm_offset", &self.pcm_offset)
            .finish_non_exhaustive()
    }
}

impl<'a> OggVorbisFile<'a> {
    /// The `memset(vf,0,sizeof(*vf))` state.
    fn zeroed() -> Self {
        OggVorbisFile {
            datasource: None,
            seekable: 0,
            offset: 0,
            end: 0,
            oy: OggSyncState::default(),
            links: 0,
            offsets: Vec::new(),
            dataoffsets: Vec::new(),
            serialnos: Vec::new(),
            pcmlengths: Vec::new(),
            vi: Vec::new(),
            vc: Vec::new(),
            pcm_offset: 0,
            ready_state: NOTOPEN,
            current_serialno: 0,
            current_link: 0,
            bittrack: 0.0,
            samptrack: 0.0,
            os: OggStreamState::default(),
            vd: VorbisDspState::default(),
            vb: VorbisBlock::default(),
        }
    }

    /// `VORBIS_IoRead()` (`size` 1): the bytes read, and whether the
    /// stream failed (C's `errno`).
    fn read_func(&mut self, buf: Option<&mut [u8]>) -> (i64, bool) {
        let Some(io) = self.datasource.as_mut() else {
            return (0, false);
        };
        let Some(buf) = buf else {
            /* (SDL_ReadIO() refuses the NULL buffer of a failed ogg_sync_buffer()) */
            return (0, false);
        };
        let n = io.read(buf);
        (n as i64, n == 0 && io.status() == IoStatus::Error)
    }

    /// `VORBIS_IoSeek()`.
    fn seek_func(&mut self, offset: i64, whence: i32) -> i32 {
        let Some(io) = self.datasource.as_mut() else {
            return -1;
        };
        let whence = match whence {
            SEEK_SET => IoWhence::Set,
            SEEK_CUR => IoWhence::Cur,
            _ => IoWhence::End,
        };
        if io.seek(offset, whence).is_err() {
            -1
        } else {
            0
        }
    }

    /// `VORBIS_IoTell()`.
    fn tell_func(&mut self) -> i64 {
        match self.datasource.as_mut() {
            Some(io) => io.tell().unwrap_or(-1),
            None => -1,
        }
    }
}

/* read a little more data from the file/pipe into the ogg_sync framer
*/
const CHUNKSIZE: i64 = 65536; /* greater-than-page-size granularity seeking */
const READSIZE: i64 = 2048; /* a smaller read size is needed for low-rate streaming. */

/// Translation of `_get_data()`.
fn _get_data(vf: &mut OggVorbisFile<'_>) -> i64 {
    if vf.datasource.is_some() {
        let mut oy = std::mem::take(&mut vf.oy);
        let buffer = ogg_sync_buffer(&mut oy, READSIZE).map(|b| &mut b[..READSIZE as usize]);
        let (bytes, errno) = vf.read_func(buffer);
        if bytes > 0 {
            ogg_sync_wrote(&mut oy, bytes);
        }
        vf.oy = oy;
        if bytes == 0 && errno {
            return -1;
        }
        bytes
    } else {
        0
    }
}

/* save a tiny smidge of verbosity to make the code more readable */
/// Translation of `_seek_helper()`.
fn _seek_helper(vf: &mut OggVorbisFile<'_>, offset: i64) -> i32 {
    if vf.datasource.is_some() {
        /* only seek if the file position isn't already there */
        if vf.offset != offset {
            if vf.seek_func(offset, SEEK_SET) == -1 {
                return OV_EREAD;
            }
            vf.offset = offset;
            ogg_sync_reset(&mut vf.oy);
        }
    } else {
        /* shouldn't happen unless someone writes a broken callback */
        return OV_EFAULT;
    }
    0
}

/* The read/seek functions track absolute position within the stream */

/* from the head of the stream, get the next page.  boundary specifies
if the function is allowed to fetch more data from the stream (and
how much) or only use internally buffered data.

boundary: -1) unbounded search
           0) read no additional data; use cached only
           n) search for a new page beginning for n bytes

return:   <0) did not find a page (OV_FALSE, OV_EOF, OV_EREAD)
           n) found a page at absolute offset n */

/// Translation of `_get_next_page()`.
fn _get_next_page(vf: &mut OggVorbisFile<'_>, og: &mut OggPage, mut boundary: i64) -> i64 {
    if boundary > 0 {
        boundary += vf.offset;
    }
    loop {
        let more: i64;

        if boundary > 0 && vf.offset >= boundary {
            return OV_FALSE as i64;
        }
        more = ogg_sync_pageseek(&mut vf.oy, Some(&mut *og));

        if more < 0 {
            /* skipped n bytes */
            vf.offset -= more;
        } else if more == 0 {
            /* send more paramedics */
            if boundary == 0 {
                return OV_FALSE as i64;
            }
            {
                let ret = _get_data(vf);
                if ret == 0 {
                    return OV_EOF as i64;
                }
                if ret < 0 {
                    return OV_EREAD as i64;
                }
            }
        } else {
            /* got a page.  Return the offset at the page beginning,
            advance the internal offset past the page end */
            let ret = vf.offset;
            vf.offset += more;
            return ret;
        }
    }
}

/* find the latest page beginning before the passed in position. Much
dirtier than the above as Ogg doesn't have any backward search
linkage.  no 'readp' as it will certainly have to read. */
/* returns offset or OV_EREAD, OV_FAULT */
/// Translation of `_get_prev_page()`.
fn _get_prev_page(vf: &mut OggVorbisFile<'_>, mut begin: i64, og: &mut OggPage) -> i64 {
    let end = begin;
    let mut ret: i64;
    let mut offset: i64 = -1;

    while offset == -1 {
        begin -= CHUNKSIZE;
        if begin < 0 {
            begin = 0;
        }

        ret = _seek_helper(vf, begin) as i64;
        if ret != 0 {
            return ret;
        }

        while vf.offset < end {
            *og = OggPage::default();
            ret = _get_next_page(vf, og, end - vf.offset);
            if ret == OV_EREAD as i64 {
                return OV_EREAD as i64;
            }
            if ret < 0 {
                break;
            } else {
                offset = ret;
            }
        }
    }

    /* In a fully compliant, non-multiplexed stream, we'll still be
    holding the last page.  In multiplexed (or noncompliant streams),
    we will probably have to re-read the last page we saw */
    if og.header.is_empty() {
        ret = _seek_helper(vf, offset) as i64;
        if ret != 0 {
            return ret;
        }

        ret = _get_next_page(vf, og, CHUNKSIZE);
        if ret < 0 {
            /* this shouldn't be possible */
            return OV_EFAULT as i64;
        }
    }

    offset
}

/// Translation of `_add_serialno()`.
fn _add_serialno(og: &OggPage, serialno_list: &mut Vec<i64>, n: &mut i32) {
    let s = ogg_page_serialno(og) as i64;
    *n += 1;

    serialno_list.push(s);
}

/* returns nonzero if found */
/// Translation of `_lookup_serialno()`.
fn _lookup_serialno(s: i64, serialno_list: &[i64], n: i32) -> i32 {
    serialno_list
        .iter()
        .take(n.max(0) as usize)
        .any(|&x| x == s) as i32
}

/// Translation of `_lookup_page_serialno()`.
fn _lookup_page_serialno(og: &OggPage, serialno_list: &[i64], n: i32) -> i32 {
    let s = ogg_page_serialno(og) as i64;
    _lookup_serialno(s, serialno_list, n)
}

/* performs the same search as _get_prev_page, but prefers pages of
the specified serial number. If a page of the specified serialno is
spotted during the seek-back-and-read-forward, it will return the
info of last page of the matching serial number instead of the very
last page.  If no page of the specified serialno is seen, it will
return the info of last page and alter *serialno.  */
/// Translation of `_get_prev_page_serial()`.
fn _get_prev_page_serial(
    vf: &mut OggVorbisFile<'_>,
    mut begin: i64,
    serial_list: &[i64],
    serial_n: i32,
    serialno: &mut i32,
    granpos: &mut i64,
) -> i64 {
    let mut og = OggPage::default();
    let end = begin;
    let mut ret: i64;

    let mut prefoffset: i64 = -1;
    let mut offset: i64 = -1;
    let mut ret_serialno: i64 = -1;
    let mut ret_gran: i64 = -1;

    while offset == -1 {
        begin -= CHUNKSIZE;
        if begin < 0 {
            begin = 0;
        }

        ret = _seek_helper(vf, begin) as i64;
        if ret != 0 {
            return ret;
        }

        while vf.offset < end {
            ret = _get_next_page(vf, &mut og, end - vf.offset);
            if ret == OV_EREAD as i64 {
                return OV_EREAD as i64;
            }
            if ret < 0 {
                break;
            } else {
                ret_serialno = ogg_page_serialno(&og) as i64;
                ret_gran = ogg_page_granulepos(&og);
                offset = ret;

                if ret_serialno == *serialno as i64 {
                    prefoffset = ret;
                    *granpos = ret_gran;
                }

                if _lookup_serialno(ret_serialno, serial_list, serial_n) == 0 {
                    /* we fell off the end of the link, which means we seeked
                    back too far and shouldn't have been looking in that link
                    to begin with.  If we found the preferred serial number,
                    forget that we saw it. */
                    prefoffset = -1;
                }
            }
        }
        /*We started from the beginning of the stream and found nothing.
        This should be impossible unless the contents of the stream changed out
        from under us after we read from it.*/
        if begin == 0 && vf.offset < 0 {
            return OV_EBADLINK as i64;
        }
    }

    /* we're not interested in the page... just the serialno and granpos. */
    if prefoffset >= 0 {
        return prefoffset;
    }

    *serialno = ret_serialno as i32;
    *granpos = ret_gran;
    offset
}

/* uses the local ogg_stream storage in vf; this is important for
non-streaming input sources */
/// Translation of `_fetch_headers()`.
fn _fetch_headers(
    vf: &mut OggVorbisFile<'_>,
    vi: &mut VorbisInfo,
    vc: &mut VorbisComment,
    mut serialno_list: Option<(&mut Vec<i64>, &mut i32)>,
    og_ptr: Option<&mut OggPage>,
) -> i32 {
    let mut og = OggPage::default();
    let mut op = OggPacket::default();
    let mut i: i32;
    let mut ret: i32;
    let mut allbos = 0;

    let og_ptr: &mut OggPage = match og_ptr {
        Some(og_ptr) => og_ptr,
        None => {
            let llret = _get_next_page(vf, &mut og, CHUNKSIZE);
            if llret == OV_EREAD as i64 {
                return OV_EREAD;
            }
            if llret < 0 {
                return OV_ENOTVORBIS;
            }
            &mut og
        }
    };

    vorbis_info_init(vi);
    vorbis_comment_init(vc);
    vf.ready_state = OPENED;

    'bail_header: {
        /* extract the serialnos of all BOS pages + the first set of vorbis
        headers we see in the link */

        while ogg_page_bos(og_ptr) != 0 {
            if let Some((list, n)) = serialno_list.as_mut() {
                if _lookup_page_serialno(og_ptr, list, **n) != 0 {
                    /* a dupe serialnumber in an initial header packet set == invalid stream */
                    list.clear();
                    **n = 0;
                    ret = OV_EBADHEADER;
                    break 'bail_header;
                }

                _add_serialno(og_ptr, list, n);
            }

            if vf.ready_state < STREAMSET {
                /* we don't have a vorbis stream in this link yet, so begin
                prospective stream setup. We need a stream to get packets */
                ogg_stream_reset_serialno(&mut vf.os, ogg_page_serialno(og_ptr));
                ogg_stream_pagein(&mut vf.os, og_ptr);

                if ogg_stream_packetout(&mut vf.os, Some(&mut op)) > 0
                    && vorbis_synthesis_idheader(vf.os.packet_data(&op), op.b_o_s) != 0
                {
                    /* vorbis header; continue setup */
                    vf.ready_state = STREAMSET;
                    ret = vorbis_synthesis_headerin(vi, vc, vf.os.packet_data(&op), op.b_o_s);
                    if ret != 0 {
                        ret = OV_EBADHEADER;
                        break 'bail_header;
                    }
                }
            }

            /* get next page */
            {
                let llret = _get_next_page(vf, og_ptr, CHUNKSIZE);
                if llret == OV_EREAD as i64 {
                    ret = OV_EREAD;
                    break 'bail_header;
                }
                if llret < 0 {
                    ret = OV_ENOTVORBIS;
                    break 'bail_header;
                }

                /* if this page also belongs to our vorbis stream, submit it and break */
                if vf.ready_state == STREAMSET && vf.os.serialno == ogg_page_serialno(og_ptr) as i64
                {
                    ogg_stream_pagein(&mut vf.os, og_ptr);
                    break;
                }
            }
        }

        if vf.ready_state != STREAMSET {
            ret = OV_ENOTVORBIS;
            break 'bail_header;
        }

        {
            i = 0;
            while i < 2 {
                /* get a page loop */

                while i < 2 {
                    /* get a packet loop */

                    let result = ogg_stream_packetout(&mut vf.os, Some(&mut op));
                    if result == 0 {
                        break;
                    }
                    if result == -1 {
                        ret = OV_EBADHEADER;
                        break 'bail_header;
                    }

                    ret = vorbis_synthesis_headerin(vi, vc, vf.os.packet_data(&op), op.b_o_s);
                    if ret != 0 {
                        break 'bail_header;
                    }

                    i += 1;
                }

                while i < 2 {
                    if _get_next_page(vf, og_ptr, CHUNKSIZE) < 0 {
                        ret = OV_EBADHEADER;
                        break 'bail_header;
                    }

                    /* if this page belongs to the correct stream, go parse it */
                    if vf.os.serialno == ogg_page_serialno(og_ptr) as i64 {
                        ogg_stream_pagein(&mut vf.os, og_ptr);
                        break;
                    }

                    /* if we never see the final vorbis headers before the link
                    ends, abort */
                    if ogg_page_bos(og_ptr) != 0 {
                        if allbos != 0 {
                            ret = OV_EBADHEADER;
                            break 'bail_header;
                        } else {
                            allbos = 1;
                        }
                    }

                    /* otherwise, keep looking */
                }
            }

            return 0;
        }
    }

    /* bail_header: */
    vorbis_info_clear(vi);
    vorbis_comment_clear(vc);
    vf.ready_state = OPENED;

    ret
}

/* Starting from current cursor position, get initial PCM offset of
next page.  Consumes the page in the process without decoding
audio, however this is only called during stream parsing upon
seekable open. */
/// Translation of `_initial_pcmoffset()`.
fn _initial_pcmoffset(vf: &mut OggVorbisFile<'_>, vi: &VorbisInfo) -> i64 {
    let mut og = OggPage::default();
    let mut accumulated: i64 = 0;
    let mut lastblock: i64 = -1;
    let mut result: i32;
    let serialno = vf.os.serialno as i32;

    loop {
        let mut op = OggPacket::default();
        if _get_next_page(vf, &mut og, -1) < 0 {
            break; /* should not be possible unless the file is truncated/mangled */
        }

        if ogg_page_bos(&og) != 0 {
            break;
        }
        if ogg_page_serialno(&og) != serialno {
            continue;
        }

        /* count blocksizes of all frames in the page */
        ogg_stream_pagein(&mut vf.os, &og);
        loop {
            result = ogg_stream_packetout(&mut vf.os, Some(&mut op));
            if result == 0 {
                break;
            }
            if result > 0 {
                /* ignore holes */
                let thisblock = vorbis_packet_blocksize(vi, vf.os.packet_data(&op));
                if thisblock >= 0 {
                    if lastblock != -1 {
                        accumulated += (lastblock + thisblock) >> 2;
                    }
                    lastblock = thisblock;
                }
            }
        }

        if ogg_page_granulepos(&og) != -1 {
            /* pcm offset of last packet on the first audio page */
            accumulated = ogg_page_granulepos(&og).wrapping_sub(accumulated);
            break;
        }
    }

    /* less than zero?  Either a corrupt file or a stream with samples
    trimmed off the beginning, a normal occurrence; in both cases set
    the offset to zero */
    if accumulated < 0 {
        accumulated = 0;
    }

    accumulated
}

/* finds each bitstream link one at a time using a bisection search
(has to begin by knowing the offset of the lb's initial page).
Recurses for each link so it can alloc the link storage after
finding them all, then unroll and fill the cache at the same time */
/// Translation of `_bisect_forward_serialno()`. The links' arrays are
/// sized (C's `_ogg_malloc()`/`_ogg_realloc()`) when the last one is
/// found; C's `vi`/`vc` reallocation keeps the first link's info.
fn _bisect_forward_serialno(
    vf: &mut OggVorbisFile<'_>,
    begin: i64,
    mut searched: i64,
    end: i64,
    mut endgran: i64,
    mut endserial: i32,
    currentno_list: &[i64],
    currentnos: i32,
    m: i64,
) -> i32 {
    let pcmoffset: i64;
    let dataoffset: i64;
    let mut endsearched = end;
    let mut next = end;
    let mut searchgran: i64 = -1;
    let mut og = OggPage::default();
    let mut ret: i64;
    let mut last: i64;
    let mut serialno = vf.os.serialno as i32;

    /* invariants:
    we have the headers and serialnos for the link beginning at 'begin'
    we have the offset and granpos of the last page in the file (potentially
      not a page we care about)
    */

    /* Is the last page in our list of current serialnumbers? */
    if _lookup_serialno(endserial as i64, currentno_list, currentnos) != 0 {
        /* last page is in the starting serialno list, so we've bisected
        down to (or just started with) a single link.  Now we need to
        find the last vorbis page belonging to the first vorbis stream
        for this link. */
        searched = end;
        while endserial != serialno {
            endserial = serialno;
            searched = _get_prev_page_serial(
                vf,
                searched,
                currentno_list,
                currentnos,
                &mut endserial,
                &mut endgran,
            );
        }

        vf.links = (m + 1) as i32;
        let links = vf.links as usize;

        vf.offsets = vec![0; links + 1];
        vf.vi.resize_with(links, VorbisInfo::default);
        vf.vc.resize_with(links, VorbisComment::default);
        vf.serialnos = vec![0; links];
        vf.dataoffsets = vec![0; links];
        vf.pcmlengths = vec![0; links * 2];

        vf.offsets[(m + 1) as usize] = end;
        vf.offsets[m as usize] = begin;
        vf.pcmlengths[(m * 2 + 1) as usize] = if endgran < 0 { 0 } else { endgran };
    } else {
        /* last page is not in the starting stream's serial number list,
        so we have multiple links.  Find where the stream that begins
        our bisection ends. */

        let mut next_serialno_list: Vec<i64> = Vec::new();
        let mut next_serialnos: i32 = 0;
        let mut vi = VorbisInfo::default();
        let mut vc = VorbisComment::default();
        let mut testserial = serialno.wrapping_add(1);

        /* the below guards against garbage seperating the last and
        first pages of two links. */
        while searched < endsearched {
            let bisect: i64;

            if endsearched - searched < CHUNKSIZE {
                bisect = searched;
            } else {
                bisect = (searched + endsearched) / 2;
            }

            ret = _seek_helper(vf, bisect) as i64;
            if ret != 0 {
                return ret as i32;
            }

            last = _get_next_page(vf, &mut og, -1);
            if last == OV_EREAD as i64 {
                return OV_EREAD;
            }
            if last < 0 || _lookup_page_serialno(&og, currentno_list, currentnos) == 0 {
                endsearched = bisect;
                if last >= 0 {
                    next = last;
                }
            } else {
                searched = vf.offset;
            }
        }

        /* Bisection point found */
        /* for the time being, fetch end PCM offset the simple way */
        searched = next;
        while testserial != serialno {
            testserial = serialno;
            searched = _get_prev_page_serial(
                vf,
                searched,
                currentno_list,
                currentnos,
                &mut testserial,
                &mut searchgran,
            );
        }

        ret = _seek_helper(vf, next) as i64;
        if ret != 0 {
            return ret as i32;
        }

        ret = _fetch_headers(
            vf,
            &mut vi,
            &mut vc,
            Some((&mut next_serialno_list, &mut next_serialnos)),
            None,
        ) as i64;
        if ret != 0 {
            return ret as i32;
        }
        serialno = vf.os.serialno as i32;
        dataoffset = vf.offset;

        /* this will consume a page, however the next bisection always
        starts with a raw seek */
        pcmoffset = _initial_pcmoffset(vf, &vi);

        ret = _bisect_forward_serialno(
            vf,
            next,
            vf.offset,
            end,
            endgran,
            endserial,
            &next_serialno_list,
            next_serialnos,
            m + 1,
        ) as i64;
        if ret != 0 {
            return ret as i32;
        }

        let m = m as usize;
        vf.offsets[m + 1] = next;
        vf.serialnos[m + 1] = serialno as i64;
        vf.dataoffsets[m + 1] = dataoffset;

        vf.vi[m + 1] = vi;
        vf.vc[m + 1] = vc;

        vf.pcmlengths[m * 2 + 1] = searchgran;
        vf.pcmlengths[m * 2 + 2] = pcmoffset;
        vf.pcmlengths[m * 2 + 3] = vf.pcmlengths[m * 2 + 3].wrapping_sub(pcmoffset);
        if vf.pcmlengths[m * 2 + 3] < 0 {
            vf.pcmlengths[m * 2 + 3] = 0;
        }
    }
    0
}

/// Translation of `_make_decode_ready()`.
fn _make_decode_ready(vf: &mut OggVorbisFile<'_>) -> i32 {
    if vf.ready_state > STREAMSET {
        return 0;
    }
    if vf.ready_state < STREAMSET {
        return OV_EFAULT;
    }
    let link = if vf.seekable != 0 {
        vf.current_link as usize
    } else {
        0
    };
    if vorbis_synthesis_init(&mut vf.vd, &mut vf.vi[link], link) != 0 {
        return OV_EBADLINK;
    }
    vorbis_block_init(&mut vf.vb);
    vf.ready_state = INITSET;
    vf.bittrack = 0.0;
    vf.samptrack = 0.0;
    0
}

/// Translation of `_open_seekable2()`.
fn _open_seekable2(vf: &mut OggVorbisFile<'_>) -> i32 {
    let dataoffset = vf.dataoffsets[0];
    let end: i64;
    let mut endgran: i64 = -1;
    let mut endserial = vf.os.serialno as i32;
    let serialno = vf.os.serialno as i32;

    /* we're partially open and have a first link header state in
    storage in vf */

    /* fetch initial PCM offset */
    let vi0 = std::mem::take(&mut vf.vi[0]);
    let pcmoffset = _initial_pcmoffset(vf, &vi0);
    vf.vi[0] = vi0;

    /* we can seek, so set out learning all about this file */
    vf.seek_func(0, SEEK_END);
    vf.end = vf.tell_func();
    vf.offset = vf.end;

    /* If seek_func is implemented, tell_func must also be implemented */
    if vf.end == -1 {
        return OV_EINVAL;
    }

    /* Get the offset of the last page of the physical bitstream, or, if
    we're lucky the last vorbis page of this link as most OggVorbis
    files will contain a single logical bitstream */
    let list = vf.serialnos[2..].to_vec();
    let n = vf.serialnos[1] as i32;
    end = _get_prev_page_serial(vf, vf.end, &list, n, &mut endserial, &mut endgran);
    if end < 0 {
        return end as i32;
    }

    /* now determine bitstream structure recursively */
    if _bisect_forward_serialno(vf, 0, dataoffset, end, endgran, endserial, &list, n, 0) < 0 {
        return OV_EREAD;
    }

    vf.offsets[0] = 0;
    vf.serialnos[0] = serialno as i64;
    vf.dataoffsets[0] = dataoffset;
    vf.pcmlengths[0] = pcmoffset;
    vf.pcmlengths[1] = vf.pcmlengths[1].wrapping_sub(pcmoffset);
    if vf.pcmlengths[1] < 0 {
        vf.pcmlengths[1] = 0;
    }

    ov_raw_seek(vf, dataoffset)
}

/* clear out the current logical bitstream decoder */
/// Translation of `_decode_clear()`.
fn _decode_clear(vf: &mut OggVorbisFile<'_>) {
    vorbis_dsp_clear(&mut vf.vd);
    vorbis_block_clear(&mut vf.vb);
    vf.ready_state = OPENED;
}

/* fetch and process a packet.  Handles the case where we're at a
bitstream boundary and dumps the decoding machine.  If the decoding
machine is unloaded, it loads it.  It also keeps pcm_offset up to
date (seek and read both use this.  seek uses a special hack with
readp).

return: <0) error, OV_HOLE (lost packet) or OV_EOF
         0) need more data (only if readp==0)
         1) got a packet
*/

/// Translation of `_fetch_and_process_packet()`.
fn _fetch_and_process_packet(vf: &mut OggVorbisFile<'_>, readp: i32, spanp: i32) -> i32 {
    let mut og = OggPage::default();

    /* handle one packet.  Try to fetch it from current stream state */
    /* extract packets from page */
    loop {
        if vf.ready_state == STREAMSET {
            let ret = _make_decode_ready(vf);
            if ret < 0 {
                return ret;
            }
        }

        /* process a packet if we can. */

        if vf.ready_state == INITSET {
            let hs = vorbis_synthesis_halfrate_p(&vf.vi[0]);

            loop {
                let mut op = OggPacket::default();
                let result = ogg_stream_packetout(&mut vf.os, Some(&mut op));
                let mut granulepos: i64;

                if result == -1 {
                    return OV_HOLE; /* hole in the data. */
                }
                if result > 0 {
                    /* got a packet.  process it */
                    granulepos = op.granulepos;
                    let vdi = vf.vd.vi.unwrap();
                    if vorbis_synthesis(
                        &mut vf.vb,
                        &mut vf.vd,
                        &vf.vi[vdi],
                        vf.os.packet_data(&op),
                        op.granulepos,
                        op.packetno,
                        op.e_o_s,
                    ) == 0
                    {
                        /* lazy check for lazy
                        header handling.  The
                        header packets aren't
                        audio, so if/when we
                        submit them,
                        vorbis_synthesis will
                        reject them */

                        /* suck in the synthesis data and track bitrate */
                        {
                            let oldsamples = vorbis_synthesis_pcmout(&vf.vd, None);
                            /* for proper use of libvorbis within libvorbisfile,
                            oldsamples will always be zero. */
                            if oldsamples != 0 {
                                return OV_EFAULT;
                            }

                            vorbis_synthesis_blockin(&mut vf.vd, &vf.vi[vdi], &vf.vb);
                            vf.samptrack += (vorbis_synthesis_pcmout(&vf.vd, None) << hs) as f64;
                            vf.bittrack += (op.bytes * 8) as f64;
                        }

                        /* update the pcm offset. */
                        if granulepos != -1 && op.e_o_s == 0 {
                            let link = if vf.seekable != 0 { vf.current_link } else { 0 };
                            let samples: i32;

                            /* this packet has a pcm_offset on it (the last packet
                            completed on a page carries the offset) After processing
                            (above), we know the pcm position of the *last* sample
                            ready to be returned. Find the offset of the *first*

                            As an aside, this trick is inaccurate if we begin
                            reading anew right at the last page; the end-of-stream
                            granulepos declares the last frame in the stream, and the
                            last packet of the last page may be a partial frame.
                            So, we need a previous granulepos from an in-sequence page
                            to have a reference point.  Thus the !op_ptr->e_o_s clause
                            above */

                            if vf.seekable != 0 && link > 0 {
                                granulepos =
                                    granulepos.wrapping_sub(vf.pcmlengths[link as usize * 2]);
                            }
                            if granulepos < 0 {
                                granulepos = 0; /* actually, this
                                                shouldn't be possible
                                                here unless the stream
                                                is very broken */
                            }

                            samples = vorbis_synthesis_pcmout(&vf.vd, None) << hs;

                            granulepos -= samples as i64;
                            for i in 0..link as usize {
                                granulepos = granulepos.wrapping_add(vf.pcmlengths[i * 2 + 1]);
                            }
                            vf.pcm_offset = granulepos;
                        }
                        return 1;
                    }
                } else {
                    break;
                }
            }
        }

        if vf.ready_state >= OPENED {
            let mut ret: i64;

            loop {
                /* the loop is not strictly necessary, but there's no sense in
                doing the extra checks of the larger loop for the common
                case in a multiplexed bistream where the page is simply
                part of a different logical bitstream; keep reading until
                we get one with the correct serialno */

                if readp == 0 {
                    return 0;
                }
                ret = _get_next_page(vf, &mut og, -1);
                if ret < 0 {
                    return OV_EOF; /* eof. leave unitialized */
                }

                /* bitrate tracking; add the header's bytes here, the body bytes
                are done by packet above */
                vf.bittrack += (og.header.len() * 8) as f64;

                if vf.ready_state == INITSET && vf.current_serialno != ogg_page_serialno(&og) as i64
                {
                    /* two possibilities:
                    1) our decoding just traversed a bitstream boundary
                    2) another stream is multiplexed into this logical section */

                    if ogg_page_bos(&og) != 0 {
                        /* boundary case */
                        if spanp == 0 {
                            return OV_EOF;
                        }

                        _decode_clear(vf);

                        if vf.seekable == 0 {
                            vorbis_info_clear(&mut vf.vi[0]);
                            vorbis_comment_clear(&mut vf.vc[0]);
                        }
                        break;
                    } else {
                        continue; /* possibility #2 */
                    }
                }

                break;
            }
        }

        /* Do we need to load a new machine before submitting the page? */
        /* This is different in the seekable and non-seekable cases.

        In the seekable case, we already have all the header
        information loaded and cached; we just initialize the machine
        with it and continue on our merry way.

        In the non-seekable (streaming) case, we'll only be at a
        boundary if we just left the previous logical bitstream and
        we're now nominally at the header of the next bitstream
        */

        if vf.ready_state != INITSET && vf.ready_state < STREAMSET {
            if vf.seekable != 0 {
                let serialno = ogg_page_serialno(&og) as i64;

                /* match the serialno to bitstream section.  We use this rather than
                offset positions to avoid problems near logical bitstream
                boundaries */

                let mut link = 0;
                while link < vf.links {
                    if vf.serialnos[link as usize] == serialno {
                        break;
                    }
                    link += 1;
                }

                if link == vf.links {
                    continue; /* not the desired Vorbis
                              bitstream section; keep
                              trying */
                }

                vf.current_serialno = serialno;
                vf.current_link = link;

                ogg_stream_reset_serialno(&mut vf.os, vf.current_serialno as i32);
                vf.ready_state = STREAMSET;
            } else {
                /* we're streaming */
                /* fetch the three header packets, build the info struct */

                let mut vi = std::mem::take(&mut vf.vi[0]);
                let mut vc = std::mem::take(&mut vf.vc[0]);
                let ret = _fetch_headers(vf, &mut vi, &mut vc, None, Some(&mut og));
                vf.vi[0] = vi;
                vf.vc[0] = vc;
                if ret != 0 {
                    return ret;
                }
                vf.current_serialno = vf.os.serialno;
                vf.current_link += 1;
            }
        }

        /* the buffered page is the data we want, and we're ready for it;
        add it to the stream state */
        ogg_stream_pagein(&mut vf.os, &og);
    }
}

/// Translation of `_ov_open1()` (`initial` is NULL, `ibytes` 0, and the
/// callbacks SDL_mixer's).
fn _ov_open1<'a>(f: IoStream<'a>, vf: &mut OggVorbisFile<'a>) -> i32 {
    let mut serialno_list: Vec<i64> = Vec::new();
    let mut serialno_list_size: i32 = 0;
    let ret: i32;

    *vf = OggVorbisFile::zeroed();
    vf.datasource = Some(f);
    let offsettest = vf.seek_func(0, SEEK_CUR);

    /* init the framing state */
    ogg_sync_init(&mut vf.oy);

    /* perhaps some data was previously read into a buffer for testing
    against other stream types.  Allow initialization from this
    previously read data (especially as we may be reading from a
    non-seekable stream) */
    /* (SDL_mixer gives no initial data) */

    /* can we seek? Stevens suggests the seek test was portable */
    if offsettest != -1 {
        vf.seekable = 1;
    }

    /* No seeking yet; Set up a 'single' (current) logical bitstream
    entry for partial open */
    vf.links = 1;
    vf.vi = vec![VorbisInfo::default()];
    vf.vc = vec![VorbisComment::default()];
    ogg_stream_init(&mut vf.os, -1); /* fill in the serialno later */

    /* Fetch all BOS pages, store the vorbis header and all seen serial
    numbers, load subsequent vorbis setup headers */
    let mut vi = std::mem::take(&mut vf.vi[0]);
    let mut vc = std::mem::take(&mut vf.vc[0]);
    ret = _fetch_headers(
        vf,
        &mut vi,
        &mut vc,
        Some((&mut serialno_list, &mut serialno_list_size)),
        None,
    );
    vf.vi[0] = vi;
    vf.vc[0] = vc;
    if ret < 0 {
        vf.datasource = None;
        ov_clear(vf);
    } else {
        /* serial number list for first link needs to be held somewhere
        for second stage of seekable stream open; this saves having to
        seek/reread first link's serialnumber data then. */
        vf.serialnos = vec![0; serialno_list_size as usize + 2];
        vf.current_serialno = vf.os.serialno;
        vf.serialnos[0] = vf.current_serialno;
        vf.serialnos[1] = serialno_list_size as i64;
        vf.serialnos[2..].copy_from_slice(&serialno_list[..serialno_list_size as usize]);

        vf.offsets = vec![0; 1];
        vf.dataoffsets = vec![0; 1];
        vf.offsets[0] = 0;
        vf.dataoffsets[0] = vf.offset;

        vf.ready_state = PARTOPEN;
    }
    ret
}

/// Translation of `_ov_open2()`.
fn _ov_open2(vf: &mut OggVorbisFile<'_>) -> i32 {
    if vf.ready_state != PARTOPEN {
        return OV_EINVAL;
    }
    vf.ready_state = OPENED;
    if vf.seekable != 0 {
        let ret = _open_seekable2(vf);
        if ret != 0 {
            vf.datasource = None;
            ov_clear(vf);
        }
        return ret;
    } else {
        vf.ready_state = STREAMSET;
    }

    0
}

/* clear out the OggVorbis_File struct */
/// Translation of `ov_clear()` (the close callback does nothing).
pub(crate) fn ov_clear(vf: &mut OggVorbisFile<'_>) -> i32 {
    *vf = OggVorbisFile::zeroed();
    0
}

/* inspects the OggVorbis file and finds/documents all the logical
bitstreams contained in it.  Tries to be tolerant of logical
bitstream sections that are truncated/woogie.

return: -1) error
         0) OK
*/

/// Translation of `ov_open_callbacks()`: the opened file, or the error
/// (after which, as in C, the stream is the caller's again).
pub(crate) fn ov_open_callbacks(f: IoStream<'_>) -> Result<Box<OggVorbisFile<'_>>, i32> {
    let mut vf = Box::new(OggVorbisFile::zeroed());
    let ret = _ov_open1(f, &mut vf);
    if ret != 0 {
        return Err(ret);
    }
    let ret = _ov_open2(&mut vf);
    if ret != 0 {
        return Err(ret);
    }
    Ok(vf)
}

/* Only partially open the vorbis file; test for Vorbisness, and load
the headers for the first chain.  Do not seek (although test for
seekability).  Use ov_test_open to finish opening the file, else
ov_clear to close/free it. Same return codes as open.

Note that vorbisfile does _not_ take ownership of the file if the
call fails; the calling applicaiton is responsible for closing the file
if this call returns an error. */

/// Translation of `ov_test_callbacks()`.
pub(crate) fn ov_test_callbacks(f: IoStream<'_>) -> Result<Box<OggVorbisFile<'_>>, i32> {
    let mut vf = Box::new(OggVorbisFile::zeroed());
    let ret = _ov_open1(f, &mut vf);
    if ret != 0 {
        return Err(ret);
    }
    Ok(vf)
}

/* returns: total PCM length (samples) of content if i==-1 PCM length
(samples) of that logical bitstream for i==0 to n
OV_EINVAL if the stream is not seekable (we can't know the
length) or only partially open
*/
/// Translation of `ov_pcm_total()`.
pub(crate) fn ov_pcm_total(vf: &OggVorbisFile<'_>, i: i32) -> i64 {
    if vf.ready_state < OPENED {
        return OV_EINVAL as i64;
    }
    if vf.seekable == 0 || i >= vf.links {
        return OV_EINVAL as i64;
    }
    if i < 0 {
        let mut acc: i64 = 0;
        for i in 0..vf.links {
            acc = acc.wrapping_add(ov_pcm_total(vf, i));
        }
        acc
    } else {
        vf.pcmlengths[i as usize * 2 + 1]
    }
}

/* seek to an offset relative to the *compressed* data. This also
scans packets to update the PCM cursor. It will cross a logical
bitstream boundary, but only if it can't get any packets out of the
tail of the bitstream we seek to (so no surprises).

returns zero on success, nonzero on failure */

/// Translation of `ov_raw_seek()`.
pub(crate) fn ov_raw_seek(vf: &mut OggVorbisFile<'_>, pos: i64) -> i32 {
    let mut work_os = OggStreamState::default();

    if vf.ready_state < OPENED {
        return OV_EINVAL;
    }
    if vf.seekable == 0 {
        return OV_ENOSEEK; /* don't dump machine if we can't seek */
    }

    if pos < 0 || pos > vf.end {
        return OV_EINVAL;
    }

    /* is the seek position outside our current link [if any]? */
    if vf.ready_state >= STREAMSET
        && (pos < vf.offsets[vf.current_link as usize]
            || pos >= vf.offsets[vf.current_link as usize + 1])
    {
        _decode_clear(vf); /* clear out stream state */
    }

    /* don't yet clear out decoding machine (if it's initialized), in
    the case we're in the same link.  Restart the decode lapping, and
    let _fetch_and_process_packet deal with a potential bitstream
    boundary */
    vf.pcm_offset = -1;
    ogg_stream_reset_serialno(&mut vf.os, vf.current_serialno as i32); /* must set serialno */
    {
        let vdi = vf.vd.vi;
        vorbis_synthesis_restart(&mut vf.vd, vdi.map(|i| &vf.vi[i]));
    }

    if _seek_helper(vf, pos) != 0 {
        /* dump the machine so we're in a known state */
        vf.pcm_offset = -1;
        _decode_clear(vf);
        return OV_EBADLINK;
    }

    /* we need to make sure the pcm_offset is set, but we don't want to
    advance the raw cursor past good packets just to get to the first
    with a granulepos.  That's not equivalent behavior to beginning
    decoding as immediately after the seek position as possible.

    So, a hack.  We use two stream states; a local scratch state and
    the shared vf->os stream state.  We use the local state to
    scan, and the shared state as a buffer for later decode.

    Unfortuantely, on the last page we still advance to last packet
    because the granulepos on the last page is not necessarily on a
    packet boundary, and we need to make sure the granpos is
    correct.
    */

    {
        let mut og = OggPage::default();
        let mut op = OggPacket::default();
        let mut lastblock: i32 = 0;
        let mut accblock: i32 = 0;
        let mut thisblock: i32;
        let mut lastflag: i32 = 0;
        let mut firstflag: i32 = 0;
        let mut pagepos: i64; /* (C's is -1 until it's first set) */

        ogg_stream_init(&mut work_os, vf.current_serialno as i32); /* get the memory ready */
        ogg_stream_reset(&mut work_os); /* eliminate the spurious OV_HOLE
                                        return from not necessarily
                                        starting from the beginning */

        loop {
            if vf.ready_state >= STREAMSET {
                /* snarf/scan a packet if we can */
                let result = ogg_stream_packetout(&mut work_os, Some(&mut op));

                if result > 0 {
                    let link = vf.current_link as usize;
                    if vf.vi[link].codec_setup.is_some() {
                        thisblock =
                            vorbis_packet_blocksize(&vf.vi[link], work_os.packet_data(&op)) as i32;
                        if thisblock < 0 {
                            ogg_stream_packetout(&mut vf.os, None);
                            thisblock = 0;
                        } else {
                            /* We can't get a guaranteed correct pcm position out of the
                            last page in a stream because it might have a 'short'
                            granpos, which can only be detected in the presence of a
                            preceding page.  However, if the last page is also the first
                            page, the granpos rules of a first page take precedence.  Not
                            only that, but for first==last, the EOS page must be treated
                            as if its a normal first page for the stream to open/play. */
                            if lastflag != 0 && firstflag == 0 {
                                ogg_stream_packetout(&mut vf.os, None);
                            } else if lastblock != 0 {
                                accblock += (lastblock + thisblock) >> 2;
                            }
                        }

                        if op.granulepos != -1 {
                            let link = vf.current_link as usize;
                            let mut granulepos =
                                op.granulepos.wrapping_sub(vf.pcmlengths[link * 2]);
                            if granulepos < 0 {
                                granulepos = 0;
                            }

                            for i in 0..link {
                                granulepos = granulepos.wrapping_add(vf.pcmlengths[i * 2 + 1]);
                            }
                            vf.pcm_offset = granulepos - accblock as i64;
                            if vf.pcm_offset < 0 {
                                vf.pcm_offset = 0;
                            }
                            break;
                        }
                        lastblock = thisblock;
                        continue;
                    } else {
                        ogg_stream_packetout(&mut vf.os, None);
                    }
                }
            }

            if lastblock == 0 {
                pagepos = _get_next_page(vf, &mut og, -1);
                if pagepos < 0 {
                    vf.pcm_offset = ov_pcm_total(vf, -1);
                    break;
                }
            } else {
                /* huh?  Bogus stream with packets but no granulepos */
                vf.pcm_offset = -1;
                break;
            }

            /* has our decoding just traversed a bitstream boundary? */
            if vf.ready_state >= STREAMSET && vf.current_serialno != ogg_page_serialno(&og) as i64 {
                /* two possibilities:
                1) our decoding just traversed a bitstream boundary
                2) another stream is multiplexed into this logical section? */

                if ogg_page_bos(&og) != 0 {
                    /* we traversed */
                    _decode_clear(vf); /* clear out stream state */
                    ogg_stream_clear(&mut work_os);
                } /* else, do nothing; next loop will scoop another page */
            }

            if vf.ready_state < STREAMSET {
                let serialno = ogg_page_serialno(&og) as i64;

                let mut link = 0;
                while link < vf.links {
                    if vf.serialnos[link as usize] == serialno {
                        break;
                    }
                    link += 1;
                }

                if link == vf.links {
                    continue; /* not the desired Vorbis
                              bitstream section; keep
                              trying */
                }
                vf.current_link = link;
                vf.current_serialno = serialno;
                ogg_stream_reset_serialno(&mut vf.os, serialno as i32);
                ogg_stream_reset_serialno(&mut work_os, serialno as i32);
                vf.ready_state = STREAMSET;
                firstflag = (pagepos <= vf.dataoffsets[link as usize]) as i32;
            }

            ogg_stream_pagein(&mut vf.os, &og);
            ogg_stream_pagein(&mut work_os, &og);
            lastflag = ogg_page_eos(&og);
        }
    }

    ogg_stream_clear(&mut work_os);
    vf.bittrack = 0.0;
    vf.samptrack = 0.0;
    0
}

/* Page granularity seek (faster than sample granularity because we
don't do the last bit of decode to find a specific sample).

Seek to the last [granule marked] page preceding the specified pos
location, such that decoding past the returned point will quickly
arrive at the requested position. */
/// Translation of `ov_pcm_seek_page()`.
pub(crate) fn ov_pcm_seek_page(vf: &mut OggVorbisFile<'_>, pos: i64) -> i32 {
    let mut link: i32;
    let mut result: i64 = 0;
    let mut total = ov_pcm_total(vf, -1);

    if vf.ready_state < OPENED {
        return OV_EINVAL;
    }
    if vf.seekable == 0 {
        return OV_ENOSEEK;
    }

    if pos < 0 || pos > total {
        return OV_EINVAL;
    }

    /* which bitstream section does this pcm offset occur in? */
    link = vf.links - 1;
    while link >= 0 {
        total = total.wrapping_sub(vf.pcmlengths[link as usize * 2 + 1]);
        if pos >= total {
            break;
        }
        link -= 1;
    }

    /* Search within the logical bitstream for the page with the highest
    pcm_pos preceding pos.  If we're looking for a position on the
    first page, bisection will halt without finding our position as
    it's before the first explicit granulepos fencepost. That case is
    handled separately below.

    There is a danger here; missing pages or incorrect frame number
    information in the bitstream could make our task impossible.
    Account for that (it would be an error condition) */

    /* new search algorithm originally by HB (Nicholas Vinen) */

    'seek_error: {
        let l = link as usize;
        let mut end = vf.offsets[l + 1];
        let mut begin = vf.dataoffsets[l];
        let mut begintime = vf.pcmlengths[l * 2];
        let mut endtime = vf.pcmlengths[l * 2 + 1].wrapping_add(begintime);
        let target = pos.wrapping_sub(total).wrapping_add(begintime);
        let mut best: i64 = -1;
        let mut got_page = 0;

        let mut og = OggPage::default();

        /* if we have only one page, there will be no bisection.  Grab the page here */
        if begin == end {
            result = _seek_helper(vf, begin) as i64;
            if result != 0 {
                break 'seek_error;
            }

            result = _get_next_page(vf, &mut og, 1);
            if result < 0 {
                break 'seek_error;
            }

            got_page = 1;
        }

        /* bisection loop */
        while begin < end {
            let mut bisect: i64;

            if end - begin < CHUNKSIZE {
                bisect = begin;
            } else {
                /* take a (pretty decent) guess. */
                bisect = begin
                    .wrapping_add(c_long(
                        target.wrapping_sub(begintime) as f64 * (end - begin) as f64
                            / endtime.wrapping_sub(begintime) as f64,
                    ))
                    .wrapping_sub(CHUNKSIZE);
                if bisect < begin + CHUNKSIZE {
                    bisect = begin;
                }
            }

            result = _seek_helper(vf, bisect) as i64;
            if result != 0 {
                break 'seek_error;
            }

            /* read loop within the bisection loop */
            while begin < end {
                result = _get_next_page(vf, &mut og, end - vf.offset);
                if result == OV_EREAD as i64 {
                    break 'seek_error;
                }
                if result < 0 {
                    /* there is no next page! */
                    if bisect <= begin + 1 {
                        /* No bisection left to perform.  We've either found the
                        best candidate already or failed. Exit loop. */
                        end = begin;
                    } else {
                        /* We tried to load a fraction of the last page; back up a
                        bit and try to get the whole last page */
                        if bisect == 0 {
                            break 'seek_error;
                        }
                        bisect -= CHUNKSIZE;

                        /* don't repeat/loop on a read we've already performed */
                        if bisect <= begin {
                            bisect = begin + 1;
                        }

                        /* seek and cntinue bisection */
                        result = _seek_helper(vf, bisect) as i64;
                        if result != 0 {
                            break 'seek_error;
                        }
                    }
                } else {
                    let granulepos: i64;
                    got_page = 1;

                    /* got a page. analyze it */
                    /* only consider pages from primary vorbis stream */
                    if ogg_page_serialno(&og) as i64 != vf.serialnos[l] {
                        continue;
                    }

                    /* only consider pages with the granulepos set */
                    granulepos = ogg_page_granulepos(&og);
                    if granulepos == -1 {
                        continue;
                    }

                    if granulepos < target {
                        /* this page is a successful candidate! Set state */

                        best = result; /* raw offset of packet with granulepos */
                        begin = vf.offset; /* raw offset of next page */
                        begintime = granulepos;

                        /* if we're before our target but within a short distance,
                        don't bisect; read forward */
                        if target.wrapping_sub(begintime) > 44100 {
                            break;
                        }

                        bisect = begin; /* *not* begin + 1 as above */
                    } else {
                        /* This is one of our pages, but the granpos is
                        post-target; it is not a bisection return
                        candidate. (The only way we'd use it is if it's the
                        first page in the stream; we handle that case later
                        outside the bisection) */
                        if bisect <= begin + 1 {
                            /* No bisection left to perform.  We've either found the
                            best candidate already or failed. Exit loop. */
                            end = begin;
                        } else if end == vf.offset {
                            /* bisection read to the end; use the known page
                            boundary (result) to update bisection, back up a
                            little bit, and try again */
                            end = result;
                            bisect -= CHUNKSIZE;
                            if bisect <= begin {
                                bisect = begin + 1;
                            }
                            result = _seek_helper(vf, bisect) as i64;
                            if result != 0 {
                                break 'seek_error;
                            }
                        } else {
                            /* Normal bisection */
                            end = bisect;
                            endtime = granulepos;
                            break;
                        }
                    }
                }
            }
        }

        /* Out of bisection: did it 'fail?' */
        if best == -1 {
            /* Check the 'looking for data in first page' special case;
            bisection would 'fail' because our search target was before the
            first PCM granule position fencepost. */

            if got_page != 0
                && begin == vf.dataoffsets[l]
                && ogg_page_serialno(&og) as i64 == vf.serialnos[l]
            {
                /* Yes, this is the beginning-of-stream case. We already have
                our page, right at the beginning of PCM data.  Set state
                and return. */

                vf.pcm_offset = total;

                if link != vf.current_link {
                    /* Different link; dump entire decode machine */
                    _decode_clear(vf);

                    vf.current_link = link;
                    vf.current_serialno = vf.serialnos[l];
                    vf.ready_state = STREAMSET;
                } else {
                    let vdi = vf.vd.vi;
                    vorbis_synthesis_restart(&mut vf.vd, vdi.map(|i| &vf.vi[i]));
                }

                ogg_stream_reset_serialno(&mut vf.os, vf.current_serialno as i32);
                ogg_stream_pagein(&mut vf.os, &og);
            } else {
                break 'seek_error;
            }
        } else {
            /* Bisection found our page. seek to it, update pcm offset. Easier case than
            raw_seek, don't keep packets preceding granulepos. */

            let mut og = OggPage::default();
            let mut op = OggPacket::default();

            /* seek */
            result = _seek_helper(vf, best) as i64;
            vf.pcm_offset = -1;
            if result != 0 {
                break 'seek_error;
            }
            result = _get_next_page(vf, &mut og, -1);
            if result < 0 {
                break 'seek_error;
            }

            if link != vf.current_link {
                /* Different link; dump entire decode machine */
                _decode_clear(vf);

                vf.current_link = link;
                vf.current_serialno = vf.serialnos[l];
                vf.ready_state = STREAMSET;
            } else {
                let vdi = vf.vd.vi;
                vorbis_synthesis_restart(&mut vf.vd, vdi.map(|i| &vf.vi[i]));
            }

            ogg_stream_reset_serialno(&mut vf.os, vf.current_serialno as i32);
            ogg_stream_pagein(&mut vf.os, &og);

            /* pull out all but last packet; the one with granulepos */
            loop {
                result = ogg_stream_packetpeek(&mut vf.os, Some(&mut op)) as i64;
                if result == 0 {
                    /* No packet returned; we exited the bisection with 'best'
                    pointing to a page with a granule position, so the packet
                    finishing this page ('best') originated on a preceding
                    page. Keep fetching previous pages until we get one with
                    a granulepos or without the 'continued' flag set.  Then
                    just use raw_seek for simplicity. */
                    /* Do not rewind past the beginning of link data; if we do,
                    it's either a bug or a broken stream */
                    result = best;
                    while result > vf.dataoffsets[l] {
                        result = _get_prev_page(vf, result, &mut og);
                        if result < 0 {
                            break 'seek_error;
                        }
                        if ogg_page_serialno(&og) as i64 == vf.current_serialno
                            && (ogg_page_granulepos(&og) > -1 || ogg_page_continued(&og) == 0)
                        {
                            return ov_raw_seek(vf, result);
                        }
                    }
                }
                if result < 0 {
                    result = OV_EBADPACKET as i64;
                    break 'seek_error;
                }
                if op.granulepos != -1 {
                    vf.pcm_offset = op
                        .granulepos
                        .wrapping_sub(vf.pcmlengths[vf.current_link as usize * 2]);
                    if vf.pcm_offset < 0 {
                        vf.pcm_offset = 0;
                    }
                    vf.pcm_offset = vf.pcm_offset.wrapping_add(total);
                    break;
                } else {
                    /* (result=) */
                    ogg_stream_packetout(&mut vf.os, None);
                }
            }
        }

        /* verify result */
        if vf.pcm_offset > pos || pos > ov_pcm_total(vf, -1) {
            result = OV_EFAULT as i64;
            break 'seek_error;
        }
        vf.bittrack = 0.0;
        vf.samptrack = 0.0;
        return 0;
    }

    /* seek_error: */
    /* dump machine so we're in a known state */
    vf.pcm_offset = -1;
    _decode_clear(vf);
    result as i32
}

/* seek to a sample offset relative to the decompressed pcm stream
returns zero on success, nonzero on failure */

/// Translation of `ov_pcm_seek()`.
pub(crate) fn ov_pcm_seek(vf: &mut OggVorbisFile<'_>, pos: i64) -> i32 {
    let mut thisblock: i32;
    let mut lastblock: i32 = 0;
    let mut ret = ov_pcm_seek_page(vf, pos);
    if ret < 0 {
        return ret;
    }
    ret = _make_decode_ready(vf);
    if ret != 0 {
        return ret;
    }

    /* discard leading packets we don't need for the lapping of the
    position we want; don't decode them */

    loop {
        let mut op = OggPacket::default();
        let mut og = OggPage::default();

        let ret = ogg_stream_packetpeek(&mut vf.os, Some(&mut op));
        if ret > 0 {
            thisblock =
                vorbis_packet_blocksize(&vf.vi[vf.current_link as usize], vf.os.packet_data(&op))
                    as i32;
            if thisblock < 0 {
                ogg_stream_packetout(&mut vf.os, None);
                continue; /* non audio packet */
            }
            if lastblock != 0 {
                vf.pcm_offset += ((lastblock + thisblock) >> 2) as i64;
            }

            if vf.pcm_offset
                + ((thisblock + super::info::vorbis_info_blocksize(&vf.vi[0], 1)) >> 2) as i64
                >= pos
            {
                break;
            }

            /* remove the packet from packet queue and track its granulepos */
            ogg_stream_packetout(&mut vf.os, None);
            let vdi = vf.vd.vi.unwrap();
            vorbis_synthesis_trackonly(
                &mut vf.vb,
                &vf.vd,
                &vf.vi[vdi],
                vf.os.packet_data(&op),
                op.granulepos,
                op.packetno,
                op.e_o_s,
            ); /* set up a vb with
               only tracking, no
               pcm_decode */
            vorbis_synthesis_blockin(&mut vf.vd, &vf.vi[vdi], &vf.vb);

            /* end of logical stream case is hard, especially with exact
            length positioning. */

            if op.granulepos > -1 {
                /* always believe the stream markers */
                vf.pcm_offset = op
                    .granulepos
                    .wrapping_sub(vf.pcmlengths[vf.current_link as usize * 2]);
                if vf.pcm_offset < 0 {
                    vf.pcm_offset = 0;
                }
                for i in 0..vf.current_link as usize {
                    vf.pcm_offset = vf.pcm_offset.wrapping_add(vf.pcmlengths[i * 2 + 1]);
                }
            }

            lastblock = thisblock;
        } else {
            if ret < 0 && ret != OV_HOLE {
                break;
            }

            /* suck in a new page */
            if _get_next_page(vf, &mut og, -1) < 0 {
                break;
            }
            if ogg_page_bos(&og) != 0 {
                _decode_clear(vf);
            }

            if vf.ready_state < STREAMSET {
                let serialno = ogg_page_serialno(&og) as i64;
                let mut link = 0;

                while link < vf.links {
                    if vf.serialnos[link as usize] == serialno {
                        break;
                    }
                    link += 1;
                }
                if link == vf.links {
                    continue;
                }
                vf.current_link = link;

                vf.ready_state = STREAMSET;
                vf.current_serialno = ogg_page_serialno(&og) as i64;
                ogg_stream_reset_serialno(&mut vf.os, serialno as i32);
                let ret = _make_decode_ready(vf);
                if ret != 0 {
                    return ret;
                }
                lastblock = 0;
            }

            ogg_stream_pagein(&mut vf.os, &og);
        }
    }

    vf.bittrack = 0.0;
    vf.samptrack = 0.0;
    /* discard samples until we reach the desired position. Crossing a
    logical bitstream boundary with abandon is OK. */
    {
        /* note that halfrate could be set differently in each link, but
        vorbisfile encoforces all links are set or unset */
        let hs = vorbis_synthesis_halfrate_p(&vf.vi[0]);
        while vf.pcm_offset < ((pos >> hs) << hs) {
            let target: i64 = (pos - vf.pcm_offset) >> hs;
            let mut samples: i64 = vorbis_synthesis_pcmout(&vf.vd, None) as i64;

            if samples > target {
                samples = target;
            }
            vorbis_synthesis_read(&mut vf.vd, samples as i32);
            vf.pcm_offset += samples << hs;

            if samples < target && _fetch_and_process_packet(vf, 1, 1) <= 0 {
                vf.pcm_offset = ov_pcm_total(vf, -1); /* eof */
            }
        }
    }
    0
}

/* return PCM offset (sample) of next PCM sample to be read */
/// Translation of `ov_pcm_tell()`.
#[allow(dead_code)]
pub(crate) fn ov_pcm_tell(vf: &OggVorbisFile<'_>) -> i64 {
    if vf.ready_state < OPENED {
        return OV_EINVAL as i64;
    }
    vf.pcm_offset
}

/*  link:   -1) return the vorbis_info struct for the bitstream section
            currently being decoded
       0-n) to request information for a specific bitstream section

In the case of a non-seekable bitstream, any call returns the
current bitstream.  NULL in the case that the machine is not
initialized */

/// Translation of `ov_info()`.
pub(crate) fn ov_info<'f>(vf: &'f OggVorbisFile<'_>, link: i32) -> Option<&'f VorbisInfo> {
    if vf.seekable != 0 {
        if link < 0 {
            if vf.ready_state >= STREAMSET {
                vf.vi.get(vf.current_link as usize)
            } else {
                vf.vi.first()
            }
        } else if link >= vf.links {
            None
        } else {
            vf.vi.get(link as usize)
        }
    } else {
        vf.vi.first()
    }
}

/* grr, strong typing, grr, no templates/inheritence, grr */
/// Translation of `ov_comment()`.
pub(crate) fn ov_comment<'f>(vf: &'f OggVorbisFile<'_>, link: i32) -> Option<&'f VorbisComment> {
    if vf.seekable != 0 {
        if link < 0 {
            if vf.ready_state >= STREAMSET {
                vf.vc.get(vf.current_link as usize)
            } else {
                vf.vc.first()
            }
        } else if link >= vf.links {
            None
        } else {
            vf.vc.get(link as usize)
        }
    } else {
        vf.vc.first()
    }
}

/* input values: pcm_channels) a float vector per channel of output
              length) the sample length being read by the app

return values: <0) error/hole in data (OV_HOLE), partial open (OV_EINVAL)
                0) EOF
                n) number of samples of PCM actually returned.  The
                below works on a packet-by-packet basis, so the
                return length is not related to the 'length' passed
                in, just guaranteed to fit.

         *section) set to the logical bitstream number */

/// Translation of `ov_read_float()`: `pcm_channels`, when given, is set
/// to where the returned samples start in each of `vf.vd.pcm`'s vectors
/// (C returns pointers there).
pub(crate) fn ov_read_float(
    vf: &mut OggVorbisFile<'_>,
    pcm_channels: Option<&mut usize>,
    length: i32,
    bitstream: Option<&mut i32>,
) -> i64 {
    if vf.ready_state < OPENED {
        return OV_EINVAL as i64;
    }

    let mut pcm_channels = pcm_channels;
    loop {
        if vf.ready_state == INITSET {
            let mut pcm = 0usize;
            let mut samples = vorbis_synthesis_pcmout(&vf.vd, Some(&mut pcm)) as i64;
            if samples != 0 {
                let hs = vorbis_synthesis_halfrate_p(&vf.vi[0]);
                if let Some(p) = pcm_channels.take() {
                    *p = pcm;
                }
                if samples > length as i64 {
                    samples = length as i64;
                }
                vorbis_synthesis_read(&mut vf.vd, samples as i32);
                vf.pcm_offset += samples << hs;
                if let Some(b) = bitstream {
                    *b = vf.current_link;
                }
                return samples;
            }
        }

        /* suck in another packet */
        {
            let ret = _fetch_and_process_packet(vf, 1, 1);
            if ret == OV_EOF {
                return 0;
            }
            if ret <= 0 {
                return ret as i64;
            }
        }
    }
}
