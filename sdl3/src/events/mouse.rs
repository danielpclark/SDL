// Rust translation of src/events/SDL_mouse.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! General mouse handling code for SDL: mouse devices, focus, motion, buttons
//! and wheel, relative mode, capture, warping, and cursors (creation from
//! bitmaps and surfaces, animation, the current and default cursor,
//! show/hide).

use std::any::Any;
use std::cell::RefCell;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::queue;
use super::window::{self, VideoHooks, WindowCore, WindowFlags};
use super::{
    Event, EventType, MouseButtonEvent, MouseDeviceEvent, MouseMotionEvent, MouseWheelEvent,
    WindowID,
};
use crate::error::{Error, Result};
use crate::hints;
use crate::thread::ReentrantMutex;
use crate::video::surface::{PROP_SURFACE_HOTSPOT_X_NUMBER, PROP_SURFACE_HOTSPOT_Y_NUMBER};
use crate::video::{PixelFormat, Rect, Surface};
use crate::{err, timer};

/// A mouse instance id. Translation of `SDL_MouseID`.
pub type MouseID = u32;

/// Mouse events not associated with a specific input device.
/// Translation of `SDL_GLOBAL_MOUSE_ID`.
pub const GLOBAL_MOUSE_ID: MouseID = 0;
/// The default mouse input device, for platforms that don't have multiple mice.
/// Translation of `SDL_DEFAULT_MOUSE_ID`.
pub const DEFAULT_MOUSE_ID: MouseID = 1;
/// The mouse id for mouse events simulated with touch input.
/// Translation of `SDL_TOUCH_MOUSEID` (`(SDL_MouseID)-1`).
pub const TOUCH_MOUSE_ID: MouseID = u32::MAX;
/// The mouse id for mouse events simulated with pen input.
/// Translation of `SDL_PEN_MOUSEID` (`(SDL_MouseID)-2`).
pub const PEN_MOUSE_ID: MouseID = u32::MAX - 1;

/// Translation of `SDL_BUTTON_LEFT`.
pub const BUTTON_LEFT: u8 = 1;
/// Translation of `SDL_BUTTON_MIDDLE`.
pub const BUTTON_MIDDLE: u8 = 2;
/// Translation of `SDL_BUTTON_RIGHT`.
pub const BUTTON_RIGHT: u8 = 3;
/// Translation of `SDL_BUTTON_X1`.
pub const BUTTON_X1: u8 = 4;
/// Translation of `SDL_BUTTON_X2`.
pub const BUTTON_X2: u8 = 5;

/// Translation of `WARP_EMULATION_THRESHOLD_NS`.
const WARP_EMULATION_THRESHOLD: Duration = Duration::from_millis(30);

/// A bitmask of pressed mouse buttons. Translation of `SDL_MouseButtonFlags`.
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct MouseButtonFlags(pub u32);

impl MouseButtonFlags {
    pub const NONE: MouseButtonFlags = MouseButtonFlags(0);
    /// Translation of `SDL_BUTTON_LMASK`.
    pub const LMASK: MouseButtonFlags = MouseButtonFlags::mask(BUTTON_LEFT);
    /// Translation of `SDL_BUTTON_MMASK`.
    pub const MMASK: MouseButtonFlags = MouseButtonFlags::mask(BUTTON_MIDDLE);
    /// Translation of `SDL_BUTTON_RMASK`.
    pub const RMASK: MouseButtonFlags = MouseButtonFlags::mask(BUTTON_RIGHT);
    /// Translation of `SDL_BUTTON_X1MASK`.
    pub const X1MASK: MouseButtonFlags = MouseButtonFlags::mask(BUTTON_X1);
    /// Translation of `SDL_BUTTON_X2MASK`.
    pub const X2MASK: MouseButtonFlags = MouseButtonFlags::mask(BUTTON_X2);

    /// The mask for a button index (first button is 1). Translation of `SDL_BUTTON_MASK(X)`.
    pub const fn mask(button: u8) -> MouseButtonFlags {
        MouseButtonFlags(1u32 << (button.wrapping_sub(1) as u32 & 31))
    }
    pub const fn contains(self, other: MouseButtonFlags) -> bool {
        self.0 & other.0 == other.0
    }
    pub const fn intersects(self, other: MouseButtonFlags) -> bool {
        self.0 & other.0 != 0
    }
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
    /// Whether `button` (1-based) is pressed.
    pub const fn is_pressed(self, button: u8) -> bool {
        self.intersects(MouseButtonFlags::mask(button))
    }
}

impl std::ops::BitOr for MouseButtonFlags {
    type Output = MouseButtonFlags;
    fn bitor(self, rhs: MouseButtonFlags) -> MouseButtonFlags {
        MouseButtonFlags(self.0 | rhs.0)
    }
}
impl std::ops::BitOrAssign for MouseButtonFlags {
    fn bitor_assign(&mut self, rhs: MouseButtonFlags) {
        self.0 |= rhs.0;
    }
}
impl std::ops::BitAnd for MouseButtonFlags {
    type Output = MouseButtonFlags;
    fn bitand(self, rhs: MouseButtonFlags) -> MouseButtonFlags {
        MouseButtonFlags(self.0 & rhs.0)
    }
}
impl std::ops::BitAndAssign for MouseButtonFlags {
    fn bitand_assign(&mut self, rhs: MouseButtonFlags) {
        self.0 &= rhs.0;
    }
}
impl std::ops::Not for MouseButtonFlags {
    type Output = MouseButtonFlags;
    fn not(self) -> MouseButtonFlags {
        MouseButtonFlags(!self.0)
    }
}
impl std::fmt::Debug for MouseButtonFlags {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "MouseButtonFlags({:#x})", self.0)
    }
}

/// Scroll direction types for the Scroll event. Translation of `SDL_MouseWheelDirection`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum MouseWheelDirection {
    /// The scroll direction is normal
    #[default]
    Normal,
    /// The scroll direction is flipped / natural
    Flipped,
}

/// Cursor types for `create_system_cursor()`. Translation of `SDL_SystemCursor`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
#[repr(u8)]
pub enum SystemCursor {
    /// Default cursor. Usually an arrow.
    #[default]
    Default,
    /// Text selection. Usually an I-beam.
    Text,
    /// Wait. Usually an hourglass or watch or spinning ball.
    Wait,
    /// Crosshair.
    Crosshair,
    /// Program is busy but still interactive. Usually it's WAIT with an arrow.
    Progress,
    /// Double arrow pointing northwest and southeast.
    NwseResize,
    /// Double arrow pointing northeast and southwest.
    NeswResize,
    /// Double arrow pointing west and east.
    EwResize,
    /// Double arrow pointing north and south.
    NsResize,
    /// Four pointed arrow pointing north, south, east, and west.
    Move,
    /// Not permitted. Usually a slashed circle or crossbones.
    NotAllowed,
    /// Pointer that indicates a link. Usually a pointing hand.
    Pointer,
    /// Window resize top-left. This may be a single arrow or a double arrow like NWSE_RESIZE.
    NwResize,
    /// Window resize top. May be NS_RESIZE.
    NResize,
    /// Window resize top-right. May be NESW_RESIZE.
    NeResize,
    /// Window resize right. May be EW_RESIZE.
    EResize,
    /// Window resize bottom-right. May be NWSE_RESIZE.
    SeResize,
    /// Window resize bottom. May be NS_RESIZE.
    SResize,
    /// Window resize bottom-left. May be NESW_RESIZE.
    SwResize,
    /// Window resize left. May be EW_RESIZE.
    WResize,
    /// A context menu is available for the object under the cursor.
    ContextMenu,
    /// Help is available for the object under the cursor.
    Help,
    /// A set of cells may be selected.
    Cell,
    /// Text selection. May be TEXT
    VerticalText,
    /// A shortcut is to be created.
    Alias,
    /// Something is to be copied.
    Copy,
    /// The dragged item cannot be dropped at this location. May be NOT_ALLOWED.
    NoDrop,
    /// The object under the cursor can be grabbed
    Grab,
    /// An object is currently being grabbed.
    Grabbing,
    /// Column resize. May be EW_RESIZE.
    ColResize,
    /// Row resize. May be NS_RESIZE.
    RowResize,
    /// Four pointed arrow pointing north, south, east, and west.
    AllScroll,
    /// Zoom in.
    ZoomIn,
    /// Zoom out.
    ZoomOut,
}

impl SystemCursor {
    /// Translation of `SDL_SYSTEM_CURSOR_COUNT`.
    pub const COUNT: usize = 34;

    /// The cursor for an `SDL_SystemCursor` integer value.
    pub fn from_index(index: i32) -> Option<SystemCursor> {
        use SystemCursor as C;
        const ALL: [SystemCursor; SystemCursor::COUNT] = [
            C::Default,
            C::Text,
            C::Wait,
            C::Crosshair,
            C::Progress,
            C::NwseResize,
            C::NeswResize,
            C::EwResize,
            C::NsResize,
            C::Move,
            C::NotAllowed,
            C::Pointer,
            C::NwResize,
            C::NResize,
            C::NeResize,
            C::EResize,
            C::SeResize,
            C::SResize,
            C::SwResize,
            C::WResize,
            C::ContextMenu,
            C::Help,
            C::Cell,
            C::VerticalText,
            C::Alias,
            C::Copy,
            C::NoDrop,
            C::Grab,
            C::Grabbing,
            C::ColResize,
            C::RowResize,
            C::AllScroll,
            C::ZoomIn,
            C::ZoomOut,
        ];
        usize::try_from(index)
            .ok()
            .and_then(|i| ALL.get(i).copied())
    }
}

/// A backend-specific mouse feature the mouse code branches on
/// (upstream tests the corresponding `SDL_Mouse` function pointer for `NULL`).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum MouseFeature {
    /// `mouse->WarpMouse`
    WarpMouse,
    /// `mouse->WarpMouseGlobal`
    WarpMouseGlobal,
    /// `mouse->SetRelativeMouseMode`
    SetRelativeMouseMode,
    /// `mouse->CaptureMouse`
    CaptureMouse,
    /// `mouse->GetGlobalMouseState`
    GetGlobalMouseState,
    /// `mouse->ApplySystemScale`
    ApplySystemScale,
    /// `mouse->MoveCursor`
    MoveCursor,
    /// `mouse->have_explicit_warp_event`
    ExplicitWarpEvent,
}

/// A mouse cursor. Translation of `SDL_Cursor`.
///
/// Cursors are reference-counted handles; the backend's per-cursor data
/// (`SDL_CursorData`) is the opaque `internal` payload.
#[derive(Clone)]
pub struct Cursor(Arc<CursorInner>);

struct CursorInner {
    internal: Option<Box<dyn Any + Send + Sync>>,
    animation: Option<CursorAnimation>,
}

/// An animation of cursor frames, for backends without animated cursors.
/// Translation of `SDL_CursorAnimation`.
struct CursorAnimation {
    frames: Vec<Cursor>,
    durations: Vec<u32>,
    /// `current_frame` and `last_update` (in milliseconds)
    state: Mutex<(usize, u64)>,
}

impl CursorAnimation {
    fn state(&self) -> std::sync::MutexGuard<'_, (usize, u64)> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The frame being shown.
    fn current(&self) -> Cursor {
        self.frames[self.state().0].clone()
    }
}

/// One frame of an animated cursor. Translation of `SDL_CursorFrameInfo`.
#[derive(Clone, Copy, Debug)]
pub struct CursorFrame<'a> {
    /// The image of the frame
    pub surface: &'a Surface<'a>,
    /// How long to show the frame, in milliseconds; 0 stops the animation
    /// on this frame
    pub duration: u32,
}

impl Cursor {
    /// A cursor carrying backend data (used by video backends).
    pub fn with_internal(internal: impl Any + Send + Sync) -> Cursor {
        Cursor(Arc::new(CursorInner {
            internal: Some(Box::new(internal)),
            animation: None,
        }))
    }

    /// A cursor without backend data (for backends that don't support true cursors).
    pub fn placeholder() -> Cursor {
        Cursor(Arc::new(CursorInner {
            internal: None,
            animation: None,
        }))
    }

    /// Whether this cursor is animated by the mouse code (the backend
    /// doesn't animate cursors itself).
    pub fn is_animated(&self) -> bool {
        self.0.animation.is_some()
    }

    /// The backend data, if any.
    pub fn internal<T: Any>(&self) -> Option<&T> {
        self.0
            .internal
            .as_deref()
            .and_then(|a| a.downcast_ref::<T>())
    }
}

impl PartialEq for Cursor {
    fn eq(&self, other: &Cursor) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for Cursor {}

impl std::fmt::Debug for Cursor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Cursor({:p})", Arc::as_ptr(&self.0))
    }
}

/// A user-defined mouse input transform applied in relative mode.
/// Translation of `SDL_MouseMotionTransformCallback`.
pub type MouseMotionTransform =
    dyn Fn(Duration, Option<WindowID>, MouseID, f32, f32) -> (f32, f32) + Send + Sync;

/// Translation of `SDL_MouseClickState`.
#[derive(Clone, Copy, Default)]
struct MouseClickState {
    last_timestamp: u64,
    click_motion_x: f64,
    click_motion_y: f64,
    click_count: u8,
}

/// Translation of `SDL_MouseInputSource`.
struct MouseInputSource {
    mouse_id: MouseID,
    buttonstate: MouseButtonFlags,
    // Data for double-click tracking
    clickstate: Vec<MouseClickState>,
}

/// Translation of `struct SDL_Mouse` (minus the backend function pointers,
/// which live on [`VideoHooks`]).
pub(crate) struct Mouse {
    // User-defined mouse input transform applied in relative mode
    input_transform: Option<Arc<MouseMotionTransform>>,

    // integer mode data
    /// 1 to enable mouse quantization, 2 to enable wheel quantization
    integer_mode_flags: u8,
    integer_mode_residual_motion_x: f32,
    integer_mode_residual_motion_y: f32,

    // Data common to all mice
    focus: Option<WindowID>,
    x: f32,
    y: f32,
    x_accu: f32,
    y_accu: f32,
    /// the last reported x and y coordinates
    last_x: f32,
    last_y: f32,
    residual_scroll_x: f32,
    residual_scroll_y: f32,
    click_motion_x: f64,
    click_motion_y: f64,
    has_position: bool,
    relative_mode: bool,
    relative_mode_warp_motion: bool,
    relative_mode_hide_cursor: bool,
    #[allow(dead_code)]
    relative_mode_center: bool,
    warp_emulation_hint: bool,
    warp_emulation_active: bool,
    warp_emulation_prohibited: bool,
    last_center_warp_time: Duration,
    enable_normal_speed_scale: bool,
    normal_speed_scale: f32,
    enable_relative_speed_scale: bool,
    relative_speed_scale: f32,
    enable_relative_system_scale: bool,
    double_click_time: u32,
    double_click_radius: i32,
    pub(crate) touch_mouse_events: bool,
    pub(crate) mouse_touch_events: bool,
    pub(crate) pen_mouse_events: bool,
    pub(crate) pen_touch_events: bool,
    /// Was a touch-mouse event pending?
    was_touch_mouse_events: bool,
    /// did we `add_touch()` a virtual touch device for the mouse?
    added_mouse_touch_device: bool,
    /// did we `add_touch()` a virtual touch device for pens?
    pub(crate) added_pen_touch_device: bool,
    auto_capture: bool,
    capture_desired: bool,
    capture_window: Option<WindowID>,

    // Data for input source state
    sources: Vec<MouseInputSource>,

    cursors: Vec<Cursor>,
    def_cursor: Option<Cursor>,
    cur_cursor: Option<Cursor>,
    cursor_visible: bool,

    // SDL_mice / SDL_mouse_names / SDL_mouse_initialized
    mice: Vec<MouseID>,
    mouse_names: Vec<(MouseID, String)>,
    initialized: bool,
    hint_callbacks: Vec<hints::Callback>,
}

impl Mouse {
    const fn zeroed() -> Mouse {
        Mouse {
            input_transform: None,
            integer_mode_flags: 0,
            integer_mode_residual_motion_x: 0.0,
            integer_mode_residual_motion_y: 0.0,
            focus: None,
            x: 0.0,
            y: 0.0,
            x_accu: 0.0,
            y_accu: 0.0,
            last_x: 0.0,
            last_y: 0.0,
            residual_scroll_x: 0.0,
            residual_scroll_y: 0.0,
            click_motion_x: 0.0,
            click_motion_y: 0.0,
            has_position: false,
            relative_mode: false,
            relative_mode_warp_motion: false,
            relative_mode_hide_cursor: false,
            relative_mode_center: false,
            warp_emulation_hint: false,
            warp_emulation_active: false,
            warp_emulation_prohibited: false,
            last_center_warp_time: Duration::ZERO,
            enable_normal_speed_scale: false,
            normal_speed_scale: 0.0,
            enable_relative_speed_scale: false,
            relative_speed_scale: 0.0,
            enable_relative_system_scale: false,
            double_click_time: 0,
            double_click_radius: 0,
            touch_mouse_events: false,
            mouse_touch_events: false,
            pen_mouse_events: false,
            pen_touch_events: false,
            was_touch_mouse_events: false,
            added_mouse_touch_device: false,
            added_pen_touch_device: false,
            auto_capture: false,
            capture_desired: false,
            capture_window: None,
            sources: Vec::new(),
            cursors: Vec::new(),
            def_cursor: None,
            cur_cursor: None,
            cursor_visible: false,
            mice: Vec::new(),
            mouse_names: Vec::new(),
            initialized: false,
            hint_callbacks: Vec::new(),
        }
    }
}

/// The mouse state. Translation of `SDL_mouse` and friends.
///
/// The lock is recursive so hint callbacks and event watchers may call back
/// into this module; the `RefCell` borrow is always released before calling
/// out (event queue, window events, video hooks).
static MOUSE: ReentrantMutex<RefCell<Mouse>> = ReentrantMutex::new(RefCell::new(Mouse::zeroed()));

/// for mapping mouse events to touch. Translation of `track_mouse_down`.
static TRACK_MOUSE_DOWN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Run `f` on the mouse state with the lock held. Translation of `SDL_GetMouse()`.
pub(crate) fn with_mouse<R>(f: impl FnOnce(&mut Mouse) -> R) -> R {
    let guard = MOUSE.lock();
    let mut mouse = guard.borrow_mut();
    f(&mut mouse)
}

fn video() -> Option<Arc<dyn VideoHooks>> {
    window::video()
}

fn supports(feature: MouseFeature) -> bool {
    video().is_some_and(|v| v.supports_mouse_feature(feature))
}

fn window_core(id: Option<WindowID>) -> Option<WindowCore> {
    id.and_then(window::window)
}

// ---------------------------------------------------------------------------
// Hint callbacks
// ---------------------------------------------------------------------------

fn mouse_double_click_time_changed(hint: Option<&str>) {
    with_mouse(|mouse| match hint {
        Some(h) if !h.is_empty() => mouse.double_click_time = crate::stdlib::atoi(h) as u32,
        _ => mouse.double_click_time = 500, // (Windows: GetDoubleClickTime())
    });
}

fn mouse_double_click_radius_changed(hint: Option<&str>) {
    with_mouse(|mouse| match hint {
        Some(h) if !h.is_empty() => mouse.double_click_radius = crate::stdlib::atoi(h),
        _ => mouse.double_click_radius = 32, // 32 pixels seems about right for touch interfaces
    });
}

fn mouse_normal_speed_scale_changed(hint: Option<&str>) {
    with_mouse(|mouse| match hint {
        Some(h) if !h.is_empty() => {
            mouse.enable_normal_speed_scale = true;
            mouse.normal_speed_scale = crate::stdlib::atof(h) as f32;
        }
        _ => {
            mouse.enable_normal_speed_scale = false;
            mouse.normal_speed_scale = 1.0;
        }
    });
}

fn mouse_relative_speed_scale_changed(hint: Option<&str>) {
    with_mouse(|mouse| match hint {
        Some(h) if !h.is_empty() => {
            mouse.enable_relative_speed_scale = true;
            mouse.relative_speed_scale = crate::stdlib::atof(h) as f32;
        }
        _ => {
            mouse.enable_relative_speed_scale = false;
            mouse.relative_speed_scale = 1.0;
        }
    });
}

fn mouse_relative_mode_center_changed(hint: Option<&str>) {
    with_mouse(|mouse| mouse.relative_mode_center = hints::string_to_bool(hint, true));
}

fn mouse_relative_system_scale_changed(hint: Option<&str>) {
    with_mouse(|mouse| mouse.enable_relative_system_scale = hints::string_to_bool(hint, false));
}

fn mouse_warp_emulation_changed(hint: Option<&str>) {
    let disable = with_mouse(|mouse| {
        mouse.warp_emulation_hint = hints::string_to_bool(hint, true);
        !mouse.warp_emulation_hint && mouse.warp_emulation_active
    });
    if disable {
        let _ = set_relative_mouse_mode(false);
        with_mouse(|mouse| mouse.warp_emulation_active = false);
    }
}

fn touch_mouse_events_changed(hint: Option<&str>) {
    with_mouse(|mouse| mouse.touch_mouse_events = hints::string_to_bool(hint, true));
}

fn mouse_touch_events_changed(hint: Option<&str>) {
    let default_value = cfg!(any(target_os = "android", target_os = "ios"));
    let (add, del) = with_mouse(|mouse| {
        mouse.mouse_touch_events = hints::string_to_bool(hint, default_value);
        if mouse.mouse_touch_events {
            if !mouse.added_mouse_touch_device {
                mouse.added_mouse_touch_device = true;
                return (true, false);
            }
        } else if mouse.added_mouse_touch_device {
            mouse.added_mouse_touch_device = false;
            return (false, true);
        }
        (false, false)
    });
    if add {
        super::touch::add_touch(
            super::touch::MOUSE_TOUCH_ID,
            super::touch::TouchDeviceType::Direct,
            "mouse_input",
        );
    }
    if del {
        super::touch::del_touch(super::touch::MOUSE_TOUCH_ID);
    }
}

fn pen_mouse_events_changed(hint: Option<&str>) {
    with_mouse(|mouse| mouse.pen_mouse_events = hints::string_to_bool(hint, true));
}

fn mouse_auto_capture_changed(hint: Option<&str>) {
    let auto_capture = hints::string_to_bool(hint, true);
    let changed = with_mouse(|mouse| {
        if auto_capture != mouse.auto_capture {
            mouse.auto_capture = auto_capture;
            true
        } else {
            false
        }
    });
    if changed {
        let _ = update_mouse_capture(false);
    }
}

fn mouse_relative_warp_motion_changed(hint: Option<&str>) {
    with_mouse(|mouse| mouse.relative_mode_warp_motion = hints::string_to_bool(hint, false));
}

fn mouse_relative_cursor_visible_changed(hint: Option<&str>) {
    with_mouse(|mouse| mouse.relative_mode_hide_cursor = !hints::string_to_bool(hint, false));
    redraw_cursor(); // Update cursor visibility
}

fn mouse_integer_mode_changed(hint: Option<&str>) {
    with_mouse(|mouse| match hint {
        Some(h) if !h.is_empty() => mouse.integer_mode_flags = crate::stdlib::atoi(h) as u8,
        _ => mouse.integer_mode_flags = 0,
    });
}

// Public functions

/// Initialize the mouse subsystem, called before the main video driver is initialized.
/// Translation of `SDL_PreInitMouse()`.
pub fn pre_init_mouse() -> Result<()> {
    with_mouse(|mouse| {
        *mouse = Mouse::zeroed();
        mouse.initialized = true;
    });

    let mut callbacks = Vec::new();
    callbacks.push(hints::watch(hints::MOUSE_DOUBLE_CLICK_TIME, |c| {
        mouse_double_click_time_changed(c.new_value)
    })?);
    callbacks.push(hints::watch(hints::MOUSE_DOUBLE_CLICK_RADIUS, |c| {
        mouse_double_click_radius_changed(c.new_value)
    })?);
    callbacks.push(hints::watch(hints::MOUSE_NORMAL_SPEED_SCALE, |c| {
        mouse_normal_speed_scale_changed(c.new_value)
    })?);
    callbacks.push(hints::watch(hints::MOUSE_RELATIVE_SPEED_SCALE, |c| {
        mouse_relative_speed_scale_changed(c.new_value)
    })?);
    callbacks.push(hints::watch(hints::MOUSE_RELATIVE_SYSTEM_SCALE, |c| {
        mouse_relative_system_scale_changed(c.new_value)
    })?);
    callbacks.push(hints::watch(hints::MOUSE_RELATIVE_MODE_CENTER, |c| {
        mouse_relative_mode_center_changed(c.new_value)
    })?);
    callbacks.push(hints::watch(
        hints::MOUSE_EMULATE_WARP_WITH_RELATIVE,
        |c| mouse_warp_emulation_changed(c.new_value),
    )?);
    callbacks.push(hints::watch(hints::TOUCH_MOUSE_EVENTS, |c| {
        touch_mouse_events_changed(c.new_value)
    })?);
    callbacks.push(hints::watch(hints::MOUSE_TOUCH_EVENTS, |c| {
        mouse_touch_events_changed(c.new_value)
    })?);
    callbacks.push(hints::watch(hints::PEN_MOUSE_EVENTS, |c| {
        pen_mouse_events_changed(c.new_value)
    })?);
    callbacks.push(hints::watch(hints::MOUSE_AUTO_CAPTURE, |c| {
        mouse_auto_capture_changed(c.new_value)
    })?);
    callbacks.push(hints::watch(hints::MOUSE_RELATIVE_WARP_MOTION, |c| {
        mouse_relative_warp_motion_changed(c.new_value)
    })?);
    callbacks.push(hints::watch(hints::MOUSE_RELATIVE_CURSOR_VISIBLE, |c| {
        mouse_relative_cursor_visible_changed(c.new_value)
    })?);
    callbacks.push(hints::watch("SDL_MOUSE_INTEGER_MODE", |c| {
        mouse_integer_mode_changed(c.new_value)
    })?);

    with_mouse(|mouse| {
        mouse.hint_callbacks = callbacks;
        mouse.was_touch_mouse_events = false; // no touch to mouse movement event pending
        mouse.cursor_visible = true;
    });
    Ok(())
}

/// Finish initializing the mouse subsystem, called after the main video driver was initialized.
/// Translation of `SDL_PostInitMouse()`.
pub fn post_init_mouse() {
    /* Create a dummy mouse cursor for video backends that don't support true cursors,
     * so that mouse grab and focus functionality will work.
     */
    if with_mouse(|mouse| mouse.def_cursor.is_none()) {
        if let Ok(surface) = Surface::new(1, 1, PixelFormat::ARGB8888) {
            set_default_cursor(create_color_cursor(&surface, 0, 0).ok());
        }
    }
}

/// Return whether a device is actually a mouse. Translation of `SDL_IsMouse()`.
pub fn is_mouse(_vendor: u16, _product: u16) -> bool {
    // Eventually we'll have a blacklist of devices that enumerate as mice but aren't really
    true
}

/// A mouse has been added to the system. Translation of `SDL_AddMouse()`.
pub fn add_mouse(mouse_id: MouseID, name: Option<&str>) {
    let added = with_mouse(|mouse| {
        if mouse.mice.contains(&mouse_id) {
            // We already know about this mouse
            return false;
        }
        debug_assert!(mouse_id != 0);
        mouse.mice.push(mouse_id);
        mouse
            .mouse_names
            .push((mouse_id, name.unwrap_or("Mouse").to_owned()));
        true
    });
    if !added {
        return;
    }

    let _ = queue::push(Event::MouseDevice(MouseDeviceEvent {
        event_type: EventType::MOUSE_ADDED,
        timestamp: Duration::ZERO,
        which: mouse_id,
    }));
}

/// A mouse has been removed from the system. Translation of `SDL_RemoveMouse()`.
pub fn remove_mouse(mouse_id: MouseID) {
    let initialized = with_mouse(|mouse| {
        let Some(index) = mouse.mice.iter().position(|&id| id == mouse_id) else {
            // We don't know about this mouse
            return None;
        };
        mouse.mice.remove(index);

        // Remove any mouse input sources for this mouseID
        if let Some(i) = mouse.sources.iter().position(|s| s.mouse_id == mouse_id) {
            mouse.sources.remove(i);
        }
        Some(mouse.initialized)
    });

    if initialized == Some(true) {
        let _ = queue::push(Event::MouseDevice(MouseDeviceEvent {
            event_type: EventType::MOUSE_REMOVED,
            timestamp: Duration::ZERO,
            which: mouse_id,
        }));
    }
}

/// Whether a mouse is currently connected. Translation of `SDL_HasMouse()`.
pub fn has_mouse() -> bool {
    with_mouse(|mouse| !mouse.mice.is_empty())
}

/// The connected mouse ids. Translation of `SDL_GetMice()`.
pub fn mice() -> Vec<MouseID> {
    with_mouse(|mouse| mouse.mice.clone())
}

/// The name of a mouse. Translation of `SDL_GetMouseNameForID()`.
pub fn mouse_name(instance_id: MouseID) -> Result<String> {
    match instance_id {
        GLOBAL_MOUSE_ID => Ok("Mouse".to_owned()),
        TOUCH_MOUSE_ID => {
            // We can't tell which touch device it was, just use the first one
            let name = super::touch::touch_devices()
                .first()
                .and_then(|&id| super::touch::touch_device_name(id).ok());
            Ok(name.unwrap_or_else(|| "Touch".to_owned()))
        }
        PEN_MOUSE_ID => Ok("Pen".to_owned()),
        _ => with_mouse(|mouse| {
            mouse
                .mouse_names
                .iter()
                .find(|(id, _)| *id == instance_id)
                .map(|(_, n)| n.clone())
        })
        .ok_or_else(|| err!("Mouse {instance_id} not found")),
    }
}

/// Set the default mouse cursor. Translation of `SDL_SetDefaultCursor()`.
pub fn set_default_cursor(cursor: Option<Cursor>) {
    let set_current = with_mouse(|mouse| {
        if cursor == mouse.def_cursor {
            return None;
        }

        if let Some(default_cursor) = mouse.def_cursor.take() {
            if mouse.cur_cursor == Some(default_cursor.clone()) {
                mouse.cur_cursor = None;
            }
            mouse.cursors.retain(|c| *c != default_cursor);
            // (FreeCursor / SDL_free: the Arc drops the backend data)
        }

        mouse.def_cursor = cursor.clone();
        if mouse.cur_cursor.is_none() {
            Some(cursor.clone())
        } else {
            None
        }
    });
    if let Some(cursor) = set_current {
        let _ = set_cursor(cursor.as_ref());
    }
}

/// Get the preferred default system cursor. Translation of `SDL_GetDefaultSystemCursor()`.
pub fn default_system_cursor() -> SystemCursor {
    let mut id = SystemCursor::Default;
    if let Some(value) = hints::get(hints::MOUSE_DEFAULT_SYSTEM_CURSOR) {
        if let Some(c) = SystemCursor::from_index(crate::stdlib::atoi(&value)) {
            id = c;
        }
    }
    id
}

/// Translation of `SDL_GetMouseButtonState()`.
fn mouse_button_state(mouse: &Mouse, mouse_id: MouseID, include_touch: bool) -> MouseButtonFlags {
    let mut buttonstate = MouseButtonFlags::NONE;

    for source in &mouse.sources {
        if mouse_id == GLOBAL_MOUSE_ID || mouse_id == TOUCH_MOUSE_ID {
            if include_touch || source.mouse_id != TOUCH_MOUSE_ID {
                buttonstate |= source.buttonstate;
            }
        } else if mouse_id == source.mouse_id {
            buttonstate |= source.buttonstate;
            break;
        }
    }
    buttonstate
}

/// The window with mouse focus, if any. Translation of `SDL_GetMouseFocus()`.
pub fn mouse_focus() -> Option<WindowID> {
    with_mouse(|mouse| mouse.focus)
}

/// The current mouse position relative to the focus window (internal helper).
pub(crate) fn position() -> (f32, f32) {
    with_mouse(|mouse| (mouse.x, mouse.y))
}

/* TODO RECONNECT: Hello from the Wayland video driver!
 * This was once removed from SDL, but it's been added back in comment form
 * because we will need it when Wayland adds compositor reconnect support.
 * If you need this before we do, great! Otherwise, leave this alone, we'll
 * uncomment it at the right time.
 * -flibit
 */
// (SDL_ResetMouse() is `#if 0` upstream and not translated.)

/// Set the mouse focus window. Translation of `SDL_SetMouseFocus()`.
pub fn set_mouse_focus(window_id: Option<WindowID>) {
    let old = with_mouse(|mouse| mouse.focus);
    if old == window_id {
        return;
    }

    /* Actually, this ends up being a bad idea, because most operating
       systems have an implicit grab when you press the mouse button down
       so you can drag things out of the window and then get the mouse up
       when it happens.  So, #if 0...
    */

    // See if the current window has lost focus
    if let Some(old) = old {
        window::send_window_event(old, EventType::WINDOW_MOUSE_LEAVE, 0, 0);
    }

    with_mouse(|mouse| {
        mouse.focus = window_id;
        mouse.has_position = false;
    });

    if let Some(new) = window_id {
        window::send_window_event(new, EventType::WINDOW_MOUSE_ENTER, 0, 0);
    }

    // Update cursor visibility
    redraw_cursor();
}

/// Check if mouse position is within window or captured by window.
/// Translation of `SDL_MousePositionInWindow()`.
pub fn mouse_position_in_window(window: Option<&WindowCore>, x: f32, y: f32) -> bool {
    let Some(window) = window else {
        return false;
    };

    if !window.flags.contains(WindowFlags::MOUSE_CAPTURE)
        && (x < 0.0 || y < 0.0 || x >= window.w as f32 || y >= window.h as f32)
    {
        return false;
    }
    true
}

/// Check to see if we need to synthesize focus events. Translation of `SDL_UpdateMouseFocus()`.
fn update_mouse_focus(
    window_id: WindowID,
    x: f32,
    y: f32,
    _buttonstate: MouseButtonFlags,
    send_mouse_motion: bool,
) -> bool {
    let window = window::window(window_id);
    let in_window = mouse_position_in_window(window.as_ref(), x, y);
    let focus = with_mouse(|mouse| mouse.focus);

    if !in_window {
        if Some(window_id) == focus {
            if send_mouse_motion {
                private_send_mouse_motion(
                    Duration::ZERO,
                    Some(window_id),
                    GLOBAL_MOUSE_ID,
                    false,
                    x,
                    y,
                );
            }
            set_mouse_focus(None);
        }
        return false;
    }

    if Some(window_id) != focus {
        set_mouse_focus(Some(window_id));
        if send_mouse_motion {
            private_send_mouse_motion(
                Duration::ZERO,
                Some(window_id),
                GLOBAL_MOUSE_ID,
                false,
                x,
                y,
            );
        }
    }
    true
}

/// Send a mouse motion event. Translation of `SDL_SendMouseMotion()`.
///
/// `relative` means `x`/`y` are deltas; `timestamp` zero means "now".
pub fn send_mouse_motion(
    timestamp: Duration,
    window: Option<WindowID>,
    mouse_id: MouseID,
    relative: bool,
    x: f32,
    y: f32,
) {
    if let Some(window_id) = window {
        if !relative {
            let buttonstate = with_mouse(|mouse| mouse_button_state(mouse, mouse_id, true));
            if !update_mouse_focus(
                window_id,
                x,
                y,
                buttonstate,
                mouse_id != TOUCH_MOUSE_ID && mouse_id != PEN_MOUSE_ID,
            ) {
                return;
            }
        }
    }
    private_send_mouse_motion(timestamp, window, mouse_id, relative, x, y);
}

/// Send a mouse motion event resulting from a pointer warp. Translation of `SDL_SendMouseWarp()`.
pub fn send_mouse_warp(
    timestamp: Duration,
    window: Option<WindowID>,
    mouse_id: MouseID,
    x: f32,
    y: f32,
) {
    let handled = with_mouse(|mouse| {
        // Ignore the previous position when we warp, as warps don't generate relative motion.
        mouse.last_x = x;
        mouse.last_y = y;
        mouse.has_position = false;

        if mouse.relative_mode {
            /* Sending motion events when warping while relative mode is active can confuse
             * clients that don't expect it, so just update the absolute position and don't
             * generate a motion event unless SDL_HINT_MOUSE_RELATIVE_WARP_MOTION is set.
             */
            if !mouse.relative_mode_warp_motion {
                mouse.x = x;
                mouse.y = y;
                mouse.has_position = true;
                return true;
            }
        }
        false
    });
    if handled {
        return;
    }

    send_mouse_motion(timestamp, window, mouse_id, false, x, y);
}

/// Translation of `ConstrainMousePosition()`.
fn constrain_mouse_position(mouse: &Mouse, window: Option<&WindowCore>, x: &mut f32, y: &mut f32) {
    /* make sure that the pointers find themselves inside the windows,
    unless we have the mouse captured. */
    if let Some(window) = window {
        if window.flags.contains(WindowFlags::MOUSE_CAPTURE) {
            return;
        }
        let (mut x_min, mut x_max) = (0, window.w - 1);
        let (mut y_min, mut y_max) = (0, window.h - 1);

        if !window.mouse_rect.is_empty() {
            let confine = window.mouse_rect;
            let window_rect = Rect {
                x: 0,
                y: 0,
                w: x_max + 1,
                h: y_max + 1,
            };
            if let Some(mouse_rect) = confine.intersection(&window_rect) {
                x_min = mouse_rect.x;
                y_min = mouse_rect.y;
                x_max = x_min + mouse_rect.w - 1;
                y_max = y_min + mouse_rect.h - 1;
            }
        }

        if *x >= (x_max + 1) as f32 {
            *x = (x_max as f32).max(mouse.last_x);
        }
        if *x < x_min as f32 {
            *x = x_min as f32;
        }

        if *y >= (y_max + 1) as f32 {
            *y = (y_max as f32).max(mouse.last_y);
        }
        if *y < y_min as f32 {
            *y = y_min as f32;
        }
    }
}

/// Translation of `SDL_PrivateSendMouseMotion()`.
fn private_send_mouse_motion(
    timestamp: Duration,
    window_id: Option<WindowID>,
    mut mouse_id: MouseID,
    relative: bool,
    mut x: f32,
    mut y: f32,
) {
    let mut xrel = 0.0f32;
    let mut yrel = 0.0f32;
    let window = window_core(window_id);

    let (mouse_touch_events, touch_mouse_events, relative_mode, focus) = with_mouse(|mouse| {
        (
            mouse.mouse_touch_events,
            mouse.touch_mouse_events,
            mouse.relative_mode,
            mouse.focus,
        )
    });
    let window_is_relative =
        window_core(focus).is_some_and(|f| f.flags.contains(WindowFlags::MOUSE_RELATIVE_MODE));

    // SDL_HINT_MOUSE_TOUCH_EVENTS: controlling whether mouse events should generate synthetic touch events
    if mouse_touch_events
        && mouse_id != TOUCH_MOUSE_ID
        && mouse_id != PEN_MOUSE_ID
        && !relative
        && TRACK_MOUSE_DOWN.load(std::sync::atomic::Ordering::Relaxed)
    {
        if let Some(window) = &window {
            let normalized_x = x / window.w as f32;
            let normalized_y = y / window.h as f32;
            super::touch::send_touch_motion(
                timestamp,
                super::touch::MOUSE_TOUCH_ID,
                BUTTON_LEFT as super::touch::FingerID,
                window_id,
                normalized_x,
                normalized_y,
                1.0,
            );
        }
    }

    // SDL_HINT_TOUCH_MOUSE_EVENTS: if not set, discard synthetic mouse events coming from platform layer
    if !touch_mouse_events && mouse_id == TOUCH_MOUSE_ID {
        return;
    }

    if relative {
        if relative_mode {
            let transform = with_mouse(|mouse| mouse.input_transform.clone());
            if let Some(transform) = transform {
                (x, y) = transform(timestamp, window_id, mouse_id, x, y);
            } else {
                let (sys_scale, speed_scale) = with_mouse(|mouse| {
                    (
                        mouse.enable_relative_system_scale,
                        mouse
                            .enable_relative_speed_scale
                            .then_some(mouse.relative_speed_scale),
                    )
                });
                if sys_scale {
                    if let Some(video) = video() {
                        if let Some((sx, sy)) =
                            video.apply_system_scale(timestamp, window_id, mouse_id, x, y)
                        {
                            (x, y) = (sx, sy);
                        }
                    }
                }
                if let Some(scale) = speed_scale {
                    x *= scale;
                    y *= scale;
                }
            }
        } else {
            with_mouse(|mouse| {
                if mouse.enable_normal_speed_scale {
                    x *= mouse.normal_speed_scale;
                    y *= mouse.normal_speed_scale;
                }
            });
        }
    }

    let event = with_mouse(|mouse| {
        if relative {
            if (mouse.integer_mode_flags & 1) != 0 {
                // Accumulate the fractional relative motion and only process the integer portion
                let (ix, fx) = modff(mouse.integer_mode_residual_motion_x + x);
                let (iy, fy) = modff(mouse.integer_mode_residual_motion_y + y);
                mouse.integer_mode_residual_motion_x = fx;
                mouse.integer_mode_residual_motion_y = fy;
                x = ix;
                y = iy;
            }
            xrel = x;
            yrel = y;
            x = mouse.last_x + xrel;
            y = mouse.last_y + yrel;
            constrain_mouse_position(mouse, window.as_ref(), &mut x, &mut y);
        } else {
            if (mouse.integer_mode_flags & 1) != 0 {
                // Discard the fractional component from absolute coordinates
                x = x.trunc();
                y = y.trunc();
            }
            constrain_mouse_position(mouse, window.as_ref(), &mut x, &mut y);
            if mouse.has_position {
                xrel = x - mouse.last_x;
                yrel = y - mouse.last_y;
            }
        }

        if mouse.has_position && xrel == 0.0 && yrel == 0.0 {
            // Drop events that don't change state
            return None;
        }

        // Ignore relative motion positioning the first touch
        if mouse_id == TOUCH_MOUSE_ID && mouse_button_state(mouse, mouse_id, true).is_empty() {
            xrel = 0.0;
            yrel = 0.0;
        }

        // modify internal state
        {
            mouse.x_accu += xrel;
            mouse.y_accu += yrel;

            if relative && mouse.has_position {
                mouse.x += xrel;
                mouse.y += yrel;
                let (mut mx, mut my) = (mouse.x, mouse.y);
                constrain_mouse_position(mouse, window.as_ref(), &mut mx, &mut my);
                mouse.x = mx;
                mouse.y = my;
            } else {
                mouse.x = x;
                mouse.y = y;
            }
            mouse.has_position = true;

            // Use unclamped values if we're getting events outside the window
            mouse.last_x = if relative { mouse.x } else { x };
            mouse.last_y = if relative { mouse.y } else { y };

            mouse.click_motion_x += xrel as f64;
            mouse.click_motion_y += yrel as f64;
        }

        // Move the mouse cursor, if needed
        let move_cursor = if mouse.cursor_visible && !mouse.relative_mode {
            mouse.cur_cursor.clone()
        } else {
            None
        };

        // Post the event, if desired
        if !queue::event_enabled(EventType::MOUSE_MOTION) {
            return Some((move_cursor, None));
        }
        if (!mouse.relative_mode || mouse.warp_emulation_active)
            && mouse_id != TOUCH_MOUSE_ID
            && mouse_id != PEN_MOUSE_ID
        {
            // We're not in relative mode, so all mouse events are global mouse events
            mouse_id = GLOBAL_MOUSE_ID;
        }

        if !relative && window_is_relative {
            if !mouse.relative_mode_warp_motion {
                return Some((move_cursor, None));
            }
            xrel = 0.0;
            yrel = 0.0;
        }

        // Set us pending (or clear during a normal mouse movement event) as having triggered
        mouse.was_touch_mouse_events = mouse_id == TOUCH_MOUSE_ID;

        Some((
            move_cursor,
            Some(Event::MouseMotion(MouseMotionEvent {
                timestamp,
                window_id: mouse.focus.unwrap_or(0),
                which: mouse_id,
                state: mouse_button_state(mouse, mouse_id, true),
                x: mouse.x,
                y: mouse.y,
                xrel,
                yrel,
            })),
        ))
    });

    let Some((move_cursor, event)) = event else {
        return;
    };
    if let Some(cursor) = move_cursor {
        if let Some(video) = video() {
            if video.supports_mouse_feature(MouseFeature::MoveCursor) {
                video.move_cursor(&cursor);
            }
        }
    }
    if let Some(event) = event {
        let _ = queue::push(event);
    }
}

/// Translation of `SDL_modff()`: returns (integer part, fractional part).
fn modff(v: f32) -> (f32, f32) {
    let i = v.trunc();
    (i, v - i)
}

/// Translation of `GetMouseInputSource()`; returns the index into `mouse.sources`.
fn get_mouse_input_source(
    mouse: &mut Mouse,
    mouse_id: MouseID,
    down: bool,
    button: u8,
) -> Option<usize> {
    if !mouse.initialized {
        return None;
    }

    let mut matched = mouse.sources.iter().position(|s| s.mouse_id == mouse_id);

    if !down && !matched.is_some_and(|i| mouse.sources[i].buttonstate.is_pressed(button)) {
        /* This might be a button release from a transition between mouse messages and raw input.
         * See if there's another mouse source that already has that button down and use that.
         */
        if let Some(i) = mouse
            .sources
            .iter()
            .position(|s| s.buttonstate.is_pressed(button))
        {
            matched = Some(i);
        }
    }

    if matched.is_some() {
        return matched;
    }

    mouse.sources.push(MouseInputSource {
        mouse_id,
        buttonstate: MouseButtonFlags::NONE,
        clickstate: Vec::new(),
    });
    Some(mouse.sources.len() - 1)
}

/// Translation of `GetMouseClickState()`.
fn get_mouse_click_state(source: &mut MouseInputSource, button: u8) -> &mut MouseClickState {
    let button = button as usize;
    if button >= source.clickstate.len() {
        source
            .clickstate
            .resize(button + 1, MouseClickState::default());
    }
    &mut source.clickstate[button]
}

/// Translation of `SDL_PrivateSendMouseButton()`; `clicks < 0` means "count them".
fn private_send_mouse_button(
    timestamp: Duration,
    window_id: Option<WindowID>,
    mut mouse_id: MouseID,
    button: u8,
    down: bool,
    mut clicks: i32,
) {
    let Some(source_index) =
        with_mouse(|mouse| get_mouse_input_source(mouse, mouse_id, down, button))
    else {
        return;
    };

    let (mouse_touch_events, touch_mouse_events, mut buttonstate) = with_mouse(|mouse| {
        (
            mouse.mouse_touch_events,
            mouse.touch_mouse_events,
            mouse.sources[source_index].buttonstate,
        )
    });

    // SDL_HINT_MOUSE_TOUCH_EVENTS: controlling whether mouse events should generate synthetic touch events
    if mouse_touch_events
        && mouse_id != TOUCH_MOUSE_ID
        && mouse_id != PEN_MOUSE_ID
        && button == BUTTON_LEFT
    {
        TRACK_MOUSE_DOWN.store(down, std::sync::atomic::Ordering::Relaxed);
        if let Some(window) = window_core(window_id) {
            let ty = if down {
                EventType::FINGER_DOWN
            } else {
                EventType::FINGER_UP
            };
            let (mx, my) = position();
            let normalized_x = mx / window.w as f32;
            let normalized_y = my / window.h as f32;
            super::touch::send_touch(
                timestamp,
                super::touch::MOUSE_TOUCH_ID,
                BUTTON_LEFT as super::touch::FingerID,
                window_id,
                ty,
                normalized_x,
                normalized_y,
                1.0,
            );
        }
    }

    // SDL_HINT_TOUCH_MOUSE_EVENTS: if not set, discard synthetic mouse events coming from platform layer
    if !touch_mouse_events && mouse_id == TOUCH_MOUSE_ID {
        return;
    }

    // Figure out which event to perform
    let ty = if down {
        buttonstate |= MouseButtonFlags::mask(button);
        EventType::MOUSE_BUTTON_DOWN
    } else {
        buttonstate &= !MouseButtonFlags::mask(button);
        EventType::MOUSE_BUTTON_UP
    };

    // We do this after calculating buttonstate so button presses gain focus
    if let Some(window_id) = window_id {
        if down {
            let (mx, my) = position();
            update_mouse_focus(window_id, mx, my, buttonstate, true);
        }
    }

    let event = with_mouse(|mouse| {
        // The source list may have changed while focus was updated; re-find it.
        let source_index = get_mouse_input_source(mouse, mouse_id, down, button)?;
        if buttonstate == mouse.sources[source_index].buttonstate {
            // Ignore this event, no state change
            return None;
        }
        mouse.sources[source_index].buttonstate = buttonstate;

        if clicks < 0 {
            let double_click_time = mouse.double_click_time as u64;
            let double_click_radius = mouse.double_click_radius as f64;
            let (cmx, cmy) = (mouse.click_motion_x, mouse.click_motion_y);
            let clickstate = get_mouse_click_state(&mut mouse.sources[source_index], button);
            if down {
                let now = timer::ticks_ms();

                if now >= clickstate.last_timestamp + double_click_time
                    || (cmx - clickstate.click_motion_x).abs() > double_click_radius
                    || (cmy - clickstate.click_motion_y).abs() > double_click_radius
                {
                    clickstate.click_count = 0;
                }
                clickstate.last_timestamp = now;
                clickstate.click_motion_x = cmx;
                clickstate.click_motion_y = cmy;
                clickstate.click_count = clickstate.click_count.saturating_add(1);
            }
            clicks = clickstate.click_count as i32;
        }

        // Post the event, if desired
        if !queue::event_enabled(ty) {
            return Some(None);
        }
        if (!mouse.relative_mode || mouse.warp_emulation_active)
            && mouse_id != TOUCH_MOUSE_ID
            && mouse_id != PEN_MOUSE_ID
        {
            // We're not in relative mode, so all mouse events are global mouse events
            mouse_id = GLOBAL_MOUSE_ID;
        } else {
            mouse_id = mouse.sources[source_index].mouse_id;
        }
        Some(Some(Event::MouseButton(MouseButtonEvent {
            timestamp,
            window_id: mouse.focus.unwrap_or(0),
            which: mouse_id,
            button,
            down,
            clicks: clicks.clamp(0, 255) as u8,
            x: mouse.x,
            y: mouse.y,
        })))
    });
    let Some(event) = event else {
        return;
    };
    if let Some(event) = event {
        let _ = queue::push(event);
    }

    // We do this after dispatching event so button releases can lose focus
    if let Some(window_id) = window_id {
        if !down {
            let (mx, my) = position();
            update_mouse_focus(window_id, mx, my, buttonstate, true);
        }
    }

    // Automatically capture the mouse while buttons are pressed
    if with_mouse(|mouse| mouse.auto_capture) {
        let _ = update_mouse_capture(false);
    }
}

/// Send a mouse button event with a click count. Translation of `SDL_SendMouseButtonClicks()`.
pub fn send_mouse_button_clicks(
    timestamp: Duration,
    window: Option<WindowID>,
    mouse_id: MouseID,
    button: u8,
    down: bool,
    clicks: i32,
) {
    let clicks = clicks.max(0);
    private_send_mouse_button(timestamp, window, mouse_id, button, down, clicks);
}

/// Send a mouse button event. Translation of `SDL_SendMouseButton()`.
pub fn send_mouse_button(
    timestamp: Duration,
    window: Option<WindowID>,
    mouse_id: MouseID,
    button: u8,
    down: bool,
) {
    private_send_mouse_button(timestamp, window, mouse_id, button, down, -1);
}

/// Send a mouse wheel event. Translation of `SDL_SendMouseWheel()`.
pub fn send_mouse_wheel(
    timestamp: Duration,
    window: Option<WindowID>,
    mut mouse_id: MouseID,
    x: f32,
    y: f32,
    direction: MouseWheelDirection,
) {
    if let Some(window) = window {
        set_mouse_focus(Some(window));
    }

    if x == 0.0 && y == 0.0 {
        return;
    }

    // Post the event, if desired
    if queue::event_enabled(EventType::MOUSE_WHEEL) {
        let event = with_mouse(|mouse| {
            if !mouse.relative_mode || mouse.warp_emulation_active {
                // We're not in relative mode, so all mouse events are global mouse events
                mouse_id = GLOBAL_MOUSE_ID;
            }

            let (integer_x, rx) = modff(mouse.residual_scroll_x + x);
            mouse.residual_scroll_x = rx;
            let (integer_y, ry) = modff(mouse.residual_scroll_y + y);
            mouse.residual_scroll_y = ry;

            // Return the accumulated values in x/y when integer wheel mode is enabled.
            // This is necessary for compatibility with sdl2-compat 2.32.54.
            let (ex, ey) = if (mouse.integer_mode_flags & 2) != 0 {
                (integer_x, integer_y)
            } else {
                (x, y)
            };

            Event::MouseWheel(MouseWheelEvent {
                timestamp,
                window_id: mouse.focus.unwrap_or(0),
                which: mouse_id,
                x: ex,
                y: ey,
                direction,
                mouse_x: mouse.x,
                mouse_y: mouse.y,
                integer_x: integer_x as i32,
                integer_y: integer_y as i32,
            })
        });
        let _ = queue::push(event);
    }
}

/// Shutdown the mouse subsystem. Translation of `SDL_QuitMouse()`.
pub fn quit_mouse() {
    let (del_mouse_touch, del_pen_touch, has_capture) = with_mouse(|mouse| {
        mouse.initialized = false;
        let a = std::mem::take(&mut mouse.added_mouse_touch_device);
        let b = std::mem::take(&mut mouse.added_pen_touch_device);
        (a, b, supports(MouseFeature::CaptureMouse))
    });
    if del_mouse_touch {
        super::touch::del_touch(super::touch::MOUSE_TOUCH_ID);
    }
    if del_pen_touch {
        super::touch::del_touch(super::touch::PEN_TOUCH_ID);
    }

    if has_capture {
        let _ = capture_mouse(false);
        let _ = update_mouse_capture(true);
    }

    let _ = set_relative_mouse_mode(false);
    show_cursor();

    if with_mouse(|mouse| mouse.def_cursor.is_some()) {
        set_default_cursor(None);
    }

    let mice: Vec<MouseID> = with_mouse(|mouse| {
        mouse.cursors.clear();
        mouse.cur_cursor = None;
        mouse.sources.clear();
        mouse.hint_callbacks.clear();
        mouse.mice.iter().rev().copied().collect()
    });
    for id in mice {
        remove_mouse(id);
    }

    with_mouse(|mouse| *mouse = Mouse::zeroed());
}

/// Set a user-defined transform for relative mouse motion (or `None` to clear it).
/// Translation of `SDL_SetRelativeMouseTransform()`.
pub fn set_relative_mouse_transform(
    transform: Option<
        impl Fn(Duration, Option<WindowID>, MouseID, f32, f32) -> (f32, f32) + Send + Sync + 'static,
    >,
) -> Result<()> {
    with_mouse(|mouse| {
        if mouse.relative_mode {
            return Err(err!(
                "Can't set mouse transform while relative mode is active"
            ));
        }
        mouse.input_transform = transform.map(|t| Arc::new(t) as Arc<MouseMotionTransform>);
        Ok(())
    })
}

/// The mouse position relative to the focus window and the button state.
/// Translation of `SDL_GetMouseState()`.
pub fn mouse_state() -> (f32, f32, MouseButtonFlags) {
    with_mouse(|mouse| {
        (
            mouse.x,
            mouse.y,
            mouse_button_state(mouse, GLOBAL_MOUSE_ID, true),
        )
    })
}

/// The motion accumulated since the last call, and the button state.
/// Translation of `SDL_GetRelativeMouseState()`.
pub fn relative_mouse_state() -> (f32, f32, MouseButtonFlags) {
    with_mouse(|mouse| {
        let r = (
            mouse.x_accu,
            mouse.y_accu,
            mouse_button_state(mouse, GLOBAL_MOUSE_ID, true),
        );
        mouse.x_accu = 0.0;
        mouse.y_accu = 0.0;
        r
    })
}

/// The mouse position in desktop coordinates and the button state.
/// Translation of `SDL_GetGlobalMouseState()`.
pub fn global_mouse_state() -> (f32, f32, MouseButtonFlags) {
    if let Some(video) = video() {
        if video.supports_mouse_feature(MouseFeature::GetGlobalMouseState) {
            if let Some(state) = video.global_mouse_state() {
                return state;
            }
        }
    }
    mouse_state()
}

/// Warp the mouse within the window, potentially overriding relative mode.
/// Translation of `SDL_PerformWarpMouseInWindow()`.
pub fn perform_warp_mouse_in_window(
    window: Option<WindowID>,
    x: f32,
    y: f32,
    ignore_relative_mode: bool,
) {
    let window_id = match window.or_else(mouse_focus) {
        Some(w) => w,
        None => return,
    };

    let Some(window) = window::window(window_id) else {
        return;
    };
    if window.flags.contains(WindowFlags::MINIMIZED) {
        return;
    }

    let have_explicit_warp_event = supports(MouseFeature::ExplicitWarpEvent);
    let (done, use_backend) = with_mouse(|mouse| {
        /* If the backend sends explicit warp events, this will be taken care of if/when the pointer actually warps,
         * Warps when in relative save the position to be applied when leaving relative mode.
         */
        if !have_explicit_warp_event || mouse.relative_mode {
            // Ignore the previous position when we warp, as warps don't generate relative motion.
            mouse.last_x = x;
            mouse.last_y = y;
            mouse.has_position = false;

            if mouse.relative_mode && !ignore_relative_mode {
                /* 2.0.22 made warping in relative mode actually functional, which
                 * surprised many applications that weren't expecting the additional
                 * mouse motion.
                 *
                 * So for now, warping in relative mode adjusts the absolute position, but
                 * doesn't generate motion events, unless SDL_HINT_MOUSE_RELATIVE_WARP_MOTION is set.
                 */
                if !mouse.relative_mode_warp_motion {
                    mouse.x = x;
                    mouse.y = y;
                    mouse.has_position = true;
                    return (true, false);
                }
            }
        }
        (false, !mouse.relative_mode)
    });
    if done {
        return;
    }

    if use_backend && supports(MouseFeature::WarpMouse) {
        if let Some(video) = video() {
            let _ = video.warp_mouse(window_id, x, y);
        }
    } else {
        private_send_mouse_motion(
            Duration::ZERO,
            Some(window_id),
            GLOBAL_MOUSE_ID,
            false,
            x,
            y,
        );
    }
}

/// Translation of `SDL_DisableMouseWarpEmulation()`.
pub fn disable_mouse_warp_emulation() {
    if with_mouse(|mouse| mouse.warp_emulation_active) {
        let _ = set_relative_mouse_mode(false);
    }
    with_mouse(|mouse| mouse.warp_emulation_prohibited = true);
}

/// Translation of `SDL_MaybeEnableWarpEmulation()`.
fn maybe_enable_warp_emulation(window: Option<WindowID>, x: f32, y: f32) {
    let eligible = with_mouse(|mouse| {
        !mouse.warp_emulation_prohibited
            && mouse.warp_emulation_hint
            && !mouse.cursor_visible
            && !mouse.warp_emulation_active
    });
    if !eligible {
        return;
    }

    let window = window_core(window.or_else(mouse_focus));
    if let Some(window) = window {
        let cx = window.w as f32 / 2.0;
        let cy = window.h as f32 / 2.0;
        if x >= cx.floor() && x <= cx.ceil() && y >= cy.floor() && y <= cy.ceil() {
            // Require two consecutive warps to the center within a certain timespan to enter warp emulation mode.
            let now = timer::ticks();
            let activate = with_mouse(|mouse| {
                let a = now.saturating_sub(mouse.last_center_warp_time) < WARP_EMULATION_THRESHOLD;
                if a {
                    mouse.warp_emulation_active = true;
                }
                a
            });
            if activate && set_relative_mouse_mode(true).is_err() {
                with_mouse(|mouse| mouse.warp_emulation_active = false);
            }
            with_mouse(|mouse| mouse.last_center_warp_time = now);
            return;
        }
    }

    with_mouse(|mouse| mouse.last_center_warp_time = Duration::ZERO);
}

/// Move the mouse cursor to the given position within the window
/// (or the focus window). Translation of `SDL_WarpMouseInWindow()`.
pub fn warp_mouse_in_window(window: Option<WindowID>, x: f32, y: f32) {
    maybe_enable_warp_emulation(window, x, y);

    let ignore = with_mouse(|mouse| mouse.warp_emulation_active);
    perform_warp_mouse_in_window(window, x, y, ignore);
}

/// Move the mouse to the given position in global screen space.
/// Translation of `SDL_WarpMouseGlobal()`.
pub fn warp_mouse_global(x: f32, y: f32) -> Result<()> {
    if let Some(video) = video() {
        if video.supports_mouse_feature(MouseFeature::WarpMouseGlobal) {
            return video.warp_mouse_global(x, y);
        }
    }
    Err(Error::unsupported())
}

/// Set relative mouse mode. Translation of `SDL_SetRelativeMouseMode()`.
pub fn set_relative_mouse_mode(enabled: bool) -> Result<()> {
    let focus_window = super::keyboard::keyboard_focus();

    let unchanged = with_mouse(|mouse| {
        if !enabled {
            // If warps were being emulated, reset the flag.
            mouse.warp_emulation_active = false;
        }
        enabled == mouse.relative_mode
    });
    if unchanged {
        return Ok(());
    }

    // Set the relative mode
    let Some(video) =
        video().filter(|v| v.supports_mouse_feature(MouseFeature::SetRelativeMouseMode))
    else {
        return Err(Error::unsupported());
    };
    video.set_relative_mouse_mode(enabled)?;

    with_mouse(|mouse| mouse.relative_mode = enabled);

    if enabled {
        // Update cursor visibility before we potentially warp the mouse
        redraw_cursor();
    }

    if enabled && focus_window.is_some() {
        set_mouse_focus(focus_window);
    }

    if let Some(focus) = focus_window {
        video.update_window_grab(focus);

        // Put the cursor back to where the application expects it
        if !enabled {
            let (mx, my) = position();
            perform_warp_mouse_in_window(Some(focus), mx, my, true);
        }

        let _ = update_mouse_capture(false);
    }

    if !enabled {
        // Update cursor visibility after we restore the mouse position
        redraw_cursor();
    }

    // Flush pending mouse motion - ideally we would pump events, but that's not always safe
    queue::flush_event(EventType::MOUSE_MOTION);

    Ok(())
}

/// Whether relative mouse mode is enabled. Translation of `SDL_GetRelativeMouseMode()`.
pub fn relative_mode_enabled() -> bool {
    with_mouse(|mouse| mouse.relative_mode)
}

/// Re-evaluate relative mode from the focus window's flags.
/// Translation of `SDL_UpdateRelativeMouseMode()`.
pub fn update_relative_mouse_mode() -> Result<()> {
    let focus = window_core(super::keyboard::keyboard_focus());
    let relative_mode = focus.is_some_and(|f| {
        f.flags.contains(WindowFlags::MOUSE_RELATIVE_MODE)
            && f.flags.contains(WindowFlags::MOUSE_FOCUS)
    });

    if relative_mode == relative_mode_enabled() {
        return Ok(());
    }

    set_relative_mouse_mode(relative_mode)
}

/// Update the mouse capture window. Translation of `SDL_UpdateMouseCapture()`.
pub fn update_mouse_capture(force_release: bool) -> Result<()> {
    let Some(video) = video().filter(|v| v.supports_mouse_feature(MouseFeature::CaptureMouse))
    else {
        return Ok(());
    };

    let message_box_count = video.message_box_count();
    let (capture_window, previous_capture) = with_mouse(|mouse| {
        let mut capture_window = None;
        if !force_release
            && message_box_count == 0
            && (mouse.capture_desired
                || (mouse.auto_capture
                    && !mouse_button_state(mouse, GLOBAL_MOUSE_ID, false).is_empty()))
            && !mouse.relative_mode
        {
            capture_window = mouse.focus;
        }
        (capture_window, mouse.capture_window)
    });

    if capture_window != previous_capture {
        /* We can get here recursively on Windows, so make sure we complete
         * all of the window state operations before we change the capture state
         * (e.g. https://github.com/libsdl-org/SDL/pull/5608)
         */
        let set_capture_flag = |id: Option<WindowID>, on: bool| {
            if let Some(id) = id {
                video.with_window(id, &mut |w| w.flags.set(WindowFlags::MOUSE_CAPTURE, on));
            }
        };
        set_capture_flag(previous_capture, false);
        set_capture_flag(capture_window, true);
        with_mouse(|mouse| mouse.capture_window = capture_window);

        if let Err(e) = video.capture_mouse(capture_window) {
            // CaptureMouse() will have set an error, just restore the state
            set_capture_flag(previous_capture, true);
            set_capture_flag(capture_window, false);
            with_mouse(|mouse| mouse.capture_window = previous_capture);
            return Err(e);
        }
    }
    Ok(())
}

/// Capture the mouse and track input outside an SDL window.
/// Translation of `SDL_CaptureMouse()`.
pub fn capture_mouse(enabled: bool) -> Result<()> {
    if !supports(MouseFeature::CaptureMouse) {
        return Err(Error::unsupported());
    }

    /* Windows mouse capture is tied to the current thread, and must be called
     * from the thread that created the window being captured. Since we update
     * the mouse capture state from the event processing, any application state
     * changes must be processed on that thread as well.
     */
    if cfg!(windows) && !crate::init::is_video_thread() {
        return Err(err!("SDL_CaptureMouse() must be called on the main thread"));
    }

    if enabled && super::keyboard::keyboard_focus().is_none() {
        return Err(err!("No window has focus"));
    }
    with_mouse(|mouse| mouse.capture_desired = enabled);

    update_mouse_capture(false)
}

/// Create a black and white cursor from bitmaps: `data` and `mask` have a
/// bit per pixel, most significant bit first, with rows of `w` rounded up
/// to a multiple of 8 pixels. Data 1 and mask 1 is black, data 0 and mask 1
/// white, data 0 and mask 0 transparent, and data 1 and mask 0 inverted
/// where the system supports it (Windows), black otherwise.
/// Translation of `SDL_CreateCursor()`.
pub fn create_cursor(
    data: &[u8],
    mask: &[u8],
    w: i32,
    h: i32,
    hot_x: i32,
    hot_y: i32,
) -> Result<Cursor> {
    const BLACK: u32 = 0xFF000000;
    const WHITE: u32 = 0xFFFFFFFF;
    const TRANSPARENT: u32 = 0x00000000;
    // Only Windows backend supports inverted pixels in mono cursors.
    const INVERTED: u32 = if cfg!(windows) {
        0x00FFFFFF
    } else {
        0xFF000000
    };

    // Make sure the width is a multiple of 8
    let w = (w + 7) & !7;

    // (upstream reads past the end of short bitmaps)
    let needed = (w.max(0) as usize / 8) * h.max(0) as usize;
    if data.len() < needed {
        return Err(Error::invalid_param("data"));
    }
    if mask.len() < needed {
        return Err(Error::invalid_param("mask"));
    }

    // Create the surface from a bitmap
    let mut surface = Surface::new(w, h, PixelFormat::ARGB8888)?;
    let pitch = surface.pitch() as usize;
    let pixels = surface
        .pixels_mut()
        .ok_or_else(|| Error::invalid_param("surface"))?;
    let (mut data, mut mask) = (data.iter(), mask.iter());
    let (mut datab, mut maskb) = (0u8, 0u8);
    for y in 0..h as usize {
        let row = &mut pixels[y * pitch..][..w as usize * 4];
        for (x, pixel) in row.chunks_exact_mut(4).enumerate() {
            if x % 8 == 0 {
                datab = *data.next().unwrap_or(&0);
                maskb = *mask.next().unwrap_or(&0);
            }
            let value = if maskb & 0x80 != 0 {
                if datab & 0x80 != 0 {
                    BLACK
                } else {
                    WHITE
                }
            } else if datab & 0x80 != 0 {
                INVERTED
            } else {
                TRANSPARENT
            };
            pixel.copy_from_slice(&value.to_ne_bytes());
            datab <<= 1;
            maskb <<= 1;
        }
    }

    create_color_cursor(&surface, hot_x, hot_y)
}

/// The hot spot of a cursor surface: its hotspot properties, or the
/// given point, checked to lie within the surface.
fn cursor_hot_spot(surface: &Surface<'_>, hot_x: i32, hot_y: i32) -> Result<(i32, i32)> {
    // Allow specifying the hot spot via properties on the surface
    let props = surface.props.as_ref();
    let hot_x = props
        .and_then(|p| p.get_number(PROP_SURFACE_HOTSPOT_X_NUMBER))
        .map_or(hot_x, |v| v as i32);
    let hot_y = props
        .and_then(|p| p.get_number(PROP_SURFACE_HOTSPOT_Y_NUMBER))
        .map_or(hot_y, |v| v as i32);

    // Sanity check the hot spot
    if hot_x < 0 || hot_y < 0 || hot_x >= surface.width() || hot_y >= surface.height() {
        return Err(err!("Cursor hot spot doesn't lie within cursor"));
    }
    Ok((hot_x, hot_y))
}

/// Create a color cursor from a surface (its hotspot properties, if set,
/// override `hot_x` and `hot_y`). Translation of `SDL_CreateColorCursor()`.
pub fn create_color_cursor(surface: &Surface<'_>, hot_x: i32, hot_y: i32) -> Result<Cursor> {
    let (hot_x, hot_y) = cursor_hot_spot(surface, hot_x, hot_y)?;

    let converted;
    let surface = if surface.format() != PixelFormat::ARGB8888 {
        converted = surface.convert(PixelFormat::ARGB8888)?;
        &converted
    } else {
        surface
    };

    let cursor = match video().and_then(|v| v.create_cursor(surface, hot_x, hot_y)) {
        Some(cursor) => cursor?,
        None => Cursor::placeholder(),
    };
    with_mouse(|mouse| mouse.cursors.insert(0, cursor.clone()));
    Ok(cursor)
}

/// Translation of `SDL_DestroyCursorAnimation()`.
fn destroy_cursor_animation(animation: &CursorAnimation) {
    for frame in &animation.frames {
        destroy_cursor(frame.clone());
    }
}

/// Translation of `SDL_CreateCursorAnimation()`.
fn create_cursor_animation(
    frames: &[CursorFrame<'_>],
    hot_x: i32,
    hot_y: i32,
) -> Result<CursorAnimation> {
    let mut animation = CursorAnimation {
        frames: Vec::with_capacity(frames.len()),
        durations: Vec::with_capacity(frames.len()),
        state: Mutex::new((0, 0)),
    };

    for frame in frames {
        match create_color_cursor(frame.surface, hot_x, hot_y) {
            Ok(cursor) => animation.frames.push(cursor),
            Err(e) => {
                destroy_cursor_animation(&animation);
                return Err(e);
            }
        }
        animation.durations.push(frame.duration);
    }

    Ok(animation)
}

/// Update the cursor animation if needed. Translation of `SDL_UpdateCursorAnimation()`.
pub(crate) fn update_cursor_animation() {
    let advanced = with_mouse(|mouse| {
        let Some(cursor) = &mouse.cur_cursor else {
            return false;
        };
        let Some(animation) = &cursor.0.animation else {
            return false;
        };

        if mouse.focus.is_none() {
            return false;
        }

        let mut state = animation.state();
        let duration = animation.durations[state.0];
        if duration == 0 {
            // We've reached the stop frame of the animation
            return false;
        }

        let now = timer::ticks().as_millis() as u64;
        if now < state.1 + duration as u64 {
            return false;
        }

        state.0 = (state.0 + 1) % animation.frames.len();
        state.1 = now;
        true
    });
    if advanced {
        redraw_cursor();
    }
}

/// Create an animated cursor from frames of the same size (the first
/// frame's hotspot properties, if set, override `hot_x` and `hot_y`).
/// Backends that can't animate cursors get a cursor the mouse code
/// animates as events are pumped. Translation of `SDL_CreateAnimatedCursor()`.
pub fn create_animated_cursor(
    frames: &[CursorFrame<'_>],
    hot_x: i32,
    hot_y: i32,
) -> Result<Cursor> {
    if frames.is_empty() {
        return Err(Error::invalid_param("frame_count"));
    }

    if frames.len() == 1 {
        return create_color_cursor(frames[0].surface, hot_x, hot_y);
    }

    let (hot_x, hot_y) = cursor_hot_spot(frames[0].surface, hot_x, hot_y)?;

    let w = frames[0].surface.width();
    let h = frames[0].surface.height();

    let mut temp_surfaces = Vec::with_capacity(frames.len());
    for frame in frames {
        // All cursor images should be the same size.
        if frame.surface.width() != w || frame.surface.height() != h {
            return Err(err!(
                "All frames in an animated sequence must have the same dimensions"
            ));
        }
        temp_surfaces.push(if frame.surface.format() == PixelFormat::ARGB8888 {
            None
        } else {
            Some(frame.surface.convert(PixelFormat::ARGB8888)?)
        });
    }
    let temp_frames: Vec<CursorFrame<'_>> = temp_surfaces
        .iter()
        .zip(frames)
        .map(|(converted, frame)| CursorFrame {
            surface: converted.as_ref().unwrap_or(frame.surface),
            duration: frame.duration,
        })
        .collect();

    let cursor = match video().and_then(|v| v.create_animated_cursor(&temp_frames, hot_x, hot_y)) {
        Some(cursor) => cursor?,
        None => {
            let animation = create_cursor_animation(&temp_frames, hot_x, hot_y)?;
            Cursor(Arc::new(CursorInner {
                internal: None,
                animation: Some(animation),
            }))
        }
    };

    with_mouse(|mouse| mouse.cursors.insert(0, cursor.clone()));
    Ok(cursor)
}

/// Create a system cursor. Translation of `SDL_CreateSystemCursor()`.
pub fn create_system_cursor(id: SystemCursor) -> Result<Cursor> {
    let Some(video) = video() else {
        return Err(err!("CreateSystemCursor is not currently supported"));
    };
    let cursor = video.create_system_cursor(id)?;
    with_mouse(|mouse| mouse.cursors.insert(0, cursor.clone()));
    Ok(cursor)
}

/// Register a cursor created by a backend so it can be made current.
pub fn register_cursor(cursor: Cursor) {
    with_mouse(|mouse| mouse.cursors.insert(0, cursor));
}

/// Translation of `SDL_RedrawCursor()`.
pub fn redraw_cursor() {
    let cursor = with_mouse(|mouse| {
        let mut cursor = if mouse.focus.is_some() {
            mouse.cur_cursor.clone()
        } else {
            mouse.def_cursor.clone()
        };

        if mouse.focus.is_some()
            && (!mouse.cursor_visible || (mouse.relative_mode && mouse.relative_mode_hide_cursor))
        {
            cursor = None;
        }

        cursor.map(|cursor| match &cursor.0.animation {
            Some(animation) => animation.current(),
            None => cursor,
        })
    });

    if let Some(video) = video() {
        video.show_cursor(cursor.as_ref());
    }
}

/* SDL_SetCursor(NULL) can be used to force the cursor redraw,
  if this is desired for any reason.  This is used when setting
  the video mode and when the SDL window gains the mouse focus.
*/
/// Set the active cursor (`None` forces a redraw). Translation of `SDL_SetCursor()`.
pub fn set_cursor(cursor: Option<&Cursor>) -> Result<()> {
    let r = with_mouse(|mouse| {
        // already on this cursor, no further action required
        if cursor == mouse.cur_cursor.as_ref() {
            return Ok(false);
        }

        // Set the new cursor
        if let Some(cursor) = cursor {
            // Make sure the cursor is still valid for this mouse
            if Some(cursor) != mouse.def_cursor.as_ref() && !mouse.cursors.contains(cursor) {
                return Err(err!("Cursor not associated with the current mouse"));
            }
            if let Some(animation) = &cursor.0.animation {
                *animation.state() = (0, timer::ticks().as_millis() as u64);
            }
            mouse.cur_cursor = Some(cursor.clone());
        }
        Ok(true)
    })?;
    if r {
        redraw_cursor();
    }
    Ok(())
}

/// The active cursor. Translation of `SDL_GetCursor()`.
pub fn cursor() -> Option<Cursor> {
    with_mouse(|mouse| mouse.cur_cursor.clone())
}

/// The default cursor. Translation of `SDL_GetDefaultCursor()`.
pub fn default_cursor() -> Option<Cursor> {
    with_mouse(|mouse| mouse.def_cursor.clone())
}

/// Free a cursor. Translation of `SDL_DestroyCursor()`.
pub fn destroy_cursor(cursor: Cursor) {
    let reset = with_mouse(|mouse| {
        if Some(&cursor) == mouse.def_cursor.as_ref() {
            return None;
        }
        Some(Some(&cursor) == mouse.cur_cursor.as_ref())
    });
    let Some(reset) = reset else {
        return;
    };
    if reset {
        let def = default_cursor();
        let _ = set_cursor(def.as_ref());
    }
    let found = with_mouse(|mouse| {
        let before = mouse.cursors.len();
        mouse.cursors.retain(|c| *c != cursor);
        mouse.cursors.len() != before
    });
    if found {
        if let Some(animation) = &cursor.0.animation {
            destroy_cursor_animation(animation);
        }
    }
    // (FreeCursor / SDL_free: the Arc drops the backend data)
}

/// Show the cursor. Translation of `SDL_ShowCursor()`.
pub fn show_cursor() {
    if with_mouse(|mouse| mouse.warp_emulation_active) {
        let _ = set_relative_mouse_mode(false);
        with_mouse(|mouse| mouse.warp_emulation_active = false);
    }

    let changed = with_mouse(|mouse| {
        if !mouse.cursor_visible {
            mouse.cursor_visible = true;
            true
        } else {
            false
        }
    });
    if changed {
        redraw_cursor();
    }
}

/// Hide the cursor. Translation of `SDL_HideCursor()`.
pub fn hide_cursor() {
    let changed = with_mouse(|mouse| {
        if mouse.cursor_visible {
            mouse.cursor_visible = false;
            true
        } else {
            false
        }
    });
    if changed {
        redraw_cursor();
    }
}

/// Whether the cursor is currently being shown. Translation of `SDL_CursorVisible()`.
pub fn cursor_visible() -> bool {
    with_mouse(|mouse| mouse.cursor_visible)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::window::tests::with_video;

    fn drain() -> Vec<Event> {
        queue::get_events(EventType::FIRST, EventType::LAST, 1000).unwrap()
    }

    #[test]
    fn flags_and_cursors() {
        assert_eq!(MouseButtonFlags::LMASK.0, 1);
        assert_eq!(MouseButtonFlags::X2MASK.0, 16);
        assert!(
            MouseButtonFlags(5).is_pressed(BUTTON_LEFT)
                && MouseButtonFlags(5).is_pressed(BUTTON_RIGHT)
        );
        assert_eq!(SystemCursor::from_index(33), Some(SystemCursor::ZoomOut));
        assert_eq!(SystemCursor::from_index(34), None);
        assert_eq!(SystemCursor::from_index(-1), None);
        assert_eq!(TOUCH_MOUSE_ID, 0xFFFF_FFFF);
        assert_eq!(PEN_MOUSE_ID, 0xFFFF_FFFE);
    }

    #[test]
    fn devices() {
        with_video(&[], |_| {
            pre_init_mouse().unwrap();
            assert!(!has_mouse());
            add_mouse(3, None);
            add_mouse(3, Some("dup"));
            add_mouse(4, Some("Trackball"));
            assert_eq!(mice(), vec![3, 4]);
            assert_eq!(mouse_name(3).unwrap(), "Mouse");
            assert_eq!(mouse_name(4).unwrap(), "Trackball");
            assert_eq!(mouse_name(GLOBAL_MOUSE_ID).unwrap(), "Mouse");
            assert_eq!(mouse_name(PEN_MOUSE_ID).unwrap(), "Pen");
            assert_eq!(mouse_name(TOUCH_MOUSE_ID).unwrap(), "Touch");
            assert_eq!(mouse_name(9).unwrap_err().message(), "Mouse 9 not found");
            remove_mouse(3);
            assert_eq!(mice(), vec![4]);
            assert_eq!(drain().len(), 3);
            assert!(is_mouse(1, 2));
            quit_mouse();
            assert!(!has_mouse());
        });
    }

    #[test]
    fn motion_focus_and_buttons() {
        with_video(&[1], |video| {
            pre_init_mouse().unwrap();
            video.with_window(1, &mut |w| w.flags &= !WindowFlags::HIDDEN);
            drain();

            // Motion inside the window gains focus (MOUSE_ENTER) and posts motion.
            send_mouse_motion(Duration::ZERO, Some(1), DEFAULT_MOUSE_ID, false, 10.0, 20.0);
            assert_eq!(mouse_focus(), Some(1));
            let events = drain();
            let types: Vec<_> = events.iter().map(|e| e.event_type()).collect();
            assert_eq!(
                types,
                vec![EventType::WINDOW_MOUSE_ENTER, EventType::MOUSE_MOTION]
            );
            match &events[1] {
                Event::MouseMotion(m) => {
                    assert_eq!((m.x, m.y, m.xrel, m.yrel), (10.0, 20.0, 0.0, 0.0));
                    assert_eq!(
                        m.which, GLOBAL_MOUSE_ID,
                        "not in relative mode: global mouse events"
                    );
                    assert_eq!(m.window_id, 1);
                }
                _ => unreachable!(),
            }

            // Relative deltas accumulate; duplicate positions are dropped.
            send_mouse_motion(Duration::ZERO, Some(1), DEFAULT_MOUSE_ID, true, 5.0, -5.0);
            send_mouse_motion(Duration::ZERO, Some(1), DEFAULT_MOUSE_ID, false, 15.0, 15.0);
            let events = drain();
            assert_eq!(events.len(), 1);
            assert_eq!(mouse_state(), (15.0, 15.0, MouseButtonFlags::NONE));
            let (ax, ay, _) = relative_mouse_state();
            assert_eq!((ax, ay), (5.0, -5.0));
            assert_eq!(relative_mouse_state().0, 0.0);

            // Positions are constrained to the window.
            send_mouse_motion(
                Duration::ZERO,
                Some(1),
                DEFAULT_MOUSE_ID,
                true,
                10000.0,
                -10000.0,
            );
            assert_eq!(mouse_state().0, 639.0);
            assert_eq!(mouse_state().1, 0.0);
            drain();

            // Buttons: click counting and state
            send_mouse_button(Duration::ZERO, Some(1), DEFAULT_MOUSE_ID, BUTTON_LEFT, true);
            send_mouse_button(Duration::ZERO, Some(1), DEFAULT_MOUSE_ID, BUTTON_LEFT, true); // no change: dropped
            send_mouse_button(
                Duration::ZERO,
                Some(1),
                DEFAULT_MOUSE_ID,
                BUTTON_LEFT,
                false,
            );
            send_mouse_button(Duration::ZERO, Some(1), DEFAULT_MOUSE_ID, BUTTON_LEFT, true);
            assert!(mouse_state().2.is_pressed(BUTTON_LEFT));
            let buttons: Vec<MouseButtonEvent> = drain()
                .into_iter()
                .filter_map(|e| {
                    if let Event::MouseButton(b) = e {
                        Some(b)
                    } else {
                        None
                    }
                })
                .collect();
            assert_eq!(buttons.len(), 3);
            assert_eq!((buttons[0].clicks, buttons[0].down), (1, true));
            assert_eq!((buttons[1].clicks, buttons[1].down), (1, false));
            assert_eq!(
                (buttons[2].clicks, buttons[2].down),
                (2, true),
                "second press within the double-click window"
            );
            send_mouse_button_clicks(
                Duration::ZERO,
                Some(1),
                DEFAULT_MOUSE_ID,
                BUTTON_RIGHT,
                true,
                7,
            );
            match drain().pop() {
                Some(Event::MouseButton(b)) => assert_eq!((b.button, b.clicks), (BUTTON_RIGHT, 7)),
                other => panic!("{other:?}"),
            }

            // Wheel accumulates fractional ticks
            send_mouse_wheel(
                Duration::ZERO,
                Some(1),
                DEFAULT_MOUSE_ID,
                0.0,
                0.0,
                MouseWheelDirection::Normal,
            );
            send_mouse_wheel(
                Duration::ZERO,
                Some(1),
                DEFAULT_MOUSE_ID,
                0.0,
                0.6,
                MouseWheelDirection::Normal,
            );
            send_mouse_wheel(
                Duration::ZERO,
                Some(1),
                DEFAULT_MOUSE_ID,
                0.0,
                0.6,
                MouseWheelDirection::Flipped,
            );
            let wheels: Vec<MouseWheelEvent> = drain()
                .into_iter()
                .filter_map(|e| {
                    if let Event::MouseWheel(w) = e {
                        Some(w)
                    } else {
                        None
                    }
                })
                .collect();
            assert_eq!(wheels.len(), 2);
            assert_eq!((wheels[0].integer_y, wheels[1].integer_y), (0, 1));
            assert_eq!(wheels[1].direction, MouseWheelDirection::Flipped);
            assert!((wheels[1].y - 0.6).abs() < 1e-6);

            // Leaving the window drops focus
            send_mouse_motion(Duration::ZERO, Some(1), DEFAULT_MOUSE_ID, false, -5.0, 5.0);
            assert_eq!(mouse_focus(), None);
            assert!(drain()
                .iter()
                .any(|e| e.event_type() == EventType::WINDOW_MOUSE_LEAVE));

            // Without a backend relative mode and capture are unsupported
            assert_eq!(
                set_relative_mouse_mode(true).unwrap_err().kind(),
                crate::ErrorKind::Unsupported
            );
            assert!(!relative_mode_enabled());
            assert_eq!(
                capture_mouse(true).unwrap_err().kind(),
                crate::ErrorKind::Unsupported
            );
            assert_eq!(
                warp_mouse_global(1.0, 1.0).unwrap_err().kind(),
                crate::ErrorKind::Unsupported
            );

            // Warping without a backend synthesizes motion
            warp_mouse_in_window(Some(1), 100.0, 100.0);
            assert_eq!(mouse_state().0, 100.0);

            // Cursor visibility
            assert!(cursor_visible());
            hide_cursor();
            assert!(!cursor_visible());
            show_cursor();
            assert!(cursor_visible());
            let c = Cursor::placeholder();
            assert!(
                set_cursor(Some(&c)).is_err(),
                "unregistered cursors are rejected"
            );
            register_cursor(c.clone());
            set_cursor(Some(&c)).unwrap();
            assert_eq!(cursor(), Some(c.clone()));
            set_default_cursor(Some(Cursor::placeholder()));
            destroy_cursor(c);
            assert_eq!(cursor(), default_cursor());
            quit_mouse();
        });
    }

    #[test]
    fn hints_touch_emulation() {
        with_video(&[1], |_| {
            pre_init_mouse().unwrap();
            assert!(!super::super::touch::touch_devices()
                .contains(&super::super::touch::MOUSE_TOUCH_ID));
            hints::set(hints::MOUSE_TOUCH_EVENTS, "1").unwrap();
            assert!(
                super::super::touch::touch_devices().contains(&super::super::touch::MOUSE_TOUCH_ID)
            );
            drain();
            send_mouse_motion(
                Duration::ZERO,
                Some(1),
                DEFAULT_MOUSE_ID,
                false,
                320.0,
                240.0,
            );
            send_mouse_button(Duration::ZERO, Some(1), DEFAULT_MOUSE_ID, BUTTON_LEFT, true);
            let types: Vec<_> = drain().iter().map(|e| e.event_type()).collect();
            assert!(types.contains(&EventType::FINGER_DOWN), "{types:?}");
            assert!(types.contains(&EventType::MOUSE_BUTTON_DOWN));
            send_mouse_button(
                Duration::ZERO,
                Some(1),
                DEFAULT_MOUSE_ID,
                BUTTON_LEFT,
                false,
            );
            hints::reset(hints::MOUSE_TOUCH_EVENTS);
            assert!(!super::super::touch::touch_devices()
                .contains(&super::super::touch::MOUSE_TOUCH_ID));

            hints::set("SDL_MOUSE_INTEGER_MODE", "1").unwrap();
            drain();
            send_mouse_motion(Duration::ZERO, Some(1), DEFAULT_MOUSE_ID, false, 10.7, 10.2);
            assert_eq!(mouse_state().0, 10.0);
            hints::reset("SDL_MOUSE_INTEGER_MODE");
            quit_mouse();
        });
    }
}
