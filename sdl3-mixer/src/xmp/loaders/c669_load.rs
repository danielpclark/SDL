// Rust translation of src/loaders/669_load.c from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2026 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Composer 669 loader.

use super::super::common::*;
use super::super::effects::*;
use super::super::hio::*;
use super::super::*;
use super::common::*;
use super::sample::libxmp_load_sample;
use super::*;

pub(crate) const LIBXMP_LOADER_669: FormatLoader = FormatLoader {
    name: "Composer 669",
    test: c669_test,
    loader: c669_load,
};

/// Translation of `c669_test()`.
fn c669_test(f: &mut HioHandle<'_>, t: Option<&mut [u8]>, start: i32) -> i32 {
    let id = hio_read16b(f);
    if id != 0x6966 && id != 0x4a4e {
        return -1;
    }

    hio_seek(f, 110, SEEK_SET);
    if hio_read8(f) > 64 {
        return -1;
    }
    if hio_read8(f) > 128 {
        return -1;
    }

    hio_seek(f, 240, SEEK_SET);
    if hio_read8(f) != 0xff {
        return -1;
    }

    hio_seek(f, start as i64 + 2, SEEK_SET);
    libxmp_read_title(f, t, 36);

    0
}

/// Translation of `struct c669_file_header`.
struct C669FileHeader {
    marker: [u8; 2],    /* 'if'=standard, 'JN'=extended */
    message: [u8; 108], /* Song message */
    nos: u8,            /* Number of samples (0-64) */
    nop: u8,            /* Number of patterns (0-128) */
    loop_: u8,          /* Loop order number */
    order: [u8; 128],   /* Order list */
    speed: [u8; 128],   /* Tempo list for patterns */
    pbrk: [u8; 128],    /* Break list for patterns */
}

/// Translation of `struct c669_instrument_header`.
#[derive(Default)]
struct C669InstrumentHeader {
    name: [u8; 13],  /* ASCIIZ instrument name */
    length: u32,     /* Instrument length */
    loop_start: u32, /* Instrument loop start */
    loopend: u32,    /* Instrument loop end */
}

/* Effects bug fixed by Miod Vallat <miodrag@multimania.com> */

static FX: [u8; 6] = [
    FX_669_PORTA_UP,
    FX_669_PORTA_DN,
    FX_669_TPORTA,
    FX_669_FINETUNE,
    FX_669_VIBRATO,
    FX_SPEED_CP,
];

/// Translation of `c669_load()`.
fn c669_load(m: &mut ModuleData, f: &mut HioHandle<'_>, _start: i32) -> i32 {
    let mut sfh = C669FileHeader {
        marker: [0; 2],
        message: [0; 108],
        nos: 0,
        nop: 0,
        loop_: 0,
        order: [0; 128],
        speed: [0; 128],
        pbrk: [0; 128],
    };
    let mut sih = C669InstrumentHeader::default();
    let mut ev = [0u8; 3];

    hio_read(&mut sfh.marker, 2, 1, f); /* 'if'=standard, 'JN'=extended */
    hio_read(&mut sfh.message, 108, 1, f); /* Song message */
    sfh.nos = hio_read8(f); /* Number of samples (0-64) */
    sfh.nop = hio_read8(f); /* Number of patterns (0-128) */

    /* Sanity check */
    if sfh.nos > 64 || sfh.nop > 128 {
        return -1;
    }

    sfh.loop_ = hio_read8(f); /* Loop order number */
    let _ = sfh.loop_;
    if hio_read(&mut sfh.order, 1, 128, f) != 128 {
        /* Order list */
        return -1;
    }
    if hio_read(&mut sfh.speed, 1, 128, f) != 128 {
        /* Tempo list for patterns */
        return -1;
    }
    if hio_read(&mut sfh.pbrk, 1, 128, f) != 128 {
        /* Break list for patterns */
        return -1;
    }

    let mod_ = &mut m.mod_;
    mod_.chn = 8;
    mod_.ins = sfh.nos as i32;
    mod_.pat = sfh.nop as i32;
    mod_.trk = mod_.chn * mod_.pat;
    let mut i = 0;
    while i < 128 {
        if sfh.order[i] > sfh.nop {
            break;
        }
        i += 1;
    }
    mod_.len = i as i32;
    mod_.xxo[..i].copy_from_slice(&sfh.order[..i]);
    mod_.spd = 6;
    mod_.bpm = 78;
    mod_.smp = mod_.ins;

    m.period_type = PERIOD_CSPD;
    m.c4rate = C4_NTSC_RATE;

    libxmp_copy_adjust(&mut m.mod_.name, &sfh.message, 36);
    libxmp_set_type(
        m,
        if &sfh.marker != b"if" {
            b"UNIS 669"
        } else {
            b"Composer 669"
        },
    );

    if let Ok(mut comment) = try_vec(109, 0u8) {
        comment[..108].copy_from_slice(&sfh.message);
        comment[108] = 0;
        m.comment = Some(comment);
    }

    /* Read and convert instruments and samples */

    if libxmp_init_instrument(m) < 0 {
        return -1;
    }

    let mod_ = &mut m.mod_;
    for i in 0..mod_.ins {
        let iu = i as usize;

        if libxmp_alloc_subinstrument(mod_, i, 1) < 0 {
            return -1;
        }

        hio_read(&mut sih.name, 13, 1, f); /* ASCIIZ instrument name */
        sih.length = hio_read32l(f); /* Instrument size */
        sih.loop_start = hio_read32l(f); /* Instrument loop start */
        sih.loopend = hio_read32l(f); /* Instrument loop end */

        /* Sanity check */
        if sih.length > MAX_SAMPLE_SIZE as u32 {
            return -1;
        }

        let xxs = &mut mod_.xxs[iu];
        xxs.len = sih.length as i32;
        xxs.lps = sih.loop_start as i32;
        xxs.lpe = if sih.loopend >= 0xfffff {
            0
        } else {
            sih.loopend as i32
        };
        xxs.flg = if xxs.lpe != 0 { XMP_SAMPLE_LOOP } else { 0 }; /* 1 == Forward loop */
        let len = xxs.len;

        let xxi = &mut mod_.xxi[iu];
        let sub = &mut xxi.sub[0];
        sub.vol = 0x40;
        sub.pan = XMP_INST_NO_DEFAULT_PAN;
        sub.sid = i;

        if len > 0 {
            xxi.nsm = 1;
        }

        libxmp_instrument_name(mod_, i, &sih.name, 13);
    }

    if libxmp_init_pattern(mod_) < 0 {
        return -1;
    }

    /* Read and convert patterns */
    for i in 0..mod_.pat {
        if libxmp_alloc_pattern_tracks(mod_, i, 64) < 0 {
            return -1;
        }

        let event = mod_.event_mut(i, 0, 0);
        event.f2t = FX_SPEED_CP;
        event.f2p = sfh.speed[i as usize];

        let pbrk = sfh.pbrk[i as usize] as i32;
        if pbrk >= 64 {
            return -1;
        }

        let event = mod_.event_mut(i, 1, pbrk);
        event.f2t = FX_BREAK;
        event.f2p = 0;

        for j in 0..64 * 8 {
            let event = mod_.event_mut(i, j % 8, j / 8);
            if hio_read(&mut ev, 1, 3, f) < 3 {
                return -1;
            }

            if (ev[0] & 0xfe) != 0xfe {
                event.note = 1 + 36 + (ev[0] >> 2);
                event.ins = (1 + msn(ev[1]) + (((ev[0] & 0x03) as i32) << 4)) as u8;
            }

            if ev[0] != 0xff {
                event.vol = ((lsn(ev[1]) << 2) + 1) as u8;
            }

            if ev[2] != 0xff {
                if msn(ev[2]) as usize >= FX.len() {
                    continue;
                }

                event.fxt = FX[msn(ev[2]) as usize];
                event.fxp = lsn(ev[2]) as u8;

                if event.fxt == FX_SPEED_CP {
                    event.f2t = FX_PER_CANCEL;
                }
            }
        }
    }

    /* Read samples */

    for i in 0..m.mod_.ins as usize {
        if m.mod_.xxs[i].len <= 2 {
            continue;
        }
        let smpctl = m.smpctl;
        if libxmp_load_sample(smpctl, Some(f), SAMPLE_FLAG_UNS, &mut m.mod_.xxs[i], None) < 0 {
            return -1;
        }
    }

    for i in 0..m.mod_.chn {
        let pan = defpan(m, (i % 2) * 0xff);
        m.mod_.xxc[i as usize].pan = pan;
    }

    m.quirk |= QUIRK_PBALL | QUIRK_PERPAT;

    0
}
