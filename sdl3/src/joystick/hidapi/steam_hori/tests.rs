// Tests for the Steam HORI HIDAPI driver.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The transcript in `tests/` comes from running upstream's
//! SDL_hidapi_steam_hori.c, with stubs for the SDL functions it uses, on
//! generated reports (see the Steam Controller driver's tests).

use super::super::steam::tests::{describe_open, hex_bytes, parse_transcript, result_line, Entry};
use super::super::tests::{run, test_device};
use super::*;
use crate::hidapi::{BusType, DeviceInfo};

const TRANSCRIPT: &str = include_str!("tests/steam_hori.txt");

/// Replays the C transcript.
#[test]
fn transcript() {
    let _l = crate::test_support::test_lock();
    let _lock = crate::joystick::lock_joysticks();

    let mut device = None;
    let mut ctx = SteamHoriContext::default();

    for entry in parse_transcript(TRANSCRIPT) {
        let step = match entry {
            Entry::Case(_) => continue,
            Entry::Step(step) => step,
        };
        let words: Vec<&str> = step.head.split_whitespace().collect();
        let events = match words[0] {
            "init" => {
                let wireless = words[1] == "1";
                let new_device = test_device(&DeviceInfo {
                    vendor_id: USB_VENDOR_HORI,
                    product_id: if wireless {
                        USB_PRODUCT_HORI_STEAM_CONTROLLER_BT
                    } else {
                        USB_PRODUCT_HORI_STEAM_CONTROLLER
                    },
                    serial_number: wireless.then(|| "a1b2c3".to_owned()),
                    bus_type: if wireless {
                        BusType::Bluetooth
                    } else {
                        BusType::Usb
                    },
                    ..DeviceInfo::default()
                });
                ctx = SteamHoriContext::default();
                let (result, mut events) = run(&new_device, |d| ctx.init_device(d));
                events.push(result_line(result.is_ok()));
                assert_eq!(new_device.name(), "Wireless HORIPAD For Steam");
                // (the joystick isn't on the device in the C harness)
                let joystick = new_device.joysticks()[0];
                super::super::del_joystick_instance_from_device(&new_device, joystick);
                device = Some(new_device);
                events
            }
            "open" => {
                let mut joystick = JoystickData::new(1);
                let (result, mut events) = run(device.as_ref().unwrap(), |d| {
                    ctx.open_joystick(d, &mut joystick)
                });
                events.extend(describe_open(&joystick));
                events.push(result_line(result.is_ok()));
                if let Some(serial) = &joystick.serial {
                    events.push(format!("serial {serial}"));
                }
                assert_eq!(joystick.nbuttons, 16);
                events
            }
            "handle" => {
                let report = hex_bytes(step.inputs[0].strip_prefix("read ").unwrap());
                let mut data = [0u8; USB_PACKET_LENGTH];
                data[..report.len()].copy_from_slice(&report);
                let serial_needs_init = ctx.serial_needs_init;
                let ((), mut events) = run(device.as_ref().unwrap(), |d| {
                    ctx.handle_state_packet(d, 1, &data, report.len())
                });
                if serial_needs_init && !ctx.serial_needs_init {
                    events.push(format!("serial {}", usb_serial(&data)));
                }
                events.push(format!("ticks {} {}", ctx.sensor_ticks, ctx.last_tick));
                events
            }
            other => panic!("unknown step {other}"),
        };
        step.check(&[], &events);
    }

    super::super::NUMJOYSTICKS.store(0, std::sync::atomic::Ordering::Relaxed);
    super::super::CHANGE_COUNT.store(0, std::sync::atomic::Ordering::Relaxed);
}

#[test]
fn axes() {
    assert_eq!(read_stick_axis(0x80), 0);
    assert_eq!(read_stick_axis(0x00), i16::MIN);
    assert_eq!(read_stick_axis(0xff), i16::MAX);
    assert_eq!(read_trigger_axis(0), -32768);
    assert_eq!(read_trigger_axis(0xff), 32767);
}

#[test]
fn supported_devices() {
    let driver = SteamHoriDriver;
    let supported = |vendor_id, product_id| {
        driver.is_supported_device(
            None,
            "",
            GamepadType::Standard,
            vendor_id,
            product_id,
            0,
            0,
            0,
            0,
            0,
        )
    };
    assert!(supported(
        USB_VENDOR_HORI,
        USB_PRODUCT_HORI_STEAM_CONTROLLER
    ));
    assert!(supported(
        USB_VENDOR_HORI,
        USB_PRODUCT_HORI_STEAM_CONTROLLER_BT
    ));
    assert!(!supported(USB_VENDOR_HORI, 0x00c1));
}
