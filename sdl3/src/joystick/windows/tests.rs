// Tests for the Windows joystick driver (run under Wine on Linux).
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

use super::xinput::{battery_information, get_xinput_name, state_inputs, Input};
use super::*;
use crate::core::windows::directx::*;
use crate::core::windows::xinput::*;
use crate::joystick::{joystick_guid_info, HAT_CENTERED, HAT_LEFTDOWN, HAT_RIGHTUP};
use crate::power::PowerState;

fn lock() -> std::sync::MutexGuard<'static, ()> {
    crate::test_support::test_lock()
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
        dxdevice: crate::core::windows::directx::DIDEVICEINSTANCEW::new(),
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

/// The CPU time a thread has used so far.
fn thread_cpu_time(thread_id: u32) -> Duration {
    use windows_sys::Win32::Foundation::{CloseHandle, FILETIME};
    use windows_sys::Win32::System::Threading::{
        GetThreadTimes, OpenThread, THREAD_QUERY_LIMITED_INFORMATION,
    };
    // SAFETY: opening a thread by ID has no memory preconditions.
    let handle = unsafe { OpenThread(THREAD_QUERY_LIMITED_INFORMATION, 0, thread_id) };
    assert!(!handle.is_null());
    let zero = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let (mut creation, mut exit, mut kernel, mut user) = (zero, zero, zero, zero);
    // SAFETY: the handle is open and the four times are writable.
    let ok = unsafe { GetThreadTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user) };
    // SAFETY: the handle was opened above.
    unsafe {
        CloseHandle(handle);
    }
    assert_ne!(ok, 0);
    let ticks = |t: &FILETIME| ((t.dwHighDateTime as u64) << 32) | t.dwLowDateTime as u64;
    Duration::from_nanos((ticks(&kernel) + ticks(&user)) * 100)
}

#[test]
fn detection_thread_waits_for_a_signalled_change() {
    let _l = lock();
    hints::set(hints::JOYSTICK_THREAD, "1").unwrap();
    crate::init::init_subsystem(crate::init::InitFlags::JOYSTICK).unwrap();
    let mut thread_id = 0;
    for _ in 0..500 {
        thread_id = JOYSTICK_THREAD_ID.load(Ordering::Acquire);
        if thread_id != 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_ne!(thread_id, 0);

    // A change nobody has detected yet leaves the thread idle, not spinning
    set_windows_device_changed();
    let before = thread_cpu_time(thread_id);
    std::thread::sleep(Duration::from_millis(300));
    let spent = thread_cpu_time(thread_id) - before;
    assert!(spent < Duration::from_millis(100), "{spent:?}");

    // Detection takes the change and the thread goes back to its messages
    {
        let _lock = crate::joystick::lock_joysticks();
        WINDOWS_JOYSTICK_DRIVER.detect();
    }
    assert!(!windows_device_changed());
    crate::init::quit_subsystem(crate::init::InitFlags::JOYSTICK);
    assert!(JOYSTICK_THREAD.lock().unwrap().is_none());
    hints::reset(hints::JOYSTICK_THREAD);
}

// --- the scanner of the Steam virtual gamepad slots ---

#[test]
fn scan_like_sscanf() {
    use super::Scan::*;
    // (expected values from upstream's sscanf() formats, run with the C library)
    let di = |path| super::dinput::get_steam_virtual_gamepad_slot(0x28DE, 0x11FF, path);
    assert_eq!(
        di("\\\\?\\HID#VID_28DE&PID_11FF&IG_00#8&2C2D2A42&0&0000#{4D1E55B2-F16F-11CF-88CB-001111000030}"),
        0
    );
    assert_eq!(di("\\\\?\\HID#VID_28DE&PID_11FF&IG_03#X"), 3);
    assert_eq!(di("\\\\?\\HID#VID_28DE&PID_11FF&IG_012"), 12);
    assert_eq!(di("\\\\?\\HID#VID_28DE&PID_11FF&IG_0"), -1);
    assert_eq!(di("\\\\?\\hid#VID_28DE&PID_11FF&IG_01"), -1);
    assert_eq!(di("\\\\?\\HID#VID_28DE&PID_11FF&IG_0-7"), -7);
    assert_eq!(di("\\\\?\\HID#VID_28DE&PID_11FF&IG_0 5"), 5);
    // Only for the Steam virtual gamepad
    assert_eq!(
        super::dinput::get_steam_virtual_gamepad_slot(
            0x045E,
            0x11FF,
            "\\\\?\\HID#VID_28DE&PID_11FF&IG_03"
        ),
        -1
    );

    let raw = |path| super::rawinput::get_steam_virtual_gamepad_slot(0x28DE, 0x11FF, path);
    assert_eq!(
        raw("\\\\.\\pipe\\HID#VID_045E&PID_028E&IG_00#28DE&11FF&1#3#123"),
        3
    );
    assert_eq!(
        raw("\\\\.\\pipe\\HID#VID_045E&PID_028E&IG_00#28de&11ff&0x10#12#0"),
        12
    );
    assert_eq!(
        raw("\\\\.\\pipe\\HID#VID_045E&PID_028E&IG_00#ZZ&1&1#3#1"),
        -1
    );
    assert_eq!(raw("\\\\.\\pipe\\HID#VID_045E&PID_028E&IG_00#1&2&3#"), -1);
    assert_eq!(raw("\\\\.\\pipe\\HID#VID_045E&PID_028E&IG_00#1&2&3#-4"), -4);

    let wgi = super::windows_gaming_input::steam_virtual_gamepad_slot_of_id;
    assert_eq!(wgi("{wgi/nrid/:steam-28DE&11FF&1#2#3}"), 2);
    assert_eq!(wgi("{wgi/nrid/:steam-28DE&11FF&1#2}"), 2);
    assert_eq!(wgi("{wgi/nrid/:other-28DE&11FF&1#2#3}"), -1);
    assert_eq!(wgi("{wgi/nrid/:steam-28DE&11FF#2#3}"), -1);
    assert_eq!(wgi("{wgi/nrid/:steam-A&B&C# 17#3}"), 17);

    assert_eq!(scan_int("", &[Int]), None);
    assert_eq!(scan_int("12", &[Lit("1"), Int]), Some(2));
}

// --- DirectInput ---

#[test]
fn pov_translation() {
    use super::dinput::translate_pov;
    // (expected values from upstream's TranslatePOV())
    let povs: [u32; 23] = [
        0, 1, 2249, 2250, 4499, 4500, 6750, 9000, 13500, 18000, 22500, 27000, 31500, 33749, 33750,
        35999, 36000, 40000, 0xFFFF, 0x1FFFF, 0xFFFFFFFF, 0xFFFFF000, 0x12345678,
    ];
    let expected: [u8; 23] = [
        1, 1, 1, 3, 3, 3, 2, 2, 6, 4, 12, 8, 9, 9, 1, 1, 1, 3, 0, 0, 0, 4, 9,
    ];
    for (pov, hat) in povs.iter().zip(expected) {
        assert_eq!(translate_pov(*pov), hat, "{pov}");
    }
}

#[test]
fn rumble_magnitudes() {
    use super::dinput::{convert_magnitude, rumble_magnitude, RumbleEffect};
    // (expected values from upstream's SDL_DINPUT_JoystickRumble() and CONVERT_MAGNITUDE())
    let cases: [((u16, u16), (i16, u32)); 8] = [
        ((0, 0), (0, 0)),
        ((0xFFFF, 0xFFFF), (32767, 10000)),
        ((0xFFFF, 0), (16383, 4999)),
        ((0, 0xFFFF), (16383, 4999)),
        ((1, 1), (0, 0)),
        ((3, 3), (1, 0)),
        ((0x8000, 0x4000), (12288, 3750)),
        ((12345, 54321), (16666, 5086)),
    ];
    for ((low, high), (magnitude, converted)) in cases {
        assert_eq!(rumble_magnitude(low, high), magnitude);
        assert_eq!(convert_magnitude(magnitude), converted);
    }

    // CreateRumbleEffectData()
    let mut effect = RumbleEffect::new(16383);
    let dieffect = effect.dieffect();
    assert_eq!(dieffect.dwSize, size_of::<DIEFFECT>() as u32);
    assert_eq!(dieffect.dwGain, 10000);
    assert_eq!(dieffect.dwFlags, DIEFF_OBJECTOFFSETS | DIEFF_CARTESIAN);
    assert_eq!(dieffect.dwDuration, 0xFFFF * 1000);
    assert_eq!(dieffect.dwTriggerButton, DIEB_NOTRIGGER);
    assert_eq!(dieffect.cAxes, 2);
    assert_eq!(dieffect.cbTypeSpecificParams, 16);
    // SAFETY: the effect points into `effect`, which is alive.
    let periodic = unsafe { *dieffect.lpvTypeSpecificParams.cast::<DIPERIODIC>() };
    assert_eq!(
        periodic,
        DIPERIODIC {
            dwMagnitude: 4999,
            lOffset: 0,
            dwPhase: 0,
            dwPeriod: 1000000
        }
    );
    // SAFETY: as above.
    unsafe {
        assert_eq!(*dieffect.rgdwAxes, 0);
        assert_eq!(*dieffect.rglDirection.add(1), 0);
    }
}

#[test]
fn joystick_data_format() {
    let format = &super::dinput::C_DF_DIJOYSTICK2.0;
    assert_eq!(format.dwSize, size_of::<DIDATAFORMAT>() as u32);
    assert_eq!(format.dwDataSize, 272);
    assert_eq!(format.dwFlags, DIDF_ABSAXIS);
    // SAFETY: the format points to its 164 static objects.
    let objects = unsafe { std::slice::from_raw_parts(format.rgodf, format.dwNumObjs as usize) };
    assert_eq!(objects.len(), 164);
    // SAFETY: the GUID pointers point to static GUIDs (or are NULL).
    let guid =
        |i: usize| unsafe { objects[i].pguid.as_ref() }.map(crate::core::windows::guid_bytes);
    let g = |g: &windows_sys::core::GUID| Some(crate::core::windows::guid_bytes(g));
    // The position axes, sliders, POVs, then the 128 buttons
    let expected_ofs = [0, 4, 8, 12, 16, 20, 24, 28, 32, 36, 40, 44, 48];
    for (i, ofs) in expected_ofs.iter().enumerate() {
        assert_eq!(objects[i].dwOfs, *ofs, "{i}");
    }
    assert_eq!(guid(0), g(&GUID_XAXIS));
    assert_eq!(guid(5), g(&GUID_RZAXIS));
    assert_eq!(guid(3), g(&GUID_RXAXIS));
    assert_eq!(guid(6), g(&GUID_SLIDER));
    assert_eq!(guid(8), g(&GUID_POV));
    assert_eq!(guid(12), None);
    assert_eq!(objects[0].dwFlags, DIDOI_ASPECTPOSITION);
    assert_eq!(
        objects[0].dwType,
        DIDFT_OPTIONAL | DIDFT_AXIS | DIDFT_ANYINSTANCE
    );
    assert_eq!(
        objects[8].dwType,
        DIDFT_OPTIONAL | DIDFT_POV | DIDFT_ANYINSTANCE
    );
    assert_eq!(objects[8].dwFlags, 0);
    assert_eq!(objects[139].dwOfs, 48 + 127);
    assert_eq!(
        objects[139].dwType,
        DIDFT_OPTIONAL | DIDFT_BUTTON | DIDFT_ANYINSTANCE
    );
    // Velocity, acceleration and force; their sliders use the position offsets
    assert_eq!(objects[140].dwOfs, 176);
    assert_eq!(objects[140].dwFlags, DIDOI_ASPECTVELOCITY);
    assert_eq!(objects[145].dwOfs, 196);
    assert_eq!(objects[146].dwOfs, 24);
    assert_eq!(objects[147].dwOfs, 28);
    assert_eq!(objects[148].dwOfs, 208);
    assert_eq!(objects[148].dwFlags, DIDOI_ASPECTACCEL);
    assert_eq!(objects[156].dwOfs, 240);
    assert_eq!(objects[163].dwOfs, 28);
    assert_eq!(objects[163].dwFlags, DIDOI_ASPECTFORCE);
    assert_eq!(guid(163), g(&GUID_SLIDER));
}

/// A device object of a type, for EnumDevObjectsCallback().
fn device_object(dw_type: u32, guid_type: windows_sys::core::GUID) -> DIDEVICEOBJECTINSTANCEW {
    // SAFETY: the structure is plain data.
    let mut object: DIDEVICEOBJECTINSTANCEW = unsafe { std::mem::zeroed() };
    object.dwType = dw_type;
    object.guidType = guid_type;
    object
}

#[test]
fn device_objects_and_their_order() {
    use super::dinput::{
        enum_dev_objects_callback, sort_dev_objects, DeviceInput, InputType, ObjectCounts,
    };
    // SAFETY: a GUID is plain data.
    let none: windows_sys::core::GUID = unsafe { std::mem::zeroed() };
    let mut counts = ObjectCounts::default();
    let objects = [
        device_object(DIDFT_BUTTON, none),
        device_object(DIDFT_AXIS, GUID_YAXIS),
        device_object(DIDFT_POV, GUID_POV),
        device_object(DIDFT_AXIS, GUID_XAXIS),
        device_object(DIDFT_AXIS, GUID_SLIDER),
        device_object(DIDFT_BUTTON, none),
        device_object(DIDFT_AXIS, GUID_RZAXIS), // refused by configure_axis below
        device_object(DIDFT_AXIS, GUID_POV),    // not an axis we can grok
        device_object(0x0100_0000, none),       // not supported
    ];
    for object in &objects {
        let refuse = object.guidType.data1 == GUID_RZAXIS.data1;
        assert!(enum_dev_objects_callback(object, &mut counts, |_| !refuse));
    }
    assert_eq!((counts.nbuttons, counts.nhats, counts.naxes), (2, 1, 3));
    assert_eq!(counts.num_sliders, 1);
    let input = |ofs, kind, num| DeviceInput { ofs, kind, num };
    assert_eq!(
        counts.inputs,
        [
            input(48, InputType::Button, 0),
            input(4, InputType::Axis, 0),
            input(32, InputType::Hat, 0),
            input(0, InputType::Axis, 1),
            input(24, InputType::Axis, 2),
            input(49, InputType::Button, 1),
        ]
    );
    sort_dev_objects(&mut counts.inputs);
    assert_eq!(
        counts.inputs,
        [
            input(0, InputType::Axis, 0),
            input(4, InputType::Axis, 1),
            input(24, InputType::Axis, 2),
            input(32, InputType::Hat, 0),
            input(48, InputType::Button, 0),
            input(49, InputType::Button, 1),
        ]
    );

    // Four POVs at most; enumeration stops at 256 inputs
    let mut counts = ObjectCounts::default();
    for _ in 0..5 {
        assert!(enum_dev_objects_callback(
            &device_object(DIDFT_POV, GUID_POV),
            &mut counts,
            |_| true
        ));
    }
    assert_eq!(counts.nhats, 4);
    for i in 0..252 {
        let go_on =
            enum_dev_objects_callback(&device_object(DIDFT_BUTTON, none), &mut counts, |_| true);
        assert_eq!(go_on, i != 251, "{i}");
    }
    assert_eq!(counts.inputs.len(), 256);
}

#[test]
fn polled_and_buffered_states() {
    use super::dinput::{buffered_inputs, polled_state_inputs, DeviceInput, InputType};
    let inputs = [
        DeviceInput {
            ofs: 0,
            kind: InputType::Axis,
            num: 0,
        },
        DeviceInput {
            ofs: 20,
            kind: InputType::Axis,
            num: 1,
        },
        DeviceInput {
            ofs: 28,
            kind: InputType::Axis,
            num: 2,
        },
        DeviceInput {
            ofs: 32 + 4,
            kind: InputType::Hat,
            num: 1,
        },
        DeviceInput {
            ofs: 48 + 3,
            kind: InputType::Button,
            num: 0,
        },
        DeviceInput {
            ofs: 48 + 200,
            kind: InputType::Button,
            num: 1,
        },
    ];
    let mut buttons = [0u8; 128];
    buttons[3] = 0x80;
    let state = DIJOYSTATE2 {
        lX: 0x12345, // (truncated to 16 bits, as the C cast does)
        lRz: -32768,
        rglSlider: [0, 77],
        rgdwPOV: [0, 9000, 0, 0],
        rgbButtons: buttons,
        ..Default::default()
    };
    assert_eq!(
        polled_state_inputs(&inputs, &state),
        [
            Input::Axis(0, 0x2345),
            Input::Axis(1, -32768),
            Input::Axis(2, 77),
            Input::Hat(1, crate::joystick::HAT_RIGHT),
            Input::Button(0, true),
            // (past rgbButtons: released)
            Input::Button(1, false),
        ]
    );

    let event = |ofs, data| DIDEVICEOBJECTDATA {
        dwOfs: ofs,
        dwData: data,
        ..Default::default()
    };
    assert_eq!(
        buffered_inputs(
            &inputs,
            &[
                event(20, 0xFFFF_8000),
                event(51, 0),
                event(36, 0xFFFF),
                event(99, 1),
                event(0, 5)
            ]
        ),
        [
            Input::Axis(1, -32768),
            Input::Button(0, false),
            Input::Hat(1, HAT_CENTERED),
            Input::Axis(0, 5),
        ]
    );
}

// --- RawInput ---

#[test]
fn match_states() {
    use super::rawinput::{
        fill_match_state, wgi_axes_match, wgi_triggers_match, xinput_axes_match,
        xinput_triggers_match,
    };
    use super::wgi_abi::GamepadReading;
    // (expected values from upstream's RAWINPUT_FillMatchState() and the
    // XInput/WGI match macros)
    let pad = |b_left: u8, b_right: u8, lx: i16, ly: i16, rx: i16, ry: i16| XINPUT_GAMEPAD {
        wButtons: 0,
        bLeftTrigger: b_left,
        bRightTrigger: b_right,
        sThumbLX: lx,
        sThumbLY: ly,
        sThumbRX: rx,
        sThumbRY: ry,
    };
    let pads = [
        pad(0, 0, 0, 0, 0, 0),
        pad(0, 0, 0x1000, 0, 0, 0),
        pad(255, 0, -32768, 32767, 0x7FFF, -1),
        pad(0, 128, 0x1234, -0x5679, 0x3000, 0),
        pad(1, 1, 0x7000, 0x7FFF, -0x1000, -0x7000),
    ];
    let reading =
        |buttons: u32, triggers: (f64, f64), left: (f64, f64), right: (f64, f64)| GamepadReading {
            Timestamp: 0,
            Buttons: buttons,
            LeftTrigger: triggers.0,
            RightTrigger: triggers.1,
            LeftThumbstickX: left.0,
            LeftThumbstickY: left.1,
            RightThumbstickX: right.0,
            RightThumbstickY: right.1,
        };
    let readings = [
        reading(0, (0.0, 0.0), (0.0, 0.0), (0.0, 0.0)),
        reading(4, (1.0, 0.0), (1.0, -1.0), (0.5, -0.5)),
        reading(0, (0.0, 0.5), (0.125, 0.25), (-0.125, 0.875)),
        reading(0x3FFF, (0.75, 0.75), (-1.0, 1.0), (0.0, 0.0)),
    ];
    type Case = (u64, [i16; 6], u16, u32, bool, &'static str, &'static str);
    let cases: [Case; 9] = [
        (
            0x0000008800000000,
            [0, 0, 0, 0, -32768, -32768],
            0,
            0,
            false,
            "1111010101",
            "11010101",
        ),
        (0, [0, 0, 0, 0, 0, 0], 0, 0, true, "1010000101", "11000000"),
        (
            0x0000008800001001,
            [0, 0, 0, 0, -32768, -32768],
            4098,
            132,
            true,
            "1111010101",
            "11010101",
        ),
        (
            0x00000088FFFF7FFF,
            [-4096, -4096, -4096, -4096, -32768, -32768],
            62463,
            16383,
            true,
            "1101010101",
            "11010101",
        ),
        (
            0x0000001234560000,
            [24576, 20480, 16384, 12288, 8192, 4096],
            0,
            0,
            true,
            "0000000001",
            "01000001",
        ),
        (
            0x000000F0F0F05555,
            [0, -4096, 0, -4096, 0, -4096],
            21178,
            10903,
            true,
            "1010000101",
            "11000100",
        ),
        (
            0x0000FFFFFFFFFFFF,
            [-4096; 6],
            62463,
            16383,
            true,
            "1000000101",
            "11000100",
        ),
        (
            0x0000000011110000,
            [4096, 4096, 4096, 4096, 0, 0],
            0,
            0,
            true,
            "0000000101",
            "01000000",
        ),
        (
            0x0000000022220000,
            [8192, 8192, 8192, 8192, 0, 0],
            0,
            0,
            true,
            "0000000101",
            "01000000",
        ),
    ];
    for (match_state, axes, xb, wb, any, xin, wgi) in cases {
        let state = fill_match_state(match_state);
        assert_eq!(state.match_axes, axes, "{match_state:x}");
        assert_eq!(state.xinput_buttons, xb, "{match_state:x}");
        assert_eq!(state.wgi_buttons, wb, "{match_state:x}");
        assert_eq!(state.any_data, any, "{match_state:x}");
        let xin_got: String = pads
            .iter()
            .map(|p| {
                format!(
                    "{}{}",
                    xinput_axes_match(p, &state) as u8,
                    xinput_triggers_match(p, &state) as u8
                )
            })
            .collect();
        assert_eq!(xin_got, xin, "{match_state:x}");
        let wgi_got: String = readings
            .iter()
            .map(|r| {
                format!(
                    "{}{}",
                    wgi_axes_match(r, &state) as u8,
                    wgi_triggers_match(r, &state) as u8
                )
            })
            .collect();
        assert_eq!(wgi_got, wgi, "{match_state:x}");
    }
}

/// The events of a state packet, as upstream's harness prints them.
fn packet_string(events: &[super::rawinput::Event], match_state: u64) -> String {
    use super::rawinput::Event;
    let mut s = String::new();
    for event in events {
        match event {
            Event::Button { button, down, .. } => s += &format!(" B{button}={}", *down as u8),
            Event::Axis { axis, value, .. } => s += &format!(" A{axis}={value}"),
            Event::Hat { hat, value, .. } => s += &format!(" H{hat}={value}"),
            Event::Power { .. } => s += " P",
        }
    }
    s + &format!(" state={match_state:016x}")
}

#[test]
fn state_packets() {
    use super::rawinput::{get_data, StatePacket};
    use crate::core::windows::hid::HIDP_DATA;
    // (expected values from upstream's RAWINPUT_HandleStatePacket(), on the same data)
    let item = |index: u16, raw: u32| HIDP_DATA {
        DataIndex: index,
        Reserved: 0,
        RawValue: raw,
    };
    let buttons = [0u16, 1, 2, 3, 4, 5, 6, 7, 8, 9];
    let axes = [11u16, 12, 13, 14];
    let hats = [10u16];
    let packet = |nbuttons, naxes, hacks: bool, has_trigger_data, match_state| StatePacket {
        nbuttons,
        naxes,
        nhats: 1,
        guide_hack: hacks,
        trigger_hack: hacks,
        trigger_hack_index: 15,
        button_indices: &buttons,
        axis_indices: &axes,
        hat_indices: &hats,
        has_trigger_data,
        match_state,
    };
    let data = [
        item(0, 1),
        item(1, 0),
        item(2, 0x101),
        item(3, 0x100),
        item(4, 1),
        item(5, 0),
        item(6, 1),
        item(7, 1),
        item(8, 0),
        item(9, 1),
        item(10, 3),
        item(11, 0),
        item(12, 0xFFFF),
        item(13, 0x8000),
        item(14, 0x12345),
        item(15, 0x9000),
    ];
    let (events, state) = packet(11, 6, true, false, 0x0000008800000000).events(&data, 0);
    assert_eq!(
        packet_string(&events, state),
        " B0=1 B1=0 B2=1 B3=0 B4=1 B5=0 B6=1 B7=1 B8=0 B9=1 A0=-32768 A1=32767 A2=0 A3=-23739 H0=2 A4=-24575 A5=-32768 state=0000008aa0784355"
    );

    let shuffled = [
        item(15, 0x7000),
        item(14, 0x4000),
        item(10, 9),
        item(0, 1),
        item(13, 1),
        item(3, 1),
    ];
    let (events, state) = packet(11, 6, true, true, 0x00000088000FFFFF).events(&shuffled, 0);
    assert_eq!(
        packet_string(&events, state),
        " B0=1 B1=0 B2=0 B3=1 B4=0 B5=0 B6=0 B7=0 B8=0 B9=0 A2=-32767 A3=-16384 H0=0 state=00000098c80f8029"
    );

    let hat_out = [item(10, 10), item(11, 0x1000)];
    let (events, state) = packet(10, 4, false, false, 0).events(&hat_out, 0);
    assert_eq!(
        packet_string(&events, state),
        " B0=0 B1=0 B2=0 B3=0 B4=0 B5=0 B6=0 B7=0 B8=0 B9=0 A0=-28672 H0=0 state=0000000000090000"
    );

    let (events, state) = packet(11, 6, true, false, 0).events(&[], 0);
    assert_eq!(
        packet_string(&events, state),
        " B0=0 B1=0 B2=0 B3=0 B4=0 B5=0 B6=0 B7=0 B8=0 B9=0 state=0000000000000000"
    );

    // GetData() looks at the expected offset first, then searches
    assert_eq!(get_data(14, &shuffled), Some(&shuffled[1]));
    assert_eq!(get_data(3, &data), Some(&data[3]));
    assert_eq!(get_data(16, &data), None);
}

// --- Windows.Gaming.Input ---

#[test]
fn wgi_conversions() {
    use super::wgi_abi::switch_position;
    use super::windows_gaming_input::{
        battery_percent, convert_axis_value, convert_hat_value, device_instance_id,
        power_state_of_battery_status,
    };
    use crate::joystick::*;
    // (expected values from upstream's WGI_JoystickUpdate() conversions)
    let values = [0.0, 1.0, -1.0, 0.5, -0.5, 0.25, 0.999, 0.0001, 0.75];
    let expected = [-32768, 32767, -32767, -1, 1, -16385, 32701, -32762, 16383];
    for (value, axis) in values.iter().zip(expected) {
        assert_eq!(convert_axis_value(*value), axis, "{value}");
    }
    let capacities = [
        (0, 0),
        (100, 50),
        (3, 1),
        (3, 2),
        (1000, 999),
        (7, 0),
        (200, 1),
        (8, 1),
    ];
    let percents = [0, 50, 33, 67, 100, 0, 1, 13];
    for ((full, curr), percent) in capacities.iter().zip(percents) {
        assert_eq!(battery_percent(*full, *curr), percent);
    }
    assert_eq!(power_state_of_battery_status(0), PowerState::NoBattery);
    assert_eq!(power_state_of_battery_status(1), PowerState::OnBattery);
    assert_eq!(power_state_of_battery_status(2), PowerState::Charged);
    assert_eq!(power_state_of_battery_status(3), PowerState::Charging);
    assert_eq!(power_state_of_battery_status(4), PowerState::Unknown);
    let hats = [
        (switch_position::CENTER, HAT_CENTERED),
        (switch_position::UP, HAT_UP),
        (switch_position::UP_RIGHT, HAT_RIGHTUP),
        (switch_position::RIGHT, HAT_RIGHT),
        (switch_position::DOWN_RIGHT, HAT_RIGHTDOWN),
        (switch_position::DOWN, HAT_DOWN),
        (switch_position::DOWN_LEFT, HAT_LEFTDOWN),
        (switch_position::LEFT, HAT_LEFT),
        (switch_position::UP_LEFT, HAT_LEFTUP),
        (9, HAT_CENTERED),
    ];
    for (position, hat) in hats {
        assert_eq!(convert_hat_value(position), hat);
    }
    assert_eq!(
        device_instance_id("\\\\?\\HID#VID_045E&PID_02FF&IG_00#9&2c203035&2&0000#{4d1e55b2-f16f-11cf-88cb-001111000030}")
            .as_deref(),
        Some("HID\\VID_045E&PID_02FF&IG_00\\9&2c203035&2&0000")
    );
    assert_eq!(
        device_instance_id("\\\\?\\HID#VID_045E&PID_02FF&IG_00#9&2c203035&2&0000"),
        None
    );
    assert_eq!(device_instance_id("HID#VID_045E#{x}"), None);
}

// --- the drivers together, under Wine (no controllers) ---

#[test]
fn drivers_with_every_api() {
    let _l = lock();
    // RawInput and WGI on, DirectInput on (the default)
    hints::set(hints::JOYSTICK_RAWINPUT, "1").unwrap();
    hints::set(hints::JOYSTICK_WGI, "1").unwrap();
    crate::init::init_subsystem(crate::init::InitFlags::JOYSTICK).unwrap();
    {
        let _lock = crate::joystick::lock_joysticks();
        // (Wine has DirectInput, so this enumerates)
        assert!(super::dinput::dinput_in_use());
        assert!(super::rawinput::is_enabled());
        assert_eq!(super::rawinput::RAWINPUT_JOYSTICK_DRIVER.count(), 0);
        assert_eq!(WINDOWS_JOYSTICK_DRIVER.count(), 0);
        assert_eq!(super::windows_gaming_input::WGI_JOYSTICK_DRIVER.count(), 0);
        assert!(
            !super::rawinput::RAWINPUT_JOYSTICK_DRIVER.is_device_present(0x045e, 0x02a1, 0, None)
        );
        assert!(!WINDOWS_JOYSTICK_DRIVER.is_device_present(0x045e, 0x02a1, 0, None));
        super::rawinput::RAWINPUT_JOYSTICK_DRIVER.detect();
        WINDOWS_JOYSTICK_DRIVER.detect();
    }
    assert_eq!(crate::joystick::joysticks(), []);
    crate::init::quit_subsystem(crate::init::InitFlags::JOYSTICK);
    assert!(!super::rawinput::is_enabled());

    // DirectInput off
    hints::set(hints::JOYSTICK_DIRECTINPUT, "0").unwrap();
    crate::init::init_subsystem(crate::init::InitFlags::JOYSTICK).unwrap();
    assert!(!super::dinput::dinput_in_use());
    assert_eq!(crate::joystick::joysticks(), []);
    crate::init::quit_subsystem(crate::init::InitFlags::JOYSTICK);

    hints::reset(hints::JOYSTICK_RAWINPUT);
    hints::reset(hints::JOYSTICK_WGI);
    hints::reset(hints::JOYSTICK_DIRECTINPUT);
}

// --- hardware (cargo test -- --ignored; see docs/HARDWARE_TESTING.md) ---

/// A name for the driver at `index` in `JOYSTICK_DRIVERS`.
fn driver_label(index: usize) -> String {
    use crate::joystick::{
        HIDAPI_DRIVER_INDEX, RAWINPUT_DRIVER_INDEX, VIRTUAL_DRIVER_INDEX, WGI_DRIVER_INDEX,
        WINDOWS_DRIVER_INDEX,
    };
    match index {
        HIDAPI_DRIVER_INDEX => "HIDAPI".to_owned(),
        RAWINPUT_DRIVER_INDEX => "RawInput".to_owned(),
        WINDOWS_DRIVER_INDEX => "DirectInput/XInput".to_owned(),
        WGI_DRIVER_INDEX => "Windows.Gaming.Input".to_owned(),
        VIRTUAL_DRIVER_INDEX => "virtual".to_owned(),
        _ => format!("driver #{index}"),
    }
}

/// With the hints in `config`, list what each joystick driver finds (and
/// which of the devices are gamepads), after a second of hotplug detection.
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
    let per_driver: Vec<(usize, Vec<crate::events::JoystickID>)> = {
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
    let mut total = 0;
    for (index, ids) in per_driver {
        for id in ids {
            print!("  {}: ", driver_label(index));
            total += 1;
            let name = joystick_name_for_id(id).ok().flatten().unwrap_or_default();
            let guid = joystick_guid_for_id(id);
            if is_gamepad(id) {
                let pad = gamepad_name_for_id(id).ok().flatten().unwrap_or_default();
                println!(
                    "{id}: {name:?} {guid}, gamepad {pad:?} ({:?})",
                    gamepad_type_for_id(id)
                );
            } else {
                println!("{id}: {name:?} {guid}");
            }
        }
    }
    assert_eq!(crate::joystick::joysticks().len(), total);
    crate::init::quit_subsystem(crate::init::InitFlags::GAMEPAD);
    for &(name, _) in config {
        hints::reset(name);
    }
}

/// Hardware: the controllers each Windows joystick driver sees. There may
/// be none; this lists them, it doesn't require any.
#[test]
#[ignore = "hardware: lists the connected controllers"]
fn hardware_joysticks_per_driver() {
    let _l = lock();
    let only = |on: &'static str| -> Vec<(&'static str, &'static str)> {
        [
            hints::JOYSTICK_HIDAPI,
            hints::JOYSTICK_RAWINPUT,
            hints::JOYSTICK_DIRECTINPUT,
            hints::XINPUT_ENABLED,
            hints::JOYSTICK_WGI,
            hints::JOYSTICK_GAMEINPUT,
        ]
        .into_iter()
        .map(|h| (h, if h == on { "1" } else { "0" }))
        .collect()
    };
    list_joysticks("the default drivers", &[]);
    list_joysticks(
        "every driver enabled",
        &[
            (hints::JOYSTICK_RAWINPUT, "1"),
            (hints::JOYSTICK_WGI, "1"),
            (hints::JOYSTICK_GAMEINPUT, "1"),
        ],
    );
    for (what, hint) in [
        ("HIDAPI alone", hints::JOYSTICK_HIDAPI),
        ("RawInput alone", hints::JOYSTICK_RAWINPUT),
        ("DirectInput alone", hints::JOYSTICK_DIRECTINPUT),
        ("XInput alone", hints::XINPUT_ENABLED),
        ("Windows.Gaming.Input alone", hints::JOYSTICK_WGI),
        ("GameInput alone", hints::JOYSTICK_GAMEINPUT),
    ] {
        list_joysticks(what, &only(hint));
    }
}

/// Hardware: input from a connected controller, through the default
/// drivers. It asks for a button press and a stick (or axis) movement and
/// waits up to 30 seconds for each, printing the events as they arrive,
/// then tries a short rumble (which a controller may not support).
#[test]
#[ignore = "hardware: needs a controller and someone to press its buttons"]
fn hardware_controller_input() {
    use crate::joystick::gamepad::{gamepad_name_for_id, gamepad_type_for_id, is_gamepad};
    use crate::joystick::{joysticks, Joystick, JOYSTICK_DRIVERS};
    use std::time::{Duration, Instant};
    let _l = lock();
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
