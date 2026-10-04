// Tests for the Steam Triton HIDAPI driver.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The transcript in `tests/` comes from running upstream's
//! SDL_hidapi_steam_triton.c, with stubs for the SDL and HID functions it
//! uses, on generated reports (see the Steam Controller driver's tests).
//! Its `advance` steps move the clock (`SDL_GetTicks()`) forward.

use super::super::steam::tests::{
    describe_open, hex_bytes, parse_transcript, result_line, Entry, FakeHid,
};
use super::super::tests::{run, test_device};
use super::*;
use crate::hidapi::DeviceInfo;

const TRANSCRIPT: &str = include_str!("tests/steam_triton.txt");

/// Replays the C transcript.
#[test]
fn transcript() {
    let _l = crate::test_support::test_lock();
    let _lock = crate::joystick::lock_joysticks();

    let mut device = test_device(&DeviceInfo::default());
    let fake = FakeHid::new(false);
    let mut ctx = SteamTritonContext::default();
    let mut joystick = None;
    let mut now = 100000;

    for entry in parse_transcript(TRANSCRIPT) {
        let step = match entry {
            Entry::Case(words) => {
                joystick = (words[1] == "1").then(|| device.joysticks()[0]);
                continue;
            }
            Entry::Step(step) => step,
        };
        fake.queue(&step);
        let words: Vec<&str> = step.head.split_whitespace().collect();
        let events = match words[0] {
            "init" => {
                device = test_device(&DeviceInfo {
                    vendor_id: USB_VENDOR_VALVE,
                    product_id: u16::from_str_radix(words[1], 16).unwrap(),
                    interface_number: 2,
                    ..DeviceInfo::default()
                });
                ctx = SteamTritonContext::default();
                let (result, mut events) = run(&device, |d| ctx.init_device(d));
                events.push(result_line(result.is_ok()));
                assert_eq!(device.name(), "Steam Controller");
                events
            }
            "open" => {
                let mut opened = JoystickData::new(1);
                let (result, mut events) = run(&device, |d| ctx.open_joystick(d, &mut opened));
                events.extend(describe_open(&opened));
                events.push(result_line(result.is_ok()));
                assert_eq!(opened.nbuttons, 18);
                events
            }
            "sensors" => {
                let result = ctx.set_sensors_enabled(&fake, words[1] == "1");
                vec![result_line(result.is_ok())]
            }
            "update" => {
                let (ok, mut events) = run(&device, |d| ctx.update(d, &fake, joystick, now));
                events.push(result_line(ok));
                events
            }
            "advance" => {
                now += words[1].parse::<u64>().unwrap();
                Vec::new()
            }
            "rumble" => {
                let result = ctx.rumble(
                    &fake,
                    words[1].parse().unwrap(),
                    words[2].parse().unwrap(),
                    now,
                );
                vec![result_line(result.is_ok())]
            }
            "effect" => {
                let data = hex_bytes(&words[1..].join(" "));
                vec![result_line(send_effect(&fake, &data).is_ok())]
            }
            other => panic!("unknown step {other}"),
        };
        step.check(&fake.take_log(), &events);
    }

    let caps = run(&device, |d| ctx.get_joystick_capabilities(d, 1)).0;
    assert_eq!(caps, JoystickCaps::RUMBLE);
    super::super::NUMJOYSTICKS.store(0, std::sync::atomic::Ordering::Relaxed);
    super::super::CHANGE_COUNT.store(0, std::sync::atomic::Ordering::Relaxed);
}

#[test]
fn report_offsets() {
    let mut data = [0u8; 45];
    for (i, b) in data.iter_mut().enumerate() {
        *b = i as u8;
    }
    let report = TritonMtuNoQuat::parse(&data);
    assert_eq!(report.buttons, 0x04030201);
    assert_eq!(report.right_stick_y, 0x100f);
    assert_eq!(report.pressure_right, 0x1c1b);
    assert_eq!(report.imu.timestamp, 0x201f1e1d);
    assert_eq!(report.imu.gyro_z, 0x2c2b);
    let report = TritonMtuNoQuat32Ts::parse(&data);
    assert_eq!(report.trackpad_timestamp, 0x1211);
    assert_eq!(report.pressure_right, 0x1e1d);
    assert_eq!(report.imu.timestamp, 0x201f);
    assert_eq!(report.imu.accel_x, 0x2221);
    let battery = TritonBatteryStatus::parse(&data);
    assert_eq!((battery.charge_state, battery.battery_level), (0, 1));
    assert_eq!(battery.temperature, 0x0d0c);
}

#[test]
fn supported_devices() {
    let driver = SteamTritonDriver;
    let supported = |product_id, interface_number| {
        driver.is_supported_device(
            None,
            "",
            GamepadType::Standard,
            USB_VENDOR_VALVE,
            product_id,
            0,
            interface_number,
            0,
            0,
            0,
        )
    };
    assert!(supported(0x1302, 0));
    // The controller interfaces of the Proteus and Nereid dongles
    assert!(supported(USB_PRODUCT_VALVE_STEAM_PROTEUS_DONGLE, 2));
    assert!(supported(USB_PRODUCT_VALVE_STEAM_NEREID_DONGLE, 5));
    assert!(!supported(USB_PRODUCT_VALVE_STEAM_PROTEUS_DONGLE, 1));
    assert!(!supported(USB_PRODUCT_VALVE_STEAM_NEREID_DONGLE, 6));
    assert!(!supported(0x1102, 2));
}
