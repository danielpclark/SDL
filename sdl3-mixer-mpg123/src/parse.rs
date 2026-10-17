/*
    parse: spawned from common; clustering around stream/frame parsing

    copyright ?-2023 by the mpg123 project - free software under the terms of the LGPL 2.1
    see COPYING and AUTHORS files in distribution or http://mpg123.org
    initially written by Michael Hipp & Thomas Orgis
*/
// Modified 2026-10-17: translated into Rust from parse.c, parse.h and mpeghead.h in SDL_mixer 3.3.0's libmpg123 1.33.7 (see NOTICE).

//! Finding, checking and reading frames: the header decoding with free
//! format size guessing, the resync and junk skipping (with ID3v1, ID3v2,
//! APE and RIFF), the read-ahead check of the first header, and the
//! Xing/Info/LAME header with its gapless information.

use crate::frame::*;
use crate::getbits::getbits;
use crate::id3::parse_new_id3;
use crate::index::{fi_add, fi_next_cond};
use crate::layer1::do_layer1;
use crate::layer2::do_layer2;
use crate::layer3::do_layer3;
use crate::mpg123::*;
use crate::readers::*;

/* a limit for number of frames in a track; beyond that unsigned long may not be enough to hold byte addresses */
const TRACK_MAX_FRAMES: u64 = u64::MAX / 4 / 1152;

/*
    mpeghead: the bits of an MPEG frame header

    copyright ?-2011 by the mpg123 project - free software under the terms of the LGPL 2.1
    see COPYING and AUTHORS files in distribution or http://mpg123.org
    initially written by Michael Hipp & Thomas Orgis (from parse.c)
*/

/* 11111111 11100000 00000000 00000000 */
const HDR_SYNC: u64 = 0xffe00000;
/* 00000000 00011000 00000000 00000000 */
const HDR_VERSION: u64 = 0x00180000;
const fn hdr_version_val(h: u64) -> u64 {
    (h & HDR_VERSION) >> 19
}
/* 00000000 00000110 00000000 00000000 */
const HDR_LAYER: u64 = 0x00060000;
const fn hdr_layer_val(h: u64) -> u64 {
    (h & HDR_LAYER) >> 17
}
/* 00000000 00000001 00000000 00000000 */
const HDR_CRC: u64 = 0x00010000;
const fn hdr_crc_val(h: u64) -> u64 {
    (h & HDR_CRC) >> 16
}
/* 00000000 00000000 11110000 00000000 */
const HDR_BITRATE: u64 = 0x0000f000;
const fn hdr_bitrate_val(h: u64) -> u64 {
    (h & HDR_BITRATE) >> 12
}
/* 00000000 00000000 00001100 00000000 */
const HDR_SAMPLERATE: u64 = 0x00000c00;
const fn hdr_samplerate_val(h: u64) -> u64 {
    (h & HDR_SAMPLERATE) >> 10
}
/* 00000000 00000000 00000010 00000000 */
const HDR_PADDING: u64 = 0x00000200;
const fn hdr_padding_val(h: u64) -> u64 {
    (h & HDR_PADDING) >> 9
}
/* 00000000 00000000 00000001 00000000 */
const HDR_PRIVATE: u64 = 0x00000100;
const fn hdr_private_val(h: u64) -> u64 {
    (h & HDR_PRIVATE) >> 8
}
/* 00000000 00000000 00000000 11000000 */
const HDR_CHANNEL: u64 = 0x000000c0;
const fn hdr_channel_val(h: u64) -> u64 {
    (h & HDR_CHANNEL) >> 6
}
/* 00000000 00000000 00000000 00110000 */
const HDR_CHANEX: u64 = 0x00000030;
const fn hdr_chanex_val(h: u64) -> u64 {
    (h & HDR_CHANEX) >> 4
}
/* 00000000 00000000 00000000 00001000 */
const HDR_COPYRIGHT: u64 = 0x00000008;
const fn hdr_copyright_val(h: u64) -> u64 {
    (h & HDR_COPYRIGHT) >> 3
}
/* 00000000 00000000 00000000 00000100 */
const HDR_ORIGINAL: u64 = 0x00000004;
const fn hdr_original_val(h: u64) -> u64 {
    (h & HDR_ORIGINAL) >> 2
}
/* 00000000 00000000 00000000 00000011 */
const HDR_EMPHASIS: u64 = 0x00000003;
const fn hdr_emphasis_val(h: u64) -> u64 {
    h & HDR_EMPHASIS
}

/*
    A generic mask for telling if a header is somewhat valid for the current stream.
    Meaning: Most basic info is not allowed to change.
    Checking of channel count needs to be done, too, though. So,
    if channel count matches, frames are decoded the same way: frame buffers and decoding
    routines can stay the same, especially frame buffers (think spf * channels!).
*/
const HDR_CMPMASK: u64 = HDR_SYNC | HDR_VERSION | HDR_LAYER | HDR_SAMPLERATE;

/* A stricter mask, for matching free format headers. */
const HDR_SAMEMASK: u64 =
    HDR_SYNC | HDR_VERSION | HDR_LAYER | HDR_BITRATE | HDR_SAMPLERATE | HDR_CHANNEL;

/* PARSE_GOOD and PARSE_BAD have to be 1 and 0 (TRUE and FALSE), others can vary. */
const PARSE_MORE: i32 = MPG123_NEED_MORE;
const PARSE_ERR: i32 = MPG123_ERR;
const PARSE_END: i32 = 10; /* No more audio data to find. */
const PARSE_GOOD: i32 = 1; /* Everything's fine. */
const PARSE_BAD: i32 = 0; /* Not fine (invalid data). */
const PARSE_RESYNC: i32 = 2; /* Header not good, go into resync. */
const PARSE_AGAIN: i32 = 3; /* Really start over, throw away and read a new header, again. */

/* bitrates for [mpeg1/2][layer] */
static TABSEL_123: [[[i32; 16]; 3]; 2] = [
    [
        [
            0, 32, 64, 96, 128, 160, 192, 224, 256, 288, 320, 352, 384, 416, 448, 0,
        ],
        [
            0, 32, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384, 0,
        ],
        [
            0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 0,
        ],
    ],
    [
        [
            0, 32, 48, 56, 64, 80, 96, 112, 128, 144, 160, 176, 192, 224, 256, 0,
        ],
        [
            0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160, 0,
        ],
        [
            0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160, 0,
        ],
    ],
];

static FREQS: [i64; 9] = [44100, 48000, 32000, 22050, 24000, 16000, 11025, 12000, 8000];

/* These two are to be replaced by one function that gives all the frame parameters (for outsiders).*/
/* Those functions are unsafe regarding bad arguments (inside the mpg123_handle), but just returning anything would also be unsafe, the caller code has to be trusted. */

/// Translation of `INT123_frame_bitrate()`.
pub(crate) fn frame_bitrate(fr: &Mpg123Handle<'_>) -> i32 {
    TABSEL_123[fr.hdr.lsf as usize][(fr.hdr.lay - 1) as usize][fr.hdr.bitrate_index as usize]
}

/// Translation of `INT123_frame_freq()`.
pub(crate) fn frame_freq(fr: &Mpg123Handle<'_>) -> i64 {
    FREQS[fr.hdr.sampling_frequency as usize]
}

/* compiler is smart enought to inline this one or should I really do it as macro...? */
fn head_check(head: u64) -> bool {
    if ((head & HDR_SYNC) != HDR_SYNC)
        ||
        /* layer: 01,10,11 is 1,2,3; 00 is reserved */
        (hdr_layer_val(head) == 0)
        ||
        /* 1111 means bad bitrate */
        (hdr_bitrate_val(head) == 0xf)
        ||
        /* sampling freq: 11 is reserved */
        (hdr_samplerate_val(head) == 0x3)
    /* here used to be a mpeg 2.5 check... re-enabled 2.5 decoding due to lack of evidence that it is really not good */
    {
        false
    }
    /* if no check failed, the header is valid (hopefully)*/
    else {
        true
    }
}

/* This is moderately sized buffers. Int offset is enough. */
fn bit_read_long(buf: &[u8], offset: &mut usize) -> u64 {
    let val = /* 32 bit value */
        ((buf[*offset] as u64) << 24)
        | ((buf[*offset + 1] as u64) << 16)
        | ((buf[*offset + 2] as u64) << 8)
        | (buf[*offset + 3] as u64);
    *offset += 4;
    val
}

fn bit_read_short(buf: &[u8], offset: &mut usize) -> u16 {
    let val = /* 16 bit value */
        ((buf[*offset] as u16) << 8)
        | (buf[*offset + 1] as u16);
    *offset += 2;
    val
}

/// `sscanf(s, "%u.%u%s", &major, &minor, rest)`, as far as the return
/// value and the two numbers go (the rest isn't used).
fn scan_lame_version(s: &[u8]) -> (i32, u32, u32) {
    // %u: like strtoul(), after skipping white space
    fn scan_u(s: &[u8], mut i: usize) -> Option<(u32, usize)> {
        while i < s.len() && (s[i] == b' ' || (b'\t'..=b'\r').contains(&s[i])) {
            i += 1;
        }
        let mut neg = false;
        if i < s.len() && (s[i] == b'+' || s[i] == b'-') {
            neg = s[i] == b'-';
            i += 1;
        }
        let start = i;
        let mut v: u32 = 0;
        while i < s.len() && s[i].is_ascii_digit() {
            v = v.wrapping_mul(10).wrapping_add((s[i] - b'0') as u32);
            i += 1;
        }
        if i == start {
            return None;
        }
        Some((if neg { v.wrapping_neg() } else { v }, i))
    }
    let s = &s[..s.iter().position(|&c| c == 0).unwrap_or(s.len())];
    let Some((major, i)) = scan_u(s, 0) else {
        return (if s.is_empty() { -1 } else { 0 }, 0, 0);
    };
    if i >= s.len() || s[i] != b'.' {
        return (1, major, 0);
    }
    let Some((minor, mut i)) = scan_u(s, i + 1) else {
        return (1, major, 0);
    };
    while i < s.len() && (s[i] == b' ' || (b'\t'..=b'\r').contains(&s[i])) {
        i += 1;
    }
    (if i < s.len() { 3 } else { 2 }, major, minor)
}

fn check_lame_tag(fr: &mut Mpg123Handle<'_>) -> i32 {
    /*
        going to look for Xing or Info at some position after the header
                                           MPEG 1  MPEG 2/2.5 (LSF)
        Stereo, Joint Stereo, Dual Channel  32      17
        Mono                                17       9
    */
    let mut lame_offset: usize = if fr.stereo == 2 {
        if fr.hdr.lsf != 0 {
            17
        } else {
            32
        }
    } else if fr.hdr.lsf != 0 {
        9
    } else {
        17
    };

    'yes: {
        if fr.p.flags & MPG123_IGNORE_INFOFRAME != 0 {
            return 0; // check_lame_tag_no
        }

        /*
            Note: CRC or not, that does not matter here.
            But, there is any combination of Xing flags in the wild. There are headers
            without the search index table! I cannot assume a reasonable minimal size
            for the actual data, have to check if each byte of information is present.
            But: 4 B Info/Xing + 4 B flags is bare minimum.
        */
        if (fr.hdr.framesize as i64) < lame_offset as i64 + 8 {
            return 0;
        }

        // (the frame body, fr->bsbuf)
        let bsbuf = fr.bsbuf;
        let b = |fr: &Mpg123Handle<'_>, i: usize| fr.bsspace[bsbuf + i];

        /* only search for tag when all zero before it (apart from checksum) */
        for i in 2..lame_offset {
            if b(fr, i) != 0 {
                return 0;
            }
        }

        if b(fr, lame_offset) == b'I'
            && b(fr, lame_offset + 1) == b'n'
            && b(fr, lame_offset + 2) == b'f'
            && b(fr, lame_offset + 3) == b'o'
        {
            /* We still have to see what there is */
        } else if b(fr, lame_offset) == b'X'
            && b(fr, lame_offset + 1) == b'i'
            && b(fr, lame_offset + 2) == b'n'
            && b(fr, lame_offset + 3) == b'g'
        {
            fr.vbr = MPG123_VBR; /* Xing header means always VBR */
        } else {
            return 0;
        }

        /* we have one of these headers... */
        lame_offset += 4;
        let xing_flags = bit_read_long(&fr.bsspace[bsbuf..], &mut lame_offset);

        /* From now on, I have to carefully check if the announced data is actually
        there! I'm always returning 'yes', though.  */
        macro_rules! check_bytes_left {
            ($n:expr) => {
                if (fr.hdr.framesize as i64) < (lame_offset + $n) as i64 {
                    break 'yes;
                }
            };
        }
        if xing_flags & 1 != 0 {
            /* total bitstream frames */
            check_bytes_left!(4);
            let long_tmp = bit_read_long(&fr.bsspace[bsbuf..], &mut lame_offset);
            if fr.p.flags & MPG123_IGNORE_STREAMLENGTH != 0 {
                // Note: Ignoring Xing frames because of MPG123_IGNORE_STREAMLENGTH
            } else {
                /* Check for endless stream, but: TRACK_MAX_FRAMES sensible at all? */
                fr.track_frames = if long_tmp > TRACK_MAX_FRAMES {
                    0
                } else {
                    long_tmp as i64
                };
                /* All or nothing: Only if encoder delay/padding is known, we'll cut
                samples for gapless. */
                if fr.p.flags & MPG123_GAPLESS != 0 {
                    frame_gapless_init(fr, fr.track_frames, 0, 0);
                }
            }
        }
        if xing_flags & 0x2 != 0 {
            /* total bitstream bytes */
            check_bytes_left!(4);
            let long_tmp = bit_read_long(&fr.bsspace[bsbuf..], &mut lame_offset);
            if fr.p.flags & MPG123_IGNORE_STREAMLENGTH != 0 {
                // Note: Ignoring Xing bytes because of MPG123_IGNORE_STREAMLENGTH
            } else {
                /* The Xing bitstream length, at least as interpreted by the Lame
                encoder, encompasses all data from the Xing header frame on,
                ignoring leading ID3v2 data. Trailing tags (ID3v1) seem to be
                included, though. */
                if fr.rdat.filelen < 1 {
                    fr.rdat.filelen = (long_tmp as i64).wrapping_add(fr.audio_start);
                /* Overflow? */
                } else {
                    // (only a message about the size difference)
                }
            }
        }
        if xing_flags & 0x4 != 0 {
            /* TOC */
            check_bytes_left!(100);
            let toc: [u8; 100] = fr.bsspace[bsbuf + lame_offset..bsbuf + lame_offset + 100]
                .try_into()
                .unwrap();
            frame_fill_toc(fr, &toc);
            lame_offset += 100;
        }
        if xing_flags & 0x8 != 0 {
            /* VBR quality */
            check_bytes_left!(4);
            let _long_tmp = bit_read_long(&fr.bsspace[bsbuf..], &mut lame_offset);
        }
        /*
            Either zeros/nothing, or:
                0-8: LAME3.90a
                9: revision/VBR method
                10: lowpass
                11-18: ReplayGain
                19: encoder flags
                20: ABR
                21-23: encoder delays
        */
        check_bytes_left!(24); /* I'm interested in 24 B of extra info. */
        if b(fr, lame_offset) != 0 {
            let mut replay_gain: [f32; 2] = [0.0, 0.0];
            let mut gain_offset: f32 = 0.0; /* going to be +6 for old lame that used 83dB */
            let mut nb = [0u8; 10];
            nb[..9].copy_from_slice(&fr.bsspace[bsbuf + lame_offset..bsbuf + lame_offset + 9]);
            nb[9] = 0;
            if &nb[..4] == b"LAME" {
                /* Lame versions before 3.95.1 used 83 dB reference level, later
                versions 89 dB. We stick with 89 dB as being "normal", adding
                6 dB. */
                let (n, major, minor) = scan_lame_version(&nb[4..]);
                if n >= 2 {
                    /* We cannot detect LAME 3.95 reliably (same version string as
                    3.95.1), so this is a blind spot. Everything < 3.95 is safe,
                    though. */
                    if major < 3 || (major == 3 && minor < 95) {
                        gain_offset = 6.0;
                    }
                }
            }
            lame_offset += 9; /* 9 in */

            /* The 4 big bits are tag revision, the small bits vbr method. */
            let lame_vbr = b(fr, lame_offset) & 15;
            lame_offset += 1; /* 10 in */
            match lame_vbr {
                /* from rev1 proposal... not sure if all good in practice */
                1 | 8 => fr.vbr = MPG123_CBR,
                2 | 9 => fr.vbr = MPG123_ABR,
                _ => fr.vbr = MPG123_VBR, /* 00==unknown is taken as VBR */
            }
            lame_offset += 1; /* 11 in, skipping lowpass filter value */

            /* ReplayGain peak ampitude, 32 bit float -- why did I parse it as int
            before??	Ah, yes, Lame seems to store it as int since some day in 2003;
            I've only seen zeros anyway until now, bah! */
            // (a non-zero peak isn't used: peak = 0 until better times arrived)
            lame_offset += 4; /* 15 in */

            /* ReplayGain values - lame only writes radio mode gain...
            16bit gain, 3 bits name, 3 bits originator, sign (1=-, 0=+),
            dB value*10 in 9 bits (fixed point) ignore the setting if name or
            originator == 000!
            radio      0 0 1 0 1 1 1 0 0 1 1 1 1 1 0 1
            audiophile 0 1 0 0 1 0 0 0 0 0 0 1 0 1 0 0 */
            for _i in 0..2 {
                let mut gt = b(fr, lame_offset) >> 5;
                let origin = (b(fr, lame_offset) >> 2) & 0x7;
                let factor: f32 = if b(fr, lame_offset) & 0x2 != 0 {
                    -0.1
                } else {
                    0.1
                };
                let gain = bit_read_short(&fr.bsspace[bsbuf..], &mut lame_offset) & 0x1ff; /* 19 in (2 cycles) */
                if origin == 0 || !(1..=2).contains(&gt) {
                    continue;
                }

                gt -= 1;
                replay_gain[gt as usize] = factor * gain as f32;
                /* Apply gain offset for automatic origin. */
                if origin == 3 {
                    replay_gain[gt as usize] += gain_offset;
                }
            }
            for i in 0..2 {
                if fr.rva.level[i] <= 0 {
                    fr.rva.peak[i] = 0.0; /* TODO: use parsed peak? */
                    fr.rva.gain[i] = replay_gain[i];
                    fr.rva.level[i] = 0;
                }
            }

            lame_offset += 1; /* 20 in, skipping encoding flags byte */

            /* ABR rate */
            if fr.vbr == MPG123_ABR {
                fr.abr_rate = b(fr, lame_offset) as i32;
            }
            lame_offset += 1; /* 21 in */

            /* Encoder delay and padding, two 12 bit values
            ... lame does write them from int. */
            let pad_in: i64 = (((b(fr, lame_offset) as i32) << 4)
                | ((b(fr, lame_offset + 1) as i32) >> 4)) as i64;
            let pad_out: i64 = ((((b(fr, lame_offset + 1) as i32) << 8)
                | (b(fr, lame_offset + 2) as i32))
                & 0xfff) as i64;
            /* Store even if libmpg123 does not do gapless decoding itself. */
            fr.enc_delay = pad_in as i32;
            fr.enc_padding = pad_out as i32;
            if fr.p.flags & MPG123_GAPLESS != 0 {
                frame_gapless_init(fr, fr.track_frames, pad_in, pad_out);
            }
            /* final: 24 B LAME data */
        }
    }

    // check_lame_tag_yes:
    /* switch buffer back ... */
    fr.bsbuf = fr.bsnum as usize * BSSPACE_SIZE + 512;
    fr.bsnum = (fr.bsnum + 1) & 1;
    1
}

/* Just tell if the header is some mono. */
fn header_mono(newhead: u64) -> bool {
    hdr_channel_val(newhead) == MPG_MD_MONO as u64
}

/* true if the two headers will work with the same decoding routines */
fn head_compatible(fred: u64, bret: u64) -> bool {
    (fred & HDR_CMPMASK) == (bret & HDR_CMPMASK) && header_mono(fred) == header_mono(bret)
}

fn halfspeed_prepare(fr: &mut Mpg123Handle<'_>) {
    /* save for repetition */
    if fr.p.halfspeed != 0 && fr.hdr.lay == 3 {
        let n = fr.hdr.ssize as usize;
        let at = fr.bsbuf;
        fr.ssave[..n].copy_from_slice(&fr.bsspace[at..at + n]);
    }
}

/* If this returns 1, the next frame is the repetition. */
fn halfspeed_do(fr: &mut Mpg123Handle<'_>) -> i32 {
    /* Speed-down hack: Play it again, Sam (the frame, I mean). */
    if fr.p.halfspeed != 0 {
        if fr.halfphase != 0 {
            /* repeat last frame */
            fr.to_decode = 1;
            fr.to_ignore = 1;
            fr.halfphase -= 1;
            set_pointer(fr, 0, 0);
            if fr.hdr.lay == 3 {
                let n = fr.hdr.ssize as usize;
                let at = fr.bsbuf;
                fr.bsspace[at..at + n].copy_from_slice(&fr.ssave[..n]);
            }
            if fr.hdr.error_protection != 0 {
                fr.crc = getbits(fr, 16); /* skip crc */
            }
            return 1;
        } else {
            fr.halfphase = (fr.p.halfspeed - 1) as i32;
        }
    }
    0
}

/*
    Temporary macro until we got this worked out.
    Idea is to filter out special return values that shall trigger direct jumps to end / resync / read again.
    Particularily, the generic ret==PARSE_BAD==0 and ret==PARSE_GOOD==1 are not affected.
*/
enum Jump {
    None,
    Bad(i32),
    Again,
    Resync,
}

fn jump_conclusion(ret: i32) -> Jump {
    if ret < 0 {
        Jump::Bad(ret)
    } else if ret == PARSE_AGAIN {
        Jump::Again
    } else if ret == PARSE_RESYNC {
        Jump::Resync
    } else if ret == PARSE_END {
        Jump::Bad(0)
    } else {
        Jump::None
    }
}

/*
    That's a big one: read the next frame. 1 is success, <= 0 is some error
    Special error READER_MORE means: Please feed more data and try again.
*/
/// Translation of `INT123_read_frame()`.
pub(crate) fn read_frame(fr: &mut Mpg123Handle<'_>) -> i32 {
    /* TODO: rework this thing */
    let mut freeformat_count = 0;
    let mut newhead: u64 = 0;
    // Start with current frame header state as copy for roll-back ability.
    let mut nhdr = fr.hdr;
    let mut ret;
    /* stuff that needs resetting if complete frame reading fails */
    let oldphase = fr.halfphase;

    /* The counter for the search-first-header loop.
    It is persistent outside the loop to prevent seemingly endless loops
    when repeatedly headers are found that do not have valid followup headers. */
    let mut headcount: i64 = 0;

    fr.fsizeold = fr.hdr.framesize; /* for Layer3 */

    if halfspeed_do(fr) == 1 {
        return 1;
    }

    /* From now on, old frame data is tainted by parsing attempts. */
    // Handling premature effects of decode_header now, more decoupling would be welcome.
    fr.to_decode = 0;
    fr.to_ignore = 0;

    if fr.p.flags & MPG123_NO_FRANKENSTEIN != 0
        && ((fr.track_frames > 0 && fr.num >= fr.track_frames - 1)
            || (fr.gapless_frames > 0 && fr.num >= fr.gapless_frames - 1))
    {
        return 0;
    }

    let mut resync = false;
    ret = 'bad: loop {
        if !resync {
            // read_again:
            /* In case we are looping to find a valid frame, discard any buffered data before the current position.
            This is essential to prevent endless looping, always going back to the beginning when feeder buffer is exhausted. */
            rd_forget(fr);

            ret = rd_head_read(fr, &mut newhead);
            if ret <= 0 {
                break 'bad ret;
            }
        }
        resync = false;

        // init_resync:

        if fr.firsthead == 0 && !head_check(newhead) {
            ret = skip_junk(fr, &mut newhead, &mut headcount, &mut nhdr);
            match jump_conclusion(ret) {
                Jump::Bad(r) => break 'bad r,
                Jump::Again => continue,
                Jump::Resync => {
                    resync = true;
                    continue;
                }
                Jump::None => {}
            }
        }

        ret = if head_check(newhead) { 1 } else { 0 };
        if ret != 0 {
            ret = decode_header(fr, &mut nhdr, newhead, &mut freeformat_count);
        }

        match jump_conclusion(ret) {
            /* That only continues for ret == PARSE_BAD or PARSE_GOOD. */
            Jump::Bad(r) => break 'bad r,
            Jump::Again => continue,
            Jump::Resync => {
                resync = true;
                continue;
            }
            Jump::None => {}
        }
        if ret == PARSE_BAD {
            /* Header was not good. */
            ret = wetwork(fr, &mut newhead); /* Messy stuff, handle junk, resync ... */
            match jump_conclusion(ret) {
                Jump::Bad(r) => break 'bad r,
                Jump::Again => continue,
                Jump::Resync => {
                    resync = true;
                    continue;
                }
                Jump::None => {}
            }
            /* Normally, we jumped already. If for some reason everything's fine to continue, do continue. */
            if ret != PARSE_GOOD {
                break 'bad ret;
            }
        }

        if fr.firsthead == 0 {
            ret = if fr.p.flags & MPG123_NO_READAHEAD != 0 {
                PARSE_GOOD
            } else {
                do_readahead(fr, &nhdr, newhead)
            };
            /* readahead can fail mit NEED_MORE, in which case we must also make the just read header available again for next go */
            if ret < 0 {
                rd_back_bytes(fr, 4);
            }
            match jump_conclusion(ret) {
                Jump::Bad(r) => break 'bad r,
                Jump::Again => continue,
                Jump::Resync => {
                    resync = true;
                    continue;
                }
                Jump::None => {}
            }
        }

        /* Now we should have our valid header and proceed to reading the frame. */

        if fr.p.flags & MPG123_NO_FRANKENSTEIN != 0
            && fr.firsthead != 0
            && !head_compatible(fr.firsthead, newhead)
        {
            return 0;
        }

        /* if filepos is invalid, so is framepos */
        let framepos = rd_tell(fr) - 4;
        /* flip/init buffer for Layer 3 */
        {
            let newbuf = fr.bsnum as usize * BSSPACE_SIZE + 512;
            /* read main data into memory */
            let n = nhdr.framesize as usize;
            let mut body = std::mem::take(&mut fr.bsspace);
            ret = rd_read_frame_body(fr, &mut body[newbuf..newbuf + n]);
            fr.bsspace = body;
            if ret < 0 {
                /* if failed: flip back */
                break 'bad ret;
            }
            fr.bsbufold = fr.bsbuf;
            fr.bsbuf = newbuf;
        }
        fr.bsnum = (fr.bsnum + 1) & 1;

        // We read the frame body, time to apply the matching header.
        // Even if erroring out later, the header state needs to match the body.
        apply_header(fr, &nhdr);

        if fr.firsthead == 0 {
            fr.firsthead = newhead; /* _now_ it's time to store it... the first real header */
            /* This is the first header of our current stream segment.
            It is only the actual first header of the whole stream when fr->num is still below zero!
            Think of resyncs where firsthead has been reset for format flexibility. */
            if fr.num < 0 {
                fr.audio_start = framepos;
                /* Only check for LAME  tag at beginning of whole stream
                ... when there indeed is one in between, it's the user's problem. */
                if fr.hdr.lay == 3 && check_lame_tag(fr) == 1 {
                    /* ...in practice, Xing/LAME tags are layer 3 only. */
                    rd_forget(fr);

                    fr.oldhead = 0;
                    continue; // goto read_again
                }
                /* now adjust volume */
                do_rva(fr);
            }
        }

        set_pointer(fr, 0, 0);

        // No use of nhdr from here on. It is fr->hdr now!

        /* Question: How bad does the floating point value get with repeated recomputation?
        Also, considering that we can play the file or parts of many times. */
        fr.mean_frames += 1;
        if fr.mean_frames != 0 {
            fr.mean_framesize = ((fr.mean_frames - 1) as f64 * fr.mean_framesize + compute_bpf(fr))
                / fr.mean_frames as f64;
        }
        fr.num += 1; /* 0 for first frame! */
        if fr.state_flags & FRAME_FRANKENSTEIN == 0
            && ((fr.track_frames > 0 && fr.num >= fr.track_frames)
                || (fr.gapless_frames > 0 && fr.num >= fr.gapless_frames))
        {
            fr.state_flags |= FRAME_FRANKENSTEIN;
        }

        halfspeed_prepare(fr);

        /* index the position */
        fr.input_offset = framepos;
        /* Keep track of true frame positions in our frame index.
        but only do so when we are sure that the frame number is accurate... */
        if (fr.state_flags & FRAME_ACCURATE) != 0 && fi_next_cond(&fr.index, fr.num) {
            fi_add(&mut fr.index, framepos);
        }

        if fr.silent_resync > 0 {
            fr.silent_resync -= 1;
        }

        rd_forget(fr);

        fr.to_decode = 1;
        fr.to_ignore = 1;
        if fr.hdr.error_protection != 0 {
            fr.crc = getbits(fr, 16); /* skip crc */
        }

        /*
            Let's check for header change after deciding that the new one is good
            and actually having read a frame.

            header_change > 1: decoder structure has to be updated
            Preserve header_change value from previous runs if it is serious.
            If we still have a big change pending, it should be dealt with outside,
            fr->header_change set to zero afterwards.
        */
        if fr.header_change < 2 {
            fr.header_change = 2; /* output format change is possible... */
            if fr.oldhead != 0 {
                /* check a following header for change */
                if fr.oldhead == newhead {
                    fr.header_change = 0;
                } else
                /* Headers that match in this test behave the same for the outside world.
                namely: same decoding routines, same amount of decoded data. */
                if head_compatible(fr.oldhead, newhead) {
                    fr.header_change = 1;
                } else {
                    fr.state_flags |= FRAME_FRANKENSTEIN;
                }
            } else if fr.firsthead != 0 && !head_compatible(fr.firsthead, newhead) {
                fr.state_flags |= FRAME_FRANKENSTEIN;
            }
        }

        fr.oldhead = newhead;

        return 1;
    };
    // read_frame_bad:
    /* Also if we searched for valid data in vein, we can forget skipped data.
    Otherwise, the feeder would hold every dead old byte in memory until the first valid frame! */
    rd_forget(fr);

    fr.silent_resync = 0;
    if fr.err == MPG123_OK {
        fr.err = MPG123_ERR_READER;
    }
    fr.halfphase = oldphase;
    /* That return code might be inherited from some feeder action, or reader error. */
    ret
}

/*
 * read ahead and find the next MPEG header, to guess framesize
 * return value: success code
 *  PARSE_GOOD: found a valid frame size (stored in the handle).
 * <0: error codes, possibly from feeder buffer (NEED_MORE)
 *  PARSE_BAD: cannot get the framesize for some reason and shall silentry try the next possible header (if this is no free format stream after all...)
 */
fn guess_freeformat_framesize(fr: &mut Mpg123Handle<'_>, oldhead: u64, framesize: &mut i32) -> i32 {
    let mut head: u64 = 0;
    if fr.rdat.flags & (READER_SEEKABLE | READER_BUFFERED) == 0 {
        // Cannot look for freeformat frame size with non-seekable and non-buffered stream!
        return PARSE_BAD;
    }
    let ret = rd_head_read(fr, &mut head);
    if ret <= 0 {
        return ret;
    }

    /* We are already 4 bytes into it */
    let mut i = 4;
    while i < MAXFRAMESIZE as i32 + 4 {
        let ret = rd_head_shift(fr, &mut head);
        if ret <= 0 {
            return ret;
        }

        /* No head_check needed, the mask contains all relevant bits. */
        if (head & HDR_SAMEMASK) == (oldhead & HDR_SAMEMASK) {
            rd_back_bytes(fr, (i + 1) as i64);
            *framesize = i - 3;
            return PARSE_GOOD; /* Success! */
        }
        i += 1;
    }
    rd_back_bytes(fr, i as i64);
    PARSE_BAD
}

/*
 * decode a header and write the information
 * into the frame structure
 * Return values are compatible with those of INT123_read_frame, namely:
 *  1: success
 *  0: no valid header
 * <0: some error
 * You are required to do a head_check() before calling!
 *
 * This now only operates on a frame header struct, not the full frame structure.
 * The scope is limited to parsing header information and determining the size of
 * the frame body to read. Everything else belongs into a later stage of applying
 * header information to the main decoder frame structure.
 */
fn decode_header(
    fr: &mut Mpg123Handle<'_>,
    fh: &mut FrameHeader,
    newhead: u64,
    freeformat_count: &mut i32,
) -> i32 {
    /* For some reason, the layer and sampling freq settings used to be wrapped
    in a weird conditional including MPG123_NO_RESYNC. What was I thinking?
    This information has to be consistent. */
    fh.lay = 4 - hdr_layer_val(newhead) as i32;

    if hdr_version_val(newhead) & 0x2 != 0 {
        fh.lsf = if hdr_version_val(newhead) & 0x1 != 0 {
            0
        } else {
            1
        };
        fh.mpeg25 = 0;
        fh.sampling_frequency = hdr_samplerate_val(newhead) as i32 + (fh.lsf * 3);
    } else {
        fh.lsf = 1;
        fh.mpeg25 = 1;
        fh.sampling_frequency = 6 + hdr_samplerate_val(newhead) as i32;
    }

    fh.error_protection = (hdr_crc_val(newhead) ^ 0x1) as i32;
    fh.bitrate_index = hdr_bitrate_val(newhead) as i32;
    fh.padding = hdr_padding_val(newhead) as i32;
    fh.extension = hdr_private_val(newhead) as i32;
    fh.mode = hdr_channel_val(newhead) as i32;
    fh.mode_ext = hdr_chanex_val(newhead) as i32;
    fh.copyright = hdr_copyright_val(newhead) as i32;
    fh.original = hdr_original_val(newhead) as i32;
    fh.emphasis = hdr_emphasis_val(newhead) as i32;
    fh.freeformat = (newhead & HDR_BITRATE == 0) as i32;

    /* we can't use tabsel_123 for freeformat, so trying to guess framesize... */
    if fh.freeformat != 0 {
        /* when we first encounter the frame with freeformat, guess framesize */
        if fh.freeformat_framesize < 0 {
            if fr.p.flags & MPG123_NO_READAHEAD != 0 {
                // Got no free-format frame size and am not allowed to read ahead.
                return PARSE_BAD;
            }
            *freeformat_count += 1;
            if *freeformat_count > 5 {
                // You fooled me too often. Refusing to guess free format frame size _again_.
                return PARSE_BAD;
            }
            let ret = guess_freeformat_framesize(fr, newhead, &mut fh.framesize);
            if ret == PARSE_GOOD {
                fh.freeformat_framesize = fh.framesize - fh.padding;
            } else {
                return ret;
            }
        }
        /* freeformat should be CBR, so the same framesize can be used at the 2nd reading or later */
        else {
            fh.framesize = fh.freeformat_framesize + fh.padding;
        }
    }
    match fh.lay {
        1 => {
            if fh.freeformat == 0 {
                let mut fs: i64 =
                    TABSEL_123[fh.lsf as usize][0][fh.bitrate_index as usize] as i64 * 12000;
                fs /= FREQS[fh.sampling_frequency as usize];
                fs = ((fs + fh.padding as i64) << 2) - 4;
                fh.framesize = fs as i32;
            }
        }
        2 => {
            if fh.freeformat == 0 {
                let mut fs: i64 =
                    TABSEL_123[fh.lsf as usize][1][fh.bitrate_index as usize] as i64 * 144000;
                fs /= FREQS[fh.sampling_frequency as usize];
                fs += fh.padding as i64 - 4;
                fh.framesize = fs as i32;
            }
        }
        3 => {
            if fh.lsf != 0 {
                fh.ssize = if fh.mode == MPG_MD_MONO { 9 } else { 17 };
            } else {
                fh.ssize = if fh.mode == MPG_MD_MONO { 17 } else { 32 };
            }

            if fh.error_protection != 0 {
                fh.ssize += 2;
            }

            if fh.freeformat == 0 {
                let mut fs: i64 =
                    TABSEL_123[fh.lsf as usize][2][fh.bitrate_index as usize] as i64 * 144000;
                fs /= FREQS[fh.sampling_frequency as usize] << fh.lsf;
                fs += fh.padding as i64 - 4;
                fh.framesize = fs as i32;
            }
            if fh.framesize < fh.ssize {
                // Frame smaller than mandatory side info (%i < %i)!
                return PARSE_BAD;
            }
        }
        _ => {
            // Layer type %i not supported in this build!
            return PARSE_BAD;
        }
    }
    if fh.framesize > MAXFRAMESIZE as i32 {
        // Frame size too big: %d
        return PARSE_BAD;
    }
    PARSE_GOOD
}

// Apply decoded header structure to frame struct, including
// main decoder function pointer.
fn apply_header(fr: &mut Mpg123Handle<'_>, hdr: &FrameHeader) {
    // copy the whole struct, do some postprocessing
    fr.hdr = *hdr;
    fr.stereo = if fr.hdr.mode == MPG_MD_MONO { 1 } else { 2 };
    match fr.hdr.lay {
        1 => {
            fr.spf = 384;
            fr.do_layer = Some(do_layer1);
        }
        2 => {
            fr.spf = 1152;
            fr.do_layer = Some(do_layer2);
        }
        3 => {
            fr.spf = if fr.hdr.lsf != 0 { 576 } else { 1152 }; /* MPEG 2.5 implies LSF.*/
            fr.do_layer = Some(do_layer3);
        }
        _ => {
            // No error checking/message here, been done in decode_header().
            fr.spf = 0;
            fr.do_layer = None;
        }
    }
}

/* Prepare for bit reading. Two stages:
  0. Layers 1 and 2, side info for layer 3
  1. Second call for possible bit reservoir for layer 3 part 2,3.
     This overwrites side info needed for stage 0.

  Continuing to read bits after layer 3 side info shall fail unless
  INT123_set_pointer() is called to refresh things.
*/
/// Translation of `INT123_set_pointer()`.
pub(crate) fn set_pointer(fr: &mut Mpg123Handle<'_>, part2: i32, backstep: i64) {
    fr.bitindex = 0;
    if fr.hdr.lay == 3 {
        if part2 != 0 {
            fr.wordpointer = (fr.bsbuf as i64 + fr.hdr.ssize as i64 - backstep) as usize;
            if backstep != 0 {
                let src = (fr.bsbufold as i64 + fr.fsizeold as i64 - backstep) as usize;
                let n = backstep as usize;
                fr.bsspace.copy_within(src..src + n, fr.wordpointer);
            }
            fr.bits_avail = (fr.hdr.framesize as i64 - fr.hdr.ssize as i64 + backstep) * 8;
        } else {
            fr.wordpointer = fr.bsbuf;
            fr.bits_avail = fr.hdr.ssize as i64 * 8;
        }
    } else {
        fr.wordpointer = fr.bsbuf;
        fr.bits_avail = fr.hdr.framesize as i64 * 8;
    }
}

/********************************/

/// Translation of `INT123_compute_bpf()`.
pub(crate) fn compute_bpf(fr: &Mpg123Handle<'_>) -> f64 {
    if fr.hdr.framesize > 0 {
        fr.hdr.framesize as f64 + 4.0
    } else {
        1.0
    }
}

/* first attempt of read ahead check to find the real first header; cannot believe what junk is out there! */
fn do_readahead(fr: &mut Mpg123Handle<'_>, nhdr: &FrameHeader, newhead: u64) -> i32 {
    let mut nexthead: u64 = 0;

    if !(fr.firsthead == 0 && fr.rdat.flags & (READER_SEEKABLE | READER_BUFFERED) != 0) {
        return PARSE_GOOD;
    }

    let start = rd_tell(fr);

    /* step framesize bytes forward and read next possible header*/
    let oret = rd_skip_bytes(fr, nhdr.framesize as i64);
    if oret < 0 {
        return if oret == MPG123_NEED_MORE as i64 {
            PARSE_MORE
        } else {
            PARSE_ERR
        };
    }

    /* Read header, seek back. */
    let hd = rd_head_read(fr, &mut nexthead);
    let back = rd_tell(fr) - start;
    if rd_back_bytes(fr, back) < 0 {
        // Cannot seek back!
        return PARSE_ERR;
    }
    if hd == MPG123_NEED_MORE {
        return PARSE_MORE;
    }

    if hd == 0 {
        // Cannot read next header, a one-frame stream? Duh...
        return PARSE_END;
    }

    if !head_check(nexthead) || !head_compatible(newhead, nexthead) {
        fr.oldhead = 0; /* start over */
        /* try next byte for valid header */
        if rd_back_bytes(fr, 3) < 0 {
            // Cannot seek 3 bytes back!
            return PARSE_ERR;
        }
        PARSE_AGAIN
    } else {
        PARSE_GOOD
    }
}

fn handle_id3v2(fr: &mut Mpg123Handle<'_>, newhead: u64) -> i32 {
    fr.oldhead = 0; /* Think about that. Used to be present only for skipping of junk, not resync-style wetwork. */
    let ret = parse_new_id3(fr, newhead);
    if ret < 0 {
        return ret;
    } else if ret > 0 {
        fr.metaflags |= MPG123_NEW_ID3 | MPG123_ID3;
    }
    PARSE_AGAIN
}

fn handle_apetag(fr: &mut Mpg123Handle<'_>, _newhead: u64) -> i32 {
    let mut apebuf = [0u8; 28];
    /* How many bytes to backpedal to get back to just after the first byte of */
    /* the supposed header. */
    let mut back_bytes: i64 = 3;
    fr.oldhead = 0;

    'bad: {
        /* Apetag headers are 32 bytes, newhead contains 4, read the rest */
        let ret = rd_fullread(fr, &mut apebuf);
        if ret < 0 {
            return ret as i32;
        }
        back_bytes += ret as i64;
        if ret < 28 {
            break 'bad;
        }

        /* Apetags start with "APETAGEX", "APET" is already tested. */
        if &apebuf[..4] != b"AGEX" {
            break 'bad;
        }

        /* Version must be 2.000 / 2000 */
        let val = ((apebuf[7] as u64) << 24)
            | ((apebuf[6] as u64) << 16)
            | ((apebuf[5] as u64) << 8)
            | apebuf[4] as u64;
        if val != 2000 {
            break 'bad;
        }

        /* Last 8 bytes must be 0 */
        for &c in &apebuf[20..28] {
            if c != 0 {
                break 'bad;
            }
        }

        /* Looks good, skip the rest. */
        let val = ((apebuf[11] as u64) << 24)
            | ((apebuf[10] as u64) << 16)
            | ((apebuf[9] as u64) << 8)
            | apebuf[8] as u64;
        /* If encountering EOF here, things are just at an end. */
        let ret = rd_skip_bytes(fr, val as i64);
        if ret < 0 {
            return ret as i32;
        }

        return PARSE_AGAIN;
    }

    // apetag_bad:
    if rd_back_bytes(fr, back_bytes) < 0 {
        // Cannot seek %d bytes back!
    }

    PARSE_AGAIN /* Give the resync code a chance to fix things */
}

/* Advance a byte in stream to get next possible header and forget
buffered data if possible (for feed reader). */
const FORGET_INTERVAL: u32 = 1024; /* Used by callers to set forget flag each <n> bytes. */
fn forget_head_shift(fr: &mut Mpg123Handle<'_>, newheadp: &mut u64, _forget: bool) -> i32 {
    let ret = rd_head_shift(fr, newheadp);
    if ret <= 0 {
        return ret;
    }
    /* Try to forget buffered data as early as possible to speed up parsing where
    new data needs to be added for resync (and things would be re-parsed again
    and again because of the start from beginning after hitting end). */
    // (the stream readers have no forget())
    ret /* No surprise here, error already triggered early return. */
}

/* watch out for junk/tags on beginning of stream by invalid header */
fn skip_junk(
    fr: &mut Mpg123Handle<'_>,
    newheadp: &mut u64,
    headcount: &mut i64,
    nhdr: &mut FrameHeader,
) -> i32 {
    let mut ret;
    let mut freeformat_count = 0;
    let mut limit: i64 = 65536;
    let mut newhead = *newheadp;
    let mut forgetcount: u32 = 0;
    /* check for id3v2; first three bytes (of 4) are "ID3" */
    if (newhead & 0xffffff00) == 0x49443300 {
        return handle_id3v2(fr, newhead);
    }

    /* I even saw RIFF headers at the beginning of MPEG streams ;( */
    if newhead
        == (((b'R' as u64) << 24) + ((b'I' as u64) << 16) + ((b'F' as u64) << 8) + b'F' as u64)
    {
        ret = rd_head_read(fr, &mut newhead);
        if ret <= 0 {
            return ret;
        }

        while newhead
            != (((b'd' as u64) << 24) + ((b'a' as u64) << 16) + ((b't' as u64) << 8) + b'a' as u64)
        {
            forgetcount += 1;
            if forgetcount > FORGET_INTERVAL {
                forgetcount = 0;
            }
            ret = forget_head_shift(fr, &mut newhead, forgetcount == 0);
            if ret <= 0 {
                return ret;
            }
        }
        ret = rd_head_read(fr, &mut newhead);
        if ret <= 0 {
            return ret;
        }

        fr.oldhead = 0;
        *newheadp = newhead;
        return PARSE_AGAIN;
    }

    /*
        Unhandled junk... just continue search for a header, stepping in single bytes through next 64K.
        This is rather identical to the resync loop.
    */
    *newheadp = 0; /* Invalidate the external value. */
    ret = 0; /* We will check the value after the loop. */

    /* We prepare for at least the 64K bytes as usual, unless
    user explicitly wanted more (even infinity). Never less. */
    if fr.p.resync_limit < 0 || fr.p.resync_limit > limit {
        limit = fr.p.resync_limit;
    }

    loop {
        *headcount += 1;
        if limit >= 0 && *headcount >= limit {
            break;
        }

        forgetcount += 1;
        if forgetcount > FORGET_INTERVAL {
            forgetcount = 0;
        }
        ret = forget_head_shift(fr, &mut newhead, forgetcount == 0);
        if ret <= 0 {
            return ret;
        }

        if head_check(newhead) {
            ret = decode_header(fr, nhdr, newhead, &mut freeformat_count);
            if ret != 0 {
                break;
            }
        }
    }
    if ret < 0 {
        return ret;
    }

    if limit >= 0 && *headcount >= limit {
        // Giving up searching valid MPEG header after %li bytes of junk.
        fr.err = MPG123_RESYNC_FAIL;
        return PARSE_ERR;
    }

    /* If the new header ist good, it is already decoded. */
    *newheadp = newhead;
    PARSE_GOOD
}

/* The newhead is bad, so let's check if it is something special, otherwise just resync. */
fn wetwork(fr: &mut Mpg123Handle<'_>, newheadp: &mut u64) -> i32 {
    let mut ret;
    let mut newhead = *newheadp;
    *newheadp = 0;

    /* Classic ID3 tags. Read, then start parsing again. */
    if (newhead & 0xffffff00)
        == (((b'T' as u64) << 24) + ((b'A' as u64) << 16) + ((b'G' as u64) << 8))
    {
        fr.id3buf[0] = ((newhead >> 24) & 0xff) as u8;
        fr.id3buf[1] = ((newhead >> 16) & 0xff) as u8;
        fr.id3buf[2] = ((newhead >> 8) & 0xff) as u8;
        fr.id3buf[3] = (newhead & 0xff) as u8;

        let mut rest = [0u8; 124];
        rest.copy_from_slice(&fr.id3buf[4..]);
        ret = rd_fullread(fr, &mut rest) as i32;
        fr.id3buf[4..].copy_from_slice(&rest);
        if ret < 0 {
            return ret;
        }

        fr.metaflags |= MPG123_NEW_ID3 | MPG123_ID3;
        fr.rdat.flags |= READER_ID3TAG; /* that marks id3v1 */

        return PARSE_AGAIN;
    }
    /* This is similar to initial junk skipping code... */
    /* Check for id3v2; first three bytes (of 4) are "ID3" */
    if (newhead & 0xffffff00) == 0x49443300 {
        return handle_id3v2(fr, newhead);
    }
    /* Check for an apetag header */
    if newhead
        == (((b'A' as u64) << 24) + ((b'P' as u64) << 16) + ((b'E' as u64) << 8) + b'T' as u64)
    {
        return handle_apetag(fr, newhead);
    }

    /* Now we got something bad at hand, try to recover. */

    if fr.p.flags & MPG123_NO_RESYNC == 0 {
        let mut try_: i64 = 0;
        let limit = fr.p.resync_limit;
        let mut forgetcount: u32 = 0;

        /* If a resync is needed the bitreservoir of previous frames is no longer valid */
        fr.bitreservoir = 0;

        loop {
            /* ... shift the header with additional single bytes until be found something that could be a header. */
            try_ += 1;
            if limit >= 0 && try_ >= limit {
                break;
            }

            forgetcount += 1;
            if forgetcount > FORGET_INTERVAL {
                forgetcount = 0;
            }
            ret = forget_head_shift(fr, &mut newhead, forgetcount == 0);
            if ret <= 0 {
                *newheadp = newhead;
                // Note: Hit end of (available) data during resync.

                return if ret != 0 { ret } else { PARSE_END };
            }
            if head_check(newhead) {
                break;
            }
        }

        *newheadp = newhead;

        /* Now we either got something that could be a header, or we gave up. */
        if limit >= 0 && try_ >= limit {
            // Giving up resync after %li bytes - your stream is not nice... (maybe increasing resync limit could help).

            fr.err = MPG123_RESYNC_FAIL;
            PARSE_ERR
        } else {
            fr.oldhead = 0;
            PARSE_RESYNC
        }
    } else {
        // not attempting to resync...

        fr.err = MPG123_OUT_OF_SYNC;
        PARSE_ERR
    }
    /* Control never goes here... we return before that. */
}
