// Rust translation of src/joystick/hidapi/SDL_hidapi_gamesir.c from Simple
// DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The GameSir controller driver (the G7 Pro 8K and the Tarantula 8K),
//! switched to their SDL mode, with a simpler report of the basic
//! controls over Bluetooth and before the switch.
//!
//! The controllers take their commands on another interface (collection,
//! on Windows), which the driver opens next to the device.

use std::time::Duration;

use super::ps4::hat_of;
use super::{
    remap_val, DeviceCtx, DriverContext, DriverImpl, HidapiDevice, JoystickCaps,
    SDL_HIDAPI_DEFAULT, USB_PACKET_LENGTH,
};
use crate::error::{Error, Result};
use crate::events::JoystickID;
use crate::hidapi::HidDevice;
use crate::hints;
use crate::joystick::device_info::is_joystick_gamesir_controller;
use crate::joystick::gamepad::{GamepadAxis, GamepadButton, GamepadType};
use crate::joystick::usb_ids::*;
use crate::joystick::{
    JoystickData, HAT_CENTERED, HAT_DOWN, HAT_LEFT, HAT_LEFTDOWN, HAT_LEFTUP, HAT_RIGHT,
    HAT_RIGHTDOWN, HAT_RIGHTUP, HAT_UP,
};
use crate::sensor::{SensorType, STANDARD_GRAVITY};

const GAMESIR_PACKET_HEADER_0: u8 = 0xA1;
const GAMESIR_PACKET_HEADER_1_GAMEPAD: u8 = 0xC8;
const GAMESIR_IMU_RATE_HZ_WIRED: u64 = 1000;
/// We can't tell whether it's connected via dongle or not...
const GAMESIR_IMU_RATE_HZ: u64 = GAMESIR_IMU_RATE_HZ_WIRED;

/// `SDL_NS_PER_SECOND`
const NS_PER_SECOND: u64 = 1_000_000_000;

const BTN_A: u8 = 0x01;
const BTN_B: u8 = 0x02;
const BTN_X: u8 = 0x08;
const BTN_Y: u8 = 0x10;
const BTN_L1: u8 = 0x40;
const BTN_R1: u8 = 0x80;

const BTN_SELECT: u8 = 0x04;
const BTN_START: u8 = 0x08;
const BTN_HOME: u8 = 0x10;
const BTN_L3: u8 = 0x20;
const BTN_R3: u8 = 0x40;
const BTN_CAPTURE: u8 = 0x80;

const BTN_UP: u8 = 0x01;
const BTN_UP_L: u8 = 0x08;
const BTN_UP_R: u8 = 0x02;
const BTN_DOWN: u8 = 0x05;
const BTN_DOWN_L: u8 = 0x06;
const BTN_DOWN_R: u8 = 0x04;
const BTN_LEFT: u8 = 0x07;
const BTN_RIGHT: u8 = 0x03;

const BTN_L4: u8 = 0x40;
const BTN_R4: u8 = 0x80;

const BTN_L5: u8 = 0x01;
const BTN_R5: u8 = 0x02;
const BTN_L6: u8 = 0x04;
const BTN_R6: u8 = 0x08;
const BTN_L7: u8 = 0x10;
const BTN_R7: u8 = 0x20;
const BTN_L8: u8 = 0x40;

/// `SDL_GAMEPAD_BUTTON_GAMESIR_SHARE`
const BUTTON_GAMESIR_SHARE: u8 = 11;
/// `SDL_GAMEPAD_BUTTON_GAMESIR_L4`
const BUTTON_GAMESIR_L4: u8 = 12;
/// `SDL_GAMEPAD_BUTTON_GAMESIR_R4`
const BUTTON_GAMESIR_R4: u8 = 13;
/// `SDL_GAMEPAD_BUTTON_GAMESIR_L5`
const BUTTON_GAMESIR_L5: u8 = 14;
/// `SDL_GAMEPAD_BUTTON_GAMESIR_R5`
const BUTTON_GAMESIR_R5: u8 = 15;
/// `SDL_GAMEPAD_BUTTON_GAMESIR_L6`
const BUTTON_GAMESIR_L6: u8 = 16;
/// `SDL_GAMEPAD_BUTTON_GAMESIR_R6`
const BUTTON_GAMESIR_R6: u8 = 17;
/// `SDL_GAMEPAD_BUTTON_GAMESIR_L7`
const BUTTON_GAMESIR_L7: u8 = 18;
/// `SDL_GAMEPAD_BUTTON_GAMESIR_R7`
const BUTTON_GAMESIR_R7: u8 = 19;
/// `SDL_GAMEPAD_BUTTON_GAMESIR_L8`
const BUTTON_GAMESIR_L8: u8 = 20;
// (SDL_GAMEPAD_BUTTON_GAMESIR_R8: this button doesn't exist?
// SDL_GAMEPAD_BUTTON_GAMESIR_MUTE: this button controls the audio mute LED
// and doesn't seem to be reported; SDL_GAMEPAD_BUTTON_GAMESIR_M: this
// button is for internal use by the firmware)

/// A command report: `0xA2`, the command and its data, zero padded to 64
/// bytes.
fn command_report(command: &[u8]) -> [u8; 64] {
    let mut buf = [0u8; 64];
    buf[0] = 0xA2;
    buf[1..1 + command.len()].copy_from_slice(command);
    buf
}

/// The command of `SendGameSirModeSwitch()` (`Gamesir_CommandMode`
/// `{ 0x01, 0x00 }`).
fn mode_switch_report() -> [u8; 64] {
    command_report(&[0x01, 0x00])
}

/// The command of `HIDAPI_DriverGameSir_RumbleJoystick()`.
fn rumble_report(low_frequency_rumble: u16, high_frequency_rumble: u16) -> [u8; 64] {
    command_report(&[
        0x03,
        (low_frequency_rumble >> 8) as u8,
        (high_frequency_rumble >> 8) as u8,
    ])
}

/// The command of `HIDAPI_DriverGameSir_SetJoystickLED()`.
fn led_report(red: u8, green: u8, blue: u8) -> [u8; 64] {
    command_report(&[0x04, 0x01, 0x01, red, green, blue])
}

/// The GameSir driver's static functions.
pub(crate) struct GameSirDriver;

impl DriverImpl for GameSirDriver {
    /// Translation of `HIDAPI_DriverGameSir_RegisterHints()`.
    fn hints(&self) -> &'static [&'static str] {
        &[hints::JOYSTICK_HIDAPI_GAMESIR]
    }

    /// Translation of `HIDAPI_DriverGameSir_IsEnabled()`.
    fn is_enabled(&self) -> bool {
        hints::get_bool(
            hints::JOYSTICK_HIDAPI_GAMESIR,
            hints::get_bool(hints::JOYSTICK_HIDAPI, SDL_HIDAPI_DEFAULT),
        )
    }

    /// Translation of `HIDAPI_DriverGameSir_IsSupportedDevice()`.
    fn is_supported_device(
        &self,
        device: Option<&HidapiDevice>,
        _name: &str,
        _gamepad_type: GamepadType,
        vendor_id: u16,
        product_id: u16,
        _version: u16,
        _interface_number: i32,
        _interface_class: i32,
        _interface_subclass: i32,
        _interface_protocol: i32,
    ) -> bool {
        if vendor_id == USB_VENDOR_GAMESIR
            && product_id == USB_PRODUCT_GAMESIR_GAMEPAD_TARANTULA_8K
            && device.is_some_and(|device| device.version() <= 553)
        {
            // This controller needs a firmware update
            return false;
        }
        is_joystick_gamesir_controller(vendor_id, product_id)
    }

    fn new_context(&self) -> Box<dyn DriverContext> {
        Box::new(GameSirContext::default())
    }
}

/// Translation of `SDL_DriverGamesir_Context`.
#[derive(Debug)]
struct GameSirContext {
    sensors_supported: bool,
    sensors_enabled: bool,
    led_supported: bool,
    sensor_timestamp_ns: u64,
    sensor_timestamp_step_ns: u64,
    accel_scale: f32,
    gyro_scale: f32,
    last_state_initialized: bool,
    last_state: [u8; USB_PACKET_LENGTH],
    /// The interface the commands go to (closed when the context is
    /// dropped, as `HIDAPI_DriverGameSir_FreeDevice()` does)
    output_handle: Option<HidDevice>,
}

impl Default for GameSirContext {
    fn default() -> Self {
        GameSirContext {
            sensors_supported: false,
            sensors_enabled: false,
            led_supported: false,
            sensor_timestamp_ns: 0,
            sensor_timestamp_step_ns: 0,
            accel_scale: 0.0,
            gyro_scale: 0.0,
            last_state_initialized: false,
            last_state: [0; USB_PACKET_LENGTH],
            output_handle: None,
        }
    }
}

/// Translation of `GetOutputHandle()`: open the interface the commands go
/// to.
fn open_output_handle(device: &HidapiDevice) -> Option<HidDevice> {
    let vendor_id = device.vendor_id();
    let product_id = device.product_id();
    let devs = crate::hidapi::enumerate(vendor_id, product_id).unwrap_or_default();
    devs.iter().find_map(|info| {
        let open = |path: Option<&str>| path.and_then(|path| HidDevice::open_path(path).ok());
        match info.interface_number {
            0 => {
                #[cfg(windows)]
                {
                    crate::hidapi::find_interface_path(vendor_id, product_id, 2)
                        .and_then(|col02_path| open(Some(&col02_path)))
                }
                #[cfg(not(windows))]
                {
                    None
                }
            }
            -1 => {
                if cfg!(windows) && info.usage_page == 0x0001 && info.usage == 0x0005 {
                    open(info.path.as_deref())
                } else {
                    None
                }
            }
            1 => open(info.path.as_deref()),
            _ => None,
        }
    })
}

/// The name of a controller (part of `HIDAPI_DriverGameSir_InitDevice()`).
fn device_name(product_id: u16) -> &'static str {
    match product_id {
        USB_PRODUCT_GAMESIR_GAMEPAD_G7_PRO_8K => "GameSir-G7 Pro 8K",
        USB_PRODUCT_GAMESIR_GAMEPAD_TARANTULA_8K => "GameSir-Tarantula 8K",
        _ => "GameSir Controller",
    }
}

/// Translation of `HIDAPI_DriverGameSir_GetJoystickButtonCount()`.
fn joystick_button_count(device: &HidapiDevice) -> usize {
    if device.is_bluetooth() {
        // Extended buttons are not supported over Bluetooth
        return 11;
    }
    if device.name() == "GameSir-Tarantula 8K" {
        return usize::from(BUTTON_GAMESIR_L8) + 1;
    }
    usize::from(BUTTON_GAMESIR_R5) + 1
}

/// A stick axis of a simple state packet (`READ_STICK_AXIS()`).
fn read_stick_axis(value: u8) -> i16 {
    if value == 0x80 {
        0
    } else {
        remap_val(
            (i32::from(value) - 0x80) as f32,
            -0x80 as f32,
            (0xff - 0x80) as f32,
            f32::from(i16::MIN),
            f32::from(i16::MAX),
        ) as i16
    }
}

/// A trigger axis of a simple state packet (`READ_TRIGGER_AXIS()`).
fn read_trigger_axis(value: u8) -> i16 {
    remap_val(
        f32::from(value),
        0.0,
        255.0,
        f32::from(i16::MIN),
        f32::from(i16::MAX),
    ) as i16
}

/// A big-endian 16-bit value of a state packet.
fn read_be16(data: &[u8], offset: usize) -> i16 {
    i16::from_be_bytes([data[offset], data[offset + 1]])
}

/// A stick Y axis, inverted: SDL convention: up is negative. Clamp
/// -(-32768) to 32767 to avoid Sint16 overflow wrapping back to -32768.
fn inverted(raw: i16) -> i16 {
    if raw == i16::MIN {
        i16::MAX
    } else {
        -raw
    }
}

impl GameSirContext {
    /// Translation of `HIDAPI_DriverGameSir_GetOutputHandle()`: the
    /// device's own handle, except on Windows.
    fn write_output(&self, device: &HidapiDevice, data: &[u8]) -> Option<Result<usize>> {
        if cfg!(windows) {
            self.output_handle.as_ref().map(|handle| handle.write(data))
        } else {
            device.dev().map(|dev| dev.write(data))
        }
    }

    /// Translation of `HIDAPI_DriverGameSir_GetInputHandle()`: on Windows,
    /// the output handle when there is one, unless over Bluetooth.
    fn read_input(&self, device: &HidapiDevice, data: &mut [u8]) -> Option<Result<usize>> {
        if cfg!(windows) && !device.is_bluetooth() {
            if let Some(handle) = &self.output_handle {
                return Some(handle.read_timeout_ms(data, 0));
            }
        }
        device.dev().map(|dev| dev.read_timeout_ms(data, 0))
    }

    /// Translation of `SendGameSirModeSwitch()`.
    fn send_mode_switch(&self, device: &HidapiDevice) -> bool {
        let buf = mode_switch_report();

        for _ in 0..3 {
            match self.write_output(device, &buf) {
                None => return false,
                Some(Ok(_)) => return true,
                Some(Err(_)) => {}
            }
            crate::timer::delay(Duration::from_millis(1));
        }
        false
    }

    /// The setup of `HIDAPI_DriverGameSir_InitDevice()` after the output
    /// handle is opened; the name of the controller.
    fn setup(&mut self, product_id: u16, is_bluetooth: bool) -> &'static str {
        self.led_supported = true;
        self.sensor_timestamp_step_ns = NS_PER_SECOND / GAMESIR_IMU_RATE_HZ;

        let name = device_name(product_id);

        if matches!(
            product_id,
            USB_PRODUCT_GAMESIR_GAMEPAD_G7_PRO_8K | USB_PRODUCT_GAMESIR_GAMEPAD_TARANTULA_8K
        ) {
            if is_bluetooth {
                // Sensors are not supported over Bluetooth
            } else {
                self.sensors_supported = true;
            }
            self.led_supported = false;
        }
        name
    }

    /// Translation of `HIDAPI_DriverGameSir_HandleStatePacket()`.
    fn handle_state_packet(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: JoystickID,
        data: &[u8],
        size: usize,
    ) {
        let timestamp = crate::timer::ticks_ns();
        let last = self.last_state;
        let is_initial_packet = !self.last_state_initialized;

        let min_payload_size = if self.sensors_enabled { 26 } else { 14 };
        if size < min_payload_size {
            return;
        }

        let button = |device: &mut DeviceCtx<'_>, button: u8, down: u8| {
            device.send_button(timestamp, joystick, button, down != 0);
        };

        if last[0] != data[0] {
            let buttons = data[0];
            // BTN1: A B C X Y Z L1 R1
            button(device, GamepadButton::South as u8, buttons & BTN_A);
            button(device, GamepadButton::East as u8, buttons & BTN_B);
            button(device, GamepadButton::West as u8, buttons & BTN_X);
            button(device, GamepadButton::North as u8, buttons & BTN_Y);
            button(device, GamepadButton::LeftShoulder as u8, buttons & BTN_L1);
            button(device, GamepadButton::RightShoulder as u8, buttons & BTN_R1);
        }

        if last[1] != data[1] {
            let buttons = data[1];
            // BTN2: L2 R2 SELECT START HOME L3 R3 CAPTURE
            // Note: L2/R2 appear as digital buttons in data[1], but their actual analog values are in data[12]/data[13].
            // Only handle the other buttons here; trigger analog values are processed later in the code.
            button(device, GamepadButton::Back as u8, buttons & BTN_SELECT);
            button(device, GamepadButton::Start as u8, buttons & BTN_START);
            button(device, GamepadButton::Guide as u8, buttons & BTN_HOME);
            button(device, GamepadButton::LeftStick as u8, buttons & BTN_L3);
            button(device, GamepadButton::RightStick as u8, buttons & BTN_R3);
            button(device, BUTTON_GAMESIR_SHARE, buttons & BTN_CAPTURE);
        }

        if last[2] != data[2] {
            let buttons = data[2];
            // BTN3: UP DOWN LEFT RIGHT M MUTE L4 R4
            // Handle the directional pad (D-pad)
            let hat = match buttons & 0x0F {
                BTN_UP_R => HAT_RIGHTUP,
                BTN_UP_L => HAT_LEFTUP,
                BTN_DOWN_R => HAT_RIGHTDOWN,
                BTN_DOWN_L => HAT_LEFTDOWN,
                BTN_UP => HAT_UP,
                BTN_DOWN => HAT_DOWN,
                BTN_LEFT => HAT_LEFT,
                BTN_RIGHT => HAT_RIGHT,
                _ => HAT_CENTERED,
            };
            device.send_hat(timestamp, joystick, 0, hat);

            // Handle other buttons
            // (M and MUTE aren't reported)
            button(device, BUTTON_GAMESIR_L4, buttons & BTN_L4);
            button(device, BUTTON_GAMESIR_R4, buttons & BTN_R4);
        }

        if last[3] != data[3] {
            let buttons = data[3];
            // BTN4: L5 R5 L6 R6 L7 R7 L8 R8
            button(device, BUTTON_GAMESIR_L5, buttons & BTN_L5);
            button(device, BUTTON_GAMESIR_R5, buttons & BTN_R5);
            button(device, BUTTON_GAMESIR_L6, buttons & BTN_L6);
            button(device, BUTTON_GAMESIR_R6, buttons & BTN_R6);
            button(device, BUTTON_GAMESIR_L7, buttons & BTN_L7);
            button(device, BUTTON_GAMESIR_R7, buttons & BTN_R7);
            button(device, BUTTON_GAMESIR_L8, buttons & BTN_L8);
            // (R8 isn't reported)
        }

        let axis = |device: &mut DeviceCtx<'_>, axis: GamepadAxis, value: i16| {
            device.send_axis(timestamp, joystick, axis as u8, value);
        };

        if is_initial_packet {
            // Initialize all joystick axes to center positions
            axis(device, GamepadAxis::LeftX, 0);
            axis(device, GamepadAxis::LeftY, 0);
            axis(device, GamepadAxis::RightX, 0);
            axis(device, GamepadAxis::RightY, 0);
        } else {
            // Left stick handling
            // Left stick: payload bytes 4-7 (16-bit values)
            // Bytes 4-5: X axis (Hi/Low combined into a signed 16-bit value, e.g. 0x7df6)
            // Bytes 6-7: Y axis (Hi/Low combined into a signed 16-bit value)
            if size >= 8 {
                let raw_x = read_be16(data, 4);
                let raw_y = read_be16(data, 6);

                let raw_changed = raw_x != read_be16(&last, 4) || raw_y != read_be16(&last, 6);

                if raw_changed {
                    axis(device, GamepadAxis::LeftX, raw_x);
                    axis(device, GamepadAxis::LeftY, inverted(raw_y));
                }
            }

            // Right stick handling
            // Right stick: payload bytes 8-11 (16-bit values)
            // Bytes 8-9: X axis (Hi/Low combined into a signed 16-bit value)
            // Bytes 10-11: Y axis (Hi/Low combined into a signed 16-bit value)
            if size >= 12 {
                let raw_x = read_be16(data, 8);
                let raw_y = read_be16(data, 10);

                let raw_changed = raw_x != read_be16(&last, 8) || raw_y != read_be16(&last, 10);

                if raw_changed {
                    axis(device, GamepadAxis::RightX, raw_x);
                    axis(device, GamepadAxis::RightY, inverted(raw_y));
                }
            }

            // Handle trigger axes
            // Protocol: L2 (payload byte 12) - analog left trigger 0-255, 0 = released, 255 = fully pressed
            //           R2 (payload byte 13) - analog right trigger 0-255, 0 = released, 255 = fully pressed
            // SDL range: -32768 to 32767 (-32768 = released, 32767 = fully pressed)
            // Linear mapping: 0-255 -> -32768..32767, formula: data * 257 - 32768 (same as PS4)
            if last[12] != data[12] {
                axis(
                    device,
                    GamepadAxis::LeftTrigger,
                    (i32::from(data[12]) * 257 - 32768) as i16,
                );
            }

            if last[13] != data[13] {
                axis(
                    device,
                    GamepadAxis::RightTrigger,
                    (i32::from(data[13]) * 257 - 32768) as i16,
                );
            }
        }

        if self.sensors_enabled && !is_initial_packet && size >= 26 {
            let sensor_timestamp = self.sensor_timestamp_ns;
            self.sensor_timestamp_ns = self
                .sensor_timestamp_ns
                .wrapping_add(self.sensor_timestamp_step_ns);

            // Accelerometer data (payload bytes 14-19)
            // Bytes 14-15: Acc X (Hi/Low combined into a signed 16-bit value)
            // Bytes 16-17: Acc Y (Hi/Low combined into a signed 16-bit value)
            // Bytes 18-19: Acc Z (Hi/Low combined into a signed 16-bit value)
            // Coordinate system matches PS4; use raw values directly without sign inversion
            let accel = |offset| f32::from(read_be16(data, offset)) * self.accel_scale;
            let values = [accel(14), accel(16), accel(18)];
            device.send_sensor(
                timestamp,
                joystick,
                SensorType::Accel,
                sensor_timestamp,
                &values,
            );

            // Gyroscope data (payload bytes 20-25)
            // Bytes 20-21: Gyro X (Hi/Low combined into a signed 16-bit value)
            // Bytes 22-23: Gyro Y (Hi/Low combined into a signed 16-bit value)
            // Bytes 24-25: Gyro Z (Hi/Low combined into a signed 16-bit value)
            // Apply scale factor and convert to floating point (radians/second)
            // Based on the PS4 implementation: use (gyro_numerator / gyro_denominator) * (π / 180)
            // The default configuration corresponds to a range of approximately ±2048 degrees/second,
            // which is a common range for gamepad gyroscopes
            // Coordinate system matches the PS4; use raw values directly without sign inversion
            let gyro = |offset| f32::from(read_be16(data, offset)) * self.gyro_scale;
            let values = [gyro(20), gyro(22), gyro(24)];
            device.send_sensor(
                timestamp,
                joystick,
                SensorType::Gyro,
                sensor_timestamp,
                &values,
            );
        }
        // (upstream reads the touchpads of 32 byte packets, at bytes
        // 26-31, without using them)

        let n = size.min(self.last_state.len());
        self.last_state[..n].copy_from_slice(&data[..n]);
        self.last_state_initialized = true;
    }

    /// The buttons, hat, sticks and triggers of a simple state packet
    /// (`HIDAPI_DriverGameSir_HandleSimpleStatePacketBluetooth()` and
    /// `HIDAPI_DriverGameSir_HandleSimpleStatePacketUSB()`), whose sticks
    /// are at `sticks`, triggers at `triggers` and buttons and hat at
    /// `buttons`.
    #[allow(clippy::too_many_arguments)]
    fn handle_simple_state_packet(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: JoystickID,
        data: &[u8],
        size: usize,
        sticks: usize,
        triggers: [usize; 2],
        buttons: [usize; 3],
    ) {
        let timestamp = crate::timer::ticks_ns();
        let last = &self.last_state;
        let [buttons1, buttons2, hat] = buttons;

        let button = |device: &mut DeviceCtx<'_>, button: GamepadButton, down: u8| {
            device.send_button(timestamp, joystick, button as u8, down != 0);
        };

        if last[buttons1] != data[buttons1] {
            let buttons = data[buttons1];
            // BTN1: A B C X Y Z L1 R1
            button(device, GamepadButton::South, buttons & BTN_A);
            button(device, GamepadButton::East, buttons & BTN_B);
            button(device, GamepadButton::West, buttons & BTN_X);
            button(device, GamepadButton::North, buttons & BTN_Y);
            button(device, GamepadButton::LeftShoulder, buttons & BTN_L1);
            button(device, GamepadButton::RightShoulder, buttons & BTN_R1);
        }

        if last[buttons2] != data[buttons2] {
            let buttons = data[buttons2];
            // BTN2: L2 R2 SELECT START HOME L3 R3 CAPTURE
            // Note: L2/R2 appear as digital buttons here, but their actual analog values are in the triggers' bytes.
            // Only handle the other buttons here; trigger analog values are processed later in the code.
            button(device, GamepadButton::Back, buttons & BTN_SELECT);
            button(device, GamepadButton::Start, buttons & BTN_START);
            button(device, GamepadButton::Guide, buttons & BTN_HOME);
            button(device, GamepadButton::LeftStick, buttons & BTN_L3);
            button(device, GamepadButton::RightStick, buttons & BTN_R3);
        }

        if last[hat] != data[hat] {
            device.send_hat(timestamp, joystick, 0, hat_of(data[hat] & 0xF));
        }

        let mut axis = |axis: GamepadAxis, value: i16| {
            device.send_axis(timestamp, joystick, axis as u8, value);
        };
        axis(GamepadAxis::LeftX, read_stick_axis(data[sticks]));
        axis(GamepadAxis::LeftY, read_stick_axis(data[sticks + 1]));
        axis(GamepadAxis::RightX, read_stick_axis(data[sticks + 2]));
        axis(GamepadAxis::RightY, read_stick_axis(data[sticks + 3]));

        axis(
            GamepadAxis::LeftTrigger,
            read_trigger_axis(data[triggers[0]]),
        );
        axis(
            GamepadAxis::RightTrigger,
            read_trigger_axis(data[triggers[1]]),
        );

        let n = size.min(self.last_state.len());
        self.last_state[..n].copy_from_slice(&data[..n]);
    }

    /// One report of `HIDAPI_DriverGameSir_UpdateDevice()`.
    fn handle_report(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: JoystickID,
        data: &[u8],
        size: usize,
    ) {
        // Check packet format: it may include a report ID (0x43) as the first byte
        // Actual packet format: 43 a1 c8 [button data...]
        // If the first byte is 0x43, the second is 0xA1 and the third is 0xC8, skip the report ID
        if size >= 3
            && data[0] == 0x43
            && data[1] == GAMESIR_PACKET_HEADER_0
            && data[2] == GAMESIR_PACKET_HEADER_1_GAMEPAD
        {
            self.handle_state_packet(device, joystick, &data[3..], size - 3);
        } else if size >= 2
            && data[0] == GAMESIR_PACKET_HEADER_0
            && data[1] == GAMESIR_PACKET_HEADER_1_GAMEPAD
        {
            self.handle_state_packet(device, joystick, &data[2..], size - 2);
        } else if size >= 10 && (data[0] == 0x02 || data[0] == 0x07) {
            // (HIDAPI_DriverGameSir_HandleSimpleStatePacketBluetooth())
            self.handle_simple_state_packet(
                device,
                joystick,
                &data[1..],
                size - 1,
                0,
                [8, 7],
                [5, 6, 4],
            );
        } else if size == 9 {
            // (HIDAPI_DriverGameSir_HandleSimpleStatePacketUSB())
            self.handle_simple_state_packet(device, joystick, data, size, 3, [7, 8], [0, 1, 2]);
        }
    }

    /// The setup of the sensors of `HIDAPI_DriverGameSir_OpenJoystick()`.
    fn open_sensors(&mut self, joystick: &mut JoystickData) {
        if !self.sensors_supported {
            return;
        }

        // GameSir SDL protocol packets currently don't expose an IMU timestamp.
        // Use a synthetic monotonic timestamp at the firmware's fixed IMU rate.
        self.sensor_timestamp_ns = crate::timer::ticks_ns();
        // Accelerometer scale factor: assume a range of ±4g, 16-bit signed values (-32768 to 32767)
        // 32768 corresponds to 4g, so the scale factor = 4 * SDL_STANDARD_GRAVITY / 32768.0f
        self.accel_scale = 4.0 * STANDARD_GRAVITY / 32768.0;

        // Gyro scale factor: based on the PS4 implementation
        // PS4 uses (gyro_numerator / gyro_denominator) * (π / 180)
        // The default value is (1 / 16) * (π / 180), corresponding to a range of approximately ±2048 degrees/second
        // This is a common range for gamepad gyroscopes
        let gyro_numerator = 1.0f32;
        let gyro_denominator = 16.0f32;
        self.gyro_scale = (gyro_numerator / gyro_denominator) * (std::f32::consts::PI / 180.0);

        let sensor_rate = GAMESIR_IMU_RATE_HZ as f32;
        joystick.add_sensor(SensorType::Gyro, sensor_rate);
        joystick.add_sensor(SensorType::Accel, sensor_rate);
    }
}

impl DriverContext for GameSirContext {
    /// Translation of `HIDAPI_DriverGameSir_InitDevice()`.
    fn init_device(&mut self, device: &mut DeviceCtx<'_>) -> Result<()> {
        self.output_handle = open_output_handle(device);
        let name = self.setup(device.product_id(), device.is_bluetooth());
        device.set_device_name(name);
        if matches!(
            device.product_id(),
            USB_PRODUCT_GAMESIR_GAMEPAD_G7_PRO_8K | USB_PRODUCT_GAMESIR_GAMEPAD_TARANTULA_8K
        ) {
            crate::log::debug!(
                crate::log::Category::Input,
                "GameSir: Device detected - {} (PID 0x{:04X})",
                device.name(),
                device.product_id()
            );
        }

        device.joystick_connected();
        Ok(())
    }

    /// Translation of `HIDAPI_DriverGameSir_UpdateDevice()`.
    fn update_device(&mut self, device: &mut DeviceCtx<'_>) -> bool {
        let joystick = device.open_joystick_id();

        let mut data = [0u8; USB_PACKET_LENGTH];
        let read_error = loop {
            match self.read_input(device, &mut data) {
                None => return false,
                Some(Ok(0)) => break false,
                Some(Ok(size)) => {
                    let Some(joystick) = joystick else {
                        continue;
                    };
                    self.handle_report(device, joystick, &data, size);
                }
                Some(Err(_)) => break true,
            }
        };

        if read_error {
            if let Some(&first) = device.joysticks().first() {
                // Read error, device is disconnected
                device.joystick_disconnected(first);
            }
        }
        !read_error
    }

    /// Translation of `HIDAPI_DriverGameSir_OpenJoystick()`.
    fn open_joystick(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: &mut JoystickData,
    ) -> Result<()> {
        crate::joystick::assert_joysticks_locked();

        self.last_state = [0; USB_PACKET_LENGTH];
        self.last_state_initialized = false;

        if !self.send_mode_switch(device) {
            crate::log::debug!(
                crate::log::Category::Input,
                "GameSir: failed to send SDL mode switch command (0xA2, 0x01)"
            );
        }

        joystick.nbuttons = joystick_button_count(device);
        joystick.naxes = GamepadAxis::COUNT;
        joystick.nhats = 1;

        self.open_sensors(joystick);

        Ok(())
    }

    /// Translation of `HIDAPI_DriverGameSir_RumbleJoystick()`.
    fn rumble_joystick(
        &mut self,
        device: &mut DeviceCtx<'_>,
        _joystick: JoystickID,
        low_frequency_rumble: u16,
        high_frequency_rumble: u16,
    ) -> Result<()> {
        let buf = rumble_report(low_frequency_rumble, high_frequency_rumble);

        match self.write_output(device, &buf) {
            Some(Ok(_)) => Ok(()),
            // (upstream fails without an error)
            _ => Err(Error::new("Couldn't send rumble packet")),
        }
    }

    /// Translation of `HIDAPI_DriverGameSir_GetJoystickCapabilities()`.
    fn get_joystick_capabilities(
        &mut self,
        _device: &mut DeviceCtx<'_>,
        _joystick: JoystickID,
    ) -> JoystickCaps {
        let mut caps = JoystickCaps::RUMBLE;
        if self.led_supported {
            caps |= JoystickCaps::RGB_LED;
        }
        caps
    }

    /// Translation of `HIDAPI_DriverGameSir_SetJoystickLED()`.
    fn set_joystick_led(
        &mut self,
        device: &mut DeviceCtx<'_>,
        _joystick: JoystickID,
        red: u8,
        green: u8,
        blue: u8,
    ) -> Result<()> {
        if !self.led_supported {
            return Err(Error::unsupported());
        }

        let buf = led_report(red, green, blue);
        match self.write_output(device, &buf) {
            Some(Ok(_)) => Ok(()),
            // (upstream fails without an error)
            _ => Err(Error::new("Couldn't send LED packet")),
        }
    }

    /// Translation of `HIDAPI_DriverGameSir_SetJoystickSensorsEnabled()`.
    fn set_joystick_sensors_enabled(
        &mut self,
        _device: &mut DeviceCtx<'_>,
        _joystick: JoystickID,
        enabled: bool,
    ) -> Result<()> {
        if self.sensors_supported {
            self.sensors_enabled = enabled;
            if enabled {
                self.sensor_timestamp_ns = crate::timer::ticks_ns();
            }
            return Ok(());
        }
        Err(Error::unsupported())
    }

    /// Translation of `HIDAPI_DriverGameSir_CloseJoystick()`.
    fn close_joystick(&mut self, _device: &mut DeviceCtx<'_>, _joystick: JoystickID) {}

    /// Translation of `HIDAPI_DriverGameSir_FreeDevice()`.
    fn free_device(&mut self, _device: &mut DeviceCtx<'_>) {
        self.output_handle = None;
    }
}

#[cfg(test)]
mod tests;
