// Tests of HIDAPI's Android backend in an app.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! An app (`harness = false`), run by `tools/android_run.py`, whose main
//! function traces HIDAPI's Android backend (upstream's hid.cpp) and the
//! HID device manager under it (HIDDeviceManager.java), with the HID
//! library alone (`SDL_hid_*()`):
//!
//! * the library started with `SDL_HINT_JOYSTICK_HIDAPI_STEAM` set, once
//!   the runner has granted `BLUETOOTH_CONNECT` (which the manifest asks
//!   for): the manager's USB and Bluetooth parts started (its log, the
//!   `hidapi` tag's lines of the device's log, which the runner gives), and
//!   no devices: the emulator has no USB devices nor Steam Controllers, and
//!   the runner's DualSense (a uhid device, plugged in before the launch)
//!   isn't a USB device, which the manager doesn't see;
//! * devices the manager tells hid.cpp of, through its natives, called
//!   through JNI on the activity's manager (`SDLActivity.mHIDDeviceManager`)
//!   as its Java calls them: a USB HID interface, a Steam Controller over
//!   Bluetooth and another USB interface, of no real device. Their
//!   enumeration (all, and by vendor and product), their opening (a path
//!   there isn't; the manager's `openDevice()` of a device it hasn't, which
//!   disconnects it; a USB permission's handshake, pending and then
//!   granted, which opens the device without the manager), their input
//!   reports (queued, the 16 latest kept; the Steam Controller's with its
//!   report ID first; read at once, and with a timeout, one arriving from
//!   another thread while it waits), their strings and information, a path
//!   opened twice, then the calls that go to the manager, which hasn't the
//!   devices: a write, a feature report, the last close, each disconnecting
//!   the device; a device disconnected while it's open;
//! * the library stopped and started again: the manager isn't initialized
//!   again.
//!
//! What a real device does (the USB permission's dialog, transfers, a
//! Steam Controller's GATT) isn't on the emulator: with
//! `SDL3_HIDAPI_DEVICE=1`, on a device with a controller, the program runs
//! a hardware check instead (see "Android device" in
//! docs/HARDWARE_TESTING.md).
//!
//! The expected trace is upstream's C SDL's, from the same program in C
//! (built with upstream's android-project, Gradle and CMake, run by the
//! runner's `--run-apk`) on the same emulator (API 35, Google APIs,
//! x86-64). Elsewhere this program does nothing.

#[cfg(not(target_os = "android"))]
fn main() {}

/// The runner's words: run as an app, with the DualSense plugged in and
/// the `BLUETOOTH_CONNECT` permission in the manifest.
#[cfg(target_os = "android")]
#[used]
#[link_section = ".sdl3_runner"]
static RUNNER: [u8; 52] = *b"app devices=dualsense permissions=BLUETOOTH_CONNECT\0";

#[cfg(target_os = "android")]
mod android_support;

#[cfg(target_os = "android")]
fn main() -> std::process::ExitCode {
    android_support::start();
    let status = android_support::checked(app::main);
    android_support::finish(status)
}

#[cfg(target_os = "android")]
mod app {
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    use sdl3::hidapi::{self, DeviceInfo, HidDevice};
    use sdl3::hints;

    use super::android_support::jni::{self, JNIEnv, Jobject, Jvalue};
    use super::android_support::{check_trace, request};

    /// Upstream's trace (the C program, on the emulator: API 35, Google
    /// APIs, x86-64).
    const EXPECTED: &str = "\
init: ok
init: change count nonzero: yes
init: log: V/hidapi: HIDDeviceRegisterCallback()
init: log: V/hidapi: initialize(true, false)
init: log: V/hidapi: initialize(false, true)
init: log: D/hidapi: Initializing Bluetooth
init: enumerate 0000/0000: 0 devices
connected: enumerate 0000/0000: 3 devices
connected: device: path=/dev/bus/usb/001/099/1209/2/1 vid=1209 pid=0002 serial= release=0 manufacturer=Maker product=Fake Stick usage=0/0 interface=1 class=3/1/2 bus=1
connected: device: path=SteamController.00:11:22:33:44:55 vid=28de pid=1106 serial=12345 release=0 manufacturer=Valve Corporation product=Steam Controller usage=0/0 interface=0 class=0/0/0 bus=2
connected: device: path=/dev/bus/usb/001/099/1209/1/0 vid=1209 pid=0001 serial=SERIAL1 release=100 manufacturer=Maker product=Fake Pad usage=0/0 interface=0 class=3/0/0 bus=1
connected: enumerate 28de/0000: 1 devices
connected: device: path=SteamController.00:11:22:33:44:55 vid=28de pid=1106 serial=12345 release=0 manufacturer=Valve Corporation product=Steam Controller usage=0/0 interface=0 class=0/0/0 bus=2
connected: enumerate 1209/0002: 1 devices
connected: device: path=/dev/bus/usb/001/099/1209/2/1 vid=1209 pid=0002 serial= release=0 manufacturer=Maker product=Fake Stick usage=0/0 interface=1 class=3/1/2 bus=1
connected: enumerate 1234/0000: 0 devices
open nope: error: Couldn't find device with path nope
open by ids: fails
open usb: error: Device open failed
open usb: log: V/hidapi: HIDDeviceConnected() id=1000 VID/PID = 1209/0001, interface 0
open usb: log: V/hidapi: HIDDeviceConnected() id=1001 VID/PID = 28de/1106, interface 0
open usb: log: V/hidapi: HIDDeviceConnected() id=1002 VID/PID = 1209/0002, interface 1
open usb: log: V/hidapi: hid_open_path( nope )
open usb: log: V/hidapi: hid_open_path( /dev/bus/usb/001/099/1209/1/0 )
open usb: log: V/hidapi: openDevice deviceID=1000
open usb: log: V/hidapi: No device for id: 1000
open usb: log: V/hidapi: Available devices: []
open usb: log: V/hidapi: HIDDeviceDisconnected() id=1000
open usb: log: V/hidapi: Device open failed
open usb: enumerate 0000/0000: 2 devices
open usb: device: path=/dev/bus/usb/001/099/1209/2/1 vid=1209 pid=0002 serial= release=0 manufacturer=Maker product=Fake Stick usage=0/0 interface=1 class=3/1/2 bus=1
open usb: device: path=SteamController.00:11:22:33:44:55 vid=28de pid=1106 serial=12345 release=0 manufacturer=Valve Corporation product=Steam Controller usage=0/0 interface=0 class=0/0/0 bus=2
open steam pending: error: Waiting for permission
open steam pending: enumerate 0000/0000: 2 devices
open steam pending: device: path=/dev/bus/usb/001/099/1209/2/1 vid=1209 pid=0002 serial= release=0 manufacturer=Maker product=Fake Stick usage=0/0 interface=1 class=3/1/2 bus=1
open steam pending: device: path=SteamController.00:11:22:33:44:55 vid=28de pid=1106 serial=12345 release=0 manufacturer=Valve Corporation product=Steam Controller usage=0/0 interface=0 class=0/0/0 bus=2
open steam: ok
open steam: log: V/hidapi: HIDDeviceOpenPending() id=1001
open steam: log: V/hidapi: hid_open_path( SteamController.00:11:22:33:44:55 )
open steam: log: V/hidapi: HIDDeviceOpenResult() id=1001, result=true
open steam: log: V/hidapi: Device open succeeded
open steam: log: V/hidapi: hid_open_path( SteamController.00:11:22:33:44:55 )
open steam: log: D/hidapi: Creating device 1001 ((pointer)), refCount = 1
steam: read: 0 (none)
steam: read: 4 03 01 02 03
steam: read: 0 (none)
steam: read: 0 (none)
steam: queued: 16 first: 03 04 08 last: 03 13 26
steam wait: read: 3 03 07 07
steam: manufacturer=Valve Corporation product=Steam Controller serial=12345 indexed=-1
steam: info: path=SteamController.00:11:22:33:44:55 vid=28de pid=1106 serial=12345 release=0 manufacturer=Valve Corporation product=Steam Controller usage=0/0 interface=0 class=0/0/0 bus=2
steam: nonblocking: -1
steam: descriptor: -1
open steam again: ok
steam after a close: read: 2 03 09
open steam again: log: V/hidapi: hid_open_path( SteamController.00:11:22:33:44:55 )
open steam again: log: D/hidapi: Incrementing device 1001 ((pointer)), refCount = 2
open steam again: log: V/hidapi: hid_close id=1001
open steam again: log: D/hidapi: Decrementing device 1001 ((pointer)), refCount = 1
steam: write: -1
steam write: log: V/hidapi: No device for id: 1001
steam write: log: V/hidapi: Available devices: []
steam write: log: V/hidapi: HIDDeviceDisconnected() id=1001
steam write: log: V/hidapi: closeDevice deviceID=1001
steam write: log: V/hidapi: No device for id: 1001
steam write: log: V/hidapi: Available devices: []
steam write: log: V/hidapi: HIDDeviceDisconnected() id=1001
steam write: enumerate 0000/0000: 1 devices
steam write: device: path=/dev/bus/usb/001/099/1209/2/1 vid=1209 pid=0002 serial= release=0 manufacturer=Maker product=Fake Stick usage=0/0 interface=1 class=3/1/2 bus=1
steam write: read: -1 (none)
steam write: manufacturer=-1 product=-1 serial=-1 indexed=-1
steam write: info: none
steam closed: log: V/hidapi: controller was disconnected
steam closed: log: V/hidapi: hid_close id=1001
steam closed: log: D/hidapi: Decrementing device 1001 ((pointer)), refCount = 0
steam closed: log: D/hidapi: Deleted device (pointer)
open stick: ok
stick: feature report: -1
stick feature report: log: V/hidapi: HIDDeviceOpenPending() id=1002
stick feature report: log: V/hidapi: HIDDeviceOpenResult() id=1002, result=true
stick feature report: log: V/hidapi: Device open succeeded
stick feature report: log: V/hidapi: hid_open_path( /dev/bus/usb/001/099/1209/2/1 )
stick feature report: log: D/hidapi: Creating device 1002 ((pointer)), refCount = 1
stick feature report: log: V/hidapi: hid_get_feature_report id=1002 length=16
stick feature report: log: V/hidapi: No device for id: 1002
stick feature report: log: V/hidapi: Available devices: []
stick feature report: log: V/hidapi: HIDDeviceDisconnected() id=1002
stick feature report: log: V/hidapi: closeDevice deviceID=1002
stick feature report: log: V/hidapi: No device for id: 1002
stick feature report: log: V/hidapi: Available devices: []
stick feature report: log: V/hidapi: HIDDeviceDisconnected() id=1002
stick feature report: log: V/hidapi: ReadReport failed
stick feature report: enumerate 0000/0000: 0 devices
open pad: ok
open pad: enumerate 0000/0000: 1 devices
open pad: device: path=/dev/bus/usb/001/099/1209/1/0 vid=1209 pid=0001 serial=SERIAL1 release=100 manufacturer=Maker product=Fake Pad usage=0/0 interface=0 class=3/0/0 bus=1
pad closed: log: V/hidapi: hid_close id=1002
pad closed: log: D/hidapi: Decrementing device 1002 ((pointer)), refCount = 0
pad closed: log: D/hidapi: Deleted device (pointer)
pad closed: log: V/hidapi: HIDDeviceConnected() id=1003 VID/PID = 1209/0001, interface 0
pad closed: log: V/hidapi: HIDDeviceOpenPending() id=1003
pad closed: log: V/hidapi: HIDDeviceOpenResult() id=1003, result=true
pad closed: log: V/hidapi: Device open succeeded
pad closed: log: V/hidapi: hid_open_path( /dev/bus/usb/001/099/1209/1/0 )
pad closed: log: D/hidapi: Creating device 1003 ((pointer)), refCount = 1
pad closed: log: V/hidapi: hid_close id=1003
pad closed: log: D/hidapi: Decrementing device 1003 ((pointer)), refCount = 0
pad closed: log: V/hidapi: closeDevice deviceID=1003
pad closed: log: V/hidapi: No device for id: 1003
pad closed: log: V/hidapi: Available devices: []
pad closed: log: V/hidapi: HIDDeviceDisconnected() id=1003
pad closed: log: V/hidapi: closeDevice deviceID=1003
pad closed: log: V/hidapi: No device for id: 1003
pad closed: log: V/hidapi: Available devices: []
pad closed: log: V/hidapi: HIDDeviceDisconnected() id=1003
pad closed: log: D/hidapi: Deleted device (pointer)
pad closed: enumerate 0000/0000: 0 devices
open pad again: ok
pad disconnected: enumerate 0000/0000: 0 devices
pad disconnected: read: -1 (none)
pad disconnected: log: V/hidapi: HIDDeviceConnected() id=1004 VID/PID = 1209/0001, interface 0
pad disconnected: log: V/hidapi: HIDDeviceOpenPending() id=1004
pad disconnected: log: V/hidapi: HIDDeviceOpenResult() id=1004, result=true
pad disconnected: log: V/hidapi: Device open succeeded
pad disconnected: log: V/hidapi: hid_open_path( /dev/bus/usb/001/099/1209/1/0 )
pad disconnected: log: D/hidapi: Creating device 1004 ((pointer)), refCount = 1
pad disconnected: log: V/hidapi: HIDDeviceDisconnected() id=1004
pad disconnected: log: V/hidapi: closeDevice deviceID=1004
pad disconnected: log: V/hidapi: No device for id: 1004
pad disconnected: log: V/hidapi: Available devices: []
pad disconnected: log: V/hidapi: HIDDeviceDisconnected() id=1004
pad disconnected: log: V/hidapi: controller was disconnected
exit: 0
init again: 0
init again: log: V/hidapi: hid_close id=1004
init again: log: D/hidapi: Decrementing device 1004 ((pointer)), refCount = 0
init again: log: D/hidapi: Deleted device (pointer)
init again: enumerate 0000/0000: 0 devices
main returns
";

    static TRACE: Mutex<Vec<String>> = Mutex::new(Vec::new());

    fn t(line: impl Into<String>) {
        let line = line.into();
        println!("trace: {line}");
        TRACE.lock().unwrap().push(line);
    }

    /// A count's result, or -1 for an error (upstream's).
    fn n(r: &sdl3::Result<usize>) -> String {
        match r {
            Ok(n) => n.to_string(),
            Err(_) => "-1".to_string(),
        }
    }

    fn opened(r: &sdl3::Result<HidDevice>) -> String {
        match r {
            Ok(_) => "ok".to_string(),
            Err(e) => format!("error: {}", e.message()),
        }
    }

    fn hex(bytes: &[u8]) -> String {
        if bytes.is_empty() {
            return "(none)".to_string();
        }
        bytes
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn info_line(dev: &DeviceInfo) -> String {
        format!(
            "path={} vid={:04x} pid={:04x} serial={} release={:x} manufacturer={} product={} usage={:x}/{:x} interface={} class={}/{}/{} bus={}",
            dev.path.as_deref().unwrap_or("(null)"),
            dev.vendor_id,
            dev.product_id,
            dev.serial_number.as_deref().unwrap_or("(null)"),
            dev.release_number,
            dev.manufacturer_string.as_deref().unwrap_or("(null)"),
            dev.product_string.as_deref().unwrap_or("(null)"),
            dev.usage_page,
            dev.usage,
            dev.interface_number,
            dev.interface_class,
            dev.interface_subclass,
            dev.interface_protocol,
            dev.bus_type as i32
        )
    }

    fn enumerate(step: &str, vendor_id: u16, product_id: u16) {
        let devs = hidapi::enumerate(vendor_id, product_id).unwrap_or_default();
        t(format!(
            "{step}: enumerate {vendor_id:04x}/{product_id:04x}: {} devices",
            devs.len()
        ));
        for dev in &devs {
            t(format!("{step}: device: {}", info_line(dev)));
        }
    }

    /// A line with its pointers (`0x` and hexadecimal digits) as
    /// `(pointer)`.
    fn mask_pointers(line: &str) -> String {
        let mut out = String::new();
        let mut rest = line;
        while let Some(i) = rest.find("0x") {
            out.push_str(&rest[..i]);
            let digits = rest[i + 2..]
                .bytes()
                .take_while(u8::is_ascii_hexdigit)
                .count();
            if digits == 0 {
                out.push_str("0x");
            } else {
                out.push_str("(pointer)");
            }
            rest = &rest[i + 2 + digits..];
        }
        out.push_str(rest);
        out
    }

    /// The lines of the device's log with the `hidapi` tag (the manager's
    /// and, in a debug build, hid.cpp's verbose ones, their pointers
    /// masked) since the last call (the runner gives them all).
    fn manager_log(step: &str) {
        static SEEN: Mutex<usize> = Mutex::new(0);
        let lines = request("logcat hidapi");
        let mut seen = SEEN.lock().unwrap();
        let new = lines.get(*seen..).unwrap_or_default();
        if new.is_empty() {
            t(format!("{step}: log: (none)"));
        }
        for line in new {
            t(format!("{step}: log: {}", mask_pointers(line)));
        }
        *seen = lines.len();
    }

    /// The JNI environment of this thread, and the activity's HID device
    /// manager (`SDLActivity.mHIDDeviceManager`).
    struct Manager {
        env: JNIEnv,
        manager: Jobject,
    }

    impl Manager {
        fn get() -> Manager {
            let env = sdl3::android::jni_env()
                .expect("a JNI environment")
                .as_ptr();
            let activity = sdl3::android::activity().expect("the activity").as_ptr();
            let class = jni::object_class(env, activity);
            let manager = jni::get_static_object_field(
                env,
                class,
                "mHIDDeviceManager",
                "Lorg/libsdl/app/HIDDeviceManager;",
            );
            assert!(!manager.is_null(), "no HID device manager");
            for r in [class, activity] {
                jni::delete_local_ref(env, r);
            }
            Manager { env, manager }
        }

        /// `HIDDeviceConnected()`.
        #[allow(clippy::too_many_arguments)]
        fn connected(
            &self,
            id: i32,
            path: &str,
            vendor_id: i32,
            product_id: i32,
            serial: &str,
            release: i32,
            manufacturer: &str,
            product: &str,
            interface: i32,
            class: [i32; 3],
            bluetooth: bool,
            report_id: i32,
        ) {
            let env = self.env;
            let strings = [path, serial, manufacturer, product].map(|s| jni::new_string(env, s));
            jni::call_void(
                env,
                self.manager,
                "HIDDeviceConnected",
                "(ILjava/lang/String;IILjava/lang/String;ILjava/lang/String;Ljava/lang/String;IIIIZI)V",
                &[
                    Jvalue { i: id },
                    Jvalue { l: strings[0] },
                    Jvalue { i: vendor_id },
                    Jvalue { i: product_id },
                    Jvalue { l: strings[1] },
                    Jvalue { i: release },
                    Jvalue { l: strings[2] },
                    Jvalue { l: strings[3] },
                    Jvalue { i: interface },
                    Jvalue { i: class[0] },
                    Jvalue { i: class[1] },
                    Jvalue { i: class[2] },
                    Jvalue { z: bluetooth as u8 },
                    Jvalue { i: report_id },
                ],
            );
            for s in strings {
                jni::delete_local_ref(env, s);
            }
        }

        /// `HIDDeviceOpenPending()`.
        fn open_pending(&self, id: i32) {
            jni::call_void(
                self.env,
                self.manager,
                "HIDDeviceOpenPending",
                "(I)V",
                &[Jvalue { i: id }],
            );
        }

        /// `HIDDeviceOpenResult()`.
        fn open_result(&self, id: i32, opened: bool) {
            jni::call_void(
                self.env,
                self.manager,
                "HIDDeviceOpenResult",
                "(IZ)V",
                &[Jvalue { i: id }, Jvalue { z: opened as u8 }],
            );
        }

        /// `HIDDeviceDisconnected()`.
        fn disconnected(&self, id: i32) {
            jni::call_void(
                self.env,
                self.manager,
                "HIDDeviceDisconnected",
                "(I)V",
                &[Jvalue { i: id }],
            );
        }

        /// `HIDDeviceInputReport()`.
        fn input_report(&self, id: i32, report: &[u8]) {
            input_report(self.env, self.manager, id, report);
        }
    }

    impl Drop for Manager {
        fn drop(&mut self) {
            jni::delete_local_ref(self.env, self.manager);
        }
    }

    fn input_report(env: JNIEnv, manager: Jobject, id: i32, report: &[u8]) {
        let array = jni::new_byte_array(env, report);
        jni::call_void(
            env,
            manager,
            "HIDDeviceInputReport",
            "(I[B)V",
            &[Jvalue { i: id }, Jvalue { l: array }],
        );
        jni::delete_local_ref(env, array);
    }

    /// A read of a report (64 bytes at most), traced.
    fn read(step: &str, dev: &HidDevice, timeout: Option<Duration>) {
        let mut data = [0u8; 64];
        let r = match timeout {
            Some(timeout) => dev.read_timeout(&mut data, Some(timeout)),
            None => dev.read(&mut data),
        };
        let bytes = match &r {
            Ok(n) => hex(&data[..*n]),
            Err(_) => "(none)".to_string(),
        };
        t(format!("{step}: read: {} {bytes}", n(&r)));
    }

    fn strings(step: &str, dev: &HidDevice) {
        let s = |r: sdl3::Result<String>| match r {
            Ok(s) => s,
            Err(_) => "-1".to_string(),
        };
        t(format!(
            "{step}: manufacturer={} product={} serial={} indexed={}",
            s(dev.manufacturer_string()),
            s(dev.product_string()),
            s(dev.serial_number_string()),
            s(dev.indexed_string(1))
        ));
        match dev.device_info() {
            Ok(info) => t(format!("{step}: info: {}", info_line(&info))),
            Err(_) => t(format!("{step}: info: none")),
        }
    }

    const USB_PATH: &str = "/dev/bus/usb/001/099/1209/1/0";
    const STEAM_PATH: &str = "SteamController.00:11:22:33:44:55";
    const USB_PATH_2: &str = "/dev/bus/usb/001/099/1209/2/1";

    /// Pump the events until `done` (or `seconds` pass): whether it was.
    fn wait_for(seconds: u64, mut done: impl FnMut() -> bool) -> bool {
        let end = Instant::now() + Duration::from_secs(seconds);
        while Instant::now() < end {
            sdl3::events::pump();
            while sdl3::events::poll().is_some() {}
            if done() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    /// The hardware check of a real HID controller on a device
    /// (`SDL3_HIDAPI_DEVICE=1`, docs/HARDWARE_TESTING.md's "Android
    /// device"): a USB controller (a HID interface, or an Xbox
    /// controller's) or a Steam Controller paired over Bluetooth LE, found
    /// by HIDAPI (two minutes, for the permissions: the USB device's
    /// dialog, Nearby devices), the HID devices listed, the controller
    /// opened as a gamepad through the HIDAPI joystick driver and what SDL
    /// says of it printed, then a button press and a stick movement asked
    /// for (half a minute each) and a rumble.
    fn device_check() {
        use sdl3::events::JoystickID;
        use sdl3::init::{self, InitFlags};
        use sdl3::joystick;
        use sdl3::joystick::gamepad::{Gamepad, GamepadAxis, GamepadButton};

        println!(
            "android_hidapi: plug in a USB controller (allow the app to access it), \
             or turn on a paired Steam Controller (allow Nearby devices)"
        );
        hints::set(hints::JOYSTICK_HIDAPI_STEAM, "1").unwrap();
        hints::set(hints::JOYSTICK_HIDAPI, "1").unwrap();
        init::init(InitFlags::GAMEPAD).expect("init gamepad");
        let hid_paths = || -> Vec<String> {
            hidapi::enumerate(0, 0)
                .unwrap_or_default()
                .into_iter()
                .filter_map(|d| d.path)
                .collect()
        };
        let mut found: Option<JoystickID> = None;
        let came = wait_for(120, || {
            let paths = hid_paths();
            found = joystick::joysticks().into_iter().find(|&id| {
                joystick::joystick_path_for_id(id)
                    .ok()
                    .is_some_and(|path| paths.contains(&path))
            });
            found.is_some()
        });
        for d in hidapi::enumerate(0, 0).unwrap_or_default() {
            println!("android_hidapi: HID device {}", info_line(&d));
        }
        assert!(came, "no HIDAPI controller in two minutes");
        let id = found.unwrap();
        let gp = Gamepad::open(id).expect("open the controller");
        println!(
            "android_hidapi: {:?} guid={} vendor={:#06x} product={:#06x} type={:?} path={:?}",
            gp.name(),
            joystick::joystick_guid_for_id(id),
            joystick::joystick_vendor_for_id(id),
            joystick::joystick_product_for_id(id),
            gp.gamepad_type(),
            joystick::joystick_path_for_id(id)
        );
        println!("android_hidapi: press and release the A (bottom face) button");
        assert!(
            wait_for(30, || gp.button(GamepadButton::South)),
            "no A press"
        );
        assert!(
            wait_for(30, || !gp.button(GamepadButton::South)),
            "no A release"
        );
        println!("android_hidapi: move the left stick fully left or right");
        assert!(
            wait_for(30, || gp.axis(GamepadAxis::LeftX).unsigned_abs() > 30000),
            "no full left stick movement"
        );
        println!(
            "android_hidapi: rumble: {:?}",
            gp.rumble(0x4000, 0x8000, 300)
        );
        wait_for(1, || false);
        drop(gp);
        init::quit();
        println!("android_hidapi: the controller check passed");
    }

    pub fn main() {
        if std::env::var("SDL3_HIDAPI_DEVICE").as_deref() == Ok("1") {
            device_check();
            return;
        }

        // The library, Bluetooth too
        request("permission grant BLUETOOTH_CONNECT");
        let _ = hints::set(hints::JOYSTICK_HIDAPI_STEAM, "1");
        let r = hidapi::init();
        t(format!(
            "init: {}",
            match &r {
                Ok(()) => "ok".to_string(),
                Err(e) => format!("error: {}", e.message()),
            }
        ));
        t(format!(
            "init: change count nonzero: {}",
            if hidapi::device_change_count() != 0 {
                "yes"
            } else {
                "no"
            }
        ));
        manager_log("init");
        enumerate("init", 0, 0);

        // Devices the manager tells of
        let manager = Manager::get();
        manager.connected(
            1000,
            USB_PATH,
            0x1209,
            0x0001,
            "SERIAL1",
            0x0100,
            "Maker",
            "Fake Pad",
            0,
            [3, 0, 0],
            false,
            0,
        );
        manager.connected(
            1001,
            STEAM_PATH,
            0x28de,
            0x1106,
            "12345",
            0,
            "Valve Corporation",
            "Steam Controller",
            0,
            [0, 0, 0],
            true,
            3,
        );
        manager.connected(
            1002,
            USB_PATH_2,
            0x1209,
            0x0002,
            "",
            0,
            "Maker",
            "Fake Stick",
            1,
            [3, 1, 2],
            false,
            0,
        );
        enumerate("connected", 0, 0);
        enumerate("connected", 0x28de, 0);
        enumerate("connected", 0x1209, 0x0002);
        enumerate("connected", 0x1234, 0);

        // Opening
        t(format!(
            "open nope: {}",
            opened(&HidDevice::open_path("nope"))
        ));
        t(format!(
            "open by ids: {}",
            if HidDevice::open(0x1209, 0x0001, None).is_ok() {
                "ok"
            } else {
                "fails"
            }
        ));
        t(format!(
            "open usb: {}",
            opened(&HidDevice::open_path(USB_PATH))
        ));
        manager_log("open usb");
        enumerate("open usb", 0, 0);
        manager.open_pending(1001);
        t(format!(
            "open steam pending: {}",
            opened(&HidDevice::open_path(STEAM_PATH))
        ));
        enumerate("open steam pending", 0, 0);
        manager.open_result(1001, true);
        let steam = HidDevice::open_path(STEAM_PATH);
        t(format!("open steam: {}", opened(&steam)));
        let steam = steam.unwrap();
        manager_log("open steam");

        // Input reports
        read("steam", &steam, None);
        manager.input_report(1001, &[1, 2, 3]);
        read("steam", &steam, None);
        read("steam", &steam, None);
        read("steam", &steam, Some(Duration::from_millis(100)));
        for i in 0..20u8 {
            manager.input_report(1001, &[i, i.wrapping_mul(2)]);
        }
        let mut count = 0;
        let mut first = String::new();
        let mut last = String::new();
        loop {
            let mut data = [0u8; 64];
            match steam.read(&mut data) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if count == 0 {
                        first = hex(&data[..n]);
                    }
                    last = hex(&data[..n]);
                    count += 1;
                }
            }
        }
        t(format!(
            "steam: queued: {count} first: {first} last: {last}"
        ));
        {
            // One while waiting, from a thread of its own (attached to the
            // VM by SDL's JNI environment)
            let manager_ref = jni::new_global_ref(manager.env, manager.manager) as usize;
            let sender = std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(100));
                let env = sdl3::android::jni_env()
                    .expect("a JNI environment")
                    .as_ptr();
                input_report(env, manager_ref as Jobject, 1001, &[7, 7]);
            });
            read("steam wait", &steam, Some(Duration::from_millis(5000)));
            sender.join().unwrap();
            jni::delete_global_ref(manager.env, manager_ref as Jobject);
        }
        strings("steam", &steam);
        t(format!(
            "steam: nonblocking: {}",
            if steam.set_nonblocking(true).is_ok() {
                "0"
            } else {
                "-1"
            }
        ));
        let mut descriptor = [0u8; 64];
        t(format!(
            "steam: descriptor: {}",
            n(&steam.report_descriptor(&mut descriptor))
        ));

        // The path opened twice
        let again = HidDevice::open_path(STEAM_PATH);
        t(format!("open steam again: {}", opened(&again)));
        drop(again);
        manager.input_report(1001, &[9]);
        read("steam after a close", &steam, None);
        manager_log("open steam again");

        // A write: the manager disconnects the device
        t(format!("steam: write: {}", n(&steam.write(&[0, 1, 2]))));
        manager_log("steam write");
        enumerate("steam write", 0, 0);
        read("steam write", &steam, None);
        strings("steam write", &steam);
        drop(steam);
        manager_log("steam closed");

        // A feature report: the same
        manager.open_pending(1002);
        manager.open_result(1002, true);
        let stick = HidDevice::open_path(USB_PATH_2);
        t(format!("open stick: {}", opened(&stick)));
        let stick = stick.unwrap();
        let mut feature = [0u8; 16];
        t(format!(
            "stick: feature report: {}",
            n(&stick.get_feature_report(&mut feature))
        ));
        manager_log("stick feature report");
        enumerate("stick feature report", 0, 0);
        drop(stick);

        // The last close: the manager closes it, and disconnects it
        manager.connected(
            1003,
            USB_PATH,
            0x1209,
            0x0001,
            "SERIAL1",
            0x0100,
            "Maker",
            "Fake Pad",
            0,
            [3, 0, 0],
            false,
            0,
        );
        manager.open_pending(1003);
        manager.open_result(1003, true);
        let pad = HidDevice::open_path(USB_PATH);
        t(format!("open pad: {}", opened(&pad)));
        enumerate("open pad", 0, 0);
        drop(pad);
        manager_log("pad closed");
        enumerate("pad closed", 0, 0);

        // Disconnected while open
        manager.connected(
            1004,
            USB_PATH,
            0x1209,
            0x0001,
            "SERIAL1",
            0x0100,
            "Maker",
            "Fake Pad",
            0,
            [3, 0, 0],
            false,
            0,
        );
        manager.open_pending(1004);
        manager.open_result(1004, true);
        let pad = HidDevice::open_path(USB_PATH);
        t(format!("open pad again: {}", opened(&pad)));
        let pad = pad.unwrap();
        manager.input_report(1004, &[5, 6]);
        manager.disconnected(1004);
        enumerate("pad disconnected", 0, 0);
        read("pad disconnected", &pad, None);
        manager_log("pad disconnected");
        drop(pad);
        drop(manager);

        // The library again
        t(format!(
            "exit: {}",
            if hidapi::exit().is_ok() { "0" } else { "-1" }
        ));
        t(format!(
            "init again: {}",
            if hidapi::init().is_ok() { "0" } else { "-1" }
        ));
        manager_log("init again");
        enumerate("init again", 0, 0);
        let _ = hidapi::exit();

        t("main returns");
        let trace = TRACE.lock().unwrap().clone();
        check_trace(&trace, EXPECTED);
    }
}
