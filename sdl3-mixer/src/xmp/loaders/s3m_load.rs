// Rust translation of src/loaders/s3m_load.c and src/loaders/s3m.h from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2026 Claudio Matsuoka and Hipolito Carraro Jr (s3m_load.c)
// Copyright (C) 1996-2024 Claudio Matsuoka and Hipolito Carraro Jr (s3m.h)
// This is an altered (translated) version of the original software; see LICENSE.txt.

/*
 * Tue, 30 Jun 1998 20:23:11 +0200
 * Reported by John v/d Kamp <blade_@dds.nl>:
 * I have this song from Purple Motion called wcharts.s3m, the global
 * volume was set to 0, creating a divide by 0 error in xmp. There should
 * be an extra test if it's 0 or not.
 *
 * Claudio's fix: global volume ignored
 */

/*
 * Sat, 29 Aug 1998 18:50:43 -0500 (CDT)
 * Reported by Joel Jordan <scriber@usa.net>:
 * S3M files support tempos outside the ranges defined by xmp (that is,
 * the MOD/XM tempo ranges).  S3M's can have tempos from 0 to 255 and speeds
 * from 0 to 255 as well, since these are handled with separate effects
 * unlike the MOD format.  This becomes an issue in such songs as Skaven's
 * "Catch that Goblin", which uses speeds above 0x1f.
 *
 * Claudio's fix: FX_S3M_SPEED added. S3M supports speeds from 0 to 255 and
 * tempos from 32 to 255 (S3M speed == xmp tempo, S3M tempo == xmp BPM).
 */

/* Wed, 21 Oct 1998 15:03:44 -0500  Geoff Reedy <vader21@imsa.edu>
 * It appears that xmp has issues loading/playing a specific instrument
 * used in LUCCA.S3M.
 * (Fixed by Hipolito in xmp-2.0.0dev34)
 */

/*
 * From http://code.pui.ch/2007/02/18/turn-demoscene-modules-into-mp3s/
 * The only flaw I noticed [in xmp] is a problem with portamento in Purple
 * Motion's second reality soundtrack (1:06-1:17)
 *
 * Claudio's note: that's a dissonant beating between channels 6 and 7
 * starting at pos12, caused by pitchbending effect F25.
 */

//! The Scream Tracker 3 S3M loader.

use super::super::common::*;
use super::super::dataio::*;
use super::super::effects::*;
use super::super::hio::*;
use super::super::period::libxmp_c2spd_to_note;
use super::super::*;
use super::common::*;
use super::sample::libxmp_load_sample;
use super::*;

/* S3M packed pattern macros */
pub(crate) const S3M_EOR: u8 = 0; /* End of row */
pub(crate) const S3M_CH_MASK: u8 = 0x1f; /* Channel */
pub(crate) const S3M_NI_FOLLOW: u8 = 0x20; /* Note and instrument follow */
pub(crate) const S3M_VOL_FOLLOWS: u8 = 0x40; /* Volume follows */
pub(crate) const S3M_FX_FOLLOWS: u8 = 0x80; /* Effect and parameter follow */

/* S3M mix volume macros */
pub(crate) const S3M_MV_VOLUME: u8 = 0x7f; /* Module mix volume, typically 16 to 127 */
pub(crate) const S3M_MV_STEREO: u8 = 0x80; /* Module is stereo if set, otherwise mono */

/* S3M channel info macros */
pub(crate) const S3M_CH_ON: u8 = 0x80; /* Psi says it's bit 8, I'll assume bit 7 */
pub(crate) const S3M_CH_OFF: u8 = 0xff;
pub(crate) const S3M_CH_NUMBER: u8 = 0x1f;
pub(crate) const S3M_CH_RIGHT: u8 = 0x08;
pub(crate) const S3M_CH_ADLIB: u8 = 0x10;

/* S3M channel pan macros */
pub(crate) const S3M_PAN_SET: u8 = 0x20;
pub(crate) const S3M_PAN_MASK: u8 = 0x0f;

/* S3M flags */
pub(crate) const S3M_ST2_VIB: u16 = 0x01; /* Not recognized */
pub(crate) const S3M_ST2_TEMPO: u16 = 0x02; /* Not recognized */
pub(crate) const S3M_AMIGA_SLIDE: u16 = 0x04; /* Not recognized */
pub(crate) const S3M_VOL_OPT: u16 = 0x08; /* Not recognized */
pub(crate) const S3M_AMIGA_RANGE: u16 = 0x10;
pub(crate) const S3M_SB_FILTER: u16 = 0x20; /* Not recognized */
pub(crate) const S3M_ST300_VOLS: u16 = 0x40;
pub(crate) const S3M_CUSTOM_DATA: u16 = 0x80; /* Not recognized */

/* S3M Adlib instrument types */
pub(crate) const S3M_INST_SAMPLE: u8 = 0x01;
pub(crate) const S3M_INST_AMEL: u8 = 0x02;
pub(crate) const S3M_INST_ABD: u8 = 0x03;
pub(crate) const S3M_INST_ASNARE: u8 = 0x04;
pub(crate) const S3M_INST_ATOM: u8 = 0x05;
pub(crate) const S3M_INST_ACYM: u8 = 0x06;
pub(crate) const S3M_INST_AHIHAT: u8 = 0x07;

/* S3M sample flags */
pub(crate) const S3M_SAMP_LOOP: u8 = 0x01;
pub(crate) const S3M_SAMP_STEREO: u8 = 0x02;
pub(crate) const S3M_SAMP_16BIT: u8 = 0x04;

/// Translation of `struct s3m_file_header`.
#[derive(Default)]
struct S3mFileHeader {
    name: [u8; 28],  /* Song name */
    type_: u8,       /* File type */
    ordnum: u16,     /* Number of orders (must be even) */
    insnum: u16,     /* Number of instruments */
    patnum: u16,     /* Number of patterns */
    flags: u16,      /* Flags */
    version: u16,    /* Tracker ID and version */
    ffi: u16,        /* File format information */
    magic: u32,      /* 'SCRM' */
    gv: u8,          /* Global volume */
    is: u8,          /* Initial speed */
    it: u8,          /* Initial tempo */
    mv: u8,          /* Master volume */
    uc: u8,          /* Ultra click removal */
    dp: u8,          /* Default pan positions if 0xfc */
    rsvd2: [u8; 8],  /* Reserved */
    special: u16,    /* Ptr to special custom data */
    chset: [u8; 32], /* Channel settings */
}

/// Translation of `struct s3m_instrument_header`.
#[derive(Default)]
struct S3mInstrumentHeader {
    dosname: [u8; 12], /* DOS file name */
    memseg_hi: u8,     /* High byte of sample pointer */
    memseg: u16,       /* Pointer to sample data */
    length: u32,       /* Length */
    loopbeg: u32,      /* Loop begin */
    loopend: u32,      /* Loop end */
    vol: u8,           /* Volume */
    pack: u8,          /* Packing type (not used) */
    flags: u8,         /* Loop/stereo/16bit samples flags */
    c2spd: u16,        /* C 4 speed */
    name: [u8; 28],    /* Instrument name */
    magic: u32,        /* 'SCRS' */
}

/// Translation of `struct s3m_adlib_header`.
#[derive(Default)]
struct S3mAdlibHeader {
    dosname: [u8; 12], /* DOS file name */
    reg: [u8; 12],     /* Adlib registers */
    vol: u8,
    dsk: u8,
    c2spd: u16,     /* C 4 speed */
    name: [u8; 28], /* Instrument name */
    magic: u32,     /* 'SCRI' */
}

const MAGIC_SCRM: u32 = magic4(b'S', b'C', b'R', b'M');
const MAGIC_SCRI: u32 = magic4(b'S', b'C', b'R', b'I');
const MAGIC_SCRS: u32 = magic4(b'S', b'C', b'R', b'S');

pub(crate) const LIBXMP_LOADER_S3M: FormatLoader = FormatLoader {
    name: "Scream Tracker 3",
    test: s3m_test,
    loader: s3m_load,
};

/// Translation of `s3m_test()`.
fn s3m_test(f: &mut HioHandle<'_>, t: Option<&mut [u8]>, start: i32) -> i32 {
    hio_seek(f, start as i64 + 44, SEEK_SET);
    if hio_read32b(f) != MAGIC_SCRM {
        return -1;
    }

    hio_seek(f, start as i64 + 29, SEEK_SET);
    if hio_read8(f) != 0x10 {
        return -1;
    }

    hio_seek(f, start as i64, SEEK_SET);
    libxmp_read_title(f, t, 28);

    0
}

const NONE: u8 = 0xff;
const FX_S3M_EXTENDED: u8 = 0xfe;

/* Effect conversion table */
const FX: [u8; 27] = [
    NONE,
    FX_S3M_SPEED,    /* Axx  Set speed to xx (the default is 06) */
    FX_JUMP,         /* Bxx  Jump to order xx (hexadecimal) */
    FX_BREAK,        /* Cxx  Break pattern to row xx (decimal) */
    FX_VOLSLIDE,     /* Dxy  Volume slide down by y/up by x */
    FX_PORTA_DN,     /* Exx  Slide down by xx */
    FX_PORTA_UP,     /* Fxx  Slide up by xx */
    FX_TONEPORTA,    /* Gxx  Tone portamento with speed xx */
    FX_VIBRATO,      /* Hxy  Vibrato with speed x and depth y */
    FX_TREMOR,       /* Ixy  Tremor with ontime x and offtime y */
    FX_S3M_ARPEGGIO, /* Jxy  Arpeggio with halfnote additions */
    FX_VIBRA_VSLIDE, /* Kxy  Dual command: H00 and Dxy */
    FX_TONE_VSLIDE,  /* Lxy  Dual command: G00 and Dxy */
    NONE,
    NONE,
    FX_OFFSET, /* Oxy  Set sample offset */
    NONE,
    FX_MULTI_RETRIG, /* Qxy  Retrig (+volumeslide) note */
    FX_TREMOLO,      /* Rxy  Tremolo with speed x and depth y */
    FX_S3M_EXTENDED, /* Sxx  (misc effects) */
    FX_S3M_BPM,      /* Txx  Tempo = xx (hex) */
    FX_FINE_VIBRATO, /* Uxx  Fine vibrato */
    FX_GLOBALVOL,    /* Vxx  Set global volume */
    NONE,
    FX_SETPAN, /* Xxx  Set pan */
    NONE,
    NONE,
];

/* Effect translation */
/// Translation of `xlat_fx()`.
fn xlat_fx(_c: i32, e: &mut XmpEvent) {
    let h = msn(e.fxp);
    let l = lsn(e.fxp);

    if e.fxt as usize >= FX.len() {
        e.fxt = 0;
        e.fxp = 0;
        return;
    }

    e.fxt = FX[e.fxt as usize];
    match e.fxt {
        FX_S3M_BPM => {
            if e.fxp < 0x20 {
                e.fxt = 0;
                e.fxp = 0;
            }
        }
        FX_S3M_EXTENDED => {
            /* Extended effects */
            e.fxt = FX_EXTENDED;
            match h {
                0x1 => {
                    /* Glissando */
                    e.fxp = (lsn(e.fxp) | (EX_GLISS << 4)) as u8;
                }
                0x2 => {
                    /* Finetune */
                    e.fxp = (((lsn(e.fxp) - 8) & 0x0f) | (EX_FINETUNE << 4)) as u8;
                }
                0x3 => {
                    /* Vibrato wave */
                    e.fxp = (lsn(e.fxp) | (EX_VIBRATO_WF << 4)) as u8;
                }
                0x4 => {
                    /* Tremolo wave */
                    e.fxp = (lsn(e.fxp) | (EX_TREMOLO_WF << 4)) as u8;
                }
                0x5 | 0x6 | 0x7 | 0x9 | 0xa => {
                    /* Ignore */
                    e.fxt = 0;
                    e.fxp = 0;
                }
                0x8 => {
                    /* Set pan */
                    e.fxt = FX_SETPAN;
                    e.fxp = (l << 4) as u8;
                }
                0xb => {
                    /* Pattern loop */
                    e.fxp = (lsn(e.fxp) | (EX_PATTERN_LOOP << 4)) as u8;
                }
                0xc => {
                    if l == 0 {
                        e.fxt = 0;
                        e.fxp = 0;
                    }
                }
                _ => {}
            }
        }
        FX_SETPAN => {
            /* Saga Musix says: "The X effect in S3M files is not
             * exclusive to IT and clones. You will find tons of S3Ms made
             * with ST3 itself using this effect (and relying on an
             * external player being used). X in S3M also behaves
             * differently than in IT, which your code does not seem to
             * handle: X00 - X80 is left... right, XA4 is surround (like
             * S91 in IT), other values are not supposed to do anything.
             */
            if e.fxp == 0xa4 {
                // surround
                e.fxt = FX_SURROUND;
                e.fxp = 1;
            } else {
                let mut pan = (e.fxp as i32) << 1;
                if pan > 0xff {
                    pan = 0xff;
                }
                e.fxp = pan as u8;
            }
        }
        NONE => {
            /* No effect */
            e.fxt = 0;
            e.fxp = 0;
        }
        _ => {}
    }
}

/// Translation of `s3m_load()`.
fn s3m_load(m: &mut ModuleData, f: &mut HioHandle<'_>, start: i32) -> i32 {
    let mut sfh = S3mFileHeader::default();
    let mut sih = S3mInstrumentHeader::default();
    let mut sah = S3mAdlibHeader::default();
    let tracker_name: String;
    let mut dummy = XmpEvent::default();
    let mut buf = [0u8; 96];

    if hio_read(&mut buf, 1, 96, f) != 96 {
        return -1;
    }

    sfh.name.copy_from_slice(&buf[..28]); /* Song name */
    sfh.type_ = buf[30]; /* File type */
    sfh.ordnum = readmem16l(&buf[32..]); /* Number of orders (must be even) */
    sfh.insnum = readmem16l(&buf[34..]); /* Number of instruments */
    sfh.patnum = readmem16l(&buf[36..]); /* Number of patterns */
    sfh.flags = readmem16l(&buf[38..]); /* Flags */
    sfh.version = readmem16l(&buf[40..]); /* Tracker ID and version */
    sfh.ffi = readmem16l(&buf[42..]); /* File format information */

    /* Sanity check */
    if sfh.ffi != 1 && sfh.ffi != 2 {
        return -1;
    }
    if sfh.ordnum > 255 || sfh.insnum > 255 || sfh.patnum > 255 {
        return -1;
    }

    sfh.magic = readmem32b(&buf[44..]); /* 'SCRM' */
    sfh.gv = buf[48]; /* Global volume */
    sfh.is = buf[49]; /* Initial speed */
    sfh.it = buf[50]; /* Initial tempo */
    sfh.mv = buf[51]; /* Master volume */
    sfh.uc = buf[52]; /* Ultra click removal */
    sfh.dp = buf[53]; /* Default pan positions if 0xfc */
    sfh.rsvd2.copy_from_slice(&buf[54..62]); /* Reserved */
    sfh.special = readmem16l(&buf[62..]); /* Ptr to special custom data */
    sfh.chset.copy_from_slice(&buf[64..96]); /* Channel settings */

    if sfh.magic != MAGIC_SCRM {
        return -1;
    }

    libxmp_copy_adjust(&mut m.mod_.name, &sfh.name, 28);

    let Ok(mut pp_ins) = try_vec(sfh.insnum as usize, 0u16) else {
        return -1;
    };

    let Ok(mut pp_pat) = try_vec(sfh.patnum as usize, 0u16) else {
        return -1;
    };

    if sfh.flags & S3M_AMIGA_RANGE != 0 {
        m.period_type = PERIOD_MODRNG;
    }
    if sfh.flags & S3M_ST300_VOLS != 0 {
        m.quirk |= QUIRK_VSALL;
    }
    /* m->volbase = 4096 / sfh.gv; */
    m.mod_.spd = sfh.is as i32;
    m.mod_.bpm = sfh.it as i32;
    m.mod_.chn = 0;

    /* Mix volume and stereo flag conversion (reported by Saga Musix).
     * 1) Old format uses mix volume 0-7, and the stereo flag is 0x10.
     * 2) Newer ST3s unconditionally convert MV 0x02 and 0x12 to 0x20.
     */
    m.mvolbase = 48;

    let stereo;
    if sfh.ffi == 1 {
        m.mvol = ((sfh.mv as i32 & 0xf) + 1) * 0x10;
        stereo = sfh.mv & 0x10;
        clamp(&mut m.mvol, 0x10, 0x7f);
    } else if sfh.mv == 0x02 || sfh.mv == 0x12 {
        m.mvol = 0x20;
        stereo = sfh.mv & 0x10;
    } else {
        m.mvol = (sfh.mv & S3M_MV_VOLUME) as i32;
        stereo = sfh.mv & S3M_MV_STEREO;

        if m.mvol == 0 {
            m.mvol = 48; /* Default is 48 */
        } else if m.mvol < 16 {
            m.mvol = 16; /* Minimum is 16 */
        }
    }

    /* "Note that in stereo, the mastermul is internally multiplied by
     * 11/8 inside the player since there is generally more room in the
     * output stream." Do the inverse to affect fewer modules. */
    if stereo == 0 {
        m.mvol = m.mvol * 8 / 11;
    }

    let mod_ = &mut m.mod_;
    for i in 0..32 {
        if sfh.chset[i] == S3M_CH_OFF {
            continue;
        }

        mod_.chn = i as i32 + 1;

        let x = sfh.chset[i] & S3M_CH_NUMBER;
        if stereo != 0 && x < S3M_CH_ADLIB {
            mod_.xxc[i].pan = if x < S3M_CH_RIGHT { 0x30 } else { 0xc0 };
        } else {
            mod_.xxc[i].pan = 0x80;
        }
    }

    if sfh.ordnum as usize <= XMP_MAX_MOD_LENGTH {
        mod_.len = sfh.ordnum as i32;
        let len = mod_.len as usize;
        if hio_read(&mut mod_.xxo, 1, len, f) != len {
            return -1;
        }
    } else {
        mod_.len = XMP_MAX_MOD_LENGTH as i32;
        let len = mod_.len as usize;
        if hio_read(&mut mod_.xxo, 1, len, f) != len {
            return -1;
        }
        if hio_seek(f, sfh.ordnum as i64 - XMP_MAX_MOD_LENGTH as i64, SEEK_CUR) < 0 {
            return -1;
        }
    }

    /* Don't trust sfh.patnum */
    mod_.pat = -1;
    for i in 0..mod_.len as usize {
        if mod_.xxo[i] < 0xfe && mod_.xxo[i] as i32 > mod_.pat {
            mod_.pat = mod_.xxo[i] as i32;
        }
    }
    mod_.pat += 1;
    if mod_.pat > sfh.patnum as i32 {
        mod_.pat = sfh.patnum as i32;
    }
    if mod_.pat == 0 {
        return -1;
    }

    mod_.trk = mod_.pat * mod_.chn;
    /* Load and convert header */
    mod_.ins = sfh.insnum as i32;
    mod_.smp = mod_.ins;

    for p in pp_ins.iter_mut() {
        *p = hio_read16l(f);
    }

    for p in pp_pat.iter_mut() {
        *p = hio_read16l(f);
    }

    /* Default pan positions */

    if sfh.dp == 0xfc {
        for i in 0..32 {
            let x = hio_read8(f);
            if x & S3M_PAN_SET != 0 {
                mod_.xxc[i].pan = ((x as i32) << 4) & 0xff;
            }
        }
    }

    m.c4rate = C4_NTSC_RATE;
    m.flow_mode = FLOW_MODE_ST3_321;

    if sfh.version == 0x1300 {
        m.quirk |= QUIRK_VSALL;
    }

    let ver_hi = (sfh.version & 0x0f00) >> 8;
    let ver_lo = sfh.version & 0xff;
    match sfh.version >> 12 {
        1 => {
            if sfh.version == 0x1320
                && sfh.special == 0
                && (sfh.ordnum & 0x0f) == 0
                && sfh.uc == 0
                && (sfh.flags & !0x50) == 0
                && sfh.dp == 0xfc
            {
                if (sfh.mv & 0x80) != 0 {
                    tracker_name = "ModPlug Tracker / OpenMPT 1.17".into();
                } else {
                    /* MPT 1.0 alpha5 doesn't set the stereo flag, but MPT 1.0 alpha6 does. */
                    tracker_name = "ModPlug Tracker 1.0 alpha".into();
                }
                m.flow_mode = FLOW_MODE_MPT_116;
            } else if sfh.version == 0x1320
                && sfh.special == 0
                && sfh.uc == 0
                && sfh.flags == 0
                && sfh.dp == 0
            {
                if sfh.gv == 64 && sfh.mv == 48 {
                    tracker_name = "PlayerPRO".into();
                } else {
                    // Always stereo
                    tracker_name = "Velvet Studio".into();
                }
            } else {
                tracker_name = format!("Scream Tracker {}.{:02x}", ver_hi, ver_lo);
                m.quirk |= QUIRK_ST3BUGS;
                if sfh.version < 0x1303 {
                    m.flow_mode = FLOW_MODE_ST3_301;
                }
            }
        }
        2 => {
            if sfh.version == 0x2013 {
                tracker_name = "PlayerPRO".into(); /* PlayerPRO on Intel doesn't byte-swap the tracker ID bytes */
            } else {
                tracker_name = format!("Imago Orpheus {}.{:02x}", ver_hi, ver_lo);
                m.flow_mode = FLOW_MODE_ORPHEUS;
            }
        }
        3 => {
            m.flow_mode = FLOW_MODE_IT_210;
            if sfh.version == 0x3216 {
                tracker_name = "Impulse Tracker 2.14v3".into();
            } else if sfh.version == 0x3217 {
                tracker_name = "Impulse Tracker 2.14v5".into();
            } else {
                tracker_name = format!("Impulse Tracker {}.{:02x}", ver_hi, ver_lo);
            }
        }
        5 => {
            if sfh.version == 0x5447 {
                tracker_name = "Graoumf Tracker".into();
            } else if sfh.rsvd2[0] != 0 || sfh.rsvd2[1] != 0 {
                tracker_name = format!(
                    "OpenMPT {}.{:02x}.{:02x}.{:02x}",
                    ver_hi, ver_lo, sfh.rsvd2[1], sfh.rsvd2[0]
                );
            } else {
                tracker_name = format!("OpenMPT {}.{:02x}", ver_hi, ver_lo);
            }
            m.quirk |= QUIRK_ST3BUGS;
        }
        4 if sfh.version != 0x4100 => {
            let mut name = [0u8; 40];
            libxmp_schism_tracker_string(
                &mut name,
                40,
                (sfh.version & 0x0fff) as i32,
                sfh.rsvd2[0] as i32 | ((sfh.rsvd2[1] as i32) << 8),
            );
            tracker_name = String::from_utf8_lossy(cstr(&name)).into_owned();
        }
        4 | 6 => {
            /* (4 falls through) */
            tracker_name = format!("BeRoTracker {}.{:02x}", ver_hi, ver_lo);
        }
        _ => {
            tracker_name = format!("unknown ({:04x})", sfh.version);
        }
    }

    /* (snprintf(tracker_name, 40, ...) keeps 39 bytes) */
    let tn = &tracker_name.as_bytes()[..tracker_name.len().min(39)];
    let mut type_ = tn.to_vec();
    type_.extend_from_slice(b" S3M");
    libxmp_set_type(m, &type_);

    let mod_ = &mut m.mod_;
    if libxmp_init_pattern(mod_) < 0 {
        return -1;
    }

    /* Read patterns */

    for i in 0..mod_.pat {
        if libxmp_alloc_pattern_tracks(mod_, i, 64) < 0 {
            return -1;
        }

        if pp_pat[i as usize] == 0 {
            continue;
        }

        hio_seek(f, start as i64 + pp_pat[i as usize] as i64 * 16, SEEK_SET);
        let mut r = 0;
        let mut pat_len = hio_read16l(f) as i32 - 2;

        while pat_len >= 0 && r < mod_.pattern(i).rows {
            let b = hio_read8(f);

            if hio_error(f) != 0 {
                return -1;
            }

            if b == S3M_EOR {
                r += 1;
                continue;
            }

            let c = (b & S3M_CH_MASK) as i32;
            let event = if c >= mod_.chn {
                &mut dummy
            } else {
                mod_.event_mut(i, c, r)
            };

            if b & S3M_NI_FOLLOW != 0 {
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
                        n = (13 + 12 * msn(n) + lsn(n)) as u8;
                    }
                }
                event.note = n;
                event.ins = hio_read8(f);
                pat_len -= 2;
            }

            if b & S3M_VOL_FOLLOWS != 0 {
                event.vol = hio_read8(f).wrapping_add(1);
                pat_len -= 1;
            }

            if b & S3M_FX_FOLLOWS != 0 {
                event.fxt = hio_read8(f);
                event.fxp = hio_read8(f);
                xlat_fx(c, event);

                pat_len -= 2;
            }
        }
    }

    if libxmp_init_instrument(m) < 0 {
        return -1;
    }

    /* Read and convert instruments and samples */

    for i in 0..m.mod_.ins {
        let iu = i as usize;
        let Ok(sub) = try_vec(1, XmpSubinstrument::default()) else {
            return -1;
        };
        m.mod_.xxi[iu].sub = sub;

        hio_seek(f, start as i64 + pp_ins[iu] as i64 * 16, SEEK_SET);
        {
            let sub = &mut m.mod_.xxi[iu].sub[0];
            sub.pan = XMP_INST_NO_DEFAULT_PAN;
            sub.sid = i;
        }

        if hio_read(&mut buf, 1, 80, f) != 80 {
            return -1;
        }

        if buf[0] >= 2 {
            /* OPL2 FM instrument */

            sah.dosname.copy_from_slice(&buf[1..13]); /* DOS file name */
            sah.reg.copy_from_slice(&buf[16..28]); /* Adlib registers */
            sah.vol = buf[28];
            sah.dsk = buf[29];
            sah.c2spd = readmem16l(&buf[32..]); /* C4 speed */
            sah.name.copy_from_slice(&buf[48..76]); /* Instrument name */
            sah.magic = readmem32b(&buf[76..]); /* 'SCRI' */

            if sah.magic != MAGIC_SCRI {
                return -1;
            }
            sah.magic = 0;

            libxmp_instrument_name(&mut m.mod_, i, &sah.name, 28);

            let xxi = &mut m.mod_.xxi[iu];
            xxi.nsm = 1;
            let sub = &mut xxi.sub[0];
            sub.vol = sah.vol as i32;
            libxmp_c2spd_to_note(sah.c2spd as i32, &mut sub.xpo, &mut sub.fin);
            sub.xpo += 12;
            let smpctl = m.smpctl;
            let ret = libxmp_load_sample(
                smpctl,
                Some(f),
                SAMPLE_FLAG_ADLIB,
                &mut m.mod_.xxs[iu],
                Some(&sah.reg),
            );
            if ret < 0 {
                return -1;
            }

            continue;
        }

        sih.dosname.copy_from_slice(&buf[1..13]); /* DOS file name */
        sih.memseg_hi = buf[13]; /* High byte of sample pointer */
        sih.memseg = readmem16l(&buf[14..]); /* Pointer to sample data */
        sih.length = readmem32l(&buf[16..]); /* Length */

        /* (upstream keeps the ST3 limit of 64000 bytes here under #if 0) */

        if sih.length > MAX_SAMPLE_SIZE as u32 {
            return -1;
        }

        sih.loopbeg = readmem32l(&buf[20..]); /* Loop begin */
        sih.loopend = readmem32l(&buf[24..]); /* Loop end */
        sih.vol = buf[28]; /* Volume */
        sih.pack = buf[30]; /* Packing type */
        sih.flags = buf[31]; /* Loop/stereo/16bit flags */
        sih.c2spd = readmem16l(&buf[32..]); /* C4 speed */
        sih.name.copy_from_slice(&buf[48..76]); /* Instrument name */
        sih.magic = readmem32b(&buf[76..]); /* 'SCRS' */

        if buf[0] == 1 && sih.magic != MAGIC_SCRS {
            return -1;
        }

        let xxs = &mut m.mod_.xxs[iu];
        xxs.len = sih.length as i32;
        m.mod_.xxi[iu].nsm = if sih.length > 0 { 1 } else { 0 };
        let xxs = &mut m.mod_.xxs[iu];
        xxs.lps = sih.loopbeg as i32;
        xxs.lpe = sih.loopend as i32;

        xxs.flg = if sih.flags & S3M_SAMP_LOOP != 0 {
            XMP_SAMPLE_LOOP
        } else {
            0
        };

        if sih.flags & S3M_SAMP_STEREO != 0 {
            xxs.flg |= XMP_SAMPLE_STEREO;
        }
        if sih.flags & S3M_SAMP_16BIT != 0 {
            xxs.flg |= XMP_SAMPLE_16BIT;
        }

        let mut load_sample_flags = if sfh.ffi == 1 { 0 } else { SAMPLE_FLAG_UNS };
        if sih.pack == 4 {
            load_sample_flags = SAMPLE_FLAG_ADPCM;
        }

        m.mod_.xxi[iu].sub[0].vol = sih.vol as i32;
        sih.magic = 0;

        libxmp_instrument_name(&mut m.mod_, i, &sih.name, 28);

        {
            let sub = &mut m.mod_.xxi[iu].sub[0];
            libxmp_c2spd_to_note(sih.c2spd as i32, &mut sub.xpo, &mut sub.fin);
        }

        let sample_segment = sih.memseg as u32 + ((sih.memseg_hi as u32) << 16);

        if hio_seek(f, start as i64 + 16 * sample_segment as i64, SEEK_SET) < 0 {
            return -1;
        }

        let smpctl = m.smpctl;
        let ret = libxmp_load_sample(
            smpctl,
            Some(f),
            load_sample_flags,
            &mut m.mod_.xxs[iu],
            None,
        );
        if ret < 0 {
            return -1;
        }
    }

    m.quirk |= QUIRKS_ST3 | QUIRK_ARPMEM;
    m.read_event_type = READ_EVENT_ST3;

    0
}
