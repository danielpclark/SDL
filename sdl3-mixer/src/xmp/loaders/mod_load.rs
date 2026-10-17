// Rust translation of src/loaders/mod_load.c and src/loaders/mod.h from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2026 Claudio Matsuoka and Hipolito Carraro Jr (mod_load.c)
// Copyright (C) 1996-2021 Claudio Matsuoka and Hipolito Carraro Jr (mod.h)
// This is an altered (translated) version of the original software; see LICENSE.txt.

/* This loader recognizes the following variants of the Protracker
 * module format:
 *
 * - Protracker M.K. and M!K!
 * - Protracker songs
 * - Noisetracker N.T. and M&K! (not tested)
 * - Fast Tracker 6CHN and 8CHN
 * - Fasttracker II/Take Tracker ?CHN and ??CH
 * - Mod's Grave M.K. w/ 8 channels (WOW)
 * - Atari Octalyser CD61 and CD81
 * - Digital Tracker FA04, FA06 and FA08
 * - TakeTracker TDZ1, TDZ2, TDZ3, and TDZ4
 * - Software Visions DMF (ModEdit M.K. with first 2108 bytes flipped)
 * - (unknown) NSMS, LARD
 *
 * The 'lite' version only recognizes Protracker M.K. and
 * Fasttracker ?CHN and ??CH formats.
 */

//! The Protracker MOD loader.

use super::super::common::*;
use super::super::dataio::*;
use super::super::hio::*;
use super::super::path::*;
use super::super::*;
use super::common::*;
use super::sample::libxmp_load_sample;
use super::*;

/// Translation of `struct mod_instrument`.
#[derive(Clone, Copy, Default)]
pub(crate) struct ModInstrument {
    pub name: [u8; 22],  /* Instrument name */
    pub size: u16,       /* Sample length in 16-bit words */
    pub finetune: i8,    /* Finetune (signed nibble) */
    pub volume: i8,      /* Linear playback volume */
    pub loop_start: u16, /* Loop start in 16-bit words */
    pub loop_size: u16,  /* Loop length in 16-bit words */
}

/// Translation of `struct mod_header`.
#[derive(Clone, Copy)]
pub(crate) struct ModHeader {
    pub name: [u8; 20],
    pub ins: [ModInstrument; 31],
    pub len: u8,
    pub restart: u8, /* Number of patterns in Soundtracker,
                      * Restart in Noisetracker/Startrekker,
                      * 0x7F in Protracker
                      */
    pub order: [u8; 128],
    pub magic: [u8; 4],
}

/// Translation of `struct mod_magic`.
struct ModMagic {
    magic: &'static [u8; 4],
    flag: i32,
    id: i32,
    ch: i32,
}

const TRACKER_PROTRACKER: i32 = 0;
const TRACKER_NOISETRACKER: i32 = 1;
const TRACKER_SOUNDTRACKER: i32 = 2;
const TRACKER_FASTTRACKER: i32 = 3;
const TRACKER_FASTTRACKER2: i32 = 4;
const TRACKER_OCTALYSER: i32 = 5;
const TRACKER_TAKETRACKER: i32 = 6;
const TRACKER_DIGITALTRACKER: i32 = 7;
const TRACKER_FLEXTRAX: i32 = 8;
const TRACKER_MODSGRAVE: i32 = 9;
const TRACKER_SCREAMTRACKER3: i32 = 10;
const TRACKER_OPENMPT: i32 = 11;
const TRACKER_SOFTWAREVISIONS: i32 = 12;
const TRACKER_UNKNOWN_CONV: i32 = 95;
const TRACKER_CONVERTEDST: i32 = 96;
const TRACKER_CONVERTED: i32 = 97;
const TRACKER_CLONE: i32 = 98;
const TRACKER_UNKNOWN: i32 = 99;

const TRACKER_PROBABLY_NOISETRACKER: i32 = 20;

#[rustfmt::skip]
const MOD_MAGIC: [ModMagic; 18] = [
    ModMagic { magic: b"M.K.", flag: 0, id: TRACKER_PROTRACKER, ch: 4 },
    ModMagic { magic: b"M!K!", flag: 1, id: TRACKER_PROTRACKER, ch: 4 },
    ModMagic { magic: b"M&K!", flag: 1, id: TRACKER_NOISETRACKER, ch: 4 },
    ModMagic { magic: b"N.T.", flag: 1, id: TRACKER_NOISETRACKER, ch: 4 },
    ModMagic { magic: b"6CHN", flag: 0, id: TRACKER_FASTTRACKER, ch: 6 },
    ModMagic { magic: b"8CHN", flag: 0, id: TRACKER_FASTTRACKER, ch: 8 },
    ModMagic { magic: b"CD61", flag: 1, id: TRACKER_OCTALYSER, ch: 6 },  /* Atari STe/Falcon */
    ModMagic { magic: b"CD81", flag: 1, id: TRACKER_OCTALYSER, ch: 8 },  /* Atari STe/Falcon */
    ModMagic { magic: b"TDZ1", flag: 1, id: TRACKER_TAKETRACKER, ch: 1 }, /* TakeTracker 1ch */
    ModMagic { magic: b"TDZ2", flag: 1, id: TRACKER_TAKETRACKER, ch: 2 }, /* TakeTracker 2ch */
    ModMagic { magic: b"TDZ3", flag: 1, id: TRACKER_TAKETRACKER, ch: 3 }, /* TakeTracker 3ch */
    ModMagic { magic: b"TDZ4", flag: 1, id: TRACKER_TAKETRACKER, ch: 4 }, /* see XModule SaveTracker.c */
    ModMagic { magic: b"FA04", flag: 1, id: TRACKER_DIGITALTRACKER, ch: 4 }, /* Atari Falcon */
    ModMagic { magic: b"FA06", flag: 1, id: TRACKER_DIGITALTRACKER, ch: 6 }, /* Atari Falcon */
    ModMagic { magic: b"FA08", flag: 1, id: TRACKER_DIGITALTRACKER, ch: 8 }, /* Atari Falcon */
    ModMagic { magic: b".M.K", flag: 1, id: TRACKER_SOFTWAREVISIONS, ch: 4 }, /* Software Visions DMF */
    ModMagic { magic: b"LARD", flag: 1, id: TRACKER_UNKNOWN, ch: 4 },    /* in judgement_day_gvine.mod */
    ModMagic { magic: b"NSMS", flag: 1, id: TRACKER_UNKNOWN, ch: 4 },    /* in Kingdom.mod */
];

/* Returns non-zero if the given tracker ONLY supports VBlank timing. This
 * should be used only when the tracker is known for sure, e.g. magic match. */
/// Translation of `tracker_is_vblank()`.
fn tracker_is_vblank(id: i32) -> bool {
    matches!(id, TRACKER_NOISETRACKER | TRACKER_SOUNDTRACKER)
}

pub(crate) const LIBXMP_LOADER_MOD: FormatLoader = FormatLoader {
    name: "Amiga Protracker/Compatible",
    test: mod_test,
    loader: mod_load,
};

/// Translation of `flip_word_bytes()`.
fn flip_word_bytes(buf: &mut [u8], bytes: usize) {
    let mut i = 0;
    while i < bytes {
        buf.swap(i, i + 1);
        i += 2;
    }
}

/// Translation of `validate_pattern()`.
fn validate_pattern(buf: &[u8]) -> i32 {
    for i in 0..64 {
        for j in 0..4 {
            let d = &buf[(i * 4 + j) * 4..];
            if (d[0] >> 4) > 1 {
                return -1;
            }
        }
    }

    0
}

/// Translation of `mod_test()`.
fn mod_test(f: &mut HioHandle<'_>, t: Option<&mut [u8]>, start: i32) -> i32 {
    let mut buf = [0u8; 4];
    let mut pat_buf = [0u8; 1024];

    hio_seek(f, start as i64 + 1080, SEEK_SET);
    if hio_read(&mut buf, 1, 4, f) < 4 {
        return -1;
    }

    'found: {
        if &buf[2..4] == b"CH" && isdigit(buf[0]) && isdigit(buf[1]) {
            let i = (buf[0] - b'0') as i32 * 10 + buf[1] as i32 - b'0' as i32;
            if i > 0 && i <= 32 {
                break 'found;
            }
        }

        if &buf[1..4] == b"CHN" && isdigit(buf[0]) && buf[0] - b'0' != 0 {
            break 'found;
        }

        let Some(i) = MOD_MAGIC.iter().position(|mm| &buf == mm.magic) else {
            return -1;
        };

        let detected = MOD_MAGIC[i].flag;
        let id = MOD_MAGIC[i].id;
        let mut smp_size = 0;

        /*
         * Sanity check to prevent loading NoiseRunner and other module
         * formats with valid magic at offset 1080 (e.g. His Master's Noise)
         */

        hio_seek(f, start as i64 + 20, SEEK_SET);
        for _ in 0..31 {
            hio_read(&mut pat_buf, 1, 30, f);
            if id == TRACKER_SOFTWAREVISIONS {
                flip_word_bytes(&mut pat_buf[22..], 8);
            }

            /* hio_seek(f, 22, SEEK_CUR);	 Instrument name */
            smp_size += (readmem16b(&pat_buf[22..]) as i32) << 1; /* sample size */

            /* Chris Spiegel tells me that sandman.mod has 0x20 in finetune */
            let x = pat_buf[24];
            if x & 0xf0 != 0 && x != 0x20 {
                /* test finetune */
                return -1;
            }
            if pat_buf[25] > 0x40 {
                /* test volume */
                return -1;
            }
            /* hio_read16b(f);		 loop start */
            /* hio_read16b(f);		 loop size */
        }

        /* The following checks are only relevant for filtering out atypical
         * M.K. variants. If the magic is from a recognizable source, skip them. */
        if detected != 0 {
            break 'found;
        }

        /* Test for UNIC tracker modules
         *
         * From Gryzor's Pro-Wizard PW_FORMATS-Engl.guide:
         * ``The UNIC format is very similar to Protracker... At least in the
         * heading... same length : 1084 bytes. Even the "M.K." is present,
         * sometimes !! Maybe to disturb the rippers.. hehe but Pro-Wizard
         * doesn't test this only!''
         */

        /* get file size */
        let size = hio_size(f);

        /* get number of patterns */
        let mut num_pat = 0;
        hio_seek(f, start as i64 + 952, SEEK_SET);
        for _ in 0..128 {
            let x = hio_read8(f);
            if x > 0x7f {
                break;
            }
            if x as i32 > num_pat {
                num_pat = x as i32;
            }
        }
        num_pat += 1;

        /* see if module size matches UNIC */
        if (start + 1084 + num_pat * 0x300 + smp_size) as i64 == size {
            return -1;
        }

        /* validate pattern data in an attempt to catch UNICs with MOD size */
        let mut count = 0;
        for i in 0..num_pat {
            hio_seek(f, (start + 1084 + 1024 * i) as i64, SEEK_SET);
            if hio_read(&mut pat_buf, 1024, 1, f) == 0 {
                return -1;
            }
            if validate_pattern(&pat_buf) < 0 {
                /* Allow a few errors, "lexstacy" has 0x52 */
                count += 1;
            }
        }
        if count > 2 {
            return -1;
        }
    }

    hio_seek(f, start as i64, SEEK_SET);
    libxmp_read_title(f, t, 20);

    0
}

/// Translation of `is_st_ins()`.
fn is_st_ins(s: &[u8]) -> bool {
    if s[0] != b's' && s[0] != b'S' {
        return false;
    }
    if s[1] != b't' && s[1] != b'T' {
        return false;
    }
    if s[2] != b'-' || s[5] != b':' {
        return false;
    }
    if !isdigit(s[3]) || !isdigit(s[4]) {
        return false;
    }

    true
}

/// Translation of `get_tracker_id()`.
fn get_tracker_id(m: &mut ModuleData, mh: &ModHeader, mut id: i32) -> i32 {
    let mod_ = &mut m.mod_;
    let mut has_loop_0 = false;
    let mut has_vol_in_empty_ins = false;

    /* Check if has instruments with loop size 0 */
    for i in 0..31 {
        if mh.ins[i].loop_size == 0 {
            has_loop_0 = true;
            break;
        }
    }

    /* Check if has instruments with size 0 and volume > 0 */
    for i in 0..31 {
        if mh.ins[i].size == 0 && mh.ins[i].volume > 0 {
            has_vol_in_empty_ins = true;
            break;
        }
    }

    /*
     * Test Protracker-like files
     */
    if mh.restart as i32 == mod_.pat {
        if mod_.chn == 4 {
            id = TRACKER_SOUNDTRACKER;
        } else {
            id = TRACKER_UNKNOWN;
        }
    } else if mh.restart == 0x78 {
        if mod_.chn == 4 {
            /* Can't trust this for Noisetracker, MOD.Data City Remix
             * has Protracker effects and Noisetracker restart byte
             */
            id = TRACKER_PROBABLY_NOISETRACKER;
        } else {
            id = TRACKER_UNKNOWN;
        }
        return id;
    } else if mh.restart < 0x7f {
        if mod_.chn == 4 && !has_vol_in_empty_ins {
            id = TRACKER_NOISETRACKER;
        } else {
            id = TRACKER_UNKNOWN; /* ? */
        }
        mod_.rst = mh.restart as i32;
    } else if mh.restart == 0x7f {
        if mod_.chn == 4 {
            if has_loop_0 {
                id = TRACKER_CLONE;
            }
        } else {
            id = TRACKER_SCREAMTRACKER3;
        }
        return id;
    } else if mh.restart > 0x7f {
        id = TRACKER_UNKNOWN; /* ? */
        return id;
    }

    if !has_loop_0 {
        /* All loops are size 2 or greater */

        for i in 0..31 {
            if mh.ins[i].size == 1 && mh.ins[i].volume == 0 {
                return TRACKER_CONVERTED;
            }
        }

        let mut i = 0;
        while i < 31 {
            if is_st_ins(&mh.ins[i].name) {
                break;
            }
            i += 1;
        }
        if i == 31 {
            /* No st- instruments */
            for i in 0..31 {
                if mh.ins[i].size != 0 || mh.ins[i].loop_size != 1 {
                    continue;
                }

                match mod_.chn {
                    4 => {
                        if has_vol_in_empty_ins {
                            id = TRACKER_OPENMPT;
                        } else {
                            id = TRACKER_NOISETRACKER;
                            /* or Octalyser */
                        }
                    }
                    6 | 8 => {
                        id = TRACKER_OCTALYSER;
                    }
                    _ => {
                        id = TRACKER_UNKNOWN;
                    }
                }
                return id;
            }

            if mod_.chn == 4 {
                id = TRACKER_PROTRACKER;
            } else if mod_.chn == 6 || mod_.chn == 8 {
                /* FastTracker 1.01? */
                id = TRACKER_FASTTRACKER;
            } else {
                id = TRACKER_UNKNOWN;
            }
        }
    } else {
        /* Has loops with size 0 */
        let mut i = 15;
        while i < 31 {
            /* Is the name or size set? */
            if mh.ins[i].name[0] != 0 || mh.ins[i].size > 0 {
                break;
            }
            i += 1;
        }
        if i == 31 && is_st_ins(&mh.ins[14].name) {
            return TRACKER_CONVERTEDST;
        }

        /* Assume that Fast Tracker modules won't have ST- instruments */
        let mut i = 0;
        while i < 31 {
            if is_st_ins(&mh.ins[i].name) {
                break;
            }
            i += 1;
        }
        if i < 31 {
            return TRACKER_UNKNOWN_CONV;
        }

        if mod_.chn == 4 || mod_.chn == 6 || mod_.chn == 8 {
            return TRACKER_FASTTRACKER;
        }

        id = TRACKER_UNKNOWN; /* ?! */
    }

    id
}

/// Translation of `mod_load()`.
#[allow(unused_assignments)]
fn mod_load(m: &mut ModuleData, f: &mut HioHandle<'_>, start: i32) -> i32 {
    let mut magic = [0u8; 8];
    let mut pat_high_fxx = [0u8; 256];
    let tracker: &str;
    let mut detected = 0;
    let mut tracker_id = TRACKER_PROTRACKER;
    let mut out_of_range = false;
    let mut maybe_wow = true;
    let mut ptsong = false;
    let mut needs_timing_detection = false;
    let mut samerow_fxx = false; /* speed + BPM set on same row */
    let mut high_fxx = false; /* high Fxx is used anywhere */
    let mut invert_loop = false; /* EFx found anywhere in module */
    let mut ptkloop = false; /* Protracker loop */

    m.mod_.ins = 31;
    m.mod_.smp = m.mod_.ins;
    m.mod_.chn = 0;
    let mut smp_size = 0;
    m.period_type = PERIOD_MODRNG;

    let Ok(mut patbuf) = try_vec(1084, 0u8) else {
        return -1;
    };
    if hio_read(&mut patbuf, 1, 1084, f) < 1084 {
        return -1;
    }
    magic[..4].copy_from_slice(&patbuf[1080..1084]);

    for mm in &MOD_MAGIC {
        if strneq(&magic, mm.magic, 4) {
            m.mod_.chn = mm.ch;
            tracker_id = mm.id;
            detected = mm.flag;
            break;
        }
    }

    /* Enable timing detection for M.K. and M!K! modules. */
    if tracker_id == TRACKER_PROTRACKER {
        needs_timing_detection = true;
    }

    /* Digital Tracker MODs have an extra four bytes after the magic.
     * These are always 00h 40h 00h 00h and can probably be ignored. */
    if tracker_id == TRACKER_DIGITALTRACKER {
        hio_read32b(f);
    }

    if m.mod_.chn == 0 {
        if strneq(&magic[2..], b"CH", 2) && isdigit(magic[0]) && isdigit(magic[1]) {
            m.mod_.chn = (magic[0] as i32 - b'0' as i32) * 10 + magic[1] as i32 - b'0' as i32;
        } else if strneq(&magic[1..], b"CHN", 3) && isdigit(magic[0]) {
            m.mod_.chn = magic[0] as i32 - b'0' as i32;
        } else {
            return -1;
        }
        tracker_id = if m.mod_.chn & 1 != 0 {
            TRACKER_TAKETRACKER
        } else {
            TRACKER_FASTTRACKER2
        };
        detected = 1;
    }

    /* For unknown reasons, the first 2108 bytes of Software Visions DMF MODs
     * (ModEdit M.K. used in Apocalypse Abyss and Software Visions Catalog)
     * are word-flipped to little endian. */
    if tracker_id == TRACKER_SOFTWAREVISIONS {
        flip_word_bytes(&mut patbuf, 1084);
    }

    let mut mh = ModHeader {
        name: [0; 20],
        ins: [ModInstrument::default(); 31],
        len: 0,
        restart: 0,
        order: [0; 128],
        magic: [0; 4],
    };
    mh.name.copy_from_slice(&patbuf[..20]);
    for i in 0..31 {
        let pos = &patbuf[20 + i * 30..];

        mh.ins[i].name.copy_from_slice(&pos[..22]); /* Instrument name */
        mh.ins[i].size = readmem16b(&pos[22..]); /* Length in 16-bit words */
        mh.ins[i].finetune = pos[24] as i8; /* Finetune (signed nibble) */
        mh.ins[i].volume = pos[25] as i8; /* Linear playback volume */
        mh.ins[i].loop_start = readmem16b(&pos[26..]); /* Loop start in 16-bit words */
        mh.ins[i].loop_size = readmem16b(&pos[28..]); /* Loop size in 16-bit words */

        /* Mod's Grave WOW files are converted from 669s and have default
         * finetune and volume.
         */
        if mh.ins[i].size != 0 && (mh.ins[i].finetune != 0 || mh.ins[i].volume != 64) {
            maybe_wow = false;
        }

        smp_size += 2 * mh.ins[i].size as i32;
    }
    mh.len = patbuf[950];
    mh.restart = patbuf[951];
    mh.order.copy_from_slice(&patbuf[952..1080]);
    drop(patbuf);

    /* Mod's Grave WOW files always have a 0 restart byte; 6692WOW implements
     * 669 repeating by inserting a pattern jump and ignores this byte.
     */
    if mh.restart != 0 {
        maybe_wow = false;
    }

    strncpy(&mut m.mod_.name, &mh.name, 20);

    let mod_ = &mut m.mod_;
    mod_.len = mh.len as i32;
    mod_.xxo[..128].copy_from_slice(&mh.order);

    if mh.restart < 0x7f && mh.restart != 0x78 && (mh.restart as i32) < mod_.len {
        /* TODO: an older version of this code was commented out 23+ years ago
         * and adding this may have rebroke something. */
        mod_.rst = mh.restart as i32;
    }

    for i in 0..128 {
        /* This fixes dragnet.mod (garbage in the order list) */
        if mod_.xxo[i] > 0x7f {
            break;
        }
        if mod_.xxo[i] as i32 > mod_.pat {
            mod_.pat = mod_.xxo[i] as i32;
        }
    }
    mod_.pat += 1;

    if libxmp_init_instrument(m) < 0 {
        return -1;
    }

    for i in 0..m.mod_.ins {
        let mod_ = &mut m.mod_;
        if libxmp_alloc_subinstrument(mod_, i, 1) < 0 {
            return -1;
        }

        let mi = &mh.ins[i as usize];
        if mi.size >= 0x8000 {
            tracker_id = TRACKER_OPENMPT;
            needs_timing_detection = false;
            detected = 1;
        }

        let xxs = &mut mod_.xxs[i as usize];

        xxs.len = 2 * mi.size as i32;
        xxs.lps = 2 * mi.loop_start as i32;
        xxs.lpe = xxs.lps + 2 * mi.loop_size as i32;
        if xxs.lpe > xxs.len {
            xxs.lpe = xxs.len;
        }
        xxs.flg = if mi.loop_size > 1 && xxs.lpe >= 4 {
            XMP_SAMPLE_LOOP
        } else {
            0
        };
        let has_len = xxs.len > 0;
        let sub = &mut mod_.xxi[i as usize].sub[0];
        sub.fin = ((mi.finetune as u8) << 4) as i8 as i32;
        sub.vol = mi.volume as i32;
        sub.pan = XMP_INST_NO_DEFAULT_PAN;
        sub.sid = i;
        libxmp_instrument_name(mod_, i, &mi.name, 22);

        if has_len {
            mod_.xxi[i as usize].nsm = 1;
        }
    }

    /* Experimental tracker-detection routine */

    'skip_test: {
        if detected != 0 {
            break 'skip_test;
        }

        let mod_ = &m.mod_;

        /* Test for Flextrax modules
         *
         * FlexTrax is a soundtracker for Atari Falcon030 compatible computers.
         * FlexTrax supports the standard MOD file format (up to eight channels)
         * for compatibility reasons but also features a new enhanced module
         * format FLX. The FLX format is an extended version of the standard
         * MOD file format with support for real-time sound effects like reverb
         * and delay.
         */

        let flx = 0x43c + mod_.pat * 4 * mod_.chn * 0x40 + smp_size;
        if flx < m.size {
            let mut idbuffer = [0u8; 4];
            let pos = hio_tell(f) as i32;
            if pos < 0 {
                return -1;
            }
            hio_seek(f, (start + flx) as i64, SEEK_SET);
            let num_read = hio_read(&mut idbuffer, 1, 4, f);
            hio_seek(f, (start + pos) as i64, SEEK_SET);

            if num_read == 4 && &idbuffer == b"FLEX" {
                tracker_id = TRACKER_FLEXTRAX;
                needs_timing_detection = false;
                detected = 1;
                break 'skip_test;
            }
        }

        /* Test for Mod's Grave WOW modules
         *
         * Stefan Danes <sdanes@marvels.hacktic.nl> said:
         * This weird format is identical to '8CHN' but still uses the 'M.K.' ID.
         * You can only test for WOW by calculating the size of the module for 8
         * channels and comparing this to the actual module length. If it's equal,
         * the module is an 8 channel WOW.
         *
         * Addendum: very rarely, WOWs will have an odd length due to an extra byte,
         * so round the filesize down in this check. False positive WOWs can be ruled
         * out by checking the restart byte and sample volume (see above).
         *
         * Worst case if there are still issues with this, OpenMPT validates later
         * patterns in potential WOW files (where sample data would be located in a
         * regular M.K. MOD) to rule out false positives.
         */

        if strneq(&magic, b"M.K.", 4)
            && maybe_wow
            && (0x43c + mod_.pat * 32 * 0x40 + smp_size) == (m.size & !1)
        {
            m.mod_.chn = 8;
            tracker_id = TRACKER_MODSGRAVE;
            needs_timing_detection = false;
            detected = 1;
        } else {
            /* Test for Protracker song files */
            ptsong = strneq(&magic, b"M.K.", 4) && (0x43c + mod_.pat * 0x400 == m.size);
            if ptsong {
                tracker_id = TRACKER_PROTRACKER;
                detected = 1;
                break 'skip_test;
            } else {
                /* something else */
                tracker_id = get_tracker_id(m, &mh, tracker_id);
            }
        }
    }

    let mod_ = &mut m.mod_;

    if mod_.chn >= XMP_MAX_CHANNELS as i32 {
        return -1;
    }

    mod_.trk = mod_.chn * mod_.pat;

    if libxmp_init_pattern(mod_) < 0 {
        return -1;
    }

    /* Load and convert patterns */

    let Ok(mut patbuf) = try_vec((64 * 4 * mod_.chn) as usize, 0u8) else {
        return -1;
    };

    for i in 0..m.mod_.pat {
        let patlen = (64 * 4 * m.mod_.chn) as usize;

        if libxmp_alloc_pattern_tracks(&mut m.mod_, i, 64) < 0 {
            return -1;
        }

        if hio_read(&mut patbuf, 1, patlen, f) < patlen {
            return -1;
        }

        /* First pattern of Software Visions DMF is word flipped (see above). */
        if tracker_id == TRACKER_SOFTWAREVISIONS && i == 0 {
            flip_word_bytes(&mut patbuf, patlen); /* should be 1024 */
        }

        let mut mod_event = 0;
        for _ in 0..64 {
            let mut speed_row = false;
            let mut bpm_row = false;
            for _ in 0..m.mod_.chn {
                let ev = &patbuf[mod_event..mod_event + 4];

                let period = (lsn(ev[0]) << 8) | ev[1] as i32;
                if period != 0 && !(108..=907).contains(&period) {
                    out_of_range = true;
                }

                /* Filter noisetracker events */
                if tracker_id == TRACKER_PROBABLY_NOISETRACKER {
                    let fxt = lsn(ev[2]);
                    let fxp = lsn(ev[3]);

                    if (fxt > 0x06 && fxt < 0x0a) || (fxt == 0x0e && fxp > 1) {
                        tracker_id = TRACKER_UNKNOWN;
                    }
                }
                /* Needs CIA/VBlank detection? */
                if lsn(ev[2]) == 0x0f {
                    if ev[3] >= 0x20 {
                        pat_high_fxx[i as usize] = ev[3];
                        m.compare_vblank = 1;
                        high_fxx = true;
                        bpm_row = true;
                    } else {
                        speed_row = true;
                    }
                }
                /* Usage of effect EFx is typically Protracker invert loop. */
                if lsn(ev[2]) == 0xe && msn(ev[3]) == 0xf {
                    invert_loop = true;
                }
                mod_event += 4;
            }
            if bpm_row && speed_row {
                samerow_fxx = true;
            }
        }

        if out_of_range {
            if tracker_id == TRACKER_UNKNOWN && mh.restart == 0x7f {
                tracker_id = TRACKER_SCREAMTRACKER3;
            }

            /* Check out-of-range notes in Amiga trackers */
            if tracker_id == TRACKER_PROTRACKER
                || tracker_id == TRACKER_NOISETRACKER
                || tracker_id == TRACKER_PROBABLY_NOISETRACKER
                || tracker_id == TRACKER_SOUNDTRACKER
            {
                /* note > B-3 */
                tracker_id = TRACKER_UNKNOWN;
            }
        } else if invert_loop
            && detected == 0
            && (tracker_id == TRACKER_NOISETRACKER || tracker_id == TRACKER_PROBABLY_NOISETRACKER)
        {
            /* Switch Noisetracker to Protracker to disable event filtering. */
            tracker_id = TRACKER_PROTRACKER;
        }

        let mut mod_event = 0;
        for j in 0..64 {
            for k in 0..m.mod_.chn {
                let ev = &patbuf[mod_event..mod_event + 4];
                let event = m.mod_.event_mut(i, k, j);
                match tracker_id {
                    TRACKER_PROBABLY_NOISETRACKER | TRACKER_NOISETRACKER => {
                        libxmp_decode_noisetracker_event(event, ev);
                    }
                    _ => libxmp_decode_protracker_event(event, ev),
                }
                mod_event += 4;
            }
        }
    }
    drop(patbuf);

    /* VBlank detection routine.
     * Despite VBlank being dependent on the tracker used, VBlank detection
     * is complex and uses heuristics mostly independent from tracker ID.
     * See also: the scan.c comparison code enabled by m->compare_vblank
     */
    if !needs_timing_detection {
        /* Noisetracker and some other trackers do not support CIA timing. The
         * only known MOD in the wild that relies on this is muppenkorva.mod
         * by Glue Master (loaded by the His Master's Noise loader).
         */
        if tracker_is_vblank(tracker_id) {
            m.quirk |= QUIRK_NOBPM;
        }
        m.compare_vblank = 0;
    } else if samerow_fxx {
        /* If low Fxx and high Fxx are on the same row, there's a high chance
         * this is from a CIA-based tracker. There are some exceptions.
         */
        if tracker_id == TRACKER_NOISETRACKER
            || tracker_id == TRACKER_PROBABLY_NOISETRACKER
            || tracker_id == TRACKER_SOUNDTRACKER
        {
            tracker_id = TRACKER_UNKNOWN;
        }
        m.compare_vblank = 0;
    } else if high_fxx && m.mod_.len >= 8 {
        /* Test for high Fxx at the end only--this is typically VBlank,
         * and is used to add silence to the end of modules.
         *
         * Exception: if the final high Fxx is F7D, this module is either CIA
         * or is VBlank that was modified to play as CIA, so do nothing.
         *
         * TODO: MPT resets modules on the end loop, so some of the very long
         * silent sections in modules affected by this probably expect CIA. It
         * should eventually be possible to detect those.
         */
        let threshold = m.mod_.len - 2;

        let mut i = 0;
        while i < threshold {
            if pat_high_fxx[m.mod_.xxo[i as usize] as usize] != 0 {
                break;
            }
            i += 1;
        }
        if i == threshold {
            let mut i = m.mod_.len - 1;
            while i >= threshold {
                let fxx = pat_high_fxx[m.mod_.xxo[i as usize] as usize];
                if fxx == 0x00 {
                    i -= 1;
                    continue;
                }
                if fxx == 0x7d {
                    break;
                }

                m.compare_vblank = 0;
                m.quirk |= QUIRK_NOBPM;
                break;
            }
        }
    }

    if invert_loop && detected == 0 && !out_of_range {
        /* If EFx was detected and NO notes were out-of-range,
         * that's a strong indicator of a Protracker origin. */
        if tracker_id != TRACKER_OPENMPT {
            tracker_id = TRACKER_PROTRACKER;
        }
        detected = 1;
    }

    match tracker_id {
        TRACKER_PROTRACKER => {
            tracker = "Protracker";
            ptkloop = true;
        }
        TRACKER_PROBABLY_NOISETRACKER | TRACKER_NOISETRACKER => {
            tracker = "Noisetracker";
        }
        TRACKER_SOUNDTRACKER => {
            tracker = "Soundtracker";
        }
        TRACKER_FASTTRACKER | TRACKER_FASTTRACKER2 => {
            tracker = "Fast Tracker";
            m.period_type = PERIOD_AMIGA;
        }
        TRACKER_TAKETRACKER => {
            tracker = "Take Tracker";
            m.period_type = PERIOD_AMIGA;
        }
        TRACKER_OCTALYSER => {
            tracker = "Octalyser";
            if detected != 0 {
                m.flow_mode = FLOW_MODE_OCTALYSER;
            }
            /* TODO: hexadecimal pattern break parameter. */
        }
        TRACKER_DIGITALTRACKER => {
            tracker = "Digital Tracker";
            m.flow_mode = FLOW_MODE_DTM_2015;
        }
        TRACKER_FLEXTRAX => {
            tracker = "Flextrax";
        }
        TRACKER_MODSGRAVE => {
            tracker = "Mod's Grave";
        }
        TRACKER_SCREAMTRACKER3 => {
            tracker = "Scream Tracker";
            m.period_type = PERIOD_AMIGA;
        }
        TRACKER_SOFTWAREVISIONS => {
            /* ModEdit variant */
            tracker = "Software Visions DMF";
        }
        TRACKER_CONVERTEDST | TRACKER_CONVERTED => {
            tracker = "Converted";
        }
        TRACKER_CLONE => {
            tracker = "Protracker clone";
            m.period_type = PERIOD_AMIGA;
        }
        TRACKER_OPENMPT => {
            tracker = "OpenMPT";
            ptkloop = true;
        }
        /* TRACKER_UNKNOWN_CONV, TRACKER_UNKNOWN */
        _ => {
            tracker = "Unknown tracker";
            m.period_type = PERIOD_AMIGA;
        }
    }

    if out_of_range {
        m.period_type = PERIOD_AMIGA;
    }

    if tracker_id == TRACKER_MODSGRAVE || tracker_id == TRACKER_SOFTWAREVISIONS {
        snprintf_bytes(&mut m.mod_.type_, XMP_NAME_SIZE, tracker.as_bytes());
    } else {
        let mut s = tracker.as_bytes().to_vec();
        s.push(b' ');
        s.extend_from_slice(&magic[..strlen(&magic)]);
        snprintf_bytes(&mut m.mod_.type_, XMP_NAME_SIZE, &s);
    }

    /* Load samples */

    for i in 0..m.mod_.smp as usize {
        if m.mod_.xxs[i].len == 0 {
            continue;
        }

        let mut flags = if ptkloop && m.mod_.xxs[i].lps == 0 {
            SAMPLE_FLAG_FULLREP
        } else {
            0
        };

        if ptsong {
            let mut sp = LibxmpPath::default();
            let mut tmpname = [0u8; 32];
            let instname = m.mod_.xxi[i].name;

            if libxmp_copy_name_for_fopen(&mut tmpname, &instname, 32) != 0 {
                continue;
            }

            libxmp_path_init(&mut sp);
            if libxmp_find_instrument_file(m, &mut sp, &tmpname) != 0 {
                continue;
            }

            let s = hio_open(sp.path.as_deref().unwrap_or(b""));
            libxmp_path_free(&mut sp);
            let Some(mut s) = s else {
                continue;
            };

            let smpctl = m.smpctl;
            if libxmp_load_sample(smpctl, Some(&mut s), flags, &mut m.mod_.xxs[i], None) < 0 {
                return -1;
            }
        } else {
            let mut buf = [0u8; 5];

            let pos = hio_tell(f);
            if pos < 0 {
                return -1;
            }
            let num = hio_read(&mut buf, 1, 5, f);

            if num == 5 && &buf == b"ADPCM" {
                flags |= SAMPLE_FLAG_ADPCM;
            } else {
                hio_seek(f, pos, SEEK_SET);
            }

            let smpctl = m.smpctl;
            if libxmp_load_sample(smpctl, Some(f), flags, &mut m.mod_.xxs[i], None) < 0 {
                return -1;
            }
        }
    }

    if tracker_id == TRACKER_PROTRACKER || tracker_id == TRACKER_OPENMPT {
        m.quirk |= QUIRK_PROTRACK;
    } else if tracker_id == TRACKER_SCREAMTRACKER3 {
        m.c4rate = C4_NTSC_RATE;
        m.quirk |= QUIRKS_ST3;
        m.read_event_type = READ_EVENT_ST3;
    } else if tracker_id == TRACKER_FASTTRACKER
        || tracker_id == TRACKER_FASTTRACKER2
        || tracker_id == TRACKER_TAKETRACKER
        || tracker_id == TRACKER_MODSGRAVE
        || m.mod_.chn > 4
    {
        m.c4rate = C4_NTSC_RATE;
        m.quirk |= QUIRKS_FT2;
        m.read_event_type = READ_EVENT_FT2;
        m.period_type = PERIOD_AMIGA;
    }

    0
}
