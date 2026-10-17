// Rust translation of src/loaders/gdm_load.c from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2026 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

/*
 * Based on the GDM (General Digital Music) version 1.0 File Format
 * Specification - Revision 2 by MenTaLguY
 */

//! The General Digital Music (GDM) loader.

use super::super::common::*;
use super::super::effects::*;
use super::super::hio::*;
use super::super::period::libxmp_c2spd_to_note;
use super::super::*;
use super::common::*;
use super::sample::libxmp_load_sample;
use super::*;

const MAGIC_GDM: u32 = magic4(b'G', b'D', b'M', 0xfe);
const MAGIC_GMFS: u32 = magic4(b'G', b'M', b'F', b'S');

pub(crate) const LIBXMP_LOADER_GDM: FormatLoader = FormatLoader {
    name: "General Digital Music",
    test: gdm_test,
    loader: gdm_load,
};

/// Translation of `gdm_test()`.
fn gdm_test(f: &mut HioHandle<'_>, t: Option<&mut [u8]>, start: i32) -> i32 {
    if hio_read32b(f) != MAGIC_GDM {
        return -1;
    }

    hio_seek(f, start as i64 + 0x47, SEEK_SET);
    if hio_read32b(f) != MAGIC_GMFS {
        return -1;
    }

    hio_seek(f, start as i64 + 4, SEEK_SET);
    libxmp_read_title(f, t, 32);

    0
}

/// Translation of `gdm_translate_effect()`.
fn gdm_translate_effect(fxt: &mut u8, fxp: &mut u8) {
    match *fxt {
        0x00 => {
            /* no effect */
            *fxp = 0;
        }
        0x01..=0x07 => { /* same as protracker */ }
        0x08 => {
            *fxt = FX_TREMOR;
        }
        0x09..=0x0d => { /* same as protracker */ }
        0x0e => {
            /* Convert some extended effects to their S3M equivalents. This is
             * necessary because the continue effects were left as the original
             * effect (e.g. FX_VOLSLIDE for the fine volume slides) by 2GDM!
             * Otherwise, these should be the same as protracker.
             */
            let h = msn(*fxp);
            let l = lsn(*fxp) as u8;
            match h {
                EX_F_PORTA_UP => {
                    *fxt = FX_PORTA_UP;
                    *fxp = l | 0xF0;
                }
                EX_F_PORTA_DN => {
                    *fxt = FX_PORTA_DN;
                    *fxp = l | 0xF0;
                }
                0x8 => {
                    /* extra fine portamento up */
                    *fxt = FX_PORTA_UP;
                    *fxp = l | 0xE0;
                }
                0x9 => {
                    /* extra fine portamento down */
                    *fxt = FX_PORTA_DN;
                    *fxp = l | 0xE0;
                }
                EX_F_VSLIDE_UP => {
                    /* Don't convert 0 as it would turn into volume slide down... */
                    if l != 0 {
                        *fxt = FX_VOLSLIDE;
                        *fxp = (l << 4) | 0xF;
                    }
                }
                EX_F_VSLIDE_DN => {
                    /* Don't convert 0 as it would turn into volume slide up... */
                    if l != 0 {
                        *fxt = FX_VOLSLIDE;
                        *fxp = l | 0xF0;
                    }
                }
                _ => {}
            }
        }
        0x0f => {
            /* set speed */
            *fxt = FX_S3M_SPEED;
        }
        0x10 => {
            /* arpeggio */
            *fxt = FX_S3M_ARPEGGIO;
        }
        0x11 => {
            /* set internal flag */
            *fxt = 0;
            *fxp = 0;
        }
        0x12 => {
            *fxt = FX_MULTI_RETRIG;
        }
        0x13 => {
            *fxt = FX_GLOBALVOL;
        }
        0x14 => {
            *fxt = FX_FINE_VIBRATO;
        }
        0x1e => {
            /* special misc */
            match msn(*fxp) {
                0x0 => {
                    /* sample control */
                    if lsn(*fxp) == 1 {
                        /* enable surround */
                        /* This is the only sample control effect
                         * that 2GDM emits. BWSB ignores it,
                         * but supporting it is harmless. */
                        *fxt = FX_SURROUND;
                        *fxp = 1;
                    } else {
                        *fxt = 0;
                        *fxp = 0;
                    }
                }
                0x8 => {
                    /* set pan position */
                    *fxt = FX_EXTENDED;
                }
                _ => {
                    *fxt = 0;
                    *fxp = 0;
                }
            }
        }
        0x1f => {
            *fxt = FX_S3M_BPM;
        }
        _ => {
            *fxt = 0;
            *fxp = 0;
        }
    }
}

/// Translation of `gdm_load()`.
fn gdm_load(m: &mut ModuleData, f: &mut HioHandle<'_>, start: i32) -> i32 {
    let mut buffer = [0u8; 32];
    let mut panmap = [0u8; 32];
    let start = start as i64;

    hio_read32b(f); /* skip magic */
    hio_read(&mut m.mod_.name, 1, 32, f);
    hio_seek(f, 32, SEEK_CUR); /* skip author */

    hio_seek(f, 7, SEEK_CUR);

    let vermaj = hio_read8(f);
    let vermin = hio_read8(f);
    let tracker = hio_read16l(f);
    let tvmaj = hio_read8(f);
    let tvmin = hio_read8(f);

    let s = if tracker == 0 {
        format!("GDM {}.{:02} (2GDM {}.{:02})", vermaj, vermin, tvmaj, tvmin)
    } else {
        format!(
            "GDM {}.{:02} (unknown tracker {}.{:02})",
            vermaj, vermin, tvmaj, tvmin
        )
    };
    libxmp_set_type(m, s.as_bytes());

    if hio_read(&mut panmap, 32, 1, f) == 0 {
        return -1;
    }
    let mod_ = &mut m.mod_;
    for i in 0..32 {
        if panmap[i] == 255 {
            panmap[i] = 8;
            mod_.xxc[i].flg |= XMP_CHANNEL_MUTE;
        } else if panmap[i] == 16 {
            panmap[i] = 8;
        }
        mod_.xxc[i].pan = 0x80 + (panmap[i] as i32 - 8) * 16;
    }

    mod_.gvl = hio_read8(f) as i32;
    mod_.spd = hio_read8(f) as i32;
    mod_.bpm = hio_read8(f) as i32;
    /*origfmt =*/
    hio_read16l(f);
    let ord_ofs = hio_read32l(f) as i32;
    mod_.len = hio_read8(f) as i32 + 1;
    let pat_ofs = hio_read32l(f) as i32;
    mod_.pat = hio_read8(f) as i32 + 1;
    let ins_ofs = hio_read32l(f) as i32;
    let smp_ofs = hio_read32l(f) as i32;
    mod_.smp = hio_read8(f) as i32 + 1;
    mod_.ins = mod_.smp;

    /* Sanity check */
    if mod_.ins > MAX_INSTRUMENTS {
        return -1;
    }

    m.c4rate = C4_NTSC_RATE;

    let mod_ = &mut m.mod_;
    hio_seek(f, start + ord_ofs as i64, SEEK_SET);

    for i in 0..mod_.len as usize {
        mod_.xxo[i] = hio_read8(f);
    }

    /* Read instrument data */

    hio_seek(f, start + ins_ofs as i64, SEEK_SET);

    if libxmp_init_instrument(m) < 0 {
        return -1;
    }

    let mod_ = &mut m.mod_;
    for i in 0..mod_.ins {
        let iu = i as usize;

        if libxmp_alloc_subinstrument(mod_, i, 1) < 0 {
            return -1;
        }

        if hio_read(&mut buffer, 1, 32, f) != 32 {
            return -1;
        }

        libxmp_instrument_name(mod_, i, &buffer, 32);
        hio_seek(f, 12, SEEK_CUR); /* skip filename */
        hio_read8(f); /* skip EMS handle */
        let xxs = &mut mod_.xxs[iu];
        xxs.len = hio_read32l(f) as i32;
        xxs.lps = hio_read32l(f) as i32;
        xxs.lpe = hio_read32l(f) as i32;
        let flg = hio_read8(f);
        let c4spd = hio_read16l(f) as i32;
        let vol = hio_read8(f) as i32;
        let pan = hio_read8(f) as i32;

        let sub = &mut mod_.xxi[iu].sub[0];
        sub.vol = if vol > 0x40 { 0x40 } else { vol };
        sub.pan = if pan > 15 {
            XMP_INST_NO_DEFAULT_PAN
        } else {
            0x80 + (pan - 8) * 16
        };
        libxmp_c2spd_to_note(c4spd, &mut sub.xpo, &mut sub.fin);

        sub.sid = i;
        xxs.flg = 0;

        if xxs.len > 0 {
            mod_.xxi[iu].nsm = 1;
        }

        if flg & 0x01 != 0 {
            xxs.flg |= XMP_SAMPLE_LOOP;
        }
        if flg & 0x02 != 0 {
            xxs.flg |= XMP_SAMPLE_16BIT;
            xxs.len >>= 1;
            xxs.lps >>= 1;
            xxs.lpe >>= 1;
        }
    }

    /* Read and convert patterns */

    hio_seek(f, start + pat_ofs as i64, SEEK_SET);

    /* Effects in muted channels are processed, so scan patterns first to
     * see the real number of channels
     */
    mod_.chn = 0;
    for _ in 0..mod_.pat {
        let mut len = hio_read16l(f) as i32;
        len -= 2;

        let mut r = 0;
        while len > 0 {
            let c = hio_read8(f) as i32;
            if hio_error(f) != 0 {
                return -1;
            }
            len -= 1;

            if c == 0 {
                r += 1;

                /* Sanity check */
                if len == 0 {
                    if r > 64 {
                        return -1;
                    }
                } else if r >= 64 {
                    return -1;
                }

                continue;
            }

            if mod_.chn <= (c & 0x1f) {
                mod_.chn = (c & 0x1f) + 1;
            }

            if c & 0x20 != 0 {
                /* note and sample follows */
                hio_read8(f);
                hio_read8(f);
                len -= 2;
            }

            if c & 0x40 != 0 {
                /* effect(s) follow */
                loop {
                    let k = hio_read8(f);
                    if hio_error(f) != 0 {
                        return -1;
                    }
                    len -= 1;
                    if (k & 0xc0) != 0xc0 {
                        hio_read8(f);
                        len -= 1;
                    }
                    if k & 0x20 == 0 {
                        break;
                    }
                }
            }
        }
    }

    mod_.trk = mod_.pat * mod_.chn;

    if libxmp_init_pattern(mod_) < 0 {
        return -1;
    }

    hio_seek(f, start + pat_ofs as i64, SEEK_SET);

    for i in 0..mod_.pat {
        if libxmp_alloc_pattern_tracks(mod_, i, 64) < 0 {
            return -1;
        }

        let mut len = hio_read16l(f) as i32;
        len -= 2;

        let mut r = 0;
        while len > 0 {
            let c = hio_read8(f) as i32;
            if hio_error(f) != 0 {
                return -1;
            }
            len -= 1;

            if c == 0 {
                r += 1;
                continue;
            }

            /* Sanity check */
            if (c & 0x1f) >= mod_.chn || r >= 64 {
                return -1;
            }

            let event = mod_.event_mut(i, c & 0x1f, r);

            if c & 0x20 != 0 {
                /* note and sample follows */
                let k = hio_read8(f);
                /* 0 is empty note */
                event.note = if k != 0 {
                    (12 + 12 * msn(k & 0x7f) + lsn(k)) as u8
                } else {
                    0
                };
                event.ins = hio_read8(f);
                len -= 2;
            }

            if c & 0x40 != 0 {
                /* effect(s) follow */
                loop {
                    let k = hio_read8(f);
                    if hio_error(f) != 0 {
                        return -1;
                    }
                    len -= 1;
                    match (k & 0xc0) >> 6 {
                        0 => {
                            event.fxt = k & 0x1f;
                            event.fxp = hio_read8(f);
                            len -= 1;
                            gdm_translate_effect(&mut event.fxt, &mut event.fxp);
                        }
                        1 => {
                            event.f2t = k & 0x1f;
                            event.f2p = hio_read8(f);
                            len -= 1;
                            gdm_translate_effect(&mut event.f2t, &mut event.f2p);
                        }
                        2 => {
                            hio_read8(f);
                            len -= 1;
                        }
                        _ => {}
                    }
                    if k & 0x20 == 0 {
                        break;
                    }
                }
            }
        }
    }

    /* Read samples */

    hio_seek(f, start + smp_ofs as i64, SEEK_SET);

    for i in 0..m.mod_.ins as usize {
        let smpctl = m.smpctl;
        if libxmp_load_sample(smpctl, Some(f), SAMPLE_FLAG_UNS, &mut m.mod_.xxs[i], None) < 0 {
            return -1;
        }
    }

    m.quirk |= QUIRK_ARPMEM | QUIRK_FINEFX;

    /* BWSB actually gets several aspects of this wrong, but this
     * seems to be the intent. No original GDMs exist so it's not
     * likely there's a reason to simulate its mistakes here. */
    m.flow_mode = FLOW_MODE_ST3_321;

    0
}
