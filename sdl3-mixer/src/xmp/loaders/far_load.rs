// Rust translation of src/loaders/far_load.c from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2026 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

/* Based on the Farandole Composer format specifications by Megan Potter.
 *
 * "(...) this format is for EDITING purposes (storing EVERYTHING you're
 * working on) so it may include information not completely neccessary."
 */

//! The Farandole Composer loader.

use super::super::common::*;
use super::super::effects::*;
use super::super::far_extras::*;
use super::super::hio::*;
use super::super::*;
use super::common::*;
use super::sample::libxmp_load_sample;
use super::*;

/// Translation of `struct far_header`.
struct FarHeader {
    magic: u32,      /* File magic: 'FAR\xfe' */
    name: [u8; 40],  /* Song name */
    crlf: [u8; 3],   /* 0x0d 0x0a 0x1A */
    headersize: u16, /* Remaining header size in bytes */
    version: u8,     /* Version MSN=major, LSN=minor */
    ch_on: [u8; 16], /* Channel on/off switches */
    rsvd1: [u8; 9],  /* Current editing values */
    tempo: u8,       /* Default tempo */
    pan: [u8; 16],   /* Channel pan definitions */
    rsvd2: [u8; 4],  /* Grid, mode (for editor) */
    textlen: u16,    /* Length of embedded text */
}

/// Translation of `struct far_header2`.
struct FarHeader2 {
    order: [u8; 256],    /* Orders */
    patterns: u8,        /* Number of stored patterns (?) */
    songlen: u8,         /* Song length in patterns */
    restart: u8,         /* Restart pos */
    patsize: [u16; 256], /* Size of each pattern in bytes */
}

/// Translation of `struct far_instrument`.
#[derive(Default)]
struct FarInstrument {
    name: [u8; 32],  /* Instrument name */
    length: u32,     /* Length of sample (up to 64Kb) */
    finetune: u8,    /* Finetune (unsupported) */
    volume: u8,      /* Volume (unsupported?) */
    loop_start: u32, /* Loop start */
    loopend: u32,    /* Loop end */
    sampletype: u8,  /* 1=16 bit sample */
    loopmode: u8,
}

const MAGIC_FAR: u32 = magic4(b'F', b'A', b'R', 0xfe);

pub(crate) const LIBXMP_LOADER_FAR: FormatLoader = FormatLoader {
    name: "Farandole Composer",
    test: far_test,
    loader: far_load,
};

/// Translation of `far_test()`.
fn far_test(f: &mut HioHandle<'_>, t: Option<&mut [u8]>, _start: i32) -> i32 {
    if hio_read32b(f) != MAGIC_FAR {
        return -1;
    }

    libxmp_read_title(f, t, 40);

    0
}

/// Translation of `far_translate_effect()`.
fn far_translate_effect(event: &mut XmpEvent, fx: i32, param: i32, vol: i32) {
    match fx {
        0x0 => {
            /* 0x0?  Global funct */
            match param {
                0x1 | 0x2 => {
                    /* 0x01  Ramp delay on */
                    /* 0x02  Ramp delay off */
                    /* These control volume ramping and can be ignored. */
                }
                0x3 => {
                    /* 0x03  Fulfill loop */
                    /* This is intended to be sustain release, but the
                     * effect is buggy and just cuts most of the time. */
                    event.fxt = FX_KEYOFF;
                }
                0x4 => {
                    /* 0x04  Old FAR tempo */
                    event.fxt = FX_FAR_TEMPO;
                    event.fxp = 0x10;
                }
                0x5 => {
                    /* 0x05  New FAR tempo */
                    event.fxt = FX_FAR_TEMPO;
                    event.fxp = 0x20;
                }
                _ => {}
            }
        }
        0x1 => {
            /* 0x1?  Pitch offset up */
            event.fxt = FX_FAR_PORTA_UP;
            event.fxp = param as u8;
        }
        0x2 => {
            /* 0x2?  Pitch offset down */
            event.fxt = FX_FAR_PORTA_DN;
            event.fxp = param as u8;
        }
        0x3 => {
            /* 0x3?  Note-port */
            event.fxt = FX_FAR_TPORTA;
            event.fxp = param as u8;
        }
        0x4 => {
            /* 0x4?  Retrigger */
            event.fxt = FX_FAR_RETRIG;
            event.fxp = param as u8;
        }
        0x5 => {
            /* 0x5?  Set Vibrato depth */
            event.fxt = FX_FAR_VIBDEPTH;
            event.fxp = param as u8;
        }
        0x6 => {
            /* 0x6?  Vibrato note */
            event.fxt = FX_FAR_VIBRATO;
            event.fxp = param as u8;
        }
        0x7 => {
            /* 0x7?  Vol Sld Up */
            event.fxt = FX_F_VSLIDE_UP;
            event.fxp = (param << 4) as u8;
        }
        0x8 => {
            /* 0x8?  Vol Sld Dn */
            event.fxt = FX_F_VSLIDE_DN;
            event.fxp = (param << 4) as u8;
        }
        0x9 => {
            /* 0x9?  Sustained vibrato */
            event.fxt = FX_FAR_VIBRATO;
            event.fxp = (0x10 /* Vibrato sustain flag */ | param) as u8;
        }
        0xa => {
            /* 0xa?  Slide-to-vol */
            if (0x01..=0x10).contains(&vol) {
                event.fxt = FX_FAR_SLIDEVOL;
                event.fxp = (((vol - 1) << 4) | param) as u8;
                event.vol = 0;
            }
        }
        0xb => {
            /* 0xb?  Balance */
            event.fxt = FX_SETPAN;
            event.fxp = ((param << 4) | param) as u8;
        }
        0xc => {
            /* 0xc?  Note Offset */
            event.fxt = FX_FAR_DELAY;
            event.fxp = param as u8;
        }
        0xd => {
            /* 0xd?  Fine tempo down */
            event.fxt = FX_FAR_F_TEMPO;
            event.fxp = param as u8;
        }
        0xe => {
            /* 0xe?  Fine tempo up */
            event.fxt = FX_FAR_F_TEMPO;
            event.fxp = (param << 4) as u8;
        }
        0xf => {
            /* 0xf?  Set tempo */
            event.fxt = FX_FAR_TEMPO;
            event.fxp = param as u8;
        }
        _ => {}
    }
}

const COMMENT_MAXLINES: usize = 44;

/// Translation of `far_read_text()`.
fn far_read_text(dest: &mut [u8], mut textlen: usize, f: &mut HioHandle<'_>) {
    /* FAR module text uses 132-char lines with no line breaks... */
    let mut d = 0;

    if textlen > COMMENT_MAXLINES * 132 {
        textlen = COMMENT_MAXLINES * 132;
    }

    while textlen != 0 {
        let mut end = textlen.min(132);
        textlen -= end;
        end = hio_read(&mut dest[d..d + end], 1, end, f);

        let mut lastchar = 0;
        for i in 0..end {
            /* Nulls in the text area are equivalent to spaces. */
            if dest[d + i] == b'\0' {
                dest[d + i] = b' ';
            } else if dest[d + i] != b' ' {
                lastchar = i;
            }
        }
        d += lastchar + 1;
        dest[d] = b'\n';
        d += 1;
    }
    dest[d] = b'\0';
}

/// Translation of `far_load()`.
fn far_load(m: &mut ModuleData, f: &mut HioHandle<'_>, _start: i32) -> i32 {
    let mut ffh = FarHeader {
        magic: 0,
        name: [0; 40],
        crlf: [0; 3],
        headersize: 0,
        version: 0,
        ch_on: [0; 16],
        rsvd1: [0; 9],
        tempo: 0,
        pan: [0; 16],
        rsvd2: [0; 4],
        textlen: 0,
    };
    let mut ffh2 = FarHeader2 {
        order: [0; 256],
        patterns: 0,
        songlen: 0,
        restart: 0,
        patsize: [0; 256],
    };
    let mut fih = FarInstrument::default();
    let mut sample_map = [0u8; 8];

    ffh.magic = hio_read32b(f); /* File magic: 'FAR\xfe' */
    hio_read(&mut ffh.name, 40, 1, f); /* Song name */
    hio_read(&mut ffh.crlf, 3, 1, f); /* 0x0d 0x0a 0x1A */
    ffh.headersize = hio_read16l(f); /* Remaining header size in bytes */
    ffh.version = hio_read8(f); /* Version MSN=major, LSN=minor */
    hio_read(&mut ffh.ch_on, 16, 1, f); /* Channel on/off switches */
    hio_seek(f, 9, SEEK_CUR); /* Current editing values */
    ffh.tempo = hio_read8(f); /* Default tempo */
    hio_read(&mut ffh.pan, 16, 1, f); /* Channel pan definitions */
    hio_read32l(f); /* Grid, mode (for editor) */
    ffh.textlen = hio_read16l(f); /* Length of embedded text */
    let _ = (ffh.magic, &ffh.rsvd1, &ffh.rsvd2);

    /* Sanity check */
    if ffh.tempo >= 16 {
        return -1;
    }

    /* (the buffer is zeroed where the C one is uninitialized) */
    if let Ok(mut comment) = try_vec(ffh.textlen as usize + COMMENT_MAXLINES + 1, 0u8) {
        far_read_text(&mut comment, ffh.textlen as usize, f);
        m.comment = Some(comment);
    } else {
        hio_seek(f, ffh.textlen as i64, SEEK_CUR); /* Skip song text */
    }

    hio_read(&mut ffh2.order, 256, 1, f); /* Orders */
    ffh2.patterns = hio_read8(f); /* Number of stored patterns (?) */
    ffh2.songlen = hio_read8(f); /* Song length in patterns */
    ffh2.restart = hio_read8(f); /* Restart pos */
    for i in 0..256 {
        ffh2.patsize[i] = hio_read16l(f); /* Size of each pattern in bytes */
    }
    let _ = ffh2.patterns;

    if hio_error(f) != 0 {
        return -1;
    }

    /* Skip unsupported header extension if it exists. The documentation claims
     * this field is the "remaining" header size, but it's the total size. */
    if ffh.headersize as i32 > 869 + ffh.textlen as i32
        && hio_seek(f, ffh.headersize as i64, SEEK_SET) != 0
    {
        return -1;
    }

    let mod_ = &mut m.mod_;
    mod_.chn = 16;
    /*mod->pat=ffh2.patterns; (Error in specs? --claudio) */
    mod_.len = ffh2.songlen as i32;
    mod_.rst = ffh2.restart as i32;
    let len = mod_.len as usize;
    mod_.xxo[..len].copy_from_slice(&ffh2.order[..len]);

    mod_.pat = 0;
    for i in 0..256 {
        if ffh2.patsize[i] != 0 {
            mod_.pat = i as i32 + 1;
        }
    }
    /* Make sure referenced zero-sized patterns are also counted. */
    for i in 0..len {
        if mod_.pat <= mod_.xxo[i] as i32 {
            mod_.pat = mod_.xxo[i] as i32 + 1;
        }
    }

    mod_.trk = mod_.chn * mod_.pat;

    if libxmp_far_new_module_extras(m) != 0 {
        return -1;
    }

    let me = far_module_extras(m).unwrap();
    me.coarse_tempo = ffh.tempo as i32;
    me.fine_tempo = 0;
    me.tempo_mode = 1;
    let coarse_tempo = me.coarse_tempo;
    let mut fine_tempo = me.fine_tempo;
    m.time_factor = FAR_TIME_FACTOR;
    libxmp_far_translate_tempo(
        1,
        0,
        coarse_tempo,
        &mut fine_tempo,
        &mut m.mod_.spd,
        &mut m.mod_.bpm,
    );
    far_module_extras(m).unwrap().fine_tempo = fine_tempo;

    m.period_type = PERIOD_CSPD;
    m.c4rate = C4_NTSC_RATE;

    m.quirk |= QUIRK_VSALL | QUIRK_PBALL | QUIRK_VIBALL;

    strncpy(&mut m.mod_.name, &ffh.name, 40);
    let type_ = format!(
        "Farandole Composer {}.{}",
        msn(ffh.version),
        lsn(ffh.version)
    );
    libxmp_set_type(m, type_.as_bytes());

    let mod_ = &mut m.mod_;
    if libxmp_init_pattern(mod_) < 0 {
        return -1;
    }

    /* Read and convert patterns */

    let Ok(mut patbuf) = try_vec(256 * 16 * 4, 0u8) else {
        return -1;
    };

    for i in 0..mod_.pat {
        if libxmp_alloc_pattern(mod_, i) < 0 {
            return -1;
        }

        if ffh2.patsize[i as usize] == 0 {
            continue;
        }

        let rows = (ffh2.patsize[i as usize] as i32 - 2) / 64;

        /* Sanity check */
        if rows <= 0 || rows > 256 {
            return -1;
        }

        mod_.pattern_mut(i).rows = rows;

        if libxmp_alloc_tracks_in_pattern(mod_, i) < 0 {
            return -1;
        }

        let brk = hio_read8(f).wrapping_add(1);
        hio_read8(f);

        if hio_read(&mut patbuf, rows as usize * 64, 1, f) < 1 {
            return -1;
        }

        let mut pos = 0;
        for j in 0..mod_.pattern(i).rows {
            for k in 0..mod_.chn {
                let event = mod_.event_mut(i, k, j);

                if k == 0 && j == brk as i32 {
                    event.f2t = FX_BREAK;
                }

                let note = patbuf[pos];
                let ins = patbuf[pos + 1];
                let vol = patbuf[pos + 2];
                let fxb = patbuf[pos + 3];
                pos += 4;

                /* The instrument number is completely ignored if no note is
                 * present; whatever note is playing will continue. */
                if note != 0 {
                    event.note = note.wrapping_add(48);
                    event.ins = ins.wrapping_add(1);
                }

                if (0x01..=0x10).contains(&vol) {
                    event.vol = (vol - 1) * 16 + 1;
                } else if note != 0 || vol != 0 {
                    /* aurora.far contains note+ins without volume. Most are
                     * after break positions, but one is played in order 16.
                     * This is invalid and sets sets the volume to zero.
                     * Volumes greater than 0x10 (f) also usually seem to be
                     * treated as zero (OOB table read?).
                     */
                    event.vol = 1;
                }

                far_translate_effect(event, msn(fxb), lsn(fxb), vol as i32);
            }
        }
    }
    drop(patbuf);

    /* Allocate tracks for any patterns referenced with a size of 0. These
     * use the configured pattern break position, which is 64 by default. */
    for i in 0..mod_.len as usize {
        let pat = mod_.xxo[i] as i32;
        if mod_.pattern(pat).rows == 0 {
            mod_.pattern_mut(pat).rows = 64;
            if libxmp_alloc_tracks_in_pattern(mod_, pat) < 0 {
                return -1;
            }
        }
    }

    mod_.ins = -1;
    if hio_read(&mut sample_map, 1, 8, f) < 8 {
        return -1;
    }
    for i in 0..64 {
        if sample_map[i / 8] & (1 << (i % 8)) != 0 {
            mod_.ins = i as i32;
        }
    }
    mod_.ins += 1;

    mod_.smp = mod_.ins;

    if libxmp_init_instrument(m) < 0 {
        return -1;
    }

    /* Read and convert instruments and samples */

    for i in 0..m.mod_.ins {
        let iu = i as usize;
        if sample_map[iu / 8] & (1 << (iu % 8)) == 0 {
            continue;
        }

        let mod_ = &mut m.mod_;
        if libxmp_alloc_subinstrument(mod_, i, 1) < 0 {
            return -1;
        }

        hio_read(&mut fih.name, 32, 1, f); /* Instrument name */
        fih.length = hio_read32l(f); /* Length of sample (up to 64Kb) */
        fih.finetune = hio_read8(f); /* Finetune (unsupported) */
        fih.volume = hio_read8(f); /* Volume (unsupported?) */
        fih.loop_start = hio_read32l(f); /* Loop start */
        fih.loopend = hio_read32l(f); /* Loop end */
        fih.sampletype = hio_read8(f); /* 1=16 bit sample */
        fih.loopmode = hio_read8(f);
        let _ = (fih.finetune, fih.volume);

        /* Sanity check */
        if fih.length > 0x10000 || fih.loop_start > 0x10000 || fih.loopend > 0x10000 {
            return -1;
        }

        let xxs = &mut mod_.xxs[iu];
        xxs.len = fih.length as i32;
        xxs.lps = fih.loop_start as i32;
        xxs.lpe = fih.loopend as i32;
        xxs.flg = 0;

        if xxs.len > 0 {
            mod_.xxi[iu].nsm = 1;
        }

        if fih.sampletype != 0 {
            xxs.flg |= XMP_SAMPLE_16BIT;
            xxs.len >>= 1;
            xxs.lps >>= 1;
            xxs.lpe >>= 1;
        }

        xxs.flg |= if fih.loopmode != 0 {
            XMP_SAMPLE_LOOP
        } else {
            0
        };
        let sub = &mut mod_.xxi[iu].sub[0];
        sub.vol = 0xff; /* fih.volume; */
        sub.pan = XMP_INST_NO_DEFAULT_PAN;
        sub.sid = i;

        libxmp_instrument_name(mod_, i, &fih.name, 32);

        let smpctl = m.smpctl;
        if libxmp_load_sample(smpctl, Some(f), 0, &mut m.mod_.xxs[iu], None) < 0 {
            return -1;
        }
    }

    let mod_ = &mut m.mod_;
    /* Panning map */
    for i in 0..16 {
        if ffh.ch_on[i] == 0 {
            mod_.xxc[i].flg |= XMP_CHANNEL_MUTE;
        }
        if ffh.pan[i] < 0x10 {
            mod_.xxc[i].pan = ((ffh.pan[i] << 4) | ffh.pan[i]) as i32;
        }
    }

    m.volbase = 0xf0;

    0
}
