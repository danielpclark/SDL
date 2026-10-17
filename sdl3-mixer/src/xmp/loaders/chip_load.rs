// Rust translation of src/loaders/chip_load.c from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2026 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Chiptracker (KRIS) loader.

use super::super::common::*;
use super::super::hio::*;
use super::super::*;
use super::common::*;
use super::mod_load::{ModHeader, ModInstrument};
use super::sample::libxmp_load_sample;
use super::*;

pub(crate) const LIBXMP_LOADER_CHIP: FormatLoader = FormatLoader {
    name: "Chiptracker",
    test: chip_test,
    loader: chip_load,
};

/// Translation of `chip_test()`.
fn chip_test(f: &mut HioHandle<'_>, t: Option<&mut [u8]>, start: i32) -> i32 {
    let mut buf = [0u8; 4];

    hio_seek(f, start as i64 + 952, SEEK_SET);
    if hio_read(&mut buf, 1, 4, f) < 4 {
        return -1;
    }

    /* Also RASP? */
    if &buf != b"KRIS" {
        return -1;
    }

    hio_seek(f, start as i64, SEEK_SET);
    libxmp_read_title(f, t, 20);

    0
}

/// Translation of `chip_load()`.
fn chip_load(m: &mut ModuleData, f: &mut HioHandle<'_>, _start: i32) -> i32 {
    let mut mh = ModHeader {
        name: [0; 20],
        ins: [ModInstrument::default(); 31],
        len: 0,
        restart: 0,
        order: [0; 128],
        magic: [0; 4],
    };

    let Ok(mut tidx) = try_vec(1024, 0u8) else {
        return -1;
    };

    hio_read(&mut mh.name, 20, 1, f);
    hio_read16b(f);

    for i in 0..31 {
        let ins = &mut mh.ins[i];
        hio_read(&mut ins.name, 22, 1, f);
        ins.size = hio_read16b(f);
        ins.finetune = hio_read8(f) as i8;
        ins.volume = hio_read8(f) as i8;
        ins.loop_start = hio_read16b(f);
        ins.loop_size = hio_read16b(f);
    }

    hio_read(&mut mh.magic, 4, 1, f);
    mh.len = hio_read8(f);

    /* Sanity check */
    if mh.len > 128 {
        return -1;
    }

    mh.restart = hio_read8(f);
    hio_read(&mut tidx, 1024, 1, f);
    hio_read16b(f);

    let mod_ = &mut m.mod_;
    mod_.chn = 4;
    mod_.ins = 31;
    mod_.smp = mod_.ins;
    mod_.len = mh.len as i32;
    mod_.pat = mh.len as i32;
    mod_.rst = mh.restart as i32;

    let mut tnum = 0;
    for i in 0..mod_.len as usize {
        mod_.xxo[i] = i as u8;

        for j in 0..4 {
            let t = tidx[2 * (4 * i + j)] as i32;
            if t > tnum {
                tnum = t;
            }
        }
    }

    mod_.trk = tnum + 1;

    strncpy(&mut mod_.name, &mh.name, 20);
    libxmp_set_type(m, b"Chiptracker");

    if libxmp_init_instrument(m) < 0 {
        return -1;
    }

    let mod_ = &mut m.mod_;
    for i in 0..mod_.ins {
        let iu = i as usize;

        if libxmp_alloc_subinstrument(mod_, i, 1) < 0 {
            return -1;
        }

        let ins = &mh.ins[iu];
        let xxs = &mut mod_.xxs[iu];
        xxs.len = 2 * ins.size as i32;
        xxs.lps = ins.loop_start as i32;
        xxs.lpe = xxs.lps + 2 * ins.loop_size as i32;
        xxs.flg = if ins.loop_size > 1 {
            XMP_SAMPLE_LOOP
        } else {
            0
        };
        let len = xxs.len;
        let xxi = &mut mod_.xxi[iu];
        let sub = &mut xxi.sub[0];
        sub.fin = ((ins.finetune as u8) << 4) as i8 as i32;
        sub.vol = ins.volume as i32;
        sub.pan = XMP_INST_NO_DEFAULT_PAN;
        sub.sid = i;

        if len > 0 {
            xxi.nsm = 1;
        }

        libxmp_instrument_name(mod_, i, &ins.name, 22);
    }

    if libxmp_init_pattern(mod_) < 0 {
        return -1;
    }

    for i in 0..mod_.len {
        if libxmp_alloc_pattern(mod_, i) < 0 {
            return -1;
        }
        mod_.pattern_mut(i).rows = 64;

        for j in 0..4 {
            let t = tidx[2 * (4 * i as usize + j)] as i32;
            mod_.pattern_mut(i).index[j] = t;
        }
    }

    /* Load and convert tracks */

    for i in 0..mod_.trk {
        if libxmp_alloc_track(mod_, i, 64) < 0 {
            return -1;
        }

        for j in 0..64 {
            let event = &mut mod_.track_mut(i).event[j];
            let mut e = [0u8; 4];

            if hio_read(&mut e, 1, 4, f) < 4 {
                return -1;
            }
            if e[0] != 0 && e[0] != 0xa8 {
                event.note = 13 + e[0] / 2;
            }
            event.ins = e[1];
            event.fxt = e[2] & 0x0f;
            event.fxp = e[3];
        }
    }

    m.period_type = PERIOD_MODRNG;

    /* Load samples */

    for i in 0..m.mod_.smp as usize {
        if m.mod_.xxs[i].len == 0 {
            continue;
        }

        let smpctl = m.smpctl;
        if libxmp_load_sample(
            smpctl,
            Some(f),
            SAMPLE_FLAG_FULLREP,
            &mut m.mod_.xxs[i],
            None,
        ) < 0
        {
            return -1;
        }
    }

    0
}
