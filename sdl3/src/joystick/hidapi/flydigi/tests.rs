// Tests for the Flydigi HIDAPI driver.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The cases in `data.rs` come from running upstream's
//! SDL_hidapi_flydigi.c, with stubs for the SDL and HID functions it uses
//! (printing the events and the reports it writes, and reading from a
//! queue as [`FakeHid`] does), on generated replies and reports. Each case
//! starts with the "init" call of the same name (with the replies of the
//! controller) and its "open" call; the reports of a case are read by one
//! update each, 40 ms apart.

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

/// The reports written, and the other lines, of an expected output.
fn split(lines: &[&str]) -> (Vec<String>, Vec<String>) {
    let (writes, others): (Vec<&str>, Vec<&str>) =
        lines.iter().partition(|l| l.starts_with("write "));
    (
        writes.into_iter().map(str::to_owned).collect(),
        others.into_iter().map(str::to_owned).collect(),
    )
}

/// Queue the reports the fake device reads next.
fn queue(fake: &FakeHid, reads: &[Vec<u8>]) {
    fake.queue(&Step {
        inputs: reads.iter().map(|r| hex_line("read", r)).collect(),
        ..Step::default()
    });
}

/// The vendor, product and name of a case's controller.
fn config(label: &str) -> (u16, u16, &'static str) {
    let v1 = |name| (USB_VENDOR_FLYDIGI_V1, USB_PRODUCT_FLYDIGI_V1_GAMEPAD, name);
    let v2 = |product, name| (USB_VENDOR_FLYDIGI_V2, product, name);
    match label {
        "v1 name vader2" => v1("Flydigi VADER2"),
        "v1 name vader" => v1("FLYDIGI Vader3 controller"),
        "v1 name apex" => v1("Flydigi APEX4"),
        _ if label.starts_with("v1 ") => v1("Flydigi"),
        "v2 vader5" | "v2 vader old firmware" => {
            v2(USB_PRODUCT_FLYDIGI_V2_VADER, "Flydigi Vader 5 Pro")
        }
        "v2 apex6" => v2(USB_PRODUCT_FLYDIGI_V2_APEX6, "Flydigi"),
        "v2 vader5 by name" => v2(USB_PRODUCT_FLYDIGI_V2_APEX6, "Flydigi Vader 5 Pro"),
        _ => v2(USB_PRODUCT_FLYDIGI_V2_APEX, "Flydigi"),
    }
}

fn state_line(device: &HidapiDevice, ctx: &FlydigiContext) -> String {
    format!(
        "guid15 {} id {} fw {:04x} wireless {} cz {} lmrm {} circle {} sensors {} step {} accel {:08x} gyro {:08x} available {}",
        device.guid().0[15],
        ctx.device_id,
        ctx.firmware_version,
        u8::from(ctx.wireless),
        u8::from(ctx.has_cz),
        u8::from(ctx.has_lmrm),
        u8::from(ctx.has_circle),
        u8::from(ctx.sensors_supported),
        ctx.sensor_timestamp_step_ns,
        ctx.accel_scale.to_bits(),
        ctx.gyro_scale.to_bits(),
        u8::from(ctx.available),
    )
}

/// Check the init call of a case; returns the device and its context.
fn check_init(label: &str) -> (std::sync::Arc<HidapiDevice>, FlydigiContext) {
    let (vendor_id, product_id, name) = config(label);
    let device = test_device(&DeviceInfo {
        vendor_id,
        product_id,
        interface_number: 2,
        ..DeviceInfo::default()
    });
    run(&device, |d| d.set_device_name(name));

    let expected = call(&format!("init {label}"));
    let replies: Vec<Vec<u8>> = expected
        .iter()
        .filter_map(|l| l.strip_prefix("reply "))
        .map(hex_bytes)
        // (the replies are 32 bytes long)
        .collect();
    let fake = FakeHid::new(false);
    if label != "v2 no reply" {
        queue(&fake, &replies);
    }

    let mut ctx = FlydigiContext::default();
    let (result, mut lines) = run(&device, |d| ctx.init(d, &fake));
    if vendor_id == USB_VENDOR_FLYDIGI_V2 {
        lines.push(format!("result {}", u8::from(result.is_ok())));
    }
    lines.push(state_line(&device, &ctx));

    let (expected_writes, others) = split(expected);
    assert_eq!(fake.take_log(), expected_writes, "writes of init {label}");
    let expected_lines: Vec<&String> = others
        .iter()
        .filter(|l| {
            !l.starts_with("reply ")
                && !l.starts_with("name ")
                && !l.starts_with("serial ")
                && !l.starts_with("error ")
        })
        .collect();
    assert_eq!(
        lines.iter().collect::<Vec<_>>(),
        expected_lines,
        "init {label}"
    );

    for line in others.iter() {
        if let Some(serial) = line.strip_prefix("serial ") {
            assert_eq!(device.serial().as_deref(), Some(serial), "init {label}");
        }
    }
    if let Some(name) = others.iter().rev().find_map(|l| l.strip_prefix("name ")) {
        assert_eq!(device.name(), name, "init {label}");
    }
    (device, ctx)
}

#[test]
fn supported_devices() {
    let driver = FlydigiDriver;
    let supported = |vid, pid, interface| {
        driver.is_supported_device(
            None,
            "",
            GamepadType::Standard,
            vid,
            pid,
            0,
            interface,
            0,
            0,
            0,
        )
    };
    // Early controllers have their custom protocol on interface 2
    assert!(supported(
        USB_VENDOR_FLYDIGI_V1,
        USB_PRODUCT_FLYDIGI_V1_GAMEPAD,
        2
    ));
    assert!(!supported(
        USB_VENDOR_FLYDIGI_V1,
        USB_PRODUCT_FLYDIGI_V1_GAMEPAD,
        1
    ));
    assert!(supported(
        USB_VENDOR_FLYDIGI_V2,
        USB_PRODUCT_FLYDIGI_V2_VADER,
        1
    ));
    assert!(supported(
        USB_VENDOR_FLYDIGI_V2,
        USB_PRODUCT_FLYDIGI_V2_APEX6,
        -1
    ));
    assert!(!supported(USB_VENDOR_FLYDIGI_V2, 0x1234, 1));
}

#[test]
fn controller_types() {
    use FlydigiControllerType::*;
    assert_eq!(controller_type_of(152, ""), Apex6);
    assert_eq!(controller_type_of(105, ""), Vader4Pro);
    assert_eq!(controller_type_of(1, "Flydigi vader4"), Unknown);
    assert_eq!(controller_type_of(1, "Flydigi vader VADER4"), Vader4Pro);
    assert_eq!(controller_type_of(1, "apex APEX5"), Apex5);
    assert_eq!(controller_type_of(1, "Apex apex5"), Unknown);
    assert_eq!(Vader5Pro as u8, 0x15);
}

#[test]
fn transcript() {
    let _l = crate::test_support::test_lock();
    let _lock = crate::joystick::lock_joysticks();

    for (label, reports) in data::CASES {
        let (device, mut ctx) = check_init(label);

        // The open call
        let mut joystick = JoystickData::new(1);
        let (sensors, mut lines) = run(&device, |d| {
            ctx.open_joystick(d, &mut joystick).unwrap();
            ctx.set_joystick_sensors_enabled(d, 1, true)
        });
        lines.extend(describe_open(&joystick));
        if sensors.is_err() {
            lines.push("unsupported".to_owned());
        }
        lines.push(format!(
            "buttons {} axes {} hats {} connection {}",
            joystick.nbuttons, joystick.naxes, joystick.nhats, joystick.connection_state as i32
        ));
        assert_eq!(lines, call(&format!("open {label}")), "open {label}");

        let fake = FakeHid::new(false);
        for (n, (report, expected)) in reports.iter().enumerate() {
            if report.is_empty() {
                queue(&fake, &[]);
            } else {
                queue(&fake, &[report.to_vec()]);
            }
            let now = 1000 + 40 * (n as u64 + 1);
            let joystick = device.joysticks().first().copied();
            let (ok, events) = run(&device, |d| ctx.update(d, &fake, joystick, now));
            assert!(ok);
            let (expected_writes, expected_events) = split(expected);
            assert_eq!(
                fake.take_log(),
                expected_writes,
                "{label} report {n} writes"
            );
            assert_eq!(events, expected_events, "{label} report {n}");
        }
    }

    super::super::NUMJOYSTICKS.store(0, std::sync::atomic::Ordering::Relaxed);
    super::super::CHANGE_COUNT.store(0, std::sync::atomic::Ordering::Relaxed);
}

#[test]
fn rumble_packets() {
    for (name, expected) in data::CALLS.iter().filter(|(n, _)| n.starts_with("rumble ")) {
        let words: Vec<u16> = name
            .split(' ')
            .skip(1)
            .map(|w| u16::from_str_radix(w, 16).unwrap())
            .collect();
        let packet = rumble_packet(words[0], words[1], words[2]);
        assert_eq!([hex_line("rumble", &packet)], *expected, "{name}");
    }
}

#[test]
fn unnumbered_reports() {
    let device = |product_id| {
        test_device(&DeviceInfo {
            vendor_id: USB_VENDOR_FLYDIGI_V2,
            product_id,
            interface_number: 1,
            ..DeviceInfo::default()
        })
    };
    let fake = FakeHid::new(false);
    // The Vader 5 and Apex 6 take their commands without the report ID
    write_packet(
        &device(USB_PRODUCT_FLYDIGI_V2_VADER),
        &fake,
        &status_command(),
    )
    .unwrap();
    write_packet(
        &device(USB_PRODUCT_FLYDIGI_V2_APEX),
        &fake,
        &status_command(),
    )
    .unwrap();
    write_packet(&device(USB_PRODUCT_FLYDIGI_V2_APEX6), &fake, &[0x05, 0x01]).unwrap();
    assert_eq!(
        fake.take_log(),
        ["write 00 5a a5 10", "write 03 5a a5 10", "write 05 01"]
    );
    assert_eq!(
        acquire_command(false)[..9],
        [3, 0x5a, 0xa5, 0x1c, 23, 0, b'S', b'D', b'L']
    );
    assert_eq!(acquire_command(true).len(), 32);
}
