/* FluidSynth - A Software Synthesizer
 *
 * Copyright (C) 2003  Peter Hanappe and others.
 *
 * This library is free software; you can redistribute it and/or
 * modify it under the terms of the GNU Lesser General Public License
 * as published by the Free Software Foundation; either version 2.1 of
 * the License, or (at your option) any later version.
 *
 * This library is distributed in the hope that it will be useful, but
 * WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the GNU
 * Lesser General Public License for more details.
 *
 * You should have received a copy of the GNU Lesser General Public
 * License along with this library; if not, see
 * <https://www.gnu.org/licenses/>.
 */
// Modified 2026-10-17: translated into Rust from fluid_gen.c, fluid_gen.h
// and include/fluidsynth/gen.h in SDL_mixer 3.3.0's FluidSynth 2.5.7 (see
// NOTICE).

use crate::synth::chan::FluidChannel;
use crate::FluidReal;

/* enum fluid_gen_type: Generator (effect) numbers (Soundfont 2.01
 * specifications section 8.1.3) */
pub(crate) const GEN_STARTADDROFS: usize = 0; /* Sample start address offset (0-32767) */
pub(crate) const GEN_ENDADDROFS: usize = 1; /* Sample end address offset (-32767-0) */
pub(crate) const GEN_STARTLOOPADDROFS: usize = 2; /* Sample loop start address offset (-32767-32767) */
pub(crate) const GEN_ENDLOOPADDROFS: usize = 3; /* Sample loop end address offset (-32767-32767) */
pub(crate) const GEN_STARTADDRCOARSEOFS: usize = 4; /* Sample start address coarse offset (X 32768) */
pub(crate) const GEN_MODLFOTOPITCH: usize = 5; /* Modulation LFO to pitch */
pub(crate) const GEN_VIBLFOTOPITCH: usize = 6; /* Vibrato LFO to pitch */
pub(crate) const GEN_MODENVTOPITCH: usize = 7; /* Modulation envelope to pitch */
pub(crate) const GEN_FILTERFC: usize = 8; /* Filter cutoff */
pub(crate) const GEN_FILTERQ: usize = 9; /* Filter Q */
pub(crate) const GEN_MODLFOTOFILTERFC: usize = 10; /* Modulation LFO to filter cutoff */
pub(crate) const GEN_MODENVTOFILTERFC: usize = 11; /* Modulation envelope to filter cutoff */
pub(crate) const GEN_ENDADDRCOARSEOFS: usize = 12; /* Sample end address coarse offset (X 32768) */
pub(crate) const GEN_MODLFOTOVOL: usize = 13; /* Modulation LFO to volume */
pub(crate) const GEN_UNUSED1: usize = 14; /* Unused */
pub(crate) const GEN_CHORUSSEND: usize = 15; /* Chorus send amount */
pub(crate) const GEN_REVERBSEND: usize = 16; /* Reverb send amount */
pub(crate) const GEN_PAN: usize = 17; /* Stereo panning */
pub(crate) const GEN_UNUSED2: usize = 18; /* Unused */
pub(crate) const GEN_UNUSED3: usize = 19; /* Unused */
pub(crate) const GEN_UNUSED4: usize = 20; /* Unused */
pub(crate) const GEN_MODLFODELAY: usize = 21; /* Modulation LFO delay */
pub(crate) const GEN_MODLFOFREQ: usize = 22; /* Modulation LFO frequency */
pub(crate) const GEN_VIBLFODELAY: usize = 23; /* Vibrato LFO delay */
pub(crate) const GEN_VIBLFOFREQ: usize = 24; /* Vibrato LFO frequency */
pub(crate) const GEN_MODENVDELAY: usize = 25; /* Modulation envelope delay */
pub(crate) const GEN_MODENVATTACK: usize = 26; /* Modulation envelope attack */
pub(crate) const GEN_MODENVHOLD: usize = 27; /* Modulation envelope hold */
pub(crate) const GEN_MODENVDECAY: usize = 28; /* Modulation envelope decay */
pub(crate) const GEN_MODENVSUSTAIN: usize = 29; /* Modulation envelope sustain */
pub(crate) const GEN_MODENVRELEASE: usize = 30; /* Modulation envelope release */
pub(crate) const GEN_KEYTOMODENVHOLD: usize = 31; /* Key to modulation envelope hold */
pub(crate) const GEN_KEYTOMODENVDECAY: usize = 32; /* Key to modulation envelope decay */
pub(crate) const GEN_VOLENVDELAY: usize = 33; /* Volume envelope delay */
pub(crate) const GEN_VOLENVATTACK: usize = 34; /* Volume envelope attack */
pub(crate) const GEN_VOLENVHOLD: usize = 35; /* Volume envelope hold */
pub(crate) const GEN_VOLENVDECAY: usize = 36; /* Volume envelope decay */
pub(crate) const GEN_VOLENVSUSTAIN: usize = 37; /* Volume envelope sustain */
pub(crate) const GEN_VOLENVRELEASE: usize = 38; /* Volume envelope release */
pub(crate) const GEN_KEYTOVOLENVHOLD: usize = 39; /* Key to volume envelope hold */
pub(crate) const GEN_KEYTOVOLENVDECAY: usize = 40; /* Key to volume envelope decay */
pub(crate) const GEN_INSTRUMENT: usize = 41; /* Instrument ID (shouldn't be set by user) */
pub(crate) const GEN_RESERVED1: usize = 42; /* Reserved */
pub(crate) const GEN_KEYRANGE: usize = 43; /* MIDI note range */
pub(crate) const GEN_VELRANGE: usize = 44; /* MIDI velocity range */
pub(crate) const GEN_STARTLOOPADDRCOARSEOFS: usize = 45; /* Sample start loop address coarse offset (X 32768) */
pub(crate) const GEN_KEYNUM: usize = 46; /* Fixed MIDI note number */
pub(crate) const GEN_VELOCITY: usize = 47; /* Fixed MIDI velocity value */
pub(crate) const GEN_ATTENUATION: usize = 48; /* Initial volume attenuation */
pub(crate) const GEN_RESERVED2: usize = 49; /* Reserved */
pub(crate) const GEN_ENDLOOPADDRCOARSEOFS: usize = 50; /* Sample end loop address coarse offset (X 32768) */
pub(crate) const GEN_COARSETUNE: usize = 51; /* Coarse tuning */
pub(crate) const GEN_FINETUNE: usize = 52; /* Fine tuning */
pub(crate) const GEN_SAMPLEID: usize = 53; /* Sample ID (shouldn't be set by user) */
pub(crate) const GEN_SAMPLEMODE: usize = 54; /* Sample mode flags */
pub(crate) const GEN_RESERVED3: usize = 55; /* Reserved */
pub(crate) const GEN_SCALETUNE: usize = 56; /* Scale tuning */
pub(crate) const GEN_EXCLUSIVECLASS: usize = 57; /* Exclusive class number */
pub(crate) const GEN_OVERRIDEROOTKEY: usize = 58; /* Sample root note override */

/* the initial pitch is not a "standard" generator. It is not
 * mentioned in the list of generator in the SF2 specifications. It
 * is used, however, as the destination for the default pitch wheel
 * modulator. */
pub(crate) const GEN_PITCH: usize = 59; /* Pitch @note Not a real SoundFont generator */

pub(crate) const GEN_CUSTOM_BALANCE: usize = 60; /* Balance @note Not a "standard" SoundFont generator */
/* non-standard generator for an additional custom high- or low-pass filter */
pub(crate) const GEN_CUSTOM_FILTERFC: usize = 61; /* Filter cutoff frequency in absolute cents used for custom IIR filter, see fluid_synth_set_custom_filter(). @note Not a "standard" SoundFont generator */
pub(crate) const GEN_CUSTOM_FILTERQ: usize = 62; /* Filter Q used for custom IIR filter, see fluid_synth_set_custom_filter(). @note Not a "standard" SoundFont generator */
pub(crate) const GEN_LAST: usize = 63; /* @internal Value defines the count of generators (#fluid_gen_type) */

/// Translation of `fluid_gen_info_t`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct FluidGenInfo {
    #[allow(dead_code)]
    pub num: i8, /* Generator number */
    pub name: &'static str,
    #[allow(dead_code)]
    pub init: i8, /* Does the generator need to be initialized (not used) */
    pub nrpn_scale: i8, /* The scale to convert from NRPN (cfr. fluid_gen_map_nrpn()) */
    pub min: f32,       /* The minimum value */
    pub max: f32,       /* The maximum value */
    pub def: f32,       /* The default value (cfr. fluid_gen_init()) */
}

/// Translation of `fluid_gen_t`: SoundFont generator structure.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct FluidGen {
    pub flags: u8, /* Is the generator set or not (#fluid_gen_flags) */
    pub val: f64,  /* The nominal value */
    pub mod_: f64, /* Change by modulators */
    pub nrpn: f64, /* Change by NRPN messages */
}

/* enum fluid_gen_flags: Enum value for 'flags' field of #fluid_gen_t (not
 * really flags). */
pub(crate) const GEN_UNUSED: u8 = 0; /* Generator value is not set */
pub(crate) const GEN_SET: u8 = 1; /* Generator value is set */

/// `fluid_gen_set_mod()`
#[inline]
pub(crate) fn fluid_gen_set_mod(gen: &mut FluidGen, val: FluidReal) {
    gen.mod_ = val;
}

/// `fluid_gen_set_nrpn()`
#[inline]
#[allow(dead_code)]
pub(crate) fn fluid_gen_set_nrpn(gen: &mut FluidGen, val: FluidReal) {
    gen.nrpn = val;
}

macro_rules! gen {
    ($num:ident, $name:literal, $init:literal, $nrpn:literal, $min:literal, $max:literal, $def:literal) => {
        FluidGenInfo {
            num: $num as i8,
            name: $name,
            init: $init,
            nrpn_scale: $nrpn,
            min: $min,
            max: $max,
            def: $def,
        }
    };
}

/* See SFSpec21 $8.1.3 */
pub(crate) static FLUID_GEN_INFO: [FluidGenInfo; GEN_LAST] = [
    /* number/name             init  nrpn-scale    min        max         def */
    gen!(
        GEN_STARTADDROFS,
        "STARTADDROFS",
        1,
        1,
        0.0f32,
        1e10f32,
        0.0f32
    ),
    gen!(GEN_ENDADDROFS, "ENDADDROFS", 1, 1, -1e10f32, 0.0f32, 0.0f32),
    gen!(
        GEN_STARTLOOPADDROFS,
        "STARTLOOPADDROFS",
        1,
        1,
        -1e10f32,
        1e10f32,
        0.0f32
    ),
    gen!(
        GEN_ENDLOOPADDROFS,
        "ENDLOOPADDROFS",
        1,
        1,
        -1e10f32,
        1e10f32,
        0.0f32
    ),
    gen!(
        GEN_STARTADDRCOARSEOFS,
        "STARTADDRCOARSEOFS",
        0,
        1,
        0.0f32,
        1e10f32,
        0.0f32
    ),
    gen!(
        GEN_MODLFOTOPITCH,
        "MODLFOTOPITCH",
        1,
        2,
        -12000.0f32,
        12000.0f32,
        0.0f32
    ),
    gen!(
        GEN_VIBLFOTOPITCH,
        "VIBLFOTOPITCH",
        1,
        2,
        -12000.0f32,
        12000.0f32,
        0.0f32
    ),
    gen!(
        GEN_MODENVTOPITCH,
        "MODENVTOPITCH",
        1,
        2,
        -12000.0f32,
        12000.0f32,
        0.0f32
    ),
    gen!(
        GEN_FILTERFC,
        "FILTERFC",
        1,
        2,
        1500.0f32,
        13500.0f32,
        13500.0f32
    ),
    gen!(GEN_FILTERQ, "FILTERQ", 1, 1, 0.0f32, 960.0f32, 0.0f32),
    gen!(
        GEN_MODLFOTOFILTERFC,
        "MODLFOTOFILTERFC",
        1,
        2,
        -12000.0f32,
        12000.0f32,
        0.0f32
    ),
    gen!(
        GEN_MODENVTOFILTERFC,
        "MODENVTOFILTERFC",
        1,
        2,
        -12000.0f32,
        12000.0f32,
        0.0f32
    ),
    gen!(
        GEN_ENDADDRCOARSEOFS,
        "ENDADDRCOARSEOFS",
        0,
        1,
        -1e10f32,
        0.0f32,
        0.0f32
    ),
    gen!(
        GEN_MODLFOTOVOL,
        "MODLFOTOVOL",
        1,
        1,
        -960.0f32,
        960.0f32,
        0.0f32
    ),
    gen!(GEN_UNUSED1, "UNUSED1", 0, 0, 0.0f32, 0.0f32, 0.0f32),
    gen!(
        GEN_CHORUSSEND,
        "CHORUSSEND",
        1,
        1,
        0.0f32,
        1000.0f32,
        0.0f32
    ),
    gen!(
        GEN_REVERBSEND,
        "REVERBSEND",
        1,
        1,
        0.0f32,
        1000.0f32,
        0.0f32
    ),
    gen!(GEN_PAN, "PAN", 1, 1, -500.0f32, 500.0f32, 0.0f32),
    gen!(GEN_UNUSED2, "UNUSED2", 0, 0, 0.0f32, 0.0f32, 0.0f32),
    gen!(GEN_UNUSED3, "UNUSED3", 0, 0, 0.0f32, 0.0f32, 0.0f32),
    gen!(GEN_UNUSED4, "UNUSED4", 0, 0, 0.0f32, 0.0f32, 0.0f32),
    gen!(
        GEN_MODLFODELAY,
        "MODLFODELAY",
        1,
        2,
        -12000.0f32,
        5000.0f32,
        -12000.0f32
    ),
    gen!(
        GEN_MODLFOFREQ,
        "MODLFOFREQ",
        1,
        4,
        -16000.0f32,
        4500.0f32,
        0.0f32
    ),
    gen!(
        GEN_VIBLFODELAY,
        "VIBLFODELAY",
        1,
        2,
        -12000.0f32,
        5000.0f32,
        -12000.0f32
    ),
    gen!(
        GEN_VIBLFOFREQ,
        "VIBLFOFREQ",
        1,
        4,
        -16000.0f32,
        4500.0f32,
        0.0f32
    ),
    gen!(
        GEN_MODENVDELAY,
        "MODENVDELAY",
        1,
        2,
        -12000.0f32,
        5000.0f32,
        -12000.0f32
    ),
    gen!(
        GEN_MODENVATTACK,
        "MODENVATTACK",
        1,
        2,
        -12000.0f32,
        8000.0f32,
        -12000.0f32
    ),
    gen!(
        GEN_MODENVHOLD,
        "MODENVHOLD",
        1,
        2,
        -12000.0f32,
        5000.0f32,
        -12000.0f32
    ),
    gen!(
        GEN_MODENVDECAY,
        "MODENVDECAY",
        1,
        2,
        -12000.0f32,
        8000.0f32,
        -12000.0f32
    ),
    gen!(
        GEN_MODENVSUSTAIN,
        "MODENVSUSTAIN",
        0,
        1,
        0.0f32,
        1000.0f32,
        0.0f32
    ),
    gen!(
        GEN_MODENVRELEASE,
        "MODENVRELEASE",
        1,
        2,
        -12000.0f32,
        8000.0f32,
        -12000.0f32
    ),
    gen!(
        GEN_KEYTOMODENVHOLD,
        "KEYTOMODENVHOLD",
        0,
        1,
        -1200.0f32,
        1200.0f32,
        0.0f32
    ),
    gen!(
        GEN_KEYTOMODENVDECAY,
        "KEYTOMODENVDECAY",
        0,
        1,
        -1200.0f32,
        1200.0f32,
        0.0f32
    ),
    gen!(
        GEN_VOLENVDELAY,
        "VOLENVDELAY",
        1,
        2,
        -12000.0f32,
        5000.0f32,
        -12000.0f32
    ),
    gen!(
        GEN_VOLENVATTACK,
        "VOLENVATTACK",
        1,
        2,
        -12000.0f32,
        8000.0f32,
        -12000.0f32
    ),
    gen!(
        GEN_VOLENVHOLD,
        "VOLENVHOLD",
        1,
        2,
        -12000.0f32,
        5000.0f32,
        -12000.0f32
    ),
    gen!(
        GEN_VOLENVDECAY,
        "VOLENVDECAY",
        1,
        2,
        -12000.0f32,
        8000.0f32,
        -12000.0f32
    ),
    gen!(
        GEN_VOLENVSUSTAIN,
        "VOLENVSUSTAIN",
        0,
        1,
        0.0f32,
        1440.0f32,
        0.0f32
    ),
    gen!(
        GEN_VOLENVRELEASE,
        "VOLENVRELEASE",
        1,
        2,
        -12000.0f32,
        8000.0f32,
        -12000.0f32
    ),
    gen!(
        GEN_KEYTOVOLENVHOLD,
        "KEYTOVOLENVHOLD",
        0,
        1,
        -1200.0f32,
        1200.0f32,
        0.0f32
    ),
    gen!(
        GEN_KEYTOVOLENVDECAY,
        "KEYTOVOLENVDECAY",
        0,
        1,
        -1200.0f32,
        1200.0f32,
        0.0f32
    ),
    gen!(GEN_INSTRUMENT, "INSTRUMENT", 0, 0, 0.0f32, 0.0f32, 0.0f32),
    gen!(GEN_RESERVED1, "RESERVED1", 0, 0, 0.0f32, 0.0f32, 0.0f32),
    gen!(GEN_KEYRANGE, "KEYRANGE", 0, 0, 0.0f32, 127.0f32, 0.0f32),
    gen!(GEN_VELRANGE, "VELRANGE", 0, 0, 0.0f32, 127.0f32, 0.0f32),
    gen!(
        GEN_STARTLOOPADDRCOARSEOFS,
        "STARTLOOPADDRCOARSEOFS",
        0,
        1,
        -1e10f32,
        1e10f32,
        0.0f32
    ),
    gen!(GEN_KEYNUM, "KEYNUM", 1, 0, 0.0f32, 127.0f32, -1.0f32),
    gen!(GEN_VELOCITY, "VELOCITY", 1, 1, 0.0f32, 127.0f32, -1.0f32),
    gen!(
        GEN_ATTENUATION,
        "ATTENUATION",
        1,
        1,
        0.0f32,
        1440.0f32,
        0.0f32
    ),
    gen!(GEN_RESERVED2, "RESERVED2", 0, 0, 0.0f32, 0.0f32, 0.0f32),
    gen!(
        GEN_ENDLOOPADDRCOARSEOFS,
        "ENDLOOPADDRCOARSEOFS",
        0,
        1,
        -1e10f32,
        1e10f32,
        0.0f32
    ),
    gen!(
        GEN_COARSETUNE,
        "COARSETUNE",
        0,
        1,
        -120.0f32,
        120.0f32,
        0.0f32
    ),
    gen!(GEN_FINETUNE, "FINETUNE", 0, 1, -99.0f32, 99.0f32, 0.0f32),
    gen!(GEN_SAMPLEID, "SAMPLEID", 0, 0, 0.0f32, 0.0f32, 0.0f32),
    gen!(GEN_SAMPLEMODE, "SAMPLEMODE", 0, 0, 0.0f32, 0.0f32, 0.0f32),
    gen!(GEN_RESERVED3, "RESERVED3", 0, 0, 0.0f32, 0.0f32, 0.0f32),
    gen!(
        GEN_SCALETUNE,
        "SCALETUNE",
        0,
        1,
        0.0f32,
        1200.0f32,
        100.0f32
    ),
    gen!(
        GEN_EXCLUSIVECLASS,
        "EXCLUSIVECLASS",
        0,
        0,
        0.0f32,
        0.0f32,
        0.0f32
    ),
    gen!(
        GEN_OVERRIDEROOTKEY,
        "OVERRIDEROOTKEY",
        1,
        0,
        0.0f32,
        127.0f32,
        -1.0f32
    ),
    gen!(GEN_PITCH, "PITCH", 1, 0, 0.0f32, 127.0f32, 0.0f32),
    gen!(
        GEN_CUSTOM_BALANCE,
        "CUSTOM_BALANCE",
        1,
        0,
        -960.0f32,
        960.0f32,
        0.0f32
    ),
    gen!(
        GEN_CUSTOM_FILTERFC,
        "CUSTOM_FILTERFC",
        1,
        2,
        0.0f32,
        22050.0f32,
        0.0f32
    ),
    gen!(
        GEN_CUSTOM_FILTERQ,
        "CUSTOM_FILTERQ",
        1,
        1,
        0.0f32,
        960.0f32,
        0.0f32
    ),
];

/// Translation of `fluid_gen_init()`: set an array of generators to their
/// initial value.
pub(crate) fn fluid_gen_init(gen: &mut [FluidGen; GEN_LAST], channel: Option<&FluidChannel>) {
    for (i, gen) in gen.iter_mut().enumerate() {
        gen.flags = GEN_UNUSED;
        gen.mod_ = 0.0;
        gen.nrpn = match channel {
            None => 0.0,
            Some(channel) => channel.get_gen(i),
        };
        gen.val = FLUID_GEN_INFO[i].def as f64;
    }
}

/// Translation of `fluid_gen_scale()`.
#[allow(dead_code)]
pub(crate) fn fluid_gen_scale(gen: usize, value: f32) -> FluidReal {
    // (float arithmetic, as fluid_gen_info's members are floats)
    (FLUID_GEN_INFO[gen].min + value * (FLUID_GEN_INFO[gen].max - FLUID_GEN_INFO[gen].min))
        as FluidReal
}

/// Translation of `fluid_gen_scale_nrpn()`.
pub(crate) fn fluid_gen_scale_nrpn(gen: usize, mut data: i32) -> FluidReal {
    data -= 8192;
    data = crate::fluidsynth_priv::fluid_clip(data, -8192, 8192);
    (data * FLUID_GEN_INFO[gen].nrpn_scale as i32) as FluidReal
}

/// Translation of `fluid_gen_name()`.
pub(crate) fn fluid_gen_name(gen: i32) -> String {
    if 0 <= gen && gen < GEN_LAST as i32 {
        return FLUID_GEN_INFO[gen as usize].name.to_string();
    }

    format!("{gen}")
}
