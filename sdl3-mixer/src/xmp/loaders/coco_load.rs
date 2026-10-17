// Rust translation of src/loaders/coco_load.c from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2026 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Coconizer loader.

use super::super::common::*;
use super::super::effects::*;
use super::super::hio::*;
use super::super::*;
use super::common::*;
use super::sample::libxmp_load_sample;
use super::voltable::LIBXMP_ARCH_VOL_TABLE;
use super::*;

pub(crate) const LIBXMP_LOADER_COCO: FormatLoader = FormatLoader {
    name: "Coconizer",
    test: coco_test,
    loader: coco_load,
};

/// Translation of `check_cr()`.
fn check_cr(s: &[u8], n: usize) -> i32 {
    if s[..n].contains(&0x0d) {
        return 0;
    }

    -1
}

/// Translation of `coco_test()`.
fn coco_test(f: &mut HioHandle<'_>, t: Option<&mut [u8]>, start: i32) -> i32 {
    let mut buf = [0u8; 20];

    let x = hio_read8(f);

    /* check number of channels */
    if x != 0x84 && x != 0x88 {
        return -1;
    }

    if hio_read(&mut buf, 1, 20, f) != 20 {
        /* read title */
        return -1;
    }
    if check_cr(&buf, 20) != 0 {
        return -1;
    }

    let n = hio_read8(f) as i32; /* instruments */
    if n <= 0 || n > 100 {
        return -1;
    }

    hio_read8(f); /* sequences */
    hio_read8(f); /* patterns */

    let y = hio_read32l(f);
    if !(64..=0x00100000).contains(&y) {
        /* offset of sequence table */
        return -1;
    }

    let y = hio_read32l(f); /* offset of patterns */
    if !(64..=0x00100000).contains(&y) {
        return -1;
    }

    for _ in 0..n {
        let ofs = hio_read32l(f) as i32;
        let len = hio_read32l(f) as i32;
        let vol = hio_read32l(f) as i32;
        let lps = hio_read32l(f) as i32;
        let lsz = hio_read32l(f) as i32;

        if !(64..=0x00100000).contains(&ofs) {
            return -1;
        }

        if !(0..=0xff).contains(&vol) {
            return -1;
        }

        if len < 0 || lps < 0 || lsz < 0 {
            return -1;
        }
        if len > 0x00100000 || lps > 0x00100000 || lsz > 0x00100000 {
            return -1;
        }

        if lps > 0 && lps + lsz - 1 > len {
            return -1;
        }

        hio_read(&mut buf, 1, 11, f);
        hio_read8(f); /* unused */
    }

    hio_seek(f, start as i64 + 1, SEEK_SET);
    libxmp_read_title(f, t, 20);

    0
}

/// Translation of `fix_effect()`.
fn fix_effect(e: &mut XmpEvent) {
    match e.fxt {
        0x00 => {
            /* 00 xy Normal play or Arpeggio */
            e.fxt = FX_ARPEGGIO;
            /* x: first halfnote to add
            y: second halftone to subtract */
        }
        0x01 | 0x05 => {
            /* 01 xx Slide Pitch Up (until Amis Max), Frequency+InfoByte*64*/
            /* 05 xx Slide Pitch Up (no limit), Frequency+InfoByte*16 */
            e.fxt = FX_PORTA_UP;
        }
        0x02 | 0x06 => {
            /* 02 xx Slide Pitch Down (until Amis Min), Frequency-InfoByte*64*/
            /* 06 xx Slide Pitch Down (0 limit),  Frequency-InfoByte*16 */
            e.fxt = FX_PORTA_DN;
        }
        0x03 => {
            /* 03 xx Fine Volume Up */
            e.fxt = FX_F_VSLIDE_UP;
        }
        0x04 => {
            /* 04 xx Fine Volume Down */
            e.fxt = FX_F_VSLIDE_DN;
        }
        0x07 => {
            /* 07 xy Set Stereo Position */
            /* y: stereo position (1-7,ignored). 1=left 4=center 7=right */
            if e.fxp > 0 && e.fxp < 8 {
                e.fxt = FX_SETPAN;
                e.fxp = 42 * e.fxp - 40;
            } else {
                e.fxt = 0;
                e.fxp = 0;
            }
        }
        0x08..=0x0b => {
            /* 08 xx Start Auto Fine Volume Up */
            /* 09 xx Start Auto Fine Volume Down */
            /* 0A xx Start Auto Pitch Up */
            /* 0B xx Start Auto Pitch Down */
            e.fxt = 0;
            e.fxp = 0; /* FIXME */
        }
        0x0c => {
            /* 0C xx Set Volume */
            e.fxt = FX_VOLSET;
            e.fxp = 0xff - e.fxp;
        }
        0x0d => {
            /* 0D xy Pattern Break */
            e.fxt = FX_BREAK;
        }
        0x0e => {
            /* 0E xx Position Jump */
            e.fxt = FX_JUMP;
        }
        0x0f => {
            /* 0F xx Set Speed */
            e.fxt = FX_SPEED;
        }
        0x10 => {
            /* 10 xx Unused */
            e.fxt = 0;
            e.fxp = 0;
        }
        0x11 | 0x12 => {
            /* 11 xx Fine Slide Pitch Up */
            /* 12 xx Fine Slide Pitch Down */
            e.fxt = 0;
            e.fxp = 0; /* FIXME */
        }
        0x13 => {
            /* 13 xx Volume Up */
            e.fxt = FX_VOLSLIDE_UP;
        }
        0x14 => {
            /* 14 xx Volume Down */
            e.fxt = FX_VOLSLIDE_DN;
        }
        _ => {
            e.fxt = 0;
            e.fxp = 0;
        }
    }
}

/// Translation of `coco_load()`.
fn coco_load(m: &mut ModuleData, f: &mut HioHandle<'_>, start: i32) -> i32 {
    /* (upstream's table holds 100 entries, which the test enforces) */
    let mut smp_ptr = [0i32; 256];

    m.mod_.chn = (hio_read8(f) & 0x3f) as i32;
    libxmp_read_title(f, Some(&mut m.mod_.name), 20);

    for i in 0..20 {
        if m.mod_.name[i] == 0x0d {
            m.mod_.name[i] = 0;
        }
    }

    libxmp_set_type(m, b"Coconizer");

    let mod_ = &mut m.mod_;
    mod_.smp = hio_read8(f) as i32;
    mod_.ins = mod_.smp;
    mod_.len = hio_read8(f) as i32;
    mod_.pat = hio_read8(f) as i32;
    mod_.trk = mod_.pat * mod_.chn;

    let seq_ptr = hio_read32l(f) as i32;
    let pat_ptr = hio_read32l(f) as i32;

    if hio_error(f) != 0 {
        return -1;
    }

    if libxmp_init_instrument(m) < 0 {
        return -1;
    }

    m.vol_table = Some(&LIBXMP_ARCH_VOL_TABLE);
    m.volbase = 0xff;

    let mod_ = &mut m.mod_;
    for i in 0..mod_.ins {
        let iu = i as usize;
        if libxmp_alloc_subinstrument(mod_, i, 1) < 0 {
            return -1;
        }

        smp_ptr[iu] = hio_read32l(f) as i32;
        let xxs = &mut mod_.xxs[iu];
        let xxi = &mut mod_.xxi[iu];
        xxs.len = hio_read32l(f) as i32;
        xxi.sub[0].vol = 0xffu32.wrapping_sub(hio_read32l(f)) as i32;
        xxi.sub[0].pan = XMP_INST_NO_DEFAULT_PAN;
        xxs.lps = hio_read32l(f) as i32;
        xxs.lpe = (xxs.lps as u32).wrapping_add(hio_read32l(f)) as i32;
        if xxs.lpe != 0 {
            xxs.lpe = xxs.lpe.wrapping_sub(1);
        }
        xxs.flg = if xxs.lps > 0 { XMP_SAMPLE_LOOP } else { 0 };
        hio_read(&mut xxi.name, 1, 11, f);
        for j in 0..11 {
            if xxi.name[j] == 0x0d {
                xxi.name[j] = 0;
            }
        }
        hio_read8(f); /* unused */
        xxi.sub[0].sid = i;

        if xxs.len > 0 {
            xxi.nsm = 1;
        }

        if hio_error(f) != 0 {
            return -1;
        }
    }

    /* Sequence */

    hio_seek(f, start.wrapping_add(seq_ptr) as i64, SEEK_SET);
    let mut i = 0;
    loop {
        let x = hio_read8(f);
        if x == 0xff {
            break;
        }
        if i < mod_.len {
            mod_.xxo[i as usize] = x;
        }
        i += 1;
    }

    /* Patterns */

    if libxmp_init_pattern(mod_) < 0 {
        return -1;
    }

    if hio_seek(f, start.wrapping_add(pat_ptr) as i64, SEEK_SET) < 0 {
        return -1;
    }

    for i in 0..mod_.pat {
        if libxmp_alloc_pattern_tracks(mod_, i, 64) < 0 {
            return -1;
        }

        for j in 0..64 {
            for k in 0..mod_.chn {
                let event = mod_.event_mut(i, k, j);
                event.fxp = hio_read8(f);
                event.fxt = hio_read8(f);
                event.ins = hio_read8(f);
                event.note = hio_read8(f);
                if event.note != 0 {
                    event.note = event.note.wrapping_add(12);
                }

                if hio_error(f) != 0 {
                    return -1;
                }

                fix_effect(event);
            }
        }
    }

    /* Read samples */

    for i in 0..m.mod_.ins as usize {
        if m.mod_.xxi[i].nsm == 0 {
            continue;
        }

        hio_seek(f, start.wrapping_add(smp_ptr[i]) as i64, SEEK_SET);
        let smpctl = m.smpctl;
        if libxmp_load_sample(smpctl, Some(f), SAMPLE_FLAG_VIDC, &mut m.mod_.xxs[i], None) < 0 {
            return -1;
        }
    }

    for i in 0..m.mod_.chn {
        let pan = defpan(m, (((i + 3) / 2) % 2) * 0xff);
        m.mod_.xxc[i as usize].pan = pan;
    }

    0
}
