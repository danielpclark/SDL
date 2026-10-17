// Rust translation of src/loaders/pt3_load.c from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2026 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Protracker 3 (IFFMODL) loader.

use super::super::common::*;
use super::super::hio::*;
use super::super::*;
use super::common::*;
use super::iff::*;
use super::mod_load::{ModHeader, ModInstrument};
use super::sample::libxmp_load_sample;
use super::*;

const MAGIC_FORM: u32 = magic4(b'F', b'O', b'R', b'M');
const MAGIC_MODL: u32 = magic4(b'M', b'O', b'D', b'L');
const MAGIC_VERS: u32 = magic4(b'V', b'E', b'R', b'S');
const MAGIC_INFO: u32 = magic4(b'I', b'N', b'F', b'O');

pub(crate) const LIBXMP_LOADER_PT3: FormatLoader = FormatLoader {
    name: "Protracker 3",
    test: pt3_test,
    loader: pt3_load,
};

/// Translation of `pt3_test()`.
fn pt3_test(f: &mut HioHandle<'_>, t: Option<&mut [u8]>, _start: i32) -> i32 {
    if hio_read32b(f) != MAGIC_FORM {
        return -1;
    }

    hio_read32b(f); /* skip size */

    if hio_read32b(f) != MAGIC_MODL {
        return -1;
    }

    if hio_read32b(f) != MAGIC_VERS {
        return -1;
    }

    hio_read32b(f); /* skip size */

    hio_seek(f, 10, SEEK_CUR);

    if hio_read32b(f) == MAGIC_INFO {
        hio_read32b(f); /* skip size */
        libxmp_read_title(f, t, 32);
    } else {
        libxmp_read_title(f, t, 0);
    }

    0
}

const PT3_FLAG_CIA: i32 = 0x0001; /* VBlank if not set */
const PT3_FLAG_FILTER: i32 = 0x0002; /* Filter status */
const PT3_FLAG_SONG: i32 = 0x0004; /* Modules have this bit unset */
const PT3_FLAG_IRQ: i32 = 0x0008; /* Soft IRQ */
const PT3_FLAG_VARPAT: i32 = 0x0010; /* Variable pattern length */
const PT3_FLAG_8VOICE: i32 = 0x0020; /* 4 voices if not set */
const PT3_FLAG_16BIT: i32 = 0x0040; /* 8 bit samples if not set */
const PT3_FLAG_RAWPAT: i32 = 0x0080; /* Packed patterns if not set */

/// Translation of `struct local_data`.
#[derive(Default)]
struct LocalData {
    has_ptdt: bool,
}

/// Translation of `get_info()`.
fn get_info(m: &mut ModuleData, _size: u32, f: &mut HioHandle<'_>, data: &mut LocalData) -> i32 {
    let mod_ = &mut m.mod_;
    /* int flags; */
    /* int day, month, year, hour, min, sec;
    int dhour, dmin, dsec; */

    /* Sanity check */
    if data.has_ptdt {
        return -1;
    }

    hio_read(&mut mod_.name, 1, 32, f);
    mod_.ins = hio_read16b(f) as i32;
    mod_.len = hio_read16b(f) as i32;
    mod_.pat = hio_read16b(f) as i32;
    mod_.gvl = hio_read16b(f) as i32;
    mod_.bpm = hio_read16b(f) as i32; /* Not clamped by Protracker 3.6 */
    /*flags =*/
    hio_read16b(f);
    /*day   =*/
    hio_read16b(f);
    /*month =*/
    hio_read16b(f);
    /*year  =*/
    hio_read16b(f);
    /*hour  =*/
    hio_read16b(f);
    /*min   =*/
    hio_read16b(f);
    /*sec   =*/
    hio_read16b(f);
    /*dhour =*/
    hio_read16b(f);
    /*dmin  =*/
    hio_read16b(f);
    /*dsec  =*/
    hio_read16b(f);

    /* Sanity check */
    if mod_.ins > 255 || mod_.len > 256 || mod_.pat > 255 {
        return -1;
    }

    0
}

/// Translation of `get_cmnt()`.
fn get_cmnt(_m: &mut ModuleData, _size: u32, _f: &mut HioHandle<'_>, _data: &mut LocalData) -> i32 {
    0
}

/// Translation of `get_ptdt()`.
fn get_ptdt(m: &mut ModuleData, size: u32, f: &mut HioHandle<'_>, data: &mut LocalData) -> i32 {
    /* Sanity check */
    if data.has_ptdt {
        return -1;
    }
    data.has_ptdt = true;

    let pos = hio_tell(f);
    /* (its result isn't checked) */
    ptdt_load(m, f, 0);

    /* Check total loaded length.
     * NOTE: some Protracker 3.6 files lie about the length of the PTDT
     * chunk due to what appears to be a bug--files with samples of length
     * 64k or greater will be undercounted in the chunk length by 65536. */
    let pos2 = hio_tell(f);
    if pos < pos2 {
        let mut adjust = 0;

        for i in 0..m.mod_.smp as usize {
            if m.mod_.xxs.get(i).is_some_and(|x| x.len >= 65536) {
                adjust += 65536;
            }
        }
        if pos2 - pos - adjust < 0 || pos2 - pos - adjust != size as i64 {
            /* (upstream warns about the failed length check) */
        }
    }
    0
}

/// Translation of `pt3_load()`.
fn pt3_load(m: &mut ModuleData, f: &mut HioHandle<'_>, _start: i32) -> i32 {
    let mut data = LocalData::default();
    let mut buf = [0u8; 20];

    hio_read32b(f); /* FORM */
    hio_read32b(f); /* size */
    hio_read32b(f); /* MODL */
    hio_read32b(f); /* VERS */
    hio_read32b(f); /* VERS size */

    if hio_read(&mut buf, 1, 10, f) < 10 {
        return -1;
    }
    let mut type_ = printf_s_left(&buf[4..10], 6, 6);
    type_.extend_from_slice(b" IFFMODL");
    libxmp_set_type(m, &type_);

    let mut handle = libxmp_iff_new::<LocalData>();

    /* IFF chunk IDs */
    let mut ret = libxmp_iff_register(&mut handle, b"INFO", get_info);
    ret |= libxmp_iff_register(&mut handle, b"CMNT", get_cmnt);
    ret |= libxmp_iff_register(&mut handle, b"PTDT", get_ptdt);

    if ret != 0 {
        return -1;
    }

    libxmp_iff_set_quirk(&mut handle, IFF_FULL_CHUNK_SIZE);

    /* Load IFF chunks */
    if libxmp_iff_load(&handle, m, f, &mut data) < 0 {
        libxmp_iff_release(handle);
        return -1;
    }

    libxmp_iff_release(handle);

    /* Sanity check */
    if m.mod_.smp <= 0 {
        return -1;
    }

    0
}

/// Translation of `ptdt_load()`.
fn ptdt_load(m: &mut ModuleData, f: &mut HioHandle<'_>, _start: i32) -> i32 {
    let mut mod_event = [0u8; 4];
    let mut mh = ModHeader {
        name: [0; 20],
        ins: [ModInstrument::default(); 31],
        len: 0,
        restart: 0,
        order: [0; 128],
        magic: [0; 4],
    };

    hio_read(&mut mh.name, 20, 1, f);
    for i in 0..31 {
        let ins = &mut mh.ins[i];
        hio_read(&mut ins.name, 22, 1, f);
        ins.size = hio_read16b(f);
        ins.finetune = hio_read8(f) as i8;
        ins.volume = hio_read8(f) as i8;
        ins.loop_start = hio_read16b(f);
        ins.loop_size = hio_read16b(f);
    }
    mh.len = hio_read8(f);
    mh.restart = hio_read8(f);
    if hio_read(&mut mh.order, 128, 1, f) < 1 {
        return -1;
    }
    hio_read(&mut mh.magic, 4, 1, f);

    let mod_ = &mut m.mod_;
    mod_.ins = 31;
    mod_.smp = mod_.ins;
    mod_.chn = 4;
    mod_.len = mh.len as i32;
    mod_.rst = mh.restart as i32;
    mod_.xxo[..128].copy_from_slice(&mh.order);

    for i in 0..128 {
        if mod_.xxo[i] as i32 > mod_.pat {
            mod_.pat = mod_.xxo[i] as i32;
        }
    }

    mod_.pat += 1;
    mod_.trk = mod_.chn * mod_.pat;

    if libxmp_init_instrument(m) < 0 {
        return -1;
    }

    let mod_ = &mut m.mod_;
    for i in 0..mod_.ins {
        let iu = i as usize;
        if libxmp_alloc_subinstrument(mod_, i, 1) < 0 {
            return -1;
        }

        let ins = &mh.ins[iu];
        let xxs = &mut mod_.xxs[iu];
        xxs.len = 2 * ins.size as i32;
        xxs.lps = 2 * ins.loop_start as i32;
        xxs.lpe = xxs.lps + 2 * ins.loop_size as i32;
        xxs.flg = if ins.loop_size > 1 {
            XMP_SAMPLE_LOOP
        } else {
            0
        };
        let len = xxs.len;

        let xxi = &mut mod_.xxi[iu];
        if len > 0 {
            xxi.nsm = 1;
        }

        let sub = &mut xxi.sub[0];
        sub.fin = ((ins.finetune as u8) << 4) as i8 as i32;
        sub.vol = ins.volume as i32;
        sub.pan = XMP_INST_NO_DEFAULT_PAN;
        sub.sid = i;
        xxi.rls = 0xfff;

        libxmp_instrument_name(mod_, i, &ins.name, 22);
    }

    if libxmp_init_pattern(mod_) < 0 {
        return -1;
    }

    /* Load and convert patterns */

    for i in 0..mod_.pat {
        if libxmp_alloc_pattern_tracks(mod_, i, 64) < 0 {
            return -1;
        }

        for j in 0..(64 * 4) {
            let event = mod_.event_mut(i, j % 4, j / 4);
            if hio_read(&mut mod_event, 1, 4, f) < 4 {
                return -1;
            }
            libxmp_decode_protracker_event(event, &mod_event);
        }
    }

    m.period_type = PERIOD_MODRNG;

    /* Load samples */

    for i in 0..m.mod_.smp as usize {
        if m.mod_.xxs[i].len == 0 {
            continue;
        }

        let smpctl = m.smpctl;
        if libxmp_load_sample(smpctl, Some(f), 0, &mut m.mod_.xxs[i], None) < 0 {
            return -1;
        }
    }

    0
}
