// Rust translation of src/loaders/masi16_load.c from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2026 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Epic MegaGames MASI 16 (PSM, Protracker Studio 16) loader.

use super::super::common::*;
use super::super::dataio::*;
use super::super::effects::*;
use super::super::hio::*;
use super::super::period::libxmp_c2spd_to_note;
use super::super::*;
use super::common::*;
use super::sample::libxmp_load_sample;
use super::*;

const MAGIC_PSM_: u32 = magic4(b'P', b'S', b'M', 0xfe);

pub(crate) const LIBXMP_LOADER_MASI16: FormatLoader = FormatLoader {
    name: "Epic MegaGames MASI 16",
    test: masi16_test,
    loader: masi16_load,
};

/// Translation of `masi16_test()`.
fn masi16_test(f: &mut HioHandle<'_>, t: Option<&mut [u8]>, _start: i32) -> i32 {
    if hio_read32b(f) != MAGIC_PSM_ {
        return -1;
    }

    libxmp_read_title(f, t, 60);

    0
}

/// Translation of `masi16_translate_effect()`.
fn masi16_translate_effect(event: &mut XmpEvent, effect: u8, param: u8, param2: u8, _param3: u8) {
    match effect {
        1 => {
            /* Fine Volume Slide Up */
            event.fxt = FX_F_VSLIDE_UP;
            event.fxp = param;
        }
        2 => {
            /* Volume Slide Up */
            event.fxt = FX_VOLSLIDE_UP;
            event.fxp = param;
        }
        3 => {
            /* Fine Volume Slide Down */
            event.fxt = FX_F_VSLIDE_DN;
            event.fxp = param;
        }
        4 => {
            /* Volume Slide Down */
            event.fxt = FX_VOLSLIDE_DN;
            event.fxp = param;
        }

        10 => {
            /* Fine Porta Up */
            event.fxt = FX_F_PORTA_UP;
            event.fxp = param;
        }
        11 => {
            /* Portamento Up */
            event.fxt = FX_PORTA_UP;
            event.fxp = param;
        }
        12 => {
            /* Fine Porta Down */
            event.fxt = FX_F_PORTA_DN;
            event.fxp = param;
        }
        13 => {
            /* Portamento Down */
            event.fxt = FX_PORTA_DN;
            event.fxp = param;
        }
        14 => {
            /* Tone Portamento */
            event.fxt = FX_TONEPORTA;
            event.fxp = param;
        }
        15 => {
            /* Glissando control */
            event.fxt = FX_EXTENDED;
            event.fxp = ((EX_GLISS << 4) as u8) | (param & 0x0f);
        }
        16 => {
            /* Tone Portamento + Volslide Up */
            event.fxt = FX_TONEPORTA;
            event.fxp = 0;
            event.f2t = FX_VOLSLIDE_UP;
            event.f2p = param;
        }
        17 => {
            /* Tone Portamento + Volslide Down */
            event.fxt = FX_TONEPORTA;
            event.fxp = 0;
            event.f2t = FX_VOLSLIDE_DN;
            event.f2p = param;
        }

        20 => {
            /* Vibrato */
            event.fxt = FX_VIBRATO;
            event.fxp = param;
        }
        21 => {
            /* Vibrato waveform */
            event.fxt = FX_EXTENDED;
            event.fxp = ((EX_VIBRATO_WF << 4) as u8) | (param & 0x0f);
        }
        22 => {
            /* Vibrato + Volume Slide Up */
            event.fxt = FX_VIBRATO;
            event.fxp = 0;
            event.f2t = FX_VOLSLIDE_UP;
            event.f2p = param;
        }
        23 => {
            /* Vibrato + Volume Slide Down */
            event.fxt = FX_VIBRATO;
            event.fxp = 0;
            event.f2t = FX_VOLSLIDE_DN;
            event.f2p = param;
        }

        30 => {
            /* Tremolo */
            event.fxt = FX_TREMOLO;
            event.fxp = param;
        }
        31 => {
            /* Tremolo waveform */
            event.fxt = FX_EXTENDED;
            event.fxp = ((EX_TREMOLO_WF << 4) as u8) | (param & 0x0f);
        }

        40 => {
            /* Sample Offset */
            /* TODO: param and param3 are the fine and high offsets. */
            event.fxt = FX_OFFSET;
            event.fxp = param2;
        }
        41 => {
            /* Retrigger Note */
            event.fxt = FX_EXTENDED;
            event.fxp = ((EX_RETRIG << 4) as u8) | (param & 0x0f);
        }
        42 => {
            /* Note Cut */
            event.fxt = FX_EXTENDED;
            event.fxp = ((EX_CUT << 4) as u8) | (param & 0x0f);
        }
        43 => {
            /* Note Delay */
            event.fxt = FX_EXTENDED;
            event.fxp = ((EX_DELAY << 4) as u8) | (param & 0x0f);
        }

        50 => {
            /* Position Jump */
            event.fxt = FX_JUMP;
            event.fxp = param;
        }
        51 => {
            /* Pattern Break */
            event.fxt = FX_BREAK;
            event.fxp = param;
        }
        52 => {
            /* Jump Loop */
            event.fxt = FX_EXTENDED;
            event.fxp = ((EX_PATTERN_LOOP << 4) as u8) | (param & 0x0f);
        }
        53 => {
            /* Pattern Delay */
            event.fxt = FX_PATT_DELAY;
            event.fxp = param;
        }

        60 => {
            /* Set Speed */
            event.fxt = FX_S3M_SPEED;
            event.fxp = param;
        }
        61 => {
            /* Set BPM */
            event.fxt = FX_S3M_BPM;
            event.fxp = param;
        }

        70 => {
            /* Arpeggio */
            event.fxt = FX_ARPEGGIO;
            event.fxp = param;
        }
        71 => {
            /* Set Finetune */
            event.fxt = FX_FINETUNE;
            event.fxp = param;
        }
        72 => {
            /* Set Balance */
            event.fxt = FX_SETPAN;
            event.fxp = (param & 0x0f) | ((param & 0x0f) << 4);
        }

        _ => {
            event.fxt = 0;
            event.fxp = 0;
        }
    }
}

/// Translation of `masi16_load()`.
fn masi16_load(m: &mut ModuleData, f: &mut HioHandle<'_>, start: i32) -> i32 {
    let mut buf = [0u8; 1024];
    let mut p_smp = [0u32; 256];
    let mut sample_map = [0u8; 256];
    /* (the offsets are added to the start as unsigned 32 bit numbers) */
    let at = |p: u32| (start as u32).wrapping_add(p) as i64;

    hio_read32b(f);

    hio_read(&mut buf, 1, 60, f);
    m.mod_.name[..59].copy_from_slice(&buf[..59]);
    m.mod_.name[59] = b'\0';

    let type_ = hio_read8(f); /* song type */
    let ver = hio_read8(f); /* song version */
    /*mode =*/
    hio_read8(f); /* pattern version */

    if type_ & 0x01 != 0 {
        /* song mode not supported */
        return -1;
    }

    let s = format!("Epic MegaGames MASI 16 PSM {}.{:02}", msn(ver), lsn(ver));
    libxmp_set_type(m, s.as_bytes());

    let mod_ = &mut m.mod_;
    mod_.spd = hio_read8(f) as i32;
    mod_.bpm = hio_read8(f) as i32;
    hio_read8(f); /* master volume */
    hio_read16l(f); /* song length */
    mod_.len = hio_read16l(f) as i32;
    mod_.pat = hio_read16l(f) as i32;
    let stored_ins = hio_read16l(f) as i32;
    hio_read16l(f); /* ignore channels to play */
    mod_.chn = hio_read16l(f) as i32; /* use channels to proceed */

    /* Sanity check */
    if mod_.len > 256 || mod_.pat > 256 || stored_ins > 255 || mod_.chn > XMP_MAX_CHANNELS as i32 {
        return -1;
    }
    mod_.trk = mod_.pat * mod_.chn;

    let p_ord = hio_read32l(f);
    let p_chn = hio_read32l(f);
    let p_pat = hio_read32l(f);
    let p_ins = hio_read32l(f);

    /* should be this way but fails with Silverball song 6 */
    //mod->flg |= ~type & 0x02 ? XXM_FLG_MODRNG : 0;

    m.c4rate = C4_NTSC_RATE;

    let mod_ = &mut m.mod_;
    hio_seek(f, at(p_ord), SEEK_SET);
    let len = mod_.len as usize;
    hio_read(&mut mod_.xxo, 1, len, f);

    buf[..mod_.chn as usize].fill(0);
    hio_seek(f, at(p_chn), SEEK_SET);
    hio_read(&mut buf, 1, 16, f);

    for i in 0..mod_.chn as usize {
        if buf[i] < 16 {
            mod_.xxc[i].pan = (buf[i] | (buf[i] << 4)) as i32;
        }
    }

    /* Get the actual instruments count... */
    mod_.ins = 0;
    for i in 0..stored_ins as usize {
        hio_seek(
            f,
            at(p_ins.wrapping_add(64 * i as u32).wrapping_add(45)),
            SEEK_SET,
        );
        sample_map[i] = hio_read16l(f).wrapping_sub(1) as u8;
        mod_.ins = mod_.ins.max(sample_map[i] as i32 + 1);
    }
    if mod_.ins > 255 || hio_error(f) != 0 {
        return -1;
    }

    mod_.smp = mod_.ins;

    if libxmp_init_instrument(m) < 0 {
        return -1;
    }

    p_smp.fill(0);

    let mod_ = &mut m.mod_;
    hio_seek(f, at(p_ins), SEEK_SET);
    for i in 0..stored_ins as usize {
        let num = sample_map[i] as usize;

        if hio_read(&mut buf, 1, 64, f) < 64 {
            return -1;
        }

        /* Don't load duplicate instruments */
        if !mod_.xxi[num].sub.is_empty() {
            continue;
        }

        if libxmp_alloc_subinstrument(mod_, num as i32, 1) < 0 {
            return -1;
        }

        let xxi = &mut mod_.xxi[num];
        let xxs = &mut mod_.xxs[num];
        let sub = &mut xxi.sub[0];

        /* hio_read(buf, 1, 13, f); (sample filename) */
        /* hio_read(buf, 1, 24, f); (sample description) */
        xxi.name[..24].copy_from_slice(&buf[13..37]);
        xxi.name[24] = b'\0';
        p_smp[i] = readmem32l(&buf[37..]);
        /* hio_read32l(f); (memory location) */
        /* hio_read16l(f); (sample number) */
        let flags = buf[47]; /* sample type */
        xxs.len = readmem32l(&buf[48..]) as i32;
        xxs.lps = readmem32l(&buf[52..]) as i32;
        xxs.lpe = readmem32l(&buf[56..]) as i32;
        let finetune = buf[60] as i32;
        sub.vol = buf[61] as i32;
        let c2spd = readmem16l(&buf[62..]);
        sub.pan = XMP_INST_NO_DEFAULT_PAN;
        sub.sid = num as i32;
        xxs.flg = if flags & 0x80 != 0 {
            XMP_SAMPLE_LOOP
        } else {
            0
        };
        xxs.flg |= if flags & 0x20 != 0 {
            XMP_SAMPLE_LOOP_BIDIR
        } else {
            0
        };

        libxmp_c2spd_to_note(c2spd as i32, &mut sub.xpo, &mut sub.fin);
        sub.fin += ((finetune & 0x0f) << 4) as i8 as i32;
        sub.xpo += (finetune >> 4) - 7;

        /* The documentation claims samples shouldn't exceed 64k. The
         * PS16 modules from Silverball and Epic Pinball confirm this.
         * Later Protracker Studio Modules (MASI) allow up to 1MB.
         */
        if xxs.len as u32 > 64 * 1024 {
            return -1;
        }

        if xxs.len > 0 {
            xxi.nsm = 1;
        }
    }

    if libxmp_init_pattern(mod_) < 0 {
        return -1;
    }

    hio_seek(f, at(p_pat), SEEK_SET);
    for i in 0..mod_.pat {
        let mut len = hio_read16l(f) as i32 - 4;
        let rows = hio_read8(f);
        if rows > 64 {
            return -1;
        }
        let chan = hio_read8(f);
        if chan > 32 {
            return -1;
        }

        if libxmp_alloc_pattern_tracks(mod_, i, rows as i32) < 0 {
            return -1;
        }

        for r in 0..rows as i32 {
            while len > 0 {
                let b = hio_read8(f);
                len -= 1;

                if b == 0 {
                    break;
                }

                let c = (b & 0x0f) as i32;
                if c >= mod_.chn {
                    return -1;
                }
                let event = mod_.event_mut(i, c, r);

                if b & 0x80 != 0 {
                    event.note = hio_read8(f).wrapping_add(36);
                    event.ins = hio_read8(f);
                    len -= 2;
                }

                if b & 0x40 != 0 {
                    event.vol = hio_read8(f).wrapping_add(1);
                    len -= 1;
                }

                if b & 0x20 != 0 {
                    let effect = hio_read8(f);
                    let param = hio_read8(f);
                    let mut param2 = 0;
                    let mut param3 = 0;

                    if effect == 40 {
                        /* Sample Offset */
                        // Note (upstream): C doesn't count these two bytes
                        // in what's left of the pattern.
                        param2 = hio_read8(f);
                        param3 = hio_read8(f);
                        len -= 2;
                    }
                    masi16_translate_effect(event, effect, param, param2, param3);
                    len -= 2;
                }
            }
        }

        if len > 0 {
            hio_seek(f, len as i64, SEEK_CUR);
        }
    }

    /* Read samples */

    for i in 0..stored_ins as usize {
        let num = sample_map[i] as usize;

        /* Don't load duplicate sample data */
        if m.mod_.xxs[num].data.is_some() {
            continue;
        }

        hio_seek(f, at(p_smp[i]), SEEK_SET);
        let smpctl = m.smpctl;
        if libxmp_load_sample(
            smpctl,
            Some(f),
            SAMPLE_FLAG_DIFF,
            &mut m.mod_.xxs[num],
            None,
        ) < 0
        {
            return -1;
        }
    }

    0
}
