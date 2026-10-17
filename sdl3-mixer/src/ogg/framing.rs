// Rust translation of src/framing.c and src/crctable.h (the decoding half)
// and the structures of include/ogg/ogg.h from libogg 1.3.5
// (https://xiph.org/ogg/, at the revision SDL_mixer's external/ogg pins).
// THE OggVorbis SOURCE CODE IS (C) COPYRIGHT 1994-2018
// by the Xiph.Org Foundation http://www.xiph.org/
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Ogg framing: finding pages in a byte stream and reassembling packets
//! from them. The encoding half (`ogg_stream_packetin()`,
//! `ogg_stream_pageout()`, `ogg_stream_flush()` and friends) isn't
//! translated; opusfile only decodes.
//!
//! The C structures point into each other's buffers; here an [`OggPage`]
//! owns copies of its header and body, and an [`OggPacket`] holds the
//! offset of its data in the [`OggStreamState`]'s body buffer (C's pointer
//! there), valid until the next `ogg_stream_pagein()`, as in C. C's `long`
//! is `i64` (its LP64 size); the `LONG_MAX` and `INT_MAX` checks keep their
//! LP64 values.

/* A complete description of Ogg framing exists in docs/framing.html */

/// Translation of `ogg_page`: the page's header and body, copied out of the
/// sync buffer (C points into it).
#[derive(Clone, Debug, Default)]
pub(crate) struct OggPage {
    pub header: Vec<u8>,
    pub body: Vec<u8>,
}

/// Translation of `ogg_packet`. `packet` is the offset of the packet's
/// data in its stream's `body_data` (see [`OggStreamState::packet_data`]).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct OggPacket {
    pub packet: usize,
    pub bytes: i64,
    pub b_o_s: i64,
    pub e_o_s: i64,

    pub granulepos: i64,

    pub packetno: i64, /* sequence number for decode; the framing
                       knows where there's a hole in the data,
                       but we need coupling so that the codec
                       (which is in a separate abstraction
                       layer) also knows about the gap */
}

/// Translation of `ogg_stream_state`; an empty `body_data` is C's NULL
/// (a cleared stream).
#[derive(Debug, Default)]
pub(crate) struct OggStreamState {
    body_data: Vec<u8>, /* bytes from packet bodies */
    body_storage: i64,  /* storage elements allocated */
    body_fill: i64,     /* elements stored; fill mark */
    body_returned: i64, /* elements of fill returned */

    lacing_vals: Vec<i32>, /* The values that will go to the segment table */
    granule_vals: Vec<i64>, /* granulepos values for headers. Not compact
                           this way, but it is simple coupled to the
                           lacing fifo */
    lacing_storage: i64,
    lacing_fill: i64,
    lacing_packet: i64,
    lacing_returned: i64,

    /* (header and header_fill, the encoder's working space, aren't needed) */
    e_o_s: i32, /* set when we have buffered the last packet in the
                logical bitstream */
    b_o_s: i32, /* set after we've written the initial page
                of a logical bitstream */
    pub serialno: i64,
    pageno: i64,
    packetno: i64, /* sequence number for decode; the framing
                   knows where there's a hole in the data,
                   but we need coupling so that the codec
                   (which is in a separate abstraction
                   layer) also knows about the gap */
    granulepos: i64,
}

/// Translation of `ogg_sync_state`.
#[derive(Debug, Default)]
pub(crate) struct OggSyncState {
    data: Vec<u8>,
    storage: i32,
    fill: i32,
    returned: i32,

    unsynced: i32,
    headerbytes: i32,
    bodybytes: i32,
}

const LONG_MAX: i64 = i64::MAX;
const INT_MAX: i32 = i32::MAX;

/// Translation of `ogg_page_version()`.
pub(crate) fn ogg_page_version(og: &OggPage) -> i32 {
    og.header[4] as i32
}

/// Translation of `ogg_page_continued()`.
pub(crate) fn ogg_page_continued(og: &OggPage) -> i32 {
    (og.header[5] & 0x01) as i32
}

/// Translation of `ogg_page_bos()`.
pub(crate) fn ogg_page_bos(og: &OggPage) -> i32 {
    (og.header[5] & 0x02) as i32
}

/// Translation of `ogg_page_eos()`.
pub(crate) fn ogg_page_eos(og: &OggPage) -> i32 {
    (og.header[5] & 0x04) as i32
}

/// Translation of `ogg_page_granulepos()`.
pub(crate) fn ogg_page_granulepos(og: &OggPage) -> i64 {
    let page = &og.header;
    let mut granulepos: u64 = (page[13] & 0xff) as u64;
    granulepos = (granulepos << 8) | (page[12] & 0xff) as u64;
    granulepos = (granulepos << 8) | (page[11] & 0xff) as u64;
    granulepos = (granulepos << 8) | (page[10] & 0xff) as u64;
    granulepos = (granulepos << 8) | (page[9] & 0xff) as u64;
    granulepos = (granulepos << 8) | (page[8] & 0xff) as u64;
    granulepos = (granulepos << 8) | (page[7] & 0xff) as u64;
    granulepos = (granulepos << 8) | (page[6] & 0xff) as u64;
    granulepos as i64
}

/// Translation of `ogg_page_serialno()`.
pub(crate) fn ogg_page_serialno(og: &OggPage) -> i32 {
    ((og.header[14] as u32)
        | ((og.header[15] as u32) << 8)
        | ((og.header[16] as u32) << 16)
        | ((og.header[17] as u32) << 24)) as i32
}

/// Translation of `ogg_page_pageno()`.
pub(crate) fn ogg_page_pageno(og: &OggPage) -> i64 {
    /* (C's long of a 32-bit value: the OR is done in ogg_uint32_t) */
    ((og.header[18] as u32)
        | ((og.header[19] as u32) << 8)
        | ((og.header[20] as u32) << 16)
        | ((og.header[21] as u32) << 24)) as i64
}

/* returns the number of packets that are completed on this page (if
the leading packet is begun on a previous page, but ends on this
page, it's counted */

/* NOTE:
If a page consists of a packet begun on a previous page, and a new
packet begun (but not completed) on this page, the return will be:
  ogg_page_packets(page)   ==1,
  ogg_page_continued(page) !=0

If a page happens to be a single packet that was begun on a
previous page, and spans to the next page (in the case of a three or
more page packet), the return will be:
  ogg_page_packets(page)   ==0,
  ogg_page_continued(page) !=0
*/

/// Translation of `ogg_page_packets()`.
pub(crate) fn ogg_page_packets(og: &OggPage) -> i32 {
    let n = og.header[26] as usize;
    let mut count = 0;
    for i in 0..n {
        if og.header[27 + i] < 255 {
            count += 1;
        }
    }
    count
}

/* helper to initialize lookup for direct-table CRC (illustrative; we
use the static init in crctable.h) */
/* (crctable.h's table is the one this computes, at compile time) */
const fn ogg_crc_init() -> [[u32; 256]; 8] {
    let mut crc_lookup = [[0u32; 256]; 8];
    let polynomial: u32 = 0x04c11db7; /* The same as the ethernet generator
                                      polynomial, although we use an
                                      unreflected alg and an init/final
                                      of 0, not 0xffffffff */
    let mut i = 0;
    while i <= 0xFF {
        let mut crc: u32 = (i as u32) << 24;

        let mut j = 0;
        while j < 8 {
            crc = (crc << 1) ^ (if crc & (1 << 31) != 0 { polynomial } else { 0 });
            j += 1;
        }

        crc_lookup[0][i] = crc;
        i += 1;
    }

    let mut i = 0;
    while i <= 0xFF {
        let mut j = 1;
        while j < 8 {
            crc_lookup[j][i] = crc_lookup[0][((crc_lookup[j - 1][i] >> 24) & 0xFF) as usize]
                ^ (crc_lookup[j - 1][i] << 8);
            j += 1;
        }
        i += 1;
    }
    crc_lookup
}

static CRC_LOOKUP: [[u32; 256]; 8] = ogg_crc_init();

impl OggStreamState {
    /// The data of `op`, a packet this stream returned (C's `op->packet`).
    pub(crate) fn packet_data(&self, op: &OggPacket) -> &[u8] {
        &self.body_data[op.packet..op.packet + op.bytes as usize]
    }
}

/// C's `os->body_returned`.
pub(crate) fn ogg_stream_body_returned(os: &OggStreamState) -> i64 {
    os.body_returned
}

/// C's `os->body_fill`.
pub(crate) fn ogg_stream_body_fill(os: &OggStreamState) -> i64 {
    os.body_fill
}

/// C's `oy->fill`.
pub(crate) fn ogg_sync_fill(oy: &OggSyncState) -> i32 {
    oy.fill
}

/// C's `oy->returned`.
pub(crate) fn ogg_sync_returned(oy: &OggSyncState) -> i32 {
    oy.returned
}

/* init the encode/decode logical stream state */

/// Translation of `ogg_stream_init()`.
pub(crate) fn ogg_stream_init(os: &mut OggStreamState, serialno: i32) -> i32 {
    *os = OggStreamState::default();
    os.body_storage = 16 * 1024;
    os.lacing_storage = 1024;

    let ok = os
        .body_data
        .try_reserve_exact(os.body_storage as usize)
        .is_ok()
        && os
            .lacing_vals
            .try_reserve_exact(os.lacing_storage as usize)
            .is_ok()
        && os
            .granule_vals
            .try_reserve_exact(os.lacing_storage as usize)
            .is_ok();
    if !ok {
        ogg_stream_clear(os);
        return -1;
    }
    os.body_data.resize(os.body_storage as usize, 0);
    os.lacing_vals.resize(os.lacing_storage as usize, 0);
    os.granule_vals.resize(os.lacing_storage as usize, 0);

    os.serialno = serialno as i64;

    0
}

/* async/delayed error detection for the ogg_stream_state */
/// Translation of `ogg_stream_check()`.
pub(crate) fn ogg_stream_check(os: &OggStreamState) -> i32 {
    if os.body_data.is_empty() {
        return -1;
    }
    0
}

/* _clear does not free os, only the non-flat storage within */
/// Translation of `ogg_stream_clear()`.
pub(crate) fn ogg_stream_clear(os: &mut OggStreamState) -> i32 {
    *os = OggStreamState::default();
    0
}

/* Helpers for ogg_stream_encode; this keeps the structure and
what's happening fairly clear */

fn grow<T: Clone + Default>(v: &mut Vec<T>, new_len: usize) -> bool {
    if v.try_reserve_exact(new_len.saturating_sub(v.len()))
        .is_err()
    {
        return false;
    }
    v.resize(new_len, T::default());
    true
}

fn os_body_expand(os: &mut OggStreamState, needed: i64) -> i32 {
    if os.body_storage - needed <= os.body_fill {
        if os.body_storage > LONG_MAX - needed {
            ogg_stream_clear(os);
            return -1;
        }
        let mut body_storage = os.body_storage + needed;
        if body_storage < LONG_MAX - 1024 {
            body_storage += 1024;
        }
        if usize::try_from(body_storage).is_err() || !grow(&mut os.body_data, body_storage as usize)
        {
            ogg_stream_clear(os);
            return -1;
        }
        os.body_storage = body_storage;
    }
    0
}

fn os_lacing_expand(os: &mut OggStreamState, needed: i64) -> i32 {
    if os.lacing_storage - needed <= os.lacing_fill {
        if os.lacing_storage > LONG_MAX - needed {
            ogg_stream_clear(os);
            return -1;
        }
        let mut lacing_storage = os.lacing_storage + needed;
        if lacing_storage < LONG_MAX - 32 {
            lacing_storage += 32;
        }
        if !grow(&mut os.lacing_vals, lacing_storage as usize) {
            ogg_stream_clear(os);
            return -1;
        }
        if !grow(&mut os.granule_vals, lacing_storage as usize) {
            ogg_stream_clear(os);
            return -1;
        }
        os.lacing_storage = lacing_storage;
    }
    0
}

/* checksum the page */
/* Direct table CRC; note that this will be faster in the future if we
perform the checksum simultaneously with other copies */

fn os_update_crc(mut crc: u32, mut buffer: &[u8]) -> u32 {
    while buffer.len() >= 8 {
        crc ^= ((buffer[0] as u32) << 24)
            | ((buffer[1] as u32) << 16)
            | ((buffer[2] as u32) << 8)
            | (buffer[3] as u32);

        crc = CRC_LOOKUP[7][(crc >> 24) as usize]
            ^ CRC_LOOKUP[6][((crc >> 16) & 0xFF) as usize]
            ^ CRC_LOOKUP[5][((crc >> 8) & 0xFF) as usize]
            ^ CRC_LOOKUP[4][(crc & 0xFF) as usize]
            ^ CRC_LOOKUP[3][buffer[4] as usize]
            ^ CRC_LOOKUP[2][buffer[5] as usize]
            ^ CRC_LOOKUP[1][buffer[6] as usize]
            ^ CRC_LOOKUP[0][buffer[7] as usize];

        buffer = &buffer[8..];
    }

    for &b in buffer {
        crc = (crc << 8) ^ CRC_LOOKUP[0][(((crc >> 24) & 0xff) ^ b as u32) as usize];
    }
    crc
}

/// Translation of `ogg_page_checksum_set()` on a page whose header and body
/// are `page[..header_len]` and `page[header_len..header_len + body_len]`
/// (which is where `ogg_sync_pageseek()` checks them).
fn ogg_page_checksum_set(page: &mut [u8], header_len: usize, body_len: usize) {
    /* safety; needed for API behavior, but not framing code */
    page[22] = 0;
    page[23] = 0;
    page[24] = 0;
    page[25] = 0;

    let mut crc_reg = os_update_crc(0, &page[..header_len]);
    crc_reg = os_update_crc(crc_reg, &page[header_len..header_len + body_len]);

    page[22] = (crc_reg & 0xff) as u8;
    page[23] = ((crc_reg >> 8) & 0xff) as u8;
    page[24] = ((crc_reg >> 16) & 0xff) as u8;
    page[25] = ((crc_reg >> 24) & 0xff) as u8;
}

/// Translation of `ogg_stream_eos()`.
#[allow(dead_code)]
pub(crate) fn ogg_stream_eos(os: &OggStreamState) -> i32 {
    if ogg_stream_check(os) != 0 {
        return 1;
    }
    os.e_o_s
}

/* DECODING PRIMITIVES: packet streaming layer **********************/

/* This has two layers to place more of the multi-serialno and paging
control in the application's hands.  First, we expose a data buffer
using ogg_sync_buffer().  The app either copies into the
buffer, or passes it directly to read(), etc.  We then call
ogg_sync_wrote() to tell how many bytes we just added.

Pages are returned (pointers into the buffer in ogg_sync_state)
by ogg_sync_pageout().  The page is then submitted to
ogg_stream_pagein() along with the appropriate
ogg_stream_state* (ie, matching serialno).  We then get raw
packets out calling ogg_stream_packetout() with a
ogg_stream_state. */

/* initialize the struct to a known state */
/// Translation of `ogg_sync_init()`.
pub(crate) fn ogg_sync_init(oy: &mut OggSyncState) -> i32 {
    *oy = OggSyncState::default();
    0
}

/* clear non-flat storage within */
/// Translation of `ogg_sync_clear()`.
pub(crate) fn ogg_sync_clear(oy: &mut OggSyncState) -> i32 {
    *oy = OggSyncState::default();
    0
}

/// Translation of `ogg_sync_check()`.
pub(crate) fn ogg_sync_check(oy: &OggSyncState) -> i32 {
    if oy.storage < 0 {
        return -1;
    }
    0
}

/// Translation of `ogg_sync_buffer()`: the buffer space at the fill mark
/// (at least `size` bytes), or `None` (C's NULL).
pub(crate) fn ogg_sync_buffer(oy: &mut OggSyncState, size: i64) -> Option<&mut [u8]> {
    if ogg_sync_check(oy) != 0 {
        return None;
    }

    /* first, clear out any space that has been previously returned */
    if oy.returned != 0 {
        oy.fill -= oy.returned;
        if oy.fill > 0 {
            let r = oy.returned as usize;
            oy.data.copy_within(r..r + oy.fill as usize, 0);
        }
        oy.returned = 0;
    }

    if size > (oy.storage - oy.fill) as i64 {
        /* We need to extend the internal buffer */
        if size > (INT_MAX - 4096 - oy.fill) as i64 {
            ogg_sync_clear(oy);
            return None;
        }
        let newsize = size + oy.fill as i64 + 4096; /* an extra page to be nice */
        if !grow(&mut oy.data, newsize as usize) {
            ogg_sync_clear(oy);
            return None;
        }
        oy.storage = newsize as i32;
    }

    /* expose a segment at least as large as requested at the fill mark */
    Some(&mut oy.data[oy.fill as usize..])
}

/// Translation of `ogg_sync_wrote()`.
pub(crate) fn ogg_sync_wrote(oy: &mut OggSyncState, bytes: i64) -> i32 {
    if ogg_sync_check(oy) != 0 {
        return -1;
    }
    if oy.fill as i64 + bytes > oy.storage as i64 {
        return -1;
    }
    oy.fill += bytes as i32;
    0
}

/* sync the stream.  This is meant to be useful for finding page
boundaries.

return values for this:
-n) skipped n bytes
 0) page not ready; more data (no bytes skipped)
 n) page synced at current location; page length n bytes

*/

/// Translation of `ogg_sync_pageseek()`.
pub(crate) fn ogg_sync_pageseek(oy: &mut OggSyncState, og: Option<&mut OggPage>) -> i64 {
    if ogg_sync_check(oy) != 0 {
        return 0;
    }

    let page_off = oy.returned as usize;
    let mut bytes = (oy.fill - oy.returned) as i64;

    'sync_fail: {
        if oy.headerbytes == 0 {
            if bytes < 27 {
                return 0; /* not enough for a header */
            }
            let page = &oy.data[page_off..];

            /* verify capture pattern */
            if &page[..4] != b"OggS" {
                break 'sync_fail;
            }

            let headerbytes = page[26] as i32 + 27;
            if bytes < headerbytes as i64 {
                return 0; /* not enough for header + seg table */
            }

            /* count up body length in the segment table */

            for i in 0..page[26] as usize {
                oy.bodybytes += page[27 + i] as i32;
            }
            oy.headerbytes = headerbytes;
        }

        if (oy.bodybytes + oy.headerbytes) as i64 > bytes {
            return 0;
        }

        /* The whole test page is buffered.  Verify the checksum */
        {
            let page = &mut oy.data[page_off..];
            /* Grab the checksum bytes, set the header field to zero */
            let mut chksum = [0u8; 4];

            chksum.copy_from_slice(&page[22..26]);
            page[22..26].fill(0);

            /* set up a temp page struct and recompute the checksum */
            ogg_page_checksum_set(page, oy.headerbytes as usize, oy.bodybytes as usize);

            /* Compare */
            if chksum != page[22..26] {
                /* D'oh.  Mismatch! Corrupt page (or miscapture and not a page
                at all) */
                /* replace the computed checksum with the one actually read in */
                page[22..26].copy_from_slice(&chksum);

                /* Bad checksum. Lose sync */
                break 'sync_fail;
            }
        }

        /* yes, have a whole page all ready to go */
        {
            if let Some(og) = og {
                let hb = oy.headerbytes as usize;
                let bb = oy.bodybytes as usize;
                og.header.clear();
                og.header
                    .extend_from_slice(&oy.data[page_off..page_off + hb]);
                og.body.clear();
                og.body
                    .extend_from_slice(&oy.data[page_off + hb..page_off + hb + bb]);
            }

            oy.unsynced = 0;
            bytes = (oy.headerbytes + oy.bodybytes) as i64;
            oy.returned += bytes as i32;
            oy.headerbytes = 0;
            oy.bodybytes = 0;
            return bytes;
        }
    }

    /* sync_fail: */

    oy.headerbytes = 0;
    oy.bodybytes = 0;

    /* search for possible capture */
    let fill = oy.fill as usize;
    let next = match oy.data[page_off + 1..fill].iter().position(|&b| b == b'O') {
        Some(p) => page_off + 1 + p,
        None => fill,
    };

    oy.returned = next as i32;
    -((next - page_off) as i64)
}

/* sync the stream and get a page.  Keep trying until we find a page.
Suppress 'sync errors' after reporting the first.

return values:
-1) recapture (hole in data)
 0) need more data
 1) page returned

Returns pointers into buffered data; invalidated by next call to
_stream, _clear, _init, or _buffer */

/// Translation of `ogg_sync_pageout()`.
#[allow(dead_code)] /* (opusfile only uses it in op_test(), which isn't translated) */
pub(crate) fn ogg_sync_pageout(oy: &mut OggSyncState, og: &mut OggPage) -> i32 {
    if ogg_sync_check(oy) != 0 {
        return 0;
    }

    /* all we need to do is verify a page at the head of the stream
    buffer.  If it doesn't verify, we look for the next potential
    frame */

    loop {
        let ret = ogg_sync_pageseek(oy, Some(&mut *og));
        if ret > 0 {
            /* have a page */
            return 1;
        }
        if ret == 0 {
            /* need more data */
            return 0;
        }

        /* head did not start a synced page... skipped some bytes */
        if oy.unsynced == 0 {
            oy.unsynced = 1;
            return -1;
        }

        /* loop. keep looking */
    }
}

/* add the incoming page to the stream state; we decompose the page
into packet segments here as well. */

/// Translation of `ogg_stream_pagein()`.
pub(crate) fn ogg_stream_pagein(os: &mut OggStreamState, og: &OggPage) -> i32 {
    let header = &og.header;
    let mut body_off = 0usize;
    let mut bodysize = og.body.len() as i64;
    let mut segptr = 0usize;

    let version = ogg_page_version(og);
    let continued = ogg_page_continued(og);
    let mut bos = ogg_page_bos(og);
    let eos = ogg_page_eos(og);
    let granulepos = ogg_page_granulepos(og);
    let serialno = ogg_page_serialno(og);
    let pageno = ogg_page_pageno(og);
    let segments = header[26] as usize;

    if ogg_stream_check(os) != 0 {
        return -1;
    }

    /* clean up 'returned data' */
    {
        let lr = os.lacing_returned;
        let br = os.body_returned;

        /* body data */
        if br != 0 {
            os.body_fill -= br;
            if os.body_fill != 0 {
                os.body_data
                    .copy_within(br as usize..(br + os.body_fill) as usize, 0);
            }
            os.body_returned = 0;
        }

        if lr != 0 {
            /* segment table */
            if os.lacing_fill - lr != 0 {
                os.lacing_vals
                    .copy_within(lr as usize..os.lacing_fill as usize, 0);
                os.granule_vals
                    .copy_within(lr as usize..os.lacing_fill as usize, 0);
            }
            os.lacing_fill -= lr;
            os.lacing_packet -= lr;
            os.lacing_returned = 0;
        }
    }

    /* check the serial number */
    if serialno as i64 != os.serialno {
        return -1;
    }
    if version > 0 {
        return -1;
    }

    if os_lacing_expand(os, segments as i64 + 1) != 0 {
        return -1;
    }

    /* are we in sequence? */
    if pageno != os.pageno {
        /* unroll previous partial packet (if any) */
        for i in os.lacing_packet..os.lacing_fill {
            os.body_fill -= (os.lacing_vals[i as usize] & 0xff) as i64;
        }
        os.lacing_fill = os.lacing_packet;

        /* make a note of dropped data in segment table */
        if os.pageno != -1 {
            os.lacing_vals[os.lacing_fill as usize] = 0x400;
            os.lacing_fill += 1;
            os.lacing_packet += 1;
        }
    }

    /* are we a 'continued packet' page?  If so, we may need to skip
    some segments */
    if continued != 0
        && (os.lacing_fill < 1
            || (os.lacing_vals[(os.lacing_fill - 1) as usize] & 0xff) < 255
            || os.lacing_vals[(os.lacing_fill - 1) as usize] == 0x400)
    {
        bos = 0;
        while segptr < segments {
            let val = header[27 + segptr] as usize;
            body_off += val;
            bodysize -= val as i64;
            if val < 255 {
                segptr += 1;
                break;
            }
            segptr += 1;
        }
    }

    if bodysize != 0 {
        if os_body_expand(os, bodysize) != 0 {
            return -1;
        }
        let bf = os.body_fill as usize;
        os.body_data[bf..bf + bodysize as usize]
            .copy_from_slice(&og.body[body_off..body_off + bodysize as usize]);
        os.body_fill += bodysize;
    }

    {
        let mut saved: i64 = -1;
        while segptr < segments {
            let val = header[27 + segptr] as i32;
            let lf = os.lacing_fill as usize;
            os.lacing_vals[lf] = val;
            os.granule_vals[lf] = -1;

            if bos != 0 {
                os.lacing_vals[lf] |= 0x100;
                bos = 0;
            }

            if val < 255 {
                saved = os.lacing_fill;
            }

            os.lacing_fill += 1;
            segptr += 1;

            if val < 255 {
                os.lacing_packet = os.lacing_fill;
            }
        }

        /* set the granulepos on the last granuleval of the last full packet */
        if saved != -1 {
            os.granule_vals[saved as usize] = granulepos;
        }
    }

    if eos != 0 {
        os.e_o_s = 1;
        if os.lacing_fill > 0 {
            os.lacing_vals[(os.lacing_fill - 1) as usize] |= 0x200;
        }
    }

    os.pageno = pageno + 1;

    0
}

/* clear things to an initial state.  Good to call, eg, before seeking */
/// Translation of `ogg_sync_reset()`.
pub(crate) fn ogg_sync_reset(oy: &mut OggSyncState) -> i32 {
    if ogg_sync_check(oy) != 0 {
        return -1;
    }

    oy.fill = 0;
    oy.returned = 0;
    oy.unsynced = 0;
    oy.headerbytes = 0;
    oy.bodybytes = 0;
    0
}

/// Translation of `ogg_stream_reset()`.
pub(crate) fn ogg_stream_reset(os: &mut OggStreamState) -> i32 {
    if ogg_stream_check(os) != 0 {
        return -1;
    }

    os.body_fill = 0;
    os.body_returned = 0;

    os.lacing_fill = 0;
    os.lacing_packet = 0;
    os.lacing_returned = 0;

    os.e_o_s = 0;
    os.b_o_s = 0;
    os.pageno = -1;
    os.packetno = 0;
    os.granulepos = 0;

    0
}

/// Translation of `ogg_stream_reset_serialno()`.
pub(crate) fn ogg_stream_reset_serialno(os: &mut OggStreamState, serialno: i32) -> i32 {
    if ogg_stream_check(os) != 0 {
        return -1;
    }
    ogg_stream_reset(os);
    os.serialno = serialno as i64;
    0
}

fn packetout(os: &mut OggStreamState, op: Option<&mut OggPacket>, adv: bool) -> i32 {
    /* The last part of decode. We have the stream broken into packet
    segments.  Now we need to group them into packets (or return the
    out of sync markers) */

    let mut ptr = os.lacing_returned as usize;

    if os.lacing_packet <= ptr as i64 {
        return 0;
    }

    if os.lacing_vals[ptr] & 0x400 != 0 {
        /* we need to tell the codec there's a gap; it might need to
        handle previous packet dependencies. */
        os.lacing_returned += 1;
        os.packetno += 1;
        return -1;
    }

    if op.is_none() && !adv {
        return 1; /* just using peek as an inexpensive way
                  to ask if there's a whole packet
                  waiting */
    }

    /* Gather the whole packet. We'll have no holes or a partial packet */
    {
        let mut size = os.lacing_vals[ptr] & 0xff;
        let mut bytes = size as i64;
        let mut eos = os.lacing_vals[ptr] & 0x200; /* last packet of the stream? */
        let bos = os.lacing_vals[ptr] & 0x100; /* first packet of the stream? */

        while size == 255 {
            ptr += 1;
            let val = os.lacing_vals[ptr];
            size = val & 0xff;
            if val & 0x200 != 0 {
                eos = 0x200;
            }
            bytes += size as i64;
        }

        if let Some(op) = op {
            op.e_o_s = eos as i64;
            op.b_o_s = bos as i64;
            op.packet = os.body_returned as usize;
            op.packetno = os.packetno;
            op.granulepos = os.granule_vals[ptr];
            op.bytes = bytes;
        }

        if adv {
            os.body_returned += bytes;
            os.lacing_returned = ptr as i64 + 1;
            os.packetno += 1;
        }
    }
    1
}

/// Translation of `ogg_stream_packetout()`.
pub(crate) fn ogg_stream_packetout(os: &mut OggStreamState, op: Option<&mut OggPacket>) -> i32 {
    if ogg_stream_check(os) != 0 {
        return 0;
    }
    packetout(os, op, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The computed table is crctable.h's.
    #[test]
    fn crc_lookup_matches_crctable_h() {
        assert_eq!(CRC_LOOKUP[0][1], 0x04c11db7);
        assert_eq!(CRC_LOOKUP[0][255], 0xb1f740b4);
        assert_eq!(CRC_LOOKUP[7][248], 0xe1c4d9a5);
        assert_eq!(CRC_LOOKUP[7][255], 0x6760d264);
    }
}
