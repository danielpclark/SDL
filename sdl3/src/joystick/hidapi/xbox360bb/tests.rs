// Tests for the Xbox 360 Big Button HIDAPI driver.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The cases in `data.rs` come from running upstream's
//! SDL_hidapi_xbox360bb.c, with stubs for the SDL functions it uses, on
//! generated reports.

use super::super::tests::{run, test_device};
use super::*;
use crate::hidapi::DeviceInfo;

mod data;

fn device() -> std::sync::Arc<HidapiDevice> {
    test_device(&DeviceInfo {
        vendor_id: USB_VENDOR_MICROSOFT,
        product_id: USB_PRODUCT_XBOX360_BIGBUTTON_RECEIVER,
        interface_number: 0,
        ..DeviceInfo::default()
    })
}

fn call(name: &str) -> &'static [&'static str] {
    data::CALLS
        .iter()
        .find(|(n, _)| *n == name)
        .unwrap_or_else(|| panic!("no call {name}"))
        .1
}

#[test]
fn supported_devices() {
    let driver = Xbox360BbDriver;
    let supported = |vid, pid| {
        driver.is_supported_device(None, "", GamepadType::Xbox360, vid, pid, 0, 0, 0, 0, 0)
    };
    assert!(supported(
        USB_VENDOR_MICROSOFT,
        USB_PRODUCT_XBOX360_BIGBUTTON_RECEIVER
    ));
    assert!(!supported(
        USB_VENDOR_MICROSOFT,
        USB_PRODUCT_XBOX360_WIRELESS_RECEIVER
    ));
}

#[test]
fn reports_and_releases() {
    let _l = crate::test_support::test_lock();
    let _lock = crate::joystick::lock_joysticks();

    let device = device();
    let mut ctx = Xbox360BbContext::default();
    let (result, events) = run(&device, |d| ctx.init_device(d));
    assert!(result.is_ok());
    assert_eq!(device.name(), "Xbox 360 Big Button Controller");
    let expected: Vec<&str> = call("init")
        .iter()
        .copied()
        .filter(|e| !e.starts_with("name "))
        .collect();
    assert_eq!(events, expected);
    assert!(ctx.joysticks.iter().all(|&j| j != 0));

    for (name, reports) in data::CASES {
        for (n, (report, expected)) in reports.iter().enumerate() {
            let mut data = [0u8; USB_PACKET_LENGTH];
            data[..report.len()].copy_from_slice(report);
            let ((), events) = run(&device, |d| ctx.handle_state_packet(d, &data, report.len()));
            assert_eq!(events, *expected, "{name} report {n}");
        }
    }

    // The receiver only reports buttons that are down
    const NOW: u64 = 1_000_000_000;
    ctx.last_packet = [NOW, NOW - 119_999_999, NOW - 120_000_000, 0];
    let joysticks = ctx.joysticks;
    ctx.joysticks[3] = 0;
    let ((), mut events) = run(&device, |d| ctx.handle_release_events(d, NOW, |j| j != 0));
    for i in 0..MAX_CONTROLLERS {
        let state = ctx.last_state[i][3] | ctx.last_state[i][4];
        events.push(format!("slot {i} {} {state}", ctx.last_packet[i]));
    }
    assert_eq!(events, call("release"));

    // An invalid controller index is ignored
    let ((), events) = run(&device, |d| {
        ctx.handle_state_packet(d, &[0x00, 0x05, 0x04, 0xff, 0xff], 5)
    });
    assert!(events.is_empty());

    let (_, _) = run(&device, |d| {
        for joystick in joysticks {
            d.joystick_disconnected(joystick);
        }
    });
    super::super::NUMJOYSTICKS.store(0, std::sync::atomic::Ordering::Relaxed);
    super::super::CHANGE_COUNT.store(0, std::sync::atomic::Ordering::Relaxed);
}
