// Tests for the Darwin haptic driver.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

use super::*;
use crate::haptic::{
    HapticCondition, HapticConditionKind, HapticConstant, HapticCustom, HapticLeftRight,
    HapticPeriodic, HapticRamp, HapticWaveform,
};

/// The axes of the test device: X, Y and Rz (`FFJOFS_X`, `FFJOFS_Y`,
/// `FFJOFS_RZ`).
const HW_AXES: [u8; 3] = [0, 4, 20];

/// An effect as the upstream harness prints an `FFEFFECT`.
fn effect_string(label: &str, e: &mut FfEffect) -> String {
    e.with_ffeffect(|e| {
        let mut s = format!(
            "{label} flags={} dur={} gain={} trig={} interval={} delay={} caxes={}",
            e.dw_flags,
            e.dw_duration,
            e.dw_gain,
            e.dw_trigger_button,
            e.dw_trigger_repeat_interval,
            e.dw_start_delay,
            e.c_axes
        );
        s += " axes=";
        for i in 0..e.c_axes as usize {
            s += &format!("{},", unsafe { *e.rgdw_axes.add(i) });
        }
        s += " dir=";
        if e.rgl_direction.is_null() {
            s += "null";
        } else {
            for i in 0..e.c_axes as usize {
                s += &format!("{},", unsafe { *e.rgl_direction.add(i) });
            }
        }
        s += " env=";
        if e.lp_envelope.is_null() {
            s += "null";
        } else {
            let env = unsafe { *e.lp_envelope };
            s += &format!(
                "{},{},{},{}",
                env.dw_attack_level, env.dw_attack_time, env.dw_fade_level, env.dw_fade_time
            );
        }
        s += &format!(" cb={} params=", e.cb_type_specific_params);
        s
    }) + &match &e.params {
        _ if matches!(&e.params, TypeSpecificParams::Condition(c) if c.is_empty()) => {
            "null".to_owned()
        }
        TypeSpecificParams::Constant(c) => format!("{}", c.l_magnitude),
        TypeSpecificParams::Periodic(p) => {
            format!(
                "{},{},{},{}",
                p.dw_magnitude, p.l_offset, p.dw_phase, p.dw_period
            )
        }
        TypeSpecificParams::Condition(conditions) => conditions
            .iter()
            .map(|c| {
                format!(
                    "[{},{},{},{},{},{}]",
                    c.l_offset,
                    c.l_positive_coefficient,
                    c.l_negative_coefficient,
                    c.dw_positive_saturation,
                    c.dw_negative_saturation,
                    c.l_dead_band
                )
            })
            .collect(),
        TypeSpecificParams::Ramp(r) => format!("{},{}", r.l_start, r.l_end),
        TypeSpecificParams::Custom {
            c_channels,
            dw_sample_period,
            c_samples,
            rgl_force_data,
        } => {
            let mut s = format!("{c_channels},{dw_sample_period},{c_samples}:");
            for d in rgl_force_data {
                s += &format!("{d},");
            }
            s
        }
    }
}

fn direction(kind: HapticDirectionType, dir: [i32; 3]) -> HapticDirection {
    HapticDirection { kind, dir }
}

fn convert_effect(label: &str, naxes: i32, effect: &HapticEffect) -> String {
    match to_ffeffect(naxes, &HW_AXES, effect) {
        Ok(mut e) => effect_string(label, &mut e),
        Err(e) => format!("{label} err: {}", e.message()),
    }
}

/// The effects of the upstream harness (SDL_SYS_ToFFEFFECT() compiled from
/// upstream's SDL_syshaptic.c, with durations and trigger repeat intervals
/// in microseconds and SDL_HAPTIC_INFINITY as FF_INFINITE, as here), with
/// the lines it printed.
#[test]
fn effects_convert_like_upstream() {
    let constant = HapticEffect::Constant(HapticConstant {
        direction: direction(HapticDirectionType::Polar, [18000, 0, 0]),
        length: 5000,
        delay: 100,
        button: 3,
        interval: 250,
        level: -20000,
        attack_length: 200,
        attack_level: 40000,
        fade_length: 0,
        fade_level: 1234,
    });
    assert_eq!(
        convert_effect("constant", 2, &constant),
        "constant flags=34 dur=5000000 gain=10000 trig=50 interval=250000 delay=100000 caxes=2 axes=0,4, dir=18000,0, env=10000,200000,376,0 cb=4 params=-6103"
    );

    let periodic = HapticEffect::Periodic(HapticPeriodic {
        waveform: HapticWaveform::SawtoothDown,
        direction: direction(HapticDirectionType::Cartesian, [1, -1, 5]),
        length: HAPTIC_INFINITY,
        period: 65535,
        magnitude: -32768,
        offset: 32767,
        phase: 27000,
        ..Default::default()
    });
    assert_eq!(
        convert_effect("periodic", 3, &periodic),
        "periodic flags=18 dur=4294967295 gain=10000 trig=4294967295 interval=0 delay=0 caxes=3 axes=0,4,20, dir=1,-1,5, env=null cb=16 params=10000,10000,9000,65535000"
    );

    let sine = HapticEffect::Periodic(HapticPeriodic {
        waveform: HapticWaveform::Sine,
        direction: direction(HapticDirectionType::Spherical, [9000, 4500, 0]),
        length: 1000,
        period: 20,
        magnitude: 16384,
        offset: -100,
        phase: 100,
        fade_length: 300,
        fade_level: 32768,
        ..Default::default()
    });
    assert_eq!(
        convert_effect("sine", 1, &sine),
        "sine flags=66 dur=1000000 gain=10000 trig=4294967295 interval=0 delay=0 caxes=1 axes=0, dir=9000, env=0,0,10000,300000 cb=16 params=5000,-30,100,20000"
    );

    let spring = HapticEffect::Condition(HapticCondition {
        kind: HapticConditionKind::Spring,
        direction: direction(HapticDirectionType::SteeringAxis, [0, 0, 0]),
        length: 2000,
        button: 1,
        right_sat: [65535, 0, 0],
        left_sat: [30000, 0, 0],
        right_coeff: [32767, 0, 0],
        left_coeff: [-32768, 0, 0],
        deadband: [1000, 0, 0],
        center: [-500, 0, 0],
        ..Default::default()
    });
    assert_eq!(
        convert_effect("spring", 3, &spring),
        "spring flags=18 dur=2000000 gain=10000 trig=48 interval=0 delay=0 caxes=1 axes=0, dir=0, env=null cb=24 params=[-152,10000,-10000,10000,4577,152]"
    );

    let damper = HapticEffect::Condition(HapticCondition {
        kind: HapticConditionKind::Damper,
        direction: direction(HapticDirectionType::Cartesian, [1, 0, 0]),
        length: 10,
        right_sat: [0, 1000, 0],
        center: [0, 0, 32767],
        ..Default::default()
    });
    assert_eq!(
        convert_effect("damper", 3, &damper),
        "damper flags=18 dur=10000 gain=10000 trig=4294967295 interval=0 delay=0 caxes=3 axes=0,4,20, dir=1,0,0, env=null cb=72 params=[0,0,0,0,0,0][0,0,0,152,0,0][10000,0,0,0,0,0]"
    );
    // Without axes, no conditions (and no direction)
    assert_eq!(
        convert_effect("damper0", 0, &damper),
        "damper0 flags=66 dur=10000 gain=10000 trig=4294967295 interval=0 delay=0 caxes=0 axes= dir=null env=null cb=0 params=null"
    );

    let ramp = HapticEffect::Ramp(HapticRamp {
        direction: direction(HapticDirectionType::Polar, [35999, 0, 0]),
        length: 750,
        delay: 65535,
        start: -32768,
        end: 32767,
        attack_length: 1,
        fade_length: 2,
        fade_level: 65535,
        ..Default::default()
    });
    assert_eq!(
        convert_effect("ramp", 2, &ramp),
        "ramp flags=34 dur=750000 gain=10000 trig=4294967295 interval=0 delay=65535000 caxes=2 axes=0,4, dir=35999,0, env=0,1000,10000,2000 cb=8 params=-10000,10000"
    );

    // Lengths past what the DWORD holds in microseconds; the longest interval
    let long = |length| {
        let HapticEffect::Ramp(ramp) = &ramp else {
            unreachable!()
        };
        HapticEffect::Ramp(HapticRamp {
            length,
            interval: 65535,
            ..*ramp
        })
    };
    assert_eq!(
        convert_effect("long", 2, &long(4294968)),
        "long flags=34 dur=4294967294 gain=10000 trig=4294967295 interval=65535000 delay=65535000 caxes=2 axes=0,4, dir=35999,0, env=0,1000,10000,2000 cb=8 params=-10000,10000"
    );
    assert_eq!(
        convert_effect("longest", 2, &long(4294967)),
        "longest flags=34 dur=4294967000 gain=10000 trig=4294967295 interval=65535000 delay=65535000 caxes=2 axes=0,4, dir=35999,0, env=0,1000,10000,2000 cb=8 params=-10000,10000"
    );

    let custom = HapticEffect::Custom(HapticCustom {
        direction: direction(HapticDirectionType::Cartesian, [-7, 0, 0]),
        length: 100,
        channels: 2,
        period: 10,
        samples: 3,
        data: vec![0, 100, 32767, 32768, 65535, 12345],
        ..Default::default()
    });
    assert_eq!(
        convert_effect("custom", 1, &custom),
        "custom flags=18 dur=100000 gain=10000 trig=4294967295 interval=0 delay=0 caxes=1 axes=0, dir=-7, env=null cb=24 params=2,10000,3:0,30,10000,10000,10000,3767,"
    );
    assert_eq!(
        convert_effect("custom0", 0, &custom),
        "custom0 flags=66 dur=100000 gain=10000 trig=4294967295 interval=0 delay=0 caxes=0 axes= dir=null env=null cb=24 params=2,10000,3:0,30,10000,10000,10000,3767,"
    );

    let leftright = HapticEffect::LeftRight(HapticLeftRight::default());
    assert_eq!(
        convert_effect("leftright", 2, &leftright),
        "leftright err: Haptic: Unknown effect type."
    );
}

#[test]
fn effect_types_and_features() {
    // Every effect but the left/right one has its ForceFeedback type
    assert_eq!(
        haptic_effect_type(&HapticEffect::Constant(HapticConstant::default())),
        Some(k_ff_effect_type_constant_force_id())
    );
    assert_eq!(
        haptic_effect_type(&HapticEffect::Periodic(HapticPeriodic {
            waveform: HapticWaveform::Triangle,
            ..Default::default()
        })),
        Some(k_ff_effect_type_triangle_id())
    );
    assert_eq!(
        haptic_effect_type(&HapticEffect::Condition(HapticCondition {
            kind: HapticConditionKind::Friction,
            ..Default::default()
        })),
        Some(k_ff_effect_type_friction_id())
    );
    assert_eq!(
        haptic_effect_type(&HapticEffect::Custom(HapticCustom::default())),
        Some(k_ff_effect_type_custom_force_id())
    );
    assert_eq!(
        haptic_effect_type(&HapticEffect::LeftRight(HapticLeftRight::default())),
        None
    );

    assert_eq!(supported_effects_features(0), HapticFeatures::NONE);
    assert_eq!(
        supported_effects_features(FFCAP_ET_SINE | FFCAP_ET_CUSTOMFORCE | FFCAP_ET_RAMPFORCE),
        HapticFeatures::SINE | HapticFeatures::CUSTOM | HapticFeatures::RAMP
    );
    assert_eq!(supported_effects_features(0xFFF).0.count_ones(), 12);

    assert_eq!(ff_get_trigger_button(0), FFEB_NOTRIGGER);
    assert_eq!(ff_get_trigger_button(1), 48);
    assert_eq!(ff_get_trigger_button(32), 79);
    assert_eq!(cconvert(0x8000), 10000);
    assert_eq!(cconvert(0x4000), 5000);
    assert_eq!(convert(-32768), -10000);
    assert_eq!(
        UPDATE_FLAGS,
        FFEP_DIRECTION
            | FFEP_DURATION
            | FFEP_ENVELOPE
            | FFEP_STARTDELAY
            | FFEP_TRIGGERBUTTON
            | FFEP_TRIGGERREPEATINTERVAL
            | FFEP_TYPESPECIFICPARAMS
    );
}

/// The HID devices with force feedback this machine has (none is fine),
/// through the haptic subsystem; a second init is refused, and the list
/// goes with quit.
#[test]
fn enumerate_through_the_haptic_subsystem() {
    let _l = crate::test_support::test_lock();
    crate::init::init_subsystem(crate::init::InitFlags::HAPTIC).unwrap();
    assert!(initialized());
    assert_eq!(
        DARWIN_HAPTIC_DRIVER.init().unwrap_err().message(),
        "Haptic subsystem already initialized!"
    );
    let ids = crate::haptic::haptics();
    if ids.is_empty() {
        println!("note: no force feedback devices");
    }
    for (index, id) in ids.iter().enumerate() {
        assert_ne!(*id, 0);
        println!(
            "haptic {id}: {:?}",
            DARWIN_HAPTIC_DRIVER.name(index).unwrap_or_default()
        );
    }
    let n = ids.len();
    assert_eq!(DARWIN_HAPTIC_DRIVER.instance_id(n), 0);
    assert_eq!(DARWIN_HAPTIC_DRIVER.name(n), None);
    // (no device is the mouse here unless one is)
    if ids.is_empty() {
        assert_eq!(DARWIN_HAPTIC_DRIVER.mouse(), None);
        assert!(crate::haptic::Haptic::open_from_mouse().is_err());
    }
    // An IO object that isn't a HID device with force feedback isn't added
    assert!(!mac_haptic_maybe_add_device(0));
    assert!(!mac_haptic_maybe_remove_device(0));
    crate::init::quit_subsystem(crate::init::InitFlags::HAPTIC);
    assert!(!initialized());
    assert_eq!(DARWIN_HAPTIC_DRIVER.count(), 0);
    // Not initialized: devices aren't picked up
    assert!(!mac_haptic_maybe_add_device(0));
}

/// The system encoding conversion of the product names.
#[test]
fn product_name_strings() {
    let s = crate::core::apple::foundation::cf_string("Force Pad");
    assert_eq!(
        unsafe { c_string_system_encoding(s.get(), 256) }.as_deref(),
        Some("Force Pad")
    );
    // Longer than the buffer: fails, as CFStringGetCString() does
    let long = crate::core::apple::foundation::cf_string(&"x".repeat(300));
    assert_eq!(unsafe { c_string_system_encoding(long.get(), 256) }, None);
    // Not a string
    let n = crate::core::apple::foundation::cf_number_i32(3);
    assert_eq!(unsafe { c_string_system_encoding(n.get(), 256) }, None);
}
