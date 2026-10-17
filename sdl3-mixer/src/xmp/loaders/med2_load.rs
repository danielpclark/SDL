// Rust translation of src/loaders/med2_load.c from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2026 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

/*
 * MED 1.12 is in Fish disk #255
 */

//! The MED 1.12 (MED2) loader.

use super::super::common::*;
use super::super::effects::*;
use super::super::hio::*;
use super::super::period::libxmp_period_to_note;
use super::super::*;
use super::common::*;
use super::mmd_common::med_load_external_instrument;
use super::*;

const MAGIC_MED2: u32 = magic4(b'M', b'E', b'D', 2);

pub(crate) const LIBXMP_LOADER_MED2: FormatLoader = FormatLoader {
    name: "MED 1.12 MED2",
    test: med2_test,
    loader: med2_load,
};

/// Translation of `med2_test()`.
fn med2_test(f: &mut HioHandle<'_>, t: Option<&mut [u8]>, _start: i32) -> i32 {
    if hio_read32b(f) != MAGIC_MED2 {
        return -1;
    }

    libxmp_read_title(f, t, 0);

    0
}

/// Translation of `med2_load()`.
fn med2_load(m: &mut ModuleData, f: &mut HioHandle<'_>, _start: i32) -> i32 {
    let mut buf = [0u8; 40];

    if hio_read32b(f) != MAGIC_MED2 {
        return -1;
    }

    libxmp_set_type(m, b"MED 1.12 MED2");

    m.mod_.ins = 32;
    m.mod_.smp = 32;

    if libxmp_init_instrument(m) < 0 {
        return -1;
    }

    let mod_ = &mut m.mod_;
    /* read instrument names */
    hio_read(&mut buf, 1, 40, f); /* skip 0 */
    for i in 0..31 {
        if hio_read(&mut buf, 1, 40, f) != 40 {
            return -1;
        }

        libxmp_instrument_name(mod_, i, &buf, 40);
        if libxmp_alloc_subinstrument(mod_, i, 1) < 0 {
            return -1;
        }
    }

    /* read instrument volumes */
    hio_read8(f); /* skip 0 */
    for i in 0..31 {
        let sub = &mut mod_.xxi[i].sub[0];
        sub.vol = hio_read8(f) as i32;
        sub.pan = XMP_INST_NO_DEFAULT_PAN;
        sub.fin = 0;
        sub.sid = i as i32;
    }

    /* read instrument loops */
    hio_read16b(f); /* skip 0 */
    for i in 0..31 {
        mod_.xxs[i].lps = hio_read16b(f) as i32;
    }

    /* read instrument loop length */
    hio_read16b(f); /* skip 0 */
    for i in 0..31 {
        let lsiz = hio_read16b(f) as u32;
        let xxs = &mut mod_.xxs[i];
        xxs.lpe = (xxs.lps as u32).wrapping_add(lsiz) as i32;
        xxs.flg = if lsiz > 1 { XMP_SAMPLE_LOOP } else { 0 };
    }

    mod_.chn = 4;
    mod_.pat = hio_read16b(f) as i32;
    mod_.trk = mod_.chn * mod_.pat;

    if hio_read(&mut mod_.xxo, 1, 100, f) != 100 {
        return -1;
    }

    mod_.len = hio_read16b(f) as i32;

    /* Sanity check */
    if mod_.pat > 256 || mod_.len > 100 {
        return -1;
    }

    let k = hio_read16b(f) as i32;
    if k < 1 {
        return -1;
    }

    mod_.spd = 6;
    mod_.bpm = k;
    m.time_factor = MED_TIME_FACTOR;

    hio_read16b(f); /* flags */
    let sliding = hio_read16b(f); /* sliding */
    hio_read32b(f); /* jumping mask */
    hio_seek(f, 16, SEEK_CUR); /* rgb */

    if sliding == 6 {
        m.quirk |= QUIRK_VSALL | QUIRK_PBALL;
    }

    let mod_ = &mut m.mod_;
    if libxmp_init_pattern(mod_) < 0 {
        return -1;
    }

    /* Load and convert patterns */

    for i in 0..mod_.pat {
        if libxmp_alloc_pattern_tracks(mod_, i, 64) < 0 {
            return -1;
        }

        hio_read32b(f);

        for j in 0..64 {
            for k in 0..4 {
                let event = mod_.event_mut(i, k, j);
                event.note = libxmp_period_to_note(hio_read16b(f) as i32) as u8;
                let x = hio_read8(f);
                event.ins = x >> 4;
                event.fxt = x & 0x0f;
                event.fxp = hio_read8(f);

                match event.fxt {
                    0x00 /* arpeggio */
                    | 0x01 /* slide up */
                    | 0x02 /* slide down */
                    | 0x03 /* portamento */
                    | 0x04 /* vibrato? */
                    | 0x0c /* volume */ => {
                        /* ...like protracker */
                    }
                    0x0d /* volslide */ | 0x0e /* volslide */ => {
                        event.fxt = FX_VOLSLIDE;
                    }
                    0x0f => {
                        event.fxt = FX_S3M_BPM;
                    }
                    _ => {}
                }
            }
        }
    }

    /* Load samples */

    for i in 0..31 {
        if med_load_external_instrument(f, m, i) != 0 {
            return -1;
        }
    }

    0
}
