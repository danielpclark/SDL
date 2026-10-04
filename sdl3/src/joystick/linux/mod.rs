// Rust translation of src/joystick/linux/SDL_sysjoystick.c and
// SDL_sysjoystick_c.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Linux joystick driver: evdev devices (`/dev/input/event*`), or the
//! classic joystick interface (`/dev/input/js*`) with
//! [`hints::JOYSTICK_LINUX_CLASSIC`]. Devices are found through libudev
//! when it is available and SDL isn't sandboxed, otherwise by watching
//! `/dev/input` with inotify, or by polling it. Rumble uses the kernel's
//! force feedback interface; gyroscopes and accelerometers that the kernel
//! exposes as a separate evdev node are matched to their joystick by their
//! unique identifier.
//!
//! This is the configuration upstream builds without HIDAPI (which isn't
//! translated yet): devices HIDAPI would handle are not skipped for it.

use std::cell::RefCell;
use std::ffi::{c_int, c_ulong, CString};
use std::os::fd::RawFd;

use super::gamepad::{GamepadMapping, MappingKind};
use super::{
    assert_joysticks_locked, create_joystick_guid, create_joystick_name, joystick_guid_info,
    joystick_handled_by_another_driver, lock_joysticks, private_joystick_added,
    private_joystick_removed, send_joystick_axis, send_joystick_ball, send_joystick_button,
    send_joystick_hat, send_joystick_sensor, should_ignore_joystick, Joystick, JoystickData,
    JoystickDriver, HAT_CENTERED, HAT_DOWN, HAT_LEFT, HAT_LEFTDOWN, HAT_LEFTUP, HAT_RIGHT,
    HAT_RIGHTDOWN, HAT_RIGHTUP, HAT_UP, JOYSTICK_AXIS_MAX, JOYSTICK_AXIS_MIN,
    PROP_JOYSTICK_CAP_RUMBLE_BOOLEAN,
};
use crate::core::linux::evdev::get_event_timestamp;
use crate::core::linux::evdev_capabilities::{
    guess_device_class, nbits, test_bit, AbsBits, DeviceClass, EvBits, KeyBits, PropBits, RelBits,
};
use crate::core::linux::input::*;
use crate::core::linux::udev::{self, UdevDeviceEvent};
use crate::error::{Error, Result};
use crate::events::JoystickID;
use crate::guid::Guid;
use crate::hints;
use crate::sensor::SensorType;
use crate::thread::ReentrantMutex;

#[cfg(test)]
mod tests;

/// `SDL_STANDARD_GRAVITY`
const STANDARD_GRAVITY: f32 = 9.80665;

/// How joysticks are found. Translation of `EnumerationMethod`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum EnumerationMethod {
    Unset,
    Libudev,
    Fallback,
}

/// An available joystick. Translation of `SDL_joylist_item`.
struct JoylistItem {
    device_instance: JoystickID,
    /// "/dev/input/event2" or whatever
    path: String,
    vendor: u16,
    // (the product ID is never read)
    /// "SideWinder 3D Pro" or whatever
    name: String,
    /// "xpad" or whatever
    driver: Option<String>,
    guid: Guid,
    devnum: libc::dev_t,
    steam_virtual_gamepad_slot: i32,
    /// Whether an open joystick uses this device (`item->hwdata`); that
    /// joystick has this item's instance ID.
    hwdata: bool,

    checked_mapping: bool,
    mapping: Option<GamepadMapping>,
}

/// An available gamepad sensor. Translation of `SDL_sensorlist_item`.
struct SensorlistItem {
    /// "/dev/input/event2" or whatever
    path: String,
    devnum: libc::dev_t,
    /// The open joystick reading this sensor (`item_sensor->hwdata`)
    hwdata: Option<JoystickID>,
}

/// Translation of `struct axis_correct`.
#[derive(Clone, Copy, Debug, Default)]
struct AxisCorrect {
    use_deadzones: bool,

    // Deadzone coefficients
    coef: [i32; 3],

    // Raw coordinate scale
    minimum: i32,
    maximum: i32,
    scale: f32,
}

/// Translation of `struct hat_axis_correct`.
#[derive(Clone, Copy, Debug, Default)]
struct HatAxisCorrect {
    use_deadzones: bool,
    minimum: [i32; 2],
    maximum: [i32; 2],
}

/// The counts of an open joystick that `ConfigJoystick()` fills in
/// (`joystick->naxes` and so on).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Counts {
    naxes: usize,
    nbuttons: usize,
    nhats: usize,
    nballs: usize,
}

/// The private structure used to keep track of a joystick.
/// Translation of `struct joystick_hwdata`.
struct HwData {
    /// The joystick this belongs to (0 for the one
    /// `LINUX_JoystickGetGamepadMapping()` opens temporarily)
    instance_id: JoystickID,
    fd: RawFd,
    // linux driver creates a separate device for gyro/accelerometer
    fd_sensor: RawFd,
    /// Whether the joylist item is still there (`hwdata->item`); it has
    /// this joystick's instance ID.
    item: bool,
    /// The vendor of the joylist item (`hwdata->item->vendor`, kept for
    /// when the item has gone).
    item_vendor: u16,
    /// The path of the sensor item, while it is there
    /// (`hwdata->item_sensor`).
    item_sensor: Option<String>,
    guid: Guid,
    /// Used in haptic subsystem
    fname: String,

    ff_rumble: bool,
    ff_sine: bool,
    effect: ff_effect,
    // (effect_expiration is never used)

    // The current Linux joystick driver maps balls to two axes
    balls: Vec<[i32; 2]>,

    // The current Linux joystick driver maps hats to two axes
    hats: Vec<[i32; 2]>,

    // Support for the Linux 2.4 unified input interface
    key_map: [u8; KEY_CNT],
    abs_map: [u8; ABS_CNT],
    has_key: [bool; KEY_CNT],
    has_abs: [bool; ABS_CNT],
    has_accelerometer: bool,
    has_gyro: bool,

    // Support for the classic joystick interface
    classic: bool,
    key_pam: Option<Vec<u16>>,
    abs_pam: Option<Vec<u8>>,

    abs_correct: [AxisCorrect; ABS_CNT],

    accelerometer_scale: [f32; 3],
    gyro_scale: [f32; 3],

    /* Each axis is read independently, if we don't get all axis this call to
     * LINUX_JoystickUpdateupdate(), store them for the next one */
    gyro_data: [f32; 3],
    accel_data: [f32; 3],
    sensor_tick: u64,
    last_tick: i32,

    report_sensor: bool,
    fresh: bool,
    recovering_from_dropped: bool,
    recovering_from_dropped_sensor: bool,

    // (the Steam Controller flag, m_bSteamController, is never used)

    // 4 = (ABS_HAT3X-ABS_HAT0X)/2 (see input-event-codes.h in kernel)
    hats_indices: [i32; 4],
    has_hat: [bool; 4],
    hat_correct: [HatAxisCorrect; 4],

    // Set when gamepad is pending removal due to ENODEV read error
    gone: bool,
    sensor_gone: bool,
}

impl HwData {
    /// A zeroed structure (`SDL_calloc(1, sizeof(*joystick->hwdata))`).
    fn new(instance_id: JoystickID) -> HwData {
        HwData {
            instance_id,
            // FIXME (upstream): the C structure is calloc'ed, so its file
            // descriptors start as 0; when PrepareJoystickHwdata() fails to
            // open the device, LINUX_JoystickGetGamepadMapping() then closes
            // fd 0 (twice). They start as -1 here, so nothing is closed.
            fd: -1,
            fd_sensor: -1,
            item: false,
            item_vendor: 0,
            item_sensor: None,
            guid: Guid::ZERO,
            fname: String::new(),
            ff_rumble: false,
            ff_sine: false,
            effect: ff_effect::zeroed(),
            balls: Vec::new(),
            hats: Vec::new(),
            key_map: [0; KEY_CNT],
            abs_map: [0; ABS_CNT],
            has_key: [false; KEY_CNT],
            has_abs: [false; ABS_CNT],
            has_accelerometer: false,
            has_gyro: false,
            classic: false,
            key_pam: None,
            abs_pam: None,
            abs_correct: [AxisCorrect::default(); ABS_CNT],
            accelerometer_scale: [0.0; 3],
            gyro_scale: [0.0; 3],
            gyro_data: [0.0; 3],
            accel_data: [0.0; 3],
            sensor_tick: 0,
            last_tick: 0,
            report_sensor: false,
            fresh: false,
            recovering_from_dropped: false,
            recovering_from_dropped_sensor: false,
            hats_indices: [0; 4],
            has_hat: [false; 4],
            hat_correct: [HatAxisCorrect::default(); 4],
            gone: false,
            sensor_gone: false,
        }
    }

    /// `key_map[code]`, unmapped (0xFF) past the end of the table. The
    /// tables have an entry for every code the kernel can report (`KEY_CNT`,
    /// `ABS_CNT`), so only a malformed code is out of range.
    fn key_map(&self, code: usize) -> u8 {
        self.key_map.get(code).copied().unwrap_or(0xFF)
    }

    /// `abs_map[code]`, unmapped (0xFF) past the end of the table.
    fn abs_map(&self, code: usize) -> u8 {
        self.abs_map.get(code).copied().unwrap_or(0xFF)
    }
}

/// An event an update delivers once the device state is no longer
/// borrowed (the `SDL_SendJoystick*()` calls of the C code, in order).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Pending {
    Axis(u64, u8, i16),
    Button(u64, u8, bool),
    Hat(u64, u8, u8),
    Ball(u64, u8, i16, i16),
    Sensor(u64, SensorType, u64, [f32; 3]),
}

/// The driver's state (the file-level statics of `SDL_sysjoystick.c`).
struct LinuxState {
    /// Translation of `SDL_classic_joysticks`.
    classic_joysticks: bool,
    /// Translation of `enumeration_method`.
    enumeration_method: EnumerationMethod,
    /// Translation of `SDL_joylist` (`numjoysticks` is its length).
    joylist: Vec<JoylistItem>,
    /// Translation of `SDL_sensorlist`.
    sensorlist: Vec<SensorlistItem>,
    /// Translation of `inotify_fd`.
    inotify_fd: RawFd,
    /// Translation of `last_joy_detect_time`.
    last_joy_detect_time: u64,
    /// Translation of `last_input_dir_mtime`.
    last_input_dir_mtime: i64,
    /// The open joysticks' `joystick->hwdata`.
    open: Vec<HwData>,
}

/// Guarded by the joystick lock upstream; the `RefCell` borrow is never
/// held across an event push or a call back into the joystick API.
static STATE: ReentrantMutex<RefCell<LinuxState>> = ReentrantMutex::new(RefCell::new(LinuxState {
    classic_joysticks: false,
    enumeration_method: EnumerationMethod::Unset,
    joylist: Vec::new(),
    sensorlist: Vec::new(),
    inotify_fd: -1,
    last_joy_detect_time: 0,
    last_input_dir_mtime: 0,
    open: Vec::new(),
}));

fn with_state<R>(f: impl FnOnce(&mut LinuxState) -> R) -> R {
    let guard = STATE.lock();
    let mut state = guard.borrow_mut();
    f(&mut state)
}

/// Run `f` on the device of an open joystick (`joystick->hwdata`).
fn with_hwdata<R>(instance_id: JoystickID, f: impl FnOnce(&mut HwData) -> R) -> Option<R> {
    with_state(|s| {
        s.open
            .iter_mut()
            .find(|h| h.instance_id == instance_id)
            .map(f)
    })
}

/// `open(path, flags, 0)`, or -1.
fn open_path(path: &str, flags: c_int) -> RawFd {
    let Ok(c) = CString::new(path) else {
        return -1;
    };
    // SAFETY: c is NUL-terminated.
    unsafe { libc::open(c.as_ptr(), flags, 0) }
}

/// `close(fd)`
fn close_fd(fd: RawFd) {
    // SAFETY: fd is a descriptor this driver opened and closes once.
    unsafe {
        libc::close(fd);
    }
}

/// `fcntl(fd, F_SETFL, O_NONBLOCK)`
fn set_nonblocking(fd: RawFd) {
    // SAFETY: fcntl on a descriptor we own; F_SETFL takes an int.
    unsafe {
        libc::fcntl(fd, libc::F_SETFL, libc::O_NONBLOCK);
    }
}

/// `ioctl(fd, EVIOCGBIT(ev, sizeof(bits)), bits) >= 0`
fn ioctl_bits(fd: RawFd, ev: u16, bits: &mut [c_ulong]) -> bool {
    // SAFETY: the request's size is the slice's size in bytes.
    unsafe {
        ioctl_ptr(
            fd,
            eviocgbit(ev, std::mem::size_of_val(bits)),
            bits.as_mut_ptr().cast(),
        ) >= 0
    }
}

/// `ioctl(fd, EVIOCGABS(axis), &absinfo)`, if it succeeds.
fn ioctl_absinfo(fd: RawFd, axis: usize) -> Option<input_absinfo> {
    let mut absinfo = input_absinfo::default();
    (ioctl_read(fd, eviocgabs(axis), &mut absinfo) >= 0).then_some(absinfo)
}

/// `ioctl(fd, request(sizeof(buf)), buf)` for a string request, with its
/// result and the string up to the first NUL.
fn ioctl_string(fd: RawFd, request: fn(usize) -> c_ulong, len: usize) -> (c_int, String) {
    let mut buf = vec![0u8; len];
    // SAFETY: buf has the length the request encodes.
    let result = unsafe { ioctl_ptr(fd, request(len), buf.as_mut_ptr().cast()) };
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    (result, String::from_utf8_lossy(&buf[..end]).into_owned())
}

/// `SDL_atoi()`
fn atoi(s: &str) -> i32 {
    crate::stdlib::string::strtol(s, 10).0 as i32
}

/// Translation of `FixupDeviceInfoForMapping()`.
fn fixup_device_info_for_mapping(fd: RawFd, inpid: &mut input_id) {
    if inpid.vendor == 0x045e && inpid.product == 0x0b05 && inpid.version == 0x0903 {
        // This is a Microsoft Xbox One Elite Series 2 controller
        let mut keybit: KeyBits = [0; nbits(KEY_MAX)];

        // The first version of the firmware duplicated all the inputs
        if ioctl_bits(fd, EV_KEY, &mut keybit) && test_bit(0x2c0, &keybit) {
            // Change the version to 0x0902, so we can map it differently
            inpid.version = 0x0902;
        }
    }

    /* For Atari vcs modern and classic controllers have the version reflecting
     * firmware version, but the mapping stays stable so ignore
     * version information */
    if inpid.vendor == 0x3250 && (inpid.product == 0x1001 || inpid.product == 0x1002) {
        inpid.version = 0;
    }
}

/// Translation of `IsVirtualJoystick()` in a build without HIDAPI.
fn is_virtual_joystick(_vendor: u16, _product: u16, _version: u16, _name: &str) -> bool {
    false
}

/// Translation of `GetSteamVirtualGamepadSlot()`.
fn get_steam_virtual_gamepad_slot(fd: RawFd) -> Option<i32> {
    let (result, name) = ioctl_string(fd, eviocgname, 128);
    if result > 0 {
        if let Some(i) = name.find("pad ") {
            let digits = &name[i + 4..];
            if digits.as_bytes().first().is_some_and(u8::is_ascii_digit) {
                return Some(atoi(digits));
            }
        }
    }
    None
}

/// Translation of `GuessDeviceClass()`.
fn guess_device_class_fd(fd: RawFd) -> DeviceClass {
    let mut propbit: PropBits = Default::default();
    let mut evbit: EvBits = Default::default();
    let mut keybit: KeyBits = [0; nbits(KEY_MAX)];
    let mut absbit: AbsBits = Default::default();
    let mut relbit: RelBits = Default::default();

    if !ioctl_bits(fd, 0, &mut evbit)
        || !ioctl_bits(fd, EV_KEY, &mut keybit)
        || !ioctl_bits(fd, EV_REL, &mut relbit)
        || !ioctl_bits(fd, EV_ABS, &mut absbit)
    {
        return DeviceClass::UNKNOWN;
    }

    /* This is a newer feature, so it's allowed to fail - if so, then the
     * device just doesn't have any properties. */
    // SAFETY: the request's size is propbit's size.
    let _ = unsafe {
        ioctl_ptr(
            fd,
            eviocgprop(size_of::<PropBits>()),
            propbit.as_mut_ptr().cast(),
        )
    };

    guess_device_class(&propbit, &evbit, &absbit, &keybit, &relbit)
}

/// Translation of `GuessIsJoystick()`.
fn guess_is_joystick(fd: RawFd) -> bool {
    guess_device_class_fd(fd).intersects(DeviceClass::JOYSTICK)
}

/// Translation of `GuessIsSensor()`.
fn guess_is_sensor(fd: RawFd) -> bool {
    guess_device_class_fd(fd).intersects(DeviceClass::ACCELEROMETER)
}

/// What `IsJoystick()` returns through its out-parameters.
struct JoystickInfo {
    name: String,
    vendor: u16,
    product: u16,
    guid: Guid,
    driver: Option<String>,
}

/// Translation of `IsJoystick()`. `fd` is opened if it is negative.
fn is_joystick(
    enumeration_method: EnumerationMethod,
    path: &str,
    fd: &mut RawFd,
) -> Option<JoystickInfo> {
    let mut inpid = input_id::default();
    let mut driver = None;
    let mut class = DeviceClass::UNKNOWN;

    // Opening input devices can generate synchronous device I/O, so avoid it if we can
    if let Some(info) = udev::product_info(path, true) {
        inpid = info.inpid;
        class = info.class;
        driver = info.driver;
        if !class.intersects(DeviceClass::JOYSTICK) {
            return None;
        }
    }

    if *fd < 0 {
        *fd = open_path(path, libc::O_RDONLY | libc::O_CLOEXEC);
    }
    if *fd < 0 {
        return None;
    }

    let (result, mut product_string) = ioctl_string(*fd, jsiocgname, 128);
    if result <= 0 {
        // When udev enumeration or classification, we only got joysticks here, so no need to test
        if enumeration_method != EnumerationMethod::Libudev
            && class.is_empty()
            && !guess_is_joystick(*fd)
        {
            return None;
        }

        // Could have vendor and product already from udev, but should agree with evdev
        if ioctl_read(*fd, EVIOCGID, &mut inpid) < 0 {
            return None;
        }

        let (result, name) = ioctl_string(*fd, eviocgname, 128);
        if result < 0 {
            return None;
        }
        product_string = name;
    }

    let name = create_joystick_name(inpid.vendor, inpid.product, None, Some(&product_string))?;

    if !is_virtual_joystick(inpid.vendor, inpid.product, inpid.version, &name)
        && joystick_handled_by_another_driver(
            super::LINUX_DRIVER_INDEX,
            inpid.vendor,
            inpid.product,
            inpid.version,
            Some(&name),
        )
    {
        return None;
    }

    fixup_device_info_for_mapping(*fd, &mut inpid);

    if should_ignore_joystick(inpid.vendor, inpid.product, inpid.version, Some(&name)) {
        return None;
    }
    Some(JoystickInfo {
        name,
        vendor: inpid.vendor,
        product: inpid.product,
        guid: create_joystick_guid(
            inpid.bustype,
            inpid.vendor,
            inpid.product,
            inpid.version,
            None,
            Some(&product_string),
            0,
            0,
        ),
        driver,
    })
}

/// Translation of `IsSensor()`. `fd` is opened if it is negative.
fn is_sensor(path: &str, fd: &mut RawFd) -> bool {
    let mut inpid = input_id::default();
    let mut class = DeviceClass::UNKNOWN;

    // Opening input devices can generate synchronous device I/O, so avoid it if we can
    if let Some(info) = udev::product_info(path, false) {
        class = info.class;
        if !class.intersects(DeviceClass::ACCELEROMETER) {
            return false;
        }
    }

    if *fd < 0 {
        *fd = open_path(path, libc::O_RDONLY | libc::O_CLOEXEC);
    }
    if *fd < 0 {
        return false;
    }

    if class.is_empty() && !guess_is_sensor(*fd) {
        return false;
    }

    if ioctl_read(*fd, EVIOCGID, &mut inpid) < 0 {
        return false;
    }

    if inpid.vendor == super::USB_VENDOR_NINTENDO
        && inpid.product == super::USB_PRODUCT_NINTENDO_WII_REMOTE
    {
        // Wii extension controls
        // These may create 3 sensor devices but we only support reading from 1: ignore them
        return false;
    }

    true
}

/// Translation of `joystick_udev_callback()`.
fn joystick_udev_callback(udev_type: UdevDeviceEvent, udev_class: DeviceClass, devpath: &str) {
    match udev_type {
        UdevDeviceEvent::Added => {
            if !udev_class.intersects(DeviceClass::JOYSTICK | DeviceClass::ACCELEROMETER) {
                return;
            }
            let classic_joysticks = with_state(|s| s.classic_joysticks);
            if classic_joysticks {
                if !is_joystick_js_node(devpath) {
                    return;
                }
            } else if is_joystick_js_node(devpath) {
                return;
            }

            // Wait a bit for the hidraw udev node to initialize
            crate::timer::delay(std::time::Duration::from_millis(10));

            maybe_add_device(devpath);
        }

        UdevDeviceEvent::Removed => {
            maybe_remove_device(devpath);
        }
    }
}

/// Translation of `MaybeAddDevice()`.
fn maybe_add_device(path: &str) {
    let mut fd = open_path(path, libc::O_RDONLY | libc::O_CLOEXEC);
    if fd < 0 {
        return;
    }

    // SAFETY: an all-zero stat is a valid value to be overwritten.
    let mut sb: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: fd is open and sb is a writable stat.
    if unsafe { libc::fstat(fd, &mut sb) } == -1 {
        close_fd(fd);
        return;
    }

    let _lock = lock_joysticks();

    let added = 'done: {
        // Check to make sure it's not already in list.
        let (known, enumeration_method) = with_state(|s| {
            (
                s.joylist.iter().any(|item| sb.st_rdev == item.devnum)
                    || s.sensorlist.iter().any(|item| sb.st_rdev == item.devnum),
                s.enumeration_method,
            )
        });
        if known {
            break 'done None; // already have this one
        }

        if let Some(info) = is_joystick(enumeration_method, path, &mut fd) {
            let mut item = JoylistItem {
                device_instance: 0,
                path: path.to_owned(),
                vendor: info.vendor,
                name: info.name,
                driver: info.driver,
                guid: info.guid,
                devnum: sb.st_rdev,
                steam_virtual_gamepad_slot: -1,
                hwdata: false,
                checked_mapping: false,
                mapping: None,
            };

            if info.vendor == super::USB_VENDOR_VALVE
                && info.product == super::USB_PRODUCT_STEAM_VIRTUAL_GAMEPAD
            {
                if let Some(slot) = get_steam_virtual_gamepad_slot(fd) {
                    item.steam_virtual_gamepad_slot = slot;
                }
            }

            item.device_instance = crate::utils::next_object_id();
            let device_instance = item.device_instance;

            // Need to increment the joystick count before we post the event
            with_state(|s| s.joylist.push(item));

            break 'done Some(device_instance);
        }

        if is_sensor(path, &mut fd) {
            let item_sensor = SensorlistItem {
                devnum: sb.st_rdev,
                path: path.to_owned(),
                hwdata: None,
            };

            with_state(|s| s.sensorlist.insert(0, item_sensor));
        }
        None
    };

    close_fd(fd);
    if let Some(device_instance) = added {
        private_joystick_added(device_instance);
    }
}

/// Translation of `RemoveJoylistItem()`: the item at `index` leaves the
/// list; returns its instance ID for `SDL_PrivateJoystickRemoved()`.
fn remove_joylist_item(s: &mut LinuxState, index: usize) -> JoystickID {
    assert_joysticks_locked();

    let item = s.joylist.remove(index);
    if item.hwdata {
        if let Some(hwdata) = s
            .open
            .iter_mut()
            .find(|h| h.instance_id == item.device_instance)
        {
            hwdata.item = false;
        }
    }

    // Need to decrement the joystick count before we post the event
    item.device_instance
}

/// Translation of `RemoveSensorlistItem()`.
fn remove_sensorlist_item(s: &mut LinuxState, index: usize) {
    assert_joysticks_locked();

    let item = s.sensorlist.remove(index);
    if let Some(instance_id) = item.hwdata {
        if let Some(hwdata) = s.open.iter_mut().find(|h| h.instance_id == instance_id) {
            hwdata.item_sensor = None;
        }
    }

    /* Do not call SDL_PrivateJoystickRemoved here as RemoveJoylistItem will do it,
     * assuming both sensor and joy item are removed at the same time */
}

/// Translation of `MaybeRemoveDevice()`.
fn maybe_remove_device(path: &str) {
    let _lock = lock_joysticks();
    let removed = with_state(|s| {
        // found it, remove it.
        if let Some(i) = s.joylist.iter().position(|item| path == item.path) {
            return Some(remove_joylist_item(s, i));
        }
        if let Some(i) = s.sensorlist.iter().position(|item| path == item.path) {
            remove_sensorlist_item(s, i);
        }
        None
    });
    if let Some(device_instance) = removed {
        private_joystick_removed(device_instance);
    }
}

/// Translation of `HandlePendingRemovals()`.
fn handle_pending_removals() {
    assert_joysticks_locked();

    let removed = with_state(|s| {
        let mut removed = Vec::new();
        let gone = |s: &LinuxState, item: &JoylistItem| {
            item.hwdata
                && s.open
                    .iter()
                    .any(|h| h.instance_id == item.device_instance && h.gone)
        };
        let mut i = 0;
        while i < s.joylist.len() {
            if gone(s, &s.joylist[i]) {
                removed.push(remove_joylist_item(s, i));
            } else {
                i += 1;
            }
        }

        let mut i = 0;
        while i < s.sensorlist.len() {
            let sensor_gone = s.sensorlist[i]
                .hwdata
                .is_some_and(|id| s.open.iter().any(|h| h.instance_id == id && h.sensor_gone));
            if sensor_gone {
                remove_sensorlist_item(s, i);
            } else {
                i += 1;
            }
        }
        removed
    });
    // (the removal events are sent once the lists are consistent again)
    for device_instance in removed {
        private_joystick_removed(device_instance);
    }
}

/// Translation of `StrIsInteger()`.
fn str_is_integer(string: &str) -> bool {
    !string.is_empty() && string.bytes().all(|p| p.is_ascii_digit())
}

/// The file name of a path (after the last slash).
fn node_name(node: &str) -> &str {
    match node.rfind('/') {
        Some(last_slash) => &node[last_slash + 1..],
        None => node,
    }
}

/// Translation of `IsJoystickJSNode()`.
fn is_joystick_js_node(node: &str) -> bool {
    let node = node_name(node);
    node.starts_with("js") && str_is_integer(&node[2..])
}

/// Translation of `IsJoystickEventNode()`.
fn is_joystick_event_node(node: &str) -> bool {
    let node = node_name(node);
    node.starts_with("event") && str_is_integer(&node[5..])
}

/// Translation of `IsJoystickDeviceNode()`.
fn is_joystick_device_node(classic_joysticks: bool, node: &str) -> bool {
    if classic_joysticks {
        is_joystick_js_node(node)
    } else {
        is_joystick_event_node(node)
    }
}

/// Translation of `SDL_inotify_init1()`.
fn inotify_init1() -> RawFd {
    // SAFETY: inotify_init1 has no preconditions.
    unsafe { libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC) }
}

/// Translation of `LINUX_InotifyJoystickDetect()`.
fn inotify_joystick_detect(inotify_fd: RawFd, classic_joysticks: bool) {
    const HEADER: usize = size_of::<libc::inotify_event>();
    let mut buf = [0u8; 4096];

    // SAFETY: buf is writable for its length.
    let bytes = unsafe { libc::read(inotify_fd, buf.as_mut_ptr().cast(), buf.len()) };

    let remain = if bytes > 0 { bytes as usize } else { 0 };

    // (upstream moves each handled event out of the buffer; this walks it)
    let mut offset = 0;
    while offset + HEADER <= remain {
        // SAFETY: a whole inotify_event header lies at offset; it may be
        // unaligned in the byte buffer.
        let event: libc::inotify_event =
            unsafe { std::ptr::read_unaligned(buf.as_ptr().add(offset).cast()) };
        let len = event.len as usize;
        if len > 0 && offset + HEADER + len <= remain {
            let name = &buf[offset + HEADER..offset + HEADER + len];
            let end = name.iter().position(|&b| b == 0).unwrap_or(name.len());
            let name = String::from_utf8_lossy(&name[..end]);
            if is_joystick_device_node(classic_joysticks, &name) {
                let path = format!("/dev/input/{name}");

                if event.mask & (libc::IN_CREATE | libc::IN_MOVED_TO | libc::IN_ATTRIB) != 0 {
                    maybe_add_device(&path);
                } else if event.mask & (libc::IN_DELETE | libc::IN_MOVED_FROM) != 0 {
                    maybe_remove_device(&path);
                }
            }
        }

        offset += HEADER + len;
    }
}

/// The names in a directory, like `scandir()` (without "." and "..",
/// which none of the filters accept).
fn scan_directory(path: &str, filter: impl Fn(&str) -> bool) -> Vec<String> {
    let Ok(dir) = std::fs::read_dir(path) else {
        return Vec::new();
    };
    dir.filter_map(|entry| entry.ok())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| filter(name))
        .collect()
}

/// Translation of `get_event_joystick_index()`.
fn get_event_joystick_index(event: i32) -> i32 {
    let mut joystick_index = -1;
    let path = format!("/sys/class/input/event{event}/device");
    let mut entries = scan_directory(&path, |_| true);
    // (alphasort)
    entries.sort();
    for entry in &entries {
        if let Some(number) = entry.strip_prefix("js") {
            joystick_index = atoi(number);
        }
    }

    joystick_index
}

/// Sort device nodes the way `sort_entries()` orders them: by number, or
/// for event nodes by the number of their classic joystick node when they
/// have one (those first).
fn sort_entries(classic_joysticks: bool, entries: &mut [String]) {
    // (the key of each entry is computed once, rather than per comparison)
    let key = |name: &str| -> (i32, i32) {
        if classic_joysticks {
            let offset = 2; // strlen("js")
            (atoi(&name[offset..]), -1)
        } else {
            let offset = 5; // strlen("event")
            let num = atoi(&name[offset..]);

            // See if we can get the joystick ordering
            (num, get_event_joystick_index(num))
        }
    };
    let mut keyed: Vec<((i32, i32), String)> = entries
        .iter()
        .map(|name| (key(name), name.clone()))
        .collect();
    keyed.sort_by(|((num_a, js_a), _), ((num_b, js_b), _)| {
        if *js_a >= 0 && *js_b >= 0 {
            js_a.cmp(js_b)
        } else if *js_a >= 0 {
            std::cmp::Ordering::Less
        } else if *js_b >= 0 {
            std::cmp::Ordering::Greater
        } else {
            num_a.cmp(num_b)
        }
    });
    for (slot, (_, name)) in entries.iter_mut().zip(keyed) {
        *slot = name;
    }
}

/// Translation of `LINUX_ScanSteamVirtualGamepads()`.
fn scan_steam_virtual_gamepads(classic_joysticks: bool) {
    /// Translation of `VirtualGamepadEntry`.
    struct VirtualGamepadEntry {
        path: String,
        slot: i32,
    }
    let mut virtual_gamepads = Vec::new();

    let entries = scan_directory("/dev/input", |name| {
        is_joystick_device_node(classic_joysticks, name)
    });
    for entry in entries {
        let path = format!("/dev/input/{entry}");

        // Opening input devices can generate synchronous device I/O, so avoid it if we can
        if let Some(info) = udev::product_info(&path, false) {
            if info.inpid.vendor != super::USB_VENDOR_VALVE
                || info.inpid.product != super::USB_PRODUCT_STEAM_VIRTUAL_GAMEPAD
            {
                continue;
            }
        }
        let fd = open_path(&path, libc::O_RDONLY | libc::O_CLOEXEC);
        if fd >= 0 {
            let mut inpid = input_id::default();
            if ioctl_read(fd, EVIOCGID, &mut inpid) == 0
                && inpid.vendor == super::USB_VENDOR_VALVE
                && inpid.product == super::USB_PRODUCT_STEAM_VIRTUAL_GAMEPAD
            {
                if let Some(slot) = get_steam_virtual_gamepad_slot(fd) {
                    virtual_gamepads.push(VirtualGamepadEntry { path, slot });
                }
            }
            close_fd(fd);
        }
    }

    if virtual_gamepads.len() > 1 {
        // (sort_virtual_gamepads)
        virtual_gamepads.sort_by_key(|entry| entry.slot);
    }
    for entry in virtual_gamepads {
        maybe_add_device(&entry.path);
    }
}

/// Translation of `LINUX_ScanInputDevices()`.
fn scan_input_devices(classic_joysticks: bool) {
    let mut entries = scan_directory("/dev/input", |name| {
        is_joystick_device_node(classic_joysticks, name)
    });
    if entries.len() > 1 {
        sort_entries(classic_joysticks, &mut entries);
    }
    for entry in entries {
        let path = format!("/dev/input/{entry}");
        maybe_add_device(&path);
    }
}

/// Translation of `LINUX_FallbackJoystickDetect()`.
fn fallback_joystick_detect() {
    const SDL_JOY_DETECT_INTERVAL_MS: u64 = 3000; // Update every 3 seconds
    let now = crate::timer::ticks_ms();

    let (last_joy_detect_time, last_input_dir_mtime, classic_joysticks) = with_state(|s| {
        (
            s.last_joy_detect_time,
            s.last_input_dir_mtime,
            s.classic_joysticks,
        )
    });
    if last_joy_detect_time == 0 || now >= last_joy_detect_time + SDL_JOY_DETECT_INTERVAL_MS {
        // Opening input devices can generate synchronous device I/O, so avoid it if we can
        use std::os::unix::fs::MetadataExt;
        if let Ok(sb) = std::fs::metadata("/dev/input") {
            if sb.mtime() != last_input_dir_mtime {
                // Look for Steam virtual gamepads first, and sort by Steam controller slot
                scan_steam_virtual_gamepads(classic_joysticks);

                scan_input_devices(classic_joysticks);

                with_state(|s| s.last_input_dir_mtime = sb.mtime());
            }
        }

        with_state(|s| s.last_joy_detect_time = now);
    }
}

/// Translation of `LINUX_JoystickDetect()`.
fn joystick_detect() {
    let (enumeration_method, inotify_fd, last_joy_detect_time, classic_joysticks) =
        with_state(|s| {
            (
                s.enumeration_method,
                s.inotify_fd,
                s.last_joy_detect_time,
                s.classic_joysticks,
            )
        });
    if enumeration_method == EnumerationMethod::Libudev {
        // Polling will happen in the main event loop
    } else if inotify_fd >= 0 && last_joy_detect_time != 0 {
        inotify_joystick_detect(inotify_fd, classic_joysticks);
    } else {
        fallback_joystick_detect();
    }

    handle_pending_removals();
}

/// Translation of `GetJoystickByDevIndex()`: run `f` on the item.
fn with_item<R>(device_index: usize, f: impl FnOnce(&mut JoylistItem) -> R) -> Option<R> {
    assert_joysticks_locked();

    with_state(|s| s.joylist.get_mut(device_index).map(f))
}

/// Translation of `GuessIfAxesAreDigitalHat()`.
fn guess_if_axes_are_digital_hat(
    absinfo_x: Option<&input_absinfo>,
    absinfo_y: Option<&input_absinfo>,
) -> bool {
    /* A "hat" is assumed to be a digital input with at most 9 possible states
     * (3 per axis: negative/zero/positive), as opposed to a true "axis" which
     * can report a continuous range of possible values. Unfortunately the Linux
     * joystick interface makes no distinction between digital hat axes and any
     * other continuous analog axis, so we have to guess. */

    // If both axes are missing, they're not anything.
    if absinfo_x.is_none() && absinfo_y.is_none() {
        return false;
    }

    // If the hint says so, treat all hats as digital.
    if hints::get_bool(hints::JOYSTICK_LINUX_DIGITAL_HATS, false) {
        return true;
    }

    // If both axes have ranges constrained between -1 and 1, they're definitely digital.
    let constrained =
        |a: Option<&input_absinfo>| a.is_none_or(|a| a.minimum == -1 && a.maximum == 1);
    if constrained(absinfo_x) && constrained(absinfo_y) {
        return true;
    }

    // If both axes lack fuzz, flat, and resolution values, they're probably digital.
    let plain = |a: Option<&input_absinfo>| {
        a.is_none_or(|a| a.fuzz == 0 && a.flat == 0 && a.resolution == 0)
    };
    if plain(absinfo_x) && plain(absinfo_y) {
        return true;
    }

    // Otherwise, treat them as analog.
    false
}

/// The unified event API half of `ConfigJoystick()`: buttons, hats, axes
/// and balls from the device's capability bitmasks, reading the axes'
/// calibration with `get_absinfo` (`ioctl(fd, EVIOCGABS(axis), ...)`).
fn config_evdev_inputs(
    hwdata: &mut HwData,
    counts: &mut Counts,
    keybit: &KeyBits,
    absbit: &AbsBits,
    relbit: &RelBits,
    mut get_absinfo: impl FnMut(usize) -> Option<input_absinfo>,
) {
    let use_deadzones = hints::get_bool(hints::JOYSTICK_LINUX_DEADZONES, false);
    let use_hat_deadzones = hints::get_bool(hints::JOYSTICK_LINUX_HAT_DEADZONES, true);

    // Get the number of buttons, axes, and other thingamajigs
    for i in (BTN_JOYSTICK..KEY_MAX).chain(0..BTN_JOYSTICK) {
        if test_bit(i, keybit) {
            hwdata.key_map[i] = counts.nbuttons as u8;
            hwdata.has_key[i] = true;
            counts.nbuttons += 1;
        }
    }
    for i in (ABS_HAT0X..=ABS_HAT3Y).step_by(2) {
        let absinfo_x = if test_bit(i, absbit) {
            get_absinfo(i)
        } else {
            None
        };
        let absinfo_y = if test_bit(i + 1, absbit) {
            get_absinfo(i + 1)
        } else {
            None
        };
        if guess_if_axes_are_digital_hat(absinfo_x.as_ref(), absinfo_y.as_ref()) {
            let hat_index = (i - ABS_HAT0X) / 2;
            let correct = &mut hwdata.hat_correct[hat_index];
            hwdata.hats_indices[hat_index] = counts.nhats as i32;
            hwdata.has_hat[hat_index] = true;
            correct.use_deadzones = use_hat_deadzones;
            correct.minimum[0] = absinfo_x.map_or(-1, |a| a.minimum);
            correct.maximum[0] = absinfo_x.map_or(1, |a| a.maximum);
            correct.minimum[1] = absinfo_y.map_or(-1, |a| a.minimum);
            correct.maximum[1] = absinfo_y.map_or(1, |a| a.maximum);
            counts.nhats += 1;
        }
    }
    for i in 0..ABS_MAX {
        // Skip digital hats
        if (ABS_HAT0X..=ABS_HAT3Y).contains(&i) && hwdata.has_hat[(i - ABS_HAT0X) / 2] {
            continue;
        }
        if test_bit(i, absbit) {
            let Some(absinfo) = get_absinfo(i) else {
                continue;
            };
            hwdata.abs_map[i] = counts.naxes as u8;
            hwdata.has_abs[i] = true;

            let correct = &mut hwdata.abs_correct[i];
            correct.minimum = absinfo.minimum;
            correct.maximum = absinfo.maximum;
            if correct.minimum != correct.maximum {
                if use_deadzones {
                    correct.use_deadzones = true;
                    correct.coef[0] = absinfo
                        .maximum
                        .wrapping_add(absinfo.minimum)
                        .wrapping_sub(2i32.wrapping_mul(absinfo.flat));
                    correct.coef[1] = absinfo
                        .maximum
                        .wrapping_add(absinfo.minimum)
                        .wrapping_add(2i32.wrapping_mul(absinfo.flat));
                    let t = absinfo
                        .maximum
                        .wrapping_sub(absinfo.minimum)
                        .wrapping_sub(4i32.wrapping_mul(absinfo.flat));
                    if t != 0 {
                        correct.coef[2] = (1 << 28) / t;
                    } else {
                        correct.coef[2] = 0;
                    }
                } else {
                    let value_range = correct.maximum.wrapping_sub(correct.minimum) as f32;
                    let output_range = (JOYSTICK_AXIS_MAX as i32 - JOYSTICK_AXIS_MIN as i32) as f32;

                    correct.scale = output_range / value_range;
                }
            }
            counts.naxes += 1;
        }
    }
    if test_bit(REL_X as usize, relbit) || test_bit(REL_Y as usize, relbit) {
        counts.nballs += 1;
    }
}

/// Translation of `ConfigJoystick()`.
fn config_joystick(hwdata: &mut HwData, counts: &mut Counts, fd: RawFd, fd_sensor: RawFd) {
    let mut keybit: KeyBits = [0; nbits(KEY_MAX)];
    let mut absbit: AbsBits = Default::default();
    let mut relbit: RelBits = Default::default();
    let mut ffbit = [0 as c_ulong; nbits(FF_MAX)];

    assert_joysticks_locked();

    // See if this device uses the new unified event API
    if ioctl_bits(fd, EV_KEY, &mut keybit)
        && ioctl_bits(fd, EV_ABS, &mut absbit)
        && ioctl_bits(fd, EV_REL, &mut relbit)
    {
        config_evdev_inputs(hwdata, counts, &keybit, &absbit, &relbit, |axis| {
            ioctl_absinfo(fd, axis)
        });
    } else {
        let mut key_pam_size: u8 = 0;
        let mut abs_pam_size: u8 = 0;
        if ioctl_read(fd, JSIOCGBUTTONS, &mut key_pam_size) >= 0
            && ioctl_read(fd, JSIOCGAXES, &mut abs_pam_size) >= 0
        {
            config_classic_inputs(hwdata, counts, fd, key_pam_size, abs_pam_size);
        }
    }

    // Sensors are only available through the new unified event API
    if fd_sensor >= 0 && ioctl_bits(fd_sensor, EV_ABS, &mut absbit) {
        if test_bit(ABS_X, &absbit) && test_bit(ABS_Y, &absbit) && test_bit(ABS_Z, &absbit) {
            hwdata.has_accelerometer = true;
            for i in 0..3 {
                let Some(absinfo) = ioctl_absinfo(fd_sensor, ABS_X + i) else {
                    hwdata.has_accelerometer = false;
                    break; // do not report an accelerometer if we can't read all axes
                };
                hwdata.accelerometer_scale[i] = absinfo.resolution as f32;
            }
        }

        if test_bit(ABS_RX, &absbit) && test_bit(ABS_RY, &absbit) && test_bit(ABS_RZ, &absbit) {
            hwdata.has_gyro = true;
            for i in 0..3 {
                let Some(absinfo) = ioctl_absinfo(fd_sensor, ABS_RX + i) else {
                    hwdata.has_gyro = false;
                    break; // do not report a gyro if we can't read all axes
                };
                hwdata.gyro_scale[i] = absinfo.resolution as f32;
            }
        }
    }

    // Allocate data to keep track of these thingamajigs
    allocate_data(hwdata, counts);

    if ioctl_bits(fd, EV_FF, &mut ffbit) {
        if test_bit(FF_RUMBLE as usize, &ffbit) {
            hwdata.ff_rumble = true;
        }
        if test_bit(FF_SINE as usize, &ffbit) {
            hwdata.ff_sine = true;
        }
    }
}

/// The classic joystick interface half of `ConfigJoystick()`.
fn config_classic_inputs(
    hwdata: &mut HwData,
    counts: &mut Counts,
    fd: RawFd,
    key_pam_size: u8,
    abs_pam_size: u8,
) {
    let mut key_pam = vec![0u16; KEY_MAX - BTN_MISC + 1];
    // SAFETY: key_pam has the size JSIOCGBTNMAP encodes.
    let key_pam = (unsafe { ioctl_ptr(fd, JSIOCGBTNMAP, key_pam.as_mut_ptr().cast()) } >= 0)
        .then_some(key_pam);
    let mut abs_pam = vec![0u8; ABS_CNT];
    // SAFETY: abs_pam has the size JSIOCGAXMAP encodes.
    let abs_pam = (unsafe { ioctl_ptr(fd, JSIOCGAXMAP, abs_pam.as_mut_ptr().cast()) } >= 0)
        .then_some(abs_pam);
    map_classic_inputs(
        hwdata,
        counts,
        key_pam.map(|k| (k, key_pam_size)),
        abs_pam.map(|a| (a, abs_pam_size)),
    );
}

/// The inputs of a classic joystick from its button and axis maps
/// (`JSIOCGBTNMAP`/`JSIOCGAXMAP` with the `JSIOCGBUTTONS`/`JSIOCGAXES`
/// counts; `None` when that map couldn't be read).
fn map_classic_inputs(
    hwdata: &mut HwData,
    counts: &mut Counts,
    key_pam: Option<(Vec<u16>, u8)>,
    abs_pam: Option<(Vec<u8>, u8)>,
) {
    hwdata.classic = true;

    let key_pam_size = key_pam.as_ref().map_or(0, |&(_, size)| size);
    hwdata.key_pam = key_pam.map(|(k, _)| k);
    for i in 0..key_pam_size as usize {
        let code = hwdata.key_pam.as_ref().map_or(0, |k| k[i]) as usize;
        if code < KEY_CNT {
            hwdata.key_map[code] = counts.nbuttons as u8;
            hwdata.has_key[code] = true;
        }
        counts.nbuttons += 1;
    }

    let abs_pam_size = abs_pam.as_ref().map_or(0, |&(_, size)| size);
    hwdata.abs_pam = abs_pam.map(|(a, _)| a);
    for i in 0..abs_pam_size as usize {
        let code = hwdata.abs_pam.as_ref().map_or(0, |a| a[i]) as usize;

        // TODO: is there any way to detect analog hats in advance via this API?
        if (ABS_HAT0X..=ABS_HAT3Y).contains(&code) {
            let hat_index = (code - ABS_HAT0X) / 2;
            if !hwdata.has_hat[hat_index] {
                hwdata.hats_indices[hat_index] = counts.nhats as i32;
                counts.nhats += 1;
                hwdata.has_hat[hat_index] = true;
                hwdata.hat_correct[hat_index].minimum[0] = -1;
                hwdata.hat_correct[hat_index].maximum[0] = 1;
                hwdata.hat_correct[hat_index].minimum[1] = -1;
                hwdata.hat_correct[hat_index].maximum[1] = 1;
            }
        } else {
            if code < ABS_CNT {
                hwdata.abs_map[code] = counts.naxes as u8;
                hwdata.has_abs[code] = true;
            }
            counts.naxes += 1;
        }
    }
}

/// Translation of `allocate_balldata()` and `allocate_hatdata()`.
fn allocate_data(hwdata: &mut HwData, counts: &Counts) {
    assert_joysticks_locked();

    hwdata.balls = vec![[0, 0]; counts.nballs];
    hwdata.hats = vec![[1, 1]; counts.nhats];
}

/// What `PrepareJoystickHwdata()` reads from the joylist item.
struct ItemInfo {
    path: String,
    guid: Guid,
    vendor: u16,
}

/// This is used to do the heavy lifting for LINUX_JoystickOpen and
/// also LINUX_JoystickGetGamepadMapping, so we can query the hardware
/// without adding an opened SDL_Joystick object to the system. This will
/// not free `hwdata` on error. Translation of `PrepareJoystickHwdata()`.
fn prepare_joystick_hwdata(
    hwdata: &mut HwData,
    counts: &mut Counts,
    item: &ItemInfo,
    item_sensor: Option<&str>,
) -> Result<()> {
    assert_joysticks_locked();

    hwdata.item = true;
    hwdata.item_vendor = item.vendor;
    hwdata.item_sensor = item_sensor.map(str::to_owned);
    hwdata.guid = item.guid;
    hwdata.effect.id = -1;
    hwdata.key_map = [0xFF; KEY_CNT];
    hwdata.abs_map = [0xFF; ABS_CNT];

    let mut fd_sensor = -1;
    // Try read-write first, so we can do rumble
    let mut fd = open_path(&item.path, libc::O_RDWR | libc::O_CLOEXEC);
    if fd < 0 {
        // Try read-only again, at least we'll get events in this case
        fd = open_path(&item.path, libc::O_RDONLY | libc::O_CLOEXEC);
    }
    if fd < 0 {
        return Err(Error::new(format!("Unable to open {}", item.path)));
    }
    // If opening sensor fail, continue with buttons and axes only
    if let Some(path) = item_sensor {
        fd_sensor = open_path(path, libc::O_RDONLY | libc::O_CLOEXEC);
    }

    hwdata.fd = fd;
    hwdata.fd_sensor = fd_sensor;
    hwdata.fname = item.path.clone();

    // Set the joystick to non-blocking read mode
    set_nonblocking(fd);
    if fd_sensor >= 0 {
        set_nonblocking(fd_sensor);
    }

    // Get the number of buttons and axes on the joystick
    config_joystick(hwdata, counts, fd, fd_sensor);
    Ok(())
}

/// `ioctl(fd, EVIOCGUNIQ(sizeof(uniq) - 1), &uniq)` on a freshly opened
/// device node.
fn device_uniq(path: &str) -> Option<String> {
    let fd = open_path(path, libc::O_RDONLY | libc::O_CLOEXEC);
    if fd < 0 {
        return None;
    }
    let mut uniq = [0u8; 128];
    // SAFETY: the request reads at most 127 bytes into the 128 byte buffer.
    let result = unsafe { ioctl_ptr(fd, eviocguniq(uniq.len() - 1), uniq.as_mut_ptr().cast()) };
    close_fd(fd);
    if result < 0 {
        return None;
    }
    let end = uniq.iter().position(|&b| b == 0).unwrap_or(uniq.len());
    Some(String::from_utf8_lossy(&uniq[..end]).into_owned())
}

/// The path of the sensor device that belongs with a joystick.
/// Translation of `GetSensor()`.
fn get_sensor(item_path: &str) -> Option<String> {
    assert_joysticks_locked();

    if with_state(|s| s.sensorlist.is_empty()) {
        return None;
    }

    let uniq_item = device_uniq(item_path)?;

    let sensors: Vec<String> = with_state(|s| {
        s.sensorlist
            .iter()
            // already associated with another joystick
            .filter(|item_sensor| item_sensor.hwdata.is_none())
            .map(|item_sensor| item_sensor.path.clone())
            .collect()
    });

    for path in sensors {
        let Some(uniq_sensor) = device_uniq(&path) else {
            continue;
        };

        if uniq_item == uniq_sensor {
            return Some(path);
        }
    }
    None
}

/// Translation of `HandleHat()`.
fn handle_hat(
    timestamp: u64,
    hwdata: &mut HwData,
    hatidx: usize,
    axis: usize,
    mut value: i32,
    out: &mut Vec<Pending>,
) {
    const POSITION_MAP: [[u8; 3]; 3] = [
        [HAT_LEFTUP, HAT_UP, HAT_RIGHTUP],
        [HAT_LEFT, HAT_CENTERED, HAT_RIGHT],
        [HAT_LEFTDOWN, HAT_DOWN, HAT_RIGHTDOWN],
    ];

    let hatnum = hwdata.hats_indices[hatidx] as usize;
    let correct = &mut hwdata.hat_correct[hatidx];
    /* Hopefully we detected any analog axes and left them as is rather than trying
     * to use them as digital hats, but just in case, the deadzones here will
     * prevent the slightest of twitches on an analog axis from registering as a hat
     * movement. If the axes really are digital, this won't hurt since they should
     * only ever be sending min, 0, or max anyway. */
    if value < 0 {
        if value <= correct.minimum[axis] {
            correct.minimum[axis] = value;
            value = 0;
        } else if !correct.use_deadzones || value < correct.minimum[axis] / 3 {
            value = 0;
        } else {
            value = 1;
        }
    } else if value > 0 {
        if value >= correct.maximum[axis] {
            correct.maximum[axis] = value;
            value = 2;
        } else if !correct.use_deadzones || value > correct.maximum[axis] / 3 {
            value = 2;
        } else {
            value = 1;
        }
    } else {
        // value == 0
        value = 1;
    }
    let Some(the_hat) = hwdata.hats.get_mut(hatnum) else {
        return;
    };
    if value != the_hat[axis] {
        the_hat[axis] = value;
        out.push(Pending::Hat(
            timestamp,
            hatnum as u8,
            POSITION_MAP[the_hat[1] as usize][the_hat[0] as usize],
        ));
    }
}

/// Translation of `HandleBall()`.
fn handle_ball(hwdata: &mut HwData, ball: usize, axis: usize, value: i32) {
    if let Some(ball) = hwdata.balls.get_mut(ball) {
        ball[axis] = ball[axis].wrapping_add(value);
    }
}

/// Translation of `AxisCorrect()`.
fn axis_correct(hwdata: &HwData, which: usize, mut value: i32) -> i32 {
    if let Some(correct) = hwdata.abs_correct.get(which) {
        if correct.minimum != correct.maximum {
            if correct.use_deadzones {
                value = value.wrapping_mul(2);
                if value > correct.coef[0] {
                    if value < correct.coef[1] {
                        return 0;
                    }
                    value = value.wrapping_sub(correct.coef[1]);
                } else {
                    value = value.wrapping_sub(correct.coef[0]);
                }
                value = value.wrapping_mul(correct.coef[2]);
                value >>= 13;
            } else {
                value = crate::stdlib::math::floorf(
                    (value.wrapping_sub(correct.minimum)) as f32 * correct.scale
                        + JOYSTICK_AXIS_MIN as f32
                        + 0.5,
                ) as i32;
            }
        }
    }

    // Clamp and return
    value.clamp(JOYSTICK_AXIS_MIN as i32, JOYSTICK_AXIS_MAX as i32)
}

/// Translation of `PollAllValues()`.
fn poll_all_values(timestamp: u64, hwdata: &mut HwData, out: &mut Vec<Pending>) {
    // Poll all axis
    for i in ABS_X..ABS_CNT {
        // We don't need to test for digital hats here, they won't have has_abs[] set
        if hwdata.has_abs[i] {
            if let Some(absinfo) = ioctl_absinfo(hwdata.fd, i) {
                let value = axis_correct(hwdata, i, absinfo.value);

                out.push(Pending::Axis(timestamp, hwdata.abs_map[i], value as i16));
            }
        }
    }

    // Poll all digital hats
    for i in ABS_HAT0X..=ABS_HAT3Y {
        let baseaxis = i - ABS_HAT0X;
        let hatidx = baseaxis / 2;
        crate::sdl_assert!(hatidx < hwdata.has_hat.len());
        // We don't need to test for analog axes here, they won't have has_hat[] set
        if hwdata.has_hat[hatidx] {
            if let Some(absinfo) = ioctl_absinfo(hwdata.fd, i) {
                let hataxis = baseaxis % 2;
                handle_hat(timestamp, hwdata, hatidx, hataxis, absinfo.value, out);
            }
        }
    }

    // Poll all buttons
    let mut keyinfo: KeyBits = [0; nbits(KEY_MAX)];
    if ioctl_read(hwdata.fd, eviocgkey(size_of::<KeyBits>()), &mut keyinfo) >= 0 {
        for i in 0..KEY_CNT {
            if hwdata.has_key[i] {
                let down = test_bit(i, &keyinfo);
                out.push(Pending::Button(timestamp, hwdata.key_map[i], down));
            }
        }
    }

    // Joyballs are relative input, so there's no poll state. Events only!
}

/// Translation of `CorrectSensorData()`.
fn correct_sensor_data(hwdata: &HwData, values: &[f32; 3]) -> [f32; 3] {
    if hwdata.item_vendor == super::USB_VENDOR_NINTENDO {
        // The Nintendo driver uses a different axis order than SDL
        [-values[1], values[2], -values[0]]
    } else {
        *values
    }
}

/// Translation of `PollAllSensors()`.
fn poll_all_sensors(timestamp: u64, hwdata: &mut HwData, out: &mut Vec<Pending>) {
    crate::sdl_assert!(hwdata.fd_sensor >= 0);

    if hwdata.has_gyro {
        let mut values = [0.0f32; 3];
        for (i, value) in values.iter_mut().enumerate() {
            if let Some(absinfo) = ioctl_absinfo(hwdata.fd_sensor, ABS_RX + i) {
                *value =
                    absinfo.value as f32 * (std::f32::consts::PI / 180.0) / hwdata.gyro_scale[i];
            }
        }
        let data = correct_sensor_data(hwdata, &values);
        out.push(Pending::Sensor(
            timestamp,
            SensorType::Gyro,
            hwdata.sensor_tick.wrapping_mul(1000),
            data,
        ));
    }
    if hwdata.has_accelerometer {
        let mut values = [0.0f32; 3];
        for (i, value) in values.iter_mut().enumerate() {
            if let Some(absinfo) = ioctl_absinfo(hwdata.fd_sensor, ABS_X + i) {
                *value = absinfo.value as f32 * STANDARD_GRAVITY / hwdata.accelerometer_scale[i];
            }
        }
        let data = correct_sensor_data(hwdata, &values);
        out.push(Pending::Sensor(
            timestamp,
            SensorType::Accel,
            hwdata.sensor_tick.wrapping_mul(1000),
            data,
        ));
    }
}

/// `read(fd, events, sizeof(events))` of up to 32 `T`s: how many whole
/// ones were read (0 when the read fails, with `errno` set).
fn read_events<T: Copy + Default>(fd: RawFd, events: &mut [T; 32]) -> usize {
    // SAFETY: events is writable for its size in bytes, and T is plain data
    // for which any bytes are a valid value.
    let len = unsafe { libc::read(fd, events.as_mut_ptr().cast(), size_of::<[T; 32]>()) };
    if len > 0 {
        len as usize / size_of::<T>()
    } else {
        0
    }
}

/// Translation of `HandleInputEvents()`.
fn handle_input_events(hwdata: &mut HwData, out: &mut Vec<Pending>) {
    let mut events = [input_event::default(); 32];

    if hwdata.fresh {
        let ticks = crate::timer::ticks_ns();
        poll_all_values(ticks, hwdata, out);
        if hwdata.report_sensor {
            poll_all_sensors(ticks, hwdata, out);
        }
        hwdata.fresh = false;
    }

    set_errno(0);

    loop {
        let len = read_events(hwdata.fd, &mut events);
        if len == 0 {
            break;
        }
        for event in &events[..len] {
            let code = event.code;

            /* If the kernel sent a SYN_DROPPED, we are supposed to ignore the
            rest of the packet (the end of it signified by a SYN_REPORT) */
            if hwdata.recovering_from_dropped && (event.type_ != EV_SYN || code != SYN_REPORT) {
                continue;
            }

            match event.type_ {
                EV_KEY => {
                    out.push(Pending::Button(
                        get_event_timestamp(event),
                        hwdata.key_map(code as usize),
                        event.value != 0,
                    ));
                }
                EV_ABS => {
                    let code = code as usize;
                    if (ABS_HAT0X..=ABS_HAT3Y).contains(&code) {
                        let hat_index = (code - ABS_HAT0X) / 2;
                        if hwdata.has_hat[hat_index] {
                            handle_hat(
                                get_event_timestamp(event),
                                hwdata,
                                hat_index,
                                code % 2,
                                event.value,
                                out,
                            );
                            continue;
                        }
                    }
                    let value = axis_correct(hwdata, code, event.value);
                    out.push(Pending::Axis(
                        get_event_timestamp(event),
                        hwdata.abs_map(code),
                        value as i16,
                    ));
                }
                EV_REL => {
                    if code == REL_X || code == REL_Y {
                        let code = (code - REL_X) as usize;
                        handle_ball(hwdata, code / 2, code % 2, event.value);
                    }
                }
                EV_SYN => match code {
                    SYN_DROPPED => {
                        hwdata.recovering_from_dropped = true;
                    }
                    SYN_REPORT if hwdata.recovering_from_dropped => {
                        hwdata.recovering_from_dropped = false;
                        // try to sync up to current state now
                        poll_all_values(crate::timer::ticks_ns(), hwdata, out);
                    }
                    _ => {}
                },
                _ => {}
            }
        }
    }

    if errno() == libc::ENODEV {
        // We have to wait until the JoystickDetect callback to remove this
        hwdata.gone = true;
        set_errno(0);
    }

    if hwdata.report_sensor {
        crate::sdl_assert!(hwdata.fd_sensor >= 0);

        loop {
            let len = read_events(hwdata.fd_sensor, &mut events);
            if len == 0 {
                break;
            }
            for event in &events[..len] {
                let code = event.code;

                /* If the kernel sent a SYN_DROPPED, we are supposed to ignore the
                rest of the packet (the end of it signified by a SYN_REPORT) */
                if hwdata.recovering_from_dropped_sensor
                    && (event.type_ != EV_SYN || code != SYN_REPORT)
                {
                    continue;
                }

                match event.type_ {
                    EV_KEY => {
                        crate::sdl_assert!(false);
                    }
                    EV_ABS => match code as usize {
                        ABS_X | ABS_Y | ABS_Z => {
                            let j = code as usize - ABS_X;
                            hwdata.accel_data[j] = event.value as f32 * STANDARD_GRAVITY
                                / hwdata.accelerometer_scale[j];
                        }
                        ABS_RX | ABS_RY | ABS_RZ => {
                            let j = code as usize - ABS_RX;
                            hwdata.gyro_data[j] = event.value as f32
                                * (std::f32::consts::PI / 180.0)
                                / hwdata.gyro_scale[j];
                        }
                        _ => {}
                    },
                    EV_MSC => {
                        if code == MSC_TIMESTAMP {
                            let tick = event.value;
                            let delta = if hwdata.last_tick <= tick {
                                tick.wrapping_sub(hwdata.last_tick)
                            } else {
                                i32::MAX
                                    .wrapping_sub(hwdata.last_tick)
                                    .wrapping_add(tick)
                                    .wrapping_add(1)
                            };
                            hwdata.sensor_tick = hwdata.sensor_tick.wrapping_add(delta as u64);
                            hwdata.last_tick = tick;
                        }
                    }
                    EV_SYN => match code {
                        SYN_DROPPED => {
                            hwdata.recovering_from_dropped_sensor = true;
                        }
                        SYN_REPORT => {
                            if hwdata.recovering_from_dropped_sensor {
                                hwdata.recovering_from_dropped_sensor = false;
                                // try to sync up to current state now
                                poll_all_sensors(crate::timer::ticks_ns(), hwdata, out);
                            } else {
                                let timestamp = get_event_timestamp(event);
                                let sensor_timestamp = hwdata.sensor_tick.wrapping_mul(1000);
                                let data = correct_sensor_data(hwdata, &hwdata.gyro_data);
                                out.push(Pending::Sensor(
                                    timestamp,
                                    SensorType::Gyro,
                                    sensor_timestamp,
                                    data,
                                ));
                                let data = correct_sensor_data(hwdata, &hwdata.accel_data);
                                out.push(Pending::Sensor(
                                    timestamp,
                                    SensorType::Accel,
                                    sensor_timestamp,
                                    data,
                                ));
                            }
                        }
                        _ => {}
                    },
                    _ => {}
                }
            }
        }
    }

    if errno() == libc::ENODEV {
        // We have to wait until the JoystickDetect callback to remove this
        hwdata.sensor_gone = true;
    }
}

/// Translation of `HandleClassicEvents()`.
fn handle_classic_events(hwdata: &mut HwData, out: &mut Vec<Pending>) {
    let mut events = [js_event::default(); 32];
    let timestamp = crate::timer::ticks_ns();

    hwdata.fresh = false;
    loop {
        let len = read_events(hwdata.fd, &mut events);
        if len == 0 {
            break;
        }
        for event in &events[..len] {
            // FIXME (upstream): key_pam/abs_pam are NULL when the mapping
            // ioctls failed, and an event then dereferences NULL; such
            // events are ignored here.
            match event.type_ {
                JS_EVENT_BUTTON => {
                    let Some(&code) = hwdata
                        .key_pam
                        .as_ref()
                        .and_then(|k| k.get(event.number as usize))
                    else {
                        continue;
                    };
                    out.push(Pending::Button(
                        timestamp,
                        hwdata.key_map(code as usize),
                        event.value != 0,
                    ));
                }
                JS_EVENT_AXIS => {
                    let Some(&code) = hwdata
                        .abs_pam
                        .as_ref()
                        .and_then(|a| a.get(event.number as usize))
                    else {
                        continue;
                    };
                    let code = code as usize;
                    if (ABS_HAT0X..=ABS_HAT3Y).contains(&code) {
                        let hat_index = (code - ABS_HAT0X) / 2;
                        if hwdata.has_hat[hat_index] {
                            handle_hat(
                                timestamp,
                                hwdata,
                                hat_index,
                                code % 2,
                                event.value as i32,
                                out,
                            );
                            continue;
                        }
                    }
                    out.push(Pending::Axis(timestamp, hwdata.abs_map(code), event.value));
                }
                _ => {}
            }
        }
    }
}

/// The events of one update: `LINUX_JoystickUpdate()` up to the sends.
fn update_events(hwdata: &mut HwData) -> Vec<Pending> {
    let mut out = Vec::new();
    if hwdata.classic {
        handle_classic_events(hwdata, &mut out);
    } else {
        handle_input_events(hwdata, &mut out);
    }

    // Deliver ball motion updates
    for (i, ball) in hwdata.balls.iter_mut().enumerate() {
        let xrel = ball[0];
        let yrel = ball[1];
        if xrel != 0 || yrel != 0 {
            ball[0] = 0;
            ball[1] = 0;
            out.push(Pending::Ball(0, i as u8, xrel as i16, yrel as i16));
        }
    }
    out
}

/// The device side of `LINUX_JoystickClose()`.
fn close_hwdata(s: &mut LinuxState, hwdata: &mut HwData) {
    if hwdata.effect.id >= 0 {
        ioctl_int(hwdata.fd, EVIOCRMFF, hwdata.effect.id as c_int);
        hwdata.effect.id = -1;
    }
    if hwdata.fd >= 0 {
        close_fd(hwdata.fd);
    }
    if hwdata.fd_sensor >= 0 {
        close_fd(hwdata.fd_sensor);
    }
    if hwdata.item {
        // FIXME (upstream): this also runs for the joystick that
        // LINUX_JoystickGetGamepadMapping() opens temporarily, unlinking the
        // device from a real open joystick if there is one.
        if let Some(item) = s.joylist.iter_mut().find(|item| {
            item.device_instance == hwdata.instance_id
                || (hwdata.instance_id == 0 && item.path == hwdata.fname)
        }) {
            item.hwdata = false;
        }
    }
    if hwdata.item_sensor.is_some() {
        if let Some(item_sensor) = s
            .sensorlist
            .iter_mut()
            .find(|i| i.hwdata == Some(hwdata.instance_id))
        {
            item_sensor.hwdata = None;
        }
    }
}

/// The file descriptor and device path of an open joystick of this driver
/// (`joystick->hwdata->fd` and `fname`), for the haptic driver; `None` for
/// a joystick of another driver (`joystick->driver !=
/// &SDL_LINUX_JoystickDriver`).
pub(crate) fn joystick_fd_and_fname(joystick: &Joystick) -> Option<(RawFd, String)> {
    assert_joysticks_locked();

    let (instance_id, driver) = joystick.with(|j| (j.instance_id, j.driver)).ok()?;
    if driver != super::LINUX_DRIVER_INDEX {
        return None;
    }
    with_hwdata(instance_id, |h| (h.fd, h.fname.clone()))
}

/// Translation of `SDL_LINUX_JoystickDriver`.
pub(super) struct LinuxJoystickDriver;

pub(super) static LINUX_JOYSTICK_DRIVER: LinuxJoystickDriver = LinuxJoystickDriver;

impl JoystickDriver for LinuxJoystickDriver {
    /// Translation of `LINUX_JoystickInit()`.
    fn init(&self) -> Result<()> {
        let devices = hints::get(hints::JOYSTICK_DEVICE);
        let udev_initialized = udev::init().is_ok();

        with_state(|s| {
            s.classic_joysticks = hints::get_bool(hints::JOYSTICK_LINUX_CLASSIC, false);

            s.enumeration_method = EnumerationMethod::Unset;
        });

        // First see if the user specified one or more joysticks to use
        if let Some(devices) = devices {
            for envpath in devices.split(':') {
                maybe_add_device(envpath);
            }
        }

        // Force immediate joystick detection if using fallback
        with_state(|s| {
            s.last_joy_detect_time = 0;
            s.last_input_dir_mtime = 0;
        });

        // Manually scan first, since we sort by device number and udev doesn't
        joystick_detect();

        let mut enumeration_method = with_state(|s| s.enumeration_method);
        if enumeration_method == EnumerationMethod::Unset {
            if hints::get_bool("SDL_JOYSTICK_DISABLE_UDEV", false) {
                crate::log::debug!(
                    crate::log::Category::Input,
                    "udev disabled by SDL_JOYSTICK_DISABLE_UDEV"
                );
                enumeration_method = EnumerationMethod::Fallback;
            } else if crate::init::sandbox() != crate::init::Sandbox::None {
                crate::log::debug!(
                    crate::log::Category::Input,
                    "Container detected, disabling udev integration"
                );
                enumeration_method = EnumerationMethod::Fallback;
            } else {
                crate::log::debug!(
                    crate::log::Category::Input,
                    "Using udev for joystick device discovery"
                );
                enumeration_method = EnumerationMethod::Libudev;
            }
        }

        if enumeration_method == EnumerationMethod::Libudev {
            if udev_initialized {
                with_state(|s| s.enumeration_method = enumeration_method);
                // Set up the udev callback
                if udev::add_callback(joystick_udev_callback).is_err() {
                    udev::quit();
                    return Err(Error::new("Could not set up joystick <-> udev callback"));
                }

                // Force a scan to build the initial device list
                let _ = udev::scan();
            } else {
                crate::log::debug!(
                    crate::log::Category::Input,
                    "udev init failed, disabling udev integration"
                );
                enumeration_method = EnumerationMethod::Fallback;
            }
        } else if udev_initialized {
            udev::quit();
        }
        with_state(|s| s.enumeration_method = enumeration_method);

        if enumeration_method != EnumerationMethod::Libudev {
            let inotify_fd = inotify_init1();

            if inotify_fd < 0 {
                crate::log::warn!(
                    crate::log::Category::Input,
                    "Unable to initialize inotify, falling back to polling: {}",
                    strerror()
                );
            } else {
                /* We need to watch for attribute changes in addition to
                 * creation, because when a device is first created, it has
                 * permissions that we can't read. When udev chmods it to
                 * something that we maybe *can* read, we'll get an
                 * IN_ATTRIB event to tell us. */
                // SAFETY: inotify_fd is open; the path is NUL-terminated.
                let watch = unsafe {
                    libc::inotify_add_watch(
                        inotify_fd,
                        c"/dev/input".as_ptr(),
                        libc::IN_CREATE | libc::IN_DELETE | libc::IN_MOVE | libc::IN_ATTRIB,
                    )
                };
                if watch < 0 {
                    let error = strerror();
                    close_fd(inotify_fd);
                    crate::log::warn!(
                        crate::log::Category::Input,
                        "Unable to add inotify watch, falling back to polling: {}",
                        error
                    );
                } else {
                    with_state(|s| s.inotify_fd = inotify_fd);
                }
            }
        }

        Ok(())
    }

    /// Translation of `LINUX_JoystickGetCount()`.
    fn count(&self) -> usize {
        assert_joysticks_locked();

        with_state(|s| s.joylist.len())
    }

    fn detect(&self) {
        joystick_detect();
    }

    /// Translation of `LINUX_JoystickIsDevicePresent()`.
    fn is_device_present(
        &self,
        _vendor_id: u16,
        _product_id: u16,
        _version: u16,
        _name: Option<&str>,
    ) -> bool {
        // We don't override any other drivers
        false
    }

    /// Translation of `LINUX_JoystickGetDeviceName()`.
    fn device_name(&self, device_index: usize) -> Option<String> {
        with_item(device_index, |item| item.name.clone())
    }

    /// Translation of `LINUX_JoystickGetDevicePath()`.
    fn device_path(&self, device_index: usize) -> Option<String> {
        with_item(device_index, |item| item.path.clone())
    }

    /// Translation of `LINUX_JoystickGetDeviceSteamVirtualGamepadSlot()`.
    fn device_steam_virtual_gamepad_slot(&self, device_index: usize) -> i32 {
        with_item(device_index, |item| item.steam_virtual_gamepad_slot).unwrap_or(-1)
    }

    /// Translation of `LINUX_JoystickGetDevicePlayerIndex()`.
    fn device_player_index(&self, _device_index: usize) -> i32 {
        -1
    }

    /// Translation of `LINUX_JoystickSetDevicePlayerIndex()`.
    fn set_device_player_index(&self, _device_index: usize, _player_index: i32) {}

    /// Translation of `LINUX_JoystickGetDeviceGUID()`.
    fn device_guid(&self, device_index: usize) -> Guid {
        with_item(device_index, |item| item.guid).unwrap_or(Guid::ZERO)
    }

    /// Function to perform the mapping from device index to the instance id
    /// for this index. Translation of `LINUX_JoystickGetDeviceInstanceID()`.
    fn device_instance_id(&self, device_index: usize) -> JoystickID {
        with_item(device_index, |item| item.device_instance).unwrap_or(0)
    }

    /// Translation of `LINUX_JoystickOpen()`.
    fn open(&self, joystick: &mut JoystickData, device_index: usize) -> Result<()> {
        assert_joysticks_locked();

        let Some(item) = with_item(device_index, |item| ItemInfo {
            path: item.path.clone(),
            guid: item.guid,
            vendor: item.vendor,
        }) else {
            return Err(Error::new("No such device"));
        };

        let mut hwdata = HwData::new(joystick.instance_id);
        let mut counts = Counts::default();

        let item_sensor = get_sensor(&item.path);
        // (SDL_SetError will already have been called)
        prepare_joystick_hwdata(&mut hwdata, &mut counts, &item, item_sensor.as_deref())?;

        let instance_id = joystick.instance_id;
        with_state(|s| {
            let item = &mut s.joylist[device_index];
            crate::sdl_assert!(!item.hwdata);
            item.hwdata = true;
            if let Some(path) = &item_sensor {
                if let Some(item_sensor) = s.sensorlist.iter_mut().find(|i| &i.path == path) {
                    crate::sdl_assert!(item_sensor.hwdata.is_none());
                    item_sensor.hwdata = Some(instance_id);
                }
            }
        });

        joystick.serial = udev::product_serial(&item.path);

        // mark joystick as fresh and ready
        hwdata.fresh = true;

        joystick.naxes = counts.naxes;
        joystick.nbuttons = counts.nbuttons;
        joystick.nhats = counts.nhats;
        joystick.nballs = counts.nballs;

        if hwdata.has_gyro {
            joystick.add_sensor(SensorType::Gyro, 0.0);
        }
        if hwdata.has_accelerometer {
            joystick.add_sensor(SensorType::Accel, 0.0);
        }
        if hwdata.fd_sensor >= 0 {
            // Don't keep fd_sensor opened while sensor is disabled
            close_fd(hwdata.fd_sensor);
            hwdata.fd_sensor = -1;
        }

        if hwdata.ff_rumble || hwdata.ff_sine {
            let _ = joystick
                .properties()
                .set(PROP_JOYSTICK_CAP_RUMBLE_BOOLEAN, true);
        }
        with_state(|s| s.open.push(hwdata));
        Ok(())
    }

    /// Translation of `LINUX_JoystickRumble()`.
    fn rumble(
        &self,
        joystick: JoystickID,
        low_frequency_rumble: u16,
        high_frequency_rumble: u16,
    ) -> Result<()> {
        assert_joysticks_locked();

        with_hwdata(joystick, |hwdata| {
            if hwdata.ff_rumble {
                let effect = &mut hwdata.effect;

                effect.type_ = FF_RUMBLE;
                effect.replay.length = super::MAX_RUMBLE_DURATION_MS as u16;
                effect.u.rumble = ff_rumble_effect {
                    strong_magnitude: low_frequency_rumble,
                    weak_magnitude: high_frequency_rumble,
                };
            } else if hwdata.ff_sine {
                // Scale and average the two rumble strengths
                let magnitude = (((low_frequency_rumble / 2) as i32
                    + (high_frequency_rumble / 2) as i32)
                    / 2) as i16;
                let effect = &mut hwdata.effect;

                effect.type_ = FF_PERIODIC;
                effect.replay.length = super::MAX_RUMBLE_DURATION_MS as u16;
                effect.u.periodic.waveform = FF_SINE;
                effect.u.periodic.magnitude = magnitude;
            } else {
                return Err(Error::unsupported());
            }

            // SAFETY: effect is an ff_effect, the size EVIOCSFF encodes.
            let upload = |hwdata: &mut HwData| unsafe {
                ioctl_ptr(
                    hwdata.fd,
                    EVIOCSFF,
                    (&mut hwdata.effect as *mut ff_effect).cast(),
                )
            };
            if upload(hwdata) < 0 {
                // The kernel may have lost this effect, try to allocate a new one
                hwdata.effect.id = -1;
                if upload(hwdata) < 0 {
                    return Err(Error::new(format!(
                        "Couldn't update rumble effect: {}",
                        strerror()
                    )));
                }
            }

            let event = input_event {
                type_: EV_FF,
                code: hwdata.effect.id as u16,
                value: 1,
                ..input_event::default()
            };
            if write_event(hwdata.fd, &event) < 0 {
                return Err(Error::new(format!(
                    "Couldn't start rumble effect: {}",
                    strerror()
                )));
            }
            Ok(())
        })
        .unwrap_or_else(|| Err(Error::invalid_param("joystick")))
    }

    /// Translation of `LINUX_JoystickRumbleTriggers()`.
    fn rumble_triggers(
        &self,
        _joystick: JoystickID,
        _left_rumble: u16,
        _right_rumble: u16,
    ) -> Result<()> {
        Err(Error::unsupported())
    }

    /// Translation of `LINUX_JoystickSetLED()`.
    fn set_led(&self, _joystick: JoystickID, _red: u8, _green: u8, _blue: u8) -> Result<()> {
        Err(Error::unsupported())
    }

    /// Translation of `LINUX_JoystickSendEffect()`.
    fn send_effect(&self, _joystick: JoystickID, _data: &[u8]) -> Result<()> {
        Err(Error::unsupported())
    }

    /// Translation of `LINUX_JoystickSetSensorsEnabled()`.
    fn set_sensors_enabled(&self, joystick: JoystickID, enabled: bool) -> Result<()> {
        assert_joysticks_locked();

        with_hwdata(joystick, |hwdata| {
            if !hwdata.has_accelerometer && !hwdata.has_gyro {
                return Err(Error::unsupported());
            }
            if enabled == hwdata.report_sensor {
                return Ok(());
            }

            if enabled {
                let Some(path) = hwdata.item_sensor.clone() else {
                    return Err(Error::new("Sensors unplugged."));
                };
                hwdata.fd_sensor = open_path(&path, libc::O_RDONLY | libc::O_CLOEXEC);
                if hwdata.fd_sensor < 0 {
                    return Err(Error::new(format!("Couldn't open sensor file {path}.")));
                }
                set_nonblocking(hwdata.fd_sensor);
            } else {
                crate::sdl_assert!(hwdata.fd_sensor >= 0);
                close_fd(hwdata.fd_sensor);
                hwdata.fd_sensor = -1;
            }

            hwdata.report_sensor = enabled;
            Ok(())
        })
        .unwrap_or_else(|| Err(Error::invalid_param("joystick")))
    }

    /// Translation of `LINUX_JoystickUpdate()`.
    fn update(&self, joystick: JoystickID) {
        assert_joysticks_locked();

        // (the events are collected first and sent without the device
        // borrowed, since they may call back into this driver)
        let Some(pending) = with_hwdata(joystick, update_events) else {
            return;
        };
        for event in pending {
            match event {
                Pending::Axis(timestamp, axis, value) => {
                    send_joystick_axis(timestamp, joystick, axis, value)
                }
                Pending::Button(timestamp, button, down) => {
                    send_joystick_button(timestamp, joystick, button, down)
                }
                Pending::Hat(timestamp, hat, value) => {
                    send_joystick_hat(timestamp, joystick, hat, value)
                }
                Pending::Ball(timestamp, ball, xrel, yrel) => {
                    send_joystick_ball(timestamp, joystick, ball, xrel, yrel)
                }
                Pending::Sensor(timestamp, sensor_type, sensor_timestamp, data) => {
                    send_joystick_sensor(timestamp, joystick, sensor_type, sensor_timestamp, &data)
                }
            }
        }
    }

    /// Function to close a joystick after use.
    /// Translation of `LINUX_JoystickClose()`.
    fn close(&self, joystick: &mut JoystickData) {
        assert_joysticks_locked();

        with_state(|s| {
            if let Some(i) = s
                .open
                .iter()
                .position(|h| h.instance_id == joystick.instance_id)
            {
                let mut hwdata = s.open.remove(i);
                close_hwdata(s, &mut hwdata);
            }
        });
    }

    /// Function to perform any system-specific joystick related cleanup.
    /// Translation of `LINUX_JoystickQuit()`.
    fn quit(&self) {
        assert_joysticks_locked();

        let enumeration_method = with_state(|s| {
            if s.inotify_fd >= 0 {
                close_fd(s.inotify_fd);
                s.inotify_fd = -1;
            }

            s.joylist.clear();
            s.sensorlist.clear();
            s.enumeration_method
        });

        if enumeration_method == EnumerationMethod::Libudev {
            udev::del_callback(joystick_udev_callback);
            udev::quit();
        }
    }

    /// This is based on the Linux Gamepad Specification
    /// available at: <https://www.kernel.org/doc/html/v4.15/input/gamepad.html>
    /// and the Android gamepad documentation,
    /// <https://developer.android.com/develop/ui/views/touch-and-input/game-controllers/controller-input>.
    /// Translation of `LINUX_JoystickGetGamepadMapping()`.
    fn gamepad_mapping(&self, device_index: usize) -> Option<GamepadMapping> {
        assert_joysticks_locked();

        let (checked_mapping, mapping, item, driver) = with_item(device_index, |item| {
            (
                item.checked_mapping,
                item.mapping.clone(),
                ItemInfo {
                    path: item.path.clone(),
                    guid: item.guid,
                    vendor: item.vendor,
                },
                item.driver.clone(),
            )
        })?;

        if checked_mapping {
            return mapping;
        }

        /* We temporarily open the device to check how it's configured. Make
        a fake SDL_Joystick object to do so. */
        let mut hwdata = HwData::new(0);
        let mut counts = Counts::default();

        with_item(device_index, |item| item.checked_mapping = true);

        let mut result = None;
        if prepare_joystick_hwdata(&mut hwdata, &mut counts, &item, None).is_ok() {
            // don't assign `item->hwdata` so it's not in any global state.

            // it is now safe to call LINUX_JoystickClose on this fake joystick.

            // (SDL_GetJoystickVendor() of the fake joystick: from its GUID)
            let vendor = joystick_guid_info(item.guid).0;
            result = generate_gamepad_mapping(&hwdata, vendor, driver.as_deref());

            // Cache the mapping for later
            if let Some(out) = &result {
                with_item(device_index, |item| item.mapping = Some(out.clone()));
            }
        }

        with_state(|s| close_hwdata(s, &mut hwdata));

        result
    }
}

/// `write(fd, event, sizeof(*event))`
fn write_event(fd: RawFd, event: &input_event) -> isize {
    // SAFETY: event is a readable input_event of the size written.
    unsafe {
        libc::write(
            fd,
            (event as *const input_event).cast(),
            size_of::<input_event>(),
        )
    }
}

/// The mapping half of `LINUX_JoystickGetGamepadMapping()`: the gamepad
/// layout of a device from its configuration, or `None` if it isn't a
/// gamepad according to the specs.
fn generate_gamepad_mapping(
    hwdata: &HwData,
    vendor: u16,
    driver: Option<&str>,
) -> Option<GamepadMapping> {
    const MAPPED_TRIGGER_LEFT: u32 = 0x1;
    const MAPPED_TRIGGER_RIGHT: u32 = 0x2;
    const MAPPED_TRIGGER_BOTH: u32 = 0x3;

    const MAPPED_DPAD_UP: u32 = 0x1;
    const MAPPED_DPAD_DOWN: u32 = 0x2;
    const MAPPED_DPAD_LEFT: u32 = 0x4;
    const MAPPED_DPAD_RIGHT: u32 = 0x8;
    const MAPPED_DPAD_ALL: u32 = 0xF;

    const MAPPED_LEFT_PADDLE1: u32 = 0x1;
    const MAPPED_RIGHT_PADDLE1: u32 = 0x2;
    const MAPPED_LEFT_PADDLE2: u32 = 0x4;
    const MAPPED_RIGHT_PADDLE2: u32 = 0x8;
    const MAPPED_PADDLE_ALL: u32 = 0xF;

    if !hwdata.has_key[BTN_GAMEPAD] {
        // Not a gamepad according to the specs.
        return None;
    }

    // We have a gamepad, start filling out the mappings
    let mut out = GamepadMapping::default();

    macro_rules! map {
        ($slot:ident, Button, $code:expr) => {
            out.$slot.kind = MappingKind::Button;
            out.$slot.target = hwdata.key_map[$code];
        };
        ($slot:ident, Axis, $code:expr) => {
            out.$slot.kind = MappingKind::Axis;
            out.$slot.target = hwdata.abs_map[$code];
        };
        ($slot:ident, Hat, $target:expr) => {
            out.$slot.kind = MappingKind::Hat;
            out.$slot.target = ($target) as u8;
        };
    }

    if hwdata.has_key[BTN_A] {
        map!(a, Button, BTN_A);
    }

    if hwdata.has_key[BTN_B] {
        map!(b, Button, BTN_B);
    }

    // Xbox controllers use BTN_X and BTN_Y, and PS4 controllers use BTN_WEST and BTN_NORTH
    if vendor == super::USB_VENDOR_SONY {
        if hwdata.has_key[BTN_WEST] {
            map!(x, Button, BTN_WEST);
        }

        if hwdata.has_key[BTN_NORTH] {
            map!(y, Button, BTN_NORTH);
        }
    } else {
        if hwdata.has_key[BTN_X] {
            map!(x, Button, BTN_X);
        }

        if hwdata.has_key[BTN_Y] {
            map!(y, Button, BTN_Y);
        }
    }

    if hwdata.has_key[BTN_SELECT] {
        map!(back, Button, BTN_SELECT);
    }

    if hwdata.has_key[BTN_START] {
        map!(start, Button, BTN_START);
    }

    if hwdata.has_key[BTN_THUMBL] {
        map!(leftstick, Button, BTN_THUMBL);
    }

    if hwdata.has_key[BTN_THUMBR] {
        map!(rightstick, Button, BTN_THUMBR);
    }

    if hwdata.has_key[BTN_MODE] {
        map!(guide, Button, BTN_MODE);
    }

    /*
      According to the specs the D-Pad, the shoulder buttons and the triggers
      can be digital, or analog, or both at the same time.
    */

    // Prefer digital shoulder buttons, but settle for digital or analog hat.
    let mut mapped = 0;

    if hwdata.has_key[BTN_TL] {
        map!(leftshoulder, Button, BTN_TL);
        mapped |= 0x1;
    }

    if hwdata.has_key[BTN_TR] {
        map!(rightshoulder, Button, BTN_TR);
        mapped |= 0x2;
    }

    if mapped != 0x3 && hwdata.has_hat[1] {
        let hat = hwdata.hats_indices[1] << 4;
        map!(leftshoulder, Hat, hat | 0x4);
        map!(rightshoulder, Hat, hat | 0x2);
        mapped |= 0x3;
    }

    if mapped & 0x1 == 0 && hwdata.has_abs[ABS_HAT1Y] {
        map!(leftshoulder, Axis, ABS_HAT1Y);
        mapped |= 0x1;
    }

    if mapped & 0x2 == 0 && hwdata.has_abs[ABS_HAT1X] {
        map!(rightshoulder, Axis, ABS_HAT1X);
        mapped |= 0x2;
    }
    let _ = mapped;

    // Prefer analog triggers, but settle for digital hat or buttons.
    mapped = 0;

    /* Unfortunately there are several conventions for how analog triggers
     * are represented as absolute axes:
     *
     * - Linux Gamepad Specification:
     *   LT = ABS_HAT2Y, RT = ABS_HAT2X
     * - Android (and therefore many Bluetooth controllers):
     *   LT = ABS_BRAKE, RT = ABS_GAS
     * - De facto standard for older Xbox and Playstation controllers:
     *   LT = ABS_Z, RT = ABS_RZ
     *
     * We try each one in turn. */
    if hwdata.has_abs[ABS_HAT2Y] {
        // Linux Gamepad Specification
        map!(lefttrigger, Axis, ABS_HAT2Y);
        mapped |= MAPPED_TRIGGER_LEFT;
    } else if hwdata.has_abs[ABS_BRAKE] {
        // Android convention
        map!(lefttrigger, Axis, ABS_BRAKE);
        mapped |= MAPPED_TRIGGER_LEFT;
    } else if hwdata.has_abs[ABS_Z] {
        // De facto standard for Xbox 360 and Playstation gamepads
        map!(lefttrigger, Axis, ABS_Z);
        mapped |= MAPPED_TRIGGER_LEFT;
    }

    if hwdata.has_abs[ABS_HAT2X] {
        // Linux Gamepad Specification
        map!(righttrigger, Axis, ABS_HAT2X);
        mapped |= MAPPED_TRIGGER_RIGHT;
    } else if hwdata.has_abs[ABS_GAS] {
        // Android convention
        map!(righttrigger, Axis, ABS_GAS);
        mapped |= MAPPED_TRIGGER_RIGHT;
    } else if hwdata.has_abs[ABS_RZ] {
        // De facto standard for Xbox 360 and Playstation gamepads
        map!(righttrigger, Axis, ABS_RZ);
        mapped |= MAPPED_TRIGGER_RIGHT;
    }

    if mapped != MAPPED_TRIGGER_BOTH && hwdata.has_hat[2] {
        let hat = hwdata.hats_indices[2] << 4;
        map!(lefttrigger, Hat, hat | 0x4);
        map!(righttrigger, Hat, hat | 0x2);
        mapped |= MAPPED_TRIGGER_BOTH;
    }

    if mapped & MAPPED_TRIGGER_LEFT == 0 && hwdata.has_key[BTN_TL2] {
        map!(lefttrigger, Button, BTN_TL2);
        mapped |= MAPPED_TRIGGER_LEFT;
    }

    if mapped & MAPPED_TRIGGER_RIGHT == 0 && hwdata.has_key[BTN_TR2] {
        map!(righttrigger, Button, BTN_TR2);
        mapped |= MAPPED_TRIGGER_RIGHT;
    }
    let _ = mapped;

    // Prefer digital D-Pad buttons, but settle for digital or analog hat.
    mapped = 0;

    if hwdata.has_key[BTN_DPAD_UP] {
        map!(dpup, Button, BTN_DPAD_UP);
        mapped |= MAPPED_DPAD_UP;
    }

    if hwdata.has_key[BTN_DPAD_DOWN] {
        map!(dpdown, Button, BTN_DPAD_DOWN);
        mapped |= MAPPED_DPAD_DOWN;
    }

    if hwdata.has_key[BTN_DPAD_LEFT] {
        map!(dpleft, Button, BTN_DPAD_LEFT);
        mapped |= MAPPED_DPAD_LEFT;
    }

    if hwdata.has_key[BTN_DPAD_RIGHT] {
        map!(dpright, Button, BTN_DPAD_RIGHT);
        mapped |= MAPPED_DPAD_RIGHT;
    }

    if mapped != MAPPED_DPAD_ALL {
        if hwdata.has_hat[0] {
            let hat = hwdata.hats_indices[0] << 4;
            map!(dpleft, Hat, hat | 0x8);
            map!(dpright, Hat, hat | 0x2);
            map!(dpup, Hat, hat | 0x1);
            map!(dpdown, Hat, hat | 0x4);
        } else if hwdata.has_abs[ABS_HAT0X] && hwdata.has_abs[ABS_HAT0Y] {
            map!(dpleft, Axis, ABS_HAT0X);
            map!(dpright, Axis, ABS_HAT0X);
            map!(dpup, Axis, ABS_HAT0Y);
            map!(dpdown, Axis, ABS_HAT0Y);
        } else if driver == Some("xpad") {
            // xpad will sometimes map the D-Pad as BTN_TRIGGER_HAPPY1 - BTN_TRIGGER_HAPPY4
            if hwdata.has_key[BTN_TRIGGER_HAPPY1]
                && hwdata.has_key[BTN_TRIGGER_HAPPY2]
                && hwdata.has_key[BTN_TRIGGER_HAPPY3]
                && hwdata.has_key[BTN_TRIGGER_HAPPY4]
            {
                map!(dpleft, Button, BTN_TRIGGER_HAPPY1);
                map!(dpright, Button, BTN_TRIGGER_HAPPY2);
                map!(dpup, Button, BTN_TRIGGER_HAPPY3);
                map!(dpdown, Button, BTN_TRIGGER_HAPPY4);
            }
        }
    }

    if hwdata.has_abs[ABS_X] && hwdata.has_abs[ABS_Y] {
        map!(leftx, Axis, ABS_X);
        map!(lefty, Axis, ABS_Y);
    }

    /* The Linux Gamepad Specification uses the RX and RY axes,
     * originally intended to represent X and Y rotation, as a second
     * joystick. This is common for USB gamepads, and also many Bluetooth
     * gamepads, particularly older ones.
     *
     * The Android mapping convention used by many Bluetooth controllers
     * instead uses the Z axis as a secondary X axis, and the RZ axis as
     * a secondary Y axis. */
    if hwdata.has_abs[ABS_RX] && hwdata.has_abs[ABS_RY] {
        // Linux Gamepad Specification, Xbox 360, Playstation etc.
        map!(rightx, Axis, ABS_RX);
        map!(righty, Axis, ABS_RY);
    } else if hwdata.has_abs[ABS_Z] && hwdata.has_abs[ABS_RZ] {
        // Android convention
        map!(rightx, Axis, ABS_Z);
        map!(righty, Axis, ABS_RZ);
    }

    mapped = 0;

    if hwdata.has_key[BTN_GRIPR] {
        map!(right_paddle1, Button, BTN_GRIPR);
        mapped |= MAPPED_RIGHT_PADDLE1;
    }
    if hwdata.has_key[BTN_GRIPL] {
        map!(left_paddle1, Button, BTN_GRIPL);
        mapped |= MAPPED_LEFT_PADDLE1;
    }
    if hwdata.has_key[BTN_GRIPR2] {
        map!(right_paddle2, Button, BTN_GRIPR2);
        mapped |= MAPPED_RIGHT_PADDLE2;
    }
    if hwdata.has_key[BTN_GRIPL2] {
        map!(left_paddle2, Button, BTN_GRIPL2);
        mapped |= MAPPED_LEFT_PADDLE2;
    }

    if mapped != MAPPED_PADDLE_ALL && vendor == super::USB_VENDOR_MICROSOFT {
        // The Xbox Elite controllers have the paddles as BTN_TRIGGER_HAPPY5 - BTN_TRIGGER_HAPPY8
        // in older drivers
        if hwdata.has_key[BTN_TRIGGER_HAPPY5]
            && hwdata.has_key[BTN_TRIGGER_HAPPY6]
            && hwdata.has_key[BTN_TRIGGER_HAPPY7]
            && hwdata.has_key[BTN_TRIGGER_HAPPY8]
        {
            map!(right_paddle1, Button, BTN_TRIGGER_HAPPY5);
            map!(left_paddle1, Button, BTN_TRIGGER_HAPPY7);
            map!(right_paddle2, Button, BTN_TRIGGER_HAPPY6);
            map!(left_paddle2, Button, BTN_TRIGGER_HAPPY8);
        }
    }

    // Xbox Series controllers have the Share button as KEY_RECORD
    if hwdata.has_key[KEY_RECORD] {
        map!(misc1, Button, KEY_RECORD);
    }

    Some(out)
}
