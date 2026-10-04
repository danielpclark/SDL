// Rust translation of src/joystick/hidapi/SDL_hidapi_steamdeck.c from Simple
// DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Steam Deck's built-in controller driver.

use super::steam::controller_constants::*;
use super::steam::controller_structs::*;
use super::steam::{SteamHid, FEATURE_REPORT_BUFFER_SIZE};
use super::{DeviceCtx, DriverContext, DriverImpl, HidapiDevice, JoystickCaps, SDL_HIDAPI_DEFAULT};
use crate::error::{Error, Result};
use crate::events::JoystickID;
use crate::hints;
use crate::joystick::gamepad::{GamepadAxis, GamepadButton, GamepadCapSenseType, GamepadType};
use crate::joystick::{
    is_joystick_steam_deck, JoystickData, HAT_DOWN, HAT_LEFT, HAT_RIGHT, HAT_UP,
};
use crate::sensor::{SensorType, STANDARD_GRAVITY};

/// `SDL_GAMEPAD_BUTTON_STEAM_DECK_QAM`
const SDL_GAMEPAD_BUTTON_STEAM_DECK_QAM: u8 = 11;
/// `SDL_GAMEPAD_BUTTON_STEAM_DECK_RIGHT_PADDLE1`
const SDL_GAMEPAD_BUTTON_STEAM_DECK_RIGHT_PADDLE1: u8 = 12;
/// `SDL_GAMEPAD_BUTTON_STEAM_DECK_LEFT_PADDLE1`
const SDL_GAMEPAD_BUTTON_STEAM_DECK_LEFT_PADDLE1: u8 = 13;
/// `SDL_GAMEPAD_BUTTON_STEAM_DECK_RIGHT_PADDLE2`
const SDL_GAMEPAD_BUTTON_STEAM_DECK_RIGHT_PADDLE2: u8 = 14;
/// `SDL_GAMEPAD_BUTTON_STEAM_DECK_LEFT_PADDLE2`
const SDL_GAMEPAD_BUTTON_STEAM_DECK_LEFT_PADDLE2: u8 = 15;
/// `SDL_GAMEPAD_BUTTON_STEAM_DECK_RIGHT_TOUCHPAD`
const SDL_GAMEPAD_BUTTON_STEAM_DECK_RIGHT_TOUCHPAD: u8 = 16;
/// `SDL_GAMEPAD_BUTTON_STEAM_DECK_LEFT_TOUCHPAD`
const SDL_GAMEPAD_BUTTON_STEAM_DECK_LEFT_TOUCHPAD: u8 = 17;
/// `SDL_GAMEPAD_NUM_STEAM_DECK_BUTTONS`
const SDL_GAMEPAD_NUM_STEAM_DECK_BUTTONS: usize = 18;

// SteamDeckButtons
#[allow(dead_code)] // (as upstream)
const STEAMDECK_LBUTTON_R2: u32 = 0x00000001;
#[allow(dead_code)] // (as upstream)
const STEAMDECK_LBUTTON_L2: u32 = 0x00000002;
const STEAMDECK_LBUTTON_R: u32 = 0x00000004;
const STEAMDECK_LBUTTON_L: u32 = 0x00000008;
const STEAMDECK_LBUTTON_Y: u32 = 0x00000010;
const STEAMDECK_LBUTTON_B: u32 = 0x00000020;
const STEAMDECK_LBUTTON_X: u32 = 0x00000040;
const STEAMDECK_LBUTTON_A: u32 = 0x00000080;
const STEAMDECK_LBUTTON_DPAD_UP: u32 = 0x00000100;
const STEAMDECK_LBUTTON_DPAD_RIGHT: u32 = 0x00000200;
const STEAMDECK_LBUTTON_DPAD_LEFT: u32 = 0x00000400;
const STEAMDECK_LBUTTON_DPAD_DOWN: u32 = 0x00000800;
const STEAMDECK_LBUTTON_VIEW: u32 = 0x00001000;
const STEAMDECK_LBUTTON_STEAM: u32 = 0x00002000;
const STEAMDECK_LBUTTON_MENU: u32 = 0x00004000;
const STEAMDECK_LBUTTON_L5: u32 = 0x00008000;
const STEAMDECK_LBUTTON_R5: u32 = 0x00010000;
const STEAMDECK_LBUTTON_LEFT_PAD: u32 = 0x00020000;
const STEAMDECK_LBUTTON_RIGHT_PAD: u32 = 0x00040000;
const STEAMDECK_LBUTTON_LEFT_TOUCHPAD_TOUCH: u32 = 0x00080000;
const STEAMDECK_LBUTTON_RIGHT_TOUCHPAD_TOUCH: u32 = 0x00100000;
const STEAMDECK_LBUTTON_L3: u32 = 0x00400000;
const STEAMDECK_LBUTTON_R3: u32 = 0x04000000;

const STEAMDECK_HBUTTON_L4: u32 = 0x00000200;
const STEAMDECK_HBUTTON_R4: u32 = 0x00000400;
const STEAMDECK_HBUTTON_LSTICK_TOUCH: u32 = 0x00004000;
const STEAMDECK_HBUTTON_RSTICK_TOUCH: u32 = 0x00008000;
const STEAMDECK_HBUTTON_QAM: u32 = 0x00040000;

/// Translation of `SDL_DriverSteamDeck_Context`.
#[derive(Debug, Default)]
pub(crate) struct SteamDeckContext {
    update_rate_us: u32,
    sensor_timestamp_ns: u64,
    last_button_state: u64,
    watchdog_counter: u8,

    left_touch_down: bool,
    left_touch_x: f32,
    left_touch_y: f32,
    right_touch_down: bool,
    right_touch_x: f32,
    right_touch_y: f32,
}

/// The settings messages of the lizard mode functions: a feature report
/// (`buffer`, with the message at `buffer + 1`) of `ID_SET_SETTINGS_VALUES`.
fn settings_buffer(settings: &[(u8, u16)]) -> [u8; FEATURE_REPORT_BUFFER_SIZE] {
    let mut buffer = [0; FEATURE_REPORT_BUFFER_SIZE];
    write_set_settings_values(&mut buffer[1..], ID_SET_SETTINGS_VALUES, settings);
    buffer
}

/// Clear the digital mappings, then send a settings message and discard
/// the lingering report read back (the body of `DisableDeckLizardMode()`
/// and `FeedDeckLizardWatchdog()`).
fn clear_mappings_and_set(dev: &dyn SteamHid, settings: &[(u8, u16)]) -> bool {
    let mut buffer = [0; FEATURE_REPORT_BUFFER_SIZE];
    buffer[1] = ID_CLEAR_DIGITAL_MAPPINGS;

    if dev.send_feature_report(&buffer).ok() != Some(buffer.len()) {
        return false;
    }

    // (upstream rewrites the message in the same buffer)
    let mut buffer = settings_buffer(settings);

    if dev.send_feature_report(&buffer).ok() != Some(buffer.len()) {
        return false;
    }

    // There may be a lingering report read back after changing settings.
    // Discard it.
    let _ = dev.get_feature_report(&mut buffer);

    true
}

/// Translation of `DisableDeckLizardMode()`.
fn disable_deck_lizard_mode(dev: &dyn SteamHid) -> bool {
    clear_mappings_and_set(
        dev,
        &[
            (SETTING_SMOOTH_ABSOLUTE_MOUSE, 0),
            (SETTING_LEFT_TRACKPAD_MODE, TRACKPAD_NONE),
            (SETTING_RIGHT_TRACKPAD_MODE, TRACKPAD_NONE), // disable mouse
            (SETTING_LEFT_TRACKPAD_CLICK_PRESSURE, 0xFFFF), // disable clicky pad
            (SETTING_RIGHT_TRACKPAD_CLICK_PRESSURE, 0xFFFF), // disable clicky pad
        ],
    )
}

/// Translation of `FeedDeckLizardWatchdog()`.
fn feed_deck_lizard_watchdog(dev: &dyn SteamHid) -> bool {
    clear_mappings_and_set(dev, &[(SETTING_RIGHT_TRACKPAD_MODE, TRACKPAD_NONE)])
}

/// The rumble feature report of `HIDAPI_DriverSteamDeck_RumbleJoystick()`.
fn rumble_report(
    low_frequency_rumble: u16,
    high_frequency_rumble: u16,
) -> [u8; FEATURE_REPORT_BUFFER_SIZE] {
    let mut buffer = [0; FEATURE_REPORT_BUFFER_SIZE];
    buffer[1] = ID_TRIGGER_RUMBLE_CMD;
    MsgSimpleRumbleCmd {
        rumble_type: 0,
        intensity: HAPTIC_INTENSITY_SYSTEM,
        left_motor_speed: low_frequency_rumble,
        right_motor_speed: high_frequency_rumble,
        left_gain: 2,
        right_gain: 0,
    }
    .write(&mut buffer[3..]);
    buffer
}

/// `HIDAPI_DriverSteamDeck_RumbleJoystick()` on `dev`.
fn rumble(dev: &dyn SteamHid, low_frequency_rumble: u16, high_frequency_rumble: u16) -> Result<()> {
    let buffer = rumble_report(low_frequency_rumble, high_frequency_rumble);

    if dev.send_feature_report(&buffer).ok() != Some(buffer.len()) {
        // (upstream sets no error)
        return Err(Error::new("Couldn't send rumble feature report"));
    }
    Ok(())
}

impl SteamDeckContext {
    /// Translation of `HIDAPI_DriverSteamDeck_HandleState()`.
    fn handle_state(&mut self, device: &mut DeviceCtx<'_>, joystick: JoystickID, report: &[u8]) {
        let deck_state = SteamDeckStatePacket::parse(&report[VALVE_IN_REPORT_HEADER_SIZE..]);
        let timestamp = crate::timer::ticks_ns();

        if deck_state.buttons != self.last_button_state {
            let mut hat = 0;
            let buttons_l = deck_state.buttons_l();
            let buttons_h = deck_state.buttons_h();

            let mut button = |button: u8, down: bool| {
                device.send_button(timestamp, joystick, button, down);
            };
            button(
                GamepadButton::South as u8,
                buttons_l & STEAMDECK_LBUTTON_A != 0,
            );
            button(
                GamepadButton::East as u8,
                buttons_l & STEAMDECK_LBUTTON_B != 0,
            );
            button(
                GamepadButton::West as u8,
                buttons_l & STEAMDECK_LBUTTON_X != 0,
            );
            button(
                GamepadButton::North as u8,
                buttons_l & STEAMDECK_LBUTTON_Y != 0,
            );

            button(
                GamepadButton::LeftShoulder as u8,
                buttons_l & STEAMDECK_LBUTTON_L != 0,
            );
            button(
                GamepadButton::RightShoulder as u8,
                buttons_l & STEAMDECK_LBUTTON_R != 0,
            );

            button(
                GamepadButton::Back as u8,
                buttons_l & STEAMDECK_LBUTTON_VIEW != 0,
            );
            button(
                GamepadButton::Start as u8,
                buttons_l & STEAMDECK_LBUTTON_MENU != 0,
            );
            button(
                GamepadButton::Guide as u8,
                buttons_l & STEAMDECK_LBUTTON_STEAM != 0,
            );
            button(
                SDL_GAMEPAD_BUTTON_STEAM_DECK_QAM,
                buttons_h & STEAMDECK_HBUTTON_QAM != 0,
            );

            button(
                GamepadButton::LeftStick as u8,
                buttons_l & STEAMDECK_LBUTTON_L3 != 0,
            );
            button(
                GamepadButton::RightStick as u8,
                buttons_l & STEAMDECK_LBUTTON_R3 != 0,
            );

            button(
                SDL_GAMEPAD_BUTTON_STEAM_DECK_RIGHT_PADDLE1,
                buttons_h & STEAMDECK_HBUTTON_R4 != 0,
            );
            button(
                SDL_GAMEPAD_BUTTON_STEAM_DECK_LEFT_PADDLE1,
                buttons_h & STEAMDECK_HBUTTON_L4 != 0,
            );
            button(
                SDL_GAMEPAD_BUTTON_STEAM_DECK_RIGHT_PADDLE2,
                buttons_l & STEAMDECK_LBUTTON_R5 != 0,
            );
            button(
                SDL_GAMEPAD_BUTTON_STEAM_DECK_LEFT_PADDLE2,
                buttons_l & STEAMDECK_LBUTTON_L5 != 0,
            );

            button(
                SDL_GAMEPAD_BUTTON_STEAM_DECK_RIGHT_TOUCHPAD,
                buttons_l & STEAMDECK_LBUTTON_RIGHT_PAD != 0,
            );
            button(
                SDL_GAMEPAD_BUTTON_STEAM_DECK_LEFT_TOUCHPAD,
                buttons_l & STEAMDECK_LBUTTON_LEFT_PAD != 0,
            );

            device.send_capsense(
                timestamp,
                joystick,
                GamepadCapSenseType::RightStick,
                buttons_h & STEAMDECK_HBUTTON_RSTICK_TOUCH != 0,
            );
            device.send_capsense(
                timestamp,
                joystick,
                GamepadCapSenseType::LeftStick,
                buttons_h & STEAMDECK_HBUTTON_LSTICK_TOUCH != 0,
            );

            if buttons_l & STEAMDECK_LBUTTON_DPAD_UP != 0 {
                hat |= HAT_UP;
            }
            if buttons_l & STEAMDECK_LBUTTON_DPAD_DOWN != 0 {
                hat |= HAT_DOWN;
            }
            if buttons_l & STEAMDECK_LBUTTON_DPAD_LEFT != 0 {
                hat |= HAT_LEFT;
            }
            if buttons_l & STEAMDECK_LBUTTON_DPAD_RIGHT != 0 {
                hat |= HAT_RIGHT;
            }
            device.send_hat(timestamp, joystick, 0, hat);

            self.last_button_state = deck_state.buttons;
        }

        let trigger_axis = |trigger: u16| (i32::from(trigger) * 2 - 32768) as i16;
        let negated = |value: i16| (-i32::from(value)) as i16;
        device.send_axis(
            timestamp,
            joystick,
            GamepadAxis::LeftTrigger as u8,
            trigger_axis(deck_state.trigger_raw_l),
        );
        device.send_axis(
            timestamp,
            joystick,
            GamepadAxis::RightTrigger as u8,
            trigger_axis(deck_state.trigger_raw_r),
        );

        device.send_axis(
            timestamp,
            joystick,
            GamepadAxis::LeftX as u8,
            deck_state.left_stick_x,
        );
        device.send_axis(
            timestamp,
            joystick,
            GamepadAxis::LeftY as u8,
            negated(deck_state.left_stick_y),
        );
        device.send_axis(
            timestamp,
            joystick,
            GamepadAxis::RightX as u8,
            deck_state.right_stick_x,
        );
        device.send_axis(
            timestamp,
            joystick,
            GamepadAxis::RightY as u8,
            negated(deck_state.right_stick_y),
        );

        self.sensor_timestamp_ns += u64::from(self.update_rate_us) * super::NS_PER_US;

        let (gyro, accel) = valve_sensor_values(
            (deck_state.gyro_x, deck_state.gyro_y, deck_state.gyro_z),
            (deck_state.accel_x, deck_state.accel_y, deck_state.accel_z),
        );
        device.send_sensor(
            timestamp,
            joystick,
            SensorType::Gyro,
            self.sensor_timestamp_ns,
            &gyro,
        );
        device.send_sensor(
            timestamp,
            joystick,
            SensorType::Accel,
            self.sensor_timestamp_ns,
            &accel,
        );

        let left_touch_down = deck_state.buttons_l() & STEAMDECK_LBUTTON_LEFT_TOUCHPAD_TOUCH != 0;
        let right_touch_down = deck_state.buttons_l() & STEAMDECK_LBUTTON_RIGHT_TOUCHPAD_TOUCH != 0;
        if left_touch_down || self.left_touch_down {
            if left_touch_down {
                self.left_touch_x = f32::from(deck_state.left_pad_x) / 65536.0 + 0.5;
                self.left_touch_y = -f32::from(deck_state.left_pad_y) / 65536.0 + 0.5;
            }
            device.send_touchpad(
                timestamp,
                joystick,
                0,
                0,
                left_touch_down,
                self.left_touch_x,
                self.left_touch_y,
                f32::from(deck_state.pressure_pad_left) / 32768.0,
            );
            self.left_touch_down = left_touch_down;
        }
        if right_touch_down || self.right_touch_down {
            if right_touch_down {
                self.right_touch_x = f32::from(deck_state.right_pad_x) / 65536.0 + 0.5;
                self.right_touch_y = -f32::from(deck_state.right_pad_y) / 65536.0 + 0.5;
            }
            device.send_touchpad(
                timestamp,
                joystick,
                1,
                0,
                right_touch_down,
                self.right_touch_x,
                self.right_touch_y,
                f32::from(deck_state.pressure_pad_right) / 32768.0,
            );
            self.right_touch_down = right_touch_down;
        }
    }

    /// `HIDAPI_DriverSteamDeck_InitDevice()` on `dev`.
    fn init(&mut self, device: &mut DeviceCtx<'_>, dev: &dyn SteamHid) -> Result<()> {
        let mut data = [0u8; 64];

        // Always 1kHz according to USB descriptor, but actually about 4 ms.
        self.update_rate_us = 4000;

        // Read a report to see if this is the correct endpoint.
        // Mouse, Keyboard and Controller have the same VID/PID but
        // only the controller hidraw device receives hid reports.
        // (a read error goes on, as upstream)
        if matches!(dev.read_timeout(&mut data, 16), Ok(0)) {
            return Err(Error::new("Steam Deck controller sent no report"));
        }

        if !disable_deck_lizard_mode(dev) {
            return Err(Error::new("Couldn't disable the Steam Deck lizard mode"));
        }

        device.set_device_name("Steam Deck");

        device.joystick_connected();
        Ok(())
    }

    /// `HIDAPI_DriverSteamDeck_UpdateDevice()` on `dev`, for the device's
    /// open joystick.
    fn update(
        &mut self,
        device: &mut DeviceCtx<'_>,
        dev: &dyn SteamHid,
        joystick: JoystickID,
    ) -> bool {
        let watchdog_counter = self.watchdog_counter;
        self.watchdog_counter = watchdog_counter.wrapping_add(1);
        if watchdog_counter > 200 {
            self.watchdog_counter = 0;
            if !feed_deck_lizard_watchdog(dev) {
                return false;
            }
        }

        let mut data = [0u8; 64];

        loop {
            let r = match dev.read(&mut data) {
                Ok(r) => r,
                Err(_) => {
                    // Failed to read from controller
                    if let Some(&first) = device.joysticks().first() {
                        device.joystick_disconnected(first);
                    }
                    return false;
                }
            };
            if r == 64 {
                let header = ValveInReportHeader::parse(&data);
                if header.report_version == K_VALVE_IN_REPORT_MSG_VERSION
                    && header.report_type == ID_CONTROLLER_DECK_STATE
                    && header.length == 64
                {
                    self.handle_state(device, joystick, &data);
                }
            }
            if r == 0 {
                break;
            }
        }

        true
    }
}

/// The gyro and accelerometer values of a Steam Deck or Triton state, in
/// SDL's units and axes.
pub(crate) fn valve_sensor_values(
    gyro: (i16, i16, i16),
    accel: (i16, i16, i16),
) -> ([f32; 3], [f32; 3]) {
    let gyro_scale = 2000.0 * (std::f32::consts::PI / 180.0);
    let negated = |value: i16| -(i32::from(value)) as f32;
    let gyro_values = [
        (f32::from(gyro.0) / 32768.0) * gyro_scale,
        (f32::from(gyro.2) / 32768.0) * gyro_scale,
        (negated(gyro.1) / 32768.0) * gyro_scale,
    ];
    let accel_values = [
        (f32::from(accel.0) / 32768.0) * 2.0 * STANDARD_GRAVITY,
        (f32::from(accel.2) / 32768.0) * 2.0 * STANDARD_GRAVITY,
        (negated(accel.1) / 32768.0) * 2.0 * STANDARD_GRAVITY,
    ];
    (gyro_values, accel_values)
}

/// The Steam Deck driver's static functions.
pub(crate) struct SteamDeckDriver;

impl DriverImpl for SteamDeckDriver {
    /// Translation of `HIDAPI_DriverSteamDeck_RegisterHints()`.
    fn hints(&self) -> &'static [&'static str] {
        &[hints::JOYSTICK_HIDAPI_STEAMDECK]
    }

    /// Translation of `HIDAPI_DriverSteamDeck_IsEnabled()`.
    fn is_enabled(&self) -> bool {
        hints::get_bool(
            hints::JOYSTICK_HIDAPI_STEAMDECK,
            hints::get_bool(hints::JOYSTICK_HIDAPI, SDL_HIDAPI_DEFAULT),
        )
    }

    /// Translation of `HIDAPI_DriverSteamDeck_IsSupportedDevice()`.
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
        is_joystick_steam_deck(vendor_id, product_id)
    }

    fn new_context(&self) -> Box<dyn DriverContext> {
        Box::new(SteamDeckContext::default())
    }
}

impl DriverContext for SteamDeckContext {
    /// Translation of `HIDAPI_DriverSteamDeck_InitDevice()`.
    fn init_device(&mut self, device: &mut DeviceCtx<'_>) -> Result<()> {
        let dev = device.device().clone();
        self.init(device, &*dev)
    }

    /// Translation of `HIDAPI_DriverSteamDeck_UpdateDevice()`.
    fn update_device(&mut self, device: &mut DeviceCtx<'_>) -> bool {
        if device.num_joysticks() == 0 {
            return false;
        }
        let Some(joystick) = device.open_joystick_id() else {
            return false;
        };

        let dev = device.device().clone();
        self.update(device, &*dev, joystick)
    }
    /// Translation of `HIDAPI_DriverSteamDeck_OpenJoystick()`.
    fn open_joystick(
        &mut self,
        _device: &mut DeviceCtx<'_>,
        joystick: &mut JoystickData,
    ) -> Result<()> {
        let update_rate_in_hz = 1.0 / self.update_rate_us as f32 * 1.0e6;

        crate::joystick::assert_joysticks_locked();

        // Initialize the joystick capabilities
        joystick.nbuttons = SDL_GAMEPAD_NUM_STEAM_DECK_BUTTONS;
        joystick.naxes = GamepadAxis::COUNT;
        joystick.nhats = 1;

        joystick.add_sensor(SensorType::Gyro, update_rate_in_hz);
        joystick.add_sensor(SensorType::Accel, update_rate_in_hz);

        joystick.add_touchpad(1);
        joystick.add_touchpad(1);

        joystick.add_capsense(GamepadCapSenseType::LeftStick);
        joystick.add_capsense(GamepadCapSenseType::RightStick);

        Ok(())
    }

    /// Translation of `HIDAPI_DriverSteamDeck_RumbleJoystick()`.
    fn rumble_joystick(
        &mut self,
        device: &mut DeviceCtx<'_>,
        _joystick: JoystickID,
        low_frequency_rumble: u16,
        high_frequency_rumble: u16,
    ) -> Result<()> {
        rumble(
            &**device.device(),
            low_frequency_rumble,
            high_frequency_rumble,
        )
    }

    /// Translation of `HIDAPI_DriverSteamDeck_GetJoystickCapabilities()`.
    fn get_joystick_capabilities(
        &mut self,
        _device: &mut DeviceCtx<'_>,
        _joystick: JoystickID,
    ) -> JoystickCaps {
        JoystickCaps::RUMBLE
    }

    /// Translation of `HIDAPI_DriverSteamDeck_SetSensorsEnabled()`.
    fn set_joystick_sensors_enabled(
        &mut self,
        _device: &mut DeviceCtx<'_>,
        _joystick: JoystickID,
        _enabled: bool,
    ) -> Result<()> {
        // On steam deck, sensors are enabled by default. Nothing to do here.
        Ok(())
    }

    /// Translation of `HIDAPI_DriverSteamDeck_CloseJoystick()`.
    fn close_joystick(&mut self, _device: &mut DeviceCtx<'_>, _joystick: JoystickID) {
        // Lizard mode id automatically re-enabled by watchdog. Nothing to do here.
    }
}

#[cfg(test)]
mod tests;
