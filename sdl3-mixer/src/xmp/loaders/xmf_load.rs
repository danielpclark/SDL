// Rust translation of src/loaders/xmf_load.c from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 2023-2026 Alice Rowan <petrifiedrowan@gmail.com>
// This is an altered (translated) version of the original software; see LICENSE.txt.

/* Loader for Astroidea XMF, used by Imperium Galactica and some other modules.
 * This format is completely unrelated to the MIDI XMF format.
 */

//! The Astroidea XMF loader.

use super::super::common::*;
use super::super::dataio::{readmem16l, readmem24l};
use super::super::effects::*;
use super::super::hio::*;
use super::super::*;
use super::common::*;
use super::sample::libxmp_load_sample;
use super::*;

pub(crate) const LIBXMP_LOADER_XMF: FormatLoader = FormatLoader {
    name: "Astroidea XMF",
    test: xmf_test,
    loader: xmf_load,
};

const XMF_SAMPLE_ARRAY_SIZE: usize = 16 * 256;

/// Translation of `xmf_test()`.
fn xmf_test(f: &mut HioHandle<'_>, t: Option<&mut [u8]>, _start: i32) -> i32 {
    let mut buf = [0u8; XMF_SAMPLE_ARRAY_SIZE];
    let mut samples_length: u32 = 0;

    /* This value is 0x03 for all Imperium Galactica modules.
     * The demo "Prostate 666" and all other XMFs use 0x04 instead. */
    let xmf_type = hio_read8(f);
    if xmf_type != 0x03 && xmf_type != 0x04 {
        return -1;
    }

    if hio_read(&mut buf, 1, XMF_SAMPLE_ARRAY_SIZE, f) < XMF_SAMPLE_ARRAY_SIZE {
        return -1;
    }

    /* Test instruments */
    let mut num_ins = 0;
    for i in 0..256 {
        let pos = &buf[i * 16..];
        let loopstart = readmem24l(&pos[0..]);
        let loopend = readmem24l(&pos[3..]);
        let datastart = readmem24l(&pos[6..]);
        let dataend = readmem24l(&pos[9..]);
        let flags = pos[13];
        let srate = readmem16l(&pos[14..]);

        let len = dataend.wrapping_sub(datastart);

        if flags & !(0x04 | 0x08 | 0x10) != 0 {
            return -1;
        }
        /* if ping-pong loop flag is enabled, normal loop flag should be enabled too */
        if (flags & (0x08 | 0x10)) == 0x10 {
            return -1;
        }
        /* if loop flag is enabled, the loop should have a valid end point */
        if flags & 0x08 != 0 && loopend == 0 {
            return -1;
        }
        /* a 16-bit sample should have an even number of bytes */
        if flags & 0x04 != 0 && len & 1 != 0 {
            return -1;
        }
        /* if this slot contains a valid sample, it should have a somewhat realistic middle-c frequency */
        if len != 0 && srate < 100 {
            return -1;
        }

        /* Despite the data start and end values, samples are stored
         * sequentially after the pattern data. These fields are still
         * required to calculate the sample length. */
        if datastart > dataend {
            return -1;
        }

        samples_length = samples_length.wrapping_add(len);

        /* All known XMFs have well-formed loops. */
        if loopend != 0 && (loopstart >= len || loopend > len || loopstart > loopend) {
            return -1;
        }

        if len > 0 {
            num_ins = i as i32 + 1;
        }
    }
    if num_ins > MAX_INSTRUMENTS {
        return -1;
    }

    /* Get pattern data size. */
    if hio_read(&mut buf, 1, 258, f) < 258 {
        return -1;
    }

    let num_channels = buf[256] as i32 + 1;
    let num_patterns = buf[257] as i32 + 1;

    if num_channels > XMP_MAX_CHANNELS as i32 {
        return -1;
    }

    /* Test total module length */
    let samples_start = 0x1103 + num_channels + num_patterns * num_channels * 64 * 6;
    let length = hio_size(f);
    if length < samples_start as i64
        || (length as u64).wrapping_sub(samples_start as u64) < samples_length as u64
    {
        return -1;
    }

    libxmp_read_title(f, t, 0);

    0
}

/* TODO: command pages would be nice, but no official modules rely on 5xy/6xy. */
/// Translation of `xmf_insert_effect()`.
fn xmf_insert_effect(event: &mut XmpEvent, fxt: u8, fxp: u8, chn: i32) {
    if chn == 0 {
        event.fxt = fxt;
        event.fxp = fxp;
    } else {
        event.f2t = fxt;
        event.f2p = fxp;
    }
}

/// Translation of `xmf_translate_effect()`.
fn xmf_translate_effect(event: &mut XmpEvent, effect: u8, mut param: u8, chn: i32) {
    /* Most effects are Protracker compatible. Only the effects actually
     * implemented by Imperium Galactica are handled here. */

    match effect {
        0x00 | 0x01 | 0x02 | 0x0f => {
            /* 0x00: none/arpeggio */
            /* 0x01: portamento up */
            /* 0x02: portamento down */
            /* 0x0f: set speed + set BPM */
            if param != 0 {
                xmf_insert_effect(event, effect, param, chn);
            }
        }

        0x03 => {
            /* tone portamento */
            xmf_insert_effect(event, FX_ULT_TPORTA, param, chn);
        }

        0x04 | 0x0c | 0x0d => {
            /* 0x04: vibrato */
            /* 0x0c: set volume */
            /* 0x0d: break */
            xmf_insert_effect(event, effect, param, chn);
        }

        0x05 | 0x06 | 0x0a => {
            /* 0x05: volume slide + tone portamento */
            /* 0x06: volume slide + vibrato */
            if effect == 0x05 {
                xmf_insert_effect(event, FX_ULT_TPORTA, 0, chn ^ 1);
            }
            if effect == 0x06 {
                xmf_insert_effect(event, FX_VIBRATO, 0, chn ^ 1);
            }

            /* fall-through */

            /* 0x0a: volume slide */
            if param & 0x0f != 0 {
                /* down takes precedence and uses the full param. */
                xmf_insert_effect(event, FX_VOLSLIDE_DN, param << 2, chn);
            } else if param & 0xf0 != 0 {
                xmf_insert_effect(event, FX_VOLSLIDE_UP, param >> 2, chn);
            }
        }

        0x0b => {
            /* pattern jump (jumps to xx + 1) */
            if param < 255 {
                xmf_insert_effect(event, FX_JUMP, param + 1, chn);
            }
        }

        0x0e => {
            /* extended */
            match param >> 4 {
                0x01 | 0x02 | 0x09 | 0x0c | 0x0d | 0x0e => {
                    /* 0x01: fine slide up */
                    /* 0x02: fine slide down */
                    /* 0x09: note retrigger (TODO: only once) */
                    /* 0x0c: note cut */
                    /* 0x0d: note delay */
                    /* 0x0e: pattern delay */
                    if param & 0x0f != 0 {
                        xmf_insert_effect(event, effect, param, chn);
                    }
                }

                0x04 => {
                    /* vibrato waveform */
                    param &= 3;
                    param = if param < 3 { param } else { 2 };
                    xmf_insert_effect(event, effect, param, chn);
                }

                0x06 => {
                    /* pattern loop */
                    xmf_insert_effect(event, effect, param, chn);
                }

                0x0a => {
                    /* fine volume slide up */
                    if param & 0x0f != 0 {
                        xmf_insert_effect(event, FX_F_VSLIDE_UP, (param & 0x0f) << 2, chn);
                    }
                }

                0x0b => {
                    /* fine volume slide down */
                    if param & 0x0f != 0 {
                        xmf_insert_effect(event, FX_F_VSLIDE_DN, (param & 0x0f) << 2, chn);
                    }
                }
                _ => {}
            }
        }

        0x10 => {
            /* panning (4-bit, GUS driver only) */
            param &= 0x0f;
            param |= param << 4;
            xmf_insert_effect(event, FX_SETPAN, param, chn);
        }

        0x11 => {
            /* Ultra Tracker retrigger */
            /* TODO: should support the full param range, needs testing. */
            xmf_insert_effect(
                event,
                FX_EXTENDED,
                ((EX_RETRIG << 4) | (param & 0x0f) as i32) as u8,
                chn,
            );
        }
        _ => {}
    }
}

/// Translation of `xmf_load()`.
fn xmf_load(m: &mut ModuleData, f: &mut HioHandle<'_>, _start: i32) -> i32 {
    /* Imperium Galactica uses 0x03, other Astroidea tracks use 0x04 */
    let xmf_type = hio_read8(f);
    if xmf_type == 0x03 {
        snprintf_bytes(&mut m.mod_.type_, XMP_NAME_SIZE, b"Imperium Galactica XMF");
    } else {
        snprintf_bytes(&mut m.mod_.type_, XMP_NAME_SIZE, b"Astroidea XMF");
    }

    let Ok(mut buf) = try_vec(XMF_SAMPLE_ARRAY_SIZE, 0u8) else {
        return -1;
    };

    /* Count instruments */
    if hio_read(&mut buf, 1, XMF_SAMPLE_ARRAY_SIZE, f) < XMF_SAMPLE_ARRAY_SIZE {
        return -1;
    }

    m.mod_.ins = 0;
    for i in 0..256 {
        let pos = &buf[i * 16..];
        if readmem24l(&pos[9..]) > readmem24l(&pos[6..]) {
            m.mod_.ins = i as i32;
        }
    }
    m.mod_.ins += 1;
    m.mod_.smp = m.mod_.ins;

    if libxmp_init_instrument(m) < 0 {
        return -1;
    }

    /* Instruments */
    for i in 0..m.mod_.ins {
        let iu = i as usize;
        let pos = &buf[iu * 16..];
        let mod_ = &mut m.mod_;

        if libxmp_alloc_subinstrument(mod_, i, 1) < 0 {
            return -1;
        }

        let xxi = &mut mod_.xxi[iu];
        let xxs = &mut mod_.xxs[iu];
        let sub = &mut xxi.sub[0];

        xxs.len = readmem24l(&pos[9..]).wrapping_sub(readmem24l(&pos[6..])) as i32;
        xxs.lps = readmem24l(&pos[0..]) as i32;
        xxs.lpe = readmem24l(&pos[3..]) as i32;
        m.xtra[iu].c5spd = readmem16l(&pos[14..]) as f64;
        sub.vol = pos[12] as i32;
        sub.pan = XMP_INST_NO_DEFAULT_PAN;
        sub.sid = i;

        /* The Sound Blaster driver will only loop if both the
         * loop start and loop end are non-zero. The Sound Blaster
         * driver does not support 16-bit samples or bidirectional
         * looping, and plays these as regular 8-bit looped samples.
         *
         * GUS: 16-bit samples are loaded as 8-bit but play as 16-bit.
         * If the first sample is 16-bit it will partly work (due to
         * having a GUS RAM address of 0?). Other 16-bit samples will
         * read from silence, garbage, or other samples.
         */
        if pos[13] & 0x04 != 0 {
            /* GUS 16-bit flag */
            xxs.flg |= XMP_SAMPLE_16BIT;
            xxs.len >>= 1;
        }
        if pos[13] & 0x08 != 0 {
            /* GUS loop enable */
            xxs.flg |= XMP_SAMPLE_LOOP;
        }
        if pos[13] & 0x10 != 0 {
            /* GUS reverse flag */
            xxs.flg |= XMP_SAMPLE_LOOP_BIDIR;
        }

        if xxs.len > 0 {
            xxi.nsm = 1;
        }
    }

    let mod_ = &mut m.mod_;
    /* Sequence */
    if hio_read(&mut mod_.xxo, 1, 256, f) < 256 {
        return -1;
    }

    mod_.chn = hio_read8(f) as i32 + 1;
    mod_.pat = hio_read8(f) as i32 + 1;
    mod_.trk = mod_.chn * mod_.pat;

    let mut i = 0;
    while i < 256 {
        if mod_.xxo[i] == 0xff {
            break;
        }
        i += 1;
    }
    mod_.len = i as i32;

    /* Panning table (supported by the Gravis UltraSound driver only) */
    let chn = mod_.chn as usize;
    if hio_read(&mut buf, 1, chn, f) < chn {
        return -1;
    }

    for i in 0..chn {
        mod_.xxc[i].pan = 0x80 + (buf[i] as i32 - 7) * 16;
        if mod_.xxc[i].pan > 255 {
            mod_.xxc[i].pan = 255;
        }
    }

    let pat_sz = chn * 6 * 64;
    if pat_sz > XMF_SAMPLE_ARRAY_SIZE {
        if buf.try_reserve_exact(pat_sz - buf.len()).is_err() {
            return -1;
        }
        buf.resize(pat_sz, 0);
    }

    if libxmp_init_pattern(mod_) < 0 {
        return -1;
    }

    /* Patterns */

    for i in 0..mod_.pat {
        if libxmp_alloc_pattern_tracks(mod_, i, 64) < 0 {
            return -1;
        }

        if hio_read(&mut buf, 1, pat_sz, f) < pat_sz {
            return -1;
        }

        let mut pos = 0;
        for j in 0..64 {
            for k in 0..mod_.chn {
                let event = mod_.event_mut(i, k, j);
                let p = &buf[pos..pos + 6];

                if p[0] > 0 {
                    event.note = p[0].wrapping_add(36);
                }
                event.ins = p[1];

                xmf_translate_effect(event, p[2], p[5], 0);
                xmf_translate_effect(event, p[3], p[4], 1);
                pos += 6;
            }
        }
    }
    drop(buf);

    /* Sample data */

    /* Despite the GUS sample start and end pointers saved in the file,
     * these are actually just loaded sequentially. */
    for i in 0..m.mod_.ins as usize {
        let smpctl = m.smpctl;
        if libxmp_load_sample(smpctl, Some(f), 0, &mut m.mod_.xxs[i], None) != 0 {
            return -1;
        }
    }

    /* With the Sound Blaster driver, full volume samples have a -0dB mix.
     * Doing this in libxmp (x4 mvolbase) clips a little bit, so use a
     * slightly lower level (x3 mvolbase, ~192 in IT terms).
     *
     * This only applies to the Imperium Galactica tracks; the tracks with
     * 0x04 use the full GUS volume range.
     */
    m.volbase = 0xff;
    m.mvolbase = 48;
    m.mvol = if xmf_type == 0x03 {
        m.mvolbase * 3
    } else {
        m.mvolbase
    };
    0
}
