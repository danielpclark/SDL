// Rust translation of src/video/wayland/SDL_waylandvideo.c and
// SDL_waylandvideo.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Wayland video device: connecting to the compositor, the driver's
//! private data (`SDL_VideoData`), the registry of globals, the outputs
//! (displays), initialization and shutdown, and the table of entry points
//! (the [`VideoDriver`] implementation).
//!
//! The mutable state lives behind a re-entrant lock and a `RefCell`
//! ([`WaylandVideo::with_data`]), borrowed only for short reads and writes:
//! never across a call back into SDL (events, focus changes...) or a
//! dispatch of Wayland events, since both may come back into the driver.
//! Protocol listeners hold a weak reference to the device.

use std::cell::RefCell;
use std::ffi::CString;
use std::ptr::NonNull;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use super::client::{AsProxy, Callbacks, Conn, Obj, Proxy, Tag};
use super::color::ColorInfoState;
use super::events::WaylandSeat;
use super::eventthread::EventThreadContext;
use super::mouse::{CursorTheme, MouseData};
use super::protocols::alpha_modifier_v1::*;
use super::protocols::color_management_v1::*;
use super::protocols::cursor_shape_v1::*;
use super::protocols::fractional_scale_v1::*;
use super::protocols::frog_color_management_v1::*;
use super::protocols::idle_inhibit_unstable_v1::*;
use super::protocols::input_timestamps_unstable_v1::*;
use super::protocols::keyboard_shortcuts_inhibit_unstable_v1::*;
use super::protocols::pointer_constraints_unstable_v1::*;
use super::protocols::pointer_gestures_unstable_v1::*;
use super::protocols::pointer_warp_v1::*;
use super::protocols::relative_pointer_unstable_v1::*;
use super::protocols::single_pixel_buffer_v1::*;
use super::protocols::tablet_v2::*;
use super::protocols::text_input_unstable_v3::*;
use super::protocols::viewporter::*;
use super::protocols::wayland::*;
use super::protocols::wp_primary_selection_unstable_v1::*;
use super::protocols::xdg_activation_v1::*;
use super::protocols::xdg_decoration_unstable_v1::*;
use super::protocols::xdg_dialog_v1::*;
use super::protocols::xdg_foreign_unstable_v2::*;
use super::protocols::xdg_output_unstable_v1::*;
use super::protocols::xdg_session_management_v1::*;
use super::protocols::xdg_shell::*;
use super::protocols::xdg_toplevel_icon_v1::*;
use super::protocols::xdg_toplevel_tag_v1::*;
use super::sys::*;
use super::window::WaylandWindowData;
use super::wldyn::{load_symbols, WaylandSyms};
use crate::error::{Error, Result};
use crate::events::mouse::{Cursor, CursorFrame, MouseButtonFlags, MouseFeature, SystemCursor};
use crate::events::window::send_display_event;
use crate::events::{DisplayID, EventType, WindowID};
use crate::hints;
use crate::properties::Properties;
use crate::thread::ReentrantMutex;
use crate::video::display::{
    add_fullscreen_display_mode, add_fullscreen_display_mode_to, add_video_display,
    del_video_display, finalize_display_mode, reset_fullscreen_display_modes,
    set_desktop_display_mode, set_display_content_scale,
};
use crate::video::messagebox::MessageBoxData;
use crate::video::sysvideo::{
    DeviceCaps, DisplayMode, DisplayOrientation, FlashOperation, FullscreenOp, FullscreenResult,
    HdrOutputProperties, VideoBootStrap, VideoDisplay, VideoDriver,
};
use crate::video::window::WindowOp;
use crate::video::{PixelFormat, Rect, Surface};

pub(crate) const WAYLANDVID_DRIVER_NAME: &str = "wayland";

// The versions of the core protocol interfaces to bind: upstream clamps them
// to what the libwayland it was built against knows; the bundled
// wayland.xml (the one upstream ships) knows these.
const SDL_WL_COMPOSITOR_VERSION: u32 = 6;
const SDL_WL_SEAT_VERSION: u32 = 11;
const SDL_WL_OUTPUT_VERSION: u32 = 4;
const SDL_WL_SHM_VERSION: u32 = 2;
/// The SDL libwayland-client minimum is 1.18, which supports version 3.
#[allow(dead_code)] // (kept as upstream, which binds the data device manager with a literal)
const SDL_WL_DATA_DEVICE_VERSION: u32 = 3;
/// wl_fixes was introduced in 1.24.0
const SDL_WL_FIXES_VERSION: u32 = 2;

const DISPLAY_INFO_NODE: &str = "org.gnome.Mutter.DisplayConfig";
const DISPLAY_INFO_PATH: &str = "/org/gnome/Mutter/DisplayConfig";
const DISPLAY_INFO_METHOD: &str = "GetCurrentState";

/// The global property with the application's `wl_display`, if it brings
/// its own (or ours). Translation of
/// `SDL_PROP_GLOBAL_VIDEO_WAYLAND_WL_DISPLAY_POINTER` (an address).
pub const PROP_GLOBAL_VIDEO_WAYLAND_WL_DISPLAY_POINTER: &str = "SDL.video.wayland.wl_display";
/// The session ID of xdg-session-management. Translation of
/// `SDL_PROP_GLOBAL_VIDEO_WAYLAND_SESSION_ID_STRING`.
pub const PROP_GLOBAL_VIDEO_WAYLAND_SESSION_ID_STRING: &str = "SDL.video.wayland.session_id";
/// The `wl_output` of a display (an address). Translation of
/// `SDL_PROP_DISPLAY_WAYLAND_WL_OUTPUT_POINTER`.
pub const PROP_DISPLAY_WAYLAND_WL_OUTPUT_POINTER: &str = "SDL.display.wayland.wl_output";

/// The tag of the surfaces of SDL windows (`SDL_WAYLAND_surface_tag`).
pub(crate) static SDL_WAYLAND_SURFACE_TAG: Tag = Tag(c"sdl-window".as_ptr());
/// The tag of the outputs SDL bound (`SDL_WAYLAND_output_tag`).
pub(crate) static SDL_WAYLAND_OUTPUT_TAG: Tag = Tag(c"sdl-output".as_ptr());

/// A rectangle of an output, in logical or pixel units.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) struct OutputRect {
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) width: i32,
    pub(crate) height: i32,
}

/// An output. Translation of `struct SDL_DisplayData`; identified by the
/// registry name of its `wl_output` (`registry_id`), which is also the
/// output's user data.
pub(crate) struct WaylandDisplayData {
    pub(crate) output: Proxy<WlOutput>,
    pub(crate) xdg_output: Option<Proxy<ZxdgOutputV1>>,
    pub(crate) wp_color_management_output: Option<Proxy<WpColorManagementOutputV1>>,
    pub(crate) color_info_state: Option<ColorInfoState>,
    pub(crate) wl_output_name: Option<String>,
    pub(crate) scale_factor: f64,
    /// The integer scale factor set by wl_output.
    pub(crate) integer_scale_factor: i32,
    pub(crate) registry_id: u32,

    pub(crate) logical: OutputRect,
    pub(crate) pixel: OutputRect,

    /// Physical width in millimeters.
    pub(crate) physical_width_mm: i32,
    /// Physical height in millimeters.
    pub(crate) physical_height_mm: i32,

    /// Refresh in mHz
    pub(crate) refresh: i32,
    /// wl_output_transform enum
    pub(crate) transform: WlOutputTransform,
    pub(crate) orientation: DisplayOrientation,

    pub(crate) hdr: HdrOutputProperties,

    /// The display, once added (0 before).
    pub(crate) display: DisplayID,
    /// The display record being filled in before it is added.
    pub(crate) placeholder: VideoDisplay,

    pub(crate) wl_output_done_count: i32,
    pub(crate) has_logical_position: bool,
    pub(crate) has_logical_size: bool,
    pub(crate) geometry_changed: bool,
}

/// The globals bound from the registry (the protocol objects of
/// `SDL_VideoData`).
#[derive(Default)]
pub(crate) struct Globals {
    pub(crate) compositor: Option<Proxy<WlCompositor>>,
    pub(crate) shm: Option<Proxy<WlShm>>,
    /// `shell.xdg`
    pub(crate) xdg_wm_base: Option<Proxy<XdgWmBase>>,
    pub(crate) subcompositor: Option<Proxy<WlSubcompositor>>,
    pub(crate) relative_pointer_manager: Option<Proxy<ZwpRelativePointerManagerV1>>,
    pub(crate) pointer_constraints: Option<Proxy<ZwpPointerConstraintsV1>>,
    pub(crate) wp_pointer_warp_v1: Option<Proxy<WpPointerWarpV1>>,
    pub(crate) cursor_shape_manager: Option<Proxy<WpCursorShapeManagerV1>>,
    pub(crate) data_device_manager: Option<Proxy<WlDataDeviceManager>>,
    pub(crate) primary_selection_device_manager: Option<Proxy<ZwpPrimarySelectionDeviceManagerV1>>,
    pub(crate) decoration_manager: Option<Proxy<ZxdgDecorationManagerV1>>,
    pub(crate) key_inhibitor_manager: Option<Proxy<ZwpKeyboardShortcutsInhibitManagerV1>>,
    pub(crate) idle_inhibit_manager: Option<Proxy<ZwpIdleInhibitManagerV1>>,
    pub(crate) activation_manager: Option<Proxy<XdgActivationV1>>,
    pub(crate) text_input_manager: Option<Proxy<ZwpTextInputManagerV3>>,
    pub(crate) xdg_output_manager: Option<Proxy<ZxdgOutputManagerV1>>,
    pub(crate) viewporter: Option<Proxy<WpViewporter>>,
    pub(crate) fractional_scale_manager: Option<Proxy<WpFractionalScaleManagerV1>>,
    pub(crate) input_timestamps_manager: Option<Proxy<ZwpInputTimestampsManagerV1>>,
    pub(crate) zxdg_exporter_v2: Option<Proxy<ZxdgExporterV2>>,
    pub(crate) xdg_wm_dialog_v1: Option<Proxy<XdgWmDialogV1>>,
    pub(crate) wp_alpha_modifier_v1: Option<Proxy<WpAlphaModifierV1>>,
    pub(crate) xdg_toplevel_icon_manager_v1: Option<Proxy<XdgToplevelIconManagerV1>>,
    pub(crate) frog_color_management_factory_v1: Option<Proxy<FrogColorManagementFactoryV1>>,
    pub(crate) wp_color_manager_v1: Option<Proxy<WpColorManagerV1>>,
    pub(crate) tablet_manager: Option<Proxy<ZwpTabletManagerV2>>,
    pub(crate) wl_fixes: Option<Proxy<WlFixes>>,
    pub(crate) zwp_pointer_gestures: Option<Proxy<ZwpPointerGesturesV1>>,
    pub(crate) single_pixel_buffer_manager: Option<Proxy<WpSinglePixelBufferManagerV1>>,
    pub(crate) xdg_session_manager: Option<Proxy<XdgSessionManagerV1>>,
    pub(crate) xdg_toplevel_tag_manager: Option<Proxy<XdgToplevelTagManagerV1>>,
}

/// A libdecor context (`shell.libdecor`), unreferenced on drop.
pub(crate) struct LibdecorContext {
    pub(crate) raw: NonNull<libdecor>,
    syms: Arc<WaylandSyms>,
}

// SAFETY: libdecor is only used on the video thread; the pointer moves with
// the device.
unsafe impl Send for LibdecorContext {}

impl Drop for LibdecorContext {
    fn drop(&mut self) {
        if let Some(l) = &self.syms.libdecor {
            // SAFETY: the context was made by libdecor_new; its frames are
            // gone (they belong to windows, destroyed first).
            unsafe { (l.libdecor_unref)(self.raw.as_ptr()) };
        }
    }
}

/// The mutable state of the device. Translation of `struct SDL_VideoData`
/// (the display connection and the event thread live directly in
/// [`WaylandVideo`]).
pub(crate) struct VideoData {
    pub(crate) registry: Option<Proxy<WlRegistry>>,
    pub(crate) g: Globals,
    pub(crate) cursor_themes: Vec<CursorTheme>,
    /// `shell.libdecor`
    pub(crate) libdecor: Option<LibdecorContext>,

    pub(crate) xdg_session: Option<Proxy<XdgSessionV1>>,
    pub(crate) xkb_context: Option<super::events::XkbContext>,

    /// The seats, in the order they were added.
    pub(crate) seat_list: Vec<WaylandSeat>,
    /// The registry names of the seats of these (`SDL_WaylandSeat *`).
    pub(crate) last_implicit_grab_seat: Option<u32>,
    pub(crate) current_data_offer_seat: Option<u32>,
    pub(crate) current_primary_selection_seat: Option<u32>,

    pub(crate) output_list: Vec<WaylandDisplayData>,

    /// The driver data of the windows (`window->internal`).
    pub(crate) windows: Vec<WaylandWindowData>,

    /// The state of `SDL_waylandmouse.c`.
    pub(crate) mouse: MouseData,

    pub(crate) initializing: bool,
    pub(crate) display_disconnected: bool,
    pub(crate) scale_to_display_enabled: bool,

    /// Vulkan variables only valid if the Vulkan loader is loaded
    pub(crate) vulkan: super::vulkan::VulkanData,
}

impl VideoData {
    /// The seat with registry name `key`.
    pub(crate) fn seat(&self, key: u32) -> Option<&WaylandSeat> {
        self.seat_list.iter().find(|s| s.registry_id == key)
    }

    /// The seat with registry name `key`.
    pub(crate) fn seat_mut(&mut self, key: u32) -> Option<&mut WaylandSeat> {
        self.seat_list.iter_mut().find(|s| s.registry_id == key)
    }

    /// The output with registry name `key`.
    pub(crate) fn output(&self, key: u32) -> Option<&WaylandDisplayData> {
        self.output_list.iter().find(|d| d.registry_id == key)
    }

    /// The output with registry name `key`.
    pub(crate) fn output_mut(&mut self, key: u32) -> Option<&mut WaylandDisplayData> {
        self.output_list.iter_mut().find(|d| d.registry_id == key)
    }

    /// The output of a display.
    pub(crate) fn output_for_display(&self, display: DisplayID) -> Option<&WaylandDisplayData> {
        self.output_list
            .iter()
            .find(|d| d.display == display && display != 0)
    }

    /// The driver data of a window.
    pub(crate) fn window(&self, window: WindowID) -> Option<&WaylandWindowData> {
        self.windows.iter().find(|w| w.sdlwindow == window)
    }

    /// The driver data of a window.
    pub(crate) fn window_mut(&mut self, window: WindowID) -> Option<&mut WaylandWindowData> {
        self.windows.iter_mut().find(|w| w.sdlwindow == window)
    }

    /// The window a surface belongs to, if it is the content surface (or
    /// the mask) of one of ours, or an external surface given to us.
    /// Translation of `Wayland_GetWindowDataForOwnedSurface()` (which reads
    /// the surface's tag and user data; looking the surface up among ours is
    /// equivalent, and never touches a destroyed surface).
    pub(crate) fn window_for_surface(&self, surface: usize) -> Option<WindowID> {
        if surface == 0 {
            return None;
        }
        self.windows
            .iter()
            .find(|w| {
                w.surface_id() == surface
                    || w.mask
                        .surface
                        .as_ref()
                        .is_some_and(|m| m.raw() as usize == surface)
            })
            .map(|w| w.sdlwindow)
    }
}

/// The Wayland video device: `SDL_VideoDevice` with its `SDL_VideoData`.
pub(crate) struct WaylandVideo {
    /// A weak reference to this device, for the protocol listeners.
    this: Weak<WaylandVideo>,
    /// The state (`SDL_VideoData`); declared before `conn` so that it goes
    /// first.
    data: ReentrantMutex<RefCell<VideoData>>,
    /// One-shot `wl_callback`s in flight (sync points), destroyed with the
    /// device if they haven't fired.
    pub(crate) callbacks: Callbacks,
    /// `event_thread_context`
    pub(crate) event_thread: Mutex<Option<Arc<EventThreadContext>>>,
    /// The connection (`display`; `display_externally_owned` when it is the
    /// application's).
    pub(crate) conn: Arc<Conn>,
    caps: DeviceCaps,
}

impl WaylandVideo {
    /// Run `f` on the private data. Never call back into SDL, or dispatch
    /// Wayland events, from `f`.
    pub(crate) fn with_data<R>(&self, f: impl FnOnce(&mut VideoData) -> R) -> R {
        let guard = self.data.lock();
        let mut data = guard.borrow_mut();
        f(&mut data)
    }

    /// A weak reference to the device, for listeners.
    pub(crate) fn weak(&self) -> Weak<WaylandVideo> {
        self.this.clone()
    }

    /// The loaded functions.
    pub(crate) fn syms(&self) -> &Arc<WaylandSyms> {
        self.conn.syms()
    }

    /// Install a listener on `proxy` that calls `f` with the device (if it
    /// still exists).
    pub(crate) fn listen<I: super::client::Interface>(
        &self,
        proxy: &mut Proxy<I>,
        f: impl Fn(&WaylandVideo, Obj<'_, I>, I::Event<'_>) + Send + Sync + 'static,
    ) {
        let weak = self.weak();
        proxy.listen(move |obj, event| {
            if let Some(video) = weak.upgrade() {
                f(&video, obj, event);
            }
        });
    }

    /// The event thread, if there is one.
    pub(crate) fn event_thread(&self) -> Option<Arc<EventThreadContext>> {
        self.event_thread
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

/// The pointer properties are addresses, stored as numbers.
fn ptr_prop(raw: *const std::ffi::c_void) -> i64 {
    raw as usize as i64
}

/// Set (or with a null pointer, clear) a pointer property.
pub(crate) fn set_ptr_prop(props: &Properties, name: &str, raw: *const std::ffi::c_void) {
    if raw.is_null() {
        let _ = props.remove(name);
    } else {
        let _ = props.set(name, ptr_prop(raw));
    }
}

/* GNOME doesn't expose displays in any particular order, but we can find the
 * primary display and its logical coordinates via a DBus method.
 */
/// Translation of `Wayland_GetGNOMEPrimaryDisplayCoordinates()`.
fn wayland_get_gnome_primary_display_coordinates() -> Option<(i32, i32)> {
    use crate::core::linux::dbus::{self, Value};

    let ctx = dbus::context()?;
    let msg = ctx.session_conn.new_method_call(
        DISPLAY_INFO_NODE,
        DISPLAY_INFO_PATH,
        DISPLAY_INFO_NODE,
        DISPLAY_INFO_METHOD,
    )?;
    let reply = ctx.session_conn.send_with_reply_and_block(&msg, -1).ok()?;
    let args = reply.args();
    let mut iter = args.iter();

    // Serial (don't care)
    if !matches!(iter.next(), Some(Value::U32(_))) {
        return None;
    }

    // Physical monitor array (don't care)
    if !matches!(iter.next(), Some(Value::Array(_))) {
        return None;
    }

    // Logical monitor array of structs
    let Some(Value::Array(logical_monitors)) = iter.next() else {
        return None;
    };

    // First logical monitor struct
    if !matches!(logical_monitors.first(), Some(Value::Struct(_))) {
        return None;
    }

    for monitor in logical_monitors {
        let Value::Struct(fields) = monitor else {
            return None;
        };
        let mut f = fields.iter();

        // Logical X
        let Some(Value::I32(logical_x)) = f.next() else {
            return None;
        };

        // Logical Y
        let Some(Value::I32(logical_y)) = f.next() else {
            return None;
        };

        // Scale (don't care)
        if !matches!(f.next(), Some(Value::F64(_))) {
            return None;
        }

        // Transform (don't care)
        if !matches!(f.next(), Some(Value::U32(_))) {
            return None;
        }

        // Primary display boolean
        let Some(Value::Bool(primary)) = f.next() else {
            return None;
        };

        if *primary {
            // We found the primary display: success.
            return Some((*logical_x, *logical_y));
        }
    }

    None
}

/// Sort the list of displays into a deterministic order. Translation of
/// `Wayland_DisplayPositionCompare()`.
fn wayland_display_position_compare(
    da: &WaylandDisplayData,
    db: &WaylandDisplayData,
) -> std::cmp::Ordering {
    use std::cmp::Ordering;

    let a_at_origin = da.logical.x == 0 && da.logical.y == 0;
    let b_at_origin = db.logical.x == 0 && db.logical.y == 0;

    // Sort the display at 0,0 to be beginning of the list, as that will be the fallback primary.
    if a_at_origin && !b_at_origin {
        return Ordering::Less;
    }
    if b_at_origin && !a_at_origin {
        return Ordering::Greater;
    }

    if da.logical.x < db.logical.x {
        return Ordering::Less;
    }
    if da.logical.x > db.logical.x {
        return Ordering::Greater;
    }
    if da.logical.y < db.logical.y {
        return Ordering::Less;
    }
    if da.logical.y > db.logical.y {
        return Ordering::Greater;
    }

    // If no position information is available, use the connector name.
    if let (Some(a), Some(b)) = (&da.wl_output_name, &db.wl_output_name) {
        return a.as_bytes().cmp(b.as_bytes());
    }

    Ordering::Equal
}

/// Wayland doesn't have the native concept of a primary display, but there
/// are clients that will base their resolution lists on, or automatically
/// make themselves fullscreen on, the first listed output, which can lead to
/// problems if the first listed output isn't necessarily the best display for
/// this. This attempts to find a primary display, first by querying the GNOME
/// DBus property, then trying to determine the 'best' display if that fails.
/// If all displays are equal, the one at position 0,0 will become the
/// primary.
///
/// The primary is determined by the following criteria, in order:
/// - Landscape is preferred over portrait
/// - The highest native resolution
/// - A higher HDR range is preferred
/// - Higher refresh is preferred (ignoring small differences)
/// - Lower scale values are preferred (larger display)
///
/// Translation of `Wayland_GetPrimaryDisplay()`.
fn wayland_get_primary_display(vid: &VideoData, gnome_primary: Option<(i32, i32)>) -> usize {
    const REFRESH_DELTA: i32 = 4000;

    // Query the DBus interface to see if the coordinates of the primary display are exposed.
    if let Some((x, y)) = gnome_primary {
        for (i, d) in vid.output_list.iter().enumerate() {
            if d.logical.x == x && d.logical.y == y {
                return i;
            }
        }
    }

    // Otherwise, choose the 'best' display.
    let mut best_width = 0;
    let mut best_height = 0;
    let mut best_scale = 0.0;
    let mut best_headroom = 0.0f32;
    let mut best_refresh = 0;
    let mut best_is_landscape = false;
    let mut best_index = 0;

    for (i, d) in vid.output_list.iter().enumerate() {
        let is_landscape = d.orientation != DisplayOrientation::Portrait
            && d.orientation != DisplayOrientation::PortraitFlipped;
        let mut have_new_best = false;

        if !best_is_landscape && is_landscape {
            // Favor landscape over portrait displays.
            have_new_best = true;
        } else if !best_is_landscape || is_landscape {
            // Ignore portrait displays if a landscape was already found.
            if d.pixel.width > best_width || d.pixel.height > best_height {
                have_new_best = true;
            } else if d.pixel.width == best_width && d.pixel.height == best_height {
                if d.hdr.hdr_headroom > best_headroom {
                    // Favor a higher HDR luminance range
                    have_new_best = true;
                } else if d.hdr.hdr_headroom == best_headroom {
                    if d.refresh - best_refresh > REFRESH_DELTA {
                        // Favor a higher refresh rate, but ignore small differences (e.g. 59.97 vs 60.1)
                        have_new_best = true;
                    } else if d.scale_factor < best_scale
                        && (d.refresh - best_refresh).abs() <= REFRESH_DELTA
                    {
                        // Prefer a lower scale display if the difference in refresh rate is small.
                        have_new_best = true;
                    }
                }
            }
        }

        if have_new_best {
            best_width = d.pixel.width;
            best_height = d.pixel.height;
            best_scale = d.scale_factor;
            best_headroom = d.hdr.hdr_headroom;
            best_refresh = d.refresh;
            best_is_landscape = is_landscape;
            best_index = i;
        }
    }

    best_index
}

/// Translation of `Wayland_SortOutputsByPriorityHint()`.
fn wayland_sort_outputs_by_priority_hint(vid: &mut VideoData) {
    if let Some(name_hint) = hints::get(hints::VIDEO_DISPLAY_PRIORITY) {
        let mut remaining: Vec<Option<WaylandDisplayData>> =
            vid.output_list.drain(..).map(Some).collect();
        let mut sorted_list = Vec::with_capacity(remaining.len());

        // Sort the requested displays to the front of the list.
        for token in name_hint.split(',').filter(|t| !t.is_empty()) {
            for slot in remaining.iter_mut() {
                if slot
                    .as_ref()
                    .is_some_and(|d| d.wl_output_name.as_deref() == Some(token))
                {
                    sorted_list.extend(slot.take());
                    break;
                }
            }
        }

        // Append the remaining outputs to the end of the list.
        sorted_list.extend(remaining.into_iter().flatten());

        // Copy the sorted list to the output list.
        vid.output_list = sorted_list;
    }
}

/// Ensure outputs are not overlapping in the pixel coordinate space.
/// Translation of `Wayland_DeriveOutputPixelCoordinates()`.
///
/// This is a simple algorithm that offsets display positions by the
/// logical/pixel difference if they are to the right of and/or below a scaled
/// display. It can leave gaps in certain scenarios, but it works well enough
/// in most cases.
///
/// Patches for a more sophisticated algorithm are welcome.
pub(crate) fn wayland_derive_output_pixel_coordinates(vid: &mut VideoData) {
    for d in vid.output_list.iter_mut() {
        d.pixel.x = d.logical.x;
        d.pixel.y = d.logical.y;
    }

    for i in 0..vid.output_list.len() {
        let d1 = (
            vid.output_list[i].logical,
            vid.output_list[i].pixel.width,
            vid.output_list[i].pixel.height,
        );
        let (logical1, pw, ph) = d1;
        if logical1.width != pw || logical1.height != ph {
            let x_adj = pw - logical1.width;
            let y_adj = ph - logical1.height;

            // Don't adjust for scale values less than 1.0.
            if x_adj > 0 && y_adj > 0 {
                for d2 in vid.output_list.iter_mut() {
                    if d2.logical.x > logical1.x {
                        d2.pixel.x += x_adj;
                    }
                    if d2.logical.y > logical1.y {
                        d2.pixel.y += y_adj;
                    }
                }
            }
        }
    }
}

/// Translation of `Wayland_SortOutputs()`.
fn wayland_sort_outputs(vid: &mut VideoData, gnome_primary: Option<(i32, i32)>) {
    // Sort by position or connector name, so the order of outputs is deterministic.
    // (a stable sort, where upstream's SDL_qsort isn't: equal outputs keep
    // their registry order)
    vid.output_list.sort_by(wayland_display_position_compare);

    // Derive the output pixel coordinates if scale to display is enabled.
    if vid.scale_to_display_enabled {
        wayland_derive_output_pixel_coordinates(vid);
    }

    // Find a suitable primary display and move it to the front of the list.
    let primary_index = wayland_get_primary_display(vid, gnome_primary);
    if primary_index != 0 {
        let primary = vid.output_list.remove(primary_index);
        vid.output_list.insert(0, primary);
    }

    // Apply the ordering hint, if specified.
    wayland_sort_outputs_by_priority_hint(vid);
}

/// Resolution lists courtesy of XWayland. The modes of `AddEmulatedModes()`.
const EMULATED_MODE_LIST: [(i32, i32); 31] = [
    // 16:9 (1.77)
    (7680, 4320),
    (6144, 3160),
    (5120, 2880),
    (4096, 2304),
    (3840, 2160),
    (3200, 1800),
    (2880, 1620),
    (2560, 1440),
    (2048, 1152),
    (1920, 1080),
    (1600, 900),
    (1368, 768),
    (1280, 720),
    (864, 486),
    // 16:10 (1.6)
    (2560, 1600),
    (1920, 1200),
    (1680, 1050),
    (1440, 900),
    (1280, 800),
    // 3:2 (1.5)
    (720, 480),
    // 4:3 (1.33)
    (2048, 1536),
    (1920, 1440),
    (1600, 1200),
    (1440, 1080),
    (1400, 1050),
    (1280, 1024),
    (1280, 960),
    (1152, 864),
    (1024, 768),
    (800, 600),
    (640, 480),
];

/// Where the display information of an output goes: the display record
/// before it is added (`placeholder`), or the added display.
enum DisplayTarget<'a> {
    Placeholder(&'a mut VideoDisplay),
    Display(DisplayID),
}

impl DisplayTarget<'_> {
    /// Translation of `SDL_AddFullscreenDisplayMode()`.
    fn add_fullscreen_mode(&mut self, mode: &DisplayMode) {
        match self {
            DisplayTarget::Placeholder(d) => {
                add_fullscreen_display_mode_to(d, mode);
            }
            DisplayTarget::Display(id) => {
                add_fullscreen_display_mode(*id, mode);
            }
        }
    }

    /// The desktop mode (`dpy->desktop_mode`).
    fn desktop_mode(&self) -> DisplayMode {
        match self {
            DisplayTarget::Placeholder(d) => d.desktop_mode,
            DisplayTarget::Display(id) => {
                crate::video::display::desktop_display_mode(*id).unwrap_or_default()
            }
        }
    }

    /// Translation of `SDL_SetDesktopDisplayMode()`.
    fn set_desktop_mode(&mut self, mode: &DisplayMode) {
        match self {
            DisplayTarget::Placeholder(d) => {
                // (a display that isn't added yet has no fullscreen window
                // and sends no events)
                d.desktop_mode = *mode;
                d.desktop_mode.display_id = d.id;
                finalize_display_mode(&mut d.desktop_mode);
            }
            DisplayTarget::Display(id) => set_desktop_display_mode(*id, mode),
        }
    }

    /// Translation of `SDL_SetDisplayContentScale()`.
    fn set_content_scale(&mut self, scale: f32) {
        match self {
            DisplayTarget::Placeholder(d) => d.content_scale = scale,
            DisplayTarget::Display(id) => set_display_content_scale(*id, scale),
        }
    }
}

/// Translation of `AddEmulatedModes()`.
fn add_emulated_modes(dpy: &mut DisplayTarget<'_>, native_width: i32, native_height: i32) {
    let desktop_mode = dpy.desktop_mode();
    let rot_90 = native_width < native_height; // Reverse width/height for portrait displays.

    for &(w, h) in EMULATED_MODE_LIST.iter() {
        let mut mode = DisplayMode {
            format: desktop_mode.format,
            refresh_rate_numerator: desktop_mode.refresh_rate_numerator,
            refresh_rate_denominator: desktop_mode.refresh_rate_denominator,
            ..DisplayMode::default()
        };

        if rot_90 {
            mode.w = h;
            mode.h = w;
        } else {
            mode.w = w;
            mode.h = h;
        }

        // Only add modes that are smaller than the native mode.
        if (mode.w < native_width && mode.h < native_height)
            || (mode.w < native_width && mode.h == native_height)
            || (mode.w == native_width && mode.h < native_height)
        {
            dpy.add_fullscreen_mode(&mode);
        }
    }
}

/// The values `handle_wl_output_done()` computes from an output.
struct OutputModes {
    native_mode: DisplayMode,
    desktop_mode: DisplayMode,
    scale_factor: f64,
    logical: OutputRect,
    has_viewporter: bool,
    scale_to_display_enabled: bool,
    mode_emulation_enabled: bool,
}

/// Fill in a display from the output's modes (the middle of
/// `handle_wl_output_done()`).
fn apply_output_modes(dpy: &mut DisplayTarget<'_>, m: &OutputModes) {
    let native_mode = m.native_mode;
    let mut desktop_mode = m.desktop_mode;

    if m.scale_to_display_enabled {
        dpy.set_content_scale(m.scale_factor as f32);
    }

    // Set the desktop display mode.
    dpy.set_desktop_mode(&desktop_mode);

    // Expose the unscaled, native resolution if the scale is 1.0 or viewports are available...
    if m.scale_factor == 1.0 || m.has_viewporter {
        dpy.add_fullscreen_mode(&native_mode);
        if native_mode.w != desktop_mode.w || native_mode.h != desktop_mode.h {
            dpy.add_fullscreen_mode(&desktop_mode);
        }
    } else {
        // ...otherwise expose the integer scaled variants of the desktop resolution down to 1.
        desktop_mode.pixel_density = 1.0;

        let mut i = m.scale_factor as i32;
        while i > 0 {
            desktop_mode.w = m.logical.width * i;
            desktop_mode.h = m.logical.height * i;
            dpy.add_fullscreen_mode(&desktop_mode);
            i -= 1;
        }
    }

    // Add emulated modes if wp_viewporter is supported and mode emulation is enabled.
    if m.has_viewporter && m.mode_emulation_enabled {
        // The transformed display pixel width/height must be used here.
        add_emulated_modes(dpy, native_mode.w, native_mode.h);
    }
}

/// What `handle_wl_output_done()` does after computing the modes.
enum OutputDone {
    /// Still waiting for events.
    Pending,
    /// The first time: the display is to be added (unless initializing).
    New { add_now: bool },
    /// An existing display was updated.
    Existing {
        display: DisplayID,
        modes: OutputModes,
        orientation: DisplayOrientation,
        geometry_changed: bool,
    },
}

/// The display orientation for an output transform (the `TF_CASE` table of
/// `handle_wl_output_geometry()`).
fn orientation_for_transform(
    transform: WlOutputTransform,
    landscape_natural: bool,
) -> Option<DisplayOrientation> {
    use DisplayOrientation as O;
    let t = transform;
    let o = if landscape_natural {
        match t {
            WlOutputTransform::NORMAL => O::Landscape,
            WlOutputTransform::_90 => O::Portrait,
            WlOutputTransform::_180 => O::LandscapeFlipped,
            WlOutputTransform::_270 => O::PortraitFlipped,
            WlOutputTransform::FLIPPED => O::LandscapeFlipped,
            WlOutputTransform::FLIPPED_90 => O::PortraitFlipped,
            WlOutputTransform::FLIPPED_180 => O::Landscape,
            WlOutputTransform::FLIPPED_270 => O::Portrait,
            _ => return None,
        }
    } else {
        match t {
            WlOutputTransform::NORMAL => O::Portrait,
            WlOutputTransform::_90 => O::Landscape,
            WlOutputTransform::_180 => O::PortraitFlipped,
            WlOutputTransform::_270 => O::LandscapeFlipped,
            WlOutputTransform::FLIPPED => O::PortraitFlipped,
            WlOutputTransform::FLIPPED_90 => O::LandscapeFlipped,
            WlOutputTransform::FLIPPED_180 => O::Portrait,
            WlOutputTransform::FLIPPED_270 => O::Landscape,
            _ => return None,
        }
    };
    Some(o)
}

impl WaylandVideo {
    /// Translation of `Wayland_RefreshWindowPositions()`.
    fn wayland_refresh_window_positions(&self) {
        for w in crate::video::core::window_ids() {
            self.wayland_update_window_position(w);
        }
    }

    /// The output listener (`output_listener`).
    fn handle_wl_output_event(&self, key: u32, event: WlOutputEvent<'_>) {
        match event {
            WlOutputEvent::Geometry {
                x,
                y,
                physical_width,
                physical_height,
                model,
                transform,
                ..
            } => self.with_data(|d| {
                let Some(internal) = d.output_mut(key) else {
                    return;
                };

                // Apply the change from wl-output only if xdg-output is not supported
                if !internal.has_logical_position {
                    internal.geometry_changed |= internal.logical.x != x || internal.logical.y != y;
                    internal.logical.x = x;
                    internal.logical.y = y;
                }
                internal.physical_width_mm = physical_width;
                internal.physical_height_mm = physical_height;

                // The model is only used for the output name if wl_output or xdg-output haven't provided a description.
                if internal.display == 0 && internal.placeholder.name.is_none() {
                    internal.placeholder.name = Some(model.to_string_lossy().into_owned());
                }

                internal.transform = transform;
                if let Some(o) = orientation_for_transform(
                    transform,
                    internal.physical_width_mm >= internal.physical_height_mm,
                ) {
                    internal.orientation = o;
                }
            }),
            WlOutputEvent::Mode {
                flags,
                width,
                height,
                refresh,
            } => self.with_data(|d| {
                let Some(internal) = d.output_mut(key) else {
                    return;
                };
                if flags.contains(WlOutputMode::CURRENT) {
                    internal.geometry_changed |=
                        internal.pixel.width != width || internal.pixel.height != height;
                    internal.pixel.width = width;
                    internal.pixel.height = height;

                    /*
                     * Don't rotate this yet, wl-output coordinates are transformed in
                     * handle_done and xdg-output coordinates are pre-transformed.
                     */
                    if !internal.has_logical_size {
                        internal.geometry_changed |=
                            internal.logical.width != width || internal.logical.height != height;
                        internal.logical.width = width;
                        internal.logical.height = height;
                    }

                    internal.refresh = refresh;
                }
            }),
            WlOutputEvent::Done => self.handle_wl_output_done(key),
            WlOutputEvent::Scale { factor } => self.with_data(|d| {
                if let Some(internal) = d.output_mut(key) {
                    internal.integer_scale_factor = factor;
                }
            }),
            WlOutputEvent::Name { name } => self.with_data(|d| {
                if let Some(internal) = d.output_mut(key) {
                    internal.wl_output_name = Some(name.to_string_lossy().into_owned());
                }
            }),
            WlOutputEvent::Description { description } => self.with_data(|d| {
                if let Some(internal) = d.output_mut(key) {
                    if internal.display == 0 {
                        // The description, if available, supersedes the model name.
                        internal.placeholder.name =
                            Some(description.to_string_lossy().into_owned());
                    }
                }
            }),
        }
    }

    /// The xdg-output listener (`xdg_output_listener`).
    fn handle_xdg_output_event(&self, key: u32, event: ZxdgOutputV1Event<'_>) {
        match event {
            ZxdgOutputV1Event::LogicalPosition { x, y } => self.with_data(|d| {
                if let Some(internal) = d.output_mut(key) {
                    internal.geometry_changed |= internal.logical.x != x || internal.logical.y != y;
                    internal.logical.x = x;
                    internal.logical.y = y;
                    internal.has_logical_position = true;
                }
            }),
            ZxdgOutputV1Event::LogicalSize { width, height } => self.with_data(|d| {
                if let Some(internal) = d.output_mut(key) {
                    internal.geometry_changed |=
                        internal.logical.width != width || internal.logical.height != height;
                    internal.logical.width = width;
                    internal.logical.height = height;
                    internal.has_logical_size = true;
                }
            }),
            ZxdgOutputV1Event::Done => {
                /*
                 * xdg-output.done events are deprecated and only apply below version 3 of the protocol.
                 * A wl-output.done event will be emitted in version 3 or higher.
                 */
                let old = self.with_data(|d| {
                    d.output(key)
                        .and_then(|o| o.xdg_output.as_ref())
                        .is_some_and(|x| x.version() < 3)
                });
                if old {
                    self.handle_wl_output_done(key);
                }
            }
            ZxdgOutputV1Event::Name { name } => self.with_data(|d| {
                if let Some(internal) = d.output_mut(key) {
                    // Deprecated as of wl_output v4.
                    if internal.output.version() < WlOutput::NAME_SINCE_VERSION
                        && internal.display == 0
                    {
                        internal.wl_output_name = Some(name.to_string_lossy().into_owned());
                    }
                }
            }),
            ZxdgOutputV1Event::Description { description } => self.with_data(|d| {
                if let Some(internal) = d.output_mut(key) {
                    // Deprecated as of wl_output v4.
                    if internal.output.version() < WlOutput::DESCRIPTION_SINCE_VERSION
                        && internal.display == 0
                    {
                        // xdg-output descriptions, if available, supersede wl-output model names.
                        internal.placeholder.name =
                            Some(description.to_string_lossy().into_owned());
                    }
                }
            }),
        }
    }

    /// Translation of `handle_wl_output_done()`.
    fn handle_wl_output_done(&self, key: u32) {
        let mode_emulation_enabled = hints::get_bool(hints::VIDEO_WAYLAND_MODE_EMULATION, true);

        let step = self.with_data(|video| {
            let has_viewporter = video.g.viewporter.is_some();
            let scale_to_display_enabled = video.scale_to_display_enabled;
            let initializing = video.initializing;
            let Some(internal) = video.output_mut(key) else {
                return OutputDone::Pending;
            };

            /*
             * When using xdg-output, two wl-output.done events will be emitted:
             * one at the completion of wl-display and one at the completion of xdg-output.
             *
             * All required events must be received before proceeding.
             */
            let event_await_count = 1 + internal.xdg_output.is_some() as i32;

            internal.wl_output_done_count =
                (internal.wl_output_done_count + 1).min(event_await_count + 1);

            if internal.wl_output_done_count < event_await_count {
                return OutputDone::Pending;
            }

            // (if the display was already created, its mode list is reset
            // below, with nothing borrowed)

            // The native display resolution
            let mut native_mode = DisplayMode {
                format: PixelFormat::XRGB8888,
                ..DisplayMode::default()
            };

            // Transform the pixel values, if necessary.
            if internal.transform.0 & WlOutputTransform::_90.0 != 0 {
                native_mode.w = internal.pixel.height;
                native_mode.h = internal.pixel.width;
            } else {
                native_mode.w = internal.pixel.width;
                native_mode.h = internal.pixel.height;
            }
            native_mode.refresh_rate_numerator = internal.refresh;
            native_mode.refresh_rate_denominator = 1000;

            if internal.has_logical_size {
                // If xdg-output is present...
                if native_mode.w != internal.logical.width
                    || native_mode.h != internal.logical.height
                {
                    // ...and the compositor scales the logical viewport...
                    if has_viewporter {
                        // ...and viewports are supported, calculate the true scale of the output.
                        internal.scale_factor =
                            native_mode.w as f64 / internal.logical.width as f64;
                    } else {
                        // ...otherwise, the 'native' pixel values are a multiple of the logical screen size.
                        internal.scale_factor = internal.integer_scale_factor as f64;
                        internal.pixel.width =
                            internal.logical.width * internal.integer_scale_factor;
                        internal.pixel.height =
                            internal.logical.height * internal.integer_scale_factor;
                    }
                } else {
                    /* ...and the output viewport is not scaled in the global compositing
                     * space, the output dimensions need to be divided by the scale factor.
                     *
                     * NOTE: This path is needed for old versions of GNOME that predate fractional scaling.
                     */
                    internal.scale_factor = internal.integer_scale_factor as f64;
                    // FIXME (upstream): a compositor sending a wl_output
                    // scale of 0 divides by zero here (and below); Rust
                    // panics on that, so the division is skipped for 0.
                    if internal.integer_scale_factor != 0 {
                        internal.logical.width /= internal.integer_scale_factor;
                        internal.logical.height /= internal.integer_scale_factor;
                    }
                }
            } else {
                /* Calculate the points from the pixel values, if xdg-output isn't present.
                 * Use the native mode pixel values since they are pre-transformed.
                 */
                internal.scale_factor = internal.integer_scale_factor as f64;
                if internal.integer_scale_factor != 0 {
                    internal.logical.width = native_mode.w / internal.integer_scale_factor;
                    internal.logical.height = native_mode.h / internal.integer_scale_factor;
                }
            }

            // The scaled desktop mode
            let mut desktop_mode = DisplayMode {
                format: PixelFormat::XRGB8888,
                ..DisplayMode::default()
            };

            if !scale_to_display_enabled {
                desktop_mode.w = internal.logical.width;
                desktop_mode.h = internal.logical.height;
                desktop_mode.pixel_density = internal.scale_factor as f32;
            } else {
                desktop_mode.w = native_mode.w;
                desktop_mode.h = native_mode.h;
                desktop_mode.pixel_density = 1.0;
            }

            desktop_mode.refresh_rate_numerator = internal.refresh;
            desktop_mode.refresh_rate_denominator = 1000;

            let modes = OutputModes {
                native_mode,
                desktop_mode,
                scale_factor: internal.scale_factor,
                logical: internal.logical,
                has_viewporter,
                scale_to_display_enabled,
                mode_emulation_enabled,
            };

            if internal.display == 0 {
                apply_output_modes(
                    &mut DisplayTarget::Placeholder(&mut internal.placeholder),
                    &modes,
                );

                // First time getting display info, initialize the VideoDisplay
                if internal.physical_width_mm >= internal.physical_height_mm {
                    internal.placeholder.natural_orientation = DisplayOrientation::Landscape;
                } else {
                    internal.placeholder.natural_orientation = DisplayOrientation::Portrait;
                }
                internal.placeholder.current_orientation = internal.orientation;

                let props = Properties::new();
                set_ptr_prop(
                    &props,
                    PROP_DISPLAY_WAYLAND_WL_OUTPUT_POINTER,
                    internal.output.raw().cast(),
                );
                internal.placeholder.props = Some(props);

                // During initialization, the displays will be added after enumeration is complete.
                OutputDone::New {
                    add_now: !initializing,
                }
            } else {
                OutputDone::Existing {
                    display: internal.display,
                    modes,
                    orientation: internal.orientation,
                    geometry_changed: internal.geometry_changed,
                }
            }
        });

        match step {
            OutputDone::Pending => return,
            OutputDone::New { add_now: false } => {}
            OutputDone::New { add_now: true } => {
                let (color, scale_to_display) = self.with_data(|d| {
                    (
                        d.g.wp_color_manager_v1.is_some(),
                        d.scale_to_display_enabled,
                    )
                });
                if color {
                    self.wayland_get_color_info_for_output(key, false);
                }
                let placeholder = self.with_data(|d| {
                    if scale_to_display {
                        wayland_derive_output_pixel_coordinates(d);
                    }
                    d.output_mut(key)
                        .map(|o| std::mem::replace(&mut o.placeholder, VideoDisplay::new()))
                });
                if let Some(placeholder) = placeholder {
                    let id = add_video_display(placeholder, true);
                    self.with_data(|d| {
                        if let Some(o) = d.output_mut(key) {
                            o.display = id;
                        }
                    });
                    self.wayland_refresh_window_positions();
                }
            }
            OutputDone::Existing {
                display,
                modes,
                orientation,
                geometry_changed,
            } => {
                // If the display was already created, reset and rebuild the mode list.
                reset_fullscreen_display_modes(display);
                apply_output_modes(&mut DisplayTarget::Display(display), &modes);

                send_display_event(
                    display,
                    EventType::DISPLAY_ORIENTATION,
                    orientation as i32,
                    0,
                );

                if geometry_changed {
                    self.with_data(|d| {
                        if d.scale_to_display_enabled {
                            wayland_derive_output_pixel_coordinates(d);
                        }
                    });

                    send_display_event(display, EventType::DISPLAY_MOVED, 0, 0);
                    self.wayland_refresh_window_positions();
                }
            }
        }

        self.with_data(|d| {
            if let Some(internal) = d.output_mut(key) {
                internal.geometry_changed = false;
            }
        });
    }

    /// Translation of `Wayland_add_display()`.
    fn wayland_add_display(&self, d: &mut VideoData, id: u32, version: u32) -> Result<()> {
        let Some(registry) = d.registry.as_ref() else {
            return Err(Error::new("Failed to retrieve output."));
        };
        let mut output = registry.bind::<WlOutput>(id, version);
        output.obj().set_user_data(id as usize);
        self.listen(&mut output, move |v, _, ev| {
            v.handle_wl_output_event(id, ev)
        });
        output.obj().set_tag(&SDL_WAYLAND_OUTPUT_TAG);

        let mut data = WaylandDisplayData {
            output,
            xdg_output: None,
            wp_color_management_output: None,
            color_info_state: None,
            wl_output_name: None,
            scale_factor: 1.0,
            integer_scale_factor: 1,
            registry_id: id,
            logical: OutputRect::default(),
            pixel: OutputRect::default(),
            physical_width_mm: 0,
            physical_height_mm: 0,
            refresh: 0,
            transform: WlOutputTransform::NORMAL,
            orientation: DisplayOrientation::Unknown,
            hdr: HdrOutputProperties::default(),
            display: 0,
            placeholder: VideoDisplay::new(),
            wl_output_done_count: 0,
            has_logical_position: false,
            has_logical_size: false,
            geometry_changed: false,
        };

        if let Some(xdg_output_manager) = &d.g.xdg_output_manager {
            let mut xdg_output = xdg_output_manager.get_xdg_output(data.output.obj());
            self.listen(&mut xdg_output, move |v, _, ev| {
                v.handle_xdg_output_event(id, ev)
            });
            data.xdg_output = Some(xdg_output);
        }
        let color = if let Some(cm) = &d.g.wp_color_manager_v1 {
            let mut cmo = cm.get_output(data.output.obj());
            self.listen(&mut cmo, move |v, _, _ev| {
                v.wayland_get_color_info_for_output(id, false)
            });
            data.wp_color_management_output = Some(cmo);
            true
        } else {
            false
        };

        // Keep a list of outputs for sorting and deferred protocol initialization.
        d.output_list.push(data);

        // If not initializing, this will be queried synchronously in wl_output.done.
        if color && d.initializing {
            self.wayland_get_color_info_for_output_locked(d, id, true);
        }
        Ok(())
    }

    /// Release an output's objects (the first part of
    /// `Wayland_free_display()`).
    fn wayland_free_display_data(display_data: WaylandDisplayData) {
        let WaylandDisplayData {
            output,
            xdg_output,
            wp_color_management_output,
            color_info_state,
            ..
        } = display_data;
        if wp_color_management_output.is_some() {
            drop(color_info_state);
            drop(wp_color_management_output);
        }

        drop(xdg_output);

        // (dropping the output releases it, with wl_output.release from
        // version 3, else just destroys the proxy)
        drop(output);
    }

    /// Translation of `Wayland_free_display()`: the output with registry
    /// name `key` is removed from the output list and freed.
    fn wayland_free_display(&self, key: u32, send_event: bool) {
        let Some((display_data, display)) = self.with_data(|d| {
            let i = d.output_list.iter().position(|o| o.registry_id == key)?;
            let o = d.output_list.remove(i);
            let display = o.display;
            Some((o, display))
        }) else {
            return;
        };

        /* A preceding surface leave event is not guaranteed when an output is removed,
         * so ensure that no window continues to hold a reference to a removed output.
         */
        for window in crate::video::core::window_ids() {
            self.wayland_remove_output_from_window(window, key);
        }

        Self::wayland_free_display_data(display_data);

        if display != 0 {
            del_video_display(display, send_event);
        }
    }

    /// Translation of `Wayland_FinalizeDisplays()`.
    fn wayland_finalize_displays(&self) {
        let gnome_primary = wayland_get_gnome_primary_display_coordinates();
        let placeholders = self.with_data(|vid| {
            wayland_sort_outputs(vid, gnome_primary);
            vid.output_list
                .iter_mut()
                .map(|o| {
                    (
                        o.registry_id,
                        std::mem::replace(&mut o.placeholder, VideoDisplay::new()),
                    )
                })
                .collect::<Vec<_>>()
        });

        for (key, placeholder) in placeholders {
            let id = add_video_display(placeholder, false);
            self.with_data(|d| {
                if let Some(o) = d.output_mut(key) {
                    o.display = id;
                }
            });
        }
    }

    /// Translation of `Wayland_init_xdg_output()`.
    fn wayland_init_xdg_output(&self, d: &mut VideoData) {
        let Some(manager) = &d.g.xdg_output_manager else {
            return;
        };
        for disp in d.output_list.iter_mut() {
            let key = disp.registry_id;
            let mut xdg_output = manager.get_xdg_output(disp.output.obj());
            self.listen(&mut xdg_output, move |v, _, ev| {
                v.handle_xdg_output_event(key, ev)
            });
            disp.xdg_output = Some(xdg_output);
        }
    }

    /// Translation of `Wayland_InitColorManager()`.
    fn wayland_init_color_manager(&self, d: &mut VideoData) {
        let Some(cm) = &d.g.wp_color_manager_v1 else {
            return;
        };
        let mut keys = Vec::new();
        for disp in d.output_list.iter_mut() {
            let key = disp.registry_id;
            let mut cmo = cm.get_output(disp.output.obj());
            self.listen(&mut cmo, move |v, _, _ev| {
                v.wayland_get_color_info_for_output(key, false)
            });
            disp.wp_color_management_output = Some(cmo);
            keys.push(key);
        }
        for key in keys {
            self.wayland_get_color_info_for_output_locked(d, key, true);
        }
    }

    /// The xdg-session listener (`xdg_session_listener`).
    fn handle_xdg_session_event(&self, event: XdgSessionV1Event<'_>) {
        match event {
            XdgSessionV1Event::Created { session_id: id } => {
                let _ = Properties::global().set(
                    PROP_GLOBAL_VIDEO_WAYLAND_SESSION_ID_STRING,
                    id.to_string_lossy().into_owned(),
                );
            }
            XdgSessionV1Event::Restored => {
                // NOP
            }
            XdgSessionV1Event::Replaced => {
                // Clean up all session objects, as they have become inert, and should be destroyed.
                let _ = Properties::global().remove(PROP_GLOBAL_VIDEO_WAYLAND_SESSION_ID_STRING);

                self.with_data(|d| {
                    for w in d.windows.iter_mut() {
                        if w.xdg_toplevel_session.take().is_some() {
                            w.session_id = None;
                        }
                    }

                    d.xdg_session = None;
                });
            }
        }
    }

    /// Translation of `Wayland_CreateSession()`.
    pub(crate) fn wayland_create_session(&self, viddata: &mut VideoData) {
        let Some(manager) = &viddata.g.xdg_session_manager else {
            // Set the ID string to null if session management is not available.
            let _ = Properties::global().remove(PROP_GLOBAL_VIDEO_WAYLAND_SESSION_ID_STRING);
            return;
        };

        // Register a new session, if one does not yet exist.
        if viddata.xdg_session.is_none() {
            let session_id =
                Properties::global().get_string(PROP_GLOBAL_VIDEO_WAYLAND_SESSION_ID_STRING);
            if let Some(session_id) = session_id {
                // Create a new session if the ID string is empty.
                let session_id = Some(session_id).filter(|s| !s.is_empty());

                let reason = if session_id.is_some() {
                    XdgSessionManagerV1Reason::SESSION_RESTORE
                } else {
                    XdgSessionManagerV1Reason::LAUNCH
                };
                let mut session = manager.get_session(reason, session_id.as_deref());
                self.listen(&mut session, |v, _, ev| v.handle_xdg_session_event(ev));
                viddata.xdg_session = Some(session);
            }
        }
    }

    /// Translation of `Wayland_SessionDestroy()`.
    fn wayland_session_destroy(&self) {
        // If the session string was cleared, remove the session.
        let Some(session) = self.with_data(|d| d.xdg_session.take()) else {
            return;
        };
        let session_id =
            Properties::global().get_string(PROP_GLOBAL_VIDEO_WAYLAND_SESSION_ID_STRING);
        if session_id.is_none_or_empty() {
            session.remove();

            let _ = self.conn.roundtrip();
        } else {
            drop(session);
        }
    }

    /// The registry listener (`registry_listener`).
    fn handle_registry_event(&self, registry: Obj<'_, WlRegistry>, event: WlRegistryEvent<'_>) {
        match event {
            WlRegistryEvent::Global {
                name,
                interface,
                version,
            } => {
                let interface = interface.to_bytes();
                self.handle_registry_global(registry, name, interface, version);
            }
            WlRegistryEvent::GlobalRemove { name } => self.handle_registry_remove_global(name),
        }
    }

    /// Translation of `handle_registry_global()`.
    fn handle_registry_global(
        &self,
        registry: Obj<'_, WlRegistry>,
        id: u32,
        interface: &[u8],
        version: u32,
    ) {
        let is = |iface: &wl_interface| {
            // SAFETY: interface names are string literals.
            interface == unsafe { std::ffi::CStr::from_ptr(iface.name) }.to_bytes()
        };

        self.with_data(|d| {
            if is(&WL_COMPOSITOR_INTERFACE) {
                d.g.compositor = Some(registry.bind(id, SDL_WL_COMPOSITOR_VERSION.min(version)));
            } else if is(&WL_SUBCOMPOSITOR_INTERFACE) {
                d.g.subcompositor = Some(registry.bind(id, 1));
            } else if is(&WL_OUTPUT_INTERFACE) {
                let _ = self.wayland_add_display(d, id, version.min(SDL_WL_OUTPUT_VERSION));
            } else if is(&WL_SEAT_INTERFACE) {
                let seat = registry.bind::<WlSeat>(id, SDL_WL_SEAT_VERSION.min(version));
                self.wayland_display_create_seat(d, seat, id);
            } else if is(&XDG_WM_BASE_INTERFACE) {
                let mut xdg = registry.bind::<XdgWmBase>(id, version.min(7));
                xdg.listen(|xdg, ev| {
                    // Translation of `handle_xdg_wm_base_ping()`.
                    let XdgWmBaseEvent::Ping { serial } = ev;
                    xdg.pong(serial);
                });
                d.g.xdg_wm_base = Some(xdg);
            } else if is(&WL_SHM_INTERFACE) {
                d.g.shm = Some(registry.bind(id, SDL_WL_SHM_VERSION.min(version)));
            } else if is(&ZWP_RELATIVE_POINTER_MANAGER_V1_INTERFACE) {
                d.g.relative_pointer_manager = Some(registry.bind(id, 1));
            } else if is(&ZWP_POINTER_CONSTRAINTS_V1_INTERFACE) {
                d.g.pointer_constraints = Some(registry.bind(id, 1));
            } else if is(&ZWP_KEYBOARD_SHORTCUTS_INHIBIT_MANAGER_V1_INTERFACE) {
                d.g.key_inhibitor_manager = Some(registry.bind(id, 1));
            } else if is(&ZWP_IDLE_INHIBIT_MANAGER_V1_INTERFACE) {
                d.g.idle_inhibit_manager = Some(registry.bind(id, 1));
            } else if is(&XDG_ACTIVATION_V1_INTERFACE) {
                d.g.activation_manager = Some(registry.bind(id, 1));
            } else if is(&ZWP_TEXT_INPUT_MANAGER_V3_INTERFACE) {
                d.g.text_input_manager = Some(registry.bind(id, 1));
                self.wayland_display_init_text_input_manager(d, id);
            } else if is(&WL_DATA_DEVICE_MANAGER_INTERFACE) {
                d.g.data_device_manager = Some(registry.bind(id, 3.min(version)));
                self.wayland_display_init_data_device_manager(d);
            } else if is(&ZWP_PRIMARY_SELECTION_DEVICE_MANAGER_V1_INTERFACE) {
                d.g.primary_selection_device_manager = Some(registry.bind(id, 1));
                self.wayland_display_init_primary_selection_device_manager(d);
            } else if is(&ZXDG_DECORATION_MANAGER_V1_INTERFACE) {
                d.g.decoration_manager = Some(registry.bind(id, 2.min(version)));
            } else if is(&ZWP_TABLET_MANAGER_V2_INTERFACE) {
                d.g.tablet_manager = Some(registry.bind(id, 1));
                self.wayland_display_init_tablet_manager(d);
            } else if is(&ZXDG_OUTPUT_MANAGER_V1_INTERFACE) {
                let version = version.min(3); // Versions 1 through 3 are supported.
                d.g.xdg_output_manager = Some(registry.bind(id, version));
                self.wayland_init_xdg_output(d);
            } else if is(&WP_VIEWPORTER_INTERFACE) {
                d.g.viewporter = Some(registry.bind(id, 1));
            } else if is(&WP_FRACTIONAL_SCALE_MANAGER_V1_INTERFACE) {
                d.g.fractional_scale_manager = Some(registry.bind(id, 1));
            } else if is(&ZWP_INPUT_TIMESTAMPS_MANAGER_V1_INTERFACE) {
                d.g.input_timestamps_manager = Some(registry.bind(id, 1));
                self.wayland_display_init_input_timestamp_manager(d);
            } else if is(&WP_CURSOR_SHAPE_MANAGER_V1_INTERFACE) {
                d.g.cursor_shape_manager = Some(registry.bind(id, version.min(2)));
                self.wayland_display_init_cursor_shape_manager(d);
            } else if is(&ZXDG_EXPORTER_V2_INTERFACE) {
                d.g.zxdg_exporter_v2 = Some(registry.bind(id, 1));
            } else if is(&XDG_WM_DIALOG_V1_INTERFACE) {
                d.g.xdg_wm_dialog_v1 = Some(registry.bind(id, 1));
            } else if is(&WP_ALPHA_MODIFIER_V1_INTERFACE) {
                d.g.wp_alpha_modifier_v1 = Some(registry.bind(id, 1));
            } else if is(&XDG_TOPLEVEL_ICON_MANAGER_V1_INTERFACE) {
                d.g.xdg_toplevel_icon_manager_v1 = Some(registry.bind(id, 1));
            } else if is(&FROG_COLOR_MANAGEMENT_FACTORY_V1_INTERFACE) {
                d.g.frog_color_management_factory_v1 = Some(registry.bind(id, 1));
            } else if is(&WP_COLOR_MANAGER_V1_INTERFACE) {
                d.g.wp_color_manager_v1 = Some(registry.bind(id, version.min(2)));
                self.wayland_init_color_manager(d);
            } else if is(&WP_POINTER_WARP_V1_INTERFACE) {
                d.g.wp_pointer_warp_v1 = Some(registry.bind(id, 1));
            } else if is(&ZWP_POINTER_GESTURES_V1_INTERFACE) {
                d.g.zwp_pointer_gestures = Some(registry.bind(id, version.min(3)));
                self.wayland_display_init_pointer_gesture_manager(d);
            } else if is(&WP_SINGLE_PIXEL_BUFFER_MANAGER_V1_INTERFACE) {
                d.g.single_pixel_buffer_manager = Some(registry.bind(id, 1));
            } else if is(&XDG_SESSION_MANAGER_V1_INTERFACE) {
                d.g.xdg_session_manager = Some(registry.bind(id, 1));
            } else if is(&XDG_TOPLEVEL_TAG_MANAGER_V1_INTERFACE) {
                d.g.xdg_toplevel_tag_manager = Some(registry.bind(id, 1));
            } else if is(&WL_FIXES_INTERFACE) {
                d.g.wl_fixes = Some(registry.bind(id, SDL_WL_FIXES_VERSION.min(version)));
            }
        });
    }

    /// Translation of `handle_registry_remove_global()`.
    fn handle_registry_remove_global(&self, id: u32) {
        // We don't get an interface, just an ID, so check outputs and seats.
        let (is_output, is_seat) = self.with_data(|d| {
            (
                d.output_list.iter().any(|o| o.registry_id == id),
                d.seat_list.iter().any(|s| s.registry_id == id),
            )
        });
        if is_output {
            self.wayland_free_display(id, true);
        } else if is_seat {
            self.wayland_seat_destroy(id, false);
        }

        // ack_remove:
        self.with_data(|d| {
            if let (Some(fixes), Some(registry)) = (&d.g.wl_fixes, &d.registry) {
                if fixes.version() >= WlFixes::ACK_GLOBAL_REMOVE_SINCE_VERSION {
                    fixes.ack_global_remove(registry.obj(), id);
                }
            }
        });
    }

    /// Translation of `should_use_libdecor()`.
    fn should_use_libdecor(&self, ignore_xdg: bool) -> bool {
        if self.syms().libdecor.is_none() {
            return false;
        }

        if hints::get_bool(hints::VIDEO_WAYLAND_PREFER_LIBDECOR, false) {
            return true;
        }

        if ignore_xdg {
            return true;
        }

        if self.with_data(|d| d.g.decoration_manager.is_some()) {
            return false;
        }

        true
    }

    /// Translation of `LibdecorNew()`.
    fn libdecor_new(&self) -> Option<LibdecorContext> {
        /// The `libdecor_interface`.
        static LIBDECOR_INTERFACE: libdecor_interface = libdecor_interface {
            error: Some(libdecor_error),
            reserved: [None; 10],
        };

        let l = self.syms().libdecor.as_ref()?;
        // SAFETY: the display is connected; the interface is a static.
        let raw = unsafe { (l.libdecor_new)(self.conn.raw(), &LIBDECOR_INTERFACE) };
        super::client::resume_pending_panic();
        NonNull::new(raw).map(|raw| LibdecorContext {
            raw,
            syms: self.syms().clone(),
        })
    }

    /// Translation of `Wayland_LoadLibdecor()`.
    pub(crate) fn wayland_load_libdecor(&self, ignore_xdg: bool) -> bool {
        if self.with_data(|d| d.libdecor.is_some()) {
            return true; // Already loaded!
        }
        if self.should_use_libdecor(ignore_xdg) {
            let context = if can_use_gtk() {
                self.libdecor_new()
            } else {
                // Intentionally initialize libdecor in a non-main thread
                // so that it will not use its GTK plugin, but instead will
                // fall back to the Cairo or dummy plugin
                let this = self.weak();
                std::thread::Builder::new()
                    .name("SDL_LibdecorNew".into())
                    .spawn(move || this.upgrade().and_then(|v| v.libdecor_new()))
                    .ok()
                    .and_then(|t| t.join().ok())
                    .flatten()
            };
            let loaded = context.is_some();
            self.with_data(|d| d.libdecor = context);
            return loaded;
        }
        false
    }

    /// Translation of `Wayland_VideoInit()`.
    fn wayland_video_init(&self) -> Result<()> {
        let context = EventThreadContext::create(&self.conn, c"SDL Event Thread Queue");
        if context.is_none() {
            crate::error!(
                crate::log::Category::Video,
                "wayland: Failed to create event thread context"
            );
        }
        *self.event_thread.lock().unwrap_or_else(|e| e.into_inner()) = context;

        let Some(xkb_context) = super::events::XkbContext::new(self.syms()) else {
            return Err(Error::new("Failed to create XKB context"));
        };
        self.with_data(|d| d.xkb_context = Some(xkb_context));

        let mut registry = self.conn.obj().get_registry();
        self.listen(&mut registry, |v, registry, ev| {
            v.handle_registry_event(registry, ev)
        });
        self.with_data(|d| d.registry = Some(registry));

        // First roundtrip to receive all registry objects.
        let _ = self.conn.roundtrip();

        // Require viewports and xdg-output for display scaling.
        self.with_data(|data| {
            if data.scale_to_display_enabled {
                if data.g.viewporter.is_none() {
                    crate::error!(
                        crate::log::Category::Video,
                        "wayland: Display scaling requires the missing 'wp_viewporter' protocol: disabling"
                    );
                    data.scale_to_display_enabled = false;
                }
                if data.g.xdg_output_manager.is_none() {
                    crate::error!(
                        crate::log::Category::Video,
                        "wayland: Display scaling requires the missing 'zxdg_output_manager_v1' protocol: disabling"
                    );
                    data.scale_to_display_enabled = false;
                }
            }
        });

        // Now that we have all the protocols, load libdecor if applicable
        self.wayland_load_libdecor(false);

        // Second roundtrip to receive all output events.
        let _ = self.conn.roundtrip();

        self.wayland_finalize_displays();

        self.wayland_init_mouse();
        self.wayland_init_keyboard();

        // (the primary selection entry points check for the manager)

        self.with_data(|d| d.initializing = false);

        Ok(())
    }

    /// Translation of `Wayland_GetDisplayBounds()`.
    fn wayland_get_display_bounds(&self, display: DisplayID) -> Result<Rect> {
        let Some((scale_to_display_enabled, logical, pixel, transform)) = self.with_data(|d| {
            let internal = d.output_for_display(display)?;
            Some((
                d.scale_to_display_enabled,
                internal.logical,
                internal.pixel,
                internal.transform,
            ))
        }) else {
            return Err(Error::new("Invalid display"));
        };

        let mut rect = Rect::default();
        if !scale_to_display_enabled {
            rect.x = logical.x;
            rect.y = logical.y;
        } else {
            rect.x = pixel.x;
            rect.y = pixel.y;
        }

        // When an emulated, exclusive fullscreen window has focus, treat the mode dimensions as the display bounds.
        let fullscreen_window =
            crate::video::core::with_display(display, |d| d.fullscreen_window).flatten();
        let fs = fullscreen_window.and_then(|w| {
            let active = self.with_data(|d| d.window(w).is_some_and(|wd| wd.active));
            crate::video::core::with_window(w, |wd| {
                (wd.fullscreen_exclusive, wd.current_fullscreen_mode)
            })
            .ok()
            .map(|(exclusive, mode)| (exclusive, active, mode))
        });
        match fs {
            Some((true, true, mode)) if mode.w != 0 && mode.h != 0 => {
                rect.w = mode.w;
                rect.h = mode.h;
            }
            _ => {
                if !scale_to_display_enabled {
                    let current = crate::video::core::with_display(display, |d| d.current_mode())
                        .unwrap_or_default();
                    rect.w = current.w;
                    rect.h = current.h;
                } else if transform.0 & WlOutputTransform::_90.0 != 0 {
                    rect.w = pixel.height;
                    rect.h = pixel.width;
                } else {
                    rect.w = pixel.width;
                    rect.h = pixel.height;
                }
            }
        }
        Ok(rect)
    }

    /// Translation of `Wayland_VideoCleanup()`.
    fn wayland_video_cleanup(&self) {
        self.wayland_session_destroy();

        let displays = crate::video::display::displays().unwrap_or_default();
        for display in displays.iter().rev() {
            let key = self.with_data(|d| d.output_for_display(*display).map(|o| o.registry_id));
            if let Some(key) = key {
                self.wayland_free_display(key, false);
            }
        }
        // (outputs never added as displays)
        let rest = self.with_data(|d| std::mem::take(&mut d.output_list));
        for o in rest {
            Self::wayland_free_display_data(o);
        }

        let seats: Vec<u32> =
            self.with_data(|d| d.seat_list.iter().map(|s| s.registry_id).collect());
        for seat in seats {
            self.wayland_seat_destroy(seat, true);
        }

        self.wayland_fini_mouse();
        self.wayland_quit_keyboard();

        let context = self
            .event_thread
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        if let Some(context) = context {
            context.destroy();
        }

        // (dropping the globals destroys them, in upstream's order)
        let globals = self.with_data(|d| {
            let g = std::mem::take(&mut d.g);
            let xkb = d.xkb_context.take();
            (g, xkb)
        });
        let (mut g, xkb_context) = globals;
        g.pointer_constraints = None;
        g.relative_pointer_manager = None;
        g.activation_manager = None;
        g.idle_inhibit_manager = None;
        g.key_inhibitor_manager = None;
        g.text_input_manager = None;
        drop(xkb_context);
        g.tablet_manager = None;
        g.data_device_manager = None;
        // (wl_shm.release from version 2)
        g.shm = None;
        g.xdg_wm_base = None;
        g.decoration_manager = None;
        g.xdg_output_manager = None;
        g.viewporter = None;
        g.primary_selection_device_manager = None;
        g.fractional_scale_manager = None;
        g.input_timestamps_manager = None;
        g.cursor_shape_manager = None;
        g.zxdg_exporter_v2 = None;
        g.xdg_wm_dialog_v1 = None;
        g.wp_alpha_modifier_v1 = None;
        g.xdg_toplevel_icon_manager_v1 = None;
        g.frog_color_management_factory_v1 = None;
        g.wp_color_manager_v1 = None;
        g.wp_pointer_warp_v1 = None;
        // (zwp_pointer_gestures_v1.release from version 2)
        g.zwp_pointer_gestures = None;
        g.single_pixel_buffer_manager = None;
        g.xdg_session_manager = None;
        g.xdg_toplevel_tag_manager = None;
        g.subcompositor = None;
        g.compositor = None;

        if let Some(registry) = self.with_data(|d| d.registry.take()) {
            if let Some(fixes) = g.wl_fixes.take() {
                fixes.destroy_registry(registry.obj());
                drop(fixes);
            }
            drop(registry);
        }

        // (the sync points that never fired)
        self.callbacks.clear();
    }

    /// Translation of `Wayland_VideoReconnect()` (disabled upstream: "TODO
    /// RECONNECT").
    fn wayland_video_reconnect(&self) -> bool {
        false
    }

    /// Translation of `Wayland_HandleDisplayDisconnected()`.
    pub(crate) fn wayland_handle_display_disconnected(&self) -> bool {
        /* Something has failed with the Wayland connection -- for example,
         * the compositor may have shut down and closed its end of the socket,
         * or there is a library-specific error.
         *
         * Try to recover once, then quit.
         */
        if self.with_data(|d| d.display_disconnected) {
            return false;
        }

        if self.wayland_video_reconnect() {
            return true;
        }

        self.with_data(|d| d.display_disconnected = true);
        crate::error!(
            crate::log::Category::Video,
            "Wayland display connection closed by server (fatal)"
        );

        // Only send a single quit message, as application shutdown might call SDL_PumpEvents().
        crate::events::queue::send_quit();

        false
    }

    /// Translation of `Wayland_VideoQuit()`.
    fn wayland_video_quit(&self) {
        self.wayland_video_cleanup();

        // (libdecor_unref)
        let libdecor = self.with_data(|d| d.libdecor.take());
        drop(libdecor);
    }
}

/// Translation of `libdecor_error()`.
unsafe extern "C" fn libdecor_error(
    _context: *mut libdecor,
    error: libdecor_error,
    message: *const std::ffi::c_char,
) {
    super::client::catch_callback(|| {
        let message = if message.is_null() {
            String::new()
        } else {
            // SAFETY: libdecor passes a NUL-terminated message.
            unsafe { std::ffi::CStr::from_ptr(message) }
                .to_string_lossy()
                .into_owned()
        };
        crate::error!(
            crate::log::Category::Video,
            "libdecor error ({error}): {message}"
        );
    })
}

/// Whether GTK would agree to run in this process (libdecor's GTK plugin
/// calls `exit()` otherwise). Translation of `CanUseGtk()`.
pub(crate) fn can_use_gtk() -> bool {
    if !hints::get_bool("SDL_ENABLE_GTK", true) {
        crate::debug!(crate::log::Category::System, "Not using GTK due to hint");
        return false;
    }

    // This is intended to match the check in gtkmain.c, rather than being
    // an exhaustive check for having elevated privileges: as a result
    // we don't use Linux getauxval() or prctl PR_GET_DUMPABLE,
    // BSD issetugid(), or similar OS-specific detection

    // "Real", "effective" and "saved" IDs: see e.g. Linux credentials(7)
    let (mut ruid, mut euid, mut suid): (libc::uid_t, libc::uid_t, libc::uid_t) =
        (libc::uid_t::MAX, libc::uid_t::MAX, libc::uid_t::MAX);
    let (mut rgid, mut egid, mut sgid): (libc::gid_t, libc::gid_t, libc::gid_t) =
        (libc::gid_t::MAX, libc::gid_t::MAX, libc::gid_t::MAX);

    #[cfg(any(
        target_os = "linux",
        target_os = "android",
        target_os = "freebsd",
        target_os = "openbsd"
    ))]
    {
        // SAFETY: the out-parameters are valid.
        if unsafe { libc::getresuid(&mut ruid, &mut euid, &mut suid) } != 0 {
            // SAFETY: getuid/geteuid have no preconditions.
            unsafe {
                ruid = libc::getuid();
                suid = ruid;
                euid = libc::geteuid();
            }
        }

        // SAFETY: as above.
        if unsafe { libc::getresgid(&mut rgid, &mut egid, &mut sgid) } != 0 {
            // SAFETY: getgid/getegid have no preconditions.
            unsafe {
                rgid = libc::getgid();
                sgid = rgid;
                egid = libc::getegid();
            }
        }
    }
    #[cfg(not(any(
        target_os = "linux",
        target_os = "android",
        target_os = "freebsd",
        target_os = "openbsd"
    )))]
    {
        // (no getresuid(): behave as though it existed but failed)
        // SAFETY: getuid and friends have no preconditions.
        unsafe {
            ruid = libc::getuid();
            suid = ruid;
            euid = libc::geteuid();
            rgid = libc::getgid();
            sgid = rgid;
            egid = libc::getegid();
        }
    }

    // Real ID != effective ID means we are setuid or setgid:
    // GTK will refuse to initialize, and instead will call exit().
    if ruid != euid || rgid != egid {
        crate::debug!(
            crate::log::Category::System,
            "Not using GTK due to setuid/setgid"
        );
        return false;
    }

    // Real ID != saved ID means we are setuid or setgid, we previously
    // dropped privileges, but we can regain them; this protects against
    // accidents but does not protect against arbitrary code execution.
    // Again, GTK will refuse to initialize if this is the case.
    if ruid != suid || rgid != sgid {
        crate::debug!(
            crate::log::Category::System,
            "Not using GTK due to saved uid/gid"
        );
        return false;
    }

    true
}

/// `Option<String>` that is `None` or empty.
trait NoneOrEmpty {
    fn is_none_or_empty(&self) -> bool;
}

impl NoneOrEmpty for Option<String> {
    fn is_none_or_empty(&self) -> bool {
        self.as_deref().is_none_or(str::is_empty)
    }
}

/// The data of `wayland_preferred_check_handle_global()`.
#[derive(Default)]
struct PreferredData {
    has_fifo_v1: bool,
    wl_fixes: Option<Proxy<WlFixes>>,
}

/// Whether the compositor has the protocols the preferred bootstrap wants.
/// Translation of `Wayland_IsPreferred()`.
fn wayland_is_preferred(conn: &Arc<Conn>) -> bool {
    let mut registry = conn.obj().get_registry();
    let preferred_data = Arc::new(Mutex::new(PreferredData::default()));

    let data = preferred_data.clone();
    registry.listen(move |registry, ev| {
        let mut d = data.lock().unwrap_or_else(|e| e.into_inner());
        match ev {
            // Translation of `wayland_preferred_check_handle_global()`.
            WlRegistryEvent::Global {
                name,
                interface,
                version,
            } => {
                let interface = interface.to_bytes();
                if interface == b"wp_fifo_manager_v1" {
                    d.has_fifo_v1 = true;
                } else if interface == b"wl_fixes" {
                    d.wl_fixes = Some(registry.bind(name, SDL_WL_FIXES_VERSION.min(version)));
                }
            }
            // Translation of `wayland_preferred_check_remove_global()`.
            WlRegistryEvent::GlobalRemove { name } => {
                if let Some(fixes) = &d.wl_fixes {
                    if fixes.version() >= WlFixes::ACK_GLOBAL_REMOVE_SINCE_VERSION {
                        fixes.ack_global_remove(registry, name);
                    }
                }
            }
        }
    });

    let _ = conn.roundtrip();

    let (has_fifo_v1, fixes) = {
        let mut d = preferred_data.lock().unwrap_or_else(|e| e.into_inner());
        (d.has_fifo_v1, d.wl_fixes.take())
    };
    if let Some(fixes) = fixes {
        fixes.destroy_registry(registry.obj());
        drop(fixes);
    }
    drop(registry);

    if !has_fifo_v1 {
        crate::info!(
            crate::log::Category::Video,
            "This compositor lacks support for the fifo-v1 protocol; falling back to XWayland for GPU performance reasons (set SDL_VIDEO_DRIVER=wayland to override)"
        );
    }
    has_fifo_v1
}

/// Translation of `Wayland_CreateDevice()`.
fn wayland_create_device(require_preferred_protocols: bool) -> Option<Arc<dyn VideoDriver>> {
    let external = Properties::global()
        .get_number(PROP_GLOBAL_VIDEO_WAYLAND_WL_DISPLAY_POINTER)
        .filter(|&p| p != 0)
        .and_then(|p| NonNull::new(p as usize as *mut wl_display));

    // Are we trying to connect to, or are currently in, a Wayland session?
    if crate::stdlib::getenv("WAYLAND_DISPLAY").is_none() {
        let session = crate::stdlib::getenv("XDG_SESSION_TYPE");
        if session.is_some_and(|s| !s.eq_ignore_ascii_case("wayland")) {
            crate::debug!(
                crate::log::Category::Video,
                "Wayland initialization failed: no Wayland session available"
            );
            return None;
        }
    }

    let syms = load_symbols()?;

    let conn = match external {
        // SAFETY: the application gave us a connected display to use.
        Some(display) => unsafe { Conn::external(&syms, display) },
        None => match Conn::connect(&syms) {
            Ok(conn) => conn,
            Err(errno) => {
                crate::debug!(
                    crate::log::Category::Video,
                    "Failed to connect to the Wayland display server: {}",
                    std::io::Error::from_raw_os_error(errno)
                );
                return None;
            }
        },
    };

    /*
     * If we are checking for preferred Wayland, then let's query for
     * fifo-v1's existence, so we don't regress GPU-bound performance
     * and frame-pacing by default due to swapchain starvation.
     */
    if require_preferred_protocols && !wayland_is_preferred(&conn) {
        return None;
    }

    if !conn.is_external() {
        let _ = Properties::global().set(
            PROP_GLOBAL_VIDEO_WAYLAND_WL_DISPLAY_POINTER,
            ptr_prop(conn.raw().cast()),
        );
    }

    let data = VideoData {
        registry: None,
        g: Globals::default(),
        cursor_themes: Vec::new(),
        libdecor: None,
        xdg_session: None,
        xkb_context: None,
        seat_list: Vec::new(),
        last_implicit_grab_seat: None,
        current_data_offer_seat: None,
        current_primary_selection_seat: None,
        output_list: Vec::new(),
        windows: Vec::new(),
        mouse: MouseData::default(),
        initializing: true,
        display_disconnected: false,
        scale_to_display_enabled: hints::get_bool(hints::VIDEO_WAYLAND_SCALE_TO_DISPLAY, false),
        vulkan: super::vulkan::VulkanData::default(),
    };

    // (`device->system_theme` is set in video_init(), once the device exists)

    let device_caps = DeviceCaps::MODE_SWITCHING_EMULATED
        | DeviceCaps::HAS_POPUP_WINDOW_SUPPORT
        | DeviceCaps::SENDS_FULLSCREEN_DIMENSIONS
        | DeviceCaps::SENDS_DISPLAY_CHANGES
        | DeviceCaps::SENDS_HDR_CHANGES;

    let callbacks = Callbacks::new();
    let device = Arc::new_cyclic(|this| WaylandVideo {
        this: this.clone(),
        data: ReentrantMutex::new(RefCell::new(data)),
        callbacks,
        event_thread: Mutex::new(None),
        conn,
        caps: device_caps,
    });
    super::util::set_current_device(Arc::downgrade(&device));
    Some(device)
}

impl Drop for WaylandVideo {
    /// Translation of `Wayland_DeleteDevice()`.
    fn drop(&mut self) {
        if self.with_data(|d| d.vulkan.loader_loaded()) {
            self.wayland_vulkan_unload_library();
        }
        if !self.conn.is_external() {
            // (the display disconnects when the last object made on it goes)
            let ours = Properties::global()
                .get_number(PROP_GLOBAL_VIDEO_WAYLAND_WL_DISPLAY_POINTER)
                == Some(ptr_prop(self.conn.raw().cast()));
            if ours {
                let _ = Properties::global().remove(PROP_GLOBAL_VIDEO_WAYLAND_WL_DISPLAY_POINTER);
            }
        }
    }
}

/// Translation of `Wayland_Preferred_CreateDevice()`.
fn wayland_preferred_create_device() -> Option<Arc<dyn VideoDriver>> {
    wayland_create_device(true)
}

/// Translation of `Wayland_Fallback_CreateDevice()`.
fn wayland_fallback_create_device() -> Option<Arc<dyn VideoDriver>> {
    wayland_create_device(false)
}

/// Translation of `Wayland_preferred_bootstrap`.
pub(crate) static WAYLAND_PREFERRED_BOOTSTRAP: VideoBootStrap = VideoBootStrap {
    name: WAYLANDVID_DRIVER_NAME,
    desc: "SDL Wayland video driver",
    create: wayland_preferred_create_device,
    show_message_box: Some(super::messagebox::wayland_show_message_box),
    is_preferred: true,
};

/// Translation of `Wayland_bootstrap`.
pub(crate) static WAYLAND_BOOTSTRAP: VideoBootStrap = VideoBootStrap {
    name: WAYLANDVID_DRIVER_NAME,
    desc: "SDL Wayland video driver",
    create: wayland_fallback_create_device,
    show_message_box: Some(super::messagebox::wayland_show_message_box),
    is_preferred: false,
};

/// `timeoutNS` for `Wayland_WaitEventTimeout()`: -1 waits forever.
fn timeout_ns(timeout: Option<Duration>) -> i64 {
    match timeout {
        Some(d) => i64::try_from(d.as_nanos()).unwrap_or(i64::MAX),
        None => -1,
    }
}

/// The entry points (`device->VideoInit = Wayland_VideoInit;` ...).
impl VideoDriver for WaylandVideo {
    fn caps(&self) -> DeviceCaps {
        self.caps
    }

    fn video_init(&self) -> Result<()> {
        // (Wayland_CreateDevice() sets device->system_theme with
        // SDL_SystemTheme_Init(), which exists once the driver is chosen; no
        // event is sent)
        if crate::core::linux::system_theme::init() {
            let theme = crate::core::linux::system_theme::get();
            let _ = crate::video::core::with_device(|v| v.system_theme = theme);
        }
        self.wayland_video_init()
    }

    fn video_quit(&self) {
        self.wayland_video_quit()
    }

    fn display_bounds(&self, display: DisplayID) -> Option<Result<Rect>> {
        Some(self.wayland_get_display_bounds(display))
    }

    fn suspend_screen_saver(&self, suspend: bool) -> Option<Result<()>> {
        Some(self.wayland_suspend_screen_saver(suspend))
    }

    fn pump_events(&self) -> Option<()> {
        self.wayland_pump_events();
        Some(())
    }

    fn wait_event_timeout(&self, timeout: Option<Duration>) -> Option<i32> {
        Some(self.wayland_wait_event_timeout(timeout_ns(timeout)))
    }

    fn send_wakeup_event(&self, window: WindowID) -> Option<()> {
        self.wayland_send_wakeup_event(window);
        Some(())
    }

    fn can_wait_events(&self) -> bool {
        true
    }

    fn create_window(&self, window: WindowID, create_props: &Properties) -> Option<Result<()>> {
        Some(self.wayland_create_window(window, create_props))
    }

    fn show_window(&self, window: WindowID) -> Option<()> {
        self.wayland_show_window(window);
        Some(())
    }

    fn hide_window(&self, window: WindowID) -> Option<()> {
        self.wayland_hide_window(window);
        Some(())
    }

    fn raise_window(&self, window: WindowID) -> Option<()> {
        self.wayland_raise_window(window);
        Some(())
    }

    fn set_window_fullscreen(
        &self,
        window: WindowID,
        display: DisplayID,
        fullscreen: FullscreenOp,
    ) -> Option<FullscreenResult> {
        Some(self.wayland_set_window_fullscreen(window, display, fullscreen))
    }

    fn maximize_window(&self, window: WindowID) -> Option<()> {
        self.wayland_maximize_window(window);
        Some(())
    }

    fn minimize_window(&self, window: WindowID) -> Option<()> {
        self.wayland_minimize_window(window);
        Some(())
    }

    fn set_window_mouse_rect(&self, window: WindowID) -> Option<Result<()>> {
        Some(self.wayland_set_window_mouse_rect(window))
    }

    fn set_window_mouse_grab(&self, window: WindowID, grabbed: bool) -> Option<Result<()>> {
        Some(self.wayland_set_window_mouse_grab(window, grabbed))
    }

    fn set_window_keyboard_grab(&self, window: WindowID, grabbed: bool) -> Option<Result<()>> {
        Some(self.wayland_set_window_keyboard_grab(window, grabbed))
    }

    fn restore_window(&self, window: WindowID) -> Option<()> {
        self.wayland_restore_window(window);
        Some(())
    }

    fn set_window_bordered(&self, window: WindowID, bordered: bool) -> Option<()> {
        self.wayland_set_window_bordered(window, bordered);
        Some(())
    }

    fn set_window_resizable(&self, window: WindowID, resizable: bool) -> Option<()> {
        self.wayland_set_window_resizable(window, resizable);
        Some(())
    }

    fn set_window_position(&self, window: WindowID) -> Option<Result<()>> {
        Some(self.wayland_set_window_position(window))
    }

    fn set_window_size(&self, window: WindowID) -> Option<()> {
        self.wayland_set_window_size(window);
        Some(())
    }

    fn set_window_aspect_ratio(&self, window: WindowID) -> Option<()> {
        self.wayland_set_window_aspect_ratio(window);
        Some(())
    }

    fn set_window_minimum_size(&self, window: WindowID) -> Option<()> {
        self.wayland_set_window_minimum_size(window);
        Some(())
    }

    fn set_window_maximum_size(&self, window: WindowID) -> Option<()> {
        self.wayland_set_window_maximum_size(window);
        Some(())
    }

    fn set_window_parent(&self, window: WindowID, parent: Option<WindowID>) -> Option<Result<()>> {
        Some(self.wayland_set_window_parent(window, parent))
    }

    fn set_window_modal(&self, window: WindowID, modal: bool) -> Option<Result<()>> {
        Some(self.wayland_set_window_modal(window, modal))
    }

    fn set_window_opacity(&self, window: WindowID, opacity: f32) -> Option<Result<()>> {
        Some(self.wayland_set_window_opacity(window, opacity))
    }

    fn set_window_title(&self, window: WindowID) -> Option<()> {
        self.wayland_set_window_title(window);
        Some(())
    }

    fn set_window_icon(&self, window: WindowID, icon: &Surface<'static>) -> Option<Result<()>> {
        Some(self.wayland_set_window_icon(window, icon))
    }

    fn window_size_in_pixels(&self, window: WindowID) -> Option<(i32, i32)> {
        Some(self.wayland_get_window_size_in_pixels(window))
    }

    fn window_content_scale(&self, window: WindowID) -> Option<f32> {
        Some(self.wayland_get_window_content_scale(window))
    }

    fn window_icc_profile(&self, window: WindowID) -> Option<Result<Vec<u8>>> {
        Some(self.wayland_get_window_icc_profile(window))
    }

    fn display_for_window(&self, window: WindowID) -> Option<DisplayID> {
        Some(self.wayland_get_display_for_window(window))
    }

    fn destroy_window(&self, window: WindowID) -> Option<()> {
        self.wayland_destroy_window(window);
        Some(())
    }

    fn set_window_hit_test(&self, window: WindowID, enabled: bool) -> Option<Result<()>> {
        Some(super::window::wayland_set_window_hit_test(window, enabled))
    }

    fn flash_window(&self, window: WindowID, operation: FlashOperation) -> Option<Result<()>> {
        Some(self.wayland_flash_window(window, operation))
    }

    fn apply_window_progress(&self, window: WindowID) -> Option<Result<()>> {
        // (DBUS_ApplyWindowProgress)
        Some(
            if crate::core::linux::progressbar::apply_window_progress(window) {
                Ok(())
            } else {
                Err(Error::new("Couldn't send the progress over D-Bus"))
            },
        )
    }

    fn has_screen_keyboard_support(&self) -> Option<bool> {
        Some(self.wayland_has_screen_keyboard_support())
    }

    fn show_window_system_menu(&self, window: WindowID, x: i32, y: i32) -> Option<()> {
        self.wayland_show_window_system_menu(window, x, y);
        Some(())
    }

    fn sync_window(&self, window: WindowID) -> Option<Result<()>> {
        Some(self.wayland_sync_window(window))
    }

    fn implements_sync_window(&self) -> bool {
        true
    }

    fn set_window_focusable(&self, window: WindowID, focusable: bool) -> Option<Result<()>> {
        Some(self.wayland_set_window_focusable(window, focusable))
    }

    fn reconfigure_window(
        &self,
        window: WindowID,
        flags: crate::events::window::WindowFlags,
    ) -> Option<Result<()>> {
        Some(self.wayland_reconfigure_window(window, flags))
    }

    fn accept_drag_and_drop(&self, window: WindowID, accept: bool) -> Option<()> {
        self.wayland_accept_drag_and_drop(window, accept);
        Some(())
    }

    fn implements_window_op(&self, op: WindowOp) -> bool {
        matches!(
            op,
            WindowOp::SetBordered
                | WindowOp::SetResizable
                | WindowOp::Maximize
                | WindowOp::Minimize
                | WindowOp::Restore
                | WindowOp::SetParent
                | WindowOp::SetModal
                | WindowOp::SetFocusable
                | WindowOp::SetMouseRect
                | WindowOp::SetHitTest
        )
    }

    // * * * The framebuffer (wl_shm; see framebuffer.rs)

    fn implements_window_framebuffer(&self) -> bool {
        true
    }

    fn create_window_framebuffer(
        &self,
        window: WindowID,
        w: i32,
        h: i32,
    ) -> Option<Result<Surface<'static>>> {
        Some(self.wayland_create_window_framebuffer(window, w, h))
    }

    fn update_window_framebuffer(
        &self,
        window: WindowID,
        surface: &Surface<'static>,
        rects: &[Rect],
    ) -> Option<Result<()>> {
        Some(self.wayland_update_window_framebuffer(window, surface, rects))
    }

    fn destroy_window_framebuffer(&self, window: WindowID) -> Option<()> {
        self.wayland_destroy_window_framebuffer(window);
        Some(())
    }

    // * * * Clipboard

    fn text_mime_types(&self) -> Option<Vec<String>> {
        Some(
            super::clipboard::wayland_get_text_mime_types()
                .iter()
                .map(|m| (*m).to_owned())
                .collect(),
        )
    }

    fn set_clipboard_data(&self) -> Option<Result<()>> {
        Some(self.wayland_set_clipboard_data())
    }

    fn clipboard_data(&self, mime_type: &str) -> Option<Option<Vec<u8>>> {
        Some(self.wayland_get_clipboard_data(mime_type))
    }

    fn has_clipboard_data(&self, mime_type: &str) -> Option<bool> {
        Some(self.wayland_has_clipboard_data(mime_type))
    }

    // These are only set (in Wayland_VideoInit()) when the compositor has
    // the primary selection protocol.
    fn set_primary_selection_text(&self, text: &str) -> Option<Result<()>> {
        if !self.with_data(|d| d.g.primary_selection_device_manager.is_some()) {
            return None;
        }
        Some(self.wayland_set_primary_selection_text(text))
    }

    fn primary_selection_text(&self) -> Option<String> {
        if !self.with_data(|d| d.g.primary_selection_device_manager.is_some()) {
            return None;
        }
        Some(self.wayland_get_primary_selection_text())
    }

    fn has_primary_selection_text(&self) -> Option<bool> {
        if !self.with_data(|d| d.g.primary_selection_device_manager.is_some()) {
            return None;
        }
        Some(self.wayland_has_primary_selection_text())
    }

    // * * * Text input

    fn start_text_input(&self, window: WindowID, props: Option<&Properties>) -> Option<Result<()>> {
        Some(self.wayland_start_text_input(window, props))
    }

    fn stop_text_input(&self, window: WindowID) -> Option<Result<()>> {
        Some(self.wayland_stop_text_input(window))
    }

    fn update_text_input_area(&self, window: WindowID) -> Option<Result<()>> {
        Some(self.wayland_update_text_input_area(window))
    }

    // * * * Message boxes

    fn show_message_box(&self, data: &MessageBoxData) -> Option<Result<i32>> {
        // (the device has no ShowMessageBox; the bootstrap's is used)
        let _ = data;
        None
    }

    // * * * Vulkan (OpenGL ES through EGL isn't translated)

    fn implements_vulkan_surfaces(&self) -> bool {
        true
    }

    fn vulkan_load_library(&self, path: Option<&str>) -> Option<Result<()>> {
        Some(self.wayland_vulkan_load_library(path))
    }

    fn vulkan_unload_library(&self) -> Option<()> {
        self.wayland_vulkan_unload_library();
        Some(())
    }

    fn vulkan_get_instance_proc_addr(&self) -> Option<usize> {
        self.wayland_vulkan_get_instance_proc_addr()
    }

    fn vulkan_instance_extensions(&self) -> Option<Vec<&'static str>> {
        Some(self.wayland_vulkan_get_instance_extensions())
    }

    fn vulkan_create_surface(
        &self,
        window: WindowID,
        instance: usize,
        allocator: usize,
    ) -> Option<Result<u64>> {
        Some(self.wayland_vulkan_create_surface(window, instance, allocator))
    }

    fn vulkan_destroy_surface(
        &self,
        instance: usize,
        surface: u64,
        allocator: usize,
    ) -> Option<()> {
        self.wayland_vulkan_destroy_surface(instance, surface, allocator);
        Some(())
    }

    fn vulkan_presentation_support(
        &self,
        instance: usize,
        physical_device: usize,
        queue_family_index: u32,
    ) -> Option<bool> {
        Some(self.wayland_vulkan_get_presentation_support(
            instance,
            physical_device,
            queue_family_index,
        ))
    }

    // * * * The SDL_Mouse entry points (Wayland_InitMouse())

    fn create_cursor(
        &self,
        surface: &Surface<'_>,
        hot_x: i32,
        hot_y: i32,
    ) -> Option<Result<Cursor>> {
        Some(self.wayland_create_cursor(surface, hot_x, hot_y))
    }

    fn create_animated_cursor(
        &self,
        frames: &[CursorFrame<'_>],
        hot_x: i32,
        hot_y: i32,
    ) -> Option<Result<Cursor>> {
        Some(self.wayland_create_animated_cursor(frames, hot_x, hot_y))
    }

    fn create_system_cursor(&self, id: SystemCursor) -> Option<Result<Cursor>> {
        Some(Ok(self.wayland_create_system_cursor(id)))
    }

    fn show_cursor(&self, cursor: Option<&Cursor>) -> Option<Result<()>> {
        Some(self.wayland_show_cursor(cursor))
    }

    fn warp_mouse(&self, window: WindowID, x: f32, y: f32) -> Option<Result<()>> {
        Some(self.wayland_warp_mouse_relative(window, x, y))
    }

    fn warp_mouse_global(&self, x: f32, y: f32) -> Option<Result<()>> {
        Some(self.wayland_warp_mouse_global(x, y))
    }

    fn set_relative_mouse_mode(&self, enabled: bool) -> Option<Result<()>> {
        Some(self.wayland_set_relative_mouse_mode(enabled))
    }

    fn global_mouse_state(&self) -> Option<(f32, f32, MouseButtonFlags)> {
        Some(self.wayland_get_global_mouse_state())
    }

    fn implements_mouse_feature(&self, feature: MouseFeature) -> bool {
        matches!(
            feature,
            MouseFeature::WarpMouse
                | MouseFeature::WarpMouseGlobal
                | MouseFeature::SetRelativeMouseMode
                | MouseFeature::GetGlobalMouseState
                | MouseFeature::ExplicitWarpEvent
        )
    }
}

/// `CString` for a Rust string handed to libdecor (cut at a NUL).
pub(crate) fn c_string(s: &str) -> CString {
    super::client::cstring_arg(s)
}
