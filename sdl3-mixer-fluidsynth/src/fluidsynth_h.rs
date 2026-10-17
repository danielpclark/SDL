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
// Modified 2026-10-17: translated into Rust from the constants of
// include/fluidsynth/*.h (misc.h, log.h, settings.h, synth.h, gen.h,
// mod.h, sfont.h, midi.h, voice.h) in SDL_mixer 3.3.0's FluidSynth 2.5.7
// (see NOTICE).

//! FluidSynth's public constants.

/* misc.h */
/// Value that indicates success, used by most libfluidsynth functions
/// (`FLUID_OK`).
pub const FLUID_OK: i32 = 0;
/// Value that indicates failure, used by most libfluidsynth functions
/// (`FLUID_FAILED`).
pub const FLUID_FAILED: i32 = -1;

/* settings.h */
/// Hint FLUID_HINT_BOUNDED_BELOW indicates that the LowerBound field of the
/// FLUID_PortRangeHint should be considered meaningful.
pub const FLUID_HINT_BOUNDED_BELOW: i32 = 0x1;
/// Hint FLUID_HINT_BOUNDED_ABOVE indicates that the UpperBound field of the
/// FLUID_PortRangeHint should be considered meaningful.
pub const FLUID_HINT_BOUNDED_ABOVE: i32 = 0x2;
/// Hint FLUID_HINT_TOGGLED indicates that the data item should be
/// considered a Boolean toggle.
pub const FLUID_HINT_TOGGLED: i32 = 0x4;
/// Setting is a list of string options (`FLUID_HINT_OPTIONLIST`).
pub const FLUID_HINT_OPTIONLIST: i32 = 0x02;

/// Settings type (`enum fluid_types_enum`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FluidType {
    /// Undefined type (`FLUID_NO_TYPE`)
    NoType = -1,
    /// Numeric (double) (`FLUID_NUM_TYPE`)
    Num = 0,
    /// Integer (`FLUID_INT_TYPE`)
    Int = 1,
    /// String (`FLUID_STR_TYPE`)
    Str = 2,
    /// Set of values (`FLUID_SET_TYPE`)
    Set = 3,
}

/* synth.h */
/// Sine wave chorus modulation (`FLUID_CHORUS_MOD_SINE`)
pub const FLUID_CHORUS_MOD_SINE: i32 = 0;
/// Triangle wave chorus modulation (`FLUID_CHORUS_MOD_TRIANGLE`)
pub const FLUID_CHORUS_MOD_TRIANGLE: i32 = 1;

/// No interpolation: Fastest, but questionable audio quality
/// (`FLUID_INTERP_NONE`)
pub const FLUID_INTERP_NONE: i32 = 0;
/// Straight-line interpolation: A bit slower, reasonable audio quality
/// (`FLUID_INTERP_LINEAR`)
pub const FLUID_INTERP_LINEAR: i32 = 1;
/// Fourth-order interpolation, best quality, the default
/// (`FLUID_INTERP_4THORDER`)
pub const FLUID_INTERP_4THORDER: i32 = 4;
/// Seventh-order interpolation (`FLUID_INTERP_7THORDER`)
pub const FLUID_INTERP_7THORDER: i32 = 7;
/// Default interpolation method (`FLUID_INTERP_DEFAULT`)
pub const FLUID_INTERP_DEFAULT: i32 = FLUID_INTERP_4THORDER;
/// Highest interpolation method (`FLUID_INTERP_HIGHEST`)
pub const FLUID_INTERP_HIGHEST: i32 = FLUID_INTERP_7THORDER;

/* enum fluid_synth_add_mod */
pub(crate) const FLUID_SYNTH_OVERWRITE: i32 = 0;
pub(crate) const FLUID_SYNTH_ADD: i32 = 1;

/* enum fluid_iir_filter_type */
pub(crate) const FLUID_IIR_DISABLED: i32 = 0;
pub(crate) const FLUID_IIR_LOWPASS: i32 = 1;
pub(crate) const FLUID_IIR_HIGHPASS: i32 = 2;

/* enum fluid_iir_filter_flags */
pub(crate) const FLUID_IIR_Q_LINEAR: i32 = 1 << 0;
pub(crate) const FLUID_IIR_Q_ZERO_OFF: i32 = 1 << 1;
pub(crate) const FLUID_IIR_NO_GAIN_AMP: i32 = 1 << 2;
pub(crate) const FLUID_IIR_BEANLAND: i32 = 1 << 3;

/* enum fluid_midi_channel_type */
pub(crate) const CHANNEL_TYPE_MELODIC: i32 = 0;
pub(crate) const CHANNEL_TYPE_DRUM: i32 = 1;

/* enum fluid_channel_mode_flags */
pub(crate) const FLUID_CHANNEL_POLY_OFF: i32 = 0x01;
pub(crate) const FLUID_CHANNEL_OMNI_OFF: i32 = 0x02;

/* enum fluid_basic_channel_modes */
pub(crate) const FLUID_CHANNEL_MODE_MASK: i32 = FLUID_CHANNEL_OMNI_OFF | FLUID_CHANNEL_POLY_OFF;
pub(crate) const FLUID_CHANNEL_MODE_OMNION_POLY: i32 =
    FLUID_CHANNEL_MODE_MASK & (!FLUID_CHANNEL_OMNI_OFF & !FLUID_CHANNEL_POLY_OFF);
#[allow(dead_code)]
pub(crate) const FLUID_CHANNEL_MODE_OMNION_MONO: i32 =
    FLUID_CHANNEL_MODE_MASK & (!FLUID_CHANNEL_OMNI_OFF & FLUID_CHANNEL_POLY_OFF);
pub(crate) const FLUID_CHANNEL_MODE_OMNIOFF_POLY: i32 =
    FLUID_CHANNEL_MODE_MASK & (FLUID_CHANNEL_OMNI_OFF & !FLUID_CHANNEL_POLY_OFF);
pub(crate) const FLUID_CHANNEL_MODE_OMNIOFF_MONO: i32 =
    FLUID_CHANNEL_MODE_MASK & (FLUID_CHANNEL_OMNI_OFF | FLUID_CHANNEL_POLY_OFF);
pub(crate) const FLUID_CHANNEL_MODE_LAST: i32 = 4;

/* enum fluid_channel_legato_mode */
pub(crate) const FLUID_CHANNEL_LEGATO_MODE_RETRIGGER: i32 = 0;
pub(crate) const FLUID_CHANNEL_LEGATO_MODE_MULTI_RETRIGGER: i32 = 1;

/* enum fluid_channel_portamento_mode */
pub(crate) const FLUID_CHANNEL_PORTAMENTO_MODE_EACH_NOTE: i32 = 0;
pub(crate) const FLUID_CHANNEL_PORTAMENTO_MODE_LEGATO_ONLY: i32 = 1;
pub(crate) const FLUID_CHANNEL_PORTAMENTO_MODE_STACCATO_ONLY: i32 = 2;

/* enum fluid_channel_breath_flags */
pub(crate) const FLUID_CHANNEL_BREATH_POLY: i32 = 0x10;
pub(crate) const FLUID_CHANNEL_BREATH_MONO: i32 = 0x20;
pub(crate) const FLUID_CHANNEL_BREATH_SYNC: i32 = 0x40;

/* enum fluid_portamento_time_mode */
pub(crate) const FLUID_PORTAMENTO_TIME_MODE_AUTO: i32 = 0;
pub(crate) const FLUID_PORTAMENTO_TIME_MODE_XG_GS: i32 = 1;
pub(crate) const FLUID_PORTAMENTO_TIME_MODE_LINEAR: i32 = 2;

/* mod.h: enum fluid_mod_flags */
pub(crate) const FLUID_MOD_POSITIVE: u8 = 0;
pub(crate) const FLUID_MOD_NEGATIVE: u8 = 1;
pub(crate) const FLUID_MOD_UNIPOLAR: u8 = 0;
pub(crate) const FLUID_MOD_BIPOLAR: u8 = 2;
pub(crate) const FLUID_MOD_LINEAR: u8 = 0;
pub(crate) const FLUID_MOD_CONCAVE: u8 = 4;
pub(crate) const FLUID_MOD_CONVEX: u8 = 8;
pub(crate) const FLUID_MOD_SWITCH: u8 = 12;
pub(crate) const FLUID_MOD_GC: u8 = 0;
pub(crate) const FLUID_MOD_CC: u8 = 16;
pub(crate) const FLUID_MOD_CUSTOM: u8 = 0x40;
pub(crate) const FLUID_MOD_SIN: u8 = 0x80;

/* enum fluid_mod_transforms */
pub(crate) const FLUID_MOD_TRANSFORM_LINEAR: u8 = 0;
pub(crate) const FLUID_MOD_TRANSFORM_ABS: u8 = 2;

/* enum fluid_mod_src */
pub(crate) const FLUID_MOD_NONE: u8 = 0;
pub(crate) const FLUID_MOD_VELOCITY: u8 = 2;
pub(crate) const FLUID_MOD_KEY: u8 = 3;
pub(crate) const FLUID_MOD_KEYPRESSURE: u8 = 10;
pub(crate) const FLUID_MOD_CHANNELPRESSURE: u8 = 13;
pub(crate) const FLUID_MOD_PITCHWHEEL: u8 = 14;
pub(crate) const FLUID_MOD_PITCHWHEELSENS: u8 = 16;

/* voice.h: enum fluid_voice_add_mod */
pub(crate) const FLUID_VOICE_OVERWRITE: i32 = 0;
pub(crate) const FLUID_VOICE_ADD: i32 = 1;
pub(crate) const FLUID_VOICE_DEFAULT: i32 = 2;

/* sfont.h */
#[allow(dead_code)]
pub(crate) const FLUID_PRESET_SELECTED: i32 = 0;
#[allow(dead_code)]
pub(crate) const FLUID_PRESET_UNSELECTED: i32 = 1;
#[allow(dead_code)]
pub(crate) const FLUID_SAMPLE_DONE: i32 = 2;
#[allow(dead_code)]
pub(crate) const FLUID_PRESET_PIN: i32 = 3;
#[allow(dead_code)]
pub(crate) const FLUID_PRESET_UNPIN: i32 = 4;

/* enum fluid_sample_type */
pub(crate) const FLUID_SAMPLETYPE_MONO: i32 = 0x1;
pub(crate) const FLUID_SAMPLETYPE_RIGHT: i32 = 0x2;
pub(crate) const FLUID_SAMPLETYPE_LEFT: i32 = 0x4;
pub(crate) const FLUID_SAMPLETYPE_LINKED: i32 = 0x8;
pub(crate) const FLUID_SAMPLETYPE_OGG_VORBIS: i32 = 0x10;
pub(crate) const FLUID_SAMPLETYPE_ROM: i32 = 0x8000;

/* midi.h: enum fluid_player_status */
/// Player is ready (`FLUID_PLAYER_READY`)
pub const FLUID_PLAYER_READY: i32 = 0;
/// Player is currently playing (`FLUID_PLAYER_PLAYING`)
pub const FLUID_PLAYER_PLAYING: i32 = 1;
/// Player is stopping, but hasn't finished yet (currently unused)
/// (`FLUID_PLAYER_STOPPING`)
pub const FLUID_PLAYER_STOPPING: i32 = 2;
/// Player is finished playing (`FLUID_PLAYER_DONE`)
pub const FLUID_PLAYER_DONE: i32 = 3;
