// Tests for the PS3 HIDAPI drivers.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The cases in `data.rs` come from running upstream's SDL_hidapi_ps3.c,
//! with stubs for the SDL functions it uses, on generated reports. A case
//! is named after its driver and context ("analog" buttons, reported
//! "sensors"); the effects upstream sends through the rumble thread are
//! compared with the reports the drivers build for them.

use super::super::tests::{run, test_device};
use super::*;
use crate::hidapi::DeviceInfo;

mod data;

fn device(vendor_id: u16, product_id: u16) -> std::sync::Arc<HidapiDevice> {
    test_device(&DeviceInfo {
        vendor_id,
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

/// The context of a case.
fn context(name: &str) -> Ps3Context {
    Ps3Context {
        has_analog_buttons: name.contains("analog"),
        report_sensors: name.contains("sensors"),
        player_index: match name.split(' ').next() {
            Some("ps3") if name.contains("sensors") => 2,
            Some("ps3") if name.contains("mini") => 5,
            Some("ps3") => -1,
            Some("sixaxis") => 1,
            _ => 0,
        },
        ..Ps3Context::default()
    }
}

#[test]
fn supported_devices() {
    let supported = |driver: &dyn DriverImpl, gamepad_type, vid, pid| {
        driver.is_supported_device(None, "", gamepad_type, vid, pid, 0, 0, 0, 0, 0)
    };
    let standard = GamepadType::Standard;
    assert!(supported(
        &Ps3Driver,
        standard,
        USB_VENDOR_SONY,
        USB_PRODUCT_SONY_DS3
    ));
    assert!(supported(
        &Ps3Driver,
        standard,
        USB_VENDOR_SHANWAN,
        USB_PRODUCT_SHANWAN_DS3
    ));
    assert!(!supported(
        &Ps3Driver,
        standard,
        USB_VENDOR_SONY,
        USB_PRODUCT_SONY_DS4
    ));
    assert!(supported(
        &Ps3SonySixaxisDriver,
        standard,
        USB_VENDOR_SONY,
        USB_PRODUCT_SONY_DS3
    ));
    assert!(!supported(
        &Ps3SonySixaxisDriver,
        standard,
        USB_VENDOR_SHANWAN,
        USB_PRODUCT_SHANWAN_DS3
    ));

    // Third party controllers might be supported until they're opened
    let third_party = &Ps3ThirdPartyDriver;
    assert!(supported(
        third_party,
        standard,
        USB_VENDOR_LOGITECH,
        USB_PRODUCT_LOGITECH_CHILLSTREAM
    ));
    assert!(supported(third_party, standard, USB_VENDOR_HORI, 0x0001));
    assert!(supported(third_party, GamepadType::Ps3, 0xabcd, 0x0001));
    assert!(!supported(
        third_party,
        GamepadType::Ps3,
        USB_VENDOR_SONY,
        0x0001
    ));
    assert!(!supported(third_party, standard, 0xabcd, 0x0001));
    // ... and not when they can't be asked
    let device = device(USB_VENDOR_HORI, 0x0001);
    assert!(third_party.is_supported_device(
        Some(&device),
        "",
        standard,
        USB_VENDOR_HORI,
        0x0001,
        0,
        0,
        0,
        0,
        0
    ));
}

#[test]
fn device_kinds() {
    assert!(is_shanwan(USB_VENDOR_SONY, "SHANWAN PS3 GamePad"));
    assert!(is_shanwan(USB_VENDOR_SHANWAN_ALT, "PS3 Controller"));
    assert!(!is_shanwan(USB_VENDOR_SONY, "ShanWa"));
    assert!(!is_shanwan(
        USB_VENDOR_SONY,
        "Sony PLAYSTATION(R)3 Controller"
    ));
    assert_eq!(
        third_party_joystick_type(USB_VENDOR_HARMONIX, USB_PRODUCT_HARMONIX_WII_RB2_DRUMS),
        Some(JoystickType::DrumKit)
    );
    assert_eq!(
        third_party_joystick_type(USB_VENDOR_SCEA, USB_PRODUCT_SCEA_PS3WIIU_GHLIVE),
        Some(JoystickType::Guitar)
    );
    assert_eq!(
        third_party_joystick_type(USB_VENDOR_SCEA, USB_PRODUCT_HARMONIX_WII_RB2_DRUMS),
        None
    );
}

#[test]
fn state_reports() {
    for (name, reports) in data::CASES {
        let kind = name.split(' ').next().unwrap();
        let device = match kind {
            "thirdparty" if name.contains("digital") => {
                device(USB_VENDOR_SWITCH, USB_PRODUCT_SWITCH_RETRO_CONTROLLER)
            }
            "thirdparty" => device(USB_VENDOR_HORI, 0x0001),
            "cyborg" => device(USB_VENDOR_SAITEK, USB_PRODUCT_SAITEK_CYBORG_V3),
            _ => device(USB_VENDOR_SONY, USB_PRODUCT_SONY_DS3),
        };
        let mut official = Ps3OfficialContext(context(name));
        let mut third_party = Ps3ThirdPartyContext(context(name));
        let mut sixaxis = Ps3SonySixaxisContext(context(name));
        for (n, (report, expected)) in reports.iter().enumerate() {
            let mut data = [0u8; USB_PACKET_LENGTH];
            data[..report.len()].copy_from_slice(report);
            let size = report.len();
            let before = crate::timer::ticks_ns();
            let (effects, events) = run(&device, |d| match kind {
                "ps3" => official
                    .handle_report(d, 1, &data, size)
                    .then(|| effect_report(&official.0.effects())),
                "sixaxis" => sixaxis
                    .handle_report(d, 1, &data, size)
                    .then(|| sixaxis_effect_report(&sixaxis.0.sixaxis_led_effects())),
                _ => {
                    third_party.handle_report(d, 1, &data, size);
                    None
                }
            });
            let after = crate::timer::ticks_ns();
            // (a sensor timestamp of the time the report came in is NOW)
            let mut events: Vec<String> = events
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
            if let Some(effects) = effects {
                events.push(hex_line("rumble", &effects));
            }
            assert_eq!(events, *expected, "{name} report {n}");
        }
    }
}

#[test]
fn effects() {
    let mut ctx = Ps3Context {
        has_analog_buttons: true,
        ..Ps3Context::default()
    };
    for player_index in [0, 1, 2, 3, 4, 7, -1] {
        ctx.player_index = player_index;
        let name = format!("player {player_index}");
        assert_eq!(
            [hex_line("rumble", &effect_report(&ctx.effects()))],
            call(&name),
            "{name}"
        );
    }
    let rumbles = [(0x1234u16, 0x0000u16), (0x0000, 0x00ff), (0xffff, 0x0100)];
    for (low, high) in rumbles {
        ctx.rumble_left = (low >> 8) as u8;
        ctx.rumble_right = (high >> 8) as u8;
        let name = format!("rumble {low:04x} {high:04x}");
        assert_eq!(
            [hex_line("rumble", &effect_report(&ctx.effects()))],
            call(&name),
            "{name}"
        );
    }

    let effect: Vec<u8> = (0..64).map(|i| 0x80 + i as u8).collect();
    for len in [0, 1, 10, 48, 49, 60] {
        let name = format!("effect {len}");
        assert_eq!(
            [hex_line("rumble", &effect_report(&effect[..len]))],
            call(&name),
            "{name}"
        );
        let name = format!("sixaxis effect {len}");
        assert_eq!(
            [hex_line("rumble", &sixaxis_effect_report(&effect[..len]))],
            call(&name),
            "{name}"
        );
    }

    let mut ctx = Ps3Context {
        has_analog_buttons: true,
        ..Ps3Context::default()
    };
    for player_index in [0, 1, 2, 3, 4, 7] {
        ctx.player_index = player_index;
        let name = format!("sixaxis player {player_index}");
        assert_eq!(
            [hex_line(
                "rumble",
                &sixaxis_effect_report(&ctx.sixaxis_led_effects())
            )],
            call(&name),
            "{name}"
        );
    }
    for (low, high) in rumbles {
        ctx.rumble_left = (low >> 8) as u8;
        ctx.rumble_right = (high >> 8) as u8;
        let name = format!("sixaxis rumble {low:04x} {high:04x}");
        assert_eq!(
            [hex_line(
                "rumble",
                &sixaxis_effect_report(&ctx.sixaxis_rumble_effects())
            )],
            call(&name),
            "{name}"
        );
    }

    // (out of bounds upstream; see sixaxis_led_effects() and effects())
    ctx.player_index = -1;
    assert_eq!(ctx.sixaxis_led_effects()[5..], [0, 0, 0, 0]);
    ctx.player_index = -2;
    assert_eq!(ctx.effects()[9], 0);
}

#[test]
fn accelerometer() {
    // DualShock 3 values are big endian, with 511 at rest
    assert_eq!(scale_accel(load16(0x01, 0xff)), 0.0);
    assert_eq!(scale_accel(load16(0x02, 0x70)), STANDARD_GRAVITY);
    assert_eq!(third_party_scale_accel(load16(0x00, 0x02)), 0.0);
    assert_eq!(
        third_party_scale_accel(load16(0x71, 0x02)),
        STANDARD_GRAVITY
    );
}
