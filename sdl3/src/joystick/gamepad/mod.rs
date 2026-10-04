// Rust translation of src/joystick/SDL_gamepad.c, SDL_gamepad_c.h and
// include/SDL3/SDL_gamepad.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Gamepads: joysticks with a standard layout of buttons and axes.
//!
//! A gamepad is a joystick for which SDL knows a *mapping*: a string such as
//! `"030000005e0400008e02000000007801,XInput Controller,a:b0,b:b1,..."`
//! binding joystick buttons, axes and hats to the named inputs of an Xbox
//! style controller (positional names: `a` is the bottom face button, and
//! so on). SDL ships a database of mappings and can generate them for
//! devices whose driver knows their layout; applications may add more
//! ([`add_gamepad_mapping`], `SDL_GAMECONTROLLERCONFIG`).
//!
//! A [`Gamepad`] is an open gamepad; it is closed when the last handle is
//! dropped.

mod generated;

use std::cell::RefCell;
use std::cmp::Ordering as CmpOrdering;
use std::collections::HashMap;
use std::mem::ManuallyDrop;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use super::vidpid::VidPidList;
use super::{
    assert_joysticks_locked, is_joystick_being_added, is_joystick_gamecube, is_joystick_hidapi,
    is_joystick_rawinput, is_joystick_steam_virtual_gamepad, is_joystick_wgi, joystick_guid_for_id,
    joystick_guid_info, joystick_guid_uses_version, joystick_name_for_id,
    joystick_virtual_gamepad_info_for_id, joysticks, lock_joysticks,
    private_joystick_get_auto_gamepad_mapping, send_joystick_sensor, set_joystick_guid_crc,
    set_joystick_guid_version, with_joystick, Joystick, JoystickConnectionState, HAT_DOWN,
    HAT_LEFT, HAT_RIGHT, HAT_UP, JOYSTICK_AXIS_MAX, JOYSTICK_AXIS_MIN,
};
use crate::error::{Error, Result};
use crate::events::queue::{self, EventWatch};
use crate::events::{
    Event, EventType, GamepadAxisEvent, GamepadButtonEvent, GamepadDeviceEvent, JoystickID,
    SensorID,
};
use crate::guid::Guid;
use crate::hints;
use crate::io::IoStream;
use crate::power::PowerState;
use crate::properties::Properties;
use crate::sensor::SensorType;
use crate::stdlib::string::{strcasecmp, strncasecmp};
use crate::thread::ReentrantMutex;
use crate::timer;

pub use super::{
    PROP_JOYSTICK_CAP_MONO_LED_BOOLEAN as PROP_GAMEPAD_CAP_MONO_LED_BOOLEAN,
    PROP_JOYSTICK_CAP_PLAYER_LED_BOOLEAN as PROP_GAMEPAD_CAP_PLAYER_LED_BOOLEAN,
    PROP_JOYSTICK_CAP_RGB_LED_BOOLEAN as PROP_GAMEPAD_CAP_RGB_LED_BOOLEAN,
    PROP_JOYSTICK_CAP_RUMBLE_BOOLEAN as PROP_GAMEPAD_CAP_RUMBLE_BOOLEAN,
    PROP_JOYSTICK_CAP_TRIGGER_RUMBLE_BOOLEAN as PROP_GAMEPAD_CAP_TRIGGER_RUMBLE_BOOLEAN,
};

/// Many gamepads turn the center button into an instantaneous button press.
/// Translation of `SDL_MINIMUM_GUIDE_BUTTON_DELAY_MS`.
const MINIMUM_GUIDE_BUTTON_DELAY_MS: u64 = 250;

const GAMEPAD_CRC_FIELD: &str = "crc:";
const GAMEPAD_CRC_FIELD_SIZE: usize = 4; // hard-coded for speed
const GAMEPAD_TYPE_FIELD: &str = "type:";
const GAMEPAD_TYPE_FIELD_SIZE: usize = GAMEPAD_TYPE_FIELD.len();
const GAMEPAD_FACE_FIELD: &str = "face:";
const GAMEPAD_PLATFORM_FIELD: &str = "platform:";
const GAMEPAD_PLATFORM_FIELD_SIZE: usize = GAMEPAD_PLATFORM_FIELD.len();
const GAMEPAD_HINT_FIELD: &str = "hint:";
const GAMEPAD_HINT_FIELD_SIZE: usize = GAMEPAD_HINT_FIELD.len();

/// Standard gamepad types. Translation of `SDL_GamepadType`.
///
/// This type does not necessarily map to first-party controllers from
/// Microsoft/Sony/Nintendo; in many cases, third-party controllers can
/// report as these, either because they were designed for a specific
/// console, or they simply most closely match that console's controllers.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum GamepadType {
    #[default]
    Unknown,
    Standard,
    Xbox360,
    XboxOne,
    Ps3,
    Ps4,
    Ps5,
    NintendoSwitchPro,
    NintendoSwitchJoyconLeft,
    NintendoSwitchJoyconRight,
    NintendoSwitchJoyconPair,
    Gamecube,
    Steam,
}

/// Translation of `map_StringForGamepadType`.
const MAP_STRING_FOR_GAMEPAD_TYPE: [&str; GamepadType::COUNT] = [
    "unknown",
    "standard",
    "xbox360",
    "xboxone",
    "ps3",
    "ps4",
    "ps5",
    "switchpro",
    "joyconleft",
    "joyconright",
    "joyconpair",
    "gamecube",
    "steam",
];

impl GamepadType {
    /// The number of gamepad types (`SDL_GAMEPAD_TYPE_COUNT`).
    pub const COUNT: usize = 13;

    /// The types, by value.
    pub(crate) const ALL: [GamepadType; GamepadType::COUNT] = [
        GamepadType::Unknown,
        GamepadType::Standard,
        GamepadType::Xbox360,
        GamepadType::XboxOne,
        GamepadType::Ps3,
        GamepadType::Ps4,
        GamepadType::Ps5,
        GamepadType::NintendoSwitchPro,
        GamepadType::NintendoSwitchJoyconLeft,
        GamepadType::NintendoSwitchJoyconRight,
        GamepadType::NintendoSwitchJoyconPair,
        GamepadType::Gamecube,
        GamepadType::Steam,
    ];

    /// Convert a string into a gamepad type (case-insensitively, with an
    /// optional leading `+` or `-`), [`GamepadType::Unknown`] if no match.
    /// Translation of `SDL_GetGamepadTypeFromString()`.
    pub fn from_string(str: &str) -> GamepadType {
        if str.is_empty() {
            return GamepadType::Unknown;
        }

        let str = str.strip_prefix(['+', '-']).unwrap_or(str);

        MAP_STRING_FOR_GAMEPAD_TYPE
            .iter()
            .position(|s| strcasecmp(str, s) == CmpOrdering::Equal)
            .map_or(GamepadType::Unknown, |i| GamepadType::ALL[i])
    }

    /// The mapping string name of the type (`None` for
    /// [`GamepadType::Unknown`]). Translation of `SDL_GetGamepadStringForType()`.
    pub fn as_str(self) -> Option<&'static str> {
        if self != GamepadType::Unknown {
            return Some(MAP_STRING_FOR_GAMEPAD_TYPE[self as usize]);
        }
        None
    }
}

/// The list of buttons available on a gamepad. Translation of `SDL_GamepadButton`.
///
/// For controllers that use a diamond pattern for the face buttons, the
/// south/east/west/north buttons below correspond to the locations in the
/// diamond pattern. For Xbox controllers, this would be A/B/X/Y, for
/// Nintendo Switch controllers, this would be B/A/Y/X, for GameCube
/// controllers this would be A/X/B/Y, for PlayStation controllers this
/// would be Cross/Circle/Square/Triangle.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum GamepadButton {
    #[default]
    Invalid = -1,
    /// Bottom face button (e.g. Xbox A button)
    South,
    /// Right face button (e.g. Xbox B button)
    East,
    /// Left face button (e.g. Xbox X button)
    West,
    /// Top face button (e.g. Xbox Y button)
    North,
    Back,
    Guide,
    Start,
    LeftStick,
    RightStick,
    LeftShoulder,
    RightShoulder,
    DpadUp,
    DpadDown,
    DpadLeft,
    DpadRight,
    /// Additional button (e.g. Xbox Series X share button, PS5 microphone
    /// button, Nintendo Switch Pro capture button, Steam Controller QAM
    /// button, Amazon Luna microphone button, Google Stadia capture button)
    Misc1,
    /// Upper or primary paddle, under your right hand (e.g. Xbox Elite
    /// paddle P1, DualSense Edge RB button, Right Joy-Con SR button, Steam
    /// Controller R4 button)
    RightPaddle1,
    /// Upper or primary paddle, under your left hand (e.g. Xbox Elite
    /// paddle P3, DualSense Edge LB button, Left Joy-Con SL button, Steam
    /// Controller L4 button)
    LeftPaddle1,
    /// Lower or secondary paddle, under your right hand (e.g. Xbox Elite
    /// paddle P2, DualSense Edge right Fn button, Right Joy-Con SL button,
    /// Steam Controller R5 button)
    RightPaddle2,
    /// Lower or secondary paddle, under your left hand (e.g. Xbox Elite
    /// paddle P4, DualSense Edge left Fn button, Left Joy-Con SR button,
    /// Steam Controller L5 button)
    LeftPaddle2,
    /// PS4/PS5 touchpad button
    Touchpad,
    /// Additional button
    Misc2,
    /// Additional button (e.g. Nintendo GameCube left trigger click)
    Misc3,
    /// Additional button (e.g. Nintendo GameCube right trigger click)
    Misc4,
    /// Additional button
    Misc5,
    /// Additional button
    Misc6,
}

/// Translation of `map_StringForGamepadButton`.
const MAP_STRING_FOR_GAMEPAD_BUTTON: [&str; GamepadButton::COUNT] = [
    "a",
    "b",
    "x",
    "y",
    "back",
    "guide",
    "start",
    "leftstick",
    "rightstick",
    "leftshoulder",
    "rightshoulder",
    "dpup",
    "dpdown",
    "dpleft",
    "dpright",
    "misc1",
    "paddle1",
    "paddle2",
    "paddle3",
    "paddle4",
    "touchpad",
    "misc2",
    "misc3",
    "misc4",
    "misc5",
    "misc6",
];

impl GamepadButton {
    /// The number of gamepad buttons (`SDL_GAMEPAD_BUTTON_COUNT`).
    pub const COUNT: usize = 26;

    const ALL: [GamepadButton; GamepadButton::COUNT] = [
        GamepadButton::South,
        GamepadButton::East,
        GamepadButton::West,
        GamepadButton::North,
        GamepadButton::Back,
        GamepadButton::Guide,
        GamepadButton::Start,
        GamepadButton::LeftStick,
        GamepadButton::RightStick,
        GamepadButton::LeftShoulder,
        GamepadButton::RightShoulder,
        GamepadButton::DpadUp,
        GamepadButton::DpadDown,
        GamepadButton::DpadLeft,
        GamepadButton::DpadRight,
        GamepadButton::Misc1,
        GamepadButton::RightPaddle1,
        GamepadButton::LeftPaddle1,
        GamepadButton::RightPaddle2,
        GamepadButton::LeftPaddle2,
        GamepadButton::Touchpad,
        GamepadButton::Misc2,
        GamepadButton::Misc3,
        GamepadButton::Misc4,
        GamepadButton::Misc5,
        GamepadButton::Misc6,
    ];

    /// Every button, in order.
    pub fn all() -> impl Iterator<Item = GamepadButton> {
        GamepadButton::ALL.into_iter()
    }

    /// The button with a raw value, [`GamepadButton::Invalid`] if none.
    pub fn from_i32(v: i32) -> GamepadButton {
        usize::try_from(v)
            .ok()
            .and_then(|i| GamepadButton::ALL.get(i).copied())
            .unwrap_or(GamepadButton::Invalid)
    }

    /// Convert a string into a button (case-insensitively),
    /// [`GamepadButton::Invalid`] if no match.
    /// Translation of `SDL_GetGamepadButtonFromString()`.
    pub fn from_string(str: &str) -> GamepadButton {
        private_get_gamepad_button_from_string(str, false, false)
    }

    /// The mapping string name of the button.
    /// Translation of `SDL_GetGamepadStringForButton()`.
    pub fn as_str(self) -> Option<&'static str> {
        usize::try_from(self as i32)
            .ok()
            .map(|i| MAP_STRING_FOR_GAMEPAD_BUTTON[i])
    }
}

/// The set of gamepad button labels. Translation of `SDL_GamepadButtonLabel`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum GamepadButtonLabel {
    #[default]
    Unknown,
    A,
    B,
    X,
    Y,
    Cross,
    Circle,
    Square,
    Triangle,
}

/// The list of axes available on a gamepad. Translation of `SDL_GamepadAxis`.
///
/// Thumbstick axis values range from [`JOYSTICK_AXIS_MIN`] to
/// [`JOYSTICK_AXIS_MAX`], and are centered within ~8000 of zero, though
/// advanced UI will allow users to set or autodetect the dead zone, which
/// varies between gamepads. Trigger axis values range from 0 (released) to
/// [`JOYSTICK_AXIS_MAX`] (fully pressed).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum GamepadAxis {
    #[default]
    Invalid = -1,
    LeftX,
    LeftY,
    RightX,
    RightY,
    LeftTrigger,
    RightTrigger,
}

/// Translation of `map_StringForGamepadAxis`.
const MAP_STRING_FOR_GAMEPAD_AXIS: [&str; GamepadAxis::COUNT] = [
    "leftx",
    "lefty",
    "rightx",
    "righty",
    "lefttrigger",
    "righttrigger",
];

impl GamepadAxis {
    /// The number of gamepad axes (`SDL_GAMEPAD_AXIS_COUNT`).
    pub const COUNT: usize = 6;

    const ALL: [GamepadAxis; GamepadAxis::COUNT] = [
        GamepadAxis::LeftX,
        GamepadAxis::LeftY,
        GamepadAxis::RightX,
        GamepadAxis::RightY,
        GamepadAxis::LeftTrigger,
        GamepadAxis::RightTrigger,
    ];

    /// Every axis, in order.
    pub fn all() -> impl Iterator<Item = GamepadAxis> {
        GamepadAxis::ALL.into_iter()
    }

    /// Convert a string into an axis (case-insensitively, with an optional
    /// leading `+` or `-`), [`GamepadAxis::Invalid`] if no match.
    /// Translation of `SDL_GetGamepadAxisFromString()`.
    pub fn from_string(str: &str) -> GamepadAxis {
        if str.is_empty() {
            return GamepadAxis::Invalid;
        }

        let str = str.strip_prefix(['+', '-']).unwrap_or(str);

        MAP_STRING_FOR_GAMEPAD_AXIS
            .iter()
            .position(|s| strcasecmp(str, s) == CmpOrdering::Equal)
            .map_or(GamepadAxis::Invalid, |i| GamepadAxis::ALL[i])
    }

    /// The mapping string name of the axis.
    /// Translation of `SDL_GetGamepadStringForAxis()`.
    pub fn as_str(self) -> Option<&'static str> {
        usize::try_from(self as i32)
            .ok()
            .map(|i| MAP_STRING_FOR_GAMEPAD_AXIS[i])
    }
}

/// The capacitive sensors on a gamepad. Translation of `SDL_GamepadCapSenseType`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum GamepadCapSenseType {
    #[default]
    Invalid = -1,
    /// Activated by touching the top of the left thumbstick
    LeftStick,
    /// Activated by touching the top of the right thumbstick
    RightStick,
    /// Activated by gripping the left handle of the controller
    LeftGrip,
    /// Activated by gripping the right handle of the controller
    RightGrip,
}

impl GamepadCapSenseType {
    /// The number of capsense types (`SDL_GAMEPAD_CAPSENSE_COUNT`).
    pub const COUNT: usize = 4;
}

/// The joystick input of a gamepad binding (`SDL_GamepadBinding::input`).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum GamepadBindingInput {
    /// `SDL_GAMEPAD_BINDTYPE_BUTTON`
    Button(i32),
    /// `SDL_GAMEPAD_BINDTYPE_AXIS`: the axis range that is bound (`axis_min`
    /// may be greater than `axis_max` for an inverted axis)
    Axis {
        axis: i32,
        axis_min: i32,
        axis_max: i32,
    },
    /// `SDL_GAMEPAD_BINDTYPE_HAT`
    Hat { hat: i32, hat_mask: i32 },
}

/// The gamepad output of a gamepad binding (`SDL_GamepadBinding::output`).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum GamepadBindingOutput {
    /// `SDL_GAMEPAD_BINDTYPE_BUTTON`
    Button(GamepadButton),
    /// `SDL_GAMEPAD_BINDTYPE_AXIS`
    Axis {
        axis: GamepadAxis,
        axis_min: i32,
        axis_max: i32,
    },
}

/// A mapping between one joystick input and one gamepad control.
/// Translation of `SDL_GamepadBinding`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct GamepadBinding {
    pub input: GamepadBindingInput,
    pub output: GamepadBindingOutput,
}

/// The kind of a driver supplied mapping input. Translation of `EMappingKind`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum MappingKind {
    #[default]
    None,
    Button,
    Axis,
    #[allow(dead_code)] // (used by the platform drivers)
    Hat,
}

/// Translation of `SDL_InputMapping`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct InputMapping {
    pub(crate) kind: MappingKind,
    pub(crate) target: u8,
    pub(crate) axis_reversed: bool,
    pub(crate) half_axis_positive: bool,
    pub(crate) half_axis_negative: bool,
}

/// The layout of a gamepad as its driver knows it.
/// Translation of `SDL_GamepadMapping`.
#[derive(Clone, Debug, Default)]
pub(crate) struct GamepadMapping {
    pub(crate) a: InputMapping,
    pub(crate) b: InputMapping,
    pub(crate) x: InputMapping,
    pub(crate) y: InputMapping,
    pub(crate) back: InputMapping,
    pub(crate) guide: InputMapping,
    pub(crate) start: InputMapping,
    pub(crate) leftstick: InputMapping,
    pub(crate) rightstick: InputMapping,
    pub(crate) leftshoulder: InputMapping,
    pub(crate) rightshoulder: InputMapping,
    pub(crate) dpup: InputMapping,
    pub(crate) dpdown: InputMapping,
    pub(crate) dpleft: InputMapping,
    pub(crate) dpright: InputMapping,
    pub(crate) misc1: InputMapping,
    pub(crate) misc2: InputMapping,
    pub(crate) misc3: InputMapping,
    pub(crate) misc4: InputMapping,
    pub(crate) misc5: InputMapping,
    pub(crate) misc6: InputMapping,
    pub(crate) right_paddle1: InputMapping,
    pub(crate) left_paddle1: InputMapping,
    pub(crate) right_paddle2: InputMapping,
    pub(crate) left_paddle2: InputMapping,
    pub(crate) leftx: InputMapping,
    pub(crate) lefty: InputMapping,
    pub(crate) rightx: InputMapping,
    pub(crate) righty: InputMapping,
    pub(crate) lefttrigger: InputMapping,
    pub(crate) righttrigger: InputMapping,
    pub(crate) touchpad: InputMapping,
}

/// The face button style of a gamepad. Translation of `SDL_GamepadFaceStyle`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum FaceStyle {
    #[default]
    Unknown,
    Abxy,
    Axby,
    Bayx,
    Sony,
}

/// Translation of `SDL_GamepadMappingPriority`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum MappingPriority {
    Default,
    Api,
    User,
}

/// Translation of `GamepadMapping_t`; mappings are identified by `id`
/// (upstream compares pointers).
struct MappingEntry {
    id: u64,
    guid: Guid,
    name: String,
    mapping: String,
    priority: MappingPriority,
}

/// Translation of `MappingChangeTracker`.
struct MappingChangeTracker {
    refcount: i32,
    joysticks: Vec<JoystickID>,
    joystick_mappings: Option<Vec<Option<u64>>>,
    changed_mappings: Vec<Option<u64>>,
}

/// The mapping state guarded by `SDL_event_lock` upstream.
struct MappingState {
    /// Translation of `s_pSupportedGamepads`, in the order added.
    supported: Vec<MappingEntry>,
    /// Translation of `s_pDefaultMapping`.
    default_mapping: Option<u64>,
    /// Translation of `s_pXInputMapping`.
    xinput_mapping: Option<u64>,
    /// Translation of `s_mappingChangeTracker`.
    tracker: Option<MappingChangeTracker>,
    /// Translation of `s_gamepadInstanceIDs`.
    instance_ids: Option<HashMap<JoystickID, bool>>,
}

static MAPPINGS: ReentrantMutex<RefCell<MappingState>> =
    ReentrantMutex::new(RefCell::new(MappingState {
        supported: Vec::new(),
        default_mapping: None,
        xinput_mapping: None,
        tracker: None,
        instance_ids: None,
    }));

static NEXT_MAPPING_ID: AtomicU64 = AtomicU64::new(1);

fn with_mappings<R>(f: impl FnOnce(&mut MappingState) -> R) -> R {
    let guard = MAPPINGS.lock();
    let mut state = guard.borrow_mut();
    f(&mut state)
}

/// The `(guid, name, mapping)` of a mapping.
fn mapping_info(id: u64) -> Option<(Guid, String, String)> {
    with_mappings(|m| {
        m.supported
            .iter()
            .find(|e| e.id == id)
            .map(|e| (e.guid, e.name.clone(), e.mapping.clone()))
    })
}

/// The SDL gamepad structure. Translation of `struct SDL_Gamepad`.
struct GamepadData {
    /// underlying joystick device
    joystick: Joystick,
    instance_id: JoystickID,
    ref_count: i32,
    serial: u64,

    name: String,
    gamepad_type: GamepadType,
    face_style: FaceStyle,
    mapping: u64,
    bindings: Vec<GamepadBinding>,
    last_match_axis: Vec<Option<GamepadBinding>>,
    last_hat_mask: Vec<u8>,
    guide_button_down: u64,
}

/// The gamepad state guarded by `SDL_event_lock` upstream.
struct GamepadState {
    /// Translation of `SDL_gamepads`, most recently opened first.
    gamepads: Vec<GamepadData>,
    /// Translation of `SDL_gamepad_names`.
    names: Option<HashMap<JoystickID, String>>,
    /// The `SDL_GamepadEventWatcher()` registration.
    watch: Option<EventWatch>,
}

/// The `RefCell` borrow is never held across a call out of this module.
static GAMEPADS: ReentrantMutex<RefCell<GamepadState>> =
    ReentrantMutex::new(RefCell::new(GamepadState {
        gamepads: Vec::new(),
        names: None,
        watch: None,
    }));

/// Translation of `SDL_gamepads_initialized`.
static GAMEPADS_INITIALIZED: AtomicBool = AtomicBool::new(false);
static NEXT_SERIAL: AtomicU64 = AtomicU64::new(1);

fn with_gamepads<R>(f: impl FnOnce(&mut GamepadState) -> R) -> R {
    let guard = GAMEPADS.lock();
    let mut state = guard.borrow_mut();
    f(&mut state)
}

/// Run `f` on the open gamepad of a joystick.
fn with_gamepad<R>(instance_id: JoystickID, f: impl FnOnce(&mut GamepadData) -> R) -> Option<R> {
    with_gamepads(|s| {
        s.gamepads
            .iter_mut()
            .find(|g| g.instance_id == instance_id)
            .map(f)
    })
}

/// Translation of `SDL_allowed_gamepads`.
static ALLOWED_GAMEPADS: VidPidList =
    VidPidList::new(Some(hints::GAMECONTROLLER_IGNORE_DEVICES_EXCEPT), None, &[]);
/// Translation of `SDL_ignored_gamepads`.
static IGNORED_GAMEPADS: VidPidList =
    VidPidList::new(Some(hints::GAMECONTROLLER_IGNORE_DEVICES), None, &[]);

/// Where a word in a gamepad name must be for the name to be blacklisted.
/// Translation of `SDL_GamepadBlacklistWordsPosition`.
#[derive(Clone, Copy)]
enum BlacklistPosition {
    Begin,
    End,
    Anywhere,
}

/// List of words in gamepad names that indicate that the gamepad should not
/// be detected. See also `initial_blacklist_devices` in SDL_joystick.c.
/// Translation of `SDL_gamepad_blacklist_words`.
static GAMEPAD_BLACKLIST_WORDS: &[(&str, BlacklistPosition)] = &[
    #[cfg(target_os = "linux")]
    (" Motion Sensors", BlacklistPosition::End), // Don't treat the PS3 and PS4 motion controls as a separate gamepad
    #[cfg(target_os = "linux")]
    (" IMU", BlacklistPosition::End), // Don't treat the Nintendo IMU as a separate gamepad
    #[cfg(target_os = "linux")]
    (" Touchpad", BlacklistPosition::End), // "Sony Interactive Entertainment DualSense Wireless Controller Touchpad"
    // Don't treat the Wii extension controls as a separate gamepad
    #[cfg(target_os = "linux")]
    (" Accelerometer", BlacklistPosition::End),
    #[cfg(target_os = "linux")]
    (" IR", BlacklistPosition::End),
    #[cfg(target_os = "linux")]
    (" Motion Plus", BlacklistPosition::End),
    #[cfg(target_os = "linux")]
    (" Nunchuk", BlacklistPosition::End),
    // The Google Pixel fingerprint sensor, as well as other fingerprint sensors, reports itself as a joystick
    ("uinput-", BlacklistPosition::Begin),
    // The IR receiver on the NVIDIA Shield TV
    ("gpio_ir_recv", BlacklistPosition::Begin),
    ("Synaptics ", BlacklistPosition::Anywhere), // "Synaptics TM2768-001", "SynPS/2 Synaptics TouchPad"
    ("Trackpad", BlacklistPosition::Anywhere),
    ("Clickpad", BlacklistPosition::Anywhere),
    // "Usb Keyboard Usb Keyboard Consumer Control", "Framework Laptop 16 Keyboard Module - ISO System Control"
    (" Keyboard", BlacklistPosition::Anywhere),
    (" Laptop ", BlacklistPosition::Anywhere), // "Framework Laptop 16 Numpad Module System Control"
    ("Mouse ", BlacklistPosition::Begin),      // "Mouse passthrough"
    (" Pen", BlacklistPosition::End),          // "Wacom One by Wacom S Pen"
    (" Finger", BlacklistPosition::End),       // "Wacom HID 495F Finger"
    (" System Control", BlacklistPosition::End), // "hid-over-i2c 0107 System Control"
    (" LED ", BlacklistPosition::Anywhere),    // "ASRock LED Controller"
    (" Thelio ", BlacklistPosition::Anywhere), // "System76 Thelio Io 2"
];

/// Translation of `HasSameOutput()`.
fn has_same_output(a: &GamepadBinding, b: &GamepadBinding) -> bool {
    match (a.output, b.output) {
        (
            GamepadBindingOutput::Axis { axis: a, .. },
            GamepadBindingOutput::Axis { axis: b, .. },
        ) => a == b,
        (GamepadBindingOutput::Button(a), GamepadBindingOutput::Button(b)) => a == b,
        _ => false,
    }
}

/// Translation of `ResetOutput()`.
fn reset_output(timestamp: u64, gamepad: JoystickID, bind: &GamepadBinding) {
    match bind.output {
        GamepadBindingOutput::Axis { axis, .. } => send_gamepad_axis(timestamp, gamepad, axis, 0),
        GamepadBindingOutput::Button(button) => {
            send_gamepad_button(timestamp, gamepad, button, false)
        }
    }
}

/// Translation of `HandleJoystickAxis()`.
fn handle_joystick_axis(timestamp: u64, gamepad: JoystickID, axis: i32, mut value: i32) {
    assert_joysticks_locked();

    let Some((last_match, matched)) = with_gamepad(gamepad, |g| {
        let last_match = usize::try_from(axis)
            .ok()
            .and_then(|a| g.last_match_axis.get(a))
            .copied()
            .flatten();
        let matched = g
            .bindings
            .iter()
            .copied()
            .find(|binding| match binding.input {
                GamepadBindingInput::Axis {
                    axis: input_axis,
                    axis_min,
                    axis_max,
                } if input_axis == axis => {
                    if axis_min < axis_max {
                        value >= axis_min && value <= axis_max
                    } else {
                        value >= axis_max && value <= axis_min
                    }
                }
                _ => false,
            });
        (last_match, matched)
    }) else {
        return;
    };

    if let Some(last_match) = &last_match {
        if matched.is_none_or(|m| !has_same_output(last_match, &m)) {
            // Clear the last input that this axis generated
            reset_output(timestamp, gamepad, last_match);
        }
    }

    if let Some(matched) = &matched {
        let GamepadBindingInput::Axis {
            axis_min: input_min,
            axis_max: input_max,
            ..
        } = matched.input
        else {
            unreachable!();
        };
        match matched.output {
            GamepadBindingOutput::Axis {
                axis: output_axis,
                axis_min: output_min,
                axis_max: output_max,
            } => {
                if input_min != output_min || input_max != output_max {
                    let normalized_value =
                        (value - input_min) as f32 / (input_max - input_min) as f32;
                    value =
                        output_min + (normalized_value * (output_max - output_min) as f32) as i32;
                }
                send_gamepad_axis(timestamp, gamepad, output_axis, value as i16);
            }
            GamepadBindingOutput::Button(button) => {
                let threshold = input_min + (input_max - input_min) / 2;
                let down = if input_max < input_min {
                    value <= threshold
                } else {
                    value >= threshold
                };
                send_gamepad_button(timestamp, gamepad, button, down);
            }
        }
    }

    with_gamepad(gamepad, |g| {
        if let Some(slot) = usize::try_from(axis)
            .ok()
            .and_then(|a| g.last_match_axis.get_mut(a))
        {
            *slot = matched;
        }
    });
}

/// Translation of `HandleJoystickButton()`.
fn handle_joystick_button(timestamp: u64, gamepad: JoystickID, button: i32, down: bool) {
    assert_joysticks_locked();

    let Some(Some(binding)) = with_gamepad(gamepad, |g| {
        g.bindings
            .iter()
            .copied()
            .find(|b| b.input == GamepadBindingInput::Button(button))
    }) else {
        return;
    };

    match binding.output {
        GamepadBindingOutput::Axis {
            axis,
            axis_min,
            axis_max,
        } => {
            let value = if down { axis_max } else { axis_min };
            send_gamepad_axis(timestamp, gamepad, axis, value as i16);
        }
        GamepadBindingOutput::Button(output) => {
            send_gamepad_button(timestamp, gamepad, output, down)
        }
    }
}

/// Translation of `HandleJoystickHat()`.
fn handle_joystick_hat(timestamp: u64, gamepad: JoystickID, hat: i32, value: u8) {
    assert_joysticks_locked();

    let Some(Some((last_mask, bindings))) = with_gamepad(gamepad, |g| {
        let last_mask = *usize::try_from(hat)
            .ok()
            .and_then(|h| g.last_hat_mask.get(h))?;
        let bindings: Vec<GamepadBinding> = g
            .bindings
            .iter()
            .copied()
            .filter(|b| matches!(b.input, GamepadBindingInput::Hat { hat: h, .. } if h == hat))
            .collect();
        Some((last_mask, bindings))
    }) else {
        return;
    };

    let changed_mask = last_mask ^ value;
    for binding in &bindings {
        let GamepadBindingInput::Hat { hat_mask, .. } = binding.input else {
            continue;
        };
        if i32::from(changed_mask) & hat_mask != 0 {
            if i32::from(value) & hat_mask != 0 {
                match binding.output {
                    GamepadBindingOutput::Axis { axis, axis_max, .. } => {
                        send_gamepad_axis(timestamp, gamepad, axis, axis_max as i16);
                    }
                    GamepadBindingOutput::Button(button) => {
                        send_gamepad_button(timestamp, gamepad, button, true)
                    }
                }
            } else {
                reset_output(timestamp, gamepad, binding);
            }
        }
    }

    with_gamepad(gamepad, |g| {
        if let Some(mask) = usize::try_from(hat)
            .ok()
            .and_then(|h| g.last_hat_mask.get_mut(h))
        {
            *mask = value;
        }
    });
}

/* The joystick layer will _also_ send events to recenter before disconnect,
but it has to make (sometimes incorrect) guesses at what being "centered"
is. The gamepad layer, however, can set a definite logical idle
position, so set them all here. If we happened to already be at the
center thanks to the joystick layer or idle hands, this won't generate
duplicate events. */
/// Translation of `RecenterGamepad()`.
fn recenter_gamepad(gamepad: JoystickID) {
    let timestamp = timer::ticks_ns();

    for button in GamepadButton::all() {
        if gamepad_button_state(gamepad, button) {
            send_gamepad_button(timestamp, gamepad, button, false);
        }
    }

    for axis in GamepadAxis::all() {
        if gamepad_axis_state(gamepad, axis) != 0 {
            send_gamepad_axis(timestamp, gamepad, axis, 0);
        }
    }
}

/// The name of a gamepad, remembered after the device goes away.
/// Translation of `SDL_UpdateGamepadNameForID()`.
fn update_gamepad_name_for_id(instance_id: JoystickID) -> Result<Option<String>> {
    assert_joysticks_locked();

    let current_name = private_get_gamepad_mapping(instance_id, true).and_then(|mapping| {
        let (_, name, _) = mapping_info(mapping)?;
        if name == "*" {
            joystick_name_for_id(instance_id).ok().flatten()
        } else {
            Some(name)
        }
    });

    with_gamepads(|s| {
        let Some(names) = &mut s.names else {
            return Ok(current_name);
        };

        let name = names.get(&instance_id);
        let Some(current_name) = current_name else {
            return match name {
                Some(name) => Ok(Some(name.clone())),
                None => Err(Error::new(format!("Gamepad {instance_id} not found"))),
            };
        };

        if name != Some(&current_name) {
            names.insert(instance_id, current_name.clone());
        }
        Ok(Some(current_name))
    })
}

fn push_device_event(event_type: EventType, timestamp: Duration, which: JoystickID) {
    let _ = queue::push(Event::GamepadDevice(GamepadDeviceEvent {
        event_type,
        timestamp,
        which,
    }));
}

/// Translation of `SDL_PrivateGamepadAdded()`.
pub(crate) fn private_gamepad_added(instance_id: JoystickID) {
    if !GAMEPADS_INITIALIZED.load(Ordering::Relaxed) || is_joystick_being_added() {
        return;
    }

    let _ = update_gamepad_name_for_id(instance_id);

    push_device_event(EventType::GAMEPAD_ADDED, Duration::ZERO, instance_id);
}

/// Translation of `SDL_PrivateGamepadRemoved()`.
pub(crate) fn private_gamepad_removed(instance_id: JoystickID) {
    assert_joysticks_locked();

    if !GAMEPADS_INITIALIZED.load(Ordering::Relaxed) {
        return;
    }

    if with_gamepad(instance_id, |_| ()).is_some() {
        recenter_gamepad(instance_id);
    }

    push_device_event(EventType::GAMEPAD_REMOVED, Duration::ZERO, instance_id);
}

/// Translation of `SDL_PrivateGamepadRemapped()`.
fn private_gamepad_remapped(instance_id: JoystickID) {
    if !GAMEPADS_INITIALIZED.load(Ordering::Relaxed) || is_joystick_being_added() {
        return;
    }

    push_device_event(EventType::GAMEPAD_REMAPPED, Duration::ZERO, instance_id);
}

fn nanos(timestamp: Duration) -> u64 {
    timestamp.as_nanos() as u64
}

/// Event filter to fire gamepad events from joystick ones.
/// Translation of `SDL_GamepadEventWatcher()`.
fn gamepad_event_watcher(event: &Event) {
    let has_gamepad = |id: JoystickID| with_gamepad(id, |_| ()).is_some();

    match event {
        Event::JoyAxis(e) => {
            let _lock = lock_joysticks();
            if has_gamepad(e.which) {
                handle_joystick_axis(
                    nanos(e.timestamp),
                    e.which,
                    i32::from(e.axis),
                    i32::from(e.value),
                );
            }
        }
        Event::JoyButton(e) => {
            let _lock = lock_joysticks();
            if has_gamepad(e.which) {
                handle_joystick_button(nanos(e.timestamp), e.which, i32::from(e.button), e.down);
            }
        }
        Event::JoyHat(e) => {
            let _lock = lock_joysticks();
            if has_gamepad(e.which) {
                handle_joystick_hat(nanos(e.timestamp), e.which, i32::from(e.hat), e.value);
            }
        }
        Event::JoyDevice(e) if e.event_type == EventType::JOYSTICK_UPDATE_COMPLETE => {
            let _lock = lock_joysticks();
            if queue::event_enabled(EventType::GAMEPAD_UPDATE_COMPLETE) && has_gamepad(e.which) {
                push_device_event(EventType::GAMEPAD_UPDATE_COMPLETE, e.timestamp, e.which);
            }
        }
        _ => {}
    }
}

#[allow(dead_code)] // (reached from the sensor drivers)
/// SDL defines sensor orientation relative to the device natural
/// orientation, so when it's changed orientation to be used as a gamepad,
/// change the sensor orientation to match.
/// Translation of `AdjustSensorOrientation()`.
fn adjust_sensor_orientation(transform: &[[f32; 3]; 3], src: &[f32]) -> [f32; 3] {
    let mut dst = [0.0; 3];
    for (i, row) in transform.iter().enumerate() {
        for (j, factor) in row.iter().enumerate() {
            dst[i] += factor * src.get(j).copied().unwrap_or(0.0);
        }
    }
    dst
}

/// Deliver system sensor data to the gamepads using the sensor (sensor
/// fusion). Translation of `SDL_GamepadSensorWatcher()`.
///
/// We don't use the gamepad event watcher for this because we want to
/// deliver gamepad sensor events when system sensor events are disabled,
/// and we also need to avoid a potential deadlock where joystick event
/// delivery locks the joysticks and then the event queue, but sensor event
/// delivery would lock the event queue and then from within the event
/// watcher function lock the joysticks.
#[allow(dead_code)] // (reached from the sensor drivers)
pub(crate) fn gamepad_sensor_watcher(
    timestamp: Duration,
    sensor: SensorID,
    sensor_timestamp: u64,
    data: &[f32],
) {
    let _lock = lock_joysticks();

    let ids: Vec<JoystickID> =
        with_gamepads(|s| s.gamepads.iter().map(|g| g.instance_id).collect());
    for id in ids {
        let Some((accel, gyro)) = with_joystick(id, |j| {
            let accel = (j.accel.is_some() && j.accel_sensor == sensor)
                .then(|| adjust_sensor_orientation(&j.sensor_transform, data));
            let gyro = (j.gyro.is_some() && j.gyro_sensor == sensor)
                .then(|| adjust_sensor_orientation(&j.sensor_transform, data));
            (accel, gyro)
        }) else {
            continue;
        };
        if let Some(gamepad_data) = accel {
            send_joystick_sensor(
                nanos(timestamp),
                id,
                SensorType::Accel,
                sensor_timestamp,
                &gamepad_data,
            );
        }
        if let Some(gamepad_data) = gyro {
            send_joystick_sensor(
                nanos(timestamp),
                id,
                SensorType::Gyro,
                sensor_timestamp,
                &gamepad_data,
            );
        }
    }
}

/// Translation of `PushMappingChangeTracking()`.
fn push_mapping_change_tracking() {
    assert_joysticks_locked();

    let nested = with_mappings(|m| match &mut m.tracker {
        Some(tracker) => {
            tracker.refcount += 1;
            true
        }
        None => false,
    });
    if nested {
        return;
    }

    // Save the list of joysticks and associated mappings
    let joysticks = joysticks();
    let num_joysticks = joysticks.len();
    with_mappings(|m| {
        m.tracker = Some(MappingChangeTracker {
            refcount: 1,
            joysticks: joysticks.clone(),
            joystick_mappings: None,
            changed_mappings: Vec::new(),
        });
    });
    if num_joysticks == 0 {
        return;
    }
    let joystick_mappings: Vec<Option<u64>> = joysticks
        .iter()
        .map(|&id| private_get_gamepad_mapping(id, false))
        .collect();
    with_mappings(|m| {
        if let Some(tracker) = &mut m.tracker {
            tracker.joystick_mappings = Some(joystick_mappings);
        }
    });
}

/// Translation of `AddMappingChangeTracking()`.
fn add_mapping_change_tracking(mapping: Option<u64>) {
    assert_joysticks_locked();

    with_mappings(|m| {
        crate::sdl_assert!(m.tracker.is_some());
        if let Some(tracker) = &mut m.tracker {
            tracker.changed_mappings.push(mapping);
        }
    });
}

/// Translation of `PopMappingChangeTracking()`.
fn pop_mapping_change_tracking() {
    assert_joysticks_locked();

    let tracker = with_mappings(|m| {
        crate::sdl_assert!(m.tracker.is_some());
        let tracker = m.tracker.as_mut()?;
        tracker.refcount -= 1;
        if tracker.refcount > 0 {
            return None;
        }
        m.tracker.take()
    });
    let Some(tracker) = tracker else {
        return;
    };

    // Now check to see what gamepads changed because of the mapping changes
    let Some(joystick_mappings) = &tracker.joystick_mappings else {
        return;
    };
    for (i, &joystick) in tracker.joysticks.iter().enumerate() {
        // Looking up the new mapping might create one and associate it with the gamepad (and generate events)
        let gamepad_mapping = with_gamepad(joystick, |g| g.mapping);
        let new_mapping = private_get_gamepad_mapping(joystick, false);
        let old_mapping = match gamepad_mapping {
            Some(mapping) => Some(mapping),
            None => joystick_mappings[i],
        };

        if new_mapping.is_some() && old_mapping.is_none() {
            with_mappings(|m| {
                if let Some(ids) = &mut m.instance_ids {
                    ids.insert(joystick, true);
                }
            });
            private_gamepad_added(joystick);
        } else if old_mapping.is_some() && new_mapping.is_none() {
            with_mappings(|m| {
                if let Some(ids) = &mut m.instance_ids {
                    ids.insert(joystick, false);
                }
            });
            private_gamepad_removed(joystick);
        } else if old_mapping != new_mapping || tracker.changed_mappings.contains(&new_mapping) {
            if let (Some(_), Some(new_mapping)) = (gamepad_mapping, new_mapping) {
                private_load_button_mapping(joystick, new_mapping);
            }
            private_gamepad_remapped(joystick);
        }
    }
}

/// Scan the mappings database for a gamepad with the specified GUID.
/// Translation of `SDL_PrivateMatchGamepadMappingForGUID()`.
fn private_match_gamepad_mapping_for_guid(
    mut guid: Guid,
    match_version: bool,
    exact_match_crc: bool,
) -> Option<u64> {
    assert_joysticks_locked();

    let (_, _, _, crc) = joystick_guid_info(guid);

    // Clear the CRC from the GUID for matching, the mappings never include it in the GUID
    set_joystick_guid_crc(&mut guid, 0);

    if !match_version {
        set_joystick_guid_version(&mut guid, 0);
    }

    with_mappings(|m| {
        let mut best_match = None;
        for mapping in &m.supported {
            if mapping.guid == Guid::ZERO {
                continue;
            }

            let mut mapping_guid = mapping.guid;
            if !match_version {
                set_joystick_guid_version(&mut mapping_guid, 0);
            }

            if guid == mapping_guid {
                if let Some(pos) = mapping.mapping.find(GAMEPAD_CRC_FIELD) {
                    let mapping_crc = crate::stdlib::string::strtol(
                        &mapping.mapping[pos + GAMEPAD_CRC_FIELD_SIZE..],
                        16,
                    )
                    .0 as u16;
                    if mapping_crc != crc {
                        // This mapping specified a CRC and they don't match
                        continue;
                    }

                    // An exact match, including CRC
                    return Some(mapping.id);
                } else if crc != 0 && exact_match_crc {
                    continue;
                }

                if best_match.is_none() {
                    best_match = Some(mapping.id);
                }
            }
        }
        best_match
    })
}

/// Scan the mappings database for a gamepad with the specified GUID,
/// generating a mapping for drivers that know their layout.
/// Translation of `SDL_PrivateGetGamepadMappingForGUID()`.
fn private_get_gamepad_mapping_for_guid(guid: Guid, adding_mapping: bool) -> Option<u64> {
    // Try first with an exact match on version and CRC
    if let Some(mapping) = private_match_gamepad_mapping_for_guid(guid, true, true) {
        return Some(mapping);
    }

    if adding_mapping {
        // We didn't find an existing mapping
        return None;
    }

    // Try without CRC match
    if let Some(mapping) = private_match_gamepad_mapping_for_guid(guid, true, false) {
        return Some(mapping);
    }

    // Try without version match
    if joystick_guid_uses_version(guid) {
        if let Some(mapping) = private_match_gamepad_mapping_for_guid(guid, false, true) {
            return Some(mapping);
        }

        if let Some(mapping) = private_match_gamepad_mapping_for_guid(guid, false, false) {
            return Some(mapping);
        }
    }

    #[cfg(target_os = "windows")]
    if super::is_joystick_xinput(guid) {
        // This is an XInput device
        return with_mappings(|m| m.xinput_mapping);
    }

    if is_joystick_hidapi(guid) {
        generated::create_mapping_for_hidapi_gamepad(guid)
    } else if is_joystick_rawinput(guid) {
        generated::create_mapping_for_rawinput_gamepad(guid)
    } else if is_joystick_wgi(guid) {
        generated::create_mapping_for_wgi_gamepad(guid)
    } else {
        // (Virtual joysticks pick up a robust mapping in the driver's
        // gamepad_mapping(); SDL_CreateMappingForAndroidGamepad() arrives with
        // the Android driver.)
        None
    }
}

/// Translation of `SDL_PrivateGetGamepadButtonFromString()`.
fn private_get_gamepad_button_from_string(str: &str, axby: bool, baxy: bool) -> GamepadButton {
    if str.is_empty() {
        return GamepadButton::Invalid;
    }

    let Some(i) = MAP_STRING_FOR_GAMEPAD_BUTTON
        .iter()
        .position(|s| strcasecmp(str, s) == CmpOrdering::Equal)
    else {
        return GamepadButton::Invalid;
    };
    let button = GamepadButton::ALL[i];
    if axby {
        // Need to swap face buttons
        match button {
            GamepadButton::East => return GamepadButton::West,
            GamepadButton::West => return GamepadButton::East,
            _ => {}
        }
    } else if baxy {
        // Need to swap face buttons
        match button {
            GamepadButton::South => return GamepadButton::East,
            GamepadButton::East => return GamepadButton::South,
            GamepadButton::West => return GamepadButton::North,
            GamepadButton::North => return GamepadButton::West,
            _ => {}
        }
    }
    button
}

/// `SDL_atoi()`.
fn atoi(s: &str) -> i32 {
    crate::stdlib::string::strtol(s, 10).0 as i32
}

/// Given a gamepad button name and a joystick name, add a binding.
/// Translation of `SDL_PrivateParseGamepadElement()`.
fn private_parse_gamepad_element(
    bindings: &mut Vec<GamepadBinding>,
    mapping: &str,
    game_button: &str,
    joystick_button: &str,
) -> bool {
    let mut game_button = game_button;
    let mut joystick_button = joystick_button;
    let mut half_axis_input = None;
    let mut half_axis_output = None;

    if let Some(rest) = game_button.strip_prefix(['+', '-']) {
        half_axis_output = game_button.chars().next();
        game_button = rest;
    }

    let axby_mapping = mapping.contains(",hint:SDL_GAMECONTROLLER_USE_GAMECUBE_LABELS:=1");
    let baxy_mapping = mapping.contains(",hint:SDL_GAMECONTROLLER_USE_BUTTON_LABELS:=1");

    // FIXME: We fix these up when loading the mapping, does this ever get hit?
    //SDL_assert(!axby_mapping && !baxy_mapping);

    let axis = GamepadAxis::from_string(game_button);
    let button = private_get_gamepad_button_from_string(game_button, axby_mapping, baxy_mapping);
    let output = if axis != GamepadAxis::Invalid {
        let is_trigger = axis == GamepadAxis::LeftTrigger || axis == GamepadAxis::RightTrigger;
        let (axis_min, axis_max) = if is_trigger || half_axis_output == Some('+') {
            (0, i32::from(JOYSTICK_AXIS_MAX))
        } else if half_axis_output == Some('-') {
            (0, i32::from(JOYSTICK_AXIS_MIN))
        } else {
            (i32::from(JOYSTICK_AXIS_MIN), i32::from(JOYSTICK_AXIS_MAX))
        };
        GamepadBindingOutput::Axis {
            axis,
            axis_min,
            axis_max,
        }
    } else if button != GamepadButton::Invalid {
        GamepadBindingOutput::Button(button)
    } else {
        return false;
    };

    if let Some(rest) = joystick_button.strip_prefix(['+', '-']) {
        half_axis_input = joystick_button.chars().next();
        joystick_button = rest;
    }
    // Note (upstream): an empty joystick input reads the byte before the
    // string; `ends_with()` doesn't, with the same result.
    let invert_input = joystick_button.ends_with('~');

    let bytes = joystick_button.as_bytes();
    let digit = |i: usize| bytes.get(i).is_some_and(u8::is_ascii_digit);
    let input = if bytes.first() == Some(&b'a') && digit(1) {
        let axis = atoi(&joystick_button[1..]);
        let (mut axis_min, mut axis_max) = if half_axis_input == Some('+') {
            (0, i32::from(JOYSTICK_AXIS_MAX))
        } else if half_axis_input == Some('-') {
            (0, i32::from(JOYSTICK_AXIS_MIN))
        } else {
            (i32::from(JOYSTICK_AXIS_MIN), i32::from(JOYSTICK_AXIS_MAX))
        };
        if invert_input {
            std::mem::swap(&mut axis_min, &mut axis_max);
        }
        GamepadBindingInput::Axis {
            axis,
            axis_min,
            axis_max,
        }
    } else if bytes.first() == Some(&b'b') && digit(1) {
        GamepadBindingInput::Button(atoi(&joystick_button[1..]))
    } else if bytes.first() == Some(&b'h') && digit(1) && bytes.get(2) == Some(&b'.') && digit(3) {
        let hat = atoi(&joystick_button[1..]);
        let mask = atoi(&joystick_button[3..]);
        GamepadBindingInput::Hat {
            hat,
            hat_mask: mask,
        }
    } else {
        return false;
    };

    let bind = GamepadBinding { input, output };
    if bindings.contains(&bind) {
        // We already have this binding, could be different face button names?
        return true;
    }

    bindings.push(bind);
    true
}

/// Parse the bindings of a mapping string (everything after the GUID and
/// name). Translation of `SDL_PrivateParseGamepadConfigString()`; a
/// too-long element stops the parse with an error, keeping the bindings
/// before it.
fn private_parse_gamepad_config_string(
    bindings: &mut Vec<GamepadBinding>,
    mapping: &str,
) -> Result<()> {
    let mut game_button = [0u8; 20];
    let mut joystick_button = [0u8; 128];
    let mut is_game_button = true;
    let mut i = 0;

    // (the buffers are C strings: an element ends at the first NUL)
    let cstr = |buf: &[u8]| {
        String::from_utf8_lossy(&buf[..buf.iter().position(|&b| b == 0).unwrap_or(buf.len())])
            .into_owned()
    };
    let element =
        |bindings: &mut Vec<GamepadBinding>, game_button: &[u8], joystick_button: &[u8]| {
            private_parse_gamepad_element(
                bindings,
                mapping,
                &cstr(game_button),
                &cstr(joystick_button),
            );
        };

    for &c in mapping.as_bytes() {
        if c == b':' {
            i = 0;
            is_game_button = false;
        } else if c == b' ' {
        } else if c == b',' {
            i = 0;
            is_game_button = true;
            element(bindings, &game_button, &joystick_button);
            game_button = [0; 20];
            joystick_button = [0; 128];
        } else if is_game_button {
            if i >= game_button.len() {
                game_button[game_button.len() - 1] = 0;
                return Err(Error::new(format!(
                    "Button name too large: {}",
                    cstr(&game_button)
                )));
            }
            game_button[i] = c;
            i += 1;
        } else {
            if i >= joystick_button.len() {
                joystick_button[joystick_button.len() - 1] = 0;
                return Err(Error::new(format!(
                    "Joystick button name too large: {}",
                    cstr(&joystick_button)
                )));
            }
            joystick_button[i] = c;
            i += 1;
        }
    }

    // No more values if the string was terminated by a comma. Don't report an error.
    if game_button[0] != 0 || joystick_button[0] != 0 {
        element(bindings, &game_button, &joystick_button);
    }
    Ok(())
}

/// The value of a `field:value,` field of a mapping string (to the next
/// comma, or the end if `to_end`).
fn mapping_field<'a>(mapping: &'a str, field: &str, skip: usize, to_end: bool) -> Option<&'a str> {
    let start = mapping.find(field)? + skip;
    let rest = mapping.get(start..)?;
    match rest.find(',') {
        Some(comma) => Some(&rest[..comma]),
        None if to_end => Some(rest),
        None => None,
    }
}

/// The gamepad type a mapping names, falling back to the device's.
/// Translation of `SDL_UpdateGamepadType()`.
fn gamepad_type_for_mapping(instance_id: JoystickID, mapping: &str) -> GamepadType {
    assert_joysticks_locked();

    let mut gamepad_type = GamepadType::Unknown;

    if let Some(type_string) =
        mapping_field(mapping, GAMEPAD_TYPE_FIELD, GAMEPAD_TYPE_FIELD_SIZE, true)
    {
        gamepad_type = GamepadType::from_string(type_string);
    }
    if gamepad_type == GamepadType::Unknown {
        gamepad_type = real_gamepad_type_for_id(instance_id);
    }
    gamepad_type
}

/// Translation of `SDL_GetGamepadFaceStyleFromString()`.
fn gamepad_face_style_from_string(string: &str) -> FaceStyle {
    match string {
        "abxy" => FaceStyle::Abxy,
        "axby" => FaceStyle::Axby,
        "bayx" => FaceStyle::Bayx,
        "sony" => FaceStyle::Sony,
        _ => FaceStyle::Unknown,
    }
}

/// Translation of `SDL_GetGamepadFaceStyleForGamepadType()`.
fn gamepad_face_style_for_gamepad_type(gamepad_type: GamepadType) -> FaceStyle {
    match gamepad_type {
        GamepadType::Ps3 | GamepadType::Ps4 | GamepadType::Ps5 => FaceStyle::Sony,
        GamepadType::NintendoSwitchPro
        | GamepadType::NintendoSwitchJoyconLeft
        | GamepadType::NintendoSwitchJoyconRight
        | GamepadType::NintendoSwitchJoyconPair => FaceStyle::Bayx,
        GamepadType::Gamecube => FaceStyle::Axby,
        _ => FaceStyle::Abxy,
    }
}

/// Translation of `SDL_UpdateGamepadFaceStyle()`.
fn gamepad_face_style_for_mapping(mapping: &str, gamepad_type: GamepadType) -> FaceStyle {
    assert_joysticks_locked();

    let mut face_style = FaceStyle::Unknown;

    // (upstream skips SDL_GAMEPAD_TYPE_FIELD_SIZE characters, the same 5)
    if let Some(face_string) =
        mapping_field(mapping, GAMEPAD_FACE_FIELD, GAMEPAD_TYPE_FIELD_SIZE, true)
    {
        face_style = gamepad_face_style_from_string(face_string);
    }

    if face_style == FaceStyle::Unknown
        && mapping.contains("SDL_GAMECONTROLLER_USE_GAMECUBE_LABELS")
    {
        // This controller uses GameCube button style
        face_style = FaceStyle::Axby;
    }
    if face_style == FaceStyle::Unknown && mapping.contains("SDL_GAMECONTROLLER_USE_BUTTON_LABELS")
    {
        // This controller uses Nintendo button style
        face_style = FaceStyle::Bayx;
    }
    if face_style == FaceStyle::Unknown {
        face_style = gamepad_face_style_for_gamepad_type(gamepad_type);
    }
    face_style
}

/// Translation of `SDL_FixupHIDAPIMapping()`.
fn fixup_hidapi_mapping(bindings: &mut [GamepadBinding]) {
    assert_joysticks_locked();

    // Check to see if we need fixup
    let mut need_fixup = false;
    for binding in bindings.iter() {
        if let GamepadBindingOutput::Button(output) = binding.output {
            if output as i32 >= GamepadButton::DpadUp as i32 {
                if binding.input == GamepadBindingInput::Button(output as i32) {
                    // Old style binding
                    need_fixup = true;
                }
                break;
            }
        }
    }
    if !need_fixup {
        return;
    }

    for binding in bindings.iter_mut() {
        if let (GamepadBindingInput::Button(input), GamepadBindingOutput::Button(output)) =
            (binding.input, binding.output)
        {
            let hat = |hat_mask: u8| GamepadBindingInput::Hat {
                hat: 0,
                hat_mask: i32::from(hat_mask),
            };
            match output {
                GamepadButton::DpadUp => binding.input = hat(HAT_UP),
                GamepadButton::DpadDown => binding.input = hat(HAT_DOWN),
                GamepadButton::DpadLeft => binding.input = hat(HAT_LEFT),
                GamepadButton::DpadRight => binding.input = hat(HAT_RIGHT),
                _ => {
                    if output as i32 > GamepadButton::DpadRight as i32 {
                        binding.input = GamepadBindingInput::Button(input - 4);
                    }
                }
            }
        }
    }
}

/// Make a new button mapping for a gamepad.
/// Translation of `SDL_PrivateLoadButtonMapping()`.
fn private_load_button_mapping(instance_id: JoystickID, mapping_id: u64) {
    assert_joysticks_locked();

    let Some((mapping_guid, name, mapping)) = mapping_info(mapping_id) else {
        return;
    };

    let gamepad_type = gamepad_type_for_mapping(instance_id, &mapping);
    let face_style = gamepad_face_style_for_mapping(&mapping, gamepad_type);

    let mut bindings = Vec::new();
    let _ = private_parse_gamepad_config_string(&mut bindings, &mapping);

    if is_joystick_hidapi(mapping_guid) {
        fixup_hidapi_mapping(&mut bindings);
    }

    let found = with_gamepad(instance_id, |g| {
        g.name = name;
        g.mapping = mapping_id;
        g.last_match_axis.fill(None);
        g.gamepad_type = gamepad_type;
        g.face_style = face_style;
        g.bindings = bindings.clone();
    });
    if found.is_none() {
        return;
    }

    // Set the zero point for triggers
    with_joystick(instance_id, |j| {
        for binding in &bindings {
            if let (
                GamepadBindingInput::Axis { axis, axis_min, .. },
                GamepadBindingOutput::Axis {
                    axis: GamepadAxis::LeftTrigger | GamepadAxis::RightTrigger,
                    ..
                },
            ) = (binding.input, binding.output)
            {
                if let Some(info) = usize::try_from(axis).ok().and_then(|a| j.axes.get_mut(a)) {
                    info.zero = axis_min as i16;
                    info.value = info.zero;
                }
            }
        }
    });
}

/// Grab the GUID string from a mapping string.
/// Translation of `SDL_PrivateGetGamepadGUIDFromMappingString()`.
fn private_get_gamepad_guid_from_mapping_string(mapping: &str) -> Option<String> {
    let first_comma = mapping.find(',')?;
    #[allow(unused_mut)]
    let mut guid = mapping[..first_comma].to_string();

    // Convert old style GUIDs to the new style in 2.0.5
    #[cfg(target_os = "windows")]
    if guid.len() == 32 && guid.is_ascii() && &guid[20..32] == "504944564944" {
        let mut g = guid.into_bytes();
        g[20..32].copy_from_slice(b"000000000000");
        let product = [g[4], g[5], g[6], g[7]];
        g[16..20].copy_from_slice(&product);
        let vendor = [g[0], g[1], g[2], g[3]];
        g[8..12].copy_from_slice(&vendor);
        g[0..8].copy_from_slice(b"03000000");
        guid = String::from_utf8(g).unwrap_or_default();
    }
    #[cfg(target_os = "macos")]
    if guid.len() == 32
        && guid.is_ascii()
        && &guid[4..16] == "000000000000"
        && &guid[20..32] == "000000000000"
    {
        let mut g = guid.into_bytes();
        g[20..32].copy_from_slice(b"000000000000");
        let vendor = [g[0], g[1], g[2], g[3]];
        g[8..12].copy_from_slice(&vendor);
        g[0..8].copy_from_slice(b"03000000");
        guid = String::from_utf8(g).unwrap_or_default();
    }
    Some(guid)
}

/// Grab the name string from a mapping string.
/// Translation of `SDL_PrivateGetGamepadNameFromMappingString()`.
fn private_get_gamepad_name_from_mapping_string(mapping: &str) -> Option<String> {
    let first_comma = mapping.find(',')?;
    let second_comma = first_comma + 1 + mapping[first_comma + 1..].find(',')?;
    Some(mapping[first_comma + 1..second_comma].to_string())
}

/// `SDL_isspace()`.
fn is_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r')
}

/// Grab the button mapping string from a mapping string.
/// Translation of `SDL_PrivateGetGamepadMappingFromMappingString()`.
fn private_get_gamepad_mapping_from_mapping_string(mapping: &str) -> Option<String> {
    let first_comma = mapping.find(',')?;
    let second_comma = first_comma + 1 + mapping[first_comma + 1..].find(',')?;

    // Skip whitespace; the mapping is everything after the second comma
    let rest =
        mapping[second_comma + 1..].trim_start_matches(|c: char| c.is_ascii() && is_space(c as u8));

    // Trim whitespace
    Some(
        rest.trim_end_matches(|c: char| c.is_ascii() && is_space(c as u8))
            .to_string(),
    )
}

/// Add (or update) a mapping for a GUID, returning the mapping and whether
/// it already existed. Translation of `SDL_PrivateAddMappingForGUID()`.
fn private_add_mapping_for_guid(
    mut guid: Guid,
    mapping_string: &str,
    priority: MappingPriority,
) -> Result<(u64, bool)> {
    assert_joysticks_locked();

    let name = private_get_gamepad_name_from_mapping_string(mapping_string)
        .ok_or_else(|| Error::new(format!("Couldn't parse name from {mapping_string}")))?;

    let mut mapping = private_get_gamepad_mapping_from_mapping_string(mapping_string)
        .ok_or_else(|| Error::new(format!("Couldn't parse {mapping_string}")))?;

    // Fix up the GUID and the mapping with the CRC, if needed
    let (_, _, _, mut crc) = joystick_guid_info(guid);
    if crc != 0 {
        // Make sure the mapping has the CRC
        let mut crc_end = String::new();
        if let Some(pos) = mapping.find(GAMEPAD_CRC_FIELD) {
            if let Some(comma) = mapping[pos..].find(',') {
                crc_end = mapping[pos + comma + 1..].to_string();
            }
            mapping.truncate(pos);
        }

        // Make sure there's a comma before the CRC
        let optional_comma = if mapping.is_empty() || mapping.ends_with(',') {
            ""
        } else {
            ","
        };

        mapping = format!("{mapping}{optional_comma}{GAMEPAD_CRC_FIELD}{crc:04x},{crc_end}");
    } else {
        // Make sure the GUID has the CRC, for matching purposes
        if let Some(pos) = mapping.find(GAMEPAD_CRC_FIELD) {
            crc = crate::stdlib::string::strtol(&mapping[pos + GAMEPAD_CRC_FIELD_SIZE..], 16).0
                as u16;
            if crc != 0 {
                set_joystick_guid_crc(&mut guid, crc);
            }
        }
    }

    push_mapping_change_tracking();

    let result = match private_get_gamepad_mapping_for_guid(guid, true) {
        Some(existing) => {
            // Only overwrite the mapping if the priority is the same or higher.
            with_mappings(|m| {
                if let Some(entry) = m.supported.iter_mut().find(|e| e.id == existing) {
                    if entry.priority <= priority {
                        // Update existing mapping
                        entry.name = name;
                        entry.mapping = mapping;
                        entry.priority = priority;
                    }
                }
            });
            add_mapping_change_tracking(Some(existing));
            (existing, true)
        }
        None => {
            // Clear the CRC, we've already added it to the mapping
            if crc != 0 {
                set_joystick_guid_crc(&mut guid, 0);
            }
            let id = NEXT_MAPPING_ID.fetch_add(1, Ordering::Relaxed);
            // Add the mapping to the end of the list
            with_mappings(|m| {
                m.supported.push(MappingEntry {
                    id,
                    guid,
                    name,
                    mapping,
                    priority,
                })
            });
            (id, false)
        }
    };

    pop_mapping_change_tracking();

    Ok(result)
}

/// Helper function to determine pre-calculated offset to certain joystick
/// mappings. Translation of `SDL_PrivateGetGamepadMappingForNameAndGUID()`.
fn private_get_gamepad_mapping_for_name_and_guid(_name: Option<&str>, guid: Guid) -> Option<u64> {
    assert_joysticks_locked();

    private_get_gamepad_mapping_for_guid(guid, false)
}

/// `SDL_strlcat()` into a buffer of `maxlen` bytes.
pub(super) fn strlcat(dst: &mut String, src: &str, maxlen: usize) {
    let available = maxlen.saturating_sub(1).saturating_sub(dst.len());
    let mut n = src.len().min(available);
    while !src.is_char_boundary(n) {
        n -= 1;
    }
    dst.push_str(&src[..n]);
}

/// Translation of `SDL_PrivateAppendToMappingString()`.
fn private_append_to_mapping_string(
    mapping_string: &mut String,
    mapping_string_len: usize,
    input_name: &str,
    mapping: &InputMapping,
) {
    if mapping.kind == MappingKind::None {
        return;
    }

    strlcat(mapping_string, input_name, mapping_string_len);
    strlcat(mapping_string, ":", mapping_string_len);
    let buffer = match mapping.kind {
        MappingKind::Button => format!("b{}", mapping.target),
        MappingKind::Axis => format!(
            "{}a{}{}",
            if mapping.half_axis_positive {
                "+"
            } else if mapping.half_axis_negative {
                "-"
            } else {
                ""
            },
            mapping.target,
            if mapping.axis_reversed { "~" } else { "" }
        ),
        MappingKind::Hat => format!("h{}.{}", mapping.target >> 4, mapping.target & 0x0F),
        MappingKind::None => unreachable!(),
    };

    strlcat(mapping_string, &buffer, mapping_string_len);
    strlcat(mapping_string, ",", mapping_string_len);
}

/// Translation of `SDL_PrivateGenerateAutomaticGamepadMapping()`.
fn private_generate_automatic_gamepad_mapping(
    name: &str,
    mut guid: Guid,
    raw_map: &GamepadMapping,
) -> Option<u64> {
    const MAPPING_SIZE: usize = 1024;

    // Remove the CRC from the GUID
    // We already know that this GUID doesn't have a mapping without the CRC, and we want newly
    // added mappings without a CRC to override this mapping.
    set_joystick_guid_crc(&mut guid, 0);

    // Remove any commas in the name
    let mut name_string = String::new();
    strlcat(&mut name_string, name, 128);
    let name_string = name_string.replace(',', " ");

    let mut mapping = String::new();
    strlcat(&mut mapping, &format!("none,{name_string},"), MAPPING_SIZE);
    let entries: [(&str, &InputMapping); 32] = [
        ("a", &raw_map.a),
        ("b", &raw_map.b),
        ("x", &raw_map.x),
        ("y", &raw_map.y),
        ("back", &raw_map.back),
        ("guide", &raw_map.guide),
        ("start", &raw_map.start),
        ("leftstick", &raw_map.leftstick),
        ("rightstick", &raw_map.rightstick),
        ("leftshoulder", &raw_map.leftshoulder),
        ("rightshoulder", &raw_map.rightshoulder),
        ("dpup", &raw_map.dpup),
        ("dpdown", &raw_map.dpdown),
        ("dpleft", &raw_map.dpleft),
        ("dpright", &raw_map.dpright),
        ("misc1", &raw_map.misc1),
        ("misc2", &raw_map.misc2),
        ("misc3", &raw_map.misc3),
        ("misc4", &raw_map.misc4),
        ("misc5", &raw_map.misc5),
        ("misc6", &raw_map.misc6),
        /* Keep using paddle1-4 in the generated mapping so that it can be
         * reused with SDL2 */
        ("paddle1", &raw_map.right_paddle1),
        ("paddle2", &raw_map.left_paddle1),
        ("paddle3", &raw_map.right_paddle2),
        ("paddle4", &raw_map.left_paddle2),
        ("leftx", &raw_map.leftx),
        ("lefty", &raw_map.lefty),
        ("rightx", &raw_map.rightx),
        ("righty", &raw_map.righty),
        ("lefttrigger", &raw_map.lefttrigger),
        ("righttrigger", &raw_map.righttrigger),
        ("touchpad", &raw_map.touchpad),
    ];
    for (input_name, input) in entries {
        private_append_to_mapping_string(&mut mapping, MAPPING_SIZE, input_name, input);
    }

    private_add_mapping_for_guid(guid, &mapping, MappingPriority::Default)
        .ok()
        .map(|(id, _)| id)
}

/// The mapping of a joystick (creating one from its driver's layout if
/// `create_mapping`), or the default mapping.
/// Translation of `SDL_PrivateGetGamepadMapping()`.
fn private_get_gamepad_mapping(instance_id: JoystickID, create_mapping: bool) -> Option<u64> {
    assert_joysticks_locked();

    let name = joystick_name_for_id(instance_id).ok().flatten();
    let guid = joystick_guid_for_id(instance_id);
    let mut mapping = private_get_gamepad_mapping_for_name_and_guid(name.as_deref(), guid);
    if mapping.is_none() && create_mapping {
        if let Some(raw_map) = private_joystick_get_auto_gamepad_mapping(instance_id) {
            mapping = private_generate_automatic_gamepad_mapping(
                name.as_deref().unwrap_or(""),
                guid,
                &raw_map,
            );
        }
    }

    mapping.or_else(|| with_mappings(|m| m.default_mapping))
}

/// Translation of `SDL_PrivateIsGamepadPlatformMatch()`.
fn private_is_gamepad_platform_match(platform: &str) -> bool {
    #[cfg(target_os = "macos")]
    {
        // We also accept the older SDL2 platform name for macOS
        if strncasecmp(platform, "Mac OS X", platform.len()) == CmpOrdering::Equal {
            return true;
        }
    }

    strncasecmp(platform, crate::init::platform(), platform.len()) == CmpOrdering::Equal
}

/// Load a set of gamepad mappings (one per line, for the current platform
/// only), returning the number of mappings added. Translation of
/// `SDL_AddGamepadMappingsFromIO()`.
pub fn add_gamepad_mappings_from_io(src: &mut IoStream<'_>) -> Result<usize> {
    let buf = src
        .load_all()
        .map_err(|_| Error::new("Could not allocate space to read DB into memory"))?;
    Ok(add_gamepad_mappings_from_bytes(&buf))
}

fn add_gamepad_mappings_from_bytes(buf: &[u8]) -> usize {
    let mut gamepads = 0;

    let _lock = lock_joysticks();

    push_mapping_change_tracking();

    for line in buf.split(|&b| b == b'\n') {
        // (each line is read as a C string)
        let line = &line[..line.iter().position(|&b| b == 0).unwrap_or(line.len())];
        let line = String::from_utf8_lossy(line);

        // Extract and verify the platform
        if let Some(platform) = mapping_field(
            &line,
            GAMEPAD_PLATFORM_FIELD,
            GAMEPAD_PLATFORM_FIELD_SIZE,
            false,
        ) {
            if private_is_gamepad_platform_match(platform) && add_gamepad_mapping(&line) == Ok(true)
            {
                gamepads += 1;
            }
        }
    }

    pop_mapping_change_tracking();

    gamepads
}

/// Load a set of gamepad mappings from a file.
/// Translation of `SDL_AddGamepadMappingsFromFile()`.
pub fn add_gamepad_mappings_from_file(file: &str) -> Result<usize> {
    let mut stream = IoStream::from_file(file, "rb")?;
    add_gamepad_mappings_from_io(&mut stream)
}

/// Reinitialize the SDL mapping database to its initial state (the built
/// in mappings and those from the hints and the mapping file).
/// Translation of `SDL_ReloadGamepadMappings()`.
pub fn reload_gamepad_mappings() -> Result<()> {
    let _lock = lock_joysticks();

    push_mapping_change_tracking();

    let open: Vec<u64> = with_gamepads(|s| s.gamepads.iter().map(|g| g.mapping).collect());
    for mapping in open {
        add_mapping_change_tracking(Some(mapping));
    }

    quit_gamepad_mappings();
    init_gamepad_mappings();

    pop_mapping_change_tracking();

    Ok(())
}

/// Swap B and X in a mapping that uses the GameCube labels, marking it as
/// positional. Translation of `SDL_ConvertMappingToPositionalAXBY()`.
fn convert_mapping_to_positional_axby(mapping: &str) -> String {
    let mut remapped = mapping.as_bytes().to_vec();
    let button_b = mapping.find(",b:");
    let button_x = mapping.find(",x:");
    let hint = mapping.find("hint:SDL_GAMECONTROLLER_USE_GAMECUBE_LABELS");

    if let Some(i) = button_b {
        remapped[i + 1] = b'x';
    }
    if let Some(i) = button_x {
        remapped[i + 1] = b'b';
    }
    if let Some(i) = hint {
        remapped.insert(i + 5, b'!');
    }
    String::from_utf8_lossy(&remapped).into_owned()
}

/// Swap A/B and X/Y in a mapping that uses the Nintendo labels, marking it
/// as positional. Translation of `SDL_ConvertMappingToPositionalBAXY()`.
fn convert_mapping_to_positional_baxy(mapping: &str) -> String {
    let mut remapped = mapping.as_bytes().to_vec();
    let button_a = mapping.find(",a:");
    let button_b = mapping.find(",b:");
    let button_x = mapping.find(",x:");
    let button_y = mapping.find(",y:");
    let hint = mapping.find("hint:SDL_GAMECONTROLLER_USE_BUTTON_LABELS");

    if let Some(i) = button_a {
        remapped[i + 1] = b'b';
    }
    if let Some(i) = button_b {
        remapped[i + 1] = b'a';
    }
    if let Some(i) = button_x {
        remapped[i + 1] = b'y';
    }
    if let Some(i) = button_y {
        remapped[i + 1] = b'x';
    }
    if let Some(i) = hint {
        remapped.insert(i + 5, b'!');
    }
    String::from_utf8_lossy(&remapped).into_owned()
}

/// Add or update an entry into the mappings database with a priority;
/// `Ok(true)` if a new mapping was added. Translation of
/// `SDL_PrivateAddGamepadMapping()`.
fn private_add_gamepad_mapping(mapping_string: &str, priority: MappingPriority) -> Result<bool> {
    assert_joysticks_locked();

    let pch_guid = private_get_gamepad_guid_from_mapping_string(mapping_string)
        .ok_or_else(|| Error::new(format!("Couldn't parse GUID from {mapping_string}")))?;
    let is_default_mapping = strcasecmp(&pch_guid, "default") == CmpOrdering::Equal;
    let is_xinput_mapping =
        !is_default_mapping && strcasecmp(&pch_guid, "xinput") == CmpOrdering::Equal;
    let guid = Guid::parse_lossy(&pch_guid);

    let mut mapping_string = mapping_string.to_string();

    let (vendor, product, _, _) = joystick_guid_info(guid);
    if is_joystick_gamecube(vendor, product)
        && !mapping_string.contains("SDL_GAMECONTROLLER_USE_GAMECUBE_LABELS")
    {
        mapping_string = format!("{mapping_string}hint:SDL_GAMECONTROLLER_USE_GAMECUBE_LABELS:=1,");
    }

    // Extract and verify the hint field
    if let Some(pos) = mapping_string.find(GAMEPAD_HINT_FIELD) {
        let mut tmp = &mapping_string.as_bytes()[pos + GAMEPAD_HINT_FIELD_SIZE..];

        let negate = tmp.first() == Some(&b'!');
        if negate {
            tmp = &tmp[1..];
        }

        let mut len = 0;
        while len < tmp.len() && tmp[len] != b',' && tmp[len] != b':' && len < 128 - 1 {
            len += 1;
        }
        let hint = String::from_utf8_lossy(&tmp[..len]).into_owned();
        tmp = &tmp[len..];

        let default_value = if tmp.starts_with(b":=") {
            atoi(&String::from_utf8_lossy(&tmp[2..])) != 0
        } else {
            false
        };

        if hint == "SDL_GAMECONTROLLER_USE_GAMECUBE_LABELS" {
            // This hint is used to signal whether the mapping uses positional buttons or not
            if negate {
                // This mapping uses positional buttons, we can use it as-is
            } else {
                // This mapping uses labeled buttons, we need to swap them to positional
                mapping_string = convert_mapping_to_positional_axby(&mapping_string);
            }
        } else if hint == "SDL_GAMECONTROLLER_USE_BUTTON_LABELS" {
            // This hint is used to signal whether the mapping uses positional buttons or not
            if negate {
                // This mapping uses positional buttons, we can use it as-is
            } else {
                // This mapping uses labeled buttons, we need to swap them to positional
                mapping_string = convert_mapping_to_positional_baxy(&mapping_string);
            }
        } else {
            let hint_value = hints::get(&hint).or_else(|| crate::stdlib::getenv_unsafe(&hint));
            let mut value = hints::string_to_bool(hint_value.as_deref(), default_value);
            if negate {
                value = !value;
            }
            if !value {
                return Ok(false);
            }
        }
    }

    // (The Android SDK version fields arrive with the Android driver.)

    let (mapping, existing) = private_add_mapping_for_guid(guid, &mapping_string, priority)?;

    if existing {
        return Ok(false);
    }
    with_mappings(|m| {
        if is_default_mapping {
            m.default_mapping = Some(mapping);
        } else if is_xinput_mapping {
            m.xinput_mapping = Some(mapping);
        }
    });
    Ok(true)
}

/// Add support for gamepads that SDL is unaware of or change the binding of
/// an existing gamepad. The mapping string has the format
/// `"GUID,name,mapping"`, where GUID is the string value from
/// [`Guid`]'s `Display`, name is the human readable string for the device
/// and mappings are gamepad mappings to joystick ones. Returns `Ok(true)`
/// if a new mapping is added, `Ok(false)` if an existing mapping is updated
/// (or the mapping's `hint:` field disables it).
/// Translation of `SDL_AddGamepadMapping()`.
pub fn add_gamepad_mapping(mapping: &str) -> Result<bool> {
    let _lock = lock_joysticks();
    private_add_gamepad_mapping(mapping, MappingPriority::Api)
}

/// Create a mapping string for a mapping. Translation of `CreateMappingString()`.
fn create_mapping_string(name: &str, mapping: &str, guid: Guid) -> String {
    assert_joysticks_locked();

    let mut mapping_string = format!("{guid},{name},{mapping}");

    if !mapping.contains(GAMEPAD_PLATFORM_FIELD) {
        // Upstream checks the mapping's last byte, reading the byte before
        // the string for an empty mapping (whose name's comma already ends
        // the string); checking the whole string here gives no `,,`.
        if !mapping_string.ends_with(',') {
            mapping_string.push(',');
        }
        mapping_string.push_str(GAMEPAD_PLATFORM_FIELD);
        mapping_string.push_str(crate::init::platform());
        mapping_string.push(',');
    }

    // Make sure multiple platform strings haven't made their way into the mapping
    if let Some(first) = mapping_string.find(GAMEPAD_PLATFORM_FIELD) {
        if let Some(second) = mapping_string[first + 1..].find(GAMEPAD_PLATFORM_FIELD) {
            mapping_string.truncate(first + 1 + second);
        }
    }
    mapping_string
}

/// The current gamepad mappings. Translation of `SDL_GetGamepadMappings()`.
pub fn gamepad_mappings() -> Vec<String> {
    let _lock = lock_joysticks();

    let entries: Vec<(Guid, String, String)> = with_mappings(|m| {
        m.supported
            .iter()
            .filter(|e| e.guid != Guid::ZERO)
            .map(|e| (e.guid, e.name.clone(), e.mapping.clone()))
            .collect()
    });
    entries
        .iter()
        .map(|(guid, name, mapping)| create_mapping_string(name, mapping, *guid))
        .collect()
}

/// The gamepad mapping string for a GUID.
/// Translation of `SDL_GetGamepadMappingForGUID()`.
pub fn gamepad_mapping_for_guid(guid: Guid) -> Result<String> {
    let _lock = lock_joysticks();
    let (_, name, mapping) = private_get_gamepad_mapping_for_guid(guid, false)
        .and_then(mapping_info)
        .ok_or_else(|| Error::new("Mapping not available"))?;
    Ok(create_mapping_string(&name, &mapping, guid))
}

/// Set the current mapping of a joystick or gamepad (`None` for an empty
/// mapping, `"*,*,"`). Details about mappings are discussed with
/// [`add_gamepad_mapping`]. Translation of `SDL_SetGamepadMapping()`.
pub fn set_gamepad_mapping(instance_id: JoystickID, mapping: Option<&str>) -> Result<()> {
    let guid = joystick_guid_for_id(instance_id);

    if guid == Guid::ZERO {
        return Err(Error::invalid_param("instance_id"));
    }

    let mapping = mapping.unwrap_or("*,*,");

    let _lock = lock_joysticks();
    private_add_mapping_for_guid(guid, mapping, MappingPriority::Api).map(|_| ())
}

/// Translation of `SDL_LoadGamepadHints()`.
fn load_gamepad_hints() {
    let Some(hint) = hints::get(hints::GAMECONTROLLERCONFIG).filter(|h| !h.is_empty()) else {
        return;
    };

    push_mapping_change_tracking();

    for user_mapping in hint.split('\n') {
        let _ = private_add_gamepad_mapping(user_mapping, MappingPriority::User);
    }

    pop_mapping_change_tracking();
}

/// The expected gamepad mapping filepath. Usually this will just be
/// `SDL_HINT_GAMECONTROLLERCONFIG_FILE`, but for Android, we want to get the
/// internal storage path. Translation of `SDL_GetGamepadMappingFilePath()`.
fn gamepad_mapping_file_path() -> Option<String> {
    const PATH_SIZE: usize = 1024;

    let hint = hints::get(hints::GAMECONTROLLERCONFIG_FILE).filter(|h| !h.is_empty())?;
    (hint.len() < PATH_SIZE).then_some(hint)
}

/// Initialize the gamepad system, mostly load our DB of gamepad config
/// mappings. Translation of `SDL_InitGamepadMappings()`.
pub(crate) fn init_gamepad_mappings() {
    assert_joysticks_locked();

    push_mapping_change_tracking();

    for section in super::gamepad_db::SECTIONS {
        for mapping in section {
            let _ = private_add_gamepad_mapping(mapping, MappingPriority::Default);
        }
    }

    if let Some(path) = gamepad_mapping_file_path() {
        let _ = add_gamepad_mappings_from_file(&path);
    }

    // load in any user supplied config
    load_gamepad_hints();

    ALLOWED_GAMEPADS.load();
    IGNORED_GAMEPADS.load();

    pop_mapping_change_tracking();
}

/// Translation of `SDL_InitGamepads()`.
pub(crate) fn init_gamepads() -> Result<()> {
    GAMEPADS_INITIALIZED.store(true, Ordering::Relaxed);

    let _lock = lock_joysticks();

    with_gamepads(|s| s.names = Some(HashMap::new()));

    // Watch for joystick events and fire gamepad ones if needed
    let watch = queue::add_watch(gamepad_event_watcher);
    with_gamepads(|s| s.watch = Some(watch));

    // Send added events for gamepads currently attached
    for id in joysticks() {
        if is_gamepad(id) {
            private_gamepad_added(id);
        }
    }

    Ok(())
}

/// Whether a gamepad is currently connected. Translation of `SDL_HasGamepad()`.
pub fn has_gamepad() -> bool {
    joysticks().into_iter().rev().any(is_gamepad)
}

/// The currently connected gamepads. Translation of `SDL_GetGamepads()`.
pub fn gamepads() -> Vec<JoystickID> {
    let mut joysticks = joysticks();
    joysticks.retain(|&id| is_gamepad(id));
    joysticks
}

/// The implementation dependent name of a gamepad (also available after it
/// is removed). Translation of `SDL_GetGamepadNameForID()`.
pub fn gamepad_name_for_id(instance_id: JoystickID) -> Result<Option<String>> {
    let _lock = lock_joysticks();
    update_gamepad_name_for_id(instance_id)
}

/// The implementation dependent path of a gamepad.
/// Translation of `SDL_GetGamepadPathForID()`.
pub fn gamepad_path_for_id(instance_id: JoystickID) -> Result<String> {
    super::joystick_path_for_id(instance_id)
}

/// The player index of a gamepad, or -1. Translation of `SDL_GetGamepadPlayerIndexForID()`.
pub fn gamepad_player_index_for_id(instance_id: JoystickID) -> i32 {
    super::joystick_player_index_for_id(instance_id)
}

/// The implementation-dependent GUID of a gamepad.
/// Translation of `SDL_GetGamepadGUIDForID()`.
pub fn gamepad_guid_for_id(instance_id: JoystickID) -> Guid {
    joystick_guid_for_id(instance_id)
}

/// The USB vendor ID of a gamepad, 0 if unavailable.
/// Translation of `SDL_GetGamepadVendorForID()`.
pub fn gamepad_vendor_for_id(instance_id: JoystickID) -> u16 {
    super::joystick_vendor_for_id(instance_id)
}

/// The USB product ID of a gamepad, 0 if unavailable.
/// Translation of `SDL_GetGamepadProductForID()`.
pub fn gamepad_product_for_id(instance_id: JoystickID) -> u16 {
    super::joystick_product_for_id(instance_id)
}

/// The product version of a gamepad, 0 if unavailable.
/// Translation of `SDL_GetGamepadProductVersionForID()`.
pub fn gamepad_product_version_for_id(instance_id: JoystickID) -> u16 {
    super::joystick_product_version_for_id(instance_id)
}

/// The type of a gamepad (as its mapping may override it).
/// Translation of `SDL_GetGamepadTypeForID()`.
pub fn gamepad_type_for_id(instance_id: JoystickID) -> GamepadType {
    let mut gamepad_type = GamepadType::Unknown;

    {
        let _lock = lock_joysticks();
        if let Some((_, _, mapping)) =
            private_get_gamepad_mapping(instance_id, true).and_then(mapping_info)
        {
            if let Some(type_string) =
                mapping_field(&mapping, GAMEPAD_TYPE_FIELD, GAMEPAD_TYPE_FIELD_SIZE, false)
            {
                gamepad_type = GamepadType::from_string(type_string);
            }
        }
    }

    if gamepad_type != GamepadType::Unknown {
        return gamepad_type;
    }
    real_gamepad_type_for_id(instance_id)
}

/// The type of a gamepad, ignoring any mapping override.
/// Translation of `SDL_GetRealGamepadTypeForID()`.
pub fn real_gamepad_type_for_id(instance_id: JoystickID) -> GamepadType {
    let _lock = lock_joysticks();
    match joystick_virtual_gamepad_info_for_id(instance_id) {
        Some(info) => info.gamepad_type,
        None => {
            let name = joystick_name_for_id(instance_id).ok().flatten();
            super::gamepad_type_from_guid(joystick_guid_for_id(instance_id), name.as_deref())
        }
    }
}

/// The mapping of a gamepad. Translation of `SDL_GetGamepadMappingForID()`.
pub fn gamepad_mapping_for_id(instance_id: JoystickID) -> Option<String> {
    let _lock = lock_joysticks();
    let (_, name, mapping) =
        private_get_gamepad_mapping(instance_id, true).and_then(mapping_info)?;
    let guid = joystick_guid_for_id(instance_id);
    Some(format!("{guid},{name},{mapping}"))
}

/// Whether a joystick with this name and GUID is a supported gamepad.
/// Translation of `SDL_IsGamepadNameAndGUID()`.
#[allow(dead_code)] // (used by the platform drivers)
pub(crate) fn is_gamepad_name_and_guid(name: Option<&str>, guid: Guid) -> bool {
    let _lock = lock_joysticks();
    with_mappings(|m| m.default_mapping.is_some())
        || private_get_gamepad_mapping_for_name_and_guid(name, guid).is_some()
}

/// Whether a joystick is supported by the gamepad interface.
/// Translation of `SDL_IsGamepad()`.
pub fn is_gamepad(instance_id: JoystickID) -> bool {
    let _lock = lock_joysticks();

    if let Some(Some(value)) = with_mappings(|m| {
        m.instance_ids
            .as_ref()
            .map(|ids| ids.get(&instance_id).copied())
    }) {
        return value;
    }

    let result = private_get_gamepad_mapping(instance_id, true).is_some();
    with_mappings(|m| {
        m.instance_ids
            .get_or_insert_with(HashMap::new)
            .insert(instance_id, result);
    });
    result
}

/// Whether a gamepad should be ignored by SDL.
/// Translation of `SDL_ShouldIgnoreGamepad()`.
pub(crate) fn should_ignore_gamepad(
    vendor_id: u16,
    product_id: u16,
    version: u16,
    name: Option<&str>,
) -> bool {
    if let Some(name) = name {
        for &(word, pos) in GAMEPAD_BLACKLIST_WORDS {
            match pos {
                BlacklistPosition::Begin => {
                    if name.starts_with(word) {
                        return true;
                    }
                }
                BlacklistPosition::End => {
                    if name.ends_with(word) {
                        return true;
                    }
                }
                BlacklistPosition::Anywhere => {
                    if name.contains(word) {
                        if name.starts_with("PG-") {
                            // Ipega gamepads have modes with keyboard keys in addition to gamepad controls
                            continue;
                        }
                        return true;
                    }
                }
            }
        }
    }

    let hint = crate::stdlib::getenv_unsafe("SDL_GAMECONTROLLER_ALLOW_STEAM_VIRTUAL_GAMEPAD");
    let allow_steam_virtual_gamepad = hints::string_to_bool(hint.as_deref(), false);
    // (The WIN_IsWine() check arrives with the Windows platform layer.)

    if is_joystick_steam_virtual_gamepad(vendor_id, product_id, version) {
        return !allow_steam_virtual_gamepad;
    }

    if ALLOWED_GAMEPADS.has_included_entries() {
        !ALLOWED_GAMEPADS.contains(vendor_id, product_id)
    } else {
        IGNORED_GAMEPADS.contains(vendor_id, product_id)
    }
}

/// The current state of a gamepad axis (`SDL_GetGamepadAxis()` by instance id).
fn gamepad_axis_state(gamepad: JoystickID, axis: GamepadAxis) -> i16 {
    let Some(bindings) = with_gamepad(gamepad, |g| g.bindings.clone()) else {
        return 0;
    };
    with_joystick(gamepad, |j| {
        for binding in &bindings {
            let GamepadBindingOutput::Axis {
                axis: output_axis,
                axis_min: output_min,
                axis_max: output_max,
            } = binding.output
            else {
                continue;
            };
            if output_axis != axis {
                continue;
            }
            let mut value = 0;

            match binding.input {
                GamepadBindingInput::Axis {
                    axis: input_axis,
                    axis_min: input_min,
                    axis_max: input_max,
                } => {
                    /* Missing axes return zero, which would normalize to a half-pressed trigger. */
                    let Some(info) = usize::try_from(input_axis).ok().and_then(|a| j.axes.get(a))
                    else {
                        continue;
                    };
                    value = i32::from(info.value);
                    let valid_input_range = if input_min < input_max {
                        value >= input_min && value <= input_max
                    } else {
                        value >= input_max && value <= input_min
                    };
                    if valid_input_range {
                        if input_min != output_min || input_max != output_max {
                            let normalized_value =
                                (value - input_min) as f32 / (input_max - input_min) as f32;
                            value = output_min
                                + (normalized_value * (output_max - output_min) as f32) as i32;
                        }
                    } else {
                        value = 0;
                    }
                }
                GamepadBindingInput::Button(button) => {
                    if usize::try_from(button).ok().and_then(|b| j.buttons.get(b)) == Some(&true) {
                        value = output_max;
                    }
                }
                GamepadBindingInput::Hat { hat, hat_mask } => {
                    let state = usize::try_from(hat)
                        .ok()
                        .and_then(|h| j.hats.get(h))
                        .copied()
                        .unwrap_or(0);
                    if i32::from(state) & hat_mask != 0 {
                        value = output_max;
                    }
                }
            }

            let valid_output_range = if output_min < output_max {
                value >= output_min && value <= output_max
            } else {
                value >= output_max && value <= output_min
            };
            // If the value is zero, there might be another binding that makes it non-zero
            if value != 0 && valid_output_range {
                return value as i16;
            }
        }
        0
    })
    .unwrap_or(0)
}

/// The current state of a gamepad button (`SDL_GetGamepadButton()` by instance id).
fn gamepad_button_state(gamepad: JoystickID, button: GamepadButton) -> bool {
    let Some(bindings) = with_gamepad(gamepad, |g| g.bindings.clone()) else {
        return false;
    };
    with_joystick(gamepad, |j| {
        let mut result = false;
        for binding in &bindings {
            if binding.output != GamepadBindingOutput::Button(button) {
                continue;
            }
            match binding.input {
                GamepadBindingInput::Axis {
                    axis,
                    axis_min,
                    axis_max,
                } => {
                    let value = usize::try_from(axis)
                        .ok()
                        .and_then(|a| j.axes.get(a))
                        .map_or(0, |a| i32::from(a.value));
                    let threshold = axis_min + (axis_max - axis_min) / 2;
                    if axis_min < axis_max {
                        let valid_input_range = value >= axis_min && value <= axis_max;
                        if valid_input_range {
                            result |= value >= threshold;
                        }
                    } else {
                        let valid_input_range = value >= axis_max && value <= axis_min;
                        if valid_input_range {
                            result |= value <= threshold;
                        }
                    }
                }
                GamepadBindingInput::Button(input) => {
                    result |=
                        usize::try_from(input).ok().and_then(|b| j.buttons.get(b)) == Some(&true);
                }
                GamepadBindingInput::Hat { hat, hat_mask } => {
                    let state = usize::try_from(hat)
                        .ok()
                        .and_then(|h| j.hats.get(h))
                        .copied()
                        .unwrap_or(0);
                    result |= (i32::from(state) & hat_mask) != 0;
                }
            }
        }
        result
    })
    .unwrap_or(false)
}

/// Translation of `SDL_GetGamepadButtonLabelForFaceStyle()`.
fn gamepad_button_label_for_face_style(
    face_style: FaceStyle,
    button: GamepadButton,
) -> GamepadButtonLabel {
    use GamepadButton as B;
    use GamepadButtonLabel as L;
    match (face_style, button) {
        (FaceStyle::Abxy, B::South) => L::A,
        (FaceStyle::Abxy, B::East) => L::B,
        (FaceStyle::Abxy, B::West) => L::X,
        (FaceStyle::Abxy, B::North) => L::Y,
        (FaceStyle::Axby, B::South) => L::A,
        (FaceStyle::Axby, B::East) => L::X,
        (FaceStyle::Axby, B::West) => L::B,
        (FaceStyle::Axby, B::North) => L::Y,
        (FaceStyle::Bayx, B::South) => L::B,
        (FaceStyle::Bayx, B::East) => L::A,
        (FaceStyle::Bayx, B::West) => L::Y,
        (FaceStyle::Bayx, B::North) => L::X,
        (FaceStyle::Sony, B::South) => L::Cross,
        (FaceStyle::Sony, B::East) => L::Circle,
        (FaceStyle::Sony, B::West) => L::Square,
        (FaceStyle::Sony, B::North) => L::Triangle,
        _ => L::Unknown,
    }
}

/// The label of a button on a gamepad type.
/// Translation of `SDL_GetGamepadButtonLabelForType()`.
pub fn gamepad_button_label_for_type(
    gamepad_type: GamepadType,
    button: GamepadButton,
) -> GamepadButtonLabel {
    gamepad_button_label_for_face_style(gamepad_face_style_for_gamepad_type(gamepad_type), button)
}

/// An open gamepad. Translation of `SDL_Gamepad *`; dropping the last
/// handle closes the gamepad (`SDL_CloseGamepad()`).
#[derive(Debug)]
pub struct Gamepad {
    instance_id: JoystickID,
    serial: u64,
}

impl Gamepad {
    /// Open a gamepad for use. Opening a gamepad that is already open adds
    /// a reference to it. Translation of `SDL_OpenGamepad()`.
    pub fn open(instance_id: JoystickID) -> Result<Gamepad> {
        let _lock = lock_joysticks();

        // If the gamepad is already open, return it
        if let Some(serial) = with_gamepad(instance_id, |g| {
            g.ref_count += 1;
            g.serial
        }) {
            return Ok(Gamepad {
                instance_id,
                serial,
            });
        }

        // Find a gamepad mapping
        let supported_gamepad =
            private_get_gamepad_mapping(instance_id, true).ok_or_else(|| {
                Error::new(format!("Couldn't find mapping for device ({instance_id})"))
            })?;

        // Create and initialize the gamepad
        let joystick = Joystick::open(instance_id)?;
        let (naxes, nhats) = joystick.with(|j| (j.axes.len(), j.hats.len()))?;

        let serial = NEXT_SERIAL.fetch_add(1, Ordering::Relaxed);
        let gamepad = GamepadData {
            joystick,
            instance_id,
            // Add the gamepad to list
            ref_count: 1,
            serial,
            name: String::new(),
            gamepad_type: GamepadType::Unknown,
            face_style: FaceStyle::Unknown,
            mapping: supported_gamepad,
            bindings: Vec::new(),
            last_match_axis: vec![None; naxes],
            last_hat_mask: vec![0; nhats],
            guide_button_down: 0,
        };

        // Link the gamepad in the list
        with_gamepads(|s| s.gamepads.insert(0, gamepad));

        private_load_button_mapping(instance_id, supported_gamepad);

        Ok(Gamepad {
            instance_id,
            serial,
        })
    }

    /// Another handle to an open gamepad (adding a reference to it), or
    /// `None` if it isn't open. Translation of `SDL_GetGamepadFromID()`.
    pub fn from_id(instance_id: JoystickID) -> Option<Gamepad> {
        let _lock = lock_joysticks();
        with_gamepad(instance_id, |g| {
            g.ref_count += 1;
            Gamepad {
                instance_id,
                serial: g.serial,
            }
        })
    }

    /// Another handle to the open gamepad with a player index.
    /// Translation of `SDL_GetGamepadFromPlayerIndex()`.
    pub fn from_player_index(player_index: i32) -> Option<Gamepad> {
        let _lock = lock_joysticks();
        let joystick = Joystick::from_player_index(player_index)?;
        Gamepad::from_id(joystick.id())
    }

    /// Run `f` on this gamepad's state (`CHECK_GAMEPAD_MAGIC`).
    fn with<R>(&self, f: impl FnOnce(&mut GamepadData) -> R) -> Result<R> {
        let _lock = lock_joysticks();
        let (result, joystick) = with_gamepads(|s| {
            let g = s
                .gamepads
                .iter_mut()
                .find(|g| g.instance_id == self.instance_id && g.serial == self.serial)?;
            let joystick = (g.joystick.instance_id, g.joystick.serial);
            Some((f(g), joystick))
        })
        .ok_or_else(|| Error::invalid_param("gamepad"))?;
        // (SDL_IsJoystickValid(gamepad->joystick))
        joystick_view(joystick)
            .with(|_| ())
            .map_err(|_| Error::invalid_param("gamepad"))?;
        Ok(result)
    }

    /// The gamepad's joystick, without taking a reference
    /// (`SDL_GetGamepadJoystick()` for internal use).
    fn joystick_view(&self) -> Result<ManuallyDrop<Joystick>> {
        let joystick = self.with(|g| (g.joystick.instance_id, g.joystick.serial))?;
        Ok(joystick_view(joystick))
    }

    /// Get the underlying joystick (a new handle to it).
    /// Translation of `SDL_GetGamepadJoystick()`.
    pub fn joystick(&self) -> Result<Joystick> {
        let _lock = lock_joysticks();
        self.joystick_view()?;
        Joystick::from_id(self.instance_id).ok_or_else(|| Error::invalid_param("gamepad"))
    }

    /// Whether the gamepad has a given axis.
    /// Translation of `SDL_GamepadHasAxis()`.
    pub fn has_axis(&self, axis: GamepadAxis) -> bool {
        self.with(|g| {
            g.bindings.iter().any(
                |b| matches!(b.output, GamepadBindingOutput::Axis { axis: a, .. } if a == axis),
            )
        })
        .unwrap_or(false)
    }

    /// The current state of an axis control on the gamepad.
    /// Translation of `SDL_GetGamepadAxis()`.
    pub fn axis(&self, axis: GamepadAxis) -> i16 {
        let _lock = lock_joysticks();
        if self.with(|_| ()).is_err() {
            return 0;
        }
        gamepad_axis_state(self.instance_id, axis)
    }

    /// Whether the gamepad has a given button.
    /// Translation of `SDL_GamepadHasButton()`.
    pub fn has_button(&self, button: GamepadButton) -> bool {
        self.with(|g| {
            g.bindings
                .iter()
                .any(|b| b.output == GamepadBindingOutput::Button(button))
        })
        .unwrap_or(false)
    }

    /// The current state of a button on the gamepad.
    /// Translation of `SDL_GetGamepadButton()`.
    pub fn button(&self, button: GamepadButton) -> bool {
        let _lock = lock_joysticks();
        if self.with(|_| ()).is_err() {
            return false;
        }
        gamepad_button_state(self.instance_id, button)
    }

    /// The label of a button on the gamepad.
    /// Translation of `SDL_GetGamepadButtonLabel()`.
    pub fn button_label(&self, button: GamepadButton) -> GamepadButtonLabel {
        match self.with(|g| g.face_style) {
            Ok(face_style) => gamepad_button_label_for_face_style(face_style, button),
            Err(_) => GamepadButtonLabel::Unknown,
        }
    }

    /// The number of touchpads on the gamepad.
    /// Translation of `SDL_GetNumGamepadTouchpads()`.
    pub fn num_touchpads(&self) -> usize {
        let _lock = lock_joysticks();
        self.joystick_view()
            .and_then(|j| j.with(|j| j.touchpads.len()))
            .unwrap_or(0)
    }

    /// The number of supported simultaneous fingers on a touchpad.
    /// Translation of `SDL_GetNumGamepadTouchpadFingers()`.
    pub fn num_touchpad_fingers(&self, touchpad: usize) -> usize {
        let _lock = lock_joysticks();
        self.joystick_view()
            .and_then(|j| j.with(|j| j.touchpads.get(touchpad).map_or(0, |t| t.fingers.len())))
            .unwrap_or(0)
    }

    /// The current state of a finger on a touchpad: `(down, x, y, pressure)`.
    /// Translation of `SDL_GetGamepadTouchpadFinger()`.
    pub fn touchpad_finger(&self, touchpad: usize, finger: usize) -> Result<(bool, f32, f32, f32)> {
        let _lock = lock_joysticks();
        self.joystick_view()?.with(|j| {
            let touchpad_info = j
                .touchpads
                .get(touchpad)
                .ok_or_else(|| Error::invalid_param("touchpad"))?;
            let info = touchpad_info
                .fingers
                .get(finger)
                .ok_or_else(|| Error::invalid_param("finger"))?;
            Ok((info.down, info.x, info.y, info.pressure))
        })?
    }

    /// Whether the gamepad has a particular sensor.
    /// Translation of `SDL_GamepadHasSensor()`.
    pub fn has_sensor(&self, sensor_type: SensorType) -> bool {
        self.joystick_view()
            .is_ok_and(|j| j.has_sensor(sensor_type))
    }

    /// Set whether data reporting for a gamepad sensor is enabled.
    /// Translation of `SDL_SetGamepadSensorEnabled()`.
    pub fn set_sensor_enabled(&self, sensor_type: SensorType, enabled: bool) -> Result<()> {
        self.joystick_view()?
            .set_sensor_enabled(sensor_type, enabled)
    }

    /// Whether sensor data reporting is enabled for a gamepad sensor.
    /// Translation of `SDL_GamepadSensorEnabled()`.
    pub fn sensor_enabled(&self, sensor_type: SensorType) -> bool {
        self.joystick_view()
            .is_ok_and(|j| j.sensor_enabled(sensor_type))
    }

    /// The data rate of a gamepad sensor. Translation of `SDL_GetGamepadSensorDataRate()`.
    pub fn sensor_data_rate(&self, sensor_type: SensorType) -> f32 {
        self.joystick_view()
            .map_or(0.0, |j| j.sensor_data_rate(sensor_type))
    }

    /// The current state of a gamepad sensor.
    /// Translation of `SDL_GetGamepadSensorData()`.
    pub fn sensor_data(&self, sensor_type: SensorType, data: &mut [f32]) -> Result<()> {
        self.joystick_view()?.sensor_data(sensor_type, data)
    }

    /// Whether the gamepad has a capacitive sensor.
    /// Translation of `SDL_GamepadHasCapSense()`.
    pub fn has_capsense(&self, capsense_type: GamepadCapSenseType) -> bool {
        let _lock = lock_joysticks();
        self.joystick_view()
            .and_then(|j| j.with(|j| j.capsenses.iter().any(|c| c.capsense_type == capsense_type)))
            .unwrap_or(false)
    }

    /// Whether a capacitive sensor is touched.
    /// Translation of `SDL_GetGamepadCapSense()`.
    pub fn capsense(&self, capsense_type: GamepadCapSenseType) -> bool {
        let _lock = lock_joysticks();
        self.joystick_view()
            .and_then(|j| {
                j.with(|j| {
                    j.capsenses
                        .iter()
                        .find(|c| c.capsense_type == capsense_type)
                        .is_some_and(|c| c.down)
                })
            })
            .unwrap_or(false)
    }

    /// The instance ID of the gamepad (0 for an invalid handle).
    /// Translation of `SDL_GetGamepadID()`.
    pub fn id(&self) -> JoystickID {
        match self.joystick_view() {
            Ok(j) => j.id(),
            Err(_) => 0,
        }
    }

    /// The properties of the gamepad. Translation of `SDL_GetGamepadProperties()`.
    pub fn properties(&self) -> Result<Properties> {
        self.joystick_view()?.properties()
    }

    /// The implementation dependent name of the gamepad.
    /// Translation of `SDL_GetGamepadName()`.
    pub fn name(&self) -> Result<Option<String>> {
        let _lock = lock_joysticks();
        let name = self.with(|g| g.name.clone())?;
        let joystick = self.joystick_view()?;
        let steam_handle = joystick.with(|j| j.steam_handle)?;
        if name == "*" || steam_handle != 0 {
            joystick.name()
        } else {
            Ok(Some(name))
        }
    }

    /// The implementation dependent path of the gamepad.
    /// Translation of `SDL_GetGamepadPath()`.
    pub fn path(&self) -> Result<String> {
        self.joystick_view()?.path()
    }

    /// The type of the gamepad. Translation of `SDL_GetGamepadType()`.
    pub fn gamepad_type(&self) -> GamepadType {
        let _lock = lock_joysticks();
        let Ok(gamepad_type) = self.with(|g| g.gamepad_type) else {
            return GamepadType::Unknown;
        };
        match joystick_virtual_gamepad_info_for_id(self.instance_id) {
            Some(info) => info.gamepad_type,
            None => gamepad_type,
        }
    }

    /// The type of the gamepad, ignoring any mapping override.
    /// Translation of `SDL_GetRealGamepadType()`.
    pub fn real_gamepad_type(&self) -> GamepadType {
        let Ok(joystick) = self.joystick_view() else {
            return GamepadType::Unknown;
        };
        let name = joystick.name().ok().flatten();
        super::gamepad_type_from_guid(joystick.guid(), name.as_deref())
    }

    /// The player index of the gamepad, or -1.
    /// Translation of `SDL_GetGamepadPlayerIndex()`.
    pub fn player_index(&self) -> i32 {
        self.joystick_view().map_or(-1, |j| j.player_index())
    }

    /// Set the player index of the gamepad (-1 to clear it).
    /// Translation of `SDL_SetGamepadPlayerIndex()`.
    pub fn set_player_index(&self, player_index: i32) -> Result<()> {
        self.joystick_view()?.set_player_index(player_index)
    }

    /// The USB vendor ID of the gamepad. Translation of `SDL_GetGamepadVendor()`.
    pub fn vendor(&self) -> u16 {
        self.joystick_view().map_or(0, |j| j.vendor())
    }

    /// The USB product ID of the gamepad. Translation of `SDL_GetGamepadProduct()`.
    pub fn product(&self) -> u16 {
        self.joystick_view().map_or(0, |j| j.product())
    }

    /// The product version of the gamepad.
    /// Translation of `SDL_GetGamepadProductVersion()`.
    pub fn product_version(&self) -> u16 {
        self.joystick_view().map_or(0, |j| j.product_version())
    }

    /// The firmware version of the gamepad.
    /// Translation of `SDL_GetGamepadFirmwareVersion()`.
    pub fn firmware_version(&self) -> u16 {
        self.joystick_view().map_or(0, |j| j.firmware_version())
    }

    /// The serial number of the gamepad. Translation of `SDL_GetGamepadSerial()`.
    pub fn serial(&self) -> Result<Option<String>> {
        self.joystick_view()?.serial()
    }

    /// The Steam Input handle of the gamepad, 0 if unavailable.
    /// Translation of `SDL_GetGamepadSteamHandle()`.
    pub fn steam_handle(&self) -> u64 {
        self.joystick_view()
            .and_then(|j| j.with(|j| j.steam_handle))
            .unwrap_or(0)
    }

    /// The connection state of the gamepad.
    /// Translation of `SDL_GetGamepadConnectionState()`.
    pub fn connection_state(&self) -> JoystickConnectionState {
        self.joystick_view()
            .map_or(JoystickConnectionState::Invalid, |j| j.connection_state())
    }

    /// The battery state of the gamepad and its charge (0 to 100, or -1).
    /// Translation of `SDL_GetGamepadPowerInfo()`.
    pub fn power_info(&self) -> (PowerState, i32) {
        self.joystick_view()
            .map_or((PowerState::Error, -1), |j| j.power_info())
    }

    /// Whether the gamepad is still attached. Translation of `SDL_GamepadConnected()`.
    pub fn connected(&self) -> bool {
        self.joystick_view().is_ok_and(|j| j.connected())
    }

    /// The SDL joystick layer bindings for this gamepad.
    /// Translation of `SDL_GetGamepadBindings()`.
    pub fn bindings(&self) -> Result<Vec<GamepadBinding>> {
        self.with(|g| g.bindings.clone())
    }

    /// The current mapping of the gamepad. Translation of `SDL_GetGamepadMapping()`.
    pub fn mapping(&self) -> Result<String> {
        let _lock = lock_joysticks();
        let mapping = self.with(|g| g.mapping)?;
        let guid = self.joystick_view()?.guid();
        let (_, name, mapping) =
            mapping_info(mapping).ok_or_else(|| Error::invalid_param("gamepad"))?;
        Ok(create_mapping_string(&name, &mapping, guid))
    }

    /// Start a rumble effect on the gamepad. Translation of `SDL_RumbleGamepad()`.
    pub fn rumble(
        &self,
        low_frequency_rumble: u16,
        high_frequency_rumble: u16,
        duration_ms: u32,
    ) -> Result<()> {
        self.joystick_view()?
            .rumble(low_frequency_rumble, high_frequency_rumble, duration_ms)
    }

    /// Start a rumble effect in the gamepad's triggers.
    /// Translation of `SDL_RumbleGamepadTriggers()`.
    pub fn rumble_triggers(
        &self,
        left_rumble: u16,
        right_rumble: u16,
        duration_ms: u32,
    ) -> Result<()> {
        self.joystick_view()?
            .rumble_triggers(left_rumble, right_rumble, duration_ms)
    }

    /// Update the gamepad's LED color. Translation of `SDL_SetGamepadLED()`.
    pub fn set_led(&self, red: u8, green: u8, blue: u8) -> Result<()> {
        self.joystick_view()?.set_led(red, green, blue)
    }

    /// Send a gamepad specific effect packet. Translation of `SDL_SendGamepadEffect()`.
    pub fn send_effect(&self, data: &[u8]) -> Result<()> {
        self.joystick_view()?.send_effect(data)
    }

    /// The Apple SF Symbols name for a button (only available with the
    /// Apple MFI driver). Translation of `SDL_GetGamepadAppleSFSymbolsNameForButton()`.
    pub fn apple_sf_symbols_name_for_button(&self, _button: GamepadButton) -> Option<String> {
        None
    }

    /// The Apple SF Symbols name for an axis (only available with the Apple
    /// MFI driver). Translation of `SDL_GetGamepadAppleSFSymbolsNameForAxis()`.
    pub fn apple_sf_symbols_name_for_axis(&self, _axis: GamepadAxis) -> Option<String> {
        None
    }
}

impl Drop for Gamepad {
    fn drop(&mut self) {
        close_gamepad(self.instance_id, self.serial);
    }
}

/// A handle to an open joystick that doesn't own a reference.
fn joystick_view((instance_id, serial): (JoystickID, u64)) -> ManuallyDrop<Joystick> {
    ManuallyDrop::new(Joystick {
        instance_id,
        serial,
    })
}

/// Translation of `SDL_CloseGamepad()`.
fn close_gamepad(instance_id: JoystickID, serial: u64) {
    let _lock = lock_joysticks();

    // First decrement ref count
    let gamepad = with_gamepads(|s| {
        let i = s
            .gamepads
            .iter()
            .position(|g| g.instance_id == instance_id && g.serial == serial)?;
        s.gamepads[i].ref_count -= 1;
        if s.gamepads[i].ref_count > 0 {
            return None;
        }
        // (unlink this entry)
        Some(s.gamepads.remove(i))
    });

    // (closes the joystick)
    drop(gamepad);
}

/// Translation of `SDL_QuitGamepads()`.
pub(crate) fn quit_gamepads() {
    let _lock = lock_joysticks();

    let open: Vec<JoystickID> =
        with_gamepads(|s| s.gamepads.iter().map(|g| g.instance_id).collect());
    for id in open {
        private_gamepad_removed(id);
    }

    GAMEPADS_INITIALIZED.store(false, Ordering::Relaxed);

    let watch = with_gamepads(|s| s.watch.take());
    drop(watch);

    while let Some((id, serial)) = with_gamepads(|s| {
        s.gamepads.first_mut().map(|g| {
            g.ref_count = 1;
            (g.instance_id, g.serial)
        })
    }) {
        close_gamepad(id, serial);
    }

    with_gamepads(|s| s.names = None);
}

/// Translation of `SDL_QuitGamepadMappings()`.
pub(crate) fn quit_gamepad_mappings() {
    assert_joysticks_locked();

    let supported = with_mappings(|m| {
        // (upstream leaves the default and XInput mapping pointers dangling)
        m.default_mapping = None;
        m.xinput_mapping = None;
        std::mem::take(&mut m.supported)
    });
    drop(supported);

    ALLOWED_GAMEPADS.free();
    IGNORED_GAMEPADS.free();

    with_mappings(|m| m.instance_ids = None);
}

/// Translation of `SDL_SendGamepadAxis()`.
fn send_gamepad_axis(timestamp: u64, gamepad: JoystickID, axis: GamepadAxis, value: i16) {
    assert_joysticks_locked();

    // translate the event, if desired
    if queue::event_enabled(EventType::GAMEPAD_AXIS_MOTION) {
        let _ = queue::push(Event::GamepadAxis(GamepadAxisEvent {
            timestamp: Duration::from_nanos(timestamp),
            which: gamepad,
            axis: axis as u8,
            value,
        }));
    }
}

/// Translation of `SDL_SendGamepadButton()`.
fn send_gamepad_button(timestamp: u64, gamepad: JoystickID, button: GamepadButton, down: bool) {
    assert_joysticks_locked();

    if button == GamepadButton::Invalid {
        return;
    }

    let event_type = if down {
        EventType::GAMEPAD_BUTTON_DOWN
    } else {
        EventType::GAMEPAD_BUTTON_UP
    };

    if button == GamepadButton::Guide {
        let now = timer::ticks_ms();
        if down {
            with_gamepad(gamepad, |g| g.guide_button_down = now);

            if with_joystick(gamepad, |j| j.delayed_guide_button) == Some(true) {
                // Skip duplicate press
                return;
            }
        } else {
            let guide_button_down = with_gamepad(gamepad, |g| g.guide_button_down).unwrap_or(0);
            if now < guide_button_down + MINIMUM_GUIDE_BUTTON_DELAY_MS {
                with_joystick(gamepad, |j| j.delayed_guide_button = true);
                return;
            }
            with_joystick(gamepad, |j| j.delayed_guide_button = false);
        }
    }

    // translate the event, if desired
    if queue::event_enabled(event_type) {
        let _ = queue::push(Event::GamepadButton(GamepadButtonEvent {
            timestamp: Duration::from_nanos(timestamp),
            which: gamepad,
            button: button as u8,
            down,
        }));
    }
}

/// Translation of `SDL_gamepad_event_list`.
const GAMEPAD_EVENT_LIST: [EventType; 12] = [
    EventType::GAMEPAD_AXIS_MOTION,
    EventType::GAMEPAD_BUTTON_DOWN,
    EventType::GAMEPAD_BUTTON_UP,
    EventType::GAMEPAD_ADDED,
    EventType::GAMEPAD_REMOVED,
    EventType::GAMEPAD_REMAPPED,
    EventType::GAMEPAD_TOUCHPAD_DOWN,
    EventType::GAMEPAD_TOUCHPAD_MOTION,
    EventType::GAMEPAD_TOUCHPAD_UP,
    EventType::GAMEPAD_SENSOR_UPDATE,
    EventType::GAMEPAD_CAPSENSE_TOUCH,
    EventType::GAMEPAD_CAPSENSE_RELEASE,
];

/// Enable or disable the gamepad events. Translation of `SDL_SetGamepadEventsEnabled()`.
pub fn set_gamepad_events_enabled(enabled: bool) {
    for event_type in GAMEPAD_EVENT_LIST {
        queue::set_event_enabled(event_type, enabled);
    }
}

/// Whether any gamepad events are enabled. Translation of `SDL_GamepadEventsEnabled()`.
pub fn gamepad_events_enabled() -> bool {
    GAMEPAD_EVENT_LIST.iter().any(|&t| queue::event_enabled(t))
}

/// Manually pump for gamepad updates. Translation of `SDL_UpdateGamepads()`.
pub fn update_gamepads() {
    // Just for API completeness; the joystick API does all the work.
    super::update_joysticks();
}

/// Send the delayed guide button release of a gamepad.
/// Translation of `SDL_GamepadHandleDelayedGuideButton()`.
pub(crate) fn gamepad_handle_delayed_guide_button(joystick: JoystickID) {
    assert_joysticks_locked();

    if with_gamepad(joystick, |_| ()).is_none() {
        return;
    }
    send_gamepad_button(0, joystick, GamepadButton::Guide, false);

    // Make sure we send an update complete event for this change
    with_joystick(joystick, |j| {
        if j.update_complete == 0 {
            j.update_complete = timer::ticks_ns();
        }
    });
}

#[cfg(test)]
mod tests;
