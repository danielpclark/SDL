// Tests for the Logitech wheel HIDAPI driver.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The cases in `data.rs` come from running upstream's SDL_hidapi_lg4ff.c,
//! with stubs for the SDL and HID functions it uses (printing the events
//! and the reports it writes, and reading from a queue as [`FakeHid`]
//! does), on generated reports. Each case starts with the "init" and
//! "open" calls of the same name; its "range", "autocenter", "led" and
//! "close" calls come after its reports.

use std::sync::{Arc, Mutex};

use super::super::steam::tests::{hex_line, FakeHid, Step};
use super::super::tests::{run, test_device};
use super::super::{del_device, with_context, HidapiDevice, DEVICES, DRIVER_LG4FF};
use super::*;
use crate::hidapi::DeviceInfo;

mod data;

/// A [`FakeHid`] the effect thread of the HIDAPI haptic driver can write
/// to while the test reads it.
#[derive(Clone)]
struct SharedHid(Arc<Mutex<FakeHid>>);

impl SharedHid {
    fn hid(&self) -> std::sync::MutexGuard<'_, FakeHid> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl SteamHid for SharedHid {
    fn is_bluetooth(&self) -> bool {
        self.hid().is_bluetooth()
    }
    fn send_feature_report(&self, data: &[u8]) -> Result<usize> {
        self.hid().send_feature_report(data)
    }
    fn get_feature_report(&self, data: &mut [u8]) -> Result<usize> {
        self.hid().get_feature_report(data)
    }
    fn read(&self, data: &mut [u8]) -> Result<usize> {
        self.hid().read(data)
    }
    fn read_timeout(&self, data: &mut [u8], milliseconds: i32) -> Result<usize> {
        self.hid().read_timeout(data, milliseconds)
    }
    fn write(&self, data: &[u8]) -> Result<usize> {
        self.hid().write(data)
    }
}

/// The driver's context on a fake HID device: the functions of
/// [`Lg4ffContext`] on `hid` instead of the device's own.
struct FakeWheelContext {
    inner: Lg4ffContext,
    hid: SharedHid,
}

impl DriverContext for FakeWheelContext {
    fn init_device(&mut self, device: &mut DeviceCtx<'_>) -> Result<()> {
        // (HIDAPI_DriverLg4ff_InitDevice() without the HID device)
        device.set_joystick_type(JoystickType::Wheel);
        device.set_device_name(device_name(device.product_id()));
        self.inner.init(device, &self.hid)
    }

    fn update_device(&mut self, device: &mut DeviceCtx<'_>) -> bool {
        let Some(joystick) = device.open_joystick_id() else {
            return false;
        };
        self.inner.update(device, &self.hid, joystick)
    }

    fn open_joystick(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: &mut JoystickData,
    ) -> Result<()> {
        self.inner.open_joystick(device, joystick)
    }

    fn get_joystick_capabilities(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: JoystickID,
    ) -> JoystickCaps {
        self.inner.get_joystick_capabilities(device, joystick)
    }

    fn send_joystick_effect(
        &mut self,
        _device: &mut DeviceCtx<'_>,
        _joystick: JoystickID,
        data: &[u8],
    ) -> Result<()> {
        send_effect(&self.hid, data)
    }

    fn close_joystick(&mut self, device: &mut DeviceCtx<'_>, _joystick: JoystickID) {
        let _ = self.inner.set_led(device.product_id(), &self.hid, 0, 0, 0);
    }
}

/// A Logitech wheel on a fake HID device, connected to the HIDAPI joystick
/// driver as if it had been found (for the tests of the HIDAPI haptic
/// driver, which reaches it through its joystick); disconnected on drop.
/// Take the test lock, with the joystick subsystem initialized.
pub(crate) struct FakeWheel {
    device: Arc<HidapiDevice>,
    hid: SharedHid,
    /// The instance ID of the wheel's joystick.
    pub(crate) id: JoystickID,
}

impl FakeWheel {
    pub(crate) fn connect(vendor_id: u16, product_id: u16, version: u16) -> FakeWheel {
        let _lock = crate::joystick::lock_joysticks();

        let device = test_device(&DeviceInfo {
            path: Some(format!("fake-lg4ff-{vendor_id:04x}-{product_id:04x}")),
            vendor_id,
            product_id,
            release_number: version,
            ..DeviceInfo::default()
        });
        let hid = SharedHid(Arc::new(Mutex::new(FakeHid::new(false))));
        device.state().driver = Some(&DRIVER_LG4FF);
        *device.context.lock().unwrap() = Some(Box::new(FakeWheelContext {
            inner: Lg4ffContext::default(),
            hid: hid.clone(),
        }));
        DEVICES.lock().unwrap().push(device.clone());

        let result = with_context(&device, |context, dctx| context.init_device(dctx));
        assert!(matches!(result, Some(Ok(()))), "{result:?}");
        let id = device.joysticks()[0];
        FakeWheel { device, hid, id }
    }

    /// The reports written since the last call.
    pub(crate) fn take_writes(&self) -> Vec<String> {
        self.hid.hid().take_log()
    }
}

impl Drop for FakeWheel {
    fn drop(&mut self) {
        let _lock = crate::joystick::lock_joysticks();
        del_device(&self.device);
    }
}

fn call(name: &str) -> &'static [&'static str] {
    data::CALLS
        .iter()
        .find(|(n, _)| *n == name)
        .unwrap_or_else(|| panic!("no call {name}"))
        .1
}

/// The product, version and range of a case's wheel.
fn config(label: &str) -> (u16, u16, u16) {
    match label {
        "g29" => (USB_DEVICE_ID_LOGITECH_G29_WHEEL, 0x1350, 0),
        "g27" => (USB_DEVICE_ID_LOGITECH_G27_WHEEL, 0x1230, 0),
        "g25" => (USB_DEVICE_ID_LOGITECH_G25_WHEEL, 0x1200, 0),
        "dfgt" => (USB_DEVICE_ID_LOGITECH_DFGT_WHEEL, 0x1300, 0),
        "dfp" => (USB_DEVICE_ID_LOGITECH_DFP_WHEEL, 0x1000, 0),
        "dfp range 300" => (USB_DEVICE_ID_LOGITECH_DFP_WHEEL, 0x1000, 300),
        "dfp range 150" => (USB_DEVICE_ID_LOGITECH_DFP_WHEEL, 0x1000, 150),
        "dfex" => (USB_DEVICE_ID_LOGITECH_WHEEL, 0x1000, 0),
        "ffex" => (USB_DEVICE_ID_LOGITECH_WHEEL, 0x2100, 0),
        _ => panic!("unknown case {label}"),
    }
}

/// The reports written, and the other lines, of an expected output.
fn split(lines: &[&str]) -> (Vec<String>, Vec<String>) {
    let (writes, others): (Vec<&str>, Vec<&str>) =
        lines.iter().partition(|l| l.starts_with("write "));
    (
        writes.into_iter().map(str::to_owned).collect(),
        others.into_iter().map(str::to_owned).collect(),
    )
}

/// Check a call that only writes reports.
fn check_writes(name: &str, fake: &FakeHid, mut lines: Vec<String>) {
    let mut writes = fake.take_log();
    writes.append(&mut lines);
    let (expected_writes, mut expected_lines) = split(call(name));
    let mut expected = expected_writes;
    expected.append(&mut expected_lines);
    assert_eq!(writes, expected, "{name}");
}

#[test]
fn mode_switches() {
    let fake = FakeHid::new(false);
    for (name, expected) in data::CALLS
        .iter()
        .filter(|(n, _)| n.starts_with("supported "))
    {
        let words: Vec<u16> = name
            .split(' ')
            .skip(1)
            .map(|w| u16::from_str_radix(w, 16).unwrap())
            .collect();
        let result = is_supported_wheel(Some(&fake), USB_VENDOR_LOGITECH, words[0], words[1]);
        let mut lines = fake.take_log();
        lines.push(format!("result {}", u8::from(result)));
        assert_eq!(lines, *expected, "{name}");
    }
    assert!(!is_supported_wheel(
        None,
        0x046e,
        USB_DEVICE_ID_LOGITECH_G29_WHEEL,
        0
    ));
    assert!(Lg4ffDriver.is_supported_device(
        None,
        "",
        GamepadType::Unknown,
        USB_VENDOR_LOGITECH,
        USB_DEVICE_ID_LOGITECH_WHEEL,
        0x1350,
        0,
        0,
        0,
        0
    ));
}

#[test]
fn transcript() {
    let _l = crate::test_support::test_lock();
    let _lock = crate::joystick::lock_joysticks();

    for (label, reports) in data::CASES {
        let (product_id, version, range) = config(label);
        let device = test_device(&DeviceInfo {
            vendor_id: USB_VENDOR_LOGITECH,
            product_id,
            release_number: version,
            ..DeviceInfo::default()
        });
        let fake = FakeHid::new(false);

        // The init call (with the setup of the device the fake one can't do)
        let mut ctx = Lg4ffContext::default();
        let (result, mut lines) = run(&device, |d| {
            d.set_joystick_type(JoystickType::Wheel);
            d.set_device_name(device_name(product_id));
            ctx.init(d, &fake)
        });
        lines.push(format!("result {}", u8::from(result.is_ok())));
        lines.push(format!(
            "type {} ffex {} range {}",
            device.state().joystick_type as i32,
            u8::from(ctx.is_ffex),
            ctx.range
        ));
        let expected = call(&format!("init {label}"));
        assert_eq!(
            Some(device.name().as_str()),
            expected.iter().find_map(|l| l.strip_prefix("name ")),
            "init {label}"
        );
        let (expected_writes, expected_lines) = split(expected);
        assert_eq!(fake.take_log(), expected_writes, "init {label}");
        let expected_lines: Vec<&String> = expected_lines
            .iter()
            .filter(|l| !l.starts_with("name "))
            .collect();
        assert_eq!(
            lines.iter().collect::<Vec<_>>(),
            expected_lines,
            "init {label}"
        );

        // The open call
        let mut joystick = JoystickData::new(1);
        let (caps, _) = run(&device, |d| {
            ctx.open_joystick(d, &mut joystick).unwrap();
            ctx.get_joystick_capabilities(d, 1)
        });
        assert_eq!(
            [format!(
                "buttons {} axes {} hats {} caps {}",
                joystick.nbuttons, joystick.naxes, joystick.nhats, caps.0
            )],
            call(&format!("open {label}")),
            "open {label}"
        );

        if range != 0 {
            ctx.range = range;
            ctx.initialized = true;
        }

        for (n, (report, expected)) in reports.iter().enumerate() {
            fake.queue(&Step {
                inputs: vec![hex_line("read", report)],
                ..Step::default()
            });
            let (ok, events) = run(&device, |d| ctx.update(d, &fake, 1));
            assert!(ok);
            let (expected_writes, expected_events) = split(expected);
            assert_eq!(
                fake.take_log(),
                expected_writes,
                "{label} report {n} writes"
            );
            assert_eq!(events, expected_events, "{label} report {n}");
        }

        for range in [10, 40, 200, 201, 300, 900, 1000, 150, 199] {
            let result = ctx.set_range(product_id, &fake, range);
            let lines = vec![
                format!("result {}", u8::from(result.is_ok())),
                format!("range {}", ctx.range),
            ];
            check_writes(&format!("range {label} {range}"), &fake, lines);
        }
        for magnitude in [-5, 0, 1, 1000, 0xaaaa, 0xaaab, 50000, 65535, 70000] {
            let result = ctx.set_auto_center(&fake, magnitude);
            let lines = vec![format!("result {}", u8::from(result.is_ok()))];
            check_writes(&format!("autocenter {label} {magnitude}"), &fake, lines);
        }
        for [red, green, blue] in [
            [0, 0, 0],
            [1, 0, 0],
            [0, 52, 51],
            [10, 20, 102],
            [0, 0, 153],
            [204, 0, 0],
            [254, 0, 0],
            [0, 255, 0],
        ] {
            let result = ctx.set_led(product_id, &fake, red, green, blue);
            let lines = if result.is_err() {
                vec!["unsupported".to_owned()]
            } else {
                Vec::new()
            };
            check_writes(&format!("led {label} {red} {green} {blue}"), &fake, lines);
        }
        // (CloseJoystick turns the LEDs off)
        let result = ctx.set_led(product_id, &fake, 0, 0, 0);
        let lines = if result.is_err() {
            vec!["unsupported".to_owned()]
        } else {
            Vec::new()
        };
        check_writes(&format!("close {label}"), &fake, lines);
    }

    super::super::NUMJOYSTICKS.store(0, std::sync::atomic::Ordering::Relaxed);
    super::super::CHANGE_COUNT.store(0, std::sync::atomic::Ordering::Relaxed);
}

#[test]
fn dfp_x_axis() {
    assert_eq!(lg4ff_adjust_dfp_x_axis(1234, 900), 1234);
    assert_eq!(lg4ff_adjust_dfp_x_axis(1234, 200), 1234);
    assert_eq!(lg4ff_adjust_dfp_x_axis(0, 100), 0);
    assert_eq!(lg4ff_adjust_dfp_x_axis(16383, 100), 16383);
    assert_eq!(lg4ff_adjust_dfp_x_axis(8192 + 100, 450), 8192 + 200);
    assert_eq!(
        device_name(USB_DEVICE_ID_LOGITECH_WHEEL),
        "Driving Force EX"
    );
    assert_eq!(report_size(USB_DEVICE_ID_LOGITECH_G27_WHEEL), 11);
}
