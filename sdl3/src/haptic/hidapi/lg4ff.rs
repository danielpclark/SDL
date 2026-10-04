// Rust translation of src/haptic/hidapi/SDL_hidapihaptic_lg4ff.c from Simple
// DirectMedia Layer.
// Copyright (C) 2025 Simon Wood <simon@mungewell.org>
// Copyright (C) 2025 Michal Malý <madcatxster@devoid-pointer.net>
// Copyright (C) 2025 Bernat Arlandis <berarma@hotmail.com>
// Copyright (C) 2025 Katharine Chui <katharine.chui@gmail.com>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Logitech wheel haptic driver (G29, G27, G25, Driving Force GT,
//! Driving Force Pro and Driving Force EX): it renders the effects itself
//! in a thread, every 2 ms, into the wheel's four force slots (a constant
//! force, a spring, a damper and friction), sending the slot commands that
//! changed through the joystick of the HIDAPI Logitech wheel driver
//! ([`crate::joystick::hidapi::lg4ff`]).

use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use super::{HapticJoystick, HidapiHapticContext, HidapiHapticDriver};
use crate::error::{Error, Result};
use crate::haptic::{
    HapticCondition, HapticConditionKind, HapticConstant, HapticDirection, HapticDirectionType,
    HapticEffect, HapticEffectID, HapticFeatures, HapticPeriodic, HapticRamp, HapticWaveform,
    HAPTIC_INFINITY,
};
use crate::joystick::hidapi::lg4ff::{
    autocenter_commands, ffex_autocenter_command, get_env_int, is_ffex, is_supported_product,
    Command,
};
use crate::joystick::usb_ids::USB_VENDOR_LOGITECH;
use crate::thread::Thread;

const LG4FF_MAX_EFFECTS: usize = 16;

const FF_EFFECT_STARTED: u32 = 0;
const FF_EFFECT_ALLSET: u32 = 1;
const FF_EFFECT_PLAYING: u32 = 2;
const FF_EFFECT_UPDATING: u32 = 3;

/// The clock of the effects, in milliseconds (`get_time_ms()`: the ticks;
/// the tests' own).
type Clock = fn() -> u64;

/// The state of an effect. Translation of `struct lg4ff_effect_state`.
#[derive(Clone, Debug, Default)]
struct EffectState {
    /// The effect uploaded (`None` before the first upload).
    effect: Option<HapticEffect>,
    start_at: u64,
    play_at: u64,
    stop_at: u64,
    flags: u32,
    time_playing: u64,
    updated_at: u64,
    phase: u32,
    phase_adj: u32,
    count: u32,

    direction_gain: f64,
    slope: i32,

    allocated: bool,
}

impl EffectState {
    /// `test_bit(bit, &state->flags)`
    fn test(&self, bit: u32) -> bool {
        self.flags & (1 << bit) != 0
    }
    /// `__set_bit(bit, &state->flags)`
    fn set(&mut self, bit: u32) {
        self.flags |= 1 << bit;
    }
    /// `__clear_bit(bit, &state->flags)`
    fn clear(&mut self, bit: u32) {
        self.flags &= !(1 << bit);
    }
    /// `STOP_EFFECT(state)`
    fn stop(&mut self) {
        self.flags = 0;
    }
}

/// What a slot plays. Translation of `struct lg4ff_effect_parameters`.
#[derive(Clone, Copy, Debug, Default)]
struct EffectParameters {
    level: i32,
    d1: i32,
    d2: i32,
    k1: i32,
    k2: i32,
    clip: u32,
}

/// The effect type of a slot (its `effect_type`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SlotType {
    Constant,
    Spring,
    Damper,
    Friction,
}

/// A force slot of the wheel. Translation of `struct lg4ff_slot` (its
/// `parameters` are never used).
#[derive(Clone, Copy, Debug)]
struct Slot {
    id: i32,
    current_cmd: Command,
    cmd_op: u32,
    is_updated: bool,
    effect_type: SlotType,
}

impl Slot {
    fn new(id: i32, effect_type: SlotType) -> Slot {
        Slot {
            id,
            current_cmd: [0; 7],
            cmd_op: 0,
            is_updated: false,
            effect_type,
        }
    }
}

/// The slots of `lg4ff_init_slots()`, in order.
const SLOT_TYPES: [SlotType; 4] = [
    SlotType::Constant,
    SlotType::Spring,
    SlotType::Damper,
    SlotType::Friction,
];

/// The effects and slots of a wheel: the part of `struct lg4ff_device`
/// its `mutex` guards.
#[derive(Debug)]
struct Lg4ffDevice {
    states: [EffectState; LG4FF_MAX_EFFECTS],
    slots: [Slot; 4],
    effects_used: i32,

    gain: i32,

    spring_level: i32,
    damper_level: i32,
    friction_level: i32,

    peak_ffb_level: i32,

    is_ffex: bool,
}

/// `CLAMP_VALUE_U16(x)`
fn clamp_value_u16(x: i64) -> u16 {
    if x > 0xffff {
        0xffff
    } else {
        x as u16
    }
}

/// `SCALE_VALUE_U16(x, bits)`
fn scale_value_u16(x: i64, bits: u32) -> i32 {
    i32::from(clamp_value_u16(x) >> (16 - bits))
}

/// `CLAMP_VALUE_S16(x)`
fn clamp_value_s16(x: i32) -> u16 {
    x.clamp(-0x8000, 0x7fff) as u16
}

/// `TRANSLATE_FORCE(x)` (truncated to the command byte it goes to).
fn translate_force(x: i32) -> u8 {
    ((u32::from(clamp_value_s16(x)) + 0x8000) >> 8) as u8
}

/// `SCALE_COEFF(x, bits)`
fn scale_coeff(x: i32, bits: u32) -> i32 {
    scale_value_u16(i64::from(x.wrapping_abs().wrapping_mul(2)), bits)
}

/// `sin_deg(in)`
fn sin_deg(degrees: f64) -> f64 {
    crate::stdlib::math::sin(degrees * std::f64::consts::PI / 180.0)
}

/// Translation of `effect_is_periodic()`.
fn effect_is_periodic(effect: &HapticEffect) -> bool {
    matches!(effect, HapticEffect::Periodic(_))
}

/// Translation of `effect_is_condition()`.
fn effect_is_condition(effect: &HapticEffect) -> bool {
    matches!(
        effect,
        HapticEffect::Condition(HapticCondition {
            kind: HapticConditionKind::Spring
                | HapticConditionKind::Damper
                | HapticConditionKind::Friction,
            ..
        })
    )
}

/// Translation of `to_linux_direction()`, linux SDL_syshaptic.c
/// `SDL_SYS_ToDirection` (with the steering axis).
fn to_linux_direction(src: &HapticDirection) -> u16 {
    match src.kind {
        HapticDirectionType::Polar => {
            let tmp = ((src.dir[0] % 36000) * 0x8000) / 18000; // convert to range [0,0xFFFF]
            tmp as u16
        }

        HapticDirectionType::Spherical => {
            /*
                We convert to polar, because that's the only supported direction on Linux.
                The first value of a spherical direction is practically the same as a
                Polar direction, except that we have to add 90 degrees. It is the angle
                from EAST {1,0} towards SOUTH {0,1}.
                --> add 9000
                --> finally convert to [0,0xFFFF] as in case SDL_HAPTIC_POLAR.
            */
            let mut tmp = (src.dir[0].wrapping_add(9000) % 36000) as u32; // Convert to polars
            tmp = tmp.wrapping_mul(0x8000) / 18000; // convert to range [0,0xFFFF]
            tmp as u16
        }

        HapticDirectionType::Cartesian => {
            if src.dir[1] == 0 {
                if src.dir[0] >= 0 {
                    0x4000
                } else {
                    0xC000
                }
            } else if src.dir[0] == 0 {
                if src.dir[1] >= 0 {
                    0x8000
                } else {
                    0
                }
            } else {
                // Ideally we'd use fixed point math instead of floats...
                let f =
                    crate::stdlib::math::atan2(f64::from(src.dir[1]), f64::from(src.dir[0])) as f32;
                /*
                    SDL_atan2 takes the parameters: Y-axis-value and X-axis-value (in that order)
                    - Y-axis-value is the second coordinate (from center to SOUTH)
                    - X-axis-value is the first coordinate (from center to EAST)
                        We add 36000, because SDL_atan2 also returns negative values. Then we practically
                        have the first spherical value. Therefore we proceed as in case
                        SDL_HAPTIC_SPHERICAL and add another 9000 to get the polar value.
                    --> add 45000 in total
                    --> finally convert to [0,0xFFFF] as in case SDL_HAPTIC_POLAR.
                */
                let mut tmp = (((f64::from(f) * 18000.0 / std::f64::consts::PI) as i32 + 45000)
                    % 36000) as u32;
                tmp = tmp.wrapping_mul(0x8000) / 18000; // convert to range [0,0xFFFF]
                tmp as u16
            }
        }
        HapticDirectionType::SteeringAxis => 0x4000,
    }
}

/// Translation of `get_effect_direction()`.
fn get_effect_direction(effect: &HapticEffect) -> u16 {
    match effect {
        HapticEffect::Periodic(p) => to_linux_direction(&p.direction),
        HapticEffect::Condition(c) if effect_is_condition(effect) => {
            to_linux_direction(&c.direction)
        }
        HapticEffect::Constant(c) => to_linux_direction(&c.direction),
        HapticEffect::Ramp(r) => to_linux_direction(&r.direction),
        _ => {
            crate::sdl_assert!(false);
            0
        }
    }
}

/// Translation of `get_effect_replay_length()`: 0 for an infinite effect.
fn get_effect_replay_length(effect: &HapticEffect) -> u32 {
    let length = match effect {
        HapticEffect::Periodic(p) => p.length,
        HapticEffect::Condition(c) if effect_is_condition(effect) => c.length,
        HapticEffect::Constant(c) => c.length,
        HapticEffect::Ramp(r) => r.length,
        _ => {
            crate::sdl_assert!(false);
            0
        }
    };

    if length == HAPTIC_INFINITY {
        0
    } else {
        length
    }
}

/// Translation of `get_effect_replay_delay()`.
fn get_effect_replay_delay(effect: &HapticEffect) -> u16 {
    match effect {
        HapticEffect::Periodic(p) => p.delay,
        HapticEffect::Condition(c) if effect_is_condition(effect) => c.delay,
        HapticEffect::Constant(c) => c.delay,
        HapticEffect::Ramp(r) => r.delay,
        _ => {
            crate::sdl_assert!(false);
            0
        }
    }
}

/// The level of an envelope's attack, `level_sign * attack_level + d *
/// time_playing / attack_length`.
///
/// FIXME (upstream): `time_playing` is unsigned 64-bit (as in new-lg4ff,
/// on 64-bit kernels), so `d * time_playing` is too: when the attack starts
/// above the effect's level (`d` negative), the level is garbage.
fn attack_level(
    level_sign: i32,
    attack_level: u16,
    d: i32,
    time_playing: u64,
    attack_length: u16,
) -> i32 {
    let base = i64::from(level_sign * i32::from(attack_level)) as u64;
    let ramp = (i64::from(d) as u64).wrapping_mul(time_playing) / u64::from(attack_length);
    base.wrapping_add(ramp) as i32
}

/// Translation of `lg4ff_calculate_constant()`.
///
/// *Ported*
/// Original function by:
/// Bernat Arlandis <berarma@hotmail.com>
/// `git blame 1a2d5727876dd7befce23d9695924e9446b31c4b hid-lg4ff.c`, <https://github.com/berarma/new-lg4ff.git>
fn calculate_constant(state: &EffectState, constant: &HapticConstant) -> i32 {
    let mut level = i32::from(constant.level);

    if state.time_playing < u64::from(constant.attack_length) {
        let level_sign = if level < 0 { -1 } else { 1 };
        let d = level - level_sign * i32::from(constant.attack_level);
        level = attack_level(
            level_sign,
            constant.attack_level,
            d,
            state.time_playing,
            constant.attack_length,
        );
    } else if constant.length != 0 && constant.fade_length != 0 {
        let t = state
            .time_playing
            .wrapping_sub(u64::from(constant.length))
            .wrapping_add(u64::from(constant.fade_length)) as i32;
        if t > 0 {
            let level_sign = if level < 0 { -1 } else { 1 };
            let d = level - level_sign * i32::from(constant.fade_level);
            level = level.wrapping_sub(d.wrapping_mul(t) / i32::from(constant.fade_length));
        }
    }

    (state.direction_gain * f64::from(level)) as i32
}

/// Translation of `lg4ff_calculate_ramp()`.
///
/// *Ported*
/// Original function by:
/// Bernat Arlandis <berarma@hotmail.com>
/// `git blame 1a2d5727876dd7befce23d9695924e9446b31c4b hid-lg4ff.c`, <https://github.com/berarma/new-lg4ff.git>
fn calculate_ramp(state: &EffectState, ramp: &HapticRamp) -> i32 {
    let level = if state.time_playing < u64::from(ramp.attack_length) {
        let start = i32::from(ramp.start);
        let level_sign = if start < 0 { -1 } else { 1 };
        let t = u64::from(ramp.attack_length).wrapping_sub(state.time_playing) as i32;
        let d = start - level_sign * i32::from(ramp.attack_level);
        level_sign * i32::from(ramp.attack_level)
            + d.wrapping_mul(t) / i32::from(ramp.attack_length)
    } else if ramp.length != 0
        && state.time_playing >= u64::from(ramp.length.wrapping_sub(u32::from(ramp.fade_length)))
        && ramp.fade_length != 0
    {
        let end = i32::from(ramp.end);
        let level_sign = if end < 0 { -1 } else { 1 };
        let t = state
            .time_playing
            .wrapping_sub(u64::from(ramp.length))
            .wrapping_add(u64::from(ramp.fade_length)) as i32;
        let d = level_sign * i32::from(ramp.fade_level) - end;
        end.wrapping_sub(d.wrapping_mul(t) / i32::from(ramp.fade_length))
    } else {
        let t = state
            .time_playing
            .wrapping_sub(u64::from(ramp.attack_length)) as i32;
        i32::from(ramp.start).wrapping_add(t.wrapping_mul(state.slope) >> 16)
    };

    (state.direction_gain * f64::from(level)) as i32
}

/// Translation of `lg4ff_calculate_periodic()`.
///
/// *Ported*
/// Original function by:
/// Bernat Arlandis <berarma@hotmail.com>
/// `git blame 1a2d5727876dd7befce23d9695924e9446b31c4b hid-lg4ff.c`, <https://github.com/berarma/new-lg4ff.git>
fn calculate_periodic(
    state: &EffectState,
    periodic: &HapticPeriodic,
    effect: &HapticEffect,
) -> i32 {
    let mut magnitude = i32::from(periodic.magnitude);
    let magnitude_sign = if magnitude < 0 { -1 } else { 1 };
    let mut level = i32::from(periodic.offset);

    if state.time_playing < u64::from(periodic.attack_length) {
        let d = magnitude - magnitude_sign * i32::from(periodic.attack_level);
        magnitude = attack_level(
            magnitude_sign,
            periodic.attack_level,
            d,
            state.time_playing,
            periodic.attack_length,
        );
    } else if periodic.length != 0 && periodic.fade_length != 0 {
        let t = state
            .time_playing
            .wrapping_sub(u64::from(get_effect_replay_length(effect)))
            .wrapping_add(u64::from(periodic.fade_length)) as i32;
        if t > 0 {
            let d = magnitude.wrapping_sub(magnitude_sign * i32::from(periodic.fade_level));
            magnitude = magnitude.wrapping_sub(d.wrapping_mul(t) / i32::from(periodic.fade_length));
        }
    }

    // FIXME (upstream): the phase is unsigned (as in new-lg4ff), so the
    // sawtooth waves are computed unsigned: they are garbage for a negative
    // magnitude.
    let phase = state.phase;
    let level_change = match periodic.waveform {
        HapticWaveform::Sine => (sin_deg(f64::from(phase)) * f64::from(magnitude)) as i32,
        HapticWaveform::Square => (if phase < 180 { 1 } else { -1i32 }).wrapping_mul(magnitude),
        HapticWaveform::Triangle => {
            let magnitude = i64::from(magnitude);
            ((i64::from(phase) * magnitude * 2 / 360 - magnitude).wrapping_abs() * 2 - magnitude)
                as i32
        }
        HapticWaveform::SawtoothUp => (phase.wrapping_mul(magnitude as u32).wrapping_mul(2) / 360)
            .wrapping_sub(magnitude as u32) as i32,
        HapticWaveform::SawtoothDown => (magnitude as u32)
            .wrapping_sub(phase.wrapping_mul(magnitude as u32).wrapping_mul(2) / 360)
            as i32,
    };
    level = level.wrapping_add(level_change);

    (state.direction_gain * f64::from(level)) as i32
}

/// Translation of `lg4ff_calculate_spring()`.
///
/// *Ported*
/// Original function by:
/// Bernat Arlandis <berarma@hotmail.com>
/// `git blame 1a2d5727876dd7befce23d9695924e9446b31c4b hid-lg4ff.c`, <https://github.com/berarma/new-lg4ff.git>
fn calculate_spring(condition: &HapticCondition, parameters: &mut EffectParameters) {
    parameters.d1 = i32::from(condition.center[0]) - i32::from(condition.deadband[0]) / 2;
    parameters.d2 = i32::from(condition.center[0]) + i32::from(condition.deadband[0]) / 2;
    parameters.k1 = i32::from(condition.left_coeff[0]);
    parameters.k2 = i32::from(condition.right_coeff[0]);
    parameters.clip = u32::from(condition.right_sat[0]);
}

/// Translation of `lg4ff_calculate_resistance()`.
///
/// *Ported*
/// Original function by:
/// Bernat Arlandis <berarma@hotmail.com>
/// `git blame 1a2d5727876dd7befce23d9695924e9446b31c4b hid-lg4ff.c`, <https://github.com/berarma/new-lg4ff.git>
fn calculate_resistance(condition: &HapticCondition, parameters: &mut EffectParameters) {
    parameters.k1 = i32::from(condition.left_coeff[0]);
    parameters.k2 = i32::from(condition.right_coeff[0]);
    parameters.clip = u32::from(condition.right_sat[0]);
}

/// Translation of `lg4ff_update_state()`.
///
/// *Ported*
/// Original function by:
/// Bernat Arlandis <berarma@hotmail.com>
/// `git blame 1a2d5727876dd7befce23d9695924e9446b31c4b hid-lg4ff.c`, <https://github.com/berarma/new-lg4ff.git>
fn update_state(state: &mut EffectState, effect: &HapticEffect, now: u64) {
    let effect_direction = get_effect_direction(effect);
    let direction_gain = || sin_deg(f64::from(i32::from(effect_direction) * 360 / 0x10000));
    let length = get_effect_replay_length(effect);
    let delay = u64::from(get_effect_replay_delay(effect));

    if !state.test(FF_EFFECT_ALLSET) {
        state.play_at = state.start_at.wrapping_add(delay);
        if !state.test(FF_EFFECT_UPDATING) {
            state.updated_at = state.play_at;
        }
        state.direction_gain = direction_gain();
        if let HapticEffect::Periodic(p) = effect {
            state.phase_adj = (i32::from(p.phase) * 360 / i32::from(p.period)) as u32;
        }
        if length != 0 {
            state.stop_at = state.play_at.wrapping_add(u64::from(length));
        }
    }
    state.set(FF_EFFECT_ALLSET);

    if state.test(FF_EFFECT_UPDATING) {
        state.clear(FF_EFFECT_PLAYING);
        state.play_at = state.updated_at.wrapping_add(delay);
        state.direction_gain = direction_gain();
        if length != 0 {
            state.stop_at = state.updated_at.wrapping_add(u64::from(length));
        }
        if effect_is_periodic(effect) {
            state.phase_adj = state.phase;
        }
    }
    state.clear(FF_EFFECT_UPDATING);

    state.slope = 0;
    if let HapticEffect::Ramp(ramp) = effect {
        // FIXME (upstream): the length is unsigned 32-bit here (16-bit in
        // the kernel), so the division is unsigned: a ramp whose end is
        // below its start gets a garbage slope.
        let duration = ramp
            .length
            .wrapping_sub(u32::from(ramp.attack_length))
            .wrapping_sub(u32::from(ramp.fade_length));
        if ramp.length != 0 && duration != 0 {
            let rise = (i32::from(ramp.end) - i32::from(ramp.start)) << 16;
            state.slope = ((rise as u32) / duration) as i32;
        }
    }

    if !state.test(FF_EFFECT_PLAYING)
        && now >= state.play_at
        && (length == 0 || now < state.stop_at)
    {
        state.set(FF_EFFECT_PLAYING);
    }

    if state.test(FF_EFFECT_PLAYING) {
        state.time_playing = now.wrapping_sub(state.play_at);
        if let HapticEffect::Periodic(p) = effect {
            let period = u64::from(p.period);
            let phase_time = now.wrapping_sub(state.updated_at);
            state.phase = ((phase_time % period) * 360 / period) as u32;
            state.phase = state.phase.wrapping_add(state.phase_adj % 360);
        }
    }
}

/// Translation of `lg4ff_update_slot()`.
///
/// *Ported*
/// Original function by:
/// Bernat Arlandis <berarma@hotmail.com>
/// `git blame 1a2d5727876dd7befce23d9695924e9446b31c4b hid-lg4ff.c`, <https://github.com/berarma/new-lg4ff.git>
fn update_slot(slot: &mut Slot, parameters: &EffectParameters) {
    let mut original_cmd = slot.current_cmd;

    if (original_cmd[0] & 0xf) == 1 {
        original_cmd[0] = (original_cmd[0] & 0xf0) + 0xc;
    }

    if slot.effect_type == SlotType::Constant {
        if slot.cmd_op == 0 {
            slot.cmd_op = 1;
        } else {
            slot.cmd_op = 0xc;
        }
    } else if parameters.clip == 0 {
        slot.cmd_op = 3;
    } else if slot.cmd_op == 3 {
        slot.cmd_op = 1;
    } else {
        slot.cmd_op = 0xc;
    }

    slot.current_cmd[0] = ((0x10 << slot.id) + slot.cmd_op) as u8;

    if slot.cmd_op == 3 {
        slot.current_cmd[1..].fill(0);
    } else {
        let s1 = i32::from(parameters.k1 < 0);
        let s2 = i32::from(parameters.k2 < 0);
        let clip = scale_value_u16(i64::from(parameters.clip), 8) as u8;
        match slot.effect_type {
            SlotType::Constant => {
                slot.current_cmd[1..].fill(0);
                slot.current_cmd[2 + slot.id as usize] = translate_force(parameters.level);
            }
            SlotType::Spring => {
                let mut d1 = scale_value_u16(i64::from((parameters.d1 + 0x8000) & 0xffff), 11);
                let mut d2 = scale_value_u16(i64::from((parameters.d2 + 0x8000) & 0xffff), 11);
                let mut k1 = parameters.k1.wrapping_abs();
                let mut k2 = parameters.k2.wrapping_abs();
                if k1 < 2048 {
                    d1 = 0;
                } else {
                    k1 -= 2048;
                }
                if k2 < 2048 {
                    d2 = 2047;
                } else {
                    k2 -= 2048;
                }
                slot.current_cmd[1] = 0x0b;
                slot.current_cmd[2] = (d1 >> 3) as u8;
                slot.current_cmd[3] = (d2 >> 3) as u8;
                slot.current_cmd[4] = ((scale_coeff(k2, 4) << 4) + scale_coeff(k1, 4)) as u8;
                slot.current_cmd[5] = (((d2 & 7) << 5) + ((d1 & 7) << 1) + (s2 << 4) + s1) as u8;
                slot.current_cmd[6] = clip;
            }
            SlotType::Damper => {
                slot.current_cmd[1] = 0x0c;
                slot.current_cmd[2] = scale_coeff(parameters.k1, 4) as u8;
                slot.current_cmd[3] = s1 as u8;
                slot.current_cmd[4] = scale_coeff(parameters.k2, 4) as u8;
                slot.current_cmd[5] = s2 as u8;
                slot.current_cmd[6] = clip;
            }
            SlotType::Friction => {
                slot.current_cmd[1] = 0x0e;
                slot.current_cmd[2] = scale_coeff(parameters.k1, 8) as u8;
                slot.current_cmd[3] = scale_coeff(parameters.k2, 8) as u8;
                slot.current_cmd[4] = clip;
                slot.current_cmd[5] = ((s2 << 4) + s1) as u8;
                slot.current_cmd[6] = 0;
            }
        }
    }

    if original_cmd != slot.current_cmd {
        slot.is_updated = true;
    }
}

impl Lg4ffDevice {
    fn new(gain: i32, spring_level: i32, damper_level: i32, friction_level: i32) -> Lg4ffDevice {
        Lg4ffDevice {
            states: Default::default(),
            slots: std::array::from_fn(|i| Slot::new(i as i32, SLOT_TYPES[i])),
            effects_used: 0,
            gain,
            spring_level,
            damper_level,
            friction_level,
            peak_ffb_level: 0,
            is_ffex: false,
        }
    }

    /// Translation of `lg4ff_play_effect()`.
    ///
    /// *Ported*
    /// Original function by:
    /// Bernat Arlandis <berarma@hotmail.com>
    /// `git blame 1a2d5727876dd7befce23d9695924e9446b31c4b hid-lg4ff.c`, <https://github.com/berarma/new-lg4ff.git>
    fn play_effect(&mut self, effect_id: usize, value: i32, now: u64) {
        let state = &mut self.states[effect_id];

        if value > 0 {
            if state.test(FF_EFFECT_STARTED) {
                state.stop();
            } else {
                self.effects_used += 1;
            }
            state.set(FF_EFFECT_STARTED);
            state.start_at = now;
            state.count = value as u32;
        } else if state.test(FF_EFFECT_STARTED) {
            state.stop();
            self.effects_used -= 1;
        }
    }

    /// Translation of `lg4ff_upload_effect()`: `false` for bad
    /// parameters.
    ///
    /// *Ported*
    /// Original function by:
    /// Bernat Arlandis <berarma@hotmail.com>
    /// `git blame 1a2d5727876dd7befce23d9695924e9446b31c4b hid-lg4ff.c`, <https://github.com/berarma/new-lg4ff.git>
    fn upload_effect(&mut self, effect: &HapticEffect, id: usize, now: u64) -> bool {
        if matches!(effect, HapticEffect::Periodic(p) if p.period == 0) {
            return false;
        }

        let state = &mut self.states[id];

        if state.test(FF_EFFECT_STARTED)
            && state.effect.as_ref().map(HapticEffect::effect_type) != Some(effect.effect_type())
        {
            return false;
        }

        state.effect = Some(effect.clone());

        if state.test(FF_EFFECT_STARTED) {
            state.set(FF_EFFECT_UPDATING);
            state.updated_at = now;
        }

        true
    }

    /// Translation of `lg4ff_init_slots()`.
    ///
    /// *Ported*
    /// Original function by:
    /// Bernat Arlandis <berarma@hotmail.com>
    /// `git blame 1a2d5727876dd7befce23d9695924e9446b31c4b hid-lg4ff.c`, <https://github.com/berarma/new-lg4ff.git>
    fn init_slots(&mut self, joystick: &dyn HapticJoystick) -> Result<()> {
        let parameters = EffectParameters::default();

        // Set/unset fixed loop mode
        //cmd[1] = fixed_loop ? 1 : 0;
        let cmd: Command = [0x0d, 0, 0, 0, 0, 0, 0];
        joystick.send_effect(&cmd)?;

        self.states = Default::default();
        self.slots = std::array::from_fn(|i| Slot::new(i as i32, SLOT_TYPES[i]));

        for slot in &mut self.slots {
            update_slot(slot, &parameters);
            // FIXME (upstream): this sends the fixed loop command again,
            // where new-lg4ff sends the slot's `current_cmd`.
            joystick.send_effect(&cmd)?;
            slot.is_updated = false;
        }

        Ok(())
    }

    /// Render the effects at `now` and send the slots that changed; -1 if
    /// a command couldn't be sent. Translation of `lg4ff_timer()`.
    ///
    /// *Ported*
    /// Original function by:
    /// Bernat Arlandis <berarma@hotmail.com>
    /// `git blame 1a2d5727876dd7befce23d9695924e9446b31c4b hid-lg4ff.c`, <https://github.com/berarma/new-lg4ff.git>
    fn timer(&mut self, now: u64, app_gain: i32, joystick: &dyn HapticJoystick) -> i32 {
        let mut parameters = [EffectParameters::default(); 4];
        let mut status = 0;

        // XXX how to detect stacked up effects here?

        let gain = ((self.gain as u32).wrapping_mul(app_gain as u32) / 0xffff) as u16;

        let mut count = self.effects_used;

        for state in &mut self.states {
            if count == 0 {
                break;
            }

            if !state.test(FF_EFFECT_STARTED) {
                continue;
            }

            count -= 1;

            // (a started effect was uploaded)
            let Some(effect) = state.effect.clone() else {
                continue;
            };

            if state.test(FF_EFFECT_ALLSET)
                && get_effect_replay_length(&effect) != 0
                && now >= state.stop_at
            {
                state.stop();
                state.count = state.count.wrapping_sub(1);
                if state.count == 0 {
                    self.effects_used -= 1;
                    continue;
                }
                state.set(FF_EFFECT_STARTED);
                state.start_at = state.stop_at;
            }

            update_state(state, &effect, now);

            if !state.test(FF_EFFECT_PLAYING) {
                continue;
            }

            match &effect {
                HapticEffect::Periodic(periodic) => {
                    parameters[0].level = parameters[0]
                        .level
                        .wrapping_add(calculate_periodic(state, periodic, &effect));
                }
                HapticEffect::Constant(constant) => {
                    parameters[0].level = parameters[0]
                        .level
                        .wrapping_add(calculate_constant(state, constant));
                }
                HapticEffect::Ramp(ramp) => {
                    parameters[0].level = parameters[0]
                        .level
                        .wrapping_add(calculate_ramp(state, ramp));
                }
                HapticEffect::Condition(condition) => match condition.kind {
                    HapticConditionKind::Spring => calculate_spring(condition, &mut parameters[1]),
                    HapticConditionKind::Damper => {
                        calculate_resistance(condition, &mut parameters[2])
                    }
                    HapticConditionKind::Friction => {
                        calculate_resistance(condition, &mut parameters[3])
                    }
                    HapticConditionKind::Inertia => {}
                },
                _ => {}
            }
        }

        let gain64 = i64::from(gain);
        parameters[0].level = (i64::from(parameters[0].level) * gain64 / 0xffff) as i32;
        parameters[1].clip = parameters[1].clip.wrapping_mul(self.spring_level as u32) / 100;
        parameters[2].clip = parameters[2].clip.wrapping_mul(self.damper_level as u32) / 100;
        parameters[3].clip = parameters[3].clip.wrapping_mul(self.friction_level as u32) / 100;

        let mut ffb_level = parameters[0].level.wrapping_abs();
        for p in &mut parameters[1..] {
            p.k1 = (i64::from(p.k1) * gain64 / 0xffff) as i32;
            p.k2 = (i64::from(p.k2) * gain64 / 0xffff) as i32;
            p.clip = p.clip.wrapping_mul(u32::from(gain)) / 0xffff;
            ffb_level =
                (ffb_level as u32).wrapping_add(p.clip.wrapping_mul(0x7fff) / 0xffff) as i32;
        }
        if ffb_level > self.peak_ffb_level {
            self.peak_ffb_level = ffb_level;
        }

        for (slot, parameters) in self.slots.iter_mut().zip(&parameters) {
            update_slot(slot, parameters);
            if slot.is_updated {
                if joystick.send_effect(&slot.current_cmd).is_err() {
                    status = -1;
                }
                slot.is_updated = false;
            }
        }

        status
    }

    /// Translation of `lg4ff_effect_slot_valid_active()`: the index of an
    /// allocated effect.
    fn valid_active(&self, id: HapticEffectID) -> Option<usize> {
        let index = usize::try_from(id)
            .ok()
            .filter(|&i| i < LG4FF_MAX_EFFECTS)?;
        self.states[index].allocated.then_some(index)
    }
}

/// What the effect thread shares with the driver functions.
struct Shared {
    /// The effects (`mutex` and what it guards).
    device: Mutex<Lg4ffDevice>,
    /// Translation of `app_gain`.
    ///
    /// Note (upstream): upstream sets it without the mutex while the
    /// thread reads it (a data race); an atomic here.
    app_gain: AtomicI32,
    /// Translation of `stop_thread`.
    ///
    /// Note (upstream): a plain `bool` upstream, which the thread may never
    /// see change; an atomic here.
    stop_thread: AtomicBool,
    /// The joystick (`hid_handle`).
    joystick: Arc<dyn HapticJoystick>,
    clock: Clock,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Lg4ffDevice> {
        self.device.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// `lg4ff_timer()` at `now`.
    fn timer(&self, now: u64) -> i32 {
        let app_gain = self.app_gain.load(Ordering::Relaxed);
        self.lock().timer(now, app_gain, &*self.joystick)
    }
}

/// Translation of `SDL_HIDAPI_HapticDriverLg4ff_ThreadFunction()`.
fn thread_function(ctx: &Shared) -> i32 {
    loop {
        if ctx.stop_thread.load(Ordering::Acquire) {
            return 0;
        }
        ctx.timer((ctx.clock)());
        crate::timer::delay(Duration::from_millis(2));
    }
}

/// An open wheel. Translation of `lg4ff_device`, the driver context.
pub(crate) struct Lg4ffHaptic {
    shared: Arc<Shared>,
    /// The effect thread (`thread`); `None` if it couldn't be created
    /// (upstream doesn't check), or in the tests, which run the timer
    /// themselves.
    thread: Option<Thread>,
}

/// The Logitech wheel haptic driver. Translation of
/// `SDL_HIDAPI_HapticDriverLg4ff`.
pub(crate) struct Lg4ffHapticDriver;

pub(crate) static LG4FF_HAPTIC_DRIVER: Lg4ffHapticDriver = Lg4ffHapticDriver;

/// Translation of `SDL_HIDAPI_HapticDriverLg4ff_JoystickSupported()`.
fn joystick_supported(joystick: &dyn HapticJoystick) -> bool {
    joystick.vendor() == USB_VENDOR_LOGITECH && is_supported_product(joystick.product())
}

impl HidapiHapticDriver for Lg4ffHapticDriver {
    fn joystick_supported(&self, joystick: &dyn HapticJoystick) -> bool {
        joystick_supported(joystick)
    }

    /// Translation of `SDL_HIDAPI_HapticDriverLg4ff_Open()`.
    fn open(&self, joystick: Arc<dyn HapticJoystick>) -> Result<Box<dyn HidapiHapticContext>> {
        Ok(Box::new(Lg4ffHaptic::open(
            joystick,
            crate::timer::ticks_ms,
            true,
        )?))
    }
}

impl Lg4ffHaptic {
    /// Translation of `SDL_HIDAPI_HapticDriverLg4ff_Open()`, with the
    /// effects' clock, and whether to start the effect thread.
    ///
    /// ffex identification method by:
    /// Simon Wood <simon@mungewell.org>
    /// Michal Malý <madcatxster@devoid-pointer.net> <madcatxster@gmail.com>
    /// lg4ff_init
    /// `git blame v6.12 drivers/hid/hid-lg4ff.c`, <https://github.com/torvalds/linux.git>
    fn open(
        joystick: Arc<dyn HapticJoystick>,
        clock: Clock,
        start_thread: bool,
    ) -> Result<Lg4ffHaptic> {
        if !joystick_supported(&*joystick) {
            return Err(Error::new(
                "Device not supported by the lg4ff hidapi haptic driver",
            ));
        }

        let mut device = Lg4ffDevice::new(0, 0, 0, 0);
        if device.init_slots(&*joystick).is_err() {
            return Err(Error::new(
                "lg4ff hidapi driver failed initializing effect slots",
            ));
        }

        device.spring_level = get_env_int("SDL_HAPTIC_LG4FF_SPRING", 0, 100, 30);
        device.damper_level = get_env_int("SDL_HAPTIC_LG4FF_DAMPER", 0, 100, 30);
        device.friction_level = get_env_int("SDL_HAPTIC_LG4FF_FRICTION", 0, 100, 30);
        device.gain = get_env_int("SDL_HAPTIC_LG4FF_GAIN", 0, 65535, 65535);

        let product_id = joystick.product();
        let release_number = joystick.product_version();
        device.is_ffex = is_ffex(product_id, release_number);

        let thread_name = format!(
            "SDL_hidapihaptic_lg4ff {} {:04x}:{:04x}",
            joystick.id(),
            USB_VENDOR_LOGITECH,
            product_id
        );
        let shared = Arc::new(Shared {
            device: Mutex::new(device),
            app_gain: AtomicI32::new(65535),
            stop_thread: AtomicBool::new(false),
            joystick,
            clock,
        });

        // (upstream sets is_ffex after starting the thread, which doesn't read it)
        let thread = if start_thread {
            let ctx = shared.clone();
            Thread::spawn(thread_name, move || thread_function(&ctx)).ok()
        } else {
            None
        };

        Ok(Lg4ffHaptic { shared, thread })
    }

    /// The effect thread's step at `now` (for the tests, which don't start
    /// the thread).
    #[cfg(test)]
    fn timer(&self, now: u64) -> i32 {
        self.shared.timer(now)
    }

    fn now(&self) -> u64 {
        (self.shared.clock)()
    }

    /// Translation of `SDL_HIDAPI_HapticDriverLg4ff_RunEffect()`.
    fn play(&mut self, id: HapticEffectID, iterations: u32) -> Result<()> {
        let mut ctx = self.shared.lock();
        let Some(index) = ctx.valid_active(id) else {
            return Err(Error::new("Bad effect id"));
        };

        // FIXME (upstream): the iterations become an int, so running an
        // effect SDL_HAPTIC_INFINITY times (-1) stops it instead.
        ctx.play_effect(index, iterations as i32, self.now());
        Ok(())
    }

    /// Send the autocenter commands, with the error of each.
    fn send_autocenter(&self, commands: &[(Command, &'static str)]) -> Result<()> {
        for (cmd, error) in commands {
            if self.shared.joystick.send_effect(cmd).is_err() {
                return Err(Error::new(*error));
            }
        }
        Ok(())
    }
}

/// `SDL_HIDAPI_HapticDriverLg4ff_GetFeatures()`
const FEATURES: HapticFeatures = HapticFeatures(
    HapticFeatures::CONSTANT.0
        | HapticFeatures::SPRING.0
        | HapticFeatures::DAMPER.0
        | HapticFeatures::AUTOCENTER.0
        | HapticFeatures::SINE.0
        | HapticFeatures::SQUARE.0
        | HapticFeatures::TRIANGLE.0
        | HapticFeatures::SAWTOOTHUP.0
        | HapticFeatures::SAWTOOTHDOWN.0
        | HapticFeatures::RAMP.0
        | HapticFeatures::FRICTION.0
        | HapticFeatures::STATUS.0
        | HapticFeatures::GAIN.0,
);

impl HidapiHapticContext for Lg4ffHaptic {
    /// Translation of `SDL_HIDAPI_HapticDriverLg4ff_Close()`.
    fn close(&mut self) {
        let _ = self.stop_effects();

        // let effects finish in lg4ff_timer
        crate::timer::delay(Duration::from_millis(50));

        self.shared.stop_thread.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            thread.wait();
        }
    }

    /// Translation of `SDL_HIDAPI_HapticDriverLg4ff_NumEffects()`.
    fn num_effects(&self) -> i32 {
        LG4FF_MAX_EFFECTS as i32
    }

    /// `NumEffectsPlaying` is `SDL_HIDAPI_HapticDriverLg4ff_NumEffects()`
    /// too.
    fn num_effects_playing(&self) -> i32 {
        LG4FF_MAX_EFFECTS as i32
    }

    /// Translation of `SDL_HIDAPI_HapticDriverLg4ff_GetFeatures()`.
    fn features(&self) -> HapticFeatures {
        FEATURES
    }

    /// Translation of `SDL_HIDAPI_HapticDriverLg4ff_NumAxes()`.
    fn num_axes(&self) -> i32 {
        1
    }

    /// Translation of `SDL_HIDAPI_HapticDriverLg4ff_CreateEffect()`.
    fn create_effect(&mut self, data: &HapticEffect) -> Result<HapticEffectID> {
        // (SDL_HIDAPI_HapticDriverLg4ff_EffectSupported())
        if !FEATURES.intersects(data.effect_type()) {
            return Err(Error::new("Unsupported effect"));
        }

        let mut ctx = self.shared.lock();
        let Some(state_slot) = ctx.states.iter().position(|s| !s.allocated) else {
            return Err(Error::new("All effect slots in-use"));
        };

        if ctx.upload_effect(data, state_slot, self.now()) {
            // Note (upstream): upstream marks the slot allocated after
            // unlocking the mutex.
            ctx.states[state_slot].allocated = true;
            Ok(state_slot as HapticEffectID)
        } else {
            Err(Error::new("Bad effect parameters"))
        }
    }

    /// Translation of `SDL_HIDAPI_HapticDriverLg4ff_UpdateEffect()`.
    fn update_effect(&mut self, id: HapticEffectID, data: &HapticEffect) -> Result<()> {
        let mut ctx = self.shared.lock();
        let Some(index) = ctx.valid_active(id) else {
            return Err(Error::new("Bad effect id"));
        };

        if ctx.upload_effect(data, index, self.now()) {
            Ok(())
        } else {
            // (upstream sets no error)
            Err(Error::new("Couldn't update effect"))
        }
    }

    /// Translation of `SDL_HIDAPI_HapticDriverLg4ff_RunEffect()`.
    fn run_effect(&mut self, id: HapticEffectID, iterations: u32) -> Result<()> {
        self.play(id, iterations)
    }

    /// Translation of `SDL_HIDAPI_HapticDriverLg4ff_StopEffect()`.
    fn stop_effect(&mut self, id: HapticEffectID) -> Result<()> {
        self.play(id, 0)
    }

    /// Translation of `SDL_HIDAPI_HapticDriverLg4ff_DestroyEffect()`.
    ///
    /// (Neither this nor `stop_effects` count the effect out of
    /// `effects_used`, as upstream; that only makes the timer look at all
    /// the effects.)
    fn destroy_effect(&mut self, id: HapticEffectID) {
        let mut ctx = self.shared.lock();
        let Some(index) = ctx.valid_active(id) else {
            return;
        };

        let state = &mut ctx.states[index];
        state.stop();
        state.allocated = false;
    }

    /// Translation of `SDL_HIDAPI_HapticDriverLg4ff_GetEffectStatus()`.
    fn effect_status(&mut self, id: HapticEffectID) -> bool {
        let ctx = self.shared.lock();
        ctx.valid_active(id)
            .is_some_and(|index| ctx.states[index].test(FF_EFFECT_STARTED))
    }

    /// Translation of `SDL_HIDAPI_HapticDriverLg4ff_SetGain()`.
    fn set_gain(&mut self, gain: i32) -> Result<()> {
        let gain = gain.clamp(0, 100);
        self.shared
            .app_gain
            .store((65535 * gain) / 100, Ordering::Relaxed);
        Ok(())
    }

    /// Translation of `SDL_HIDAPI_HapticDriverLg4ff_SetAutocenter()`.
    ///
    /// *Ported*
    /// Original functions by:
    /// Simon Wood <simon@mungewell.org>
    /// Michal Malý <madcatxster@devoid-pointer.net> <madcatxster@gmail.com>
    /// lg4ff_set_autocenter_default lg4ff_set_autocenter_ffex
    /// `git blame v6.12 drivers/hid/hid-lg4ff.c`, <https://github.com/torvalds/linux.git>
    fn set_autocenter(&mut self, autocenter: i32) -> Result<()> {
        let autocenter = autocenter.clamp(0, 100);

        let ctx = self.shared.lock();
        if ctx.is_ffex {
            let magnitude = (90 * autocenter) / 100;
            let cmd = ffex_autocenter_command(magnitude as u32);
            self.send_autocenter(&[(cmd, "Failed sending autocenter command")])
        } else {
            // first disable, set strength (when not 0) and enable: the
            // joystick driver's commands
            let magnitude = (65535 * autocenter) / 100;
            const ERRORS: [&str; 3] = [
                "Failed sending autocenter disable command",
                "Failed sending autocenter magnitude command",
                "Failed sending autocenter enable command",
            ];
            let commands: Vec<(Command, &str)> = autocenter_commands(false, magnitude as u32)
                .into_iter()
                .zip(ERRORS)
                .collect();
            self.send_autocenter(&commands)
        }
    }

    /// Translation of `SDL_HIDAPI_HapticDriverLg4ff_Pause()`.
    fn pause(&mut self) -> Result<()> {
        Err(Error::unsupported())
    }

    /// Translation of `SDL_HIDAPI_HapticDriverLg4ff_Resume()`.
    fn resume(&mut self) -> Result<()> {
        Err(Error::unsupported())
    }

    /// Translation of `SDL_HIDAPI_HapticDriverLg4ff_StopEffects()`.
    fn stop_effects(&mut self) -> Result<()> {
        let mut ctx = self.shared.lock();
        for state in &mut ctx.states {
            state.stop();
        }
        Ok(())
    }
}

impl Drop for Lg4ffHaptic {
    fn drop(&mut self) {
        // (the front end closes it first; don't leave the thread running)
        if self.thread.is_some() {
            self.close();
        }
    }
}

#[cfg(test)]
pub(super) mod tests;
