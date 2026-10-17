// Rust translation of src/loaders/ptm_load.c from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2026 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Poly Tracker (PTM) loader.

use super::super::common::*;
use super::super::effects::*;
use super::super::hio::*;
use super::super::period::libxmp_c2spd_to_note;
use super::super::*;
use super::common::*;
use super::sample::libxmp_load_sample;
use super::*;

const PTM_CH_MASK: u8 = 0x1f;
const PTM_NI_FOLLOW: u8 = 0x20;
const PTM_VOL_FOLLOWS: u8 = 0x80;
const PTM_FX_FOLLOWS: u8 = 0x40;

/// Translation of `struct ptm_file_header`.
#[derive(Clone, Copy)]
struct PtmFileHeader {
    name: [u8; 28],   /* Song name */
    doseof: u8,       /* 0x1a */
    vermin: u8,       /* Minor version */
    vermaj: u8,       /* Major type */
    rsvd1: u8,        /* Reserved */
    ordnum: u16,      /* Number of orders (must be even) */
    insnum: u16,      /* Number of instruments */
    patnum: u16,      /* Number of patterns */
    chnnum: u16,      /* Number of channels */
    flags: u16,       /* Flags (set to 0) */
    rsvd2: u16,       /* Reserved */
    magic: u32,       /* 'PTMF' */
    rsvd3: [u8; 16],  /* Reserved */
    chset: [u8; 32],  /* Channel settings */
    order: [u8; 256], /* Orders */
    patseg: [u16; 128],
}

const PTM_INS_NONE: u8 = 0;
const PTM_INS_SAMPLE: u8 = 1;
const PTM_INS_OPL: u8 = 2; /* unused */
const PTM_INS_MIDI: u8 = 3; /* unused */
/// `PTM_INS_TYPE(x)`
const fn ptm_ins_type(x: u8) -> u8 {
    x & 3
}
const PTM_INS_LOOP: u8 = 1 << 2;
const PTM_INS_LOOP_BIDIR: u8 = 1 << 3;
const PTM_INS_LOOP_16BIT: u8 = 1 << 4;

/// Translation of `struct ptm_instrument_header`.
#[derive(Clone, Copy, Default)]
struct PtmInstrumentHeader {
    type_: u8,         /* Sample type */
    dosname: [u8; 12], /* DOS file name */
    vol: u8,           /* Volume */
    c4spd: u16,        /* C4 speed */
    smpseg: u16,       /* Sample segment (not used) */
    smpofs: u32,       /* Sample offset */
    length: u32,       /* Length */
    loopbeg: u32,      /* Loop begin */
    loopend: u32,      /* Loop end */
    gusbeg: u32,       /* GUS begin address */
    guslps: u32,       /* GUS loop start address */
    guslpe: u32,       /* GUS loop end address */
    gusflg: u8,        /* GUS loop flags */
    rsvd1: u8,         /* Reserved */
    name: [u8; 28],    /* Instrument name */
    magic: u32,        /* 'PTMS' */
}

const MAGIC_PTMF: u32 = magic4(b'P', b'T', b'M', b'F');

pub(crate) const LIBXMP_LOADER_PTM: FormatLoader = FormatLoader {
    name: "Poly Tracker",
    test: ptm_test,
    loader: ptm_load,
};

/// Translation of `ptm_test()`.
fn ptm_test(f: &mut HioHandle<'_>, t: Option<&mut [u8]>, start: i32) -> i32 {
    hio_seek(f, start as i64 + 44, SEEK_SET);
    if hio_read32b(f) != MAGIC_PTMF {
        return -1;
    }

    hio_seek(f, start as i64, SEEK_SET);
    libxmp_read_title(f, t, 28);

    0
}

#[rustfmt::skip]
static PTM_VOL: [i32; 65] = [
    0, 5, 8, 10, 12, 14, 15, 17, 18, 20, 21, 22, 23, 25, 26,
    27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 37, 38, 39, 40,
    41, 42, 42, 43, 44, 45, 46, 46, 47, 48, 49, 49, 50, 51, 51,
    52, 53, 54, 54, 55, 56, 56, 57, 58, 58, 59, 59, 60, 61, 61,
    62, 63, 63, 64, 64,
];

/// Translation of `ptm_load()`.
fn ptm_load(m: &mut ModuleData, f: &mut HioHandle<'_>, start: i32) -> i32 {
    let mut smp_ofs = [0i32; 256];
    let mut pfh = PtmFileHeader {
        name: [0; 28],
        doseof: 0,
        vermin: 0,
        vermaj: 0,
        rsvd1: 0,
        ordnum: 0,
        insnum: 0,
        patnum: 0,
        chnnum: 0,
        flags: 0,
        rsvd2: 0,
        magic: 0,
        rsvd3: [0; 16],
        chset: [0; 32],
        order: [0; 256],
        patseg: [0; 128],
    };
    let mut pih = PtmInstrumentHeader::default();

    /* Load and convert header */

    hio_read(&mut pfh.name, 28, 1, f); /* Song name */
    pfh.doseof = hio_read8(f); /* 0x1a */
    pfh.vermin = hio_read8(f); /* Minor version */
    pfh.vermaj = hio_read8(f); /* Major type */
    pfh.rsvd1 = hio_read8(f); /* Reserved */
    pfh.ordnum = hio_read16l(f); /* Number of orders (must be even) */
    pfh.insnum = hio_read16l(f); /* Number of instruments */
    pfh.patnum = hio_read16l(f); /* Number of patterns */
    pfh.chnnum = hio_read16l(f); /* Number of channels */
    pfh.flags = hio_read16l(f); /* Flags (set to 0) */
    pfh.rsvd2 = hio_read16l(f); /* Reserved */
    pfh.magic = hio_read32b(f); /* 'PTMF' */

    if pfh.magic != MAGIC_PTMF {
        return -1;
    }

    /* Sanity check */
    if pfh.ordnum > 256 || pfh.insnum > 255 || pfh.patnum > 128 || pfh.chnnum > 32 {
        return -1;
    }

    hio_read(&mut pfh.rsvd3, 16, 1, f); /* Reserved */
    hio_read(&mut pfh.chset, 32, 1, f); /* Channel settings */
    hio_read(&mut pfh.order, 256, 1, f); /* Orders */
    for i in 0..128 {
        pfh.patseg[i] = hio_read16l(f);
    }

    if hio_error(f) != 0 {
        return -1;
    }

    let mod_ = &mut m.mod_;
    mod_.len = pfh.ordnum as i32;
    mod_.ins = pfh.insnum as i32;
    mod_.pat = pfh.patnum as i32;
    mod_.chn = pfh.chnnum as i32;
    mod_.trk = mod_.pat * mod_.chn;
    mod_.smp = mod_.ins;
    mod_.spd = 6;
    mod_.bpm = 125;
    mod_.xxo[..256].copy_from_slice(&pfh.order);

    m.c4rate = C4_NTSC_RATE;

    libxmp_copy_adjust(&mut m.mod_.name, &pfh.name, 28);
    let s = format!("Poly Tracker PTM {}.{:02x}", pfh.vermaj, pfh.vermin);
    libxmp_set_type(m, s.as_bytes());

    if libxmp_init_instrument(m) < 0 {
        return -1;
    }

    let mod_ = &mut m.mod_;
    /* Read and convert instruments and samples */

    for i in 0..mod_.ins {
        let iu = i as usize;

        pih.type_ = hio_read8(f); /* Sample type */
        hio_read(&mut pih.dosname, 12, 1, f); /* DOS file name */
        pih.vol = hio_read8(f); /* Volume */
        pih.c4spd = hio_read16l(f); /* C4 speed */
        pih.smpseg = hio_read16l(f); /* Sample segment (not used) */
        pih.smpofs = hio_read32l(f); /* Sample offset */
        pih.length = hio_read32l(f); /* Length */
        pih.loopbeg = hio_read32l(f); /* Loop begin */
        pih.loopend = hio_read32l(f); /* Loop end */
        pih.gusbeg = hio_read32l(f); /* GUS begin address */
        pih.guslps = hio_read32l(f); /* GUS loop start address */
        pih.guslpe = hio_read32l(f); /* GUS loop end address */
        pih.gusflg = hio_read8(f); /* GUS loop flags */
        pih.rsvd1 = hio_read8(f); /* Reserved */
        hio_read(&mut pih.name, 28, 1, f); /* Instrument name */
        pih.magic = hio_read32b(f); /* 'PTMS' */

        if hio_error(f) != 0 {
            return -1;
        }

        if libxmp_alloc_subinstrument(mod_, i, 1) < 0 {
            return -1;
        }

        smp_ofs[iu] = pih.smpofs as i32;
        let xxs = &mut mod_.xxs[iu];
        xxs.len = pih.length as i32;
        xxs.lps = pih.loopbeg as i32;
        xxs.lpe = pih.loopend as i32;

        if xxs.len > 0 && ptm_ins_type(pih.type_) == PTM_INS_SAMPLE {
            mod_.xxi[iu].nsm = 1;
        }

        let xxs = &mut mod_.xxs[iu];
        xxs.flg = 0;
        if pih.type_ & PTM_INS_LOOP != 0 {
            xxs.flg |= XMP_SAMPLE_LOOP;
        }
        if pih.type_ & PTM_INS_LOOP_BIDIR != 0 {
            xxs.flg |= XMP_SAMPLE_LOOP | XMP_SAMPLE_LOOP_BIDIR;
        }
        if pih.type_ & PTM_INS_LOOP_16BIT != 0 {
            xxs.flg |= XMP_SAMPLE_16BIT;
            xxs.len >>= 1;
            xxs.lps >>= 1;
            xxs.lpe >>= 1;
        }

        let sub = &mut mod_.xxi[iu].sub[0];
        sub.vol = pih.vol as i32;
        sub.pan = XMP_INST_NO_DEFAULT_PAN;
        sub.sid = i;
        pih.magic = 0;

        libxmp_instrument_name(mod_, i, &pih.name, 28);

        /* Convert C4SPD to relnote/finetune */
        let sub = &mut mod_.xxi[iu].sub[0];
        libxmp_c2spd_to_note(pih.c4spd as i32, &mut sub.xpo, &mut sub.fin);
    }

    if libxmp_init_pattern(mod_) < 0 {
        return -1;
    }

    /* Read patterns */

    for i in 0..mod_.pat {
        /* channel control to prevent infinite loop in pattern reading */
        /* addresses fuzz bug reported by Lionel Debroux in 20161223 */
        let mut chn_ctrl = [false; 32];

        if pfh.patseg[i as usize] == 0 {
            continue;
        }

        if libxmp_alloc_pattern_tracks(mod_, i, 64) < 0 {
            return -1;
        }

        hio_seek(
            f,
            start as i64 + 16 * pfh.patseg[i as usize] as i64,
            SEEK_SET,
        );
        let mut r = 0;

        while r < 64 {
            let b = hio_read8(f);
            if b == 0 {
                r += 1;
                chn_ctrl = [false; 32];
                continue;
            }

            let c = (b & PTM_CH_MASK) as i32;
            if chn_ctrl[c as usize] {
                /* uh-oh, something wrong happened */
                return -1;
            }
            /* mark this channel as read */
            chn_ctrl[c as usize] = true;

            if c >= mod_.chn {
                continue;
            }

            let event = mod_.event_mut(i, c, r);
            if b & PTM_NI_FOLLOW != 0 {
                let mut n = hio_read8(f);
                match n {
                    255 => {
                        n = 0;
                        /* Empty note */
                    }
                    254 => {
                        n = XMP_KEY_OFF;
                        /* Key off */
                    }
                    _ => {
                        n = n.wrapping_add(12);
                    }
                }
                event.note = n;
                event.ins = hio_read8(f);
            }
            if b & PTM_FX_FOLLOWS != 0 {
                event.fxt = hio_read8(f);
                event.fxp = hio_read8(f);

                if event.fxt > 0x17 {
                    event.fxt = 0;
                    event.fxp = 0;
                }

                match event.fxt {
                    0x0d => {
                        /* Break (hex parameter) */
                        event.fxt = FX_IT_BREAK;
                    }
                    0x0e => {
                        /* Extended effect */
                        if msn(event.fxp) == 0x8 {
                            /* Pan set */
                            event.fxt = FX_SETPAN;
                            event.fxp = (lsn(event.fxp) << 4) as u8;
                        }
                    }
                    0x10 => {
                        /* Set global volume */
                        event.fxt = FX_GLOBALVOL;
                    }
                    0x11 => {
                        /* Multi retrig */
                        event.fxt = FX_MULTI_RETRIG;
                    }
                    0x12 => {
                        /* Fine vibrato */
                        event.fxt = FX_FINE_VIBRATO;
                    }
                    0x13 => {
                        /* Note slide down */
                        event.fxt = FX_NSLIDE_DN;
                    }
                    0x14 => {
                        /* Note slide up */
                        event.fxt = FX_NSLIDE_UP;
                    }
                    0x15 => {
                        /* Note slide down + retrig */
                        event.fxt = FX_NSLIDE_R_DN;
                    }
                    0x16 => {
                        /* Note slide up + retrig */
                        event.fxt = FX_NSLIDE_R_UP;
                    }
                    0x17 => {
                        /* Reverse sample */
                        event.fxt = 0;
                        event.fxp = 0;
                    }
                    _ => {}
                }
            }
            if b & PTM_VOL_FOLLOWS != 0 {
                event.vol = hio_read8(f).wrapping_add(1);
            }
        }
    }

    for i in 0..m.mod_.smp as usize {
        if m.mod_.xxi[i].nsm == 0 {
            continue;
        }

        if m.mod_.xxs[i].len == 0 {
            continue;
        }

        hio_seek(f, start as i64 + smp_ofs[i] as i64, SEEK_SET);
        let smpctl = m.smpctl;
        if libxmp_load_sample(
            smpctl,
            Some(f),
            SAMPLE_FLAG_8BDIFF,
            &mut m.mod_.xxs[i],
            None,
        ) < 0
        {
            return -1;
        }
    }

    m.vol_table = Some(&PTM_VOL);

    let mod_ = &mut m.mod_;
    for i in 0..mod_.chn as usize {
        mod_.xxc[i].pan = (pfh.chset[i] as i32) << 4;
    }

    m.quirk |= QUIRKS_ST3;
    /* Has none of ST3's loop quirks; loop jumps unset prior breaks.
     * TODO: pattern jump is handled every tick, causing some strange bugs:
     * - At speed 1, loop jump unsets position jump (FLOW_LOOP_UNSET_JUMP),
     *   but at higher speeds, position jump overwrites the loop jump.
     * - At speed 1, Bxx Dyy jumps to pattern X at row Y, but at higher
     *   speeds, position jump overwrites the break row with 0.
     */
    m.flow_mode = FLOW_LOOP_GLOBAL | FLOW_LOOP_UNSET_BREAK;
    m.read_event_type = READ_EVENT_ST3;

    0
}
