// Rust translation of the window parts of src/video/SDL_video.c from
// Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Windows.
//!
//! A [`Window`] is a handle to a window owned by the video subsystem, as
//! `SDL_Window *` is: it is `Copy`, and the window lives until
//! [`Window::destroy`], the destruction of its parent, or the end of the
//! video subsystem. Every call checks the handle, so using a destroyed
//! window fails with "Invalid window" (upstream's `CHECK_WINDOW_MAGIC`)
//! instead of touching freed memory.

use std::sync::{Arc, Mutex, MutexGuard};

use crate::error::{Error, Result};
use crate::events::mouse;
use crate::events::window::{send_window_event, WindowFlags};
use crate::events::{keyboard, DisplayID, EventType, WindowID};
use crate::hints;
use crate::properties::Properties;
use crate::video::{PixelFormat, Point, Rect, Surface};

use super::core::{
    self, context_not_supported, driver, initialized, sync_if_required, uninitialized_video,
    update_fullscreen_mode, update_window_hierarchy, with_device, with_window,
};
use super::display::{
    self, display_bounds, display_index, display_usable_bounds, primary_display, ModeMatch,
};
use super::sysvideo::{
    windowpos_is_centered, windowpos_is_undefined, DeviceCaps, DisplayMode, FlashOperation,
    FullscreenOp, HitTest, ProgressState, SharedSurface, WindowData, WINDOWPOS_UNDEFINED,
};

// Window creation properties (SDL_PROP_WINDOW_CREATE_*)
pub const PROP_WINDOW_CREATE_ALWAYS_ON_TOP_BOOLEAN: &str = "SDL.window.create.always_on_top";
pub const PROP_WINDOW_CREATE_BORDERLESS_BOOLEAN: &str = "SDL.window.create.borderless";
pub const PROP_WINDOW_CREATE_CONSTRAIN_POPUP_BOOLEAN: &str = "SDL.window.create.constrain_popup";
pub const PROP_WINDOW_CREATE_FOCUSABLE_BOOLEAN: &str = "SDL.window.create.focusable";
pub const PROP_WINDOW_CREATE_EXTERNAL_GRAPHICS_CONTEXT_BOOLEAN: &str =
    "SDL.window.create.external_graphics_context";
pub const PROP_WINDOW_CREATE_FLAGS_NUMBER: &str = "SDL.window.create.flags";
pub const PROP_WINDOW_CREATE_FULLSCREEN_BOOLEAN: &str = "SDL.window.create.fullscreen";
pub const PROP_WINDOW_CREATE_HEIGHT_NUMBER: &str = "SDL.window.create.height";
pub const PROP_WINDOW_CREATE_HIDDEN_BOOLEAN: &str = "SDL.window.create.hidden";
pub const PROP_WINDOW_CREATE_HIGH_PIXEL_DENSITY_BOOLEAN: &str =
    "SDL.window.create.high_pixel_density";
pub const PROP_WINDOW_CREATE_MAXIMIZED_BOOLEAN: &str = "SDL.window.create.maximized";
pub const PROP_WINDOW_CREATE_MENU_BOOLEAN: &str = "SDL.window.create.menu";
pub const PROP_WINDOW_CREATE_METAL_BOOLEAN: &str = "SDL.window.create.metal";
pub const PROP_WINDOW_CREATE_MINIMIZED_BOOLEAN: &str = "SDL.window.create.minimized";
pub const PROP_WINDOW_CREATE_MODAL_BOOLEAN: &str = "SDL.window.create.modal";
pub const PROP_WINDOW_CREATE_MOUSE_GRABBED_BOOLEAN: &str = "SDL.window.create.mouse_grabbed";
pub const PROP_WINDOW_CREATE_OPENGL_BOOLEAN: &str = "SDL.window.create.opengl";
/// The parent [`Window`] (stored as an `Any` value).
pub const PROP_WINDOW_CREATE_PARENT_POINTER: &str = "SDL.window.create.parent";
pub const PROP_WINDOW_CREATE_RESIZABLE_BOOLEAN: &str = "SDL.window.create.resizable";
pub const PROP_WINDOW_CREATE_TITLE_STRING: &str = "SDL.window.create.title";
pub const PROP_WINDOW_CREATE_TRANSPARENT_BOOLEAN: &str = "SDL.window.create.transparent";
pub const PROP_WINDOW_CREATE_TOOLTIP_BOOLEAN: &str = "SDL.window.create.tooltip";
pub const PROP_WINDOW_CREATE_UTILITY_BOOLEAN: &str = "SDL.window.create.utility";
pub const PROP_WINDOW_CREATE_VULKAN_BOOLEAN: &str = "SDL.window.create.vulkan";
pub const PROP_WINDOW_CREATE_WIDTH_NUMBER: &str = "SDL.window.create.width";
pub const PROP_WINDOW_CREATE_X_NUMBER: &str = "SDL.window.create.x";
pub const PROP_WINDOW_CREATE_Y_NUMBER: &str = "SDL.window.create.y";
/// The `HWND` of an existing window to wrap (Windows).
pub const PROP_WINDOW_CREATE_WIN32_HWND_POINTER: &str = "SDL.window.create.win32.hwnd";
/// A window whose pixel format the new window shares (Windows).
pub const PROP_WINDOW_CREATE_WIN32_PIXEL_FORMAT_HWND_POINTER: &str =
    "SDL.window.create.win32.pixel_format_hwnd";
/// Extra extended window styles for the new window (Windows).
pub const PROP_WINDOW_CREATE_WIN32_STYLE_EX_NUMBER: &str = "SDL.window.create.win32.style_ex";

/// The X11 `Window` to wrap (instead of creating one). Translation of
/// `SDL_PROP_WINDOW_CREATE_X11_WINDOW_NUMBER`.
pub const PROP_WINDOW_CREATE_X11_WINDOW_NUMBER: &str = "SDL.window.create.x11.window";
/// Give the window's `wl_surface` a custom role (no xdg-shell toplevel or
/// popup). Translation of `SDL_PROP_WINDOW_CREATE_WAYLAND_SURFACE_ROLE_CUSTOM_BOOLEAN`.
pub const PROP_WINDOW_CREATE_WAYLAND_SURFACE_ROLE_CUSTOM_BOOLEAN: &str =
    "SDL.window.create.wayland.surface_role_custom";
/// Create a `wl_egl_window` for the window. Translation of
/// `SDL_PROP_WINDOW_CREATE_WAYLAND_CREATE_EGL_WINDOW_BOOLEAN`.
pub const PROP_WINDOW_CREATE_WAYLAND_CREATE_EGL_WINDOW_BOOLEAN: &str =
    "SDL.window.create.wayland.create_egl_window";
/// The window's ID string, for session management and toplevel tags.
/// Translation of `SDL_PROP_WINDOW_CREATE_WAYLAND_WINDOW_ID_STRING`.
pub const PROP_WINDOW_CREATE_WAYLAND_WINDOW_ID_STRING: &str = "SDL.window.create.wayland.window_id";
/// The `wl_surface` to wrap (an address, as a number). Translation of
/// `SDL_PROP_WINDOW_CREATE_WAYLAND_WL_SURFACE_POINTER`.
pub const PROP_WINDOW_CREATE_WAYLAND_WL_SURFACE_POINTER: &str =
    "SDL.window.create.wayland.wl_surface";
/// Use the `PROP_WINDOW_WAYLAND_BORDER_INSET_*` window properties. Translation
/// of `SDL_PROP_WINDOW_CREATE_WAYLAND_ENABLE_INSETS_BOOLEAN`.
pub const PROP_WINDOW_CREATE_WAYLAND_ENABLE_INSETS_BOOLEAN: &str =
    "SDL.window.create.wayland.enable_insets";

// Window properties (SDL_PROP_WINDOW_*)
/// The X11 `Display *` of the window, as an address (a number). Translation
/// of `SDL_PROP_WINDOW_X11_DISPLAY_POINTER`.
pub const PROP_WINDOW_X11_DISPLAY_POINTER: &str = "SDL.window.x11.display";
/// The X11 screen number of the window. Translation of
/// `SDL_PROP_WINDOW_X11_SCREEN_NUMBER`.
pub const PROP_WINDOW_X11_SCREEN_NUMBER: &str = "SDL.window.x11.screen";
/// The X11 `Window` of the window. Translation of
/// `SDL_PROP_WINDOW_X11_WINDOW_NUMBER`.
pub const PROP_WINDOW_X11_WINDOW_NUMBER: &str = "SDL.window.x11.window";
/// The `wl_display` of the window (an address, as a number). Translation of
/// `SDL_PROP_WINDOW_WAYLAND_DISPLAY_POINTER`.
pub const PROP_WINDOW_WAYLAND_DISPLAY_POINTER: &str = "SDL.window.wayland.display";
/// The `wl_surface` of the window (an address). Translation of
/// `SDL_PROP_WINDOW_WAYLAND_SURFACE_POINTER`.
pub const PROP_WINDOW_WAYLAND_SURFACE_POINTER: &str = "SDL.window.wayland.surface";
/// The `wp_viewport` of the window (an address). Translation of
/// `SDL_PROP_WINDOW_WAYLAND_VIEWPORT_POINTER`.
pub const PROP_WINDOW_WAYLAND_VIEWPORT_POINTER: &str = "SDL.window.wayland.viewport";
/// The `wl_egl_window` of the window (an address; never set, as EGL isn't
/// translated). Translation of `SDL_PROP_WINDOW_WAYLAND_EGL_WINDOW_POINTER`.
pub const PROP_WINDOW_WAYLAND_EGL_WINDOW_POINTER: &str = "SDL.window.wayland.egl_window";
/// The window's ID string. Translation of `SDL_PROP_WINDOW_WAYLAND_WINDOW_ID_STRING`.
pub const PROP_WINDOW_WAYLAND_WINDOW_ID_STRING: &str = "SDL.window.wayland.window_id";
/// The `xdg_surface` of the window (an address). Translation of
/// `SDL_PROP_WINDOW_WAYLAND_XDG_SURFACE_POINTER`.
pub const PROP_WINDOW_WAYLAND_XDG_SURFACE_POINTER: &str = "SDL.window.wayland.xdg_surface";
/// The `xdg_toplevel` of the window (an address). Translation of
/// `SDL_PROP_WINDOW_WAYLAND_XDG_TOPLEVEL_POINTER`.
pub const PROP_WINDOW_WAYLAND_XDG_TOPLEVEL_POINTER: &str = "SDL.window.wayland.xdg_toplevel";
/// The xdg-foreign export handle of the window. Translation of
/// `SDL_PROP_WINDOW_WAYLAND_XDG_TOPLEVEL_EXPORT_HANDLE_STRING`.
pub const PROP_WINDOW_WAYLAND_XDG_TOPLEVEL_EXPORT_HANDLE_STRING: &str =
    "SDL.window.wayland.xdg_toplevel_export_handle";
/// The `xdg_popup` of the window (an address). Translation of
/// `SDL_PROP_WINDOW_WAYLAND_XDG_POPUP_POINTER`.
pub const PROP_WINDOW_WAYLAND_XDG_POPUP_POINTER: &str = "SDL.window.wayland.xdg_popup";
/// The `xdg_positioner` of the window (an address). Translation of
/// `SDL_PROP_WINDOW_WAYLAND_XDG_POSITIONER_POINTER`.
pub const PROP_WINDOW_WAYLAND_XDG_POSITIONER_POINTER: &str = "SDL.window.wayland.xdg_positioner";
/// The left border inset of the window. Translation of
/// `SDL_PROP_WINDOW_WAYLAND_BORDER_INSET_LEFT_NUMBER`.
pub const PROP_WINDOW_WAYLAND_BORDER_INSET_LEFT_NUMBER: &str =
    "SDL.window.wayland.border_inset_left";
/// The top border inset of the window. Translation of
/// `SDL_PROP_WINDOW_WAYLAND_BORDER_INSET_TOP_NUMBER`.
pub const PROP_WINDOW_WAYLAND_BORDER_INSET_TOP_NUMBER: &str = "SDL.window.wayland.border_inset_top";
/// The right border inset of the window. Translation of
/// `SDL_PROP_WINDOW_WAYLAND_BORDER_INSET_RIGHT_NUMBER`.
pub const PROP_WINDOW_WAYLAND_BORDER_INSET_RIGHT_NUMBER: &str =
    "SDL.window.wayland.border_inset_right";
/// The bottom border inset of the window. Translation of
/// `SDL_PROP_WINDOW_WAYLAND_BORDER_INSET_BOTTOM_NUMBER`.
pub const PROP_WINDOW_WAYLAND_BORDER_INSET_BOTTOM_NUMBER: &str =
    "SDL.window.wayland.border_inset_bottom";
/// The window shape set with [`Window::set_shape`] (an `Any` [`Surface`]).
pub const PROP_WINDOW_SHAPE_POINTER: &str = "SDL.window.shape";
pub const PROP_WINDOW_HDR_ENABLED_BOOLEAN: &str = "SDL.window.HDR_enabled";
pub const PROP_WINDOW_SDR_WHITE_LEVEL_FLOAT: &str = "SDL.window.SDR_white_level";
pub const PROP_WINDOW_HDR_HEADROOM_FLOAT: &str = "SDL.window.HDR_headroom";
/// The window's `HWND` (Windows), as a number.
pub const PROP_WINDOW_WIN32_HWND_POINTER: &str = "SDL.window.win32.hwnd";
/// The window's `HDC` (Windows), as a number.
pub const PROP_WINDOW_WIN32_HDC_POINTER: &str = "SDL.window.win32.hdc";
/// The window's `HINSTANCE` (Windows), as a number.
pub const PROP_WINDOW_WIN32_INSTANCE_POINTER: &str = "SDL.window.win32.instance";

/// Translation of `SDL_PROP_SDL2_COMPAT_WINDOW_PREFERRED_FULLSCREEN_DISPLAY`.
const PROP_SDL2_COMPAT_WINDOW_PREFERRED_FULLSCREEN_DISPLAY: &str =
    "sdl2-compat.window.preferred_fullscreen_display";

/// The flags a window can be created with. Translation of `CREATE_FLAGS`.
const CREATE_FLAGS: WindowFlags = WindowFlags(
    WindowFlags::OPENGL.0
        | WindowFlags::BORDERLESS.0
        | WindowFlags::RESIZABLE.0
        | WindowFlags::HIGH_PIXEL_DENSITY.0
        | WindowFlags::ALWAYS_ON_TOP.0
        | WindowFlags::POPUP_MENU.0
        | WindowFlags::UTILITY.0
        | WindowFlags::TOOLTIP.0
        | WindowFlags::VULKAN.0
        | WindowFlags::MINIMIZED.0
        | WindowFlags::METAL.0
        | WindowFlags::TRANSPARENT.0
        | WindowFlags::NOT_FOCUSABLE.0
        | WindowFlags::FILL_DOCUMENT.0,
);

const GRAPHICS_FLAGS: WindowFlags =
    WindowFlags(WindowFlags::OPENGL.0 | WindowFlags::METAL.0 | WindowFlags::VULKAN.0);

/// The boolean creation properties and the flags they set. Translation of
/// `SDL_WindowFlagProperties`.
const WINDOW_FLAG_PROPERTIES: &[(&str, WindowFlags, bool)] = &[
    (
        PROP_WINDOW_CREATE_ALWAYS_ON_TOP_BOOLEAN,
        WindowFlags::ALWAYS_ON_TOP,
        false,
    ),
    (
        PROP_WINDOW_CREATE_BORDERLESS_BOOLEAN,
        WindowFlags::BORDERLESS,
        false,
    ),
    (
        PROP_WINDOW_CREATE_FOCUSABLE_BOOLEAN,
        WindowFlags::NOT_FOCUSABLE,
        true,
    ),
    (
        PROP_WINDOW_CREATE_FULLSCREEN_BOOLEAN,
        WindowFlags::FULLSCREEN,
        false,
    ),
    (
        PROP_WINDOW_CREATE_HIDDEN_BOOLEAN,
        WindowFlags::HIDDEN,
        false,
    ),
    (
        PROP_WINDOW_CREATE_HIGH_PIXEL_DENSITY_BOOLEAN,
        WindowFlags::HIGH_PIXEL_DENSITY,
        false,
    ),
    (
        PROP_WINDOW_CREATE_MAXIMIZED_BOOLEAN,
        WindowFlags::MAXIMIZED,
        false,
    ),
    (
        PROP_WINDOW_CREATE_MENU_BOOLEAN,
        WindowFlags::POPUP_MENU,
        false,
    ),
    (PROP_WINDOW_CREATE_METAL_BOOLEAN, WindowFlags::METAL, false),
    (
        PROP_WINDOW_CREATE_MINIMIZED_BOOLEAN,
        WindowFlags::MINIMIZED,
        false,
    ),
    (PROP_WINDOW_CREATE_MODAL_BOOLEAN, WindowFlags::MODAL, false),
    (
        PROP_WINDOW_CREATE_MOUSE_GRABBED_BOOLEAN,
        WindowFlags::MOUSE_GRABBED,
        false,
    ),
    (
        PROP_WINDOW_CREATE_OPENGL_BOOLEAN,
        WindowFlags::OPENGL,
        false,
    ),
    (
        PROP_WINDOW_CREATE_RESIZABLE_BOOLEAN,
        WindowFlags::RESIZABLE,
        false,
    ),
    (
        PROP_WINDOW_CREATE_TRANSPARENT_BOOLEAN,
        WindowFlags::TRANSPARENT,
        false,
    ),
    (
        PROP_WINDOW_CREATE_TOOLTIP_BOOLEAN,
        WindowFlags::TOOLTIP,
        false,
    ),
    (
        PROP_WINDOW_CREATE_UTILITY_BOOLEAN,
        WindowFlags::UTILITY,
        false,
    ),
    (
        PROP_WINDOW_CREATE_VULKAN_BOOLEAN,
        WindowFlags::VULKAN,
        false,
    ),
];

/// Translation of `SDL_GetWindowFlagProperties()`.
fn window_flag_properties(props: &Properties) -> WindowFlags {
    let mut flags = WindowFlags(
        props
            .get_number(PROP_WINDOW_CREATE_FLAGS_NUMBER)
            .unwrap_or(0) as u64,
    );

    for &(name, flag, invert_value) in WINDOW_FLAG_PROPERTIES {
        if invert_value {
            if !props.get_bool(name).unwrap_or(true) {
                flags |= flag;
            }
        } else if props.get_bool(name).unwrap_or(false) {
            flags |= flag;
        }
    }
    flags
}

/// Whether more than one bit of `flags` is set (`flags & (flags - 1)`).
fn more_than_one(flags: WindowFlags) -> bool {
    flags.0 & flags.0.wrapping_sub(1) != 0
}

/// A handle to a window. Translation of `SDL_Window *`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Window {
    id: WindowID,
}

/// How to create a window: a convenience over the creation properties of
/// [`Window::create_with_properties`].
#[derive(Clone, Debug, Default)]
pub struct WindowBuilder {
    title: Option<String>,
    x: Option<i32>,
    y: Option<i32>,
    w: i32,
    h: i32,
    flags: WindowFlags,
    parent: Option<Window>,
    external_graphics_context: bool,
    constrain_popup: Option<bool>,
}

impl WindowBuilder {
    /// The title of the window, in UTF-8 encoding.
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }
    /// The position: a coordinate, or [`WINDOWPOS_CENTERED`] or
    /// [`WINDOWPOS_UNDEFINED`] (the default), optionally for a display.
    ///
    /// [`WINDOWPOS_CENTERED`]: super::sysvideo::WINDOWPOS_CENTERED
    pub fn position(mut self, x: i32, y: i32) -> Self {
        self.x = Some(x);
        self.y = Some(y);
        self
    }
    /// The size of the window in screen coordinates.
    pub fn size(mut self, w: i32, h: i32) -> Self {
        self.w = w;
        self.h = h;
        self
    }
    /// The window flags ([`WindowFlags::RESIZABLE`] etc.).
    pub fn flags(mut self, flags: WindowFlags) -> Self {
        self.flags = flags;
        self
    }
    /// The parent window, required for popups and modal windows; a popup's
    /// position is relative to its parent's.
    pub fn parent(mut self, parent: &Window) -> Self {
        self.parent = Some(*parent);
        self
    }
    /// The app manages the graphics context of the window itself.
    pub fn external_graphics_context(mut self, external: bool) -> Self {
        self.external_graphics_context = external;
        self
    }
    /// Whether a popup should be kept inside the display (default true).
    pub fn constrain_popup(mut self, constrain: bool) -> Self {
        self.constrain_popup = Some(constrain);
        self
    }

    /// The creation properties for this window.
    pub fn properties(&self) -> Result<Properties> {
        let props = Properties::new();
        if let Some(title) = self.title.as_ref().filter(|t| !t.is_empty()) {
            props.set(PROP_WINDOW_CREATE_TITLE_STRING, title.as_str())?;
        }
        if let Some(x) = self.x {
            props.set(PROP_WINDOW_CREATE_X_NUMBER, x as i64)?;
        }
        if let Some(y) = self.y {
            props.set(PROP_WINDOW_CREATE_Y_NUMBER, y as i64)?;
        }
        props.set(PROP_WINDOW_CREATE_WIDTH_NUMBER, self.w as i64)?;
        props.set(PROP_WINDOW_CREATE_HEIGHT_NUMBER, self.h as i64)?;
        props.set(PROP_WINDOW_CREATE_FLAGS_NUMBER, self.flags.0 as i64)?;
        if let Some(parent) = self.parent {
            props.set(
                PROP_WINDOW_CREATE_PARENT_POINTER,
                crate::properties::Value::any(parent),
            )?;
        }
        if self.external_graphics_context {
            props.set(PROP_WINDOW_CREATE_EXTERNAL_GRAPHICS_CONTEXT_BOOLEAN, true)?;
        }
        if let Some(constrain) = self.constrain_popup {
            props.set(PROP_WINDOW_CREATE_CONSTRAIN_POPUP_BOOLEAN, constrain)?;
        }
        Ok(props)
    }

    /// Create the window.
    pub fn build(self) -> Result<Window> {
        Window::create_with_properties(&self.properties()?)
    }
}

/// The windows, the most recently created first. Translation of
/// `SDL_GetWindows()`.
pub fn windows() -> Result<Vec<Window>> {
    with_device(|v| v.windows.iter().map(|w| Window { id: w.core.id }).collect())
}

/// The window that currently has an input grab, if any. Translation of
/// `SDL_GetGrabbedWindow()`.
pub fn grabbed_window() -> Option<Window> {
    core::grabbed_window().map(|id| Window { id })
}

/// Translation of `IsAcceptingDragAndDrop()`.
fn is_accepting_drag_and_drop() -> bool {
    crate::events::event_enabled(EventType::DROP_FILE)
        || crate::events::event_enabled(EventType::DROP_TEXT)
}

/// Translation of `PrepareDragAndDropSupport()`.
fn prepare_drag_and_drop_support(window: WindowID) {
    if let Ok(driver) = driver() {
        let _ = driver.accept_drag_and_drop(window, is_accepting_drag_and_drop());
    }
}

/// Toggle drag and drop for all existing windows. Translation of
/// `SDL_ToggleDragAndDropSupport()`.
pub(crate) fn toggle_drag_and_drop_support() {
    let Ok(driver) = driver() else { return };
    let enable = is_accepting_drag_and_drop();
    for window in core::window_ids() {
        if driver.accept_drag_and_drop(window, enable).is_none() {
            return;
        }
    }
}

/// Apply flags a window was created with (or had pending while hidden).
/// Translation of `ApplyWindowFlags()`.
pub(crate) fn apply_window_flags(window: WindowID, flags: WindowFlags) {
    let w = Window { id: window };
    if with_window(window, |w| w.is_popup()).unwrap_or(true) {
        return;
    }
    if !flags.intersects(WindowFlags::MINIMIZED | WindowFlags::MAXIMIZED) {
        let _ = w.restore();
    }
    if flags.contains(WindowFlags::MAXIMIZED) {
        let _ = w.maximize();
    }

    let _ = w.set_fullscreen(flags.contains(WindowFlags::FULLSCREEN));

    if flags.contains(WindowFlags::MINIMIZED) {
        let _ = w.minimize();
    }

    if flags.contains(WindowFlags::MODAL) {
        let _ = w.set_modal(true);
    }

    if flags.contains(WindowFlags::MOUSE_GRABBED) {
        let _ = w.set_mouse_grab(true);
    }
    if flags.contains(WindowFlags::KEYBOARD_GRABBED) {
        let _ = w.set_keyboard_grab(true);
    }
}

/// Translation of `SDL_FinishWindowCreation()`.
fn finish_window_creation(window: WindowID, flags: WindowFlags) {
    prepare_drag_and_drop_support(window);

    let external =
        with_window(window, |w| w.flags().contains(WindowFlags::EXTERNAL)).unwrap_or(false);
    if external {
        // Whoever has created the window has already applied whatever flags are needed
    } else {
        apply_window_flags(window, flags);
        if !flags.contains(WindowFlags::HIDDEN) {
            let _ = Window { id: window }.show();
        }
    }

    if cfg!(target_os = "linux") && !with_window(window, |w| w.is_popup()).unwrap_or(true) {
        // On Linux the progress state is persisted throughout multiple program runs, so reset state on window creation
        let w = Window { id: window };
        let _ = w.set_progress_state(ProgressState::None);
        let _ = w.set_progress_value(0.0);
    }
}

/// The graphics backends some platforms enable by default. Translation of
/// `SDL_DefaultGraphicsBackends()` (whose cases are all for backends not
/// translated yet: OpenGL on macOS/iOS/QNX, Metal on Apple platforms,
/// OpenVR).
fn default_graphics_backends() -> WindowFlags {
    WindowFlags::NONE
}

/// Shared by [`relative_to_global_for_window`] and its inverse: the
/// offsets of the popup chain above a window.
fn popup_offset(window: WindowID) -> (i32, i32) {
    let mut dx = 0;
    let mut dy = 0;
    if with_window(window, |w| w.is_popup()).unwrap_or(false) {
        // Calculate the total offset of the popup from the parents
        let mut w = with_window(window, |w| w.parent).ok().flatten();
        while let Some(parent) = w {
            let Ok((x, y, popup, next)) =
                with_window(parent, |p| (p.core.x, p.core.y, p.is_popup(), p.parent))
            else {
                break;
            };
            dx += x;
            dy += y;
            if !popup {
                break;
            }
            w = next;
        }
    }
    (dx, dy)
}

/// Convert popup-relative coordinates to global ones. Translation of
/// `SDL_RelativeToGlobalForWindow()`.
pub(crate) fn relative_to_global_for_window(
    window: WindowID,
    rel_x: i32,
    rel_y: i32,
) -> (i32, i32) {
    let (dx, dy) = popup_offset(window);
    (rel_x + dx, rel_y + dy)
}

/// Convert global coordinates to popup-relative ones. Translation of
/// `SDL_GlobalToRelativeForWindow()`.
#[allow(dead_code)] // (used by the video drivers)
pub(crate) fn global_to_relative_for_window(
    window: WindowID,
    abs_x: i32,
    abs_y: i32,
) -> (i32, i32) {
    let (dx, dy) = popup_offset(window);
    (abs_x - dx, abs_y - dy)
}

/// Translation of `SDL_GetDisplayForWindow()`.
pub(crate) fn display_for_window(window: WindowID) -> Result<DisplayID> {
    let window_props = with_window(window, |w| w.properties())?;

    /* sdl2-compat calls this function to get a display on which to make the window fullscreen,
     * so pass it the preferred fullscreen display ID in a property.
     */
    match core::display_for_fullscreen_window(window)? {
        0 => {
            let _ = window_props.remove(PROP_SDL2_COMPAT_WINDOW_PREFERRED_FULLSCREEN_DISPLAY);
        }
        fs_display => {
            let _ = window_props.set(
                PROP_SDL2_COMPAT_WINDOW_PREFERRED_FULLSCREEN_DISPLAY,
                fs_display as i64,
            );
        }
    }

    // An explicit fullscreen display overrides all
    let mut display_id = with_window(window, |w| {
        if w.flags().contains(WindowFlags::FULLSCREEN) {
            w.current_fullscreen_mode.display_id
        } else {
            0
        }
    })?;

    if display_id == 0 {
        display_id = core::display_for_window_position(window)?;
    }
    Ok(display_id)
}

/// The fullscreen mode a window would use (requested, or current while
/// fullscreen), as a match in its display's mode list. Translation of
/// `SDL_GetWindowFullscreenMode()`.
pub(crate) fn window_fullscreen_mode_match(window: WindowID) -> Result<Option<ModeMatch>> {
    let (popup, mode) = with_window(window, |w| {
        (
            w.is_popup(),
            if w.flags().contains(WindowFlags::FULLSCREEN) {
                w.current_fullscreen_mode
            } else {
                w.requested_fullscreen_mode
            },
        )
    })?;
    if popup {
        return Err(Error::new("Operation invalid on popup windows"));
    }
    Ok(display::fullscreen_mode_match(&mode))
}

/// The size of a window's client area in pixels. Translation of
/// `SDL_GetWindowSizeInPixels()`.
pub(crate) fn window_size_in_pixels(window: WindowID) -> Result<(i32, i32)> {
    let (w, h, fullscreen) = with_window(window, |w| {
        (
            w.core.w,
            w.core.h,
            w.flags().contains(WindowFlags::FULLSCREEN),
        )
    })?;

    if let Some(size) = driver()?.window_size_in_pixels(window) {
        return Ok(size);
    }

    let display_id = display_for_window(window)?;
    let mode = if fullscreen
        && window_fullscreen_mode_match(window)
            .ok()
            .flatten()
            .is_some()
    {
        display::current_display_mode(display_id)
    } else {
        display::desktop_display_mode(display_id)
    };
    Ok(match mode {
        Ok(mode) => (
            crate::stdlib::math::ceilf(w as f32 * mode.pixel_density) as i32,
            crate::stdlib::math::ceilf(h as f32 * mode.pixel_density) as i32,
        ),
        Err(_) => (w, h),
    })
}

/// Translation of `SDL_GetWindowPixelDensity()`.
pub(crate) fn window_pixel_density(window: WindowID) -> Result<f32> {
    let (window_w, _) = with_window(window, |w| (w.core.w, w.core.h))?;
    let mut pixel_density = 1.0;
    if let Ok((pixel_w, _)) = window_size_in_pixels(window) {
        pixel_density = pixel_w as f32 / window_w as f32;
    }
    Ok(pixel_density)
}

/// Translation of `SDL_MinimizeWindow()` (for internal callers).
pub(crate) fn minimize_window(window: WindowID) -> Result<()> {
    Window { id: window }.minimize()
}

/// Translation of `SDL_SyncWindow()` (for internal callers).
pub(crate) fn sync_window(window: WindowID) -> Result<()> {
    Window { id: window }.sync()
}

/// Translation of `SDL_DestroyWindow()` (for internal callers).
pub(crate) fn destroy_window(window: WindowID) {
    Window { id: window }.destroy();
}

/// Translation of `SDL_ShouldAllowTopmost()`.
#[allow(dead_code)] // (used by the video drivers)
pub(crate) fn should_allow_topmost() -> bool {
    hints::get_bool(hints::WINDOW_ALLOW_TOPMOST, true)
}

/// Translation of `SDL_ShouldRelinquishPopupFocus()`: where focus should
/// go when a popup goes away, and whether to move it.
#[allow(dead_code)] // (used by the video drivers)
pub(crate) fn should_relinquish_popup_focus(window: WindowID) -> Result<(Option<WindowID>, bool)> {
    let (mut focus, mut set_focus) = with_window(window, |w| {
        (w.parent, w.flags().contains(WindowFlags::INPUT_FOCUS))
    })?;

    // Find the highest level window, up to the toplevel parent, that isn't being hidden or destroyed, and can grab the keyboard focus.
    while let Some(f) = focus {
        let Ok((popup, skip, parent)) = with_window(f, |w| {
            (
                w.is_popup(),
                w.flags().contains(WindowFlags::NOT_FOCUSABLE)
                    || w.is_hiding
                    || w.core.is_destroying,
                w.parent,
            )
        }) else {
            break;
        };
        if !(popup && skip) {
            break;
        }
        focus = parent;

        // If some window in the chain currently had focus, set it to the new lowest-level window.
        if !set_focus {
            if let Some(f) = focus {
                set_focus = with_window(f, |w| w.flags().contains(WindowFlags::INPUT_FOCUS))
                    .unwrap_or(false);
            }
        }
    }

    Ok((focus, set_focus))
}

/// Translation of `SDL_ShouldFocusPopup()`.
#[allow(dead_code)] // (used by the video drivers)
pub(crate) fn should_focus_popup(window: WindowID) -> bool {
    let mut toplevel_parent = with_window(window, |w| w.parent).ok().flatten();
    while let Some(p) = toplevel_parent {
        match with_window(p, |w| (w.is_popup(), w.parent)) {
            Ok((true, parent)) => toplevel_parent = parent,
            _ => break,
        }
    }
    let Some(toplevel_parent) = toplevel_parent else {
        return true;
    };

    let current_focus = with_window(toplevel_parent, |w| w.keyboard_focus)
        .ok()
        .flatten();
    let mut found_higher_focus = false;

    /* Traverse the window tree from the currently focused window to the toplevel parent and see if we encounter
     * the new focus request. If the new window is found, a higher-level window already has focus.
     */
    let mut w = current_focus;
    while let Some(id) = w {
        if id == toplevel_parent {
            break;
        }
        if id == window {
            found_higher_focus = true;
            break;
        }
        w = with_window(id, |w| w.parent).ok().flatten();
    }

    !found_higher_focus || w == Some(toplevel_parent)
}

/// A handle to a window's framebuffer surface (from [`Window::surface`]).
///
/// Lock it to draw, then [`update`](WindowSurface::update) to show the
/// result; don't hold the lock while updating. The surface becomes stale
/// when the window is resized (updating it then fails); get a new one.
#[derive(Clone)]
pub struct WindowSurface {
    window: WindowID,
    surface: SharedSurface,
}

impl std::fmt::Debug for WindowSurface {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WindowSurface")
            .field("window", &self.window)
            .finish()
    }
}

impl WindowSurface {
    /// Lock the surface to read or draw.
    pub fn lock(&self) -> MutexGuard<'_, Surface<'static>> {
        self.surface.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Copy the whole surface to the screen. Translation of
    /// `SDL_UpdateWindowSurface()`.
    pub fn update(&self) -> Result<()> {
        Window { id: self.window }.update_surface()
    }

    /// Copy areas of the surface to the screen. Translation of
    /// `SDL_UpdateWindowSurfaceRects()`.
    pub fn update_rects(&self, rects: &[Rect]) -> Result<()> {
        Window { id: self.window }.update_surface_rects(rects)
    }

    /// The window this is the surface of.
    pub fn window(&self) -> Window {
        Window { id: self.window }
    }
}

/// Whether to show window surfaces through a texture of a GPU renderer.
/// Translation of `ShouldAttemptTextureFramebuffer()`.
fn should_attempt_texture_framebuffer(has_framebuffer: bool) -> bool {
    // If the driver doesn't support window framebuffers, always use the render API.
    if !has_framebuffer {
        return true;
    }

    // See if there's a hint override
    match hints::get(hints::FRAMEBUFFER_ACCELERATION).as_deref() {
        Some(hint) if !hint.is_empty() => {
            !(hint.starts_with('0')
                || hint.eq_ignore_ascii_case("false")
                || hint.eq_ignore_ascii_case(crate::render::SOFTWARE_RENDERER))
        }
        _ => core::caps().contains(DeviceCaps::SLOW_FRAMEBUFFER),
    }
}

/// Create the framebuffer of a window through a texture of a GPU renderer.
/// Translation of `SDL_CreateWindowTexture()`, whose renderer search finds
/// no GPU renderers yet (they aren't translated).
fn create_window_texture() -> Result<Surface<'static>> {
    // See if there's a render driver being requested
    let hint = hints::get(hints::FRAMEBUFFER_ACCELERATION).filter(|h| {
        !h.starts_with('0')
            && !h.starts_with('1')
            && !h.eq_ignore_ascii_case("true")
            && !h.eq_ignore_ascii_case("false")
            && !h.eq_ignore_ascii_case(crate::render::SOFTWARE_RENDERER)
    });
    let render_driver = hint
        .or_else(|| hints::get(hints::RENDER_DRIVER))
        .filter(|d| !d.eq_ignore_ascii_case(crate::render::SOFTWARE_RENDERER) && !d.is_empty());
    match render_driver {
        Some(name) => Err(Error::new(format!(
            "Couldn't find matching render driver ({name})"
        ))),
        None => Err(Error::new("No hardware accelerated renderers available")),
    }
}

impl Window {
    /// The handle of a window known to exist (no check).
    pub(crate) fn from_raw(id: WindowID) -> Window {
        Window { id }
    }

    /// Create a window with the specified dimensions and flags. Translation
    /// of `SDL_CreateWindow()`.
    pub fn create(title: &str, w: i32, h: i32, flags: WindowFlags) -> Result<Window> {
        Window::builder()
            .title(title)
            .size(w, h)
            .flags(flags)
            .build()
    }

    /// Options for a new window.
    pub fn builder() -> WindowBuilder {
        WindowBuilder::default()
    }

    /// Create a popup window (a tooltip or popup menu: `flags` must contain
    /// [`WindowFlags::TOOLTIP`] or [`WindowFlags::POPUP_MENU`]) at an offset
    /// from its parent. Translation of `SDL_CreatePopupWindow()`.
    pub fn create_popup(
        parent: &Window,
        offset_x: i32,
        offset_y: i32,
        w: i32,
        h: i32,
        flags: WindowFlags,
    ) -> Result<Window> {
        // Popups must specify either the tooltip or popup menu window flags
        if !flags.intersects(WindowFlags::TOOLTIP | WindowFlags::POPUP_MENU) {
            return Err(Error::new(
                "Popup windows must specify either the 'SDL_WINDOW_TOOLTIP' or the 'SDL_WINDOW_POPUP_MENU' flag",
            ));
        }

        Window::builder()
            .parent(parent)
            .position(offset_x, offset_y)
            .size(w, h)
            .flags(flags)
            .build()
    }

    /// Create a window from creation properties (`PROP_WINDOW_CREATE_*`),
    /// initializing video if needed. Translation of
    /// `SDL_CreateWindowWithProperties()`.
    pub fn create_with_properties(props: &Properties) -> Result<Window> {
        let title = props.get_string(PROP_WINDOW_CREATE_TITLE_STRING);
        let mut x = props
            .get_number(PROP_WINDOW_CREATE_X_NUMBER)
            .map_or(WINDOWPOS_UNDEFINED, |v| v as i32);
        let mut y = props
            .get_number(PROP_WINDOW_CREATE_Y_NUMBER)
            .map_or(WINDOWPOS_UNDEFINED, |v| v as i32);
        let mut w = props
            .get_number(PROP_WINDOW_CREATE_WIDTH_NUMBER)
            .unwrap_or(0) as i32;
        let mut h = props
            .get_number(PROP_WINDOW_CREATE_HEIGHT_NUMBER)
            .unwrap_or(0) as i32;
        let parent = props
            .get_any::<Window>(PROP_WINDOW_CREATE_PARENT_POINTER)
            .map(|w| w.id);
        let mut flags = window_flag_properties(props);
        let mut display_id: DisplayID = 0;
        let mut undefined_x = false;
        let mut undefined_y = false;
        let external_graphics_context = props
            .get_bool(PROP_WINDOW_CREATE_EXTERNAL_GRAPHICS_CONTEXT_BOOLEAN)
            .unwrap_or(false);

        if !initialized() {
            // Initialize the video system if needed
            crate::init::init(crate::init::InitFlags::VIDEO)?;

            if !initialized() {
                return Err(uninitialized_video());
            }
        }
        let driver = driver()?;
        let caps = core::caps();

        let parent_valid = parent.is_some_and(|p| with_window(p, |_| ()).is_ok());
        if flags.contains(WindowFlags::MODAL) && !parent_valid {
            return Err(Error::new("Modal windows must specify a parent window"));
        }

        if flags.intersects(WindowFlags::TOOLTIP | WindowFlags::POPUP_MENU) {
            if !caps.contains(DeviceCaps::HAS_POPUP_WINDOW_SUPPORT) {
                return Err(Error::unsupported());
            }

            // Tooltip and popup menu window must specify a parent window
            if !parent_valid {
                return Err(Error::new(
                    "Tooltip and popup menu windows must specify a parent window",
                ));
            }

            // Remove invalid flags
            flags &= !(WindowFlags::MINIMIZED
                | WindowFlags::MAXIMIZED
                | WindowFlags::FULLSCREEN
                | WindowFlags::BORDERLESS);
        }

        // Ensure no more than one of these flags is set
        let type_flags = flags
            & (WindowFlags::UTILITY
                | WindowFlags::TOOLTIP
                | WindowFlags::POPUP_MENU
                | WindowFlags::MODAL);
        if more_than_one(type_flags) {
            return Err(Error::new(format!(
                "Conflicting window type flags specified: 0x{:08x}",
                type_flags.0 as u32
            )));
        }

        // Make sure the display list is up to date for window placement
        let _ = driver.refresh_displays();

        // Some platforms can't create zero-sized windows
        if w < 1 {
            w = 1;
        }
        if h < 1 {
            h = 1;
        }

        if windowpos_is_undefined(x)
            || windowpos_is_undefined(y)
            || windowpos_is_centered(x)
            || windowpos_is_centered(y)
        {
            if (windowpos_is_undefined(x) || windowpos_is_centered(x)) && (x & 0xFFFF) != 0 {
                display_id = (x & 0xFFFF) as DisplayID;
            } else if (windowpos_is_undefined(y) || windowpos_is_centered(y)) && (y & 0xFFFF) != 0 {
                display_id = (y & 0xFFFF) as DisplayID;
            }
            if display_id == 0 || display_index(display_id).is_err() {
                display_id = primary_display().unwrap_or(0);
            }

            let mut bounds = display_usable_bounds(display_id).unwrap_or_default();
            if w > bounds.w || h > bounds.h {
                // This window is larger than the usable bounds, just center on the display
                bounds = display_bounds(display_id).unwrap_or(bounds);
            }
            if windowpos_is_centered(x) || windowpos_is_undefined(x) {
                if windowpos_is_undefined(x) {
                    undefined_x = true;
                }
                x = bounds.x + (bounds.w - w) / 2;
            }
            if windowpos_is_centered(y) || windowpos_is_undefined(y) {
                if windowpos_is_undefined(y) {
                    undefined_y = true;
                }
                y = bounds.y + (bounds.h - h) / 2;
            }
        }

        // ensure no more than one of these flags is set
        let graphics_flags = flags & GRAPHICS_FLAGS;
        if more_than_one(graphics_flags) {
            return Err(Error::new(format!(
                "Conflicting window graphics flags specified: 0x{:08x}",
                graphics_flags.0 as u32
            )));
        }

        // Some platforms have certain graphics backends enabled by default
        if graphics_flags.is_empty() && !external_graphics_context {
            flags |= default_graphics_backends();
        }

        if flags.contains(WindowFlags::OPENGL) {
            if !driver.implements_gl_contexts() {
                return Err(context_not_supported("OpenGL"));
            }
            super::gl::gl_load_library(None)?;
        }

        if flags.contains(WindowFlags::VULKAN) {
            if !driver.implements_vulkan_surfaces() {
                return Err(context_not_supported("Vulkan"));
            }
            super::vulkan::vulkan_load_library(None)?;
        }

        if flags.contains(WindowFlags::METAL) && !driver.implements_metal_views() {
            return Err(context_not_supported("Metal"));
        }

        let id = crate::utils::next_object_id();
        let mut data = WindowData::new(id);
        data.core.flags = WindowFlags::NONE;
        data.core.x = x;
        data.core.y = y;
        data.core.w = w;
        data.core.h = h;
        data.core.windowed = Rect::new(x, y, w, h);
        data.core.floating = Rect::new(x, y, w, h);
        data.core.undefined_x = undefined_x;
        data.core.undefined_y = undefined_y;
        data.core.pending_display_id = display_id;
        // (upstream links the window into the list after the display
        // queries below; it is linked first here so they can find it)
        with_device(|v| v.windows.insert(0, data))?;

        let display = display_for_window(id).ok();
        if let Some(hdr) = display.and_then(|d| core::with_display(d, |d| d.hdr)) {
            display::set_window_hdr_properties(id, hdr, false);
        }

        if flags.contains(WindowFlags::FULLSCREEN) || caps.contains(DeviceCaps::FULLSCREEN_ONLY) {
            let bounds = display_bounds(display.unwrap_or_else(|| primary_display().unwrap_or(0)))
                .unwrap_or_default();
            with_window(id, |w| {
                w.core.x = bounds.x;
                w.core.y = bounds.y;
                w.core.w = bounds.w;
                w.core.h = bounds.h;
                w.core.pending_flags |= WindowFlags::FULLSCREEN;
            })?;
            flags |= WindowFlags::FULLSCREEN;
        }

        let constrain_popup = props
            .get_bool(PROP_WINDOW_CREATE_CONSTRAIN_POPUP_BOOLEAN)
            .unwrap_or(true);
        with_window(id, |w| {
            w.core.flags = (flags & CREATE_FLAGS) | WindowFlags::HIDDEN;
            w.display_scale = 1.0;
            w.opacity = 1.0;
            w.core.is_destroying = false;
        })?;
        let display_id = display_for_window(id).unwrap_or(0);
        let fill_document = driver.implements_fill_document();
        with_window(id, |w| {
            w.core.display_id = display_id;
            w.external_graphics_context = external_graphics_context;
            w.constrain_popup = constrain_popup;

            if !fill_document {
                w.core.flags &= !WindowFlags::FILL_DOCUMENT; // not an error, just unsupported here, so remove the flag.
            }
        })?;

        // Set the parent before creation.
        update_window_hierarchy(id, parent);

        if let Some(Err(e)) = driver.create_window(id, props) {
            Window { id }.destroy();
            return Err(e);
        }

        /* Clear minimized if not on windows, only windows handles it at create rather than FinishWindowCreation,
         * but it's important or window focus will get broken on windows!
         */
        if !cfg!(windows) {
            with_window(id, |w| w.core.flags &= !WindowFlags::MINIMIZED)?;
        }

        let window = Window { id };
        if let Some(title) = title {
            let _ = window.set_title(&title);
        }
        finish_window_creation(id, flags);

        // Make sure window pixel size is up to date
        core::check_window_pixel_size_changed(id);

        Ok(window)
    }

    /// Try to reconfigure the window in place for different graphics
    /// flags. Translation of `SDL_ReconfigureWindowInternal()`.
    fn reconfigure_internal(&self, flags: WindowFlags) -> Result<()> {
        let window = self.id;
        let driver = driver()?;

        // (`ReconfigureWindow` missing: the caller recreates the window)
        let unsupported = || Error::new("Reconfiguring windows isn't supported");
        if driver
            .reconfigure_window(window, WindowFlags::NONE)
            .is_none()
        {
            return Err(unsupported());
        }

        let graphics_flags = flags & GRAPHICS_FLAGS;
        if more_than_one(graphics_flags) {
            return Err(Error::new("Conflicting window flags specified"));
        }

        if flags.contains(WindowFlags::OPENGL) && !driver.implements_gl_contexts() {
            return Err(context_not_supported("OpenGL"));
        }
        if flags.contains(WindowFlags::VULKAN) && !driver.implements_vulkan_surfaces() {
            return Err(context_not_supported("Vulkan"));
        }
        if flags.contains(WindowFlags::METAL) && !driver.implements_metal_views() {
            return Err(context_not_supported("Metal"));
        }

        let current = self.flags()?;
        if !current.contains(WindowFlags::EXTERNAL) {
            // Only attempt to reconfigure if the window has no existing graphics flags.
            if current.intersects(GRAPHICS_FLAGS) {
                return Err(unsupported());
            }
        } else {
            // Can't destroy and recreate an external window, so try our best to reconfigure it.
            // (done by the probe above)

            // Reload the GL/Vulkan libraries in case the profile changed.
            if current.contains(WindowFlags::OPENGL) {
                super::gl::gl_unload_library();
            }
            if current.contains(WindowFlags::VULKAN) {
                super::vulkan::vulkan_unload_library();
            }

            with_window(window, |w| w.core.flags &= !GRAPHICS_FLAGS)?;
        }

        let _ = self.destroy_surface();

        let mut loaded_opengl = false;
        let mut loaded_vulkan = false;
        if graphics_flags.contains(WindowFlags::OPENGL) {
            super::gl::gl_load_library(None)?;
            loaded_opengl = true;
        } else if graphics_flags.contains(WindowFlags::VULKAN) {
            super::vulkan::vulkan_load_library(None)?;
            loaded_vulkan = true;
        }

        // Try to reconfigure the window for the requested graphics flags.
        if let Some(Err(e)) = driver.reconfigure_window(window, graphics_flags) {
            if loaded_opengl {
                super::gl::gl_unload_library();
            }
            if loaded_vulkan {
                super::vulkan::vulkan_unload_library();
            }

            return Err(e);
        }

        with_window(window, |w| w.core.flags |= graphics_flags)
    }

    /// Destroy and recreate the native window with new flags. Translation
    /// of `SDL_RecreateWindow()`.
    pub(crate) fn recreate(&self, mut flags: WindowFlags) -> Result<()> {
        let window = self.id;
        let driver = driver()?;

        // ensure no more than one of these flags is set
        let graphics_flags = flags & GRAPHICS_FLAGS;
        if more_than_one(graphics_flags) {
            return Err(Error::new("Conflicting window flags specified"));
        }

        if flags.contains(WindowFlags::OPENGL) && !driver.implements_gl_contexts() {
            return Err(context_not_supported("OpenGL"));
        }
        if flags.contains(WindowFlags::VULKAN) && !driver.implements_vulkan_surfaces() {
            return Err(context_not_supported("Vulkan"));
        }
        if flags.contains(WindowFlags::METAL) && !driver.implements_metal_views() {
            return Err(context_not_supported("Metal"));
        }

        let current = self.flags()?;
        if current.contains(WindowFlags::EXTERNAL) {
            // Can't destroy and re-create external windows, hrm
            flags |= WindowFlags::EXTERNAL;
        } else {
            flags &= !WindowFlags::EXTERNAL;
        }

        // If this is a modal dialog, clear the modal status.
        if current.contains(WindowFlags::MODAL) {
            let _ = self.set_modal(false);
        }

        // Restore video mode, etc.
        if !current.contains(WindowFlags::EXTERNAL) {
            let restore_on_show = with_window(window, |w| w.restore_on_show)?;
            let _ = self.hide();
            with_window(window, |w| w.restore_on_show = restore_on_show)?;
        }

        // Tear down the old native window
        let _ = self.destroy_surface();

        let current = self.flags()?;
        let (need_gl_load, need_gl_unload) =
            if current.contains(WindowFlags::OPENGL) != flags.contains(WindowFlags::OPENGL) {
                (
                    flags.contains(WindowFlags::OPENGL),
                    !flags.contains(WindowFlags::OPENGL),
                )
            } else {
                let gl = current.contains(WindowFlags::OPENGL);
                (gl, gl)
            };
        let (need_vulkan_load, need_vulkan_unload) =
            if current.contains(WindowFlags::VULKAN) != flags.contains(WindowFlags::VULKAN) {
                (
                    flags.contains(WindowFlags::VULKAN),
                    !flags.contains(WindowFlags::VULKAN),
                )
            } else {
                let vulkan = current.contains(WindowFlags::VULKAN);
                (vulkan, vulkan)
            };

        if need_gl_unload {
            super::gl::gl_unload_library();
        }

        if need_vulkan_unload {
            super::vulkan::vulkan_unload_library();
        }

        if !flags.contains(WindowFlags::EXTERNAL) {
            let _ = driver.destroy_window(window);
        }

        let mut loaded_opengl = false;
        let mut loaded_vulkan = false;
        if need_gl_load {
            super::gl::gl_load_library(None)?;
            loaded_opengl = true;
        }

        if need_vulkan_load {
            super::vulkan::vulkan_load_library(None)?;
            loaded_vulkan = true;
        }

        with_window(window, |w| {
            w.core.flags = (flags & CREATE_FLAGS) | WindowFlags::HIDDEN;
            w.core.is_destroying = false;
        })?;

        if !flags.contains(WindowFlags::EXTERNAL) {
            /* Reset the window size to the original floating value, so the
             * recreated window has the proper base size.
             */
            with_window(window, |w| {
                let floating = w.core.floating;
                w.core.x = floating.x;
                w.core.y = floating.y;
                w.core.w = floating.w;
                w.core.h = floating.h;
                w.core.windowed = floating;
            })?;

            if let Some(Err(e)) = driver.create_window(window, &Properties::new()) {
                if loaded_opengl {
                    super::gl::gl_unload_library();
                    with_window(window, |w| w.core.flags &= !WindowFlags::OPENGL)?;
                }
                if loaded_vulkan {
                    super::vulkan::vulkan_unload_library();
                    with_window(window, |w| w.core.flags &= !WindowFlags::VULKAN)?;
                }
                return Err(e);
            }
        }

        if flags.contains(WindowFlags::EXTERNAL) {
            with_window(window, |w| w.core.flags |= WindowFlags::EXTERNAL)?;
        }

        let (has_title, icon, has_min, has_max, has_aspect, has_hit_test) =
            with_window(window, |w| {
                (
                    w.title.is_some(),
                    w.icon.clone(),
                    w.core.min_w != 0 || w.core.min_h != 0,
                    w.core.max_w != 0 || w.core.max_h != 0,
                    w.min_aspect > 0.0 || w.max_aspect > 0.0,
                    w.hit_test.is_some(),
                )
            })?;
        if has_title {
            let _ = driver.set_window_title(window);
        }

        if let Some(icon) = icon {
            let _ = driver.set_window_icon(window, &icon);
        }

        if has_min {
            let _ = driver.set_window_minimum_size(window);
        }

        if has_max {
            let _ = driver.set_window_maximum_size(window);
        }

        if has_aspect {
            let _ = driver.set_window_aspect_ratio(window);
        }

        if has_hit_test {
            let _ = driver.set_window_hit_test(window, true);
        }

        finish_window_creation(window, flags);

        Ok(())
    }

    /// Reconfigure the window for new graphics flags in place if the backend
    /// can, otherwise recreate it. Translation of `SDL_ReconfigureWindow()`.
    #[allow(dead_code)] // (used by the renderers)
    pub(crate) fn reconfigure(&self, flags: WindowFlags) -> Result<()> {
        // Try to reconfigure the window for the desired flags first, before completely destroying and recreating it.
        if self.reconfigure_internal(flags).is_err() {
            return self.recreate(flags);
        }
        Ok(())
    }

    /// The numeric ID of the window. Translation of `SDL_GetWindowID()`.
    pub fn id(&self) -> WindowID {
        self.id
    }

    /// The window with an ID, if it exists. Translation of
    /// `SDL_GetWindowFromID()`.
    pub fn from_id(id: WindowID) -> Result<Window> {
        if !initialized() {
            return Err(uninitialized_video());
        }
        if id != 0 && with_window(id, |_| ()).is_ok() {
            return Ok(Window { id });
        }
        Err(Error::new("Invalid window ID"))
    }

    /// Whether the window still exists.
    pub fn is_valid(&self) -> bool {
        with_window(self.id, |_| ()).is_ok()
    }

    /// The parent of the window. Translation of `SDL_GetWindowParent()`.
    pub fn parent(&self) -> Result<Option<Window>> {
        with_window(self.id, |w| w.parent.map(|id| Window { id }))
    }

    /// The properties associated with the window. Translation of
    /// `SDL_GetWindowProperties()`.
    pub fn properties(&self) -> Result<Properties> {
        with_window(self.id, |w| w.properties())
    }

    /// The window flags, with pending ones. Translation of
    /// `SDL_GetWindowFlags()`.
    pub fn flags(&self) -> Result<WindowFlags> {
        with_window(self.id, |w| w.core.flags | w.core.pending_flags)
    }

    /// Fail on popup windows (`CHECK_WINDOW_NOT_POPUP`).
    fn check_not_popup(&self) -> Result<()> {
        if with_window(self.id, |w| w.is_popup())? {
            return Err(Error::new("Operation invalid on popup windows"));
        }
        Ok(())
    }

    /// Set the title of the window, in UTF-8 format. Translation of
    /// `SDL_SetWindowTitle()`.
    pub fn set_title(&self, title: &str) -> Result<()> {
        self.check_not_popup()?;

        let changed = with_window(self.id, |w| {
            if w.title.as_deref() == Some(title) {
                false
            } else {
                w.title = Some(title.to_owned());
                true
            }
        })?;
        if changed {
            let _ = driver()?.set_window_title(self.id);
        }
        Ok(())
    }

    /// The title of the window. Translation of `SDL_GetWindowTitle()`.
    pub fn title(&self) -> Result<String> {
        with_window(self.id, |w| w.title.clone().unwrap_or_default())
    }

    /// Set the icon for the window (converted to ARGB8888). Translation of
    /// `SDL_SetWindowIcon()`.
    pub fn set_icon(&self, icon: &Surface<'_>) -> Result<()> {
        with_window(self.id, |w| w.icon = None)?;

        // Convert the icon into ARGB8888
        let icon = Arc::new(icon.convert(PixelFormat::ARGB8888)?);
        with_window(self.id, |w| w.icon = Some(icon.clone()))?;

        driver()?
            .set_window_icon(self.id, &icon)
            .unwrap_or_else(|| Err(Error::unsupported()))
    }

    /// Request that the window's position be set: coordinates, or
    /// [`WINDOWPOS_CENTERED`]/[`WINDOWPOS_UNDEFINED`]; popup coordinates are
    /// relative to the parent. The move may be asynchronous (see
    /// [`Window::sync`]). Translation of `SDL_SetWindowPosition()`.
    ///
    /// [`WINDOWPOS_CENTERED`]: super::sysvideo::WINDOWPOS_CENTERED
    pub fn set_position(&self, mut x: i32, mut y: i32) -> Result<()> {
        let window = self.id;
        let (w, h, windowed) = with_window(window, |w| {
            (
                if w.core.last_size_pending {
                    w.pending.w
                } else {
                    w.core.windowed.w
                },
                if w.core.last_size_pending {
                    w.pending.h
                } else {
                    w.core.windowed.h
                },
                w.core.windowed,
            )
        })?;

        let original_display_id = display_for_window(window)?;
        with_window(window, |w| w.core.pending_display_id = 0)?;

        if windowpos_is_undefined(x) {
            x = windowed.x;
        }
        if windowpos_is_undefined(y) {
            y = windowed.y;
        }
        if windowpos_is_centered(x) || windowpos_is_centered(y) {
            let mut display_id = original_display_id;

            if windowpos_is_centered(x) && (x & 0xFFFF) != 0 {
                display_id = (x & 0xFFFF) as DisplayID;
            } else if windowpos_is_centered(y) && (y & 0xFFFF) != 0 {
                display_id = (y & 0xFFFF) as DisplayID;
            }
            if display_id == 0 || display_index(display_id).is_err() {
                display_id = primary_display().unwrap_or(0);
            }

            with_window(window, |w| w.core.pending_display_id = display_id)?;

            let bounds = match display_usable_bounds(display_id) {
                Ok(bounds) if w <= bounds.w && h <= bounds.h => bounds,
                _ => display_bounds(display_id)?,
            };
            if windowpos_is_centered(x) {
                x = bounds.x + (bounds.w - w) / 2;
            }
            if windowpos_is_centered(y) {
                y = bounds.y + (bounds.h - h) / 2;
            }
        } else {
            /* See if the requested window position matches the origin of any displays and set
             * the pending fullscreen display ID if it does. This needs to be set early in case
             * the window is prevented from moving to the exact origin due to struts.
             */
            let at_origin = display::display_at_origin(x, y);
            with_window(window, |w| w.core.pending_display_id = at_origin)?;
        }

        let driver = driver()?;
        let has_sync = driver.implements_sync_window();
        with_window(window, |w| {
            w.pending.x = x;
            w.pending.y = y;

            /* Windows are placed at the coordinates received while in fullscreen after leaving fullscreen.
             * Asynchronous backends need special handling in this case.
             */
            if !has_sync && w.flags().contains(WindowFlags::FULLSCREEN) {
                w.core.floating.x = x;
                w.core.windowed.x = x;
                w.core.floating.y = y;
                w.core.windowed.y = y;
            }
            w.core.undefined_x = false;
            w.core.undefined_y = false;
            w.core.last_position_pending = true;
        })?;

        match driver.set_window_position(window) {
            Some(result) => {
                if result.is_ok() {
                    sync_if_required(window);
                }
                result
            }
            None => Err(Error::unsupported()),
        }
    }

    /// The position of the window (a fullscreen window is at its display's
    /// origin). Translation of `SDL_GetWindowPosition()`.
    pub fn position(&self) -> Result<(i32, i32)> {
        let (flags, pending, x, y, last_position_pending) = with_window(self.id, |w| {
            (
                w.flags(),
                w.pending,
                w.core.x,
                w.core.y,
                w.core.last_position_pending,
            )
        })?;

        // On newer MacBooks, the fullscreen window might be placed below the camera notch, so use the actual window position
        // Fullscreen windows are always at their display's origin
        let use_display_origin =
            !cfg!(target_os = "macos") && flags.contains(WindowFlags::FULLSCREEN);
        if use_display_origin {
            /* Find the window's monitor and update to the
            monitor offset. */
            let display_id = display_for_window(self.id)?;
            if display_id != 0 {
                let bounds = display_bounds(display_id).unwrap_or_default();
                return Ok((bounds.x, bounds.y));
            }
            Ok((0, 0))
        } else {
            let use_pending = flags.contains(WindowFlags::HIDDEN) && last_position_pending;
            Ok(if use_pending {
                (pending.x, pending.y)
            } else {
                (x, y)
            })
        }
    }

    /// Set the border state of the window. Translation of
    /// `SDL_SetWindowBordered()`.
    pub fn set_bordered(&self, bordered: bool) -> Result<()> {
        self.check_not_popup()?;
        let driver = driver()?;
        if !driver.implements_window_op(WindowOp::SetBordered) {
            return Err(Error::unsupported());
        }

        let changed = with_window(self.id, |w| {
            let have = !w.flags().contains(WindowFlags::BORDERLESS);
            if bordered != have {
                w.core.flags.set(WindowFlags::BORDERLESS, !bordered);
                true
            } else {
                false
            }
        })?;
        if changed {
            let _ = driver.set_window_bordered(self.id, bordered);
        }
        Ok(())
    }

    /// Set the user-resizable state of the window. Translation of
    /// `SDL_SetWindowResizable()`.
    pub fn set_resizable(&self, resizable: bool) -> Result<()> {
        self.check_not_popup()?;
        let driver = driver()?;
        if !driver.implements_window_op(WindowOp::SetResizable) {
            return Err(Error::unsupported());
        }

        let changed = with_window(self.id, |w| {
            let have = w.flags().contains(WindowFlags::RESIZABLE);
            if resizable != have {
                if resizable {
                    w.core.flags |= WindowFlags::RESIZABLE;
                } else {
                    w.core.flags &= !WindowFlags::RESIZABLE;
                    w.core.windowed = w.core.floating;
                }
                true
            } else {
                false
            }
        })?;
        if changed {
            let _ = driver.set_window_resizable(self.id, resizable);
        }
        Ok(())
    }

    /// Set the window to always be above the others. Translation of
    /// `SDL_SetWindowAlwaysOnTop()`.
    pub fn set_always_on_top(&self, on_top: bool) -> Result<()> {
        self.check_not_popup()?;
        let driver = driver()?;
        if !driver.implements_window_op(WindowOp::SetAlwaysOnTop) {
            return Err(Error::unsupported());
        }

        let changed = with_window(self.id, |w| {
            let have = w.flags().contains(WindowFlags::ALWAYS_ON_TOP);
            if on_top != have {
                w.core.flags.set(WindowFlags::ALWAYS_ON_TOP, on_top);
                true
            } else {
                false
            }
        })?;
        if changed {
            let _ = driver.set_window_always_on_top(self.id, on_top);
        }
        Ok(())
    }

    /// Request that the size of the window's client area be set (within its
    /// aspect ratio and size limits). Translation of `SDL_SetWindowSize()`.
    pub fn set_size(&self, mut w: i32, mut h: i32) -> Result<()> {
        let window = self.id;
        let (min_aspect, max_aspect, min_w, max_w, min_h, max_h) = with_window(window, |win| {
            (
                win.min_aspect,
                win.max_aspect,
                win.core.min_w,
                win.core.max_w,
                win.core.min_h,
                win.core.max_h,
            )
        })?;

        if w <= 0 {
            return Err(Error::invalid_param("w"));
        }
        if h <= 0 {
            return Err(Error::invalid_param("h"));
        }

        // It is possible for the aspect ratio constraints to not satisfy the size constraints.
        // The size constraints will override the aspect ratio constraints so we will apply the
        // the aspect ratio constraints first
        let new_aspect = w as f32 / h as f32;
        if max_aspect > 0.0 && new_aspect > max_aspect {
            w = crate::stdlib::math::roundf(h as f32 * max_aspect) as i32;
        } else if min_aspect > 0.0 && new_aspect < min_aspect {
            h = crate::stdlib::math::roundf(w as f32 / min_aspect) as i32;
        }

        // Make sure we don't exceed any window size limits
        if min_w != 0 && w < min_w {
            w = min_w;
        }
        if max_w != 0 && w > max_w {
            w = max_w;
        }
        if min_h != 0 && h < min_h {
            h = min_h;
        }
        if max_h != 0 && h > max_h {
            h = max_h;
        }

        with_window(window, |win| {
            win.core.last_size_pending = true;
            win.pending.w = w;
            win.pending.h = h;
        })?;

        match driver()?.set_window_size(window) {
            Some(()) => {
                sync_if_required(window);
                Ok(())
            }
            None => Err(Error::unsupported()),
        }
    }

    /// The size of the window's client area. Translation of
    /// `SDL_GetWindowSize()`.
    pub fn size(&self) -> Result<(i32, i32)> {
        with_window(self.id, |w| (w.core.w, w.core.h))
    }

    /// Limit the aspect ratio (width / height) of the window's client area
    /// (0.0 for no limit). Translation of `SDL_SetWindowAspectRatio()`.
    pub fn set_aspect_ratio(&self, min_aspect: f32, max_aspect: f32) -> Result<()> {
        with_window(self.id, |w| {
            w.min_aspect = min_aspect;
            w.max_aspect = max_aspect;
        })?;

        let _ = driver()?.set_window_aspect_ratio(self.id);

        // Ensure that window has the correct aspect ratio
        let (w, h) = self.pending_or_floating_size()?;
        self.set_size(w, h)
    }

    /// The pending size if a resize is pending, otherwise the floating size.
    fn pending_or_floating_size(&self) -> Result<(i32, i32)> {
        with_window(self.id, |w| {
            if w.core.last_size_pending {
                (w.pending.w, w.pending.h)
            } else {
                (w.core.floating.w, w.core.floating.h)
            }
        })
    }

    /// The aspect ratio limits: (min, max). Translation of
    /// `SDL_GetWindowAspectRatio()`.
    pub fn aspect_ratio(&self) -> Result<(f32, f32)> {
        with_window(self.id, |w| (w.min_aspect, w.max_aspect))
    }

    /// The size of the window's borders (decorations): (top, left, bottom,
    /// right). Translation of `SDL_GetWindowBordersSize()`.
    pub fn borders_size(&self) -> Result<(i32, i32, i32, i32)> {
        with_window(self.id, |_| ())?;
        driver()?
            .window_borders_size(self.id)
            .unwrap_or_else(|| Err(Error::unsupported()))
    }

    /// The size of the window's client area in pixels. Translation of
    /// `SDL_GetWindowSizeInPixels()`.
    pub fn size_in_pixels(&self) -> Result<(i32, i32)> {
        window_size_in_pixels(self.id)
    }

    /// Set the minimum size of the window's client area (0 for no limit).
    /// Translation of `SDL_SetWindowMinimumSize()`.
    pub fn set_minimum_size(&self, min_w: i32, min_h: i32) -> Result<()> {
        let (max_w, max_h) = with_window(self.id, |w| (w.core.max_w, w.core.max_h))?;
        if min_w < 0 {
            return Err(Error::invalid_param("min_w"));
        }
        if min_h < 0 {
            return Err(Error::invalid_param("min_h"));
        }

        if (max_w != 0 && min_w > max_w) || (max_h != 0 && min_h > max_h) {
            return Err(Error::new(
                "SDL_SetWindowMinimumSize(): Tried to set minimum size larger than maximum size",
            ));
        }

        with_window(self.id, |w| {
            w.core.min_w = min_w;
            w.core.min_h = min_h;
        })?;

        let _ = driver()?.set_window_minimum_size(self.id);

        // Ensure that window is not smaller than minimal size
        let (w, h) = self.pending_or_floating_size()?;
        self.set_size(w, h)
    }

    /// The minimum size of the window's client area. Translation of
    /// `SDL_GetWindowMinimumSize()`.
    pub fn minimum_size(&self) -> Result<(i32, i32)> {
        with_window(self.id, |w| (w.core.min_w, w.core.min_h))
    }

    /// Set the maximum size of the window's client area (0 for no limit).
    /// Translation of `SDL_SetWindowMaximumSize()`.
    pub fn set_maximum_size(&self, max_w: i32, max_h: i32) -> Result<()> {
        let (min_w, min_h) = with_window(self.id, |w| (w.core.min_w, w.core.min_h))?;
        if max_w < 0 {
            return Err(Error::invalid_param("max_w"));
        }
        if max_h < 0 {
            return Err(Error::invalid_param("max_h"));
        }

        if (max_w != 0 && max_w < min_w) || (max_h != 0 && max_h < min_h) {
            return Err(Error::new(
                "SDL_SetWindowMaximumSize(): Tried to set maximum size smaller than minimum size",
            ));
        }

        with_window(self.id, |w| {
            w.core.max_w = max_w;
            w.core.max_h = max_h;
        })?;

        let _ = driver()?.set_window_maximum_size(self.id);

        // Ensure that window is not larger than maximal size
        let (w, h) = self.pending_or_floating_size()?;
        self.set_size(w, h)
    }

    /// The maximum size of the window's client area. Translation of
    /// `SDL_GetWindowMaximumSize()`.
    pub fn maximum_size(&self) -> Result<(i32, i32)> {
        with_window(self.id, |w| (w.core.max_w, w.core.max_h))
    }

    /// Show the window (and the children it hid). Translation of
    /// `SDL_ShowWindow()`.
    pub fn show(&self) -> Result<()> {
        let window = self.id;
        let (hidden, parent) = with_window(window, |w| {
            (w.flags().contains(WindowFlags::HIDDEN), w.parent)
        })?;

        if !hidden {
            return Ok(());
        }

        // If the parent is hidden, set the flag to restore this when the parent is shown
        if let Some(parent) = parent {
            if with_window(parent, |p| p.flags().contains(WindowFlags::HIDDEN)).unwrap_or(false) {
                with_window(window, |w| w.restore_on_show = true)?;
                return Ok(());
            }
        }

        if driver()?.show_window(window).is_none() {
            mouse::set_mouse_focus(Some(window));
            let _ = keyboard::set_keyboard_focus(Some(window));
        }
        send_window_event(window, EventType::WINDOW_SHOWN, 0, 0);

        // Restore child windows
        let children = with_window(window, |w| w.children.clone()).unwrap_or_default();
        for child in children {
            let Ok((restore_on_show, hidden)) = with_window(child, |c| {
                (c.restore_on_show, c.flags().contains(WindowFlags::HIDDEN))
            }) else {
                continue;
            };
            if !restore_on_show && hidden {
                break;
            }
            let _ = Window { id: child }.show();
            let _ = with_window(child, |c| c.restore_on_show = false);
        }
        Ok(())
    }

    /// Hide the window (and its children). Translation of `SDL_HideWindow()`.
    pub fn hide(&self) -> Result<()> {
        let window = self.id;
        let hidden = with_window(window, |w| w.flags().contains(WindowFlags::HIDDEN))?;

        if hidden {
            with_window(window, |w| w.restore_on_show = false)?;
            return Ok(());
        }

        // Hide all child windows
        let children = with_window(window, |w| w.children.clone()).unwrap_or_default();
        for child in children {
            if with_window(child, |c| c.flags().contains(WindowFlags::HIDDEN)).unwrap_or(true) {
                break;
            }
            let _ = Window { id: child }.hide();
            let _ = with_window(child, |c| c.restore_on_show = true);
        }

        // Store the flags for restoration later.
        let pending_mask = WindowFlags::MAXIMIZED
            | WindowFlags::MINIMIZED
            | WindowFlags::FULLSCREEN
            | WindowFlags::KEYBOARD_GRABBED
            | WindowFlags::MOUSE_GRABBED;
        with_window(window, |w| {
            w.core.pending_flags = w.flags() & pending_mask;
            w.is_hiding = true;
        })?;
        if driver()?.hide_window(window).is_none() {
            mouse::set_mouse_focus(None);
            let _ = keyboard::set_keyboard_focus(None);
        }
        let _ = with_window(window, |w| w.is_hiding = false);
        send_window_event(window, EventType::WINDOW_HIDDEN, 0, 0);
        Ok(())
    }

    /// Request that the window be raised above the others and get input
    /// focus. Translation of `SDL_RaiseWindow()`.
    pub fn raise(&self) -> Result<()> {
        if with_window(self.id, |w| w.flags().contains(WindowFlags::HIDDEN))? {
            return Ok(());
        }
        let _ = driver()?.raise_window(self.id);
        Ok(())
    }

    /// Request that the window be made as large as possible (it must be
    /// resizable). Translation of `SDL_MaximizeWindow()`.
    pub fn maximize(&self) -> Result<()> {
        self.check_not_popup()?;
        let driver = driver()?;
        if !driver.implements_window_op(WindowOp::Maximize) {
            return Err(Error::unsupported());
        }

        let flags = with_window(self.id, |w| w.flags())?;
        if !flags.contains(WindowFlags::RESIZABLE) {
            return Err(Error::new(
                "A window without the 'SDL_WINDOW_RESIZABLE' flag can't be maximized",
            ));
        }

        if flags.contains(WindowFlags::HIDDEN) {
            with_window(self.id, |w| w.core.pending_flags |= WindowFlags::MAXIMIZED)?;
            return Ok(());
        }

        let _ = driver.maximize_window(self.id);
        sync_if_required(self.id);
        Ok(())
    }

    /// Request that the window be minimized to an iconic representation.
    /// Translation of `SDL_MinimizeWindow()`.
    pub fn minimize(&self) -> Result<()> {
        self.check_not_popup()?;
        let driver = driver()?;
        if !driver.implements_window_op(WindowOp::Minimize) {
            return Err(Error::unsupported());
        }

        if with_window(self.id, |w| w.flags().contains(WindowFlags::HIDDEN))? {
            with_window(self.id, |w| w.core.pending_flags |= WindowFlags::MINIMIZED)?;
            return Ok(());
        }

        let _ = driver.minimize_window(self.id);
        sync_if_required(self.id);
        Ok(())
    }

    /// Request that the size and position of a minimized or maximized window
    /// be restored. Translation of `SDL_RestoreWindow()`.
    pub fn restore(&self) -> Result<()> {
        self.check_not_popup()?;
        let driver = driver()?;
        if !driver.implements_window_op(WindowOp::Restore) {
            return Err(Error::unsupported());
        }

        if with_window(self.id, |w| w.flags().contains(WindowFlags::HIDDEN))? {
            with_window(self.id, |w| {
                w.core.pending_flags &= !(WindowFlags::MAXIMIZED | WindowFlags::MINIMIZED)
            })?;
            return Ok(());
        }

        let _ = driver.restore_window(self.id);
        sync_if_required(self.id);
        Ok(())
    }

    /// Request that the window's fullscreen state be changed (fullscreen
    /// desktop, or the mode of [`Window::set_fullscreen_mode`]).
    /// Translation of `SDL_SetWindowFullscreen()`.
    pub fn set_fullscreen(&self, fullscreen: bool) -> Result<()> {
        self.check_not_popup()?;

        if with_window(self.id, |w| w.flags().contains(WindowFlags::HIDDEN))? {
            with_window(self.id, |w| {
                w.core
                    .pending_flags
                    .set(WindowFlags::FULLSCREEN, fullscreen)
            })?;
            return Ok(());
        }

        if fullscreen {
            // Set the current fullscreen mode to the desired mode
            with_window(self.id, |w| {
                w.current_fullscreen_mode = w.requested_fullscreen_mode
            })?;
        }

        let result = update_fullscreen_mode(
            self.id,
            if fullscreen {
                FullscreenOp::Enter
            } else {
                FullscreenOp::Leave
            },
            true,
        );

        if !fullscreen || result.is_err() {
            // Clear the current fullscreen mode.
            let _ = with_window(self.id, |w| {
                w.current_fullscreen_mode = DisplayMode::default()
            });
        }

        if result.is_ok() {
            sync_if_required(self.id);
        }

        result
    }

    /// Block until any pending window state is finalized. Translation of
    /// `SDL_SyncWindow()`.
    pub fn sync(&self) -> Result<()> {
        with_window(self.id, |_| ())?;
        driver()?.sync_window(self.id).unwrap_or(Ok(()))
    }

    /// Set the mode to use for exclusive fullscreen (`None`: fullscreen
    /// desktop). Translation of `SDL_SetWindowFullscreenMode()`.
    pub fn set_fullscreen_mode(&self, mode: Option<&DisplayMode>) -> Result<()> {
        self.check_not_popup()?;

        let requested = match mode {
            Some(mode) => {
                if display::fullscreen_mode_match(mode).is_none() {
                    return Err(Error::new("Invalid fullscreen display mode"));
                }

                // Save the mode so we can look up the closest match later
                *mode
            }
            None => DisplayMode::default(),
        };

        /* Copy to the current mode now, in case an asynchronous fullscreen window request
         * is in progress. It will be overwritten if a new request is made.
         */
        let visible = with_window(self.id, |w| {
            w.requested_fullscreen_mode = requested;
            w.current_fullscreen_mode = requested;
            w.fullscreen_visible()
        })?;

        if visible {
            let _ = update_fullscreen_mode(self.id, FullscreenOp::Update, true);
            sync_if_required(self.id);
        }

        Ok(())
    }

    /// The exclusive fullscreen mode of the window (`None`: fullscreen
    /// desktop). Translation of `SDL_GetWindowFullscreenMode()`.
    pub fn fullscreen_mode(&self) -> Result<Option<DisplayMode>> {
        Ok(window_fullscreen_mode_match(self.id)?.map(|m| m.mode))
    }

    /// The raw ICC profile data for the screen the window is on.
    /// Translation of `SDL_GetWindowICCProfile()`.
    pub fn icc_profile(&self) -> Result<Vec<u8>> {
        driver()?
            .window_icc_profile(self.id)
            .unwrap_or_else(|| Err(Error::unsupported()))
    }

    /// The pixel format of the display the window is on. Translation of
    /// `SDL_GetWindowPixelFormat()`.
    pub fn pixel_format(&self) -> Result<PixelFormat> {
        let display_id = display_for_window(self.id)?;
        Ok(display::current_display_mode(display_id).map_or(PixelFormat::UNKNOWN, |m| m.format))
    }

    /// The display the window is on (its fullscreen display while
    /// fullscreen). Translation of `SDL_GetDisplayForWindow()`.
    pub fn display(&self) -> Result<DisplayID> {
        display_for_window(self.id)
    }

    /// The ratio of pixels to screen coordinates of the window.
    /// Translation of `SDL_GetWindowPixelDensity()`.
    pub fn pixel_density(&self) -> Result<f32> {
        window_pixel_density(self.id)
    }

    /// The content display scale of the window: its pixel density times
    /// its display's content scale. Translation of
    /// `SDL_GetWindowDisplayScale()`.
    pub fn display_scale(&self) -> Result<f32> {
        with_window(self.id, |w| w.display_scale)
    }

    /// Whether the window has a surface. Translation of
    /// `SDL_WindowHasSurface()`.
    pub fn has_surface(&self) -> Result<bool> {
        with_window(self.id, |w| w.surface.is_some())
    }

    /// Create the window's framebuffer. Translation of
    /// `SDL_CreateWindowFramebuffer()`.
    fn create_framebuffer(&self) -> Result<Surface<'static>> {
        let (w, h) = self.size_in_pixels()?;
        let driver = driver()?;

        // (whether the backend implements framebuffers)
        let has_framebuffer = driver.implements_window_framebuffer();

        /* This will switch the video backend from using a software surface to
        using a GPU texture through the 2D render API, if we think this would
        be more efficient. This only checks once, on demand. */
        if !with_device(|v| v.checked_texture_framebuffer)? {
            if should_attempt_texture_framebuffer(has_framebuffer) {
                /* !!! FIXME: if this failed halfway (made renderer, failed to make texture, etc),
                !!! FIXME:  we probably need to clean this up so it doesn't interfere with
                !!! FIXME:  a software fallback at the system level (can we blit to an
                !!! FIXME:  OpenGL window? etc). */
                let _ = create_window_texture();
            }

            with_device(|v| v.checked_texture_framebuffer = true)?; // don't check this again.
        }

        if !has_framebuffer {
            return Err(Error::new("Window framebuffer support not available"));
        }

        driver
            .create_window_framebuffer(self.id, w, h)
            .unwrap_or_else(|| Err(Error::new("Window framebuffer support not available")))
    }

    /// The window's framebuffer surface, for 2D drawing without a renderer
    /// (created on first use, and again after the window changes size).
    /// Translation of `SDL_GetWindowSurface()`.
    pub fn surface(&self) -> Result<WindowSurface> {
        let current = with_window(self.id, |w| {
            if w.surface_valid {
                w.surface.clone()
            } else {
                w.surface = None;
                None
            }
        })?;
        let surface = match current {
            Some(surface) => surface,
            None => {
                let surface = self.create_framebuffer()?;
                let shared = with_window(self.id, |w| {
                    // We may have gone recursive and already created the surface
                    let shared = w
                        .surface
                        .get_or_insert_with(|| Arc::new(Mutex::new(surface)))
                        .clone();
                    w.surface_valid = true;
                    shared
                })?;
                shared
            }
        };
        Ok(WindowSurface {
            window: self.id,
            surface,
        })
    }

    /// Whether `surface` is the window's current, valid surface
    /// (`window->surface_valid` for the surface the caller holds).
    pub(crate) fn is_current_surface(&self, surface: &WindowSurface) -> bool {
        with_window(self.id, |w| {
            w.surface_valid
                && w.surface
                    .as_ref()
                    .is_some_and(|s| Arc::ptr_eq(s, &surface.surface))
        })
        .unwrap_or(false)
    }

    /// Toggle VSync for the window surface (0 off, 1 on, -1 adaptive).
    /// Translation of `SDL_SetWindowSurfaceVSync()`.
    pub fn set_surface_vsync(&self, vsync: i32) -> Result<()> {
        with_window(self.id, |_| ())?;
        driver()?
            .set_window_framebuffer_vsync(self.id, vsync)
            .unwrap_or_else(|| Err(Error::unsupported()))
    }

    /// The VSync of the window surface. Translation of
    /// `SDL_GetWindowSurfaceVSync()`.
    pub fn surface_vsync(&self) -> Result<i32> {
        with_window(self.id, |_| ())?;
        driver()?
            .window_framebuffer_vsync(self.id)
            .unwrap_or_else(|| Err(Error::unsupported()))
    }

    /// Copy the window surface to the screen. Translation of
    /// `SDL_UpdateWindowSurface()`.
    pub fn update_surface(&self) -> Result<()> {
        let (w, h) = self.size_in_pixels()?;
        self.update_surface_rects(&[Rect::new(0, 0, w, h)])
    }

    /// Copy areas of the window surface to the screen. Translation of
    /// `SDL_UpdateWindowSurfaceRects()`.
    pub fn update_surface_rects(&self, rects: &[Rect]) -> Result<()> {
        let surface = with_window(self.id, |w| {
            if w.surface_valid {
                w.surface.clone()
            } else {
                None
            }
        })?;
        let Some(surface) = surface else {
            return Err(Error::new(
                "Window surface is invalid, please call SDL_GetWindowSurface() to get a new surface",
            ));
        };

        crate::sdl_assert!(with_device(|v| v.checked_texture_framebuffer).unwrap_or(false)); // we should have done this before we had a valid surface.

        let guard = surface
            .try_lock()
            .map_err(|_| Error::new("The window surface is locked"))?;
        driver()?
            .update_window_framebuffer(self.id, &guard, rects)
            .unwrap_or_else(|| Err(Error::new("Window framebuffer support not available")))
    }

    /// Destroy the window surface. Translation of `SDL_DestroyWindowSurface()`.
    pub fn destroy_surface(&self) -> Result<()> {
        with_window(self.id, |w| {
            w.surface = None;
            w.surface_valid = false;
        })?;

        if with_device(|v| v.checked_texture_framebuffer)? {
            // never checked? No framebuffer to destroy. Don't risk calling the wrong implementation.
            let _ = driver()?.destroy_window_framebuffer(self.id);
        }
        Ok(())
    }

    /// Set the opacity of the window (clamped to 0.0..=1.0). Translation of
    /// `SDL_SetWindowOpacity()`.
    pub fn set_opacity(&self, opacity: f32) -> Result<()> {
        with_window(self.id, |_| ())?;
        let opacity = opacity.clamp(0.0, 1.0);

        let result = driver()?
            .set_window_opacity(self.id, opacity)
            .unwrap_or_else(|| Err(Error::unsupported()));
        if result.is_ok() {
            with_window(self.id, |w| w.opacity = opacity)?;
        }
        result
    }

    /// The opacity of the window. Translation of `SDL_GetWindowOpacity()`.
    pub fn opacity(&self) -> Result<f32> {
        with_window(self.id, |w| w.opacity)
    }

    /// Set (or, with `None`, clear) the parent of the window. Translation
    /// of `SDL_SetWindowParent()`.
    pub fn set_parent(&self, parent: Option<&Window>) -> Result<()> {
        self.check_not_popup()?;

        if let Some(parent) = parent {
            parent.check_not_popup()?;
        }

        if parent == Some(self) {
            return Err(Error::new("Cannot set the parent of a window to itself."));
        }

        let driver = driver()?;
        if !driver.implements_window_op(WindowOp::SetParent) {
            return Err(Error::unsupported());
        }

        let (modal, current_parent) = with_window(self.id, |w| {
            (w.flags().contains(WindowFlags::MODAL), w.parent)
        })?;
        if modal {
            return Err(Error::new(
                "Modal windows cannot change parents; call SDL_SetWindowModal() to clear modal status first.",
            ));
        }

        let parent = parent.map(|p| p.id);
        if current_parent == parent {
            return Ok(());
        }

        let ret = driver
            .set_window_parent(self.id, parent)
            .unwrap_or_else(|| Err(Error::unsupported()));
        update_window_hierarchy(self.id, if ret.is_ok() { parent } else { None });

        ret
    }

    /// Toggle the modal state of the window (it must have a parent).
    /// Translation of `SDL_SetWindowModal()`.
    pub fn set_modal(&self, modal: bool) -> Result<()> {
        self.check_not_popup()?;
        let driver = driver()?;
        if !driver.implements_window_op(WindowOp::SetModal) {
            return Err(Error::unsupported());
        }

        let (has_parent, is_modal) = with_window(self.id, |w| {
            (w.parent.is_some(), w.flags().contains(WindowFlags::MODAL))
        })?;
        if modal {
            if !has_parent {
                return Err(Error::new(
                    "Window must have a parent to enable the modal state; use SDL_SetWindowParent() to set the parent first.",
                ));
            }
            with_window(self.id, |w| w.core.flags |= WindowFlags::MODAL)?;
        } else if is_modal {
            with_window(self.id, |w| w.core.flags &= !WindowFlags::MODAL)?;
        } else {
            return Ok(()); // Already not modal, so nothing to do.
        }

        if with_window(self.id, |w| w.flags().contains(WindowFlags::HIDDEN))? {
            return Ok(());
        }

        driver
            .set_window_modal(self.id, modal)
            .unwrap_or_else(|| Err(Error::unsupported()))
    }

    /// Set whether the window may have input focus. Translation of
    /// `SDL_SetWindowFocusable()`.
    pub fn set_focusable(&self, focusable: bool) -> Result<()> {
        let have = with_window(self.id, |w| !w.flags().contains(WindowFlags::NOT_FOCUSABLE))?;
        let driver = driver()?;
        if focusable != have && driver.implements_window_op(WindowOp::SetFocusable) {
            with_window(self.id, |w| {
                w.core.flags.set(WindowFlags::NOT_FOCUSABLE, !focusable)
            })?;
            if let Some(Err(e)) = driver.set_window_focusable(self.id, focusable) {
                return Err(e);
            }
        }
        Ok(())
    }

    /// Set whether the window fills the document (web platforms).
    /// Translation of `SDL_SetWindowFillDocument()`.
    pub fn set_fill_document(&self, fill: bool) -> Result<()> {
        let have = with_window(self.id, |w| w.flags().contains(WindowFlags::FILL_DOCUMENT))?;
        let driver = driver()?;
        if fill != have {
            if let Some(result) = driver.set_window_fill_document(self.id, fill) {
                result?;
                with_window(self.id, |w| {
                    w.core.flags.set(WindowFlags::FILL_DOCUMENT, fill)
                })?;
            }
        }
        Ok(())
    }

    /// Set the window's keyboard grab mode (while it has focus).
    /// Translation of `SDL_SetWindowKeyboardGrab()`.
    pub fn set_keyboard_grab(&self, grabbed: bool) -> Result<()> {
        self.set_grab(grabbed, WindowFlags::KEYBOARD_GRABBED)
    }

    /// Set the window's mouse grab mode (while it has focus). Translation of
    /// `SDL_SetWindowMouseGrab()`.
    pub fn set_mouse_grab(&self, grabbed: bool) -> Result<()> {
        self.set_grab(grabbed, WindowFlags::MOUSE_GRABBED)
    }

    /// `SDL_SetWindowKeyboardGrab()`/`SDL_SetWindowMouseGrab()`.
    fn set_grab(&self, grabbed: bool, flag: WindowFlags) -> Result<()> {
        self.check_not_popup()?;

        let flags = with_window(self.id, |w| w.flags())?;
        if flags.contains(WindowFlags::HIDDEN) {
            with_window(self.id, |w| w.core.pending_flags.set(flag, grabbed))?;
            return Ok(());
        }

        if grabbed == flags.contains(flag) {
            return Ok(());
        }
        with_window(self.id, |w| w.core.flags.set(flag, grabbed))?;
        core::update_window_grab(self.id);

        if grabbed && !with_window(self.id, |w| w.flags().contains(flag))? {
            return Err(Error::new("Couldn't grab input"));
        }
        Ok(())
    }

    /// Whether the window has the keyboard grabbed. Translation of
    /// `SDL_GetWindowKeyboardGrab()`.
    pub fn keyboard_grab(&self) -> Result<bool> {
        let flags = with_window(self.id, |w| w.flags())?;
        Ok(
            core::grabbed_window() == Some(self.id)
                && flags.contains(WindowFlags::KEYBOARD_GRABBED),
        )
    }

    /// Whether the window has the mouse grabbed. Translation of
    /// `SDL_GetWindowMouseGrab()`.
    pub fn mouse_grab(&self) -> Result<bool> {
        let flags = with_window(self.id, |w| w.flags())?;
        let grabbed = with_device(|v| v.grabbed_window)?;
        Ok(grabbed == Some(self.id) && flags.contains(WindowFlags::MOUSE_GRABBED))
    }

    /// Confine the cursor to an area of the window (`None`: no
    /// confinement). Translation of `SDL_SetWindowMouseRect()`.
    pub fn set_mouse_rect(&self, rect: Option<&Rect>) -> Result<()> {
        with_window(self.id, |_| ())?;
        let driver = driver()?;
        if !driver.implements_window_op(WindowOp::SetMouseRect) {
            return Err(Error::unsupported());
        }

        let rect = rect.copied().unwrap_or_default();
        let changed = with_window(self.id, |w| {
            if w.core.mouse_rect == rect {
                false
            } else {
                w.core.mouse_rect = rect;
                true
            }
        })?;
        if !changed {
            return Ok(());
        }

        driver
            .set_window_mouse_rect(self.id)
            .unwrap_or_else(|| Err(Error::unsupported()))
    }

    /// The area the cursor is confined to, if any. Translation of
    /// `SDL_GetWindowMouseRect()`.
    pub fn mouse_rect(&self) -> Result<Option<Rect>> {
        with_window(self.id, |w| {
            if w.core.mouse_rect.is_empty() {
                None
            } else {
                Some(w.core.mouse_rect)
            }
        })
    }

    /// Set relative mouse mode for the window. Translation of
    /// `SDL_SetWindowRelativeMouseMode()`.
    pub fn set_relative_mouse_mode(&self, enabled: bool) -> Result<()> {
        with_window(self.id, |_| ())?;

        /* If the app toggles relative mode directly, it probably shouldn't
         * also be emulating it using repeated mouse warps, so disable
         * mouse warp emulation by default.
         */
        mouse::disable_mouse_warp_emulation();

        if enabled == self.relative_mouse_mode()? {
            return Ok(());
        }

        with_window(self.id, |w| {
            w.core.flags.set(WindowFlags::MOUSE_RELATIVE_MODE, enabled)
        })?;

        if let Err(e) = mouse::update_relative_mouse_mode() {
            with_window(self.id, |w| {
                w.core.flags.set(WindowFlags::MOUSE_RELATIVE_MODE, !enabled)
            })?;
            return Err(e);
        }
        Ok(())
    }

    /// Whether relative mouse mode is on for the window. Translation of
    /// `SDL_GetWindowRelativeMouseMode()`.
    pub fn relative_mouse_mode(&self) -> Result<bool> {
        with_window(self.id, |w| {
            w.flags().contains(WindowFlags::MOUSE_RELATIVE_MODE)
        })
    }

    /// Request a window to demand attention from the user. Translation of
    /// `SDL_FlashWindow()`.
    pub fn flash(&self, operation: FlashOperation) -> Result<()> {
        self.check_not_popup()?;
        driver()?
            .flash_window(self.id, operation)
            .unwrap_or_else(|| Err(Error::unsupported()))
    }

    /// Set the state of the progress bar in the window's taskbar icon.
    /// Translation of `SDL_SetWindowProgressState()`.
    pub fn set_progress_state(&self, state: ProgressState) -> Result<()> {
        self.check_not_popup()?;
        with_window(self.id, |w| w.progress_state = state)?;
        if let Some(Err(e)) = driver()?.apply_window_progress(self.id) {
            return Err(e);
        }
        Ok(())
    }

    /// The state of the progress bar. Translation of
    /// `SDL_GetWindowProgressState()`.
    pub fn progress_state(&self) -> Result<ProgressState> {
        self.check_not_popup()?;
        with_window(self.id, |w| w.progress_state)
    }

    /// Set the value of the progress bar (clamped to 0.0..=1.0).
    /// Translation of `SDL_SetWindowProgressValue()`.
    pub fn set_progress_value(&self, value: f32) -> Result<()> {
        self.check_not_popup()?;
        let value = value.clamp(0.0, 1.0);
        with_window(self.id, |w| w.progress_value = value)?;
        if let Some(Err(e)) = driver()?.apply_window_progress(self.id) {
            return Err(e);
        }
        Ok(())
    }

    /// The value of the progress bar. Translation of
    /// `SDL_GetWindowProgressValue()`.
    pub fn progress_value(&self) -> Result<f32> {
        self.check_not_popup()?;
        with_window(self.id, |w| w.progress_value)
    }

    /// The safe area of the window: where content won't be covered by
    /// notches, rounded corners and such. Translation of
    /// `SDL_GetWindowSafeArea()`.
    pub fn safe_area(&self) -> Result<Rect> {
        with_window(self.id, |w| {
            if w.safe_rect.is_empty() {
                Rect::new(0, 0, w.core.w, w.core.h)
            } else {
                w.safe_rect
            }
        })
    }

    /// Display the system-level window menu at a point of the window.
    /// Translation of `SDL_ShowWindowSystemMenu()`.
    pub fn show_system_menu(&self, x: i32, y: i32) -> Result<()> {
        self.check_not_popup()?;
        match driver()?.show_window_system_menu(self.id, x, y) {
            Some(()) => Ok(()),
            None => Err(Error::unsupported()),
        }
    }

    /// Provide a callback that decides if a window region has special
    /// properties (draggable, resize borders...), or remove it.
    /// Translation of `SDL_SetWindowHitTest()`.
    pub fn set_hit_test(&self, callback: Option<HitTest>) -> Result<()> {
        with_window(self.id, |_| ())?;
        let driver = driver()?;
        if !driver.implements_window_op(WindowOp::SetHitTest) {
            return Err(Error::unsupported());
        }

        let enabled = callback.is_some();
        with_window(self.id, |w| w.hit_test = callback)?;

        driver
            .set_window_hit_test(self.id, enabled)
            .unwrap_or_else(|| Err(Error::unsupported()))
    }

    /// Run the hit test callback for a point, if there is one.
    #[allow(dead_code)] // (used by the video drivers)
    pub(crate) fn hit_test(&self, point: Point) -> Option<super::sysvideo::HitTestResult> {
        let callback = with_window(self.id, |w| w.hit_test.clone())
            .ok()
            .flatten()?;
        Some(callback(self.id, point))
    }

    /// Set the shape of a transparent window (`None`: the window's own
    /// shape): its alpha channel decides which parts are part of the window.
    /// Translation of `SDL_SetWindowShape()`.
    pub fn set_shape(&self, shape: Option<&Surface<'_>>) -> Result<()> {
        if !with_window(self.id, |w| w.flags().contains(WindowFlags::TRANSPARENT))? {
            return Err(Error::new(
                "Window must be created with SDL_WINDOW_TRANSPARENT",
            ));
        }

        let props = self.properties()?;

        let surface = match shape {
            Some(shape) => Some(shape.convert(PixelFormat::ARGB32)?),
            None => None,
        };

        match &surface {
            Some(surface) => props.set(
                PROP_WINDOW_SHAPE_POINTER,
                crate::properties::Value::any(surface.duplicate()?),
            )?,
            None => {
                let _ = props.remove(PROP_WINDOW_SHAPE_POINTER);
            }
        }

        if let Some(Err(e)) = driver()?.update_window_shape(self.id, surface.as_ref()) {
            return Err(e);
        }
        Ok(())
    }

    /// Destroy the window (and its children). Translation of
    /// `SDL_DestroyWindow()`.
    pub fn destroy(self) {
        let window = self.id;
        if with_window(window, |w| w.core.is_destroying = true).is_err() {
            return;
        }

        // Destroy any child windows of this window
        while let Some(child) = with_window(window, |w| w.children.first().copied())
            .ok()
            .flatten()
        {
            let before = with_window(window, |w| w.children.len()).unwrap_or(0);
            Window { id: child }.destroy();
            if with_window(window, |w| w.children.len()).unwrap_or(0) >= before {
                // (the child didn't go away; don't spin)
                let _ = with_window(window, |w| w.children.retain(|&c| c != child));
            }
        }

        send_window_event(window, EventType::WINDOW_DESTROYED, 0, 0);

        let _ = self.destroy_surface();

        crate::render::destroy_window_renderer(window);

        // Restore video mode, etc.
        let _ = update_fullscreen_mode(window, FullscreenOp::Leave, true);
        let flags = with_window(window, |w| w.flags()).unwrap_or_default();
        if !flags.contains(WindowFlags::EXTERNAL) {
            let _ = self.hide();
        }

        let _ = with_window(window, |w| {
            w.text_input_props = None;
            w.props = None;
        });

        let Ok(driver) = driver() else { return };
        let flags = with_window(window, |w| w.flags()).unwrap_or_default();

        /* Clear the modal status, but don't unset the parent just yet, as it
         * may be needed later in the destruction process if a backend needs
         * to update the input focus.
         */
        if flags.contains(WindowFlags::MODAL) {
            let _ = driver.set_window_modal(window, false);
        }

        // Make sure the destroyed window isn't referenced by any display as a fullscreen window.
        let _ = with_device(|v| {
            for d in &mut v.displays {
                if d.fullscreen_window == Some(window) {
                    d.fullscreen_window = None;
                }
            }
        });

        // Make sure this window no longer has focus
        if keyboard::keyboard_focus() == Some(window) {
            let _ = keyboard::set_keyboard_focus(None);
        }
        if flags.contains(WindowFlags::MOUSE_CAPTURE) {
            let _ = mouse::update_mouse_capture(true);
        }
        if mouse::mouse_focus() == Some(window) {
            mouse::set_mouse_focus(None);
        }

        // Make no context current if this is the current context window
        if flags.contains(WindowFlags::OPENGL) && super::gl::current_window_id() == Some(window) {
            let _ = super::gl::make_current_raw(Some(window), None);
        }

        let _ = driver.destroy_window(window);

        // Unload the graphics libraries after the window is destroyed, which may clean up EGL surfaces
        if flags.contains(WindowFlags::OPENGL) {
            super::gl::gl_unload_library();
        }
        if flags.contains(WindowFlags::VULKAN) {
            super::vulkan::vulkan_unload_library();
        }

        let _ = with_device(|v| {
            if v.grabbed_window == Some(window) {
                v.grabbed_window = None; // ungrabbing input.
            }
            if v.wakeup_window == Some(window) {
                v.wakeup_window = None;
            }
        });
        super::gl::forget_window(window);

        // Unlink the window from its siblings.
        update_window_hierarchy(window, None);

        // Unlink the window from the global window list (which invalidates it)
        let _ = with_device(|v| v.windows.retain(|w| w.core.id != window));
    }
}

/// Optional window operations whose presence (a non-NULL function pointer
/// upstream) changes what the front end does.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum WindowOp {
    SetBordered,
    SetResizable,
    SetAlwaysOnTop,
    Maximize,
    Minimize,
    Restore,
    SetParent,
    SetModal,
    SetFocusable,
    SetMouseRect,
    SetHitTest,
}
