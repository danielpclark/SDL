// Rust translation of src/loaders/mdl_load.c from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2026 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

/* Note: envelope switching (effect 9) and sample status change (effect 8)
 * not supported.
 */

//! The Digitrakker (MDL) loader.

use super::super::common::*;
use super::super::dataio::*;
use super::super::effects::*;
use super::super::hio::*;
use super::super::*;
use super::common::*;
use super::iff::*;
use super::sample::libxmp_load_sample;
use super::*;

const MAGIC_DMDL: u32 = magic4(b'D', b'M', b'D', b'L');

pub(crate) const LIBXMP_LOADER_MDL: FormatLoader = FormatLoader {
    name: "Digitrakker",
    test: mdl_test,
    loader: mdl_load,
};

/// Translation of `mdl_test()`.
fn mdl_test(f: &mut HioHandle<'_>, t: Option<&mut [u8]>, _start: i32) -> i32 {
    if hio_read32b(f) != MAGIC_DMDL {
        return -1;
    }

    hio_read8(f); /* version */
    let id = hio_read16b(f);

    if id == 0x494e {
        /* IN */
        hio_read32b(f);
        libxmp_read_title(f, t, 32);
    } else {
        libxmp_read_title(f, t, 0);
    }

    0
}

const MDL_NOTE_FOLLOWS: i32 = 0x04;
const MDL_INSTRUMENT_FOLLOWS: i32 = 0x08;
const MDL_VOLUME_FOLLOWS: i32 = 0x10;
const MDL_EFFECT_FOLLOWS: i32 = 0x20;
const MDL_PARAMETER1_FOLLOWS: i32 = 0x40;
const MDL_PARAMETER2_FOLLOWS: i32 = 0x80;

/// Translation of `struct mdl_envelope`.
#[derive(Clone, Copy, Default)]
struct MdlEnvelope {
    num: u8,
    data: [u8; 30],
    sus: u8,
    loop_: u8,
}

/// Translation of `struct local_data`.
#[derive(Default)]
struct LocalData {
    i_index: Vec<i32>,
    s_index: Vec<i32>,
    v_index: Vec<i32>, /* volume envelope */
    p_index: Vec<i32>, /* pan envelope */
    f_index: Vec<i32>, /* pitch envelope */
    packinfo: Option<Vec<i32>>,
    has_in: bool,
    has_pa: bool,
    has_tr: bool,
    has_ii: bool,
    has_is: bool,
    has_sa: bool,
    v_envnum: i32,
    p_envnum: i32,
    f_envnum: i32,
    v_env: Option<Vec<MdlEnvelope>>,
    p_env: Option<Vec<MdlEnvelope>>,
    f_env: Option<Vec<MdlEnvelope>>,
}

/// Translation of `fix_env()`.
fn fix_env(i: usize, ei: &mut XmpEnvelope, env: Option<&[MdlEnvelope]>, idx: &[i32], envnum: i32) {
    if idx[i] >= 0 {
        ei.flg = XMP_ENVELOPE_ON;
        ei.npt = 15;

        let env = env.unwrap_or(&[]);
        for j in 0..envnum as usize {
            if idx[i] == env[j].num as i32 {
                ei.flg |= if env[j].sus & 0x10 != 0 {
                    XMP_ENVELOPE_SUS
                } else {
                    0
                };
                ei.flg |= if env[j].sus & 0x20 != 0 {
                    XMP_ENVELOPE_LOOP
                } else {
                    0
                };
                ei.sus = (env[j].sus & 0x0f) as i32;
                ei.lps = (env[j].loop_ & 0x0f) as i32;
                // Note (upstream): C doesn't shift the loop end's nibble
                // down (making it 0, 16, ... 240).
                ei.lpe = ((env[j].loop_ & 0xf0) >> 4) as i32;

                let mut lastx: i32 = -1;

                let mut k = 0;
                while k < ei.npt as usize {
                    let x = env[j].data[k * 2] as i32;

                    if x == 0 {
                        break;
                    }
                    ei.data[k * 2] = (lastx + x) as i16;
                    ei.data[k * 2 + 1] = env[j].data[k * 2 + 1] as i16;

                    lastx = ei.data[k * 2] as i32;
                    k += 1;
                }

                ei.npt = k as i32;
                break;
            }
        }
    }
}

/* Effects 1-6 (note effects) can only be entered in the first effect
 * column, G-L (volume-effects) only in the second column.
 */

/// Translation of `xlat_fx_common()`.
fn xlat_fx_common(t: &mut u8, p: &mut u8) {
    match *t {
        0x07 => {
            /* 7 - Set BPM */
            *t = FX_S3M_BPM;
        }
        0x08 /* 8 - Set pan */ | 0x09 /* 9 - Set envelope -- not supported */ | 0x0a /* A - Not used */ => {
            *t = 0x00;
            *p = 0x00;
        }
        0x0b /* B - Position jump */ | 0x0c /* C - Set volume */ | 0x0d /* D - Pattern break */ => {
            /* Like protracker */
        }
        0x0e => {
            /* E - Extended */
            match msn(*p) {
                0x0 /* E0 - not used */ | 0x3 /* E3 - not used */ | 0x8 /* Set sample status -- unsupported */ => {
                    *t = 0x00;
                    *p = 0x00;
                }
                0x1 => {
                    /* Pan slide left */
                    *t = FX_PANSLIDE;
                    *p <<= 4;
                }
                0x2 => {
                    /* Pan slide right */
                    *t = FX_PANSLIDE;
                    *p &= 0x0f;
                }
                _ => {}
            }
        }
        0x0f => {
            *t = FX_S3M_SPEED;
        }
        _ => {}
    }
}

/// Translation of `xlat_fx1()`.
fn xlat_fx1(t: &mut u8, p: &mut u8) {
    match *t {
        0x00 => {
            /* - - No effect */
            *p = 0;
        }
        0x05 => {
            /* 5 - Arpeggio */
            *t = FX_ARPEGGIO;
        }
        0x06 => {
            /* 6 - Not used */
            *t = 0x00;
            *p = 0x00;
        }
        _ => {}
    }

    xlat_fx_common(t, p);
}

/// Translation of `xlat_fx2()`.
fn xlat_fx2(t: &mut u8, p: &mut u8) {
    match *t {
        0x00 => {
            /* - - No effect */
            *p = 0;
        }
        0x01 => {
            /* G - Volume slide up */
            *t = FX_VOLSLIDE_UP;
        }
        0x02 => {
            /* H - Volume slide down */
            *t = FX_VOLSLIDE_DN;
        }
        0x03 => {
            /* I - Multi-retrig */
            *t = FX_MULTI_RETRIG;
        }
        0x04 => {
            /* J - Tremolo */
            *t = FX_TREMOLO;
        }
        0x05 => {
            /* K - Tremor */
            *t = FX_TREMOR;
        }
        0x06 => {
            /* L - Not used */
            *t = 0x00;
            *p = 0x00;
        }
        _ => {}
    }

    xlat_fx_common(t, p);
}

/// Translation of `struct bits`.
#[derive(Default)]
struct Bits {
    b: u32,
    n: u32,
}

/// Translation of `get_bits()`: `buf` is the packed data from its read
/// position (followed by the four zero bytes upstream adds).
fn get_bits(i: u32, buf: &[u8], pos: &mut usize, len: &mut i32, bits: &mut Bits) -> u32 {
    if i == 0 {
        bits.b = readmem32l(&buf[*pos..]);
        *pos += 4;
        *len -= 4;
        bits.n = 32;
    }

    let x = bits.b & ((1u32 << i) - 1); /* get i bits */
    bits.b >>= i;
    bits.n -= i;
    if bits.n <= 24 {
        if *len <= 0 {
            /* FIXME: last few bits can't be consumed */
            return x;
        }
        bits.b |= readmem32l(&buf[*pos..]) << bits.n;
        *pos += 1;
        bits.n += 8;
        *len -= 1;
    }

    x
}

/* From the Digitrakker docs:
 *
 * The description of the sample-packmethode (1) [8bit packing]:...
 * ----------------------------------------------------------------
 *
 * The method is based on the Huffman algorithm. It's easy and very fast
 * and effective on samples. The packed sample is a bit stream:
 *
 *	     Byte 0    Byte 1    Byte 2    Byte 3
 *	Bit 76543210  fedcba98  nmlkjihg  ....rqpo
 *
 * A packed byte is stored in the following form:
 *
 *	xxxx10..0s => byte = <xxxx> + (number of <0> bits between
 *		s and 1) * 16 - 8;
 *	if s==1 then byte = byte xor 255
 *
 * If there are no <0> bits between the first bit (sign) and the <1> bit,
 * you have the following form:
 *
 *	xxx1s => byte = <xxx>; if s=1 then byte = byte xor 255
 */

/// Translation of `unpack_sample8()`.
fn unpack_sample8(t: &mut [u8], f: &[u8], mut len: i32, l: i32) -> i32 {
    let mut bits = Bits::default();
    let mut pos = 0usize;

    get_bits(0, f, &mut pos, &mut len, &mut bits);

    let mut b: u8;
    let mut d: u8 = 0;
    for i in 0..l.max(0) as usize {
        /* Sanity check */
        if len < 0 {
            return -1;
        }

        let s = get_bits(1, f, &mut pos, &mut len, &mut bits);
        if get_bits(1, f, &mut pos, &mut len, &mut bits) != 0 {
            b = get_bits(3, f, &mut pos, &mut len, &mut bits) as u8;
        } else {
            b = 8;
            while len >= 0 && get_bits(1, f, &mut pos, &mut len, &mut bits) == 0 {
                /* Sanity check */
                if b >= 240 {
                    return -1;
                }
                b += 16;
            }
            b = b.wrapping_add(get_bits(4, f, &mut pos, &mut len, &mut bits) as u8);
        }

        if s != 0 {
            b ^= 0xff;
        }

        d = d.wrapping_add(b);
        t[i] = d;
    }

    0
}

/*
 * The description of the sample-packmethode (2) [16bit packing]:...
 * ----------------------------------------------------------------
 *
 * It works as methode (1) but it only crunches every 2nd byte (the high-
 * bytes of 16 bit samples). So when you depack 16 bit samples, you have to
 * read 8 bits from the data-stream first. They present the lowbyte of the
 * sample-word. Then depack the highbyte in the descripted way (methode [1]).
 * Only the highbytes are delta-values. So take the lowbytes as they are.
 * Go on this way for the whole sample!
 */

/// Translation of `unpack_sample16()`.
fn unpack_sample16(t: &mut [u8], f: &[u8], mut len: i32, l: i32) -> i32 {
    let mut bits = Bits::default();
    let mut pos = 0usize;

    get_bits(0, f, &mut pos, &mut len, &mut bits);

    let mut b: u8;
    let mut d: u8 = 0;
    for i in 0..l.max(0) as usize {
        /* Sanity check */
        if len < 0 {
            return -1;
        }

        let lo = get_bits(8, f, &mut pos, &mut len, &mut bits);
        let s = get_bits(1, f, &mut pos, &mut len, &mut bits);
        if get_bits(1, f, &mut pos, &mut len, &mut bits) != 0 {
            b = get_bits(3, f, &mut pos, &mut len, &mut bits) as u8;
        } else {
            b = 8;
            while len >= 0 && get_bits(1, f, &mut pos, &mut len, &mut bits) == 0 {
                /* Sanity check */
                if b >= 240 {
                    return -1;
                }
                b += 16;
            }
            b = b.wrapping_add(get_bits(4, f, &mut pos, &mut len, &mut bits) as u8);
        }

        if s != 0 {
            b ^= 0xff;
        }
        d = d.wrapping_add(b);

        t[i * 2] = lo as u8;
        t[i * 2 + 1] = d;
    }

    0
}

/*
 * IFF chunk handlers
 */

/// Translation of `get_chunk_in()`.
fn get_chunk_in(
    m: &mut ModuleData,
    _size: u32,
    f: &mut HioHandle<'_>,
    data: &mut LocalData,
) -> i32 {
    let mod_ = &mut m.mod_;

    /* Sanity check */
    if data.has_in {
        return -1;
    }
    data.has_in = true;

    hio_read(&mut mod_.name, 1, 32, f);
    mod_.name[32] = b'\0';
    hio_seek(f, 20, SEEK_CUR);

    mod_.len = hio_read16l(f) as i32;
    mod_.rst = hio_read16l(f) as i32;
    hio_read8(f); /* gvol */
    mod_.spd = hio_read8(f) as i32;
    mod_.bpm = hio_read8(f) as i32;

    /* Sanity check */
    if mod_.len > 256 || mod_.rst > 255 {
        return -1;
    }

    let mut i = 0;
    while i < 32 {
        let chinfo = hio_read8(f);
        if chinfo & 0x80 != 0 {
            break;
        }
        mod_.xxc[i].pan = (chinfo as i32) << 1;
        i += 1;
    }
    mod_.chn = i as i32;
    hio_seek(f, 32 - i as i64 - 1, SEEK_CUR);

    let len = mod_.len as usize;
    if hio_read(&mut mod_.xxo, 1, len, f) != len {
        return -1;
    }

    0
}

/// Translation of `get_chunk_pa()`.
fn get_chunk_pa(
    m: &mut ModuleData,
    _size: u32,
    f: &mut HioHandle<'_>,
    data: &mut LocalData,
) -> i32 {
    let mod_ = &mut m.mod_;

    /* Sanity check */
    if data.has_pa || !data.has_in {
        return -1;
    }
    data.has_pa = true;

    mod_.pat = hio_read8(f) as i32;

    let Ok(xxp) = try_vec(mod_.pat as usize, None) else {
        return -1;
    };
    mod_.xxp = Some(xxp);

    for i in 0..mod_.pat {
        if libxmp_alloc_pattern(mod_, i) < 0 {
            return -1;
        }

        let chn = hio_read8(f) as i32;
        mod_.pattern_mut(i).rows = hio_read8(f) as i32 + 1;

        hio_seek(f, 16, SEEK_CUR); /* Skip pattern name */
        for j in 0..chn {
            let x = hio_read16l(f) as i32;

            if j < mod_.chn {
                mod_.pattern_mut(i).index[j as usize] = x;
            }
        }
    }

    0
}

/// Translation of `get_chunk_p0()`.
fn get_chunk_p0(
    m: &mut ModuleData,
    _size: u32,
    f: &mut HioHandle<'_>,
    data: &mut LocalData,
) -> i32 {
    let mod_ = &mut m.mod_;

    /* Sanity check */
    if data.has_pa || !data.has_in {
        return -1;
    }
    data.has_pa = true;

    mod_.pat = hio_read8(f) as i32;

    let Ok(xxp) = try_vec(mod_.pat as usize, None) else {
        return -1;
    };
    mod_.xxp = Some(xxp);

    for i in 0..mod_.pat {
        if libxmp_alloc_pattern(mod_, i) < 0 {
            return -1;
        }
        mod_.pattern_mut(i).rows = 64;

        for j in 0..32 {
            let x = hio_read16l(f);

            if j < mod_.chn {
                mod_.pattern_mut(i).index[j as usize] = x as i32;
            }
        }
    }

    0
}

/// Translation of `get_chunk_tr()`.
fn get_chunk_tr(
    m: &mut ModuleData,
    _size: u32,
    f: &mut HioHandle<'_>,
    data: &mut LocalData,
) -> i32 {
    let mod_ = &mut m.mod_;

    /* Sanity check */
    if data.has_tr || !data.has_pa {
        return -1;
    }
    data.has_tr = true;

    mod_.trk = hio_read16l(f) as i32 + 1;

    /* Sanity check */
    let mut max_trk = 0;
    for i in 0..mod_.pat {
        for j in 0..mod_.chn as usize {
            if max_trk < mod_.pattern(i).index[j] {
                max_trk = mod_.pattern(i).index[j];
            }
        }
    }
    if max_trk >= mod_.trk {
        return -1;
    }

    let Ok(xxt) = try_vec(mod_.trk as usize, None) else {
        return -1;
    };
    mod_.xxt = Some(xxt);

    let mut track;

    /* Empty track 0 is not stored in the file */
    if libxmp_alloc_track(mod_, 0, 256) < 0 {
        return -1;
    }

    for i in 1..mod_.trk {
        /* Length of the track in bytes */
        let mut len = hio_read16l(f) as i32;

        track = [XmpEvent::default(); 256];

        let mut row: i32 = 0;
        while len != 0 {
            /* Sanity check */
            if row > 255 {
                return -1;
            }

            let r = row as usize;

            let j = hio_read8(f) as i32;

            len -= 1;
            match j & 0x03 {
                0 => {
                    row += j >> 2;
                }
                1 => {
                    /* Sanity check */
                    if row < 1 || row + (j >> 2) > 255 {
                        return -1;
                    }

                    let mut k = 0;
                    while k <= (j >> 2) {
                        track[r + k as usize] = track[r - 1];
                        k += 1;
                    }
                    row += k - 1;
                }
                2 => {
                    /* Sanity check */
                    if (j >> 2) == row {
                        return -1;
                    }
                    track[r] = track[(j >> 2) as usize];
                }
                _ => {
                    let ev = &mut track[r];
                    if j & MDL_NOTE_FOLLOWS != 0 {
                        let b = hio_read8(f);
                        len -= 1;
                        ev.note = if b == 0xff {
                            XMP_KEY_OFF
                        } else {
                            b.wrapping_add(12)
                        };
                    }
                    if j & MDL_INSTRUMENT_FOLLOWS != 0 {
                        len -= 1;
                        ev.ins = hio_read8(f);
                    }
                    if j & MDL_VOLUME_FOLLOWS != 0 {
                        len -= 1;
                        ev.vol = hio_read8(f);
                    }
                    if j & MDL_EFFECT_FOLLOWS != 0 {
                        len -= 1;
                        let k = hio_read8(f);
                        ev.fxt = lsn(k) as u8;
                        ev.f2t = msn(k) as u8;
                    }
                    if j & MDL_PARAMETER1_FOLLOWS != 0 {
                        len -= 1;
                        ev.fxp = hio_read8(f);
                    }
                    if j & MDL_PARAMETER2_FOLLOWS != 0 {
                        len -= 1;
                        ev.f2p = hio_read8(f);
                    }
                }
            }

            row += 1;
        }

        if row <= 64 {
            row = 64;
        } else if row <= 128 {
            row = 128;
        } else {
            row = 256;
        }

        if libxmp_alloc_track(mod_, i, row) < 0 {
            return -1;
        }

        let t = mod_.track_mut(i);
        t.event.copy_from_slice(&track[..row as usize]);

        t.rows = row;

        /* Translate effects */
        for ev in t.event.iter_mut() {
            xlat_fx1(&mut ev.fxt, &mut ev.fxp);
            xlat_fx2(&mut ev.f2t, &mut ev.f2p);
        }
    }

    0
}

/// Translation of `get_chunk_ii()`.
fn get_chunk_ii(
    m: &mut ModuleData,
    _size: u32,
    f: &mut HioHandle<'_>,
    data: &mut LocalData,
) -> i32 {
    let mod_ = &mut m.mod_;
    let mut buf = [0u8; 40];

    /* Sanity check */
    if data.has_ii {
        return -1;
    }
    data.has_ii = true;

    mod_.ins = hio_read8(f) as i32;

    let Ok(xxi) = try_vec(mod_.ins as usize, XmpInstrument::default()) else {
        return -1;
    };
    mod_.xxi = xxi;

    for i in 0..mod_.ins {
        let iu = i as usize;

        data.i_index[iu] = hio_read8(f) as i32;
        mod_.xxi[iu].nsm = hio_read8(f) as i32;
        if hio_read(&mut buf, 1, 32, f) < 32 {
            return -1;
        }
        buf[32] = 0;
        libxmp_instrument_name(mod_, i, &buf, 32);

        let nsm = mod_.xxi[iu].nsm;
        if libxmp_alloc_subinstrument(mod_, i, nsm) < 0 {
            return -1;
        }

        let xxi = &mut mod_.xxi[iu];
        for j in 0..XMP_MAX_KEYS {
            xxi.map[j].ins = 0xff;
        }

        let mut last_map = 0;
        for j in 0..nsm as usize {
            let sub = &mut xxi.sub[j];

            sub.sid = hio_read8(f) as i32;
            let map = hio_read8(f) as i32 + 12;
            sub.vol = hio_read8(f) as i32;
            for k in last_map..=map {
                if k < XMP_MAX_KEYS as i32 {
                    xxi.map[k as usize].ins = j as u8;
                }
            }
            last_map = map + 1;

            let sub = &mut xxi.sub[j];
            let mut x = hio_read8(f) as i32; /* Volume envelope */
            if j == 0 {
                data.v_index[iu] = if x & 0x80 != 0 { x & 0x3f } else { -1 };
            }
            if !x & 0x40 != 0 {
                sub.vol = 0xff;
            }

            sub.pan = (hio_read8(f) as i32) << 1;

            x = hio_read8(f) as i32; /* Pan envelope */
            if j == 0 {
                data.p_index[iu] = if x & 0x80 != 0 { x & 0x3f } else { -1 };
            }
            if !x & 0x40 != 0 {
                sub.pan = XMP_INST_NO_DEFAULT_PAN;
            }

            x = hio_read16l(f) as i32;
            if j == 0 {
                xxi.rls = x;
            }

            let sub = &mut xxi.sub[j];
            sub.vra = hio_read8(f) as i32; /* vibrato rate */
            sub.vde = (hio_read8(f) as i32) << 1; /* vibrato depth */
            sub.vsw = hio_read8(f) as i32; /* vibrato sweep */
            sub.vwf = hio_read8(f) as i32; /* vibrato waveform */
            hio_read8(f); /* Reserved */

            x = hio_read8(f) as i32; /* Pitch envelope */
            if j == 0 {
                data.f_index[iu] = if x & 0x80 != 0 { x & 0x3f } else { -1 };
            }
        }
    }

    0
}

/// Translation of `get_chunk_is()`.
fn get_chunk_is(
    m: &mut ModuleData,
    _size: u32,
    f: &mut HioHandle<'_>,
    data: &mut LocalData,
) -> i32 {
    let mut buf = [0u8; 64];

    /* Sanity check */
    if data.has_is {
        return -1;
    }
    data.has_is = true;

    m.mod_.smp = hio_read8(f) as i32;
    let n = m.mod_.smp as usize;
    let Ok(xxs) = try_vec(n, XmpSample::default()) else {
        return -1;
    };
    m.mod_.xxs = xxs;
    let Ok(xtra) = try_vec(n, ExtraSampleData::default()) else {
        return -1;
    };
    m.xtra = xtra;

    let Ok(packinfo) = try_vec(n, 0) else {
        return -1;
    };
    let packinfo = data.packinfo.insert(packinfo);

    for i in 0..n {
        let xxs = &mut m.mod_.xxs[i];

        data.s_index[i] = hio_read8(f) as i32; /* Sample number */
        if hio_read(&mut buf, 1, 32, f) < 32 {
            return -1;
        }
        buf[32] = 0;
        libxmp_copy_adjust(&mut xxs.name, &buf, 31);

        hio_seek(f, 8, SEEK_CUR); /* Sample filename */

        let c5spd = hio_read32l(f) as i32;

        xxs.len = hio_read32l(f) as i32;
        xxs.lps = hio_read32l(f) as i32;
        xxs.lpe = hio_read32l(f) as i32;

        /* Sanity check */
        if xxs.len < 0 || xxs.lps < 0 || xxs.lps > xxs.len || xxs.lpe > (xxs.len - xxs.lps) {
            return -1;
        }

        xxs.flg = if xxs.lpe > 0 { XMP_SAMPLE_LOOP } else { 0 };
        xxs.lpe += xxs.lps;

        m.xtra[i].c5spd = c5spd as f64;

        hio_read8(f); /* Volume in DMDL 0.0 */
        let x = hio_read8(f);
        if x & 0x01 != 0 {
            xxs.flg |= XMP_SAMPLE_16BIT;
            xxs.len >>= 1;
            xxs.lps >>= 1;
            xxs.lpe >>= 1;
        }
        xxs.flg |= if x & 0x02 != 0 {
            XMP_SAMPLE_LOOP_BIDIR
        } else {
            0
        };
        packinfo[i] = ((x & 0x0c) >> 2) as i32;
    }

    0
}

/// Translation of `get_chunk_i0()`.
fn get_chunk_i0(
    m: &mut ModuleData,
    _size: u32,
    f: &mut HioHandle<'_>,
    data: &mut LocalData,
) -> i32 {
    let mut buf = [0u8; 64];

    /* Sanity check */
    if data.has_ii || data.has_is {
        return -1;
    }
    data.has_ii = true;
    data.has_is = true;

    m.mod_.smp = hio_read8(f) as i32;
    m.mod_.ins = m.mod_.smp;

    if libxmp_init_instrument(m) < 0 {
        return -1;
    }

    let Ok(packinfo) = try_vec(m.mod_.smp as usize, 0) else {
        return -1;
    };
    let packinfo = data.packinfo.insert(packinfo);

    for i in 0..m.mod_.ins {
        let iu = i as usize;
        let mod_ = &mut m.mod_;

        mod_.xxi[iu].nsm = 1;
        if libxmp_alloc_subinstrument(mod_, i, 1) < 0 {
            return -1;
        }

        let v = hio_read8(f) as i32;
        data.s_index[iu] = v;
        data.i_index[iu] = v;
        mod_.xxi[iu].sub[0].sid = v;

        if hio_read(&mut buf, 1, 32, f) < 32 {
            return -1;
        }
        buf[32] = 0;
        hio_seek(f, 8, SEEK_CUR); /* Sample filename */
        libxmp_instrument_name(mod_, i, &buf, 32);

        let c5spd = hio_read16l(f) as i32;

        let xxs = &mut mod_.xxs[iu];
        xxs.len = hio_read32l(f) as i32;
        xxs.lps = hio_read32l(f) as i32;
        xxs.lpe = hio_read32l(f) as i32;

        /* Sanity check */
        if xxs.len < 0 || xxs.lps < 0 || xxs.lps > xxs.len || xxs.lpe > (xxs.len - xxs.lps) {
            return -1;
        }

        xxs.flg = if xxs.lpe > 0 { XMP_SAMPLE_LOOP } else { 0 };
        xxs.lpe += xxs.lps;

        let sub = &mut mod_.xxi[iu].sub[0];
        sub.vol = hio_read8(f) as i32; /* Volume */
        sub.pan = XMP_INST_NO_DEFAULT_PAN;

        m.xtra[iu].c5spd = c5spd as f64;

        let xxs = &mut m.mod_.xxs[iu];
        let x = hio_read8(f);
        if x & 0x01 != 0 {
            xxs.flg |= XMP_SAMPLE_16BIT;
            xxs.len >>= 1;
            xxs.lps >>= 1;
            xxs.lpe >>= 1;
        }
        xxs.flg |= if x & 0x02 != 0 {
            XMP_SAMPLE_LOOP_BIDIR
        } else {
            0
        };
        packinfo[iu] = ((x & 0x0c) >> 2) as i32;
    }

    0
}

/// Translation of `get_chunk_sa()`.
fn get_chunk_sa(m: &mut ModuleData, size: u32, f: &mut HioHandle<'_>, data: &mut LocalData) -> i32 {
    let mut smpbuf: Vec<u8> = Vec::new();
    let mut smpbuf_alloc: i32 = -1;
    let mut left: i64 = hio_size(f) - hio_tell(f);

    /* Sanity check */
    if data.has_sa || !data.has_is || data.packinfo.is_none() {
        return -1;
    }
    data.has_sa = true;
    let packinfo = data.packinfo.as_ref().unwrap();

    if (size as i64) < left {
        left = size as i64;
    }

    for i in 0..m.mod_.smp as usize {
        let (xlen, xflg) = (m.mod_.xxs[i].len, m.mod_.xxs[i].flg);

        let mut len = xlen;
        if xflg & XMP_SAMPLE_16BIT != 0 {
            len <<= 1;
        }

        /* Bound the packed sample data size before trying to allocate RAM for it... */
        let size_bound = match packinfo[i] {
            0 => len,
            1 => {
                /* See unpack_sample8: each byte packs to 5 bits minimum. */
                (len >> 3) * 5
            }
            2 => {
                /* See unpack_sample16: each upper byte packs to 5 bits minimum, lower bytes are not packed. */
                (len >> 4) * 13
            }
            _ => {
                /* Sanity check */
                return -1;
            }
        };

        /* Sanity check */
        if left < size_bound as i64 {
            return -1;
        }

        if len > smpbuf_alloc {
            let add = len as usize - smpbuf.len();
            if smpbuf.try_reserve_exact(add).is_err() {
                return -1;
            }
            smpbuf.resize(len as usize, 0);
            smpbuf_alloc = len;
        }

        match packinfo[i] {
            0 => {
                if hio_read(&mut smpbuf, 1, len as usize, f) < len as usize {
                    return -1;
                }
                left -= len as i64;
            }
            1 => {
                len = hio_read32l(f) as i32;
                /* Sanity check */
                if xflg & XMP_SAMPLE_16BIT != 0 {
                    return -1;
                }
                if len <= 0 || len > 0x80000 {
                    /* Max compressed sample size */
                    return -1;
                }
                let Ok(mut buf) = try_vec(len as usize + 4, 0u8) else {
                    return -1;
                };
                if hio_read(&mut buf, 1, len as usize, f) != len as usize {
                    return -1;
                }
                /* The unpack function may read slightly beyond the end. */
                /* (the four bytes after it are zero) */
                if unpack_sample8(&mut smpbuf, &buf, len, xlen) < 0 {
                    return -1;
                }
                left -= len as i64 + 4;
            }
            2 => {
                len = hio_read32l(f) as i32;
                /* Sanity check */
                if !xflg & XMP_SAMPLE_16BIT != 0 {
                    return -1;
                }
                if len <= 0 || len > MAX_SAMPLE_SIZE {
                    return -1;
                }
                let Ok(mut buf) = try_vec(len as usize + 4, 0u8) else {
                    return -1;
                };
                if hio_read(&mut buf, 1, len as usize, f) != len as usize {
                    return -1;
                }
                /* The unpack function may read slightly beyond the end. */
                /* (the four bytes after it are zero) */
                if unpack_sample16(&mut smpbuf, &buf, len, xlen) < 0 {
                    return -1;
                }
                left -= len as i64 + 4;
            }
            _ => {}
        }

        let smpctl = m.smpctl;
        if libxmp_load_sample(
            smpctl,
            None,
            SAMPLE_FLAG_NOLOAD,
            &mut m.mod_.xxs[i],
            Some(&smpbuf),
        ) < 0
        {
            return -1;
        }
    }

    0
}

/// The body of `get_chunk_ve()`, `get_chunk_pe()` and `get_chunk_fe()`.
fn get_chunk_env(
    f: &mut HioHandle<'_>,
    env: &mut Option<Vec<MdlEnvelope>>,
    envnum: &mut i32,
) -> i32 {
    /* Sanity check */
    if env.is_some() {
        return -1;
    }

    *envnum = hio_read8(f) as i32;
    if *envnum == 0 {
        return 0;
    }

    let Ok(e) = try_vec(*envnum as usize, MdlEnvelope::default()) else {
        return -1;
    };
    let e = env.insert(e);

    for x in e.iter_mut() {
        x.num = hio_read8(f);
        hio_read(&mut x.data, 1, 30, f);
        x.sus = hio_read8(f);
        x.loop_ = hio_read8(f);
    }

    0
}

/// Translation of `get_chunk_ve()`.
fn get_chunk_ve(
    _m: &mut ModuleData,
    _size: u32,
    f: &mut HioHandle<'_>,
    data: &mut LocalData,
) -> i32 {
    get_chunk_env(f, &mut data.v_env, &mut data.v_envnum)
}

/// Translation of `get_chunk_pe()`.
fn get_chunk_pe(
    _m: &mut ModuleData,
    _size: u32,
    f: &mut HioHandle<'_>,
    data: &mut LocalData,
) -> i32 {
    get_chunk_env(f, &mut data.p_env, &mut data.p_envnum)
}

/// Translation of `get_chunk_fe()`.
fn get_chunk_fe(
    _m: &mut ModuleData,
    _size: u32,
    f: &mut HioHandle<'_>,
    data: &mut LocalData,
) -> i32 {
    get_chunk_env(f, &mut data.f_env, &mut data.f_envnum)
}

/// Translation of `mdl_load()`.
fn mdl_load(m: &mut ModuleData, f: &mut HioHandle<'_>, _start: i32) -> i32 {
    let mut buf = [0u8; 8];
    let mut data = LocalData::default();
    let mut retval = 0;

    /* Check magic and get version */
    hio_read32b(f);
    if hio_read(&mut buf, 1, 1, f) < 1 {
        return -1;
    }

    let mut handle = libxmp_iff_new::<LocalData>();

    /* IFFoid chunk IDs */
    libxmp_iff_register(&mut handle, b"IN", get_chunk_in); /* Module info */
    libxmp_iff_register(&mut handle, b"TR", get_chunk_tr); /* Tracks */
    libxmp_iff_register(&mut handle, b"SA", get_chunk_sa); /* Sampled data */
    libxmp_iff_register(&mut handle, b"VE", get_chunk_ve); /* Volume envelopes */
    libxmp_iff_register(&mut handle, b"PE", get_chunk_pe); /* Pan envelopes */
    libxmp_iff_register(&mut handle, b"FE", get_chunk_fe); /* Pitch envelopes */

    if msn(buf[0]) != 0 {
        libxmp_iff_register(&mut handle, b"II", get_chunk_ii); /* Instruments */
        libxmp_iff_register(&mut handle, b"PA", get_chunk_pa); /* Patterns */
        libxmp_iff_register(&mut handle, b"IS", get_chunk_is); /* Sample info */
    } else {
        libxmp_iff_register(&mut handle, b"PA", get_chunk_p0); /* Old 0.0 patterns */
        libxmp_iff_register(&mut handle, b"IS", get_chunk_i0); /* Old 0.0 Sample info */
    }

    /* MDL uses a IFF-style file format with 16 bit IDs and little endian
     * 32 bit chunk size. There's only one chunk per data type (i.e. one
     * big chunk for all samples).
     */
    libxmp_iff_id_size(&mut handle, 2);
    libxmp_iff_set_quirk(&mut handle, IFF_LITTLE_ENDIAN);

    let s = format!("Digitrakker MDL {}.{}", msn(buf[0]), lsn(buf[0]));
    libxmp_set_type(m, s.as_bytes());

    m.volbase = 0xff;
    m.c4rate = C4_NTSC_RATE;

    data.v_envnum = 0;
    data.p_envnum = 0;
    data.f_envnum = 0;
    'err: {
        let (Ok(s_index), Ok(i_index), Ok(v_index), Ok(p_index), Ok(f_index)) = (
            try_vec(256, 0),
            try_vec(256, 0),
            try_vec(256, -1),
            try_vec(256, -1),
            try_vec(256, -1),
        ) else {
            break 'err;
        };
        data.s_index = s_index;
        data.i_index = i_index;
        data.v_index = v_index;
        data.p_index = p_index;
        data.f_index = f_index;

        /* Load IFFoid chunks */
        if libxmp_iff_load(&handle, m, f, &mut data) < 0 {
            libxmp_iff_release(handle);
            retval = -1;
            break 'err;
        }

        libxmp_iff_release(handle);

        let mod_ = &mut m.mod_;
        /* Reindex instruments */
        for i in 0..mod_.trk {
            let ins = mod_.ins;
            for e in mod_.track_mut(i).event.iter_mut() {
                for l in 0..ins {
                    if e.ins != 0 && e.ins as i32 == data.i_index[l as usize] {
                        e.ins = (l + 1) as u8;
                        break;
                    }
                }
            }
        }

        /* Reindex envelopes, etc. */
        for i in 0..mod_.ins as usize {
            let xxi = &mut mod_.xxi[i];
            fix_env(
                i,
                &mut xxi.aei,
                data.v_env.as_deref(),
                &data.v_index,
                data.v_envnum,
            );
            fix_env(
                i,
                &mut xxi.pei,
                data.p_env.as_deref(),
                &data.p_index,
                data.p_envnum,
            );
            fix_env(
                i,
                &mut xxi.fei,
                data.f_env.as_deref(),
                &data.f_index,
                data.f_envnum,
            );

            for j in 0..xxi.nsm as usize {
                for k in 0..mod_.smp {
                    if xxi.sub[j].sid == data.s_index[k as usize] {
                        xxi.sub[j].sid = k;
                        /*libxmp_c2spd_to_note(data.c2spd[k],
                        &mod->xxi[i].sub[j].xpo, &mod->xxi[i].sub[j].fin);*/
                        break;
                    }
                }
            }
        }
    }

    m.quirk |= QUIRK_FINEFX | QUIRK_KEYOFF;
    m.read_event_type = READ_EVENT_FT2;

    retval
}
