// Rust translation of src/core/linux/SDL_evdev_capabilities.c and
// SDL_evdev_capabilities.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// Copyright (C) 2020 Collabora Ltd.
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Guessing what kind of input device an evdev node is from its capability
//! bitmasks, the way udev's `input_id` builtin does
//! (`SDL_EVDEV_GuessDeviceClass()`).

use std::ffi::c_ulong;
use std::ops::{BitAnd, BitOr, BitOrAssign};

use super::input::*;

/// The classes of a device (a device can be any combination of these).
/// Translation of `SDL_UDEV_deviceclass`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub(crate) struct DeviceClass(pub(crate) u32);

impl DeviceClass {
    /// `SDL_UDEV_DEVICE_UNKNOWN`
    pub(crate) const UNKNOWN: DeviceClass = DeviceClass(0x0000);
    /// `SDL_UDEV_DEVICE_MOUSE`
    pub(crate) const MOUSE: DeviceClass = DeviceClass(0x0001);
    /// `SDL_UDEV_DEVICE_KEYBOARD`
    pub(crate) const KEYBOARD: DeviceClass = DeviceClass(0x0002);
    /// `SDL_UDEV_DEVICE_JOYSTICK`
    pub(crate) const JOYSTICK: DeviceClass = DeviceClass(0x0004);
    /// `SDL_UDEV_DEVICE_SOUND`
    pub(crate) const SOUND: DeviceClass = DeviceClass(0x0008);
    /// `SDL_UDEV_DEVICE_TOUCHSCREEN`
    pub(crate) const TOUCHSCREEN: DeviceClass = DeviceClass(0x0010);
    /// `SDL_UDEV_DEVICE_ACCELEROMETER`
    pub(crate) const ACCELEROMETER: DeviceClass = DeviceClass(0x0020);
    /// `SDL_UDEV_DEVICE_TOUCHPAD`
    pub(crate) const TOUCHPAD: DeviceClass = DeviceClass(0x0040);
    /// `SDL_UDEV_DEVICE_HAS_KEYS`
    pub(crate) const HAS_KEYS: DeviceClass = DeviceClass(0x0080);
    /// `SDL_UDEV_DEVICE_VIDEO_CAPTURE`
    pub(crate) const VIDEO_CAPTURE: DeviceClass = DeviceClass(0x0100);

    /// Whether any bit of `other` is set (`class & other`).
    pub(crate) const fn intersects(self, other: DeviceClass) -> bool {
        self.0 & other.0 != 0
    }

    /// Whether no class is set (`!class`).
    pub(crate) const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

impl BitOr for DeviceClass {
    type Output = DeviceClass;
    fn bitor(self, rhs: DeviceClass) -> DeviceClass {
        DeviceClass(self.0 | rhs.0)
    }
}

impl BitOrAssign for DeviceClass {
    fn bitor_assign(&mut self, rhs: DeviceClass) {
        self.0 |= rhs.0;
    }
}

impl BitAnd for DeviceClass {
    type Output = DeviceClass;
    fn bitand(self, rhs: DeviceClass) -> DeviceClass {
        DeviceClass(self.0 & rhs.0)
    }
}

/// `BITS_PER_LONG`
pub(crate) const BITS_PER_LONG: usize = c_ulong::BITS as usize;

/// The number of `unsigned long`s holding bits `0..=x` (`NBITS(x)`).
pub(crate) const fn nbits(x: usize) -> usize {
    ((x - 1) / BITS_PER_LONG) + 1
}

/// `EVDEV_OFF(x)`
const fn evdev_off(x: usize) -> usize {
    x % BITS_PER_LONG
}

/// `EVDEV_LONG(x)`
const fn evdev_long(x: usize) -> usize {
    x / BITS_PER_LONG
}

/// Whether bit `bit` of a kernel bitmask is set (`test_bit(bit, array)`).
pub(crate) fn test_bit(bit: usize, array: &[c_ulong]) -> bool {
    ((array[evdev_long(bit)] >> evdev_off(bit)) & 1) != 0
}

/// `unsigned long bitmask_props[NBITS(INPUT_PROP_MAX)]`
pub(crate) type PropBits = [c_ulong; nbits(INPUT_PROP_MAX)];
/// `unsigned long bitmask_ev[NBITS(EV_MAX)]`
pub(crate) type EvBits = [c_ulong; nbits(EV_MAX)];
/// `unsigned long bitmask_abs[NBITS(ABS_MAX)]`
pub(crate) type AbsBits = [c_ulong; nbits(ABS_MAX)];
/// `unsigned long bitmask_key[NBITS(KEY_MAX)]`
pub(crate) type KeyBits = [c_ulong; nbits(KEY_MAX)];
/// `unsigned long bitmask_rel[NBITS(REL_MAX)]`
pub(crate) type RelBits = [c_ulong; nbits(REL_MAX)];

/// Guess the classes of a device from its capabilities. Translation of
/// `SDL_EVDEV_GuessDeviceClass()`.
pub(crate) fn guess_device_class(
    bitmask_props: &PropBits,
    bitmask_ev: &EvBits,
    bitmask_abs: &AbsBits,
    bitmask_key: &KeyBits,
    bitmask_rel: &RelBits,
) -> DeviceClass {
    struct Range {
        start: usize,
        end: usize,
    }

    // key code ranges above BTN_MISC (start is inclusive, stop is exclusive)
    const HIGH_KEY_BLOCKS: [Range; 2] = [
        Range {
            start: KEY_OK,
            end: BTN_DPAD_UP,
        },
        Range {
            start: KEY_ALS_TOGGLE,
            end: BTN_TRIGGER_HAPPY,
        },
    ];

    let mut devclass = DeviceClass::UNKNOWN;

    // If the kernel specifically says it's an accelerometer, believe it
    if test_bit(INPUT_PROP_ACCELEROMETER, bitmask_props) {
        return DeviceClass::ACCELEROMETER;
    }

    // We treat pointing sticks as indistinguishable from mice
    if test_bit(INPUT_PROP_POINTING_STICK, bitmask_props) {
        return DeviceClass::MOUSE;
    }

    // We treat buttonpads as equivalent to touchpads
    if test_bit(INPUT_PROP_TOPBUTTONPAD, bitmask_props)
        || test_bit(INPUT_PROP_BUTTONPAD, bitmask_props)
        || test_bit(INPUT_PROP_SEMI_MT, bitmask_props)
    {
        return DeviceClass::TOUCHPAD;
    }

    let ev_abs = test_bit(EV_ABS as usize, bitmask_ev);
    let ev_key = test_bit(EV_KEY as usize, bitmask_ev);

    // X, Y, Z axes but no buttons probably means an accelerometer
    if ev_abs
        && test_bit(ABS_X, bitmask_abs)
        && test_bit(ABS_Y, bitmask_abs)
        && test_bit(ABS_Z, bitmask_abs)
        && !ev_key
    {
        return DeviceClass::ACCELEROMETER;
    }

    /* RX, RY, RZ axes but no buttons probably means a gyro or
     * accelerometer (we don't distinguish) */
    if ev_abs
        && test_bit(ABS_RX, bitmask_abs)
        && test_bit(ABS_RY, bitmask_abs)
        && test_bit(ABS_RZ, bitmask_abs)
        && !ev_key
    {
        return DeviceClass::ACCELEROMETER;
    }

    if ev_abs && test_bit(ABS_X, bitmask_abs) && test_bit(ABS_Y, bitmask_abs) {
        if test_bit(BTN_STYLUS, bitmask_key) || test_bit(BTN_TOOL_PEN, bitmask_key) {
            // ID_INPUT_TABLET
        } else if test_bit(BTN_TOOL_FINGER, bitmask_key) && !test_bit(BTN_TOOL_PEN, bitmask_key) {
            devclass |= DeviceClass::TOUCHPAD; // ID_INPUT_TOUCHPAD
        } else if test_bit(BTN_MOUSE, bitmask_key) {
            devclass |= DeviceClass::MOUSE; // ID_INPUT_MOUSE
        } else if test_bit(BTN_TOUCH, bitmask_key) {
            /* TODO: better determining between touchscreen and multitouch touchpad,
            see https://github.com/systemd/systemd/blob/master/src/udev/udev-builtin-input_id.c */
            devclass |= DeviceClass::TOUCHSCREEN; // ID_INPUT_TOUCHSCREEN
        }

        if test_bit(BTN_TRIGGER, bitmask_key)
            || test_bit(BTN_A, bitmask_key)
            || test_bit(BTN_1, bitmask_key)
            || test_bit(ABS_RX, bitmask_abs)
            || test_bit(ABS_RY, bitmask_abs)
            || test_bit(ABS_RZ, bitmask_abs)
            || test_bit(ABS_THROTTLE, bitmask_abs)
            || test_bit(ABS_RUDDER, bitmask_abs)
            || test_bit(ABS_WHEEL, bitmask_abs)
            || test_bit(ABS_GAS, bitmask_abs)
            || test_bit(ABS_BRAKE, bitmask_abs)
        {
            devclass |= DeviceClass::JOYSTICK; // ID_INPUT_JOYSTICK
        }
    }

    if test_bit(EV_REL as usize, bitmask_ev)
        && test_bit(REL_X as usize, bitmask_rel)
        && test_bit(REL_Y as usize, bitmask_rel)
        && test_bit(BTN_MOUSE, bitmask_key)
    {
        devclass |= DeviceClass::MOUSE; // ID_INPUT_MOUSE
    }

    if ev_key {
        let mut found: c_ulong = 0;

        for &bits in &bitmask_key[..BTN_MISC / BITS_PER_LONG] {
            found |= bits;
        }
        // If there are no keys in the lower block, check the higher blocks
        if found == 0 {
            for block in &HIGH_KEY_BLOCKS {
                for i in block.start..block.end {
                    if test_bit(i, bitmask_key) {
                        found = 1;
                        break;
                    }
                }
            }
        }

        if found > 0 {
            devclass |= DeviceClass::HAS_KEYS; // ID_INPUT_KEY
        }
    }

    /* the first 32 bits are ESC, numbers, and Q to D, so if we have all of
     * those, consider it to be a fully-featured keyboard;
     * do not test KEY_RESERVED, though */
    let keyboard_mask: c_ulong = 0xFFFFFFFE;
    if (bitmask_key[0] & keyboard_mask) == keyboard_mask {
        devclass |= DeviceClass::KEYBOARD; // ID_INPUT_KEYBOARD
    }

    devclass
}

#[cfg(test)]
mod tests {
    use super::super::guess_tests::GUESS_TESTS;
    use super::*;

    /// Load little-endian bytes into a native `unsigned long` bitmask, the
    /// way the kernel provides it (`SDL_memcpy()` then `SwapLongLE()`).
    fn bitmask<const N: usize>(bytes: &[u8]) -> [c_ulong; N] {
        const LONG: usize = size_of::<c_ulong>();
        let mut out = [0; N];
        for (i, chunk) in bytes.chunks(LONG).enumerate().take(N) {
            let mut word = [0u8; LONG];
            word[..chunk.len()].copy_from_slice(chunk);
            out[i] = c_ulong::from_le_bytes(word);
        }
        out
    }

    /// The device classes upstream's `device_classes[]` names.
    fn class_names(class: DeviceClass) -> Vec<&'static str> {
        [
            (DeviceClass::MOUSE, "MOUSE"),
            (DeviceClass::KEYBOARD, "KEYBOARD"),
            (DeviceClass::HAS_KEYS, "HAS_KEYS"),
            (DeviceClass::JOYSTICK, "JOYSTICK"),
            (DeviceClass::SOUND, "SOUND"),
            (DeviceClass::TOUCHSCREEN, "TOUCHSCREEN"),
            (DeviceClass::ACCELEROMETER, "ACCELEROMETER"),
            (DeviceClass::TOUCHPAD, "TOUCHPAD"),
        ]
        .into_iter()
        .filter(|(c, _)| class.intersects(*c))
        .map(|(_, n)| n)
        .collect()
    }

    /// Translation of `run_test()` in test/testevdev.c.
    #[test]
    fn testevdev_table() {
        assert_eq!(GUESS_TESTS.len(), 83);
        let mut failures = Vec::new();
        let mut known = 0;
        for t in GUESS_TESTS {
            let actual = guess_device_class(
                &bitmask(t.props),
                &bitmask(t.ev),
                &bitmask(t.abs),
                &bitmask(t.keys),
                &bitmask(t.rel),
            );
            if actual != t.expected {
                if t.todo.is_some() {
                    // Known issue, ignoring
                    known += 1;
                } else {
                    failures.push(format!(
                        "{}: expected 0x{:08x} {:?}, got 0x{:08x} {:?}",
                        t.name,
                        t.expected.0,
                        class_names(t.expected),
                        actual.0,
                        class_names(actual)
                    ));
                }
            }
        }
        assert!(failures.is_empty(), "{failures:#?}");
        // The Proton virtual sensors upstream lists as known issues
        assert_eq!(known, 5);
    }

    #[test]
    fn bit_helpers() {
        assert_eq!(nbits(KEY_MAX), 768 / BITS_PER_LONG);
        assert_eq!(nbits(EV_MAX), 1);
        let mut keys: KeyBits = [0; nbits(KEY_MAX)];
        keys[BTN_A / BITS_PER_LONG] |= 1 << (BTN_A % BITS_PER_LONG);
        assert!(test_bit(BTN_A, &keys));
        assert!(!test_bit(BTN_B, &keys));
    }
}
