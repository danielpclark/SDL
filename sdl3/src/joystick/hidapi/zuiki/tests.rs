// Tests for the ZUIKI HIDAPI driver.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The cases in `data.rs` come from running upstream's SDL_hidapi_zuiki.c,
//! with stubs for the SDL functions it uses, on generated reports. Each
//! case starts with the "init" call of the same name, with the first
//! report of the controller.

use super::super::tests::{run, test_device};
use super::*;
use crate::hidapi::DeviceInfo;

mod data;

fn device(product_id: u16) -> std::sync::Arc<HidapiDevice> {
    test_device(&DeviceInfo {
        vendor_id: USB_VENDOR_ZUIKI,
        product_id,
        interface_number: 0,
        ..DeviceInfo::default()
    })
}

fn hex_line(tag: &str, data: &[u8]) -> String {
    data.iter()
        .fold(tag.to_owned(), |s, b| s + &format!(" {b:02x}"))
}

fn call(name: &str) -> &'static [&'static str] {
    data::CALLS
        .iter()
        .find(|(n, _)| *n == name)
        .unwrap_or_else(|| panic!("no call {name}"))
        .1
}

fn product(label: &str) -> u16 {
    match label {
        "mascon" => USB_PRODUCT_ZUIKI_MASCON_PRO,
        "evotop pc" => USB_PRODUCT_ZUIKI_EVOTOP_PC_DINPUT,
        "evotop uwb" => USB_PRODUCT_ZUIKI_EVOTOP_UWB_DINPUT,
        "evotop bt" | "evotop bt nosensors" => USB_PRODUCT_ZUIKI_EVOTOP_PC_BT,
        "axis" => USB_PRODUCT_ZUIKI_EVOTOP_AXIS_DINPUT,
        _ => panic!("unknown case {label}"),
    }
}

#[test]
fn supported_devices() {
    let driver = ZuikiDriver;
    let supported = |vid, pid| {
        driver.is_supported_device(None, "", GamepadType::Standard, vid, pid, 0, 0, 0, 0, 0)
    };
    assert!(supported(USB_VENDOR_ZUIKI, USB_PRODUCT_ZUIKI_MASCON_PRO));
    assert!(supported(USB_VENDOR_ZUIKI, USB_PRODUCT_ZUIKI_EVOTOP_PC_BT));
    assert!(!supported(USB_VENDOR_ZUIKI, 0x0001));
}

#[test]
fn state_reports() {
    for (label, reports) in data::CASES {
        let product_id = product(label);
        let device = device(product_id);
        let mut ctx = ZuikiContext::default();

        // The first report
        let init = call(&format!("init {label}"));
        let first: Vec<u8> = init[0]
            .split(' ')
            .skip(1)
            .map(|b| u8::from_str_radix(b, 16).unwrap())
            .collect();
        let mut data = [0u8; USB_PACKET_LENGTH * 2];
        data[..first.len()].copy_from_slice(&first);
        let mut lines = vec![init[0].to_owned()];
        if let Some(name) = ctx.apply_first_report(product_id, &data) {
            lines.push(format!("name {name}"));
        }
        lines.push("added".to_owned());
        lines.push(format!(
            "sensors {} rate {:08x}",
            u8::from(ctx.sensors_supported),
            ctx.sensor_rate.to_bits()
        ));
        assert_eq!(lines, init, "init {label}");

        for (n, (report, expected)) in reports.iter().enumerate() {
            let mut data = [0u8; USB_PACKET_LENGTH];
            data[..report.len()].copy_from_slice(report);
            let before = crate::timer::ticks_ns();
            let ((), events) = run(&device, |d| ctx.handle_report(d, 1, &data, report.len()));
            let after = crate::timer::ticks_ns();
            // (a sensor timestamp of the time the report came in is NOW)
            let events: Vec<String> = events
                .into_iter()
                .map(|e| {
                    let mut words: Vec<String> = e.split(' ').map(str::to_owned).collect();
                    if words[0] == "sensor" {
                        let t: u64 = words[2].parse().unwrap();
                        if (before..=after).contains(&t) {
                            words[2] = "NOW".to_owned();
                        }
                    }
                    words.join(" ")
                })
                .collect();
            assert_eq!(events, *expected, "{label} report {n}");
        }
    }
}

#[test]
fn rumble_packets() {
    for (low, high) in [(0u16, 0u16), (0x1234, 0xabcd), (0xffff, 0x00ff)] {
        let name = format!("rumble {low:04x} {high:04x}");
        assert_eq!(
            [hex_line("rumble", &rumble_packet(low, high))],
            call(&name),
            "{name}"
        );
    }
    let mut ctx = ZuikiContext::default();
    let (caps, _) = run(&device(USB_PRODUCT_ZUIKI_EVOTOP_AXIS_DINPUT), |d| {
        ctx.get_joystick_capabilities(d, 1)
    });
    assert_eq!(caps, JoystickCaps(0));
    let (caps, _) = run(&device(USB_PRODUCT_ZUIKI_MASCON_PRO), |d| {
        ctx.get_joystick_capabilities(d, 1)
    });
    assert_eq!(caps, JoystickCaps::RUMBLE);
}

#[test]
fn median_filter() {
    let mut filter = MedianFilter::default();
    assert_eq!(filter.update(5.0), 5.0);
    assert_eq!(filter.update(1.0), 5.0);
    assert_eq!(filter.update(3.0), 3.0);
    for _ in 0..FILTER_SIZE {
        filter.update(-1.0);
    }
    assert_eq!(filter.update(100.0), -1.0);
    assert_eq!(filter.count, FILTER_SIZE);
}

#[test]
fn out_of_range_triggers() {
    // (undefined upstream; see read_bt_axis())
    assert_eq!(read_bt_axis(0xff, 0x03, 0x03ff), i16::MAX);
    assert_eq!(read_bt_axis(0x00, 0x04, 0x03ff), i16::MAX);
    assert_eq!(read_bt_axis(0x00, 0x00, 0x03ff), i16::MIN);
}
