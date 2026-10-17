// Rust translation of src/loaders/stm_load.c from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2026 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Scream Tracker 2 (STM) loader.

use super::super::common::*;
use super::super::effects::*;
use super::super::hio::*;
use super::super::path::*;
use super::super::period::libxmp_c2spd_to_note;
use super::super::*;
use super::common::*;
use super::sample::libxmp_load_sample;
use super::*;

/* STM tempo is calculated based on the mix buffer size, which requires knowing
 * the mix rate of ST2. This is the highest mix rate in released versions. */
const STM_MIX_RATE: i32 = 23863;

const STM_TYPE_SONG: u8 = 0x01;
const STM_TYPE_MODULE: u8 = 0x02;

/// Translation of `struct stm_instrument_header`.
#[derive(Clone, Copy, Default)]
struct StmInstrumentHeader {
    name: [u8; 13], /* Instrument name in 8.3 format (ASCIIZ) */
    idisk: u8,      /* Instrument disk */
    rsvd1: u16,     /* Reserved */
    length: u16,    /* Sample length */
    loopbeg: u16,   /* Loop begin */
    loopend: u16,   /* Loop end */
    volume: u8,     /* Playback volume */
    rsvd2: u8,      /* Reserved */
    c2spd: u16,     /* C4 speed */
    rsvd3: u32,     /* Reserved */
    paralen: u16,   /* Length in paragraphs */
}

/* v1 format header based on disassembled ST2 */
/// Translation of `struct stm_file_subheader_v1`.
#[derive(Clone, Copy, Default)]
struct StmFileSubheaderV1 {
    insnum: u16,  /* Number of instruments */
    ordnum: u16,  /* Number of orders */
    patnum: u16,  /* Number of patterns */
    srate: u16,   /* Sample rate? */
    tempo: u8,    /* Playback tempo */
    channels: u8, /* Number of channels */
    psize: u16,   /* Pattern size */
    rsvd2: u16,   /* Reserved */
    skip: u16,    /* Bytes to skip */
}

/// Translation of `struct stm_file_subheader_v2`.
#[derive(Clone, Copy, Default)]
struct StmFileSubheaderV2 {
    tempo: u8,       /* Playback tempo */
    patterns: u8,    /* Number of patterns */
    gvol: u8,        /* Global volume */
    rsvd2: [u8; 13], /* Reserved */
}

/// Translation of `struct stm_file_header` (the union of the two
/// subheaders is both of them here).
#[derive(Clone, Copy, Default)]
struct StmFileHeader {
    name: [u8; 20], /* ASCIIZ song name */
    magic: [u8; 8], /* '!Scream!' */
    rsvd1: u8,      /* '\x1a' */
    type_: u8,      /* 1=song, 2=module */
    vermaj: u8,     /* Major version number */
    vermin: u8,     /* Minor version number */
    v1: StmFileSubheaderV1,
    v2: StmFileSubheaderV2,
    ins: [StmInstrumentHeader; 32],
}

pub(crate) const LIBXMP_LOADER_STM: FormatLoader = FormatLoader {
    name: "Scream Tracker 2",
    test: stm_test,
    loader: stm_load,
};

/// Translation of `stm_test()`.
fn stm_test(f: &mut HioHandle<'_>, t: Option<&mut [u8]>, start: i32) -> i32 {
    let mut buf = [0u8; 8];

    hio_seek(f, start as i64 + 20, SEEK_SET);
    if hio_read(&mut buf, 1, 8, f) < 8 {
        return -1;
    }

    if libxmp_test_name(&buf, 8, 0) != 0 {
        /* Tracker name should be ASCII */
        return -1;
    }

    /* EOF should be 0x1a. putup10.stm and putup11.stm have 2 instead. */
    buf[0] = hio_read8(f);
    if buf[0] != 0x1a && buf[0] != 0x02 {
        return -1;
    }

    if hio_read8(f) > STM_TYPE_MODULE {
        return -1;
    }

    buf[0] = hio_read8(f);
    buf[1] = hio_read8(f);
    let version = (100 * buf[0] as u16).wrapping_add(buf[1] as u16);

    if version != 110 && version != 200 && version != 210 && version != 220 && version != 221 {
        return -1;
    }

    hio_seek(f, start as i64 + 60, SEEK_SET);
    if hio_read(&mut buf, 1, 4, f) < 4 {
        return -1;
    }
    if &buf[..4] == b"SCRM" {
        /* We don't want STX files */
        return -1;
    }

    hio_seek(f, start as i64, SEEK_SET);
    libxmp_read_title(f, t, 20);

    0
}

const FX_NONE: u8 = 0xff;

/*
 * Skaven's note from http://www.futurecrew.com/skaven/oldies_music.html
 *
 * FYI for the tech-heads: In the old Scream Tracker 2 the Arpeggio command
 * (Jxx), if used in a single row with a 0x value, caused the note to skip
 * the specified amount of halftones upwards halfway through the row. I used
 * this in some songs to give the lead some character. However, when played
 * in ModPlug Tracker, this effect doesn't work the way it did back then.
 */
static FX: [u8; 16] = [
    FX_NONE,
    FX_SPEED,     /* A - Set tempo to [INFO]. 60 normal. */
    FX_JUMP,      /* B - Break pattern and jmp to order [INFO] */
    FX_BREAK,     /* C - Break pattern */
    FX_VOLSLIDE,  /* D - Slide volume; Hi-nibble=up, Lo-nibble=down */
    FX_PORTA_DN,  /* E - Slide down at speed [INFO] */
    FX_PORTA_UP,  /* F - Slide up at speed [INFO] */
    FX_TONEPORTA, /* G - Slide to the note specified at speed [INFO] */
    FX_VIBRATO,   /* H - Vibrato; Hi-nibble, speed. Lo-nibble, size */
    FX_TREMOR,    /* I - Tremor; Hi-nibble, ontime. Lo-nibble, offtime */
    FX_ARPEGGIO,  /* J - Arpeggio */
    FX_NONE,
    FX_NONE,
    FX_NONE,
    FX_NONE,
    FX_NONE,
];

/* Following cs127's research fairly closely here out of necessity. Notes:
 * - maximum effective BPM is 125.
 * - divisor may be negative.
 * - the potentially negative quotient of this division may be modified,
 *   thus the mix rate can not be optimized out.
 * - TODO: tempo effects take effect the next tick, not the current tick.
 * - TODO: tempos 1E, 1F, and 05-0F result in BPMs lower than XMP_MIN_BPM.
 */
/// Translation of `stm_calculate_bpm()`.
fn stm_calculate_bpm(spd: i32, tempo_factor: i32) -> i32 {
    static SPEED_FACTOR: [u8; 16] = [140, 50, 25, 15, 10, 7, 6, 4, 3, 3, 2, 2, 2, 2, 1, 1];
    let divisor = 50 - ((SPEED_FACTOR[spd as usize] as i32 * tempo_factor) >> 4);

    /* Safety check; 0 should not be possible */
    let mut tick_frames = if divisor != 0 {
        STM_MIX_RATE / divisor
    } else {
        -1
    };
    tick_frames = ((tick_frames as u32) & 0xffff) as i32;

    /* TODO: BPMs can be non-integer. */
    (STM_MIX_RATE * 5 + tick_frames/*round*/) / (tick_frames * 2)
}

/// Translation of `stm_convert_tempo()`.
fn stm_convert_tempo(spd: &mut i32, bpm: &mut i32, tempo: u32, version: i32) {
    if version >= 221 {
        *spd = msn(tempo as u8);
        *bpm = stm_calculate_bpm(*spd, lsn(tempo as u8));
    } else if version >= 110 {
        /* TODO: ST <2.2 behavior unclear. ST 2.2+ imports higher
         * speeds (effect speed is modulo 16) and then reads the
         * speed factor table out-of-bounds. */
        *spd = (tempo / 10) as i32;
        *bpm = stm_calculate_bpm((*spd).min(15), (tempo % 10) as i32);
    } else {
        *spd = tempo as i32;
        *bpm = 125;
    }
    *spd = (*spd).max(1);
    *bpm = (*bpm).max(XMP_MIN_BPM);
}

/// Translation of `stm_load()`.
fn stm_load(m: &mut ModuleData, f: &mut HioHandle<'_>, start: i32) -> i32 {
    let mut sfh = StmFileHeader::default();
    let mut blank_pattern = false;
    let mut spd = 0;
    let mut bpm = 0;

    hio_read(&mut sfh.name, 20, 1, f); /* ASCIIZ song name */
    hio_read(&mut sfh.magic, 8, 1, f); /* '!Scream!' */
    sfh.rsvd1 = hio_read8(f); /* '\x1a' */
    sfh.type_ = hio_read8(f); /* 1=song, 2=module */
    sfh.vermaj = hio_read8(f); /* Major version number */
    sfh.vermin = hio_read8(f); /* Minor version number */
    let version = (100 * sfh.vermaj as u16).wrapping_add(sfh.vermin as u16) as i32;

    if version != 110 && version != 200 && version != 210 && version != 220 && version != 221 {
        return -1;
    }

    // TODO: improve robustness of the loader against bad parameters

    let mod_ = &mut m.mod_;
    if version >= 200 {
        sfh.v2.tempo = hio_read8(f); /* Playback tempo */
        sfh.v2.patterns = hio_read8(f); /* Number of patterns */
        sfh.v2.gvol = hio_read8(f); /* Global volume */
        hio_read(&mut sfh.v2.rsvd2, 13, 1, f); /* Reserved */
        mod_.chn = 4;
        mod_.pat = sfh.v2.patterns as i32;

        stm_convert_tempo(&mut mod_.spd, &mut mod_.bpm, sfh.v2.tempo as u32, version);
        mod_.ins = 31;
        mod_.len = if version == 200 { 64 } else { 128 };
    } else {
        sfh.v1.insnum = hio_read16l(f);
        if sfh.v1.insnum > 32 {
            /* Number of instruments */
            return -1;
        }
        sfh.v1.ordnum = hio_read16l(f);
        if sfh.v1.ordnum as usize > XMP_MAX_MOD_LENGTH {
            /* Number of orders */
            return -1;
        }
        sfh.v1.patnum = hio_read16l(f);
        if sfh.v1.patnum as usize > XMP_MAX_MOD_LENGTH {
            /* Number of patterns */
            return -1;
        }
        sfh.v1.srate = hio_read16l(f); /* Sample rate? */
        sfh.v1.tempo = hio_read8(f); /* Playback tempo */
        sfh.v1.channels = hio_read8(f);
        if sfh.v1.channels != 4 {
            /* Number of channels */
            return -1;
        }
        sfh.v1.psize = hio_read16l(f);
        if sfh.v1.psize != 64 {
            /* Pattern size */
            return -1;
        }
        sfh.v1.rsvd2 = hio_read16l(f); /* Reserved */
        sfh.v1.skip = hio_read16l(f); /* Bytes to skip */
        hio_seek(f, sfh.v1.skip as i64, SEEK_CUR); /* Skip bytes */
        mod_.chn = sfh.v1.channels as i32;
        mod_.pat = sfh.v1.patnum as i32;

        stm_convert_tempo(&mut mod_.spd, &mut mod_.bpm, sfh.v1.tempo as u32, version);
        mod_.ins = sfh.v1.insnum as i32;
        mod_.len = sfh.v1.ordnum as i32;
    }

    for i in 0..mod_.ins as usize {
        let ins = &mut sfh.ins[i];
        hio_read(&mut ins.name, 13, 1, f); /* Instrument name */
        ins.idisk = hio_read8(f); /* Instrument disk */
        ins.rsvd1 = hio_read16l(f); /* Reserved */
        ins.length = hio_read16l(f); /* Sample length */
        ins.loopbeg = hio_read16l(f); /* Loop begin */
        ins.loopend = hio_read16l(f); /* Loop end */
        ins.volume = hio_read8(f); /* Playback volume */
        ins.rsvd2 = hio_read8(f); /* Reserved */
        ins.c2spd = hio_read16l(f); /* C4 speed */
        ins.rsvd3 = hio_read32l(f); /* Reserved */
        ins.paralen = hio_read16l(f); /* Length in paragraphs */
    }

    if hio_error(f) != 0 {
        return -1;
    }

    mod_.smp = mod_.ins;
    m.c4rate = C4_NTSC_RATE;

    libxmp_copy_adjust(&mut m.mod_.name, &sfh.name, 20);

    if sfh.magic[0] == 0 || strneq(&sfh.magic, b"PCSTV", 5) || strneq(&sfh.magic, b"!Scream!", 8) {
        let s = format!("Scream Tracker {}.{:02}", sfh.vermaj, sfh.vermin);
        libxmp_set_type(m, s.as_bytes());
    } else if strneq(&sfh.magic, b"SWavePro", 8) {
        let s = format!("SoundWave Pro {}.{:02}", sfh.vermaj, sfh.vermin);
        libxmp_set_type(m, s.as_bytes());
    } else {
        libxmp_copy_adjust(&mut m.mod_.type_, &sfh.magic, 8);
    }

    if libxmp_init_instrument(m) < 0 {
        return -1;
    }

    let mod_ = &mut m.mod_;
    /* Read and convert instruments and samples */
    for i in 0..mod_.ins {
        let iu = i as usize;
        if libxmp_alloc_subinstrument(mod_, i, 1) < 0 {
            return -1;
        }

        let ins = &sfh.ins[iu];
        let xxs = &mut mod_.xxs[iu];
        xxs.len = ins.length as i32;
        xxs.lps = ins.loopbeg as i32;
        xxs.lpe = ins.loopend as i32;
        if xxs.lpe == 0xffff {
            xxs.lpe = 0;
        }
        xxs.flg = if xxs.lpe > 0 { XMP_SAMPLE_LOOP } else { 0 };
        let len = xxs.len;
        let xxi = &mut mod_.xxi[iu];
        xxi.sub[0].vol = ins.volume as i32;
        xxi.sub[0].pan = XMP_INST_NO_DEFAULT_PAN;
        xxi.sub[0].sid = i;

        if len > 0 {
            xxi.nsm = 1;
        }

        libxmp_instrument_name(mod_, i, &ins.name, 12);

        let sub = &mut mod_.xxi[iu].sub[0];
        libxmp_c2spd_to_note(ins.c2spd as i32, &mut sub.xpo, &mut sub.fin);
    }

    let len = mod_.len as usize;
    if hio_read(&mut mod_.xxo, 1, len, f) < len {
        return -1;
    }

    let mut i = 0;
    while i < mod_.len as usize {
        if mod_.xxo[i] >= 99 {
            break;
        }
        /* Patterns >= the pattern count are valid blank patterns.
         * Examples: jimmy.stm, Rauno/dogs.stm, Skaven/hevijanis istu maas.stm.
         * Patterns >= 64 have undefined behavior in Screamtracker 2.
         */
        if mod_.xxo[i] as i32 >= mod_.pat {
            mod_.xxo[i] = mod_.pat as u8;
            blank_pattern = true;
        }
        i += 1;
    }
    let stored_patterns = mod_.pat;
    if blank_pattern {
        mod_.pat += 1;
    }

    mod_.trk = mod_.pat * mod_.chn;
    mod_.len = i as i32;

    if libxmp_init_pattern(mod_) < 0 {
        return -1;
    }

    /* Read and convert patterns */

    if blank_pattern && libxmp_alloc_pattern_tracks(mod_, stored_patterns, 64) < 0 {
        return -1;
    }

    for i in 0..stored_patterns {
        if libxmp_alloc_pattern_tracks(mod_, i, 64) < 0 {
            return -1;
        }

        if hio_error(f) != 0 {
            return -1;
        }

        for j in 0..64 {
            for k in 0..mod_.chn {
                let event = mod_.event_mut(i, k, j);
                let mut b = hio_read8(f);
                if b == 251 || b == 252 {
                    continue; /* Empty note */
                }

                if b == 253 {
                    event.note = XMP_KEY_OFF;
                    continue; /* Key off */
                }

                if b == 254 {
                    event.note = XMP_KEY_OFF;
                } else if b == 255 {
                    event.note = 0;
                } else {
                    event.note = (1 + lsn(b) + 12 * (3 + msn(b))) as u8;
                }

                b = hio_read8(f);
                event.vol = b & 0x07;
                event.ins = (b & 0xf8) >> 3;

                b = hio_read8(f);
                event.vol = event.vol.wrapping_add((b & 0xf0) >> 1);
                if version >= 200 {
                    event.vol = if event.vol > 0x40 { 0 } else { event.vol + 1 };
                } else if event.vol > 0 {
                    event.vol = if event.vol > 0x40 { 1 } else { event.vol + 1 };
                }

                event.fxt = FX[lsn(b) as usize];
                event.fxp = hio_read8(f);
                match event.fxt {
                    FX_BREAK => {
                        /* ST2 always breaks to row 0 */
                        event.fxp = 0;
                    }
                    FX_SPEED => {
                        if event.fxp != 0 {
                            stm_convert_tempo(&mut spd, &mut bpm, event.fxp as u32, version);
                            event.fxt = FX_S3M_SPEED;
                            event.fxp = spd as u8;
                            if version >= 110 {
                                event.f2t = FX_S3M_BPM;
                                event.f2p = bpm as u8;
                            }
                        } else {
                            /* A00 is a no-op */
                            event.fxt = 0;
                            event.fxp = 0;
                        }
                    }
                    FX_NONE => {
                        event.fxp = 0;
                        event.fxt = 0;
                    }
                    _ => {}
                }
            }
        }
    }

    /* Read samples */

    for i in 0..m.mod_.ins as usize {
        if sfh.ins[i].volume == 0 || sfh.ins[i].length == 0 {
            m.mod_.xxi[i].nsm = 0;
            continue;
        }

        if sfh.type_ == STM_TYPE_SONG {
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

            let s = sp.path.as_deref().and_then(hio_open);
            libxmp_path_free(&mut sp);
            let Some(mut s) = s else {
                continue;
            };

            let smpctl = m.smpctl;
            if libxmp_load_sample(
                smpctl,
                Some(&mut s),
                SAMPLE_FLAG_UNS,
                &mut m.mod_.xxs[i],
                None,
            ) < 0
            {
                return -1;
            }
        } else {
            hio_seek(f, start as i64 + ((sfh.ins[i].rsvd1 as i64) << 4), SEEK_SET);
            let smpctl = m.smpctl;
            if libxmp_load_sample(smpctl, Some(f), 0, &mut m.mod_.xxs[i], None) < 0 {
                return -1;
            }
        }
    }

    m.quirk |= QUIRK_VSALL | QUIRKS_ST3;
    m.flow_mode = FLOW_MODE_ST2;
    m.read_event_type = READ_EVENT_ST3;

    0
}
