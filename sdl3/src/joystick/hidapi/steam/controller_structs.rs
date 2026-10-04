// Rust translation of src/joystick/hidapi/steam/controller_structs.h from
// Simple DirectMedia Layer.
// Copyright (C) 2021 Valve Corporation
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The wire structures of the Valve controller protocol. Upstream's packed
//! structs are read from (and written to) the little endian report bytes
//! at their offsets.
//!
//! Only what the drivers use is translated. Left out: the other feature
//! report payloads (settings and attribute reads, controller mode, haptic
//! pulses and commands), the other Triton haptic output reports, and the
//! debug, trackpad image, wireless and status input payloads.

/// `HID_FEATURE_REPORT_BYTES`
pub(crate) const HID_FEATURE_REPORT_BYTES: usize = 64;

/// `sizeof(ControllerSetting)`
pub(crate) const CONTROLLER_SETTING_SIZE: usize = 3;

/// `sizeof(ControllerAttribute)`
pub(crate) const CONTROLLER_ATTRIBUTE_SIZE: usize = 5;

fn u16_at(data: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([data[offset], data[offset + 1]])
}

fn i16_at(data: &[u8], offset: usize) -> i16 {
    u16_at(data, offset) as i16
}

fn u32_at(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ])
}

fn u64_at(data: &[u8], offset: usize) -> u64 {
    let mut bytes = [0; 8];
    bytes.copy_from_slice(&data[offset..offset + 8]);
    u64::from_le_bytes(bytes)
}

/// Write a `FeatureReportMsg` with a `setSettingsValues` payload to `msg`:
/// its `header` (`type` and `length`) and settings (`settingNum`,
/// `settingValue`).
pub(crate) fn write_set_settings_values(msg: &mut [u8], msg_type: u8, settings: &[(u8, u16)]) {
    msg[0] = msg_type;
    msg[1] = (settings.len() * CONTROLLER_SETTING_SIZE) as u8;
    for (i, &(setting_num, setting_value)) in settings.iter().enumerate() {
        let setting = &mut msg[2 + i * CONTROLLER_SETTING_SIZE..][..CONTROLLER_SETTING_SIZE];
        setting[0] = setting_num;
        setting[1..].copy_from_slice(&setting_value.to_le_bytes());
    }
}

/// A controller attribute (`ControllerAttribute`) of a `getAttributes`
/// payload.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct ControllerAttribute {
    pub(crate) attribute_tag: u8,
    pub(crate) attribute_value: u32,
}

impl ControllerAttribute {
    pub(crate) fn parse(data: &[u8]) -> ControllerAttribute {
        ControllerAttribute {
            attribute_tag: data[0],
            attribute_value: u32_at(data, 1),
        }
    }
}

/// The simple rumble payload of a feature report (`MsgSimpleRumbleCmd`).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) struct MsgSimpleRumbleCmd {
    pub(crate) rumble_type: u8,
    pub(crate) intensity: u16,
    pub(crate) left_motor_speed: u16,
    pub(crate) right_motor_speed: u16,
    pub(crate) left_gain: i8,
    pub(crate) right_gain: i8,
}

impl MsgSimpleRumbleCmd {
    /// Write the payload to `payload`.
    pub(crate) fn write(&self, payload: &mut [u8]) {
        payload[0] = self.rumble_type;
        payload[1..3].copy_from_slice(&self.intensity.to_le_bytes());
        payload[3..5].copy_from_slice(&self.left_motor_speed.to_le_bytes());
        payload[5..7].copy_from_slice(&self.right_motor_speed.to_le_bytes());
        payload[7] = self.left_gain as u8;
        payload[8] = self.right_gain as u8;
    }
}

// Triton and derivatives utilize output reports for haptic commands. This is a
// snapshot from Nov 2024 -- things may change.

/// `HID_RUMBLE_OUTPUT_REPORT_BYTES` (including the report ID)
pub(crate) const HID_RUMBLE_OUTPUT_REPORT_BYTES: usize = 10;

/// `ID_OUT_REPORT_HAPTIC_RUMBLE` (`ValveTritonOutReportMessageIDs`)
pub(crate) const ID_OUT_REPORT_HAPTIC_RUMBLE: u8 = 0x80;

/// A motor of [`MsgHapticRumble`].
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) struct HapticRumbleMotor {
    pub(crate) speed: u16,
    pub(crate) gain: i8,
}

/// The rumble output report payload of Triton (`MsgHapticRumble`).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) struct MsgHapticRumble {
    pub(crate) haptic_type: u8,
    pub(crate) intensity: u16,
    pub(crate) left: HapticRumbleMotor,
    pub(crate) right: HapticRumbleMotor,
}

impl MsgHapticRumble {
    /// The output report (`OutputReportMsg`) carrying the payload.
    pub(crate) fn output_report(&self) -> [u8; HID_RUMBLE_OUTPUT_REPORT_BYTES] {
        let mut report = [0; HID_RUMBLE_OUTPUT_REPORT_BYTES];
        report[0] = ID_OUT_REPORT_HAPTIC_RUMBLE;
        report[1] = self.haptic_type;
        report[2..4].copy_from_slice(&self.intensity.to_le_bytes());
        report[4..6].copy_from_slice(&self.left.speed.to_le_bytes());
        report[6] = self.left.gain as u8;
        report[7..9].copy_from_slice(&self.right.speed.to_le_bytes());
        report[9] = self.right.gain as u8;
        report
    }
}

// Roll this version forward anytime that you are breaking compatibility of existing
// message types within ValveInReport_t or the header itself.  Hopefully this should
// be super rare and instead you should just add new message payloads to the union,
// or just add fields to the end of existing payload structs which is expected to be
// safe in all code consuming these as they should just consume/copy up to the prior size
// they were aware of when processing.

/// `k_ValveInReportMsgVersion`
pub(crate) const K_VALVE_IN_REPORT_MSG_VERSION: u16 = 0x01;

// ValveInReportMessageIDs

/// `ID_CONTROLLER_STATE`
pub(crate) const ID_CONTROLLER_STATE: u8 = 1;
/// `ID_CONTROLLER_BLE_STATE`
pub(crate) const ID_CONTROLLER_BLE_STATE: u8 = 7;
/// `ID_CONTROLLER_DECK_STATE`
pub(crate) const ID_CONTROLLER_DECK_STATE: u8 = 9;

/// `sizeof(ValveInReportHeader_t)`: the payload of a `ValveInReport_t`
/// follows it.
pub(crate) const VALVE_IN_REPORT_HEADER_SIZE: usize = 4;

/// The header of a Valve input report (`ValveInReportHeader_t`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct ValveInReportHeader {
    pub(crate) report_version: u16,
    pub(crate) report_type: u8,
    pub(crate) length: u8,
}

impl ValveInReportHeader {
    pub(crate) fn parse(data: &[u8]) -> ValveInReportHeader {
        ValveInReportHeader {
            report_version: u16_at(data, 0),
            report_type: data[2],
            length: data[3],
        }
    }
}

/// The state payload (`ValveControllerStatePacket_t`).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) struct ValveControllerStatePacket {
    /// If packet num matches that on your prior call, then the controller state hasn't been changed since
    /// your last call and there is no need to process it
    pub(crate) packet_num: u32,
    /// Button bitmask and trigger data (`ButtonTriggerData.ulButtons`)
    pub(crate) buttons: u64,
    pub(crate) left_pad_x: i16,
    pub(crate) left_pad_y: i16,
    pub(crate) right_pad_x: i16,
    pub(crate) right_pad_y: i16,
    /// This is redundant, packed above, but still sent over wired
    pub(crate) trigger_l: u16,
    pub(crate) trigger_r: u16,
    // FIXME figure out a way to grab this stuff over wireless
    pub(crate) accel_x: i16,
    pub(crate) accel_y: i16,
    pub(crate) accel_z: i16,
    pub(crate) gyro_x: i16,
    pub(crate) gyro_y: i16,
    pub(crate) gyro_z: i16,
    pub(crate) gyro_quat_w: i16,
    pub(crate) gyro_quat_x: i16,
    pub(crate) gyro_quat_y: i16,
    pub(crate) gyro_quat_z: i16,
}

impl ValveControllerStatePacket {
    pub(crate) fn parse(data: &[u8]) -> ValveControllerStatePacket {
        ValveControllerStatePacket {
            packet_num: u32_at(data, 0),
            buttons: u64_at(data, 4),
            left_pad_x: i16_at(data, 12),
            left_pad_y: i16_at(data, 14),
            right_pad_x: i16_at(data, 16),
            right_pad_y: i16_at(data, 18),
            trigger_l: u16_at(data, 20),
            trigger_r: u16_at(data, 22),
            accel_x: i16_at(data, 24),
            accel_y: i16_at(data, 26),
            accel_z: i16_at(data, 28),
            gyro_x: i16_at(data, 30),
            gyro_y: i16_at(data, 32),
            gyro_z: i16_at(data, 34),
            gyro_quat_w: i16_at(data, 36),
            gyro_quat_x: i16_at(data, 38),
            gyro_quat_y: i16_at(data, 40),
            gyro_quat_z: i16_at(data, 42),
        }
    }

    /// The left trigger, packed in the button bits
    /// (`ButtonTriggerData.Triggers.nLeft`).
    pub(crate) fn trigger_left(&self) -> u8 {
        (self.buttons >> 24) as u8
    }

    /// The right trigger, packed in the button bits
    /// (`ButtonTriggerData.Triggers.nRight`).
    pub(crate) fn trigger_right(&self) -> u8 {
        (self.buttons >> 32) as u8
    }
}

/// The BLE state payload (`ValveControllerBLEStatePacket_t`). This has to
/// be re-formatted from the normal state because BLE controller shows up
/// as a HID device and we don't want to send all the optional parts of
/// the message. Its fields up to the pads are those of
/// [`ValveControllerStatePacket`].
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) struct ValveControllerBleStatePacket {
    /// This mimics how the dongle reconstitutes HID packets, there will be 0-4 shorts depending on gyro mode
    pub(crate) gyro_data_type: u8,
    pub(crate) gyro: [i16; 4],
}

impl ValveControllerBleStatePacket {
    pub(crate) fn parse(data: &[u8]) -> ValveControllerBleStatePacket {
        ValveControllerBleStatePacket {
            gyro_data_type: data[20],
            gyro: std::array::from_fn(|i| i16_at(data, 21 + i * 2)),
        }
    }
}

/// The Steam Deck state payload (`SteamDeckStatePacket_t`).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) struct SteamDeckStatePacket {
    /// If packet num matches that on your prior call, then the controller
    /// state hasn't been changed since your last call and there is no need to
    /// process it
    pub(crate) packet_num: u32,
    /// Button bitmask and trigger data (`ulButtons`, the low half
    /// `ulButtonsL` and the high half `ulButtonsH`)
    pub(crate) buttons: u64,
    pub(crate) left_pad_x: i16,
    pub(crate) left_pad_y: i16,
    pub(crate) right_pad_x: i16,
    pub(crate) right_pad_y: i16,
    pub(crate) accel_x: i16,
    pub(crate) accel_y: i16,
    pub(crate) accel_z: i16,
    pub(crate) gyro_x: i16,
    pub(crate) gyro_y: i16,
    pub(crate) gyro_z: i16,
    pub(crate) gyro_quat_w: i16,
    pub(crate) gyro_quat_x: i16,
    pub(crate) gyro_quat_y: i16,
    pub(crate) gyro_quat_z: i16,
    /// Uncalibrated trigger values
    pub(crate) trigger_raw_l: u16,
    pub(crate) trigger_raw_r: u16,
    pub(crate) left_stick_x: i16,
    pub(crate) left_stick_y: i16,
    pub(crate) right_stick_x: i16,
    pub(crate) right_stick_y: i16,
    /// Touchpad pressures
    pub(crate) pressure_pad_left: u16,
    pub(crate) pressure_pad_right: u16,
}

impl SteamDeckStatePacket {
    pub(crate) fn parse(data: &[u8]) -> SteamDeckStatePacket {
        SteamDeckStatePacket {
            packet_num: u32_at(data, 0),
            buttons: u64_at(data, 4),
            left_pad_x: i16_at(data, 12),
            left_pad_y: i16_at(data, 14),
            right_pad_x: i16_at(data, 16),
            right_pad_y: i16_at(data, 18),
            accel_x: i16_at(data, 20),
            accel_y: i16_at(data, 22),
            accel_z: i16_at(data, 24),
            gyro_x: i16_at(data, 26),
            gyro_y: i16_at(data, 28),
            gyro_z: i16_at(data, 30),
            gyro_quat_w: i16_at(data, 32),
            gyro_quat_x: i16_at(data, 34),
            gyro_quat_y: i16_at(data, 36),
            gyro_quat_z: i16_at(data, 38),
            trigger_raw_l: u16_at(data, 40),
            trigger_raw_r: u16_at(data, 42),
            left_stick_x: i16_at(data, 44),
            left_stick_y: i16_at(data, 46),
            right_stick_x: i16_at(data, 48),
            right_stick_y: i16_at(data, 50),
            pressure_pad_left: u16_at(data, 52),
            pressure_pad_right: u16_at(data, 54),
        }
    }

    /// `ulButtonsL`
    pub(crate) fn buttons_l(&self) -> u32 {
        self.buttons as u32
    }

    /// `ulButtonsH`
    pub(crate) fn buttons_h(&self) -> u32 {
        (self.buttons >> 32) as u32
    }
}

// EBLEPacketReportNums

/// `k_EBLEReportState`
pub(crate) const K_EBLE_REPORT_STATE: u8 = 4;

// EBLEOptionDataChunksBitmask: enumeration of data chunks in BLE state packets

// First byte upper nibble
/// `k_EBLEButtonChunk1`
pub(crate) const K_EBLE_BUTTON_CHUNK1: u32 = 0x10;
/// `k_EBLEButtonChunk2`
pub(crate) const K_EBLE_BUTTON_CHUNK2: u32 = 0x20;
/// `k_EBLEButtonChunk3`
pub(crate) const K_EBLE_BUTTON_CHUNK3: u32 = 0x40;
/// `k_EBLELeftJoystickChunk`
pub(crate) const K_EBLE_LEFT_JOYSTICK_CHUNK: u32 = 0x80;

// Second full byte
/// `k_EBLELeftTrackpadChunk`
pub(crate) const K_EBLE_LEFT_TRACKPAD_CHUNK: u32 = 0x100;
/// `k_EBLERightTrackpadChunk`
pub(crate) const K_EBLE_RIGHT_TRACKPAD_CHUNK: u32 = 0x200;
/// `k_EBLEIMUAccelChunk`
pub(crate) const K_EBLE_IMU_ACCEL_CHUNK: u32 = 0x400;
/// `k_EBLEIMUGyroChunk`
pub(crate) const K_EBLE_IMU_GYRO_CHUNK: u32 = 0x800;
/// `k_EBLEIMUQuatChunk`
pub(crate) const K_EBLE_IMU_QUAT_CHUNK: u32 = 0x1000;

// Triton and derivatives do not use the ValveInReport_t structure

// ETritonReportIDTypes

/// `ID_TRITON_CONTROLLER_STATE`
pub(crate) const ID_TRITON_CONTROLLER_STATE: u8 = 0x42;
/// `ID_TRITON_BATTERY_STATUS`
pub(crate) const ID_TRITON_BATTERY_STATUS: u8 = 0x43;
/// `ID_TRITON_CONTROLLER_STATE_BLE`
pub(crate) const ID_TRITON_CONTROLLER_STATE_BLE: u8 = 0x45;
/// `ID_TRITON_WIRELESS_STATUS_X`
pub(crate) const ID_TRITON_WIRELESS_STATUS_X: u8 = 0x46;
/// `ID_TRITON_CONTROLLER_STATE_TIMESTAMP`
pub(crate) const ID_TRITON_CONTROLLER_STATE_TIMESTAMP: u8 = 0x47;
/// `ID_TRITON_WIRELESS_STATUS`
pub(crate) const ID_TRITON_WIRELESS_STATUS: u8 = 0x79;

// ETritonWirelessState

/// `k_ETritonWirelessStateDisconnect`
pub(crate) const K_ETRITON_WIRELESS_STATE_DISCONNECT: u8 = 1;
/// `k_ETritonWirelessStateConnect`
pub(crate) const K_ETRITON_WIRELESS_STATE_CONNECT: u8 = 2;

/// `sizeof(TritonMTUNoQuat_t)`
pub(crate) const TRITON_MTU_NO_QUAT_SIZE: usize = 45;
/// `sizeof(TritonMTUNoQuat32TS_t)`
pub(crate) const TRITON_MTU_NO_QUAT_32TS_SIZE: usize = 45;

/// The IMU part of [`TritonMtuNoQuat`] (`TritonMTUIMUNoQuat_t`), or of
/// [`TritonMtuNoQuat32Ts`] (`TritonMTUIMUNoQuat32usTS_t`, with a 16-bit
/// timestamp).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) struct TritonMtuImu<T> {
    pub(crate) timestamp: T,
    pub(crate) accel_x: i16,
    pub(crate) accel_y: i16,
    pub(crate) accel_z: i16,
    pub(crate) gyro_x: i16,
    pub(crate) gyro_y: i16,
    pub(crate) gyro_z: i16,
}

impl<T> TritonMtuImu<T> {
    /// The IMU values from `data`, past the timestamp.
    fn parse(timestamp: T, data: &[u8]) -> TritonMtuImu<T> {
        TritonMtuImu {
            timestamp,
            accel_x: i16_at(data, 0),
            accel_y: i16_at(data, 2),
            accel_z: i16_at(data, 4),
            gyro_x: i16_at(data, 6),
            gyro_y: i16_at(data, 8),
            gyro_z: i16_at(data, 10),
        }
    }
}

/// A Triton state report without the quaternion (`TritonMTUNoQuat_t`).
/// The newer state reports are identical until the touchpads.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) struct TritonMtuNoQuat {
    pub(crate) seq_num: u8,
    pub(crate) buttons: u32,
    pub(crate) trigger_left: i16,
    pub(crate) trigger_right: i16,
    pub(crate) left_stick_x: i16,
    pub(crate) left_stick_y: i16,
    pub(crate) right_stick_x: i16,
    pub(crate) right_stick_y: i16,
    pub(crate) left_pad_x: i16,
    pub(crate) left_pad_y: i16,
    pub(crate) pressure_left: u16,
    pub(crate) right_pad_x: i16,
    pub(crate) right_pad_y: i16,
    pub(crate) pressure_right: u16,
    pub(crate) imu: TritonMtuImu<u32>,
}

impl TritonMtuNoQuat {
    pub(crate) fn parse(data: &[u8]) -> TritonMtuNoQuat {
        TritonMtuNoQuat {
            seq_num: data[0],
            buttons: u32_at(data, 1),
            trigger_left: i16_at(data, 5),
            trigger_right: i16_at(data, 7),
            left_stick_x: i16_at(data, 9),
            left_stick_y: i16_at(data, 11),
            right_stick_x: i16_at(data, 13),
            right_stick_y: i16_at(data, 15),
            left_pad_x: i16_at(data, 17),
            left_pad_y: i16_at(data, 19),
            pressure_left: u16_at(data, 21),
            right_pad_x: i16_at(data, 23),
            right_pad_y: i16_at(data, 25),
            pressure_right: u16_at(data, 27),
            imu: TritonMtuImu::parse(u32_at(data, 29), &data[33..]),
        }
    }
}

/// The Ibex state report (`TritonMTUNoQuat32TS_t`): it adds a timestamp to
/// the trackpad sampling and reduces the size of the IMU timestamp.
/// Timestamps are now 16 bits.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) struct TritonMtuNoQuat32Ts {
    pub(crate) seq_num: u8,
    pub(crate) buttons: u32,
    pub(crate) trigger_left: i16,
    pub(crate) trigger_right: i16,
    pub(crate) left_stick_x: i16,
    pub(crate) left_stick_y: i16,
    pub(crate) right_stick_x: i16,
    pub(crate) right_stick_y: i16,
    pub(crate) trackpad_timestamp: u16,
    pub(crate) left_pad_x: i16,
    pub(crate) left_pad_y: i16,
    pub(crate) pressure_left: u16,
    pub(crate) right_pad_x: i16,
    pub(crate) right_pad_y: i16,
    pub(crate) pressure_right: u16,
    pub(crate) imu: TritonMtuImu<u16>,
}

impl TritonMtuNoQuat32Ts {
    pub(crate) fn parse(data: &[u8]) -> TritonMtuNoQuat32Ts {
        TritonMtuNoQuat32Ts {
            seq_num: data[0],
            buttons: u32_at(data, 1),
            trigger_left: i16_at(data, 5),
            trigger_right: i16_at(data, 7),
            left_stick_x: i16_at(data, 9),
            left_stick_y: i16_at(data, 11),
            right_stick_x: i16_at(data, 13),
            right_stick_y: i16_at(data, 15),
            trackpad_timestamp: u16_at(data, 17),
            left_pad_x: i16_at(data, 19),
            left_pad_y: i16_at(data, 21),
            pressure_left: u16_at(data, 23),
            right_pad_x: i16_at(data, 25),
            right_pad_y: i16_at(data, 27),
            pressure_right: u16_at(data, 29),
            imu: TritonMtuImu::parse(u16_at(data, 31), &data[33..]),
        }
    }
}

// EChargeState

/// `k_EChargeStateDischarging`
pub(crate) const K_ECHARGE_STATE_DISCHARGING: u8 = 1;
/// `k_EChargeStateCharging`
pub(crate) const K_ECHARGE_STATE_CHARGING: u8 = 2;
/// `k_EChargeStateChargingDone`
pub(crate) const K_ECHARGE_STATE_CHARGING_DONE: u8 = 4;

/// `sizeof(TritonBatteryStatus_t)`
pub(crate) const TRITON_BATTERY_STATUS_SIZE: usize = 14;

/// A Triton battery report (`TritonBatteryStatus_t`).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) struct TritonBatteryStatus {
    /// `EChargeState`
    pub(crate) charge_state: u8,
    pub(crate) battery_level: u8,
    pub(crate) battery_voltage: u16,
    pub(crate) system_voltage: u16,
    pub(crate) input_voltage: u16,
    pub(crate) current: u16,
    pub(crate) input_current: u16,
    pub(crate) temperature: u16,
}

impl TritonBatteryStatus {
    pub(crate) fn parse(data: &[u8]) -> TritonBatteryStatus {
        TritonBatteryStatus {
            charge_state: data[0],
            battery_level: data[1],
            battery_voltage: u16_at(data, 2),
            system_voltage: u16_at(data, 4),
            input_voltage: u16_at(data, 6),
            current: u16_at(data, 8),
            input_current: u16_at(data, 10),
            temperature: u16_at(data, 12),
        }
    }
}

/// `sizeof(TritonWirelessStatus_t)`
pub(crate) const TRITON_WIRELESS_STATUS_SIZE: usize = 1;
