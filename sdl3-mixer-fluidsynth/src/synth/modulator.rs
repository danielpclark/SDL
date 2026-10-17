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
// Modified 2026-10-17: translated into Rust from fluid_mod.c and
// fluid_mod.h in SDL_mixer 3.3.0's FluidSynth 2.5.7 (see NOTICE).
// Modulator lists are `Vec`s instead of the `next` links. The custom
// mapping functions of fluid_mod_set_custom_mapping() (a public API
// SDL_mixer doesn't use) are left out: no modulator can carry the
// FLUID_MOD_CUSTOM flag here, as neither SoundFont files nor the default
// modulators can set it.

use crate::fluidsynth_h::*;
use crate::midi::midi::*;
use crate::synth::chan::{fluid_channel_is_valid_note, FluidChannel};
use crate::synth::synth::DEFAULT_VEL2FILTER_MOD;
use crate::synth::voice::{
    fluid_voice_get_actual_key, fluid_voice_get_actual_velocity, FluidVoice,
};
use crate::utils::conv::{fluid_concave, fluid_convex};
use crate::utils::conv_tables::FLUID_VEL_CB_SIZE;
use crate::utils::sys::{flog, fluid_log, FLUID_ERR, FLUID_WARN};
use crate::FluidReal;

/// Translation of `fluid_mod_t`: modulator structure.  See SoundFont 2.04
/// PDF section 8.2.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct FluidMod {
    pub dest: u8,    /* Destination generator to control */
    pub src1: u8,    /* Source controller 1 */
    pub flags1: u8,  /* Source controller 1 flags */
    pub src2: u8,    /* Source controller 2 */
    pub flags2: u8,  /* Source controller 2 flags */
    pub trans: u8,   /* Output transform flag */
    pub amount: f64, /* Multiplier amount */
}

pub(crate) const FLUID_MOD_POLAR_MASK: u8 = FLUID_MOD_BIPOLAR | FLUID_MOD_UNIPOLAR;
pub(crate) const FLUID_MOD_MAP_MASK: u8 =
    FLUID_MOD_LINEAR | FLUID_MOD_CONCAVE | FLUID_MOD_CONVEX | FLUID_MOD_SWITCH | FLUID_MOD_SIN;
pub(crate) const FLUID_MOD_SIGN_MASK: u8 = FLUID_MOD_POSITIVE | FLUID_MOD_NEGATIVE;

impl FluidMod {
    /// Translation of `fluid_mod_clone()`: clone the modulators
    /// destination, sources, flags and amount.
    pub(crate) fn clone_from_mod(&mut self, src: &FluidMod) {
        self.dest = src.dest;
        self.src1 = src.src1;
        self.flags1 = src.flags1;
        self.src2 = src.src2;
        self.flags2 = src.flags2;
        self.amount = src.amount;
        self.trans = src.trans;
    }

    /// Translation of `fluid_mod_set_source1()`: set a modulator's primary
    /// source controller and flags.
    #[allow(dead_code)]
    pub(crate) fn set_source1(&mut self, mut src: i32, flags: i32) {
        if (flags as u8 & FLUID_MOD_MAP_MASK) == FLUID_MOD_SIN {
            fluid_log(FLUID_ERR, "This version of fluidsynth no longer supports FLUID_MOD_SIN, pls. use fluid_mod_set_custom_mapping() instead. Disabling source.");
            src = FLUID_MOD_NONE as i32;
        }

        self.src1 = src as u8;
        self.flags1 = flags as u8;
    }

    /// Translation of `fluid_mod_set_source2()`: set a modulator's
    /// secondary source controller and flags.
    #[allow(dead_code)]
    pub(crate) fn set_source2(&mut self, mut src: i32, flags: i32) {
        if (flags as u8 & FLUID_MOD_MAP_MASK) == FLUID_MOD_SIN {
            fluid_log(FLUID_ERR, "This version of fluidsynth no longer supports FLUID_MOD_SIN, pls. use fluid_mod_set_custom_mapping() instead. Disabling source.");
            src = FLUID_MOD_NONE as i32;
        }

        self.src2 = src as u8;
        self.flags2 = flags as u8;
    }

    /// Translation of `fluid_mod_set_dest()`.
    #[allow(dead_code)]
    pub(crate) fn set_dest(&mut self, dest: i32) {
        self.dest = dest as u8;
    }

    /// Translation of `fluid_mod_set_amount()`.
    #[allow(dead_code)]
    pub(crate) fn set_amount(&mut self, amount: f64) {
        self.amount = amount;
    }

    /// Translation of `fluid_mod_set_transform()`.
    #[allow(dead_code)]
    pub(crate) fn set_transform(&mut self, type_: i32) {
        let flag = type_ as u8;
        if flag != FLUID_MOD_TRANSFORM_LINEAR && flag != FLUID_MOD_TRANSFORM_ABS {
            flog!(
                FLUID_ERR,
                "fluid_mod_set_transform() called with invalid transform type {}",
                type_
            );
            return;
        }
        self.trans = flag;
    }
}

/// Translation of `fluid_mod_get_source_value()`: retrieves the initial
/// value from the given source of the modulator.
pub(crate) fn fluid_mod_get_source_value(
    mod_src: u8,
    mod_flags: u8,
    range: &mut FluidReal,
    voice: &FluidVoice,
    chan: &FluidChannel,
) -> FluidReal {
    let mut val: FluidReal;

    if mod_flags & FLUID_MOD_CC != 0 {
        val = chan.get_cc(mod_src as i32) as FluidReal;

        if mod_src as i32 == PORTAMENTO_CTRL {
            // an invalid portamento fromkey should be treated as 0 when it's actually used for modulating
            if !fluid_channel_is_valid_note(val) {
                val = 0.0;
            }
        }
    } else {
        match mod_src {
            FLUID_MOD_NONE => {
                /* SF 2.01 8.2.1 item 0: src enum=0 => value is 1 */
                val = *range;
            }

            FLUID_MOD_VELOCITY => {
                val = fluid_voice_get_actual_velocity(voice) as FluidReal;
            }

            FLUID_MOD_KEY => {
                val = fluid_voice_get_actual_key(voice) as FluidReal;
            }

            FLUID_MOD_KEYPRESSURE => {
                val = chan.get_key_pressure(voice.key as i32) as FluidReal;
            }

            FLUID_MOD_CHANNELPRESSURE => {
                val = chan.get_channel_pressure() as FluidReal;
            }

            FLUID_MOD_PITCHWHEEL => {
                val = chan.get_pitch_bend() as FluidReal;
                *range = 0x4000 as FluidReal;
            }

            FLUID_MOD_PITCHWHEELSENS => {
                val = chan.get_pitch_wheel_sensitivity() as FluidReal;
            }

            _ => {
                flog!(
                    FLUID_ERR,
                    "Unknown modulator source '{}', disabling modulator.",
                    mod_src
                );
                val = 0.0;
            }
        }
    }

    val
}

/// Translation of `fluid_mod_transform_source_value()`: transforms the
/// initial value retrieved by \c fluid_mod_get_source_value into
/// [0.0;1.0].
pub(crate) fn fluid_mod_transform_source_value(
    mod_: &FluidMod,
    mut val: FluidReal,
    range: FluidReal,
    is_src1: bool,
) -> FluidReal {
    /* normalized value, i.e. usually in the range [0;1] */
    let val_norm: FluidReal = val / range;
    /* inverted value used for negative mapping functions */
    let inv_norm: FluidReal = 1.0f32 as FluidReal - 1.0f32 as FluidReal / range - val_norm;

    /* we could also only switch case the lower nibble of mod_flags, however
     * this would keep us from adding further mod types in the future
     *
     * instead just remove the flag(s) we already took care of
     */
    let mod_src = if is_src1 { mod_.src1 } else { mod_.src2 };
    let mut mod_flags = if is_src1 { mod_.flags1 } else { mod_.flags2 };
    mod_flags &= !FLUID_MOD_CC;

    if mod_src == FLUID_MOD_NONE {
        // early opt out if no source has been set
        return 1.0;
    }

    if (mod_flags & FLUID_MOD_CUSTOM) != 0 {
        // (mod->mapping_func is always NULL here, see the top of this file;
        // upstream also sets mod->amount to 0)
        fluid_log(FLUID_ERR, "Modulator has FLUID_MOD_CUSTOM flag set, but doesn't provide a mapping function, disabling modulator.");
        val = 0.0;
    } else if (mod_flags & FLUID_MOD_POLAR_MASK) == FLUID_MOD_UNIPOLAR {
        if (mod_flags & FLUID_MOD_SIGN_MASK) == FLUID_MOD_NEGATIVE {
            val = inv_norm;
        } else {
            val = val_norm;
        }

        if (mod_flags & FLUID_MOD_MAP_MASK) == FLUID_MOD_SWITCH {
            val = if val >= 0.5 { 1.0 } else { 0.0 };
        } else if (mod_flags & FLUID_MOD_MAP_MASK) == FLUID_MOD_CONCAVE {
            // The maximum mapped value should not be 1.0! According to sections 8.4.1 and 8.4.7 it must be 127/128 yet again.
            // Same is assumed for convex below, though this is not explicitly said by the spec.
            val = fmin(
                fluid_concave(FLUID_VEL_CB_SIZE as FluidReal * val),
                (range - 1.0) / range,
            );
        } else if (mod_flags & FLUID_MOD_MAP_MASK) == FLUID_MOD_CONVEX {
            val = fmin(
                fluid_convex(FLUID_VEL_CB_SIZE as FluidReal * val),
                (range - 1.0) / range,
            );
        } else {
            // FLUID_MOD_LINEAR, just keep it as is
        }
    } else if (mod_flags & FLUID_MOD_POLAR_MASK) == FLUID_MOD_BIPOLAR {
        if (mod_flags & FLUID_MOD_SIGN_MASK) == FLUID_MOD_NEGATIVE {
            val = if inv_norm == (range - 1.0) / range {
                inv_norm
            } else {
                -1.0 + 2.0 * inv_norm
            };
        } else {
            val = if val_norm == (range - 1.0) / range {
                val_norm
            } else {
                -1.0 + 2.0 * val_norm
            };
        }

        if (mod_flags & FLUID_MOD_MAP_MASK) == FLUID_MOD_SWITCH {
            val = if val >= 0.0 { 1.0 } else { -1.0 };
        } else if (mod_flags & FLUID_MOD_MAP_MASK) == FLUID_MOD_CONCAVE {
            // The maximum mapped value should not be 1.0! According to sections 8.4.1 and 8.4.7 it must be 127/128 yet again.
            // Same is assumed for convex below, though this is not explicitly said by the spec.
            val = if val >= 0.0 {
                fmin(
                    fluid_concave(FLUID_VEL_CB_SIZE as FluidReal * val),
                    (range - 1.0) / range,
                )
            } else {
                -fluid_concave(FLUID_VEL_CB_SIZE as FluidReal * -val)
            };
        } else if (mod_flags & FLUID_MOD_MAP_MASK) == FLUID_MOD_CONVEX {
            val = if val >= 0.0 {
                fmin(
                    fluid_convex(FLUID_VEL_CB_SIZE as FluidReal * val),
                    (range - 1.0) / range,
                )
            } else {
                -fluid_convex(FLUID_VEL_CB_SIZE as FluidReal * -val)
            };
        } else {
            // FLUID_MOD_LINEAR
        }
    }

    val
}

/// C's `fmin()` (NaN-aware like C: returns the other argument if one is
/// NaN).
#[inline]
fn fmin(a: f64, b: f64) -> f64 {
    a.min(b)
}

/// Translation of `fluid_mod_get_value()`: computes and return modulator
/// output following SF2.01 (See SoundFont Modulator Controller Model
/// Chapter 9.5).
///
/// Output = Transform(Amount * Map(primary source input) * Map(secondary source input))
///
/// Note:
/// fluid_mod_get_value ignores the Transform operator. The result is:
///
///   Output = Amount * Map(primary source input) * Map(secondary source input)
pub(crate) fn fluid_mod_get_value(
    mod_: &FluidMod,
    voice: &FluidVoice,
    chan: &FluidChannel,
) -> FluidReal {
    /* The wording of the default modulators refers to a range of 127/128.
     * And the table in section 9.5.3 suggests, that this mapping should be applied
     * to all unipolar and bipolar mappings respectively.
     *
     * Thinking about this further, this is actually pretty clever, as this is properly
     * addresses MIDI Recommended Practice (RP-036) Default Pan Formula
     * "Since MIDI controller values range from 0 to 127, the exact center
     * of the range, 63.5, cannot be represented."
     *
     * When changing the overall range to 127/128 however, the "middle pan" value of 64
     * can be correctly represented.
     */
    let mut range1: FluidReal = 128.0;
    let mut range2: FluidReal = 128.0;

    /* 'special treatment' for default controller
     *
     *  Reference: SF2.01 section 8.4.2
     *
     * The GM default controller 'vel-to-filter cut off' is not clearly
     * defined: If implemented according to the specs, the filter
     * frequency jumps between vel=63 and vel=64.  To maintain
     * compatibility with existing sound fonts, the implementation is
     * 'hardcoded', it is impossible to implement using only one
     * modulator otherwise.
     *
     * I assume here, that the 'intention' of the paragraph is one
     * octave (1200 cents) filter frequency shift between vel=127 and
     * vel=64.  'amount' is (-2400), at least as long as the controller
     * is set to default.
     *
     * Further, the 'appearance' of the modulator (source enumerator,
     * destination enumerator, flags etc) is different from that
     * described in section 8.4.2, but it matches the definition used in
     * several SF2.1 sound fonts (where it is used only to turn it off).
     * */
    if fluid_mod_test_identity(mod_, &DEFAULT_VEL2FILTER_MOD) {
        /*
            if (voice->vel < 64){
              return (fluid_real_t) mod->amount / 2.0;
            } else {
              return (fluid_real_t) mod->amount * (127 - voice->vel) / 127;
            }
            return (fluid_real_t) mod->amount / 2.0;
        */
        // S. Christian Collins' mod, to stop forcing velocity based filtering
        return 0.0;
    }

    /* Get the initial value of the first source.
     *
     * Even if the src is FLUID_MOD_NONE, the value has to be transformed, see #1389
     */
    let mut v1 = fluid_mod_get_source_value(mod_.src1, mod_.flags1, &mut range1, voice, chan);

    /* transform the input value */
    v1 = fluid_mod_transform_source_value(mod_, v1, range1, true);

    /* get the second input source */
    let mut v2 = fluid_mod_get_source_value(mod_.src2, mod_.flags2, &mut range2, voice, chan);

    /* transform the second input value */
    v2 = fluid_mod_transform_source_value(mod_, v2, range2, false);

    /* it indeed is as simple as that: */
    let mut final_value = mod_.amount as FluidReal * v1 * v2;

    /* check for absolute value transform */
    if mod_.trans == FLUID_MOD_TRANSFORM_ABS {
        final_value = final_value.abs();
    }
    final_value
}

/// Translation of `new_fluid_mod()`: create a new uninitialized
/// modulator structure.
pub(crate) fn new_fluid_mod() -> FluidMod {
    FluidMod::default()
}

/// Translation of `fluid_mod_is_src1_none()`: checks if modulator with
/// source 1 other than CC is FLUID_MOD_NONE.
fn fluid_mod_is_src1_none(mod_: &FluidMod) -> bool {
    ((mod_.flags1 & FLUID_MOD_CC) == 0) && (mod_.src1 == FLUID_MOD_NONE)
}

/// Translation of `fluid_mod_check_non_cc_source()`: checks if modulators
/// source other than CC source is invalid.
///
/// @param src1_select, source input selection to check.
///   1 to check src1 source.
///   0 to check src2 source.
/// @return FALSE if selected modulator source other than cc is invalid, TRUE otherwise.
///
/// (specs SF 2.01  7.4, 7.8, 8.2.1)
fn fluid_mod_check_non_cc_source(mod_: &FluidMod, src1_select: bool) -> bool {
    let (flags, src) = if src1_select {
        (mod_.flags1, mod_.src1)
    } else {
        (mod_.flags2, mod_.src2)
    };

    ((flags & FLUID_MOD_CC) != 0)  /* src is a CC */
        /* SF2.01 section 8.2.1: Constant value */
        || ((src == FLUID_MOD_NONE)
            || (src == FLUID_MOD_VELOCITY)        /* Note-on velocity */
            || (src == FLUID_MOD_KEY)             /* Note-on key number */
            || (src == FLUID_MOD_KEYPRESSURE)     /* Poly pressure */
            || (src == FLUID_MOD_CHANNELPRESSURE) /* Channel pressure */
            || (src == FLUID_MOD_PITCHWHEEL)      /* Pitch wheel */
            || (src == FLUID_MOD_PITCHWHEELSENS)) /* Pitch wheel sensitivity */
}

/// Translation of `fluid_mod_check_cc_source()`: checks if modulator CC
/// source is invalid (specs SF 2.01  7.4, 7.8, 8.2.1).
///
/// @src1_select, source input selection:
///   1 to check src1 source or
///   0 to check src2 source.
/// @return FALSE if selected modulator's source CC is invalid, TRUE otherwise.
fn fluid_mod_check_cc_source(mod_: &FluidMod, src1_select: bool) -> bool {
    let (flags, src) = if src1_select {
        (mod_.flags1, mod_.src1)
    } else {
        (mod_.flags2, mod_.src2)
    };
    let src = src as i32;

    ((flags & FLUID_MOD_CC) == 0)  /* src is non CC */
        || ((src != BANK_SELECT_MSB)
            && (src != BANK_SELECT_LSB)
            && (src != DATA_ENTRY_MSB)
            && (src != DATA_ENTRY_LSB)
            /* is src not NRPN_LSB, NRPN_MSB, RPN_LSB, RPN_MSB */
            && ((src < NRPN_LSB) || (RPN_MSB < src))
            /* is src not ALL_SOUND_OFF, ALL_CTRL_OFF, LOCAL_CONTROL, ALL_NOTES_OFF ? */
            /* is src not OMNI_OFF, OMNI_ON, POLY_OFF, POLY_ON ? */
            && (src < ALL_SOUND_OFF))
    /* CC lsb shouldn't allowed to modulate (spec SF 2.01 - 8.2.1)
    However, as long fluidsynth will use only CC 7 bits resolution,
    it is safe to ignore these SF recommendations on CC receive.
    See explanations in fluid_synth_cc_LOCAL() */
    /* uncomment next line to forbid CC lsb  */
    /* && ((src < 32) || (63 < src)) */
}

/// Translation of `fluid_mod_check_sources()`: checks valid modulator
/// sources (specs SF 2.01  7.4, 7.8, 8.2.1)
///
/// @param name,if not NULL, pointer on a string displayed as a warning.
/// @return TRUE if modulator sources src1, src2 are valid, FALSE otherwise.
pub(crate) fn fluid_mod_check_sources(mod_: &FluidMod, name: Option<&str>) -> bool {
    /* checks valid non cc sources */
    if !fluid_mod_check_non_cc_source(mod_, true) {
        /* check src1 */
        if let Some(name) = name {
            flog!(
                FLUID_WARN,
                "Invalid modulator, using non-CC source {}.src{}={}",
                name,
                1,
                mod_.src1
            );
        }

        return false;
    }

    /*
      When src1 is non CC source FLUID_MOD_NONE, the modulator is valid, yet it's pretty unusual
      which is why it deserves a warning. However, it's totally legal. A use-case could be a
      constant modulator that is meant to apply some offset to a generator.
    */
    if fluid_mod_is_src1_none(mod_) {
        if let Some(name) = name {
            flog!(
                FLUID_WARN,
                "Modulator with source 1 set to none {}.src1={}",
                name,
                mod_.src1
            );
        }

        return true;
    }

    if !fluid_mod_check_non_cc_source(mod_, false) {
        /* check src2 */
        if let Some(name) = name {
            flog!(
                FLUID_WARN,
                "Invalid modulator, using non-CC source {}.src{}={}",
                name,
                2,
                mod_.src2
            );
        }

        return false;
    }

    /* checks valid cc sources */
    if !fluid_mod_check_cc_source(mod_, true) {
        /* check src1 */
        if let Some(name) = name {
            flog!(
                FLUID_WARN,
                "Invalid modulator, using CC source {}.src{}={}",
                name,
                1,
                mod_.src1
            );
        }

        return false;
    }

    if !fluid_mod_check_cc_source(mod_, false) {
        /* check src2 */
        if let Some(name) = name {
            flog!(
                FLUID_WARN,
                "Invalid modulator, using CC source {}.src{}={}",
                name,
                2,
                mod_.src2
            );
        }

        return false;
    }

    true
}

/// Translation of `fluid_mod_test_identity()`: checks if two modulators
/// are identical in sources, flags and destination.
///
/// SF2.01 section 9.5.1 page 69, 'bullet' 3 defines 'identical'.
pub(crate) fn fluid_mod_test_identity(mod1: &FluidMod, mod2: &FluidMod) -> bool {
    mod1.dest == mod2.dest
        && mod1.src1 == mod2.src1
        && mod1.src2 == mod2.src2
        && mod1.flags1 == mod2.flags1
        && mod1.flags2 == mod2.flags2
}

/// Translation of `fluid_mod_has_source()`: check if the modulator has the
/// given source.
///
/// @param cc Boolean value indicating if ctrl is a CC controller or not
/// @param ctrl The source to check for (if \c cc == FALSE : a value of type #fluid_mod_src, else the value of the MIDI CC to check for)
pub(crate) fn fluid_mod_has_source(mod_: &FluidMod, cc: bool, ctrl: i32) -> bool {
    ((mod_.src1 as i32 == ctrl) && ((mod_.flags1 & FLUID_MOD_CC) != 0) && cc)
        || ((mod_.src1 as i32 == ctrl) && ((mod_.flags1 & FLUID_MOD_CC) == 0) && !cc)
        || (((mod_.src2 as i32 == ctrl) && ((mod_.flags2 & FLUID_MOD_CC) != 0) && cc)
            || ((mod_.src2 as i32 == ctrl) && ((mod_.flags2 & FLUID_MOD_CC) == 0) && !cc))
}

/// Translation of `fluid_mod_has_dest()`: check if the modulator has the
/// given destination.
pub(crate) fn fluid_mod_has_dest(mod_: &FluidMod, gen: i32) -> bool {
    mod_.dest as i32 == gen
}
