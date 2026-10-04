// Rust translation of src/joystick/hidapi/SDL_hidapi_lg4ff.c from Simple
// DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Logitech wheel driver (G29, G27, G25, Driving Force GT, Driving
//! Force Pro and Driving Force EX), switching the wheels to their native
//! mode and setting their range and autocenter spring. The force feedback
//! effects are the HIDAPI haptic driver's.
//!
//! The commands go through the HID I/O of the Valve drivers
//! ([`SteamHid`]), which the tests fake.

use super::ps4::hat_of;
use super::steam::SteamHid;
use super::{DeviceCtx, DriverContext, DriverImpl, HidapiDevice, JoystickCaps, SDL_HIDAPI_DEFAULT};
use crate::error::{Error, Result};
use crate::events::JoystickID;
use crate::hints;
use crate::joystick::gamepad::{GamepadAxis, GamepadButton, GamepadType};
use crate::joystick::usb_ids::USB_VENDOR_LOGITECH;
use crate::joystick::{JoystickData, JoystickType};

const USB_DEVICE_ID_LOGITECH_G29_WHEEL: u16 = 0xc24f;
const USB_DEVICE_ID_LOGITECH_G27_WHEEL: u16 = 0xc29b;
const USB_DEVICE_ID_LOGITECH_G25_WHEEL: u16 = 0xc299;
const USB_DEVICE_ID_LOGITECH_DFGT_WHEEL: u16 = 0xc29a;
const USB_DEVICE_ID_LOGITECH_DFP_WHEEL: u16 = 0xc298;
const USB_DEVICE_ID_LOGITECH_WHEEL: u16 = 0xc294;

/// The supported wheels, with their names (`supported_device_ids` and
/// `supported_device_names`).
const SUPPORTED_DEVICES: [(u16, &str); 6] = [
    (USB_DEVICE_ID_LOGITECH_G29_WHEEL, "Logitech G29"),
    (USB_DEVICE_ID_LOGITECH_G27_WHEEL, "Logitech G27"),
    (USB_DEVICE_ID_LOGITECH_G25_WHEEL, "Logitech G25"),
    (
        USB_DEVICE_ID_LOGITECH_DFGT_WHEEL,
        "Logitech Driving Force GT",
    ),
    (
        USB_DEVICE_ID_LOGITECH_DFP_WHEEL,
        "Logitech Driving Force Pro",
    ),
    (USB_DEVICE_ID_LOGITECH_WHEEL, "Driving Force EX"),
];

/// A wheel command.
type Command = [u8; 7];

/// Translation of `HIDAPI_DriverLg4ff_GetDeviceName()`.
fn device_name(device_id: u16) -> &'static str {
    SUPPORTED_DEVICES
        .iter()
        .find(|(id, _)| *id == device_id)
        .map_or("", |(_, name)| name)
}

/// Translation of `HIDAPI_DriverLg4ff_GetNumberOfButtons()`.
fn number_of_buttons(device_id: u16) -> usize {
    match device_id {
        USB_DEVICE_ID_LOGITECH_G29_WHEEL => 25,
        USB_DEVICE_ID_LOGITECH_G27_WHEEL => 23,
        USB_DEVICE_ID_LOGITECH_G25_WHEEL => 19,
        USB_DEVICE_ID_LOGITECH_DFGT_WHEEL => 21,
        USB_DEVICE_ID_LOGITECH_DFP_WHEEL => 14,
        USB_DEVICE_ID_LOGITECH_WHEEL => 13,
        _ => 0,
    }
}

/// Translation of `HIDAPI_DriverLg4ff_IdentifyWheel()`: the native mode of
/// a wheel, 0 if unknown.
///
/// Wheel id information by:
/// Michal Malý <madcatxster@devoid-pointer.net> <madcatxster@gmail.com>
/// Simon Wood <simon@mungewell.org>
/// `git blame v6.12 drivers/hid/hid-lg4ff.c`, <https://github.com/torvalds/linux.git>
fn identify_wheel(device_id: u16, release_number: u16) -> u16 {
    let is_device = |m: u16, r: u16| (release_number & m) == r;
    let is_dfp = || is_device(0xf000, 0x1000).then_some(USB_DEVICE_ID_LOGITECH_DFP_WHEEL);
    let is_dfgt = || is_device(0xff00, 0x1300).then_some(USB_DEVICE_ID_LOGITECH_DFGT_WHEEL);
    let is_g25 = || is_device(0xff00, 0x1200).then_some(USB_DEVICE_ID_LOGITECH_G25_WHEEL);
    let is_g27 = || is_device(0xfff0, 0x1230).then_some(USB_DEVICE_ID_LOGITECH_G27_WHEEL);
    let is_g29 = || {
        (is_device(0xfff8, 0x1350) || is_device(0xff00, 0x8900))
            .then_some(USB_DEVICE_ID_LOGITECH_G29_WHEEL)
    };
    match device_id {
        USB_DEVICE_ID_LOGITECH_DFP_WHEEL | USB_DEVICE_ID_LOGITECH_WHEEL => is_g29()
            .or_else(is_g27)
            .or_else(is_g25)
            .or_else(is_dfgt)
            .or_else(is_dfp),
        USB_DEVICE_ID_LOGITECH_DFGT_WHEEL => is_g29().or_else(is_dfgt),
        USB_DEVICE_ID_LOGITECH_G25_WHEEL => is_g29().or_else(is_g27).or_else(is_g25),
        USB_DEVICE_ID_LOGITECH_G27_WHEEL => is_g29().or_else(is_g27),
        USB_DEVICE_ID_LOGITECH_G29_WHEEL => is_g29(),
        _ => None,
    }
    .unwrap_or(0)
}

/// Translation of `SDL_HIDAPI_DriverLg4ff_GetEnvInt()`.
fn get_env_int(env_name: &str, min: i32, max: i32, def: i32) -> i32 {
    let Some(env) = crate::stdlib::getenv(env_name) else {
        return def;
    };
    crate::stdlib::atoi(&env).clamp(min, max)
}

/// The command of `HIDAPI_DriverLg4ff_SwitchMode()`.
///
/// Commands by:
/// Michal Malý <madcatxster@devoid-pointer.net> <madcatxster@gmail.com>
/// Simon Wood <simon@mungewell.org>
/// `git blame v6.12 drivers/hid/hid-lg4ff.c`, <https://github.com/torvalds/linux.git>
fn switch_mode_command(target_product_id: u16) -> Option<Command> {
    match target_product_id {
        USB_DEVICE_ID_LOGITECH_G29_WHEEL => Some([0xf8, 0x09, 0x05, 0x01, 0x01, 0x00, 0x00]),
        USB_DEVICE_ID_LOGITECH_G27_WHEEL => Some([0xf8, 0x09, 0x04, 0x01, 0x00, 0x00, 0x00]),
        USB_DEVICE_ID_LOGITECH_G25_WHEEL => Some([0xf8, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00]),
        USB_DEVICE_ID_LOGITECH_DFGT_WHEEL => Some([0xf8, 0x09, 0x03, 0x01, 0x00, 0x00, 0x00]),
        USB_DEVICE_ID_LOGITECH_DFP_WHEEL => Some([0xf8, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00]),
        USB_DEVICE_ID_LOGITECH_WHEEL => Some([0xf8, 0x09, 0x00, 0x01, 0x00, 0x00, 0x00]),
        _ => None,
    }
}

/// Send commands in order, until one fails.
fn send_commands(dev: &dyn SteamHid, commands: &[Command]) -> Result<()> {
    for cmd in commands {
        dev.write(cmd)?;
    }
    Ok(())
}

/// Translation of `HIDAPI_DriverLg4ff_SwitchMode()`.
fn switch_mode(dev: &dyn SteamHid, target_product_id: u16) -> Result<()> {
    match switch_mode_command(target_product_id) {
        Some(cmd) => send_commands(dev, &[cmd]),
        None => Ok(()),
    }
}

/// The commands of `HIDAPI_DriverLg4ff_SetRange()`, for a range of 40 to
/// 900 degrees.
///
/// *Ported*
/// Original functions by:
/// Michal Malý <madcatxster@devoid-pointer.net> <madcatxster@gmail.com>
/// lg4ff_set_range_g25 lg4ff_set_range_dfp
/// `git blame v6.12 drivers/hid/hid-lg4ff.c`, <https://github.com/torvalds/linux.git>
fn range_commands(product_id: u16, range: u16) -> Vec<Command> {
    match product_id {
        USB_DEVICE_ID_LOGITECH_G29_WHEEL
        | USB_DEVICE_ID_LOGITECH_G27_WHEEL
        | USB_DEVICE_ID_LOGITECH_G25_WHEEL
        | USB_DEVICE_ID_LOGITECH_DFGT_WHEEL => {
            let [low, high] = range.to_le_bytes();
            vec![[0xf8, 0x81, low, high, 0, 0, 0]]
        }
        USB_DEVICE_ID_LOGITECH_DFP_WHEEL => {
            let range = i32::from(range);

            // Prepare "coarse" limit command
            let (coarse, full_range) = if range > 200 {
                (0x03, 900)
            } else {
                (0x02, 200)
            };
            let coarse_command = [0xf8, coarse, 0x00, 0x00, 0x00, 0x00, 0x00];

            // Prepare "fine" limit command
            let mut fine_command = [0x81, 0x0b, 0x00, 0x00, 0x00, 0x00, 0x00];

            if range != 200 && range != 900 {
                // Construct fine limit command
                let start_left = ((full_range - range + 1) * 2047) / full_range;
                let start_right = 0xfff - start_left;

                fine_command[2] = (start_left >> 4) as u8;
                fine_command[3] = (start_right >> 4) as u8;
                fine_command[4] = 0xff;
                fine_command[5] = ((start_right & 0xe) << 4 | (start_left & 0xe)) as u8;
                fine_command[6] = 0xff;
            }

            vec![coarse_command, fine_command]
        }
        // no range setting for ffex/dfex
        _ => Vec::new(),
    }
}

/// The commands of `HIDAPI_DriverLg4ff_SetAutoCenter()`, for a magnitude
/// of 0 to 65535.
///
/// *Ported*
/// Original functions by:
/// Simon Wood <simon@mungewell.org>
/// Michal Malý <madcatxster@devoid-pointer.net> <madcatxster@gmail.com>
/// lg4ff_set_autocenter_default lg4ff_set_autocenter_ffex
/// `git blame v6.12 drivers/hid/hid-lg4ff.c`, <https://github.com/torvalds/linux.git>
fn autocenter_commands(is_ffex: bool, magnitude: u32) -> Vec<Command> {
    if is_ffex {
        let magnitude = magnitude * 90 / 65535;

        return vec![[
            0xfe,
            0x03,
            ((magnitude as u16) >> 14) as u8,
            ((magnitude as u16) >> 14) as u8,
            magnitude as u8,
            0,
            0,
        ]];
    }

    // first disable
    let mut commands = vec![[0xf5, 0, 0, 0, 0, 0, 0]];

    if magnitude == 0 {
        return commands;
    }

    // set strength

    let (mut expand_a, expand_b) = if magnitude <= 0xaaaa {
        (0x0c * magnitude, 0x80 * magnitude)
    } else {
        (
            (0x0c * 0xaaaa) + 0x06 * (magnitude - 0xaaaa),
            (0x80 * 0xaaaa) + 0xff * (magnitude - 0xaaaa),
        )
    };
    // TODO do not adjust for MOMO wheels, when support is added
    expand_a >>= 1;

    commands.push([
        0xfe,
        0x0d,
        (expand_a / 0xaaaa) as u8,
        (expand_a / 0xaaaa) as u8,
        (expand_b / 0xaaaa) as u8,
        0,
        0,
    ]);

    // enable
    commands.push([0x14, 0, 0, 0, 0, 0, 0]);
    commands
}

/// The command of `HIDAPI_DriverLg4ff_SendLedCommand()`, for 0 to 5 lit
/// LEDs.
///
/// Commands by:
/// Michal Malý <madcatxster@devoid-pointer.net> <madcatxster@gmail.com>
/// Simon Wood <simon@mungewell.org>
/// lg4ff_led_set_brightness lg4ff_set_leds
/// `git blame v6.12 drivers/hid/hid-lg4ff.c`, <https://github.com/torvalds/linux.git>
fn led_command(state: u8) -> Command {
    let led_state = match state {
        0 => 0,
        1 => 1,
        2 => 3,
        3 => 7,
        4 => 15,
        5 => 31,
        _ => 0,
    };

    [0xf8, 0x12, led_state, 0x00, 0x00, 0x00, 0x00]
}

/// Translation of `HIDAPI_DriverLg4ff_GetBit()`.
fn get_bit(buf: &[u8], bit_num: usize) -> bool {
    let byte_offset = bit_num / 8;
    let local_bit = bit_num % 8;
    let mask = 1 << local_bit;
    (buf[byte_offset] & mask) != 0
}

/// Translation of `lg4ff_adjust_dfp_x_axis()`.
///
/// *Ported*
/// Original functions by:
/// Michal Malý <madcatxster@devoid-pointer.net> <madcatxster@gmail.com>
/// lg4ff_adjust_dfp_x_axis
/// `git blame v6.12 drivers/hid/hid-lg4ff.c`, <https://github.com/torvalds/linux.git>
fn lg4ff_adjust_dfp_x_axis(value: u16, range: u16) -> u16 {
    if range == 900 || range == 200 {
        return value;
    }
    let max_range = if range < 200 { 200 } else { 900 };

    let new_value = 8192 + ((i32::from(value) - 8192) * max_range / i32::from(range));
    new_value.clamp(0, 16383) as u16
}

/// The size of a wheel's input report (part of
/// `HIDAPI_DriverLg4ff_UpdateDevice()`).
fn report_size(product_id: u16) -> usize {
    match product_id {
        USB_DEVICE_ID_LOGITECH_G29_WHEEL => 12,
        USB_DEVICE_ID_LOGITECH_G27_WHEEL | USB_DEVICE_ID_LOGITECH_G25_WHEEL => 11,
        USB_DEVICE_ID_LOGITECH_DFGT_WHEEL | USB_DEVICE_ID_LOGITECH_DFP_WHEEL => 8,
        USB_DEVICE_ID_LOGITECH_WHEEL => 27,
        _ => 0,
    }
}

/// A trigger or pedal axis.
fn pedal(value: u8) -> i16 {
    (i32::from(value) * 257 - 32768) as i16
}

/// The Logitech wheel driver's static functions.
pub(crate) struct Lg4ffDriver;

impl DriverImpl for Lg4ffDriver {
    /// Translation of `HIDAPI_DriverLg4ff_RegisterHints()`.
    fn hints(&self) -> &'static [&'static str] {
        &[hints::JOYSTICK_HIDAPI_LG4FF]
    }

    /// Translation of `HIDAPI_DriverLg4ff_IsEnabled()`.
    fn is_enabled(&self) -> bool {
        let hint_default = if cfg!(windows) {
            /*
             * hid.dll simply cannot send 7 bytes reports unlike other platforms
             * it enforces full length repots of 17 from the device's descriptor, which does not work on the device
             * this breaks ffb and led control, so we disable this by default
             */
            false
        } else {
            hints::get_bool(hints::JOYSTICK_HIDAPI, SDL_HIDAPI_DEFAULT)
        };
        hints::get_bool(hints::JOYSTICK_HIDAPI_LG4FF, hint_default)
    }

    /// Translation of `HIDAPI_DriverLg4ff_IsSupportedDevice()`.
    fn is_supported_device(
        &self,
        device: Option<&HidapiDevice>,
        _name: &str,
        _gamepad_type: GamepadType,
        vendor_id: u16,
        product_id: u16,
        version: u16,
        _interface_number: i32,
        _interface_class: i32,
        _interface_subclass: i32,
        _interface_protocol: i32,
    ) -> bool {
        is_supported_wheel(
            device.map(|device| device as &dyn SteamHid),
            vendor_id,
            product_id,
            version,
        )
    }

    fn new_context(&self) -> Box<dyn DriverContext> {
        Box::new(Lg4ffContext::default())
    }
}

/// `HIDAPI_DriverLg4ff_IsSupportedDevice()` on `dev`, the device if there
/// is one.
fn is_supported_wheel(
    dev: Option<&dyn SteamHid>,
    vendor_id: u16,
    product_id: u16,
    version: u16,
) -> bool {
    if vendor_id != USB_VENDOR_LOGITECH {
        return false;
    }
    if !SUPPORTED_DEVICES.iter().any(|(id, _)| *id == product_id) {
        return false;
    }
    let real_id = identify_wheel(product_id, version);
    if real_id == product_id || real_id == 0 {
        // either it is already in native mode, or we don't know what the native mode is
        return true;
    }
    // a supported native mode is found, send mode change command, then still state that we support the device
    if let Some(dev) = dev {
        if get_env_int("SDL_HIDAPI_LG4FF_NO_MODE_SWITCH", 0, 1, 0) == 0 {
            let _ = switch_mode(dev, real_id);
        }
    }
    true
}

/// Translation of `SDL_DriverLg4ff_Context`.
#[derive(Debug, Default)]
struct Lg4ffContext {
    last_report_buf: [u8; 32],
    initialized: bool,
    is_ffex: bool,
    range: u16,
}

impl Lg4ffContext {
    /// Translation of `HIDAPI_DriverLg4ff_SetRange()`.
    fn set_range(&mut self, product_id: u16, dev: &dyn SteamHid, range: i32) -> Result<()> {
        let range = range.clamp(40, 900) as u16;

        self.range = range;
        send_commands(dev, &range_commands(product_id, range))
    }

    /// Translation of `HIDAPI_DriverLg4ff_SetAutoCenter()`.
    fn set_auto_center(&self, dev: &dyn SteamHid, magnitude: i32) -> Result<()> {
        let magnitude = magnitude.clamp(0, 65535) as u32;
        send_commands(dev, &autocenter_commands(self.is_ffex, magnitude))
    }

    /// `HIDAPI_DriverLg4ff_InitDevice()` on `dev`, after the device is set
    /// up.
    ///
    /// ffex identification method by:
    /// Simon Wood <simon@mungewell.org>
    /// Michal Malý <madcatxster@devoid-pointer.net> <madcatxster@gmail.com>
    /// lg4ff_init
    /// `git blame v6.12 drivers/hid/hid-lg4ff.c`, <https://github.com/torvalds/linux.git>
    fn init(&mut self, device: &mut DeviceCtx<'_>, dev: &dyn SteamHid) -> Result<()> {
        self.set_auto_center(dev, 0)?;

        self.is_ffex = device.product_id() == USB_DEVICE_ID_LOGITECH_WHEEL
            && (device.version() >> 8) == 0x21
            && (device.version() & 0xff) == 0x00;

        self.range = 900;

        device.joystick_connected();
        Ok(())
    }

    /// Translation of `HIDAPI_DriverLg4ff_HandleState()`.
    fn handle_state(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: JoystickID,
        report_buf: &[u8; 32],
        report_size: usize,
    ) -> bool {
        let product_id = device.product_id();
        let last = self.last_report_buf;
        let mut num_buttons = number_of_buttons(product_id);
        let timestamp = crate::timer::ticks_ns();

        let mut state_changed = false;

        let (hat, last_hat) = match product_id {
            USB_DEVICE_ID_LOGITECH_G29_WHEEL
            | USB_DEVICE_ID_LOGITECH_G27_WHEEL
            | USB_DEVICE_ID_LOGITECH_G25_WHEEL
            | USB_DEVICE_ID_LOGITECH_DFGT_WHEEL => (report_buf[0] & 0x0f, last[0] & 0x0f),
            USB_DEVICE_ID_LOGITECH_DFP_WHEEL => (report_buf[3] >> 4, last[3] >> 4),
            USB_DEVICE_ID_LOGITECH_WHEEL => (report_buf[2] & 0x0F, last[2] & 0x0F),
            _ => (0, 0),
        };

        if hat != last_hat {
            state_changed = true;
            // (8 is centered; other values too: do not assert out, in
            // case hardware can report weird hat values)
            device.send_hat(timestamp, joystick, 0, hat_of(hat));
        }

        let bit_offset = match product_id {
            USB_DEVICE_ID_LOGITECH_G29_WHEEL
            | USB_DEVICE_ID_LOGITECH_G27_WHEEL
            | USB_DEVICE_ID_LOGITECH_G25_WHEEL
            | USB_DEVICE_ID_LOGITECH_DFGT_WHEEL => 4,
            USB_DEVICE_ID_LOGITECH_DFP_WHEEL => 14,
            _ => 0,
        };

        let mut button = |device: &mut DeviceCtx<'_>, bit_num: usize, button: usize| {
            let button_on = get_bit(report_buf, bit_num);
            let button_was_on = get_bit(&last, bit_num);
            if button_on != button_was_on {
                state_changed = true;
                device.send_button(
                    timestamp,
                    joystick,
                    (GamepadButton::South as usize + button) as u8,
                    button_on,
                );
            }
        };

        if product_id == USB_DEVICE_ID_LOGITECH_G27_WHEEL {
            // ref https://github.com/sonik-br/lgff_wheel_adapter/blob/d97f7823154818e1b3edff6d51498a122c302728/pico_lgff_wheel_adapter/reports.h#L265-L310
            // shifter_r is outside of the main button bit field for this particular wheel
            num_buttons -= 1;

            button(device, 80, num_buttons);
        }

        for i in 0..num_buttons {
            button(device, bit_offset + i, i);
        }

        let mut axis = |axis: GamepadAxis, value: i32| {
            state_changed = true;
            device.send_axis(timestamp, joystick, axis as u8, value as i16);
        };

        match product_id {
            USB_DEVICE_ID_LOGITECH_G29_WHEEL => {
                // (a native endian Uint16 upstream)
                let x = u16::from_le_bytes([report_buf[4], report_buf[5]]);
                let last_x = u16::from_le_bytes([last[4], last[5]]);
                if x != last_x {
                    axis(GamepadAxis::LeftX, i32::from(x) - 32768);
                }
                if report_buf[6] != last[6] {
                    axis(GamepadAxis::RightX, pedal(report_buf[6]).into());
                }
                if report_buf[7] != last[7] {
                    axis(GamepadAxis::RightY, pedal(report_buf[7]).into());
                }
                if report_buf[8] != last[8] {
                    axis(GamepadAxis::LeftY, pedal(report_buf[8]).into());
                }
            }
            USB_DEVICE_ID_LOGITECH_G27_WHEEL | USB_DEVICE_ID_LOGITECH_G25_WHEEL => {
                let x = u16::from(report_buf[4]) << 6 | u16::from(report_buf[3] >> 2);
                let last_x = u16::from(last[4]) << 6 | u16::from(last[3] >> 2);
                if x != last_x {
                    axis(GamepadAxis::LeftX, i32::from(x) * 4 - 32768);
                }
                if report_buf[5] != last[5] {
                    axis(GamepadAxis::RightX, pedal(report_buf[5]).into());
                }
                if report_buf[6] != last[6] {
                    axis(GamepadAxis::RightY, pedal(report_buf[6]).into());
                }
                if report_buf[7] != last[7] {
                    axis(GamepadAxis::LeftY, pedal(report_buf[7]).into());
                }
            }
            USB_DEVICE_ID_LOGITECH_DFGT_WHEEL => {
                let x = u16::from(report_buf[4]) | u16::from(report_buf[5] & 0x3F) << 8;
                let last_x = u16::from(last[4]) | u16::from(last[5] & 0x3F) << 8;
                if x != last_x {
                    axis(GamepadAxis::LeftX, i32::from(x) * 4 - 32768);
                }
                if report_buf[6] != last[6] {
                    axis(GamepadAxis::LeftY, pedal(report_buf[6]).into());
                }
                if report_buf[7] != last[7] {
                    axis(GamepadAxis::RightX, pedal(report_buf[7]).into());
                }
            }
            USB_DEVICE_ID_LOGITECH_DFP_WHEEL => {
                let x = u16::from(report_buf[0]) | u16::from(report_buf[1] & 0x3F) << 8;
                let last_x = u16::from(last[0]) | u16::from(last[1] & 0x3F) << 8;
                if x != last_x {
                    axis(
                        GamepadAxis::LeftX,
                        i32::from(lg4ff_adjust_dfp_x_axis(x, self.range)) * 4 - 32768,
                    );
                }
                if report_buf[5] != last[5] {
                    axis(GamepadAxis::LeftY, pedal(report_buf[5]).into());
                }
                if report_buf[6] != last[6] {
                    axis(GamepadAxis::RightX, pedal(report_buf[6]).into());
                }
            }
            USB_DEVICE_ID_LOGITECH_WHEEL => {
                if report_buf[3] != last[3] {
                    axis(GamepadAxis::LeftX, pedal(report_buf[3]).into());
                }
                if report_buf[4] != last[4] {
                    axis(GamepadAxis::LeftY, pedal(report_buf[4]).into());
                }
                if report_buf[5] != last[5] {
                    axis(GamepadAxis::RightX, pedal(report_buf[5]).into());
                }
                // FIXME (upstream): the change of byte 6 sends byte 7.
                if report_buf[6] != last[6] {
                    axis(GamepadAxis::RightY, pedal(report_buf[7]).into());
                }
            }
            _ => {}
        }

        self.last_report_buf[..report_size].copy_from_slice(&report_buf[..report_size]);
        state_changed
    }

    /// `HIDAPI_DriverLg4ff_UpdateDevice()` on `dev`, for an open joystick.
    fn update(
        &mut self,
        device: &mut DeviceCtx<'_>,
        dev: &dyn SteamHid,
        joystick: JoystickID,
    ) -> bool {
        let mut report_buf = [0u8; 32];
        let report_size = report_size(device.product_id());

        loop {
            match dev.read(&mut report_buf[..report_size]) {
                Err(_) => {
                    /* Failed to read from controller */
                    if let Some(&first) = device.joysticks().first() {
                        device.joystick_disconnected(first);
                    }
                    return false;
                }
                Ok(0) => break,
                Ok(r) if r == report_size => {
                    let state_changed =
                        self.handle_state(device, joystick, &report_buf, report_size);
                    if state_changed && !self.initialized {
                        self.initialized = true;
                        let _ = self.set_range(
                            device.product_id(),
                            dev,
                            get_env_int("SDL_HIDAPI_LG4FF_RANGE", 40, 900, 900),
                        );
                        let _ = self.set_auto_center(dev, 0);
                    }
                }
                Ok(_) => {}
            }
        }

        true
    }

    /// Translation of `HIDAPI_DriverLg4ff_SetJoystickLED()`.
    fn set_led(
        &self,
        product_id: u16,
        dev: &dyn SteamHid,
        red: u8,
        green: u8,
        blue: u8,
    ) -> Result<()> {
        // only g27/g29, and g923 when supported is added
        if product_id != USB_DEVICE_ID_LOGITECH_G29_WHEEL
            && product_id != USB_DEVICE_ID_LOGITECH_G27_WHEEL
        {
            return Err(Error::unsupported());
        }

        let max_led = u32::from(red.max(green).max(blue));

        // (HIDAPI_DriverLg4ff_SendLedCommand())
        let cmd = led_command(((5 * max_led) / 255) as u8);
        if dev.write(&cmd).ok() != Some(cmd.len()) {
            // (upstream sets no error)
            return Err(Error::new("Couldn't send LED command"));
        }
        Ok(())
    }
}

impl DriverContext for Lg4ffContext {
    /// Translation of `HIDAPI_DriverLg4ff_InitDevice()`.
    fn init_device(&mut self, device: &mut DeviceCtx<'_>) -> Result<()> {
        device.set_joystick_type(JoystickType::Wheel);

        device.set_device_name(device_name(device.product_id()));

        let Some(hid) = device.dev() else {
            return Err(Error::invalid_param("device"));
        };
        hid.set_nonblocking(true)?;

        let dev = device.device().clone();
        self.init(device, &*dev)
    }

    /// Translation of `HIDAPI_DriverLg4ff_UpdateDevice()`.
    fn update_device(&mut self, device: &mut DeviceCtx<'_>) -> bool {
        let Some(joystick) = device.open_joystick_id() else {
            return false;
        };
        let dev = device.device().clone();
        self.update(device, &*dev, joystick)
    }

    /// Translation of `HIDAPI_DriverLg4ff_OpenJoystick()`.
    fn open_joystick(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: &mut JoystickData,
    ) -> Result<()> {
        crate::joystick::assert_joysticks_locked();

        // Initialize the joystick capabilities
        joystick.nhats = 1;
        joystick.nbuttons = number_of_buttons(device.product_id());
        joystick.naxes = match device.product_id() {
            USB_DEVICE_ID_LOGITECH_G29_WHEEL
            | USB_DEVICE_ID_LOGITECH_G27_WHEEL
            | USB_DEVICE_ID_LOGITECH_G25_WHEEL
            | USB_DEVICE_ID_LOGITECH_WHEEL => 4,
            USB_DEVICE_ID_LOGITECH_DFGT_WHEEL => 3,
            USB_DEVICE_ID_LOGITECH_DFP_WHEEL => 3,
            _ => joystick.naxes,
        };

        Ok(())
    }

    /// Translation of `HIDAPI_DriverLg4ff_GetJoystickCapabilities()`.
    fn get_joystick_capabilities(
        &mut self,
        device: &mut DeviceCtx<'_>,
        _joystick: JoystickID,
    ) -> JoystickCaps {
        match device.product_id() {
            USB_DEVICE_ID_LOGITECH_G29_WHEEL | USB_DEVICE_ID_LOGITECH_G27_WHEEL => {
                JoystickCaps::MONO_LED
            }
            _ => JoystickCaps(0),
        }
    }

    /// Translation of `HIDAPI_DriverLg4ff_SetJoystickLED()`.
    fn set_joystick_led(
        &mut self,
        device: &mut DeviceCtx<'_>,
        _joystick: JoystickID,
        red: u8,
        green: u8,
        blue: u8,
    ) -> Result<()> {
        self.set_led(device.product_id(), &**device.device(), red, green, blue)
    }

    /// Translation of `HIDAPI_DriverLg4ff_SendJoystickEffect()`.
    fn send_joystick_effect(
        &mut self,
        device: &mut DeviceCtx<'_>,
        _joystick: JoystickID,
        data: &[u8],
    ) -> Result<()> {
        // allow programs to send raw commands
        if SteamHid::write(&**device.device(), data).ok() != Some(data.len()) {
            // (upstream sets no error)
            return Err(Error::new("Couldn't send effect"));
        }
        Ok(())
    }

    /// Translation of `HIDAPI_DriverLg4ff_SetSensorsEnabled()`.
    fn set_joystick_sensors_enabled(
        &mut self,
        _device: &mut DeviceCtx<'_>,
        _joystick: JoystickID,
        _enabled: bool,
    ) -> Result<()> {
        // On steam deck, sensors are enabled by default. Nothing to do here.
        Err(Error::unsupported())
    }

    /// Translation of `HIDAPI_DriverLg4ff_CloseJoystick()`.
    fn close_joystick(&mut self, device: &mut DeviceCtx<'_>, _joystick: JoystickID) {
        // remember to stop effects on haptics close, when implemented
        let _ = self.set_led(device.product_id(), &**device.device(), 0, 0, 0);
    }

    // (FreeDevice: device context is freed in SDL_hidapijoystick.c)
}

#[cfg(test)]
mod tests;
