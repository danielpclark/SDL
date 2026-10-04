// Tests for the Nintendo Wii HIDAPI driver.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The transcripts in `tests/` come from running upstream's
//! SDL_hidapi_wii.c, with stubs for the SDL and HID functions it uses
//! (printing the events and reports it sends, replying from a queue and
//! keeping a clock that only moves on `SDL_Delay()`, as [`FakeWii`] does),
//! through scripted cases: the extension handshakes of each kind of
//! controller, opening, input reports, Motion Plus checks, rumble, player
//! LEDs and sensors. `wii_linux.txt` is the same harness built for Linux,
//! where the driver leaves the Motion Plus mode alone.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;

use super::super::steam::tests::{
    describe_open, hex_bytes, hex_line, parse_transcript, result_line, Entry, Step,
};
use super::super::tests::{run, test_device};
use super::*;
use crate::hidapi::DeviceInfo;

/// The transcript of this platform.
const TRANSCRIPT: &str = if cfg!(target_os = "linux") {
    include_str!("tests/wii_linux.txt")
} else {
    include_str!("tests/wii.txt")
};

/// A Wii Remote for the tests: it logs the reports sent to it, replies
/// from a queue and has its own clock, as the stubs of the transcripts.
struct FakeWii {
    /// The reports to read (`None` for a read error)
    reads: RefCell<VecDeque<Option<Vec<u8>>>>,
    log: RefCell<Vec<String>>,
    now: Cell<u64>,
}

impl FakeWii {
    fn new() -> FakeWii {
        FakeWii {
            reads: RefCell::new(VecDeque::new()),
            log: RefCell::new(Vec::new()),
            now: Cell::new(1000),
        }
    }

    /// Queue the reads of a step, after the left-overs of the last step
    /// are dropped.
    fn queue(&self, step: &Step) {
        let mut reads = self.reads.borrow_mut();
        reads.clear();
        for input in &step.inputs {
            if let Some(read) = input.strip_prefix("read ") {
                reads.push_back((read != "-1").then(|| hex_bytes(read)));
            }
        }
    }

    fn take_log(&self) -> Vec<String> {
        std::mem::take(&mut *self.log.borrow_mut())
    }
}

impl WiiLink for FakeWii {
    fn rumble_pending(&self) -> bool {
        false
    }
    fn read_timeout(&self, data: &mut [u8], _milliseconds: i32) -> Result<usize> {
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
    fn write(&self, data: &[u8]) -> Result<usize> {
        self.log.borrow_mut().push(hex_line("write", data));
        Ok(data.len())
    }
    fn send_rumble(&self, data: &[u8]) -> Result<usize> {
        self.log.borrow_mut().push(hex_line("rumble", data));
        Ok(data.len())
    }
    fn ticks(&self) -> u64 {
        self.now.get()
    }
    fn ticks_ns(&self) -> u64 {
        1_000_000_000
    }
    fn delay(&self, ms: u64) {
        self.now.set(self.now.get() + ms);
    }
}

/// Check what the Rust driver did against a step's outputs: the reports
/// it wrote and sent through the rumble thread (`hid`), and its events
/// and the harness' lines (`events`). The C errors aren't compared.
fn check(step: &Step, hid: &[String], events: &[String]) {
    let is_hid = |l: &&String| l.starts_with("write ") || l.starts_with("rumble ");
    let expected_hid: Vec<&String> = step.outputs.iter().filter(is_hid).collect();
    let expected_events: Vec<&String> = step
        .outputs
        .iter()
        .filter(|l| !is_hid(l) && !l.starts_with("error ") && *l != "unsupported")
        .collect();
    assert_eq!(
        hid.iter().collect::<Vec<_>>(),
        expected_hid,
        "reports of {:?} {:?}",
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

fn wii_remote() -> Arc<HidapiDevice> {
    test_device(&DeviceInfo {
        vendor_id: USB_VENDOR_NINTENDO,
        product_id: USB_PRODUCT_NINTENDO_WII_REMOTE,
        interface_number: -1,
        ..DeviceInfo::default()
    })
}

#[test]
fn transcript() {
    let _l = crate::test_support::test_lock();
    let _lock = crate::joystick::lock_joysticks();
    hints::reset(hints::JOYSTICK_HIDAPI_WII_PLAYER_LED);

    let fake = FakeWii::new();
    let mut device = wii_remote();
    let mut ctx = WiiContext::default();
    let mut joystick = JoystickData::new(1);
    let mut steps = 0;

    for entry in parse_transcript(TRANSCRIPT) {
        let step = match entry {
            Entry::Case(_) => {
                device = wii_remote();
                ctx = WiiContext::default();
                joystick = JoystickData::new(1);
                hints::reset(hints::JOYSTICK_HIDAPI_WII_PLAYER_LED);
                continue;
            }
            Entry::Step(step) => step,
        };
        steps += 1;
        fake.queue(&step);
        let words: Vec<&str> = step.head.split_whitespace().collect();
        let arg = |i: usize| words[i].parse::<i64>().unwrap();

        let events = match words[0] {
            "init" => {
                let (result, mut events) = run(&device, |d| ctx.init(d, &fake));
                events.push(format!(
                    "identity {} {}",
                    device.guid().0[15],
                    device.name()
                ));
                events.push(result_line(result.is_ok()));
                events
            }
            "open" => {
                joystick = JoystickData::new(1);
                ctx.open(&fake, &mut joystick);
                let mut events = describe_open(&joystick);
                events.push(format!(
                    "buttons {} axes {}",
                    joystick.nbuttons, joystick.naxes
                ));
                events
            }
            "update" => {
                fake.delay(arg(1) as u64);
                let (ok, mut events) = run(&device, |d| {
                    ctx.update(d, &fake, Some(JoystickRef::Opening(&mut joystick)))
                });
                events.push(format!("connection {}", joystick.connection_state as i32));
                events.push(result_line(ok));
                events
            }
            "rumble" => {
                ctx.rumble(&fake, arg(1) as u16, arg(2) as u16);
                Vec::new()
            }
            "sensors" => {
                ctx.set_sensors_enabled(&fake, arg(1) == 1);
                Vec::new()
            }
            "player" => {
                ctx.set_player_index(&fake, arg(1) as i32);
                Vec::new()
            }
            "hint" => {
                hints::set(hints::JOYSTICK_HIDAPI_WII_PLAYER_LED, words[1]).unwrap();
                ctx.hint_changes(&fake);
                Vec::new()
            }
            other => panic!("unknown step {other}"),
        };
        check(&step, &fake.take_log(), &events);
    }
    assert!(steps > 40);

    hints::reset(hints::JOYSTICK_HIDAPI_WII_PLAYER_LED);
    super::super::NUMJOYSTICKS.store(0, Ordering::Relaxed);
    super::super::CHANGE_COUNT.store(0, Ordering::Relaxed);
}

#[test]
fn power_levels() {
    assert_eq!(wii_power_percent(0), 5);
    assert_eq!(wii_power_percent(14), 20);
    assert_eq!(wii_power_percent(52), 70);
    assert_eq!(wii_power_percent(179), 100);
    // Charging (bit 3 clear) and plugged in (bit 2 clear)
    assert_eq!(
        wiiu_power_info(0x40),
        (JoystickConnectionState::Wired, PowerState::Charging, 100)
    );
    assert_eq!(
        wiiu_power_info(0x38),
        (JoystickConnectionState::Wired, PowerState::Charged, 70)
    );
    assert_eq!(
        wiiu_power_info(0x2C),
        (JoystickConnectionState::Wireless, PowerState::OnBattery, 40)
    );
    assert_eq!(
        wiiu_power_info(0x1C),
        (JoystickConnectionState::Wireless, PowerState::OnBattery, 10)
    );
    assert_eq!(
        wiiu_power_info(0x0C),
        (JoystickConnectionState::Wireless, PowerState::OnBattery, 3)
    );
}

#[test]
fn supported_devices() {
    let driver = WiiDriver;
    let supported = |vendor_id, product_id| {
        driver.is_supported_device(
            None,
            "",
            GamepadType::Standard,
            vendor_id,
            product_id,
            0,
            -1,
            0,
            0,
            0,
        )
    };
    assert!(supported(
        USB_VENDOR_NINTENDO,
        USB_PRODUCT_NINTENDO_WII_REMOTE
    ));
    assert!(supported(
        USB_VENDOR_NINTENDO,
        USB_PRODUCT_NINTENDO_WII_REMOTE2
    ));
    assert!(!supported(
        USB_VENDOR_NINTENDO,
        USB_PRODUCT_NINTENDO_SWITCH_PRO
    ));
    assert!(!supported(0x1234, USB_PRODUCT_NINTENDO_WII_REMOTE));
    assert_eq!(get_extension_type(0x2E2E), ExtensionControllerType::None);
    assert_eq!(get_extension_type(0x0101), ExtensionControllerType::Gamepad);
    assert_eq!(get_extension_type(0x0005), ExtensionControllerType::Unknown);
    // The Wii buttons follow the gamepad buttons
    assert_eq!(WII_BUTTON_A, 15);
    assert_eq!(WII_BUTTON_MAX, 26);
}

#[test]
fn disabled_by_default() {
    let _l = crate::test_support::test_lock();
    hints::reset(hints::JOYSTICK_HIDAPI_WII);
    assert!(!WiiDriver.is_enabled());
    hints::set(hints::JOYSTICK_HIDAPI_WII, "1").unwrap();
    assert!(WiiDriver.is_enabled());
    hints::reset(hints::JOYSTICK_HIDAPI_WII);
}
