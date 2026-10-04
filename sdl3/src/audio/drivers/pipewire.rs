// Rust translation of src/audio/pipewire/SDL_pipewire.c and
// src/audio/pipewire/SDL_pipewire.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

// The PipeWire driver. libpipewire is loaded at run time
// (SDL_AUDIO_DRIVER_PIPEWIRE_DYNAMIC) and only the parts of its API SDL uses
// are declared here; the static inline helpers of the SPA headers (pods,
// JSON, dictionaries) and the interface method macros (pw_core_sync(),
// pw_registry_bind(), ...) are translated in `spa` and below.
//
// Upstream keeps the hotplug loop and its lists in file-level statics and
// gives the callbacks NULL (or the node) as userdata; here the hotplug
// state is one heap object the callbacks get as their userdata, and its
// lists are Vecs. Node objects live in Boxes instead of their proxy's user
// data, and the stream callbacks get the device's backend data.

mod spa;

use std::cell::UnsafeCell;
use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::ptr;
use std::sync::atomic::{AtomicI32, AtomicPtr, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};

use spa::{
    spa_dict_lookup, spa_format_audio_raw_build, spa_format_audio_raw_parse, spa_pod_find_prop,
    spa_pod_get_int, spa_pod_get_values, Pod, SpaAudioInfoRaw, SpaDict, SpaDictItem, SpaJson,
    SpaPodBuilder, SPA_AUDIO_FORMAT_UNKNOWN, SPA_CHOICE_RANGE, SPA_FORMAT_AUDIO_CHANNELS,
    SPA_FORMAT_AUDIO_RATE, SPA_TYPE_CHOICE,
};

use crate::audio::device::{
    add_audio_device, audio_device_disconnected, default_audio_device_changed,
    find_physical_audio_device_by_handle, get_audio_thread_name, playback_audio_thread_iterate,
    recording_audio_thread_iterate, updated_audio_device_format, with_device_state, AudioBootStrap,
    AudioDriverImpl, DeviceBackend, DriverFlags, PhysState, PhysicalDevice,
};
use crate::audio::format::AudioSpec;
use crate::audio::AudioFormat;
use crate::error::{Error, Result};
use crate::loadso::SharedObject;

// This seems to be a sane lower limit as Pipewire
// uses it in several of it's own modules.
const PW_MIN_SAMPLES: i32 = 32; // About 0.67ms at 48kHz
const PW_BASE_CLOCK_RATE: i32 = 48000;

const PW_POD_BUFFER_LENGTH: usize = 1024;
const PW_MAX_IDENTIFIER_LENGTH: usize = 256;

// enum PW_READY_FLAGS
const PW_READY_FLAG_BUFFER_ADDED: i32 = 0x1;
const PW_READY_FLAG_STREAM_READY: i32 = 0x2;
const PW_READY_FLAG_ALL_PREOPEN_BITS: i32 = 0x3;
const PW_READY_FLAG_OPEN_COMPLETE: i32 = 0x4;
const PW_READY_FLAG_ALL_BITS: i32 = 0x7;

// The parts of <pipewire/pipewire.h> and the SPA headers SDL uses.

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
    PwMainLoop,
    PwLoop,
    PwThreadLoop,
    PwContext,
    PwCore,
    PwRegistry,
    PwProxy,
    PwStream,
    PwProperties,
    SpaMeta
);

const PW_ID_CORE: u32 = 0;
const PW_ID_ANY: u32 = 0xffff_ffff;
const PW_VERSION_REGISTRY: u32 = 3;
const PW_VERSION_CORE_EVENTS: u32 = 1;
const PW_VERSION_REGISTRY_EVENTS: u32 = 0;
const PW_VERSION_NODE_EVENTS: u32 = 0;
const PW_VERSION_METADATA_EVENTS: u32 = 0;
const PW_VERSION_CLIENT_EVENTS: u32 = 0;
const PW_VERSION_STREAM_EVENTS: u32 = 2;

const PW_TYPE_INTERFACE_NODE: &str = "PipeWire:Interface:Node";
const PW_TYPE_INTERFACE_METADATA: &str = "PipeWire:Interface:Metadata";
const PW_TYPE_INTERFACE_CLIENT: &str = "PipeWire:Interface:Client";

const PW_KEY_CONFIG_NAME: &CStr = c"config.name";
const PW_KEY_APP_NAME: &CStr = c"application.name";
const PW_KEY_APP_ID: &CStr = c"application.id";
const PW_KEY_APP_ICON_NAME: &CStr = c"application.icon-name";
const PW_KEY_NODE_NAME: &CStr = c"node.name";
const PW_KEY_NODE_DESCRIPTION: &CStr = c"node.description";
const PW_KEY_NODE_LATENCY: &CStr = c"node.latency";
const PW_KEY_NODE_RATE: &CStr = c"node.rate";
const PW_KEY_NODE_ALWAYS_PROCESS: &CStr = c"node.always-process";
const PW_KEY_MEDIA_TYPE: &CStr = c"media.type";
const PW_KEY_MEDIA_CATEGORY: &CStr = c"media.category";
const PW_KEY_MEDIA_ROLE: &CStr = c"media.role";
const PW_KEY_MEDIA_CLASS: &str = "media.class";
const PW_KEY_MEDIA_NAME: &CStr = c"media.name";
const PW_KEY_AUDIO_CHANNELS: &str = "audio.channels";
const PW_KEY_TARGET_OBJECT: &CStr = c"target.object";

// enum pw_stream_state
const PW_STREAM_STATE_ERROR: c_int = -1;
const PW_STREAM_STATE_STREAMING: c_int = 3;

// enum pw_direction (spa_direction)
const PW_DIRECTION_INPUT: c_int = 0;
const PW_DIRECTION_OUTPUT: c_int = 1;

// enum pw_stream_flags
const PW_STREAM_FLAG_AUTOCONNECT: c_int = 1 << 0;
const PW_STREAM_FLAG_MAP_BUFFERS: c_int = 1 << 2;

const SPA_PARAM_ENUM_FORMAT: u32 = 3;
const SPA_PARAM_FORMAT: u32 = 4;

// enum spa_audio_format
const SPA_AUDIO_FORMAT_S8: u32 = 0x101;
const SPA_AUDIO_FORMAT_U8: u32 = 0x102;
const SPA_AUDIO_FORMAT_S16_LE: u32 = 0x103;
const SPA_AUDIO_FORMAT_S16_BE: u32 = 0x104;
const SPA_AUDIO_FORMAT_S32_LE: u32 = 0x10b;
const SPA_AUDIO_FORMAT_S32_BE: u32 = 0x10c;
const SPA_AUDIO_FORMAT_F32_LE: u32 = 0x11b;
const SPA_AUDIO_FORMAT_F32_BE: u32 = 0x11c;

// enum spa_audio_channel
const SPA_AUDIO_CHANNEL_MONO: u32 = 2;
const SPA_AUDIO_CHANNEL_FL: u32 = 3;
const SPA_AUDIO_CHANNEL_FR: u32 = 4;
const SPA_AUDIO_CHANNEL_FC: u32 = 5;
const SPA_AUDIO_CHANNEL_LFE: u32 = 6;
const SPA_AUDIO_CHANNEL_SL: u32 = 7;
const SPA_AUDIO_CHANNEL_SR: u32 = 8;
const SPA_AUDIO_CHANNEL_RC: u32 = 11;
const SPA_AUDIO_CHANNEL_RL: u32 = 12;
const SPA_AUDIO_CHANNEL_RR: u32 = 13;

/// `struct spa_list`.
#[repr(C)]
struct SpaList {
    next: *mut SpaList,
    prev: *mut SpaList,
}

/// `struct spa_callbacks`.
#[repr(C)]
struct SpaCallbacks {
    funcs: *const c_void,
    data: *mut c_void,
}

/// `struct spa_interface`: what every proxy (core, registry, node) starts with.
#[repr(C)]
struct SpaInterface {
    type_: *const c_char,
    version: u32,
    cb: SpaCallbacks,
}

/// `struct spa_hook`. libpipewire links it into its lists, so it lives in
/// an `UnsafeCell` at a stable address.
#[repr(C)]
struct SpaHook {
    link: SpaList,
    cb: SpaCallbacks,
    removed: Option<unsafe extern "C" fn(*mut SpaHook)>,
    priv_: *mut c_void,
}

impl SpaHook {
    /// `spa_zero(hook)`.
    const fn zeroed() -> UnsafeCell<SpaHook> {
        UnsafeCell::new(SpaHook {
            link: SpaList {
                next: ptr::null_mut(),
                prev: ptr::null_mut(),
            },
            cb: SpaCallbacks {
                funcs: ptr::null(),
                data: ptr::null_mut(),
            },
            removed: None,
            priv_: ptr::null_mut(),
        })
    }
}

/// Translation of `spa_hook_remove()`.
///
/// # Safety
///
/// `hook` must be a zeroed hook or one libpipewire linked into a list that
/// is still alive.
unsafe fn spa_hook_remove(hook: *mut SpaHook) {
    // SAFETY: the hook is valid (the caller's contract); a linked hook's
    // neighbours are valid list nodes (spa_list_is_initialized/spa_list_remove).
    unsafe {
        if !(*hook).link.prev.is_null() {
            let elem = &mut (*hook).link;
            (*elem.prev).next = elem.next;
            (*elem.next).prev = elem.prev;
        }
        if let Some(removed) = (*hook).removed {
            removed(hook);
        }
    }
}

/// `struct pw_core_info`.
#[repr(C)]
struct PwCoreInfo {
    id: u32,
    cookie: u32,
    user_name: *const c_char,
    host_name: *const c_char,
    version: *const c_char,
    name: *const c_char,
    change_mask: u64,
    props: *mut SpaDict,
}

/// `struct spa_param_info`.
#[repr(C)]
struct SpaParamInfo {
    id: u32,
    flags: u32,
    user: u32,
    seq: i32,
    padding: [u32; 4],
}

/// `struct pw_node_info`.
#[repr(C)]
struct PwNodeInfo {
    id: u32,
    max_input_ports: u32,
    max_output_ports: u32,
    change_mask: u64,
    n_input_ports: u32,
    n_output_ports: u32,
    state: c_int,
    error: *const c_char,
    props: *mut SpaDict,
    params: *mut SpaParamInfo,
    n_params: u32,
}

/// `struct pw_client_info`.
#[repr(C)]
struct PwClientInfo {
    id: u32,
    change_mask: u64,
    props: *mut SpaDict,
}

/// `struct spa_chunk`.
#[repr(C)]
struct SpaChunk {
    offset: u32,
    size: u32,
    stride: i32,
    flags: i32,
}

/// `struct spa_data`.
#[repr(C)]
struct SpaData {
    type_: u32,
    flags: u32,
    fd: i64,
    mapoffset: u32,
    maxsize: u32,
    data: *mut c_void,
    chunk: *mut SpaChunk,
}

/// `struct spa_buffer`.
#[repr(C)]
struct SpaBuffer {
    n_metas: u32,
    n_datas: u32,
    metas: *mut SpaMeta,
    datas: *mut SpaData,
}

/// `struct pw_buffer`.
#[repr(C)]
struct PwBuffer {
    buffer: *mut SpaBuffer,
    user_data: *mut c_void,
    size: u64,
    requested: u64,
    time: u64,
}

type Unused = Option<unsafe extern "C" fn()>;

/// `struct pw_core_events`.
#[repr(C)]
struct PwCoreEvents {
    version: u32,
    info: Option<unsafe extern "C" fn(*mut c_void, *const PwCoreInfo)>,
    done: Option<unsafe extern "C" fn(*mut c_void, u32, c_int)>,
    ping: Unused,
    error: Unused,
    remove_id: Unused,
    bound_id: Unused,
    add_mem: Unused,
    remove_mem: Unused,
    bound_props: Unused,
}

/// `struct pw_core_methods`.
#[repr(C)]
struct PwCoreMethods {
    version: u32,
    add_listener: Option<
        unsafe extern "C" fn(*mut c_void, *mut SpaHook, *const PwCoreEvents, *mut c_void) -> c_int,
    >,
    hello: Unused,
    sync: Option<unsafe extern "C" fn(*mut c_void, u32, c_int) -> c_int>,
    pong: Unused,
    error: Unused,
    get_registry: Option<unsafe extern "C" fn(*mut c_void, u32, usize) -> *mut PwRegistry>,
    create_object: Unused,
    destroy: Unused,
}

/// `struct pw_registry_events`.
#[repr(C)]
struct PwRegistryEvents {
    version: u32,
    global: Option<unsafe extern "C" fn(*mut c_void, u32, u32, *const c_char, u32, *const SpaDict)>,
    global_remove: Option<unsafe extern "C" fn(*mut c_void, u32)>,
}

/// `struct pw_registry_methods`.
#[repr(C)]
struct PwRegistryMethods {
    version: u32,
    add_listener: Option<
        unsafe extern "C" fn(
            *mut c_void,
            *mut SpaHook,
            *const PwRegistryEvents,
            *mut c_void,
        ) -> c_int,
    >,
    bind: Option<unsafe extern "C" fn(*mut c_void, u32, *const c_char, u32, usize) -> *mut c_void>,
    destroy: Unused,
}

/// `struct pw_node_events`.
#[repr(C)]
struct PwNodeEvents {
    version: u32,
    info: Option<unsafe extern "C" fn(*mut c_void, *const PwNodeInfo)>,
    param: Option<unsafe extern "C" fn(*mut c_void, c_int, u32, u32, u32, *const u8)>,
}

/// `struct pw_node_methods`.
#[repr(C)]
struct PwNodeMethods {
    version: u32,
    add_listener: Unused,
    subscribe_params: Unused,
    enum_params:
        Option<unsafe extern "C" fn(*mut c_void, c_int, u32, u32, u32, *const u8) -> c_int>,
    set_param: Unused,
    send_command: Unused,
}

/// `struct pw_metadata_events`.
#[repr(C)]
struct PwMetadataEvents {
    version: u32,
    property: Option<
        unsafe extern "C" fn(
            *mut c_void,
            u32,
            *const c_char,
            *const c_char,
            *const c_char,
        ) -> c_int,
    >,
}

/// `struct pw_client_events`.
#[repr(C)]
struct PwClientEvents {
    version: u32,
    info: Option<unsafe extern "C" fn(*mut c_void, *const PwClientInfo)>,
    permissions: Unused,
}

/// `struct pw_stream_events`.
#[repr(C)]
struct PwStreamEvents {
    version: u32,
    destroy: Unused,
    state_changed: Option<unsafe extern "C" fn(*mut c_void, c_int, c_int, *const c_char)>,
    control_info: Unused,
    io_changed: Unused,
    param_changed: Unused,
    add_buffer: Option<unsafe extern "C" fn(*mut c_void, *mut PwBuffer)>,
    remove_buffer: Unused,
    process: Option<unsafe extern "C" fn(*mut c_void)>,
    drained: Unused,
    command: Unused,
    trigger_done: Unused,
}

/// The methods table and data of a proxy's interface, if it has one
/// (`spa_interface_call_res()`; every call here is version 0).
///
/// # Safety
///
/// `object` must be a live proxy whose interface's methods are `M`.
unsafe fn interface_methods<'a, M>(object: *mut c_void) -> Option<(&'a M, *mut c_void)> {
    // SAFETY: proxies start with their `spa_interface` (the caller's contract).
    let iface = unsafe { &*object.cast::<SpaInterface>() };
    let funcs = iface.cb.funcs.cast::<M>();
    // SAFETY: a non-NULL methods table of type `M`, owned by libpipewire.
    (!funcs.is_null()).then(|| (unsafe { &*funcs }, iface.cb.data))
}

/// Translation of `pw_core_add_listener()`.
///
/// # Safety
///
/// `core` is live; `listener` is zeroed and stays at its address while
/// added; `events` and `data` outlive the listener.
unsafe fn pw_core_add_listener(
    core: *mut PwCore,
    listener: *mut SpaHook,
    events: &'static PwCoreEvents,
    data: *mut c_void,
) -> c_int {
    // SAFETY: the caller's contract.
    match unsafe { interface_methods::<PwCoreMethods>(core.cast()) } {
        // SAFETY: as above.
        Some((m, d)) => m
            .add_listener
            .map_or(-libc::ENOTSUP, |f| unsafe { f(d, listener, events, data) }),
        None => -libc::ENOTSUP,
    }
}

/// Translation of `pw_core_sync()`.
///
/// # Safety
///
/// `core` must be live.
unsafe fn pw_core_sync(core: *mut PwCore, id: u32, seq: c_int) -> c_int {
    // SAFETY: the caller's contract.
    match unsafe { interface_methods::<PwCoreMethods>(core.cast()) } {
        // SAFETY: as above.
        Some((m, d)) => m.sync.map_or(-libc::ENOTSUP, |f| unsafe { f(d, id, seq) }),
        None => -libc::ENOTSUP,
    }
}

/// Translation of `pw_core_get_registry()`.
///
/// # Safety
///
/// `core` must be live.
unsafe fn pw_core_get_registry(
    core: *mut PwCore,
    version: u32,
    user_data_size: usize,
) -> *mut PwRegistry {
    // SAFETY: the caller's contract.
    match unsafe { interface_methods::<PwCoreMethods>(core.cast()) } {
        // SAFETY: as above.
        Some((m, d)) => m.get_registry.map_or(ptr::null_mut(), |f| unsafe {
            f(d, version, user_data_size)
        }),
        None => ptr::null_mut(),
    }
}

/// Translation of `pw_registry_add_listener()`.
///
/// # Safety
///
/// As for [`pw_core_add_listener`].
unsafe fn pw_registry_add_listener(
    registry: *mut PwRegistry,
    listener: *mut SpaHook,
    events: &'static PwRegistryEvents,
    data: *mut c_void,
) -> c_int {
    // SAFETY: the caller's contract.
    match unsafe { interface_methods::<PwRegistryMethods>(registry.cast()) } {
        // SAFETY: as above.
        Some((m, d)) => m
            .add_listener
            .map_or(-libc::ENOTSUP, |f| unsafe { f(d, listener, events, data) }),
        None => -libc::ENOTSUP,
    }
}

/// Translation of `pw_registry_bind()`.
///
/// # Safety
///
/// `registry` must be live; `type_` a C string.
unsafe fn pw_registry_bind(
    registry: *mut PwRegistry,
    id: u32,
    type_: *const c_char,
    version: u32,
    user_data_size: usize,
) -> *mut PwProxy {
    // SAFETY: the caller's contract.
    match unsafe { interface_methods::<PwRegistryMethods>(registry.cast()) } {
        // SAFETY: as above.
        Some((m, d)) => m
            .bind
            .map_or(ptr::null_mut(), |f| unsafe {
                f(d, id, type_, version, user_data_size)
            })
            .cast(),
        None => ptr::null_mut(),
    }
}

/// Translation of `pw_node_enum_params()`.
///
/// # Safety
///
/// `node` must be a live node proxy.
unsafe fn pw_node_enum_params(
    node: *mut PwProxy,
    seq: c_int,
    id: u32,
    start: u32,
    num: u32,
    filter: *const u8,
) -> c_int {
    // SAFETY: the caller's contract.
    match unsafe { interface_methods::<PwNodeMethods>(node.cast()) } {
        // SAFETY: as above.
        Some((m, d)) => m.enum_params.map_or(-libc::ENOTSUP, |f| unsafe {
            f(d, seq, id, start, num, filter)
        }),
        None => -libc::ENOTSUP,
    }
}

macro_rules! pipewire_syms {
    ($($name:ident: fn($($arg:ty),*) $(-> $ret:ty)?;)*) => {
        /// The libpipewire entry points SDL uses (the `PIPEWIRE_pw_*`
        /// function pointers), with the library they came from. (All are
        /// loaded, as upstream does, though not every one is called.)
        #[allow(dead_code)]
        struct PwLib {
            $($name: unsafe extern "C" fn($($arg),*) $(-> $ret)?,)*
            pw_properties_new: unsafe extern "C" fn(*const c_char, ...) -> *mut PwProperties,
            pw_properties_setf: unsafe extern "C" fn(*mut PwProperties, *const c_char, *const c_char, ...) -> c_int,
            _handle: SharedObject,
        }

        impl PwLib {
            /// Translation of `load_pipewire_syms()`.
            fn load_syms(handle: SharedObject) -> Result<PwLib> {
                Ok(PwLib {
                    $(
                        // SAFETY: the declared type is the symbol's C
                        // signature from <pipewire/pipewire.h>, and the
                        // pointer is only called while `_handle` keeps the
                        // library loaded.
                        $name: unsafe { handle.function(stringify!($name))? },
                    )*
                    // SAFETY: as above (variadic).
                    pw_properties_new: unsafe { handle.function("pw_properties_new")? },
                    // SAFETY: as above (variadic).
                    pw_properties_setf: unsafe { handle.function("pw_properties_setf")? },
                    _handle: handle,
                })
            }
        }
    };
}

pipewire_syms! {
    pw_get_library_version: fn() -> *const c_char;
    pw_init: fn(*mut c_int, *mut *mut *mut c_char);
    pw_deinit: fn();
    pw_main_loop_new: fn(*const SpaDict) -> *mut PwMainLoop;
    pw_main_loop_get_loop: fn(*mut PwMainLoop) -> *mut PwLoop;
    pw_main_loop_run: fn(*mut PwMainLoop) -> c_int;
    pw_main_loop_quit: fn(*mut PwMainLoop) -> c_int;
    pw_main_loop_destroy: fn(*mut PwMainLoop);
    pw_thread_loop_new: fn(*const c_char, *const SpaDict) -> *mut PwThreadLoop;
    pw_thread_loop_destroy: fn(*mut PwThreadLoop);
    pw_thread_loop_stop: fn(*mut PwThreadLoop);
    pw_thread_loop_get_loop: fn(*mut PwThreadLoop) -> *mut PwLoop;
    pw_thread_loop_lock: fn(*mut PwThreadLoop);
    pw_thread_loop_unlock: fn(*mut PwThreadLoop);
    pw_thread_loop_signal: fn(*mut PwThreadLoop, bool);
    pw_thread_loop_wait: fn(*mut PwThreadLoop);
    pw_thread_loop_timed_wait: fn(*mut PwThreadLoop, c_int) -> c_int;
    pw_thread_loop_start: fn(*mut PwThreadLoop) -> c_int;
    pw_context_new: fn(*mut PwLoop, *mut PwProperties, usize) -> *mut PwContext;
    pw_context_destroy: fn(*mut PwContext);
    pw_context_connect: fn(*mut PwContext, *mut PwProperties, usize) -> *mut PwCore;
    pw_proxy_add_object_listener: fn(*mut PwProxy, *mut SpaHook, *const c_void, *mut c_void);
    pw_proxy_get_user_data: fn(*mut PwProxy) -> *mut c_void;
    pw_proxy_destroy: fn(*mut PwProxy);
    pw_core_disconnect: fn(*mut PwCore) -> c_int;
    pw_stream_new_simple: fn(*mut PwLoop, *const c_char, *mut PwProperties, *const PwStreamEvents, *mut c_void) -> *mut PwStream;
    pw_stream_destroy: fn(*mut PwStream);
    pw_stream_connect: fn(*mut PwStream, c_int, u32, c_int, *mut *const u8, u32) -> c_int;
    pw_stream_get_state: fn(*mut PwStream, *mut *const c_char) -> c_int;
    pw_stream_dequeue_buffer: fn(*mut PwStream) -> *mut PwBuffer;
    pw_stream_queue_buffer: fn(*mut PwStream, *mut PwBuffer) -> c_int;
    pw_properties_set: fn(*mut PwProperties, *const c_char, *const c_char) -> c_int;
    pw_stream_update_properties: fn(*mut PwStream, *const SpaDict) -> c_int;
}

/// `SDL_AUDIO_DRIVER_PIPEWIRE_DYNAMIC`.
const PIPEWIRE_LIBRARY: &str = "libpipewire-0.3.so.0";

/// Translation of `init_pipewire_library()` (with `load_pipewire_library()`).
fn init_pipewire_library() -> Option<Arc<PwLib>> {
    let handle = SharedObject::load(PIPEWIRE_LIBRARY).ok()?;
    let lib = PwLib::load_syms(handle).ok()?;
    // SAFETY: pw_init() accepts NULL arguments.
    unsafe { (lib.pw_init)(ptr::null_mut(), ptr::null_mut()) };
    Some(Arc::new(lib))
}

/// Translation of `deinit_pipewire_library()` (the library is unloaded with
/// its last reference).
fn deinit_pipewire_library(lib: &PwLib) {
    // SAFETY: balances the pw_init() of init_pipewire_library().
    unsafe { (lib.pw_deinit)() };
}

/// `errno`, for the error messages.
fn errno() -> i32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

/// A C string as `&str` (empty for NULL or invalid UTF-8).
///
/// # Safety
///
/// `s` must be NULL or a C string that outlives `'a`.
unsafe fn c_str<'a>(s: *const c_char) -> Option<&'a str> {
    if s.is_null() {
        None
    } else {
        // SAFETY: the caller's contract.
        Some(unsafe { CStr::from_ptr(s) }.to_str().unwrap_or(""))
    }
}

/// A generic Pipewire node object used for enumeration. Translation of
/// `struct node_object`; it's boxed (its hooks must not move) and handed
/// to the callbacks as a raw pointer.
struct NodeObject {
    id: u32,
    seq: c_int,
    persist: bool,

    /// The I/O node being filled in, for interface nodes. (Upstream's
    /// `void *userdata`, freed with the node unless it was moved out.)
    userdata: Option<Box<IoNode>>,

    proxy: *mut PwProxy,
    node_listener: UnsafeCell<SpaHook>,
    core_listener: UnsafeCell<SpaHook>,

    hotplug: *const Hotplug,
}

/// A sink/source node used for stream I/O. Translation of `struct io_node`.
#[derive(Clone, Debug)]
struct IoNode {
    id: u32,
    recording: bool,
    spec: AudioSpec,

    /// Friendly name
    name: String,
    /// OS identifier (i.e. ALSA endpoint)
    path: String,
}

/// The state upstream keeps in file-level statics next to the hotplug
/// loop, behind the loop's lock there (and this mutex here).
#[derive(Default)]
struct HotplugState {
    /// `hotplug_pending_list`
    pending_list: Vec<*mut NodeObject>,
    /// `hotplug_io_list`: the active node list
    io_list: Vec<IoNode>,
    hotplug_init_seq_val: c_int,
    hotplug_init_complete: bool,
    hotplug_events_enabled: bool,

    pipewire_have_session_services: bool,
    pipewire_have_audio_service: bool,
    pipewire_version_major: i32,
    pipewire_version_minor: i32,
    pipewire_version_patch: i32,
    pipewire_default_sink_id: Option<String>,
    pipewire_default_source_id: Option<String>,
}

/// The global hotplug thread and associated objects (`hotplug_loop`,
/// `hotplug_core`, ...).
struct Hotplug {
    lib: Arc<PwLib>,
    hotplug_loop: *mut PwThreadLoop,
    hotplug_core: *mut PwCore,
    hotplug_context: *mut PwContext,
    hotplug_registry: *mut PwRegistry,
    hotplug_registry_listener: UnsafeCell<SpaHook>,
    hotplug_core_listener: UnsafeCell<SpaHook>,
    state: Mutex<HotplugState>,
}

// SAFETY: the PipeWire objects are used from the hotplug loop's thread and,
// under the loop's lock, from other threads, as upstream does; the hooks
// are only touched by libpipewire (and spa_hook_remove()) under that lock;
// the Rust state is behind a mutex.
unsafe impl Send for Hotplug {}
// SAFETY: as above.
unsafe impl Sync for Hotplug {}

impl Hotplug {
    fn state(&self) -> MutexGuard<'_, HotplugState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn lock(&self) {
        // SAFETY: the loop is alive until `hotplug_loop_destroy`.
        unsafe { (self.lib.pw_thread_loop_lock)(self.hotplug_loop) }
    }

    fn unlock(&self) {
        // SAFETY: as above; we hold the lock.
        unsafe { (self.lib.pw_thread_loop_unlock)(self.hotplug_loop) }
    }

    fn wait(&self) {
        // SAFETY: as above; we hold the lock.
        unsafe { (self.lib.pw_thread_loop_wait)(self.hotplug_loop) }
    }

    fn userdata(&self) -> *mut c_void {
        ptr::from_ref(self).cast_mut().cast()
    }

    /// Translation of `pipewire_core_version_at_least()`.
    fn pipewire_core_version_at_least(&self, major: i32, minor: i32, patch: i32) -> bool {
        let s = self.state();
        (s.pipewire_version_major >= major)
            && (s.pipewire_version_major > major || s.pipewire_version_minor >= minor)
            && (s.pipewire_version_major > major
                || s.pipewire_version_minor > minor
                || s.pipewire_version_patch >= patch)
    }

    /// Translation of `io_list_check_add()`: false (and the node back) if
    /// it's already in the list.
    fn io_list_check_add(&self, node: Box<IoNode>) -> std::result::Result<(), Box<IoNode>> {
        let events_enabled = {
            let mut s = self.state();
            // See if the node is already in the list
            if s.io_list.iter().any(|n| n.id == node.id) {
                return Err(node);
            }

            // Add to the list if the node doesn't already exist
            s.io_list.push((*node).clone());
            s.hotplug_events_enabled
        };

        if events_enabled {
            let _ = add_audio_device(
                node.recording,
                &node.name,
                None,
                Some(&node.spec),
                node.id as usize,
            );
        }

        Ok(())
    }

    /// Translation of `io_list_remove()`.
    fn io_list_remove(&self, id: u32) {
        // Find and remove the node from the list
        let events_enabled = {
            let mut s = self.state();
            let Some(i) = s.io_list.iter().position(|n| n.id == id) else {
                return;
            };
            s.io_list.remove(i);
            s.hotplug_events_enabled
        };

        if events_enabled {
            if let Some(device) = find_physical_audio_device_by_handle(id as usize) {
                audio_device_disconnected(&device);
            }
        }
    }

    /// Translation of `io_list_clear()`.
    fn io_list_clear(&self) {
        self.state().io_list.clear();
    }

    /// Translation of `io_list_get_by_id()`.
    fn io_list_get_by_id(&self, id: u32) -> Option<IoNode> {
        self.state().io_list.iter().find(|n| n.id == id).cloned()
    }

    /// Translation of `pending_list_add()`.
    fn pending_list_add(&self, node: *mut NodeObject) {
        crate::sdl_assert!(!node.is_null());
        self.state().pending_list.push(node);
    }

    /// Translation of `pending_list_remove()`.
    fn pending_list_remove(&self, id: u32) {
        let nodes: Vec<*mut NodeObject> = self
            .state()
            .pending_list
            .iter()
            .copied()
            // SAFETY: pending nodes are alive until node_object_destroy().
            .filter(|&n| unsafe { (*n).id } == id)
            .collect();
        for node in nodes {
            // SAFETY: a live pending node, destroyed once.
            unsafe { self.node_object_destroy(node) };
        }
    }

    /// Translation of `pending_list_clear()`.
    fn pending_list_clear(&self) {
        let nodes: Vec<*mut NodeObject> = self.state().pending_list.clone();
        for node in nodes {
            // SAFETY: a live pending node, destroyed once.
            unsafe { self.node_object_destroy(node) };
        }
    }

    /// Translation of `node_object_destroy()`.
    ///
    /// # Safety
    ///
    /// `node` must be a live node from [`Hotplug::node_object_new`]; it's
    /// freed here.
    unsafe fn node_object_destroy(&self, node: *mut NodeObject) {
        crate::sdl_assert!(!node.is_null());

        self.state().pending_list.retain(|&n| n != node);
        // SAFETY: the node is alive (the caller's contract); its hooks were
        // added (or are zeroed).
        unsafe {
            spa_hook_remove((*node).node_listener.get());
            spa_hook_remove((*node).core_listener.get());
            (*node).userdata = None; // SDL_free(node->userdata)
            (self.lib.pw_proxy_destroy)((*node).proxy);
            drop(Box::from_raw(node));
        }
    }

    /// Translation of `node_object_new()`.
    fn node_object_new(
        &self,
        id: u32,
        type_: *const c_char,
        version: u32,
        funcs: *const c_void,
        core_events: &'static PwCoreEvents,
    ) -> Option<*mut NodeObject> {
        // Create the proxy object
        // (the node lives in its own Box rather than the proxy's user data)
        // SAFETY: the registry is live; `type_` is libpipewire's C string.
        let proxy = unsafe { pw_registry_bind(self.hotplug_registry, id, type_, version, 0) };
        if proxy.is_null() {
            // SDL_SetError("Pipewire: Failed to create proxy object (%i)", errno)
            let _ = errno();
            return None;
        }

        let node = Box::into_raw(Box::new(NodeObject {
            id,
            seq: 0,
            persist: false,
            userdata: None,
            proxy,
            node_listener: SpaHook::zeroed(),
            core_listener: SpaHook::zeroed(),
            hotplug: ptr::from_ref(self),
        }));

        // Add the callbacks
        // SAFETY: the core and proxy are live; the hooks are zeroed and
        // boxed; the events are static; the node outlives its hooks
        // (node_object_destroy() removes them first).
        unsafe {
            pw_core_add_listener(
                self.hotplug_core,
                (*node).core_listener.get(),
                core_events,
                node.cast(),
            );
            (self.lib.pw_proxy_add_object_listener)(
                proxy,
                (*node).node_listener.get(),
                funcs,
                node.cast(),
            );
        }

        // Add the node to the active list
        self.pending_list_add(node);

        Some(node)
    }

    /// Translation of `hotplug_core_sync()`.
    ///
    /// # Safety
    ///
    /// `node` must be NULL or a live node.
    unsafe fn hotplug_core_sync(&self, node: *mut NodeObject) {
        /*
         * Node sync events *must* come before the hotplug init sync events or the initial
         * I/O list will be incomplete when the main hotplug sync point is hit.
         */
        if !node.is_null() {
            // SAFETY: a live node and core.
            unsafe { (*node).seq = pw_core_sync(self.hotplug_core, PW_ID_CORE, (*node).seq) };
        }

        let (init_complete, seq) = {
            let s = self.state();
            (s.hotplug_init_complete, s.hotplug_init_seq_val)
        };
        if !init_complete {
            // SAFETY: the core is live.
            let seq = unsafe { pw_core_sync(self.hotplug_core, PW_ID_CORE, seq) };
            self.state().hotplug_init_seq_val = seq;
        }
    }

    /// Translation of `change_default_device()`.
    fn change_default_device(&self, path: Option<&str>) {
        let id = {
            let s = self.state();
            if !s.hotplug_events_enabled {
                return;
            }
            // FIXME (upstream): `path` can be NULL here (a value that isn't a
            // JSON object) and is passed to SDL_strcmp(); nothing matches it here.
            let Some(path) = path else { return };
            s.io_list.iter().find(|n| n.path == path).map(|n| n.id)
        };
        if let Some(id) = id {
            if let Some(device) = find_physical_audio_device_by_handle(id as usize) {
                // FIXME (upstream): this runs on the hotplug loop thread with
                // the loop locked and takes device locks, while OpenDevice holds
                // a device lock and waits for the hotplug loop lock.
                default_audio_device_changed(&device);
            }
            // found it, we're done.
        }
    }
}

// Core sync points

/// Translation of `core_events_hotplug_init_callback()`; the userdata is the `Hotplug`.
unsafe extern "C" fn core_events_hotplug_init_callback(object: *mut c_void, id: u32, seq: c_int) {
    // SAFETY: the userdata is the live `Hotplug` (see hotplug_loop_init()).
    let hotplug = unsafe { &*object.cast::<Hotplug>() };
    if id == PW_ID_CORE && seq == hotplug.state().hotplug_init_seq_val {
        // This core listener is no longer needed.
        // SAFETY: the hook was added in hotplug_loop_init(); we're on the loop thread.
        unsafe { spa_hook_remove(hotplug.hotplug_core_listener.get()) };

        // Signal that the initial I/O list is populated
        hotplug.state().hotplug_init_complete = true;
        // SAFETY: the loop is alive.
        unsafe { (hotplug.lib.pw_thread_loop_signal)(hotplug.hotplug_loop, false) };
    }
}

/// Parse `"%d.%d.%d"` like `SDL_sscanf()`, returning how many matched.
fn sscanf_version(s: &str, out: &mut [i32; 3]) -> usize {
    let mut rest = s;
    for (i, slot) in out.iter_mut().enumerate() {
        if i > 0 {
            let Some(r) = rest.strip_prefix('.') else {
                return i;
            };
            rest = r;
        }
        let t = rest.trim_start();
        let digits_start = usize::from(t.starts_with(['-', '+']));
        let digits = t[digits_start..]
            .bytes()
            .take_while(u8::is_ascii_digit)
            .count();
        if digits == 0 {
            return i;
        }
        let (num, r) = t.split_at(digits_start + digits);
        *slot = crate::stdlib::atoi(num);
        rest = r;
    }
    3
}

/// Translation of `core_events_hotplug_info_callback()`.
unsafe extern "C" fn core_events_hotplug_info_callback(data: *mut c_void, info: *const PwCoreInfo) {
    // SAFETY: the userdata is the live `Hotplug`; `info` is valid for this call.
    let (hotplug, version) = unsafe { (&*data.cast::<Hotplug>(), c_str((*info).version)) };
    let mut v = [0i32; 3];
    let n = sscanf_version(version.unwrap_or(""), &mut v);
    let mut s = hotplug.state();
    s.pipewire_version_major = v[0];
    s.pipewire_version_minor = v[1];
    s.pipewire_version_patch = v[2];
    if n < 3 {
        s.pipewire_version_major = 0;
        s.pipewire_version_minor = 0;
        s.pipewire_version_patch = 0;
    }
}

/// Translation of `core_events_interface_callback()`; the userdata is the node.
unsafe extern "C" fn core_events_interface_callback(object: *mut c_void, id: u32, seq: c_int) {
    let node = object.cast::<NodeObject>();
    // SAFETY: the userdata is a live node (its hooks are removed before it's freed).
    let (hotplug, node_seq) = unsafe { (&*(*node).hotplug, (*node).seq) };

    if id == PW_ID_CORE && seq == node_seq {
        /*
         * Move the I/O node to the connected list.
         * On success, the list owns the I/O node object.
         */
        // SAFETY: as above.
        if let Some(io) = unsafe { (*node).userdata.take() } {
            if let Err(io) = hotplug.io_list_check_add(io) {
                // SAFETY: as above.
                unsafe { (*node).userdata = Some(io) };
            }
        }

        // SAFETY: as above; the node isn't used after this.
        unsafe { hotplug.node_object_destroy(node) };
    }
}

/// Translation of `core_events_generic_callback()`; the userdata is the node.
unsafe extern "C" fn core_events_generic_callback(object: *mut c_void, id: u32, seq: c_int) {
    let node = object.cast::<NodeObject>();
    // SAFETY: the userdata is a live node.
    let (hotplug, node_seq, persist) = unsafe { (&*(*node).hotplug, (*node).seq, (*node).persist) };

    if id == PW_ID_CORE && seq == node_seq && !persist {
        // SAFETY: as above; the node isn't used after this.
        unsafe { hotplug.node_object_destroy(node) };
    }
}

static HOTPLUG_INIT_CORE_EVENTS: PwCoreEvents = PwCoreEvents {
    version: PW_VERSION_CORE_EVENTS,
    info: Some(core_events_hotplug_info_callback),
    done: Some(core_events_hotplug_init_callback),
    ping: None,
    error: None,
    remove_id: None,
    bound_id: None,
    add_mem: None,
    remove_mem: None,
    bound_props: None,
};
static INTERFACE_CORE_EVENTS: PwCoreEvents = PwCoreEvents {
    version: PW_VERSION_CORE_EVENTS,
    info: None,
    done: Some(core_events_interface_callback),
    ping: None,
    error: None,
    remove_id: None,
    bound_id: None,
    add_mem: None,
    remove_mem: None,
    bound_props: None,
};
static GENERIC_CORE_EVENTS: PwCoreEvents = PwCoreEvents {
    version: PW_VERSION_CORE_EVENTS,
    info: None,
    done: Some(core_events_generic_callback),
    ping: None,
    error: None,
    remove_id: None,
    bound_id: None,
    add_mem: None,
    remove_mem: None,
    bound_props: None,
};

// Helpers for retrieving values from params

/// Translation of `get_range_param()`: (default, min, max).
fn get_range_param(param: Pod<'_>, key: u32) -> Option<(i32, i32, i32)> {
    let prop = spa_pod_find_prop(param, None, key)?;

    if prop.value.type_() == SPA_TYPE_CHOICE {
        let (value, n_values, choice) = spa_pod_get_values(prop.value);

        if n_values == 3 && choice == SPA_CHOICE_RANGE {
            let v = value.body(); // SPA_POD_BODY(value)
            let at = |i: usize| {
                v.get(i * 4..i * 4 + 4)
                    .map(|b| u32::from_ne_bytes([b[0], b[1], b[2], b[3]]) as i32)
            };
            if let (Some(def), Some(min), Some(max)) = (at(0), at(1), at(2)) {
                return Some((def, min, max));
            }
        }
    }

    None
}

/// Translation of `get_int_param()`.
fn get_int_param(param: Pod<'_>, key: u32) -> Option<i32> {
    let prop = spa_pod_find_prop(param, None, key)?;
    spa_pod_get_int(prop.value).ok()
}

/// Translation of `SPAFormatToSDL()`.
fn spa_format_to_sdl(spafmt: u32) -> AudioFormat {
    match spafmt {
        SPA_AUDIO_FORMAT_U8 => AudioFormat::U8,
        SPA_AUDIO_FORMAT_S8 => AudioFormat::S8,
        SPA_AUDIO_FORMAT_S16_LE => AudioFormat::S16LE,
        SPA_AUDIO_FORMAT_S16_BE => AudioFormat::S16BE,
        SPA_AUDIO_FORMAT_S32_LE => AudioFormat::S32LE,
        SPA_AUDIO_FORMAT_S32_BE => AudioFormat::S32BE,
        SPA_AUDIO_FORMAT_F32_LE => AudioFormat::F32LE,
        SPA_AUDIO_FORMAT_F32_BE => AudioFormat::F32BE,
        _ => AudioFormat::UNKNOWN,
    }
}

// Interface node callbacks

/// Translation of `node_event_info()`; the userdata is the node.
unsafe extern "C" fn node_event_info(object: *mut c_void, info: *const PwNodeInfo) {
    let node = object.cast::<NodeObject>();
    if info.is_null() {
        return;
    }
    // SAFETY: a live node; `info` is valid for this call.
    let (hotplug, info) = unsafe { (&*(*node).hotplug, &*info) };

    // SAFETY: the info's props are a valid dictionary (or NULL).
    if let Some(prop_val) = unsafe { spa_dict_lookup(info.props, PW_KEY_AUDIO_CHANNELS) } {
        let channels = crate::stdlib::atoi(&prop_val.to_string_lossy()) as u8;
        // SAFETY: a live node; this runs on the loop thread.
        if let Some(io) = unsafe { (*node).userdata.as_mut() } {
            io.spec.channels = channels as i32;
        }
    }

    // Need to parse the parameters to get the sample rate
    for i in 0..info.n_params as usize {
        // SAFETY: `params` holds `n_params` entries; the proxy is a live node.
        unsafe {
            let id = (*info.params.add(i)).id;
            pw_node_enum_params((*node).proxy, 0, id, 0, 0, ptr::null());
        }
    }

    // SAFETY: a live node.
    unsafe { hotplug.hotplug_core_sync(node) };
}

/// Translation of `node_event_param()`; the userdata is the node.
unsafe extern "C" fn node_event_param(
    object: *mut c_void,
    _seq: c_int,
    id: u32,
    _index: u32,
    _next: u32,
    param: *const u8,
) {
    let node = object.cast::<NodeObject>();
    // SAFETY: libpipewire passes a valid pod (or NULL).
    let Some(param) = (unsafe { Pod::from_ptr(param) }) else {
        return;
    };
    // SAFETY: a live node; this runs on the loop thread.
    let Some(io) = (unsafe { (*node).userdata.as_mut() }) else {
        return;
    };

    // FIXME (upstream): the node's format is preset to S16 when it's
    // created, so this never runs; and spa_format_audio_raw_parse()
    // returns the number of fields it read, so `== 0` only holds when
    // it read none.
    if id == SPA_PARAM_FORMAT && io.spec.format == AudioFormat::UNKNOWN {
        let mut info = SpaAudioInfoRaw::default();
        if spa_format_audio_raw_parse(param, &mut info) == 0 {
            //SDL_Log("Sink Format: %d, Rate: %d Hz, Channels: %d", info.format, info.rate, info.channels);
            io.spec.format = spa_format_to_sdl(info.format);
        }
    }

    // Get the default frequency
    if io.spec.freq == 0 {
        if let Some((def, _, _)) = get_range_param(param, SPA_FORMAT_AUDIO_RATE) {
            io.spec.freq = def;
        }
    }

    /*
     * The channel count should have come from the node properties,
     * but it is stored here as well. If one failed, try the other.
     */
    if io.spec.channels == 0 {
        if let Some(channels) = get_int_param(param, SPA_FORMAT_AUDIO_CHANNELS) {
            io.spec.channels = channels as u8 as i32;
        }
    }
}

static INTERFACE_NODE_EVENTS: PwNodeEvents = PwNodeEvents {
    version: PW_VERSION_NODE_EVENTS,
    info: Some(node_event_info),
    param: Some(node_event_param),
};

/// Translation of `get_name_from_json()`.
fn get_name_from_json(json: &[u8]) -> Option<String> {
    let mut parser0 = SpaJson::new(json);
    // Not actually JSON
    let mut parser1 = parser0.enter_object().ok()?;
    // Not actually a key/value pair
    parser1.get_string(7).ok()?; // "name"
                                 // Somehow had a key with no value?
    let value = parser1.get_string(PW_MAX_IDENTIFIER_LENGTH).ok()?;
    Some(String::from_utf8_lossy(&value).into_owned())
}

// Metadata node callback

/// Translation of `metadata_property()`; the userdata is the node.
unsafe extern "C" fn metadata_property(
    object: *mut c_void,
    subject: u32,
    key: *const c_char,
    _type: *const c_char,
    value: *const c_char,
) -> c_int {
    let node = object.cast::<NodeObject>();
    // SAFETY: a live node; the strings are valid for this call.
    let (hotplug, key, value) = unsafe { (&*(*node).hotplug, c_str(key), c_str(value)) };

    if let (PW_ID_CORE, Some(key), Some(value)) = (subject, key, value) {
        if key == "default.audio.sink" {
            let id = get_name_from_json(value.as_bytes());
            hotplug.state().pipewire_default_sink_id = id.clone();
            // SAFETY: a live node, on the loop thread.
            unsafe { (*node).persist = true };
            hotplug.change_default_device(id.as_deref());
        } else if key == "default.audio.source" {
            let id = get_name_from_json(value.as_bytes());
            hotplug.state().pipewire_default_source_id = id.clone();
            // SAFETY: as above.
            unsafe { (*node).persist = true };
            hotplug.change_default_device(id.as_deref());
        }
    }

    0
}

static METADATA_NODE_EVENTS: PwMetadataEvents = PwMetadataEvents {
    version: PW_VERSION_METADATA_EVENTS,
    property: Some(metadata_property),
};

/// Translation of `client_info()`: client info node callback; the userdata is the node.
unsafe extern "C" fn client_info(data: *mut c_void, info: *const PwClientInfo) {
    let node = data.cast::<NodeObject>();
    // SAFETY: a live node; `info` is valid for this call.
    let hotplug = unsafe { &*(*node).hotplug };
    // If WirePlumber lists the session services, check to see if audio is enabled.
    // SAFETY: the info's props are a valid dictionary (or NULL).
    let Some(services) = (unsafe { spa_dict_lookup((*info).props, "session.services") }) else {
        return;
    };
    hotplug.state().pipewire_have_session_services = true;

    // Services are in a JSON array.
    let mut iter0 = SpaJson::new(services.to_bytes());
    if let Ok(mut iter1) = iter0.enter_array() {
        while let Ok(element) = iter1.get_string(PW_MAX_IDENTIFIER_LENGTH) {
            if element == b"audio" {
                hotplug.state().pipewire_have_audio_service = true;
                break;
            }
        }
    }
}

static CLIENT_NODE_EVENTS: PwClientEvents = PwClientEvents {
    version: PW_VERSION_CLIENT_EVENTS,
    info: Some(client_info),
    permissions: None,
};

// Global registry callbacks

/// Translation of `registry_event_global_callback()`; the userdata is the `Hotplug`.
unsafe extern "C" fn registry_event_global_callback(
    object: *mut c_void,
    id: u32,
    _permissions: u32,
    type_: *const c_char,
    version: u32,
    props: *const SpaDict,
) {
    // SAFETY: the userdata is the live `Hotplug`; `type_` is a C string.
    let (hotplug, type_str) = unsafe { (&*object.cast::<Hotplug>(), c_str(type_).unwrap_or("")) };

    // We're only interested in interface and metadata nodes.
    if type_str == PW_TYPE_INTERFACE_NODE {
        // SAFETY: `props` is a valid dictionary (or NULL) for this call.
        let Some(media_class) = (unsafe { spa_dict_lookup(props, PW_KEY_MEDIA_CLASS) }) else {
            return;
        };
        let media_class = media_class.to_string_lossy();

        // Just want sink and source
        let recording = if media_class.eq_ignore_ascii_case("Audio/Sink") {
            false
        } else if media_class.eq_ignore_ascii_case("Audio/Source")
            || media_class.eq_ignore_ascii_case("Audio/Source/Virtual")
        {
            true
        } else {
            return;
        };

        // SAFETY: as above.
        let node_desc = unsafe { spa_dict_lookup(props, "node.description") };
        // SAFETY: as above.
        let node_path = unsafe { spa_dict_lookup(props, "node.name") };

        if let (Some(node_desc), Some(node_path)) = (node_desc, node_path) {
            let Some(node) = hotplug.node_object_new(
                id,
                type_,
                version,
                ptr::from_ref(&INTERFACE_NODE_EVENTS).cast(),
                &INTERFACE_CORE_EVENTS,
            ) else {
                // SDL_SetError("Pipewire: Failed to allocate interface node")
                return;
            };

            // Allocate and initialize the I/O node information struct
            let mut io = Box::new(IoNode {
                // Begin setting the node properties
                id,
                recording,
                spec: AudioSpec::new(AudioFormat::UNKNOWN, 0, 0),
                name: node_desc.to_string_lossy().into_owned(),
                path: node_path.to_string_lossy().into_owned(),
            });
            if io.spec.format == AudioFormat::UNKNOWN {
                io.spec.format = AudioFormat::S16; // we'll go conservative here if for some reason the format isn't known.
            }
            // SAFETY: the node was just created and is alive.
            unsafe { (*node).userdata = Some(io) };

            // Update sync points
            // SAFETY: as above.
            unsafe { hotplug.hotplug_core_sync(node) };
        }
    } else if type_str == PW_TYPE_INTERFACE_METADATA {
        let Some(node) = hotplug.node_object_new(
            id,
            type_,
            version,
            ptr::from_ref(&METADATA_NODE_EVENTS).cast(),
            &GENERIC_CORE_EVENTS,
        ) else {
            // SDL_SetError("Pipewire: Failed to allocate metadata node")
            return;
        };

        // Update sync points
        // SAFETY: the node was just created and is alive.
        unsafe { hotplug.hotplug_core_sync(node) };
    } else if type_str == PW_TYPE_INTERFACE_CLIENT {
        let Some(node) = hotplug.node_object_new(
            id,
            type_,
            version,
            ptr::from_ref(&CLIENT_NODE_EVENTS).cast(),
            &GENERIC_CORE_EVENTS,
        ) else {
            // SDL_SetError("Pipewire: Failed to allocate client info node")
            return;
        };

        // Update sync points
        // SAFETY: the node was just created and is alive.
        unsafe { hotplug.hotplug_core_sync(node) };
    }
}

/// Translation of `registry_event_remove_callback()`; the userdata is the `Hotplug`.
unsafe extern "C" fn registry_event_remove_callback(object: *mut c_void, id: u32) {
    // SAFETY: the userdata is the live `Hotplug`.
    let hotplug = unsafe { &*object.cast::<Hotplug>() };
    hotplug.io_list_remove(id);
    hotplug.pending_list_remove(id);
}

static REGISTRY_EVENTS: PwRegistryEvents = PwRegistryEvents {
    version: PW_VERSION_REGISTRY_EVENTS,
    global: Some(registry_event_global_callback),
    global_remove: Some(registry_event_remove_callback),
};

// The hotplug thread

/// Translation of `hotplug_loop_init()`. On failure, what was set up is
/// torn down again (upstream's caller does that with PIPEWIRE_Deinitialize()).
fn hotplug_loop_init(lib: &Arc<PwLib>) -> Result<Box<Hotplug>> {
    let mut hotplug = Box::new(Hotplug {
        lib: lib.clone(),
        hotplug_loop: ptr::null_mut(),
        hotplug_core: ptr::null_mut(),
        hotplug_context: ptr::null_mut(),
        hotplug_registry: ptr::null_mut(),
        hotplug_registry_listener: SpaHook::zeroed(),
        hotplug_core_listener: SpaHook::zeroed(),
        state: Mutex::new(HotplugState::default()),
    });

    let result = (|| {
        // SAFETY: the name is NUL-terminated; NULL props are allowed.
        hotplug.hotplug_loop =
            unsafe { (lib.pw_thread_loop_new)(c"SDLPwAudioPlug".as_ptr(), ptr::null()) };
        if hotplug.hotplug_loop.is_null() {
            return Err(Error::new(format!(
                "Pipewire: Failed to create hotplug detection loop ({})",
                errno()
            )));
        }

        // SAFETY: the loop is live; NULL props are allowed.
        hotplug.hotplug_context = unsafe {
            (lib.pw_context_new)(
                (lib.pw_thread_loop_get_loop)(hotplug.hotplug_loop),
                ptr::null_mut(),
                0,
            )
        };
        if hotplug.hotplug_context.is_null() {
            return Err(Error::new(format!(
                "Pipewire: Failed to create hotplug detection context ({})",
                errno()
            )));
        }

        // SAFETY: the context is live; NULL props are allowed.
        hotplug.hotplug_core =
            unsafe { (lib.pw_context_connect)(hotplug.hotplug_context, ptr::null_mut(), 0) };
        if hotplug.hotplug_core.is_null() {
            return Err(Error::new(format!(
                "Pipewire: Failed to connect hotplug detection context ({})",
                errno()
            )));
        }

        // SAFETY: the core is live.
        hotplug.hotplug_registry =
            unsafe { pw_core_get_registry(hotplug.hotplug_core, PW_VERSION_REGISTRY, 0) };
        if hotplug.hotplug_registry.is_null() {
            return Err(Error::new(format!(
                "Pipewire: Failed to acquire hotplug detection registry ({})",
                errno()
            )));
        }

        let data = hotplug.userdata();
        // SAFETY: the registry and core are live; the hooks are zeroed and
        // boxed with `hotplug`, which outlives them (hotplug_loop_destroy()
        // tears down the objects they're attached to).
        unsafe {
            pw_registry_add_listener(
                hotplug.hotplug_registry,
                hotplug.hotplug_registry_listener.get(),
                &REGISTRY_EVENTS,
                data,
            );

            pw_core_add_listener(
                hotplug.hotplug_core,
                hotplug.hotplug_core_listener.get(),
                &HOTPLUG_INIT_CORE_EVENTS,
                data,
            );

            let seq = pw_core_sync(hotplug.hotplug_core, PW_ID_CORE, 0);
            hotplug.state().hotplug_init_seq_val = seq;
        }

        // SAFETY: the loop is live.
        let res = unsafe { (lib.pw_thread_loop_start)(hotplug.hotplug_loop) };
        if res != 0 {
            return Err(Error::new(
                "Pipewire: Failed to start hotplug detection loop",
            ));
        }

        Ok(())
    })();

    match result {
        Ok(()) => Ok(hotplug),
        Err(e) => {
            hotplug.hotplug_loop_destroy();
            Err(e)
        }
    }
}

impl Hotplug {
    /// Translation of `hotplug_loop_destroy()`.
    fn hotplug_loop_destroy(&self) {
        let lib = &self.lib;
        if !self.hotplug_loop.is_null() {
            // SAFETY: the loop is live; this joins its thread.
            unsafe { (lib.pw_thread_loop_stop)(self.hotplug_loop) };
        }

        self.pending_list_clear();
        self.io_list_clear();

        {
            let mut s = self.state();
            s.hotplug_init_complete = false;
            s.hotplug_events_enabled = false;

            s.pipewire_default_sink_id = None;
            s.pipewire_default_source_id = None;
        }

        // SAFETY (the teardown below): each object is live and released
        // once, in this order, with the loop thread stopped.
        if !self.hotplug_registry.is_null() {
            // SAFETY: as above.
            unsafe { (lib.pw_proxy_destroy)(self.hotplug_registry.cast()) };
        }

        if !self.hotplug_core.is_null() {
            // SAFETY: as above.
            unsafe { (lib.pw_core_disconnect)(self.hotplug_core) };
        }

        if !self.hotplug_context.is_null() {
            // SAFETY: as above.
            unsafe { (lib.pw_context_destroy)(self.hotplug_context) };
        }

        if !self.hotplug_loop.is_null() {
            // SAFETY: as above.
            unsafe { (lib.pw_thread_loop_destroy)(self.hotplug_loop) };
        }
    }
}

// Channel maps that match the order in SDL_Audio.h
const PIPEWIRE_CHANNEL_MAP_1: &[u32] = &[SPA_AUDIO_CHANNEL_MONO];
const PIPEWIRE_CHANNEL_MAP_2: &[u32] = &[SPA_AUDIO_CHANNEL_FL, SPA_AUDIO_CHANNEL_FR];
const PIPEWIRE_CHANNEL_MAP_3: &[u32] = &[
    SPA_AUDIO_CHANNEL_FL,
    SPA_AUDIO_CHANNEL_FR,
    SPA_AUDIO_CHANNEL_LFE,
];
const PIPEWIRE_CHANNEL_MAP_4: &[u32] = &[
    SPA_AUDIO_CHANNEL_FL,
    SPA_AUDIO_CHANNEL_FR,
    SPA_AUDIO_CHANNEL_RL,
    SPA_AUDIO_CHANNEL_RR,
];
const PIPEWIRE_CHANNEL_MAP_5: &[u32] = &[
    SPA_AUDIO_CHANNEL_FL,
    SPA_AUDIO_CHANNEL_FR,
    SPA_AUDIO_CHANNEL_LFE,
    SPA_AUDIO_CHANNEL_RL,
    SPA_AUDIO_CHANNEL_RR,
];
const PIPEWIRE_CHANNEL_MAP_6: &[u32] = &[
    SPA_AUDIO_CHANNEL_FL,
    SPA_AUDIO_CHANNEL_FR,
    SPA_AUDIO_CHANNEL_FC,
    SPA_AUDIO_CHANNEL_LFE,
    SPA_AUDIO_CHANNEL_RL,
    SPA_AUDIO_CHANNEL_RR,
];
const PIPEWIRE_CHANNEL_MAP_7: &[u32] = &[
    SPA_AUDIO_CHANNEL_FL,
    SPA_AUDIO_CHANNEL_FR,
    SPA_AUDIO_CHANNEL_FC,
    SPA_AUDIO_CHANNEL_LFE,
    SPA_AUDIO_CHANNEL_RC,
    SPA_AUDIO_CHANNEL_SL,
    SPA_AUDIO_CHANNEL_SR,
];
const PIPEWIRE_CHANNEL_MAP_8: &[u32] = &[
    SPA_AUDIO_CHANNEL_FL,
    SPA_AUDIO_CHANNEL_FR,
    SPA_AUDIO_CHANNEL_FC,
    SPA_AUDIO_CHANNEL_LFE,
    SPA_AUDIO_CHANNEL_RL,
    SPA_AUDIO_CHANNEL_RR,
    SPA_AUDIO_CHANNEL_SL,
    SPA_AUDIO_CHANNEL_SR,
];

/// Translation of `initialize_spa_info()`.
fn initialize_spa_info(spec: &AudioSpec, info: &mut SpaAudioInfoRaw) {
    info.channels = spec.channels as u32;
    info.rate = spec.freq as u32;

    let map: &[u32] = match spec.channels {
        1 => PIPEWIRE_CHANNEL_MAP_1,
        2 => PIPEWIRE_CHANNEL_MAP_2,
        3 => PIPEWIRE_CHANNEL_MAP_3,
        4 => PIPEWIRE_CHANNEL_MAP_4,
        5 => PIPEWIRE_CHANNEL_MAP_5,
        6 => PIPEWIRE_CHANNEL_MAP_6,
        7 => PIPEWIRE_CHANNEL_MAP_7,
        8 => PIPEWIRE_CHANNEL_MAP_8,
        _ => &[],
    };
    info.position[..map.len()].copy_from_slice(map); // COPY_CHANNEL_MAP(c)

    // Pipewire natively supports all of SDL's sample formats
    info.format = match spec.format {
        AudioFormat::U8 => SPA_AUDIO_FORMAT_U8,
        AudioFormat::S8 => SPA_AUDIO_FORMAT_S8,
        AudioFormat::S16LE => SPA_AUDIO_FORMAT_S16_LE,
        AudioFormat::S16BE => SPA_AUDIO_FORMAT_S16_BE,
        AudioFormat::S32LE => SPA_AUDIO_FORMAT_S32_LE,
        AudioFormat::S32BE => SPA_AUDIO_FORMAT_S32_BE,
        AudioFormat::F32LE => SPA_AUDIO_FORMAT_F32_LE,
        AudioFormat::F32BE => SPA_AUDIO_FORMAT_F32_BE,
        _ => SPA_AUDIO_FORMAT_UNKNOWN,
    };
}

/// An opened stream. Translation of `struct SDL_PrivateAudioData`; the
/// stream callbacks get it as their userdata (upstream passes the device).
struct PwDevice {
    lib: Arc<PwLib>,
    /// The physical device, for the stream callbacks to run the device
    /// thread iterations (a weak reference: the device owns this).
    device: Weak<PhysicalDevice>,
    recording: bool,
    silence_value: u8,

    loop_: AtomicPtr<PwThreadLoop>,
    stream: AtomicPtr<PwStream>,
    context: AtomicPtr<PwContext>,

    node_name: String,

    /// Bytes-per-frame
    stride: i32,
    stream_init_status: AtomicI32,

    /// Set in GetDeviceBuf, filled in AudioThreadIterate, queued in PlayDevice
    pw_buf: AtomicPtr<PwBuffer>,

    /// `device->buffer_size` as the buffer callback sees it while opening.
    buffer_size: AtomicUsize,
    /// A buffer size clamp from the buffer callback while opening, applied
    /// once the open finishes (see `stream_add_buffer_callback`).
    pending_resize: Mutex<Option<(i32, usize)>>,
    /// The stream name hint watcher (`SDL_AddHintCallback()`).
    name_hint: Mutex<Option<crate::hints::Callback>>,
}

// SAFETY: the PipeWire objects are used from the stream loop's thread and,
// under the loop's lock (or once it's stopped), from other threads, as
// upstream does; everything else is atomic or behind a mutex.
unsafe impl Send for PwDevice {}
// SAFETY: as above.
unsafe impl Sync for PwDevice {}

impl PwDevice {
    fn stream(&self) -> *mut PwStream {
        self.stream.load(Ordering::Acquire)
    }

    /// `PIPEWIRE_GetDeviceBuf()` proper: the dequeued buffer's memory.
    fn dequeue_device_buf(&self) -> Option<*mut u8> {
        // See if a buffer is available. If this sets *buffer_size=0, then SDL_PlaybackAudioThreadIterate will skip this iteration but try again next time.
        let stream = self.stream();
        // SAFETY: the stream is live; this runs on the loop thread.
        let pw_buf = unsafe { (self.lib.pw_stream_dequeue_buffer)(stream) };
        if pw_buf.is_null() {
            return None;
        }

        // SAFETY: a dequeued buffer has a spa_buffer with at least one data.
        let data = unsafe { (*(*(*pw_buf).buffer).datas).data };
        if data.is_null() {
            // SAFETY: we dequeued it; it goes back once.
            unsafe { (self.lib.pw_stream_queue_buffer)(stream, pw_buf) };
            return None;
        }

        self.pw_buf.store(pw_buf, Ordering::Release);
        Some(data.cast())
    }

    /// `PIPEWIRE_PlayDevice()` proper: queue the dequeued buffer with
    /// `buffer_size` bytes in it.
    fn queue_device_buf(&self, buffer_size: u32) {
        let stream = self.stream();
        let pw_buf = self.pw_buf.swap(ptr::null_mut(), Ordering::AcqRel);
        if pw_buf.is_null() {
            // FIXME (upstream): output_callback() calls PIPEWIRE_PlayDevice()
            // even when GetDeviceBuf found no buffer, which dereferences a NULL
            // pw_buf; there's nothing to queue here.
            return;
        }
        // SAFETY: we dequeued `pw_buf`; its first data has a chunk.
        unsafe {
            let spa_buf = (*pw_buf).buffer;
            let chunk = (*(*spa_buf).datas).chunk;
            (*chunk).offset = 0;
            (*chunk).stride = self.stride;
            (*chunk).size = buffer_size;

            (self.lib.pw_stream_queue_buffer)(stream, pw_buf);
        }
    }

    /// Translation of `PIPEWIRE_CloseDevice()` (also the cleanup of a failed
    /// open, which upstream leaves to the core's call of it).
    fn close(&self) {
        drop(
            self.name_hint
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .take(),
        ); // SDL_RemoveHintCallback()

        let lib = &self.lib;
        let loop_ = self.loop_.swap(ptr::null_mut(), Ordering::AcqRel);
        let stream = self.stream.swap(ptr::null_mut(), Ordering::AcqRel);
        let context = self.context.swap(ptr::null_mut(), Ordering::AcqRel);
        // SAFETY (all below): each object is live and released once; the
        // loop is stopped (its thread joined) before the others go.
        if !loop_.is_null() {
            // SAFETY: as above.
            unsafe { (lib.pw_thread_loop_stop)(loop_) };
        }

        if !stream.is_null() {
            // SAFETY: as above.
            unsafe { (lib.pw_stream_destroy)(stream) };
        }

        if !context.is_null() {
            // SAFETY: as above.
            unsafe { (lib.pw_context_destroy)(context) };
        }

        if !loop_.is_null() {
            // SAFETY: as above.
            unsafe { (lib.pw_thread_loop_destroy)(loop_) };
        }

        // (SDL_AudioThreadFinalize() is a no-op in the core.)
    }
}

/// The stream callbacks' `PwDevice`.
///
/// # Safety
///
/// `data` must be the userdata given to pw_stream_new_simple().
unsafe fn pw_device<'a>(data: *mut c_void) -> &'a PwDevice {
    // SAFETY: the userdata is the `PwDevice`, alive until the stream is destroyed.
    unsafe { &*data.cast::<PwDevice>() }
}

/// Translation of `output_callback()`.
unsafe extern "C" fn output_callback(data: *mut c_void) {
    // SAFETY: our stream userdata.
    let h = unsafe { pw_device(data) };

    // this callback can fire in a background thread during OpenDevice, while we're still blocking
    // _with the device lock_ until the stream is ready, causing a deadlock. Write silence in this case.
    if h.stream_init_status.load(Ordering::Acquire) != PW_READY_FLAG_ALL_BITS {
        // FIXME (upstream): PIPEWIRE_GetDeviceBuf() doesn't set the size,
        // so `bufsize` stays 0, nothing is silenced and an empty buffer is
        // queued.
        let bufsize: usize = 0;
        if let Some(buf) = h.dequeue_device_buf() {
            if bufsize != 0 {
                // SAFETY: the buffer has room for `bufsize` bytes.
                unsafe { ptr::write_bytes(buf, h.silence_value, bufsize) };
            }
        }
        h.queue_device_buf(bufsize as u32);
        return;
    }

    if let Some(device) = h.device.upgrade() {
        playback_audio_thread_iterate(&device);
    }
}

/// Translation of `input_callback()`.
unsafe extern "C" fn input_callback(data: *mut c_void) {
    // SAFETY: our stream userdata.
    let h = unsafe { pw_device(data) };

    // this callback can fire in a background thread during OpenDevice, while we're still blocking
    // _with the device lock_ until the stream is ready, causing a deadlock. Drop data in this case.
    if h.stream_init_status.load(Ordering::Acquire) != PW_READY_FLAG_ALL_BITS {
        h.flush();
        return;
    }

    if let Some(device) = h.device.upgrade() {
        recording_audio_thread_iterate(&device);
    }
}

/// Translation of `stream_add_buffer_callback()`.
unsafe extern "C" fn stream_add_buffer_callback(data: *mut c_void, buffer: *mut PwBuffer) {
    // SAFETY: our stream userdata.
    let h = unsafe { pw_device(data) };

    if !h.recording {
        /* Clamp the output spec samples and size to the max size of the Pipewire buffer.
        If they exceed the maximum size of the Pipewire buffer, double buffering will be used. */
        // SAFETY: libpipewire passes a valid buffer with at least one data.
        let maxsize = unsafe { (*(*(*buffer).buffer).datas).maxsize } as usize;
        let sample_frames = maxsize as i32 / h.stride;
        if h.stream_init_status.load(Ordering::Acquire) & PW_READY_FLAG_OPEN_COMPLETE == 0 {
            // FIXME (upstream): this locks the device here, but OpenDevice is
            // holding the device lock while it waits for this callback, so
            // that deadlocks; the change is applied when the open finishes.
            if h.buffer_size.load(Ordering::Acquire) > maxsize {
                *h.pending_resize.lock().unwrap_or_else(|e| e.into_inner()) =
                    Some((sample_frames, maxsize));
                h.buffer_size.store(maxsize, Ordering::Release);
            }
        } else if let Some(device) = h.device.upgrade() {
            with_device_state(&device, |st| {
                if st.buffer_size > maxsize {
                    st.sample_frames = sample_frames;
                    st.buffer_size = maxsize;
                }
            });
        }
    }

    h.stream_init_status
        .fetch_or(PW_READY_FLAG_BUFFER_ADDED, Ordering::AcqRel);
    // SAFETY: the stream's loop is alive while its callbacks run.
    unsafe { (h.lib.pw_thread_loop_signal)(h.loop_.load(Ordering::Acquire), false) };
}

/// Translation of `stream_state_changed_callback()`.
unsafe extern "C" fn stream_state_changed_callback(
    data: *mut c_void,
    _old: c_int,
    state: c_int,
    _error: *const c_char,
) {
    // SAFETY: our stream userdata.
    let h = unsafe { pw_device(data) };

    if state == PW_STREAM_STATE_STREAMING {
        h.stream_init_status
            .fetch_or(PW_READY_FLAG_STREAM_READY, Ordering::AcqRel);
    }

    if state == PW_STREAM_STATE_STREAMING || state == PW_STREAM_STATE_ERROR {
        // SAFETY: the stream's loop is alive while its callbacks run.
        unsafe { (h.lib.pw_thread_loop_signal)(h.loop_.load(Ordering::Acquire), false) };
    }
}

static STREAM_OUTPUT_EVENTS: PwStreamEvents = PwStreamEvents {
    version: PW_VERSION_STREAM_EVENTS,
    destroy: None,
    state_changed: Some(stream_state_changed_callback),
    control_info: None,
    io_changed: None,
    param_changed: None,
    add_buffer: Some(stream_add_buffer_callback),
    remove_buffer: None,
    process: Some(output_callback),
    drained: None,
    command: None,
    trigger_done: None,
};
static STREAM_INPUT_EVENTS: PwStreamEvents = PwStreamEvents {
    version: PW_VERSION_STREAM_EVENTS,
    destroy: None,
    state_changed: Some(stream_state_changed_callback),
    control_info: None,
    io_changed: None,
    param_changed: None,
    add_buffer: Some(stream_add_buffer_callback),
    remove_buffer: None,
    process: Some(input_callback),
    drained: None,
    command: None,
    trigger_done: None,
};

/// Translation of `PIPEWIRE_StreamNameChanged()`.
fn pipewire_stream_name_changed(priv_: &PwDevice, new_value: Option<&str>) {
    let stream = priv_.stream();
    let loop_ = priv_.loop_.load(Ordering::Acquire);
    if stream.is_null() || loop_.is_null() {
        return; // stream not ready yet, skip it.
    } else if new_value == Some(priv_.node_name.as_str()) {
        return; // don't set the media and node names to the same thing. Looks bad in the system UI, the node name is enough.
    }

    let value = new_value.and_then(|v| CString::new(v).ok());
    let items = [SpaDictItem {
        key: PW_KEY_MEDIA_NAME.as_ptr(),
        value: value.as_ref().map_or(ptr::null(), |v| v.as_ptr()),
    }];
    let dict = SpaDict {
        flags: 0,
        n_items: 1,
        items: items.as_ptr(),
    };

    // SAFETY: the loop and stream are live; the dict outlives the call.
    unsafe {
        (priv_.lib.pw_thread_loop_lock)(loop_);
        (priv_.lib.pw_stream_update_properties)(stream, &dict);
        (priv_.lib.pw_thread_loop_unlock)(loop_);
    }
}

impl PwDevice {
    /// Translation of `PIPEWIRE_FlushRecording()`.
    fn flush(&self) {
        let stream = self.stream();
        // SAFETY: the stream is live; this runs on its loop thread.
        let pw_buf = unsafe { (self.lib.pw_stream_dequeue_buffer)(stream) };
        if !pw_buf.is_null() {
            // just requeue it without any further thought.
            // SAFETY: we dequeued it; it goes back once.
            unsafe { (self.lib.pw_stream_queue_buffer)(stream, pw_buf) };
        }
    }
}

impl DeviceBackend for PwDevice {
    /// Translation of `PIPEWIRE_GetDeviceBuf()`.
    fn get_device_buf(&self, _device: &PhysicalDevice, buffer_size: usize) -> Option<usize> {
        // (the size isn't changed when a buffer is found)
        Some(if self.dequeue_device_buf().is_some() {
            buffer_size
        } else {
            0
        })
    }

    /// Translation of `PIPEWIRE_PlayDevice()`.
    fn play_device(&self, _device: &PhysicalDevice, buffer: &[u8]) -> bool {
        // (the core mixed into its own buffer; move it to PipeWire's)
        let pw_buf = self.pw_buf.load(Ordering::Acquire);
        if !pw_buf.is_null() {
            // SAFETY: we dequeued `pw_buf`; its first data maps `maxsize` bytes.
            unsafe {
                let d = &*(*(*pw_buf).buffer).datas;
                let n = buffer.len().min(d.maxsize as usize);
                ptr::copy_nonoverlapping(buffer.as_ptr(), d.data.cast::<u8>(), n);
            }
        }
        self.queue_device_buf(buffer.len() as u32);
        true
    }

    /// Translation of `PIPEWIRE_RecordDevice()`.
    fn record_device(&self, _device: &PhysicalDevice, buffer: &mut [u8]) -> Result<usize> {
        let stream = self.stream();
        // SAFETY: the stream is live; this runs on its loop thread.
        let pw_buf = unsafe { (self.lib.pw_stream_dequeue_buffer)(stream) };
        if pw_buf.is_null() {
            return Ok(0);
        }

        // SAFETY: we dequeued `pw_buf`.
        let spa_buf = unsafe { (*pw_buf).buffer };
        if spa_buf.is_null() {
            // SAFETY: it goes back once.
            unsafe { (self.lib.pw_stream_queue_buffer)(stream, pw_buf) };
            return Ok(0);
        }

        // SAFETY: a mapped buffer's first data has `maxsize` bytes and a chunk.
        let cpy = unsafe {
            let d = &*(*spa_buf).datas;
            let src = d.data.cast::<u8>().cast_const();
            let offset = (*d.chunk).offset.min(d.maxsize);
            let size = (*d.chunk).size.min(d.maxsize - offset);
            let cpy = buffer.len().min(size as usize);

            crate::sdl_assert!(size as usize <= buffer.len()); // We'll have to reengineer some stuff if this turns out to not be true.

            ptr::copy_nonoverlapping(src.add(offset as usize), buffer.as_mut_ptr(), cpy);
            (self.lib.pw_stream_queue_buffer)(stream, pw_buf);
            cpy
        };

        Ok(cpy)
    }

    /// Translation of `PIPEWIRE_FlushRecording()`.
    fn flush_recording(&self, _device: &PhysicalDevice) {
        self.flush();
    }

    /// Translation of `PIPEWIRE_CloseDevice()`.
    fn close_device(&self, _device: &PhysicalDevice) {
        self.close();
    }
}

/// The PipeWire driver (`PipewireInitialize()`).
struct Pipewire {
    lib: Arc<PwLib>,
    /// The hotplug loop and its state, until `hotplug_loop_destroy()`.
    hotplug: Mutex<Option<Box<Hotplug>>>,
}

impl Pipewire {
    fn hotplug(&self) -> MutexGuard<'_, Option<Box<Hotplug>>> {
        self.hotplug.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Translation of `PIPEWIRE_Deinitialize()`.
    fn pipewire_deinitialize(&self) {
        if let Some(hotplug) = self.hotplug().take() {
            hotplug.hotplug_loop_destroy();
        }
        // (pipewire_have_session_services and friends go with the hotplug state)
        deinit_pipewire_library(&self.lib);
    }
}

impl AudioDriverImpl for Pipewire {
    fn flags(&self) -> DriverFlags {
        DriverFlags {
            has_recording_support: true,
            provides_own_callback_thread: true,
            ..DriverFlags::default()
        }
    }

    /// Translation of `PIPEWIRE_DetectDevices()`.
    fn detect_devices(&self) -> (Option<Arc<PhysicalDevice>>, Option<Arc<PhysicalDevice>>) {
        let mut default_playback = None;
        let mut default_recording = None;
        let guard = self.hotplug();
        let Some(hotplug) = guard.as_ref() else {
            return (None, None);
        };

        hotplug.lock();

        // Wait until the initial registry enumeration is complete
        if !hotplug.state().hotplug_init_complete {
            hotplug.wait();
        }

        let (io_list, sink, source) = {
            let s = hotplug.state();
            (
                s.io_list.clone(),
                s.pipewire_default_sink_id.clone(),
                s.pipewire_default_source_id.clone(),
            )
        };
        for io in &io_list {
            let device =
                add_audio_device(io.recording, &io.name, None, Some(&io.spec), io.id as usize);
            if sink.as_deref() == Some(io.path.as_str()) {
                if !io.recording {
                    default_playback = device;
                }
            } else if source.as_deref() == Some(io.path.as_str()) && io.recording {
                default_recording = device;
            }
        }

        hotplug.state().hotplug_events_enabled = true;

        hotplug.unlock();

        (default_playback, default_recording)
    }

    /// Translation of `PIPEWIRE_OpenDevice()`.
    fn open_device(
        &self,
        device: &PhysicalDevice,
        state: &mut PhysState,
    ) -> Result<Arc<dyn DeviceBackend>> {
        /*
         * NOTE: The PW_STREAM_FLAG_RT_PROCESS flag can be set to call the stream
         * processing callback from the realtime thread.  However, it comes with some
         * caveats: no file IO, allocations, locking or other blocking operations
         * must occur in the mixer callback.  As this cannot be guaranteed when the
         * callback is in the calling application, this flag is omitted.
         */
        const STREAM_FLAGS: c_int = PW_STREAM_FLAG_AUTOCONNECT | PW_STREAM_FLAG_MAP_BUFFERS;

        let lib = &self.lib;
        let mut pod_buffer = [0u64; PW_POD_BUFFER_LENGTH / 8];
        // SAFETY: viewing the u64 buffer (aligned for pods) as bytes.
        let pod_bytes = unsafe {
            std::slice::from_raw_parts_mut(
                pod_buffer.as_mut_ptr().cast::<u8>(),
                PW_POD_BUFFER_LENGTH,
            )
        };
        let mut b = SpaPodBuilder::new(pod_bytes);
        let mut spa_info = SpaAudioInfoRaw::default();
        let node_id = if device.handle == 0 {
            PW_ID_ANY
        } else {
            device.handle as u32
        };
        let recording = device.recording;
        let mut wait_for_ready_timeouted = false;

        // Clamp the period size to sane values
        let min_period = PW_MIN_SAMPLES * (state.spec.freq / PW_BASE_CLOCK_RATE).max(1);

        // Get the hints for the application name, icon name, stream name and role
        let app_name = crate::init::app_metadata_property(crate::init::AppMetadata::Name);

        let icon_name = crate::hints::get(crate::hints::AUDIO_DEVICE_APP_ICON_NAME)
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "applications-games".to_owned());

        // App ID. Default to NULL if not available.
        let app_id = crate::init::app_metadata_property(crate::init::AppMetadata::Identifier);

        let stream_name = crate::hints::get(crate::hints::AUDIO_DEVICE_STREAM_NAME)
            .filter(|s| !s.is_empty())
            .or_else(|| app_name.clone())
            .or_else(|| app_id.clone())
            .unwrap_or_else(|| "SDL Audio Stream".to_owned());

        /*
         * 'Music' is the default used internally by Pipewire and it's modules,
         * but 'Game' seems more appropriate for the majority of SDL applications.
         */
        let stream_role = crate::hints::get(crate::hints::AUDIO_DEVICE_STREAM_ROLE)
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "Game".to_owned());

        // Initialize the Pipewire stream info from the SDL audio spec
        initialize_spa_info(&state.spec, &mut spa_info);
        let Some(params_range) =
            spa_format_audio_raw_build(&mut b, SPA_PARAM_ENUM_FORMAT, &spa_info)
        else {
            return Err(Error::new(
                "Pipewire: Failed to set audio format parameters",
            ));
        };
        let mut params: *const u8 = b.bytes(params_range).as_ptr();

        // node_name/description describes the app, media_name what's currently playing
        let node_name = app_name
            .clone()
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| stream_name.clone());

        // Size of a single audio frame in bytes
        let stride = state.spec.frame_size() as i32;

        if state.sample_frames < min_period {
            state.sample_frames = min_period;
        }

        updated_audio_device_format(state);

        let priv_ = Arc::new(PwDevice {
            lib: lib.clone(),
            device: find_physical_audio_device_by_handle(device.handle)
                .as_ref()
                .map_or_else(Weak::new, Arc::downgrade),
            recording,
            silence_value: state.silence_value,
            loop_: AtomicPtr::new(ptr::null_mut()),
            stream: AtomicPtr::new(ptr::null_mut()),
            context: AtomicPtr::new(ptr::null_mut()),
            node_name: node_name.clone(),
            stride,
            stream_init_status: AtomicI32::new(0),
            pw_buf: AtomicPtr::new(ptr::null_mut()),
            buffer_size: AtomicUsize::new(state.buffer_size),
            pending_resize: Mutex::new(None),
            name_hint: Mutex::new(None),
        });

        let cstr = |s: &str| CString::new(s).unwrap_or_default();
        let result = (|| -> Result<()> {
            let thread_name = cstr(&get_audio_thread_name(device));
            // SAFETY: the name is NUL-terminated; NULL props are allowed.
            let loop_ = unsafe { (lib.pw_thread_loop_new)(thread_name.as_ptr(), ptr::null()) };
            if loop_.is_null() {
                return Err(Error::new(format!(
                    "Pipewire: Failed to create stream loop ({})",
                    errno()
                )));
            }
            priv_.loop_.store(loop_, Ordering::Release);

            // Load the realtime module so Pipewire can set the loop thread to the appropriate priority.
            // SAFETY: a NULL-terminated list of NUL-terminated key/value strings.
            let props = unsafe {
                (lib.pw_properties_new)(
                    PW_KEY_CONFIG_NAME.as_ptr(),
                    c"client-rt.conf".as_ptr(),
                    ptr::null::<c_char>(),
                )
            };
            if props.is_null() {
                return Err(Error::new(format!(
                    "Pipewire: Failed to create stream context properties ({})",
                    errno()
                )));
            }

            // SAFETY: the loop is live; the context takes the props.
            let context =
                unsafe { (lib.pw_context_new)((lib.pw_thread_loop_get_loop)(loop_), props, 0) };
            if context.is_null() {
                return Err(Error::new(format!(
                    "Pipewire: Failed to create stream context ({})",
                    errno()
                )));
            }
            priv_.context.store(context, Ordering::Release);

            // SAFETY: an empty, NULL-terminated list.
            let props = unsafe { (lib.pw_properties_new)(ptr::null(), ptr::null::<c_char>()) };
            if props.is_null() {
                return Err(Error::new(format!(
                    "Pipewire: Failed to create stream properties ({})",
                    errno()
                )));
            }

            let set = |key: &CStr, value: &str| {
                let value = cstr(value);
                // SAFETY: `props` is live; both strings are NUL-terminated.
                unsafe { (lib.pw_properties_set)(props, key.as_ptr(), value.as_ptr()) };
            };
            set(PW_KEY_MEDIA_TYPE, "Audio");
            set(
                PW_KEY_MEDIA_CATEGORY,
                if recording { "Capture" } else { "Playback" },
            );
            set(PW_KEY_MEDIA_ROLE, &stream_role);
            set(PW_KEY_APP_NAME, app_name.as_deref().unwrap_or(""));
            set(PW_KEY_APP_ICON_NAME, &icon_name);
            if let Some(app_id) = &app_id {
                set(PW_KEY_APP_ID, app_id);
            }

            set(PW_KEY_NODE_NAME, &node_name);
            set(PW_KEY_NODE_DESCRIPTION, &node_name);
            // SAFETY: `props` is live; the format matches the arguments.
            unsafe {
                (lib.pw_properties_setf)(
                    props,
                    PW_KEY_NODE_LATENCY.as_ptr(),
                    c"%u/%i".as_ptr(),
                    state.sample_frames as std::ffi::c_uint,
                    state.spec.freq as c_int,
                );
                (lib.pw_properties_setf)(
                    props,
                    PW_KEY_NODE_RATE.as_ptr(),
                    c"1/%u".as_ptr(),
                    state.spec.freq as std::ffi::c_uint,
                );
            }
            set(PW_KEY_NODE_ALWAYS_PROCESS, "true");

            // only set a stream-specific name if it's different than the app name, otherwise the system UI might show the stream as "Team Fortress 2 - Team Fortress 2" or whatever. Better to just show the name once.
            if node_name != stream_name {
                set(PW_KEY_MEDIA_NAME, &stream_name);
            }

            // UPDATE: This prevents users from moving the audio to a new sink (device) using standard tools. This is slightly in conflict
            //  with how SDL wants to manage audio devices, but if people want to do it, we should let them, so this is commented out
            //  for now. We might revisit later.
            //PIPEWIRE_pw_properties_set(props, PW_KEY_NODE_DONT_RECONNECT, "true");  // Requesting a specific device, don't migrate to new default hardware.

            if node_id != PW_ID_ANY {
                if let Some(hotplug) = self.hotplug().as_ref() {
                    hotplug.lock();
                    let node = hotplug.io_list_get_by_id(node_id);
                    if let Some(node) = node {
                        set(PW_KEY_TARGET_OBJECT, &node.path);
                    }
                    hotplug.unlock();
                }
            }

            // add this early so it will do its initial trigger when we aren't setup--skipping the attempt to update the name--since we're already explicitly setting it here.
            let weak = Arc::downgrade(&priv_);
            let watcher =
                crate::hints::watch(crate::hints::AUDIO_DEVICE_STREAM_NAME, move |change| {
                    if let Some(priv_) = weak.upgrade() {
                        pipewire_stream_name_changed(&priv_, change.new_value);
                    }
                })
                .ok();
            *priv_.name_hint.lock().unwrap_or_else(|e| e.into_inner()) = watcher;

            // Create the new stream
            let stream_name_c = cstr(&stream_name);
            // SAFETY: the loop is live; the stream takes the props; the
            // events are static; the userdata (`priv_`) outlives the stream.
            let stream = unsafe {
                (lib.pw_stream_new_simple)(
                    (lib.pw_thread_loop_get_loop)(loop_),
                    stream_name_c.as_ptr(),
                    props,
                    if recording {
                        &STREAM_INPUT_EVENTS
                    } else {
                        &STREAM_OUTPUT_EVENTS
                    },
                    Arc::as_ptr(&priv_).cast_mut().cast(),
                )
            };
            if stream.is_null() {
                return Err(Error::new(format!(
                    "Pipewire: Failed to create stream ({})",
                    errno()
                )));
            }
            priv_.stream.store(stream, Ordering::Release);

            // The target node is passed via PW_KEY_TARGET_OBJECT; target_id is a legacy parameter and must be PW_ID_ANY.
            // SAFETY: the stream is live; `params` points to the built pod.
            let res = unsafe {
                (lib.pw_stream_connect)(
                    stream,
                    if recording {
                        PW_DIRECTION_INPUT
                    } else {
                        PW_DIRECTION_OUTPUT
                    },
                    PW_ID_ANY,
                    STREAM_FLAGS,
                    &mut params,
                    1,
                )
            };
            if res != 0 {
                return Err(Error::new("Pipewire: Failed to connect stream"));
            }

            // SAFETY: the loop is live.
            let res = unsafe { (lib.pw_thread_loop_start)(loop_) };
            if res != 0 {
                return Err(Error::new("Pipewire: Failed to start stream loop"));
            }

            // Wait until timeout (no device), or all pre-open init flags are set, or the stream has failed
            // SAFETY (the loop calls): the loop and stream are live.
            unsafe { (lib.pw_thread_loop_lock)(loop_) };
            while !wait_for_ready_timeouted
                && priv_.stream_init_status.load(Ordering::Acquire) != PW_READY_FLAG_ALL_PREOPEN_BITS
                // SAFETY: as above.
                && unsafe { (lib.pw_stream_get_state)(stream, ptr::null_mut()) } != PW_STREAM_STATE_ERROR
            {
                // SAFETY: as above; we hold the lock.
                wait_for_ready_timeouted =
                    unsafe { (lib.pw_thread_loop_timed_wait)(loop_, 2) } == libc::ETIMEDOUT;
            }
            priv_
                .stream_init_status
                .fetch_or(PW_READY_FLAG_OPEN_COMPLETE, Ordering::AcqRel);
            // SAFETY: as above.
            unsafe { (lib.pw_thread_loop_unlock)(loop_) };

            if wait_for_ready_timeouted {
                return Err(Error::new(
                    "Pipewire: timeout waiting for audio device to be ready",
                ));
            }

            let mut error: *const c_char = ptr::null();
            // SAFETY: the stream is live; `error` is a valid out-pointer.
            if unsafe { (lib.pw_stream_get_state)(stream, &mut error) } == PW_STREAM_STATE_ERROR {
                // SAFETY: libpipewire's error string (or NULL).
                let error = unsafe { c_str(error) }.unwrap_or("(null)");
                return Err(Error::new(format!("Pipewire: Stream error: {error}")));
            }

            Ok(())
        })();

        if let Err(e) = result {
            priv_.close(); // (the core's PIPEWIRE_CloseDevice() call)
            return Err(e);
        }

        // (the buffer size clamp the buffer callback deferred, see there)
        if let Some((sample_frames, buffer_size)) = priv_
            .pending_resize
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            if state.buffer_size > buffer_size {
                state.sample_frames = sample_frames;
                state.buffer_size = buffer_size;
            }
        }

        Ok(priv_)
    }

    /// Translation of `PIPEWIRE_DeinitializeStart()`.
    fn deinitialize_start(&self) {
        if let Some(hotplug) = self.hotplug().take() {
            hotplug.hotplug_loop_destroy();
        }
    }

    /// Translation of `PIPEWIRE_Deinitialize()`.
    fn deinitialize(&self) {
        self.pipewire_deinitialize();
    }
}

/// Translation of `PipewireInitialize()`.
fn pipewire_initialize() -> Option<Pipewire> {
    let lib = init_pipewire_library()?;

    let pipewire = Pipewire {
        lib: lib.clone(),
        hotplug: Mutex::new(None),
    };

    match hotplug_loop_init(&lib) {
        Ok(hotplug) => *pipewire.hotplug() = Some(hotplug),
        Err(_) => {
            pipewire.pipewire_deinitialize();
            return None;
        }
    }

    Some(pipewire)
}

/// Translation of `PIPEWIRE_PREFERRED_Init()`.
fn pipewire_preferred_init() -> Option<Arc<dyn AudioDriverImpl>> {
    let pipewire = pipewire_initialize()?;

    let fail = {
        let guard = pipewire.hotplug();
        let hotplug = guard.as_ref()?;

        // run device detection but don't add any devices to SDL; we're just waiting to see if PipeWire sees any devices. If not, fall back to the next backend.
        hotplug.lock();

        // Wait until the initial registry enumeration is complete
        if !hotplug.state().hotplug_init_complete {
            hotplug.wait();
        }

        let no_devices = hotplug.state().io_list.is_empty();

        hotplug.unlock();

        let (have_session_services, have_audio_service) = {
            let s = hotplug.state();
            (
                s.pipewire_have_session_services,
                s.pipewire_have_audio_service,
            )
        };
        (have_session_services && !have_audio_service)
            || (!have_session_services
                && (no_devices || !hotplug.pipewire_core_version_at_least(1, 0, 0)))
    };
    if fail {
        pipewire.pipewire_deinitialize();
        return None;
    }

    Some(Arc::new(pipewire)) // this will move on to PIPEWIRE_DetectDevices and reuse hotplug_io_list.
}

/// Translation of `PIPEWIRE_Init()`.
fn pipewire_init() -> Option<Arc<dyn AudioDriverImpl>> {
    pipewire_initialize().map(|p| Arc::new(p) as Arc<dyn AudioDriverImpl>)
}

/// Translation of `PIPEWIRE_PREFERRED_bootstrap`.
pub(crate) static PIPEWIRE_PREFERRED_BOOTSTRAP: AudioBootStrap = AudioBootStrap {
    name: "pipewire",
    desc: "Pipewire",
    init: pipewire_preferred_init,
    demand_only: false,
    is_preferred: true,
};

/// Translation of `PIPEWIRE_bootstrap`.
pub(crate) static PIPEWIRE_BOOTSTRAP: AudioBootStrap = AudioBootStrap {
    name: "pipewire",
    desc: "Pipewire",
    init: pipewire_init,
    demand_only: false,
    is_preferred: false,
};

#[cfg(test)]
mod tests;
