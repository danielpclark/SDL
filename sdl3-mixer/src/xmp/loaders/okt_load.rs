// Rust translation of src/loaders/okt_load.c from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2026 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

/* Based on the format description written by Harald Zappe.
 * Additional information about Oktalyzer modules from Bernardo
 * Innocenti's XModule 3.4 sources.
 */

//! The Oktalyzer loader.

use super::super::common::*;
use super::super::effects::*;
use super::super::hio::*;
use super::super::*;
use super::common::*;
use super::iff::*;
use super::sample::libxmp_load_sample;
use super::*;

pub(crate) const LIBXMP_LOADER_OKT: FormatLoader = FormatLoader {
    name: "Oktalyzer",
    test: okt_test,
    loader: okt_load,
};

/// Translation of `okt_test()`.
fn okt_test(f: &mut HioHandle<'_>, t: Option<&mut [u8]>, _start: i32) -> i32 {
    let mut magic = [0u8; 8];

    if hio_read(&mut magic, 1, 8, f) < 8 {
        return -1;
    }

    if &magic != b"OKTASONG" {
        return -1;
    }

    libxmp_read_title(f, t, 0);

    0
}

const OKT_MODE8: i32 = 0x00; /* 7 bit samples */
#[allow(dead_code)]
const OKT_MODE4: i32 = 0x01; /* 8 bit samples */
const OKT_MODEB: i32 = 0x02; /* Both */

const NONE: u8 = 0xff;

/// Translation of `struct local_data`.
struct LocalData {
    mode: [i32; 36],
    idx: [i32; 36],
    pattern: i32,
    sample: i32,
    samples: i32,
    has_cmod: bool,
    has_samp: bool,
    has_slen: bool,
}

static FX: [u8; 32] = [
    NONE,
    FX_PORTA_UP, /*  1 */
    FX_PORTA_DN, /*  2 */
    NONE,
    NONE,
    NONE,
    NONE,
    NONE,
    NONE,
    NONE,
    FX_OKT_ARP3,   /* 10 */
    FX_OKT_ARP4,   /* 11 */
    FX_OKT_ARP5,   /* 12 */
    FX_NSLIDE2_DN, /* 13 */
    NONE,
    NONE, /* 15 - filter */
    NONE,
    FX_NSLIDE2_UP, /* 17 */
    NONE,
    NONE,
    NONE,
    FX_NSLIDE_DN, /* 21 */
    NONE,
    NONE,
    NONE,
    FX_JUMP, /* 25 */
    NONE,
    NONE,     /* 27 - release */
    FX_SPEED, /* 28 */
    NONE,
    FX_NSLIDE_UP, /* 30 */
    FX_VOLSET,    /* 31 */
];

/// Translation of `okt_translate_effect()`.
fn okt_translate_effect(event: &mut XmpEvent, fxt: i32, fxp: i32) -> i32 {
    if fxt as usize >= FX.len() {
        return -1;
    }
    event.fxt = FX[fxt as usize];
    event.fxp = fxp as u8;

    if event.fxt == FX_VOLSET && event.fxp > 0x40 {
        if event.fxp <= 0x50 {
            event.fxt = FX_VOLSLIDE;
            event.fxp -= 0x40;
        } else if event.fxp <= 0x60 {
            event.fxt = FX_VOLSLIDE;
            event.fxp = (event.fxp - 0x50) << 4;
        } else if event.fxp <= 0x70 {
            event.fxt = FX_F_VSLIDE_DN;
            event.fxp -= 0x60;
        } else if event.fxp <= 0x80 {
            event.fxt = FX_F_VSLIDE_UP;
            event.fxp -= 0x70;
        }
    }
    if event.fxt == FX_ARPEGGIO {
        /* Arpeggio fixup */
        event.fxp = ((((24 - msn(event.fxp)) % 12) << 4) | lsn(event.fxp)) as u8;
    }
    if event.fxt == NONE {
        event.fxt = 0;
        event.fxp = 0;
    }

    0
}

/// Translation of `get_cmod()`.
fn get_cmod(m: &mut ModuleData, size: u32, f: &mut HioHandle<'_>, data: &mut LocalData) -> i32 {
    /* Sanity check */
    if data.has_cmod || size < 8 {
        return -1;
    }
    data.has_cmod = true;

    let defpan = m.defpan;
    let mod_ = &mut m.mod_;
    mod_.chn = 0;
    for i in 0..4 {
        let pan = (((i + 1) / 2) % 2) * 0xff;
        let p = 0x80 + (pan - 0x80) * defpan / 100;

        if hio_read16b(f) == 0 {
            mod_.xxc[mod_.chn as usize].pan = p;
            mod_.chn += 1;
        } else {
            mod_.xxc[mod_.chn as usize].flg |= XMP_CHANNEL_SPLIT | (i << 4);
            mod_.xxc[mod_.chn as usize].pan = p;
            mod_.chn += 1;
            mod_.xxc[mod_.chn as usize].flg |= XMP_CHANNEL_SPLIT | (i << 4);
            mod_.xxc[mod_.chn as usize].pan = p;
            mod_.chn += 1;
        }
    }

    0
}

/// Translation of `get_samp()`.
fn get_samp(m: &mut ModuleData, size: u32, f: &mut HioHandle<'_>, data: &mut LocalData) -> i32 {
    /* Sanity check */
    if data.has_samp || size != 36 * 32 {
        return -1;
    }
    data.has_samp = true;

    /* Should be always 36 */
    m.mod_.ins = (size / 32) as i32; /* sizeof(struct okt_instrument_header); */
    m.mod_.smp = m.mod_.ins;

    if libxmp_init_instrument(m) < 0 {
        return -1;
    }

    let mod_ = &mut m.mod_;
    let mut j = 0;
    for i in 0..mod_.ins {
        let iu = i as usize;
        let ju = j as usize;

        if libxmp_alloc_subinstrument(mod_, i, 1) < 0 {
            return -1;
        }

        let xxi = &mut mod_.xxi[iu];
        let xxs = &mut mod_.xxs[ju];
        let sub = &mut xxi.sub[0];

        hio_read(&mut xxi.name, 1, 20, f);

        /* Sample size is always rounded down */
        xxs.len = (hio_read32b(f) & !1) as i32;
        xxs.lps = (hio_read16b(f) as i32) << 1;
        let looplen = (hio_read16b(f) as i32) << 1;
        xxs.lpe = xxs.lps + looplen;
        xxs.flg = if looplen > 2 { XMP_SAMPLE_LOOP } else { 0 };

        sub.vol = hio_read16b(f) as i32;
        data.mode[iu] = hio_read16b(f) as i32;

        sub.pan = XMP_INST_NO_DEFAULT_PAN;
        sub.sid = j;

        data.idx[ju] = i;

        if xxs.len > 0 {
            xxi.nsm = 1;
            j += 1;
        }
    }
    data.samples = j;

    0
}

/// Translation of `get_spee()`.
fn get_spee(m: &mut ModuleData, _size: u32, f: &mut HioHandle<'_>, _data: &mut LocalData) -> i32 {
    let mod_ = &mut m.mod_;

    mod_.spd = hio_read16b(f) as i32;
    mod_.bpm = 125;

    0
}

/// Translation of `get_slen()`.
fn get_slen(m: &mut ModuleData, size: u32, f: &mut HioHandle<'_>, data: &mut LocalData) -> i32 {
    let mod_ = &mut m.mod_;

    /* Sanity check */
    if data.has_slen || !data.has_cmod || size < 2 {
        return -1;
    }
    data.has_slen = true;

    mod_.pat = hio_read16b(f) as i32;
    mod_.trk = mod_.pat * mod_.chn;

    0
}

/// Translation of `get_plen()`.
fn get_plen(m: &mut ModuleData, _size: u32, f: &mut HioHandle<'_>, _data: &mut LocalData) -> i32 {
    let mod_ = &mut m.mod_;

    mod_.len = hio_read16b(f) as i32;

    /* Sanity check */
    if mod_.len > 256 {
        return -1;
    }

    0
}

/// Translation of `get_patt()`.
fn get_patt(m: &mut ModuleData, _size: u32, f: &mut HioHandle<'_>, _data: &mut LocalData) -> i32 {
    let mod_ = &mut m.mod_;

    let len = mod_.len as usize;
    if hio_read(&mut mod_.xxo, 1, len, f) != len {
        return -1;
    }

    0
}

/// Translation of `get_pbod()`.
fn get_pbod(m: &mut ModuleData, _size: u32, f: &mut HioHandle<'_>, data: &mut LocalData) -> i32 {
    let mod_ = &mut m.mod_;

    /* Sanity check */
    if !data.has_slen || !data.has_cmod {
        return -1;
    }

    if data.pattern >= mod_.pat {
        return 0;
    }

    if data.pattern == 0 && libxmp_init_pattern(mod_) < 0 {
        return -1;
    }

    let rows = hio_read16b(f);

    if libxmp_alloc_pattern_tracks(mod_, data.pattern, rows as i32) < 0 {
        return -1;
    }

    for j in 0..rows as i32 {
        for k in 0..mod_.chn {
            let mut c = [0u8; 4];

            let e = mod_.event_mut(data.pattern, k, j);
            *e = XmpEvent::default();

            if hio_read(&mut c, 1, 4, f) < 4 {
                return -1;
            }

            let note = c[0];
            let ins = c[1];

            if note != 0 {
                e.note = 48u8.wrapping_add(note);
                e.ins = 1u8.wrapping_add(ins);
            }

            if okt_translate_effect(e, c[2] as i32, c[3] as i32) < 0 {
                return -1;
            }
        }
    }
    data.pattern += 1;

    0
}

/// Translation of `get_sbod()`.
fn get_sbod(m: &mut ModuleData, _size: u32, f: &mut HioHandle<'_>, data: &mut LocalData) -> i32 {
    let mut flags = 0;

    if data.sample >= data.samples {
        return 0;
    }

    let i = data.idx[data.sample as usize] as usize;
    if data.mode[i] == OKT_MODE8 || data.mode[i] == OKT_MODEB {
        flags = SAMPLE_FLAG_7BIT;
    }

    let sid = m.mod_.xxi[i].sub[0].sid as usize;
    let smpctl = m.smpctl;
    if libxmp_load_sample(smpctl, Some(f), flags, &mut m.mod_.xxs[sid], None) < 0 {
        return -1;
    }

    data.sample += 1;

    0
}

/// Translation of `okt_load()`.
fn okt_load(m: &mut ModuleData, f: &mut HioHandle<'_>, _start: i32) -> i32 {
    hio_seek(f, 8, SEEK_CUR); /* OKTASONG */

    let mut handle = libxmp_iff_new::<LocalData>();

    let mut data = LocalData {
        mode: [0; 36],
        idx: [0; 36],
        pattern: 0,
        sample: 0,
        samples: 0,
        has_cmod: false,
        has_samp: false,
        has_slen: false,
    };

    /* IFF chunk IDs */
    let mut ret = libxmp_iff_register(&mut handle, b"CMOD", get_cmod);
    ret |= libxmp_iff_register(&mut handle, b"SAMP", get_samp);
    ret |= libxmp_iff_register(&mut handle, b"SPEE", get_spee);
    ret |= libxmp_iff_register(&mut handle, b"SLEN", get_slen);
    ret |= libxmp_iff_register(&mut handle, b"PLEN", get_plen);
    ret |= libxmp_iff_register(&mut handle, b"PATT", get_patt);
    ret |= libxmp_iff_register(&mut handle, b"PBOD", get_pbod);
    ret |= libxmp_iff_register(&mut handle, b"SBOD", get_sbod);

    if ret != 0 {
        return -1;
    }

    libxmp_set_type(m, b"Oktalyzer");

    /* Load IFF chunks */
    if libxmp_iff_load(&handle, m, f, &mut data) < 0 {
        libxmp_iff_release(handle);
        return -1;
    }

    libxmp_iff_release(handle);

    m.period_type = PERIOD_MODRNG;

    0
}
