// Tests for the haptic front end.

use super::*;
use crate::init::{self, InitFlags};
use crate::test_support::TEST_LOCK;

#[test]
fn axes_hint_parsing() {
    let list = parse_axes_list("0x046d/0xc24f/2,0xffff/0xffff/1, 0x1/0x2/3");
    assert_eq!(
        list,
        vec![
            VidPidNaxes {
                vid: 0x046d,
                pid: 0xc24f,
                naxes: 2
            },
            VidPidNaxes {
                vid: 0xffff,
                pid: 0xffff,
                naxes: 1
            },
        ]
    );

    // %hx skips whitespace after the literal "0x", and truncates to 16 bits
    let list = parse_axes_list("0x 12345/0x2/ 7,0x3/0x4/5");
    assert_eq!(
        list,
        vec![
            VidPidNaxes {
                vid: 0x2345,
                pid: 2,
                naxes: 7
            },
            VidPidNaxes {
                vid: 3,
                pid: 4,
                naxes: 5
            },
        ]
    );

    assert!(parse_axes_list("").is_empty());
    assert!(parse_axes_list("046d/c24f/2").is_empty());
    assert!(parse_axes_list("0x046d/0xc24f").is_empty());
}

#[test]
fn hinted_axes() {
    let _l = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _lock = lock_joysticks();
    hints::set(
        hints::JOYSTICK_HAPTIC_AXES,
        "0xffff/0xffff/3,0x1234/0x5678/2",
    )
    .unwrap();
    assert_eq!(hinted_naxes(0x1234, 0x5678), Some(2));
    assert_eq!(hinted_naxes(0x1111, 0x2222), Some(3));
    hints::set(hints::JOYSTICK_HAPTIC_AXES, "0x1234/0x5678/2").unwrap();
    assert_eq!(hinted_naxes(0x1111, 0x2222), None);
    hints::reset(hints::JOYSTICK_HAPTIC_AXES);
    assert_eq!(hinted_naxes(0x1234, 0x5678), None);
}

#[test]
fn effect_types() {
    let sine = HapticEffect::Periodic(HapticPeriodic::default());
    assert_eq!(sine.effect_type(), HapticFeatures::SINE);
    let friction = HapticEffect::Condition(HapticCondition {
        kind: HapticConditionKind::Friction,
        ..Default::default()
    });
    assert_eq!(friction.effect_type(), HapticFeatures(1 << 10));
    assert_eq!(
        HapticEffect::LeftRight(HapticLeftRight::default()).effect_type(),
        HapticFeatures(1 << 11)
    );
    assert_eq!(
        HapticEffect::Custom(HapticCustom::default()).effect_type(),
        HapticFeatures(1 << 15)
    );
    assert!((HapticFeatures::SINE | HapticFeatures::GAIN).contains(HapticFeatures::GAIN));
}

#[test]
fn dummy_driver() {
    let _l = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    init::init_subsystem(InitFlags::HAPTIC).unwrap();
    assert!(haptics().is_empty());
    assert_eq!(
        Haptic::open(1).unwrap_err().message(),
        "Haptic device 1 not found"
    );
    assert_eq!(
        Haptic::open(0).unwrap_err().message(),
        "Haptic device 0 not found"
    );
    assert!(haptic_name_for_id(1).is_err());
    assert!(!is_mouse_haptic());
    assert_eq!(
        Haptic::open_from_mouse().unwrap_err().message(),
        "Haptic: Mouse isn't a haptic device."
    );
    assert!(Haptic::from_id(1).is_none());
    init::quit_subsystem(InitFlags::HAPTIC);
}

#[test]
fn joystick_is_not_haptic() {
    let _l = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    use crate::joystick::VirtualJoystickDesc;

    init::init_subsystem(InitFlags::JOYSTICK | InitFlags::HAPTIC).unwrap();
    let id = crate::joystick::attach_virtual_joystick(VirtualJoystickDesc {
        naxes: 2,
        nbuttons: 2,
        name: Some("Haptic test".into()),
        ..Default::default()
    })
    .unwrap();
    let joystick = Joystick::open(id).unwrap();
    assert!(!is_joystick_haptic(&joystick));
    assert_eq!(
        Haptic::open_from_joystick(&joystick).unwrap_err().message(),
        "Haptic: Joystick isn't a haptic device."
    );
    drop(joystick);
    crate::joystick::detach_virtual_joystick(id).unwrap();
    init::quit_subsystem(InitFlags::JOYSTICK | InitFlags::HAPTIC);
}
