/*
    format: routines to deal with audio (output) format

    copyright 2008-23 by the mpg123 project - free software under the terms of the LGPL 2.1
    see COPYING and AUTHORS files in distribution or http://mpg123.org
    initially written by Thomas Orgis, starting with parts of the old audio.c, with only faintly manage to show now

    A Major change from mpg123 <= 1.18 is that all encodings are only really
    disabled when done so via specific build configuration. Otherwise, the
    missing support of decoders to produce a certain format is augmented by
    postprocessing that converts the samples. This means happily creating
    data with higher resolution from less accurate decoder output.

    The main point is to still offer float encoding when the decoding core uses
    a fixed point representation that has only 16 bit output. Actually, that's
    the only point: A fixed-point build needs to create float from 16 bit, also
    32 or 24 bit from the same source. That's all there is to it: Everything else
    is covered by fallback synth functions. It may be a further step to check if
    there are cases where conversion in postprocessing works well enough to omit
    a certain specialized decoder ... but usually, they are justified by some
    special way to get from float to integer to begin with.

    I won't cover the case of faking double output with float/s16 decoders here.
    Double precision output is a thing for experimental builds anyway. Mostly
    theoretical and without a point.
*/
// Modified 2026-10-17: translated into Rust from format.c in SDL_mixer 3.3.0's libmpg123 1.33.7 (see NOTICE).

//! The output format choice and the conversions after the synth. Every
//! encoding of the build is here; the forced rate (`MPG123_FORCE_RATE`)
//! and the NtoM fallback aren't, as in a build with `NO_NTOM` (see the
//! crate docs).

use crate::frame::*;
use crate::mpg123::*;
use crate::parse::frame_freq;

/* static int chans[NUM_CHANNELS] = { 1 , 2 }; */
/* only the standard rates */
static MY_RATES: [i64; MPG123_RATES] =
    [8000, 11025, 12000, 16000, 22050, 24000, 32000, 44100, 48000];

static MY_ENCODINGS: [i32; MPG123_ENCODINGS] = [
    MPG123_ENC_SIGNED_16,
    MPG123_ENC_UNSIGNED_16,
    MPG123_ENC_SIGNED_32,
    MPG123_ENC_UNSIGNED_32,
    MPG123_ENC_SIGNED_24,
    MPG123_ENC_UNSIGNED_24,
    /* Floating point range, see below. */
    MPG123_ENC_FLOAT_32,
    MPG123_ENC_FLOAT_64,
    /* 8 bit range, see below. */
    MPG123_ENC_SIGNED_8,
    MPG123_ENC_UNSIGNED_8,
    MPG123_ENC_ULAW_8,
    MPG123_ENC_ALAW_8,
];

/* Make that match the above table.
And yes, I still don't like this kludgy stuff. */
/* range[0] <= i < range[1] for forced floating point */
const ENC_FLOAT_RANGE: [usize; 2] = [6, 8];
/* same for 8 bit encodings */
const ENC_8BIT_RANGE: [usize; 2] = [8, 12];
// for 24 bit quality (24 and 32 bit integers)
const ENC_24BIT_RANGE: [usize; 2] = [2, 6];
// for completeness, the 16 bits
const ENC_16BIT_RANGE: [usize; 2] = [0, 2];

/*
    Only one type of float is supported.
    Actually, double is a very special experimental case not occuring in normal
    builds. Might actually get rid of it.

    Remember here: Also with REAL_IS_FIXED, I want to be able to produce float
    output (f32) via post-processing.
*/
const MPG123_FLOAT_ENC: i32 = MPG123_ENC_FLOAT_32;

/* The list of actually possible encodings. */
static GOOD_ENCODINGS: [i32; 11] = [
    MPG123_ENC_SIGNED_16,
    MPG123_ENC_UNSIGNED_16,
    MPG123_ENC_SIGNED_32,
    MPG123_ENC_UNSIGNED_32,
    MPG123_ENC_SIGNED_24,
    MPG123_ENC_UNSIGNED_24,
    MPG123_FLOAT_ENC,
    MPG123_ENC_SIGNED_8,
    MPG123_ENC_UNSIGNED_8,
    MPG123_ENC_ULAW_8,
    MPG123_ENC_ALAW_8,
];

/* Check if encoding is a valid one in this build.
...lazy programming: linear search. */
fn good_enc(enc: i32) -> bool {
    GOOD_ENCODINGS.contains(&enc)
}

/// Translation of `mpg123_rates()`.
pub fn mpg123_rates() -> &'static [i64] {
    &MY_RATES
}

/// Translation of `mpg123_encodings()`.
pub fn mpg123_encodings() -> &'static [i32] {
    &GOOD_ENCODINGS
}

/// Translation of `mpg123_encsize()`.
pub fn mpg123_encsize(encoding: i32) -> i32 {
    mpg123_samplesize(encoding)
}

/*	char audio_caps[NUM_CHANNELS][MPG123_RATES+1][MPG123_ENCODINGS]; */

fn rate2num(_mp: Option<&Mpg123Pars>, r: i64) -> i32 {
    for (i, &rate) in MY_RATES.iter().enumerate() {
        if rate == r {
            return i as i32;
        }
    }

    -1
}

fn enc2num(encoding: i32) -> i32 {
    for (i, &enc) in MY_ENCODINGS.iter().enumerate() {
        if enc == encoding {
            return i as i32;
        }
    }

    -1
}

fn cap_fit(p: &Mpg123Pars, nf: &mut AudioFormat, f0: usize, f2: usize) -> bool {
    let c = (nf.channels - 1) as usize;
    let rn = rate2num(Some(p), nf.rate);
    if rn >= 0 {
        for i in f0..f2 {
            if p.audio_caps[c][rn as usize][i] != 0 {
                nf.encoding = MY_ENCODINGS[i];
                return true;
            }
        }
    }
    false
}

// Find a possible encoding with given rate and channel count,
// try differing channel count, too.
// This updates the given format and returns TRUE if an encoding
// was found.
fn enc_chan_fit(
    p: &Mpg123Pars,
    rate: i64,
    nnf: &mut AudioFormat,
    f0: usize,
    f2: usize,
    try_float: bool,
) -> bool {
    let encrange = |range: [usize; 2]| (f0.max(range[0]), f2.min(range[1]));
    let mut nf = *nnf;
    nf.rate = rate;
    let fits = |nf: &mut AudioFormat| {
        let (a, b) = encrange(ENC_16BIT_RANGE);
        if cap_fit(p, nf, a, b) {
            return true;
        }
        let (a, b) = encrange(ENC_24BIT_RANGE);
        if cap_fit(p, nf, a, b) {
            return true;
        }
        let (a, b) = encrange(ENC_FLOAT_RANGE);
        if try_float && cap_fit(p, nf, a, b) {
            return true;
        }
        let (a, b) = encrange(ENC_8BIT_RANGE);
        cap_fit(p, nf, a, b)
    };
    if fits(&mut nf) {
        *nnf = nf;
        return true;
    }

    /* try again with different stereoness */
    if nf.channels == 2 && p.flags & MPG123_FORCE_STEREO == 0 {
        nf.channels = 1;
    } else if nf.channels == 1 && p.flags & MPG123_FORCE_MONO == 0 {
        nf.channels = 2;
    }

    if fits(&mut nf) {
        *nnf = nf;
        return true;
    }
    false
}

/* match constraints against supported audio formats, store possible setup in frame
return: -1: error; 0: no format change; 1: format change */
/// Translation of `INT123_frame_output_format()`.
pub(crate) fn frame_output_format(fr: &mut Mpg123Handle<'_>) -> i32 {
    let mut nf = AudioFormat::default();
    let mut f0 = 0usize;
    let mut f2 = MPG123_ENCODINGS + 1; // Include all encodings by default.
    let p = &fr.p;
    let mut try_float = p.flags & MPG123_FLOAT_FALLBACK == 0;
    /* initialize new format, encoding comes later */
    nf.channels = fr.stereo;
    // I intended the forcing stuff to be weaved into the format support table,
    // but this probably will never happen, as this would change library behaviour.
    // One could introduce an additional effective format table that takes for
    // forcings into account, but that would have to be updated on any flag
    // change. Tedious.
    if p.flags & MPG123_FORCE_8BIT != 0 {
        f0 = ENC_8BIT_RANGE[0];
        f2 = ENC_8BIT_RANGE[1];
    }
    if p.flags & MPG123_FORCE_FLOAT != 0 {
        try_float = true;
        f0 = ENC_FLOAT_RANGE[0];
        f2 = ENC_FLOAT_RANGE[1];
    }

    /* force stereo is stronger */
    if p.flags & MPG123_FORCE_MONO != 0 {
        nf.channels = 1;
    }
    if p.flags & MPG123_FORCE_STEREO != 0 {
        nf.channels = 2;
    }

    // Strategy update: Avoid too early triggering of the NtoM decoder.
    // Main target is the native rate, with any encoding.
    // Then, native rate with any channel count and any encoding.
    // Then, it's down_sample from native rate.
    // As last resort: NtoM rate.
    // So the priority is 1. rate 2. channels 3. encoding.
    // As encodings go, 16 bit is tranditionally preferred as efficient choice.
    // Next in line are wider float and integer encodings, then 8 bit as
    // last resort.

    let freq = frame_freq(fr);
    'end: {
        // Native decoder rate first.
        if enc_chan_fit(p, freq >> p.down_sample, &mut nf, f0, f2, try_float) {
            break 'end;
        }
        // Then downsamplings.
        if p.flags & MPG123_AUTO_RESAMPLE != 0 && p.down_sample < 2 {
            if enc_chan_fit(p, freq >> (p.down_sample + 1), &mut nf, f0, f2, try_float) {
                break 'end;
            }
            if p.down_sample < 1 && enc_chan_fit(p, freq >> 2, &mut nf, f0, f2, try_float) {
                break 'end;
            }
        }
        // And again the whole deal with float fallback.
        if !try_float {
            if enc_chan_fit(p, freq >> p.down_sample, &mut nf, f0, f2, true) {
                break 'end;
            }
            // Then downsamplings.
            if p.flags & MPG123_AUTO_RESAMPLE != 0 && p.down_sample < 2 {
                if enc_chan_fit(p, freq >> (p.down_sample + 1), &mut nf, f0, f2, true) {
                    break 'end;
                }
                if p.down_sample < 1 && enc_chan_fit(p, freq >> 2, &mut nf, f0, f2, true) {
                    break 'end;
                }
            }
        }

        /* Here is the _bad_ end. */
        // Unable to set up output format! Constraints: %s%s%li, %li or %liHz.

        fr.err = MPG123_BAD_OUTFORMAT;
        return -1;
    }

    // end: /* Here is the _good_ end. */
    /* we had a successful match, now see if there's a change */
    if nf.rate == fr.af.rate && nf.channels == fr.af.channels && nf.encoding == fr.af.encoding {
        0 /* the same format as before */
    } else {
        /* a new format */
        fr.af.rate = nf.rate;
        fr.af.channels = nf.channels;
        fr.af.encoding = nf.encoding;
        /* Cache the size of one sample in bytes, for ease of use. */
        fr.af.encsize = mpg123_encsize(fr.af.encoding);
        if fr.af.encsize < 1 {
            // Some unknown encoding??? (%i)

            fr.err = MPG123_BAD_OUTFORMAT;
            return -1;
        }
        /* Set up the decoder synth format. Might differ. */
        fr.af.dec_enc = match fr.af.encoding {
            MPG123_ENC_SIGNED_24 | MPG123_ENC_UNSIGNED_24 | MPG123_ENC_UNSIGNED_32 => {
                MPG123_ENC_SIGNED_32
            }
            MPG123_ENC_UNSIGNED_16 => MPG123_ENC_SIGNED_16,
            _ => fr.af.encoding,
        };
        fr.af.dec_encsize = mpg123_encsize(fr.af.dec_enc);
        1
    }
}

/// Translation of `mpg123_fmt_none()`.
pub(crate) fn mpg123_fmt_none(mp: &mut Mpg123Pars) -> i32 {
    mp.audio_caps = [[[0; MPG123_ENCODINGS]; MPG123_RATES + 1]; NUM_CHANNELS];
    MPG123_OK
}

/// Translation of `mpg123_fmt_all()`.
pub(crate) fn mpg123_fmt_all(mp: &mut Mpg123Pars) -> i32 {
    for ch in 0..NUM_CHANNELS {
        for rate in 0..MPG123_RATES + 1 {
            for enc in 0..MPG123_ENCODINGS {
                mp.audio_caps[ch][rate][enc] = if good_enc(MY_ENCODINGS[enc]) { 1 } else { 0 };
            }
        }
    }

    MPG123_OK
}

/// Translation of `mpg123_fmt2()`.
pub(crate) fn mpg123_fmt2(mp: &mut Mpg123Pars, rate: i64, channels: i32, encodings: i32) -> i32 {
    let mut ch = [0usize, 1];
    let r1;
    let r2;
    if channels & (MPG123_MONO | MPG123_STEREO) == 0 {
        return MPG123_BAD_CHANNEL;
    }

    if channels & MPG123_STEREO == 0 {
        ch[1] = 0; /* {0,0} */
    } else if channels & MPG123_MONO == 0 {
        ch[0] = 1; /* {1,1} */
    }
    if rate != 0 {
        r1 = rate2num(Some(mp), rate);
        r2 = r1 + 1;
    } else {
        r1 = 0;
        r2 = MPG123_RATES as i32 + 1; /* including forced rate */
    }

    if r1 < 0 {
        return MPG123_BAD_RATE;
    }

    /* now match the encodings */
    for ratei in r1..r2 {
        for ic in 0..2 {
            for ie in 0..MPG123_ENCODINGS {
                if good_enc(MY_ENCODINGS[ie])
                    && ((MY_ENCODINGS[ie] & encodings) == MY_ENCODINGS[ie])
                {
                    mp.audio_caps[ch[ic]][ratei as usize][ie] = 1;
                }
            }

            if ch[0] == ch[1] {
                break; /* no need to do it again */
            }
        }
    }

    MPG123_OK
}

// Keep old behaviour, error on rate=0.
/// Translation of `mpg123_fmt()`.
pub(crate) fn mpg123_fmt(mp: &mut Mpg123Pars, rate: i64, channels: i32, encodings: i32) -> i32 {
    if rate == 0 {
        MPG123_BAD_RATE
    } else {
        mpg123_fmt2(mp, rate, channels, encodings)
    }
}

/// Translation of `mpg123_fmt_support()`.
pub(crate) fn mpg123_fmt_support(mp: &Mpg123Pars, rate: i64, encoding: i32) -> i32 {
    let mut ch = 0;
    let ratei = rate2num(Some(mp), rate);
    let enci = enc2num(encoding);
    if ratei < 0 || enci < 0 {
        return 0;
    }
    if mp.audio_caps[0][ratei as usize][enci as usize] != 0 {
        ch |= MPG123_MONO;
    }
    if mp.audio_caps[1][ratei as usize][enci as usize] != 0 {
        ch |= MPG123_STEREO;
    }
    ch
}

/* Call this one to ensure that any valid format will be something different than this. */
/// Translation of `INT123_invalidate_format()`.
pub(crate) fn mpg123_invalidate_format(af: &mut AudioFormat) {
    af.encoding = 0;
    af.rate = 0;
    af.channels = 0;
}

/* Number of bytes the decoder produces. */
/// Translation of `INT123_decoder_synth_bytes()`.
pub(crate) fn decoder_synth_bytes(fr: &Mpg123Handle<'_>, s: i64) -> i64 {
    s * fr.af.dec_encsize as i64 * fr.af.channels as i64
}

/* Samples/bytes for output buffer after post-processing. */
/* take into account: channels, bytes per sample -- NOT resampling!*/
/// Translation of `INT123_samples_to_bytes()`.
pub(crate) fn samples_to_bytes(fr: &Mpg123Handle<'_>, s: i64) -> i64 {
    s * fr.af.encsize as i64 * fr.af.channels as i64
}

/// Translation of `INT123_bytes_to_samples()`.
pub(crate) fn bytes_to_samples(fr: &Mpg123Handle<'_>, b: i64) -> i64 {
    b / fr.af.encsize as i64 / fr.af.channels as i64
}

/* Number of bytes needed for decoding _and_ post-processing. */
/// Translation of `INT123_outblock_bytes()`.
pub(crate) fn outblock_bytes(fr: &Mpg123Handle<'_>, s: i64) -> i64 {
    let encsize = if fr.af.encoding & MPG123_ENC_24 != 0 {
        4 /* Intermediate 32 bit. */
    } else if fr.af.encsize > fr.af.dec_encsize {
        fr.af.encsize
    } else {
        fr.af.dec_encsize
    };
    s * encsize as i64 * fr.af.channels as i64
}

/* Remove every fourth byte, facilitating conversion from 32 bit to 24 bit integers.
This has to be aware of endianness, of course. */
fn chop_fourth_byte(buf: &mut OutBuffer) {
    let mut wpos = 0usize;
    let mut rpos = 0usize;
    let blocks = buf.fill / 4;
    for _i in 0..blocks {
        // (DROP4BYTE())
        if cfg!(target_endian = "big") {
            /* Highest byte first. Drop last. */
            buf.data.copy_within(rpos..rpos + 3, wpos);
        } else {
            /* Lowest byte first, drop that. */
            buf.data.copy_within(rpos + 1..rpos + 4, wpos);
        }
        wpos += 3;
        rpos += 4;
    }
    buf.fill = wpos;
}

fn conv_s32_to_u32(buf: &mut OutBuffer) {
    let count = buf.fill / 4;

    for i in 0..count {
        let s = i32::from_ne_bytes(buf.data[i * 4..i * 4 + 4].try_into().unwrap());
        /* Finished 32 bit sample to unsigned 32 bit sample. */
        let u: u32 = if s >= 0 {
            (s as u32).wrapping_add(2147483648)
        } else if s == i32::MIN {
            0 /* Separate because negation would overflow. */
        } else {
            2147483648u32 - ((-s) as u32)
        };
        buf.data[i * 4..i * 4 + 4].copy_from_slice(&u.to_ne_bytes());
    }
}

/* We always assume that whole numbers are written!
partials will be cut out. */

fn conv_s16_to_u16(buf: &mut OutBuffer) {
    let count = buf.fill / 2;

    for i in 0..count {
        let s = i16::from_ne_bytes(buf.data[i * 2..i * 2 + 2].try_into().unwrap());
        let u = (s as i32 + 32768) as u16;
        buf.data[i * 2..i * 2 + 2].copy_from_slice(&u.to_ne_bytes());
    }
}

fn conv_s16_to_f32(buf: &mut OutBuffer) {
    let count = buf.fill / 2;
    /* Does that make any sense? In x86, there is an actual instruction to divide
    float by integer ... but then, if we have that FPU, we don't really need
    fixed point decoder hacks ...? */
    let scale = (1.0 / SHORT_SCALE as f64) as f32;

    if buf.size < count * 4 {
        // Fatal: Buffer too small for postprocessing!
        return;
    }

    /* Work from the back since output is bigger. */
    for i in (0..count).rev() {
        let s = i16::from_ne_bytes(buf.data[i * 2..i * 2 + 2].try_into().unwrap());
        let f = s as f32 * scale;
        buf.data[i * 4..i * 4 + 4].copy_from_slice(&f.to_ne_bytes());
    }

    buf.fill = count * 4;
}

fn conv_s16_to_s32(buf: &mut OutBuffer) {
    let count = buf.fill / 2;

    if buf.size < count * 4 {
        // Fatal: Buffer too small for postprocessing!
        return;
    }

    /* Work from the back since output is bigger. */
    for i in (0..count).rev() {
        let s = i16::from_ne_bytes(buf.data[i * 2..i * 2 + 2].try_into().unwrap());
        let mut o = s as i32;
        /* Could just shift bytes, but would have to mess with sign bit. */
        o = o.wrapping_mul(S32_RESCALE);
        buf.data[i * 4..i * 4 + 4].copy_from_slice(&o.to_ne_bytes());
    }

    buf.fill = count * 4;
}

/*
    swap_bytes: Swap byte order of samples in a buffer.

    copyright 2018 by the mpg123 project
    licensed under the terms of the LGPL 2.1
    see COPYING and AUTHORS files in distribution or http://mpg123.org

    initially written by Thomas Orgis
*/
fn swap_endian(buf: &mut OutBuffer, block: i32) {
    if block >= 2 {
        let block = block as usize;
        let count = buf.fill / block;
        for k in 0..count {
            buf.data[k * block..(k + 1) * block].reverse();
        }
    }
}

/// Translation of `INT123_postprocess_buffer()`.
pub(crate) fn postprocess_buffer(fr: &mut Mpg123Handle<'_>) {
    /*
        This caters for the final output formats that are never produced by
        decoder synth directly (wide unsigned and 24 bit formats) or that are
        missing because of limited decoder precision (16 bit synth but 32 or
        24 bit output).
    */
    match fr.af.dec_enc {
        MPG123_ENC_SIGNED_32 => match fr.af.encoding {
            MPG123_ENC_UNSIGNED_32 => conv_s32_to_u32(&mut fr.buffer),
            MPG123_ENC_UNSIGNED_24 => {
                conv_s32_to_u32(&mut fr.buffer);
                chop_fourth_byte(&mut fr.buffer);
            }
            MPG123_ENC_SIGNED_24 => chop_fourth_byte(&mut fr.buffer),
            _ => {}
        },
        MPG123_ENC_SIGNED_16 => match fr.af.encoding {
            MPG123_ENC_UNSIGNED_16 => conv_s16_to_u16(&mut fr.buffer),
            MPG123_ENC_FLOAT_32 => conv_s16_to_f32(&mut fr.buffer),
            MPG123_ENC_SIGNED_32 => conv_s16_to_s32(&mut fr.buffer),
            MPG123_ENC_UNSIGNED_32 => {
                conv_s16_to_s32(&mut fr.buffer);
                conv_s32_to_u32(&mut fr.buffer);
            }
            MPG123_ENC_UNSIGNED_24 => {
                conv_s16_to_s32(&mut fr.buffer);
                conv_s32_to_u32(&mut fr.buffer);
                chop_fourth_byte(&mut fr.buffer);
            }
            MPG123_ENC_SIGNED_24 => {
                conv_s16_to_s32(&mut fr.buffer);
                chop_fourth_byte(&mut fr.buffer);
            }
            _ => {}
        },
        _ => {}
    }
    if fr.p.flags & MPG123_FORCE_ENDIAN != 0 {
        let big = fr.p.flags & MPG123_BIG_ENDIAN != 0;
        if big != cfg!(target_endian = "big") {
            swap_endian(&mut fr.buffer, mpg123_encsize(fr.af.encoding));
        }
    }
}
