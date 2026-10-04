// Rust translation of src/joystick/hidapi/SDL_hidapi_wii.c (and the Wii
// part of SDL_hidapi_nintendo.h) from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Nintendo Wii Remote driver, with its Nunchuk, Classic Controller and
//! Motion Plus extensions, and the Wii U Pro Controller (which speaks the
//! same protocol).
//!
//! It is disabled by default, as upstream: it doesn't work with the
//! DolphinBar.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use super::rumble::lock_rumble;
use super::{
    DeviceCtx, DriverContext, DriverImpl, HidapiDevice, HintWatch, JoystickCaps, JoystickRef,
};
use crate::error::{Error, Result};
use crate::events::JoystickID;
use crate::hints;
use crate::joystick::gamepad::{GamepadAxis, GamepadButton, GamepadType};
use crate::joystick::usb_ids::*;
use crate::joystick::{joystick_player_index_for_id, JoystickConnectionState, JoystickData};
use crate::power::PowerState;
use crate::sensor::{SensorType, STANDARD_GRAVITY};

// SDL_hidapi_nintendo.h

/// What is plugged into the extension port of a Wii Remote; the last byte
/// of the device GUID. Translation of `EWiiExtensionControllerType`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
#[repr(u8)]
enum ExtensionControllerType {
    #[default]
    Unknown = 0,
    None = 128,
    Nunchuk = 129,
    Gamepad = 130,
    WiiUPro = 131,
}

// SDL_hidapi_wii.c

/// `ENABLE_CONTINUOUS_REPORTING`
const ENABLE_CONTINUOUS_REPORTING: bool = true;

const INPUT_WAIT_TIMEOUT_MS: u64 = 3 * 1000;
const MOTION_PLUS_UPDATE_TIME_MS: u64 = 8 * 1000;
const STATUS_UPDATE_TIME_MS: u64 = 15 * 60 * 1000;

const WII_EXTENSION_NONE: u16 = 0x2E2E;
const WII_EXTENSION_UNINITIALIZED: u16 = 0xFFFF;
const WII_EXTENSION_NUNCHUK: u16 = 0x0000;
const WII_EXTENSION_GAMEPAD: u16 = 0x0101;
const WII_EXTENSION_WIIUPRO: u16 = 0x0120;
const WII_EXTENSION_MOTIONPLUS_MASK: u16 = 0xF0FF;
const WII_EXTENSION_MOTIONPLUS_ID: u16 = 0x0005;

const WII_MOTIONPLUS_MODE_NONE: u8 = 0x00;
const WII_MOTIONPLUS_MODE_STANDARD: u8 = 0x04;
const WII_MOTIONPLUS_MODE_NUNCHUK: u8 = 0x05;
const WII_MOTIONPLUS_MODE_GAMEPAD: u8 = 0x07;

// EWiiInputReportIDs
const INPUT_STATUS: u8 = 0x20;
const INPUT_READ_MEMORY: u8 = 0x21;
const INPUT_ACKNOWLEDGE: u8 = 0x22;
const INPUT_BUTTON_DATA_0: u8 = 0x30;
const INPUT_BUTTON_DATA_1: u8 = 0x31;
const INPUT_BUTTON_DATA_2: u8 = 0x32;
const INPUT_BUTTON_DATA_3: u8 = 0x33;
const INPUT_BUTTON_DATA_4: u8 = 0x34;
const INPUT_BUTTON_DATA_5: u8 = 0x35;
const INPUT_BUTTON_DATA_6: u8 = 0x36;
const INPUT_BUTTON_DATA_7: u8 = 0x37;
const INPUT_BUTTON_DATA_D: u8 = 0x3D;
const INPUT_BUTTON_DATA_F: u8 = 0x3F;

// EWiiOutputReportIDs
const OUTPUT_RUMBLE: u8 = 0x10;
const OUTPUT_LEDS: u8 = 0x11;
const OUTPUT_DATA_REPORTING_MODE: u8 = 0x12;
const OUTPUT_STATUS_REQUEST: u8 = 0x15;
const OUTPUT_WRITE_MEMORY: u8 = 0x16;
const OUTPUT_READ_MEMORY: u8 = 0x17;

// EWiiPlayerLEDs
const PLAYER_LED_P1: u8 = 0x10;
const PLAYER_LED_P2: u8 = 0x20;
const PLAYER_LED_P3: u8 = 0x40;
const PLAYER_LED_P4: u8 = 0x80;

/// Translation of `EWiiCommunicationState`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum CommunicationState {
    /// No special communications happening
    #[default]
    None,
    /// Sent standard extension identify request
    CheckMotionPlusStage1,
    /// Sent Motion Plus extension identify request
    CheckMotionPlusStage2,
}

// EWiiButtons: the Wii Remote's own buttons, after the gamepad buttons
const WII_BUTTON_A: u8 = GamepadButton::Misc1 as u8;
const WII_BUTTON_B: u8 = WII_BUTTON_A + 1;
const WII_BUTTON_ONE: u8 = WII_BUTTON_A + 2;
const WII_BUTTON_TWO: u8 = WII_BUTTON_A + 3;
const WII_BUTTON_PLUS: u8 = WII_BUTTON_A + 4;
const WII_BUTTON_MINUS: u8 = WII_BUTTON_A + 5;
const WII_BUTTON_HOME: u8 = WII_BUTTON_A + 6;
const WII_BUTTON_DPAD_UP: u8 = WII_BUTTON_A + 7;
const WII_BUTTON_DPAD_DOWN: u8 = WII_BUTTON_A + 8;
const WII_BUTTON_DPAD_LEFT: u8 = WII_BUTTON_A + 9;
const WII_BUTTON_DPAD_RIGHT: u8 = WII_BUTTON_A + 10;
const WII_BUTTON_MAX: u8 = WII_BUTTON_A + 11;

/// `k_unWiiPacketDataLength`
const WII_PACKET_DATA_LENGTH: usize = 22;

/// The parts of an input report (`WiiButtonData`).
#[derive(Clone, Copy, Debug, Default)]
struct ButtonData {
    base_buttons: [u8; 2],
    accelerometer: [u8; 3],
    extension: [u8; 21],
    has_base_buttons: bool,
    has_accelerometer: bool,
    n_extension_bytes: u8,
}

/// Translation of `StickCalibrationData`.
#[derive(Clone, Copy, Debug, Default)]
struct StickCalibration {
    min: u16,
    max: u16,
    center: u16,
    deadzone: u16,
}

/// What the driver does with its device, and the clock it reads: the
/// `SDL_hid_*()` calls on `ctx->device->dev`, the rumble thread,
/// `SDL_GetTicks()` and `SDL_Delay()`. The tests replace them.
pub(crate) trait WiiLink {
    /// `SDL_GetAtomicInt(&ctx->device->rumble_pending) > 0`
    fn rumble_pending(&self) -> bool;
    /// `SDL_hid_read_timeout()`
    fn read_timeout(&self, data: &mut [u8], milliseconds: i32) -> Result<usize>;
    /// `SDL_hid_write()`
    fn write(&self, data: &[u8]) -> Result<usize>;
    /// `SDL_HIDAPI_LockRumble()` and `SDL_HIDAPI_SendRumbleAndUnlock()`
    fn send_rumble(&self, data: &[u8]) -> Result<usize>;
    /// `SDL_GetTicks()`
    fn ticks(&self) -> u64;
    /// `SDL_GetTicksNS()`
    fn ticks_ns(&self) -> u64;
    /// `SDL_Delay()`
    fn delay(&self, ms: u64);
}

impl WiiLink for Arc<HidapiDevice> {
    fn rumble_pending(&self) -> bool {
        self.rumble_pending.load(Ordering::Acquire) > 0
    }
    fn read_timeout(&self, data: &mut [u8], milliseconds: i32) -> Result<usize> {
        self.dev()
            .ok_or_else(|| Error::invalid_param("device"))?
            .read_timeout_ms(data, milliseconds)
    }
    fn write(&self, data: &[u8]) -> Result<usize> {
        self.dev()
            .ok_or_else(|| Error::invalid_param("device"))?
            .write(data)
    }
    fn send_rumble(&self, data: &[u8]) -> Result<usize> {
        lock_rumble()?.send_and_unlock(self, data)
    }
    fn ticks(&self) -> u64 {
        crate::timer::ticks_ms()
    }
    fn ticks_ns(&self) -> u64 {
        crate::timer::ticks_ns()
    }
    fn delay(&self, ms: u64) {
        crate::timer::delay(Duration::from_millis(ms));
    }
}

/// The id of the joystick a driver function changes.
fn joystick_id(joystick: &JoystickRef<'_>) -> JoystickID {
    match joystick {
        JoystickRef::Opening(joystick) => joystick.instance_id,
        JoystickRef::Open(id) => *id,
    }
}

/// Translation of `SDL_DriverWii_Context`.
#[derive(Debug, Default)]
struct WiiContext {
    /// The open joystick (`ctx->joystick`)
    joystick: Option<JoystickID>,
    timestamp: u64,
    comm_state: CommunicationState,
    extension_controller_type: ExtensionControllerType,
    player_lights: bool,
    player_index: i32,
    rumble_active: bool,
    motion_plus_present: bool,
    motion_plus_mode: u8,
    report_sensors: bool,
    read_buffer: [u8; WII_PACKET_DATA_LENGTH],
    last_input: u64,
    last_status: u64,
    next_motion_plus_check: u64,
    disconnected: bool,

    stick_calibration_data: [StickCalibration; 6],

    /// The `SDL_PlayerLEDHintChanged()` callback.
    player_led_hint: Option<HintWatch>,
}

/// The Wii driver's static functions.
pub(crate) struct WiiDriver;

impl DriverImpl for WiiDriver {
    /// Translation of `HIDAPI_DriverWii_RegisterHints()`.
    fn hints(&self) -> &'static [&'static str] {
        &[hints::JOYSTICK_HIDAPI_WII]
    }

    /// Translation of `HIDAPI_DriverWii_IsEnabled()`.
    fn is_enabled(&self) -> bool {
        // This doesn't work with the dolphinbar, so don't enable by default right now
        // (upstream's disabled alternative: the JOYSTICK_HIDAPI hint, then SDL_HIDAPI_DEFAULT)
        hints::get_bool(hints::JOYSTICK_HIDAPI_WII, false)
    }

    /// Translation of `HIDAPI_DriverWii_IsSupportedDevice()`.
    fn is_supported_device(
        &self,
        _device: Option<&HidapiDevice>,
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
        vendor_id == USB_VENDOR_NINTENDO
            && (product_id == USB_PRODUCT_NINTENDO_WII_REMOTE
                || product_id == USB_PRODUCT_NINTENDO_WII_REMOTE2)
    }

    fn new_context(&self) -> Box<dyn DriverContext> {
        Box::new(WiiContext::default())
    }
}

/// Translation of `IsWriteMemoryResponse()`.
fn is_write_memory_response(data: &[u8]) -> bool {
    data[3] == OUTPUT_WRITE_MEMORY
}

/// Translation of `GetExtensionType()`.
fn get_extension_type(extension_id: u16) -> ExtensionControllerType {
    match extension_id {
        WII_EXTENSION_NONE => ExtensionControllerType::None,
        WII_EXTENSION_NUNCHUK => ExtensionControllerType::Nunchuk,
        WII_EXTENSION_GAMEPAD => ExtensionControllerType::Gamepad,
        WII_EXTENSION_WIIUPRO => ExtensionControllerType::WiiUPro,
        _ => ExtensionControllerType::Unknown,
    }
}

/// The power level of a Wii Remote's status report (part of
/// `UpdatePowerLevelWii()`).
fn wii_power_percent(battery_level_byte: u8) -> i32 {
    if battery_level_byte > 178 {
        100
    } else if battery_level_byte > 51 {
        70
    } else if battery_level_byte > 13 {
        20
    } else {
        5
    }
}

/// The connection and power of a Wii U Pro Controller's battery byte
/// (part of `UpdatePowerLevelWiiU()`).
fn wiiu_power_info(extension_battery_byte: u8) -> (JoystickConnectionState, PowerState, i32) {
    let charging = extension_battery_byte & 0x08 == 0;
    let plugged_in = extension_battery_byte & 0x04 == 0;
    let battery_level = extension_battery_byte >> 4;

    let connection_state = if plugged_in {
        JoystickConnectionState::Wired
    } else {
        JoystickConnectionState::Wireless
    };

    /* Not sure if all Wii U Pro controllers act like this, but on mine
     * 4, 3, and 2 are held for about 20 hours each
     * 1 is held for about 6 hours
     * 0 is held for about 2 hours
     * No value above 4 has been observed.
     */
    let state = if charging {
        PowerState::Charging
    } else if plugged_in {
        PowerState::Charged
    } else {
        PowerState::OnBattery
    };
    let percent = match battery_level {
        4.. => 100,
        3 => 70,
        2 => 40,
        1 => 10,
        0 => 3,
    };
    (connection_state, state, percent)
}

/// Translation of `PostStickCalibrated()`.
fn post_stick_calibrated(
    device: &mut DeviceCtx<'_>,
    timestamp: u64,
    joystick: JoystickID,
    calibration: &mut StickCalibration,
    axis: GamepadAxis,
    data: u16,
) {
    let mut value: i16 = 0;
    if calibration.center == 0 {
        // Center on first read
        calibration.center = data;
        return;
    }
    if data < calibration.min {
        calibration.min = data;
    }
    if data > calibration.max {
        calibration.max = data;
    }
    // (the comparisons are in int, the rest in Uint16)
    if i32::from(data) < i32::from(calibration.center) - i32::from(calibration.deadzone) {
        let zero = calibration.center.wrapping_sub(calibration.deadzone);
        let range = zero.wrapping_sub(calibration.min);
        let distance = zero.wrapping_sub(data);
        let fvalue = f32::from(distance) / f32::from(range);
        value = (fvalue * f32::from(i16::MIN)) as i16;
    } else if i32::from(data) > i32::from(calibration.center) + i32::from(calibration.deadzone) {
        let zero = calibration.center.wrapping_add(calibration.deadzone);
        let range = calibration.max.wrapping_sub(zero);
        let distance = data.wrapping_sub(zero);
        let fvalue = f32::from(distance) / f32::from(range);
        value = (fvalue * f32::from(i16::MAX)) as i16;
    }
    if matches!(axis, GamepadAxis::LeftY | GamepadAxis::RightY) && value != 0 {
        value = !value;
    }
    device.send_axis(timestamp, joystick, axis as u8, value);
}

/// An unused bit of a button map.
const NO_BUTTON: u8 = 0xFF;

/// Send button data to SDL. Translation of `PostPackedButtonData()`.
///
/// `defs` is a mapping for each bit to which button it represents
/// (`NO_BUTTON` for an unused bit), `data` the button data from the
/// controller, one byte for each array of 8 mappings in `defs`; `on` is the
/// joystick value to be sent if a bit is on, `off` if it's off.
fn post_packed_button_data(
    device: &mut DeviceCtx<'_>,
    timestamp: u64,
    joystick: JoystickID,
    defs: &[[u8; 8]],
    data: &[u8],
    on: bool,
    off: bool,
) {
    for (byte, defs) in data.iter().zip(defs) {
        for (j, &button) in defs.iter().enumerate() {
            if button != NO_BUTTON {
                let down = if (byte >> j) & 1 != 0 { on } else { off };
                device.send_button(timestamp, joystick, button, down);
            }
        }
    }
}

const GAMEPAD_BUTTON_DEFS: [[u8; 8]; 3] = [
    [
        NO_BUTTON, // Unused
        GamepadButton::RightShoulder as u8,
        GamepadButton::Start as u8,
        GamepadButton::Guide as u8,
        GamepadButton::Back as u8,
        GamepadButton::LeftShoulder as u8,
        GamepadButton::DpadDown as u8,
        GamepadButton::DpadRight as u8,
    ],
    [
        GamepadButton::DpadUp as u8,
        GamepadButton::DpadLeft as u8,
        NO_BUTTON, // ZR
        GamepadButton::North as u8,
        GamepadButton::East as u8,
        GamepadButton::West as u8,
        GamepadButton::South as u8,
        NO_BUTTON, // ZL
    ],
    [
        GamepadButton::RightStick as u8,
        GamepadButton::LeftStick as u8,
        NO_BUTTON, // Charging
        NO_BUTTON, // Plugged In
        NO_BUTTON, // Unused
        NO_BUTTON, // Unused
        NO_BUTTON, // Unused
        NO_BUTTON, // Unused
    ],
];

const MP_GAMEPAD_BUTTON_DEFS: [[u8; 8]; 3] = [
    [
        NO_BUTTON, // Unused
        GamepadButton::RightShoulder as u8,
        GamepadButton::Start as u8,
        GamepadButton::Guide as u8,
        GamepadButton::Back as u8,
        GamepadButton::LeftShoulder as u8,
        GamepadButton::DpadDown as u8,
        GamepadButton::DpadRight as u8,
    ],
    [
        NO_BUTTON, // Motion Plus data
        NO_BUTTON, // Motion Plus data
        NO_BUTTON, // ZR
        GamepadButton::North as u8,
        GamepadButton::East as u8,
        GamepadButton::West as u8,
        GamepadButton::South as u8,
        NO_BUTTON, // ZL
    ],
    [
        GamepadButton::RightStick as u8,
        GamepadButton::LeftStick as u8,
        NO_BUTTON, // Charging
        NO_BUTTON, // Plugged In
        NO_BUTTON, // Unused
        NO_BUTTON, // Unused
        NO_BUTTON, // Unused
        NO_BUTTON, // Unused
    ],
];

const MP_FIXUP_DPAD_BUTTON_DEFS: [[u8; 8]; 2] = [
    [
        GamepadButton::DpadUp as u8,
        NO_BUTTON,
        NO_BUTTON,
        NO_BUTTON,
        NO_BUTTON,
        NO_BUTTON,
        NO_BUTTON,
        NO_BUTTON,
    ],
    [
        GamepadButton::DpadLeft as u8,
        NO_BUTTON,
        NO_BUTTON,
        NO_BUTTON,
        NO_BUTTON,
        NO_BUTTON,
        NO_BUTTON,
        NO_BUTTON,
    ],
];

/// The Wii Remote's buttons (of `HandleWiiRemoteButtonData()`).
const WII_REMOTE_BUTTON_DEFS: [[u8; 8]; 2] = [
    [
        WII_BUTTON_DPAD_LEFT,
        WII_BUTTON_DPAD_RIGHT,
        WII_BUTTON_DPAD_DOWN,
        WII_BUTTON_DPAD_UP,
        WII_BUTTON_PLUS,
        NO_BUTTON, // Unused
        NO_BUTTON, // Unused
        NO_BUTTON, // Unused
    ],
    [
        WII_BUTTON_TWO,
        WII_BUTTON_ONE,
        WII_BUTTON_B,
        WII_BUTTON_A,
        WII_BUTTON_MINUS,
        NO_BUTTON, // Unused
        NO_BUTTON, // Unused
        WII_BUTTON_HOME,
    ],
];

/* Wii remote maps really badly to a normal controller
 * Mapped 1 and 2 as X and Y
 * Not going to attempt positional mapping
 */
/// The Wii Remote's buttons as a gamepad (of
/// `HandleWiiRemoteButtonDataAsMainController()`).
const WII_REMOTE_MAIN_CONTROLLER_BUTTON_DEFS: [[u8; 8]; 2] = [
    [
        GamepadButton::DpadLeft as u8,
        GamepadButton::DpadRight as u8,
        GamepadButton::DpadDown as u8,
        GamepadButton::DpadUp as u8,
        GamepadButton::Start as u8,
        NO_BUTTON, // Unused
        NO_BUTTON, // Unused
        NO_BUTTON, // Unused
    ],
    [
        GamepadButton::North as u8,
        GamepadButton::West as u8,
        GamepadButton::South as u8,
        GamepadButton::East as u8,
        GamepadButton::Back as u8,
        NO_BUTTON, // Unused
        NO_BUTTON, // Unused
        GamepadButton::Guide as u8,
    ],
];

/// A trigger that is either pressed or not.
fn digital_trigger(pressed: bool) -> i16 {
    if pressed {
        i16::MIN
    } else {
        i16::MAX
    }
}

/// Translation of `GetBaseButtons()`.
fn get_base_buttons(dst: &mut ButtonData, src: &[u8]) {
    dst.base_buttons.copy_from_slice(&src[..2]);
    dst.has_base_buttons = true;
}

/// Translation of `GetAccelerometer()`.
fn get_accelerometer(dst: &mut ButtonData, src: &[u8]) {
    dst.accelerometer.copy_from_slice(&src[..3]);
    dst.has_accelerometer = true;
}

/// Translation of `GetExtensionData()`.
fn get_extension_data(dst: &mut ButtonData, src: &[u8], size: usize) {
    let size = size.min(dst.extension.len());

    let valid_data = src[..size].iter().any(|&b| b != 0xFF);
    if valid_data {
        dst.extension[..size].copy_from_slice(&src[..size]);
        dst.n_extension_bytes = size as u8;
    }
}

impl WiiContext {
    /// Translation of `ReadInput()`.
    fn read_input(&mut self, link: &dyn WiiLink) -> Result<usize> {
        // Make sure we don't try to read at the same time a write is happening
        if link.rumble_pending() {
            return Ok(0);
        }

        // (DEBUG_WII_PROTOCOL would dump the packet here)
        link.read_timeout(&mut self.read_buffer, 0)
    }

    /// Translation of `WriteOutput()`.
    fn write_output(&self, link: &dyn WiiLink, data: &[u8], sync: bool) -> bool {
        if sync {
            link.write(data).is_ok()
        } else {
            // Use the rumble thread for general asynchronous writes
            link.send_rumble(data).is_ok()
        }
    }

    /// Translation of `ReadInputSync()`: whether the report is in the read
    /// buffer.
    fn read_input_sync(
        &mut self,
        link: &dyn WiiLink,
        expected_id: u8,
        is_mine: Option<fn(&[u8]) -> bool>,
    ) -> Result<()> {
        let end_ticks = link.ticks() + 250; // Seeing successful reads after about 200 ms

        while let Ok(n_read) = self.read_input(link) {
            if n_read > 0 {
                if self.read_buffer[0] == expected_id
                    && is_mine.is_none_or(|is_mine| is_mine(&self.read_buffer))
                {
                    return Ok(());
                }
            } else {
                if link.ticks() >= end_ticks {
                    break;
                }
                link.delay(1);
            }
        }
        Err(Error::new("Read timed out"))
    }

    /// Translation of `WriteRegister()`.
    fn write_register(
        &mut self,
        link: &dyn WiiLink,
        address: u32,
        data: &[u8],
        sync: bool,
    ) -> Result<()> {
        let mut write_request = [0u8; WII_PACKET_DATA_LENGTH];
        let size = data.len();

        write_request[0] = OUTPUT_WRITE_MEMORY;
        write_request[1] = 0x04 | u8::from(self.rumble_active);
        write_request[2] = (address >> 16) as u8;
        write_request[3] = (address >> 8) as u8;
        write_request[4] = address as u8;
        write_request[5] = size as u8;
        debug_assert!(size > 0 && size <= 16);
        write_request[6..6 + size].copy_from_slice(data);

        if !self.write_output(link, &write_request, sync) {
            return Err(Error::new("Couldn't write register"));
        }
        if sync {
            // Wait for response
            self.read_input_sync(link, INPUT_ACKNOWLEDGE, Some(is_write_memory_response))?;
            if self.read_buffer[4] != 0 {
                return Err(Error::new(format!(
                    "Write memory failed: {}",
                    self.read_buffer[4]
                )));
            }
        }
        Ok(())
    }

    /// Translation of `ReadRegister()`.
    fn read_register(
        &mut self,
        link: &dyn WiiLink,
        address: u32,
        size: usize,
        sync: bool,
    ) -> Result<()> {
        let read_request = [
            OUTPUT_READ_MEMORY,
            0x04 | u8::from(self.rumble_active),
            (address >> 16) as u8,
            (address >> 8) as u8,
            address as u8,
            (size >> 8) as u8,
            size as u8,
        ];

        debug_assert!(size > 0 && size <= 0xffff);

        if !self.write_output(link, &read_request, sync) {
            return Err(Error::new("Couldn't read register"));
        }
        if sync {
            // Only waiting for one packet is supported right now
            debug_assert!(size <= 16);
            // Wait for response
            self.read_input_sync(link, INPUT_READ_MEMORY, None)?;
        }
        Ok(())
    }

    /// Translation of `SendExtensionIdentify()`.
    fn send_extension_identify(&mut self, link: &dyn WiiLink, sync: bool) -> Result<()> {
        self.read_register(link, 0xA400FE, 2, sync)
    }

    /// Translation of `ParseExtensionIdentifyResponse()`: the extension ID
    /// in the read buffer.
    fn parse_extension_identify_response(&self) -> Result<u16> {
        let buf = &self.read_buffer;

        if buf[0] != INPUT_READ_MEMORY {
            return Err(Error::new("Unexpected extension response type"));
        }

        if buf[4] != 0x00 || buf[5] != 0xFE {
            return Err(Error::new("Unexpected extension response address"));
        }

        if buf[3] != 0x10 {
            let error = buf[3] & 0xF;

            if error == 7 {
                // The extension memory isn't mapped
                return Ok(WII_EXTENSION_NONE);
            }

            if error != 0 {
                return Err(Error::new(format!(
                    "Failed to read extension type: {error}"
                )));
            }
            return Err(Error::new(format!(
                "Unexpected read length when reading extension type: {}",
                (buf[3] >> 4) + 1
            )));
        }

        Ok(u16::from_be_bytes([buf[6], buf[7]]))
    }

    /// Translation of `SendExtensionReset()`.
    fn send_extension_reset(&mut self, link: &dyn WiiLink, sync: bool) -> Result<()> {
        let result = self.write_register(link, 0xA400F0, &[0x55], sync);
        // This write will fail if there is no extension connected, that's fine
        let _ = self.write_register(link, 0xA400FB, &[0x00], sync);
        result
    }

    /// Translation of `GetMotionPlusState()`: whether the Motion Plus
    /// extension is connected, and its mode if it is active. (Upstream also
    /// returns whether it could find out, which nothing reads.)
    fn get_motion_plus_state(&mut self, link: &dyn WiiLink) -> (bool, u8) {
        if self.extension_controller_type == ExtensionControllerType::WiiUPro {
            // The Wii U Pro controller never has the Motion Plus extension
            return (false, 0);
        }

        if self.send_extension_identify(link, true).is_ok() {
            if let Ok(extension) = self.parse_extension_identify_response() {
                if (extension & WII_EXTENSION_MOTIONPLUS_MASK) == WII_EXTENSION_MOTIONPLUS_ID {
                    // Motion Plus is currently active
                    return (true, (extension >> 8) as u8);
                }
            }
        }

        if self.read_register(link, 0xA600FE, 2, true).is_ok() {
            if let Ok(extension) = self.parse_extension_identify_response() {
                // Motion Plus is currently connected
                let connected =
                    (extension & WII_EXTENSION_MOTIONPLUS_MASK) == WII_EXTENSION_MOTIONPLUS_ID;
                return (connected, 0);
            }
        }

        // Failed to read the register or parse the response
        (false, 0)
    }

    /// Translation of `NeedsPeriodicMotionPlusCheck()`.
    fn needs_periodic_motion_plus_check(&self, status_update: bool) -> bool {
        if self.extension_controller_type == ExtensionControllerType::WiiUPro {
            // The Wii U Pro controller never has the Motion Plus extension
            return false;
        }

        if self.motion_plus_mode != WII_MOTIONPLUS_MODE_NONE && !status_update {
            // We'll get a status update when Motion Plus is disconnected
            return false;
        }

        true
    }

    /// Translation of `SchedulePeriodicMotionPlusCheck()`.
    fn schedule_periodic_motion_plus_check(&mut self, link: &dyn WiiLink) {
        self.next_motion_plus_check = link.ticks() + MOTION_PLUS_UPDATE_TIME_MS;
    }

    /// Translation of `CheckMotionPlusConnection()`.
    fn check_motion_plus_connection(&mut self, link: &dyn WiiLink) {
        let _ = self.send_extension_identify(link, false);

        self.comm_state = CommunicationState::CheckMotionPlusStage1;
    }

    /// Translation of `ActivateMotionPlusWithMode()`.
    fn activate_motion_plus_with_mode(&mut self, link: &dyn WiiLink, mode: u8) {
        /* Linux drivers maintain a lot of state around the Motion Plus
         * extension, so don't mess with it here.
         */
        if !cfg!(target_os = "linux") {
            let _ = self.write_register(link, 0xA600FE, &[mode], true);

            self.motion_plus_mode = mode;
        }
    }

    /// Translation of `ActivateMotionPlus()`.
    fn activate_motion_plus(&mut self, link: &dyn WiiLink) {
        // Pick the pass-through mode based on the connected controller
        let mode = match self.extension_controller_type {
            ExtensionControllerType::Nunchuk => WII_MOTIONPLUS_MODE_NUNCHUK,
            ExtensionControllerType::Gamepad => WII_MOTIONPLUS_MODE_GAMEPAD,
            _ => WII_MOTIONPLUS_MODE_STANDARD,
        };
        self.activate_motion_plus_with_mode(link, mode);
    }

    /// Translation of `DeactivateMotionPlus()`.
    fn deactivate_motion_plus(&mut self, link: &dyn WiiLink) {
        let _ = self.write_register(link, 0xA400F0, &[0x55], true);

        // Wait for the deactivation status message
        let _ = self.read_input_sync(link, INPUT_STATUS, None);

        self.motion_plus_mode = WII_MOTIONPLUS_MODE_NONE;
    }

    /// Translation of `UpdatePowerLevelWii()`.
    fn update_power_level_wii(
        &self,
        device: &mut DeviceCtx<'_>,
        joystick: JoystickID,
        battery_level_byte: u8,
    ) {
        device.send_power_info(
            joystick,
            PowerState::OnBattery,
            wii_power_percent(battery_level_byte),
        );
    }

    /// Translation of `UpdatePowerLevelWiiU()`.
    fn update_power_level_wiiu(
        &self,
        device: &mut DeviceCtx<'_>,
        joystick: &mut JoystickRef<'_>,
        extension_battery_byte: u8,
    ) {
        crate::joystick::assert_joysticks_locked();

        let (connection_state, state, percent) = wiiu_power_info(extension_battery_byte);
        joystick.with(|j| j.connection_state = connection_state);
        device.send_power_info(joystick_id(joystick), state, percent);
    }

    /// Translation of `GetButtonPacketType()`.
    fn get_button_packet_type(&self) -> u8 {
        match self.extension_controller_type {
            ExtensionControllerType::WiiUPro => INPUT_BUTTON_DATA_D,
            ExtensionControllerType::Nunchuk | ExtensionControllerType::Gamepad => {
                if self.report_sensors {
                    INPUT_BUTTON_DATA_5
                } else {
                    INPUT_BUTTON_DATA_2
                }
            }
            _ => {
                if self.report_sensors {
                    INPUT_BUTTON_DATA_5
                } else {
                    INPUT_BUTTON_DATA_0
                }
            }
        }
    }

    /// Translation of `RequestButtonPacketType()`.
    fn request_button_packet_type(&self, link: &dyn WiiLink, report_type: u8) -> bool {
        let mut tt = u8::from(self.rumble_active);

        // Continuous reporting off, tt & 4 == 0
        if ENABLE_CONTINUOUS_REPORTING {
            tt |= 4;
        }

        let data = [OUTPUT_DATA_REPORTING_MODE, tt, report_type];
        self.write_output(link, &data, false)
    }

    /// Translation of `ResetButtonPacketType()`.
    fn reset_button_packet_type(&self, link: &dyn WiiLink) {
        self.request_button_packet_type(link, self.get_button_packet_type());
    }

    /// Translation of `InitStickCalibrationData()`.
    fn init_stick_calibration_data(&mut self) {
        let data = &mut self.stick_calibration_data;
        match self.extension_controller_type {
            ExtensionControllerType::WiiUPro => {
                for calibration in &mut data[..4] {
                    *calibration = StickCalibration {
                        min: 1000,
                        max: 3000,
                        center: 0,
                        deadzone: 100,
                    };
                }
            }
            ExtensionControllerType::Gamepad => {
                for (i, calibration) in data[..4].iter_mut().enumerate() {
                    *calibration = StickCalibration {
                        min: if i < 2 { 9 } else { 5 },
                        max: if i < 2 { 54 } else { 26 },
                        center: 0,
                        deadzone: if i < 2 { 4 } else { 2 },
                    };
                }
            }
            ExtensionControllerType::Nunchuk => {
                for calibration in &mut data[..2] {
                    *calibration = StickCalibration {
                        min: 40,
                        max: 215,
                        center: 0,
                        deadzone: 10,
                    };
                }
            }
            _ => {}
        }
    }

    /// Translation of `InitializeExtension()`.
    fn initialize_extension(&mut self, link: &dyn WiiLink) {
        let _ = self.send_extension_reset(link, true);
        self.init_stick_calibration_data();
        self.reset_button_packet_type(link);
    }

    /// Translation of `UpdateSlotLED()`.
    fn update_slot_led(&self, link: &dyn WiiLink) {
        // The lowest bit needs to have the rumble status
        let mut leds = u8::from(self.rumble_active);

        if self.player_lights {
            let player_index = self.player_index;
            // Use the same LED codes as Smash 8-player for 5-7
            if player_index == 0 || player_index > 3 {
                leds |= PLAYER_LED_P1;
            }
            if player_index == 1 || player_index == 4 {
                leds |= PLAYER_LED_P2;
            }
            if player_index == 2 || player_index == 5 {
                leds |= PLAYER_LED_P3;
            }
            if player_index == 3 || player_index == 6 {
                leds |= PLAYER_LED_P4;
            }
            // Turn on all lights for other player indexes
            if !(0..=6).contains(&player_index) {
                leds |= PLAYER_LED_P1 | PLAYER_LED_P2 | PLAYER_LED_P3 | PLAYER_LED_P4;
            }
        }

        let data = [OUTPUT_LEDS, leds];
        self.write_output(link, &data, false);
    }

    /// Translation of `SDL_PlayerLEDHintChanged()`.
    fn player_led_hint_changed(&mut self, link: &dyn WiiLink, hint: Option<&str>) {
        let player_lights = hints::string_to_bool(hint, true);

        if player_lights != self.player_lights {
            self.player_lights = player_lights;

            self.update_slot_led(link);
        }
    }

    /// The player LED hint's change since the last call, if any (the
    /// hint callback of upstream).
    fn hint_changes(&mut self, link: &dyn WiiLink) {
        if let Some(hint) = self.player_led_hint.as_ref().and_then(HintWatch::take) {
            self.player_led_hint_changed(link, hint.as_deref());
        }
    }

    /// Translation of `ReadExtensionControllerType()`.
    fn read_extension_controller_type(&mut self, link: &dyn WiiLink) -> ExtensionControllerType {
        let mut extension_controller_type = ExtensionControllerType::Unknown;
        const MAX_ATTEMPTS: usize = 20;

        // Create enough of a context to read the controller type from the device
        for _ in 0..MAX_ATTEMPTS {
            if self.send_extension_identify(link, true).is_err() {
                continue;
            }
            let Ok(mut extension) = self.parse_extension_identify_response() else {
                continue;
            };
            let mut motion_plus_mode = 0;
            if (extension & WII_EXTENSION_MOTIONPLUS_MASK) == WII_EXTENSION_MOTIONPLUS_ID {
                motion_plus_mode = (extension >> 8) as u8;
            }
            if motion_plus_mode != 0 || extension == WII_EXTENSION_UNINITIALIZED {
                let _ = self.send_extension_reset(link, true);
                if self.send_extension_identify(link, true).is_ok() {
                    if let Ok(id) = self.parse_extension_identify_response() {
                        extension = id;
                    }
                }
            }

            extension_controller_type = get_extension_type(extension);

            // Reset the Motion Plus controller if needed
            if motion_plus_mode != 0 {
                self.activate_motion_plus_with_mode(link, motion_plus_mode);
            }
            break;
        }
        extension_controller_type
    }

    /// Translation of `UpdateDeviceIdentity()`.
    fn update_device_identity(&self, device: &mut DeviceCtx<'_>) {
        device.set_device_name(match self.extension_controller_type {
            ExtensionControllerType::None => "Nintendo Wii Remote",
            ExtensionControllerType::Nunchuk => "Nintendo Wii Remote with Nunchuk",
            ExtensionControllerType::Gamepad => "Nintendo Wii Remote with Classic Controller",
            ExtensionControllerType::WiiUPro => "Nintendo Wii U Pro Controller",
            ExtensionControllerType::Unknown => "Nintendo Wii Remote with Unknown Extension",
        });
        device.set_guid_byte(15, self.extension_controller_type as u8);
    }

    /// `HIDAPI_DriverWii_InitDevice()` on a link.
    fn init(&mut self, device: &mut DeviceCtx<'_>, link: &dyn WiiLink) -> Result<()> {
        if device.vendor_id() == USB_VENDOR_NINTENDO {
            self.extension_controller_type = self.read_extension_controller_type(link);

            self.update_device_identity(device);
        }
        device.joystick_connected();
        Ok(())
    }

    /// `HIDAPI_DriverWii_OpenJoystick()` on a link.
    fn open(&mut self, link: &dyn WiiLink, joystick: &mut JoystickData) {
        crate::joystick::assert_joysticks_locked();

        self.joystick = Some(joystick.instance_id);

        self.initialize_extension(link);

        (self.motion_plus_present, self.motion_plus_mode) = self.get_motion_plus_state(link);

        if self.needs_periodic_motion_plus_check(false) {
            self.schedule_periodic_motion_plus_check(link);
        }

        if matches!(
            self.extension_controller_type,
            ExtensionControllerType::None | ExtensionControllerType::Nunchuk
        ) {
            joystick.add_sensor(SensorType::Accel, 100.0);
            if self.extension_controller_type == ExtensionControllerType::Nunchuk {
                joystick.add_sensor(SensorType::AccelL, 100.0);
            }

            if self.motion_plus_present {
                joystick.add_sensor(SensorType::Gyro, 100.0);
            }
        }

        // Initialize player index (needed for setting LEDs)
        self.player_index = joystick_player_index_for_id(joystick.instance_id);
        self.player_lights = hints::get_bool(hints::JOYSTICK_HIDAPI_WII_PLAYER_LED, true);
        self.update_slot_led(link);

        self.player_led_hint = Some(HintWatch::new(hints::JOYSTICK_HIDAPI_WII_PLAYER_LED));
        self.hint_changes(link);

        // Initialize the joystick capabilities
        joystick.nbuttons = if self.extension_controller_type == ExtensionControllerType::WiiUPro {
            15
        } else {
            // Maximum is Classic Controller + Wiimote
            usize::from(WII_BUTTON_MAX)
        };
        joystick.naxes = GamepadAxis::COUNT;

        self.last_input = link.ticks();
    }

    /// `HIDAPI_DriverWii_SetDevicePlayerIndex()` on a link.
    fn set_player_index(&mut self, link: &dyn WiiLink, player_index: i32) {
        if self.joystick.is_none() {
            return;
        }

        self.player_index = player_index;

        self.update_slot_led(link);
    }

    /// `HIDAPI_DriverWii_RumbleJoystick()` on a link.
    fn rumble(
        &mut self,
        link: &dyn WiiLink,
        low_frequency_rumble: u16,
        high_frequency_rumble: u16,
    ) {
        let active = low_frequency_rumble != 0 || high_frequency_rumble != 0;

        if active != self.rumble_active {
            let data = [OUTPUT_RUMBLE, u8::from(active)];
            self.write_output(link, &data, false);

            self.rumble_active = active;
        }
    }

    /// `HIDAPI_DriverWii_SetJoystickSensorsEnabled()` on a link.
    fn set_sensors_enabled(&mut self, link: &dyn WiiLink, enabled: bool) {
        if enabled != self.report_sensors {
            self.report_sensors = enabled;

            if self.motion_plus_present {
                if enabled {
                    self.activate_motion_plus(link);
                } else {
                    self.deactivate_motion_plus(link);
                }
            }

            self.reset_button_packet_type(link);
        }
    }

    /// Translation of `HandleWiiUProButtonData()`.
    fn handle_wiiu_pro_button_data(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: &mut JoystickRef<'_>,
        data: &ButtonData,
    ) {
        const AXES: [GamepadAxis; 4] = [
            GamepadAxis::LeftX,
            GamepadAxis::RightX,
            GamepadAxis::LeftY,
            GamepadAxis::RightY,
        ];
        let id = joystick_id(joystick);
        let timestamp = self.timestamp;

        if data.n_extension_bytes < 11 {
            return;
        }

        // Buttons
        post_packed_button_data(
            device,
            timestamp,
            id,
            &GAMEPAD_BUTTON_DEFS,
            &data.extension[8..11],
            false,
            true,
        );

        // Triggers
        let zl = data.extension[9] & 0x80;
        let zr = data.extension[9] & 0x04;
        device.send_axis(
            timestamp,
            id,
            GamepadAxis::LeftTrigger as u8,
            digital_trigger(zl != 0),
        );
        device.send_axis(
            timestamp,
            id,
            GamepadAxis::RightTrigger as u8,
            digital_trigger(zr != 0),
        );

        // Sticks
        for (i, axis) in AXES.into_iter().enumerate() {
            let value = u16::from_le_bytes([data.extension[i * 2], data.extension[i * 2 + 1]]);
            post_stick_calibrated(
                device,
                timestamp,
                id,
                &mut self.stick_calibration_data[i],
                axis,
                value,
            );
        }

        // Power
        self.update_power_level_wiiu(device, joystick, data.extension[10]);
    }

    /// Translation of `HandleGamepadControllerButtonData()`.
    fn handle_gamepad_controller_button_data(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: JoystickID,
        data: &ButtonData,
    ) {
        let motion_plus = self.motion_plus_mode == WII_MOTIONPLUS_MODE_GAMEPAD;
        let buttons = if motion_plus {
            &MP_GAMEPAD_BUTTON_DEFS
        } else {
            &GAMEPAD_BUTTON_DEFS
        };
        let timestamp = self.timestamp;

        if data.n_extension_bytes < 6 {
            return;
        }

        // Buttons
        post_packed_button_data(
            device,
            timestamp,
            joystick,
            buttons,
            &data.extension[4..6],
            false,
            true,
        );
        if motion_plus {
            post_packed_button_data(
                device,
                timestamp,
                joystick,
                &MP_FIXUP_DPAD_BUTTON_DEFS,
                &data.extension[..2],
                false,
                true,
            );
        }

        // Triggers
        let zl = data.extension[5] & 0x80;
        let zr = data.extension[5] & 0x04;
        device.send_axis(
            timestamp,
            joystick,
            GamepadAxis::LeftTrigger as u8,
            digital_trigger(zl != 0),
        );
        device.send_axis(
            timestamp,
            joystick,
            GamepadAxis::RightTrigger as u8,
            digital_trigger(zr != 0),
        );

        // Sticks
        let ext = &data.extension;
        let (lx, ly) = if motion_plus {
            (ext[0] & 0x3E, ext[1] & 0x3E)
        } else {
            (ext[0] & 0x3F, ext[1] & 0x3F)
        };
        let rx = (ext[2] >> 7) | ((ext[1] >> 5) & 0x06) | ((ext[0] >> 3) & 0x18);
        let ry = ext[2] & 0x1F;
        for (i, (axis, value)) in [
            (GamepadAxis::LeftX, lx),
            (GamepadAxis::LeftY, ly),
            (GamepadAxis::RightX, rx),
            (GamepadAxis::RightY, ry),
        ]
        .into_iter()
        .enumerate()
        {
            post_stick_calibrated(
                device,
                timestamp,
                joystick,
                &mut self.stick_calibration_data[i],
                axis,
                u16::from(value),
            );
        }
    }

    /// Translation of `HandleWiiRemoteButtonData()`.
    fn handle_wii_remote_button_data(
        &self,
        device: &mut DeviceCtx<'_>,
        joystick: JoystickID,
        data: &ButtonData,
    ) {
        if data.has_base_buttons {
            post_packed_button_data(
                device,
                self.timestamp,
                joystick,
                &WII_REMOTE_BUTTON_DEFS,
                &data.base_buttons,
                true,
                false,
            );
        }
    }

    /// Translation of `HandleWiiRemoteButtonDataAsMainController()`.
    fn handle_wii_remote_button_data_as_main_controller(
        &self,
        device: &mut DeviceCtx<'_>,
        joystick: JoystickID,
        data: &ButtonData,
    ) {
        if data.has_base_buttons {
            post_packed_button_data(
                device,
                self.timestamp,
                joystick,
                &WII_REMOTE_MAIN_CONTROLLER_BUTTON_DEFS,
                &data.base_buttons,
                true,
                false,
            );
        }
    }

    /// Translation of `HandleNunchuckButtonData()`.
    fn handle_nunchuck_button_data(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: JoystickID,
        data: &ButtonData,
    ) {
        let timestamp = self.timestamp;
        let ext = &data.extension;

        if data.n_extension_bytes < 6 {
            return;
        }

        let motion_plus = self.motion_plus_mode == WII_MOTIONPLUS_MODE_NUNCHUK;
        let (c_button, z_button) = if motion_plus {
            (ext[5] & 0x08 == 0, ext[5] & 0x04 == 0)
        } else {
            (ext[5] & 0x02 == 0, ext[5] & 0x01 == 0)
        };
        device.send_button(
            timestamp,
            joystick,
            GamepadButton::LeftShoulder as u8,
            c_button,
        );
        device.send_axis(
            timestamp,
            joystick,
            GamepadAxis::LeftTrigger as u8,
            if z_button { i16::MAX } else { i16::MIN },
        );
        post_stick_calibrated(
            device,
            timestamp,
            joystick,
            &mut self.stick_calibration_data[0],
            GamepadAxis::LeftX,
            u16::from(ext[0]),
        );
        post_stick_calibrated(
            device,
            timestamp,
            joystick,
            &mut self.stick_calibration_data[1],
            GamepadAxis::LeftY,
            u16::from(ext[1]),
        );

        if self.report_sensors {
            const ACCEL_RES_PER_G: f32 = 200.0;

            let mut x = i16::from(ext[2]) << 2;
            let mut y = i16::from(ext[3]) << 2;
            let mut z = i16::from(ext[4]) << 2;

            if motion_plus {
                x |= i16::from((ext[5] >> 3) & 0x02);
                y |= i16::from((ext[5] >> 4) & 0x02);
                z &= !0x04;
                z |= i16::from((ext[5] >> 5) & 0x06);
            } else {
                x |= i16::from((ext[5] >> 2) & 0x03);
                y |= i16::from((ext[5] >> 4) & 0x03);
                z |= i16::from((ext[5] >> 6) & 0x03);
            }

            x -= 0x200;
            y -= 0x200;
            z -= 0x200;

            let values = [
                -(f32::from(x) / ACCEL_RES_PER_G) * STANDARD_GRAVITY,
                (f32::from(z) / ACCEL_RES_PER_G) * STANDARD_GRAVITY,
                (f32::from(y) / ACCEL_RES_PER_G) * STANDARD_GRAVITY,
            ];
            device.send_sensor(timestamp, joystick, SensorType::AccelL, timestamp, &values);
        }
    }

    /// Translation of `HandleMotionPlusData()`.
    fn handle_motion_plus_data(
        &self,
        device: &mut DeviceCtx<'_>,
        joystick: JoystickID,
        data: &ButtonData,
    ) {
        if self.report_sensors {
            const GYRO_RES_PER_DEGREE: f32 = 8192.0;
            let ext = &data.extension;

            let raw =
                |low: u8, high: u8| (i32::from(low) | ((i32::from(high) << 6) & 0xFF00)) - 8192;
            let mut x = raw(ext[0], ext[3]);
            let mut y = raw(ext[1], ext[4]);
            let mut z = raw(ext[2], ext[5]);

            // Slow rotation rate: 8192/440 units per deg/s
            // Fast rotation rate: 8192/2000 units per deg/s
            let rate = |slow: bool| if slow { 440 } else { 2000 };
            x *= rate(ext[3] & 0x02 != 0);
            y *= rate(ext[4] & 0x02 != 0);
            z *= rate(ext[3] & 0x01 != 0);

            let pi = std::f32::consts::PI;
            let values = [
                -(z as f32 / GYRO_RES_PER_DEGREE) * pi / 180.0,
                (x as f32 / GYRO_RES_PER_DEGREE) * pi / 180.0,
                (y as f32 / GYRO_RES_PER_DEGREE) * pi / 180.0,
            ];
            device.send_sensor(
                self.timestamp,
                joystick,
                SensorType::Gyro,
                self.timestamp,
                &values,
            );
        }
    }

    /// Translation of `HandleWiiRemoteAccelData()`.
    fn handle_wii_remote_accel_data(
        &self,
        device: &mut DeviceCtx<'_>,
        joystick: JoystickID,
        data: &ButtonData,
    ) {
        const ACCEL_RES_PER_G: f32 = 100.0;

        if !self.report_sensors {
            return;
        }

        // FIXME (upstream): this doesn't check `hasAccelerometer`, so the
        // status and acknowledge reports, and the button reports without
        // accelerometer data, report the zeroed accelerometer bytes as a
        // reading of (-0x200, -0x200, -0x200) while sensors are enabled.
        let accel = &data.accelerometer;
        let buttons = &data.base_buttons;
        let x = ((i16::from(accel[0]) << 2) | i16::from((buttons[0] >> 5) & 0x03)) - 0x200;
        let y = ((i16::from(accel[1]) << 2) | i16::from((buttons[1] >> 4) & 0x02)) - 0x200;
        let z = ((i16::from(accel[2]) << 2) | i16::from((buttons[1] >> 5) & 0x02)) - 0x200;

        let values = [
            -(f32::from(x) / ACCEL_RES_PER_G) * STANDARD_GRAVITY,
            (f32::from(z) / ACCEL_RES_PER_G) * STANDARD_GRAVITY,
            (f32::from(y) / ACCEL_RES_PER_G) * STANDARD_GRAVITY,
        ];
        device.send_sensor(
            self.timestamp,
            joystick,
            SensorType::Accel,
            self.timestamp,
            &values,
        );
    }

    /// Translation of `HandleButtonData()`.
    fn handle_button_data(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: &mut JoystickRef<'_>,
        data: &mut ButtonData,
    ) {
        if self.extension_controller_type == ExtensionControllerType::WiiUPro {
            self.handle_wiiu_pro_button_data(device, joystick, data);
            return;
        }
        let id = joystick_id(joystick);

        if self.motion_plus_mode != WII_MOTIONPLUS_MODE_NONE && data.n_extension_bytes > 5 {
            if data.extension[5] & 0x01 != 0 {
                // The data is invalid, possibly during a hotplug
                return;
            }

            if data.extension[4] & 0x01 != 0 {
                if self.extension_controller_type == ExtensionControllerType::None {
                    // Something was plugged into the extension port, reinitialize to get new state
                    self.disconnected = true;
                }
            } else if self.extension_controller_type != ExtensionControllerType::None {
                // Something was removed from the extension port, reinitialize to get new state
                self.disconnected = true;
            }

            if data.extension[5] & 0x02 != 0 {
                self.handle_motion_plus_data(device, id, data);

                // The extension data is consumed
                data.n_extension_bytes = 0;
            }
        }

        self.handle_wii_remote_button_data(device, id, data);
        match self.extension_controller_type {
            ExtensionControllerType::Nunchuk => {
                self.handle_nunchuck_button_data(device, id, data);
                // (falls through to the Wii Remote as the main controller)
                self.handle_wii_remote_button_data_as_main_controller(device, id, data);
            }
            ExtensionControllerType::None => {
                self.handle_wii_remote_button_data_as_main_controller(device, id, data);
            }
            ExtensionControllerType::Gamepad => {
                self.handle_gamepad_controller_button_data(device, id, data);
            }
            _ => {}
        }
        self.handle_wii_remote_accel_data(device, id, data);
    }

    /// Translation of `HandleStatus()`.
    fn handle_status(
        &mut self,
        device: &mut DeviceCtx<'_>,
        link: &dyn WiiLink,
        joystick: &mut JoystickRef<'_>,
    ) {
        let had_extension = self.extension_controller_type != ExtensionControllerType::None;
        let has_extension = self.read_buffer[3] & 2 != 0;
        let mut data = ButtonData::default();
        get_base_buttons(&mut data, &self.read_buffer[1..]);
        self.handle_button_data(device, joystick, &mut data);

        if self.extension_controller_type != ExtensionControllerType::WiiUPro {
            // Wii U has separate battery level tracking
            self.update_power_level_wii(device, joystick_id(joystick), self.read_buffer[6]);
        }

        // The report data format has been reset, need to update it
        self.reset_button_packet_type(link);

        crate::log::debug!(
            crate::log::Category::Input,
            "HIDAPI Wii: Status update, extension {}",
            if has_extension {
                "CONNECTED"
            } else {
                "DISCONNECTED"
            }
        );

        /* When Motion Plus is active, we get extension connect/disconnect status
         * through the Motion Plus packets. Otherwise we can use the status here.
         */
        if self.motion_plus_mode != WII_MOTIONPLUS_MODE_NONE {
            /* Check to make sure the Motion Plus extension state hasn't changed,
             * otherwise we'll get extension connect/disconnect status through
             * Motion Plus packets.
             */
            if self.needs_periodic_motion_plus_check(true) {
                self.next_motion_plus_check = link.ticks();
            }
        } else if had_extension != has_extension {
            // Reinitialize to get new state
            self.disconnected = true;
        }
    }

    /// Translation of `HandleResponse()`.
    fn handle_response(
        &mut self,
        device: &mut DeviceCtx<'_>,
        link: &dyn WiiLink,
        joystick: &mut JoystickRef<'_>,
    ) {
        debug_assert!(matches!(
            self.read_buffer[0],
            INPUT_ACKNOWLEDGE | INPUT_READ_MEMORY
        ));
        let mut data = ButtonData::default();
        get_base_buttons(&mut data, &self.read_buffer[1..]);
        self.handle_button_data(device, joystick, &mut data);

        match self.comm_state {
            CommunicationState::None => {}

            CommunicationState::CheckMotionPlusStage1
            | CommunicationState::CheckMotionPlusStage2 => {
                let stage = if self.comm_state == CommunicationState::CheckMotionPlusStage1 {
                    1
                } else {
                    2
                };
                if let Ok(extension) = self.parse_extension_identify_response() {
                    if (extension & WII_EXTENSION_MOTIONPLUS_MASK) == WII_EXTENSION_MOTIONPLUS_ID {
                        // Motion Plus is currently active
                        crate::log::debug!(
                            crate::log::Category::Input,
                            "HIDAPI Wii: Motion Plus CONNECTED (stage {})",
                            stage
                        );

                        if !self.motion_plus_present {
                            // Reinitialize to get new sensor availability
                            self.disconnected = true;
                        }
                        self.comm_state = CommunicationState::None;
                    } else if self.comm_state == CommunicationState::CheckMotionPlusStage1 {
                        // Check to see if Motion Plus is present
                        let _ = self.read_register(link, 0xA600FE, 2, false);

                        self.comm_state = CommunicationState::CheckMotionPlusStage2;
                    } else {
                        // Motion Plus is not present
                        crate::log::debug!(
                            crate::log::Category::Input,
                            "HIDAPI Wii: Motion Plus DISCONNECTED (stage {})",
                            stage
                        );

                        if self.motion_plus_present {
                            // Reinitialize to get new sensor availability
                            self.disconnected = true;
                        }
                        self.comm_state = CommunicationState::None;
                    }
                }
            }
        }
    }

    /// Translation of `HandleButtonPacket()`.
    fn handle_button_packet(
        &mut self,
        device: &mut DeviceCtx<'_>,
        link: &dyn WiiLink,
        joystick: &mut JoystickRef<'_>,
    ) {
        let expected_report = self.get_button_packet_type();
        let buf = self.read_buffer;

        // FIXME: This should see if the data format is compatible rather than equal
        if expected_report != buf[0] {
            crate::log::debug!(
                crate::log::Category::Input,
                "HIDAPI Wii: Resetting report mode to {}",
                expected_report
            );
            self.request_button_packet_type(link, expected_report);
        }

        // IR camera data is not supported
        let mut data = ButtonData::default();
        match buf[0] {
            INPUT_BUTTON_DATA_0 => {
                // 30 BB BB
                get_base_buttons(&mut data, &buf[1..]);
            }
            INPUT_BUTTON_DATA_1 | INPUT_BUTTON_DATA_3 => {
                // 31 BB BB AA AA AA
                // 33 BB BB AA AA AA II II II II II II II II II II II II
                get_base_buttons(&mut data, &buf[1..]);
                get_accelerometer(&mut data, &buf[3..]);
            }
            INPUT_BUTTON_DATA_2 => {
                // 32 BB BB EE EE EE EE EE EE EE EE
                get_base_buttons(&mut data, &buf[1..]);
                get_extension_data(&mut data, &buf[3..], 8);
            }
            INPUT_BUTTON_DATA_4 => {
                // 34 BB BB EE EE EE EE EE EE EE EE EE EE EE EE EE EE EE EE EE EE EE
                get_base_buttons(&mut data, &buf[1..]);
                get_extension_data(&mut data, &buf[3..], 19);
            }
            INPUT_BUTTON_DATA_5 => {
                // 35 BB BB AA AA AA EE EE EE EE EE EE EE EE EE EE EE EE EE EE EE EE
                get_base_buttons(&mut data, &buf[1..]);
                get_accelerometer(&mut data, &buf[3..]);
                get_extension_data(&mut data, &buf[6..], 16);
            }
            INPUT_BUTTON_DATA_6 => {
                // 36 BB BB II II II II II II II II II II EE EE EE EE EE EE EE EE EE
                get_base_buttons(&mut data, &buf[1..]);
                get_extension_data(&mut data, &buf[13..], 9);
            }
            INPUT_BUTTON_DATA_7 => {
                // 37 BB BB AA AA AA II II II II II II II II II II EE EE EE EE EE EE
                get_base_buttons(&mut data, &buf[1..]);
                get_extension_data(&mut data, &buf[16..], 6);
            }
            INPUT_BUTTON_DATA_D => {
                // 3d EE EE EE EE EE EE EE EE EE EE EE EE EE EE EE EE EE EE EE EE EE
                get_extension_data(&mut data, &buf[1..], 21);
            }
            // (k_eWiiInputReportIDs_ButtonDataE and ButtonDataF too)
            _ => {
                crate::log::debug!(
                    crate::log::Category::Input,
                    "HIDAPI Wii: Unsupported button data type {:02x}",
                    buf[0]
                );
                return;
            }
        }
        self.handle_button_data(device, joystick, &mut data);
    }

    /// Translation of `HandleInput()`.
    fn handle_input(
        &mut self,
        device: &mut DeviceCtx<'_>,
        link: &dyn WiiLink,
        joystick: &mut JoystickRef<'_>,
    ) {
        let report_type = self.read_buffer[0];

        // Set up for handling input
        self.timestamp = link.ticks_ns();

        if report_type == INPUT_STATUS {
            self.handle_status(device, link, joystick);
        } else if report_type == INPUT_ACKNOWLEDGE || report_type == INPUT_READ_MEMORY {
            self.handle_response(device, link, joystick);
        } else if (INPUT_BUTTON_DATA_0..=INPUT_BUTTON_DATA_F).contains(&report_type) {
            self.handle_button_packet(device, link, joystick);
        } else {
            crate::log::debug!(
                crate::log::Category::Input,
                "HIDAPI Wii: Unexpected input packet of type {:x}",
                report_type
            );
        }
    }

    /// `HIDAPI_DriverWii_UpdateDevice()` on a link, with the joystick if
    /// it is open.
    fn update(
        &mut self,
        device: &mut DeviceCtx<'_>,
        link: &dyn WiiLink,
        mut joystick: Option<JoystickRef<'_>>,
    ) -> bool {
        let Some(&first) = device.joysticks().first() else {
            return false;
        };

        // (the hint callback of upstream)
        if joystick.is_some() && self.joystick.is_some() {
            self.hint_changes(link);
        }

        let now = link.ticks();

        // (upstream's last `size`: a read error is negative)
        let mut ok = loop {
            match self.read_input(link) {
                Ok(0) => break true,
                Err(_) => break false,
                Ok(_) => {}
            }
            if let Some(joystick) = joystick.as_mut() {
                self.handle_input(device, link, joystick);
            }
            self.last_input = now;
        };

        /* Check to see if we've lost connection to the controller.
         * We have continuous reporting enabled, so this should be reliable now.
         */
        const _: () = assert!(ENABLE_CONTINUOUS_REPORTING);
        if now >= self.last_input + INPUT_WAIT_TIMEOUT_MS {
            // Bluetooth may have disconnected, try reopening the controller
            ok = false;
        }

        // These checks aren't needed on the Wii U Pro Controller
        if joystick.is_some() && self.extension_controller_type != ExtensionControllerType::WiiUPro
        {
            // Check to see if the Motion Plus extension status has changed
            if self.next_motion_plus_check != 0 && now >= self.next_motion_plus_check {
                self.check_motion_plus_connection(link);
                if self.needs_periodic_motion_plus_check(false) {
                    self.schedule_periodic_motion_plus_check(link);
                } else {
                    self.next_motion_plus_check = 0;
                }
            }

            // Request a status update periodically to make sure our battery value is up to date
            if self.last_status == 0 || now >= self.last_status + STATUS_UPDATE_TIME_MS {
                let data = [OUTPUT_STATUS_REQUEST, u8::from(self.rumble_active)];
                self.write_output(link, &data, false);

                self.last_status = now;
            }
        }

        if !ok || self.disconnected {
            // Read error, device is disconnected
            device.joystick_disconnected(first);
        }
        ok
    }
}

impl DriverContext for WiiContext {
    /// Translation of `HIDAPI_DriverWii_InitDevice()`.
    fn init_device(&mut self, device: &mut DeviceCtx<'_>) -> Result<()> {
        let dev = device.device().clone();
        self.init(device, &dev)
    }

    /// Translation of `HIDAPI_DriverWii_SetDevicePlayerIndex()`.
    fn set_device_player_index(
        &mut self,
        device: &mut DeviceCtx<'_>,
        _instance_id: JoystickID,
        player_index: i32,
    ) {
        self.set_player_index(device.device(), player_index);
    }

    /// Translation of `HIDAPI_DriverWii_UpdateDevice()`.
    fn update_device(&mut self, device: &mut DeviceCtx<'_>) -> bool {
        let dev = device.device().clone();
        let joystick = device.open_joystick_id().map(JoystickRef::Open);
        self.update(device, &dev, joystick)
    }

    /// Translation of `HIDAPI_DriverWii_OpenJoystick()`.
    fn open_joystick(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: &mut JoystickData,
    ) -> Result<()> {
        let dev = device.device().clone();
        self.open(&dev, joystick);
        Ok(())
    }

    /// Translation of `HIDAPI_DriverWii_RumbleJoystick()`.
    fn rumble_joystick(
        &mut self,
        device: &mut DeviceCtx<'_>,
        _joystick: JoystickID,
        low_frequency_rumble: u16,
        high_frequency_rumble: u16,
    ) -> Result<()> {
        self.rumble(device.device(), low_frequency_rumble, high_frequency_rumble);
        Ok(())
    }

    /// Translation of `HIDAPI_DriverWii_GetJoystickCapabilities()`.
    fn get_joystick_capabilities(
        &mut self,
        _device: &mut DeviceCtx<'_>,
        _joystick: JoystickID,
    ) -> JoystickCaps {
        JoystickCaps::RUMBLE
    }

    /// Translation of `HIDAPI_DriverWii_SetJoystickSensorsEnabled()`.
    fn set_joystick_sensors_enabled(
        &mut self,
        device: &mut DeviceCtx<'_>,
        _joystick: JoystickID,
        enabled: bool,
    ) -> Result<()> {
        self.set_sensors_enabled(device.device(), enabled);
        Ok(())
    }

    /// Translation of `HIDAPI_DriverWii_CloseJoystick()`.
    fn close_joystick(&mut self, _device: &mut DeviceCtx<'_>, _joystick: JoystickID) {
        self.player_led_hint = None;

        self.joystick = None;
    }
}

#[cfg(test)]
mod tests;
