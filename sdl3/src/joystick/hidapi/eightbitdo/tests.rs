// Tests for the 8BitDo HIDAPI driver.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The cases in `data.rs` come from running upstream's SDL_hidapi_8bitdo.c,
//! with stubs for the SDL functions it uses, on generated reports. Each
//! case is opened by the "open" call of the same name.

use super::super::tests::{run, test_device};
use super::*;
use crate::hidapi::{BusType, DeviceInfo};

mod data;

fn device(product_id: u16, bluetooth: bool) -> std::sync::Arc<HidapiDevice> {
    test_device(&DeviceInfo {
        vendor_id: USB_VENDOR_8BITDO,
        product_id,
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

fn call(name: &str) -> &'static [&'static str] {
    data::CALLS
        .iter()
        .find(|(n, _)| *n == name)
        .unwrap_or_else(|| panic!("no call {name}"))
        .1
}

/// The configuration of a case: product, Bluetooth, sensors, battery and
/// sensor timestamps supported.
fn config(label: &str) -> (u16, bool, bool, bool, bool) {
    match label {
        "old" => (USB_PRODUCT_8BITDO_SN30_PRO, false, false, false, false),
        "basic" => (USB_PRODUCT_8BITDO_SF30_PRO_BT, true, false, false, false),
        "pro2" => (USB_PRODUCT_8BITDO_PRO_2, false, true, true, true),
        "ultimate2" => (
            USB_PRODUCT_8BITDO_ULTIMATE2_WIRELESS,
            true,
            true,
            true,
            false,
        ),
        "ultimate3" => (USB_PRODUCT_8BITDO_ULTIMATE3, false, true, true, false),
        "unsupported" => (USB_PRODUCT_8BITDO_SF30_PRO, false, false, false, false),
        _ => panic!("unknown case {label}"),
    }
}

#[test]
fn supported_devices() {
    let driver = EightBitDoDriver;
    let supported = |vid, pid| {
        driver.is_supported_device(None, "", GamepadType::Standard, vid, pid, 0, 0, 0, 0, 0)
    };
    assert!(supported(USB_VENDOR_8BITDO, USB_PRODUCT_8BITDO_PRO_2));
    assert!(supported(USB_VENDOR_8BITDO, USB_PRODUCT_8BITDO_ULTIMATE3));
    assert!(!supported(USB_VENDOR_8BITDO, 0x3106));
    assert!(!supported(USB_VENDOR_NINTENDO, USB_PRODUCT_8BITDO_PRO_2));
}

#[test]
fn state_reports() {
    let _l = crate::test_support::test_lock();
    let _lock = crate::joystick::lock_joysticks();

    for (label, reports) in data::CASES {
        let (product_id, bluetooth, sensors, power, timestamp) = config(label);
        let device = device(product_id, bluetooth);
        let mut ctx = EightBitDoContext {
            sensors_supported: sensors,
            powerstate_supported: power,
            sensor_timestamp_supported: timestamp,
            ..EightBitDoContext::default()
        };

        // The open call
        let mut joystick = JoystickData::new(1);
        let (result, _) = run(&device, |d| {
            ctx.open_joystick(d, &mut joystick).unwrap();
            ctx.set_joystick_sensors_enabled(d, 1, true)
        });
        let mut lines: Vec<String> = joystick
            .sensors
            .iter()
            .map(|s| {
                let sensor_type = match s.sensor_type {
                    SensorType::Accel => 1,
                    SensorType::Gyro => 2,
                    _ => 0,
                };
                format!("add sensor {sensor_type} {:08x}", s.rate.to_bits())
            })
            .collect();
        lines.push(format!(
            "buttons {} interval {} gyro {:08x} accel {:08x}",
            joystick.nbuttons,
            ctx.sensor_timestamp_interval,
            ctx.gyro_scale.to_bits(),
            ctx.accel_scale.to_bits()
        ));
        if result.is_err() {
            lines.push("unsupported".to_owned());
        }
        assert_eq!(lines, call(&format!("open {label}")), "open {label}");

        for (n, (report, expected)) in reports.iter().enumerate() {
            let mut data = [0u8; USB_PACKET_LENGTH];
            data[..report.len()].copy_from_slice(report);
            let ((), events) = run(&device, |d| ctx.handle_report(d, 1, &data, report.len()));
            assert_eq!(events, *expected, "{label} report {n}");
        }
    }
}

#[test]
fn rumble_packets() {
    let mut ctx = EightBitDoContext {
        rumble_supported: true,
        ..EightBitDoContext::default()
    };
    type Setup = fn(&mut EightBitDoContext);
    let rumbles: [(&str, Setup); 7] = [
        ("rumble 1234 abcd", |_| {}),
        ("triggers 1234 abcd", |_| {}),
        ("rumble 5678 9abc", |ctx| {
            ctx.trigger_rumble_supported = true
        }),
        ("triggers ff00 00ff", |_| {}),
        ("rumble 0000 ffff", |_| {}),
        ("triggers 4000 8000", |ctx| ctx.rumble_supported = false),
        ("rumble 4000 8000", |_| {}),
    ];
    for (name, setup) in rumbles {
        setup(&mut ctx);
        let words: Vec<&str> = name.split(' ').collect();
        let low = u16::from_str_radix(words[1], 16).unwrap();
        let high = u16::from_str_radix(words[2], 16).unwrap();
        let packet = if words[0] == "rumble" {
            ctx.rumble_packet(low, high)
        } else {
            ctx.trigger_rumble_packet(low, high)
        };
        let line = packet.map_or("unsupported".to_owned(), |p| hex_line("rumble", &p));
        assert_eq!([line.as_str()], call(name), "{name}");
    }
}

#[test]
fn feature_reports() {
    let mut data = [0u8; USB_PACKET_LENGTH];
    for (i, b) in data.iter_mut().enumerate() {
        *b = (i * 17 + 3) as u8;
    }
    type Setup = fn(&mut [u8; USB_PACKET_LENGTH]);
    let features: [(&str, u16, usize, Setup); 9] = [
        ("features pro2 14", USB_PRODUCT_8BITDO_PRO_2, 14, |d| {
            d[13] = 0xAA
        }),
        ("features sn30 64", USB_PRODUCT_8BITDO_SN30_PRO, 64, |d| {
            d[13] = 0xAB
        }),
        ("features sf30 11", USB_PRODUCT_8BITDO_SF30_PRO, 11, |_| {}),
        ("features pro3 12", USB_PRODUCT_8BITDO_PRO_3, 12, |d| {
            d[10] = 0
        }),
        ("features pro2bt 0", USB_PRODUCT_8BITDO_PRO_2_BT, 0, |_| {}),
        ("ultimate3 17", USB_PRODUCT_8BITDO_ULTIMATE3, 17, |d| {
            d[3] = 0x03;
            d[4] = 0x01;
        }),
        ("ultimate3 16", USB_PRODUCT_8BITDO_ULTIMATE3, 16, |d| {
            d[3] = 0x00;
            d[4] = 0x00;
        }),
        ("ultimate3 64", USB_PRODUCT_8BITDO_ULTIMATE3, 64, |d| {
            d[3] = 0x01;
            d[4] = 0x00;
            d[11] = 0;
        }),
        ("ultimate3 0", USB_PRODUCT_8BITDO_ULTIMATE3, 0, |_| {}),
    ];
    for (name, product_id, size, setup) in features {
        setup(&mut data);
        let mut report = data;
        let mut ctx = EightBitDoContext::default();
        let mut lines = Vec::new();
        // (the feature report starts with its report ID)
        let serial = if product_id == USB_PRODUCT_8BITDO_ULTIMATE3 {
            ctx.sensors_supported = true;
            ctx.rumble_supported = true;
            ctx.powerstate_supported = true;
            ctx.sensor_timestamp_supported = true;
            report[0] = FEATURE_REPORTID;
            (size > 0)
                .then(|| ctx.apply_ultimate3_features(&report, size))
                .flatten()
        } else {
            report[0] = FEATURE_REPORTID_ENABLE_SDL_REPORTID;
            (size > 0)
                .then(|| ctx.apply_features(&report, size))
                .flatten()
        };
        if let Some(serial) = serial {
            lines.push(format!("serial {serial}"));
        }
        if let Some(name) = device_name(product_id) {
            lines.push(format!("name {name}"));
        }
        lines.push("added".to_owned());
        lines.push(format!(
            "flags sensors {} rumble {} trigger {} power {} timestamp {}",
            u8::from(ctx.sensors_supported),
            u8::from(ctx.rumble_supported),
            u8::from(ctx.trigger_rumble_supported),
            u8::from(ctx.powerstate_supported),
            u8::from(ctx.sensor_timestamp_supported)
        ));
        assert_eq!(lines, call(name), "{name}");
    }
}

#[test]
fn imu_rates() {
    let mut ctx = EightBitDoContext::default();
    assert_eq!(ctx.imu_rate(USB_PRODUCT_8BITDO_SN30_PRO, true), 70);
    assert_eq!(ctx.imu_rate(USB_PRODUCT_8BITDO_SN30_PRO, false), 100);
    assert_eq!(ctx.imu_rate(USB_PRODUCT_8BITDO_PRO_3, true), 85);
    assert_eq!(
        ctx.imu_rate(USB_PRODUCT_8BITDO_ULTIMATE2_WIRELESS, false),
        1000
    );
    ctx.sensor_timestamp_supported = true;
    assert_eq!(ctx.imu_rate(USB_PRODUCT_8BITDO_PRO_2_BT, false), 200);
    assert_eq!(ctx.imu_rate(USB_PRODUCT_8BITDO_ULTIMATE3, false), 120);
}
