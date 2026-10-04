// Rust translation of src/joystick/SDL_joystick.c, SDL_sysjoystick.h,
// SDL_joystick_c.h and include/SDL3/SDL_joystick.h from Simple DirectMedia
// Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Joysticks: low level access to game controllers.
//!
//! A [`Joystick`] is an open joystick device; it is closed when the last
//! handle to it is dropped (upstream reference counts `SDL_OpenJoystick()`
//! calls). Joysticks are identified by a [`JoystickID`] that stays the same
//! while the device is connected, and by a stable [`Guid`] describing the
//! kind of device. For game controllers with a standard layout, the
//! [`gamepad`] module maps joystick inputs to named buttons and axes.
//!
//! The joystick drivers are platform backends. So far the Linux driver
//! (evdev devices, found through libudev or inotify), the virtual driver
//! ([`attach_virtual_joystick`]) and the dummy driver (on platforms without
//! a driver), which reports no devices, exist; the HIDAPI drivers and the
//! other platform drivers arrive later.

mod controller_type;
mod device_info;
#[cfg(not(target_os = "linux"))]
mod dummy;
pub mod gamepad;
mod gamepad_db;
#[cfg(target_os = "linux")]
pub(crate) mod linux;
mod steam_virtual_gamepad;
mod tables;
mod usb_ids;
mod vidpid;
mod virtual_joystick;

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

pub use device_info::joystick_guid_info;
pub(crate) use device_info::*;
pub use virtual_joystick::{
    VirtualJoystickDesc, VirtualJoystickSensorDesc, VirtualJoystickTouchpadDesc,
};

use crate::error::{Error, Result};
use crate::events::queue::{self, EVENT_LOCK};
use crate::events::{
    Event, EventType, GamepadCapSenseEvent, GamepadDeviceEvent, GamepadSensorEvent,
    GamepadTouchpadEvent, JoyAxisEvent, JoyBallEvent, JoyBatteryEvent, JoyButtonEvent,
    JoyDeviceEvent, JoyHatEvent, JoystickID, SensorID,
};
use crate::guid::Guid;
use crate::hints;
use crate::init::{self, InitFlags};
use crate::power::PowerState;
use crate::properties::Properties;
use crate::sensor::{Sensor, SensorType};
use crate::thread::{RawMutexGuard, ReentrantMutex};
use crate::timer;
use gamepad::{GamepadAxis, GamepadCapSenseType, GamepadMapping, GamepadType};
use steam_virtual_gamepad::SteamVirtualGamepadInfo;
use usb_ids::*;

/// The largest value an axis reports. Translation of `SDL_JOYSTICK_AXIS_MAX`.
pub const JOYSTICK_AXIS_MAX: i16 = 32767;
/// The smallest value an axis reports. Translation of `SDL_JOYSTICK_AXIS_MIN`.
pub const JOYSTICK_AXIS_MIN: i16 = -32768;

/// Hat positions (bit flags). Translation of `SDL_HAT_CENTERED`.
pub const HAT_CENTERED: u8 = 0x00;
/// Translation of `SDL_HAT_UP`.
pub const HAT_UP: u8 = 0x01;
/// Translation of `SDL_HAT_RIGHT`.
pub const HAT_RIGHT: u8 = 0x02;
/// Translation of `SDL_HAT_DOWN`.
pub const HAT_DOWN: u8 = 0x04;
/// Translation of `SDL_HAT_LEFT`.
pub const HAT_LEFT: u8 = 0x08;
/// Translation of `SDL_HAT_RIGHTUP`.
pub const HAT_RIGHTUP: u8 = HAT_RIGHT | HAT_UP;
/// Translation of `SDL_HAT_RIGHTDOWN`.
pub const HAT_RIGHTDOWN: u8 = HAT_RIGHT | HAT_DOWN;
/// Translation of `SDL_HAT_LEFTUP`.
pub const HAT_LEFTUP: u8 = HAT_LEFT | HAT_UP;
/// Translation of `SDL_HAT_LEFTDOWN`.
pub const HAT_LEFTDOWN: u8 = HAT_LEFT | HAT_DOWN;

/// true if this joystick has an LED that has adjustable brightness.
/// Translation of `SDL_PROP_JOYSTICK_CAP_MONO_LED_BOOLEAN`.
pub const PROP_JOYSTICK_CAP_MONO_LED_BOOLEAN: &str = "SDL.joystick.cap.mono_led";
/// true if this joystick has an LED that has adjustable color.
/// Translation of `SDL_PROP_JOYSTICK_CAP_RGB_LED_BOOLEAN`.
pub const PROP_JOYSTICK_CAP_RGB_LED_BOOLEAN: &str = "SDL.joystick.cap.rgb_led";
/// true if this joystick has a player LED.
/// Translation of `SDL_PROP_JOYSTICK_CAP_PLAYER_LED_BOOLEAN`.
pub const PROP_JOYSTICK_CAP_PLAYER_LED_BOOLEAN: &str = "SDL.joystick.cap.player_led";
/// true if this joystick has left/right rumble.
/// Translation of `SDL_PROP_JOYSTICK_CAP_RUMBLE_BOOLEAN`.
pub const PROP_JOYSTICK_CAP_RUMBLE_BOOLEAN: &str = "SDL.joystick.cap.rumble";
/// true if this joystick has simple trigger rumble.
/// Translation of `SDL_PROP_JOYSTICK_CAP_TRIGGER_RUMBLE_BOOLEAN`.
pub const PROP_JOYSTICK_CAP_TRIGGER_RUMBLE_BOOLEAN: &str = "SDL.joystick.cap.trigger_rumble";

/// An enum of some common joystick types. Translation of `SDL_JoystickType`.
///
/// In some cases, SDL can identify a low-level joystick as being a certain
/// type of device, and will report it through
/// [`Joystick::joystick_type`] (or [`joystick_type_for_id`]).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum JoystickType {
    #[default]
    Unknown,
    Gamepad,
    Wheel,
    ArcadeStick,
    FlightStick,
    DancePad,
    Guitar,
    DrumKit,
    ArcadePad,
    Throttle,
}

impl JoystickType {
    /// The number of joystick types (`SDL_JOYSTICK_TYPE_COUNT`).
    pub const COUNT: usize = 10;

    /// The joystick type of a raw value, [`JoystickType::Unknown`] for an
    /// unknown one.
    pub fn from_u8(v: u8) -> JoystickType {
        match v {
            1 => JoystickType::Gamepad,
            2 => JoystickType::Wheel,
            3 => JoystickType::ArcadeStick,
            4 => JoystickType::FlightStick,
            5 => JoystickType::DancePad,
            6 => JoystickType::Guitar,
            7 => JoystickType::DrumKit,
            8 => JoystickType::ArcadePad,
            9 => JoystickType::Throttle,
            _ => JoystickType::Unknown,
        }
    }
}

/// Possible connection states for a joystick device.
/// Translation of `SDL_JoystickConnectionState`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum JoystickConnectionState {
    Invalid = -1,
    #[default]
    Unknown,
    Wired,
    Wireless,
}

/// Windows and Mac OSX has a limit of MAX_DWORD / 1000, Linux kernel has a
/// limit of 0xFFFF. Translation of `SDL_MAX_RUMBLE_DURATION_MS`.
const MAX_RUMBLE_DURATION_MS: u32 = 0xFFFF;

/// Dualshock4 only rumbles for about 5 seconds max, resend rumble command
/// every 2 seconds to make long rumble work. Translation of `SDL_RUMBLE_RESEND_MS`.
const RUMBLE_RESEND_MS: u64 = 2000;

/// Translation of `SDL_LED_MIN_REPEAT_MS`.
const LED_MIN_REPEAT_MS: u64 = 5000;

/// Translation of `SDL_JoystickAxisInfo`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct AxisInfo {
    /// Initial axis state
    pub(crate) initial_value: i16,
    /// Current axis state
    pub(crate) value: i16,
    /// Zero point on the axis (-32768 for triggers)
    pub(crate) zero: i16,
    /// Whether we've seen a value on the axis yet
    pub(crate) has_initial_value: bool,
    /// Whether we've seen a second value on the axis yet
    pub(crate) has_second_value: bool,
    /// Whether we've sent the initial axis value
    pub(crate) sent_initial_value: bool,
    /// Whether we are sending the initial axis value
    pub(crate) sending_initial_value: bool,
}

/// Translation of `SDL_JoystickBallData`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct BallData {
    pub(crate) dx: i32,
    pub(crate) dy: i32,
}

/// Translation of `SDL_JoystickTouchpadFingerInfo`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct TouchpadFingerInfo {
    pub(crate) down: bool,
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) pressure: f32,
}

/// Translation of `SDL_JoystickTouchpadInfo`.
#[derive(Clone, Debug, Default)]
pub(crate) struct TouchpadInfo {
    pub(crate) fingers: Vec<TouchpadFingerInfo>,
}

/// Translation of `SDL_JoystickSensorInfo`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct JoystickSensorInfo {
    pub(crate) sensor_type: SensorType,
    pub(crate) enabled: bool,
    pub(crate) rate: f32,
    /// If this needs to expand, update `GamepadSensorEvent`
    pub(crate) data: [f32; 3],
}

/// Translation of `SDL_JoystickCapSenseInfo`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct CapSenseInfo {
    pub(crate) capsense_type: GamepadCapSenseType,
    pub(crate) down: bool,
}

/// The state of an open joystick. Translation of `struct SDL_Joystick`.
pub(crate) struct JoystickData {
    /// Device instance, monotonically increasing from 0
    pub(crate) instance_id: JoystickID,
    /// Joystick name - system dependent
    pub(crate) name: Option<String>,
    /// Joystick path - system dependent
    pub(crate) path: Option<String>,
    /// Joystick serial
    pub(crate) serial: Option<String>,
    /// Joystick guid
    pub(crate) guid: Guid,
    /// Firmware version, if available
    pub(crate) firmware_version: u16,
    /// Steam controller API handle
    pub(crate) steam_handle: u64,
    /// Whether we should swap face buttons
    pub(crate) swap_face_buttons: bool,
    /// Whether this is a virtual joystick
    pub(crate) is_virtual: bool,

    /// Number of axis controls on the joystick (set by the driver's open)
    pub(crate) naxes: usize,
    pub(crate) axes: Vec<AxisInfo>,

    /// Number of trackballs on the joystick (set by the driver's open)
    pub(crate) nballs: usize,
    /// Current ball motion deltas
    pub(crate) balls: Vec<BallData>,

    /// Number of hats on the joystick (set by the driver's open)
    pub(crate) nhats: usize,
    /// Current hat states
    pub(crate) hats: Vec<u8>,

    /// Number of buttons on the joystick (set by the driver's open)
    pub(crate) nbuttons: usize,
    /// Current button states
    pub(crate) buttons: Vec<bool>,

    /// Current touchpad states
    pub(crate) touchpads: Vec<TouchpadInfo>,

    pub(crate) nsensors_enabled: i32,
    pub(crate) sensors: Vec<JoystickSensorInfo>,

    /// Current capsense states
    pub(crate) capsenses: Vec<CapSenseInfo>,

    low_frequency_rumble: u16,
    high_frequency_rumble: u16,
    rumble_expiration: u64,
    rumble_resend: u64,

    left_trigger_rumble: u16,
    right_trigger_rumble: u16,
    trigger_rumble_expiration: u64,
    trigger_rumble_resend: u64,

    led_red: u8,
    led_green: u8,
    led_blue: u8,
    led_expiration: u64,

    pub(crate) attached: bool,
    pub(crate) connection_state: JoystickConnectionState,
    battery_state: PowerState,
    battery_percent: i32,

    /// true if this device has the guide button event delayed
    pub(crate) delayed_guide_button: bool,

    pub(crate) accel_sensor: SensorID,
    pub(crate) accel: Option<Sensor>,
    pub(crate) gyro_sensor: SensorID,
    pub(crate) gyro: Option<Sensor>,
    pub(crate) sensor_transform: [[f32; 3]; 3],

    /// The timestamp (ns) of the last state change, for the update complete event
    pub(crate) update_complete: u64,

    /// The index of the driver in the driver list
    driver: usize,

    /// Driver dependent information
    pub(crate) hwdata: Option<Box<dyn std::any::Any + Send>>,

    props: Option<Properties>,

    /// Reference count for multiple opens
    ref_count: i32,
    /// Identifies this open joystick (handles of an earlier open of the same
    /// instance id don't refer to it).
    open_serial: u64,
}

impl JoystickData {
    fn new(instance_id: JoystickID) -> JoystickData {
        JoystickData {
            instance_id,
            name: None,
            path: None,
            serial: None,
            guid: Guid::ZERO,
            firmware_version: 0,
            steam_handle: 0,
            swap_face_buttons: false,
            is_virtual: false,
            naxes: 0,
            axes: Vec::new(),
            nballs: 0,
            balls: Vec::new(),
            nhats: 0,
            hats: Vec::new(),
            nbuttons: 0,
            buttons: Vec::new(),
            touchpads: Vec::new(),
            nsensors_enabled: 0,
            sensors: Vec::new(),
            capsenses: Vec::new(),
            low_frequency_rumble: 0,
            high_frequency_rumble: 0,
            rumble_expiration: 0,
            rumble_resend: 0,
            left_trigger_rumble: 0,
            right_trigger_rumble: 0,
            trigger_rumble_expiration: 0,
            trigger_rumble_resend: 0,
            led_red: 0,
            led_green: 0,
            led_blue: 0,
            led_expiration: 0,
            attached: false,
            connection_state: JoystickConnectionState::Unknown,
            battery_state: PowerState::Unknown,
            battery_percent: 0,
            delayed_guide_button: false,
            accel_sensor: 0,
            accel: None,
            gyro_sensor: 0,
            gyro: None,
            sensor_transform: [[0.0; 3]; 3],
            update_complete: 0,
            driver: 0,
            hwdata: None,
            props: None,
            ref_count: 0,
            open_serial: 0,
        }
    }

    /// The joystick's properties, created on first use
    /// (`SDL_GetJoystickProperties()` for drivers).
    pub(crate) fn properties(&mut self) -> Properties {
        self.props.get_or_insert_with(Properties::new).clone()
    }

    /// Add a touchpad with `nfingers` simultaneous fingers.
    /// Translation of `SDL_PrivateJoystickAddTouchpad()`.
    pub(crate) fn add_touchpad(&mut self, nfingers: usize) {
        assert_joysticks_locked();

        self.touchpads.push(TouchpadInfo {
            fingers: vec![TouchpadFingerInfo::default(); nfingers],
        });
    }

    /// Add a sensor. Translation of `SDL_PrivateJoystickAddSensor()`.
    pub(crate) fn add_sensor(&mut self, sensor_type: SensorType, rate: f32) {
        assert_joysticks_locked();

        self.sensors.push(JoystickSensorInfo {
            sensor_type,
            enabled: false,
            rate,
            data: [0.0; 3],
        });
    }

    /// Update the data rate of a sensor. Translation of `SDL_PrivateJoystickSensorRate()`.
    #[allow(dead_code)] // (used by the HIDAPI drivers)
    pub(crate) fn set_sensor_rate(&mut self, sensor_type: SensorType, rate: f32) {
        assert_joysticks_locked();

        for sensor in &mut self.sensors {
            if sensor.sensor_type == sensor_type {
                sensor.rate = rate;
            }
        }
    }

    /// Add a capacitive sensor. Translation of `SDL_PrivateJoystickAddCapSense()`.
    #[allow(dead_code)] // (used by the HIDAPI drivers)
    pub(crate) fn add_capsense(&mut self, capsense_type: GamepadCapSenseType) {
        assert_joysticks_locked();

        self.capsenses.push(CapSenseInfo {
            capsense_type,
            down: false,
        });
    }
}

/// The functions of a joystick backend. Translation of `SDL_JoystickDriver`.
///
/// The functions take `&self`: a driver keeps its state behind its own
/// interior mutability, since the update function delivers events whose
/// watchers may call back into the joystick API. Functions acting on an
/// open joystick take its instance id; the front end's state for it is
/// reachable with [`with_joystick`].
pub(crate) trait JoystickDriver: Send + Sync {
    /// Scan the system for joysticks. Joystick 0 should be the system
    /// default joystick. Fails on an unrecoverable error.
    fn init(&self) -> Result<()>;

    /// The number of joystick devices plugged in right now.
    fn count(&self) -> usize;

    /// Cause any queued joystick insertions to be processed.
    fn detect(&self);

    /// Whether a device is currently detected by this driver.
    fn is_device_present(
        &self,
        vendor_id: u16,
        product_id: u16,
        version: u16,
        name: Option<&str>,
    ) -> bool;

    /// The device-dependent name of a joystick.
    fn device_name(&self, device_index: usize) -> Option<String>;

    /// The device-dependent path of a joystick.
    fn device_path(&self, device_index: usize) -> Option<String>;

    /// The Steam virtual gamepad slot of a joystick, or -1.
    fn device_steam_virtual_gamepad_slot(&self, device_index: usize) -> i32;

    /// The player index of a joystick, or -1.
    fn device_player_index(&self, device_index: usize) -> i32;

    /// Set the player index of a joystick.
    fn set_device_player_index(&self, device_index: usize, player_index: i32);

    /// The stable GUID for a plugged in device.
    fn device_guid(&self, device_index: usize) -> Guid;

    /// The current instance id of the joystick located at `device_index`.
    fn device_instance_id(&self, device_index: usize) -> JoystickID;

    /// Open a joystick for use. The joystick to open is specified by the
    /// device index. This should fill the `nbuttons` and `naxes` fields of
    /// the joystick structure.
    fn open(&self, joystick: &mut JoystickData, device_index: usize) -> Result<()>;

    /// Rumble functionality.
    fn rumble(
        &self,
        joystick: JoystickID,
        low_frequency_rumble: u16,
        high_frequency_rumble: u16,
    ) -> Result<()>;
    /// Trigger rumble functionality.
    fn rumble_triggers(
        &self,
        joystick: JoystickID,
        left_rumble: u16,
        right_rumble: u16,
    ) -> Result<()>;

    /// LED functionality.
    fn set_led(&self, joystick: JoystickID, red: u8, green: u8, blue: u8) -> Result<()>;

    /// General effects.
    fn send_effect(&self, joystick: JoystickID, data: &[u8]) -> Result<()>;

    /// Sensor functionality.
    fn set_sensors_enabled(&self, joystick: JoystickID, enabled: bool) -> Result<()>;

    /// Update the state of a joystick - called as a device poll. This
    /// function shouldn't update the joystick structure directly, but
    /// instead should call the `send_joystick_*` functions to deliver events
    /// and update joystick device state.
    fn update(&self, joystick: JoystickID);

    /// Close a joystick after use.
    fn close(&self, joystick: &mut JoystickData);

    /// Perform any system-specific joystick related cleanup.
    fn quit(&self);

    /// The autodetected controller mapping, if there is one.
    fn gamepad_mapping(&self, device_index: usize) -> Option<GamepadMapping>;
}

/// The index of the Linux driver in [`JOYSTICK_DRIVERS`].
#[cfg(target_os = "linux")]
const LINUX_DRIVER_INDEX: usize = 0;

/// The index of the virtual driver in [`JOYSTICK_DRIVERS`] (after the
/// platform driver, if there is one).
const VIRTUAL_DRIVER_INDEX: usize = if cfg!(target_os = "linux") { 1 } else { 0 };

/// The available joystick drivers, in priority order. Translation of
/// `SDL_joystick_drivers`; the dummy driver is only there without a
/// platform driver, as upstream builds it.
static JOYSTICK_DRIVERS: &[&dyn JoystickDriver] = &[
    #[cfg(target_os = "linux")]
    &linux::LINUX_JOYSTICK_DRIVER,
    &virtual_joystick::VIRTUAL_JOYSTICK_DRIVER,
    #[cfg(not(target_os = "linux"))]
    &dummy::DUMMY_JOYSTICK_DRIVER,
];

/// Translation of `SDL_joysticks_locked`.
static JOYSTICKS_LOCKED: AtomicI32 = AtomicI32::new(0);
/// Translation of `SDL_joysticks_initialized`.
static JOYSTICKS_INITIALIZED: AtomicBool = AtomicBool::new(false);
/// Translation of `SDL_joysticks_quitting`.
static JOYSTICKS_QUITTING: AtomicBool = AtomicBool::new(false);
/// Translation of `SDL_joystick_being_added`.
static JOYSTICK_BEING_ADDED: AtomicBool = AtomicBool::new(false);
/// Translation of `SDL_joystick_allows_background_events`.
static JOYSTICK_ALLOWS_BACKGROUND_EVENTS: AtomicBool = AtomicBool::new(false);
static ALLOW_BACKGROUND_EVENTS_WATCH: Mutex<Option<hints::Callback>> = Mutex::new(None);
static NEXT_SERIAL: AtomicU64 = AtomicU64::new(1);

/// The state guarded by `SDL_event_lock` upstream.
struct JoystickState {
    /// Translation of `SDL_joysticks`: the open joysticks, most recently
    /// opened first.
    joysticks: Vec<JoystickData>,
    /// Translation of `SDL_joystick_players`.
    players: Vec<JoystickID>,
    /// Translation of `SDL_joystick_names`.
    names: Option<HashMap<JoystickID, String>>,
}

/// The `RefCell` borrow is never held across a driver call or an event push.
static JOYSTICKS: ReentrantMutex<RefCell<JoystickState>> =
    ReentrantMutex::new(RefCell::new(JoystickState {
        joysticks: Vec::new(),
        players: Vec::new(),
        names: None,
    }));

/// The joystick lock (`SDL_event_lock`, counted); unlocks on drop.
///
/// The joystick functions take the lock themselves; hold it to make a
/// series of calls atomic with respect to other threads (for instance to
/// enumerate joysticks and open them without one being removed between the
/// two).
pub struct JoystickLock {
    _guard: RawMutexGuard<'static>,
}

impl std::fmt::Debug for JoystickLock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("JoystickLock")
    }
}

impl Drop for JoystickLock {
    fn drop(&mut self) {
        // Translation of `SDL_UnlockJoysticks()`.
        JOYSTICKS_LOCKED.fetch_sub(1, Ordering::Relaxed);
    }
}

/// Locking for atomic access to the joystick API. Translation of
/// `SDL_LockJoysticks()`; dropping the guard is `SDL_UnlockJoysticks()`.
pub fn lock_joysticks() -> JoystickLock {
    let guard = EVENT_LOCK.guard();
    JOYSTICKS_LOCKED.fetch_add(1, Ordering::Relaxed);
    JoystickLock { _guard: guard }
}

/// Translation of `SDL_JoysticksLocked()`.
pub(crate) fn joysticks_locked() -> bool {
    JOYSTICKS_LOCKED.load(Ordering::Relaxed) > 0
}

/// Translation of `SDL_AssertJoysticksLocked()`.
pub(crate) fn assert_joysticks_locked() {
    crate::sdl_assert!(joysticks_locked());
}

fn with_state<R>(f: impl FnOnce(&mut JoystickState) -> R) -> R {
    let guard = JOYSTICKS.lock();
    let mut state = guard.borrow_mut();
    f(&mut state)
}

/// Run `f` on the open joystick `instance_id` (the lock must be held).
pub(crate) fn with_joystick<R>(
    instance_id: JoystickID,
    f: impl FnOnce(&mut JoystickData) -> R,
) -> Option<R> {
    with_state(|s| {
        s.joysticks
            .iter_mut()
            .find(|j| j.instance_id == instance_id)
            .map(f)
    })
}

/// Translation of `SDL_JoysticksInitialized()`.
pub(crate) fn joysticks_initialized() -> bool {
    JOYSTICKS_INITIALIZED.load(Ordering::Relaxed)
}

/// Translation of `SDL_JoysticksQuitting()`.
pub(crate) fn joysticks_quitting() -> bool {
    JOYSTICKS_QUITTING.load(Ordering::Relaxed)
}

/// Get the driver and device index for a joystick instance ID. This should
/// be called while the joystick lock is held, to prevent another thread
/// from updating the list. Translation of `SDL_GetDriverAndJoystickIndex()`.
fn driver_and_joystick_index(instance_id: JoystickID) -> Result<(usize, usize)> {
    assert_joysticks_locked();

    if instance_id > 0 {
        for (i, driver) in JOYSTICK_DRIVERS.iter().enumerate() {
            let num_joysticks = driver.count();
            for device_index in 0..num_joysticks {
                let joystick_id = driver.device_instance_id(device_index);
                if joystick_id == instance_id {
                    return Ok((i, device_index));
                }
            }
        }
    }

    Err(Error::new(format!("Joystick {instance_id} not found")))
}

/// Translation of `SDL_FindFreePlayerIndex()`.
fn find_free_player_index() -> i32 {
    assert_joysticks_locked();

    with_state(|s| {
        s.players
            .iter()
            .position(|&id| id == 0)
            .unwrap_or(s.players.len()) as i32
    })
}

/// Translation of `SDL_GetPlayerIndexForJoystickID()`.
fn player_index_for_joystick_id(instance_id: JoystickID) -> i32 {
    assert_joysticks_locked();

    with_state(|s| {
        s.players
            .iter()
            .position(|&id| id == instance_id)
            .map_or(-1, |i| i as i32)
    })
}

/// Translation of `SDL_GetJoystickIDForPlayerIndex()`.
fn joystick_id_for_player_index(player_index: i32) -> JoystickID {
    assert_joysticks_locked();

    if player_index < 0 {
        return 0;
    }
    with_state(|s| s.players.get(player_index as usize).copied().unwrap_or(0))
}

/// Translation of `SDL_SetJoystickIDForPlayerIndex()`.
fn set_joystick_id_for_player_index(player_index: i32, instance_id: JoystickID) {
    let existing_instance = joystick_id_for_player_index(player_index);

    assert_joysticks_locked();

    let already_assigned = with_state(|s| {
        if player_index >= s.players.len() as i32 {
            s.players.resize(player_index as usize + 1, 0);
            false
        } else {
            // Joystick is already assigned the requested player index
            player_index >= 0 && s.players[player_index as usize] == instance_id
        }
    });
    if already_assigned {
        return;
    }

    // Clear the old player index
    let existing_player_index = player_index_for_joystick_id(instance_id);
    with_state(|s| {
        if existing_player_index >= 0 {
            s.players[existing_player_index as usize] = 0;
        }

        if player_index >= 0 {
            s.players[player_index as usize] = instance_id;
        }
    });

    // Update the driver with the new index
    if let Ok((driver, device_index)) = driver_and_joystick_index(instance_id) {
        JOYSTICK_DRIVERS[driver].set_device_player_index(device_index, player_index);
    }

    // Move any existing joystick to another slot
    if existing_instance > 0 {
        set_joystick_id_for_player_index(find_free_player_index(), existing_instance);
    }
}

/// Translation of `SDL_InitJoysticks()`.
pub(crate) fn init_joysticks() -> Result<()> {
    init::init_subsystem(InitFlags::EVENTS)?;

    let result = {
        let _lock = lock_joysticks();

        JOYSTICKS_INITIALIZED.store(true, Ordering::Relaxed);

        with_state(|s| s.names = Some(HashMap::new()));

        for list in &VIDPID_LISTS {
            list.load();
        }

        gamepad::init_gamepad_mappings();

        // See if we should allow joystick events while in the background
        // (translation of `SDL_JoystickAllowBackgroundEventsChanged()`)
        let watch = hints::watch(hints::JOYSTICK_ALLOW_BACKGROUND_EVENTS, |change| {
            JOYSTICK_ALLOWS_BACKGROUND_EVENTS.store(
                hints::string_to_bool(change.new_value, false),
                Ordering::Relaxed,
            );
        });
        *ALLOW_BACKGROUND_EVENTS_WATCH
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = watch.ok();

        steam_virtual_gamepad::init_steam_virtual_gamepad_info();

        let mut result = Err(Error::new("No joystick drivers available"));
        for driver in JOYSTICK_DRIVERS {
            match driver.init() {
                Ok(()) => result = Ok(()),
                Err(e) if result.is_err() => result = Err(e),
                Err(_) => {}
            }
        }
        result
    };

    if result.is_err() {
        quit_joysticks();
    }

    result
}

/// Whether there are any joysticks opened by the application.
/// Translation of `SDL_JoysticksOpened()`.
pub(crate) fn joysticks_opened() -> bool {
    let _lock = lock_joysticks();
    with_state(|s| !s.joysticks.is_empty())
}

/// Whether a device is handled by a higher priority driver than `driver`.
/// Translation of `SDL_JoystickHandledByAnotherDriver()`.
#[allow(dead_code)] // (used by the platform drivers)
pub(crate) fn joystick_handled_by_another_driver(
    driver: usize,
    vendor_id: u16,
    product_id: u16,
    version: u16,
    name: Option<&str>,
) -> bool {
    let _lock = lock_joysticks();
    // (Higher priority drivers come before this one)
    JOYSTICK_DRIVERS[..driver.min(JOYSTICK_DRIVERS.len())]
        .iter()
        .any(|d| d.is_device_present(vendor_id, product_id, version, name))
}

/// Whether a joystick is currently connected. Translation of `SDL_HasJoystick()`.
pub fn has_joystick() -> bool {
    let total_joysticks: usize = {
        let _lock = lock_joysticks();
        JOYSTICK_DRIVERS.iter().map(|d| d.count()).sum()
    };
    total_joysticks > 0
}

/// The currently connected joysticks. Translation of `SDL_GetJoysticks()`.
pub fn joysticks() -> Vec<JoystickID> {
    let _lock = lock_joysticks();

    let total_joysticks: usize = JOYSTICK_DRIVERS.iter().map(|d| d.count()).sum();
    let mut joysticks = Vec::with_capacity(total_joysticks);
    for driver in JOYSTICK_DRIVERS {
        let num_joysticks = driver.count();
        for device_index in 0..num_joysticks {
            crate::sdl_assert!(joysticks.len() < total_joysticks);
            let id = driver.device_instance_id(device_index);
            crate::sdl_assert!(id > 0);
            joysticks.push(id);
        }
    }
    crate::sdl_assert!(joysticks.len() == total_joysticks);
    joysticks
}

/// The Steam virtual gamepad info for a joystick.
/// Translation of `SDL_GetJoystickVirtualGamepadInfoForID()`.
pub(crate) fn joystick_virtual_gamepad_info_for_id(
    instance_id: JoystickID,
) -> Option<SteamVirtualGamepadInfo> {
    if !steam_virtual_gamepad::steam_virtual_gamepad_enabled() {
        return None;
    }
    let (driver, device_index) = driver_and_joystick_index(instance_id).ok()?;
    steam_virtual_gamepad::steam_virtual_gamepad_info(
        JOYSTICK_DRIVERS[driver].device_steam_virtual_gamepad_slot(device_index),
    )
}

/// The implementation dependent name of a joystick, remembered after the
/// device goes away. Translation of `SDL_UpdateJoystickNameForID()`.
fn update_joystick_name_for_id(instance_id: JoystickID) -> Result<Option<String>> {
    assert_joysticks_locked();

    let mut not_found = None;
    let current_name = if let Some(info) = joystick_virtual_gamepad_info_for_id(instance_id) {
        info.name
    } else {
        match driver_and_joystick_index(instance_id) {
            Ok((driver, device_index)) => JOYSTICK_DRIVERS[driver].device_name(device_index),
            Err(e) => {
                not_found = Some(e);
                None
            }
        }
    };

    with_state(|s| {
        let Some(names) = &mut s.names else {
            return match (current_name, not_found) {
                (None, Some(e)) => Err(e),
                (name, _) => Ok(name),
            };
        };

        let name = names.get(&instance_id);
        let Some(current_name) = current_name else {
            return match name {
                Some(name) => Ok(Some(name.clone())),
                None => Err(Error::new(format!("Joystick {instance_id} not found"))),
            };
        };

        if name != Some(&current_name) {
            names.insert(instance_id, current_name.clone());
        }
        Ok(Some(current_name))
    })
}

/// The implementation dependent name of a joystick (also available after
/// it is removed). Translation of `SDL_GetJoystickNameForID()`.
pub fn joystick_name_for_id(instance_id: JoystickID) -> Result<Option<String>> {
    let _lock = lock_joysticks();
    update_joystick_name_for_id(instance_id)
}

/// The implementation dependent path of a joystick.
/// Translation of `SDL_GetJoystickPathForID()`.
pub fn joystick_path_for_id(instance_id: JoystickID) -> Result<String> {
    let path = {
        let _lock = lock_joysticks();
        let (driver, device_index) = driver_and_joystick_index(instance_id)?;
        JOYSTICK_DRIVERS[driver].device_path(device_index)
    };

    path.ok_or_else(Error::unsupported)
}

/// The player index of a joystick, or -1 if it's not available.
/// Translation of `SDL_GetJoystickPlayerIndexForID()`.
pub fn joystick_player_index_for_id(instance_id: JoystickID) -> i32 {
    let _lock = lock_joysticks();
    player_index_for_joystick_id(instance_id)
}

/// The USB vendor and product IDs of an open joystick's state, honoring the
/// Steam virtual gamepad information (`SDL_GetJoystickVendor()` /
/// `SDL_GetJoystickProduct()` on a joystick being opened).
fn vendor_and_product(instance_id: JoystickID, guid: Guid) -> (u16, u16) {
    match joystick_virtual_gamepad_info_for_id(instance_id) {
        Some(info) => (info.vendor_id, info.product_id),
        None => {
            let (vendor, product, _, _) = joystick_guid_info(guid);
            (vendor, product)
        }
    }
}

/// Return true if this joystick is known to have all axes centered at zero.
/// This isn't generally needed unless the joystick never generates an
/// initial axis value near zero, e.g. it's emulating axes with digital
/// buttons. Translation of `SDL_JoystickAxesCenteredAtZero()`.
fn joystick_axes_centered_at_zero(joystick: &JoystickData) -> bool {
    assert_joysticks_locked();

    if joystick.naxes == 2 {
        // Assume D-pad or thumbstick style axes are centered at 0
        return true;
    }

    let (vendor, product) = vendor_and_product(joystick.instance_id, joystick.guid);
    ZERO_CENTERED_DEVICES.contains(vendor, product)
}

/// Translation of `IsROGAlly()`.
fn is_rog_ally(guid: Guid) -> bool {
    // The ROG Ally controller spoofs an Xbox 360 controller
    let (vendor, product, _, _) = joystick_guid_info(guid);
    if vendor == USB_VENDOR_MICROSOFT && product == USB_PRODUCT_XBOX360_WIRED_CONTROLLER {
        // Check to see if this system has the expected sensors
        let mut has_ally_accel = false;
        let mut has_ally_gyro = false;

        if init::init_subsystem(InitFlags::SENSOR).is_ok() {
            for sensor in crate::sensor::sensors() {
                if !has_ally_accel
                    && crate::sensor::sensor_type_for_id(sensor).ok() == Some(SensorType::Accel)
                {
                    if let Ok(Some(sensor_name)) = crate::sensor::sensor_name_for_id(sensor) {
                        if sensor_name == "Sensor BMI320 Acc" {
                            has_ally_accel = true;
                        }
                    }
                }
                if !has_ally_gyro
                    && crate::sensor::sensor_type_for_id(sensor).ok() == Some(SensorType::Gyro)
                {
                    if let Ok(Some(sensor_name)) = crate::sensor::sensor_name_for_id(sensor) {
                        if sensor_name == "Sensor BMI320 Gyr" {
                            has_ally_gyro = true;
                        }
                    }
                }
            }
            init::quit_subsystem(InitFlags::SENSOR);
        }
        if has_ally_accel && has_ally_gyro {
            return true;
        }
    }
    false
}

/// Whether to use the system gyro and accelerometer for a gamepad, and
/// whether to invert their data. Translation of `ShouldAttemptSensorFusion()`.
fn should_attempt_sensor_fusion(joystick: &JoystickData) -> Option<bool> {
    assert_joysticks_locked();

    // The SDL controller sensor API is only available for gamepads (at the moment)
    if !gamepad::is_gamepad(joystick.instance_id) {
        return None;
    }

    // If the controller already has sensors, use those
    if !joystick.sensors.is_empty() {
        return None;
    }

    if let Some(hint) = hints::get(hints::GAMECONTROLLER_SENSOR_FUSION).filter(|h| !h.is_empty()) {
        if hint.starts_with('@') || hint.starts_with("0x") {
            // See if the gamepad is in our list of devices to enable
            let (vendor, product, _, _) = joystick_guid_info(joystick.guid);
            let in_list = vidpid::vidpid_list_from_hint(&hint);
            if in_list(vendor, product) {
                return Some(false);
            }
        } else {
            return hints::string_to_bool(Some(&hint), false).then_some(false);
        }
    }

    // See if this is another known wraparound gamepad
    if joystick
        .name
        .as_deref()
        .is_some_and(|name| name.contains("Backbone One") || name.contains("Kishi"))
    {
        return Some(false);
    }
    if is_rog_ally(joystick.guid) {
        /* I'm not sure if this is a Windows thing, or a quirk for ROG Ally,
         * but we need to invert the sensor data on all axes.
         */
        return Some(true);
    }
    None
}

/// Translation of `AttemptSensorFusion()`.
fn attempt_sensor_fusion(joystick: &mut JoystickData, invert_sensors: bool) {
    assert_joysticks_locked();

    if init::init_subsystem(InitFlags::SENSOR).is_err() {
        return;
    }

    for sensor in crate::sensor::sensors() {
        if joystick.accel_sensor == 0
            && crate::sensor::sensor_type_for_id(sensor).ok() == Some(SensorType::Accel)
        {
            // Increment the sensor subsystem reference count
            let _ = init::init_subsystem(InitFlags::SENSOR);

            joystick.accel_sensor = sensor;
            joystick.add_sensor(SensorType::Accel, 0.0);
        }
        if joystick.gyro_sensor == 0
            && crate::sensor::sensor_type_for_id(sensor).ok() == Some(SensorType::Gyro)
        {
            // Increment the sensor subsystem reference count
            let _ = init::init_subsystem(InitFlags::SENSOR);

            joystick.gyro_sensor = sensor;
            joystick.add_sensor(SensorType::Gyro, 0.0);
        }
    }
    init::quit_subsystem(InitFlags::SENSOR);

    /* SDL defines sensor orientation for phones relative to the natural
      orientation, and for gamepads relative to being held in front of you.
      When a phone is being used as a gamepad, its orientation changes,
      so adjust sensor axes to match.
    */
    // (Without the video subsystem the natural orientation is unknown, which
    // takes the portrait branch.)
    {
        /* When a device in portrait orientation is rotated left and laid flat,
           the axes change orientation as follows:
            -X to +X becomes +Z to -Z
            -Y to +Y becomes +X to -X
            -Z to +Z becomes -Y to +Y
        */
        joystick.sensor_transform[0][1] = -1.0;
        joystick.sensor_transform[1][2] = 1.0;
        joystick.sensor_transform[2][0] = -1.0;
    }

    if invert_sensors {
        for row in &mut joystick.sensor_transform {
            for value in row {
                *value *= -1.0;
            }
        }
    }
}

/// Translation of `CleanupSensorFusion()`.
fn cleanup_sensor_fusion(instance_id: JoystickID) {
    assert_joysticks_locked();

    let Some((accel_sensor, accel, gyro_sensor, gyro)) = with_joystick(instance_id, |j| {
        let taken = (j.accel_sensor, j.accel.take(), j.gyro_sensor, j.gyro.take());
        j.accel_sensor = 0;
        j.gyro_sensor = 0;
        taken
    }) else {
        return;
    };

    if accel_sensor != 0 || gyro_sensor != 0 {
        if accel_sensor != 0 {
            drop(accel);

            // Decrement the sensor subsystem reference count
            init::quit_subsystem(InitFlags::SENSOR);
        }
        if gyro_sensor != 0 {
            drop(gyro);

            // Decrement the sensor subsystem reference count
            init::quit_subsystem(InitFlags::SENSOR);
        }
    }
}

/// Translation of `ShouldSwapFaceButtons()`.
fn should_swap_face_buttons(info: &SteamVirtualGamepadInfo) -> bool {
    // When "Use Nintendo Button Layout" is enabled under Steam (the default)
    // it will send button 0 for the A (east) button and button 1 for the
    // B (south) button. This is done so that games that interpret the
    // buttons as Xbox input will get button 0 for "A" as they expect.
    //
    // However, SDL reports positional buttons, so we need to swap
    // the buttons so they show up in the correct position. This provides
    // consistent behavior regardless of whether we're running under Steam,
    // under the default settings.
    matches!(
        info.gamepad_type,
        GamepadType::NintendoSwitchPro
            | GamepadType::NintendoSwitchJoyconLeft
            | GamepadType::NintendoSwitchJoyconRight
            | GamepadType::NintendoSwitchJoyconPair
    )
}

/// An open joystick. Translation of `SDL_Joystick *`; dropping the last
/// handle closes the joystick (`SDL_CloseJoystick()`).
#[derive(Debug)]
pub struct Joystick {
    instance_id: JoystickID,
    serial: u64,
}

impl Joystick {
    /// Open a joystick for use. Opening a joystick that is already open
    /// adds a reference to it. Translation of `SDL_OpenJoystick()`.
    pub fn open(instance_id: JoystickID) -> Result<Joystick> {
        let _lock = lock_joysticks();

        let (driver_index, device_index) = driver_and_joystick_index(instance_id)?;
        let driver = JOYSTICK_DRIVERS[driver_index];

        /* If the joystick is already open, return it
         * it is important that we have a single joystick for each instance id
         */
        if let Some(serial) = with_joystick(instance_id, |j| {
            j.ref_count += 1;
            j.open_serial
        }) {
            return Ok(Joystick {
                instance_id,
                serial,
            });
        }

        // Create and initialize the joystick
        let mut joystick = JoystickData::new(instance_id);
        joystick.driver = driver_index;
        joystick.attached = true;
        joystick.led_expiration = timer::ticks_ms();
        joystick.battery_percent = -1;
        joystick.is_virtual = driver_index == VIRTUAL_DRIVER_INDEX;

        driver.open(&mut joystick, device_index)?;

        joystick.name = driver.device_name(device_index);
        joystick.path = driver.device_path(device_index);
        joystick.guid = driver.device_guid(device_index);

        joystick.axes = vec![AxisInfo::default(); joystick.naxes];
        joystick.balls = vec![BallData::default(); joystick.nballs];
        joystick.hats = vec![0; joystick.nhats];
        joystick.buttons = vec![false; joystick.nbuttons];

        // If this joystick is known to have all zero centered axes, skip the auto-centering code
        if joystick_axes_centered_at_zero(&joystick) {
            for axis in &mut joystick.axes {
                axis.has_initial_value = true;
            }
        }

        // We know the initial values for HIDAPI and XInput joysticks
        if (is_joystick_hidapi(joystick.guid)
            || is_joystick_xinput(joystick.guid)
            || is_joystick_rawinput(joystick.guid)
            || is_joystick_wgi(joystick.guid))
            && joystick.naxes >= GamepadAxis::COUNT
        {
            let (left_trigger, right_trigger) = if is_joystick_xinput(joystick.guid) {
                (2, 5)
            } else {
                (
                    GamepadAxis::LeftTrigger as usize,
                    GamepadAxis::RightTrigger as usize,
                )
            };
            for (i, axis) in joystick
                .axes
                .iter_mut()
                .take(GamepadAxis::COUNT)
                .enumerate()
            {
                let initial_value = if i == left_trigger || i == right_trigger {
                    i16::MIN
                } else {
                    0
                };
                axis.value = initial_value;
                axis.zero = initial_value;
                axis.initial_value = initial_value;
                axis.has_initial_value = true;
            }
        }

        // Get the Steam Input API handle
        if let Some(info) = joystick_virtual_gamepad_info_for_id(instance_id) {
            joystick.steam_handle = info.handle;
            joystick.swap_face_buttons = should_swap_face_buttons(&info);
        }

        // Use system gyro and accelerometer if the gamepad doesn't have built-in sensors
        if let Some(invert_sensors) = should_attempt_sensor_fusion(&joystick) {
            attempt_sensor_fusion(&mut joystick, invert_sensors);
        }

        // Add joystick to list
        joystick.ref_count += 1;
        let serial = NEXT_SERIAL.fetch_add(1, Ordering::Relaxed);
        joystick.open_serial = serial;
        // Link the joystick in the list
        with_state(|s| s.joysticks.insert(0, joystick));

        driver.update(instance_id);

        Ok(Joystick {
            instance_id,
            serial,
        })
    }

    /// Another handle to an open joystick (adding a reference to it), or
    /// `None` if it isn't open. Translation of `SDL_GetJoystickFromID()`.
    pub fn from_id(instance_id: JoystickID) -> Option<Joystick> {
        let _lock = lock_joysticks();
        with_joystick(instance_id, |j| {
            j.ref_count += 1;
            Joystick {
                instance_id,
                serial: j.open_serial,
            }
        })
    }

    /// Another handle to the open joystick with a player index (adding a
    /// reference to it). Translation of `SDL_GetJoystickFromPlayerIndex()`.
    pub fn from_player_index(player_index: i32) -> Option<Joystick> {
        let _lock = lock_joysticks();
        let instance_id = joystick_id_for_player_index(player_index);
        Joystick::from_id(instance_id)
    }

    /// Run `f` on this joystick's state (`CHECK_JOYSTICK_MAGIC`).
    pub(crate) fn with<R>(&self, f: impl FnOnce(&mut JoystickData) -> R) -> Result<R> {
        let _lock = lock_joysticks();
        with_state(|s| {
            s.joysticks
                .iter_mut()
                .find(|j| j.instance_id == self.instance_id && j.open_serial == self.serial)
                .map(f)
        })
        .ok_or_else(|| Error::invalid_param("joystick"))
    }

    /// `CHECK_JOYSTICK_VIRTUAL`.
    fn check_virtual(&self) -> Result<()> {
        if !self.with(|j| j.is_virtual)? {
            return Err(Error::new("joystick isn't virtual"));
        }
        Ok(())
    }

    /// Set the state of an axis on an opened virtual joystick (applied on
    /// the next joystick update). Translation of `SDL_SetJoystickVirtualAxis()`.
    pub fn set_virtual_axis(&self, axis: usize, value: i16) -> Result<()> {
        let _lock = lock_joysticks();
        self.check_virtual()?;
        virtual_joystick::set_joystick_virtual_axis_inner(self.instance_id, axis, value)
    }

    /// Generate ball motion on an opened virtual joystick.
    /// Translation of `SDL_SetJoystickVirtualBall()`.
    pub fn set_virtual_ball(&self, ball: usize, xrel: i16, yrel: i16) -> Result<()> {
        let _lock = lock_joysticks();
        self.check_virtual()?;
        virtual_joystick::set_joystick_virtual_ball_inner(self.instance_id, ball, xrel, yrel)
    }

    /// Set the state of a button on an opened virtual joystick.
    /// Translation of `SDL_SetJoystickVirtualButton()`.
    pub fn set_virtual_button(&self, button: usize, down: bool) -> Result<()> {
        let _lock = lock_joysticks();
        self.check_virtual()?;
        virtual_joystick::set_joystick_virtual_button_inner(self.instance_id, button, down)
    }

    /// Set the state of a hat on an opened virtual joystick.
    /// Translation of `SDL_SetJoystickVirtualHat()`.
    pub fn set_virtual_hat(&self, hat: usize, value: u8) -> Result<()> {
        let _lock = lock_joysticks();
        self.check_virtual()?;
        virtual_joystick::set_joystick_virtual_hat_inner(self.instance_id, hat, value)
    }

    /// Set touchpad finger state on an opened virtual joystick.
    /// Translation of `SDL_SetJoystickVirtualTouchpad()`.
    #[allow(clippy::too_many_arguments)]
    pub fn set_virtual_touchpad(
        &self,
        touchpad: usize,
        finger: usize,
        down: bool,
        x: f32,
        y: f32,
        pressure: f32,
    ) -> Result<()> {
        let _lock = lock_joysticks();
        self.check_virtual()?;
        virtual_joystick::set_joystick_virtual_touchpad_inner(
            self.instance_id,
            touchpad,
            finger,
            down,
            x,
            y,
            pressure,
        )
    }

    /// Send a sensor update for an opened virtual joystick.
    /// Translation of `SDL_SendJoystickVirtualSensorData()`.
    pub fn send_virtual_sensor_data(
        &self,
        sensor_type: SensorType,
        sensor_timestamp: u64,
        data: &[f32],
    ) -> Result<()> {
        let _lock = lock_joysticks();
        self.check_virtual()?;
        virtual_joystick::send_joystick_virtual_sensor_data_inner(
            self.instance_id,
            sensor_type,
            sensor_timestamp,
            data,
        )
    }

    /// The properties of the joystick. Translation of `SDL_GetJoystickProperties()`.
    pub fn properties(&self) -> Result<Properties> {
        self.with(|j| j.properties())
    }

    /// The implementation dependent name of the joystick.
    /// Translation of `SDL_GetJoystickName()`.
    pub fn name(&self) -> Result<Option<String>> {
        let _lock = lock_joysticks();
        let name = self.with(|j| j.name.clone())?;
        Ok(
            match joystick_virtual_gamepad_info_for_id(self.instance_id) {
                Some(info) => info.name,
                None => name,
            },
        )
    }

    /// The implementation dependent path of the joystick.
    /// Translation of `SDL_GetJoystickPath()`.
    pub fn path(&self) -> Result<String> {
        self.with(|j| j.path.clone())?
            .ok_or_else(Error::unsupported)
    }

    /// The player index of the joystick, or -1 if it's not available.
    /// Translation of `SDL_GetJoystickPlayerIndex()`.
    pub fn player_index(&self) -> i32 {
        let _lock = lock_joysticks();
        if self.with(|_| ()).is_err() {
            return -1;
        }
        player_index_for_joystick_id(self.instance_id)
    }

    /// Set the player index of the joystick (-1 to clear it).
    /// Translation of `SDL_SetJoystickPlayerIndex()`.
    pub fn set_player_index(&self, player_index: i32) -> Result<()> {
        let _lock = lock_joysticks();
        self.with(|_| ())?;
        set_joystick_id_for_player_index(player_index, self.instance_id);
        Ok(())
    }

    /// The implementation-dependent GUID of the joystick (zero for an
    /// invalid handle). Translation of `SDL_GetJoystickGUID()`.
    pub fn guid(&self) -> Guid {
        self.with(|j| j.guid).unwrap_or(Guid::ZERO)
    }

    /// The USB vendor ID of the joystick, 0 if unavailable.
    /// Translation of `SDL_GetJoystickVendor()`.
    pub fn vendor(&self) -> u16 {
        let _lock = lock_joysticks();
        match self.with(|j| j.guid) {
            Ok(guid) => vendor_and_product(self.instance_id, guid).0,
            Err(_) => 0,
        }
    }

    /// The USB product ID of the joystick, 0 if unavailable.
    /// Translation of `SDL_GetJoystickProduct()`.
    pub fn product(&self) -> u16 {
        let _lock = lock_joysticks();
        match self.with(|j| j.guid) {
            Ok(guid) => vendor_and_product(self.instance_id, guid).1,
            Err(_) => 0,
        }
    }

    /// The product version of the joystick, 0 if unavailable.
    /// Translation of `SDL_GetJoystickProductVersion()`.
    pub fn product_version(&self) -> u16 {
        joystick_guid_info(self.guid()).2
    }

    /// The firmware version of the joystick, 0 if unavailable.
    /// Translation of `SDL_GetJoystickFirmwareVersion()`.
    pub fn firmware_version(&self) -> u16 {
        self.with(|j| j.firmware_version).unwrap_or(0)
    }

    /// The serial number of the joystick, if available.
    /// Translation of `SDL_GetJoystickSerial()`.
    pub fn serial(&self) -> Result<Option<String>> {
        self.with(|j| j.serial.clone())
    }

    /// The type of the joystick. Translation of `SDL_GetJoystickType()`.
    pub fn joystick_type(&self) -> JoystickType {
        let mut joystick_type = joystick_guid_type(self.guid());
        if joystick_type == JoystickType::Unknown {
            let _lock = lock_joysticks();
            if self.with(|_| ()).is_ok() && gamepad::is_gamepad(self.instance_id) {
                joystick_type = JoystickType::Gamepad;
            }
        }
        joystick_type
    }

    /// Whether the joystick is still attached to the system.
    /// Translation of `SDL_JoystickConnected()`.
    pub fn connected(&self) -> bool {
        self.with(|j| j.attached).unwrap_or(false)
    }

    /// The instance ID of the joystick. Translation of `SDL_GetJoystickID()`.
    pub fn id(&self) -> JoystickID {
        self.instance_id
    }

    /// The number of general axis controls on the joystick.
    /// Translation of `SDL_GetNumJoystickAxes()`.
    pub fn num_axes(&self) -> Result<usize> {
        self.with(|j| j.axes.len())
    }

    /// The number of trackballs on the joystick.
    /// Translation of `SDL_GetNumJoystickBalls()`.
    pub fn num_balls(&self) -> Result<usize> {
        self.with(|j| j.balls.len())
    }

    /// The number of POV hats on the joystick.
    /// Translation of `SDL_GetNumJoystickHats()`.
    pub fn num_hats(&self) -> Result<usize> {
        self.with(|j| j.hats.len())
    }

    /// The number of buttons on the joystick.
    /// Translation of `SDL_GetNumJoystickButtons()`.
    pub fn num_buttons(&self) -> Result<usize> {
        self.with(|j| j.buttons.len())
    }

    /// The current state of an axis control (-32768 to 32767).
    /// Translation of `SDL_GetJoystickAxis()`.
    pub fn axis(&self, axis: usize) -> Result<i16> {
        self.with(|j| match j.axes.get(axis) {
            Some(info) => Ok(info.value),
            None => Err(Error::new(format!(
                "Joystick only has {} axes",
                j.axes.len()
            ))),
        })?
    }

    /// The initial state of an axis control: `Ok(Some(state))` if the axis
    /// has an initial value. Translation of `SDL_GetJoystickAxisInitialState()`.
    pub fn axis_initial_state(&self, axis: usize) -> Result<Option<i16>> {
        self.with(|j| match j.axes.get(axis) {
            Some(info) => Ok(info.has_initial_value.then_some(info.initial_value)),
            None => Err(Error::new(format!(
                "Joystick only has {} axes",
                j.axes.len()
            ))),
        })?
    }

    /// The ball axis change `(dx, dy)` since the last poll (which this
    /// resets). Translation of `SDL_GetJoystickBall()`.
    pub fn ball(&self, ball: usize) -> Result<(i32, i32)> {
        self.with(|j| {
            let nballs = j.balls.len();
            match j.balls.get_mut(ball) {
                Some(data) => Ok((std::mem::take(&mut data.dx), std::mem::take(&mut data.dy))),
                None => Err(Error::new(format!("Joystick only has {nballs} balls"))),
            }
        })?
    }

    /// The current state of a POV hat (`HAT_*` bits).
    /// Translation of `SDL_GetJoystickHat()`.
    pub fn hat(&self, hat: usize) -> Result<u8> {
        self.with(|j| match j.hats.get(hat) {
            Some(&state) => Ok(state),
            None => Err(Error::new(format!(
                "Joystick only has {} hats",
                j.hats.len()
            ))),
        })?
    }

    /// The current state of a button. Translation of `SDL_GetJoystickButton()`.
    pub fn button(&self, button: usize) -> Result<bool> {
        self.with(|j| match j.buttons.get(button) {
            Some(&down) => Ok(down),
            None => Err(Error::new(format!(
                "Joystick only has {} buttons",
                j.buttons.len()
            ))),
        })?
    }

    /// Whether the joystick has a particular sensor.
    /// Translation of `SDL_JoystickHasSensor()`.
    pub fn has_sensor(&self, sensor_type: SensorType) -> bool {
        self.with(|j| j.sensors.iter().any(|s| s.sensor_type == sensor_type))
            .unwrap_or(false)
    }

    /// Set whether data reporting for a joystick sensor is enabled.
    /// Translation of `SDL_SetJoystickSensorEnabled()`.
    pub fn set_sensor_enabled(&self, sensor_type: SensorType, enabled: bool) -> Result<()> {
        let _lock = lock_joysticks();

        let (index, already, accel_sensor, gyro_sensor, nsensors_enabled, driver) =
            self.with(|j| {
                let index = j.sensors.iter().position(|s| s.sensor_type == sensor_type);
                (
                    index,
                    index.is_some_and(|i| j.sensors[i].enabled == enabled),
                    j.accel_sensor,
                    j.gyro_sensor,
                    j.nsensors_enabled,
                    j.driver,
                )
            })?;
        let Some(index) = index else {
            return Err(error_no_such_sensor());
        };
        if already {
            return Ok(());
        }

        if sensor_type == SensorType::Accel && accel_sensor != 0 {
            if enabled {
                let sensor = Sensor::open(accel_sensor)?;
                self.with(|j| j.accel = Some(sensor))?;
            } else {
                let sensor = self.with(|j| j.accel.take())?;
                drop(sensor);
            }
        } else if sensor_type == SensorType::Gyro && gyro_sensor != 0 {
            if enabled {
                let sensor = Sensor::open(gyro_sensor)?;
                self.with(|j| j.gyro = Some(sensor))?;
            } else {
                let sensor = self.with(|j| j.gyro.take())?;
                drop(sensor);
            }
        } else if enabled {
            if nsensors_enabled == 0 {
                JOYSTICK_DRIVERS[driver].set_sensors_enabled(self.instance_id, true)?;
            }
            self.with(|j| j.nsensors_enabled += 1)?;
        } else {
            if nsensors_enabled == 1 {
                JOYSTICK_DRIVERS[driver].set_sensors_enabled(self.instance_id, false)?;
            }
            self.with(|j| j.nsensors_enabled -= 1)?;
        }

        self.with(|j| {
            if let Some(sensor) = j.sensors.get_mut(index) {
                sensor.enabled = enabled;
            }
        })
    }

    /// Whether sensor data reporting is enabled for a joystick sensor.
    /// Translation of `SDL_JoystickSensorEnabled()`.
    pub fn sensor_enabled(&self, sensor_type: SensorType) -> bool {
        self.with(|j| {
            j.sensors
                .iter()
                .find(|s| s.sensor_type == sensor_type)
                .is_some_and(|s| s.enabled)
        })
        .unwrap_or(false)
    }

    /// The data rate (number of events per second) of a joystick sensor,
    /// or 0.0 if unavailable. Translation of `SDL_GetJoystickSensorDataRate()`.
    pub fn sensor_data_rate(&self, sensor_type: SensorType) -> f32 {
        self.with(|j| {
            j.sensors
                .iter()
                .find(|s| s.sensor_type == sensor_type)
                .map_or(0.0, |s| s.rate)
        })
        .unwrap_or(0.0)
    }

    /// The current state of a joystick sensor: as many values as `data`
    /// holds, up to the sensor's 3. Translation of `SDL_GetJoystickSensorData()`.
    pub fn sensor_data(&self, sensor_type: SensorType, data: &mut [f32]) -> Result<()> {
        self.with(
            |j| match j.sensors.iter().find(|s| s.sensor_type == sensor_type) {
                Some(sensor) => {
                    let num_values = data.len().min(sensor.data.len());
                    data[..num_values].copy_from_slice(&sensor.data[..num_values]);
                    Ok(())
                }
                None => Err(error_no_such_sensor()),
            },
        )?
    }

    /// Start a rumble effect: each call cancels any previous rumble effect,
    /// and calling it with 0 intensity stops any rumbling. The duration is
    /// capped at 65535 ms; 0 rumbles until changed.
    /// Translation of `SDL_RumbleJoystick()`.
    pub fn rumble(
        &self,
        low_frequency_rumble: u16,
        high_frequency_rumble: u16,
        duration_ms: u32,
    ) -> Result<()> {
        let _lock = lock_joysticks();
        self.with(|_| ())?;
        rumble_joystick(
            self.instance_id,
            low_frequency_rumble,
            high_frequency_rumble,
            duration_ms,
        )
    }

    /// Start a rumble effect in the joystick's triggers.
    /// Translation of `SDL_RumbleJoystickTriggers()`.
    pub fn rumble_triggers(
        &self,
        left_rumble: u16,
        right_rumble: u16,
        duration_ms: u32,
    ) -> Result<()> {
        let _lock = lock_joysticks();
        self.with(|_| ())?;
        rumble_joystick_triggers(self.instance_id, left_rumble, right_rumble, duration_ms)
    }

    /// Update the joystick's LED color. Translation of `SDL_SetJoystickLED()`.
    pub fn set_led(&self, red: u8, green: u8, blue: u8) -> Result<()> {
        let _lock = lock_joysticks();

        let (isfreshvalue, expired, driver) = self.with(|j| {
            (
                red != j.led_red || green != j.led_green || blue != j.led_blue,
                timer::ticks_ms() >= j.led_expiration,
                j.driver,
            )
        })?;

        let result = if isfreshvalue || expired {
            let result = JOYSTICK_DRIVERS[driver].set_led(self.instance_id, red, green, blue);
            let _ = self.with(|j| j.led_expiration = timer::ticks_ms() + LED_MIN_REPEAT_MS);
            result
        } else {
            // Avoid spamming the driver
            Ok(())
        };

        // Save the LED value regardless of success, so we don't spam the driver
        let _ = self.with(|j| {
            j.led_red = red;
            j.led_green = green;
            j.led_blue = blue;
        });

        result
    }

    /// Send a joystick specific effect packet.
    /// Translation of `SDL_SendJoystickEffect()`.
    pub fn send_effect(&self, data: &[u8]) -> Result<()> {
        let _lock = lock_joysticks();
        let driver = self.with(|j| j.driver)?;
        JOYSTICK_DRIVERS[driver].send_effect(self.instance_id, data)
    }

    /// The connection state of the joystick.
    /// Translation of `SDL_GetJoystickConnectionState()`.
    pub fn connection_state(&self) -> JoystickConnectionState {
        self.with(|j| j.connection_state)
            .unwrap_or(JoystickConnectionState::Invalid)
    }

    /// The battery state of the joystick and its charge (0 to 100, or -1
    /// if unknown). Translation of `SDL_GetJoystickPowerInfo()`.
    pub fn power_info(&self) -> (PowerState, i32) {
        self.with(|j| (j.battery_state, j.battery_percent))
            .unwrap_or((PowerState::Error, -1))
    }
}

impl Drop for Joystick {
    fn drop(&mut self) {
        close_joystick(self.instance_id, self.serial);
    }
}

fn error_no_such_sensor() -> Error {
    Error::new("No such sensor on this device")
}

/// Attach a new virtual joystick, returning its instance ID.
/// Translation of `SDL_AttachVirtualJoystick()`.
pub fn attach_virtual_joystick(desc: VirtualJoystickDesc) -> Result<JoystickID> {
    let _lock = lock_joysticks();
    virtual_joystick::joystick_attach_virtual_inner(desc)
}

/// Detach a virtual joystick. Translation of `SDL_DetachVirtualJoystick()`.
pub fn detach_virtual_joystick(instance_id: JoystickID) -> Result<()> {
    let _lock = lock_joysticks();
    virtual_joystick::joystick_detach_virtual_inner(instance_id)
}

/// Whether a joystick is virtual. Translation of `SDL_IsJoystickVirtual()`.
pub fn is_joystick_virtual(instance_id: JoystickID) -> bool {
    let _lock = lock_joysticks();
    matches!(
        driver_and_joystick_index(instance_id),
        Ok((VIRTUAL_DRIVER_INDEX, _))
    )
}

/// The autodetected gamepad mapping of a joystick, from its driver.
/// Translation of `SDL_PrivateJoystickGetAutoGamepadMapping()`.
pub(crate) fn private_joystick_get_auto_gamepad_mapping(
    instance_id: JoystickID,
) -> Option<GamepadMapping> {
    let _lock = lock_joysticks();
    let (driver, device_index) = driver_and_joystick_index(instance_id).ok()?;
    JOYSTICK_DRIVERS[driver].gamepad_mapping(device_index)
}

/// Translation of `SDL_RumbleJoystick()` on a joystick known to be open.
fn rumble_joystick(
    instance_id: JoystickID,
    low_frequency_rumble: u16,
    high_frequency_rumble: u16,
    duration_ms: u32,
) -> Result<()> {
    let Some((same, driver)) = with_joystick(instance_id, |j| {
        (
            low_frequency_rumble == j.low_frequency_rumble
                && high_frequency_rumble == j.high_frequency_rumble,
            j.driver,
        )
    }) else {
        return Err(Error::invalid_param("joystick"));
    };

    let result = if same {
        // Just update the expiration
        Ok(())
    } else {
        let result = JOYSTICK_DRIVERS[driver].rumble(
            instance_id,
            low_frequency_rumble,
            high_frequency_rumble,
        );
        with_joystick(instance_id, |j| {
            if result.is_ok() {
                j.rumble_resend = timer::ticks_ms() + RUMBLE_RESEND_MS;
                if j.rumble_resend == 0 {
                    j.rumble_resend = 1;
                }
            } else {
                j.rumble_resend = 0;
            }
        });
        result
    };

    if result.is_ok() {
        with_joystick(instance_id, |j| {
            j.low_frequency_rumble = low_frequency_rumble;
            j.high_frequency_rumble = high_frequency_rumble;

            if (low_frequency_rumble != 0 || high_frequency_rumble != 0) && duration_ms != 0 {
                j.rumble_expiration =
                    timer::ticks_ms() + u64::from(duration_ms.min(MAX_RUMBLE_DURATION_MS));
                if j.rumble_expiration == 0 {
                    j.rumble_expiration = 1;
                }
            } else {
                j.rumble_expiration = 0;
                j.rumble_resend = 0;
            }
        });
    }

    result
}

/// Translation of `SDL_RumbleJoystickTriggers()` on a joystick known to be open.
fn rumble_joystick_triggers(
    instance_id: JoystickID,
    left_rumble: u16,
    right_rumble: u16,
    duration_ms: u32,
) -> Result<()> {
    let Some((same, driver)) = with_joystick(instance_id, |j| {
        (
            left_rumble == j.left_trigger_rumble && right_rumble == j.right_trigger_rumble,
            j.driver,
        )
    }) else {
        return Err(Error::invalid_param("joystick"));
    };

    let result = if same {
        // Just update the expiration
        Ok(())
    } else {
        let result =
            JOYSTICK_DRIVERS[driver].rumble_triggers(instance_id, left_rumble, right_rumble);
        with_joystick(instance_id, |j| {
            if result.is_ok() {
                j.trigger_rumble_resend = timer::ticks_ms() + RUMBLE_RESEND_MS;
                if j.trigger_rumble_resend == 0 {
                    j.trigger_rumble_resend = 1;
                }
            } else {
                j.trigger_rumble_resend = 0;
            }
        });
        result
    };

    if result.is_ok() {
        with_joystick(instance_id, |j| {
            j.left_trigger_rumble = left_rumble;
            j.right_trigger_rumble = right_rumble;

            if (left_rumble != 0 || right_rumble != 0) && duration_ms != 0 {
                j.trigger_rumble_expiration =
                    timer::ticks_ms() + u64::from(duration_ms.min(MAX_RUMBLE_DURATION_MS));
            } else {
                j.trigger_rumble_expiration = 0;
                j.trigger_rumble_resend = 0;
            }
        });
    }

    result
}

/// Close a joystick previously opened with [`Joystick::open`].
/// Translation of `SDL_CloseJoystick()`.
fn close_joystick(instance_id: JoystickID, serial: u64) {
    let _lock = lock_joysticks();

    // First decrement ref count
    let remaining = with_state(|s| {
        s.joysticks
            .iter_mut()
            .find(|j| j.instance_id == instance_id && j.open_serial == serial)
            .map(|j| {
                j.ref_count -= 1;
                j.ref_count
            })
    });
    match remaining {
        Some(n) if n <= 0 => {}
        _ => return,
    }

    let (props, rumbling, trigger_rumbling) = with_joystick(instance_id, |j| {
        (
            j.props.take(),
            j.rumble_expiration != 0,
            j.trigger_rumble_expiration != 0,
        )
    })
    .unwrap_or_default();
    drop(props);

    if rumbling {
        let _ = rumble_joystick(instance_id, 0, 0, 0);
    }
    if trigger_rumbling {
        let _ = rumble_joystick_triggers(instance_id, 0, 0, 0);
    }

    cleanup_sensor_fusion(instance_id);

    // (unlink this entry)
    let Some(mut joystick) = with_state(|s| {
        let i = s
            .joysticks
            .iter()
            .position(|j| j.instance_id == instance_id && j.open_serial == serial)?;
        Some(s.joysticks.remove(i))
    }) else {
        return;
    };

    JOYSTICK_DRIVERS[joystick.driver].close(&mut joystick);
    joystick.hwdata = None;

    // Free the data associated with this joystick
    drop(joystick);
}

/// Translation of `SDL_QuitJoysticks()`.
pub(crate) fn quit_joysticks() {
    let _lock = lock_joysticks();

    JOYSTICKS_QUITTING.store(true, Ordering::Relaxed);

    for id in joysticks() {
        private_joystick_removed(id);
    }

    while let Some((id, serial)) = with_state(|s| {
        s.joysticks.first_mut().map(|j| {
            j.ref_count = 1;
            (j.instance_id, j.open_serial)
        })
    }) {
        close_joystick(id, serial);
    }

    // Quit drivers in reverse order to avoid breaking dependencies between drivers
    for driver in JOYSTICK_DRIVERS.iter().rev() {
        driver.quit();
    }

    with_state(|s| s.players = Vec::new());

    init::quit_subsystem(InitFlags::EVENTS);

    steam_virtual_gamepad::quit_steam_virtual_gamepad_info();

    let watch = ALLOW_BACKGROUND_EVENTS_WATCH
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take();
    drop(watch);

    for list in &VIDPID_LISTS {
        list.free();
    }

    gamepad::quit_gamepad_mappings();

    with_state(|s| s.names = None);

    JOYSTICKS_QUITTING.store(false, Ordering::Relaxed);
    JOYSTICKS_INITIALIZED.store(false, Ordering::Relaxed);
}

/// Translation of `SDL_PrivateJoystickShouldIgnoreEvent()`.
fn private_joystick_should_ignore_event() -> bool {
    if JOYSTICK_ALLOWS_BACKGROUND_EVENTS.load(Ordering::Relaxed) {
        return false;
    }

    let has_windows = crate::events::window::video().is_some_and(|v| v.has_windows());
    if has_windows && crate::events::keyboard::keyboard_focus().is_none() {
        // We have windows but we don't have focus, ignore the event.
        return true;
    }
    false
}

fn push(event: Event) {
    let _ = queue::push(event);
}

/// Report a newly connected joystick. Called by drivers with the joystick
/// lock held. Translation of `SDL_PrivateJoystickAdded()`.
pub(crate) fn private_joystick_added(instance_id: JoystickID) {
    assert_joysticks_locked();

    if joysticks_quitting() {
        return;
    }

    JOYSTICK_BEING_ADDED.store(true, Ordering::Relaxed);

    let mut player_index = -1;
    if let Ok((driver, device_index)) = driver_and_joystick_index(instance_id) {
        let driver = JOYSTICK_DRIVERS[driver];
        player_index = driver.device_steam_virtual_gamepad_slot(device_index);
        if player_index < 0 {
            player_index = driver.device_player_index(device_index);
        }
    }
    if player_index < 0 && gamepad::is_gamepad(instance_id) {
        player_index = find_free_player_index();
    }
    if player_index >= 0 {
        set_joystick_id_for_player_index(player_index, instance_id);
    }

    let _ = update_joystick_name_for_id(instance_id);

    if queue::event_enabled(EventType::JOYSTICK_ADDED) {
        push(Event::JoyDevice(JoyDeviceEvent {
            event_type: EventType::JOYSTICK_ADDED,
            timestamp: Duration::ZERO,
            which: instance_id,
        }));
    }

    // This might create an automatic gamepad mapping, so wait to send the event
    let is_gamepad = gamepad::is_gamepad(instance_id);

    JOYSTICK_BEING_ADDED.store(false, Ordering::Relaxed);

    if is_gamepad {
        gamepad::private_gamepad_added(instance_id);
    }
}

/// Translation of `SDL_IsJoystickBeingAdded()`.
pub(crate) fn is_joystick_being_added() -> bool {
    JOYSTICK_BEING_ADDED.load(Ordering::Relaxed)
}

/// Tell the app that everything is centered/unpressed.
/// Translation of `SDL_PrivateJoystickForceRecentering()`.
pub(crate) fn private_joystick_force_recentering(instance_id: JoystickID) {
    let timestamp = timer::ticks_ns();

    assert_joysticks_locked();

    let Some((axes, nbuttons, nhats, touchpads)) = with_joystick(instance_id, |j| {
        let axes: Vec<(usize, i16)> = j
            .axes
            .iter()
            .enumerate()
            .filter(|(_, a)| a.has_initial_value)
            .map(|(i, a)| (i, a.zero))
            .collect();
        let touchpads: Vec<usize> = j.touchpads.iter().map(|t| t.fingers.len()).collect();
        (axes, j.buttons.len(), j.hats.len(), touchpads)
    }) else {
        return;
    };

    // Tell the app that everything is centered/unpressed...
    for (i, zero) in axes {
        send_joystick_axis(timestamp, instance_id, i as u8, zero);
    }

    for i in 0..nbuttons {
        send_joystick_button(timestamp, instance_id, i as u8, false);
    }

    for i in 0..nhats {
        send_joystick_hat(timestamp, instance_id, i as u8, HAT_CENTERED);
    }

    for (i, nfingers) in touchpads.into_iter().enumerate() {
        for j in 0..nfingers {
            send_joystick_touchpad(
                timestamp,
                instance_id,
                i as i32,
                j as i32,
                false,
                0.0,
                0.0,
                0.0,
            );
        }
    }
}

/// Report a disconnected joystick. Called by drivers with the joystick
/// lock held. Translation of `SDL_PrivateJoystickRemoved()`.
pub(crate) fn private_joystick_removed(instance_id: JoystickID) {
    assert_joysticks_locked();

    // Find this joystick...
    if with_joystick(instance_id, |_| ()).is_some() {
        private_joystick_force_recentering(instance_id);
        with_joystick(instance_id, |j| j.attached = false);
    }

    if gamepad::is_gamepad(instance_id) {
        gamepad::private_gamepad_removed(instance_id);
    }

    if queue::event_enabled(EventType::JOYSTICK_REMOVED) {
        push(Event::JoyDevice(JoyDeviceEvent {
            event_type: EventType::JOYSTICK_REMOVED,
            timestamp: Duration::ZERO,
            which: instance_id,
        }));
    }

    let player_index = player_index_for_joystick_id(instance_id);
    if player_index >= 0 {
        with_state(|s| s.players[player_index as usize] = 0);
    }
}

/// Deliver a new axis value. Translation of `SDL_SendJoystickAxis()`.
pub(crate) fn send_joystick_axis(timestamp: u64, joystick: JoystickID, axis: u8, value: i16) {
    enum Step {
        Drop,
        SendInitial(i16),
        Post,
    }

    assert_joysticks_locked();

    // Make sure we're not getting garbage or duplicate events
    let step = with_joystick(joystick, |j| {
        let is_virtual = is_joystick_virtual_guid(j.guid);
        let Some(info) = j.axes.get_mut(usize::from(axis)) else {
            return Step::Drop;
        };
        if !info.has_initial_value
            || (!info.has_second_value
                && (info.initial_value <= -32767 || info.initial_value == 32767)
                && i32::from(value).abs() < i32::from(JOYSTICK_AXIS_MAX) / 4)
        {
            info.initial_value = value;
            info.value = value;
            info.zero = value;
            info.has_initial_value = true;
        } else if value == info.value && !info.sending_initial_value {
            return Step::Drop;
        } else {
            info.has_second_value = true;
        }
        if !info.sent_initial_value {
            // Make sure we don't send motion until there's real activity on this axis
            const MAX_ALLOWED_JITTER: i32 = JOYSTICK_AXIS_MAX as i32 / 80; // ShanWan PS3 controller needed 96
            if (i32::from(value) - i32::from(info.value)).abs() <= MAX_ALLOWED_JITTER && !is_virtual
            {
                return Step::Drop;
            }
            info.sent_initial_value = true;
            info.sending_initial_value = true;
            return Step::SendInitial(info.initial_value);
        }
        Step::Post
    })
    .unwrap_or(Step::Drop);

    match step {
        Step::Drop => return,
        Step::SendInitial(initial_value) => {
            send_joystick_axis(timestamp, joystick, axis, initial_value);
            with_joystick(joystick, |j| {
                j.axes[usize::from(axis)].sending_initial_value = false
            });
        }
        Step::Post => {}
    }

    /* We ignore events if we don't have keyboard focus, except for centering
     * events.
     */
    let should_ignore = private_joystick_should_ignore_event();
    let post = with_joystick(joystick, |j| {
        let info = &mut j.axes[usize::from(axis)];
        if should_ignore
            && (info.sending_initial_value
                || (value > info.zero && value >= info.value)
                || (value < info.zero && value <= info.value))
        {
            return false;
        }

        // Update internal joystick state
        crate::sdl_assert!(timestamp != 0);
        info.value = value;
        j.update_complete = timestamp;
        true
    })
    .unwrap_or(false);

    // Post the event, if desired
    if post && queue::event_enabled(EventType::JOYSTICK_AXIS_MOTION) {
        push(Event::JoyAxis(JoyAxisEvent {
            timestamp: Duration::from_nanos(timestamp),
            which: joystick,
            axis,
            value,
        }));
    }
}

/// Deliver trackball motion. Translation of `SDL_SendJoystickBall()`.
pub(crate) fn send_joystick_ball(
    timestamp: u64,
    joystick: JoystickID,
    ball: u8,
    xrel: i16,
    yrel: i16,
) {
    assert_joysticks_locked();

    // Make sure we're not getting garbage events
    if with_joystick(joystick, |j| usize::from(ball) < j.balls.len()) != Some(true) {
        return;
    }

    // We ignore events if we don't have keyboard focus.
    if private_joystick_should_ignore_event() {
        return;
    }

    // Update internal mouse state
    with_joystick(joystick, |j| {
        let data = &mut j.balls[usize::from(ball)];
        data.dx += i32::from(xrel);
        data.dy += i32::from(yrel);
    });

    // Post the event, if desired
    if queue::event_enabled(EventType::JOYSTICK_BALL_MOTION) {
        push(Event::JoyBall(JoyBallEvent {
            timestamp: Duration::from_nanos(timestamp),
            which: joystick,
            ball,
            xrel,
            yrel,
        }));
    }
}

/// Deliver a new hat position. Translation of `SDL_SendJoystickHat()`.
pub(crate) fn send_joystick_hat(timestamp: u64, joystick: JoystickID, hat: u8, value: u8) {
    assert_joysticks_locked();

    // Make sure we're not getting garbage or duplicate events
    match with_joystick(joystick, |j| j.hats.get(usize::from(hat)).copied()) {
        Some(Some(current)) if current != value => {}
        _ => return,
    }

    /* We ignore events if we don't have keyboard focus, except for centering
     * events.
     */
    if private_joystick_should_ignore_event() && value != HAT_CENTERED {
        return;
    }

    // Update internal joystick state
    crate::sdl_assert!(timestamp != 0);
    with_joystick(joystick, |j| {
        j.hats[usize::from(hat)] = value;
        j.update_complete = timestamp;
    });

    // Post the event, if desired
    if queue::event_enabled(EventType::JOYSTICK_HAT_MOTION) {
        push(Event::JoyHat(JoyHatEvent {
            timestamp: Duration::from_nanos(timestamp),
            which: joystick,
            hat,
            value,
        }));
    }
}

/// Deliver a button press or release. Translation of `SDL_SendJoystickButton()`.
pub(crate) fn send_joystick_button(timestamp: u64, joystick: JoystickID, button: u8, down: bool) {
    assert_joysticks_locked();

    let event_type = if down {
        EventType::JOYSTICK_BUTTON_DOWN
    } else {
        EventType::JOYSTICK_BUTTON_UP
    };

    let Some(swap_face_buttons) = with_joystick(joystick, |j| j.swap_face_buttons) else {
        return;
    };
    let button = if swap_face_buttons {
        match button {
            0 => 1,
            1 => 0,
            2 => 3,
            3 => 2,
            other => other,
        }
    } else {
        button
    };

    let current =
        with_joystick(joystick, |j| j.buttons.get(usize::from(button)).copied()).flatten();

    // Make sure we're not getting garbage or duplicate events
    match current {
        Some(current) if current != down => {}
        _ => return,
    }

    /* We ignore events if we don't have keyboard focus, except for button
     * release. */
    if private_joystick_should_ignore_event() && down {
        return;
    }

    // Update internal joystick state
    crate::sdl_assert!(timestamp != 0);
    with_joystick(joystick, |j| {
        j.buttons[usize::from(button)] = down;
        j.update_complete = timestamp;
    });

    // Post the event, if desired
    if queue::event_enabled(event_type) {
        push(Event::JoyButton(JoyButtonEvent {
            timestamp: Duration::from_nanos(timestamp),
            which: joystick,
            button,
            down,
        }));
    }
}

/// Translation of `SendSteamHandleUpdateEvents()`.
fn send_steam_handle_update_events() {
    assert_joysticks_locked();

    // Check to see if any Steam handles changed
    let ids: Vec<JoystickID> = with_state(|s| s.joysticks.iter().map(|j| j.instance_id).collect());
    for id in ids {
        if !gamepad::is_gamepad(id) {
            continue;
        }

        let info = joystick_virtual_gamepad_info_for_id(id);
        let changed = with_joystick(id, |j| {
            let mut changed = false;
            if let Some(info) = &info {
                if j.steam_handle != info.handle {
                    j.steam_handle = info.handle;
                    j.swap_face_buttons = should_swap_face_buttons(info);
                    changed = true;
                }
            } else if j.steam_handle != 0 {
                j.steam_handle = 0;
                j.swap_face_buttons = false;
                changed = true;
            }
            changed
        })
        .unwrap_or(false);
        if changed {
            push(Event::GamepadDevice(GamepadDeviceEvent {
                event_type: EventType::GAMEPAD_STEAM_HANDLE_UPDATED,
                timestamp: Duration::ZERO,
                which: id,
            }));
        }
    }
}

/// Poll the joystick drivers for new input and changes in the connected
/// devices (done by the event loop unless `SDL_HINT_AUTO_UPDATE_JOYSTICKS`
/// is off). Translation of `SDL_UpdateJoysticks()`.
pub fn update_joysticks() {
    if !joysticks_initialized() {
        return;
    }

    let _lock = lock_joysticks();

    if steam_virtual_gamepad::update_steam_virtual_gamepad_info() {
        send_steam_handle_update_events();
    }

    // (HIDAPI_UpdateDevices() arrives with the HIDAPI driver)

    let open: Vec<(JoystickID, usize)> = with_state(|s| {
        s.joysticks
            .iter()
            .filter(|j| j.attached)
            .map(|j| (j.instance_id, j.driver))
            .collect()
    });
    for (id, driver) in open {
        if with_joystick(id, |j| j.attached) != Some(true) {
            continue;
        }

        JOYSTICK_DRIVERS[driver].update(id);

        if with_joystick(id, |j| j.delayed_guide_button) == Some(true) {
            gamepad::gamepad_handle_delayed_guide_button(id);
        }

        let now = timer::ticks_ms();
        let Some((rumble_expiration, trigger_rumble_expiration)) =
            with_joystick(id, |j| (j.rumble_expiration, j.trigger_rumble_expiration))
        else {
            continue;
        };
        if rumble_expiration != 0 && now >= rumble_expiration {
            let _ = rumble_joystick(id, 0, 0, 0);
            with_joystick(id, |j| j.rumble_resend = 0);
        }

        if let Some((low, high)) = with_joystick(id, |j| {
            (j.rumble_resend != 0 && now >= j.rumble_resend)
                .then_some((j.low_frequency_rumble, j.high_frequency_rumble))
        })
        .flatten()
        {
            let _ = JOYSTICK_DRIVERS[driver].rumble(id, low, high);
            with_joystick(id, |j| {
                j.rumble_resend = now + RUMBLE_RESEND_MS;
                if j.rumble_resend == 0 {
                    j.rumble_resend = 1;
                }
            });
        }

        if trigger_rumble_expiration != 0 && now >= trigger_rumble_expiration {
            let _ = rumble_joystick_triggers(id, 0, 0, 0);
            with_joystick(id, |j| j.trigger_rumble_resend = 0);
        }

        if let Some((left, right)) = with_joystick(id, |j| {
            (j.trigger_rumble_resend != 0 && now >= j.trigger_rumble_resend)
                .then_some((j.left_trigger_rumble, j.right_trigger_rumble))
        })
        .flatten()
        {
            let _ = JOYSTICK_DRIVERS[driver].rumble_triggers(id, left, right);
            with_joystick(id, |j| {
                j.trigger_rumble_resend = now + RUMBLE_RESEND_MS;
                if j.trigger_rumble_resend == 0 {
                    j.trigger_rumble_resend = 1;
                }
            });
        }
    }

    if queue::event_enabled(EventType::JOYSTICK_UPDATE_COMPLETE) {
        let completed: Vec<(JoystickID, u64)> = with_state(|s| {
            s.joysticks
                .iter_mut()
                .filter(|j| j.update_complete != 0)
                .map(|j| (j.instance_id, std::mem::take(&mut j.update_complete)))
                .collect()
        });
        for (id, update_complete) in completed {
            push(Event::JoyDevice(JoyDeviceEvent {
                event_type: EventType::JOYSTICK_UPDATE_COMPLETE,
                timestamp: Duration::from_nanos(update_complete),
                which: id,
            }));
        }
    }

    /* this needs to happen AFTER walking the joystick list above, so that any
      dangling hardware data from removed devices can be free'd
    */
    for driver in JOYSTICK_DRIVERS {
        driver.detect();
    }
}

/// Translation of `SDL_joystick_event_list`.
const JOYSTICK_EVENT_LIST: [EventType; 8] = [
    EventType::JOYSTICK_AXIS_MOTION,
    EventType::JOYSTICK_BALL_MOTION,
    EventType::JOYSTICK_HAT_MOTION,
    EventType::JOYSTICK_BUTTON_DOWN,
    EventType::JOYSTICK_BUTTON_UP,
    EventType::JOYSTICK_ADDED,
    EventType::JOYSTICK_REMOVED,
    EventType::JOYSTICK_BATTERY_UPDATED,
];

/// Enable or disable the joystick events. Translation of `SDL_SetJoystickEventsEnabled()`.
pub fn set_joystick_events_enabled(enabled: bool) {
    for event_type in JOYSTICK_EVENT_LIST {
        queue::set_event_enabled(event_type, enabled);
    }
}

/// Whether any joystick events are enabled. Translation of `SDL_JoystickEventsEnabled()`.
pub fn joystick_events_enabled() -> bool {
    JOYSTICK_EVENT_LIST.iter().any(|&t| queue::event_enabled(t))
}

/// The implementation-dependent GUID of a joystick (zero if not found).
/// Translation of `SDL_GetJoystickGUIDForID()`.
pub fn joystick_guid_for_id(instance_id: JoystickID) -> Guid {
    let _lock = lock_joysticks();
    match driver_and_joystick_index(instance_id) {
        Ok((driver, device_index)) => JOYSTICK_DRIVERS[driver].device_guid(device_index),
        Err(_) => Guid::ZERO,
    }
}

/// The USB vendor ID of a joystick, 0 if unavailable.
/// Translation of `SDL_GetJoystickVendorForID()`.
pub fn joystick_vendor_for_id(instance_id: JoystickID) -> u16 {
    let _lock = lock_joysticks();
    match joystick_virtual_gamepad_info_for_id(instance_id) {
        Some(info) => info.vendor_id,
        None => joystick_guid_info(joystick_guid_for_id(instance_id)).0,
    }
}

/// The USB product ID of a joystick, 0 if unavailable.
/// Translation of `SDL_GetJoystickProductForID()`.
pub fn joystick_product_for_id(instance_id: JoystickID) -> u16 {
    let _lock = lock_joysticks();
    match joystick_virtual_gamepad_info_for_id(instance_id) {
        Some(info) => info.product_id,
        None => joystick_guid_info(joystick_guid_for_id(instance_id)).1,
    }
}

/// The product version of a joystick, 0 if unavailable.
/// Translation of `SDL_GetJoystickProductVersionForID()`.
pub fn joystick_product_version_for_id(instance_id: JoystickID) -> u16 {
    joystick_guid_info(joystick_guid_for_id(instance_id)).2
}

/// The type of a joystick. Translation of `SDL_GetJoystickTypeForID()`.
pub fn joystick_type_for_id(instance_id: JoystickID) -> JoystickType {
    let mut joystick_type = joystick_guid_type(joystick_guid_for_id(instance_id));
    if joystick_type == JoystickType::Unknown && gamepad::is_gamepad(instance_id) {
        joystick_type = JoystickType::Gamepad;
    }
    joystick_type
}

/// Deliver a battery update. Translation of `SDL_SendJoystickPowerInfo()`.
#[allow(dead_code)] // (used by the platform drivers)
pub(crate) fn send_joystick_power_info(joystick: JoystickID, state: PowerState, percent: i32) {
    assert_joysticks_locked();

    let changed = with_joystick(joystick, |j| {
        if state != j.battery_state || percent != j.battery_percent {
            j.battery_state = state;
            j.battery_percent = percent;
            true
        } else {
            false
        }
    })
    .unwrap_or(false);

    if changed && queue::event_enabled(EventType::JOYSTICK_BATTERY_UPDATED) {
        push(Event::JoyBattery(JoyBatteryEvent {
            timestamp: Duration::ZERO,
            which: joystick,
            state,
            percent,
        }));
    }
}

/// Deliver a touchpad finger update. Translation of `SDL_SendJoystickTouchpad()`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn send_joystick_touchpad(
    timestamp: u64,
    joystick: JoystickID,
    touchpad: i32,
    finger: i32,
    down: bool,
    mut x: f32,
    mut y: f32,
    mut pressure: f32,
) {
    assert_joysticks_locked();

    let Some(Some(finger_info)) = with_joystick(joystick, |j| {
        let touchpad_info = j.touchpads.get(usize::try_from(touchpad).ok()?)?;
        touchpad_info
            .fingers
            .get(usize::try_from(finger).ok()?)
            .copied()
    }) else {
        return;
    };

    if !down {
        if x == 0.0 && y == 0.0 {
            x = finger_info.x;
            y = finger_info.y;
        }
        pressure = 0.0;
    }

    x = x.clamp(0.0, 1.0);
    y = y.clamp(0.0, 1.0);
    pressure = pressure.clamp(0.0, 1.0);

    if down == finger_info.down
        && (!down || (x == finger_info.x && y == finger_info.y && pressure == finger_info.pressure))
    {
        return;
    }

    let event_type = if down == finger_info.down {
        EventType::GAMEPAD_TOUCHPAD_MOTION
    } else if down {
        EventType::GAMEPAD_TOUCHPAD_DOWN
    } else {
        EventType::GAMEPAD_TOUCHPAD_UP
    };

    // We ignore events if we don't have keyboard focus, except for touch release
    if private_joystick_should_ignore_event() && event_type != EventType::GAMEPAD_TOUCHPAD_UP {
        return;
    }

    // Update internal joystick state
    crate::sdl_assert!(timestamp != 0);
    with_joystick(joystick, |j| {
        let info = &mut j.touchpads[touchpad as usize].fingers[finger as usize];
        info.down = down;
        info.x = x;
        info.y = y;
        info.pressure = pressure;
        j.update_complete = timestamp;
    });

    // Post the event, if desired
    if queue::event_enabled(event_type) {
        push(Event::GamepadTouchpad(GamepadTouchpadEvent {
            event_type,
            timestamp: Duration::from_nanos(timestamp),
            which: joystick,
            touchpad,
            finger,
            x,
            y,
            pressure,
        }));
    }
}

/// Deliver sensor data. Translation of `SDL_SendJoystickSensor()`.
pub(crate) fn send_joystick_sensor(
    timestamp: u64,
    joystick: JoystickID,
    sensor_type: SensorType,
    sensor_timestamp: u64,
    data: &[f32],
) {
    assert_joysticks_locked();

    // We ignore events if we don't have keyboard focus
    if private_joystick_should_ignore_event() {
        return;
    }

    let updated = with_joystick(joystick, |j| {
        let sensor = j
            .sensors
            .iter_mut()
            .find(|s| s.sensor_type == sensor_type)?;
        if !sensor.enabled {
            return None;
        }
        let num_values = data.len().min(sensor.data.len());

        // Update internal sensor state
        sensor.data[..num_values].copy_from_slice(&data[..num_values]);
        j.update_complete = timestamp;
        Some(num_values)
    })
    .flatten();

    // Post the event, if desired
    if let Some(num_values) = updated {
        if queue::event_enabled(EventType::GAMEPAD_SENSOR_UPDATE) {
            let mut event = GamepadSensorEvent {
                timestamp: Duration::from_nanos(timestamp),
                which: joystick,
                sensor: sensor_type as i32,
                data: [0.0; 3],
                sensor_timestamp,
            };
            let num_values = num_values.min(event.data.len());
            event.data[..num_values].copy_from_slice(&data[..num_values]);
            push(Event::GamepadSensor(event));
        }
    }
}

/// Deliver a capacitive sensor touch or release.
/// Translation of `SDL_SendJoystickCapSense()`.
#[allow(dead_code)] // (used by the HIDAPI drivers)
pub(crate) fn send_joystick_capsense(
    timestamp: u64,
    joystick: JoystickID,
    capsense_type: GamepadCapSenseType,
    down: bool,
) {
    assert_joysticks_locked();

    // We ignore events if we don't have keyboard focus, except for button
    // (capsense) release
    if private_joystick_should_ignore_event() && down {
        return;
    }

    let changed = with_joystick(joystick, |j| {
        let capsense = j
            .capsenses
            .iter_mut()
            .find(|c| c.capsense_type == capsense_type)?;

        // Ignore duplicate events
        if down == capsense.down {
            return None;
        }

        // Update internal joystick state
        capsense.down = down;
        j.update_complete = timestamp;
        Some(())
    })
    .flatten();

    if changed.is_some() {
        let event_type = if down {
            EventType::GAMEPAD_CAPSENSE_TOUCH
        } else {
            EventType::GAMEPAD_CAPSENSE_RELEASE
        };

        // Post the event, if desired
        if queue::event_enabled(event_type) {
            push(Event::GamepadCapSense(GamepadCapSenseEvent {
                timestamp: Duration::from_nanos(timestamp),
                which: joystick,
                capsense: capsense_type as u8,
                down,
            }));
        }
    }
}

#[cfg(test)]
mod tests;
