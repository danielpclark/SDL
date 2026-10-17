// Rust translation of src/loaders/sample.c from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2024 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Loading sample data, converting it to the player's: signed, native
//! endian, interleaved, with guard samples around it for interpolation.
//!
//! A sample's `data` keeps upstream's layout: `SAMPLE_GUARD` (4) bytes
//! before the sample, which upstream reaches as `data[-4..-1]`.

use super::super::common::*;
use super::super::hio::*;
use super::super::*;
use super::*;

/*
 * From the Audio File Formats (version 2.5)
 * Submitted-by: Guido van Rossum <guido@cwi.nl>
 * Last-modified: 27-Aug-1992
 *
 * The Acorn Archimedes uses a variation on U-LAW with the bit order
 * reversed and the sign bit in bit 0.  Being a 'minority' architecture,
 * Arc owners are quite adept at converting sound/image formats from
 * other machines, and it is unlikely that you'll ever encounter sound in
 * one of the Arc's own formats (there are several).
 */
#[rustfmt::skip]
static VDIC_TABLE: [i8; 128] = [
    /*   0 */   0,   0,   0,   0,   0,   0,   0,   0,
    /*   8 */   0,   0,   0,   0,   0,   0,   0,   0,
    /*  16 */   0,   0,   0,   0,   0,   0,   0,   0,
    /*  24 */   1,   1,   1,   1,   1,   1,   1,   1,
    /*  32 */   1,   1,   1,   1,   2,   2,   2,   2,
    /*  40 */   2,   2,   2,   2,   3,   3,   3,   3,
    /*  48 */   3,   3,   4,   4,   4,   4,   5,   5,
    /*  56 */   5,   5,   6,   6,   6,   6,   7,   7,
    /*  64 */   7,   8,   8,   9,   9,  10,  10,  11,
    /*  72 */  11,  12,  12,  13,  13,  14,  14,  15,
    /*  80 */  15,  16,  17,  18,  19,  20,  21,  22,
    /*  88 */  23,  24,  25,  26,  27,  28,  29,  30,
    /*  96 */  31,  33,  34,  36,  38,  40,  42,  44,
    /* 104 */  46,  48,  50,  52,  54,  56,  58,  60,
    /* 112 */  62,  65,  68,  72,  77,  80,  84,  91,
    /* 120 */  95,  98, 103, 109, 114, 120, 126, 127,
];

/* Convert 7 bit samples to 8 bit */
/// Translation of `convert_7bit_to_8bit()`.
fn convert_7bit_to_8bit(p: &mut [u8], l: usize) {
    for b in &mut p[..l] {
        *b <<= 1;
    }
}

/* Convert Archimedes VIDC samples to linear */
/// Translation of `convert_vidc_to_linear()`.
fn convert_vidc_to_linear(p: &mut [u8], l: usize) {
    for b in &mut p[..l] {
        let x = *b;
        let amp = VDIC_TABLE[(x >> 1) as usize];
        *b = if x & 0x01 != 0 {
            amp.wrapping_neg()
        } else {
            amp
        } as u8;
    }
}

/// Translation of `adpcm4_decoder()`: decodes the nibbles at `inp` of
/// `buf` into `buf` from its start (they overlap, as upstream).
fn adpcm4_decoder(buf: &mut [u8], mut inp: usize, tab: &[u8; 16], len: i32) {
    let mut delta: u8 = 0;
    let mut outp = 0;

    let len = (len + 1) / 2;

    for _ in 0..len {
        let b0 = buf[inp];
        let b1 = buf[inp] >> 4;
        inp += 1;
        delta = delta.wrapping_add(tab[(b0 & 0x0f) as usize]);
        buf[outp] = delta;
        outp += 1;
        delta = delta.wrapping_add(tab[(b1 & 0x0f) as usize]);
        buf[outp] = delta;
        outp += 1;
    }
}

/* Convert differential to absolute sample data */
/// Translation of `convert_delta()`.
fn convert_delta(p: &mut [u8], frames: i32, is_16bit: bool, channels: i32) {
    let mut o = 0;

    if is_16bit {
        for _ in 0..channels {
            let mut absval: u16 = 0;
            for _ in 0..frames {
                let w = u16::from_ne_bytes([p[o], p[o + 1]]);
                absval = w.wrapping_add(absval);
                p[o..o + 2].copy_from_slice(&absval.to_ne_bytes());
                o += 2;
            }
        }
    } else {
        for _ in 0..channels {
            let mut absval: u16 = 0;
            for _ in 0..frames {
                absval = (p[o] as u16).wrapping_add(absval);
                p[o] = absval as u8;
                o += 1;
            }
        }
    }
}

/* Convert signed to unsigned sample data */
/// Translation of `convert_signal()`.
fn convert_signal(p: &mut [u8], l: i32, r: bool) {
    if r {
        for w in p.chunks_exact_mut(2).take(l.max(0) as usize) {
            let v = u16::from_ne_bytes([w[0], w[1]]).wrapping_add(0x8000);
            w.copy_from_slice(&v.to_ne_bytes());
        }
    } else {
        for b in p.iter_mut().take(l.max(0) as usize) {
            *b = b.wrapping_add(0x80);
        }
    }
}

/* Convert little-endian 16 bit samples to big-endian */
/// Translation of `convert_endian()`.
fn convert_endian(p: &mut [u8], l: i32) {
    for w in p.chunks_exact_mut(2).take(l.max(0) as usize) {
        w.swap(0, 1);
    }
}

/* Convert non-interleaved stereo to interleaved stereo.
 * Due to tracker quirks this should be done after delta decoding, etc. */
/// Translation of `convert_stereo_interleaved()`.
fn convert_stereo_interleaved(out: &mut [u8], inp: &[u8], frames: i32, is_16bit: bool) {
    let frames = frames as usize;
    if is_16bit {
        let (in_l, in_r) = inp.split_at(frames * 2);
        for i in 0..frames {
            out[i * 4..i * 4 + 2].copy_from_slice(&in_l[i * 2..i * 2 + 2]);
            out[i * 4 + 2..i * 4 + 4].copy_from_slice(&in_r[i * 2..i * 2 + 2]);
        }
    } else {
        let (in_l, in_r) = inp.split_at(frames);
        for i in 0..frames {
            out[i * 2] = in_l[i];
            out[i * 2 + 1] = in_r[i];
        }
    }
}

/// Translation of `libxmp_load_sample()`. Of the module (`m`, which can
/// be `NULL` upstream), only its `smpctl` is used: 0 stands for `NULL`.
pub(crate) fn libxmp_load_sample(
    smpctl: i32,
    mut f: Option<&mut HioHandle<'_>>,
    flags: i32,
    xxs: &mut XmpSample,
    buffer: Option<&[u8]>,
) -> i32 {
    let mut channels = 1;

    /* Adlib FM patches */
    if flags & SAMPLE_FLAG_ADLIB != 0 {
        return 0;
    }

    /* Empty or invalid samples
     */
    if xxs.len <= 0 {
        return 0;
    }

    /* Skip sample loading
     * FIXME: fails for ADPCM samples
     *
     * + Sanity check: skip huge samples (likely corrupt module)
     */
    if xxs.len > MAX_SAMPLE_SIZE || smpctl & XMP_SMPCTL_SKIP != 0 {
        if !flags & SAMPLE_FLAG_NOLOAD != 0 {
            if let Some(f) = f {
                hio_seek(f, xxs.len as i64, SEEK_CUR);
            }
        }
        return 0;
    }

    /* Patches with samples
     * Allocate extra sample for interpolation.
     */
    let mut bytelen = xxs.len;
    let mut framelen = 1;
    let mut extralen = 4;

    if xxs.flg & XMP_SAMPLE_16BIT != 0 {
        bytelen *= 2;
        extralen *= 2;
        framelen *= 2;
    }
    if xxs.flg & XMP_SAMPLE_STEREO != 0 {
        bytelen *= 2;
        extralen *= 2;
        framelen *= 2;
        channels = 2;
    }

    /* If this sample starts at or after EOF, skip it entirely.
     */
    if !flags & SAMPLE_FLAG_NOLOAD != 0 {
        let mut over: i64 = 0;
        let Some(f) = f.as_deref_mut() else {
            return 0;
        };
        let file_pos = hio_tell(f);
        let file_len = hio_size(f);
        if file_pos >= file_len {
            return 0;
        }
        /* If this sample goes past EOF, truncate it. */
        let remaining = file_len - file_pos;
        if flags & SAMPLE_FLAG_ADPCM != 0 {
            let bound = 16 + ((bytelen as i64 + 1) >> 1);
            if remaining < 16 {
                return 0;
            }
            if bound > remaining {
                over = bound - remaining;
                bytelen = ((remaining - 16) << 1) as i32;
            }
        } else if bytelen as i64 > remaining {
            over = bytelen as i64 - remaining;
            bytelen = remaining as i32;
        }

        if over != 0 {
            /* Trim extra bytes non-aligned to sample frame. */
            bytelen -= bytelen & (framelen - 1);

            xxs.len = bytelen;
            if xxs.flg & XMP_SAMPLE_16BIT != 0 {
                xxs.len >>= 1;
            }
            if xxs.flg & XMP_SAMPLE_STEREO != 0 {
                xxs.len >>= 1;
            }
        }
    }

    /* Loop parameters sanity check
     */
    if xxs.lps < 0 {
        xxs.lps = 0;
    }
    if xxs.lpe > xxs.len {
        xxs.lpe = xxs.len;
    }
    if xxs.lps >= xxs.len || xxs.lps >= xxs.lpe {
        xxs.lps = 0;
        xxs.lpe = 0;
        xxs.flg &= !(XMP_SAMPLE_LOOP | XMP_SAMPLE_LOOP_BIDIR);
    }

    /* Disable bidirectional loop flag if sample is not looped
     */
    if xxs.flg & XMP_SAMPLE_LOOP_BIDIR != 0 && !xxs.flg & XMP_SAMPLE_LOOP != 0 {
        xxs.flg &= !XMP_SAMPLE_LOOP_BIDIR;
    }
    if xxs.flg & XMP_SAMPLE_SLOOP_BIDIR != 0 && !xxs.flg & XMP_SAMPLE_SLOOP != 0 {
        xxs.flg &= !XMP_SAMPLE_SLOOP_BIDIR;
    }

    let bytelen_u = bytelen as usize;
    let extralen_u = extralen as usize;
    let framelen_u = framelen as usize;

    /* add guard bytes before the buffer for higher order interpolation */
    let Ok(mut data) = try_vec(SAMPLE_GUARD + bytelen_u + extralen_u, 0u8) else {
        xxs.data = None;
        return -1;
    };

    /* If this is a non-interleaved stereo sample, most conversions need
     * to occur in an intermediate buffer prior to interleaving. Most
     * formats supporting stereo samples use non-interleaved stereo.
     */
    let use_tmp = xxs.flg & XMP_SAMPLE_STEREO != 0 && !flags & SAMPLE_FLAG_INTERLEAVED != 0;
    let mut tmp = Vec::new();
    if use_tmp {
        let Ok(t) = try_vec(bytelen_u, 0u8) else {
            xxs.data = None;
            return -1;
        };
        tmp = t;
    }

    {
        let dest: &mut [u8] = if use_tmp {
            &mut tmp
        } else {
            &mut data[SAMPLE_GUARD..]
        };

        if flags & SAMPLE_FLAG_NOLOAD != 0 {
            let buffer = buffer.unwrap_or(&[]);
            // FIXME (upstream): memcpy() of bytelen bytes from a buffer
            // the loader may have made shorter; the rest stays zero here.
            let n = bytelen_u.min(buffer.len());
            dest[..n].copy_from_slice(&buffer[..n]);
        } else if flags & SAMPLE_FLAG_ADPCM != 0 {
            let f = f.as_deref_mut().unwrap();
            let x2 = ((bytelen + 1) >> 1) as usize;
            let mut table = [0u8; 16];

            if hio_read(&mut table, 1, 16, f) != 16 {
                xxs.data = None;
                return -1;
            }
            if hio_read(&mut dest[x2..], 1, x2, f) != x2 {
                xxs.data = None;
                return -1;
            }
            adpcm4_decoder(dest, x2, &table, bytelen);
        } else {
            let f = f.as_deref_mut().unwrap();
            let x = hio_read(dest, 1, bytelen_u, f);
            if x != bytelen_u {
                dest[x..bytelen_u].fill(0);
            }
        }

        if flags & SAMPLE_FLAG_7BIT != 0 {
            convert_7bit_to_8bit(dest, (xxs.len * channels) as usize);
        }

        /* Fix endianism if needed */
        if xxs.flg & XMP_SAMPLE_16BIT != 0 {
            if cfg!(target_endian = "big") {
                if !flags & SAMPLE_FLAG_BIGEND != 0 {
                    convert_endian(dest, xxs.len * channels);
                }
            } else if flags & SAMPLE_FLAG_BIGEND != 0 {
                convert_endian(dest, xxs.len * channels);
            }
        }

        /* Convert delta samples */
        if flags & SAMPLE_FLAG_DIFF != 0 {
            convert_delta(dest, xxs.len, xxs.flg & XMP_SAMPLE_16BIT != 0, channels);
        } else if flags & SAMPLE_FLAG_8BDIFF != 0 {
            let mut len = xxs.len;
            if xxs.flg & XMP_SAMPLE_16BIT != 0 {
                len *= 2;
            }
            convert_delta(dest, len, false, channels);
        }

        /* Convert samples to signed */
        if flags & SAMPLE_FLAG_UNS != 0 {
            convert_signal(dest, xxs.len * channels, xxs.flg & XMP_SAMPLE_16BIT != 0);
        }

        if flags & SAMPLE_FLAG_VIDC != 0 {
            convert_vidc_to_linear(dest, (xxs.len * channels) as usize);
        }
    }

    /* Done converting individual samples; convert to interleaved. */
    if use_tmp {
        convert_stereo_interleaved(
            &mut data[SAMPLE_GUARD..],
            &tmp,
            xxs.len,
            xxs.flg & XMP_SAMPLE_16BIT != 0,
        );
    }

    /* Check for full loop samples */
    if flags & SAMPLE_FLAG_FULLREP != 0 && xxs.lps == 0 && xxs.len > xxs.lpe {
        xxs.flg |= XMP_SAMPLE_LOOP_FULL;
    }

    /* Add extra samples at end */
    let d = SAMPLE_GUARD;
    for i in 0..extralen_u {
        data[d + bytelen_u + i] = data[d + bytelen_u - framelen_u + i];
    }

    /* Add extra samples at start */
    for i in 1..=4 {
        data[d - i] = data[d + framelen_u - i];
    }

    xxs.data = Some(data);
    0
}

/// Translation of `libxmp_free_sample()`.
pub(crate) fn libxmp_free_sample(s: &mut XmpSample) {
    s.data = None; /* prevent double free in PCM load error */
}
