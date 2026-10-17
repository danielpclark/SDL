// Rust translation of src/loaders/dbm_load.c from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2026 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

/* Based on DigiBooster_E.guide from the DigiBoosterPro 2.20 package.
 * DigiBooster Pro written by Tomasz & Waldemar Piasta
 */

//! The DigiBooster Pro (DBM0) loader.

use super::super::common::*;
use super::super::dataio::*;
use super::super::effects::*;
use super::super::hio::*;
use super::super::period::libxmp_c2spd_to_note;
use super::super::*;
use super::common::*;
use super::iff::*;
use super::sample::libxmp_load_sample;
use super::*;

const MAGIC_DBM0: u32 = magic4(b'D', b'B', b'M', b'0');

pub(crate) const LIBXMP_LOADER_DBM: FormatLoader = FormatLoader {
    name: "DigiBooster Pro",
    test: dbm_test,
    loader: dbm_load,
};

/// Translation of `dbm_test()`.
fn dbm_test(f: &mut HioHandle<'_>, t: Option<&mut [u8]>, _start: i32) -> i32 {
    if hio_read32b(f) != MAGIC_DBM0 {
        return -1;
    }

    hio_seek(f, 12, SEEK_CUR);
    libxmp_read_title(f, t, 44);

    0
}

const DBM_INSTRUMENT_LOOP: i32 = 1 << 0;
const DBM_INSTRUMENT_LOOP_BIDIR: i32 = 1 << 1;

const DBM_SAMPLE_8BIT: u32 = 1 << 0;
const DBM_SAMPLE_16BIT: u32 = 1 << 1;
const DBM_SAMPLE_32BIT: u32 = 1 << 2;

/// Translation of `struct local_data`.
#[derive(Default)]
struct LocalData {
    have_info: bool,
    have_song: bool,
    have_patt: bool,
    have_smpl: bool,
    have_inst: bool,
    have_venv: bool,
    have_penv: bool,
    maj_version: i32,
    min_version: i32,
}

/// Translation of `struct dbm_envelope_node`.
#[derive(Clone, Copy, Default)]
struct DbmEnvelopeNode {
    position: u16,
    value: i16,
}

/// Translation of `struct dbm_envelope`.
#[derive(Clone, Copy, Default)]
struct DbmEnvelope {
    ins: i32,
    flg: i32,
    npt: i32,
    sus: i32,
    lps: i32,
    lpe: i32,
    sus2: i32,
    nodes: [DbmEnvelopeNode; 32],
}

/// Translation of `dbm_translate_effect()` (`event` is given as its note).
fn dbm_translate_effect(note: u8, fxt: &mut u8, fxp: &mut u8) {
    match *fxt {
        0x0e => match msn(*fxp) {
            0x3 => {
                /* Play from backward */
                /* TODO: this is supposed to play the sample in
                 * reverse only once, then forward. */
                if note != 0 {
                    *fxt = FX_REVERSE;
                    *fxp = 1;
                } else {
                    *fxt = 0;
                    *fxp = 0;
                }
            }
            0x4 => {
                /* Turn off sound in channel */
                *fxt = FX_EXTENDED;
                *fxp = (EX_CUT << 4) as u8;
            }
            0x5 => {
                /* Turn on/off channel */
                /* In DigiBooster Pro, this is tied to
                 * the channel mute toggle in the UI. */
                *fxt = FX_TRK_VOL;
                *fxp = if *fxp != 0 { 0x40 } else { 0x00 };
            }
            _ => {}
        },

        0x1c => {
            /* Set Real BPM */
            *fxt = FX_S3M_BPM;
        }

        _ => {
            if *fxt > 0x1c {
                *fxt = 0;
                *fxp = 0;
            }
        }
    }
}

/// Translation of `get_info()`.
fn get_info(m: &mut ModuleData, size: u32, f: &mut HioHandle<'_>, data: &mut LocalData) -> i32 {
    /* Sanity check */
    if data.have_info || size < 10 {
        return -1;
    }
    data.have_info = true;

    let mod_ = &mut m.mod_;
    let val = hio_read16b(f) as i32;
    if !(0..=255).contains(&val) {
        return -1;
    }
    mod_.ins = val;

    let val = hio_read16b(f) as i32;
    /* (never negative) */
    mod_.smp = val;

    hio_read16b(f); /* Songs */

    let val = hio_read16b(f) as i32;
    if !(0..=256).contains(&val) {
        mod_.smp = 0;
        mod_.ins = 0;
        return -1;
    }
    mod_.pat = val;

    let val = hio_read16b(f) as i32;
    if !(0..=XMP_MAX_CHANNELS as i32).contains(&val) {
        mod_.pat = 0;
        mod_.smp = 0;
        mod_.ins = 0;
        return -1;
    }
    mod_.chn = val;

    mod_.trk = mod_.pat * mod_.chn;

    if libxmp_init_instrument(m) < 0 {
        return -1;
    }

    0
}

/// Translation of `get_song()`.
fn get_song(m: &mut ModuleData, size: u32, f: &mut HioHandle<'_>, data: &mut LocalData) -> i32 {
    let mut buffer = [0u8; 50];

    /* Sanity check */
    if data.have_song || size < 46 {
        return 0;
    }
    data.have_song = true;

    hio_read(&mut buffer, 44, 1, f);

    let mod_ = &mut m.mod_;
    mod_.len = hio_read16b(f) as i32;

    /* Sanity check */
    if mod_.len > 256 {
        return -1;
    }

    for i in 0..mod_.len as usize {
        mod_.xxo[i] = hio_read16b(f) as u8;
    }

    0
}

/// Translation of `get_inst()`.
fn get_inst(m: &mut ModuleData, size: u32, f: &mut HioHandle<'_>, data: &mut LocalData) -> i32 {
    let mod_ = &mut m.mod_;
    let mut buffer = [0u8; 50];

    /* Sanity check */
    if data.have_inst || (size as i64) < 50 * mod_.ins as i64 {
        return -1;
    }
    data.have_inst = true;

    for i in 0..mod_.ins {
        let iu = i as usize;

        mod_.xxi[iu].nsm = 1;
        if libxmp_alloc_subinstrument(mod_, i, 1) < 0 {
            return -1;
        }

        if hio_read(&mut buffer, 1, 50, f) < 50 {
            return -1;
        }

        libxmp_instrument_name(mod_, i, &buffer, 30);
        let mut snum = readmem16b(&buffer[30..]) as i32;
        if snum == 0 || snum > mod_.smp {
            /* Skip remaining data for this instrument. */
            continue;
        }
        snum -= 1;
        let sub = &mut mod_.xxi[iu].sub[0];
        sub.sid = snum;
        let xxs = &mut mod_.xxs[snum as usize];

        sub.vol = readmem16b(&buffer[32..]) as i32;
        let c2spd = readmem32b(&buffer[34..]) as i32;
        xxs.lps = readmem32b(&buffer[38..]) as i32;
        xxs.lpe = (xxs.lps as u32).wrapping_add(readmem32b(&buffer[42..])) as i32;
        /* Format documentation incorrectly states pan is 0-255. */
        sub.pan = 0x80 + readmem16b(&buffer[46..]) as i16 as i32;
        let flags = readmem16b(&buffer[48..]) as i32;

        if flags & (DBM_INSTRUMENT_LOOP | DBM_INSTRUMENT_LOOP_BIDIR) != 0 {
            xxs.flg |= XMP_SAMPLE_LOOP;
        }
        if flags & DBM_INSTRUMENT_LOOP_BIDIR != 0 {
            xxs.flg |= XMP_SAMPLE_LOOP_BIDIR;
        }

        clamp(&mut sub.vol, 0, 64);
        clamp(&mut sub.pan, 0, 255);

        libxmp_c2spd_to_note(c2spd, &mut sub.xpo, &mut sub.fin);
    }

    0
}

/// Translation of `get_patt()`.
fn get_patt(m: &mut ModuleData, _size: u32, f: &mut HioHandle<'_>, data: &mut LocalData) -> i32 {
    let mod_ = &mut m.mod_;
    let mut dummy = XmpEvent::default();

    /* Sanity check */
    if data.have_patt || !data.have_info {
        return -1;
    }
    data.have_patt = true;

    if libxmp_init_pattern(mod_) < 0 {
        return -1;
    }

    /*
     * Note: channel and flag bytes are inverted in the format
     * description document
     */

    for i in 0..mod_.pat {
        let rows = hio_read16b(f) as i32;
        if hio_error(f) != 0 {
            return -1;
        }

        if libxmp_alloc_pattern_tracks(mod_, i, rows) < 0 {
            return -1;
        }

        let mut sz = hio_read32b(f) as i32;

        let mut r = 0;
        /*c = -1;*/

        while sz > 0 {
            sz -= 1;
            let mut c = hio_read8(f) as i32;
            if hio_error(f) != 0 {
                return -1;
            }

            if c == 0 {
                r += 1;
                continue;
            }
            c -= 1;

            sz -= 1;
            if sz < 0 {
                break;
            }
            let n = hio_read8(f);

            let event = if c >= mod_.chn || r >= mod_.pattern(i).rows {
                &mut dummy
            } else {
                mod_.event_mut(i, c, r)
            };

            *event = XmpEvent::default();

            if n & 0x01 != 0 && {
                sz -= 1;
                sz >= 0
            } {
                let x = hio_read8(f);
                if x == 0x1f {
                    event.note = XMP_KEY_OFF;
                } else {
                    event.note = (13 + msn(x) * 12 + lsn(x)) as u8;
                }
            }
            if n & 0x02 != 0 && {
                sz -= 1;
                sz >= 0
            } {
                event.ins = hio_read8(f);
            }
            if n & 0x04 != 0 && {
                sz -= 1;
                sz >= 0
            } {
                event.fxt = hio_read8(f);
            }
            if n & 0x08 != 0 && {
                sz -= 1;
                sz >= 0
            } {
                event.fxp = hio_read8(f);
            }
            if n & 0x10 != 0 && {
                sz -= 1;
                sz >= 0
            } {
                event.f2t = hio_read8(f);
            }
            if n & 0x20 != 0 && {
                sz -= 1;
                sz >= 0
            } {
                event.f2p = hio_read8(f);
            }

            let note = event.note;
            dbm_translate_effect(note, &mut event.fxt, &mut event.fxp);
            dbm_translate_effect(note, &mut event.f2t, &mut event.f2p);
        }
    }

    0
}

/// Translation of `get_smpl()`.
fn get_smpl(m: &mut ModuleData, _size: u32, f: &mut HioHandle<'_>, data: &mut LocalData) -> i32 {
    /* Sanity check */
    if data.have_smpl || !data.have_info {
        return -1;
    }
    data.have_smpl = true;

    for i in 0..m.mod_.smp as usize {
        let xxs = &mut m.mod_.xxs[i];
        let flags = hio_read32b(f);
        xxs.len = hio_read32b(f) as i32;

        /* This seems to be roughly how DigiBooster 2.21 prioritizes
         * sample formats, with the caveat that it doesn't actually
         * support 32-bit samples. No flags set -> assumes 16-bit,
         * but don't support this (in case 3.0 is different...).
         */
        if flags & DBM_SAMPLE_8BIT != 0 {
            /* nop */
        } else if flags & DBM_SAMPLE_16BIT != 0 {
            xxs.flg |= XMP_SAMPLE_16BIT;
        } else if flags & DBM_SAMPLE_32BIT != 0 {
            /* Skip 32-bit samples */
            hio_seek(f, ((xxs.len as u32) << 2) as i64, SEEK_CUR);
            continue;
        } else {
            return -1;
        }

        let smpctl = m.smpctl;
        if libxmp_load_sample(
            smpctl,
            Some(f),
            SAMPLE_FLAG_BIGEND,
            &mut m.mod_.xxs[i],
            None,
        ) < 0
        {
            return -1;
        }
    }

    0
}

/// Translation of `read_envelope()`.
fn read_envelope(mod_: &XmpModule, env: &mut DbmEnvelope, f: &mut HioHandle<'_>) -> i32 {
    env.ins = hio_read16b(f) as i32 - 1;
    env.flg = (hio_read8(f) & 0x7) as i32;
    env.npt = hio_read8(f) as i32 + 1; /* DBM counts sections, not points. */
    env.sus = hio_read8(f) as i32;
    env.lps = hio_read8(f) as i32;
    env.lpe = hio_read8(f) as i32;
    env.sus2 = hio_read8(f) as i32;

    /* The format document claims there should be a reserved byte here but
     * no DigiBooster Pro module actually has this. The revised document
     * on the DigiBooster 3 website is corrected.
     */

    /* Sanity check */
    if env.ins < 0
        || env.ins >= mod_.ins
        || env.npt > 32
        || env.sus >= 32
        || env.lps >= 32
        || env.lpe >= 32
    {
        return -1;
    }

    for n in env.nodes.iter_mut() {
        n.position = hio_read16b(f);
        n.value = hio_read16b(f) as i16;
    }

    if hio_error(f) != 0 {
        return -1;
    }

    0
}

/// Translation of `get_venv()`.
fn get_venv(m: &mut ModuleData, _size: u32, f: &mut HioHandle<'_>, data: &mut LocalData) -> i32 {
    let mod_ = &mut m.mod_;
    let mut env = DbmEnvelope::default();

    /* Sanity check */
    if data.have_venv || !data.have_info {
        return -1;
    }
    data.have_venv = true;

    let nenv = hio_read16b(f) as i32;

    for _ in 0..nenv {
        if read_envelope(mod_, &mut env, f) != 0 {
            return -1;
        }

        let aei = &mut mod_.xxi[env.ins as usize].aei;
        aei.flg = env.flg;
        aei.npt = env.npt;
        aei.sus = env.sus;
        aei.lps = env.lps;
        aei.lpe = env.lpe;

        for j in 0..32 {
            aei.data[j * 2] = env.nodes[j].position as i16;
            aei.data[j * 2 + 1] = env.nodes[j].value;
        }
    }

    0
}

/// Translation of `get_penv()`.
fn get_penv(m: &mut ModuleData, _size: u32, f: &mut HioHandle<'_>, data: &mut LocalData) -> i32 {
    let mod_ = &mut m.mod_;
    let mut env = DbmEnvelope::default();

    /* Sanity check */
    if data.have_penv || !data.have_info {
        return -1;
    }
    data.have_penv = true;

    let nenv = hio_read16b(f) as i32;

    for _ in 0..nenv {
        if read_envelope(mod_, &mut env, f) != 0 {
            return -1;
        }

        let pei = &mut mod_.xxi[env.ins as usize].pei;
        pei.flg = env.flg;
        pei.npt = env.npt;
        pei.sus = env.sus;
        pei.lps = env.lps;
        pei.lpe = env.lpe;

        for j in 0..32 {
            /* DigiBooster Pro 2 stores the pan value between 0 and 64.
             * DigiBooster 3 stores it from -128 to 128 (Krashan - M2.dbm).
             */
            if data.maj_version >= 3 {
                env.nodes[j].value = env.nodes[j].value / 4 + 32;
            }

            pei.data[j * 2] = env.nodes[j].position as i16;
            pei.data[j * 2 + 1] = env.nodes[j].value;
        }
    }

    0
}

/// Translation of `dbm_load()`.
fn dbm_load(m: &mut ModuleData, f: &mut HioHandle<'_>, _start: i32) -> i32 {
    let mut name = [0u8; XMP_NAME_SIZE];
    let mut data = LocalData::default();

    hio_read32b(f); /* DBM0 */

    let version = hio_read16b(f);
    data.maj_version = (version >> 8) as i32;
    data.min_version = (version & 0xFF) as i32;

    hio_seek(f, 10, SEEK_CUR);
    if hio_read(&mut name, 1, 44, f) < 44 {
        return -1;
    }
    name[44] = b'\0';

    let mut handle = libxmp_iff_new::<LocalData>();

    m.c4rate = C4_NTSC_RATE;
    m.quirk |= QUIRK_FINEFX;

    /* IFF chunk IDs */
    let mut ret = libxmp_iff_register(&mut handle, b"INFO", get_info);
    ret |= libxmp_iff_register(&mut handle, b"SONG", get_song);
    ret |= libxmp_iff_register(&mut handle, b"INST", get_inst);
    ret |= libxmp_iff_register(&mut handle, b"PATT", get_patt);
    ret |= libxmp_iff_register(&mut handle, b"SMPL", get_smpl);
    ret |= libxmp_iff_register(&mut handle, b"VENV", get_venv);
    ret |= libxmp_iff_register(&mut handle, b"PENV", get_penv);

    if ret != 0 {
        return -1;
    }

    let mod_ = &mut m.mod_;
    strncpy(&mut mod_.name, &name, XMP_NAME_SIZE);
    let s = format!(
        "DigiBooster Pro {}.{:02x} DBM0",
        data.maj_version, data.min_version
    );
    snprintf_bytes(&mut mod_.type_, XMP_NAME_SIZE, s.as_bytes());

    /* Load IFF chunks */
    if libxmp_iff_load(&handle, m, f, &mut data) < 0 {
        libxmp_iff_release(handle);
        return -1;
    }

    libxmp_iff_release(handle);

    let mod_ = &mut m.mod_;
    for i in 0..mod_.chn as usize {
        mod_.xxc[i].pan = 0x80;
    }

    0
}
