// Rust translation of src/joystick/hidapi/SDL_hidapi_xboxone.c from Simple
// DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Xbox One and Xbox Series controller driver: the Game Input Protocol
//! (GIP) over USB, the Bluetooth reports, and on Linux the reports the
//! kernel describes with a synthesized report descriptor.
//!
//! GIP protocol handling adapted under the Zlib license with permission
//! from @medusalix: <https://github.com/medusalix/xone/blob/master/bus/protocol.h>
//! and <https://github.com/medusalix/xone/blob/master/bus/protocol.c>
//!
//! Not translated: the macOS GCController check, whose backend isn't
//! translated.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use super::report_descriptor::{make_usage, read_report_data, ReportDescriptor};
use super::rumble::{lock_rumble, send_rumble};
use super::{
    load16, DeviceCtx, DriverContext, DriverImpl, HidapiDevice, HintWatch, JoystickCaps,
    SDL_HIDAPI_DEFAULT, USB_PACKET_LENGTH,
};
use crate::error::{Error, Result};
use crate::events::JoystickID;
use crate::hints;
use crate::joystick::gamepad::{GamepadAxis, GamepadButton, GamepadType};
use crate::joystick::usb_ids::*;
use crate::joystick::{
    is_joystick_xbox_one_elite, is_joystick_xbox_series_x, JoystickData, HAT_CENTERED, HAT_DOWN,
    HAT_LEFT, HAT_LEFTDOWN, HAT_LEFTUP, HAT_RIGHT, HAT_RIGHTDOWN, HAT_RIGHTUP, HAT_UP,
};
use crate::power::PowerState;

/// `XBOX_ONE_DRIVER_ACTIVE`: Windows has a driver doing the GIP
/// identification, startup and acknowledgements.
const XBOX_ONE_DRIVER_ACTIVE: bool = cfg!(windows);

const CONTROLLER_IDENTIFY_TIMEOUT_MS: u64 = 100;
const CONTROLLER_PREPARE_INPUT_TIMEOUT_MS: u64 = 50;

/// `SDL_GAMEPAD_BUTTON_XBOX_SHARE_BUTTON`
const SDL_GAMEPAD_BUTTON_XBOX_SHARE_BUTTON: u8 = 11;

// Power on
const XBOX_INIT_POWER_ON: &[u8] = &[0x05, 0x20, 0x00, 0x01, 0x00];
// Enable LED
const XBOX_INIT_ENABLE_LED: &[u8] = &[0x0A, 0x20, 0x00, 0x03, 0x00, 0x01, 0x14];
// This controller passed security check
const XBOX_INIT_SECURITY_PASSED: &[u8] = &[0x06, 0x20, 0x00, 0x02, 0x01, 0x00];
// Some PowerA controllers need to actually start the rumble motors
const XBOX_INIT_POWERA_RUMBLE: &[u8] = &[
    0x09, 0x00, 0x00, 0x09, 0x00, 0x0F, 0x00, 0x00, 0x1D, 0x1D, 0xFF, 0x00, 0x00,
];
// Setup rumble (not needed for Microsoft controllers, but it doesn't hurt)
const XBOX_INIT_RUMBLE: &[u8] = &[
    0x09, 0x00, 0x00, 0x09, 0x00, 0x0F, 0x00, 0x00, 0x00, 0x00, 0xFF, 0x00, 0xEB,
];

/// The selection of init packets that a gamepad will be sent on init
/// *and* the order in which they will be sent. The correct sequence number
/// will be added when the packet is going to be sent. Translation of
/// `SDL_DriverXboxOne_InitPacket`.
struct InitPacket {
    vendor_id: u16,
    product_id: u16,
    data: &'static [u8],
}

/// Translation of `xboxone_init_packets`.
const XBOXONE_INIT_PACKETS: [InitPacket; 7] = [
    InitPacket {
        vendor_id: 0x0000,
        product_id: 0x0000,
        data: XBOX_INIT_POWER_ON,
    },
    InitPacket {
        vendor_id: 0x0000,
        product_id: 0x0000,
        data: XBOX_INIT_ENABLE_LED,
    },
    InitPacket {
        vendor_id: 0x0000,
        product_id: 0x0000,
        data: XBOX_INIT_SECURITY_PASSED,
    },
    InitPacket {
        vendor_id: 0x24c6,
        product_id: 0x541a,
        data: XBOX_INIT_POWERA_RUMBLE,
    },
    InitPacket {
        vendor_id: 0x24c6,
        product_id: 0x542a,
        data: XBOX_INIT_POWERA_RUMBLE,
    },
    InitPacket {
        vendor_id: 0x24c6,
        product_id: 0x543a,
        data: XBOX_INIT_POWERA_RUMBLE,
    },
    InitPacket {
        vendor_id: 0x0000,
        product_id: 0x0000,
        data: XBOX_INIT_RUMBLE,
    },
];

/// Translation of `SDL_XboxOneInitState`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Default)]
enum InitState {
    #[default]
    Announced,
    Identifying,
    Startup,
    PrepareInput,
    Complete,
}

/// Translation of `SDL_XboxOneRumbleState`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum RumbleState {
    #[default]
    Idle,
    Queued,
    Busy,
}

/// Translation of `SDL_DriverXboxOne_Context`.
#[derive(Debug)]
struct XboxOneContext {
    vendor_id: u16,
    product_id: u16,
    init_state: InitState,
    start_time: u64,
    sequence: u8,
    send_time: u64,
    has_guide_packet: bool,
    has_color_led: bool,
    has_paddles: bool,
    has_unmapped_state: bool,
    has_trigger_rumble: bool,
    has_share_button: bool,
    has_separate_back_button: bool,
    has_separate_guide_button: bool,
    last_paddle_state: u8,
    low_frequency_rumble: u8,
    high_frequency_rumble: u8,
    left_trigger_rumble: u8,
    right_trigger_rumble: u8,
    rumble_state: RumbleState,
    /// Set by the rumble thread when a rumble packet was sent
    /// (`HIDAPI_DriverXboxOne_RumbleSent()`).
    rumble_time: Arc<AtomicU64>,
    rumble_pending: bool,
    descriptor: Option<ReportDescriptor>,
    last_buttons: u32,
    last_state: [u8; USB_PACKET_LENGTH],
    chunk_buffer: Option<Vec<u8>>,
    chunk_length: u32,
    /// The `SDL_HomeLEDHintChanged()` callback.
    home_led_hint: Option<HintWatch>,
}

impl Default for XboxOneContext {
    fn default() -> Self {
        XboxOneContext {
            vendor_id: 0,
            product_id: 0,
            init_state: InitState::Announced,
            start_time: 0,
            sequence: 0,
            send_time: 0,
            has_guide_packet: false,
            has_color_led: false,
            has_paddles: false,
            has_unmapped_state: false,
            has_trigger_rumble: false,
            has_share_button: false,
            has_separate_back_button: false,
            has_separate_guide_button: false,
            last_paddle_state: 0,
            low_frequency_rumble: 0,
            high_frequency_rumble: 0,
            left_trigger_rumble: 0,
            right_trigger_rumble: 0,
            rumble_state: RumbleState::Idle,
            rumble_time: Arc::new(AtomicU64::new(0)),
            rumble_pending: false,
            descriptor: None,
            last_buttons: 0,
            last_state: [0; USB_PACKET_LENGTH],
            chunk_buffer: None,
            chunk_length: 0,
            home_led_hint: None,
        }
    }
}

/// Translation of `ControllerHasColorLED()`.
fn controller_has_color_led(vendor_id: u16, product_id: u16) -> bool {
    vendor_id == USB_VENDOR_MICROSOFT && product_id == USB_PRODUCT_XBOX_ONE_ELITE_SERIES_2
}

/// Translation of `ControllerHasPaddles()`.
fn controller_has_paddles(vendor_id: u16, product_id: u16) -> bool {
    is_joystick_xbox_one_elite(vendor_id, product_id)
}

/// Translation of `ControllerHasTriggerRumble()`.
fn controller_has_trigger_rumble(vendor_id: u16, _product_id: u16) -> bool {
    // All the Microsoft Xbox One controllers have trigger rumble
    if vendor_id == USB_VENDOR_MICROSOFT {
        return true;
    }

    /* It turns out other controllers a mixed bag as to whether they support
      trigger rumble or not, and when they do it's often a buzz rather than
      the vibration of the Microsoft trigger rumble, so for now just pretend
      that it is not available.
    */
    false
}

/// Translation of `ControllerHasShareButton()`.
fn controller_has_share_button(vendor_id: u16, product_id: u16) -> bool {
    is_joystick_xbox_series_x(vendor_id, product_id)
}

/// Translation of `GetHomeLEDBrightness()`.
fn get_home_led_brightness(hint: Option<&str>) -> i32 {
    const MAX_VALUE: i32 = 50;
    let mut value = 20;

    if let Some(hint) = hint.filter(|h| !h.is_empty()) {
        if hint.contains('.') {
            value = (f64::from(MAX_VALUE) * crate::stdlib::string::strtod(hint).0) as i32;
        } else if !hints::string_to_bool(Some(hint), true) {
            value = 0;
        }
    }
    value
}

/// Translation of `SetHomeLED()`.
fn set_home_led(device: &Arc<HidapiDevice>, value: i32) {
    let mut led_packet = [0x0A, 0x20, 0x00, 0x03, 0x00, 0x00, 0x00];

    if value > 0 {
        led_packet[5] = 0x01;
        led_packet[6] = value as u8;
    }
    let _ = send_rumble(device, &led_packet);
}

/// Translation of `ControllerSendsAnnouncement()`.
fn controller_sends_announcement(vendor_id: u16, _product_id: u16) -> bool {
    // The PDP Rock Candy (PID 0x0246) and PowerA Fusion Pro 4 (PID 0x400b)
    // don't send the announce packet on Linux for some reason.
    //
    // Just to be safe and cover future products, we'll always send the startup
    // protocol sequence for PDP and PowerA controllers
    !(vendor_id == USB_VENDOR_PDP
        || vendor_id == USB_VENDOR_POWERA
        || vendor_id == USB_VENDOR_POWERA_ALT)
}

// GIP protocol

const GIP_HEADER_MIN_LENGTH: usize = 3;

// Internal commands
const GIP_CMD_ACKNOWLEDGE: u8 = 0x01;
const GIP_CMD_ANNOUNCE: u8 = 0x02;
const GIP_CMD_STATUS: u8 = 0x03;
const GIP_CMD_IDENTIFY: u8 = 0x04;
const GIP_CMD_POWER: u8 = 0x05;
const GIP_CMD_AUTHENTICATE: u8 = 0x06;
const GIP_CMD_VIRTUAL_KEY: u8 = 0x07;
const GIP_CMD_SERIAL_NUMBER: u8 = 0x1E;

// External commands
const GIP_CMD_UNMAPPED_STATE: u8 = 0x0C;
const GIP_CMD_INPUT: u8 = 0x20;

// Header option flags
const GIP_OPT_ACKNOWLEDGE: u8 = 0x10;
const GIP_OPT_INTERNAL: u8 = 0x20;
const GIP_OPT_CHUNK_START: u8 = 0x40;
const GIP_OPT_CHUNK: u8 = 0x80;

/// Translation of `struct gip_header`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
struct GipHeader {
    command: u8,
    options: u8,
    sequence: u8,
    packet_length: u32,
    chunk_offset: u32,
}

/// `sizeof(struct gip_pkt_acknowledge)` (packed)
const GIP_PKT_ACKNOWLEDGE_SIZE: u32 = 9;

/// Translation of `EncodeVariableInt()`: the bytes written to `buf`
/// (upstream's count, which is one more than the bytes written for values
/// of 28 bits or more).
fn encode_variable_int(buf: &mut [u8], mut val: u32) -> usize {
    let mut i = 0;
    while i < size_of::<u32>() {
        buf[i] = val as u8;
        if val > 0x7F {
            buf[i] |= 0x80;
        }

        val >>= 7;
        if val == 0 {
            break;
        }
        i += 1;
    }
    i + 1
}

/// Translation of `DecodeVariableInt()`: the value and the bytes it took.
fn decode_variable_int(data: &[u8]) -> (u32, usize) {
    let mut val = 0u32;
    let mut i = 0;
    while i < size_of::<u32>() && i < data.len() {
        val |= u32::from(data[i] & 0x7F) << (i * 7);

        if data[i] & 0x80 == 0 {
            break;
        }
        i += 1;
    }
    (val, i + 1)
}

/// Translation of `HIDAPI_GIP_GetActualHeaderLength()`.
fn gip_get_actual_header_length(hdr: &GipHeader) -> usize {
    let mut pkt_len = hdr.packet_length;
    let mut chunk_offset = hdr.chunk_offset;
    let mut len = GIP_HEADER_MIN_LENGTH;

    loop {
        len += 1;
        pkt_len >>= 7;
        if pkt_len == 0 {
            break;
        }
    }

    if hdr.options & GIP_OPT_CHUNK != 0 {
        while chunk_offset != 0 {
            len += 1;
            chunk_offset >>= 7;
        }
    }

    len
}

/// Translation of `HIDAPI_GIP_GetHeaderLength()`.
fn gip_get_header_length(hdr: &GipHeader) -> usize {
    let len = gip_get_actual_header_length(hdr);

    // Header length must be even
    len + (len % 2)
}

/// Translation of `HIDAPI_GIP_EncodeHeader()`.
fn gip_encode_header(hdr: &GipHeader, buf: &mut [u8]) {
    let mut hdr_len = 0;

    buf[hdr_len] = hdr.command;
    hdr_len += 1;
    buf[hdr_len] = hdr.options;
    hdr_len += 1;
    buf[hdr_len] = hdr.sequence;
    hdr_len += 1;

    hdr_len += encode_variable_int(&mut buf[hdr_len..], hdr.packet_length);

    // Header length must be even
    if !gip_get_actual_header_length(hdr).is_multiple_of(2) {
        buf[hdr_len - 1] |= 0x80;
        buf[hdr_len] = 0;
        hdr_len += 1;
    }

    if hdr.options & GIP_OPT_CHUNK != 0 {
        encode_variable_int(&mut buf[hdr_len..], hdr.chunk_offset);
    }
}

/// Translation of `HIDAPI_GIP_DecodeHeader()`: the header and its length.
fn gip_decode_header(data: &[u8]) -> (GipHeader, usize) {
    let mut hdr = GipHeader {
        command: data[0],
        options: data[1],
        sequence: data[2],
        ..GipHeader::default()
    };
    let mut hdr_len = 3;

    let (packet_length, len) = decode_variable_int(data.get(hdr_len..).unwrap_or(&[]));
    hdr.packet_length = packet_length;
    hdr_len += len;

    if hdr.options & GIP_OPT_CHUNK != 0 {
        let (chunk_offset, len) = decode_variable_int(data.get(hdr_len..).unwrap_or(&[]));
        hdr.chunk_offset = chunk_offset;
        hdr_len += len;
    }
    (hdr, hdr_len)
}

/// A packet for the handlers that read fixed offsets: the bytes of `data`
/// in a buffer of at least a USB packet.
// FIXME (upstream): the handlers read fixed offsets past the end of the
// packet (into the read buffer, or past a short reassembled chunk); here
// the bytes past the data read as 0.
fn padded(data: &[u8]) -> Vec<u8> {
    let mut packet = data.to_vec();
    if packet.len() < USB_PACKET_LENGTH {
        packet.resize(USB_PACKET_LENGTH, 0);
    }
    packet
}

/// Translation of `HIDAPI_DriverXboxOne_HandleBatteryState()`.
fn handle_battery_state(device: &mut DeviceCtx<'_>, joystick: JoystickID, flags: u32) {
    let on_usb = ((flags & 0x0C) >> 2) == 0;

    // Mapped percentage value from:
    // https://learn.microsoft.com/en-us/gaming/gdk/_content/gc/reference/input/gameinput/interfaces/igameinputdevice/methods/igameinputdevice_getbatterystate
    let percent = match flags & 0x03 {
        0 => 10,
        1 => 40,
        2 => 70,
        _ => 100,
    };
    let state = if on_usb {
        PowerState::Charging
    } else {
        PowerState::OnBattery
    };
    device.send_power_info(joystick, state, percent);
}

/// Translation of `HandleDescriptorAxis()`.
fn handle_descriptor_axis(
    device: &mut DeviceCtx<'_>,
    timestamp: u64,
    joystick: JoystickID,
    axis: GamepadAxis,
    value: u32,
) {
    let axis_value = (value as i32 - 0x8000) as i16;
    device.send_axis(timestamp, joystick, axis as u8, axis_value);
}

/// Translation of `HandleDescriptorTrigger()`.
fn handle_descriptor_trigger(
    device: &mut DeviceCtx<'_>,
    timestamp: u64,
    joystick: JoystickID,
    axis: GamepadAxis,
    value: u32,
) {
    let mut axis_value = ((value as i32) * 64 - 32768) as i16;
    if axis_value == 32704 {
        axis_value = 32767;
    }
    device.send_axis(timestamp, joystick, axis as u8, axis_value);
}

/// The hat of a hat switch value (1 is up, clockwise).
fn hat_of(value: u32) -> u8 {
    match value {
        1 => HAT_UP,
        2 => HAT_RIGHTUP,
        3 => HAT_RIGHT,
        4 => HAT_RIGHTDOWN,
        5 => HAT_DOWN,
        6 => HAT_LEFTDOWN,
        7 => HAT_LEFT,
        8 => HAT_LEFTUP,
        _ => HAT_CENTERED,
    }
}

impl XboxOneContext {
    /// Translation of `SetInitState()`.
    fn set_init_state(&mut self, state: InitState) {
        self.init_state = state;
    }

    /// Translation of `GetNextPacketSequence()`.
    fn get_next_packet_sequence(&mut self) -> u8 {
        self.sequence = self.sequence.wrapping_add(1);
        if self.sequence == 0 {
            self.sequence = 1;
        }
        self.sequence
    }

    /// Translation of `SendProtocolPacket()`.
    fn send_protocol_packet(&mut self, device: &DeviceCtx<'_>, data: &[u8]) -> bool {
        self.send_time = crate::timer::ticks_ms();

        let Ok(lock) = lock_rumble() else {
            return false;
        };
        lock.send_and_unlock(device.device(), data).ok() == Some(data.len())
    }

    /// Translation of `SendIdentificationRequest()`.
    fn send_identification_request(&mut self, device: &DeviceCtx<'_>) -> Result<()> {
        // Request identification, sent in response to announce packet
        let mut packet = [0x04, 0x20, 0x00, 0x00];

        packet[2] = self.get_next_packet_sequence();

        if !self.send_protocol_packet(device, &packet) {
            return Err(Error::new("Couldn't send identification request packet"));
        }
        Ok(())
    }

    /// Translation of `SendControllerStartup()`.
    fn send_controller_startup(&mut self, device: &DeviceCtx<'_>) -> Result<()> {
        let vendor_id = self.vendor_id;
        let product_id = self.product_id;
        let mut init_packet = [0u8; USB_PACKET_LENGTH];

        for packet in &XBOXONE_INIT_PACKETS {
            if packet.vendor_id != 0 && vendor_id != packet.vendor_id {
                continue;
            }

            if packet.product_id != 0 && product_id != packet.product_id {
                continue;
            }

            let size = packet.data.len();
            init_packet[..size].copy_from_slice(packet.data);
            init_packet[2] = self.get_next_packet_sequence();

            if init_packet[0] == 0x0A {
                // Get the initial brightness value
                let brightness = get_home_led_brightness(
                    hints::get(hints::JOYSTICK_HIDAPI_XBOX_ONE_HOME_LED).as_deref(),
                );
                init_packet[5] = if brightness > 0 { 0x01 } else { 0x00 };
                init_packet[6] = brightness as u8;
            }

            if !self.send_protocol_packet(device, &init_packet[..size]) {
                return Err(Error::new("Couldn't send initialization packet"));
            }

            // Wait to process the rumble packet
            if std::ptr::eq(packet.data, XBOX_INIT_POWERA_RUMBLE) {
                crate::timer::delay(std::time::Duration::from_millis(10));
            }
        }
        Ok(())
    }

    /// Translation of `SDL_HomeLEDHintChanged()`, for a change recorded
    /// since the last call.
    fn home_led_hint_changed(&mut self, device: &DeviceCtx<'_>) {
        let Some(hint) = self.home_led_hint.as_ref().and_then(HintWatch::take) else {
            return;
        };
        if let Some(hint) = hint.filter(|h| !h.is_empty()) {
            set_home_led(device.device(), get_home_led_brightness(Some(&hint)));
        }
    }

    /// Translation of `HIDAPI_DriverXboxOne_UpdateRumble()`.
    fn update_rumble(&mut self, device: &DeviceCtx<'_>) -> Result<()> {
        if self.rumble_state == RumbleState::Queued && self.rumble_time.load(Ordering::Acquire) != 0
        {
            self.rumble_state = RumbleState::Busy;
        }

        if self.rumble_state == RumbleState::Busy {
            let rumble_busy_time_ms = if device.is_bluetooth() { 50 } else { 10 };
            if crate::timer::ticks_ms()
                >= self.rumble_time.load(Ordering::Acquire) + rumble_busy_time_ms
            {
                self.rumble_time.store(0, Ordering::Release);
                self.rumble_state = RumbleState::Idle;
            }
        }

        if !self.rumble_pending {
            return Ok(());
        }

        if self.rumble_state != RumbleState::Idle {
            return Ok(());
        }

        // We're no longer pending, even if we fail to send the rumble below
        self.rumble_pending = false;

        let lock = lock_rumble()?;

        // (translation of HIDAPI_DriverXboxOne_RumbleSent())
        let rumble_time = self.rumble_time.clone();
        let callback =
            Box::new(move || rumble_time.store(crate::timer::ticks_ms(), Ordering::Release));

        if device.is_bluetooth() {
            let mut rumble_packet = [0x03, 0x0F, 0x00, 0x00, 0x00, 0x00, 0xFF, 0x00, 0xEB];

            rumble_packet[2] = self.left_trigger_rumble;
            rumble_packet[3] = self.right_trigger_rumble;
            rumble_packet[4] = self.low_frequency_rumble;
            rumble_packet[5] = self.high_frequency_rumble;

            if lock
                .send_with_callback_and_unlock(device.device(), &rumble_packet, Some(callback))
                .ok()
                != Some(rumble_packet.len())
            {
                return Err(Error::new("Couldn't send rumble packet"));
            }
        } else {
            let mut rumble_packet = [
                0x09, 0x00, 0x00, 0x09, 0x00, 0x0F, 0x00, 0x00, 0x00, 0x00, 0xFF, 0x00, 0xEB,
            ];

            rumble_packet[6] = self.left_trigger_rumble;
            rumble_packet[7] = self.right_trigger_rumble;
            rumble_packet[8] = self.low_frequency_rumble;
            rumble_packet[9] = self.high_frequency_rumble;

            if lock
                .send_with_callback_and_unlock(device.device(), &rumble_packet, Some(callback))
                .ok()
                != Some(rumble_packet.len())
            {
                return Err(Error::new("Couldn't send rumble packet"));
            }
        }

        self.rumble_state = RumbleState::Queued;

        Ok(())
    }

    /// Translation of `HIDAPI_DriverXboxOne_HandleDescriptorReport()`.
    fn handle_descriptor_report(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: JoystickID,
        data: &[u8],
        size: usize,
    ) -> bool {
        let timestamp = crate::timer::ticks_ns();

        if size == 0 {
            return false;
        }

        // Skip the report ID
        let report_id = data[0];
        let data = &data[1..];
        let size = size - 1;

        let fields = self
            .descriptor
            .as_ref()
            .map(|d| d.fields.clone())
            .unwrap_or_default();
        for field in &fields {
            if field.report_id != report_id {
                continue;
            }

            let Ok(mut value) = read_report_data(data, size, field.bit_offset, field.bit_size)
            else {
                continue;
            };

            const GENERIC_X: u32 = make_usage(USB_USAGEPAGE_GENERIC_DESKTOP, USB_USAGE_GENERIC_X);
            const GENERIC_Y: u32 = make_usage(USB_USAGEPAGE_GENERIC_DESKTOP, USB_USAGE_GENERIC_Y);
            const GENERIC_Z: u32 = make_usage(USB_USAGEPAGE_GENERIC_DESKTOP, USB_USAGE_GENERIC_Z);
            const GENERIC_RX: u32 = make_usage(USB_USAGEPAGE_GENERIC_DESKTOP, USB_USAGE_GENERIC_RX);
            const GENERIC_RY: u32 = make_usage(USB_USAGEPAGE_GENERIC_DESKTOP, USB_USAGE_GENERIC_RY);
            const GENERIC_RZ: u32 = make_usage(USB_USAGEPAGE_GENERIC_DESKTOP, USB_USAGE_GENERIC_RZ);
            const SIMULATION_BRAKE: u32 =
                make_usage(USB_USAGEPAGE_SIMULATION, USB_USAGE_SIMULATION_BRAKE);
            const SIMULATION_ACCELERATOR: u32 =
                make_usage(USB_USAGEPAGE_SIMULATION, USB_USAGE_SIMULATION_ACCELERATOR);
            const GENERIC_HAT: u32 =
                make_usage(USB_USAGEPAGE_GENERIC_DESKTOP, USB_USAGE_GENERIC_HAT);
            const BUTTON_1: u32 = make_usage(USB_USAGEPAGE_BUTTON, 1);
            const CONSUMER_AC_BACK: u32 =
                make_usage(USB_USAGEPAGE_CONSUMER, USB_USAGE_CONSUMER_AC_BACK);
            const CONSUMER_AC_HOME: u32 =
                make_usage(USB_USAGEPAGE_CONSUMER, USB_USAGE_CONSUMER_AC_HOME);
            const CONSUMER_RECORD: u32 =
                make_usage(USB_USAGEPAGE_CONSUMER, USB_USAGE_CONSUMER_RECORD);
            const CONSUMER_ORDER_MOVIE: u32 =
                make_usage(USB_USAGEPAGE_CONSUMER, USB_USAGE_CONSUMER_ORDER_MOVIE);
            const CONSUMER_ASSIGN_SELECTION: u32 =
                make_usage(USB_USAGEPAGE_CONSUMER, USB_USAGE_CONSUMER_ASSIGN_SELECTION);
            const BATTERY_STRENGTH: u32 = make_usage(
                USB_USAGEPAGE_DEVICE_CONTROLS,
                USB_USAGE_DEVICE_CONTROLS_BATTERY_STRENGTH,
            );

            match field.usage {
                GENERIC_X => {
                    handle_descriptor_axis(device, timestamp, joystick, GamepadAxis::LeftX, value)
                }
                GENERIC_Y => {
                    handle_descriptor_axis(device, timestamp, joystick, GamepadAxis::LeftY, value)
                }
                // Some controllers use Z+RZ for the right thumbstick and BRAKE and ACCEL for the left and right triggers
                // and others use RX+RY for the right thumbstick and Z and RZ for the left and right triggers
                GENERIC_Z => {
                    if field.bit_size == 16 {
                        handle_descriptor_axis(
                            device,
                            timestamp,
                            joystick,
                            GamepadAxis::RightX,
                            value,
                        );
                    } else if field.bit_size == 10 {
                        handle_descriptor_trigger(
                            device,
                            timestamp,
                            joystick,
                            GamepadAxis::LeftTrigger,
                            value,
                        );
                    }
                }
                GENERIC_RX => {
                    handle_descriptor_axis(device, timestamp, joystick, GamepadAxis::RightX, value)
                }
                GENERIC_RY => {
                    handle_descriptor_axis(device, timestamp, joystick, GamepadAxis::RightY, value)
                }
                GENERIC_RZ => {
                    if field.bit_size == 16 {
                        handle_descriptor_axis(
                            device,
                            timestamp,
                            joystick,
                            GamepadAxis::RightY,
                            value,
                        );
                    } else if field.bit_size == 10 {
                        handle_descriptor_trigger(
                            device,
                            timestamp,
                            joystick,
                            GamepadAxis::RightTrigger,
                            value,
                        );
                    }
                }
                SIMULATION_BRAKE => handle_descriptor_trigger(
                    device,
                    timestamp,
                    joystick,
                    GamepadAxis::LeftTrigger,
                    value,
                ),
                SIMULATION_ACCELERATOR => handle_descriptor_trigger(
                    device,
                    timestamp,
                    joystick,
                    GamepadAxis::RightTrigger,
                    value,
                ),
                GENERIC_HAT => device.send_hat(timestamp, joystick, 0, hat_of(value)),
                BUTTON_1 => {
                    use GamepadButton as B;
                    const BUTTON_MAP_12: [GamepadButton; 12] = [
                        B::South,         // 0x0001
                        B::East,          // 0x0002
                        B::West,          // 0x0004
                        B::North,         // 0x0008
                        B::LeftShoulder,  // 0x0010
                        B::RightShoulder, // 0x0020
                        B::Back,          // 0x0040
                        B::Start,         // 0x0080
                        B::LeftStick,     // 0x0100
                        B::RightStick,    // 0x0200
                        B::Guide,         // 0x0400
                        B::Invalid,       // 0x0800
                    ];
                    const BUTTON_MAP_15: [GamepadButton; 15] = [
                        B::South,         // 0x0001
                        B::East,          // 0x0002
                        B::Invalid,       // 0x0004
                        B::West,          // 0x0008
                        B::North,         // 0x0010
                        B::Invalid,       // 0x0020
                        B::LeftShoulder,  // 0x0040
                        B::RightShoulder, // 0x0080
                        B::Invalid,       // 0x0100
                        B::Invalid,       // 0x0200
                        B::Back,          // 0x0400
                        B::Start,         // 0x0800
                        B::Guide,         // 0x1000
                        B::LeftStick,     // 0x2000
                        B::RightStick,    // 0x4000
                    ];

                    if value == self.last_buttons {
                        continue;
                    }
                    self.last_buttons = value;

                    let button_map: &[GamepadButton] = match field.bit_size {
                        12 => &BUTTON_MAP_12,
                        15 => &BUTTON_MAP_15,
                        // Should never happen
                        _ => continue,
                    };
                    for &button in button_map {
                        let pressed = value & 1 != 0;
                        value >>= 1;
                        if button == B::Invalid {
                            continue;
                        }
                        if button == B::Back && self.has_separate_back_button {
                            continue;
                        }
                        if button == B::Guide && self.has_separate_guide_button {
                            continue;
                        }

                        device.send_button(timestamp, joystick, button as u8, pressed);
                    }
                }
                CONSUMER_AC_BACK => {
                    let pressed = value != 0;
                    if pressed {
                        self.has_separate_back_button = true;
                    }
                    device.send_button(timestamp, joystick, GamepadButton::Back as u8, pressed);
                }
                CONSUMER_AC_HOME => {
                    let pressed = value != 0;
                    if pressed {
                        self.has_separate_guide_button = true;
                    }
                    device.send_button(timestamp, joystick, GamepadButton::Guide as u8, pressed);
                }
                CONSUMER_RECORD => {
                    if self.has_share_button {
                        let pressed = value != 0;
                        device.send_button(
                            timestamp,
                            joystick,
                            SDL_GAMEPAD_BUTTON_XBOX_SHARE_BUTTON,
                            pressed,
                        );
                    }
                }
                CONSUMER_ORDER_MOVIE => {
                    // This value is the currently selected profile
                    self.has_unmapped_state = value == 0;
                }
                CONSUMER_ASSIGN_SELECTION => {
                    if self.has_paddles {
                        if !self.has_unmapped_state {
                            value = 0;
                        }

                        let button =
                            SDL_GAMEPAD_BUTTON_XBOX_SHARE_BUTTON + u8::from(self.has_share_button); // Next available button
                        device.send_button(timestamp, joystick, button, value & 0x1 != 0);
                        device.send_button(timestamp, joystick, button + 1, value & 0x2 != 0);
                        device.send_button(timestamp, joystick, button + 2, value & 0x4 != 0);
                        device.send_button(timestamp, joystick, button + 3, value & 0x8 != 0);
                    }
                }
                BATTERY_STRENGTH => handle_battery_state(device, joystick, value),
                _ => {}
            }
        }
        true
    }

    /// The four paddle buttons, if their state changed.
    #[allow(clippy::too_many_arguments)]
    fn send_paddles(
        &mut self,
        device: &mut DeviceCtx<'_>,
        timestamp: u64,
        joystick: JoystickID,
        first_button: u8,
        paddles: u8,
        bits: [u8; 4],
    ) {
        if self.last_paddle_state != paddles {
            for (n_button, bit) in (first_button..).zip(bits) {
                device.send_button(timestamp, joystick, n_button, paddles & bit != 0);
            }
            self.last_paddle_state = paddles;
        }
    }

    /// Translation of `HIDAPI_DriverXboxOne_HandleUnmappedStatePacket()`:
    /// the Xbox One Elite controller with 5.13+ firmware sends the unmapped
    /// state in a separate packet. We can use this to send the paddle state
    /// when they aren't mapped.
    fn handle_unmapped_state_packet(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: JoystickID,
        data: &mut [u8],
        size: usize,
    ) {
        let timestamp = crate::timer::ticks_ns();

        let (paddle_index, bits, paddles_mapped) = if size == 17 {
            // XBox One Elite Series 2
            let profile = data[15];

            let paddles_mapped = if profile == 0 {
                false
            } else {
                // We're using a profile, but paddles aren't mapped if the
                // state is unchanged; otherwise something is mapped, we
                // can't use the paddles
                data[0..14] != self.last_state[0..14]
            };
            (14, [0x01, 0x02, 0x04, 0x08], paddles_mapped)
        } else {
            // Unknown format
            return;
        };

        if paddles_mapped {
            // Respect that the paddles are being used for other controls and don't pass them on to the app
            data[paddle_index] = 0;
        }

        let first = SDL_GAMEPAD_BUTTON_XBOX_SHARE_BUTTON + u8::from(self.has_share_button); // Next available button
        self.send_paddles(device, timestamp, joystick, first, data[paddle_index], bits);
        self.has_unmapped_state = true;
    }

    /// Translation of `HIDAPI_DriverXboxOne_HandleStatePacket()`.
    fn handle_state_packet(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: JoystickID,
        data: &mut [u8],
        size: usize,
    ) {
        let timestamp = crate::timer::ticks_ns();

        // Enable paddles on the Xbox Elite controller when connected over USB
        if self.has_paddles && !self.has_unmapped_state && size == 46 {
            let packet = [0x4d, 0x00, 0x00, 0x02, 0x07, 0x00];

            let _ = send_rumble(device.device(), &packet);
        }

        if self.last_state[0] != data[0] {
            device.send_button(
                timestamp,
                joystick,
                GamepadButton::Start as u8,
                data[0] & 0x04 != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                GamepadButton::Back as u8,
                data[0] & 0x08 != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                GamepadButton::South as u8,
                data[0] & 0x10 != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                GamepadButton::East as u8,
                data[0] & 0x20 != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                GamepadButton::West as u8,
                data[0] & 0x40 != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                GamepadButton::North as u8,
                data[0] & 0x80 != 0,
            );
        }

        if self.last_state[1] != data[1] {
            let mut hat = 0;

            if data[1] & 0x01 != 0 {
                hat |= HAT_UP;
            }
            if data[1] & 0x02 != 0 {
                hat |= HAT_DOWN;
            }
            if data[1] & 0x04 != 0 {
                hat |= HAT_LEFT;
            }
            if data[1] & 0x08 != 0 {
                hat |= HAT_RIGHT;
            }
            device.send_hat(timestamp, joystick, 0, hat);

            if self.vendor_id == USB_VENDOR_RAZER && self.product_id == USB_PRODUCT_RAZER_ATROX {
                // The Razer Atrox has the right and left shoulder bits reversed
                device.send_button(
                    timestamp,
                    joystick,
                    GamepadButton::LeftShoulder as u8,
                    data[1] & 0x20 != 0,
                );
                device.send_button(
                    timestamp,
                    joystick,
                    GamepadButton::RightShoulder as u8,
                    data[1] & 0x10 != 0,
                );
            } else {
                device.send_button(
                    timestamp,
                    joystick,
                    GamepadButton::LeftShoulder as u8,
                    data[1] & 0x10 != 0,
                );
                device.send_button(
                    timestamp,
                    joystick,
                    GamepadButton::RightShoulder as u8,
                    data[1] & 0x20 != 0,
                );
            }
            device.send_button(
                timestamp,
                joystick,
                GamepadButton::LeftStick as u8,
                data[1] & 0x40 != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                GamepadButton::RightStick as u8,
                data[1] & 0x80 != 0,
            );
        }

        if self.has_share_button {
            /* Xbox Series X firmware version 5.0, report is 32 bytes, share button is in byte 14
             * Xbox Series X firmware version 5.1, report is 40 bytes, share button is in byte 14
             * Xbox Series X firmware version 5.5, report is 44 bytes, share button is in byte 18
             * Victrix Gambit Tournament Controller, report is 46 bytes, share button is in byte 28
             * ThrustMaster eSwap PRO Controller Xbox, report is 60 bytes, share button is in byte 42
             */
            let share_index = match size {
                s if s < 44 => Some(14),
                44 => Some(18),
                46 => Some(28),
                60 => Some(42),
                _ => None,
            };
            if let Some(i) = share_index {
                if self.last_state[i] != data[i] {
                    device.send_button(
                        timestamp,
                        joystick,
                        SDL_GAMEPAD_BUTTON_XBOX_SHARE_BUTTON,
                        data[i] & 0x01 != 0,
                    );
                }
            }
        }

        /* Xbox One S report is 14 bytes
           Xbox One Elite Series 1 report is 29 bytes, paddles in data[28], mode in data[28] & 0x10, both modes have mapped paddles by default
            Paddle bits:
                P3: 0x01 (A)    P1: 0x02 (B)
                P4: 0x04 (X)    P2: 0x08 (Y)
           Xbox One Elite Series 2 4.x firmware report is 34 bytes, paddles in data[14], mode in data[15], mode 0 has no mapped paddles by default
            Paddle bits:
                P3: 0x04 (A)    P1: 0x01 (B)
                P4: 0x08 (X)    P2: 0x02 (Y)
           Xbox One Elite Series 2 5.x firmware report is 46 bytes, paddles in data[18], mode in data[19], mode 0 has no mapped paddles by default
            Paddle bits:
                P3: 0x04 (A)    P1: 0x01 (B)
                P4: 0x08 (X)    P2: 0x02 (Y)
           Xbox One Elite Series 2 5.17+ firmware report is 47 bytes, paddles in data[14], mode in data[20], mode 0 has no mapped paddles by default
            Paddle bits:
                P3: 0x04 (A)    P1: 0x01 (B)
                P4: 0x08 (X)    P2: 0x02 (Y)
        */
        if self.has_paddles && !self.has_unmapped_state && matches!(size, 29 | 34 | 46 | 47) {
            let (paddle_index, bits, paddles_mapped) = match size {
                29 => {
                    // XBox One Elite Series 1
                    // The mapped controller state is at offset 0, the raw state is at offset 14, compare them to see if the paddles are mapped
                    (28, [0x02, 0x08, 0x01, 0x04], data[0..2] != data[14..16])
                }
                34 => {
                    // XBox One Elite Series 2
                    (14, [0x01, 0x02, 0x04, 0x08], data[15] != 0)
                }
                46 => {
                    // XBox One Elite Series 2
                    (18, [0x01, 0x02, 0x04, 0x08], data[19] != 0)
                }
                _ => {
                    // XBox One Elite Series 2 (47 bytes)
                    (14, [0x01, 0x02, 0x04, 0x08], data[20] != 0)
                }
            };

            if paddles_mapped {
                // Respect that the paddles are being used for other controls and don't pass them on to the app
                data[paddle_index] = 0;
            }

            let first = SDL_GAMEPAD_BUTTON_XBOX_SHARE_BUTTON + u8::from(self.has_share_button); // Next available button
            self.send_paddles(device, timestamp, joystick, first, data[paddle_index], bits);
        }

        let mut axis = (i32::from(load16(data[2], data[3])) * 64 - 32768) as i16;
        if axis == 32704 {
            axis = 32767;
        }
        if axis == -32768 && size == 26 && (data[18] & 0x80) != 0 {
            axis = 32767;
        }
        device.send_axis(timestamp, joystick, GamepadAxis::LeftTrigger as u8, axis);

        axis = (i32::from(load16(data[4], data[5])) * 64 - 32768) as i16;
        if axis == -32768 && size == 26 && (data[18] & 0x40) != 0 {
            axis = 32767;
        }
        if axis == 32704 {
            axis = 32767;
        }
        device.send_axis(timestamp, joystick, GamepadAxis::RightTrigger as u8, axis);

        axis = load16(data[6], data[7]);
        device.send_axis(timestamp, joystick, GamepadAxis::LeftX as u8, axis);
        axis = load16(data[8], data[9]);
        device.send_axis(timestamp, joystick, GamepadAxis::LeftY as u8, !axis);
        axis = load16(data[10], data[11]);
        device.send_axis(timestamp, joystick, GamepadAxis::RightX as u8, axis);
        axis = load16(data[12], data[13]);
        device.send_axis(timestamp, joystick, GamepadAxis::RightY as u8, !axis);

        let n = size.min(self.last_state.len());
        self.last_state[..n].copy_from_slice(&data[..n]);

        // We don't have the unmapped state for this packet
        self.has_unmapped_state = false;
    }

    /// Translation of `HIDAPI_DriverXboxOne_HandleStatusPacket()`.
    fn handle_status_packet(&mut self) {
        if self.init_state < InitState::Complete {
            self.set_init_state(InitState::Complete);
        }
    }

    /// Translation of `HIDAPI_DriverXboxOne_HandleModePacket()`.
    fn handle_mode_packet(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: JoystickID,
        data: &[u8],
    ) {
        let timestamp = crate::timer::ticks_ns();

        device.send_button(
            timestamp,
            joystick,
            GamepadButton::Guide as u8,
            data[0] & 0x01 != 0,
        );
    }

    /// Translation of `HIDAPI_DriverXboxOneBluetooth_HandleButtons16()`:
    /// Xbox One S with firmware 3.1.1221 uses a 16 byte packet and the
    /// GUIDE button in a separate packet.
    fn bluetooth_handle_buttons16(
        &mut self,
        device: &mut DeviceCtx<'_>,
        timestamp: u64,
        joystick: JoystickID,
        data: &[u8],
    ) {
        if self.last_state[14] != data[14] {
            device.send_button(
                timestamp,
                joystick,
                GamepadButton::South as u8,
                data[14] & 0x01 != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                GamepadButton::East as u8,
                data[14] & 0x02 != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                GamepadButton::West as u8,
                data[14] & 0x04 != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                GamepadButton::North as u8,
                data[14] & 0x08 != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                GamepadButton::LeftShoulder as u8,
                data[14] & 0x10 != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                GamepadButton::RightShoulder as u8,
                data[14] & 0x20 != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                GamepadButton::Back as u8,
                data[14] & 0x40 != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                GamepadButton::Start as u8,
                data[14] & 0x80 != 0,
            );
        }

        if self.last_state[15] != data[15] {
            device.send_button(
                timestamp,
                joystick,
                GamepadButton::LeftStick as u8,
                data[15] & 0x01 != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                GamepadButton::RightStick as u8,
                data[15] & 0x02 != 0,
            );
        }
    }

    /// Translation of `HIDAPI_DriverXboxOneBluetooth_HandleButtons()`:
    /// * Xbox One S with firmware 4.8.1923 uses a 17 byte packet with BACK button in byte 16 and the GUIDE button in a separate packet (on Windows), or in byte 15 (on Linux)
    /// * Xbox One S with firmware 5.x uses a 17 byte packet with BACK and GUIDE buttons in byte 15
    /// * Xbox One Elite Series 2 with firmware 4.7.1872 uses a 55 byte packet with BACK button in byte 16, paddles starting at byte 33, and the GUIDE button in a separate packet
    /// * Xbox One Elite Series 2 with firmware 4.8.1908 uses a 33 byte packet with BACK button in byte 16, paddles starting at byte 17, and the GUIDE button in a separate packet
    /// * Xbox One Elite Series 2 with firmware 5.11.3112 uses a 19 byte packet with BACK and GUIDE buttons in byte 15
    /// * Xbox Series X with firmware 5.5.2641 uses a 17 byte packet with BACK and GUIDE buttons in byte 15, and SHARE button in byte 17
    fn bluetooth_handle_buttons(
        &mut self,
        device: &mut DeviceCtx<'_>,
        timestamp: u64,
        joystick: JoystickID,
        data: &mut [u8],
        size: usize,
    ) {
        if self.last_state[14] != data[14] {
            device.send_button(
                timestamp,
                joystick,
                GamepadButton::South as u8,
                data[14] & 0x01 != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                GamepadButton::East as u8,
                data[14] & 0x02 != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                GamepadButton::West as u8,
                data[14] & 0x08 != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                GamepadButton::North as u8,
                data[14] & 0x10 != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                GamepadButton::LeftShoulder as u8,
                data[14] & 0x40 != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                GamepadButton::RightShoulder as u8,
                data[14] & 0x80 != 0,
            );
        }

        if self.last_state[15] != data[15] {
            if !self.has_guide_packet {
                device.send_button(
                    timestamp,
                    joystick,
                    GamepadButton::Guide as u8,
                    data[15] & 0x10 != 0,
                );
            }
            device.send_button(
                timestamp,
                joystick,
                GamepadButton::Start as u8,
                data[15] & 0x08 != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                GamepadButton::LeftStick as u8,
                data[15] & 0x20 != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                GamepadButton::RightStick as u8,
                data[15] & 0x40 != 0,
            );
        }

        if self.has_share_button {
            device.send_button(
                timestamp,
                joystick,
                GamepadButton::Back as u8,
                data[15] & 0x04 != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                SDL_GAMEPAD_BUTTON_XBOX_SHARE_BUTTON,
                data[16] & 0x01 != 0,
            );
        } else {
            device.send_button(
                timestamp,
                joystick,
                GamepadButton::Back as u8,
                (data[15] & 0x04) != 0 || (data[16] & 0x01) != 0,
            );
        }

        /*
            Paddle bits:
                P3: 0x04 (A)    P1: 0x01 (B)
                P4: 0x08 (X)    P2: 0x02 (Y)
        */
        if self.has_paddles && matches!(size, 20 | 39 | 55) {
            let (paddle_index, paddles_mapped) = match size {
                // Initial firmware for the Xbox Elite Series 2 controller
                55 => (33, data[35] != 0),
                // Updated firmware for the Xbox Elite Series 2 controller
                39 => (17, data[19] != 0),
                // Updated firmware for the Xbox Elite Series 2 controller (5.13+)
                _ => (19, data[17] != 0),
            };

            if paddles_mapped {
                // Respect that the paddles are being used for other controls and don't pass them on to the app
                data[paddle_index] = 0;
            }

            // Next available button
            self.send_paddles(
                device,
                timestamp,
                joystick,
                SDL_GAMEPAD_BUTTON_XBOX_SHARE_BUTTON,
                data[paddle_index],
                [0x01, 0x02, 0x04, 0x08],
            );
        }
    }

    /// Translation of `HIDAPI_DriverXboxOneBluetooth_HandleStatePacket()`.
    fn bluetooth_handle_state_packet(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: JoystickID,
        data: &mut [u8],
        size: usize,
    ) {
        let timestamp = crate::timer::ticks_ns();

        if size == 16 {
            // Original Xbox One S, with separate report for guide button
            self.bluetooth_handle_buttons16(device, timestamp, joystick, data);
        } else if size > 16 {
            self.bluetooth_handle_buttons(device, timestamp, joystick, data, size);
        } else {
            // Unknown Bluetooth state packet format
            return;
        }

        if self.last_state[13] != data[13] {
            device.send_hat(timestamp, joystick, 0, hat_of(u32::from(data[13])));
        }

        let mut axis = (i32::from(load16(data[9], data[10])) * 64 - 32768) as i16;
        if axis == 32704 {
            axis = 32767;
        }
        device.send_axis(timestamp, joystick, GamepadAxis::LeftTrigger as u8, axis);

        axis = (i32::from(load16(data[11], data[12])) * 64 - 32768) as i16;
        if axis == 32704 {
            axis = 32767;
        }
        device.send_axis(timestamp, joystick, GamepadAxis::RightTrigger as u8, axis);

        axis = (i32::from(load16(data[1], data[2]) as u16) - 0x8000) as i16;
        device.send_axis(timestamp, joystick, GamepadAxis::LeftX as u8, axis);
        axis = (i32::from(load16(data[3], data[4]) as u16) - 0x8000) as i16;
        device.send_axis(timestamp, joystick, GamepadAxis::LeftY as u8, axis);
        axis = (i32::from(load16(data[5], data[6]) as u16) - 0x8000) as i16;
        device.send_axis(timestamp, joystick, GamepadAxis::RightX as u8, axis);
        axis = (i32::from(load16(data[7], data[8]) as u16) - 0x8000) as i16;
        device.send_axis(timestamp, joystick, GamepadAxis::RightY as u8, axis);

        let n = size.min(self.last_state.len());
        self.last_state[..n].copy_from_slice(&data[..n]);
    }

    /// Translation of `HIDAPI_DriverXboxOneBluetooth_HandleGuidePacket()`.
    fn bluetooth_handle_guide_packet(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: JoystickID,
        data: &[u8],
    ) {
        let timestamp = crate::timer::ticks_ns();

        self.has_guide_packet = true;
        device.send_button(
            timestamp,
            joystick,
            GamepadButton::Guide as u8,
            data[1] & 0x01 != 0,
        );
    }

    /// Translation of `HIDAPI_DriverXboxOneBluetooth_HandleBatteryPacket()`.
    fn bluetooth_handle_battery_packet(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: JoystickID,
        data: &[u8],
    ) {
        handle_battery_state(device, joystick, u32::from(data[1]));
    }

    /// Translation of `HIDAPI_DriverXboxOne_HandleSerialIDPacket()`.
    fn handle_serial_id_packet(&mut self, device: &DeviceCtx<'_>, data: &[u8]) {
        let mut serial = [0u8; 29];

        for i in 0..14 {
            // (SDL_uitoa: lowercase hex without leading zeros, then a NUL)
            let digits = format!("{:x}", data[2 + i]);
            serial[i * 2..i * 2 + digits.len()].copy_from_slice(digits.as_bytes());
            serial[i * 2 + digits.len()] = 0;
        }
        serial[14 * 2] = 0;
        // FIXME (upstream): a byte under 0x10 gives one digit and a NUL,
        // which cuts the serial number short there.
        let end = serial.iter().position(|&c| c == 0).unwrap_or(serial.len());

        device.set_device_serial(std::str::from_utf8(&serial[..end]).unwrap_or(""));
    }

    /// Translation of `HIDAPI_DriverXboxOne_UpdateInitState()`.
    fn update_init_state(&mut self, device: &DeviceCtx<'_>) {
        loop {
            let prev_state = self.init_state;

            match self.init_state {
                InitState::Announced => {
                    if XBOX_ONE_DRIVER_ACTIVE {
                        // The driver is taking care of identification
                        self.set_init_state(InitState::Complete);
                    } else {
                        let _ = self.send_identification_request(device);
                        self.set_init_state(InitState::Identifying);
                    }
                }
                InitState::Identifying => {
                    if crate::timer::ticks_ms() >= self.send_time + CONTROLLER_IDENTIFY_TIMEOUT_MS {
                        // We haven't heard anything, let's move on
                        self.set_init_state(InitState::Startup);
                    }
                }
                InitState::Startup => {
                    if XBOX_ONE_DRIVER_ACTIVE {
                        // The driver is taking care of startup
                        self.set_init_state(InitState::Complete);
                    } else {
                        let _ = self.send_controller_startup(device);
                        self.set_init_state(InitState::PrepareInput);
                    }
                }
                InitState::PrepareInput => {
                    if crate::timer::ticks_ms()
                        >= self.send_time + CONTROLLER_PREPARE_INPUT_TIMEOUT_MS
                    {
                        self.set_init_state(InitState::Complete);
                    }
                }
                InitState::Complete => {}
            }

            if self.init_state == prev_state {
                break;
            }
        }
    }

    /// Translation of `HIDAPI_GIP_SendPacket()`.
    fn gip_send_packet(
        &mut self,
        device: &DeviceCtx<'_>,
        hdr: &mut GipHeader,
        data: Option<&[u8]>,
    ) -> Result<()> {
        let mut packet = [0u8; USB_PACKET_LENGTH];

        let hdr_len = gip_get_header_length(hdr);
        let size = hdr_len + hdr.packet_length as usize;
        if size > packet.len() {
            return Err(Error::new(format!(
                "Couldn't send GIP packet, size ({size}) too large"
            )));
        }

        if hdr.sequence == 0 {
            hdr.sequence = self.get_next_packet_sequence();
        }

        gip_encode_header(hdr, &mut packet);
        if let Some(data) = data {
            let n = hdr.packet_length as usize;
            packet[hdr_len..hdr_len + n].copy_from_slice(&data[..n]);
        }

        if !self.send_protocol_packet(device, &packet[..size]) {
            return Err(Error::new("Couldn't send protocol packet"));
        }
        Ok(())
    }

    /// Translation of `HIDAPI_GIP_AcknowledgePacket()`.
    fn gip_acknowledge_packet(&mut self, device: &DeviceCtx<'_>, ack: &GipHeader) -> Result<()> {
        if XBOX_ONE_DRIVER_ACTIVE {
            // The driver is taking care of acks
            return Ok(());
        }

        let mut hdr = GipHeader {
            command: GIP_CMD_ACKNOWLEDGE,
            options: GIP_OPT_INTERNAL,
            sequence: ack.sequence,
            packet_length: GIP_PKT_ACKNOWLEDGE_SIZE,
            chunk_offset: 0,
        };

        // struct gip_pkt_acknowledge: unknown, command, options, length,
        // padding[2], remaining
        let length = ack.chunk_offset.wrapping_add(ack.packet_length) as u16;
        let mut remaining = 0u16;
        if ack.options & GIP_OPT_CHUNK != 0 && self.chunk_buffer.is_some() {
            remaining = (self.chunk_length as u16).wrapping_sub(length);
        }
        let mut pkt = [0u8; GIP_PKT_ACKNOWLEDGE_SIZE as usize];
        pkt[1] = ack.command;
        pkt[2] = GIP_OPT_INTERNAL;
        pkt[3..5].copy_from_slice(&length.to_le_bytes());
        pkt[7..9].copy_from_slice(&remaining.to_le_bytes());

        self.gip_send_packet(device, &mut hdr, Some(&pkt))
    }

    /// Translation of `HIDAPI_GIP_DispatchPacket()`.
    fn gip_dispatch_packet(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: Option<JoystickID>,
        hdr: &GipHeader,
        data: &[u8],
        size: u32,
    ) -> bool {
        if (hdr.options & 0x0F) != 0 {
            // This is a packet for a device plugged into the controller, skip it
            return true;
        }

        let size = size as usize;
        if hdr.options & GIP_OPT_INTERNAL != 0 {
            match hdr.command {
                GIP_CMD_ACKNOWLEDGE => {
                    // Ignore this packet
                }
                GIP_CMD_ANNOUNCE => {
                    // Controller is connected and waiting for initialization
                    /* The data bytes are:
                       0x02 0x20 NN 0x1c, where NN is the packet sequence
                       then 6 bytes of wireless MAC address
                       then 2 bytes padding
                       then 16-bit VID
                       then 16-bit PID
                       then 16-bit firmware version quartet AA.BB.CC.DD
                            e.g. 0x05 0x00 0x05 0x00 0x51 0x0a 0x00 0x00
                                 is firmware version 5.5.2641.0, and product version 0x0505 = 1285
                       then 8 bytes of unknown data
                    */
                    self.set_init_state(InitState::Announced);
                }
                GIP_CMD_STATUS => {
                    // Controller status update
                    self.handle_status_packet();
                }
                GIP_CMD_IDENTIFY => {
                    self.set_init_state(InitState::Startup);
                }
                GIP_CMD_POWER => {
                    // Ignore this packet
                }
                GIP_CMD_AUTHENTICATE => {
                    // Ignore this packet
                }
                GIP_CMD_VIRTUAL_KEY => {
                    if let Some(joystick) = joystick {
                        self.handle_mode_packet(device, joystick, &padded(data));
                    }
                }
                GIP_CMD_SERIAL_NUMBER => {
                    /* If the packet starts with this:
                        0x1E 0x30 0x00 0x10 0x04 0x00
                        then the next 14 bytes are the controller serial number
                            e.g. 0x30 0x39 0x37 0x31 0x32 0x33 0x33 0x32 0x33 0x35 0x34 0x30 0x33 0x36
                            is serial number "3039373132333332333534303336"

                       The controller sends that in response to this request:
                        0x1E 0x20 0x00 0x01 0x04
                    */
                    self.handle_serial_id_packet(device, &padded(data));
                }
                _ => {
                    // Unknown Xbox One packet
                }
            }
        } else {
            match hdr.command {
                GIP_CMD_INPUT => {
                    if self.init_state < InitState::Complete {
                        self.set_init_state(InitState::Complete);

                        // Ignore the first input, it may be spurious
                        return true;
                    }
                    if let Some(joystick) = joystick {
                        let mut packet = padded(data);
                        self.handle_state_packet(device, joystick, &mut packet, size);
                    }
                }
                GIP_CMD_UNMAPPED_STATE => {
                    if let Some(joystick) = joystick {
                        let mut packet = padded(data);
                        self.handle_unmapped_state_packet(device, joystick, &mut packet, size);
                    }
                }
                _ => {
                    // Unknown Xbox One packet
                }
            }
        }
        true
    }

    /// Translation of `HIDAPI_GIP_CreateChunkBuffer()`.
    fn gip_create_chunk_buffer(&mut self, size: u32) -> bool {
        self.chunk_buffer = None;
        self.chunk_length = 0;

        // (allocation failure is a failure, as upstream's malloc)
        let mut buffer = Vec::new();
        if buffer.try_reserve_exact(size as usize).is_err() {
            return false;
        }
        buffer.resize(size as usize, 0);
        self.chunk_buffer = Some(buffer);
        self.chunk_length = size;
        true
    }

    /// Translation of `HIDAPI_GIP_ProcessPacketChunked()`.
    fn gip_process_packet_chunked(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: Option<JoystickID>,
        hdr: &GipHeader,
        data: &[u8],
    ) -> bool {
        let Some(chunk_buffer) = self.chunk_buffer.as_mut() else {
            return false;
        };

        if hdr.chunk_offset.wrapping_add(hdr.packet_length) > self.chunk_length {
            return false;
        }

        if hdr.packet_length != 0 {
            let offset = hdr.chunk_offset as usize;
            let n = hdr.packet_length as usize;
            chunk_buffer[offset..offset + n].copy_from_slice(&data[..n]);
            return true;
        }

        let chunk = self.chunk_buffer.take().unwrap_or_default();
        let result = self.gip_dispatch_packet(device, joystick, hdr, &chunk, self.chunk_length);

        // (HIDAPI_GIP_DestroyChunkBuffer())
        self.chunk_length = 0;

        result
    }

    /// Translation of `HIDAPI_GIP_ProcessPacket()`.
    fn gip_process_packet(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: Option<JoystickID>,
        hdr: &mut GipHeader,
        data: &[u8],
    ) -> bool {
        if hdr.options & GIP_OPT_CHUNK_START != 0 {
            if !self.gip_create_chunk_buffer(hdr.chunk_offset) {
                return false;
            }
            self.chunk_length = hdr.chunk_offset;

            hdr.chunk_offset = 0;
        }

        if hdr.options & GIP_OPT_ACKNOWLEDGE != 0
            && self.gip_acknowledge_packet(device, hdr).is_err()
        {
            return false;
        }

        if hdr.options & GIP_OPT_CHUNK != 0 {
            self.gip_process_packet_chunked(device, joystick, hdr, data)
        } else {
            let size = hdr.packet_length;
            self.gip_dispatch_packet(device, joystick, hdr, data, size)
        }
    }

    /// Translation of `HIDAPI_GIP_ProcessData()`; `data` is the report.
    fn gip_process_data(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: Option<JoystickID>,
        data: &[u8],
    ) -> bool {
        let mut data = data;

        while data.len() > GIP_HEADER_MIN_LENGTH {
            let size = data.len();
            let (mut hdr, hdr_len) = gip_decode_header(data);
            if hdr_len > size {
                // FIXME (upstream): a header longer than the rest of the
                // report makes the packet length wrap around, and the
                // packet is processed from past the report; here the
                // report ends.
                break;
            }
            if hdr_len as u64 + u64::from(hdr.packet_length) > size as u64 {
                // On macOS we get a shortened version of the real report
                hdr.packet_length = (size - hdr_len) as u32;
            }

            if !self.gip_process_packet(device, joystick, &mut hdr, &data[hdr_len..]) {
                return false;
            }

            data = &data[hdr_len + hdr.packet_length as usize..];
        }
        true
    }

    /// The Linux report descriptor setup of `HIDAPI_DriverXboxOne_InitDevice()`:
    /// the Xbox controller doesn't have real HID report descriptors, but
    /// Linux synthesizes them for us.
    #[cfg(target_os = "linux")]
    fn load_descriptor(&mut self, device: &DeviceCtx<'_>) {
        let mut descriptor = [0u8; 1024];
        let descriptor_len = device.get_report_descriptor(&mut descriptor).unwrap_or(0);
        if descriptor_len > 0 {
            super::dump_packet(
                "Xbox One report descriptor: size = {}",
                &descriptor[..descriptor_len],
            );

            match super::report_descriptor::parse_report_descriptor(&descriptor[..descriptor_len]) {
                Ok(parsed) => self.descriptor = collapse_descriptor_buttons(parsed),
                Err(e) => crate::log::warn!(
                    crate::log::Category::Input,
                    "Couldn't parse Xbox report descriptor: {}",
                    e.message()
                ),
            }
        } else {
            crate::log::debug!(
                crate::log::Category::Input,
                "Xbox report descriptor not available"
            );
        }
    }
}

#[cfg(any(test, target_os = "linux"))]
/// Collapse the buttons of a descriptor into a single field read, and keep
/// it if it has the expected usages (part of `HIDAPI_DriverXboxOne_InitDevice()`).
fn collapse_descriptor_buttons(mut descriptor: ReportDescriptor) -> Option<ReportDescriptor> {
    const BUTTON_1: u32 = make_usage(USB_USAGEPAGE_BUTTON, 1);

    let mut button_count = 0;
    if let Some(i) = descriptor
        .fields
        .iter()
        .position(|field| field.usage == BUTTON_1)
    {
        // The buttons with consecutive usages and offsets
        let first = descriptor.fields[i];
        button_count = descriptor.fields[i..]
            .iter()
            .zip(0..)
            .take_while(|&(other, n)| {
                other.usage == first.usage.wrapping_add(n as u32)
                    && other.bit_offset == first.bit_offset.wrapping_add(n)
            })
            .count() as i32;
        descriptor.fields[i].bit_size = button_count;

        descriptor.fields.drain(i + 1..i + button_count as usize);
    }
    if !descriptor.has_usage(USB_USAGEPAGE_GENERIC_DESKTOP, USB_USAGE_GENERIC_X)
        || !descriptor.has_usage(USB_USAGEPAGE_GENERIC_DESKTOP, USB_USAGE_GENERIC_Y)
        || (button_count != 12 && button_count != 15)
    {
        crate::log::warn!(
            crate::log::Category::Input,
            "Xbox report descriptor missing expected usages, ignoring"
        );
        return None;
    }
    Some(descriptor)
}

/// The Xbox One driver's static functions.
pub(crate) struct XboxOneDriver;

impl DriverImpl for XboxOneDriver {
    /// Translation of `HIDAPI_DriverXboxOne_RegisterHints()`.
    fn hints(&self) -> &'static [&'static str] {
        &[hints::JOYSTICK_HIDAPI_XBOX, hints::JOYSTICK_HIDAPI_XBOX_ONE]
    }

    /// Translation of `HIDAPI_DriverXboxOne_IsEnabled()`.
    fn is_enabled(&self) -> bool {
        hints::get_bool(
            hints::JOYSTICK_HIDAPI_XBOX_ONE,
            hints::get_bool(
                hints::JOYSTICK_HIDAPI_XBOX,
                hints::get_bool(hints::JOYSTICK_HIDAPI, SDL_HIDAPI_DEFAULT),
            ),
        )
    }

    /// Translation of `HIDAPI_DriverXboxOne_IsSupportedDevice()`.
    fn is_supported_device(
        &self,
        device: Option<&HidapiDevice>,
        _name: &str,
        gamepad_type: GamepadType,
        _vendor_id: u16,
        _product_id: u16,
        _version: u16,
        _interface_number: i32,
        interface_class: i32,
        interface_subclass: i32,
        interface_protocol: i32,
    ) -> bool {
        const LIBUSB_CLASS_VENDOR_SPEC: i32 = 0xFF;
        const XBONE_IFACE_SUBCLASS: i32 = 71;
        const XBONE_IFACE_PROTOCOL: i32 = 208;

        if cfg!(windows) && device.is_some_and(|d| d.path().starts_with(r"\\?\HID#")) {
            // Windows provides a fake HID endpoint for XGIP controllers, don't use this
            return false;
        }
        if interface_class != 0
            && (interface_class != LIBUSB_CLASS_VENDOR_SPEC
                || interface_subclass != XBONE_IFACE_SUBCLASS
                || interface_protocol != XBONE_IFACE_PROTOCOL)
        {
            // This isn't the Xbox gamepad interface
            return false;
        }
        gamepad_type == GamepadType::XboxOne
    }

    fn new_context(&self) -> Box<dyn DriverContext> {
        Box::new(XboxOneContext::default())
    }
}

impl DriverContext for XboxOneContext {
    /// Translation of `HIDAPI_DriverXboxOne_InitDevice()`.
    fn init_device(&mut self, device: &mut DeviceCtx<'_>) -> Result<()> {
        #[cfg(target_os = "linux")]
        self.load_descriptor(device);

        self.vendor_id = device.vendor_id();
        self.product_id = device.product_id();
        self.start_time = crate::timer::ticks_ms();
        self.sequence = 0;
        self.has_color_led = controller_has_color_led(self.vendor_id, self.product_id);
        self.has_paddles = controller_has_paddles(self.vendor_id, self.product_id);
        self.has_trigger_rumble = controller_has_trigger_rumble(self.vendor_id, self.product_id);
        self.has_share_button = controller_has_share_button(self.vendor_id, self.product_id);

        // Assume that the controller is correctly initialized when we start
        self.init_state = if !controller_sends_announcement(device.vendor_id(), device.product_id())
        {
            // Jump into the startup sequence for this controller
            InitState::Startup
        } else {
            InitState::Complete
        };

        device.set_gamepad_type(GamepadType::XboxOne);

        device.joystick_connected();
        Ok(())
    }

    /// Translation of `HIDAPI_DriverXboxOne_OpenJoystick()`.
    fn open_joystick(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: &mut JoystickData,
    ) -> Result<()> {
        crate::joystick::assert_joysticks_locked();

        self.low_frequency_rumble = 0;
        self.high_frequency_rumble = 0;
        self.left_trigger_rumble = 0;
        self.right_trigger_rumble = 0;
        self.rumble_state = RumbleState::Idle;
        self.rumble_time.store(0, Ordering::Release);
        self.rumble_pending = false;
        self.last_state = [0; USB_PACKET_LENGTH];

        // Initialize the joystick capabilities
        joystick.nbuttons = 11;
        if self.has_share_button {
            joystick.nbuttons += 1;
        }
        if self.has_paddles {
            joystick.nbuttons += 4;
        }
        joystick.naxes = GamepadAxis::COUNT;
        joystick.nhats = 1;

        self.home_led_hint = Some(HintWatch::new(hints::JOYSTICK_HIDAPI_XBOX_ONE_HOME_LED));
        self.home_led_hint_changed(device);
        Ok(())
    }

    /// Translation of `HIDAPI_DriverXboxOne_RumbleJoystick()`.
    fn rumble_joystick(
        &mut self,
        device: &mut DeviceCtx<'_>,
        _joystick: JoystickID,
        low_frequency_rumble: u16,
        high_frequency_rumble: u16,
    ) -> Result<()> {
        // Magnitude is 1..100 so scale the 16-bit input here
        self.low_frequency_rumble = (low_frequency_rumble / 655) as u8;
        self.high_frequency_rumble = (high_frequency_rumble / 655) as u8;
        self.rumble_pending = true;

        self.update_rumble(device)
    }

    /// Translation of `HIDAPI_DriverXboxOne_RumbleJoystickTriggers()`.
    fn rumble_joystick_triggers(
        &mut self,
        device: &mut DeviceCtx<'_>,
        _joystick: JoystickID,
        left_rumble: u16,
        right_rumble: u16,
    ) -> Result<()> {
        if !self.has_trigger_rumble {
            return Err(Error::unsupported());
        }

        // Magnitude is 1..100 so scale the 16-bit input here
        self.left_trigger_rumble = (left_rumble / 655) as u8;
        self.right_trigger_rumble = (right_rumble / 655) as u8;
        self.rumble_pending = true;

        self.update_rumble(device)
    }

    /// Translation of `HIDAPI_DriverXboxOne_GetJoystickCapabilities()`.
    fn get_joystick_capabilities(
        &mut self,
        _device: &mut DeviceCtx<'_>,
        _joystick: JoystickID,
    ) -> JoystickCaps {
        let mut result = JoystickCaps::RUMBLE;
        if self.has_trigger_rumble {
            result |= JoystickCaps::TRIGGER_RUMBLE;
        }

        if self.has_color_led {
            result |= JoystickCaps::RGB_LED;
        }

        result
    }

    /// Translation of `HIDAPI_DriverXboxOne_SetJoystickLED()`.
    fn set_joystick_led(
        &mut self,
        device: &mut DeviceCtx<'_>,
        _joystick: JoystickID,
        red: u8,
        green: u8,
        blue: u8,
    ) -> Result<()> {
        if !self.has_color_led {
            return Err(Error::unsupported());
        }

        let mut led_packet = [0x0E, 0x00, 0x00, 0x05, 0x00, 0x00, 0x00, 0x00, 0x00];

        led_packet[5] = 0x00; // Whiteness? Sets white intensity when RGB is 0, seems additive
        led_packet[6] = red;
        led_packet[7] = green;
        led_packet[8] = blue;

        if send_rumble(device.device(), &led_packet).ok() != Some(led_packet.len()) {
            return Err(Error::new("Couldn't send LED packet"));
        }
        Ok(())
    }

    /// Translation of `HIDAPI_DriverXboxOne_UpdateDevice()`.
    fn update_device(&mut self, device: &mut DeviceCtx<'_>) -> bool {
        let Some(&first) = device.joysticks().first() else {
            return false;
        };
        let joystick = device.joystick_open(first).then_some(first);

        // (the home LED hint callback of upstream)
        if joystick.is_some() {
            self.home_led_hint_changed(device);
        }

        let mut data = [0u8; USB_PACKET_LENGTH];
        let read_error = loop {
            let size = match device.read_timeout(&mut data, 0) {
                Ok(0) => break false,
                Ok(size) => size,
                Err(_) => break true,
            };

            if self.descriptor.is_some() {
                let Some(joystick) = joystick else {
                    break false;
                };
                self.handle_descriptor_report(device, joystick, &data, size);
            } else if device.is_bluetooth() {
                match data[0] {
                    0x01 => {
                        if let Some(joystick) = joystick {
                            if size >= 16 {
                                self.bluetooth_handle_state_packet(
                                    device, joystick, &mut data, size,
                                );
                            }
                            // (else an unknown Xbox One Bluetooth packet size)
                        }
                    }
                    0x02 => {
                        if let Some(joystick) = joystick {
                            self.bluetooth_handle_guide_packet(device, joystick, &data);
                        }
                    }
                    0x04 => {
                        if let Some(joystick) = joystick {
                            self.bluetooth_handle_battery_packet(device, joystick, &data);
                        }
                    }
                    _ => {
                        // Unknown Xbox One packet
                    }
                }
            } else {
                self.gip_process_data(device, joystick, &data[..size]);
            }
        };

        self.update_init_state(device);
        let _ = self.update_rumble(device);

        if read_error {
            // Read error, device is disconnected
            device.joystick_disconnected(first);
        }
        !read_error
    }

    /// Translation of `HIDAPI_DriverXboxOne_CloseJoystick()`.
    fn close_joystick(&mut self, _device: &mut DeviceCtx<'_>, _joystick: JoystickID) {
        self.home_led_hint = None;
    }

    /// Translation of `HIDAPI_DriverXboxOne_FreeDevice()`.
    fn free_device(&mut self, _device: &mut DeviceCtx<'_>) {
        self.descriptor = None;

        // (HIDAPI_GIP_DestroyChunkBuffer())
        self.chunk_buffer = None;
        self.chunk_length = 0;
    }
}

#[cfg(test)]
mod tests;
