// Tests for the PS4 HIDAPI driver.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The cases in `data.rs` come from running upstream's SDL_hidapi_ps4.c,
//! with stubs for the SDL functions it uses, on generated reports. Its
//! rumble packets and disconnections aren't part of the expectations (the
//! tests have no device to write to), and the bytes past a report read as
//! 0 in both.

use super::super::tests::{run, test_device};
use super::*;
use crate::hidapi::{BusType, DeviceInfo};
use std::sync::Arc;

mod data;

pub(super) struct StateCase {
    name: &'static str,
    bluetooth: bool,
    official: bool,
    nacon_dongle: bool,
    touchpad: bool,
    battery: bool,
    sensors: bool,
    enhanced_reports: bool,
    whammy: bool,
    tilt: bool,
    selector: bool,
    reports: &'static [(&'static [u8], &'static [&'static str])],
}

pub(super) struct EffectCase {
    bluetooth: bool,
    official: bool,
    report_interval: u8,
    mask: u8,
    effect: &'static [u8],
    packet: &'static [u8],
}

pub(super) struct CalibrationCase {
    bluetooth: bool,
    dongle: bool,
    strikepad: bool,
    gyro: (u16, u16),
    accel: (u16, u16),
    report: &'static [u8],
    calibration: [(i16, u32); 6],
    hardware_calibration: bool,
}

fn ds4(bluetooth: bool) -> Arc<HidapiDevice> {
    test_device(&DeviceInfo {
        vendor_id: USB_VENDOR_SONY,
        product_id: USB_PRODUCT_SONY_DS4_SLIM,
        interface_number: 0,
        bus_type: if bluetooth {
            BusType::Bluetooth
        } else {
            BusType::Usb
        },
        ..DeviceInfo::default()
    })
}

/// The context of the harness.
fn context() -> Ps4Context {
    Ps4Context {
        joystick: Some(1),
        gyro_numerator: 1,
        gyro_denominator: 16,
        accel_numerator: 1,
        accel_denominator: 8192,
        calibration: std::array::from_fn(|i| ImuCalibration {
            bias: (i as i16) * 3 - 7,
            scale: (i + 1) as f32 * 0.25,
        }),
        ..Ps4Context::default()
    }
}

#[test]
fn state_reports() {
    for case in data::STATE_CASES {
        let device = ds4(case.bluetooth);
        let mut ctx = Ps4Context {
            official_controller: case.official,
            is_nacon_dongle: case.nacon_dongle,
            report_touchpad: case.touchpad,
            report_battery: case.battery,
            report_sensors: case.sensors,
            enhanced_reports: case.enhanced_reports,
            guitar_whammy_supported: case.whammy,
            guitar_tilt_supported: case.tilt,
            guitar_effects_selector_supported: case.selector,
            ..context()
        };
        for (n, (report, expected)) in case.reports.iter().enumerate() {
            let mut data = [0u8; USB_PACKET_LENGTH * 2];
            data[..report.len()].copy_from_slice(report);
            let before = crate::timer::ticks_ns();
            let (_, events) = run(&device, |d| {
                ctx.handle_report(d, Some(1), &data, report.len())
            });
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
            assert_eq!(events, *expected, "{} report {n}", case.name);
        }
    }
}

#[test]
fn effect_packets() {
    for (n, case) in data::EFFECT_CASES.iter().enumerate() {
        let ctx = Ps4Context {
            official_controller: case.official,
            report_interval: case.report_interval,
            ..context()
        };
        let (packet, size) = ctx.effect_packet(case.bluetooth, case.effect, case.mask);
        assert_eq!(&packet[..size], case.packet, "effect {n}");
    }
}

#[test]
fn calibration() {
    for (n, case) in data::CALIBRATION_CASES.iter().enumerate() {
        let mut ctx = Ps4Context {
            official_controller: true,
            is_dongle: case.dongle,
            gyro_numerator: case.gyro.0,
            gyro_denominator: case.gyro.1,
            accel_numerator: case.accel.0,
            accel_denominator: case.accel.1,
            ..context()
        };
        let mut report = [0u8; USB_PACKET_LENGTH];
        report[..case.report.len()].copy_from_slice(case.report);
        ctx.apply_calibration_report(
            &CalibrationReport::Data(report),
            case.bluetooth,
            case.strikepad,
        );
        let calibration: Vec<(i16, u32)> = ctx
            .calibration
            .iter()
            .map(|c| (c.bias, c.scale.to_bits()))
            .collect();
        assert_eq!(calibration, case.calibration, "calibration {n}");
        assert_eq!(
            ctx.hardware_calibration, case.hardware_calibration,
            "calibration {n}"
        );
    }

    // Without a report, the calibration is the default one
    let mut ctx = context();
    ctx.apply_calibration_report(&CalibrationReport::Unavailable, false, false);
    assert_eq!(ctx.calibration[0].bias, 0);
    assert_eq!(
        ctx.calibration[0].scale,
        ((1.0f64 / 16.0) * std::f64::consts::PI / 180.0) as f32
    );
    assert_eq!(
        ctx.calibration[3].scale,
        ((1.0f64 / 8192.0) * f64::from(STANDARD_GRAVITY)) as f32
    );
    assert_eq!(
        apply_calibration(
            &ImuCalibration {
                bias: 4,
                scale: 0.5
            },
            10
        ),
        3.0
    );
}

#[test]
fn crcs() {
    let mut data = [0x5au8; 78];
    data[0] = 0x11;
    // An input report's CRC has another header byte than an output report's
    set_output_crc(&mut data);
    assert!(!verify_crc(&data));
    let crc = crate::stdlib::crc32(crate::stdlib::crc32(0, &[0xA1]), &data[..74]);
    data[74..].copy_from_slice(&crc.to_le_bytes());
    assert!(verify_crc(&data));
    data[10] ^= 1;
    assert!(!verify_crc(&data));
}

#[test]
fn bluetooth_crc_counting() {
    let mut ctx = Ps4Context {
        official_controller: true,
        ..context()
    };
    let mut good = [0u8; USB_PACKET_LENGTH * 2];
    good[0] = 0x11;
    good[1] = 0xC0;
    let crc = crate::stdlib::crc32(crate::stdlib::crc32(0, &[0xA1]), &good[..74]);
    good[74..78].copy_from_slice(&crc.to_le_bytes());
    let mut bad = good;
    bad[74] ^= 0xFF;

    // A bad CRC is fine unless there are still three good ones after it
    assert!(ctx.is_packet_valid(&bad, 78));
    for _ in 0..3 {
        assert!(ctx.is_packet_valid(&good, 78));
    }
    assert_eq!(ctx.valid_crc_packets, 3);
    assert!(ctx.is_packet_valid(&bad, 78));
    assert_eq!(ctx.valid_crc_packets, 2);
    assert!(ctx.is_packet_valid(&good, 78));
    assert!(ctx.is_packet_valid(&good, 78));
    assert!(!ctx.is_packet_valid(&bad, 78));
    assert_eq!(ctx.valid_crc_packets, 3);
    // Reports without HID data aren't valid
    let mut no_data = good;
    no_data[1] = 0x40;
    assert!(!ctx.is_packet_valid(&no_data, 78));
}

#[test]
fn serials_and_leds() {
    assert_eq!(dashed_serial(Some("a0b1c2d3e4f5")), "a0-b1-c2-d3-e4-f5");
    assert_eq!(dashed_serial(Some("a0b1c2")), "");
    assert_eq!(dashed_serial(None), "");

    let mut report = [0u8; USB_PACKET_LENGTH];
    report[..7].copy_from_slice(&[0x12, 0x01, 0x02, 0x03, 0x04, 0x05, 0xa6]);
    assert_eq!(
        serial_from_report(Ok(16), &report).as_deref(),
        Some("a6-05-04-03-02-01")
    );
    assert_eq!(serial_from_report(Ok(6), &report), None);
    assert_eq!(serial_from_report(Err(Error::new("")), &report), None);
    report[1..7].fill(0);
    assert_eq!(serial_from_report(Ok(16), &report), None);

    let mut effects = Ds4EffectsState::default();
    set_leds_for_player_index(&mut effects, 1);
    assert_eq!(effects.to_bytes()[2..5], [0x40, 0x00, 0x00]);
    set_leds_for_player_index(&mut effects, 8);
    assert_eq!(effects.to_bytes()[2..5], [0x40, 0x00, 0x00]);
    set_leds_for_player_index(&mut effects, -1);
    assert_eq!(effects.to_bytes()[2..5], [0x00, 0x00, 0x40]);

    assert_eq!(hat_of(0), HAT_UP);
    assert_eq!(hat_of(5), HAT_LEFTDOWN);
    assert_eq!(hat_of(8), HAT_CENTERED);
    assert_eq!(joystick_type_of(0x07), JoystickType::ArcadeStick);
    assert_eq!(joystick_type_of(0x03), JoystickType::Unknown);
}

#[test]
fn third_party_capabilities() {
    let mut data = [0u8; USB_PACKET_LENGTH];
    data[2] = 0x27;
    data[4] = 0x02 | 0x08;
    data[5] = 0x01;
    data[24] = 0x04;
    data[10..18].copy_from_slice(&[2, 0, 32, 0, 0, 0, 0, 0x20]);
    assert!(is_third_party_capabilities(Ok(48), &data));
    assert!(!is_third_party_capabilities(Ok(47), &data));

    let mut ctx = context();
    assert_eq!(ctx.set_capabilities(&data), JoystickType::Guitar);
    assert!(ctx.sensors_supported && ctx.vibration_supported && ctx.guitar_whammy_supported);
    assert!(!ctx.lightbar_supported && !ctx.touchpad_supported && !ctx.guitar_tilt_supported);
    assert_eq!((ctx.gyro_numerator, ctx.gyro_denominator), (2, 32));
    // A zero numerator keeps the default
    assert_eq!((ctx.accel_numerator, ctx.accel_denominator), (1, 8192));
}

#[test]
fn enhanced_mode() {
    let _l = crate::test_support::test_lock();
    let _lock = crate::joystick::lock_joysticks();

    let device = ds4(true);
    let mut ctx = Ps4Context {
        official_controller: true,
        touchpad_supported: true,
        sensors_supported: true,
        lightbar_supported: true,
        effects_supported: true,
        ..context()
    };
    let mut joystick = JoystickData::new(1);

    // The report interval comes first; "auto" only advertises the features
    let (_, events) = run(&device, |d| {
        let mut js = JoystickRef::Opening(&mut joystick);
        ctx.report_interval_hint_changed(d, &mut js, Some("2"));
        ctx.enhanced_reports_changed(d, &mut js, Some("auto"));
    });
    assert_eq!(events, ["properties"]);
    assert_eq!(ctx.report_interval, 2);
    assert!(ctx.enhanced_mode_available && !ctx.enhanced_mode);
    assert_eq!(joystick.touchpads.len(), 1);
    assert_eq!(joystick.sensors.len(), 2);
    assert_eq!(joystick.sensors[0].rate, 500.0);
    assert!(ctx.report_touchpad && ctx.report_battery);

    // An application request switches to enhanced mode
    run(&device, |d| {
        let mut js = JoystickRef::Opening(&mut joystick);
        ctx.update_enhanced_mode_on_application_usage(d, &mut js);
    });
    assert!(ctx.enhanced_mode);
    assert_eq!(ctx.enhanced_report_hint, EnhancedReportHint::On);
    assert_eq!(joystick.touchpads.len(), 1);

    // Invalid intervals are the default
    run(&device, |d| {
        let mut js = JoystickRef::Opening(&mut joystick);
        ctx.report_interval_hint_changed(d, &mut js, Some("3"));
    });
    assert_eq!(ctx.report_interval, 4);
    assert_eq!(joystick.sensors[1].rate, 250.0);

    // Over USB, enhanced reports are always on
    let usb = ds4(false);
    let mut ctx = Ps4Context {
        official_controller: true,
        ..context()
    };
    run(&usb, |d| {
        ctx.enhanced_reports_changed(d, &mut JoystickRef::Opening(&mut joystick), Some("0"));
    });
    assert_eq!(ctx.enhanced_report_hint, EnhancedReportHint::On);
    assert!(ctx.enhanced_mode);
}

#[test]
fn supported_devices() {
    let driver = Ps4Driver;
    let supported = |gamepad_type, vendor_id, product_id| {
        driver.is_supported_device(None, "", gamepad_type, vendor_id, product_id, 0, 0, 0, 0, 0)
    };
    assert!(supported(
        GamepadType::Ps4,
        USB_VENDOR_SONY,
        USB_PRODUCT_SONY_DS4
    ));
    // Devices that may be third party PS4 controllers are probed when opened
    assert!(supported(GamepadType::Standard, USB_VENDOR_HORI, 0x0001));
    assert!(!supported(
        GamepadType::Standard,
        USB_VENDOR_LOGITECH,
        0x0001
    ));
}
