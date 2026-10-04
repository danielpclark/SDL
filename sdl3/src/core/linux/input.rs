// Declarations from the Linux kernel's <linux/input.h>,
// <linux/input-event-codes.h> and <linux/joystick.h> (uapi), written by hand
// for the evdev backends of this translation of Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The evdev and joystick interfaces of the Linux kernel: event codes,
//! structures and ioctl requests (the parts SDL uses; `libc` doesn't
//! declare them). The values are the kernel's ABI; the comments name the
//! C macros.

#![allow(non_camel_case_types)]

use std::ffi::{c_int, c_long, c_ulong, c_void};
use std::os::fd::RawFd;

// Event types

pub(crate) const EV_SYN: u16 = 0x00;
pub(crate) const EV_KEY: u16 = 0x01;
pub(crate) const EV_REL: u16 = 0x02;
pub(crate) const EV_ABS: u16 = 0x03;
pub(crate) const EV_MSC: u16 = 0x04;
pub(crate) const EV_FF: u16 = 0x15;
pub(crate) const EV_MAX: usize = 0x1f;

// Synchronization events

pub(crate) const SYN_REPORT: u16 = 0;
// This isn't defined in older Linux kernel headers
pub(crate) const SYN_DROPPED: u16 = 3;

// Misc events

// This isn't defined in older Linux kernel headers
pub(crate) const MSC_TIMESTAMP: u16 = 0x05;

// Keys and buttons

pub(crate) const KEY_RECORD: usize = 167;
pub(crate) const KEY_OK: usize = 0x160;
// missing defines in older Linux kernel headers
pub(crate) const KEY_ALS_TOGGLE: usize = 0x230;
pub(crate) const KEY_MAX: usize = 0x2ff;
pub(crate) const KEY_CNT: usize = KEY_MAX + 1;

pub(crate) const BTN_MISC: usize = 0x100;
pub(crate) const BTN_1: usize = 0x101;
pub(crate) const BTN_MOUSE: usize = 0x110;
pub(crate) const BTN_JOYSTICK: usize = 0x120;
pub(crate) const BTN_TRIGGER: usize = 0x120;
pub(crate) const BTN_GAMEPAD: usize = 0x130;
pub(crate) const BTN_A: usize = 0x130;
pub(crate) const BTN_B: usize = 0x131;
pub(crate) const BTN_X: usize = 0x133;
pub(crate) const BTN_Y: usize = 0x134;
pub(crate) const BTN_NORTH: usize = 0x133;
pub(crate) const BTN_WEST: usize = 0x134;
pub(crate) const BTN_TL: usize = 0x136;
pub(crate) const BTN_TR: usize = 0x137;
pub(crate) const BTN_TL2: usize = 0x138;
pub(crate) const BTN_TR2: usize = 0x139;
pub(crate) const BTN_SELECT: usize = 0x13a;
pub(crate) const BTN_START: usize = 0x13b;
pub(crate) const BTN_MODE: usize = 0x13c;
pub(crate) const BTN_THUMBL: usize = 0x13d;
pub(crate) const BTN_THUMBR: usize = 0x13e;
pub(crate) const BTN_TOOL_PEN: usize = 0x140;
pub(crate) const BTN_TOOL_FINGER: usize = 0x145;
pub(crate) const BTN_TOUCH: usize = 0x14a;
pub(crate) const BTN_STYLUS: usize = 0x14b;
pub(crate) const BTN_DPAD_UP: usize = 0x220;
pub(crate) const BTN_DPAD_DOWN: usize = 0x221;
pub(crate) const BTN_DPAD_LEFT: usize = 0x222;
pub(crate) const BTN_DPAD_RIGHT: usize = 0x223;
pub(crate) const BTN_GRIPL: usize = 0x224;
pub(crate) const BTN_GRIPR: usize = 0x225;
pub(crate) const BTN_GRIPL2: usize = 0x226;
pub(crate) const BTN_GRIPR2: usize = 0x227;
pub(crate) const BTN_TRIGGER_HAPPY: usize = 0x2c0;
pub(crate) const BTN_TRIGGER_HAPPY1: usize = 0x2c0;
pub(crate) const BTN_TRIGGER_HAPPY2: usize = 0x2c1;
pub(crate) const BTN_TRIGGER_HAPPY3: usize = 0x2c2;
pub(crate) const BTN_TRIGGER_HAPPY4: usize = 0x2c3;
pub(crate) const BTN_TRIGGER_HAPPY5: usize = 0x2c4;
pub(crate) const BTN_TRIGGER_HAPPY6: usize = 0x2c5;
pub(crate) const BTN_TRIGGER_HAPPY7: usize = 0x2c6;
pub(crate) const BTN_TRIGGER_HAPPY8: usize = 0x2c7;

// Relative axes

pub(crate) const REL_X: u16 = 0x00;
pub(crate) const REL_Y: u16 = 0x01;
pub(crate) const REL_MAX: usize = 0x0f;

// Absolute axes

pub(crate) const ABS_X: usize = 0x00;
pub(crate) const ABS_Y: usize = 0x01;
pub(crate) const ABS_Z: usize = 0x02;
pub(crate) const ABS_RX: usize = 0x03;
pub(crate) const ABS_RY: usize = 0x04;
pub(crate) const ABS_RZ: usize = 0x05;
pub(crate) const ABS_THROTTLE: usize = 0x06;
pub(crate) const ABS_RUDDER: usize = 0x07;
pub(crate) const ABS_WHEEL: usize = 0x08;
pub(crate) const ABS_GAS: usize = 0x09;
pub(crate) const ABS_BRAKE: usize = 0x0a;
pub(crate) const ABS_HAT0X: usize = 0x10;
pub(crate) const ABS_HAT0Y: usize = 0x11;
pub(crate) const ABS_HAT1X: usize = 0x12;
pub(crate) const ABS_HAT1Y: usize = 0x13;
pub(crate) const ABS_HAT2X: usize = 0x14;
pub(crate) const ABS_HAT2Y: usize = 0x15;
pub(crate) const ABS_HAT3X: usize = 0x16;
pub(crate) const ABS_HAT3Y: usize = 0x17;
pub(crate) const ABS_MAX: usize = 0x3f;
pub(crate) const ABS_CNT: usize = ABS_MAX + 1;

// Device properties

pub(crate) const INPUT_PROP_BUTTONPAD: usize = 0x02;
pub(crate) const INPUT_PROP_SEMI_MT: usize = 0x03;
pub(crate) const INPUT_PROP_TOPBUTTONPAD: usize = 0x04;
pub(crate) const INPUT_PROP_POINTING_STICK: usize = 0x05;
pub(crate) const INPUT_PROP_ACCELEROMETER: usize = 0x06;
pub(crate) const INPUT_PROP_MAX: usize = 0x1f;

// Force feedback effect types

pub(crate) const FF_RUMBLE: u16 = 0x50;
pub(crate) const FF_PERIODIC: u16 = 0x51;
pub(crate) const FF_CONSTANT: u16 = 0x52;
pub(crate) const FF_SPRING: u16 = 0x53;
pub(crate) const FF_FRICTION: u16 = 0x54;
pub(crate) const FF_DAMPER: u16 = 0x55;
pub(crate) const FF_INERTIA: u16 = 0x56;
pub(crate) const FF_RAMP: u16 = 0x57;

// Force feedback periodic effect types

pub(crate) const FF_SQUARE: u16 = 0x58;
pub(crate) const FF_TRIANGLE: u16 = 0x59;
pub(crate) const FF_SINE: u16 = 0x5a;
pub(crate) const FF_SAW_UP: u16 = 0x5b;
pub(crate) const FF_SAW_DOWN: u16 = 0x5c;
pub(crate) const FF_CUSTOM: u16 = 0x5d;

// Set ff device properties

pub(crate) const FF_GAIN: u16 = 0x60;
pub(crate) const FF_AUTOCENTER: u16 = 0x61;

pub(crate) const FF_MAX: usize = 0x7f;

// The classic joystick interface (<linux/joystick.h>)

/// a button pressed/released (`JS_EVENT_BUTTON`)
pub(crate) const JS_EVENT_BUTTON: u8 = 0x01;
/// a joystick moved (`JS_EVENT_AXIS`)
pub(crate) const JS_EVENT_AXIS: u8 = 0x02;

/// The time of an event (`struct timeval` as the kernel writes it into an
/// `input_event`: two longs).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct input_event_time {
    pub(crate) tv_sec: c_long,
    pub(crate) tv_usec: c_long,
}

/// The event structure itself (`struct input_event`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct input_event {
    pub(crate) time: input_event_time,
    pub(crate) type_: u16,
    pub(crate) code: u16,
    pub(crate) value: i32,
}

/// `struct input_id`
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct input_id {
    pub(crate) bustype: u16,
    pub(crate) vendor: u16,
    pub(crate) product: u16,
    pub(crate) version: u16,
}

/// `struct input_absinfo`: the state and calibration of an absolute axis.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct input_absinfo {
    pub(crate) value: i32,
    pub(crate) minimum: i32,
    pub(crate) maximum: i32,
    pub(crate) fuzz: i32,
    pub(crate) flat: i32,
    pub(crate) resolution: i32,
}

/// `struct ff_replay`: scheduling of the effect.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ff_replay {
    pub(crate) length: u16,
    pub(crate) delay: u16,
}

/// `struct ff_trigger`: what triggers the effect.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ff_trigger {
    pub(crate) button: u16,
    pub(crate) interval: u16,
}

/// `struct ff_envelope`: generic effect envelope.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ff_envelope {
    pub(crate) attack_length: u16,
    pub(crate) attack_level: u16,
    pub(crate) fade_length: u16,
    pub(crate) fade_level: u16,
}

/// `struct ff_constant_effect`
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ff_constant_effect {
    pub(crate) level: i16,
    pub(crate) envelope: ff_envelope,
}

/// `struct ff_ramp_effect`
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ff_ramp_effect {
    pub(crate) start_level: i16,
    pub(crate) end_level: i16,
    pub(crate) envelope: ff_envelope,
}

/// `struct ff_condition_effect`
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ff_condition_effect {
    pub(crate) right_saturation: u16,
    pub(crate) left_saturation: u16,
    pub(crate) right_coeff: i16,
    pub(crate) left_coeff: i16,
    pub(crate) deadband: u16,
    pub(crate) center: i16,
}

/// `struct ff_periodic_effect`
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ff_periodic_effect {
    pub(crate) waveform: u16,
    pub(crate) period: u16,
    pub(crate) magnitude: i16,
    pub(crate) offset: i16,
    pub(crate) phase: u16,
    pub(crate) envelope: ff_envelope,
    pub(crate) custom_len: u32,
    pub(crate) custom_data: *mut i16,
}

/// `struct ff_rumble_effect`
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ff_rumble_effect {
    pub(crate) strong_magnitude: u16,
    pub(crate) weak_magnitude: u16,
}

/// The effect-specific part of an `ff_effect` (its anonymous union `u`).
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) union ff_effect_u {
    pub(crate) constant: ff_constant_effect,
    pub(crate) ramp: ff_ramp_effect,
    pub(crate) periodic: ff_periodic_effect,
    /// One for each axis
    pub(crate) condition: [ff_condition_effect; 2],
    pub(crate) rumble: ff_rumble_effect,
}

/// `struct ff_effect`: a force feedback effect.
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct ff_effect {
    pub(crate) type_: u16,
    pub(crate) id: i16,
    pub(crate) direction: u16,
    pub(crate) trigger: ff_trigger,
    pub(crate) replay: ff_replay,
    pub(crate) u: ff_effect_u,
}

// SAFETY: SDL never uploads custom effects, so `u.periodic.custom_data` is
// always null: the structure owns no memory and refers to nothing.
unsafe impl Send for ff_effect {}

impl ff_effect {
    /// An all-zero effect (`SDL_zerop()`).
    pub(crate) fn zeroed() -> ff_effect {
        // SAFETY: every field is an integer, or the null custom_data pointer;
        // all-zero is a valid value of each.
        unsafe { std::mem::zeroed() }
    }
}

impl std::fmt::Debug for ff_effect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ff_effect")
            .field("type", &self.type_)
            .field("id", &self.id)
            .field("direction", &self.direction)
            .field("trigger", &self.trigger)
            .field("replay", &self.replay)
            .finish_non_exhaustive()
    }
}

/// `struct js_event` (classic joystick interface)
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct js_event {
    /// event timestamp in milliseconds
    pub(crate) time: u32,
    /// value
    pub(crate) value: i16,
    /// event type
    pub(crate) type_: u8,
    /// axis/button number
    pub(crate) number: u8,
}

// ioctl request encoding (<asm-generic/ioctl.h>, and the variants of
// PowerPC, MIPS and SPARC)

const IOC_NRBITS: u32 = 8;
const IOC_TYPEBITS: u32 = 8;
#[cfg(any(
    target_arch = "powerpc",
    target_arch = "powerpc64",
    target_arch = "mips",
    target_arch = "mips64",
    target_arch = "mips32r6",
    target_arch = "mips64r6",
    target_arch = "sparc",
    target_arch = "sparc64"
))]
mod ioc_dir {
    pub(super) const IOC_SIZEBITS: u32 = 13;
    pub(super) const IOC_NONE: u32 = 1;
    pub(super) const IOC_READ: u32 = 2;
    pub(super) const IOC_WRITE: u32 = 4;
}
#[cfg(not(any(
    target_arch = "powerpc",
    target_arch = "powerpc64",
    target_arch = "mips",
    target_arch = "mips64",
    target_arch = "mips32r6",
    target_arch = "mips64r6",
    target_arch = "sparc",
    target_arch = "sparc64"
)))]
mod ioc_dir {
    pub(super) const IOC_SIZEBITS: u32 = 14;
    pub(super) const IOC_NONE: u32 = 0;
    pub(super) const IOC_READ: u32 = 2;
    pub(super) const IOC_WRITE: u32 = 1;
}
use ioc_dir::*;

const IOC_NRSHIFT: u32 = 0;
const IOC_TYPESHIFT: u32 = IOC_NRSHIFT + IOC_NRBITS;
const IOC_SIZESHIFT: u32 = IOC_TYPESHIFT + IOC_TYPEBITS;
const IOC_DIRSHIFT: u32 = IOC_SIZESHIFT + IOC_SIZEBITS;

/// `_IOC(dir, type, nr, size)`
pub(crate) const fn ioc(dir: u32, ty: u8, nr: u32, size: usize) -> c_ulong {
    ((dir << IOC_DIRSHIFT)
        | ((ty as u32) << IOC_TYPESHIFT)
        | (nr << IOC_NRSHIFT)
        | ((size as u32) << IOC_SIZESHIFT)) as c_ulong
}

/// `_IO(type, nr)`
pub(crate) const fn io(ty: u8, nr: u32) -> c_ulong {
    ioc(IOC_NONE, ty, nr, 0)
}

/// `_IOR(type, nr, size)`
pub(crate) const fn ior(ty: u8, nr: u32, size: usize) -> c_ulong {
    ioc(IOC_READ, ty, nr, size)
}

/// `_IOW(type, nr, size)`
pub(crate) const fn iow(ty: u8, nr: u32, size: usize) -> c_ulong {
    ioc(IOC_WRITE, ty, nr, size)
}

/// get device ID (`EVIOCGID`)
pub(crate) const EVIOCGID: c_ulong = ior(b'E', 0x02, size_of::<input_id>());

/// get device name (`EVIOCGNAME(len)`)
pub(crate) const fn eviocgname(len: usize) -> c_ulong {
    ioc(IOC_READ, b'E', 0x06, len)
}

/// get unique identifier (`EVIOCGUNIQ(len)`)
pub(crate) const fn eviocguniq(len: usize) -> c_ulong {
    ioc(IOC_READ, b'E', 0x08, len)
}

/// get device properties (`EVIOCGPROP(len)`)
pub(crate) const fn eviocgprop(len: usize) -> c_ulong {
    ioc(IOC_READ, b'E', 0x09, len)
}

/// get global key state (`EVIOCGKEY(len)`)
pub(crate) const fn eviocgkey(len: usize) -> c_ulong {
    ioc(IOC_READ, b'E', 0x18, len)
}

/// get event bits (`EVIOCGBIT(ev, len)`)
pub(crate) const fn eviocgbit(ev: u16, len: usize) -> c_ulong {
    ioc(IOC_READ, b'E', 0x20 + ev as u32, len)
}

/// get abs value/limits (`EVIOCGABS(abs)`)
pub(crate) const fn eviocgabs(abs: usize) -> c_ulong {
    ior(b'E', 0x40 + abs as u32, size_of::<input_absinfo>())
}

/// send a force effect to a force feedback device (`EVIOCSFF`)
pub(crate) const EVIOCSFF: c_ulong = iow(b'E', 0x80, size_of::<ff_effect>());
/// Erase a force effect (`EVIOCRMFF`)
pub(crate) const EVIOCRMFF: c_ulong = iow(b'E', 0x81, size_of::<c_int>());
/// Report number of effects playable at the same time (`EVIOCGEFFECTS`)
pub(crate) const EVIOCGEFFECTS: c_ulong = ior(b'E', 0x84, size_of::<c_int>());

/// get number of axes (`JSIOCGAXES`)
pub(crate) const JSIOCGAXES: c_ulong = ior(b'j', 0x11, size_of::<u8>());
/// get number of buttons (`JSIOCGBUTTONS`)
pub(crate) const JSIOCGBUTTONS: c_ulong = ior(b'j', 0x12, size_of::<u8>());

/// get identifier string (`JSIOCGNAME(len)`)
pub(crate) const fn jsiocgname(len: usize) -> c_ulong {
    ioc(IOC_READ, b'j', 0x13, len)
}

/// get axis mapping (`JSIOCGAXMAP`)
pub(crate) const JSIOCGAXMAP: c_ulong = ior(b'j', 0x32, ABS_CNT);
/// get button mapping (`JSIOCGBTNMAP`)
pub(crate) const JSIOCGBTNMAP: c_ulong = ior(b'j', 0x34, 2 * (KEY_MAX - BTN_MISC + 1));

/// `ioctl(fd, request, arg)` with a pointer argument; returns ioctl's
/// result (negative on error, with `errno` set).
///
/// # Safety
///
/// `arg` must point to memory of the size and type `request` reads or
/// writes.
pub(crate) unsafe fn ioctl_ptr(fd: RawFd, request: c_ulong, arg: *mut c_void) -> c_int {
    // SAFETY: the caller guarantees that arg fits the request; the request
    // type differs between C libraries, hence the cast.
    unsafe { libc::ioctl(fd, request as _, arg) }
}

/// `ioctl(fd, request, value)` with an integer argument.
pub(crate) fn ioctl_int(fd: RawFd, request: c_ulong, value: c_int) -> c_int {
    // SAFETY: the requests used this way take their argument by value and
    // touch no memory of ours.
    unsafe { libc::ioctl(fd, request as _, value) }
}

/// `ioctl()` reading into `out`, a plain-data value (or array) of exactly
/// the size the request encodes.
pub(crate) fn ioctl_read<T: Copy>(fd: RawFd, request: c_ulong, out: &mut T) -> c_int {
    // SAFETY: out is a valid, writable T, and T is plain data (integers or
    // arrays of them) for every caller, so any bytes the kernel writes form
    // a valid T.
    unsafe { ioctl_ptr(fd, request, (out as *mut T).cast()) }
}

/// `strerror(errno)` for the last OS error.
pub(crate) fn strerror() -> String {
    errno_string(std::io::Error::last_os_error().raw_os_error().unwrap_or(0))
}

/// `strerror(e)`.
pub(crate) fn errno_string(e: c_int) -> String {
    // SAFETY: strerror returns a NUL-terminated string that stays valid
    // until the next strerror call on this thread; it is copied at once.
    unsafe {
        let s = libc::strerror(e);
        if s.is_null() {
            String::new()
        } else {
            std::ffi::CStr::from_ptr(s).to_string_lossy().into_owned()
        }
    }
}

/// `errno`
pub(crate) fn errno() -> c_int {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

/// `errno = value`
pub(crate) fn set_errno(value: c_int) {
    // SAFETY: __errno_location returns this thread's errno.
    unsafe {
        *libc::__errno_location() = value;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layouts_match_the_kernel() {
        assert_eq!(size_of::<input_id>(), 8);
        assert_eq!(size_of::<input_absinfo>(), 24);
        assert_eq!(size_of::<js_event>(), 8);
        assert_eq!(size_of::<input_event>(), 8 + 2 * size_of::<c_long>());
        #[cfg(target_pointer_width = "64")]
        assert_eq!(size_of::<ff_effect>(), 48);
        #[cfg(target_pointer_width = "32")]
        assert_eq!(size_of::<ff_effect>(), 44);
    }

    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    #[test]
    fn request_numbers() {
        // The values <linux/input.h> and <linux/joystick.h> produce on
        // x86-64 and AArch64
        assert_eq!(EVIOCGID, 0x80084502);
        assert_eq!(eviocgname(128), 0x80804506);
        assert_eq!(eviocguniq(127), 0x807f4508);
        assert_eq!(eviocgprop(8), 0x80084509);
        assert_eq!(eviocgkey(96), 0x80604518);
        assert_eq!(eviocgbit(0, 8), 0x80084520);
        assert_eq!(eviocgbit(EV_KEY, 96), 0x80604521);
        assert_eq!(eviocgbit(EV_FF, 16), 0x80104535);
        assert_eq!(eviocgabs(ABS_HAT0X), 0x80184550);
        assert_eq!(EVIOCSFF, 0x40304580);
        assert_eq!(EVIOCRMFF, 0x40044581);
        assert_eq!(EVIOCGEFFECTS, 0x80044584);
        assert_eq!(JSIOCGAXES, 0x80016a11);
        assert_eq!(JSIOCGBUTTONS, 0x80016a12);
        assert_eq!(jsiocgname(128), 0x80806a13);
        assert_eq!(JSIOCGAXMAP, 0x80406a32);
        assert_eq!(JSIOCGBTNMAP, 0x84006a34);
    }
}
