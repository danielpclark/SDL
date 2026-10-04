// Tests for the GameSir HIDAPI driver.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The cases in `data.rs` come from running upstream's SDL_hidapi_gamesir.c,
//! with stubs for the SDL and HID functions it uses (printing the events
//! and the reports it writes; there is no output interface, so the
//! commands go to the device, as on the platforms other than Windows), on
//! generated reports. Each case starts with the "init" and "open" calls of
//! the same name.

use super::super::steam::tests::{describe_open, hex_line};
use super::super::tests::{run, test_device};
use super::*;
use crate::hidapi::{BusType, DeviceInfo};

mod data;

fn call(name: &str) -> &'static [&'static str] {
    data::CALLS
        .iter()
        .find(|(n, _)| *n == name)
        .unwrap_or_else(|| panic!("no call {name}"))
        .1
}

/// The product and Bluetooth of a case's controller.
fn config(label: &str) -> (u16, bool) {
    match label {
        "g7 usb" => (USB_PRODUCT_GAMESIR_GAMEPAD_G7_PRO_8K, false),
        "tarantula usb" => (USB_PRODUCT_GAMESIR_GAMEPAD_TARANTULA_8K, false),
        "g7 bt" => (USB_PRODUCT_GAMESIR_GAMEPAD_G7_PRO_8K, true),
        _ => (0x1234, false),
    }
}

#[test]
fn supported_devices() {
    let driver = GameSirDriver;
    let device = |version| {
        test_device(&DeviceInfo {
            vendor_id: USB_VENDOR_GAMESIR,
            product_id: USB_PRODUCT_GAMESIR_GAMEPAD_TARANTULA_8K,
            release_number: version,
            ..DeviceInfo::default()
        })
    };
    let supported = |device: Option<&HidapiDevice>, pid| {
        driver.is_supported_device(
            device,
            "",
            GamepadType::Standard,
            USB_VENDOR_GAMESIR,
            pid,
            0,
            0,
            0,
            0,
            0,
        )
    };
    assert!(supported(None, USB_PRODUCT_GAMESIR_GAMEPAD_TARANTULA_8K));
    assert!(supported(None, USB_PRODUCT_GAMESIR_GAMEPAD_G7_PRO_8K));
    assert!(!supported(None, 0x1234));
    // This controller needs a firmware update
    assert!(!supported(
        Some(&device(553)),
        USB_PRODUCT_GAMESIR_GAMEPAD_TARANTULA_8K
    ));
    assert!(supported(
        Some(&device(554)),
        USB_PRODUCT_GAMESIR_GAMEPAD_TARANTULA_8K
    ));
}

#[test]
fn transcript() {
    let _l = crate::test_support::test_lock();
    let _lock = crate::joystick::lock_joysticks();

    for (label, reports) in data::CASES {
        let (product_id, bluetooth) = config(label);
        let device = test_device(&DeviceInfo {
            vendor_id: USB_VENDOR_GAMESIR,
            product_id,
            interface_number: 0,
            bus_type: if bluetooth {
                BusType::Bluetooth
            } else {
                BusType::Usb
            },
            ..DeviceInfo::default()
        });

        // The init call (without the output interface)
        let mut ctx = GameSirContext::default();
        let name = ctx.setup(product_id, bluetooth);
        run(&device, |d| d.set_device_name(name));
        let lines = [
            format!("name {name}"),
            "added".to_owned(),
            format!(
                "sensors {} led {} step {}",
                u8::from(ctx.sensors_supported),
                u8::from(ctx.led_supported),
                ctx.sensor_timestamp_step_ns
            ),
        ];
        assert_eq!(lines, call(&format!("init {label}")), "init {label}");

        // The open call
        let mut joystick = JoystickData::new(1);
        let (caps, mut lines) = run(&device, |d| {
            ctx.open_joystick(d, &mut joystick).unwrap();
            let sensors = ctx.set_joystick_sensors_enabled(d, 1, true);
            (sensors.is_ok(), ctx.get_joystick_capabilities(d, 1))
        });
        // (the stub's SDL_GetTicksNS())
        ctx.sensor_timestamp_ns = 1_000_000_000;
        // (the mode switch goes to the device, which the test one doesn't have)
        lines.push(hex_line("write", &mode_switch_report()));
        lines.extend(describe_open(&joystick));
        if !caps.0 {
            lines.push("unsupported".to_owned());
        }
        lines.push(format!(
            "buttons {} axes {} hats {} caps {} accel {:08x} gyro {:08x}",
            joystick.nbuttons,
            joystick.naxes,
            joystick.nhats,
            caps.1 .0,
            ctx.accel_scale.to_bits(),
            ctx.gyro_scale.to_bits()
        ));
        assert_eq!(lines, call(&format!("open {label}")), "open {label}");

        for (n, (report, expected)) in reports.iter().enumerate() {
            if n == 11 && ctx.sensors_supported {
                run(&device, |d| ctx.set_joystick_sensors_enabled(d, 1, false))
                    .0
                    .unwrap();
            }
            let mut data = [0u8; USB_PACKET_LENGTH];
            data[..report.len()].copy_from_slice(report);
            let ((), events) = run(&device, |d| ctx.handle_report(d, 1, &data, report.len()));
            assert_eq!(events, *expected, "{label} report {n}");
        }

        for (low, high) in [(0, 0), (0x1234, 0xabcd), (0xffff, 0x00ff)] {
            let name = format!("rumble {label} {low:04x} {high:04x}");
            let line = hex_line("write", &rumble_report(low, high));
            assert_eq!([line.as_str()], call(&name), "{name}");
        }
        let name = format!("led {label}");
        let line = if ctx.led_supported {
            hex_line("write", &led_report(0x12, 0x34, 0x56))
        } else {
            "unsupported".to_owned()
        };
        assert_eq!([line.as_str()], call(&name), "{name}");
    }
}
