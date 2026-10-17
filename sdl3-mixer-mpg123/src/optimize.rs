/*
    optimize: get a grip on the different optimizations

    copyright 2006-21 by the mpg123 project - free software under the terms of the LGPL 2.1
    see COPYING and AUTHORS files in distribution or http://mpg123.org
    initially written by Thomas Orgis, inspired by 3DNow stuff in mpg123.[hc]

    Currently, this file contains the struct and function to choose an optimization variant and works only when OPT_MULTI is in effect.
*/
// Modified 2026-10-17: translated into Rust from optimize.h, optimize.c and synths.h in SDL_mixer 3.3.0's libmpg123 1.33.7 (see NOTICE).

//! The choice of decoder and synth functions. SDL_mixer's build of
//! libmpg123 on x86-64 is `OPT_MULTI` with the `x86-64` and `AVX`
//! assembly decoders and the `generic` and `generic_dither` C ones; the
//! automatic choice there is the best one the CPU supports. This
//! translation has the plain C `generic` decoder only (no SIMD/assembly,
//! and no dithering: SDL_mixer never names a decoder, and the automatic
//! choice never takes `generic_dither`), so the automatic choice is
//! `generic`, as on a CPU without the assembly decoders' instructions.
//! Only the 1:1 synths are here: the 2:1 and 4:1 downsampling and the NtoM
//! resampling synths are left out (see the crate docs).

use crate::frame::{frame_buffers, Mpg123Handle};
use crate::layer2::{init_layer12_stuff, init_layer12_table};
use crate::layer3::{init_layer3_gainpow2, init_layer3_stuff};
use crate::mpg123::*;
use crate::synth::*;
use crate::tabinit::{make_conv16to8_table, make_decode_tables};

/// Translation of `enum optdec`.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OptDec {
    Autodec = 0,
    Generic,
    GenericDither,
    Idrei,
    Ivier,
    Ifuenf,
    IfuenfDither,
    Mmx,
    Dreidnow,
    Dreidnowext,
    Altivec,
    Sse,
    X86_64,
    Arm,
    Neon,
    Neon64,
    Avx,
    DreidnowVintage,
    DreidnowextVintage,
    SseVintage,
    Nodec,
}

/// `decname[]`
static DECNAME: [&str; 21] = [
    "auto",
    "generic",
    "generic_dither",
    "i386",
    "i486",
    "i586",
    "i586_dither",
    "MMX",
    "3DNow",
    "3DNowExt",
    "AltiVec",
    "SSE",
    "x86-64",
    "ARM",
    "NEON",
    "NEON64",
    "AVX",
    "3DNow_vintage",
    "3DNowExt_vintage",
    "SSE_vintage",
    "nodec",
];

const ALL_DECS: [OptDec; 21] = [
    OptDec::Autodec,
    OptDec::Generic,
    OptDec::GenericDither,
    OptDec::Idrei,
    OptDec::Ivier,
    OptDec::Ifuenf,
    OptDec::IfuenfDither,
    OptDec::Mmx,
    OptDec::Dreidnow,
    OptDec::Dreidnowext,
    OptDec::Altivec,
    OptDec::Sse,
    OptDec::X86_64,
    OptDec::Arm,
    OptDec::Neon,
    OptDec::Neon64,
    OptDec::Avx,
    OptDec::DreidnowVintage,
    OptDec::DreidnowextVintage,
    OptDec::SseVintage,
    OptDec::Nodec,
];

/// Translation of `enum optcla`.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OptCla {
    Nocla = 0,
    Normal,
    Mmxsse,
}

/* The handle needs these types for selecting the decoding routine at runtime.
Not just for optimization, mainly for XtoY, mono/stereo. */
pub(crate) type FuncSynth = fn(&mut [f32], i32, &mut Mpg123Handle<'_>, i32) -> i32;
pub(crate) type FuncSynthMono = fn(&mut [f32], &mut Mpg123Handle<'_>) -> i32;
pub(crate) type FuncSynthStereo = fn(&mut [f32], &mut [f32], &mut Mpg123Handle<'_>) -> i32;

/// Translation of `enum synth_resample` (`r_1to1` only: built as with
/// NO_DOWNSAMPLE and NO_NTOM).
pub(crate) const R_1TO1: usize = 0;
pub(crate) const R_LIMIT: usize = 1;

/// Translation of `enum synth_format`.
pub(crate) const F_16: usize = 0;
pub(crate) const F_8: usize = 1;
pub(crate) const F_REAL: usize = 2;
pub(crate) const F_32: usize = 3;
pub(crate) const F_LIMIT: usize = 4;

/// Translation of `struct synth_s`.
#[derive(Clone, Copy)]
pub(crate) struct SynthS {
    pub plain: [[FuncSynth; F_LIMIT]; R_LIMIT],
    pub stereo: [[FuncSynthStereo; F_LIMIT]; R_LIMIT],
    pub mono2stereo: [[FuncSynthMono; F_LIMIT]; R_LIMIT],
    pub mono: [[FuncSynthMono; F_LIMIT]; R_LIMIT],
}

impl Default for SynthS {
    fn default() -> Self {
        SYNTH_BASE
    }
}

/* The call of left and right plain synth, wrapped.
This may be replaced by a direct stereo optimized synth. */
fn synth_stereo_wrap(
    bandptr_l: &mut [f32],
    bandptr_r: &mut [f32],
    fr: &mut Mpg123Handle<'_>,
) -> i32 {
    let synth = fr.synth.expect("synth set up");
    let mut clip = synth(bandptr_l, 0, fr, 0);
    clip += synth(bandptr_r, 1, fr, 1);
    clip
}

static SYNTH_BASE: SynthS = SynthS {
    plain: [
        /* plain */
        [synth_1to1, synth_1to1_8bit, synth_1to1_real, synth_1to1_s32],
    ],
    stereo: [
        /* stereo, by default only wrappers over plain synth */
        [
            synth_stereo_wrap,
            synth_stereo_wrap,
            synth_stereo_wrap,
            synth_stereo_wrap,
        ],
    ],
    mono2stereo: [/* mono2stereo */ [
        synth_1to1_m2s,
        synth_1to1_8bit_m2s,
        synth_1to1_real_m2s,
        synth_1to1_s32_m2s,
    ]],
    mono: [/* mono*/ [
        synth_1to1_mono,
        synth_1to1_8bit_mono,
        synth_1to1_real_mono,
        synth_1to1_s32_mono,
    ]],
};

/// Translation of `INT123_defdec()` (`defopt`, which is `nodec` in an
/// `OPT_MULTI` build).
pub(crate) fn defdec() -> OptDec {
    OptDec::Nodec
}

/// Translation of `INT123_decclass()`.
pub(crate) fn decclass(type_: OptDec) -> OptCla {
    if type_ == OptDec::Mmx
        || type_ == OptDec::Sse
        || type_ == OptDec::SseVintage
        || type_ == OptDec::DreidnowextVintage
        || type_ == OptDec::Dreidnowext
        || type_ == OptDec::X86_64
        || type_ == OptDec::Neon
        || type_ == OptDec::Neon64
        || type_ == OptDec::Avx
    {
        OptCla::Mmxsse
    } else {
        OptCla::Normal
    }
}

fn find_synth(synth: FuncSynth, synths: &[[FuncSynth; F_LIMIT]; R_LIMIT]) -> bool {
    for row in synths {
        for &s in row {
            if std::ptr::fn_addr_eq(synth, s) {
                return true;
            }
        }
    }
    false
}

/* Determine what kind of decoder is actually active
This depends on runtime choices which may cause fallback to i386 or generic code. */
fn find_dectype(fr: &mut Mpg123Handle<'_>) -> i32 {
    let mut type_ = OptDec::Nodec;
    /* Direct and indirect usage, 1to1 stereo decoding.
    Concentrating on the plain stereo synth should be fine, mono stuff is derived. */
    if let Some(basic_synth) = fr.synth {
        if find_synth(basic_synth, &SYNTH_BASE.plain) {
            type_ = OptDec::Generic;
        }
    }

    if type_ != OptDec::Nodec {
        fr.cpu_opts_type = type_;
        fr.cpu_opts_class = decclass(type_);
        MPG123_OK
    } else {
        fr.err = MPG123_BAD_DECODER_SETUP;
        MPG123_ERR
    }
}

/// Translation of `INT123_set_synth_functions()`: set synth functions for
/// current frame, optimizations handled by opt_* macros.
pub(crate) fn set_synth_functions(fr: &mut Mpg123Handle<'_>) -> i32 {
    let mut resample = None;
    let basic_format; /* Default is always 16bit, or whatever. */

    /* Select the basic output format, different from 16bit: 8bit, real. */
    if fr.af.dec_enc & MPG123_ENC_16 != 0 {
        basic_format = F_16;
    } else if fr.af.dec_enc & MPG123_ENC_8 != 0 {
        basic_format = F_8;
    } else if fr.af.dec_enc & MPG123_ENC_FLOAT != 0 {
        basic_format = F_REAL;
    }
    /* 24 bit integer means decoding to 32 bit first. */
    else if fr.af.dec_enc & MPG123_ENC_32 != 0 || fr.af.dec_enc & MPG123_ENC_24 != 0 {
        basic_format = F_32;
    } else {
        /* Make sure the chosen format is compiled into this lib. */
        return -1;
    }

    /* Be explicit about downsampling variant. */
    if fr.down_sample == 0 {
        resample = Some(R_1TO1);
    }

    let Some(resample) = resample else {
        // INT123_set_synth_functions: This resampling mode is not supported in this build!
        return -1;
    };

    /* Finally selecting the synth functions for stereo / mono. */
    fr.synth = Some(fr.synths.plain[resample][basic_format]);
    fr.synth_stereo = Some(fr.synths.stereo[resample][basic_format]);
    fr.synth_mono = Some(if fr.af.channels == 2 {
        fr.synths.mono2stereo[resample][basic_format] /* Mono MPEG file decoded to stereo. */
    } else {
        fr.synths.mono[resample][basic_format] /* Mono MPEG file decoded to mono. */
    });

    if find_dectype(fr) != MPG123_OK {
        /* Actually determine the currently active decoder breed. */
        fr.err = MPG123_BAD_DECODER_SETUP;
        return MPG123_ERR;
    }

    if frame_buffers(fr) != 0 {
        fr.err = MPG123_NO_BUFFERS;
        return MPG123_ERR;
    }

    if basic_format == F_8 && make_conv16to8_table(fr) != 0 {
        /* it's a bit more work to get proper error propagation up */
        return -1;
    }

    init_layer3_stuff(fr, init_layer3_gainpow2);
    init_layer12_stuff(fr, init_layer12_table);
    fr.make_decode_tables = Some(make_decode_tables);

    /* We allocated the table buffers just now, so (re)create the tables. */
    make_decode_tables(fr);

    0
}

/// Translation of `INT123_frame_cpu_opt()`.
pub(crate) fn frame_cpu_opt(fr: &mut Mpg123Handle<'_>, cpu: Option<&str>) -> i32 {
    let mut done = false;

    let want_dec = dectype(cpu);
    let auto_choose = want_dec == OptDec::Autodec;
    /* Fill whole array of synth functions with generic code first. */
    fr.synths = SYNTH_BASE;

    fr.cpu_opts_type = OptDec::Nodec;

    if !done && (auto_choose || want_dec == OptDec::Generic) {
        fr.cpu_opts_type = OptDec::Generic;
        done = true;
    }

    fr.cpu_opts_class = decclass(fr.cpu_opts_type);

    if done {
        1
    } else {
        // Could not set optimization!
        0
    }
}

/// Translation of `INT123_dectype()`.
pub(crate) fn dectype(decoder: Option<&str>) -> OptDec {
    let decoder = match decoder {
        None => return OptDec::Autodec,
        Some("") => return OptDec::Autodec,
        Some(d) => d,
    };

    for (dt, name) in ALL_DECS
        .iter()
        .zip(DECNAME.iter())
        .take(OptDec::Nodec as usize)
    {
        if decoder.eq_ignore_ascii_case(name) {
            return *dt;
        }
    }

    OptDec::Nodec /* If we found nothing... */
}

/// Translation of `mpg123_current_decoder()`.
pub(crate) fn current_decoder(mh: &Mpg123Handle<'_>) -> &'static str {
    DECNAME[mh.cpu_opts_type as usize]
}
