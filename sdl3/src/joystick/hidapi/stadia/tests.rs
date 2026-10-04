// Tests for the Stadia HIDAPI driver.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The cases in `data.rs` come from running upstream's SDL_hidapi_stadia.c,
//! with stubs for the SDL functions it uses, on generated reports.

use super::super::tests::{run, test_device};
use super::*;
use crate::error::ErrorKind;
use crate::hidapi::DeviceInfo;
use crate::joystick::usb_ids::*;

mod data;

fn device() -> std::sync::Arc<HidapiDevice> {
    test_device(&DeviceInfo {
        vendor_id: USB_VENDOR_GOOGLE,
        product_id: USB_PRODUCT_GOOGLE_STADIA_CONTROLLER,
        interface_number: 0,
        ..DeviceInfo::default()
    })
}

fn hex_line(tag: &str, data: &[u8]) -> String {
    data.iter()
        .fold(tag.to_owned(), |s, b| s + &format!(" {b:02x}"))
}

#[test]
fn supported_devices() {
    let driver = StadiaDriver;
    let supported = |vid, pid| {
        driver.is_supported_device(None, "", GamepadType::Standard, vid, pid, 0, 0, 0, 0, 0)
    };
    assert!(supported(
        USB_VENDOR_GOOGLE,
        USB_PRODUCT_GOOGLE_STADIA_CONTROLLER
    ));
    assert!(!supported(USB_VENDOR_GOOGLE, 0x0001));
}

#[test]
fn state_reports() {
    let device = device();
    for (name, reports) in data::CASES {
        let mut ctx = StadiaContext::default();
        for (n, (report, expected)) in reports.iter().enumerate() {
            let mut data = [0u8; USB_PACKET_LENGTH];
            data[..report.len()].copy_from_slice(report);
            let ((), events) = run(&device, |d| {
                ctx.handle_state_packet(d, 1, &data, report.len())
            });
            assert_eq!(events, *expected, "{name} report {n}");
        }
    }
}

#[test]
fn rumble_packets() {
    for (name, expected) in data::CALLS {
        let words: Vec<&str> = name.split(' ').collect();
        let line = match words[1] {
            "unsupported" => {
                let mut ctx = StadiaContext::default();
                let (result, _) = run(&device(), |d| ctx.rumble_joystick(d, 1, 1, 1));
                assert_eq!(result.unwrap_err().kind(), ErrorKind::Unsupported);
                "unsupported".to_owned()
            }
            _ => {
                let low = u16::from_str_radix(words[1], 16).unwrap();
                let high = u16::from_str_radix(words[2], 16).unwrap();
                hex_line("rumble", &rumble_packet(low, high))
            }
        };
        assert_eq!([line.as_str()], *expected, "{name}");
    }
}

#[test]
fn out_of_range_stick() {
    // (undefined upstream; see read_stick_axis())
    assert_eq!(read_stick_axis(0x00), i16::MIN);
    assert_eq!(read_stick_axis(0x01), i16::MIN);
    assert_eq!(read_stick_axis(0x80), 0);
    assert_eq!(read_stick_axis(0xff), i16::MAX);
}
