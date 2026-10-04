// The parts of libpipewire's API (<pipewire/pipewire.h> and the SPA
// headers) that SDL's PipeWire audio and camera drivers share. Not a
// translation of one upstream file: each driver declares these for
// itself in C (src/audio/pipewire/SDL_pipewire.c,
// src/camera/pipewire/SDL_camera_pipewire.c, which include the headers);
// here they're declared once. libpipewire is loaded at run time by each
// driver, with its own symbol list ([`pipewire_syms!`]).
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The libpipewire declarations both PipeWire drivers use, the interface
//! method macros (`pw_core_sync()`, `pw_registry_bind()`, ...) translated
//! as functions, and the SPA inline helpers (in [`spa`]).

pub(crate) mod spa;

use std::cell::UnsafeCell;
use std::ffi::{c_char, c_int, c_void, CStr};
use std::ptr;

use spa::SpaDict;

macro_rules! opaque {
    ($($name:ident),*) => {
        $(
            #[repr(C)]
            pub(crate) struct $name {
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

pub(crate) const PW_ID_CORE: u32 = 0;
pub(crate) const PW_ID_ANY: u32 = 0xffff_ffff;
pub(crate) const PW_VERSION_REGISTRY: u32 = 3;
pub(crate) const PW_VERSION_CORE_EVENTS: u32 = 1;
pub(crate) const PW_VERSION_REGISTRY_EVENTS: u32 = 0;
pub(crate) const PW_VERSION_NODE_EVENTS: u32 = 0;
pub(crate) const PW_VERSION_STREAM_EVENTS: u32 = 2;

pub(crate) const PW_TYPE_INTERFACE_NODE: &str = "PipeWire:Interface:Node";
pub(crate) const PW_KEY_NODE_NAME: &CStr = c"node.name";
pub(crate) const PW_KEY_NODE_DESCRIPTION: &CStr = c"node.description";
pub(crate) const PW_KEY_MEDIA_TYPE: &CStr = c"media.type";
pub(crate) const PW_KEY_MEDIA_CATEGORY: &CStr = c"media.category";
pub(crate) const PW_KEY_MEDIA_ROLE: &CStr = c"media.role";
pub(crate) const PW_KEY_MEDIA_CLASS: &str = "media.class";
pub(crate) const PW_KEY_TARGET_OBJECT: &CStr = c"target.object";

// enum pw_stream_state
pub(crate) const PW_STREAM_STATE_ERROR: c_int = -1;
pub(crate) const PW_STREAM_STATE_STREAMING: c_int = 3;

// enum pw_direction (spa_direction)
pub(crate) const PW_DIRECTION_INPUT: c_int = 0;
pub(crate) const PW_DIRECTION_OUTPUT: c_int = 1;

// enum pw_stream_flags
pub(crate) const PW_STREAM_FLAG_AUTOCONNECT: c_int = 1 << 0;
pub(crate) const PW_STREAM_FLAG_MAP_BUFFERS: c_int = 1 << 2;

pub(crate) const SPA_PARAM_ENUM_FORMAT: u32 = 3;
pub(crate) const SPA_PARAM_FORMAT: u32 = 4;

/// `struct spa_list`.
#[repr(C)]
pub(crate) struct SpaList {
    pub(crate) next: *mut SpaList,
    pub(crate) prev: *mut SpaList,
}

/// `struct spa_callbacks`.
#[repr(C)]
pub(crate) struct SpaCallbacks {
    pub(crate) funcs: *const c_void,
    pub(crate) data: *mut c_void,
}

/// `struct spa_interface`: what every proxy (core, registry, node) starts with.
#[repr(C)]
pub(crate) struct SpaInterface {
    pub(crate) type_: *const c_char,
    pub(crate) version: u32,
    pub(crate) cb: SpaCallbacks,
}

/// `struct spa_hook`. libpipewire links it into its lists, so it lives in
/// an `UnsafeCell` at a stable address.
#[repr(C)]
pub(crate) struct SpaHook {
    pub(crate) link: SpaList,
    pub(crate) cb: SpaCallbacks,
    pub(crate) removed: Option<unsafe extern "C" fn(*mut SpaHook)>,
    pub(crate) priv_: *mut c_void,
}

impl SpaHook {
    /// `spa_zero(hook)`.
    pub(crate) const fn zeroed() -> UnsafeCell<SpaHook> {
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
pub(crate) unsafe fn spa_hook_remove(hook: *mut SpaHook) {
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
pub(crate) struct PwCoreInfo {
    pub(crate) id: u32,
    pub(crate) cookie: u32,
    pub(crate) user_name: *const c_char,
    pub(crate) host_name: *const c_char,
    pub(crate) version: *const c_char,
    pub(crate) name: *const c_char,
    pub(crate) change_mask: u64,
    pub(crate) props: *mut SpaDict,
}

/// `struct spa_param_info`.
#[repr(C)]
pub(crate) struct SpaParamInfo {
    pub(crate) id: u32,
    pub(crate) flags: u32,
    pub(crate) user: u32,
    pub(crate) seq: i32,
    pub(crate) padding: [u32; 4],
}

/// `struct pw_node_info`.
#[repr(C)]
pub(crate) struct PwNodeInfo {
    pub(crate) id: u32,
    pub(crate) max_input_ports: u32,
    pub(crate) max_output_ports: u32,
    pub(crate) change_mask: u64,
    pub(crate) n_input_ports: u32,
    pub(crate) n_output_ports: u32,
    pub(crate) state: c_int,
    pub(crate) error: *const c_char,
    pub(crate) props: *mut SpaDict,
    pub(crate) params: *mut SpaParamInfo,
    pub(crate) n_params: u32,
}

/// `struct spa_chunk`.
#[repr(C)]
pub(crate) struct SpaChunk {
    pub(crate) offset: u32,
    pub(crate) size: u32,
    pub(crate) stride: i32,
    pub(crate) flags: i32,
}

/// `struct spa_data`.
#[repr(C)]
pub(crate) struct SpaData {
    pub(crate) type_: u32,
    pub(crate) flags: u32,
    pub(crate) fd: i64,
    pub(crate) mapoffset: u32,
    pub(crate) maxsize: u32,
    pub(crate) data: *mut c_void,
    pub(crate) chunk: *mut SpaChunk,
}

/// `struct spa_buffer`.
#[repr(C)]
pub(crate) struct SpaBuffer {
    pub(crate) n_metas: u32,
    pub(crate) n_datas: u32,
    pub(crate) metas: *mut SpaMeta,
    pub(crate) datas: *mut SpaData,
}

/// `struct pw_buffer`.
#[repr(C)]
pub(crate) struct PwBuffer {
    pub(crate) buffer: *mut SpaBuffer,
    pub(crate) user_data: *mut c_void,
    pub(crate) size: u64,
    pub(crate) requested: u64,
    pub(crate) time: u64,
}

pub(crate) type Unused = Option<unsafe extern "C" fn()>;

/// `struct pw_core_events`.
#[repr(C)]
pub(crate) struct PwCoreEvents {
    pub(crate) version: u32,
    pub(crate) info: Option<unsafe extern "C" fn(*mut c_void, *const PwCoreInfo)>,
    pub(crate) done: Option<unsafe extern "C" fn(*mut c_void, u32, c_int)>,
    pub(crate) ping: Unused,
    pub(crate) error: Unused,
    pub(crate) remove_id: Unused,
    pub(crate) bound_id: Unused,
    pub(crate) add_mem: Unused,
    pub(crate) remove_mem: Unused,
    pub(crate) bound_props: Unused,
}

/// `struct pw_core_methods`.
#[repr(C)]
pub(crate) struct PwCoreMethods {
    pub(crate) version: u32,
    pub(crate) add_listener: Option<
        unsafe extern "C" fn(*mut c_void, *mut SpaHook, *const PwCoreEvents, *mut c_void) -> c_int,
    >,
    pub(crate) hello: Unused,
    pub(crate) sync: Option<unsafe extern "C" fn(*mut c_void, u32, c_int) -> c_int>,
    pub(crate) pong: Unused,
    pub(crate) error: Unused,
    pub(crate) get_registry:
        Option<unsafe extern "C" fn(*mut c_void, u32, usize) -> *mut PwRegistry>,
    pub(crate) create_object: Unused,
    pub(crate) destroy: Unused,
}

/// `struct pw_registry_events`.
#[repr(C)]
pub(crate) struct PwRegistryEvents {
    pub(crate) version: u32,
    pub(crate) global:
        Option<unsafe extern "C" fn(*mut c_void, u32, u32, *const c_char, u32, *const SpaDict)>,
    pub(crate) global_remove: Option<unsafe extern "C" fn(*mut c_void, u32)>,
}

/// `struct pw_registry_methods`.
#[repr(C)]
pub(crate) struct PwRegistryMethods {
    pub(crate) version: u32,
    pub(crate) add_listener: Option<
        unsafe extern "C" fn(
            *mut c_void,
            *mut SpaHook,
            *const PwRegistryEvents,
            *mut c_void,
        ) -> c_int,
    >,
    pub(crate) bind:
        Option<unsafe extern "C" fn(*mut c_void, u32, *const c_char, u32, usize) -> *mut c_void>,
    pub(crate) destroy: Unused,
}

/// `struct pw_node_events`.
#[repr(C)]
pub(crate) struct PwNodeEvents {
    pub(crate) version: u32,
    pub(crate) info: Option<unsafe extern "C" fn(*mut c_void, *const PwNodeInfo)>,
    pub(crate) param: Option<unsafe extern "C" fn(*mut c_void, c_int, u32, u32, u32, *const u8)>,
}

/// `struct pw_node_methods`.
#[repr(C)]
pub(crate) struct PwNodeMethods {
    pub(crate) version: u32,
    pub(crate) add_listener: Unused,
    pub(crate) subscribe_params: Unused,
    pub(crate) enum_params:
        Option<unsafe extern "C" fn(*mut c_void, c_int, u32, u32, u32, *const u8) -> c_int>,
    pub(crate) set_param: Unused,
    pub(crate) send_command: Unused,
}

/// `struct pw_stream_events`.
#[repr(C)]
pub(crate) struct PwStreamEvents {
    pub(crate) version: u32,
    pub(crate) destroy: Unused,
    pub(crate) state_changed:
        Option<unsafe extern "C" fn(*mut c_void, c_int, c_int, *const c_char)>,
    pub(crate) control_info: Unused,
    pub(crate) io_changed: Unused,
    pub(crate) param_changed: Option<unsafe extern "C" fn(*mut c_void, u32, *const u8)>,
    pub(crate) add_buffer: Option<unsafe extern "C" fn(*mut c_void, *mut PwBuffer)>,
    pub(crate) remove_buffer: Option<unsafe extern "C" fn(*mut c_void, *mut PwBuffer)>,
    pub(crate) process: Option<unsafe extern "C" fn(*mut c_void)>,
    pub(crate) drained: Unused,
    pub(crate) command: Unused,
    pub(crate) trigger_done: Unused,
}

/// The methods table and data of a proxy's interface, if it has one
/// (`spa_interface_call_res()`; every call here is version 0).
///
/// # Safety
///
/// `object` must be a live proxy whose interface's methods are `M`.
pub(crate) unsafe fn interface_methods<'a, M>(object: *mut c_void) -> Option<(&'a M, *mut c_void)> {
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
pub(crate) unsafe fn pw_core_add_listener(
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
pub(crate) unsafe fn pw_core_sync(core: *mut PwCore, id: u32, seq: c_int) -> c_int {
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
pub(crate) unsafe fn pw_core_get_registry(
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
pub(crate) unsafe fn pw_registry_add_listener(
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
pub(crate) unsafe fn pw_registry_bind(
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
pub(crate) unsafe fn pw_node_enum_params(
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

/// `errno`, for the error messages.
pub(crate) fn errno() -> i32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

/// A C string as `&str` (empty for NULL or invalid UTF-8).
///
/// # Safety
///
/// `s` must be NULL or a C string that outlives `'a`.
pub(crate) unsafe fn c_str<'a>(s: *const c_char) -> Option<&'a str> {
    if s.is_null() {
        None
    } else {
        // SAFETY: the caller's contract.
        Some(unsafe { CStr::from_ptr(s) }.to_str().unwrap_or(""))
    }
}

/// Parse `"%d.%d.%d"` like `SDL_sscanf()`, returning how many matched.
pub(crate) fn sscanf_version(s: &str, out: &mut [i32; 3]) -> usize {
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

/// `struct pw_proxy_events`.
#[repr(C)]
pub(crate) struct PwProxyEvents {
    pub(crate) version: u32,
    pub(crate) destroy: Option<unsafe extern "C" fn(*mut c_void)>,
    pub(crate) bound: Unused,
    pub(crate) removed: Option<unsafe extern "C" fn(*mut c_void)>,
    pub(crate) done: Unused,
    pub(crate) error: Unused,
    pub(crate) bound_props: Unused,
}

pub(crate) const PW_VERSION_PROXY_EVENTS: u32 = 1;

/// Declares a driver's table of libpipewire entry points (the
/// `PIPEWIRE_pw_*` function pointers of its C file) and its
/// `load_pipewire_syms()`: every symbol must load, as upstream requires.
/// `pw_properties_new()` and `pw_properties_setf()` (variadic) are always
/// part of it.
macro_rules! pipewire_syms {
    ($(#[$meta:meta])* struct $lib:ident { $($name:ident: fn($($arg:ty),*) $(-> $ret:ty)?;)* }) => {
        $(#[$meta])*
        #[allow(dead_code)]
        struct $lib {
            $($name: unsafe extern "C" fn($($arg),*) $(-> $ret)?,)*
            pw_properties_new: unsafe extern "C" fn(*const ::std::ffi::c_char, ...)
                -> *mut $crate::core::linux::pipewire::PwProperties,
            pw_properties_setf: unsafe extern "C" fn(
                *mut $crate::core::linux::pipewire::PwProperties,
                *const ::std::ffi::c_char,
                *const ::std::ffi::c_char,
                ...
            ) -> ::std::ffi::c_int,
            _handle: $crate::loadso::SharedObject,
        }

        impl $lib {
            /// Translation of `load_pipewire_syms()`.
            fn load_syms(handle: $crate::loadso::SharedObject) -> $crate::error::Result<$lib> {
                Ok($lib {
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
pub(crate) use pipewire_syms;
