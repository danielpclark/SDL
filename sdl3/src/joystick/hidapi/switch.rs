// Rust translation of src/joystick/hidapi/SDL_hidapi_switch.c and
// SDL_hidapi_nintendo.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Nintendo Switch Pro controller driver, with the Joy-Con and
//! Nintendo Online classic controller drivers that share its code.
//!
//! Code and logic contributed by Valve Corporation under the SDL zlib
//! license.
//!
//! Not translated: the macOS wait for the OS handshake when opening.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use super::rumble::lock_rumble;
use super::{
    remap_val, DeviceCtx, DriverContext, DriverImpl, HidapiDevice, HintWatch, JoystickCaps,
    JoystickRef, SDL_HIDAPI_DEFAULT,
};
use crate::error::{Error, Result};
use crate::events::JoystickID;
use crate::hints;
use crate::joystick::gamepad::{GamepadAxis, GamepadButton, GamepadType};
use crate::joystick::usb_ids::*;
use crate::joystick::{
    is_joystick_gamecube, is_joystick_nintendo_switch2_pro,
    is_joystick_nintendo_switch2_pro_input_only, is_joystick_nintendo_switch_pro_input_only,
    joystick_player_index_for_id, JoystickData, HAT_CENTERED, HAT_DOWN, HAT_LEFT, HAT_LEFTDOWN,
    HAT_LEFTUP, HAT_RIGHT, HAT_RIGHTDOWN, HAT_RIGHTUP, HAT_UP,
};
use crate::power::PowerState;
use crate::sensor::{SensorType, STANDARD_GRAVITY};

// SDL_hidapi_nintendo.h

/// The controller type byte of the controller GUID. These values come
/// directly out of the hardware, so other values occur too. Translation
/// of `ESwitchDeviceInfoControllerType`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Default)]
pub(crate) struct ControllerType(pub(crate) u8);

impl ControllerType {
    pub(crate) const UNKNOWN: ControllerType = ControllerType(0);
    pub(crate) const JOYCON_LEFT: ControllerType = ControllerType(1);
    pub(crate) const JOYCON_RIGHT: ControllerType = ControllerType(2);
    pub(crate) const PRO_CONTROLLER: ControllerType = ControllerType(3);
    pub(crate) const LIC_PRO_CONTROLLER: ControllerType = ControllerType(6);
    pub(crate) const HVC_LEFT: ControllerType = ControllerType(7);
    pub(crate) const HVC_RIGHT: ControllerType = ControllerType(8);
    pub(crate) const NES_LEFT: ControllerType = ControllerType(9);
    pub(crate) const NES_RIGHT: ControllerType = ControllerType(10);
    pub(crate) const SNES: ControllerType = ControllerType(11);
    pub(crate) const N64: ControllerType = ControllerType(12);
    pub(crate) const SEGA_GENESIS: ControllerType = ControllerType(13);
}

// SDL_hidapi_switch.c

/// How often you can write rumble commands to the controller. If you send
/// commands more frequently than this, you can turn off the controller in
/// Bluetooth mode, or the motors can miss the command in USB mode.
const RUMBLE_WRITE_FREQUENCY_MS: u64 = 30;

/// How often you have to refresh a long duration rumble to keep the
/// motors running
const RUMBLE_REFRESH_FREQUENCY_MS: u64 = 50;

const SWITCH_GYRO_SCALE: f32 = 14.2842;
const SWITCH_ACCEL_SCALE: f32 = 4096.0;

const SWITCH_GYRO_SCALE_MULT: f32 = 936.0;
const SWITCH_ACCEL_SCALE_MULT: f32 = 4.0;

// The Switch buttons past the standard ones
const SDL_GAMEPAD_BUTTON_SWITCH_SHARE: u8 = 11;
const SDL_GAMEPAD_BUTTON_SWITCH_RIGHT_PADDLE1: u8 = 12;
const SDL_GAMEPAD_BUTTON_SWITCH_LEFT_PADDLE1: u8 = 13;
const SDL_GAMEPAD_BUTTON_SWITCH_RIGHT_PADDLE2: u8 = 14;
const SDL_GAMEPAD_BUTTON_SWITCH_LEFT_PADDLE2: u8 = 15;
const SDL_GAMEPAD_NUM_SWITCH_BUTTONS: usize = 16;

const SDL_GAMEPAD_NUM_SWITCH_INPUT_ONLY_BUTTONS: usize = 12;

const SDL_GAMEPAD_BUTTON_SWITCH2_C: u8 = 12;
const SDL_GAMEPAD_NUM_SWITCH2_BUTTONS: usize = 13;

// ESwitchInputReportIDs
const INPUT_REPORT_SUBCOMMAND_REPLY: u8 = 0x21;
const INPUT_REPORT_FULL_CONTROLLER_STATE: u8 = 0x30;
const INPUT_REPORT_FULL_CONTROLLER_AND_MCU_STATE: u8 = 0x31;
const INPUT_REPORT_SIMPLE_CONTROLLER_STATE: u8 = 0x3F;
const INPUT_REPORT_COMMAND_ACK: u8 = 0x81;

// ESwitchOutputReportIDs
const OUTPUT_REPORT_RUMBLE_AND_SUBCOMMAND: u8 = 0x01;
const OUTPUT_REPORT_RUMBLE: u8 = 0x10;
const OUTPUT_REPORT_PROPRIETARY: u8 = 0x80;

// ESwitchSubcommandIDs
const SUBCOMMAND_REQUEST_DEVICE_INFO: u8 = 0x02;
const SUBCOMMAND_SET_INPUT_REPORT_MODE: u8 = 0x03;
const SUBCOMMAND_SPI_FLASH_READ: u8 = 0x10;
const SUBCOMMAND_SET_PLAYER_LIGHTS: u8 = 0x30;
const SUBCOMMAND_SET_HOME_LIGHT: u8 = 0x38;
const SUBCOMMAND_ENABLE_IMU: u8 = 0x40;
const SUBCOMMAND_ENABLE_VIBRATION: u8 = 0x48;

// ESwitchProprietaryCommandIDs
const PROPRIETARY_STATUS: u8 = 0x01;
const PROPRIETARY_HANDSHAKE: u8 = 0x02;
const PROPRIETARY_HIGH_SPEED: u8 = 0x03;
const PROPRIETARY_FORCE_USB: u8 = 0x04;

const SWITCH_OUTPUT_PACKET_DATA_LENGTH: usize = 49;
const SWITCH_MAX_OUTPUT_PACKET_LENGTH: usize = 64;
const SWITCH_BLUETOOTH_PACKET_LENGTH: usize = SWITCH_OUTPUT_PACKET_DATA_LENGTH;
const SWITCH_USB_PACKET_LENGTH: usize = SWITCH_MAX_OUTPUT_PACKET_LENGTH;

const SPI_STICK_FACTORY_CALIBRATION_START_OFFSET: u32 = 0x603D;
const SPI_STICK_FACTORY_CALIBRATION_END_OFFSET: u32 = 0x604E;
const SPI_STICK_FACTORY_CALIBRATION_LENGTH: u8 = (SPI_STICK_FACTORY_CALIBRATION_END_OFFSET
    - SPI_STICK_FACTORY_CALIBRATION_START_OFFSET
    + 1) as u8;

const SPI_STICK_USER_CALIBRATION_START_OFFSET: u32 = 0x8010;
const SPI_STICK_USER_CALIBRATION_END_OFFSET: u32 = 0x8025;
const SPI_STICK_USER_CALIBRATION_LENGTH: u8 =
    (SPI_STICK_USER_CALIBRATION_END_OFFSET - SPI_STICK_USER_CALIBRATION_START_OFFSET + 1) as u8;

const SPI_IMU_SCALE_START_OFFSET: u32 = 0x6020;
const SPI_IMU_SCALE_END_OFFSET: u32 = 0x6037;
const SPI_IMU_SCALE_LENGTH: u8 = (SPI_IMU_SCALE_END_OFFSET - SPI_IMU_SCALE_START_OFFSET + 1) as u8;

const SPI_IMU_USER_SCALE_START_OFFSET: u32 = 0x8026;
const SPI_IMU_USER_SCALE_END_OFFSET: u32 = 0x8039;
const SPI_IMU_USER_SCALE_LENGTH: u8 =
    (SPI_IMU_USER_SCALE_END_OFFSET - SPI_IMU_USER_SCALE_START_OFFSET + 1) as u8;

// The packed packets, as byte offsets.

/// `sizeof(SwitchInputOnlyControllerStatePacket_t)`: buttons[2], the
/// stick hat, the left and right sticks (a byte per axis).
const INPUT_ONLY_STATE_SIZE: usize = 7;
/// `sizeof(SwitchSimpleStatePacket_t)`: buttons[2], the stick hat, the
/// left and right sticks (16 bits per axis).
const SIMPLE_STATE_SIZE: usize = 11;
/// `sizeof(SwitchControllerStatePacket_t)`
const CONTROLLER_STATE_SIZE: usize = 12;
/// `sizeof(SwitchControllerIMUState_t)`
const IMU_STATE_SIZE: usize = 12;
/// `sizeof(SwitchStatePacket_t)`
const FULL_STATE_SIZE: usize = CONTROLLER_STATE_SIZE + 3 * IMU_STATE_SIZE;

// SwitchControllerStatePacket_t
const STATE_BATTERY_AND_CONNECTION: usize = 1;
const STATE_BUTTONS: usize = 2;
const STATE_JOYSTICK_LEFT: usize = 5;
const STATE_JOYSTICK_RIGHT: usize = 8;

// SwitchSubcommandInputPacket_t
const REPLY_SUBCOMMAND_ACK: usize = CONTROLLER_STATE_SIZE;
const REPLY_SUBCOMMAND_ID: usize = CONTROLLER_STATE_SIZE + 1;
const REPLY_SUBCOMMAND_DATA: usize = CONTROLLER_STATE_SIZE + 2;
/// `sizeof(SwitchSPIOpData_t)`: the address (32 bits) and the length
const SPI_OP_DATA_SIZE: usize = 5;
/// `spiReadData.rgucReadData`
const REPLY_SPI_READ_DATA: usize = REPLY_SUBCOMMAND_DATA + SPI_OP_DATA_SIZE;
/// `k_unSubcommandDataBytes - sizeof(SwitchSPIOpData_t)`
const SPI_READ_DATA_SIZE: usize = 35 - SPI_OP_DATA_SIZE;
/// `deviceInfo.ucDeviceType`
const REPLY_DEVICE_TYPE: usize = REPLY_SUBCOMMAND_DATA + 2;
/// `deviceInfo.rgucMACAddress`
const REPLY_MAC_ADDRESS: usize = REPLY_SUBCOMMAND_DATA + 4;

/// `sizeof(SwitchCommonOutputPacket_t)`
const COMMON_OUTPUT_PACKET_SIZE: usize = 10;
/// `sizeof(SwitchSubcommandOutputPacket_t::rgucSubcommandData)`
const SUBCOMMAND_DATA_SIZE: usize =
    SWITCH_OUTPUT_PACKET_DATA_LENGTH - COMMON_OUTPUT_PACKET_SIZE - 1;
/// `sizeof(SwitchProprietaryOutputPacket_t::rgucProprietaryData)`
const PROPRIETARY_DATA_SIZE: usize = SWITCH_OUTPUT_PACKET_DATA_LENGTH - 1 - 1;

/// An SPI flash read (`SwitchSPIOpData_t`), as sent and echoed.
fn spi_op_data(address: u32, length: u8) -> [u8; SPI_OP_DATA_SIZE] {
    let mut data = [0; SPI_OP_DATA_SIZE];
    data[..4].copy_from_slice(&address.to_le_bytes());
    data[4] = length;
    data
}

/// The rumble state sent with every output packet
/// (`SwitchCommonOutputPacket_t`).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
struct CommonOutputPacket {
    packet_type: u8,
    packet_number: u8,
    rumble_data: [[u8; 4]; 2],
}

impl CommonOutputPacket {
    fn to_bytes(self) -> [u8; COMMON_OUTPUT_PACKET_SIZE] {
        let mut data = [0; COMMON_OUTPUT_PACKET_SIZE];
        data[0] = self.packet_type;
        data[1] = self.packet_number;
        data[2..6].copy_from_slice(&self.rumble_data[0]);
        data[6..10].copy_from_slice(&self.rumble_data[1]);
        data
    }
}

/// The enhanced report hint mode (`HIDAPI_Switch_EnhancedReportHint`).
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

/// An axis of `struct StickCalibrationData`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
struct StickAxisCalibration {
    center: i16,
    min: i16,
    max: i16,
}

/// An axis of `struct StickExtents`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
struct StickExtent {
    min: i16,
    max: i16,
}

/// Translation of `struct IMUScaleData`.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
struct ImuScaleData {
    accel_scale_x: f32,
    accel_scale_y: f32,
    accel_scale_z: f32,

    gyro_scale_x: f32,
    gyro_scale_y: f32,
    gyro_scale_z: f32,

    gyro_offset_x: i16,
    gyro_offset_y: i16,
    gyro_offset_z: i16,
}

/// Where a context's packets go (`ctx->device`).
#[derive(Clone, Copy)]
enum Link<'a> {
    /// A device with a driver, which can also be written to through the
    /// rumble thread
    Device(&'a Arc<HidapiDevice>),
    /// A device being probed, written to directly
    Probe(&'a HidapiDevice),
}

impl Link<'_> {
    fn device(&self) -> &HidapiDevice {
        match self {
            Link::Device(device) => device,
            Link::Probe(device) => device,
        }
    }

    /// `SDL_hid_write(ctx->device->dev, ...)`
    fn write(&self, data: &[u8]) -> Result<usize> {
        self.device()
            .dev()
            .ok_or_else(|| Error::invalid_param("device"))?
            .write(data)
    }
}

/// Translation of `SDL_DriverSwitch_Context`.
#[derive(Debug)]
struct SwitchContext {
    /// The open joystick (`ctx->joystick`)
    joystick: Option<JoystickID>,
    input_only: bool,
    switch2: bool,
    use_button_labels: bool,
    player_lights: bool,
    player_index: i32,
    sync_write: bool,
    max_write_attempts: i32,
    controller_type: ControllerType,
    initial_input_mode: u8,
    current_input_mode: u8,
    mac_address: [u8; 6],
    command_number: u8,
    enhanced_report_hint: EnhancedReportHint,
    enhanced_mode: bool,
    enhanced_mode_available: bool,
    rumble_packet: CommonOutputPacket,
    read_buffer: [u8; SWITCH_MAX_OUTPUT_PACKET_LENGTH],
    rumble_active: bool,
    rumble_sent: u64,
    rumble_pending: bool,
    rumble_zero_pending: bool,
    /// The low (high 16 bits) and high frequency rumble that's pending
    rumble_pending_value: u32,
    sensors_supported: bool,
    report_sensors: bool,
    has_sensor_data: bool,
    last_input: u64,
    last_imu_reset: u64,
    imu_sample_timestamp_ns: u64,
    imu_samples: u32,
    imu_update_interval_ns: u64,
    timestamp_ns: u64,
    vertical_mode: bool,
    power_state: PowerState,
    power_percent: i32,

    last_input_only_state: [u8; INPUT_ONLY_STATE_SIZE],
    last_simple_state: [u8; SIMPLE_STATE_SIZE],
    last_full_state: [u8; FULL_STATE_SIZE],

    stick_cal_data: [[StickAxisCalibration; 2]; 2],
    stick_extents: [[StickExtent; 2]; 2],
    simple_stick_extents: [[StickExtent; 2]; 2],
    imu_scale_data: ImuScaleData,

    /// The `SDL_EnhancedReportsChanged()` callback
    enhanced_reports_hint: Option<HintWatch>,
    /// The `SDL_HomeLEDHintChanged()` callback
    home_led_hint: Option<HintWatch>,
    /// The `SDL_PlayerLEDHintChanged()` callback
    player_led_hint: Option<HintWatch>,
}

impl Default for SwitchContext {
    fn default() -> Self {
        SwitchContext {
            joystick: None,
            input_only: false,
            switch2: false,
            use_button_labels: false,
            player_lights: false,
            player_index: 0,
            sync_write: false,
            max_write_attempts: 0,
            controller_type: ControllerType::UNKNOWN,
            initial_input_mode: 0,
            current_input_mode: 0,
            mac_address: [0; 6],
            command_number: 0,
            enhanced_report_hint: EnhancedReportHint::Off,
            enhanced_mode: false,
            enhanced_mode_available: false,
            rumble_packet: CommonOutputPacket::default(),
            read_buffer: [0; SWITCH_MAX_OUTPUT_PACKET_LENGTH],
            rumble_active: false,
            rumble_sent: 0,
            rumble_pending: false,
            rumble_zero_pending: false,
            rumble_pending_value: 0,
            sensors_supported: false,
            report_sensors: false,
            has_sensor_data: false,
            last_input: 0,
            last_imu_reset: 0,
            imu_sample_timestamp_ns: 0,
            imu_samples: 0,
            imu_update_interval_ns: 0,
            timestamp_ns: 0,
            vertical_mode: false,
            power_state: PowerState::Unknown,
            power_percent: 0,
            last_input_only_state: [0; INPUT_ONLY_STATE_SIZE],
            last_simple_state: [0; SIMPLE_STATE_SIZE],
            last_full_state: [0; FULL_STATE_SIZE],
            stick_cal_data: [[StickAxisCalibration::default(); 2]; 2],
            stick_extents: [[StickExtent::default(); 2]; 2],
            simple_stick_extents: [[StickExtent::default(); 2]; 2],
            imu_scale_data: ImuScaleData::default(),
            enhanced_reports_hint: None,
            home_led_hint: None,
            player_led_hint: None,
        }
    }
}

/// Translation of `EncodeRumbleHighAmplitude()`.
fn encode_rumble_high_amplitude(amplitude: u16) -> u8 {
    // More information about these values can be found here:
    // https://github.com/dekuNukem/Nintendo_Switch_Reverse_Engineering/blob/master/rumble_data_table.md
    const HFA: [(u16, u8); 101] = [
        (0, 0x0),
        (514, 0x2),
        (775, 0x4),
        (921, 0x6),
        (1096, 0x8),
        (1303, 0x0a),
        (1550, 0x0c),
        (1843, 0x0e),
        (2192, 0x10),
        (2606, 0x12),
        (3100, 0x14),
        (3686, 0x16),
        (4383, 0x18),
        (5213, 0x1a),
        (6199, 0x1c),
        (7372, 0x1e),
        (7698, 0x20),
        (8039, 0x22),
        (8395, 0x24),
        (8767, 0x26),
        (9155, 0x28),
        (9560, 0x2a),
        (9984, 0x2c),
        (10426, 0x2e),
        (10887, 0x30),
        (11369, 0x32),
        (11873, 0x34),
        (12398, 0x36),
        (12947, 0x38),
        (13520, 0x3a),
        (14119, 0x3c),
        (14744, 0x3e),
        (15067, 0x40),
        (15397, 0x42),
        (15734, 0x44),
        (16079, 0x46),
        (16431, 0x48),
        (16790, 0x4a),
        (17158, 0x4c),
        (17534, 0x4e),
        (17918, 0x50),
        (18310, 0x52),
        (18711, 0x54),
        (19121, 0x56),
        (19540, 0x58),
        (19967, 0x5a),
        (20405, 0x5c),
        (20851, 0x5e),
        (21308, 0x60),
        (21775, 0x62),
        (22251, 0x64),
        (22739, 0x66),
        (23236, 0x68),
        (23745, 0x6a),
        (24265, 0x6c),
        (24797, 0x6e),
        (25340, 0x70),
        (25894, 0x72),
        (26462, 0x74),
        (27041, 0x76),
        (27633, 0x78),
        (28238, 0x7a),
        (28856, 0x7c),
        (29488, 0x7e),
        (30134, 0x80),
        (30794, 0x82),
        (31468, 0x84),
        (32157, 0x86),
        (32861, 0x88),
        (33581, 0x8a),
        (34316, 0x8c),
        (35068, 0x8e),
        (35836, 0x90),
        (36620, 0x92),
        (37422, 0x94),
        (38242, 0x96),
        (39079, 0x98),
        (39935, 0x9a),
        (40809, 0x9c),
        (41703, 0x9e),
        (42616, 0xa0),
        (43549, 0xa2),
        (44503, 0xa4),
        (45477, 0xa6),
        (46473, 0xa8),
        (47491, 0xaa),
        (48531, 0xac),
        (49593, 0xae),
        (50679, 0xb0),
        (51789, 0xb2),
        (52923, 0xb4),
        (54082, 0xb6),
        (55266, 0xb8),
        (56476, 0xba),
        (57713, 0xbc),
        (58977, 0xbe),
        (60268, 0xc0),
        (61588, 0xc2),
        (62936, 0xc4),
        (64315, 0xc6),
        (65535, 0xc8),
    ];
    HFA.iter()
        .find(|&&(limit, _)| amplitude <= limit)
        .map_or(HFA[100].1, |&(_, value)| value)
}

/// Translation of `EncodeRumbleLowAmplitude()`.
fn encode_rumble_low_amplitude(amplitude: u16) -> u16 {
    // More information about these values can be found here:
    // https://github.com/dekuNukem/Nintendo_Switch_Reverse_Engineering/blob/master/rumble_data_table.md
    const LFA: [(u16, u16); 101] = [
        (0, 0x0040),
        (514, 0x8040),
        (775, 0x0041),
        (921, 0x8041),
        (1096, 0x0042),
        (1303, 0x8042),
        (1550, 0x0043),
        (1843, 0x8043),
        (2192, 0x0044),
        (2606, 0x8044),
        (3100, 0x0045),
        (3686, 0x8045),
        (4383, 0x0046),
        (5213, 0x8046),
        (6199, 0x0047),
        (7372, 0x8047),
        (7698, 0x0048),
        (8039, 0x8048),
        (8395, 0x0049),
        (8767, 0x8049),
        (9155, 0x004a),
        (9560, 0x804a),
        (9984, 0x004b),
        (10426, 0x804b),
        (10887, 0x004c),
        (11369, 0x804c),
        (11873, 0x004d),
        (12398, 0x804d),
        (12947, 0x004e),
        (13520, 0x804e),
        (14119, 0x004f),
        (14744, 0x804f),
        (15067, 0x0050),
        (15397, 0x8050),
        (15734, 0x0051),
        (16079, 0x8051),
        (16431, 0x0052),
        (16790, 0x8052),
        (17158, 0x0053),
        (17534, 0x8053),
        (17918, 0x0054),
        (18310, 0x8054),
        (18711, 0x0055),
        (19121, 0x8055),
        (19540, 0x0056),
        (19967, 0x8056),
        (20405, 0x0057),
        (20851, 0x8057),
        (21308, 0x0058),
        (21775, 0x8058),
        (22251, 0x0059),
        (22739, 0x8059),
        (23236, 0x005a),
        (23745, 0x805a),
        (24265, 0x005b),
        (24797, 0x805b),
        (25340, 0x005c),
        (25894, 0x805c),
        (26462, 0x005d),
        (27041, 0x805d),
        (27633, 0x005e),
        (28238, 0x805e),
        (28856, 0x005f),
        (29488, 0x805f),
        (30134, 0x0060),
        (30794, 0x8060),
        (31468, 0x0061),
        (32157, 0x8061),
        (32861, 0x0062),
        (33581, 0x8062),
        (34316, 0x0063),
        (35068, 0x8063),
        (35836, 0x0064),
        (36620, 0x8064),
        (37422, 0x0065),
        (38242, 0x8065),
        (39079, 0x0066),
        (39935, 0x8066),
        (40809, 0x0067),
        (41703, 0x8067),
        (42616, 0x0068),
        (43549, 0x8068),
        (44503, 0x0069),
        (45477, 0x8069),
        (46473, 0x006a),
        (47491, 0x806a),
        (48531, 0x006b),
        (49593, 0x806b),
        (50679, 0x006c),
        (51789, 0x806c),
        (52923, 0x006d),
        (54082, 0x806d),
        (55266, 0x006e),
        (56476, 0x806e),
        (57713, 0x006f),
        (58977, 0x806f),
        (60268, 0x0070),
        (61588, 0x8070),
        (62936, 0x0071),
        (64315, 0x8071),
        (65535, 0x0072),
    ];
    LFA.iter()
        .find(|&&(limit, _)| amplitude <= limit)
        .map_or(LFA[100].1, |&(_, value)| value)
}

/// Translation of `SetNeutralRumble()`.
fn neutral_rumble(vendor_id: u16, product_id: u16) -> [u8; 4] {
    if vendor_id == USB_VENDOR_NINTENDO && product_id == USB_PRODUCT_NINTENDO_N64_CONTROLLER {
        // The 8BitDo 64 Bluetooth Controller rumbles at startup with the standard neutral value,
        // so we'll use a 0 amplitude value instead.
        [0x00, 0x00, 0x01, 0x40]
    } else {
        // The KingKong2 PRO Controller doesn't initialize correctly with a 0 amplitude value
        // over Bluetooth, so we'll use the standard value in all other cases.
        [0x00, 0x01, 0x40, 0x40]
    }
}

/// Translation of `EncodeRumble()`.
fn encode_rumble(
    vendor_id: u16,
    product_id: u16,
    high_freq: u16,
    high_freq_amp: u8,
    low_freq: u8,
    low_freq_amp: u16,
) -> [u8; 4] {
    if high_freq_amp > 0 || low_freq_amp > 0 {
        // High-band frequency and low-band amplitude are actually nine-bits each so they
        // take a bit from the high-band amplitude and low-band frequency bytes respectively
        [
            (high_freq & 0xFF) as u8,
            high_freq_amp | ((high_freq >> 8) & 0x01) as u8,
            low_freq | ((low_freq_amp >> 8) & 0x80) as u8,
            (low_freq_amp & 0xFF) as u8,
        ]
    } else {
        neutral_rumble(vendor_id, product_id)
    }
}

/// Translation of `CalculateControllerType()`.
fn calculate_controller_type(
    device: &HidapiDevice,
    mut controller_type: ControllerType,
) -> ControllerType {
    // The N64 controller reports as a Pro controller over USB
    if controller_type == ControllerType::PRO_CONTROLLER
        && device.product_id() == USB_PRODUCT_NINTENDO_N64_CONTROLLER
    {
        controller_type = ControllerType::N64;
    }

    if controller_type == ControllerType::UNKNOWN {
        // This might be a Joy-Con that's missing from a charging grip slot
        if device.product_id() == USB_PRODUCT_NINTENDO_SWITCH_JOYCON_GRIP {
            if device.interface_number() == 1 {
                controller_type = ControllerType::JOYCON_LEFT;
            } else {
                controller_type = ControllerType::JOYCON_RIGHT;
            }
        }
    }
    controller_type
}

/// The LED intensity of `SetHomeLED()`.
fn home_led_intensity(brightness: u8) -> u8 {
    if brightness == 0 {
        0
    } else if brightness < 65 {
        (brightness + 5) / 10
    } else {
        (15.0 * (f32::from(brightness) / 100.0).powf(2.13)).ceil() as u8
    }
}

/// The value of a home LED hint (part of `SDL_HomeLEDHintChanged()`), if
/// it's set.
fn home_led_hint_value(hint: Option<&str>) -> Option<u8> {
    let hint = hint.filter(|h| !h.is_empty())?;
    let value = if hint.contains('.') {
        ((100.0 * crate::stdlib::string::strtod(hint).0 as f32) as i32).min(255)
    } else if hints::string_to_bool(Some(hint), true) {
        100
    } else {
        0
    };
    // (a negative value wraps, as upstream's Uint8)
    Some(value as u8)
}

/// Translation of `GetMaxWriteAttempts()`.
fn get_max_write_attempts(device: &HidapiDevice) -> i32 {
    if device.vendor_id() == USB_VENDOR_NINTENDO
        && device.product_id() == USB_PRODUCT_NINTENDO_SWITCH_JOYCON_GRIP
    {
        // This device is a little slow and we know we're always on USB
        20
    } else {
        5
    }
}

/// Translation of `AlwaysUsesLabels()`.
fn always_uses_labels(vendor_id: u16, product_id: u16, controller_type: ControllerType) -> bool {
    // Some controllers don't have a diamond button configuration, so should always use labels
    if is_joystick_gamecube(vendor_id, product_id) {
        return true;
    }
    matches!(
        controller_type,
        ControllerType::HVC_LEFT
            | ControllerType::HVC_RIGHT
            | ControllerType::NES_LEFT
            | ControllerType::NES_RIGHT
            | ControllerType::N64
            | ControllerType::SEGA_GENESIS
    )
}

/// The hat of a hat value (0 is up, clockwise).
fn hat_of(value: u8) -> u8 {
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

/// The hat of the full report's d-pad bits.
fn dpad_hat(data: u8) -> u8 {
    let mut hat = 0;
    if data & 0x01 != 0 {
        hat |= HAT_DOWN;
    }
    if data & 0x02 != 0 {
        hat |= HAT_UP;
    }
    if data & 0x04 != 0 {
        hat |= HAT_RIGHT;
    }
    if data & 0x08 != 0 {
        hat |= HAT_LEFT;
    }
    hat
}

/// A trigger axis of a digital trigger.
fn digital_trigger(pressed: bool) -> i16 {
    if pressed {
        i16::MAX
    } else {
        i16::MIN
    }
}

/// The X and Y axes of a stick used as a hat, for a hat value (`table`
/// has the 8 directions; other values are centered).
fn stick_hat_axes(table: &[(i16, i16); 8], value: u8) -> (i16, i16) {
    table.get(usize::from(value)).copied().unwrap_or((0, 0))
}

const AXIS_MAX: i16 = i16::MAX;
const AXIS_MIN: i16 = i16::MIN;

/// The stick of `HandleCombinedSimpleControllerStateL()`.
const COMBINED_SIMPLE_L_STICK: [(i16, i16); 8] = [
    (AXIS_MAX, 0),
    (AXIS_MAX, AXIS_MAX),
    (0, AXIS_MAX),
    (AXIS_MIN, AXIS_MAX),
    (AXIS_MIN, 0),
    (AXIS_MIN, AXIS_MIN),
    (0, AXIS_MIN),
    (AXIS_MAX, AXIS_MIN),
];

/// The stick of `HandleCombinedSimpleControllerStateR()`.
const COMBINED_SIMPLE_R_STICK: [(i16, i16); 8] = [
    (AXIS_MIN, 0),
    (AXIS_MIN, AXIS_MIN),
    (0, AXIS_MIN),
    (AXIS_MAX, AXIS_MIN),
    (AXIS_MAX, 0),
    (AXIS_MAX, AXIS_MAX),
    (0, AXIS_MAX),
    (AXIS_MIN, AXIS_MAX),
];

/// The stick of `HandleMiniSimpleControllerStateL()` and
/// `HandleMiniSimpleControllerStateR()`.
const MINI_SIMPLE_STICK: [(i16, i16); 8] = [
    (0, AXIS_MIN),
    (AXIS_MAX, AXIS_MIN),
    (AXIS_MAX, 0),
    (AXIS_MAX, AXIS_MAX),
    (0, AXIS_MAX),
    (AXIS_MIN, AXIS_MAX),
    (AXIS_MIN, 0),
    (AXIS_MIN, AXIS_MIN),
];

/// The two 12 bit axes of a full report's stick.
fn stick_axes(stick: &[u8]) -> (i16, i16) {
    (
        (i16::from(stick[0]) | ((i16::from(stick[1]) & 0xF) << 8)),
        (((i16::from(stick[1]) & 0xF0) >> 4) | (i16::from(stick[2]) << 4)),
    )
}

/// The `i16` at `at` of a packed packet.
fn le_i16(data: &[u8], at: usize) -> i16 {
    i16::from_le_bytes([data[at], data[at + 1]])
}

/// The stick calibration of the 9 packed calibration bytes of each stick
/// (part of `LoadStickCalibration()`).
fn stick_calibration(left: &[u8; 9], right: &[u8; 9]) -> [[StickAxisCalibration; 2]; 2] {
    // Stick calibration values are 12-bits each and are packed by bit
    // For whatever reason the fields are in a different order for each stick
    // Left:  X-Max, Y-Max, X-Center, Y-Center, X-Min, Y-Min
    // Right: X-Center, Y-Center, X-Min, Y-Min, X-Max, Y-Max
    let x = |cal: &[u8; 9], i: usize| ((i16::from(cal[i + 1]) << 8) & 0xF00) | i16::from(cal[i]);
    let y = |cal: &[u8; 9], i: usize| (i16::from(cal[i + 1]) << 4) | (i16::from(cal[i]) >> 4);

    let mut cal = [[StickAxisCalibration::default(); 2]; 2];

    // Left stick
    cal[0][0].max = x(left, 0); // X Axis max above center
    cal[0][1].max = y(left, 1); // Y Axis max above center
    cal[0][0].center = x(left, 3); // X Axis center
    cal[0][1].center = y(left, 4); // Y Axis center
    cal[0][0].min = x(left, 6); // X Axis min below center
    cal[0][1].min = y(left, 7); // Y Axis min below center

    // Right stick
    cal[1][0].center = x(right, 0); // X Axis center
    cal[1][1].center = y(right, 1); // Y Axis center
    cal[1][0].min = x(right, 3); // X Axis min below center
    cal[1][1].min = y(right, 4); // Y Axis min below center
    cal[1][0].max = x(right, 6); // X Axis max above center
    cal[1][1].max = y(right, 7); // Y Axis max above center

    // Filter out any values that were uninitialized (0xFFF) in the SPI read
    for axis in cal.iter_mut().flatten() {
        if axis.center == 0xFFF {
            axis.center = 2048;
        }
        if axis.max == 0xFFF {
            axis.max = (f32::from(axis.center) * 0.7) as i16;
        }
        if axis.min == 0xFFF {
            axis.min = (f32::from(axis.center) * 0.7) as i16;
        }
    }
    cal
}

/// The IMU scale of the factory and (if it's there) user calibration
/// SPI read data (part of `LoadIMUCalibration()`).
fn imu_scale_data(factory: &[u8], user: Option<&[u8]>) -> ImuScaleData {
    // IMU scale gives us multipliers for converting raw values to real world values
    let mut scale = factory;

    let mut accel_raw_x = le_i16(scale, 0);
    let mut accel_raw_y = le_i16(scale, 2);
    let mut accel_raw_z = le_i16(scale, 4);

    let accel_sens_coeff_x = le_i16(scale, 6);
    let accel_sens_coeff_y = le_i16(scale, 8);
    let accel_sens_coeff_z = le_i16(scale, 10);

    let mut gyro_raw_x = le_i16(scale, 12);
    let mut gyro_raw_y = le_i16(scale, 14);
    let mut gyro_raw_z = le_i16(scale, 16);

    let gyro_sens_coeff_x = le_i16(scale, 18);
    let gyro_sens_coeff_y = le_i16(scale, 20);
    let gyro_sens_coeff_z = le_i16(scale, 22);

    // Check for user calibration data. If it's present and set, it'll override the factory settings
    if let Some(user) = user {
        if (u16::from(user[0]) | (u16::from(user[1]) << 8)) == 0xA1B2 {
            scale = user;

            accel_raw_x = le_i16(scale, 2);
            accel_raw_y = le_i16(scale, 4);
            accel_raw_z = le_i16(scale, 6);

            gyro_raw_x = le_i16(scale, 14);
            gyro_raw_y = le_i16(scale, 16);
            gyro_raw_z = le_i16(scale, 18);
        }
    }

    let accel = |coeff: i16, raw: i16| {
        SWITCH_ACCEL_SCALE_MULT / (f32::from(coeff) - f32::from(raw)) * STANDARD_GRAVITY
    };
    let gyro = |coeff: i16, raw: i16| {
        SWITCH_GYRO_SCALE_MULT / (f32::from(coeff) - f32::from(raw)) * std::f32::consts::PI / 180.0
    };
    ImuScaleData {
        // Gyro zero-rate offset
        gyro_offset_x: gyro_raw_x,
        gyro_offset_y: gyro_raw_y,
        gyro_offset_z: gyro_raw_z,

        // Accelerometer scale
        accel_scale_x: accel(accel_sens_coeff_x, accel_raw_x),
        accel_scale_y: accel(accel_sens_coeff_y, accel_raw_y),
        accel_scale_z: accel(accel_sens_coeff_z, accel_raw_z),

        // Gyro scale
        gyro_scale_x: gyro(gyro_sens_coeff_x, gyro_raw_x),
        gyro_scale_y: gyro(gyro_sens_coeff_y, gyro_raw_y),
        gyro_scale_z: gyro(gyro_sens_coeff_z, gyro_raw_z),
    }
}

/// The default IMU scale of `LoadIMUCalibration()`.
fn default_imu_scale_data() -> ImuScaleData {
    let accel_scale = STANDARD_GRAVITY / SWITCH_ACCEL_SCALE;
    let gyro_scale = std::f32::consts::PI / 180.0 / SWITCH_GYRO_SCALE;

    ImuScaleData {
        accel_scale_x: accel_scale,
        accel_scale_y: accel_scale,
        accel_scale_z: accel_scale,
        gyro_scale_x: gyro_scale,
        gyro_scale_y: gyro_scale,
        gyro_scale_z: gyro_scale,
        gyro_offset_x: 0,
        gyro_offset_y: 0,
        gyro_offset_z: 0,
    }
}

/// The value of a remapped axis (`(Sint16)HIDAPI_RemapVal(...)`).
// FIXME (upstream): a stick extent of 0 makes the remapped value NaN or
// infinite, which is undefined behaviour to convert; here it saturates
// (NaN is 0).
fn remap_axis(value: i16, value_min: i16, value_max: i16, output_min: i16, output_max: i16) -> i16 {
    remap_val(
        f32::from(value),
        f32::from(value_min),
        f32::from(value_max),
        f32::from(output_min),
        f32::from(output_max),
    ) as i16
}

/// The lesser of two power states, as upstream compares their values.
fn min_power_state(a: PowerState, b: PowerState) -> PowerState {
    if (a as i32) <= (b as i32) {
        a
    } else {
        b
    }
}

impl SwitchContext {
    /// Translation of `ReadInput()`.
    fn read_input(&mut self, link: Link<'_>) -> Result<usize> {
        // Make sure we don't try to read at the same time a write is happening
        if link.device().rumble_pending.load(Ordering::Acquire) > 0 {
            return Ok(0);
        }

        let result = link
            .device()
            .dev()
            .ok_or_else(|| Error::invalid_param("device"))?
            .read_timeout_ms(&mut self.read_buffer, 0);

        // See if we can guess the initial input mode
        if matches!(result, Ok(n) if n > 0) && !self.input_only && self.initial_input_mode == 0 {
            match self.read_buffer[0] {
                INPUT_REPORT_FULL_CONTROLLER_STATE
                | INPUT_REPORT_FULL_CONTROLLER_AND_MCU_STATE
                | INPUT_REPORT_SIMPLE_CONTROLLER_STATE => {
                    self.initial_input_mode = self.read_buffer[0]
                }
                _ => {}
            }
        }
        result
    }

    /// Translation of `WriteOutput()`: use the rumble thread for general
    /// asynchronous writes.
    fn write_output(&self, link: Link<'_>, data: &[u8]) -> Result<usize> {
        match link {
            Link::Device(device) => lock_rumble()?.send_and_unlock(device, data),
            // (a probed device is only written to directly)
            Link::Probe(_) => link.write(data),
        }
    }

    /// Translation of `ReadSubcommandReply()`: whether the reply is in
    /// the read buffer.
    fn read_subcommand_reply(&mut self, link: Link<'_>, expected_id: u8, buf: &[u8]) -> bool {
        // Average response time for messages is ~30ms
        let end_ticks = crate::timer::ticks_ms() + 100;

        while let Ok(read) = self.read_input(link) {
            if read > 0 {
                if self.read_buffer[0] == INPUT_REPORT_SUBCOMMAND_REPLY {
                    let reply = &self.read_buffer[1..];
                    if reply[REPLY_SUBCOMMAND_ID] != expected_id
                        || (reply[REPLY_SUBCOMMAND_ACK] & 0x80) == 0
                    {
                        continue;
                    }
                    if reply[REPLY_SUBCOMMAND_ID] == SUBCOMMAND_SPI_FLASH_READ
                        && reply[REPLY_SUBCOMMAND_DATA..REPLY_SUBCOMMAND_DATA + buf.len()] != *buf
                    {
                        // This was a reply for another SPI read command
                        continue;
                    }
                    return true;
                }
            } else {
                crate::timer::delay(Duration::from_millis(1));
            }

            if crate::timer::ticks_ms() >= end_ticks {
                break;
            }
        }
        false
    }

    /// The reply in the read buffer (`SwitchSubcommandInputPacket_t`).
    fn reply(&self) -> &[u8] {
        &self.read_buffer[1..]
    }

    /// Translation of `ReadProprietaryReply()`.
    fn read_proprietary_reply(&mut self, link: Link<'_>, expected_id: u8) -> bool {
        // Average response time for messages is ~30ms
        let end_ticks = crate::timer::ticks_ms() + 100;

        while let Ok(read) = self.read_input(link) {
            if read > 0 {
                if self.read_buffer[0] == INPUT_REPORT_COMMAND_ACK
                    && self.read_buffer[1] == expected_id
                {
                    return true;
                }
            } else {
                crate::timer::delay(Duration::from_millis(1));
            }

            if crate::timer::ticks_ms() >= end_ticks {
                break;
            }
        }
        false
    }

    /// Translation of `ConstructSubcommand()`.
    fn construct_subcommand(
        &mut self,
        command_id: u8,
        buf: &[u8],
    ) -> [u8; SWITCH_OUTPUT_PACKET_DATA_LENGTH] {
        let mut packet = [0u8; SWITCH_OUTPUT_PACKET_DATA_LENGTH];

        packet[0] = OUTPUT_REPORT_RUMBLE_AND_SUBCOMMAND;
        packet[1] = self.command_number;

        packet[2..6].copy_from_slice(&self.rumble_packet.rumble_data[0]);
        packet[6..10].copy_from_slice(&self.rumble_packet.rumble_data[1]);

        packet[COMMON_OUTPUT_PACKET_SIZE] = command_id;
        packet[COMMON_OUTPUT_PACKET_SIZE + 1..COMMON_OUTPUT_PACKET_SIZE + 1 + buf.len()]
            .copy_from_slice(buf);

        self.command_number = (self.command_number + 1) & 0xF;
        packet
    }

    /// Translation of `WritePacket()`.
    fn write_packet(&self, link: Link<'_>, data: &[u8]) -> bool {
        let write_size = if link.device().is_bluetooth() {
            SWITCH_BLUETOOTH_PACKET_LENGTH
        } else {
            SWITCH_USB_PACKET_LENGTH
        };

        if data.len() > SWITCH_OUTPUT_PACKET_DATA_LENGTH {
            return false;
        }

        // (padded to the write size)
        let mut buf = [0u8; SWITCH_MAX_OUTPUT_PACKET_LENGTH];
        buf[..data.len()].copy_from_slice(data);
        let packet = &buf[..write_size.max(data.len())];
        if self.sync_write {
            link.write(packet).is_ok()
        } else {
            self.write_output(link, packet).is_ok()
        }
    }

    /// Translation of `WriteSubcommand()`: whether a reply came, which is
    /// then in the read buffer.
    fn write_subcommand(&mut self, link: Link<'_>, command_id: u8, buf: &[u8]) -> bool {
        if buf.len() > SUBCOMMAND_DATA_SIZE {
            // FIXME (upstream): subcommand data longer than the packet's
            // (from SDL_SendJoystickEffect()) is copied past the packet;
            // here it isn't sent.
            return false;
        }

        for _ in 0..self.max_write_attempts.max(0) {
            let command_packet = self.construct_subcommand(command_id, buf);

            if !self.write_packet(link, &command_packet) {
                continue;
            }

            if self.read_subcommand_reply(link, command_id, buf) {
                return true;
            }
        }
        false
    }

    /// Translation of `WriteProprietary()`.
    fn write_proprietary(
        &mut self,
        link: Link<'_>,
        command: u8,
        buf: &[u8],
        wait_for_reply: bool,
    ) -> bool {
        for _ in 0..self.max_write_attempts.max(0) {
            if buf.len() > PROPRIETARY_DATA_SIZE {
                return false;
            }

            let mut packet = [0u8; SWITCH_OUTPUT_PACKET_DATA_LENGTH];
            packet[0] = OUTPUT_REPORT_PROPRIETARY;
            packet[1] = command;
            packet[2..2 + buf.len()].copy_from_slice(buf);

            if !self.write_packet(link, &packet) {
                continue;
            }

            if !wait_for_reply || self.read_proprietary_reply(link, command) {
                return true;
            }
        }
        false
    }

    /// Translation of `WriteRumble()`.
    fn write_rumble(&mut self, link: Link<'_>) -> bool {
        // Write into m_RumblePacket rather than a temporary buffer to allow the current rumble state
        // to be retained for subsequent rumble or subcommand packets sent to the controller
        self.rumble_packet.packet_type = OUTPUT_REPORT_RUMBLE;
        self.rumble_packet.packet_number = self.command_number;
        self.command_number = (self.command_number + 1) & 0xF;

        // Refresh the rumble state periodically
        self.rumble_sent = crate::timer::ticks_ms();

        self.write_packet(link, &self.rumble_packet.to_bytes())
    }

    /// Translation of `BReadDeviceInfo()`.
    fn read_device_info(&mut self, link: Link<'_>) -> bool {
        if link.device().is_bluetooth() {
            if self.write_subcommand(link, SUBCOMMAND_REQUEST_DEVICE_INFO, &[]) {
                let reply = self.reply();
                // Byte 2: Controller ID (1=LJC, 2=RJC, 3=Pro)
                let controller_type = ControllerType(reply[REPLY_DEVICE_TYPE]);
                let mut mac_address = [0u8; 6];
                // Bytes 4-9: MAC address (big-endian)
                mac_address.copy_from_slice(&reply[REPLY_MAC_ADDRESS..REPLY_MAC_ADDRESS + 6]);

                self.controller_type = calculate_controller_type(link.device(), controller_type);
                self.mac_address = mac_address;
                return true;
            }
        } else if self.write_proprietary(link, PROPRIETARY_STATUS, &[], true) {
            // (SwitchProprietaryStatusPacket_t)
            let status = &self.read_buffer;
            let controller_type = ControllerType(status[3]);
            let mut mac_address = [0u8; 6];
            for (i, byte) in mac_address.iter_mut().enumerate() {
                *byte = status[4 + 6 - i - 1];
            }

            self.controller_type = calculate_controller_type(link.device(), controller_type);
            self.mac_address = mac_address;
            return true;
        }
        false
    }

    /// Translation of `BTrySetupUSB()`.
    fn try_setup_usb(&mut self, link: Link<'_>) -> bool {
        // We have to send a connection handshake to the controller when communicating over USB
        // before we're able to send it other commands. Luckily this command is not supported
        // over Bluetooth, so we can use the controller's lack of response as a way to
        // determine if the connection is over USB or Bluetooth
        if !self.write_proprietary(link, PROPRIETARY_HANDSHAKE, &[], true) {
            return false;
        }
        // The 8BitDo M30 and SF30 Pro don't respond to this command, but otherwise work correctly
        let _ = self.write_proprietary(link, PROPRIETARY_HIGH_SPEED, &[], true);
        // This fails on the right Joy-Con when plugged into the charging grip
        let _ = self.write_proprietary(link, PROPRIETARY_HANDSHAKE, &[], true);
        self.write_proprietary(link, PROPRIETARY_FORCE_USB, &[], false)
    }

    /// Translation of `SetVibrationEnabled()`.
    fn set_vibration_enabled(&mut self, link: Link<'_>, enabled: u8) -> bool {
        self.write_subcommand(link, SUBCOMMAND_ENABLE_VIBRATION, &[enabled])
    }

    /// Translation of `SetInputMode()`.
    fn set_input_mode(&mut self, link: Link<'_>, input_mode: u8) -> bool {
        if input_mode == self.current_input_mode {
            true
        } else {
            self.current_input_mode = input_mode;

            self.write_subcommand(link, SUBCOMMAND_SET_INPUT_REPORT_MODE, &[input_mode])
        }
    }

    /// Translation of `SetHomeLED()`.
    fn set_home_led(&mut self, link: Link<'_>, brightness: u8) -> bool {
        let led_intensity = home_led_intensity(brightness);

        let buffer = [
            0x1,                        // 0 mini cycles (besides first), cycle duration 8ms
            (led_intensity & 0xF) << 4, // LED start intensity (0x0-0xF), 0 cycles (LED stays on at start intensity after first cycle)
            (led_intensity & 0xF) << 4, // First cycle LED intensity, 0x0 intensity for second cycle
            0x0, // 8ms fade transition to first cycle, 8ms first cycle LED duration
        ];

        self.write_subcommand(link, SUBCOMMAND_SET_HOME_LIGHT, &buffer)
    }

    /// Translation of `SDL_HomeLEDHintChanged()`.
    fn home_led_hint_changed(&mut self, link: Link<'_>, hint: Option<&str>) {
        if let Some(value) = home_led_hint_value(hint) {
            self.set_home_led(link, value);
        }
    }

    /// Translation of `UpdateSlotLED()`.
    fn update_slot_led(&mut self, link: Link<'_>) {
        if !self.input_only {
            const PLAYER_PATTERN: [u8; 8] = [0x1, 0x3, 0x7, 0xf, 0x9, 0x5, 0xd, 0x6];
            let mut led_data = 0;

            if self.player_lights && self.player_index >= 0 {
                led_data = PLAYER_PATTERN[self.player_index as usize % 8];
            }
            self.write_subcommand(link, SUBCOMMAND_SET_PLAYER_LIGHTS, &[led_data]);
        }
    }

    /// Translation of `SDL_PlayerLEDHintChanged()`.
    fn player_led_hint_changed(
        &mut self,
        device: &mut DeviceCtx<'_>,
        link: Link<'_>,
        hint: Option<&str>,
    ) {
        let player_lights = hints::string_to_bool(hint, true);

        if player_lights != self.player_lights {
            self.player_lights = player_lights;

            self.update_slot_led(link);
            device.update_device_properties();
        }
    }

    /// Translation of `GetInitialInputMode()`.
    fn get_initial_input_mode(&mut self, link: Link<'_>) {
        if self.initial_input_mode == 0 {
            // This will set the initial input mode if it can
            let _ = self.read_input(link);
        }
    }

    /// Translation of `GetDefaultInputMode()`.
    fn get_default_input_mode(&self, device: &HidapiDevice) -> u8 {
        // Determine the desired input mode
        let mut input_mode = if self.initial_input_mode != 0 {
            self.initial_input_mode
        } else if device.is_bluetooth() {
            INPUT_REPORT_SIMPLE_CONTROLLER_STATE
        } else {
            INPUT_REPORT_FULL_CONTROLLER_STATE
        };

        match self.enhanced_report_hint {
            EnhancedReportHint::Off => input_mode = INPUT_REPORT_SIMPLE_CONTROLLER_STATE,
            EnhancedReportHint::On => {
                if input_mode == INPUT_REPORT_SIMPLE_CONTROLLER_STATE {
                    input_mode = INPUT_REPORT_FULL_CONTROLLER_STATE;
                }
            }
            EnhancedReportHint::Auto => {
                // Joy-Con controllers switch their thumbsticks into D-pad mode in simple mode,
                // so let's enable full controller state for them.
                if device.vendor_id() == USB_VENDOR_NINTENDO
                    && (device.product_id() == USB_PRODUCT_NINTENDO_SWITCH_JOYCON_LEFT
                        || device.product_id() == USB_PRODUCT_NINTENDO_SWITCH_JOYCON_RIGHT)
                {
                    input_mode = INPUT_REPORT_FULL_CONTROLLER_STATE;
                }
            }
        }

        // Wired controllers break if they are put into simple controller state
        if input_mode == INPUT_REPORT_SIMPLE_CONTROLLER_STATE && !device.is_bluetooth() {
            input_mode = INPUT_REPORT_FULL_CONTROLLER_STATE;
        }
        input_mode
    }

    /// Translation of `GetSensorInputMode()`.
    fn get_sensor_input_mode(&self) -> u8 {
        // Determine the desired input mode
        if self.initial_input_mode == 0
            || self.initial_input_mode == INPUT_REPORT_SIMPLE_CONTROLLER_STATE
        {
            INPUT_REPORT_FULL_CONTROLLER_STATE
        } else {
            self.initial_input_mode
        }
    }

    /// Translation of `UpdateInputMode()`.
    fn update_input_mode(&mut self, link: Link<'_>) {
        let input_mode = if self.report_sensors {
            self.get_sensor_input_mode()
        } else {
            self.get_default_input_mode(link.device())
        };
        self.set_input_mode(link, input_mode);
    }

    /// Translation of `SetEnhancedModeAvailable()`.
    fn set_enhanced_mode_available(
        &mut self,
        device: &HidapiDevice,
        joystick: &mut JoystickRef<'_>,
    ) {
        if self.enhanced_mode_available {
            return;
        }
        self.enhanced_mode_available = true;

        if self.sensors_supported {
            let has_parent = device.parent().is_some();
            let controller_type = self.controller_type;
            joystick.with(|joystick| {
                // Use the right sensor in the combined Joy-Con pair
                if !has_parent || controller_type == ControllerType::JOYCON_RIGHT {
                    joystick.add_sensor(SensorType::Gyro, 200.0);
                    joystick.add_sensor(SensorType::Accel, 200.0);
                }
                if has_parent && controller_type == ControllerType::JOYCON_LEFT {
                    joystick.add_sensor(SensorType::GyroL, 200.0);
                    joystick.add_sensor(SensorType::AccelL, 200.0);
                }
                if has_parent && controller_type == ControllerType::JOYCON_RIGHT {
                    joystick.add_sensor(SensorType::GyroR, 200.0);
                    joystick.add_sensor(SensorType::AccelR, 200.0);
                }
            });
        }
    }

    /// Translation of `SetEnhancedReportHint()`.
    fn set_enhanced_report_hint(
        &mut self,
        link: Link<'_>,
        joystick: &mut JoystickRef<'_>,
        enhanced_report_hint: EnhancedReportHint,
    ) {
        self.enhanced_report_hint = enhanced_report_hint;

        match enhanced_report_hint {
            EnhancedReportHint::Off => self.enhanced_mode = false,
            EnhancedReportHint::On => {
                self.set_enhanced_mode_available(link.device(), joystick);
                self.enhanced_mode = true;
            }
            EnhancedReportHint::Auto => self.set_enhanced_mode_available(link.device(), joystick),
        }

        self.update_input_mode(link);
    }

    /// Translation of `UpdateEnhancedModeOnEnhancedReport()`.
    fn update_enhanced_mode_on_enhanced_report(
        &mut self,
        link: Link<'_>,
        joystick: &mut JoystickRef<'_>,
    ) {
        if self.enhanced_report_hint == EnhancedReportHint::Auto {
            self.set_enhanced_report_hint(link, joystick, EnhancedReportHint::On);
        }
    }

    /// Translation of `UpdateEnhancedModeOnApplicationUsage()`.
    fn update_enhanced_mode_on_application_usage(
        &mut self,
        link: Link<'_>,
        joystick: &mut JoystickRef<'_>,
    ) {
        if self.enhanced_report_hint == EnhancedReportHint::Auto {
            self.set_enhanced_report_hint(link, joystick, EnhancedReportHint::On);
        }
    }

    /// Translation of `SDL_EnhancedReportsChanged()`.
    fn enhanced_reports_changed(
        &mut self,
        link: Link<'_>,
        joystick: &mut JoystickRef<'_>,
        hint: Option<&str>,
    ) {
        let mode = if hint.is_some_and(|h| h.eq_ignore_ascii_case("auto")) {
            EnhancedReportHint::Auto
        } else if hints::string_to_bool(hint, true) {
            EnhancedReportHint::On
        } else {
            EnhancedReportHint::Off
        };
        self.set_enhanced_report_hint(link, joystick, mode);
    }

    /// Apply the hint changes recorded since the last call (upstream's
    /// hint callbacks).
    fn hint_changes(
        &mut self,
        device: &mut DeviceCtx<'_>,
        link: Link<'_>,
        joystick: &mut JoystickRef<'_>,
    ) {
        if let Some(hint) = self
            .enhanced_reports_hint
            .as_ref()
            .and_then(HintWatch::take)
        {
            self.enhanced_reports_changed(link, joystick, hint.as_deref());
        }
        if let Some(hint) = self.home_led_hint.as_ref().and_then(HintWatch::take) {
            self.home_led_hint_changed(link, hint.as_deref());
        }
        if let Some(hint) = self.player_led_hint.as_ref().and_then(HintWatch::take) {
            self.player_led_hint_changed(device, link, hint.as_deref());
        }
    }

    /// Translation of `SetIMUEnabled()`.
    fn set_imu_enabled(&mut self, link: Link<'_>, enabled: bool) -> bool {
        self.write_subcommand(link, SUBCOMMAND_ENABLE_IMU, &[u8::from(enabled)])
    }

    /// The 9 calibration bytes at `at` of the SPI read data in the reply.
    fn reply_calibration(&self, at: usize) -> [u8; 9] {
        let mut cal = [0u8; 9];
        cal.copy_from_slice(
            &self.reply()[REPLY_SUBCOMMAND_DATA + at..REPLY_SUBCOMMAND_DATA + at + 9],
        );
        cal
    }

    /// Translation of `LoadStickCalibration()`.
    fn load_stick_calibration(&mut self, link: Link<'_>) -> bool {
        let mut left_stick_cal = None;
        let mut right_stick_cal = None;

        // Read User Calibration Info
        let read_user_params = spi_op_data(
            SPI_STICK_USER_CALIBRATION_START_OFFSET,
            SPI_STICK_USER_CALIBRATION_LENGTH,
        );

        // This isn't readable on all controllers, so ignore failure
        if self.write_subcommand(link, SUBCOMMAND_SPI_FLASH_READ, &read_user_params) {
            // Automatically select the user calibration if magic bytes are set
            // (stickUserCalibration: opData, left magic, left calibration,
            // right magic, right calibration)
            let data = &self.reply()[REPLY_SUBCOMMAND_DATA..];
            if data[5] == 0xB2 && data[6] == 0xA1 {
                left_stick_cal = Some(self.reply_calibration(7));
            }
            let data = &self.reply()[REPLY_SUBCOMMAND_DATA..];
            if data[16] == 0xB2 && data[17] == 0xA1 {
                right_stick_cal = Some(self.reply_calibration(18));
            }
        }

        // Only read the factory calibration info if we failed to receive the correct magic bytes
        if left_stick_cal.is_none() || right_stick_cal.is_none() {
            // Read Factory Calibration Info
            let read_factory_params = spi_op_data(
                SPI_STICK_FACTORY_CALIBRATION_START_OFFSET,
                SPI_STICK_FACTORY_CALIBRATION_LENGTH,
            );

            const MAX_ATTEMPTS: i32 = 3;
            let mut attempt = 0;
            loop {
                if !self.write_subcommand(link, SUBCOMMAND_SPI_FLASH_READ, &read_factory_params) {
                    return false;
                }

                let data = &self.reply()[REPLY_SUBCOMMAND_DATA..];
                let address = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
                if address == SPI_STICK_FACTORY_CALIBRATION_START_OFFSET {
                    // We successfully read the calibration data
                    // Avoid overriding user calibration if it's present for only one stick (known to happen on JoyCons)
                    // (stickFactoryCalibration: opData, left, right)
                    if left_stick_cal.is_none() {
                        left_stick_cal = Some(self.reply_calibration(5));
                    }
                    if right_stick_cal.is_none() {
                        right_stick_cal = Some(self.reply_calibration(14));
                    }
                    break;
                }

                if attempt == MAX_ATTEMPTS {
                    return false;
                }
                attempt += 1;
            }
        }

        // If we still don't have calibration data, return false
        let (Some(left), Some(right)) = (left_stick_cal, right_stick_cal) else {
            return false;
        };
        self.set_stick_calibration(&left, &right);
        true
    }

    /// The stick calibration and extents of the calibration bytes (part
    /// of `LoadStickCalibration()`).
    fn set_stick_calibration(&mut self, left: &[u8; 9], right: &[u8; 9]) {
        self.stick_cal_data = stick_calibration(left, right);

        for (extents, cal) in self
            .stick_extents
            .iter_mut()
            .flatten()
            .zip(self.stick_cal_data.iter().flatten())
        {
            extents.min = -((f32::from(cal.min) * 0.7) as i16);
            extents.max = (f32::from(cal.max) * 0.7) as i16;
        }

        for extents in self.simple_stick_extents.iter_mut().flatten() {
            extents.min = (f32::from(i16::MIN) * 0.5) as i16;
            extents.max = (f32::from(i16::MAX) * 0.5) as i16;
        }
    }

    /// The SPI read data of the reply.
    fn reply_spi_read_data(&self) -> [u8; SPI_READ_DATA_SIZE] {
        let mut data = [0u8; SPI_READ_DATA_SIZE];
        data.copy_from_slice(
            &self.reply()[REPLY_SPI_READ_DATA..REPLY_SPI_READ_DATA + SPI_READ_DATA_SIZE],
        );
        data
    }

    /// Translation of `LoadIMUCalibration()`.
    fn load_imu_calibration(&mut self, link: Link<'_>) -> bool {
        // Read Calibration Info
        let read_params = spi_op_data(SPI_IMU_SCALE_START_OFFSET, SPI_IMU_SCALE_LENGTH);

        if self.write_subcommand(link, SUBCOMMAND_SPI_FLASH_READ, &read_params) {
            let factory = self.reply_spi_read_data();

            // Check for user calibration data. If it's present and set, it'll override the factory settings
            let read_params =
                spi_op_data(SPI_IMU_USER_SCALE_START_OFFSET, SPI_IMU_USER_SCALE_LENGTH);
            let user = self
                .write_subcommand(link, SUBCOMMAND_SPI_FLASH_READ, &read_params)
                .then(|| self.reply_spi_read_data());

            self.imu_scale_data = imu_scale_data(&factory, user.as_ref().map(|u| &u[..]));
        } else {
            // Use default values
            self.imu_scale_data = default_imu_scale_data();
        }
        true
    }

    /// Translation of `ApplyStickCalibration()`.
    fn apply_stick_calibration(&mut self, stick: usize, axis: usize, raw_value: i16) -> i16 {
        let raw_value = raw_value.wrapping_sub(self.stick_cal_data[stick][axis].center);
        let extents = &mut self.stick_extents[stick][axis];

        if raw_value >= 0 {
            if raw_value > extents.max {
                extents.max = raw_value;
            }
            remap_axis(raw_value, 0, extents.max, 0, i16::MAX)
        } else {
            if raw_value < extents.min {
                extents.min = raw_value;
            }
            remap_axis(raw_value, extents.min, 0, i16::MIN, 0)
        }
    }

    /// Translation of `ApplySimpleStickCalibration()`.
    fn apply_simple_stick_calibration(&mut self, stick: usize, axis: usize, raw_value: i16) -> i16 {
        // 0x8000 is the neutral value for all joystick axes
        const JOYSTICK_CENTER: u16 = 0x8000;

        let raw_value = raw_value.wrapping_sub(JOYSTICK_CENTER as i16);
        let extents = &mut self.simple_stick_extents[stick][axis];

        if raw_value >= 0 {
            if raw_value > extents.max {
                extents.max = raw_value;
            }
            remap_axis(raw_value, 0, extents.max, 0, i16::MAX)
        } else {
            if raw_value < extents.min {
                extents.min = raw_value;
            }
            remap_axis(raw_value, extents.min, 0, i16::MIN, 0)
        }
    }

    /// Translation of `RemapButton()`.
    fn remap_button(&self, button: GamepadButton) -> u8 {
        if self.use_button_labels {
            // Use button labels instead of positions, e.g. Nintendo Online Classic controllers
            let remapped = match button {
                GamepadButton::South => GamepadButton::East,
                GamepadButton::East => GamepadButton::South,
                GamepadButton::West => GamepadButton::North,
                GamepadButton::North => GamepadButton::West,
                other => other,
            };
            return remapped as u8;
        }
        button as u8
    }

    /// Translation of `HasHomeLED()`.
    fn has_home_led(&self, vendor_id: u16, product_id: u16) -> bool {
        // The Power A Nintendo Switch Pro controllers don't have a Home LED
        if vendor_id == 0 && product_id == 0 {
            return false;
        }

        // HORI Wireless Switch Pad
        if vendor_id == 0x0f0d && product_id == 0x00f6 {
            return false;
        }

        // Third party controllers don't have a home LED and will shut off if we try to set it
        if self.controller_type == ControllerType::UNKNOWN
            || self.controller_type == ControllerType::LIC_PRO_CONTROLLER
        {
            return false;
        }

        // The Nintendo Online classic controllers don't have a Home LED
        if vendor_id == USB_VENDOR_NINTENDO && self.controller_type > ControllerType::PRO_CONTROLLER
        {
            return false;
        }

        true
    }

    /// Translation of `UpdateDeviceIdentity()`.
    fn update_device_identity(&mut self, device: &mut DeviceCtx<'_>) {
        if self.input_only {
            if is_joystick_gamecube(device.vendor_id(), device.product_id()) {
                device.set_gamepad_type(GamepadType::Gamecube);
            }
            return;
        }

        match self.controller_type {
            ControllerType::JOYCON_LEFT => {
                device.set_device_name("Nintendo Switch Joy-Con (L)");
                device.set_device_product(
                    USB_VENDOR_NINTENDO,
                    USB_PRODUCT_NINTENDO_SWITCH_JOYCON_LEFT,
                );
                device.set_gamepad_type(GamepadType::NintendoSwitchJoyconLeft);
            }
            ControllerType::JOYCON_RIGHT => {
                device.set_device_name("Nintendo Switch Joy-Con (R)");
                device.set_device_product(
                    USB_VENDOR_NINTENDO,
                    USB_PRODUCT_NINTENDO_SWITCH_JOYCON_RIGHT,
                );
                device.set_gamepad_type(GamepadType::NintendoSwitchJoyconRight);
            }
            ControllerType::PRO_CONTROLLER | ControllerType::LIC_PRO_CONTROLLER => {
                device.set_device_name("Nintendo Switch Pro Controller");
                device.set_device_product(USB_VENDOR_NINTENDO, USB_PRODUCT_NINTENDO_SWITCH_PRO);
                device.set_gamepad_type(GamepadType::NintendoSwitchPro);
            }
            ControllerType::HVC_LEFT => {
                device.set_device_name("Nintendo Family Computer Controller (1)");
                device.set_gamepad_type(GamepadType::Standard);
            }
            ControllerType::HVC_RIGHT => {
                device.set_device_name("Nintendo Family Computer Controller (2)");
                device.set_gamepad_type(GamepadType::Standard);
            }
            ControllerType::NES_LEFT => {
                device.set_device_name("Nintendo NES Controller (L)");
                device.set_gamepad_type(GamepadType::Standard);
            }
            ControllerType::NES_RIGHT => {
                device.set_device_name("Nintendo NES Controller (R)");
                device.set_gamepad_type(GamepadType::Standard);
            }
            ControllerType::SNES => {
                device.set_device_name("Nintendo SNES Controller");
                device
                    .set_device_product(USB_VENDOR_NINTENDO, USB_PRODUCT_NINTENDO_SNES_CONTROLLER);
                device.set_gamepad_type(GamepadType::Standard);
            }
            ControllerType::N64 => {
                device.set_device_name("Nintendo N64 Controller");
                device.set_device_product(USB_VENDOR_NINTENDO, USB_PRODUCT_NINTENDO_N64_CONTROLLER);
                device.set_gamepad_type(GamepadType::Standard);
            }
            ControllerType::SEGA_GENESIS => {
                device.set_device_name("Nintendo SEGA Genesis Controller");
                device.set_device_product(
                    USB_VENDOR_NINTENDO,
                    USB_PRODUCT_NINTENDO_SEGA_GENESIS_CONTROLLER,
                );
                device.set_gamepad_type(GamepadType::Standard);
            }
            ControllerType::UNKNOWN => {
                // We couldn't read the device info for this controller, might not be fully compliant
                if device.vendor_id() == USB_VENDOR_NINTENDO {
                    match device.product_id() {
                        USB_PRODUCT_NINTENDO_SWITCH_JOYCON_LEFT => {
                            self.controller_type = ControllerType::JOYCON_LEFT;
                            device.set_device_name("Nintendo Switch Joy-Con (L)");
                            device.set_gamepad_type(GamepadType::NintendoSwitchJoyconLeft);
                        }
                        USB_PRODUCT_NINTENDO_SWITCH_JOYCON_RIGHT => {
                            self.controller_type = ControllerType::JOYCON_RIGHT;
                            device.set_device_name("Nintendo Switch Joy-Con (R)");
                            device.set_gamepad_type(GamepadType::NintendoSwitchJoyconRight);
                        }
                        USB_PRODUCT_NINTENDO_SWITCH_PRO => {
                            self.controller_type = ControllerType::PRO_CONTROLLER;
                            device.set_device_name("Nintendo Switch Pro Controller");
                            device.set_gamepad_type(GamepadType::NintendoSwitchPro);
                        }
                        _ => {}
                    }
                }
                return;
            }
            _ => device.set_gamepad_type(GamepadType::Standard),
        }
        device.set_guid_byte(15, self.controller_type.0);

        let mac = self.mac_address;
        device.set_device_serial(&format!(
            "{:02x}-{:02x}-{:02x}-{:02x}-{:02x}-{:02x}",
            mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
        ));
    }

    /// Translation of `HIDAPI_DriverSwitch_ActuallyRumbleJoystick()`.
    fn actually_rumble_joystick(
        &mut self,
        link: Link<'_>,
        low_frequency_rumble: u16,
        high_frequency_rumble: u16,
    ) -> Result<()> {
        // Experimentally determined rumble values. These will only matter on some controllers as tested ones
        // seem to disregard these and just use any non-zero rumble values as a binary flag for constant rumble
        //
        // More information about these values can be found here:
        // https://github.com/dekuNukem/Nintendo_Switch_Reverse_Engineering/blob/master/rumble_data_table.md
        const HIGH_FREQ: u16 = 0x0074;
        let high_freq_amp = encode_rumble_high_amplitude(high_frequency_rumble);
        const LOW_FREQ: u8 = 0x3D;
        let low_freq_amp = encode_rumble_low_amplitude(low_frequency_rumble);

        let (vendor_id, product_id) = (link.device().vendor_id(), link.device().product_id());
        let rumble = if low_frequency_rumble != 0 || high_frequency_rumble != 0 {
            encode_rumble(
                vendor_id,
                product_id,
                HIGH_FREQ,
                high_freq_amp,
                LOW_FREQ,
                low_freq_amp,
            )
        } else {
            neutral_rumble(vendor_id, product_id)
        };
        self.rumble_packet.rumble_data = [rumble, rumble];

        self.rumble_active = low_frequency_rumble != 0 || high_frequency_rumble != 0;

        if !self.write_rumble(link) {
            return Err(Error::new("Couldn't send rumble packet"));
        }
        Ok(())
    }

    /// Translation of `HIDAPI_DriverSwitch_SendPendingRumble()`.
    fn send_pending_rumble(&mut self, link: Link<'_>) -> Result<()> {
        if crate::timer::ticks_ms() < self.rumble_sent + RUMBLE_WRITE_FREQUENCY_MS {
            return Ok(());
        }

        if self.rumble_pending {
            let low_frequency_rumble = (self.rumble_pending_value >> 16) as u16;
            let high_frequency_rumble = self.rumble_pending_value as u16;

            self.rumble_pending = false;
            self.rumble_pending_value = 0;

            return self.actually_rumble_joystick(
                link,
                low_frequency_rumble,
                high_frequency_rumble,
            );
        }

        if self.rumble_zero_pending {
            self.rumble_zero_pending = false;

            return self.actually_rumble_joystick(link, 0, 0);
        }

        Ok(())
    }

    /// Translation of `HandleInputOnlyControllerState()`.
    fn handle_input_only_controller_state(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: JoystickID,
        packet: &[u8],
    ) {
        use GamepadButton as B;
        let timestamp = crate::timer::ticks_ns();
        let packet = &packet[..INPUT_ONLY_STATE_SIZE];
        let last = self.last_input_only_state;
        let (buttons, stick_hat, left, right) =
            (&packet[0..2], packet[2], &packet[3..5], &packet[5..7]);

        if buttons[0] != last[0] {
            let data = buttons[0];
            device.send_button(
                timestamp,
                joystick,
                self.remap_button(B::South),
                (data & 0x02) != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                self.remap_button(B::East),
                (data & 0x04) != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                self.remap_button(B::West),
                (data & 0x01) != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                self.remap_button(B::North),
                (data & 0x08) != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                B::LeftShoulder as u8,
                (data & 0x10) != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                B::RightShoulder as u8,
                (data & 0x20) != 0,
            );
        }

        if buttons[1] != last[1] {
            let data = buttons[1];
            device.send_button(timestamp, joystick, B::Back as u8, (data & 0x01) != 0);
            device.send_button(timestamp, joystick, B::Start as u8, (data & 0x02) != 0);
            device.send_button(timestamp, joystick, B::LeftStick as u8, (data & 0x04) != 0);
            device.send_button(timestamp, joystick, B::RightStick as u8, (data & 0x08) != 0);
            device.send_button(timestamp, joystick, B::Guide as u8, (data & 0x10) != 0);
            device.send_button(
                timestamp,
                joystick,
                SDL_GAMEPAD_BUTTON_SWITCH_SHARE,
                (data & 0x20) != 0,
            );
        }

        if stick_hat != last[2] {
            if self.switch2 {
                device.send_button(
                    timestamp,
                    joystick,
                    SDL_GAMEPAD_BUTTON_SWITCH2_C,
                    (stick_hat & 0x80) != 0,
                );
            }

            device.send_hat(timestamp, joystick, 0, hat_of(stick_hat & 0x0F));
        }

        device.send_axis(
            timestamp,
            joystick,
            GamepadAxis::LeftTrigger as u8,
            digital_trigger(buttons[0] & 0x40 != 0),
        );
        device.send_axis(
            timestamp,
            joystick,
            GamepadAxis::RightTrigger as u8,
            digital_trigger(buttons[0] & 0x80 != 0),
        );

        let remap = |value: u8| remap_axis_u8(value);
        let sticks = [
            (left[0], last[3], GamepadAxis::LeftX),
            (left[1], last[4], GamepadAxis::LeftY),
            (right[0], last[5], GamepadAxis::RightX),
            (right[1], last[6], GamepadAxis::RightY),
        ];
        for (value, last_value, axis) in sticks {
            if value != last_value {
                device.send_axis(timestamp, joystick, axis as u8, remap(value));
            }
        }

        self.last_input_only_state.copy_from_slice(packet);
    }

    /// Translation of `HandleCombinedSimpleControllerStateL()`.
    fn handle_combined_simple_controller_state_l(
        &mut self,
        device: &mut DeviceCtx<'_>,
        timestamp: u64,
        joystick: JoystickID,
        packet: &[u8],
    ) {
        let last = self.last_simple_state;
        if packet[0] != last[0] {
            let data = packet[0];
            let mut hat = 0;

            if data & 0x01 != 0 {
                hat |= HAT_LEFT;
            }
            if data & 0x02 != 0 {
                hat |= HAT_DOWN;
            }
            if data & 0x04 != 0 {
                hat |= HAT_UP;
            }
            if data & 0x08 != 0 {
                hat |= HAT_RIGHT;
            }
            device.send_hat(timestamp, joystick, 0, hat);

            device.send_button(
                timestamp,
                joystick,
                SDL_GAMEPAD_BUTTON_SWITCH_LEFT_PADDLE1,
                (data & 0x10) != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                SDL_GAMEPAD_BUTTON_SWITCH_LEFT_PADDLE2,
                (data & 0x20) != 0,
            );
        }

        if packet[1] != last[1] {
            let data = packet[1];
            device.send_button(
                timestamp,
                joystick,
                GamepadButton::Back as u8,
                (data & 0x01) != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                GamepadButton::LeftStick as u8,
                (data & 0x04) != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                SDL_GAMEPAD_BUTTON_SWITCH_SHARE,
                (data & 0x20) != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                GamepadButton::LeftShoulder as u8,
                (data & 0x40) != 0,
            );
        }

        device.send_axis(
            timestamp,
            joystick,
            GamepadAxis::LeftTrigger as u8,
            digital_trigger(packet[1] & 0x80 != 0),
        );

        if packet[2] != last[2] {
            let (x, y) = stick_hat_axes(&COMBINED_SIMPLE_L_STICK, packet[2]);
            device.send_axis(timestamp, joystick, GamepadAxis::LeftX as u8, x);
            device.send_axis(timestamp, joystick, GamepadAxis::LeftY as u8, y);
        }
    }

    /// Translation of `HandleCombinedSimpleControllerStateR()`.
    fn handle_combined_simple_controller_state_r(
        &mut self,
        device: &mut DeviceCtx<'_>,
        timestamp: u64,
        joystick: JoystickID,
        packet: &[u8],
    ) {
        use GamepadButton as B;
        let last = self.last_simple_state;
        if packet[0] != last[0] {
            let data = packet[0];
            device.send_button(
                timestamp,
                joystick,
                self.remap_button(B::East),
                (data & 0x01) != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                self.remap_button(B::North),
                (data & 0x02) != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                self.remap_button(B::South),
                (data & 0x04) != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                self.remap_button(B::West),
                (data & 0x08) != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                SDL_GAMEPAD_BUTTON_SWITCH_RIGHT_PADDLE2,
                (data & 0x10) != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                SDL_GAMEPAD_BUTTON_SWITCH_RIGHT_PADDLE1,
                (data & 0x20) != 0,
            );
        }

        if packet[1] != last[1] {
            let data = packet[1];
            device.send_button(timestamp, joystick, B::Start as u8, (data & 0x02) != 0);
            device.send_button(timestamp, joystick, B::RightStick as u8, (data & 0x08) != 0);
            device.send_button(timestamp, joystick, B::Guide as u8, (data & 0x10) != 0);
            device.send_button(
                timestamp,
                joystick,
                B::RightShoulder as u8,
                (data & 0x40) != 0,
            );
        }

        device.send_axis(
            timestamp,
            joystick,
            GamepadAxis::RightTrigger as u8,
            digital_trigger(packet[1] & 0x80 != 0),
        );

        if packet[2] != last[2] {
            let (x, y) = stick_hat_axes(&COMBINED_SIMPLE_R_STICK, packet[2]);
            device.send_axis(timestamp, joystick, GamepadAxis::RightX as u8, x);
            device.send_axis(timestamp, joystick, GamepadAxis::RightY as u8, y);
        }
    }

    /// Translation of `HandleMiniSimpleControllerStateL()`.
    fn handle_mini_simple_controller_state_l(
        &mut self,
        device: &mut DeviceCtx<'_>,
        timestamp: u64,
        joystick: JoystickID,
        packet: &[u8],
    ) {
        use GamepadButton as B;
        let last = self.last_simple_state;
        if packet[0] != last[0] {
            let data = packet[0];
            device.send_button(
                timestamp,
                joystick,
                self.remap_button(B::South),
                (data & 0x01) != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                self.remap_button(B::East),
                (data & 0x02) != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                self.remap_button(B::West),
                (data & 0x04) != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                self.remap_button(B::North),
                (data & 0x08) != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                B::LeftShoulder as u8,
                (data & 0x10) != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                B::RightShoulder as u8,
                (data & 0x20) != 0,
            );
        }

        if packet[1] != last[1] {
            let data = packet[1];
            device.send_button(timestamp, joystick, B::Start as u8, (data & 0x01) != 0);
            device.send_button(timestamp, joystick, B::LeftStick as u8, (data & 0x04) != 0);
            device.send_button(timestamp, joystick, B::Guide as u8, (data & 0x20) != 0);
            device.send_button(
                timestamp,
                joystick,
                SDL_GAMEPAD_BUTTON_SWITCH_LEFT_PADDLE1,
                (data & 0x40) != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                SDL_GAMEPAD_BUTTON_SWITCH_LEFT_PADDLE2,
                (data & 0x80) != 0,
            );
        }

        if packet[2] != last[2] {
            let (x, y) = stick_hat_axes(&MINI_SIMPLE_STICK, packet[2]);
            device.send_axis(timestamp, joystick, GamepadAxis::LeftX as u8, x);
            device.send_axis(timestamp, joystick, GamepadAxis::LeftY as u8, y);
        }
    }

    /// Translation of `HandleMiniSimpleControllerStateR()`.
    fn handle_mini_simple_controller_state_r(
        &mut self,
        device: &mut DeviceCtx<'_>,
        timestamp: u64,
        joystick: JoystickID,
        packet: &[u8],
    ) {
        use GamepadButton as B;
        let last = self.last_simple_state;
        if packet[0] != last[0] {
            let data = packet[0];
            device.send_button(
                timestamp,
                joystick,
                self.remap_button(B::South),
                (data & 0x01) != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                self.remap_button(B::East),
                (data & 0x02) != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                self.remap_button(B::West),
                (data & 0x04) != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                self.remap_button(B::North),
                (data & 0x08) != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                B::LeftShoulder as u8,
                (data & 0x10) != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                B::RightShoulder as u8,
                (data & 0x20) != 0,
            );
        }

        if packet[1] != last[1] {
            let data = packet[1];
            device.send_button(timestamp, joystick, B::Start as u8, (data & 0x02) != 0);
            device.send_button(timestamp, joystick, B::LeftStick as u8, (data & 0x08) != 0);
            device.send_button(timestamp, joystick, B::Guide as u8, (data & 0x10) != 0);
            device.send_button(
                timestamp,
                joystick,
                SDL_GAMEPAD_BUTTON_SWITCH_SHARE,
                (data & 0x20) != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                SDL_GAMEPAD_BUTTON_SWITCH_RIGHT_PADDLE1,
                (data & 0x40) != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                SDL_GAMEPAD_BUTTON_SWITCH_RIGHT_PADDLE2,
                (data & 0x80) != 0,
            );
        }

        if packet[2] != last[2] {
            let (x, y) = stick_hat_axes(&MINI_SIMPLE_STICK, packet[2]);
            device.send_axis(timestamp, joystick, GamepadAxis::LeftX as u8, x);
            device.send_axis(timestamp, joystick, GamepadAxis::LeftY as u8, y);
        }
    }

    /// Translation of `HandleSimpleControllerState()`.
    fn handle_simple_controller_state(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: JoystickID,
        packet: &[u8],
    ) {
        use GamepadButton as B;
        let timestamp = crate::timer::ticks_ns();
        let packet = &packet[..SIMPLE_STATE_SIZE];
        let combined = device.parent().is_some() || self.vertical_mode;

        if self.controller_type == ControllerType::JOYCON_LEFT {
            if combined {
                self.handle_combined_simple_controller_state_l(device, timestamp, joystick, packet);
            } else {
                self.handle_mini_simple_controller_state_l(device, timestamp, joystick, packet);
            }
        } else if self.controller_type == ControllerType::JOYCON_RIGHT {
            if combined {
                self.handle_combined_simple_controller_state_r(device, timestamp, joystick, packet);
            } else {
                self.handle_mini_simple_controller_state_r(device, timestamp, joystick, packet);
            }
        } else {
            let last = self.last_simple_state;
            if packet[0] != last[0] {
                let data = packet[0];
                device.send_button(
                    timestamp,
                    joystick,
                    self.remap_button(B::South),
                    (data & 0x01) != 0,
                );
                device.send_button(
                    timestamp,
                    joystick,
                    self.remap_button(B::East),
                    (data & 0x02) != 0,
                );
                device.send_button(
                    timestamp,
                    joystick,
                    self.remap_button(B::West),
                    (data & 0x04) != 0,
                );
                device.send_button(
                    timestamp,
                    joystick,
                    self.remap_button(B::North),
                    (data & 0x08) != 0,
                );
                device.send_button(
                    timestamp,
                    joystick,
                    B::LeftShoulder as u8,
                    (data & 0x10) != 0,
                );
                device.send_button(
                    timestamp,
                    joystick,
                    B::RightShoulder as u8,
                    (data & 0x20) != 0,
                );
            }

            if packet[1] != last[1] {
                let data = packet[1];
                device.send_button(timestamp, joystick, B::Back as u8, (data & 0x01) != 0);
                device.send_button(timestamp, joystick, B::Start as u8, (data & 0x02) != 0);
                device.send_button(timestamp, joystick, B::LeftStick as u8, (data & 0x04) != 0);
                device.send_button(timestamp, joystick, B::RightStick as u8, (data & 0x08) != 0);
                device.send_button(timestamp, joystick, B::Guide as u8, (data & 0x10) != 0);
                device.send_button(
                    timestamp,
                    joystick,
                    SDL_GAMEPAD_BUTTON_SWITCH_SHARE,
                    (data & 0x20) != 0,
                );
            }

            if packet[2] != last[2] {
                device.send_hat(timestamp, joystick, 0, hat_of(packet[2]));
            }

            device.send_axis(
                timestamp,
                joystick,
                GamepadAxis::LeftTrigger as u8,
                digital_trigger(packet[0] & 0x40 != 0),
            );
            device.send_axis(
                timestamp,
                joystick,
                GamepadAxis::RightTrigger as u8,
                digital_trigger((packet[0] & 0x80) != 0 || (packet[1] & 0x80) != 0),
            );

            let axes = [
                (0, 0, 3, GamepadAxis::LeftX),
                (0, 1, 5, GamepadAxis::LeftY),
                (1, 0, 7, GamepadAxis::RightX),
                (1, 1, 9, GamepadAxis::RightY),
            ];
            for (stick, axis, at, gamepad_axis) in axes {
                let value = self.apply_simple_stick_calibration(stick, axis, le_i16(packet, at));
                device.send_axis(timestamp, joystick, gamepad_axis as u8, value);
            }
        }

        self.last_simple_state.copy_from_slice(packet);
    }

    /// Translation of `SendSensorUpdate()`; `values` are the three values
    /// of a sensor in the packed IMU state.
    fn send_sensor_update(
        &self,
        device: &mut DeviceCtx<'_>,
        timestamp: u64,
        joystick: JoystickID,
        sensor_type: SensorType,
        sensor_timestamp: u64,
        values: &[u8],
    ) {
        let value = |i: usize| le_i16(values, i * 2);
        let scale = &self.imu_scale_data;
        let mut data = [0.0f32; 3];

        // Note the order of components has been shuffled to match PlayStation controllers,
        // since that's our de facto standard from already supporting those controllers, and
        // users will want consistent axis mappings across devices.
        if matches!(
            sensor_type,
            SensorType::Gyro | SensorType::GyroL | SensorType::GyroR
        ) {
            let gyro_x = (i32::from(value(0)) - i32::from(scale.gyro_offset_x)) as f32;
            let gyro_y = (i32::from(value(1)) - i32::from(scale.gyro_offset_y)) as f32;
            let gyro_z = (i32::from(value(2)) - i32::from(scale.gyro_offset_z)) as f32;

            data[0] = -(scale.gyro_scale_y * gyro_y);
            data[1] = scale.gyro_scale_z * gyro_z;
            data[2] = -(scale.gyro_scale_x * gyro_x);
        } else {
            data[0] = -(scale.accel_scale_y * f32::from(value(1)));
            data[1] = scale.accel_scale_z * f32::from(value(2));
            data[2] = -(scale.accel_scale_x * f32::from(value(0)));
        }

        // Right Joy-Con flips some axes, so let's flip them back for consistency
        if self.controller_type == ControllerType::JOYCON_RIGHT {
            data[0] = -data[0];
            data[1] = -data[1];
        }

        let mini = device.parent().is_none() && !self.vertical_mode;
        if self.controller_type == ControllerType::JOYCON_LEFT && mini {
            // Mini-gamepad mode, swap some axes around
            let tmp = data[2];
            data[2] = -data[0];
            data[0] = tmp;
        }

        if self.controller_type == ControllerType::JOYCON_RIGHT && mini {
            // Mini-gamepad mode, swap some axes around
            let tmp = data[2];
            data[2] = data[0];
            data[0] = -tmp;
        }

        device.send_sensor(timestamp, joystick, sensor_type, sensor_timestamp, &data);
    }

    /// Translation of `HandleCombinedControllerStateL()`.
    fn handle_combined_controller_state_l(
        &mut self,
        device: &mut DeviceCtx<'_>,
        timestamp: u64,
        joystick: JoystickID,
        packet: &[u8],
    ) {
        let buttons = &packet[STATE_BUTTONS..STATE_BUTTONS + 3];
        let last = &self.last_full_state[STATE_BUTTONS..STATE_BUTTONS + 3];

        if buttons[1] != last[1] {
            let data = buttons[1];
            device.send_button(
                timestamp,
                joystick,
                GamepadButton::Back as u8,
                (data & 0x01) != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                GamepadButton::LeftStick as u8,
                (data & 0x08) != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                SDL_GAMEPAD_BUTTON_SWITCH_SHARE,
                (data & 0x20) != 0,
            );
        }

        if buttons[2] != last[2] {
            let data = buttons[2];
            device.send_hat(timestamp, joystick, 0, dpad_hat(data));

            device.send_button(
                timestamp,
                joystick,
                SDL_GAMEPAD_BUTTON_SWITCH_LEFT_PADDLE2,
                (data & 0x10) != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                SDL_GAMEPAD_BUTTON_SWITCH_LEFT_PADDLE1,
                (data & 0x20) != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                GamepadButton::LeftShoulder as u8,
                (data & 0x40) != 0,
            );
            device.send_axis(
                timestamp,
                joystick,
                GamepadAxis::LeftTrigger as u8,
                digital_trigger(data & 0x80 != 0),
            );
        }

        let (x, y) = stick_axes(&packet[STATE_JOYSTICK_LEFT..]);
        let axis = self.apply_stick_calibration(0, 0, x);
        device.send_axis(timestamp, joystick, GamepadAxis::LeftX as u8, axis);

        let axis = self.apply_stick_calibration(0, 1, y);
        device.send_axis(timestamp, joystick, GamepadAxis::LeftY as u8, !axis);
    }

    /// Translation of `HandleCombinedControllerStateR()`.
    fn handle_combined_controller_state_r(
        &mut self,
        device: &mut DeviceCtx<'_>,
        timestamp: u64,
        joystick: JoystickID,
        packet: &[u8],
    ) {
        use GamepadButton as B;
        let buttons = &packet[STATE_BUTTONS..STATE_BUTTONS + 3];
        let last = &self.last_full_state[STATE_BUTTONS..STATE_BUTTONS + 3];

        if buttons[0] != last[0] {
            let data = buttons[0];
            device.send_button(
                timestamp,
                joystick,
                self.remap_button(B::South),
                (data & 0x04) != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                self.remap_button(B::East),
                (data & 0x08) != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                self.remap_button(B::West),
                (data & 0x01) != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                self.remap_button(B::North),
                (data & 0x02) != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                SDL_GAMEPAD_BUTTON_SWITCH_RIGHT_PADDLE1,
                (data & 0x10) != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                SDL_GAMEPAD_BUTTON_SWITCH_RIGHT_PADDLE2,
                (data & 0x20) != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                B::RightShoulder as u8,
                (data & 0x40) != 0,
            );
            device.send_axis(
                timestamp,
                joystick,
                GamepadAxis::RightTrigger as u8,
                digital_trigger(data & 0x80 != 0),
            );
        }

        if buttons[1] != last[1] {
            let data = buttons[1];
            device.send_button(timestamp, joystick, B::Start as u8, (data & 0x02) != 0);
            device.send_button(timestamp, joystick, B::RightStick as u8, (data & 0x04) != 0);
            device.send_button(timestamp, joystick, B::Guide as u8, (data & 0x10) != 0);
        }

        let (x, y) = stick_axes(&packet[STATE_JOYSTICK_RIGHT..]);
        let axis = self.apply_stick_calibration(1, 0, x);
        device.send_axis(timestamp, joystick, GamepadAxis::RightX as u8, axis);

        let axis = self.apply_stick_calibration(1, 1, y);
        device.send_axis(timestamp, joystick, GamepadAxis::RightY as u8, !axis);
    }

    /// Translation of `HandleMiniControllerStateL()`.
    fn handle_mini_controller_state_l(
        &mut self,
        device: &mut DeviceCtx<'_>,
        timestamp: u64,
        joystick: JoystickID,
        packet: &[u8],
    ) {
        use GamepadButton as B;
        let buttons = &packet[STATE_BUTTONS..STATE_BUTTONS + 3];
        let last = &self.last_full_state[STATE_BUTTONS..STATE_BUTTONS + 3];

        if buttons[1] != last[1] {
            let data = buttons[1];
            device.send_button(timestamp, joystick, B::Start as u8, (data & 0x01) != 0);
            device.send_button(timestamp, joystick, B::LeftStick as u8, (data & 0x08) != 0);
            device.send_button(timestamp, joystick, B::Guide as u8, (data & 0x20) != 0);
        }

        if buttons[2] != last[2] {
            let data = buttons[2];
            device.send_button(
                timestamp,
                joystick,
                self.remap_button(B::South),
                (data & 0x08) != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                self.remap_button(B::East),
                (data & 0x01) != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                self.remap_button(B::West),
                (data & 0x02) != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                self.remap_button(B::North),
                (data & 0x04) != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                B::RightShoulder as u8,
                (data & 0x10) != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                B::LeftShoulder as u8,
                (data & 0x20) != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                SDL_GAMEPAD_BUTTON_SWITCH_LEFT_PADDLE1,
                (data & 0x40) != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                SDL_GAMEPAD_BUTTON_SWITCH_LEFT_PADDLE2,
                (data & 0x80) != 0,
            );
        }

        let (x, y) = stick_axes(&packet[STATE_JOYSTICK_LEFT..]);
        let axis = self.apply_stick_calibration(0, 0, x);
        device.send_axis(timestamp, joystick, GamepadAxis::LeftY as u8, !axis);

        let axis = self.apply_stick_calibration(0, 1, y);
        device.send_axis(timestamp, joystick, GamepadAxis::LeftX as u8, !axis);
    }

    /// Translation of `HandleMiniControllerStateR()`.
    fn handle_mini_controller_state_r(
        &mut self,
        device: &mut DeviceCtx<'_>,
        timestamp: u64,
        joystick: JoystickID,
        packet: &[u8],
    ) {
        use GamepadButton as B;
        let buttons = &packet[STATE_BUTTONS..STATE_BUTTONS + 3];
        let last = &self.last_full_state[STATE_BUTTONS..STATE_BUTTONS + 3];

        if buttons[0] != last[0] {
            let data = buttons[0];
            device.send_button(
                timestamp,
                joystick,
                self.remap_button(B::South),
                (data & 0x08) != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                self.remap_button(B::East),
                (data & 0x02) != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                self.remap_button(B::West),
                (data & 0x04) != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                self.remap_button(B::North),
                (data & 0x01) != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                B::RightShoulder as u8,
                (data & 0x10) != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                B::LeftShoulder as u8,
                (data & 0x20) != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                SDL_GAMEPAD_BUTTON_SWITCH_RIGHT_PADDLE1,
                (data & 0x40) != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                SDL_GAMEPAD_BUTTON_SWITCH_RIGHT_PADDLE2,
                (data & 0x80) != 0,
            );
        }

        if buttons[1] != last[1] {
            let data = buttons[1];
            device.send_button(timestamp, joystick, B::Start as u8, (data & 0x02) != 0);
            device.send_button(timestamp, joystick, B::LeftStick as u8, (data & 0x04) != 0);
            device.send_button(timestamp, joystick, B::Guide as u8, (data & 0x10) != 0);
        }

        let (x, y) = stick_axes(&packet[STATE_JOYSTICK_RIGHT..]);
        let axis = self.apply_stick_calibration(1, 0, x);
        device.send_axis(timestamp, joystick, GamepadAxis::LeftY as u8, axis);

        let axis = self.apply_stick_calibration(1, 1, y);
        device.send_axis(timestamp, joystick, GamepadAxis::LeftX as u8, axis);
    }

    /// Translation of `HandleFullControllerState()`.
    fn handle_full_controller_state(
        &mut self,
        device: &mut DeviceCtx<'_>,
        link: Link<'_>,
        joystick: JoystickID,
        packet: &[u8],
    ) {
        use GamepadButton as B;
        let timestamp = crate::timer::ticks_ns();
        let packet = &packet[..FULL_STATE_SIZE];
        let parent = device.parent();
        let combined = parent.is_some() || self.vertical_mode;

        if self.controller_type == ControllerType::JOYCON_LEFT {
            if combined {
                self.handle_combined_controller_state_l(device, timestamp, joystick, packet);
            } else {
                self.handle_mini_controller_state_l(device, timestamp, joystick, packet);
            }
        } else if self.controller_type == ControllerType::JOYCON_RIGHT {
            if combined {
                self.handle_combined_controller_state_r(device, timestamp, joystick, packet);
            } else {
                self.handle_mini_controller_state_r(device, timestamp, joystick, packet);
            }
        } else {
            let buttons = &packet[STATE_BUTTONS..STATE_BUTTONS + 3];
            let last: [u8; 3] = [
                self.last_full_state[STATE_BUTTONS],
                self.last_full_state[STATE_BUTTONS + 1],
                self.last_full_state[STATE_BUTTONS + 2],
            ];

            if buttons[0] != last[0] {
                let data = buttons[0];
                device.send_button(
                    timestamp,
                    joystick,
                    self.remap_button(B::South),
                    (data & 0x04) != 0,
                );
                device.send_button(
                    timestamp,
                    joystick,
                    self.remap_button(B::East),
                    (data & 0x08) != 0,
                );
                device.send_button(
                    timestamp,
                    joystick,
                    self.remap_button(B::West),
                    (data & 0x01) != 0,
                );
                device.send_button(
                    timestamp,
                    joystick,
                    self.remap_button(B::North),
                    (data & 0x02) != 0,
                );
                device.send_button(
                    timestamp,
                    joystick,
                    B::RightShoulder as u8,
                    (data & 0x40) != 0,
                );
            }

            if buttons[1] != last[1] {
                let data = buttons[1];
                device.send_button(timestamp, joystick, B::Back as u8, (data & 0x01) != 0);
                device.send_button(timestamp, joystick, B::Start as u8, (data & 0x02) != 0);
                device.send_button(timestamp, joystick, B::RightStick as u8, (data & 0x04) != 0);
                device.send_button(timestamp, joystick, B::LeftStick as u8, (data & 0x08) != 0);

                device.send_button(timestamp, joystick, B::Guide as u8, (data & 0x10) != 0);
                device.send_button(
                    timestamp,
                    joystick,
                    SDL_GAMEPAD_BUTTON_SWITCH_SHARE,
                    (data & 0x20) != 0,
                );
                if self.switch2 {
                    device.send_button(
                        timestamp,
                        joystick,
                        SDL_GAMEPAD_BUTTON_SWITCH2_C,
                        (data & 0x40) != 0,
                    );
                }
            }

            if buttons[2] != last[2] {
                let data = buttons[2];
                device.send_hat(timestamp, joystick, 0, dpad_hat(data));

                device.send_button(
                    timestamp,
                    joystick,
                    B::LeftShoulder as u8,
                    (data & 0x40) != 0,
                );
            }

            device.send_axis(
                timestamp,
                joystick,
                GamepadAxis::RightTrigger as u8,
                digital_trigger(buttons[0] & 0x80 != 0),
            );
            device.send_axis(
                timestamp,
                joystick,
                GamepadAxis::LeftTrigger as u8,
                digital_trigger(buttons[2] & 0x80 != 0),
            );

            let (x, y) = stick_axes(&packet[STATE_JOYSTICK_LEFT..]);
            let axis = self.apply_stick_calibration(0, 0, x);
            device.send_axis(timestamp, joystick, GamepadAxis::LeftX as u8, axis);
            let axis = self.apply_stick_calibration(0, 1, y);
            device.send_axis(timestamp, joystick, GamepadAxis::LeftY as u8, !axis);

            let (x, y) = stick_axes(&packet[STATE_JOYSTICK_RIGHT..]);
            let axis = self.apply_stick_calibration(1, 0, x);
            device.send_axis(timestamp, joystick, GamepadAxis::RightX as u8, axis);
            let axis = self.apply_stick_calibration(1, 1, y);
            device.send_axis(timestamp, joystick, GamepadAxis::RightY as u8, !axis);
        }

        // High nibble of battery/connection byte is battery level, low nibble is connection status (always 0 on 8BitDo Pro 2)
        // LSB of connection nibble is USB/Switch connection status
        // LSB of the battery nibble is used to report charging.
        // The battery level is reported from 0(empty)-8(full)
        let battery_and_connection = packet[STATE_BATTERY_AND_CONNECTION];
        let charging = battery_and_connection & 0x10 != 0;
        let level = i32::from(battery_and_connection & 0xE0) >> 4;
        self.power_state = if charging {
            if level == 8 {
                PowerState::Charged
            } else {
                PowerState::Charging
            }
        } else {
            PowerState::OnBattery
        };
        self.power_percent = ((level as f32 / 8.0) * 100.0).round() as i32;

        match &parent {
            None => device.send_power_info(joystick, self.power_state, self.power_percent),
            Some(parent) if self.controller_type == ControllerType::JOYCON_RIGHT => {
                // (the other Joy-Con's state; this one's if it's busy)
                let (other_state, other_percent) = parent
                    .children()
                    .first()
                    .and_then(|other| other.context_power_info())
                    .unwrap_or((self.power_state, self.power_percent));
                let state = min_power_state(self.power_state, other_state);
                let percent = self.power_percent.min(other_percent);
                device.send_power_info(joystick, state, percent);
            }
            Some(_) => {}
        }

        if self.report_sensors {
            let imu =
                |i: usize| &packet[CONTROLLER_STATE_SIZE + i * IMU_STATE_SIZE..][..IMU_STATE_SIZE];
            let has_sensor_data = imu(0)[..6].iter().any(|&b| b != 0);
            if has_sensor_data {
                const IMU_UPDATE_RATE_SAMPLE_FREQUENCY: u32 = 1000;

                self.has_sensor_data = true;

                // We got three IMU samples, calculate the IMU update rate and timestamps
                self.imu_samples += 3;
                if self.imu_samples >= IMU_UPDATE_RATE_SAMPLE_FREQUENCY {
                    let now = crate::timer::ticks_ns();
                    let elapsed = now.wrapping_sub(self.imu_sample_timestamp_ns);

                    if elapsed > 0 {
                        self.imu_update_interval_ns = elapsed / u64::from(self.imu_samples);
                    }
                    self.imu_samples = 0;
                    self.imu_sample_timestamp_ns = now;
                }

                let mut sensor_timestamp = [0u64; 3];
                for t in &mut sensor_timestamp {
                    self.timestamp_ns += self.imu_update_interval_ns;
                    *t = self.timestamp_ns;
                }

                // (the gyro and accelerometer values of each IMU state)
                let accel = |i: usize| &imu(i)[0..6];
                let gyro = |i: usize| &imu(i)[6..12];
                let send = |this: &Self, device: &mut DeviceCtx<'_>, gyro_type, accel_type| {
                    for (t, i) in sensor_timestamp.iter().zip([2, 1, 0]) {
                        this.send_sensor_update(
                            device,
                            timestamp,
                            joystick,
                            gyro_type,
                            *t,
                            gyro(i),
                        );
                        this.send_sensor_update(
                            device,
                            timestamp,
                            joystick,
                            accel_type,
                            *t,
                            accel(i),
                        );
                    }
                };
                if parent.is_none() || self.controller_type == ControllerType::JOYCON_RIGHT {
                    send(self, device, SensorType::Gyro, SensorType::Accel);
                }
                if parent.is_some() && self.controller_type == ControllerType::JOYCON_LEFT {
                    send(self, device, SensorType::GyroL, SensorType::AccelL);
                }
                if parent.is_some() && self.controller_type == ControllerType::JOYCON_RIGHT {
                    send(self, device, SensorType::GyroR, SensorType::AccelR);
                }
            } else if self.has_sensor_data {
                // Uh oh, someone turned off the IMU?
                const IMU_RESET_DELAY_MS: u64 = 3000;
                let now = crate::timer::ticks_ms();

                if now >= self.last_imu_reset + IMU_RESET_DELAY_MS {
                    self.set_imu_enabled(link, true);
                    self.last_imu_reset = now;
                }
            } else {
                // We have never gotten IMU data, probably not supported on this device
            }
        }

        self.last_full_state.copy_from_slice(packet);
    }

    /// One report of `HIDAPI_DriverSwitch_UpdateDevice()`'s read loop,
    /// for an open joystick; the report is in the read buffer.
    fn handle_report(&mut self, device: &mut DeviceCtx<'_>, link: Link<'_>, joystick: JoystickID) {
        let buf = self.read_buffer;

        if self.input_only {
            self.handle_input_only_controller_state(device, joystick, &buf);
        } else {
            if buf[0] == INPUT_REPORT_SUBCOMMAND_REPLY {
                return;
            }

            self.current_input_mode = buf[0];

            match buf[0] {
                INPUT_REPORT_SIMPLE_CONTROLLER_STATE => {
                    self.handle_simple_controller_state(device, joystick, &buf[1..])
                }
                INPUT_REPORT_FULL_CONTROLLER_STATE | INPUT_REPORT_FULL_CONTROLLER_AND_MCU_STATE => {
                    // This is the extended report, we can enable sensors now in auto mode
                    self.update_enhanced_mode_on_enhanced_report(
                        link,
                        &mut JoystickRef::Open(joystick),
                    );

                    self.handle_full_controller_state(device, link, joystick, &buf[1..]);
                }
                _ => {}
            }
        }
    }
}

/// The axis of an input-only stick byte.
fn remap_axis_u8(value: u8) -> i16 {
    remap_val(
        f32::from(value),
        0.0,
        255.0,
        f32::from(i16::MIN),
        f32::from(i16::MAX),
    ) as i16
}

/// Translation of `ReadJoyConControllerType()`.
fn read_joycon_controller_type(device: &HidapiDevice) -> ControllerType {
    let mut controller_type = ControllerType::UNKNOWN;
    const MAX_ATTEMPTS: i32 = 1; // Don't try too long, in case this is a zombie Bluetooth controller
    let mut attempts = 0;
    let link = Link::Probe(device);

    // Create enough of a context to read the controller type from the device
    let mut ctx = SwitchContext {
        sync_write: true,
        max_write_attempts: get_max_write_attempts(device),
        ..SwitchContext::default()
    };

    loop {
        attempts += 1;
        if device.is_bluetooth() {
            if ctx.write_subcommand(link, SUBCOMMAND_REQUEST_DEVICE_INFO, &[]) {
                controller_type = calculate_controller_type(
                    device,
                    ControllerType(ctx.reply()[REPLY_DEVICE_TYPE]),
                );
            }
        } else if ctx.write_proprietary(link, PROPRIETARY_STATUS, &[], true) {
            controller_type = calculate_controller_type(device, ControllerType(ctx.read_buffer[3]));
        }
        if controller_type == ControllerType::UNKNOWN && attempts < MAX_ATTEMPTS {
            // Wait a bit and try again
            crate::timer::delay(Duration::from_millis(100));
            continue;
        }
        break;
    }
    controller_type
}

/// The static functions of the Nintendo Online classic controller driver.
pub(crate) struct NintendoClassicDriver;

/// Translation of `HIDAPI_DriverNintendoClassic_IsSupportedDevice()`.
fn nintendo_classic_is_supported(name: &str, vendor_id: u16, product_id: u16) -> bool {
    if vendor_id == USB_VENDOR_NINTENDO {
        if product_id == USB_PRODUCT_NINTENDO_SWITCH_JOYCON_RIGHT
            && (name.starts_with("NES Controller") || name.starts_with("HVC Controller"))
        {
            return true;
        }

        if product_id == USB_PRODUCT_NINTENDO_N64_CONTROLLER {
            return true;
        }

        if product_id == USB_PRODUCT_NINTENDO_SEGA_GENESIS_CONTROLLER {
            return true;
        }

        if product_id == USB_PRODUCT_NINTENDO_SNES_CONTROLLER {
            return true;
        }
    }
    false
}

/// Translation of `HIDAPI_DriverJoyCons_IsSupportedDevice()`.
fn joycons_is_supported(device: Option<&HidapiDevice>, vendor_id: u16, product_id: u16) -> bool {
    if vendor_id == USB_VENDOR_NINTENDO {
        if product_id == USB_PRODUCT_NINTENDO_SWITCH_PRO {
            if let Some(device) = device.filter(|d| d.dev().is_some()) {
                // This might be a Kinvoca Joy-Con that reports VID/PID as a Switch Pro controller
                let controller_type = read_joycon_controller_type(device);
                if controller_type == ControllerType::JOYCON_LEFT
                    || controller_type == ControllerType::JOYCON_RIGHT
                {
                    return true;
                }
            }
        }

        if product_id == USB_PRODUCT_NINTENDO_SWITCH_JOYCON_LEFT
            || product_id == USB_PRODUCT_NINTENDO_SWITCH_JOYCON_RIGHT
            || product_id == USB_PRODUCT_NINTENDO_SWITCH_JOYCON_GRIP
        {
            return true;
        }
    }
    false
}

impl DriverImpl for NintendoClassicDriver {
    /// Translation of `HIDAPI_DriverNintendoClassic_RegisterHints()`.
    fn hints(&self) -> &'static [&'static str] {
        &[hints::JOYSTICK_HIDAPI_NINTENDO_CLASSIC]
    }

    /// Translation of `HIDAPI_DriverNintendoClassic_IsEnabled()`.
    fn is_enabled(&self) -> bool {
        hints::get_bool(
            hints::JOYSTICK_HIDAPI_NINTENDO_CLASSIC,
            hints::get_bool(hints::JOYSTICK_HIDAPI, SDL_HIDAPI_DEFAULT),
        )
    }

    fn is_supported_device(
        &self,
        _device: Option<&HidapiDevice>,
        name: &str,
        _gamepad_type: GamepadType,
        vendor_id: u16,
        product_id: u16,
        _version: u16,
        _interface_number: i32,
        _interface_class: i32,
        _interface_subclass: i32,
        _interface_protocol: i32,
    ) -> bool {
        nintendo_classic_is_supported(name, vendor_id, product_id)
    }

    fn new_context(&self) -> Box<dyn DriverContext> {
        Box::new(SwitchContext::default())
    }
}

/// The static functions of the Joy-Con driver.
pub(crate) struct JoyConsDriver;

impl DriverImpl for JoyConsDriver {
    /// Translation of `HIDAPI_DriverJoyCons_RegisterHints()`.
    fn hints(&self) -> &'static [&'static str] {
        &[hints::JOYSTICK_HIDAPI_JOY_CONS]
    }

    /// Translation of `HIDAPI_DriverJoyCons_IsEnabled()`.
    fn is_enabled(&self) -> bool {
        hints::get_bool(
            hints::JOYSTICK_HIDAPI_JOY_CONS,
            hints::get_bool(hints::JOYSTICK_HIDAPI, SDL_HIDAPI_DEFAULT),
        )
    }

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
        joycons_is_supported(device, vendor_id, product_id)
    }

    fn new_context(&self) -> Box<dyn DriverContext> {
        Box::new(SwitchContext::default())
    }
}

/// The static functions of the Switch Pro controller driver.
pub(crate) struct SwitchDriver;

impl DriverImpl for SwitchDriver {
    /// Translation of `HIDAPI_DriverSwitch_RegisterHints()`.
    fn hints(&self) -> &'static [&'static str] {
        &[hints::JOYSTICK_HIDAPI_SWITCH]
    }

    /// Translation of `HIDAPI_DriverSwitch_IsEnabled()`.
    fn is_enabled(&self) -> bool {
        hints::get_bool(
            hints::JOYSTICK_HIDAPI_SWITCH,
            hints::get_bool(hints::JOYSTICK_HIDAPI, SDL_HIDAPI_DEFAULT),
        )
    }

    /// Translation of `HIDAPI_DriverSwitch_IsSupportedDevice()`.
    fn is_supported_device(
        &self,
        device: Option<&HidapiDevice>,
        name: &str,
        gamepad_type: GamepadType,
        vendor_id: u16,
        product_id: u16,
        _version: u16,
        _interface_number: i32,
        _interface_class: i32,
        _interface_subclass: i32,
        _interface_protocol: i32,
    ) -> bool {
        // The HORI Wireless Switch Pad enumerates as a HID device when connected via USB
        // with the same VID/PID as when connected over Bluetooth but doesn't actually
        // support communication over USB. The most reliable way to block this without allowing the
        // controller to continually attempt to reconnect is to filter it out by manufacturer/product string.
        // Note that the controller does have a different product string when connected over Bluetooth.
        if name == "HORI Wireless Switch Pad" {
            return false;
        }

        // If it's handled by another driver, it's not handled here
        if nintendo_classic_is_supported(name, vendor_id, product_id)
            || joycons_is_supported(device, vendor_id, product_id)
        {
            return false;
        }

        if gamepad_type != GamepadType::NintendoSwitchPro {
            return false;
        }

        // The Nintendo Switch 2 Pro uses another driver
        !(vendor_id == USB_VENDOR_NINTENDO && product_id == USB_PRODUCT_NINTENDO_SWITCH2_PRO)
    }

    fn new_context(&self) -> Box<dyn DriverContext> {
        Box::new(SwitchContext::default())
    }
}

impl DriverContext for SwitchContext {
    /// Translation of `HIDAPI_DriverSwitch_InitDevice()`.
    fn init_device(&mut self, device: &mut DeviceCtx<'_>) -> Result<()> {
        let dev = device.device().clone();
        let link = Link::Device(&dev);
        let (vendor_id, product_id) = (device.vendor_id(), device.product_id());

        self.max_write_attempts = get_max_write_attempts(device);
        self.sync_write = true;

        // Find out whether or not we can send output reports
        // Third party controllers use the full Switch Pro wireless protocol over Bluetooth
        if !device.is_bluetooth() {
            self.input_only = is_joystick_nintendo_switch_pro_input_only(vendor_id, product_id)
                || is_joystick_nintendo_switch2_pro_input_only(vendor_id, product_id);
        }
        self.switch2 = is_joystick_nintendo_switch2_pro(vendor_id, product_id);

        if !self.input_only {
            // Initialize rumble data, important for reading device info on the MOBAPAD M073
            let neutral = neutral_rumble(vendor_id, product_id);
            self.rumble_packet.rumble_data = [neutral, neutral];

            self.read_device_info(link);
        }
        self.update_device_identity(device);

        // Prefer the USB device over the Bluetooth device
        let serial = device.serial();
        if device.is_bluetooth() {
            if device.has_connected_usb_device(serial.as_deref()) {
                return Ok(());
            }
        } else {
            device.disconnect_bluetooth_device(serial.as_deref());
        }
        device.joystick_connected();
        Ok(())
    }

    /// Translation of `HIDAPI_DriverSwitch_SetDevicePlayerIndex()`.
    fn set_device_player_index(
        &mut self,
        device: &mut DeviceCtx<'_>,
        _instance_id: JoystickID,
        player_index: i32,
    ) {
        if self.joystick.is_none() {
            return;
        }

        self.player_index = player_index;

        let dev = device.device().clone();
        self.update_slot_led(Link::Device(&dev));
    }

    /// Translation of `HIDAPI_DriverSwitch_UpdateDevice()`.
    fn update_device(&mut self, device: &mut DeviceCtx<'_>) -> bool {
        let dev = device.device().clone();
        let link = Link::Device(&dev);
        let mut packet_count = 0;
        let now = crate::timer::ticks_ms();

        let joystick = device.open_joystick_id();

        // (the hint callbacks of upstream)
        if let Some(joystick) = joystick.filter(|&j| self.joystick == Some(j)) {
            self.hint_changes(device, link, &mut JoystickRef::Open(joystick));
        }

        // (upstream's last `size >= 0`)
        let mut ok = loop {
            match self.read_input(link) {
                Ok(0) => break true,
                Err(_) => break false,
                Ok(_) => {}
            }
            packet_count += 1;
            self.last_input = now;

            if let Some(joystick) = joystick {
                self.handle_report(device, link, joystick);
            }
        };

        if joystick.is_some() {
            if packet_count == 0 {
                if !self.input_only
                    && !device.is_bluetooth()
                    && device.product_id() != USB_PRODUCT_NINTENDO_SWITCH_JOYCON_GRIP
                {
                    const INPUT_WAIT_TIMEOUT_MS: u64 = 100;
                    if now >= self.last_input + INPUT_WAIT_TIMEOUT_MS {
                        // Steam may have put the controller back into non-reporting mode
                        let was_sync_write = self.sync_write;

                        self.sync_write = true;
                        self.write_proprietary(link, PROPRIETARY_FORCE_USB, &[], false);
                        self.sync_write = was_sync_write;
                    }
                } else if device.is_bluetooth()
                    && self.current_input_mode != INPUT_REPORT_SIMPLE_CONTROLLER_STATE
                {
                    const INPUT_WAIT_TIMEOUT_MS: u64 = 3000;
                    if now >= self.last_input + INPUT_WAIT_TIMEOUT_MS {
                        // Bluetooth may have disconnected, try reopening the controller
                        ok = false;
                    }
                }
            }

            if self.rumble_pending || self.rumble_zero_pending {
                let _ = self.send_pending_rumble(link);
            } else if self.rumble_active && now >= self.rumble_sent + RUMBLE_REFRESH_FREQUENCY_MS {
                self.write_rumble(link);
            }
        }

        // Reconnect the Bluetooth device once the USB device is gone
        if device.num_joysticks() == 0
            && device.is_bluetooth()
            && packet_count > 0
            && device.parent().is_none()
            && !device.has_connected_usb_device(device.serial().as_deref())
        {
            device.joystick_connected();
        }

        if !ok {
            if let Some(&first) = device.joysticks().first() {
                // Read error, device is disconnected
                device.joystick_disconnected(first);
            }
        }
        ok
    }

    /// Translation of `HIDAPI_DriverSwitch_OpenJoystick()`.
    fn open_joystick(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: &mut JoystickData,
    ) -> Result<()> {
        crate::joystick::assert_joysticks_locked();

        let dev = device.device().clone();
        let link = Link::Device(&dev);
        let (vendor_id, product_id) = (device.vendor_id(), device.product_id());

        self.joystick = Some(joystick.instance_id);

        self.sync_write = true;

        if !self.input_only {
            self.get_initial_input_mode(link);
            self.current_input_mode = self.initial_input_mode;

            // Initialize rumble data
            let neutral = neutral_rumble(vendor_id, product_id);
            self.rumble_packet.rumble_data = [neutral, neutral];

            if !device.is_bluetooth() && !self.try_setup_usb(link) {
                return Err(Error::new("Couldn't setup USB mode"));
            }

            if !self.load_stick_calibration(link) {
                return Err(Error::new("Couldn't load stick calibration"));
            }

            if !matches!(
                self.controller_type,
                ControllerType::HVC_LEFT
                    | ControllerType::HVC_RIGHT
                    | ControllerType::NES_LEFT
                    | ControllerType::NES_RIGHT
                    | ControllerType::SNES
                    | ControllerType::N64
                    | ControllerType::SEGA_GENESIS
            ) && !(vendor_id == USB_VENDOR_PDP && product_id == USB_PRODUCT_PDP_REALMZ_WIRELESS)
                && self.load_imu_calibration(link)
            {
                self.sensors_supported = true;
            }

            // Enable vibration
            self.set_vibration_enabled(link, 1);

            // Set desired input mode
            self.enhanced_reports_hint = Some(HintWatch::new(hints::JOYSTICK_ENHANCED_REPORTS));
            if let Some(hint) = self
                .enhanced_reports_hint
                .as_ref()
                .and_then(HintWatch::take)
            {
                self.enhanced_reports_changed(
                    link,
                    &mut JoystickRef::Opening(joystick),
                    hint.as_deref(),
                );
            }

            // Start sending USB reports
            if !device.is_bluetooth() {
                // ForceUSB doesn't generate an ACK, so don't wait for a reply
                if !self.write_proprietary(link, PROPRIETARY_FORCE_USB, &[], false) {
                    return Err(Error::new("Couldn't start USB reports"));
                }
            }

            // Set the LED state
            if self.has_home_led(vendor_id, product_id) {
                let name = if self.controller_type == ControllerType::JOYCON_LEFT
                    || self.controller_type == ControllerType::JOYCON_RIGHT
                {
                    hints::JOYSTICK_HIDAPI_JOYCON_HOME_LED
                } else {
                    hints::JOYSTICK_HIDAPI_SWITCH_HOME_LED
                };
                self.home_led_hint = Some(HintWatch::new(name));
                if let Some(hint) = self.home_led_hint.as_ref().and_then(HintWatch::take) {
                    self.home_led_hint_changed(link, hint.as_deref());
                }
            }
        }

        if always_uses_labels(vendor_id, product_id, self.controller_type) {
            self.use_button_labels = true;
        }

        // Initialize player index (needed for setting LEDs)
        self.player_index = joystick_player_index_for_id(joystick.instance_id);
        self.player_lights = hints::get_bool(hints::JOYSTICK_HIDAPI_SWITCH_PLAYER_LED, true);
        self.update_slot_led(link);

        self.player_led_hint = Some(HintWatch::new(hints::JOYSTICK_HIDAPI_SWITCH_PLAYER_LED));
        if let Some(hint) = self.player_led_hint.as_ref().and_then(HintWatch::take) {
            self.player_led_hint_changed(device, link, hint.as_deref());
        }

        // Initialize the joystick capabilities
        joystick.nbuttons = if self.switch2 {
            SDL_GAMEPAD_NUM_SWITCH2_BUTTONS
        } else if self.input_only {
            SDL_GAMEPAD_NUM_SWITCH_INPUT_ONLY_BUTTONS
        } else {
            SDL_GAMEPAD_NUM_SWITCH_BUTTONS
        };
        joystick.naxes = GamepadAxis::COUNT;
        joystick.nhats = 1;

        // Set up for input
        self.sync_write = false;
        self.last_input = crate::timer::ticks_ms();
        self.last_imu_reset = self.last_input;
        self.imu_update_interval_ns = 5_000_000; // Start off at 5 ms update rate

        // Set up for vertical mode
        self.vertical_mode = hints::get_bool(hints::JOYSTICK_HIDAPI_VERTICAL_JOY_CONS, false);

        Ok(())
    }

    /// Translation of `HIDAPI_DriverSwitch_RumbleJoystick()`.
    fn rumble_joystick(
        &mut self,
        device: &mut DeviceCtx<'_>,
        _joystick: JoystickID,
        mut low_frequency_rumble: u16,
        mut high_frequency_rumble: u16,
    ) -> Result<()> {
        if self.input_only {
            return Err(Error::unsupported());
        }

        if device.parent().is_some() {
            if self.controller_type == ControllerType::JOYCON_LEFT {
                // Just handle low frequency rumble
                high_frequency_rumble = 0;
            } else if self.controller_type == ControllerType::JOYCON_RIGHT {
                // Just handle high frequency rumble
                low_frequency_rumble = 0;
            }
        }

        let dev = device.device().clone();
        let link = Link::Device(&dev);
        if self.rumble_pending {
            self.send_pending_rumble(link)?;
        }

        if crate::timer::ticks_ms() < self.rumble_sent + RUMBLE_WRITE_FREQUENCY_MS {
            if low_frequency_rumble != 0 || high_frequency_rumble != 0 {
                let rumble_pending =
                    (u32::from(low_frequency_rumble) << 16) | u32::from(high_frequency_rumble);

                // Keep the highest rumble intensity in the given interval
                if rumble_pending > self.rumble_pending_value {
                    self.rumble_pending_value = rumble_pending;
                }
                self.rumble_pending = true;
                self.rumble_zero_pending = false;
            } else {
                // When rumble is complete, turn it off
                self.rumble_zero_pending = true;
            }
            return Ok(());
        }

        self.actually_rumble_joystick(link, low_frequency_rumble, high_frequency_rumble)
    }

    /// Translation of `HIDAPI_DriverSwitch_GetJoystickCapabilities()`.
    fn get_joystick_capabilities(
        &mut self,
        _device: &mut DeviceCtx<'_>,
        _joystick: JoystickID,
    ) -> JoystickCaps {
        let mut result = JoystickCaps(0);

        if self.player_lights && !self.input_only {
            result |= JoystickCaps::PLAYER_LED;
        }

        if self.controller_type == ControllerType::PRO_CONTROLLER && !self.input_only {
            // Doesn't have an RGB LED, so don't return SDL_JOYSTICK_CAP_RGB_LED here
            result |= JoystickCaps::RUMBLE;
            // But has the HOME LED, so treat it like a mono LED
            result |= JoystickCaps::MONO_LED;
        } else if self.controller_type == ControllerType::JOYCON_LEFT
            || self.controller_type == ControllerType::JOYCON_RIGHT
        {
            result |= JoystickCaps::RUMBLE;
            if self.controller_type == ControllerType::JOYCON_RIGHT {
                result |= JoystickCaps::MONO_LED; // Right JoyCon also have the HOME LED
            }
        }
        result
    }

    /// Translation of `HIDAPI_DriverSwitch_SetJoystickLED()`.
    fn set_joystick_led(
        &mut self,
        device: &mut DeviceCtx<'_>,
        _joystick: JoystickID,
        red: u8,
        green: u8,
        blue: u8,
    ) -> Result<()> {
        if !(self.controller_type == ControllerType::PRO_CONTROLLER && !self.input_only)
            && self.controller_type != ControllerType::JOYCON_RIGHT
        {
            return Err(Error::unsupported());
        }

        // The colors are received between 0-255 and we need them to be 0-100
        let value = ((f32::from(red.max(green.max(blue))) / 255.0) * 100.0) as i32;
        let dev = device.device().clone();
        if !self.set_home_led(Link::Device(&dev), value as u8) {
            return Err(Error::new("Couldn't set the home LED"));
        }
        Ok(())
    }

    /// Translation of `HIDAPI_DriverSwitch_SendJoystickEffect()`.
    fn send_joystick_effect(
        &mut self,
        device: &mut DeviceCtx<'_>,
        _joystick: JoystickID,
        data: &[u8],
    ) -> Result<()> {
        let dev = device.device().clone();
        let link = Link::Device(&dev);

        if data.len() == COMMON_OUTPUT_PACKET_SIZE {
            // (SwitchCommonOutputPacket_t)
            if data[0] != OUTPUT_REPORT_RUMBLE {
                return Err(Error::new("Unknown Nintendo Switch Pro effect type"));
            }

            self.rumble_packet.rumble_data[0].copy_from_slice(&data[2..6]);
            self.rumble_packet.rumble_data[1].copy_from_slice(&data[6..10]);
            if !self.write_rumble(link) {
                return Err(Error::new("Couldn't send rumble packet"));
            }

            // This overwrites any internal rumble
            self.rumble_pending = false;
            self.rumble_zero_pending = false;
            return Ok(());
        } else if (2..=256).contains(&data.len()) {
            let cmd = data[0];

            if cmd == SUBCOMMAND_SET_INPUT_REPORT_MODE && !device.is_bluetooth() {
                // Going into simple mode over USB disables input reports, so don't do that
                return Ok(());
            }
            if cmd == SUBCOMMAND_SET_HOME_LIGHT
                && !self.has_home_led(device.vendor_id(), device.product_id())
            {
                // Setting the home LED when it's not supported can cause the controller to reset
                return Ok(());
            }

            if !self.write_subcommand(link, cmd, &data[1..]) {
                return Err(Error::new("Couldn't send subcommand"));
            }
            return Ok(());
        }
        Err(Error::unsupported())
    }

    /// Translation of `HIDAPI_DriverSwitch_SetJoystickSensorsEnabled()`.
    fn set_joystick_sensors_enabled(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: JoystickID,
        enabled: bool,
    ) -> Result<()> {
        let dev = device.device().clone();
        let link = Link::Device(&dev);

        self.update_enhanced_mode_on_application_usage(link, &mut JoystickRef::Open(joystick));

        if !self.sensors_supported || (enabled && !self.enhanced_mode) {
            return Err(Error::unsupported());
        }

        self.report_sensors = enabled;
        self.imu_samples = 0;
        self.imu_sample_timestamp_ns = crate::timer::ticks_ns();

        self.update_input_mode(link);
        self.set_imu_enabled(link, enabled);

        Ok(())
    }

    /// Translation of `HIDAPI_DriverSwitch_CloseJoystick()`.
    fn close_joystick(&mut self, device: &mut DeviceCtx<'_>, _joystick: JoystickID) {
        if !self.input_only {
            // Restore simple input mode for other applications
            if self.initial_input_mode == 0
                || self.initial_input_mode == INPUT_REPORT_SIMPLE_CONTROLLER_STATE
            {
                let dev = device.device().clone();
                self.set_input_mode(Link::Device(&dev), INPUT_REPORT_SIMPLE_CONTROLLER_STATE);
            }
        }

        self.enhanced_reports_hint = None;
        self.home_led_hint = None;
        self.player_led_hint = None;

        self.joystick = None;

        self.report_sensors = false;
        self.enhanced_mode = false;
        self.enhanced_mode_available = false;
    }

    fn power_info(&self) -> Option<(PowerState, i32)> {
        Some((self.power_state, self.power_percent))
    }
}

#[cfg(test)]
mod tests;
