// Rust translation of src/video/SDL_sysvideo.h and parts of
// include/SDL3/SDL_video.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The video driver interface and the records behind windows and displays.
//!
//! Upstream's `SDL_VideoDevice` is a table of function pointers, many of
//! them optional (NULL when the backend doesn't implement them). Here the
//! table is the (crate-internal) `VideoDriver` trait; an optional entry
//! point returns `Option`, and its default `None` means "not implemented"
//! (a NULL pointer). Backends refer to windows and displays by ID and reach
//! their state through the video core, never holding it across a call back
//! into SDL.

use std::any::Any;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::error::Result;
use crate::events::mouse::{
    Cursor, CursorFrame, MouseButtonFlags, MouseFeature, MouseID, SystemCursor,
};
use crate::events::window::{WindowCore, WindowFlags};
use crate::events::{DisplayID, WindowID};
use crate::properties::Properties;
use crate::video::{PixelFormat, Point, Rect, Surface};

use super::gl::{GlConfig, GlContext};
use super::messagebox::MessageBoxData;

// ---------------------------------------------------------------------------
// Public types of SDL_video.h
// ---------------------------------------------------------------------------

/// The system theme. Translation of `SDL_SystemTheme`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum SystemTheme {
    /// Unknown system theme
    #[default]
    Unknown,
    /// Light colored system theme
    Light,
    /// Dark colored system theme
    Dark,
}

/// The structure that defines a display mode. Translation of
/// `SDL_DisplayMode` (backend data for a mode is kept by the backend).
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct DisplayMode {
    /// the display this mode is associated with
    pub display_id: DisplayID,
    /// pixel format
    pub format: PixelFormat,
    /// width
    pub w: i32,
    /// height
    pub h: i32,
    /// scale converting size to pixels (e.g. a 1920x1080 mode with 2.0 scale would have 3840x2160 pixels)
    pub pixel_density: f32,
    /// refresh rate (or 0.0f for unspecified)
    pub refresh_rate: f32,
    /// precise refresh rate numerator (or 0 for unspecified)
    pub refresh_rate_numerator: i32,
    /// precise refresh rate denominator
    pub refresh_rate_denominator: i32,
}

/// Display orientation values; the way a display is rotated. Translation
/// of `SDL_DisplayOrientation`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum DisplayOrientation {
    /// The display orientation can't be determined
    #[default]
    Unknown = 0,
    /// The display is in landscape mode, with the right side up, relative to portrait mode
    Landscape = 1,
    /// The display is in landscape mode, with the left side up, relative to portrait mode
    LandscapeFlipped = 2,
    /// The display is in portrait mode
    Portrait = 3,
    /// The display is in portrait mode, upside down
    PortraitFlipped = 4,
}

impl DisplayOrientation {
    /// The orientation for a raw `SDL_DisplayOrientation` value.
    pub fn from_i32(v: i32) -> DisplayOrientation {
        match v {
            1 => DisplayOrientation::Landscape,
            2 => DisplayOrientation::LandscapeFlipped,
            3 => DisplayOrientation::Portrait,
            4 => DisplayOrientation::PortraitFlipped,
            _ => DisplayOrientation::Unknown,
        }
    }
}

/// Window flash operation. Translation of `SDL_FlashOperation`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum FlashOperation {
    /// Cancel any window flash state
    Cancel,
    /// Flash the window briefly to get attention
    Briefly,
    /// Flash the window until it gets focus
    UntilFocused,
}

/// Window progress state. Translation of `SDL_ProgressState` (whose
/// `SDL_PROGRESS_STATE_INVALID` is an error here).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum ProgressState {
    /// No progress bar is shown
    #[default]
    None,
    /// The progress bar is shown in a indeterminate state
    Indeterminate,
    /// The progress bar is shown in a normal state
    Normal,
    /// The progress bar is shown in a paused state
    Paused,
    /// The progress bar is shown in a state indicating the application had an error
    Error,
}

/// Possible return values from a hit test callback. Translation of
/// `SDL_HitTestResult`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum HitTestResult {
    /// Region is normal. No special properties.
    #[default]
    Normal,
    /// Region can drag entire window.
    Draggable,
    /// Region is the resizable top-left corner border.
    ResizeTopLeft,
    /// Region is the resizable top border.
    ResizeTop,
    /// Region is the resizable top-right corner border.
    ResizeTopRight,
    /// Region is the resizable right border.
    ResizeRight,
    /// Region is the resizable bottom-right corner border.
    ResizeBottomRight,
    /// Region is the resizable bottom border.
    ResizeBottom,
    /// Region is the resizable bottom-left corner border.
    ResizeBottomLeft,
    /// Region is the resizable left border.
    ResizeLeft,
}

/// Callback used for hit-testing: given a window and a point in it, what
/// kind of region the point is. Translation of `SDL_HitTest` (the closure
/// replaces its `data`).
pub type HitTest = Arc<dyn Fn(WindowID, Point) -> HitTestResult + Send + Sync>;

/// Translation of `SDL_WINDOWPOS_UNDEFINED_MASK`.
pub const WINDOWPOS_UNDEFINED_MASK: i32 = 0x1FFF0000;
/// Translation of `SDL_WINDOWPOS_CENTERED_MASK`.
pub const WINDOWPOS_CENTERED_MASK: i32 = 0x2FFF0000;
/// Used to indicate that you don't care what the window position is.
/// Translation of `SDL_WINDOWPOS_UNDEFINED`.
pub const WINDOWPOS_UNDEFINED: i32 = WINDOWPOS_UNDEFINED_MASK;
/// Used to indicate that the window position should be centered.
/// Translation of `SDL_WINDOWPOS_CENTERED`.
pub const WINDOWPOS_CENTERED: i32 = WINDOWPOS_CENTERED_MASK;

/// An undefined position on a specific display. Translation of
/// `SDL_WINDOWPOS_UNDEFINED_DISPLAY()`.
pub const fn windowpos_undefined_display(display: DisplayID) -> i32 {
    WINDOWPOS_UNDEFINED_MASK | (display as i32 & 0xFFFF)
}

/// A centered position on a specific display. Translation of
/// `SDL_WINDOWPOS_CENTERED_DISPLAY()`.
pub const fn windowpos_centered_display(display: DisplayID) -> i32 {
    WINDOWPOS_CENTERED_MASK | (display as i32 & 0xFFFF)
}

/// Whether a window position is undefined. Translation of
/// `SDL_WINDOWPOS_ISUNDEFINED()`.
pub const fn windowpos_is_undefined(x: i32) -> bool {
    (x as u32 & 0xFFFF0000) == WINDOWPOS_UNDEFINED_MASK as u32
}

/// Whether a window position is centered. Translation of
/// `SDL_WINDOWPOS_ISCENTERED()`.
pub const fn windowpos_is_centered(x: i32) -> bool {
    (x as u32 & 0xFFFF0000) == WINDOWPOS_CENTERED_MASK as u32
}

// ---------------------------------------------------------------------------
// The driver-side records (SDL_sysvideo.h)
// ---------------------------------------------------------------------------

/// Translation of `SDL_HDROutputProperties`.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub(crate) struct HdrOutputProperties {
    pub(crate) sdr_white_level: f32,
    pub(crate) hdr_headroom: f32,
}

/// The surface of a window, shared with the app's [`WindowSurface`]
/// handles (upstream lends the app the window's `SDL_Surface *`).
///
/// [`WindowSurface`]: super::window::WindowSurface
pub(crate) type SharedSurface = Arc<Mutex<Surface<'static>>>;

/// The state of a window. Translation of `struct SDL_Window`.
///
/// The fields the event core touches live in [`WindowCore`] (`core`).
pub(crate) struct WindowData {
    /// id, flags, position, size, windowed/floating rects, display IDs...
    pub(crate) core: WindowCore,
    pub(crate) title: Option<String>,
    pub(crate) icon: Option<Arc<Surface<'static>>>,
    pub(crate) min_aspect: f32,
    pub(crate) max_aspect: f32,
    pub(crate) display_scale: f32,
    pub(crate) external_graphics_context: bool,
    /// The window is currently fullscreen exclusive
    pub(crate) fullscreen_exclusive: bool,
    /// The last fullscreen_exclusive display
    pub(crate) last_fullscreen_exclusive_display: DisplayID,

    /// The last client requested size and position for the window.
    pub(crate) pending: Rect,

    pub(crate) requested_fullscreen_mode: DisplayMode,
    pub(crate) current_fullscreen_mode: DisplayMode,
    pub(crate) hdr: HdrOutputProperties,

    pub(crate) opacity: f32,

    pub(crate) surface: Option<SharedSurface>,
    pub(crate) surface_valid: bool,

    pub(crate) is_hiding: bool,
    /// Child was hidden recursively by the parent, restore when shown.
    pub(crate) restore_on_show: bool,
    pub(crate) constrain_popup: bool,

    pub(crate) safe_inset_left: i32,
    pub(crate) safe_inset_right: i32,
    pub(crate) safe_inset_top: i32,
    pub(crate) safe_inset_bottom: i32,
    pub(crate) safe_rect: Rect,

    pub(crate) text_input_props: Option<Properties>,
    pub(crate) text_input_rect: Rect,
    pub(crate) text_input_cursor: i32,

    pub(crate) hit_test: Option<HitTest>,

    pub(crate) progress_state: ProgressState,
    pub(crate) progress_value: f32,

    pub(crate) props: Option<Properties>,

    /// Driver-specific data (`SDL_WindowData *internal`).
    pub(crate) internal: Option<Box<dyn Any + Send>>,

    /// If a toplevel window, holds the current keyboard focus for grabbing popups.
    pub(crate) keyboard_focus: Option<WindowID>,

    pub(crate) parent: Option<WindowID>,
    /// The child windows, the most recently added first (`first_child`,
    /// `next_sibling`...).
    pub(crate) children: Vec<WindowID>,
}

impl WindowData {
    pub(crate) fn new(id: WindowID) -> WindowData {
        WindowData {
            core: WindowCore::new(id),
            title: None,
            icon: None,
            min_aspect: 0.0,
            max_aspect: 0.0,
            display_scale: 0.0,
            external_graphics_context: false,
            fullscreen_exclusive: false,
            last_fullscreen_exclusive_display: 0,
            pending: Rect::default(),
            requested_fullscreen_mode: DisplayMode::default(),
            current_fullscreen_mode: DisplayMode::default(),
            hdr: HdrOutputProperties::default(),
            opacity: 0.0,
            surface: None,
            surface_valid: false,
            is_hiding: false,
            restore_on_show: false,
            constrain_popup: false,
            safe_inset_left: 0,
            safe_inset_right: 0,
            safe_inset_top: 0,
            safe_inset_bottom: 0,
            safe_rect: Rect::default(),
            text_input_props: None,
            text_input_rect: Rect::default(),
            text_input_cursor: 0,
            hit_test: None,
            progress_state: ProgressState::None,
            progress_value: 0.0,
            props: None,
            internal: None,
            keyboard_focus: None,
            parent: None,
            children: Vec::new(),
        }
    }

    /// The window's flags.
    pub(crate) fn flags(&self) -> WindowFlags {
        self.core.flags
    }

    /// Translation of `SDL_WINDOW_FULLSCREEN_VISIBLE()`.
    pub(crate) fn fullscreen_visible(&self) -> bool {
        let flags = self.core.flags;
        flags.contains(WindowFlags::FULLSCREEN)
            && !flags.contains(WindowFlags::HIDDEN)
            && !flags.contains(WindowFlags::MINIMIZED)
    }

    /// Translation of `SDL_WINDOW_IS_POPUP()`.
    pub(crate) fn is_popup(&self) -> bool {
        self.core
            .flags
            .intersects(WindowFlags::TOOLTIP | WindowFlags::POPUP_MENU)
    }

    /// The window's properties, created on demand.
    pub(crate) fn properties(&mut self) -> Properties {
        self.props.get_or_insert_with(Properties::new).clone()
    }
}

/// Which mode a display is currently in (upstream's `current_mode`
/// pointer, which points at the desktop mode, into the fullscreen mode
/// list, or at a mode the backend owns).
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum CurrentMode {
    Desktop,
    Fullscreen(usize),
    Other(DisplayMode),
}

/// A physical monitor attached to the system. Translation of
/// `struct SDL_VideoDisplay`.
pub(crate) struct VideoDisplay {
    pub(crate) id: DisplayID,
    pub(crate) name: Option<String>,
    pub(crate) fullscreen_modes: Vec<DisplayMode>,
    pub(crate) desktop_mode: DisplayMode,
    pub(crate) current_mode: CurrentMode,
    pub(crate) natural_orientation: DisplayOrientation,
    pub(crate) current_orientation: DisplayOrientation,
    pub(crate) content_scale: f32,
    pub(crate) hdr: HdrOutputProperties,

    /// This is true if we are fullscreen or fullscreen is pending
    pub(crate) fullscreen_active: bool,
    pub(crate) fullscreen_window: Option<WindowID>,

    pub(crate) props: Option<Properties>,

    /// Driver-specific data (`SDL_DisplayData *internal`).
    #[allow(dead_code)] // (used by the video drivers)
    pub(crate) internal: Option<Box<dyn Any + Send>>,
}

impl VideoDisplay {
    /// A display for a backend to fill in and pass to
    /// [`add_video_display`](super::display::add_video_display).
    pub(crate) fn new() -> VideoDisplay {
        VideoDisplay {
            id: 0,
            name: None,
            fullscreen_modes: Vec::new(),
            desktop_mode: DisplayMode::default(),
            current_mode: CurrentMode::Desktop,
            natural_orientation: DisplayOrientation::Unknown,
            current_orientation: DisplayOrientation::Unknown,
            content_scale: 0.0,
            hdr: HdrOutputProperties::default(),
            fullscreen_active: false,
            fullscreen_window: None,
            props: None,
            internal: None,
        }
    }

    /// The mode `current_mode` refers to.
    pub(crate) fn current_mode(&self) -> DisplayMode {
        match self.current_mode {
            CurrentMode::Desktop => self.desktop_mode,
            CurrentMode::Fullscreen(i) => self
                .fullscreen_modes
                .get(i)
                .copied()
                .unwrap_or(self.desktop_mode),
            CurrentMode::Other(mode) => mode,
        }
    }
}

/// Video device capabilities. Translation of `DeviceCaps`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) struct DeviceCaps(pub(crate) u32);

#[allow(dead_code)] // (used by the video drivers)
impl DeviceCaps {
    pub(crate) const NONE: DeviceCaps = DeviceCaps(0);
    pub(crate) const MODE_SWITCHING_EMULATED: DeviceCaps = DeviceCaps(0x01);
    pub(crate) const HAS_POPUP_WINDOW_SUPPORT: DeviceCaps = DeviceCaps(0x02);
    pub(crate) const SENDS_FULLSCREEN_DIMENSIONS: DeviceCaps = DeviceCaps(0x04);
    pub(crate) const FULLSCREEN_ONLY: DeviceCaps = DeviceCaps(0x08);
    pub(crate) const SENDS_DISPLAY_CHANGES: DeviceCaps = DeviceCaps(0x10);
    pub(crate) const SENDS_HDR_CHANGES: DeviceCaps = DeviceCaps(0x20);
    pub(crate) const SLOW_FRAMEBUFFER: DeviceCaps = DeviceCaps(0x40);

    pub(crate) const fn contains(self, other: DeviceCaps) -> bool {
        self.0 & other.0 == other.0
    }
}

impl std::ops::BitOr for DeviceCaps {
    type Output = DeviceCaps;
    fn bitor(self, rhs: DeviceCaps) -> DeviceCaps {
        DeviceCaps(self.0 | rhs.0)
    }
}

/// Fullscreen operations. Translation of `SDL_FullscreenOp`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum FullscreenOp {
    Leave = 0,
    Enter,
    Update,
}

/// Translation of `SDL_FullscreenResult`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[allow(dead_code)] // (used by the video drivers)
pub(crate) enum FullscreenResult {
    Failed,
    Succeeded,
    Pending,
}

/// The entry points of a video backend. Translation of the function
/// pointers of `struct SDL_VideoDevice` (and of `SDL_Mouse` that video
/// backends fill in).
///
/// An entry point returning `Option` is optional: `None` means the
/// backend doesn't implement it. Entry points are called without any
/// video state borrowed, so they may call back into SDL (to send window
/// events, read window state with
/// [`with_window`](super::core::with_window)...).
#[allow(unused_variables)]
pub(crate) trait VideoDriver: Send + Sync {
    /// The video device flags (`device_caps`).
    fn caps(&self) -> DeviceCaps {
        DeviceCaps::NONE
    }

    // * * * Initialization/Query functions

    /// Initialize the native video subsystem, filling in the list of
    /// displays for this driver.
    fn video_init(&self) -> Result<()>;

    /// Reverse the effects `video_init()` -- called if `video_init()`
    /// fails or if the application is shutting down the video subsystem.
    fn video_quit(&self);

    /// Reinitialize the touch devices -- called if an unknown touch ID occurs.
    fn reset_touch(&self) -> Option<()> {
        None
    }

    // * * * Display functions

    /// Refresh the display list
    fn refresh_displays(&self) -> Option<()> {
        None
    }

    /// Get the bounds of a display
    fn display_bounds(&self, display: DisplayID) -> Option<Result<Rect>> {
        None
    }

    /// Get the usable bounds of a display (bounds minus menubar or whatever)
    fn display_usable_bounds(&self, display: DisplayID) -> Option<Result<Rect>> {
        None
    }

    /// Add the available display modes for a display (with
    /// [`add_fullscreen_display_mode`](super::display::add_fullscreen_display_mode)).
    fn display_modes(&self, display: DisplayID) -> Option<()> {
        None
    }

    /// Setting the display mode is independent of creating windows, so
    /// when the display mode is changed, all existing windows should have
    /// their data updated accordingly, including the display surfaces
    /// associated with them.
    fn set_display_mode(&self, display: DisplayID, mode: &DisplayMode) -> Option<Result<()>> {
        None
    }

    // * * * Window functions

    fn create_window(&self, window: WindowID, create_props: &Properties) -> Option<Result<()>> {
        None
    }
    fn set_window_title(&self, window: WindowID) -> Option<()> {
        None
    }
    fn set_window_icon(&self, window: WindowID, icon: &Surface<'static>) -> Option<Result<()>> {
        None
    }
    fn set_window_position(&self, window: WindowID) -> Option<Result<()>> {
        None
    }
    fn set_window_size(&self, window: WindowID) -> Option<()> {
        None
    }
    fn set_window_minimum_size(&self, window: WindowID) -> Option<()> {
        None
    }
    fn set_window_maximum_size(&self, window: WindowID) -> Option<()> {
        None
    }
    fn set_window_aspect_ratio(&self, window: WindowID) -> Option<()> {
        None
    }
    /// The border sizes: (top, left, bottom, right).
    fn window_borders_size(&self, window: WindowID) -> Option<Result<(i32, i32, i32, i32)>> {
        None
    }
    fn window_content_scale(&self, window: WindowID) -> Option<f32> {
        None
    }
    fn window_size_in_pixels(&self, window: WindowID) -> Option<(i32, i32)> {
        None
    }
    fn set_window_opacity(&self, window: WindowID, opacity: f32) -> Option<Result<()>> {
        None
    }
    fn set_window_parent(&self, window: WindowID, parent: Option<WindowID>) -> Option<Result<()>> {
        None
    }
    fn set_window_modal(&self, window: WindowID, modal: bool) -> Option<Result<()>> {
        None
    }
    fn show_window(&self, window: WindowID) -> Option<()> {
        None
    }
    fn hide_window(&self, window: WindowID) -> Option<()> {
        None
    }
    fn raise_window(&self, window: WindowID) -> Option<()> {
        None
    }
    fn maximize_window(&self, window: WindowID) -> Option<()> {
        None
    }
    fn minimize_window(&self, window: WindowID) -> Option<()> {
        None
    }
    fn restore_window(&self, window: WindowID) -> Option<()> {
        None
    }
    fn set_window_bordered(&self, window: WindowID, bordered: bool) -> Option<()> {
        None
    }
    fn set_window_resizable(&self, window: WindowID, resizable: bool) -> Option<()> {
        None
    }
    fn set_window_always_on_top(&self, window: WindowID, on_top: bool) -> Option<()> {
        None
    }
    fn set_window_fullscreen(
        &self,
        window: WindowID,
        display: DisplayID,
        fullscreen: FullscreenOp,
    ) -> Option<FullscreenResult> {
        None
    }
    fn window_icc_profile(&self, window: WindowID) -> Option<Result<Vec<u8>>> {
        None
    }
    fn display_for_window(&self, window: WindowID) -> Option<DisplayID> {
        None
    }
    fn set_window_mouse_rect(&self, window: WindowID) -> Option<Result<()>> {
        None
    }
    fn set_window_mouse_grab(&self, window: WindowID, grabbed: bool) -> Option<Result<()>> {
        None
    }
    fn set_window_keyboard_grab(&self, window: WindowID, grabbed: bool) -> Option<Result<()>> {
        None
    }
    fn destroy_window(&self, window: WindowID) -> Option<()> {
        None
    }
    /// Create a framebuffer for the window, `w`x`h` pixels: the surface
    /// the app draws into (upstream returns its format, pixels and pitch).
    fn create_window_framebuffer(
        &self,
        window: WindowID,
        w: i32,
        h: i32,
    ) -> Option<Result<Surface<'static>>> {
        None
    }
    fn set_window_framebuffer_vsync(&self, window: WindowID, vsync: i32) -> Option<Result<()>> {
        None
    }
    fn window_framebuffer_vsync(&self, window: WindowID) -> Option<Result<i32>> {
        None
    }
    /// Show the framebuffer `surface` (the window's) for `rects`.
    fn update_window_framebuffer(
        &self,
        window: WindowID,
        surface: &Surface<'static>,
        rects: &[Rect],
    ) -> Option<Result<()>> {
        None
    }
    fn destroy_window_framebuffer(&self, window: WindowID) -> Option<()> {
        None
    }
    fn on_window_enter(&self, window: WindowID) -> Option<()> {
        None
    }
    fn update_window_shape(
        &self,
        window: WindowID,
        shape: Option<&Surface<'static>>,
    ) -> Option<Result<()>> {
        None
    }
    fn flash_window(&self, window: WindowID, operation: FlashOperation) -> Option<Result<()>> {
        None
    }
    fn apply_window_progress(&self, window: WindowID) -> Option<Result<()>> {
        None
    }
    fn set_window_focusable(&self, window: WindowID, focusable: bool) -> Option<Result<()>> {
        None
    }
    fn set_window_fill_document(&self, window: WindowID, fill: bool) -> Option<Result<()>> {
        None
    }
    fn sync_window(&self, window: WindowID) -> Option<Result<()>> {
        None
    }
    fn reconfigure_window(&self, window: WindowID, flags: WindowFlags) -> Option<Result<()>> {
        None
    }

    // * * * Graphics APIs (whether `GL_CreateContext`, `Vulkan_CreateSurface`
    // and `Metal_CreateView` are set)

    fn implements_gl_contexts(&self) -> bool {
        false
    }
    fn implements_vulkan_surfaces(&self) -> bool {
        false
    }
    fn implements_metal_views(&self) -> bool {
        false
    }
    fn gl_load_library(&self, path: Option<&str>) -> Option<Result<()>> {
        None
    }
    fn gl_unload_library(&self) -> Option<()> {
        None
    }
    fn vulkan_load_library(&self, path: Option<&str>) -> Option<Result<()>> {
        None
    }
    fn vulkan_unload_library(&self) -> Option<()> {
        None
    }

    // * * * Event manager functions

    /// 1 = got an event, 0 = timed out, -1 = error.
    fn wait_event_timeout(&self, timeout: Option<Duration>) -> Option<i32> {
        None
    }
    fn send_wakeup_event(&self, window: WindowID) -> Option<()> {
        None
    }
    fn pump_events(&self) -> Option<()> {
        None
    }

    /// Suspend/resume the screensaver
    fn suspend_screen_saver(&self, suspend: bool) -> Option<Result<()>> {
        None
    }

    // * * * Hit-testing, drag and drop, the system menu

    fn set_window_hit_test(&self, window: WindowID, enabled: bool) -> Option<Result<()>> {
        None
    }
    /// Tell window that app enabled drag'n'drop events
    fn accept_drag_and_drop(&self, window: WindowID, accept: bool) -> Option<()> {
        None
    }
    /// Display the system-level window menu
    fn show_window_system_menu(&self, window: WindowID, x: i32, y: i32) -> Option<()> {
        None
    }

    // * * * The SDL_Mouse entry points video backends fill in

    fn warp_mouse(&self, window: WindowID, x: f32, y: f32) -> Option<Result<()>> {
        None
    }
    fn warp_mouse_global(&self, x: f32, y: f32) -> Option<Result<()>> {
        None
    }
    fn set_relative_mouse_mode(&self, enabled: bool) -> Option<Result<()>> {
        None
    }
    fn capture_mouse(&self, window: Option<WindowID>) -> Option<Result<()>> {
        None
    }
    fn global_mouse_state(&self) -> Option<(f32, f32, MouseButtonFlags)> {
        None
    }
    /// `mouse->ApplySystemScale` (with `mouse->system_scale_data` kept by the
    /// backend): scale relative motion like the OS would.
    fn apply_system_scale(
        &self,
        timestamp: Duration,
        window: Option<WindowID>,
        mouse_id: MouseID,
        x: f32,
        y: f32,
    ) -> Option<(f32, f32)> {
        None
    }
    /// `mouse->CreateCursor`, for an ARGB8888 surface.
    fn create_cursor(
        &self,
        surface: &Surface<'_>,
        hot_x: i32,
        hot_y: i32,
    ) -> Option<Result<Cursor>> {
        None
    }
    /// `mouse->CreateAnimatedCursor`, for ARGB8888 frames.
    fn create_animated_cursor(
        &self,
        frames: &[CursorFrame<'_>],
        hot_x: i32,
        hot_y: i32,
    ) -> Option<Result<Cursor>> {
        None
    }
    /// `mouse->CreateSystemCursor`.
    fn create_system_cursor(&self, id: SystemCursor) -> Option<Result<Cursor>> {
        None
    }
    /// `mouse->ShowCursor`: show `cursor`, or hide the cursor for `None`.
    fn show_cursor(&self, cursor: Option<&Cursor>) -> Option<Result<()>> {
        None
    }
    /// `mouse->MoveCursor`.
    fn move_cursor(&self, cursor: &Cursor) -> Option<Result<()>> {
        None
    }
    /// Whether one of the mouse entry points (or features) is implemented.
    fn implements_mouse_feature(&self, feature: MouseFeature) -> bool {
        false
    }

    /// Whether these entry points are implemented: they decide what the
    /// front end does before calling them.
    fn implements_window_op(&self, op: super::window::WindowOp) -> bool {
        false
    }
    fn implements_sync_window(&self) -> bool {
        false
    }
    fn implements_fill_document(&self) -> bool {
        false
    }
    /// Whether `create_window_framebuffer` and `update_window_framebuffer`
    /// are implemented.
    fn implements_window_framebuffer(&self) -> bool {
        false
    }

    /// Whether both `wait_event_timeout` and `send_wakeup_event` are implemented.
    fn can_wait_events(&self) -> bool {
        false
    }

    // * * * Text input

    fn start_text_input(&self, window: WindowID, props: Option<&Properties>) -> Option<Result<()>> {
        None
    }
    fn stop_text_input(&self, window: WindowID) -> Option<Result<()>> {
        None
    }
    fn update_text_input_area(&self, window: WindowID) -> Option<Result<()>> {
        None
    }
    fn clear_composition(&self, window: WindowID) -> Option<Result<()>> {
        None
    }

    // * * * Screen keyboard

    fn has_screen_keyboard_support(&self) -> Option<bool> {
        None
    }
    fn show_screen_keyboard(&self, window: WindowID, props: Option<&Properties>) -> Option<()> {
        None
    }
    fn hide_screen_keyboard(&self, window: WindowID) -> Option<()> {
        None
    }
    fn set_text_input_properties(
        &self,
        window: WindowID,
        props: Option<&Properties>,
    ) -> Option<()> {
        None
    }

    // * * * Clipboard (a backend implementing the `*_clipboard_data` entry
    // points doesn't need the `*_clipboard_text` ones)

    fn text_mime_types(&self) -> Option<Vec<String>> {
        None
    }
    /// Take ownership of the clipboard, offering the mime types of
    /// [`clipboard_mime_types`](super::clipboard::clipboard_mime_types).
    fn set_clipboard_data(&self) -> Option<Result<()>> {
        None
    }
    fn clipboard_data(&self, mime_type: &str) -> Option<Option<Vec<u8>>> {
        None
    }
    fn has_clipboard_data(&self, mime_type: &str) -> Option<bool> {
        None
    }
    /// Whether `set_clipboard_text` is implemented.
    fn implements_clipboard_text(&self) -> bool {
        false
    }
    fn set_clipboard_text(&self, text: &str) -> Option<Result<()>> {
        None
    }
    fn clipboard_text(&self) -> Option<Option<String>> {
        None
    }
    fn has_clipboard_text(&self) -> Option<bool> {
        None
    }
    // These functions are only needed if the platform has a separate primary selection buffer
    fn set_primary_selection_text(&self, text: &str) -> Option<Result<()>> {
        None
    }
    fn primary_selection_text(&self) -> Option<String> {
        None
    }
    fn has_primary_selection_text(&self) -> Option<bool> {
        None
    }

    // * * * MessageBox

    /// Show a message box, returning the ID of the button pressed.
    fn show_message_box(&self, data: &MessageBoxData) -> Option<Result<i32>> {
        None
    }

    // * * * OpenGL support

    fn gl_get_proc_address(&self, proc_name: &str) -> Option<Option<usize>> {
        None
    }
    fn gl_create_context(&self, window: WindowID) -> Option<Result<GlContext>> {
        None
    }
    fn gl_make_current(
        &self,
        window: Option<WindowID>,
        context: Option<GlContext>,
    ) -> Option<Result<()>> {
        None
    }
    fn gl_set_swap_interval(&self, interval: i32) -> Option<Result<()>> {
        None
    }
    fn gl_get_swap_interval(&self) -> Option<Result<i32>> {
        None
    }
    fn gl_swap_window(&self, window: WindowID) -> Option<Result<()>> {
        None
    }
    fn gl_destroy_context(&self, context: GlContext) -> Option<Result<()>> {
        None
    }
    fn gl_set_default_profile_config(&self, config: &mut GlConfig) -> Option<()> {
        None
    }
    /// Whether `gl_make_current` accepts a context without a window.
    fn gl_allow_no_surface(&self) -> bool {
        false
    }

    // * * * Vulkan support (handles are the raw Vulkan handles)

    fn vulkan_get_instance_proc_addr(&self) -> Option<usize> {
        None
    }
    fn vulkan_instance_extensions(&self) -> Option<Vec<&'static str>> {
        None
    }
    fn vulkan_create_surface(
        &self,
        window: WindowID,
        instance: usize,
        allocator: usize,
    ) -> Option<Result<u64>> {
        None
    }
    fn vulkan_destroy_surface(
        &self,
        instance: usize,
        surface: u64,
        allocator: usize,
    ) -> Option<()> {
        None
    }
    fn vulkan_presentation_support(
        &self,
        instance: usize,
        physical_device: usize,
        queue_family_index: u32,
    ) -> Option<bool> {
        None
    }

    // * * * Metal support (the views are `SDL_MetalView` handles)

    fn metal_create_view(&self, window: WindowID) -> Option<Option<usize>> {
        None
    }
    fn metal_destroy_view(&self, view: usize) -> Option<()> {
        None
    }
    fn metal_get_layer(&self, view: usize) -> Option<Option<usize>> {
        None
    }
}

/// Translation of `VideoBootStrap`.
pub(crate) struct VideoBootStrap {
    pub(crate) name: &'static str,
    #[allow(dead_code)]
    pub(crate) desc: &'static str,
    pub(crate) create: fn() -> Option<Arc<dyn VideoDriver>>,
    /// Can be done without initializing backend!
    pub(crate) show_message_box: Option<fn(&MessageBoxData) -> Result<i32>>,
    pub(crate) is_preferred: bool,
}
