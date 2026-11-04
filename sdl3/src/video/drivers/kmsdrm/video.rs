// Rust translation of src/video/kmsdrm/SDL_kmsdrmvideo.c and
// SDL_kmsdrmvideo.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

// Atomic KMSDRM backend originally written by Manuel Alfayate Corchete <redwindwanderer@gmail.com>

//! The KMS/DRM driver's device, displays and windows: the card
//! (`/dev/dri/cardN`: the first with a connected connector and DRM master,
//! or `SDL_KMSDRM_DEVICE_INDEX`), a display per connected connector (its
//! encoder and CRTC, its modes, VRR and panel orientation), the GBM device
//! and the primary plane (set up with the first window), the windows'
//! GBM and EGL surfaces (all windows are fullscreen: the mode is set on the
//! next swap), the framebuffers of their buffer objects, page flips, the
//! atomic requests, and the VT switches (surfaces destroyed and DRM master
//! dropped on release, both back on acquire).
//!
//! Upstream's `SDL_VideoData`, `SDL_DisplayData` and `SDL_WindowData` are
//! kept together in the driver ([`State`]), by display and window ID,
//! rather than in the video core's records: the driver's lock is never
//! held while it calls back into SDL (events, the display lists, the
//! mouse), which may call the driver again.
//!
//! Only Linux's DRM device names are translated (`/dev/dri/cardN`; the
//! OpenBSD `/dev/drmN` names before 6.9 are not), and the console input is
//! the evdev reader's (`SDL_INPUT_LINUXEV`; not wscons). Vulkan is not
//! translated yet: the driver behaves as upstream built without
//! `SDL_VIDEO_VULKAN` (no Vulkan entry points; the `vulkan_mode` paths are
//! kept but never taken).

use std::ffi::{c_int, c_void, CStr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};

use super::kmsdrmdyn::{self, KmsdrmSyms};
use super::sys::*;
use crate::error::{Error, Result};
use crate::events::window::{send_window_event, WindowFlags};
use crate::events::{DisplayID, EventType, WindowID};
use crate::hints;
use crate::video::core::with_window;
use crate::video::display::{
    self, add_fullscreen_display_mode, display_bounds, displays, finalize_display_mode,
};
use crate::video::egl::{self, EGLSurface, EGLSyncKHR, EGL_NO_SURFACE};
use crate::video::gl;
use crate::video::sysvideo::{DisplayMode, VideoDisplay};
use crate::video::window::display_for_window;
use crate::video::PixelFormat;

/// `kmsdrm_dri_path` (`moderndri`: Linux's).
pub(crate) const KMSDRM_DRI_PATH: &str = "/dev/dri/";
/// `kmsdrm_dri_devname`.
pub(crate) const KMSDRM_DRI_DEVNAME: &str = "card";
/// `kmsdrm_dri_cardpath`.
pub(crate) const KMSDRM_DRI_CARDPATH: &str = "/dev/dri/card";

/// `EGL_CONDITION_SATISFIED_KHR` (EGL_KHR_fence_sync).
const EGL_CONDITION_SATISFIED_KHR: i32 = 0x30F6;
/// `EGL_FOREVER_KHR` (EGL_KHR_fence_sync).
const EGL_FOREVER_KHR: u64 = 0xFFFFFFFFFFFFFFFF;

/// Translation of `KMSDRM_plane`.
pub(crate) struct KmsdrmPlane {
    pub(crate) plane: *mut drmModePlane,
    pub(crate) props: *mut drmModeObjectProperties,
    pub(crate) props_info: Vec<*mut drmModePropertyRes>,
}

/// Translation of `KMSDRM_crtc`.
pub(crate) struct KmsdrmCrtc {
    pub(crate) crtc: *mut drmModeCrtc,
    pub(crate) props: *mut drmModeObjectProperties,
    pub(crate) props_info: Vec<*mut drmModePropertyRes>,
}

/// Translation of `KMSDRM_connector`.
pub(crate) struct KmsdrmConnector {
    pub(crate) connector: *mut drmModeConnector,
    pub(crate) props: *mut drmModeObjectProperties,
    pub(crate) props_info: Vec<*mut drmModePropertyRes>,
}

/// The properties of an object (its `props` and `props_info`), for
/// `add_*_property()`.
trait PropObject {
    fn props(&self) -> *mut drmModeObjectProperties;
    fn props_info(&self) -> &[*mut drmModePropertyRes];
}

macro_rules! prop_object {
    ($t:ty) => {
        impl PropObject for $t {
            fn props(&self) -> *mut drmModeObjectProperties {
                self.props
            }
            fn props_info(&self) -> &[*mut drmModePropertyRes] {
                &self.props_info
            }
        }
    };
}
prop_object!(KmsdrmPlane);
prop_object!(KmsdrmCrtc);
prop_object!(KmsdrmConnector);

/// Translation of `struct SDL_VideoData`.
pub(crate) struct VideoData {
    /// device index that was passed on creation
    pub(crate) devindex: i32,
    /// DRM file desc
    pub(crate) drm_fd: c_int,
    /// DRM dev path.
    pub(crate) devpath: String,

    pub(crate) gbm_dev: *mut gbm_device,

    /// Has VideoInit succeeded?
    pub(crate) video_init: bool,
    /// Are we in Vulkan mode? One VK window is enough to be.
    pub(crate) vulkan_mode: bool,
    /// Does the hardware support async. pageflips?
    pub(crate) async_pageflip_support: bool,

    /// The windows (`windows`, `max_windows`, `num_windows`).
    pub(crate) windows: Vec<WindowID>,

    /// Even if we have several displays, we only have to
    /// open 1 FD and create 1 gbm device.
    pub(crate) gbm_init: bool,

    /// true if atomic interfaces are supported.
    pub(crate) is_atomic: bool,

    /// Whether `KMSDRM_InitMouse()` has filled in the mouse's entry points
    /// (`mouse->CreateCursor`, ...: NULL before the first window).
    pub(crate) mouse_init: bool,
}

/// Translation of `struct SDL_DisplayData` (with the `SDL_DisplayModeData`
/// of its modes, which the video core's modes don't carry).
pub(crate) struct DisplayData {
    pub(crate) display_plane: Option<Box<KmsdrmPlane>>,
    pub(crate) cursor_plane: Option<Box<KmsdrmPlane>>,
    pub(crate) crtc: KmsdrmCrtc,
    pub(crate) connector: KmsdrmConnector,

    pub(crate) mode: drmModeModeInfo,
    pub(crate) original_mode: drmModeModeInfo,
    pub(crate) fullscreen_mode: drmModeModeInfo,

    /// previous vrr state
    pub(crate) saved_vrr: bool,

    /* DRM & GBM cursor stuff lives here, not in an SDL_Cursor's internal struct,
    because setting/unsetting up these is done on window creation/destruction,
    where we may not have an SDL_Cursor at all (so no SDL_Cursor internal).
    There's only one cursor GBM BO because we only support one cursor. */
    pub(crate) cursor_bo: *mut gbm_bo,
    pub(crate) cursor_bo_drm_fd: c_int,
    pub(crate) cursor_w: u64,
    pub(crate) cursor_h: u64,

    /* Central atomic request list, used for the prop
    changeset related to pageflip in SwapWindow. */
    pub(crate) atomic_req: *mut drmModeAtomicReq,

    pub(crate) kms_in_fence_fd: c_int,
    pub(crate) kms_out_fence_fd: c_int,
    pub(crate) kms_fence: EGLSyncKHR,
    pub(crate) gpu_fence: EGLSyncKHR,

    pub(crate) default_cursor_init: bool,

    /// `display->desktop_mode.internal->mode_index`
    pub(crate) desktop_mode_index: i32,
    /// `mode->internal->mode_index` of the fullscreen modes, by the mode.
    pub(crate) modedata: Vec<(DisplayMode, i32)>,
}

/// Which plane of a display a [`KmsdrmPlaneInfo`] is about.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum PlaneSel {
    Display,
    Cursor,
}

/// Translation of `KMSDRM_PlaneInfo` (the plane named, a plane of the
/// display the info is used with).
#[derive(Clone, Copy, Debug)]
pub(crate) struct KmsdrmPlaneInfo {
    pub(crate) plane: PlaneSel,
    pub(crate) fb_id: u32,
    pub(crate) crtc_id: u32,
    pub(crate) src_x: i32,
    pub(crate) src_y: i32,
    pub(crate) src_w: i32,
    pub(crate) src_h: i32,
    pub(crate) crtc_x: i32,
    pub(crate) crtc_y: i32,
    pub(crate) crtc_w: i32,
    pub(crate) crtc_h: i32,
}

impl KmsdrmPlaneInfo {
    /// `SDL_zero(info)` with the plane set.
    pub(crate) fn zeroed(plane: PlaneSel) -> KmsdrmPlaneInfo {
        KmsdrmPlaneInfo {
            plane,
            fb_id: 0,
            crtc_id: 0,
            src_x: 0,
            src_y: 0,
            src_w: 0,
            src_h: 0,
            crtc_x: 0,
            crtc_y: 0,
            crtc_w: 0,
            crtc_h: 0,
        }
    }
}

/// This dictates what approach we'll use for SwapBuffers (`swap_window`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum SwapPath {
    /// `KMSDRM_GLES_SwapWindowFenced`
    Fenced,
    /// `KMSDRM_GLES_SwapWindowDoubleBuffered`
    DoubleBuffered,
    /// `KMSDRM_GLES_SwapWindowLegacy`
    Legacy,
}

/// Translation of `struct SDL_WindowData`.
pub(crate) struct WindowData {
    /* SDL internals expect EGL surface to be here, and in KMSDRM the GBM surface is
    what supports the EGL surface on the driver side, so all these surfaces and buffers
    are expected to be here, in the struct pointed by SDL_Window internal pointer:
    this one. So don't try to move these to dispdata!  */
    pub(crate) gs: *mut gbm_surface,
    pub(crate) bo: *mut gbm_bo,
    pub(crate) next_bo: *mut gbm_bo,

    /// (shared with the page flip handler, which clears it)
    pub(crate) waiting_for_flip: Arc<AtomicBool>,
    pub(crate) double_buffer: bool,

    pub(crate) egl_surface: EGLSurface,
    pub(crate) egl_surface_dirty: bool,

    /// This dictates what approach we'll use for SwapBuffers.
    pub(crate) swap_window: Option<SwapPath>,
}

/// Translation of `KMSDRM_FBInfo`: the user data of a buffer object, which
/// removes its framebuffer when the buffer is destroyed.
struct KmsdrmFbInfo {
    /// DRM file desc
    drm_fd: c_int,
    /// DRM framebuffer ID
    fb_id: u32,
    /// The libraries, loaded while the buffer is.
    syms: Arc<KmsdrmSyms>,
}

/// The driver's state: `SDL_VideoData`, and the `SDL_DisplayData` and
/// `SDL_WindowData` of the displays and windows.
pub(crate) struct State {
    pub(crate) vid: VideoData,
    pub(crate) displays: Vec<(DisplayID, Box<DisplayData>)>,
    pub(crate) windata: Vec<(WindowID, Box<WindowData>)>,
}

// SAFETY: the pointers are libdrm's, libgbm's and EGL's objects of the
// device, used by the video thread (the lock only moves them around).
unsafe impl Send for State {}

impl State {
    pub(crate) fn display(&mut self, id: DisplayID) -> Option<&mut DisplayData> {
        self.displays
            .iter_mut()
            .find(|(d, _)| *d == id)
            .map(|(_, d)| &mut **d)
    }

    pub(crate) fn window(&mut self, id: WindowID) -> Option<&mut WindowData> {
        self.windata
            .iter_mut()
            .find(|(w, _)| *w == id)
            .map(|(_, w)| &mut **w)
    }

    /// The display's and the window's data, and the device's.
    pub(crate) fn parts(
        &mut self,
        display: DisplayID,
        window: WindowID,
    ) -> Option<(&mut VideoData, &mut DisplayData, &mut WindowData)> {
        let State {
            vid,
            displays,
            windata,
        } = self;
        let d = displays
            .iter_mut()
            .find(|(d, _)| *d == display)
            .map(|(_, d)| &mut **d)?;
        let w = windata
            .iter_mut()
            .find(|(w, _)| *w == window)
            .map(|(_, w)| &mut **w)?;
        Some((vid, d, w))
    }

    /// The display's data, and the device's.
    pub(crate) fn display_parts(
        &mut self,
        display: DisplayID,
    ) -> Option<(&mut VideoData, &mut DisplayData)> {
        let State { vid, displays, .. } = self;
        let d = displays
            .iter_mut()
            .find(|(d, _)| *d == display)
            .map(|(_, d)| &mut **d)?;
        Some((vid, d))
    }
}

/// The KMS/DRM video device (`SDL_VideoDevice` with its `SDL_VideoData`).
pub(crate) struct KmsdrmVideo {
    /// The device itself (`_this`, the userdata of the VT callbacks).
    this: Weak<KmsdrmVideo>,
    /// The libraries (`SDL_KMSDRM_LoadSymbols()`, released with the device:
    /// `KMSDRM_DeleteDevice()`).
    pub(crate) syms: Arc<KmsdrmSyms>,
    state: Mutex<State>,
}

/// `open(path, O_RDWR | O_CLOEXEC)`.
fn open_rdwr(path: &str) -> c_int {
    let Ok(cpath) = std::ffi::CString::new(path) else {
        return -1;
    };
    // SAFETY: the path is NUL-terminated.
    unsafe { libc::open(cpath.as_ptr(), libc::O_RDWR | libc::O_CLOEXEC) }
}

fn close(fd: c_int) {
    // SAFETY: the descriptor is the driver's.
    unsafe { libc::close(fd) };
}

fn errno() -> c_int {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

/// A NUL-terminated C name (a property's, an enum's) as bytes.
fn c_name(name: &[std::ffi::c_char]) -> &[u8] {
    // SAFETY: c_char and u8 have the same layout.
    let bytes = unsafe { std::slice::from_raw_parts(name.as_ptr().cast::<u8>(), name.len()) };
    let len = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    &bytes[..len]
}

/// The slice of a C array (empty for NULL or a non-positive count).
///
/// # Safety
///
/// `ptr` must point to `count` elements that outlive the slice.
unsafe fn c_array<'a, T>(ptr: *const T, count: impl TryInto<usize>) -> &'a [T] {
    let count = count.try_into().unwrap_or(0);
    if ptr.is_null() || count == 0 {
        &[]
    } else {
        // SAFETY: as the caller promises.
        unsafe { std::slice::from_raw_parts(ptr, count) }
    }
}

/// Translation of `get_driindex()`.
fn get_driindex() -> i32 {
    let mut available = -libc::ENOENT;

    if let Some(hint) = hints::get(hints::KMSDRM_DEVICE_INDEX) {
        if !hint.is_empty() {
            let (idx, len) = crate::stdlib::string::strtol(&hint, 10);
            let idx = idx as c_int;
            if len == hint.len() && idx >= 0 {
                // *endptr==0 means "whole string was a valid number"
                return idx; // we'll take the user's request here.
            }
        }
    }

    let folder = match std::fs::read_dir(KMSDRM_DRI_PATH) {
        Ok(folder) => folder,
        Err(_) => {
            // (SDL_SetError("Failed to open directory '%s'"): the error is
            // the bootstrap's to drop)
            return -libc::ENOENT;
        }
    };

    for res in folder {
        if available >= 0 {
            break;
        }
        let Ok(res) = res else { break };
        let name = res.file_name();
        let Some(name) = name.to_str() else { continue };
        let Some(rest) = name.strip_prefix(KMSDRM_DRI_DEVNAME) else {
            continue;
        };
        let device = format!("{KMSDRM_DRI_CARDPATH}{rest}");

        let drm_fd = open_rdwr(&device);
        if drm_fd >= 0 {
            let devindex = crate::stdlib::atoi(rest);
            if let Some(syms) = kmsdrmdyn::load_symbols() {
                let drm = &syms.drm;
                // SAFETY: the descriptor is the card's; the resources and
                // connectors come from libdrm, freed here.
                unsafe {
                    let resources = (drm.drmModeGetResources)(drm_fd);
                    if !resources.is_null() {
                        let r = &*resources;
                        crate::debug!(
                            crate::log::Category::Video,
                            "{}{} connector, encoder and CRTC counts are: {} {} {}",
                            KMSDRM_DRI_CARDPATH,
                            devindex,
                            r.count_connectors,
                            r.count_encoders,
                            r.count_crtcs
                        );

                        if r.count_connectors > 0 && r.count_encoders > 0 && r.count_crtcs > 0 {
                            available = -libc::ENOENT;
                            for &conn_id in c_array(r.connectors, r.count_connectors) {
                                if available >= 0 {
                                    break;
                                }
                                let conn = (drm.drmModeGetConnector)(drm_fd, conn_id);

                                if conn.is_null() {
                                    continue;
                                }

                                if (*conn).connection == DRM_MODE_CONNECTED
                                    && (*conn).count_modes != 0
                                {
                                    let mut access_denied = false;
                                    if hints::get_bool(hints::KMSDRM_REQUIRE_DRM_MASTER, true) {
                                        /* Skip this device if we can't obtain
                                         * DRM master */
                                        (drm.drmSetMaster)(drm_fd);
                                        if (drm.drmAuthMagic)(drm_fd, 0) == -libc::EACCES {
                                            access_denied = true;
                                        }
                                    }

                                    if !access_denied {
                                        available = devindex;
                                    }
                                }

                                (drm.drmModeFreeConnector)(conn);
                            }
                        }
                        (drm.drmModeFreeResources)(resources);
                    }
                }
                // (SDL_KMSDRM_UnloadSymbols(): the handle's drop)
            }
            close(drm_fd);
        } else {
            crate::debug!(
                crate::log::Category::Video,
                "Failed to open KMSDRM device {}, errno: {}",
                device,
                errno()
            );
        }
    }

    available
}

/// Translation of `CalculateRefreshRate()`.
pub(crate) fn calculate_refresh_rate(mode: &drmModeModeInfo) -> (i32, i32) {
    let mut numerator = (mode.clock as i32).wrapping_mul(1000);
    let mut denominator = (mode.htotal as i32).wrapping_mul(mode.vtotal as i32);

    if mode.flags & DRM_MODE_FLAG_INTERLACE != 0 {
        numerator = numerator.wrapping_mul(2);
    }

    if mode.flags & DRM_MODE_FLAG_DBLSCAN != 0 {
        denominator = denominator.wrapping_mul(2);
    }

    if mode.vscan > 1 {
        denominator = denominator.wrapping_mul(mode.vscan as i32);
    }

    (numerator, denominator)
}

/// The ID of the object's property `name` (`None`: none of that name).
fn find_prop(obj: &dyn PropObject, name: &str) -> Option<u32> {
    let props = obj.props();
    if props.is_null() {
        return None;
    }
    // SAFETY: the properties and their infos are libdrm's, alive with the
    // object.
    unsafe {
        let count = (*props).count_props as usize;
        for &info in obj.props_info().iter().take(count) {
            if !info.is_null() && c_name(&(*info).name) == name.as_bytes() {
                return Some((*info).prop_id);
            }
        }
    }
    None
}

impl KmsdrmVideo {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Run `f` on the driver's state (never call back into SDL in it).
    pub(crate) fn with_state<R>(&self, f: impl FnOnce(&mut State) -> R) -> R {
        f(&mut self.lock())
    }

    /// Translation of `add_connector_property()`.
    pub(crate) fn add_connector_property(
        &self,
        req: *mut drmModeAtomicReq,
        conn: &KmsdrmConnector,
        name: &str,
        value: u64,
    ) -> c_int {
        // FIXME (upstream): prop_id starts at 0 here (-1 in the other two),
        // so a missing connector property is never reported and property 0
        // is added instead; kept.
        let prop_id = find_prop(conn, name).unwrap_or(0);
        let Some(atomic) = &self.syms.atomic else {
            return -libc::EINVAL;
        };
        // SAFETY: the request is libdrm's; the connector is the display's.
        unsafe {
            (atomic.drmModeAtomicAddProperty)(req, (*conn.connector).connector_id, prop_id, value)
        }
    }

    /// Translation of `add_crtc_property()`.
    pub(crate) fn add_crtc_property(
        &self,
        req: *mut drmModeAtomicReq,
        c: &KmsdrmCrtc,
        name: &str,
        value: u64,
    ) -> c_int {
        let Some(prop_id) = find_prop(c, name) else {
            // (SDL_SetError("no crtc property: %s"), which nothing reports)
            return -libc::EINVAL;
        };
        let Some(atomic) = &self.syms.atomic else {
            return -libc::EINVAL;
        };
        // SAFETY: as above.
        unsafe { (atomic.drmModeAtomicAddProperty)(req, (*c.crtc).crtc_id, prop_id, value) }
    }

    /// Translation of `add_plane_property()`.
    pub(crate) fn add_plane_property(
        &self,
        req: *mut drmModeAtomicReq,
        p: &KmsdrmPlane,
        name: &str,
        value: u64,
    ) -> c_int {
        let Some(prop_id) = find_prop(p, name) else {
            // (SDL_SetError("no plane property: %s"), which nothing reports)
            return -libc::EINVAL;
        };
        let Some(atomic) = &self.syms.atomic else {
            return -libc::EINVAL;
        };
        // SAFETY: as above.
        unsafe { (atomic.drmModeAtomicAddProperty)(req, (*p.plane).plane_id, prop_id, value) }
    }

    /* Get the plane_id of a plane that is of the specified plane type (primary,
    overlay, cursor...) and can use specified CRTC. */
    /// Translation of `get_plane_id()`.
    fn get_plane_id(&self, vid: &VideoData, crtc_id: u32, plane_type: u32) -> c_int {
        let drm = &self.syms.drm;
        let mut crtc_index: u32 = 0;
        let mut ret = -libc::EINVAL;
        let mut found = false;

        // SAFETY: the descriptor is the card's; everything else is libdrm's,
        // freed here.
        unsafe {
            let resources = (drm.drmModeGetResources)(vid.drm_fd);

            /* Get the crtc_index for the current CRTC.
            It's needed to find out if a plane supports the CRTC. */
            // Note (upstream): resources isn't checked for NULL there.
            if !resources.is_null() {
                for (i, &c) in c_array((*resources).crtcs, (*resources).count_crtcs)
                    .iter()
                    .enumerate()
                {
                    if c == crtc_id {
                        crtc_index = i as u32;
                        break;
                    }
                }
            }

            let plane_resources = (drm.drmModeGetPlaneResources)(vid.drm_fd);
            if plane_resources.is_null() {
                // (SDL_SetError("drmModeGetPlaneResources failed."), which
                // returns false: 0, the invalid plane ID)
                if !resources.is_null() {
                    (drm.drmModeFreeResources)(resources);
                }
                return 0;
            }

            // Iterate on all the available planes.
            for &plane_id in c_array((*plane_resources).planes, (*plane_resources).count_planes) {
                if found {
                    break;
                }

                let plane = (drm.drmModeGetPlane)(vid.drm_fd, plane_id);
                if plane.is_null() {
                    continue;
                }

                // See if the current CRTC is available for this plane.
                if (*plane).possible_crtcs & (1 << crtc_index) != 0 {
                    let props = (drm.drmModeObjectGetProperties)(
                        vid.drm_fd,
                        plane_id,
                        DRM_MODE_OBJECT_PLANE,
                    );
                    ret = plane_id as c_int;

                    /* Iterate on the plane props to find the type of the plane,
                    to see if it's of the type we want. */
                    if !props.is_null() {
                        let ids = c_array((*props).props, (*props).count_props);
                        let values = c_array((*props).prop_values, (*props).count_props);
                        for (j, &prop) in ids.iter().enumerate() {
                            let p = (drm.drmModeGetProperty)(vid.drm_fd, prop);
                            if p.is_null() {
                                continue;
                            }

                            if c_name(&(*p).name) == b"type" && values[j] == plane_type as u64 {
                                // found our plane, use that:
                                found = true;
                            }

                            (drm.drmModeFreeProperty)(p);
                        }

                        (drm.drmModeFreeObjectProperties)(props);
                    }
                }

                (drm.drmModeFreePlane)(plane);
            }

            (drm.drmModeFreePlaneResources)(plane_resources);
            if !resources.is_null() {
                (drm.drmModeFreeResources)(resources);
            }
        }

        ret
    }

    /// The properties of an object and their infos (the `props` and
    /// `props_info` of a plane, CRTC or connector).
    fn object_props(
        &self,
        drm_fd: c_int,
        object_id: u32,
        object_type: u32,
    ) -> (*mut drmModeObjectProperties, Vec<*mut drmModePropertyRes>) {
        let drm = &self.syms.drm;
        // SAFETY: the descriptor is the card's; the properties are libdrm's.
        unsafe {
            let props = (drm.drmModeObjectGetProperties)(drm_fd, object_id, object_type);
            let mut props_info = Vec::new();
            if !props.is_null() {
                for &prop in c_array((*props).props, (*props).count_props) {
                    props_info.push((drm.drmModeGetProperty)(drm_fd, prop));
                }
            }
            (props, props_info)
        }
    }

    /// Free an object's `props` and `props_info`.
    ///
    /// Note (upstream): upstream frees only the `props_info` array (of a
    /// plane), leaking the properties themselves (and all of the CRTC's
    /// and the connector's); here they are freed.
    fn free_props(
        &self,
        props: &mut *mut drmModeObjectProperties,
        info: &mut Vec<*mut drmModePropertyRes>,
    ) {
        let drm = &self.syms.drm;
        // SAFETY: the properties are libdrm's, not used after this.
        unsafe {
            for p in info.drain(..) {
                if !p.is_null() {
                    (drm.drmModeFreeProperty)(p);
                }
            }
            if !props.is_null() {
                (drm.drmModeFreeObjectProperties)(*props);
            }
        }
        *props = std::ptr::null_mut();
    }

    /// Setup a plane and it's props. Translation of `setup_plane()`.
    pub(crate) fn setup_plane(
        &self,
        vid: &VideoData,
        dispdata: &DisplayData,
        plane_type: u32,
    ) -> Result<Box<KmsdrmPlane>> {
        // Get plane ID for a given CRTC and plane type.
        // SAFETY: the display's CRTC is libdrm's.
        let crtc_id = unsafe { (*dispdata.crtc.crtc).crtc_id };
        let plane_id = self.get_plane_id(vid, crtc_id, plane_type);

        // Note (upstream): get_plane_id() returns -EINVAL when no plane can
        // use the CRTC, which upstream takes for a plane ID (the plane is
        // then NULL); the same here.
        if plane_id == 0 {
            return Err(Error::new("Invalid Plane ID"));
        }

        let mut plane = Box::new(KmsdrmPlane {
            plane: std::ptr::null_mut(),
            props: std::ptr::null_mut(),
            props_info: Vec::new(),
        });

        // Get the DRM plane itself.
        // SAFETY: the descriptor is the card's.
        plane.plane = unsafe { (self.syms.drm.drmModeGetPlane)(vid.drm_fd, plane_id as u32) };

        // Get the DRM plane properties.
        if !plane.plane.is_null() {
            // SAFETY: the plane is libdrm's.
            let id = unsafe { (*plane.plane).plane_id };
            let (props, props_info) = self.object_props(vid.drm_fd, id, DRM_MODE_OBJECT_PLANE);
            plane.props = props;
            plane.props_info = props_info;
        }

        Ok(plane)
    }

    /// Free a plane and it's props. Translation of `free_plane()`.
    pub(crate) fn free_plane(&self, plane: &mut Option<Box<KmsdrmPlane>>) {
        if let Some(mut p) = plane.take() {
            if !p.plane.is_null() {
                // SAFETY: the plane is libdrm's, not used after this.
                unsafe { (self.syms.drm.drmModeFreePlane)(p.plane) };
                p.plane = std::ptr::null_mut();
            }
            let KmsdrmPlane {
                props, props_info, ..
            } = &mut *p;
            self.free_props(props, props_info);
        }
    }

    /**********************************************************************************/
    /* The most important ATOMIC fn of the backend.                                   */
    /* A PLANE reads a BUFFER, and a CRTC reads a PLANE and sends it's contents       */
    /*   over to a CONNECTOR->ENCODER system (several CONNECTORS can be connected     */
    /*   to the same PLANE).                                                          */
    /*   Think of a plane as a "frame" surrounding a picture, where the "picture"     */
    /*   is the buffer, and we move the "frame" from  a picture to another,           */
    /*   and the one that has the "frame" is the one sent over to the screen          */
    /*   via the CONNECTOR->ENCODER system.                                           */
    /*   Think of a PLANE as being "in the middle", it's the CENTRAL part             */
    /*   between the CRTC and the BUFFER that is shown on screen.                     */
    /*   What we do here is connect a PLANE to a CRTC and a BUFFER.                   */
    /*   -ALWAYS set the CRTC_ID and FB_ID attribs of a plane at the same time,       */
    /*   meaning IN THE SAME atomic request.                                          */
    /*   -And NEVER destroy a GBM surface whose buffers are being read by a plane:    */
    /*   first, move the plane away from those buffers and ONLY THEN destroy the      */
    /*   buffers and/or the GBM surface containing them.                              */
    /**********************************************************************************/
    /// Translation of `drm_atomic_set_plane_props()`.
    pub(crate) fn drm_atomic_set_plane_props(
        &self,
        dispdata: &mut DisplayData,
        info: &KmsdrmPlaneInfo,
    ) {
        // Do we have a set of changes already in the making? If not, allocate a new one.
        if dispdata.atomic_req.is_null() {
            if let Some(atomic) = &self.syms.atomic {
                // SAFETY: drmModeAtomicAlloc has no preconditions.
                dispdata.atomic_req = unsafe { (atomic.drmModeAtomicAlloc)() };
            }
        }

        let req = dispdata.atomic_req;
        let plane = match info.plane {
            PlaneSel::Display => dispdata.display_plane.as_deref(),
            PlaneSel::Cursor => dispdata.cursor_plane.as_deref(),
        };
        // Note (upstream): a display without the plane (setup_plane()
        // failed) dereferences NULL there; nothing is added here.
        let Some(plane) = plane.filter(|p| !p.plane.is_null()) else {
            return;
        };
        self.add_plane_property(req, plane, "FB_ID", info.fb_id as u64);
        self.add_plane_property(req, plane, "CRTC_ID", info.crtc_id as u64);
        self.add_plane_property(req, plane, "SRC_W", (info.src_w << 16) as i64 as u64);
        self.add_plane_property(req, plane, "SRC_H", (info.src_h << 16) as i64 as u64);
        self.add_plane_property(req, plane, "SRC_X", info.src_x as i64 as u64);
        self.add_plane_property(req, plane, "SRC_Y", info.src_y as i64 as u64);
        self.add_plane_property(req, plane, "CRTC_W", info.crtc_w as i64 as u64);
        self.add_plane_property(req, plane, "CRTC_H", info.crtc_h as i64 as u64);
        self.add_plane_property(req, plane, "CRTC_X", info.crtc_x as i64 as u64);
        self.add_plane_property(req, plane, "CRTC_Y", info.crtc_y as i64 as u64);
    }

    /// Translation of `drm_atomic_commit()`.
    pub(crate) fn drm_atomic_commit(
        &self,
        vid: &VideoData,
        dispdata: &mut DisplayData,
        blocking: bool,
        allow_modeset: bool,
    ) -> c_int {
        let mut atomic_flags: u32 = 0;

        if !blocking {
            atomic_flags |= DRM_MODE_ATOMIC_NONBLOCK;
        }

        if allow_modeset {
            atomic_flags |= DRM_MODE_ATOMIC_ALLOW_MODESET;
        }

        /* Never issue a new atomic commit if previous has not yet completed,
        or it will error. */
        self.drm_atomic_waitpending(dispdata);

        let Some(atomic) = &self.syms.atomic else {
            return -libc::EINVAL;
        };

        // SAFETY: the descriptor is the card's; the request is libdrm's (or
        // NULL, which fails), freed here.
        let ret = unsafe {
            (atomic.drmModeAtomicCommit)(
                vid.drm_fd,
                dispdata.atomic_req,
                atomic_flags,
                std::ptr::null_mut(),
            )
        };

        if ret != 0 {
            // (SDL_SetError("Atomic commit failed, returned %d."), which the
            // callers replace with their own)
            crate::debug!(
                crate::log::Category::Video,
                "Atomic commit failed, returned {}.",
                ret
            );
        } else if dispdata.kms_in_fence_fd != -1 {
            close(dispdata.kms_in_fence_fd);
            dispdata.kms_in_fence_fd = -1;
        }

        // SAFETY: as above.
        unsafe { (atomic.drmModeAtomicFree)(dispdata.atomic_req) };
        dispdata.atomic_req = std::ptr::null_mut();

        ret
    }

    /// Translation of `drm_atomic_waitpending()`.
    pub(crate) fn drm_atomic_waitpending(&self, dispdata: &mut DisplayData) {
        /* Will return immediately if we have already destroyed the fence, because we NULL-ify it just after.
        Also, will return immediately in double-buffer mode, because kms_fence will alsawys be NULL. */
        if !dispdata.kms_fence.is_null() {
            let Some(e) = egl::egl() else { return };

            loop {
                let status = e.client_wait_sync_khr(dispdata.kms_fence, 0, EGL_FOREVER_KHR);
                if status == EGL_CONDITION_SATISFIED_KHR {
                    break;
                }
            }

            e.destroy_sync_khr(dispdata.kms_fence);
            dispdata.kms_fence = std::ptr::null_mut();
        }
    }
}

/// Translation of `KMSDRM_Available()`.
fn kmsdrm_available() -> bool {
    // (the paths are Linux's: moderndri)
    get_driindex() >= 0
}

/// Translation of `KMSDRM_CreateDevice()`.
pub(crate) fn kmsdrm_create_device() -> Option<Arc<KmsdrmVideo>> {
    if !kmsdrm_available() {
        return None;
    }

    let devindex = get_driindex();
    if devindex < 0 {
        // (SDL_SetError("devindex (%d) must not be negative."))
        return None;
    }

    let syms = kmsdrmdyn::load_symbols()?;

    Some(Arc::new_cyclic(|this| KmsdrmVideo {
        this: this.clone(),
        syms,
        state: Mutex::new(State {
            vid: VideoData {
                devindex,
                drm_fd: -1,
                devpath: String::new(),
                gbm_dev: std::ptr::null_mut(),
                video_init: false,
                vulkan_mode: false,
                async_pageflip_support: false,
                windows: Vec::new(),
                gbm_init: false,
                is_atomic: false,
                mouse_init: false,
            },
            displays: Vec::new(),
            windata: Vec::new(),
        }),
    }))
}

/// Translation of `KMSDRM_FBDestroyCallback()`.
unsafe extern "C" fn kmsdrm_fb_destroy_callback(_bo: *mut gbm_bo, data: *mut c_void) {
    if data.is_null() {
        return;
    }
    // SAFETY: the user data is the box KMSDRM_FBFromBO() leaked for the
    // buffer, given back once, here.
    let fb_info = unsafe { Box::from_raw(data.cast::<KmsdrmFbInfo>()) };

    if fb_info.drm_fd >= 0 && fb_info.fb_id != 0 {
        // SAFETY: the descriptor and framebuffer are the device's.
        unsafe { (fb_info.syms.drm.drmModeRmFB)(fb_info.drm_fd, fb_info.fb_id) };
        crate::debug!(
            crate::log::Category::Video,
            "Delete DRM FB {}",
            fb_info.fb_id
        );
    }

    // (SDL_free(fb_info): the box's drop)
}

/// Translation of `KMSDRM_FlipHandler()`.
unsafe extern "C" fn kmsdrm_flip_handler(
    _fd: c_int,
    _frame: std::ffi::c_uint,
    _sec: std::ffi::c_uint,
    _usec: std::ffi::c_uint,
    data: *mut c_void,
) {
    // SAFETY: the user data is the window's waiting_for_flip, kept alive
    // by the window (an Arc) while a flip is pending.
    unsafe { (*data.cast::<AtomicBool>()).store(false, Ordering::SeqCst) };
}

/// The fields of `SDL_Window` the driver reads.
#[derive(Clone, Copy, Debug)]
pub(crate) struct WindowInfo {
    pub(crate) flags: WindowFlags,
    pub(crate) w: i32,
    pub(crate) h: i32,
    pub(crate) windowed_w: i32,
    pub(crate) windowed_h: i32,
    pub(crate) fullscreen_exclusive: bool,
}

pub(crate) fn window_info(window: WindowID) -> Result<WindowInfo> {
    with_window(window, |w| WindowInfo {
        flags: w.flags(),
        w: w.core.w,
        h: w.core.h,
        windowed_w: w.core.windowed.w,
        windowed_h: w.core.windowed.h,
        fullscreen_exclusive: w.fullscreen_exclusive,
    })
}

/// The parts of a mode that identify it once finalized (the core adds the
/// display ID and fills in the refresh rate), to find its
/// `SDL_DisplayModeData`.
fn mode_key(mode: &DisplayMode) -> (i32, i32, i32, i32, u32) {
    let mut m = *mode;
    finalize_display_mode(&mut m);
    (
        m.w,
        m.h,
        m.refresh_rate_numerator,
        m.refresh_rate_denominator,
        m.format.0,
    )
}

impl KmsdrmVideo {
    /// Translation of `KMSDRM_FBFromBO()`: the framebuffer ID of the
    /// buffer, made the first time.
    pub(crate) fn kmsdrm_fb_from_bo(&self, vid: &VideoData, bo: *mut gbm_bo) -> Option<u32> {
        let gbm = &self.syms.gbm;
        let drm = &self.syms.drm;
        let mut rc: c_int = -1;
        let mut strides = [0u32; 4];
        let mut handles = [0u32; 4];
        let mut offsets = [0u32; 4];
        let mut flags: u32 = 0;
        let mut modifiers = [0u64; 4];

        // SAFETY: the buffer is GBM's; its user data is NULL or the
        // KmsdrmFbInfo set below.
        unsafe {
            // Check for an existing framebuffer
            let existing = (gbm.gbm_bo_get_user_data)(bo).cast::<KmsdrmFbInfo>();

            if !existing.is_null() {
                return Some((*existing).fb_id);
            }

            /* Create a structure that contains enough info to remove the framebuffer
            when the backing buffer is destroyed */
            let mut fb_info = Box::new(KmsdrmFbInfo {
                drm_fd: vid.drm_fd,
                fb_id: 0,
                syms: self.syms.clone(),
            });

            /* Create framebuffer object for the buffer using the modifiers requested by GBM.
            Use of the modifiers is necessary on some platforms. */
            let w = (gbm.gbm_bo_get_width)(bo);
            let h = (gbm.gbm_bo_get_height)(bo);
            let format = (gbm.gbm_bo_get_format)(bo);

            if let (
                Some(add_fb2_with_modifiers),
                Some(get_modifier),
                Some(get_plane_count),
                Some(get_offset),
                Some(get_stride_for_plane),
                Some(get_handle_for_plane),
            ) = (
                drm.drmModeAddFB2WithModifiers,
                gbm.gbm_bo_get_modifier,
                gbm.gbm_bo_get_plane_count,
                gbm.gbm_bo_get_offset,
                gbm.gbm_bo_get_stride_for_plane,
                gbm.gbm_bo_get_handle_for_plane,
            ) {
                modifiers[0] = get_modifier(bo);
                let num_planes = get_plane_count(bo);
                // Note (upstream): more than 4 planes would write past the
                // arrays there; they are capped here.
                for i in 0..num_planes.clamp(0, 4) {
                    let iu = i as usize;
                    strides[iu] = get_stride_for_plane(bo, i);
                    handles[iu] = get_handle_for_plane(bo, i).u32_;
                    offsets[iu] = get_offset(bo, i);
                    modifiers[iu] = modifiers[0];
                }

                if modifiers[0] != 0 && modifiers[0] != DRM_FORMAT_MOD_INVALID {
                    flags = DRM_MODE_FB_MODIFIERS;
                }

                rc = add_fb2_with_modifiers(
                    vid.drm_fd,
                    w,
                    h,
                    format,
                    handles.as_ptr(),
                    strides.as_ptr(),
                    offsets.as_ptr(),
                    modifiers.as_ptr(),
                    &mut fb_info.fb_id,
                    flags,
                );
            }

            if rc < 0 {
                strides[0] = (gbm.gbm_bo_get_stride)(bo);
                handles[0] = (gbm.gbm_bo_get_handle)(bo).u32_;
                rc = (drm.drmModeAddFB)(
                    vid.drm_fd,
                    w,
                    h,
                    24,
                    32,
                    strides[0],
                    handles[0],
                    &mut fb_info.fb_id,
                );
            }

            if rc < 0 {
                return None;
            }

            crate::debug!(
                crate::log::Category::Video,
                "New DRM FB ({}): {}x{}, from BO {:p}",
                fb_info.fb_id,
                w,
                h,
                bo
            );

            let fb_id = fb_info.fb_id;
            // Associate our DRM framebuffer with this buffer object
            (gbm.gbm_bo_set_user_data)(
                bo,
                Box::into_raw(fb_info).cast(),
                Some(kmsdrm_fb_destroy_callback),
            );

            Some(fb_id)
        }
    }

    /// Translation of `KMSDRM_WaitPageflip()`.
    pub(crate) fn kmsdrm_wait_pageflip(&self, vid: &VideoData, windata: &WindowData) -> bool {
        let mut ev = drmEventContext {
            version: DRM_EVENT_CONTEXT_VERSION,
            page_flip_handler: Some(kmsdrm_flip_handler),
            ..Default::default()
        };

        let mut pfd = libc::pollfd {
            fd: vid.drm_fd,
            events: libc::POLLIN,
            revents: 0,
        };

        /* Stay on the while loop until we get the desired event.
        We need the while the loop because we could be in a situation where:
        -We get and event on the FD in time, thus not on exiting on return number 1.
        -The event is not an error, thus not exiting on return number 2.
        -The event is of POLLIN type, but even then, if the event is not a pageflip,
         drmHandleEvent() won't unset wait_for_pageflip, so we have to iterate
         and go polling again.

         If it wasn't for the while loop, we could erroneously exit the function
         without the pageflip event to arrive!

        For example, vblank events hit the FD and they are POLLIN events too (POLLIN
        means "there's data to read on the FD"), but they are not the pageflip event
        we are waiting for, so the drmEventHandle() doesn't run the flip handler, and
        since waiting_for_flip is set on the pageflip handle, it's not set and we stay
        on the loop, until we get the event for the pageflip, which is fine.
        */
        while windata.waiting_for_flip.load(Ordering::SeqCst) {
            pfd.revents = 0;

            /* poll() waits for events arriving on the FD, and returns < 0 if timeout passes
            with no events or a signal occurred before any requested event (-EINTR).
            We wait forever (timeout = -1), but even if we DO get an event, we have yet
            to see if it's of the required type, then if it's a pageflip, etc */
            // SAFETY: one pollfd.
            let ret = unsafe { libc::poll(&mut pfd, 1, -1) };

            if ret < 0 {
                if errno() == libc::EINTR {
                    /* poll() returning < 0 and setting errno = EINTR means there was a signal before
                    any requested event, so we immediately poll again. */
                    continue;
                } else {
                    // There was another error. Don't pull again or we could get into a busy loop.
                    crate::error!(crate::log::Category::Video, "DRM poll error");
                    return false;
                }
            }

            if pfd.revents & (libc::POLLHUP | libc::POLLERR) != 0 {
                // An event arrived on the FD in time, but it's an error.
                crate::error!(crate::log::Category::Video, "DRM poll hup or error");
                return false;
            }

            if pfd.revents & libc::POLLIN != 0 {
                /* There is data to read on the FD!
                Is the event a pageflip? We know it is a pageflip if it matches the
                event we are passing in &ev. If it does, drmHandleEvent() will unset
                windata->waiting_for_flip and we will get out of the "while" loop.
                If it's not, we keep iterating on the loop. */
                // SAFETY: the descriptor is the card's; the context's
                // handler takes the user data of the window's flips.
                unsafe { (self.syms.drm.drmHandleEvent)(vid.drm_fd, &mut ev) };
            }

            /* If we got to this point in the loop, we may iterate or exit the loop:
            -A legit (non-error) event arrived, and it was a POLLING event, and it was consumed
             by drmHandleEvent().
               -If it was a PAGEFLIP event, waiting_for_flip will be unset by drmHandleEvent()
                and we will exit the loop.
               -If it wasn't a PAGEFLIP, drmHandleEvent() won't unset waiting_for_flip, so we
                iterare back to polling.
            -A legit (non-error) event arrived, but it's not a POLLIN event, so it hasn't to be
             consumed by drmHandleEvent(), so waiting_for_flip isn't set and we iterate back
             to polling. */
        }

        true
    }

    /* Given w, h and refresh rate, returns the closest DRM video mode
    available on the DRM connector of the display.
    We use the SDL mode list (which we filled in KMSDRM_GetDisplayModes)
    because it's ordered, while the list on the connector is mostly random.*/
    /// Translation of `KMSDRM_GetClosestDisplayMode()`: the index of the
    /// mode on the display's connector (the video core is asked: call it
    /// without the driver's state).
    pub(crate) fn kmsdrm_get_closest_display_mode(
        &self,
        display: DisplayID,
        width: i32,
        height: i32,
    ) -> Option<usize> {
        let closest =
            display::closest_fullscreen_display_mode(display, width, height, 0.0, false).ok()?;
        let key = mode_key(&closest);
        self.with_state(|s| {
            let d = s.display(display)?;
            d.modedata
                .iter()
                .find(|(m, _)| mode_key(m) == key)
                .map(|&(_, i)| i as usize)
        })
    }

    /*****************************************************************************/
    // SDL Video and Display initialization/handling functions
    /* _this is a SDL_VideoDevice *                                              */
    /*****************************************************************************/

    /// Translation of `KMSDRM_DropMaster()`.
    fn kmsdrm_drop_master(&self, vid: &VideoData) -> bool {
        let drm = &self.syms.drm;
        // SAFETY: the descriptor is the card's.
        unsafe {
            if vid.is_atomic {
                // turn off atomic support until we are in control again.
                (drm.drmSetClientCap)(vid.drm_fd, DRM_CLIENT_CAP_ATOMIC, 0);
                (drm.drmSetClientCap)(vid.drm_fd, DRM_CLIENT_CAP_UNIVERSAL_PLANES, 0);
            }

            // Check if we have DRM master to begin with
            if (drm.drmAuthMagic)(vid.drm_fd, 0) == -libc::EACCES {
                // Nope, nothing to do then
                return true;
            }

            (drm.drmDropMaster)(vid.drm_fd) == 0
        }
    }

    /// Deinitializes the internal of the SDL Displays in the SDL display
    /// list. Translation of `KMSDRM_DeinitDisplays()`.
    fn kmsdrm_deinit_displays(&self) {
        let ids = displays().unwrap_or_default();
        let drm = &self.syms.drm;
        self.with_state(|s| {
            // Iterate on the SDL Display list.
            for id in ids {
                // Get the internal for this display
                let Some(dispdata) = s.display(id) else {
                    continue;
                };

                // Free connector
                if !dispdata.connector.connector.is_null() {
                    // SAFETY: the connector is libdrm's, not used after this.
                    unsafe { (drm.drmModeFreeConnector)(dispdata.connector.connector) };
                    dispdata.connector.connector = std::ptr::null_mut();
                }

                // Free CRTC
                if !dispdata.crtc.crtc.is_null() {
                    // SAFETY: as above.
                    unsafe { (drm.drmModeFreeCrtc)(dispdata.crtc.crtc) };
                    dispdata.crtc.crtc = std::ptr::null_mut();
                }

                // (see free_props())
                let KmsdrmCrtc {
                    props, props_info, ..
                } = &mut dispdata.crtc;
                self.free_props(props, props_info);
                let KmsdrmConnector {
                    props, props_info, ..
                } = &mut dispdata.connector;
                self.free_props(props, props_info);
            }

            if s.vid.drm_fd >= 0 {
                close(s.vid.drm_fd);
                s.vid.drm_fd = -1;
            }
        });
    }
}

/// Translation of `KMSDRM_ConnectorCheckVrrCapable()`.
fn kmsdrm_connector_check_vrr_capable(
    drm: &kmsdrmdyn::Libdrm,
    drm_fd: c_int,
    output_id: u32,
) -> bool {
    let mut found = false;
    let mut prop_value: u64 = 0;

    // SAFETY: the descriptor is the card's; the properties are libdrm's,
    // freed here.
    unsafe {
        let props = (drm.drmModeObjectGetProperties)(drm_fd, output_id, DRM_MODE_OBJECT_CONNECTOR);
        if !props.is_null() {
            let ids = c_array((*props).props, (*props).count_props);
            let values = c_array((*props).prop_values, (*props).count_props);
            for (i, &id) in ids.iter().enumerate() {
                if found {
                    break;
                }
                let prop = (drm.drmModeGetProperty)(drm_fd, id);
                if !prop.is_null() {
                    if c_name(&(*prop).name).eq_ignore_ascii_case(b"VRR_CAPABLE") {
                        prop_value = values[i];
                        found = true;
                    }
                    (drm.drmModeFreeProperty)(prop);
                }
            }
            (drm.drmModeFreeObjectProperties)(props);
        }
    }
    if found {
        return prop_value != 0;
    }
    false
}

/// Translation of `KMSDRM_VrrPropId()`.
fn kmsdrm_vrr_prop_id(drm: &kmsdrmdyn::Libdrm, drm_fd: c_int, crtc_id: u32) -> Option<u32> {
    let mut found = None;
    // SAFETY: as above.
    unsafe {
        let props = (drm.drmModeObjectGetProperties)(drm_fd, crtc_id, DRM_MODE_OBJECT_CRTC);
        if !props.is_null() {
            for &id in c_array((*props).props, (*props).count_props) {
                if found.is_some() {
                    break;
                }
                let prop = (drm.drmModeGetProperty)(drm_fd, id);
                if !prop.is_null() {
                    if c_name(&(*prop).name) == b"VRR_ENABLED" {
                        found = Some((*prop).prop_id);
                    }
                    (drm.drmModeFreeProperty)(prop);
                }
            }
            (drm.drmModeFreeObjectProperties)(props);
        }
    }
    found
}

/// Translation of `KMSDRM_CrtcSetVrr()`.
pub(crate) fn kmsdrm_crtc_set_vrr(
    drm: &kmsdrmdyn::Libdrm,
    drm_fd: c_int,
    crtc_id: u32,
    enabled: bool,
) {
    let Some(vrr_prop_id) = kmsdrm_vrr_prop_id(drm, drm_fd, crtc_id) else {
        return;
    };

    // SAFETY: the descriptor is the card's.
    unsafe {
        (drm.drmModeObjectSetProperty)(
            drm_fd,
            crtc_id,
            DRM_MODE_OBJECT_CRTC,
            vrr_prop_id,
            enabled as u64,
        )
    };
}

/// Translation of `KMSDRM_CrtcGetVrr()`.
fn kmsdrm_crtc_get_vrr(drm: &kmsdrmdyn::Libdrm, drm_fd: c_int, crtc_id: u32) -> bool {
    let mut found = false;
    let mut prop_value: u64 = 0;

    let Some(vrr_prop_id) = kmsdrm_vrr_prop_id(drm, drm_fd, crtc_id) else {
        return false;
    };

    // SAFETY: as above.
    unsafe {
        let props = (drm.drmModeObjectGetProperties)(drm_fd, crtc_id, DRM_MODE_OBJECT_CRTC);
        if !props.is_null() {
            let ids = c_array((*props).props, (*props).count_props);
            let values = c_array((*props).prop_values, (*props).count_props);
            for (i, &id) in ids.iter().enumerate() {
                if found {
                    break;
                }
                let prop = (drm.drmModeGetProperty)(drm_fd, id);
                if !prop.is_null() {
                    if (*prop).prop_id == vrr_prop_id {
                        prop_value = values[i];
                        found = true;
                    }
                    (drm.drmModeFreeProperty)(prop);
                }
            }
            (drm.drmModeFreeObjectProperties)(props);
        }
    }
    if found {
        return prop_value != 0;
    }
    false
}

/// Translation of `KMSDRM_CrtcGetOrientation()`.
///
/// FIXME (upstream): the CRTC's ID is looked up as a connector
/// (`DRM_MODE_OBJECT_CONNECTOR`), which finds no object, so no properties;
/// and the orientation would come from the property's first enum value
/// ("Normal"), not from its current value. The orientation is always 0;
/// kept.
fn kmsdrm_crtc_get_orientation(drm: &kmsdrmdyn::Libdrm, drm_fd: c_int, crtc_id: u32) -> i32 {
    let mut found = false;
    let mut orientation = 0;

    // SAFETY: as above.
    unsafe {
        let props = (drm.drmModeObjectGetProperties)(drm_fd, crtc_id, DRM_MODE_OBJECT_CONNECTOR);
        if !props.is_null() {
            for &id in c_array((*props).props, (*props).count_props) {
                if found {
                    break;
                }
                let prop = (drm.drmModeGetProperty)(drm_fd, id);
                if !prop.is_null() {
                    if c_name(&(*prop).name).eq_ignore_ascii_case(b"panel orientation")
                        && (*prop).flags & DRM_MODE_PROP_ENUM != 0
                    {
                        if (*prop).count_enums != 0 && !(*prop).enums.is_null() {
                            // "Normal" is the default of no rotation (0 degrees)
                            let name = c_name(&(*(*prop).enums).name);
                            if name == b"Left Side Up" {
                                orientation = 90;
                            } else if name == b"Upside Down" {
                                orientation = 180;
                            } else if name == b"Right Side Up" {
                                orientation = 270;
                            }
                        }
                        found = true;
                    }
                    (drm.drmModeFreeProperty)(prop);
                }
            }
            (drm.drmModeFreeObjectProperties)(props);
        }
    }
    orientation
}

/// Translation of `set_client_atomic_caps()`.
///
/// Note (upstream): without libdrm's atomic functions (the `ATOMIC`
/// module, which upstream doesn't require) upstream would call NULL; here
/// atomic isn't used then.
fn set_client_atomic_caps(syms: &KmsdrmSyms, fd: c_int) -> bool {
    let drm = &syms.drm;
    if !hints::get_bool(hints::KMSDRM_ATOMIC, true) {
        return false; // emergency escape hatch.
    }
    if syms.atomic.is_none() {
        return false;
    }
    // SAFETY: the descriptor is the card's.
    unsafe {
        if (drm.drmSetClientCap)(fd, DRM_CLIENT_CAP_ATOMIC, 1) != 0 {
            return false; // no atomic modesetting support.
        } else if (drm.drmSetClientCap)(fd, DRM_CLIENT_CAP_UNIVERSAL_PLANES, 1) != 0 {
            (drm.drmSetClientCap)(fd, DRM_CLIENT_CAP_ATOMIC, 0); // turn this back off again.
            return false; // no universal planes support.
        }
    }
    true
}

impl KmsdrmVideo {
    /* Gets a DRM connector, builds an SDL_Display with it, and adds it to the
    list of SDL Displays in _this->displays[]  */
    /// Translation of `KMSDRM_AddDisplay()` (the connector is the display's
    /// on success, freed on failure).
    fn kmsdrm_add_display(
        &self,
        drm_fd: c_int,
        conn: *mut drmModeConnector,
        resources: *mut drmModeRes,
    ) {
        let drm = &self.syms.drm;
        let mut encoder: *mut drmModeEncoder = std::ptr::null_mut();
        let mut crtc: *mut drmModeCrtc = std::ptr::null_mut();
        let mut name_fmt: Option<String> = None;

        // SAFETY: the connector, resources, encoder and CRTC are libdrm's;
        // the descriptor is the card's.
        let result: std::result::Result<(), ()> = unsafe {
            (|| {
                let c = &*conn;
                let r = &*resources;

                // Try to find the connector's current encoder
                for &enc_id in c_array(r.encoders, r.count_encoders) {
                    encoder = (drm.drmModeGetEncoder)(drm_fd, enc_id);

                    if encoder.is_null() {
                        continue;
                    }

                    if (*encoder).encoder_id == c.encoder_id {
                        break;
                    }

                    (drm.drmModeFreeEncoder)(encoder);
                    encoder = std::ptr::null_mut();
                }

                if encoder.is_null() {
                    // No encoder was connected, find the first supported one
                    for &enc_id in c_array(r.encoders, r.count_encoders) {
                        encoder = (drm.drmModeGetEncoder)(drm_fd, enc_id);

                        if encoder.is_null() {
                            continue;
                        }

                        if c_array(c.encoders, c.count_encoders).contains(&(*encoder).encoder_id) {
                            break;
                        }

                        (drm.drmModeFreeEncoder)(encoder);
                        encoder = std::ptr::null_mut();
                    }
                }

                if encoder.is_null() {
                    // (SDL_SetError("No connected encoder found for connector."))
                    return Err(());
                }

                // Try to find a CRTC connected to this encoder
                crtc = (drm.drmModeGetCrtc)(drm_fd, (*encoder).crtc_id);

                /* If no CRTC was connected to the encoder, find the first CRTC
                that is supported by the encoder, and use that. */
                if crtc.is_null() {
                    for (i, &crtc_id) in c_array(r.crtcs, r.count_crtcs).iter().enumerate() {
                        if (*encoder).possible_crtcs & (1 << i) != 0 {
                            (*encoder).crtc_id = crtc_id;
                            crtc = (drm.drmModeGetCrtc)(drm_fd, (*encoder).crtc_id);
                            break;
                        }
                    }
                }

                if crtc.is_null() {
                    // (SDL_SetError("No CRTC found for connector."))
                    return Err(());
                }

                // Find the index of the mode attached to this CRTC
                let modes = c_array(c.modes, c.count_modes);
                let mut mode_index: i32 = modes
                    .iter()
                    .position(|m| *m == (*crtc).mode)
                    .map_or(-1, |i| i as i32);

                if mode_index == -1 {
                    let mut largest_area = 0;

                    // Find the preferred mode or the highest resolution mode
                    for (i, mode) in modes.iter().enumerate() {
                        if mode.type_ & DRM_MODE_TYPE_PREFERRED != 0 {
                            mode_index = i as i32;
                            break;
                        }

                        let current_area = mode.hdisplay as i32 * mode.vdisplay as i32;
                        if current_area > largest_area {
                            mode_index = i as i32;
                            largest_area = current_area;
                        }
                    }
                    if mode_index != -1 {
                        (*crtc).mode = modes[mode_index as usize];
                    }
                }

                if mode_index == -1 {
                    // (SDL_SetError("Failed to find index of mode attached to the CRTC."))
                    return Err(());
                }

                /*********************************************/
                // Create an SDL Display for this connector.
                /*********************************************/

                /*********************************************/
                // Part 1: setup the SDL_Display internal.
                /*********************************************/

                /* Get the mode currently setup for this display,
                which is the mode currently setup on the CRTC
                we found for the active connector. */
                let mode = (*crtc).mode;

                if mode.hdisplay == 0 || mode.vdisplay == 0 {
                    // (SDL_SetError("Couldn't get a valid connector videomode."))
                    return Err(());
                }

                let crtc_id = (*crtc).crtc_id;

                // save previous vrr state
                let saved_vrr = kmsdrm_crtc_get_vrr(drm, drm_fd, crtc_id);
                // try to enable vrr
                if kmsdrm_connector_check_vrr_capable(drm, drm_fd, c.connector_id) {
                    crate::debug!(crate::log::Category::Video, "Enabling VRR");
                    kmsdrm_crtc_set_vrr(drm, drm_fd, crtc_id, true);
                }

                // Set the name by the connector type, if possible
                if let Some(get_connector_type_name) = drm.drmModeGetConnectorTypeName {
                    let connector_type = get_connector_type_name(c.connector_type);
                    let connector_type = if connector_type.is_null() {
                        "Unknown".to_owned()
                    } else {
                        CStr::from_ptr(connector_type)
                            .to_string_lossy()
                            .into_owned()
                    };
                    // (snprintf into 64 bytes)
                    let mut name = format!("{connector_type}-{}", c.connector_type_id);
                    while name.len() > 63 {
                        name.pop();
                    }
                    name_fmt = Some(name);
                }

                let (crtc_props, crtc_props_info) =
                    self.object_props(drm_fd, crtc_id, DRM_MODE_OBJECT_CRTC);

                // Get connector properties
                let (conn_props, conn_props_info) =
                    self.object_props(drm_fd, c.connector_id, DRM_MODE_OBJECT_CONNECTOR);

                /*****************************************/
                // Part 2: setup the SDL_Display itself.
                /*****************************************/

                let dispdata = Box::new(DisplayData {
                    display_plane: None,
                    cursor_plane: None,
                    crtc: KmsdrmCrtc {
                        crtc,
                        props: crtc_props,
                        props_info: crtc_props_info,
                    },
                    connector: KmsdrmConnector {
                        connector: conn,
                        props: conn_props,
                        props_info: conn_props_info,
                    },
                    mode,
                    original_mode: mode,
                    fullscreen_mode: mode,
                    saved_vrr,
                    // Initialize some of the members of the new display's
                    // internal to sane values.
                    cursor_bo: std::ptr::null_mut(),
                    cursor_bo_drm_fd: -1,
                    cursor_w: 0,
                    cursor_h: 0,
                    atomic_req: std::ptr::null_mut(),
                    kms_in_fence_fd: -1,
                    kms_out_fence_fd: -1,
                    kms_fence: std::ptr::null_mut(),
                    gpu_fence: std::ptr::null_mut(),
                    /* Since we create and show the default cursor on KMSDRM_InitMouse(),
                    and we call KMSDRM_InitMouse() when we create a window, we have to know
                    if the display used by the window already has a default cursor or not.
                    If we don't, new default cursors would stack up on mouse->cursors and SDL
                    would have to hide and delete them at quit, not to mention the memory leak... */
                    default_cursor_init: false,
                    desktop_mode_index: mode_index,
                    modedata: Vec::new(),
                });

                /* Setup the display.
                There's no problem with it being still incomplete. */
                let mut display = VideoDisplay::new();
                let (num, den) = calculate_refresh_rate(&mode);
                display.desktop_mode = DisplayMode {
                    w: mode.hdisplay as i32,
                    h: mode.vdisplay as i32,
                    refresh_rate_numerator: num,
                    refresh_rate_denominator: den,
                    format: PixelFormat::ARGB8888,
                    ..Default::default()
                };
                display.name = name_fmt.clone();

                // Add the display to the list of SDL displays.
                let display_id = display::add_video_display(display, false);
                if display_id == 0 {
                    let mut d = dispdata;
                    let KmsdrmCrtc {
                        props, props_info, ..
                    } = &mut d.crtc;
                    self.free_props(props, props_info);
                    let KmsdrmConnector {
                        props, props_info, ..
                    } = &mut d.connector;
                    self.free_props(props, props_info);
                    return Err(());
                }
                self.with_state(|s| s.displays.push((display_id, dispdata)));

                let orientation = kmsdrm_crtc_get_orientation(drm, drm_fd, crtc_id);
                if let Ok(display_properties) = display::display_properties(display_id) {
                    let _ = display_properties.set(
                        display::PROP_DISPLAY_KMSDRM_PANEL_ORIENTATION_NUMBER,
                        orientation as i64,
                    );
                }

                Ok(())
            })()
        };

        // SAFETY: as above; on success the connector and CRTC are the
        // display's.
        unsafe {
            if !encoder.is_null() {
                (drm.drmModeFreeEncoder)(encoder);
            }
            if result.is_err() {
                // Error (complete) cleanup
                (drm.drmModeFreeConnector)(conn);
                if !crtc.is_null() {
                    (drm.drmModeFreeCrtc)(crtc);
                }
            }
        }
    }

    /// Translation of `KMSDRM_SortDisplays()`.
    fn kmsdrm_sort_displays(&self) {
        let Some(name_hint) = hints::get(hints::VIDEO_DISPLAY_PRIORITY) else {
            return;
        };

        let _ = crate::video::core::with_device(|v| {
            let mut remaining: Vec<Option<VideoDisplay>> = v.displays.drain(..).map(Some).collect();
            let mut sorted_list = Vec::with_capacity(remaining.len());

            // Sort the requested displays to the front of the list.
            for token in name_hint.split(',').filter(|t| !t.is_empty()) {
                for slot in remaining.iter_mut() {
                    if slot
                        .as_ref()
                        .is_some_and(|d| d.name.as_deref() == Some(token))
                    {
                        sorted_list.push(slot.take().expect("checked"));
                        break;
                    }
                }
            }

            // Append the remaining displays to the end of the list.
            sorted_list.extend(remaining.into_iter().flatten());

            // Copy the sorted list back to the display list.
            v.displays = sorted_list;
        });
    }

    /* Initializes the list of SDL displays: we build a new display for each
    connecter connector we find.
    This is to be called early, in VideoInit(), because it gets us
    the videomode information, which SDL needs immediately after VideoInit(). */
    /// Translation of `KMSDRM_InitDisplays()`.
    fn kmsdrm_init_displays(&self) -> Result<()> {
        let drm = &self.syms.drm;

        // Open /dev/dri/cardNN (/dev/drmN if on OpenBSD version less than 6.9)
        let (devpath, drm_fd) = self.with_state(|s| {
            s.vid.devpath = format!("{KMSDRM_DRI_CARDPATH}{}", s.vid.devindex);

            crate::debug!(
                crate::log::Category::Video,
                "Opening device {}",
                s.vid.devpath
            );
            s.vid.drm_fd = open_rdwr(&s.vid.devpath);
            (s.vid.devpath.clone(), s.vid.drm_fd)
        });

        let fail = |e: Error| -> Result<()> {
            self.with_state(|s| {
                if s.vid.drm_fd >= 0 {
                    close(s.vid.drm_fd);
                    s.vid.drm_fd = -1;
                }
            });
            Err(e)
        };

        if drm_fd < 0 {
            return fail(Error::new(format!("Could not open {devpath}")));
        }

        crate::debug!(crate::log::Category::Video, "Opened DRM FD ({})", drm_fd);

        // Set ATOMIC & UNIVERSAL PLANES compatibility
        let is_atomic = set_client_atomic_caps(&self.syms, drm_fd);
        self.with_state(|s| s.vid.is_atomic = is_atomic);

        crate::debug!(
            crate::log::Category::Video,
            "DRM FD ({}) {} atomic",
            drm_fd,
            if is_atomic {
                "SUPPORTS"
            } else {
                "DOES NOT SUPPORT"
            }
        );

        // Get all of the available connectors / devices / crtcs
        // SAFETY: the descriptor is the card's.
        let resources = unsafe { (drm.drmModeGetResources)(drm_fd) };
        if resources.is_null() {
            return fail(Error::new(format!("drmModeGetResources({drm_fd}) failed")));
        }

        /* Iterate on the available connectors. For every connected connector,
        we create an SDL_Display and add it to the list of SDL Displays. */
        // SAFETY: the resources are libdrm's, freed below.
        let connector_ids: Vec<u32> =
            unsafe { c_array((*resources).connectors, (*resources).count_connectors).to_vec() };
        for conn_id in connector_ids {
            // SAFETY: the descriptor is the card's.
            let conn = unsafe { (drm.drmModeGetConnector)(drm_fd, conn_id) };
            if conn.is_null() {
                continue;
            }

            // SAFETY: the connector is libdrm's.
            if unsafe { (*conn).connection == DRM_MODE_CONNECTED && (*conn).count_modes != 0 } {
                /* If it's a connected connector with available videomodes, try to add
                an SDL Display representing it. KMSDRM_AddDisplay() is purposely void,
                so if it fails (no encoder for connector, no valid video mode for
                connector etc...) we can keep looking for connected connectors. */
                self.kmsdrm_add_display(drm_fd, conn, resources);
            } else {
                // If it's not, free it now.
                // SAFETY: as above.
                unsafe { (drm.drmModeFreeConnector)(conn) };
            }
        }

        // SAFETY: the resources are libdrm's, not used after this.
        let free_resources = || unsafe { (drm.drmModeFreeResources)(resources) };

        // Have we added any SDL displays?
        if display::primary_display().is_err() {
            free_resources();
            return fail(Error::new("No connected displays found."));
        }

        // Sort the displays, if necessary
        self.kmsdrm_sort_displays();

        // Determine if video hardware supports async pageflips.
        let mut async_pageflip: u64 = 0;
        // SAFETY: the descriptor is the card's; an out-parameter.
        if unsafe { (drm.drmGetCap)(drm_fd, DRM_CAP_ASYNC_PAGE_FLIP, &mut async_pageflip) } != 0 {
            crate::error!(
                crate::log::Category::Video,
                "Could not determine async page flip capability."
            );
        }

        self.with_state(|s| {
            s.vid.async_pageflip_support = async_pageflip != 0;

            /***********************************/
            // Block for Vulkan compatibility.
            /***********************************/

            /* Vulkan requires DRM master on its own FD to work, so try to drop master
            on our FD. This will only work without root on kernels v5.8 and later.
            If it doesn't work, just close the FD and we'll reopen it later. */
            if !self.kmsdrm_drop_master(&s.vid) {
                close(s.vid.drm_fd);
                s.vid.drm_fd = -1;
            }
        });

        free_resources();
        Ok(())
    }

    /* Init the Vulkan-INCOMPATIBLE stuff:
    Reopen FD, create gbm dev, create dumb buffer and setup display plane.
    This is to be called late, in WindowCreate(), and ONLY if this is not
    a Vulkan window.
    We are doing this so late to allow Vulkan to work if we build a VK window.
    These things are incompatible with Vulkan, which accesses the same resources
    internally so they must be free when trying to build a Vulkan surface.
    */
    /// Translation of `KMSDRM_GBMInit()`.
    fn kmsdrm_gbm_init(&self, s: &mut State, display: DisplayID) -> Result<()> {
        let drm = &self.syms.drm;
        let gbm = &self.syms.gbm;
        let State { vid, displays, .. } = s;
        let Some(dispdata) = displays
            .iter_mut()
            .find(|(d, _)| *d == display)
            .map(|(_, d)| &mut **d)
        else {
            return Err(Error::new("Invalid display"));
        };

        // Reopen the FD if we weren't able to drop master on the original one
        if vid.drm_fd < 0 {
            vid.drm_fd = open_rdwr(&vid.devpath);
            if vid.drm_fd < 0 {
                return Err(Error::new(format!("Could not reopen {}", vid.devpath)));
            }
        }

        vid.is_atomic = set_client_atomic_caps(&self.syms, vid.drm_fd);

        // SAFETY: the descriptor is the card's.
        unsafe {
            // Set the FD as current DRM master.
            (drm.drmSetMaster)(vid.drm_fd);

            // Create the GBM device.
            vid.gbm_dev = (gbm.gbm_create_device)(vid.drm_fd);
        }
        let result = if vid.gbm_dev.is_null() {
            Err(Error::new("Couldn't create gbm device."))
        } else {
            // Setup the display plane. ONLY do this after dispdata has the right
            // crtc and connector, because these are used in this function.
            match self.setup_plane(vid, dispdata, DRM_PLANE_TYPE_PRIMARY) {
                Ok(plane) => {
                    dispdata.display_plane = Some(plane);
                    Ok(())
                }
                Err(_) => Err(Error::new("can't find suitable display plane.")),
            }
        };

        vid.gbm_init = true;

        result
    }

    /// Deinit the Vulkan-incompatible KMSDRM stuff. Translation of
    /// `KMSDRM_GBMDeinit()`.
    fn kmsdrm_gbm_deinit(&self, s: &mut State, display: DisplayID) {
        let State { vid, displays, .. } = s;
        if let Some(dispdata) = displays
            .iter_mut()
            .find(|(d, _)| *d == display)
            .map(|(_, d)| &mut **d)
        {
            // Free display plane
            self.free_plane(&mut dispdata.display_plane);

            // Free cursor plane (if still not freed)
            self.free_plane(&mut dispdata.cursor_plane);
        }

        /* Destroy GBM device. GBM surface is destroyed by DestroySurfaces(),
        already called when we get here. */
        if !vid.gbm_dev.is_null() {
            // SAFETY: the device is GBM's, not used after this.
            unsafe { (self.syms.gbm.gbm_device_destroy)(vid.gbm_dev) };
            vid.gbm_dev = std::ptr::null_mut();
        }

        /* Finally drop DRM master if possible, otherwise close DRM FD.
        May be reopened on next non-vulkan window creation. */
        if vid.drm_fd >= 0 && !self.kmsdrm_drop_master(vid) {
            close(vid.drm_fd);
            vid.drm_fd = -1;
        }

        vid.gbm_init = false;
    }

    /// Translation of `KMSDRM_DestroySurfaces()` (on the locked state: it
    /// doesn't call back into SDL).
    pub(crate) fn kmsdrm_destroy_surfaces(
        &self,
        s: &mut State,
        window: WindowID,
        display: DisplayID,
    ) {
        let drm = &self.syms.drm;
        let gbm = &self.syms.gbm;
        let Some((vid, dispdata, windata)) = s.parts(display, window) else {
            return;
        };

        /**********************************************/
        // Wait for last issued pageflip to complete.
        /**********************************************/
        // KMSDRM_WaitPageflip(_this, windata);

        // SAFETY: the display's CRTC and connector are libdrm's.
        let (crtc_id, buffer_id, mode_valid, connector_id) = unsafe {
            (
                (*dispdata.crtc.crtc).crtc_id,
                (*dispdata.crtc.crtc).buffer_id,
                (*dispdata.crtc.crtc).mode_valid,
                (*dispdata.connector.connector).connector_id,
            )
        };

        if vid.is_atomic {
            /* TODO : Continue investigating why this doesn't work. We should do this instead
            of making the display plane point to the TTY console, which isn't there
            after creating and destroying a Vulkan window. */
            // (upstream's `#if 0` block, which has bitrotted, isn't translated)

            /************************************************************/
            /* Make the display plane point to the original TTY buffer. */
            /* We have to configure it's input and output scaling       */
            /* parameters accordingly.                                  */
            /************************************************************/

            let mut plane_info = KmsdrmPlaneInfo::zeroed(PlaneSel::Display);
            plane_info.crtc_id = crtc_id;
            plane_info.fb_id = buffer_id;
            plane_info.src_w = dispdata.original_mode.hdisplay as i32;
            plane_info.src_h = dispdata.original_mode.vdisplay as i32;
            plane_info.crtc_w = dispdata.original_mode.hdisplay as i32;
            plane_info.crtc_h = dispdata.original_mode.vdisplay as i32;

            self.drm_atomic_set_plane_props(dispdata, &plane_info);

            if self.drm_atomic_commit(vid, dispdata, true, false) != 0 {
                // (SDL_SetError("Failed to issue atomic commit on surfaces
                // destruction."), which nothing reports)
            }
        }

        /************************************************************************/
        // Restore the original CRTC configuration: configure the crtc with the
        // original video mode and make it point to the original TTY buffer.
        /************************************************************************/

        let mut connector_id = connector_id;
        // SAFETY: the descriptor is the card's; the connector and mode are
        // read for the call.
        let mut ret = unsafe {
            (drm.drmModeSetCrtc)(
                vid.drm_fd,
                crtc_id,
                buffer_id,
                0,
                0,
                &mut connector_id,
                1,
                &mut dispdata.original_mode,
            )
        };

        // If we failed to set the original mode, try to set the connector preferred mode.
        if ret != 0 && mode_valid == 0 {
            // SAFETY: as above.
            ret = unsafe {
                (drm.drmModeSetCrtc)(
                    vid.drm_fd,
                    crtc_id,
                    buffer_id,
                    0,
                    0,
                    &mut connector_id,
                    1,
                    &mut dispdata.original_mode,
                )
            };
        }

        if ret != 0 {
            crate::debug!(crate::log::Category::Video, "Could not restore CRTC");
        }

        /***************************/
        // Destroy the GBM buffers
        /***************************/

        // SAFETY: the buffers are the GBM surface's, locked by the window.
        unsafe {
            if !windata.bo.is_null() {
                if windata.bo != windata.next_bo {
                    (gbm.gbm_surface_release_buffer)(windata.gs, windata.bo);
                }
                windata.bo = std::ptr::null_mut();
            }

            if !windata.next_bo.is_null() {
                (gbm.gbm_surface_release_buffer)(windata.gs, windata.next_bo);
                windata.next_bo = std::ptr::null_mut();
            }
        }

        /***************************/
        // Destroy the EGL surface
        /***************************/

        let _ = egl::make_current(EGL_NO_SURFACE, None);

        if windata.egl_surface != EGL_NO_SURFACE {
            egl::destroy_surface(windata.egl_surface);
            windata.egl_surface = EGL_NO_SURFACE;
        }

        /***************************/
        // Destroy the GBM surface
        /***************************/

        if !windata.gs.is_null() {
            // SAFETY: the surface is GBM's, not used after this.
            unsafe { (gbm.gbm_surface_destroy)(windata.gs) };
            windata.gs = std::ptr::null_mut();
        }
    }

    /// The mode to set for the window: `KMSDRM_GetModeToSet()`, with the
    /// closest mode's index (`KMSDRM_GetClosestDisplayMode()`, asked of the
    /// video core beforehand).
    fn mode_to_set(
        info: &WindowInfo,
        dispdata: &DisplayData,
        closest: Option<usize>,
    ) -> drmModeModeInfo {
        if info.fullscreen_exclusive {
            dispdata.fullscreen_mode
        } else {
            // SAFETY: the connector's modes are libdrm's.
            let modes = unsafe {
                let conn = &*dispdata.connector.connector;
                c_array(conn.modes, conn.count_modes)
            };
            match closest.and_then(|i| modes.get(i)) {
                Some(mode) => *mode,
                None => dispdata.original_mode,
            }
        }
    }

    /// The closest mode's index for the window's windowed size (asked of
    /// the video core: without the driver's state).
    fn closest_for_window(&self, display: DisplayID, info: &WindowInfo) -> Option<usize> {
        if info.fullscreen_exclusive {
            None
        } else {
            self.kmsdrm_get_closest_display_mode(display, info.windowed_w, info.windowed_h)
        }
    }

    /// Translation of `KMSDRM_DirtySurfaces()`.
    pub(crate) fn kmsdrm_dirty_surfaces(&self, window: WindowID) {
        let Ok(info) = window_info(window) else {
            return;
        };
        let Ok(display) = display_for_window(window) else {
            return;
        };
        let closest = self.closest_for_window(display, &info);

        let mode = self.with_state(|s| {
            let windata = s.window(window)?;

            /* Can't recreate EGL surfaces right now, need to wait until SwapWindow
            so the correct thread-local surface and context state are available */
            windata.egl_surface_dirty = true;

            /* The app may be waiting for the resize event after calling SetWindowSize
            or SetWindowFullscreen, send a fake event for now since the actual
            recreation is deferred */
            let dispdata = s.display(display)?;
            Some(Self::mode_to_set(&info, dispdata, closest))
        });
        if let Some(mode) = mode {
            send_window_event(
                window,
                EventType::WINDOW_RESIZED,
                mode.hdisplay as i32,
                mode.vdisplay as i32,
            );
        }
    }

    /* This determines the size of the fb, which comes from the GBM surface
    that we create here. */
    /// Translation of `KMSDRM_CreateSurfaces()`.
    pub(crate) fn kmsdrm_create_surfaces(&self, window: WindowID) -> Result<()> {
        // !!! FIXME: I have no idea if this is a good preferred order for this list.
        const GBM_FORMATS: [(u32, &str, i32, i32, i32, i32); 26] = [
            (GBM_FORMAT_ARGB8888, "ARGB8888", 8, 8, 8, 8),
            (GBM_FORMAT_ABGR8888, "ABGR8888", 8, 8, 8, 8),
            (GBM_FORMAT_BGRA8888, "BGRA8888", 8, 8, 8, 8),
            (GBM_FORMAT_RGBA8888, "RGBA8888", 8, 8, 8, 8),
            (GBM_FORMAT_BGRX8888, "BGRX8888", 8, 8, 8, 0),
            (GBM_FORMAT_RGBX8888, "RGBX8888", 8, 8, 8, 0),
            (GBM_FORMAT_XRGB8888, "XRGB8888", 8, 8, 8, 0),
            (GBM_FORMAT_XBGR8888, "XBGR8888", 8, 8, 8, 0),
            (GBM_FORMAT_RGBA1010102, "RGBA1010102", 10, 10, 10, 2),
            (GBM_FORMAT_BGRA1010102, "BGRA1010102", 10, 10, 10, 2),
            (GBM_FORMAT_ARGB2101010, "ARGB2101010", 10, 10, 10, 2),
            (GBM_FORMAT_ABGR2101010, "ABGR2101010", 10, 10, 10, 2),
            (GBM_FORMAT_XRGB2101010, "XRGB2101010", 10, 10, 10, 0),
            (GBM_FORMAT_XBGR2101010, "XBGR2101010", 10, 10, 10, 0),
            (GBM_FORMAT_RGBX1010102, "RGBX1010102", 10, 10, 10, 0),
            (GBM_FORMAT_BGRX1010102, "BGRX1010102", 10, 10, 10, 0),
            (GBM_FORMAT_ARGB1555, "ARGB1555", 5, 5, 5, 1),
            (GBM_FORMAT_ABGR1555, "ABGR1555", 5, 5, 5, 1),
            (GBM_FORMAT_RGBA5551, "RGBA5551", 5, 5, 5, 1),
            (GBM_FORMAT_BGRA5551, "BGRA5551", 5, 5, 5, 1),
            (GBM_FORMAT_XRGB1555, "XRGB1555", 5, 5, 5, 0),
            (GBM_FORMAT_XBGR1555, "XBGR1555", 5, 5, 5, 0),
            (GBM_FORMAT_RGBX5551, "RGBX5551", 5, 5, 5, 0),
            (GBM_FORMAT_BGRX5551, "BGRX5551", 5, 5, 5, 0),
            (GBM_FORMAT_RGB565, "RGB565", 5, 6, 5, 0),
            (GBM_FORMAT_BGR565, "BGR565", 5, 6, 5, 0),
        ];

        let gbm = &self.syms.gbm;
        let info = window_info(window)?;
        let display = display_for_window(window)?;
        // (KMSDRM_GetModeToSet()'s closest mode, asked of the video core
        // here, without the driver's state)
        let closest = self.closest_for_window(display, &info);
        /* Current context passing to EGL is now done here. If something fails,
        go back to delayed SDL_EGL_MakeCurrent() call in SwapWindow. */
        let egl_context = gl::current_thread_context();

        let mut result = false;
        let mut last_error: Option<Error> = None;
        let mut resized = None;

        self.with_state(|s| {
            // If the current window already has surfaces, destroy them before creating other.
            if s.window(window).is_some_and(|w| !w.gs.is_null()) {
                self.kmsdrm_destroy_surfaces(s, window, display);
            }

            let Some((vid, dispdata, windata)) = s.parts(display, window) else {
                last_error = Some(Error::new("Invalid window"));
                return;
            };

            let mut tried_setting_format = false;
            let surface_flags = GBM_BO_USE_SCANOUT | GBM_BO_USE_RENDERING;
            for &(surface_fmt, name, r, g, b, a) in &GBM_FORMATS {
                crate::log!("KMSDRM: Trying format GBM_FORMAT_{}", name);

                // SAFETY: the device is GBM's.
                if unsafe {
                    (gbm.gbm_device_is_format_supported)(vid.gbm_dev, surface_fmt, surface_flags)
                } == 0
                {
                    crate::warn!(
                        crate::log::Category::Video,
                        "KMSDRM: GBM_FORMAT_{} is not supported.",
                        name
                    );
                    continue; // try the next one.
                }

                tried_setting_format = true;

                /* The KMSDRM backend doesn't always set the mode the higher-level code in
                SDL_video.c expects. Hulk-smash the display's current_mode to keep the
                mode that's set in sync with what SDL_video.c thinks is set

                FIXME: How do we do that now? Can we get a better idea at the higher level?
                */
                dispdata.mode = Self::mode_to_set(&info, dispdata, closest);

                let (w, h) = (dispdata.mode.hdisplay as u32, dispdata.mode.vdisplay as u32);
                // SAFETY: as above.
                windata.gs = unsafe {
                    (gbm.gbm_surface_create)(vid.gbm_dev, w, h, surface_fmt, surface_flags)
                };
                if windata.gs.is_null() && errno() == libc::ENOSYS {
                    // Try again without the scanout flags, needed on NVIDIA drivers
                    // SAFETY: as above.
                    windata.gs =
                        unsafe { (gbm.gbm_surface_create)(vid.gbm_dev, w, h, surface_fmt, 0) };
                }
                if windata.gs.is_null() {
                    let e = errno();
                    crate::warn!(
                        crate::log::Category::Video,
                        "KMSDRM: gbm_surface_create(GBM_FORMAT_{}) failed.",
                        name
                    );
                    // set error in case this was the last supposedly-supported format.
                    last_error = Some(Error::new(format!(
                        "Could not create {name} GBM surface: {}",
                        crate::core::linux::input::errno_string(e)
                    )));
                    continue; // try the next one.
                }

                /* We can't get the EGL context yet because SDL_CreateRenderer has not been called,
                but we need an EGL surface NOW, or GL won't be able to render into any surface
                and we won't see the first frame. */
                egl::set_required_visual_id(surface_fmt as i32);
                match egl::create_surface(Some(window), windata.gs.cast()) {
                    Ok(surface) if surface != EGL_NO_SURFACE => windata.egl_surface = surface,
                    other => {
                        let message = match other {
                            Err(e) => e.message().to_owned(),
                            Ok(_) => String::new(),
                        };
                        crate::warn!(
                            crate::log::Category::Video,
                            "KMSDRM: SDL_EGL_CreateSurface(GBM_FORMAT_{}) failed: {}",
                            name,
                            message
                        );
                        windata.egl_surface = EGL_NO_SURFACE;
                        // SAFETY: the surface is GBM's, not used after this.
                        unsafe { (gbm.gbm_surface_destroy)(windata.gs) };
                        windata.gs = std::ptr::null_mut();
                        last_error = Some(Error::new(message));
                        continue; // try the next one.
                    }
                }

                if let Err(e) = egl::make_current(windata.egl_surface, egl_context) {
                    crate::warn!(
                        crate::log::Category::Video,
                        "KMSDRM: SDL_EGL_MakeCurrent with GBM_FORMAT_{} failed: {}",
                        name,
                        e.message()
                    );
                    last_error = Some(e);
                    continue;
                }

                gl::with_gl_config(|c| {
                    c.red_size = r;
                    c.green_size = g;
                    c.blue_size = b;
                    c.alpha_size = a;
                });

                windata.egl_surface_dirty = false;

                result = true;

                resized = Some((dispdata.mode.hdisplay as i32, dispdata.mode.vdisplay as i32));

                break;
            }

            crate::sdl_assert!(result || windata.gs.is_null());

            if !result {
                crate::error!(
                    crate::log::Category::Video,
                    "KMSDRM: Failed to create surfaces in _any_ format!"
                );
                if !tried_setting_format {
                    // make sure there's an error string set.
                    last_error = Some(Error::new(
                        "Could not find any supported GBM surface format.",
                    ));
                }
            }
        });

        if let Some((w, h)) = resized {
            send_window_event(window, EventType::WINDOW_RESIZED, w, h);
        }

        if result {
            Ok(())
        } else {
            Err(last_error.unwrap_or_else(|| Error::new("")))
        }
    }

    /// Translation of `KMSDRM_ReleaseVT()`.
    fn kmsdrm_release_vt(&self) {
        let windows = self.with_state(|s| s.vid.windows.clone());
        for window in windows {
            let Ok(info) = window_info(window) else {
                continue;
            };
            if !info.flags.contains(WindowFlags::VULKAN) {
                let Ok(display) = display_for_window(window) else {
                    continue;
                };
                self.with_state(|s| self.kmsdrm_destroy_surfaces(s, window, display));
            }
        }
        self.with_state(|s| {
            // SAFETY: the descriptor is the card's.
            unsafe { (self.syms.drm.drmDropMaster)(s.vid.drm_fd) };
        });
    }

    /// Translation of `KMSDRM_AcquireVT()`.
    fn kmsdrm_acquire_vt(&self) {
        let windows = self.with_state(|s| {
            // SAFETY: the descriptor is the card's.
            unsafe { (self.syms.drm.drmSetMaster)(s.vid.drm_fd) };
            s.vid.windows.clone()
        });
        for window in windows {
            let Ok(info) = window_info(window) else {
                continue;
            };
            if !info.flags.contains(WindowFlags::VULKAN) {
                let _ = self.kmsdrm_create_surfaces(window);
            }
        }
    }

    /// Translation of `KMSDRM_VideoInit()`.
    pub(crate) fn kmsdrm_video_init(&self) -> Result<()> {
        let mut result = Ok(());

        crate::debug!(crate::log::Category::Video, "KMSDRM_VideoInit()");

        self.with_state(|s| {
            s.vid.video_init = false;
            s.vid.gbm_init = false;
        });

        /* Get KMSDRM resources info and store what we need. Getting and storing
        this info isn't a problem for VK compatibility.
        For VK-incompatible initializations we have KMSDRM_GBMInit(), which is
        called on window creation, and only when we know it's not a VK window. */
        if self.kmsdrm_init_displays().is_err() {
            result = Err(Error::new("error getting KMSDRM displays information"));
        }

        let _ = crate::core::linux::evdev::init();
        let release = self.this.clone();
        let acquire = self.this.clone();
        crate::core::linux::evdev::set_vt_switch_callbacks(
            Some(Box::new(move || {
                if let Some(this) = release.upgrade() {
                    this.kmsdrm_release_vt();
                }
            })),
            Some(Box::new(move || {
                if let Some(this) = acquire.upgrade() {
                    this.kmsdrm_acquire_vt();
                }
            })),
        );

        self.with_state(|s| s.vid.video_init = true);

        result
    }

    /* The internal pointers, like dispdata, viddata, windata, etc...
    are freed by SDL internals, so not our job. */
    /// Translation of `KMSDRM_VideoQuit()`.
    pub(crate) fn kmsdrm_video_quit(&self) {
        self.kmsdrm_deinit_displays();

        crate::core::linux::evdev::set_vt_switch_callbacks(None, None);
        crate::core::linux::evdev::quit();

        self.with_state(|s| {
            // Clear out the window list
            s.vid.windows.clear();
            s.vid.video_init = false;
            // (the displays' internals, freed with the displays upstream)
            s.displays.clear();
            s.vid.mouse_init = false;
        });
    }

    /// Read modes from the connector modes, and store them in
    /// display->display_modes. Translation of `KMSDRM_GetDisplayModes()`.
    pub(crate) fn kmsdrm_get_display_modes(&self, display: DisplayID) {
        let modes: Vec<drmModeModeInfo> = self.with_state(|s| {
            s.display(display).map_or_else(Vec::new, |d| {
                // SAFETY: the connector's modes are libdrm's.
                unsafe {
                    let conn = &*d.connector.connector;
                    c_array(conn.modes, conn.count_modes).to_vec()
                }
            })
        });

        let mut added = Vec::new();
        for (i, m) in modes.iter().enumerate() {
            let (num, den) = calculate_refresh_rate(m);
            let mode = DisplayMode {
                w: m.hdisplay as i32,
                h: m.vdisplay as i32,
                refresh_rate_numerator: num,
                refresh_rate_denominator: den,
                format: PixelFormat::ARGB8888,
                ..Default::default()
            };

            if add_fullscreen_display_mode(display, &mode) {
                added.push((mode, i as i32));
            }
        }

        self.with_state(|s| {
            if let Some(d) = s.display(display) {
                d.modedata.extend(added);
            }
        });
    }

    /// Translation of `KMSDRM_SetDisplayMode()`.
    pub(crate) fn kmsdrm_set_display_mode(
        &self,
        display: DisplayID,
        mode: &DisplayMode,
    ) -> Result<()> {
        /* Set the dispdata->mode to the new mode and leave actual modesetting
        pending to be done on SwapWindow() via drmModeSetCrtc() */

        // (the desktop mode's SDL_DisplayModeData, or a fullscreen mode's)
        let is_desktop = display::desktop_display_mode(display).is_ok_and(|d| d == *mode);
        let key = mode_key(mode);
        let windows = self.with_state(|s| {
            // Don't do anything if we are in Vulkan mode.
            if s.vid.vulkan_mode {
                return Ok(None);
            }

            let Some(dispdata) = s.display(display) else {
                return Err(Error::new("Invalid display"));
            };
            let mode_index = if is_desktop {
                Some(dispdata.desktop_mode_index)
            } else {
                dispdata
                    .modedata
                    .iter()
                    .find(|(m, _)| mode_key(m) == key)
                    .map(|&(_, i)| i)
            };
            let Some(mode_index) = mode_index else {
                return Err(Error::new("Mode doesn't have an associated index"));
            };

            /* Take note of the new mode to be set, and leave the CRTC modeset pending
            so it's done in SwapWindow. */
            // SAFETY: the connector's modes are libdrm's.
            dispdata.fullscreen_mode = unsafe {
                let conn = &*dispdata.connector.connector;
                c_array(conn.modes, conn.count_modes)[mode_index as usize]
            };

            Ok(Some(s.vid.windows.clone()))
        })?;

        for window in windows.unwrap_or_default() {
            self.kmsdrm_dirty_surfaces(window);
        }

        Ok(())
    }

    /// Translation of `KMSDRM_DestroyWindow()`.
    pub(crate) fn kmsdrm_destroy_window(&self, window: WindowID) {
        let Ok(info) = window_info(window) else {
            return;
        };
        let display = display_for_window(window).ok();
        let is_vulkan = info.flags.contains(WindowFlags::VULKAN); // Is this a VK window?

        let gbm_window = self.with_state(|s| {
            s.window(window)?;

            // restore vrr state
            if let Some(display) = display {
                let drm_fd = s.vid.drm_fd;
                if let Some(dispdata) = s.display(display) {
                    // SAFETY: the display's CRTC is libdrm's.
                    let crtc_id = unsafe { (*dispdata.crtc.crtc).crtc_id };
                    kmsdrm_crtc_set_vrr(&self.syms.drm, drm_fd, crtc_id, dispdata.saved_vrr);
                }
            }

            if !is_vulkan && s.vid.gbm_init {
                Some(true)
            } else {
                // If we were in Vulkan mode, get out of it.
                if s.vid.vulkan_mode {
                    s.vid.vulkan_mode = false;
                }
                Some(false)
            }
        });
        let Some(gbm_window) = gbm_window else {
            return;
        };

        if gbm_window {
            if let Some(display) = display {
                // Destroy cursor GBM BO of the display of this window.
                self.kmsdrm_destroy_cursor_bo(display);

                // Destroy GBM surface and buffers.
                self.with_state(|s| self.kmsdrm_destroy_surfaces(s, window, display));
            }

            /* Unload library and deinit GBM, but only if this is the last window.
            Note that this is the right comparison because num_windows could be 1
            if there is a complete window, or 0 if we got here from SDL_CreateWindow()
            because KMSDRM_CreateWindow() returned an error so the window wasn't
            added to the windows list. */
            if self.with_state(|s| s.vid.windows.len() <= 1) {
                // Unload EGL/GL library and free egl_data.
                if egl::is_loaded() {
                    egl::unload_library();
                    gl::set_driver_loaded(|_| 0);
                }

                // Free display plane, and destroy GBM device.
                if let Some(display) = display {
                    self.with_state(|s| self.kmsdrm_gbm_deinit(s, display));
                }
            }
        }

        self.with_state(|s| {
            /********************************************/
            // Remove from the internal SDL window list
            /********************************************/
            if let Some(i) = s.vid.windows.iter().position(|&w| w == window) {
                s.vid.windows.remove(i);
            }

            /*********************************************************************/
            // Free the window internal. Bye bye, surface and buffer pointers!
            /*********************************************************************/
            s.windata.retain(|(w, _)| *w != window);
        });
    }

    /**********************************************************************/
    // We simply IGNORE if it's a fullscreen window, window->flags don't
    // reflect it: if it's fullscreen, KMSDRM_SetWindowFullscreen() will
    // be called by SDL later, and we can manage it there.
    /**********************************************************************/
    /// Translation of `KMSDRM_CreateWindow()`.
    pub(crate) fn kmsdrm_create_window(&self, window: WindowID) -> Result<()> {
        let info = window_info(window)?;
        let display = display_for_window(window)?;
        let is_vulkan = info.flags.contains(WindowFlags::VULKAN); // Is this a VK window?

        // Allocate window internal data
        // Do we want a double buffering scheme to get low video lag?
        let double_buffer = hints::get_bool(hints::VIDEO_DOUBLE_BUFFER, false);
        let vulkan_mode = self.with_state(|s| {
            // Setup driver data for this window
            s.windata.push((
                window,
                Box::new(WindowData {
                    gs: std::ptr::null_mut(),
                    bo: std::ptr::null_mut(),
                    next_bo: std::ptr::null_mut(),
                    waiting_for_flip: Arc::new(AtomicBool::new(false)),
                    double_buffer,
                    egl_surface: EGL_NO_SURFACE,
                    egl_surface_dirty: false,
                    swap_window: None,
                }),
            ));
            s.vid.vulkan_mode // Do we have any Vulkan windows?
        });

        if !is_vulkan && !vulkan_mode {
            // NON-Vulkan block.

            /* Maybe you didn't ask for an OPENGL window, but that's what you will get.
            See following comments on why. */
            with_window(window, |w| w.core.flags |= WindowFlags::OPENGL)?;

            if !self.with_state(|s| s.vid.gbm_init) {
                /* After SDL_CreateWindow, most SDL programs will do SDL_CreateRenderer(),
                which will in turn call GL_CreateRenderer() or GLES2_CreateRenderer().
                In order for the GL_CreateRenderer() or GLES2_CreateRenderer() call to
                succeed without an unnecessary window re-creation, we must:
                -Mark the window as being OPENGL
                -Load the GL library (which can't be done until the GBM device has been
                 created, so we have to do it here instead of doing it on VideoInit())
                 and mark it as loaded by setting gl_config.driver_loaded to 1.
                So if you ever see KMSDRM_CreateWindow() to be called two times in tests,
                don't be shy to debug GL_CreateRenderer() or GLES2_CreateRenderer()
                to find out why!
                */

                /* Reopen FD, create gbm dev, setup display plane, etc,.
                but only when we come here for the first time,
                and only if it's not a VK window. */
                if self
                    .with_state(|s| self.kmsdrm_gbm_init(s, display))
                    .is_err()
                {
                    return Err(Error::new("Can't init GBM on window creation."));
                }
            }

            /* Manually load the GL library. KMSDRM_EGL_LoadLibrary() has already
            been called by SDL_CreateWindow() but we don't do anything there,
            our KMSDRM_EGL_LoadLibrary() is a dummy precisely to be able to load it here.
            If we let SDL_CreateWindow() load the lib, it would be loaded
            before we call KMSDRM_GBMInit(), causing all GLES programs to fail. */
            if !egl::is_loaded() {
                let egl_display = self.with_state(|s| s.vid.gbm_dev).cast::<c_void>();
                if egl::load_library(None, egl_display).is_err() {
                    // Try again with OpenGL ES 2.0
                    gl::with_gl_config(|c| {
                        c.profile_mask = gl::GL_CONTEXT_PROFILE_ES;
                        c.major_version = 2;
                        c.minor_version = 0;
                    });
                    if egl::load_library(None, egl_display).is_err() {
                        return Err(Error::new("Can't load EGL/GL library on window creation."));
                    }
                }

                gl::set_driver_loaded(|_| 1);
            }

            /* Create the cursor BO for the display of this window,
            now that we know this is not a VK window. */
            let _ = self.kmsdrm_create_cursor_bo(display);

            /* Create and set the default cursor for the display
            of this window, now that we know this is not a VK window. */
            self.kmsdrm_init_mouse(display);

            /* The FULLSCREEN flags are cut out from window->flags at this point,
            so we can't know if a window is fullscreen or not, hence all windows
            are considered "windowed" at this point of their life.
            If a window is fullscreen, SDL internals will call
            KMSDRM_SetWindowFullscreen() to reconfigure it if necessary. */
            let mode =
                self.kmsdrm_get_closest_display_mode(display, info.windowed_w, info.windowed_h);

            self.with_state(|s| {
                if let Some(dispdata) = s.display(display) {
                    // SAFETY: the connector's modes are libdrm's.
                    let modes = unsafe {
                        let conn = &*dispdata.connector.connector;
                        c_array(conn.modes, conn.count_modes)
                    };
                    dispdata.fullscreen_mode = match mode.and_then(|i| modes.get(i)) {
                        Some(m) => *m,
                        None => dispdata.original_mode,
                    };
                }
            });

            /* Create the window surfaces with the size we have just chosen.
            Needs the window driverdata in place. */
            self.kmsdrm_create_surfaces(window)?;
        } // NON-Vulkan block ends.

        /* Add window to the internal list of tracked windows. Note, while it may
        seem odd to support multiple fullscreen windows, some apps create an
        extra window as a dummy surface when working with multiple contexts */
        let (devindex, drm_fd, gbm_dev) = self.with_state(|s| {
            s.vid.windows.push(window);

            // If we have just created a Vulkan window, establish that we are in Vulkan mode now.
            s.vid.vulkan_mode = is_vulkan;

            (s.vid.devindex, s.vid.drm_fd, s.vid.gbm_dev)
        });

        let props = with_window(window, |w| w.properties())?;
        let _ = props.set(
            crate::video::window::PROP_WINDOW_KMSDRM_DEVICE_INDEX_NUMBER,
            devindex as i64,
        );
        let _ = props.set(
            crate::video::window::PROP_WINDOW_KMSDRM_DRM_FD_NUMBER,
            drm_fd as i64,
        );
        let _ = props.set(
            crate::video::window::PROP_WINDOW_KMSDRM_GBM_DEVICE_POINTER,
            gbm_dev as usize as i64,
        );

        if !info.flags.contains(WindowFlags::NOT_FOCUSABLE) {
            /* Focus on the newly created window.
            SDL_SetMouseFocus() also takes care of calling KMSDRM_ShowCursor() if necessary. */
            crate::events::mouse::set_mouse_focus(Some(window));
            let _ = crate::events::keyboard::set_keyboard_focus(Some(window));
        }

        // Tell the app that the window has moved to top-left.
        let display_bounds = display_for_window(window)
            .and_then(display_bounds)
            .unwrap_or_default();
        send_window_event(
            window,
            EventType::WINDOW_MOVED,
            display_bounds.x,
            display_bounds.y,
        );

        /* Allocated windata will be freed in KMSDRM_DestroyWindow,
        and KMSDRM_DestroyWindow() will be called by SDL_CreateWindow()
        if we return error on any of the previous returns of the function. */
        Ok(())
    }

    /// Translation of `KMSDRM_SetWindowSize()`.
    pub(crate) fn kmsdrm_set_window_size(&self, window: WindowID) {
        if !self.with_state(|s| s.vid.vulkan_mode) {
            self.kmsdrm_dirty_surfaces(window);
        }
    }

    /// Translation of `KMSDRM_SetWindowFullscreen()`.
    pub(crate) fn kmsdrm_set_window_fullscreen(&self, window: WindowID) {
        if !self.with_state(|s| s.vid.vulkan_mode) {
            self.kmsdrm_dirty_surfaces(window);
        }
    }
}
