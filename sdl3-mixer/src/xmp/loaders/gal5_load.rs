// Rust translation of src/loaders/gal5_load.c from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2026 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

/* Galaxy Music System 5.0 module file loader
 *
 * Based on the format description by Dr.Eggman
 * (http://www.jazz2online.com/J2Ov2/articles/view.php?articleID=288)
 * and Jazz Jackrabbit modules by Alexander Brandon from Lori Central
 * (http://www.loricentral.com/jj2music.html)
 */

//! The Galaxy Music System 5.0 (J2B) loader.

use super::super::common::*;
use super::super::effects::*;
use super::super::hio::*;
use super::super::period::libxmp_c2spd_to_note;
use super::super::*;
use super::common::*;
use super::iff::*;
use super::sample::libxmp_load_sample;
use super::*;

pub(crate) const LIBXMP_LOADER_GAL5: FormatLoader = FormatLoader {
    name: "Galaxy Music System 5.0 (J2B)",
    test: gal5_test,
    loader: gal5_load,
};

/// Translation of `struct local_data`.
struct LocalData {
    chn_pan: [u8; 64],
}

/// Translation of `gal5_test()`.
fn gal5_test(f: &mut HioHandle<'_>, t: Option<&mut [u8]>, _start: i32) -> i32 {
    if hio_read32b(f) != magic4(b'R', b'I', b'F', b'F') {
        return -1;
    }

    hio_read32b(f);

    if hio_read32b(f) != magic4(b'A', b'M', b' ', b' ') {
        return -1;
    }

    if hio_read32b(f) != magic4(b'I', b'N', b'I', b'T') {
        return -1;
    }

    hio_read32b(f); /* skip size */
    libxmp_read_title(f, t, 64);

    0
}

/// Translation of `get_init()`.
fn get_init(m: &mut ModuleData, _size: u32, f: &mut HioHandle<'_>, data: &mut LocalData) -> i32 {
    let mut buf = [0u8; 64];

    if hio_read(&mut buf, 1, 64, f) < 64 {
        return -1;
    }
    strncpy(&mut m.mod_.name, &buf, 63); /* ensure string terminator */
    m.mod_.name[63] = b'\0';
    libxmp_set_type(m, b"Galaxy Music System 5.0");
    let flags = hio_read8(f); /* bit 0: Amiga period */
    if !flags & 0x01 != 0 {
        m.period_type = PERIOD_LINEAR;
    }
    let mod_ = &mut m.mod_;
    mod_.chn = hio_read8(f) as i32;
    mod_.spd = hio_read8(f) as i32;
    mod_.bpm = hio_read8(f) as i32;
    hio_read16l(f); /* unknown - 0x01c5 */
    hio_read16l(f); /* unknown - 0xff00 */
    hio_read8(f); /* unknown - 0x80 */

    if hio_read(&mut data.chn_pan, 1, 64, f) != 64 {
        return -1;
    }

    /* Sanity check */
    if mod_.chn > XMP_MAX_CHANNELS as i32 {
        return -1;
    }

    0
}

/// Translation of `get_ordr()`.
fn get_ordr(m: &mut ModuleData, _size: u32, f: &mut HioHandle<'_>, _data: &mut LocalData) -> i32 {
    let mod_ = &mut m.mod_;

    mod_.len = hio_read8(f) as i32 + 1;
    /* Don't follow Dr.Eggman's specs here */

    for i in 0..mod_.len as usize {
        mod_.xxo[i] = hio_read8(f);
    }

    0
}

/// Translation of `get_patt_cnt()`.
fn get_patt_cnt(
    m: &mut ModuleData,
    _size: u32,
    f: &mut HioHandle<'_>,
    _data: &mut LocalData,
) -> i32 {
    let mod_ = &mut m.mod_;

    let i = hio_read8(f) as i32 + 1; /* pattern number */

    if i > mod_.pat {
        mod_.pat = i;
    }

    0
}

/// Translation of `get_inst_cnt()`.
fn get_inst_cnt(
    m: &mut ModuleData,
    _size: u32,
    f: &mut HioHandle<'_>,
    _data: &mut LocalData,
) -> i32 {
    let mod_ = &mut m.mod_;

    hio_read32b(f); /* 42 01 00 00 */
    hio_read8(f); /* 00 */
    let i = hio_read8(f) as i32 + 1; /* instrument number */

    /* Sanity check */
    if i > MAX_INSTRUMENTS {
        return -1;
    }

    if i > mod_.ins {
        mod_.ins = i;
    }

    0
}

/// Translation of `get_patt()`.
fn get_patt(m: &mut ModuleData, _size: u32, f: &mut HioHandle<'_>, _data: &mut LocalData) -> i32 {
    let mod_ = &mut m.mod_;
    let mut dummy = XmpEvent::default();

    let i = hio_read8(f) as i32; /* pattern number */
    let len = hio_read32l(f) as i32;

    let rows = hio_read8(f) as i32 + 1;

    /* Sanity check - don't allow duplicate patterns. */
    /* (the first pass counted the pattern) */
    let Some(Some(exists)) = mod_
        .xxp
        .as_ref()
        .map(|x| x.get(i as usize).map(|p| p.is_some()))
    else {
        return -1;
    };
    if len < 0 || exists {
        return -1;
    }

    if libxmp_alloc_pattern_tracks(mod_, i, rows) < 0 {
        return -1;
    }

    let mut r = 0;
    while r < rows {
        let flag = hio_read8(f);
        if flag == 0 {
            r += 1;
            continue;
        }
        if hio_error(f) != 0 {
            return -1;
        }

        let chan = (flag & 0x1f) as i32;

        let event = if chan < mod_.chn {
            mod_.event_mut(i, chan, r)
        } else {
            &mut dummy
        };

        if flag & 0x80 != 0 {
            let mut fxp = hio_read8(f);
            let mut fxt = hio_read8(f);

            match fxt {
                0x14 => {
                    /* speed */
                    fxt = FX_S3M_SPEED;
                }
                _ => {
                    if fxt > 0x0f {
                        fxt = 0;
                        fxp = 0;
                    }
                }
            }

            event.fxt = fxt;
            event.fxp = fxp;
        }

        if flag & 0x40 != 0 {
            event.ins = hio_read8(f);
            event.note = hio_read8(f);

            if event.note == 128 {
                event.note = XMP_KEY_OFF;
            }
        }

        if flag & 0x20 != 0 {
            event.vol = 1 + hio_read8(f) / 2;
        }
    }

    0
}

/// Translation of `get_inst()`.
fn get_inst(m: &mut ModuleData, _size: u32, f: &mut HioHandle<'_>, _data: &mut LocalData) -> i32 {
    hio_read32b(f); /* 42 01 00 00 */
    hio_read8(f); /* 00 */
    let i = hio_read8(f) as usize; /* instrument number */

    /* Sanity check - don't allow duplicate instruments. */
    /* (the first pass counted the instrument) */
    if i >= m.mod_.xxi.len() || m.mod_.xxi[i].nsm != 0 {
        return -1;
    }

    let mod_ = &mut m.mod_;
    let xxi = &mut mod_.xxi[i];

    hio_read(&mut xxi.name, 1, 28, f);
    hio_seek(f, 290, SEEK_CUR); /* Sample/note map, envelopes */
    xxi.nsm = hio_read16l(f) as i32;

    if xxi.nsm == 0 {
        return 0;
    }

    let nsm = xxi.nsm;
    if libxmp_alloc_subinstrument(mod_, i as i32, nsm) < 0 {
        return -1;
    }

    /* FIXME: Currently reading only the first sample */
    let xxi = &mut mod_.xxi[i];
    let sub = &mut xxi.sub[0];
    let xxs = &mut mod_.xxs[i];

    hio_read32b(f); /* RIFF */
    hio_read32b(f); /* size */
    hio_read32b(f); /* AS   */
    hio_read32b(f); /* SAMP */
    hio_read32b(f); /* size */
    hio_read32b(f); /* unknown - usually 0x40000000 */

    hio_read(&mut xxs.name, 1, 28, f);

    hio_read32b(f); /* unknown - 0x0000 */
    hio_read8(f); /* unknown - 0x00 */

    sub.sid = i as i32;
    xxi.vol = hio_read8(f) as i32;
    sub.pan = XMP_INST_NO_DEFAULT_PAN;
    sub.vol = (hio_read16l(f) as i32 + 1) / 512;
    let flags = hio_read16l(f);
    hio_read16l(f); /* unknown - 0x0080 */
    xxs.len = hio_read32l(f) as i32;
    xxs.lps = hio_read32l(f) as i32;
    xxs.lpe = hio_read32l(f) as i32;

    xxs.flg = 0;
    let mut has_unsigned_sample = false;
    if flags & 0x04 != 0 {
        xxs.flg |= XMP_SAMPLE_16BIT;
    }
    if flags & 0x08 != 0 {
        xxs.flg |= XMP_SAMPLE_LOOP;
    }
    if flags & 0x10 != 0 {
        xxs.flg |= XMP_SAMPLE_LOOP | XMP_SAMPLE_LOOP_BIDIR;
    }
    if !flags & 0x80 != 0 {
        has_unsigned_sample = true;
    }

    let srate = hio_read32l(f) as i32;
    let finetune = 0;
    libxmp_c2spd_to_note(srate, &mut sub.xpo, &mut sub.fin);
    sub.fin += finetune;

    hio_read32l(f); /* 0x00000000 */
    hio_read32l(f); /* unknown */

    if xxs.len > 1 {
        let smpctl = m.smpctl;
        if libxmp_load_sample(
            smpctl,
            Some(f),
            if has_unsigned_sample {
                SAMPLE_FLAG_UNS
            } else {
                0
            },
            &mut m.mod_.xxs[i],
            None,
        ) < 0
        {
            return -1;
        }
    }

    0
}

/// Translation of `gal5_load()`.
fn gal5_load(m: &mut ModuleData, f: &mut HioHandle<'_>, start: i32) -> i32 {
    let mut data = LocalData { chn_pan: [0; 64] };

    hio_read32b(f); /* Skip RIFF */
    hio_read32b(f); /* Skip size */
    hio_read32b(f); /* Skip AM   */

    let offset = hio_tell(f) as i32;

    m.mod_.ins = 0;
    m.mod_.smp = 0;

    let mut handle = libxmp_iff_new::<LocalData>();

    m.c4rate = C4_NTSC_RATE;

    /* IFF chunk IDs */
    let mut ret = libxmp_iff_register(&mut handle, b"INIT", get_init); /* Galaxy 5.0 */
    ret |= libxmp_iff_register(&mut handle, b"ORDR", get_ordr);
    ret |= libxmp_iff_register(&mut handle, b"PATT", get_patt_cnt);
    ret |= libxmp_iff_register(&mut handle, b"INST", get_inst_cnt);

    if ret != 0 {
        return -1;
    }

    libxmp_iff_set_quirk(&mut handle, IFF_LITTLE_ENDIAN);
    libxmp_iff_set_quirk(&mut handle, IFF_SKIP_EMBEDDED);
    libxmp_iff_set_quirk(&mut handle, IFF_CHUNK_ALIGN2);

    /* Load IFF chunks */
    if libxmp_iff_load(&handle, m, f, &mut data) < 0 {
        libxmp_iff_release(handle);
        return -1;
    }

    libxmp_iff_release(handle);

    let mod_ = &mut m.mod_;
    mod_.trk = mod_.pat * mod_.chn;
    mod_.smp = mod_.ins;

    if libxmp_init_instrument(m) < 0 {
        return -1;
    }

    if libxmp_init_pattern(&mut m.mod_) < 0 {
        return -1;
    }

    hio_seek(f, start as i64 + offset as i64, SEEK_SET);

    let mut handle = libxmp_iff_new::<LocalData>();

    /* IFF chunk IDs */
    let mut ret = libxmp_iff_register(&mut handle, b"PATT", get_patt);
    ret |= libxmp_iff_register(&mut handle, b"INST", get_inst);

    if ret != 0 {
        return -1;
    }

    libxmp_iff_set_quirk(&mut handle, IFF_LITTLE_ENDIAN);
    libxmp_iff_set_quirk(&mut handle, IFF_SKIP_EMBEDDED);
    libxmp_iff_set_quirk(&mut handle, IFF_CHUNK_ALIGN2);

    /* Load IFF chunks */
    if libxmp_iff_load(&handle, m, f, &mut data) < 0 {
        libxmp_iff_release(handle);
        return -1;
    }

    libxmp_iff_release(handle);

    let mod_ = &mut m.mod_;
    /* Alloc missing patterns */
    for i in 0..mod_.pat {
        if mod_.xxp.as_ref().is_some_and(|x| x[i as usize].is_none())
            && libxmp_alloc_pattern_tracks(mod_, i, 64) < 0
        {
            return -1;
        }
    }

    for i in 0..mod_.chn as usize {
        mod_.xxc[i].pan = data.chn_pan[i] as i32 * 2;
    }

    /* TODO: JJ2 1.24 Secret Files shareware does not support fine slides
     * from converted XMs. Should be tested with converted S3Ms, though. */
    m.quirk |= QUIRK_FINEFX;
    m.read_event_type = READ_EVENT_FT2;

    0
}
