// Rust translation of src/loaders/imf_load.c from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2026 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

/* Loader for Imago Orpheus modules based on the format description
 * written by Lutz Roeder.
 */

//! The Imago Orpheus (IMF) loader.

use super::super::common::*;
use super::super::effects::*;
use super::super::hio::*;
use super::super::period::libxmp_c2spd_to_note;
use super::super::*;
use super::common::*;
use super::sample::libxmp_load_sample;
use super::*;

const IMF_EOR: u8 = 0x00;
const IMF_CH_MASK: u8 = 0x1f;
const IMF_NI_FOLLOW: u8 = 0x20;
const IMF_FX_FOLLOWS: u8 = 0x80;
const IMF_F2_FOLLOWS: u8 = 0x40;

/* Sample flags */
const IMF_SAMPLE_LOOP: u8 = 0x01;
const IMF_SAMPLE_BIDI: u8 = 0x02;
const IMF_SAMPLE_16BIT: u8 = 0x04;
const IMF_SAMPLE_DEFPAN: u8 = 0x08;

/// Translation of `struct imf_channel`.
#[derive(Clone, Copy, Default)]
struct ImfChannel {
    name: [u8; 12], /* Channelname (ASCIIZ-String, max 11 chars) */
    status: u8,     /* Channel status */
    pan: u8,        /* Pan positions */
    chorus: u8,     /* Default chorus */
    reverb: u8,     /* Default reverb */
}

/// Translation of `struct imf_header`.
#[derive(Clone, Copy)]
struct ImfHeader {
    name: [u8; 32], /* Songname (ASCIIZ-String, max. 31 chars) */
    len: u16,       /* Number of orders saved */
    pat: u16,       /* Number of patterns saved */
    ins: u16,       /* Number of instruments saved */
    flg: u16,       /* Module flags */
    unused1: [u8; 8],
    tpo: u8, /* Default tempo (1..255) */
    bpm: u8, /* Default beats per minute (BPM) (32..255) */
    vol: u8, /* Default mastervolume (0..64) */
    amp: u8, /* Amplification factor (4..127) */
    unused2: [u8; 8],
    magic: u32,            /* 'IM10' */
    chn: [ImfChannel; 32], /* Channel settings */
    pos: [u8; 256],        /* Order list */
}

/// Translation of `struct imf_env`.
#[derive(Clone, Copy, Default)]
struct ImfEnv {
    npt: u8, /* Number of envelope points */
    sus: u8, /* Envelope sustain point */
    lps: u8, /* Envelope loop start point */
    lpe: u8, /* Envelope loop end point */
    flg: u8, /* Envelope flags */
    unused: [u8; 3],
}

/// Translation of `struct imf_instrument`.
#[derive(Clone, Copy)]
struct ImfInstrument {
    name: [u8; 32], /* Inst. name (ASCIIZ-String, max. 31 chars) */
    map: [u8; 120], /* Multisample settings */
    unused: [u8; 8],
    vol_env: [u16; 32],   /* Volume envelope settings */
    pan_env: [u16; 32],   /* Pan envelope settings */
    pitch_env: [u16; 32], /* Pitch envelope settings */
    env: [ImfEnv; 3],
    fadeout: u16, /* Fadeout rate (0...0FFFH) */
    nsm: u16,     /* Number of samples in instrument */
    magic: u32,   /* 'II10' */
}

/// Translation of `struct imf_sample`.
#[derive(Clone, Copy, Default)]
struct ImfSample {
    name: [u8; 13], /* Sample filename (12345678.ABC) */
    unused1: [u8; 3],
    len: u32,  /* Length */
    lps: u32,  /* Loop start */
    lpe: u32,  /* Loop end */
    rate: u32, /* Samplerate */
    vol: u8,   /* Default volume (0..64) */
    pan: u8,   /* Default pan (00h = Left / 80h = Middle) */
    unused2: [u8; 14],
    flg: u8, /* Sample flags */
    unused3: [u8; 5],
    ems: u16,   /* Reserved for internal usage */
    dram: u32,  /* Reserved for internal usage */
    magic: u32, /* 'IS10' */
}

const MAGIC_IM10: u32 = magic4(b'I', b'M', b'1', b'0');
const MAGIC_II10: u32 = magic4(b'I', b'I', b'1', b'0');
const MAGIC_IS10: u32 = magic4(b'I', b'S', b'1', b'0');
const MAGIC_IW10: u32 = magic4(b'I', b'W', b'1', b'0'); /* leaving all behind.imf */

pub(crate) const LIBXMP_LOADER_IMF: FormatLoader = FormatLoader {
    name: "Imago Orpheus v1.0",
    test: imf_test,
    loader: imf_load,
};

/// Translation of `imf_test()`.
fn imf_test(f: &mut HioHandle<'_>, t: Option<&mut [u8]>, start: i32) -> i32 {
    hio_seek(f, start as i64 + 60, SEEK_SET);
    if hio_read32b(f) != MAGIC_IM10 {
        return -1;
    }

    hio_seek(f, start as i64, SEEK_SET);
    libxmp_read_title(f, t, 32);

    0
}

const NONE: u8 = 0xff;

/* Effect conversion table */
static FX: [u8; 36] = [
    NONE,
    FX_S3M_SPEED,
    FX_S3M_BPM,
    FX_TONEPORTA,
    FX_TONE_VSLIDE,
    FX_VIBRATO,
    FX_VIBRA_VSLIDE,
    FX_FINE_VIBRATO,
    FX_TREMOLO,
    FX_S3M_ARPEGGIO,
    FX_SETPAN,
    FX_PANSLIDE,
    FX_VOLSET,
    FX_VOLSLIDE,
    FX_F_VSLIDE,
    FX_FINETUNE,
    FX_NSLIDE_UP,
    FX_NSLIDE_DN,
    FX_PORTA_UP,
    FX_PORTA_DN,
    FX_IMF_FPORTA_UP,
    FX_IMF_FPORTA_DN,
    FX_FLT_CUTOFF,
    FX_FLT_RESN,
    FX_OFFSET,
    NONE, /* fine offset */
    FX_KEYOFF,
    FX_MULTI_RETRIG,
    FX_TREMOR,
    FX_JUMP,
    FX_BREAK,
    FX_GLOBALVOL,
    FX_GVOL_SLIDE,
    FX_EXTENDED,
    FX_CHORUS,
    FX_REVERB,
];

/* Effect translation */
/// Translation of `xlat_fx()`.
fn xlat_fx(_c: i32, fxt: &mut u8, fxp: &mut u8) {
    let h = msn(*fxp);
    let l = lsn(*fxp) as u8;

    if *fxt as usize >= FX.len() {
        *fxt = 0;
        *fxp = 0;
        return;
    }

    *fxt = FX[*fxt as usize];
    match *fxt {
        FX_EXTENDED => {
            /* Extended effects */
            match h {
                0x1 /* Set filter */
                | 0x2 /* Undefined */
                | 0x4 /* Undefined */
                | 0x6 /* Undefined */
                | 0x7 /* Undefined */
                | 0x9 /* Undefined */
                | 0xe /* Ignore envelope */
                | 0xf /* Invert loop */ => {
                    *fxp = 0;
                    *fxt = 0;
                }
                0x3 => {
                    /* Glissando */
                    *fxp = l | ((EX_GLISS << 4) as u8);
                }
                0x5 => {
                    /* Vibrato waveform */
                    *fxp = l | ((EX_VIBRATO_WF << 4) as u8);
                }
                0x8 => {
                    /* Tremolo waveform */
                    *fxp = l | ((EX_TREMOLO_WF << 4) as u8);
                }
                0xa => {
                    /* Pattern loop */
                    *fxp = l | ((EX_PATTERN_LOOP << 4) as u8);
                }
                0xb => {
                    /* Pattern delay */
                    *fxp = l | ((EX_PATT_DELAY << 4) as u8);
                }
                0xc => {
                    if l == 0 {
                        *fxt = 0;
                        *fxp = 0;
                    }
                }
                _ => {}
            }
        }
        NONE => {
            /* No effect */
            *fxt = 0;
            *fxp = 0;
        }
        _ => {}
    }
}

/// Translation of `imf_load()`.
fn imf_load(m: &mut ModuleData, f: &mut HioHandle<'_>, _start: i32) -> i32 {
    let mut dummy = XmpEvent::default();
    let mut ih = ImfHeader {
        name: [0; 32],
        len: 0,
        pat: 0,
        ins: 0,
        flg: 0,
        unused1: [0; 8],
        tpo: 0,
        bpm: 0,
        vol: 0,
        amp: 0,
        unused2: [0; 8],
        magic: 0,
        chn: [ImfChannel::default(); 32],
        pos: [0; 256],
    };
    let mut ii = ImfInstrument {
        name: [0; 32],
        map: [0; 120],
        unused: [0; 8],
        vol_env: [0; 32],
        pan_env: [0; 32],
        pitch_env: [0; 32],
        env: [ImfEnv::default(); 3],
        fadeout: 0,
        nsm: 0,
        magic: 0,
    };
    let mut is = ImfSample::default();

    /* Load and convert header */
    hio_read(&mut ih.name, 32, 1, f);
    ih.len = hio_read16l(f);
    ih.pat = hio_read16l(f);
    ih.ins = hio_read16l(f);
    ih.flg = hio_read16l(f);
    hio_read(&mut ih.unused1, 8, 1, f);
    ih.tpo = hio_read8(f);
    ih.bpm = hio_read8(f);
    ih.vol = hio_read8(f);
    ih.amp = hio_read8(f);
    hio_read(&mut ih.unused2, 8, 1, f);
    ih.magic = hio_read32b(f);

    /* Sanity check */
    if ih.len > 256 || ih.pat > 256 || ih.ins > 255 {
        return -1;
    }

    for i in 0..32 {
        hio_read(&mut ih.chn[i].name, 12, 1, f);
        ih.chn[i].chorus = hio_read8(f);
        ih.chn[i].reverb = hio_read8(f);
        ih.chn[i].pan = hio_read8(f);
        ih.chn[i].status = hio_read8(f);
    }

    if hio_read(&mut ih.pos, 256, 1, f) < 1 {
        return -1;
    }

    if ih.magic != MAGIC_IM10 {
        return -1;
    }

    libxmp_copy_adjust(&mut m.mod_.name, &ih.name, 32);

    let mod_ = &mut m.mod_;
    mod_.len = ih.len as i32;
    mod_.ins = ih.ins as i32;
    mod_.smp = 1024;
    mod_.pat = ih.pat as i32;

    if ih.flg & 0x01 != 0 {
        m.period_type = PERIOD_LINEAR;
    }

    mod_.spd = ih.tpo as i32;
    mod_.bpm = ih.bpm as i32;

    libxmp_set_type(m, b"Imago Orpheus 1.0 IMF");

    let mod_ = &mut m.mod_;
    mod_.chn = 0;
    for i in 0..32 {
        /* 0=enabled; 1=muted, but still processed; 2=disabled.*/
        if ih.chn[i].status >= 2 {
            continue;
        }

        mod_.chn = i as i32 + 1;
        mod_.xxc[i].pan = ih.chn[i].pan as i32;
        /* (upstream keeps the channels' chorus and reverb here under
         * #if 0, as a FIXME) */
    }

    mod_.trk = mod_.pat * mod_.chn;

    let len = mod_.len as usize;
    mod_.xxo[..len].copy_from_slice(&ih.pos[..len]);
    for i in 0..len {
        if mod_.xxo[i] == 0xff {
            mod_.xxo[i] -= 1;
        }
    }

    m.c4rate = C4_NTSC_RATE;

    let mod_ = &mut m.mod_;
    if libxmp_init_pattern(mod_) < 0 {
        return -1;
    }

    /* Read patterns */

    for i in 0..mod_.pat {
        let mut pat_len = hio_read16l(f) as i32 - 4;

        let rows = hio_read16l(f) as i32;

        /* Sanity check */
        if rows > 256 {
            return -1;
        }

        if libxmp_alloc_pattern_tracks(mod_, i, rows) < 0 {
            return -1;
        }

        let mut r = 0;

        loop {
            pat_len -= 1;
            if pat_len < 0 {
                break;
            }
            let b = hio_read8(f);

            if b == IMF_EOR {
                r += 1;
                continue;
            }

            /* Sanity check */
            if r >= rows {
                return -1;
            }

            let c = (b & IMF_CH_MASK) as i32;
            let event = if c >= mod_.chn {
                &mut dummy
            } else {
                mod_.event_mut(i, c, r)
            };

            if b & IMF_NI_FOLLOW != 0 {
                let mut n = hio_read8(f);
                match n {
                    255 => {
                        /* No note */
                        n = 0;
                    }
                    160 => {
                        /* Key off */
                        n = XMP_KEY_OFF;
                    }
                    _ => {
                        n = (13 + 12 * msn(n) + lsn(n)) as u8;
                    }
                }

                event.note = n;
                event.ins = hio_read8(f);
                pat_len -= 2;
            }
            if b & IMF_FX_FOLLOWS != 0 {
                event.fxt = hio_read8(f);
                event.fxp = hio_read8(f);
                xlat_fx(c, &mut event.fxt, &mut event.fxp);
                pat_len -= 2;
            }
            if b & IMF_F2_FOLLOWS != 0 {
                event.f2t = hio_read8(f);
                event.f2p = hio_read8(f);
                xlat_fx(c, &mut event.f2t, &mut event.f2p);
                pat_len -= 2;
            }
        }
    }

    if libxmp_init_instrument(m) < 0 {
        return -1;
    }

    /* Read and convert instruments and samples */

    let mut smp_num = 0usize;
    for i in 0..m.mod_.ins {
        let iu = i as usize;

        hio_read(&mut ii.name, 32, 1, f);
        ii.name[31] = 0;
        hio_read(&mut ii.map, 120, 1, f);
        hio_read(&mut ii.unused, 8, 1, f);
        for j in 0..32 {
            ii.vol_env[j] = hio_read16l(f);
        }
        for j in 0..32 {
            ii.pan_env[j] = hio_read16l(f);
        }
        for j in 0..32 {
            ii.pitch_env[j] = hio_read16l(f);
        }
        for j in 0..3 {
            ii.env[j].npt = hio_read8(f);
            ii.env[j].sus = hio_read8(f);
            ii.env[j].lps = hio_read8(f);
            ii.env[j].lpe = hio_read8(f);
            ii.env[j].flg = hio_read8(f);
            hio_read(&mut ii.env[j].unused, 3, 1, f);
        }
        ii.fadeout = hio_read16l(f);
        ii.nsm = hio_read16l(f);
        ii.magic = hio_read32b(f);

        /* Sanity check */
        if ii.nsm > 255 {
            return -1;
        }

        /* Imago Orpheus may emit blank instruments with a signature
         * of four nuls. Found in "leaving all behind.imf" by Karsten Koch. */
        if ii.magic != MAGIC_II10 && ii.magic != 0 {
            return -2;
        }

        let mod_ = &mut m.mod_;
        mod_.xxi[iu].nsm = ii.nsm as i32;

        if mod_.xxi[iu].nsm > 0 && libxmp_alloc_subinstrument(mod_, i, ii.nsm as i32) < 0 {
            return -1;
        }

        let xxi = &mut mod_.xxi[iu];
        strncpy(&mut xxi.name, &ii.name, 31);
        xxi.name[31] = b'\0';

        for j in 0..108 {
            xxi.map[j + 12].ins = ii.map[j];
        }

        xxi.aei.npt = ii.env[0].npt as i32;
        xxi.aei.sus = ii.env[0].sus as i32;
        xxi.aei.lps = ii.env[0].lps as i32;
        xxi.aei.lpe = ii.env[0].lpe as i32;
        xxi.aei.flg = if ii.env[0].flg & 0x01 != 0 {
            XMP_ENVELOPE_ON
        } else {
            0
        };
        xxi.aei.flg |= if ii.env[0].flg & 0x02 != 0 {
            XMP_ENVELOPE_SUS
        } else {
            0
        };
        xxi.aei.flg |= if ii.env[0].flg & 0x04 != 0 {
            XMP_ENVELOPE_LOOP
        } else {
            0
        };

        /* Sanity check */
        if xxi.aei.npt > 16 {
            return -1;
        }

        for j in 0..xxi.aei.npt as usize {
            xxi.aei.data[j * 2] = ii.vol_env[j * 2] as i16;
            xxi.aei.data[j * 2 + 1] = ii.vol_env[j * 2 + 1] as i16;
        }

        for j in 0..ii.nsm as usize {
            hio_read(&mut is.name, 13, 1, f);
            hio_read(&mut is.unused1, 3, 1, f);
            is.len = hio_read32l(f);
            is.lps = hio_read32l(f);
            is.lpe = hio_read32l(f);
            is.rate = hio_read32l(f);
            is.vol = hio_read8(f);
            is.pan = hio_read8(f);
            hio_read(&mut is.unused2, 14, 1, f);
            is.flg = hio_read8(f);
            hio_read(&mut is.unused3, 5, 1, f);
            is.ems = hio_read16l(f);
            is.dram = hio_read32l(f);
            is.magic = hio_read32b(f);

            if is.magic != MAGIC_IS10 && is.magic != MAGIC_IW10 {
                return -1;
            }

            /* Sanity check */
            if is.len > 0x100000 || is.lps > 0x100000 || is.lpe > 0x100000 {
                return -1;
            }

            // FIXME (upstream): the instruments' samples aren't checked
            // against the 1024 allocated, which upstream overflows.
            if smp_num >= m.mod_.xxs.len() {
                return -1;
            }

            let mod_ = &mut m.mod_;
            let sub = &mut mod_.xxi[iu].sub[j];
            let xxs = &mut mod_.xxs[smp_num];

            sub.sid = smp_num as i32;
            sub.vol = is.vol as i32;
            sub.pan = if is.flg & IMF_SAMPLE_DEFPAN != 0 {
                is.pan as i32
            } else {
                XMP_INST_NO_DEFAULT_PAN
            };
            xxs.len = is.len as i32;
            xxs.lps = is.lps as i32;
            xxs.lpe = is.lpe as i32;
            xxs.flg = 0;

            if is.flg & IMF_SAMPLE_LOOP != 0 {
                xxs.flg |= XMP_SAMPLE_LOOP;
            }
            if is.flg & IMF_SAMPLE_BIDI != 0 {
                xxs.flg |= XMP_SAMPLE_LOOP_BIDIR;
            }
            if is.flg & IMF_SAMPLE_16BIT != 0 {
                xxs.flg |= XMP_SAMPLE_16BIT;
                xxs.len >>= 1;
                xxs.lps >>= 1;
                xxs.lpe >>= 1;
            }

            libxmp_c2spd_to_note(is.rate as i32, &mut sub.xpo, &mut sub.fin);

            let len = xxs.len;
            let sid = sub.sid as usize;
            smp_num += 1;
            if len <= 0 {
                continue;
            }

            let smpctl = m.smpctl;
            if libxmp_load_sample(smpctl, Some(f), 0, &mut m.mod_.xxs[sid], None) < 0 {
                return -1;
            }
        }
    }

    let mod_ = &mut m.mod_;
    mod_.smp = smp_num as i32;
    // FIXME (upstream): the samples are reallocated to their number with
    // realloc(), which for none frees them and returns NULL (with glibc),
    // failing the load.
    if mod_.smp == 0 {
        mod_.xxs = Vec::new();
        return -1;
    }
    mod_.xxs.truncate(smp_num);
    m.xtra.truncate(smp_num);

    m.c4rate = C4_NTSC_RATE;
    m.quirk |= QUIRK_FILTER | QUIRK_MARKER | QUIRK_RSTCHN | QUIRK_ARPMEM;
    m.flow_mode = FLOW_MODE_ORPHEUS;
    m.read_event_type = READ_EVENT_ST3;

    m.gvol = ih.vol as i32;
    m.mvol = ih.amp as i32;
    m.mvolbase = 48;
    clamp(&mut m.mvol, 4, 127);

    0
}
