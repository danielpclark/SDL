// Tests for the Steam Controller HIDAPI driver.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The transcripts in `tests/` come from running upstream's
//! SDL_hidapi_steam.c, with stubs for the SDL and HID functions it uses
//! (printing the events and reports it sends, and replying from queues as
//! [`FakeHid`] does), on generated reports. `steam_win.txt` has the steps
//! that differ when built for Windows (BLE feature reports with a second
//! report ID).
//!
//! The fake HID device and the transcript reader are shared with the Steam
//! Deck and Triton driver tests.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use super::super::tests::{run, test_device};
use super::*;
use crate::hidapi::{BusType, DeviceInfo};

/// A HID device for the Valve drivers' tests: it logs the reports sent to
/// it and replies from queues, as the stubs of the C transcripts.
#[derive(Default)]
pub(in crate::joystick::hidapi) struct FakeHid {
    pub(in crate::joystick::hidapi) bluetooth: bool,
    replies: RefCell<VecDeque<Vec<u8>>>,
    /// The reports to read (`None` for a read error)
    reads: RefCell<VecDeque<Option<Vec<u8>>>>,
    log: RefCell<Vec<String>>,
}

/// `tag` and the bytes in hex, as the transcripts print them.
pub(in crate::joystick::hidapi) fn hex_line(tag: &str, data: &[u8]) -> String {
    let mut line = tag.to_owned();
    for byte in data {
        line.push_str(&format!(" {byte:02x}"));
    }
    line
}

/// The bytes of a transcript's hex dump.
pub(in crate::joystick::hidapi) fn hex_bytes(text: &str) -> Vec<u8> {
    text.split_whitespace()
        .map(|b| u8::from_str_radix(b, 16).unwrap())
        .collect()
}

impl FakeHid {
    pub(in crate::joystick::hidapi) fn new(bluetooth: bool) -> FakeHid {
        FakeHid {
            bluetooth,
            ..FakeHid::default()
        }
    }

    /// The reports sent since the last call.
    pub(in crate::joystick::hidapi) fn take_log(&self) -> Vec<String> {
        std::mem::take(&mut *self.log.borrow_mut())
    }

    /// Queue the inputs of a transcript step (its `read` and `reply`
    /// lines), after the left-overs of the last step are dropped.
    pub(in crate::joystick::hidapi) fn queue(&self, step: &Step) {
        self.replies.borrow_mut().clear();
        self.reads.borrow_mut().clear();
        for input in &step.inputs {
            if let Some(read) = input.strip_prefix("read ") {
                // ("read 0" and "read -1" are sizes, the rest hex bytes)
                self.reads.borrow_mut().push_back(match read {
                    "0" => Some(Vec::new()),
                    "-1" => None,
                    _ => Some(hex_bytes(read)),
                });
            } else if let Some(reply) = input.strip_prefix("reply ") {
                self.replies.borrow_mut().push_back(hex_bytes(reply));
            }
        }
    }
}

impl SteamHid for FakeHid {
    fn is_bluetooth(&self) -> bool {
        self.bluetooth
    }
    fn send_feature_report(&self, data: &[u8]) -> Result<usize> {
        self.log.borrow_mut().push(hex_line("feature", data));
        Ok(data.len())
    }
    fn get_feature_report(&self, data: &mut [u8]) -> Result<usize> {
        self.log
            .borrow_mut()
            .push(format!("getfeature {}", data.len()));
        let reply = self
            .replies
            .borrow_mut()
            .pop_front()
            .ok_or_else(|| Error::new("no reply"))?;
        let size = reply.len().min(data.len());
        data[..size].copy_from_slice(&reply[..size]);
        Ok(size)
    }
    fn read(&self, data: &mut [u8]) -> Result<usize> {
        match self.reads.borrow_mut().pop_front() {
            None => Ok(0),
            Some(None) => Err(Error::new("read error")),
            Some(Some(report)) => {
                data.fill(0);
                let size = report.len().min(data.len());
                data[..size].copy_from_slice(&report[..size]);
                Ok(report.len())
            }
        }
    }
    fn read_timeout(&self, data: &mut [u8], _milliseconds: i32) -> Result<usize> {
        self.read(data)
    }
    fn write(&self, data: &[u8]) -> Result<usize> {
        self.log.borrow_mut().push(hex_line("write", data));
        Ok(data.len())
    }
}

/// A step of a transcript: its head line (what the C driver did), the
/// inputs it was given and its outputs, up to the `end` line.
#[derive(Clone, Debug, Default)]
pub(in crate::joystick::hidapi) struct Step {
    pub(in crate::joystick::hidapi) head: String,
    pub(in crate::joystick::hidapi) inputs: Vec<String>,
    pub(in crate::joystick::hidapi) outputs: Vec<String>,
}

/// A line of a transcript.
#[derive(Clone, Debug)]
pub(in crate::joystick::hidapi) enum Entry {
    /// A `case` line, which sets up the steps up to `endcase`
    Case(Vec<String>),
    Step(Step),
}

/// Read a transcript.
pub(in crate::joystick::hidapi) fn parse_transcript(text: &str) -> Vec<Entry> {
    let mut entries = Vec::new();
    let mut step: Option<Step> = None;
    for line in text.lines().filter(|l| !l.starts_with('#')) {
        if step.is_none() {
            if let Some(case) = line.strip_prefix("case ") {
                entries.push(Entry::Case(
                    case.split_whitespace().map(str::to_owned).collect(),
                ));
                continue;
            }
            if line == "endcase" {
                continue;
            }
        }
        let current = step.get_or_insert_with(Step::default);
        if line == "end" {
            entries.push(Entry::Step(step.take().unwrap()));
        } else if line.starts_with("read ") || line.starts_with("reply ") {
            current.inputs.push(line.to_owned());
        } else if current.head.is_empty() {
            current.head = line.to_owned();
        } else {
            current.outputs.push(line.to_owned());
        }
    }
    entries
}

impl Step {
    /// Check what the Rust driver did against the step's outputs: the
    /// reports sent to the device (`hid`), and the events and other
    /// outputs, in order (`events`). The C errors and device names aren't
    /// compared.
    pub(in crate::joystick::hidapi) fn check(&self, hid: &[String], events: &[String]) {
        let is_hid = |l: &&String| {
            l.starts_with("feature ") || l.starts_with("getfeature ") || l.starts_with("write ")
        };
        let expected_hid: Vec<&String> = self.outputs.iter().filter(is_hid).collect();
        let expected_events: Vec<&String> = self
            .outputs
            .iter()
            .filter(|l| {
                !is_hid(l)
                    && !l.starts_with("error ")
                    && *l != "unsupported"
                    && !l.starts_with("name ")
            })
            .collect();
        assert_eq!(
            hid.iter().collect::<Vec<_>>(),
            expected_hid,
            "reports of {:?} {:?}",
            self.head,
            self.inputs
        );
        assert_eq!(
            events.iter().collect::<Vec<_>>(),
            expected_events,
            "events of {:?} {:?}",
            self.head,
            self.inputs
        );
    }
}

/// The `add` lines of an opened joystick.
pub(in crate::joystick::hidapi) fn describe_open(joystick: &JoystickData) -> Vec<String> {
    let mut lines = Vec::new();
    for sensor in &joystick.sensors {
        lines.push(format!(
            "add sensor {} {:08x}",
            sensor.sensor_type as i32,
            sensor.rate.to_bits()
        ));
    }
    for touchpad in &joystick.touchpads {
        lines.push(format!("add touchpad {}", touchpad.fingers.len()));
    }
    for capsense in &joystick.capsenses {
        lines.push(format!("add capsense {}", capsense.capsense_type as i32));
    }
    lines
}

/// The `result` line of a step.
pub(in crate::joystick::hidapi) fn result_line(ok: bool) -> String {
    format!("result {}", u8::from(ok))
}

fn controller(bluetooth: bool, product_id: u16) -> Arc<HidapiDevice> {
    test_device(&DeviceInfo {
        vendor_id: USB_VENDOR_VALVE,
        product_id,
        interface_number: 2,
        bus_type: if bluetooth {
            BusType::Bluetooth
        } else {
            BusType::Usb
        },
        ..DeviceInfo::default()
    })
}

const STEAM_TRANSCRIPT: &str = include_str!("tests/steam.txt");
const STEAM_WIN_TRANSCRIPT: &str = include_str!("tests/steam_win.txt");

/// The steps of the Windows transcript, by head.
fn windows_steps() -> HashMap<String, Step> {
    parse_transcript(STEAM_WIN_TRANSCRIPT)
        .into_iter()
        .filter_map(|entry| match entry {
            Entry::Step(step) => Some((step.head.clone(), step)),
            Entry::Case(_) => None,
        })
        .collect()
}

/// A BLE feature report read, with or without the second report ID.
fn check_ble_get_feature_report(step: &Step, duplicate_report_id: bool) {
    let fake = FakeHid::new(true);
    fake.queue(step);
    let mut buf = [0u8; FEATURE_REPORT_BUFFER_SIZE];
    let mut lines = Vec::new();
    match get_feature_report_with(&fake, &mut buf, duplicate_report_id) {
        Ok(n) => {
            lines.push(format!("result {n}"));
            lines.push(hex_line("buffer", &buf[..n + 1]));
        }
        Err(_) => lines.push("result -1".to_owned()),
    }
    step.check(&fake.take_log(), &lines);
}

/// Replays the C transcript.
#[test]
fn transcript() {
    let _l = crate::test_support::test_lock();
    let _lock = crate::joystick::lock_joysticks();
    PAIRING_CONTEXT.store(0, Ordering::Relaxed);

    let windows = windows_steps();

    let mut device = controller(false, 0x1102);
    let mut fake = FakeHid::new(false);
    let mut ctx = SteamContext::default();
    let mut joystick_open = None;
    let mut inits: HashMap<String, (Arc<HidapiDevice>, SteamContext)> = HashMap::new();
    let mut connection_state = JoystickConnectionState::Unknown;

    for entry in parse_transcript(STEAM_TRANSCRIPT) {
        let step = match entry {
            Entry::Case(case) => {
                let name = case[0].as_str();
                let bluetooth = case[1] == "1";
                ctx = SteamContext {
                    report_sensors: case[2] == "1",
                    ..SteamContext::default()
                };
                let product_id = match name {
                    "ble" => 0x1106,
                    "dongle" => 0x1142,
                    _ => 0x1102,
                };
                device = controller(bluetooth, product_id);
                fake = FakeHid::new(bluetooth);
                ctx.connected = name != "dongle";
                ctx.update_rate_in_us = if name == "ble" { 7500 } else { 9000 };
                ctx.assembler = PacketAssembler::new(bluetooth);
                joystick_open = (case[3] == "1").then_some(1);
                continue;
            }
            Entry::Step(step) => step,
        };
        let words: Vec<&str> = step.head.split_whitespace().collect();
        let name = words.get(1).copied().unwrap_or("");

        match words[0] {
            "update" => {
                fake.queue(&step);
                let (ok, mut events) = run(&device, |d| ctx.update(d, &fake, joystick_open));
                events.push(result_line(ok));
                step.check(&fake.take_log(), &events);
            }
            "init" => {
                if name == "dongle_disconnected" {
                    hints::set(HINT_JOYSTICK_HIDAPI_STEAM_PAIRING_ENABLED, "1").unwrap();
                }
                let init_device = controller(false, if name == "wired" { 0x1102 } else { 0x1142 });
                let mut init_ctx = SteamContext::default();
                let init_fake = FakeHid::new(false);
                init_fake.queue(&step);
                let (result, mut events) = run(&init_device, |d| init_ctx.init(d, &init_fake));
                events.push(result_line(result.is_ok()));
                step.check(&init_fake.take_log(), &events);
                assert_eq!(init_device.name(), "Steam Controller");
                if name == "wired" {
                    hints::reset(HINT_JOYSTICK_HIDAPI_STEAM_PAIRING_ENABLED);
                }
                inits.insert(name.to_owned(), (init_device, init_ctx));
            }
            "free" => {
                let (init_device, init_ctx) = inits.get_mut(name).unwrap();
                let init_fake = FakeHid::new(false);
                let ((), events) = run(init_device, |d| init_ctx.free(d, &init_fake));
                step.check(&init_fake.take_log(), &events);
            }
            "open" => {
                let step = windows
                    .get(&step.head)
                    .filter(|_| BLE_DUPLICATE_FEATURE_REPORT_ID)
                    .unwrap_or(&step);
                hints::set(hints::JOYSTICK_HIDAPI_STEAM_HOME_LED, "0.5").unwrap();
                let bluetooth = name == "ble";
                let product_id = match name {
                    "ble" => 0x1106,
                    "dongle" => 0x1142,
                    _ => 0x1102,
                };
                let open_device = controller(bluetooth, product_id);
                let mut open_ctx = SteamContext::default();
                let open_fake = FakeHid::new(bluetooth);
                open_fake.queue(step);
                let mut joystick = JoystickData::new(1);
                joystick.connection_state = connection_state;
                let (result, mut events) = run(&open_device, |d| {
                    open_ctx.open(d, &open_fake, &mut joystick)
                });
                events.extend(describe_open(&joystick));
                events.push(result_line(result.is_ok()));
                // (the harness prints the connection state, except after
                // the bad attributes)
                if name != "bad_attributes" {
                    events.push(format!("connection {}", joystick.connection_state as i32));
                }
                if result.is_ok() {
                    assert_eq!(joystick.nbuttons, 13);
                    assert_eq!(joystick.naxes, 6);
                    assert_eq!(joystick.nhats, 1);
                }
                connection_state = joystick.connection_state;
                step.check(&open_fake.take_log(), &events);
                hints::reset(hints::JOYSTICK_HIDAPI_STEAM_HOME_LED);
            }
            "homeled" => {
                let value = step.head.strip_prefix("homeled ").unwrap_or("");
                let led_fake = FakeHid::new(false);
                home_led_hint_changed(&led_fake, Some(value));
                step.check(&led_fake.take_log(), &[]);
            }
            "sensors" => {
                let sensors_fake = FakeHid::new(name == "1");
                let enabled = words[2] == "1";
                let mut sensors_ctx = SteamContext::default();
                sensors_ctx
                    .set_sensors_enabled(&sensors_fake, enabled)
                    .unwrap();
                assert_eq!(sensors_ctx.report_sensors, enabled);
                step.check(&sensors_fake.take_log(), &[]);
            }
            "effect" => {
                let effect_fake = FakeHid::new(name == "1");
                let data = hex_bytes(&words[2..].join(" "));
                let result = send_effect(&effect_fake, &data);
                step.check(&effect_fake.take_log(), &[result_line(result.is_ok())]);
                // Other sizes aren't sent
                assert!(send_effect(&effect_fake, &data[..64]).is_err());
                assert!(effect_fake.take_log().is_empty());
            }
            "close" => {
                let close_fake = FakeHid::new(name == "1");
                close_steam_controller(&close_fake);
                step.check(&close_fake.take_log(), &[]);
            }
            "getfeature" => {
                check_ble_get_feature_report(&step, false);
                check_ble_get_feature_report(&windows[&step.head], true);
            }
            other => panic!("unknown step {other}"),
        }
    }

    PAIRING_CONTEXT.store(0, Ordering::Relaxed);
    super::super::NUMJOYSTICKS.store(0, Ordering::Relaxed);
    super::super::CHANGE_COUNT.store(0, Ordering::Relaxed);
}

#[test]
fn supported_devices() {
    let driver = SteamDriver;
    let supported = |device: Option<&HidapiDevice>, product_id, interface_number| {
        driver.is_supported_device(
            device,
            "",
            GamepadType::Standard,
            USB_VENDOR_VALVE,
            product_id,
            0,
            interface_number,
            0,
            0,
            0,
        )
    };
    // Without a device, any interface might be one
    assert!(supported(None, 0x1102, 0));
    // The controller interface of a wired controller, the wireless
    // controller interfaces of the dongle, and any BLE interface
    let wired = controller(false, 0x1102);
    assert!(supported(Some(&wired), 0x1102, 2));
    assert!(!supported(Some(&wired), 0x1102, 1));
    let dongle = controller(false, 0x1142);
    assert!(supported(Some(&dongle), 0x1142, 1));
    assert!(supported(Some(&dongle), 0x1142, 4));
    assert!(!supported(Some(&dongle), 0x1142, 0));
    assert!(!supported(Some(&dongle), 0x1142, 5));
    let ble = controller(true, 0x1106);
    assert!(supported(Some(&ble), 0x1106, 0));
    // Other Valve devices aren't Steam Controllers
    assert!(!supported(None, 0x1205, 0));
}

#[test]
fn constants() {
    // The values of upstream's enums
    assert_eq!(
        [
            ID_CLEAR_DIGITAL_MAPPINGS,
            ID_GET_ATTRIBUTES_VALUES,
            ID_SET_DEFAULT_DIGITAL_MAPPINGS,
            ID_SET_SETTINGS_VALUES,
            ID_LOAD_DEFAULT_SETTINGS,
            ID_ENABLE_PAIRING,
            ID_DONGLE_COMMIT_DEVICE,
            ID_DONGLE_GET_WIRELESS_STATE,
            ID_TRIGGER_RUMBLE_CMD,
        ],
        [0x81, 0x83, 0x85, 0x87, 0x8e, 0xad, 0xb3, 0xb4, 0xeb]
    );
    assert_eq!(
        [
            SETTING_LEFT_TRACKPAD_MODE,
            SETTING_RIGHT_TRACKPAD_MODE,
            SETTING_LIZARD_MODE,
            SETTING_SMOOTH_ABSOLUTE_MOUSE,
            SETTING_LED_USER_BRIGHTNESS,
            SETTING_IMU_MODE,
            SETTING_WIRELESS_PACKET_VERSION,
            SETTING_LEFT_TRACKPAD_CLICK_PRESSURE,
            SETTING_RIGHT_TRACKPAD_CLICK_PRESSURE,
        ],
        [7, 8, 9, 24, 45, 48, 49, 52, 53]
    );
    assert_eq!(
        [
            ATTRIB_UNIQUE_ID,
            ATTRIB_PRODUCT_ID,
            ATTRIB_CAPABILITIES,
            ATTRIB_CONNECTION_INTERVAL_IN_US
        ],
        [0, 1, 2, 11]
    );
    assert_eq!((TRACKPAD_ABSOLUTE_MOUSE, TRACKPAD_NONE), (0, 7));
}

#[test]
fn state_packets() {
    // A ValveControllerStatePacket_t at its offsets
    let mut data = [0u8; 48];
    for (i, b) in data.iter_mut().enumerate() {
        *b = i as u8;
    }
    let packet = ValveControllerStatePacket::parse(&data);
    assert_eq!(packet.packet_num, 0x03020100);
    assert_eq!(packet.buttons, 0x0b0a090807060504);
    assert_eq!(
        (packet.trigger_left(), packet.trigger_right()),
        (0x07, 0x08)
    );
    assert_eq!(packet.left_pad_x, 0x0d0c);
    assert_eq!(packet.gyro_quat_z, 0x2b2a);
    let ble = ValveControllerBleStatePacket::parse(&data);
    assert_eq!(ble.gyro_data_type, 20);
    assert_eq!(ble.gyro, [0x1615, 0x1817, 0x1a19, 0x1c1b]);

    // The touch filter
    let jitter = 256.0 / 65536.0;
    assert_eq!(filter_touch(0.5 + jitter * 0.25, 0.5), 0.5);
    assert_eq!(
        filter_touch(0.5 + jitter * 0.75, 0.5),
        0.5 * 0.75 + (0.5 + jitter * 0.75) * 0.25
    );
    assert_eq!(
        filter_touch(0.5 + jitter * 1.5, 0.5),
        (0.5 + 0.5 + jitter * 1.5) * 0.5
    );
    assert_eq!(filter_touch(0.75, 0.5), 0.75);

    assert_eq!(remap_val_clamped(5.0, 1.0, 1.0, -1.0, 2.0), 2.0);
    assert_eq!(remap_val_clamped(0.0, 1.0, 1.0, -1.0, 2.0), -1.0);
    assert_eq!(
        remap_val_clamped(30000.0, 0.0, 26000.0, 0.0, 32767.0),
        32767.0
    );
}
