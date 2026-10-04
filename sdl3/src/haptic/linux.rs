// Rust translation of src/haptic/linux/SDL_syshaptic.c from Simple
// DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Linux haptic driver: the kernel's force feedback interface on evdev
//! nodes (`/dev/input/event*`), found by probing the first 32 nodes and
//! then through udev.

use std::ffi::{c_int, CString};
use std::os::fd::RawFd;
use std::sync::Mutex;

use super::{
    HapticConditionKind, HapticData, HapticDirection, HapticDirectionType, HapticDriver,
    HapticEffect, HapticEffectSlot, HapticFeatures, HapticID, HapticWaveform, HAPTIC_INFINITY,
};
use crate::core::linux::evdev_capabilities::{test_bit, DeviceClass};
use crate::core::linux::input::*;
use crate::core::linux::udev::{self, UdevDeviceEvent};
use crate::error::{Error, Result};
use crate::joystick::Joystick;

/// It's doubtful someone has more then 32 evdev. Translation of `MAX_HAPTICS`.
const MAX_HAPTICS: usize = 32;

/// An available haptic device. Translation of `SDL_hapticlist_item`.
struct HapticlistItem {
    instance_id: HapticID,
    /// Dev path name (like /dev/input/event1)
    fname: String,
    // (the associated haptic, `haptic`, is never set)
    dev_num: libc::dev_t,
}

/// Haptic system hardware data. Translation of `struct haptic_hwdata`.
struct HapticHwData {
    /// File descriptor of the device.
    fd: RawFd,
    /// The device path (upstream's points to the name in SDL_hapticlist).
    fname: String,
}

/// Haptic system effect data. Translation of `struct haptic_hweffect`.
struct HapticHwEffect {
    /// The linux kernel effect structure.
    effect: ff_effect,
}

/// List of available haptic devices. Translation of `SDL_hapticlist`
/// (`numhaptics` is its length).
static HAPTICLIST: Mutex<Vec<HapticlistItem>> = Mutex::new(Vec::new());

fn with_list<R>(f: impl FnOnce(&mut Vec<HapticlistItem>) -> R) -> R {
    f(&mut HAPTICLIST.lock().unwrap_or_else(|e| e.into_inner()))
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

/// `write(fd, &event, sizeof(event))` of an `EV_FF` event.
fn write_ff(fd: RawFd, code: u16, value: i32) -> isize {
    let event = input_event {
        type_: EV_FF,
        code,
        value,
        ..input_event::default()
    };
    // SAFETY: event is a readable input_event of the size written.
    unsafe {
        libc::write(
            fd,
            (&event as *const input_event).cast(),
            size_of::<input_event>(),
        )
    }
}

/// The driver data of an open device.
fn hwdata(haptic: &HapticData) -> &HapticHwData {
    haptic
        .hwdata
        .as_ref()
        .and_then(|h| h.downcast_ref::<HapticHwData>())
        .expect("an open Linux haptic device")
}

/// The kernel effect in an effect slot.
fn hweffect(slot: &mut HapticEffectSlot) -> &mut HapticHwEffect {
    slot.hweffect
        .as_mut()
        .and_then(|h| h.downcast_mut::<HapticHwEffect>())
        .expect("a Linux haptic effect")
}

/// Test whether a device has haptic properties. Returns available
/// properties or nothing if there are none. Translation of `EV_IsHaptic()`.
fn ev_is_haptic(fd: RawFd) -> HapticFeatures {
    let mut features = [0 as std::ffi::c_ulong; 1 + FF_MAX / size_of::<std::ffi::c_ulong>()];
    let mut ret = HapticFeatures::NONE;

    // Ask device for what it has.
    // SAFETY: the request's size is the buffer's size.
    if unsafe {
        ioctl_ptr(
            fd,
            eviocgbit(EV_FF, std::mem::size_of_val(&features)),
            features.as_mut_ptr().cast(),
        )
    } < 0
    {
        // ("Haptic: Unable to get device's features: %s", which no caller
        // reports)
        return HapticFeatures::NONE;
    }

    // Convert supported features to SDL_HAPTIC platform-neutral features.
    let mut ev_test = |ev: u16, f: HapticFeatures| {
        if test_bit(ev as usize, &features) {
            ret |= f;
        }
    };
    ev_test(FF_CONSTANT, HapticFeatures::CONSTANT);
    ev_test(FF_SINE, HapticFeatures::SINE);
    ev_test(FF_SQUARE, HapticFeatures::SQUARE);
    ev_test(FF_TRIANGLE, HapticFeatures::TRIANGLE);
    ev_test(FF_SAW_UP, HapticFeatures::SAWTOOTHUP);
    ev_test(FF_SAW_DOWN, HapticFeatures::SAWTOOTHDOWN);
    ev_test(FF_RAMP, HapticFeatures::RAMP);
    ev_test(FF_SPRING, HapticFeatures::SPRING);
    ev_test(FF_FRICTION, HapticFeatures::FRICTION);
    ev_test(FF_DAMPER, HapticFeatures::DAMPER);
    ev_test(FF_INERTIA, HapticFeatures::INERTIA);
    ev_test(FF_CUSTOM, HapticFeatures::CUSTOM);
    ev_test(FF_GAIN, HapticFeatures::GAIN);
    ev_test(FF_AUTOCENTER, HapticFeatures::AUTOCENTER);
    ev_test(FF_RUMBLE, HapticFeatures::LEFTRIGHT);

    // Return what it supports.
    ret
}

/// Tests whether a device is a mouse or not. Translation of `EV_IsMouse()`.
fn ev_is_mouse(fd: RawFd) -> bool {
    let mut argp = [0 as std::ffi::c_ulong; 40];

    // Ask for supported features.
    // SAFETY: the request's size is the buffer's size.
    if unsafe {
        ioctl_ptr(
            fd,
            eviocgbit(EV_KEY, std::mem::size_of_val(&argp)),
            argp.as_mut_ptr().cast(),
        )
    } < 0
    {
        return false;
    }

    keys_are_mouse(&argp)
}

/// The test of `EV_IsMouse()` on the device's key bits. Upstream's returns
/// true on both branches, so every device whose keys can be read is taken
/// for a mouse; a device without `BTN_MOUSE` isn't one.
fn keys_are_mouse(argp: &[std::ffi::c_ulong]) -> bool {
    // Currently we only test for BTN_MOUSE which can give fake positives.
    test_bit(BTN_MOUSE, argp)
}

/// Translation of `haptic_udev_callback()`.
fn haptic_udev_callback(udev_type: UdevDeviceEvent, udev_class: DeviceClass, devpath: &str) {
    if !udev_class.intersects(DeviceClass::JOYSTICK) {
        return;
    }

    match udev_type {
        UdevDeviceEvent::Added => {
            maybe_add_device(devpath);
        }

        UdevDeviceEvent::Removed => {
            maybe_remove_device(devpath);
        }
    }
}

/// Translation of `MaybeAddDevice()`.
fn maybe_add_device(path: &str) -> bool {
    // try to open
    let fd = open_path(path, libc::O_RDWR | libc::O_CLOEXEC);
    if fd < 0 {
        return false;
    }

    // get file status
    // SAFETY: an all-zero stat is a valid value to be overwritten.
    let mut sb: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: fd is open and sb is a writable stat.
    if unsafe { libc::fstat(fd, &mut sb) } != 0 {
        close_fd(fd);
        return false;
    }

    // check for duplicates
    if with_list(|l| l.iter().any(|item| item.dev_num == sb.st_rdev)) {
        close_fd(fd);
        return false; // duplicate.
    }

    // see if it works
    let supported = ev_is_haptic(fd);
    close_fd(fd);
    if supported.is_empty() {
        return false;
    }

    let item = HapticlistItem {
        instance_id: crate::utils::next_object_id(),
        fname: path.to_owned(),
        dev_num: sb.st_rdev,
    };

    // TODO: should we add instance IDs?
    with_list(|l| l.push(item));

    // !!! TODO: Send a haptic add event?

    true
}

/// Translation of `MaybeRemoveDevice()`.
fn maybe_remove_device(path: &str) -> bool {
    with_list(|l| {
        // found it, remove it.
        if let Some(i) = l.iter().position(|item| path == item.fname) {
            // (whether a haptic was associated: never, it isn't set)
            let result = false;

            // Need to decrement the haptic count
            l.remove(i);
            // !!! TODO: Send a haptic remove event?

            return result;
        }
        false
    })
}

/// Gets the name from a file descriptor. Translation of
/// `SDL_SYS_HapticNameFromFD()`.
fn haptic_name_from_fd(fd: RawFd) -> Option<String> {
    let mut namebuf = [0u8; 128];

    // We use the evdev name ioctl.
    // SAFETY: the request's size is namebuf's size.
    if unsafe { ioctl_ptr(fd, eviocgname(namebuf.len()), namebuf.as_mut_ptr().cast()) } <= 0 {
        return None;
    }

    let end = namebuf
        .iter()
        .position(|&b| b == 0)
        .unwrap_or(namebuf.len());
    Some(String::from_utf8_lossy(&namebuf[..end]).into_owned())
}

/// Opens the haptic device from the file descriptor, which is closed on
/// error. Translation of `SDL_SYS_HapticOpenFromFD()`.
fn haptic_open_from_fd(haptic: &mut HapticData, fd: RawFd) -> Result<()> {
    // Set the data.
    haptic.supported = ev_is_haptic(fd);
    haptic.naxes = 2; // Hardcoded for now, not sure if it's possible to find out.

    // Set the effects
    let mut neffects: c_int = 0;
    if ioctl_read(fd, EVIOCGEFFECTS, &mut neffects) < 0 {
        let error = Error::new(format!(
            "Haptic: Unable to query device memory: {}",
            strerror()
        ));
        close_fd(fd);
        return Err(error);
    }
    haptic.nplaying = neffects; // Linux makes no distinction.
    haptic.effects = (0..neffects.max(0))
        .map(|_| HapticEffectSlot::default())
        .collect();

    haptic.hwdata = Some(Box::new(HapticHwData {
        fd,
        fname: String::new(),
    }));
    Ok(())
}

/// The path of an open device (`haptic->hwdata->fname`).
fn set_fname(haptic: &mut HapticData, fname: &str) {
    if let Some(h) = haptic
        .hwdata
        .as_mut()
        .and_then(|h| h.downcast_mut::<HapticHwData>())
    {
        h.fname = fname.to_owned();
    }
}

/// Converts an SDL button to a ff_trigger button.
/// Translation of `SDL_SYS_ToButton()`.
fn to_button(button: u16) -> u16 {
    let mut ff_button = 0;

    /*
     * Not sure what the proper syntax is because this actually isn't implemented
     * in the current kernel from what I've seen (2.6.26).
     */
    if button != 0 {
        ff_button = (BTN_GAMEPAD as u16).wrapping_add(button).wrapping_sub(1);
    }

    ff_button
}

/// Initializes the ff_effect usable direction from a SDL_HapticDirection.
/// Translation of `SDL_SYS_ToDirection()`.
fn to_direction(src: &HapticDirection) -> u16 {
    match src.kind {
        HapticDirectionType::Polar => {
            let tmp = (((src.dir[0] % 36000).wrapping_mul(0x8000)) / 18000) as u32; // convert to range [0,0xFFFF]
            tmp as u16
        }

        HapticDirectionType::Spherical => {
            /*
                We convert to polar, because that's the only supported direction on Linux.
                The first value of a spherical direction is practically the same as a
                Polar direction, except that we have to add 90 degrees. It is the angle
                from EAST {1,0} towards SOUTH {0,1}.
                --> add 9000
                --> finally convert to [0,0xFFFF] as in case SDL_HAPTIC_POLAR.
            */
            let mut tmp = (src.dir[0].wrapping_add(9000) % 36000) as u32; // Convert to polars
            tmp = tmp.wrapping_mul(0x8000) / 18000; // convert to range [0,0xFFFF]
            tmp as u16
        }

        HapticDirectionType::Cartesian => {
            if src.dir[1] == 0 {
                if src.dir[0] >= 0 {
                    0x4000
                } else {
                    0xC000
                }
            } else if src.dir[0] == 0 {
                if src.dir[1] >= 0 {
                    0x8000
                } else {
                    0
                }
            } else {
                let f = crate::stdlib::math::atan2f(src.dir[1] as f32, src.dir[0] as f32); // Ideally we'd use fixed point math instead of floats...
                                                                                           /*
                                                                                             SDL_atan2 takes the parameters: Y-axis-value and X-axis-value (in that order)
                                                                                              - Y-axis-value is the second coordinate (from center to SOUTH)
                                                                                              - X-axis-value is the first coordinate (from center to EAST)
                                                                                               We add 36000, because SDL_atan2 also returns negative values. Then we practically
                                                                                               have the first spherical value. Therefore we proceed as in case
                                                                                               SDL_HAPTIC_SPHERICAL and add another 9000 to get the polar value.
                                                                                             --> add 45000 in total
                                                                                             --> finally convert to [0,0xFFFF] as in case SDL_HAPTIC_POLAR.
                                                                                           */
                let mut tmp = (((f as f64 * 18000.0 / std::f64::consts::PI) as i32)
                    .wrapping_add(45000)
                    % 36000) as u32;
                tmp = tmp.wrapping_mul(0x8000) / 18000; // convert to range [0,0xFFFF]
                tmp as u16
            }
        }
        HapticDirectionType::SteeringAxis => 0x4000,
    }
}

/// `CLAMP(x)`: values above 32767 (for unsigned) are unspecified so we
/// must clamp.
fn clamp(x: u32) -> u16 {
    if x > 32767 {
        32767
    } else {
        x as u16
    }
}

/// The replay length of an effect (infinity is 0 for the kernel).
fn replay_length(length: u32) -> u16 {
    if length == HAPTIC_INFINITY {
        0
    } else {
        clamp(length)
    }
}

/// Initializes the Linux effect struct from a haptic_effect.
/// Values above 32767 (for unsigned) are unspecified so we must clamp.
/// Translation of `SDL_SYS_ToFFEffect()`.
fn to_ff_effect(src: &HapticEffect) -> Result<ff_effect> {
    // Clear up
    let mut dest = ff_effect::zeroed();

    let envelope =
        |attack_length: u16, attack_level: u16, fade_length: u16, fade_level: u16| ff_envelope {
            attack_length: clamp(attack_length as u32),
            attack_level: clamp(attack_level as u32),
            fade_length: clamp(fade_length as u32),
            fade_level: clamp(fade_level as u32),
        };

    match src {
        HapticEffect::Constant(constant) => {
            // Header
            dest.type_ = FF_CONSTANT;
            dest.direction = to_direction(&constant.direction);

            // Replay
            dest.replay.length = replay_length(constant.length);
            dest.replay.delay = clamp(constant.delay as u32);

            // Trigger
            dest.trigger.button = to_button(constant.button);
            dest.trigger.interval = clamp(constant.interval as u32);

            // Constant
            dest.u.constant = ff_constant_effect {
                level: constant.level,

                // Envelope
                envelope: envelope(
                    constant.attack_length,
                    constant.attack_level,
                    constant.fade_length,
                    constant.fade_level,
                ),
            };
        }

        HapticEffect::Periodic(periodic) => {
            // Header
            dest.type_ = FF_PERIODIC;
            dest.direction = to_direction(&periodic.direction);

            // Replay
            dest.replay.length = replay_length(periodic.length);
            dest.replay.delay = clamp(periodic.delay as u32);

            // Trigger
            dest.trigger.button = to_button(periodic.button);
            dest.trigger.interval = clamp(periodic.interval as u32);

            // Periodic
            let waveform = match periodic.waveform {
                HapticWaveform::Sine => FF_SINE,
                HapticWaveform::Square => FF_SQUARE,
                HapticWaveform::Triangle => FF_TRIANGLE,
                HapticWaveform::SawtoothUp => FF_SAW_UP,
                HapticWaveform::SawtoothDown => FF_SAW_DOWN,
            };
            dest.u.periodic = ff_periodic_effect {
                waveform,
                period: clamp(periodic.period as u32),
                magnitude: periodic.magnitude,
                offset: periodic.offset,
                // Linux phase is defined in interval "[0x0000, 0x10000[", corresponds with "[0deg, 360deg[" phase shift.
                phase: ((periodic.phase as u32 * 0x10000) / 36000) as u16,

                // Envelope
                envelope: envelope(
                    periodic.attack_length,
                    periodic.attack_level,
                    periodic.fade_length,
                    periodic.fade_level,
                ),
                custom_len: 0,
                custom_data: std::ptr::null_mut(),
            };
        }

        HapticEffect::Condition(condition) => {
            // Header
            dest.type_ = match condition.kind {
                HapticConditionKind::Spring => FF_SPRING,
                HapticConditionKind::Damper => FF_DAMPER,
                HapticConditionKind::Inertia => FF_INERTIA,
                HapticConditionKind::Friction => FF_FRICTION,
            };

            dest.direction = to_direction(&condition.direction);

            // Replay
            dest.replay.length = replay_length(condition.length);
            dest.replay.delay = clamp(condition.delay as u32);

            // Trigger
            dest.trigger.button = to_button(condition.button);
            dest.trigger.interval = clamp(condition.interval as u32);

            // Condition
            // X axis, Y axis
            let axis = |i: usize| ff_condition_effect {
                right_saturation: condition.right_sat[i],
                left_saturation: condition.left_sat[i],
                right_coeff: condition.right_coeff[i],
                left_coeff: condition.left_coeff[i],
                deadband: condition.deadband[i],
                center: condition.center[i],
            };
            dest.u.condition = [axis(0), axis(1)];

            /*
             * There is no envelope in the linux force feedback api for conditions.
             */
        }

        HapticEffect::Ramp(ramp) => {
            // Header
            dest.type_ = FF_RAMP;
            dest.direction = to_direction(&ramp.direction);

            // Replay
            dest.replay.length = replay_length(ramp.length);
            dest.replay.delay = clamp(ramp.delay as u32);

            // Trigger
            dest.trigger.button = to_button(ramp.button);
            dest.trigger.interval = clamp(ramp.interval as u32);

            // Ramp
            dest.u.ramp = ff_ramp_effect {
                start_level: ramp.start,
                end_level: ramp.end,

                // Envelope
                envelope: envelope(
                    ramp.attack_length,
                    ramp.attack_level,
                    ramp.fade_length,
                    ramp.fade_level,
                ),
            };
        }

        HapticEffect::LeftRight(leftright) => {
            // Header
            dest.type_ = FF_RUMBLE;
            dest.direction = 0x4000;

            // Replay
            dest.replay.length = replay_length(leftright.length);

            // Trigger
            dest.trigger.button = 0;
            dest.trigger.interval = 0;

            // Rumble (Linux expects 0-65535, so multiply by 2)
            dest.u.rumble = ff_rumble_effect {
                strong_magnitude: clamp(leftright.large_magnitude as u32) * 2,
                weak_magnitude: clamp(leftright.small_magnitude as u32) * 2,
            };
        }

        HapticEffect::Custom(_) => {
            return Err(Error::new("Haptic: Unknown effect type."));
        }
    }

    Ok(dest)
}

/// `ioctl(fd, EVIOCSFF, effect)`
fn upload_effect(fd: RawFd, effect: &mut ff_effect) -> c_int {
    // SAFETY: effect is an ff_effect, the size EVIOCSFF encodes; the kernel
    // writes the effect's id back.
    unsafe { ioctl_ptr(fd, EVIOCSFF, (effect as *mut ff_effect).cast()) }
}

/// The Linux haptic driver (the `SDL_SYS_Haptic*()` functions of
/// `SDL_syshaptic.c`).
pub(super) struct LinuxHapticDriver;

pub(super) static LINUX_HAPTIC_DRIVER: LinuxHapticDriver = LinuxHapticDriver;

impl HapticDriver for LinuxHapticDriver {
    /// Initializes the haptic subsystem by finding available devices.
    /// Translation of `SDL_SYS_HapticInit()`.
    fn init(&self) -> Result<()> {
        /*
         * Limit amount of checks to MAX_HAPTICS since we may or may not have
         * permission to some or all devices.
         */
        for i in 0..MAX_HAPTICS {
            let path = format!("/dev/input/event{i}");
            maybe_add_device(&path);
        }

        if udev::init().is_err() {
            return Err(Error::new("Could not initialize UDEV"));
        }

        if udev::add_callback(haptic_udev_callback).is_err() {
            udev::quit();
            return Err(Error::new("Could not setup haptic <-> udev callback"));
        }

        // Force a scan to build the initial device list
        let _ = udev::scan();

        Ok(())
    }

    /// Translation of `SDL_SYS_NumHaptics()`.
    fn count(&self) -> usize {
        with_list(|l| l.len())
    }

    /// Return the instance ID of a haptic device, does not need to be opened.
    /// Translation of `SDL_SYS_HapticInstanceID()`.
    fn instance_id(&self, index: usize) -> HapticID {
        with_list(|l| l.get(index).map_or(0, |item| item.instance_id))
    }

    /// Return the name of a haptic device, does not need to be opened.
    /// Translation of `SDL_SYS_HapticName()`.
    fn name(&self, index: usize) -> Option<String> {
        let fname = with_list(|l| l.get(index).map(|item| item.fname.clone()))?;

        // Open the haptic device.
        let fd = open_path(&fname, libc::O_RDONLY | libc::O_CLOEXEC);
        if fd < 0 {
            return None;
        }

        // No name found, return device character device
        let name = haptic_name_from_fd(fd).unwrap_or(fname);
        close_fd(fd);
        Some(name)
    }

    /// Opens a haptic device for usage. Translation of `SDL_SYS_HapticOpen()`.
    fn open(&self, haptic: &mut HapticData) -> Result<()> {
        let fname = with_list(|l| {
            l.iter()
                .find(|item| item.instance_id == haptic.instance_id)
                .map(|item| item.fname.clone())
        })
        // (HapticByInstanceID(); the front end has checked that it is there)
        .ok_or_else(|| Error::new(format!("Haptic device {} not found", haptic.instance_id)))?;
        // Open the character device
        let fd = open_path(&fname, libc::O_RDWR | libc::O_CLOEXEC);
        if fd < 0 {
            return Err(Error::new(format!(
                "Haptic: Unable to open {fname}: {}",
                strerror()
            )));
        }

        // Try to create the haptic.
        haptic_open_from_fd(haptic, fd)?; // Already closes on error.

        // Set the fname.
        set_fname(haptic, &fname);
        Ok(())
    }

    /// Opens a haptic device from first mouse it finds for usage.
    /// Translation of `SDL_SYS_HapticMouse()`.
    fn mouse(&self) -> Option<usize> {
        let fnames: Vec<String> = with_list(|l| l.iter().map(|item| item.fname.clone()).collect());
        for (device_index, fname) in fnames.iter().enumerate() {
            // Open the device.
            let fd = open_path(fname, libc::O_RDWR | libc::O_CLOEXEC);
            if fd < 0 {
                // (upstream returns SDL_SetError(), which is false, i.e.
                // device index 0: a device that can't be opened isn't the
                // mouse, so report none, the -1 upstream means)
                return None;
            }

            // Is it a mouse?
            if ev_is_mouse(fd) {
                close_fd(fd);
                return Some(device_index);
            }

            close_fd(fd);
        }

        None
    }

    /// Checks to see if a joystick has haptic features.
    /// Translation of `SDL_SYS_JoystickIsHaptic()`.
    fn joystick_is_haptic(&self, joystick: &Joystick) -> bool {
        crate::joystick::assert_joysticks_locked();

        let Some((fd, _)) = crate::joystick::linux::joystick_fd_and_fname(joystick) else {
            return false;
        };
        !ev_is_haptic(fd).is_empty()
    }

    /// Opens a SDL_Haptic from a SDL_Joystick.
    /// Translation of `SDL_SYS_HapticOpenFromJoystick()`.
    fn open_from_joystick(&self, haptic: &mut HapticData, joystick: &Joystick) -> Result<()> {
        crate::joystick::assert_joysticks_locked();

        let Some((_, joystick_fname)) = crate::joystick::linux::joystick_fd_and_fname(joystick)
        else {
            return Err(Error::new("Haptic: joystick isn't a Linux joystick"));
        };
        // Find the joystick in the haptic list.
        if let Some(instance_id) = with_list(|l| {
            l.iter()
                .find(|item| item.fname == joystick_fname)
                .map(|item| item.instance_id)
        }) {
            haptic.instance_id = instance_id;
        }

        let fd = open_path(&joystick_fname, libc::O_RDWR | libc::O_CLOEXEC);
        if fd < 0 {
            return Err(Error::new(format!(
                "Haptic: Unable to open {joystick_fname}: {}",
                strerror()
            )));
        }
        haptic_open_from_fd(haptic, fd)?; // Already closes on error.

        set_fname(haptic, &joystick_fname);

        if let Some(name) = haptic_name_from_fd(fd) {
            haptic.name = Some(name);
        }
        Ok(())
    }

    /// Checks to see if the haptic device and joystick are in reality the
    /// same. Translation of `SDL_SYS_JoystickSameHaptic()`.
    fn joystick_same_haptic(&self, haptic: &HapticData, joystick: &Joystick) -> bool {
        crate::joystick::assert_joysticks_locked();

        let Some((_, joystick_fname)) = crate::joystick::linux::joystick_fd_and_fname(joystick)
        else {
            return false;
        };
        /* We are assuming Linux is using evdev which should trump the old
         * joystick methods. */
        joystick_fname == hwdata(haptic).fname
    }

    /// Closes the haptic device. Translation of `SDL_SYS_HapticClose()`.
    fn close(&self, haptic: &mut HapticData) {
        if haptic.hwdata.is_some() {
            // Free effects.
            haptic.effects.clear();

            // Clean up
            close_fd(hwdata(haptic).fd);

            // Free
            haptic.hwdata = None;
        }

        // Clear the rest. (SDL_zerop(haptic): the front end drops the
        // device next.)
        haptic.nplaying = 0;
        haptic.supported = HapticFeatures::NONE;
        haptic.naxes = 0;
    }

    /// Clean up after system specific haptic stuff.
    /// Translation of `SDL_SYS_HapticQuit()`.
    fn quit(&self) {
        /* Opened and not closed haptics are leaked, this is on purpose.
         * Close your haptic devices after usage. */
        with_list(|l| l.clear());

        udev::del_callback(haptic_udev_callback);
        udev::quit();
    }

    /// Creates a new haptic effect. Translation of `SDL_SYS_HapticNewEffect()`.
    fn new_effect(
        &self,
        haptic: &mut HapticData,
        effect: usize,
        base: &HapticEffect,
    ) -> Result<()> {
        // Prepare the ff_effect
        let mut linux_effect = to_ff_effect(base)?;
        linux_effect.id = -1; // Have the kernel give it an id

        // Upload the effect
        if upload_effect(hwdata(haptic).fd, &mut linux_effect) < 0 {
            return Err(Error::new(format!(
                "Haptic: Error uploading effect to the device: {}",
                strerror()
            )));
        }

        haptic.effects[effect].hweffect = Some(Box::new(HapticHwEffect {
            effect: linux_effect,
        }));
        Ok(())
    }

    /// Updates an effect.
    ///
    /// Note: Dynamically updating the direction can in some cases force
    /// the effect to restart and run once.
    /// Translation of `SDL_SYS_HapticUpdateEffect()`.
    fn update_effect(
        &self,
        haptic: &mut HapticData,
        effect: usize,
        data: &HapticEffect,
    ) -> Result<()> {
        // Create the new effect
        let mut linux_effect = to_ff_effect(data)?;
        linux_effect.id = hweffect(&mut haptic.effects[effect]).effect.id;

        // See if it can be uploaded.
        if upload_effect(hwdata(haptic).fd, &mut linux_effect) < 0 {
            return Err(Error::new(format!(
                "Haptic: Error updating the effect: {}",
                strerror()
            )));
        }

        // Copy the new effect into memory.
        hweffect(&mut haptic.effects[effect]).effect = linux_effect;

        Ok(())
    }

    /// Runs an effect. Translation of `SDL_SYS_HapticRunEffect()`.
    fn run_effect(&self, haptic: &mut HapticData, effect: usize, iterations: u32) -> Result<()> {
        // Prepare to run the effect
        let code = hweffect(&mut haptic.effects[effect]).effect.id as u16;
        // We don't actually have infinity here, so we just do INT_MAX which is pretty damn close.
        let value = iterations.min(i32::MAX as u32) as i32;

        if write_ff(hwdata(haptic).fd, code, value) < 0 {
            return Err(Error::new(format!(
                "Haptic: Unable to run the effect: {}",
                strerror()
            )));
        }

        Ok(())
    }

    /// Stops an effect. Translation of `SDL_SYS_HapticStopEffect()`.
    fn stop_effect(&self, haptic: &mut HapticData, effect: usize) -> Result<()> {
        let code = hweffect(&mut haptic.effects[effect]).effect.id as u16;

        if write_ff(hwdata(haptic).fd, code, 0) < 0 {
            return Err(Error::new(format!(
                "Haptic: Unable to stop the effect: {}",
                strerror()
            )));
        }

        Ok(())
    }

    /// Frees the effect. Translation of `SDL_SYS_HapticDestroyEffect()`.
    fn destroy_effect(&self, haptic: &mut HapticData, effect: usize) {
        let id = hweffect(&mut haptic.effects[effect]).effect.id;
        // ("Haptic: Error removing the effect from the device: %s" when this
        // fails, which nobody reads)
        let _ = ioctl_int(hwdata(haptic).fd, EVIOCRMFF, id as c_int);
        haptic.effects[effect].hweffect = None;
    }

    /// Gets the status of a haptic effect (not supported atm).
    /// Translation of `SDL_SYS_HapticGetEffectStatus()`.
    fn effect_status(&self, _haptic: &mut HapticData, _effect: usize) -> Result<bool> {
        Err(Error::unsupported())
    }

    /// Sets the gain. Translation of `SDL_SYS_HapticSetGain()`.
    fn set_gain(&self, haptic: &mut HapticData, gain: i32) -> Result<()> {
        let value = ((0xFFFF_u64.wrapping_mul(gain as u64)) / 100) as i32;

        if write_ff(hwdata(haptic).fd, FF_GAIN, value) < 0 {
            return Err(Error::new(format!(
                "Haptic: Error setting gain: {}",
                strerror()
            )));
        }

        Ok(())
    }

    /// Sets the autocentering. Translation of `SDL_SYS_HapticSetAutocenter()`.
    fn set_autocenter(&self, haptic: &mut HapticData, autocenter: i32) -> Result<()> {
        let value = ((0xFFFF_u64.wrapping_mul(autocenter as u64)) / 100) as i32;

        if write_ff(hwdata(haptic).fd, FF_AUTOCENTER, value) < 0 {
            return Err(Error::new(format!(
                "Haptic: Error setting autocenter: {}",
                strerror()
            )));
        }

        Ok(())
    }

    /// Pausing is not supported atm by linux.
    /// Translation of `SDL_SYS_HapticPause()`.
    fn pause(&self, _haptic: &mut HapticData) -> Result<()> {
        Err(Error::unsupported())
    }

    /// Unpausing is not supported atm by linux.
    /// Translation of `SDL_SYS_HapticResume()`.
    fn resume(&self, _haptic: &mut HapticData) -> Result<()> {
        Err(Error::unsupported())
    }

    /// Stops all the currently playing effects.
    /// Translation of `SDL_SYS_HapticStopAll()`.
    fn stop_all(&self, haptic: &mut HapticData) -> Result<()> {
        // Linux does not support this natively so we have to loop.
        for i in 0..haptic.effects.len() {
            if haptic.effects[i].hweffect.is_some() && self.stop_effect(haptic, i).is_err() {
                return Err(Error::new(
                    "Haptic: Error while trying to stop all playing effects.",
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::{
        HapticCondition, HapticConstant, HapticLeftRight, HapticPeriodic, HapticRamp,
    };
    use super::*;

    fn polar(dir: i32) -> HapticDirection {
        HapticDirection {
            kind: HapticDirectionType::Polar,
            dir: [dir, 0, 0],
        }
    }

    #[test]
    fn directions() {
        // Polar: hundredths of a degree onto 0..0xFFFF
        assert_eq!(to_direction(&polar(0)), 0);
        assert_eq!(to_direction(&polar(9000)), 0x4000);
        assert_eq!(to_direction(&polar(18000)), 0x8000);
        assert_eq!(to_direction(&polar(27000)), 0xC000);
        assert_eq!(to_direction(&polar(36000 + 9000)), 0x4000);
        // (negative angles wrap through the unsigned conversion)
        assert_eq!(to_direction(&polar(-9000)), (-16384i32) as u32 as u16);

        // Spherical: 90 degrees further
        let spherical = |dir| HapticDirection {
            kind: HapticDirectionType::Spherical,
            dir: [dir, 0, 0],
        };
        assert_eq!(to_direction(&spherical(0)), 0x4000);
        assert_eq!(to_direction(&spherical(9000)), 0x8000);
        assert_eq!(to_direction(&spherical(27000)), 0);

        // Cartesian: the axes, then atan2
        let cartesian = |x, y| HapticDirection {
            kind: HapticDirectionType::Cartesian,
            dir: [x, y, 0],
        };
        assert_eq!(to_direction(&cartesian(1, 0)), 0x4000);
        assert_eq!(to_direction(&cartesian(-1, 0)), 0xC000);
        assert_eq!(to_direction(&cartesian(0, 1)), 0x8000);
        assert_eq!(to_direction(&cartesian(0, -1)), 0);
        // South-east: atan2(1, 1) = 45 degrees, + 450 -> 135 degrees
        assert_eq!(
            to_direction(&cartesian(1, 1)),
            (13500u32 * 0x8000 / 18000) as u16
        );
        // North-west: atan2(-1, -1) = -135 degrees -> 315 degrees
        assert_eq!(
            to_direction(&cartesian(-1, -1)),
            (31500u32 * 0x8000 / 18000) as u16
        );

        let steering = HapticDirection {
            kind: HapticDirectionType::SteeringAxis,
            dir: [0; 3],
        };
        assert_eq!(to_direction(&steering), 0x4000);
    }

    #[test]
    fn effects() {
        let constant = to_ff_effect(&HapticEffect::Constant(HapticConstant {
            direction: polar(18000),
            length: 40000,
            delay: 100,
            button: 2,
            interval: 50000,
            level: -1000,
            attack_length: 10,
            attack_level: 40000,
            fade_length: 20,
            fade_level: 30,
        }))
        .unwrap();
        assert_eq!(constant.type_, FF_CONSTANT);
        assert_eq!(constant.direction, 0x8000);
        assert_eq!(
            constant.replay,
            ff_replay {
                length: 32767,
                delay: 100
            }
        );
        assert_eq!(
            constant.trigger,
            ff_trigger {
                button: BTN_GAMEPAD as u16 + 1,
                interval: 32767
            }
        );
        // SAFETY: the constant member is the one written.
        let c = unsafe { constant.u.constant };
        assert_eq!(c.level, -1000);
        assert_eq!(
            c.envelope,
            ff_envelope {
                attack_length: 10,
                attack_level: 32767,
                fade_length: 20,
                fade_level: 30
            }
        );

        let periodic = to_ff_effect(&HapticEffect::Periodic(HapticPeriodic {
            waveform: HapticWaveform::Triangle,
            length: HAPTIC_INFINITY,
            period: 100,
            magnitude: 5000,
            phase: 9000,
            ..Default::default()
        }))
        .unwrap();
        assert_eq!(periodic.type_, FF_PERIODIC);
        assert_eq!(periodic.replay.length, 0);
        assert_eq!(periodic.trigger.button, 0);
        // SAFETY: the periodic member is the one written.
        let p = unsafe { periodic.u.periodic };
        assert_eq!(
            (p.waveform, p.period, p.magnitude, p.phase),
            (FF_TRIANGLE, 100, 5000, 0x4000)
        );
        assert!(p.custom_data.is_null());

        let condition = to_ff_effect(&HapticEffect::Condition(HapticCondition {
            kind: HapticConditionKind::Damper,
            right_sat: [1, 2, 3],
            left_sat: [4, 5, 6],
            right_coeff: [-7, 8, 9],
            deadband: [10, 11, 12],
            center: [13, -14, 15],
            ..Default::default()
        }))
        .unwrap();
        assert_eq!(condition.type_, FF_DAMPER);
        // SAFETY: the condition member is the one written.
        let c = unsafe { condition.u.condition };
        assert_eq!(c[0].right_saturation, 1);
        assert_eq!(c[1].left_saturation, 5);
        assert_eq!(c[0].right_coeff, -7);
        assert_eq!(c[1].deadband, 11);
        assert_eq!(c[1].center, -14);

        let ramp = to_ff_effect(&HapticEffect::Ramp(HapticRamp {
            start: -100,
            end: 100,
            ..Default::default()
        }))
        .unwrap();
        assert_eq!(ramp.type_, FF_RAMP);
        // SAFETY: the ramp member is the one written.
        let r = unsafe { ramp.u.ramp };
        assert_eq!((r.start_level, r.end_level), (-100, 100));

        let rumble = to_ff_effect(&HapticEffect::LeftRight(HapticLeftRight {
            length: 1000,
            large_magnitude: 0xFFFF,
            small_magnitude: 0x1000,
        }))
        .unwrap();
        assert_eq!((rumble.type_, rumble.direction), (FF_RUMBLE, 0x4000));
        assert_eq!(rumble.replay.length, 1000);
        // SAFETY: the rumble member is the one written.
        let r = unsafe { rumble.u.rumble };
        // (clamped to 32767, then doubled)
        assert_eq!((r.strong_magnitude, r.weak_magnitude), (65534, 0x2000));

        assert_eq!(
            to_ff_effect(&HapticEffect::Custom(Default::default()))
                .unwrap_err()
                .message(),
            "Haptic: Unknown effect type."
        );
    }

    #[test]
    fn mouse_detection() {
        // A device with BTN_MOUSE is a mouse; one without isn't.
        let mut keys = [0 as std::ffi::c_ulong; 40];
        assert!(!keys_are_mouse(&keys));
        let bits = std::ffi::c_ulong::BITS as usize;
        keys[BTN_MOUSE / bits] |= 1 << (BTN_MOUSE % bits);
        assert!(keys_are_mouse(&keys));

        // A listed device that can't be opened isn't reported as the mouse.
        let _l = crate::test_support::test_lock();
        let saved = with_list(std::mem::take);
        with_list(|l| {
            l.push(HapticlistItem {
                instance_id: 1,
                fname: "/nonexistent/sdl-haptic-test".to_owned(),
                dev_num: 0,
            })
        });
        assert_eq!(LINUX_HAPTIC_DRIVER.mouse(), None);
        with_list(|l| *l = saved);
    }

    #[test]
    fn buttons() {
        assert_eq!(to_button(0), 0);
        assert_eq!(to_button(1), BTN_GAMEPAD as u16);
        assert_eq!(to_button(3), BTN_GAMEPAD as u16 + 2);
    }

    #[test]
    fn subsystem_without_devices() {
        let _l = crate::test_support::test_lock();
        // Whatever this system has, the list matches what the driver reports
        if LINUX_HAPTIC_DRIVER.init().is_err() {
            crate::test_support::skip("udev", "the haptic driver didn't initialize");
            return;
        }
        let n = LINUX_HAPTIC_DRIVER.count();
        for i in 0..n {
            assert!(LINUX_HAPTIC_DRIVER.instance_id(i) > 0);
        }
        assert_eq!(LINUX_HAPTIC_DRIVER.instance_id(n), 0);
        assert_eq!(LINUX_HAPTIC_DRIVER.name(n), None);
        // Not a haptic device
        assert!(!maybe_add_device("/dev/null"));
        assert!(!maybe_remove_device("/dev/null"));
        LINUX_HAPTIC_DRIVER.quit();
        assert_eq!(LINUX_HAPTIC_DRIVER.count(), 0);
    }
}
