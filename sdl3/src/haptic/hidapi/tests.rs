// Tests for the HIDAPI haptic layer, and the haptic front end on it.
// Copyright (C) 2025 Katharine Chui <katharine.chui@gmail.com>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! A Logitech wheel on a fake HID device ([`FakeWheel`]) gives the HIDAPI
//! joystick driver a joystick; its haptic is opened from it and the
//! effect thread's commands arrive at the fake device through the
//! joystick, as on hardware.

use std::time::{Duration, Instant};

use super::lg4ff::tests::frontend_calls;
use super::*;
use crate::haptic::{Haptic, HapticConditionKind};
use crate::haptic::{
    HapticCondition, HapticConstant, HapticDirection, HapticDirectionType, HapticLeftRight,
    HAPTIC_INFINITY,
};
use crate::init::{self, InitFlags};
use crate::joystick::hidapi::lg4ff::tests::FakeWheel;
use crate::joystick::hidapi::lg4ff::{
    USB_DEVICE_ID_LOGITECH_G29_WHEEL, USB_DEVICE_ID_LOGITECH_WHEEL,
};
use crate::joystick::usb_ids::USB_VENDOR_LOGITECH;
use crate::joystick::{lock_joysticks, VirtualJoystickDesc};

const STEERING: HapticDirection = HapticDirection {
    kind: HapticDirectionType::SteeringAxis,
    dir: [0; 3],
};

/// A constant force along the wheel.
fn constant(length: u32, level: i16) -> HapticEffect {
    HapticEffect::Constant(HapticConstant {
        direction: STEERING,
        length,
        level,
        ..Default::default()
    })
}

/// A spring towards the center.
fn spring() -> HapticEffect {
    HapticEffect::Condition(HapticCondition {
        kind: HapticConditionKind::Spring,
        direction: STEERING,
        length: HAPTIC_INFINITY,
        right_sat: [0xffff; 3],
        left_sat: [0xffff; 3],
        right_coeff: [0x4000; 3],
        left_coeff: [0x4000; 3],
        ..Default::default()
    })
}

/// A virtual joystick (not the HIDAPI driver's), detached on drop.
struct Virtual(crate::events::JoystickID);

impl Virtual {
    fn attach() -> Virtual {
        let id = crate::joystick::attach_virtual_joystick(VirtualJoystickDesc {
            naxes: 1,
            nbuttons: 1,
            name: Some("Not a wheel".into()),
            ..Default::default()
        })
        .unwrap();
        Virtual(id)
    }
}

impl Drop for Virtual {
    fn drop(&mut self) {
        let _ = crate::joystick::detach_virtual_joystick(self.0);
    }
}

/// Wait until the wheel was sent `line` (by the effect thread); the
/// commands until then.
fn wait_for(wheel: &FakeWheel, line: &str) -> Vec<String> {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut writes = Vec::new();
    loop {
        writes.append(&mut wheel.take_writes());
        if writes.iter().any(|w| w == line) {
            return writes;
        }
        assert!(Instant::now() < deadline, "no {line:?} in {writes:?}");
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// The lines of a call of the transcript's `frontend` part, without the
/// constant force slot's commands (the effect thread's, which the C
/// harness doesn't run).
fn without_thread_writes(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .filter(|l| !l.starts_with("write 1c "))
        .cloned()
        .collect()
}

/// `name`'s call of the transcript's `frontend` part.
fn expected_call(name: &str) -> Vec<String> {
    frontend_calls()
        .into_iter()
        .find(|(n, _)| n == name)
        .unwrap_or_else(|| panic!("no call {name}"))
        .1
}

/// `SDL_HIDAPI_JoystickIsHaptic()` and `SDL_HIDAPI_HapticOpenFromJoystick()`
/// on a joystick that can't open, as the transcript prints them.
fn open_lines(joystick: &Joystick, open: bool) -> Vec<String> {
    let _lock = lock_joysticks();
    let mut lines = vec![format!("haptic {}", u8::from(joystick_is_haptic(joystick)))];
    if open {
        let mut haptic = HapticData::new();
        let result = open_from_joystick(&mut haptic, joystick);
        if let Err(e) = &result {
            lines.push(format!("error {}", e.message()));
        }
        lines.push(format!("result {}", u8::from(result.is_ok())));
        assert!(!is_hidapi(&haptic));
    }
    lines
}

/// The `SDL_HIDAPI_*` functions, as the transcript's `frontend` part calls
/// them.
#[test]
fn hidapi_layer() {
    let _l = crate::test_support::test_lock();
    init::init_subsystem(InitFlags::JOYSTICK | InitFlags::HAPTIC).unwrap();

    // A joystick of another driver
    let other = Virtual::attach();
    let other_joystick = Joystick::open(other.0).unwrap();
    assert_eq!(
        open_lines(&other_joystick, true),
        expected_call("nonhidapi")
    );

    // A HIDAPI joystick of another vendor, or an unknown wheel
    for (name, vendor, product) in [
        ("vendor", 0x046e, USB_DEVICE_ID_LOGITECH_G29_WHEEL),
        ("product", USB_VENDOR_LOGITECH, 0xc262),
    ] {
        let wheel = FakeWheel::connect(vendor, product, 0x1350);
        let joystick = Joystick::open(wheel.id).unwrap();
        assert_eq!(open_lines(&joystick, name == "vendor"), expected_call(name));
        drop(joystick);
    }

    // A G29
    let wheel = FakeWheel::connect(
        USB_VENDOR_LOGITECH,
        USB_DEVICE_ID_LOGITECH_G29_WHEEL,
        0x1350,
    );
    assert_eq!(wheel.take_writes(), ["write f5 00 00 00 00 00 00"]);
    let joystick = Joystick::open(wheel.id).unwrap();
    let mut haptic = HapticData::new();
    let mut lines = Vec::new();
    {
        let _lock = lock_joysticks();
        lines.push(format!(
            "haptic {}",
            u8::from(joystick_is_haptic(&joystick))
        ));
        let result = open_from_joystick(&mut haptic, &joystick);
        lines.append(&mut wheel.take_writes());
        lines.push(format!("result {}", u8::from(result.is_ok())));
        lines.push(format!(
            "instance {} effects {} playing {} supported {} axes {} hidapi {}",
            haptic.instance_id,
            haptic.effects.len(),
            haptic.nplaying,
            haptic.supported.0,
            haptic.naxes,
            u8::from(is_hidapi(&haptic))
        ));
        lines.push(format!(
            "same {}",
            u8::from(joystick_same_haptic(&haptic, &joystick))
        ));
        lines.push(format!(
            "other {}",
            u8::from(joystick_same_haptic(&haptic, &other_joystick))
        ));
    }
    let id = new_effect(&mut haptic, &constant(100, 1000)).unwrap();
    let stored = haptic.effects[id as usize].effect.as_ref().unwrap();
    lines.push(format!("effect {id} type {}", stored.effect_type().0));
    let ok = |result: Result<()>, lines: &mut Vec<String>| {
        if let Err(e) = &result {
            lines.push(format!("error {}", e.message()));
        }
        u8::from(result.is_ok())
    };
    let run = ok(run_effect(&mut haptic, id, 1), &mut lines);
    lines.push(format!("run {run}"));
    lines.push(format!(
        "status {}",
        u8::from(effect_status(&mut haptic, id))
    ));
    let gain = ok(set_gain(&mut haptic, 100), &mut lines);
    lines.push(format!("gain {gain}"));
    let autocenter = ok(set_autocenter(&mut haptic, 0), &mut lines);
    lines.append(&mut wheel.take_writes());
    lines.push(format!("autocenter {autocenter}"));
    let pause = ok(pause(&mut haptic), &mut lines);
    lines.push(format!("pause {pause}"));
    let resume = ok(resume(&mut haptic), &mut lines);
    lines.push(format!("resume {resume}"));
    let stop = ok(stop_effect(&mut haptic, id), &mut lines);
    lines.push(format!("stop {stop}"));
    let stopall = ok(stop_all(&mut haptic), &mut lines);
    lines.push(format!("stopall {stopall}"));
    destroy_effect(&mut haptic, id);
    lines.push(format!(
        "destroyed status {}",
        u8::from(effect_status(&mut haptic, id))
    ));
    close(&mut haptic);
    lines.push(format!("closed hidapi {}", u8::from(is_hidapi(&haptic))));
    assert!(haptic.effects.is_empty());
    assert_eq!(
        without_thread_writes(&lines),
        without_thread_writes(&expected_call("open"))
    );

    drop(joystick);
    drop(wheel);
    drop(other_joystick);
    drop(other);
    init::quit_subsystem(InitFlags::JOYSTICK | InitFlags::HAPTIC);
}

/// The haptic API on a wheel: opening it from its joystick, its
/// capabilities, and effects played by the effect thread.
#[test]
fn wheel_through_the_front_end() {
    let _l = crate::test_support::test_lock();
    init::init_subsystem(InitFlags::JOYSTICK | InitFlags::HAPTIC).unwrap();

    let wheel = FakeWheel::connect(
        USB_VENDOR_LOGITECH,
        USB_DEVICE_ID_LOGITECH_G29_WHEEL,
        0x1350,
    );
    // (the joystick driver turns the autocenter spring off)
    assert_eq!(wheel.take_writes(), ["write f5 00 00 00 00 00 00"]);
    let joystick = Joystick::open(wheel.id).unwrap();
    assert!(crate::haptic::is_joystick_haptic(&joystick));

    // Opening it sets up the slots (with upstream's repeated fixed loop
    // command), the gain and no autocenter
    let haptic = Haptic::open_from_joystick(&joystick).unwrap();
    assert_eq!(
        wheel.take_writes(),
        [
            "write 0d 00 00 00 00 00 00",
            "write 0d 00 00 00 00 00 00",
            "write 0d 00 00 00 00 00 00",
            "write 0d 00 00 00 00 00 00",
            "write 0d 00 00 00 00 00 00",
            "write f5 00 00 00 00 00 00",
        ]
    );
    assert_eq!(haptic.id(), 255);
    assert_eq!(haptic.name().unwrap(), None);
    assert_eq!(haptic.max_effects().unwrap(), 16);
    assert_eq!(haptic.max_effects_playing().unwrap(), 16);
    assert_eq!(haptic.num_axes().unwrap(), 1);
    let features = haptic.features().unwrap();
    // (every effect but inertia, left/right and custom; gain, autocenter
    // and status)
    assert_eq!(features.0, 0x7_05ff);
    assert!(features.contains(HapticFeatures::FRICTION | HapticFeatures::STATUS));
    assert!(!features.intersects(HapticFeatures::PAUSE | HapticFeatures::LEFTRIGHT));
    assert!(haptic.rumble_supported());

    // The same joystick gives the same device
    let again = Haptic::open_from_joystick(&joystick).unwrap();
    assert_eq!(again.id(), 255);
    drop(again);
    assert_eq!(Haptic::from_id(255).map(|h| h.id()), Some(255));
    assert!(haptic.features().is_ok());

    // A constant force, played and stopped by the thread
    let id = haptic
        .create_effect(&constant(HAPTIC_INFINITY, 0x4000))
        .unwrap();
    assert_eq!(id, 0);
    assert!(!haptic.effect_status(id).unwrap());
    haptic.run_effect(id, 1).unwrap();
    assert!(haptic.effect_status(id).unwrap());
    wait_for(&wheel, "write 1c 00 c0 00 00 00 00");
    haptic.stop_effect(id).unwrap();
    assert!(!haptic.effect_status(id).unwrap());
    wait_for(&wheel, "write 1c 00 80 00 00 00 00");

    // Updated: the type can't change
    assert_eq!(
        haptic.update_effect(id, &spring()).unwrap_err().message(),
        "Haptic: Updating effect type is illegal."
    );
    haptic
        .update_effect(id, &constant(HAPTIC_INFINITY, -0x4000))
        .unwrap();
    haptic.run_effect(id, 1).unwrap();
    wait_for(&wheel, "write 1c 00 40 00 00 00 00");
    haptic.stop_effects().unwrap();
    assert!(!haptic.effect_status(id).unwrap());
    wait_for(&wheel, "write 1c 00 80 00 00 00 00");

    // A spring, destroyed while it plays
    let spring_id = haptic.create_effect(&spring()).unwrap();
    assert_eq!(spring_id, 1);
    haptic.run_effect(spring_id, 1).unwrap();
    // (deadband 1024 to 1024, coefficients 0x4000 - 2048 scaled to 7, the
    // saturation at the default 30%)
    let writes = wait_for(&wheel, "write 21 0b 80 80 77 00 4c");
    assert!(
        writes.iter().all(|w| w.starts_with("write 21 ")),
        "{writes:?}"
    );
    haptic.destroy_effect(spring_id);
    assert!(!haptic.effect_status(spring_id).unwrap());
    assert_eq!(
        haptic.run_effect(spring_id, 1).unwrap_err().message(),
        "Bad effect id"
    );
    wait_for(&wheel, "write 23 00 00 00 00 00 00");

    // What the wheel doesn't do
    let leftright = HapticEffect::LeftRight(HapticLeftRight::default());
    assert!(!haptic.effect_supported(&leftright));
    assert_eq!(
        haptic.create_effect(&leftright).unwrap_err().message(),
        "Haptic: Effect not supported by haptic device."
    );
    assert_eq!(
        haptic.pause().unwrap_err().message(),
        "Haptic: Device does not support setting pausing."
    );
    haptic.resume().unwrap();
    assert_eq!(
        haptic.run_effect(-1, 1).unwrap_err().message(),
        "Bad effect id"
    );
    assert!(!haptic.effect_status(16).unwrap());

    // Gain and autocenter
    haptic.set_gain(50).unwrap();
    assert!(haptic.set_gain(101).is_err());
    haptic.set_autocenter(50).unwrap();
    assert_eq!(
        wheel.take_writes(),
        [
            "write f5 00 00 00 00 00 00",
            "write fe 0d 04 04 5f 00 00",
            "write 14 00 00 00 00 00 00",
        ]
    );

    // Rumble, a sine wave
    haptic.init_rumble().unwrap();
    haptic.play_rumble(0.5, 100).unwrap();
    haptic.stop_rumble().unwrap();

    // Every slot in use (0 and the rumble's are)
    let mut created = 0;
    let error = loop {
        match haptic.create_effect(&constant(10, 0)) {
            Ok(_) => created += 1,
            Err(e) => break e,
        }
    };
    assert_eq!(created, 14);
    assert_eq!(error.message(), "All effect slots in-use");

    // Closing it stops the thread and lets go of the joystick
    drop(haptic);
    wheel.take_writes();
    std::thread::sleep(Duration::from_millis(20));
    assert_eq!(wheel.take_writes(), Vec::<String>::new());
    assert!(Haptic::from_id(255).is_none());

    drop(joystick);
    drop(wheel);
    init::quit_subsystem(InitFlags::JOYSTICK | InitFlags::HAPTIC);
}

/// A Formula Force EX's autocenter, and the haptic closed by quitting.
#[test]
fn ffex_autocenter_and_quit() {
    let _l = crate::test_support::test_lock();
    init::init_subsystem(InitFlags::JOYSTICK | InitFlags::HAPTIC).unwrap();

    let wheel = FakeWheel::connect(USB_VENDOR_LOGITECH, USB_DEVICE_ID_LOGITECH_WHEEL, 0x2100);
    // (the joystick driver turns the autocenter off before it knows the
    // wheel is an FFEX, with the other wheels' command)
    assert_eq!(wheel.take_writes(), ["write f5 00 00 00 00 00 00"]);
    let joystick = Joystick::open(wheel.id).unwrap();
    let haptic = Haptic::open_from_joystick(&joystick).unwrap();
    wheel.take_writes();
    haptic.set_autocenter(100).unwrap();
    assert_eq!(wheel.take_writes(), ["write fe 03 00 00 5a 00 00"]);

    // Quitting closes the device; the handle is no longer valid
    crate::haptic::quit_haptics();
    assert!(haptic.features().is_err());
    drop(haptic);

    drop(joystick);
    drop(wheel);
    init::quit_subsystem(InitFlags::JOYSTICK | InitFlags::HAPTIC);
}
