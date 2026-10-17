// Rust translation of src/loaders/gal4_load.c from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2026 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

/* Galaxy Music System 4.0 module file loader
 *
 * Based on modules converted using mod2j2b.exe
 */

//! The Galaxy Music System 4.0 loader.

use super::super::common::*;
use super::super::dataio::*;
use super::super::effects::*;
use super::super::hio::*;
use super::super::period::libxmp_c2spd_to_note;
use super::super::*;
use super::common::*;
use super::iff::*;
use super::sample::libxmp_load_sample;
use super::*;

pub(crate) const LIBXMP_LOADER_GAL4: FormatLoader = FormatLoader {
    name: "Galaxy Music System 4.0",
    test: gal4_test,
    loader: gal4_load,
};

/// Translation of `gal4_test()`.
fn gal4_test(f: &mut HioHandle<'_>, t: Option<&mut [u8]>, _start: i32) -> i32 {
    if hio_read32b(f) != magic4(b'R', b'I', b'F', b'F') {
        return -1;
    }

    hio_read32b(f);

    if hio_read32b(f) != magic4(b'A', b'M', b'F', b'F') {
        return -1;
    }

    if hio_read32b(f) != magic4(b'M', b'A', b'I', b'N') {
        return -1;
    }

    hio_read32b(f); /* skip size */
    libxmp_read_title(f, t, 64);

    0
}

const GAL4_SAMP_16BIT: u8 = 1 << 2;
const GAL4_SAMP_LOOP: u8 = 1 << 3;
const GAL4_SAMP_LOOP_BIDIR: u8 = 1 << 4;
const GAL4_SAMP_SET_PANNING: u8 = 1 << 5;

/// Translation of `struct local_data`.
#[derive(Default)]
struct LocalData {
    snum: i32,
}

/// Translation of `get_main()`.
fn get_main(m: &mut ModuleData, _size: u32, f: &mut HioHandle<'_>, _data: &mut LocalData) -> i32 {
    let mut buf = [0u8; 64];

    if hio_read(&mut buf, 1, 64, f) < 64 {
        return -1;
    }
    strncpy(&mut m.mod_.name, &buf, 63); /* ensure string terminator */
    m.mod_.name[63] = b'\0';
    libxmp_set_type(m, b"Galaxy Music System 4.0");

    let flags = hio_read8(f);
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

    /* Sanity check */
    if mod_.chn > 32 {
        return -1;
    }

    0
}

/// Translation of `get_ordr()`.
fn get_ordr(m: &mut ModuleData, _size: u32, f: &mut HioHandle<'_>, _data: &mut LocalData) -> i32 {
    let mod_ = &mut m.mod_;

    mod_.len = hio_read8(f) as i32 + 1;
    if hio_error(f) != 0 {
        return -1;
    }

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

    hio_read8(f); /* 00 */
    let i = hio_read8(f) as i32 + 1; /* instrument number */

    /* Sanity check */
    if i > MAX_INSTRUMENTS {
        return -1;
    }

    if i > mod_.ins {
        mod_.ins = i;
    }

    hio_seek(f, 28, SEEK_CUR); /* skip name */

    mod_.smp += hio_read8(f) as i32;

    0
}

/// Translation of `get_patt()`.
fn get_patt(m: &mut ModuleData, _size: u32, f: &mut HioHandle<'_>, _data: &mut LocalData) -> i32 {
    let mod_ = &mut m.mod_;
    let mut dummy = XmpEvent::default();

    let i = hio_read8(f) as i32; /* pattern number */
    let len = hio_read32l(f) as i32;

    /* Sanity check */
    if i >= mod_.pat || len <= 0 || mod_.xxp.as_ref().is_some_and(|x| x[i as usize].is_some()) {
        return -1;
    }

    let rows = hio_read8(f) as i32 + 1;

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

    9
}

/// Translation of `get_inst()`.
fn get_inst(m: &mut ModuleData, _size: u32, f: &mut HioHandle<'_>, data: &mut LocalData) -> i32 {
    let mut buf = [0u8; 30];

    hio_read8(f); /* 00 */
    let i = hio_read8(f) as i32; /* instrument number */

    /* Sanity check */
    if i >= m.mod_.ins || m.mod_.xxi[i as usize].nsm != 0 {
        return -1;
    }
    let iu = i as usize;
    let mod_ = &mut m.mod_;
    let xxi = &mut mod_.xxi[iu];

    hio_read(&mut xxi.name, 1, 28, f);
    xxi.nsm = hio_read8(f) as i32;

    for j in 0..108 {
        xxi.map[j].ins = hio_read8(f);
    }

    hio_seek(f, 11, SEEK_CUR); /* unknown */
    let vwf = hio_read8(f) as i32; /* vibrato waveform */
    let vsw = hio_read8(f) as i32; /* vibrato sweep */
    hio_read8(f); /* unknown */
    hio_read8(f); /* unknown */
    let vde = hio_read8(f) as i32; /* vibrato depth */
    let vra = hio_read16l(f) as i32 / 16; /* vibrato speed */
    hio_read8(f); /* unknown */

    let aei = &mut xxi.aei;
    let pei = &mut xxi.pei;
    let mut val = hio_read8(f); /* PV envelopes flags */
    if lsn(val) & 0x01 != 0 {
        aei.flg |= XMP_ENVELOPE_ON;
    }
    if lsn(val) & 0x02 != 0 {
        aei.flg |= XMP_ENVELOPE_SUS;
    }
    if lsn(val) & 0x04 != 0 {
        aei.flg |= XMP_ENVELOPE_LOOP;
    }
    if msn(val) & 0x01 != 0 {
        pei.flg |= XMP_ENVELOPE_ON;
    }
    if msn(val) & 0x02 != 0 {
        pei.flg |= XMP_ENVELOPE_SUS;
    }
    if msn(val) & 0x04 != 0 {
        pei.flg |= XMP_ENVELOPE_LOOP;
    }

    val = hio_read8(f); /* PV envelopes points */
    aei.npt = lsn(val) + 1;
    pei.npt = msn(val) + 1;

    val = hio_read8(f); /* PV envelopes sustain point */
    aei.sus = lsn(val);
    pei.sus = msn(val);

    val = hio_read8(f); /* PV envelopes loop start */
    aei.lps = lsn(val);
    pei.lps = msn(val);

    // FIXME (upstream): the loop end is set from the loop start's byte, not
    // the one read here.
    hio_read8(f); /* PV envelopes loop end */
    aei.lpe = lsn(val);
    pei.lpe = msn(val);

    if aei.npt <= 0 || aei.npt > 10.min(XMP_MAX_ENV_POINTS as i32) {
        aei.flg &= !XMP_ENVELOPE_ON;
    }

    if pei.npt <= 0 || pei.npt > 10.min(XMP_MAX_ENV_POINTS as i32) {
        pei.flg &= !XMP_ENVELOPE_ON;
    }

    if hio_read(&mut buf, 1, 30, f) < 30 {
        /* volume envelope points */
        return -1;
    }
    for j in 0..aei.npt as usize {
        if j >= 10 {
            break;
        }
        aei.data[j * 2] = (readmem16l(&buf[j * 3..]) / 16) as i16;
        aei.data[j * 2 + 1] = buf[j * 3 + 2] as i16;
    }

    if hio_read(&mut buf, 1, 30, f) < 30 {
        /* pan envelope points */
        return -1;
    }
    for j in 0..pei.npt as usize {
        if j >= 10 {
            break;
        }
        pei.data[j * 2] = (readmem16l(&buf[j * 3..]) / 16) as i16;
        pei.data[j * 2 + 1] = buf[j * 3 + 2] as i16;
    }

    /*fade =*/
    hio_read8(f); /* fadeout - 0x80->0x02 0x310->0x0c */
    hio_read8(f); /* unknown */

    if xxi.nsm == 0 {
        return 0;
    }

    let nsm = xxi.nsm;
    if libxmp_alloc_subinstrument(mod_, i, nsm) < 0 {
        return -1;
    }

    for j in 0..nsm as usize {
        let snum = data.snum as usize;
        /* (the first pass counted the samples) */
        if snum >= m.mod_.xxs.len() {
            return -1;
        }
        let mod_ = &mut m.mod_;
        let sub = &mut mod_.xxi[iu].sub[j];
        let xxs = &mut mod_.xxs[snum];

        hio_read32b(f); /* SAMP */
        hio_read32b(f); /* size */

        hio_read(&mut xxs.name, 1, 28, f);

        let pan = hio_read8(f) as i32;
        sub.vol = hio_read8(f) as i32;
        let flags = hio_read8(f);
        hio_read8(f); /* unknown - 0x80 */

        sub.pan = if flags & GAL4_SAMP_SET_PANNING != 0 {
            (pan * 4).min(255)
        } else {
            XMP_INST_NO_DEFAULT_PAN
        };
        sub.vwf = vwf;
        sub.vde = vde;
        sub.vra = vra;
        sub.vsw = vsw;
        sub.sid = data.snum;

        xxs.len = hio_read32l(f) as i32;
        xxs.lps = hio_read32l(f) as i32;
        xxs.lpe = hio_read32l(f) as i32;

        xxs.flg = 0;
        if flags & GAL4_SAMP_16BIT != 0 {
            xxs.flg |= XMP_SAMPLE_16BIT;
        }
        if flags & GAL4_SAMP_LOOP != 0 {
            xxs.flg |= XMP_SAMPLE_LOOP;
        }
        if flags & GAL4_SAMP_LOOP_BIDIR != 0 {
            xxs.flg |= XMP_SAMPLE_LOOP_BIDIR;
        }
        /* if (flags & 0x80)
        xxs->flg |= ? */

        let srate = hio_read32l(f) as i32;
        let finetune = 0;
        libxmp_c2spd_to_note(srate, &mut sub.xpo, &mut sub.fin);
        sub.fin += finetune;

        hio_read32l(f); /* 0x00000000 */
        hio_read32l(f); /* unknown */

        if xxs.len > 1 {
            let smpctl = m.smpctl;
            if libxmp_load_sample(smpctl, Some(f), 0, &mut m.mod_.xxs[snum], None) < 0 {
                return -1;
            }
        }
        data.snum += 1;
    }

    0
}

/// Translation of `gal4_load()`.
fn gal4_load(m: &mut ModuleData, f: &mut HioHandle<'_>, start: i32) -> i32 {
    let mut data = LocalData::default();

    hio_read32b(f); /* Skip RIFF */
    hio_read32b(f); /* Skip size */
    hio_read32b(f); /* Skip AM   */

    let offset = hio_tell(f) as i32;

    m.mod_.ins = 0;
    m.mod_.smp = 0;

    let mut handle = libxmp_iff_new::<LocalData>();

    m.c4rate = C4_NTSC_RATE;

    /* IFF chunk IDs */
    let mut ret = libxmp_iff_register(&mut handle, b"MAIN", get_main);
    ret |= libxmp_iff_register(&mut handle, b"ORDR", get_ordr);
    ret |= libxmp_iff_register(&mut handle, b"PATT", get_patt_cnt);
    ret |= libxmp_iff_register(&mut handle, b"INST", get_inst_cnt);

    if ret != 0 {
        return -1;
    }

    libxmp_iff_set_quirk(&mut handle, IFF_LITTLE_ENDIAN);
    libxmp_iff_set_quirk(&mut handle, IFF_CHUNK_TRUNC4);

    /* Load IFF chunks */
    if libxmp_iff_load(&handle, m, f, &mut data) < 0 {
        libxmp_iff_release(handle);
        return -1;
    }

    libxmp_iff_release(handle);

    m.mod_.trk = m.mod_.pat * m.mod_.chn;

    if libxmp_init_instrument(m) < 0 {
        return -1;
    }

    if libxmp_init_pattern(&mut m.mod_) < 0 {
        return -1;
    }

    hio_seek(f, start as i64 + offset as i64, SEEK_SET);
    data.snum = 0;

    let mut handle = libxmp_iff_new::<LocalData>();

    /* IFF chunk IDs */
    let mut ret = libxmp_iff_register(&mut handle, b"PATT", get_patt);
    ret |= libxmp_iff_register(&mut handle, b"INST", get_inst);

    if ret != 0 {
        return -1;
    }

    libxmp_iff_set_quirk(&mut handle, IFF_LITTLE_ENDIAN);
    libxmp_iff_set_quirk(&mut handle, IFF_CHUNK_TRUNC4);

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
        mod_.xxc[i].pan = 0x80;
    }

    /* TODO: JJ2 1.20 shareware does not support fine slides from
     * converted XMs. Should be tested with converted S3Ms, though. */
    m.quirk |= QUIRK_FINEFX;
    m.read_event_type = READ_EVENT_FT2;

    0
}
