// Tests for the Nintendo Switch 2 HIDAPI driver.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! `tests/switch2.txt` comes from running upstream's SDL_hidapi_switch2.c,
//! with stubs for the SDL, HID and libusb functions it uses (printing the
//! events and the bulk transfers, replying to the transfers in from a
//! queue as [`FakeBulk`] does, and failing the transfer of a `fail` line),
//! through scripted cases for each controller: the flash reads and
//! initialization, opening, input reports with the sensor timestamp
//! calibration, rumble, player LEDs and sensors. The libusb calls around
//! the transfers (claiming and releasing the interface) aren't compared.

use std::collections::VecDeque;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use super::super::steam::tests::{
    describe_open, hex_bytes, hex_line, parse_transcript, result_line, Entry, Step,
};
use super::super::tests::{run, test_device};
use super::*;
use crate::hidapi::{BusType, DeviceInfo};

const TRANSCRIPT: &str = include_str!("tests/switch2.txt");

#[derive(Default)]
struct BulkState {
    replies: VecDeque<Vec<u8>>,
    log: Vec<String>,
    count: usize,
    error_at: Option<usize>,
}

/// The bulk endpoints of a controller for the tests: they log the
/// transfers, reply from a queue and fail the transfer of a `fail` line,
/// as the stubs of the transcript.
#[derive(Clone, Default)]
struct FakeBulk(Arc<Mutex<BulkState>>);

impl FakeBulk {
    fn state(&self) -> std::sync::MutexGuard<'_, BulkState> {
        self.0.lock().unwrap()
    }

    /// Queue the replies of a step, after the left-overs of the last step
    /// are dropped.
    fn queue(&self, step: &Step) {
        let mut state = self.state();
        state.replies.clear();
        state.count = 0;
        state.error_at = None;
        for input in &step.inputs {
            if let Some(reply) = input.strip_prefix("reply ") {
                state.replies.push_back(hex_bytes(reply));
            }
        }
        for output in &step.outputs {
            if let Some(n) = output.strip_prefix("fail ") {
                state.error_at = Some(n.parse().unwrap());
            }
        }
    }

    fn take_log(&self) -> Vec<String> {
        std::mem::take(&mut self.state().log)
    }

    /// Whether this transfer fails.
    fn fails(state: &mut BulkState) -> bool {
        let n = state.count;
        state.count += 1;
        if state.error_at == Some(n) {
            state.log.push("bulkerror".to_owned());
            return true;
        }
        false
    }
}

impl BulkEndpoints for FakeBulk {
    fn transfer_out(&self, data: &[u8]) -> Result<usize> {
        let mut state = self.state();
        if FakeBulk::fails(&mut state) {
            return Err(Error::new("transfer error"));
        }
        state.log.push(hex_line("bulkout 1000", data));
        Ok(data.len())
    }

    fn transfer_in(&self, data: &mut [u8]) -> Result<usize> {
        let mut state = self.state();
        if FakeBulk::fails(&mut state) {
            return Err(Error::new("transfer error"));
        }
        state.log.push(format!("bulkin {} 100", data.len()));
        Ok(match state.replies.pop_front() {
            Some(reply) => {
                let size = reply.len().min(data.len());
                data[..size].copy_from_slice(&reply[..size]);
                size
            }
            None => 0,
        })
    }
}

/// Check what the Rust driver did against a step's outputs: the bulk
/// transfers and rumble reports (`hid`), and the events and the harness'
/// lines (`events`).
fn check(step: &Step, hid: &[String], events: &[String]) {
    let is_hid = |l: &&String| {
        l.starts_with("bulkout ")
            || l.starts_with("bulkin ")
            || *l == "bulkerror"
            || l.starts_with("rumble ")
    };
    let expected_hid: Vec<&String> = step.outputs.iter().filter(is_hid).collect();
    let expected_events: Vec<&String> = step
        .outputs
        .iter()
        .filter(|l| {
            !is_hid(l)
                && !l.is_empty()
                && !l.starts_with("error ")
                && !l.starts_with("fail ")
                && !l.starts_with("claim ")
                && !l.starts_with("release ")
        })
        .collect();
    assert_eq!(
        hid.iter().collect::<Vec<_>>(),
        expected_hid,
        "transfers of {:?} {:?}",
        step.head,
        step.inputs
    );
    assert_eq!(
        events.iter().collect::<Vec<_>>(),
        expected_events,
        "events of {:?} {:?}",
        step.head,
        step.inputs
    );
}

fn controller(product_id: u16, bluetooth: bool) -> Arc<HidapiDevice> {
    test_device(&DeviceInfo {
        vendor_id: USB_VENDOR_NINTENDO,
        product_id,
        interface_number: 0,
        product_string: Some("Test Controller".to_owned()),
        bus_type: if bluetooth {
            BusType::Bluetooth
        } else {
            BusType::Usb
        },
        ..DeviceInfo::default()
    })
}

fn axis_line(a: &AxisCalibration) -> String {
    format!(" {} {} {}", a.neutral, a.max, a.min)
}

#[test]
fn transcript() {
    let _l = crate::test_support::test_lock();
    let _lock = crate::joystick::lock_joysticks();
    hints::reset(hints::JOYSTICK_HIDAPI_SWITCH_PLAYER_LED);

    let mut device = controller(USB_PRODUCT_NINTENDO_SWITCH2_PRO, false);
    let mut initial_name = String::new();
    let mut ctx = Switch2Context::default();
    let bulk = FakeBulk::default();
    let mut has_parent = false;
    // (the harness' clock, which only moves for the rumble)
    let mut now = 1000;
    let mut rumble_log = Vec::new();
    let mut steps = 0;

    for entry in parse_transcript(TRANSCRIPT) {
        let step = match entry {
            Entry::Case(case) => {
                let product_id = u16::from_str_radix(&case[1], 16).unwrap();
                device = controller(product_id, case[2] == "1");
                initial_name = device.name();
                ctx = Switch2Context::default();
                has_parent = false;
                hints::reset(hints::JOYSTICK_HIDAPI_SWITCH_PLAYER_LED);
                continue;
            }
            Entry::Step(step) => step,
        };
        steps += 1;
        bulk.queue(&step);
        let words: Vec<&str> = step.head.split_whitespace().collect();
        let arg = |i: usize| words[i].parse::<i64>().unwrap();

        let events = match words[0] {
            "init" => {
                // (init_device() with the endpoints libusb would give)
                let (result, mut events) = run(&device, |d| {
                    if d.is_bluetooth() {
                        init_bluetooth()?;
                    } else {
                        ctx.init_usb(d, Box::new(bulk.clone()))?;
                    }
                    ctx.finish_init(d);
                    Ok::<(), Error>(())
                });
                events.push(format!(
                    "sticks{}{}{}{} {} {}",
                    axis_line(&ctx.left_stick.x),
                    axis_line(&ctx.left_stick.y),
                    axis_line(&ctx.right_stick.x),
                    axis_line(&ctx.right_stick.y),
                    ctx.left_trigger_zero,
                    ctx.right_trigger_zero
                ));
                events.push(format!(
                    "bias {:08x} {:08x} {:08x} {:08x} {:08x} {:08x}",
                    ctx.gyro_bias_x.to_bits(),
                    ctx.gyro_bias_y.to_bits(),
                    ctx.gyro_bias_z.to_bits(),
                    ctx.accel_bias_x.to_bits(),
                    ctx.accel_bias_y.to_bits(),
                    ctx.accel_bias_z.to_bits()
                ));
                let name = device.name();
                events.push(format!(
                    "identity {} {}",
                    device.serial().unwrap_or_else(|| "-".to_owned()),
                    if name == initial_name { "-" } else { &name }
                ));
                events.push(result_line(result.is_ok()));
                events
            }
            "open" => {
                has_parent = arg(1) == 1;
                hints::set(hints::JOYSTICK_HIDAPI_VERTICAL_JOY_CONS, words[2]).unwrap();
                let mut joystick = JoystickData::new(1);
                let ((), mut events) = run(&device, |d| ctx.open(d, &mut joystick, has_parent));
                hints::reset(hints::JOYSTICK_HIDAPI_VERTICAL_JOY_CONS);
                events.extend(describe_open(&joystick));
                events.push(format!(
                    "buttons {} axes {} hats {}",
                    joystick.nbuttons, joystick.naxes, joystick.nhats
                ));
                events
            }
            "state" => {
                let packets: Vec<Vec<u8>> = step
                    .inputs
                    .iter()
                    .filter_map(|l| l.strip_prefix("read ").map(hex_bytes))
                    .collect();
                let ((), mut events) = run(&device, |d| {
                    for packet in &packets {
                        ctx.handle_state_packet(d, 1, has_parent, packet);
                    }
                });
                events.push(format!(
                    "ready {} {} {} {:08x} {} {}",
                    u8::from(ctx.sensors_ready),
                    ctx.sample_count,
                    ctx.first_sensor_timestamp,
                    ctx.gyro_coeff.to_bits(),
                    u8::from(ctx.sensors_enabled),
                    ctx.sensor_ts_coeff
                ));
                events
            }
            "preset" => {
                ctx.sample_count = arg(1) as i32;
                ctx.first_sensor_timestamp = arg(2) as u64;
                Vec::new()
            }
            "free" => run(&device, |d| ctx.free_device(d)).1,
            "makeready" => {
                ctx.sensors_ready = true;
                Vec::new()
            }
            "sensors" => {
                let result = ctx.set_sensors_enabled(arg(1) == 1);
                vec![result_line(result.is_ok())]
            }
            "rumble" => {
                now += arg(3) as u64;
                run(&device, |d| {
                    ctx.rumble_joystick(d, 1, arg(1) as u16, arg(2) as u16)
                })
                .0
                .unwrap();
                // (update_rumble() at the harness' clock)
                if ctx.rumble_due(now) {
                    let report = ctx.rumble_report(device.product_id(), has_parent, now);
                    rumble_log.push(hex_line("rumble", &report));
                }
                Vec::new()
            }
            "player" => {
                run(&device, |d| {
                    ctx.set_device_player_index(d, 1, arg(1) as i32)
                })
                .1
            }
            "hint" => {
                hints::set(hints::JOYSTICK_HIDAPI_SWITCH_PLAYER_LED, words[1]).unwrap();
                run(&device, |d| ctx.hint_changes(d)).1
            }
            other => panic!("unknown step {other}"),
        };
        let mut hid = bulk.take_log();
        hid.append(&mut rumble_log);
        check(&step, &hid, &events);
    }
    assert!(steps > 80);

    hints::reset(hints::JOYSTICK_HIDAPI_SWITCH_PLAYER_LED);
    super::super::NUMJOYSTICKS.store(0, Ordering::Relaxed);
    super::super::CHANGE_COUNT.store(0, Ordering::Relaxed);
}

#[test]
fn stick_calibration() {
    let data = [0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0, 0x11];
    assert_eq!(
        parse_stick_calibration(&data),
        StickCalibration {
            x: AxisCalibration {
                neutral: 0x412,
                max: 0xa78,
                min: 0x0de,
            },
            y: AxisCalibration {
                neutral: 0x563,
                max: 0xbc9,
                min: 0x11f,
            },
        }
    );
    // Without a calibration the raw range is remapped
    let none = AxisCalibration::default();
    assert_eq!(map_joystick_axis_value(&none, 0.0, false), i16::MIN);
    assert_eq!(map_joystick_axis_value(&none, 2048.0, false), 0);
    assert_eq!(map_joystick_axis_value(&none, 0.0, true), i16::MAX);
    let calib = AxisCalibration {
        neutral: 2000,
        max: 1000,
        min: 500,
    };
    assert_eq!(map_joystick_axis_value(&calib, 3500.0, false), i16::MAX);
    assert_eq!(map_joystick_axis_value(&calib, 1750.0, false), -16383);
    assert_eq!(map_trigger_axis_value(32, 232.0), i16::MAX);
    assert_eq!(map_trigger_axis_value(32, 10.0), i16::MIN);
}

#[test]
fn disabled_without_libusb() {
    let _l = crate::test_support::test_lock();
    hints::set(hints::JOYSTICK_HIDAPI_SWITCH2, "1").unwrap();
    assert!(!Switch2Driver.is_enabled());
    hints::reset(hints::JOYSTICK_HIDAPI_SWITCH2);

    // There are no bulk endpoints to set the controllers up with
    let device = controller(USB_PRODUCT_NINTENDO_SWITCH2_PRO, false);
    let mut ctx = Switch2Context::default();
    let (result, events) = run(&device, |d| ctx.init_device(d));
    assert!(result.is_err());
    assert!(events.is_empty());

    let supported = |product_id| {
        Switch2Driver.is_supported_device(
            None,
            "",
            GamepadType::Standard,
            USB_VENDOR_NINTENDO,
            product_id,
            0,
            0,
            0,
            0,
            0,
        )
    };
    assert!(supported(USB_PRODUCT_NINTENDO_SWITCH2_PRO));
    assert!(supported(USB_PRODUCT_NINTENDO_SWITCH2_GAMECUBE_CONTROLLER));
    assert!(supported(USB_PRODUCT_NINTENDO_SWITCH2_JOYCON_LEFT));
    assert!(supported(USB_PRODUCT_NINTENDO_SWITCH2_JOYCON_RIGHT));
    assert!(!supported(USB_PRODUCT_NINTENDO_SWITCH2_JOYCON_PAIR));
    assert!(!supported(USB_PRODUCT_NINTENDO_SWITCH_PRO));
}
