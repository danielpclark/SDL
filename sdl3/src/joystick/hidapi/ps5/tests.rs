// Tests for the PS5 HIDAPI driver.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The cases in `data.rs` come from running upstream's SDL_hidapi_ps5.c,
//! with stubs for the SDL functions it uses, on generated reports. Its
//! rumble packets and disconnections aren't part of the state
//! expectations (the tests have no device to write to), and the bytes
//! past a report read as 0 in both.

use super::super::tests::{run, test_device};
use super::*;
use crate::hidapi::{BusType, DeviceInfo};
use std::sync::Arc;

mod data;

pub(super) struct StateCase {
    name: &'static str,
    bluetooth: bool,
    alternate: bool,
    dongle: bool,
    touchpad: bool,
    battery: bool,
    sensors: bool,
    hardware_calibration: bool,
    whammy: bool,
    tilt: bool,
    selector: bool,
    reports: &'static [(&'static [u8], &'static [&'static str])],
}

pub(super) struct EffectCase {
    bluetooth: bool,
    mask: u8,
    vibration: bool,
    enhanced_rumble: bool,
    rumble: (u8, u8),
    lightbar: bool,
    color: Option<(u8, u8, u8)>,
    playerled: bool,
    player_lights: bool,
    player_index: i32,
    enhanced_reports: bool,
    led_reset_state: LedResetState,
    packet: Option<&'static [u8]>,
    led_reset_state_after: LedResetState,
}

pub(super) struct CalibrationCase {
    report: &'static [u8],
    calibration: [(i16, u32); 6],
    hardware_calibration: bool,
}

fn ds5(bluetooth: bool) -> Arc<HidapiDevice> {
    test_device(&DeviceInfo {
        vendor_id: USB_VENDOR_SONY,
        product_id: USB_PRODUCT_SONY_DS5,
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
fn context() -> Ps5Context {
    Ps5Context {
        joystick: Some(1),
        calibration: std::array::from_fn(|i| ImuCalibration {
            bias: (i as i16) * 3 - 7,
            sensitivity: (i + 1) as f32 * 0.25,
        }),
        ..Ps5Context::default()
    }
}

/// The events with the sensor timestamps of the time of the report as
/// NOW.
fn with_now(events: Vec<String>, before: u64, after: u64) -> Vec<String> {
    events
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
        .collect()
}

#[test]
fn state_reports() {
    for case in data::STATE_CASES {
        let device = ds5(case.bluetooth);
        let mut ctx = Ps5Context {
            use_alternate_report: case.alternate,
            is_dongle: case.dongle,
            report_touchpad: case.touchpad,
            report_battery: case.battery,
            report_sensors: case.sensors,
            hardware_calibration: case.hardware_calibration,
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
            let events = with_now(events, before, crate::timer::ticks_ns());
            assert_eq!(events, *expected, "{} report {n}", case.name);
        }
    }
}

#[test]
fn effect_packets() {
    for (n, case) in data::EFFECT_CASES.iter().enumerate() {
        let mut ctx = Ps5Context {
            vibration_supported: case.vibration,
            enhanced_rumble: case.enhanced_rumble,
            rumble_left: case.rumble.0,
            rumble_right: case.rumble.1,
            lightbar_supported: case.lightbar,
            color_set: case.color.is_some(),
            led_red: case.color.map_or(0, |c| c.0),
            led_green: case.color.map_or(0, |c| c.1),
            led_blue: case.color.map_or(0, |c| c.2),
            playerled_supported: case.playerled,
            player_lights: case.player_lights,
            player_index: case.player_index,
            enhanced_reports: case.enhanced_reports,
            led_reset_state: case.led_reset_state,
            ..context()
        };
        let packet = ctx.effects(case.bluetooth, case.mask).map(|effects| {
            let (data, size, _) = Ps5Context::effect_packet(case.bluetooth, &effects.to_bytes());
            data[..size].to_vec()
        });
        assert_eq!(packet.as_deref(), case.packet, "effect {n}");
        assert_eq!(
            ctx.led_reset_state, case.led_reset_state_after,
            "effect {n}"
        );
    }
}

#[test]
fn calibration() {
    for (n, case) in data::CALIBRATION_CASES.iter().enumerate() {
        let mut ctx = context();
        let mut report = [0u8; USB_PACKET_LENGTH];
        report[..case.report.len()].copy_from_slice(case.report);
        ctx.apply_calibration_report(&report);
        let calibration: Vec<(i16, u32)> = ctx
            .calibration
            .iter()
            .map(|c| (c.bias, c.sensitivity.to_bits()))
            .collect();
        assert_eq!(calibration, case.calibration, "calibration {n}");
        assert_eq!(
            ctx.hardware_calibration, case.hardware_calibration,
            "calibration {n}"
        );
    }

    // Without hardware calibration, the raw values are scaled
    let ctx = Ps5Context::default();
    assert_eq!(
        ctx.apply_calibration_data(0, 1024),
        (64.0 * std::f32::consts::PI) / 180.0
    );
    assert_eq!(ctx.apply_calibration_data(3, 8192), STANDARD_GRAVITY);
}

#[test]
fn helpers() {
    assert_eq!(battery_state(0x05), (PowerState::OnBattery, 55));
    assert_eq!(battery_state(0x1A), (PowerState::Charging, 100));
    assert_eq!(battery_state(0x23), (PowerState::Charged, 100));
    assert_eq!(battery_state(0xF3), (PowerState::Unknown, 0));
    assert_eq!(trigger_axis(0, true), i16::MAX);
    assert_eq!(trigger_axis(0, false), i16::MIN);
    assert_eq!(trigger_axis(255, false), i16::MAX);

    let mut effects = Ds5EffectsState::default();
    set_lights_for_player_index(&mut effects, 8);
    assert_eq!(effects.pad_lights, 0x0A | 0x20);
    set_lights_for_player_index(&mut effects, -1);
    assert_eq!(effects.pad_lights, 0);
    set_leds_for_player_index(&mut effects, 4);
    assert_eq!(effects.to_bytes()[44..47], [0x20, 0x10, 0x00]);
}

#[test]
fn bluetooth_crcs() {
    let mut ctx = context();
    let mut data = [0u8; USB_PACKET_LENGTH * 2];
    data[0] = REPORT_ID_BLUETOOTH_STATE;
    // Too short to have a CRC
    assert!(!ctx.is_packet_valid(&data, 3));
    let crc = crate::stdlib::crc32(crate::stdlib::crc32(0, &[0xA1]), &data[..74]);
    data[74..78].copy_from_slice(&crc.to_le_bytes());
    assert!(ctx.is_packet_valid(&data, 78));
    assert!(!ctx.is_packet_valid(&data, 77));
}

#[test]
fn led_reset() {
    let device = ds5(true);
    let mut ctx = Ps5Context {
        enhanced_reports: true,
        sensors_supported: true,
        ..context()
    };
    // The connection animation isn't over
    ctx.last_state[common::SENSOR_TIMESTAMP..common::SENSOR_TIMESTAMP + 4]
        .copy_from_slice(&10199999u32.to_le_bytes());
    ctx.led_reset_state = LedResetState::Pending;
    run(&device, |d| {
        ctx.check_pending_led_reset(d, &mut JoystickRef::Open(1))
    });
    assert_eq!(ctx.led_reset_state, LedResetState::Pending);
    ctx.last_state[common::SENSOR_TIMESTAMP..common::SENSOR_TIMESTAMP + 4]
        .copy_from_slice(&10200000u32.to_le_bytes());
    run(&device, |d| {
        ctx.check_pending_led_reset(d, &mut JoystickRef::Open(1))
    });
    assert_eq!(ctx.led_reset_state, LedResetState::Complete);
}

#[test]
fn enhanced_mode() {
    let _l = crate::test_support::test_lock();
    let _lock = crate::joystick::lock_joysticks();

    let device = ds5(false);
    let mut ctx = Ps5Context {
        touchpad_supported: true,
        sensors_supported: true,
        ..context()
    };
    let mut joystick = JoystickData::new(1);
    let (_, events) = run(&device, |d| {
        ctx.enhanced_reports_changed(d, &mut JoystickRef::Opening(&mut joystick), Some("0"))
    });
    // Over USB, enhanced reports are always on
    assert_eq!(events, ["properties"]);
    assert!(ctx.enhanced_mode && ctx.report_touchpad && ctx.report_battery);
    assert_eq!(joystick.touchpads.len(), 1);
    assert_eq!(joystick.sensors.len(), 2);
    assert_eq!(joystick.sensors[0].rate, 250.0);
}

#[test]
fn supported_devices() {
    let driver = Ps5Driver;
    let supported = |gamepad_type, vendor_id, product_id| {
        driver.is_supported_device(None, "", gamepad_type, vendor_id, product_id, 0, 0, 0, 0, 0)
    };
    assert!(supported(
        GamepadType::Ps5,
        USB_VENDOR_SONY,
        USB_PRODUCT_SONY_DS5
    ));
    assert!(!supported(
        GamepadType::Ps5,
        USB_VENDOR_BACKBONE,
        USB_PRODUCT_BACKBONE_ONE_PS5_V2
    ));
    assert!(supported(GamepadType::Standard, USB_VENDOR_HORI, 0x0001));
    assert!(!supported(
        GamepadType::Standard,
        USB_VENDOR_LOGITECH,
        0x0001
    ));
}
