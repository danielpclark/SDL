// Rust translation of src/loaders/it_load.c and src/loaders/it.h from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2026 Claudio Matsuoka and Hipolito Carraro Jr (it_load.c)
// Copyright (C) 1996-2025 Claudio Matsuoka and Hipolito Carraro Jr (it.h)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Impulse Tracker IT loader.

use super::super::common::*;
use super::super::dataio::*;
use super::super::effects::*;
use super::super::hio::*;
use super::super::period::libxmp_c2spd_to_note;
use super::super::*;
use super::common::*;
use super::itsex::{itsex_decompress16, itsex_decompress8};
use super::sample::libxmp_load_sample;
use super::*;

/* IT flags */
pub(crate) const IT_STEREO: u16 = 0x01;
pub(crate) const IT_VOL_OPT: u16 = 0x02; /* Not recognized */
pub(crate) const IT_USE_INST: u16 = 0x04;
pub(crate) const IT_LINEAR_FREQ: u16 = 0x08;
pub(crate) const IT_OLD_FX: u16 = 0x10;
pub(crate) const IT_LINK_GXX: u16 = 0x20;
pub(crate) const IT_MIDI_WHEEL: u16 = 0x40;
pub(crate) const IT_MIDI_CONFIG: u16 = 0x80;

/* IT special */
pub(crate) const IT_HAS_MSG: u16 = 0x01;
pub(crate) const IT_EDIT_HISTORY: u16 = 0x02;
pub(crate) const IT_HIGHLIGHTS: u16 = 0x04;
pub(crate) const IT_SPEC_MIDICFG: u16 = 0x08;

/* IT instrument flags */
pub(crate) const IT_INST_SAMPLE: u8 = 0x01;
pub(crate) const IT_INST_16BIT: u8 = 0x02;
pub(crate) const IT_INST_STEREO: u8 = 0x04;
pub(crate) const IT_INST_LOOP: u8 = 0x10;
pub(crate) const IT_INST_SLOOP: u8 = 0x20;
pub(crate) const IT_INST_BLOOP: u8 = 0x40;
pub(crate) const IT_INST_BSLOOP: u8 = 0x80;

/* IT sample flags */
pub(crate) const IT_SMP_SAMPLE: u8 = 0x01;
pub(crate) const IT_SMP_16BIT: u8 = 0x02;
pub(crate) const IT_SMP_STEREO: u8 = 0x04; /* unsupported */
pub(crate) const IT_SMP_COMP: u8 = 0x08; /* unsupported */
pub(crate) const IT_SMP_LOOP: u8 = 0x10;
pub(crate) const IT_SMP_SLOOP: u8 = 0x20;
pub(crate) const IT_SMP_BLOOP: u8 = 0x40;
pub(crate) const IT_SMP_BSLOOP: u8 = 0x80;

/* IT sample conversion flags */
pub(crate) const IT_CVT_SIGNED: u8 = 0x01;
pub(crate) const IT_CVT_BIGEND: u8 = 0x02; /* 'safe to ignore' according to ittech.txt */
pub(crate) const IT_CVT_DIFF: u8 = 0x04; /* Compressed sample flag */
pub(crate) const IT_CVT_BYTEDIFF: u8 = 0x08; /* 'safe to ignore' according to ittech.txt */
pub(crate) const IT_CVT_12BIT: u8 = 0x10; /* 'safe to ignore' according to ittech.txt */
pub(crate) const IT_CVT_ADPCM: u8 = 0xff; /* Special: always indicates Modplug ADPCM4 */

/* IT envelope flags */
pub(crate) const IT_ENV_ON: u8 = 0x01;
pub(crate) const IT_ENV_LOOP: u8 = 0x02;
pub(crate) const IT_ENV_SLOOP: u8 = 0x04;
pub(crate) const IT_ENV_CARRY: u8 = 0x08;
pub(crate) const IT_ENV_FILTER: u8 = 0x80;

/// Translation of `struct it_file_header`.
struct ItFileHeader {
    magic: u32,      /* 'IMPM' */
    name: [u8; 26],  /* ASCIIZ Song name */
    hilite_min: u8,  /* Pattern editor highlight */
    hilite_maj: u8,  /* Pattern editor highlight */
    ordnum: u16,     /* Number of orders (must be even) */
    insnum: u16,     /* Number of instruments */
    smpnum: u16,     /* Number of samples */
    patnum: u16,     /* Number of patterns */
    cwt: u16,        /* Tracker ID and version */
    cmwt: u16,       /* Format version */
    flags: u16,      /* Flags */
    special: u16,    /* More flags */
    gv: u8,          /* Global volume */
    mv: u8,          /* Master volume */
    is: u8,          /* Initial speed */
    it: u8,          /* Initial tempo */
    sep: u8,         /* Panning separation */
    pwd: u8,         /* Pitch wheel depth */
    msglen: u16,     /* Message length */
    msgofs: u32,     /* Message offset */
    rsvd: u32,       /* Reserved */
    chpan: [u8; 64], /* Channel pan settings */
    chvol: [u8; 64], /* Channel volume settings */
}

/// Translation of `struct it_instrument1_header`.
struct ItInstrument1Header {
    magic: u32,        /* 'IMPI' */
    dosname: [u8; 12], /* DOS filename */
    zero: u8,          /* Always zero */
    flags: u8,         /* Instrument flags */
    vls: u8,           /* Volume loop start */
    vle: u8,           /* Volume loop end */
    sls: u8,           /* Sustain loop start */
    sle: u8,           /* Sustain loop end */
    fadeout: u16,      /* Fadeout (release) */
    nna: u8,           /* New note action */
    dnc: u8,           /* Duplicate note check */
    trkvers: u16,      /* Tracker version */
    nos: u8,           /* Number of samples */
    name: [u8; 26],    /* ASCIIZ Instrument name */
    keys: [u8; 240],
    epoint: [u8; 200],
    enode: [u8; 50],
}

/// Translation of `struct it_instrument2_header`.
struct ItInstrument2Header {
    magic: u32,        /* 'IMPI' */
    dosname: [u8; 12], /* DOS filename */
    zero: u8,          /* Always zero */
    nna: u8,           /* New Note Action */
    dct: u8,           /* Duplicate Check Type */
    dca: u8,           /* Duplicate Check Action */
    fadeout: u16,
    pps: u8,        /* Pitch-Pan Separation */
    ppc: u8,        /* Pitch-Pan Center */
    gbv: u8,        /* Global Volume */
    dfp: u8,        /* Default pan */
    rv: u8,         /* Random volume variation */
    rp: u8,         /* Random pan variation */
    trkvers: u16,   /* Not used: tracked version */
    nos: u8,        /* Not used: number of samples */
    name: [u8; 26], /* ASCIIZ Instrument name */
    ifc: u8,        /* Initial filter cutoff */
    ifr: u8,        /* Initial filter resonance */
    mch: u8,        /* MIDI channel */
    mpr: u8,        /* MIDI program */
    mbnk: u16,      /* MIDI bank */
    keys: [u8; 240],
}

/// Translation of `struct it_envelope_node`.
#[derive(Clone, Copy, Default)]
struct ItEnvelopeNode {
    y: i8,
    x: u16,
}

/// Translation of `struct it_envelope`.
#[derive(Default)]
struct ItEnvelope {
    flg: u8, /* Flags */
    num: u8, /* Number of node points */
    lpb: u8, /* Loop beginning */
    lpe: u8, /* Loop end */
    slb: u8, /* Sustain loop beginning */
    sle: u8, /* Sustain loop end */
    node: [ItEnvelopeNode; 25],
}

/// Translation of `struct it_sample_header`.
#[derive(Default)]
struct ItSampleHeader {
    magic: u32,        /* 'IMPS' */
    dosname: [u8; 12], /* DOS filename */
    zero: u8,          /* Always zero */
    gvl: u8,           /* Global volume for instrument */
    flags: u8,         /* Sample flags */
    vol: u8,           /* Volume */
    name: [u8; 26],    /* ASCIIZ sample name */
    convert: u8,       /* Sample flags */
    dfp: u8,           /* Default pan */
    length: u32,       /* Length */
    loopbeg: u32,      /* Loop begin */
    loopend: u32,      /* Loop end */
    c5spd: u32,        /* C 5 speed */
    sloopbeg: u32,     /* SusLoop begin */
    sloopend: u32,     /* SusLoop end */
    sample_ptr: u32,   /* Sample pointer */
    vis: u8,           /* Vibrato speed */
    vid: u8,           /* Vibrato depth */
    vir: u8,           /* Vibrato rate */
    vit: u8,           /* Vibrato waveform */
}

const MAGIC_IMPM: u32 = magic4(b'I', b'M', b'P', b'M');
const MAGIC_IMPI: u32 = magic4(b'I', b'M', b'P', b'I');
const MAGIC_IMPS: u32 = magic4(b'I', b'M', b'P', b'S');

const TEMP_BUFFER_LEN: usize = 65536;

pub(crate) const LIBXMP_LOADER_IT: FormatLoader = FormatLoader {
    name: "Impulse Tracker",
    test: it_test,
    loader: it_load,
};

/// Translation of `it_test()`.
fn it_test(f: &mut HioHandle<'_>, t: Option<&mut [u8]>, _start: i32) -> i32 {
    if hio_read32b(f) != MAGIC_IMPM {
        return -1;
    }

    libxmp_read_title(f, t, 26);

    0
}

const FX_NONE: u8 = 0xff;
const FX_XTND: u8 = 0xfe;
const L_CHANNELS: usize = 64;

#[rustfmt::skip]
const FX: [u8; 32] = [
    /*   */ FX_NONE,
    /* A */ FX_S3M_SPEED,
    /* B */ FX_JUMP,
    /* C */ FX_IT_BREAK,
    /* D */ FX_VOLSLIDE,
    /* E */ FX_PORTA_DN,
    /* F */ FX_PORTA_UP,
    /* G */ FX_TONEPORTA,
    /* H */ FX_VIBRATO,
    /* I */ FX_TREMOR,
    /* J */ FX_S3M_ARPEGGIO,
    /* K */ FX_VIBRA_VSLIDE,
    /* L */ FX_TONE_VSLIDE,
    /* M */ FX_TRK_VOL,
    /* N */ FX_TRK_VSLIDE,
    /* O */ FX_OFFSET,
    /* P */ FX_IT_PANSLIDE,
    /* Q */ FX_MULTI_RETRIG,
    /* R */ FX_TREMOLO,
    /* S */ FX_XTND,
    /* T */ FX_IT_BPM,
    /* U */ FX_FINE_VIBRATO,
    /* V */ FX_GLOBALVOL,
    /* W */ FX_GVOL_SLIDE,
    /* X */ FX_SETPAN,
    /* Y */ FX_PANBRELLO,
    /* Z */ FX_MACRO,
    /* ? */ FX_NONE,
    /* / */ FX_MACROSMOOTH,
    /* ? */ FX_NONE,
    /* ? */ FX_NONE,
    /* ? */ FX_NONE,
];

/// Translation of `xlat_fx()`.
fn xlat_fx(c: usize, e: &mut XmpEvent, last_fxp: &mut [u8; 64], new_fx: bool) {
    let mut h = msn(e.fxp) as u8;
    let mut l = lsn(e.fxp) as u8;

    e.fxt = FX[e.fxt as usize];
    match e.fxt {
        FX_XTND => {
            /* Extended effect */
            e.fxt = FX_EXTENDED;

            if h == 0 && e.fxp == 0 {
                e.fxp = last_fxp[c];
                h = msn(e.fxp) as u8;
                l = lsn(e.fxp) as u8;
            } else {
                last_fxp[c] = e.fxp;
            }

            match h {
                0x1 => {
                    /* Glissando */
                    e.fxp = 0x30 | l;
                }
                0x2 => {
                    /* Finetune -- not supported */
                    e.fxt = 0;
                    e.fxp = 0;
                }
                0x3 => {
                    /* Vibrato wave */
                    e.fxp = 0x40 | l;
                }
                0x4 => {
                    /* Tremolo wave */
                    e.fxp = 0x70 | l;
                }
                0x5 => {
                    /* Panbrello wave */
                    if l <= 3 {
                        e.fxt = FX_PANBRELLO_WF;
                        e.fxp = l;
                    } else {
                        e.fxt = 0;
                        e.fxp = 0;
                    }
                }
                0x6 => {
                    /* Pattern delay */
                    e.fxp = 0xe0 | l;
                }
                0x7 => {
                    /* Instrument functions */
                    e.fxt = FX_IT_INSTFUNC;
                    e.fxp &= 0x0f;
                }
                0x8 => {
                    /* Set pan position */
                    e.fxt = FX_SETPAN;
                    e.fxp = l << 4;
                }
                0x9 => {
                    if l == 0 || l == 1 {
                        /* 0x91 = set surround */
                        e.fxt = FX_SURROUND;
                        e.fxp = l;
                    } else if l == 0xe || l == 0xf {
                        /* 0x9f Play reverse (MPT) */
                        e.fxt = FX_REVERSE;
                        e.fxp = l - 0xe;
                    }
                }
                0xa => {
                    /* High offset */
                    e.fxt = FX_HIOFFSET;
                    e.fxp = l;
                }
                0xb => {
                    /* Pattern loop */
                    e.fxp = 0x60 | l;
                }
                0xc | 0xd => {
                    /* Note cut */
                    /* Note delay */
                    e.fxp = l;
                    if e.fxp == 0 {
                        e.fxp += 1; /* SD0 and SC0 become SD1 and SC1 */
                    }
                    e.fxp |= h << 4;
                }
                0xe => {
                    /* Pattern row delay */
                    e.fxt = FX_IT_ROWDELAY;
                    e.fxp = l;
                }
                0xf => {
                    /* Set parametered macro */
                    e.fxt = FX_MACRO_SET;
                    e.fxp = l;
                }
                _ => {
                    e.fxt = 0;
                    e.fxp = 0;
                }
            }
        }
        FX_TREMOR => {
            if !new_fx && e.fxp != 0 {
                e.fxp = (((msn(e.fxp) + 1) << 4) | (lsn(e.fxp) + 1)) as u8;
            }
        }
        FX_GLOBALVOL => {
            if e.fxp > 0x80 {
                /* See storlek test 16 */
                e.fxt = 0;
                e.fxp = 0;
            }
        }
        FX_NONE => {
            /* No effect */
            e.fxt = 0;
            e.fxp = 0;
        }
        _ => {}
    }
}

/// Translation of `xlat_volfx()`.
fn xlat_volfx(event: &mut XmpEvent) {
    let b = event.vol as i32;
    event.vol = 0;

    if b <= 0x40 {
        event.vol = (b + 1) as u8;
    } else if (65..=74).contains(&b) {
        /* A */
        event.f2t = FX_F_VSLIDE_UP_2;
        event.f2p = (b - 65) as u8;
    } else if (75..=84).contains(&b) {
        /* B */
        event.f2t = FX_F_VSLIDE_DN_2;
        event.f2p = (b - 75) as u8;
    } else if (85..=94).contains(&b) {
        /* C */
        event.f2t = FX_VSLIDE_UP_2;
        event.f2p = (b - 85) as u8;
    } else if (95..=104).contains(&b) {
        /* D */
        event.f2t = FX_VSLIDE_DN_2;
        event.f2p = (b - 95) as u8;
    } else if (105..=114).contains(&b) {
        /* E */
        event.f2t = FX_PORTA_DN;
        event.f2p = ((b - 105) << 2) as u8;
    } else if (115..=124).contains(&b) {
        /* F */
        event.f2t = FX_PORTA_UP;
        event.f2p = ((b - 115) << 2) as u8;
    } else if (128..=192).contains(&b) {
        /* pan */
        if b == 192 {
            event.f2p = 0xff;
        } else {
            event.f2p = ((b - 128) << 2) as u8;
        }
        event.f2t = FX_SETPAN;
    } else if (193..=202).contains(&b) {
        /* G */
        const VAL: [u8; 10] = [0x00, 0x01, 0x04, 0x08, 0x10, 0x20, 0x40, 0x60, 0x80, 0xff];
        event.f2t = FX_TONEPORTA;
        event.f2p = VAL[(b - 193) as usize];
    } else if (203..=212).contains(&b) {
        /* H */
        event.f2t = FX_VIBRATO;
        event.f2p = (b - 203) as u8;
    }
}

/// Translation of `fix_name()`.
fn fix_name(s: &mut [u8], mut l: i32) {
    /* IT names can have 0 at start of data, replace with space */
    l -= 1;
    let mut i = 0;
    while i < l {
        if s[i as usize] == 0 {
            s[i as usize] = b' ';
        }
        i += 1;
    }
    i -= 1;
    while i >= 0 && s[i as usize] == b' ' {
        if s[i as usize] == b' ' {
            s[i as usize] = 0;
        }
        i -= 1;
    }
}

/// Translation of `load_it_midi_config()`.
fn load_it_midi_config(m: &mut ModuleData, f: &mut HioHandle<'_>) -> i32 {
    let midi = m.midi.insert(Box::default());

    /* Skip global MIDI macros */
    if hio_seek(f, 9 * 32, SEEK_CUR) < 0 {
        return -1;
    }

    /* SFx macros */
    for i in 0..16 {
        if hio_read(&mut midi.param[i].data, 1, 32, f) < 32 {
            return -1;
        }
        midi.param[i].data[31] = b'\0';
    }
    /* Zxx macros */
    for i in 0..128 {
        if hio_read(&mut midi.fixed[i].data, 1, 32, f) < 32 {
            return -1;
        }
        midi.fixed[i].data[31] = b'\0';
    }
    0
}

/// Translation of `read_envelope()`.
fn read_envelope(ei: &mut XmpEnvelope, env: &mut ItEnvelope, f: &mut HioHandle<'_>) -> i32 {
    let mut buf = [0u8; 82];

    if hio_read(&mut buf, 1, 82, f) != 82 {
        return -1;
    }

    env.flg = buf[0];
    env.num = buf[1].min(25); /* Clamp to IT max */

    env.lpb = buf[2];
    env.lpe = buf[3];
    env.slb = buf[4];
    env.sle = buf[5];

    for i in 0..25 {
        env.node[i].y = buf[6 + i * 3] as i8;
        env.node[i].x = readmem16l(&buf[7 + i * 3..]);
    }

    ei.flg = if env.flg & IT_ENV_ON != 0 {
        XMP_ENVELOPE_ON
    } else {
        0
    };

    if env.flg & IT_ENV_LOOP != 0 {
        ei.flg |= XMP_ENVELOPE_LOOP;
    }

    if env.flg & IT_ENV_SLOOP != 0 {
        ei.flg |= XMP_ENVELOPE_SUS | XMP_ENVELOPE_SLOOP;
    }

    if env.flg & IT_ENV_CARRY != 0 {
        ei.flg |= XMP_ENVELOPE_CARRY;
    }

    ei.npt = env.num as i32;
    ei.sus = env.slb as i32;
    ei.sue = env.sle as i32;
    ei.lps = env.lpb as i32;
    ei.lpe = env.lpe as i32;

    if ei.npt > 0 && ei.npt <= 25
    /* XMP_MAX_ENV_POINTS */
    {
        for i in 0..ei.npt as usize {
            ei.data[i * 2] = env.node[i].x as i16;
            ei.data[i * 2 + 1] = env.node[i].y as i16;
        }
    } else {
        ei.flg &= !XMP_ENVELOPE_ON;
    }

    0
}

/// Translation of `identify_tracker()`.
fn identify_tracker(
    m: &mut ModuleData,
    ifh: &mut ItFileHeader,
    pat_before_smp: bool,
    is_mpt_116: &mut bool,
) {
    let tracker_name: String;
    let sample_mode = !ifh.flags & IT_USE_INST != 0;
    let ver_hi = (ifh.cwt & 0x0f00) >> 8;
    let ver_lo = ifh.cwt & 0xff;

    m.flow_mode = FLOW_MODE_IT_210;
    match ifh.cwt >> 8 {
        0x00 => {
            tracker_name = "unmo3".into();
        }
        0x01 | 0x02 => {
            /* test from Schism Tracker sources */
            if ifh.cmwt == 0x0200
                && ifh.cwt == 0x0214
                && ifh.flags == 9
                && ifh.special == 0
                && ifh.hilite_maj == 0
                && ifh.hilite_min == 0
                && ifh.insnum == 0
                && ifh.patnum as i32 + 1 == ifh.ordnum as i32
                && ifh.gv == 128
                && ifh.mv == 100
                && ifh.is == 1
                && ifh.sep == 128
                && ifh.pwd == 0
                && ifh.msglen == 0
                && ifh.msgofs == 0
                && ifh.rsvd == 0
            {
                tracker_name = "OpenSPC conversion".into();
            } else if ifh.cmwt == 0x0200 && ifh.cwt == 0x0217 {
                tracker_name = "ModPlug Tracker 1.16".into();
                /* ModPlug Tracker files aren't really IMPM 2.00 */
                ifh.cmwt = if sample_mode { 0x100 } else { 0x214 };
                m.flow_mode = FLOW_MODE_MPT_116;
                *is_mpt_116 = true;
            } else if ifh.cmwt == 0x0200 && ifh.cwt == 0x0202 && pat_before_smp {
                /* ModPlug Tracker ITs from pre-alpha 4 use tracker
                 * 0x0202 and format 0x0200. Unfortunately, ITs from
                 * Impulse Tracker may *also* use this. These MPT ITs
                 * can be detected because they write patterns before
                 * samples/instruments. */
                tracker_name = "ModPlug Tracker 1.0 pre-alpha".into();
                ifh.cmwt = if sample_mode { 0x100 } else { 0x200 };
                /* TODO: pre-alpha 4 has its own Pattern Loop behavior;
                 * the <=1.16 behavior is present in pre-alpha 6. */
                m.flow_mode = FLOW_MODE_MPT_116;
                *is_mpt_116 = true;
            } else if ifh.cwt == 0x0216 {
                tracker_name = "Impulse Tracker 2.14v3".into();
            } else if ifh.cwt == 0x0217 {
                tracker_name = "Impulse Tracker 2.14v5".into();
            } else if ifh.cwt == 0x0214 && ifh.rsvd.to_ne_bytes() == *b"CHBI" {
                tracker_name = "Chibi Tracker".into();
            } else {
                tracker_name = format!("Impulse Tracker {}.{:02x}", ver_hi, ver_lo);

                if ifh.cwt < 0x104 {
                    m.flow_mode = FLOW_MODE_IT_100;
                } else if ifh.cwt < 0x200 {
                    m.flow_mode = FLOW_MODE_IT_104;
                } else if ifh.cwt < 0x210 {
                    m.flow_mode = FLOW_MODE_IT_200;
                }
            }
        }
        0x08 | 0x7f => {
            if ifh.cwt == 0x0888 {
                tracker_name = "OpenMPT 1.17".into();
                /* TODO: 1.17.02.49 onward implement IT 2.10+
                 * Pattern Loop when the IT compatibility flag is set
                 * (by default, it is not set). */
                m.flow_mode = FLOW_MODE_MPT_116;
                *is_mpt_116 = true;
            } else if ifh.cwt == 0x7fff {
                tracker_name = "munch.py".into();
            } else {
                tracker_name = format!("unknown ({:04x})", ifh.cwt);
            }
        }
        _ => match ifh.cwt >> 12 {
            0x1 => {
                let mut name = [0u8; 40];
                libxmp_schism_tracker_string(
                    &mut name,
                    40,
                    (ifh.cwt & 0x0fff) as i32,
                    ifh.rsvd as i32,
                );
                tracker_name = String::from_utf8_lossy(cstr(&name)).into_owned();
            }
            0x5 => {
                let mut name = format!("OpenMPT {}.{:02x}", ver_hi, ver_lo);
                if ifh.rsvd.to_ne_bytes() != *b"OMPT" {
                    /* strncat(tracker_name, " (compat.)", 39) */
                    name.push_str(" (compat.)");
                }
                tracker_name = name;
            }
            0x06 => {
                tracker_name = format!("BeRoTracker {}.{:02x}", ver_hi, ver_lo);
            }
            _ => {
                tracker_name = format!("unknown ({:04x})", ifh.cwt);
            }
        },
    }

    /* (tracker_name holds 39 bytes) */
    let tn = &tracker_name.as_bytes()[..tracker_name.len().min(39)];
    let mut type_ = tn.to_vec();
    type_.extend_from_slice(format!(" IT {}.{:02x}", ifh.cmwt >> 8, ifh.cmwt & 0xff).as_bytes());
    libxmp_set_type(m, &type_);
}

/// Translation of `load_old_it_instrument()`.
fn load_old_it_instrument(xxi: &mut XmpInstrument, f: &mut HioHandle<'_>) -> i32 {
    let mut inst_map = [0xffu8; 255];
    let mut inst_rmap = [0u8; XMP_MAX_KEYS];
    let mut buf = [0u8; 64];

    if hio_read(&mut buf, 1, 64, f) != 64 {
        return -1;
    }

    let mut i1h = ItInstrument1Header {
        magic: readmem32b(&buf),
        dosname: [0; 12],
        zero: 0,
        flags: 0,
        vls: 0,
        vle: 0,
        sls: 0,
        sle: 0,
        fadeout: 0,
        nna: 0,
        dnc: 0,
        trkvers: 0,
        nos: 0,
        name: [0; 26],
        keys: [0; 240],
        epoint: [0; 200],
        enode: [0; 50],
    };
    if i1h.magic != MAGIC_IMPI {
        return -1;
    }
    i1h.dosname.copy_from_slice(&buf[4..16]);
    i1h.zero = buf[16];
    i1h.flags = buf[17];
    i1h.vls = buf[18];
    i1h.vle = buf[19];
    i1h.sls = buf[20];
    i1h.sle = buf[21];
    i1h.fadeout = readmem16l(&buf[24..]);
    i1h.nna = buf[26];
    i1h.dnc = buf[27];
    i1h.trkvers = readmem16l(&buf[28..]);
    i1h.nos = buf[30];

    i1h.name.copy_from_slice(&buf[32..58]);
    fix_name(&mut i1h.name, 26);

    if hio_read(&mut i1h.keys, 1, 240, f) != 240 {
        return -1;
    }
    if hio_read(&mut i1h.epoint, 1, 200, f) != 200 {
        return -1;
    }
    if hio_read(&mut i1h.enode, 1, 50, f) != 50 {
        return -1;
    }

    libxmp_copy_adjust(&mut xxi.name, &i1h.name, 25);

    xxi.rls = (i1h.fadeout as i32) << 7;

    xxi.aei.flg = 0;
    if i1h.flags & IT_ENV_ON != 0 {
        xxi.aei.flg |= XMP_ENVELOPE_ON;
    }
    if i1h.flags & IT_ENV_LOOP != 0 {
        xxi.aei.flg |= XMP_ENVELOPE_LOOP;
    }
    if i1h.flags & IT_ENV_SLOOP != 0 {
        xxi.aei.flg |= XMP_ENVELOPE_SUS | XMP_ENVELOPE_SLOOP;
    }
    if i1h.flags & IT_ENV_CARRY != 0 {
        xxi.aei.flg |= XMP_ENVELOPE_SUS | XMP_ENVELOPE_CARRY;
    }
    xxi.aei.lps = i1h.vls as i32;
    xxi.aei.lpe = i1h.vle as i32;
    xxi.aei.sus = i1h.sls as i32;
    xxi.aei.sue = i1h.sle as i32;

    let mut k = 0;
    while k < 25 && i1h.enode[k * 2] != 0xff {
        k += 1;
    }

    /* Sanity check */
    if k >= 25 || i1h.enode[k * 2] != 0xff {
        return -1;
    }

    xxi.aei.npt = k as i32;
    while k > 0 {
        k -= 1;
        xxi.aei.data[k * 2] = i1h.enode[k * 2] as i16;
        xxi.aei.data[k * 2 + 1] = i1h.enode[k * 2 + 1] as i16;
    }

    /* See how many different instruments we have */

    let mut k = 0usize;
    for j in 0..XMP_MAX_KEYS {
        let c = if j < 120 {
            i1h.keys[j * 2 + 1] as i32 - 1
        } else {
            -1
        };
        if c < 0 {
            xxi.map[j].ins = 0;
            xxi.map[j].xpo = 0;
            continue;
        }
        if inst_map[c as usize] == 0xff {
            inst_map[c as usize] = k as u8;
            inst_rmap[k] = c as u8;
            k += 1;
        }
        xxi.map[j].ins = inst_map[c as usize];
        xxi.map[j].xpo = (i1h.keys[j * 2] as i32 - j as i32) as i8;
    }

    xxi.nsm = k as i32;
    xxi.vol = 0x40;

    if k != 0 {
        let Ok(sub) = try_vec(k, XmpSubinstrument::default()) else {
            return -1;
        };
        xxi.sub = sub;

        for j in 0..k {
            let sub = &mut xxi.sub[j];

            sub.sid = inst_rmap[j] as i32;
            sub.nna = i1h.nna as i32;
            sub.dct = if i1h.dnc != 0 {
                XMP_INST_DCT_NOTE
            } else {
                XMP_INST_DCT_OFF
            };
            sub.dca = XMP_INST_DCA_CUT;
            sub.pan = XMP_INST_NO_DEFAULT_PAN;
        }
    }

    0
}

/// Translation of `load_new_it_instrument()`.
fn load_new_it_instrument(xxi: &mut XmpInstrument, f: &mut HioHandle<'_>) -> i32 {
    let mut inst_map = [0xffu8; 255];
    let mut inst_rmap = [0u8; XMP_MAX_KEYS];
    let mut env = ItEnvelope::default();
    let dca2nna: [i32; 4] = [0, 2, 3, 3 /* Northern Sky (cj-north.it) has this... */];
    let mut buf = [0u8; 64];

    if hio_read(&mut buf, 1, 64, f) != 64 {
        return -1;
    }

    let mut i2h = ItInstrument2Header {
        magic: readmem32b(&buf),
        dosname: [0; 12],
        zero: 0,
        nna: 0,
        dct: 0,
        dca: 0,
        fadeout: 0,
        pps: 0,
        ppc: 0,
        gbv: 0,
        dfp: 0,
        rv: 0,
        rp: 0,
        trkvers: 0,
        nos: 0,
        name: [0; 26],
        ifc: 0,
        ifr: 0,
        mch: 0,
        mpr: 0,
        mbnk: 0,
        keys: [0; 240],
    };
    if i2h.magic != MAGIC_IMPI {
        return -1;
    }
    i2h.dosname.copy_from_slice(&buf[4..16]);
    i2h.zero = buf[16];
    i2h.nna = buf[17];
    i2h.dct = buf[18];
    i2h.dca = buf[19];

    /* Sanity check */
    if i2h.dca > 3 {
        /* Northern Sky has an instrument with DCA 3 */
        i2h.dca = 0;
    }

    i2h.fadeout = readmem16l(&buf[20..]);
    i2h.pps = buf[22];
    i2h.ppc = buf[23];
    i2h.gbv = buf[24];
    i2h.dfp = buf[25];
    i2h.rv = buf[26];
    i2h.rp = buf[27];
    i2h.trkvers = readmem16l(&buf[28..]);
    i2h.nos = buf[30];

    i2h.name.copy_from_slice(&buf[32..58]);
    fix_name(&mut i2h.name, 26);

    i2h.ifc = buf[58];
    i2h.ifr = buf[59];
    i2h.mch = buf[60];
    i2h.mpr = buf[61];
    i2h.mbnk = readmem16l(&buf[62..]);

    if hio_read(&mut i2h.keys, 1, 240, f) != 240 {
        return -1;
    }

    libxmp_copy_adjust(&mut xxi.name, &i2h.name, 25);
    xxi.rls = (i2h.fadeout as i32) << 6;

    /* Envelopes */

    if read_envelope(&mut xxi.aei, &mut env, f) < 0 {
        return -1;
    }
    if read_envelope(&mut xxi.pei, &mut env, f) < 0 {
        return -1;
    }
    if read_envelope(&mut xxi.fei, &mut env, f) < 0 {
        return -1;
    }

    if xxi.pei.flg & XMP_ENVELOPE_ON != 0 {
        for j in 0..xxi.pei.npt.max(0) as usize {
            xxi.pei.data[j * 2 + 1] = xxi.pei.data[j * 2 + 1].wrapping_add(32);
        }
    }

    if xxi.aei.flg & XMP_ENVELOPE_ON != 0 && xxi.aei.npt == 0 {
        xxi.aei.npt = 1;
    }
    if xxi.pei.flg & XMP_ENVELOPE_ON != 0 && xxi.pei.npt == 0 {
        xxi.pei.npt = 1;
    }
    if xxi.fei.flg & XMP_ENVELOPE_ON != 0 && xxi.fei.npt == 0 {
        xxi.fei.npt = 1;
    }

    if env.flg & IT_ENV_FILTER != 0 {
        xxi.fei.flg |= XMP_ENVELOPE_FLT;
        for j in 0..env.num as usize {
            let d = &mut xxi.fei.data[j * 2 + 1];
            *d = d.wrapping_add(32);
            *d = d.wrapping_mul(4);
        }
    } else {
        /* Pitch envelope is *50 to get fine interpolation */
        for j in 0..env.num as usize {
            let d = &mut xxi.fei.data[j * 2 + 1];
            *d = d.wrapping_mul(50);
        }
    }

    /* See how many different instruments we have */

    let mut k = 0usize;
    for j in 0..120 {
        let c = i2h.keys[j * 2 + 1] as i32 - 1;
        if c < 0 {
            xxi.map[j].ins = 0xff; /* No sample */
            xxi.map[j].xpo = 0;
            continue;
        }
        if inst_map[c as usize] == 0xff {
            inst_map[c as usize] = k as u8;
            inst_rmap[k] = c as u8;
            k += 1;
        }
        xxi.map[j].ins = inst_map[c as usize];
        xxi.map[j].xpo = (i2h.keys[j * 2] as i32 - j as i32) as i8;
    }

    xxi.nsm = k as i32;
    xxi.vol = (i2h.gbv.min(128) >> 1) as i32;

    if k != 0 {
        let Ok(sub) = try_vec(k, XmpSubinstrument::default()) else {
            return -1;
        };
        xxi.sub = sub;

        for j in 0..k {
            let sub = &mut xxi.sub[j];

            sub.sid = inst_rmap[j] as i32;
            sub.nna = i2h.nna as i32;
            sub.dct = i2h.dct as i32;
            sub.dca = dca2nna[i2h.dca as usize];
            sub.pan = if i2h.dfp & 0x80 != 0 {
                XMP_INST_NO_DEFAULT_PAN
            } else {
                i2h.dfp as i32 * 4
            };
            sub.ifc = i2h.ifc as i32;
            sub.ifr = i2h.ifr as i32;
            sub.rvv = ((i2h.rp as i32) << 8) | i2h.rv as i32;
        }
    }

    0
}

/// Translation of `force_sample_length()`.
fn force_sample_length(xxs: &mut XmpSample, xtra: Option<&mut ExtraSampleData>, len: i32) {
    xxs.len = len;

    if xxs.lpe > xxs.len {
        xxs.lpe = xxs.len;
    }

    if xxs.lps >= xxs.len {
        xxs.flg &= !XMP_SAMPLE_LOOP;
    }

    if let Some(xtra) = xtra {
        if xtra.sue > xxs.len {
            xtra.sue = xxs.len;
        }

        if xtra.sus >= xxs.len {
            xxs.flg &= !(XMP_SAMPLE_SLOOP | XMP_SAMPLE_SLOOP_BIDIR);
        }
    }
}

/// Translation of `unpack_it_sample()`.
fn unpack_it_sample(
    xxs: &XmpSample,
    ish: &ItSampleHeader,
    tmpbuf: &mut [u8],
    f: &mut HioHandle<'_>,
) -> Option<Vec<u8>> {
    let mut bytes = xxs.len;
    let mut channels = 1;

    if ish.flags & IT_SMP_16BIT != 0 {
        bytes <<= 1;
    }

    if ish.flags & IT_SMP_STEREO != 0 {
        bytes <<= 1;
        channels = 2;
    }

    let mut decbuf = try_vec(bytes.max(0) as usize, 0u8).ok()?;
    let it215 = ish.convert & IT_CVT_DIFF != 0;
    let len = xxs.len.max(0) as usize;

    if ish.flags & IT_SMP_16BIT != 0 {
        let mut pos = 0;

        for _ in 0..channels {
            itsex_decompress16(
                f,
                &mut decbuf[pos * 2..],
                xxs.len,
                tmpbuf,
                TEMP_BUFFER_LEN as i32,
                it215,
            );
            pos += len;
        }
    } else {
        let mut pos = 0;

        for _ in 0..channels {
            itsex_decompress8(
                f,
                &mut decbuf[pos..],
                xxs.len,
                tmpbuf,
                TEMP_BUFFER_LEN as i32,
                it215,
            );
            pos += len;
        }
    }
    Some(decbuf)
}

/// Translation of `load_it_sample()`.
fn load_it_sample(
    m: &mut ModuleData,
    i: i32,
    start: i32,
    sample_mode: bool,
    tmpbuf: &mut [u8],
    f: &mut HioHandle<'_>,
) -> i32 {
    let mut ish = ItSampleHeader::default();
    let mut buf = [0u8; 80];
    let iu = i as usize;

    if sample_mode {
        let Ok(sub) = try_vec(1, XmpSubinstrument::default()) else {
            return -1;
        };
        m.mod_.xxi[iu].sub = sub;
    }

    if hio_read(&mut buf, 1, 80, f) != 80 {
        return -1;
    }

    ish.magic = readmem32b(&buf);
    /* Changed to continue to allow use-brdg.it and use-funk.it to
     * load correctly (both IT 2.04)
     */
    if ish.magic != MAGIC_IMPS {
        return 0;
    }

    ish.dosname.copy_from_slice(&buf[4..16]);
    ish.zero = buf[16];
    ish.gvl = buf[17];
    ish.flags = buf[18];
    ish.vol = buf[19];

    ish.name.copy_from_slice(&buf[20..46]);
    fix_name(&mut ish.name, 26);

    ish.convert = buf[46];
    ish.dfp = buf[47];
    ish.length = readmem32l(&buf[48..]);
    ish.loopbeg = readmem32l(&buf[52..]);
    ish.loopend = readmem32l(&buf[56..]);
    ish.c5spd = readmem32l(&buf[60..]);
    ish.sloopbeg = readmem32l(&buf[64..]);
    ish.sloopend = readmem32l(&buf[68..]);
    ish.sample_ptr = readmem32l(&buf[72..]);
    ish.vis = buf[76];
    ish.vid = buf[77];
    ish.vir = buf[78];
    ish.vit = buf[79];

    {
        let xxs = &mut m.mod_.xxs[iu];
        if ish.flags & IT_SMP_16BIT != 0 {
            xxs.flg = XMP_SAMPLE_16BIT;
        }
        if ish.flags & IT_SMP_STEREO != 0 {
            xxs.flg |= XMP_SAMPLE_STEREO;
        }
        xxs.len = ish.length as i32;

        xxs.lps = ish.loopbeg as i32;
        xxs.lpe = ish.loopend as i32;
        xxs.flg |= if ish.flags & IT_SMP_LOOP != 0 {
            XMP_SAMPLE_LOOP
        } else {
            0
        };
        xxs.flg |= if ish.flags & IT_SMP_BLOOP != 0 {
            XMP_SAMPLE_LOOP_BIDIR
        } else {
            0
        };
        xxs.flg |= if ish.flags & IT_SMP_SLOOP != 0 {
            XMP_SAMPLE_SLOOP
        } else {
            0
        };
        xxs.flg |= if ish.flags & IT_SMP_BSLOOP != 0 {
            XMP_SAMPLE_SLOOP_BIDIR
        } else {
            0
        };
    }

    if ish.flags & IT_SMP_SLOOP != 0 {
        let xtra = &mut m.xtra[iu];
        xtra.sus = ish.sloopbeg as i32;
        xtra.sue = ish.sloopend as i32;
    }

    if sample_mode {
        /* Create an instrument for each sample */
        let has_len = m.mod_.xxs[iu].len != 0;
        let xxi = &mut m.mod_.xxi[iu];
        xxi.vol = 64;
        xxi.sub[0].vol = ish.vol as i32;
        xxi.sub[0].pan = XMP_INST_NO_DEFAULT_PAN;
        xxi.sub[0].sid = i;
        xxi.nsm = has_len as i32;
        libxmp_instrument_name(&mut m.mod_, i, &ish.name, 25);
    } else {
        libxmp_copy_adjust(&mut m.mod_.xxs[iu].name, &ish.name, 25);
    }

    /* Convert C5SPD to relnote/finetune
     *
     * In IT we can have a sample associated with two or more
     * instruments, but c5spd is a sample attribute -- so we must
     * scan all xmp instruments to set the correct transposition
     */

    for j in 0..m.mod_.ins as usize {
        for k in 0..m.mod_.xxi[j].nsm as usize {
            let sub = &mut m.mod_.xxi[j].sub[k];
            if sub.sid == i {
                sub.vol = ish.vol as i32;
                sub.gvl = ish.gvl.min(64) as i32;
                sub.vra = ish.vis as i32; /* sample to sub-instrument vibrato */
                sub.vde = (ish.vid as i32) << 1;
                sub.vwf = ish.vit as i32;
                sub.vsw = (0xff - ish.vir as i32) >> 1;

                libxmp_c2spd_to_note(ish.c5spd as i32, &mut sub.xpo, &mut sub.fin);

                /* Set sample pan (overrides subinstrument) */
                if ish.dfp & 0x80 != 0 {
                    sub.pan = (ish.dfp as i32 & 0x7f) * 4;
                } else if sample_mode {
                    sub.pan = XMP_INST_NO_DEFAULT_PAN;
                }
            }
        }
    }

    if ish.flags & IT_SMP_SAMPLE != 0 && m.mod_.xxs[iu].len > 1 {
        let mut cvt = 0;

        /* Sanity check - some modules may have invalid sizes on
         * unused samples so only check this if the sample flag is set. */
        if m.mod_.xxs[iu].len > MAX_SAMPLE_SIZE {
            return -1;
        }

        if 0 != hio_seek(
            f,
            (start as u32).wrapping_add(ish.sample_ptr) as i64,
            SEEK_SET,
        ) {
            return -1;
        }

        {
            let xxs = &mut m.mod_.xxs[iu];
            if xxs.lpe > xxs.len || xxs.lps >= xxs.lpe {
                xxs.flg &= !XMP_SAMPLE_LOOP;
            }
        }

        if ish.convert == IT_CVT_ADPCM {
            cvt |= SAMPLE_FLAG_ADPCM;
        }

        if !ish.convert & IT_CVT_SIGNED != 0 {
            cvt |= SAMPLE_FLAG_UNS;
        }

        /* compressed samples */
        if ish.flags & IT_SMP_COMP != 0 {
            let mut samples = m.mod_.xxs[iu].len;

            if ish.flags & IT_SMP_STEREO != 0 {
                samples <<= 1;
            }

            /* Sanity check - the lower bound on IT compressed
             * sample size (in bytes) is a little over 1/8th of the
             * number of SAMPLES in the sample.
             */
            let file_len = hio_size(f);
            let min_size = (samples >> 3) as i64;
            let left = file_len - ish.sample_ptr as i64;
            /* No data to read at all? Just skip it... */
            if left <= 0 {
                return 0;
            }

            if file_len > 0 && left < min_size {
                force_sample_length(
                    &mut m.mod_.xxs[iu],
                    Some(&mut m.xtra[iu]),
                    (left << 3) as i32,
                );
            }

            let Some(decbuf) = unpack_it_sample(&m.mod_.xxs[iu], &ish, tmpbuf, f) else {
                return -1;
            };

            if cfg!(target_endian = "big") && ish.flags & IT_SMP_16BIT != 0 {
                /* decompression generates native-endian
                 * samples, but we want little-endian.
                 */
                cvt |= SAMPLE_FLAG_BIGEND;
            }

            let smpctl = m.smpctl;
            let ret = libxmp_load_sample(
                smpctl,
                None,
                SAMPLE_FLAG_NOLOAD | cvt,
                &mut m.mod_.xxs[iu],
                Some(&decbuf),
            );
            if ret < 0 {
                return -1;
            }
        } else {
            let smpctl = m.smpctl;
            if libxmp_load_sample(smpctl, Some(f), cvt, &mut m.mod_.xxs[iu], None) < 0 {
                return -1;
            }
        }
    }

    0
}

/// Translation of `load_it_pattern()`.
fn load_it_pattern(
    m: &mut ModuleData,
    i: i32,
    new_fx: bool,
    patbuf: &mut [u8],
    f: &mut HioHandle<'_>,
) -> i32 {
    let mod_ = &mut m.mod_;
    let mut dummy = XmpEvent::default();
    let mut lastevent = [XmpEvent::default(); L_CHANNELS];
    let mut mask = [0u8; L_CHANNELS];
    let mut last_fxp = [0u8; 64];

    let mut r = 0;

    let pat_len = hio_read16l(f) as usize /* - 4 */;
    let num_rows = hio_read16l(f) as i32;
    mod_.pattern_mut(i).rows = num_rows;

    if libxmp_alloc_tracks_in_pattern(mod_, i) < 0 {
        return -1;
    }

    hio_read16l(f);
    hio_read16l(f);

    if hio_read(patbuf, 1, pat_len, f) < pat_len {
        return -1;
    }
    let mut pos = 0usize;
    let end = pat_len;

    'rows: while r < num_rows && pos < end {
        let mut b = patbuf[pos];
        pos += 1;
        if b == 0 {
            r += 1;
            continue;
        }
        let c = ((b as usize).wrapping_sub(1)) & 63;

        if b & 0x80 != 0 {
            if pos >= end {
                break;
            }
            mask[c] = patbuf[pos];
            pos += 1;
        }
        /*
         * WARNING: we IGNORE events in disabled channels. Disabled
         * channels should be muted only, but we don't know the
         * real number of channels before loading the patterns and
         * we don't want to set it to 64 channels.
         */
        let event = if c as i32 >= mod_.chn {
            &mut dummy
        } else {
            mod_.event_mut(i, c as i32, r)
        };

        if (mask[c] & 0x0f) != 0 {
            if mask[c] & 0x01 != 0 {
                if pos >= end {
                    break 'rows;
                }
                b = patbuf[pos];
                pos += 1;

                /* From ittech.txt:
                 * Note ranges from 0->119 (C-0 -> B-9)
                 * 255 = note off, 254 = notecut
                 * Others = note fade (already programmed into IT's player
                 *                     but not available in the editor)
                 */
                match b {
                    0xff => {
                        /* key off */
                        b = XMP_KEY_OFF;
                    }
                    0xfe => {
                        /* cut */
                        b = XMP_KEY_CUT;
                    }
                    _ => {
                        if b > 119 {
                            /* fade */
                            b = XMP_KEY_FADE;
                        } else {
                            b += 1; /* note */
                        }
                    }
                }
                event.note = b;
                lastevent[c].note = b;
            }
            if mask[c] & 0x02 != 0 {
                if pos >= end {
                    break 'rows;
                }
                b = patbuf[pos];
                pos += 1;
                event.ins = b;
                lastevent[c].ins = b;
            }
            if mask[c] & 0x04 != 0 {
                if pos >= end {
                    break 'rows;
                }
                b = patbuf[pos];
                pos += 1;
                event.vol = b;
                lastevent[c].vol = b;
                xlat_volfx(event);
            }
            if mask[c] & 0x08 != 0 {
                if pos + 1 >= end {
                    break 'rows;
                }
                b = patbuf[pos];
                pos += 1;
                if b as usize >= FX.len() {
                    pos += 1;
                } else {
                    event.fxt = b;
                    event.fxp = patbuf[pos];
                    pos += 1;

                    xlat_fx(c, event, &mut last_fxp, new_fx);
                    lastevent[c].fxt = event.fxt;
                    lastevent[c].fxp = event.fxp;
                }
            }
        }

        /* skip_packed_event: */
        if (mask[c] & 0xf0) == 0 {
            continue;
        }
        if mask[c] & 0x10 != 0 {
            event.note = lastevent[c].note;
        }
        if mask[c] & 0x20 != 0 {
            event.ins = lastevent[c].ins;
        }
        if mask[c] & 0x40 != 0 {
            event.vol = lastevent[c].vol;
            xlat_volfx(event);
        }
        if mask[c] & 0x80 != 0 {
            event.fxt = lastevent[c].fxt;
            event.fxp = lastevent[c].fxp;
        }
    }

    0
}

/// Translation of `it_load()`.
fn it_load(m: &mut ModuleData, f: &mut HioHandle<'_>, start: i32) -> i32 {
    let mut ifh = ItFileHeader {
        magic: 0,
        name: [0; 26],
        hilite_min: 0,
        hilite_maj: 0,
        ordnum: 0,
        insnum: 0,
        smpnum: 0,
        patnum: 0,
        cwt: 0,
        cmwt: 0,
        flags: 0,
        special: 0,
        gv: 0,
        mv: 0,
        is: 0,
        it: 0,
        sep: 0,
        pwd: 0,
        msglen: 0,
        msgofs: 0,
        rsvd: 0,
        chpan: [0; 64],
        chvol: [0; 64],
    };
    let mut pat_before_smp = false;
    let mut is_mpt_116 = false;

    /* Load and convert header */
    ifh.magic = hio_read32b(f);
    if ifh.magic != MAGIC_IMPM {
        return -1;
    }

    hio_read(&mut ifh.name, 26, 1, f);
    ifh.hilite_min = hio_read8(f);
    ifh.hilite_maj = hio_read8(f);

    ifh.ordnum = hio_read16l(f);
    ifh.insnum = hio_read16l(f);
    ifh.smpnum = hio_read16l(f);
    ifh.patnum = hio_read16l(f);

    ifh.cwt = hio_read16l(f);
    ifh.cmwt = hio_read16l(f);
    ifh.flags = hio_read16l(f);
    ifh.special = hio_read16l(f);

    ifh.gv = hio_read8(f);
    ifh.mv = hio_read8(f);
    ifh.is = hio_read8(f);
    ifh.it = hio_read8(f);
    ifh.sep = hio_read8(f);
    ifh.pwd = hio_read8(f);

    /* Sanity check */
    if ifh.gv > 0x80 {
        return -1;
    }

    ifh.msglen = hio_read16l(f);
    ifh.msgofs = hio_read32l(f);
    ifh.rsvd = hio_read32l(f);

    hio_read(&mut ifh.chpan, 64, 1, f);
    hio_read(&mut ifh.chvol, 64, 1, f);

    if hio_error(f) != 0 {
        return -1;
    }

    let mod_ = &mut m.mod_;
    mod_.name[..26].copy_from_slice(&ifh.name);
    /* sizeof(ifh.name) == 26, sizeof(mod->name) == 64. */
    mod_.name[26] = b'\0';
    mod_.len = ifh.ordnum as i32;
    mod_.ins = ifh.insnum as i32;
    mod_.smp = ifh.smpnum as i32;
    mod_.pat = ifh.patnum as i32;

    /* Sanity check */
    if mod_.ins > 255 || mod_.smp > 255 || mod_.pat > 255 {
        return -1;
    }

    let Ok(mut pp_ins) = try_vec(mod_.ins as usize, 0u32) else {
        return -1;
    };
    let Ok(mut pp_smp) = try_vec(mod_.smp as usize, 0u32) else {
        return -1;
    };
    let Ok(mut pp_pat) = try_vec(mod_.pat as usize, 0u32) else {
        return -1;
    };

    mod_.spd = ifh.is as i32;
    mod_.bpm = ifh.it as i32;

    let sample_mode = !ifh.flags & IT_USE_INST != 0;

    if ifh.flags & IT_LINEAR_FREQ != 0 {
        m.period_type = PERIOD_LINEAR;
    }

    let mod_ = &mut m.mod_;
    for i in 0..64 {
        let xxc = &mut mod_.xxc[i];
        let pan = (ifh.chpan[i] & 0x7f) as i32;

        if pan == 100 {
            /* Surround -> center */
            xxc.flg |= XMP_CHANNEL_SURROUND;
        }

        if ifh.chpan[i] & 0x80 != 0 {
            /* Channel mute */
            xxc.flg |= XMP_CHANNEL_MUTE;
        }

        if ifh.flags & IT_STEREO != 0 {
            xxc.pan = pan * 0x80 >> 5;
            if xxc.pan > 0xff {
                xxc.pan = 0xff;
            }
        } else {
            xxc.pan = 0x80;
        }

        xxc.vol = ifh.chvol[i] as i32;
    }

    if mod_.len <= XMP_MAX_MOD_LENGTH as i32 {
        let len = mod_.len as usize;
        hio_read(&mut mod_.xxo, 1, len, f);
    } else {
        hio_read(&mut mod_.xxo, 1, XMP_MAX_MOD_LENGTH, f);
        hio_seek(f, (mod_.len - XMP_MAX_MOD_LENGTH as i32) as i64, SEEK_CUR);
        mod_.len = XMP_MAX_MOD_LENGTH as i32;
    }

    let new_fx = ifh.flags & IT_OLD_FX == 0;

    for p in pp_ins.iter_mut() {
        *p = hio_read32l(f);
    }
    for p in pp_smp.iter_mut() {
        *p = hio_read32l(f);
    }
    for p in pp_pat.iter_mut() {
        *p = hio_read32l(f);
    }

    /* Skip edit history if it exists. */
    if ifh.special & IT_EDIT_HISTORY != 0 {
        let skip = hio_read16l(f) as i32 * 8;
        if hio_error(f) != 0 || (skip != 0 && hio_seek(f, skip as i64, SEEK_CUR) < 0) {
            return -1;
        }
    }

    if ((ifh.flags & IT_MIDI_CONFIG) != 0 || (ifh.special & IT_SPEC_MIDICFG) != 0)
        && load_it_midi_config(m, f) < 0
    {
        return -1;
    }
    if m.mod_.smp != 0 && m.mod_.pat != 0 && pp_pat[0] != 0 && pp_pat[0] < pp_smp[0] {
        pat_before_smp = true;
    }

    m.c4rate = C4_NTSC_RATE;

    identify_tracker(m, &mut ifh, pat_before_smp, &mut is_mpt_116);

    if sample_mode {
        m.mod_.ins = m.mod_.smp;
    }

    if libxmp_init_instrument(m) < 0 {
        return -1;
    }

    for i in 0..m.mod_.ins as usize {
        /*
         * IT files can have three different instrument types: 'New'
         * instruments, 'old' instruments or just samples. We need a
         * different loader for each of them.
         */

        let xxi = &mut m.mod_.xxi[i];

        if !sample_mode && ifh.cmwt >= 0x200 {
            /* New instrument format */
            if hio_seek(f, (start as u32).wrapping_add(pp_ins[i]) as i64, SEEK_SET) < 0 {
                return -1;
            }

            if load_new_it_instrument(xxi, f) < 0 {
                return -1;
            }
        } else if !sample_mode {
            /* Old instrument format */
            if hio_seek(f, (start as u32).wrapping_add(pp_ins[i]) as i64, SEEK_SET) < 0 {
                return -1;
            }

            if load_old_it_instrument(xxi, f) < 0 {
                return -1;
            }
        }
    }

    /* This buffer should be able to hold any pattern or sample block.
     * Round up to a multiple of 4--the sample decompressor relies on
     * this to simplify its code.
     */
    let Ok(mut patbuf) = try_vec(TEMP_BUFFER_LEN, 0u8) else {
        return -1;
    };

    for i in 0..m.mod_.smp {
        if hio_seek(
            f,
            (start as u32).wrapping_add(pp_smp[i as usize]) as i64,
            SEEK_SET,
        ) < 0
        {
            return -1;
        }

        if load_it_sample(m, i, start, sample_mode, &mut patbuf, f) < 0 {
            return -1;
        }
    }
    /* Reset any error status set by truncated samples. */
    hio_error(f);

    /* Effects in muted channels are processed, so scan patterns first to
     * see the real number of channels
     */
    let mut max_ch = 0;
    for i in 0..m.mod_.pat as usize {
        let mut mask = [0u8; L_CHANNELS];

        /* If the offset to a pattern is 0, the pattern is empty */
        if pp_pat[i] == 0 {
            continue;
        }

        hio_seek(f, (start as u32).wrapping_add(pp_pat[i]) as i64, SEEK_SET);
        let pat_len = hio_read16l(f) as usize /* - 4 */;
        let num_rows = hio_read16l(f) as i32;
        hio_read16l(f);
        hio_read16l(f);

        /* Sanity check:
         * - Impulse Tracker and Schism Tracker allow up to 200 rows.
         * - ModPlug Tracker 1.16 allows 256 rows.
         * - OpenMPT allows 1024 rows.
         */
        if num_rows > 1024 {
            pp_pat[i] = 0;
            continue;
        }

        if hio_read(&mut patbuf, 1, pat_len, f) < pat_len {
            return -1;
        }
        let mut pos = 0usize;
        let end = pat_len;

        let mut row = 0;
        while row < num_rows && pos < end {
            /* Bits 0, 1, 2 add 1 byte each. Bit 3 adds 2 bytes. */
            const BYTES_IN_PACKED_EVENT: [usize; 16] =
                [0, 1, 1, 2, 1, 2, 2, 3, 2, 3, 3, 4, 3, 4, 4, 5];
            let b = patbuf[pos] as i32;
            pos += 1;
            if b == 0 {
                row += 1;
                continue;
            }

            let c = ((b - 1) & 63) as usize;

            if c as i32 > max_ch {
                max_ch = c as i32;
            }

            if b & 0x80 != 0 {
                if pos >= end {
                    break;
                }
                /* The high nibble is not required to
                 * calculate the event size. */
                mask[c] = patbuf[pos] & 0x0f;
                pos += 1;
            }
            /* Skip packed event */
            pos += BYTES_IN_PACKED_EVENT[mask[c] as usize];
        }
    }

    /* Set the number of channels actually used
     */
    let mod_ = &mut m.mod_;
    mod_.chn = max_ch + 1;
    mod_.trk = mod_.pat * mod_.chn;

    if libxmp_init_pattern(mod_) < 0 {
        return -1;
    }

    /* Read patterns */
    for i in 0..m.mod_.pat {
        let mod_ = &mut m.mod_;
        if libxmp_alloc_pattern(mod_, i) < 0 {
            return -1;
        }

        /* If the offset to a pattern is 0, the pattern is empty */
        if pp_pat[i as usize] == 0 {
            mod_.pattern_mut(i).rows = 64;
            for j in 0..mod_.chn {
                let tnum = i * mod_.chn + j;
                if libxmp_alloc_track(mod_, tnum, 64) < 0 {
                    return -1;
                }
                mod_.pattern_mut(i).index[j as usize] = tnum;
            }
            continue;
        }

        if hio_seek(
            f,
            (start as u32).wrapping_add(pp_pat[i as usize]) as i64,
            SEEK_SET,
        ) < 0
        {
            return -1;
        }

        if load_it_pattern(m, i, new_fx, &mut patbuf, f) < 0 {
            return -1;
        }
    }

    drop(patbuf);

    /* Song message */
    if (ifh.special & IT_HAS_MSG) != 0 && ifh.msglen > 0 {
        if let Ok(mut comment) = try_vec(ifh.msglen as usize, 0u8) {
            hio_seek(f, (start as u32).wrapping_add(ifh.msgofs) as i64, SEEK_SET);

            ifh.msglen = hio_read(&mut comment, 1, ifh.msglen as usize, f) as u16;
            hio_error(f); /* Clear error if any */

            let mut j = 0usize;
            while j + 1 < ifh.msglen as usize {
                let b = comment[j] as i8 as i32;
                if b == b'\r' as i32 {
                    comment[j] = b'\n';
                } else if (b < 32 || b > 127) && b != b'\n' as i32 && b != b'\t' as i32 {
                    comment[j] = b'.';
                }
                j += 1;
            }
            comment[j] = 0;
            comment.truncate(j + 1);
            m.comment = Some(comment);
        }
    }

    /* Format quirks */

    m.quirk |= QUIRKS_IT | QUIRK_ARPMEM | QUIRK_INSVOL;

    if ifh.flags & IT_LINK_GXX != 0 {
        m.quirk |= QUIRK_PRENV;
    } else {
        m.quirk |= QUIRK_UNISLD;
    }

    if new_fx {
        m.quirk |= QUIRK_VIBHALF | QUIRK_VIBINV;
    } else {
        m.quirk &= !QUIRK_VIBALL;
        m.quirk |= QUIRK_ITOLDFX;
    }

    if sample_mode {
        m.quirk &= !(QUIRK_VIRTUAL | QUIRK_RSTCHN);
    }

    m.gvolbase = 0x80;
    m.gvol = ifh.gv as i32;
    m.mvolbase = 48;
    m.mvol = ifh.mv as i32;
    m.read_event_type = READ_EVENT_IT;

    if is_mpt_116 {
        libxmp_apply_mpt_preamp(m);
    }

    0
}
