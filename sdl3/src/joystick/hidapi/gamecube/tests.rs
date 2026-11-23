// Tests for the GameCube HIDAPI driver.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see
// LICENSE.txt.

//! The cases in `data.rs` come from running upstream's
//! SDL_hidapi_gamecube.c, with stubs for the SDL functions it uses, on
//! generated reports: an adapter in Wii U mode, then in PC mode, with the
//! slot handling of Windows ("windows") and of the other platforms
//! ("unix"). Each case's rumble calls follow it in the list of calls.

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

/// The open joysticks of the harness: all connected ones.
fn is_open(joystick: JoystickID) -> bool {
    joystick != 0
}

fn feed_case(ctx: &mut GameCubeContext, device: &std::sync::Arc<HidapiDevice>, name: &str) {
    let (_, reports) = data::CASES.iter().find(|(n, _)| *n == name).unwrap();
    for (n, (report, expected)) in reports.iter().enumerate() {
        let mut data = [0u8; USB_PACKET_LENGTH];
        data[..report.len()].copy_from_slice(report);
        let ((), events) = run(device, |d| {
            ctx.handle_report(d, &data, report.len(), is_open)
        });
        assert_eq!(events, *expected, "{name} report {n}");
    }
}

/// A rumble call of the harness: "rumble <slot> <low> <high> [brake]".
fn rumble(ctx: &mut GameCubeContext, name: &str) -> Vec<String> {
    let words: Vec<&str> = name.split(' ').collect();
    let mut lines = Vec::new();
    let joystick = match words[1] {
        "none" => 12345,
        slot => ctx.joysticks[slot.parse::<usize>().unwrap()],
    };
    let (low, high) = match words.get(2) {
        Some(low) => (
            u16::from_str_radix(low, 16).unwrap(),
            u16::from_str_radix(words[3], 16).unwrap(),
        ),
        None => (1, 1),
    };
    ctx.use_rumble_brake = words.get(4) == Some(&"brake");
    match ctx.update_rumble(joystick, low, high) {
        Ok(true) => lines.push(hex_line("rumble", &ctx.pc_rumble_packet())),
        Ok(false) => {}
        Err(e) => lines.push(format!("error {}", e.message())),
    }
    if words[1] != "none" {
        lines.push(hex_line("state", &ctx.rumble));
        lines.push(format!(
            "update {} active {}",
            u8::from(ctx.rumble_update),
            u8::from(ctx.rumble_active)
        ));
    }
    lines
}

#[test]
fn supported_devices() {
    let driver = GameCubeDriver;
    let supported = |vid, pid| {
        driver.is_supported_device(None, "", GamepadType::Standard, vid, pid, 0, 0, 0, 0, 0)
    };
    assert!(supported(
        USB_VENDOR_NINTENDO,
        USB_PRODUCT_NINTENDO_GAMECUBE_ADAPTER
    ));
    assert!(supported(
        USB_VENDOR_DRAGONRISE,
        USB_PRODUCT_EVORETRO_GAMECUBE_ADAPTER2
    ));
    assert!(!supported(USB_VENDOR_DRAGONRISE, 0x0001));
}

#[test]
fn adapter_reports_and_rumble() {
    let _l = crate::test_support::test_lock();
    let _lock = crate::joystick::lock_joysticks();

    // The Wii U adapter
    let wii_u = device(USB_VENDOR_NINTENDO, USB_PRODUCT_NINTENDO_GAMECUBE_ADAPTER);
    let mut ctx = GameCubeContext::default();
    ctx.rumble[0] = 0x11;
    feed_case(&mut ctx, &wii_u, "nintendo");
    ctx.rumble_allowed[2] = false;
    let mut calls = data::CALLS.iter();
    for (name, expected) in calls.by_ref() {
        if name.starts_with("init") {
            break;
        }
        assert_eq!(rumble(&mut ctx, name), *expected, "{name}");
    }
    let (_, joysticks) = run(&wii_u, |d| {
        for joystick in ctx.joysticks.into_iter().filter(|&j| j != 0) {
            d.joystick_disconnected(joystick);
        }
        for i in 0..MAX_CONTROLLERS {
            assert_eq!(
                ctx.get_device_player_index(d, ctx.joysticks[i]),
                if ctx.joysticks[i] != 0 { i as i32 } else { -1 }
            );
        }
    });
    assert!(joysticks.iter().all(|e| e == "removed"));

    // An EVORETRO adapter in PC mode, on each platform
    let device = device(
        USB_VENDOR_DRAGONRISE,
        USB_PRODUCT_EVORETRO_GAMECUBE_ADAPTER1,
    );
    let platform = if cfg!(windows) { "windows" } else { "unix" };
    let mut checked = false;
    let mut calls = data::CALLS
        .iter()
        .skip_while(|(name, _)| !name.starts_with("init"))
        .peekable();
    while let Some((name, expected)) = calls.next() {
        let this_platform = name.ends_with(platform);
        let mut ctx = GameCubeContext {
            pc_mode: true,
            ..GameCubeContext::default()
        };
        ctx.rumble[0] = 0x11;
        let (_, events) = run(&device, |d| ctx.connect_pc_slots(d, true));
        let expected: Vec<&str> = expected
            .iter()
            .copied()
            .filter(|e| !e.starts_with("name "))
            .collect();
        if this_platform {
            assert_eq!(events, expected, "{name}");
            feed_case(&mut ctx, &device, &format!("pc {platform}"));
        }
        while let Some((name, expected)) = calls.next_if(|(n, _)| !n.starts_with("init")) {
            if this_platform {
                if *name == "rumble 0 0100 0000" && expected[0].starts_with("error") {
                    ctx.rumble_allowed[0] = false;
                }
                assert_eq!(rumble(&mut ctx, name), *expected, "{name}");
                checked = true;
            }
        }
        let (_, _) = run(&device, |d| {
            for joystick in ctx.joysticks.into_iter().filter(|&j| j != 0) {
                d.joystick_disconnected(joystick);
            }
        });
    }
    assert!(checked);

    super::super::NUMJOYSTICKS.store(0, std::sync::atomic::Ordering::Relaxed);
    super::super::CHANGE_COUNT.store(0, std::sync::atomic::Ordering::Relaxed);
}

#[test]
fn capabilities() {
    let device = device(USB_VENDOR_NINTENDO, USB_PRODUCT_NINTENDO_GAMECUBE_ADAPTER);
    let mut ctx = GameCubeContext {
        joysticks: [1, 2, 3, 0],
        wireless: [false, true, false, false],
        rumble_allowed: [true, true, false, false],
        ..GameCubeContext::default()
    };
    let _l = crate::test_support::test_lock();
    let _lock = crate::joystick::lock_joysticks();
    let (caps, _) = run(&device, |d| {
        [1, 2, 3, 4].map(|j| ctx.get_joystick_capabilities(d, j))
    });
    assert_eq!(
        caps,
        [
            JoystickCaps::RUMBLE,
            JoystickCaps(0),
            JoystickCaps(0),
            JoystickCaps(0)
        ]
    );
    // In PC mode the WaveBird check is skipped
    ctx.pc_mode = true;
    let (caps, _) = run(&device, |d| ctx.get_joystick_capabilities(d, 2));
    assert_eq!(caps, JoystickCaps::RUMBLE);
}
