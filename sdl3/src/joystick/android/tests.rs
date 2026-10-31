// Tests of the Android joystick driver (the program spawned on the
// device: no Java VM, so the joystick handler finds nothing; in an app:
// sdl3/tests/android_joystick.rs).
//
// The devices are added and moved as the joystick handler's natives would
// do it (`nativeAddJoystick()`, `onNativePadDown()`, `onNativeJoy()`,
// `onNativeHat()`, ...), with the joystick subsystem up.

use super::*;
use crate::events::queue;
use crate::events::{Event, EventType};
use crate::init::{self, InitFlags};
use crate::joystick::gamepad::{self, Gamepad, GamepadAxis};
use crate::joystick::joysticks;

#[test]
fn keycodes_and_scancodes() {
    for (keycode, button) in [
        (AKEYCODE_BUTTON_A, 0),
        (AKEYCODE_BUTTON_B, 1),
        (AKEYCODE_BUTTON_X, 2),
        (AKEYCODE_BUTTON_Y, 3),
        (AKEYCODE_BACK, 4),
        (AKEYCODE_BUTTON_SELECT, 4),
        (AKEYCODE_BUTTON_MODE, 5),
        (AKEYCODE_MENU, 6),
        (AKEYCODE_BUTTON_START, 6),
        (AKEYCODE_BUTTON_THUMBL, 7),
        (AKEYCODE_BUTTON_THUMBR, 8),
        (AKEYCODE_BUTTON_L1, 9),
        (AKEYCODE_BUTTON_R1, 10),
        (AKEYCODE_DPAD_UP, 11),
        (AKEYCODE_DPAD_DOWN, 12),
        (AKEYCODE_DPAD_LEFT, 13),
        (AKEYCODE_DPAD_RIGHT, 14),
        (AKEYCODE_DPAD_CENTER, 0),
        (AKEYCODE_BUTTON_L2, 15),
        (AKEYCODE_MEDIA_RECORD, 15),
        (AKEYCODE_BUTTON_R2, 16),
        (AKEYCODE_BUTTON_C, 17),
        (AKEYCODE_BUTTON_Z, 18),
        (AKEYCODE_BUTTON_1, 20),
        (AKEYCODE_BUTTON_16, 35),
    ] {
        assert_eq!(keycode_to_sdl(keycode), button, "keycode {keycode}");
    }
    // (the letters, ESCAPE, ...)
    for keycode in [0, 29, 111, 187, 204] {
        assert_eq!(keycode_to_sdl(keycode), -1, "keycode {keycode}");
    }
    assert_eq!(scancode_to_sdl(BTN_DPAD_UP), 11);
    assert_eq!(scancode_to_sdl(BTN_DPAD_RIGHT), 14);
    assert_eq!(scancode_to_sdl(0x130), -1);
    assert_eq!(button_to_scancode(0), Scancode::RETURN);
    assert_eq!(button_to_scancode(1), Scancode::ESCAPE);
    assert_eq!(button_to_scancode(4), Scancode::ESCAPE);
    assert_eq!(button_to_scancode(6), Scancode::MENU);
    assert_eq!(button_to_scancode(13), Scancode::LEFT);
    assert_eq!(button_to_scancode(2), Scancode::UNKNOWN);
}

/// The gamepad buttons upstream's `getButtonMask()` gives a pad with the
/// face buttons, BACK, MODE, START, the thumbs and the shoulders.
const BUTTON_MASK: i32 = 0x07ff;

/// The events queued, as text (taken from the queue, which isn't pumped).
fn events() -> Vec<String> {
    let mut out = Vec::new();
    for e in queue::get_events(EventType::FIRST, EventType::LAST, usize::MAX).unwrap() {
        match e {
            Event::JoyDevice(e) => out.push(format!("joy device {:#x}", e.event_type.0)),
            Event::GamepadDevice(e) => out.push(format!("gamepad device {:#x}", e.event_type.0)),
            Event::JoyAxis(e) => out.push(format!("joy axis {} {}", e.axis, e.value)),
            Event::JoyButton(e) => out.push(format!("joy button {} {}", e.button, e.down as i32)),
            Event::GamepadAxis(e) => out.push(format!("gamepad axis {} {}", e.axis, e.value)),
            Event::GamepadButton(e) => {
                out.push(format!("gamepad button {} {}", e.button, e.down as i32))
            }
            Event::Key(e) => out.push(format!("key {} {}", e.scancode.0, e.down as i32)),
            _ => {}
        }
    }
    out
}

#[test]
fn devices_from_the_joystick_handler() {
    let _l = crate::test_support::test_lock();
    init::init_subsystem(InitFlags::GAMEPAD).unwrap();
    queue::flush_events(EventType::FIRST, EventType::LAST);
    let before = joysticks().len();

    // A pad with the face buttons and the others, six axes and a hat
    add_joystick(
        4242,
        Some("Test Pad"),
        "0123456789abcdef",
        0x1209,
        0x5d3a,
        BUTTON_MASK,
        6,
        0x3f,
        1,
        false,
        false,
        false,
        false,
    );
    let list = joysticks();
    assert_eq!(list.len(), before + 1);
    let id = *list.last().unwrap();
    assert_eq!(
        crate::joystick::joystick_name_for_id(id)
            .unwrap()
            .as_deref(),
        Some("Test Pad")
    );
    let guid = crate::joystick::joystick_guid_for_id(id);
    // (Bluetooth, the IDs, and the masks: the hat's d-pad buttons too)
    assert_eq!(guid.0[0], HARDWARE_BUS_BLUETOOTH as u8);
    assert_eq!(&guid.0[4..6], &0x1209u16.to_le_bytes());
    assert_eq!(&guid.0[8..10], &0x5d3au16.to_le_bytes());
    assert_eq!(&guid.0[12..14], &0x7fffu16.to_le_bytes());
    assert_eq!(&guid.0[14..16], &0x3fu16.to_le_bytes());
    assert!(gamepad::is_gamepad(id));
    let mapping = gamepad::gamepad_mapping_for_id(id).unwrap();
    assert!(
        mapping.contains(",*,a:b0,b:b1,x:b2,y:b3,back:b4,guide:b5,start:b6,leftstick:b7,rightstick:b8,leftshoulder:b9,rightshoulder:b10,dpup:b11,dpdown:b12,dpleft:b13,dpright:b14,leftx:a0,lefty:a1,rightx:a2,righty:a3,lefttrigger:a4,righttrigger:a5,"),
        "{mapping}"
    );
    assert_eq!(
        events(),
        ["joy device 0x605", "gamepad device 0x653"],
        "the added events"
    );

    // The same device again is ignored
    add_joystick(
        4242,
        Some("Test Pad"),
        "",
        0x1209,
        0x5d3a,
        BUTTON_MASK,
        6,
        0x3f,
        1,
        false,
        false,
        false,
        false,
    );
    assert_eq!(joysticks().len(), before + 1);

    // Its keys are keyboard keys until it's open
    assert!(on_pad_down(4242, AKEYCODE_BUTTON_A, 0));
    assert!(on_pad_up(4242, AKEYCODE_BUTTON_A, 0));
    assert!(!on_pad_down(4242, 29, 0));
    // (SDL_SCANCODE_RETURN)
    assert_eq!(events(), ["key 40 1", "key 40 0"]);

    let pad = Gamepad::open(id).unwrap();
    let joy = pad.joystick().unwrap();
    assert_eq!(joy.num_buttons().unwrap(), 15);
    assert_eq!(joy.num_axes().unwrap(), 6);
    assert_eq!(joy.num_hats().unwrap(), 0);
    queue::flush_events(EventType::FIRST, EventType::LAST);

    assert!(on_pad_down(4242, AKEYCODE_BUTTON_B, 0));
    // (an axis's first value is its initial one, not sent)
    assert!(on_joy(4242, 0, 0.0));
    assert!(on_joy(4242, 4, 0.0));
    assert!(on_joy(4242, 0, 0.5));
    assert!(on_joy(4242, 4, -1.0));
    assert!(on_hat(4242, 0, -1, 1));
    assert!(on_hat(4242, 0, 0, 1));
    assert!(!on_hat(4242, 0, 2, 0));
    // (the left Joy-Con's d-pad, by scan code)
    assert!(on_pad_down(4242, 0, BTN_DPAD_UP));
    assert!(on_pad_up(4242, AKEYCODE_BUTTON_B, 0));
    crate::joystick::update_joysticks();
    let got = events();
    for expected in [
        "joy button 1 1",
        "gamepad button 1 1",
        "joy axis 0 16383",
        "gamepad axis 0 16383",
        "joy axis 4 -32767",
        "joy button 12 1",
        "joy button 13 1",
        "joy button 13 0",
        "joy button 11 1",
        "joy button 1 0",
    ] {
        assert!(got.iter().any(|e| e == expected), "{expected:?} in {got:?}");
    }
    assert_eq!(pad.axis(GamepadAxis::LeftX), 16383);

    // No rumble, LED or sensors
    assert!(joy.rumble(1, 2, 100).is_err());
    assert!(joy.set_led(1, 2, 3).is_err());

    // Removed
    remove_joystick(4242);
    remove_joystick(4242);
    assert_eq!(joysticks().len(), before);
    assert!(!joy.connected());
    drop(joy);
    drop(pad);

    init::quit_subsystem(InitFlags::GAMEPAD);
}

#[test]
fn devices_ignored() {
    let _l = crate::test_support::test_lock();

    // Before the subsystem is up: ignored (the handler tells again then)
    add_joystick(
        77,
        Some("Early"),
        "",
        0x1209,
        0x5d3b,
        BUTTON_MASK,
        2,
        3,
        0,
        false,
        false,
        false,
        false,
    );
    init::init_subsystem(InitFlags::JOYSTICK).unwrap();
    let before = joysticks().len();
    assert!(!joysticks()
        .iter()
        .any(|&id| crate::joystick::joystick_name_for_id(id)
            .ok()
            .flatten()
            .as_deref()
            == Some("Early")));

    // A remote (no axes, no hats) without SDL_TV_REMOTE_AS_JOYSTICK
    crate::hints::set(hints::TV_REMOTE_AS_JOYSTICK, "0").unwrap();
    add_joystick(
        78,
        Some("Remote"),
        "",
        0x1209,
        0x5d3c,
        0x10,
        0,
        0,
        0,
        false,
        false,
        false,
        false,
    );
    assert_eq!(joysticks().len(), before);
    crate::hints::reset(hints::TV_REMOTE_AS_JOYSTICK);

    // No name
    add_joystick(
        79,
        None,
        "",
        0x1209,
        0x5d3d,
        BUTTON_MASK,
        2,
        3,
        0,
        false,
        false,
        false,
        false,
    );
    assert_eq!(joysticks().len(), before);

    // A remote with it: a joystick, with no gamepad mapping (no face
    // buttons)
    add_joystick(
        80,
        Some("Remote"),
        "",
        0x1209,
        0x5d3c,
        0x10,
        0,
        0,
        0,
        false,
        false,
        false,
        false,
    );
    assert_eq!(joysticks().len(), before + 1);
    let id = *joysticks().last().unwrap();
    assert!(!gamepad::is_gamepad(id));
    remove_joystick(80);
    assert_eq!(joysticks().len(), before);

    init::quit_subsystem(InitFlags::JOYSTICK);
}
