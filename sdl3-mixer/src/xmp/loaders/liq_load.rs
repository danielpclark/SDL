// Rust translation of src/loaders/liq_load.c from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2026 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

/* Liquid Tracker module loader based on the format description written
 * by Nir Oren. Tested with Shell.liq sent by Adi Sapir.
 *
 * TODO:
 * - Vibrato, tremolo intensities wrong?
 * - Gxx allows values >64 :(
 * - MCx sets volume to 0
 * - MDx doesn't delay volume?
 * - MEx repeated tick 0s don't act like tick 0
 * - Nxx has some bizarre behavior that prevents it from working between
 *   patterns sometimes and sometimes causes notes to drop out?
 * - Pxx shares effect memory with Lxy, possibly other related awful things.
 * - LIQ portamento is 4x more fine than S3M-compatibility portamento?
 *   (i.e. LIQ-mode UF1 seems equivalent to S3M-mode UE1). LT's S3M loader
 *   renames notes down by two octaves which is possibly implicated here.
 */

//! The Liquid Tracker (LIQ) loader.

use super::super::common::*;
use super::super::effects::*;
use super::super::hio::*;
use super::super::period::libxmp_c2spd_to_note;
use super::super::*;
use super::common::*;
use super::sample::libxmp_load_sample;
use super::*;

/// Translation of `struct liq_header`.
#[derive(Clone, Copy, Default)]
struct LiqHeader {
    magic: [u8; 14],   /* "Liquid Module:" */
    name: [u8; 30],    /* ASCIIZ module name */
    author: [u8; 20],  /* Author name */
    _0x1a: u8,         /* 0x1a */
    tracker: [u8; 20], /* Tracker name */
    version: u16,      /* Format version */
    speed: u16,        /* Initial speed */
    bpm: u16,          /* Initial bpm */
    low: u16,          /* Lowest note (Amiga Period*4) */
    high: u16,         /* Uppest note (Amiga Period*4) */
    chn: u16,          /* Number of channels */
    flags: u32,        /* Module flags */
    pat: u16,          /* Number of patterns saved */
    ins: u16,          /* Number of instruments */
    len: u16,          /* Module length */
    hdrsz: u16,        /* Header size */
}

/*#define LIQ_FLAG_CUT_ON_LIMIT		0x01 */
const LIQ_FLAG_SCREAM_TRACKER_COMPAT: u32 = 0x02;

/// Translation of `struct liq_instrument`.
#[derive(Clone, Copy, Default)]
struct LiqInstrument {
    /* (upstream keeps the magic 'L', 'D', 'S', 'S' here under #if 0) */
    version: u16,       /* LDSS header version */
    name: [u8; 30],     /* Instrument name */
    editor: [u8; 20],   /* Generator name */
    author: [u8; 20],   /* Author name */
    hw_id: u8,          /* Hardware used to record the sample */
    length: u32,        /* Sample length */
    loopstart: u32,     /* Sample loop start */
    loopend: u32,       /* Sample loop end */
    c2spd: u32,         /* C2SPD */
    vol: u8,            /* Volume */
    flags: u8,          /* Flags */
    pan: u8,            /* Pan */
    midi_ins: u8,       /* General MIDI instrument */
    gvl: u8,            /* Global volume */
    chord: u8,          /* Chord type */
    hdrsz: u16,         /* LDSS header size */
    comp: u16,          /* Compression algorithm */
    crc: u32,           /* CRC */
    midi_ch: u8,        /* MIDI channel */
    loop_type: u8,      /* -1 or 0: normal, 1: ping pong*/
    rsvd: [u8; 10],     /* Reserved */
    filename: [u8; 25], /* DOS file name */
}

const LIQ_SAMPLE_16BIT: u8 = 0x01;
const LIQ_SAMPLE_STEREO: u8 = 0x02;
const LIQ_SAMPLE_SIGNED: u8 = 0x04;

/// Translation of `struct liq_pattern`.
#[derive(Clone, Copy, Default)]
struct LiqPattern {
    /* (upstream keeps the magic 'L', 'P', 0, 0 here under #if 0) */
    name: [u8; 30], /* ASCIIZ pattern name */
    rows: u16,      /* Number of rows */
    size: u32,      /* Size of packed pattern */
    reserved: u32,  /* Reserved */
}

pub(crate) const LIBXMP_LOADER_LIQ: FormatLoader = FormatLoader {
    name: "Liquid Tracker",
    test: liq_test,
    loader: liq_load,
};

/// Translation of `liq_test()`.
fn liq_test(f: &mut HioHandle<'_>, t: Option<&mut [u8]>, _start: i32) -> i32 {
    let mut buf = [0u8; 15];

    if hio_read(&mut buf, 1, 14, f) < 14 {
        return -1;
    }

    if &buf[..14] != b"Liquid Module:" {
        return -1;
    }

    libxmp_read_title(f, t, 30);

    0
}

const NONE: u8 = 0xff;

static FX: [u8; 25] = [
    FX_ARPEGGIO,
    FX_S3M_BPM,
    FX_BREAK,
    FX_PORTA_DN,
    NONE,
    FX_FINE_VIBRATO,
    FX_GLOBALVOL,
    NONE,
    NONE,
    FX_JUMP,
    NONE,
    FX_VOLSLIDE,
    FX_EXTENDED,
    FX_TONEPORTA,
    FX_OFFSET,
    FX_SETPAN,
    NONE,
    FX_RETRIG,
    FX_S3M_SPEED,
    FX_TREMOLO,
    FX_PORTA_UP,
    FX_VIBRATO,
    NONE,
    FX_TONE_VSLIDE,
    FX_VIBRA_VSLIDE,
];

/// Translation of `liq_translate_effect()`.
fn liq_translate_effect(e: &mut XmpEvent) {
    let h = msn(e.fxp);
    let mut l = lsn(e.fxp) as u8;

    if e.fxt as usize >= FX.len() {
        e.fxt = 0;
        e.fxp = 0;
        return;
    }

    e.fxt = FX[e.fxt as usize];
    match e.fxt {
        FX_GLOBALVOL => {
            /* Global volume (decimal) */
            l = ((e.fxp >> 4) * 10).wrapping_add(e.fxp & 0x0f);
            e.fxp = l;
        }
        FX_EXTENDED => {
            /* Extended effects */
            match h {
                0x3 => {
                    /* Glissando */
                    e.fxp = l | ((EX_GLISS << 4) as u8);
                }
                0x4 => {
                    /* Vibrato wave */
                    if (l & 3) == 3 {
                        l -= 1;
                    }
                    e.fxp = l | ((EX_VIBRATO_WF << 4) as u8);
                }
                0x5 => {
                    /* Finetune */
                    e.fxp = l | ((EX_FINETUNE << 4) as u8);
                }
                0x6 => {
                    /* Pattern loop */
                    e.fxp = l | ((EX_PATTERN_LOOP << 4) as u8);
                }
                0x7 => {
                    /* Tremolo wave */
                    if (l & 3) == 3 {
                        l -= 1;
                    }
                    e.fxp = l | ((EX_TREMOLO_WF << 4) as u8);
                }
                0xc => {
                    /* Cut */
                    e.fxp = l | ((EX_CUT << 4) as u8);
                }
                0xd => {
                    /* Delay */
                    e.fxp = l | ((EX_DELAY << 4) as u8);
                }
                0xe => {
                    /* Pattern delay */
                    e.fxp = l | ((EX_PATT_DELAY << 4) as u8);
                }
                _ => {
                    /* Ignore */
                    e.fxt = 0;
                    e.fxp = 0;
                }
            }
        }
        FX_SETPAN => {
            /* Pan control */
            l = ((e.fxp >> 4) * 10).wrapping_add(e.fxp & 0x0f);
            if l == 70 {
                /* TODO: if the effective value is 70, reset ALL channels to
                 * default pan positions. */
                e.fxt = 0;
                e.fxp = 0;
            } else if l == 66 {
                e.fxt = FX_SURROUND;
                e.fxp = 1;
            } else if l <= 64 {
                e.fxp = (l as i32 * 0xff / 64) as u8;
            } else {
                e.fxt = 0;
                e.fxp = 0;
            }
        }
        NONE => {
            /* No effect */
            e.fxt = 0;
            e.fxp = 0;
        }
        _ => {}
    }
}

/// Translation of `liq_translate_note()`.
fn liq_translate_note(note: u8, event: &mut XmpEvent) -> i32 {
    if note == 0xfe {
        event.note = XMP_KEY_OFF;
    } else {
        /* 1.00+ format documents claim a 9 octave range, but Liquid Tracker
         * <=1.50 only allows the use of the first 7. 0.00 should be within
         * the NO range of 5 octaves. Either way, they convert the same for
         * now, so just ignore any note that libxmp can't handle. */
        if note > 107 {
            return -1;
        }
        if note as usize + 36 >= XMP_MAX_KEYS {
            return 0;
        }
        event.note = note + 1 + 36;
    }
    0
}

/// Translation of `decode_event()`.
fn decode_event(x1: u8, event: &mut XmpEvent, f: &mut HioHandle<'_>) -> i32 {
    *event = XmpEvent::default();

    if x1 & 0x01 != 0 {
        let x2 = hio_read8(f);
        if liq_translate_note(x2, event) < 0 {
            return -1;
        }
    }

    if x1 & 0x02 != 0 {
        event.ins = hio_read8(f).wrapping_add(1);
    }

    if x1 & 0x04 != 0 {
        event.vol = hio_read8(f).wrapping_add(1);
    }

    if x1 & 0x08 != 0 {
        event.fxt = hio_read8(f).wrapping_sub(b'A');
    } else {
        event.fxt = NONE;
    }

    if x1 & 0x10 != 0 {
        event.fxp = hio_read8(f);
    } else {
        event.fxp = 0xff;
    }

    /* Sanity check */
    if event.ins > 100 || event.vol > 65 {
        return -1;
    }

    liq_translate_effect(event);
    0
}

/// The labels of the pattern reading in `liq_load()`.
enum Label {
    ReadEvent,
    TestEvent,
    NextRow,
}

/// Translation of `liq_load()`.
fn liq_load(m: &mut ModuleData, f: &mut HioHandle<'_>, start: i32) -> i32 {
    let mut lh = LiqHeader::default();
    let mut li = LiqInstrument::default();
    let mut lp = LiqPattern::default();
    let mut tracker_name = [0u8; 21];

    hio_read(&mut lh.magic, 14, 1, f);
    hio_read(&mut lh.name, 30, 1, f);
    hio_read(&mut lh.author, 20, 1, f);
    hio_read8(f);
    hio_read(&mut lh.tracker, 20, 1, f);

    lh.version = hio_read16l(f);
    lh.speed = hio_read16l(f);
    lh.bpm = hio_read16l(f);
    lh.low = hio_read16l(f);
    lh.high = hio_read16l(f);
    lh.chn = hio_read16l(f);
    lh.flags = hio_read32l(f);
    lh.pat = hio_read16l(f);
    lh.ins = hio_read16l(f);
    lh.len = hio_read16l(f);
    lh.hdrsz = hio_read16l(f);

    /* Sanity check */
    if lh.chn as usize > XMP_MAX_CHANNELS || lh.pat > 256 || lh.ins > 256 {
        return -1;
    }

    let num_channels_stored: i32;
    let num_orders_stored: i32;
    if (lh.version >> 8) == 0 {
        lh.hdrsz = lh.len;
        lh.len = 0;
        /* Skip 3 of 5 undocumented bytes (already read 2). */
        hio_seek(f, 3, SEEK_CUR);
        num_channels_stored = 64;
        num_orders_stored = 256;
        if lh.chn > 64 {
            return -1;
        }
    } else {
        num_channels_stored = lh.chn as i32;
        num_orders_stored = lh.len as i32;
    }

    if lh.len > 256 {
        return -1;
    }

    let mod_ = &mut m.mod_;
    mod_.spd = lh.speed as i32;
    mod_.bpm = (lh.bpm as i32).min(255);
    mod_.chn = lh.chn as i32;
    mod_.pat = lh.pat as i32;
    mod_.smp = lh.ins as i32;
    mod_.ins = mod_.smp;
    mod_.len = lh.len as i32;
    mod_.trk = mod_.chn * mod_.pat;

    m.quirk |= QUIRK_INSVOL;

    let mod_ = &mut m.mod_;
    strncpy(&mut mod_.name, &lh.name, 30);
    strncpy(&mut tracker_name, &lh.tracker, 20);
    /* strncpy(m->author, (char *)lh.author, 20); */
    tracker_name[20] = 0;
    for i in (0..=20).rev() {
        if tracker_name[i] == 0x20 {
            tracker_name[i] = 0;
        }
        if tracker_name[i] != 0 {
            break;
        }
    }
    let mut type_ = cstr(&tracker_name).to_vec();
    type_.extend_from_slice(
        format!(" LIQ {}.{:02}", lh.version >> 8, lh.version & 0x00ff).as_bytes(),
    );
    snprintf_bytes(&mut mod_.type_, XMP_NAME_SIZE, &type_);

    let mut i = 0;
    while i < mod_.chn {
        let mut pan = hio_read8(f);

        if pan < 64 {
            pan <<= 2;
        } else if pan == 64 {
            pan = 0xff;
        } else if pan == 66 {
            pan = 0x80;
            mod_.xxc[i as usize].flg |= XMP_CHANNEL_SURROUND;
        } else {
            /* Sanity check */
            return -1;
        }

        mod_.xxc[i as usize].pan = pan as i32;
        i += 1;
    }
    if i < num_channels_stored {
        hio_seek(f, (num_channels_stored - i) as i64, SEEK_CUR);
    }

    let mut i = 0;
    while i < mod_.chn {
        mod_.xxc[i as usize].vol = hio_read8(f) as i32;
        i += 1;
    }
    if i < num_channels_stored {
        hio_seek(f, (num_channels_stored - i) as i64, SEEK_CUR);
    }

    hio_read(&mut mod_.xxo, 1, num_orders_stored as usize, f);

    /* Skip 1.01 echo pools */
    hio_seek(f, start as i64 + lh.hdrsz as i64, SEEK_SET);

    /* Version 0.00 doesn't store length. */
    if lh.version < 0x100 {
        let mut i = 0;
        while i < 256 {
            if mod_.xxo[i] == 0xff {
                break;
            }
            i += 1;
        }
        mod_.len = i as i32;
    }

    m.c4rate = C4_NTSC_RATE;

    let mod_ = &mut m.mod_;
    if libxmp_init_pattern(mod_) < 0 {
        return -1;
    }

    /* Read and convert patterns */

    let mut x1: u8 = 0;
    let mut x2: u8 = 0;
    for i in 0..mod_.pat {
        if libxmp_alloc_pattern(mod_, i) < 0 {
            return -1;
        }

        let pmag = hio_read32b(f);
        /* In spite of the documentation's claims, a magic of !!!! doesn't
         * do anything special here. Liquid Tracker expects a full pattern
         * specification regardless of the magic and no modules use !!!!. */
        if pmag != magic4(b'L', b'P', 0, 0) {
            return -1;
        }

        hio_read(&mut lp.name, 30, 1, f);
        lp.rows = hio_read16l(f);
        lp.size = hio_read32l(f);
        lp.reserved = hio_read32l(f);

        /* Sanity check */
        if lp.rows > 256 {
            return -1;
        }

        mod_.pattern_mut(i).rows = lp.rows as i32;
        libxmp_alloc_tracks_in_pattern(mod_, i);

        let mut row: i32 = 0;
        let mut channel: i32 = 0;
        let count = hio_tell(f);

        /*
         * Packed pattern data is stored full Track after full Track from the left to
         * the right (all Intervals in Track and then going Track right). You should
         * expect 0C0h on any pattern end, and then your Unpacked Patterndata Pointer
         * should be equal to the value in offset [24h]; if it's not, you should exit
         * with an error.
         */

        let mut label = Label::ReadEvent;
        loop {
            match label {
                Label::ReadEvent => {
                    /* Sanity check */
                    if i >= mod_.pat || channel >= mod_.chn || row >= mod_.pattern(i).rows {
                        return -1;
                    }

                    if x2 != 0 {
                        let event = mod_.event_mut(i, channel, row);
                        if decode_event(x1, event, f) < 0 {
                            return -1;
                        }
                        x2 -= 1;
                        label = Label::NextRow;
                        continue;
                    }

                    x1 = hio_read8(f);
                    label = Label::TestEvent;
                }
                Label::TestEvent => {
                    /* Sanity check */
                    if i >= mod_.pat || channel >= mod_.chn || row >= mod_.pattern(i).rows {
                        return -1;
                    }

                    match x1 {
                        0xc0 => {
                            /* end of pattern */
                            if hio_tell(f) - count != lp.size as i64 {
                                return -1;
                            }
                            break; /* next_pattern */
                        }
                        0xe1 | 0xa0 => {
                            if x1 == 0xe1 {
                                /* skip channels */
                                x1 = hio_read8(f);
                                channel += x1 as i32;
                                /* fall thru */
                            }
                            /* next channel */
                            channel += 1;
                            if channel >= mod_.chn {
                                channel -= 1;
                            }
                            row = -1;
                            label = Label::NextRow;
                            continue;
                        }
                        0xe0 | 0x80 => {
                            if x1 == 0xe0 {
                                /* skip rows */
                                x1 = hio_read8(f);
                                row += x1 as i32;
                                /* fall thru */
                            }
                            /* next row */
                            label = Label::NextRow;
                            continue;
                        }
                        _ => {}
                    }

                    if x1 > 0xc0 && x1 < 0xe0 {
                        /* packed data */
                        let event = mod_.event_mut(i, channel, row);
                        if decode_event(x1, event, f) < 0 {
                            return -1;
                        }
                        label = Label::NextRow;
                        continue;
                    }

                    if x1 > 0xa0 && x1 < 0xc0 {
                        /* packed data repeat */
                        x2 = hio_read8(f);
                        let event = mod_.event_mut(i, channel, row);
                        if decode_event(x1, event, f) < 0 {
                            return -1;
                        }
                        label = Label::NextRow;
                        continue;
                    }

                    if x1 > 0x80 && x1 < 0xa0 {
                        /* packed data repeat, keep note */
                        x2 = hio_read8(f);
                        let event = mod_.event_mut(i, channel, row);
                        if decode_event(x1, event, f) < 0 {
                            return -1;
                        }
                        let ev = *event;
                        while x2 != 0 {
                            row += 1;

                            /* Sanity check */
                            if row >= lp.rows as i32 {
                                return -1;
                            }

                            *mod_.event_mut(i, channel, row) = ev;
                            x2 -= 1;
                        }
                        label = Label::NextRow;
                        continue;
                    }

                    /* unpacked data */
                    let event = mod_.event_mut(i, channel, row);
                    if liq_translate_note(x1, event) < 0 {
                        return -1;
                    }

                    x1 = hio_read8(f);
                    if x1 > 100 {
                        row += 1;
                        label = Label::TestEvent;
                        continue;
                    }
                    if x1 != 0xff {
                        event.ins = x1 + 1;
                    }

                    x1 = hio_read8(f);
                    if x1 != 0xff {
                        event.vol = x1.wrapping_add(1);
                    }

                    x1 = hio_read8(f);
                    if x1 != 0xff {
                        event.fxt = x1.wrapping_sub(b'A');
                    }

                    x1 = hio_read8(f);
                    event.fxp = x1;

                    /* Sanity check */
                    if event.ins > 100 || event.vol > 65 {
                        return -1;
                    }

                    liq_translate_effect(event);
                    label = Label::NextRow;
                }
                Label::NextRow => {
                    row += 1;
                    if row >= mod_.pattern(i).rows {
                        row = 0;
                        x2 = 0;
                        channel += 1;
                    }

                    /* Sanity check */
                    if channel >= mod_.chn {
                        channel = 0;
                    }

                    label = Label::ReadEvent;
                }
            }
        }
    }

    /* Read and convert instruments */

    if libxmp_init_instrument(m) < 0 {
        return -1;
    }

    for i in 0..m.mod_.ins {
        let iu = i as usize;
        let mod_ = &mut m.mod_;

        if libxmp_alloc_subinstrument(mod_, i, 1) < 0 {
            return -1;
        }

        let ldss_magic = hio_read32b(f);
        if ldss_magic == magic4(b'?', b'?', b'?', b'?') {
            continue;
        }
        if ldss_magic != magic4(b'L', b'D', b'S', b'S') {
            return -1;
        }

        li.version = hio_read16l(f);
        hio_read(&mut li.name, 30, 1, f);
        hio_read(&mut li.editor, 20, 1, f);
        hio_read(&mut li.author, 20, 1, f);
        li.hw_id = hio_read8(f);

        li.length = hio_read32l(f);
        li.loopstart = hio_read32l(f);
        li.loopend = hio_read32l(f);
        li.c2spd = hio_read32l(f);

        li.vol = hio_read8(f);
        li.flags = hio_read8(f);
        li.pan = hio_read8(f);
        li.midi_ins = hio_read8(f);
        li.gvl = hio_read8(f);
        li.chord = hio_read8(f);

        li.hdrsz = hio_read16l(f);
        li.comp = hio_read16l(f);
        li.crc = hio_read32l(f);

        li.midi_ch = hio_read8(f);
        li.loop_type = hio_read8(f);
        hio_read(&mut li.rsvd, 1, 10, f);
        hio_read(&mut li.filename, 25, 1, f);

        /* Sanity check */
        if hio_error(f) != 0 {
            return -1;
        }

        let xxi = &mut mod_.xxi[iu];
        xxi.nsm = (li.length != 0) as i32;
        xxi.vol = 0x40;

        let xxs = &mut mod_.xxs[iu];
        xxs.len = li.length as i32;
        xxs.lps = li.loopstart as i32;
        xxs.lpe = li.loopend as i32;

        /* Note: LIQ_SAMPLE_SIGNED is ignored by Liquid Tracker 1.50;
         * all samples are interpreted as signed. */
        if li.flags & LIQ_SAMPLE_16BIT != 0 {
            xxs.flg |= XMP_SAMPLE_16BIT;
            xxs.len >>= 1;
            xxs.lps >>= 1;
            xxs.lpe >>= 1;
        }
        /* Storage is the same as S3M i.e. left then right. The shareware
         * version mixes stereo samples to mono, as stereo samples were
         * listed as a registered feature (yet to be verified). */
        if li.flags & LIQ_SAMPLE_STEREO != 0 {
            xxs.flg |= XMP_SAMPLE_STEREO;
            xxs.len >>= 1;
            xxs.lps >>= 1;
            xxs.lpe >>= 1;
        }

        if li.loopend > 0 {
            xxs.flg |= XMP_SAMPLE_LOOP;
            if li.loop_type == 1 {
                xxs.flg |= XMP_SAMPLE_LOOP_BIDIR;
            }
        }

        /* Global volume was added(?) in LDSS 1.01 and, like the channel
         * volume, has a range of 0-64 with 32=100%. */
        if li.version < 0x101 {
            li.gvl = 0x20;
        }

        let sub = &mut mod_.xxi[iu].sub[0];
        sub.vol = li.vol as i32;
        sub.gvl = li.gvl as i32;
        /* TODO: 66 = surround */
        sub.pan = if li.pan <= 64 {
            (li.pan as i32 * 4).min(255)
        } else {
            XMP_INST_NO_DEFAULT_PAN
        };
        sub.sid = i;

        libxmp_instrument_name(mod_, i, &li.name, 30);

        let sub = &mut mod_.xxi[iu].sub[0];
        libxmp_c2spd_to_note(li.c2spd as i32, &mut sub.xpo, &mut sub.fin);
        hio_seek(f, li.hdrsz as i64 - 0x90, SEEK_CUR);

        if mod_.xxs[iu].len == 0 {
            continue;
        }

        let smpctl = m.smpctl;
        if libxmp_load_sample(smpctl, Some(f), 0, &mut m.mod_.xxs[iu], None) < 0 {
            return -1;
        }
    }

    m.quirk |= QUIRK_FINEFX | QUIRK_RTONCE;
    m.flow_mode = if lh.flags & LIQ_FLAG_SCREAM_TRACKER_COMPAT != 0 {
        FLOW_MODE_LIQUID_COMPAT
    } else {
        FLOW_MODE_LIQUID
    };
    m.read_event_type = READ_EVENT_ST3;

    /* Channel volume and instrument global volume are both "normally" 32 and
     * can be increased to 64, effectively allowing per-channel and
     * per-instrument gain of 2x each. Simulate this by keeping the original
     * volumes while increasing mix volume by 2x each.
     *
     * The global volume effect also (unintentionally?) has gain functionality
     * in the sense that values >64 are not ignored. This isn't supported yet.
     */
    m.mvol = 48 * 2 * 2;
    m.mvolbase = 48;

    0
}
