// Rust translation of src/joystick/hidapi/SDL_hidapijoystick.c and
// SDL_hidapijoystick_c.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The HIDAPI joystick driver: game controllers driven directly through
//! their HID reports ([`crate::hidapi`]), by a device driver per kind of
//! controller. It is the highest priority joystick driver, so the platform
//! drivers leave the devices it handles to it.
//!
//! One HID device ([`HidapiDevice`]) can provide several joysticks (a
//! wireless receiver) or be part of one (the two Joy-Cons of a pair, which
//! the combined driver joins). The device drivers keep their state in a
//! per-device context ([`DriverContext`]) and reach the device and the
//! joystick front end through a [`DeviceCtx`].
//!
//! The driver contexts are called with the joystick lock held, as upstream.
//! The joystick events, connections and disconnections a driver reports
//! are delivered when its function returns (in order), since they can call
//! back into the joystick API (and into this driver) from event watchers.
//! Driver hint callbacks only record the change; the drivers act on it at
//! their next update, since hint callbacks run with the hint lock held and
//! can't take the joystick lock.
//!
//! Device drivers translated: all of upstream's. GameCube, Luna, SHIELD,
//! PS3 (with its third party and Sony Sixaxis drivers), Stadia, the Valve
//! controllers (Steam Controller, Wireless HORIPAD For Steam, Steam Deck and
//! the Triton Steam Controller), Nintendo Switch (with the combined
//! Joy-Cons), Nintendo Switch 2 (without its libusb setup, so it stays
//! disabled), Wii, Xbox 360 (wired, wireless and Big Button), GIP (wired Xbox
//! One), Xbox One, the Logitech wheels (lg4ff), PS4, PS5, 8BitDo, Flydigi,
//! SInput, GameSir and ZUIKI.

mod combined;
mod eightbitdo;
mod flydigi;
mod gamecube;
mod gamesir;
mod gip;
mod lg4ff;
mod luna;
mod ps3;
mod ps4;
mod ps5;
pub(crate) mod report_descriptor;
pub(crate) mod rumble;
mod shield;
mod sinput;
mod stadia;
mod steam;
mod steam_hori;
mod steam_triton;
mod steamdeck;
mod switch;
mod switch2;
mod wii;
mod xbox360;
mod xbox360bb;
mod xbox360w;
mod xboxone;
mod zuiki;

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::Duration;

use super::gamepad::{GamepadCapSenseType, GamepadType};
use super::usb_ids::*;
use super::{
    assert_joysticks_locked, create_joystick_guid, create_joystick_name, gamepad_type_from_vidpid,
    joystick_guid_info, lock_joysticks, private_joystick_added, private_joystick_removed,
    send_joystick_axis, send_joystick_button, send_joystick_capsense, send_joystick_hat,
    send_joystick_power_info, send_joystick_sensor, send_joystick_touchpad, set_joystick_guid_crc,
    should_ignore_joystick, with_joystick, JoystickConnectionState, JoystickData, JoystickDriver,
    JoystickType, HARDWARE_BUS_BLUETOOTH, HARDWARE_BUS_USB, PROP_JOYSTICK_CAP_MONO_LED_BOOLEAN,
    PROP_JOYSTICK_CAP_PLAYER_LED_BOOLEAN, PROP_JOYSTICK_CAP_RGB_LED_BOOLEAN,
    PROP_JOYSTICK_CAP_RUMBLE_BOOLEAN, PROP_JOYSTICK_CAP_TRIGGER_RUMBLE_BOOLEAN,
};
use crate::error::{Error, Result};
use crate::events::JoystickID;
use crate::guid::Guid;
use crate::hidapi::{BusType, DeviceInfo, HidDevice};
use crate::hints;
use crate::power::PowerState;
use crate::sensor::SensorType;

/// Whether HIDAPI is enabled by default (`SDL_HIDAPI_DEFAULT`). On Android,
/// HIDAPI prompts for permissions and acquires exclusive access to the
/// device, and on Apple mobile platforms it doesn't do anything except for
/// handling Bluetooth Steam Controllers, so it's off there by default.
pub(crate) const SDL_HIDAPI_DEFAULT: bool = !cfg!(any(target_os = "android", target_os = "ios"));

/// `SDL_NS_PER_US`
pub(crate) const NS_PER_US: u64 = 1000;

/// The maximum size of a USB packet for HID devices (`USB_PACKET_LENGTH`).
pub(crate) const USB_PACKET_LENGTH: usize = 64;

/// What a joystick can do (the `SDL_JOYSTICK_CAP_*` bits).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) struct JoystickCaps(pub(crate) u32);

impl JoystickCaps {
    /// `SDL_JOYSTICK_CAP_MONO_LED`
    pub(crate) const MONO_LED: JoystickCaps = JoystickCaps(0x00000001);
    /// `SDL_JOYSTICK_CAP_RGB_LED`
    pub(crate) const RGB_LED: JoystickCaps = JoystickCaps(0x00000002);
    /// `SDL_JOYSTICK_CAP_PLAYER_LED`
    pub(crate) const PLAYER_LED: JoystickCaps = JoystickCaps(0x00000004);
    /// `SDL_JOYSTICK_CAP_RUMBLE`
    pub(crate) const RUMBLE: JoystickCaps = JoystickCaps(0x00000010);
    /// `SDL_JOYSTICK_CAP_TRIGGER_RUMBLE`
    pub(crate) const TRIGGER_RUMBLE: JoystickCaps = JoystickCaps(0x00000020);

    pub(crate) fn contains(self, other: JoystickCaps) -> bool {
        self.0 & other.0 == other.0
    }
}

impl std::ops::BitOr for JoystickCaps {
    type Output = JoystickCaps;
    fn bitor(self, rhs: JoystickCaps) -> JoystickCaps {
        JoystickCaps(self.0 | rhs.0)
    }
}

impl std::ops::BitOrAssign for JoystickCaps {
    fn bitor_assign(&mut self, rhs: JoystickCaps) {
        self.0 |= rhs.0;
    }
}

/// `LOAD16(A, B)`
pub(crate) fn load16(a: u8, b: u8) -> i16 {
    u16::from_le_bytes([a, b]) as i16
}

/// `LOAD32(A, B, C, D)`
#[allow(dead_code)] // (used by drivers not translated yet)
pub(crate) fn load32(a: u8, b: u8, c: u8, d: u8) -> u32 {
    u32::from_le_bytes([a, b, c, d])
}

/// The functions of a kind of controller that don't belong to a device
/// (the static half of `SDL_HIDAPI_DeviceDriver`).
pub(crate) trait DriverImpl: Sync {
    /// The hints whose changes re-evaluate the drivers (`RegisterHints`).
    fn hints(&self) -> &'static [&'static str];

    /// Whether the driver is enabled by its hints (`IsEnabled`).
    fn is_enabled(&self) -> bool;

    /// Whether the driver handles a device (`IsSupportedDevice`). `device`
    /// is `None` when only the IDs are known (another driver asking whether
    /// a device is present); its HID device may not be open yet either.
    #[allow(clippy::too_many_arguments)]
    fn is_supported_device(
        &self,
        device: Option<&HidapiDevice>,
        name: &str,
        gamepad_type: GamepadType,
        vendor_id: u16,
        product_id: u16,
        version: u16,
        interface_number: i32,
        interface_class: i32,
        interface_subclass: i32,
        interface_protocol: i32,
    ) -> bool;

    /// A fresh context for a device the driver handles (the zeroed
    /// `device->context` of `InitDevice`).
    fn new_context(&self) -> Box<dyn DriverContext>;
}

/// A device driver. Translation of `SDL_HIDAPI_DeviceDriver`.
pub(crate) struct HidapiDeviceDriver {
    /// The hint that enables the driver (its `name`).
    pub(crate) name: &'static str,
    enabled: AtomicBool,
    pub(crate) imp: &'static dyn DriverImpl,
}

impl HidapiDeviceDriver {
    pub(crate) const fn new(
        name: &'static str,
        imp: &'static dyn DriverImpl,
    ) -> HidapiDeviceDriver {
        HidapiDeviceDriver {
            name,
            enabled: AtomicBool::new(true),
            imp,
        }
    }

    /// Whether the driver was enabled the last time the drivers were
    /// updated (`driver->enabled`).
    pub(crate) fn enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }
}

impl std::fmt::Debug for HidapiDeviceDriver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name)
    }
}

/// A hint a device driver watches while a joystick is open (the
/// `SDL_AddHintCallback()` of an `OpenJoystick`, removed by dropping it).
///
/// Hint callbacks run with the hint lock held, on whichever thread changes
/// the hint, so they can't take the joystick lock; the callback records
/// the new value, which the driver takes ([`HintWatch::take`]) and applies
/// when it next runs (right after opening, for the value the callback is
/// called with when it is added, then at its updates).
pub(crate) struct HintWatch {
    change: Arc<Mutex<Option<Option<String>>>>,
    _callback: Option<hints::Callback>,
}

impl HintWatch {
    /// Watch a hint; its current value is the first change.
    pub(crate) fn new(name: &'static str) -> HintWatch {
        let change = Arc::new(Mutex::new(None));
        let recorder = change.clone();
        let callback = hints::watch(name, move |hint_change| {
            *recorder.lock().unwrap_or_else(|e| e.into_inner()) =
                Some(hint_change.new_value.map(str::to_owned));
        })
        .ok();
        HintWatch {
            change,
            _callback: callback,
        }
    }

    /// The hint's value if it changed since the last call (the value the
    /// callback would have been called with).
    pub(crate) fn take(&self) -> Option<Option<String>> {
        self.change.lock().unwrap_or_else(|e| e.into_inner()).take()
    }
}

impl std::fmt::Debug for HintWatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HintWatch")
    }
}

/// The joystick a driver changes (`ctx->joystick`): the one being opened,
/// which isn't in the joystick list yet, or an open one.
pub(crate) enum JoystickRef<'a> {
    /// The joystick of an `OpenJoystick`
    Opening(&'a mut JoystickData),
    /// An open joystick
    Open(JoystickID),
}

impl JoystickRef<'_> {
    /// Run `f` on the joystick; `None` if it isn't open (anymore). `f`
    /// must not look up joysticks itself.
    pub(crate) fn with<R>(&mut self, f: impl FnOnce(&mut JoystickData) -> R) -> Option<R> {
        match self {
            JoystickRef::Opening(joystick) => Some(f(joystick)),
            JoystickRef::Open(id) => with_joystick(*id, f),
        }
    }
}

/// The per-device functions of a device driver (the functions of
/// `SDL_HIDAPI_DeviceDriver` taking a device, with `device->context` as
/// `self`). The defaults are the drivers' "unsupported" functions.
pub(crate) trait DriverContext: Send {
    /// Set up a device; may connect joysticks (`InitDevice`).
    fn init_device(&mut self, device: &mut DeviceCtx<'_>) -> Result<()>;

    /// `GetDevicePlayerIndex`
    fn get_device_player_index(
        &mut self,
        _device: &mut DeviceCtx<'_>,
        _instance_id: JoystickID,
    ) -> i32 {
        -1
    }

    /// `SetDevicePlayerIndex`
    fn set_device_player_index(
        &mut self,
        _device: &mut DeviceCtx<'_>,
        _instance_id: JoystickID,
        _player_index: i32,
    ) {
    }

    /// Read and handle the device's reports; `false` on a read error
    /// (`UpdateDevice`).
    fn update_device(&mut self, device: &mut DeviceCtx<'_>) -> bool;

    /// `OpenJoystick`
    fn open_joystick(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: &mut JoystickData,
    ) -> Result<()>;

    /// `RumbleJoystick`
    fn rumble_joystick(
        &mut self,
        _device: &mut DeviceCtx<'_>,
        _joystick: JoystickID,
        _low_frequency_rumble: u16,
        _high_frequency_rumble: u16,
    ) -> Result<()> {
        Err(Error::unsupported())
    }

    /// `RumbleJoystickTriggers`
    fn rumble_joystick_triggers(
        &mut self,
        _device: &mut DeviceCtx<'_>,
        _joystick: JoystickID,
        _left_rumble: u16,
        _right_rumble: u16,
    ) -> Result<()> {
        Err(Error::unsupported())
    }

    /// `GetJoystickCapabilities`
    fn get_joystick_capabilities(
        &mut self,
        _device: &mut DeviceCtx<'_>,
        _joystick: JoystickID,
    ) -> JoystickCaps {
        JoystickCaps(0)
    }

    /// `SetJoystickLED`
    fn set_joystick_led(
        &mut self,
        _device: &mut DeviceCtx<'_>,
        _joystick: JoystickID,
        _red: u8,
        _green: u8,
        _blue: u8,
    ) -> Result<()> {
        Err(Error::unsupported())
    }

    /// `SendJoystickEffect`
    fn send_joystick_effect(
        &mut self,
        _device: &mut DeviceCtx<'_>,
        _joystick: JoystickID,
        _data: &[u8],
    ) -> Result<()> {
        Err(Error::unsupported())
    }

    /// `SetJoystickSensorsEnabled`
    fn set_joystick_sensors_enabled(
        &mut self,
        _device: &mut DeviceCtx<'_>,
        _joystick: JoystickID,
        _enabled: bool,
    ) -> Result<()> {
        Err(Error::unsupported())
    }

    /// `CloseJoystick`
    fn close_joystick(&mut self, device: &mut DeviceCtx<'_>, joystick: JoystickID);

    /// `FreeDevice`
    fn free_device(&mut self, _device: &mut DeviceCtx<'_>) {}

    /// The power state the controller last reported, for the other half
    /// of a combined device (where upstream reads the other device's
    /// context).
    fn power_info(&self) -> Option<(PowerState, i32)> {
        None
    }
}

/// The changing part of a device.
struct DeviceState {
    name: String,
    serial: Option<String>,
    guid: Guid,
    joystick_type: JoystickType,
    gamepad_type: GamepadType,
    steam_virtual_gamepad_slot: i32,
    driver: Option<&'static HidapiDeviceDriver>,
    joysticks: Vec<JoystickID>,
    /// Used during scanning for device changes
    seen: bool,
    /// Used to flag devices that failed open. This can happen on Windows
    /// with Bluetooth devices that have turned off
    broken: bool,
    parent: Weak<HidapiDevice>,
    children: Vec<Arc<HidapiDevice>>,
}

/// A HID device that may be a game controller. Translation of
/// `SDL_HIDAPI_Device`.
pub(crate) struct HidapiDevice {
    manufacturer_string: Option<String>,
    product_string: Option<String>,
    path: String,
    vendor_id: u16,
    product_id: u16,
    version: u16,
    /// Available on Windows and Linux
    interface_number: i32,
    interface_class: i32,
    interface_subclass: i32,
    interface_protocol: i32,
    /// Available on Windows and macOS
    usage_page: u16,
    /// Available on Windows and macOS
    usage: u16,
    is_bluetooth: bool,

    state: Mutex<DeviceState>,
    /// The device's driver context (`context`); taken out while a driver
    /// function runs.
    context: Mutex<Option<Box<dyn DriverContext>>>,
    /// The open HID device (`dev`), shared with the rumble thread.
    dev: Mutex<Option<Arc<HidDevice>>>,
    /// The rumble requests queued for this device.
    pub(crate) rumble_pending: AtomicI32,
    /// `SDL_ObjectValid(device, SDL_OBJECT_TYPE_HIDAPI_JOYSTICK)`
    valid: AtomicBool,
}

impl std::fmt::Debug for HidapiDevice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HidapiDevice")
            .field("path", &self.path)
            .field("vendor_id", &self.vendor_id)
            .field("product_id", &self.product_id)
            .finish_non_exhaustive()
    }
}

impl HidapiDevice {
    fn state(&self) -> MutexGuard<'_, DeviceState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(crate) fn path(&self) -> &str {
        &self.path
    }
    pub(crate) fn vendor_id(&self) -> u16 {
        self.vendor_id
    }
    pub(crate) fn product_id(&self) -> u16 {
        self.product_id
    }
    pub(crate) fn version(&self) -> u16 {
        self.version
    }
    pub(crate) fn interface_number(&self) -> i32 {
        self.interface_number
    }
    #[allow(dead_code)] // (used by drivers not translated yet)
    pub(crate) fn usage_page(&self) -> u16 {
        self.usage_page
    }
    pub(crate) fn is_bluetooth(&self) -> bool {
        self.is_bluetooth
    }
    pub(crate) fn product_string(&self) -> Option<&str> {
        self.product_string.as_deref()
    }
    pub(crate) fn name(&self) -> String {
        self.state().name.clone()
    }
    pub(crate) fn serial(&self) -> Option<String> {
        self.state().serial.clone()
    }
    pub(crate) fn guid(&self) -> Guid {
        self.state().guid
    }
    pub(crate) fn gamepad_type(&self) -> GamepadType {
        self.state().gamepad_type
    }
    fn driver(&self) -> Option<&'static HidapiDeviceDriver> {
        self.state().driver
    }
    fn broken(&self) -> bool {
        self.state().broken
    }
    /// The joysticks of the device (`joysticks`).
    pub(crate) fn joysticks(&self) -> Vec<JoystickID> {
        self.state().joysticks.clone()
    }
    /// `device->num_joysticks`
    pub(crate) fn num_joysticks(&self) -> usize {
        self.state().joysticks.len()
    }
    pub(crate) fn parent(&self) -> Option<Arc<HidapiDevice>> {
        self.state().parent.upgrade()
    }
    pub(crate) fn children(&self) -> Vec<Arc<HidapiDevice>> {
        self.state().children.clone()
    }

    /// The power info of the device's driver context (`DriverContext::power_info`).
    pub(crate) fn context_power_info(&self) -> Option<(PowerState, i32)> {
        self.context
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .and_then(|context| context.power_info())
    }

    /// The open HID device (`device->dev`).
    pub(crate) fn dev(&self) -> Option<Arc<HidDevice>> {
        self.dev.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn set_dev(&self, dev: Option<Arc<HidDevice>>) {
        *self.dev.lock().unwrap_or_else(|e| e.into_inner()) = dev;
    }
}

/// An action of a driver function, delivered once it returns.
enum Pending {
    Axis(u64, JoystickID, u8, i16),
    Button(u64, JoystickID, u8, bool),
    Hat(u64, JoystickID, u8, u8),
    Touchpad(u64, JoystickID, i32, i32, bool, f32, f32, f32),
    Sensor(u64, JoystickID, SensorType, u64, [f32; 3], usize),
    CapSense(u64, JoystickID, GamepadCapSenseType, bool),
    PowerInfo(JoystickID, PowerState, i32),
    /// The `SDL_PrivateJoystickAdded()` of `HIDAPI_JoystickConnected()`
    Added(JoystickID),
    /// The joystick closing and `SDL_PrivateJoystickRemoved()` of
    /// `HIDAPI_JoystickDisconnected()`
    Removed(Arc<HidapiDevice>, JoystickID),
    /// `HIDAPI_UpdateDeviceProperties()`
    UpdateProperties(Arc<HidapiDevice>),
}

/// A device as a driver function sees it: the device and the joystick
/// front end (the `SDL_HIDAPI_Device *device` of the driver functions).
pub(crate) struct DeviceCtx<'a> {
    device: &'a Arc<HidapiDevice>,
    pending: &'a mut Vec<Pending>,
}

impl std::ops::Deref for DeviceCtx<'_> {
    type Target = HidapiDevice;
    fn deref(&self) -> &HidapiDevice {
        self.device
    }
}

impl DeviceCtx<'_> {
    /// The device.
    pub(crate) fn device(&self) -> &Arc<HidapiDevice> {
        self.device
    }

    /// The open HID device, or an error if there is none.
    fn hid(&self) -> Result<Arc<HidDevice>> {
        self.device
            .dev()
            .ok_or_else(|| Error::invalid_param("device"))
    }

    /// `SDL_hid_read_timeout(device->dev, ...)`
    pub(crate) fn read_timeout(&self, data: &mut [u8], milliseconds: i32) -> Result<usize> {
        self.hid()?.read_timeout_ms(data, milliseconds)
    }

    /// `SDL_hid_write(device->dev, ...)`
    pub(crate) fn write(&self, data: &[u8]) -> Result<usize> {
        self.hid()?.write(data)
    }

    /// `SDL_hid_get_report_descriptor(device->dev, ...)`
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub(crate) fn get_report_descriptor(&self, buf: &mut [u8]) -> Result<usize> {
        self.hid()?.report_descriptor(buf)
    }

    /// `device->type = ...`
    pub(crate) fn set_gamepad_type(&self, gamepad_type: GamepadType) {
        self.device.state().gamepad_type = gamepad_type;
    }

    /// `device->joystick_type = ...`
    pub(crate) fn set_joystick_type(&self, joystick_type: JoystickType) {
        self.device.state().joystick_type = joystick_type;
    }

    /// `device->steam_virtual_gamepad_slot = ...`
    pub(crate) fn set_steam_virtual_gamepad_slot(&self, slot: i32) {
        self.device.state().steam_virtual_gamepad_slot = slot;
    }

    /// `device->guid.data[index] = value`
    pub(crate) fn set_guid_byte(&self, index: usize, value: u8) {
        self.device.state().guid.0[index] = value;
    }

    /// Translation of `HIDAPI_SetDeviceName()`.
    pub(crate) fn set_device_name(&self, name: &str) {
        let mut state = self.device.state();
        if !name.is_empty() && name != state.name {
            state.name = name.to_owned();
            set_joystick_guid_crc(&mut state.guid, crate::stdlib::crc16(0, name.as_bytes()));
        }
    }

    /// Translation of `HIDAPI_SetDeviceProduct()`.
    #[allow(dead_code)] // (used by drivers not translated yet)
    pub(crate) fn set_device_product(&self, vendor_id: u16, product_id: u16) {
        let mut state = self.device.state();
        // Don't set the device product ID directly, or we'll constantly re-enumerate this device
        state.guid = create_joystick_guid(
            u16::from(state.guid.0[0]),
            vendor_id,
            product_id,
            self.device.version,
            self.device.manufacturer_string.as_deref(),
            self.device.product_string.as_deref(),
            b'h',
            0,
        );
    }

    /// Translation of `HIDAPI_SetDeviceSerial()`.
    pub(crate) fn set_device_serial(&self, serial: &str) {
        set_device_serial(self.device, serial);
    }

    /// Forget the device's serial (`device->serial = NULL`).
    pub(crate) fn clear_device_serial(&self) {
        self.device.state().serial = None;
    }

    /// Whether a joystick is open (`SDL_GetJoystickFromID() != NULL`).
    pub(crate) fn joystick_open(&self, joystick: JoystickID) -> bool {
        joystick != 0 && with_joystick(joystick, |_| ()).is_some()
    }

    /// The first joystick of the device, if it is open (`joystick =
    /// SDL_GetJoystickFromID(device->joysticks[0])`).
    pub(crate) fn open_joystick_id(&self) -> Option<JoystickID> {
        self.device
            .joysticks()
            .first()
            .copied()
            .filter(|&id| self.joystick_open(id))
    }

    /// Translation of `HIDAPI_JoystickConnected()`: a new joystick on this
    /// device (and its children); the added event is sent when the driver
    /// function returns.
    pub(crate) fn joystick_connected(&mut self) -> JoystickID {
        joystick_connected(self.device, self.pending)
    }

    /// Translation of `HIDAPI_JoystickDisconnected()`; the joystick is
    /// closed and the removed event sent when the driver function returns.
    pub(crate) fn joystick_disconnected(&mut self, joystick: JoystickID) {
        joystick_disconnected(self.device, joystick, self.pending);
    }

    /// Translation of `HIDAPI_UpdateDeviceProperties()`, done when the
    /// driver function returns.
    pub(crate) fn update_device_properties(&mut self) {
        self.pending
            .push(Pending::UpdateProperties(self.device.clone()));
    }

    /// Translation of `HIDAPI_HasConnectedUSBDevice()`.
    pub(crate) fn has_connected_usb_device(&self, serial: Option<&str>) -> bool {
        has_connected_usb_device(serial)
    }

    /// Translation of `HIDAPI_DisconnectBluetoothDevice()`.
    pub(crate) fn disconnect_bluetooth_device(&mut self, serial: Option<&str>) {
        assert_joysticks_locked();

        let Some(serial) = serial else {
            return;
        };

        for device in devices() {
            let (has_driver, broken) = {
                let s = device.state();
                (s.driver.is_some(), s.broken)
            };
            if !has_driver || broken {
                continue;
            }

            if !device.is_bluetooth {
                continue;
            }

            if device.serial().as_deref() == Some(serial) {
                while let Some(&joystick) = device.joysticks().first() {
                    joystick_disconnected(&device, joystick, self.pending);
                }
            }
        }
    }

    /// `SDL_SendJoystickAxis()`
    pub(crate) fn send_axis(&mut self, timestamp: u64, joystick: JoystickID, axis: u8, value: i16) {
        self.pending
            .push(Pending::Axis(timestamp, joystick, axis, value));
    }

    /// `SDL_SendJoystickButton()`
    pub(crate) fn send_button(
        &mut self,
        timestamp: u64,
        joystick: JoystickID,
        button: u8,
        down: bool,
    ) {
        self.pending
            .push(Pending::Button(timestamp, joystick, button, down));
    }

    /// `SDL_SendJoystickHat()`
    pub(crate) fn send_hat(&mut self, timestamp: u64, joystick: JoystickID, hat: u8, value: u8) {
        self.pending
            .push(Pending::Hat(timestamp, joystick, hat, value));
    }

    /// `SDL_SendJoystickTouchpad()`
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn send_touchpad(
        &mut self,
        timestamp: u64,
        joystick: JoystickID,
        touchpad: i32,
        finger: i32,
        down: bool,
        x: f32,
        y: f32,
        pressure: f32,
    ) {
        self.pending.push(Pending::Touchpad(
            timestamp, joystick, touchpad, finger, down, x, y, pressure,
        ));
    }

    /// `SDL_SendJoystickSensor()`
    pub(crate) fn send_sensor(
        &mut self,
        timestamp: u64,
        joystick: JoystickID,
        sensor_type: SensorType,
        sensor_timestamp: u64,
        data: &[f32],
    ) {
        let mut values = [0.0; 3];
        let num_values = data.len().min(3);
        values[..num_values].copy_from_slice(&data[..num_values]);
        self.pending.push(Pending::Sensor(
            timestamp,
            joystick,
            sensor_type,
            sensor_timestamp,
            values,
            num_values,
        ));
    }

    /// `SDL_SendJoystickCapSense()`
    pub(crate) fn send_capsense(
        &mut self,
        timestamp: u64,
        joystick: JoystickID,
        capsense_type: GamepadCapSenseType,
        down: bool,
    ) {
        self.pending
            .push(Pending::CapSense(timestamp, joystick, capsense_type, down));
    }

    /// `SDL_SendJoystickPowerInfo()`
    pub(crate) fn send_power_info(
        &mut self,
        joystick: JoystickID,
        state: PowerState,
        percent: i32,
    ) {
        self.pending
            .push(Pending::PowerInfo(joystick, state, percent));
    }

    /// Run a function of a child device's driver (the combined driver
    /// calling `child->driver->...`), within this call.
    pub(crate) fn with_child<R>(
        &mut self,
        child: &Arc<HidapiDevice>,
        f: impl FnOnce(&mut dyn DriverContext, &mut DeviceCtx<'_>) -> R,
    ) -> Option<R> {
        call_context(child, self.pending, f)
    }
}

/// Take a device's context out, run `f` on it and put it back; `None` if
/// the device has no context (or its context is busy in an outer call).
fn call_context<R>(
    device: &Arc<HidapiDevice>,
    pending: &mut Vec<Pending>,
    f: impl FnOnce(&mut dyn DriverContext, &mut DeviceCtx<'_>) -> R,
) -> Option<R> {
    let mut context = device
        .context
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take()?;
    let result = {
        let mut dctx = DeviceCtx { device, pending };
        f(&mut *context, &mut dctx)
    };
    let mut slot = device.context.lock().unwrap_or_else(|e| e.into_inner());
    if slot.is_none() && device.driver().is_some() {
        *slot = Some(context);
    }
    Some(result)
}

/// Call a driver function and deliver what it reported.
fn with_context<R>(
    device: &Arc<HidapiDevice>,
    f: impl FnOnce(&mut dyn DriverContext, &mut DeviceCtx<'_>) -> R,
) -> Option<R> {
    let mut pending = Vec::new();
    let result = call_context(device, &mut pending, f);
    deliver(pending);
    result
}

/// Deliver the actions of a driver function, in order.
fn deliver(pending: Vec<Pending>) {
    for action in pending {
        match action {
            Pending::Axis(timestamp, joystick, axis, value) => {
                send_joystick_axis(timestamp, joystick, axis, value)
            }
            Pending::Button(timestamp, joystick, button, down) => {
                send_joystick_button(timestamp, joystick, button, down)
            }
            Pending::Hat(timestamp, joystick, hat, value) => {
                send_joystick_hat(timestamp, joystick, hat, value)
            }
            Pending::Touchpad(timestamp, joystick, touchpad, finger, down, x, y, pressure) => {
                send_joystick_touchpad(timestamp, joystick, touchpad, finger, down, x, y, pressure)
            }
            Pending::Sensor(
                timestamp,
                joystick,
                sensor_type,
                sensor_timestamp,
                data,
                num_values,
            ) => send_joystick_sensor(
                timestamp,
                joystick,
                sensor_type,
                sensor_timestamp,
                &data[..num_values],
            ),
            Pending::CapSense(timestamp, joystick, capsense_type, down) => {
                send_joystick_capsense(timestamp, joystick, capsense_type, down)
            }
            Pending::PowerInfo(joystick, state, percent) => {
                send_joystick_power_info(joystick, state, percent)
            }
            Pending::Added(joystick) => private_joystick_added(joystick),
            Pending::Removed(device, joystick) => {
                // (the HIDAPI_JoystickClose() of HIDAPI_JoystickDisconnected())
                if let Some(Some(hwdata)) = with_joystick(joystick, |j| j.hwdata.take()) {
                    if hwdata.downcast_ref::<HwData>().is_some() {
                        joystick_close_device(&device, joystick);
                    }
                }

                if !SHUTTING_DOWN.load(Ordering::Relaxed) {
                    private_joystick_removed(joystick);
                }
            }
            Pending::UpdateProperties(device) => {
                // (translation of HIDAPI_UpdateDeviceProperties())
                let _lock = lock_joysticks();
                for joystick in device.joysticks() {
                    if let Some(props) = with_joystick(joystick, |j| j.properties()) {
                        update_joystick_properties(&device, joystick, &props);
                    }
                }
            }
        }
    }
}

/// `struct joystick_hwdata`: the device of an open joystick.
struct HwData {
    device: Arc<HidapiDevice>,
}

// The drivers, in priority order

/// The combined Joy-Cons driver (`SDL_HIDAPI_DriverCombined`), used for
/// devices with children; it isn't in the driver list.
pub(crate) static DRIVER_COMBINED: HidapiDeviceDriver =
    HidapiDeviceDriver::new("SDL_JOYSTICK_HIDAPI_COMBINED", &combined::CombinedDriver);

/// `SDL_HIDAPI_DriverGameCube`
pub(crate) static DRIVER_GAMECUBE: HidapiDeviceDriver =
    HidapiDeviceDriver::new(hints::JOYSTICK_HIDAPI_GAMECUBE, &gamecube::GameCubeDriver);
/// `SDL_HIDAPI_DriverLuna`
pub(crate) static DRIVER_LUNA: HidapiDeviceDriver =
    HidapiDeviceDriver::new(hints::JOYSTICK_HIDAPI_LUNA, &luna::LunaDriver);
/// `SDL_HIDAPI_DriverShield`
pub(crate) static DRIVER_SHIELD: HidapiDeviceDriver =
    HidapiDeviceDriver::new(hints::JOYSTICK_HIDAPI_SHIELD, &shield::ShieldDriver);
/// `SDL_HIDAPI_DriverPS3`
pub(crate) static DRIVER_PS3: HidapiDeviceDriver =
    HidapiDeviceDriver::new(hints::JOYSTICK_HIDAPI_PS3, &ps3::Ps3Driver);
/// `SDL_HIDAPI_DriverPS3ThirdParty`
pub(crate) static DRIVER_PS3_THIRD_PARTY: HidapiDeviceDriver =
    HidapiDeviceDriver::new(hints::JOYSTICK_HIDAPI_PS3, &ps3::Ps3ThirdPartyDriver);
/// `SDL_HIDAPI_DriverPS3SonySixaxis`
pub(crate) static DRIVER_PS3_SONY_SIXAXIS: HidapiDeviceDriver = HidapiDeviceDriver::new(
    hints::JOYSTICK_HIDAPI_PS3_SIXAXIS_DRIVER,
    &ps3::Ps3SonySixaxisDriver,
);
/// `SDL_HIDAPI_DriverPS4`
pub(crate) static DRIVER_PS4: HidapiDeviceDriver =
    HidapiDeviceDriver::new(hints::JOYSTICK_HIDAPI_PS4, &ps4::Ps4Driver);
/// `SDL_HIDAPI_DriverPS5`
pub(crate) static DRIVER_PS5: HidapiDeviceDriver =
    HidapiDeviceDriver::new(hints::JOYSTICK_HIDAPI_PS5, &ps5::Ps5Driver);
/// `SDL_HIDAPI_DriverStadia`
pub(crate) static DRIVER_STADIA: HidapiDeviceDriver =
    HidapiDeviceDriver::new(hints::JOYSTICK_HIDAPI_STADIA, &stadia::StadiaDriver);
/// `SDL_HIDAPI_DriverSteam`
pub(crate) static DRIVER_STEAM: HidapiDeviceDriver =
    HidapiDeviceDriver::new(hints::JOYSTICK_HIDAPI_STEAM, &steam::SteamDriver);
/// `SDL_HIDAPI_DriverSteamHori`
pub(crate) static DRIVER_STEAM_HORI: HidapiDeviceDriver = HidapiDeviceDriver::new(
    hints::JOYSTICK_HIDAPI_STEAM_HORI,
    &steam_hori::SteamHoriDriver,
);
/// `SDL_HIDAPI_DriverSteamDeck`
pub(crate) static DRIVER_STEAMDECK: HidapiDeviceDriver = HidapiDeviceDriver::new(
    hints::JOYSTICK_HIDAPI_STEAMDECK,
    &steamdeck::SteamDeckDriver,
);
/// `SDL_HIDAPI_DriverSteamTriton`
pub(crate) static DRIVER_STEAM_TRITON: HidapiDeviceDriver = HidapiDeviceDriver::new(
    hints::JOYSTICK_HIDAPI_STEAM,
    &steam_triton::SteamTritonDriver,
);
/// `SDL_HIDAPI_DriverNintendoClassic`
pub(crate) static DRIVER_NINTENDO_CLASSIC: HidapiDeviceDriver = HidapiDeviceDriver::new(
    hints::JOYSTICK_HIDAPI_NINTENDO_CLASSIC,
    &switch::NintendoClassicDriver,
);
/// `SDL_HIDAPI_DriverJoyCons`
pub(crate) static DRIVER_JOYCONS: HidapiDeviceDriver =
    HidapiDeviceDriver::new(hints::JOYSTICK_HIDAPI_JOY_CONS, &switch::JoyConsDriver);
/// `SDL_HIDAPI_DriverSwitch`
pub(crate) static DRIVER_SWITCH: HidapiDeviceDriver =
    HidapiDeviceDriver::new(hints::JOYSTICK_HIDAPI_SWITCH, &switch::SwitchDriver);
/// `SDL_HIDAPI_DriverSwitch2`
pub(crate) static DRIVER_SWITCH2: HidapiDeviceDriver =
    HidapiDeviceDriver::new(hints::JOYSTICK_HIDAPI_SWITCH2, &switch2::Switch2Driver);
/// `SDL_HIDAPI_DriverWii`
pub(crate) static DRIVER_WII: HidapiDeviceDriver =
    HidapiDeviceDriver::new(hints::JOYSTICK_HIDAPI_WII, &wii::WiiDriver);
/// `SDL_HIDAPI_DriverXbox360`
pub(crate) static DRIVER_XBOX360: HidapiDeviceDriver =
    HidapiDeviceDriver::new(hints::JOYSTICK_HIDAPI_XBOX_360, &xbox360::Xbox360Driver);
/// `SDL_HIDAPI_DriverXbox360W`
pub(crate) static DRIVER_XBOX360W: HidapiDeviceDriver = HidapiDeviceDriver::new(
    hints::JOYSTICK_HIDAPI_XBOX_360_WIRELESS,
    &xbox360w::Xbox360WDriver,
);
/// `SDL_HIDAPI_DriverXbox360BB`
pub(crate) static DRIVER_XBOX360BB: HidapiDeviceDriver =
    HidapiDeviceDriver::new(xbox360bb::DRIVER_NAME, &xbox360bb::Xbox360BbDriver);
/// `SDL_HIDAPI_DriverGIP`
pub(crate) static DRIVER_GIP: HidapiDeviceDriver =
    HidapiDeviceDriver::new(hints::JOYSTICK_HIDAPI_GIP, &gip::GipDriver);
/// `SDL_HIDAPI_DriverXboxOne`
pub(crate) static DRIVER_XBOXONE: HidapiDeviceDriver =
    HidapiDeviceDriver::new(hints::JOYSTICK_HIDAPI_XBOX_ONE, &xboxone::XboxOneDriver);
/// `SDL_HIDAPI_DriverLg4ff`
pub(crate) static DRIVER_LG4FF: HidapiDeviceDriver =
    HidapiDeviceDriver::new(hints::JOYSTICK_HIDAPI_LG4FF, &lg4ff::Lg4ffDriver);
/// `SDL_HIDAPI_Driver8BitDo`
pub(crate) static DRIVER_8BITDO: HidapiDeviceDriver =
    HidapiDeviceDriver::new(hints::JOYSTICK_HIDAPI_8BITDO, &eightbitdo::EightBitDoDriver);
/// `SDL_HIDAPI_DriverFlydigi`
pub(crate) static DRIVER_FLYDIGI: HidapiDeviceDriver =
    HidapiDeviceDriver::new(hints::JOYSTICK_HIDAPI_FLYDIGI, &flydigi::FlydigiDriver);
/// `SDL_HIDAPI_DriverSInput`
pub(crate) static DRIVER_SINPUT: HidapiDeviceDriver =
    HidapiDeviceDriver::new(hints::JOYSTICK_HIDAPI_SINPUT, &sinput::SInputDriver);
/// `SDL_HIDAPI_DriverGameSir`
pub(crate) static DRIVER_GAMESIR: HidapiDeviceDriver =
    HidapiDeviceDriver::new(hints::JOYSTICK_HIDAPI_GAMESIR, &gamesir::GameSirDriver);
/// `SDL_HIDAPI_DriverZUIKI`
pub(crate) static DRIVER_ZUIKI: HidapiDeviceDriver =
    HidapiDeviceDriver::new(hints::JOYSTICK_HIDAPI_ZUIKI, &zuiki::ZuikiDriver);

/// Translation of `SDL_HIDAPI_drivers`, in upstream's order.
static HIDAPI_DRIVERS: &[&HidapiDeviceDriver] = &[
    &DRIVER_GAMECUBE,
    &DRIVER_LUNA,
    &DRIVER_SHIELD,
    &DRIVER_PS3,
    &DRIVER_PS3_THIRD_PARTY,
    &DRIVER_PS3_SONY_SIXAXIS,
    &DRIVER_PS4,
    &DRIVER_PS5,
    &DRIVER_STADIA,
    &DRIVER_STEAM,
    &DRIVER_STEAM_HORI,
    &DRIVER_STEAMDECK,
    &DRIVER_STEAM_TRITON,
    &DRIVER_NINTENDO_CLASSIC,
    &DRIVER_JOYCONS,
    &DRIVER_SWITCH,
    &DRIVER_SWITCH2,
    &DRIVER_WII,
    &DRIVER_XBOX360,
    &DRIVER_XBOX360W,
    &DRIVER_XBOX360BB,
    &DRIVER_GIP,
    &DRIVER_XBOXONE,
    &DRIVER_LG4FF,
    &DRIVER_8BITDO,
    &DRIVER_FLYDIGI,
    &DRIVER_SINPUT,
    &DRIVER_GAMESIR,
    &DRIVER_ZUIKI,
];

// The framework state

/// Translation of `SDL_HIDAPI_numdrivers`.
static NUMDRIVERS: AtomicUsize = AtomicUsize::new(0);
/// Translation of `SDL_HIDAPI_updating_devices`.
static UPDATING_DEVICES: AtomicBool = AtomicBool::new(false);
/// Translation of `SDL_HIDAPI_hints_changed`.
static HINTS_CHANGED: AtomicBool = AtomicBool::new(false);
/// Translation of `SDL_HIDAPI_change_count`.
static CHANGE_COUNT: AtomicU32 = AtomicU32::new(0);
/// Translation of `SDL_HIDAPI_numjoysticks`.
static NUMJOYSTICKS: AtomicUsize = AtomicUsize::new(0);
/// Translation of `SDL_HIDAPI_combine_joycons`.
static COMBINE_JOYCONS: AtomicBool = AtomicBool::new(true);
/// Translation of `initialized`.
static INITIALIZED: AtomicBool = AtomicBool::new(false);
/// Translation of `shutting_down`.
static SHUTTING_DOWN: AtomicBool = AtomicBool::new(false);

/// Translation of `SDL_HIDAPI_devices`, guarded by the joystick lock (the
/// mutex is only held to read or change the list).
static DEVICES: Mutex<Vec<Arc<HidapiDevice>>> = Mutex::new(Vec::new());
/// The hint callbacks of `HIDAPI_JoystickInit()`.
static HINT_WATCHES: Mutex<Vec<hints::Callback>> = Mutex::new(Vec::new());

/// The devices, in order.
fn devices() -> Vec<Arc<HidapiDevice>> {
    DEVICES.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Translation of `HIDAPI_ConvertString()` (the strings are already UTF-8).
fn convert_string(wide_string: Option<&str>) -> Option<String> {
    wide_string.map(str::to_owned)
}

/// Translation of `HIDAPI_DumpPacket()`: log a packet at debug priority.
/// `prefix` contains `{}` for the size.
#[allow(dead_code)] // (the drivers' debug logging)
pub(crate) fn dump_packet(prefix: &str, data: &[u8]) {
    let mut buffer = prefix.replacen("{}", &data.len().to_string(), 1);
    for (i, byte) in data.iter().enumerate() {
        if i % 8 == 0 {
            buffer.push_str(&format!("\n{i:02}:      "));
        }
        buffer.push_str(&format!(" 0x{byte:02x}"));
    }
    buffer.push('\n');
    crate::log::debug!(crate::log::Category::Input, "{buffer}");
}

/// Whether a controller might be a PlayStation controller worth probing.
/// Translation of `HIDAPI_SupportsPlaystationDetection()`.
pub(crate) fn supports_playstation_detection(vendor: u16, product: u16) -> bool {
    /* If we already know the controller is a different type, don't try to detect it.
     * This fixes a hang with the HORIPAD for Nintendo Switch (0x0f0d/0x00c1)
     */
    if gamepad_type_from_vidpid(vendor, product, None, false) != GamepadType::Standard {
        return false;
    }

    match vendor {
        USB_VENDOR_CRKD => true,
        USB_VENDOR_DRAGONRISE => true,
        USB_VENDOR_CORSAIR => true,
        USB_VENDOR_HORI => true,
        USB_VENDOR_LOGITECH => {
            /* Most Logitech devices are not PlayStation controllers, and some of them
             * lock up or reset when we send them the Sony third-party query feature
             * report, so don't include that vendor here. Instead add devices as
             * appropriate to controller_list.h
             */
            false
        }
        USB_VENDOR_MADCATZ => {
            if product == USB_PRODUCT_MADCATZ_SAITEK_SIDE_PANEL_CONTROL_DECK {
                // This is not a Playstation compatible device
                return false;
            }
            true
        }
        USB_VENDOR_MAYFLASH => true,
        USB_VENDOR_NACON | USB_VENDOR_NACON_ALT => true,
        USB_VENDOR_PDP => true,
        USB_VENDOR_POWERA => true,
        USB_VENDOR_POWERA_ALT => true,
        USB_VENDOR_QANBA => true,
        USB_VENDOR_RAZER => {
            /* Most Razer devices are not PlayStation controllers, and some of them
             * lock up or reset when we send them the Sony third-party query feature
             * report, so don't include that vendor here. Instead add devices as
             * appropriate to controller_list.h
             *
             * Reference: https://github.com/libsdl-org/SDL/issues/6733
             *            https://github.com/libsdl-org/SDL/issues/6799
             */
            false
        }
        USB_VENDOR_RED_OCTANE_GAMES => true,
        USB_VENDOR_SHANWAN => true,
        USB_VENDOR_SHANWAN_ALT => true,
        USB_VENDOR_THRUSTMASTER => {
            /* Most of these are wheels, don't have the full set of effects, and
             * at least in the case of the T248 and T300 RS, the hid-tmff2 driver
             * puts them in a non-standard report mode and they can't be read.
             *
             * If these should use the HIDAPI driver, add them to controller_list.h
             */
            false
        }
        USB_VENDOR_ZEROPLUS => true,
        0x7545 /* SZ-MYPOWER */ => true,
        _ => false,
    }
}

/// Translation of `HIDAPI_RemapVal()`.
#[allow(dead_code)] // (used by drivers not translated yet)
pub(crate) fn remap_val(
    val: f32,
    val_min: f32,
    val_max: f32,
    output_min: f32,
    output_max: f32,
) -> f32 {
    output_min + (output_max - output_min) * (val - val_min) / (val_max - val_min)
}

/// Translation of `SDL_GetJoystickGameControllerProtocol()`.
fn get_joystick_game_controller_protocol(
    name: Option<&str>,
    vendor: u16,
    product: u16,
    interface_number: i32,
    interface_class: i32,
    interface_subclass: i32,
    interface_protocol: i32,
) -> GamepadType {
    const LIBUSB_CLASS_VENDOR_SPEC: i32 = 0xFF;
    const XB360_IFACE_SUBCLASS: i32 = 93;
    const XB360_IFACE_PROTOCOL: i32 = 1; // Wired
    const XB360W_IFACE_PROTOCOL: i32 = 129; // Wireless
    const XBONE_IFACE_SUBCLASS: i32 = 71;
    const XBONE_IFACE_PROTOCOL: i32 = 208;

    let mut gamepad_type = GamepadType::Standard;

    // This code should match the checks in libusb/hid.c and HIDDeviceManager.java
    if interface_class == LIBUSB_CLASS_VENDOR_SPEC
        && interface_subclass == XB360_IFACE_SUBCLASS
        && (interface_protocol == XB360_IFACE_PROTOCOL
            || interface_protocol == XB360W_IFACE_PROTOCOL)
    {
        const SUPPORTED_VENDORS: [u16; 33] = [
            0x0079, // GPD Win 2
            0x0351, // CRKD
            0x044f, // Thrustmaster
            0x045e, // Microsoft
            0x046d, // Logitech
            0x056e, // Elecom
            0x06a3, // Saitek
            0x0738, // Mad Catz
            0x07ff, // Mad Catz
            0x0e6f, // PDP
            0x0f0d, // Hori
            0x1038, // SteelSeries
            0x10f5, // Turtle Beach
            0x11c9, // Nacon
            0x1209, // Generic
            0x12ab, // Unknown
            0x1430, // RedOctane
            0x146b, // BigBen
            0x1532, // Razer
            0x15e4, // Numark
            0x162e, // Joytech
            0x1689, // Razer Onza
            0x1949, // Lab126, Inc.
            0x1bad, // Harmonix
            0x20d6, // PowerA
            0x24c6, // PowerA
            0x2c22, // Qanba
            0x2dc8, // 8BitDo
            0x3537, // GameSir
            0x3651, // CRKD
            0x37d7, // Flydigi
            0x3958, // Red Octane Games
            0x9886, // ASTRO Gaming
        ];

        if SUPPORTED_VENDORS.contains(&vendor) {
            gamepad_type = GamepadType::Xbox360;
        }
    }

    if interface_number == 0
        && interface_class == LIBUSB_CLASS_VENDOR_SPEC
        && interface_subclass == XBONE_IFACE_SUBCLASS
        && interface_protocol == XBONE_IFACE_PROTOCOL
    {
        const SUPPORTED_VENDORS: [u16; 22] = [
            0x0351, // CRKD
            0x03f0, // HP
            0x044f, // Thrustmaster
            0x045e, // Microsoft
            0x0738, // Mad Catz
            0x0b05, // ASUS
            0x0e6f, // PDP
            0x0f0d, // Hori
            0x10f5, // Turtle Beach
            0x1209, // Generic
            0x1532, // Razer
            0x20d6, // PowerA
            0x24c6, // PowerA
            0x294b, // Snakebyte
            0x2dc8, // 8BitDo
            0x2e24, // Hyperkin
            0x2e95, // SCUF
            0x3285, // Nacon
            0x3537, // GameSir
            0x3651, // CRKD
            0x366c, // ByoWave
            0x3958, // Red Octane Games
        ];

        if SUPPORTED_VENDORS.contains(&vendor) {
            gamepad_type = GamepadType::XboxOne;
        }
    }

    if gamepad_type == GamepadType::Standard {
        gamepad_type = gamepad_type_from_vidpid(vendor, product, name, false);
    }
    gamepad_type
}

/// Translation of `HIDAPI_IsDeviceSupported()`.
fn is_device_supported(vendor_id: u16, product_id: u16, version: u16, name: &str) -> bool {
    let gamepad_type =
        get_joystick_game_controller_protocol(Some(name), vendor_id, product_id, -1, 0, 0, 0);

    HIDAPI_DRIVERS.iter().any(|driver| {
        driver.enabled()
            && driver.imp.is_supported_device(
                None,
                name,
                gamepad_type,
                vendor_id,
                product_id,
                version,
                -1,
                0,
                0,
                0,
            )
    })
}

/// Translation of `HIDAPI_GetDeviceDriver()`.
fn get_device_driver(device: &HidapiDevice) -> Option<&'static HidapiDeviceDriver> {
    const USAGE_PAGE_GENERIC_DESKTOP: u16 = 0x0001;
    const USAGE_JOYSTICK: u16 = 0x0004;
    const USAGE_GAMEPAD: u16 = 0x0005;
    const USAGE_MULTIAXISCONTROLLER: u16 = 0x0008;

    if !device.children().is_empty() {
        return Some(&DRIVER_COMBINED);
    }

    let name = device.name();
    if should_ignore_joystick(
        device.vendor_id,
        device.product_id,
        device.version,
        Some(&name),
    ) {
        return None;
    }

    if device.vendor_id != USB_VENDOR_VALVE
        && device.vendor_id != USB_VENDOR_FLYDIGI_V1
        && device.vendor_id != USB_VENDOR_FLYDIGI_V2
    {
        if device.usage_page != 0 && device.usage_page != USAGE_PAGE_GENERIC_DESKTOP {
            return None;
        }
        if device.usage != 0
            && device.usage != USAGE_JOYSTICK
            && device.usage != USAGE_GAMEPAD
            && device.usage != USAGE_MULTIAXISCONTROLLER
        {
            return None;
        }
    }

    let gamepad_type = device.gamepad_type();
    HIDAPI_DRIVERS.iter().copied().find(|driver| {
        driver.enabled()
            && driver.imp.is_supported_device(
                Some(device),
                &name,
                gamepad_type,
                device.vendor_id,
                device.product_id,
                device.version,
                device.interface_number,
                device.interface_class,
                device.interface_subclass,
                device.interface_protocol,
            )
    })
}

/// Translation of `HIDAPI_GetDeviceByIndex()`: the device of a joystick
/// index, with the joystick's instance id.
fn get_device_by_index(mut device_index: usize) -> Option<(Arc<HidapiDevice>, JoystickID)> {
    assert_joysticks_locked();

    for device in devices() {
        let s = device.state();
        if s.parent.upgrade().is_some() || s.broken {
            continue;
        }
        if s.driver.is_some() {
            if device_index < s.joysticks.len() {
                let joystick = s.joysticks[device_index];
                drop(s);
                return Some((device, joystick));
            }
            device_index -= s.joysticks.len();
        }
    }
    None
}

/// Translation of `HIDAPI_GetJoystickByInfo()`.
fn get_joystick_by_info(path: &str, vendor_id: u16, product_id: u16) -> Option<Arc<HidapiDevice>> {
    assert_joysticks_locked();

    devices().into_iter().find(|device| {
        device.vendor_id == vendor_id && device.product_id == product_id && device.path == path
    })
}

/// Translation of `HIDAPI_CleanupDeviceDriver()`.
fn cleanup_device_driver(device: &Arc<HidapiDevice>) {
    if device.driver().is_none() {
        return; // Already cleaned up
    }

    // Disconnect any joysticks
    while let Some(&joystick) = device.joysticks().first() {
        let mut pending = Vec::new();
        joystick_disconnected(device, joystick, &mut pending);
        deliver(pending);
    }

    with_context(device, |context, dctx| context.free_device(dctx));
    device.state().driver = None;

    device.set_dev(None);

    *device.context.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// Translation of `HIDAPI_SetupDeviceDriver()` (its `removed` result is
/// never set upstream).
fn setup_device_driver(device: &Arc<HidapiDevice>) {
    if let Some(driver) = device.driver() {
        let mut enabled = if device.vendor_id == USB_VENDOR_NINTENDO
            && (device.product_id == USB_PRODUCT_NINTENDO_SWITCH_JOYCON_PAIR
                || device.product_id == USB_PRODUCT_NINTENDO_SWITCH2_JOYCON_PAIR)
        {
            COMBINE_JOYCONS.load(Ordering::Relaxed)
        } else {
            driver.enabled()
        };
        for child in device.children() {
            if child.driver().is_none_or(|d| !d.enabled()) {
                enabled = false;
                break;
            }
        }
        if !enabled {
            cleanup_device_driver(device);
        }
        return; // Already setup
    }

    if get_device_driver(device).is_some() {
        // We might have a device driver for this device, try opening it and see
        if device.children().is_empty() {
            // Wait a little bit for the device to initialize
            crate::timer::delay(Duration::from_millis(10));

            let dev = match HidDevice::open_path(&device.path) {
                Ok(dev) => dev,
                Err(e) => {
                    crate::log::debug!(
                        crate::log::Category::Input,
                        "HIDAPI_SetupDeviceDriver() couldn't open {}: {}",
                        device.path,
                        e.message()
                    );
                    return;
                }
            };
            let _ = dev.set_nonblocking(true);

            device.set_dev(Some(Arc::new(dev)));
        }

        let driver = get_device_driver(device);
        device.state().driver = driver;

        // Initialize the device, which may cause a connected event
        if let Some(driver) = driver {
            *device.context.lock().unwrap_or_else(|e| e.into_inner()) =
                Some(driver.imp.new_context());
            let result = with_context(device, |context, dctx| context.init_device(dctx));
            if !matches!(result, Some(Ok(()))) {
                cleanup_device_driver(device);
            }
        }

        if device.driver().is_none() && device.dev().is_some() {
            // No driver claimed this device, go ahead and close it
            device.set_dev(None);
        }
    }
}

/// Translation of `SDL_HIDAPI_UpdateDrivers()`.
fn update_drivers() {
    assert_joysticks_locked();

    let mut numdrivers = 0;
    for driver in HIDAPI_DRIVERS {
        let enabled = driver.imp.is_enabled();
        driver.enabled.store(enabled, Ordering::Relaxed);
        if enabled && !std::ptr::eq(*driver, &DRIVER_COMBINED) {
            numdrivers += 1;
        }
    }
    NUMDRIVERS.store(numdrivers, Ordering::Relaxed);

    for device in devices() {
        setup_device_driver(&device);
    }
}

/// Translation of `SDL_HIDAPIDriverHintChanged()`.
fn driver_hint_changed(name: &str, hint: Option<&str>) {
    if name == hints::JOYSTICK_HIDAPI_COMBINE_JOY_CONS {
        COMBINE_JOYCONS.store(hints::string_to_bool(hint, true), Ordering::Relaxed);
    }
    HINTS_CHANGED.store(true, Ordering::Relaxed);
    CHANGE_COUNT.store(0, Ordering::Relaxed);
}

/// Translation of `HIDAPI_JoystickInit()`.
fn joystick_init() -> Result<()> {
    if INITIALIZED.load(Ordering::Relaxed) {
        return Ok(());
    }

    if crate::hidapi::init().is_err() {
        return Err(Error::new("Couldn't initialize hidapi"));
    }

    let mut names: Vec<&'static str> = Vec::new();
    for driver in HIDAPI_DRIVERS {
        names.extend_from_slice(driver.imp.hints());
    }
    names.push(hints::JOYSTICK_HIDAPI_COMBINE_JOY_CONS);
    names.push(hints::JOYSTICK_HIDAPI);
    let mut watches = Vec::new();
    for name in names {
        if let Ok(watch) = hints::watch(name, move |change| {
            driver_hint_changed(name, change.new_value)
        }) {
            watches.push(watch);
        }
    }
    *HINT_WATCHES.lock().unwrap_or_else(|e| e.into_inner()) = watches;

    CHANGE_COUNT.store(crate::hidapi::device_change_count(), Ordering::Relaxed);
    update_device_list();
    update_devices();

    INITIALIZED.store(true, Ordering::Relaxed);

    Ok(())
}

/// Translation of `HIDAPI_AddJoystickInstanceToDevice()`.
fn add_joystick_instance_to_device(device: &HidapiDevice, joystick: JoystickID) {
    device.state().joysticks.push(joystick);
}

/// Translation of `HIDAPI_DelJoystickInstanceFromDevice()`.
fn del_joystick_instance_from_device(device: &HidapiDevice, joystick: JoystickID) -> bool {
    let mut s = device.state();
    match s.joysticks.iter().position(|&id| id == joystick) {
        Some(i) => {
            s.joysticks.remove(i);
            true
        }
        None => false,
    }
}

/// Translation of `HIDAPI_JoystickInstanceIsUnique()`.
fn joystick_instance_is_unique(device: &HidapiDevice) -> bool {
    if let Some(parent) = device.parent() {
        let joysticks = device.joysticks();
        let parent_joysticks = parent.joysticks();
        if joysticks.len() == 1
            && parent_joysticks.len() == 1
            && joysticks[0] == parent_joysticks[0]
        {
            return false;
        }
    }
    true
}

/// Translation of `HIDAPI_UpdateJoystickSerial()`.
fn update_joystick_serial(device: &HidapiDevice) {
    assert_joysticks_locked();

    if let Some(serial) = device.serial() {
        for joystick in device.joysticks() {
            with_joystick(joystick, |j| j.serial = Some(serial.clone()));
        }
    }
}

/// Translation of `HIDAPI_SerialIsEmpty()`.
fn serial_is_empty(device: &HidapiDevice) -> bool {
    device
        .serial()
        .is_none_or(|serial| serial.bytes().all(|c| c == b'0'))
}

/// Translation of `HIDAPI_SetDeviceSerial()`.
fn set_device_serial(device: &HidapiDevice, serial: &str) {
    {
        let mut s = device.state();
        if serial.is_empty() || s.serial.as_deref() == Some(serial) {
            return;
        }
        s.serial = Some(serial.to_owned());
    }
    update_joystick_serial(device);
}

/// Translation of `HIDAPI_SetDeviceSerialW()` (with its `wcstrcmp()`).
fn set_device_serial_w(device: &HidapiDevice, serial: Option<&str>) {
    if let Some(serial) = serial {
        if !serial.is_empty() && device.serial().as_deref() != Some(serial) {
            device.state().serial = convert_string(Some(serial));
            update_joystick_serial(device);
        }
    }
}

/// Translation of `HIDAPI_HasConnectedUSBDevice()`.
fn has_connected_usb_device(serial: Option<&str>) -> bool {
    assert_joysticks_locked();

    let Some(serial) = serial else {
        return false;
    };

    devices().iter().any(|device| {
        let s = device.state();
        s.driver.is_some()
            && !s.broken
            && !device.is_bluetooth
            && s.serial.as_deref() == Some(serial)
    })
}

/// Translation of `HIDAPI_JoystickConnected()`.
fn joystick_connected(device: &Arc<HidapiDevice>, pending: &mut Vec<Pending>) -> JoystickID {
    assert_joysticks_locked();

    for child in device.children() {
        for joystick in child.joysticks().into_iter().rev() {
            joystick_disconnected(&child, joystick, pending);
        }
    }

    let joystick = crate::utils::next_object_id();
    add_joystick_instance_to_device(device, joystick);

    for child in device.children() {
        add_joystick_instance_to_device(&child, joystick);
    }

    NUMJOYSTICKS.fetch_add(1, Ordering::Relaxed);

    pending.push(Pending::Added(joystick));

    joystick
}

/// Translation of `HIDAPI_JoystickDisconnected()`: the joystick leaves the
/// device now; it is closed and reported removed by [`deliver`].
fn joystick_disconnected(
    device: &Arc<HidapiDevice>,
    joystick: JoystickID,
    pending: &mut Vec<Pending>,
) {
    let _lock = lock_joysticks();

    let device = if !joystick_instance_is_unique(device) {
        // Disconnecting a child always disconnects the parent
        device.parent().unwrap_or_else(|| device.clone())
    } else {
        device.clone()
    };

    if device.joysticks().contains(&joystick) {
        del_joystick_instance_from_device(&device, joystick);

        for child in device.children() {
            del_joystick_instance_from_device(&child, joystick);
        }

        NUMJOYSTICKS.fetch_sub(1, Ordering::Relaxed);

        pending.push(Pending::Removed(device.clone(), joystick));
    }

    // Rescan the device list in case device state has changed
    CHANGE_COUNT.store(0, Ordering::Relaxed);
}

/// Translation of `HIDAPI_UpdateJoystickProperties()`.
fn update_joystick_properties(
    device: &Arc<HidapiDevice>,
    joystick: JoystickID,
    props: &crate::properties::Properties,
) {
    let Some(caps) = with_context(device, |context, dctx| {
        context.get_joystick_capabilities(dctx, joystick)
    }) else {
        return;
    };

    let _ = props.set(
        PROP_JOYSTICK_CAP_MONO_LED_BOOLEAN,
        caps.contains(JoystickCaps::MONO_LED),
    );
    let _ = props.set(
        PROP_JOYSTICK_CAP_RGB_LED_BOOLEAN,
        caps.contains(JoystickCaps::RGB_LED),
    );
    let _ = props.set(
        PROP_JOYSTICK_CAP_PLAYER_LED_BOOLEAN,
        caps.contains(JoystickCaps::PLAYER_LED),
    );
    let _ = props.set(
        PROP_JOYSTICK_CAP_RUMBLE_BOOLEAN,
        caps.contains(JoystickCaps::RUMBLE),
    );
    let _ = props.set(
        PROP_JOYSTICK_CAP_TRIGGER_RUMBLE_BOOLEAN,
        caps.contains(JoystickCaps::TRIGGER_RUMBLE),
    );
}

/// Translation of `HIDAPI_AddDevice()`.
fn add_device(info: &DeviceInfo, children: Vec<Arc<HidapiDevice>>) -> Option<Arc<HidapiDevice>> {
    assert_joysticks_locked();

    let path = info.path.clone()?;

    // Need the device name before getting the driver to know whether to ignore this device
    let serial_number = convert_string(info.serial_number.as_deref());
    let manufacturer_string = convert_string(info.manufacturer_string.as_deref());
    let product_string = convert_string(info.product_string.as_deref());
    let name = create_joystick_name(
        info.vendor_id,
        info.product_id,
        manufacturer_string.as_deref(),
        product_string.as_deref(),
    )?;
    let serial = serial_number.filter(|s| !s.is_empty());

    let bus = if info.bus_type == BusType::Bluetooth {
        HARDWARE_BUS_BLUETOOTH
    } else {
        HARDWARE_BUS_USB
    };
    let guid = create_joystick_guid(
        bus,
        info.vendor_id,
        info.product_id,
        info.release_number,
        manufacturer_string.as_deref(),
        product_string.as_deref(),
        b'h',
        0,
    );
    let gamepad_type = get_joystick_game_controller_protocol(
        Some(&name),
        info.vendor_id,
        info.product_id,
        info.interface_number,
        info.interface_class,
        info.interface_subclass,
        info.interface_protocol,
    );

    let device = Arc::new(HidapiDevice {
        manufacturer_string,
        product_string,
        path,
        vendor_id: info.vendor_id,
        product_id: info.product_id,
        version: info.release_number,
        interface_number: info.interface_number,
        interface_class: info.interface_class,
        interface_subclass: info.interface_subclass,
        interface_protocol: info.interface_protocol,
        usage_page: info.usage_page,
        usage: info.usage,
        is_bluetooth: info.bus_type == BusType::Bluetooth,
        state: Mutex::new(DeviceState {
            name,
            serial,
            guid,
            joystick_type: JoystickType::Gamepad,
            gamepad_type,
            steam_virtual_gamepad_slot: -1,
            driver: None,
            joysticks: Vec::new(),
            seen: true,
            broken: false,
            parent: Weak::new(),
            children: Vec::new(),
        }),
        context: Mutex::new(None),
        dev: Mutex::new(None),
        rumble_pending: AtomicI32::new(0),
        valid: AtomicBool::new(true),
    });

    if !children.is_empty() {
        for child in &children {
            child.state().parent = Arc::downgrade(&device);
        }
        device.state().children = children;
    }

    // Add it to the list
    DEVICES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(device.clone());

    setup_device_driver(&device);

    log_device("Added", &device);

    Some(device)
}

/// The debug log line of `HIDAPI_AddDevice()` and `HIDAPI_DelDevice()`.
fn log_device(what: &str, device: &HidapiDevice) {
    let driver = device.driver();
    crate::log::debug!(
        crate::log::Category::Input,
        "{} HIDAPI device '{}' VID 0x{:04x}, PID 0x{:04x}, bluetooth {}, version {}, serial {}, interface {}, interface_class {}, interface_subclass {}, interface_protocol {}, usage page 0x{:04x}, usage 0x{:04x}, path = {}, driver = {} ({})",
        if what == "Added" { "Added" } else { "Removing" },
        device.name(),
        device.vendor_id,
        device.product_id,
        i32::from(device.is_bluetooth),
        device.version,
        device.serial().as_deref().unwrap_or("NONE"),
        device.interface_number,
        device.interface_class,
        device.interface_subclass,
        device.interface_protocol,
        device.usage_page,
        device.usage,
        device.path,
        driver.map_or("NONE", |d| d.name),
        if driver.is_some_and(|d| d.enabled()) { "ENABLED" } else { "DISABLED" }
    );
}

/// Translation of `HIDAPI_DelDevice()`.
fn del_device(device: &Arc<HidapiDevice>) {
    assert_joysticks_locked();

    log_device("Removing", device);

    let removed = {
        let mut list = DEVICES.lock().unwrap_or_else(|e| e.into_inner());
        match list.iter().position(|d| Arc::ptr_eq(d, device)) {
            Some(i) => {
                list.remove(i);
                true
            }
            None => false,
        }
    };
    if !removed {
        return;
    }

    cleanup_device_driver(device);

    // Make sure the rumble thread is done with this device
    while device.rumble_pending.load(Ordering::Acquire) > 0 {
        crate::timer::delay(Duration::from_millis(10));
    }

    for child in device.children() {
        child.state().parent = Weak::new();
    }

    device.valid.store(false, Ordering::Relaxed);
}

/// Translation of `HIDAPI_CreateCombinedJoyCons()`.
fn create_combined_joycons() -> bool {
    assert_joysticks_locked();

    if !COMBINE_JOYCONS.load(Ordering::Relaxed) {
        return false;
    }

    let mut joycons: [Option<Arc<HidapiDevice>>; 2] = [None, None];
    for device in devices() {
        if device.driver().is_none() {
            // Unsupported device
            continue;
        }
        if device.parent().is_some() {
            // This device is already part of a combined device
            continue;
        }
        if device.broken() {
            // This device can't be used
            continue;
        }

        let (vendor, product, _, _) = joystick_guid_info(device.guid());
        let name = device.name();

        if joycons[0].is_none()
            && (super::is_joystick_nintendo_switch_joycon_left(vendor, product)
                || (super::is_joystick_nintendo_switch_joycon_grip(vendor, product)
                    && name.contains("(L)")))
        {
            joycons[0] = Some(device.clone());
        }
        if joycons[1].is_none()
            && (super::is_joystick_nintendo_switch_joycon_right(vendor, product)
                || (super::is_joystick_nintendo_switch_joycon_grip(vendor, product)
                    && name.contains("(R)")))
        {
            joycons[1] = Some(device.clone());
        }
        if let [Some(left), Some(right)] = &joycons {
            let children = vec![left.clone(), right.clone()];

            let info = DeviceInfo {
                path: Some("nintendo_joycons_combined".to_owned()),
                vendor_id: USB_VENDOR_NINTENDO,
                product_id: if left.product_id == USB_PRODUCT_NINTENDO_SWITCH_JOYCON_LEFT {
                    USB_PRODUCT_NINTENDO_SWITCH_JOYCON_PAIR
                } else {
                    USB_PRODUCT_NINTENDO_SWITCH2_JOYCON_PAIR
                },
                interface_number: -1,
                usage_page: USB_USAGEPAGE_GENERIC_DESKTOP,
                usage: USB_USAGE_GENERIC_GAMEPAD,
                manufacturer_string: Some("Nintendo".to_owned()),
                product_string: Some("Switch Joy-Con (L/R)".to_owned()),
                bus_type: if left.is_bluetooth || right.is_bluetooth {
                    BusType::Bluetooth
                } else {
                    BusType::Usb
                },
                ..DeviceInfo::default()
            };

            return match add_device(&info, children) {
                Some(combined) if combined.driver().is_some() => true,
                Some(combined) => {
                    del_device(&combined);
                    false
                }
                None => false,
            };
        }
    }
    false
}

/// Translation of `HIDAPI_UpdateDeviceList()`.
fn update_device_list() {
    let _lock = lock_joysticks();

    if HINTS_CHANGED.load(Ordering::Relaxed) {
        update_drivers();
        HINTS_CHANGED.store(false, Ordering::Relaxed);
    }

    // Prepare the existing device list
    for device in devices() {
        let mut s = device.state();
        if !s.children.is_empty() {
            continue;
        }
        s.seen = false;
    }

    // Enumerate the devices
    if NUMDRIVERS.load(Ordering::Relaxed) > 0 {
        if let Ok(devs) = crate::hidapi::enumerate(0, 0) {
            for info in &devs {
                let Some(path) = info.path.as_deref() else {
                    // We can't open this, ignore it
                    continue;
                };

                match get_joystick_by_info(path, info.vendor_id, info.product_id) {
                    Some(device) => {
                        device.state().seen = true;

                        // Check to see if the serial number is available now
                        if serial_is_empty(&device) {
                            set_device_serial_w(&device, info.serial_number.as_deref());
                        }
                    }
                    None => {
                        add_device(info, Vec::new());
                    }
                }
            }
        }
    }

    // Remove any devices that weren't seen or have been disconnected due to read errors
    'check_removed: loop {
        for device in devices() {
            if !DEVICES
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .iter()
                .any(|d| Arc::ptr_eq(d, &device))
            {
                // (deleted earlier in this pass)
                continue;
            }

            let (seen, gone) = {
                let s = device.state();
                (
                    s.seen,
                    (s.driver.is_some() || !s.children.is_empty()) && s.joysticks.is_empty(),
                )
            };
            if !seen || (gone && device.dev().is_none()) {
                if let Some(parent) = device.parent() {
                    // When a child device goes away, so does the parent
                    for child in parent.children() {
                        del_device(&child);
                    }
                    del_device(&parent);

                    // Update the device list again to pick up any children left
                    CHANGE_COUNT.store(0, Ordering::Relaxed);

                    // We deleted more than one device here, restart the loop
                    continue 'check_removed;
                } else {
                    del_device(&device);

                    // Update the device list again in case this device comes back
                    CHANGE_COUNT.store(0, Ordering::Relaxed);
                    continue;
                }
            }
            if device.broken() {
                if let Some(parent) = device.parent() {
                    del_device(&parent);

                    // We deleted a different device here, restart the loop
                    continue 'check_removed;
                }
            }
        }
        break;
    }

    // See if we can create any combined Joy-Con controllers
    while create_combined_joycons() {}
}

/// Translation of `HIDAPI_IsEquivalentToDevice()`.
fn is_equivalent_to_device(vendor_id: u16, product_id: u16, device: &HidapiDevice) -> bool {
    if vendor_id == device.vendor_id && product_id == device.product_id {
        return true;
    }

    if vendor_id == USB_VENDOR_MICROSOFT {
        // If we're looking for the wireless XBox 360 controller, also look for the dongle
        if product_id == USB_PRODUCT_XBOX360_XUSB_CONTROLLER
            && device.product_id == USB_PRODUCT_XBOX360_WIRELESS_RECEIVER
        {
            return true;
        }

        // If we're looking for the raw input Xbox One controller, match it against any other Xbox One controller
        if product_id == USB_PRODUCT_XBOX_ONE_XBOXGIP_CONTROLLER
            && device.gamepad_type() == GamepadType::XboxOne
        {
            return true;
        }

        // If we're looking for an XInput controller, match it against any other Xbox controller
        if product_id == USB_PRODUCT_XBOX360_XUSB_CONTROLLER
            && matches!(
                device.gamepad_type(),
                GamepadType::Xbox360 | GamepadType::XboxOne
            )
        {
            return true;
        }
    }

    if vendor_id == USB_VENDOR_NVIDIA {
        // If we're looking for the NVIDIA SHIELD controller Xbox interface, match it against any NVIDIA SHIELD controller
        if product_id == 0xb400
            && super::is_joystick_nvidia_shield_controller(vendor_id, product_id)
        {
            return true;
        }
    }
    false
}

/// Translation of `HIDAPI_StartUpdatingDevices()`.
fn start_updating_devices() -> bool {
    UPDATING_DEVICES
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
        .is_ok()
}

/// Translation of `HIDAPI_FinishUpdatingDevices()`.
fn finish_updating_devices() {
    UPDATING_DEVICES.store(false, Ordering::Release);
}

/// Whether a device of a gamepad type is handled by this driver.
/// Translation of `HIDAPI_IsDeviceTypePresent()`.
#[allow(dead_code)] // (used by joystick drivers not translated yet)
pub(crate) fn is_device_type_present(gamepad_type: GamepadType) -> bool {
    // Make sure we're initialized, as this could be called from other drivers during startup
    if joystick_init().is_err() {
        return false;
    }

    if start_updating_devices() {
        update_device_list();
        finish_updating_devices();
    }

    let _lock = lock_joysticks();
    devices()
        .iter()
        .any(|device| device.driver().is_some() && device.gamepad_type() == gamepad_type)
}

/// Translation of `HIDAPI_IsDevicePresent()`.
fn is_device_present(vendor_id: u16, product_id: u16, version: u16, name: Option<&str>) -> bool {
    // Make sure we're initialized, as this could be called from other drivers during startup
    if joystick_init().is_err() {
        return false;
    }

    /* Only update the device list for devices we know might be supported.
      If we did this for every device, it would hit the USB driver too hard and potentially
      lock up the system. This won't catch devices that we support but can only detect using
      USB interface details, like Xbox controllers, but hopefully the device list update is
      responsive enough to catch those.
    */
    let name_str = name.unwrap_or("");
    let mut supported = is_device_supported(vendor_id, product_id, version, name_str);
    if !supported
        && (name_str.contains("Xbox") || name_str.contains("X-Box") || name_str.contains("XBOX"))
    {
        supported = true;
    }
    if supported && start_updating_devices() {
        update_device_list();
        finish_updating_devices();
    }

    /* Note that this isn't a perfect check - there may be multiple devices with 0 VID/PID,
      or a different name than we have it listed here, etc, but if we support the device
      and we have something similar in our device list, mark it as present.
    */
    let _lock = lock_joysticks();
    devices().iter().any(|device| {
        // The HIDAPI functionality will be available when the FlyDigi Space Station app has
        // enabled third party controller mapping, so the driver needs to be active to watch
        // for that change. Since this is dynamic and we don't have a way to re-trigger device
        // changes when that happens, we'll pretend the driver isn't available so the XInput
        // interface will always show up (but won't have any input when the controller is in
        // enhanced mode)
        if device.vendor_id == USB_VENDOR_FLYDIGI_V2
            && device
                .driver()
                .is_some_and(|d| std::ptr::eq(d, &DRIVER_FLYDIGI))
        {
            return false;
        }

        device.driver().is_some() && is_equivalent_to_device(vendor_id, product_id, device)
    })
}

/// The product name of a connected device. Translation of
/// `HIDAPI_GetDeviceProductName()`.
#[cfg_attr(not(windows), allow(dead_code))] // (used by the DirectInput driver)
pub(crate) fn get_device_product_name(vendor_id: u16, product_id: u16) -> Option<String> {
    let _lock = lock_joysticks();
    devices()
        .into_iter()
        .find(|device| vendor_id == device.vendor_id && product_id == device.product_id)
        .and_then(|device| device.product_string.clone())
}

/// The manufacturer of a connected device. Translation of
/// `HIDAPI_GetDeviceManufacturerName()`.
#[cfg_attr(not(windows), allow(dead_code))] // (used by the DirectInput driver)
pub(crate) fn get_device_manufacturer_name(vendor_id: u16, product_id: u16) -> Option<String> {
    let _lock = lock_joysticks();
    devices()
        .into_iter()
        .find(|device| vendor_id == device.vendor_id && product_id == device.product_id)
        .and_then(|device| device.manufacturer_string.clone())
}

/// The joystick type of a HIDAPI joystick GUID. Translation of
/// `HIDAPI_GetJoystickTypeFromGUID()`.
pub(crate) fn get_joystick_type_from_guid(guid: Guid) -> JoystickType {
    let _lock = lock_joysticks();
    devices()
        .into_iter()
        .find(|device| device.guid() == guid)
        .map_or(JoystickType::Unknown, |device| device.state().joystick_type)
}

/// The gamepad type of a HIDAPI joystick GUID. Translation of
/// `HIDAPI_GetGamepadTypeFromGUID()`.
pub(crate) fn get_gamepad_type_from_guid(guid: Guid) -> GamepadType {
    let _lock = lock_joysticks();
    devices()
        .into_iter()
        .find(|device| device.guid() == guid)
        .map_or(GamepadType::Standard, |device| device.gamepad_type())
}

/// Translation of `HIDAPI_JoystickDetect()`.
fn joystick_detect() {
    if start_updating_devices() {
        let count = crate::hidapi::device_change_count();
        if CHANGE_COUNT.load(Ordering::Relaxed) != count {
            CHANGE_COUNT.store(count, Ordering::Relaxed);
            update_device_list();
        }
        finish_updating_devices();
    }
}

/// Update the devices, which may change connected joysticks and send
/// events. Called by the joystick update, as a single device can provide
/// several joysticks. Translation of `HIDAPI_UpdateDevices()`.
pub(crate) fn update_devices() {
    assert_joysticks_locked();

    // Prepare the existing device list
    if start_updating_devices() {
        for device in devices() {
            if device.parent().is_some() {
                continue;
            }
            if device.driver().is_some() {
                with_context(&device, |context, dctx| context.update_device(dctx));
            }
        }
        finish_updating_devices();
    }
}

/// Translation of `HIDAPI_GetJoystickDevice()`: the device of an open
/// joystick, if it is still valid and has a driver.
fn get_joystick_device(joystick: JoystickID) -> Option<Arc<HidapiDevice>> {
    assert_joysticks_locked();

    let device = with_joystick(joystick, |j| {
        j.hwdata
            .as_ref()
            .and_then(|h| h.downcast_ref::<HwData>())
            .map(|h| h.device.clone())
    })??;
    (device.valid.load(Ordering::Relaxed) && device.driver().is_some()).then_some(device)
}

/// The driver side of `HIDAPI_JoystickClose()`.
fn joystick_close_device(device: &Arc<HidapiDevice>, joystick: JoystickID) {
    // Wait up to 30 ms for pending rumble to complete
    for _ in 0..3 {
        if device.rumble_pending.load(Ordering::Acquire) > 0 {
            crate::timer::delay(Duration::from_millis(10));
        }
    }

    with_context(device, |context, dctx| {
        context.close_joystick(dctx, joystick)
    });
}

/// The HIDAPI joystick driver. Translation of `SDL_HIDAPI_JoystickDriver`.
pub(crate) struct HidapiJoystickDriver;

/// Translation of `SDL_HIDAPI_JoystickDriver`.
pub(crate) static HIDAPI_JOYSTICK_DRIVER: HidapiJoystickDriver = HidapiJoystickDriver;

impl JoystickDriver for HidapiJoystickDriver {
    fn init(&self) -> Result<()> {
        joystick_init()
    }

    /// Translation of `HIDAPI_JoystickGetCount()`.
    fn count(&self) -> usize {
        NUMJOYSTICKS.load(Ordering::Relaxed)
    }

    fn detect(&self) {
        joystick_detect();
    }

    fn is_device_present(
        &self,
        vendor_id: u16,
        product_id: u16,
        version: u16,
        name: Option<&str>,
    ) -> bool {
        is_device_present(vendor_id, product_id, version, name)
    }

    /// Translation of `HIDAPI_JoystickGetDeviceName()`.
    fn device_name(&self, device_index: usize) -> Option<String> {
        get_device_by_index(device_index).map(|(device, _)| device.name())
    }

    /// Translation of `HIDAPI_JoystickGetDevicePath()`.
    fn device_path(&self, device_index: usize) -> Option<String> {
        get_device_by_index(device_index).map(|(device, _)| device.path.clone())
    }

    /// Translation of `HIDAPI_JoystickGetDeviceSteamVirtualGamepadSlot()`.
    fn device_steam_virtual_gamepad_slot(&self, device_index: usize) -> i32 {
        get_device_by_index(device_index)
            .map_or(-1, |(device, _)| device.state().steam_virtual_gamepad_slot)
    }

    /// Translation of `HIDAPI_JoystickGetDevicePlayerIndex()`.
    fn device_player_index(&self, device_index: usize) -> i32 {
        match get_device_by_index(device_index) {
            Some((device, instance_id)) => with_context(&device, |context, dctx| {
                context.get_device_player_index(dctx, instance_id)
            })
            .unwrap_or(-1),
            None => -1,
        }
    }

    /// Translation of `HIDAPI_JoystickSetDevicePlayerIndex()`.
    fn set_device_player_index(&self, device_index: usize, player_index: i32) {
        if let Some((device, instance_id)) = get_device_by_index(device_index) {
            with_context(&device, |context, dctx| {
                context.set_device_player_index(dctx, instance_id, player_index)
            });
        }
    }

    /// Translation of `HIDAPI_JoystickGetDeviceGUID()`.
    fn device_guid(&self, device_index: usize) -> Guid {
        get_device_by_index(device_index).map_or(Guid::ZERO, |(device, _)| device.guid())
    }

    /// Translation of `HIDAPI_JoystickGetDeviceInstanceID()`.
    fn device_instance_id(&self, device_index: usize) -> JoystickID {
        get_device_by_index(device_index).map_or(0, |(_, joystick)| joystick)
    }

    /// Translation of `HIDAPI_JoystickOpen()`.
    fn open(&self, joystick: &mut JoystickData, device_index: usize) -> Result<()> {
        assert_joysticks_locked();

        let Some((device, joystick_id)) = get_device_by_index(device_index)
            .filter(|(device, _)| device.driver().is_some() && !device.broken())
        else {
            // This should never happen - validated before being called
            return Err(Error::new(format!(
                "Couldn't find HIDAPI device at index {device_index}"
            )));
        };

        // Process any pending reports before opening the device
        with_context(&device, |context, dctx| context.update_device(dctx));

        // UpdateDevice() may have called HIDAPI_JoystickDisconnected() if the device went away
        if device.num_joysticks() == 0 {
            return Err(Error::new("HIDAPI device disconnected while opening"));
        }

        // Set the default connection state, can be overridden below
        joystick.connection_state = if device.is_bluetooth {
            JoystickConnectionState::Wireless
        } else {
            JoystickConnectionState::Wired
        };

        let result = with_context(&device, |context, dctx| {
            context.open_joystick(dctx, joystick)
        })
        .unwrap_or_else(|| Err(Error::new("HIDAPI device is busy")));
        if let Err(e) = result {
            // The open failed, mark this device as disconnected and update devices
            let mut pending = Vec::new();
            joystick_disconnected(&device, joystick_id, &mut pending);
            deliver(pending);
            return Err(e);
        }

        let props = joystick.properties();
        update_joystick_properties(&device, joystick.instance_id, &props);

        if let Some(serial) = device.serial() {
            joystick.serial = Some(serial);
        }

        joystick.hwdata = Some(Box::new(HwData { device }));
        Ok(())
    }

    /// Translation of `HIDAPI_JoystickRumble()`.
    fn rumble(
        &self,
        joystick: JoystickID,
        low_frequency_rumble: u16,
        high_frequency_rumble: u16,
    ) -> Result<()> {
        match get_joystick_device(joystick) {
            Some(device) => with_context(&device, |context, dctx| {
                context.rumble_joystick(dctx, joystick, low_frequency_rumble, high_frequency_rumble)
            })
            .unwrap_or_else(|| Err(Error::new("Rumble failed, device disconnected"))),
            None => Err(Error::new("Rumble failed, device disconnected")),
        }
    }

    /// Translation of `HIDAPI_JoystickRumbleTriggers()`.
    fn rumble_triggers(
        &self,
        joystick: JoystickID,
        left_rumble: u16,
        right_rumble: u16,
    ) -> Result<()> {
        match get_joystick_device(joystick) {
            Some(device) => with_context(&device, |context, dctx| {
                context.rumble_joystick_triggers(dctx, joystick, left_rumble, right_rumble)
            })
            .unwrap_or_else(|| Err(Error::new("Rumble failed, device disconnected"))),
            None => Err(Error::new("Rumble failed, device disconnected")),
        }
    }

    /// Translation of `HIDAPI_JoystickSetLED()`.
    fn set_led(&self, joystick: JoystickID, red: u8, green: u8, blue: u8) -> Result<()> {
        match get_joystick_device(joystick) {
            Some(device) => with_context(&device, |context, dctx| {
                context.set_joystick_led(dctx, joystick, red, green, blue)
            })
            .unwrap_or_else(|| Err(Error::new("SetLED failed, device disconnected"))),
            None => Err(Error::new("SetLED failed, device disconnected")),
        }
    }

    /// Translation of `HIDAPI_JoystickSendEffect()`.
    fn send_effect(&self, joystick: JoystickID, data: &[u8]) -> Result<()> {
        match get_joystick_device(joystick) {
            Some(device) => with_context(&device, |context, dctx| {
                context.send_joystick_effect(dctx, joystick, data)
            })
            .unwrap_or_else(|| Err(Error::new("SendEffect failed, device disconnected"))),
            None => Err(Error::new("SendEffect failed, device disconnected")),
        }
    }

    /// Translation of `HIDAPI_JoystickSetSensorsEnabled()`.
    fn set_sensors_enabled(&self, joystick: JoystickID, enabled: bool) -> Result<()> {
        match get_joystick_device(joystick) {
            Some(device) => with_context(&device, |context, dctx| {
                context.set_joystick_sensors_enabled(dctx, joystick, enabled)
            })
            .unwrap_or_else(|| Err(Error::new("SetSensorsEnabled failed, device disconnected"))),
            None => Err(Error::new("SetSensorsEnabled failed, device disconnected")),
        }
    }

    /// Translation of `HIDAPI_JoystickUpdate()`: this is handled in
    /// [`update_devices`].
    fn update(&self, _joystick: JoystickID) {}

    /// Translation of `HIDAPI_JoystickClose()`.
    fn close(&self, joystick: &mut JoystickData) {
        assert_joysticks_locked();

        if let Some(hwdata) = joystick.hwdata.take() {
            if let Ok(hwdata) = hwdata.downcast::<HwData>() {
                joystick_close_device(&hwdata.device, joystick.instance_id);
            }
        }
    }

    /// Translation of `HIDAPI_JoystickQuit()`.
    fn quit(&self) {
        assert_joysticks_locked();

        SHUTTING_DOWN.store(true, Ordering::Relaxed);

        rumble::quit_rumble();

        while let Some(device) = devices().first().cloned() {
            if let Some(parent) = device.parent() {
                // When a child device goes away, so does the parent
                for child in parent.children() {
                    del_device(&child);
                }
                del_device(&parent);
            } else {
                del_device(&device);
            }
        }

        // Make sure the drivers cleaned up properly
        crate::sdl_assert!(NUMJOYSTICKS.load(Ordering::Relaxed) == 0);

        HINT_WATCHES
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();

        let _ = crate::hidapi::exit();

        CHANGE_COUNT.store(0, Ordering::Relaxed);
        SHUTTING_DOWN.store(false, Ordering::Relaxed);
        INITIALIZED.store(false, Ordering::Relaxed);
    }

    /// Translation of `HIDAPI_JoystickGetGamepadMapping()`.
    fn gamepad_mapping(&self, _device_index: usize) -> Option<super::gamepad::GamepadMapping> {
        None
    }
}

#[cfg(test)]
mod tests;
