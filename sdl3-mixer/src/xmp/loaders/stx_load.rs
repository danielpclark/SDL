// Rust translation of src/loaders/stx_load.c from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2026 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

/* From the STMIK 0.2 documentation:
 *
 * "The STMIK uses a special [Scream Tracker] beta-V3.0 module format.
 *  Due to the module formats beta nature, the current STMIK uses a .STX
 *  extension instead of the normal .STM. I'm not intending to do a
 *  STX->STM converter, so treat STX as the format to be used in finished
 *  programs, NOT as a format to be used in distributing modules. A program
 *  called STM2STX is included, and it'll convert STM modules to the STX
 *  format for usage in your own programs."
 *
 * Tested using "Future Brain" from Mental Surgery by Future Crew and
 * STMs converted with STM2STX.
 */

//! The STMIK 0.2 (STX) loader.

use super::super::common::*;
use super::super::effects::*;
use super::super::hio::*;
use super::super::period::libxmp_c2spd_to_note;
use super::super::*;
use super::common::*;
use super::s3m_load::*;
use super::sample::libxmp_load_sample;
use super::*;

/// Translation of `struct stx_file_header`.
#[derive(Clone, Copy, Default)]
struct StxFileHeader {
    name: [u8; 20], /* Song name */
    magic: [u8; 8], /* !Scream! */
    psize: u16,     /* Pattern 0 size? */
    unknown1: u16,  /* ?! */
    pp_pat: u16,    /* Pointer to pattern table */
    pp_ins: u16,    /* Pattern to instrument table */
    pp_chn: u16,    /* Pointer to channel table (?) */
    unknown2: u16,
    unknown3: u16,
    gvol: u8,  /* Global volume */
    tempo: u8, /* Playback tempo */
    unknown4: u16,
    unknown5: u16,
    patnum: u16,     /* Number of patterns */
    insnum: u16,     /* Number of instruments */
    ordnum: u16,     /* Number of orders */
    unknown6: u16,   /* Flags? */
    unknown7: u16,   /* Version? */
    unknown8: u16,   /* Ffi? */
    magic2: [u8; 4], /* 'SCRM' */
}

/// Translation of `struct stx_instrument_header`.
#[derive(Clone, Copy, Default)]
struct StxInstrumentHeader {
    type_: u8,         /* Instrument type */
    dosname: [u8; 13], /* DOS file name */
    memseg: u16,       /* Pointer to sample data */
    length: u32,       /* Length */
    loopbeg: u32,      /* Loop begin */
    loopend: u32,      /* Loop end */
    vol: u8,           /* Volume */
    rsvd1: u8,         /* Reserved */
    pack: u8,          /* Packing type (not used) */
    flags: u8,         /* Loop/stereo/16bit samples flags */
    c2spd: u16,        /* C 4 speed */
    rsvd2: u16,        /* Reserved */
    rsvd3: [u8; 4],    /* Reserved */
    int_gp: u16,       /* Internal - GUS pointer */
    int_512: u16,      /* Internal - SB pointer */
    int_last: u32,     /* Internal - SB index */
    name: [u8; 28],    /* Instrument name */
    magic: [u8; 4],    /* Reserved (for 'SCRS') */
}

pub(crate) const LIBXMP_LOADER_STX: FormatLoader = FormatLoader {
    name: "STMIK 0.2",
    test: stx_test,
    loader: stx_load,
};

/// Translation of `stx_test()`.
fn stx_test(f: &mut HioHandle<'_>, t: Option<&mut [u8]>, start: i32) -> i32 {
    let mut buf = [0u8; 8];

    hio_seek(f, start as i64 + 20, SEEK_SET);
    if hio_read(&mut buf, 1, 8, f) < 8 {
        return -1;
    }
    if &buf != b"!Scream!" && &buf != b"BMOD2STM" {
        return -1;
    }

    hio_seek(f, start as i64 + 60, SEEK_SET);
    if hio_read(&mut buf, 1, 4, f) < 4 {
        return -1;
    }
    if &buf[..4] != b"SCRM" {
        return -1;
    }

    hio_seek(f, start as i64, SEEK_SET);
    libxmp_read_title(f, t, 20);

    0
}

const FX_NONE: u8 = 0xff;
static FX: [u8; 11] = [
    FX_NONE,
    FX_SPEED,
    FX_JUMP,
    FX_BREAK,
    FX_VOLSLIDE,
    FX_PORTA_DN,
    FX_PORTA_UP,
    FX_TONEPORTA,
    FX_VIBRATO,
    FX_TREMOR,
    FX_ARPEGGIO,
];

/// Translation of `stx_load()`.
fn stx_load(m: &mut ModuleData, f: &mut HioHandle<'_>, start: i32) -> i32 {
    let mut broken = false;
    let mut dummy = XmpEvent::default();
    let mut sfh = StxFileHeader::default();
    let mut sih = StxInstrumentHeader::default();
    let mut bmod2stm = false;
    let start = start as i64;

    hio_read(&mut sfh.name, 20, 1, f);
    hio_read(&mut sfh.magic, 8, 1, f);
    sfh.psize = hio_read16l(f);
    sfh.unknown1 = hio_read16l(f);
    sfh.pp_pat = hio_read16l(f);
    sfh.pp_ins = hio_read16l(f);
    sfh.pp_chn = hio_read16l(f);
    sfh.unknown2 = hio_read16l(f);
    sfh.unknown3 = hio_read16l(f);
    sfh.gvol = hio_read8(f);
    sfh.tempo = hio_read8(f);
    sfh.unknown4 = hio_read16l(f);
    sfh.unknown5 = hio_read16l(f);
    sfh.patnum = hio_read16l(f);
    sfh.insnum = hio_read16l(f);
    sfh.ordnum = hio_read16l(f);
    sfh.unknown6 = hio_read16l(f);
    sfh.unknown7 = hio_read16l(f);
    sfh.unknown8 = hio_read16l(f);
    hio_read(&mut sfh.magic2, 4, 1, f);

    /* Sanity check */
    if sfh.patnum > 254 || sfh.insnum as i32 > MAX_INSTRUMENTS || sfh.ordnum > 256 {
        return -1;
    }

    /* BMOD2STM does not convert pitch */
    if strneq(&sfh.magic, b"BMOD2STM", 8) {
        bmod2stm = true;
    }

    /* (upstream keeps a check of both magics here under #if 0) */

    let mod_ = &mut m.mod_;
    mod_.ins = sfh.insnum as i32;
    mod_.pat = sfh.patnum as i32;
    mod_.trk = mod_.pat * mod_.chn;
    mod_.len = sfh.ordnum as i32;
    mod_.spd = msn(sfh.tempo);
    mod_.smp = mod_.ins;
    m.c4rate = C4_NTSC_RATE;

    /* STM2STX 1.0 released with STMIK 0.2 converts STMs with the pattern
     * length encoded in the first two bytes of the pattern (like S3M).
     */
    hio_seek(f, start + ((sfh.pp_pat as i64) << 4), SEEK_SET);
    let mut x16 = hio_read16l(f);
    hio_seek(f, start + ((x16 as i64) << 4), SEEK_SET);
    x16 = hio_read16l(f);
    if x16 == sfh.psize {
        broken = true;
    }

    strncpy(&mut m.mod_.name, &sfh.name, 20);
    if bmod2stm {
        libxmp_set_type(m, b"BMOD2STM STX");
    } else {
        let s = format!("STM2STX 1.{}", if broken { 0 } else { 1 });
        snprintf_bytes(&mut m.mod_.type_, XMP_NAME_SIZE, s.as_bytes());
    }

    let mod_ = &mut m.mod_;
    let Ok(mut pp_pat) = try_vec(mod_.pat as usize, 0u16) else {
        return -1;
    };

    let Ok(mut pp_ins) = try_vec(mod_.ins as usize, 0u16) else {
        return -1;
    };

    /* Read pattern pointers */
    hio_seek(f, start + ((sfh.pp_pat as i64) << 4), SEEK_SET);
    for p in pp_pat.iter_mut() {
        *p = hio_read16l(f);
    }

    /* Read instrument pointers */
    hio_seek(f, start + ((sfh.pp_ins as i64) << 4), SEEK_SET);
    for p in pp_ins.iter_mut() {
        *p = hio_read16l(f);
    }

    /* Skip channel table (?) */
    hio_seek(f, start + ((sfh.pp_chn as i64) << 4) + 32, SEEK_SET);

    /* Read orders */
    for i in 0..mod_.len as usize {
        mod_.xxo[i] = hio_read8(f);
        hio_seek(f, 4, SEEK_CUR);
    }

    if libxmp_init_instrument(m) < 0 {
        return -1;
    }

    let mod_ = &mut m.mod_;
    /* Read and convert instruments and samples */
    for i in 0..mod_.ins {
        let iu = i as usize;
        if libxmp_alloc_subinstrument(mod_, i, 1) < 0 {
            return -1;
        }

        hio_seek(f, start + ((pp_ins[iu] as i64) << 4), SEEK_SET);

        sih.type_ = hio_read8(f);
        hio_read(&mut sih.dosname, 13, 1, f);
        sih.memseg = hio_read16l(f);
        sih.length = hio_read32l(f);
        sih.loopbeg = hio_read32l(f);
        sih.loopend = hio_read32l(f);
        sih.vol = hio_read8(f);
        sih.rsvd1 = hio_read8(f);
        sih.pack = hio_read8(f);
        sih.flags = hio_read8(f);
        sih.c2spd = hio_read16l(f);
        sih.rsvd2 = hio_read16l(f);
        hio_read(&mut sih.rsvd3, 4, 1, f);
        sih.int_gp = hio_read16l(f);
        sih.int_512 = hio_read16l(f);
        sih.int_last = hio_read32l(f);
        hio_read(&mut sih.name, 28, 1, f);
        hio_read(&mut sih.magic, 4, 1, f);
        if hio_error(f) != 0 {
            return -1;
        }

        let xxs = &mut mod_.xxs[iu];
        xxs.len = sih.length as i32;
        xxs.lps = sih.loopbeg as i32;
        xxs.lpe = sih.loopend as i32;
        if xxs.lpe == 0xffff {
            xxs.lpe = 0;
        }
        xxs.flg = if xxs.lpe > 0 { XMP_SAMPLE_LOOP } else { 0 };
        let xxi = &mut mod_.xxi[iu];
        xxi.sub[0].vol = sih.vol as i32;
        xxi.sub[0].pan = XMP_INST_NO_DEFAULT_PAN;
        xxi.sub[0].sid = i;
        xxi.nsm = 1;

        libxmp_instrument_name(mod_, i, &sih.name, 12);

        let sub = &mut mod_.xxi[iu].sub[0];
        libxmp_c2spd_to_note(sih.c2spd as i32, &mut sub.xpo, &mut sub.fin);
    }

    if libxmp_init_pattern(mod_) < 0 {
        return -1;
    }

    /* Read and convert patterns */

    for i in 0..mod_.pat {
        if libxmp_alloc_pattern_tracks(mod_, i, 64) < 0 {
            return -1;
        }

        if pp_pat[i as usize] == 0 {
            continue;
        }

        hio_seek(f, start + ((pp_pat[i as usize] as i64) << 4), SEEK_SET);
        if broken {
            hio_seek(f, 2, SEEK_CUR);
        }

        let mut r = 0;
        while r < 64 {
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
                        n = (37 + 12 * msn(n) + lsn(n)) as u8;
                    }
                }

                event.note = n;
                event.ins = hio_read8(f);
            }

            if b & S3M_VOL_FOLLOWS != 0 {
                event.vol = hio_read8(f).wrapping_add(1);
            }

            if b & S3M_FX_FOLLOWS != 0 {
                let t = hio_read8(f) as usize;
                let p = hio_read8(f);
                if t < FX.len() {
                    event.fxt = FX[t];
                    event.fxp = p;
                    match event.fxt {
                        FX_SPEED => {
                            event.fxp = msn(event.fxp) as u8;
                        }
                        FX_NONE => {
                            event.fxp = 0;
                            event.fxt = 0;
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    drop(pp_ins);
    drop(pp_pat);

    /* Read samples */

    for i in 0..m.mod_.ins as usize {
        let smpctl = m.smpctl;
        if libxmp_load_sample(smpctl, Some(f), 0, &mut m.mod_.xxs[i], None) < 0 {
            return -1;
        }
    }

    m.quirk |= QUIRK_VSALL | QUIRKS_ST3;
    m.read_event_type = READ_EVENT_ST3;

    0
}
