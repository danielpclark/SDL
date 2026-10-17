// Rust translation of src/loaders/ult_load.c from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2026 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

/* Based on the format description by FreeJack of The Elven Nation
 *
 * The loader recognizes four subformats:
 * - MAS_UTrack_V001: Ultra Tracker version < 1.4
 * - MAS_UTrack_V002: Ultra Tracker version 1.4
 * - MAS_UTrack_V003: Ultra Tracker version 1.5
 * - MAS_UTrack_V004: Ultra Tracker version 1.6
 */

//! The Ultra Tracker (ULT) loader.

use super::super::common::*;
use super::super::effects::*;
use super::super::hio::*;
use super::super::period::libxmp_c2spd_to_note;
use super::super::*;
use super::common::*;
use super::sample::libxmp_load_sample;
use super::*;

pub(crate) const LIBXMP_LOADER_ULT: FormatLoader = FormatLoader {
    name: "Ultra Tracker",
    test: ult_test,
    loader: ult_load,
};

/// Translation of `ult_test()`.
fn ult_test(f: &mut HioHandle<'_>, t: Option<&mut [u8]>, _start: i32) -> i32 {
    let mut buf = [0u8; 15];

    if hio_read(&mut buf, 1, 15, f) < 15 {
        return -1;
    }

    if &buf[..14] != b"MAS_UTrack_V00" {
        return -1;
    }

    if buf[14] < b'1' || buf[14] > b'4' {
        return -1;
    }

    libxmp_read_title(f, t, 32);

    0
}

/// Translation of `struct ult_header`.
#[derive(Clone, Copy, Default)]
struct UltHeader {
    magic: [u8; 15], /* 'MAS_UTrack_V00x' */
    name: [u8; 32],  /* Song name */
    msgsize: u8,     /* ver < 1.4: zero */
}

/// Translation of `struct ult_header2`.
#[derive(Clone, Copy)]
struct UltHeader2 {
    order: [u8; 256], /* Orders */
    channels: u8,     /* Number of channels - 1 */
    patterns: u8,     /* Number of patterns - 1 */
}

/// Translation of `struct ult_instrument`.
#[derive(Clone, Copy, Default)]
struct UltInstrument {
    name: [u8; 32],    /* Instrument name */
    dosname: [u8; 12], /* DOS file name */
    loop_start: u32,   /* Loop start */
    loopend: u32,      /* Loop end */
    sizestart: u32,    /* Sample size is sizeend - sizestart */
    sizeend: u32,
    volume: u8,    /* Volume (log; ver >= 1.4 linear) */
    bidiloop: u8,  /* Sample loop flags */
    finetune: i16, /* Finetune */
    c2spd: u16,    /* C2 frequency */
}

/// Translation of `struct ult_event` (read as its four bytes).
#[derive(Clone, Copy, Default)]
struct UltEvent {
    /* uint8 note; */
    ins: u8,
    fxt: u8, /* MSN = fxt, LSN = f2t */
    f2p: u8, /* Secondary comes first -- little endian! */
    fxp: u8,
}

/// Translation of `ult_translate_effect()`.
fn ult_translate_effect(fxt: &mut u8, fxp: &mut u8) {
    match *fxt {
        0x03 => {
            /* Tone portamento */
            *fxt = FX_ULT_TPORTA;
        }
        0x05 /* 'Special' effect */ | 0x06 /* Reserved */ => {
            *fxt = 0;
            *fxp = 0;
        }
        0x0b => {
            /* Pan */
            *fxt = FX_SETPAN;
            *fxp <<= 4;
        }
        0x09 => {
            /* Sample offset */
            /* TODO: fine sample offset (requires new effect or 2 more effect lanes) */
            *fxp <<= 2;
        }
        0x0f => {
            /* Speed/BPM */
            /* 00:    default speed (6)/BPM (125)
             * 01-2f: set speed
             * 30-ff: set BPM
             */
            *fxt = FX_ULT_TEMPO;
        }
        _ => {}
    }
}

/// Translation of `ult_load()`.
fn ult_load(m: &mut ModuleData, f: &mut HioHandle<'_>, _start: i32) -> i32 {
    let mut ufh = UltHeader::default();
    let mut ufh2 = UltHeader2 {
        order: [0; 256],
        channels: 0,
        patterns: 0,
    };
    let mut uih = UltInstrument::default();
    let mut ue = UltEvent::default();
    const VERSTR: [&str; 4] = ["< 1.4", "1.4", "1.5", "1.6"];

    hio_read(&mut ufh.magic, 15, 1, f);
    hio_read(&mut ufh.name, 32, 1, f);
    ufh.msgsize = hio_read8(f);

    let ver = ufh.magic[14] as i32 - b'0' as i32;
    /* (the test made sure of the version) */
    let Some(verstr) = VERSTR.get((ver - 1) as usize) else {
        return -1;
    };

    strncpy(&mut m.mod_.name, &ufh.name, 32);
    m.mod_.name[32] = b'\0';
    let s = format!("Ultra Tracker {} ULT V{:03}", verstr, ver);
    libxmp_set_type(m, s.as_bytes());

    m.c4rate = C4_NTSC_RATE;

    if ufh.msgsize > 0 {
        if let Ok(mut comment) = try_vec(ufh.msgsize as usize * 33, 0u8) {
            let mut pos = 0;
            for _ in 0..ufh.msgsize {
                if hio_read(&mut comment[pos..], 1, 32, f) < 32 {
                    m.comment = Some(comment);
                    return -1;
                }
                comment[pos + 32] = b'\n';
                pos += 33;
            }
            pos -= 1;
            comment[pos] = b'\0';
            m.comment = Some(comment);
        } else {
            hio_seek(f, ufh.msgsize as i64 * 32, SEEK_CUR);
        }
    }

    m.mod_.smp = hio_read8(f) as i32;
    m.mod_.ins = m.mod_.smp;
    /* mod->flg |= XXM_FLG_LINEAR; */

    /* Read and convert instruments */

    if libxmp_init_instrument(m) < 0 {
        return -1;
    }

    let mod_ = &mut m.mod_;
    for i in 0..mod_.ins {
        let iu = i as usize;
        if libxmp_alloc_subinstrument(mod_, i, 1) < 0 {
            return -1;
        }

        hio_read(&mut uih.name, 32, 1, f);
        hio_read(&mut uih.dosname, 12, 1, f);
        uih.loop_start = hio_read32l(f);
        uih.loopend = hio_read32l(f);
        uih.sizestart = hio_read32l(f);
        uih.sizeend = hio_read32l(f);
        uih.volume = hio_read8(f);
        uih.bidiloop = hio_read8(f);
        uih.c2spd = if ver >= 4 { hio_read16l(f) } else { 0 }; /* Incorrect in ult_form.txt */
        uih.finetune = hio_read16l(f) as i16;
        if hio_error(f) != 0 {
            return -1;
        }

        /* Sanity check:
         * "[SizeStart] seems to tell UT how to load the sample into the GUS's
         * onboard memory." The maximum supported GUS RAM is 16 MB (PnP).
         * Samples also can't cross 256k boundaries. In practice it seems like
         * nothing ever goes over 1 MB, the maximum on most GUS cards.
         */
        if uih.sizestart > uih.sizeend
            || uih.sizeend > (16 << 20)
            || uih.sizeend - uih.sizestart > (256 << 10)
        {
            return -1;
        }

        let xxs = &mut mod_.xxs[iu];
        xxs.len = (uih.sizeend - uih.sizestart) as i32;
        xxs.lps = uih.loop_start as i32;
        xxs.lpe = uih.loopend as i32;

        if xxs.len > 0 {
            mod_.xxi[iu].nsm = 1;
        }

        /* BiDi Loop : (Bidirectional Loop)
         *
         * UT takes advantage of the Gus's ability to loop a sample in
         * several different ways. By setting the Bidi Loop, the sample can
         * be played forward or backwards, looped or not looped. The Bidi
         * variable also tracks the sample resolution (8 or 16 bit).
         *
         * The following table shows the possible values of the Bidi Loop.
         * Bidi = 0  : No looping, forward playback,  8bit sample
         * Bidi = 4  : No Looping, forward playback, 16bit sample
         * Bidi = 8  : Loop Sample, forward playback, 8bit sample
         * Bidi = 12 : Loop Sample, forward playback, 16bit sample
         * Bidi = 24 : Loop Sample, reverse playback 8bit sample
         * Bidi = 28 : Loop Sample, reverse playback, 16bit sample
         */

        /* Claudio's note: I'm ignoring reverse playback for samples */

        let xxs = &mut mod_.xxs[iu];
        match uih.bidiloop {
            20 /* Type 20 is in seasons.ult */ | 4 => {
                xxs.flg = XMP_SAMPLE_16BIT;
            }
            8 => {
                xxs.flg = XMP_SAMPLE_LOOP;
            }
            12 => {
                xxs.flg = XMP_SAMPLE_16BIT | XMP_SAMPLE_LOOP;
            }
            24 => {
                xxs.flg = XMP_SAMPLE_LOOP | XMP_SAMPLE_LOOP_REVERSE;
            }
            28 => {
                xxs.flg = XMP_SAMPLE_16BIT | XMP_SAMPLE_LOOP | XMP_SAMPLE_LOOP_REVERSE;
            }
            _ => {}
        }

        /* TODO: Add logarithmic volume support */
        let sub = &mut mod_.xxi[iu].sub[0];
        sub.vol = uih.volume as i32;
        sub.pan = XMP_INST_NO_DEFAULT_PAN;
        sub.sid = i;

        libxmp_instrument_name(mod_, i, &uih.name, 24);

        if ver > 3 {
            let sub = &mut mod_.xxi[iu].sub[0];
            libxmp_c2spd_to_note(uih.c2spd as i32, &mut sub.xpo, &mut sub.fin);
        }
    }

    hio_read(&mut ufh2.order, 256, 1, f);
    ufh2.channels = hio_read8(f);
    ufh2.patterns = hio_read8(f);

    if hio_error(f) != 0 {
        return -1;
    }

    let mut i = 0;
    while i < 256 {
        if ufh2.order[i] == 0xff {
            break;
        }
        mod_.xxo[i] = ufh2.order[i];
        i += 1;
    }
    mod_.len = i as i32;
    mod_.chn = ufh2.channels as i32 + 1;
    mod_.pat = ufh2.patterns as i32 + 1;
    mod_.spd = 6;
    mod_.bpm = 125;
    mod_.trk = mod_.chn * mod_.pat;

    /* Sanity check */
    if mod_.chn > XMP_MAX_CHANNELS as i32 {
        return -1;
    }

    for i in 0..mod_.chn {
        if ver >= 3 {
            let x8 = hio_read8(f) as i32;
            m.mod_.xxc[i as usize].pan = 255 * x8 / 15;
        } else {
            let pan = defpan(m, (((i + 1) / 2) % 2) * 0xff); /* ??? */
            m.mod_.xxc[i as usize].pan = pan;
        }
    }

    let mod_ = &mut m.mod_;
    if libxmp_init_pattern(mod_) < 0 {
        return -1;
    }

    /* Read and convert patterns */

    /* Events are stored by channel */
    for i in 0..mod_.pat {
        if libxmp_alloc_pattern_tracks(mod_, i, 64) < 0 {
            return -1;
        }
    }

    for i in 0..mod_.chn {
        let mut j = 0;
        while j < 64 * mod_.pat {
            let mut cnt = 1;
            let mut x8 = hio_read8(f); /* Read note or repeat code (0xfc) */
            if x8 == 0xfc {
                cnt = hio_read8(f) as i32; /* Read repeat count */
                x8 = hio_read8(f); /* Read note */
            }
            let mut buf = [0u8; 4];
            if hio_read(&mut buf, 1, 4, f) < 4 {
                /* Read rest of the event */
                return -1;
            }
            ue.ins = buf[0];
            ue.fxt = buf[1];
            ue.f2p = buf[2];
            ue.fxp = buf[3];

            if cnt == 0 {
                cnt += 1;
            }

            if j + cnt > 64 * mod_.pat {
                return -1;
            }

            for _ in 0..cnt {
                let event = mod_.event_mut(j >> 6, i, j & 0x3f);
                *event = XmpEvent::default();
                if x8 != 0 {
                    event.note = x8.wrapping_add(36);
                }
                event.ins = ue.ins;
                event.fxt = msn(ue.fxt) as u8;
                event.f2t = lsn(ue.fxt) as u8;
                event.fxp = ue.fxp;
                event.f2p = ue.f2p;

                ult_translate_effect(&mut event.fxt, &mut event.fxp);
                ult_translate_effect(&mut event.f2t, &mut event.f2p);
                j += 1;
            }
        }
    }

    for i in 0..m.mod_.ins as usize {
        if m.mod_.xxs[i].len == 0 {
            continue;
        }
        let smpctl = m.smpctl;
        if libxmp_load_sample(smpctl, Some(f), 0, &mut m.mod_.xxs[i], None) < 0 {
            return -1;
        }
    }

    m.volbase = 0x100;

    0
}
