// Tests for the IOKit joystick driver.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

use super::*;
use crate::joystick::{HAT_CENTERED, HAT_DOWN, HAT_LEFT, HAT_RIGHT, HAT_UP};

fn element(usage: u32, cookie: u32) -> RecElement {
    RecElement {
        element_ref: std::ptr::null_mut(),
        cookie,
        usage_page: K_HID_PAGE_GENERIC_DESKTOP,
        usage,
        min: 0,
        max: 255,
        min_report: 0,
        max_report: 255,
    }
}

#[test]
fn elements_sorted_by_usage_once() {
    let mut list = Vec::new();
    assert!(add_sorted(&mut list, element(K_HID_USAGE_GD_Y, 1)));
    assert!(add_sorted(&mut list, element(K_HID_USAGE_GD_X, 2)));
    assert!(add_sorted(&mut list, element(K_HID_USAGE_GD_RZ, 3)));
    // Equal usages go after the ones already there
    assert!(add_sorted(&mut list, element(K_HID_USAGE_GD_Y, 4)));
    // The same cookie isn't added twice
    assert!(!add_sorted(&mut list, element(K_HID_USAGE_GD_Z, 3)));
    let order: Vec<(u32, u32)> = list.iter().map(|e| (e.usage, e.cookie)).collect();
    assert_eq!(
        order,
        [
            (K_HID_USAGE_GD_X, 2),
            (K_HID_USAGE_GD_Y, 1),
            (K_HID_USAGE_GD_Y, 4),
            (K_HID_USAGE_GD_RZ, 3)
        ]
    );
}

#[test]
fn element_kinds() {
    use ElementKind::*;
    let gd = K_HID_PAGE_GENERIC_DESKTOP;
    let input_misc = K_IOHID_ELEMENT_TYPE_INPUT_MISC;
    assert_eq!(element_kind(input_misc, gd, K_HID_USAGE_GD_X), Some(Axis));
    assert_eq!(
        element_kind(K_IOHID_ELEMENT_TYPE_INPUT_AXIS, gd, K_HID_USAGE_GD_WHEEL),
        Some(Axis)
    );
    assert_eq!(
        element_kind(input_misc, gd, K_HID_USAGE_GD_HATSWITCH),
        Some(Hat)
    );
    assert_eq!(
        element_kind(input_misc, gd, K_HID_USAGE_GD_DPAD_LEFT),
        Some(Button)
    );
    assert_eq!(
        element_kind(input_misc, gd, K_HID_USAGE_GD_SYSTEM_MAIN_MENU),
        Some(Button)
    );
    assert_eq!(element_kind(input_misc, gd, K_HID_USAGE_GD_GAMEPAD), None);
    assert_eq!(
        element_kind(input_misc, K_HID_PAGE_SIMULATION, K_HID_USAGE_SIM_BRAKE),
        Some(Axis)
    );
    assert_eq!(element_kind(input_misc, K_HID_PAGE_SIMULATION, 0xB0), None);
    assert_eq!(
        element_kind(K_IOHID_ELEMENT_TYPE_INPUT_BUTTON, K_HID_PAGE_BUTTON, 1),
        Some(Button)
    );
    assert_eq!(
        element_kind(input_misc, K_HID_PAGE_CONSUMER, 0xCD),
        Some(Button)
    );
    // Outputs and features aren't read
    assert_eq!(
        element_kind(K_IOHID_ELEMENT_TYPE_OUTPUT, K_HID_PAGE_BUTTON, 1),
        None
    );
    assert_eq!(
        element_kind(K_IOHID_ELEMENT_TYPE_FEATURE, gd, K_HID_USAGE_GD_X),
        None
    );
    assert_eq!(element_kind(input_misc, 0xFF00, 1), None);
}

#[test]
fn hat_positions() {
    // 8 positions
    let eight: Vec<u8> = (0..9).map(|v| hat_position(v, 0, 7)).collect();
    assert_eq!(
        eight,
        [
            HAT_UP,
            HAT_RIGHTUP,
            HAT_RIGHT,
            HAT_RIGHTDOWN,
            HAT_DOWN,
            HAT_LEFTDOWN,
            HAT_LEFT,
            HAT_LEFTUP,
            HAT_CENTERED
        ]
    );
    assert_eq!(hat_position(15, 0, 7), HAT_CENTERED);
    // 4 positions, from a logical minimum of 1
    assert_eq!(hat_position(1, 1, 4), HAT_UP);
    assert_eq!(hat_position(2, 1, 4), HAT_RIGHT);
    assert_eq!(hat_position(3, 1, 4), HAT_DOWN);
    assert_eq!(hat_position(4, 1, 4), HAT_LEFT);
    assert_eq!(hat_position(0, 1, 4), HAT_CENTERED);
    // Other ranges are always centered
    assert_eq!(hat_position(0, 0, 14), HAT_CENTERED);
}

#[test]
fn calibrated_scaling() {
    // 0..255 to the joystick range
    assert_eq!(scale_calibrated(0, 0, 255.0, -32768, 32767), -32768);
    assert_eq!(scale_calibrated(128, 0, 255.0, -32768, 32767), 128);
    assert_eq!(scale_calibrated(255, 0, 255.0, -32768, 32767), 32767);
    // An empty range read so far leaves the value alone
    assert_eq!(scale_calibrated(77, 77, 0.0, -32768, 32767), 77);
    // (float arithmetic, truncated toward zero)
    assert_eq!(scale_calibrated(1, 0, 3.0, -32768, 32767), -10923);
    // The NACON Revolution X Unlimited's sticks
    assert_eq!(nacon_axis_value(32767), 0);
    assert_eq!(nacon_axis_value(0), -32767);
    assert_eq!(nacon_axis_value(-32768), 0);
    assert_eq!(nacon_axis_value(-1), 32767);
}

#[test]
fn steam_virtual_gamepad_slots() {
    let (ms, wired) = (USB_VENDOR_MICROSOFT, USB_PRODUCT_XBOX360_WIRED_CONTROLLER);
    assert_eq!(get_steam_virtual_gamepad_slot(ms, wired, "GamePad-3"), 2);
    assert_eq!(get_steam_virtual_gamepad_slot(ms, wired, "GamePad- 1"), 0);
    assert_eq!(get_steam_virtual_gamepad_slot(ms, wired, "GamePad-"), -1);
    assert_eq!(get_steam_virtual_gamepad_slot(ms, wired, "Controller"), -1);
    assert_eq!(get_steam_virtual_gamepad_slot(ms, 0x0b13, "GamePad-3"), -1);
}

#[test]
fn rumble_effect_data() {
    let mut effect = create_rumble_effect_data(0x7FFF);
    let e = effect.effect;
    assert_eq!(e.dw_size, 80);
    assert_eq!(e.dw_gain, 10000);
    assert_eq!(e.dw_flags, FFEFF_OBJECTOFFSETS | FFEFF_CARTESIAN);
    assert_eq!(e.dw_duration, 0xFFFF * 1000);
    assert_eq!(e.dw_trigger_button, FFEB_NOTRIGGER);
    assert_eq!(e.c_axes, 2);
    assert_eq!(e.cb_type_specific_params, 16);
    // The effect points into its own buffers
    let periodic = unsafe { *(e.lpv_type_specific_params as *const FFPERIODIC) };
    assert_eq!(periodic.dw_magnitude, 10000);
    assert_eq!(periodic.dw_period, 1000000);
    assert_eq!(unsafe { *e.rgdw_axes.add(1) }, 0);
    assert_eq!(unsafe { *e.rgl_direction.add(1) }, 0);
    effect.periodic_mut().dw_magnitude = convert_magnitude(0x4000);
    let periodic = unsafe { *(effect.effect.lpv_type_specific_params as *const FFPERIODIC) };
    assert_eq!(periodic.dw_magnitude, 5000);
    // Negative magnitudes wrap into the DWORD, as upstream's int does
    assert_eq!(convert_magnitude(-1), 0);
    assert_eq!(convert_magnitude(-0x7FFF), (-10000i32) as u32);
}

/// The run loop mode is one string for the process' life (the HID manager
/// keeps it without retaining it: a released one crashes the unschedule
/// at quit).
#[test]
fn run_loop_mode_is_a_constant() {
    let mode = joystick_runloop_mode();
    assert_eq!(mode, joystick_runloop_mode());
    assert_eq!(
        unsafe { string_from_cf(mode) }.as_deref(),
        Some("SDLJoystick")
    );
}

#[test]
fn product_names_fit_upstreams_buffer() {
    assert_eq!(truncate_product("Pad".to_owned()), "Pad");
    assert_eq!(truncate_product("x".repeat(300)).len(), 255);
    // (cut at a character boundary)
    let name = format!("{}é", "x".repeat(254));
    assert_eq!(truncate_product(name), "x".repeat(254));
}

/// The driver starts its HID manager, enumerates whatever controllers
/// there are (none is fine) and stops it again, through the joystick
/// subsystem; its devices have names, GUIDs and instance IDs.
#[test]
fn enumerate_through_the_joystick_subsystem() {
    let _l = crate::test_support::test_lock();
    crate::init::init_subsystem(crate::init::InitFlags::JOYSTICK).unwrap();
    assert!(!lock_state().hidman.is_null());
    let devices: Vec<(JoystickID, Option<String>, Guid)> = {
        let _lock = crate::joystick::lock_joysticks();
        DARWIN_JOYSTICK_DRIVER.detect();
        (0..DARWIN_JOYSTICK_DRIVER.count())
            .map(|i| {
                (
                    DARWIN_JOYSTICK_DRIVER.device_instance_id(i),
                    DARWIN_JOYSTICK_DRIVER.device_name(i),
                    DARWIN_JOYSTICK_DRIVER.device_guid(i),
                )
            })
            .collect()
    };
    if devices.is_empty() {
        println!("note: no IOKit joysticks");
    }
    for (id, name, guid) in &devices {
        assert_ne!(*id, 0);
        assert!(name.is_some());
        assert_ne!(*guid, Guid::ZERO);
    }
    {
        let _lock = crate::joystick::lock_joysticks();
        let n = DARWIN_JOYSTICK_DRIVER.count();
        assert_eq!(
            DARWIN_JOYSTICK_DRIVER.device_name(n).as_deref(),
            Some("UNKNOWN")
        );
        assert_eq!(DARWIN_JOYSTICK_DRIVER.device_instance_id(n), 0);
        assert_eq!(
            DARWIN_JOYSTICK_DRIVER.device_steam_virtual_gamepad_slot(n),
            -1
        );
        assert_eq!(DARWIN_JOYSTICK_DRIVER.device_path(0), None);
        assert!(!DARWIN_JOYSTICK_DRIVER.is_device_present(0x045e, 0x028e, 0, None));
    }
    crate::init::quit_subsystem(crate::init::InitFlags::JOYSTICK);
    assert!(lock_state().hidman.is_null());
    assert!(lock_state().devices.is_empty());

    // Disabled by its hint, the driver starts without a manager
    hints::set(hints::JOYSTICK_IOKIT, "0").unwrap();
    crate::init::init_subsystem(crate::init::InitFlags::JOYSTICK).unwrap();
    assert!(lock_state().hidman.is_null());
    crate::init::quit_subsystem(crate::init::InitFlags::JOYSTICK);
    hints::reset(hints::JOYSTICK_IOKIT);
}

fn driver_label(index: usize) -> String {
    use crate::joystick::{DARWIN_DRIVER_INDEX, HIDAPI_DRIVER_INDEX, VIRTUAL_DRIVER_INDEX};
    match index {
        HIDAPI_DRIVER_INDEX => "HIDAPI".to_owned(),
        DARWIN_DRIVER_INDEX => "IOKit".to_owned(),
        VIRTUAL_DRIVER_INDEX => "virtual".to_owned(),
        _ => format!("driver #{index}"),
    }
}

fn list_joysticks(what: &str, config: &[(&str, &str)]) {
    use crate::joystick::gamepad::{gamepad_name_for_id, gamepad_type_for_id, is_gamepad};
    use crate::joystick::{joystick_guid_for_id, joystick_name_for_id, JOYSTICK_DRIVERS};
    for &(name, value) in config {
        hints::set(name, value).unwrap();
    }
    crate::init::init_subsystem(crate::init::InitFlags::GAMEPAD).unwrap();
    let start = std::time::Instant::now();
    while start.elapsed() < std::time::Duration::from_secs(1) {
        crate::events::pump();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let per_driver: Vec<(usize, Vec<JoystickID>)> = {
        let _lock = crate::joystick::lock_joysticks();
        JOYSTICK_DRIVERS
            .iter()
            .enumerate()
            .map(|(i, d)| (i, (0..d.count()).map(|n| d.device_instance_id(n)).collect()))
            .collect()
    };
    let counts: Vec<String> = per_driver
        .iter()
        .map(|(index, ids)| format!("{} {}", driver_label(*index), ids.len()))
        .collect();
    println!("{what}: {}", counts.join(", "));
    for (index, ids) in per_driver {
        for id in ids {
            let name = joystick_name_for_id(id).ok().flatten().unwrap_or_default();
            let guid = joystick_guid_for_id(id);
            if is_gamepad(id) {
                let pad = gamepad_name_for_id(id).ok().flatten().unwrap_or_default();
                println!(
                    "  {}: {id}: {name:?} {guid}, gamepad {pad:?} ({:?})",
                    driver_label(index),
                    gamepad_type_for_id(id)
                );
            } else {
                println!("  {}: {id}: {name:?} {guid}", driver_label(index));
            }
        }
    }
    crate::init::quit_subsystem(crate::init::InitFlags::GAMEPAD);
    for &(name, _) in config {
        hints::reset(name);
    }
}

/// Hardware: the controllers the HIDAPI and IOKit drivers see, together
/// and each alone. There may be none; this lists them, it doesn't require
/// any.
#[test]
#[ignore = "hardware: lists the connected controllers"]
fn hardware_joysticks_per_driver() {
    let _l = crate::test_support::test_lock();
    list_joysticks("the default drivers", &[]);
    list_joysticks("HIDAPI alone", &[(hints::JOYSTICK_IOKIT, "0")]);
    list_joysticks("IOKit alone", &[(hints::JOYSTICK_HIDAPI, "0")]);
}

/// Hardware: input from a connected controller, through the default
/// drivers. It asks for a button press and a stick (or axis) movement and
/// waits up to 30 seconds for each, then tries a short rumble (which a
/// controller may not support).
#[test]
#[ignore = "hardware: needs a controller and someone to press its buttons"]
fn hardware_controller_input() {
    use crate::joystick::gamepad::{gamepad_name_for_id, gamepad_type_for_id, is_gamepad};
    use crate::joystick::{joysticks, Joystick, JOYSTICK_DRIVERS};
    use std::time::{Duration, Instant};
    let _l = crate::test_support::test_lock();
    crate::init::init_subsystem(crate::init::InitFlags::GAMEPAD).unwrap();
    let settle = Instant::now();
    while joysticks().is_empty() && settle.elapsed() < Duration::from_secs(3) {
        crate::events::pump();
        std::thread::sleep(Duration::from_millis(10));
    }
    let Some(&id) = joysticks().first() else {
        crate::init::quit_subsystem(crate::init::InitFlags::GAMEPAD);
        crate::test_support::skip("controller", "no controller connected");
        return;
    };
    let driver = {
        let _lock = crate::joystick::lock_joysticks();
        JOYSTICK_DRIVERS
            .iter()
            .position(|d| (0..d.count()).any(|n| d.device_instance_id(n) == id))
    };
    let joystick = Joystick::open(id).unwrap();
    let (axes, buttons, hats) = (
        joystick.num_axes().unwrap(),
        joystick.num_buttons().unwrap(),
        joystick.num_hats().unwrap(),
    );
    println!(
        "{:?} through {}: {} {:04x}:{:04x}, {axes} axes, {buttons} buttons, {hats} hats",
        joystick.name().unwrap().unwrap_or_default(),
        driver.map_or_else(|| "?".to_owned(), driver_label),
        joystick.guid(),
        joystick.vendor(),
        joystick.product(),
    );
    if is_gamepad(id) {
        println!(
            "gamepad {:?} ({:?})",
            gamepad_name_for_id(id).ok().flatten().unwrap_or_default(),
            gamepad_type_for_id(id)
        );
    } else {
        println!("no gamepad mapping");
    }

    let wait = |what: &str, done: &mut dyn FnMut() -> bool| {
        println!("{what} (30 seconds)");
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(30) {
            crate::events::pump();
            if done() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        false
    };
    let initial: Vec<i16> = (0..axes).map(|a| joystick.axis(a).unwrap()).collect();
    let mut pressed = None;
    assert!(
        wait("press and release any button", &mut || {
            for b in 0..buttons {
                let down = joystick.button(b).unwrap();
                match pressed {
                    None if down => {
                        println!("button {b} down");
                        pressed = Some(b);
                    }
                    Some(p) if p == b && !down => {
                        println!("button {b} up");
                        return true;
                    }
                    _ => {}
                }
            }
            false
        }),
        "no button press and release arrived"
    );
    if axes > 0 {
        assert!(
            wait("move a stick (or any axis) all the way", &mut || {
                (0..axes).any(|a| {
                    let v = joystick.axis(a).unwrap();
                    let moved = (i32::from(v) - i32::from(initial[a])).abs() > 16000;
                    if moved {
                        println!("axis {a}: {} -> {v}", initial[a]);
                    }
                    moved
                })
            }),
            "no axis movement arrived"
        );
    }
    match joystick.rumble(0x4000, 0x4000, 250) {
        Ok(()) => println!("rumble: sent"),
        Err(e) => println!("rumble: {}", e.message()),
    }
    drop(joystick);
    crate::init::quit_subsystem(crate::init::InitFlags::GAMEPAD);
}
