// Tests for the Windows joystick driver (run under Wine on Linux).
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

use super::xinput::{battery_information, get_xinput_name, state_inputs, Input};
use super::*;
use crate::core::windows::xinput::*;
use crate::joystick::{joystick_guid_info, HAT_CENTERED, HAT_LEFTDOWN, HAT_RIGHTUP};
use crate::power::PowerState;

fn lock() -> std::sync::MutexGuard<'static, ()> {
    crate::test_support::TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

fn state(pad: XINPUT_GAMEPAD) -> XINPUT_STATE {
    XINPUT_STATE {
        dwPacketNumber: 1,
        Gamepad: pad,
    }
}

fn axes(inputs: &[Input]) -> Vec<i16> {
    inputs
        .iter()
        .filter_map(|i| match i {
            Input::Axis(_, v) => Some(*v),
            _ => None,
        })
        .collect()
}

fn buttons(inputs: &[Input]) -> Vec<bool> {
    inputs
        .iter()
        .filter_map(|i| match i {
            Input::Button(_, d) => Some(*d),
            _ => None,
        })
        .collect()
}

fn hat(inputs: &[Input]) -> u8 {
    match inputs.last() {
        Some(Input::Hat(0, v)) => *v,
        other => panic!("{other:?}"),
    }
}

#[test]
fn neutral_state() {
    let inputs = state_inputs(&state(XINPUT_GAMEPAD::default()));
    // Six axes, eleven buttons and a hat, in that order
    assert_eq!(inputs.len(), 6 + 11 + 1);
    for (i, input) in inputs[..6].iter().enumerate() {
        assert!(matches!(input, Input::Axis(a, _) if *a as usize == i));
    }
    for (i, input) in inputs[6..17].iter().enumerate() {
        assert!(matches!(input, Input::Button(b, false) if *b as usize == i));
    }
    // The Y axes are inverted with ~ (so 0 is -1) and the triggers rest at
    // the minimum
    assert_eq!(axes(&inputs), [0, -1, -32768, 0, -1, -32768]);
    assert_eq!(hat(&inputs), HAT_CENTERED);
}

#[test]
fn full_state() {
    let inputs = state_inputs(&state(XINPUT_GAMEPAD {
        wButtons: !(XINPUT_GAMEPAD_DPAD_DOWN | XINPUT_GAMEPAD_DPAD_LEFT),
        bLeftTrigger: 255,
        bRightTrigger: 128,
        sThumbLX: -32768,
        sThumbLY: 32767,
        sThumbRX: 32767,
        sThumbRY: -32768,
    }));
    assert_eq!(
        axes(&inputs),
        [
            -32768,
            -32768,
            32767,
            32767,
            32767,
            (128 * 257 - 32768) as i16
        ]
    );
    assert_eq!(buttons(&inputs), [true; 11]);
    assert_eq!(hat(&inputs), HAT_RIGHTUP);
}

#[test]
fn button_order() {
    // A, B, X, Y, LB, RB, BACK, START, LS, RS, GUIDE
    let masks = [
        XINPUT_GAMEPAD_A,
        XINPUT_GAMEPAD_B,
        XINPUT_GAMEPAD_X,
        XINPUT_GAMEPAD_Y,
        XINPUT_GAMEPAD_LEFT_SHOULDER,
        XINPUT_GAMEPAD_RIGHT_SHOULDER,
        XINPUT_GAMEPAD_BACK,
        XINPUT_GAMEPAD_START,
        XINPUT_GAMEPAD_LEFT_THUMB,
        XINPUT_GAMEPAD_RIGHT_THUMB,
        XINPUT_GAMEPAD_GUIDE,
    ];
    for (i, &mask) in masks.iter().enumerate() {
        let inputs = state_inputs(&state(XINPUT_GAMEPAD {
            wButtons: mask | XINPUT_GAMEPAD_DPAD_DOWN | XINPUT_GAMEPAD_DPAD_LEFT,
            ..Default::default()
        }));
        let down = buttons(&inputs);
        assert!(down[i], "button {i}");
        assert_eq!(down.iter().filter(|d| **d).count(), 1);
        assert_eq!(hat(&inputs), HAT_LEFTDOWN);
    }
}

#[test]
fn battery_levels() {
    let info = |ty, level| XINPUT_BATTERY_INFORMATION_EX {
        BatteryType: ty,
        BatteryLevel: level,
    };
    assert_eq!(
        battery_information(&info(BATTERY_TYPE_WIRED, BATTERY_LEVEL_FULL)),
        (PowerState::Charging, 100)
    );
    assert_eq!(
        battery_information(&info(BATTERY_TYPE_DISCONNECTED, BATTERY_LEVEL_FULL)),
        (PowerState::Unknown, -1)
    );
    assert_eq!(
        battery_information(&info(BATTERY_TYPE_UNKNOWN, BATTERY_LEVEL_LOW)),
        (PowerState::Unknown, -1)
    );
    // Alkaline, NiMH, ...
    for (level, percent) in [
        (BATTERY_LEVEL_EMPTY, 10),
        (BATTERY_LEVEL_LOW, 40),
        (BATTERY_LEVEL_MEDIUM, 70),
        (BATTERY_LEVEL_FULL, 100),
        (0x7F, 100),
    ] {
        assert_eq!(
            battery_information(&info(0x02, level)),
            (PowerState::OnBattery, percent)
        );
    }
    // (a zeroed structure, when XInputGetBatteryInformation is missing)
    assert_eq!(
        battery_information(&XINPUT_BATTERY_INFORMATION_EX::default()),
        (PowerState::Unknown, -1)
    );
}

#[test]
fn names() {
    assert_eq!(
        get_xinput_name(0, XINPUT_DEVSUBTYPE_GAMEPAD),
        "XInput Controller #1"
    );
    assert_eq!(
        get_xinput_name(3, XINPUT_DEVSUBTYPE_WHEEL),
        "XInput Wheel #4"
    );
    assert_eq!(
        get_xinput_name(1, XINPUT_DEVSUBTYPE_GUITAR_BASS),
        "XInput Guitar #2"
    );
    assert_eq!(
        get_xinput_name(2, XINPUT_DEVSUBTYPE_ARCADE_PAD),
        "XInput ArcadePad #3"
    );
    assert_eq!(get_xinput_name(0, 0x42), "XInput Device #1");
}

/// A device as AddXInputDevice() creates it.
fn device(userid: u8, sub_type: u8) -> JoyStickDeviceData {
    JoyStickDeviceData {
        guid: Guid::ZERO,
        joystickname: String::new(),
        send_add_event: false,
        n_instance_id: 100 + userid as u32,
        b_xinput_device: true,
        sub_type,
        xinput_user_id: userid,
        path: format!("XInput#{userid}"),
        steam_virtual_gamepad_slot: 0,
    }
}

#[test]
fn device_list_bookkeeping() {
    let _l = lock();
    // A device already known moves from the previous list to the new one
    let mut context = vec![
        device(0, XINPUT_DEVSUBTYPE_GAMEPAD),
        device(1, XINPUT_DEVSUBTYPE_GAMEPAD),
    ];
    let mut sys = Vec::new();
    let add = |userid, sub_type, context: &mut Vec<_>, sys: &mut Vec<_>| {
        super::xinput::add_xinput_device(userid, sub_type, context, sys)
    };
    add(1, XINPUT_DEVSUBTYPE_GAMEPAD, &mut context, &mut sys);
    assert_eq!(context.len(), 1);
    assert_eq!(sys.len(), 1);
    assert_eq!(sys[0].n_instance_id, 101);
    assert!(!sys[0].send_add_event);

    // The same slot with another subtype is a new device
    add(0, XINPUT_DEVSUBTYPE_WHEEL, &mut context, &mut sys);
    assert_eq!(context.len(), 1);
    assert_eq!(sys.len(), 2);
    let new = &sys[0];
    assert!(new.send_add_event);
    assert!(new.n_instance_id > 0);
    assert_eq!(new.path, "XInput#0");
    assert_eq!(
        (new.xinput_user_id, new.sub_type),
        (0, XINPUT_DEVSUBTYPE_WHEEL)
    );
    // Without XInputGetCapabilitiesEx() data, a generic XInput controller
    let (vendor, product, _, _) = joystick_guid_info(new.guid);
    assert_eq!(
        (vendor, product),
        (
            crate::joystick::USB_VENDOR_MICROSOFT,
            crate::joystick::USB_PRODUCT_XBOX360_XUSB_CONTROLLER
        )
    );
    assert_eq!(new.guid.0[14], b'x');
    assert_eq!(new.guid.0[15], XINPUT_DEVSUBTYPE_WHEEL);
    assert!(crate::joystick::is_joystick_xinput(new.guid));
    assert_eq!(
        Some(new.joystickname.clone()),
        crate::joystick::create_joystick_name(vendor, product, None, Some("XInput Wheel #1"))
    );

    // Unknown subtypes are skipped
    add(2, XINPUT_DEVSUBTYPE_UNKNOWN, &mut context, &mut sys);
    assert_eq!(sys.len(), 2);
}

#[test]
fn driver_without_controllers() {
    let _l = lock();
    for thread in ["1", "0"] {
        hints::set(hints::JOYSTICK_THREAD, thread).unwrap();
        crate::init::init_subsystem(crate::init::InitFlags::JOYSTICK).unwrap();
        {
            let _lock = crate::joystick::lock_joysticks();
            // Wine (and CI) have no XInput controllers
            assert_eq!(WINDOWS_JOYSTICK_DRIVER.count(), 0);
            assert_eq!(WINDOWS_JOYSTICK_DRIVER.device_instance_id(0), 0);
            assert_eq!(WINDOWS_JOYSTICK_DRIVER.device_name(0), None);
            assert!(!WINDOWS_JOYSTICK_DRIVER.is_device_present(0x045e, 0x02a1, 0, None));
            WINDOWS_JOYSTICK_DRIVER.detect();
            assert_eq!(
                JOYSTICK_THREAD_ENABLED.load(Ordering::Relaxed),
                thread == "1"
            );
        }
        // (stops the detection thread, which must not hang)
        crate::init::quit_subsystem(crate::init::InitFlags::JOYSTICK);
        assert!(JOYSTICK_THREAD.lock().unwrap().is_none());
    }
    hints::reset(hints::JOYSTICK_THREAD);
}

#[test]
fn device_change_flag() {
    let _l = lock();
    set_windows_device_changed();
    assert!(windows_device_changed());
    LAST_DEVICE_CHANGE.store(get_last_device_notification(), Ordering::Release);
    assert!(!windows_device_changed());
}
