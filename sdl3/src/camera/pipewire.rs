// Rust translation of src/camera/pipewire/SDL_camera_pipewire.c from Simple
// DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The PipeWire camera driver: `Video/Source` nodes, found through the
//! registry on a hotplug thread loop, read through a capture stream each.
//! libpipewire is loaded at run time (`SDL_CAMERA_DRIVER_PIPEWIRE_DYNAMIC`),
//! with its own symbol list; the declarations, the interface method macros
//! and the SPA pod helpers are shared with the audio driver
//! (`core::linux::pipewire`). Inside a sandbox, access goes through the
//! camera portal first.
//!
//! Upstream keeps the hotplug loop in a file-level static and its globals
//! in their proxies' user data, linked in `spa_list`s; here the hotplug
//! state is one shared object the callbacks get as their userdata, each
//! global is boxed (its hooks must not move), and the lists are `Vec`s. A
//! frame gets a copy of its buffer's data; the buffer goes back to the
//! stream when the frame is released, as upstream.

use std::cell::UnsafeCell;
use std::ffi::{c_char, c_int, c_void, CString};
use std::os::fd::IntoRawFd;
use std::ptr;
use std::sync::{Arc, Mutex, MutexGuard, Weak};

use super::{
    add_camera, camera_permission_outcome, AcquiredFrame, CameraBackend, CameraBootStrap,
    CameraDevice, CameraDriverImpl, CameraFrameResult, CameraPosition, CameraSpec,
};
use crate::core::linux::dbus::{camera_portal_request_access, CameraPortalAccess};
use crate::core::linux::pipewire::spa::{
    spa_dict_lookup, spa_pod_find_prop, spa_pod_get_values, spa_pod_is_object, Pod, SpaDict,
    SpaPodBuilder, SPA_CHOICE_ENUM, SPA_CHOICE_NONE, SPA_FORMAT_MEDIA_SUBTYPE,
    SPA_FORMAT_MEDIA_TYPE, SPA_FORMAT_VIDEO_FORMAT, SPA_FORMAT_VIDEO_FRAMERATE,
    SPA_FORMAT_VIDEO_SIZE, SPA_MEDIA_SUBTYPE_MJPG, SPA_MEDIA_SUBTYPE_RAW, SPA_MEDIA_TYPE_VIDEO,
    SPA_TYPE_FRACTION, SPA_TYPE_ID, SPA_TYPE_OBJECT_FORMAT, SPA_TYPE_RECTANGLE,
};
use crate::core::linux::pipewire::*;
use crate::error::{Error, Result};
use crate::loadso::SharedObject;
use crate::video::{Colorspace, PixelFormat};

const PW_REQUIRED_MAJOR: i32 = 1;
const PW_REQUIRED_MINOR: i32 = 0;
const PW_REQUIRED_PATCH: i32 = 0;

const PW_VERSION_NODE: u32 = 3;
const PW_NODE_CHANGE_MASK_PARAMS: u64 = 1 << 4;
const SPA_PARAM_INFO_READ: u32 = 1 << 1;
const SPA_ID_INVALID: u32 = 0xffff_ffff;

/// `SPA_RESULT_IS_ASYNC()`.
const fn spa_result_is_async(res: c_int) -> bool {
    const SPA_ASYNC_BIT: u32 = 1 << 30;
    const SPA_ASYNC_MASK: u32 = 3 << 30;
    (res as u32 & SPA_ASYNC_MASK) == SPA_ASYNC_BIT
}

// enum spa_video_format
const SPA_VIDEO_FORMAT_UNKNOWN: u32 = 0;
const SPA_VIDEO_FORMAT_I420: u32 = 2;
const SPA_VIDEO_FORMAT_YV12: u32 = 3;
const SPA_VIDEO_FORMAT_YUY2: u32 = 4;
const SPA_VIDEO_FORMAT_UYVY: u32 = 5;
const SPA_VIDEO_FORMAT_RGBX: u32 = 7;
const SPA_VIDEO_FORMAT_BGRX: u32 = 8;
const SPA_VIDEO_FORMAT_XRGB: u32 = 9;
const SPA_VIDEO_FORMAT_XBGR: u32 = 10;
const SPA_VIDEO_FORMAT_RGBA: u32 = 11;
const SPA_VIDEO_FORMAT_BGRA: u32 = 12;
const SPA_VIDEO_FORMAT_ARGB: u32 = 13;
const SPA_VIDEO_FORMAT_ABGR: u32 = 14;
const SPA_VIDEO_FORMAT_RGB: u32 = 15;
const SPA_VIDEO_FORMAT_BGR: u32 = 16;
const SPA_VIDEO_FORMAT_YVYU: u32 = 19;
const SPA_VIDEO_FORMAT_NV12: u32 = 23;
const SPA_VIDEO_FORMAT_NV21: u32 = 24;

pipewire_syms! {
    /// The libpipewire entry points the camera driver uses (the
    /// `PIPEWIRE_pw_*` function pointers), with the library they came from.
    /// (All are loaded, as upstream does, though not every one is called.)
    struct PwLib {
        pw_get_library_version: fn() -> *const c_char;
        pw_check_library_version: fn(c_int, c_int, c_int) -> bool;
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
        pw_thread_loop_start: fn(*mut PwThreadLoop) -> c_int;
        pw_context_new: fn(*mut PwLoop, *mut PwProperties, usize) -> *mut PwContext;
        pw_context_destroy: fn(*mut PwContext);
        pw_context_connect: fn(*mut PwContext, *mut PwProperties, usize) -> *mut PwCore;
        pw_context_connect_fd: fn(*mut PwContext, c_int, *mut PwProperties, usize) -> *mut PwCore;
        pw_proxy_add_listener: fn(*mut PwProxy, *mut SpaHook, *const PwProxyEvents, *mut c_void);
        pw_proxy_add_object_listener: fn(*mut PwProxy, *mut SpaHook, *const c_void, *mut c_void);
        pw_proxy_get_user_data: fn(*mut PwProxy) -> *mut c_void;
        pw_proxy_destroy: fn(*mut PwProxy);
        pw_core_disconnect: fn(*mut PwCore) -> c_int;
        pw_node_info_merge: fn(*mut PwNodeInfo, *const PwNodeInfo, bool) -> *mut PwNodeInfo;
        pw_node_info_free: fn(*mut PwNodeInfo);
        pw_stream_new: fn(*mut PwCore, *const c_char, *mut PwProperties) -> *mut PwStream;
        pw_stream_add_listener: fn(*mut PwStream, *mut SpaHook, *const PwStreamEvents, *mut c_void);
        pw_stream_destroy: fn(*mut PwStream);
        pw_stream_connect: fn(*mut PwStream, c_int, u32, c_int, *mut *const u8, u32) -> c_int;
        pw_stream_get_state: fn(*mut PwStream, *mut *const c_char) -> c_int;
        pw_stream_dequeue_buffer: fn(*mut PwStream) -> *mut PwBuffer;
        pw_stream_queue_buffer: fn(*mut PwStream, *mut PwBuffer) -> c_int;
        pw_properties_new_dict: fn(*const SpaDict) -> *mut PwProperties;
        pw_properties_set: fn(*mut PwProperties, *const c_char, *const c_char) -> c_int;
    }
}

/// `SDL_CAMERA_DRIVER_PIPEWIRE_DYNAMIC`.
const PIPEWIRE_LIBRARY: &str = "libpipewire-0.3.so.0";

/// Translation of `init_pipewire_library()` (with `load_pipewire_library()`).
fn init_pipewire_library() -> Result<Arc<PwLib>> {
    let handle = SharedObject::load(PIPEWIRE_LIBRARY)?;
    let lib = PwLib::load_syms(handle)?;
    // SAFETY: pw_init() accepts NULL arguments.
    unsafe { (lib.pw_init)(ptr::null_mut(), ptr::null_mut()) };
    Ok(Arc::new(lib))
}

/// Translation of `deinit_pipewire_library()` (the library is unloaded with
/// its last reference).
fn deinit_pipewire_library(lib: &PwLib) {
    // SAFETY: balances the pw_init() of init_pipewire_library().
    unsafe { (lib.pw_deinit)() };
}

// --- params ---

/// A param of a node, copied out of libpipewire's pod. Translation of
/// `struct param`.
#[derive(Clone, Debug, PartialEq)]
struct Param {
    id: u32,
    seq: i32,
    /// The pod's bytes; `None` marks "clear the params of this id".
    param: Option<Vec<u8>>,
}

impl Param {
    fn pod(&self) -> Option<Pod<'_>> {
        self.param.as_deref().and_then(Pod::new)
    }
}

/// Translation of `param_clear()`: remove the params of `id` (all for
/// `SPA_ID_INVALID`); returns how many.
fn param_clear(param_list: &mut Vec<Param>, id: u32) -> u32 {
    let before = param_list.len();
    param_list.retain(|p| !(id == SPA_ID_INVALID || p.id == id));
    (before - param_list.len()) as u32
}

/// Translation of `param_add()`: false (upstream's `errno = EINVAL`) when
/// the id comes from a param that isn't an object.
fn param_add(params: &mut Vec<Param>, seq: i32, mut id: u32, param: Option<Pod<'_>>) -> bool {
    if id == SPA_ID_INVALID {
        match param {
            Some(p) if spa_pod_is_object(p) => id = p.object_id(),
            _ => return false,
        }
    }

    let param = match param {
        Some(p) => Some(p.as_bytes().to_vec()),
        None => {
            param_clear(params, id);
            None
        }
    };
    params.push(Param { id, seq, param });

    true
}

/// Translation of `param_update()`: drop the pending results of stale
/// enumerations, then apply the pending list to the param list.
fn param_update(
    param_list: &mut Vec<Param>,
    pending_list: &mut Vec<Param>,
    params: &[SpaParamInfo],
) {
    for info in params {
        pending_list.retain(|p| !(p.id == info.id && p.seq != info.seq && p.param.is_some()));
    }
    for p in pending_list.drain(..) {
        if p.param.is_none() {
            param_clear(param_list, p.id);
        } else {
            param_list.push(p);
        }
    }
}

// --- formats ---

/// Translation of `sdl_video_formats`.
const SDL_VIDEO_FORMATS: [(PixelFormat, Colorspace, u32); 17] = [
    (PixelFormat::RGBX32, Colorspace::SRGB, SPA_VIDEO_FORMAT_RGBX),
    (PixelFormat::XRGB32, Colorspace::SRGB, SPA_VIDEO_FORMAT_XRGB),
    (PixelFormat::BGRX32, Colorspace::SRGB, SPA_VIDEO_FORMAT_BGRX),
    (PixelFormat::XBGR32, Colorspace::SRGB, SPA_VIDEO_FORMAT_XBGR),
    (PixelFormat::RGBA32, Colorspace::SRGB, SPA_VIDEO_FORMAT_RGBA),
    (PixelFormat::ARGB32, Colorspace::SRGB, SPA_VIDEO_FORMAT_ARGB),
    (PixelFormat::BGRA32, Colorspace::SRGB, SPA_VIDEO_FORMAT_BGRA),
    (PixelFormat::ABGR32, Colorspace::SRGB, SPA_VIDEO_FORMAT_ABGR),
    (PixelFormat::RGB24, Colorspace::SRGB, SPA_VIDEO_FORMAT_RGB),
    (PixelFormat::BGR24, Colorspace::SRGB, SPA_VIDEO_FORMAT_BGR),
    (
        PixelFormat::YV12,
        Colorspace::BT709_LIMITED,
        SPA_VIDEO_FORMAT_YV12,
    ),
    (
        PixelFormat::IYUV,
        Colorspace::BT709_LIMITED,
        SPA_VIDEO_FORMAT_I420,
    ),
    (
        PixelFormat::YUY2,
        Colorspace::BT709_LIMITED,
        SPA_VIDEO_FORMAT_YUY2,
    ),
    (
        PixelFormat::UYVY,
        Colorspace::BT709_LIMITED,
        SPA_VIDEO_FORMAT_UYVY,
    ),
    (
        PixelFormat::YVYU,
        Colorspace::BT709_LIMITED,
        SPA_VIDEO_FORMAT_YVYU,
    ),
    (
        PixelFormat::NV12,
        Colorspace::BT709_LIMITED,
        SPA_VIDEO_FORMAT_NV12,
    ),
    (
        PixelFormat::NV21,
        Colorspace::BT709_LIMITED,
        SPA_VIDEO_FORMAT_NV21,
    ),
];

/// Translation of `sdl_format_to_id()`.
fn sdl_format_to_id(format: PixelFormat) -> u32 {
    SDL_VIDEO_FORMATS
        .iter()
        .find(|f| f.0 == format)
        .map_or(SPA_VIDEO_FORMAT_UNKNOWN, |f| f.2)
}

/// Translation of `id_to_sdl_format()`.
fn id_to_sdl_format(id: u32) -> (PixelFormat, Colorspace) {
    SDL_VIDEO_FORMATS
        .iter()
        .find(|f| f.2 == id)
        .map_or((PixelFormat::UNKNOWN, Colorspace::UNKNOWN), |f| (f.0, f.1))
}

/// The pairs of 32-bit words (rectangles, fractions) of a values pod.
fn word_pairs(values: Pod<'_>, n_vals: u32) -> impl Iterator<Item = (u32, u32)> + '_ {
    // Note (upstream): the C code reads `n_vals` values whatever the child
    // size says, past the pod if it's short; here the reading stops at its end.
    values
        .body()
        .chunks_exact(8)
        .take(n_vals as usize)
        .map(|c| {
            (
                u32::from_ne_bytes([c[0], c[1], c[2], c[3]]),
                u32::from_ne_bytes([c[4], c[5], c[6], c[7]]),
            )
        })
}

/// The 32-bit ids of a values pod.
fn ids(values: Pod<'_>, n_vals: u32) -> impl Iterator<Item = u32> + '_ {
    // Note (upstream): as in word_pairs().
    values
        .body()
        .chunks_exact(4)
        .take(n_vals as usize)
        .map(|c| u32::from_ne_bytes([c[0], c[1], c[2], c[3]]))
}

/// The values of a property of `param`, if it has one of type `type_`
/// (the steps every `collect_*()` starts with): the values, their count
/// (1 for a plain value) and the choice.
fn prop_values(param: Pod<'_>, key: u32, type_: u32) -> Option<(Pod<'_>, u32, u32)> {
    let prop = spa_pod_find_prop(param, None, key)?;
    let (values, mut n_vals, choice) = spa_pod_get_values(prop.value);
    if values.type_() != type_ || n_vals == 0 {
        return None;
    }
    if choice == SPA_CHOICE_NONE {
        n_vals = 1;
    }
    Some((values, n_vals, choice))
}

/// Translation of `collect_rates()`.
fn collect_rates(
    data: &mut Vec<CameraSpec>,
    p: Pod<'_>,
    sdlfmt: PixelFormat,
    colorspace: Colorspace,
    size: (u32, u32),
) {
    let Some((values, n_vals, choice)) =
        prop_values(p, SPA_FORMAT_VIDEO_FRAMERATE, SPA_TYPE_FRACTION)
    else {
        return;
    };

    match choice {
        SPA_CHOICE_NONE | SPA_CHOICE_ENUM => {
            for (num, denom) in word_pairs(values, n_vals) {
                super::add_camera_format(
                    data,
                    sdlfmt,
                    colorspace,
                    size.0 as i32,
                    size.1 as i32,
                    num as i32,
                    denom as i32,
                );
            }
        }
        _ => crate::log!("CAMERA: unimplemented choice:{}", choice),
    }
}

/// Translation of `collect_size()`.
fn collect_size(
    data: &mut Vec<CameraSpec>,
    p: Pod<'_>,
    sdlfmt: PixelFormat,
    colorspace: Colorspace,
) {
    let Some((values, n_vals, choice)) = prop_values(p, SPA_FORMAT_VIDEO_SIZE, SPA_TYPE_RECTANGLE)
    else {
        return;
    };

    match choice {
        SPA_CHOICE_NONE | SPA_CHOICE_ENUM => {
            for rectangle in word_pairs(values, n_vals) {
                collect_rates(data, p, sdlfmt, colorspace, rectangle);
            }
        }
        _ => crate::log!("CAMERA: unimplemented choice:{}", choice),
    }
}

/// Translation of `collect_raw()`.
fn collect_raw(data: &mut Vec<CameraSpec>, p: Pod<'_>) {
    let Some((values, n_vals, choice)) = prop_values(p, SPA_FORMAT_VIDEO_FORMAT, SPA_TYPE_ID)
    else {
        return;
    };

    match choice {
        SPA_CHOICE_NONE | SPA_CHOICE_ENUM => {
            for id in ids(values, n_vals) {
                let (sdlfmt, colorspace) = id_to_sdl_format(id);
                if sdlfmt == PixelFormat::UNKNOWN {
                    continue;
                }
                collect_size(data, p, sdlfmt, colorspace);
            }
        }
        _ => crate::log!("CAMERA: unimplemented choice: {}", choice),
    }
}

/// Translation of `collect_format()`: the specs an `EnumFormat` param
/// offers.
fn collect_format(data: &mut Vec<CameraSpec>, p: Pod<'_>) {
    let Some((values, n_vals, choice)) = prop_values(p, SPA_FORMAT_MEDIA_SUBTYPE, SPA_TYPE_ID)
    else {
        return;
    };

    match choice {
        SPA_CHOICE_NONE | SPA_CHOICE_ENUM => {
            for id in ids(values, n_vals) {
                match id {
                    SPA_MEDIA_SUBTYPE_RAW => collect_raw(data, p),
                    SPA_MEDIA_SUBTYPE_MJPG => {
                        collect_size(data, p, PixelFormat::MJPG, Colorspace::JPEG)
                    }
                    _ => {} // Unsupported format
                }
            }
        }
        _ => crate::log!("CAMERA: unimplemented choice: {}", choice),
    }
}

/// The `EnumFormat` param `PIPEWIRECAMERA_OpenDevice()` connects its stream
/// with, in `buffer`; `None` if it doesn't fit.
fn build_enum_format(buffer: &mut [u8], spec: &CameraSpec) -> Option<std::ops::Range<usize>> {
    let mut b = SpaPodBuilder::new(buffer);
    // spa_pod_builder_add_object(&b, SPA_TYPE_OBJECT_Format, SPA_PARAM_EnumFormat, ...)
    b.push_object(SPA_TYPE_OBJECT_FORMAT, SPA_PARAM_ENUM_FORMAT);
    b.prop(SPA_FORMAT_MEDIA_TYPE, 0);
    b.id(SPA_MEDIA_TYPE_VIDEO);
    b.prop(SPA_FORMAT_MEDIA_SUBTYPE, 0);
    if spec.format == PixelFormat::MJPG {
        b.id(SPA_MEDIA_SUBTYPE_MJPG);
    } else {
        b.id(SPA_MEDIA_SUBTYPE_RAW);
        b.prop(SPA_FORMAT_VIDEO_FORMAT, 0);
        b.id(sdl_format_to_id(spec.format));
    }
    b.prop(SPA_FORMAT_VIDEO_SIZE, 0);
    b.rectangle(spec.width as u32, spec.height as u32);
    b.prop(SPA_FORMAT_VIDEO_FRAMERATE, 0);
    b.fraction(
        spec.framerate_numerator as u32,
        spec.framerate_denominator as u32,
    );
    let range = b.pop()?;
    Some(range)
}

// --- the hotplug loop ---

/// A registry global SDL follows: a `Video/Source` node. Translation of
/// `struct global` (only the node class exists, so it's built in).
struct Global {
    hotplug: *const Hotplug,

    id: u32,
    #[allow(dead_code)] // (unused, as upstream)
    permissions: u32,
    /// FIXME (upstream): never freed (nor used).
    #[allow(dead_code)]
    props: *mut PwProperties,

    name: String,

    proxy: *mut PwProxy,
    proxy_listener: UnsafeCell<SpaHook>,
    object_listener: UnsafeCell<SpaHook>,

    data: Mutex<GlobalData>,
}

/// The parts of a `Global` its callbacks change.
struct GlobalData {
    changed: i32,
    /// The node's info, merged (`pw_node_info_merge()`); libpipewire's.
    info: *mut PwNodeInfo,
    pending_list: Vec<Param>,
    param_list: Vec<Param>,

    added: bool,
}

impl Global {
    fn data(&self) -> MutexGuard<'_, GlobalData> {
        self.data.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn hotplug(&self) -> &Hotplug {
        // SAFETY: the hotplug object outlives its globals (they're
        // destroyed with the core, in hotplug_loop_destroy()).
        unsafe { &*self.hotplug }
    }
}

/// The hotplug state behind the loop's lock (and this mutex here).
#[derive(Default)]
struct HotplugState {
    server_major: i32,
    server_minor: i32,
    server_patch: i32,
    #[allow(dead_code)] // (unused, as upstream)
    last_seq: c_int,
    pending_seq: c_int,

    global_list: Vec<*mut Global>,

    have_1_0_5: bool,
    init_complete: bool,
    events_enabled: bool,
}

/// The global hotplug thread and associated objects. Translation of the
/// `hotplug` static.
struct Hotplug {
    lib: Arc<PwLib>,
    loop_: *mut PwThreadLoop,
    context: *mut PwContext,
    core: *mut PwCore,
    core_listener: UnsafeCell<SpaHook>,
    registry: *mut PwRegistry,
    registry_listener: UnsafeCell<SpaHook>,
    state: Mutex<HotplugState>,
}

// SAFETY: the PipeWire objects are used from the hotplug loop's thread and,
// under the loop's lock, from other threads, as upstream does; the hooks
// are only touched by libpipewire (and spa_hook_remove()) under that lock;
// the globals are only reached under it too; the Rust state is behind
// mutexes.
unsafe impl Send for Hotplug {}
// SAFETY: as above.
unsafe impl Sync for Hotplug {}

impl Hotplug {
    fn state(&self) -> MutexGuard<'_, HotplugState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn lock(&self) {
        // SAFETY: the loop is alive until hotplug_loop_destroy().
        unsafe { (self.lib.pw_thread_loop_lock)(self.loop_) }
    }

    fn unlock(&self) {
        // SAFETY: as above; we hold the lock.
        unsafe { (self.lib.pw_thread_loop_unlock)(self.loop_) }
    }

    fn wait(&self) {
        // SAFETY: as above; we hold the lock.
        unsafe { (self.lib.pw_thread_loop_wait)(self.loop_) }
    }

    fn signal(&self) {
        // SAFETY: the loop is alive.
        unsafe { (self.lib.pw_thread_loop_signal)(self.loop_, false) }
    }

    fn userdata(&self) -> *mut c_void {
        ptr::from_ref(self).cast_mut().cast()
    }

    /// Translation of `do_resync()`.
    fn do_resync(&self) {
        // SAFETY: the core is live.
        let seq = unsafe { pw_core_sync(self.core, PW_ID_CORE, 0) };
        self.state().pending_seq = seq;
    }

    /// When in a container, the library version can differ from the
    /// underlying core version, so make sure the underlying Pipewire
    /// implementation meets the version requirement. Translation of
    /// `pipewire_server_version_at_least()`.
    fn pipewire_server_version_at_least(&self, major: i32, minor: i32, patch: i32) -> bool {
        let s = self.state();
        (s.server_major >= major)
            && (s.server_major > major || s.server_minor >= minor)
            && (s.server_major > major || s.server_minor > minor || s.server_patch >= patch)
    }

    /// Translation of `PIPEWIRECAMERA_Deinitialize()`'s teardown of the
    /// hotplug objects.
    fn hotplug_loop_destroy(&self) {
        let lib = &self.lib;
        // SAFETY (the teardown below): each object is live and released once,
        // in upstream's order, with the loop locked; destroying the core
        // destroys the globals' proxies (proxy_destroy() frees them).
        unsafe {
            if !self.loop_.is_null() {
                (lib.pw_thread_loop_lock)(self.loop_);
            }
            if !self.registry.is_null() {
                spa_hook_remove(self.registry_listener.get());
                (lib.pw_proxy_destroy)(self.registry.cast());
            }
            if !self.core.is_null() {
                spa_hook_remove(self.core_listener.get());
                (lib.pw_core_disconnect)(self.core);
            }
            if !self.context.is_null() {
                (lib.pw_context_destroy)(self.context);
            }
            if !self.loop_.is_null() {
                (lib.pw_thread_loop_unlock)(self.loop_);
                (lib.pw_thread_loop_destroy)(self.loop_);
            }
        }
        *self.state() = HotplugState::default();
    }
}

/// Translation of `add_device()`.
fn add_device(g: &Global) {
    let mut d = g.data();
    let mut data = Vec::new();

    for p in &d.param_list {
        if p.id != SPA_PARAM_ENUM_FORMAT {
            continue;
        }
        if let Some(pod) = p.pod() {
            collect_format(&mut data, pod);
        }
    }
    if !data.is_empty() {
        add_camera(&g.name, CameraPosition::Unknown, &data, Box::new(g.id));
    }

    d.added = true;
}

// node

/// Translation of `node_event_info()`; the userdata is the global.
unsafe extern "C" fn node_event_info(object: *mut c_void, info: *const PwNodeInfo) {
    // SAFETY: the userdata is a live global (its hooks are removed before
    // it's freed); `info` is valid for this call.
    let g = unsafe { &*object.cast::<Global>() };
    let hotplug = g.hotplug();
    let mut d = g.data();

    // SAFETY: g.info is NULL or our merged info; `info` is valid.
    let merged = unsafe { (hotplug.lib.pw_node_info_merge)(d.info, info, d.changed == 0) };
    d.info = merged;
    if merged.is_null() {
        return;
    }
    // SAFETY: libpipewire's merged info, which we own until node_destroy();
    // its params are `n_params` entries.
    let info = unsafe { &mut *merged };

    if info.change_mask & PW_NODE_CHANGE_MASK_PARAMS != 0 && !info.params.is_null() {
        // SAFETY: as above.
        let params = unsafe { std::slice::from_raw_parts_mut(info.params, info.n_params as usize) };
        for param in params {
            let id = param.id;

            if param.user == 0 {
                continue;
            }
            param.user = 0;

            if id != SPA_PARAM_ENUM_FORMAT {
                continue;
            }

            param_add(&mut d.pending_list, param.seq, id, None);
            if param.flags & SPA_PARAM_INFO_READ == 0 {
                continue;
            }

            param.seq = param.seq.wrapping_add(1);
            // SAFETY: the proxy is a live node proxy.
            let res =
                unsafe { pw_node_enum_params(g.proxy, param.seq, id, 0, u32::MAX, ptr::null()) };
            if spa_result_is_async(res) {
                param.seq = res;
            }

            d.changed += 1;
        }
    }
    drop(d);
    hotplug.do_resync();
}

/// Translation of `node_event_param()`; the userdata is the global.
unsafe extern "C" fn node_event_param(
    object: *mut c_void,
    seq: c_int,
    id: u32,
    _index: u32,
    _next: u32,
    param: *const u8,
) {
    // SAFETY: the userdata is a live global; `param` is NULL or a valid pod
    // for this call.
    let (g, param) = unsafe { (&*object.cast::<Global>(), Pod::from_ptr(param)) };
    param_add(&mut g.data().pending_list, seq, id, param);
}

static NODE_EVENTS: PwNodeEvents = PwNodeEvents {
    version: PW_VERSION_NODE_EVENTS,
    info: Some(node_event_info),
    param: Some(node_event_param),
};

/// Translation of `node_destroy()`.
fn node_destroy(g: &Global) {
    let mut d = g.data();
    if !d.info.is_null() {
        // SAFETY: our merged info, freed once.
        unsafe { (g.hotplug().lib.pw_node_info_free)(d.info) };
        d.info = ptr::null_mut();
    }
}

// proxy

/// Translation of `proxy_removed()`; the userdata is the global.
unsafe extern "C" fn proxy_removed(data: *mut c_void) {
    // SAFETY: the userdata is a live global; this destroys its proxy (and,
    // through proxy_destroy(), the global), which isn't used after.
    unsafe {
        let g = &*data.cast::<Global>();
        (g.hotplug().lib.pw_proxy_destroy)(g.proxy);
    }
}

/// Translation of `proxy_destroy()`; the userdata is the global, which is
/// freed here (upstream's lives in the proxy's user data).
unsafe extern "C" fn proxy_destroy(data: *mut c_void) {
    let g = data.cast::<Global>();
    // SAFETY: the userdata is a live global, boxed by
    // hotplug_registry_global_callback().
    let gr = unsafe { &*g };
    gr.hotplug().state().global_list.retain(|&x| x != g);
    // SAFETY: the hooks were added to this proxy, which is still alive
    // during its destroy event; removing a hook from its own callback is
    // allowed.
    unsafe {
        spa_hook_remove(gr.object_listener.get());
    }
    node_destroy(gr);
    {
        let mut d = gr.data();
        param_clear(&mut d.param_list, SPA_ID_INVALID);
        param_clear(&mut d.pending_list, SPA_ID_INVALID);
    }
    // (the name goes with the box; the proxy listener must be unlinked
    // before its memory is freed, which upstream's isn't)
    // SAFETY: as above; nothing refers to the global after this.
    unsafe {
        spa_hook_remove(gr.proxy_listener.get());
        drop(Box::from_raw(g));
    }
}

static PROXY_EVENTS: PwProxyEvents = PwProxyEvents {
    version: PW_VERSION_PROXY_EVENTS,
    destroy: Some(proxy_destroy),
    bound: None,
    removed: Some(proxy_removed),
    done: None,
    error: None,
    bound_props: None,
};

/// Translation of `hotplug_registry_global_callback()`; called with
/// thread_loop lock; the userdata is the `Hotplug`.
unsafe extern "C" fn hotplug_registry_global_callback(
    object: *mut c_void,
    id: u32,
    permissions: u32,
    type_: *const c_char,
    _version: u32,
    props: *const SpaDict,
) {
    // SAFETY: the userdata is the live `Hotplug`; `type_` is a C string.
    let (hotplug, type_str) = unsafe { (&*object.cast::<Hotplug>(), c_str(type_).unwrap_or("")) };

    if type_str != PW_TYPE_INTERFACE_NODE {
        return;
    }
    if props.is_null() {
        return;
    }
    // SAFETY: `props` is a valid dictionary for this call.
    let is_video_source = unsafe { spa_dict_lookup(props, PW_KEY_MEDIA_CLASS) }
        .is_some_and(|s| s.to_bytes() == b"Video/Source");
    if !is_video_source {
        return;
    }

    // SAFETY: as above.
    let name = unsafe {
        spa_dict_lookup(props, "node.description").or_else(|| spa_dict_lookup(props, "node.name"))
    }
    .map_or_else(
        || "unnamed camera".to_owned(),
        |s| s.to_string_lossy().into_owned(),
    );

    // SAFETY: the registry is live; `type_` is libpipewire's C string.
    // (the global lives in its own Box rather than the proxy's user data)
    let proxy = unsafe { pw_registry_bind(hotplug.registry, id, type_, PW_VERSION_NODE, 0) };
    if proxy.is_null() {
        // Note (upstream): the C code uses the proxy's user data unchecked.
        return;
    }

    let lib = &hotplug.lib;
    // SAFETY: `props` is a valid dictionary.
    let g_props = unsafe { (lib.pw_properties_new_dict)(props) };
    let g = Box::into_raw(Box::new(Global {
        hotplug: ptr::from_ref(hotplug),
        id,
        permissions,
        props: g_props,
        name,
        proxy,
        proxy_listener: SpaHook::zeroed(),
        object_listener: SpaHook::zeroed(),
        data: Mutex::new(GlobalData {
            changed: 0,
            info: ptr::null_mut(),
            pending_list: Vec::new(),
            param_list: Vec::new(),
            added: false,
        }),
    }));
    hotplug.state().global_list.push(g);

    // SAFETY: the proxy is live; the hooks are zeroed and boxed with the
    // global, which outlives them (proxy_destroy() removes them before
    // freeing it); the events are static.
    unsafe {
        (lib.pw_proxy_add_listener)(proxy, (*g).proxy_listener.get(), &PROXY_EVENTS, g.cast());
        (lib.pw_proxy_add_object_listener)(
            proxy,
            (*g).object_listener.get(),
            ptr::from_ref(&NODE_EVENTS).cast(),
            g.cast(),
        );
    }

    hotplug.do_resync();
}

/// Translation of `hotplug_registry_global_remove_callback()`; called with
/// thread_loop lock.
///
/// FIXME (upstream): this does nothing, so a camera that goes away is
/// never reported as disconnected (its global is freed when the server
/// removes the proxy; the SDL device keeps only its id here).
unsafe extern "C" fn hotplug_registry_global_remove_callback(_object: *mut c_void, _id: u32) {}

static HOTPLUG_REGISTRY_EVENTS: PwRegistryEvents = PwRegistryEvents {
    version: PW_VERSION_REGISTRY_EVENTS,
    global: Some(hotplug_registry_global_callback),
    global_remove: Some(hotplug_registry_global_remove_callback),
};

/// Translation of `parse_version()`.
fn parse_version(s: &str) -> (i32, i32, i32) {
    let mut v = [0; 3];
    if sscanf_version(s, &mut v) < 3 {
        return (0, 0, 0);
    }
    (v[0], v[1], v[2])
}

/// Core info, called with thread_loop lock. Translation of
/// `hotplug_core_info_callback()`; the userdata is the `Hotplug`.
unsafe extern "C" fn hotplug_core_info_callback(data: *mut c_void, info: *const PwCoreInfo) {
    // SAFETY: the userdata is the live `Hotplug`; `info` is valid for this call.
    let (hotplug, version) = unsafe { (&*data.cast::<Hotplug>(), c_str((*info).version)) };
    let (major, minor, patch) = parse_version(version.unwrap_or(""));
    let mut s = hotplug.state();
    s.server_major = major;
    s.server_minor = minor;
    s.server_patch = patch;
}

/// Core sync points, called with thread_loop lock. Translation of
/// `hotplug_core_done_callback()`; the userdata is the `Hotplug`.
unsafe extern "C" fn hotplug_core_done_callback(object: *mut c_void, id: u32, seq: c_int) {
    // SAFETY: the userdata is the live `Hotplug`.
    let hotplug = unsafe { &*object.cast::<Hotplug>() };
    let (globals, events_enabled) = {
        let mut s = hotplug.state();
        s.last_seq = seq;
        if id != PW_ID_CORE || seq != s.pending_seq {
            return;
        }
        (s.global_list.clone(), s.events_enabled)
    };

    for g in globals {
        // SAFETY: the globals in the list are alive (we're on the loop
        // thread, which is the only one that frees them while it runs).
        let g = unsafe { &*g };
        let added = {
            let mut d = g.data();
            if d.changed == 0 {
                continue;
            }

            let info = d.info;
            let d = &mut *d;
            // SAFETY: a changed global has its merged info, whose params are
            // `n_params` entries.
            let params = unsafe {
                if info.is_null() || (*info).params.is_null() {
                    &[][..]
                } else {
                    std::slice::from_raw_parts((*info).params, (*info).n_params as usize)
                }
            };
            param_update(&mut d.param_list, &mut d.pending_list, params);
            d.added
        };

        if !added && events_enabled {
            add_device(g);
        }
    }
    hotplug.state().init_complete = true;
    hotplug.signal();
}

static HOTPLUG_CORE_EVENTS: PwCoreEvents = PwCoreEvents {
    version: PW_VERSION_CORE_EVENTS,
    info: Some(hotplug_core_info_callback),
    done: Some(hotplug_core_done_callback),
    ping: None,
    error: None,
    remove_id: None,
    bound_id: None,
    add_mem: None,
    remove_mem: None,
    bound_props: None,
};

/// The hotplug thread. Translation of `hotplug_loop_init()`; on failure,
/// what was set up is torn down again (upstream's caller does that with
/// PIPEWIRECAMERA_Deinitialize()).
fn hotplug_loop_init(lib: &Arc<PwLib>) -> Result<Arc<Hotplug>> {
    let fd = match camera_portal_request_access() {
        CameraPortalAccess::Error => {
            // (upstream returns without setting an error of its own)
            return Err(Error::new("Pipewire: camera portal access failed"));
        }
        CameraPortalAccess::Denied => None,
        CameraPortalAccess::Granted(fd) => Some(fd),
    };

    // SAFETY: pw_check_library_version() has no preconditions.
    let have_1_0_5 = unsafe { (lib.pw_check_library_version)(1, 0, 5) };

    let mut hotplug = Hotplug {
        lib: lib.clone(),
        loop_: ptr::null_mut(),
        context: ptr::null_mut(),
        core: ptr::null_mut(),
        core_listener: SpaHook::zeroed(),
        registry: ptr::null_mut(),
        registry_listener: SpaHook::zeroed(),
        state: Mutex::new(HotplugState {
            have_1_0_5,
            ..HotplugState::default()
        }),
    };

    // (the objects are made first, then the shared object their callbacks
    // get as userdata, so it never moves)
    let made = (|| {
        // SAFETY: the name is NUL-terminated; NULL props are allowed.
        hotplug.loop_ =
            unsafe { (lib.pw_thread_loop_new)(c"SDLPwCameraPlug".as_ptr(), ptr::null()) };
        if hotplug.loop_.is_null() {
            return Err(Error::new(format!(
                "Pipewire: Failed to create hotplug detection loop ({})",
                errno()
            )));
        }

        // SAFETY: the loop is live; NULL props are allowed.
        hotplug.context = unsafe {
            (lib.pw_context_new)(
                (lib.pw_thread_loop_get_loop)(hotplug.loop_),
                ptr::null_mut(),
                0,
            )
        };
        if hotplug.context.is_null() {
            return Err(Error::new(format!(
                "Pipewire: Failed to create hotplug detection context ({})",
                errno()
            )));
        }
        hotplug.core = match fd {
            // SAFETY: the context is live; libpipewire takes ownership of
            // the descriptor.
            Some(fd) => unsafe {
                (lib.pw_context_connect_fd)(hotplug.context, fd.into_raw_fd(), ptr::null_mut(), 0)
            },
            // SAFETY: the context is live; NULL props are allowed.
            None => unsafe { (lib.pw_context_connect)(hotplug.context, ptr::null_mut(), 0) },
        };
        if hotplug.core.is_null() {
            return Err(Error::new(format!(
                "Pipewire: Failed to connect hotplug detection context ({})",
                errno()
            )));
        }

        // SAFETY: the core is live.
        hotplug.registry = unsafe { pw_core_get_registry(hotplug.core, PW_VERSION_REGISTRY, 0) };
        if hotplug.registry.is_null() {
            return Err(Error::new(format!(
                "Pipewire: Failed to acquire hotplug detection registry ({})",
                errno()
            )));
        }
        Ok(())
    })();
    let hotplug = Arc::new(hotplug);
    if let Err(e) = made {
        hotplug.hotplug_loop_destroy();
        return Err(e);
    }

    let data = hotplug.userdata();
    // SAFETY: the core and registry are live; the hooks are zeroed and live
    // in the shared object, which outlives them (hotplug_loop_destroy()
    // removes them); the events are static.
    unsafe {
        pw_core_add_listener(
            hotplug.core,
            hotplug.core_listener.get(),
            &HOTPLUG_CORE_EVENTS,
            data,
        );
        pw_registry_add_listener(
            hotplug.registry,
            hotplug.registry_listener.get(),
            &HOTPLUG_REGISTRY_EVENTS,
            data,
        );
    }

    hotplug.do_resync();

    // SAFETY: the loop is live.
    let res = unsafe { (lib.pw_thread_loop_start)(hotplug.loop_) };
    if res != 0 {
        hotplug.hotplug_loop_destroy();
        return Err(Error::new(
            "Pipewire: Failed to start hotplug detection loop",
        ));
    }

    hotplug.lock();
    while !hotplug.state().init_complete {
        hotplug.wait();
    }
    hotplug.unlock();

    if !hotplug.pipewire_server_version_at_least(
        PW_REQUIRED_MAJOR,
        PW_REQUIRED_MINOR,
        PW_REQUIRED_PATCH,
    ) {
        let (major, minor, patch) = {
            let s = hotplug.state();
            (s.server_major, s.server_minor, s.server_patch)
        };
        hotplug.hotplug_loop_destroy();
        return Err(Error::new(format!(
            "Pipewire: server version is too old {major}.{minor}.{patch} < {PW_REQUIRED_MAJOR}.{PW_REQUIRED_MINOR}.{PW_REQUIRED_PATCH}"
        )));
    }

    Ok(hotplug)
}

// --- the devices ---

/// The parts of an opened device behind its mutex.
struct StreamState {
    stream: *mut PwStream,
    /// The stream's buffers (`struct pw_array buffers`).
    buffers: Vec<*mut PwBuffer>,
    /// The buffers whose data the app holds a copy of, by the copy's
    /// address; they go back to the stream when the frame is released.
    lent: Vec<(usize, *mut PwBuffer)>,
    /// Frames given back, to copy the next ones into.
    spare: Vec<Vec<u8>>,
}

/// An opened device. Translation of `struct SDL_PrivateCameraData`; the
/// stream callbacks get it as their userdata.
struct PwCamera {
    hotplug: Arc<Hotplug>,
    device: Weak<CameraDevice>,
    stream_listener: UnsafeCell<SpaHook>,
    state: Mutex<StreamState>,
}

// SAFETY: the stream and its buffers are used under the hotplug loop's lock
// (as upstream does) and the mutex; the hook is only touched by
// libpipewire under the loop's lock.
unsafe impl Send for PwCamera {}
// SAFETY: as above.
unsafe impl Sync for PwCamera {}

impl PwCamera {
    fn state(&self) -> MutexGuard<'_, StreamState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Translation of `PIPEWIRECAMERA_CloseDevice()`'s work.
    fn close(&self) {
        self.hotplug.lock();
        // (the state is unlocked while the stream is destroyed: that calls
        // on_remove_buffer())
        let stream = std::mem::replace(&mut self.state().stream, ptr::null_mut());
        if !stream.is_null() {
            // SAFETY: our live stream, destroyed once, with the loop locked.
            unsafe { (self.hotplug.lib.pw_stream_destroy)(stream) };
        }
        let mut s = self.state();
        s.buffers.clear();
        s.lent.clear();
        drop(s);
        self.hotplug.unlock();
    }
}

/// The `PwCamera` of a stream callback's userdata.
///
/// # Safety
///
/// `data` must be the userdata given to pw_stream_add_listener(): a
/// `PwCamera` that outlives its stream.
unsafe fn pw_camera<'a>(data: *mut c_void) -> &'a PwCamera {
    // SAFETY: the caller's contract.
    unsafe { &*data.cast::<PwCamera>() }
}

/// Translation of `on_process()`.
unsafe extern "C" fn on_process(data: *mut c_void) {
    // SAFETY: the userdata is the live device.
    unsafe { pw_camera(data) }.hotplug.signal();
}

/// Translation of `on_stream_state_changed()`.
unsafe extern "C" fn on_stream_state_changed(
    data: *mut c_void,
    _old: c_int,
    state: c_int,
    _error: *const c_char,
) {
    // SAFETY: the userdata is the live device.
    let camera = unsafe { pw_camera(data) };
    if state == PW_STREAM_STATE_STREAMING {
        // FIXME (upstream): this runs on the hotplug loop thread with the
        // loop locked and takes the device lock, while the device thread
        // holds the device lock in AcquireFrame and waits for the loop lock;
        // a stream that starts streaming again then deadlocks.
        if let Some(device) = camera.device.upgrade() {
            camera_permission_outcome(&device, true);
        }
    }
}

/// Translation of `on_stream_param_changed()`.
unsafe extern "C" fn on_stream_param_changed(_data: *mut c_void, _id: u32, _param: *const u8) {}

/// Translation of `on_add_buffer()`.
unsafe extern "C" fn on_add_buffer(data: *mut c_void, buffer: *mut PwBuffer) {
    // SAFETY: the userdata is the live device.
    unsafe { pw_camera(data) }.state().buffers.push(buffer);
}

/// Translation of `on_remove_buffer()`.
unsafe extern "C" fn on_remove_buffer(data: *mut c_void, buffer: *mut PwBuffer) {
    // SAFETY: the userdata is the live device.
    let mut s = unsafe { pw_camera(data) }.state();
    if let Some(i) = s.buffers.iter().position(|&b| b == buffer) {
        s.buffers.remove(i);
    }
    s.lent.retain(|&(_, b)| b != buffer);
}

static STREAM_EVENTS: PwStreamEvents = PwStreamEvents {
    version: PW_VERSION_STREAM_EVENTS,
    destroy: None,
    state_changed: Some(on_stream_state_changed),
    control_info: None,
    io_changed: None,
    param_changed: Some(on_stream_param_changed),
    add_buffer: Some(on_add_buffer),
    remove_buffer: Some(on_remove_buffer),
    process: Some(on_process),
    drained: None,
    command: None,
    trigger_done: None,
};

impl CameraBackend for PwCamera {
    fn wait_device(&self, _device: &CameraDevice) -> bool {
        // Translation of `PIPEWIRECAMERA_WaitDevice()`.
        // FIXME (upstream): nothing signals the loop when the device is
        // asked to shut down, so closing a camera whose stream never starts
        // (or stops) producing frames waits here forever.
        self.hotplug.lock();
        self.hotplug.wait();
        self.hotplug.unlock();
        true
    }

    fn acquire_frame(&self, device: &CameraDevice) -> CameraFrameResult {
        // Translation of `PIPEWIRECAMERA_AcquireFrame()`.
        let lib = &self.hotplug.lib;
        let format = device.actual_spec().format;
        self.hotplug.lock();
        let mut s = self.state();
        let stream = s.stream;
        let mut b: *mut PwBuffer = ptr::null_mut();
        if !stream.is_null() {
            loop {
                // SAFETY: our live stream, with the loop locked.
                let t = unsafe { (lib.pw_stream_dequeue_buffer)(stream) };
                if t.is_null() {
                    break;
                }
                if !b.is_null() {
                    // SAFETY: a buffer we dequeued, given back.
                    unsafe { (lib.pw_stream_queue_buffer)(stream, b) };
                }
                b = t;
            }
        }
        if b.is_null() {
            drop(s);
            self.hotplug.unlock();
            return CameraFrameResult::Skip;
        }

        let timestamp_ns = if self.hotplug.state().have_1_0_5 {
            // SAFETY: a dequeued buffer, valid until it's queued again.
            unsafe { (*b).time }
        } else {
            crate::timer::ticks_ns()
        };

        // SAFETY: a dequeued buffer has a spa_buffer with at least one data
        // block (the stream's buffers are made with one); with
        // PW_STREAM_FLAG_MAP_BUFFERS its data is mapped, `maxsize` bytes,
        // and its chunk is valid.
        let copied = unsafe {
            let buffer = (*b).buffer;
            if buffer.is_null() || (*buffer).n_datas == 0 || (*(*buffer).datas).data.is_null() {
                None
            } else {
                let d = &*(*buffer).datas;
                let chunk = &*d.chunk;
                let (pitch, len) = if format == PixelFormat::MJPG {
                    (chunk.size as i32, (chunk.size).min(d.maxsize) as usize)
                } else {
                    (chunk.stride, d.maxsize as usize)
                };
                let src = std::slice::from_raw_parts(d.data.cast::<u8>(), len);
                let mut pixels = s.spare.pop().unwrap_or_default();
                pixels.clear();
                pixels.extend_from_slice(src);
                Some((pixels, pitch))
            }
        };
        let Some((pixels, pitch)) = copied else {
            // FIXME (upstream): a buffer without mapped data becomes a frame
            // without pixels there; here it's given back and skipped.
            // SAFETY: a buffer we dequeued, given back.
            unsafe { (lib.pw_stream_queue_buffer)(stream, b) };
            drop(s);
            self.hotplug.unlock();
            return CameraFrameResult::Skip;
        };
        s.lent.push((pixels.as_ptr() as usize, b));

        drop(s);
        self.hotplug.unlock();

        CameraFrameResult::Ready(AcquiredFrame {
            pixels,
            pitch,
            timestamp_ns,
            rotation: 0.0,
        })
    }

    fn release_frame(&self, _device: &CameraDevice, pixels: Vec<u8>) {
        // Translation of `PIPEWIRECAMERA_ReleaseFrame()`.
        self.hotplug.lock();
        let mut s = self.state();
        let addr = pixels.as_ptr() as usize;
        if let Some(i) = s.lent.iter().position(|&(a, _)| a == addr) {
            let (_, b) = s.lent.remove(i);
            // (only a buffer the stream still has goes back, as upstream
            // looks the frame up in its buffer list)
            if !s.stream.is_null() && s.buffers.contains(&b) {
                // SAFETY: our live stream and one of its dequeued buffers.
                unsafe { (self.hotplug.lib.pw_stream_queue_buffer)(s.stream, b) };
            }
            s.spare.push(pixels);
        }
        drop(s);
        self.hotplug.unlock();
    }

    fn close_device(&self, _device: &CameraDevice) {
        // Translation of `PIPEWIRECAMERA_CloseDevice()`.
        self.close();
    }
}

/// The driver: the hotplug loop (`pipewire_initialized` is whether there
/// is one).
struct PipewireCamera {
    hotplug: Mutex<Option<Arc<Hotplug>>>,
}

impl PipewireCamera {
    fn hotplug(&self) -> Option<Arc<Hotplug>> {
        self.hotplug
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

impl CameraDriverImpl for PipewireCamera {
    fn detect_devices(&self) {
        // Translation of `PIPEWIRECAMERA_DetectDevices()`.
        let Some(hotplug) = self.hotplug() else {
            return;
        };
        hotplug.lock();

        // Wait until the initial registry enumeration is complete
        while !hotplug.state().init_complete {
            hotplug.wait();
        }

        let globals = hotplug.state().global_list.clone();
        for g in globals {
            // SAFETY: the globals are alive while we hold the loop's lock.
            let g = unsafe { &*g };
            let added = g.data().added;
            if !added {
                add_device(g);
            }
        }

        hotplug.state().events_enabled = true;

        hotplug.unlock();
    }

    fn open_device(
        &self,
        device: &Arc<CameraDevice>,
        spec: &CameraSpec,
    ) -> Result<Arc<dyn CameraBackend>> {
        // Translation of `PIPEWIRECAMERA_OpenDevice()`.
        let Some(hotplug) = self.hotplug() else {
            return Err(Error::new("Pipewire camera driver is not initialized"));
        };
        let lib = &hotplug.lib;
        let camera = Arc::new(PwCamera {
            hotplug: hotplug.clone(),
            device: Arc::downgrade(device),
            stream_listener: SpaHook::zeroed(),
            state: Mutex::new(StreamState {
                stream: ptr::null_mut(),
                buffers: Vec::with_capacity(64),
                lent: Vec::new(),
                spare: Vec::new(),
            }),
        });

        hotplug.lock();

        // FIXME (upstream): the failures below return with the thread loop
        // still locked, so the hotplug loop is stuck from then on.
        let target = CString::new(device.name.as_str()).unwrap_or_default();
        // FIXME (upstream): target.object is given the node's description
        // (SDL's device name), which PipeWire doesn't match against (it
        // wants a node name or serial), so the stream connects to the
        // default video source rather than the chosen one.
        // SAFETY: the keys and values are NUL-terminated; the list ends
        // with NULL.
        let props = unsafe {
            (lib.pw_properties_new)(
                PW_KEY_MEDIA_TYPE.as_ptr(),
                c"Video".as_ptr(),
                PW_KEY_MEDIA_CATEGORY.as_ptr(),
                c"Capture".as_ptr(),
                PW_KEY_MEDIA_ROLE.as_ptr(),
                c"Camera".as_ptr(),
                PW_KEY_TARGET_OBJECT.as_ptr(),
                target.as_ptr(),
                ptr::null::<c_char>(),
            )
        };
        if props.is_null() {
            return Err(Error::new("Pipewire: Failed to create stream properties"));
        }

        // SAFETY: the core is live; the stream takes the properties.
        let stream =
            unsafe { (lib.pw_stream_new)(hotplug.core, c"SDL PipeWire Camera".as_ptr(), props) };
        if stream.is_null() {
            return Err(Error::new("Pipewire: Failed to create stream"));
        }
        camera.state().stream = stream;

        // SAFETY: the stream is live; the hook is zeroed and lives in the
        // device, which outlives the stream (close() destroys it); the
        // events are static.
        unsafe {
            (lib.pw_stream_add_listener)(
                stream,
                camera.stream_listener.get(),
                &STREAM_EVENTS,
                Arc::as_ptr(&camera).cast_mut().cast(),
            );
        }

        let mut buffer = [0u64; 128]; // uint8_t buffer[1024] (pods are 8-aligned)
                                      // SAFETY: viewing the u64 buffer as its 1024 bytes.
        let bytes =
            unsafe { std::slice::from_raw_parts_mut(buffer.as_mut_ptr().cast::<u8>(), 1024) };
        let Some(range) = build_enum_format(bytes, spec) else {
            // (spa_pod_builder_add_object() gives NULL, and the connection
            // fails; the stream is destroyed as below)
            camera.close();
            return Err(Error::new("Pipewire: Failed to build the stream format"));
        };
        let mut params = [bytes[range].as_ptr()];

        // SAFETY: our live stream; the param is a valid pod in `buffer`,
        // which libpipewire copies.
        let res = unsafe {
            (lib.pw_stream_connect)(
                stream,
                PW_DIRECTION_INPUT,
                PW_ID_ANY,
                PW_STREAM_FLAG_AUTOCONNECT | PW_STREAM_FLAG_MAP_BUFFERS,
                params.as_mut_ptr(),
                params.len() as u32,
            )
        };
        if res < 0 {
            // (the stream's callbacks point at `camera`, which is freed
            // with this error: destroy the stream, as SDL_OpenCamera()'s
            // ClosePhysicalCamera() does upstream; its lock and unlock
            // leave the loop locked, as there)
            camera.close();
            return Err(Error::new(format!(
                "Pipewire: Failed to connect stream ({res})"
            )));
        }

        hotplug.unlock();

        Ok(camera)
    }

    fn free_device_handle(&self, _device: &CameraDevice) {
        // Translation of `PIPEWIRECAMERA_FreeDeviceHandle()`.
    }

    fn deinitialize(&self) {
        // Translation of `PIPEWIRECAMERA_Deinitialize()`.
        let hotplug = self
            .hotplug
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        if let Some(hotplug) = hotplug {
            hotplug.hotplug_loop_destroy();
            deinit_pipewire_library(&hotplug.lib);
        }
    }
}

/// Translation of `PIPEWIRECAMERA_Init()`.
fn pipewirecamera_init() -> Result<Arc<dyn CameraDriverImpl>> {
    let lib = init_pipewire_library()?;
    match hotplug_loop_init(&lib) {
        Ok(hotplug) => Ok(Arc::new(PipewireCamera {
            hotplug: Mutex::new(Some(hotplug)),
        })),
        Err(e) => {
            // PIPEWIRECAMERA_Deinitialize() (the loop's objects are gone)
            deinit_pipewire_library(&lib);
            Err(e)
        }
    }
}

/// Translation of `PIPEWIRECAMERA_bootstrap`.
pub(super) static PIPEWIRECAMERA_BOOTSTRAP: CameraBootStrap = CameraBootStrap {
    name: "pipewire",
    desc: "SDL PipeWire camera driver",
    init: pipewirecamera_init,
    demand_only: false,
};

#[cfg(test)]
mod tests;
