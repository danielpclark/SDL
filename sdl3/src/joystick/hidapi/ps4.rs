// Rust translation of src/joystick/hidapi/SDL_hidapi_ps4.c from Simple
// DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The PS4 controller driver. It supports both simplified reports and the
//! extended input reports enabled by Steam.
//!
//! Code and logic contributed by Valve Corporation under the SDL zlib
//! license.

use super::rumble::{lock_rumble, send_rumble};
use super::{
    load16, load32, supports_playstation_detection, DeviceCtx, DriverContext, DriverImpl,
    HidapiDevice, HintWatch, JoystickCaps, JoystickRef, SDL_HIDAPI_DEFAULT, USB_PACKET_LENGTH,
};
use crate::error::{Error, Result};
use crate::events::JoystickID;
use crate::hints;
use crate::joystick::gamepad::{GamepadAxis, GamepadButton, GamepadType};
use crate::joystick::usb_ids::*;
use crate::joystick::{
    joystick_player_index_for_id, JoystickData, JoystickType, HAT_CENTERED, HAT_DOWN, HAT_LEFT,
    HAT_LEFTDOWN, HAT_LEFTUP, HAT_RIGHT, HAT_RIGHTDOWN, HAT_RIGHTUP, HAT_UP,
};
use crate::power::PowerState;
use crate::sensor::{SensorType, STANDARD_GRAVITY};

const BLUETOOTH_DISCONNECT_TIMEOUT_MS: u64 = 500;

/// `SDL_GAMEPAD_BUTTON_PS4_TOUCHPAD`
const SDL_GAMEPAD_BUTTON_PS4_TOUCHPAD: u8 = 11;

// EPS4ReportId
const REPORT_ID_USB_STATE: u8 = 1;
const REPORT_ID_USB_EFFECTS: u8 = 5;
const REPORT_ID_BLUETOOTH_STATE1: u8 = 17;
const REPORT_ID_BLUETOOTH_STATE9: u8 = 25;
const REPORT_ID_BLUETOOTH_EFFECTS: u8 = 17;

// EPS4FeatureReportID
const FEATURE_REPORT_ID_GYRO_CALIBRATION_USB: u8 = 0x02;
const FEATURE_REPORT_ID_CAPABILITIES: u8 = 0x03;
const FEATURE_REPORT_ID_GYRO_CALIBRATION_BT: u8 = 0x05;
const FEATURE_REPORT_ID_SERIAL_NUMBER: u8 = 0x12;

/// `sizeof(PS4StatePacket_t)`
const PS4_STATE_PACKET_SIZE: usize = 54;

/// Translation of `PS4StatePacket_t` (without its padding).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) struct Ps4StatePacket {
    left_joystick_x: u8,
    left_joystick_y: u8,
    right_joystick_x: u8,
    right_joystick_y: u8,
    buttons_hat_and_counter: [u8; 3],
    trigger_left: u8,
    trigger_right: u8,
    timestamp: [u8; 2],
    gyro_x: [u8; 2],
    gyro_y: [u8; 2],
    gyro_z: [u8; 2],
    accel_x: [u8; 2],
    accel_y: [u8; 2],
    accel_z: [u8; 2],
    battery_level: u8,
    touchpad_counter1: u8,
    touchpad_data1: [u8; 3],
    touchpad_counter2: u8,
    touchpad_data2: [u8; 3],
    device_specific: [u8; 12],
}

/// `N` bytes of `data` at `at`.
fn bytes<const N: usize>(data: &[u8], at: usize) -> [u8; N] {
    let mut out = [0; N];
    out.copy_from_slice(&data[at..at + N]);
    out
}

impl Ps4StatePacket {
    /// The packet at the start of `data` (of at least
    /// `PS4_STATE_PACKET_SIZE` bytes).
    fn parse(data: &[u8]) -> Ps4StatePacket {
        let data = &data[..PS4_STATE_PACKET_SIZE];
        Ps4StatePacket {
            left_joystick_x: data[0],
            left_joystick_y: data[1],
            right_joystick_x: data[2],
            right_joystick_y: data[3],
            buttons_hat_and_counter: bytes(data, 4),
            trigger_left: data[7],
            trigger_right: data[8],
            timestamp: bytes(data, 9),
            gyro_x: bytes(data, 12),
            gyro_y: bytes(data, 14),
            gyro_z: bytes(data, 16),
            accel_x: bytes(data, 18),
            accel_y: bytes(data, 20),
            accel_z: bytes(data, 22),
            battery_level: data[29],
            touchpad_counter1: data[34],
            touchpad_data1: bytes(data, 35),
            touchpad_counter2: data[38],
            touchpad_data2: bytes(data, 39),
            device_specific: bytes(data, 42),
        }
    }
}

/// `sizeof(DS4EffectsState_t)`
const DS4_EFFECTS_STATE_SIZE: usize = 19;

/// The rumble and LED state of `DS4EffectsState_t` (whose LED delays and
/// volumes this driver leaves 0).
#[derive(Clone, Copy, Default)]
struct Ds4EffectsState {
    rumble_right: u8,
    rumble_left: u8,
    led_red: u8,
    led_green: u8,
    led_blue: u8,
}

impl Ds4EffectsState {
    fn to_bytes(self) -> [u8; DS4_EFFECTS_STATE_SIZE] {
        let mut data = [0; DS4_EFFECTS_STATE_SIZE];
        data[..5].copy_from_slice(&[
            self.rumble_right,
            self.rumble_left,
            self.led_red,
            self.led_green,
            self.led_blue,
        ]);
        data
    }
}

// EPS4Effect: these values match with the validity flags in the PS4
// output report
const EFFECT_RUMBLE: u8 = 1 << 0;
const EFFECT_LED: u8 = 1 << 1;
const EFFECT_LED_BLINK: u8 = 1 << 2;

/// Translation of `IMUCalibrationData`.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub(crate) struct ImuCalibration {
    bias: i16,
    scale: f32,
}

/// What `HIDAPI_DriverPS4_LoadOfficialCalibrationData()` read.
enum CalibrationReport {
    /// Not an official controller, or a short read
    Unavailable,
    /// Only zeros
    Empty,
    /// The calibration feature report
    Data([u8; USB_PACKET_LENGTH]),
}

/// The rumble hint mode (`HIDAPI_PS4_EnhancedReportHint`).
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

/// Translation of `SDL_DriverPS4_Context`.
#[derive(Debug, Default)]
struct Ps4Context {
    /// The open joystick (`ctx->joystick`)
    joystick: Option<JoystickID>,
    is_dongle: bool,
    is_nacon_dongle: bool,
    official_controller: bool,
    sensors_supported: bool,
    lightbar_supported: bool,
    vibration_supported: bool,
    touchpad_supported: bool,
    effects_supported: bool,
    guitar_whammy_supported: bool,
    guitar_tilt_supported: bool,
    guitar_effects_selector_supported: bool,
    enhanced_report_hint: EnhancedReportHint,
    enhanced_reports: bool,
    enhanced_mode: bool,
    enhanced_mode_available: bool,
    report_interval: u8,
    report_sensors: bool,
    report_touchpad: bool,
    report_battery: bool,
    hardware_calibration: bool,
    calibration: [ImuCalibration; 6],
    last_packet: u64,
    player_index: i32,
    rumble_left: u8,
    rumble_right: u8,
    color_set: bool,
    led_red: u8,
    led_green: u8,
    led_blue: u8,
    gyro_numerator: u16,
    gyro_denominator: u16,
    accel_numerator: u16,
    accel_denominator: u16,
    sensor_ticks: u64,
    last_tick: u16,
    /// A wrapping counter
    valid_crc_packets: u16,
    last_state: Ps4StatePacket,
    /// The `SDL_PS4ReportIntervalHintChanged()` callback
    report_interval_hint: Option<HintWatch>,
    /// The `SDL_PS4EnhancedReportsChanged()` callback
    enhanced_reports_hint: Option<HintWatch>,
}

/// Translation of `ReadFeatureReport()`.
fn read_feature_report(device: &HidapiDevice, report_id: u8, report: &mut [u8]) -> Result<usize> {
    report.fill(0);
    report[0] = report_id;
    device
        .dev()
        .ok_or_else(|| Error::invalid_param("device"))?
        .get_feature_report(report)
}

/// Whether a capabilities feature report is of a supported third party
/// controller.
fn is_third_party_capabilities(size: Result<usize>, data: &[u8]) -> bool {
    matches!(size, Ok(48)) && data[2] == 0x27
}

/// Translation of `SetLedsForPlayerIndex()`.
fn set_leds_for_player_index(effects: &mut Ds4EffectsState, player_index: i32) {
    // This list is the same as what hid-sony.c uses in the Linux kernel.
    // The first 4 values correspond to what the PS4 assigns.
    const COLORS: [[u8; 3]; 7] = [
        [0x00, 0x00, 0x40], // Blue
        [0x40, 0x00, 0x00], // Red
        [0x00, 0x40, 0x00], // Green
        [0x20, 0x00, 0x20], // Pink
        [0x02, 0x01, 0x00], // Orange
        [0x00, 0x01, 0x01], // Teal
        [0x01, 0x01, 0x01], // White
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

/// The serial number of a serial number feature report, if it has one.
fn serial_from_report(size: Result<usize>, data: &[u8]) -> Option<String> {
    match size {
        Ok(size) if size >= 7 && data[1..7].iter().any(|&b| b != 0) => Some(format!(
            "{:02x}-{:02x}-{:02x}-{:02x}-{:02x}-{:02x}",
            data[6], data[5], data[4], data[3], data[2], data[1]
        )),
        _ => None,
    }
}

/// Translation of `ReadWiredSerial()`.
fn read_wired_serial(device: &HidapiDevice) -> Option<String> {
    let mut data = [0u8; USB_PACKET_LENGTH];
    let size = read_feature_report(device, FEATURE_REPORT_ID_SERIAL_NUMBER, &mut data);
    serial_from_report(size, &data)
}

/// A 12 character serial number with dashes between the byte pairs (part
/// of `HIDAPI_DriverPS4_InitDevice()`).
fn dashed_serial(serial: Option<&str>) -> String {
    match serial {
        Some(serial) if serial.len() == 12 => {
            let pairs: Vec<&[u8]> = serial.as_bytes().chunks(2).collect();
            // (upstream copies bytes; a pair split inside a character
            // isn't valid UTF-8 and is replaced)
            String::from_utf8_lossy(&pairs.join(&b'-')).into_owned()
        }
        _ => String::new(),
    }
}

/// The hat of the PS4 hat value (0 is up, clockwise).
pub(crate) fn hat_of(value: u8) -> u8 {
    match value {
        0 => HAT_UP,
        1 => HAT_RIGHTUP,
        2 => HAT_RIGHT,
        3 => HAT_RIGHTDOWN,
        4 => HAT_DOWN,
        5 => HAT_LEFTDOWN,
        6 => HAT_LEFT,
        7 => HAT_LEFTUP,
        _ => HAT_CENTERED,
    }
}

/// The calibration in a calibration feature report (part of
/// `HIDAPI_DriverPS4_LoadOfficialCalibrationData()`); whether it's valid.
fn parse_calibration(
    calibration: &mut [ImuCalibration; 6],
    data: &[u8],
    bluetooth_or_dongle: bool,
    gyro: (u16, u16),
    accel: (u16, u16),
) -> bool {
    let at = |i: usize| i32::from(load16(data[i], data[i + 1]));
    let (gyro_numerator, gyro_denominator) = gyro;
    let (accel_numerator, accel_denominator) = accel;

    let gyro_pitch_bias = at(1);
    let gyro_yaw_bias = at(3);
    let gyro_roll_bias = at(5);

    let (
        gyro_pitch_plus,
        gyro_yaw_plus,
        gyro_roll_plus,
        gyro_pitch_minus,
        gyro_yaw_minus,
        gyro_roll_minus,
    ) = if bluetooth_or_dongle {
        (at(7), at(9), at(11), at(13), at(15), at(17))
    } else {
        (at(7), at(11), at(15), at(9), at(13), at(17))
    };

    let gyro_speed_plus = at(19);
    let gyro_speed_minus = at(21);

    let acc_x_plus = at(23);
    let acc_x_minus = at(25);
    let acc_y_plus = at(27);
    let acc_y_minus = at(29);
    let acc_z_plus = at(31);
    let acc_z_minus = at(33);

    let numerator = (gyro_speed_plus + gyro_speed_minus) as f32 * f32::from(gyro_denominator)
        / f32::from(gyro_numerator);
    let gyros = [
        (gyro_pitch_bias, gyro_pitch_plus, gyro_pitch_minus),
        (gyro_yaw_bias, gyro_yaw_plus, gyro_yaw_minus),
        (gyro_roll_bias, gyro_roll_plus, gyro_roll_minus),
    ];
    for (i, (bias, plus, minus)) in gyros.into_iter().enumerate() {
        let denominator = ((plus - bias).abs() + (minus - bias).abs()) as f32;
        if denominator != 0.0 {
            calibration[i].bias = bias as i16;
            calibration[i].scale = numerator / denominator;
        }
    }

    let accels = [
        (acc_x_plus, acc_x_minus),
        (acc_y_plus, acc_y_minus),
        (acc_z_plus, acc_z_minus),
    ];
    for (i, (plus, minus)) in accels.into_iter().enumerate() {
        let range_2g = (plus - minus) as i16;
        calibration[3 + i].bias = (plus - i32::from(range_2g) / 2) as i16;
        calibration[3 + i].scale =
            (2.0 * f32::from(accel_denominator) / f32::from(accel_numerator)) / f32::from(range_2g);
    }

    // Some controllers have a bad calibration
    calibration
        .iter()
        .all(|c| i32::from(c.bias).abs() <= 1024 && (1.0 - c.scale).abs() <= 0.5)
}

/// Scale the calibration to the units expected by SDL (part of
/// `HIDAPI_DriverPS4_LoadCalibrationData()`).
fn scale_calibration(
    calibration: &mut [ImuCalibration; 6],
    gyro: (u16, u16),
    accel: (u16, u16),
    strikepad: bool,
) {
    for (i, c) in calibration.iter_mut().enumerate() {
        let mut scale = f64::from(c.scale);

        if i < 3 {
            scale *= (f64::from(gyro.0) / f64::from(gyro.1)) * std::f64::consts::PI / 180.0;

            if strikepad {
                // The Armor-X Pro seems to only deliver half the rotation it should
                scale *= 2.0;
            }
        } else {
            scale *= (f64::from(accel.0) / f64::from(accel.1)) * f64::from(STANDARD_GRAVITY);

            if strikepad {
                // The Armor-X Pro seems to only deliver half the
                // acceleration it should, and in the opposite direction on
                // all axes
                scale *= -2.0;
            }
        }
        c.scale = scale as f32;
    }
}

/// Translation of `HIDAPI_DriverPS4_ApplyCalibrationData()`.
fn apply_calibration(calibration: &ImuCalibration, value: i16) -> f32 {
    (f32::from(value) - f32::from(calibration.bias)) * calibration.scale
}

/// Translation of `VerifyCRC()`, of a 78 byte Bluetooth report.
fn verify_crc(data: &[u8]) -> bool {
    let hdr = 0xA1; // hidp header is part of the CRC calculation
    let size = data.len();
    let crc = crate::stdlib::crc32(crate::stdlib::crc32(0, &[hdr]), &data[..size - 4]);
    let packet_crc = load32(
        data[size - 4],
        data[size - 3],
        data[size - 2],
        data[size - 1],
    );
    crc == packet_crc
}

/// Add the CRC Bluetooth output reports need at the end of the packet (at
/// least on Linux).
pub(crate) fn set_output_crc(data: &mut [u8]) {
    let hdr = 0xA2; // hidp header is part of the CRC calculation
    let size = data.len();
    let crc = crate::stdlib::crc32(crate::stdlib::crc32(0, &[hdr]), &data[..size - 4]);
    // FIXME (upstream): the CRC is copied in host byte order, which is
    // wrong on big endian hosts; it's little endian here.
    data[size - 4..].copy_from_slice(&crc.to_le_bytes());
}

impl Ps4Context {
    /// The sensor rate of the report interval.
    fn sensor_rate(&self) -> f32 {
        // (the interval hint is applied before sensors are added, so the
        // interval isn't 0)
        (1000 / u32::from(self.report_interval.max(1))) as f32
    }

    /// The calibration report read by
    /// `HIDAPI_DriverPS4_LoadOfficialCalibrationData()`.
    fn read_calibration_report(&self, device: &DeviceCtx<'_>) -> CalibrationReport {
        let mut data = [0u8; USB_PACKET_LENGTH];

        if !self.official_controller {
            // Not an official controller, ignoring calibration
            return CalibrationReport::Unavailable;
        }

        for _ in 0..5 {
            // For Bluetooth controllers, this report switches them into advanced report mode
            let mut size = match read_feature_report(
                device,
                FEATURE_REPORT_ID_GYRO_CALIBRATION_USB,
                &mut data,
            ) {
                Ok(size) if size >= 35 => size,
                // Short read of calibration data, ignoring calibration
                _ => return CalibrationReport::Unavailable,
            };

            if device.is_bluetooth() {
                size = match read_feature_report(
                    device,
                    FEATURE_REPORT_ID_GYRO_CALIBRATION_BT,
                    &mut data,
                ) {
                    Ok(size) if size >= 35 => size,
                    _ => return CalibrationReport::Unavailable,
                };
            }

            // In some cases this report returns all zeros. Usually immediately after connection with the PS4 Dongle
            if data[..size.min(data.len())].iter().any(|&b| b != 0) {
                return CalibrationReport::Data(data);
            }

            crate::timer::delay(std::time::Duration::from_millis(2));
        }
        CalibrationReport::Empty
    }

    /// Translation of `HIDAPI_DriverPS4_LoadCalibrationData()`, with the
    /// report `HIDAPI_DriverPS4_LoadOfficialCalibrationData()` read.
    fn apply_calibration_report(
        &mut self,
        report: &CalibrationReport,
        bluetooth: bool,
        strikepad: bool,
    ) {
        let official = match report {
            CalibrationReport::Unavailable => false,
            // Calibration data not available (upstream keeps the last result)
            CalibrationReport::Empty => self.hardware_calibration,
            CalibrationReport::Data(data) => {
                self.hardware_calibration = parse_calibration(
                    &mut self.calibration,
                    data,
                    bluetooth || self.is_dongle,
                    (self.gyro_numerator, self.gyro_denominator),
                    (self.accel_numerator, self.accel_denominator),
                );
                self.hardware_calibration
            }
        };
        if !official {
            self.calibration = [ImuCalibration {
                bias: 0,
                scale: 1.0,
            }; 6];
        }

        // Scale the raw data to the units expected by SDL
        scale_calibration(
            &mut self.calibration,
            (self.gyro_numerator, self.gyro_denominator),
            (self.accel_numerator, self.accel_denominator),
            strikepad,
        );
    }

    /// Translation of `HIDAPI_DriverPS4_LoadCalibrationData()`.
    fn load_calibration_data(&mut self, device: &DeviceCtx<'_>) {
        let report = self.read_calibration_report(device);
        self.apply_calibration_report(
            &report,
            device.is_bluetooth(),
            device.vendor_id() == USB_VENDOR_SONY
                && device.product_id() == USB_PRODUCT_SONY_DS4_STRIKEPAD,
        );
    }

    /// Translation of `HIDAPI_DriverPS4_UpdateEffects()`.
    fn update_effects(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: &mut JoystickRef<'_>,
        mask: u8,
        application_usage: bool,
    ) -> Result<()> {
        let mut effects = Ds4EffectsState::default();

        if self.vibration_supported {
            effects.rumble_left = self.rumble_left;
            effects.rumble_right = self.rumble_right;
        }

        if self.lightbar_supported {
            // Populate the LED state with the appropriate color from our lookup table
            if self.color_set {
                effects.led_red = self.led_red;
                effects.led_green = self.led_green;
                effects.led_blue = self.led_blue;
            } else {
                set_leds_for_player_index(&mut effects, self.player_index);
            }
        }
        self.internal_send_joystick_effect(
            device,
            joystick,
            &effects.to_bytes(),
            mask,
            application_usage,
        )
    }

    /// Translation of `HIDAPI_DriverPS4_TickleBluetooth()`.
    fn tickle_bluetooth(&mut self, device: &DeviceCtx<'_>) {
        if self.enhanced_reports {
            // This is just a dummy packet that should have no effect, since we don't set the CRC
            let mut data = [0u8; 78];

            data[0] = REPORT_ID_BLUETOOTH_EFFECTS;
            data[1] = 0xC0; // Magic value HID + CRC

            if let Ok(lock) = lock_rumble() {
                let _ = lock.send_and_unlock(device.device(), &data);
            }
        }
        // (else upstream would disconnect, but doesn't: the 8BitDo Zero 2
        // has perfect emulation of a PS4 controller, except it only sends
        // reports when the state changes)
    }

    /// Translation of `HIDAPI_DriverPS4_SetEnhancedModeAvailable()`.
    fn set_enhanced_mode_available(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: &mut JoystickRef<'_>,
    ) {
        if self.enhanced_mode_available {
            return;
        }
        self.enhanced_mode_available = true;

        let rate = self.sensor_rate();
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
                joystick.add_sensor(SensorType::Gyro, rate);
                joystick.add_sensor(SensorType::Accel, rate);
            }
            if tilt {
                joystick.add_sensor(SensorType::Accel, rate);
            }
        });
        if touchpad {
            self.report_touchpad = true;
        }

        if self.official_controller {
            self.report_battery = true;
        }

        device.update_device_properties();
    }

    /// Translation of `HIDAPI_DriverPS4_SetEnhancedMode()`.
    fn set_enhanced_mode(&mut self, device: &mut DeviceCtx<'_>, joystick: &mut JoystickRef<'_>) {
        self.set_enhanced_mode_available(device, joystick);

        if !self.enhanced_mode {
            self.enhanced_mode = true;

            // Switch into enhanced report mode
            let _ = self.update_effects(device, joystick, 0, false);
        }
    }

    /// Translation of `HIDAPI_DriverPS4_SetEnhancedReportHint()`.
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

    /// Translation of `HIDAPI_DriverPS4_UpdateEnhancedModeOnEnhancedReport()`.
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

    /// Translation of `HIDAPI_DriverPS4_UpdateEnhancedModeOnApplicationUsage()`.
    fn update_enhanced_mode_on_application_usage(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: &mut JoystickRef<'_>,
    ) {
        if self.enhanced_report_hint == EnhancedReportHint::Auto {
            self.set_enhanced_report_hint(device, joystick, EnhancedReportHint::On);
        }
    }

    /// Translation of `SDL_PS4EnhancedReportsChanged()`.
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

    /// Translation of `SDL_PS4ReportIntervalHintChanged()`.
    fn report_interval_hint_changed(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: &mut JoystickRef<'_>,
        hint: Option<&str>,
    ) {
        const DEFAULT_REPORT_INTERVAL: u8 = 4;
        let mut new_report_interval = DEFAULT_REPORT_INTERVAL;

        if let Some(hint) = hint {
            let report_interval = crate::stdlib::atoi(hint);
            if matches!(report_interval, 1 | 2 | 4) {
                // Valid values
                new_report_interval = report_interval as u8;
            }
        }

        if new_report_interval != self.report_interval {
            self.report_interval = new_report_interval;

            let _ = self.update_effects(device, joystick, 0, false);
            let rate = self.sensor_rate();
            joystick.with(|joystick| {
                joystick.set_sensor_rate(SensorType::Gyro, rate);
                joystick.set_sensor_rate(SensorType::Accel, rate);
            });
        }
    }

    /// Apply the hint changes recorded since the last call (upstream's
    /// hint callbacks, in the order they're added).
    fn hint_changes(&mut self, device: &mut DeviceCtx<'_>, joystick: &mut JoystickRef<'_>) {
        if let Some(hint) = self.report_interval_hint.as_ref().and_then(HintWatch::take) {
            self.report_interval_hint_changed(device, joystick, hint.as_deref());
        }
        if let Some(hint) = self
            .enhanced_reports_hint
            .as_ref()
            .and_then(HintWatch::take)
        {
            self.enhanced_reports_changed(device, joystick, hint.as_deref());
        }
    }

    /// Translation of `HIDAPI_DriverPS4_InternalSendJoystickEffect()`.
    fn internal_send_joystick_effect(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: &mut JoystickRef<'_>,
        effect: &[u8],
        mask: u8,
        application_usage: bool,
    ) -> Result<()> {
        if !self.effects_supported {
            // We shouldn't be sending packets to this controller
            return Err(Error::unsupported());
        }

        if !self.enhanced_mode {
            if application_usage {
                self.update_enhanced_mode_on_application_usage(device, joystick);
            }

            if !self.enhanced_mode {
                // We're not in enhanced mode, effects aren't allowed
                return Err(Error::unsupported());
            }
        }

        let (data, report_size) = self.effect_packet(device.is_bluetooth(), effect, mask);
        if send_rumble(device.device(), &data[..report_size]).ok() != Some(report_size) {
            return Err(Error::new("Couldn't send rumble packet"));
        }
        Ok(())
    }

    /// The output report of `HIDAPI_DriverPS4_InternalSendJoystickEffect()`
    /// and its size.
    fn effect_packet(&self, bluetooth: bool, effect: &[u8], mask: u8) -> ([u8; 78], usize) {
        let mut data = [0u8; 78];

        let (report_size, offset) = if bluetooth && self.official_controller {
            data[0] = REPORT_ID_BLUETOOTH_EFFECTS;
            data[1] = 0xC0 | self.report_interval; // Magic value HID + CRC, also sets update interval

            // Some third-party PS4 gamepads expect to receive both rumble and LED together,
            // so we will only send them separately to official Sony gamepads. Unfortunately,
            // this means that we'll fight with other apps that might be controlling one or
            // the other separately for third-party PS4 gamepads. We can add a whitelist for
            // compatible third-party gamepads later if we want.
            data[3] = if self.official_controller {
                mask
            } else {
                EFFECT_RUMBLE | EFFECT_LED
            };

            (78, 6)
        } else {
            data[0] = REPORT_ID_USB_EFFECTS;

            // FIXME: Should we send a consistent default effect mask between BT and USB?
            data[1] = if self.official_controller {
                mask
            } else {
                EFFECT_RUMBLE | EFFECT_LED | EFFECT_LED_BLINK
            };

            (32, 4)
        };

        let n = effect.len().min(data.len() - offset);
        data[offset..offset + n].copy_from_slice(&effect[..n]);

        if bluetooth {
            set_output_crc(&mut data[..report_size]);
        }
        (data, report_size)
    }

    /// Translation of `HIDAPI_DriverPS4_HandleStatePacket()`.
    fn handle_state_packet(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: JoystickID,
        packet: &mut Ps4StatePacket,
        size: usize,
    ) {
        // (upstream's literals)
        #[allow(clippy::excessive_precision)]
        const TOUCHPAD_SCALEX: f32 = 5.20833333e-4; // 1.0f / 1920
        #[allow(clippy::excessive_precision)]
        const TOUCHPAD_SCALEY: f32 = 1.08695652e-3; // 1.0f / 920 // This is noted as being 944 resolution, but 920 feels better
        let timestamp = crate::timer::ticks_ns();

        if size > 9 && self.report_touchpad && self.enhanced_reports {
            let touches = [
                (packet.touchpad_counter1, packet.touchpad_data1),
                (packet.touchpad_counter2, packet.touchpad_data2),
            ];
            for (finger, (counter, data)) in (0..).zip(touches) {
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

        if self.last_state.buttons_hat_and_counter[0] != packet.buttons_hat_and_counter[0] {
            let data = packet.buttons_hat_and_counter[0] >> 4;

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

            let data = packet.buttons_hat_and_counter[0] & 0x0F;
            device.send_hat(timestamp, joystick, 0, hat_of(data));
        }

        if self.last_state.buttons_hat_and_counter[1] != packet.buttons_hat_and_counter[1] {
            let data = packet.buttons_hat_and_counter[1];

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

        // Some fightsticks, ex: Victrix FS Pro will only this these digital trigger bits and not the analog values so this needs to run whenever the
        // trigger is evaluated
        if packet.buttons_hat_and_counter[1] & 0x0C != 0 {
            let data = packet.buttons_hat_and_counter[1];
            if (data & 0x04) != 0 && packet.trigger_left == 0 {
                packet.trigger_left = 255;
            }
            if (data & 0x08) != 0 && packet.trigger_right == 0 {
                packet.trigger_right = 255;
            }
        }

        if self.last_state.buttons_hat_and_counter[2] != packet.buttons_hat_and_counter[2] {
            let data = packet.buttons_hat_and_counter[2] & 0x03;

            device.send_button(
                timestamp,
                joystick,
                GamepadButton::Guide as u8,
                (data & 0x01) != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                SDL_GAMEPAD_BUTTON_PS4_TOUCHPAD,
                (data & 0x02) != 0,
            );
        }

        let axis = |value: u8| (i32::from(value) * 257 - 32768) as i16;
        device.send_axis(
            timestamp,
            joystick,
            GamepadAxis::LeftTrigger as u8,
            axis(packet.trigger_left),
        );
        device.send_axis(
            timestamp,
            joystick,
            GamepadAxis::RightTrigger as u8,
            axis(packet.trigger_right),
        );
        device.send_axis(
            timestamp,
            joystick,
            GamepadAxis::LeftX as u8,
            axis(packet.left_joystick_x),
        );
        device.send_axis(
            timestamp,
            joystick,
            GamepadAxis::LeftY as u8,
            axis(packet.left_joystick_y),
        );
        device.send_axis(
            timestamp,
            joystick,
            GamepadAxis::RightX as u8,
            axis(packet.right_joystick_x),
        );
        device.send_axis(
            timestamp,
            joystick,
            GamepadAxis::RightY as u8,
            axis(packet.right_joystick_y),
        );

        if size > 9 && self.report_battery && self.enhanced_reports {
            let level = i32::from(packet.battery_level & 0x0F);

            let (state, percent) = if packet.battery_level & 0x10 != 0 {
                if level <= 10 {
                    (PowerState::Charging, (level * 10 + 5).min(100))
                } else if level == 11 {
                    (PowerState::Charged, 100)
                } else {
                    (PowerState::Unknown, 0)
                }
            } else {
                (PowerState::OnBattery, (level * 10 + 5).min(100))
            };
            device.send_power_info(joystick, state, percent);
        }

        if size > 9 && self.report_sensors {
            let tick = load16(packet.timestamp[0], packet.timestamp[1]) as u16;
            let delta = if self.last_tick <= tick {
                tick - self.last_tick
            } else {
                // (wraps to 0 for a last tick of 0xFFFF, as upstream's Uint16)
                (u16::MAX - self.last_tick)
                    .wrapping_add(tick)
                    .wrapping_add(1)
            };
            self.sensor_ticks += u64::from(delta);
            self.last_tick = tick;

            // Sensor timestamp is in 5.33us units
            let sensor_timestamp = (self.sensor_ticks * super::NS_PER_US * 16) / 3;

            let gyro = [packet.gyro_x, packet.gyro_y, packet.gyro_z];
            let data: [f32; 3] = std::array::from_fn(|i| {
                apply_calibration(&self.calibration[i], load16(gyro[i][0], gyro[i][1]))
            });
            device.send_sensor(
                timestamp,
                joystick,
                SensorType::Gyro,
                sensor_timestamp,
                &data,
            );

            let accel = [packet.accel_x, packet.accel_y, packet.accel_z];
            let data: [f32; 3] = std::array::from_fn(|i| {
                apply_calibration(&self.calibration[3 + i], load16(accel[i][0], accel[i][1]))
            });
            device.send_sensor(
                timestamp,
                joystick,
                SensorType::Accel,
                sensor_timestamp,
                &data,
            );
        }

        if self.guitar_whammy_supported {
            device.send_axis(
                timestamp,
                joystick,
                GamepadAxis::RightX as u8,
                axis(packet.device_specific[1]),
            );
        }

        if self.guitar_effects_selector_supported {
            // Align pickup selector mappings with PS3 instruments
            const EFFECTS_MAPPINGS: [i16; 5] = [24576, 11008, -1792, -13568, -26880];
            if let Some(&value) = EFFECTS_MAPPINGS.get(usize::from(packet.device_specific[0])) {
                device.send_axis(timestamp, joystick, GamepadAxis::RightY as u8, value);
            }
        }

        if self.guitar_tilt_supported {
            let sensor_data = [
                (f32::from(packet.device_specific[2]) / 255.0) * STANDARD_GRAVITY,
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
                packet.device_specific[2] > 0xF0,
            );
        }

        self.last_state = *packet;
    }

    /// Translation of `HIDAPI_DriverPS4_IsPacketValid()`; `data` is the
    /// read buffer, holding a report of `size` bytes.
    fn is_packet_valid(&mut self, data: &[u8], size: usize) -> bool {
        match data[0] {
            REPORT_ID_USB_STATE => {
                if size == 10 {
                    // This is non-enhanced mode, this packet is fine
                    return true;
                }

                if self.is_nacon_dongle && size > PS4_STATE_PACKET_SIZE {
                    // The report timestamp doesn't change when the controller isn't connected
                    let packet = Ps4StatePacket::parse(&data[1..]);
                    if packet.timestamp == self.last_state.timestamp {
                        return false;
                    }
                    let last = &self.last_state;
                    if last.accel_x == [0, 0] && last.accel_y == [0, 0] && last.accel_z == [0, 0] {
                        // We don't have any state to compare yet, go ahead and copy it
                        self.last_state = packet;
                        return false;
                    }
                }

                // In the case of a DS4 USB dongle, bit[2] of byte 31 indicates if a DS4 is actually connected (indicated by '0').
                // For non-dongle, this bit is always 0 (connected).
                // This is usually the ID over USB, but the DS4v2 that started shipping with the PS4 Slim will also send this
                // packet over BT with a size of 128
                size >= 64 && (data[31] & 0x04) == 0
            }
            REPORT_ID_BLUETOOTH_STATE1..=REPORT_ID_BLUETOOTH_STATE9 => {
                // Bluetooth state packets have two additional bytes at the beginning, the first notes if HID data is present
                if size >= 78 && (data[1] & 0x80) != 0 {
                    if verify_crc(&data[..78]) {
                        self.valid_crc_packets = self.valid_crc_packets.wrapping_add(1);
                    } else {
                        if self.valid_crc_packets > 0 {
                            self.valid_crc_packets -= 1;
                        }
                        if self.valid_crc_packets >= 3 {
                            // We're generally getting valid CRC, but failed one
                            return false;
                        }
                    }
                    return true;
                }
                false
            }
            _ => false,
        }
    }

    /// The reports of `HIDAPI_DriverPS4_UpdateDevice()`'s read loop:
    /// whether the report is valid.
    fn handle_report(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: Option<JoystickID>,
        data: &[u8],
        size: usize,
    ) -> bool {
        if !self.is_packet_valid(data, size) {
            return false;
        }

        let Some(joystick) = joystick else {
            return true;
        };

        match data[0] {
            REPORT_ID_USB_STATE => {
                let mut packet = Ps4StatePacket::parse(&data[1..]);
                self.handle_state_packet(device, joystick, &mut packet, size - 1);
            }
            REPORT_ID_BLUETOOTH_STATE1..=REPORT_ID_BLUETOOTH_STATE9 => {
                // This is the extended report, we can enable effects now in auto mode
                self.update_enhanced_mode_on_enhanced_report(
                    device,
                    &mut JoystickRef::Open(joystick),
                );

                // Bluetooth state packets have two additional bytes at the beginning, the first notes if HID is present
                let mut packet = Ps4StatePacket::parse(&data[3..]);
                self.handle_state_packet(device, joystick, &mut packet, size - 3);
            }
            _ => {
                // Unknown PS4 packet
            }
        }
        true
    }
}

/// The PS4 driver's static functions.
pub(crate) struct Ps4Driver;

impl DriverImpl for Ps4Driver {
    /// Translation of `HIDAPI_DriverPS4_RegisterHints()`.
    fn hints(&self) -> &'static [&'static str] {
        &[hints::JOYSTICK_HIDAPI_PS4]
    }

    /// Translation of `HIDAPI_DriverPS4_IsEnabled()`.
    fn is_enabled(&self) -> bool {
        hints::get_bool(
            hints::JOYSTICK_HIDAPI_PS4,
            hints::get_bool(hints::JOYSTICK_HIDAPI, SDL_HIDAPI_DEFAULT),
        )
    }

    /// Translation of `HIDAPI_DriverPS4_IsSupportedDevice()`.
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
        if gamepad_type == GamepadType::Ps4 {
            return true;
        }

        if supports_playstation_detection(vendor_id, product_id) {
            return match device.filter(|d| d.dev().is_some()) {
                Some(device) => {
                    let mut data = [0u8; USB_PACKET_LENGTH];
                    let size =
                        read_feature_report(device, FEATURE_REPORT_ID_CAPABILITIES, &mut data);
                    // Supported third party controller
                    is_third_party_capabilities(size, &data)
                }
                // Might be supported by this driver, enumerate and find out
                None => true,
            };
        }

        false
    }

    fn new_context(&self) -> Box<dyn DriverContext> {
        Box::new(Ps4Context::default())
    }
}

/// The joystick type of a capabilities report's device type.
fn joystick_type_of(device_type: u8) -> JoystickType {
    match device_type {
        0x00 => JoystickType::Gamepad,
        0x01 => JoystickType::Guitar,
        0x02 => JoystickType::DrumKit,
        0x04 => JoystickType::DancePad,
        0x06 => JoystickType::Wheel,
        0x07 => JoystickType::ArcadeStick,
        0x08 => JoystickType::FlightStick,
        _ => JoystickType::Unknown,
    }
}

impl Ps4Context {
    /// The capabilities of a third party controller's capabilities report
    /// (part of `HIDAPI_DriverPS4_InitDevice()`).
    fn set_capabilities(&mut self, data: &[u8]) -> JoystickType {
        let capabilities = data[4];
        let device_type = data[5];
        let device_specific_capabilities = data[24];
        let gyro_numerator = load16(data[10], data[11]) as u16;
        let gyro_denominator = load16(data[12], data[13]) as u16;
        let accel_numerator = load16(data[14], data[15]) as u16;
        let accel_denominator = load16(data[16], data[17]) as u16;

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

        if device_type == 0x01 {
            if device_specific_capabilities & 0x01 != 0 {
                self.guitar_effects_selector_supported = true;
            }
            if device_specific_capabilities & 0x02 != 0 {
                self.guitar_tilt_supported = true;
            }
            if device_specific_capabilities & 0x04 != 0 {
                self.guitar_whammy_supported = true;
            }
        }

        if gyro_numerator != 0 && gyro_denominator != 0 {
            self.gyro_numerator = gyro_numerator;
            self.gyro_denominator = gyro_denominator;
        }
        if accel_numerator != 0 && accel_denominator != 0 {
            self.accel_numerator = accel_numerator;
            self.accel_denominator = accel_denominator;
        }
        joystick_type_of(device_type)
    }
}

impl DriverContext for Ps4Context {
    /// Translation of `HIDAPI_DriverPS4_InitDevice()`.
    fn init_device(&mut self, device: &mut DeviceCtx<'_>) -> Result<()> {
        let mut data = [0u8; USB_PACKET_LENGTH];
        let mut joystick_type = JoystickType::Gamepad;

        self.gyro_numerator = 1;
        self.gyro_denominator = 16;
        self.accel_numerator = 1;
        self.accel_denominator = 8192;

        let mut serial = dashed_serial(device.serial().as_deref());

        // Check for type of connection
        let (vendor_id, product_id) = (device.vendor_id(), device.product_id());
        self.is_dongle = vendor_id == USB_VENDOR_SONY && product_id == USB_PRODUCT_SONY_DS4_DONGLE;
        if self.is_dongle {
            if let Some(wired) = read_wired_serial(device) {
                serial = wired;
            }
            self.enhanced_reports = true;
        } else if vendor_id == USB_VENDOR_SONY && product_id == USB_PRODUCT_SONY_DS4_STRIKEPAD {
            self.enhanced_reports = true;
        } else if vendor_id == USB_VENDOR_SONY {
            if device.is_bluetooth() {
                // Read a report to see if we're in enhanced mode
                let size = device.read_timeout(&mut data, 16).unwrap_or(0);
                if size > 0
                    && (REPORT_ID_BLUETOOTH_STATE1..=REPORT_ID_BLUETOOTH_STATE9).contains(&data[0])
                {
                    self.enhanced_reports = true;
                }
            } else {
                if let Some(wired) = read_wired_serial(device) {
                    serial = wired;
                }
                self.enhanced_reports = true;
            }
        } else {
            // Third party controllers appear to all be wired
            self.enhanced_reports = true;
        }

        if vendor_id == USB_VENDOR_SONY {
            self.official_controller = true;
            self.sensors_supported = true;
            self.lightbar_supported = true;
            self.vibration_supported = true;
            self.touchpad_supported = true;
        } else {
            // Third party controller capability request
            let size = read_feature_report(device, FEATURE_REPORT_ID_CAPABILITIES, &mut data);
            // Get the device capabilities
            if is_third_party_capabilities(size, &data) {
                joystick_type = self.set_capabilities(&data);
            } else if vendor_id == USB_VENDOR_RAZER {
                // The Razer Raiju doesn't respond to the detection protocol, but has a touchpad and vibration
                self.vibration_supported = true;
                self.touchpad_supported = true;
            }
        }
        self.effects_supported = self.lightbar_supported || self.vibration_supported;

        if vendor_id == USB_VENDOR_NACON_ALT
            && product_id == USB_PRODUCT_NACON_REVOLUTION_5_PRO_PS4_WIRELESS
        {
            self.is_nacon_dongle = true;
        }

        if vendor_id == USB_VENDOR_PDP
            && (product_id == USB_PRODUCT_VICTRIX_FS_PRO
                || product_id == USB_PRODUCT_VICTRIX_FS_PRO_V2)
        {
            // The Victrix FS Pro V2 reports that it has lightbar support,
            // but it doesn't respond to the effects packet, and will hang
            // on reboot if we send it.
            self.effects_supported = false;
        }

        device.set_joystick_type(joystick_type);
        device.set_gamepad_type(GamepadType::Ps4);
        if self.official_controller {
            device.set_device_name("PS4 Controller");
        }
        device.set_device_serial(&serial);

        // Prefer the USB device over the Bluetooth device
        let device_serial = device.serial();
        if device.is_bluetooth() {
            if device.has_connected_usb_device(device_serial.as_deref()) {
                return Ok(());
            }
        } else {
            device.disconnect_bluetooth_device(device_serial.as_deref());
        }
        if (self.is_dongle || self.is_nacon_dongle) && serial.is_empty() {
            // Not yet connected
            return Ok(());
        }
        device.joystick_connected();
        Ok(())
    }

    /// Translation of `HIDAPI_DriverPS4_SetDevicePlayerIndex()`.
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
        let _ = self.update_effects(device, &mut JoystickRef::Open(joystick), EFFECT_LED, false);
    }

    /// Translation of `HIDAPI_DriverPS4_UpdateDevice()`.
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

        if self.is_dongle || self.is_nacon_dongle {
            if packet_count == 0 {
                if let Some(&first) = device.joysticks().first() {
                    // Check to see if it looks like the device disconnected
                    if now >= self.last_packet + BLUETOOTH_DISCONNECT_TIMEOUT_MS {
                        device.joystick_disconnected(first);
                    }
                }
            } else if device.num_joysticks() == 0 {
                let size = read_feature_report(device, FEATURE_REPORT_ID_SERIAL_NUMBER, &mut data);
                ok = size.is_ok();
                if let Some(serial) = serial_from_report(size, &data) {
                    device.set_device_serial(&serial);
                }
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

    /// Translation of `HIDAPI_DriverPS4_OpenJoystick()`.
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
        self.last_state = Ps4StatePacket::default();

        // Initialize player index (needed for setting LEDs)
        self.player_index = joystick_player_index_for_id(joystick.instance_id);

        // Initialize the joystick capabilities
        joystick.nbuttons = 11;
        if self.touchpad_supported {
            joystick.nbuttons += 1;
        }
        joystick.naxes = GamepadAxis::COUNT;
        joystick.nhats = 1;

        self.report_interval_hint =
            Some(HintWatch::new(hints::JOYSTICK_HIDAPI_PS4_REPORT_INTERVAL));
        self.enhanced_reports_hint = Some(HintWatch::new(hints::JOYSTICK_ENHANCED_REPORTS));
        self.hint_changes(device, &mut JoystickRef::Opening(joystick));
        Ok(())
    }

    /// Translation of `HIDAPI_DriverPS4_RumbleJoystick()`.
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

        self.rumble_left = (low_frequency_rumble >> 8) as u8;
        self.rumble_right = (high_frequency_rumble >> 8) as u8;

        self.update_effects(
            device,
            &mut JoystickRef::Open(joystick),
            EFFECT_RUMBLE,
            true,
        )
    }

    /// Translation of `HIDAPI_DriverPS4_GetJoystickCapabilities()`.
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
            if self.vibration_supported {
                result |= JoystickCaps::RUMBLE;
            }
        }

        result
    }

    /// Translation of `HIDAPI_DriverPS4_SetJoystickLED()`.
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

    /// Translation of `HIDAPI_DriverPS4_SendJoystickEffect()`.
    fn send_joystick_effect(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: JoystickID,
        effect: &[u8],
    ) -> Result<()> {
        // Unlike the PS5 driver, the PS4 driver does not expect the validity bits as part of
        // the provided effect data. The default values we provide also diverge for different
        // gamepads and connection mediums. We should probably clean this up eventually.
        let mask = if device.is_bluetooth() && self.official_controller {
            EFFECT_RUMBLE | EFFECT_LED
        } else {
            EFFECT_RUMBLE | EFFECT_LED | EFFECT_LED_BLINK
        };

        self.internal_send_joystick_effect(
            device,
            &mut JoystickRef::Open(joystick),
            effect,
            mask,
            true,
        )
    }

    /// Translation of `HIDAPI_DriverPS4_SetJoystickSensorsEnabled()`.
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

    /// Translation of `HIDAPI_DriverPS4_CloseJoystick()`.
    fn close_joystick(&mut self, _device: &mut DeviceCtx<'_>, _joystick: JoystickID) {
        self.report_interval_hint = None;
        self.enhanced_reports_hint = None;

        self.joystick = None;

        self.report_sensors = false;
        self.enhanced_mode = false;
        self.enhanced_mode_available = false;
    }
}

#[cfg(test)]
mod tests;
