// Tests for the SInput HIDAPI driver.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The cases in `data.rs` come from running upstream's SDL_hidapi_sinput.c,
//! with stubs for the SDL and HID functions it uses (printing the events
//! and the reports it writes, and reading from a queue as [`FakeHid`]
//! does), on generated feature replies and reports. Each case starts with
//! the "init" call of the same name (with the reports the controller sends
//! first) and its "open" call.

use super::super::steam::tests::{describe_open, hex_bytes, hex_line, FakeHid, Step};
use super::super::tests::{run, test_device};
use super::*;
use crate::hidapi::DeviceInfo;

mod data;

fn call(name: &str) -> &'static [&'static str] {
    data::CALLS
        .iter()
        .find(|(n, _)| *n == name)
        .unwrap_or_else(|| panic!("no call {name}"))
        .1
}

/// The vendor and product of a case's controller.
fn config(label: &str) -> (u16, u16) {
    match label {
        "full" => (
            USB_VENDOR_RASPBERRYPI,
            USB_PRODUCT_HANDHELDLEGEND_GCULTIMATE,
        ),
        "left only" => (USB_VENDOR_RASPBERRYPI, USB_PRODUCT_HANDHELDLEGEND_PROGCC),
        "right digital" => (USB_VENDOR_RASPBERRYPI, USB_PRODUCT_VOIDGAMING_PS4FIREBIRD),
        "dual stage" => (USB_VENDOR_ANDGAMER, USB_PRODUCT_VOIDGAMING_GENESIS_SINPUT),
        "one bumper" => (USB_VENDOR_RASPBERRYPI, USB_PRODUCT_BONZIRICHANNEL_FIREBIRD),
        _ => (
            USB_VENDOR_RASPBERRYPI,
            USB_PRODUCT_HANDHELDLEGEND_SINPUT_GENERIC,
        ),
    }
}

fn state_line(device: &HidapiDevice, ctx: &SInputContext) -> String {
    let guid = device.guid().0;
    format!(
        "type {} guid {:02x} {:02x} {:02x} buttons {} axes {} masks {:02x} {:02x} {:02x} {:02x} dpad {} accel {:08x} gyro {:08x}",
        device.gamepad_type() as i32,
        guid[12],
        guid[13],
        guid[15],
        ctx.buttons_count,
        ctx.axes_count,
        ctx.usage_masks[0],
        ctx.usage_masks[1],
        ctx.usage_masks[2],
        ctx.usage_masks[3],
        u8::from(ctx.dpad_supported),
        ctx.accel_scale.to_bits(),
        ctx.gyro_scale.to_bits(),
    )
}

/// The init call of a case; returns the device and its context.
fn check_init(label: &str) -> (std::sync::Arc<HidapiDevice>, SInputContext) {
    let (vendor_id, product_id) = config(label);
    let device = test_device(&DeviceInfo {
        vendor_id,
        product_id,
        interface_number: 0,
        ..DeviceInfo::default()
    });
    // (the C device starts without a type and a GUID)
    run(&device, |d| {
        d.set_gamepad_type(GamepadType::Unknown);
        for i in [12, 13, 15] {
            d.set_guid_byte(i, 0);
        }
    });

    let expected = call(&format!("init {label}"));
    let fake = FakeHid::new(false);
    fake.queue(&Step {
        inputs: expected
            .iter()
            .filter(|l| l.starts_with("read "))
            .take(if label == "no reply" { 1 } else { 2 })
            .map(|l| (*l).to_owned())
            .collect(),
        ..Step::default()
    });

    let mut ctx = SInputContext::default();
    let (result, mut lines) = run(&device, |d| ctx.init(d, &fake));
    lines.push(format!("result {}", u8::from(result.is_ok())));
    lines.push(state_line(&device, &ctx));

    let writes: Vec<&str> = expected
        .iter()
        .copied()
        .filter(|l| l.starts_with("write "))
        .collect();
    assert_eq!(fake.take_log(), writes, "writes of init {label}");
    let expected_lines: Vec<String> = expected
        .iter()
        .filter(|l| {
            !["read ", "write ", "name ", "serial ", "error "]
                .iter()
                .any(|p| l.starts_with(p))
        })
        // Note (upstream): upstream keeps SDL_GAMEPAD_TYPE_COUNT, which a
        // GamepadType can't be
        .map(|l| l.replace("type 13 ", "type 0 "))
        .collect();
    assert_eq!(lines, expected_lines, "init {label}");

    for line in expected {
        if let Some(serial) = line.strip_prefix("serial ") {
            assert_eq!(device.serial().as_deref(), Some(serial), "init {label}");
        }
        if let Some(name) = line.strip_prefix("name ") {
            assert_eq!(device.name(), name, "init {label}");
        }
    }
    (device, ctx)
}

#[test]
fn supported_devices() {
    let driver = SInputDriver;
    let supported = |vid, pid| {
        driver.is_supported_device(None, "", GamepadType::Standard, vid, pid, 0, 0, 0, 0, 0)
    };
    assert!(supported(
        USB_VENDOR_RASPBERRYPI,
        USB_PRODUCT_HANDHELDLEGEND_PROGCC
    ));
    assert!(supported(
        USB_VENDOR_ANDGAMER,
        USB_PRODUCT_VOIDGAMING_GENESIS_SINPUT
    ));
    assert!(!supported(
        USB_VENDOR_ANDGAMER,
        USB_PRODUCT_HANDHELDLEGEND_PROGCC
    ));
}

#[test]
fn transcript() {
    let _l = crate::test_support::test_lock();
    let _lock = crate::joystick::lock_joysticks();

    // (the failed init has no case)
    check_init("no reply");

    for (label, reports) in data::CASES {
        let (device, mut ctx) = check_init(label);

        // The open call
        let mut joystick = JoystickData::new(1);
        let (caps, mut lines) = run(&device, |d| {
            ctx.open_joystick(d, &mut joystick).unwrap();
            let sensors = ctx.set_joystick_sensors_enabled(d, 1, true);
            (sensors.is_ok(), ctx.get_joystick_capabilities(d, 1))
        });
        lines.extend(describe_open(&joystick));
        if !caps.0 {
            lines.push("unsupported".to_owned());
        }
        lines.push(format!(
            "buttons {} axes {} hats {} caps {}",
            joystick.nbuttons, joystick.naxes, joystick.nhats, caps.1 .0
        ));
        assert_eq!(lines, call(&format!("open {label}")), "open {label}");

        for (n, (report, expected)) in reports.iter().enumerate() {
            let mut data = [0u8; USB_PACKET_LENGTH];
            data[..report.len()].copy_from_slice(report);
            let ((), events) = run(&device, |d| ctx.handle_report(d, 1, &data, report.len()));
            assert_eq!(events, *expected, "{label} report {n}");
        }

        // The commands
        for (low, high) in [(0x1234, 0xabcd), (0xffff, 0x00ff)] {
            let name = format!("rumble {label} {low:04x} {high:04x}");
            let line = if ctx.rumble_supported {
                hex_line("rumble", &rumble_report(low, high))
            } else {
                "unsupported".to_owned()
            };
            assert_eq!([line.as_str()], call(&name), "{name}");
        }
        let name = format!("led {label}");
        let line = ctx
            .joystick_rgb_command(0x12, 0x34, 0x56)
            .map_or("unsupported".to_owned(), |c| hex_line("write", &c));
        assert_eq!([line.as_str()], call(&name), "{name}");
        for player in [-1, 0, 3, 300] {
            let name = format!("player {label} {player}");
            let lines: Vec<String> = ctx
                .player_led_command(player)
                .iter()
                .map(|c| hex_line("write", c))
                .collect();
            assert_eq!(lines, call(&name), "{name}");
        }
    }

    super::super::NUMJOYSTICKS.store(0, std::sync::atomic::Ordering::Relaxed);
    super::super::CHANGE_COUNT.store(0, std::sync::atomic::Ordering::Relaxed);
}

#[test]
fn scales() {
    assert_eq!(calculate_gyro_scale(0), 0.0);
    assert_eq!(calculate_accel_scale(32768), STANDARD_GRAVITY);
    assert_eq!(
        hex_bytes("03 01 02 12 00 ab 00"),
        rumble_report(0x1234, 0xabcd)[..7]
    );
}
