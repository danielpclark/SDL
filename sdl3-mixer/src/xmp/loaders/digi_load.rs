// Rust translation of src/loaders/digi_load.c from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2026 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

/* Based on the DIGI Booster player v1.6 by Tap (Tomasz Piasta), with the
 * help of Louise Heimann <louise.heimann@softhome.net>. The following
 * DIGI Booster effects are _NOT_ recognized by this player:
 *
 * 8xx robot
 * e00 filter off
 * e01 filter on
 * e30 backwd play sample
 * e31 backwd play sample+loop
 * e50 channel off
 * e51 channel on
 * e8x sample offset 2
 * e9x retrace
 */

//! The DIGI Booster loader.

use super::super::common::*;
use super::super::hio::*;
use super::super::*;
use super::common::*;
use super::sample::libxmp_load_sample;
use super::*;

pub(crate) const LIBXMP_LOADER_DIGI: FormatLoader = FormatLoader {
    name: "DIGI Booster",
    test: digi_test,
    loader: digi_load,
};

/// Translation of `digi_test()`.
fn digi_test(f: &mut HioHandle<'_>, t: Option<&mut [u8]>, _start: i32) -> i32 {
    let mut buf = [0u8; 20];

    if hio_read(&mut buf, 1, 20, f) < 20 {
        return -1;
    }

    if &buf[..19] != b"DIGI Booster module" {
        return -1;
    }

    hio_seek(f, 156, SEEK_CUR);
    hio_seek(f, 3 * 4 * 32, SEEK_CUR);
    hio_seek(f, 2 * 32, SEEK_CUR);

    libxmp_read_title(f, t, 32);

    0
}

/// Translation of `struct digi_header`.
struct DigiHeader {
    id: [u8; 20],            /* ID: "DIGI Booster module\0" */
    vstr: [u8; 4],           /* Version string: "Vx.y" */
    ver: u8,                 /* Version hi-nibble.lo-nibble */
    chn: u8,                 /* Number of channels */
    pack: u8,                /* PackEnable */
    unknown: [u8; 19],       /* ?! */
    pat: u8,                 /* Number of patterns */
    len: u8,                 /* Song length */
    ord: [u8; 128],          /* Orders */
    slen: [u32; 31],         /* Sample length for 31 samples */
    sloop: [u32; 31],        /* Sample loop start for 31 samples */
    sllen: [u32; 31],        /* Sample loop length for 31 samples */
    vol: [u8; 31],           /* Instrument volumes */
    fin: [i8; 31],           /* Finetunes */
    title: [u8; 32],         /* Song name */
    insname: [[u8; 30]; 31], /* Instrument names */
}

/// Translation of `digi_load()`.
fn digi_load(m: &mut ModuleData, f: &mut HioHandle<'_>, _start: i32) -> i32 {
    let mut digi_event = [0u8; 4];
    let mut chn_table = [0u8; 64];
    let mut dh = DigiHeader {
        id: [0; 20],
        vstr: [0; 4],
        ver: 0,
        chn: 0,
        pack: 0,
        unknown: [0; 19],
        pat: 0,
        len: 0,
        ord: [0; 128],
        slen: [0; 31],
        sloop: [0; 31],
        sllen: [0; 31],
        vol: [0; 31],
        fin: [0; 31],
        title: [0; 32],
        insname: [[0; 30]; 31],
    };

    hio_read(&mut dh.id, 20, 1, f);

    hio_read(&mut dh.vstr, 4, 1, f);
    dh.ver = hio_read8(f);
    dh.chn = hio_read8(f);
    dh.pack = hio_read8(f);
    hio_read(&mut dh.unknown, 19, 1, f);
    dh.pat = hio_read8(f);
    dh.len = hio_read8(f);

    /* Sanity check */
    if dh.len > 127 {
        return -1;
    }

    hio_read(&mut dh.ord, 128, 1, f);

    for i in 0..31 {
        dh.slen[i] = hio_read32b(f);
    }
    for i in 0..31 {
        dh.sloop[i] = hio_read32b(f);
    }
    for i in 0..31 {
        dh.sllen[i] = hio_read32b(f);
    }
    for i in 0..31 {
        dh.vol[i] = hio_read8(f);
    }
    for i in 0..31 {
        dh.fin[i] = hio_read8s(f);
    }

    if hio_read(&mut dh.title, 1, 32, f) < 32 {
        return -1;
    }

    for i in 0..31 {
        if hio_read(&mut dh.insname[i], 1, 30, f) < 30 {
            return -1;
        }
    }

    let mod_ = &mut m.mod_;
    mod_.ins = 31;
    mod_.smp = mod_.ins;
    mod_.pat = dh.pat as i32 + 1;
    mod_.chn = dh.chn as i32;
    mod_.trk = mod_.pat * mod_.chn;
    mod_.len = dh.len as i32 + 1;

    m.period_type = PERIOD_MODRNG;

    libxmp_copy_adjust(&mut m.mod_.name, &dh.title, 32);
    let mut type_ = b"DIGI Booster ".to_vec();
    type_.extend_from_slice(&printf_s_left(&dh.vstr, 4, 4));
    libxmp_set_type(m, &type_);

    let mod_ = &mut m.mod_;
    for i in 0..mod_.len as usize {
        mod_.xxo[i] = dh.ord[i];
    }

    if libxmp_init_instrument(m) < 0 {
        return -1;
    }

    let mod_ = &mut m.mod_;
    /* Read and convert instruments and samples */

    for i in 0..mod_.ins {
        let iu = i as usize;
        if libxmp_alloc_subinstrument(mod_, i, 1) < 0 {
            return -1;
        }

        let xxs = &mut mod_.xxs[iu];
        xxs.len = dh.slen[iu] as i32;
        xxs.lps = dh.sloop[iu] as i32;
        xxs.lpe = dh.sloop[iu].wrapping_add(dh.sllen[iu]) as i32;
        xxs.flg = if xxs.lpe > 0 { XMP_SAMPLE_LOOP } else { 0 };
        let len = xxs.len;
        let sub = &mut mod_.xxi[iu].sub[0];
        sub.vol = dh.vol[iu] as i32;
        sub.fin = dh.fin[iu] as i32;
        sub.pan = XMP_INST_NO_DEFAULT_PAN;
        sub.sid = i;

        if len > 0 {
            mod_.xxi[iu].nsm = 1;
        }

        libxmp_instrument_name(mod_, i, &dh.insname[iu], 30);
    }

    if libxmp_init_pattern(mod_) < 0 {
        return -1;
    }

    /* Read and convert patterns */

    for i in 0..mod_.pat {
        let mut w: u16;

        if libxmp_alloc_pattern_tracks(mod_, i, 64) < 0 {
            return -1;
        }

        if dh.pack != 0 {
            w = ((hio_read16b(f) as i32 - 64) >> 2) as u16;
            if hio_read(&mut chn_table, 1, 64, f) < 64 {
                return -1;
            }
        } else {
            w = (64 * mod_.chn) as u16;
            chn_table.fill(0xff);
        }

        for j in 0..64 {
            let mut k: u32 = 0x80;
            for c in 0..mod_.chn {
                if chn_table[j as usize] as u32 & k != 0 {
                    if hio_read(&mut digi_event, 1, 4, f) < 4 {
                        return -1;
                    }
                    let event = mod_.event_mut(i, c, j);
                    libxmp_decode_protracker_event(event, &digi_event);
                    match event.fxt {
                        0x08 => {
                            /* Robot */
                            event.fxt = 0;
                            event.fxp = 0;
                        }
                        0x0e => match msn(event.fxp) {
                            0x00 | 0x03 | 0x08 | 0x09 => {
                                event.fxt = 0;
                                event.fxp = 0;
                            }
                            0x04 => {
                                event.fxt = 0x0c;
                                event.fxp = 0x00;
                            }
                            _ => {}
                        },
                        _ => {}
                    }
                    w = w.wrapping_sub(1);
                }
                k >>= 1;
            }
        }

        if w != 0 { /* (upstream reports the corrupted file) */ }
    }

    /* Read samples */
    for i in 0..m.mod_.ins as usize {
        let smpctl = m.smpctl;
        if libxmp_load_sample(smpctl, Some(f), 0, &mut m.mod_.xxs[i], None) < 0 {
            return -1;
        }
    }

    0
}
