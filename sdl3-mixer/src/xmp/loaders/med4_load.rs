// Rust translation of src/loaders/med4_load.c from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2026 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

/*
 * MED 2.13 is in Fish disk #424 and has a couple of demo modules, get it
 * from ftp://ftp.funet.fi/pub/amiga/fish/401-500/ff424. Alex Van Starrex's
 * HappySong MED4 is in ff401. MED 3.00 is in ff476.
 */

//! The MED 2.10 (MED4) loader.

use super::super::common::*;
use super::super::effects::*;
use super::super::hio::*;
use super::super::med_extras::*;
use super::super::path::*;
use super::super::*;
use super::common::*;
use super::med::*;
use super::mmd_common::{mmd_alloc_tables, mmd_convert_tempo};
use super::sample::libxmp_load_sample;
use super::*;

const MAGIC_MED4: u32 = magic4(b'M', b'E', b'D', 4);

pub(crate) const LIBXMP_LOADER_MED4: FormatLoader = FormatLoader {
    name: "MED 2.10 MED4",
    test: med4_test,
    loader: med4_load,
};

/// Translation of `med4_test()`.
fn med4_test(f: &mut HioHandle<'_>, t: Option<&mut [u8]>, _start: i32) -> i32 {
    if hio_read32b(f) != MAGIC_MED4 {
        return -1;
    }

    libxmp_read_title(f, t, 0);

    0
}

/// Translation of `fix_effect()`.
fn fix_effect(event: &mut XmpEvent, hexvol: bool) {
    match event.fxt {
        0x00 /* arpeggio */
        | 0x01 /* slide up */
        | 0x02 /* slide down */
        | 0x03 /* portamento */
        | 0x04 /* vibrato? */ => {}
        0x09 => {
            /* set speed (3.00+) */
            if event.fxp >= 0x01 && event.fxp <= 0x20 {
                event.fxt = FX_SPEED;
            } else {
                event.fxt = 0;
                event.fxp = 0;
            }
        }
        0x0c => {
            /* set volume (BCD) */
            if !hexvol {
                event.fxp = (msn(event.fxp) * 10 + lsn(event.fxp)) as u8;
            }
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

/// Translation of `struct stream`.
struct Stream {
    has_nibble: bool,
    value: u8,
}

/// Translation of `stream_init()`.
#[inline]
fn stream_init() -> Stream {
    Stream {
        has_nibble: false,
        value: 0,
    }
}

/// Translation of `stream_read4()`.
#[inline]
fn stream_read4(s: &mut Stream, f: &mut HioHandle<'_>) -> u32 {
    s.has_nibble = !s.has_nibble;
    if !s.has_nibble {
        (s.value & 0x0f) as u32
    } else {
        s.value = hio_read8(f);
        (s.value >> 4) as u32
    }
}

/// Translation of `stream_read8()`.
#[inline]
fn stream_read8(s: &mut Stream, f: &mut HioHandle<'_>) -> u32 {
    let a = stream_read4(s, f);
    let b = stream_read4(s, f);
    (a << 4) | b
}

/// Translation of `stream_read12()`.
#[inline]
fn stream_read12(s: &mut Stream, f: &mut HioHandle<'_>) -> u32 {
    let a = stream_read4(s, f);
    let b = stream_read4(s, f);
    let c = stream_read4(s, f);
    (a << 8) | (b << 4) | c
}

/// Translation of `stream_read16()`.
#[inline]
fn stream_read16(s: &mut Stream, f: &mut HioHandle<'_>) -> u16 {
    let a = stream_read4(s, f);
    let b = stream_read4(s, f);
    let c = stream_read4(s, f);
    let d = stream_read4(s, f);
    ((a << 12) | (b << 8) | (c << 4) | d) as u16
}

/// Translation of `stream_read_aligned16()`.
#[inline]
fn stream_read_aligned16(s: &mut Stream, f: &mut HioHandle<'_>, bits: i32) -> u16 {
    if bits <= 4 {
        return (stream_read4(s, f) << 12) as u16;
    }
    if bits <= 8 {
        return (stream_read8(s, f) << 8) as u16;
    }
    if bits <= 12 {
        return (stream_read12(s, f) << 4) as u16;
    }
    stream_read16(s, f)
}

/// Translation of `struct temp_inst`.
#[derive(Clone, Copy, Default)]
struct TempInst {
    name: [u8; 32],
    loop_start: i32,
    loop_end: i32,
    volume: i32,
    transpose: i32,
}

/// Translation of `med4_sample_check()`.
fn med4_sample_check(m: &mut ModuleData, pos: i32, needed: i32) -> i32 {
    if pos + needed > m.mod_.smp {
        if pos + needed > MAX_SAMPLES {
            return -1;
        }
        if libxmp_realloc_samples(m, pos + needed + 64) < 0 {
            return -1;
        }
    }
    0
}

/// Translation of `med4_load_sampled_instrument()`.
fn med4_load_sampled_instrument(
    f: &mut HioHandle<'_>,
    m: &mut ModuleData,
    i: i32,
    length: i32,
    smp_idx: &mut i32,
    temp_inst: &[TempInst; 64],
) -> i32 {
    let iu = i as usize;
    let mod_ = &mut m.mod_;

    mod_.xxi[iu].nsm = 1;
    if libxmp_alloc_subinstrument(mod_, i, 1) < 0 {
        return -1;
    }

    let sub = &mut mod_.xxi[iu].sub[0];

    sub.vol = temp_inst[iu].volume;
    sub.pan = XMP_INST_NO_DEFAULT_PAN;
    sub.xpo = temp_inst[iu].transpose;
    sub.sid = *smp_idx;

    /* May need to add samples due to external synths. */
    if med4_sample_check(m, *smp_idx, 1) < 0 {
        return -1;
    }

    let xxs = &mut m.mod_.xxs[*smp_idx as usize];

    xxs.len = length;
    xxs.lps = temp_inst[iu].loop_start;
    xxs.lpe = temp_inst[iu].loop_end;
    xxs.flg = if temp_inst[iu].loop_end > 2 {
        XMP_SAMPLE_LOOP
    } else {
        0
    };

    let smpctl = m.smpctl;
    if libxmp_load_sample(smpctl, Some(f), 0, &mut m.mod_.xxs[*smp_idx as usize], None) < 0 {
        return -1;
    }

    /* Limit range to 3 octave (see MED.El toro) */
    let xxi = &mut m.mod_.xxi[iu];
    for j in 0..9 {
        for k in 0..12 {
            let mut xpo = 0;

            if j < 4 {
                xpo = 12 * (4 - j);
            } else if j > 6 {
                xpo = -12 * (j - 6);
            }

            xxi.map[(12 * j + k) as usize].xpo = xpo as i8;
        }
    }

    *smp_idx += 1;
    0
}

/* This is very similar to MMD1 synth/hybrid instruments,
 * but just different enough to be reimplemented here.
 */
/// Translation of `med4_load_synth_instrument()`.
fn med4_load_synth_instrument(
    f: &mut HioHandle<'_>,
    m: &mut ModuleData,
    i: i32,
    type_: i32,
    smp_idx: &mut i32,
    temp_inst: &[TempInst; 64],
) -> i32 {
    let iu = i as usize;
    let mut synth = SynthInstr::default();

    let pos = hio_tell(f);
    if pos < 0 {
        return -1;
    }

    if hio_read32b(f) != magic4(b'M', b'S', b'H', 0) {
        return -1;
    }

    let type2 = hio_read16b(f) as i16 as i32;
    if type2 != -1 && type2 != -2 {
        return -1;
    }
    if type_ != type2 { /* (upstream warns about the type mismatch) */ }

    hio_read16b(f); /* ? - 0000 */
    hio_read16b(f); /* ? - 0000 */
    synth.rep = hio_read16b(f); /* ? */
    synth.replen = hio_read16b(f); /* ? */
    synth.voltbllen = hio_read16b(f);
    synth.wftbllen = hio_read16b(f);
    synth.volspeed = hio_read8(f);
    synth.wfspeed = hio_read8(f);
    synth.wforms = hio_read16b(f);

    if synth.wforms == 0xffff {
        return 0;
    }

    /* Sanity check */
    if synth.voltbllen > 128 || synth.wftbllen > 128 || synth.wforms > 64 {
        return -1;
    }

    let n = synth.voltbllen as usize;
    hio_read(&mut synth.voltbl[..n], 1, n, f);
    let n = synth.wftbllen as usize;
    hio_read(&mut synth.wftbl[..n], 1, n, f);

    for j in 0..synth.wforms as usize {
        synth.wf[j] = hio_read32b(f);
    }

    if hio_error(f) != 0 {
        return -1;
    }

    if libxmp_med_new_instrument_extras(&mut m.mod_.xxi[iu]) != 0 {
        return -1;
    }

    if let Some(me) = med_module_extras_mut(m) {
        me.tracker_version = MED_VER_210;
    }

    let mod_ = &mut m.mod_;
    mod_.xxi[iu].nsm = synth.wforms as i32;
    if libxmp_alloc_subinstrument(mod_, i, synth.wforms as i32) < 0 {
        return -1;
    }

    if let Some(ie) = med_instrument_extras_mut(&mut mod_.xxi[iu]) {
        ie.vts = synth.volspeed as i32;
        ie.wts = synth.wfspeed as i32;
        ie.vtlen = synth.voltbllen as i32;
        ie.wtlen = synth.wftbllen as i32;
    }

    /* Synth may be external--can't always calculate ahead of time. */
    if med4_sample_check(m, *smp_idx, synth.wforms as i32) < 0 {
        return -1;
    }

    let mut j = 0;
    if type_ == -2 && synth.wforms > 0 {
        /* Hybrid */
        hio_seek(f, pos + synth.wf[0] as i64, SEEK_SET);

        let length = hio_read32b(f);
        if hio_read16b(f) != 0 {
            return -1;
        }

        let mod_ = &mut m.mod_;
        let sub = &mut mod_.xxi[iu].sub[0];

        sub.pan = XMP_INST_NO_DEFAULT_PAN;
        sub.vol = temp_inst[iu].volume;
        sub.xpo = temp_inst[iu].transpose;
        sub.sid = *smp_idx;
        sub.fin = 0 /*exp_smp.finetune*/;

        let xxs = &mut mod_.xxs[*smp_idx as usize];

        xxs.len = length as i32;
        xxs.lps = temp_inst[iu].loop_start;
        xxs.lpe = temp_inst[iu].loop_end;
        xxs.flg = if temp_inst[iu].loop_end > 2 {
            XMP_SAMPLE_LOOP
        } else {
            0
        };

        let smpctl = m.smpctl;
        if libxmp_load_sample(smpctl, Some(f), 0, &mut m.mod_.xxs[*smp_idx as usize], None) < 0 {
            return -1;
        }

        *smp_idx += 1;
        j += 1;
    }

    while j < synth.wforms as usize {
        let mod_ = &mut m.mod_;

        /* Sanity check */
        if *smp_idx >= mod_.smp {
            return -1;
        }

        let sub = &mut mod_.xxi[iu].sub[j];

        sub.pan = XMP_INST_NO_DEFAULT_PAN;
        sub.vol = 64;
        sub.xpo = -24;
        sub.sid = *smp_idx;
        sub.fin = 0 /*exp_smp.finetune*/;

        hio_seek(f, pos + synth.wf[j] as i64, SEEK_SET);

        let xxs = &mut mod_.xxs[*smp_idx as usize];

        xxs.len = hio_read16b(f) as i32 * 2;
        xxs.lps = 0;
        xxs.lpe = xxs.len;
        xxs.flg = XMP_SAMPLE_LOOP;

        let smpctl = m.smpctl;
        if libxmp_load_sample(smpctl, Some(f), 0, &mut m.mod_.xxs[*smp_idx as usize], None) < 0 {
            return -1;
        }

        *smp_idx += 1;
        j += 1;
    }

    if mmd_alloc_tables(m, i, &synth) != 0 {
        return -1;
    }

    0
}

/// Translation of `med4_load_instrument()`.
fn med4_load_instrument(
    f: &mut HioHandle<'_>,
    m: &mut ModuleData,
    i: i32,
    length: i32,
    type_: i32,
    smp_idx: &mut i32,
    temp_inst: &[TempInst; 64],
) -> i32 {
    if type_ == 0 {
        /* Sampled */
        return med4_load_sampled_instrument(f, m, i, length, smp_idx, temp_inst);
    }

    if type_ == -1 || type_ == -2 {
        /* Synthetic or Hybrid */
        let pos = hio_tell(f);
        if pos < 0 {
            return -1;
        }

        if med4_load_synth_instrument(f, m, i, type_, smp_idx, temp_inst) < 0 {
            return -1;
        }

        hio_seek(f, pos + length as i64, SEEK_SET);
        return 0;
    }

    /* Skip unknown instrument type */
    hio_seek(f, length as i64, SEEK_CUR);
    0
}

/// Translation of `med4_load_external_instrument()`.
fn med4_load_external_instrument(
    _f: &mut HioHandle<'_>,
    m: &mut ModuleData,
    i: i32,
    smp_idx: &mut i32,
    temp_inst: &[TempInst; 64],
) -> i32 {
    let mut sp = LibxmpPath::default();
    let mut ins_name = [0u8; 32];

    let name = m.mod_.xxi[i as usize].name;
    if libxmp_copy_name_for_fopen(&mut ins_name, &name, 32) != 0 {
        return 0;
    }

    libxmp_path_init(&mut sp);
    if libxmp_find_instrument_file(m, &mut sp, &ins_name) != 0 {
        return 0;
    }

    let s = sp.path.as_deref().and_then(hio_open);
    libxmp_path_free(&mut sp);
    let Some(mut s) = s else {
        return 0;
    };

    let length = hio_size(&s) as i32;
    if length == 0 {
        return 0;
    }

    if length >= 6 {
        /* May be an external synth/hybrid */
        let magic = hio_read32b(&mut s);
        let type_ = hio_read16b(&mut s) as i16 as i32;
        hio_seek(&mut s, 0, SEEK_SET);

        if magic == magic4(b'M', b'S', b'H', 0) && (type_ == -1 || type_ == -2) {
            return med4_load_synth_instrument(&mut s, m, i, type_, smp_idx, temp_inst);
        }
    }

    med4_load_sampled_instrument(&mut s, m, i, length, smp_idx, temp_inst)
}

/// Translation of `med4_load()`.
fn med4_load(m: &mut ModuleData, f: &mut HioHandle<'_>, start: i32) -> i32 {
    let mut trkvol = [0u8; 16];
    let mut buf = [0u8; 1024];
    let mut hexvol = false;
    let mut temp_inst = [TempInst::default(); 64];

    hio_read32b(f); /* Skip magic */

    let mut vermaj = 2;
    let mut vermin = 10;

    /*
     * Check if we have a MEDV chunk at the end of the file
     */
    let mut pos = hio_tell(f);
    if pos < 0 {
        return -1;
    }

    hio_seek(f, 0, SEEK_END);
    if hio_tell(f) > 2000 {
        hio_seek(f, -1024, SEEK_CUR);
        hio_read(&mut buf, 1, 1024, f);
        for i in 0..1013 {
            if &buf[i..i + 8] == b"MEDV\x00\x00\x00\x04" {
                vermaj = buf[i + 10] as i32;
                vermin = buf[i + 11] as i32;
                break;
            }
        }
    }
    hio_seek(f, start as i64 + pos, SEEK_SET);

    let s = format!("MED {}.{:02} MED4", vermaj, vermin);
    snprintf_bytes(&mut m.mod_.type_, XMP_NAME_SIZE, s.as_bytes());

    let mut m0 = hio_read8(f);

    let mut mask: u64 = 0;
    let mut masksz = 0;
    let mut i = 0;
    while m0 != 0 && i < 8 {
        if m0 & 0x80 != 0 {
            mask <<= 8;
            mask |= hio_read8(f) as u64;
            masksz += 1;
        }
        i += 1;
        m0 <<= 1;
    }

    /* CID 128662 (#1 of 1): Bad bit shift operation (BAD_SHIFT)
     * large_shift: left shifting by more than 63 bits has undefined
     * behavior.
     */
    if masksz > 0 {
        mask <<= 8 * (8 - masksz);
    }

    /* read instrument names in temporary space */

    let mut num_ins = 0;
    let mut i = 0;
    while mask != 0 && i < 64 {
        let mut loop_len: u16 = 0;

        if (mask as i64) > 0 {
            i += 1;
            mask <<= 1;
            continue;
        }

        num_ins = i + 1;

        /* read flags */
        let c = hio_read8(f);

        /* read instrument name */
        let size = hio_read8(f) as usize;
        for j in 0..size {
            buf[j] = hio_read8(f);
        }
        buf[size] = 0;

        let ti = &mut temp_inst[i];
        ti.volume = 0x40;

        if (c & 0x01) == 0 {
            ti.loop_start = (hio_read16b(f) as i32) << 1;
        }
        if (c & 0x02) == 0 {
            loop_len = hio_read16b(f) << 1;
        }
        if (c & 0x04) == 0 {
            /* ? Tanko2 (MED 3.00 demo) */
            hio_read8(f);
        }
        if (c & 0x08) == 0 {
            /* Tim Newsham's "span" */
            hio_read8(f);
        }
        if (c & 0x30) == 0 {
            ti.volume = hio_read8(f) as i32;
        }
        if (c & 0x40) == 0 {
            ti.transpose = hio_read8s(f) as i32;
        }

        ti.loop_end = ti.loop_start + loop_len as i32;

        let mut name = [0u8; 33];
        libxmp_copy_adjust(&mut name, &buf, 32);
        // Note (upstream): libxmp_copy_adjust() clears the byte past C's 32
        // byte name, which is the first byte of loop_start in memory (its
        // low byte on a little-endian machine); here the name's terminator
        // goes in a buffer of its own.
        ti.name.copy_from_slice(&name[..32]);

        i += 1;
        mask <<= 1;
    }

    let mod_ = &mut m.mod_;
    mod_.pat = hio_read16b(f) as i32;
    mod_.len = hio_read16b(f) as i32;

    if hio_error(f) != 0 {
        return -1;
    }

    if mod_.pat > 256 || mod_.len > XMP_MAX_MOD_LENGTH as i32 {
        return -1;
    }
    let len = mod_.len as usize;
    hio_read(&mut mod_.xxo, 1, len, f);

    /* From MED V3.00 docs:
     *
     * The left proportional gadget controls the primary tempo. It canbe
     * 1 - 240. The bigger the number, the faster the speed. Note that
     * tempos 1 - 10 are Tracker-compatible (but obsolete, because
     * secondary tempo can be used now).
     */
    let tempo = hio_read16b(f) as i32;
    let transp = hio_read8s(f) as i32;
    let flags = hio_read8s(f) as i32;
    mod_.spd = hio_read16b(f) as i32;

    mod_.bpm = mmd_convert_tempo(tempo, false, false);
    m.time_factor = MED_TIME_FACTOR;

    m.quirk |= QUIRK_RTONCE; /* FF1 */
    if !flags & 0x20 != 0 {
        /* sliding */
        m.quirk |= QUIRK_VSALL | QUIRK_PBALL;
    }

    if flags & 0x10 != 0 {
        /* dec/hex volumes */
        hexvol = true;
    }

    let mod_ = &mut m.mod_;
    /* This is just a guess... */
    if vermaj == 2 {
        /* Happy.med has tempo 5 but loads as 6 */
        mod_.spd = if flags & 0x20 != 0 { 5 } else { 6 };
    }

    hio_seek(f, 20, SEEK_CUR);

    hio_read(&mut trkvol, 1, 16, f);
    hio_read8(f); /* master vol */

    for ti in temp_inst.iter_mut() {
        ti.transpose += transp;
    }

    /* Scan patterns to determine number of channels */
    mod_.chn = 0;
    pos = hio_tell(f);
    if pos < 0 {
        return -1;
    }

    for _ in 0..mod_.pat {
        let size = hio_read8(f) as i32; /* pattern control block */
        let chn = hio_read8(f) as i32;
        if chn > mod_.chn {
            mod_.chn = chn;
        }
        hio_read8(f); /* skip number of rows */
        let plen = hio_read16b(f) as i32;

        hio_seek(f, (size + plen - 4) as i64, SEEK_CUR);
    }

    /* Sanity check */
    if mod_.chn > 16 {
        return -1;
    }

    mod_.trk = mod_.chn * mod_.pat;

    if libxmp_init_pattern(mod_) < 0 {
        return -1;
    }

    hio_seek(f, pos, SEEK_SET);

    /* Load and convert patterns */

    for i in 0..mod_.pat {
        let mut ctl = [0u8; 4];
        let mut linemask = [0u32; 8];
        let mut fxmask = [0u32; 8];

        let size = hio_read8(f) as i64; /* pattern control block */
        pos = hio_tell(f);
        if pos < 0 {
            return -1;
        }
        let chn = hio_read8(f) as i32;
        if chn > mod_.chn {
            return -1;
        }
        let rows = hio_read8(f) as i32 + 1;
        let plen = hio_read16b(f) as i64;
        /* read control byte */
        for j in 0..4 {
            if rows > j * 64 {
                ctl[j as usize] = hio_read8(f);
            } else {
                break;
            }
        }

        if libxmp_alloc_pattern_tracks(mod_, i, rows) < 0 {
            return -1;
        }

        /* initialize masks */
        /* (they're zero) */

        /* read masks */
        let mut num_masks = 0;
        for y in 0..8 {
            if rows > y * 32 {
                let c = ctl[(y / 2) as usize] as i32;
                let s = 4 * (y % 2);
                linemask[y as usize] = if c & (0x80 >> s) != 0 {
                    !0
                } else if c & (0x40 >> s) != 0 {
                    0
                } else {
                    hio_read32b(f)
                };
                fxmask[y as usize] = if c & (0x20 >> s) != 0 {
                    !0
                } else if c & (0x10 >> s) != 0 {
                    0
                } else {
                    hio_read32b(f)
                };
                num_masks += 1;
            } else {
                break;
            }
        }

        hio_seek(f, pos + size, SEEK_SET);
        let mut stream = stream_init();

        for y in 0..num_masks {
            for j in 0..32 {
                let line = y * 32 + j;

                if line >= rows {
                    break;
                }

                if linemask[y as usize] & 0x80000000 != 0 {
                    let mut chmsk = stream_read_aligned16(&mut stream, f, chn) as u32;
                    for k in 0..chn {
                        let event = mod_.event_mut(i, k, line);

                        if chmsk & 0x8000 != 0 {
                            let x = stream_read12(&mut stream, f);
                            event.note = (x >> 4) as u8;
                            if event.note != 0 {
                                event.note = event.note.wrapping_add(48);
                            }
                            event.ins = (x & 0x0f) as u8;
                        }
                        chmsk <<= 1;
                    }
                }

                if fxmask[y as usize] & 0x80000000 != 0 {
                    let mut chmsk = stream_read_aligned16(&mut stream, f, chn) as u32;
                    for k in 0..chn {
                        let event = mod_.event_mut(i, k, line);

                        if chmsk & 0x8000 != 0 {
                            let x = stream_read12(&mut stream, f);
                            event.fxt = (x >> 8) as u8;
                            event.fxp = (x & 0xff) as u8;
                            fix_effect(event, hexvol);
                        }
                        chmsk <<= 1;
                    }
                }

                linemask[y as usize] <<= 1;
                fxmask[y as usize] <<= 1;
            }
        }
        hio_seek(f, pos + size + plen, SEEK_SET);
    }

    mod_.ins = num_ins as i32;

    if libxmp_med_new_module_extras(m) != 0 {
        return -1;
    }

    /*
     * Load samples
     */
    'parse_iff: {
        if !flags & FLAG_INSTRSATT as i32 != 0 {
            /* Song file */
            /* Arbitrary initial count -- may grow during load. */
            m.mod_.smp = m.mod_.ins;

            if libxmp_init_instrument(m) < 0 {
                return -1;
            }

            let mut smp_idx = 0;
            for i in 0..num_ins {
                let xxi = &mut m.mod_.xxi[i];

                strncpy(&mut xxi.name, &temp_inst[i].name, 32);
                xxi.name[31] = b'\0';

                if med4_load_external_instrument(f, m, i as i32, &mut smp_idx, &temp_inst) < 0 {
                    return -1;
                }
            }

            /* Shrink samples to used */
            if m.mod_.smp > smp_idx && libxmp_realloc_samples(m, smp_idx) < 0 {
                return -1;
            }
            break 'parse_iff;
        }

        /* Internal samples */
        mask = hio_read32b(f) as u64;
        if mask == magic4(b'M', b'E', b'D', b'V') as u64 {
            return -1;
        }
        mask <<= 32;
        mask |= hio_read32b(f) as u64;

        mask <<= 1; /* no instrument #0 */

        /* obtain number of samples */
        pos = hio_tell(f);
        if pos < 0 {
            return -1;
        }
        let mut num_smp = 0;
        {
            let mut mask = mask;
            let mut i = 0;
            while mask != 0 && i < 64 {
                if (mask as i64) > 0 {
                    i += 1;
                    mask <<= 1;
                    continue;
                }

                let mut len = hio_read32b(f) as i32;
                let type_ = hio_read16b(f) as i16 as i32;

                if type_ == 0 {
                    num_smp += 1;
                } else if type_ == -1 || type_ == -2 {
                    if len < 22 {
                        return -1;
                    }
                    hio_seek(f, 20, SEEK_CUR);
                    num_smp += hio_read16b(f) as i32;
                    len -= 22;
                }

                if len < 0 || len as i64 > hio_size(f) {
                    return -1;
                }
                hio_seek(f, len as i64, SEEK_CUR);
                i += 1;
                mask <<= 1;
            }
        }
        hio_seek(f, pos, SEEK_SET);

        m.mod_.smp = num_smp;

        if libxmp_init_instrument(m) < 0 {
            return -1;
        }

        let mut smp_idx = 0;
        let mut i = 0;
        while mask != 0 && i < num_ins {
            if (mask as i64) > 0 {
                i += 1;
                mask <<= 1;
                continue;
            }

            let length = hio_read32b(f) as i32;
            let type_ = hio_read16b(f) as i16 as i32; /* instrument type */

            let xxi = &mut m.mod_.xxi[i];
            strncpy(&mut xxi.name, &temp_inst[i].name, 32);
            xxi.name[31] = b'\0';

            if med4_load_instrument(f, m, i as i32, length, type_, &mut smp_idx, &temp_inst) < 0 {
                return -1;
            }
            i += 1;
            mask <<= 1;
        }

        /* Not sure what this was supposed to be, but it isn't present in
         * Synth-a-sysmic.med or any other MED4 module on ModLand. */
        /*hio_read16b(f);*/ /* unknown */
    }

    /* IFF-like section */
    /* parse_iff: */
    while hio_eof(f) == 0 {
        let id = hio_read32b(f) as i32;
        if id <= 0 {
            break;
        }

        let mut size = hio_read32b(f) as i32;
        if size <= 0 {
            break;
        }

        const MEDV: i32 = magic4(b'M', b'E', b'D', b'V') as i32;
        const ANNO: i32 = magic4(b'A', b'N', b'N', b'O') as i32;
        const HLDC: i32 = magic4(b'H', b'L', b'D', b'C') as i32;
        match id {
            MEDV => {
                let ver = hio_read32b(f) as i32;
                size -= 4;
                vermaj = (ver & 0xff00) >> 8;
                vermin = ver & 0xff;
                let _ = (vermaj, vermin);
                if let Some(me) = med_module_extras_mut(m) {
                    me.tracker_version = ver & 0xfff;
                }
            }
            ANNO => {
                /* annotation */
                let s2 = if size < 1023 { size } else { 1023 };
                if let Ok(mut comment) = try_vec(s2 as usize + 1, 0u8) {
                    let read_len = hio_read(&mut comment, 1, s2 as usize, f);
                    comment[read_len] = b'\0';
                    m.comment = Some(comment);

                    size -= s2;
                }
            }
            HLDC => { /* hold & decay */ }
            _ => {}
        }

        hio_seek(f, size as i64, SEEK_CUR);
    }

    m.read_event_type = READ_EVENT_MED;

    0
}
