// The program of uikit_hidapi.rs and uikit_hidapi_ble.rs: HIDAPI on UIKit.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! HIDAPI's iOS backend, without and with Bluetooth (the Steam
//! Controllers' hint unset or set: the backend's manager reads it once, so
//! each is a program of its own). The main function traces:
//!
//! * the hint, and CoreBluetooth's authorization of the app (`+[CBManager
//!   authorization]`);
//! * `SDL_hid_init()` (twice) and the backend's manager (upstream's
//!   `HIDBLEManager`, by its class name): its central manager and the
//!   central manager's state (waited for, with Bluetooth: the simulator
//!   has none), its devices, scans and pairings;
//! * the device change count, the enumerations (all devices, the Steam
//!   Controllers) and opening devices that aren't there;
//! * `SDL_hid_ble_scan()` started and stopped (more often than started);
//! * the app's resign-active and become-active notifications (posted),
//!   which drop the devices and start a 20 second scan, then its end;
//! * `SDL_hid_exit()`, and the gamepads, with the HIDAPI joystick driver
//!   off (iOS's default) and on;
//! * between the steps, what the backend logged (NSLog's lines, through
//!   Foundation's private `_NSSetLogCStringFunction()`).
//!
//! The expected traces are upstream's C SDL's, from the same program in C
//! run the same way on the same simulator (an iPhone 16 with iOS 18.5).

use std::ffi::{c_char, c_void, CString};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use sdl3::hidapi::{self, HidDevice};
use sdl3::hints;
use sdl3::init::{self, InitFlags};
use sdl3::joystick;

use super::uikit_support::objc::{self, Id};
use super::uikit_support::{check_trace, checked, elapsed, exit};

/// Upstream's trace without Bluetooth (the C program, on an iPhone 16
/// simulator with iOS 18.5).
const EXPECTED: &str = "\
app state: active
start: log: BLE: the log hook works
hint SDL_JOYSTICK_HIDAPI_STEAM: (null)
start: CBManager authorization: 3 (allowed always)
SDL_hid_init: 0
SDL_hid_init again: 0
init: HIDBLEManager: central=no devices=0 scans=0 pairs=0
init: log: (none)
SDL_hid_device_change_count: 2
SDL_hid_device_change_count again: the same
init: SDL_hid_enumerate(0x0000, 0x0000): (none)
init: SDL_hid_enumerate(0x28de, 0x1106): (none)
init: SDL_hid_enumerate(0x28de, 0x1303): (none)
SDL_hid_open_path(a UUID): NULL
SDL_hid_open_path(\"\"): NULL
SDL_hid_open(0x28de, 0x1106): NULL
open: HIDBLEManager: central=no devices=0 scans=0 pairs=0
SDL_hid_ble_scan(true)
scan: HIDBLEManager: central=no devices=0 scans=0 pairs=0
scan: log: (none)
SDL_hid_ble_scan(true)
scan twice: HIDBLEManager: central=no devices=0 scans=0 pairs=0
SDL_hid_ble_scan(false)
stop: HIDBLEManager: central=no devices=0 scans=0 pairs=0
SDL_hid_ble_scan(false)
stop twice: HIDBLEManager: central=no devices=0 scans=0 pairs=0
SDL_hid_ble_scan(false)
stop thrice: HIDBLEManager: central=no devices=0 scans=0 pairs=0
stop: log: (none)
posted UIApplicationWillResignActiveNotification
resign active: HIDBLEManager: central=no devices=0 scans=0 pairs=0
posted UIApplicationDidBecomeActiveNotification
become active: HIDBLEManager: central=no devices=0 scans=0 pairs=0
become active: log: (none)
SDL_hid_exit: 0
exit: HIDBLEManager: central=no devices=0 scans=0 pairs=0
SDL_hid_exit: 0
SDL_hid_exit: 0
exit twice: HIDBLEManager: central=no devices=0 scans=0 pairs=0
init gamepad: ok
gamepad: joysticks: Gamepad (0500a5d0ac05000004000000fa256d04)
gamepad: HIDBLEManager: central=no devices=0 scans=0 pairs=0
quit gamepad
init gamepad (SDL_JOYSTICK_HIDAPI=1): ok
HIDAPI on: joysticks: Gamepad (0500a5d0ac05000004000000fa256d04)
HIDAPI on: HIDBLEManager: central=no devices=0 scans=0 pairs=0
HIDAPI on: SDL_hid_enumerate(0x0000, 0x0000): (none)
quit gamepad
gamepad: log: (none)
end: CBManager authorization: 3 (allowed always)
done
";

/// Upstream's trace with Bluetooth.
const EXPECTED_BLE: &str = "\
app state: active
start: log: BLE: the log hook works
hint SDL_JOYSTICK_HIDAPI_STEAM: 1
start: CBManager authorization: 3 (allowed always)
SDL_hid_init: 0
SDL_hid_init again: 0
state update: HIDBLEManager: central=yes state=2 (unsupported) scanning=no devices=0 scans=0 pairs=0
state update: CBManager authorization: 3 (allowed always)
init: log: CoreBluetooth BLE hardware is unsupported on this platform
SDL_hid_device_change_count: 2
SDL_hid_device_change_count again: the same
init: SDL_hid_enumerate(0x0000, 0x0000): (none)
init: SDL_hid_enumerate(0x28de, 0x1106): (none)
init: SDL_hid_enumerate(0x28de, 0x1303): (none)
SDL_hid_open_path(a UUID): NULL
SDL_hid_open_path(\"\"): NULL
SDL_hid_open(0x28de, 0x1106): NULL
open: HIDBLEManager: central=yes state=2 (unsupported) scanning=no devices=0 scans=0 pairs=0
SDL_hid_ble_scan(true)
scan: HIDBLEManager: central=yes state=2 (unsupported) scanning=no devices=0 scans=1 pairs=0
scan: log: BLE: requesting scan for 0 seconds
SDL_hid_ble_scan(true)
scan twice: HIDBLEManager: central=yes state=2 (unsupported) scanning=no devices=0 scans=2 pairs=0
SDL_hid_ble_scan(false)
stop: HIDBLEManager: central=yes state=2 (unsupported) scanning=no devices=0 scans=1 pairs=0
SDL_hid_ble_scan(false)
stop twice: HIDBLEManager: central=yes state=2 (unsupported) scanning=no devices=0 scans=0 pairs=0
SDL_hid_ble_scan(false)
stop thrice: HIDBLEManager: central=yes state=2 (unsupported) scanning=no devices=0 scans=0 pairs=0
stop: log: BLE: requesting scan for 0 seconds, BLE: stopping scan, BLE: stopping scan, BLE: stopping scan
posted UIApplicationWillResignActiveNotification
resign active: HIDBLEManager: central=yes state=2 (unsupported) scanning=no devices=0 scans=0 pairs=0
posted UIApplicationDidBecomeActiveNotification
become active: HIDBLEManager: central=yes state=2 (unsupported) scanning=no devices=0 scans=1 pairs=0
become active: log: BLE: requesting scan for 20 seconds
20 s later: HIDBLEManager: central=yes state=2 (unsupported) scanning=no devices=0 scans=0 pairs=0
20 s later: log: BLE: stopping scan
SDL_hid_exit: 0
exit: HIDBLEManager: central=yes state=2 (unsupported) scanning=no devices=0 scans=0 pairs=0
SDL_hid_exit: 0
SDL_hid_exit: 0
exit twice: HIDBLEManager: central=yes state=2 (unsupported) scanning=no devices=0 scans=0 pairs=0
init gamepad: ok
gamepad: joysticks: Gamepad (0500a5d0ac05000004000000fa256d04)
gamepad: HIDBLEManager: central=yes state=2 (unsupported) scanning=no devices=0 scans=0 pairs=0
quit gamepad
init gamepad (SDL_JOYSTICK_HIDAPI=1): ok
HIDAPI on: joysticks: Gamepad (0500a5d0ac05000004000000fa256d04)
HIDAPI on: HIDBLEManager: central=yes state=2 (unsupported) scanning=no devices=0 scans=0 pairs=0
HIDAPI on: SDL_hid_enumerate(0x0000, 0x0000): (none)
quit gamepad
gamepad: log: (none)
end: CBManager authorization: 3 (allowed always)
done
";

static TRACE: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn t(line: impl Into<String>) {
    let line = line.into();
    println!("trace: {line}");
    TRACE.lock().unwrap().push(line);
}

fn yes_no(b: bool) -> &'static str {
    if b {
        "yes"
    } else {
        "no"
    }
}

/// "A, B, C" or "(none)".
fn list(items: &[String]) -> String {
    if items.is_empty() {
        "(none)".into()
    } else {
        items.join(", ")
    }
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFRunLoopDefaultMode: *const c_void;
    fn CFRunLoopRunInMode(mode: *const c_void, seconds: f64, return_after: u8) -> i32;
}

// (linked, as the C program links them)
#[link(name = "CoreBluetooth", kind = "framework")]
unsafe extern "C" {}

#[link(name = "UIKit", kind = "framework")]
unsafe extern "C" {
    static UIApplicationWillResignActiveNotification: Id;
    static UIApplicationDidBecomeActiveNotification: Id;
}

unsafe extern "C" {
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}

/// `RTLD_DEFAULT`
const RTLD_DEFAULT: *mut c_void = -2isize as *mut c_void;

/// Run the main run loop for `ms` milliseconds.
fn run_loop(ms: u64) {
    let end = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < end {
        // SAFETY: the main thread's run loop, in its default mode.
        unsafe { CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.01, 0) };
    }
}

/// NSLog's lines (from the log hook, on any thread).
static LOG_LINES: Mutex<Vec<String>> = Mutex::new(Vec::new());
static LOG_SEEN: Mutex<usize> = Mutex::new(0);

extern "C" fn log_hook(string: *const c_char, length: u32, _banner: i8) {
    // SAFETY: Foundation's bytes, of that length.
    let bytes = unsafe { std::slice::from_raw_parts(string as *const u8, length as usize) };
    let line = String::from_utf8_lossy(bytes).into_owned();
    eprintln!("uikit_hidapi: NSLog: {line}");
    let mut lines = LOG_LINES.lock().unwrap_or_else(|e| e.into_inner());
    if lines.len() < 64 {
        lines.push(line);
    }
}

/// Whether a logged line is one of the hidapi backend's.
fn hid_message(line: &str) -> bool {
    const PREFIXES: [&str; 8] = [
        "BLE",
        "CoreBluetooth",
        "HIDBLE",
        "connected peripheral",
        "Found ",
        "Failed to connect",
        "Disconnected",
        "hid_read_timeout",
    ];
    PREFIXES.iter().any(|p| line.starts_with(p))
}

/// The hidapi backend's NSLog lines since the last call.
fn logs(label: &str) {
    let lines = LOG_LINES.lock().unwrap_or_else(|e| e.into_inner());
    let mut seen = LOG_SEEN.lock().unwrap();
    let new: Vec<String> = lines[*seen..]
        .iter()
        .filter(|l| hid_message(l))
        .cloned()
        .collect();
    *seen = lines.len();
    t(format!("{label}: log: {}", list(&new)));
}

fn state_name(s: isize) -> &'static str {
    match s {
        0 => "unknown",
        1 => "resetting",
        2 => "unsupported",
        3 => "unauthorized",
        4 => "powered off",
        5 => "powered on",
        _ => "?",
    }
}

fn authorization_name(a: isize) -> &'static str {
    match a {
        0 => "not determined",
        1 => "restricted",
        2 => "denied",
        3 => "allowed always",
        _ => "?",
    }
}

/// `+[CBManager authorization]` (iOS 13.1).
fn authorization(label: &str) {
    let cls = objc::class("CBManager");
    // SAFETY: the class's method, when it has it.
    unsafe {
        let responds: bool = objc::send1(cls, "respondsToSelector:", objc::sel("authorization"));
        if responds {
            let a: isize = objc::send0(cls, "authorization");
            t(format!(
                "{label}: CBManager authorization: {a} ({})",
                authorization_name(a)
            ));
        }
    }
}

/// The backend's `HIDBLEManager`, by its class name.
fn manager() -> Id {
    let cls = objc::class("HIDBLEManager");
    if cls.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: the class's method.
    unsafe { objc::send0(cls, "sharedInstance") }
}

fn central_state(central: Id) -> isize {
    // SAFETY: a CBCentralManager's property.
    unsafe { objc::send0(central, "state") }
}

fn manager_state(label: &str) {
    let m = manager();
    if m.is_null() {
        t(format!("{label}: HIDBLEManager: none"));
        return;
    }
    // SAFETY: the manager's getters, the map table's count and the
    // central manager's properties.
    unsafe {
        let central: Id = objc::send0(m, "centralManager");
        let map: Id = objc::send0(m, "deviceMap");
        let devices: usize = objc::send0(map, "count");
        let scans: i32 = objc::send0(m, "nPendingScans");
        let pairs: i32 = objc::send0(m, "nPendingPairs");
        if !central.is_null() {
            let state = central_state(central);
            let scanning: bool = objc::send0(central, "isScanning");
            t(format!(
                "{label}: HIDBLEManager: central=yes state={state} ({}) scanning={} devices={devices} scans={scans} pairs={pairs}",
                state_name(state),
                yes_no(scanning)
            ));
        } else {
            t(format!(
                "{label}: HIDBLEManager: central=no devices={devices} scans={scans} pairs={pairs}"
            ));
        }
    }
}

fn enumerate(label: &str, vid: u16, pid: u16) {
    let devs: Vec<String> = hidapi::enumerate(vid, pid)
        .unwrap_or_default()
        .iter()
        .map(|d| {
            format!(
                "{:04x}/{:04x} {}",
                d.vendor_id,
                d.product_id,
                d.product_string.as_deref().unwrap_or("")
            )
        })
        .collect();
    t(format!(
        "{label}: SDL_hid_enumerate({vid:#06x}, {pid:#06x}): {}",
        list(&devs)
    ));
}

fn joysticks(label: &str) {
    let all: Vec<String> = joystick::joysticks()
        .into_iter()
        .map(|id| {
            format!(
                "{} ({})",
                joystick::joystick_name_for_id(id)
                    .ok()
                    .flatten()
                    .unwrap_or_default(),
                joystick::joystick_guid_for_id(id)
            )
        })
        .collect();
    t(format!("{label}: joysticks: {}", list(&all)));
}

fn post(name: Id, what: &str) {
    objc::post_notification_for(name, std::ptr::null_mut());
    t(format!("posted {what}"));
}

fn result_code(r: &sdl3::error::Result<()>) -> i32 {
    if r.is_ok() {
        0
    } else {
        -1
    }
}

fn opened(r: &sdl3::error::Result<HidDevice>) -> &'static str {
    if r.is_ok() {
        "opened"
    } else {
        "NULL"
    }
}

fn application_state() -> isize {
    // SAFETY: a UIApplication property.
    unsafe { objc::send0(objc::shared_application(), "applicationState") }
}

/// The program (`ble`: with the hint set). With Bluetooth and
/// `SDL3_HIDAPI_STEAM_CONTROLLER=1` (on a device), the check of a real
/// Steam Controller instead ([`steam_controller_check`]).
pub(super) fn main(ble: bool) {
    elapsed();
    // (SDL's ticks start here, as the C program's do with its first
    // SDL_GetTicksNS(): the device change count's detection interval
    // counts from them)
    sdl3::timer::ticks_ns();
    let device_check = ble && std::env::var("SDL3_HIDAPI_STEAM_CONTROLLER").as_deref() == Ok("1");
    let status = sdl3::app::run_app(move |_args| {
        let status = if device_check {
            checked(steam_controller_check)
        } else {
            checked(|| main_function(ble))
        };
        exit(status);
    });
    println!("uikit_hidapi: run_app returned {status}, which it never does");
    exit(1);
}

fn main_function(ble: bool) {
    println!(
        "uikit_hidapi: {:.3} s: main (BLE={})",
        elapsed(),
        u8::from(ble)
    );
    let name = CString::new("_NSSetLogCStringFunction").unwrap();
    // SAFETY: Foundation's function, when it has it, given a function of
    // its callback's signature.
    let hooked = unsafe {
        let set_log = dlsym(RTLD_DEFAULT, name.as_ptr());
        if !set_log.is_null() {
            let set_log: extern "C" fn(extern "C" fn(*const c_char, u32, i8)) =
                std::mem::transmute(set_log);
            set_log(log_hook);
        }
        !set_log.is_null()
    };
    println!("uikit_hidapi: NSLog hook: {}", yes_no(hooked));
    #[link(name = "Foundation", kind = "framework")]
    unsafe extern "C" {
        fn NSLog(format: Id, ...);
    }
    // SAFETY: a format without arguments.
    unsafe { NSLog(objc::ns_string("BLE: the log hook works")) };

    let end = Instant::now() + Duration::from_secs(30);
    while application_state() != 0 && Instant::now() < end {
        run_loop(10);
    }
    t(format!(
        "app state: {}",
        if application_state() == 0 {
            "active"
        } else {
            "not active"
        }
    ));
    logs("start");
    if ble {
        hints::set(hints::JOYSTICK_HIDAPI_STEAM, "1").unwrap();
    }
    t(format!(
        "hint SDL_JOYSTICK_HIDAPI_STEAM: {}",
        hints::get(hints::JOYSTICK_HIDAPI_STEAM)
            .as_deref()
            .unwrap_or("(null)")
    ));
    authorization("start");

    t(format!("SDL_hid_init: {}", result_code(&hidapi::init())));
    t(format!(
        "SDL_hid_init again: {}",
        result_code(&hidapi::init())
    ));
    if !ble {
        manager_state("init");
    } else {
        // (the central manager's state comes from Bluetooth's queue)
        let end = Instant::now() + Duration::from_secs(5);
        // SAFETY: the manager's getter.
        let central: Id = unsafe { objc::send0(manager(), "centralManager") };
        while Instant::now() < end && central_state(central) == 0 {
            run_loop(10);
        }
        run_loop(300);
        manager_state("state update");
        authorization("state update");
    }
    logs("init");

    let count = hidapi::device_change_count();
    t(format!("SDL_hid_device_change_count: {count}"));
    t(format!(
        "SDL_hid_device_change_count again: {}",
        if hidapi::device_change_count() == count {
            "the same"
        } else {
            "changed"
        }
    ));
    enumerate("init", 0, 0);
    enumerate("init", 0x28de, 0x1106);
    enumerate("init", 0x28de, 0x1303);

    let dev = HidDevice::open_path("00000000-0000-0000-0000-000000000000");
    t(format!("SDL_hid_open_path(a UUID): {}", opened(&dev)));
    drop(dev);
    let dev = HidDevice::open_path("");
    t(format!("SDL_hid_open_path(\"\"): {}", opened(&dev)));
    drop(dev);
    let dev = HidDevice::open(0x28de, 0x1106, None);
    t(format!("SDL_hid_open(0x28de, 0x1106): {}", opened(&dev)));
    drop(dev);
    manager_state("open");

    hidapi::ble_scan(true);
    t("SDL_hid_ble_scan(true)");
    run_loop(200);
    manager_state("scan");
    logs("scan");
    hidapi::ble_scan(true);
    t("SDL_hid_ble_scan(true)");
    run_loop(200);
    manager_state("scan twice");
    hidapi::ble_scan(false);
    t("SDL_hid_ble_scan(false)");
    run_loop(200);
    manager_state("stop");
    hidapi::ble_scan(false);
    t("SDL_hid_ble_scan(false)");
    run_loop(200);
    manager_state("stop twice");
    hidapi::ble_scan(false);
    t("SDL_hid_ble_scan(false)");
    run_loop(200);
    manager_state("stop thrice");
    logs("stop");

    // SAFETY: UIKit's constants.
    let (resign, become_active) = unsafe {
        (
            UIApplicationWillResignActiveNotification,
            UIApplicationDidBecomeActiveNotification,
        )
    };
    post(resign, "UIApplicationWillResignActiveNotification");
    run_loop(200);
    manager_state("resign active");
    post(become_active, "UIApplicationDidBecomeActiveNotification");
    run_loop(200);
    manager_state("become active");
    logs("become active");
    if ble {
        // the 20 s scan's end
        run_loop(21000);
        manager_state("20 s later");
        logs("20 s later");
    }

    t(format!("SDL_hid_exit: {}", result_code(&hidapi::exit())));
    manager_state("exit");
    t(format!("SDL_hid_exit: {}", result_code(&hidapi::exit())));
    t(format!("SDL_hid_exit: {}", result_code(&hidapi::exit())));
    manager_state("exit twice");

    let r = init::init(InitFlags::GAMEPAD);
    t(format!(
        "init gamepad: {}",
        r.as_ref()
            .map_or_else(|e| e.message().to_string(), |()| "ok".into())
    ));
    run_loop(500);
    sdl3::events::pump();
    joysticks("gamepad");
    manager_state("gamepad");
    init::quit_subsystem(InitFlags::GAMEPAD);
    t("quit gamepad");

    hints::set(hints::JOYSTICK_HIDAPI, "1").unwrap();
    let r = init::init(InitFlags::GAMEPAD);
    t(format!(
        "init gamepad (SDL_JOYSTICK_HIDAPI=1): {}",
        r.as_ref()
            .map_or_else(|e| e.message().to_string(), |()| "ok".into())
    ));
    run_loop(500);
    sdl3::events::pump();
    joysticks("HIDAPI on");
    manager_state("HIDAPI on");
    enumerate("HIDAPI on", 0, 0);
    init::quit_subsystem(InitFlags::GAMEPAD);
    hints::reset(hints::JOYSTICK_HIDAPI);
    t("quit gamepad");
    logs("gamepad");
    authorization("end");

    init::quit();
    t("done");

    check_trace(
        &TRACE.lock().unwrap(),
        if ble { EXPECTED_BLE } else { EXPECTED },
    );
}

/// Pump the events and run the main run loop until `done` (or `seconds`
/// pass): whether it was.
fn wait_for(seconds: u64, mut done: impl FnMut() -> bool) -> bool {
    let end = Instant::now() + Duration::from_secs(seconds);
    while Instant::now() < end {
        run_loop(20);
        sdl3::events::pump();
        while sdl3::events::poll().is_some() {}
        if done() {
            return true;
        }
    }
    false
}

/// The hardware check of a Steam Controller over Bluetooth LE, on an iOS
/// or tvOS device (`docs/HARDWARE_TESTING.md`): it waits for the
/// controller to pair and connect (two minutes), lists the HID devices,
/// opens the controller as a gamepad and prints what SDL says of it, asks
/// for a button press and a stick movement (half a minute each), and
/// rumbles it.
fn steam_controller_check() {
    use sdl3::joystick::gamepad::{Gamepad, GamepadAxis, GamepadButton};

    println!(
        "uikit_hidapi: put the Steam Controller in Bluetooth pairing mode \
         (hold Y, then press Steam until it blinks), and allow Bluetooth and \
         the pairing request"
    );
    hints::set(hints::JOYSTICK_HIDAPI_STEAM, "1").unwrap();
    hints::set(hints::JOYSTICK_HIDAPI, "1").unwrap();
    init::init(InitFlags::GAMEPAD).expect("init gamepad");
    let mut found = None;
    let paired = wait_for(120, || {
        found = joystick::joysticks()
            .into_iter()
            .find(|&id| joystick::joystick_vendor_for_id(id) == 0x28de);
        found.is_some()
    });
    authorization("check");
    manager_state("check");
    assert!(paired, "no Steam Controller in two minutes");
    let id = found.unwrap();
    for d in hidapi::enumerate(0x28de, 0).unwrap() {
        println!(
            "uikit_hidapi: HID device {:04x}/{:04x} {:?} {:?} {:?} path {:?}",
            d.vendor_id, d.product_id, d.manufacturer_string, d.product_string, d.bus_type, d.path
        );
    }
    let gp = Gamepad::open(id).expect("open the controller");
    println!(
        "uikit_hidapi: {:?} guid={} vendor={:#06x} product={:#06x} type={:?} path={:?}",
        gp.name(),
        joystick::joystick_guid_for_id(id),
        joystick::joystick_vendor_for_id(id),
        joystick::joystick_product_for_id(id),
        gp.gamepad_type(),
        joystick::joystick_path_for_id(id)
    );
    println!("uikit_hidapi: press and release the A button");
    assert!(
        wait_for(30, || gp.button(GamepadButton::South)),
        "no A press"
    );
    assert!(
        wait_for(30, || !gp.button(GamepadButton::South)),
        "no A release"
    );
    println!("uikit_hidapi: move the left stick (or pad) fully left or right");
    assert!(
        wait_for(30, || gp.axis(GamepadAxis::LeftX).unsigned_abs() > 30000),
        "no full left stick movement"
    );
    println!("uikit_hidapi: rumble: {:?}", gp.rumble(0x4000, 0x8000, 300));
    wait_for(1, || false);
    drop(gp);
    init::quit();
    println!("uikit_hidapi: the Steam Controller check passed");
}
