// Rust translation of src/events/ and include/SDL3/SDL_events.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Event queue management and input state.
//!
//! The [`Event`] enum is the translation of the `SDL_Event` union: one
//! variant per payload struct, each carrying its timestamp. Where C stores a
//! `const char *` into thread-local "temporary memory", the Rust event simply
//! owns a `String`.
//!
//! Submodules mirror upstream `src/events/`: [`queue`] (`SDL_events.c`),
//! [`keyboard`] (`SDL_keyboard.c`, `SDL_keymap.c`), [`mouse`], [`touch`],
//! [`pen`], and [`window`] (`SDL_windowevents.c` and the small
//! display/clipboard/drop/notification event sources).

use std::fmt;
use std::time::Duration;

pub mod keyboard;
pub mod mouse;
pub mod pen;
pub mod queue;
mod quit;
pub mod touch;
pub mod window;

pub use queue::*;

use crate::power::PowerState;
use keyboard::{Keycode, Keymod, Scancode};
use mouse::{MouseButtonFlags, MouseID, MouseWheelDirection};
use pen::{PenAxis, PenDeviceType, PenID, PenInputFlags};
use touch::{FingerID, TouchID};

/// A window's unique id (0 is never valid). Translation of `SDL_WindowID`.
pub type WindowID = u32;
/// A display's unique id (0 is never valid). Translation of `SDL_DisplayID`.
pub type DisplayID = u32;
/// A joystick instance id. Translation of `SDL_JoystickID`.
pub type JoystickID = u32;
/// An audio device id. Translation of `SDL_AudioDeviceID`.
pub type AudioDeviceID = u32;
/// A sensor instance id. Translation of `SDL_SensorID`.
pub type SensorID = u32;
/// A camera id. Translation of `SDL_CameraID`.
pub type CameraID = u32;
/// A notification id. Translation of `SDL_NotificationID`.
pub type NotificationID = u32;
/// A keyboard instance id. Translation of `SDL_KeyboardID`.
pub type KeyboardID = u32;

/// The types of events that can be delivered. Translation of `SDL_EventType`.
///
/// A newtype so that application-registered types ([`register_events`]) and
/// future/private values round-trip; the known values are associated consts.
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct EventType(pub u32);

macro_rules! event_types {
    ($($(#[$m:meta])* $name:ident = $val:expr;)*) => {
        impl EventType {
            $( $(#[$m])* pub const $name: EventType = EventType($val); )*
        }
        impl EventType {
            /// The `SDL_EVENT_*` constant name, or `None` for values that are
            /// not named constants (user and private events).
            pub fn constant_name(self) -> Option<&'static str> {
                $( if self == EventType::$name { return Some(concat!("SDL_EVENT_", stringify!($name))); } )*
                None
            }
        }
    };
}

event_types! {
    /// Unused (do not remove)
    FIRST = 0;
    /// User-requested quit
    QUIT = 0x100;
    /// The application is being terminated by the OS. Must be handled in an event watch.
    TERMINATING = 0x101;
    /// The application is low on memory, free memory if possible. Must be handled in an event watch.
    LOW_MEMORY = 0x102;
    /// The application is about to enter the background. Must be handled in an event watch.
    WILL_ENTER_BACKGROUND = 0x103;
    /// The application did enter the background and may not get CPU for some time. Must be handled in an event watch.
    DID_ENTER_BACKGROUND = 0x104;
    /// The application is about to enter the foreground. Must be handled in an event watch.
    WILL_ENTER_FOREGROUND = 0x105;
    /// The application is now interactive. Must be handled in an event watch.
    DID_ENTER_FOREGROUND = 0x106;
    /// The user's locale preferences have changed.
    LOCALE_CHANGED = 0x107;
    /// The system theme changed
    SYSTEM_THEME_CHANGED = 0x108;
    /// Display orientation has changed to data1
    DISPLAY_ORIENTATION = 0x151;
    /// Display has been added to the system
    DISPLAY_ADDED = 0x152;
    /// Display has been removed from the system
    DISPLAY_REMOVED = 0x153;
    /// Display has changed position
    DISPLAY_MOVED = 0x154;
    /// Display has changed desktop mode
    DISPLAY_DESKTOP_MODE_CHANGED = 0x155;
    /// Display has changed current mode
    DISPLAY_CURRENT_MODE_CHANGED = 0x156;
    /// Display has changed content scale
    DISPLAY_CONTENT_SCALE_CHANGED = 0x157;
    /// Display has changed usable bounds
    DISPLAY_USABLE_BOUNDS_CHANGED = 0x158;
    /// Window has been shown
    WINDOW_SHOWN = 0x202;
    /// Window has been hidden
    WINDOW_HIDDEN = 0x203;
    /// Window has been exposed and should be redrawn; data1 is 1 for live-resize expose events, 0 otherwise.
    WINDOW_EXPOSED = 0x204;
    /// Window has been moved to data1, data2
    WINDOW_MOVED = 0x205;
    /// Window has been resized to data1xdata2
    WINDOW_RESIZED = 0x206;
    /// The pixel size of the window has changed to data1xdata2
    WINDOW_PIXEL_SIZE_CHANGED = 0x207;
    /// The pixel size of a Metal view associated with the window has changed
    WINDOW_METAL_VIEW_RESIZED = 0x208;
    /// Window has been minimized
    WINDOW_MINIMIZED = 0x209;
    /// Window has been maximized
    WINDOW_MAXIMIZED = 0x20A;
    /// Window has been restored to normal size and position
    WINDOW_RESTORED = 0x20B;
    /// Window has gained mouse focus
    WINDOW_MOUSE_ENTER = 0x20C;
    /// Window has lost mouse focus
    WINDOW_MOUSE_LEAVE = 0x20D;
    /// Window has gained keyboard focus
    WINDOW_FOCUS_GAINED = 0x20E;
    /// Window has lost keyboard focus
    WINDOW_FOCUS_LOST = 0x20F;
    /// The window manager requests that the window be closed
    WINDOW_CLOSE_REQUESTED = 0x210;
    /// Window had a hit test that wasn't SDL_HITTEST_NORMAL
    WINDOW_HIT_TEST = 0x211;
    /// The window's ICC profile has changed
    WINDOW_ICCPROF_CHANGED = 0x212;
    /// Window has been moved to display data1
    WINDOW_DISPLAY_CHANGED = 0x213;
    /// Window display scale has been changed
    WINDOW_DISPLAY_SCALE_CHANGED = 0x214;
    /// The window safe area has been changed
    WINDOW_SAFE_AREA_CHANGED = 0x215;
    /// The window has been occluded
    WINDOW_OCCLUDED = 0x216;
    /// The window has entered fullscreen mode
    WINDOW_ENTER_FULLSCREEN = 0x217;
    /// The window has left fullscreen mode
    WINDOW_LEAVE_FULLSCREEN = 0x218;
    /// The window with the associated ID is being or has been destroyed.
    WINDOW_DESTROYED = 0x219;
    /// Window HDR properties have changed
    WINDOW_HDR_STATE_CHANGED = 0x21A;
    /// Window settings have changed (on visionOS)
    WINDOW_SETTINGS_CHANGED = 0x21B;
    /// Key pressed
    KEY_DOWN = 0x300;
    /// Key released
    KEY_UP = 0x301;
    /// Keyboard text editing (composition)
    TEXT_EDITING = 0x302;
    /// Keyboard text input
    TEXT_INPUT = 0x303;
    /// Keymap changed due to a system event such as an input language or keyboard layout change.
    KEYMAP_CHANGED = 0x304;
    /// A new keyboard has been inserted into the system
    KEYBOARD_ADDED = 0x305;
    /// A keyboard has been removed
    KEYBOARD_REMOVED = 0x306;
    /// Keyboard text editing candidates
    TEXT_EDITING_CANDIDATES = 0x307;
    /// The on-screen keyboard has been shown
    SCREEN_KEYBOARD_SHOWN = 0x308;
    /// The on-screen keyboard has been hidden
    SCREEN_KEYBOARD_HIDDEN = 0x309;
    /// Mouse moved
    MOUSE_MOTION = 0x400;
    /// Mouse button pressed
    MOUSE_BUTTON_DOWN = 0x401;
    /// Mouse button released
    MOUSE_BUTTON_UP = 0x402;
    /// Mouse wheel motion
    MOUSE_WHEEL = 0x403;
    /// A new mouse has been inserted into the system
    MOUSE_ADDED = 0x404;
    /// A mouse has been removed
    MOUSE_REMOVED = 0x405;
    /// Joystick axis motion
    JOYSTICK_AXIS_MOTION = 0x600;
    /// Joystick trackball motion
    JOYSTICK_BALL_MOTION = 0x601;
    /// Joystick hat position change
    JOYSTICK_HAT_MOTION = 0x602;
    /// Joystick button pressed
    JOYSTICK_BUTTON_DOWN = 0x603;
    /// Joystick button released
    JOYSTICK_BUTTON_UP = 0x604;
    /// A new joystick has been inserted into the system
    JOYSTICK_ADDED = 0x605;
    /// An opened joystick has been removed
    JOYSTICK_REMOVED = 0x606;
    /// Joystick battery level change
    JOYSTICK_BATTERY_UPDATED = 0x607;
    /// Joystick update is complete
    JOYSTICK_UPDATE_COMPLETE = 0x608;
    /// Gamepad axis motion
    GAMEPAD_AXIS_MOTION = 0x650;
    /// Gamepad button pressed
    GAMEPAD_BUTTON_DOWN = 0x651;
    /// Gamepad button released
    GAMEPAD_BUTTON_UP = 0x652;
    /// A new gamepad has been inserted into the system
    GAMEPAD_ADDED = 0x653;
    /// A gamepad has been removed
    GAMEPAD_REMOVED = 0x654;
    /// The gamepad mapping was updated
    GAMEPAD_REMAPPED = 0x655;
    /// Gamepad touchpad was touched
    GAMEPAD_TOUCHPAD_DOWN = 0x656;
    /// Gamepad touchpad finger was moved
    GAMEPAD_TOUCHPAD_MOTION = 0x657;
    /// Gamepad touchpad finger was lifted
    GAMEPAD_TOUCHPAD_UP = 0x658;
    /// Gamepad sensor was updated
    GAMEPAD_SENSOR_UPDATE = 0x659;
    /// Gamepad update is complete
    GAMEPAD_UPDATE_COMPLETE = 0x65A;
    /// Gamepad Steam handle has changed
    GAMEPAD_STEAM_HANDLE_UPDATED = 0x65B;
    /// Gamepad capsense was touched
    GAMEPAD_CAPSENSE_TOUCH = 0x65C;
    /// Gamepad capsense was released
    GAMEPAD_CAPSENSE_RELEASE = 0x65D;
    FINGER_DOWN = 0x700;
    FINGER_UP = 0x701;
    FINGER_MOTION = 0x702;
    FINGER_CANCELED = 0x703;
    /// Pinch gesture started
    PINCH_BEGIN = 0x710;
    /// Pinch gesture updated
    PINCH_UPDATE = 0x711;
    /// Pinch gesture ended
    PINCH_END = 0x712;
    /// The clipboard changed
    CLIPBOARD_UPDATE = 0x900;
    /// The system requests a file open
    DROP_FILE = 0x1000;
    /// text/plain drag-and-drop event
    DROP_TEXT = 0x1001;
    /// A new set of drops is beginning
    DROP_BEGIN = 0x1002;
    /// Current set of drops is now complete
    DROP_COMPLETE = 0x1003;
    /// Position while moving over the window
    DROP_POSITION = 0x1004;
    /// A new audio device is available
    AUDIO_DEVICE_ADDED = 0x1100;
    /// An audio device has been removed.
    AUDIO_DEVICE_REMOVED = 0x1101;
    /// An audio device's format has been changed by the system.
    AUDIO_DEVICE_FORMAT_CHANGED = 0x1102;
    /// A sensor was updated
    SENSOR_UPDATE = 0x1200;
    /// Pressure-sensitive pen has become available
    PEN_PROXIMITY_IN = 0x1300;
    /// Pressure-sensitive pen has become unavailable
    PEN_PROXIMITY_OUT = 0x1301;
    /// Pressure-sensitive pen touched drawing surface
    PEN_DOWN = 0x1302;
    /// Pressure-sensitive pen stopped touching drawing surface
    PEN_UP = 0x1303;
    /// Pressure-sensitive pen button pressed
    PEN_BUTTON_DOWN = 0x1304;
    /// Pressure-sensitive pen button released
    PEN_BUTTON_UP = 0x1305;
    /// Pressure-sensitive pen is moving on the tablet
    PEN_MOTION = 0x1306;
    /// Pressure-sensitive pen angle/pressure/etc changed
    PEN_AXIS = 0x1307;
    /// A new camera device is available
    CAMERA_DEVICE_ADDED = 0x1400;
    /// A camera device has been removed.
    CAMERA_DEVICE_REMOVED = 0x1401;
    /// A camera device has been approved for use by the user.
    CAMERA_DEVICE_APPROVED = 0x1402;
    /// A camera device has been denied for use by the user.
    CAMERA_DEVICE_DENIED = 0x1403;
    /// A user response to a system notification was received.
    NOTIFICATION_ACTION_INVOKED = 0x1500;
    /// The render targets have been reset and their contents need to be updated
    RENDER_TARGETS_RESET = 0x2000;
    /// The device has been reset and all textures need to be recreated
    RENDER_DEVICE_RESET = 0x2001;
    /// The device has been lost and can't be recovered.
    RENDER_DEVICE_LOST = 0x2002;
    PRIVATE0 = 0x4000;
    PRIVATE1 = 0x4001;
    PRIVATE2 = 0x4002;
    PRIVATE3 = 0x4003;
    /// Signals the end of an event poll cycle
    POLL_SENTINEL = 0x7F00;
    /// Events `USER` through `LAST` are for your use, and should be allocated with [`register_events`].
    USER = 0x8000;
    /// This last event is only for bounding internal arrays
    LAST = 0xFFFF;
}

impl EventType {
    pub const DISPLAY_FIRST: EventType = EventType::DISPLAY_ORIENTATION;
    pub const DISPLAY_LAST: EventType = EventType::DISPLAY_USABLE_BOUNDS_CHANGED;
    pub const WINDOW_FIRST: EventType = EventType::WINDOW_SHOWN;
    pub const WINDOW_LAST: EventType = EventType::WINDOW_SETTINGS_CHANGED;
    pub const KEYBOARD_FIRST: EventType = EventType::KEY_DOWN;
    pub const KEYBOARD_LAST: EventType = EventType::SCREEN_KEYBOARD_HIDDEN;
    pub const MOUSE_FIRST: EventType = EventType::MOUSE_MOTION;
    pub const MOUSE_LAST: EventType = EventType::MOUSE_REMOVED;
    pub const JOYSTICK_FIRST: EventType = EventType::JOYSTICK_AXIS_MOTION;
    pub const JOYSTICK_LAST: EventType = EventType::JOYSTICK_UPDATE_COMPLETE;
    pub const GAMEPAD_FIRST: EventType = EventType::GAMEPAD_AXIS_MOTION;
    pub const GAMEPAD_LAST: EventType = EventType::GAMEPAD_CAPSENSE_RELEASE;
    pub const FINGER_FIRST: EventType = EventType::FINGER_DOWN;
    pub const FINGER_LAST: EventType = EventType::FINGER_CANCELED;
    pub const PINCH_FIRST: EventType = EventType::PINCH_BEGIN;
    pub const PINCH_LAST: EventType = EventType::PINCH_END;
    pub const CLIPBOARD_FIRST: EventType = EventType::CLIPBOARD_UPDATE;
    pub const CLIPBOARD_LAST: EventType = EventType::CLIPBOARD_UPDATE;
    pub const DROP_FIRST: EventType = EventType::DROP_FILE;
    pub const DROP_LAST: EventType = EventType::DROP_POSITION;
    pub const AUDIO_DEVICE_FIRST: EventType = EventType::AUDIO_DEVICE_ADDED;
    pub const AUDIO_DEVICE_LAST: EventType = EventType::AUDIO_DEVICE_FORMAT_CHANGED;
    pub const SENSOR_FIRST: EventType = EventType::SENSOR_UPDATE;
    pub const SENSOR_LAST: EventType = EventType::SENSOR_UPDATE;
    pub const PEN_FIRST: EventType = EventType::PEN_PROXIMITY_IN;
    pub const PEN_LAST: EventType = EventType::PEN_AXIS;
    pub const CAMERA_DEVICE_FIRST: EventType = EventType::CAMERA_DEVICE_ADDED;
    pub const CAMERA_DEVICE_LAST: EventType = EventType::CAMERA_DEVICE_DENIED;
    pub const NOTIFICATION_FIRST: EventType = EventType::NOTIFICATION_ACTION_INVOKED;
    pub const NOTIFICATION_LAST: EventType = EventType::NOTIFICATION_ACTION_INVOKED;
    pub const RENDER_FIRST: EventType = EventType::RENDER_TARGETS_RESET;
    pub const RENDER_LAST: EventType = EventType::RENDER_DEVICE_LOST;

    /// The raw value.
    pub const fn as_u32(self) -> u32 {
        self.0
    }

    /// True if the value lies in `USER..=LAST`.
    pub const fn is_user(self) -> bool {
        self.0 >= EventType::USER.0 && self.0 <= EventType::LAST.0
    }

    /// The payload family this type belongs to. Translation of the internal `SDL_GetEventCategory()`.
    pub fn category(self) -> EventCategory {
        use EventCategory as C;
        let t = self;
        if t.is_user() {
            return C::User;
        }
        if t >= EventType::DISPLAY_FIRST && t <= EventType::DISPLAY_LAST {
            return C::Display;
        }
        if t >= EventType::WINDOW_FIRST && t <= EventType::WINDOW_LAST {
            return C::Window;
        }
        match t {
            EventType::KEYMAP_CHANGED
            | EventType::TERMINATING
            | EventType::LOW_MEMORY
            | EventType::WILL_ENTER_BACKGROUND
            | EventType::DID_ENTER_BACKGROUND
            | EventType::WILL_ENTER_FOREGROUND
            | EventType::DID_ENTER_FOREGROUND
            | EventType::LOCALE_CHANGED
            | EventType::SYSTEM_THEME_CHANGED
            | EventType::SCREEN_KEYBOARD_SHOWN
            | EventType::SCREEN_KEYBOARD_HIDDEN => C::System,
            EventType::RENDER_TARGETS_RESET
            | EventType::RENDER_DEVICE_RESET
            | EventType::RENDER_DEVICE_LOST => C::Render,
            EventType::QUIT => C::Quit,
            EventType::KEY_DOWN | EventType::KEY_UP => C::Key,
            EventType::TEXT_EDITING => C::Edit,
            EventType::TEXT_INPUT => C::Text,
            EventType::KEYBOARD_ADDED | EventType::KEYBOARD_REMOVED => C::KeyboardDevice,
            EventType::TEXT_EDITING_CANDIDATES => C::EditCandidates,
            EventType::MOUSE_MOTION => C::Motion,
            EventType::MOUSE_BUTTON_DOWN | EventType::MOUSE_BUTTON_UP => C::Button,
            EventType::MOUSE_WHEEL => C::Wheel,
            EventType::MOUSE_ADDED | EventType::MOUSE_REMOVED => C::MouseDevice,
            EventType::JOYSTICK_AXIS_MOTION => C::JoyAxis,
            EventType::JOYSTICK_BALL_MOTION => C::JoyBall,
            EventType::JOYSTICK_HAT_MOTION => C::JoyHat,
            EventType::JOYSTICK_BUTTON_DOWN | EventType::JOYSTICK_BUTTON_UP => C::JoyButton,
            EventType::JOYSTICK_ADDED
            | EventType::JOYSTICK_REMOVED
            | EventType::JOYSTICK_UPDATE_COMPLETE => C::JoyDevice,
            EventType::JOYSTICK_BATTERY_UPDATED => C::JoyBattery,
            EventType::GAMEPAD_AXIS_MOTION => C::GamepadAxis,
            EventType::GAMEPAD_BUTTON_DOWN | EventType::GAMEPAD_BUTTON_UP => C::GamepadButton,
            EventType::GAMEPAD_ADDED
            | EventType::GAMEPAD_REMOVED
            | EventType::GAMEPAD_REMAPPED
            | EventType::GAMEPAD_UPDATE_COMPLETE
            | EventType::GAMEPAD_STEAM_HANDLE_UPDATED => C::GamepadDevice,
            EventType::GAMEPAD_TOUCHPAD_DOWN
            | EventType::GAMEPAD_TOUCHPAD_MOTION
            | EventType::GAMEPAD_TOUCHPAD_UP => C::GamepadTouchpad,
            EventType::GAMEPAD_SENSOR_UPDATE => C::GamepadSensor,
            EventType::GAMEPAD_CAPSENSE_TOUCH | EventType::GAMEPAD_CAPSENSE_RELEASE => {
                C::GamepadCapSense
            }
            EventType::FINGER_DOWN
            | EventType::FINGER_UP
            | EventType::FINGER_CANCELED
            | EventType::FINGER_MOTION => C::TouchFinger,
            EventType::PINCH_BEGIN | EventType::PINCH_UPDATE | EventType::PINCH_END => C::Pinch,
            EventType::CLIPBOARD_UPDATE => C::Clipboard,
            EventType::DROP_FILE
            | EventType::DROP_TEXT
            | EventType::DROP_BEGIN
            | EventType::DROP_COMPLETE
            | EventType::DROP_POSITION => C::Drop,
            EventType::AUDIO_DEVICE_ADDED
            | EventType::AUDIO_DEVICE_REMOVED
            | EventType::AUDIO_DEVICE_FORMAT_CHANGED => C::AudioDevice,
            EventType::SENSOR_UPDATE => C::Sensor,
            EventType::PEN_PROXIMITY_IN | EventType::PEN_PROXIMITY_OUT => C::PenProximity,
            EventType::PEN_DOWN | EventType::PEN_UP => C::PenTouch,
            EventType::PEN_BUTTON_DOWN | EventType::PEN_BUTTON_UP => C::PenButton,
            EventType::PEN_MOTION => C::PenMotion,
            EventType::PEN_AXIS => C::PenAxis,
            EventType::CAMERA_DEVICE_ADDED
            | EventType::CAMERA_DEVICE_REMOVED
            | EventType::CAMERA_DEVICE_APPROVED
            | EventType::CAMERA_DEVICE_DENIED => C::CameraDevice,
            EventType::NOTIFICATION_ACTION_INVOKED => C::Notification,
            EventType::POLL_SENTINEL => C::PollSentinel,
            _ => C::Unknown,
        }
    }
}

impl fmt::Debug for EventType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.constant_name() {
            Some(n) => write!(f, "{n}"),
            None if self.is_user() => write!(f, "SDL_EVENT_USER+{}", self.0 - EventType::USER.0),
            None => write!(f, "EventType({:#x})", self.0),
        }
    }
}

/// Payload families. Translation of the internal `SDL_EventCategory`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[non_exhaustive]
pub enum EventCategory {
    Unknown,
    System,
    Display,
    Window,
    KeyboardDevice,
    Key,
    Edit,
    EditCandidates,
    Text,
    MouseDevice,
    Motion,
    Button,
    Wheel,
    JoyDevice,
    JoyAxis,
    JoyBall,
    JoyHat,
    JoyButton,
    JoyBattery,
    GamepadDevice,
    GamepadAxis,
    GamepadButton,
    GamepadTouchpad,
    GamepadSensor,
    GamepadCapSense,
    AudioDevice,
    CameraDevice,
    Sensor,
    Quit,
    User,
    TouchFinger,
    PenProximity,
    PenTouch,
    PenMotion,
    PenButton,
    PenAxis,
    Drop,
    Clipboard,
    Render,
    Notification,
    Pinch,
    PollSentinel,
}

/// Fields shared by every event. Translation of `SDL_CommonEvent`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct CommonEvent {
    pub event_type: EventType,
    /// Time since the tick counter started ([`timer::ticks`](crate::timer::ticks)); filled in at push if zero.
    pub timestamp: Duration,
}

/// Display state change event data. Translation of `SDL_DisplayEvent`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct DisplayEvent {
    /// One of `EventType::DISPLAY_*`
    pub event_type: EventType,
    pub timestamp: Duration,
    /// The associated display
    pub display_id: DisplayID,
    /// event dependent data
    pub data1: i32,
    /// event dependent data
    pub data2: i32,
}

/// Window state change event data. Translation of `SDL_WindowEvent`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct WindowEvent {
    /// One of `EventType::WINDOW_*`
    pub event_type: EventType,
    pub timestamp: Duration,
    /// The associated window
    pub window_id: WindowID,
    pub data1: i32,
    pub data2: i32,
}

/// Keyboard device event data. Translation of `SDL_KeyboardDeviceEvent`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct KeyboardDeviceEvent {
    /// `KEYBOARD_ADDED` or `KEYBOARD_REMOVED`
    pub event_type: EventType,
    pub timestamp: Duration,
    /// The keyboard instance id
    pub which: KeyboardID,
}

/// Keyboard button event structure. Translation of `SDL_KeyboardEvent`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct KeyboardEvent {
    pub timestamp: Duration,
    /// The window with keyboard focus, if any
    pub window_id: WindowID,
    /// The keyboard instance id, or 0 if unknown or virtual
    pub which: KeyboardID,
    /// SDL physical key code
    pub scancode: Scancode,
    /// SDL virtual key code
    pub key: Keycode,
    /// current key modifiers
    pub modifiers: Keymod,
    /// The platform dependent scancode for this event
    pub raw: u16,
    /// true if the key is pressed
    pub down: bool,
    /// true if this is a key repeat
    pub repeat: bool,
}

/// Keyboard IME composition event. Translation of `SDL_TextEditingEvent`.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct TextEditingEvent {
    pub timestamp: Duration,
    /// The window with keyboard focus, if any
    pub window_id: WindowID,
    /// The editing text
    pub text: String,
    /// The start cursor of selected editing text, or -1 if not set
    pub start: i32,
    /// The length of selected editing text, or -1 if not set
    pub length: i32,
}

/// Keyboard IME candidates event. Translation of `SDL_TextEditingCandidatesEvent`.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct TextEditingCandidatesEvent {
    pub timestamp: Duration,
    pub window_id: WindowID,
    /// The list of candidates (empty if none are available)
    pub candidates: Vec<String>,
    /// The index of the selected candidate, or -1 if no candidate is selected
    pub selected_candidate: i32,
    /// true if the list is horizontal, false if it's vertical
    pub horizontal: bool,
}

/// Keyboard text input event. Translation of `SDL_TextInputEvent`.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct TextInputEvent {
    pub timestamp: Duration,
    pub window_id: WindowID,
    /// The input text, UTF-8 encoded
    pub text: String,
}

/// Mouse device event. Translation of `SDL_MouseDeviceEvent`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct MouseDeviceEvent {
    /// `MOUSE_ADDED` or `MOUSE_REMOVED`
    pub event_type: EventType,
    pub timestamp: Duration,
    pub which: MouseID,
}

/// Mouse motion event. Translation of `SDL_MouseMotionEvent`.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct MouseMotionEvent {
    pub timestamp: Duration,
    /// The window with mouse focus, if any
    pub window_id: WindowID,
    /// The mouse instance id in relative mode, `TOUCH_MOUSE_ID` for touch events, `PEN_MOUSE_ID` for pen events, or 0
    pub which: MouseID,
    /// The current button state
    pub state: MouseButtonFlags,
    /// X coordinate, relative to window
    pub x: f32,
    /// Y coordinate, relative to window
    pub y: f32,
    /// The relative motion in the X direction
    pub xrel: f32,
    /// The relative motion in the Y direction
    pub yrel: f32,
}

/// Mouse button event. Translation of `SDL_MouseButtonEvent`.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct MouseButtonEvent {
    pub timestamp: Duration,
    pub window_id: WindowID,
    pub which: MouseID,
    /// The mouse button index
    pub button: u8,
    /// true if the button is pressed
    pub down: bool,
    /// 1 for single-click, 2 for double-click, etc.
    pub clicks: u8,
    pub x: f32,
    pub y: f32,
}

/// Mouse wheel event. Translation of `SDL_MouseWheelEvent`.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct MouseWheelEvent {
    pub timestamp: Duration,
    pub window_id: WindowID,
    pub which: MouseID,
    /// The amount scrolled horizontally, positive to the right and negative to the left
    pub x: f32,
    /// The amount scrolled vertically, positive away from the user and negative toward the user
    pub y: f32,
    /// When `Flipped` the values in X and Y will be opposite. Multiply by -1 to change them back
    pub direction: MouseWheelDirection,
    pub mouse_x: f32,
    pub mouse_y: f32,
    /// The amount scrolled horizontally, accumulated to whole scroll "ticks"
    pub integer_x: i32,
    /// The amount scrolled vertically, accumulated to whole scroll "ticks"
    pub integer_y: i32,
}

/// Joystick axis motion event. Translation of `SDL_JoyAxisEvent`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct JoyAxisEvent {
    pub timestamp: Duration,
    pub which: JoystickID,
    pub axis: u8,
    /// The axis value (range: -32768 to 32767)
    pub value: i16,
}

/// Joystick trackball motion event. Translation of `SDL_JoyBallEvent`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct JoyBallEvent {
    pub timestamp: Duration,
    pub which: JoystickID,
    pub ball: u8,
    pub xrel: i16,
    pub yrel: i16,
}

/// Joystick hat position change event. Translation of `SDL_JoyHatEvent`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct JoyHatEvent {
    pub timestamp: Duration,
    pub which: JoystickID,
    pub hat: u8,
    /// The hat position value (`SDL_HAT_*` bits).
    pub value: u8,
}

/// Joystick button event. Translation of `SDL_JoyButtonEvent`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct JoyButtonEvent {
    pub timestamp: Duration,
    pub which: JoystickID,
    pub button: u8,
    pub down: bool,
}

/// Joystick device event. Translation of `SDL_JoyDeviceEvent`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct JoyDeviceEvent {
    /// `JOYSTICK_ADDED`, `JOYSTICK_REMOVED` or `JOYSTICK_UPDATE_COMPLETE`
    pub event_type: EventType,
    pub timestamp: Duration,
    pub which: JoystickID,
}

/// Joystick battery level change event. Translation of `SDL_JoyBatteryEvent`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct JoyBatteryEvent {
    pub timestamp: Duration,
    pub which: JoystickID,
    pub state: PowerState,
    /// The joystick battery percent charge remaining
    pub percent: i32,
}

/// Gamepad axis motion event. Translation of `SDL_GamepadAxisEvent`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct GamepadAxisEvent {
    pub timestamp: Duration,
    pub which: JoystickID,
    /// The gamepad axis (`SDL_GamepadAxis`)
    pub axis: u8,
    pub value: i16,
}

/// Gamepad button event. Translation of `SDL_GamepadButtonEvent`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct GamepadButtonEvent {
    pub timestamp: Duration,
    pub which: JoystickID,
    /// The gamepad button (`SDL_GamepadButton`)
    pub button: u8,
    pub down: bool,
}

/// Gamepad device event. Translation of `SDL_GamepadDeviceEvent`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct GamepadDeviceEvent {
    /// `GAMEPAD_ADDED`, `GAMEPAD_REMOVED`, `GAMEPAD_REMAPPED`, `GAMEPAD_UPDATE_COMPLETE` or `GAMEPAD_STEAM_HANDLE_UPDATED`
    pub event_type: EventType,
    pub timestamp: Duration,
    pub which: JoystickID,
}

/// Gamepad touchpad event. Translation of `SDL_GamepadTouchpadEvent`.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct GamepadTouchpadEvent {
    /// `GAMEPAD_TOUCHPAD_DOWN`, `GAMEPAD_TOUCHPAD_MOTION` or `GAMEPAD_TOUCHPAD_UP`
    pub event_type: EventType,
    pub timestamp: Duration,
    pub which: JoystickID,
    pub touchpad: i32,
    pub finger: i32,
    /// Normalized in the range 0...1 with 0 being on the left
    pub x: f32,
    /// Normalized in the range 0...1 with 0 being at the top
    pub y: f32,
    /// Normalized in the range 0...1
    pub pressure: f32,
}

/// Gamepad sensor event. Translation of `SDL_GamepadSensorEvent`.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct GamepadSensorEvent {
    pub timestamp: Duration,
    pub which: JoystickID,
    /// The type of the sensor, one of the values of `SDL_SensorType`
    pub sensor: i32,
    /// Up to 3 values from the sensor
    pub data: [f32; 3],
    /// The timestamp of the sensor reading in nanoseconds, not necessarily synchronized with the system clock
    pub sensor_timestamp: u64,
}

/// Gamepad capacitive sensing event. Translation of `SDL_GamepadCapSenseEvent`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct GamepadCapSenseEvent {
    pub timestamp: Duration,
    pub which: JoystickID,
    /// The capsense type (`SDL_GamepadCapSenseType`)
    pub capsense: u8,
    /// true if the capsense is touched
    pub down: bool,
}

/// Audio device event. Translation of `SDL_AudioDeviceEvent`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct AudioDeviceEvent {
    /// `AUDIO_DEVICE_ADDED`, `AUDIO_DEVICE_REMOVED` or `AUDIO_DEVICE_FORMAT_CHANGED`
    pub event_type: EventType,
    pub timestamp: Duration,
    pub which: AudioDeviceID,
    /// false if a playback device, true if a recording device.
    pub recording: bool,
}

/// Camera device event. Translation of `SDL_CameraDeviceEvent`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct CameraDeviceEvent {
    /// `CAMERA_DEVICE_ADDED`, `CAMERA_DEVICE_REMOVED`, `CAMERA_DEVICE_APPROVED` or `CAMERA_DEVICE_DENIED`
    pub event_type: EventType,
    pub timestamp: Duration,
    pub which: CameraID,
}

/// Notification action event. Translation of `SDL_NotificationEvent`.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct NotificationEvent {
    pub timestamp: Duration,
    /// The ID of the notification that generated this event.
    pub which: NotificationID,
    /// The identifier string of the action invoked in the notification dialog.
    pub action_id: String,
}

/// Renderer event. Translation of `SDL_RenderEvent`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct RenderEvent {
    /// `RENDER_TARGETS_RESET`, `RENDER_DEVICE_RESET` or `RENDER_DEVICE_LOST`
    pub event_type: EventType,
    pub timestamp: Duration,
    /// The window containing the renderer in question.
    pub window_id: WindowID,
}

/// Touch finger event. Translation of `SDL_TouchFingerEvent`.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct TouchFingerEvent {
    /// `FINGER_DOWN`, `FINGER_UP`, `FINGER_MOTION` or `FINGER_CANCELED`
    pub event_type: EventType,
    pub timestamp: Duration,
    pub touch_id: TouchID,
    pub finger_id: FingerID,
    /// Normalized in the range 0...1
    pub x: f32,
    /// Normalized in the range 0...1
    pub y: f32,
    /// Normalized in the range -1...1
    pub dx: f32,
    /// Normalized in the range -1...1
    pub dy: f32,
    /// Normalized in the range 0...1
    pub pressure: f32,
    /// The window underneath the finger, if any
    pub window_id: WindowID,
}

/// Pinch gesture event. Translation of `SDL_PinchFingerEvent`.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct PinchFingerEvent {
    /// `PINCH_BEGIN`, `PINCH_UPDATE` or `PINCH_END`
    pub event_type: EventType,
    pub timestamp: Duration,
    /// The scale change since the last `PINCH_UPDATE`. Scale < 1 is "zoom out". Scale > 1 is "zoom in".
    pub scale: f32,
    pub window_id: WindowID,
    /// Average X distance between the pointers in window coordinates, or -1.
    pub span_x: f32,
    pub span_y: f32,
    /// X coordinate of the gesture's focal point in window coordinates, or -1.
    pub focus_x: f32,
    pub focus_y: f32,
}

/// Pen proximity event. Translation of `SDL_PenProximityEvent`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PenProximityEvent {
    /// `PEN_PROXIMITY_IN` or `PEN_PROXIMITY_OUT`
    pub event_type: EventType,
    pub timestamp: Duration,
    pub window_id: WindowID,
    pub which: PenID,
    /// Complete pen input state at time of event.
    pub pen_state: PenInputFlags,
    pub device_type: PenDeviceType,
}

/// Pen motion event. Translation of `SDL_PenMotionEvent`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct PenMotionEvent {
    pub timestamp: Duration,
    pub window_id: WindowID,
    pub which: PenID,
    pub pen_state: PenInputFlags,
    pub x: f32,
    pub y: f32,
    pub device_type: PenDeviceType,
}

/// Pen touch event. Translation of `SDL_PenTouchEvent`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct PenTouchEvent {
    pub timestamp: Duration,
    pub window_id: WindowID,
    pub which: PenID,
    pub pen_state: PenInputFlags,
    pub x: f32,
    pub y: f32,
    /// true if eraser end is used (not all pens support this).
    pub eraser: bool,
    /// true if the pen is touching or false if the pen is lifted off
    pub down: bool,
    pub device_type: PenDeviceType,
}

/// Pen button event. Translation of `SDL_PenButtonEvent`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct PenButtonEvent {
    pub timestamp: Duration,
    pub window_id: WindowID,
    pub which: PenID,
    pub pen_state: PenInputFlags,
    pub x: f32,
    pub y: f32,
    /// The pen button index (first button is 1).
    pub button: u8,
    pub down: bool,
    pub device_type: PenDeviceType,
}

/// Pen axis event. Translation of `SDL_PenAxisEvent`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct PenAxisEvent {
    pub timestamp: Duration,
    pub window_id: WindowID,
    pub which: PenID,
    pub pen_state: PenInputFlags,
    pub x: f32,
    pub y: f32,
    /// Axis that has changed
    pub axis: PenAxis,
    /// New value of axis
    pub value: f32,
    pub device_type: PenDeviceType,
}

/// Drag and drop event. Translation of `SDL_DropEvent`.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct DropEvent {
    /// `DROP_BEGIN`, `DROP_FILE`, `DROP_TEXT`, `DROP_COMPLETE` or `DROP_POSITION`
    pub event_type: EventType,
    pub timestamp: Duration,
    /// The window that was dropped on, if any
    pub window_id: WindowID,
    /// X coordinate, relative to window (not on begin)
    pub x: f32,
    pub y: f32,
    /// The source app that sent this drop event, if available
    pub source: Option<String>,
    /// The text for `DROP_TEXT` and the file name for `DROP_FILE`, `None` for other events
    pub data: Option<String>,
}

/// Clipboard event. Translation of `SDL_ClipboardEvent`.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct ClipboardEvent {
    pub timestamp: Duration,
    /// are we owning the clipboard (internal update)
    pub owner: bool,
    /// current mime types
    pub mime_types: Vec<String>,
}

/// Sensor event. Translation of `SDL_SensorEvent`.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct SensorEvent {
    pub timestamp: Duration,
    pub which: SensorID,
    /// Up to 6 values from the sensor
    pub data: [f32; 6],
    pub sensor_timestamp: u64,
}

/// A user-defined event. Translation of `SDL_UserEvent`.
///
/// `data1`/`data2` are `void *` in C; here any shared `Any` value.
#[derive(Clone, Default)]
pub struct UserEvent {
    /// `USER` through `LAST`; allocate with [`register_events`].
    pub event_type: EventType,
    pub timestamp: Duration,
    /// The associated window if any
    pub window_id: WindowID,
    /// User defined event code
    pub code: i32,
    pub data1: Option<std::sync::Arc<dyn std::any::Any + Send + Sync>>,
    pub data2: Option<std::sync::Arc<dyn std::any::Any + Send + Sync>>,
}

impl fmt::Debug for UserEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UserEvent")
            .field("event_type", &self.event_type)
            .field("timestamp", &self.timestamp)
            .field("window_id", &self.window_id)
            .field("code", &self.code)
            .field("data1", &self.data1.as_ref().map(std::sync::Arc::as_ptr))
            .field("data2", &self.data2.as_ref().map(std::sync::Arc::as_ptr))
            .finish()
    }
}

impl PartialEq for UserEvent {
    fn eq(&self, o: &Self) -> bool {
        self.event_type == o.event_type
            && self.timestamp == o.timestamp
            && self.window_id == o.window_id
            && self.code == o.code
            && match (&self.data1, &o.data1) {
                (Some(a), Some(b)) => std::sync::Arc::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            }
            && match (&self.data2, &o.data2) {
                (Some(a), Some(b)) => std::sync::Arc::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            }
    }
}

/// An SDL event. Translation of the `SDL_Event` union.
#[derive(Clone, PartialEq, Debug)]
#[non_exhaustive]
pub enum Event {
    /// `QUIT`
    Quit(CommonEvent),
    /// Application lifecycle and system events (`TERMINATING`, `LOW_MEMORY`,
    /// `*_BACKGROUND`, `*_FOREGROUND`, `LOCALE_CHANGED`, `SYSTEM_THEME_CHANGED`,
    /// `KEYMAP_CHANGED`, `SCREEN_KEYBOARD_*`) — payload is just the type and timestamp.
    App(CommonEvent),
    Display(DisplayEvent),
    Window(WindowEvent),
    KeyboardDevice(KeyboardDeviceEvent),
    Key(KeyboardEvent),
    TextEditing(TextEditingEvent),
    TextEditingCandidates(TextEditingCandidatesEvent),
    TextInput(TextInputEvent),
    MouseDevice(MouseDeviceEvent),
    MouseMotion(MouseMotionEvent),
    MouseButton(MouseButtonEvent),
    MouseWheel(MouseWheelEvent),
    JoyDevice(JoyDeviceEvent),
    JoyAxis(JoyAxisEvent),
    JoyBall(JoyBallEvent),
    JoyHat(JoyHatEvent),
    JoyButton(JoyButtonEvent),
    JoyBattery(JoyBatteryEvent),
    GamepadDevice(GamepadDeviceEvent),
    GamepadAxis(GamepadAxisEvent),
    GamepadButton(GamepadButtonEvent),
    GamepadTouchpad(GamepadTouchpadEvent),
    GamepadSensor(GamepadSensorEvent),
    GamepadCapSense(GamepadCapSenseEvent),
    AudioDevice(AudioDeviceEvent),
    CameraDevice(CameraDeviceEvent),
    Sensor(SensorEvent),
    User(UserEvent),
    TouchFinger(TouchFingerEvent),
    Pinch(PinchFingerEvent),
    PenProximity(PenProximityEvent),
    PenTouch(PenTouchEvent),
    PenMotion(PenMotionEvent),
    PenButton(PenButtonEvent),
    PenAxis(PenAxisEvent),
    Render(RenderEvent),
    Drop(DropEvent),
    Clipboard(ClipboardEvent),
    Notification(NotificationEvent),
    /// `POLL_SENTINEL` (internal; never returned by [`poll`]/[`wait`]).
    PollSentinel(CommonEvent),
    /// Any other type (private platform events, unregistered user ranges).
    Other(CommonEvent),
}

impl Event {
    /// The event's type. Translation of `event->type`.
    pub fn event_type(&self) -> EventType {
        match self {
            Event::Quit(_) => EventType::QUIT,
            Event::App(c) | Event::PollSentinel(c) | Event::Other(c) => c.event_type,
            Event::Display(e) => e.event_type,
            Event::Window(e) => e.event_type,
            Event::KeyboardDevice(e) => e.event_type,
            Event::Key(e) => {
                if e.down {
                    EventType::KEY_DOWN
                } else {
                    EventType::KEY_UP
                }
            }
            Event::TextEditing(_) => EventType::TEXT_EDITING,
            Event::TextEditingCandidates(_) => EventType::TEXT_EDITING_CANDIDATES,
            Event::TextInput(_) => EventType::TEXT_INPUT,
            Event::MouseDevice(e) => e.event_type,
            Event::MouseMotion(_) => EventType::MOUSE_MOTION,
            Event::MouseButton(e) => {
                if e.down {
                    EventType::MOUSE_BUTTON_DOWN
                } else {
                    EventType::MOUSE_BUTTON_UP
                }
            }
            Event::MouseWheel(_) => EventType::MOUSE_WHEEL,
            Event::JoyDevice(e) => e.event_type,
            Event::JoyAxis(_) => EventType::JOYSTICK_AXIS_MOTION,
            Event::JoyBall(_) => EventType::JOYSTICK_BALL_MOTION,
            Event::JoyHat(_) => EventType::JOYSTICK_HAT_MOTION,
            Event::JoyButton(e) => {
                if e.down {
                    EventType::JOYSTICK_BUTTON_DOWN
                } else {
                    EventType::JOYSTICK_BUTTON_UP
                }
            }
            Event::JoyBattery(_) => EventType::JOYSTICK_BATTERY_UPDATED,
            Event::GamepadDevice(e) => e.event_type,
            Event::GamepadAxis(_) => EventType::GAMEPAD_AXIS_MOTION,
            Event::GamepadButton(e) => {
                if e.down {
                    EventType::GAMEPAD_BUTTON_DOWN
                } else {
                    EventType::GAMEPAD_BUTTON_UP
                }
            }
            Event::GamepadTouchpad(e) => e.event_type,
            Event::GamepadSensor(_) => EventType::GAMEPAD_SENSOR_UPDATE,
            Event::GamepadCapSense(e) => {
                if e.down {
                    EventType::GAMEPAD_CAPSENSE_TOUCH
                } else {
                    EventType::GAMEPAD_CAPSENSE_RELEASE
                }
            }
            Event::AudioDevice(e) => e.event_type,
            Event::CameraDevice(e) => e.event_type,
            Event::Sensor(_) => EventType::SENSOR_UPDATE,
            Event::User(e) => e.event_type,
            Event::TouchFinger(e) => e.event_type,
            Event::Pinch(e) => e.event_type,
            Event::PenProximity(e) => e.event_type,
            Event::PenTouch(e) => {
                if e.down {
                    EventType::PEN_DOWN
                } else {
                    EventType::PEN_UP
                }
            }
            Event::PenMotion(_) => EventType::PEN_MOTION,
            Event::PenButton(e) => {
                if e.down {
                    EventType::PEN_BUTTON_DOWN
                } else {
                    EventType::PEN_BUTTON_UP
                }
            }
            Event::PenAxis(_) => EventType::PEN_AXIS,
            Event::Render(e) => e.event_type,
            Event::Drop(e) => e.event_type,
            Event::Clipboard(_) => EventType::CLIPBOARD_UPDATE,
            Event::Notification(_) => EventType::NOTIFICATION_ACTION_INVOKED,
        }
    }

    /// The event's timestamp (time since the tick counter started).
    pub fn timestamp(&self) -> Duration {
        *self.timestamp_ref()
    }

    /// Mutable access to the timestamp (used by [`push`] to stamp events).
    pub fn timestamp_mut(&mut self) -> &mut Duration {
        match self {
            Event::Quit(c) | Event::App(c) | Event::PollSentinel(c) | Event::Other(c) => {
                &mut c.timestamp
            }
            Event::Display(e) => &mut e.timestamp,
            Event::Window(e) => &mut e.timestamp,
            Event::KeyboardDevice(e) => &mut e.timestamp,
            Event::Key(e) => &mut e.timestamp,
            Event::TextEditing(e) => &mut e.timestamp,
            Event::TextEditingCandidates(e) => &mut e.timestamp,
            Event::TextInput(e) => &mut e.timestamp,
            Event::MouseDevice(e) => &mut e.timestamp,
            Event::MouseMotion(e) => &mut e.timestamp,
            Event::MouseButton(e) => &mut e.timestamp,
            Event::MouseWheel(e) => &mut e.timestamp,
            Event::JoyDevice(e) => &mut e.timestamp,
            Event::JoyAxis(e) => &mut e.timestamp,
            Event::JoyBall(e) => &mut e.timestamp,
            Event::JoyHat(e) => &mut e.timestamp,
            Event::JoyButton(e) => &mut e.timestamp,
            Event::JoyBattery(e) => &mut e.timestamp,
            Event::GamepadDevice(e) => &mut e.timestamp,
            Event::GamepadAxis(e) => &mut e.timestamp,
            Event::GamepadButton(e) => &mut e.timestamp,
            Event::GamepadTouchpad(e) => &mut e.timestamp,
            Event::GamepadSensor(e) => &mut e.timestamp,
            Event::GamepadCapSense(e) => &mut e.timestamp,
            Event::AudioDevice(e) => &mut e.timestamp,
            Event::CameraDevice(e) => &mut e.timestamp,
            Event::Sensor(e) => &mut e.timestamp,
            Event::User(e) => &mut e.timestamp,
            Event::TouchFinger(e) => &mut e.timestamp,
            Event::Pinch(e) => &mut e.timestamp,
            Event::PenProximity(e) => &mut e.timestamp,
            Event::PenTouch(e) => &mut e.timestamp,
            Event::PenMotion(e) => &mut e.timestamp,
            Event::PenButton(e) => &mut e.timestamp,
            Event::PenAxis(e) => &mut e.timestamp,
            Event::Render(e) => &mut e.timestamp,
            Event::Drop(e) => &mut e.timestamp,
            Event::Clipboard(e) => &mut e.timestamp,
            Event::Notification(e) => &mut e.timestamp,
        }
    }

    fn timestamp_ref(&self) -> &Duration {
        // SAFETY-free trick: reuse the mutable accessor through a shared clone is
        // not possible, so mirror the match.
        match self {
            Event::Quit(c) | Event::App(c) | Event::PollSentinel(c) | Event::Other(c) => {
                &c.timestamp
            }
            Event::Display(e) => &e.timestamp,
            Event::Window(e) => &e.timestamp,
            Event::KeyboardDevice(e) => &e.timestamp,
            Event::Key(e) => &e.timestamp,
            Event::TextEditing(e) => &e.timestamp,
            Event::TextEditingCandidates(e) => &e.timestamp,
            Event::TextInput(e) => &e.timestamp,
            Event::MouseDevice(e) => &e.timestamp,
            Event::MouseMotion(e) => &e.timestamp,
            Event::MouseButton(e) => &e.timestamp,
            Event::MouseWheel(e) => &e.timestamp,
            Event::JoyDevice(e) => &e.timestamp,
            Event::JoyAxis(e) => &e.timestamp,
            Event::JoyBall(e) => &e.timestamp,
            Event::JoyHat(e) => &e.timestamp,
            Event::JoyButton(e) => &e.timestamp,
            Event::JoyBattery(e) => &e.timestamp,
            Event::GamepadDevice(e) => &e.timestamp,
            Event::GamepadAxis(e) => &e.timestamp,
            Event::GamepadButton(e) => &e.timestamp,
            Event::GamepadTouchpad(e) => &e.timestamp,
            Event::GamepadSensor(e) => &e.timestamp,
            Event::GamepadCapSense(e) => &e.timestamp,
            Event::AudioDevice(e) => &e.timestamp,
            Event::CameraDevice(e) => &e.timestamp,
            Event::Sensor(e) => &e.timestamp,
            Event::User(e) => &e.timestamp,
            Event::TouchFinger(e) => &e.timestamp,
            Event::Pinch(e) => &e.timestamp,
            Event::PenProximity(e) => &e.timestamp,
            Event::PenTouch(e) => &e.timestamp,
            Event::PenMotion(e) => &e.timestamp,
            Event::PenButton(e) => &e.timestamp,
            Event::PenAxis(e) => &e.timestamp,
            Event::Render(e) => &e.timestamp,
            Event::Drop(e) => &e.timestamp,
            Event::Clipboard(e) => &e.timestamp,
            Event::Notification(e) => &e.timestamp,
        }
    }

    /// The window associated with the event, if the event type carries one.
    /// Translation of `SDL_GetWindowFromEvent()` (which then looks the window up).
    pub fn window_id(&self) -> Option<WindowID> {
        let id = match self {
            Event::User(e) => e.window_id,
            Event::Window(e) => e.window_id,
            Event::Key(e) => e.window_id,
            Event::TextEditing(e) => e.window_id,
            Event::TextInput(e) => e.window_id,
            Event::TextEditingCandidates(e) => e.window_id,
            Event::MouseMotion(e) => e.window_id,
            Event::MouseButton(e) => e.window_id,
            Event::MouseWheel(e) => e.window_id,
            Event::TouchFinger(e) => e.window_id,
            Event::PenProximity(e) => e.window_id,
            Event::PenTouch(e) => e.window_id,
            Event::PenButton(e) => e.window_id,
            Event::PenMotion(e) => e.window_id,
            Event::PenAxis(e) => e.window_id,
            Event::Drop(e) => e.window_id,
            Event::Render(e) => e.window_id,
            Event::Pinch(e) => e.window_id,
            _ => return None,
        };
        (id != 0).then_some(id)
    }

    /// A one-line description of the event (the same text `SDL_GetEventDescription()`
    /// produces for event logging).
    pub fn description(&self) -> String {
        let ts = self.timestamp().as_nanos();
        let name = match self.event_type().constant_name() {
            Some(n) => n.to_owned(),
            None if self.event_type().is_user() => {
                let plus = self.event_type().0 - EventType::USER.0;
                if plus > 0 {
                    format!("SDL_EVENT_USER+{plus}")
                } else {
                    "SDL_EVENT_USER".to_owned()
                }
            }
            None => "UNKNOWN".to_owned(),
        };
        let pressed = |d: bool| if d { "pressed" } else { "released" };
        let details = match self {
            Event::Quit(_) => format!(" (timestamp={ts})"),
            Event::App(_) => String::new(),
            Event::Other(c) if c.event_type == EventType::FIRST => " (THIS IS PROBABLY A BUG!)".to_owned(),
            Event::Other(c) => format!(" {:#x}", c.event_type.0),
            Event::PollSentinel(_) => String::new(),
            Event::Render(e) => format!(" (timestamp={ts} event={name} windowid={})", e.window_id),
            Event::Display(e) => format!(" (timestamp={ts} display={} event={name} data1={}, data2={})", e.display_id, e.data1, e.data2),
            Event::Window(e) => format!(" (timestamp={ts} windowid={} event={name} data1={} data2={})", e.window_id, e.data1, e.data2),
            Event::KeyboardDevice(e) => format!(" (timestamp={ts} which={})", e.which),
            Event::Key(e) => format!(
                " (timestamp={ts} windowid={} which={} state={} repeat={} scancode={} keycode={} mod={:#x})",
                e.window_id, e.which, pressed(e.down), e.repeat, e.scancode.0, e.key.0, e.modifiers.0
            ),
            Event::TextEditing(e) => format!(" (timestamp={ts} windowid={} text='{}' start={} length={})", e.window_id, e.text, e.start, e.length),
            Event::TextEditingCandidates(e) => format!(
                " (timestamp={ts} windowid={} num_candidates={} selected_candidate={})",
                e.window_id,
                e.candidates.len(),
                e.selected_candidate
            ),
            Event::TextInput(e) => format!(" (timestamp={ts} windowid={} text='{}')", e.window_id, e.text),
            Event::MouseDevice(e) => format!(" (timestamp={ts} which={})", e.which),
            Event::MouseMotion(e) => format!(
                " (timestamp={ts} windowid={} which={} state={} x={} y={} xrel={} yrel={})",
                e.window_id, e.which, e.state.0, e.x, e.y, e.xrel, e.yrel
            ),
            Event::MouseButton(e) => format!(
                " (timestamp={ts} windowid={} which={} button={} state={} clicks={} x={} y={})",
                e.window_id,
                e.which,
                e.button,
                pressed(e.down),
                e.clicks,
                e.x,
                e.y
            ),
            Event::MouseWheel(e) => format!(
                " (timestamp={ts} windowid={} which={} x={} y={} integer_x={} integer_y={} direction={})",
                e.window_id,
                e.which,
                e.x,
                e.y,
                e.integer_x,
                e.integer_y,
                if e.direction == MouseWheelDirection::Normal { "normal" } else { "flipped" }
            ),
            Event::JoyAxis(e) => format!(" (timestamp={ts} which={} axis={} value={})", e.which, e.axis, e.value),
            Event::JoyBall(e) => format!(" (timestamp={ts} which={} ball={} xrel={} yrel={})", e.which, e.ball, e.xrel, e.yrel),
            Event::JoyHat(e) => format!(" (timestamp={ts} which={} hat={} value={})", e.which, e.hat, e.value),
            Event::JoyButton(e) => format!(" (timestamp={ts} which={} button={} state={})", e.which, e.button, pressed(e.down)),
            Event::JoyBattery(e) => format!(" (timestamp={ts} which={} state={:?} percent={})", e.which, e.state, e.percent),
            Event::JoyDevice(e) => format!(" (timestamp={ts} which={})", e.which),
            Event::GamepadAxis(e) => format!(" (timestamp={ts} which={} axis={} value={})", e.which, e.axis, e.value),
            Event::GamepadButton(e) => format!(" (timestamp={ts} which={} button={} state={})", e.which, e.button, pressed(e.down)),
            Event::GamepadDevice(e) => format!(" (timestamp={ts} which={})", e.which),
            Event::GamepadTouchpad(e) => format!(
                " (timestamp={ts} which={} touchpad={} finger={} x={} y={} pressure={})",
                e.which, e.touchpad, e.finger, e.x, e.y, e.pressure
            ),
            Event::GamepadSensor(e) => format!(
                " (timestamp={ts} which={} sensor={} data[0]={} data[1]={} data[2]={})",
                e.which, e.sensor, e.data[0], e.data[1], e.data[2]
            ),
            Event::GamepadCapSense(e) => format!(
                " (timestamp={ts} which={} capsense={} state={})",
                e.which,
                e.capsense,
                if e.down { "touch" } else { "release" }
            ),
            Event::TouchFinger(e) => format!(
                " (timestamp={ts} touchid={} fingerid={} x={} y={} dx={} dy={} pressure={})",
                e.touch_id, e.finger_id, e.x, e.y, e.dx, e.dy, e.pressure
            ),
            Event::Pinch(e) => format!(" (timestamp={ts} scale={})", e.scale),
            Event::PenTouch(e) => format!(
                " (timestamp={ts} windowid={} which={} pen_state={} x={} y={} eraser={} state={})",
                e.window_id,
                e.which,
                e.pen_state.0,
                e.x,
                e.y,
                if e.eraser { "yes" } else { "no" },
                if e.down { "down" } else { "up" }
            ),
            Event::PenProximity(e) => format!(" (timestamp={ts} windowid={} which={})", e.window_id, e.which),
            Event::PenAxis(e) => format!(
                " (timestamp={ts} windowid={} which={} pen_state={} x={} y={} axis={} value={})",
                e.window_id,
                e.which,
                e.pen_state.0,
                e.x,
                e.y,
                e.axis.name(),
                e.value
            ),
            Event::PenMotion(e) => format!(
                " (timestamp={ts} windowid={} which={} pen_state={} x={} y={})",
                e.window_id, e.which, e.pen_state.0, e.x, e.y
            ),
            Event::PenButton(e) => format!(
                " (timestamp={ts} windowid={} which={} pen_state={} x={} y={} button={} state={})",
                e.window_id,
                e.which,
                e.pen_state.0,
                e.x,
                e.y,
                e.button,
                if e.down { "down" } else { "up" }
            ),
            Event::Drop(e) => format!(
                " (data='{}' timestamp={ts} windowid={} x={} y={})",
                e.data.as_deref().unwrap_or("(null)"),
                e.window_id,
                e.x,
                e.y
            ),
            Event::AudioDevice(e) => format!(" (timestamp={ts} which={} recording={})", e.which, e.recording),
            Event::CameraDevice(e) => format!(" (timestamp={ts} which={})", e.which),
            Event::Notification(e) => format!(" (timestamp={ts} which={} button_id='{}')", e.which, e.action_id),
            Event::Sensor(e) => format!(
                " (timestamp={ts} which={} data[0]={} data[1]={} data[2]={} data[3]={} data[4]={} data[5]={})",
                e.which, e.data[0], e.data[1], e.data[2], e.data[3], e.data[4], e.data[5]
            ),
            Event::User(e) => format!(
                " (timestamp={ts} windowid={} code={} data1={:?} data2={:?})",
                e.window_id,
                e.code,
                e.data1.as_ref().map(std::sync::Arc::as_ptr),
                e.data2.as_ref().map(std::sync::Arc::as_ptr)
            ),
            Event::Clipboard(_) => String::new(),
        };
        format!("{name}{details}")
    }

    /// Build the payload-less event for a type (quit, app/system events,
    /// sentinel, unknown). Used by the event sources.
    pub(crate) fn simple(event_type: EventType, timestamp: Duration) -> Event {
        let c = CommonEvent {
            event_type,
            timestamp,
        };
        match event_type.category() {
            EventCategory::Quit => Event::Quit(c),
            EventCategory::System => Event::App(c),
            EventCategory::PollSentinel => Event::PollSentinel(c),
            _ => Event::Other(c),
        }
    }
}

impl fmt::Display for Event {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.description())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn types_and_categories() {
        assert_eq!(EventType::QUIT.0, 0x100);
        assert_eq!(EventType::WINDOW_LAST.0, 0x21B);
        assert_eq!(EventType::GAMEPAD_LAST.0, 0x65D);
        assert_eq!(EventType::QUIT.category(), EventCategory::Quit);
        assert_eq!(EventType::WINDOW_MOVED.category(), EventCategory::Window);
        assert_eq!(EventType::DISPLAY_ADDED.category(), EventCategory::Display);
        assert_eq!(EventType(0x8005).category(), EventCategory::User);
        assert_eq!(EventType(0x4000).category(), EventCategory::Unknown);
        assert_eq!(format!("{:?}", EventType::KEY_DOWN), "SDL_EVENT_KEY_DOWN");
        assert_eq!(format!("{:?}", EventType(0x8002)), "SDL_EVENT_USER+2");
        assert!(EventType::USER.is_user() && !EventType::QUIT.is_user());
    }

    #[test]
    fn event_accessors() {
        let mut e = Event::Key(KeyboardEvent {
            down: true,
            window_id: 7,
            ..Default::default()
        });
        assert_eq!(e.event_type(), EventType::KEY_DOWN);
        assert_eq!(e.window_id(), Some(7));
        *e.timestamp_mut() = Duration::from_nanos(5);
        assert_eq!(e.timestamp(), Duration::from_nanos(5));
        assert!(e
            .description()
            .starts_with("SDL_EVENT_KEY_DOWN (timestamp=5 windowid=7"));
        let q = Event::simple(EventType::QUIT, Duration::ZERO);
        assert!(matches!(q, Event::Quit(_)));
        assert_eq!(q.window_id(), None);
        assert_eq!(
            Event::simple(EventType::TERMINATING, Duration::ZERO).event_type(),
            EventType::TERMINATING
        );
        assert_eq!(
            Event::simple(EventType::POLL_SENTINEL, Duration::ZERO).to_string(),
            "SDL_EVENT_POLL_SENTINEL"
        );
        let u = Event::User(UserEvent {
            event_type: EventType(0x8001),
            code: 3,
            ..Default::default()
        });
        assert!(u
            .description()
            .starts_with("SDL_EVENT_USER+1 (timestamp=0 windowid=0 code=3"));
        assert_eq!(
            Event::simple(EventType::FIRST, Duration::ZERO).description(),
            "SDL_EVENT_FIRST (THIS IS PROBABLY A BUG!)"
        );
    }
}
