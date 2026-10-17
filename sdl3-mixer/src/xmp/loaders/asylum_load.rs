// Rust translation of src/loaders/asylum_load.c from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2026 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

/*
 * Based on AMF->MOD converter written by Mr. P / Powersource, 1995
 */

//! The Asylum Music Format (AMF) loader.

use super::super::common::*;
use super::super::dataio::*;
use super::super::effects::*;
use super::super::hio::*;
use super::super::*;
use super::common::*;
use super::sample::libxmp_load_sample;
use super::*;

pub(crate) const LIBXMP_LOADER_ASYLUM: FormatLoader = FormatLoader {
    name: "Asylum Music Format v1.0",
    test: asylum_test,
    loader: asylum_load,
};

/// Translation of `asylum_test()`.
fn asylum_test(f: &mut HioHandle<'_>, t: Option<&mut [u8]>, _start: i32) -> i32 {
    let mut buf = [0u8; 32];

    if hio_read(&mut buf, 1, 32, f) < 32 {
        return -1;
    }

    if &buf != b"ASYLUM Music Format V1.0\0\0\0\0\0\0\0\0" {
        return -1;
    }

    libxmp_read_title(f, t, 0);

    0
}

/// Translation of `asylum_load()`.
fn asylum_load(m: &mut ModuleData, f: &mut HioHandle<'_>, start: i32) -> i32 {
    let mut buf = [0u8; 2048];

    let mod_ = &mut m.mod_;
    hio_seek(f, 32, SEEK_CUR); /* skip magic */
    mod_.spd = hio_read8(f) as i32; /* initial speed */
    mod_.bpm = hio_read8(f) as i32; /* initial BPM */
    mod_.ins = hio_read8(f) as i32; /* number of instruments */
    mod_.pat = hio_read8(f) as i32; /* number of patterns */
    mod_.len = hio_read8(f) as i32; /* module length */
    mod_.rst = hio_read8(f) as i32; /* restart byte */

    /* Sanity check - this format only stores 64 sample structures. */
    if mod_.ins > 64 {
        return -1;
    }

    let len = mod_.len as usize;
    hio_read(&mut mod_.xxo, 1, len, f); /* read orders */
    hio_seek(f, start as i64 + 294, SEEK_SET);

    mod_.chn = 8;
    mod_.smp = mod_.ins;
    mod_.trk = mod_.pat * mod_.chn;

    snprintf_bytes(&mut mod_.type_, XMP_NAME_SIZE, b"Asylum Music Format v1.0");

    if libxmp_init_instrument(m) < 0 {
        return -1;
    }

    let mod_ = &mut m.mod_;
    /* Read and convert instruments and samples */
    for i in 0..mod_.ins {
        let iu = i as usize;
        let mut insbuf = [0u8; 37];

        if libxmp_alloc_subinstrument(mod_, i, 1) < 0 {
            return -1;
        }

        if hio_read(&mut insbuf, 1, 37, f) != 37 {
            return -1;
        }

        libxmp_instrument_name(mod_, i, &insbuf, 22);
        let sub = &mut mod_.xxi[iu].sub[0];
        sub.fin = (insbuf[22] << 4) as i8 as i32;
        sub.vol = insbuf[23] as i32;
        sub.xpo = insbuf[24] as i8 as i32;
        sub.pan = XMP_INST_NO_DEFAULT_PAN;
        sub.sid = i;

        let xxs = &mut mod_.xxs[iu];
        xxs.len = readmem32l(&insbuf[25..]) as i32;
        xxs.lps = readmem32l(&insbuf[29..]) as i32;
        xxs.lpe = xxs.lps.wrapping_add(readmem32l(&insbuf[33..]) as i32);

        /* Sanity check - ASYLUM modules are converted from MODs. */
        if xxs.len as u32 >= 0x20000 {
            return -1;
        }

        xxs.flg = if xxs.lpe > 2 { XMP_SAMPLE_LOOP } else { 0 };
    }

    hio_seek(f, 37 * (64 - mod_.ins) as i64, SEEK_CUR);

    if libxmp_init_pattern(mod_) < 0 {
        return -1;
    }

    /* Read and convert patterns */

    for i in 0..mod_.pat {
        if libxmp_alloc_pattern_tracks(mod_, i, 64) < 0 {
            return -1;
        }

        if hio_read(&mut buf, 1, 2048, f) < 2048 {
            return -1;
        }

        for (j, pos) in buf.chunks_exact(4).enumerate() {
            let j = j as i32;
            let event = mod_.event_mut(i, j % 8, j / 8);
            *event = XmpEvent::default();
            let note = pos[0];

            if note != 0 {
                event.note = note.wrapping_add(13);
            }

            event.ins = pos[1];
            event.fxt = pos[2];
            event.fxp = pos[3];

            /* TODO: m07.amf and m12.amf from Crusader: No Remorse
             * use 0x1b for what looks *plausibly* like retrigger.
             * No other ASYLUM modules use effects over 16. */
            if event.fxt >= 0x10 && event.fxt != FX_MULTI_RETRIG {
                event.fxt = 0;
                event.fxp = 0;
            }
        }
    }

    /* Read samples */

    for i in 0..m.mod_.ins as usize {
        if m.mod_.xxs[i].len > 1 {
            let smpctl = m.smpctl;
            if libxmp_load_sample(smpctl, Some(f), 0, &mut m.mod_.xxs[i], None) < 0 {
                return -1;
            }
            m.mod_.xxi[i].nsm = 1;
        }
    }

    0
}
