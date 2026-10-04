// Rust translation of src/joystick/hidapi/SDL_hidapi_steam_triton.c from
// Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Steam Controller (2026, "Triton") driver, wired or through its
//! Proteus and Nereid dongles.
//!
//! Not translated: the protocol debug logging (`DEBUG_STEAM_PROTOCOL`).

use super::steam::controller_constants::*;
use super::steam::controller_structs::*;
use super::steam::SteamHid;
use super::steamdeck::valve_sensor_values;
use super::{DeviceCtx, DriverContext, DriverImpl, HidapiDevice, JoystickCaps, SDL_HIDAPI_DEFAULT};
use crate::error::{Error, Result};
use crate::events::JoystickID;
use crate::hints;
use crate::joystick::gamepad::{GamepadAxis, GamepadButton, GamepadCapSenseType, GamepadType};
use crate::joystick::usb_ids::*;
use crate::joystick::{
    is_joystick_steam_triton, JoystickData, HAT_DOWN, HAT_LEFT, HAT_RIGHT, HAT_UP,
};
use crate::power::PowerState;
use crate::sensor::SensorType;

/// Always 1kHz according to USB descriptor, but actually about 4 ms.
const TRITON_SENSOR_UPDATE_INTERVAL_US: u32 = 4032;

/// Steam Controller hardware safety timeout is around 50ms, so we resend rumble every 40ms
const TRITON_RUMBLE_RESEND_INTERVAL_MS: u64 = 40;

/// `SDL_GAMEPAD_BUTTON_TRITON_QAM`
const SDL_GAMEPAD_BUTTON_TRITON_QAM: u8 = 11;
/// `SDL_GAMEPAD_BUTTON_TRITON_RIGHT_PADDLE1`
const SDL_GAMEPAD_BUTTON_TRITON_RIGHT_PADDLE1: u8 = 12;
/// `SDL_GAMEPAD_BUTTON_TRITON_LEFT_PADDLE1`
const SDL_GAMEPAD_BUTTON_TRITON_LEFT_PADDLE1: u8 = 13;
/// `SDL_GAMEPAD_BUTTON_TRITON_RIGHT_PADDLE2`
const SDL_GAMEPAD_BUTTON_TRITON_RIGHT_PADDLE2: u8 = 14;
/// `SDL_GAMEPAD_BUTTON_TRITON_LEFT_PADDLE2`
const SDL_GAMEPAD_BUTTON_TRITON_LEFT_PADDLE2: u8 = 15;
/// `SDL_GAMEPAD_BUTTON_TRITON_RIGHT_TOUCHPAD`
const SDL_GAMEPAD_BUTTON_TRITON_RIGHT_TOUCHPAD: u8 = 16;
/// `SDL_GAMEPAD_BUTTON_TRITON_LEFT_TOUCHPAD`
const SDL_GAMEPAD_BUTTON_TRITON_LEFT_TOUCHPAD: u8 = 17;
/// `SDL_GAMEPAD_NUM_TRITON_BUTTONS`
const SDL_GAMEPAD_NUM_TRITON_BUTTONS: usize = 18;

// TritonButtons
const TRITON_LBUTTON_A: u32 = 0x00000001;
const TRITON_LBUTTON_B: u32 = 0x00000002;
const TRITON_LBUTTON_X: u32 = 0x00000004;
const TRITON_LBUTTON_Y: u32 = 0x00000008;

const TRITON_HBUTTON_QAM: u32 = 0x00000010;
const TRITON_LBUTTON_R3: u32 = 0x00000020;
const TRITON_LBUTTON_VIEW: u32 = 0x00000040;
const TRITON_HBUTTON_R4: u32 = 0x00000080;

const TRITON_LBUTTON_R5: u32 = 0x00000100;
const TRITON_LBUTTON_R: u32 = 0x00000200;
const TRITON_LBUTTON_DPAD_DOWN: u32 = 0x00000400;
const TRITON_LBUTTON_DPAD_RIGHT: u32 = 0x00000800;

const TRITON_LBUTTON_DPAD_LEFT: u32 = 0x00001000;
const TRITON_LBUTTON_DPAD_UP: u32 = 0x00002000;
const TRITON_LBUTTON_MENU: u32 = 0x00004000;
const TRITON_LBUTTON_L3: u32 = 0x00008000;

const TRITON_LBUTTON_STEAM: u32 = 0x00010000;
const TRITON_HBUTTON_L4: u32 = 0x00020000;
const TRITON_LBUTTON_L5: u32 = 0x00040000;
const TRITON_LBUTTON_L: u32 = 0x00080000;

const TRITON_RIGHT_JOYSTICK_TOUCH: u32 = 0x00100000;
const TRITON_RIGHT_TOUCHPAD_TOUCH: u32 = 0x00200000;
const TRITON_RIGHT_TOUCHPAD_CLICK: u32 = 0x00400000;
#[allow(dead_code)] // (as upstream)
const TRITON_RIGHT_TRIGGER_CLICK: u32 = 0x00800000;

const TRITON_LEFT_JOYSTICK_TOUCH: u32 = 0x01000000;
const TRITON_LEFT_TOUCHPAD_TOUCH: u32 = 0x02000000;
const TRITON_LEFT_TOUCHPAD_CLICK: u32 = 0x04000000;
#[allow(dead_code)] // (as upstream)
const TRITON_LEFT_TRIGGER_CLICK: u32 = 0x08000000;

const TRITON_RIGHT_GRIP_TOUCH: u32 = 0x10000000;
const TRITON_LEFT_GRIP_TOUCH: u32 = 0x20000000;

/// Translation of `SDL_DriverSteamTriton_Context`.
#[derive(Debug, Default)]
pub(crate) struct SteamTritonContext {
    connected: bool,
    report_sensors: bool,
    last_sensor_tick16: u16,
    last_sensor_tick32: u32,
    sensor_timestamp_ns: u64,
    last_button_state: u64,
    last_lizard_update: u64,
    low_frequency_rumble: u16,
    high_frequency_rumble: u16,
    last_rumble_time: u64,

    left_touch_down: bool,
    left_touch_x: f32,
    left_touch_y: f32,
    right_touch_down: bool,
    right_touch_x: f32,
    right_touch_y: f32,
}

/// Translation of `IsProteusDongle()`.
fn is_proteus_dongle(product_id: u16) -> bool {
    product_id == USB_PRODUCT_VALVE_STEAM_PROTEUS_DONGLE
        || product_id == USB_PRODUCT_VALVE_STEAM_NEREID_DONGLE
}

/// The 64-byte feature report of report ID 1 with a settings message
/// (`buffer`, with the message at `buffer + 1`).
fn settings_report(settings: &[(u8, u16)]) -> [u8; HID_FEATURE_REPORT_BYTES] {
    let mut buffer = [0; HID_FEATURE_REPORT_BYTES];
    buffer[0] = 1;
    write_set_settings_values(&mut buffer[1..], ID_SET_SETTINGS_VALUES, settings);
    buffer
}

/// Translation of `DisableSteamTritonLizardMode()`.
fn disable_steam_triton_lizard_mode(dev: &dyn SteamHid) -> bool {
    let buffer = settings_report(&[(SETTING_LIZARD_MODE, LIZARD_MODE_OFF)]);

    dev.send_feature_report(&buffer).ok() == Some(buffer.len())
}

/// The touchpads of a state report.
#[derive(Clone, Copy, Debug)]
struct TritonTouchpads {
    buttons: u32,
    left_pad_x: i16,
    left_pad_y: i16,
    pressure_left: u16,
    right_pad_x: i16,
    right_pad_y: i16,
    pressure_right: u16,
}

/// The IMU values of a state report, with the time since the last one in
/// microseconds, if it changed.
#[derive(Clone, Copy, Debug)]
struct TritonImu {
    delta_us: u32,
    gyro: (i16, i16, i16),
    accel: (i16, i16, i16),
}

impl SteamTritonContext {
    /// Translation of `HIDAPI_DriverSteamTriton_HandleGenericState()`.
    /// Triton newer state MTUs are identical until touchpads. Parse them
    /// using this routine.
    fn handle_generic_state(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: JoystickID,
        timestamp: u64,
        report: &TritonMtuNoQuat,
    ) {
        let buttons = report.buttons;
        if u64::from(buttons) != self.last_button_state {
            let mut hat = 0;

            let mut button = |button: u8, mask: u32| {
                device.send_button(timestamp, joystick, button, buttons & mask != 0);
            };
            button(GamepadButton::South as u8, TRITON_LBUTTON_A);
            button(GamepadButton::East as u8, TRITON_LBUTTON_B);
            button(GamepadButton::West as u8, TRITON_LBUTTON_X);
            button(GamepadButton::North as u8, TRITON_LBUTTON_Y);

            button(GamepadButton::LeftShoulder as u8, TRITON_LBUTTON_L);
            button(GamepadButton::RightShoulder as u8, TRITON_LBUTTON_R);

            button(GamepadButton::Back as u8, TRITON_LBUTTON_MENU);
            button(GamepadButton::Start as u8, TRITON_LBUTTON_VIEW);
            button(GamepadButton::Guide as u8, TRITON_LBUTTON_STEAM);
            button(SDL_GAMEPAD_BUTTON_TRITON_QAM, TRITON_HBUTTON_QAM);

            button(GamepadButton::LeftStick as u8, TRITON_LBUTTON_L3);
            button(GamepadButton::RightStick as u8, TRITON_LBUTTON_R3);

            button(SDL_GAMEPAD_BUTTON_TRITON_RIGHT_PADDLE1, TRITON_HBUTTON_R4);
            button(SDL_GAMEPAD_BUTTON_TRITON_LEFT_PADDLE1, TRITON_HBUTTON_L4);
            button(SDL_GAMEPAD_BUTTON_TRITON_RIGHT_PADDLE2, TRITON_LBUTTON_R5);
            button(SDL_GAMEPAD_BUTTON_TRITON_LEFT_PADDLE2, TRITON_LBUTTON_L5);

            button(
                SDL_GAMEPAD_BUTTON_TRITON_RIGHT_TOUCHPAD,
                TRITON_RIGHT_TOUCHPAD_CLICK,
            );
            button(
                SDL_GAMEPAD_BUTTON_TRITON_LEFT_TOUCHPAD,
                TRITON_LEFT_TOUCHPAD_CLICK,
            );

            let mut capsense = |capsense: GamepadCapSenseType, mask: u32| {
                device.send_capsense(timestamp, joystick, capsense, buttons & mask != 0);
            };
            capsense(GamepadCapSenseType::RightStick, TRITON_RIGHT_JOYSTICK_TOUCH);
            capsense(GamepadCapSenseType::LeftStick, TRITON_LEFT_JOYSTICK_TOUCH);

            capsense(GamepadCapSenseType::RightGrip, TRITON_RIGHT_GRIP_TOUCH);
            capsense(GamepadCapSenseType::LeftGrip, TRITON_LEFT_GRIP_TOUCH);

            if buttons & TRITON_LBUTTON_DPAD_UP != 0 {
                hat |= HAT_UP;
            }
            if buttons & TRITON_LBUTTON_DPAD_DOWN != 0 {
                hat |= HAT_DOWN;
            }
            if buttons & TRITON_LBUTTON_DPAD_LEFT != 0 {
                hat |= HAT_LEFT;
            }
            if buttons & TRITON_LBUTTON_DPAD_RIGHT != 0 {
                hat |= HAT_RIGHT;
            }
            device.send_hat(timestamp, joystick, 0, hat);

            self.last_button_state = u64::from(buttons);
        }

        let trigger_axis = |trigger: i16| (i32::from(trigger) * 2 - 32768) as i16;
        let negated = |value: i16| (-i32::from(value)) as i16;
        device.send_axis(
            timestamp,
            joystick,
            GamepadAxis::LeftTrigger as u8,
            trigger_axis(report.trigger_left),
        );
        device.send_axis(
            timestamp,
            joystick,
            GamepadAxis::RightTrigger as u8,
            trigger_axis(report.trigger_right),
        );
        device.send_axis(
            timestamp,
            joystick,
            GamepadAxis::LeftX as u8,
            report.left_stick_x,
        );
        device.send_axis(
            timestamp,
            joystick,
            GamepadAxis::LeftY as u8,
            negated(report.left_stick_y),
        );
        device.send_axis(
            timestamp,
            joystick,
            GamepadAxis::RightX as u8,
            report.right_stick_x,
        );
        device.send_axis(
            timestamp,
            joystick,
            GamepadAxis::RightY as u8,
            negated(report.right_stick_y),
        );
    }

    /// The touchpads and sensors of `HIDAPI_DriverSteamTriton_HandleState()`
    /// and `HIDAPI_DriverSteamTriton_HandleState_Timestamp()`.
    fn handle_touchpads_and_imu(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: JoystickID,
        timestamp: u64,
        touchpads: TritonTouchpads,
        imu: Option<TritonImu>,
    ) {
        let left_touch_down = touchpads.buttons & TRITON_LEFT_TOUCHPAD_TOUCH != 0;
        let right_touch_down = touchpads.buttons & TRITON_RIGHT_TOUCHPAD_TOUCH != 0;

        if left_touch_down || self.left_touch_down {
            if left_touch_down {
                self.left_touch_x = f32::from(touchpads.left_pad_x) / 65536.0 + 0.5;
                self.left_touch_y = -f32::from(touchpads.left_pad_y) / 65536.0 + 0.5;
            }
            device.send_touchpad(
                timestamp,
                joystick,
                0,
                0,
                left_touch_down,
                self.left_touch_x,
                self.left_touch_y,
                f32::from(touchpads.pressure_left) / 32768.0,
            );
            self.left_touch_down = left_touch_down;
        }
        if right_touch_down || self.right_touch_down {
            if right_touch_down {
                self.right_touch_x = f32::from(touchpads.right_pad_x) / 65536.0 + 0.5;
                self.right_touch_y = -f32::from(touchpads.right_pad_y) / 65536.0 + 0.5;
            }
            device.send_touchpad(
                timestamp,
                joystick,
                1,
                0,
                right_touch_down,
                self.right_touch_x,
                self.right_touch_y,
                f32::from(touchpads.pressure_right) / 32768.0,
            );
            self.right_touch_down = right_touch_down;
        }

        if let Some(imu) = imu {
            self.sensor_timestamp_ns += u64::from(imu.delta_us) * super::NS_PER_US;

            let (gyro, accel) = valve_sensor_values(imu.gyro, imu.accel);
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
        }
    }

    /// Translation of `HIDAPI_DriverSteamTriton_HandleState()`.
    fn handle_state(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: JoystickID,
        report: &TritonMtuNoQuat,
    ) {
        let timestamp = crate::timer::ticks_ns();

        self.handle_generic_state(device, joystick, timestamp, report);

        let imu =
            (self.report_sensors && report.imu.timestamp != self.last_sensor_tick32).then(|| {
                let delta_us = report.imu.timestamp.wrapping_sub(self.last_sensor_tick32);
                self.last_sensor_tick32 = report.imu.timestamp;
                TritonImu {
                    delta_us,
                    gyro: (report.imu.gyro_x, report.imu.gyro_y, report.imu.gyro_z),
                    accel: (report.imu.accel_x, report.imu.accel_y, report.imu.accel_z),
                }
            });
        self.handle_touchpads_and_imu(
            device,
            joystick,
            timestamp,
            TritonTouchpads {
                buttons: report.buttons,
                left_pad_x: report.left_pad_x,
                left_pad_y: report.left_pad_y,
                pressure_left: report.pressure_left,
                right_pad_x: report.right_pad_x,
                right_pad_y: report.right_pad_y,
                pressure_right: report.pressure_right,
            },
            imu,
        );
    }

    /// Translation of `HIDAPI_DriverSteamTriton_HandleState_Timestamp()`;
    /// `data` is the report past its ID.
    fn handle_state_timestamp(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: JoystickID,
        data: &[u8],
    ) {
        let report = TritonMtuNoQuat32Ts::parse(data);
        let timestamp = crate::timer::ticks_ns();

        // (the report is read as a TritonMTUNoQuat_t, as upstream)
        self.handle_generic_state(device, joystick, timestamp, &TritonMtuNoQuat::parse(data));

        let imu =
            (self.report_sensors && report.imu.timestamp != self.last_sensor_tick16).then(|| {
                // The timestamp is in units of 32 microseconds
                // FIXME (upstream): the 16-bit timestamps are subtracted
                // as ints, so when they wrap around the difference is
                // negative and the delta becomes huge.
                let delta_us = ((i32::from(report.imu.timestamp)
                    - i32::from(self.last_sensor_tick16)) as u32)
                    .wrapping_mul(32);
                self.last_sensor_tick16 = report.imu.timestamp;
                TritonImu {
                    delta_us,
                    gyro: (report.imu.gyro_x, report.imu.gyro_y, report.imu.gyro_z),
                    accel: (report.imu.accel_x, report.imu.accel_y, report.imu.accel_z),
                }
            });
        self.handle_touchpads_and_imu(
            device,
            joystick,
            timestamp,
            TritonTouchpads {
                buttons: report.buttons,
                left_pad_x: report.left_pad_x,
                left_pad_y: report.left_pad_y,
                pressure_left: report.pressure_left,
                right_pad_x: report.right_pad_x,
                right_pad_y: report.right_pad_y,
                pressure_right: report.pressure_right,
            },
            imu,
        );
    }

    /// Translation of `HIDAPI_DriverSteamTriton_HandleBatteryStatus()`.
    fn handle_battery_status(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: JoystickID,
        battery_status: &TritonBatteryStatus,
    ) {
        let state = match battery_status.charge_state {
            K_ECHARGE_STATE_DISCHARGING => PowerState::OnBattery,
            K_ECHARGE_STATE_CHARGING => PowerState::Charging,
            K_ECHARGE_STATE_CHARGING_DONE => PowerState::Charged,
            // Error state?
            _ => PowerState::Unknown,
        };
        device.send_power_info(joystick, state, i32::from(battery_status.battery_level));
    }

    /// Translation of `HIDAPI_DriverSteamTriton_SetControllerConnected()`.
    fn set_controller_connected(&mut self, device: &mut DeviceCtx<'_>, connected: bool) {
        if self.connected != connected {
            self.connected = connected;

            if connected {
                device.joystick_connected();
            } else if let Some(&first) = device.joysticks().first() {
                device.joystick_disconnected(first);
            }
        }
    }

    /// Translation of `HIDAPI_DriverSteamTriton_HandleWirelessStatus()`.
    fn handle_wireless_status(&mut self, device: &mut DeviceCtx<'_>, state: u8) {
        match state {
            K_ETRITON_WIRELESS_STATE_CONNECT => self.set_controller_connected(device, true),
            K_ETRITON_WIRELESS_STATE_DISCONNECT => self.set_controller_connected(device, false),
            _ => {}
        }
    }

    /// `HIDAPI_DriverSteamTriton_RumbleJoystick()` on `dev`, at `now`
    /// (`SDL_GetTicks()`).
    fn rumble(
        &mut self,
        dev: &dyn SteamHid,
        low_frequency_rumble: u16,
        high_frequency_rumble: u16,
        now: u64,
    ) -> Result<()> {
        self.low_frequency_rumble = low_frequency_rumble;
        self.high_frequency_rumble = high_frequency_rumble;
        self.last_rumble_time = now;

        let buffer = MsgHapticRumble {
            haptic_type: 0,
            intensity: 0,
            left: HapticRumbleMotor {
                speed: low_frequency_rumble,
                gain: 0,
            },
            right: HapticRumbleMotor {
                speed: high_frequency_rumble,
                gain: 0,
            },
        }
        .output_report();

        if let Err(e) = dev.write(&buffer) {
            crate::log::error!(
                crate::log::Category::Input,
                "Steam Controller HID Write FAILED! rc: {}. SDL_Error: {}",
                -1,
                e.message()
            );

            return Err(e);
        }
        Ok(())
    }

    /// `HIDAPI_DriverSteamTriton_SetSensorsEnabled()` on `dev`.
    fn set_sensors_enabled(&mut self, dev: &dyn SteamHid, enabled: bool) -> Result<()> {
        let imu_mode = if enabled {
            SETTING_GYRO_MODE_SEND_RAW_ACCEL | SETTING_GYRO_MODE_SEND_RAW_GYRO
        } else {
            SETTING_GYRO_MODE_OFF
        };
        let buffer = settings_report(&[(SETTING_IMU_MODE, imu_mode)]);

        if dev.send_feature_report(&buffer).ok() != Some(buffer.len()) {
            // (upstream sets no error)
            return Err(Error::new("Couldn't send sensor feature report"));
        }

        self.report_sensors = enabled;

        Ok(())
    }

    /// `HIDAPI_DriverSteamTriton_UpdateDevice()` on `dev` at `now`
    /// (`SDL_GetTicks()`), with the first joystick of the device if it's
    /// open.
    fn update(
        &mut self,
        device: &mut DeviceCtx<'_>,
        dev: &dyn SteamHid,
        mut joystick: Option<JoystickID>,
        now: u64,
    ) -> bool {
        if self.connected && joystick.is_some() {
            if self.last_lizard_update == 0 || now.wrapping_sub(self.last_lizard_update) >= 3000 {
                disable_steam_triton_lizard_mode(dev);
                self.last_lizard_update = now;
            }

            if (self.low_frequency_rumble != 0 || self.high_frequency_rumble != 0)
                && now.wrapping_sub(self.last_rumble_time) >= TRITON_RUMBLE_RESEND_INTERVAL_MS
            {
                let _ = self.rumble(
                    dev,
                    self.low_frequency_rumble,
                    self.high_frequency_rumble,
                    now,
                );
            }
        }

        loop {
            let mut data = [0u8; 64];
            let r = match dev.read(&mut data) {
                Ok(0) => return true,
                Ok(r) => r,
                Err(_) => {
                    // Failed to read from controller
                    self.set_controller_connected(device, false);
                    return false;
                }
            };

            match data[0] {
                ID_TRITON_CONTROLLER_STATE | ID_TRITON_CONTROLLER_STATE_BLE => {
                    if joystick.is_none() {
                        self.set_controller_connected(device, true);
                        joystick = device.open_joystick_id();
                    }
                    if let Some(joystick) = joystick.filter(|_| r > TRITON_MTU_NO_QUAT_SIZE) {
                        let report = TritonMtuNoQuat::parse(&data[1..]);
                        self.handle_state(device, joystick, &report);
                    }
                }
                ID_TRITON_CONTROLLER_STATE_TIMESTAMP => {
                    if joystick.is_none() {
                        self.set_controller_connected(device, true);
                        joystick = device.open_joystick_id();
                    }
                    if let Some(joystick) = joystick.filter(|_| r > TRITON_MTU_NO_QUAT_32TS_SIZE) {
                        self.handle_state_timestamp(device, joystick, &data[1..]);
                    }
                }
                ID_TRITON_BATTERY_STATUS => {
                    if let Some(joystick) = joystick.filter(|_| r > TRITON_BATTERY_STATUS_SIZE) {
                        let battery_status = TritonBatteryStatus::parse(&data[1..]);
                        self.handle_battery_status(device, joystick, &battery_status);
                    }
                }
                ID_TRITON_WIRELESS_STATUS_X | ID_TRITON_WIRELESS_STATUS
                    if r > TRITON_WIRELESS_STATUS_SIZE =>
                {
                    self.handle_wireless_status(device, data[1]);
                }
                _ => {}
            }
        }
    }
}

/// `HIDAPI_DriverSteamTriton_SendJoystickEffect()` on `dev`.
fn send_effect(dev: &dyn SteamHid, data: &[u8]) -> Result<()> {
    if data.len() == HID_FEATURE_REPORT_BYTES {
        if dev.send_feature_report(data).ok() != Some(data.len()) {
            // (upstream sets no error)
            return Err(Error::new("Couldn't send effect feature report"));
        }
        return Ok(());
    }
    Err(Error::unsupported())
}

/// The Steam Triton driver's static functions.
pub(crate) struct SteamTritonDriver;

impl DriverImpl for SteamTritonDriver {
    /// Translation of `HIDAPI_DriverSteamTriton_RegisterHints()`.
    fn hints(&self) -> &'static [&'static str] {
        &[hints::JOYSTICK_HIDAPI_STEAM]
    }

    /// Translation of `HIDAPI_DriverSteamTriton_IsEnabled()`.
    fn is_enabled(&self) -> bool {
        hints::get_bool(
            hints::JOYSTICK_HIDAPI_STEAM,
            hints::get_bool(hints::JOYSTICK_HIDAPI, SDL_HIDAPI_DEFAULT),
        )
    }

    /// Translation of `HIDAPI_DriverSteamTriton_IsSupportedDevice()`.
    fn is_supported_device(
        &self,
        _device: Option<&HidapiDevice>,
        _name: &str,
        _gamepad_type: GamepadType,
        vendor_id: u16,
        product_id: u16,
        _version: u16,
        interface_number: i32,
        _interface_class: i32,
        _interface_subclass: i32,
        _interface_protocol: i32,
    ) -> bool {
        if is_proteus_dongle(product_id) {
            if (2..=5).contains(&interface_number) {
                // The set of controller interfaces for Proteus & Nereid...currently
                return true;
            }
        } else if is_joystick_steam_triton(vendor_id, product_id) {
            return true;
        }
        false
    }

    fn new_context(&self) -> Box<dyn DriverContext> {
        Box::new(SteamTritonContext::default())
    }
}

impl DriverContext for SteamTritonContext {
    /// Translation of `HIDAPI_DriverSteamTriton_InitDevice()`.
    fn init_device(&mut self, device: &mut DeviceCtx<'_>) -> Result<()> {
        device.set_device_name("Steam Controller");

        if is_proteus_dongle(device.product_id()) {
            return Ok(());
        }

        // Wired controller, connected!
        self.set_controller_connected(device, true);
        Ok(())
    }

    /// Translation of `HIDAPI_DriverSteamTriton_UpdateDevice()`.
    fn update_device(&mut self, device: &mut DeviceCtx<'_>) -> bool {
        let dev = device.device().clone();
        let joystick = device.open_joystick_id();
        self.update(device, &*dev, joystick, crate::timer::ticks_ms())
    }

    /// Translation of `HIDAPI_DriverSteamTriton_OpenJoystick()`.
    fn open_joystick(
        &mut self,
        _device: &mut DeviceCtx<'_>,
        joystick: &mut JoystickData,
    ) -> Result<()> {
        let update_rate_in_hz = 1000000.0 / TRITON_SENSOR_UPDATE_INTERVAL_US as f32;

        crate::joystick::assert_joysticks_locked();

        // Initialize the joystick capabilities
        joystick.nbuttons = SDL_GAMEPAD_NUM_TRITON_BUTTONS;
        joystick.naxes = GamepadAxis::COUNT;
        joystick.nhats = 1;

        joystick.add_sensor(SensorType::Gyro, update_rate_in_hz);
        joystick.add_sensor(SensorType::Accel, update_rate_in_hz);

        joystick.add_touchpad(1);
        joystick.add_touchpad(1);

        joystick.add_capsense(GamepadCapSenseType::LeftStick);
        joystick.add_capsense(GamepadCapSenseType::RightStick);
        joystick.add_capsense(GamepadCapSenseType::LeftGrip);
        joystick.add_capsense(GamepadCapSenseType::RightGrip);

        Ok(())
    }

    /// Translation of `HIDAPI_DriverSteamTriton_RumbleJoystick()`.
    fn rumble_joystick(
        &mut self,
        device: &mut DeviceCtx<'_>,
        _joystick: JoystickID,
        low_frequency_rumble: u16,
        high_frequency_rumble: u16,
    ) -> Result<()> {
        self.rumble(
            &**device.device(),
            low_frequency_rumble,
            high_frequency_rumble,
            crate::timer::ticks_ms(),
        )
    }

    /// Translation of `HIDAPI_DriverSteamTriton_GetJoystickCapabilities()`.
    fn get_joystick_capabilities(
        &mut self,
        _device: &mut DeviceCtx<'_>,
        _joystick: JoystickID,
    ) -> JoystickCaps {
        JoystickCaps::RUMBLE
    }

    /// Translation of `HIDAPI_DriverSteamTriton_SendJoystickEffect()`.
    fn send_joystick_effect(
        &mut self,
        device: &mut DeviceCtx<'_>,
        _joystick: JoystickID,
        data: &[u8],
    ) -> Result<()> {
        send_effect(&**device.device(), data)
    }

    /// Translation of `HIDAPI_DriverSteamTriton_SetSensorsEnabled()`.
    fn set_joystick_sensors_enabled(
        &mut self,
        device: &mut DeviceCtx<'_>,
        _joystick: JoystickID,
        enabled: bool,
    ) -> Result<()> {
        self.set_sensors_enabled(&**device.device(), enabled)
    }

    /// Translation of `HIDAPI_DriverSteamTriton_CloseJoystick()`.
    fn close_joystick(&mut self, _device: &mut DeviceCtx<'_>, _joystick: JoystickID) {
        // Lizard mode id automatically re-enabled by watchdog. Nothing to do here.
    }
}

#[cfg(test)]
mod tests;
