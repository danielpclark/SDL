// Rust translation of src/loaders/emod_load.c from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2026 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Quadra Composer (EMOD) loader.

use super::super::common::*;
use super::super::hio::*;
use super::super::*;
use super::common::*;
use super::iff::*;
use super::sample::libxmp_load_sample;
use super::*;

const MAGIC_FORM: u32 = magic4(b'F', b'O', b'R', b'M');
const MAGIC_EMOD: u32 = magic4(b'E', b'M', b'O', b'D');
const MAGIC_EMIC: u32 = magic4(b'E', b'M', b'I', b'C');

pub(crate) const LIBXMP_LOADER_EMOD: FormatLoader = FormatLoader {
    name: "Quadra Composer",
    test: emod_test,
    loader: emod_load,
};

/// Translation of `emod_test()`.
fn emod_test(f: &mut HioHandle<'_>, t: Option<&mut [u8]>, _start: i32) -> i32 {
    if hio_read32b(f) != MAGIC_FORM {
        return -1;
    }

    hio_read32b(f);

    if hio_read32b(f) != MAGIC_EMOD {
        return -1;
    }

    if hio_read32b(f) == MAGIC_EMIC {
        hio_read32b(f); /* skip size */
        hio_read16b(f); /* skip version */
        libxmp_read_title(f, t, 20);
    } else {
        libxmp_read_title(f, t, 0);
    }

    0
}

/// Translation of `struct local_data`.
#[derive(Default)]
struct LocalData {
    has_emic: bool,
    has_patt: bool,
    has_8smp: bool,
}

/// Translation of `get_emic()`.
fn get_emic(m: &mut ModuleData, _size: u32, f: &mut HioHandle<'_>, data: &mut LocalData) -> i32 {
    let mut reorder = [0u8; 256];

    /* Sanity check */
    if data.has_emic {
        return -1;
    }
    data.has_emic = true;

    let ver = hio_read16b(f) as i32;
    let mod_ = &mut m.mod_;
    hio_read(&mut mod_.name, 1, 20, f);
    hio_seek(f, 20, SEEK_CUR);
    mod_.bpm = hio_read8(f) as i32;
    mod_.ins = hio_read8(f) as i32;
    mod_.smp = mod_.ins;

    m.period_type = PERIOD_MODRNG;

    let type_ = format!("Quadra Composer EMOD v{ver}");
    libxmp_set_type(m, type_.as_bytes());

    if libxmp_init_instrument(m) < 0 {
        return -1;
    }

    let mod_ = &mut m.mod_;
    for i in 0..mod_.ins {
        let iu = i as usize;
        let mut name = [0u8; 20];

        if libxmp_alloc_subinstrument(mod_, i, 1) < 0 {
            return -1;
        }

        hio_read8(f); /* num */
        mod_.xxi[iu].sub[0].vol = hio_read8(f) as i32;
        mod_.xxs[iu].len = 2 * hio_read16b(f) as i32;
        if hio_read(&mut name, 1, 20, f) < 20 {
            return -1;
        }
        libxmp_instrument_name(mod_, i, &name, 20);
        let xxi = &mut mod_.xxi[iu];
        let xxs = &mut mod_.xxs[iu];
        let sub = &mut xxi.sub[0];
        xxs.flg = if hio_read8(f) & 1 != 0 {
            XMP_SAMPLE_LOOP
        } else {
            0
        };
        sub.fin = (hio_read8(f) << 4) as i8 as i32;
        xxs.lps = 2 * hio_read16b(f) as i32;
        xxs.lpe = xxs.lps + 2 * hio_read16b(f) as i32;
        hio_read32b(f); /* ptr */

        xxi.nsm = 1;
        sub.pan = XMP_INST_NO_DEFAULT_PAN;
        sub.sid = i;
    }

    hio_read8(f); /* pad */
    mod_.pat = hio_read8(f) as i32;
    mod_.trk = mod_.pat * mod_.chn;

    if libxmp_init_pattern(mod_) < 0 {
        return -1;
    }

    for i in 0..mod_.pat {
        reorder[hio_read8(f) as usize] = i as u8;

        if libxmp_alloc_pattern_tracks(mod_, i, hio_read8(f) as i32 + 1) < 0 {
            return -1;
        }

        hio_seek(f, 20, SEEK_CUR); /* skip name */
        hio_read32b(f); /* ptr */
    }

    mod_.len = hio_read8(f) as i32;

    for i in 0..mod_.len as usize {
        mod_.xxo[i] = reorder[hio_read8(f) as usize];
    }

    0
}

/// Translation of `get_patt()`.
fn get_patt(m: &mut ModuleData, _size: u32, f: &mut HioHandle<'_>, data: &mut LocalData) -> i32 {
    let mod_ = &mut m.mod_;

    /* Sanity check */
    if data.has_patt || !data.has_emic {
        return -1;
    }
    data.has_patt = true;

    for i in 0..mod_.pat {
        for j in 0..mod_.pattern(i).rows {
            for k in 0..mod_.chn {
                let event = mod_.event_mut(i, k, j);
                event.ins = hio_read8(f);
                event.note = hio_read8(f).wrapping_add(1);
                if event.note != 0 {
                    event.note = event.note.wrapping_add(48);
                }
                event.fxt = hio_read8(f) & 0x0f;
                event.fxp = hio_read8(f);

                /* Fix effects */
                match event.fxt {
                    0x04 => {
                        let x = event.fxp;
                        event.fxp = (x & 0xf0) | ((x << 1) & 0x0f);
                    }
                    0x09 => {
                        /* Note (upstream): C shifts the effect type (making
                         * it 0x12, no effect) instead of the parameter;
                         * Quadra Composer's offsets are in 512 bytes. (One
                         * of 0x80 or more still loses its top bit.) */
                        event.fxp <<= 1;
                    }
                    0x0d => {
                        /* Note (upstream): C converts the type of a 0x0b
                         * (position jump) here, making it a global volume
                         * slide; Quadra Composer's jump and break take a
                         * binary parameter, and the break's row is made
                         * the BCD that FX_BREAK reads. */
                        let x = event.fxp as u32;
                        event.fxp = (16 * (x / 10) + x % 10) as u8;
                    }
                    _ => {}
                }
            }
        }
    }

    0
}

/// Translation of `get_8smp()`.
fn get_8smp(m: &mut ModuleData, _size: u32, f: &mut HioHandle<'_>, data: &mut LocalData) -> i32 {
    /* Sanity check */
    if data.has_8smp || !data.has_emic {
        return -1;
    }
    data.has_8smp = true;

    for i in 0..m.mod_.smp as usize {
        let smpctl = m.smpctl;
        if libxmp_load_sample(smpctl, Some(f), 0, &mut m.mod_.xxs[i], None) < 0 {
            return -1;
        }
    }

    0
}

/// Translation of `emod_load()`.
fn emod_load(m: &mut ModuleData, f: &mut HioHandle<'_>, _start: i32) -> i32 {
    let mut data = LocalData::default();

    hio_read32b(f); /* FORM */
    hio_read32b(f);
    hio_read32b(f); /* EMOD */

    let mut handle = libxmp_iff_new::<LocalData>();

    /* IFF chunk IDs */
    let mut ret = libxmp_iff_register(&mut handle, b"EMIC", get_emic);
    ret |= libxmp_iff_register(&mut handle, b"PATT", get_patt);
    ret |= libxmp_iff_register(&mut handle, b"8SMP", get_8smp);

    if ret != 0 {
        return -1;
    }

    /* Load IFF chunks */
    if libxmp_iff_load(&handle, m, f, &mut data) < 0 {
        libxmp_iff_release(handle);
        return -1;
    }

    libxmp_iff_release(handle);

    0
}
