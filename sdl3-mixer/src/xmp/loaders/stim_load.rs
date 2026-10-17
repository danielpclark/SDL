// Rust translation of src/loaders/stim_load.c from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2026 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

/* Loader for Slamtilt modules based on the format description
 * written by Sylvain Chipaux (Asle/ReDoX). Get the Slamtilt demo
 * from game/demo in Aminet.
 */

/* Tested with the Slamtilt modules sent by Sipos Attila */

//! The Slamtilt loader.

use super::super::common::*;
use super::super::hio::*;
use super::super::*;
use super::common::*;
use super::sample::libxmp_load_sample;
use super::*;

const MAGIC_STIM: u32 = magic4(b'S', b'T', b'I', b'M');

pub(crate) const LIBXMP_LOADER_STIM: FormatLoader = FormatLoader {
    name: "Slamtilt",
    test: stim_test,
    loader: stim_load,
};

/// Translation of `stim_test()`.
fn stim_test(f: &mut HioHandle<'_>, t: Option<&mut [u8]>, _start: i32) -> i32 {
    if hio_read32b(f) != MAGIC_STIM {
        return -1;
    }

    if hio_read16b(f) > 16 {
        return -1;
    }

    libxmp_read_title(f, t, 0);

    0
}

/// Translation of `struct stim_instrument`.
#[derive(Default)]
struct StimInstrument {
    size: u16,       /* Length of the sample (/2) */
    finetune: u8,    /* Finetune (as ptk) */
    volume: u8,      /* Volume (as ptk) */
    loop_start: u16, /* Loop start (/2) */
    loop_size: u16,  /* Loop length (/2) */
}

/// Translation of `struct stim_header`.
struct StimHeader {
    id: u32,            /* "STIM" ID string */
    smpaddr: u32,       /* Address of the sample descriptions */
    nos: u16,           /* Number of samples (?) */
    len: u16,           /* Size of pattern list */
    pat: u16,           /* Number of patterns saved */
    order: [u8; 128],   /* Pattern list */
    pataddr: [u32; 64], /* Pattern addresses (add 0xc) */
}

/// Translation of `stim_load()`.
fn stim_load(m: &mut ModuleData, f: &mut HioHandle<'_>, start: i32) -> i32 {
    let mut sh = StimHeader {
        id: 0,
        smpaddr: 0,
        nos: 0,
        len: 0,
        pat: 0,
        order: [0; 128],
        pataddr: [0; 64],
    };
    let mut si = StimInstrument::default();

    sh.id = hio_read32b(f);
    sh.smpaddr = hio_read32b(f);
    hio_read32b(f);
    hio_read32b(f);
    sh.nos = hio_read16b(f);
    sh.len = hio_read16b(f);
    sh.pat = hio_read16b(f);
    if hio_read(&mut sh.order, 128, 1, f) == 0 {
        return -1;
    }
    let _ = sh.id;

    /* Sanity check */
    if sh.nos > 31 || sh.len > 128 || sh.pat > 64 {
        return -1;
    }

    for i in 0..64 {
        sh.pataddr[i] = hio_read32b(f).wrapping_add(0x0c);
        if sh.pataddr[i] > 0x00100000 {
            return -1;
        }
    }

    let mod_ = &mut m.mod_;
    mod_.chn = 4;
    mod_.len = sh.len as i32;
    mod_.pat = sh.pat as i32;
    mod_.ins = sh.nos as i32;
    mod_.smp = mod_.ins;
    mod_.trk = mod_.pat * mod_.chn;

    for i in 0..mod_.len as usize {
        mod_.xxo[i] = sh.order[i];
    }

    libxmp_set_type(m, b"Slamtilt");

    let mod_ = &mut m.mod_;
    if libxmp_init_pattern(mod_) < 0 {
        return -1;
    }

    /* Load and convert patterns */

    for i in 0..mod_.pat {
        if libxmp_alloc_pattern_tracks(mod_, i, 64) < 0 {
            return -1;
        }

        hio_seek(
            f,
            (start as u32)
                .wrapping_add(sh.pataddr[i as usize])
                .wrapping_add(8) as i64,
            SEEK_SET,
        );

        for j in 0..4 {
            let mut k = 0;
            while k < 64 {
                let event = mod_.event_mut(i, j, k);
                let b1 = hio_read8(f);

                if b1 & 0x80 != 0 {
                    k += (b1 & 0x7f) as i32;
                    k += 1;
                    continue;
                }

                /* STIM event format:
                 *
                 *     __ Fx __
                 *    /        \
                 *   ||        ||
                 *  0000 0000  0000 0000  0000 0000
                 *  |  |    |    |     |  |       |
                 *  |   \  /      \   /    \     /
                 *  |    smp      note      Fx Val
                 *  |
                 *  Description bit set to 0.
                 */

                let b2 = hio_read8(f);
                let b3 = hio_read8(f);

                event.note = b2 & 0x3f;
                if event.note != 0 {
                    event.note += 47;
                }
                event.ins = b1 & 0x1f;
                event.fxt = ((b2 >> 4) & 0x0c) | (b1 >> 5);
                event.fxp = b3;

                libxmp_disable_continue_fx(event);
                k += 1;
            }
        }
    }

    if libxmp_init_instrument(m) < 0 {
        return -1;
    }

    let smp = m.mod_.smp;
    hio_seek(
        f,
        (start as u32)
            .wrapping_add(sh.smpaddr)
            .wrapping_add((smp * 4) as u32) as i64,
        SEEK_SET,
    );

    for i in 0..smp {
        let iu = i as usize;
        si.size = hio_read16b(f);
        si.finetune = hio_read8(f);
        si.volume = hio_read8(f);
        si.loop_start = hio_read16b(f);
        si.loop_size = hio_read16b(f);

        let mod_ = &mut m.mod_;
        if libxmp_alloc_subinstrument(mod_, i, 1) < 0 {
            return -1;
        }

        let xxs = &mut mod_.xxs[iu];
        xxs.len = 2 * si.size as i32;
        xxs.lps = 2 * si.loop_start as i32;
        xxs.lpe = xxs.lps + 2 * si.loop_size as i32;
        xxs.flg = if si.loop_size > 1 { XMP_SAMPLE_LOOP } else { 0 };
        let len = xxs.len;
        let xxi = &mut mod_.xxi[iu];
        xxi.sub[0].fin = (si.finetune << 4) as i8 as i32;
        xxi.sub[0].vol = si.volume as i32;
        xxi.sub[0].pan = XMP_INST_NO_DEFAULT_PAN;
        xxi.sub[0].sid = i;
        xxi.rls = 0xfff;

        if len > 0 {
            xxi.nsm = 1;
        }

        if len == 0 {
            continue;
        }

        let smpctl = m.smpctl;
        if libxmp_load_sample(smpctl, Some(f), 0, &mut m.mod_.xxs[iu], None) < 0 {
            return -1;
        }
    }

    m.period_type = PERIOD_MODRNG;

    0
}
