// Rust translation of src/loaders/sfx_load.c from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2026 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

/* Reverse engineered from the two SFX files in the Delitracker mods disk
 * and music from Future Wars, Twinworld and Operation Stealth. Effects
 * must be verified/implemented.
 */

/* From the ExoticRipper docs:
 * [SoundFX 2.0 is] simply the same as SoundFX 1.3, except that it
 * uses 31 samples [instead of 15].
 */

//! The SoundFX v1.3/2.0 loader.

use super::super::common::*;
use super::super::effects::*;
use super::super::hio::*;
use super::super::period::libxmp_period_to_note;
use super::super::*;
use super::common::*;
use super::sample::libxmp_load_sample;
use super::*;

const MAGIC_SONG: u32 = magic4(b'S', b'O', b'N', b'G');

pub(crate) const LIBXMP_LOADER_SFX: FormatLoader = FormatLoader {
    name: "SoundFX v1.3/2.0",
    test: sfx_test,
    loader: sfx_load,
};

/// Translation of `sfx_test()`.
fn sfx_test(f: &mut HioHandle<'_>, t: Option<&mut [u8]>, _start: i32) -> i32 {
    hio_seek(f, 4 * 15, SEEK_CUR);
    let a = hio_read32b(f);
    hio_seek(f, 4 * 15, SEEK_CUR);
    let b = hio_read32b(f);

    if a != MAGIC_SONG && b != MAGIC_SONG {
        return -1;
    }

    libxmp_read_title(f, t, 0);

    0
}

/// Translation of `struct sfx_ins`.
#[derive(Clone, Copy, Default)]
struct SfxIns {
    name: [u8; 22],   /* Instrument name */
    len: u16,         /* Sample length in words */
    finetune: u8,     /* Finetune */
    volume: u8,       /* Volume (0-63) */
    loop_start: u16,  /* Sample loop start in bytes */
    loop_length: u16, /* Sample loop length in words */
}

/// Translation of `struct sfx_header`.
struct SfxHeader {
    magic: u32,        /* 'SONG' */
    delay: u16,        /* Delay value (tempo), default is 0x38e5 */
    unknown: [u8; 14], /* ? */
}

/// Translation of `struct sfx_header2`.
struct SfxHeader2 {
    len: u8,          /* Song length */
    restart: u8,      /* Restart pos (?) */
    order: [u8; 128], /* Order list */
}

/// Translation of `sfx_translate_effect()`.
fn sfx_translate_effect(event: &mut XmpEvent, fxt: i32, fxp: i32) {
    event.fxp = fxp as u8;

    match fxt {
        0x01 => {
            /* Arpeggio */
            event.fxt = FX_ARPEGGIO;
        }
        0x02 => {
            /* Pitch bend */
            if event.fxp >> 4 != 0 {
                event.fxt = FX_PORTA_DN;
                event.fxp >>= 4;
            } else if event.fxp & 0x0f != 0 {
                event.fxt = FX_PORTA_UP;
                event.fxp &= 0x0f;
            }
        }
        0x5 => {
            /* Add to volume */
            event.fxt = FX_VOL_ADD;
        }
        0x6 => {
            /* Subtract from volume */
            event.fxt = FX_VOL_SUB;
        }
        0x7 => {
            /* Add semitones to period */
            event.fxt = FX_PITCH_ADD;
        }
        0x8 => {
            /* Subtract semitones from period */
            event.fxt = FX_PITCH_SUB;
        }
        /* 0x3: LED on */
        /* 0x4: LED off */
        _ => {
            event.fxt = 0;
            event.fxp = 0;
        }
    }
}

/// Translation of `sfx_13_20_load()`.
fn sfx_13_20_load(m: &mut ModuleData, f: &mut HioHandle<'_>, nins: i32, _start: i32) -> i32 {
    let mut ev = [0u8; 4];
    let mut ins_size = [0i32; 31];
    let mut ins = [SfxIns::default(); 31]; /* Instruments */
    let mut sfx = SfxHeader {
        magic: 0,
        delay: 0,
        unknown: [0; 14],
    };
    let mut sfx2 = SfxHeader2 {
        len: 0,
        restart: 0,
        order: [0; 128],
    };

    for size in ins_size.iter_mut().take(nins as usize) {
        *size = hio_read32b(f) as i32;
    }

    sfx.magic = hio_read32b(f);
    sfx.delay = hio_read16b(f);
    if sfx.delay < 178 {
        /* min value for 10000bpm */
        return -1;
    }

    hio_read(&mut sfx.unknown, 14, 1, f);

    if sfx.magic != MAGIC_SONG {
        return -1;
    }

    let mod_ = &mut m.mod_;
    mod_.chn = 4;
    mod_.ins = nins;
    mod_.smp = mod_.ins;
    mod_.bpm = 14565 * 122 / sfx.delay as i32;

    for item in ins.iter_mut().take(mod_.ins as usize) {
        hio_read(&mut item.name, 22, 1, f);
        item.len = hio_read16b(f);
        item.finetune = hio_read8(f);
        item.volume = hio_read8(f);
        item.loop_start = hio_read16b(f);
        item.loop_length = hio_read16b(f);
    }

    sfx2.len = hio_read8(f);
    sfx2.restart = hio_read8(f);
    let _ = sfx2.restart;
    if hio_read(&mut sfx2.order, 1, 128, f) != 128 {
        return -1;
    }

    mod_.len = sfx2.len as i32;
    if mod_.len > 0x7f {
        return -1;
    }

    let len = mod_.len as usize;
    mod_.xxo[..len].copy_from_slice(&sfx2.order[..len]);
    mod_.pat = 0;
    for i in 0..len {
        if mod_.xxo[i] as i32 > mod_.pat {
            mod_.pat = mod_.xxo[i] as i32;
        }
    }
    mod_.pat += 1;

    mod_.trk = mod_.chn * mod_.pat;

    if mod_.ins == 15 {
        libxmp_set_type(m, b"SoundFX 1.3");
    } else {
        libxmp_set_type(m, b"SoundFX 2.0");
    }

    if libxmp_init_instrument(m) < 0 {
        return -1;
    }

    let mod_ = &mut m.mod_;
    for i in 0..mod_.ins {
        let iu = i as usize;

        if libxmp_alloc_subinstrument(mod_, i, 1) < 0 {
            return -1;
        }

        let xxi = &mut mod_.xxi[iu];
        let xxs = &mut mod_.xxs[iu];
        let sub = &mut xxi.sub[0];

        xxs.len = ins_size[iu];
        xxs.lps = ins[iu].loop_start as i32;
        xxs.lpe = xxs.lps + 2 * ins[iu].loop_length as i32;
        xxs.flg = if ins[iu].loop_length > 1 {
            XMP_SAMPLE_LOOP
        } else {
            0
        };
        xxi.nsm = 1;
        sub.vol = ins[iu].volume as i32;
        sub.fin = (ins[iu].finetune << 4) as i8 as i32; /* unsure */
        sub.pan = XMP_INST_NO_DEFAULT_PAN;
        sub.sid = i;

        let _ = ins[iu].len;
        libxmp_instrument_name(mod_, i, &ins[iu].name, 22);
    }

    if libxmp_init_pattern(mod_) < 0 {
        return -1;
    }

    for i in 0..mod_.pat {
        if libxmp_alloc_pattern_tracks(mod_, i, 64) < 0 {
            return -1;
        }

        for j in 0..64 {
            for k in 0..mod_.chn {
                let event = mod_.event_mut(i, k, j);
                if hio_read(&mut ev, 1, 4, f) < 4 {
                    return -1;
                }

                event.note = libxmp_period_to_note((lsn(ev[0]) << 8) | ev[1] as i32) as u8;
                event.ins = ((msn(ev[0]) << 4) | msn(ev[2])) as u8;

                sfx_translate_effect(event, lsn(ev[2]), ev[3] as i32);
            }
        }
    }

    m.quirk |= QUIRK_PBALL;
    m.period_type = PERIOD_MODRNG;

    /* Read samples */

    for i in 0..m.mod_.ins as usize {
        if m.mod_.xxs[i].len <= 2 {
            continue;
        }
        let smpctl = m.smpctl;
        if libxmp_load_sample(smpctl, Some(f), 0, &mut m.mod_.xxs[i], None) < 0 {
            return -1;
        }
    }

    0
}

/// Translation of `sfx_load()`.
fn sfx_load(m: &mut ModuleData, f: &mut HioHandle<'_>, start: i32) -> i32 {
    if sfx_13_20_load(m, f, 15, start) < 0 {
        /* Note (upstream): C's 31 instrument attempt reads on from
         * wherever the 15 instrument attempt stopped, instead of from the
         * start of the module */
        hio_seek(f, start as i64, SEEK_SET);
        return sfx_13_20_load(m, f, 31, start);
    }

    0
}
