// Rust translation of src/SDL.c and include/SDL3/SDL_init.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Library initialization and shutdown, subsystem reference counting, and
//! application metadata.
//!
//! Direct translation of `SDL.c`.

use std::ops::{BitAnd, BitOr, BitOrAssign, Not};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;

use crate::err;
use crate::error::{Error, Result};
use crate::properties::Properties;
use crate::thread::current_thread_id;

/// Initialization flags for [`init`]/[`init_subsystem`]. Translation of `SDL_InitFlags`.
///
/// Combine with `|`; test with [`InitFlags::contains`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct InitFlags(pub u32);

impl InitFlags {
    pub const NONE: InitFlags = InitFlags(0);
    /// `SDL_INIT_AUDIO`; implies `EVENTS`.
    pub const AUDIO: InitFlags = InitFlags(0x0000_0010);
    /// `SDL_INIT_VIDEO`; implies `EVENTS`, should be initialized on the main thread.
    pub const VIDEO: InitFlags = InitFlags(0x0000_0020);
    /// `SDL_INIT_JOYSTICK`; implies `EVENTS`.
    pub const JOYSTICK: InitFlags = InitFlags(0x0000_0200);
    /// `SDL_INIT_HAPTIC`.
    pub const HAPTIC: InitFlags = InitFlags(0x0000_1000);
    /// `SDL_INIT_GAMEPAD`; implies `JOYSTICK`.
    pub const GAMEPAD: InitFlags = InitFlags(0x0000_2000);
    /// `SDL_INIT_EVENTS`.
    pub const EVENTS: InitFlags = InitFlags(0x0000_4000);
    /// `SDL_INIT_SENSOR`; implies `EVENTS`.
    pub const SENSOR: InitFlags = InitFlags(0x0000_8000);
    /// `SDL_INIT_CAMERA`; implies `EVENTS`.
    pub const CAMERA: InitFlags = InitFlags(0x0001_0000);
    /// Every subsystem (translation of the internal `SDL_ALL_SUBSYSTEM_FLAGS`).
    pub const ALL: InitFlags = InitFlags(!0);

    pub const fn contains(self, other: InitFlags) -> bool {
        (self.0 & other.0) == other.0
    }
    pub const fn intersects(self, other: InitFlags) -> bool {
        (self.0 & other.0) != 0
    }
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

impl BitOr for InitFlags {
    type Output = InitFlags;
    fn bitor(self, rhs: InitFlags) -> InitFlags {
        InitFlags(self.0 | rhs.0)
    }
}
impl BitOrAssign for InitFlags {
    fn bitor_assign(&mut self, rhs: InitFlags) {
        self.0 |= rhs.0;
    }
}
impl BitAnd for InitFlags {
    type Output = InitFlags;
    fn bitand(self, rhs: InitFlags) -> InitFlags {
        InitFlags(self.0 & rhs.0)
    }
}
impl Not for InitFlags {
    type Output = InitFlags;
    fn not(self) -> InitFlags {
        InitFlags(!self.0)
    }
}

impl std::fmt::Debug for InitFlags {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names = [
            (InitFlags::AUDIO, "AUDIO"),
            (InitFlags::VIDEO, "VIDEO"),
            (InitFlags::JOYSTICK, "JOYSTICK"),
            (InitFlags::HAPTIC, "HAPTIC"),
            (InitFlags::GAMEPAD, "GAMEPAD"),
            (InitFlags::EVENTS, "EVENTS"),
            (InitFlags::SENSOR, "SENSOR"),
            (InitFlags::CAMERA, "CAMERA"),
        ];
        let mut first = true;
        let mut rest = self.0;
        write!(f, "InitFlags(")?;
        for (flag, name) in names {
            if self.contains(flag) {
                if !first {
                    write!(f, " | ")?;
                }
                write!(f, "{name}")?;
                first = false;
                rest &= !flag.0;
            }
        }
        if rest != 0 {
            if !first {
                write!(f, " | ")?;
            }
            write!(f, "{rest:#x}")?;
            first = false;
        }
        if first {
            write!(f, "NONE")?;
        }
        write!(f, ")")
    }
}

/// Return values for optional main callbacks. Translation of `SDL_AppResult`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum AppResult {
    /// Value that requests that the app continue from the main callbacks.
    Continue,
    /// Value that requests termination with success from the main callbacks.
    Success,
    /// Value that requests termination with error from the main callbacks.
    Failure,
}

/// Application metadata properties. Translation of the `SDL_PROP_APP_METADATA_*` names.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum AppMetadata {
    /// The human-readable name of the application, like "My Game 2: Bad Guy's Revenge!".
    Name,
    /// The version of the application, like "1.0.0beta5".
    Version,
    /// A unique string in reverse-domain format, like "com.example.mygame2".
    Identifier,
    /// The human-readable name of the creator/developer/maker of this app.
    Creator,
    /// The human-readable copyright notice.
    Copyright,
    /// A URL to the app on the web.
    Url,
    /// The type of application: "game", "mediaplayer", "application", ...
    Type,
}

impl AppMetadata {
    /// The property name used in the global properties (`SDL_PROP_APP_METADATA_*_STRING`).
    pub const fn property_name(self) -> &'static str {
        match self {
            AppMetadata::Name => "SDL.app.metadata.name",
            AppMetadata::Version => "SDL.app.metadata.version",
            AppMetadata::Identifier => "SDL.app.metadata.identifier",
            AppMetadata::Creator => "SDL.app.metadata.creator",
            AppMetadata::Copyright => "SDL.app.metadata.copyright",
            AppMetadata::Url => "SDL.app.metadata.url",
            AppMetadata::Type => "SDL.app.metadata.type",
        }
    }

    /// Translation of `SDL_ValidMetadataProperty()`.
    pub fn from_property_name(name: &str) -> Option<AppMetadata> {
        [
            AppMetadata::Name,
            AppMetadata::Version,
            AppMetadata::Identifier,
            AppMetadata::Creator,
            AppMetadata::Copyright,
            AppMetadata::Url,
            AppMetadata::Type,
        ]
        .into_iter()
        .find(|m| m.property_name() == name)
    }
}

/// Specify basic metadata about your app. Translation of `SDL_SetAppMetadata()`.
///
/// You can optionally provide metadata about your app to SDL. This is not
/// required, but strongly encouraged: it is used by the OS and desktop
/// environment to identify the application (window titles, task switchers,
/// audio device lists). Passing `None` for a value clears it.
pub fn set_app_metadata(
    appname: Option<&str>,
    appversion: Option<&str>,
    appidentifier: Option<&str>,
) {
    set_app_metadata_property(AppMetadata::Name, appname);
    set_app_metadata_property(AppMetadata::Version, appversion);
    set_app_metadata_property(AppMetadata::Identifier, appidentifier);
}

/// Specify metadata about your app through one property. Translation of `SDL_SetAppMetadataProperty()`.
pub fn set_app_metadata_property(name: AppMetadata, value: Option<&str>) {
    let props = Properties::global();
    match value {
        Some(v) => {
            let _ = props.set(name.property_name(), v);
        }
        None => {
            props.remove(name.property_name());
        }
    }
}

/// Get metadata about your app, with SDL's fallbacks: the `SDL_APP_NAME` /
/// `SDL_APP_ID` hints win, then whatever was set, then the executable name or
/// "SDL Application" for the name and "application" for the type.
/// Translation of `SDL_GetAppMetadataProperty()`.
pub fn app_metadata_property(name: AppMetadata) -> Option<String> {
    let mut value = match name {
        AppMetadata::Name => crate::hints::get(crate::hints::APP_NAME),
        AppMetadata::Identifier => crate::hints::get(crate::hints::APP_ID),
        _ => None,
    };
    if value.as_deref().is_none_or(str::is_empty) {
        value = Properties::global().get_string(name.property_name());
    }
    if value.as_deref().is_none_or(str::is_empty) {
        value = match name {
            AppMetadata::Name => Some(exe_name().unwrap_or_else(|| "SDL Application".to_owned())),
            AppMetadata::Type => Some("application".to_owned()),
            _ => None,
        };
    }
    value
}

/// Translation of `SDL_GetExeName()` (the platform filesystem layer's view of `argv[0]`).
fn exe_name() -> Option<String> {
    std::env::current_exe()
        .ok()?
        .file_stem()?
        .to_str()
        .map(str::to_owned)
}

// The initialized subsystems
static MAIN_IS_READY: AtomicBool = AtomicBool::new(true); // (!SDL_MAIN_NEEDED)
static MAIN_THREAD_ID: AtomicU64 = AtomicU64::new(0);
static EVENTS_THREAD_ID: AtomicU64 = AtomicU64::new(0);
static VIDEO_THREAD_ID: AtomicU64 = AtomicU64::new(0);
static IN_MAIN_QUIT: AtomicBool = AtomicBool::new(false);
static SUBSYSTEM_REFCOUNT: Mutex<[u8; 32]> = Mutex::new([0; 32]);
static DONE_INFO: AtomicBool = AtomicBool::new(false);

fn refcounts() -> std::sync::MutexGuard<'static, [u8; 32]> {
    SUBSYSTEM_REFCOUNT.lock().unwrap_or_else(|e| e.into_inner())
}

/// Translation of `SDL_MostSignificantBitIndex32()` for a single-flag value.
fn subsystem_index(subsystem: InitFlags) -> Option<usize> {
    if subsystem.0 == 0 {
        None
    } else {
        Some(31 - subsystem.0.leading_zeros() as usize)
    }
}

/// Private helper to increment a subsystem's ref counter. Translation of `SDL_IncrementSubsystemRefCount()`.
fn increment_refcount(subsystem: InitFlags) {
    if let Some(i) = subsystem_index(subsystem) {
        let mut rc = refcounts();
        debug_assert!(rc[i] < 255);
        rc[i] = rc[i].saturating_add(1);
    }
}

/// Translation of `SDL_DecrementSubsystemRefCount()`.
fn decrement_refcount(subsystem: InitFlags) {
    if let Some(i) = subsystem_index(subsystem) {
        let mut rc = refcounts();
        if rc[i] > 0 {
            if IN_MAIN_QUIT.load(Ordering::Relaxed) {
                rc[i] = 0;
            } else {
                rc[i] -= 1;
            }
        }
    }
}

/// Translation of `SDL_ShouldInitSubsystem()`.
fn should_init_subsystem(subsystem: InitFlags) -> bool {
    subsystem_index(subsystem).is_some_and(|i| refcounts()[i] == 0)
}

/// Translation of `SDL_ShouldQuitSubsystem()`.
fn should_quit_subsystem(subsystem: InitFlags) -> bool {
    let Some(i) = subsystem_index(subsystem) else {
        return IN_MAIN_QUIT.load(Ordering::Relaxed);
    };
    let rc = refcounts()[i];
    if rc == 0 {
        return false;
    }
    /* If we're in SDL_Quit, we shut down every subsystem, even if refcount
     * isn't zero.
     */
    rc == 1 || IN_MAIN_QUIT.load(Ordering::Relaxed)
}

/// Translation of `SDL_InitOrIncrementSubsystem()`.
#[allow(dead_code)] // used by the video subsystem (SDL_InitOrIncrementSubsystem)
fn init_or_increment_subsystem(subsystem: InitFlags) -> Result<()> {
    let Some(i) = subsystem_index(subsystem) else {
        return Err(Error::invalid_param("subsystem"));
    };
    {
        let mut rc = refcounts();
        if rc[i] > 0 {
            rc[i] += 1;
            return Ok(());
        }
    }
    init_subsystem(subsystem)
}

/// Circumvent failure of [`init`] when not using the `SDL_main` entry point.
/// Translation of `SDL_SetMainReady()`.
pub fn set_main_ready() {
    MAIN_IS_READY.store(true, Ordering::Release);
    let _ = MAIN_THREAD_ID.compare_exchange(
        0,
        current_thread_id(),
        Ordering::AcqRel,
        Ordering::Acquire,
    );
}

/// Return whether this is the main thread. Translation of `SDL_IsMainThread()`.
///
/// On Apple platforms, the main thread is the thread that runs your program's
/// `main()` entry point. On other platforms, the main thread is the one that
/// calls `init(VIDEO)`, which should usually be the one that runs `main()`.
pub fn is_main_thread() -> bool {
    let me = current_thread_id();
    let video = VIDEO_THREAD_ID.load(Ordering::Acquire);
    if video != 0 {
        return me == video;
    }
    let events = EVENTS_THREAD_ID.load(Ordering::Acquire);
    if events != 0 {
        return me == events;
    }
    let main = MAIN_THREAD_ID.load(Ordering::Acquire);
    if main != 0 {
        return me == main;
    }
    true
}

/// Translation of `SDL_IsVideoThread()`.
pub(crate) fn is_video_thread() -> bool {
    current_thread_id() == VIDEO_THREAD_ID.load(Ordering::Acquire)
}

/// Initialize all the subsystems that require initialization before threads start.
/// Translation of `SDL_InitMainThread()`.
pub(crate) fn init_main_thread() {
    // If we haven't done it by now, mark this as the main thread
    let _ = MAIN_THREAD_ID.compare_exchange(
        0,
        current_thread_id(),
        Ordering::AcqRel,
        Ordering::Acquire,
    );
    // (TLS data initializes lazily in Rust)
    crate::stdlib::init_environment();
    let _ = crate::timer::ticks_ns();
    crate::filesystem::init_filesystem();
    crate::events::create_event_lock();

    if !DONE_INFO.swap(true, Ordering::AcqRel) {
        let show = |v: Option<String>| v.unwrap_or_else(|| "<unspecified>".to_owned());
        crate::log::info!(
            crate::log::Category::System,
            "App name: {}",
            show(app_metadata_property(AppMetadata::Name))
        );
        crate::log::info!(
            crate::log::Category::System,
            "App version: {}",
            show(app_metadata_property(AppMetadata::Version))
        );
        crate::log::info!(
            crate::log::Category::System,
            "App ID: {}",
            show(app_metadata_property(AppMetadata::Identifier))
        );
        crate::log::info!(
            crate::log::Category::System,
            "SDL revision: {}",
            crate::version::revision()
        );
    }
}

/// Translation of `SDL_QuitMainThread()`.
fn quit_main_thread() {
    crate::events::destroy_event_lock();
    crate::filesystem::quit_filesystem();
    crate::timer::quit_ticks();
    crate::stdlib::quit_environment();
}

/// Initialize specific SDL subsystems (reference counted). Translation of `SDL_InitSubSystem()`.
///
/// Subsystem initialization is ref-counted: you must call [`quit_subsystem`]
/// for each [`init_subsystem`] to correctly shut down a subsystem manually
/// (or call [`quit`] to force shutdown).
pub fn init_subsystem(flags: InitFlags) -> Result<()> {
    let mut flags_initialized = InitFlags::NONE;

    if !MAIN_IS_READY.load(Ordering::Acquire) {
        return Err(err!("Application didn't initialize properly, did you include SDL_main.h in the file containing your main() function?"));
    }

    init_main_thread();

    let result = (|| -> Result<()> {
        // Initialize the event subsystem
        if flags.contains(InitFlags::EVENTS) {
            if should_init_subsystem(InitFlags::EVENTS) {
                increment_refcount(InitFlags::EVENTS);
                // Note which thread initialized events
                // This is the thread which should be pumping events
                EVENTS_THREAD_ID.store(current_thread_id(), Ordering::Release);
                if let Err(e) = crate::events::init_events() {
                    decrement_refcount(InitFlags::EVENTS);
                    return Err(e);
                }
            } else {
                increment_refcount(InitFlags::EVENTS);
            }
            flags_initialized |= InitFlags::EVENTS;
        }

        // Initialize the video subsystem
        if flags.contains(InitFlags::VIDEO) {
            if should_init_subsystem(InitFlags::VIDEO) {
                // video implies events
                init_or_increment_subsystem(InitFlags::EVENTS)?;

                increment_refcount(InitFlags::VIDEO);

                // We initialize video on the main thread
                // On Apple platforms this is a requirement.
                // On other platforms, this is the definition.
                VIDEO_THREAD_ID.store(current_thread_id(), Ordering::Release);

                if let Err(e) = crate::video::core::init_video(None) {
                    decrement_refcount(InitFlags::VIDEO);
                    quit_subsystem(InitFlags::EVENTS);
                    return Err(e);
                }
            } else {
                increment_refcount(InitFlags::VIDEO);
            }
            flags_initialized |= InitFlags::VIDEO;
        }
        // Initialize the audio subsystem
        if flags.contains(InitFlags::AUDIO) {
            if should_init_subsystem(InitFlags::AUDIO) {
                // audio implies events
                init_or_increment_subsystem(InitFlags::EVENTS)?;

                increment_refcount(InitFlags::AUDIO);
                if let Err(e) = crate::audio::init_audio(None) {
                    decrement_refcount(InitFlags::AUDIO);
                    quit_subsystem(InitFlags::EVENTS);
                    return Err(e);
                }
            } else {
                increment_refcount(InitFlags::AUDIO);
            }
            flags_initialized |= InitFlags::AUDIO;
        }
        // Initialize the joystick subsystem
        if flags.contains(InitFlags::JOYSTICK) {
            if should_init_subsystem(InitFlags::JOYSTICK) {
                // joystick implies events
                init_or_increment_subsystem(InitFlags::EVENTS)?;

                increment_refcount(InitFlags::JOYSTICK);
                if let Err(e) = crate::joystick::init_joysticks() {
                    decrement_refcount(InitFlags::JOYSTICK);
                    quit_subsystem(InitFlags::EVENTS);
                    return Err(e);
                }
            } else {
                increment_refcount(InitFlags::JOYSTICK);
            }
            flags_initialized |= InitFlags::JOYSTICK;
        }

        if flags.contains(InitFlags::GAMEPAD) {
            if should_init_subsystem(InitFlags::GAMEPAD) {
                // game controller implies joystick
                init_or_increment_subsystem(InitFlags::JOYSTICK)?;

                increment_refcount(InitFlags::GAMEPAD);
                if let Err(e) = crate::joystick::gamepad::init_gamepads() {
                    decrement_refcount(InitFlags::GAMEPAD);
                    quit_subsystem(InitFlags::JOYSTICK);
                    return Err(e);
                }
            } else {
                increment_refcount(InitFlags::GAMEPAD);
            }
            flags_initialized |= InitFlags::GAMEPAD;
        }

        // Initialize the haptic subsystem
        if flags.contains(InitFlags::HAPTIC) {
            if should_init_subsystem(InitFlags::HAPTIC) {
                increment_refcount(InitFlags::HAPTIC);
                if let Err(e) = crate::haptic::init_haptics() {
                    decrement_refcount(InitFlags::HAPTIC);
                    return Err(e);
                }
            } else {
                increment_refcount(InitFlags::HAPTIC);
            }
            flags_initialized |= InitFlags::HAPTIC;
        }

        // Initialize the sensor subsystem
        if flags.contains(InitFlags::SENSOR) {
            if should_init_subsystem(InitFlags::SENSOR) {
                increment_refcount(InitFlags::SENSOR);
                if let Err(e) = crate::sensor::init_sensors() {
                    decrement_refcount(InitFlags::SENSOR);
                    return Err(e);
                }
            } else {
                increment_refcount(InitFlags::SENSOR);
            }
            flags_initialized |= InitFlags::SENSOR;
        }
        // Initialize the camera subsystem
        if flags.contains(InitFlags::CAMERA) {
            if should_init_subsystem(InitFlags::CAMERA) {
                // camera implies events
                init_or_increment_subsystem(InitFlags::EVENTS)?;

                increment_refcount(InitFlags::CAMERA);
                if let Err(e) = crate::camera::init_camera(None) {
                    decrement_refcount(InitFlags::CAMERA);
                    quit_subsystem(InitFlags::EVENTS);
                    return Err(e);
                }
            } else {
                increment_refcount(InitFlags::CAMERA);
            }
            flags_initialized |= InitFlags::CAMERA;
        }
        Ok(())
    })();

    if result.is_err() {
        // quit_and_error:
        quit_subsystem(flags_initialized);
    }
    result
}

/// Initialize the SDL library. Translation of `SDL_Init()`.
///
/// Consider reporting some basic metadata about your application before
/// calling `init`, using [`set_app_metadata`].
pub fn init(flags: InitFlags) -> Result<()> {
    init_subsystem(flags)
}

/// Shut down specific SDL subsystems. Translation of `SDL_QuitSubSystem()`.
pub fn quit_subsystem(flags: InitFlags) {
    // Shut down requested initialized subsystems

    if flags.contains(InitFlags::CAMERA) {
        if should_quit_subsystem(InitFlags::CAMERA) {
            crate::camera::quit_camera();
            // camera implies events
            quit_subsystem(InitFlags::EVENTS);
        }
        decrement_refcount(InitFlags::CAMERA);
    }

    if flags.contains(InitFlags::SENSOR) {
        if should_quit_subsystem(InitFlags::SENSOR) {
            crate::sensor::quit_sensors();
        }
        decrement_refcount(InitFlags::SENSOR);
    }

    if flags.contains(InitFlags::GAMEPAD) {
        if should_quit_subsystem(InitFlags::GAMEPAD) {
            crate::joystick::gamepad::quit_gamepads();
            // game controller implies joystick
            quit_subsystem(InitFlags::JOYSTICK);
        }
        decrement_refcount(InitFlags::GAMEPAD);
    }

    if flags.contains(InitFlags::JOYSTICK) {
        if should_quit_subsystem(InitFlags::JOYSTICK) {
            crate::joystick::quit_joysticks();
            // joystick implies events
            quit_subsystem(InitFlags::EVENTS);
        }
        decrement_refcount(InitFlags::JOYSTICK);
    }

    if flags.contains(InitFlags::HAPTIC) {
        if should_quit_subsystem(InitFlags::HAPTIC) {
            crate::haptic::quit_haptics();
        }
        decrement_refcount(InitFlags::HAPTIC);
    }

    if flags.contains(InitFlags::AUDIO) {
        if should_quit_subsystem(InitFlags::AUDIO) {
            crate::audio::quit_audio();
            // audio implies events
            quit_subsystem(InitFlags::EVENTS);
        }
        decrement_refcount(InitFlags::AUDIO);
    }

    if flags.contains(InitFlags::VIDEO) {
        if should_quit_subsystem(InitFlags::VIDEO) {
            crate::video::core::quit_video();
            VIDEO_THREAD_ID.store(0, Ordering::Release);
            // video implies events
            quit_subsystem(InitFlags::EVENTS);
        }
        decrement_refcount(InitFlags::VIDEO);
    }

    if flags.contains(InitFlags::EVENTS) {
        if should_quit_subsystem(InitFlags::EVENTS) {
            crate::events::quit_events();
            EVENTS_THREAD_ID.store(0, Ordering::Release);
        }
        decrement_refcount(InitFlags::EVENTS);
    }
}

/// Get a mask of the specified subsystems which are currently initialized.
/// With `InitFlags::NONE`, returns every initialized subsystem.
/// Translation of `SDL_WasInit()`.
pub fn was_init(flags: InitFlags) -> InitFlags {
    let rc = refcounts();

    // Fast path for checking one flag
    if flags.0.count_ones() == 1 {
        let i = subsystem_index(flags).unwrap_or(0);
        return if rc[i] != 0 { flags } else { InitFlags::NONE };
    }

    let mut flags = if flags.is_empty() {
        InitFlags::ALL.0
    } else {
        flags.0
    };
    let num_subsystems = rc.len().min(32 - flags.leading_zeros() as usize);

    // Iterate over each bit in flags, and check the matching subsystem.
    let mut initialized = 0u32;
    for (i, &count) in rc.iter().enumerate().take(num_subsystems) {
        if (flags & 1) != 0 && count > 0 {
            initialized |= 1 << i;
        }
        flags >>= 1;
    }
    InitFlags(initialized)
}

/// Clean up all initialized subsystems. Translation of `SDL_Quit()`.
///
/// You should call this function even if you have already shutdown each
/// initialized subsystem with [`quit_subsystem`]. It is safe to call this
/// function even in the case of errors in initialization.
pub fn quit() {
    IN_MAIN_QUIT.store(true, Ordering::Release);

    // Quit all subsystems
    quit_subsystem(InitFlags::ALL);
    crate::tray::cleanup_trays();
    crate::notification::cleanup_notifications();

    crate::timer::quit_timers();
    crate::io::quit_async_io();

    crate::assert::assertions_quit();
    crate::cpuinfo::quit_cpu_info();
    // (object validation, pixel format cache: nothing to free in Rust)

    /* Now that every subsystem has been quit, we reset the subsystem refcount
     * and the list of initialized subsystems.
     */
    *refcounts() = [0; 32];

    crate::log::quit_log();
    crate::hints::quit_hints();
    crate::properties::quit_properties();
    quit_main_thread();

    IN_MAIN_QUIT.store(false, Ordering::Release);
}

/// Get the name of the platform. Translation of `SDL_GetPlatform()`.
pub const fn platform() -> &'static str {
    if cfg!(target_os = "android") {
        "Android"
    } else if cfg!(target_os = "emscripten") {
        "Emscripten"
    } else if cfg!(target_os = "freebsd") {
        "FreeBSD"
    } else if cfg!(target_os = "haiku") {
        "Haiku"
    } else if cfg!(target_os = "linux") {
        "Linux"
    } else if cfg!(target_os = "macos") {
        "macOS"
    } else if cfg!(target_os = "netbsd") {
        "NetBSD"
    } else if cfg!(target_os = "openbsd") {
        "OpenBSD"
    } else if cfg!(target_os = "solaris") || cfg!(target_os = "illumos") {
        "Solaris"
    } else if cfg!(target_os = "windows") {
        "Windows"
    } else if cfg!(target_os = "visionos") {
        "visionOS"
    } else if cfg!(target_os = "ios") {
        "iOS"
    } else if cfg!(target_os = "tvos") {
        "tvOS"
    } else if cfg!(target_os = "horizon") {
        "Nintendo 3DS"
    } else if cfg!(target_os = "hurd") {
        "GNU/Hurd"
    } else {
        "Unknown (see SDL_platform.h)"
    }
}

/// Device form factors. Translation of `SDL_FormFactor`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum FormFactor {
    Unknown,
    Desktop,
    Laptop,
    Phone,
    Tablet,
    Console,
    Handheld,
    Watch,
    Tv,
    Headset,
    Car,
}

impl FormFactor {
    /// Translation of `SDL_GetDeviceFormFactorName()`.
    pub const fn name(self) -> &'static str {
        match self {
            FormFactor::Unknown => "SDL_FORMFACTOR_UNKNOWN",
            FormFactor::Desktop => "SDL_FORMFACTOR_DESKTOP",
            FormFactor::Laptop => "SDL_FORMFACTOR_LAPTOP",
            FormFactor::Phone => "SDL_FORMFACTOR_PHONE",
            FormFactor::Tablet => "SDL_FORMFACTOR_TABLET",
            FormFactor::Console => "SDL_FORMFACTOR_CONSOLE",
            FormFactor::Handheld => "SDL_FORMFACTOR_HANDHELD",
            FormFactor::Watch => "SDL_FORMFACTOR_WATCH",
            FormFactor::Tv => "SDL_FORMFACTOR_TV",
            FormFactor::Headset => "SDL_FORMFACTOR_HEADSET",
            FormFactor::Car => "SDL_FORMFACTOR_CAR",
        }
    }
}

/// The device's form factor. Translation of `SDL_GetDeviceFormFactor()`.
///
/// Android/OpenHarmony/iOS query the OS at runtime (platform backends, not
/// translated yet, so those report `Unknown`); other platforms are fixed.
pub const fn device_form_factor() -> FormFactor {
    if cfg!(any(target_os = "android", target_os = "ios")) {
        FormFactor::Unknown
    } else if cfg!(target_os = "horizon") {
        FormFactor::Handheld
    } else {
        FormFactor::Desktop
    }
}

/// Translation of `SDL_IsPhone()`.
pub const fn is_phone() -> bool {
    matches!(device_form_factor(), FormFactor::Phone)
}
/// Translation of `SDL_IsTablet()`.
pub const fn is_tablet() -> bool {
    matches!(device_form_factor(), FormFactor::Tablet)
}
/// Translation of `SDL_IsTV()`.
pub const fn is_tv() -> bool {
    matches!(device_form_factor(), FormFactor::Tv)
}

/// Application sandbox environments. Translation of `SDL_Sandbox`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Sandbox {
    None,
    UnknownContainer,
    Flatpak,
    Snap,
    MacOS,
    Lomiri,
}

/// Translation of `SDL_DetectSandbox()`.
fn detect_sandbox() -> Sandbox {
    if cfg!(target_os = "linux") {
        if std::path::Path::new("/.flatpak-info").exists() {
            return Sandbox::Flatpak;
        }
        /* For Snap, we check multiple variables because they might be set for
         * unrelated reasons. This is the same thing WebKitGTK does. */
        let has = |v: &str| crate::stdlib::getenv(v).is_some();
        if has("SNAP") && has("SNAP_NAME") && has("SNAP_REVISION") {
            return Sandbox::Snap;
        }
        /* Ubuntu Touch also supports Snap; check for classic sandboxing only if
         * Snap hasn't been detected. */
        if has("LOMIRI_APPLICATION_ISOLATION") || has("CLICKABLE_DESKTOP_MODE") {
            return Sandbox::Lomiri;
        }
        if std::path::Path::new("/run/host/container-manager").exists() {
            return Sandbox::UnknownContainer;
        }
    } else if cfg!(target_os = "macos")
        && crate::stdlib::getenv("APP_SANDBOX_CONTAINER_ID").is_some()
    {
        return Sandbox::MacOS;
    }
    Sandbox::None
}

/// Get the application sandbox environment, if any. Translation of `SDL_GetSandbox()`.
pub fn sandbox() -> Sandbox {
    static SANDBOX: std::sync::OnceLock<Sandbox> = std::sync::OnceLock::new();
    *SANDBOX.get_or_init(detect_sandbox)
}

/// Terminate the process immediately, without running destructors.
/// Translation of the internal `SDL_ExitProcess()`.
#[allow(dead_code)] // used by SDL_main callbacks (later phase)
pub(crate) fn exit_process(exitcode: i32) -> ! {
    std::process::exit(exitcode)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) use crate::test_support::TEST_LOCK;

    #[test]
    fn flags() {
        let f = InitFlags::VIDEO | InitFlags::EVENTS;
        assert!(f.contains(InitFlags::VIDEO));
        assert!(!f.contains(InitFlags::AUDIO));
        assert!(f.intersects(InitFlags::EVENTS | InitFlags::AUDIO));
        assert_eq!(format!("{f:?}"), "InitFlags(VIDEO | EVENTS)");
        assert_eq!(format!("{:?}", InitFlags::NONE), "InitFlags(NONE)");
        assert_eq!(subsystem_index(InitFlags::EVENTS), Some(14));
        assert_eq!(subsystem_index(InitFlags::NONE), None);
    }

    #[test]
    fn metadata() {
        let _l = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_app_metadata(Some("Test App"), Some("1.2"), Some("com.example.test"));
        assert_eq!(
            app_metadata_property(AppMetadata::Name).as_deref(),
            Some("Test App")
        );
        assert_eq!(
            app_metadata_property(AppMetadata::Version).as_deref(),
            Some("1.2")
        );
        assert_eq!(
            app_metadata_property(AppMetadata::Identifier).as_deref(),
            Some("com.example.test")
        );
        assert_eq!(
            app_metadata_property(AppMetadata::Type).as_deref(),
            Some("application")
        );
        assert_eq!(app_metadata_property(AppMetadata::Url), None);
        set_app_metadata_property(AppMetadata::Name, None);
        assert!(app_metadata_property(AppMetadata::Name).is_some()); // falls back to the exe name
        assert_eq!(
            AppMetadata::from_property_name("SDL.app.metadata.url"),
            Some(AppMetadata::Url)
        );
        assert_eq!(AppMetadata::from_property_name("nope"), None);
    }

    #[test]
    fn init_quit_refcounting() {
        let _l = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(was_init(InitFlags::EVENTS), InitFlags::NONE);
        init(InitFlags::EVENTS).unwrap();
        init_subsystem(InitFlags::EVENTS).unwrap();
        assert_eq!(was_init(InitFlags::EVENTS), InitFlags::EVENTS);
        assert_eq!(was_init(InitFlags::NONE), InitFlags::EVENTS);
        assert!(is_main_thread());
        quit_subsystem(InitFlags::EVENTS);
        assert_eq!(was_init(InitFlags::EVENTS), InitFlags::EVENTS); // still one reference
        quit_subsystem(InitFlags::EVENTS);
        assert_eq!(was_init(InitFlags::EVENTS), InitFlags::NONE);

        // A subsystem that fails rolls back anything that was initialized
        // in the same call (without a driver hint, no video driver is
        // available).
        crate::hints::reset(crate::hints::VIDEO_DRIVER);
        let e = init(InitFlags::EVENTS | InitFlags::VIDEO).unwrap_err();
        assert_eq!(e.message(), "No available video device");
        assert_eq!(was_init(InitFlags::NONE), InitFlags::NONE);

        init(InitFlags::EVENTS).unwrap();
        init(InitFlags::EVENTS).unwrap();
        quit(); // forces everything down regardless of refcounts
        assert_eq!(was_init(InitFlags::NONE), InitFlags::NONE);
    }

    #[test]
    fn platform_info() {
        assert!(!platform().is_empty());
        assert_eq!(FormFactor::Desktop.name(), "SDL_FORMFACTOR_DESKTOP");
        let _ = device_form_factor();
        let _ = (is_phone(), is_tablet(), is_tv());
        let _ = sandbox();
    }
}
