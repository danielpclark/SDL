// Rust translation of src/video/wayland/SDL_waylandwindow.c and
// SDL_waylandwindow.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Wayland windows: xdg-shell toplevels and popups (or libdecor frames),
//! their geometry (viewports, fractional scaling, the mask subsurface),
//! decorations, fullscreen/maximize/minimize, activation, icons, idle
//! inhibition and session management.
//!
//! The driver data of a window (`SDL_WindowData`) is [`WaylandWindowData`],
//! kept in [`VideoData::windows`]. The window itself (`SDL_Window`) belongs
//! to the video core: its state is read into a [`Win`] snapshot before the
//! driver data is borrowed, and the video core is only called with nothing
//! borrowed (as it may call back into the driver).
//!
//! libdecor is never called with the driver data borrowed either: a libdecor
//! call may run the frame callbacks (`commit`) synchronously.
//!
//! OpenGL windows get a `wl_egl_window` with an EGL surface on it, and the
//! GLES swap frame callback on its own queue that `SwapWindow` waits on (see
//! [`super::opengles`]).

use std::ffi::{c_char, c_int, c_void};
use std::os::fd::RawFd;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};

use super::client::{
    catch_callback, resume_pending_panic, AsProxy, Conn, EventQueue, Fixed, Obj, Proxy,
};
use super::color::ColorInfoState;
use super::framebuffer::Framebuffer;
use super::keyboard::TextInputProps;
use super::protocols::alpha_modifier_v1::*;
use super::protocols::color_management_v1::*;
use super::protocols::fractional_scale_v1::*;
use super::protocols::frog_color_management_v1::*;
use super::protocols::idle_inhibit_unstable_v1::*;
use super::protocols::viewporter::*;
use super::protocols::wayland::*;
use super::protocols::xdg_activation_v1::*;
use super::protocols::xdg_decoration_unstable_v1::*;
use super::protocols::xdg_dialog_v1::*;
use super::protocols::xdg_foreign_unstable_v2::*;
use super::protocols::xdg_session_management_v1::*;
use super::protocols::xdg_shell::*;
use super::protocols::xdg_toplevel_icon_v1::*;
use super::protocols::xdg_toplevel_tag_v1::*;
use super::shmbuffer::{wayland_create_single_pixel_buffer, ShmPool};
use super::sys::*;
use super::video::{
    set_ptr_prop, Globals, VideoData, WaylandVideo, SDL_WAYLAND_OUTPUT_TAG, SDL_WAYLAND_SURFACE_TAG,
};
use super::wldyn::{Libdecor, WaylandSyms};
use crate::error::{Error, Result};
use crate::events::window::{send_window_event, WindowFlags};
use crate::events::{keyboard, DisplayID, EventType, WindowID};
use crate::hints;
use crate::properties::Properties;
use crate::video::core::{display_for_fullscreen_window, update_fullscreen_mode, with_window};
use crate::video::egl;
use crate::video::gl::{with_gl_config, EglSurface};
use crate::video::sysvideo::{
    DisplayMode, FlashOperation, FullscreenOp, FullscreenResult, HitTestResult,
};
use crate::video::window::{
    should_focus_popup, should_relinquish_popup_focus,
    PROP_WINDOW_CREATE_WAYLAND_CREATE_EGL_WINDOW_BOOLEAN,
    PROP_WINDOW_CREATE_WAYLAND_ENABLE_INSETS_BOOLEAN,
    PROP_WINDOW_CREATE_WAYLAND_SURFACE_ROLE_CUSTOM_BOOLEAN,
    PROP_WINDOW_CREATE_WAYLAND_WINDOW_ID_STRING, PROP_WINDOW_CREATE_WAYLAND_WL_SURFACE_POINTER,
    PROP_WINDOW_WAYLAND_BORDER_INSET_BOTTOM_NUMBER, PROP_WINDOW_WAYLAND_BORDER_INSET_LEFT_NUMBER,
    PROP_WINDOW_WAYLAND_BORDER_INSET_RIGHT_NUMBER, PROP_WINDOW_WAYLAND_BORDER_INSET_TOP_NUMBER,
    PROP_WINDOW_WAYLAND_DISPLAY_POINTER, PROP_WINDOW_WAYLAND_EGL_WINDOW_POINTER,
    PROP_WINDOW_WAYLAND_SURFACE_POINTER, PROP_WINDOW_WAYLAND_VIEWPORT_POINTER,
    PROP_WINDOW_WAYLAND_WINDOW_ID_STRING, PROP_WINDOW_WAYLAND_XDG_POPUP_POINTER,
    PROP_WINDOW_WAYLAND_XDG_POSITIONER_POINTER, PROP_WINDOW_WAYLAND_XDG_SURFACE_POINTER,
    PROP_WINDOW_WAYLAND_XDG_TOPLEVEL_EXPORT_HANDLE_STRING,
    PROP_WINDOW_WAYLAND_XDG_TOPLEVEL_POINTER,
};
use crate::video::{PixelFormat, Rect, Surface};

// ---------------------------------------------------------------------------
// The window state enums and flags
// ---------------------------------------------------------------------------

/// The role of the window's surface (`shell_surface_type`).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum ShellSurfaceType {
    #[default]
    Unknown,
    XdgToplevel,
    XdgPopup,
    Libdecor,
    Custom,
}

/// Where the window is in being shown (`shell_surface_status`).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum ShellSurfaceStatus {
    #[default]
    Hidden,
    WaitingForConfigure,
    WaitingForFrame,
    ShowPending,
    Shown,
}

// The window manager capabilities (`wm_caps`).
pub(crate) const WAYLAND_WM_CAPS_WINDOW_MENU: u32 = 0x01;
pub(crate) const WAYLAND_WM_CAPS_MAXIMIZE: u32 = 0x02;
pub(crate) const WAYLAND_WM_CAPS_FULLSCREEN: u32 = 0x04;
pub(crate) const WAYLAND_WM_CAPS_MINIMIZE: u32 = 0x08;
pub(crate) const WAYLAND_WM_CAPS_ALL: u32 = WAYLAND_WM_CAPS_WINDOW_MENU
    | WAYLAND_WM_CAPS_MAXIMIZE
    | WAYLAND_WM_CAPS_FULLSCREEN
    | WAYLAND_WM_CAPS_MINIMIZE;

// The edges the window can't be resized from (`toplevel_constraints`).
pub(crate) const WAYLAND_TOPLEVEL_CONSTRAINED_LEFT: u32 = 0x01;
pub(crate) const WAYLAND_TOPLEVEL_CONSTRAINED_RIGHT: u32 = 0x02;
pub(crate) const WAYLAND_TOPLEVEL_CONSTRAINED_TOP: u32 = 0x04;
pub(crate) const WAYLAND_TOPLEVEL_CONSTRAINED_BOTTOM: u32 = 0x08;

// The axes of an interactive resize (`resize_edge`).
const WAYLAND_RESIZE_EDGE_LR: u32 = 0x01;
const WAYLAND_RESIZE_EDGE_TB: u32 = 0x02;
const WAYLAND_RESIZE_EDGE_CORNER: u32 = WAYLAND_RESIZE_EDGE_LR | WAYLAND_RESIZE_EDGE_TB;

/// `SDL_MAX_SINT16`
const MAX_SINT16: i64 = i16::MAX as i64;

// ---------------------------------------------------------------------------
// The driver data of a window
// ---------------------------------------------------------------------------

/// The pointer scale (`pointer_scale`).
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub(crate) struct PointerScale {
    pub(crate) x: f64,
    pub(crate) y: f64,
}

/// The in-flight window size request (`requested`).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) struct RequestedSize {
    /// The requested logical window size.
    pub(crate) logical_width: i32,
    pub(crate) logical_height: i32,

    /// The size of the window in pixels, when using screen space scaling.
    pub(crate) pixel_width: i32,
    pub(crate) pixel_height: i32,
}

/// The current size of the window and drawable backing store (`current`).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) struct CurrentSize {
    /// The size of the underlying window.
    pub(crate) logical_width: i32,
    pub(crate) logical_height: i32,

    /// The size of the window backbuffer in pixels.
    pub(crate) pixel_width: i32,
    pub(crate) pixel_height: i32,

    /// The dimensions of the active viewport, in logical units.
    pub(crate) viewport_width: i32,
    pub(crate) viewport_height: i32,
}

/// The last compositor requested parameters; used for deduplication of
/// window geometry configuration (`last_configure`).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) struct LastConfigure {
    pub(crate) width: i32,
    pub(crate) height: i32,
    pub(crate) insets_apply: bool,
}

/// System enforced window size limits (`system_limits`).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) struct SystemLimits {
    /// Minimum allowed logical window size.
    pub(crate) min_width: i32,
    pub(crate) min_height: i32,
}

/// `toplevel_bounds`
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) struct ToplevelBounds {
    pub(crate) width: i32,
    pub(crate) height: i32,
}

/// The subsurface masking the area outside the viewport (`mask`).
#[derive(Debug, Default)]
pub(crate) struct Mask {
    pub(crate) surface: Option<Proxy<WlSurface>>,
    pub(crate) subsurface: Option<Proxy<WlSubsurface>>,
    pub(crate) buffer: Option<Proxy<WlBuffer>>,
    pub(crate) viewport: Option<Proxy<WpViewport>>,

    pub(crate) offset_x: i32,
    pub(crate) offset_y: i32,

    pub(crate) mapped: bool,
    pub(crate) opaque: bool,
}

/// The border insets from the window properties; they only apply to
/// xdg-toplevel windows (`BorderInsets`).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
struct BorderInsets {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

/// The `wl_surface` of a window: ours (destroyed with the window), or the
/// application's (external, never destroyed).
pub(crate) struct WindowSurface {
    proxy: std::mem::ManuallyDrop<Proxy<WlSurface>>,
    external: bool,
}

impl std::fmt::Debug for WindowSurface {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:?}{}",
            *self.proxy,
            if self.external { " (external)" } else { "" }
        )
    }
}

impl WindowSurface {
    /// The surface as an object.
    pub(crate) fn obj(&self) -> Obj<'_, WlSurface> {
        self.proxy.obj()
    }
}

impl Drop for WindowSurface {
    fn drop(&mut self) {
        // SAFETY: the proxy is taken once, here.
        let proxy = unsafe { std::mem::ManuallyDrop::take(&mut self.proxy) };
        if self.external {
            // (the application's surface: forget it without destroying it)
            proxy.forget_destroyed();
        } else {
            drop(proxy);
        }
    }
}

/// What libdecor hands the frame callbacks (`user_data`: upstream passes
/// the `SDL_WindowData *`).
struct FrameUserData {
    video: Weak<WaylandVideo>,
    window: WindowID,
}

/// A libdecor frame (`shell_surface.libdecor.frame`), unreferenced on drop.
pub(crate) struct LibdecorFrame {
    raw: NonNull<libdecor_frame>,
    syms: Arc<WaylandSyms>,
    /// The data of the frame callbacks; outlives the frame.
    _user_data: Box<FrameUserData>,
}

// SAFETY: libdecor frames are only used on the video thread; the pointer
// moves with the window data.
unsafe impl Send for LibdecorFrame {}

impl std::fmt::Debug for LibdecorFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "libdecor_frame@{:p}", self.raw)
    }
}

impl LibdecorFrame {
    /// The `struct libdecor_frame *`.
    pub(crate) fn raw(&self) -> *mut libdecor_frame {
        self.raw.as_ptr()
    }
}

impl Drop for LibdecorFrame {
    fn drop(&mut self) {
        if let Some(l) = &self.syms.libdecor {
            // SAFETY: the frame was made by libdecor_decorate and is
            // unreferenced once; its callbacks' data outlives it.
            unsafe { (l.libdecor_frame_unref)(self.raw()) };
        }
    }
}

/// The shell surface objects of a window (`shell_surface`: upstream's
/// union of the libdecor frame and the xdg-shell objects).
#[derive(Debug, Default)]
pub(crate) struct ShellSurface {
    pub(crate) libdecor_frame: Option<LibdecorFrame>,
    pub(crate) xdg_surface: Option<Proxy<XdgSurface>>,
    pub(crate) xdg_toplevel: Option<Proxy<XdgToplevel>>,
    pub(crate) xdg_popup: Option<Proxy<XdgPopup>>,
    pub(crate) xdg_positioner: Option<Proxy<XdgPositioner>>,
    pub(crate) serial: u32,
}

/// A `wl_egl_window` (`egl_window`), destroyed on drop.
pub(crate) struct EglWindow {
    raw: NonNull<wl_egl_window>,
    syms: Arc<WaylandSyms>,
}

// SAFETY: the EGL window is only used on the video thread; the pointer
// moves with the window data.
unsafe impl Send for EglWindow {}

impl std::fmt::Debug for EglWindow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "wl_egl_window@{:p}", self.raw)
    }
}

impl EglWindow {
    /// `WAYLAND_wl_egl_window_create()` (`None`: NULL).
    fn create(surface: Obj<'_, WlSurface>, width: i32, height: i32) -> Option<EglWindow> {
        let syms = surface.conn().syms().clone();
        // SAFETY: the surface is alive; libwayland-egl takes no ownership.
        let raw = unsafe { (syms.egl.wl_egl_window_create)(surface.raw(), width, height) };
        NonNull::new(raw).map(|raw| EglWindow { raw, syms })
    }

    /// The `struct wl_egl_window *`.
    pub(crate) fn raw(&self) -> *mut wl_egl_window {
        self.raw.as_ptr()
    }

    /// `WAYLAND_wl_egl_window_resize()`
    fn resize(&self, width: i32, height: i32, dx: i32, dy: i32) {
        // SAFETY: the EGL window is alive.
        unsafe { (self.syms.egl.wl_egl_window_resize)(self.raw(), width, height, dx, dy) };
    }
}

impl Drop for EglWindow {
    fn drop(&mut self) {
        // SAFETY: the EGL window was made by wl_egl_window_create and is
        // destroyed once, here (after the EGL surface on it).
        unsafe { (self.syms.egl.wl_egl_window_destroy)(self.raw()) };
    }
}

/// The GLES swap frame callback of an OpenGL window, on its own queue
/// (`gles_swap_frame_callback`, `gles_swap_frame_event_queue` and
/// `gles_swap_frame_surface_wrapper`), shared with the callback's listener;
/// destroyed in that order on drop.
#[derive(Debug)]
pub(crate) struct GlesSwapFrame {
    callback: Mutex<Option<Proxy<WlCallback>>>,
    surface_wrapper: Proxy<WlSurface>,
    pub(crate) event_queue: EventQueue,
    /// The window's `swap_interval_ready`.
    swap_interval_ready: Arc<AtomicI32>,
}

impl GlesSwapFrame {
    /// The queue, the surface wrapper on it and the first frame callback
    /// (`None` if the queue can't be made).
    fn new(
        conn: &Arc<Conn>,
        surface: Obj<'_, WlSurface>,
        swap_interval_ready: Arc<AtomicI32>,
    ) -> Option<Arc<GlesSwapFrame>> {
        // (upstream's queue has no name)
        let event_queue = conn.create_queue(c"SDL GLES Swap Frame Queue")?;
        let surface_wrapper = surface.create_wrapper(&event_queue);
        let frame = Arc::new(GlesSwapFrame {
            callback: Mutex::new(None),
            surface_wrapper,
            event_queue,
            swap_interval_ready,
        });
        let cb = frame.new_callback();
        *frame.lock_callback() = Some(cb);
        Some(frame)
    }

    fn lock_callback(&self) -> std::sync::MutexGuard<'_, Option<Proxy<WlCallback>>> {
        self.callback.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A frame callback on the queue (`gles_swap_frame_listener`).
    fn new_callback(self: &Arc<Self>) -> Proxy<WlCallback> {
        let mut cb = self.surface_wrapper.frame();
        let weak = Arc::downgrade(self);
        cb.listen(move |_, _| {
            if let Some(frame) = weak.upgrade() {
                frame.gles_swap_frame_done();
            }
        });
        cb
    }

    /// Translation of `gles_swap_frame_done()`.
    fn gles_swap_frame_done(self: &Arc<Self>) {
        self.swap_interval_ready.store(1, Ordering::SeqCst); // mark window as ready to present again.

        // reset this callback to fire again once a new frame was presented and compositor wants the next one.
        let cb = self.new_callback();
        let old = self.lock_callback().replace(cb);
        drop(old);
    }
}

/// The driver data of a window. Translation of `struct SDL_WindowData`.
#[derive(Debug)]
pub(crate) struct WaylandWindowData {
    /// The window (`sdlwindow`).
    pub(crate) sdlwindow: WindowID,
    pub(crate) surface: WindowSurface,
    pub(crate) surface_frame_callback: Option<Proxy<WlCallback>>,
    pub(crate) gles_swap_frame: Option<Arc<GlesSwapFrame>>,

    pub(crate) shell_surface: ShellSurface,
    pub(crate) shell_surface_type: ShellSurfaceType,
    pub(crate) shell_surface_status: ShellSurfaceStatus,
    pub(crate) wm_caps: u32,
    pub(crate) toplevel_constraints: u32,
    pub(crate) resize_edge: u32,

    pub(crate) server_decoration: Option<Proxy<ZxdgToplevelDecorationV1>>,
    pub(crate) idle_inhibitor: Option<Proxy<ZwpIdleInhibitorV1>>,
    pub(crate) activation_token: Option<Proxy<XdgActivationTokenV1>>,
    pub(crate) viewport: Option<Proxy<WpViewport>>,
    pub(crate) fractional_scale: Option<Proxy<WpFractionalScaleV1>>,
    pub(crate) exported: Option<Proxy<ZxdgExportedV2>>,
    pub(crate) xdg_dialog_v1: Option<Proxy<XdgDialogV1>>,
    pub(crate) wp_alpha_modifier_surface_v1: Option<Proxy<WpAlphaModifierSurfaceV1>>,
    pub(crate) xdg_toplevel_icon_v1: Option<Proxy<XdgToplevelIconV1>>,
    pub(crate) frog_color_managed_surface: Option<Proxy<FrogColorManagedSurface>>,
    pub(crate) wp_color_management_surface_feedback:
        Option<Proxy<WpColorManagementSurfaceFeedbackV1>>,
    pub(crate) xdg_toplevel_session: Option<Proxy<XdgToplevelSessionV1>>,
    pub(crate) egl_window: Option<EglWindow>,
    pub(crate) egl_surface: Option<EglSurface>,

    pub(crate) color_info_state: Option<ColorInfoState>,

    /// The outputs the surface is on, by registry name (`outputs`).
    pub(crate) outputs: Vec<u32>,

    pub(crate) app_id: String,
    pub(crate) session_id: Option<String>,
    pub(crate) scale_factor: f64,

    pub(crate) icon_buffers: Vec<Proxy<WlBuffer>>,

    // Keyboard, pointer, and touch focus refcount.
    pub(crate) keyboard_focus_count: i32,
    pub(crate) pointer_focus_count: i32,
    pub(crate) active_touch_count: i32,

    pub(crate) pointer_scale: PointerScale,

    pub(crate) swap_interval_ready: Arc<AtomicI32>,

    /// The in-flight window size request.
    pub(crate) requested: RequestedSize,
    /// The current size of the window and drawable backing store.
    pub(crate) current: CurrentSize,
    pub(crate) last_configure: LastConfigure,
    pub(crate) system_limits: SystemLimits,
    pub(crate) toplevel_bounds: ToplevelBounds,
    pub(crate) mask: Mask,
    pub(crate) text_input_props: TextInputProps,

    pub(crate) explicit_geometry: Rect,
    pub(crate) last_display_id: DisplayID,
    pub(crate) pending_state_deadline_count: i32,
    pub(crate) last_focus_event_time_ns: u64,
    pub(crate) last_resize_event_time_ns: u64,
    pub(crate) icc_fd: RawFd,
    pub(crate) icc_size: u32,
    pub(crate) floating: bool,
    pub(crate) suspended: bool,
    pub(crate) resizing: bool,
    pub(crate) active: bool,
    pub(crate) pending_config_ack: bool,
    pub(crate) pending_state_commit: bool,
    pub(crate) limits_changed: bool,
    pub(crate) is_fullscreen: bool,
    pub(crate) fullscreen_exclusive: bool,
    pub(crate) drop_fullscreen_requests: bool,
    pub(crate) showing_window: bool,
    pub(crate) fullscreen_was_positioned: bool,
    pub(crate) show_hide_sync_required: bool,
    pub(crate) scale_to_display: bool,
    pub(crate) reparenting_required: bool,
    pub(crate) double_buffer: bool,
    pub(crate) accepts_drag_and_drop: bool,
    pub(crate) enable_insets: bool,

    pub(crate) hit_test_result: HitTestResult,

    /// The window framebuffer (not in upstream, see framebuffer.rs).
    pub(crate) framebuffer: Option<Framebuffer>,
}

impl WaylandWindowData {
    /// The surface of the window.
    pub(crate) fn surface(&self) -> Obj<'_, WlSurface> {
        self.surface.obj()
    }

    /// The address of the surface of the window, which identifies it.
    pub(crate) fn surface_id(&self) -> usize {
        self.surface.obj().raw() as usize
    }
}

impl Drop for WaylandWindowData {
    /// The destruction part of `Wayland_DestroyWindow()`, in its order.
    fn drop(&mut self) {
        self.framebuffer.take();
        self.mask.viewport.take();
        self.mask.buffer.take();
        self.mask.subsurface.take();
        self.mask.surface.take();
        if let Some(egl_surface) = self.egl_surface.take() {
            egl::destroy_surface(egl_surface.as_ptr());
        }
        self.egl_window.take();
        self.idle_inhibitor.take();
        self.activation_token.take();
        self.viewport.take();
        self.fractional_scale.take();
        self.wp_alpha_modifier_surface_v1.take();
        self.frog_color_managed_surface.take();
        if self.wp_color_management_surface_feedback.is_some() {
            self.color_info_state.take();
            self.wp_color_management_surface_feedback.take();
        }
        self.gles_swap_frame.take();
        // (the shell surface objects are gone if the window was hidden first,
        // as the video core does; otherwise they go before their surface)
        self.server_decoration.take();
        self.exported.take();
        self.xdg_dialog_v1.take();
        self.xdg_toplevel_session.take();
        self.surface_frame_callback.take();
        let ShellSurface {
            libdecor_frame,
            xdg_surface,
            xdg_toplevel,
            xdg_popup,
            xdg_positioner,
            ..
        } = std::mem::take(&mut self.shell_surface);
        drop((
            xdg_popup,
            xdg_positioner,
            xdg_toplevel,
            xdg_surface,
            libdecor_frame,
        ));
        // (the surface field is dropped after this: destroyed unless external)
        self.xdg_toplevel_icon_v1.take();
        self.icon_buffers.clear();
        // FIXME (upstream): the ICC profile descriptor (icc_fd) isn't closed.
    }
}

// ---------------------------------------------------------------------------
// The video core's window state
// ---------------------------------------------------------------------------

/// The state of a window the driver reads from the video core (the
/// `SDL_Window` fields), read at once.
#[derive(Clone)]
pub(crate) struct Win {
    pub(crate) id: WindowID,
    pub(crate) flags: WindowFlags,
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) w: i32,
    pub(crate) h: i32,
    pub(crate) pending: Rect,
    pub(crate) floating: Rect,
    pub(crate) windowed: Rect,
    pub(crate) min_w: i32,
    pub(crate) min_h: i32,
    pub(crate) max_w: i32,
    pub(crate) max_h: i32,
    pub(crate) min_aspect: f32,
    pub(crate) max_aspect: f32,
    pub(crate) opacity: f32,
    pub(crate) fullscreen_exclusive: bool,
    pub(crate) current_fullscreen_mode: DisplayMode,
    pub(crate) undefined_x: bool,
    pub(crate) undefined_y: bool,
    pub(crate) last_position_pending: bool,
    pub(crate) tiled: bool,
    pub(crate) parent: Option<WindowID>,
    pub(crate) children: Vec<WindowID>,
    pub(crate) constrain_popup: bool,
    pub(crate) props: Properties,
}

impl Win {
    /// Translation of `SDL_WINDOW_IS_POPUP()`.
    pub(crate) fn is_popup(&self) -> bool {
        self.flags
            .intersects(WindowFlags::TOOLTIP | WindowFlags::POPUP_MENU)
    }
}

/// Read the state of a window.
pub(crate) fn snapshot(window: WindowID) -> Option<Win> {
    with_window(window, |w| Win {
        id: window,
        flags: w.core.flags,
        x: w.core.x,
        y: w.core.y,
        w: w.core.w,
        h: w.core.h,
        pending: w.pending,
        floating: w.core.floating,
        windowed: w.core.windowed,
        min_w: w.core.min_w,
        min_h: w.core.min_h,
        max_w: w.core.max_w,
        max_h: w.core.max_h,
        min_aspect: w.min_aspect,
        max_aspect: w.max_aspect,
        opacity: w.opacity,
        fullscreen_exclusive: w.fullscreen_exclusive,
        current_fullscreen_mode: w.current_fullscreen_mode,
        undefined_x: w.core.undefined_x,
        undefined_y: w.core.undefined_y,
        last_position_pending: w.core.last_position_pending,
        tiled: w.core.tiled,
        parent: w.parent,
        children: w.children.clone(),
        constrain_popup: w.constrain_popup,
        props: w.properties(),
    })
    .ok()
}

/// The window flags.
fn window_flags(window: WindowID) -> WindowFlags {
    with_window(window, |w| w.flags()).unwrap_or_default()
}

fn find(windows: &[WaylandWindowData], window: WindowID) -> Option<&WaylandWindowData> {
    windows.iter().find(|w| w.sdlwindow == window)
}

fn find_mut(windows: &mut [WaylandWindowData], window: WindowID) -> Option<&mut WaylandWindowData> {
    windows.iter_mut().find(|w| w.sdlwindow == window)
}

// ---------------------------------------------------------------------------
// Geometry helpers
// ---------------------------------------------------------------------------

/* According to the Wayland spec:
 *
 * "If the [fullscreen] surface doesn't cover the whole output, the compositor will
 * position the surface in the center of the output and compensate with border fill
 * covering the rest of the output. The content of the border fill is undefined, but
 * should be assumed to be in some way that attempts to blend into the surrounding area
 * (e.g. solid black)."
 *
 * KDE (6.7 at the time of writing) doesn't do this (https://invent.kde.org/plasma/kwin/-/merge_requests/6953),
 * so fullscreen modes that don't cover the output need to be manually masked.
 *
 * This must not be done universally, as some compositors do not correctly honor subsurface
 * offsets on fullscreen windows, but those also follow the spec regarding automatic masking
 * around fullscreen windows, so SDL doesn't need to apply its own mask.
 *
 * TODO: Remove this once KDE is spec-compliant.
 */
/// Translation of `ShouldMaskFullscreen()`.
fn should_mask_fullscreen() -> bool {
    static MASK_REQUIRED: OnceLock<bool> = OnceLock::new();
    *MASK_REQUIRED
        .get_or_init(|| crate::stdlib::getenv("XDG_CURRENT_DESKTOP").is_some_and(|d| d == "KDE"))
}

/// Translation of `GetWindowScale()`.
fn window_scale(flags: WindowFlags, wd: &WaylandWindowData) -> f64 {
    if flags.contains(WindowFlags::HIGH_PIXEL_DENSITY) || wd.scale_to_display {
        wd.scale_factor
    } else {
        1.0
    }
}

// These are point->pixel->point round trip safe; the inverse is not round trip safe due to rounding.
/// Translation of `PointToPixel()`, with the window's scale.
fn point_to_pixel(scale: f64, point: i32) -> i32 {
    /* Rounds halfway away from zero as per the Wayland fractional scaling protocol spec.
     * Wayland scale units are in units of 1/120, so the offset is required to correct for
     * rounding errors when using certain scale values.
     */
    if point != 0 {
        ((point as f64 * scale + 1e-6).round() as i32).max(1)
    } else {
        0
    }
}

/// Translation of `PixelToPoint()`, with the window's scale.
fn pixel_to_point(scale: f64, pixel: i32) -> i32 {
    if pixel != 0 {
        ((pixel as f64 / scale).round() as i32).max(1)
    } else {
        0
    }
}

/// `SDL_lroundf()`
fn lroundf(x: f32) -> i32 {
    x.round() as i32
}

/// Translation of `enum WaylandModeScale`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum WaylandModeScale {
    Aspect,
    Stretch,
    None,
}

/// Translation of `GetModeScaleMethod()`.
fn mode_scale_method() -> WaylandModeScale {
    static SCALE_MODE: OnceLock<WaylandModeScale> = OnceLock::new();
    *SCALE_MODE.get_or_init(|| match hints::get(hints::VIDEO_WAYLAND_MODE_SCALING) {
        Some(h) if h.eq_ignore_ascii_case("stretch") => WaylandModeScale::Stretch,
        Some(h) if h.eq_ignore_ascii_case("none") => WaylandModeScale::None,
        _ => WaylandModeScale::Aspect,
    })
}

/// The border insets from the window properties; they only apply to
/// xdg-toplevel windows. Translation of `GetBorderInsets()`.
fn border_insets(wd: &WaylandWindowData, props: &Properties) -> BorderInsets {
    if !wd.enable_insets {
        return BorderInsets::default();
    }
    if wd.shell_surface_type == ShellSurfaceType::XdgToplevel {
        let get = |name: &str| props.get_number(name).unwrap_or(0).clamp(0, MAX_SINT16) as i32;
        BorderInsets {
            left: get(PROP_WINDOW_WAYLAND_BORDER_INSET_LEFT_NUMBER),
            top: get(PROP_WINDOW_WAYLAND_BORDER_INSET_TOP_NUMBER),
            right: get(PROP_WINDOW_WAYLAND_BORDER_INSET_RIGHT_NUMBER),
            bottom: get(PROP_WINDOW_WAYLAND_BORDER_INSET_BOTTOM_NUMBER),
        }
    } else {
        BorderInsets::default()
    }
}

/// Translation of `EnsurePopupPositionIsValid()`: `(w, h)` is the popup's
/// size and `(parent_w, parent_h)` its parent's.
fn ensure_popup_position_is_valid(
    w: i32,
    h: i32,
    parent_w: i32,
    parent_h: i32,
    x: &mut i32,
    y: &mut i32,
) {
    let mut adj_count = 0;

    /* Per the xdg-positioner spec, child popup windows must intersect or at
     * least be partially adjoining the parent window.
     *
     * Failure to ensure this on a compositor that enforces this restriction
     * can result in behavior ranging from the window being spuriously closed
     * to a protocol violation.
     */
    if *x + w <= 0 {
        *x = -w;
        adj_count += 1;
    }
    if *y + h <= 0 {
        *y = -h;
        adj_count += 1;
    }
    if *x >= parent_w {
        *x = parent_w;
        adj_count += 1;
    }
    if *y >= parent_h {
        *y = parent_h;
        adj_count += 1;
    }

    /* If adjustment was required on the x and y axes, the popup is aligned with
     * the parent corner-to-corner and is neither overlapping nor adjoining, so it
     * must be nudged by 1 to be considered adjoining.
     */
    if adj_count > 1 {
        *x += if *x < 0 { 1 } else { -1 };
    }
}

/// The window geometry: the explicitly set one, or the full surface if SDL
/// has never set it. Translation of `GetWindowGeometry()`.
fn window_geometry(wd: &WaylandWindowData) -> Rect {
    let mut geometry = wd.explicit_geometry;
    if geometry.is_empty() {
        geometry = Rect {
            x: 0,
            y: 0,
            w: wd.current.logical_width,
            h: wd.current.logical_height,
        };
    }
    geometry
}

/// Positioner offsets are relative to the origin of the parent's window
/// geometry. Translation of `AdjustPopupOffset()` (`parent` is the popup's
/// parent).
fn adjust_popup_offset(syms: &WaylandSyms, parent: &WaylandWindowData, x: &mut i32, y: &mut i32) {
    if parent.shell_surface_type == ShellSurfaceType::Libdecor {
        let (mut adj_x, mut adj_y): (c_int, c_int) = (0, 0);
        if let (Some(frame), Some(l)) = (&parent.shell_surface.libdecor_frame, &syms.libdecor) {
            // SAFETY: the frame is alive; a pure computation.
            unsafe {
                (l.libdecor_frame_translate_coordinate)(frame.raw(), *x, *y, &mut adj_x, &mut adj_y)
            };
        }
        // FIXME (upstream): with no frame yet, the frame is NULL here (and
        // libdecor dereferences it); the offset becomes 0 instead.
        *x = adj_x;
        *y = adj_y;
    } else {
        *x -= parent.explicit_geometry.x;
        *y -= parent.explicit_geometry.y;
    }
}

/// Translation of `SetSurfaceOpaqueRegion()`.
fn set_surface_opaque_region(g: &Globals, surface: Obj<'_, WlSurface>, width: i32, height: i32) {
    if width != 0 && height != 0 {
        if let Some(compositor) = &g.compositor {
            let region = compositor.create_region();
            region.add(0, 0, width, height);
            surface.set_opaque_region(Some(region.obj()));
            // (the region is destroyed as it drops)
        }
    } else {
        surface.set_opaque_region(None);
    }
}

/// Damage all of a surface (`wl_surface_damage_buffer()` if available).
fn damage_all(surface: Obj<'_, WlSurface>, version: u32) {
    if version >= WlSurface::DAMAGE_BUFFER_SINCE_VERSION {
        surface.damage_buffer(0, 0, i32::MAX, i32::MAX);
    } else {
        surface.damage(0, 0, i32::MAX, i32::MAX);
    }
}

/// Translation of `RepositionPopup()`: `parent` is the popup's parent.
fn reposition_popup(
    windows: &[WaylandWindowData],
    syms: &WaylandSyms,
    window: &Win,
    parent: &Win,
    use_current_position: bool,
) {
    let (Some(wind), Some(parent_data)) = (find(windows, window.id), find(windows, parent.id))
    else {
        return;
    };

    if wind.shell_surface_type == ShellSurfaceType::XdgPopup {
        let (Some(positioner), Some(popup)) = (
            &wind.shell_surface.xdg_positioner,
            &wind.shell_surface.xdg_popup,
        ) else {
            return;
        };
        if popup.version() < XdgPopup::REPOSITION_SINCE_VERSION {
            return;
        }
        let mut x = if use_current_position {
            window.x
        } else {
            window.pending.x
        };
        let mut y = if use_current_position {
            window.y
        } else {
            window.pending.y
        };

        ensure_popup_position_is_valid(window.w, window.h, parent.w, parent.h, &mut x, &mut y);
        if wind.scale_to_display {
            let parent_scale = window_scale(parent.flags, parent_data);
            x = pixel_to_point(parent_scale, x);
            y = pixel_to_point(parent_scale, y);
        }
        adjust_popup_offset(syms, parent_data, &mut x, &mut y);
        let parent_geometry = window_geometry(parent_data);
        positioner.set_anchor_rect(0, 0, parent_geometry.w, parent_geometry.h);
        positioner.set_size(wind.current.logical_width, wind.current.logical_height);
        positioner.set_offset(x, y);
        popup.reposition(positioner.obj(), 0);
    }
}

/// Translation of `ApplyGeometryLimits()`.
fn apply_geometry_limits(
    win: &Win,
    resizing: bool,
    resize_edge: u32,
    req_w: i32,
    req_h: i32,
) -> (i32, i32) {
    let mut w = req_w;
    let mut h = req_h;
    let aspect = req_w as f32 / req_h as f32;

    /* If adjusting for aspect causes the adjusted dimension to exceed the min/max limit, clamp to the limit
     * and attempt to adjust the opposite dimension to compensate.
     */
    if !resizing || (resize_edge & WAYLAND_RESIZE_EDGE_CORNER) == WAYLAND_RESIZE_EDGE_CORNER {
        if win.min_aspect != 0.0 && aspect < win.min_aspect {
            h = lroundf(req_w as f32 / win.min_aspect).max(1);

            if h < win.min_h {
                w = lroundf(win.min_h as f32 * win.min_aspect).max(1);
            }
            if win.max_h != 0 && h > win.max_h {
                w = lroundf(win.max_h as f32 * win.min_aspect).max(1);
            }
        } else if win.max_aspect != 0.0 && aspect > win.max_aspect {
            w = lroundf(req_h as f32 * win.max_aspect).max(1);

            if w < win.min_w {
                h = lroundf(win.min_w as f32 / win.max_aspect).max(1);
            }
            if win.max_w != 0 && w > win.max_w {
                h = lroundf(win.max_w as f32 / win.max_aspect).max(1);
            }
        }
    } else if resize_edge & WAYLAND_RESIZE_EDGE_LR != 0 {
        if win.min_aspect != 0.0 && aspect < win.min_aspect {
            h = lroundf(req_w as f32 / win.min_aspect).max(1);

            if h < win.min_h {
                w = lroundf(win.min_h as f32 * win.min_aspect).max(1);
            }
            if win.max_h != 0 && h > win.max_h {
                w = lroundf(win.max_h as f32 * win.min_aspect).max(1);
            }
        } else if win.max_aspect != 0.0 && aspect > win.max_aspect {
            h = lroundf(req_w as f32 / win.max_aspect).max(1);

            if h < win.min_h {
                w = lroundf(win.min_h as f32 * win.max_aspect).max(1);
            }
            if win.max_h != 0 && h > win.max_h {
                w = lroundf(win.max_h as f32 * win.max_aspect).max(1);
            }
        }
    } else if resize_edge & WAYLAND_RESIZE_EDGE_TB != 0 {
        if win.min_aspect != 0.0 && aspect < win.min_aspect {
            w = lroundf(req_h as f32 * win.min_aspect).max(1);

            if w < win.min_w {
                h = lroundf(win.min_w as f32 / win.min_aspect).max(1);
            }
            if win.max_w != 0 && w > win.max_w {
                h = lroundf(win.max_w as f32 / win.min_aspect).max(1);
            }
        } else if win.max_aspect != 0.0 && aspect > win.max_aspect {
            w = lroundf(req_h as f32 * win.max_aspect).max(1);

            if w < win.min_w {
                h = lroundf(win.min_w as f32 / win.max_aspect).max(1);
            }
            if win.max_w != 0 && w > win.max_w {
                h = lroundf(win.max_w as f32 / win.max_aspect).max(1);
            }
        }
    }

    if win.max_w != 0 {
        w = w.min(win.max_w);
    }
    let out_w = w.max(win.min_w);

    if win.max_h != 0 {
        h = h.min(win.max_h);
    }
    let out_h = h.max(win.min_h);

    (out_w, out_h)
}

/// Translation of `DetermineResizeAxis()`.
fn determine_resize_axis(wind: &mut WaylandWindowData, width: i32, height: i32, resizing: bool) {
    /* Try to determine the axis along which the window is being resized.
     *
     * Every compositor is different, and some will start sending strange values once the resize
     * begins, so only the first change is safe to try and determine the resize edge.
     *
     * Some compositors don't send a configure event with the resize flag cleared when the resize ends,
     * so a reset timer is required.
     */
    if resizing {
        let now = crate::timer::ticks_ns();
        if now >= wind.last_resize_event_time_ns + 500_000_000 {
            wind.resize_edge = 0;
        }

        wind.last_resize_event_time_ns = now;

        if wind.resize_edge == 0 {
            if width != wind.last_configure.width || height == 0 {
                wind.resize_edge |= WAYLAND_RESIZE_EDGE_LR;
            }
            if height != wind.last_configure.height || width == 0 {
                wind.resize_edge |= WAYLAND_RESIZE_EDGE_TB;
            }
        }
    } else {
        wind.resize_edge = 0;
    }

    wind.resizing = resizing;
}

/// The libdecor functions (loaded with libdecor: only libdecor windows
/// reach this).
fn libdecor_syms(syms: &WaylandSyms) -> Option<&Libdecor> {
    syms.libdecor.as_ref()
}

/// The events `ConfigureWindowGeometry()` ends with.
struct GeometryEvents {
    resized: (i32, i32),
    pixel_size: (i32, i32),
    exposed: bool,
    occluded: bool,
    pointer_scale_changed: bool,
}

/// The part of `ConfigureWindowGeometry()` done with the driver data.
fn configure_window_geometry_locked(
    d: &mut VideoData,
    syms: &WaylandSyms,
    window: &Win,
    children: &[Win],
) -> Option<GeometryEvents> {
    let VideoData { g, windows, .. } = d;
    let data = find_mut(windows, window.id)?;
    let scale_factor = window_scale(window.flags, data);
    let prev_pointer_scale = data.pointer_scale;
    let old_pixel_width = data.current.pixel_width;
    let old_pixel_height = data.current.pixel_height;
    let window_width;
    let window_height;
    let mut viewport_width;
    let mut viewport_height;
    let window_size_changed;
    let buffer_size_changed;
    let is_opaque = !window.flags.contains(WindowFlags::TRANSPARENT) && window.opacity == 1.0;

    if data.is_fullscreen && window.fullscreen_exclusive {
        window_width = window.current_fullscreen_mode.w;
        window_height = window.current_fullscreen_mode.h;

        viewport_width = data.requested.logical_width;
        viewport_height = data.requested.logical_height;

        let mut scale_mode = mode_scale_method();
        if scale_mode == WaylandModeScale::None {
            /* The Wayland spec states that the advertised fullscreen dimensions are a maximum.
             * Windows can request a smaller size, but exceeding these dimensions is a protocol violation,
             * thus, modes that exceed the output size still need to be scaled with a viewport.
             */
            if window_width <= viewport_width && window_height <= viewport_height {
                viewport_width = window_width;
                viewport_height = window_height;
            } else {
                scale_mode = WaylandModeScale::Aspect; // (SDL_FALLTHROUGH)
            }
        }
        if scale_mode == WaylandModeScale::Aspect {
            let output_ratio = viewport_width as f32 / viewport_height as f32;
            let mode_ratio = window_width as f32 / window_height as f32;

            if output_ratio > mode_ratio {
                viewport_width =
                    lroundf(window_width as f32 * (viewport_height as f32 / window_height as f32));
            } else if output_ratio < mode_ratio {
                viewport_height =
                    lroundf(window_height as f32 * (viewport_width as f32 / window_width as f32));
            }
        }

        window_size_changed = window_width != window.w
            || window_height != window.h
            || data.current.viewport_width != viewport_width
            || data.current.viewport_height != viewport_height;

        // Exclusive fullscreen window sizes are always in pixel units.
        data.current.pixel_width = window_width;
        data.current.pixel_height = window_height;
        buffer_size_changed = data.current.pixel_width != old_pixel_width
            || data.current.pixel_height != old_pixel_height;

        if window_size_changed || buffer_size_changed {
            if let Some(viewport) = &data.viewport {
                viewport.set_destination(viewport_width, viewport_height);

                data.current.logical_width = data.requested.logical_width;
                data.current.logical_height = data.requested.logical_height;
                data.current.viewport_width = viewport_width;
                data.current.viewport_height = viewport_height;
            } else {
                // Calculate the integer scale from the mode and output.
                let int_scale = (window.current_fullscreen_mode.w / viewport_width).max(1);

                data.surface().set_buffer_scale(int_scale);
                data.current.logical_width = window.current_fullscreen_mode.w;
                data.current.logical_height = window.current_fullscreen_mode.h;
            }

            data.pointer_scale.x = window_width as f64 / viewport_width as f64;
            data.pointer_scale.y = window_height as f64 / viewport_height as f64;
        }
    } else {
        if !data.scale_to_display {
            viewport_width = data.requested.logical_width;
            viewport_height = data.requested.logical_height;
        } else {
            viewport_width = data.requested.pixel_width;
            viewport_height = data.requested.pixel_height;
        }

        if !data.enable_insets
            && data.shell_surface_status != ShellSurfaceStatus::Hidden
            && data.viewport.is_some()
            && g.subcompositor.is_some()
            && !data.floating
            && !data.is_fullscreen
        {
            let (min_width, min_height, max_width, max_height) =
                if window.flags.contains(WindowFlags::RESIZABLE) {
                    (window.min_w, window.min_h, window.max_w, window.max_h)
                } else {
                    // Use the fixed size in the odd case where a non-resizable window somehow wound up tiled or maximized.
                    (
                        window.floating.w,
                        window.floating.h,
                        window.floating.w,
                        window.floating.h,
                    )
                };

            if min_width != 0 {
                viewport_width = viewport_width.max(min_width);
            }
            if min_height != 0 {
                viewport_height = viewport_height.max(min_height);
            }
            if max_width != 0 {
                viewport_width = viewport_width.min(max_width);
            }
            if max_height != 0 {
                viewport_height = viewport_height.min(max_height);
            }

            let mut aspect = viewport_width as f32 / viewport_height as f32;
            if window.min_aspect != 0.0 && aspect < window.min_aspect {
                viewport_height = lroundf(viewport_width as f32 / window.min_aspect).max(1);
            } else if window.max_aspect != 0.0 && aspect > window.max_aspect {
                viewport_width = lroundf(viewport_height as f32 * window.max_aspect).max(1);
            }

            // At this point, the viewport matches the virtual window dimensions, but the viewport might be clamped to the output window dimensions beyond here.
            window_width = viewport_width;
            window_height = viewport_height;

            // If the viewport bounds exceed the window size, scale them while maintaining the aspect ratio.
            let (limit_w, limit_h) = if !data.scale_to_display {
                (data.requested.logical_width, data.requested.logical_height)
            } else {
                (data.requested.pixel_width, data.requested.pixel_height)
            };
            if viewport_width > limit_w || viewport_height > limit_h {
                aspect = viewport_width as f32 / viewport_height as f32;
                let window_ratio = limit_w as f32 / limit_h as f32;
                if aspect >= window_ratio {
                    viewport_width = limit_w;
                    viewport_height = lroundf(viewport_width as f32 / aspect);
                } else if aspect < window_ratio {
                    viewport_height = limit_h;
                    viewport_width = lroundf(viewport_height as f32 * aspect);
                }
            }

            viewport_width = viewport_width.max(1);
            viewport_height = viewport_height.max(1);
        } else {
            window_width = viewport_width;
            window_height = viewport_height;
        }

        if !data.scale_to_display {
            data.current.pixel_width = point_to_pixel(scale_factor, window_width);
            data.current.pixel_height = point_to_pixel(scale_factor, window_height);
        } else {
            // The viewport size is in pixels at this point; convert it to logical units.
            data.current.pixel_width = window_width;
            data.current.pixel_height = window_height;
            viewport_width = pixel_to_point(scale_factor, viewport_width);
            viewport_height = pixel_to_point(scale_factor, viewport_height);
        }

        // Clamp the physical window size to the system minimum required size.
        data.requested.logical_width = data
            .requested
            .logical_width
            .max(data.system_limits.min_width);
        data.requested.logical_height = data
            .requested
            .logical_height
            .max(data.system_limits.min_height);

        window_size_changed = data.requested.logical_width != data.current.logical_width
            || data.requested.logical_height != data.current.logical_height
            || viewport_width != data.current.viewport_width
            || viewport_height != data.current.viewport_height;

        buffer_size_changed = data.current.pixel_width != old_pixel_width
            || data.current.pixel_height != old_pixel_height;

        if window_size_changed || buffer_size_changed {
            if let Some(viewport) = &data.viewport {
                viewport.set_destination(viewport_width, viewport_height);
            } else if window.flags.contains(WindowFlags::HIGH_PIXEL_DENSITY) {
                // Don't change this if the DPI awareness flag is unset, as an application may have set this manually on a custom or external surface.
                data.surface().set_buffer_scale(scale_factor as i32);
            }

            data.current.logical_width = data.requested.logical_width;
            data.current.logical_height = data.requested.logical_height;
            data.current.viewport_width = viewport_width;
            data.current.viewport_height = viewport_height;

            data.pointer_scale.x = window_width as f64 / viewport_width as f64;
            data.pointer_scale.y = window_height as f64 / viewport_height as f64;
        }
    }

    if let (Some(egl_window), true) = (&data.egl_window, buffer_size_changed) {
        egl_window.resize(data.current.pixel_width, data.current.pixel_height, 0, 0);
    }

    /* Calculate the mask size and offset.
     * Fullscreen windows are centered and masked automatically by the compositor, unless it lacks the capability.
     */
    if !data.enable_insets
        && data.viewport.is_some()
        && g.subcompositor.is_some()
        && (!data.is_fullscreen || should_mask_fullscreen())
        && (viewport_width != data.current.logical_width
            || viewport_height != data.current.logical_height)
    {
        let mut old_buffer = None;

        if data.mask.surface.is_none() {
            if let Some(compositor) = &g.compositor {
                let surface = compositor.create_surface();
                surface.obj().set_tag(&SDL_WAYLAND_SURFACE_TAG);
                data.mask.surface = Some(surface);
            }
        }
        let mask_surface = data.mask.surface.as_ref()?;
        if data.mask.subsurface.is_none() {
            if let Some(subcompositor) = &g.subcompositor {
                data.mask.subsurface =
                    Some(subcompositor.get_subsurface(mask_surface.obj(), data.surface.obj()));
            }
        }
        if data.mask.viewport.is_none() {
            if let Some(viewporter) = &g.viewporter {
                data.mask.viewport = Some(viewporter.get_viewport(mask_surface.obj()));
            }
        }
        if data.mask.buffer.is_none() || data.mask.opaque != is_opaque {
            old_buffer = data.mask.buffer.take();
            data.mask.opaque = is_opaque;
            data.mask.buffer = wayland_create_single_pixel_buffer(
                g,
                0,
                0,
                0,
                if is_opaque { u32::MAX } else { 0 },
            );
        }

        mask_surface.attach(data.mask.buffer.as_ref().map(|b| b.obj()), 0, 0);

        if let Some(subsurface) = &data.mask.subsurface {
            subsurface.place_below(data.surface.obj());
        }
        if let Some(viewport) = &data.mask.viewport {
            viewport.set_destination(data.current.logical_width, data.current.logical_height);
        }

        damage_all(mask_surface.obj(), mask_surface.version());

        if is_opaque {
            set_surface_opaque_region(
                g,
                mask_surface.obj(),
                data.current.logical_width,
                data.current.logical_height,
            );
        } else {
            set_surface_opaque_region(g, mask_surface.obj(), 0, 0);
        }

        // Can't use an offset subsurface with libdecor (yet), or the decorations won't line up properly.
        if data.shell_surface_type != ShellSurfaceType::Libdecor {
            data.mask.offset_x = -(data.current.logical_width - viewport_width) / 2;
            data.mask.offset_y = -(data.current.logical_height - viewport_height) / 2;
        } else {
            data.mask.offset_x = 0;
            data.mask.offset_y = 0;
        }

        if let Some(subsurface) = &data.mask.subsurface {
            subsurface.set_position(data.mask.offset_x, data.mask.offset_y);
        }
        mask_surface.commit();

        drop(old_buffer);

        data.mask.mapped = true;
    } else if data.mask.mapped {
        if let Some(subsurface) = &data.mask.subsurface {
            subsurface.set_position(0, 0);
        }
        if let Some(mask_surface) = &data.mask.surface {
            mask_surface.attach(None, 0, 0);
            mask_surface.commit();
        }

        data.mask.offset_x = 0;
        data.mask.offset_y = 0;
        data.mask.mapped = false;
    }

    if data.shell_surface_type == ShellSurfaceType::XdgToplevel
        && data.shell_surface.xdg_surface.is_some()
        && (data.enable_insets || data.viewport.is_none())
    {
        /* The window geometry is the surface minus the declared border insets. They don't apply
         * when an exact size is required (maximized/fullscreen).
         */
        let insets = border_insets(data, &window.props);
        let use_insets =
            (insets.left != 0 || insets.top != 0 || insets.right != 0 || insets.bottom != 0)
                && !window
                    .flags
                    .intersects(WindowFlags::MAXIMIZED | WindowFlags::FULLSCREEN);
        let mut geometry = Rect {
            x: 0,
            y: 0,
            w: data.current.logical_width,
            h: data.current.logical_height,
        };
        if use_insets {
            geometry.x = insets.left;
            geometry.y = insets.top;
            geometry.w = (geometry.w - (insets.left + insets.right)).max(1);
            geometry.h = (geometry.h - (insets.top + insets.bottom)).max(1);
        }
        /* An explicitly set geometry does not track the surface, so it must be kept in sync
         * once set. Otherwise this is only done when viewports aren't supported and the size
         * has changed (XXX: a hack) to avoid a potential protocol violation if a buffer with
         * an old size is committed.
         */
        if (use_insets
            || !data.explicit_geometry.is_empty()
            || (data.viewport.is_none() && window_size_changed))
            && geometry != data.explicit_geometry
        {
            if let Some(xdg_surface) = &data.shell_surface.xdg_surface {
                xdg_surface.set_window_geometry(geometry.x, geometry.y, geometry.w, geometry.h);
            }
            data.explicit_geometry = geometry;
        }
    }

    /* The opaque region and pointer confinement region only need to be
     * recalculated if the output size has changed.
     */
    if window_size_changed {
        if is_opaque {
            set_surface_opaque_region(g, data.surface.obj(), viewport_width, viewport_height);
        } else {
            set_surface_opaque_region(g, data.surface.obj(), 0, 0);
        }
    }

    let pointer_scale_changed = prev_pointer_scale != data.pointer_scale;
    let scale_to_display = data.scale_to_display;
    let pixel_size = (data.current.pixel_width, data.current.pixel_height);
    let shown = data.shell_surface_status == ShellSurfaceStatus::Shown;
    let suspended = data.suspended;

    if window_size_changed {
        // Ensure that child popup windows are still in bounds.
        for child in children {
            reposition_popup(windows, syms, child, window, true);
        }
    }

    // Unconditionally send the window and drawable size, the video core will deduplicate when required.
    let resized = if !scale_to_display {
        (window_width, window_height)
    } else {
        pixel_size
    };

    /* Send an exposure event if the window is in the shown state and the size has changed,
     * even if the window is occluded, as the client needs to commit a new frame for the
     * changes to take effect.
     *
     * The occlusion state is immediately set again afterward, if necessary.
     */
    let exposed = shown
        && ((buffer_size_changed || window_size_changed)
            || (!suspended && window.flags.contains(WindowFlags::OCCLUDED)));
    let occluded = shown && suspended;

    Some(GeometryEvents {
        resized,
        pixel_size,
        exposed,
        occluded,
        pointer_scale_changed,
    })
}

/// Translation of `xdg_toplevel_wm_capabilities` values to `wm_caps`.
fn wm_caps_from_xdg(capabilities: &[u8]) -> u32 {
    let mut wm_caps = 0;
    for cap in super::client::array_u32(capabilities) {
        match XdgToplevelWmCapabilities(cap) {
            XdgToplevelWmCapabilities::WINDOW_MENU => wm_caps |= WAYLAND_WM_CAPS_WINDOW_MENU,
            XdgToplevelWmCapabilities::MAXIMIZE => wm_caps |= WAYLAND_WM_CAPS_MAXIMIZE,
            XdgToplevelWmCapabilities::FULLSCREEN => wm_caps |= WAYLAND_WM_CAPS_FULLSCREEN,
            XdgToplevelWmCapabilities::MINIMIZE => wm_caps |= WAYLAND_WM_CAPS_MINIMIZE,
            _ => {}
        }
    }
    wm_caps
}

/// The states of an `xdg_toplevel.configure` event.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
struct ToplevelStates {
    fullscreen: bool,
    maximized: bool,
    floating: bool,
    tiled: bool,
    active: bool,
    resizing: bool,
    suspended: bool,
    constraints: u32,
}

/// Read the states of an `xdg_toplevel.configure` event.
fn parse_toplevel_states(states: &[u8]) -> ToplevelStates {
    let mut s = ToplevelStates {
        floating: true,
        ..Default::default()
    };
    for state in super::client::array_u32(states) {
        match XdgToplevelState(state) {
            XdgToplevelState::FULLSCREEN => {
                s.fullscreen = true;
                s.floating = false;
            }
            XdgToplevelState::MAXIMIZED => {
                s.maximized = true;
                s.floating = false;
            }
            XdgToplevelState::RESIZING => s.resizing = true,
            XdgToplevelState::ACTIVATED => s.active = true,
            XdgToplevelState::TILED_LEFT
            | XdgToplevelState::TILED_RIGHT
            | XdgToplevelState::TILED_TOP
            | XdgToplevelState::TILED_BOTTOM => {
                s.tiled = true;
                s.floating = false;
            }
            XdgToplevelState::SUSPENDED => s.suspended = true,
            XdgToplevelState::CONSTRAINED_LEFT => {
                s.constraints |= WAYLAND_TOPLEVEL_CONSTRAINED_LEFT
            }
            XdgToplevelState::CONSTRAINED_RIGHT => {
                s.constraints |= WAYLAND_TOPLEVEL_CONSTRAINED_RIGHT
            }
            XdgToplevelState::CONSTRAINED_TOP => s.constraints |= WAYLAND_TOPLEVEL_CONSTRAINED_TOP,
            XdgToplevelState::CONSTRAINED_BOTTOM => {
                s.constraints |= WAYLAND_TOPLEVEL_CONSTRAINED_BOTTOM
            }
            _ => {}
        }
    }
    s
}

/// The HDR properties of a `frog_color_managed_surface.preferred_metadata`
/// event (from `frog_preferred_metadata_handler()`).
fn frog_hdr(
    transfer_function: FrogColorManagedSurfaceTransferFunction,
    max_luminance: u32,
) -> crate::video::sysvideo::HdrOutputProperties {
    let hdr_headroom = match transfer_function {
        FrogColorManagedSurfaceTransferFunction::ST2084_PQ => {
            /* ITU-R BT.2408-7 (Sept 2023) has the reference PQ white level at 203 nits,
             * while older Dolby documentation claims a reference level of 100 nits.
             *
             * Use 203 nits for now.
             */
            max_luminance as f32 / 203.0
        }
        FrogColorManagedSurfaceTransferFunction::SCRGB_LINEAR => max_luminance as f32 / 80.0,
        _ => 1.0,
    };

    crate::video::sysvideo::HdrOutputProperties {
        sdr_white_level: 1.0,
        hdr_headroom,
    }
}

// ---------------------------------------------------------------------------
// The libdecor frame callbacks (`libdecor_frame_interface`)
// ---------------------------------------------------------------------------

/// The window and device of a frame callback.
///
/// # Safety
///
/// `user_data` is the [`FrameUserData`] of a live [`LibdecorFrame`].
unsafe fn frame_target(user_data: *mut c_void) -> (Option<Arc<WaylandVideo>>, WindowID) {
    // SAFETY: as documented; the data is only read here, before the
    // callback can drop the frame.
    let ud = unsafe { &*(user_data as *const FrameUserData) };
    (ud.video.upgrade(), ud.window)
}

/// Translation of `decoration_frame_configure()`.
unsafe extern "C" fn decoration_frame_configure_c(
    frame: *mut libdecor_frame,
    configuration: *mut libdecor_configuration,
    user_data: *mut c_void,
) {
    // SAFETY: libdecor passes the user data given to libdecor_decorate.
    let (video, window) = unsafe { frame_target(user_data) };
    catch_callback(|| {
        if let Some(video) = video {
            video.decoration_frame_configure(window, frame, configuration);
        }
    });
}

/// Translation of `decoration_frame_close()`.
unsafe extern "C" fn decoration_frame_close_c(_frame: *mut libdecor_frame, user_data: *mut c_void) {
    // SAFETY: as above.
    let (_, window) = unsafe { frame_target(user_data) };
    catch_callback(|| {
        send_window_event(window, EventType::WINDOW_CLOSE_REQUESTED, 0, 0);
    });
}

/// Translation of `decoration_frame_commit()`.
unsafe extern "C" fn decoration_frame_commit_c(
    _frame: *mut libdecor_frame,
    user_data: *mut c_void,
) {
    // SAFETY: as above.
    let (video, window) = unsafe { frame_target(user_data) };
    catch_callback(|| {
        /* libdecor decoration subsurfaces are synchronous, so the client needs to
         * commit a frame to trigger an update of the decoration surfaces.
         */
        let Some(video) = video else {
            return;
        };
        let expose = video.with_data(|d| {
            d.window(window).is_some_and(|w| {
                !w.suspended && w.shell_surface_status == ShellSurfaceStatus::Shown
            })
        });
        if expose {
            send_window_event(window, EventType::WINDOW_EXPOSED, 0, 0);
        }
    });
}

/// Translation of `decoration_dismiss_popup()`.
unsafe extern "C" fn decoration_dismiss_popup_c(
    _frame: *mut libdecor_frame,
    _seat_name: *const c_char,
    _user_data: *mut c_void,
) {
    // NOP
}

/// Translation of `libdecor_frame_interface`.
static LIBDECOR_FRAME_INTERFACE: libdecor_frame_interface = libdecor_frame_interface {
    configure: Some(decoration_frame_configure_c),
    close: Some(decoration_frame_close_c),
    commit: Some(decoration_frame_commit_c),
    dismiss_popup: Some(decoration_dismiss_popup_c),
    reserved: [None; 10],
};

/// Translation of `Wayland_SetWindowHitTest()`.
pub(crate) fn wayland_set_window_hit_test(_window: WindowID, _enabled: bool) -> Result<()> {
    Ok(()) // just succeed, the real work is done elsewhere.
}

impl WaylandVideo {
    // -----------------------------------------------------------------------
    // Small helpers
    // -----------------------------------------------------------------------

    /// Run `libdecor_dispatch()` (with nothing borrowed).
    fn libdecor_dispatch(&self, timeout: c_int) -> c_int {
        let (Some(ctx), Some(l)) = (
            self.with_data(|d| d.libdecor.as_ref().map(|c| c.raw)),
            &self.syms().libdecor,
        ) else {
            return -1;
        };
        // SAFETY: the context is alive (only video_quit frees it, on this
        // thread); listeners may run.
        let r = unsafe { (l.libdecor_dispatch)(ctx.as_ptr(), timeout) };
        resume_pending_panic();
        r
    }

    /// Call libdecor on the frame of a window (with nothing borrowed, as
    /// libdecor may call the frame callbacks).
    fn with_libdecor_frame<R>(
        &self,
        window: WindowID,
        f: impl FnOnce(&Libdecor, *mut libdecor_frame) -> R,
    ) -> Option<R> {
        let frame = self.with_data(|d| {
            let wd = d.window(window)?;
            if wd.shell_surface_type != ShellSurfaceType::Libdecor {
                return None;
            }
            wd.shell_surface.libdecor_frame.as_ref().map(|f| f.raw())
        })?;
        let l = libdecor_syms(self.syms())?;
        let r = f(l, frame);
        resume_pending_panic();
        Some(r)
    }

    /// Translation of `GetToplevelForWindow()`: the `xdg_toplevel` of a
    /// window (from libdecor or ours).
    fn toplevel_for_window<'a>(
        &'a self,
        wind: Option<&WaylandWindowData>,
    ) -> Option<Obj<'a, XdgToplevel>> {
        let wind = wind?;
        /* Libdecor crashes on attempts to unset the parent by passing null, which is allowed by the
         * toplevel spec, so just use the raw xdg-toplevel instead (that's what libdecor does
         * internally anyways).
         */
        if wind.shell_surface_type == ShellSurfaceType::Libdecor {
            let frame = wind.shell_surface.libdecor_frame.as_ref()?;
            let l = libdecor_syms(self.syms())?;
            // SAFETY: the frame is alive; a getter.
            let raw = unsafe { (l.libdecor_frame_get_xdg_toplevel)(frame.raw()) };
            // SAFETY: libdecor's xdg_toplevel lives as long as the frame.
            unsafe { Obj::from_raw(raw.cast(), &self.conn) }
        } else if wind.shell_surface_type == ShellSurfaceType::XdgToplevel {
            let toplevel = wind.shell_surface.xdg_toplevel.as_ref()?;
            // SAFETY: the toplevel is alive while the window data is
            // borrowed (the object borrows the connection of the device).
            unsafe { Obj::from_raw(toplevel.raw().cast(), &self.conn) }
        } else {
            None
        }
    }

    /// Translation of `CommitLibdecorFrame()`.
    fn commit_libdecor_frame(&self, window: WindowID) {
        let Some((w, h)) = self.with_data(|d| {
            d.window(window)
                .map(|wd| (wd.current.logical_width, wd.current.logical_height))
        }) else {
            return;
        };
        self.with_libdecor_frame(window, |l, frame| {
            // SAFETY: the frame is alive; the state is freed once.
            unsafe {
                let state = (l.libdecor_state_new)(w, h);
                (l.libdecor_frame_commit)(frame, state, std::ptr::null_mut());
                (l.libdecor_state_free)(state);
            }
        });
    }

    /// Translation of `AddPendingStateSync()`.
    fn add_pending_state_sync(&self, d: &mut VideoData, window: WindowID) {
        let Some(wd) = d.window_mut(window) else {
            return;
        };
        wd.pending_state_deadline_count += 1;
        let cb = self.conn.obj().sync();
        let weak = self.weak();
        self.callbacks.add(cb, move |_| {
            // Get the window from the ID, as it may have been destroyed.
            if let Some(video) = weak.upgrade() {
                video.with_data(|d| {
                    if let Some(wd) = d.window_mut(window) {
                        wd.pending_state_deadline_count -= 1;
                    }
                });
            }
        });
    }

    /// Translation of `FlushPendingEvents()`.
    fn flush_pending_events(&self, window: WindowID) {
        // Serialize and restore the pending flags, as they may be overwritten while flushing.
        let Ok((last_position_pending, last_size_pending)) = with_window(window, |w| {
            (w.core.last_position_pending, w.core.last_size_pending)
        }) else {
            return;
        };

        // FIXME (upstream): this loops forever if the connection fails before
        // the sync points are reached (kept as upstream).
        while self.with_data(|d| {
            d.window(window)
                .map_or(0, |w| w.pending_state_deadline_count)
        }) > 0
        {
            let _ = self.conn.roundtrip();
        }

        let _ = with_window(window, |w| {
            w.core.last_position_pending = last_position_pending;
            w.core.last_size_pending = last_size_pending;
        });
    }

    /// The show/hide sync point (`show_hide_sync_listener`).
    fn add_show_hide_sync(&self, window: WindowID) {
        let cb = self.conn.obj().sync();
        let weak = self.weak();
        self.callbacks.add(cb, move |_| {
            // Get the window from the ID as it may have been destroyed
            if let Some(video) = weak.upgrade() {
                video.with_data(|d| {
                    if let Some(wd) = d.window_mut(window) {
                        wd.show_hide_sync_required = false;
                    }
                });
            }
        });
    }

    // -----------------------------------------------------------------------
    // Geometry
    // -----------------------------------------------------------------------

    /// Translation of `SetMinMaxDimensions()`.
    fn set_min_max_dimensions(&self, window: WindowID) {
        let Some(win) = snapshot(window) else {
            return;
        };
        let libdecor_limits =
            self.with_data(|d| {
                let wind = d.window(window)?;

                /* Keep the limits off while the window is in a fixed-size state, or the controls
                 * to exit that state may be disabled.
                 */
                let (mut min_width, mut min_height, mut max_width, mut max_height);
                if win
                    .flags
                    .intersects(WindowFlags::FULLSCREEN | WindowFlags::MAXIMIZED)
                {
                    min_width = 0;
                    min_height = 0;
                    max_width = 0;
                    max_height = 0;
                } else if win.flags.contains(WindowFlags::RESIZABLE) {
                    let scale = window_scale(win.flags, wind);
                    let mut adj_w = win.min_w.max(wind.system_limits.min_width);
                    let mut adj_h = win.min_h.max(wind.system_limits.min_height);
                    if wind.scale_to_display {
                        adj_w = pixel_to_point(scale, adj_w);
                        adj_h = pixel_to_point(scale, adj_h);
                    }
                    min_width = adj_w;
                    min_height = adj_h;

                    adj_w = if win.max_w != 0 {
                        win.max_w.max(wind.system_limits.min_width)
                    } else {
                        0
                    };
                    adj_h = if win.max_h != 0 {
                        win.max_h.max(wind.system_limits.min_height)
                    } else {
                        0
                    };
                    if wind.scale_to_display {
                        adj_w = pixel_to_point(scale, adj_w);
                        adj_h = pixel_to_point(scale, adj_h);
                    }
                    max_width = adj_w;
                    max_height = adj_h;
                } else {
                    min_width = wind.current.logical_width;
                    min_height = wind.current.logical_height;
                    max_width = wind.current.logical_width;
                    max_height = wind.current.logical_height;
                }

                if wind.shell_surface_type == ShellSurfaceType::Libdecor {
                    // Can't do anything yet, wait for ShowWindow
                    let frame = wind.shell_surface.libdecor_frame.as_ref()?;
                    Some((frame.raw(), min_width, min_height, max_width, max_height))
                } else if wind.shell_surface_type == ShellSurfaceType::XdgToplevel {
                    // Can't do anything yet, wait for ShowWindow
                    let toplevel = wind.shell_surface.xdg_toplevel.as_ref()?;
                    /* The limits are in window geometry space; keep an existing minimum at 1 or more,
                     * or the window can be spuriously closed.
                     */
                    let insets = border_insets(wind, &win.props);
                    min_width = (min_width - (insets.left + insets.right)).max(if min_width != 0 {
                        1
                    } else {
                        0
                    });
                    min_height = (min_height - (insets.top + insets.bottom))
                        .max(if min_height != 0 { 1 } else { 0 });
                    if max_width != 0 {
                        max_width = (max_width - (insets.left + insets.right)).max(1);
                    }
                    if max_height != 0 {
                        max_height = (max_height - (insets.top + insets.bottom)).max(1);
                    }
                    toplevel.set_min_size(min_width, min_height);
                    toplevel.set_max_size(max_width, max_height);
                    None
                } else {
                    None
                }
            });

        if let (Some((frame, min_width, min_height, max_width, max_height)), Some(l)) =
            (libdecor_limits, libdecor_syms(self.syms()))
        {
            // SAFETY: the frame is alive; libdecor may call the frame
            // callbacks, with nothing borrowed.
            unsafe {
                if min_width != 0
                    && min_height != 0
                    && min_width == max_width
                    && min_height == max_height
                {
                    (l.libdecor_frame_unset_capabilities)(frame, LIBDECOR_ACTION_RESIZE);
                } else {
                    (l.libdecor_frame_set_capabilities)(frame, LIBDECOR_ACTION_RESIZE);
                }

                /* No need to change these values if the window is non-resizable,
                 * as libdecor will just overwrite them internally.
                 */
                if (l.libdecor_frame_has_capability)(frame, LIBDECOR_ACTION_RESIZE) {
                    (l.libdecor_frame_set_min_content_size)(frame, min_width, min_height);
                    (l.libdecor_frame_set_max_content_size)(frame, max_width, max_height);
                }
            }
            resume_pending_panic();
        }
    }

    /// Translation of `ConfigureWindowGeometry()`.
    pub(crate) fn configure_window_geometry(&self, window: WindowID) {
        let Some(win) = snapshot(window) else {
            return;
        };
        let children: Vec<Win> = win.children.iter().filter_map(|&c| snapshot(c)).collect();
        let syms = self.syms().clone();
        let Some(ev) =
            self.with_data(|d| configure_window_geometry_locked(d, &syms, &win, &children))
        else {
            return;
        };

        // Update the scale for any focused cursors.
        if ev.pointer_scale_changed {
            self.wayland_display_update_pointer_focused_scale(window);
        }

        /* Update the min/max dimensions, primarily if the state was changed, and for non-resizable
         * xdg-toplevel windows where the limits should match the window size.
         */
        self.set_min_max_dimensions(window);

        // Unconditionally send the window and drawable size, the video core will deduplicate when required.
        send_window_event(
            window,
            EventType::WINDOW_RESIZED,
            ev.resized.0,
            ev.resized.1,
        );
        send_window_event(
            window,
            EventType::WINDOW_PIXEL_SIZE_CHANGED,
            ev.pixel_size.0,
            ev.pixel_size.1,
        );

        if ev.exposed {
            send_window_event(window, EventType::WINDOW_EXPOSED, 0, 0);
        }
        if ev.occluded {
            send_window_event(window, EventType::WINDOW_OCCLUDED, 0, 0);
        }
    }

    /* While we can't get window position from the compositor, we do at least know
     * what monitor we're on, so let's send move events that put the window at the
     * center of the whatever display the wl_surface_listener events give us.
     */
    /// Translation of `Wayland_UpdateWindowPosition()`.
    pub(crate) fn wayland_update_window_position(&self, window: WindowID) {
        let Some((display_id, x, y, is_popup)) = self.with_data(|d| {
            let scale_to_display_enabled = d.scale_to_display_enabled;
            // A window may not be on any displays if minimized.
            let key = *d.window(window)?.outputs.last()?;
            let display = d.output(key)?;
            let (x, y) = if !scale_to_display_enabled {
                (display.logical.x, display.logical.y)
            } else {
                (display.pixel.x, display.pixel.y)
            };
            let display_id = display.display;
            let wind = d.window_mut(window)?;

            /* We want to send a very very specific combination here:
             *
             * 1. A coordinate that tells the application what display we're on
             * 2. Exactly (0, 0)
             *
             * Part 1 is useful information but is also really important for
             * ensuring we end up on the right display for fullscreen, while
             * part 2 is important because numerous applications use a specific
             * combination of GetWindowPosition and GetGlobalMouseState, and of
             * course neither are supported by Wayland. Since global mouse will
             * fall back to just GetMouseState, we need the window position to
             * be zero so the cursor math works without it going off in some
             * random direction. See UE5 Editor for a notable example of this!
             *
             * This may be an issue some day if we're ever able to implement
             * SDL_GetDisplayUsableBounds!
             *
             * -flibit
             */
            wind.last_display_id = display_id;
            Some((
                display_id,
                x,
                y,
                wind.shell_surface_type == ShellSurfaceType::XdgPopup,
            ))
        }) else {
            return;
        };

        if !is_popup {
            send_window_event(window, EventType::WINDOW_MOVED, x, y);
            send_window_event(
                window,
                EventType::WINDOW_DISPLAY_CHANGED,
                display_id as i32,
                0,
            );
        }
    }

    /// Translation of `SetFullscreen()`: `output` is the registry name of
    /// the output.
    fn set_fullscreen(&self, window: WindowID, output: Option<u32>, fullscreen: bool) {
        let Some(win) = snapshot(window) else {
            return;
        };
        enum Libdecor {
            Set(*mut libdecor_frame, *mut wl_proxy),
            Unset(*mut libdecor_frame),
        }
        let Some(libdecor_op) = self.with_data(|d| {
            let output_raw = output.and_then(|k| d.output(k)).map(|o| o.output.raw());
            // SAFETY: the output is alive while the data is borrowed.
            let output_obj = output_raw
                .and_then(|raw| unsafe { Obj::<WlOutput>::from_raw(raw.cast(), &self.conn) });
            let wind = d.window_mut(window)?;
            if wind.shell_surface_type == ShellSurfaceType::Libdecor {
                // Can't do anything yet, wait for ShowWindow
                let frame = wind.shell_surface.libdecor_frame.as_ref()?.raw();

                wind.fullscreen_exclusive = if output.is_some() {
                    win.fullscreen_exclusive
                } else {
                    false
                };
                Some(Some(if fullscreen {
                    Libdecor::Set(frame, output_raw.unwrap_or(std::ptr::null_mut()))
                } else {
                    Libdecor::Unset(frame)
                }))
            } else if wind.shell_surface_type == ShellSurfaceType::XdgToplevel {
                // Can't do anything yet, wait for ShowWindow
                let toplevel = wind.shell_surface.xdg_toplevel.as_ref()?;

                wind.fullscreen_exclusive = if output.is_some() {
                    win.fullscreen_exclusive
                } else {
                    false
                };
                if fullscreen {
                    toplevel.set_fullscreen(output_obj);
                } else {
                    toplevel.unset_fullscreen();
                }
                Some(None)
            } else {
                Some(None)
            }
        }) else {
            return;
        };

        if let (Some(op), Some(l)) = (libdecor_op, libdecor_syms(self.syms())) {
            // SAFETY: the frame and output are alive.
            unsafe {
                match op {
                    Libdecor::Set(frame, output) => {
                        (l.libdecor_frame_set_fullscreen)(frame, output)
                    }
                    Libdecor::Unset(frame) => (l.libdecor_frame_unset_fullscreen)(frame),
                }
            }
            resume_pending_panic();
        }

        self.with_data(|d| self.add_pending_state_sync(d, window));
    }

    /// Translation of `UpdateWindowFullscreen()`.
    fn update_window_fullscreen(&self, window: WindowID, fullscreen: bool) {
        let found = self.with_data(|d| {
            let wind = d.window_mut(window)?;
            wind.is_fullscreen = fullscreen;
            Some(())
        });
        if found.is_none() {
            return;
        }
        let flags = window_flags(window);

        if fullscreen {
            if !flags.contains(WindowFlags::FULLSCREEN) {
                let _ = with_window(window, |w| {
                    w.current_fullscreen_mode = w.requested_fullscreen_mode
                });
                send_window_event(window, EventType::WINDOW_ENTER_FULLSCREEN, 0, 0);
                let _ = update_fullscreen_mode(window, FullscreenOp::Enter, false);

                /* Set the output for exclusive fullscreen windows when entering fullscreen from a
                 * compositor event, or if the fullscreen parameters were changed between the initial
                 * fullscreen request and now, to ensure that the window is on the correct output,
                 * as requested by the client.
                 */
                let Some(win) = snapshot(window) else {
                    return;
                };
                let output = self.with_data(|d| {
                    let wind = d.window(window)?;
                    if win.fullscreen_exclusive
                        && (!wind.fullscreen_exclusive || !wind.fullscreen_was_positioned)
                    {
                        let key = d
                            .output_for_display(win.current_fullscreen_mode.display_id)?
                            .registry_id;
                        d.window_mut(window)?.fullscreen_was_positioned = true;
                        Some(key)
                    } else {
                        None
                    }
                });
                if let Some(output) = output {
                    self.set_fullscreen(window, Some(output), true);
                }
            }
        } else {
            // Don't change the fullscreen flags if the window is hidden or being hidden.
            let is_hiding = with_window(window, |w| w.is_hiding).unwrap_or(false);
            if flags.contains(WindowFlags::FULLSCREEN)
                && !is_hiding
                && !flags.contains(WindowFlags::HIDDEN)
            {
                send_window_event(window, EventType::WINDOW_LEAVE_FULLSCREEN, 0, 0);
                let _ = update_fullscreen_mode(window, FullscreenOp::Leave, false);
                self.with_data(|d| {
                    if let Some(wind) = d.window_mut(window) {
                        wind.fullscreen_was_positioned = false;
                    }
                });

                /* Send a move event, in case it was deferred while the fullscreen window was moving and
                 * on multiple outputs.
                 */
                self.wayland_update_window_position(window);
            }
        }
    }

    // -----------------------------------------------------------------------
    // Listeners
    // -----------------------------------------------------------------------

    /// A new surface frame callback for the window (`surface_frame_listener`).
    fn new_surface_frame_callback(&self, wd: &WaylandWindowData) -> Proxy<WlCallback> {
        let mut cb = wd.surface().frame();
        let window = wd.sdlwindow;
        self.listen(&mut cb, move |v, _, _| v.surface_frame_done(window));
        cb
    }

    /// Translation of `surface_frame_done()`.
    fn surface_frame_done(&self, window: WindowID) {
        let Some((configure_ack, became_shown, suspended)) = self.with_data(|d| {
            let compositor_version = d.g.compositor.as_ref().map_or(0, |c| c.version());
            let wind = d.window_mut(window)?;

            /* XXX: This is needed to work around an Nvidia egl-wayland bug due to buffer coordinates
             *      being used with wl_surface_damage, which causes part of the output to not be
             *      updated when using a viewport with an output region larger than the source region.
             */
            damage_all(wind.surface(), compositor_version);

            wind.pending_state_commit = false;

            let mut configure_ack = false;
            if wind.shell_surface_type == ShellSurfaceType::XdgToplevel {
                if wind.pending_config_ack {
                    wind.pending_config_ack = false;
                    configure_ack = true;
                }
            } else {
                wind.resizing = false;
            }

            let mut became_shown = false;
            if wind.shell_surface_status == ShellSurfaceStatus::WaitingForFrame {
                wind.shell_surface_status = ShellSurfaceStatus::Shown;
                became_shown = true;
            }
            Some((configure_ack, became_shown, wind.suspended))
        }) else {
            return;
        };

        if configure_ack {
            self.configure_window_geometry(window);
            self.with_data(|d| {
                if let Some(wind) = d.window(window) {
                    if let Some(xdg_surface) = &wind.shell_surface.xdg_surface {
                        xdg_surface.ack_configure(wind.shell_surface.serial);
                    }
                }
            });
        }

        if became_shown {
            // If any child windows are waiting on this window to be shown, show them now
            let children = with_window(window, |w| w.children.clone()).unwrap_or_default();
            for w in children {
                let Some((status, reparenting_required)) = self.with_data(|d| {
                    d.window(w)
                        .map(|cw| (cw.shell_surface_status, cw.reparenting_required))
                }) else {
                    continue;
                };
                if status == ShellSurfaceStatus::ShowPending {
                    self.wayland_show_window(w);
                } else if reparenting_required {
                    let Some(cwin) = snapshot(w) else {
                        continue;
                    };
                    let _ = self.wayland_set_window_parent(w, cwin.parent);
                    if cwin.flags.contains(WindowFlags::MODAL) {
                        let _ = self.wayland_set_window_modal(w, true);
                    }
                }
            }

            /* If the window was initially set to the suspended state, send the occluded event now,
             * as we don't want to mark the window as occluded until at least one frame has been submitted.
             */
            if suspended {
                send_window_event(window, EventType::WINDOW_OCCLUDED, 0, 0);
            }
        }

        self.with_data(|d| {
            if let Some(wind) = d.window(window) {
                // (the old callback is destroyed as it is replaced)
                let cb = self.new_surface_frame_callback(wind);
                if let Some(wind) = d.window_mut(window) {
                    wind.surface_frame_callback = Some(cb);
                }
            }
        });
    }

    /// Translation of `handle_xdg_surface_configure()`.
    fn handle_xdg_surface_configure(
        &self,
        window: WindowID,
        xdg: Obj<'_, XdgSurface>,
        serial: u32,
    ) {
        enum Action {
            Configure,
            Expose,
            None,
        }
        /* Interactive resizes are throttled by acking and committing only the most recent configuration at
         * the next frame callback, or certain combinations of clients and compositors can exhibit severe lag
         * when resizing.
         */
        let action = self.with_data(|d| {
            let Some(wind) = d.window_mut(window) else {
                return Action::None;
            };
            wind.shell_surface.serial = serial;
            if !wind.resizing {
                wind.pending_config_ack = false;
                Action::Configure
            } else if !wind.pending_config_ack {
                wind.pending_config_ack = true;
                Action::Expose
            } else {
                Action::None
            }
        });
        match action {
            Action::Configure => {
                self.configure_window_geometry(window);
                xdg.ack_configure(serial);
            }
            Action::Expose => {
                // Always send an exposure event during a new frame to ensure forward progress if the frame callback already occurred.
                send_window_event(window, EventType::WINDOW_EXPOSED, 0, 0);
            }
            Action::None => {}
        }

        self.with_data(|d| {
            if let Some(wind) = d.window_mut(window) {
                if wind.shell_surface_status == ShellSurfaceStatus::WaitingForConfigure {
                    wind.shell_surface_status = ShellSurfaceStatus::WaitingForFrame;
                }
            }
        });
    }

    /// Translation of `handle_xdg_toplevel_configure()`.
    fn handle_xdg_toplevel_configure(
        &self,
        window: WindowID,
        mut width: i32,
        mut height: i32,
        states: &[u8],
    ) {
        let s = parse_toplevel_states(states);
        let Some(win) = snapshot(window) else {
            return;
        };

        /* Configure sizes are in window geometry space; add the border insets, if any, to get
         * the surface size. Maximized and fullscreen windows must use the configured size as-is,
         * so a change in inset applicability requires re-adopting even an unchanged size, and
         * sizes adopted from the cached window size are converted back to geometry space for
         * the last_configure store.
         */
        let Some((insets_apply, new_configure_size, insets)) = self.with_data(|d| {
            let wind = d.window_mut(window)?;
            wind.toplevel_constraints = s.constraints;
            let insets_apply = wind.enable_insets && !s.fullscreen && !s.maximized;

            // When resizing, dimensions other than 0 are a maximum.
            let new_configure_size = width != wind.last_configure.width
                || height != wind.last_configure.height
                || insets_apply != wind.last_configure.insets_apply;

            let insets = if insets_apply {
                border_insets(wind, &win.props)
            } else {
                BorderInsets::default()
            };

            determine_resize_axis(wind, width, height, s.resizing);
            Some((insets_apply, new_configure_size, insets))
        }) else {
            return;
        };

        self.update_window_fullscreen(window, s.fullscreen);

        /* Always send a maximized/restore event; if the event is redundant it will
         * automatically be discarded (see src/events/SDL_windowevents.c)
         *
         * No, we do not get minimize events from xdg-shell, however, the minimized
         * state can be programmatically set. The meaning of 'minimized' is compositor
         * dependent, but in general, we can assume that the flag should remain set until
         * the next focused configure event occurs.
         */
        let flags = window_flags(window);
        if s.active || !flags.contains(WindowFlags::MINIMIZED) {
            if flags.contains(WindowFlags::MINIMIZED) {
                // If we were minimized, send a restored event before possibly sending maximized.
                send_window_event(window, EventType::WINDOW_RESTORED, 0, 0);
            }
            send_window_event(
                window,
                if s.maximized && !s.fullscreen {
                    EventType::WINDOW_MAXIMIZED
                } else {
                    EventType::WINDOW_RESTORED
                },
                0,
                0,
            );
        }

        let Some(win) = snapshot(window) else {
            return;
        };
        self.with_data(|d| {
            let Some(wind) = d.window_mut(window) else {
                return;
            };
            let scale = window_scale(win.flags, wind);

            if !s.fullscreen {
                /* xdg_toplevel spec states that this is a suggestion.
                 * Ignore if less than or greater than max/min size.
                 */
                if win.flags.contains(WindowFlags::RESIZABLE) || s.maximized {
                    if width == 0 {
                        /* This happens when the compositor indicates that the size is
                         * up to the client, so use the cached window size here.
                         */
                        if s.floating {
                            width = win.floating.w;

                            // Clamp the window to the toplevel bounds, if any are set.
                            if wind.shell_surface_status == ShellSurfaceStatus::WaitingForConfigure
                                && wind.toplevel_bounds.width != 0
                            {
                                width = wind.toplevel_bounds.width.min(width);
                            }
                        } else {
                            width = win.windowed.w;
                        }

                        if !wind.scale_to_display {
                            wind.requested.logical_width = width;
                        } else {
                            wind.requested.pixel_width = width;
                            width = pixel_to_point(scale, width);
                            wind.requested.logical_width = width;
                        }
                        width = (width - (insets.left + insets.right)).max(1);
                    } else if new_configure_size {
                        /* Don't apply the supplied dimensions if they haven't changed from the last configuration
                         * event, or a newer size set programmatically can be overwritten by old data.
                         */

                        wind.requested.logical_width = width + (insets.left + insets.right);

                        if wind.scale_to_display {
                            wind.requested.pixel_width =
                                point_to_pixel(scale, wind.requested.logical_width);
                        }
                    }
                    if height == 0 {
                        /* This happens when the compositor indicates that the size is
                         * up to the client, so use the cached window size here.
                         */
                        if s.floating {
                            height = win.floating.h;

                            // Clamp the window to the toplevel bounds, if any are set.
                            if wind.shell_surface_status == ShellSurfaceStatus::WaitingForConfigure
                                && wind.toplevel_bounds.height != 0
                            {
                                height = wind.toplevel_bounds.height.min(height);
                            }
                        } else {
                            height = win.windowed.h;
                        }

                        if !wind.scale_to_display {
                            wind.requested.logical_height = height;
                        } else {
                            wind.requested.pixel_height = height;
                            height = pixel_to_point(scale, height);
                            wind.requested.logical_height = height;
                        }
                        height = (height - (insets.top + insets.bottom)).max(1);
                    } else if new_configure_size {
                        /* Don't apply the supplied dimensions if they haven't changed from the last configuration
                         * event, or a newer size set programmatically can be overwritten by old data.
                         */
                        wind.requested.logical_height = height + (insets.top + insets.bottom);

                        if wind.scale_to_display {
                            wind.requested.pixel_height =
                                point_to_pixel(scale, wind.requested.logical_height);
                        }
                    }
                } else {
                    /* If we're a fixed-size, non-maximized window, we know our size for sure.
                     * Always assume the configure is wrong.
                     */
                    if !wind.scale_to_display {
                        width = win.floating.w;
                        height = win.floating.h;
                        wind.requested.logical_width = width;
                        wind.requested.logical_height = height;
                    } else {
                        wind.requested.pixel_width = win.floating.w;
                        wind.requested.pixel_height = win.floating.h;
                        width = pixel_to_point(scale, win.floating.w);
                        height = pixel_to_point(scale, win.floating.h);
                        wind.requested.logical_width = width;
                        wind.requested.logical_height = height;
                    }
                    width = (width - (insets.left + insets.right)).max(1);
                    height = (height - (insets.top + insets.bottom)).max(1);
                }

                /* Notes on the spec and implementations:
                 *
                 * - The content limits are only a hint, which the compositor is free to ignore,
                 *   so apply them manually when appropriate.
                 *
                 * - Only floating windows are truly safe to resize: maximized windows must have
                 *   their exact dimensions respected, or a protocol violation can occur, and tiled
                 *   windows can technically use dimensions smaller than the ones supplied by the
                 *   compositor, but doing so can cause visual glitches and odd behavior. In these cases
                 *   it's best to use the supplied dimensions and use a viewport + mask to enforce the
                 *   size limits and/or aspect ratio.
                 */
                if s.floating {
                    if !wind.scale_to_display {
                        let (w, h) = apply_geometry_limits(
                            &win,
                            wind.resizing,
                            wind.resize_edge,
                            wind.requested.logical_width,
                            wind.requested.logical_height,
                        );
                        wind.requested.logical_width = w;
                        wind.requested.logical_height = h;
                    } else {
                        let (w, h) = apply_geometry_limits(
                            &win,
                            wind.resizing,
                            wind.resize_edge,
                            wind.requested.pixel_width,
                            wind.requested.pixel_height,
                        );
                        wind.requested.pixel_width = w;
                        wind.requested.pixel_height = h;

                        wind.requested.logical_width =
                            pixel_to_point(scale, wind.requested.pixel_width);
                        wind.requested.logical_height =
                            pixel_to_point(scale, wind.requested.pixel_height);
                    }
                }
            } else {
                // Fullscreen windows know their exact size.
                if width == 0 || height == 0 {
                    width = wind.requested.logical_width;
                    height = wind.requested.logical_height;
                } else {
                    wind.requested.logical_width = width;
                    wind.requested.logical_height = height;
                }

                if wind.scale_to_display {
                    wind.requested.pixel_width = point_to_pixel(scale, width);
                    wind.requested.pixel_height = point_to_pixel(scale, height);
                }
            }

            wind.last_configure.width = width;
            wind.last_configure.height = height;
            wind.last_configure.insets_apply = insets_apply;
            wind.floating = s.floating;
            wind.suspended = s.suspended;
            wind.active = s.active;
        });
        let _ = with_window(window, |w| w.core.tiled = s.tiled);
    }

    /// The `xdg_toplevel` listener (`toplevel_listener_xdg`).
    fn handle_xdg_toplevel_event(&self, window: WindowID, event: XdgToplevelEvent<'_>) {
        match event {
            XdgToplevelEvent::Configure {
                width,
                height,
                states,
            } => {
                self.handle_xdg_toplevel_configure(window, width, height, states);
            }
            XdgToplevelEvent::Close => {
                send_window_event(window, EventType::WINDOW_CLOSE_REQUESTED, 0, 0);
            }
            XdgToplevelEvent::ConfigureBounds { width, height } => {
                self.with_data(|d| {
                    if let Some(wind) = d.window_mut(window) {
                        wind.toplevel_bounds.width = width;
                        wind.toplevel_bounds.height = height;
                    }
                });
            }
            XdgToplevelEvent::WmCapabilities { capabilities } => {
                let wm_caps = wm_caps_from_xdg(capabilities);
                self.with_data(|d| {
                    if let Some(wind) = d.window_mut(window) {
                        wind.wm_caps = wm_caps;
                    }
                });
            }
        }
    }

    /// Translation of `handle_xdg_popup_configure()`.
    fn handle_xdg_popup_configure(
        &self,
        window: WindowID,
        mut x: i32,
        mut y: i32,
        mut width: i32,
        mut height: i32,
    ) {
        let Some(win) = snapshot(window) else {
            return;
        };
        let parent_flags = win.parent.map(window_flags).unwrap_or_default();
        let syms = self.syms().clone();
        let Some(parent_scale) = self.with_data(|d| {
            let wind = d.window(window)?;
            let parent = win.parent.and_then(|p| d.window(p));
            let mut offset_x = 0;
            let mut offset_y = 0;

            // Adjust the position if it was offset for libdecor
            if let Some(parent) = parent {
                adjust_popup_offset(&syms, parent, &mut offset_x, &mut offset_y);
            }
            x -= offset_x;
            y -= offset_y;

            /* This happens when the compositor indicates that the size is
             * up to the client, so use the cached window size here.
             */
            if width == 0 || height == 0 {
                width = win.floating.w;
                height = win.floating.h;
            }

            let scale = window_scale(win.flags, wind);
            let parent_scale = parent.map_or(1.0, |p| window_scale(parent_flags, p));
            let wind = d.window_mut(window)?;

            /* Don't apply the supplied dimensions if they haven't changed from the last configuration
             * event, or a newer size set programmatically can be overwritten by old data.
             */
            if width != wind.last_configure.width || height != wind.last_configure.height {
                wind.requested.logical_width = width;
                wind.requested.logical_height = height;

                if wind.scale_to_display {
                    wind.requested.pixel_width = point_to_pixel(scale, width);
                    wind.requested.pixel_height = point_to_pixel(scale, height);
                }
            }

            Some(if wind.scale_to_display {
                Some(parent_scale)
            } else {
                None
            })
        }) else {
            return;
        };

        if let Some(parent_scale) = parent_scale {
            x = point_to_pixel(parent_scale, x);
            y = point_to_pixel(parent_scale, y);
        }

        send_window_event(window, EventType::WINDOW_MOVED, x, y);

        self.with_data(|d| {
            if let Some(wind) = d.window_mut(window) {
                wind.last_configure.width = width;
                wind.last_configure.height = height;

                if wind.shell_surface_status == ShellSurfaceStatus::WaitingForConfigure {
                    wind.shell_surface_status = ShellSurfaceStatus::WaitingForFrame;
                }
            }
        });
    }

    /// The `xdg_popup` listener (`_xdg_popup_listener`).
    fn handle_xdg_popup_event(&self, window: WindowID, event: XdgPopupEvent) {
        match event {
            XdgPopupEvent::Configure {
                x,
                y,
                width,
                height,
            } => {
                self.handle_xdg_popup_configure(window, x, y, width, height);
            }
            XdgPopupEvent::PopupDone => {
                send_window_event(window, EventType::WINDOW_CLOSE_REQUESTED, 0, 0);
            }
            XdgPopupEvent::Repositioned { .. } => {
                // No-op, configure does all the work we care about
            }
        }
    }

    /// Translation of `handle_xdg_toplevel_decoration_configure()`.
    fn handle_xdg_toplevel_decoration_configure(
        &self,
        window: WindowID,
        mode: ZxdgToplevelDecorationV1Mode,
    ) {
        /* If the compositor tries to force CSD anyway, bail on direct XDG support
         * and fall back to libdecor, it will handle these events from then on.
         *
         * To do this we have to fully unmap, then map with libdecor loaded.
         */
        if mode == ZxdgToplevelDecorationV1Mode::CLIENT_SIDE {
            if window_flags(window).contains(WindowFlags::BORDERLESS) {
                // borderless windows do request CSD, so we got what we wanted
                return;
            }
            if !self.wayland_load_libdecor(true) {
                // libdecor isn't available, so no borders for you... oh well
                return;
            }
            let _ = self.conn.roundtrip();

            self.wayland_hide_window(window);
            self.with_data(|d| {
                if let Some(wind) = d.window_mut(window) {
                    wind.shell_surface_type = ShellSurfaceType::Libdecor;
                }
            });
            self.wayland_show_window(window);
        }
    }

    /// Translation of `OverrideLibdecorLimits()`: on libdecor versions
    /// without `libdecor_frame_get_min_content_size()`, the internal limits
    /// must always be overridden to ensure that very small windows don't
    /// cause errors or crashes.
    fn override_libdecor_limits(&self, frame: *mut libdecor_frame, win: &Win) {
        let Some(l) = libdecor_syms(self.syms()) else {
            return;
        };
        if l.libdecor_frame_get_min_content_size.is_none() {
            // SAFETY: the frame is alive.
            unsafe { (l.libdecor_frame_set_min_content_size)(frame, win.min_w, win.min_h) };
            resume_pending_panic();
        }
    }

    /// Translation of `decoration_frame_configure()`.
    fn decoration_frame_configure(
        &self,
        window: WindowID,
        frame: *mut libdecor_frame,
        configuration: *mut libdecor_configuration,
    ) {
        let Some(l) = libdecor_syms(self.syms()) else {
            return;
        };

        let tiled_states = LIBDECOR_WINDOW_STATE_TILED_LEFT
            | LIBDECOR_WINDOW_STATE_TILED_RIGHT
            | LIBDECOR_WINDOW_STATE_TILED_TOP
            | LIBDECOR_WINDOW_STATE_TILED_BOTTOM;

        let Some(waiting_for_configure) = self.with_data(|d| {
            let wind = d.window_mut(window)?;
            wind.toplevel_constraints = 0;
            Some(wind.shell_surface_status == ShellSurfaceStatus::WaitingForConfigure)
        }) else {
            return;
        };

        // (LibdecorGetMinContentSize(): the minimum content size, if libdecor can tell)
        if waiting_for_configure {
            if let Some(get_min_content_size) = l.libdecor_frame_get_min_content_size {
                let (mut min_w, mut min_h) = (0, 0);
                // SAFETY: the frame is alive; a getter.
                unsafe { get_min_content_size(frame, &mut min_w, &mut min_h) };
                self.with_data(|d| {
                    if let Some(wind) = d.window_mut(window) {
                        wind.system_limits.min_width = min_w;
                        wind.system_limits.min_height = min_h;
                    }
                });
            }
        }

        let mut active = false;
        let mut fullscreen = false;
        let mut maximized = false;
        let mut tiled = false;
        let mut suspended = false;
        // (LIBDECOR_WINDOW_STATE_RESIZING and the constraints are libdecor 0.3)
        let resizing = false;

        // Window State
        let mut window_state: libdecor_window_state = LIBDECOR_WINDOW_STATE_NONE;
        // SAFETY: the configuration is valid during the callback.
        if unsafe { (l.libdecor_configuration_get_window_state)(configuration, &mut window_state) }
        {
            fullscreen = (window_state & LIBDECOR_WINDOW_STATE_FULLSCREEN) != 0;
            maximized = (window_state & LIBDECOR_WINDOW_STATE_MAXIMIZED) != 0;
            active = (window_state & LIBDECOR_WINDOW_STATE_ACTIVE) != 0;
            tiled = (window_state & tiled_states) != 0;
            suspended = (window_state & LIBDECOR_WINDOW_STATE_SUSPENDED) != 0;
        }
        let floating = !(fullscreen || maximized || tiled);

        // The content size of the configuration, if it has one.
        let content_size = || {
            let (mut w, mut h): (c_int, c_int) = (0, 0);
            // SAFETY: the configuration and frame are valid during the callback.
            let ok = unsafe {
                (l.libdecor_configuration_get_content_size)(configuration, frame, &mut w, &mut h)
            };
            ok.then_some((w, h))
        };

        self.update_window_fullscreen(window, fullscreen);

        /* Always send a maximized/restore event; if the event is redundant it will
         * automatically be discarded (see src/events/SDL_windowevents.c)
         *
         * No, we do not get minimize events from libdecor, however, the minimized
         * state can be programmatically set. The meaning of 'minimized' is compositor
         * dependent, but in general, we can assume that the flag should remain set until
         * the next focused configure event occurs.
         */
        let flags = window_flags(window);
        if active || !flags.contains(WindowFlags::MINIMIZED) {
            if flags.contains(WindowFlags::MINIMIZED) {
                // If we were minimized, send a restored event before possibly sending maximized.
                send_window_event(window, EventType::WINDOW_RESTORED, 0, 0);
            }
            send_window_event(
                window,
                if maximized && !fullscreen {
                    EventType::WINDOW_MAXIMIZED
                } else {
                    EventType::WINDOW_RESTORED
                },
                0,
                0,
            );
        }

        let Some(win) = snapshot(window) else {
            return;
        };

        /* For fullscreen or fixed-size windows we know our size.
         * Always assume the configure is wrong.
         */
        let fixed_size = !fullscreen && !win.flags.contains(WindowFlags::RESIZABLE) && !maximized;
        // (the content size, read before borrowing the data: libdecor only reads its state)
        let configured_size = content_size();

        let Some(started_resize) = self.with_data(|d| {
            let wind = d.window_mut(window)?;
            let scale = window_scale(win.flags, wind);
            let mut width;
            let mut height;

            if fullscreen {
                match configured_size {
                    None => {
                        width = wind.requested.logical_width;
                        height = wind.requested.logical_height;
                    }
                    Some((w, h)) => {
                        width = w;
                        height = h;
                        // Fullscreen windows know their exact size.
                        wind.requested.logical_width = width;
                        wind.requested.logical_height = height;

                        if wind.scale_to_display {
                            wind.requested.pixel_width = point_to_pixel(scale, width);
                            wind.requested.pixel_height = point_to_pixel(scale, height);
                        }
                    }
                }
            } else {
                if fixed_size {
                    /* If we're a fixed-size, non-maximized window, we know our size for sure.
                     * Always assume the configure is wrong.
                     */
                    if !wind.scale_to_display {
                        width = win.floating.w;
                        height = win.floating.h;
                        wind.requested.logical_width = width;
                        wind.requested.logical_height = height;
                    } else {
                        wind.requested.pixel_width = win.floating.w;
                        wind.requested.pixel_height = win.floating.h;
                        width = pixel_to_point(scale, win.floating.w);
                        height = pixel_to_point(scale, win.floating.h);
                        wind.requested.logical_width = width;
                        wind.requested.logical_height = height;
                    }
                } else {
                    /* XXX: The libdecor cairo plugin sends bogus content sizes that add the
                     *      height of the title bar when transitioning from a fixed-size to
                     *      floating state. Ignore the sent window dimensions in this case,
                     *      in favor of the cached value to avoid the window increasing in
                     *      size after every state transition.
                     *
                     *      https://gitlab.freedesktop.org/libdecor/libdecor/-/issues/34
                     */
                    match configured_size {
                        Some((w, h))
                            if !(floating
                                && (!wind.floating
                                    && !win.flags.contains(WindowFlags::BORDERLESS))) =>
                        {
                            width = w;
                            height = h;
                        }
                        _ => {
                            width = 0;
                            height = 0;
                        }
                    }

                    let new_configure_size =
                        width != wind.last_configure.width || height != wind.last_configure.height;
                    determine_resize_axis(wind, width, height, resizing);

                    if width == 0 {
                        /* This happens when we're being restored from a non-floating state,
                         * or the compositor indicates that the size is up to the client, so
                         * used the cached window size here.
                         */
                        if floating {
                            width = win.floating.w;

                            // Clamp the window to the toplevel bounds, if any are set.
                            if wind.shell_surface_status == ShellSurfaceStatus::WaitingForConfigure
                                && wind.toplevel_bounds.width != 0
                            {
                                width = wind.toplevel_bounds.width.min(width);
                            }
                        } else {
                            width = win.windowed.w;
                        }

                        if !wind.scale_to_display {
                            wind.requested.logical_width = width;
                        } else {
                            wind.requested.pixel_width = width;
                            width = pixel_to_point(scale, width);
                            wind.requested.logical_width = width;
                        }
                    } else {
                        /* Don't apply the supplied dimensions if they haven't changed from the last configuration
                         * event, or a newer size set programmatically can be overwritten by old data.
                         *
                         * If a client takes a long time to present the first frame after creating the window, a
                         * configure event to set the suspended state may arrive with the content size increased
                         * by the decoration dimensions, which should also be ignored.
                         */
                        if new_configure_size
                            && !(wind.shell_surface_status == ShellSurfaceStatus::WaitingForFrame
                                && wind.suspended != suspended)
                        {
                            wind.requested.logical_width = width;

                            if wind.scale_to_display {
                                wind.requested.pixel_width = point_to_pixel(scale, width);
                            }
                        }
                    }

                    if height == 0 {
                        /* This happens when we're being restored from a non-floating state,
                         * or the compositor indicates that the size is up to the client, so
                         * used the cached window size here.
                         */
                        if floating {
                            height = win.floating.h;

                            // Clamp the window to the toplevel bounds, if any are set.
                            if wind.shell_surface_status == ShellSurfaceStatus::WaitingForConfigure
                                && wind.toplevel_bounds.height != 0
                            {
                                height = wind.toplevel_bounds.height.min(height);
                            }
                        } else {
                            height = win.windowed.h;
                        }

                        if !wind.scale_to_display {
                            wind.requested.logical_height = height;
                        } else {
                            wind.requested.pixel_height = height;
                            height = pixel_to_point(scale, height);
                            wind.requested.logical_height = height;
                        }
                    } else {
                        /* Don't apply the supplied dimensions if they haven't changed from the last configuration
                         * event, or a newer size set programmatically can be overwritten by old data.
                         *
                         * If a client takes a long time to present the first frame after creating the window, a
                         * configure event to set the suspended state may arrive with the content size increased
                         * by the decoration dimensions, which should also be ignored.
                         */
                        if new_configure_size
                            && !(wind.shell_surface_status == ShellSurfaceStatus::WaitingForFrame
                                && wind.suspended != suspended)
                        {
                            wind.requested.logical_height = height;

                            if wind.scale_to_display {
                                wind.requested.pixel_height = point_to_pixel(scale, height);
                            }
                        }
                    }
                }

                /* Notes on the spec and implementations:
                 *
                 * - The content limits are only a hint, which the compositor is free to ignore,
                 *   so apply them manually when appropriate.
                 *
                 * - Only floating windows are truly safe to resize: maximized windows must have
                 *   their exact dimensions respected, or a protocol violation can occur, and tiled
                 *   windows can technically use dimensions smaller than the ones supplied by the
                 *   compositor, but doing so can cause odd behavior. In these cases it's best to use
                 *   the supplied dimensions and use a viewport + mask to enforce the size limits and/or
                 *   aspect ratio.
                 */
                if floating {
                    if !wind.scale_to_display {
                        let (w, h) = apply_geometry_limits(
                            &win,
                            wind.resizing,
                            wind.resize_edge,
                            wind.requested.logical_width,
                            wind.requested.logical_height,
                        );
                        wind.requested.logical_width = w;
                        wind.requested.logical_height = h;
                    } else {
                        let (w, h) = apply_geometry_limits(
                            &win,
                            wind.resizing,
                            wind.resize_edge,
                            wind.requested.pixel_width,
                            wind.requested.pixel_height,
                        );
                        wind.requested.pixel_width = w;
                        wind.requested.pixel_height = h;

                        wind.requested.logical_width =
                            pixel_to_point(scale, wind.requested.pixel_width);
                        wind.requested.logical_height =
                            pixel_to_point(scale, wind.requested.pixel_height);
                    }
                }
            }

            // Store the new state.
            let started_resize = !wind.resizing && resizing;
            wind.last_configure.width = width;
            wind.last_configure.height = height;
            wind.floating = floating;
            wind.suspended = suspended;
            wind.active = active;
            wind.resizing = resizing;
            Some(started_resize)
        }) else {
            return;
        };
        let _ = with_window(window, |w| w.core.tiled = tiled);

        if fixed_size {
            self.override_libdecor_limits(frame, &win);
        }

        // (updating the window manager capabilities needs libdecor 0.3)

        if !resizing || started_resize {
            /* Calculate the new window geometry and commit the changes on the libdecor side.
             *
             * XXX: This will potentially leave un-acked configurations, but libdecor invalidates the
             *      configuration upon returning from the frame event, so there is nothing that can be
             *      done, unless libdecor adds the ability to copy or refcount the configuration state
             *      to apply later.
             */
            self.configure_window_geometry(window);
            let (w, h) = self.with_data(|d| {
                d.window(window).map_or((0, 0), |wd| {
                    (wd.current.logical_width, wd.current.logical_height)
                })
            });
            // SAFETY: the frame and configuration are valid during the
            // callback; the state is freed once.
            unsafe {
                let state = (l.libdecor_state_new)(w, h);
                (l.libdecor_frame_commit)(frame, state, configuration);
                (l.libdecor_state_free)(state);
            }

            // Always send an exposure event during a new frame to ensure forward progress if the frame callback already occurred.
            if started_resize {
                send_window_event(window, EventType::WINDOW_EXPOSED, 0, 0);
            }
        }

        self.with_data(|d| {
            if let Some(wind) = d.window_mut(window) {
                if wind.shell_surface_status == ShellSurfaceStatus::WaitingForConfigure {
                    wind.shell_surface_status = ShellSurfaceStatus::WaitingForFrame;
                }
            }
        });
    }

    /// Translation of `Wayland_HandlePreferredScaleChanged()`.
    fn handle_preferred_scale_changed(&self, window: WindowID, mut factor: f64) {
        let Some(win) = snapshot(window) else {
            return;
        };
        let reconfigure = self.with_data(|d| {
            let window_data = d.window_mut(window)?;
            let old_factor = window_data.scale_factor;

            // Round the scale factor if viewports aren't available.
            if window_data.viewport.is_none() {
                factor = factor.ceil();
            }

            if factor == old_factor {
                return None;
            }
            window_data.scale_factor = factor;
            let scale = window_scale(win.flags, window_data);

            if window_data.scale_to_display {
                /* If the window is in the floating state with a user/application specified size, calculate the new
                 * logical size from the backbuffer size. Otherwise, use the fixed underlying logical size to calculate
                 * the new backbuffer dimensions.
                 */
                if window_data.floating {
                    /* Some compositors will send a configure event immediately after a scale event, however,
                     * this event can contain the old logical size, and should be ignored, or the window can
                     * incorrectly change size when moved between displays with differing scale factors.
                     *
                     * Store the last requested logical size as the last configure size, so a configure event
                     * with the old size will be ignored. Configure sizes are in window geometry space,
                     * so subtract any border insets.
                     */
                    let insets = border_insets(window_data, &win.props);
                    window_data.last_configure.width =
                        (window_data.requested.logical_width - (insets.left + insets.right)).max(1);
                    window_data.last_configure.height = (window_data.requested.logical_height
                        - (insets.top + insets.bottom))
                        .max(1);

                    window_data.requested.logical_width =
                        pixel_to_point(scale, window_data.requested.pixel_width);
                    window_data.requested.logical_height =
                        pixel_to_point(scale, window_data.requested.pixel_height);
                } else {
                    window_data.requested.pixel_width =
                        point_to_pixel(scale, window_data.requested.logical_width);
                    window_data.requested.pixel_height =
                        point_to_pixel(scale, window_data.requested.logical_height);
                }
            }

            Some(
                win.flags.contains(WindowFlags::HIGH_PIXEL_DENSITY) || window_data.scale_to_display,
            )
        });

        if reconfigure == Some(true) {
            self.configure_window_geometry(window);
            self.commit_libdecor_frame(window);
        }
    }

    /// Translation of `Wayland_MaybeUpdateScaleFactor()`.
    fn maybe_update_scale_factor(&self, window: WindowID) {
        let factor = self.with_data(|d| {
            let wind = d.window(window)?;

            /* If the fractional scale protocol is present or the core protocol supports the
             * preferred buffer scale event, the compositor will explicitly tell the application
             * what scale it wants via these events, so don't try to determine the scale factor
             * from which displays the surface has entered.
             */
            if wind.fractional_scale.is_some()
                || wind.surface().version() >= WlSurface::PREFERRED_BUFFER_SCALE_SINCE_VERSION
            {
                return None;
            }

            if !wind.outputs.is_empty() {
                // Check every display's factor, use the highest
                let mut factor: f64 = 0.0;
                for &key in &wind.outputs {
                    if let Some(internal) = d.output(key) {
                        factor = factor.max(internal.scale_factor);
                    }
                }
                Some(factor)
            } else {
                // All outputs removed, just fall back.
                Some(wind.scale_factor)
            }
        });

        if let Some(factor) = factor {
            self.handle_preferred_scale_changed(window, factor);
        }
    }

    /// Translation of `Wayland_RemoveOutputFromWindow()`: `display_data` is
    /// the registry name of the output.
    pub(crate) fn wayland_remove_output_from_window(&self, window: WindowID, display_data: u32) {
        let update = self.with_data(|d| {
            let wind = d.window_mut(window)?;
            wind.outputs.retain(|&o| o != display_data);
            Some(!wind.outputs.is_empty() && (!wind.is_fullscreen || wind.outputs.len() == 1))
        });

        if update == Some(true) {
            self.wayland_update_window_position(window);
            self.maybe_update_scale_factor(window);
        }
    }

    /// The `wl_surface` listener (`surface_listener`).
    fn handle_surface_event(
        &self,
        window: WindowID,
        surface: Obj<'_, WlSurface>,
        event: WlSurfaceEvent<'_>,
    ) {
        match event {
            WlSurfaceEvent::Enter { output } => {
                // Translation of `handle_surface_enter()`.
                let Some(output) = output else {
                    return;
                };
                if !output.has_tag(&SDL_WAYLAND_OUTPUT_TAG)
                    || !surface.has_tag(&SDL_WAYLAND_SURFACE_TAG)
                {
                    return;
                }
                let key = output.user_data() as u32;

                let update = self.with_data(|d| {
                    let wind = d.window_mut(window)?;
                    wind.outputs.push(key);
                    Some(!wind.is_fullscreen || wind.outputs.len() == 1)
                });

                // Update the scale factor after the move so that fullscreen outputs are updated.
                if update == Some(true) {
                    self.wayland_update_window_position(window);
                    self.maybe_update_scale_factor(window);
                }
            }
            WlSurfaceEvent::Leave { output } => {
                // Translation of `handle_surface_leave()`.
                let Some(output) = output else {
                    return;
                };
                if !output.has_tag(&SDL_WAYLAND_OUTPUT_TAG)
                    || !surface.has_tag(&SDL_WAYLAND_SURFACE_TAG)
                {
                    return;
                }

                self.wayland_remove_output_from_window(window, output.user_data() as u32);
            }
            WlSurfaceEvent::PreferredBufferScale { factor } => {
                /* The spec is unclear on how this interacts with the fractional scaling protocol,
                 * so, for now, assume that the fractional scaling protocol takes priority and
                 * only listen to this event if the fractional scaling protocol is not present.
                 */
                let fractional = self.with_data(|d| {
                    d.window(window)
                        .is_some_and(|w| w.fractional_scale.is_some())
                });
                if !fractional {
                    self.handle_preferred_scale_changed(window, factor as f64);
                }
            }
            WlSurfaceEvent::PreferredBufferTransform { .. } => {
                // Nothing to do here.
            }
        }
    }

    /// Translation of `Wayland_SetKeyboardFocus()`.
    fn set_keyboard_focus(&self, window: WindowID, set_focus: bool) {
        let mut toplevel = window;

        // Find the toplevel parent
        while let Some(parent) = snapshot(toplevel)
            .filter(Win::is_popup)
            .and_then(|w| w.parent)
        {
            toplevel = parent;
        }

        let _ = with_window(toplevel, |w| w.keyboard_focus = Some(window));

        let (is_hiding, is_destroying) =
            with_window(window, |w| (w.is_hiding, w.core.is_destroying)).unwrap_or((true, true));
        if set_focus && !is_hiding && !is_destroying {
            let _ = keyboard::set_keyboard_focus(Some(window));
        }
    }

    // -----------------------------------------------------------------------
    // Parents and sessions
    // -----------------------------------------------------------------------

    /// Translation of `Wayland_SetWindowParent()`.
    pub(crate) fn wayland_set_window_parent(
        &self,
        window: WindowID,
        parent_window: Option<WindowID>,
    ) -> Result<()> {
        self.with_data(|d| {
            let parent_status = parent_window
                .and_then(|p| d.window(p))
                .map(|p| p.shell_surface_status);
            let Some(child_data) = d.window_mut(window) else {
                return;
            };

            child_data.reparenting_required = false;

            if parent_status.is_some_and(|s| s != ShellSurfaceStatus::Shown) {
                // Need to wait for the parent to become mapped, or it's the same as setting a null parent.
                child_data.reparenting_required = true;
                return;
            }

            let child_toplevel = self.toplevel_for_window(d.window(window));
            let parent_toplevel = self.toplevel_for_window(parent_window.and_then(|p| d.window(p)));

            if let Some(child_toplevel) = child_toplevel {
                child_toplevel.set_parent(parent_toplevel);
            }
        });

        Ok(())
    }

    /// Translation of `Wayland_SetWindowModal()`.
    pub(crate) fn wayland_set_window_modal(&self, window: WindowID, modal: bool) -> Result<()> {
        let parent = with_window(window, |w| w.parent)?;
        self.with_data(|d| {
            // (the video core only makes windows with a parent modal)
            let parent_status = parent
                .and_then(|p| d.window(p))
                .map(|p| p.shell_surface_status);
            let Some(data) = d.window_mut(window) else {
                return;
            };

            if parent_status != Some(ShellSurfaceStatus::Shown) {
                // Need to wait for the parent to become mapped before changing modal status.
                data.reparenting_required = true;
                return;
            } else {
                data.reparenting_required = false;
            }

            let Some(toplevel) = self.toplevel_for_window(d.window(window)) else {
                return;
            };

            let Some(dialog_manager) = &d.g.xdg_wm_dialog_v1 else {
                return;
            };
            let toplevel_raw = toplevel.raw();
            let Some(data) = d.windows.iter_mut().find(|w| w.sdlwindow == window) else {
                return;
            };
            if modal {
                if data.xdg_dialog_v1.is_none() {
                    // SAFETY: the toplevel is alive (the data is borrowed).
                    if let Some(toplevel) =
                        unsafe { Obj::<XdgToplevel>::from_raw(toplevel_raw.cast(), &self.conn) }
                    {
                        data.xdg_dialog_v1 = Some(dialog_manager.get_xdg_dialog(toplevel));
                    }
                }

                if let Some(dialog) = &data.xdg_dialog_v1 {
                    dialog.set_modal();
                }
            } else if let Some(dialog) = &data.xdg_dialog_v1 {
                dialog.unset_modal();
            }
        });

        Ok(())
    }

    /// Translation of `Wayland_RegisterToplevelForSession()`.
    fn register_toplevel_for_session(&self, window: WindowID) {
        let Ok(id) = with_window(window, |w| {
            w.properties()
                .get_string(PROP_WINDOW_WAYLAND_WINDOW_ID_STRING)
        }) else {
            return;
        };
        let Some(id) = id.filter(|id| !id.is_empty()) else {
            return;
        };

        self.with_data(|viddata| {
            let Some(toplevel) = self.toplevel_for_window(viddata.window(window)) else {
                return;
            };
            let toplevel_raw = toplevel.raw();

            if let Some(tag_manager) = &viddata.g.xdg_toplevel_tag_manager {
                tag_manager.set_toplevel_tag(toplevel, &id);
            }

            if viddata.g.xdg_session_manager.is_some() {
                self.wayland_create_session(viddata);

                if let Some(session) = &viddata.xdg_session {
                    // Windows added to a session must not have a duplicate ID string, or a protocol error will result.
                    if viddata.windows.iter().any(|w| w.session_id.as_deref() == Some(id.as_str())) {
                        crate::error!(
                            crate::log::Category::Video,
                            "Duplicate window ID string {} found; window will not be added to session",
                            id
                        );
                        return;
                    }

                    // SAFETY: the toplevel is alive (the data is borrowed).
                    let Some(toplevel) = (unsafe { Obj::<XdgToplevel>::from_raw(toplevel_raw.cast(), &self.conn) })
                    else {
                        return;
                    };
                    let toplevel_session = session.restore_toplevel(toplevel, &id);
                    if let Some(data) = viddata.windows.iter_mut().find(|w| w.sdlwindow == window) {
                        data.xdg_toplevel_session = Some(toplevel_session);
                        data.session_id = Some(id.clone());
                    }
                }
            }
        });
    }

    /// Translation of `Wayland_DestroyToplevelSession()`.
    fn destroy_toplevel_session(&self, window: WindowID) {
        let id = with_window(window, |w| {
            w.properties()
                .get_string(PROP_WINDOW_WAYLAND_WINDOW_ID_STRING)
        })
        .ok()
        .flatten();
        let session = self.with_data(|viddata| {
            let VideoData {
                windows,
                xdg_session,
                ..
            } = viddata;
            let data = find_mut(windows, window)?;
            let toplevel_session = data.xdg_toplevel_session.take()?;

            // If the ID string was cleared, remove the window from the session.
            if id.as_deref().is_none_or(str::is_empty) {
                if let (Some(session), Some(session_id)) =
                    (xdg_session.as_ref(), data.session_id.as_deref())
                {
                    session.remove_toplevel(session_id);
                }
            }

            data.session_id = None;
            Some(toplevel_session)
        });
        // (xdg_toplevel_session_v1_destroy())
        drop(session);
    }

    // -----------------------------------------------------------------------
    // Show and hide
    // -----------------------------------------------------------------------

    /// Translation of `Wayland_ShowWindow()`.
    pub(crate) fn wayland_show_window(&self, window: WindowID) {
        let Some(win) = snapshot(window) else {
            return;
        };
        let props = win.props.clone();

        let Some((ty, parent_status)) = self.with_data(|d| {
            let data = d.window(window)?;
            let parent_status = win
                .parent
                .and_then(|p| d.window(p))
                .map(|p| p.shell_surface_status);
            Some((data.shell_surface_type, parent_status))
        }) else {
            return;
        };

        // Custom surfaces don't get toplevels and are always considered 'shown'; nothing to do here.
        if ty == ShellSurfaceType::Custom {
            return;
        }

        /* If this is a child window, the parent *must* be in the final shown state,
         * meaning that it has received a configure event, followed by a frame callback.
         * If not, a race condition can result, with effects ranging from the child
         * window to spuriously closing to protocol errors.
         *
         * If waiting on the parent window, set the pending status and the window will
         * be shown when the parent is in the shown state.
         */
        if win.parent.is_some() && parent_status != Some(ShellSurfaceStatus::Shown) {
            self.with_data(|d| {
                if let Some(data) = d.window_mut(window) {
                    data.shell_surface_status = ShellSurfaceStatus::ShowPending;
                }
            });
            return;
        }

        // Always roundtrip to ensure there are no pending buffer attachments.
        // FIXME (upstream): this loops forever if the connection fails while
        // a show/hide sync point is pending (kept as upstream).
        loop {
            let _ = self.conn.roundtrip();
            if !self.with_data(|d| d.window(window).is_some_and(|w| w.show_hide_sync_required)) {
                break;
            }
        }

        let libdecor_ctx = self.with_data(|d| {
            let data = d.window_mut(window)?;
            data.shell_surface_status = ShellSurfaceStatus::WaitingForConfigure;

            /* Detach any previous buffers before resetting everything, otherwise when
             * calling this a second time you'll get an annoying protocol error!
             *
             * FIXME: This was originally moved to HideWindow, which _should_ make
             * sense, but for whatever reason UE5's popups require that this actually
             * be in both places at once? Possibly from renderers making commits? I can't
             * fully remember if this location caused crashes or if I was fixing a pair
             * of Hide/Show calls. In any case, UE gives us a pretty good test and having
             * both detach calls passes. This bug may be relevant if I'm wrong:
             *
             * https://bugs.kde.org/show_bug.cgi?id=448856
             *
             * -flibit
             */
            data.surface().attach(None, 0, 0);
            data.surface().commit();
            let surface_raw = data.surface().raw();
            let app_id = data.app_id.clone();
            Some((d.libdecor.as_ref().map(|c| c.raw), surface_raw, app_id))
        });
        let Some((libdecor_ctx, surface_raw, app_id)) = libdecor_ctx else {
            return;
        };

        // Create the shell surface and map the toplevel/popup
        let mut focus_popup = false;
        if ty == ShellSurfaceType::Libdecor {
            let frame = match (libdecor_ctx, libdecor_syms(self.syms())) {
                (Some(ctx), Some(l)) => {
                    let user_data = Box::new(FrameUserData {
                        video: self.weak(),
                        window,
                    });
                    // SAFETY: the context and surface are alive; the
                    // interface is static; the user data outlives the frame
                    // (it is kept with it).
                    let raw = unsafe {
                        (l.libdecor_decorate)(
                            ctx.as_ptr(),
                            surface_raw,
                            &LIBDECOR_FRAME_INTERFACE,
                            &*user_data as *const FrameUserData as *mut c_void,
                        )
                    };
                    resume_pending_panic();
                    NonNull::new(raw).map(|raw| LibdecorFrame {
                        raw,
                        syms: self.syms().clone(),
                        _user_data: user_data,
                    })
                }
                _ => None,
            };
            match frame {
                None => {
                    crate::error!(
                        crate::log::Category::Video,
                        "Failed to create libdecor frame!"
                    );
                }
                Some(frame) => {
                    let raw = frame.raw();
                    self.with_data(|d| {
                        if let Some(data) = d.window_mut(window) {
                            data.shell_surface.libdecor_frame = Some(frame);
                        }
                    });
                    if let Some(l) = libdecor_syms(self.syms()) {
                        let app_id = super::video::c_string(&app_id);
                        // SAFETY: the frame is alive; nothing is borrowed, so
                        // the frame callbacks may run.
                        unsafe {
                            (l.libdecor_frame_set_app_id)(raw, app_id.as_ptr());
                            (l.libdecor_frame_map)(raw);
                            if win.flags.contains(WindowFlags::BORDERLESS) {
                                // Note: Calling this with 'true' immediately after mapping will cause the libdecor Cairo plugin to crash.
                                (l.libdecor_frame_set_visibility)(raw, false);
                            }
                        }
                        resume_pending_panic();

                        self.with_data(|d| {
                            let toplevel_raw =
                                self.toplevel_for_window(d.window(window)).map(|t| t.raw());
                            let VideoData { g, windows, .. } = d;
                            let Some(data) = find_mut(windows, window) else {
                                return;
                            };
                            if let Some(exporter) = &g.zxdg_exporter_v2 {
                                let mut exported = exporter.export_toplevel(data.surface.obj());
                                self.listen_exported(&mut exported, window);
                                data.exported = Some(exported);
                            }

                            if let (Some(manager), Some(icon), Some(toplevel_raw)) = (
                                &g.xdg_toplevel_icon_manager_v1,
                                &data.xdg_toplevel_icon_v1,
                                toplevel_raw,
                            ) {
                                // SAFETY: libdecor's toplevel lives with the frame.
                                if let Some(toplevel) =
                                    unsafe { Obj::from_raw(toplevel_raw.cast(), &self.conn) }
                                {
                                    manager.set_icon(toplevel, Some(icon.obj()));
                                }
                            }

                            // SAFETY: the frame is alive; getters.
                            let (xdg_surface, xdg_toplevel) = unsafe {
                                (
                                    (l.libdecor_frame_get_xdg_surface)(raw),
                                    (l.libdecor_frame_get_xdg_toplevel)(raw),
                                )
                            };
                            set_ptr_prop(
                                &props,
                                PROP_WINDOW_WAYLAND_XDG_SURFACE_POINTER,
                                xdg_surface.cast(),
                            );
                            set_ptr_prop(
                                &props,
                                PROP_WINDOW_WAYLAND_XDG_TOPLEVEL_POINTER,
                                xdg_toplevel.cast(),
                            );
                        });
                    }
                }
            }
        } else if ty == ShellSurfaceType::XdgToplevel || ty == ShellSurfaceType::XdgPopup {
            let parent_win = win.parent.and_then(snapshot);
            let syms = self.syms().clone();
            focus_popup = self.with_data(|d| {
                // (the parent's xdg_surface, from libdecor or ours)
                let parent_xdg_surface_raw = parent_win.as_ref().and_then(|p| {
                    let parent_data = d.window(p.id)?;
                    if parent_data.shell_surface_type == ShellSurfaceType::Libdecor {
                        let frame = parent_data.shell_surface.libdecor_frame.as_ref()?;
                        let l = libdecor_syms(&syms)?;
                        // SAFETY: the frame is alive; a getter.
                        Some(unsafe { (l.libdecor_frame_get_xdg_surface)(frame.raw()) })
                    } else if parent_data.shell_surface_type == ShellSurfaceType::XdgToplevel
                        || parent_data.shell_surface_type == ShellSurfaceType::XdgPopup
                    {
                        parent_data
                            .shell_surface
                            .xdg_surface
                            .as_ref()
                            .map(|s| s.raw())
                    } else {
                        None
                    }
                });
                // The popup parameters that depend on the parent.
                let parent_params = parent_win.as_ref().and_then(|p| {
                    let parent_data = d.window(p.id)?;
                    Some((
                        window_geometry(parent_data),
                        window_scale(p.flags, parent_data),
                        p.w,
                        p.h,
                    ))
                });

                let mut position = None;
                if ty == ShellSurfaceType::XdgPopup {
                    if let Some((_, parent_scale, parent_w, parent_h)) = parent_params {
                        // Set the popup initial position
                        let mut position_x = if win.last_position_pending {
                            win.pending.x
                        } else {
                            win.x
                        };
                        let mut position_y = if win.last_position_pending {
                            win.pending.y
                        } else {
                            win.y
                        };
                        ensure_popup_position_is_valid(
                            win.w,
                            win.h,
                            parent_w,
                            parent_h,
                            &mut position_x,
                            &mut position_y,
                        );
                        if d.window(window).is_some_and(|w| w.scale_to_display) {
                            position_x = pixel_to_point(parent_scale, position_x);
                            position_y = pixel_to_point(parent_scale, position_y);
                        }
                        if let Some(parent_data) = parent_win.as_ref().and_then(|p| d.window(p.id))
                        {
                            adjust_popup_offset(
                                &syms,
                                parent_data,
                                &mut position_x,
                                &mut position_y,
                            );
                        }
                        position = Some((position_x, position_y));
                    }
                }

                let VideoData { g, windows, .. } = d;
                let Some(data) = find_mut(windows, window) else {
                    return false;
                };
                let Some(xdg_wm_base) = &g.xdg_wm_base else {
                    return false;
                };
                let mut xdg_surface = xdg_wm_base.get_xdg_surface(data.surface.obj());
                self.listen(
                    &mut xdg_surface,
                    move |v, obj, XdgSurfaceEvent::Configure { serial }| {
                        v.handle_xdg_surface_configure(window, obj, serial);
                    },
                );
                set_ptr_prop(
                    &props,
                    PROP_WINDOW_WAYLAND_XDG_SURFACE_POINTER,
                    xdg_surface.raw().cast(),
                );

                let mut focus_popup = false;
                if ty == ShellSurfaceType::XdgPopup {
                    // Set up the positioner for the popup and configure the constraints
                    let positioner = xdg_wm_base.create_positioner();
                    positioner.set_anchor(XdgPositionerAnchor::TOP_LEFT);
                    let parent_geometry = parent_params.map(|p| p.0).unwrap_or_default();
                    positioner.set_anchor_rect(0, 0, parent_geometry.w, parent_geometry.h);

                    let constraint = if win.constrain_popup {
                        XdgPositionerConstraintAdjustment::SLIDE_X
                            | XdgPositionerConstraintAdjustment::SLIDE_Y
                    } else {
                        XdgPositionerConstraintAdjustment::NONE
                    };
                    positioner.set_constraint_adjustment(constraint);
                    positioner.set_gravity(XdgPositionerGravity::BOTTOM_RIGHT);
                    positioner.set_size(data.current.logical_width, data.current.logical_height);

                    let (position_x, position_y) = position.unwrap_or((0, 0));
                    positioner.set_offset(position_x, position_y);

                    // Assign the popup role
                    // SAFETY: the parent's xdg_surface is alive while the data is borrowed.
                    let parent_xdg_surface = parent_xdg_surface_raw.and_then(|raw| unsafe {
                        Obj::<XdgSurface>::from_raw(raw.cast(), &self.conn)
                    });
                    let mut popup = xdg_surface.get_popup(parent_xdg_surface, positioner.obj());
                    self.listen(&mut popup, move |v, _, ev| {
                        v.handle_xdg_popup_event(window, ev)
                    });

                    if win.flags.contains(WindowFlags::TOOLTIP) {
                        // Tooltips can't be interacted with, so turn off the input region to avoid blocking anything behind them
                        if let Some(compositor) = &g.compositor {
                            let region = compositor.create_region();
                            region.add(0, 0, 0, 0);
                            data.surface().set_input_region(Some(region.obj()));
                        }
                    } else if win.flags.contains(WindowFlags::POPUP_MENU)
                        && !win.flags.contains(WindowFlags::NOT_FOCUSABLE)
                    {
                        focus_popup = true;
                    }

                    set_ptr_prop(
                        &props,
                        PROP_WINDOW_WAYLAND_XDG_POPUP_POINTER,
                        popup.raw().cast(),
                    );
                    set_ptr_prop(
                        &props,
                        PROP_WINDOW_WAYLAND_XDG_POSITIONER_POINTER,
                        positioner.raw().cast(),
                    );
                    data.shell_surface.xdg_popup = Some(popup);
                    data.shell_surface.xdg_positioner = Some(positioner);
                } else {
                    let mut toplevel = xdg_surface.get_toplevel();
                    toplevel.set_app_id(&data.app_id);
                    self.listen(&mut toplevel, move |v, _, ev| {
                        v.handle_xdg_toplevel_event(window, ev)
                    });

                    // Create the window decorations
                    if let Some(decoration_manager) = &g.decoration_manager {
                        let mut decoration =
                            decoration_manager.get_toplevel_decoration(toplevel.obj());
                        self.listen(
                            &mut decoration,
                            move |v, _, ZxdgToplevelDecorationV1Event::Configure { mode }| {
                                v.handle_xdg_toplevel_decoration_configure(window, mode);
                            },
                        );
                        let mode = if !win.flags.contains(WindowFlags::BORDERLESS) {
                            ZxdgToplevelDecorationV1Mode::SERVER_SIDE
                        } else {
                            ZxdgToplevelDecorationV1Mode::CLIENT_SIDE
                        };
                        decoration.set_mode(mode);
                        data.server_decoration = Some(decoration);
                    }

                    if let Some(exporter) = &g.zxdg_exporter_v2 {
                        let mut exported = exporter.export_toplevel(data.surface.obj());
                        self.listen_exported(&mut exported, window);
                        data.exported = Some(exported);
                    }

                    if let (Some(manager), Some(icon)) =
                        (&g.xdg_toplevel_icon_manager_v1, &data.xdg_toplevel_icon_v1)
                    {
                        manager.set_icon(toplevel.obj(), Some(icon.obj()));
                    }

                    set_ptr_prop(
                        &props,
                        PROP_WINDOW_WAYLAND_XDG_TOPLEVEL_POINTER,
                        toplevel.raw().cast(),
                    );
                    data.shell_surface.xdg_toplevel = Some(toplevel);
                }
                data.shell_surface.xdg_surface = Some(xdg_surface);
                focus_popup
            });
        }

        if focus_popup {
            self.set_keyboard_focus(window, true);
        }

        // Restore state that was set prior to this call
        self.register_toplevel_for_session(window);
        let _ = self.wayland_set_window_parent(window, win.parent);

        if win.flags.contains(WindowFlags::MODAL) {
            let _ = self.wayland_set_window_modal(window, true);
        }

        self.wayland_set_window_title(window);

        /* We have to wait until the surface gets a "configure" event, or use of
         * this surface will fail. This is a new rule for xdg_shell.
         */
        let waiting = |v: &WaylandVideo| {
            v.with_data(|d| {
                d.window(window).is_some_and(|w| {
                    w.shell_surface_status == ShellSurfaceStatus::WaitingForConfigure
                })
            })
        };
        if ty == ShellSurfaceType::Libdecor {
            if self.with_data(|d| {
                d.window(window)
                    .is_some_and(|w| w.shell_surface.libdecor_frame.is_some())
            }) {
                while waiting(self) {
                    if self.libdecor_dispatch(-1) < 0 && !self.wayland_handle_display_disconnected()
                    {
                        return;
                    }
                    if self.conn.dispatch_pending().is_err()
                        && !self.wayland_handle_display_disconnected()
                    {
                        return;
                    }
                }
            }
        } else if ty == ShellSurfaceType::XdgPopup || ty == ShellSurfaceType::XdgToplevel {
            /* Unlike libdecor we need to call this explicitly to prevent a deadlock.
             * libdecor will call this as part of their configure event!
             * -flibit
             */
            let has_xdg_surface = self.with_data(|d| {
                d.window(window).is_some_and(|w| {
                    w.surface().commit();
                    w.shell_surface.xdg_surface.is_some()
                })
            });
            if has_xdg_surface {
                while waiting(self) {
                    if self.conn.dispatch().is_err() && !self.wayland_handle_display_disconnected()
                    {
                        return;
                    }
                }
            }
        } else {
            // Nothing to see here, just commit.
            self.with_data(|d| {
                if let Some(w) = d.window(window) {
                    w.surface().commit();
                }
            });
        }

        // Make sure the window can't be resized to 0, or it can be spuriously closed by the window manager.
        self.with_data(|d| {
            if let Some(data) = d.window_mut(window) {
                data.system_limits.min_width = data.system_limits.min_width.max(1);
                data.system_limits.min_height = data.system_limits.min_height.max(1);
            }
        });

        /* Unlike the rest of window state we have to set this _after_ flushing the
         * display, because we need to create the decorations before possibly hiding
         * them immediately afterward.
         */
        if ty == ShellSurfaceType::Libdecor {
            let windowed = with_window(window, |w| w.core.windowed).unwrap_or_default();
            let too_small = self.with_data(|d| {
                let data = d.window_mut(window)?;
                // Libdecor plugins can enforce minimum window sizes, so adjust if the initial window size is too small.
                if windowed.w < data.system_limits.min_width
                    || windowed.h < data.system_limits.min_height
                {
                    let limits = data.system_limits;
                    data.current.logical_width = windowed.w.max(data.system_limits.min_width);
                    data.current.logical_height = windowed.h.max(data.system_limits.min_height);
                    Some(limits)
                } else {
                    None
                }
            });
            if let Some(limits) = too_small {
                // Warn if the window frame will be larger than the content surface.
                crate::warn!(
                    crate::log::Category::Video,
                    "Window dimensions ({}, {}) are smaller than the system enforced minimum ({}, {}); window borders will be larger than the content surface.",
                    windowed.w,
                    windowed.h,
                    limits.min_width,
                    limits.min_height
                );
                self.commit_libdecor_frame(window);
            }
        }
        self.wayland_set_window_resizable(
            window,
            window_flags(window).contains(WindowFlags::RESIZABLE),
        );

        // We're finally done putting the window together, raise if possible
        let has_activation_manager = self.with_data(|d| d.g.activation_manager.is_some());
        if has_activation_manager && hints::get_bool(hints::WINDOW_ACTIVATE_WHEN_SHOWN, true) {
            /* if the process was passed an activation token, use it when showing
             * the initial window.
             *
             * Note that we don't check for empty strings, as that is still
             * considered a valid activation token!
             */
            if let Some(activation_token) = crate::stdlib::getenv("XDG_ACTIVATION_TOKEN") {
                self.with_data(|d| {
                    if let (Some(manager), Some(data)) = (&d.g.activation_manager, d.window(window))
                    {
                        manager.activate(&activation_token, data.surface());
                    }
                });

                // Clear this variable, per the protocol's request
                let _ = crate::stdlib::unsetenv_unsafe("XDG_ACTIVATION_TOKEN");
            } else {
                // Try to generate an activation token for this window.
                self.wayland_raise_window(window);
            }
        } else {
            // Clear the ignored activation token, as it won't be used.
            let _ = crate::stdlib::unsetenv_unsafe("XDG_ACTIVATION_TOKEN");
        }

        // No frame callback on an external surface, as it may already have one attached.
        if !window_flags(window).contains(WindowFlags::EXTERNAL) {
            // Fire a callback when the compositor wants a new frame.
            self.with_data(|d| {
                if let Some(data) = d.window(window) {
                    let cb = self.new_surface_frame_callback(data);
                    if let Some(data) = d.window_mut(window) {
                        data.surface_frame_callback = Some(cb);
                    }
                }
            });
        }

        self.with_data(|d| {
            if let Some(data) = d.window_mut(window) {
                data.show_hide_sync_required = true;
            }
        });
        self.add_show_hide_sync(window);

        self.set_showing_window(window, true);
        send_window_event(window, EventType::WINDOW_SHOWN, 0, 0);
        self.set_showing_window(window, false);

        // Send an exposure event to signal that the client should draw.
        send_window_event(window, EventType::WINDOW_EXPOSED, 0, 0);
    }

    fn set_showing_window(&self, window: WindowID, showing: bool) {
        self.with_data(|d| {
            if let Some(data) = d.window_mut(window) {
                data.showing_window = showing;
            }
        });
    }

    /// The xdg-foreign export listener (`exported_v2_listener`).
    fn listen_exported(&self, exported: &mut Proxy<ZxdgExportedV2>, window: WindowID) {
        self.listen(
            exported,
            move |_, _, ZxdgExportedV2Event::Handle { handle }| {
                // Translation of `exported_handle_handler()`.
                let handle = handle.to_string_lossy().into_owned();
                let _ = with_window(window, |w| {
                    let _ = w.properties().set(
                        PROP_WINDOW_WAYLAND_XDG_TOPLEVEL_EXPORT_HANDLE_STRING,
                        handle,
                    );
                });
            },
        );
    }

    /// Translation of `Wayland_ReleasePopup()`.
    fn release_popup(&self, popup: WindowID) {
        // Basic sanity checks to weed out the weird popup closures
        let Some(flags) = with_window(popup, |w| w.flags()).ok() else {
            return;
        };

        // This may already be freed by a parent popup!
        if !self.with_data(|d| {
            d.window(popup)
                .is_some_and(|w| w.shell_surface.xdg_popup.is_some())
        }) {
            return;
        }

        if flags.contains(WindowFlags::POPUP_MENU) && !flags.contains(WindowFlags::NOT_FOCUSABLE) {
            if let Ok((Some(new_focus), set_focus)) = should_relinquish_popup_focus(popup) {
                self.set_keyboard_focus(new_focus, set_focus);
            }
        }

        let objects = self.with_data(|d| {
            let popupdata = d.window_mut(popup)?;
            Some((
                popupdata.shell_surface.xdg_popup.take(),
                popupdata.shell_surface.xdg_positioner.take(),
            ))
        });
        // (xdg_popup_destroy(), then xdg_positioner_destroy())
        drop(objects);

        if let Ok(props) = with_window(popup, |w| w.properties()) {
            let _ = props.remove(PROP_WINDOW_WAYLAND_XDG_POPUP_POINTER);
            let _ = props.remove(PROP_WINDOW_WAYLAND_XDG_POSITIONER_POINTER);
        }
    }

    /// Translation of `Wayland_HideWindow()`.
    pub(crate) fn wayland_hide_window(&self, window: WindowID) {
        let Ok(props) = with_window(window, |w| w.properties()) else {
            return;
        };

        let Some((ty, sync_required)) = self.with_data(|d| {
            d.window(window)
                .map(|w| (w.shell_surface_type, w.show_hide_sync_required))
        }) else {
            return;
        };

        // Custom surfaces have nothing to destroy and are always considered to be 'shown'; nothing to do here.
        if ty == ShellSurfaceType::Custom {
            return;
        }

        /* The window was shown, but the sync point hasn't yet been reached.
         * Pump events to avoid a possible protocol violation.
         */
        if sync_required {
            let _ = self.conn.roundtrip();
        }

        let objects = self.with_data(|d| {
            let wind = d.window_mut(window)?;
            wind.shell_surface_status = ShellSurfaceStatus::Hidden;
            Some((
                wind.surface_frame_callback.take(),
                wind.server_decoration.take(),
                wind.exported.take(),
                wind.xdg_dialog_v1.take(),
            ))
        });
        let Some((frame_callback, server_decoration, exported, dialog)) = objects else {
            return;
        };
        drop(frame_callback);
        drop(server_decoration);

        // Clean up the export handle.
        if exported.is_some() {
            drop(exported);
            let _ = props.remove(PROP_WINDOW_WAYLAND_XDG_TOPLEVEL_EXPORT_HANDLE_STRING);
        }

        drop(dialog);

        if ty == ShellSurfaceType::Libdecor {
            let frame =
                self.with_data(|d| d.window_mut(window)?.shell_surface.libdecor_frame.take());
            if let Some(frame) = frame {
                // (libdecor_frame_unref())
                drop(frame);

                let _ = props.remove(PROP_WINDOW_WAYLAND_XDG_SURFACE_POINTER);
                let _ = props.remove(PROP_WINDOW_WAYLAND_XDG_TOPLEVEL_POINTER);
            }
        } else {
            if ty == ShellSurfaceType::XdgPopup {
                self.release_popup(window);
            } else {
                let toplevel =
                    self.with_data(|d| d.window_mut(window)?.shell_surface.xdg_toplevel.take());
                if toplevel.is_some() {
                    drop(toplevel);
                    let _ = props.remove(PROP_WINDOW_WAYLAND_XDG_TOPLEVEL_POINTER);
                }
            }

            let xdg_surface =
                self.with_data(|d| d.window_mut(window)?.shell_surface.xdg_surface.take());
            if xdg_surface.is_some() {
                drop(xdg_surface);
                let _ = props.remove(PROP_WINDOW_WAYLAND_XDG_SURFACE_POINTER);
            }
        }

        // Attach a null buffer to unmap the surface.
        self.with_data(|d| {
            if let Some(wind) = d.window(window) {
                if let Some(mask_surface) = &wind.mask.surface {
                    mask_surface.attach(None, 0, 0);
                    mask_surface.commit();
                }
                wind.surface().attach(None, 0, 0);
                wind.surface().commit();
            }
        });

        // Need to destroy the session object after unmapping the window, or the state may not be saved.
        self.destroy_toplevel_session(window);

        let shell_surface = self.with_data(|d| {
            let wind = d.window_mut(window)?;
            wind.explicit_geometry = Rect::default();
            wind.show_hide_sync_required = true;
            Some(std::mem::take(&mut wind.shell_surface))
        });
        drop(shell_surface);
        self.add_show_hide_sync(window);
    }

    // -----------------------------------------------------------------------
    // Activation
    // -----------------------------------------------------------------------

    /* The xdg-activation protocol considers "activation" to be one of two things:
     *
     * 1: Raising a window to the top and flashing the titlebar
     * 2: Flashing the titlebar while keeping the window where it is
     *
     * As you might expect from Wayland, the general policy is to go with #2 unless
     * the client can prove to the compositor beyond a reasonable doubt that raising
     * the window will not be malicious behavior.
     *
     * For SDL this means RaiseWindow and FlashWindow both use the same protocol,
     * but in different ways: RaiseWindow will provide as _much_ information as
     * possible while FlashWindow will provide as _little_ information as possible,
     * to nudge the compositor into doing what we want.
     *
     * This isn't _strictly_ what the protocol says will happen, but this is what
     * current implementations are doing (as of writing, YMMV in the far distant
     * future).
     *
     * -flibit
     */
    /// Translation of `Wayland_activate_window()`.
    fn activate_window(&self, target_wind: WindowID, set_serial: bool) {
        let old_token = self.with_data(|data| {
            let seat = data.last_implicit_grab_seat.and_then(|s| data.seat(s));
            let focus = seat.and_then(|s| s.keyboard.focus.or(s.pointer.focus));
            let requesting_surface = focus
                .and_then(|f| data.window(f))
                .map(|w| w.surface().raw());

            let VideoData {
                g,
                windows,
                seat_list,
                last_implicit_grab_seat,
                ..
            } = data;
            let manager = g.activation_manager.as_ref()?;
            // (we're about to overwrite this with a new request)
            let old = find_mut(windows, target_wind)?.activation_token.take();

            let mut token = manager.get_activation_token();
            self.listen(
                &mut token,
                move |v, obj, XdgActivationTokenV1Event::Done { token }| {
                    v.handle_xdg_activation_done(
                        target_wind,
                        obj.raw() as usize,
                        &token.to_string_lossy(),
                    );
                },
            );

            /* Note that we are not setting the app_id here.
             *
             * Hypothetically we could set the app_id from data->classname, but
             * that part of the API is for _external_ programs, not ourselves.
             *
             * -flibit
             */
            if let Some(raw) = requesting_surface {
                // This specifies the surface from which the activation request is originating, not the activation target surface.
                // SAFETY: the surface is alive (the data is borrowed).
                if let Some(surface) = unsafe { Obj::<WlSurface>::from_raw(raw.cast(), &self.conn) }
                {
                    token.set_surface(surface);
                }
            }
            if set_serial {
                if let Some(seat) = last_implicit_grab_seat
                    .and_then(|s| seat_list.iter().find(|x| x.registry_id == s))
                {
                    token.set_serial(seat.last_implicit_grab_serial, seat.wl_seat.obj());
                }
            }
            token.commit();
            if let Some(wind) = find_mut(windows, target_wind) {
                wind.activation_token = Some(token);
            }
            Some(old)
        });
        drop(old_token);
    }

    /// Translation of `handle_xdg_activation_done()`.
    fn handle_xdg_activation_done(&self, window: WindowID, token_proxy: usize, token: &str) {
        let done = self.with_data(|d| {
            let VideoData { g, windows, .. } = d;
            let wind = find_mut(windows, window)?;
            if wind.activation_token.as_ref().map(|t| t.raw() as usize) == Some(token_proxy) {
                if let Some(manager) = &g.activation_manager {
                    manager.activate(token, wind.surface.obj());
                }
                wind.activation_token.take()
            } else {
                None
            }
        });
        // (xdg_activation_token_v1_destroy())
        drop(done);
    }

    /// Translation of `Wayland_RaiseWindow()`.
    pub(crate) fn wayland_raise_window(&self, window: WindowID) {
        if self.with_data(|d| d.g.activation_manager.is_some()) {
            /* Check for an activation token, in case the window is being
             * raised in response to a system notification.
             *
             * Note that we don't check for empty strings, as that is still
             * considered a valid activation token!
             */
            if let Some(activation_token) = crate::notification::notification_activation_token() {
                self.with_data(|d| {
                    if let (Some(manager), Some(wind)) = (&d.g.activation_manager, d.window(window))
                    {
                        manager.activate(&activation_token, wind.surface());
                    }
                });
                return;
            }
        }

        // No token? Try to activate the window via an event serial.
        self.activate_window(window, true);
    }

    /// Translation of `Wayland_FlashWindow()`.
    pub(crate) fn wayland_flash_window(
        &self,
        window: WindowID,
        _operation: FlashOperation,
    ) -> Result<()> {
        /* Not setting the serial will specify 'urgency' without switching focus as per
         * https://gitlab.freedesktop.org/wayland/wayland-protocols/-/merge_requests/9#note_854977
         */
        self.activate_window(window, false);
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Window state
    // -----------------------------------------------------------------------

    /// Translation of `Wayland_SetWindowFullscreen()`.
    pub(crate) fn wayland_set_window_fullscreen(
        &self,
        window: WindowID,
        display: DisplayID,
        fullscreen: FullscreenOp,
    ) -> FullscreenResult {
        let Some((ty, showing_window, sync_required)) = self.with_data(|d| {
            d.window(window).map(|w| {
                (
                    w.shell_surface_type,
                    w.showing_window,
                    w.show_hide_sync_required,
                )
            })
        }) else {
            return FullscreenResult::Failed;
        };

        // Custom surfaces have no toplevel to make fullscreen.
        if ty == ShellSurfaceType::Custom {
            return FullscreenResult::Failed;
        }

        // Drop fullscreen leave requests when showing the window.
        if showing_window && fullscreen == FullscreenOp::Leave {
            return FullscreenResult::Succeeded;
        }

        if sync_required {
            let _ = self.conn.roundtrip();
        }

        // Flushing old events pending a new one, ignore this request.
        let dropping = self.with_data(|d| {
            let wind = d.window_mut(window)?;
            if wind.drop_fullscreen_requests {
                return Some(true);
            }
            wind.drop_fullscreen_requests = true;
            Some(false)
        });
        if dropping != Some(false) {
            return FullscreenResult::Succeeded;
        }

        self.flush_pending_events(window);
        let Some((is_fullscreen, num_outputs, last_display_id, pending_state_commit, output)) =
            self.with_data(|d| {
                let output = d.output_for_display(display).map(|o| o.registry_id);
                let wind = d.window_mut(window)?;
                wind.drop_fullscreen_requests = false;
                Some((
                    wind.is_fullscreen,
                    wind.outputs.len(),
                    wind.last_display_id,
                    wind.pending_state_commit,
                    output,
                ))
            })
        else {
            return FullscreenResult::Failed;
        };
        let mut output = output;

        // Nothing to do if the window is not fullscreen, and this isn't an explicit enter request.
        if !is_fullscreen {
            if fullscreen == FullscreenOp::Update {
                // Request was out of date; signal the video core not to update any state.
                return FullscreenResult::Pending;
            } else if fullscreen == FullscreenOp::Leave {
                // Already not fullscreen; nothing to do.
                return FullscreenResult::Succeeded;
            }
        }

        let enter = fullscreen != FullscreenOp::Leave;
        let Some(win) = snapshot(window) else {
            return FullscreenResult::Failed;
        };

        // Don't send redundant fullscreen set/unset events.
        if enter != is_fullscreen {
            self.with_data(|d| {
                if let Some(wind) = d.window_mut(window) {
                    wind.fullscreen_was_positioned = enter;
                }
            });

            /* Only use the specified output if an exclusive mode is being used, or a position was explicitly requested
             * before entering fullscreen desktop. Otherwise, let the compositor handle placement, as it has more
             * information about where the window is and where it should go, particularly if fullscreen is being requested
             * before the window is mapped, or the window spans multiple outputs.
             */
            if !win.fullscreen_exclusive
                && (win.undefined_x
                    || win.undefined_y
                    || (num_outputs != 0 && !win.last_position_pending))
            {
                output = None;
            }

            // Commit to set any pending size or limit data.
            if enter && pending_state_commit {
                self.commit_surface(window);
            }
            self.set_fullscreen(window, output, enter);
        } else if is_fullscreen {
            /*
             * If the window is already fullscreen, this is likely a request to switch between
             * fullscreen and fullscreen desktop, change outputs, or change the video mode.
             *
             * If the window is already positioned on the target output, just update the
             * window geometry.
             */
            if last_display_id != display {
                self.with_data(|d| {
                    if let Some(wind) = d.window_mut(window) {
                        wind.fullscreen_was_positioned = true;
                    }
                });
                self.set_fullscreen(window, output, true);
            } else {
                self.configure_window_geometry(window);
                self.commit_libdecor_frame(window);

                return FullscreenResult::Succeeded;
            }
        }

        FullscreenResult::Pending
    }

    /// `wl_surface_commit()` on the surface of a window.
    fn commit_surface(&self, window: WindowID) {
        self.with_data(|d| {
            if let Some(wind) = d.window(window) {
                wind.surface().commit();
            }
        });
    }

    /// Translation of `Wayland_RestoreWindow()`.
    pub(crate) fn wayland_restore_window(&self, window: WindowID) {
        let flags = window_flags(window);
        let Some((ty, showing_window, pending)) = self.with_data(|d| {
            d.window(window).map(|w| {
                (
                    w.shell_surface_type,
                    w.showing_window,
                    w.pending_state_deadline_count,
                )
            })
        }) else {
            return;
        };

        // Drop restore requests when showing the window.
        if showing_window {
            return;
        }

        // Not currently fullscreen or maximized, and no state pending; nothing to do.
        if !flags.intersects(WindowFlags::FULLSCREEN | WindowFlags::MAXIMIZED) && pending == 0 {
            return;
        }

        if ty == ShellSurfaceType::Libdecor {
            // (Can't do anything yet without a frame, wait for ShowWindow)
            // SAFETY: the frame is alive.
            if self
                .with_libdecor_frame(window, |l, frame| unsafe {
                    (l.libdecor_frame_unset_maximized)(frame)
                })
                .is_some()
            {
                self.with_data(|d| self.add_pending_state_sync(d, window));
            }
        } else if ty == ShellSurfaceType::XdgToplevel {
            // Note that xdg-shell does NOT provide a way to unset minimize!
            self.with_data(|d| {
                let Some(toplevel) = d
                    .window(window)
                    .and_then(|w| w.shell_surface.xdg_toplevel.as_ref())
                else {
                    return; // Can't do anything yet, wait for ShowWindow
                };
                toplevel.unset_maximized();
                self.add_pending_state_sync(d, window);
            });
        }
    }

    /// Translation of `Wayland_SetWindowBordered()`.
    pub(crate) fn wayland_set_window_bordered(&self, window: WindowID, bordered: bool) {
        let Some(ty) = self.with_data(|d| d.window(window).map(|w| w.shell_surface_type)) else {
            return;
        };

        if ty == ShellSurfaceType::Libdecor {
            // SAFETY: the frame is alive; nothing is borrowed.
            self.with_libdecor_frame(window, |l, frame| unsafe {
                (l.libdecor_frame_set_visibility)(frame, bordered)
            });
        } else if ty == ShellSurfaceType::XdgToplevel {
            self.with_data(|d| {
                if d.g.decoration_manager.is_some() {
                    if let Some(decoration) =
                        d.window(window).and_then(|w| w.server_decoration.as_ref())
                    {
                        let mode = if bordered {
                            ZxdgToplevelDecorationV1Mode::SERVER_SIDE
                        } else {
                            ZxdgToplevelDecorationV1Mode::CLIENT_SIDE
                        };
                        decoration.set_mode(mode);
                    }
                }
            });
        }
    }

    /// Translation of `Wayland_SetWindowResizable()`.
    pub(crate) fn wayland_set_window_resizable(&self, window: WindowID, _resizable: bool) {
        /* When changing the resize capability on libdecor windows, the limits must always
         * be reapplied, as when libdecor changes states, it overwrites the values internally.
         */
        self.set_min_max_dimensions(window);
        self.commit_libdecor_frame(window);

        self.with_data(|d| {
            if let Some(wind) = d.window_mut(window) {
                if wind.shell_surface_status == ShellSurfaceStatus::Shown {
                    wind.pending_state_commit = true;
                }
            }
        });
    }

    /// Translation of `Wayland_MaximizeWindow()`.
    pub(crate) fn wayland_maximize_window(&self, window: WindowID) {
        if self.with_data(|d| d.window(window).is_some_and(|w| w.show_hide_sync_required)) {
            let _ = self.conn.roundtrip();
        }

        let flags = window_flags(window);
        let Some((ty, pending, pending_state_commit)) = self.with_data(|d| {
            d.window(window).map(|w| {
                (
                    w.shell_surface_type,
                    w.pending_state_deadline_count,
                    w.pending_state_commit,
                )
            })
        }) else {
            return;
        };

        // Not fullscreen, already maximized, and no state pending; nothing to do.
        if !flags.contains(WindowFlags::FULLSCREEN)
            && flags.contains(WindowFlags::MAXIMIZED)
            && pending == 0
        {
            return;
        }

        if ty == ShellSurfaceType::Libdecor {
            if !self.with_data(|d| {
                d.window(window)
                    .is_some_and(|w| w.shell_surface.libdecor_frame.is_some())
            }) {
                return; // Can't do anything yet, wait for ShowWindow
            }

            // Commit to set any pending size or limit data.
            if pending_state_commit {
                self.commit_surface(window);
            }
            // SAFETY: the frame is alive.
            self.with_libdecor_frame(window, |l, frame| unsafe {
                (l.libdecor_frame_set_maximized)(frame)
            });
            self.with_data(|d| self.add_pending_state_sync(d, window));
        } else if ty == ShellSurfaceType::XdgToplevel {
            self.with_data(|d| {
                let Some(wind) = d.window(window) else {
                    return;
                };
                let Some(toplevel) = &wind.shell_surface.xdg_toplevel else {
                    return; // Can't do anything yet, wait for ShowWindow
                };

                // Commit to set any pending size or limit data.
                if pending_state_commit {
                    wind.surface().commit();
                }
                toplevel.set_maximized();
                self.add_pending_state_sync(d, window);
            });
        }
    }

    /// Translation of `Wayland_MinimizeWindow()`.
    pub(crate) fn wayland_minimize_window(&self, window: WindowID) {
        let Some((ty, wm_caps)) =
            self.with_data(|d| d.window(window).map(|w| (w.shell_surface_type, w.wm_caps)))
        else {
            return;
        };

        if wm_caps & WAYLAND_WM_CAPS_MINIMIZE == 0 {
            return;
        }

        if ty == ShellSurfaceType::Libdecor {
            // SAFETY: the frame is alive.
            if self
                .with_libdecor_frame(window, |l, frame| unsafe {
                    (l.libdecor_frame_set_minimized)(frame)
                })
                .is_some()
            {
                send_window_event(window, EventType::WINDOW_MINIMIZED, 0, 0);
            }
        } else if ty == ShellSurfaceType::XdgToplevel {
            let done = self.with_data(|d| {
                let toplevel = d.window(window)?.shell_surface.xdg_toplevel.as_ref()?;
                toplevel.set_minimized();
                Some(())
            });
            if done.is_some() {
                send_window_event(window, EventType::WINDOW_MINIMIZED, 0, 0);
            }
        }
    }

    /// Translation of `Wayland_SetWindowMouseRect()`.
    pub(crate) fn wayland_set_window_mouse_rect(&self, window: WindowID) -> Result<()> {
        /* This may look suspiciously like SetWindowGrab, despite SetMouseRect not
         * implicitly doing a grab. And you're right! Wayland doesn't let us mess
         * around with mouse focus whatsoever, so it just happens to be that the
         * work that we can do in these two functions ends up being the same.
         *
         * Just know that this call lets you confine with a rect, SetWindowGrab
         * lets you confine without a rect.
         */
        if self.with_data(|d| d.g.pointer_constraints.is_none()) {
            return Err(Error::new(
                "Failed to grab mouse: compositor lacks support for the required zwp_pointer_constraints_v1 protocol",
            ));
        }
        self.wayland_display_update_pointer_grabs(Some(window));
        Ok(())
    }

    /// Translation of `Wayland_SetWindowMouseGrab()`.
    pub(crate) fn wayland_set_window_mouse_grab(
        &self,
        window: WindowID,
        _grabbed: bool,
    ) -> Result<()> {
        if self.with_data(|d| d.g.pointer_constraints.is_none()) {
            return Err(Error::new(
                "Failed to grab mouse: compositor lacks support for the required zwp_pointer_constraints_v1 protocol",
            ));
        }
        self.wayland_display_update_pointer_grabs(Some(window));
        Ok(())
    }

    /// Translation of `Wayland_SetWindowKeyboardGrab()`.
    pub(crate) fn wayland_set_window_keyboard_grab(
        &self,
        window: WindowID,
        _grabbed: bool,
    ) -> Result<()> {
        if self.with_data(|d| d.g.key_inhibitor_manager.is_none()) {
            return Err(Error::new("Failed to grab keyboard: compositor lacks support for the required zwp_keyboard_shortcuts_inhibit_manager_v1 protocol"));
        }
        self.wayland_display_update_keyboard_grabs(Some(window));
        Ok(())
    }

    /// Translation of `Wayland_ReconfigureWindow()`.
    pub(crate) fn wayland_reconfigure_window(
        &self,
        window: WindowID,
        flags: WindowFlags,
    ) -> Result<()> {
        let mapped = self.with_data(|d| {
            d.window(window).is_some_and(|data| {
                data.shell_surface_status == ShellSurfaceStatus::Shown
                    && data.shell_surface_type != ShellSurfaceType::Custom
            })
        });

        // Don't try to reconfigure mapped windows, unless they are custom or external.
        if mapped {
            // Window is already mapped; abort.
            return Err(Error::new("Window is already mapped"));
        }

        /* The caller guarantees that only one of the GL or Vulkan flags will be set.
         * Note that Vulkan doesn't require any specific configuration, so only EGL
         * objects are added and removed as required.
         */
        if flags.contains(WindowFlags::OPENGL) {
            let nw = self.with_data(|d| {
                let data = d.window_mut(window)?;
                if data.egl_window.is_none() {
                    data.egl_window = EglWindow::create(
                        data.surface(),
                        data.current.pixel_width,
                        data.current.pixel_height,
                    );
                }
                Some(
                    data.egl_window
                        .as_ref()
                        .map_or(std::ptr::null_mut(), |e| e.raw()),
                )
            });
            let Some(nw) = nw else {
                return Err(Error::new("Invalid window"));
            };

            // SDL_EGL_CreateSurface should have set the error.
            let egl_surface = egl::create_surface(Some(window), nw.cast())?;

            self.with_data(|d| {
                if let Some(data) = d.window_mut(window) {
                    data.egl_surface = EglSurface::from_ptr(egl_surface);

                    if data.gles_swap_frame.is_none() {
                        data.gles_swap_frame = GlesSwapFrame::new(
                            &self.conn,
                            data.surface(),
                            data.swap_interval_ready.clone(),
                        );
                    }
                }
            });
        } else {
            let objects = self.with_data(|d| {
                let data = d.window_mut(window)?;
                Some((
                    data.egl_surface.take(),
                    data.egl_window.take(),
                    data.gles_swap_frame.take(),
                ))
            });
            if let Some((egl_surface, egl_window, gles_swap_frame)) = objects {
                if let Some(egl_surface) = egl_surface {
                    egl::destroy_surface(egl_surface.as_ptr());
                }
                drop(egl_window);
                drop(gles_swap_frame);
            }
        }

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Creation
    // -----------------------------------------------------------------------

    /// Translation of `Wayland_CreateWindow()`.
    pub(crate) fn wayland_create_window(
        &self,
        window: WindowID,
        create_props: &Properties,
    ) -> Result<()> {
        let external_surface = create_props
            .get_number(PROP_WINDOW_CREATE_WAYLAND_WL_SURFACE_POINTER)
            .or_else(|| create_props.get_number("sdl2-compat.external_window"))
            .filter(|&p| p != 0)
            .map(|p| p as usize as *mut wl_proxy);
        let custom_surface_role = external_surface.is_some()
            || create_props
                .get_bool(PROP_WINDOW_CREATE_WAYLAND_SURFACE_ROLE_CUSTOM_BOOLEAN)
                .unwrap_or(false);
        let flags = window_flags(window);
        let create_egl_window = flags.contains(WindowFlags::OPENGL)
            || create_props
                .get_bool(PROP_WINDOW_CREATE_WAYLAND_CREATE_EGL_WINDOW_BOOLEAN)
                .unwrap_or(false);

        let enable_insets = create_props
            .get_bool(PROP_WINDOW_CREATE_WAYLAND_ENABLE_INSETS_BOOLEAN)
            .unwrap_or(false);

        let _ = with_window(window, |w| {
            if w.core.x == crate::video::sysvideo::WINDOWPOS_UNDEFINED {
                w.core.x = 0;
            }
            if w.core.y == crate::video::sysvideo::WINDOWPOS_UNDEFINED {
                w.core.y = 0;
            }
        });

        let Some(win) = snapshot(window) else {
            return Err(Error::new("Invalid window"));
        };
        let parent_win = win.parent.and_then(snapshot);

        /* The popup scale comes from the parent. (Upstream copies the parent's
         * scale_to_display too, then overwrites it with the device's.)
         */
        let parent_scale_factor = self.with_data(|d| {
            parent_win
                .as_ref()
                .and_then(|p| d.window(p.id))
                .map_or(1.0, |p| p.scale_factor)
        });

        let mut scale_factor = 1.0;
        if win.is_popup() {
            let mut x = win.x;
            let mut y = win.y;
            if let Some(p) = &parent_win {
                ensure_popup_position_is_valid(win.w, win.h, p.w, p.h, &mut x, &mut y);
            }
            let _ = with_window(window, |w| {
                w.core.x = x;
                w.core.y = y;
            });
            scale_factor = parent_scale_factor;
        } else {
            self.with_data(|d| {
                for o in d.output_list.iter().filter(|o| o.display != 0) {
                    scale_factor = f64::max(scale_factor, o.scale_factor);
                }
            });
        }

        // Cache the app_id at creation time, as it may change before the window is mapped.
        let app_id = crate::core::unix::app_id();

        let created = self.with_data(|c| -> Result<()> {
            let scale_to_display = c.scale_to_display_enabled;
            let compositor =
                c.g.compositor
                    .as_ref()
                    .ok_or_else(|| Error::new("No wl_compositor"))?;

            let surface = match external_surface {
                None => {
                    let surface = compositor.create_surface();
                    surface.obj().set_tag(&SDL_WAYLAND_SURFACE_TAG);
                    WindowSurface {
                        proxy: std::mem::ManuallyDrop::new(surface),
                        external: false,
                    }
                }
                Some(raw) => {
                    /* External surfaces are registered by being put in a list, as changing tags or userdata
                     * can cause problems with external toolkits.
                     *
                     * (here: the window data list, see VideoData::window_for_surface())
                     */
                    // SAFETY: the application's live wl_surface; it is never
                    // destroyed or given a listener by us (forgotten on drop).
                    let proxy = unsafe {
                        Proxy::<WlSurface>::from_new(NonNull::new_unchecked(raw), &self.conn)
                    };
                    WindowSurface {
                        proxy: std::mem::ManuallyDrop::new(proxy),
                        external: true,
                    }
                }
            };

            let mut data = WaylandWindowData {
                sdlwindow: window,
                surface,
                surface_frame_callback: None,
                gles_swap_frame: None,
                shell_surface: ShellSurface::default(),
                shell_surface_type: ShellSurfaceType::Unknown,
                shell_surface_status: ShellSurfaceStatus::Hidden,
                // Default to all capabilities
                wm_caps: WAYLAND_WM_CAPS_ALL,
                toplevel_constraints: 0,
                resize_edge: 0,
                server_decoration: None,
                idle_inhibitor: None,
                activation_token: None,
                viewport: None,
                fractional_scale: None,
                exported: None,
                xdg_dialog_v1: None,
                wp_alpha_modifier_surface_v1: None,
                xdg_toplevel_icon_v1: None,
                frog_color_managed_surface: None,
                wp_color_management_surface_feedback: None,
                xdg_toplevel_session: None,
                egl_window: None,
                egl_surface: None,
                color_info_state: None,
                outputs: Vec::new(),
                app_id: app_id.clone(),
                session_id: None,
                scale_factor,
                icon_buffers: Vec::new(),
                keyboard_focus_count: 0,
                pointer_focus_count: 0,
                active_touch_count: 0,
                pointer_scale: PointerScale::default(),
                swap_interval_ready: Arc::new(AtomicI32::new(0)),
                requested: RequestedSize::default(),
                current: CurrentSize::default(),
                last_configure: LastConfigure::default(),
                system_limits: SystemLimits::default(),
                toplevel_bounds: ToplevelBounds::default(),
                mask: Mask::default(),
                text_input_props: TextInputProps::default(),
                explicit_geometry: Rect::default(),
                last_display_id: 0,
                pending_state_deadline_count: 0,
                last_focus_event_time_ns: 0,
                last_resize_event_time_ns: 0,
                icc_fd: -1,
                icc_size: 0,
                floating: false,
                suspended: false,
                resizing: false,
                active: false,
                pending_config_ack: false,
                pending_state_commit: false,
                limits_changed: false,
                is_fullscreen: false,
                fullscreen_exclusive: false,
                drop_fullscreen_requests: false,
                showing_window: false,
                fullscreen_was_positioned: false,
                show_hide_sync_required: false,
                scale_to_display,
                reparenting_required: false,
                double_buffer: false,
                accepts_drag_and_drop: false,
                enable_insets,
                hit_test_result: HitTestResult::Normal,
                framebuffer: None,
            };
            let scale = window_scale(win.flags, &data);

            if !data.scale_to_display {
                data.requested.logical_width = win.floating.w;
                data.requested.logical_height = win.floating.h;
            } else {
                data.requested.logical_width = pixel_to_point(scale, win.floating.w);
                data.requested.logical_height = pixel_to_point(scale, win.floating.h);
                data.requested.pixel_width = win.floating.w;
                data.requested.pixel_height = win.floating.h;
            }

            if external_surface.is_none() {
                // SAFETY: the surface is ours and has no listener yet.
                let proxy: &mut Proxy<WlSurface> = &mut data.surface.proxy;
                self.listen(proxy, move |v, obj, ev| {
                    v.handle_surface_event(window, obj, ev)
                });
            }

            /* Always attach a viewport and fractional scale manager if available and the surface is not custom/external,
             * or the custom/external surface was explicitly flagged as high pixel density aware, which signals that the
             * application wants SDL to handle scaling.
             */
            if !custom_surface_role || win.flags.contains(WindowFlags::HIGH_PIXEL_DENSITY) {
                if let Some(viewporter) = &c.g.viewporter {
                    let viewport = viewporter.get_viewport(data.surface.obj());

                    // The viewport always uses the entire buffer.
                    viewport.set_source(
                        Fixed::from_int(-1),
                        Fixed::from_int(-1),
                        Fixed::from_int(-1),
                        Fixed::from_int(-1),
                    );
                    data.viewport = Some(viewport);
                }
                if let Some(manager) = &c.g.fractional_scale_manager {
                    let mut fractional_scale = manager.get_fractional_scale(data.surface.obj());
                    self.listen(
                        &mut fractional_scale,
                        move |v, _, WpFractionalScaleV1Event::PreferredScale { scale }| {
                            // Translation of `handle_fractional_scale_preferred()`.
                            let factor = scale as f64 / 120.0; // 120 is a magic number defined in the spec as a common denominator
                            v.handle_preferred_scale_changed(window, factor);
                        },
                    );
                    data.fractional_scale = Some(fractional_scale);
                }
            }

            if !custom_surface_role {
                if let Some(manager) = &c.g.wp_color_manager_v1 {
                    let mut feedback = manager.get_surface_feedback(data.surface.obj());
                    self.listen(&mut feedback, move |v, _, _| {
                        // Translation of `handle_surface_feedback_preferred_changed()`
                        // and `handle_surface_feedback_preferred_changed2()`.
                        v.wayland_get_color_info_for_window(window, false);
                    });
                    data.wp_color_management_surface_feedback = Some(feedback);
                } else if let Some(factory) = &c.g.frog_color_management_factory_v1 {
                    let mut frog = factory.get_color_managed_surface(data.surface.obj());
                    self.listen(&mut frog, move |_, _, ev| {
                        // Translation of `frog_preferred_metadata_handler()`.
                        let FrogColorManagedSurfaceEvent::PreferredMetadata {
                            transfer_function,
                            max_luminance,
                            ..
                        } = ev;
                        let hdr = frog_hdr(transfer_function, max_luminance);
                        crate::video::display::set_window_hdr_properties(window, hdr, true);
                    });
                    data.frog_color_managed_surface = Some(frog);
                }

                if let Some(alpha_modifier) = &c.g.wp_alpha_modifier_v1 {
                    let surface = alpha_modifier.get_surface(data.surface.obj());
                    surface.set_multiplier(u32::MAX);
                    data.wp_alpha_modifier_surface_v1 = Some(surface);
                }
            }

            if !custom_surface_role {
                if c.libdecor.is_some() && !win.is_popup() {
                    data.shell_surface_type = ShellSurfaceType::Libdecor;
                } else if c.g.xdg_wm_base.is_some() {
                    if win.is_popup() {
                        data.shell_surface_type = ShellSurfaceType::XdgPopup;
                    } else {
                        data.shell_surface_type = ShellSurfaceType::XdgToplevel;
                    }
                } // All other cases will be WAYLAND_SURFACE_UNKNOWN
            } else {
                // Roleless and external surfaces are always considered to be in the shown state by the backend.
                data.shell_surface_type = ShellSurfaceType::Custom;
                data.shell_surface_status = ShellSurfaceStatus::Shown;
            }

            if hints::get_bool(hints::VIDEO_DOUBLE_BUFFER, false) {
                data.double_buffer = true;
            }

            c.windows.push(data);
            Ok(())
        });
        created?;

        if external_surface.is_some() {
            let _ = with_window(window, |w| {
                w.core.flags |= WindowFlags::EXTERNAL;
                // External windows are presumed to be shown.
                w.core.flags &= !WindowFlags::HIDDEN;
            });
        }

        if self.with_data(|d| {
            d.window(window)
                .is_some_and(|w| w.wp_color_management_surface_feedback.is_some())
        }) {
            self.wayland_get_color_info_for_window(window, true);
        }

        // Must be called before EGL configuration to set the drawable backbuffer size.
        self.configure_window_geometry(window);

        /* Fire a callback when the compositor wants a new frame rendered.
         * Right now this only matters for OpenGL; we use this callback to add a
         * wait timeout that avoids getting deadlocked by the compositor when the
         * window isn't visible.
         */
        if flags.contains(WindowFlags::OPENGL) {
            self.with_data(|d| {
                if let Some(data) = d.window_mut(window) {
                    data.gles_swap_frame = GlesSwapFrame::new(
                        &self.conn,
                        data.surface(),
                        data.swap_interval_ready.clone(),
                    );
                }
            });
        }

        if flags.contains(WindowFlags::TRANSPARENT) {
            with_gl_config(|c| {
                if c.alpha_size == 0 {
                    c.alpha_size = 8;
                }
            });
        }

        let mut egl_window_raw: *mut wl_egl_window = std::ptr::null_mut();
        if create_egl_window {
            egl_window_raw = self
                .with_data(|d| {
                    let data = d.window_mut(window)?;
                    data.egl_window = EglWindow::create(
                        data.surface(),
                        data.current.pixel_width,
                        data.current.pixel_height,
                    );
                    data.egl_window.as_ref().map(|e| e.raw())
                })
                .unwrap_or(std::ptr::null_mut());
        }

        if flags.contains(WindowFlags::OPENGL) {
            // Create the GLES window surface
            // (SDL_EGL_CreateSurface should have set error)
            let egl_surface = egl::create_surface(Some(window), egl_window_raw.cast())?;
            self.with_data(|d| {
                if let Some(data) = d.window_mut(window) {
                    data.egl_surface = EglSurface::from_ptr(egl_surface);
                }
            });
        }

        // We may need to create an idle inhibitor for this new window
        let suspend = crate::video::core::with_device(|v| v.suspend_screensaver).unwrap_or(false);
        let _ = self.wayland_suspend_screen_saver(suspend);

        let Some((surface_raw, viewport_raw, ty)) = self.with_data(|d| {
            d.window(window).map(|w| {
                (
                    w.surface().raw(),
                    w.viewport
                        .as_ref()
                        .map_or(std::ptr::null_mut(), |v| v.raw()),
                    w.shell_surface_type,
                )
            })
        }) else {
            return Err(Error::new("Invalid window"));
        };
        let props = win.props;
        set_ptr_prop(
            &props,
            PROP_WINDOW_WAYLAND_DISPLAY_POINTER,
            self.conn.raw().cast(),
        );
        set_ptr_prop(
            &props,
            PROP_WINDOW_WAYLAND_SURFACE_POINTER,
            surface_raw.cast(),
        );
        set_ptr_prop(
            &props,
            PROP_WINDOW_WAYLAND_VIEWPORT_POINTER,
            viewport_raw.cast(),
        );
        set_ptr_prop(
            &props,
            PROP_WINDOW_WAYLAND_EGL_WINDOW_POINTER,
            egl_window_raw.cast(),
        );
        if ty == ShellSurfaceType::XdgToplevel || ty == ShellSurfaceType::Libdecor {
            if let Some(window_id) = create_props
                .get_string(PROP_WINDOW_CREATE_WAYLAND_WINDOW_ID_STRING)
                .filter(|id| !id.is_empty())
            {
                let _ = props.set(PROP_WINDOW_WAYLAND_WINDOW_ID_STRING, window_id);
            }
        }

        Ok(())
    }

    /// Translation of `Wayland_SetWindowMinimumSize()`.
    pub(crate) fn wayland_set_window_minimum_size(&self, window: WindowID) {
        // Will be committed when Wayland_SetWindowSize() is called by the video core.
        self.set_limits_changed(window);
    }

    /// Translation of `Wayland_SetWindowMaximumSize()`.
    pub(crate) fn wayland_set_window_maximum_size(&self, window: WindowID) {
        // Will be committed when Wayland_SetWindowSize() is called by the video core.
        self.set_limits_changed(window);
    }

    /// Translation of `Wayland_SetWindowAspectRatio()`.
    pub(crate) fn wayland_set_window_aspect_ratio(&self, window: WindowID) {
        // Will be committed when Wayland_SetWindowSize() is called by the video core.
        self.set_limits_changed(window);
    }

    fn set_limits_changed(&self, window: WindowID) {
        self.with_data(|d| {
            if let Some(w) = d.window_mut(window) {
                w.limits_changed = true;
            }
        });
    }

    /// Translation of `Wayland_SetWindowPosition()`.
    pub(crate) fn wayland_set_window_position(&self, window: WindowID) -> Result<()> {
        let Some(ty) = self.with_data(|d| d.window(window).map(|w| w.shell_surface_type)) else {
            return Err(Error::new("Invalid window"));
        };

        // Only popup windows can be positioned relative to the parent.
        if ty == ShellSurfaceType::XdgPopup {
            let too_old = self.with_data(|d| {
                d.window(window)
                    .and_then(|w| w.shell_surface.xdg_popup.as_ref())
                    .is_some_and(|p| p.version() < XdgPopup::REPOSITION_SINCE_VERSION)
            });
            if too_old {
                return Err(Error::unsupported());
            }

            if let Some(win) = snapshot(window) {
                if let Some(parent) = win.parent.and_then(snapshot) {
                    let syms = self.syms().clone();
                    self.with_data(|d| reposition_popup(&d.windows, &syms, &win, &parent, false));
                }
            }
            return Ok(());
        } else if ty == ShellSurfaceType::Libdecor || ty == ShellSurfaceType::XdgToplevel {
            /* Catch up on any pending state before attempting to change the fullscreen window
             * display via a set fullscreen call to make sure the window doesn't have a pending
             * leave fullscreen event that it might override.
             */
            self.flush_pending_events(window);

            if self.with_data(|d| d.window(window).is_some_and(|w| w.is_fullscreen)) {
                let display = display_for_fullscreen_window(window).unwrap_or(0);
                let target = self.with_data(|d| {
                    let last_display_id = d.window(window)?.last_display_id;
                    if display != 0 && last_display_id != display {
                        Some(d.output_for_display(display).map(|o| o.registry_id))
                    } else {
                        None
                    }
                });
                if let Some(output) = target {
                    self.set_fullscreen(window, output, true);

                    return Ok(());
                }
            }
        }
        Err(Error::new("wayland cannot position non-popup windows"))
    }

    /// Translation of `Wayland_SetWindowSize()`.
    pub(crate) fn wayland_set_window_size(&self, window: WindowID) {
        /* Flush any pending state operations, as fullscreen windows do not get
         * explicitly resized, not strictly obeying the size of a maximized window
         * is a protocol violation, and pending restore events might result in a
         * configure event overwriting the requested size.
         *
         * Calling this on a custom surface is informative, so the size must
         * always be passed through.
         */
        self.flush_pending_events(window);

        let Some(win) = snapshot(window) else {
            return;
        };
        let maximized = win.flags.contains(WindowFlags::MAXIMIZED);
        let resizable_state = !win
            .flags
            .intersects(WindowFlags::MAXIMIZED | WindowFlags::FULLSCREEN);

        let Some(store_floating) = self.with_data(|d| {
            let wind = d.window_mut(window)?;
            let scale = window_scale(win.flags, wind);
            let mut store_floating = None;

            /* Maximized and fullscreen windows don't get resized, and the new size is ignored
             * if this is just to recalculate the min/max or aspect limits on a tiled window.
             *
             * Some compositors will mark tiled windows as maximized. In these cases, the content
             * size is recalculated, but window size itself is left alone, as the spec states that
             * the configure size of a maximized window must be obeyed.
             */
            if resizable_state
                || (win.tiled && !wind.limits_changed)
                || wind.shell_surface_type == ShellSurfaceType::Custom
            {
                // Don't change the window size if maximized and tiled.
                if !maximized {
                    if !wind.scale_to_display {
                        wind.requested.logical_width = win.pending.w;
                        wind.requested.logical_height = win.pending.h;
                    } else {
                        wind.requested.logical_width = pixel_to_point(scale, win.pending.w);
                        wind.requested.logical_height = pixel_to_point(scale, win.pending.h);
                        wind.requested.pixel_width = win.pending.w;
                        wind.requested.pixel_height = win.pending.h;
                    }
                }

                /* If a non-resizable window somehow wound up in the tiled state, store the new
                 * size in the floating parameters so the size will be preserved for future
                 * configure events.
                 *
                 * This shouldn't happen, but apparently it can on at least one compositor...
                 */
                if win.tiled && !win.flags.contains(WindowFlags::RESIZABLE) {
                    store_floating = Some(true);
                }
            } else {
                // Can't resize the window.
                store_floating = Some(false);
            }

            wind.limits_changed = false;
            if wind.shell_surface_status == ShellSurfaceStatus::Shown {
                wind.pending_state_commit = true;
            }
            Some(store_floating)
        }) else {
            return;
        };

        match store_floating {
            Some(true) => {
                let _ = with_window(window, |w| {
                    w.core.floating.w = w.pending.w;
                    w.core.floating.h = w.pending.h;
                });
            }
            Some(false) => {
                let _ = with_window(window, |w| w.core.last_size_pending = false);
            }
            None => {}
        }

        // Always recalculate the geometry, as this may be in response to a min/max limit change.
        self.configure_window_geometry(window);
        self.commit_libdecor_frame(window);
    }

    /// Translation of `Wayland_GetWindowSizeInPixels()`.
    pub(crate) fn wayland_get_window_size_in_pixels(&self, window: WindowID) -> (i32, i32) {
        self.with_data(|d| {
            d.window(window).map_or((0, 0), |data| {
                (data.current.pixel_width, data.current.pixel_height)
            })
        })
    }

    /// Translation of `Wayland_GetWindowContentScale()`.
    pub(crate) fn wayland_get_window_content_scale(&self, window: WindowID) -> f32 {
        let flags = window_flags(window);
        self.with_data(|d| {
            let Some(wind) = d.window(window) else {
                return 1.0;
            };
            if flags.contains(WindowFlags::HIGH_PIXEL_DENSITY)
                || wind.scale_to_display
                || wind.fullscreen_exclusive
            {
                wind.scale_factor as f32
            } else {
                1.0
            }
        })
    }

    /// Translation of `Wayland_GetDisplayForWindow()`.
    pub(crate) fn wayland_get_display_for_window(&self, window: WindowID) -> DisplayID {
        self.with_data(|d| d.window(window).map_or(0, |wind| wind.last_display_id))
    }

    /// Translation of `Wayland_SetWindowOpacity()`.
    pub(crate) fn wayland_set_window_opacity(&self, window: WindowID, opacity: f32) -> Result<()> {
        let flags = window_flags(window);
        self.with_data(|d| {
            let VideoData { g, windows, .. } = d;
            let wind = find_mut(windows, window).ok_or_else(|| Error::new("Invalid window"))?;
            let Some(alpha_modifier) = &wind.wp_alpha_modifier_surface_v1 else {
                return Err(Error::new(
                    "wayland: set window opacity failed; compositor lacks support for the required wp_alpha_modifier_v1 protocol",
                ));
            };
            let is_opaque = !flags.contains(WindowFlags::TRANSPARENT) && opacity == 1.0;

            if wind.mask.mapped && wind.mask.opaque != is_opaque {
                let old_buffer = wind.mask.buffer.take();
                wind.mask.opaque = is_opaque;
                wind.mask.buffer =
                    wayland_create_single_pixel_buffer(g, 0, 0, 0, if is_opaque { u32::MAX } else { 0 });

                if let Some(mask_surface) = &wind.mask.surface {
                    mask_surface.attach(wind.mask.buffer.as_ref().map(|b| b.obj()), 0, 0);
                    damage_all(mask_surface.obj(), mask_surface.version());

                    if is_opaque {
                        set_surface_opaque_region(
                            g,
                            mask_surface.obj(),
                            wind.current.logical_width,
                            wind.current.logical_height,
                        );
                    } else {
                        set_surface_opaque_region(g, mask_surface.obj(), 0, 0);
                    }

                    mask_surface.commit();
                }

                drop(old_buffer);
            }

            if is_opaque {
                set_surface_opaque_region(
                    g,
                    wind.surface.obj(),
                    wind.current.viewport_width,
                    wind.current.viewport_height,
                );
            } else {
                set_surface_opaque_region(g, wind.surface.obj(), 0, 0);
            }

            alpha_modifier.set_multiplier((u32::MAX as f64 * opacity as f64) as u32);

            Ok(())
        })
    }

    /// Translation of `Wayland_SetWindowTitle()`.
    pub(crate) fn wayland_set_window_title(&self, window: WindowID) {
        let title = with_window(window, |w| w.title.clone())
            .ok()
            .flatten()
            .unwrap_or_default();
        let Some(ty) = self.with_data(|d| d.window(window).map(|w| w.shell_surface_type)) else {
            return;
        };

        if ty == ShellSurfaceType::Libdecor {
            let title = super::video::c_string(&title);
            // SAFETY: the frame is alive; libdecor copies the title.
            self.with_libdecor_frame(window, |l, frame| unsafe {
                (l.libdecor_frame_set_title)(frame, title.as_ptr())
            });
        } else if ty == ShellSurfaceType::XdgToplevel {
            self.with_data(|d| {
                if let Some(toplevel) = d
                    .window(window)
                    .and_then(|w| w.shell_surface.xdg_toplevel.as_ref())
                {
                    toplevel.set_title(&title);
                }
            });
        }
    }

    /// Translation of `Wayland_SetWindowIcon()`.
    pub(crate) fn wayland_set_window_icon(
        &self,
        window: WindowID,
        icon: &Surface<'static>,
    ) -> Result<()> {
        if self.with_data(|d| d.g.xdg_toplevel_icon_manager_v1.is_none()) {
            return Err(Error::new(
                "wayland: cannot set icon; required xdg_toplevel_icon_v1 protocol not supported",
            ));
        }

        let mut images: Vec<&Surface<'_>> = std::iter::once(icon)
            .chain(icon.alternate_images().iter())
            .collect();

        // Release the old icon resources.
        let old = self.with_data(|d| {
            let wind = d.window_mut(window)?;
            Some((
                wind.xdg_toplevel_icon_v1.take(),
                std::mem::take(&mut wind.icon_buffers),
            ))
        });
        drop(old);

        // Calculate the size of the buffer pool.
        let mut pool_size: usize = 0;
        for image in &images {
            // Images must be square. Non-square images will be centered.
            let size = image.width().max(image.height()) as usize;
            pool_size += size * size * 4;
        }

        // Sort the images in ascending order by size.
        images.sort_by_key(|s| s.width() as i64 * s.height() as i64);

        let result = self.with_data(|d| -> Result<()> {
            let VideoData { g, windows, .. } = d;
            let manager = g
                .xdg_toplevel_icon_manager_v1
                .as_ref()
                .ok_or_else(|| Error::new("wayland: cannot set icon"))?;
            let wind = find_mut(windows, window).ok_or_else(|| Error::new("Invalid window"))?;
            let icon_proxy = manager.create_icon();
            let mut buffers = Vec::new();

            let shm = g.shm.as_ref().ok_or_else(|| {
                Error::new("wayland: failed to allocate an SHM pool for the icon")
            })?;
            let mut shm_pool = ShmPool::alloc(shm.obj(), pool_size as i32)
                .map_err(|_| Error::new("wayland: failed to allocate an SHM pool for the icon"))?;

            let base_size = icon.width().max(icon.height()) as f64;
            for &image in &images {
                // Choose the largest image for each integer scale, ignoring any below the base size.
                let level_size = image.width().max(image.height());
                let scale = (level_size as f64 / base_size).floor() as i32;
                if scale == 0 {
                    continue;
                }

                let converted;
                let surface: &Surface<'_> = if image.format() != PixelFormat::ARGB8888 {
                    converted = image.convert(PixelFormat::ARGB8888).map_err(|_| {
                        Error::new("wayland: failed to convert the icon image to ARGB8888 format")
                    })?;
                    &converted
                } else {
                    image
                };

                let Some((buffer, buffer_mem)) = shm_pool.alloc_buffer(level_size, level_size)
                else {
                    return Err(Error::new(
                        "wayland: failed to allocate a wl_buffer for the icon",
                    ));
                };

                // Center non-square images.
                let mut offset = 0;
                if surface.width() < level_size {
                    buffer_mem.fill(0);
                    offset = ((level_size - surface.width()) / 2 * 4) as usize;
                } else if surface.height() < level_size {
                    buffer_mem.fill(0);
                    offset = ((level_size - surface.height()) / 2 * (level_size * 4)) as usize;
                }

                let pixels = surface.pixels().unwrap_or(&[]);
                let _ = crate::video::surface::premultiply_alpha(
                    surface.width(),
                    surface.height(),
                    surface.format(),
                    pixels,
                    surface.pitch(),
                    PixelFormat::ARGB8888,
                    &mut buffer_mem[offset..],
                    level_size * 4,
                    true,
                );

                icon_proxy.add_buffer(buffer.obj(), scale);
                buffers.push(buffer);
            }

            if let Some(toplevel) = self.toplevel_for_window(Some(wind)) {
                manager.set_icon(toplevel, Some(icon_proxy.obj()));
            }

            wind.xdg_toplevel_icon_v1 = Some(icon_proxy);
            wind.icon_buffers = buffers;
            // (Wayland_ReleaseSHMPool(): the pool goes, the buffers stay)
            drop(shm_pool);
            Ok(())
        });

        // (failure_cleanup: the icon and its buffers made so far are dropped)
        result
    }

    /// Translation of `Wayland_GetWindowICCProfile()`.
    pub(crate) fn wayland_get_window_icc_profile(&self, window: WindowID) -> Result<Vec<u8>> {
        let (icc_fd, icc_size) =
            self.with_data(|d| d.window(window).map_or((-1, 0), |w| (w.icc_fd, w.icc_size)));

        if icc_size > 0 {
            // SAFETY: a private read-only mapping of the profile descriptor.
            let icc_map = unsafe {
                libc::mmap(
                    std::ptr::null_mut(),
                    icc_size as usize,
                    libc::PROT_READ,
                    libc::MAP_PRIVATE,
                    icc_fd,
                    0,
                )
            };
            if icc_map != libc::MAP_FAILED {
                // SAFETY: the mapping is `icc_size` bytes long.
                let ret =
                    unsafe { std::slice::from_raw_parts(icc_map as *const u8, icc_size as usize) }
                        .to_vec();
                // SAFETY: the mapping made above.
                unsafe { libc::munmap(icc_map, icc_size as usize) };
                return Ok(ret);
            }
        }

        Err(Error::new("The window has no ICC profile"))
    }

    /// Translation of `Wayland_SyncWindow()`.
    pub(crate) fn wayland_sync_window(&self, window: WindowID) -> Result<()> {
        // FIXME (upstream): this loops forever if the connection fails before
        // the sync points are reached (kept as upstream).
        loop {
            let _ = self.conn.roundtrip();
            if self.with_data(|d| {
                d.window(window)
                    .map_or(0, |w| w.pending_state_deadline_count)
            }) == 0
            {
                break;
            }
        }

        Ok(())
    }

    /// Translation of `Wayland_AcceptDragAndDrop()`.
    pub(crate) fn wayland_accept_drag_and_drop(&self, window: WindowID, accept: bool) {
        self.with_data(|d| {
            if let Some(w) = d.window_mut(window) {
                w.accepts_drag_and_drop = accept;
            }
        });
    }

    /// Translation of `Wayland_SetWindowFocusable()`.
    pub(crate) fn wayland_set_window_focusable(
        &self,
        window: WindowID,
        focusable: bool,
    ) -> Result<()> {
        let flags = window_flags(window);
        if flags.contains(WindowFlags::POPUP_MENU) {
            if !flags.contains(WindowFlags::HIDDEN) {
                if !focusable && flags.contains(WindowFlags::INPUT_FOCUS) {
                    if let Ok((Some(new_focus), set_focus)) = should_relinquish_popup_focus(window)
                    {
                        self.set_keyboard_focus(new_focus, set_focus);
                    }
                } else if focusable && should_focus_popup(window) {
                    self.set_keyboard_focus(window, true);
                }
            }

            return Ok(());
        }

        Err(Error::new(
            "wayland: focus can only be toggled on popup menu windows",
        ))
    }

    /// Translation of `Wayland_ShowWindowSystemMenu()`.
    pub(crate) fn wayland_show_window_system_menu(&self, window: WindowID, mut x: i32, mut y: i32) {
        let flags = window_flags(window);
        enum Menu {
            Libdecor(*mut libdecor_frame, *mut wl_proxy, u32),
            Done,
        }
        let menu = self.with_data(|d| {
            let seat = d.seat(d.last_implicit_grab_seat?)?;
            let wind = d.window(window)?;

            if wind.scale_to_display {
                let scale = window_scale(flags, wind);
                x = pixel_to_point(scale, x);
                y = pixel_to_point(scale, y);
            }

            if wind.shell_surface_type == ShellSurfaceType::Libdecor {
                let frame = wind.shell_surface.libdecor_frame.as_ref()?;
                Some(Menu::Libdecor(
                    frame.raw(),
                    seat.wl_seat.raw(),
                    seat.last_implicit_grab_serial,
                ))
            } else if wind.shell_surface_type == ShellSurfaceType::XdgToplevel {
                if let Some(toplevel) = &wind.shell_surface.xdg_toplevel {
                    toplevel.show_window_menu(
                        seat.wl_seat.obj(),
                        seat.last_implicit_grab_serial,
                        x,
                        y,
                    );
                }
                Some(Menu::Done)
            } else {
                Some(Menu::Done)
            }
        });

        if let (Some(Menu::Libdecor(frame, seat, serial)), Some(l)) =
            (menu, libdecor_syms(self.syms()))
        {
            // SAFETY: the frame and seat are alive.
            unsafe { (l.libdecor_frame_show_window_menu)(frame, seat, serial, x, y) };
            resume_pending_panic();
        }
    }

    /// Translation of `Wayland_SuspendScreenSaver()`.
    pub(crate) fn wayland_suspend_screen_saver(&self, suspend_screensaver: bool) -> Result<()> {
        #[cfg(all(unix, not(target_vendor = "apple"), not(target_os = "android")))]
        if crate::core::linux::dbus::screensaver_inhibit(suspend_screensaver) {
            return Ok(());
        }

        /* The idle_inhibit_unstable_v1 protocol suspends the screensaver
           on a per wl_surface basis, but SDL assumes that suspending
           the screensaver can be done independently of any window.

           To reconcile these differences, we propagate the idle inhibit
           state to each window. If there is no window active, we will
           be able to inhibit idle once the first window is created.
        */
        let removed = self.with_data(|data| {
            let VideoData { g, windows, .. } = data;
            let mut removed = Vec::new();
            if let Some(manager) = &g.idle_inhibit_manager {
                for win_data in windows.iter_mut() {
                    if suspend_screensaver && win_data.idle_inhibitor.is_none() {
                        win_data.idle_inhibitor =
                            Some(manager.create_inhibitor(win_data.surface.obj()));
                    } else if !suspend_screensaver {
                        removed.extend(win_data.idle_inhibitor.take());
                    }
                }
            }
            removed
        });
        drop(removed);

        Ok(())
    }

    /// Translation of `Wayland_DestroyWindow()`.
    pub(crate) fn wayland_destroy_window(&self, window: WindowID) {
        let Some(sync_required) =
            self.with_data(|d| d.window(window).map(|w| w.show_hide_sync_required))
        else {
            return;
        };

        /* Roundtrip before destroying the window to make sure that it has received input leave events, so that
         * no internal structures are left pointing to the destroyed window.
         */
        if sync_required {
            /* Make sure hit test isn't executed during roundtrip */
            let _ = with_window(window, |w| w.hit_test = None);
            let _ = self.conn.roundtrip();
        }

        /* The compositor should have relinquished keyboard, pointer, touch, and tablet tool focus when the toplevel
         * window was destroyed upon being hidden, but there is no guarantee of this, so ensure that all references
         * to the window held by seats are released before destroying the underlying surface and struct.
         */
        self.wayland_display_remove_window_references_from_seats(window);

        let wind = self.with_data(|d| {
            let i = d.windows.iter().position(|w| w.sdlwindow == window)?;
            Some(d.windows.remove(i))
        });
        // (the objects are destroyed as the data drops, in upstream's order)
        drop(wind);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn point_pixel_conversions_round_trip() {
        for scale in [1.0, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0] {
            for p in [0, 1, 2, 3, 99, 100, 640, 1919] {
                assert_eq!(
                    pixel_to_point(scale, point_to_pixel(scale, p)),
                    p,
                    "{scale} {p}"
                );
            }
        }
        // Nonzero sizes never round down to zero.
        assert_eq!(point_to_pixel(0.1, 1), 1);
        assert_eq!(pixel_to_point(4.0, 1), 1);
        assert_eq!(point_to_pixel(2.0, 0), 0);
    }

    #[test]
    fn popups_are_kept_adjoining_their_parent() {
        let (mut x, mut y) = (-500, 10);
        ensure_popup_position_is_valid(100, 50, 640, 480, &mut x, &mut y);
        assert_eq!((x, y), (-100, 10));

        // Corner to corner: nudged by one to adjoin.
        let (mut x, mut y) = (700, 500);
        ensure_popup_position_is_valid(100, 50, 640, 480, &mut x, &mut y);
        assert_eq!((x, y), (639, 480));

        let (mut x, mut y) = (-200, -200);
        ensure_popup_position_is_valid(100, 50, 640, 480, &mut x, &mut y);
        assert_eq!((x, y), (-99, -50));
    }

    fn test_win() -> Win {
        Win {
            id: 1,
            flags: WindowFlags::RESIZABLE,
            x: 0,
            y: 0,
            w: 640,
            h: 480,
            pending: Rect::default(),
            floating: Rect::default(),
            windowed: Rect::default(),
            min_w: 0,
            min_h: 0,
            max_w: 0,
            max_h: 0,
            min_aspect: 0.0,
            max_aspect: 0.0,
            opacity: 1.0,
            fullscreen_exclusive: false,
            current_fullscreen_mode: DisplayMode::default(),
            undefined_x: false,
            undefined_y: false,
            last_position_pending: false,
            tiled: false,
            parent: None,
            children: Vec::new(),
            constrain_popup: false,
            props: Properties::new(),
        }
    }

    #[test]
    fn geometry_limits_apply_the_aspect_ratio_and_size_limits() {
        let mut win = test_win();
        win.min_aspect = 1.0;
        win.max_aspect = 1.0;
        // A square window, too wide: the width follows the height...
        assert_eq!(apply_geometry_limits(&win, false, 0, 400, 300), (300, 300));
        // ...too tall: the height follows the width.
        assert_eq!(apply_geometry_limits(&win, false, 0, 300, 400), (300, 300));
        // Resizing horizontally: the height follows.
        assert_eq!(
            apply_geometry_limits(&win, true, WAYLAND_RESIZE_EDGE_LR, 500, 300),
            (500, 500)
        );
        // Resizing vertically: the width follows.
        assert_eq!(
            apply_geometry_limits(&win, true, WAYLAND_RESIZE_EDGE_TB, 500, 300),
            (300, 300)
        );

        let mut win = test_win();
        win.min_w = 200;
        win.min_h = 100;
        win.max_w = 800;
        win.max_h = 600;
        assert_eq!(apply_geometry_limits(&win, false, 0, 100, 50), (200, 100));
        assert_eq!(
            apply_geometry_limits(&win, false, 0, 1000, 1000),
            (800, 600)
        );
    }

    #[test]
    fn toplevel_states_are_read() {
        let states: Vec<u8> = [
            XdgToplevelState::MAXIMIZED.0,
            XdgToplevelState::ACTIVATED.0,
            13,
        ]
        .iter()
        .flat_map(|s| s.to_ne_bytes())
        .collect();
        let s = parse_toplevel_states(&states);
        assert!(s.maximized && s.active && !s.floating && !s.fullscreen);
        assert_eq!(s.constraints, WAYLAND_TOPLEVEL_CONSTRAINED_BOTTOM);
        assert!(parse_toplevel_states(&[]).floating);

        let caps: Vec<u8> = [1u32, 4].iter().flat_map(|s| s.to_ne_bytes()).collect();
        assert_eq!(
            wm_caps_from_xdg(&caps),
            WAYLAND_WM_CAPS_WINDOW_MENU | WAYLAND_WM_CAPS_MINIMIZE
        );
    }

    #[test]
    fn frog_metadata_maps_to_hdr_headroom() {
        let pq = frog_hdr(FrogColorManagedSurfaceTransferFunction::ST2084_PQ, 406);
        assert_eq!(pq.hdr_headroom, 2.0);
        assert_eq!(pq.sdr_white_level, 1.0);
        let scrgb = frog_hdr(FrogColorManagedSurfaceTransferFunction::SCRGB_LINEAR, 160);
        assert_eq!(scrgb.hdr_headroom, 2.0);
        assert_eq!(
            frog_hdr(FrogColorManagedSurfaceTransferFunction(0), 1000).hdr_headroom,
            1.0
        );
    }
}
