// Rust translation of src/haptic/SDL_haptic.c, SDL_syshaptic.h,
// SDL_haptic_c.h and include/SDL3/SDL_haptic.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Haptic (force feedback) devices.
//!
//! A [`Haptic`] is an open haptic device; it is closed when the last handle
//! to it is dropped (upstream reference counts `SDL_OpenHaptic()` calls).
//! Devices are opened by instance ID ([`haptics`]), from a joystick
//! ([`Haptic::open_from_joystick`]) or from the mouse
//! ([`Haptic::open_from_mouse`]).
//!
//! The simple rumble API ([`Haptic::init_rumble`], [`Haptic::play_rumble`])
//! covers most uses; the effect API ([`Haptic::create_effect`] with a
//! [`HapticEffect`]) gives full control over force feedback.
//!
//! The haptic backends are platform drivers; so far the Linux driver (the
//! kernel's force feedback interface on evdev devices), the Windows driver
//! (DirectInput force feedback) and the dummy driver, which reports no
//! devices, exist. Where the HIDAPI joystick driver is (Linux and
//! Windows), the HIDAPI haptic drivers come first for its joysticks: the
//! Logitech wheels' force feedback (`haptic/hidapi`).

// (on Linux and Windows, only the tests use the dummy driver)
#[cfg_attr(any(target_os = "linux", windows), allow(dead_code))]
mod dummy;
#[cfg(any(target_os = "linux", windows))]
pub(crate) mod hidapi;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(windows)]
pub(crate) mod windows;

use std::any::Any;
use std::cell::RefCell;
use std::ops::{BitAnd, BitOr, BitOrAssign};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::error::{Error, Result};
use crate::hints;
use crate::joystick::{lock_joysticks, Joystick};
use crate::thread::ReentrantMutex;

/// The instance ID of a haptic device: unique and never reused while SDL
/// runs; zero is invalid. Translation of `SDL_HapticID`.
pub type HapticID = u32;

/// The index of an effect on a haptic device. Translation of
/// `SDL_HapticEffectID`.
pub type HapticEffectID = i32;

/// Run an effect forever (as the `iterations` of [`Haptic::run_effect`] or
/// the `length` of an effect). Translation of `SDL_HAPTIC_INFINITY`.
pub const HAPTIC_INFINITY: u32 = 4294967295;

/// The effects and features a haptic device supports (the `SDL_HAPTIC_*`
/// bits); the effect bits are also the types of the [`HapticEffect`]s.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct HapticFeatures(pub u32);

impl HapticFeatures {
    pub const NONE: HapticFeatures = HapticFeatures(0);
    /// A constant force applied in a direction. `SDL_HAPTIC_CONSTANT`.
    pub const CONSTANT: HapticFeatures = HapticFeatures(1 << 0);
    /// A periodic sine wave. `SDL_HAPTIC_SINE`.
    pub const SINE: HapticFeatures = HapticFeatures(1 << 1);
    /// A periodic square wave. `SDL_HAPTIC_SQUARE`.
    pub const SQUARE: HapticFeatures = HapticFeatures(1 << 2);
    /// A periodic triangle wave. `SDL_HAPTIC_TRIANGLE`.
    pub const TRIANGLE: HapticFeatures = HapticFeatures(1 << 3);
    /// A periodic upwards sawtooth wave. `SDL_HAPTIC_SAWTOOTHUP`.
    pub const SAWTOOTHUP: HapticFeatures = HapticFeatures(1 << 4);
    /// A periodic downwards sawtooth wave. `SDL_HAPTIC_SAWTOOTHDOWN`.
    pub const SAWTOOTHDOWN: HapticFeatures = HapticFeatures(1 << 5);
    /// A linear ramp of force. `SDL_HAPTIC_RAMP`.
    pub const RAMP: HapticFeatures = HapticFeatures(1 << 6);
    /// A spring, based on the axes' position. `SDL_HAPTIC_SPRING`.
    pub const SPRING: HapticFeatures = HapticFeatures(1 << 7);
    /// A damper, based on the axes' velocity. `SDL_HAPTIC_DAMPER`.
    pub const DAMPER: HapticFeatures = HapticFeatures(1 << 8);
    /// An inertia, based on the axes' acceleration. `SDL_HAPTIC_INERTIA`.
    pub const INERTIA: HapticFeatures = HapticFeatures(1 << 9);
    /// Friction, based on the axes' movement. `SDL_HAPTIC_FRICTION`.
    pub const FRICTION: HapticFeatures = HapticFeatures(1 << 10);
    /// Control of the large and small motors (as XInput has).
    /// `SDL_HAPTIC_LEFTRIGHT`.
    pub const LEFTRIGHT: HapticFeatures = HapticFeatures(1 << 11);
    /// Reserved for future use. `SDL_HAPTIC_RESERVED1`.
    pub const RESERVED1: HapticFeatures = HapticFeatures(1 << 12);
    /// Reserved for future use. `SDL_HAPTIC_RESERVED2`.
    pub const RESERVED2: HapticFeatures = HapticFeatures(1 << 13);
    /// Reserved for future use. `SDL_HAPTIC_RESERVED3`.
    pub const RESERVED3: HapticFeatures = HapticFeatures(1 << 14);
    /// A user-defined effect. `SDL_HAPTIC_CUSTOM`.
    pub const CUSTOM: HapticFeatures = HapticFeatures(1 << 15);
    /// The global gain can be set ([`Haptic::set_gain`]). `SDL_HAPTIC_GAIN`.
    pub const GAIN: HapticFeatures = HapticFeatures(1 << 16);
    /// Autocenter can be set ([`Haptic::set_autocenter`]).
    /// `SDL_HAPTIC_AUTOCENTER`.
    pub const AUTOCENTER: HapticFeatures = HapticFeatures(1 << 17);
    /// Effect status can be queried ([`Haptic::effect_status`]).
    /// `SDL_HAPTIC_STATUS`.
    pub const STATUS: HapticFeatures = HapticFeatures(1 << 18);
    /// The device can be paused ([`Haptic::pause`]). `SDL_HAPTIC_PAUSE`.
    pub const PAUSE: HapticFeatures = HapticFeatures(1 << 19);

    /// Whether every bit of `other` is set.
    pub const fn contains(self, other: HapticFeatures) -> bool {
        self.0 & other.0 == other.0
    }

    /// Whether any bit of `other` is set.
    pub const fn intersects(self, other: HapticFeatures) -> bool {
        self.0 & other.0 != 0
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

impl BitOr for HapticFeatures {
    type Output = HapticFeatures;
    fn bitor(self, rhs: HapticFeatures) -> HapticFeatures {
        HapticFeatures(self.0 | rhs.0)
    }
}

impl BitOrAssign for HapticFeatures {
    fn bitor_assign(&mut self, rhs: HapticFeatures) {
        self.0 |= rhs.0;
    }
}

impl BitAnd for HapticFeatures {
    type Output = HapticFeatures;
    fn bitand(self, rhs: HapticFeatures) -> HapticFeatures {
        HapticFeatures(self.0 & rhs.0)
    }
}

/// How a [`HapticDirection`] is encoded. Translation of
/// `SDL_HapticDirectionType`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum HapticDirectionType {
    /// Polar coordinates: `dir[0]` is the direction in hundredths of a
    /// degree, clockwise from north (0 = north, 9000 = east).
    /// `SDL_HAPTIC_POLAR`.
    #[default]
    Polar = 0,
    /// Cartesian coordinates: `dir` is a vector where positive X is east,
    /// positive Y south and positive Z up (the direction the force comes
    /// from). `SDL_HAPTIC_CARTESIAN`.
    Cartesian = 1,
    /// Spherical coordinates: `dir[0]` is the rotation from the positive
    /// X axis toward the positive Y axis and `dir[1]` the elevation, in
    /// hundredths of a degree. `SDL_HAPTIC_SPHERICAL`.
    Spherical = 2,
    /// Along the first axis only, whatever the device; for steering wheels.
    /// `SDL_HAPTIC_STEERING_AXIS`.
    SteeringAxis = 3,
}

/// The direction an effect comes from (see `SDL_haptic.h` for the
/// coordinate systems). Translation of `SDL_HapticDirection`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct HapticDirection {
    /// The type of encoding.
    pub kind: HapticDirectionType,
    /// The encoded direction.
    pub dir: [i32; 3],
}

/// A constant force: [`HapticFeatures::CONSTANT`]. Translation of
/// `SDL_HapticConstant`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct HapticConstant {
    /// Direction of the effect.
    pub direction: HapticDirection,
    /// Duration of the effect (ms).
    pub length: u32,
    /// Delay before starting the effect (ms).
    pub delay: u16,
    /// Button that triggers the effect.
    pub button: u16,
    /// How soon it can be triggered again after the button (ms).
    pub interval: u16,
    /// Strength of the constant effect.
    pub level: i16,
    /// Duration of the attack (ms).
    pub attack_length: u16,
    /// Level at the start of the attack.
    pub attack_level: u16,
    /// Duration of the fade (ms).
    pub fade_length: u16,
    /// Level at the end of the fade.
    pub fade_level: u16,
}

/// The wave shape of a [`HapticPeriodic`] effect.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum HapticWaveform {
    /// [`HapticFeatures::SINE`].
    #[default]
    Sine,
    /// [`HapticFeatures::SQUARE`].
    Square,
    /// [`HapticFeatures::TRIANGLE`].
    Triangle,
    /// [`HapticFeatures::SAWTOOTHUP`].
    SawtoothUp,
    /// [`HapticFeatures::SAWTOOTHDOWN`].
    SawtoothDown,
}

/// A periodic wave effect. Translation of `SDL_HapticPeriodic`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct HapticPeriodic {
    /// The wave shape (the effect type).
    pub waveform: HapticWaveform,
    /// Direction of the effect.
    pub direction: HapticDirection,
    /// Duration of the effect (ms).
    pub length: u32,
    /// Delay before starting the effect (ms).
    pub delay: u16,
    /// Button that triggers the effect.
    pub button: u16,
    /// How soon it can be triggered again after the button (ms).
    pub interval: u16,
    /// Period of the wave (ms).
    pub period: u16,
    /// Peak value; if negative, equivalent to 180 degrees extra phase shift.
    pub magnitude: i16,
    /// Mean value of the wave.
    pub offset: i16,
    /// Positive phase shift given by hundredth of a degree.
    pub phase: u16,
    /// Duration of the attack (ms).
    pub attack_length: u16,
    /// Level at the start of the attack.
    pub attack_level: u16,
    /// Duration of the fade (ms).
    pub fade_length: u16,
    /// Level at the end of the fade.
    pub fade_level: u16,
}

/// What a [`HapticCondition`] effect is based on.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum HapticConditionKind {
    /// The axes' position: [`HapticFeatures::SPRING`].
    #[default]
    Spring,
    /// The axes' velocity: [`HapticFeatures::DAMPER`].
    Damper,
    /// The axes' acceleration: [`HapticFeatures::INERTIA`].
    Inertia,
    /// The axes' movement: [`HapticFeatures::FRICTION`].
    Friction,
}

/// A condition effect, based on the state of the axes; the arrays hold a
/// value per axis. Translation of `SDL_HapticCondition`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct HapticCondition {
    /// What the effect is based on (the effect type).
    pub kind: HapticConditionKind,
    /// Direction of the effect.
    pub direction: HapticDirection,
    /// Duration of the effect (ms).
    pub length: u32,
    /// Delay before starting the effect (ms).
    pub delay: u16,
    /// Button that triggers the effect.
    pub button: u16,
    /// How soon it can be triggered again after the button (ms).
    pub interval: u16,
    /// Level when the joystick is to the positive side; max 0xFFFF.
    pub right_sat: [u16; 3],
    /// Level when the joystick is to the negative side; max 0xFFFF.
    pub left_sat: [u16; 3],
    /// How fast to increase the force towards the positive side.
    pub right_coeff: [i16; 3],
    /// How fast to increase the force towards the negative side.
    pub left_coeff: [i16; 3],
    /// Size of the dead zone; max 0xFFFF: whole axis-range when 0-centered.
    pub deadband: [u16; 3],
    /// Position of the dead zone.
    pub center: [i16; 3],
}

/// A linear ramp of force: [`HapticFeatures::RAMP`]. Translation of
/// `SDL_HapticRamp`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct HapticRamp {
    /// Direction of the effect.
    pub direction: HapticDirection,
    /// Duration of the effect (ms).
    pub length: u32,
    /// Delay before starting the effect (ms).
    pub delay: u16,
    /// Button that triggers the effect.
    pub button: u16,
    /// How soon it can be triggered again after the button (ms).
    pub interval: u16,
    /// Beginning strength level.
    pub start: i16,
    /// Ending strength level.
    pub end: i16,
    /// Duration of the attack (ms).
    pub attack_length: u16,
    /// Level at the start of the attack.
    pub attack_level: u16,
    /// Duration of the fade (ms).
    pub fade_length: u16,
    /// Level at the end of the fade.
    pub fade_level: u16,
}

/// Control of the large (low frequency) and small (high frequency) motors:
/// [`HapticFeatures::LEFTRIGHT`]. Translation of `SDL_HapticLeftRight`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct HapticLeftRight {
    /// Duration of the effect in milliseconds.
    pub length: u32,
    /// Control of the large controller motor.
    pub large_magnitude: u16,
    /// Control of the small controller motor.
    pub small_magnitude: u16,
}

/// A user-defined effect: [`HapticFeatures::CUSTOM`]. Translation of
/// `SDL_HapticCustom`.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Default)]
pub struct HapticCustom {
    /// Direction of the effect.
    pub direction: HapticDirection,
    /// Duration of the effect (ms).
    pub length: u32,
    /// Delay before starting the effect (ms).
    pub delay: u16,
    /// Button that triggers the effect.
    pub button: u16,
    /// How soon it can be triggered again after the button (ms).
    pub interval: u16,
    /// Axes to use, minimum of one.
    pub channels: u8,
    /// Sample periods.
    pub period: u16,
    /// Amount of samples.
    pub samples: u16,
    /// Should contain `channels * samples` items.
    pub data: Vec<u16>,
    /// Duration of the attack (ms).
    pub attack_length: u16,
    /// Level at the start of the attack.
    pub attack_level: u16,
    /// Duration of the fade (ms).
    pub fade_length: u16,
    /// Level at the end of the fade.
    pub fade_level: u16,
}

/// A haptic effect. Translation of the `SDL_HapticEffect` union, its
/// `type` field being the variant.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum HapticEffect {
    /// Constant effect.
    Constant(HapticConstant),
    /// Periodic effect.
    Periodic(HapticPeriodic),
    /// Condition effect.
    Condition(HapticCondition),
    /// Ramp effect.
    Ramp(HapticRamp),
    /// Left/Right effect.
    LeftRight(HapticLeftRight),
    /// Custom effect.
    Custom(HapticCustom),
}

impl HapticEffect {
    /// The effect type: the [`HapticFeatures`] bit a device must support to
    /// play the effect (the union's `type` field).
    pub fn effect_type(&self) -> HapticFeatures {
        match self {
            HapticEffect::Constant(_) => HapticFeatures::CONSTANT,
            HapticEffect::Periodic(p) => match p.waveform {
                HapticWaveform::Sine => HapticFeatures::SINE,
                HapticWaveform::Square => HapticFeatures::SQUARE,
                HapticWaveform::Triangle => HapticFeatures::TRIANGLE,
                HapticWaveform::SawtoothUp => HapticFeatures::SAWTOOTHUP,
                HapticWaveform::SawtoothDown => HapticFeatures::SAWTOOTHDOWN,
            },
            HapticEffect::Condition(c) => match c.kind {
                HapticConditionKind::Spring => HapticFeatures::SPRING,
                HapticConditionKind::Damper => HapticFeatures::DAMPER,
                HapticConditionKind::Inertia => HapticFeatures::INERTIA,
                HapticConditionKind::Friction => HapticFeatures::FRICTION,
            },
            HapticEffect::Ramp(_) => HapticFeatures::RAMP,
            HapticEffect::LeftRight(_) => HapticFeatures::LEFTRIGHT,
            HapticEffect::Custom(_) => HapticFeatures::CUSTOM,
        }
    }
}

/// An effect slot of a device. Translation of `struct haptic_effect`.
#[derive(Default)]
pub(crate) struct HapticEffectSlot {
    /// The current effect (`None` until one is created in the slot).
    pub(crate) effect: Option<HapticEffect>,
    /// The hardware behind the effect; `None` marks a free slot. Set by
    /// the driver's `new_effect`, cleared by its `destroy_effect`.
    pub(crate) hweffect: Option<Box<dyn Any + Send>>,
}

/// The state of an open haptic device. Translation of `struct SDL_Haptic`.
pub(crate) struct HapticData {
    /// Device instance, monotonically increasing from 0.
    pub(crate) instance_id: HapticID,
    /// Tells handles to different openings of the same instance apart.
    serial: u64,
    /// Device name - system dependent.
    pub(crate) name: Option<String>,
    /// Allocated effects; the driver sizes this to the maximum amount of
    /// effects when opening the device.
    pub(crate) effects: Vec<HapticEffectSlot>,
    /// Maximum amount of effects to play at the same time.
    pub(crate) nplaying: i32,
    /// Supported effects and features.
    pub(crate) supported: HapticFeatures,
    /// Number of axes on the device.
    pub(crate) naxes: i32,
    /// Driver dependent.
    #[cfg_attr(not(any(target_os = "linux", windows)), allow(dead_code))]
    // (used by the haptic drivers)
    pub(crate) hwdata: Option<Box<dyn Any + Send>>,
    /// Count for multiple opens.
    ref_count: i32,
    /// ID of rumble effect for simple rumble API.
    rumble_id: HapticEffectID,
    /// Rumble effect.
    rumble_effect: Option<HapticEffect>,
}

impl HapticData {
    fn new() -> HapticData {
        HapticData {
            instance_id: 0,
            serial: 0,
            name: None,
            effects: Vec::new(),
            nplaying: 0,
            supported: HapticFeatures::NONE,
            naxes: 0,
            hwdata: None,
            ref_count: 0,
            rumble_id: -1,
            rumble_effect: None,
        }
    }
}

/// The functions of a haptic backend: the `SDL_SYS_Haptic*` functions.
///
/// The functions that take a [`HapticData`] run with the device list
/// borrowed, so they must not call back into the haptic API.
pub(crate) trait HapticDriver: Send + Sync {
    /// Initialize the haptic subsystem (scan the system for devices).
    fn init(&self) -> Result<()>;

    /// The number of haptic devices plugged in right now.
    fn count(&self) -> usize;

    /// The instance ID of the haptic device at `index` (zero, with the
    /// error set, if there isn't one).
    fn instance_id(&self, index: usize) -> HapticID;

    /// The name of the haptic device at `index`.
    fn name(&self, index: usize) -> Option<String>;

    /// Open the device `haptic.instance_id`, filling in the effect slots,
    /// supported features and so on.
    fn open(&self, haptic: &mut HapticData) -> Result<()>;

    /// The index of the haptic core pointer (mouse), if it is haptic.
    fn mouse(&self) -> Option<usize>;

    /// Whether the joystick has haptic capabilities.
    fn joystick_is_haptic(&self, joystick: &Joystick) -> bool;

    /// Open the haptic device of a joystick; this should fill in the
    /// instance ID and name.
    fn open_from_joystick(&self, haptic: &mut HapticData, joystick: &Joystick) -> Result<()>;

    /// Whether the haptic device and the joystick are the same device.
    fn joystick_same_haptic(&self, haptic: &HapticData, joystick: &Joystick) -> bool;

    /// Close a haptic device after use.
    fn close(&self, haptic: &mut HapticData);

    /// Clean up the haptic subsystem.
    fn quit(&self);

    /// Create a new effect in the free slot `effect`.
    fn new_effect(&self, haptic: &mut HapticData, effect: usize, base: &HapticEffect)
        -> Result<()>;

    /// Update the effect in slot `effect`.
    fn update_effect(
        &self,
        haptic: &mut HapticData,
        effect: usize,
        data: &HapticEffect,
    ) -> Result<()>;

    /// Run the effect in slot `effect`.
    fn run_effect(&self, haptic: &mut HapticData, effect: usize, iterations: u32) -> Result<()>;

    /// Stop the effect in slot `effect`.
    fn stop_effect(&self, haptic: &mut HapticData, effect: usize) -> Result<()>;

    /// Clean up the effect in slot `effect`, freeing the slot.
    fn destroy_effect(&self, haptic: &mut HapticData, effect: usize);

    /// Whether the effect in slot `effect` is playing.
    fn effect_status(&self, haptic: &mut HapticData, effect: usize) -> Result<bool>;

    /// Set the gain (0 to 100).
    fn set_gain(&self, haptic: &mut HapticData, gain: i32) -> Result<()>;

    /// Set the autocentering (0 to 100).
    fn set_autocenter(&self, haptic: &mut HapticData, autocenter: i32) -> Result<()>;

    /// Pause the device.
    fn pause(&self, haptic: &mut HapticData) -> Result<()>;

    /// Unpause the device.
    fn resume(&self, haptic: &mut HapticData) -> Result<()>;

    /// Stop all the currently playing effects.
    fn stop_all(&self, haptic: &mut HapticData) -> Result<()>;
}

/// The haptic backend (`SDL_SYS_*`, chosen when upstream is built).
#[cfg(target_os = "linux")]
static DRIVER: &dyn HapticDriver = &linux::LINUX_HAPTIC_DRIVER;
/// The haptic backend (`SDL_SYS_*`, chosen when upstream is built).
#[cfg(windows)]
static DRIVER: &dyn HapticDriver = &windows::WINDOWS_HAPTIC_DRIVER;
/// The haptic backend (`SDL_SYS_*`, chosen when upstream is built).
#[cfg(not(any(target_os = "linux", windows)))]
static DRIVER: &dyn HapticDriver = &dummy::DUMMY_HAPTIC_DRIVER;

static NEXT_SERIAL: AtomicU64 = AtomicU64::new(1);

/// Translation of `SDL_haptics`: the open devices, most recently opened
/// first. (Upstream doesn't lock the list.)
static HAPTICS: ReentrantMutex<RefCell<Vec<HapticData>>> =
    ReentrantMutex::new(RefCell::new(Vec::new()));

/// Run `f` on the open devices.
fn with_haptics<R>(f: impl FnOnce(&mut Vec<HapticData>) -> R) -> R {
    let guard = HAPTICS.lock();
    let mut haptics = guard.borrow_mut();
    f(&mut haptics)
}

/// An entry of the [`hints::JOYSTICK_HAPTIC_AXES`] hint.
/// Translation of `SDL_Haptic_VIDPID_Naxes`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct VidPidNaxes {
    vid: u16,
    pid: u16,
    naxes: u16,
}

/// Parse an unsigned number the way `SDL_sscanf()`'s `%hx`/`%hu` do:
/// skip whitespace, scan, truncate to 16 bits. Returns the value and the
/// bytes consumed, or `None` if nothing was scanned.
fn scan_u16(text: &[u8], radix: u32) -> Option<(u16, usize)> {
    let skipped = text.iter().take_while(|c| c.is_ascii_whitespace()).count();
    let (value, used) = crate::stdlib::string::strtoul(&text[skipped..], radix);
    (used > 0).then_some((value as u16, skipped + used))
}

/// Parse the haptic axes hint: `0xVID/0xPID/NAXES` entries separated by
/// commas, stopping at the first one that doesn't scan
/// (`SDL_sscanf(spot, "0x%hx/0x%hx/%hu%n", ...)`).
/// Translation of `SDL_Haptic_Load_Axes_List()`.
fn parse_axes_list(hint: &str) -> Vec<VidPidNaxes> {
    let mut entries = Vec::new();
    let mut spot = hint.as_bytes();

    let scan = |text: &[u8]| -> Option<(VidPidNaxes, usize)> {
        let mut at = 0;
        at += text.get(at..)?.strip_prefix(b"0x").map(|_| 2)?;
        let (vid, used) = scan_u16(&text[at..], 16)?;
        at += used;
        at += text.get(at..)?.strip_prefix(b"/0x").map(|_| 3)?;
        let (pid, used) = scan_u16(&text[at..], 16)?;
        at += used;
        at += text.get(at..)?.strip_prefix(b"/").map(|_| 1)?;
        let (naxes, used) = scan_u16(&text[at..], 10)?;
        at += used;
        Some((VidPidNaxes { vid, pid, naxes }, at))
    };

    while let Some((entry, length)) = scan(spot) {
        crate::sdl_assert!(length > 0);
        spot = &spot[length..];
        entries.push(entry);

        if let Some(rest) = spot.strip_prefix(b",") {
            spot = rest;
        }
    }
    entries
}

/// The number of axes the [`hints::JOYSTICK_HAPTIC_AXES`] hint
/// gives a device, if any (an entry for `0xFFFF/0xFFFF` matches every
/// device). Translation of `SDL_Haptic_Get_Naxes()`.
fn hinted_naxes(vid: u16, pid: u16) -> Option<u16> {
    let list = parse_axes_list(&hints::get(hints::JOYSTICK_HAPTIC_AXES)?);
    let index = |vid: u16, pid: u16| list.iter().find(|e| e.vid == vid && e.pid == pid);

    let mut naxes = None;
    // Perform "wildcard" pass
    if let Some(entry) = index(0xffff, 0xffff) {
        naxes = Some(entry.naxes);
    }
    if let Some(entry) = index(vid, pid) {
        naxes = Some(entry.naxes);
    }
    naxes
}

/// Translation of `SDL_InitHaptics()` (`SDL_HIDAPI_HapticInit()` has
/// nothing to do, see [`hidapi`]).
pub(crate) fn init_haptics() -> Result<()> {
    DRIVER.init()
}

/// The driver index of the device `instance_id`. Translation of
/// `SDL_GetHapticIndex()`.
fn haptic_index(instance_id: HapticID) -> Result<usize> {
    if instance_id > 0 {
        let num_haptics = DRIVER.count();
        for device_index in 0..num_haptics {
            if DRIVER.instance_id(device_index) == instance_id {
                return Ok(device_index);
            }
        }
    }

    Err(Error::new(format!("Haptic device {instance_id} not found")))
}

/// The instance ID of the driver's haptic mouse, if it has one.
///
/// Upstream `SDL_OpenHapticFromMouse()` passes the device index from
/// `SDL_SYS_HapticMouse()` to `SDL_OpenHaptic()`, which expects an instance
/// ID; the index is turned into the device's instance ID here.
fn mouse_instance_id(driver: &dyn HapticDriver) -> Option<HapticID> {
    driver
        .mouse()
        .map(|device_index| driver.instance_id(device_index))
}

/// The haptic devices currently connected. Translation of
/// `SDL_GetHaptics()`.
pub fn haptics() -> Vec<HapticID> {
    let num_haptics = DRIVER.count();
    (0..num_haptics)
        .map(|device_index| {
            let id = DRIVER.instance_id(device_index);
            crate::sdl_assert!(id > 0);
            id
        })
        .collect()
}

/// The implementation dependent name of a haptic device, without opening
/// it. Translation of `SDL_GetHapticNameForID()`.
pub fn haptic_name_for_id(instance_id: HapticID) -> Result<Option<String>> {
    let device_index = haptic_index(instance_id)?;
    Ok(DRIVER.name(device_index))
}

/// Whether the mouse is haptic. Translation of `SDL_IsMouseHaptic()`.
pub fn is_mouse_haptic() -> bool {
    DRIVER.mouse().is_some()
}

/// Whether a joystick has haptic features: it must be a valid joystick
/// and (except under sdl2-compat) not a gamepad. Translation of
/// `SDL_IsJoystickHaptic()`.
pub fn is_joystick_haptic(joystick: &Joystick) -> bool {
    let _lock = lock_joysticks();

    // Must be a valid joystick, but not a gamepad unless running under sdl2-compat
    joystick.with(|_| ()).is_ok()
        && (hints::get_bool("SDL2_COMPAT", false)
            || !crate::joystick::gamepad::is_gamepad(joystick.id()))
        && (DRIVER.joystick_is_haptic(joystick) || hidapi_joystick_is_haptic(joystick))
}

/// `SDL_HIDAPI_JoystickIsHaptic()`, where there is the HIDAPI joystick
/// driver.
fn hidapi_joystick_is_haptic(joystick: &Joystick) -> bool {
    #[cfg(any(target_os = "linux", windows))]
    {
        hidapi::joystick_is_haptic(joystick)
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        let _ = joystick;
        false
    }
}

/// `SDL_HIDAPI_JoystickSameHaptic()`, where there is the HIDAPI joystick
/// driver.
fn hidapi_joystick_same_haptic(haptic: &HapticData, joystick: &Joystick) -> bool {
    #[cfg(any(target_os = "linux", windows))]
    {
        hidapi::joystick_same_haptic(haptic, joystick)
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        let _ = (haptic, joystick);
        false
    }
}

/// The settings applied to a newly opened device: autocenter off and gain
/// to max.
fn apply_open_defaults(haptic: &mut HapticData) {
    // Disable autocenter and set gain to max.
    if haptic.supported.intersects(HapticFeatures::GAIN) {
        let _ = set_gain(haptic, 100);
    }
    if haptic.supported.intersects(HapticFeatures::AUTOCENTER) {
        let _ = set_autocenter(haptic, 0);
    }
}

/// An open haptic device. Translation of `SDL_Haptic *`; dropping the last
/// handle closes the device (`SDL_CloseHaptic()`).
#[derive(Debug)]
pub struct Haptic {
    instance_id: HapticID,
    serial: u64,
}

impl Haptic {
    /// Open a haptic device for use. Opening a device that is already open
    /// adds a reference to it. Translation of `SDL_OpenHaptic()`.
    ///
    /// When the device supports them, the gain is set to the maximum and
    /// autocenter is disabled.
    pub fn open(instance_id: HapticID) -> Result<Haptic> {
        let device_index = haptic_index(instance_id)?;

        /* If the haptic device is already open, return it
         * it is important that we have a single haptic device for each instance id
         */
        if let Some(handle) = Haptic::from_id(instance_id) {
            return Ok(handle);
        }

        // Create the haptic device
        let mut haptic = HapticData::new();

        // Initialize the haptic device
        haptic.instance_id = instance_id;
        haptic.rumble_id = -1;
        DRIVER.open(&mut haptic)?;

        if haptic.name.is_none() {
            haptic.name = DRIVER.name(device_index);
        }

        Ok(Haptic::add(haptic))
    }

    /// Link a newly opened device into the list and apply the defaults.
    fn add(mut haptic: HapticData) -> Haptic {
        // Add haptic to list
        haptic.ref_count += 1;
        haptic.serial = NEXT_SERIAL.fetch_add(1, Ordering::Relaxed);
        let handle = Haptic {
            instance_id: haptic.instance_id,
            serial: haptic.serial,
        };
        // Link the haptic in the list
        with_haptics(|h| {
            h.insert(0, haptic);
            apply_open_defaults(&mut h[0]);
        });
        handle
    }

    /// Another handle to an open haptic device (adding a reference to it),
    /// or `None` if it isn't open. Translation of `SDL_GetHapticFromID()`.
    pub fn from_id(instance_id: HapticID) -> Option<Haptic> {
        with_haptics(|h| {
            h.iter_mut()
                .find(|h| h.instance_id == instance_id)
                .map(|h| {
                    h.ref_count += 1;
                    Haptic {
                        instance_id,
                        serial: h.serial,
                    }
                })
        })
    }

    /// Open the mouse as a haptic device. Translation of
    /// `SDL_OpenHapticFromMouse()`.
    pub fn open_from_mouse() -> Result<Haptic> {
        let Some(instance_id) = mouse_instance_id(DRIVER) else {
            return Err(Error::new("Haptic: Mouse isn't a haptic device."));
        };

        Haptic::open(instance_id)
    }

    /// Open the haptic device of a joystick (see [`is_joystick_haptic`]).
    /// The [`hints::JOYSTICK_HAPTIC_AXES`] hint can override the
    /// number of axes. Translation of `SDL_OpenHapticFromJoystick()`.
    pub fn open_from_joystick(joystick: &Joystick) -> Result<Haptic> {
        // (the device list is locked before the joysticks, the order of
        // every other path that takes both: a HIDAPI haptic's effect
        // thread waits for the joysticks with its device locked, which
        // the haptic functions lock with the list locked)
        let _haptics = HAPTICS.lock();
        let mut haptic = {
            let _lock = lock_joysticks();

            // Joystick must be valid and haptic
            if !is_joystick_haptic(joystick) {
                return Err(Error::new("Haptic: Joystick isn't a haptic device."));
            }

            // Check to see if joystick's haptic is already open
            let existing = with_haptics(|h| {
                h.iter_mut()
                    .find(|h| {
                        DRIVER.joystick_same_haptic(h, joystick)
                            || hidapi_joystick_same_haptic(h, joystick)
                    })
                    .map(|h| {
                        h.ref_count += 1;
                        Haptic {
                            instance_id: h.instance_id,
                            serial: h.serial,
                        }
                    })
            });
            if let Some(handle) = existing {
                return Ok(handle);
            }

            // Create the haptic device
            let mut haptic = HapticData::new();

            /* Initialize the haptic device
             * This function should fill in the instance ID and name.
             */
            haptic.rumble_id = -1;
            if hidapi_joystick_is_haptic(joystick) {
                #[cfg(any(target_os = "linux", windows))]
                if hidapi::open_from_joystick(&mut haptic, joystick).is_err() {
                    return Err(Error::new(
                        "Haptic: SDL_HIDAPI_HapticOpenFromJoystick failed.",
                    ));
                }
            } else if DRIVER.open_from_joystick(&mut haptic, joystick).is_err() {
                return Err(Error::new("Haptic: SDL_SYS_HapticOpenFromJoystick failed."));
            }
            crate::sdl_assert!(haptic.instance_id != 0);
            haptic
        };

        // Check if custom number of haptic axes was defined
        let vid = joystick.vendor();
        let pid = joystick.product();
        let general_axes = joystick.num_axes().map_or(-1, |n| n as i32);

        let naxes = hinted_naxes(vid, pid).map_or(-1, i32::from);
        if naxes > 0 {
            haptic.naxes = naxes;
        }

        // Limit to the actual number of axes found on the device
        if general_axes >= 0 && naxes > general_axes {
            haptic.naxes = general_axes;
        }

        Ok(Haptic::add(haptic))
    }

    /// Run `f` on this device's state (`CHECK_HAPTIC_MAGIC`).
    fn with<R>(&self, f: impl FnOnce(&mut HapticData) -> R) -> Result<R> {
        with_haptics(|h| {
            h.iter_mut()
                .find(|h| h.instance_id == self.instance_id && h.serial == self.serial)
                .map(f)
        })
        .ok_or_else(|| Error::invalid_param("haptic"))
    }

    /// The instance ID of the device. Translation of `SDL_GetHapticID()`.
    pub fn id(&self) -> HapticID {
        self.instance_id
    }

    /// The implementation dependent name of the device. Translation of
    /// `SDL_GetHapticName()`.
    pub fn name(&self) -> Result<Option<String>> {
        self.with(|h| h.name.clone())
    }

    /// The number of effects the device can store. (On some platforms this
    /// is an approximation; use [`Haptic::max_effects_playing`] to know how
    /// many can play at once.) Translation of `SDL_GetMaxHapticEffects()`.
    pub fn max_effects(&self) -> Result<usize> {
        self.with(|h| h.effects.len())
    }

    /// The number of effects the device can play at the same time.
    /// Translation of `SDL_GetMaxHapticEffectsPlaying()`.
    pub fn max_effects_playing(&self) -> Result<i32> {
        self.with(|h| h.nplaying)
    }

    /// The effects and features the device supports. Translation of
    /// `SDL_GetHapticFeatures()`.
    pub fn features(&self) -> Result<HapticFeatures> {
        self.with(|h| h.supported)
    }

    /// The number of axes of the device. Translation of
    /// `SDL_GetNumHapticAxes()`.
    pub fn num_axes(&self) -> Result<i32> {
        self.with(|h| h.naxes)
    }

    /// Whether the device supports an effect. Translation of
    /// `SDL_HapticEffectSupported()`.
    pub fn effect_supported(&self, effect: &HapticEffect) -> bool {
        self.with(|h| h.supported.intersects(effect.effect_type()))
            .unwrap_or(false)
    }

    /// Create a new effect on the device, returning its ID. Translation of
    /// `SDL_CreateHapticEffect()`.
    pub fn create_effect(&self, effect: &HapticEffect) -> Result<HapticEffectID> {
        self.with(|h| create_effect(h, effect))?
    }

    /// Update an effect; its type can't change. It can be updated while
    /// running, but the device might not support that. Translation of
    /// `SDL_UpdateHapticEffect()`.
    pub fn update_effect(&self, effect: HapticEffectID, data: &HapticEffect) -> Result<()> {
        self.with(|h| update_effect(h, effect, data))?
    }

    /// Run an effect `iterations` times ([`HAPTIC_INFINITY`]: forever); the
    /// effect's envelope is repeated each iteration. Translation of
    /// `SDL_RunHapticEffect()`.
    pub fn run_effect(&self, effect: HapticEffectID, iterations: u32) -> Result<()> {
        self.with(|h| run_effect(h, effect, iterations))?
    }

    /// Stop an effect. Translation of `SDL_StopHapticEffect()`.
    pub fn stop_effect(&self, effect: HapticEffectID) -> Result<()> {
        self.with(|h| stop_effect(h, effect))?
    }

    /// Stop and destroy an effect. Translation of
    /// `SDL_DestroyHapticEffect()`.
    pub fn destroy_effect(&self, effect: HapticEffectID) {
        let _ = self.with(|h| destroy_effect(h, effect));
    }

    /// Whether an effect is playing; the device must support
    /// [`HapticFeatures::STATUS`]. Translation of
    /// `SDL_GetHapticEffectStatus()`.
    pub fn effect_status(&self, effect: HapticEffectID) -> Result<bool> {
        self.with(|h| {
            #[cfg(any(target_os = "linux", windows))]
            if hidapi::is_hidapi(h) {
                return Ok(hidapi::effect_status(h, effect));
            }

            let index = valid_effect(h, effect)?;

            if !h.supported.intersects(HapticFeatures::STATUS) {
                return Err(Error::new(
                    "Haptic: Device does not support status queries.",
                ));
            }

            // (a backend error reads as "not playing", as upstream's `> 0`)
            Ok(DRIVER.effect_status(h, index).unwrap_or(false))
        })?
    }

    /// Set the global gain, 0 to 100; the device must support
    /// [`HapticFeatures::GAIN`]. The `SDL_HAPTIC_GAIN_MAX` environment
    /// variable scales it (to the percentage it gives). Translation of
    /// `SDL_SetHapticGain()`.
    pub fn set_gain(&self, gain: i32) -> Result<()> {
        self.with(|h| set_gain(h, gain))?
    }

    /// Set the global autocenter, 0 (off) to 100; the device must support
    /// [`HapticFeatures::AUTOCENTER`]. Translation of
    /// `SDL_SetHapticAutocenter()`.
    pub fn set_autocenter(&self, autocenter: i32) -> Result<()> {
        self.with(|h| set_autocenter(h, autocenter))?
    }

    /// Pause the device; the device must support [`HapticFeatures::PAUSE`].
    /// Effects can't be modified while paused. Translation of
    /// `SDL_PauseHaptic()`.
    pub fn pause(&self) -> Result<()> {
        self.with(|h| {
            if !h.supported.intersects(HapticFeatures::PAUSE) {
                return Err(Error::new(
                    "Haptic: Device does not support setting pausing.",
                ));
            }

            #[cfg(any(target_os = "linux", windows))]
            if hidapi::is_hidapi(h) {
                return hidapi::pause(h);
            }

            DRIVER.pause(h)
        })?
    }

    /// Resume a paused device. Translation of `SDL_ResumeHaptic()`.
    pub fn resume(&self) -> Result<()> {
        self.with(|h| {
            if !h.supported.intersects(HapticFeatures::PAUSE) {
                return Ok(()); // Not going to be paused, so we pretend it's unpaused.
            }

            #[cfg(any(target_os = "linux", windows))]
            if hidapi::is_hidapi(h) {
                return hidapi::resume(h);
            }

            DRIVER.resume(h)
        })?
    }

    /// Stop all the currently playing effects. Translation of
    /// `SDL_StopHapticEffects()`.
    pub fn stop_effects(&self) -> Result<()> {
        self.with(|h| {
            #[cfg(any(target_os = "linux", windows))]
            if hidapi::is_hidapi(h) {
                return hidapi::stop_all(h);
            }

            DRIVER.stop_all(h)
        })?
    }

    /// Whether rumble is supported. Translation of
    /// `SDL_HapticRumbleSupported()`.
    pub fn rumble_supported(&self) -> bool {
        // Most things can use SINE, but XInput only has LEFTRIGHT.
        self.with(|h| {
            h.supported
                .intersects(HapticFeatures::SINE | HapticFeatures::LEFTRIGHT)
        })
        .unwrap_or(false)
    }

    /// Initialize the device for simple rumble playback. Translation of
    /// `SDL_InitHapticRumble()`.
    pub fn init_rumble(&self) -> Result<()> {
        self.with(|h| {
            // Already allocated.
            if h.rumble_id >= 0 {
                return Ok(());
            }

            let efx = if h.supported.intersects(HapticFeatures::SINE) {
                HapticEffect::Periodic(HapticPeriodic {
                    waveform: HapticWaveform::Sine,
                    direction: HapticDirection {
                        kind: HapticDirectionType::Cartesian,
                        dir: [0; 3],
                    },
                    period: 1000,
                    magnitude: 0x4000,
                    length: 5000,
                    attack_length: 0,
                    fade_length: 0,
                    ..Default::default()
                })
            } else if h.supported.intersects(HapticFeatures::LEFTRIGHT) {
                // XInput?
                HapticEffect::LeftRight(HapticLeftRight {
                    length: 5000,
                    large_magnitude: 0x4000,
                    small_magnitude: 0x4000,
                })
            } else {
                h.rumble_effect = None;
                return Err(Error::new("Device doesn't support rumble"));
            };

            let result = create_effect(h, &efx);
            h.rumble_effect = Some(efx);
            h.rumble_id = result.as_ref().map_or(-1, |&id| id);
            result.map(|_| ())
        })?
    }

    /// Run a simple rumble effect: `strength` from 0.0 to 1.0 (clamped),
    /// for `length` milliseconds. Translation of `SDL_PlayHapticRumble()`.
    pub fn play_rumble(&self, strength: f32, length: u32) -> Result<()> {
        self.with(|h| {
            if h.rumble_id < 0 {
                return Err(Error::new(
                    "Haptic: Rumble effect not initialized on haptic device",
                ));
            }

            // Clamp strength.
            let strength = strength.clamp(0.0, 1.0);
            let magnitude = (32767.0f32 * strength) as i16;

            match &mut h.rumble_effect {
                Some(HapticEffect::Periodic(p)) => {
                    p.magnitude = magnitude;
                    p.length = length;
                }
                Some(HapticEffect::LeftRight(lr)) => {
                    lr.large_magnitude = magnitude as u16;
                    lr.small_magnitude = magnitude as u16;
                    lr.length = length;
                }
                _ => {
                    // "This should have been caught elsewhere"
                    crate::sdl_assert!(false);
                }
            }

            let rumble_id = h.rumble_id;
            let efx = h.rumble_effect.clone();
            if let Some(efx) = efx {
                update_effect(h, rumble_id, &efx)?;
            }

            run_effect(h, rumble_id, 1)
        })?
    }

    /// Stop the simple rumble. Translation of `SDL_StopHapticRumble()`.
    pub fn stop_rumble(&self) -> Result<()> {
        self.with(|h| {
            if h.rumble_id < 0 {
                return Err(Error::new(
                    "Haptic: Rumble effect not initialized on haptic device",
                ));
            }

            let rumble_id = h.rumble_id;
            stop_effect(h, rumble_id)
        })?
    }
}

impl Drop for Haptic {
    fn drop(&mut self) {
        close_haptic(self.instance_id, self.serial);
    }
}

/// Translation of `SDL_CreateHapticEffect()`.
fn create_effect(haptic: &mut HapticData, effect: &HapticEffect) -> Result<HapticEffectID> {
    // Check to see if effect is supported
    if !haptic.supported.intersects(effect.effect_type()) {
        return Err(Error::new("Haptic: Effect not supported by haptic device."));
    }

    #[cfg(any(target_os = "linux", windows))]
    if hidapi::is_hidapi(haptic) {
        return hidapi::new_effect(haptic, effect);
    }

    // See if there's a free slot
    let Some(i) = haptic.effects.iter().position(|e| e.hweffect.is_none()) else {
        return Err(Error::new("Haptic: Device has no free space left."));
    };

    // Now let the backend create the real effect
    DRIVER.new_effect(haptic, i, effect)?; // Backend failed to create effect

    haptic.effects[i].effect = Some(effect.clone());
    Ok(i as HapticEffectID)
}

/// The slot index of an effect ID. Translation of `ValidEffect()`.
fn valid_effect(haptic: &HapticData, effect: HapticEffectID) -> Result<usize> {
    if effect < 0 || effect as usize >= haptic.effects.len() {
        return Err(Error::new("Haptic: Invalid effect identifier."));
    }
    Ok(effect as usize)
}

/// Translation of `SDL_UpdateHapticEffect()`.
fn update_effect(
    haptic: &mut HapticData,
    effect: HapticEffectID,
    data: &HapticEffect,
) -> Result<()> {
    let index = valid_effect(haptic, effect)?;

    // Can't change type dynamically.
    let current_type = haptic.effects[index]
        .effect
        .as_ref()
        .map(HapticEffect::effect_type);
    if current_type != Some(data.effect_type()) {
        return Err(Error::new("Haptic: Updating effect type is illegal."));
    }

    #[cfg(any(target_os = "linux", windows))]
    if hidapi::is_hidapi(haptic) {
        return hidapi::update_effect(haptic, effect, data);
    }

    // Updates the effect
    DRIVER.update_effect(haptic, index, data)?;

    haptic.effects[index].effect = Some(data.clone());
    Ok(())
}

/// Translation of `SDL_RunHapticEffect()`.
fn run_effect(haptic: &mut HapticData, effect: HapticEffectID, iterations: u32) -> Result<()> {
    #[cfg(any(target_os = "linux", windows))]
    if hidapi::is_hidapi(haptic) {
        return hidapi::run_effect(haptic, effect, iterations);
    }

    let index = valid_effect(haptic, effect)?;

    // Run the effect
    DRIVER.run_effect(haptic, index, iterations)
}

/// Translation of `SDL_StopHapticEffect()`.
fn stop_effect(haptic: &mut HapticData, effect: HapticEffectID) -> Result<()> {
    #[cfg(any(target_os = "linux", windows))]
    if hidapi::is_hidapi(haptic) {
        return hidapi::stop_effect(haptic, effect);
    }

    let index = valid_effect(haptic, effect)?;

    // Stop the effect
    DRIVER.stop_effect(haptic, index)
}

/// Translation of `SDL_DestroyHapticEffect()`.
fn destroy_effect(haptic: &mut HapticData, effect: HapticEffectID) {
    #[cfg(any(target_os = "linux", windows))]
    if hidapi::is_hidapi(haptic) {
        hidapi::destroy_effect(haptic, effect);
        return;
    }

    let Ok(index) = valid_effect(haptic, effect) else {
        return;
    };

    // Not allocated
    if haptic.effects[index].hweffect.is_none() {
        return;
    }

    DRIVER.destroy_effect(haptic, index);
}

/// Translation of `SDL_SetHapticGain()`.
fn set_gain(haptic: &mut HapticData, gain: i32) -> Result<()> {
    if !haptic.supported.intersects(HapticFeatures::GAIN) {
        return Err(Error::new("Haptic: Device does not support setting gain."));
    }

    if !(0..=100).contains(&gain) {
        return Err(Error::new("Haptic: Gain must be between 0 and 100."));
    }

    // The user can use an environment variable to override the max gain.
    let real_gain = match crate::stdlib::getenv("SDL_HAPTIC_GAIN_MAX") {
        Some(env) => {
            // Check for sanity.
            let max_gain = crate::stdlib::atoi(&env).clamp(0, 100);

            // We'll scale it linearly with SDL_HAPTIC_GAIN_MAX
            (gain * max_gain) / 100
        }
        None => gain,
    };

    #[cfg(any(target_os = "linux", windows))]
    if hidapi::is_hidapi(haptic) {
        return hidapi::set_gain(haptic, real_gain);
    }

    DRIVER.set_gain(haptic, real_gain)
}

/// Translation of `SDL_SetHapticAutocenter()`.
fn set_autocenter(haptic: &mut HapticData, autocenter: i32) -> Result<()> {
    if !haptic.supported.intersects(HapticFeatures::AUTOCENTER) {
        return Err(Error::new(
            "Haptic: Device does not support setting autocenter.",
        ));
    }

    if !(0..=100).contains(&autocenter) {
        return Err(Error::new("Haptic: Autocenter must be between 0 and 100."));
    }

    #[cfg(any(target_os = "linux", windows))]
    if hidapi::is_hidapi(haptic) {
        return hidapi::set_autocenter(haptic, autocenter);
    }

    DRIVER.set_autocenter(haptic, autocenter)
}

/// Close the device unless other handles still reference it. Translation
/// of `SDL_CloseHaptic()`.
fn close_haptic(instance_id: HapticID, serial: u64) {
    with_haptics(|list| {
        let Some(i) = list
            .iter()
            .position(|h| h.instance_id == instance_id && h.serial == serial)
        else {
            return;
        };
        let haptic = &mut list[i];

        // Check if it's still in use
        haptic.ref_count -= 1;
        if haptic.ref_count > 0 {
            return;
        }

        close_device(haptic);

        // Remove from the list
        list.remove(i);
    });
}

/// The backend's part of `SDL_CloseHaptic()`.
fn close_device(haptic: &mut HapticData) {
    #[cfg(any(target_os = "linux", windows))]
    if hidapi::is_hidapi(haptic) {
        hidapi::close(haptic);
        return;
    }

    // Close it, properly removing effects if needed
    for effect in 0..haptic.effects.len() {
        if haptic.effects[effect].hweffect.is_some() {
            destroy_effect(haptic, effect as HapticEffectID);
        }
    }
    DRIVER.close(haptic);
}

/// Translation of `SDL_QuitHaptics()`: closes every open device, whatever
/// handles remain (they become invalid).
pub(crate) fn quit_haptics() {
    loop {
        let first = with_haptics(|h| h.first().map(|h| (h.instance_id, h.serial)));
        let Some((instance_id, serial)) = first else {
            break;
        };
        close_haptic(instance_id, serial);
    }

    // (SDL_HIDAPI_HapticQuit() has nothing to do, see `hidapi`)
    DRIVER.quit();
}

#[cfg(test)]
mod tests;
