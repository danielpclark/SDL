// Rust translation of src/joystick/hidapi/steam/controller_constants.h from
// Simple DirectMedia Layer.
// Copyright (C) 2021 Valve Corporation
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The constants of the Valve controller protocol: feature report message
//! IDs, controller attributes and settings.
//!
//! Only what the drivers use is translated. Left out: the USB PIDs and
//! scan intervals (the drivers use `usb_ids` and their own rates), the
//! digital and analog IO, device, keyboard, mouse, gamepad button and mode
//! adjust lists (used by the mouse mode upstream compiles out), the status
//! codes, string attributes, haptic pulse flags, setting ranges, dongle
//! settings and audio slots.

// FeatureReportMessageIDs: the messages exchanged between the host and
// the target (only add to this enum and never change the order)

/// `ID_CLEAR_DIGITAL_MAPPINGS`
pub(crate) const ID_CLEAR_DIGITAL_MAPPINGS: u8 = 0x81;
/// `ID_GET_ATTRIBUTES_VALUES`
pub(crate) const ID_GET_ATTRIBUTES_VALUES: u8 = 0x83;
/// `ID_SET_DEFAULT_DIGITAL_MAPPINGS`
pub(crate) const ID_SET_DEFAULT_DIGITAL_MAPPINGS: u8 = 0x85;
/// `ID_SET_SETTINGS_VALUES`
pub(crate) const ID_SET_SETTINGS_VALUES: u8 = 0x87;
/// `ID_LOAD_DEFAULT_SETTINGS`
pub(crate) const ID_LOAD_DEFAULT_SETTINGS: u8 = 0x8E;
/// `ID_ENABLE_PAIRING`
pub(crate) const ID_ENABLE_PAIRING: u8 = 0xAD;
/// `ID_DONGLE_COMMIT_DEVICE`
pub(crate) const ID_DONGLE_COMMIT_DEVICE: u8 = 0xB3;
/// `ID_DONGLE_GET_WIRELESS_STATE`
pub(crate) const ID_DONGLE_GET_WIRELESS_STATE: u8 = 0xB4;
/// `ID_TRIGGER_RUMBLE_CMD` (Deck only)
pub(crate) const ID_TRIGGER_RUMBLE_CMD: u8 = 0xEB;

// ControllerAttributes: read-only attributes of controllers (only add to
// this enum and never change the order)

/// `ATTRIB_UNIQUE_ID`
pub(crate) const ATTRIB_UNIQUE_ID: u8 = 0;
/// `ATTRIB_PRODUCT_ID`
pub(crate) const ATTRIB_PRODUCT_ID: u8 = 1;
/// `ATTRIB_CAPABILITIES` (intentional aliasing of the deprecated
/// `ATTRIB_PRODUCT_REVISION`)
pub(crate) const ATTRIB_CAPABILITIES: u8 = 2;
/// `ATTRIB_CONNECTION_INTERVAL_IN_US`
pub(crate) const ATTRIB_CONNECTION_INTERVAL_IN_US: u8 = 11;

// TrackpadDPadMode

/// `TRACKPAD_ABSOLUTE_MOUSE`
pub(crate) const TRACKPAD_ABSOLUTE_MOUSE: u16 = 0;
/// `TRACKPAD_NONE`
pub(crate) const TRACKPAD_NONE: u16 = 7;

// LizardModeState_t

/// `LIZARD_MODE_OFF`
pub(crate) const LIZARD_MODE_OFF: u16 = 0;

// ControllerSettings: read-write controller settings (only add to this
// enum and never change the order)

/// `SETTING_LEFT_TRACKPAD_MODE`
pub(crate) const SETTING_LEFT_TRACKPAD_MODE: u8 = 7;
/// `SETTING_RIGHT_TRACKPAD_MODE`
pub(crate) const SETTING_RIGHT_TRACKPAD_MODE: u8 = 8;
/// `SETTING_LIZARD_MODE`
pub(crate) const SETTING_LIZARD_MODE: u8 = 9;
/// `SETTING_SMOOTH_ABSOLUTE_MOUSE`
pub(crate) const SETTING_SMOOTH_ABSOLUTE_MOUSE: u8 = 24;
/// `SETTING_LED_USER_BRIGHTNESS`
pub(crate) const SETTING_LED_USER_BRIGHTNESS: u8 = 45;
/// `SETTING_IMU_MODE`
pub(crate) const SETTING_IMU_MODE: u8 = 48;
/// `SETTING_WIRELESS_PACKET_VERSION`
pub(crate) const SETTING_WIRELESS_PACKET_VERSION: u8 = 49;
/// `SETTING_LEFT_TRACKPAD_CLICK_PRESSURE`
pub(crate) const SETTING_LEFT_TRACKPAD_CLICK_PRESSURE: u8 = 52;
/// `SETTING_RIGHT_TRACKPAD_CLICK_PRESSURE`
pub(crate) const SETTING_RIGHT_TRACKPAD_CLICK_PRESSURE: u8 = 53;

// SettingGyroMode: bitmask that define which IMU features to enable.

/// `SETTING_GYRO_MODE_OFF`
pub(crate) const SETTING_GYRO_MODE_OFF: u16 = 0x0000;
/// `SETTING_GYRO_MODE_SEND_RAW_ACCEL`
pub(crate) const SETTING_GYRO_MODE_SEND_RAW_ACCEL: u16 = 0x0008;
/// `SETTING_GYRO_MODE_SEND_RAW_GYRO`
pub(crate) const SETTING_GYRO_MODE_SEND_RAW_GYRO: u16 = 0x0010;

// haptic_intensity_t

/// `HAPTIC_INTENSITY_SYSTEM`
pub(crate) const HAPTIC_INTENSITY_SYSTEM: u16 = 0;
