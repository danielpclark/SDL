// Tests for the Luna HIDAPI driver.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The cases in `data.rs` come from running upstream's SDL_hidapi_luna.c,
//! with stubs for the SDL functions it uses, on generated reports.

use super::super::tests::{run, test_device};
use super::*;
use crate::hidapi::{BusType, DeviceInfo};

mod data;

fn device(bluetooth: bool) -> std::sync::Arc<HidapiDevice> {
    test_device(&DeviceInfo {
        vendor_id: if bluetooth {
            BLUETOOTH_VENDOR_AMAZON
        } else {
            USB_VENDOR_AMAZON
        },
        product_id: USB_PRODUCT_AMAZON_LUNA_CONTROLLER,
        interface_number: 0,
        bus_type: if bluetooth {
            BusType::Bluetooth
        } else {
            BusType::Usb
        },
        ..DeviceInfo::default()
    })
}

fn hex_line(tag: &str, data: &[u8]) -> String {
    data.iter()
        .fold(tag.to_owned(), |s, b| s + &format!(" {b:02x}"))
}

#[test]
fn supported_devices() {
    let driver = LunaDriver;
    let supported = |vid, pid| {
        driver.is_supported_device(None, "", GamepadType::Standard, vid, pid, 0, 0, 0, 0, 0)
    };
    assert!(supported(
        USB_VENDOR_AMAZON,
        USB_PRODUCT_AMAZON_LUNA_CONTROLLER
    ));
    assert!(supported(
        BLUETOOTH_VENDOR_AMAZON,
        BLUETOOTH_PRODUCT_LUNA_CONTROLLER
    ));
    assert!(!supported(USB_VENDOR_AMAZON, 0x0001));
}

#[test]
fn state_reports() {
    for (name, reports) in data::CASES {
        let device = device(*name == "bluetooth");
        let mut ctx = LunaContext::default();
        for (n, (report, expected)) in reports.iter().enumerate() {
            let mut data = [0u8; USB_PACKET_LENGTH];
            data[..report.len()].copy_from_slice(report);
            let ((), events) = run(&device, |d| ctx.handle_report(d, 1, &data, report.len()));
            assert_eq!(events, *expected, "{name} report {n}");
        }
    }
}

#[test]
fn rumble_packets() {
    for (name, expected) in data::CALLS {
        let words: Vec<&str> = name.split(' ').collect();
        let (low, high) = match words[1] {
            "usb" => {
                // (upstream sends the Bluetooth packet to USB controllers too)
                assert!(has_bluetooth_rumble(&device(false)) == ENABLE_LUNA_BLUETOOTH_RUMBLE);
                (1, 1)
            }
            _ => (
                u16::from_str_radix(words[1], 16).unwrap(),
                u16::from_str_radix(words[2], 16).unwrap(),
            ),
        };
        let line = hex_line("rumble", &bluetooth_rumble_packet(low, high));
        assert_eq!([line.as_str()], *expected, "{name}");
    }
    let mut ctx = LunaContext::default();
    let (caps, _) = run(&device(true), |d| ctx.get_joystick_capabilities(d, 1));
    assert_eq!(
        caps.contains(JoystickCaps::RUMBLE),
        ENABLE_LUNA_BLUETOOTH_RUMBLE
    );
}
