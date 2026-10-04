// Tests for the Windows haptic driver (run under Wine on Linux).
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

use super::super::*;
use super::dinput::*;
use crate::core::windows::directx::*;

/// The axes of the test device: X, Y and Rz.
const HW_AXES: [u32; 3] = [DIJOFS_X, DIJOFS_Y, DIJOFS_RZ];

/// An effect as upstream's harness prints a `DIEFFECT`.
fn effect_string(label: &str, e: &DiEffect) -> String {
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
    for axis in &e.rgdw_axes {
        s += &format!("{axis},");
    }
    s += " dir=";
    match &e.rgl_direction {
        Some(direction) => {
            for d in direction {
                s += &format!("{d},");
            }
        }
        None => s += "null",
    }
    s += " env=";
    match &e.lp_envelope {
        Some(env) => {
            s += &format!(
                "{},{},{},{}",
                env.dwAttackLevel, env.dwAttackTime, env.dwFadeLevel, env.dwFadeTime
            )
        }
        None => s += "null",
    }
    let cb = match &e.params {
        TypeSpecificParams::Constant(_) => 4,
        TypeSpecificParams::Periodic(_) => 16,
        TypeSpecificParams::Condition(c) => 24 * c.len(),
        TypeSpecificParams::Ramp(_) => 8,
        TypeSpecificParams::Custom { .. } => size_of::<DICUSTOMFORCE>(),
    };
    s += &format!(" cb={cb} params=");
    match &e.params {
        TypeSpecificParams::Constant(c) => s += &format!("{}", c.lMagnitude),
        TypeSpecificParams::Periodic(p) => {
            s += &format!(
                "{},{},{},{}",
                p.dwMagnitude, p.lOffset, p.dwPhase, p.dwPeriod
            )
        }
        TypeSpecificParams::Condition(conditions) => {
            for c in conditions {
                s += &format!(
                    "[{},{},{},{},{},{}]",
                    c.lOffset,
                    c.lPositiveCoefficient,
                    c.lNegativeCoefficient,
                    c.dwPositiveSaturation,
                    c.dwNegativeSaturation,
                    c.lDeadBand
                );
            }
        }
        TypeSpecificParams::Ramp(r) => s += &format!("{},{}", r.lStart, r.lEnd),
        TypeSpecificParams::Custom {
            c_channels,
            dw_sample_period,
            c_samples,
            rgl_force_data,
        } => {
            s += &format!("{c_channels},{dw_sample_period},{c_samples}:");
            for d in rgl_force_data {
                s += &format!("{d},");
            }
        }
    }
    s
}

fn direction(kind: HapticDirectionType, dir: [i32; 3]) -> HapticDirection {
    HapticDirection { kind, dir }
}

#[test]
fn effects_convert_like_upstream() {
    // (expected values from upstream's SDL_SYS_ToDIEFFECT(), run on the same
    // effects)
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
        effect_string("constant", &to_dieffect(2, &HW_AXES, &constant).unwrap()),
        "constant flags=34 dur=5000000 gain=10000 trig=50 interval=250 delay=100000 caxes=2 axes=0,4, dir=18000,0, env=10000,200000,376,0 cb=4 params=-6103"
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
        effect_string("periodic", &to_dieffect(3, &HW_AXES, &periodic).unwrap()),
        "periodic flags=18 dur=4294966296 gain=10000 trig=4294967295 interval=0 delay=0 caxes=3 axes=0,4,20, dir=1,-1,5, env=null cb=16 params=10000,10000,9000,65535000"
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
        fade_level: 0x7FFF,
        attack_level: 0x8000,
        ..Default::default()
    });
    assert_eq!(
        effect_string("sine0", &to_dieffect(0, &HW_AXES, &sine).unwrap()),
        "sine0 flags=66 dur=1000000 gain=10000 trig=4294967295 interval=0 delay=0 caxes=0 axes= dir=null env=10000,0,10000,300000 cb=16 params=5000,-30,100,20000"
    );

    let spring = HapticEffect::Condition(HapticCondition {
        kind: HapticConditionKind::Spring,
        direction: direction(HapticDirectionType::SteeringAxis, [0; 3]),
        length: 100,
        button: 1,
        right_sat: [0xFFFF, 7, 0],
        left_sat: [0x1234, 0, 0],
        right_coeff: [-32768, 0, 0],
        left_coeff: [32767, 0, 0],
        deadband: [0xFFFF, 0, 0],
        center: [-1000, 99, 0],
        ..Default::default()
    });
    assert_eq!(
        effect_string("spring", &to_dieffect(3, &HW_AXES, &spring).unwrap()),
        "spring flags=18 dur=100000 gain=10000 trig=48 interval=0 delay=0 caxes=1 axes=0, dir=0, env=null cb=24 params=[-305,-10000,10000,10000,711,10000]"
    );

    let friction = HapticEffect::Condition(HapticCondition {
        kind: HapticConditionKind::Friction,
        direction: direction(HapticDirectionType::Cartesian, [3, 4, 0]),
        right_sat: [1000, 2000, 3000],
        left_sat: [30000, 30001, 30002],
        right_coeff: [0, -1000, -2000],
        left_coeff: [0, 3000, 6000],
        deadband: [0, 500, 1000],
        center: [0, 7, 14],
        ..Default::default()
    });
    assert_eq!(
        effect_string("friction", &to_dieffect(2, &HW_AXES, &friction).unwrap()),
        "friction flags=18 dur=0 gain=10000 trig=4294967295 interval=0 delay=0 caxes=2 axes=0,4, dir=3,4, env=null cb=48 params=[0,0,0,152,4577,0][2,-305,915,305,4577,76]"
    );

    let ramp = HapticEffect::Ramp(HapticRamp {
        direction: direction(HapticDirectionType::Polar, [0; 3]),
        length: 4294968,
        start: -32767,
        end: 12345,
        fade_length: 1,
        attack_length: 2,
        attack_level: 77,
        ..Default::default()
    });
    assert_eq!(
        effect_string("ramp", &to_dieffect(2, &HW_AXES, &ramp).unwrap()),
        "ramp flags=34 dur=704 gain=10000 trig=4294967295 interval=0 delay=0 caxes=2 axes=0,4, dir=0,0, env=23,2000,0,1000 cb=8 params=-10000,3767"
    );

    let custom = HapticEffect::Custom(HapticCustom {
        direction: direction(HapticDirectionType::Polar, [0; 3]),
        channels: 2,
        period: 10,
        samples: 3,
        data: vec![0, 1, 0x7FFF, 0x8000, 0xFFFF, 1000],
        length: 30,
        attack_length: 5,
        ..Default::default()
    });
    assert_eq!(
        effect_string("custom", &to_dieffect(2, &HW_AXES, &custom).unwrap()),
        format!(
            "custom flags=34 dur=30000 gain=10000 trig=4294967295 interval=0 delay=0 caxes=2 axes=0,4, dir=0,0, env=0,5000,0,0 cb={} params=2,10000,3:0,0,10000,10000,10000,305,",
            size_of::<DICUSTOMFORCE>()
        )
    );
    // (missing samples read as 0, where upstream reads past the array)
    let HapticEffect::Custom(mut short) = custom.clone() else {
        unreachable!()
    };
    short.data = vec![0x7FFF];
    let effect = to_dieffect(2, &HW_AXES, &HapticEffect::Custom(short)).unwrap();
    assert!(
        matches!(&effect.params, TypeSpecificParams::Custom { rgl_force_data, .. } if rgl_force_data == &[10000, 0, 0, 0, 0, 0])
    );

    // DirectInput has no left/right effect
    let left_right = HapticEffect::LeftRight(HapticLeftRight::default());
    assert!(to_dieffect(2, &HW_AXES, &left_right).is_err());
    assert!(haptic_effect_type(&left_right).is_none());
    // (and conditions need an axis)
    let HapticEffect::Condition(mut no_axes) = friction.clone() else {
        unreachable!()
    };
    no_axes.direction.kind = HapticDirectionType::Polar;
    assert!(to_dieffect(0, &HW_AXES, &HapticEffect::Condition(no_axes)).is_err());
    // More than 3 axes (the haptic axes hint): the others are 0
    let effect = to_dieffect(5, &HW_AXES, &friction).unwrap();
    assert_eq!(effect.rgdw_axes, [0, 4, 20, 0, 0]);
    assert_eq!(effect.rgl_direction, Some(vec![3, 4, 0, 0, 0]));
}

#[test]
fn effect_types() {
    let periodic = |waveform| {
        HapticEffect::Periodic(HapticPeriodic {
            waveform,
            ..Default::default()
        })
    };
    let condition = |kind| {
        HapticEffect::Condition(HapticCondition {
            kind,
            ..Default::default()
        })
    };
    let cases = [
        (
            HapticEffect::Constant(Default::default()),
            GUID_CONSTANTFORCE,
        ),
        (HapticEffect::Ramp(Default::default()), GUID_RAMPFORCE),
        (periodic(HapticWaveform::Square), GUID_SQUARE),
        (periodic(HapticWaveform::Sine), GUID_SINE),
        (periodic(HapticWaveform::Triangle), GUID_TRIANGLE),
        (periodic(HapticWaveform::SawtoothUp), GUID_SAWTOOTHUP),
        (periodic(HapticWaveform::SawtoothDown), GUID_SAWTOOTHDOWN),
        (condition(HapticConditionKind::Spring), GUID_SPRING),
        (condition(HapticConditionKind::Damper), GUID_DAMPER),
        (condition(HapticConditionKind::Inertia), GUID_INERTIA),
        (condition(HapticConditionKind::Friction), GUID_FRICTION),
        (HapticEffect::Custom(Default::default()), GUID_CUSTOMFORCE),
    ];
    for (effect, guid) in &cases {
        let got = haptic_effect_type(effect).unwrap();
        assert!(
            crate::core::windows::is_equal_guid(&got, guid),
            "{effect:?}"
        );
        // DI_EffectCallback() maps the GUID back
        assert_eq!(di_effect_feature(guid), effect.effect_type());
    }
    assert_eq!(di_effect_feature(&GUID_XAXIS), HapticFeatures::NONE);
}

#[test]
fn update_flags_like_upstream() {
    // (expected values from upstream's DICalculateUpdateFlags())
    let mut constant = HapticConstant {
        direction: direction(HapticDirectionType::Polar, [0; 3]),
        length: 1000,
        level: 100,
        ..Default::default()
    };
    let effect = |naxes, c: &HapticConstant| {
        to_dieffect(naxes, &HW_AXES, &HapticEffect::Constant(*c)).unwrap()
    };
    let before = effect(2, &constant);
    assert_eq!(
        di_calculate_update_flags(&before, &effect(2, &constant)),
        985
    );
    constant.level = 200;
    assert_eq!(
        di_calculate_update_flags(&before, &effect(2, &constant)),
        256
    );
    constant.direction.dir[0] = 5;
    constant.length = 2000;
    assert_eq!(
        di_calculate_update_flags(&before, &effect(2, &constant)),
        321
    );
    constant.attack_length = 10;
    constant.delay = 3;
    constant.button = 2;
    constant.interval = 9;
    assert_eq!(
        di_calculate_update_flags(&before, &effect(2, &constant)),
        985
    );
    let no_axes = effect(0, &constant);
    assert_eq!(di_calculate_update_flags(&before, &no_axes), 985);
    assert_eq!(
        di_calculate_update_flags(&effect(0, &constant), &no_axes),
        985
    );

    // Custom effects always count as changed (upstream compares pointers)
    let custom = to_dieffect(
        1,
        &HW_AXES,
        &HapticEffect::Custom(HapticCustom {
            channels: 1,
            samples: 1,
            data: vec![5],
            ..Default::default()
        }),
    )
    .unwrap();
    assert_eq!(
        di_calculate_update_flags(&custom, &custom.clone()),
        DIEP_TYPESPECIFICPARAMS
    );
}

#[test]
fn conversions() {
    // DIGetTriggerButton()
    assert_eq!(di_get_trigger_button(0), DIEB_NOTRIGGER);
    assert_eq!(di_get_trigger_button(1), 48);
    assert_eq!(di_get_trigger_button(128), 48 + 127);
    // CCONVERT() and CONVERT()
    assert_eq!(cconvert(0x7FFF), 10000);
    assert_eq!(cconvert(0x8000), 10000);
    assert_eq!(cconvert(1), 0);
    assert_eq!(cconvert(4), 1);
    assert_eq!(convert(-32768), -10000);
    assert_eq!(convert(-4), -1);
    assert_eq!(convert(32767), 10000);

    // The DIEFFECT a DiEffect gives DirectInput
    let mut effect = to_dieffect(
        2,
        &HW_AXES,
        &HapticEffect::Constant(HapticConstant {
            direction: direction(HapticDirectionType::Cartesian, [7, 8, 9]),
            level: 32767,
            attack_length: 1,
            ..Default::default()
        }),
    )
    .unwrap();
    effect.with_dieffect(|e| {
        assert_eq!(e.dwSize, size_of::<DIEFFECT>() as u32);
        assert_eq!(e.cAxes, 2);
        assert_eq!(e.cbTypeSpecificParams, 4);
        // SAFETY: the pointers point into `effect` for the call.
        unsafe {
            assert_eq!(*e.rgdwAxes.add(1), 4);
            assert_eq!(*e.rglDirection.add(1), 8);
            assert_eq!((*e.lpEnvelope).dwAttackTime, 1000);
            assert_eq!(
                (*e.lpvTypeSpecificParams.cast::<DICONSTANTFORCE>()).lMagnitude,
                10000
            );
        }
    });
    let mut custom = to_dieffect(
        0,
        &HW_AXES,
        &HapticEffect::Custom(HapticCustom {
            channels: 1,
            samples: 2,
            data: vec![0x7FFF, 0],
            ..Default::default()
        }),
    )
    .unwrap();
    custom.with_dieffect(|e| {
        assert!(e.rgdwAxes.is_null());
        assert!(e.rglDirection.is_null());
        assert!(e.lpEnvelope.is_null());
        // SAFETY: as above.
        let force = unsafe { *e.lpvTypeSpecificParams.cast::<DICUSTOMFORCE>() };
        assert_eq!((force.cChannels, force.cSamples), (1, 2));
        // SAFETY: as above; the force data holds 2 samples.
        assert_eq!(unsafe { *force.rglForceData }, 10000);
    });
}

#[test]
fn driver_without_devices() {
    let _l = crate::test_support::test_lock();
    // Wine (and CI) have no force feedback devices; the joystick subsystem
    // first, then without it (when haptics enumerates game controllers too)
    for joystick in [true, false] {
        if joystick {
            crate::init::init_subsystem(crate::init::InitFlags::JOYSTICK).unwrap();
        }
        crate::init::init_subsystem(crate::init::InitFlags::HAPTIC).unwrap();
        // (Wine has DirectInput, so this enumerates)
        assert!(dinput_in_use());
        assert_eq!(haptics(), []);
        assert!(!is_mouse_haptic());
        assert!(Haptic::open(1).is_err());
        assert_eq!(super::WINDOWS_HAPTIC_DRIVER.instance_id(0), 0);
        assert_eq!(super::WINDOWS_HAPTIC_DRIVER.name(0), None);
        crate::init::quit_subsystem(crate::init::InitFlags::HAPTIC);
        if joystick {
            crate::init::quit_subsystem(crate::init::InitFlags::JOYSTICK);
        }
    }

    // With DirectInput off nothing is enumerated at all
    crate::hints::set(crate::hints::JOYSTICK_DIRECTINPUT, "0").unwrap();
    crate::init::init_subsystem(crate::init::InitFlags::HAPTIC).unwrap();
    assert!(!dinput_in_use());
    assert_eq!(haptics(), []);
    crate::init::quit_subsystem(crate::init::InitFlags::HAPTIC);
    crate::hints::reset(crate::hints::JOYSTICK_DIRECTINPUT);
}
