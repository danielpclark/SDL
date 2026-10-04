// Rust translation of src/audio/pulseaudio/SDL_pulseaudio.c and
// src/audio/pulseaudio/SDL_pulseaudio.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

// The PulseAudio driver. libpulse is loaded at run time
// (SDL_AUDIO_DRIVER_PULSEAUDIO_DYNAMIC), and only the parts of its API SDL
// uses are declared here.
//
// Upstream keeps the main loop, the context and the default device paths
// in file-level statics and passes NULL userdata to most callbacks; here
// they live in `PulseShared`, which the callbacks get as their userdata.

use std::collections::HashMap;
use std::ffi::{c_char, c_int, c_uint, c_void, CStr, CString};
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use crate::audio::device::{
    add_audio_device, audio_device_disconnected, default_audio_device_changed,
    find_physical_audio_device_by_callback, updated_audio_device_format, AudioBootStrap,
    AudioDriverImpl, DeviceBackend, DriverFlags, PhysState, PhysicalDevice,
};
use crate::audio::format::AudioSpec;
use crate::audio::AudioFormat;
use crate::error::{Error, Result};
use crate::loadso::SharedObject;
use crate::thread::{Semaphore, Thread, ThreadPriority};

// The parts of <pulse/pulseaudio.h> SDL uses.

macro_rules! opaque {
    ($($name:ident),*) => {
        $(
            #[repr(C)]
            struct $name {
                _private: [u8; 0],
            }
        )*
    };
}
opaque!(
    PaThreadedMainloop,
    PaMainloopApi,
    PaProplist,
    PaOperation,
    PaContext,
    PaStream,
    PaSpawnApi,
    PaCvolumeOpaque
);

const PA_CHANNELS_MAX: usize = 32;
const PA_INVALID_INDEX: u32 = u32::MAX;

const PA_CONTEXT_CONNECTING: c_int = 1;
const PA_CONTEXT_READY: c_int = 4;

/// `PA_CONTEXT_IS_GOOD()`.
fn pa_context_is_good(x: c_int) -> bool {
    (PA_CONTEXT_CONNECTING..=PA_CONTEXT_READY).contains(&x)
}

const PA_STREAM_CREATING: c_int = 1;
const PA_STREAM_READY: c_int = 2;

/// `PA_STREAM_IS_GOOD()`.
fn pa_stream_is_good(x: c_int) -> bool {
    x == PA_STREAM_CREATING || x == PA_STREAM_READY
}

const PA_OPERATION_RUNNING: c_int = 0;

const PA_SAMPLE_U8: c_int = 0;
const PA_SAMPLE_S16LE: c_int = 3;
const PA_SAMPLE_S16BE: c_int = 4;
const PA_SAMPLE_FLOAT32LE: c_int = 5;
const PA_SAMPLE_FLOAT32BE: c_int = 6;
const PA_SAMPLE_S32LE: c_int = 7;
const PA_SAMPLE_S32BE: c_int = 8;
const PA_SAMPLE_INVALID: c_int = -1;

const PA_STREAM_ADJUST_LATENCY: c_uint = 0x2000;
const PA_SEEK_RELATIVE: c_int = 0;

const PA_SUBSCRIPTION_MASK_SINK: c_uint = 0x0001;
const PA_SUBSCRIPTION_MASK_SOURCE: c_uint = 0x0002;
const PA_SUBSCRIPTION_MASK_SERVER: c_uint = 0x0080;
const PA_SUBSCRIPTION_EVENT_SINK: c_uint = 0x0000;
const PA_SUBSCRIPTION_EVENT_SOURCE: c_uint = 0x0001;
const PA_SUBSCRIPTION_EVENT_FACILITY_MASK: c_uint = 0x000F;
const PA_SUBSCRIPTION_EVENT_NEW: c_uint = 0x0000;
const PA_SUBSCRIPTION_EVENT_CHANGE: c_uint = 0x0010;
const PA_SUBSCRIPTION_EVENT_REMOVE: c_uint = 0x0020;
const PA_SUBSCRIPTION_EVENT_TYPE_MASK: c_uint = 0x0030;

const PA_CHANNEL_POSITION_MONO: c_int = 0;
const PA_CHANNEL_POSITION_FRONT_LEFT: c_int = 1;
const PA_CHANNEL_POSITION_FRONT_RIGHT: c_int = 2;
const PA_CHANNEL_POSITION_FRONT_CENTER: c_int = 3;
const PA_CHANNEL_POSITION_REAR_CENTER: c_int = 4;
const PA_CHANNEL_POSITION_REAR_LEFT: c_int = 5;
const PA_CHANNEL_POSITION_REAR_RIGHT: c_int = 6;
const PA_CHANNEL_POSITION_LFE: c_int = 7;
const PA_CHANNEL_POSITION_SIDE_LEFT: c_int = 10;
const PA_CHANNEL_POSITION_SIDE_RIGHT: c_int = 11;

const PA_PROP_APPLICATION_ICON_NAME: &CStr = c"application.icon_name";

/// `pa_sample_spec`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct PaSampleSpec {
    format: c_int,
    rate: u32,
    channels: u8,
}

/// `pa_channel_map`.
#[repr(C)]
struct PaChannelMap {
    channels: u8,
    map: [c_int; PA_CHANNELS_MAX],
}

/// `pa_buffer_attr`.
#[repr(C)]
struct PaBufferAttr {
    maxlength: u32,
    tlength: u32,
    prebuf: u32,
    minreq: u32,
    fragsize: u32,
}

/// `pa_cvolume`.
#[repr(C)]
struct PaCvolume {
    channels: u8,
    values: [u32; PA_CHANNELS_MAX],
}

/// The leading fields of `pa_sink_info` (only read through pointers from libpulse).
#[repr(C)]
struct PaSinkInfo {
    name: *const c_char,
    index: u32,
    description: *const c_char,
    sample_spec: PaSampleSpec,
}

/// The leading fields of `pa_source_info`.
#[repr(C)]
struct PaSourceInfo {
    name: *const c_char,
    index: u32,
    description: *const c_char,
    sample_spec: PaSampleSpec,
    channel_map: PaChannelMap,
    owner_module: u32,
    volume: PaCvolume,
    mute: c_int,
    monitor_of_sink: u32,
}

/// The leading fields of `pa_server_info`.
#[repr(C)]
struct PaServerInfo {
    user_name: *const c_char,
    host_name: *const c_char,
    server_version: *const c_char,
    server_name: *const c_char,
    sample_spec: PaSampleSpec,
    default_sink_name: *const c_char,
    default_source_name: *const c_char,
}

type PaContextNotifyCb = Option<unsafe extern "C" fn(*mut PaContext, *mut c_void)>;
type PaOperationNotifyCb = Option<unsafe extern "C" fn(*mut PaOperation, *mut c_void)>;
type PaSinkInfoCb =
    Option<unsafe extern "C" fn(*mut PaContext, *const PaSinkInfo, c_int, *mut c_void)>;
type PaSourceInfoCb =
    Option<unsafe extern "C" fn(*mut PaContext, *const PaSourceInfo, c_int, *mut c_void)>;
type PaServerInfoCb =
    Option<unsafe extern "C" fn(*mut PaContext, *const PaServerInfo, *mut c_void)>;
type PaContextSuccessCb = Option<unsafe extern "C" fn(*mut PaContext, c_int, *mut c_void)>;
type PaContextSubscribeCb = Option<unsafe extern "C" fn(*mut PaContext, c_uint, u32, *mut c_void)>;
type PaStreamNotifyCb = Option<unsafe extern "C" fn(*mut PaStream, *mut c_void)>;
type PaStreamRequestCb = Option<unsafe extern "C" fn(*mut PaStream, usize, *mut c_void)>;
type PaStreamSuccessCb = Option<unsafe extern "C" fn(*mut PaStream, c_int, *mut c_void)>;
type PaFreeCb = Option<unsafe extern "C" fn(*mut c_void)>;

macro_rules! pulse_syms {
    ($($name:ident: fn($($arg:ty),*) $(-> $ret:ty)?;)*) => {
        /// The libpulse entry points SDL uses (the `PULSEAUDIO_pa_*` function
        /// pointers), with the library they came from. (All are loaded, as
        /// upstream does, though not every one is called.)
        #[allow(dead_code)]
        struct PulseLib {
            $($name: unsafe extern "C" fn($($arg),*) $(-> $ret)?,)*
            // optional
            /// needs pulseaudio 4.0
            pa_operation_set_state_callback:
                Option<unsafe extern "C" fn(*mut PaOperation, PaOperationNotifyCb, *mut c_void)>,
            /// needs pulseaudio 5.0
            pa_threaded_mainloop_set_name:
                Option<unsafe extern "C" fn(*mut PaThreadedMainloop, *const c_char)>,
            _handle: SharedObject,
        }

        impl PulseLib {
            /// Translation of `load_pulseaudio_syms()`.
            fn load_syms(handle: SharedObject) -> Result<PulseLib> {
                Ok(PulseLib {
                    $(
                        // SAFETY: the declared type is the symbol's C
                        // signature from <pulse/pulseaudio.h>, and the
                        // pointer is only called while `_handle` keeps the
                        // library loaded.
                        $name: unsafe { handle.function(stringify!($name))? },
                    )*
                    // SAFETY: as above.
                    pa_operation_set_state_callback: unsafe {
                        handle.function("pa_operation_set_state_callback").ok()
                    },
                    // SAFETY: as above.
                    pa_threaded_mainloop_set_name: unsafe {
                        handle.function("pa_threaded_mainloop_set_name").ok()
                    },
                    _handle: handle,
                })
            }
        }
    };
}

pulse_syms! {
    pa_get_library_version: fn() -> *const c_char;
    pa_threaded_mainloop_new: fn() -> *mut PaThreadedMainloop;
    pa_threaded_mainloop_get_api: fn(*mut PaThreadedMainloop) -> *mut PaMainloopApi;
    pa_threaded_mainloop_start: fn(*mut PaThreadedMainloop) -> c_int;
    pa_threaded_mainloop_stop: fn(*mut PaThreadedMainloop);
    pa_threaded_mainloop_lock: fn(*mut PaThreadedMainloop);
    pa_threaded_mainloop_unlock: fn(*mut PaThreadedMainloop);
    pa_threaded_mainloop_wait: fn(*mut PaThreadedMainloop);
    pa_threaded_mainloop_signal: fn(*mut PaThreadedMainloop, c_int);
    pa_threaded_mainloop_free: fn(*mut PaThreadedMainloop);
    pa_operation_get_state: fn(*const PaOperation) -> c_int;
    pa_operation_cancel: fn(*mut PaOperation);
    pa_operation_unref: fn(*mut PaOperation);
    pa_context_new_with_proplist: fn(*mut PaMainloopApi, *const c_char, *const PaProplist) -> *mut PaContext;
    pa_context_set_state_callback: fn(*mut PaContext, PaContextNotifyCb, *mut c_void);
    pa_context_connect: fn(*mut PaContext, *const c_char, c_uint, *const PaSpawnApi) -> c_int;
    pa_context_get_sink_info_list: fn(*mut PaContext, PaSinkInfoCb, *mut c_void) -> *mut PaOperation;
    pa_context_get_source_info_list: fn(*mut PaContext, PaSourceInfoCb, *mut c_void) -> *mut PaOperation;
    pa_context_get_sink_info_by_index: fn(*mut PaContext, u32, PaSinkInfoCb, *mut c_void) -> *mut PaOperation;
    pa_context_get_source_info_by_index: fn(*mut PaContext, u32, PaSourceInfoCb, *mut c_void) -> *mut PaOperation;
    pa_context_get_state: fn(*const PaContext) -> c_int;
    pa_context_subscribe: fn(*mut PaContext, c_uint, PaContextSuccessCb, *mut c_void) -> *mut PaOperation;
    pa_context_set_subscribe_callback: fn(*mut PaContext, PaContextSubscribeCb, *mut c_void);
    pa_context_disconnect: fn(*mut PaContext);
    pa_context_unref: fn(*mut PaContext);
    pa_stream_new: fn(*mut PaContext, *const c_char, *const PaSampleSpec, *const PaChannelMap) -> *mut PaStream;
    pa_stream_set_state_callback: fn(*mut PaStream, PaStreamNotifyCb, *mut c_void);
    pa_stream_connect_playback: fn(*mut PaStream, *const c_char, *const PaBufferAttr, c_uint, *const PaCvolumeOpaque, *mut PaStream) -> c_int;
    pa_stream_connect_record: fn(*mut PaStream, *const c_char, *const PaBufferAttr, c_uint) -> c_int;
    pa_stream_get_buffer_attr: fn(*mut PaStream) -> *const PaBufferAttr;
    pa_stream_get_state: fn(*const PaStream) -> c_int;
    pa_stream_writable_size: fn(*const PaStream) -> usize;
    pa_stream_readable_size: fn(*const PaStream) -> usize;
    pa_stream_begin_write: fn(*mut PaStream, *mut *mut c_void, *mut usize) -> c_int;
    pa_stream_write: fn(*mut PaStream, *const c_void, usize, PaFreeCb, i64, c_int) -> c_int;
    pa_stream_drain: fn(*mut PaStream, PaStreamSuccessCb, *mut c_void) -> *mut PaOperation;
    pa_stream_disconnect: fn(*mut PaStream) -> c_int;
    pa_stream_peek: fn(*mut PaStream, *mut *const c_void, *mut usize) -> c_int;
    pa_stream_drop: fn(*mut PaStream) -> c_int;
    pa_stream_flush: fn(*mut PaStream, PaStreamSuccessCb, *mut c_void) -> *mut PaOperation;
    pa_stream_unref: fn(*mut PaStream);
    pa_channel_map_init_auto: fn(*mut PaChannelMap, c_uint, c_int) -> *mut PaChannelMap;
    pa_strerror: fn(c_int) -> *const c_char;
    pa_stream_set_write_callback: fn(*mut PaStream, PaStreamRequestCb, *mut c_void);
    pa_stream_set_read_callback: fn(*mut PaStream, PaStreamRequestCb, *mut c_void);
    pa_context_get_server_info: fn(*mut PaContext, PaServerInfoCb, *mut c_void) -> *mut PaOperation;
    pa_proplist_new: fn() -> *mut PaProplist;
    pa_proplist_free: fn(*mut PaProplist);
    pa_proplist_sets: fn(*mut PaProplist, *const c_char, *const c_char) -> c_int;
}

/// `SDL_AUDIO_DRIVER_PULSEAUDIO_DYNAMIC`.
const PULSEAUDIO_LIBRARY: &str = "libpulse.so.0";

/// Translation of `LoadPulseAudioLibrary()`.
fn load_pulseaudio_library() -> Result<PulseLib> {
    // Don't call SDL_SetError(): SDL_LoadObject already did.
    let handle = SharedObject::load(PULSEAUDIO_LIBRARY)?;
    PulseLib::load_syms(handle) // (UnloadPulseAudioLibrary() is dropping it)
}

/// A C string from libpulse as a `String` (`None` for NULL).
///
/// # Safety
///
/// `s` must be NULL or point to a NUL-terminated string.
unsafe fn opt_str(s: *const c_char) -> Option<String> {
    if s.is_null() {
        None
    } else {
        // SAFETY: non-NULL and NUL-terminated (the caller's contract).
        Some(unsafe { CStr::from_ptr(s) }.to_string_lossy().into_owned())
    }
}

/// Translation of `PulseDeviceHandle`.
#[derive(Clone, Debug)]
struct PulseDeviceHandle {
    device_path: String,
    device_index: u32,
}

/// The default devices (`default_sink_path` and friends). These are the OS
/// identifiers (i.e. ALSA strings)...these are allocated in a callback when
/// the default changes, and noticed by the hotplug thread when it alerts SDL
/// to the change.
#[derive(Default)]
struct Defaults {
    default_sink_path: Option<String>,
    default_source_path: Option<String>,
    default_sink_changed: bool,
    default_source_changed: bool,
}

/// The connection and everything the callbacks share (upstream's file-level statics).
struct PulseShared {
    lib: PulseLib,
    /// `pulseaudio_threaded_mainloop`
    mainloop: *mut PaThreadedMainloop,
    /// `pulseaudio_context`
    context: *mut PaContext,
    /// What the context's state callback signals.
    _context_signal: Box<MainloopSignal>,
    /// should we include monitors in the device list? Set at SDL_Init time
    include_monitors: bool,
    defaults: Mutex<Defaults>,
    /// The handles given to `add_audio_device` (`PulseDeviceHandle *` upstream).
    handles: Mutex<HashMap<usize, PulseDeviceHandle>>,
    next_handle: AtomicUsize,
    /// `pulseaudio_hotplug_thread_active`
    hotplug_thread_active: AtomicBool,
}

// SAFETY: the main loop and context are libpulse objects meant to be used
// from any thread while holding the main loop's lock, which is how every
// call below uses them (as upstream does).
unsafe impl Send for PulseShared {}
// SAFETY: as above; the Rust-side state is behind mutexes and atomics.
unsafe impl Sync for PulseShared {}

impl PulseShared {
    fn lock(&self) {
        // SAFETY: the main loop is alive until `deinitialize`.
        unsafe { (self.lib.pa_threaded_mainloop_lock)(self.mainloop) }
    }

    fn unlock(&self) {
        // SAFETY: as above; we hold the lock.
        unsafe { (self.lib.pa_threaded_mainloop_unlock)(self.mainloop) }
    }

    /// this releases the lock and blocks on an internal condition variable.
    fn wait(&self) {
        // SAFETY: as above; we hold the lock.
        unsafe { (self.lib.pa_threaded_mainloop_wait)(self.mainloop) }
    }

    fn signal(&self) {
        // SAFETY: the main loop is alive.
        unsafe { (self.lib.pa_threaded_mainloop_signal)(self.mainloop, 0) }
    }

    fn context_state(&self) -> c_int {
        // SAFETY: the context is alive; we hold the lock.
        unsafe { (self.lib.pa_context_get_state)(self.context) }
    }

    fn userdata(&self) -> *mut c_void {
        ptr::from_ref(self).cast_mut().cast()
    }

    fn handle_of(&self, device: &PhysicalDevice) -> Option<PulseDeviceHandle> {
        self.handles
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&device.handle)
            .cloned()
    }

    /// Translation of `WaitForPulseOperation()`.
    ///
    /// This function assume you are holding `mainloop`'s lock. The operation is unref'd in here, assuming
    /// you did the work in the callback and just want to know it's done, though.
    fn wait_for_pulse_operation(&self, o: *mut PaOperation) {
        // This checks for NO errors currently. Either fix that, check results elsewhere, or do things you don't care about.
        crate::sdl_assert!(!self.mainloop.is_null());
        if !o.is_null() {
            // note that if PULSEAUDIO_pa_operation_set_state_callback == NULL, then `o` must have a callback that will signal pulseaudio_threaded_mainloop.
            // If not, on really old (earlier PulseAudio 4.0, from the year 2013!) installs, this call will block forever.
            // On more modern installs, we won't ever block forever, and maybe be more efficient, thanks to pa_operation_set_state_callback.
            // WARNING: at the time of this writing: the Steam Runtime is still on PulseAudio 1.1!
            if let Some(set_state_callback) = self.lib.pa_operation_set_state_callback {
                // SAFETY: `o` is a live operation; the callback's userdata is
                // `self`, which outlives it.
                unsafe {
                    set_state_callback(o, Some(operation_state_change_callback), self.userdata())
                };
            }
            // SAFETY: `o` is a live operation; we hold the lock.
            while unsafe { (self.lib.pa_operation_get_state)(o) } == PA_OPERATION_RUNNING {
                self.wait(); // this releases the lock and blocks on an internal condition variable.
            }
            // SAFETY: we own this reference to `o`.
            unsafe { (self.lib.pa_operation_unref)(o) };
        }
    }

    /// Translation of `DisconnectFromPulseServer()`.
    fn disconnect_from_pulse_server(&self) {
        disconnect_from_pulse_server(&self.lib, self.mainloop, self.context);
    }
}

/// Translation of `getAppName()`.
fn get_app_name() -> String {
    crate::init::app_metadata_property(crate::init::AppMetadata::Name).unwrap_or_default()
}

/// Translation of `OperationStateChangeCallback()`; the userdata is the `PulseShared`.
unsafe extern "C" fn operation_state_change_callback(_o: *mut PaOperation, userdata: *mut c_void) {
    // SAFETY: the userdata is the live `PulseShared` (see the registration).
    let shared = unsafe { &*userdata.cast::<PulseShared>() };
    shared.signal(); // just signal any waiting code, it can look up the details.
}

/// What the context's state callback signals (upstream's callbacks use the
/// global main loop and function pointer); boxed, so its address is stable
/// for as long as the context lives.
struct MainloopSignal {
    signal: unsafe extern "C" fn(*mut PaThreadedMainloop, c_int),
    mainloop: *mut PaThreadedMainloop,
}

/// Translation of `DisconnectFromPulseServer()`.
fn disconnect_from_pulse_server(
    lib: &PulseLib,
    mainloop: *mut PaThreadedMainloop,
    context: *mut PaContext,
) {
    // SAFETY (all calls): the objects are live, unlocked, and released once here.
    if !mainloop.is_null() {
        // SAFETY: as above.
        unsafe { (lib.pa_threaded_mainloop_stop)(mainloop) };
    }
    if !context.is_null() {
        // SAFETY: as above.
        unsafe {
            (lib.pa_context_disconnect)(context);
            (lib.pa_context_unref)(context);
        }
    }
    if !mainloop.is_null() {
        // SAFETY: as above.
        unsafe { (lib.pa_threaded_mainloop_free)(mainloop) };
    }
}

/// Translation of `PulseContextStateChangeCallback()`; the userdata is a `MainloopSignal`.
unsafe extern "C" fn pulse_context_state_change_callback(
    _context: *mut PaContext,
    userdata: *mut c_void,
) {
    // SAFETY: the userdata is the live `MainloopSignal` (see the registration),
    // and its main loop is alive while the context is.
    unsafe {
        let s = &*userdata.cast::<MainloopSignal>();
        (s.signal)(s.mainloop, 0) // just signal any waiting code, it can look up the details.
    };
}

/// The connection `ConnectToPulseServer()` sets up.
struct Connection {
    mainloop: *mut PaThreadedMainloop,
    context: *mut PaContext,
    context_signal: Box<MainloopSignal>,
}

/// Translation of `ConnectToPulseServer()`.
fn connect_to_pulse_server(lib: &PulseLib) -> Result<Connection> {
    // Set up a new main loop
    // SAFETY: no preconditions.
    let mainloop = unsafe { (lib.pa_threaded_mainloop_new)() };
    if mainloop.is_null() {
        return Err(Error::new("pa_threaded_mainloop_new() failed"));
    }

    if let Some(set_name) = lib.pa_threaded_mainloop_set_name {
        // SAFETY: `mainloop` is live; the name is NUL-terminated.
        unsafe { set_name(mainloop, c"PulseMainloop".as_ptr()) };
    }

    // SAFETY: `mainloop` is live.
    if unsafe { (lib.pa_threaded_mainloop_start)(mainloop) } < 0 {
        // SAFETY: `mainloop` is live and freed once.
        unsafe { (lib.pa_threaded_mainloop_free)(mainloop) };
        return Err(Error::new("pa_threaded_mainloop_start() failed"));
    }

    // SAFETY: `mainloop` is running.
    unsafe { (lib.pa_threaded_mainloop_lock)(mainloop) };

    let context_signal = Box::new(MainloopSignal {
        signal: lib.pa_threaded_mainloop_signal,
        mainloop,
    });
    let mut context: *mut PaContext = ptr::null_mut();
    let result = (|| {
        // SAFETY: we hold the lock.
        let mainloop_api = unsafe { (lib.pa_threaded_mainloop_get_api)(mainloop) };
        crate::sdl_assert!(!mainloop_api.is_null()); // this never fails, right?

        // SAFETY: no preconditions.
        let proplist = unsafe { (lib.pa_proplist_new)() };
        if proplist.is_null() {
            return Err(Error::new("pa_proplist_new() failed"));
        }

        let icon_name = crate::hints::get(crate::hints::AUDIO_DEVICE_APP_ICON_NAME)
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "applications-games".to_owned());
        let icon_name = CString::new(icon_name).unwrap_or_default();
        // SAFETY: `proplist` is live; both strings are NUL-terminated.
        unsafe {
            (lib.pa_proplist_sets)(
                proplist,
                PA_PROP_APPLICATION_ICON_NAME.as_ptr(),
                icon_name.as_ptr(),
            )
        };

        let app_name = CString::new(get_app_name()).unwrap_or_default();
        // SAFETY: we hold the lock; the api, name and proplist are valid.
        context = unsafe {
            (lib.pa_context_new_with_proplist)(mainloop_api, app_name.as_ptr(), proplist)
        };
        if context.is_null() {
            // FIXME (upstream): the proplist is leaked on this path.
            return Err(Error::new("pa_context_new_with_proplist() failed"));
        }
        // SAFETY: the context copied what it needs; we free our proplist once.
        unsafe { (lib.pa_proplist_free)(proplist) };

        // SAFETY: the context is live; the userdata (`context_signal`) is
        // kept alive with the context.
        unsafe {
            (lib.pa_context_set_state_callback)(
                context,
                Some(pulse_context_state_change_callback),
                ptr::from_ref(&*context_signal).cast_mut().cast(),
            )
        };

        // Connect to the PulseAudio server
        // SAFETY: the context is live; NULL server and spawn API are allowed.
        if unsafe { (lib.pa_context_connect)(context, ptr::null(), 0, ptr::null()) } < 0 {
            return Err(Error::new("Could not setup connection to PulseAudio"));
        }

        // SAFETY (the state queries): the context is live; we hold the lock.
        let mut state = unsafe { (lib.pa_context_get_state)(context) };
        while pa_context_is_good(state) && state != PA_CONTEXT_READY {
            // SAFETY: we hold the lock.
            unsafe { (lib.pa_threaded_mainloop_wait)(mainloop) };
            // SAFETY: as above.
            state = unsafe { (lib.pa_context_get_state)(context) };
        }

        if state != PA_CONTEXT_READY {
            return Err(Error::new("Could not connect to PulseAudio"));
        }
        Ok(())
    })();

    // SAFETY: we hold the lock.
    unsafe { (lib.pa_threaded_mainloop_unlock)(mainloop) };

    match result {
        Ok(()) => Ok(Connection {
            mainloop,
            context,
            context_signal,
        }), // connected and ready!
        Err(e) => {
            // (failed:)
            disconnect_from_pulse_server(lib, mainloop, context);
            Err(e)
        }
    }
}

/// The begin-write buffer and the recording fragment, which upstream keeps
/// in `struct SDL_PrivateAudioData`.
struct StreamBuffers {
    /// The buffer `pa_stream_begin_write()` handed out for this iteration.
    write_buf: *mut u8,
    write_len: usize,
    recordingbuf: *const u8,
    recordinglen: usize,
}

/// An opened stream (`struct SDL_PrivateAudioData`; the core owns the mix
/// buffer, so `mixbuf` isn't needed).
struct PulseDevice {
    shared: Arc<PulseShared>,
    /// pulseaudio structures
    stream: *mut PaStream,
    /// bytes of data the hardware wants _now_.
    bytes_requested: AtomicI32,
    buffers: Mutex<StreamBuffers>,
}

// SAFETY: the stream is used under the main loop's lock (or, as upstream
// does for begin_write/peek data, only from the device thread).
unsafe impl Send for PulseDevice {}
// SAFETY: as above.
unsafe impl Sync for PulseDevice {}

impl PulseDevice {
    fn lib(&self) -> &PulseLib {
        &self.shared.lib
    }

    fn buffers(&self) -> std::sync::MutexGuard<'_, StreamBuffers> {
        self.buffers.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn stream_state(&self) -> c_int {
        // SAFETY: the stream is live; we hold the lock.
        unsafe { (self.lib().pa_stream_get_state)(self.stream) }
    }

    /// Translation of `PULSEAUDIO_CloseDevice()` (also the cleanup of a
    /// failed open, which upstream leaves to the core's call of it).
    fn close(&self) {
        self.shared.lock();

        if !self.stream.is_null() {
            if !self.buffers().recordingbuf.is_null() {
                // SAFETY: the stream is live; we hold the lock.
                unsafe { (self.lib().pa_stream_drop)(self.stream) };
            }
            // FIXME (upstream): WriteCallback's userdata (the device's
            // private data) is freed right after this while libpulse may
            // still call it; the write callback is unregistered first here.
            // SAFETY: the stream is live and released once, here.
            unsafe {
                (self.lib().pa_stream_set_write_callback)(self.stream, None, ptr::null_mut());
                (self.lib().pa_stream_disconnect)(self.stream);
                (self.lib().pa_stream_unref)(self.stream);
            }
        }
        self.shared.signal(); // in case the device thread is waiting somewhere, this will unblock it.
        self.shared.unlock();
    }
}

/// Translation of `WriteCallback()`; the userdata is the device.
unsafe extern "C" fn write_callback(_p: *mut PaStream, nbytes: usize, userdata: *mut c_void) {
    // SAFETY: the userdata is the `PulseDevice` that owns the stream, alive
    // until the stream is released (see `PulseDevice::close`).
    let h = unsafe { &*userdata.cast::<PulseDevice>() };
    //SDL_Log("PULSEAUDIO WRITE CALLBACK! nbytes=%u", (unsigned int) nbytes);
    h.bytes_requested.fetch_add(nbytes as i32, Ordering::AcqRel);
    h.shared.signal();
}

/// Translation of `ReadCallback()`; the userdata is the `PulseShared`
/// (upstream passes the private data but uses only the global main loop).
unsafe extern "C" fn read_callback(_p: *mut PaStream, _nbytes: usize, userdata: *mut c_void) {
    // SAFETY: the userdata is the live `PulseShared`, which outlives every
    // stream (they're released before the context is disconnected).
    let shared = unsafe { &*userdata.cast::<PulseShared>() };
    //SDL_Log("PULSEAUDIO READ CALLBACK! nbytes=%u", (unsigned int) nbytes);
    shared.signal(); // the recording code queries what it needs, we just need to signal to end any wait
}

/// Translation of `PulseStreamStateChangeCallback()`; the userdata is the
/// `PulseShared` (upstream's NULL userdata and global main loop).
unsafe extern "C" fn pulse_stream_state_change_callback(
    _stream: *mut PaStream,
    userdata: *mut c_void,
) {
    // SAFETY: as in `read_callback`.
    let shared = unsafe { &*userdata.cast::<PulseShared>() };
    shared.signal(); // just signal any waiting code, it can look up the details.
}

impl DeviceBackend for PulseDevice {
    /// Translation of `PULSEAUDIO_WaitDevice()`: this function waits until
    /// it is possible to write a full sound buffer.
    fn wait_device(&self, device: &PhysicalDevice) -> bool {
        let mut result = true;

        //SDL_Log("PULSEAUDIO WAITDEVICE START! mixlen=%d", available);

        self.shared.lock();

        while !device.shutting_down() && self.bytes_requested.load(Ordering::Acquire) == 0 {
            //SDL_Log("PULSEAUDIO WAIT IN WAITDEVICE!");
            self.shared.wait();

            if self.shared.context_state() != PA_CONTEXT_READY
                || self.stream_state() != PA_STREAM_READY
            {
                //SDL_Log("PULSEAUDIO DEVICE FAILURE IN WAITDEVICE!");
                result = false;
                break;
            }
        }

        self.shared.unlock();

        result
    }

    /// Translation of `PULSEAUDIO_PlayDevice()`.
    fn play_device(&self, _device: &PhysicalDevice, buffer: &[u8]) -> bool {
        //SDL_Log("PULSEAUDIO PLAYDEVICE START! mixlen=%d", available);

        let buffer_size = buffer.len();
        crate::sdl_assert!(self.bytes_requested.load(Ordering::Acquire) >= buffer_size as i32);

        // (GetDeviceBuf may have handed out libpulse's own buffer; the core
        // mixed into its buffer, so move the data there and write that.)
        let (write_buf, write_len) = {
            let mut b = self.buffers();
            let r = (b.write_buf, b.write_len);
            b.write_buf = ptr::null_mut();
            b.write_len = 0;
            r
        };
        let data: *const u8 = if !write_buf.is_null() && buffer_size <= write_len {
            // SAFETY: `write_buf` holds `write_len` bytes from pa_stream_begin_write().
            unsafe { ptr::copy_nonoverlapping(buffer.as_ptr(), write_buf, buffer_size) };
            write_buf
        } else {
            buffer.as_ptr()
        };

        self.shared.lock();
        // SAFETY: the stream is live; `data` holds `buffer_size` bytes; we hold the lock.
        let rc = unsafe {
            (self.lib().pa_stream_write)(
                self.stream,
                data.cast(),
                buffer_size,
                None,
                0,
                PA_SEEK_RELATIVE,
            )
        };
        self.shared.unlock();

        if rc < 0 {
            return false;
        }

        //SDL_Log("PULSEAUDIO FEED! nbytes=%d", buffer_size);
        // FIXME (upstream): this is updated without the main loop lock,
        // racing WriteCallback's update; an atomic keeps both here.
        self.bytes_requested
            .fetch_sub(buffer_size as i32, Ordering::AcqRel);

        //SDL_Log("PULSEAUDIO PLAYDEVICE END! written=%d", written);
        true
    }

    /// Translation of `PULSEAUDIO_GetDeviceBuf()`.
    fn get_device_buf(&self, _device: &PhysicalDevice, buffer_size: usize) -> Option<usize> {
        let reqsize =
            (buffer_size as i32).min(self.bytes_requested.load(Ordering::Acquire)) as usize;
        let mut nbytes = reqsize;
        let mut data: *mut c_void = ptr::null_mut();
        // FIXME (upstream): pa_stream_begin_write() is called without the main loop lock.
        // SAFETY: the stream is live; `data` and `nbytes` are valid out-pointers.
        if unsafe { (self.lib().pa_stream_begin_write)(self.stream, &mut data, &mut nbytes) } == 0 {
            let mut b = self.buffers();
            b.write_buf = data.cast();
            b.write_len = nbytes;
            return Some(nbytes);
        }

        // don't know why this would fail, but we'll fall back just in case.
        Some(reqsize)
    }

    /// Translation of `PULSEAUDIO_WaitRecordingDevice()`.
    fn wait_recording_device(&self, device: &PhysicalDevice) -> bool {
        if !self.buffers().recordingbuf.is_null() {
            return true; // there's still data available to read.
        }

        let mut result = true;

        self.shared.lock();

        while !device.shutting_down() {
            self.shared.wait();
            // SAFETY (the stream calls): the stream is live; we hold the lock.
            if self.shared.context_state() != PA_CONTEXT_READY
                || self.stream_state() != PA_STREAM_READY
            {
                //SDL_Log("PULSEAUDIO DEVICE FAILURE IN WAITRECORDINGDEVICE!");
                result = false;
                break;
            } else if unsafe { (self.lib().pa_stream_readable_size)(self.stream) } > 0 {
                // a new fragment is available!
                let mut data: *const c_void = ptr::null();
                let mut nbytes: usize = 0;
                // SAFETY: as above; `data` and `nbytes` are valid out-pointers.
                unsafe { (self.lib().pa_stream_peek)(self.stream, &mut data, &mut nbytes) };
                crate::sdl_assert!(nbytes > 0);
                if data.is_null() {
                    // If NULL, then the buffer had a hole, ignore that
                    // SAFETY: as above.
                    unsafe { (self.lib().pa_stream_drop)(self.stream) }; // drop this fragment.
                } else {
                    // store this fragment's data for use with RecordDevice
                    //SDL_Log("PULSEAUDIO: recorded %d new bytes", (int) nbytes);
                    let mut b = self.buffers();
                    b.recordingbuf = data.cast();
                    b.recordinglen = nbytes;
                    break;
                }
            }
        }

        self.shared.unlock();

        result
    }

    /// Translation of `PULSEAUDIO_RecordDevice()`.
    fn record_device(&self, _device: &PhysicalDevice, buffer: &mut [u8]) -> Result<usize> {
        let mut b = self.buffers();
        if !b.recordingbuf.is_null() {
            let cpy = buffer.len().min(b.recordinglen);
            if cpy > 0 {
                //SDL_Log("PULSEAUDIO: fed %d recorded bytes", cpy);
                // SAFETY: `recordingbuf` holds `recordinglen` bytes from
                // pa_stream_peek(), valid until pa_stream_drop().
                unsafe { ptr::copy_nonoverlapping(b.recordingbuf, buffer.as_mut_ptr(), cpy) };
                // SAFETY: still within the fragment.
                b.recordingbuf = unsafe { b.recordingbuf.add(cpy) };
                b.recordinglen -= cpy;
            }
            if b.recordinglen == 0 {
                b.recordingbuf = ptr::null();
                drop(b);
                self.shared.lock(); // don't know if you _have_ to lock for this, but just in case.
                                    // SAFETY: the stream is live; we hold the lock.
                unsafe { (self.lib().pa_stream_drop)(self.stream) }; // done with this fragment.
                self.shared.unlock();
            }
            return Ok(cpy); // new data, return it.
        }

        Ok(0)
    }

    /// Translation of `PULSEAUDIO_FlushRecording()`.
    fn flush_recording(&self, device: &PhysicalDevice) {
        let mut data: *const c_void = ptr::null();
        let mut nbytes: usize = 0;

        self.shared.lock();

        {
            let mut b = self.buffers();
            if !b.recordingbuf.is_null() {
                // SAFETY: the stream is live; we hold the lock.
                unsafe { (self.lib().pa_stream_drop)(self.stream) };
                b.recordingbuf = ptr::null();
                b.recordinglen = 0;
            }
        }

        // SAFETY (the stream calls): the stream is live; we hold the lock.
        let mut buflen = unsafe { (self.lib().pa_stream_readable_size)(self.stream) };
        while !device.shutting_down() && buflen > 0 {
            self.shared.wait();
            if self.shared.context_state() != PA_CONTEXT_READY
                || self.stream_state() != PA_STREAM_READY
            {
                //SDL_Log("PULSEAUDIO DEVICE FAILURE IN FLUSHRECORDING!");
                audio_device_disconnected(device);
                break;
            }

            // a fragment of audio present before FlushCapture was call is
            // still available! Just drop it.
            // SAFETY: as above; `data` and `nbytes` are valid out-pointers.
            unsafe {
                (self.lib().pa_stream_peek)(self.stream, &mut data, &mut nbytes);
                (self.lib().pa_stream_drop)(self.stream);
            }
            buflen = buflen.wrapping_sub(nbytes);
        }

        self.shared.unlock();
    }

    /// Translation of `PULSEAUDIO_CloseDevice()`.
    fn close_device(&self, _device: &PhysicalDevice) {
        self.close();
    }
}

// Channel maps that match the order in SDL_Audio.h
const PULSE_MAP_1: &[c_int] = &[PA_CHANNEL_POSITION_MONO];
const PULSE_MAP_2: &[c_int] = &[
    PA_CHANNEL_POSITION_FRONT_LEFT,
    PA_CHANNEL_POSITION_FRONT_RIGHT,
];

const PULSE_MAP_3: &[c_int] = &[
    PA_CHANNEL_POSITION_FRONT_LEFT,
    PA_CHANNEL_POSITION_FRONT_RIGHT,
    PA_CHANNEL_POSITION_LFE,
];

const PULSE_MAP_4: &[c_int] = &[
    PA_CHANNEL_POSITION_FRONT_LEFT,
    PA_CHANNEL_POSITION_FRONT_RIGHT,
    PA_CHANNEL_POSITION_REAR_LEFT,
    PA_CHANNEL_POSITION_REAR_RIGHT,
];

const PULSE_MAP_5: &[c_int] = &[
    PA_CHANNEL_POSITION_FRONT_LEFT,
    PA_CHANNEL_POSITION_FRONT_RIGHT,
    PA_CHANNEL_POSITION_LFE,
    PA_CHANNEL_POSITION_REAR_LEFT,
    PA_CHANNEL_POSITION_REAR_RIGHT,
];

const PULSE_MAP_6: &[c_int] = &[
    PA_CHANNEL_POSITION_FRONT_LEFT,
    PA_CHANNEL_POSITION_FRONT_RIGHT,
    PA_CHANNEL_POSITION_FRONT_CENTER,
    PA_CHANNEL_POSITION_LFE,
    PA_CHANNEL_POSITION_REAR_LEFT,
    PA_CHANNEL_POSITION_REAR_RIGHT,
];

const PULSE_MAP_7: &[c_int] = &[
    PA_CHANNEL_POSITION_FRONT_LEFT,
    PA_CHANNEL_POSITION_FRONT_RIGHT,
    PA_CHANNEL_POSITION_FRONT_CENTER,
    PA_CHANNEL_POSITION_LFE,
    PA_CHANNEL_POSITION_REAR_CENTER,
    PA_CHANNEL_POSITION_SIDE_LEFT,
    PA_CHANNEL_POSITION_SIDE_RIGHT,
];

const PULSE_MAP_8: &[c_int] = &[
    PA_CHANNEL_POSITION_FRONT_LEFT,
    PA_CHANNEL_POSITION_FRONT_RIGHT,
    PA_CHANNEL_POSITION_FRONT_CENTER,
    PA_CHANNEL_POSITION_LFE,
    PA_CHANNEL_POSITION_REAR_LEFT,
    PA_CHANNEL_POSITION_REAR_RIGHT,
    PA_CHANNEL_POSITION_SIDE_LEFT,
    PA_CHANNEL_POSITION_SIDE_RIGHT,
];

/// Translation of `PulseCreateChannelMap()`.
fn pulse_create_channel_map(pacmap: &mut PaChannelMap, channels: u8) {
    crate::sdl_assert!(channels as usize <= PA_CHANNELS_MAX);

    pacmap.channels = channels;

    let map = match channels {
        1 => PULSE_MAP_1,
        2 => PULSE_MAP_2,
        3 => PULSE_MAP_3,
        4 => PULSE_MAP_4,
        5 => PULSE_MAP_5,
        6 => PULSE_MAP_6,
        7 => PULSE_MAP_7,
        8 => PULSE_MAP_8,
        _ => &[],
    };
    pacmap.map[..map.len()].copy_from_slice(map); // COPY_CHANNEL_MAP(c)
}

/// The PulseAudio sample format for an SDL format (the `switch` in
/// `PULSEAUDIO_OpenDevice()`).
fn sdl_format_to_pulse_format(format: AudioFormat) -> Option<c_int> {
    Some(match format {
        AudioFormat::U8 => PA_SAMPLE_U8,
        AudioFormat::S16LE => PA_SAMPLE_S16LE,
        AudioFormat::S16BE => PA_SAMPLE_S16BE,
        AudioFormat::S32LE => PA_SAMPLE_S32LE,
        AudioFormat::S32BE => PA_SAMPLE_S32BE,
        AudioFormat::F32LE => PA_SAMPLE_FLOAT32LE,
        AudioFormat::F32BE => PA_SAMPLE_FLOAT32BE,
        _ => return None,
    })
}

/// Translation of `PulseFormatToSDLFormat()`.
fn pulse_format_to_sdl_format(format: c_int) -> AudioFormat {
    match format {
        PA_SAMPLE_U8 => AudioFormat::U8,
        PA_SAMPLE_S16LE => AudioFormat::S16LE,
        PA_SAMPLE_S16BE => AudioFormat::S16BE,
        PA_SAMPLE_S32LE => AudioFormat::S32LE,
        PA_SAMPLE_S32BE => AudioFormat::S32BE,
        PA_SAMPLE_FLOAT32LE => AudioFormat::F32LE,
        PA_SAMPLE_FLOAT32BE => AudioFormat::F32BE,
        _ => AudioFormat::UNKNOWN,
    }
}

impl PulseShared {
    /// Translation of `AddPulseAudioDevice()`.
    fn add_pulse_audio_device(
        &self,
        recording: bool,
        description: &str,
        name: &str,
        index: u32,
        sample_spec: &PaSampleSpec,
    ) {
        let spec = AudioSpec::new(
            pulse_format_to_sdl_format(sample_spec.format),
            sample_spec.channels as i32,
            sample_spec.rate as i32,
        );
        let handle = self.next_handle.fetch_add(1, Ordering::Relaxed);
        self.handles
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(
                handle,
                PulseDeviceHandle {
                    device_path: name.to_owned(),
                    device_index: index,
                },
            );
        // FIXME (upstream): if SDL_AddAudioDevice() fails, the handle is leaked.
        let _ = add_audio_device(recording, description, None, Some(&spec), handle);
    }
}

/// Translation of `SinkInfoCallback()`: this is called when PulseAudio
/// adds an playback ("sink") device.
unsafe extern "C" fn sink_info_callback(
    _c: *mut PaContext,
    i: *const PaSinkInfo,
    _is_last: c_int,
    data: *mut c_void,
) {
    // SAFETY: the userdata is the live `PulseShared` (see the registrations).
    let shared = unsafe { &*data.cast::<PulseShared>() };
    if !i.is_null() {
        // SAFETY: libpulse passes a valid sink info for this call.
        let i = unsafe { &*i };
        // SAFETY: the strings are NUL-terminated (or NULL).
        let (description, name) = unsafe { (opt_str(i.description), opt_str(i.name)) };
        shared.add_pulse_audio_device(
            false,
            &description.unwrap_or_default(),
            &name.unwrap_or_default(),
            i.index,
            &i.sample_spec,
        );
    }
    shared.signal();
}

/// Translation of `SourceInfoCallback()`: this is called when PulseAudio
/// adds a recording ("source") device.
unsafe extern "C" fn source_info_callback(
    _c: *mut PaContext,
    i: *const PaSourceInfo,
    _is_last: c_int,
    data: *mut c_void,
) {
    // SAFETY: the userdata is the live `PulseShared` (see the registrations).
    let shared = unsafe { &*data.cast::<PulseShared>() };
    // Maybe skip "monitor" sources. These are just output from other sinks.
    if !i.is_null() {
        // SAFETY: libpulse passes a valid source info for this call.
        let i = unsafe { &*i };
        if shared.include_monitors || i.monitor_of_sink == PA_INVALID_INDEX {
            // SAFETY: the strings are NUL-terminated (or NULL).
            let (description, name) = unsafe { (opt_str(i.description), opt_str(i.name)) };
            shared.add_pulse_audio_device(
                true,
                &description.unwrap_or_default(),
                &name.unwrap_or_default(),
                i.index,
                &i.sample_spec,
            );
        }
    }
    shared.signal();
}

/// Translation of `ServerInfoCallback()`.
unsafe extern "C" fn server_info_callback(
    _c: *mut PaContext,
    i: *const PaServerInfo,
    data: *mut c_void,
) {
    // SAFETY: the userdata is the live `PulseShared` (see the registrations).
    let shared = unsafe { &*data.cast::<PulseShared>() };
    //SDL_Log("PULSEAUDIO ServerInfoCallback!");

    if !i.is_null() {
        // SAFETY: libpulse passes a valid server info; its strings are
        // NUL-terminated or NULL.
        let (sink, source) = unsafe {
            (
                opt_str((*i).default_sink_name),
                opt_str((*i).default_source_name),
            )
        };
        let mut d = shared.defaults.lock().unwrap_or_else(|e| e.into_inner());
        // FIXME (upstream): a NULL default name (no sink/source) is passed to
        // SDL_strcmp(); here it's skipped, like a failed SDL_strdup().
        if let Some(str) = sink {
            if d.default_sink_path.as_deref() != Some(&str) {
                d.default_sink_path = Some(str);
                d.default_sink_changed = true;
            }
        }

        if let Some(str) = source {
            if d.default_source_path.as_deref() != Some(&str) {
                d.default_source_path = Some(str);
                d.default_source_changed = true;
            }
        }
    }

    shared.signal();
}

impl PulseShared {
    /// Translation of `FindAudioDeviceByIndex()`.
    fn find_audio_device_by_index(&self, device: &PhysicalDevice, idx: u32) -> bool {
        self.handle_of(device)
            .is_some_and(|h| h.device_index == idx)
    }

    /// Translation of `FindAudioDeviceByPath()`.
    fn find_audio_device_by_path(&self, device: &PhysicalDevice, path: &str) -> bool {
        self.handle_of(device)
            .is_some_and(|h| h.device_path == path)
    }

    /// Translation of `CheckDefaultDevice()`.
    fn check_default_device(&self, changed: bool, device_path: Option<&str>) -> bool {
        if !changed {
            return false; // nothing's happening, leave the flag marked as unchanged.
        }
        let Some(device_path) = device_path else {
            return true; // check again later, we don't have a device name...
        };

        if let Ok(device) = find_physical_audio_device_by_callback(|d| {
            self.find_audio_device_by_path(d, device_path)
        }) {
            // if NULL, we might still be waiting for a SinkInfoCallback or something, we'll try later.
            default_audio_device_changed(&device);
            return false; // changing complete, set flag to unchanged for future tests.
        }
        true // couldn't find the changed device, leave it marked as changed to try again later.
    }
}

/// Translation of `HotplugCallback()`: this is called when PulseAudio has a
/// device connected/removed/changed.
unsafe extern "C" fn hotplug_callback(c: *mut PaContext, t: c_uint, idx: u32, data: *mut c_void) {
    // SAFETY: the userdata is the live `PulseShared` (see the registration).
    let shared = unsafe { &*data.cast::<PulseShared>() };
    let lib = &shared.lib;
    let added = (t & PA_SUBSCRIPTION_EVENT_TYPE_MASK) == PA_SUBSCRIPTION_EVENT_NEW;
    let removed = (t & PA_SUBSCRIPTION_EVENT_TYPE_MASK) == PA_SUBSCRIPTION_EVENT_REMOVE;
    let changed = (t & PA_SUBSCRIPTION_EVENT_TYPE_MASK) == PA_SUBSCRIPTION_EVENT_CHANGE;

    if added || removed || changed {
        // we only care about add/remove events.
        let sink = (t & PA_SUBSCRIPTION_EVENT_FACILITY_MASK) == PA_SUBSCRIPTION_EVENT_SINK;
        let source = (t & PA_SUBSCRIPTION_EVENT_FACILITY_MASK) == PA_SUBSCRIPTION_EVENT_SOURCE;

        // SAFETY (the operations): this runs on the main loop thread with
        // its lock held; the callbacks' userdata is the live `PulseShared`.
        if changed {
            unsafe {
                (lib.pa_operation_unref)((lib.pa_context_get_server_info)(
                    c,
                    Some(server_info_callback),
                    data,
                ))
            };
        }

        /* adds need sink details from the PulseAudio server. Another callback...
        (just unref all these operations right away, because we aren't going to wait on them
        and their callbacks will handle any work, so they can free as soon as that happens.) */
        if added && sink {
            // SAFETY: as above.
            unsafe {
                (lib.pa_operation_unref)((lib.pa_context_get_sink_info_by_index)(
                    c,
                    idx,
                    Some(sink_info_callback),
                    data,
                ))
            };
        } else if added && source {
            // SAFETY: as above.
            unsafe {
                (lib.pa_operation_unref)((lib.pa_context_get_source_info_by_index)(
                    c,
                    idx,
                    Some(source_info_callback),
                    data,
                ))
            };
        } else if removed && (sink || source) {
            // removes we can handle just with the device index.
            // FIXME (upstream): sinks and sources are numbered separately, so
            // this can match (and disconnect) a device of the other kind.
            if let Ok(device) = find_physical_audio_device_by_callback(|d| {
                shared.find_audio_device_by_index(d, idx)
            }) {
                audio_device_disconnected(&device);
            }
        }
    }
    shared.signal();
}

/// Translation of `HotplugThread()`: this runs as a thread while the Pulse
/// target is initialized to catch hotplug events.
fn hotplug_thread(shared: Arc<PulseShared>, ready_sem: Arc<Semaphore>) -> i32 {
    let lib = &shared.lib;

    let _ = crate::thread::set_current_thread_priority(ThreadPriority::Low);
    shared.lock();
    // SAFETY: the context is live; we hold the lock; the userdata is `shared`,
    // which outlives the subscription (removed at the end of this thread).
    unsafe {
        (lib.pa_context_set_subscribe_callback)(
            shared.context,
            Some(hotplug_callback),
            shared.userdata(),
        )
    };

    // don't WaitForPulseOperation on the subscription; when it's done we'll be able to get hotplug events, but waiting doesn't changing anything.
    // SAFETY: as above.
    let mut op = unsafe {
        (lib.pa_context_subscribe)(
            shared.context,
            PA_SUBSCRIPTION_MASK_SINK | PA_SUBSCRIPTION_MASK_SOURCE | PA_SUBSCRIPTION_MASK_SERVER,
            None,
            ptr::null_mut(),
        )
    };

    ready_sem.signal();

    while shared.hotplug_thread_active.load(Ordering::Acquire) {
        shared.wait();
        // SAFETY: `op` is our live reference; we hold the lock.
        if !op.is_null() && unsafe { (lib.pa_operation_get_state)(op) } != PA_OPERATION_RUNNING {
            // SAFETY: as above; released once.
            unsafe { (lib.pa_operation_unref)(op) };
            op = ptr::null_mut();
        }

        // Update default devices; don't hold the pulse lock during this, since it could deadlock vs a playing device that we're about to lock here.
        let (
            mut check_default_sink,
            mut check_default_source,
            current_default_sink,
            current_default_source,
        ) = {
            let mut d = shared.defaults.lock().unwrap_or_else(|e| e.into_inner());
            let check_default_sink = d.default_sink_changed;
            let check_default_source = d.default_source_changed;
            let current_default_sink = if check_default_sink {
                d.default_sink_path.clone()
            } else {
                None
            };
            let current_default_source = if check_default_source {
                d.default_source_path.clone()
            } else {
                None
            };
            d.default_sink_changed = false;
            d.default_source_changed = false;
            (
                check_default_sink,
                check_default_source,
                current_default_sink,
                current_default_source,
            )
        };
        shared.unlock();
        check_default_sink =
            shared.check_default_device(check_default_sink, current_default_sink.as_deref());
        check_default_source =
            shared.check_default_device(check_default_source, current_default_source.as_deref());
        shared.lock();

        // (our copies are dropped here; they're None if nothing changed)

        // set these to true if we didn't handle the change OR there was _another_ change while we were working unlocked.
        let mut d = shared.defaults.lock().unwrap_or_else(|e| e.into_inner());
        d.default_sink_changed = d.default_sink_changed || check_default_sink;
        d.default_source_changed = d.default_source_changed || check_default_source;
    }

    if !op.is_null() {
        // SAFETY: our live reference, released once.
        unsafe { (lib.pa_operation_unref)(op) };
    }

    // SAFETY: the context is live; we hold the lock.
    unsafe { (lib.pa_context_set_subscribe_callback)(shared.context, None, ptr::null_mut()) };
    shared.unlock();
    0
}

/// The PulseAudio driver (`PULSEAUDIO_Init()`).
struct PulseAudio {
    shared: Arc<PulseShared>,
    /// `pulseaudio_hotplug_thread`
    hotplug_thread: Mutex<Option<Thread>>,
}

impl AudioDriverImpl for PulseAudio {
    fn flags(&self) -> DriverFlags {
        DriverFlags {
            has_recording_support: true,
            ..DriverFlags::default()
        }
    }

    /// Translation of `PULSEAUDIO_DetectDevices()`.
    fn detect_devices(&self) -> (Option<Arc<PhysicalDevice>>, Option<Arc<PhysicalDevice>>) {
        let shared = &self.shared;
        let lib = &shared.lib;
        let ready_sem = Arc::new(Semaphore::new(0));

        shared.lock();
        // SAFETY (the three operations): the context is live; we hold the
        // lock; the userdata is `shared`, alive past these operations.
        unsafe {
            shared.wait_for_pulse_operation((lib.pa_context_get_server_info)(
                shared.context,
                Some(server_info_callback),
                shared.userdata(),
            ));
            shared.wait_for_pulse_operation((lib.pa_context_get_sink_info_list)(
                shared.context,
                Some(sink_info_callback),
                shared.userdata(),
            ));
            shared.wait_for_pulse_operation((lib.pa_context_get_source_info_list)(
                shared.context,
                Some(source_info_callback),
                shared.userdata(),
            ));
        }
        shared.unlock();

        let (default_sink_path, default_source_path) = {
            let d = shared.defaults.lock().unwrap_or_else(|e| e.into_inner());
            (d.default_sink_path.clone(), d.default_source_path.clone())
        };

        let mut default_playback = None;
        if let Some(path) = default_sink_path {
            default_playback = find_physical_audio_device_by_callback(|d| {
                shared.find_audio_device_by_path(d, &path)
            })
            .ok();
        }

        let mut default_recording = None;
        if let Some(path) = default_source_path {
            default_recording = find_physical_audio_device_by_callback(|d| {
                shared.find_audio_device_by_path(d, &path)
            })
            .ok();
        }

        // ok, we have a sane list, let's set up hotplug notifications now...
        shared.hotplug_thread_active.store(true, Ordering::Release);
        let thread_shared = shared.clone();
        let thread_sem = ready_sem.clone();
        match Thread::spawn("PulseHotplug", move || {
            hotplug_thread(thread_shared, thread_sem)
        }) {
            Ok(thread) => {
                *self
                    .hotplug_thread
                    .lock()
                    .unwrap_or_else(|e| e.into_inner()) = Some(thread);
                ready_sem.wait(); // wait until the thread hits it's main loop.
            }
            Err(_) => {
                shared.hotplug_thread_active.store(false, Ordering::Release); // thread failed to start, we'll go on without hotplug.
            }
        }

        (default_playback, default_recording)
    }

    /// Translation of `PULSEAUDIO_OpenDevice()`.
    fn open_device(
        &self,
        device: &PhysicalDevice,
        state: &mut PhysState,
    ) -> Result<Arc<dyn DeviceBackend>> {
        let shared = &self.shared;
        let lib = &shared.lib;
        let recording = device.recording;
        let mut flags: c_uint = 0;
        let mut format = PA_SAMPLE_INVALID;

        crate::sdl_assert!(!shared.mainloop.is_null());
        crate::sdl_assert!(!shared.context.is_null());

        // Try for a closest match on audio format
        let mut test_format = AudioFormat::UNKNOWN;
        for &closefmt in state.spec.format.closest_formats() {
            // (#ifdef DEBUG_AUDIO: SDL_Log("pulseaudio: Trying format 0x%4.4x", test_format);)
            if let Some(f) = sdl_format_to_pulse_format(closefmt) {
                format = f;
                test_format = closefmt;
                break;
            }
        }
        if test_format == AudioFormat::UNKNOWN {
            return Err(Error::new("pulseaudio: Unsupported audio format"));
        }
        state.spec.format = test_format;
        let mut paspec = PaSampleSpec {
            format,
            ..PaSampleSpec::default()
        };

        // Calculate the final parameters for this audio specification
        updated_audio_device_format(state);

        // (the core allocates the silenced mixing buffer)

        paspec.channels = state.spec.channels as u8;
        paspec.rate = state.spec.freq as u32;

        // Reduced prebuffering compared to the defaults.

        let buffer_size = state.buffer_size as u32;
        let paattr = PaBufferAttr {
            fragsize: buffer_size * 2, // despite the name, this is only used for recording devices, according to PulseAudio docs!  (times 2 because we want _more_ than our buffer size sent from the server at a time, which helps some drivers).
            tlength: buffer_size,
            prebuf: u32::MAX,
            maxlength: u32::MAX,
            minreq: u32::MAX,
        };
        flags |= PA_STREAM_ADJUST_LATENCY;

        shared.lock();

        let name = crate::hints::get(crate::hints::AUDIO_DEVICE_STREAM_NAME)
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| "Audio Stream".to_owned()); // stream description
        let name = CString::new(name).unwrap_or_default();

        let mut pacmap = PaChannelMap {
            channels: 0,
            map: [0; PA_CHANNELS_MAX],
        };
        pulse_create_channel_map(&mut pacmap, state.spec.channels as u8);

        // SAFETY: the context is live; we hold the lock; the spec and map are valid.
        let stream = unsafe {
            (lib.pa_stream_new)(
                shared.context,
                name.as_ptr(),
                &paspec, // sample format spec
                &pacmap, // channel map
            )
        };

        if stream.is_null() {
            shared.unlock();
            return Err(Error::new("Could not set up PulseAudio stream"));
        }

        let h = Arc::new(PulseDevice {
            shared: shared.clone(),
            stream,
            bytes_requested: AtomicI32::new(0),
            buffers: Mutex::new(StreamBuffers {
                write_buf: ptr::null_mut(),
                write_len: 0,
                recordingbuf: ptr::null(),
                recordinglen: 0,
            }),
        });
        let userdata: *mut c_void = Arc::as_ptr(&h).cast_mut().cast();

        let result = (|| {
            // SAFETY (the stream calls): the stream is live; we hold the lock;
            // the write callback's userdata is `h`, alive until the stream is
            // released, the others' is `shared`, which outlives the stream.
            unsafe {
                (lib.pa_stream_set_state_callback)(
                    stream,
                    Some(pulse_stream_state_change_callback),
                    shared.userdata(),
                )
            };

            // SDL manages device moves if the default changes, so don't ever let Pulse automatically migrate this stream.
            // UPDATE: This prevents users from moving the audio to a new sink (device) using standard tools. This is slightly in conflict
            //  with how SDL wants to manage audio devices, but if people want to do it, we should let them, so this is commented out
            //  for now. We might revisit later.
            //flags |= PA_STREAM_DONT_MOVE;

            let device_path = shared
                .handle_of(device)
                .map(|h| h.device_path)
                .unwrap_or_default();
            let device_path = CString::new(device_path).unwrap_or_default();
            let rc = if recording {
                // SAFETY: as above.
                unsafe {
                    (lib.pa_stream_set_read_callback)(
                        stream,
                        Some(read_callback),
                        shared.userdata(),
                    );
                    (lib.pa_stream_connect_record)(stream, device_path.as_ptr(), &paattr, flags)
                }
            } else {
                // SAFETY: as above.
                unsafe {
                    (lib.pa_stream_set_write_callback)(stream, Some(write_callback), userdata);
                    (lib.pa_stream_connect_playback)(
                        stream,
                        device_path.as_ptr(),
                        &paattr,
                        flags,
                        ptr::null(),
                        ptr::null_mut(),
                    )
                }
            };

            if rc < 0 {
                return Err(Error::new("Could not connect PulseAudio stream"));
            }
            let mut stream_state = h.stream_state();
            while pa_stream_is_good(stream_state) && stream_state != PA_STREAM_READY {
                shared.wait();
                stream_state = h.stream_state();
            }

            if !pa_stream_is_good(stream_state) {
                return Err(Error::new("Could not connect PulseAudio stream"));
            }
            // SAFETY: as above.
            let actual_bufattr = unsafe { (lib.pa_stream_get_buffer_attr)(stream) };
            if actual_bufattr.is_null() {
                return Err(Error::new(
                    "Could not determine connected PulseAudio stream's buffer attributes",
                ));
            }
            // SAFETY: libpulse returned a valid buffer attribute struct.
            let actual_bufattr = unsafe { &*actual_bufattr };
            state.buffer_size = if recording {
                actual_bufattr.fragsize
            } else {
                actual_bufattr.tlength
            } as usize;
            state.sample_frames = (state.buffer_size / state.spec.frame_size()) as i32;
            Ok(())
        })();

        shared.unlock();

        match result {
            // We're (hopefully) ready to rock and roll. :-)
            Ok(()) => Ok(h),
            Err(e) => {
                h.close(); // (the core's PULSEAUDIO_CloseDevice() call)
                Err(e)
            }
        }
    }

    /// Translation of `PULSEAUDIO_FreeDeviceHandle()`.
    fn free_device_handle(&self, device: &PhysicalDevice) {
        self.shared
            .handles
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&device.handle);
    }

    /// Translation of `PULSEAUDIO_DeinitializeStart()`.
    fn deinitialize_start(&self) {
        let thread = self
            .hotplug_thread
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        if let Some(thread) = thread {
            self.shared.lock();
            self.shared
                .hotplug_thread_active
                .store(false, Ordering::Release);
            self.shared.signal();
            self.shared.unlock();
            thread.wait();
        }
    }

    /// Translation of `PULSEAUDIO_Deinitialize()`.
    fn deinitialize(&self) {
        self.shared.disconnect_from_pulse_server();

        *self
            .shared
            .defaults
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Defaults::default();

        // (UnloadPulseAudioLibrary(): the library goes with the last reference to it.)
    }
}

/// Translation of `PULSEAUDIO_Init()`.
fn pulseaudio_init() -> Option<Arc<dyn AudioDriverImpl>> {
    let lib = load_pulseaudio_library().ok()?;
    let connection = connect_to_pulse_server(&lib).ok()?; // (UnloadPulseAudioLibrary() on failure is dropping `lib`)

    let include_monitors = crate::hints::get_bool(crate::hints::AUDIO_INCLUDE_MONITORS, false);

    Some(Arc::new(PulseAudio {
        shared: Arc::new(PulseShared {
            lib,
            mainloop: connection.mainloop,
            context: connection.context,
            _context_signal: connection.context_signal,
            include_monitors,
            defaults: Mutex::new(Defaults::default()),
            handles: Mutex::new(HashMap::new()),
            next_handle: AtomicUsize::new(1),
            hotplug_thread_active: AtomicBool::new(false),
        }),
        hotplug_thread: Mutex::new(None),
    }))
}

/// Translation of `PULSEAUDIO_bootstrap`.
pub(crate) static PULSEAUDIO_BOOTSTRAP: AudioBootStrap = AudioBootStrap {
    name: "pulseaudio",
    desc: "PulseAudio",
    init: pulseaudio_init,
    demand_only: false,
    is_preferred: false,
};

#[cfg(test)]
mod tests;
