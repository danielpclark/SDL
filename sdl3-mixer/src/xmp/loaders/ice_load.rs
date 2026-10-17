// Rust translation of src/loaders/ice_load.c from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2026 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

/* Loader for Soundtracker 2.6/Ice Tracker modules */

//! The Soundtracker 2.6 / Ice Tracker loader.

use super::super::common::*;
use super::super::effects::*;
use super::super::hio::*;
use super::super::*;
use super::common::*;
use super::sample::libxmp_load_sample;
use super::*;

const MAGIC_MTN_: u32 = magic4(b'M', b'T', b'N', 0);
const MAGIC_IT10: u32 = magic4(b'I', b'T', b'1', b'0');

pub(crate) const LIBXMP_LOADER_ICE: FormatLoader = FormatLoader {
    name: "Soundtracker 2.6/Ice Tracker",
    test: ice_test,
    loader: ice_load,
};

/// Translation of `ice_test()`.
fn ice_test(f: &mut HioHandle<'_>, t: Option<&mut [u8]>, start: i32) -> i32 {
    hio_seek(f, start as i64 + 1464, SEEK_SET);
    let magic = hio_read32b(f);
    if magic != MAGIC_MTN_ && magic != MAGIC_IT10 {
        return -1;
    }

    hio_seek(f, start as i64, SEEK_SET);
    libxmp_read_title(f, t, 28);

    0
}

/// Translation of `struct ice_ins`.
#[derive(Clone, Copy, Default)]
struct IceIns {
    name: [u8; 22],  /* Instrument name */
    len: u16,        /* Sample length / 2 */
    finetune: u8,    /* Finetune */
    volume: u8,      /* Volume (0-63) */
    loop_start: u16, /* Sample loop start in file */
    loop_size: u16,  /* Loop size / 2 */
}

/// Translation of `struct ice_header`.
#[derive(Clone, Copy)]
struct IceHeader {
    title: [u8; 20],
    ins: [IceIns; 31], /* Instruments */
    len: u8,           /* Size of the pattern list */
    trk: u8,           /* Number of tracks */
    ord: [[u8; 4]; 128],
    magic: u32, /* 'MTN\0', 'IT10' */
}

/// Translation of `ice_load()`.
fn ice_load(m: &mut ModuleData, f: &mut HioHandle<'_>, _start: i32) -> i32 {
    let mut ev = [0u8; 4];
    let mut ih = IceHeader {
        title: [0; 20],
        ins: [IceIns::default(); 31],
        len: 0,
        trk: 0,
        ord: [[0; 4]; 128],
        magic: 0,
    };

    hio_read(&mut ih.title, 20, 1, f);
    for i in 0..31 {
        hio_read(&mut ih.ins[i].name, 22, 1, f);
        ih.ins[i].len = hio_read16b(f);
        ih.ins[i].finetune = hio_read8(f);
        ih.ins[i].volume = hio_read8(f);
        ih.ins[i].loop_start = hio_read16b(f);
        ih.ins[i].loop_size = hio_read16b(f);
    }
    ih.len = hio_read8(f);
    ih.trk = hio_read8(f);
    let mut ord = [0u8; 128 * 4];
    hio_read(&mut ord, 128 * 4, 1, f);
    for (o, c) in ih.ord.iter_mut().zip(ord.chunks_exact(4)) {
        o.copy_from_slice(c);
    }
    ih.magic = hio_read32b(f);

    /* Sanity check */
    if ih.len > 128 {
        return -1;
    }
    for i in 0..ih.len as usize {
        for j in 0..4 {
            if ih.ord[i][j] >= ih.trk {
                return -1;
            }
        }
    }

    if ih.magic == MAGIC_IT10 {
        libxmp_set_type(m, b"Ice Tracker");
    } else if ih.magic == MAGIC_MTN_ {
        libxmp_set_type(m, b"Soundtracker 2.6");
    } else {
        return -1;
    }

    let mod_ = &mut m.mod_;
    mod_.ins = 31;
    mod_.smp = mod_.ins;
    mod_.pat = ih.len as i32;
    mod_.len = ih.len as i32;
    mod_.trk = ih.trk as i32;

    strncpy(&mut mod_.name, &ih.title, 20);

    if libxmp_init_instrument(m) < 0 {
        return -1;
    }

    let mod_ = &mut m.mod_;
    for i in 0..mod_.ins {
        let iu = i as usize;
        if libxmp_alloc_subinstrument(mod_, i, 1) < 0 {
            return -1;
        }

        let xxs = &mut mod_.xxs[iu];
        xxs.len = 2 * ih.ins[iu].len as i32;
        xxs.lps = 2 * ih.ins[iu].loop_start as i32;
        xxs.lpe = xxs.lps + 2 * ih.ins[iu].loop_size as i32;
        xxs.flg = if ih.ins[iu].loop_size > 1 {
            XMP_SAMPLE_LOOP
        } else {
            0
        };
        let len = xxs.len;
        let xxi = &mut mod_.xxi[iu];
        xxi.sub[0].vol = ih.ins[iu].volume as i32;
        /* xxi->sub[0].fin = (int8)(ih.ins[i].finetune << 4); */
        xxi.sub[0].pan = XMP_INST_NO_DEFAULT_PAN;
        xxi.sub[0].sid = i;

        if len > 0 {
            xxi.nsm = 1;
        }
    }

    if libxmp_init_pattern(mod_) < 0 {
        return -1;
    }

    for i in 0..mod_.pat {
        if libxmp_alloc_pattern(mod_, i) < 0 {
            return -1;
        }
        mod_.pattern_mut(i).rows = 64;

        for j in 0..mod_.chn as usize {
            mod_.pattern_mut(i).index[j] = ih.ord[i as usize][j] as i32;
        }
        mod_.xxo[i as usize] = i as u8;
    }

    for i in 0..mod_.trk {
        if libxmp_alloc_track(mod_, i, 64) < 0 {
            return -1;
        }

        for j in 0..mod_.track(i).rows as usize {
            let event = &mut mod_.track_mut(i).event[j];
            if hio_read(&mut ev, 1, 4, f) < 4 {
                return -1;
            }
            libxmp_decode_protracker_event(event, &ev);

            if event.fxt == FX_SPEED && msn(event.fxp) != 0 && lsn(event.fxp) != 0 {
                event.fxt = FX_ICE_SPEED;
            }
        }
    }

    m.period_type = PERIOD_MODRNG;

    /* Read samples */

    for i in 0..m.mod_.ins as usize {
        if m.mod_.xxs[i].len <= 4 {
            continue;
        }
        let smpctl = m.smpctl;
        if libxmp_load_sample(smpctl, Some(f), 0, &mut m.mod_.xxs[i], None) < 0 {
            return -1;
        }
    }

    0
}
