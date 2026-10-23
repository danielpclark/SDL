// Tests for the MFi (GameController) joystick driver.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

use super::*;
use crate::core::apple::objc::{autoreleasepool, class, msg_send, Id};

/// The framework constants upstream compares with, against their values in
/// the SDK (printed by a C program built with it).
#[test]
fn sdk_constants() {
    assert!(load_frameworks(), "GameController and CoreHaptics load");
    assert!(!class!(GCController).is_null());
    assert!(!class!(CHHapticEngine).is_null());
    let expect = [
        (gc_input_button_a(), "Button A"),
        (gc_input_button_b(), "Button B"),
        (gc_input_button_x(), "Button X"),
        (gc_input_button_y(), "Button Y"),
        (gc_input_left_shoulder(), "Left Shoulder"),
        (gc_input_right_shoulder(), "Right Shoulder"),
        (gc_input_left_trigger(), "Left Trigger"),
        (gc_input_right_trigger(), "Right Trigger"),
        (gc_input_left_thumbstick(), "Left Thumbstick"),
        (gc_input_right_thumbstick(), "Right Thumbstick"),
        (gc_input_left_thumbstick_button(), "Left Thumbstick Button"),
        (
            gc_input_right_thumbstick_button(),
            "Right Thumbstick Button",
        ),
        (gc_input_button_menu(), "Button Menu"),
        (gc_input_button_options(), "Button Options"),
        (gc_input_direction_pad(), "Direction Pad"),
        (gc_input_dual_shock_touchpad_button(), "Touchpad Button"),
        (gc_input_dual_shock_touchpad_one(), "Touchpad 1"),
        (gc_input_dual_shock_touchpad_two(), "Touchpad 2"),
        (gc_input_xbox_paddle_one(), "Paddle 1"),
        (gc_input_xbox_paddle_two(), "Paddle 2"),
        (gc_input_xbox_paddle_three(), "Paddle 3"),
        (gc_input_xbox_paddle_four(), "Paddle 4"),
        (
            gc_controller_did_connect_notification(),
            "GCControllerDidConnectNotification",
        ),
        (
            gc_controller_did_disconnect_notification(),
            "GCControllerDidDisconnectNotification",
        ),
        (gc_haptics_locality_handles(), "Handles"),
        (gc_haptics_locality_left_handle(), "Left Handle"),
        (gc_haptics_locality_right_handle(), "Right Handle"),
        (gc_haptics_locality_triggers(), "Triggers"),
        (gc_haptics_locality_left_trigger(), "Left Trigger"),
        (gc_haptics_locality_right_trigger(), "Right Trigger"),
        (
            ch_haptic_event_parameter_id_haptic_intensity(),
            "HapticIntensity",
        ),
        (ch_haptic_event_type_haptic_continuous(), "HapticContinuous"),
        (
            ch_haptic_dynamic_parameter_id_haptic_intensity_control(),
            "HapticIntensityControl",
        ),
    ];
    for (value, expected) in expect {
        assert_eq!(constant_str(value), expected);
    }
    assert_eq!(gc_haptic_duration_infinite(), 1_000_000.0);
    assert_eq!(size_of::<GCRotationRate>(), 24);
    assert_eq!(size_of::<GCAcceleration>(), 24);
}

/// `SDL_sscanf(category, "Siri Remote (%i%*s Generation)", &n)`, against
/// the C library's sscanf().
#[test]
fn siri_remote_generation_like_sscanf() {
    let inputs = [
        "Siri Remote (2nd Generation)",
        "Siri Remote (1st Generation)",
        "Siri Remote (3rd Generation)",
        "Siri Remote",
        "Siri Remote ()",
        "Siri Remote (x)",
        "Siri  Remote(  12th Generation)",
        "SiriRemote (4th Generation)",
        "Siri Remote (0x1f)",
        "Siri Remote (010th Generation)",
        "Siri Remote (-3rd)",
        "Siri Remote (+7",
        "Siri Remote (0x)",
        "Siri Remote (09)",
        "Siri Remote\t(5)",
    ];
    for input in inputs {
        let c_input = std::ffi::CString::new(input).unwrap();
        let mut n: libc::c_int = 1;
        // SAFETY: a NUL-terminated input and format, one int to store.
        let converted = unsafe {
            libc::sscanf(
                c_input.as_ptr(),
                c"Siri Remote (%i%*s Generation)".as_ptr(),
                &mut n as *mut libc::c_int,
            )
        };
        let expected = if converted >= 1 { Some(n) } else { None };
        assert_eq!(scan_siri_remote_generation(input), expected, "{input:?}");
    }
}

/// A device with the flags of a profile line.
fn device_with_flags(flags: &str) -> MfiDevice {
    let mut device = MfiDevice::default();
    for flag in flags.split(',').filter(|f| !f.is_empty()) {
        match flag {
            "joycon_pair" => device.is_switch_joycon_pair = true,
            "backbone" => device.is_backbone_one = true,
            "nintendo" => device.has_nintendo_buttons = true,
            "joyconL" => device.is_switch_joycon_l = true,
            "joyconR" => device.is_switch_joycon_r = true,
            _ => {
                let n = flag.strip_prefix("siri=").expect("profile flag");
                device.is_siri_remote = n.parse().unwrap();
            }
        }
    }
    device
}

/// The keys sorted as upstream sorts them
/// (`sortedArrayUsingSelector:@selector(caseInsensitiveCompare:)`).
fn sorted_case_insensitive(names: &[&str]) -> Vec<String> {
    autoreleasepool(|| unsafe {
        let strings: Vec<StrongPtr> = names.iter().map(|n| ns_string(n)).collect();
        let ptrs: Vec<Id> = strings.iter().map(StrongPtr::get).collect();
        let array: Id = msg_send![class!(NSArray), arrayWithObjects: ptrs.as_ptr() count: ptrs.len() as NSUInteger];
        let sorted: Id = msg_send![array, sortedArrayUsingSelector: sel!(caseInsensitiveCompare:)];
        ns_array_items(sorted)
            .into_iter()
            .map(|s| string_from_ns(s).unwrap())
            .collect()
    })
}

fn format_mapping(label: &str, m: &GamepadMapping) -> String {
    let mut out = format!("{label} mapping:");
    let fields: [(&str, &InputMapping); 32] = [
        ("a", &m.a),
        ("b", &m.b),
        ("x", &m.x),
        ("y", &m.y),
        ("back", &m.back),
        ("guide", &m.guide),
        ("start", &m.start),
        ("leftstick", &m.leftstick),
        ("rightstick", &m.rightstick),
        ("leftshoulder", &m.leftshoulder),
        ("rightshoulder", &m.rightshoulder),
        ("dpup", &m.dpup),
        ("dpdown", &m.dpdown),
        ("dpleft", &m.dpleft),
        ("dpright", &m.dpright),
        ("misc1", &m.misc1),
        ("misc2", &m.misc2),
        ("misc3", &m.misc3),
        ("misc4", &m.misc4),
        ("misc5", &m.misc5),
        ("misc6", &m.misc6),
        ("right_paddle1", &m.right_paddle1),
        ("left_paddle1", &m.left_paddle1),
        ("right_paddle2", &m.right_paddle2),
        ("left_paddle2", &m.left_paddle2),
        ("leftx", &m.leftx),
        ("lefty", &m.lefty),
        ("rightx", &m.rightx),
        ("righty", &m.righty),
        ("lefttrigger", &m.lefttrigger),
        ("righttrigger", &m.righttrigger),
        ("touchpad", &m.touchpad),
    ];
    for (name, mapping) in fields {
        if mapping.kind != MappingKind::None {
            out += &format!(
                " {name}:{}{}{}{}{}",
                if mapping.kind == MappingKind::Button {
                    'b'
                } else {
                    'a'
                },
                mapping.target,
                if mapping.axis_reversed { "~" } else { "" },
                if mapping.half_axis_positive { "+" } else { "" },
                if mapping.half_axis_negative { "-" } else { "" },
            );
        }
    }
    out
}

/// The element selection of `IOS_AddMFIJoystickDevice()` (with
/// `ElementAlreadyHandled()`) and `IOS_JoystickGetGamepadMapping()` over
/// controller profiles, against upstream's code run on them (a copy of it
/// in an Objective-C program, with the element classes as kind letters:
/// `a` analog axis, `B` analog button, `b` digital button, `d` direction
/// pad).
#[test]
fn element_selection_and_mapping_match_upstream() {
    let profiles = include_str!("../testdata/mfi_profiles.txt");
    let expected = include_str!("../testdata/mfi_mapping.txt");
    let names = input_names();
    let mut out = Vec::new();
    for line in profiles.lines() {
        let mut parts = line.split('|');
        let (label, flags, spec) = (
            parts.next().unwrap(),
            parts.next().unwrap(),
            parts.next().unwrap(),
        );
        let mut device = device_with_flags(flags);
        let elements: Vec<(&str, char)> = spec
            .split(';')
            .filter(|e| !e.is_empty())
            .map(|e| {
                let (name, kind) = e.rsplit_once(':').unwrap();
                (name, kind.chars().next().unwrap())
            })
            .collect();
        let kind_of = |name: &str| elements.iter().find(|e| e.0 == name).unwrap().1;
        let sorted = sorted_case_insensitive(&elements.iter().map(|e| e.0).collect::<Vec<_>>());
        let entries: Vec<(&str, ElementKind, bool)> = sorted
            .iter()
            .map(|name| {
                let (kind, analog) = match kind_of(name) {
                    'a' => (ElementKind::Axis, true),
                    'B' => (ElementKind::Button, true),
                    'b' => (ElementKind::Button, false),
                    _ => (ElementKind::Other, false),
                };
                (name.as_str(), kind, analog)
            })
            .collect();
        let has = |name: &str| elements.iter().any(|e| e.0 == name);
        let (axes, buttons) = select_elements(&device, &entries, &has);
        device.axes = axes.iter().map(|&i| ElementKey::new(&sorted[i])).collect();
        device.buttons = buttons
            .iter()
            .map(|&i| ElementKey::new(&sorted[i]))
            .collect();

        let list = |keys: &[ElementKey]| -> String {
            keys.iter().map(|k| format!(" [{}]", k.name)).collect()
        };
        out.push(format!("{label} axes:{}", list(&device.axes)));
        out.push(format!("{label} buttons:{}", list(&device.buttons)));
        out.push(format_mapping(label, &gamepad_mapping_for(&device, names)));
    }
    let expected: Vec<&str> = expected.lines().collect();
    assert_eq!(out.len(), expected.len());
    for (actual, expected) in out.iter().zip(expected) {
        assert_eq!(actual, expected);
    }
}

/// The GUID signature: the CRC-16 of the name, then of each axis and
/// button name, as upstream chains `SDL_crc16()`.
#[test]
fn element_signature_chains_crc16() {
    let device = MfiDevice {
        name: "Xbox Series X Controller".to_owned(),
        axes: vec![ElementKey::new("Left Thumbstick X Axis")],
        buttons: vec![ElementKey::new("Button A"), ElementKey::new("Button B")],
        ..MfiDevice::default()
    };
    let mut crc = crate::stdlib::crc16(0, b"Xbox Series X Controller");
    crc = crate::stdlib::crc16(crc, b"Left Thumbstick X Axis");
    crc = crate::stdlib::crc16(crc, b"Button A");
    crc = crate::stdlib::crc16(crc, b"Button B");
    assert_eq!(element_signature(&device), crc);
    assert_eq!(element_signature(&MfiDevice::default()), 0);
}

/// The rumble results: `result &= a; result &= b` keeps the last error.
#[test]
fn rumble_results_keep_the_last_error() {
    assert!(and_results(Ok(()), Ok(())).is_ok());
    let e = |m: &'static str| Err(Error::new(m));
    assert_eq!(and_results(e("a"), Ok(())).unwrap_err().message(), "a");
    assert_eq!(and_results(Ok(()), e("b")).unwrap_err().message(), "b");
    assert_eq!(and_results(e("a"), e("b")).unwrap_err().message(), "b");
}

/// The driver starts (there being no controller here is fine, and nothing
/// waits for GameController's discovery), is placed after the IOKit driver,
/// and quits.
#[test]
fn driver_starts_and_quits() {
    use crate::joystick::{
        lock_joysticks, DARWIN_DRIVER_INDEX, JOYSTICK_DRIVERS, MFI_DRIVER_INDEX,
    };
    let _l = crate::test_support::test_lock();
    assert_eq!(MFI_DRIVER_INDEX, DARWIN_DRIVER_INDEX + 1);
    assert!(std::ptr::eq(
        JOYSTICK_DRIVERS[MFI_DRIVER_INDEX] as *const dyn JoystickDriver as *const u8,
        &MFI_JOYSTICK_DRIVER as *const MfiJoystickDriver as *const u8
    ));
    crate::init::init_subsystem(crate::init::InitFlags::JOYSTICK).unwrap();
    let count = {
        let _lock = lock_joysticks();
        MFI_JOYSTICK_DRIVER.detect();
        let count = MFI_JOYSTICK_DRIVER.count();
        for i in 0..count {
            println!(
                "MFi: {:?} {}",
                MFI_JOYSTICK_DRIVER.device_name(i),
                MFI_JOYSTICK_DRIVER.device_guid(i)
            );
            assert!(MFI_JOYSTICK_DRIVER.gamepad_mapping(i).is_some());
        }
        assert!(MFI_JOYSTICK_DRIVER.gamepad_mapping(count).is_none());
        assert_eq!(
            MFI_JOYSTICK_DRIVER.device_name(count).as_deref(),
            Some("Unknown")
        );
        assert_eq!(MFI_JOYSTICK_DRIVER.device_guid(count), Guid::ZERO);
        assert_eq!(MFI_JOYSTICK_DRIVER.device_player_index(count), -1);
        count
    };
    println!("MFi controllers: {count}");
    assert!(lock_state().connect_observer.get() != NIL || !available!(macos(11, 0)));
    crate::init::quit_subsystem(crate::init::InitFlags::JOYSTICK);
    assert!(lock_state().devices.is_empty());
    assert!(lock_state().connect_observer.is_nil());

    // With the hint off, the driver doesn't start
    hints::set(hints::JOYSTICK_MFI, "0").unwrap();
    crate::init::init_subsystem(crate::init::InitFlags::JOYSTICK).unwrap();
    assert!(lock_state().connect_observer.is_nil());
    crate::init::quit_subsystem(crate::init::InitFlags::JOYSTICK);
    hints::reset(hints::JOYSTICK_MFI);
}

/// Hardware: what GameController reports for the connected controllers
/// through this driver alone (HIDAPI and IOKit off): names, GUIDs, the
/// generated gamepad mapping, SF Symbols names, sensors, LED, battery and
/// rumble. GameController reports controllers it discovers on the main
/// run loop, which a test thread doesn't run, so only the ones it already
/// knew when the driver started are there. There may be none.
#[test]
#[ignore = "hardware: lists the connected MFi controllers"]
fn hardware_mfi_controllers() {
    use crate::joystick::gamepad::{Gamepad, GamepadAxis, GamepadButton};
    use crate::joystick::{joysticks, Joystick};
    let _l = crate::test_support::test_lock();
    hints::set(hints::JOYSTICK_HIDAPI, "0").unwrap();
    hints::set(hints::JOYSTICK_IOKIT, "0").unwrap();
    crate::init::init_subsystem(crate::init::InitFlags::GAMEPAD).unwrap();
    let start = std::time::Instant::now();
    while start.elapsed() < std::time::Duration::from_secs(1) {
        crate::events::pump();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let ids = joysticks();
    println!("MFi controllers: {}", ids.len());
    if ids.is_empty() {
        crate::test_support::skip("controller", "no MFi controller connected");
    }
    for id in ids {
        let joystick = Joystick::open(id).unwrap();
        println!(
            "{id}: {:?} {} {:04x}:{:04x}, {} axes, {} buttons, {} hats",
            joystick.name().unwrap().unwrap_or_default(),
            joystick.guid(),
            joystick.vendor(),
            joystick.product(),
            joystick.num_axes().unwrap(),
            joystick.num_buttons().unwrap(),
            joystick.num_hats().unwrap(),
        );
        if let Ok(gamepad) = Gamepad::open(id) {
            println!("  mapping: {:?}", gamepad.mapping().ok());
            println!(
                "  SF Symbols: A {:?}, Start {:?}, d-pad up {:?}, left stick {:?}, right trigger {:?}",
                gamepad.apple_sf_symbols_name_for_button(GamepadButton::South),
                gamepad.apple_sf_symbols_name_for_button(GamepadButton::Start),
                gamepad.apple_sf_symbols_name_for_button(GamepadButton::DpadUp),
                gamepad.apple_sf_symbols_name_for_axis(GamepadAxis::LeftX),
                gamepad.apple_sf_symbols_name_for_axis(GamepadAxis::RightTrigger),
            );
        }
        println!("  power: {:?}", joystick.power_info());
        println!(
            "  LED: {:?}",
            joystick
                .set_led(0, 64, 255)
                .map_err(|e| e.message().to_owned())
        );
        println!(
            "  rumble: {:?}",
            joystick
                .rumble(0x4000, 0x4000, 250)
                .map_err(|e| e.message().to_owned())
        );
        println!(
            "  trigger rumble: {:?}",
            joystick
                .rumble_triggers(0x4000, 0x4000, 250)
                .map_err(|e| e.message().to_owned())
        );
    }
    crate::init::quit_subsystem(crate::init::InitFlags::GAMEPAD);
    hints::reset(hints::JOYSTICK_HIDAPI);
    hints::reset(hints::JOYSTICK_IOKIT);
}
