// Rust translation of src/loaders/sym_load.c from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2026 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Digital Symphony (DSym) loader.

use super::super::common::*;
use super::super::effects::*;
use super::super::hio::*;
use super::super::*;
use super::common::*;
use super::lzw::*;
use super::sample::libxmp_load_sample;
use super::*;

pub(crate) const LIBXMP_LOADER_SYM: FormatLoader = FormatLoader {
    name: "Digital Symphony",
    test: sym_test,
    loader: sym_load,
};

/// Translation of `sym_test()`.
fn sym_test(f: &mut HioHandle<'_>, t: Option<&mut [u8]>, _start: i32) -> i32 {
    let a = hio_read32b(f);
    let b = hio_read32b(f);

    if a != 0x02011313 || b != 0x1412010B {
        /* BASSTRAK */
        return -1;
    }

    let ver = hio_read8(f);

    /* v1 files are the same as v0 but may contain strange compression
     * formats. Deal with that problem later if it arises.
     */
    if ver > 1 {
        return -1;
    }

    hio_read8(f); /* chn */
    hio_read16l(f); /* pat */
    hio_read16l(f); /* trk */
    hio_read24l(f); /* infolen */

    for _ in 0..63 {
        if !hio_read8(f) & 0x80 != 0 {
            hio_read24l(f);
        }
    }

    let n = hio_read8(f) as i32;
    libxmp_read_title(f, t, n);

    0
}

/// Translation of `fix_effect()`.
fn fix_effect(e: &mut XmpEvent, mut parm: i32) {
    match e.fxt {
        0x00 /* 00 xyz Normal play or Arpeggio + Volume Slide Up */
        | 0x01 /* 01 xyy Slide Up + Volume Slide Up */
        | 0x02 /* 02 xyy Slide Down + Volume Slide Up */ => {
            if parm & 0xff != 0 {
                e.fxp = (parm & 0xff) as u8;
            } else {
                e.fxt = 0;
            }
            if parm >> 8 != 0 {
                e.f2t = FX_VOLSLIDE_UP;
                e.f2p = (parm >> 8) as u8;
            }
        }
        0x03 /* 03 xyy Tone Portamento */
        | 0x04 /* 04 xyz Vibrato */
        | 0x07 /* 07 xyz Tremolo */ => {
            e.fxp = parm as u8;
        }
        0x05 /* 05 xyz Tone Portamento + Volume Slide */
        | 0x06 /* 06 xyz Vibrato + Volume Slide */ => {
            e.fxp = parm as u8;
            if parm == 0 {
                e.fxt -= 2;
            }
        }
        0x09 => {
            /* 09 xxx Set Sample Offset */
            e.fxp = (parm >> 1) as u8;
            e.f2t = FX_HIOFFSET;
            e.f2p = (parm >> 9) as u8;
        }
        0x0a => {
            /* 0A xyz Volume Slide + Fine Slide Up */
            if parm & 0xff != 0 {
                e.fxp = (parm & 0xff) as u8;
            } else {
                e.fxt = 0;
            }
            e.f2t = FX_EXTENDED;
            e.f2p = ((EX_F_PORTA_UP << 4) | ((parm & 0xf00) >> 8)) as u8;
        }
        0x0b /* 0B xxx Position Jump */ | 0x0c /* 0C xyy Set Volume */ => {
            e.fxp = parm as u8;
        }
        0x0d => {
            /* 0D xyy Pattern Break */
            e.fxt = FX_IT_BREAK;
            e.fxp = if (parm & 0xff) < 0x40 { parm as u8 } else { 0 };
        }
        0x0f => {
            /* 0F xxx Set Speed */
            if parm != 0 {
                e.fxt = FX_S3M_SPEED;
                e.fxp = parm.min(255) as u8;
            } else {
                e.fxt = 0;
            }
        }
        0x13 => {
            /* 13 xxy Glissando Control */
            e.fxt = FX_EXTENDED;
            e.fxp = ((EX_GLISS << 4) | (parm & 0x0f)) as u8;
        }
        0x14 => {
            /* 14 xxy Set Vibrato Waveform */
            e.fxt = FX_EXTENDED;
            e.fxp = ((EX_VIBRATO_WF << 4) | (parm & 0x0f)) as u8;
        }
        0x15 => {
            /* 15 xxy Set Fine Tune */
            e.fxt = FX_EXTENDED;
            e.fxp = ((EX_FINETUNE << 4) | (parm & 0x0f)) as u8;
        }
        0x16 => {
            /* 16 xxx Jump to Loop */
            /* TODO: 16, 19 should be able to support larger params. */
            e.fxt = FX_EXTENDED;
            e.fxp = ((EX_PATTERN_LOOP << 4) | parm.min(0x0f)) as u8;
        }
        0x17 => {
            /* 17 xxy Set Tremolo Waveform */
            e.fxt = FX_EXTENDED;
            e.fxp = ((EX_TREMOLO_WF << 4) | (parm & 0x0f)) as u8;
        }
        0x19 => {
            /* 19 xxx Retrig Note */
            if parm < 0x100 {
                e.fxt = FX_RETRIG;
                e.fxp = (parm & 0xff) as u8;
            } else {
                /* ignore */
                e.fxt = 0;
            }
        }
        0x11 /* 11 xyy Fine Slide Up + Fine Volume Slide Up */
        | 0x12 /* 12 xyy Fine Slide Down + Fine Volume Slide Up */
        | 0x1a /* 1A xyy Fine Slide Up + Fine Volume Slide Down */
        | 0x1b /* 1B xyy Fine Slide Down + Fine Volume Slide Down */ => {
            let pitch_effect = if e.fxt == 0x11 || e.fxt == 0x1a {
                FX_F_PORTA_UP
            } else {
                FX_F_PORTA_DN
            };
            let vol_effect = if e.fxt == 0x11 || e.fxt == 0x12 {
                FX_F_VSLIDE_UP
            } else {
                FX_F_VSLIDE_DN
            };

            if parm & 0xff != 0 {
                e.fxt = pitch_effect;
                e.fxp = (parm & 0xff) as u8;
            } else {
                e.fxt = 0;
            }
            if parm >> 8 != 0 {
                e.f2t = vol_effect;
                e.f2p = (parm >> 8) as u8;
            }
        }
        0x1c => {
            /* 1C xxx Note Cut */
            /* TODO: 1c, 1d, 1e should be able to support larger params. */
            e.fxt = FX_EXTENDED;
            e.fxp = ((EX_CUT << 4) | parm.min(0x0f)) as u8;
        }
        0x1d => {
            /* 1D xxx Note Delay */
            e.fxt = FX_EXTENDED;
            e.fxp = ((EX_DELAY << 4) | parm.min(0x0f)) as u8;
        }
        0x1e => {
            /* 1E xxx Pattern Delay */
            e.fxt = FX_EXTENDED;
            e.fxp = ((EX_PATT_DELAY << 4) | parm.min(0x0f)) as u8;
        }
        0x1f => {
            /* 1F xxy Invert Loop */
            e.fxt = FX_EXTENDED;
            e.fxp = ((EX_INVLOOP << 4) | (parm & 0xf)) as u8;
        }
        0x20 => {
            /* 20 xyz Normal play or Arpeggio + Volume Slide Down */
            e.fxt = FX_ARPEGGIO;
            e.fxp = (parm & 0xff) as u8;
            if parm >> 8 != 0 {
                e.f2t = FX_VOLSLIDE_DN;
                e.f2p = (parm >> 8) as u8;
            }
        }
        0x21 => {
            /* 21 xyy Slide Up + Volume Slide Down */
            if parm & 0xff != 0 {
                e.fxt = FX_PORTA_UP;
                e.fxp = (parm & 0xff) as u8;
            } else {
                e.fxt = 0;
            }
            if parm >> 8 != 0 {
                e.f2t = FX_VOLSLIDE_DN;
                e.f2p = (parm >> 8) as u8;
            }
        }
        0x22 => {
            /* 22 xyy Slide Down + Volume Slide Down */
            if parm & 0xff != 0 {
                e.fxt = FX_PORTA_DN;
                e.fxp = (parm & 0xff) as u8;
            } else {
                e.fxt = 0;
            }
            if parm >> 8 != 0 {
                e.f2t = FX_VOLSLIDE_DN;
                e.f2p = (parm >> 8) as u8;
            }
        }
        0x2a => {
            /* 2A xyz Volume Slide + Fine Slide Down */
            if parm & 0xff != 0 {
                e.fxt = FX_VOLSLIDE;
                e.fxp = (parm & 0xff) as u8;
            } else {
                e.fxt = 0;
            }
            e.f2t = FX_EXTENDED;
            e.f2p = ((EX_F_PORTA_DN << 4) | (parm >> 8)) as u8;
        }
        0x2b => {
            /* 2B xyy Line Jump */
            e.fxt = FX_LINE_JUMP;
            e.fxp = if parm < 0x40 { parm as u8 } else { 0 };
        }
        0x2f => {
            /* 2F xxx Set Tempo */
            if parm != 0 {
                parm = (parm + 4) >> 3; /* round to nearest */
                clamp(&mut parm, XMP_MIN_BPM, 255);
                e.fxt = FX_S3M_BPM;
                e.fxp = parm as u8;
            } else {
                e.fxt = 0;
            }
        }
        0x30 => {
            /* 30 xxy Set Stereo */
            e.fxt = FX_SETPAN;
            if parm & 7 != 0 {
                /* !Tracker-style panning: 1=left, 4=center, 7=right. */
                if parm & 8 == 0 {
                    e.fxp = (42 * ((parm & 7) - 1) + 2) as u8;
                } else {
                    e.fxt = 0;
                }
            } else {
                parm >>= 4;
                if parm < 128 {
                    e.fxp = (parm + 128) as u8;
                } else if parm > 128 {
                    e.fxp = (255 - parm) as u8;
                } else {
                    e.fxt = 0;
                }
            }
        }
        0x31 => {
            /* 31 xxx Song Upcall */
            e.fxt = 0;
        }
        0x32 => {
            /* 32 xxx Unset Sample Repeat */
            e.fxt = FX_KEYOFF;
            e.fxp = 0;
        }
        _ => {
            e.fxt = 0;
        }
    }
}

/// Translation of `readptr32l()`.
fn readptr32l(p: &[u8]) -> u32 {
    let a = p[0] as u32;
    let b = p[1] as u32;
    let c = p[2] as u32;
    let d = p[3] as u32;

    (d << 24) | (c << 16) | (b << 8) | a
}

/// Translation of `readptr16l()`.
fn readptr16l(p: &[u8]) -> u32 {
    let a = p[0] as u32;
    let b = p[1] as u32;

    (b << 8) | a
}

/// Translation of `sym_load()`.
fn sym_load(m: &mut ModuleData, f: &mut HioHandle<'_>, _start: i32) -> i32 {
    let mut sn = [0i32; 64];
    let mut max_sample_size = 1;
    let mut allowed_effects = [0u8; 8];

    hio_seek(f, 8, SEEK_CUR); /* BASSTRAK */

    /*ver =*/
    hio_read8(f);
    libxmp_set_type(m, b"Digital Symphony");

    let mod_ = &mut m.mod_;
    mod_.chn = hio_read8(f) as i32;
    mod_.pat = hio_read16l(f) as i32;
    mod_.len = mod_.pat;

    /* Sanity check */
    if mod_.chn < 1 || mod_.chn > 8 || mod_.pat > XMP_MAX_MOD_LENGTH as i32 {
        return -1;
    }

    mod_.trk = hio_read16l(f) as i32; /* Symphony patterns are actually tracks */
    let infolen = hio_read24l(f) as i32;

    /* Sanity check - track 0x1000 is used to indicate the empty track. */
    if mod_.trk > 0x1000 {
        return -1;
    }

    mod_.ins = 63;
    mod_.smp = 63;

    if libxmp_init_instrument(m) < 0 {
        return -1;
    }

    let mod_ = &mut m.mod_;
    for i in 0..mod_.ins {
        let iu = i as usize;
        if libxmp_alloc_subinstrument(mod_, i, 1) < 0 {
            return -1;
        }

        sn[iu] = hio_read8(f) as i32; /* sample name length */

        if !sn[iu] & 0x80 != 0 {
            mod_.xxs[iu].len = (hio_read24l(f) << 1) as i32;
            mod_.xxi[iu].nsm = 1;

            /* Sanity check */
            if mod_.xxs[iu].len > 0x80000 {
                return -1;
            }

            if max_sample_size < mod_.xxs[iu].len {
                max_sample_size = mod_.xxs[iu].len;
            }
        }
    }

    let a = hio_read8(f) as u32; /* track name length */
    if a > 32 {
        hio_read(&mut mod_.name, 1, 32, f);
        // Note (upstream): C seeks to the length less 32 from the start of
        // the file, not past the rest of the name.
        hio_seek(f, (a - 32) as i64, SEEK_CUR);
    } else {
        hio_read(&mut mod_.name, 1, a as usize, f);
    }

    hio_read(&mut allowed_effects, 1, 8, f);

    mod_.trk += 1; /* alloc extra empty track */
    if libxmp_init_pattern(mod_) < 0 {
        return -1;
    }

    /* Determine the required size of temporary buffer and allocate it now. */
    /* Uncompressed sequence size */
    let sequence_size = mod_.len * mod_.chn * 2;
    if sequence_size > max_sample_size {
        max_sample_size = sequence_size;
    }

    let tracks_size = 64 * (mod_.trk - 1) * 4; /* Uncompressed tracks size */
    if tracks_size > max_sample_size {
        max_sample_size = tracks_size;
    }

    let Ok(mut buf) = try_vec(max_sample_size as usize, 0u8) else {
        return -1;
    };

    /* Sequence */
    let a = hio_read8(f); /* packing */

    if a != 0 && a != 1 {
        return -1;
    }

    let ss = sequence_size as usize;
    if a != 0 {
        if libxmp_read_lzw(&mut buf[..ss], ss, LZW_FLAGS_SYM, f) < 0 {
            return -1;
        }
    } else if hio_read(&mut buf[..ss], 1, ss, f) != ss {
        return -1;
    }

    for i in 0..mod_.len {
        if libxmp_alloc_pattern(mod_, i) < 0 {
            return -1;
        }

        mod_.pattern_mut(i).rows = 64;

        for j in 0..mod_.chn {
            let idx = (2 * (i * mod_.chn + j)) as usize;
            let mut t = readptr16l(&buf[idx..]) as i32;

            if t == 0x1000 {
                /* empty trk */
                t = mod_.trk - 1;
            } else if t >= mod_.trk - 1 {
                /* Sanity check */
                return -1;
            }

            mod_.pattern_mut(i).index[j as usize] = t;
        }
        mod_.xxo[i as usize] = i as u8;
    }

    /* Read and convert patterns */

    /* Patterns are stored in blocks of up to 2000 patterns. If there are
     * more than 2000 patterns, they need to be read in multiple passes.
     *
     * See 4096_patterns.dsym.
     */
    let mut i = 0;
    while i < tracks_size {
        let blk_size = (tracks_size - i).min(4 * 64 * 2000) as usize;
        let iu = i as usize;

        let a = hio_read8(f);

        if a != 0 && a != 1 {
            return -1;
        }

        if a != 0 {
            if libxmp_read_lzw(&mut buf[iu..iu + blk_size], blk_size, LZW_FLAGS_SYM, f) < 0 {
                return -1;
            }
        } else if hio_read(&mut buf[iu..iu + blk_size], 1, blk_size, f) != blk_size {
            return -1;
        }
        i += 4 * 64 * 2000;
    }

    let mut i = 0;
    while i < mod_.trk - 1 {
        if libxmp_alloc_track(mod_, i, 64) < 0 {
            return -1;
        }

        for j in 0..mod_.track(i).rows {
            let event = &mut mod_.track_mut(i).event[j as usize];

            let b = readptr32l(&buf[(4 * (i * 64 + j)) as usize..]);
            event.note = (b & 0x0000003f) as u8;
            if event.note != 0 {
                event.note += 48;
            }
            event.ins = ((b & 0x00001fc0) >> 6) as u8;
            event.fxt = ((b & 0x000fc000) >> 14) as u8;
            let parm = ((b & 0xfff00000) >> 20) as i32;

            if allowed_effects[(event.fxt >> 3) as usize] & (1 << (event.fxt & 7)) != 0 {
                fix_effect(event, parm);
            } else {
                event.fxt = 0;
            }
        }
        i += 1;
    }

    /* Extra track */
    if libxmp_alloc_track(mod_, i, 64) < 0 {
        return -1;
    }

    /* Load and convert instruments */

    for i in 0..m.mod_.ins {
        let iu = i as usize;
        let mut namebuf = [0u8; 128];

        let n = (sn[iu] & 0x7f) as usize;
        hio_read(&mut namebuf, 1, n, f);
        libxmp_instrument_name(&mut m.mod_, i, &namebuf, 32);

        if !sn[iu] & 0x80 != 0 {
            let xxs_len = m.mod_.xxs[iu].len;
            let xtra = &mut m.xtra[iu];
            xtra.sus = (hio_read24l(f) << 1) as i32;
            let looplen = (hio_read24l(f) << 1) as i32;
            xtra.sue = xtra.sus + looplen;
            if xtra.sus < xxs_len && xtra.sus < xtra.sue && xtra.sue <= xxs_len && looplen > 2 {
                m.mod_.xxs[iu].flg |= XMP_SAMPLE_SLOOP;
            }
            let sub = &mut m.mod_.xxi[iu].sub[0];
            sub.vol = hio_read8(f) as i32;
            sub.pan = XMP_INST_NO_DEFAULT_PAN;
            /* finetune adjusted comparing DSym and S3M versions
             * of "inside out" */
            sub.fin = (hio_read8(f) << 4) as i8 as i32;
            sub.sid = i;
        }

        if sn[iu] & 0x80 != 0 || m.mod_.xxs[iu].len == 0 {
            continue;
        }

        let a = hio_read8(f);

        let smpctl = m.smpctl;
        let ret;
        match a {
            0 => {
                /* Signed 8-bit, logarithmic. */
                ret = libxmp_load_sample(
                    smpctl,
                    Some(f),
                    SAMPLE_FLAG_VIDC,
                    &mut m.mod_.xxs[iu],
                    None,
                );
            }

            1 => {
                /* LZW compressed signed 8-bit delta, linear. */
                let size = m.mod_.xxs[iu].len as usize;

                if libxmp_read_lzw(&mut buf[..size], size, LZW_FLAGS_SYM, f) < 0 {
                    return -1;
                }

                ret = libxmp_load_sample(
                    smpctl,
                    None,
                    SAMPLE_FLAG_NOLOAD | SAMPLE_FLAG_DIFF,
                    &mut m.mod_.xxs[iu],
                    Some(&buf),
                );
            }

            2 => {
                /* Signed 8-bit, linear. */
                ret = libxmp_load_sample(smpctl, Some(f), 0, &mut m.mod_.xxs[iu], None);
            }

            3 => {
                /* Signed 16-bit, linear. */
                m.mod_.xxs[iu].flg |= XMP_SAMPLE_16BIT;
                ret = libxmp_load_sample(smpctl, Some(f), 0, &mut m.mod_.xxs[iu], None);
            }

            4 => {
                /* Sigma-delta compressed unsigned 8-bit, linear. */
                let size = m.mod_.xxs[iu].len as usize;
                if libxmp_read_sigma_delta(&mut buf[..size], size, f) < 0 {
                    return -1;
                }

                ret = libxmp_load_sample(
                    smpctl,
                    None,
                    SAMPLE_FLAG_NOLOAD | SAMPLE_FLAG_UNS,
                    &mut m.mod_.xxs[iu],
                    Some(&buf),
                );
            }

            5 => {
                /* Sigma-delta compressed signed 8-bit, logarithmic. */
                let size = m.mod_.xxs[iu].len as usize;
                if libxmp_read_sigma_delta(&mut buf[..size], size, f) < 0 {
                    return -1;
                }

                /* This uses a bit packing that isn't either mu-law or
                 * normal Archimedes VIDC. Convert to the latter... */
                for b in buf[..size].iter_mut() {
                    let t = if *b < 128 { !*b } else { *b };
                    *b = (*b >> 7) | (t << 1);
                }

                ret = libxmp_load_sample(
                    smpctl,
                    None,
                    SAMPLE_FLAG_NOLOAD | SAMPLE_FLAG_VIDC,
                    &mut m.mod_.xxs[iu],
                    Some(&buf),
                );
            }

            _ => {
                return -1;
            }
        }

        if ret < 0 {
            return -1;
        }
    }

    /* Information text */
    if infolen > 0 {
        let a = hio_read8(f); /* Packing */

        if let Ok(mut comment) = try_vec(infolen as usize + 1, 0u8) {
            comment[infolen as usize] = b'\0';
            let n = infolen as usize;
            let ret = if a != 0 {
                libxmp_read_lzw(&mut comment[..n], n, LZW_FLAGS_SYM, f)
            } else {
                let size = hio_read(&mut comment[..n], 1, n, f);
                if size < n {
                    -1
                } else {
                    0
                }
            };
            if ret >= 0 {
                m.comment = Some(comment);
            }
        }
    }

    for i in 0..m.mod_.chn {
        let pan = defpan(m, (((i + 3) / 2) % 2) * 0xff);
        m.mod_.xxc[i as usize].pan = pan;
    }

    m.quirk = QUIRK_VIBALL | QUIRK_KEYOFF | QUIRK_INVLOOP;

    0
}
