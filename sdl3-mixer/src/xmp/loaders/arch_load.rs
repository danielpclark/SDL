// Rust translation of src/loaders/arch_load.c from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2026 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Archimedes Tracker (MUSX) loader.

use super::super::common::*;
use super::super::effects::*;
use super::super::hio::*;
use super::super::*;
use super::common::*;
use super::iff::*;
use super::sample::libxmp_load_sample;
use super::voltable::LIBXMP_ARCH_VOL_TABLE;
use super::*;

const MAGIC_MUSX: u32 = magic4(b'M', b'U', b'S', b'X');
const MAGIC_MNAM: u32 = magic4(b'M', b'N', b'A', b'M');
const MAGIC_SNAM: u32 = magic4(b'S', b'N', b'A', b'M');
const MAGIC_SVOL: u32 = magic4(b'S', b'V', b'O', b'L');
const MAGIC_SLEN: u32 = magic4(b'S', b'L', b'E', b'N');
const MAGIC_ROFS: u32 = magic4(b'R', b'O', b'F', b'S');
const MAGIC_RLEN: u32 = magic4(b'R', b'L', b'E', b'N');
const MAGIC_SDAT: u32 = magic4(b'S', b'D', b'A', b'T');

pub(crate) const LIBXMP_LOADER_ARCH: FormatLoader = FormatLoader {
    name: "Archimedes Tracker",
    test: arch_test,
    loader: arch_load,
};

/*
 * Linear (0 to 0x40) to logarithmic volume conversion.
 * This is only used for the Protracker-compatible "linear volume" effect in
 * Andy Southgate's StasisMod.  In this implementation linear and logarithmic
 * volumes can be freely intermixed.
 */
#[rustfmt::skip]
static LIN_TABLE: [u8; 65] = [
    0x00, 0x48, 0x64, 0x74, 0x82, 0x8a, 0x92, 0x9a,
    0xa2, 0xa6, 0xaa, 0xae, 0xb2, 0xb6, 0xea, 0xbe,
    0xc2, 0xc4, 0xc6, 0xc8, 0xca, 0xcc, 0xce, 0xd0,
    0xd2, 0xd4, 0xd6, 0xd8, 0xda, 0xdc, 0xde, 0xe0,
    0xe2, 0xe2, 0xe4, 0xe4, 0xe6, 0xe6, 0xe8, 0xe8,
    0xea, 0xea, 0xec, 0xec, 0xee, 0xee, 0xf0, 0xf0,
    0xf2, 0xf2, 0xf4, 0xf4, 0xf6, 0xf6, 0xf8, 0xf8,
    0xfa, 0xfa, 0xfc, 0xfc, 0xfe, 0xfe, 0xfe, 0xfe,
    0xfe,
];

/* (upstream keeps a logarithmic volume conversion here under #if 0) */

/// Translation of `arch_test()`.
fn arch_test(f: &mut HioHandle<'_>, t: Option<&mut [u8]>, _start: i32) -> i32 {
    if hio_read32b(f) != MAGIC_MUSX {
        return -1;
    }

    hio_read32l(f);

    let mut t = t;
    while hio_eof(f) == 0 {
        let id = hio_read32b(f);
        let len = hio_read32l(f);

        /* Sanity check */
        if len > 0x100000 {
            return -1;
        }

        if id == MAGIC_MNAM {
            libxmp_read_title(f, t.take(), 32);
            return 0;
        }

        hio_seek(f, len as i64, SEEK_CUR);
    }

    libxmp_read_title(f, t.take(), 0);

    0
}

/// Translation of `struct local_data`.
struct LocalData {
    year: i32,
    month: i32,
    day: i32,
    pflag: bool,
    sflag: bool,
    max_ins: i32,
    max_pat: i32,
    has_mvox: bool,
    has_pnum: bool,
    ster: [u8; 8],
    rows: [u8; 64],
}

impl Default for LocalData {
    fn default() -> Self {
        LocalData {
            year: 0,
            month: 0,
            day: 0,
            pflag: false,
            sflag: false,
            max_ins: 0,
            max_pat: 0,
            has_mvox: false,
            has_pnum: false,
            ster: [0; 8],
            rows: [0; 64],
        }
    }
}

/// Translation of `fix_effect()`.
fn fix_effect(e: &mut XmpEvent) {
    /* (upstream keeps a debugging printout here under #if 0) */
    match e.fxt {
        0x00 => {
            /* 00 xy Normal play or Arpeggio */
            e.fxt = FX_ARPEGGIO;
            /* x: first halfnote to add
            y: second halftone to subtract */
        }
        0x01 => {
            /* 01 xx Slide Up */
            e.fxt = FX_PORTA_UP;
        }
        0x02 => {
            /* 02 xx Slide Down */
            e.fxt = FX_PORTA_DN;
        }
        0x03 => {
            /* 03 xx Tone Portamento */
            e.fxt = FX_TONEPORTA;
        }
        0x0b => {
            /* 0B xx Break Pattern */
            e.fxt = FX_BREAK;
        }
        0x0c => {
            /* Set linear volume */
            if e.fxp <= 64 {
                e.fxt = FX_VOLSET;
                e.fxp = LIN_TABLE[e.fxp as usize];
            } else {
                e.fxp = 0;
                e.fxt = 0;
            }
        }
        0x0e /* 0E xy Set Stereo */ | 0x19 /* StasisMod's non-standard set panning effect */ => {
            /* y: stereo position (1-7,ignored). 1=left 4=center 7=right */
            if e.fxp > 0 && e.fxp < 8 {
                e.fxt = FX_SETPAN;
                e.fxp = 42 * e.fxp - 40;
            } else {
                e.fxt = 0;
                e.fxp = 0;
            }
        }
        0x10 => {
            /* 10 xx Volume Slide Up */
            e.fxt = FX_VOLSLIDE_UP;
        }
        0x11 => {
            /* 11 xx Volume Slide Down */
            e.fxt = FX_VOLSLIDE_DN;
        }
        0x13 => {
            /* 13 xx Position Jump */
            e.fxt = FX_JUMP;
        }
        0x15 => {
            /* 15 xy Line Jump. (not in manual) */
            /* Jump to line 10*x+y in same pattern. (10*x+y>63 ignored) */
            if msn(e.fxp) * 10 + lsn(e.fxp) < 64 {
                e.fxt = FX_LINE_JUMP;
                e.fxp = (msn(e.fxp) * 10 + lsn(e.fxp)) as u8;
            } else {
                e.fxt = 0;
                e.fxp = 0;
            }
        }
        0x1c => {
            /* 1C xy Set Speed */
            e.fxt = FX_SPEED;
        }
        0x1f => {
            /* 1F xx Set Volume */
            e.fxt = FX_VOLSET;
            /* all volumes are logarithmic */
            /* e->fxp = convert_vol (e->fxp); */
        }
        _ => {
            e.fxt = 0;
            e.fxp = 0;
        }
    }
}

/// Translation of `get_tinf()`.
fn get_tinf(_m: &mut ModuleData, _size: u32, f: &mut HioHandle<'_>, data: &mut LocalData) -> i32 {
    let mut x = hio_read8(f) as i32;
    data.year = ((x & 0xf0) >> 4) * 10 + (x & 0x0f);
    x = hio_read8(f) as i32;
    data.year += ((x & 0xf0) >> 4) * 1000 + (x & 0x0f) * 100;

    x = hio_read8(f) as i32;
    data.month = ((x & 0xf0) >> 4) * 10 + (x & 0x0f);

    x = hio_read8(f) as i32;
    data.day = ((x & 0xf0) >> 4) * 10 + (x & 0x0f);

    0
}

/// Translation of `get_mvox()`.
fn get_mvox(m: &mut ModuleData, _size: u32, f: &mut HioHandle<'_>, data: &mut LocalData) -> i32 {
    let chn = hio_read32l(f);

    /* Sanity check */
    if !(1..=8).contains(&chn) || data.has_mvox {
        return -1;
    }

    m.mod_.chn = chn as i32;
    data.has_mvox = true;
    0
}

/// Translation of `get_ster()`.
fn get_ster(m: &mut ModuleData, _size: u32, f: &mut HioHandle<'_>, data: &mut LocalData) -> i32 {
    let mod_ = &mut m.mod_;

    if hio_read(&mut data.ster, 1, 8, f) != 8 {
        return -1;
    }

    for i in 0..mod_.chn as usize {
        /* (the channels are at most 8, or 4 before MVOX) */
        let Some(&s) = data.ster.get(i) else {
            break;
        };
        if s > 0 && s < 8 {
            mod_.xxc[i].pan = 42 * s as i32 - 40;
        }
    }

    0
}

/// Translation of `get_mnam()`.
fn get_mnam(m: &mut ModuleData, _size: u32, f: &mut HioHandle<'_>, _data: &mut LocalData) -> i32 {
    if hio_read(&mut m.mod_.name, 1, 32, f) != 32 {
        return -1;
    }

    0
}

/// Translation of `get_anam()`.
fn get_anam(_m: &mut ModuleData, _size: u32, _f: &mut HioHandle<'_>, _data: &mut LocalData) -> i32 {
    /*hio_read(m->author, 1, 32, f); */

    0
}

/// Translation of `get_mlen()`.
fn get_mlen(m: &mut ModuleData, _size: u32, f: &mut HioHandle<'_>, _data: &mut LocalData) -> i32 {
    let len = hio_read32l(f);

    /* Sanity check */
    if len > 0xff {
        return -1;
    }

    m.mod_.len = len as i32;
    0
}

/// Translation of `get_pnum()`.
fn get_pnum(m: &mut ModuleData, _size: u32, f: &mut HioHandle<'_>, data: &mut LocalData) -> i32 {
    let pat = hio_read32l(f);

    /* Sanity check */
    if !(1..=64).contains(&pat) || data.has_pnum {
        return -1;
    }

    m.mod_.pat = pat as i32;
    data.has_pnum = true;
    0
}

/// Translation of `get_plen()`.
fn get_plen(_m: &mut ModuleData, _size: u32, f: &mut HioHandle<'_>, data: &mut LocalData) -> i32 {
    if hio_read(&mut data.rows, 1, 64, f) != 64 {
        return -1;
    }

    0
}

/// Translation of `get_sequ()`.
fn get_sequ(m: &mut ModuleData, _size: u32, f: &mut HioHandle<'_>, _data: &mut LocalData) -> i32 {
    hio_read(&mut m.mod_.xxo, 1, 128, f);
    libxmp_set_type(m, b"Archimedes Tracker");

    0
}

/// Translation of `get_patt()`.
fn get_patt(m: &mut ModuleData, _size: u32, f: &mut HioHandle<'_>, data: &mut LocalData) -> i32 {
    let mod_ = &mut m.mod_;

    /* Sanity check */
    if !data.has_mvox || !data.has_pnum {
        return -1;
    }

    if !data.pflag {
        data.pflag = true;
        data.max_pat = 0;
        mod_.trk = mod_.pat * mod_.chn;

        if libxmp_init_pattern(mod_) < 0 {
            return -1;
        }
    }

    /* Sanity check */
    if data.max_pat >= mod_.pat || data.max_pat >= 64 {
        return -1;
    }

    let i = data.max_pat;

    if libxmp_alloc_pattern_tracks(mod_, i, data.rows[i as usize] as i32) < 0 {
        return -1;
    }

    for j in 0..data.rows[i as usize] as i32 {
        for k in 0..mod_.chn {
            let event = mod_.event_mut(i, k, j);

            event.fxp = hio_read8(f);
            event.fxt = hio_read8(f);
            event.ins = hio_read8(f);
            event.note = hio_read8(f);

            if event.note != 0 {
                event.note = event.note.wrapping_add(48);
            }

            fix_effect(event);
        }
    }

    data.max_pat += 1;

    0
}

/// Translation of `get_samp()`.
fn get_samp(m: &mut ModuleData, _size: u32, f: &mut HioHandle<'_>, data: &mut LocalData) -> i32 {
    if !data.sflag {
        m.mod_.ins = 36;
        m.mod_.smp = 36;
        if libxmp_init_instrument(m) < 0 {
            return -1;
        }

        data.sflag = true;
        data.max_ins = 0;
    }

    /* FIXME: More than 36 sample slots used.  Unfortunately we
     * have no way to handle this without two passes, and it's
     * officially supposed to be 36, so ignore the rest.
     */
    if data.max_ins >= 36 {
        return 0;
    }

    let i = data.max_ins;
    let iu = i as usize;
    let mod_ = &mut m.mod_;

    mod_.xxi[iu].nsm = 1;
    if libxmp_alloc_subinstrument(mod_, i, 1) < 0 {
        return -1;
    }

    if hio_read32b(f) != MAGIC_SNAM {
        /* SNAM */
        return -1;
    }

    {
        /* should usually be 0x14 but zero is not unknown */
        let name_len = hio_read32l(f) as i32;

        /* Sanity check */
        if !(0..=32).contains(&name_len) {
            return -1;
        }

        // Note (upstream): a name of 32 bytes fills the name without its
        // terminator, which C's string functions then read past; here it
        // is read within the name's array.
        hio_read(&mut mod_.xxi[iu].name, 1, name_len as usize, f);
    }

    if hio_read32b(f) != MAGIC_SVOL {
        /* SVOL */
        return -1;
    }
    hio_read32l(f);
    /* mod->xxi[i].sub[0].vol = convert_vol(hio_read32l(f)); */
    mod_.xxi[iu].sub[0].vol = (hio_read32l(f) & 0xff) as i32;

    if hio_read32b(f) != MAGIC_SLEN {
        /* SLEN */
        return -1;
    }
    hio_read32l(f);
    mod_.xxs[iu].len = hio_read32l(f) as i32;

    if hio_read32b(f) != MAGIC_ROFS {
        /* ROFS */
        return -1;
    }
    hio_read32l(f);
    mod_.xxs[iu].lps = hio_read32l(f) as i32;

    if hio_read32b(f) != MAGIC_RLEN {
        /* RLEN */
        return -1;
    }
    hio_read32l(f);
    mod_.xxs[iu].lpe = hio_read32l(f) as i32;

    if hio_read32b(f) != MAGIC_SDAT {
        /* SDAT */
        /* This is corrupt in the final sample of 5-OverPar.
         * TODO: a more robust IFF handler for these fields would
         * be better. */
        mod_.xxs[iu].len = 0;
    }
    hio_read32l(f);
    hio_read32l(f); /* 0x00000000 */

    mod_.xxi[iu].sub[0].sid = i;
    mod_.xxi[iu].sub[0].pan = XMP_INST_NO_DEFAULT_PAN;

    m.vol_table = Some(&LIBXMP_ARCH_VOL_TABLE);
    m.volbase = 0xff;

    let xxs = &mut m.mod_.xxs[iu];
    /* Clean bad loops */
    if xxs.lps < 0 || xxs.lps >= xxs.len {
        xxs.lps = 0;
        xxs.lpe = 0;
    }

    if xxs.lpe > 2 {
        if xxs.lpe > xxs.len - xxs.lps {
            xxs.lpe = xxs.len - xxs.lps;
        }
        xxs.flg = XMP_SAMPLE_LOOP;
        xxs.lpe += xxs.lps;
    } else if xxs.lpe == 2 && xxs.lps > 0 {
        /* non-zero repeat offset and repeat length of 2
         * means loop to end of sample */
        xxs.flg = XMP_SAMPLE_LOOP;
        xxs.lpe = xxs.len;
    }

    let smpctl = m.smpctl;
    if libxmp_load_sample(smpctl, Some(f), SAMPLE_FLAG_VIDC, &mut m.mod_.xxs[iu], None) < 0 {
        return -1;
    }

    data.max_ins += 1;

    0
}

/// Translation of `arch_load()`.
fn arch_load(m: &mut ModuleData, f: &mut HioHandle<'_>, _start: i32) -> i32 {
    let mut data = LocalData::default();

    hio_read32b(f); /* MUSX */
    hio_read32b(f);

    let mut handle = libxmp_iff_new::<LocalData>();

    /* IFF chunk IDs */
    libxmp_iff_register(&mut handle, b"TINF", get_tinf);
    libxmp_iff_register(&mut handle, b"MVOX", get_mvox);
    libxmp_iff_register(&mut handle, b"STER", get_ster);
    libxmp_iff_register(&mut handle, b"MNAM", get_mnam);
    libxmp_iff_register(&mut handle, b"ANAM", get_anam);
    libxmp_iff_register(&mut handle, b"MLEN", get_mlen);
    libxmp_iff_register(&mut handle, b"PNUM", get_pnum);
    libxmp_iff_register(&mut handle, b"PLEN", get_plen);
    libxmp_iff_register(&mut handle, b"SEQU", get_sequ);
    libxmp_iff_register(&mut handle, b"PATT", get_patt);
    libxmp_iff_register(&mut handle, b"SAMP", get_samp);

    libxmp_iff_set_quirk(&mut handle, IFF_LITTLE_ENDIAN);

    /* Load IFF chunks */
    if libxmp_iff_load(&handle, m, f, &mut data) < 0 {
        libxmp_iff_release(handle);
        return -1;
    }

    libxmp_iff_release(handle);

    for i in 0..m.mod_.chn {
        let pan = defpan(m, (((i + 3) / 2) % 2) * 0xff);
        m.mod_.xxc[i as usize].pan = pan;
    }

    0
}
