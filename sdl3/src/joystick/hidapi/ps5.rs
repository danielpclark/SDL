// Rust translation of src/joystick/hidapi/SDL_hidapi_ps5.c from Simple
// DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The PS5 (DualSense) controller driver.
//!
//! The helpers this file shares with the PS4 driver (feature reports,
//! CRCs, serial numbers and hats) are upstream's duplicates, used from
//! there.

use super::ps4::{dashed_serial, hat_of, read_feature_report, set_output_crc, verify_crc};
use super::rumble::lock_rumble;
use super::{
    load16, load32, supports_playstation_detection, DeviceCtx, DriverContext, DriverImpl,
    HidapiDevice, HintWatch, JoystickCaps, JoystickRef, NS_PER_US, SDL_HIDAPI_DEFAULT,
    USB_PACKET_LENGTH,
};
use crate::error::{Error, Result};
use crate::events::JoystickID;
use crate::hints;
use crate::joystick::gamepad::{GamepadAxis, GamepadButton, GamepadType};
use crate::joystick::usb_ids::*;
use crate::joystick::{
    is_joystick_dual_sense_edge, joystick_player_index_for_id, JoystickConnectionState,
    JoystickData, JoystickType,
};
use crate::power::PowerState;
use crate::sensor::{SensorType, STANDARD_GRAVITY};

const GYRO_RES_PER_DEGREE: f32 = 1024.0;
const ACCEL_RES_PER_G: f32 = 8192.0;
const BLUETOOTH_DISCONNECT_TIMEOUT_MS: u64 = 500;

// The PS5 buttons past the standard ones
const SDL_GAMEPAD_BUTTON_PS5_TOUCHPAD: u8 = 11;
const SDL_GAMEPAD_BUTTON_PS5_MICROPHONE: u8 = 12;
const SDL_GAMEPAD_BUTTON_PS5_LEFT_FUNCTION: u8 = 13;
const SDL_GAMEPAD_BUTTON_PS5_RIGHT_FUNCTION: u8 = 14;
const SDL_GAMEPAD_BUTTON_PS5_LEFT_PADDLE: u8 = 15;
const SDL_GAMEPAD_BUTTON_PS5_RIGHT_PADDLE: u8 = 16;

// EPS5ReportId
const REPORT_ID_STATE: u8 = 0x01;
const REPORT_ID_USB_EFFECTS: u8 = 0x02;
const REPORT_ID_BLUETOOTH_EFFECTS: u8 = 0x31;
const REPORT_ID_BLUETOOTH_STATE: u8 = 0x31;

// EPS5FeatureReportId
const FEATURE_REPORT_ID_CAPABILITIES: u8 = 0x03;
const FEATURE_REPORT_ID_CALIBRATION: u8 = 0x05;
const FEATURE_REPORT_ID_SERIAL_NUMBER: u8 = 0x09;
const FEATURE_REPORT_ID_FIRMWARE_INFO: u8 = 0x20;

// The state packets, which upstream overlays as the structs
// `PS5SimpleStatePacket_t`, `PS5StatePacketCommon_t`, `PS5StatePacket_t`
// and `PS5StatePacketAlt_t`: their byte offsets.

/// `PS5SimpleStatePacket_t`
mod simple {
    pub(super) const LEFT_JOYSTICK_X: usize = 0;
    pub(super) const LEFT_JOYSTICK_Y: usize = 1;
    pub(super) const RIGHT_JOYSTICK_X: usize = 2;
    pub(super) const RIGHT_JOYSTICK_Y: usize = 3;
    pub(super) const BUTTONS_HAT_AND_COUNTER: usize = 4;
    pub(super) const TRIGGER_LEFT: usize = 7;
    pub(super) const TRIGGER_RIGHT: usize = 8;
    /// `sizeof(PS5SimpleStatePacket_t)`
    pub(super) const SIZE: usize = 9;
}

/// `PS5StatePacketCommon_t`, the start of the others
mod common {
    pub(super) const LEFT_JOYSTICK_X: usize = 0;
    pub(super) const LEFT_JOYSTICK_Y: usize = 1;
    pub(super) const RIGHT_JOYSTICK_X: usize = 2;
    pub(super) const RIGHT_JOYSTICK_Y: usize = 3;
    pub(super) const TRIGGER_LEFT: usize = 4;
    pub(super) const TRIGGER_RIGHT: usize = 5;
    pub(super) const BUTTONS_AND_HAT: usize = 7;
    /// 32 bit little endian
    pub(super) const PACKET_SEQUENCE: usize = 11;
    pub(super) const GYRO_X: usize = 15;
    pub(super) const ACCEL_X: usize = 21;
    /// 16/32 bit little endian
    pub(super) const SENSOR_TIMESTAMP: usize = 27;
}

/// `PS5StatePacket_t`
mod full {
    /// High bit clear + counter
    pub(super) const TOUCHPAD_COUNTER1: usize = 32;
    /// X/Y, 12 bits per axis
    pub(super) const TOUCHPAD_DATA1: usize = 33;
    pub(super) const TOUCHPAD_COUNTER2: usize = 36;
    pub(super) const TOUCHPAD_DATA2: usize = 37;
    pub(super) const BATTERY_LEVEL: usize = 52;
}

/// `PS5StatePacketAlt_t`
mod alt {
    pub(super) const BATTERY_LEVEL: usize = 29;
    pub(super) const TOUCHPAD_COUNTER1: usize = 31;
    pub(super) const TOUCHPAD_DATA1: usize = 32;
    pub(super) const TOUCHPAD_COUNTER2: usize = 35;
    pub(super) const TOUCHPAD_DATA2: usize = 36;
    /// (39, though upstream's comment says 40)
    pub(super) const DEVICE_SPECIFIC: usize = 39;
    /// `sizeof(PS5StatePacketAlt_t)`
    pub(super) const SIZE: usize = 48;
}

/// The size of the `last_state` union.
const LAST_STATE_SIZE: usize = 64;

/// `sizeof(DS5EffectsState_t)`
const DS5_EFFECTS_STATE_SIZE: usize = 47;

/// The parts of `DS5EffectsState_t` this driver sets (the volumes,
/// trigger effects and LED animation stay 0).
#[derive(Clone, Copy, Default)]
struct Ds5EffectsState {
    enable_bits1: u8,
    enable_bits2: u8,
    rumble_right: u8,
    rumble_left: u8,
    mic_light_mode: u8,
    enable_bits3: u8,
    pad_lights: u8,
    led_red: u8,
    led_green: u8,
    led_blue: u8,
}

impl Ds5EffectsState {
    fn to_bytes(self) -> [u8; DS5_EFFECTS_STATE_SIZE] {
        let mut data = [0; DS5_EFFECTS_STATE_SIZE];
        data[0] = self.enable_bits1;
        data[1] = self.enable_bits2;
        data[2] = self.rumble_right;
        data[3] = self.rumble_left;
        data[8] = self.mic_light_mode;
        data[38] = self.enable_bits3;
        data[43] = self.pad_lights;
        data[44] = self.led_red;
        data[45] = self.led_green;
        data[46] = self.led_blue;
        data
    }
}

// EDS5Effect
const EFFECT_RUMBLE_START: u8 = 1 << 0;
const EFFECT_RUMBLE: u8 = 1 << 1;
const EFFECT_LED_RESET: u8 = 1 << 2;
const EFFECT_LED: u8 = 1 << 3;
const EFFECT_PAD_LIGHTS: u8 = 1 << 4;
const EFFECT_MIC_LIGHT: u8 = 1 << 5;

/// Translation of `EDS5LEDResetState`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum LedResetState {
    #[default]
    None,
    Pending,
    Complete,
}

/// Translation of `IMUCalibrationData`.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
struct ImuCalibration {
    bias: i16,
    sensitivity: f32,
}

/// The rumble hint mode (`HIDAPI_PS5_EnhancedReportHint`).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum EnhancedReportHint {
    /// Enhanced features are never used
    #[default]
    Off,
    /// Enhanced features are always used
    On,
    /// Enhanced features are advertised to the application, but SDL
    /// doesn't touch the controller state unless the application
    /// explicitly requests it
    Auto,
}

/// Translation of `SDL_DriverPS5_Context`.
#[derive(Debug)]
struct Ps5Context {
    /// The open joystick (`ctx->joystick`)
    joystick: Option<JoystickID>,
    is_dongle: bool,
    use_alternate_report: bool,
    sensors_supported: bool,
    lightbar_supported: bool,
    vibration_supported: bool,
    playerled_supported: bool,
    touchpad_supported: bool,
    effects_supported: bool,
    guitar_whammy_supported: bool,
    guitar_tilt_supported: bool,
    guitar_effects_selector_supported: bool,
    enhanced_report_hint: EnhancedReportHint,
    enhanced_reports: bool,
    enhanced_mode: bool,
    enhanced_mode_available: bool,
    report_sensors: bool,
    report_touchpad: bool,
    report_battery: bool,
    hardware_calibration: bool,
    calibration: [ImuCalibration; 6],
    firmware_version: u16,
    last_packet: u64,
    player_index: i32,
    player_lights: bool,
    enhanced_rumble: bool,
    rumble_left: u8,
    rumble_right: u8,
    color_set: bool,
    led_red: u8,
    led_green: u8,
    led_blue: u8,
    led_reset_state: LedResetState,
    sensor_ticks: u64,
    last_tick: u32,
    /// The bytes of upstream's `last_state` union
    last_state: [u8; LAST_STATE_SIZE],
    /// The `SDL_PS5EnhancedReportsChanged()` callback
    enhanced_reports_hint: Option<HintWatch>,
    /// The `SDL_PS5PlayerLEDHintChanged()` callback
    player_led_hint: Option<HintWatch>,
}

impl Default for Ps5Context {
    fn default() -> Self {
        Ps5Context {
            joystick: None,
            is_dongle: false,
            use_alternate_report: false,
            sensors_supported: false,
            lightbar_supported: false,
            vibration_supported: false,
            playerled_supported: false,
            touchpad_supported: false,
            effects_supported: false,
            guitar_whammy_supported: false,
            guitar_tilt_supported: false,
            guitar_effects_selector_supported: false,
            enhanced_report_hint: EnhancedReportHint::Off,
            enhanced_reports: false,
            enhanced_mode: false,
            enhanced_mode_available: false,
            report_sensors: false,
            report_touchpad: false,
            report_battery: false,
            hardware_calibration: false,
            calibration: [ImuCalibration::default(); 6],
            firmware_version: 0,
            last_packet: 0,
            player_index: 0,
            player_lights: false,
            enhanced_rumble: false,
            rumble_left: 0,
            rumble_right: 0,
            color_set: false,
            led_red: 0,
            led_green: 0,
            led_blue: 0,
            led_reset_state: LedResetState::None,
            sensor_ticks: 0,
            last_tick: 0,
            last_state: [0; LAST_STATE_SIZE],
            enhanced_reports_hint: None,
            player_led_hint: None,
        }
    }
}

/// Translation of `SetLedsForPlayerIndex()`.
fn set_leds_for_player_index(effects: &mut Ds5EffectsState, player_index: i32) {
    // This list is the same as what hid-sony.c uses in the Linux kernel.
    // The first 4 values correspond to what the PS4 assigns.
    const COLORS: [[u8; 3]; 7] = [
        [0x00, 0x00, 0x40], // Blue
        [0x40, 0x00, 0x00], // Red
        [0x00, 0x40, 0x00], // Green
        [0x20, 0x00, 0x20], // Pink
        [0x20, 0x10, 0x00], // Orange
        [0x00, 0x10, 0x10], // Teal
        [0x10, 0x10, 0x10], // White
    ];

    let index = if player_index >= 0 {
        player_index as usize % COLORS.len()
    } else {
        0
    };

    effects.led_red = COLORS[index][0];
    effects.led_green = COLORS[index][1];
    effects.led_blue = COLORS[index][2];
}

/// Translation of `SetLightsForPlayerIndex()`.
fn set_lights_for_player_index(effects: &mut Ds5EffectsState, player_index: i32) {
    const LIGHTS: [u8; 7] = [0x04, 0x0A, 0x15, 0x1B, 0x1F, 0x11, 0x0E];

    if player_index >= 0 {
        // Bitmask, 0x1F enables all lights, 0x20 changes instantly instead of fade
        effects.pad_lights = LIGHTS[player_index as usize % LIGHTS.len()] | 0x20;
    } else {
        effects.pad_lights = 0x00;
    }
}

/// The battery state of a battery level byte (part of the state packet
/// handlers).
fn battery_state(battery_level: u8) -> (PowerState, i32) {
    let status = (battery_level >> 4) & 0x0F;
    let level = i32::from(battery_level & 0x0F);

    match status {
        0 => (PowerState::OnBattery, (level * 10 + 5).min(100)),
        1 => (PowerState::Charging, (level * 10 + 5).min(100)),
        2 => (PowerState::Charged, 100),
        _ => (PowerState::Unknown, 0),
    }
}

/// The calibration of a calibration feature report (part of
/// `HIDAPI_DriverPS5_LoadCalibrationData()`); whether it's valid.
fn parse_calibration(calibration: &mut [ImuCalibration; 6], data: &[u8]) -> bool {
    let at = |i: usize| i32::from(load16(data[i], data[i + 1]));

    let gyro_pitch_bias = at(1);
    let gyro_yaw_bias = at(3);
    let gyro_roll_bias = at(5);

    let gyro_pitch_plus = at(7);
    let gyro_pitch_minus = at(9);
    let gyro_yaw_plus = at(11);
    let gyro_yaw_minus = at(13);
    let gyro_roll_plus = at(15);
    let gyro_roll_minus = at(17);

    let gyro_speed_plus = at(19);
    let gyro_speed_minus = at(21);

    let numerator = (gyro_speed_plus + gyro_speed_minus) as f32 * GYRO_RES_PER_DEGREE;
    let gyros = [
        (gyro_pitch_bias, gyro_pitch_plus, gyro_pitch_minus),
        (gyro_yaw_bias, gyro_yaw_plus, gyro_yaw_minus),
        (gyro_roll_bias, gyro_roll_plus, gyro_roll_minus),
    ];
    for (c, (bias, plus, minus)) in calibration.iter_mut().zip(gyros) {
        c.bias = bias as i16;
        c.sensitivity = numerator / (plus - minus) as f32;
    }

    for (i, c) in calibration[3..].iter_mut().enumerate() {
        let plus = at(23 + i * 4);
        let minus = at(25 + i * 4);
        let range_2g = (plus - minus) as i16;
        c.bias = (plus - i32::from(range_2g) / 2) as i16;
        c.sensitivity = 2.0 * ACCEL_RES_PER_G / f32::from(range_2g);
    }

    // Some controllers have a bad calibration
    calibration.iter().enumerate().all(|(i, c)| {
        let divisor = if i < 3 { 64.0 } else { 1.0 };
        i32::from(c.bias).abs() <= 1024 && (1.0 - c.sensitivity / divisor).abs() <= 0.5
    })
}

/// The touchpad events of a packet's touchpad bytes.
fn send_touchpad_fingers(
    device: &mut DeviceCtx<'_>,
    timestamp: u64,
    joystick: JoystickID,
    fingers: [(u8, &[u8]); 2],
) {
    // (upstream's literals)
    #[allow(clippy::excessive_precision)]
    const TOUCHPAD_SCALEX: f32 = 5.20833333e-4; // 1.0f / 1920
    #[allow(clippy::excessive_precision)]
    const TOUCHPAD_SCALEY: f32 = 9.34579439e-4; // 1.0f / 1070

    for (finger, (counter, data)) in (0..).zip(fingers) {
        let touchpad_down = (counter & 0x80) == 0;
        let touchpad_x = i32::from(data[0]) | ((i32::from(data[1]) & 0x0F) << 8);
        let touchpad_y = i32::from(data[1] >> 4) | (i32::from(data[2]) << 4);
        device.send_touchpad(
            timestamp,
            joystick,
            0,
            finger,
            touchpad_down,
            touchpad_x as f32 * TOUCHPAD_SCALEX,
            touchpad_y as f32 * TOUCHPAD_SCALEY,
            if touchpad_down { 1.0 } else { 0.0 },
        );
    }
}

/// The axis value of a stick or trigger byte.
fn axis_of(value: u8) -> i16 {
    (i32::from(value) * 257 - 32768) as i16
}

/// A trigger, at its maximum if only its digital bit is set.
fn trigger_axis(value: u8, digital: bool) -> i16 {
    if value == 0 && digital {
        i16::MAX
    } else {
        axis_of(value)
    }
}

impl Ps5Context {
    /// Translation of `HIDAPI_DriverPS5_LoadCalibrationData()`.
    fn load_calibration_data(&mut self, device: &DeviceCtx<'_>) {
        let mut data = [0u8; USB_PACKET_LENGTH];

        match read_feature_report(device, FEATURE_REPORT_ID_CALIBRATION, &mut data) {
            Ok(size) if size >= 35 => self.apply_calibration_report(&data),
            // Short read of calibration data, ignoring calibration
            _ => {}
        }
    }

    /// The calibration of a calibration feature report.
    fn apply_calibration_report(&mut self, data: &[u8]) {
        self.hardware_calibration = parse_calibration(&mut self.calibration, data);
    }

    /// Translation of `HIDAPI_DriverPS5_ApplyCalibrationData()`.
    fn apply_calibration_data(&self, index: usize, value: i16) -> f32 {
        let mut result = if self.hardware_calibration {
            let calibration = &self.calibration[index];

            (i32::from(value) - i32::from(calibration.bias)) as f32 * calibration.sensitivity
        } else if index < 3 {
            f32::from(value) * 64.0
        } else {
            f32::from(value)
        };

        // Convert the raw data to the units expected by SDL
        if index < 3 {
            result = (result / GYRO_RES_PER_DEGREE) * std::f32::consts::PI / 180.0;
        } else {
            result = (result / ACCEL_RES_PER_G) * STANDARD_GRAVITY;
        }
        result
    }

    /// The effects of `HIDAPI_DriverPS5_UpdateEffects()`, `None` if they
    /// wait for the Bluetooth connection sequence.
    fn effects(&mut self, bluetooth: bool, effect_mask: u8) -> Option<Ds5EffectsState> {
        // Make sure the Bluetooth connection sequence has completed before sending LED color change
        if bluetooth
            && self.enhanced_reports
            && (effect_mask & (EFFECT_LED | EFFECT_PAD_LIGHTS)) != 0
            && self.led_reset_state != LedResetState::Complete
        {
            self.led_reset_state = LedResetState::Pending;
            return None;
        }

        let mut effects = Ds5EffectsState::default();

        if self.vibration_supported {
            if self.rumble_left != 0 || self.rumble_right != 0 {
                if self.enhanced_rumble {
                    effects.enable_bits3 |= 0x04; // Enable improved rumble emulation on 2.24 firmware and newer

                    effects.rumble_left = self.rumble_left;
                    effects.rumble_right = self.rumble_right;
                } else {
                    effects.enable_bits1 |= 0x01; // Enable rumble emulation

                    // Shift to reduce effective rumble strength to match Xbox controllers
                    effects.rumble_left = self.rumble_left >> 1;
                    effects.rumble_right = self.rumble_right >> 1;
                }
                effects.enable_bits1 |= 0x02; // Disable audio haptics
            } else {
                // Leaving emulated rumble bits off will restore audio haptics
            }

            if (effect_mask & EFFECT_RUMBLE_START) != 0 {
                effects.enable_bits1 |= 0x02; // Disable audio haptics
            }
            // (EFFECT_RUMBLE is already handled above)
        }
        if self.lightbar_supported {
            if (effect_mask & EFFECT_LED_RESET) != 0 {
                effects.enable_bits2 |= 0x08; // Reset LED state
            }
            if (effect_mask & EFFECT_LED) != 0 {
                effects.enable_bits2 |= 0x04; // Enable LED color

                // Populate the LED state with the appropriate color from our lookup table
                if self.color_set {
                    effects.led_red = self.led_red;
                    effects.led_green = self.led_green;
                    effects.led_blue = self.led_blue;
                } else {
                    set_leds_for_player_index(&mut effects, self.player_index);
                }
            }
        }
        if self.playerled_supported && (effect_mask & EFFECT_PAD_LIGHTS) != 0 {
            effects.enable_bits2 |= 0x10; // Enable touchpad lights

            if self.player_lights {
                set_lights_for_player_index(&mut effects, self.player_index);
            } else {
                effects.pad_lights = 0x00;
            }
        }
        if (effect_mask & EFFECT_MIC_LIGHT) != 0 {
            effects.enable_bits2 |= 0x01; // Enable microphone light

            effects.mic_light_mode = 0; // Bitmask, 0x00 = off, 0x01 = solid, 0x02 = pulse
        }
        Some(effects)
    }

    /// Translation of `HIDAPI_DriverPS5_UpdateEffects()`.
    fn update_effects(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: &mut JoystickRef<'_>,
        effect_mask: u8,
        application_usage: bool,
    ) -> Result<()> {
        match self.effects(device.is_bluetooth(), effect_mask) {
            Some(effects) => self.internal_send_joystick_effect(
                device,
                joystick,
                &effects.to_bytes(),
                application_usage,
            ),
            None => Ok(()),
        }
    }

    /// Translation of `HIDAPI_DriverPS5_CheckPendingLEDReset()`.
    fn check_pending_led_reset(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: &mut JoystickRef<'_>,
    ) {
        let led_reset_complete =
            if self.enhanced_reports && self.sensors_supported && !self.use_alternate_report {
                // Check the timer to make sure the Bluetooth connection LED animation is complete
                const CONNECTION_COMPLETE: u32 = 10200000;
                let t = &self.last_state[common::SENSOR_TIMESTAMP..];
                load32(t[0], t[1], t[2], t[3]) >= CONNECTION_COMPLETE
            } else {
                // We don't know how to check the timer, just assume it's complete for now
                true
            };

        if led_reset_complete {
            let _ = self.update_effects(device, joystick, EFFECT_LED_RESET, false);

            self.led_reset_state = LedResetState::Complete;

            let _ = self.update_effects(device, joystick, EFFECT_LED | EFFECT_PAD_LIGHTS, false);
        }
    }

    /// Translation of `HIDAPI_DriverPS5_TickleBluetooth()`.
    fn tickle_bluetooth(&mut self, device: &mut DeviceCtx<'_>) {
        if self.enhanced_reports {
            // This is just a dummy packet that should have no effect, since we don't set the CRC
            let mut data = [0u8; 78];

            data[0] = REPORT_ID_BLUETOOTH_EFFECTS;
            data[1] = 0x02; // Magic value

            if let Ok(lock) = lock_rumble() {
                let _ = lock.send_and_unlock(device.device(), &data);
            }
        } else {
            // We can't even send an invalid effects packet, or it will put the controller in enhanced mode
            if let Some(&first) = device.joysticks().first() {
                device.joystick_disconnected(first);
            }
        }
    }

    /// Translation of `HIDAPI_DriverPS5_SetEnhancedModeAvailable()`.
    fn set_enhanced_mode_available(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: &mut JoystickRef<'_>,
    ) {
        if self.enhanced_mode_available {
            return;
        }
        self.enhanced_mode_available = true;

        // Standard DualSense sensor update rate is 250 Hz over USB
        let mut update_rate = 250.0;
        if device.is_bluetooth() {
            // Bluetooth sensor update rate appears to be 1000 Hz
            update_rate = 1000.0;
        } else if is_joystick_dual_sense_edge(device.vendor_id(), device.product_id()) {
            // DualSense Edge sensor update rate is 1000 Hz over USB
            update_rate = 1000.0;
        }

        let (touchpad, sensors, tilt) = (
            self.touchpad_supported,
            self.sensors_supported,
            self.guitar_tilt_supported,
        );
        joystick.with(|joystick| {
            if touchpad {
                joystick.add_touchpad(2);
            }
            if sensors {
                joystick.add_sensor(SensorType::Gyro, update_rate);
                joystick.add_sensor(SensorType::Accel, update_rate);
            }
            if tilt {
                joystick.add_sensor(SensorType::Accel, 250.0);
            }
        });
        if touchpad {
            self.report_touchpad = true;
        }

        self.report_battery = true;

        device.update_device_properties();
    }

    /// Translation of `HIDAPI_DriverPS5_SetEnhancedMode()`.
    fn set_enhanced_mode(&mut self, device: &mut DeviceCtx<'_>, joystick: &mut JoystickRef<'_>) {
        self.set_enhanced_mode_available(device, joystick);

        if !self.enhanced_mode {
            self.enhanced_mode = true;

            // Switch into enhanced report mode
            let _ = self.update_effects(device, joystick, 0, false);

            // Update the light effects
            let _ = self.update_effects(device, joystick, EFFECT_LED | EFFECT_PAD_LIGHTS, false);
        }
    }

    /// Translation of `HIDAPI_DriverPS5_SetEnhancedReportHint()`.
    fn set_enhanced_report_hint(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: &mut JoystickRef<'_>,
        enhanced_report_hint: EnhancedReportHint,
    ) {
        match enhanced_report_hint {
            EnhancedReportHint::Off => {
                // Nothing to do, enhanced mode is a one-way ticket
            }
            EnhancedReportHint::On => self.set_enhanced_mode(device, joystick),
            EnhancedReportHint::Auto => self.set_enhanced_mode_available(device, joystick),
        }
        self.enhanced_report_hint = enhanced_report_hint;
    }

    /// Translation of `HIDAPI_DriverPS5_UpdateEnhancedModeOnEnhancedReport()`.
    fn update_enhanced_mode_on_enhanced_report(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: &mut JoystickRef<'_>,
    ) {
        self.enhanced_reports = true;

        if self.enhanced_report_hint == EnhancedReportHint::Auto {
            self.set_enhanced_report_hint(device, joystick, EnhancedReportHint::On);
        }
    }

    /// Translation of `HIDAPI_DriverPS5_UpdateEnhancedModeOnApplicationUsage()`.
    fn update_enhanced_mode_on_application_usage(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: &mut JoystickRef<'_>,
    ) {
        if self.enhanced_report_hint == EnhancedReportHint::Auto {
            self.set_enhanced_report_hint(device, joystick, EnhancedReportHint::On);
        }
    }

    /// Translation of `SDL_PS5EnhancedReportsChanged()`.
    fn enhanced_reports_changed(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: &mut JoystickRef<'_>,
        hint: Option<&str>,
    ) {
        let mode = if device.is_bluetooth() {
            if hint.is_some_and(|h| h.eq_ignore_ascii_case("auto")) {
                EnhancedReportHint::Auto
            } else if hints::string_to_bool(hint, true) {
                EnhancedReportHint::On
            } else {
                EnhancedReportHint::Off
            }
        } else {
            EnhancedReportHint::On
        };
        self.set_enhanced_report_hint(device, joystick, mode);
    }

    /// Translation of `SDL_PS5PlayerLEDHintChanged()`.
    fn player_led_hint_changed(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: &mut JoystickRef<'_>,
        hint: Option<&str>,
    ) {
        let player_lights = hints::string_to_bool(hint, true);

        if player_lights != self.player_lights {
            self.player_lights = player_lights;

            let _ = self.update_effects(device, joystick, EFFECT_PAD_LIGHTS, false);
        }
    }

    /// Apply the hint changes recorded since the last call (upstream's
    /// hint callbacks, in the order they're added).
    fn hint_changes(&mut self, device: &mut DeviceCtx<'_>, joystick: &mut JoystickRef<'_>) {
        if let Some(hint) = self
            .enhanced_reports_hint
            .as_ref()
            .and_then(HintWatch::take)
        {
            self.enhanced_reports_changed(device, joystick, hint.as_deref());
        }
        if let Some(hint) = self.player_led_hint.as_ref().and_then(HintWatch::take) {
            self.player_led_hint_changed(device, joystick, hint.as_deref());
        }
    }

    /// The output report of `HIDAPI_DriverPS5_InternalSendJoystickEffect()`,
    /// its size and the offset of the effects in it.
    fn effect_packet(bluetooth: bool, effect: &[u8]) -> ([u8; 78], usize, usize) {
        let mut data = [0u8; 78];

        let (report_size, offset) = if bluetooth {
            data[0] = REPORT_ID_BLUETOOTH_EFFECTS;
            data[1] = 0x00; // Tag and sequence
            data[2] = 0x10; // Magic value

            (78, 3)
        } else {
            data[0] = REPORT_ID_USB_EFFECTS;

            (48, 1)
        };

        let n = effect.len().min(data.len() - offset);
        data[offset..offset + n].copy_from_slice(&effect[..n]);

        if bluetooth {
            set_output_crc(&mut data[..report_size]);
        }
        (data, report_size, offset)
    }

    /// Translation of `HIDAPI_DriverPS5_InternalSendJoystickEffect()`.
    fn internal_send_joystick_effect(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: &mut JoystickRef<'_>,
        effect: &[u8],
        application_usage: bool,
    ) -> Result<()> {
        if !self.effects_supported {
            // We shouldn't be sending packets to this controller
            return Err(Error::unsupported());
        }

        if !self.enhanced_mode {
            if application_usage {
                self.update_enhanced_mode_on_application_usage(device, joystick);

                // Wait briefly before sending additional effects
                crate::timer::delay(std::time::Duration::from_millis(10));
            }

            if !self.enhanced_mode {
                // We're not in enhanced mode, effects aren't allowed
                return Err(Error::unsupported());
            }
        }

        let (data, report_size, offset) = Ps5Context::effect_packet(device.is_bluetooth(), effect);

        let mut lock = lock_rumble()?;

        // See if we can update an existing pending request
        if let Some((pending_data, pending_size)) = lock.pending_mut(device.device()) {
            // (the enable bits of the DS5EffectsState_t in both)
            if report_size == *pending_size
                && data[offset..offset + 2] == pending_data[offset..offset + 2]
            {
                // We're simply updating the data for this request
                pending_data[..report_size].copy_from_slice(&data[..report_size]);
                return Ok(());
            }
        }

        if lock.send_and_unlock(device.device(), &data[..report_size])? != report_size {
            return Err(Error::new("Couldn't send rumble packet"));
        }
        Ok(())
    }

    /// Translation of `HIDAPI_DriverPS5_HandleSimpleStatePacket()`; the
    /// packet is the read buffer from the packet on.
    fn handle_simple_state_packet(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: JoystickID,
        packet: &[u8],
        timestamp: u64,
    ) {
        use simple::*;
        let buttons = &packet[BUTTONS_HAT_AND_COUNTER..BUTTONS_HAT_AND_COUNTER + 3];
        let last = &self.last_state[BUTTONS_HAT_AND_COUNTER..BUTTONS_HAT_AND_COUNTER + 3];

        self.send_buttons(device, joystick, timestamp, buttons, last, false);

        device.send_axis(
            timestamp,
            joystick,
            GamepadAxis::LeftTrigger as u8,
            trigger_axis(packet[TRIGGER_LEFT], buttons[1] & 0x04 != 0),
        );
        device.send_axis(
            timestamp,
            joystick,
            GamepadAxis::RightTrigger as u8,
            trigger_axis(packet[TRIGGER_RIGHT], buttons[1] & 0x08 != 0),
        );
        device.send_axis(
            timestamp,
            joystick,
            GamepadAxis::LeftX as u8,
            axis_of(packet[LEFT_JOYSTICK_X]),
        );
        device.send_axis(
            timestamp,
            joystick,
            GamepadAxis::LeftY as u8,
            axis_of(packet[LEFT_JOYSTICK_Y]),
        );
        device.send_axis(
            timestamp,
            joystick,
            GamepadAxis::RightX as u8,
            axis_of(packet[RIGHT_JOYSTICK_X]),
        );
        device.send_axis(
            timestamp,
            joystick,
            GamepadAxis::RightY as u8,
            axis_of(packet[RIGHT_JOYSTICK_Y]),
        );

        self.last_state[..SIZE].copy_from_slice(&packet[..SIZE]);
    }

    /// The buttons and hat of the first three button bytes (the simple
    /// packet's `rgucButtonsHatAndCounter`, or `rgucButtonsAndHat`), if
    /// they changed from `last`; `extended` for the PS5 buttons of the
    /// common packet.
    fn send_buttons(
        &self,
        device: &mut DeviceCtx<'_>,
        joystick: JoystickID,
        timestamp: u64,
        buttons: &[u8],
        last: &[u8],
        extended: bool,
    ) {
        if last[0] != buttons[0] {
            let data = buttons[0] >> 4;

            device.send_button(
                timestamp,
                joystick,
                GamepadButton::West as u8,
                (data & 0x01) != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                GamepadButton::South as u8,
                (data & 0x02) != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                GamepadButton::East as u8,
                (data & 0x04) != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                GamepadButton::North as u8,
                (data & 0x08) != 0,
            );

            device.send_hat(timestamp, joystick, 0, hat_of(buttons[0] & 0x0F));
        }

        if last[1] != buttons[1] {
            let data = buttons[1];

            device.send_button(
                timestamp,
                joystick,
                GamepadButton::LeftShoulder as u8,
                (data & 0x01) != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                GamepadButton::RightShoulder as u8,
                (data & 0x02) != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                GamepadButton::Back as u8,
                (data & 0x10) != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                GamepadButton::Start as u8,
                (data & 0x20) != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                GamepadButton::LeftStick as u8,
                (data & 0x40) != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                GamepadButton::RightStick as u8,
                (data & 0x80) != 0,
            );
        }

        if last[2] != buttons[2] {
            let data = buttons[2];

            device.send_button(
                timestamp,
                joystick,
                GamepadButton::Guide as u8,
                (data & 0x01) != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                SDL_GAMEPAD_BUTTON_PS5_TOUCHPAD,
                (data & 0x02) != 0,
            );
            if extended {
                device.send_button(
                    timestamp,
                    joystick,
                    SDL_GAMEPAD_BUTTON_PS5_MICROPHONE,
                    (data & 0x04) != 0,
                );
                device.send_button(
                    timestamp,
                    joystick,
                    SDL_GAMEPAD_BUTTON_PS5_LEFT_FUNCTION,
                    (data & 0x10) != 0,
                );
                device.send_button(
                    timestamp,
                    joystick,
                    SDL_GAMEPAD_BUTTON_PS5_RIGHT_FUNCTION,
                    (data & 0x20) != 0,
                );
                device.send_button(
                    timestamp,
                    joystick,
                    SDL_GAMEPAD_BUTTON_PS5_LEFT_PADDLE,
                    (data & 0x40) != 0,
                );
                device.send_button(
                    timestamp,
                    joystick,
                    SDL_GAMEPAD_BUTTON_PS5_RIGHT_PADDLE,
                    (data & 0x80) != 0,
                );
            }
        }
    }

    /// Translation of `HIDAPI_DriverPS5_HandleStatePacketCommon()`.
    fn handle_state_packet_common(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: JoystickID,
        packet: &[u8],
        timestamp: u64,
    ) {
        use common::*;
        let buttons = &packet[BUTTONS_AND_HAT..BUTTONS_AND_HAT + 3];
        let last = &self.last_state[BUTTONS_AND_HAT..BUTTONS_AND_HAT + 3];

        self.send_buttons(device, joystick, timestamp, buttons, last, true);

        device.send_axis(
            timestamp,
            joystick,
            GamepadAxis::LeftTrigger as u8,
            trigger_axis(packet[TRIGGER_LEFT], buttons[1] & 0x04 != 0),
        );
        device.send_axis(
            timestamp,
            joystick,
            GamepadAxis::RightTrigger as u8,
            trigger_axis(packet[TRIGGER_RIGHT], buttons[1] & 0x08 != 0),
        );
        device.send_axis(
            timestamp,
            joystick,
            GamepadAxis::LeftX as u8,
            axis_of(packet[LEFT_JOYSTICK_X]),
        );
        device.send_axis(
            timestamp,
            joystick,
            GamepadAxis::LeftY as u8,
            axis_of(packet[LEFT_JOYSTICK_Y]),
        );
        device.send_axis(
            timestamp,
            joystick,
            GamepadAxis::RightX as u8,
            axis_of(packet[RIGHT_JOYSTICK_X]),
        );
        device.send_axis(
            timestamp,
            joystick,
            GamepadAxis::RightY as u8,
            axis_of(packet[RIGHT_JOYSTICK_Y]),
        );

        if self.report_sensors {
            let t = &packet[SENSOR_TIMESTAMP..SENSOR_TIMESTAMP + 4];
            let sensor_timestamp = if self.use_alternate_report {
                // 16-bit timestamp
                let tick = u32::from(load16(t[0], t[1]) as u16);
                let delta = if self.last_tick <= tick {
                    tick - self.last_tick
                } else {
                    // (in upstream's 32 bits)
                    0xFFFF_u32
                        .wrapping_sub(self.last_tick)
                        .wrapping_add(tick)
                        .wrapping_add(1)
                };
                self.last_tick = tick;
                self.sensor_ticks += u64::from(delta);

                // Sensor timestamp is in 1us units
                self.sensor_ticks * NS_PER_US
            } else {
                // 32-bit timestamp
                let tick = load32(t[0], t[1], t[2], t[3]);
                let delta = if self.last_tick < tick {
                    tick - self.last_tick
                } else {
                    u32::MAX
                        .wrapping_sub(self.last_tick)
                        .wrapping_add(tick)
                        .wrapping_add(1)
                };
                self.last_tick = tick;
                self.sensor_ticks += u64::from(delta);

                // Sensor timestamp is in 0.33us units
                (self.sensor_ticks * NS_PER_US) / 3
            };

            let value = |offset: usize| load16(packet[offset], packet[offset + 1]);
            let data: [f32; 3] =
                std::array::from_fn(|i| self.apply_calibration_data(i, value(GYRO_X + i * 2)));
            device.send_sensor(
                timestamp,
                joystick,
                SensorType::Gyro,
                sensor_timestamp,
                &data,
            );

            let data: [f32; 3] =
                std::array::from_fn(|i| self.apply_calibration_data(3 + i, value(ACCEL_X + i * 2)));
            device.send_sensor(
                timestamp,
                joystick,
                SensorType::Accel,
                sensor_timestamp,
                &data,
            );
        }
    }

    /// Translation of `HIDAPI_DriverPS5_HandleStatePacket()`.
    fn handle_state_packet(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: JoystickID,
        packet: &[u8],
        timestamp: u64,
    ) {
        use full::*;

        if self.report_touchpad {
            send_touchpad_fingers(
                device,
                timestamp,
                joystick,
                [
                    (
                        packet[TOUCHPAD_COUNTER1],
                        &packet[TOUCHPAD_DATA1..TOUCHPAD_DATA1 + 3],
                    ),
                    (
                        packet[TOUCHPAD_COUNTER2],
                        &packet[TOUCHPAD_DATA2..TOUCHPAD_DATA2 + 3],
                    ),
                ],
            );
        }

        if self.report_battery {
            let (state, percent) = battery_state(packet[BATTERY_LEVEL]);
            device.send_power_info(joystick, state, percent);
        }

        self.handle_state_packet_common(device, joystick, packet, timestamp);

        self.last_state.copy_from_slice(&packet[..LAST_STATE_SIZE]);
    }

    /// Translation of `HIDAPI_DriverPS5_HandleStatePacketAlt()`.
    fn handle_state_packet_alt(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: JoystickID,
        packet: &[u8],
        timestamp: u64,
    ) {
        use alt::*;

        if self.report_touchpad {
            send_touchpad_fingers(
                device,
                timestamp,
                joystick,
                [
                    (
                        packet[TOUCHPAD_COUNTER1],
                        &packet[TOUCHPAD_DATA1..TOUCHPAD_DATA1 + 3],
                    ),
                    (
                        packet[TOUCHPAD_COUNTER2],
                        &packet[TOUCHPAD_DATA2..TOUCHPAD_DATA2 + 3],
                    ),
                ],
            );
        }

        if self.report_battery {
            // 0x0C means a controller isn't reporting battery levels
            if packet[BATTERY_LEVEL] & 0x0F != 0x0C {
                let (state, percent) = battery_state(packet[BATTERY_LEVEL]);
                device.send_power_info(joystick, state, percent);
            }
        }

        self.handle_state_packet_common(device, joystick, packet, timestamp);

        let device_specific = &packet[DEVICE_SPECIFIC..DEVICE_SPECIFIC + 8];
        if self.guitar_whammy_supported {
            device.send_axis(
                timestamp,
                joystick,
                GamepadAxis::RightX as u8,
                axis_of(device_specific[1]),
            );
        }

        if self.guitar_effects_selector_supported {
            // Align pickup selector mappings with PS3 instruments
            const EFFECTS_MAPPINGS: [i16; 5] = [24576, 11008, -1792, -13568, -26880];
            if let Some(&value) = EFFECTS_MAPPINGS.get(usize::from(device_specific[0])) {
                device.send_axis(timestamp, joystick, GamepadAxis::RightY as u8, value);
            }
        }

        if self.guitar_tilt_supported {
            let sensor_data = [
                (f32::from(device_specific[2]) / 255.0) * STANDARD_GRAVITY,
                0.0,
                0.0,
            ];
            device.send_sensor(
                timestamp,
                joystick,
                SensorType::Accel,
                timestamp,
                &sensor_data,
            );

            // Align tilt mappings with PS3 instruments
            device.send_button(
                timestamp,
                joystick,
                GamepadButton::RightShoulder as u8,
                device_specific[2] > 0xF0,
            );
        }

        self.last_state.copy_from_slice(&packet[..LAST_STATE_SIZE]);
    }

    /// Translation of `HIDAPI_DriverPS5_IsPacketValid()`; `data` is the
    /// read buffer, holding a report of `size` bytes.
    fn is_packet_valid(&mut self, data: &[u8], size: usize) -> bool {
        match data[0] {
            REPORT_ID_STATE => {
                if self.is_dongle && size > alt::SIZE {
                    // The report timestamp doesn't change when the controller isn't connected
                    let sequence =
                        &data[1 + common::PACKET_SEQUENCE..1 + common::PACKET_SEQUENCE + 4];
                    let last_sequence =
                        &self.last_state[common::PACKET_SEQUENCE..common::PACKET_SEQUENCE + 4];
                    if sequence == last_sequence {
                        return false;
                    }
                    if last_sequence == [0, 0, 0, 0] {
                        // We don't have any state to compare yet, go ahead and copy it
                        self.last_state[..alt::SIZE].copy_from_slice(&data[1..1 + alt::SIZE]);
                        return false;
                    }
                }
                true
            }
            // FIXME (upstream): a report of under 4 bytes has its CRC read
            // from before the buffer; here it isn't valid.
            REPORT_ID_BLUETOOTH_STATE => size >= 4 && verify_crc(&data[..size]),
            _ => false,
        }
    }

    /// The reports of `HIDAPI_DriverPS5_UpdateDevice()`'s read loop:
    /// whether the report is valid.
    fn handle_report(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: Option<JoystickID>,
        data: &[u8],
        size: usize,
    ) -> bool {
        let timestamp = crate::timer::ticks_ns();

        if !self.is_packet_valid(data, size) {
            return false;
        }

        let Some(joystick) = joystick else {
            return true;
        };

        match data[0] {
            REPORT_ID_STATE => {
                if size == 10 || size == 78 {
                    self.handle_simple_state_packet(device, joystick, &data[1..], timestamp);
                } else if self.use_alternate_report {
                    self.handle_state_packet_alt(device, joystick, &data[1..], timestamp);
                } else {
                    self.handle_state_packet(device, joystick, &data[1..], timestamp);
                }
            }
            REPORT_ID_BLUETOOTH_STATE => {
                // This is the extended report, we can enable effects now in auto mode
                let mut js = JoystickRef::Open(joystick);
                self.update_enhanced_mode_on_enhanced_report(device, &mut js);

                if self.use_alternate_report {
                    self.handle_state_packet_alt(device, joystick, &data[2..], timestamp);
                } else {
                    self.handle_state_packet(device, joystick, &data[2..], timestamp);
                }
                if self.led_reset_state == LedResetState::Pending {
                    self.check_pending_led_reset(device, &mut js);
                }
            }
            _ => {}
        }
        true
    }

    /// The capabilities of a third party controller's capabilities report
    /// (part of `HIDAPI_DriverPS5_InitDevice()`).
    fn set_capabilities(&mut self, data: &[u8], vendor_id: u16, product_id: u16) -> JoystickType {
        let capabilities = data[4];
        let capabilities2 = data[20];
        let device_specific_capabilities = data[24];
        let device_type = data[5];

        if capabilities & 0x02 != 0 {
            self.sensors_supported = true;
        }
        if capabilities & 0x04 != 0 {
            self.lightbar_supported = true;
        }
        if capabilities & 0x08 != 0 {
            self.vibration_supported = true;
        }
        if capabilities & 0x40 != 0 {
            self.touchpad_supported = true;
        }
        if capabilities2 & 0x80 != 0 {
            self.playerled_supported = true;
        }

        if capabilities2 & 0x01 != 0 {
            self.report_battery = true;
        }

        let joystick_type = match device_type {
            0x00 => JoystickType::Gamepad,
            0x01 => {
                if device_specific_capabilities & 0x01 != 0 {
                    self.guitar_effects_selector_supported = true;
                }
                if device_specific_capabilities & 0x02 != 0 {
                    self.guitar_tilt_supported = true;
                }
                if device_specific_capabilities & 0x04 != 0 {
                    self.guitar_whammy_supported = true;
                }
                JoystickType::Guitar
            }
            0x02 => JoystickType::DrumKit,
            0x06 => JoystickType::Wheel,
            0x07 => JoystickType::ArcadeStick,
            0x08 => JoystickType::FlightStick,
            _ => JoystickType::Unknown,
        };

        self.use_alternate_report = true;
        self.report_battery = true;

        if vendor_id == USB_VENDOR_NACON_ALT
            && (product_id == USB_PRODUCT_NACON_REVOLUTION_5_PRO_PS5_WIRED
                || product_id == USB_PRODUCT_NACON_REVOLUTION_5_PRO_PS5_WIRELESS)
        {
            // This doesn't report vibration capability, but it can do rumble
            self.vibration_supported = true;
        }
        joystick_type
    }
}

/// The PS5 driver's static functions.
pub(crate) struct Ps5Driver;

impl DriverImpl for Ps5Driver {
    /// Translation of `HIDAPI_DriverPS5_RegisterHints()`.
    fn hints(&self) -> &'static [&'static str] {
        &[hints::JOYSTICK_HIDAPI_PS5]
    }

    /// Translation of `HIDAPI_DriverPS5_IsEnabled()`.
    fn is_enabled(&self) -> bool {
        hints::get_bool(
            hints::JOYSTICK_HIDAPI_PS5,
            hints::get_bool(hints::JOYSTICK_HIDAPI, SDL_HIDAPI_DEFAULT),
        )
    }

    /// Translation of `HIDAPI_DriverPS5_IsSupportedDevice()`.
    fn is_supported_device(
        &self,
        device: Option<&HidapiDevice>,
        _name: &str,
        gamepad_type: GamepadType,
        vendor_id: u16,
        product_id: u16,
        _version: u16,
        _interface_number: i32,
        _interface_class: i32,
        _interface_subclass: i32,
        _interface_protocol: i32,
    ) -> bool {
        if vendor_id == USB_VENDOR_BACKBONE && product_id == USB_PRODUCT_BACKBONE_ONE_PS5_V2 {
            // This product doesn't appear to use the DualSense protocol
            return false;
        }

        if gamepad_type == GamepadType::Ps5 {
            return true;
        }

        if supports_playstation_detection(vendor_id, product_id) {
            return match device.filter(|d| d.dev().is_some()) {
                Some(device) => {
                    let mut data = [0u8; USB_PACKET_LENGTH];
                    let size =
                        read_feature_report(device, FEATURE_REPORT_ID_CAPABILITIES, &mut data);
                    // Supported third party controller
                    matches!(size, Ok(48)) && data[2] == 0x28
                }
                // Might be supported by this driver, enumerate and find out
                None => true,
            };
        }
        false
    }

    fn new_context(&self) -> Box<dyn DriverContext> {
        Box::new(Ps5Context::default())
    }
}

impl DriverContext for Ps5Context {
    /// Translation of `HIDAPI_DriverPS5_InitDevice()`.
    fn init_device(&mut self, device: &mut DeviceCtx<'_>) -> Result<()> {
        let mut data = [0u8; USB_PACKET_LENGTH * 2];
        let mut joystick_type = JoystickType::Gamepad;

        let mut serial = dashed_serial(device.serial().as_deref());

        // Read a report to see what mode we're in
        let size = device.read_timeout(&mut data, 16).unwrap_or(0);
        if size == 64 {
            // Connected over USB
            self.enhanced_reports = true;
        } else if size > 0 && data[0] == REPORT_ID_BLUETOOTH_EFFECTS {
            // Connected over Bluetooth, using enhanced reports
            self.enhanced_reports = true;
        } else {
            // Connected over Bluetooth, using simple reports (DirectInput enabled)
        }

        let (vendor_id, product_id) = (device.vendor_id(), device.product_id());
        if vendor_id == USB_VENDOR_SONY && self.enhanced_reports {
            // Read the serial number (Bluetooth address in reverse byte order)
            // This will also enable enhanced reports over Bluetooth
            if matches!(read_feature_report(device, FEATURE_REPORT_ID_SERIAL_NUMBER, &mut data), Ok(size) if size >= 7)
            {
                serial = format!(
                    "{:02x}-{:02x}-{:02x}-{:02x}-{:02x}-{:02x}",
                    data[6], data[5], data[4], data[3], data[2], data[1]
                );
            }

            // Read the firmware version
            // This will also enable enhanced reports over Bluetooth
            if matches!(read_feature_report(device, FEATURE_REPORT_ID_FIRMWARE_INFO, &mut data[..USB_PACKET_LENGTH]), Ok(size) if size >= 46)
            {
                self.firmware_version = u16::from(data[44]) | (u16::from(data[45]) << 8);
            }
        }

        if vendor_id == USB_VENDOR_SONY
            && (product_id == USB_PRODUCT_SONY_DS5_EDGE
                || self.firmware_version == 0 // Assume that it's updated firmware over Bluetooth
                || self.firmware_version >= 0x0224)
        {
            self.enhanced_rumble = true;
        }

        // Get the device capabilities
        if vendor_id == USB_VENDOR_SONY {
            self.sensors_supported = true;
            self.lightbar_supported = true;
            self.vibration_supported = true;
            self.playerled_supported = true;
            self.touchpad_supported = true;
        } else {
            // Third party controller capability request
            let size = read_feature_report(device, FEATURE_REPORT_ID_CAPABILITIES, &mut data);
            if matches!(size, Ok(48)) && data[2] == 0x28 {
                joystick_type = self.set_capabilities(&data, vendor_id, product_id);
            } else if vendor_id == USB_VENDOR_RAZER
                && (product_id == USB_PRODUCT_RAZER_WOLVERINE_V2_PRO_PS5_WIRED
                    || product_id == USB_PRODUCT_RAZER_WOLVERINE_V2_PRO_PS5_WIRELESS)
            {
                // The Razer Wolverine V2 Pro doesn't respond to the detection protocol, but has a touchpad and sensors and no vibration
                self.sensors_supported = true;
                self.touchpad_supported = true;
                self.use_alternate_report = true;
            } else if vendor_id == USB_VENDOR_RAZER
                && (product_id == USB_PRODUCT_RAZER_KITSUNE
                    || product_id == USB_PRODUCT_RAZER_RAIJU_V3_PRO_PS5_WIRED
                    || product_id == USB_PRODUCT_RAZER_RAIJU_V3_PRO_PS5_WIRELESS)
            {
                // The Razer Kitsune and Raiju don't respond to the detection protocol, but have a touchpad
                joystick_type = JoystickType::ArcadeStick;
                self.touchpad_supported = true;
                self.use_alternate_report = true;
            }
        }
        self.effects_supported =
            self.lightbar_supported || self.vibration_supported || self.playerled_supported;

        if (vendor_id == USB_VENDOR_NACON_ALT
            && product_id == USB_PRODUCT_NACON_REVOLUTION_5_PRO_PS5_WIRELESS)
            || (vendor_id == USB_VENDOR_RAZER
                && (product_id == USB_PRODUCT_NACON_REVOLUTION_5_PRO_PS5_WIRELESS
                    || product_id == USB_PRODUCT_RAZER_WOLVERINE_V2_PRO_PS5_WIRELESS
                    || product_id == USB_PRODUCT_RAZER_RAIJU_V3_PRO_PS5_WIRELESS))
        {
            self.is_dongle = true;
        }

        device.set_joystick_type(joystick_type);
        device.set_gamepad_type(GamepadType::Ps5);
        if vendor_id == USB_VENDOR_SONY {
            if is_joystick_dual_sense_edge(vendor_id, product_id) {
                device.set_device_name("DualSense Edge Wireless Controller");
            } else {
                device.set_device_name("DualSense Wireless Controller");
            }
        }
        device.set_device_serial(&serial);

        if self.is_dongle {
            // We don't know if this is connected yet, wait for reports
            return Ok(());
        }

        // Prefer the USB device over the Bluetooth device
        let device_serial = device.serial();
        if device.is_bluetooth() {
            if device.has_connected_usb_device(device_serial.as_deref()) {
                return Ok(());
            }
        } else {
            device.disconnect_bluetooth_device(device_serial.as_deref());
        }
        device.joystick_connected();
        Ok(())
    }

    /// Translation of `HIDAPI_DriverPS5_SetDevicePlayerIndex()`.
    fn set_device_player_index(
        &mut self,
        device: &mut DeviceCtx<'_>,
        _instance_id: JoystickID,
        player_index: i32,
    ) {
        let Some(joystick) = self.joystick else {
            return;
        };

        self.player_index = player_index;

        // This will set the new LED state based on the new player index
        // SDL automatically calls this, so it doesn't count as an application action to enable enhanced mode
        let _ = self.update_effects(
            device,
            &mut JoystickRef::Open(joystick),
            EFFECT_LED | EFFECT_PAD_LIGHTS,
            false,
        );
    }

    /// Translation of `HIDAPI_DriverPS5_UpdateDevice()`.
    fn update_device(&mut self, device: &mut DeviceCtx<'_>) -> bool {
        let mut data = [0u8; USB_PACKET_LENGTH * 2];
        let mut packet_count = 0;
        let now = crate::timer::ticks_ms();

        let joystick = device.open_joystick_id();

        // (the hint callbacks of upstream)
        if let Some(joystick) = joystick.filter(|&j| self.joystick == Some(j)) {
            self.hint_changes(device, &mut JoystickRef::Open(joystick));
        }

        // (upstream's last `size >= 0`)
        let mut ok = true;
        loop {
            let size = match device.read_timeout(&mut data, 0) {
                Ok(0) => break,
                Ok(size) => size,
                Err(_) => {
                    ok = false;
                    break;
                }
            };
            if self.handle_report(device, joystick, &data, size) {
                packet_count += 1;
                self.last_packet = now;
            }
        }

        if device.is_bluetooth() {
            if packet_count == 0 {
                // Check to see if it looks like the device disconnected
                if now >= self.last_packet + BLUETOOTH_DISCONNECT_TIMEOUT_MS {
                    // Send an empty output report to tickle the Bluetooth stack
                    self.tickle_bluetooth(device);
                    self.last_packet = now;
                }
            } else {
                // Reconnect the Bluetooth device once the USB device is gone
                if device.num_joysticks() == 0
                    && !device.has_connected_usb_device(device.serial().as_deref())
                {
                    device.joystick_connected();
                }
            }
        }

        if self.is_dongle {
            if packet_count == 0 {
                if let Some(&first) = device.joysticks().first() {
                    // Check to see if it looks like the device disconnected
                    if now >= self.last_packet + BLUETOOTH_DISCONNECT_TIMEOUT_MS {
                        device.joystick_disconnected(first);
                    }
                }
            } else if device.num_joysticks() == 0 {
                device.joystick_connected();
            }
        }

        if packet_count == 0 && !ok {
            if let Some(&first) = device.joysticks().first() {
                // Read error, device is disconnected
                device.joystick_disconnected(first);
            }
        }
        ok
    }

    /// Translation of `HIDAPI_DriverPS5_OpenJoystick()`.
    fn open_joystick(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: &mut JoystickData,
    ) -> Result<()> {
        crate::joystick::assert_joysticks_locked();

        self.joystick = Some(joystick.instance_id);
        self.last_packet = crate::timer::ticks_ms();
        self.report_sensors = false;
        self.report_touchpad = false;
        self.rumble_left = 0;
        self.rumble_right = 0;
        self.color_set = false;
        self.led_reset_state = LedResetState::None;
        self.last_state = [0; LAST_STATE_SIZE];

        // Initialize player index (needed for setting LEDs)
        self.player_index = joystick_player_index_for_id(joystick.instance_id);
        self.player_lights = hints::get_bool(hints::JOYSTICK_HIDAPI_PS5_PLAYER_LED, true);

        // Initialize the joystick capabilities
        joystick.nbuttons = if is_joystick_dual_sense_edge(device.vendor_id(), device.product_id())
        {
            17 // paddles and touchpad and microphone
        } else if self.touchpad_supported {
            13 // touchpad and microphone
        } else {
            11
        };
        joystick.naxes = GamepadAxis::COUNT;
        joystick.nhats = 1;
        joystick.firmware_version = self.firmware_version;

        if self.is_dongle {
            joystick.connection_state = JoystickConnectionState::Wireless;
        }

        self.enhanced_reports_hint = Some(HintWatch::new(hints::JOYSTICK_ENHANCED_REPORTS));
        self.player_led_hint = Some(HintWatch::new(hints::JOYSTICK_HIDAPI_PS5_PLAYER_LED));
        self.hint_changes(device, &mut JoystickRef::Opening(joystick));
        Ok(())
    }

    /// Translation of `HIDAPI_DriverPS5_RumbleJoystick()`.
    fn rumble_joystick(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: JoystickID,
        low_frequency_rumble: u16,
        high_frequency_rumble: u16,
    ) -> Result<()> {
        if !self.vibration_supported {
            return Err(Error::unsupported());
        }

        let mut js = JoystickRef::Open(joystick);
        if self.rumble_left == 0 && self.rumble_right == 0 {
            let _ = self.update_effects(device, &mut js, EFFECT_RUMBLE_START, true);
        }

        self.rumble_left = (low_frequency_rumble >> 8) as u8;
        self.rumble_right = (high_frequency_rumble >> 8) as u8;

        self.update_effects(device, &mut js, EFFECT_RUMBLE, true)
    }

    /// Translation of `HIDAPI_DriverPS5_GetJoystickCapabilities()`.
    fn get_joystick_capabilities(
        &mut self,
        _device: &mut DeviceCtx<'_>,
        _joystick: JoystickID,
    ) -> JoystickCaps {
        let mut result = JoystickCaps(0);

        if self.enhanced_mode_available {
            if self.lightbar_supported {
                result |= JoystickCaps::RGB_LED;
            }
            if self.playerled_supported {
                result |= JoystickCaps::PLAYER_LED;
            }
            if self.vibration_supported {
                result |= JoystickCaps::RUMBLE;
            }
        }

        result
    }

    /// Translation of `HIDAPI_DriverPS5_SetJoystickLED()`.
    fn set_joystick_led(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: JoystickID,
        red: u8,
        green: u8,
        blue: u8,
    ) -> Result<()> {
        if !self.lightbar_supported {
            return Err(Error::unsupported());
        }

        self.color_set = true;
        self.led_red = red;
        self.led_green = green;
        self.led_blue = blue;

        self.update_effects(device, &mut JoystickRef::Open(joystick), EFFECT_LED, true)
    }

    /// Translation of `HIDAPI_DriverPS5_SendJoystickEffect()`.
    fn send_joystick_effect(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: JoystickID,
        effect: &[u8],
    ) -> Result<()> {
        self.internal_send_joystick_effect(device, &mut JoystickRef::Open(joystick), effect, true)
    }

    /// Translation of `HIDAPI_DriverPS5_SetJoystickSensorsEnabled()`.
    fn set_joystick_sensors_enabled(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: JoystickID,
        enabled: bool,
    ) -> Result<()> {
        self.update_enhanced_mode_on_application_usage(device, &mut JoystickRef::Open(joystick));

        if (!self.sensors_supported || (enabled && !self.enhanced_mode))
            && !self.guitar_tilt_supported
        {
            return Err(Error::unsupported());
        }

        if enabled {
            self.load_calibration_data(device);
        }
        self.report_sensors = enabled;

        Ok(())
    }

    /// Translation of `HIDAPI_DriverPS5_CloseJoystick()`.
    fn close_joystick(&mut self, _device: &mut DeviceCtx<'_>, _joystick: JoystickID) {
        self.enhanced_reports_hint = None;
        self.player_led_hint = None;

        self.joystick = None;

        self.report_sensors = false;
        self.enhanced_mode = false;
        self.enhanced_mode_available = false;
    }
}

#[cfg(test)]
mod tests;
