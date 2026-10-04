// Tests for the Steam Deck HIDAPI driver.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The transcript in `tests/` comes from running upstream's
//! SDL_hidapi_steamdeck.c, with stubs for the SDL and HID functions it
//! uses, on generated reports (see the Steam Controller driver's tests).

use super::super::steam::tests::{describe_open, parse_transcript, result_line, Entry, FakeHid};
use super::super::tests::{run, test_device};
use super::*;
use crate::hidapi::DeviceInfo;
use crate::joystick::usb_ids::USB_VENDOR_VALVE;

const TRANSCRIPT: &str = include_str!("tests/steamdeck.txt");

/// Replays the C transcript.
#[test]
fn transcript() {
    let _l = crate::test_support::test_lock();
    let _lock = crate::joystick::lock_joysticks();

    let device = test_device(&DeviceInfo {
        vendor_id: USB_VENDOR_VALVE,
        product_id: 0x1205,
        interface_number: 2,
        ..DeviceInfo::default()
    });
    let fake = FakeHid::new(false);
    let mut ctx = SteamDeckContext::default();
    let mut case = String::new();

    for entry in parse_transcript(TRANSCRIPT) {
        let step = match entry {
            Entry::Case(words) => {
                case = words[0].clone();
                continue;
            }
            Entry::Step(step) => step,
        };
        fake.queue(&step);
        let words: Vec<&str> = step.head.split_whitespace().collect();
        let events = match words[0] {
            "init" => {
                // (the context of a failed init is dropped)
                ctx = SteamDeckContext::default();
                let (result, mut events) = run(&device, |d| ctx.init(d, &fake));
                events.push(result_line(result.is_ok()));
                events
            }
            "open" => {
                let mut joystick = JoystickData::new(1);
                let (result, mut events) = run(&device, |d| ctx.open_joystick(d, &mut joystick));
                events.extend(describe_open(&joystick));
                events.push(result_line(result.is_ok()));
                assert_eq!(joystick.nbuttons, 18);
                events
            }
            "setwatchdog" => {
                ctx.watchdog_counter = words[1].parse().unwrap();
                Vec::new()
            }
            "update" => {
                let (ok, mut events) = run(&device, |d| match device.joysticks().first() {
                    Some(&joystick) if case == "state" || !step.inputs.is_empty() => {
                        ctx.update(d, &fake, joystick)
                    }
                    _ => ctx.update_device(d),
                });
                events.push(result_line(ok));
                events.push(format!("watchdog {}", ctx.watchdog_counter));
                events
            }
            "rumble" => {
                let result = rumble(&fake, words[1].parse().unwrap(), words[2].parse().unwrap());
                vec![result_line(result.is_ok())]
            }
            other => panic!("unknown step {other}"),
        };
        if words[0] == "init" && events.last().is_some_and(|l| l == "result 1") {
            assert_eq!(device.name(), "Steam Deck");
        }
        step.check(&fake.take_log(), &events);
    }

    let caps = run(&device, |d| ctx.get_joystick_capabilities(d, 1)).0;
    assert_eq!(caps, JoystickCaps::RUMBLE);
    super::super::NUMJOYSTICKS.store(0, std::sync::atomic::Ordering::Relaxed);
    super::super::CHANGE_COUNT.store(0, std::sync::atomic::Ordering::Relaxed);
}

#[test]
fn state_packet_offsets() {
    let mut data = [0u8; 56];
    for (i, b) in data.iter_mut().enumerate() {
        *b = i as u8;
    }
    let packet = SteamDeckStatePacket::parse(&data);
    assert_eq!(packet.buttons_l(), 0x07060504);
    assert_eq!(packet.buttons_h(), 0x0b0a0908);
    assert_eq!(packet.trigger_raw_l, 0x2928);
    assert_eq!(packet.left_stick_x, 0x2d2c);
    assert_eq!(packet.pressure_pad_right, 0x3736);
}

#[test]
fn supported_devices() {
    let driver = SteamDeckDriver;
    let supported = |product_id| {
        driver.is_supported_device(
            None,
            "",
            GamepadType::Standard,
            USB_VENDOR_VALVE,
            product_id,
            0,
            0,
            0,
            0,
            0,
        )
    };
    assert!(supported(0x1205));
    assert!(!supported(0x1102));
}
