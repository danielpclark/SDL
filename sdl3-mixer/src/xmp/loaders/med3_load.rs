// Rust translation of src/loaders/med3_load.c from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2026 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

/*
 * MED 2.00 is in Fish disk #349 and has a couple of demo modules, get it
 * from ftp://ftp.funet.fi/pub/amiga/fish/301-400/ff349
 */

//! The MED 2.00 (MED3) loader.

use super::super::common::*;
use super::super::effects::*;
use super::super::hio::*;
use super::super::*;
use super::common::*;
use super::med::*;
use super::mmd_common::{med_load_external_instrument, mmd_convert_tempo};
use super::sample::libxmp_load_sample;
use super::*;

const MAGIC_MED3: u32 = magic4(b'M', b'E', b'D', 3);

pub(crate) const LIBXMP_LOADER_MED3: FormatLoader = FormatLoader {
    name: "MED 2.00 MED3",
    test: med3_test,
    loader: med3_load,
};

/// Translation of `med3_test()`.
fn med3_test(f: &mut HioHandle<'_>, t: Option<&mut [u8]>, _start: i32) -> i32 {
    if hio_read32b(f) != MAGIC_MED3 {
        return -1;
    }

    libxmp_read_title(f, t, 0);

    0
}

const MASK: u32 = 0x80000000;

const M0F_LINEMSK0F: u8 = 0x01;
const M0F_LINEMSK1F: u8 = 0x02;
const M0F_FXMSK0F: u8 = 0x04;
const M0F_FXMSK1F: u8 = 0x08;
const M0F_LINEMSK00: u8 = 0x10;
const M0F_LINEMSK10: u8 = 0x20;
const M0F_FXMSK00: u8 = 0x40;
const M0F_FXMSK10: u8 = 0x80;

/*
 * From the MED 2.00 file loading/saving routines by Teijo Kinnunen, 1990
 */

/// Translation of `get_nibble()`.
fn get_nibble(mem: &[u8], nbnum: &mut u16) -> u8 {
    let mloc = mem[(*nbnum / 2) as usize];

    let res = if *nbnum & 0x1 != 0 {
        mloc & 0x0f
    } else {
        mloc >> 4
    };
    *nbnum = nbnum.wrapping_add(1);

    res
}

/// Translation of `get_nibbles()`.
fn get_nibbles(mem: &[u8], nbnum: &mut u16, mut nbs: u8) -> u16 {
    let mut res: u16 = 0;

    while nbs > 0 {
        nbs -= 1;
        res <<= 4;
        res |= get_nibble(mem, nbnum) as u16;
    }

    res
}

/// Translation of `unpack_block()`: `masks` are the line and effect
/// masks, `from` the packed data (of `convsz` bytes).
fn unpack_block(m: &mut ModuleData, bnum: u16, masks: [u32; 4], from: &[u8], convsz: u16) -> i32 {
    let mod_ = &mut m.mod_;
    let [mut linemsk0, mut linemsk1, mut fxmsk0, mut fxmsk1] = masks;
    let mut fromn: u16 = 0;
    let fromst = from;
    let mut nibs_left: u32 = convsz as u32 * 2;
    let trkn = mod_.chn;

    /*from += 16;*/
    let Ok(mut patbuf) = try_vec(3 * 4 * 64, 0u8) else {
        return -1;
    };
    let mut to = 0usize;

    for i in 0..64 {
        let (lmptr, fxptr) = if i < 32 {
            (&mut linemsk0, &mut fxmsk0)
        } else {
            (&mut linemsk1, &mut fxmsk1)
        };

        if *lmptr & MASK != 0 {
            if (trkn / 4) as u32 > nibs_left {
                return -1;
            }
            nibs_left -= (trkn / 4) as u32;

            let mut lmsk = get_nibbles(fromst, &mut fromn, (trkn / 4) as u8);
            lmsk <<= 16 - trkn;
            let mut tmpto = to;

            for _ in 0..trkn {
                if lmsk & 0x8000 != 0 {
                    if nibs_left < 3 {
                        return -1;
                    }
                    nibs_left -= 3;
                    patbuf[tmpto] = get_nibbles(fromst, &mut fromn, 2) as u8;
                    patbuf[tmpto + 1] = get_nibble(fromst, &mut fromn) << 4;
                }
                lmsk <<= 1;
                tmpto += 3;
            }
        }

        if *fxptr & MASK != 0 {
            if (trkn / 4) as u32 > nibs_left {
                return -1;
            }
            nibs_left -= (trkn / 4) as u32;

            let mut lmsk = get_nibbles(fromst, &mut fromn, (trkn / 4) as u8);
            lmsk <<= 16 - trkn;
            let mut tmpto = to;

            for _ in 0..trkn {
                if lmsk & 0x8000 != 0 {
                    if nibs_left < 3 {
                        return -1;
                    }
                    nibs_left -= 3;
                    patbuf[tmpto + 1] |= get_nibble(fromst, &mut fromn);
                    patbuf[tmpto + 2] = get_nibbles(fromst, &mut fromn, 2) as u8;
                }
                lmsk <<= 1;
                tmpto += 3;
            }
        }
        to += 3 * trkn as usize;
        *lmptr <<= 1;
        *fxptr <<= 1;
    }

    for i in 0..64 {
        for j in 0..4 {
            let event = mod_.event_mut(bnum as i32, j as i32, i as i32);
            let p = &patbuf[i * 12 + j * 3..];

            event.note = p[0];
            if event.note != 0 {
                event.note = event.note.wrapping_add(48);
            }
            event.ins = p[1] >> 4;
            if event.ins != 0 {
                event.ins += 1;
            }
            event.fxt = p[1] & 0x0f;
            event.fxp = p[2];

            match event.fxt {
                0x00 /* arpeggio */
                | 0x01 /* slide up */
                | 0x02 /* slide down */
                | 0x03 /* portamento */
                | 0x04 /* vibrato? */ => {}
                0x0c => {
                    /* set volume (BCD) */
                    event.fxp = (msn(event.fxp) * 10 + lsn(event.fxp)) as u8;
                }
                0x0d => {
                    /* volume slides */
                    event.fxt = FX_VOLSLIDE;
                }
                0x0f => {
                    /* tempo/break */
                    if event.fxp == 0 {
                        event.fxt = FX_BREAK;
                    } else if event.fxp == 0xff {
                        event.fxp = 0;
                        event.fxt = 0;
                        event.vol = 1;
                    } else if event.fxp == 0xfe {
                        event.fxp = 0;
                        event.fxt = 0;
                    } else if event.fxp == 0xf1 {
                        /* Retrigger once on tick 3 */
                        event.fxt = FX_EXTENDED;
                        event.fxp = ((EX_RETRIG << 4) | 3) as u8;
                    } else if event.fxp == 0xf2 {
                        /* Delay until tick 3 */
                        event.fxt = FX_EXTENDED;
                        event.fxp = ((EX_DELAY << 4) | 3) as u8;
                    } else if event.fxp == 0xf3 {
                        /* Retrigger every 2 ticks (TODO: buggy) */
                        event.fxt = FX_MED_RETRIG;
                        event.fxp = 0x02;
                    } else if event.fxp <= 0xf0 {
                        event.fxt = FX_S3M_BPM;
                        event.fxp = mmd_convert_tempo(event.fxp as i32, false, false) as u8;
                    } else {
                        event.fxp = 0;
                        event.fxt = 0;
                    }
                }
                _ => {
                    event.fxp = 0;
                    event.fxt = 0;
                }
            }
        }
    }

    0
}

/// Translation of `med3_load()`.
fn med3_load(m: &mut ModuleData, f: &mut HioHandle<'_>, _start: i32) -> i32 {
    hio_read32b(f);

    libxmp_set_type(m, b"MED 2.00 MED3");

    m.mod_.ins = 32;
    m.mod_.smp = 32;

    if libxmp_init_instrument(m) < 0 {
        return -1;
    }

    let mod_ = &mut m.mod_;
    /* read instrument names */
    for i in 0..32 {
        let mut buf = [0u8; 40];
        for j in 0..40 {
            let c = hio_read8(f);
            buf[j] = c;
            if c == 0 {
                break;
            }
        }
        libxmp_instrument_name(mod_, i, &buf, 32);
        if libxmp_alloc_subinstrument(mod_, i, 1) < 0 {
            return -1;
        }
    }

    /* read instrument volumes */
    let mut mask = hio_read32b(f);
    for i in 0..32 {
        let sub = &mut mod_.xxi[i].sub[0];
        sub.vol = if mask & MASK != 0 {
            hio_read8(f) as i32
        } else {
            0
        };
        sub.pan = XMP_INST_NO_DEFAULT_PAN;
        sub.fin = 0;
        sub.sid = i as i32;
        mask <<= 1;
    }

    /* read instrument loops */
    mask = hio_read32b(f);
    for i in 0..32 {
        mod_.xxs[i].lps = if mask & MASK != 0 {
            hio_read16b(f) as i32
        } else {
            0
        };
        mask <<= 1;
    }

    /* read instrument loop length */
    mask = hio_read32b(f);
    for i in 0..32 {
        let lsiz = if mask & MASK != 0 {
            hio_read16b(f) as u32
        } else {
            0
        };
        let xxs = &mut mod_.xxs[i];
        xxs.len = (xxs.lps as u32).wrapping_add(lsiz) as i32;
        xxs.lpe = (xxs.lps as u32).wrapping_add(lsiz) as i32;
        xxs.flg = if lsiz > 1 { XMP_SAMPLE_LOOP } else { 0 };
        mask <<= 1;
    }

    mod_.chn = 4;
    mod_.pat = hio_read16b(f) as i32;
    mod_.trk = mod_.chn * mod_.pat;

    mod_.len = hio_read16b(f) as i32;

    /* Sanity check */
    if mod_.len > 256 || mod_.pat > 256 {
        return -1;
    }

    let len = mod_.len as usize;
    hio_read(&mut mod_.xxo, 1, len, f);
    let tempo = hio_read16b(f) as i32;
    let transp = hio_read8s(f) as i32;
    let flags = hio_read8(f); /* flags */
    let sliding = hio_read16b(f); /* sliding */
    hio_read32b(f); /* jumping mask */
    hio_seek(f, 16, SEEK_CUR); /* rgb */

    mod_.spd = 6;
    mod_.bpm = mmd_convert_tempo(tempo, false, false);
    m.time_factor = MED_TIME_FACTOR;

    /* read midi channels */
    mask = hio_read32b(f);
    for _ in 0..32 {
        if mask & MASK != 0 {
            hio_read8(f);
        }
        mask <<= 1;
    }

    /* read midi programs */
    mask = hio_read32b(f);
    for _ in 0..32 {
        if mask & MASK != 0 {
            hio_read8(f);
        }
        mask <<= 1;
    }

    m.quirk |= QUIRK_RTONCE; /* FF1 */
    if sliding == 6 {
        m.quirk |= QUIRK_VSALL | QUIRK_PBALL;
    }

    let mod_ = &mut m.mod_;
    for i in 0..32 {
        mod_.xxi[i].sub[0].xpo = transp;
    }

    if libxmp_init_pattern(mod_) < 0 {
        return -1;
    }

    /* Load and convert patterns */

    for i in 0..m.mod_.pat {
        let mut conv = [0u32; 4];

        if libxmp_alloc_pattern_tracks(&mut m.mod_, i, 64) < 0 {
            return -1;
        }

        /* TODO: not clear if this should be respected. Later MED
         * formats are capable of having different track counts. */
        /*tracks =*/
        hio_read8(f);

        let b = hio_read8(f);
        let convsz = hio_read16b(f);
        let Ok(mut data) = try_vec(convsz as usize, 0u8) else {
            return -1;
        };

        if b & M0F_LINEMSK00 != 0 {
            conv[0] = 0;
        } else if b & M0F_LINEMSK0F != 0 {
            conv[0] = 0xffffffff;
        } else {
            conv[0] = hio_read32b(f);
        }

        if b & M0F_LINEMSK10 != 0 {
            conv[1] = 0;
        } else if b & M0F_LINEMSK1F != 0 {
            conv[1] = 0xffffffff;
        } else {
            conv[1] = hio_read32b(f);
        }

        if b & M0F_FXMSK00 != 0 {
            conv[2] = 0;
        } else if b & M0F_FXMSK0F != 0 {
            conv[2] = 0xffffffff;
        } else {
            conv[2] = hio_read32b(f);
        }

        if b & M0F_FXMSK10 != 0 {
            conv[3] = 0;
        } else if b & M0F_FXMSK1F != 0 {
            conv[3] = 0xffffffff;
        } else {
            conv[3] = hio_read32b(f);
        }

        if hio_read(&mut data, 1, convsz as usize, f) != convsz as usize {
            return -1;
        }

        if unpack_block(m, i as u16, conv, &data, convsz) < 0 {
            return -1;
        }
    }

    /* Load samples */

    mask = hio_read32b(f);
    for i in 0..32 {
        let skip = !mask & MASK != 0;
        mask <<= 1;
        if skip {
            continue;
        }

        if !flags & FLAG_INSTRSATT != 0 {
            /* Song file */
            if med_load_external_instrument(f, m, i) != 0 {
                return -1;
            }
            continue;
        }

        /* Module file */
        let iu = i as usize;
        m.mod_.xxi[iu].nsm = 1;
        m.mod_.xxs[iu].len = hio_read32b(f) as i32;

        if m.mod_.xxs[iu].len == 0 {
            m.mod_.xxi[iu].nsm = 0;
        }

        if hio_read16b(f) != 0 {
            /* type */
            continue;
        }

        let smpctl = m.smpctl;
        if libxmp_load_sample(smpctl, Some(f), 0, &mut m.mod_.xxs[iu], None) < 0 {
            return -1;
        }
    }

    0
}
