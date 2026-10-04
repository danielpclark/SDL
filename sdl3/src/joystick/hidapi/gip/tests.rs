// Tests for the GIP HIDAPI driver.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The transcript in `tests/` comes from running upstream's
//! SDL_hidapi_gip.c, with stubs for the SDL and HID functions it uses
//! (printing the reports it writes, the ones it queues on the rumble
//! thread as `async`, the joystick and keyboard events, and replying from
//! a queue of reads as [`FakeHid`] does), on generated messages: the
//! handshakes (hello, metadata in fragments, the init sequences, the
//! metadata retries, resets and faked metadata), the acknowledgements, the
//! input reports of each kind of device, the Elite paddles, the chatpad,
//! the battery, rumble, the guide button LED and the messages that go
//! wrong. Its clock starts at 1000 ms and moves only on `advance` steps;
//! the rumble queue sends right away.
//!
//! The reports written, the joystick events, the keyboard actions and the
//! other outputs of a step are each compared in order (the C stubs print
//! them as they happen; here they come from separate places).

use std::cell::{Cell, RefCell};

use super::super::steam::tests::{hex_line, parse_transcript, Entry, FakeHid, Step};
use super::super::steam::SteamHid;
use super::super::tests::{run, test_device};
use super::*;
use crate::hidapi::{BusType, DeviceInfo};

const TRANSCRIPT: &str = include_str!("tests/gip.txt");

/// The device of the tests: the reads of a [`FakeHid`], a log of what's
/// written and queued, a clock and the open joysticks.
struct FakePort {
    hid: FakeHid,
    log: RefCell<Vec<String>>,
    now: Cell<u64>,
    open: Cell<bool>,
    device: Arc<HidapiDevice>,
}

impl FakePort {
    fn new(device: &Arc<HidapiDevice>) -> FakePort {
        FakePort {
            hid: FakeHid::new(false),
            log: RefCell::new(Vec::new()),
            now: Cell::new(1000),
            open: Cell::new(true),
            device: device.clone(),
        }
    }

    fn take_log(&self) -> Vec<String> {
        std::mem::take(&mut *self.log.borrow_mut())
    }
}

impl GipPort for FakePort {
    fn read_timeout(&self, data: &mut [u8], milliseconds: i32) -> Result<usize> {
        self.hid.read_timeout(data, milliseconds)
    }
    fn write(&self, data: &[u8]) -> Result<usize> {
        self.log.borrow_mut().push(hex_line("write", data));
        Ok(data.len())
    }
    fn send_async(
        &self,
        data: &[u8],
        on_sent: Option<Box<dyn FnOnce(u64) + Send>>,
    ) -> Result<usize> {
        if data.len() > 2 * USB_PACKET_LENGTH {
            return Err(Error::new("too big"));
        }
        self.log.borrow_mut().push(hex_line("async", data));
        if let Some(on_sent) = on_sent {
            on_sent(self.now.get());
        }
        Ok(data.len())
    }
    fn ticks_ms(&self) -> u64 {
        self.now.get()
    }
    fn joystick_open(&self, joystick: JoystickID) -> bool {
        self.open.get() && joystick != 0 && self.device.joysticks().contains(&joystick)
    }
}

/// The keyboard actions as the transcript prints them.
fn describe_keyboard(actions: &[KeyboardAction]) -> Vec<String> {
    actions
        .iter()
        .map(|action| match action {
            KeyboardAction::Add(_) => format!("keyboard add {CHATPAD_NAME}"),
            KeyboardAction::Remove(_) => "keyboard remove".to_owned(),
            KeyboardAction::Key(_, _, scancode, down) => {
                format!("key {} {}", scancode.0, u8::from(*down))
            }
            KeyboardAction::Text(text) => hex_line("text", text.as_bytes()),
        })
        .collect()
}

/// Run a driver function with the fake port: its result, joystick events
/// and keyboard actions.
fn run_io<R>(
    device: &Arc<HidapiDevice>,
    port: &FakePort,
    f: impl FnOnce(&mut Io<'_, '_>) -> R,
) -> (R, Vec<String>, Vec<String>) {
    let ((result, keyboard), events) = run(device, |d| {
        let mut io = Io::new(port, d);
        let result = f(&mut io);
        (result, io.keyboard)
    });
    (result, events, describe_keyboard(&keyboard))
}

/// The lines of a driver function's result.
fn result_lines(result: &Result<()>) -> Vec<String> {
    let mut lines = Vec::new();
    if let Err(e) = result {
        if e.kind() == crate::error::ErrorKind::Unsupported {
            lines.push("unsupported".to_owned());
        } else {
            lines.push(format!("error {}", e.message()));
        }
    }
    lines.push(format!("result {}", u8::from(result.is_ok())));
    lines
}

/// The kinds of output lines, compared separately.
fn kind_of(line: &str) -> usize {
    let word = line.split(' ').next().unwrap_or_default();
    match word {
        "write" | "async" => 0,
        "added" | "removed" | "button" | "axis" | "hat" | "power" => 1,
        "keyboard" | "key" | "text" => 2,
        _ => 3,
    }
}

/// Compare a step's outputs with what the Rust driver did.
fn check(
    step: &Step,
    hid: Vec<String>,
    events: Vec<String>,
    keyboard: Vec<String>,
    other: Vec<String>,
) {
    let actual = [hid, events, keyboard, other];
    for (kind, actual) in actual.iter().enumerate() {
        let expected: Vec<&String> = step
            .outputs
            .iter()
            .filter(|line| kind_of(line) == kind)
            .collect();
        assert_eq!(
            actual.iter().collect::<Vec<_>>(),
            expected,
            "outputs of kind {kind} of {:?} {:?}",
            step.head,
            step.inputs
        );
    }
}

fn controller(vendor_id: u16, product_id: u16) -> Arc<HidapiDevice> {
    test_device(&DeviceInfo {
        vendor_id,
        product_id,
        interface_number: 0,
        ..DeviceInfo::default()
    })
}

/// Replay a transcript of the C driver.
fn replay(transcript: &str) {
    let mut device = controller(0, 0);
    let mut port = FakePort::new(&device);
    let mut ctx = GipContext::default();

    for entry in parse_transcript(transcript) {
        let step = match entry {
            Entry::Case(words) => {
                // case <name> <vendor> <product> <reset for metadata> <series X>
                let vendor_id = u16::from_str_radix(&words[1], 16).unwrap();
                let product_id = u16::from_str_radix(&words[2], 16).unwrap();
                device = controller(vendor_id, product_id);
                port = FakePort::new(&device);
                ctx = GipContext::default();
                hints::set(hints::JOYSTICK_HIDAPI_GIP_RESET_FOR_METADATA, &words[3]).unwrap();
                assert_eq!(
                    is_joystick_xbox_series_x(vendor_id, product_id),
                    words[4] == "1",
                    "{}",
                    words[0]
                );
                continue;
            }
            Entry::Step(step) => step,
        };
        port.hid.queue(&step);
        let words: Vec<&str> = step.head.split_whitespace().collect();
        let arg = |i: usize| words[i].parse::<u16>().unwrap();
        let joystick_of = |ctx: &GipContext, index: usize| {
            ctx.attachments[index]
                .as_ref()
                .map_or(0, |attachment| attachment.joystick)
        };
        let serial = device.serial();
        let (events, keyboard, mut other) = match words[0] {
            "init" => {
                let ((), events, keyboard) = run_io(&device, &port, |io| ctx.init(io));
                (events, keyboard, vec!["result 1".to_owned()])
            }
            "update" => {
                let (ok, events, keyboard) = run_io(&device, &port, |io| ctx.update(io));
                (events, keyboard, vec![format!("result {}", u8::from(ok))])
            }
            "advance" => {
                port.now.set(port.now.get() + u64::from(arg(1)));
                Default::default()
            }
            "joystickopen" => {
                port.open.set(words[1] == "1");
                Default::default()
            }
            "open" => {
                let mut joystick = JoystickData::new(joystick_of(&ctx, usize::from(arg(1))));
                let (result, events, keyboard) =
                    run_io(&device, &port, |io| ctx.open(io, &mut joystick));
                let mut other = vec![format!(
                    "buttons {} axes {} hats {} type {}",
                    joystick.nbuttons,
                    joystick.naxes,
                    joystick.nhats,
                    device.state().joystick_type as i32
                )];
                other.extend(result_lines(&result));
                (events, keyboard, other)
            }
            "rumble" | "triggers" | "led" => {
                let joystick = joystick_of(&ctx, usize::from(arg(1)));
                let result = match words[0] {
                    "rumble" => ctx.rumble(&port, joystick, arg(2), arg(3)),
                    "triggers" => ctx.rumble_triggers(&port, joystick, arg(2), arg(3)),
                    _ => ctx.set_led(&port, joystick, arg(2) as u8, arg(3) as u8, arg(4) as u8),
                };
                (Vec::new(), Vec::new(), result_lines(&result))
            }
            "caps" => {
                let joystick = joystick_of(&ctx, usize::from(arg(1)));
                let (caps, _) = run(&device, |d| ctx.get_joystick_capabilities(d, joystick));
                (Vec::new(), Vec::new(), vec![format!("caps {:x}", caps.0)])
            }
            "free" => (Vec::new(), describe_keyboard(&ctx.free()), Vec::new()),
            other => panic!("unknown step {other}"),
        };
        if device.serial() != serial {
            other.insert(0, format!("serial {}", device.serial().unwrap_or_default()));
        }
        check(&step, port.take_log(), events, keyboard, other);
    }
}

/// Replays the C transcript.
#[test]
fn transcript() {
    let _l = crate::test_support::test_lock();
    let _lock = crate::joystick::lock_joysticks();

    // (the list of the older controllers, without a share button, that
    // the joystick subsystem loads)
    let old_xboxone_controllers = &crate::joystick::device_info::OLD_XBOXONE_CONTROLLERS;
    old_xboxone_controllers.load();
    replay(TRANSCRIPT);
    old_xboxone_controllers.free();

    hints::reset(hints::JOYSTICK_HIDAPI_GIP_RESET_FOR_METADATA);
    super::super::NUMJOYSTICKS.store(0, Ordering::Relaxed);
    super::super::CHANGE_COUNT.store(0, Ordering::Relaxed);
}

#[test]
fn lengths() {
    let mut buf = [0u8; 10];
    assert_eq!(encode_length(0, &mut buf), 1);
    assert_eq!(buf[0], 0);
    assert_eq!(encode_length(0x7f, &mut buf), 1);
    assert_eq!(encode_length(0x1234, &mut buf), 2);
    assert_eq!(&buf[..2], &[0xb4, 0x24]);
    assert_eq!(encode_length(0x4000, &mut buf), 3);
    assert_eq!(&buf[..3], &[0x80, 0x80, 0x01]);
    // Only as many bytes as there's room for
    assert_eq!(encode_length(0x4000, &mut buf[..2]), 2);
    assert_eq!(decode_length(&[0xb4, 0x24, 0x99]), (0x1234, 2));
    assert_eq!(decode_length(&[0x80, 0x80]), (0, 2));
    assert_eq!(decode_length(&[]), (0, 0));
    // (past nine bytes, the shift wraps around)
    let mut long = [0x80u8; 11];
    long[10] = 0x01;
    assert_eq!(decode_length(&long), (1 << 6, 11));
}

#[test]
fn guids_and_sequences() {
    // The bytes of 9776ff56-9bfd-4581-ad45-b645bba526d6 on the wire
    assert_eq!(
        GUID_ICONTROLLER,
        [
            0x56, 0xff, 0x76, 0x97, 0xfd, 0x9b, 0x81, 0x45, 0xad, 0x45, 0xb6, 0x45, 0xbb, 0xa5,
            0x26, 0xd6
        ]
    );

    let mut attachment = Attachment::new(0);
    // The sequence numbers skip 0, each kind of message on its own
    assert_eq!(attachment.sequence_next(GIP_CMD_METADATA, true), 1);
    assert_eq!(attachment.sequence_next(GIP_CMD_LED, true), 2);
    assert_eq!(attachment.sequence_next(GIP_CMD_SECURITY, true), 1);
    assert_eq!(attachment.sequence_next(GIP_CMD_EXTENDED, true), 1);
    assert_eq!(attachment.sequence_next(GIP_AUDIO_DATA, true), 1);
    assert_eq!(attachment.sequence_next(GIP_CMD_GUIDE_COLOR, false), 1);
    assert_eq!(attachment.sequence_next(GIP_CMD_DIRECT_MOTOR, false), 0);
    attachment.seq_vendor = 255;
    assert_eq!(attachment.sequence_next(GIP_CMD_GUIDE_COLOR, false), 255);
    assert_eq!(attachment.sequence_next(GIP_CMD_GUIDE_COLOR, false), 1);

    // The default system messages, and a message past 31 (with the shift
    // masked, as meant)
    assert!(attachment.supports_system_message(GIP_CMD_METADATA, true));
    assert!(!attachment.supports_system_message(GIP_CMD_GUIDE_BUTTON, true));
    assert!(attachment.supports_system_message(GIP_CMD_LED, false));
    assert!(!attachment.supports_system_message(GIP_AUDIO_DATA, true));
    attachment.metadata.device.in_system_messages[3] = 1;
    assert!(attachment.supports_system_message(GIP_AUDIO_DATA, true));
    assert_eq!(Attachment::new(1).attachment_type, AttachmentType::Unknown);
}

#[test]
fn reports_past_their_tables() {
    let _l = crate::test_support::test_lock();
    let _lock = crate::joystick::lock_joysticks();

    let device = controller(USB_VENDOR_PDP, 0x0170);
    let port = FakePort::new(&device);
    let mut attachment = Attachment::new(0);
    let mut bytes = padded(&[0; 10]);
    // A pickup selector past the table sends no axis
    bytes[4] = 0x50;
    let (_, events, _) = run_io(&device, &port, |io| {
        attachment.handle_guitar_report(io, 0, &bytes, 10)
    });
    assert_eq!(events.len(), 12);
    assert_eq!(events.last().unwrap(), "axis 4 -32768");
    bytes[4] = 0x40;
    let (_, events, _) = run_io(&device, &port, |io| {
        attachment.handle_guitar_report(io, 0, &bytes, 10)
    });
    assert_eq!(events.last().unwrap(), "axis 3 24576");

    // A flight stick's extra buttons past the eighth are never down
    attachment.extra_buttons = 10;
    attachment.extra_button_idx = 20;
    let mut bytes = padded(&[0xff; 19]);
    bytes[2] = 0;
    let (_, events, _) = run_io(&device, &port, |io| {
        attachment.handle_flight_stick_report(io, 0, &bytes, 19)
    });
    assert_eq!(events[..10].iter().filter(|e| e.ends_with(" 1")).count(), 8);
    assert_eq!(events[8], "button 28 0");
    assert_eq!(events[9], "button 29 0");

    // A reassembled input report longer than the saved input
    attachment.attachment_type = AttachmentType::Gamepad;
    attachment.features = GIP_FEATURE_CONSOLE_FUNCTION_MAP;
    attachment.share_button_idx = 11;
    let report = vec![0u8; 100];
    let mut gip = GipDevice::default();
    let joystick = run(&device, |d| d.joystick_connected()).0;
    attachment.joystick = joystick;
    let (ok, events, _) = run_io(&device, &port, |io| {
        attachment.handle_ll_input_report(&mut gip, io, &report, report.len())
    });
    assert!(ok);
    assert_eq!(events.last().unwrap(), "button 11 0");
    run(&device, |d| d.joystick_disconnected(joystick));
    super::super::NUMJOYSTICKS.store(0, Ordering::Relaxed);
    super::super::CHANGE_COUNT.store(0, Ordering::Relaxed);
}

/// A fragment header, then `data`.
fn fragment(flags: u8, length: u64, offset: u64, data: &[u8]) -> Vec<u8> {
    let mut packet = vec![GIP_LL_STATIC_CONFIGURATION, GIP_FLAG_FRAGMENT | flags, 1];
    let mut buf = [0u8; 10];
    let n = encode_length(length, &mut buf);
    packet.extend_from_slice(&buf[..n]);
    let n = encode_length(offset, &mut buf);
    packet.extend_from_slice(&buf[..n]);
    packet.extend_from_slice(data);
    packet
}

#[test]
fn fragments_that_would_crash() {
    let _l = crate::test_support::test_lock();
    let _lock = crate::joystick::lock_joysticks();

    let device = controller(USB_VENDOR_MICROSOFT, USB_PRODUCT_XBOX_ONE_S);
    let port = FakePort::new(&device);
    let mut ctx = GipContext::default();

    // A fragment whose end wraps around counts as too long: it's
    // acknowledged with what's missing
    let first = fragment(GIP_FLAG_INIT_FRAG, 4, 100, &[1, 2, 3, 4]);
    run_io(&device, &port, |io| ctx.receive_packet(io, &first));
    let wrapping = fragment(0, u64::MAX - 1, 4, &[5, 6]);
    run_io(&device, &port, |io| ctx.receive_packet(io, &wrapping));
    assert_eq!(
        port.take_log(),
        ["write 01 20 01 09 00 21 00 04 00 00 00 60 00"]
    );
    let attachment = ctx.attachments[0].as_ref().unwrap();
    assert_eq!(attachment.fragment_retries, 1);
    assert_eq!(
        attachment.fragment_data.as_ref().unwrap()[..4],
        [1, 2, 3, 4]
    );

    // After an initial fragment that's too long, there's no buffer: the
    // fragments and the message are dropped
    let too_long = fragment(GIP_FLAG_INIT_FRAG, 8, 6, &[0; 8]);
    run_io(&device, &port, |io| ctx.receive_packet(io, &too_long));
    let attachment = ctx.attachments[0].as_mut().unwrap();
    assert!(attachment.fragment_data.is_none());
    assert_eq!(attachment.fragment_message, GIP_LL_STATIC_CONFIGURATION);
    attachment.fragment_offset = 0;
    let next = fragment(GIP_FLAG_ACME, 4, 0, &[1, 2, 3, 4]);
    run_io(&device, &port, |io| ctx.receive_packet(io, &next));
    assert_eq!(
        port.take_log(),
        ["write 01 20 01 09 00 21 00 04 00 00 00 02 00"]
    );
    let last = fragment(GIP_FLAG_ACME, 0, 4, &[]);
    run_io(&device, &port, |io| ctx.receive_packet(io, &last));
    assert!(port.take_log().is_empty());
    assert_eq!(ctx.attachments[0].as_ref().unwrap().fragment_message, 0);
}

#[test]
fn supported_devices() {
    let _l = crate::test_support::test_lock();
    let driver = GipDriver;
    let usb = controller(USB_VENDOR_MICROSOFT, USB_PRODUCT_XBOX_ONE_S);
    let bluetooth = test_device(&DeviceInfo {
        vendor_id: USB_VENDOR_MICROSOFT,
        product_id: USB_PRODUCT_XBOX_ONE_S,
        bus_type: BusType::Bluetooth,
        ..DeviceInfo::default()
    });
    let supported = |device: Option<&HidapiDevice>, gamepad_type| {
        driver.is_supported_device(device, "", gamepad_type, 0, 0, 0, 0, 0, 0, 0)
    };
    assert!(supported(Some(&usb), GamepadType::XboxOne));
    assert!(supported(None, GamepadType::XboxOne));
    assert!(!supported(Some(&bluetooth), GamepadType::XboxOne));
    assert!(!supported(Some(&usb), GamepadType::Xbox360));

    // The hints, from the most specific
    assert!(driver.is_enabled() == SDL_HIDAPI_DEFAULT);
    hints::set(hints::JOYSTICK_HIDAPI_XBOX, "0").unwrap();
    assert!(!driver.is_enabled());
    hints::set(hints::JOYSTICK_HIDAPI_XBOX_ONE, "1").unwrap();
    assert!(driver.is_enabled());
    hints::set(hints::JOYSTICK_HIDAPI_GIP, "0").unwrap();
    assert!(!driver.is_enabled());
    for hint in [
        hints::JOYSTICK_HIDAPI_XBOX,
        hints::JOYSTICK_HIDAPI_XBOX_ONE,
        hints::JOYSTICK_HIDAPI_GIP,
    ] {
        hints::reset(hint);
    }

    // The driver comes right before the Xbox One driver
    let drivers = super::super::HIDAPI_DRIVERS;
    let gip = drivers
        .iter()
        .position(|d| std::ptr::eq(*d, &super::super::DRIVER_GIP))
        .unwrap();
    assert!(std::ptr::eq(
        drivers[gip + 1],
        &super::super::DRIVER_XBOXONE
    ));
}
