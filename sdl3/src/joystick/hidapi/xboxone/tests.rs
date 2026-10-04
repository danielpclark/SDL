// Tests for the Xbox One HIDAPI driver.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The expected values follow upstream's SDL_hidapi_xboxone.c.

use super::super::report_descriptor::DescriptorInputField;
use super::super::tests::{run, test_device};
use super::*;
use crate::hidapi::DeviceInfo;

fn xbox_one_s() -> Arc<HidapiDevice> {
    test_device(&DeviceInfo {
        vendor_id: USB_VENDOR_MICROSOFT,
        product_id: USB_PRODUCT_XBOX_ONE_S,
        interface_number: 0,
        ..DeviceInfo::default()
    })
}

fn complete_context() -> XboxOneContext {
    XboxOneContext {
        vendor_id: USB_VENDOR_MICROSOFT,
        product_id: USB_PRODUCT_XBOX_ONE_S,
        init_state: InitState::Complete,
        ..XboxOneContext::default()
    }
}

/// A GIP packet: the encoded header followed by `payload`.
fn gip_packet(hdr: &GipHeader, payload: &[u8]) -> Vec<u8> {
    let hdr_len = gip_get_header_length(hdr);
    let mut packet = vec![0u8; hdr_len + payload.len()];
    gip_encode_header(hdr, &mut packet);
    packet[hdr_len..].copy_from_slice(payload);
    packet
}

#[test]
fn variable_ints() {
    let mut buf = [0u8; 5];
    assert_eq!(encode_variable_int(&mut buf, 0x0e), 1);
    assert_eq!(buf[0], 0x0e);
    assert_eq!(encode_variable_int(&mut buf, 0x1234), 2);
    assert_eq!(&buf[..2], &[0xb4, 0x24]);
    assert_eq!(decode_variable_int(&[0xb4, 0x24, 0x99]), (0x1234, 2));
    assert_eq!(decode_variable_int(&[0x80, 0x00]), (0, 2));
    // Only four bytes are read
    assert_eq!(
        decode_variable_int(&[0xff, 0xff, 0xff, 0xff, 0x01]),
        (0x0fff_ffff, 5)
    );
    // (upstream's count for 28 bits or more is one past the bytes written)
    assert_eq!(encode_variable_int(&mut buf, 0x1000_0000), 5);
    assert_eq!(&buf[..4], &[0x80, 0x80, 0x80, 0x80]);
}

#[test]
fn gip_headers() {
    // An even header
    let hdr = GipHeader {
        command: GIP_CMD_INPUT,
        options: 0,
        sequence: 1,
        packet_length: 14,
        chunk_offset: 0,
    };
    assert_eq!(gip_get_header_length(&hdr), 4);
    let packet = gip_packet(&hdr, &[]);
    assert_eq!(packet, [0x20, 0x00, 0x01, 0x0e]);
    assert_eq!(gip_decode_header(&packet), (hdr, 4));

    // An odd header gets padded with a continuation byte
    let hdr = GipHeader {
        packet_length: 200,
        ..hdr
    };
    assert_eq!(gip_get_actual_header_length(&hdr), 5);
    assert_eq!(gip_get_header_length(&hdr), 6);
    let packet = gip_packet(&hdr, &[]);
    assert_eq!(packet, [0x20, 0x00, 0x01, 0xc8, 0x81, 0x00]);
    assert_eq!(gip_decode_header(&packet), (hdr, 6));

    // A chunk header
    let hdr = GipHeader {
        command: GIP_CMD_INPUT,
        options: GIP_OPT_CHUNK | GIP_OPT_CHUNK_START,
        sequence: 2,
        packet_length: 58,
        chunk_offset: 300,
    };
    assert_eq!(gip_get_header_length(&hdr), 6);
    let packet = gip_packet(&hdr, &[]);
    assert_eq!(packet, [0x20, 0xc0, 0x02, 0x3a, 0xac, 0x02]);
    assert_eq!(gip_decode_header(&packet), (hdr, 6));
}

/// An Xbox One S input report: A and d-pad up pressed, the left trigger
/// all the way down.
const XBOX_ONE_S_INPUT: [u8; 14] = [
    0x10, 0x01, 0xff, 0x03, 0x00, 0x00, 0x34, 0x12, 0x00, 0x00, 0x00, 0x80, 0xff, 0xff,
];

const XBOX_ONE_S_EVENTS: [&str; 17] = [
    "button 6 0",
    "button 4 0",
    "button 0 1",
    "button 1 0",
    "button 2 0",
    "button 3 0",
    "hat 0 1",
    "button 9 0",
    "button 10 0",
    "button 7 0",
    "button 8 0",
    "axis 4 32767",
    "axis 5 -32768",
    "axis 0 4660",
    "axis 1 -1",
    "axis 2 -32768",
    "axis 3 0",
];

#[test]
fn gip_input() {
    let device = xbox_one_s();
    let mut ctx = XboxOneContext {
        init_state: InitState::PrepareInput,
        ..complete_context()
    };
    let hdr = GipHeader {
        command: GIP_CMD_INPUT,
        options: 0,
        sequence: 1,
        packet_length: 14,
        chunk_offset: 0,
    };
    let report = gip_packet(&hdr, &XBOX_ONE_S_INPUT);

    // The first input completes the initialization and is ignored
    let (ok, events) = run(&device, |d| ctx.gip_process_data(d, Some(1), &report));
    assert!(ok);
    assert!(events.is_empty());
    assert_eq!(ctx.init_state, InitState::Complete);

    let (ok, events) = run(&device, |d| ctx.gip_process_data(d, Some(1), &report));
    assert!(ok);
    assert_eq!(events, XBOX_ONE_S_EVENTS);

    // Unchanged buttons aren't sent again, the axes are
    let (_, events) = run(&device, |d| ctx.gip_process_data(d, Some(1), &report));
    assert_eq!(events, XBOX_ONE_S_EVENTS[11..]);

    // Without an open joystick, nothing is sent
    let (_, events) = run(&device, |d| ctx.gip_process_data(d, None, &report));
    assert!(events.is_empty());

    // Packets for devices plugged into the controller are skipped
    let attachment = gip_packet(
        &GipHeader {
            options: 0x01,
            ..hdr
        },
        &XBOX_ONE_S_INPUT,
    );
    let (_, events) = run(&device, |d| ctx.gip_process_data(d, Some(1), &attachment));
    assert!(events.is_empty());
}

#[test]
fn gip_internal_packets() {
    let device = xbox_one_s();
    let mut ctx = XboxOneContext {
        init_state: InitState::Identifying,
        ..complete_context()
    };
    let internal = |command, payload: &[u8]| {
        gip_packet(
            &GipHeader {
                command,
                options: GIP_OPT_INTERNAL,
                sequence: 1,
                packet_length: payload.len() as u32,
                chunk_offset: 0,
            },
            payload,
        )
    };

    // Identification moves on to the startup
    run(&device, |d| {
        ctx.gip_process_data(d, Some(1), &internal(GIP_CMD_IDENTIFY, &[0; 4]))
    });
    assert_eq!(ctx.init_state, InitState::Startup);
    // A status packet completes the initialization
    run(&device, |d| {
        ctx.gip_process_data(d, Some(1), &internal(GIP_CMD_STATUS, &[0; 4]))
    });
    assert_eq!(ctx.init_state, InitState::Complete);
    // An announcement starts it over
    run(&device, |d| {
        ctx.gip_process_data(d, Some(1), &internal(GIP_CMD_ANNOUNCE, &[0; 28]))
    });
    assert_eq!(ctx.init_state, InitState::Announced);

    // The guide button
    let (_, events) = run(&device, |d| {
        ctx.gip_process_data(d, Some(1), &internal(GIP_CMD_VIRTUAL_KEY, &[0x01, 0x5b]))
    });
    assert_eq!(events, ["button 5 1"]);

    // Two packets in one report
    let mut report = internal(GIP_CMD_VIRTUAL_KEY, &[0x00, 0x5b]);
    report.extend(internal(GIP_CMD_VIRTUAL_KEY, &[0x01, 0x5b]));
    let (_, events) = run(&device, |d| ctx.gip_process_data(d, Some(1), &report));
    assert_eq!(events, ["button 5 0", "button 5 1"]);
}

#[test]
fn gip_chunks() {
    let device = xbox_one_s();
    let mut ctx = complete_context();
    let chunk = |options, packet_length: usize, chunk_offset, payload: &[u8]| {
        gip_packet(
            &GipHeader {
                command: GIP_CMD_INPUT,
                options,
                sequence: 3,
                packet_length: packet_length as u32,
                chunk_offset,
            },
            payload,
        )
    };

    // The start packet's chunk offset is the total length
    let first = chunk(
        GIP_OPT_CHUNK | GIP_OPT_CHUNK_START,
        4,
        14,
        &XBOX_ONE_S_INPUT[..4],
    );
    let (ok, events) = run(&device, |d| ctx.gip_process_data(d, Some(1), &first));
    assert!(ok && events.is_empty());
    assert_eq!(ctx.chunk_length, 14);

    let second = chunk(GIP_OPT_CHUNK, 10, 4, &XBOX_ONE_S_INPUT[4..]);
    let (ok, events) = run(&device, |d| ctx.gip_process_data(d, Some(1), &second));
    assert!(ok && events.is_empty());

    // A chunk past the end fails
    let past = chunk(GIP_OPT_CHUNK, 4, 12, &[0; 4]);
    let (ok, _) = run(&device, |d| ctx.gip_process_data(d, Some(1), &past));
    assert!(!ok);

    // An empty chunk dispatches the packet
    let last = chunk(GIP_OPT_CHUNK, 0, 14, &[]);
    let (ok, events) = run(&device, |d| ctx.gip_process_data(d, Some(1), &last));
    assert!(ok);
    assert_eq!(events, XBOX_ONE_S_EVENTS);
    assert!(ctx.chunk_buffer.is_none());
    assert_eq!(ctx.chunk_length, 0);

    // A chunk without a start packet fails
    let (ok, _) = run(&device, |d| ctx.gip_process_data(d, Some(1), &second));
    assert!(!ok);
}

#[test]
fn short_reports() {
    let device = xbox_one_s();
    let mut ctx = complete_context();

    // A header longer than the report
    let (ok, events) = run(&device, |d| {
        ctx.gip_process_data(d, Some(1), &[0x20, 0x00, 0x01, 0x8e])
    });
    assert!(ok && events.is_empty());

    // A packet longer than the report is shortened; the missing bytes read 0
    let mut report = vec![0x20, 0x00, 0x01, 0x0e];
    report.extend_from_slice(&XBOX_ONE_S_INPUT[..6]);
    let (ok, events) = run(&device, |d| ctx.gip_process_data(d, Some(1), &report));
    assert!(ok);
    assert_eq!(
        &events[11..],
        [
            "axis 4 32767",
            "axis 5 -32768",
            "axis 0 0",
            "axis 1 -1",
            "axis 2 0",
            "axis 3 -1"
        ]
    );
}

#[test]
fn elite_paddles() {
    let device = test_device(&DeviceInfo {
        vendor_id: USB_VENDOR_MICROSOFT,
        product_id: USB_PRODUCT_XBOX_ONE_ELITE_SERIES_2,
        interface_number: 0,
        ..DeviceInfo::default()
    });
    let mut ctx = XboxOneContext {
        product_id: USB_PRODUCT_XBOX_ONE_ELITE_SERIES_2,
        has_paddles: true,
        ..complete_context()
    };

    // Xbox One Elite Series 2 4.x firmware: paddles in byte 14, profile in byte 15
    let mut packet = [0u8; USB_PACKET_LENGTH];
    packet[14] = 0x05;
    let (_, events) = run(&device, |d| ctx.handle_state_packet(d, 1, &mut packet, 34));
    assert_eq!(
        &events[..4],
        ["button 11 1", "button 12 0", "button 13 1", "button 14 0"]
    );
    assert_eq!(ctx.last_paddle_state, 0x05);

    // With a profile, the paddles are mapped and read as released
    let mut packet = [0u8; USB_PACKET_LENGTH];
    packet[14] = 0x05;
    packet[15] = 1;
    let (_, events) = run(&device, |d| ctx.handle_state_packet(d, 1, &mut packet, 34));
    assert_eq!(
        &events[..4],
        ["button 11 0", "button 12 0", "button 13 0", "button 14 0"]
    );
    assert_eq!(packet[14], 0);

    // The unmapped state packet of 5.13+ firmware
    let mut packet = [0u8; USB_PACKET_LENGTH];
    packet[14] = 0x02;
    let (_, events) = run(&device, |d| {
        ctx.handle_unmapped_state_packet(d, 1, &mut packet, 17)
    });
    assert_eq!(
        events,
        ["button 11 0", "button 12 1", "button 13 0", "button 14 0"]
    );
    assert!(ctx.has_unmapped_state);
}

#[test]
fn bluetooth_state() {
    let device = xbox_one_s();
    let mut ctx = complete_context();
    let mut packet = [0u8; USB_PACKET_LENGTH];
    packet[..17].copy_from_slice(&[
        0x01, 0x00, 0x00, 0xff, 0xff, 0x00, 0x80, 0x01, 0x80, 0xff, 0x03, 0x00, 0x00, 0x03, 0x01,
        0x18, 0x00,
    ]);
    let (_, events) = run(&device, |d| {
        ctx.bluetooth_handle_state_packet(d, 1, &mut packet, 17)
    });
    assert_eq!(
        events,
        [
            "button 0 1",
            "button 1 0",
            "button 2 0",
            "button 3 0",
            "button 9 0",
            "button 10 0",
            "button 5 1",
            "button 6 1",
            "button 7 0",
            "button 8 0",
            "button 4 0",
            "hat 0 2",
            "axis 4 32767",
            "axis 5 -32768",
            "axis 0 -32768",
            "axis 1 32767",
            "axis 2 0",
            "axis 3 1",
        ]
    );

    // With a separate guide packet, byte 15 doesn't have the guide button
    let (_, events) = run(&device, |d| {
        ctx.bluetooth_handle_guide_packet(d, 1, &[0x02, 0x01])
    });
    assert_eq!(events, ["button 5 1"]);
    packet[15] = 0x10;
    let (_, events) = run(&device, |d| {
        ctx.bluetooth_handle_state_packet(d, 1, &mut packet, 17)
    });
    assert_eq!(
        &events[..4],
        ["button 6 0", "button 7 0", "button 8 0", "button 4 0"]
    );

    // Short packets are ignored
    let (_, events) = run(&device, |d| {
        ctx.bluetooth_handle_state_packet(d, 1, &mut packet, 15)
    });
    assert!(events.is_empty());

    // The battery
    let (_, events) = run(&device, |d| {
        ctx.bluetooth_handle_battery_packet(d, 1, &[0x04, 0x06])
    });
    assert_eq!(events, ["power OnBattery 70"]);
    let (_, events) = run(&device, |d| {
        ctx.bluetooth_handle_battery_packet(d, 1, &[0x04, 0x03])
    });
    assert_eq!(events, ["power Charging 100"]);
}

#[test]
fn serial_numbers() {
    let _l = crate::test_support::test_lock();
    let _lock = crate::joystick::lock_joysticks();

    let device = xbox_one_s();
    let mut ctx = complete_context();
    let mut data = [0u8; USB_PACKET_LENGTH];
    data[2..16].copy_from_slice(&[
        0x30, 0x39, 0x37, 0x31, 0x32, 0x33, 0x33, 0x32, 0x33, 0x35, 0x34, 0x30, 0x33, 0x36,
    ]);
    run(&device, |d| ctx.handle_serial_id_packet(d, &data));
    assert_eq!(
        device.serial().as_deref(),
        Some("3039373132333332333534303336")
    );

    // (upstream's truncation at a byte under 0x10)
    data[4] = 0x05;
    run(&device, |d| ctx.handle_serial_id_packet(d, &data));
    assert_eq!(device.serial().as_deref(), Some("30395"));
}

#[test]
fn home_led_brightness() {
    assert_eq!(get_home_led_brightness(None), 20);
    assert_eq!(get_home_led_brightness(Some("")), 20);
    assert_eq!(get_home_led_brightness(Some("1")), 20);
    assert_eq!(get_home_led_brightness(Some("0")), 0);
    assert_eq!(get_home_led_brightness(Some("0.5")), 25);
    assert_eq!(get_home_led_brightness(Some("1.0")), 50);
}

#[test]
fn controller_features() {
    assert!(controller_has_color_led(
        USB_VENDOR_MICROSOFT,
        USB_PRODUCT_XBOX_ONE_ELITE_SERIES_2
    ));
    assert!(!controller_has_color_led(
        USB_VENDOR_MICROSOFT,
        USB_PRODUCT_XBOX_ONE_S
    ));
    assert!(controller_has_trigger_rumble(USB_VENDOR_MICROSOFT, 0x1234));
    assert!(!controller_has_trigger_rumble(USB_VENDOR_PDP, 0x1234));
    assert!(!controller_sends_announcement(USB_VENDOR_POWERA, 0x400b));
    assert!(controller_sends_announcement(
        USB_VENDOR_MICROSOFT,
        USB_PRODUCT_XBOX_ONE_S
    ));

    let driver = XboxOneDriver;
    let supported = |class, subclass, protocol, gamepad_type| {
        driver.is_supported_device(
            None,
            "",
            gamepad_type,
            0,
            0,
            0,
            0,
            class,
            subclass,
            protocol,
        )
    };
    assert!(supported(0, 0, 0, GamepadType::XboxOne));
    assert!(supported(0xff, 71, 208, GamepadType::XboxOne));
    assert!(!supported(0xff, 93, 1, GamepadType::XboxOne));
    assert!(!supported(0, 0, 0, GamepadType::Xbox360));
}

fn field(usage_page: u16, usage: u16, bit_offset: i32, bit_size: i32) -> DescriptorInputField {
    DescriptorInputField {
        report_id: 1,
        usage: make_usage(usage_page, usage),
        bit_offset,
        bit_size,
    }
}

/// The fields of a Bluetooth Xbox controller descriptor, with separate
/// buttons.
fn bluetooth_descriptor(buttons: u16) -> ReportDescriptor {
    let mut fields = vec![
        field(USB_USAGEPAGE_GENERIC_DESKTOP, USB_USAGE_GENERIC_X, 0, 16),
        field(USB_USAGEPAGE_GENERIC_DESKTOP, USB_USAGE_GENERIC_Y, 16, 16),
        field(USB_USAGEPAGE_GENERIC_DESKTOP, USB_USAGE_GENERIC_Z, 32, 16),
        field(USB_USAGEPAGE_GENERIC_DESKTOP, USB_USAGE_GENERIC_RZ, 48, 16),
        field(USB_USAGEPAGE_SIMULATION, USB_USAGE_SIMULATION_BRAKE, 64, 10),
        field(
            USB_USAGEPAGE_SIMULATION,
            USB_USAGE_SIMULATION_ACCELERATOR,
            80,
            10,
        ),
        field(USB_USAGEPAGE_GENERIC_DESKTOP, USB_USAGE_GENERIC_HAT, 96, 4),
    ];
    for i in 0..buttons {
        fields.push(field(USB_USAGEPAGE_BUTTON, 1 + i, 104 + i32::from(i), 1));
    }
    fields.push(field(
        USB_USAGEPAGE_CONSUMER,
        USB_USAGE_CONSUMER_AC_BACK,
        120,
        1,
    ));
    ReportDescriptor { fields }
}

#[test]
fn descriptor_buttons() {
    let collapsed = collapse_descriptor_buttons(bluetooth_descriptor(15)).unwrap();
    assert_eq!(collapsed.fields.len(), 9);
    assert_eq!(collapsed.fields[7], field(USB_USAGEPAGE_BUTTON, 1, 104, 15));
    assert_eq!(
        collapsed.fields[8].usage,
        make_usage(USB_USAGEPAGE_CONSUMER, USB_USAGE_CONSUMER_AC_BACK)
    );

    // The expected button counts are 12 and 15
    assert!(collapse_descriptor_buttons(bluetooth_descriptor(12)).is_some());
    assert!(collapse_descriptor_buttons(bluetooth_descriptor(16)).is_none());
    // ... and the descriptor needs X and Y
    let mut descriptor = bluetooth_descriptor(15);
    descriptor.fields.remove(1);
    assert!(collapse_descriptor_buttons(descriptor).is_none());
}

#[test]
fn descriptor_reports() {
    let device = xbox_one_s();
    let mut ctx = XboxOneContext {
        descriptor: collapse_descriptor_buttons(bluetooth_descriptor(15)),
        ..complete_context()
    };
    let report = [
        0x01, 0x00, 0x00, 0xff, 0xff, 0x00, 0x80, 0x01, 0x80, 0xff, 0x03, 0x00, 0x00, 0x01, 0x01,
        0x04, 0x00,
    ];
    let (_, events) = run(&device, |d| {
        ctx.handle_descriptor_report(d, 1, &report, report.len())
    });
    assert_eq!(
        events,
        [
            "axis 0 -32768",
            "axis 1 32767",
            "axis 2 0",
            "axis 3 1",
            "axis 4 32767",
            "axis 5 -32768",
            "hat 0 1",
            "button 0 1",
            "button 1 0",
            "button 2 0",
            "button 3 0",
            "button 9 0",
            "button 10 0",
            "button 4 1",
            "button 6 0",
            "button 5 0",
            "button 7 0",
            "button 8 0",
            "button 4 0",
        ]
    );

    // Unchanged buttons aren't sent again; a separate back button takes
    // over from the one in the buttons
    let mut report = report;
    report[16] = 0x01;
    let (_, events) = run(&device, |d| {
        ctx.handle_descriptor_report(d, 1, &report, report.len())
    });
    assert_eq!(&events[7..], ["button 4 1"]);
    assert!(ctx.has_separate_back_button);
    report[15] = 0x00;
    let (_, events) = run(&device, |d| {
        ctx.handle_descriptor_report(d, 1, &report, report.len())
    });
    assert_eq!(
        &events[7..],
        [
            "button 0 1",
            "button 1 0",
            "button 2 0",
            "button 3 0",
            "button 9 0",
            "button 10 0",
            "button 6 0",
            "button 5 0",
            "button 7 0",
            "button 8 0",
            "button 4 1"
        ]
    );

    // Other reports are ignored
    let (_, events) = run(&device, |d| {
        ctx.handle_descriptor_report(d, 1, &[0x02, 0x01], 2)
    });
    assert!(events.is_empty());
    let (ok, _) = run(&device, |d| ctx.handle_descriptor_report(d, 1, &[], 0));
    assert!(!ok);
}
