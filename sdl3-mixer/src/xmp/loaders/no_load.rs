// Rust translation of src/loaders/no_load.c from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2026 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

/* Nir Oren's Liquid Tracker old "NO" format. I have only one NO module,
 * Moti Radomski's "Time after time" from ftp.modland.com.
 *
 * Another NO "Waste of Time" is bundled with The Liquid Tracker 0.80b, and
 * a converter called MOD2LIQ is bundled with The Liquid Player 1.00.
 */

//! The old Liquid Tracker (NO) loader.

use super::super::common::*;
use super::super::dataio::*;
use super::super::effects::*;
use super::super::hio::*;
use super::super::period::libxmp_c2spd_to_note;
use super::super::*;
use super::common::*;
use super::sample::libxmp_load_sample;
use super::*;

pub(crate) const LIBXMP_LOADER_NO: FormatLoader = FormatLoader {
    name: "Liquid Tracker NO",
    test: no_test,
    loader: no_load,
};

/// Translation of `no_test()`.
fn no_test(f: &mut HioHandle<'_>, t: Option<&mut [u8]>, start: i32) -> i32 {
    let mut buf = [0u8; 33];

    hio_seek(f, start as i64, SEEK_CUR);

    if hio_read32b(f) != 0x4e4f0000 {
        /* NO 0x00 0x00 */
        return -1;
    }

    if hio_read(&mut buf, 1, 33, f) < 33 {
        return -1;
    }

    let nsize = buf[0] as usize;
    if nsize > 29 {
        return -1;
    }

    /* test title */
    for i in 0..nsize {
        if buf[i + 1] == b'\0' {
            return -1;
        }
    }

    /* test number of patterns */
    let pat = buf[30];
    if pat == 0 {
        return -1;
    }

    /* test number of channels */
    let chn = buf[32];
    if chn == 0 || chn > 16 {
        return -1;
    }

    hio_seek(f, start as i64 + 5, SEEK_SET);

    libxmp_read_title(f, t, nsize as i32);

    0
}

/* Note that 0.80b or slightly earlier started translating these in the
 * editor to the new Liquid Module effects. It's not clear if the saving
 * format was modified at this point, but 0.80b and 0.82b are not aware
 * of the Liquid Module format.
 *
 * TODO: the changelog alleges an Fxx (05h) global volume effect was added
 * in 0.67b, which would be present in releases 0.68b and 0.69b only.
 */
static FX: [u8; 15] = [
    FX_SPEED,
    FX_VIBRATO,
    FX_BREAK,
    FX_PORTA_DN,
    FX_PORTA_UP,
    0,
    FX_ARPEGGIO,
    FX_SETPAN,
    0, /* special */
    FX_JUMP,
    FX_TREMOLO,
    FX_VOLSLIDE,
    0, /* special */
    FX_TONEPORTA,
    FX_OFFSET,
];

/* These are all FX_EXTENDED but in a custom order. */
static FX_MISC2: [u8; 16] = [
    EX_F_PORTA_UP as u8,
    EX_F_PORTA_DN as u8,
    EX_F_VSLIDE_UP as u8,
    EX_F_VSLIDE_DN as u8,
    EX_VIBRATO_WF as u8,
    EX_TREMOLO_WF as u8,
    EX_RETRIG as u8,
    EX_CUT as u8,
    EX_DELAY as u8,
    0,
    0,
    EX_PATTERN_LOOP as u8,
    EX_PATT_DELAY as u8,
    0,
    0,
    0,
];

/// Translation of `no_translate_effect()`.
fn no_translate_effect(event: &mut XmpEvent, fxt: i32, mut fxp: i32) {
    match fxt {
        0x0 /* Axx Set Speed/BPM */
        | 0x1 /* Bxy Vibrato */
        | 0x2 /* Cxx Cut (break) */
        | 0x3 /* Dxx Porta Down */
        | 0x4 /* Exx Porta Up */
        | 0x6 /* Gxx Arpeggio */
        | 0x9 /* Jxx Jump Position */
        | 0xa /* Kxy Tremolo */
        | 0xb /* Lxy Volslide (fine 0.80b+ only) */
        | 0xd /* Nxx Note Portamento */
        | 0xe /* Oxx Sample Offset */ => {
            event.fxt = FX[fxt as usize];
            event.fxp = fxp as u8;
        }

        0x7 => {
            /* Hxx Pan Control */
            /* Value is decimal, effective values >64 are ignored */
            let value = msn(fxp as u8) * 10 + lsn(fxp as u8);
            if value == 70 {
                /* TODO: reset panning H70 (H6A also works)
                 * this resets ALL channels to default pan. */
            } else if value <= 64 {
                event.fxt = FX_SETPAN;
                event.fxp = (value * 0xff / 64) as u8;
            }
        }

        0x8 => {
            /* Ixy Misc 1 */
            let l = lsn(fxp as u8) as u8;
            match msn(fxp as u8) {
                0x0 => {
                    /* I0y Vibrato + volslide up */
                    event.fxt = FX_VIBRA_VSLIDE;
                    event.fxp = l << 4;
                }
                0x1 => {
                    /* I1y Vibrato + volslide down */
                    event.fxt = FX_VIBRA_VSLIDE;
                    event.fxp = l;
                }
                0x2 => {
                    /* I2y Noteporta + volslide up */
                    event.fxt = FX_TONE_VSLIDE;
                    event.fxp = l << 4;
                }
                0x3 => {
                    /* I3y Noteporta + volslide down */
                    event.fxt = FX_TONE_VSLIDE;
                    event.fxp = l;
                }
                /* TODO: if these were ever implemented they were after 0.64b
                 * and before 0.80b only, i.e. versions not available to test. */
                0x4 => {
                    /* I4y Tremolo + volslide up */
                    event.fxt = FX_TREMOLO;
                    event.fxp = 0;
                    event.f2t = FX_VOLSLIDE;
                    event.f2p = l << 4;
                }
                0x5 => {
                    /* I5y Tremolo + volslide down */
                    event.fxt = FX_TREMOLO;
                    event.fxp = 0;
                    event.f2t = FX_VOLSLIDE;
                    event.f2p = l;
                }
                _ => {}
            }
        }

        0xc => {
            /* Mxy Misc 2 */
            let value = msn(fxp as u8);
            fxp = lsn(fxp as u8);
            match value {
                0x4 /* M4x Vibrato Waveform */
                | 0x5 /* M5x Tremolo Waveform */
                | 0x0 /* M0y Fine Porta Up */
                | 0x1 /* M1y Fine Porta Down */
                | 0x2 /* M2y Fine Volslide Up */
                | 0x3 /* M3y Fine Volslide Down */
                | 0x6 /* M6y Note Retrigger */
                | 0x7 /* M7y Note Cut */
                | 0x8 /* M8y Note Delay */
                | 0xb /* MBy Pattern Loop */
                | 0xc /* MCy Pattern Delay */ => {
                    if (value == 0x4 || value == 0x5) && (fxp & 3) == 3 {
                        fxp -= 1;
                    }
                    /* (4 and 5 fall through) */
                    event.fxt = FX_EXTENDED;
                    event.fxp = (((FX_MISC2[value as usize] as i32) << 4) | fxp) as u8;
                }
                _ => {}
            }
        }
        _ => {}
    }
}

/// Translation of `no_load()`.
fn no_load(m: &mut ModuleData, f: &mut HioHandle<'_>, _start: i32) -> i32 {
    let mut buf = [0u8; 46];

    hio_read32b(f); /* NO 0x00 0x00 */

    libxmp_set_type(m, b"Liquid Tracker");

    let mod_ = &mut m.mod_;
    let nsize = hio_read8(f) as usize;
    if hio_read(&mut mod_.name, 1, 29, f) < 29 {
        return -1;
    }
    /* (the test made sure the name fits) */
    if let Some(c) = mod_.name.get_mut(nsize) {
        *c = b'\0';
    }

    mod_.pat = hio_read8(f) as i32;
    hio_read8(f);
    mod_.chn = hio_read8(f) as i32;
    mod_.trk = mod_.pat * mod_.chn;
    hio_read8(f);
    hio_read16l(f);
    hio_read16l(f);
    hio_read8(f);
    mod_.ins = 63;
    mod_.smp = 63;

    let mut i = 0;
    while i < 256 {
        let x = hio_read8(f);
        if x == 0xff {
            break;
        }
        mod_.xxo[i] = x;
        i += 1;
    }
    hio_seek(f, 255 - i as i64, SEEK_CUR);
    mod_.len = i as i32;

    m.c4rate = C4_NTSC_RATE;

    if libxmp_init_instrument(m) < 0 {
        return -1;
    }

    let mod_ = &mut m.mod_;
    /* Read instrument names */
    for i in 0..mod_.ins {
        let iu = i as usize;

        if libxmp_alloc_subinstrument(mod_, i, 1) < 0 {
            return -1;
        }

        if hio_read(&mut buf, 1, 46, f) < 46 {
            return -1;
        }

        let nsize = (buf[0] as i32).min(30);
        libxmp_instrument_name(mod_, i, &buf[1..], nsize);

        mod_.xxi[iu].sub[0].vol = buf[31] as i32;
        let c2spd = readmem16l(&buf[32..]) as i32;
        let xxs = &mut mod_.xxs[iu];
        xxs.len = readmem32l(&buf[34..]) as i32;
        xxs.lps = readmem32l(&buf[38..]) as i32;
        xxs.lpe = readmem32l(&buf[42..]) as i32;

        if xxs.len > 0 {
            mod_.xxi[iu].nsm = 1;
        }

        let xxs = &mut mod_.xxs[iu];
        xxs.flg = if xxs.lpe > 0 { XMP_SAMPLE_LOOP } else { 0 };
        let sub = &mut mod_.xxi[iu].sub[0];
        sub.fin = 0;
        sub.pan = XMP_INST_NO_DEFAULT_PAN;
        sub.sid = i;

        libxmp_c2spd_to_note(c2spd, &mut sub.xpo, &mut sub.fin);
    }

    if libxmp_init_pattern(mod_) < 0 {
        return -1;
    }

    /* Read and convert patterns */

    for i in 0..mod_.pat {
        if libxmp_alloc_pattern_tracks(mod_, i, 64) < 0 {
            return -1;
        }

        for j in 0..mod_.pattern(i).rows {
            for k in 0..mod_.chn {
                let event = mod_.event_mut(i, k, j);

                let x = hio_read32l(f);
                let note = x & 0x0000003f;
                let ins = (x & 0x00001fc0) >> 6;
                let vol = (x & 0x000fe000) >> 13;
                let fxt = (x & 0x00f00000) >> 20;
                let fxp = (x & 0xff000000) >> 24;

                if note != 0x3f {
                    event.note = (37 + note) as u8;
                }
                if ins != 0x7f {
                    event.ins = (1 + ins) as u8;
                }
                if vol != 0x7f {
                    event.vol = (1 + vol) as u8;
                }
                if fxt != 0x0f {
                    no_translate_effect(event, fxt as i32, fxp as i32);
                }
            }
        }
    }

    /* Read samples */

    for i in 0..m.mod_.ins as usize {
        if m.mod_.xxs[i].len == 0 {
            continue;
        }
        let smpctl = m.smpctl;
        if libxmp_load_sample(smpctl, Some(f), SAMPLE_FLAG_UNS, &mut m.mod_.xxs[i], None) < 0 {
            return -1;
        }
    }

    m.quirk |= QUIRK_FINEFX | QUIRK_RTONCE;
    m.flow_mode = FLOW_MODE_LIQUID;
    m.read_event_type = READ_EVENT_ST3;

    for i in 0..m.mod_.chn {
        let pan = defpan(m, if i & 1 != 0 { 0xff } else { 0x00 });
        m.mod_.xxc[i as usize].pan = pan;
    }

    0
}
