// Tests for the SHIELD HIDAPI driver.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The cases in `data.rs` come from running upstream's SDL_hidapi_shield.c,
//! with stubs for the SDL functions it uses, on generated reports. The
//! commands upstream sends through the rumble thread are compared with the
//! packets the driver builds for them.

use super::super::tests::{run, test_device};
use super::*;
use crate::hidapi::DeviceInfo;

mod data;

fn device(product_id: u16) -> std::sync::Arc<HidapiDevice> {
    test_device(&DeviceInfo {
        vendor_id: USB_VENDOR_NVIDIA,
        product_id,
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
    let driver = ShieldDriver;
    let supported = |vid, pid| {
        driver.is_supported_device(None, "", GamepadType::Standard, vid, pid, 0, 0, 0, 0, 0)
    };
    assert!(supported(
        USB_VENDOR_NVIDIA,
        USB_PRODUCT_NVIDIA_SHIELD_CONTROLLER_V103
    ));
    assert!(supported(
        USB_VENDOR_NVIDIA,
        USB_PRODUCT_NVIDIA_SHIELD_CONTROLLER_V104
    ));
    assert!(!supported(USB_VENDOR_NVIDIA, 0xb400));
}

#[test]
fn state_reports() {
    for (name, reports) in data::CASES {
        let device = device(match *name {
            "v103" => USB_PRODUCT_NVIDIA_SHIELD_CONTROLLER_V103,
            _ => USB_PRODUCT_NVIDIA_SHIELD_CONTROLLER_V104,
        });
        let mut ctx = ShieldContext::default();
        for (n, (report, expected)) in reports.iter().enumerate() {
            let mut data = [0u8; USB_PACKET_LENGTH];
            data[..report.len()].copy_from_slice(report);
            let ((), events) = run(&device, |d| ctx.handle_report(d, 1, &data, report.len()));
            assert_eq!(events, *expected, "{name} report {n}");
        }
    }
}

/// Run a call of the harness on the packet builders: the lines of what
/// upstream sends.
fn call(ctx: &mut ShieldContext, name: &str) -> Vec<String> {
    let words: Vec<&str> = name.split(' ').collect();
    let effect31: Vec<u8> = (0..31).map(|i| (i * 7) as u8).collect();
    let command = |ctx: &mut ShieldContext, cmd: u8, payload: &[u8]| {
        vec![hex_line("rumble", &ctx.command_packet(cmd, payload))]
    };
    match words[0] {
        "effect" => match words[1] {
            "1" => command(ctx, 0x42, &[]),
            "4" => command(ctx, 0x39, &[0x01, 0x02, 0x03]),
            "31" => command(ctx, effect31[0], &effect31[1..]),
            _ => {
                let (result, _) = run(&device(USB_PRODUCT_NVIDIA_SHIELD_CONTROLLER_V104), |d| {
                    ctx.send_joystick_effect(d, 1, &[])
                });
                vec![format!("error {}", result.unwrap_err().message())]
            }
        },
        "rumble" => {
            let low = u16::from_str_radix(words[1], 16).unwrap();
            let high = u16::from_str_radix(words[2], 16).unwrap();
            assert!(ctx.set_rumble(low, high));
            let rumble_data = ctx.next_rumble().unwrap();
            command(ctx, CMD_RUMBLE, &rumble_data)
        }
        "ack" => {
            // (the CMD_RUMBLE response sends the pending update)
            ctx.rumble_report_pending = false;
            let rumble_data = ctx.next_rumble().unwrap();
            command(ctx, CMD_RUMBLE, &rumble_data)
        }
        "v103" => {
            let low = u16::from_str_radix(words[1], 16).unwrap();
            let high = u16::from_str_radix(words[2], 16).unwrap();
            vec![hex_line("rumble", &v103_rumble_packet(low, high))]
        }
        _ => panic!("unknown call {name}"),
    }
}

#[test]
fn commands() {
    let mut ctx = ShieldContext {
        seq_num: 0xfe,
        ..ShieldContext::default()
    };
    for (name, expected) in data::CALLS {
        if *name == "ack" {
            ctx.rumble_update_pending = true;
        }
        assert_eq!(call(&mut ctx, name), *expected, "{name}");
    }
    // Nothing is pending once sent
    assert_eq!(ctx.next_rumble(), None);
}

#[test]
fn oversized_command() {
    let mut ctx = ShieldContext::default();
    let (result, _) = run(&device(USB_PRODUCT_NVIDIA_SHIELD_CONTROLLER_V104), |d| {
        ctx.send_command(d, 0x01, &[0; COMMAND_PAYLOAD_SIZE + 1])
    });
    let expected = if cfg!(target_os = "macos") {
        "That operation is not supported"
    } else {
        "Command data exceeds HID report size"
    };
    assert_eq!(result.unwrap_err().message(), expected);
    // The sequence number isn't used up
    assert_eq!(ctx.seq_num, 0);
}
