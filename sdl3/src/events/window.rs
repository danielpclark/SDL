// Rust translation of src/events/SDL_windowevents.c, SDL_displayevents.c,
// SDL_clipboardevents.c, SDL_dropevents.c and SDL_notificationevents.c
// from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Window, display, clipboard, drag-and-drop and notification event sources.
//!
//! The event core refers to windows by [`WindowID`] and reaches window state
//! through the [`VideoHooks`] trait, which the video subsystem implements
//! (later phase). [`WindowCore`] holds exactly the `SDL_Window` fields these
//! event sources read and write, so the video module can embed it.

use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use super::queue::{self, EventWatch, WatchList};
use super::{
    ClipboardEvent, DisplayEvent, DisplayID, DropEvent, Event, EventType, NotificationEvent,
    NotificationID, WindowEvent, WindowID,
};
use crate::error::Result;
use crate::hints;
use crate::video::Rect;

// ---------------------------------------------------------------------------
// Window flags
// ---------------------------------------------------------------------------

/// The flags on a window. Translation of `SDL_WindowFlags`.
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct WindowFlags(pub u64);

macro_rules! window_flags {
    ($($(#[$m:meta])* $name:ident = $val:expr;)*) => {
        impl WindowFlags {
            $( $(#[$m])* pub const $name: WindowFlags = WindowFlags($val); )*
        }
        impl std::fmt::Debug for WindowFlags {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                let mut rest = self.0;
                let mut first = true;
                write!(f, "WindowFlags(")?;
                $(
                    #[allow(clippy::bad_bit_mask)] // NONE = 0 is skipped at runtime
                    {
                        const V: u64 = $val;
                        if V != 0 && self.0 & V == V {
                            if !first { write!(f, " | ")?; }
                            write!(f, stringify!($name))?;
                            first = false;
                            rest &= !V;
                        }
                    }
                )*
                if rest != 0 || first {
                    if !first { write!(f, " | ")?; }
                    write!(f, "{rest:#x}")?;
                }
                write!(f, ")")
            }
        }
    };
}

window_flags! {
    /// no flags
    NONE = 0;
    /// window is in fullscreen mode
    FULLSCREEN = 0x0000000000000001;
    /// window usable with OpenGL context
    OPENGL = 0x0000000000000002;
    /// window is occluded
    OCCLUDED = 0x0000000000000004;
    /// window is neither mapped onto the desktop nor shown in the taskbar/dock/window list
    HIDDEN = 0x0000000000000008;
    /// no window decoration
    BORDERLESS = 0x0000000000000010;
    /// window can be resized
    RESIZABLE = 0x0000000000000020;
    /// window is minimized
    MINIMIZED = 0x0000000000000040;
    /// window is maximized
    MAXIMIZED = 0x0000000000000080;
    /// window has grabbed mouse input
    MOUSE_GRABBED = 0x0000000000000100;
    /// window has input focus
    INPUT_FOCUS = 0x0000000000000200;
    /// window has mouse focus
    MOUSE_FOCUS = 0x0000000000000400;
    /// window not created by SDL
    EXTERNAL = 0x0000000000000800;
    /// window is modal
    MODAL = 0x0000000000001000;
    /// window uses high pixel density back buffer if possible
    HIGH_PIXEL_DENSITY = 0x0000000000002000;
    /// window has mouse captured (unrelated to MOUSE_GRABBED)
    MOUSE_CAPTURE = 0x0000000000004000;
    /// window has relative mode enabled
    MOUSE_RELATIVE_MODE = 0x0000000000008000;
    /// window should always be above others
    ALWAYS_ON_TOP = 0x0000000000010000;
    /// window should be treated as a utility window, not showing in the task bar and window list
    UTILITY = 0x0000000000020000;
    /// window should be treated as a tooltip and does not get mouse or keyboard focus, requires a parent window
    TOOLTIP = 0x0000000000040000;
    /// window should be treated as a popup menu, requires a parent window
    POPUP_MENU = 0x0000000000080000;
    /// window has grabbed keyboard input
    KEYBOARD_GRABBED = 0x0000000000100000;
    /// window will fill the entire document area
    FILL_DOCUMENT = 0x0000000000200000;
    /// window usable for Vulkan surface
    VULKAN = 0x0000000010000000;
    /// window usable for Metal view
    METAL = 0x0000000020000000;
    /// window with transparent buffer
    TRANSPARENT = 0x0000000040000000;
    /// window should not be focusable
    NOT_FOCUSABLE = 0x0000000080000000;
}

impl WindowFlags {
    /// True if every flag in `other` is set.
    pub const fn contains(self, other: WindowFlags) -> bool {
        self.0 & other.0 == other.0
    }
    /// True if any flag in `other` is set.
    pub const fn intersects(self, other: WindowFlags) -> bool {
        self.0 & other.0 != 0
    }
    /// True if no flag is set.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
    /// Set or clear `other`.
    pub fn set(&mut self, other: WindowFlags, on: bool) {
        if on {
            self.0 |= other.0;
        } else {
            self.0 &= !other.0;
        }
    }
}

impl std::ops::BitOr for WindowFlags {
    type Output = WindowFlags;
    fn bitor(self, rhs: WindowFlags) -> WindowFlags {
        WindowFlags(self.0 | rhs.0)
    }
}
impl std::ops::BitOrAssign for WindowFlags {
    fn bitor_assign(&mut self, rhs: WindowFlags) {
        self.0 |= rhs.0;
    }
}
impl std::ops::BitAnd for WindowFlags {
    type Output = WindowFlags;
    fn bitand(self, rhs: WindowFlags) -> WindowFlags {
        WindowFlags(self.0 & rhs.0)
    }
}
impl std::ops::BitAndAssign for WindowFlags {
    fn bitand_assign(&mut self, rhs: WindowFlags) {
        self.0 &= rhs.0;
    }
}
impl std::ops::Not for WindowFlags {
    type Output = WindowFlags;
    fn not(self) -> WindowFlags {
        WindowFlags(!self.0)
    }
}

// ---------------------------------------------------------------------------
// Window state visible to the event core
// ---------------------------------------------------------------------------

/// The `SDL_Window` fields the event sources read and update.
///
/// The video subsystem embeds one of these in each window and exposes it via
/// [`VideoHooks::with_window`].
#[derive(Clone, Debug, PartialEq)]
pub struct WindowCore {
    pub id: WindowID,
    pub flags: WindowFlags,
    pub pending_flags: WindowFlags,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    /// Position/size when not fullscreen. Translation of `window->windowed`.
    pub windowed: Rect,
    /// Position/size when not maximized or tiled. Translation of `window->floating`.
    pub floating: Rect,
    pub last_pixel_w: i32,
    pub last_pixel_h: i32,
    pub undefined_x: bool,
    pub undefined_y: bool,
    pub last_position_pending: bool,
    pub last_size_pending: bool,
    pub tiled: bool,
    pub display_id: DisplayID,
    pub pending_display_id: DisplayID,
    pub update_fullscreen_on_display_changed: bool,
    pub is_destroying: bool,
    pub is_dropping: bool,
    /// True if the window has a parent (`window->parent != NULL`).
    pub has_parent: bool,
    /// Translation of `window->mouse_rect` (empty when unset).
    pub mouse_rect: Rect,
    /// Translation of `window->text_input_active`.
    pub text_input_active: bool,
    /// Translation of `window->min_w/min_h/max_w/max_h`.
    pub min_w: i32,
    pub min_h: i32,
    pub max_w: i32,
    pub max_h: i32,
}

impl WindowCore {
    /// A fresh, hidden window record with the given id.
    pub fn new(id: WindowID) -> Self {
        WindowCore {
            id,
            flags: WindowFlags::HIDDEN,
            pending_flags: WindowFlags::NONE,
            x: 0,
            y: 0,
            w: 0,
            h: 0,
            windowed: Rect::default(),
            floating: Rect::default(),
            last_pixel_w: 0,
            last_pixel_h: 0,
            undefined_x: false,
            undefined_y: false,
            last_position_pending: false,
            last_size_pending: false,
            tiled: false,
            display_id: 0,
            pending_display_id: 0,
            update_fullscreen_on_display_changed: false,
            is_destroying: false,
            is_dropping: false,
            has_parent: false,
            mouse_rect: Rect::default(),
            text_input_active: false,
            min_w: 0,
            min_h: 0,
            max_w: 0,
            max_h: 0,
        }
    }
}

/// Display state the display event source updates (`SDL_VideoDisplay` subset).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DisplayCore {
    pub id: DisplayID,
    /// Translation of `display->current_orientation` (`SDL_DisplayOrientation` raw value).
    pub current_orientation: i32,
}

/// Translation of `SDL_ORIENTATION_UNKNOWN`.
pub const ORIENTATION_UNKNOWN: i32 = 0;

/// The services the event core needs from the video subsystem.
///
/// Each method corresponds to an `SDL_VideoDevice` function pointer or a
/// window query that upstream reaches directly through `SDL_Window *`. The
/// default implementations describe a build without video: there are no
/// windows, no waiting, and relative/warp/capture are unsupported.
pub trait VideoHooks: Send + Sync {
    /// Run `f` on the window's core state. Returns `false` if no such window.
    /// (Replaces direct `SDL_Window *` access.)
    fn with_window(&self, id: WindowID, f: &mut dyn FnMut(&mut WindowCore)) -> bool;
    /// The core state of a window, if it exists (a convenience over `with_window`).
    fn window(&self, id: WindowID) -> Option<WindowCore> {
        let mut out = None;
        self.with_window(id, &mut |w| out = Some(w.clone()));
        out
    }
    /// Translation of `_this->is_quitting`.
    fn is_quitting(&self) -> bool {
        false
    }
    /// The `SDL_OnWindow*()` callbacks run after a window event is posted.
    fn on_window_event(&self, _id: WindowID, _event_type: EventType) {}
    /// Translation of `SDL_CheckWindowPixelSizeChanged()`.
    fn check_window_pixel_size_changed(&self, _id: WindowID) {}
    /// The number of top-level windows that are not hidden
    /// (the loop over `_this->windows` in `SDL_SendWindowEvent`).
    fn visible_toplevel_window_count(&self) -> usize {
        0
    }
    /// Whether any windows exist. Translation of `SDL_HasWindows()`.
    fn has_windows(&self) -> bool {
        false
    }
    /// Translation of `SDL_HasActiveTrays()`.
    fn has_active_trays(&self) -> bool {
        false
    }
    /// Translation of `_this->PumpEvents`.
    fn pump_events(&self) {}
    /// True if both `WaitEventTimeout` and `SendWakeupEvent` are implemented.
    fn can_wait(&self) -> bool {
        false
    }
    /// Translation of `_this->WaitEventTimeout`: 1 = got an event, 0 = timed out, -1 = error.
    fn wait_event_timeout(&self, _timeout: Option<Duration>) -> i32 {
        -1
    }
    /// Translation of `_this->wakeup_window` (set atomically around a wait).
    fn set_wakeup_window(&self, _window: Option<WindowID>) {}
    /// Translation of `SDL_find_active_window()`.
    fn find_active_window(&self) -> Option<WindowID> {
        None
    }
    /// Translation of `SDL_SendWakeupEvent()`'s backend call.
    fn send_wakeup_event(&self) {}
    /// Translation of `SDL_ToggleDragAndDropSupport()`.
    fn toggle_drag_and_drop_support(&self) {}
    /// Translation of `SDL_OnDisplayAdded()` / `SDL_OnDisplayMoved()`.
    fn on_display_event(&self, _id: DisplayID, _event_type: EventType) {}
    /// Run `f` on a display's core state. Returns `false` if no such display.
    fn with_display(&self, _id: DisplayID, _f: &mut dyn FnMut(&mut DisplayCore)) -> bool {
        false
    }
    /// Translation of `SDL_CancelClipboardData()` + `SDL_SaveClipboardMimeTypes()`.
    fn clipboard_lost(&self, _mime_types: &[String]) {}

    // --- mouse services (SDL_mouse.c) ---

    /// Whether the backend implements a mouse feature (upstream: is the
    /// corresponding `SDL_Mouse` function pointer non-NULL?).
    fn supports_mouse_feature(&self, _feature: super::mouse::MouseFeature) -> bool {
        false
    }
    /// Translation of `SDL_MinimizeWindow()`.
    fn minimize_window(&self, _id: WindowID) {}
    /// Translation of `SDL_UpdateWindowGrab()`.
    fn update_window_grab(&self, _id: WindowID) {}
    /// Translation of `mouse->WarpMouse` (window relative).
    fn warp_mouse(&self, _id: WindowID, _x: f32, _y: f32) -> Result<()> {
        Err(crate::Error::unsupported())
    }
    /// Translation of `mouse->WarpMouseGlobal`.
    fn warp_mouse_global(&self, _x: f32, _y: f32) -> Result<()> {
        Err(crate::Error::unsupported())
    }
    /// Translation of `mouse->SetRelativeMouseMode`.
    fn set_relative_mouse_mode(&self, _enabled: bool) -> Result<()> {
        Err(crate::Error::unsupported())
    }
    /// Translation of `mouse->CaptureMouse`.
    fn capture_mouse(&self, _window: Option<WindowID>) -> Result<()> {
        Err(crate::Error::unsupported())
    }
    /// Translation of `mouse->GetGlobalMouseState`; returns (x, y, buttons).
    fn global_mouse_state(&self) -> Option<(f32, f32, super::mouse::MouseButtonFlags)> {
        None
    }
    /// Translation of `mouse->ApplySystemScale`; `None` means no backend scaling.
    fn apply_system_scale(
        &self,
        _timestamp: Duration,
        _id: Option<WindowID>,
        _mouse_id: super::mouse::MouseID,
        _x: f32,
        _y: f32,
    ) -> Option<(f32, f32)> {
        None
    }
    /// Translation of `mouse->ShowCursor`: show the cursor, or hide if `None`.
    fn show_cursor(&self, _cursor: Option<&super::mouse::Cursor>) {}
    /// Translation of `mouse->MoveCursor`: called when a mouse motion event occurs.
    fn move_cursor(&self, _cursor: &super::mouse::Cursor) {}
    /// Translation of `mouse->CreateSystemCursor`.
    fn create_system_cursor(
        &self,
        _id: super::mouse::SystemCursor,
    ) -> Result<super::mouse::Cursor> {
        Err(crate::err!("CreateSystemCursor is not currently supported"))
    }
    /// Translation of `SDL_GetMessageBoxCount()`.
    fn message_box_count(&self) -> i32 {
        0
    }
    /// Translation of `_this->ResetTouch`: returns `true` if the backend reset its touch devices.
    fn reset_touch(&self) -> bool {
        false
    }

    // --- keyboard services (SDL_keyboard.c) ---

    /// Translation of `video->StartTextInput` (called when a text-input window gains focus).
    fn start_text_input(&self, _id: WindowID) {}
    /// Translation of `video->StopTextInput` (called when a text-input window loses focus).
    fn stop_text_input(&self, _id: WindowID) {}
}

/// Translation of `SDL_GetVideoDevice()`: `None` until the video subsystem
/// registers itself.
static VIDEO: RwLock<Option<Arc<dyn VideoHooks>>> = RwLock::new(None);

/// The registered video hooks, if any. Translation of `SDL_GetVideoDevice()`.
pub fn video() -> Option<Arc<dyn VideoHooks>> {
    VIDEO.read().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Register (or, with `None`, unregister) the video subsystem's hooks.
///
/// The video subsystem calls this from its init and quit; an embedding
/// application that provides its own window system may also use it.
pub fn set_video(hooks: Option<Arc<dyn VideoHooks>>) {
    *VIDEO.write().unwrap_or_else(|e| e.into_inner()) = hooks;
}

/// Translation of `SDL_GetWindowFromID()` as far as the event core can see it.
pub(crate) fn window(id: WindowID) -> Option<WindowCore> {
    if id == 0 {
        return None;
    }
    video().and_then(|v| v.window(id))
}

// ---------------------------------------------------------------------------
// Window event watch lists
// ---------------------------------------------------------------------------

/// Translation of `SDL_WindowEventWatchPriority`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WindowEventWatchPriority {
    Early = 0,
    Normal = 1,
}

/// Translation of `SDL_window_event_watchers[NUM_WINDOW_EVENT_WATCH_PRIORITIES]`.
static WINDOW_EVENT_WATCHERS: [WatchList; 2] = [WatchList::new(), WatchList::new()];

/// Translation of `SDL_InitWindowEventWatch()`.
pub(crate) fn init_window_event_watch() {}

/// Translation of `SDL_QuitWindowEventWatch()`.
pub(crate) fn quit_window_event_watch() {
    for list in &WINDOW_EVENT_WATCHERS {
        list.quit();
    }
}

/// Watch window events before they are posted (used by the video subsystem).
/// Translation of `SDL_AddWindowEventWatch()`.
pub fn add_window_event_watch(
    priority: WindowEventWatchPriority,
    callback: impl Fn(&Event) + Send + Sync + 'static,
) -> EventWatch {
    let list = &WINDOW_EVENT_WATCHERS[priority as usize];
    let id = list.add(Arc::new(callback));
    EventWatch::new(list, id)
}

// ---------------------------------------------------------------------------
// Window events
// ---------------------------------------------------------------------------

/// Update a window's state for an event and post it. Returns `true` if the
/// event was posted. Translation of `SDL_SendWindowEvent()`.
pub fn send_window_event(
    window_id: WindowID,
    windowevent: EventType,
    data1: i32,
    data2: i32,
) -> bool {
    let mut post_event = true;
    let mut posted = false;

    let Some(video) = video() else {
        return false;
    };

    // Update the window state; `keep` is false when upstream `return false`s.
    let mut keep = true;
    let mut window_is_topmost = true;
    let mut check_pixel_size = false;
    let found = video.with_window(window_id, &mut |window| {
        keep = true;
        match windowevent {
            EventType::WINDOW_SHOWN => {
                if !window.flags.contains(WindowFlags::HIDDEN) {
                    keep = false;
                    return;
                }
                window.flags &= !(WindowFlags::HIDDEN | WindowFlags::MINIMIZED);
            }
            EventType::WINDOW_HIDDEN => {
                if window.flags.contains(WindowFlags::HIDDEN) {
                    keep = false;
                    return;
                }
                window.flags |= WindowFlags::HIDDEN;
            }
            EventType::WINDOW_EXPOSED => {
                window.flags &= !WindowFlags::OCCLUDED;
            }
            EventType::WINDOW_MOVED => {
                window.undefined_x = false;
                window.undefined_y = false;
                /* Clear the pending display if this move was not the result of an explicit request,
                 * and the window is not scheduled to become fullscreen when shown.
                 */
                if !window.last_position_pending
                    && !window.pending_flags.contains(WindowFlags::FULLSCREEN)
                {
                    window.pending_display_id = 0;
                }
                window.last_position_pending = false;
                if !window.flags.contains(WindowFlags::FULLSCREEN) {
                    window.windowed.x = data1;
                    window.windowed.y = data2;

                    if !window.flags.contains(WindowFlags::MAXIMIZED) && !window.tiled {
                        window.floating.x = data1;
                        window.floating.y = data2;
                    }
                }
                if data1 == window.x && data2 == window.y {
                    keep = false;
                    return;
                }
                window.x = data1;
                window.y = data2;
            }
            EventType::WINDOW_RESIZED => {
                window.last_size_pending = false;
                if !window.flags.contains(WindowFlags::FULLSCREEN) {
                    window.windowed.w = data1;
                    window.windowed.h = data2;

                    if !window.flags.contains(WindowFlags::MAXIMIZED) && !window.tiled {
                        window.floating.w = data1;
                        window.floating.h = data2;
                    }
                }
                if data1 == window.w && data2 == window.h {
                    check_pixel_size = true;
                    keep = false;
                    return;
                }
                window.w = data1;
                window.h = data2;
            }
            EventType::WINDOW_PIXEL_SIZE_CHANGED => {
                if data1 == window.last_pixel_w && data2 == window.last_pixel_h {
                    keep = false;
                    return;
                }
                window.last_pixel_w = data1;
                window.last_pixel_h = data2;
            }
            EventType::WINDOW_MINIMIZED => {
                if window.flags.contains(WindowFlags::MINIMIZED) {
                    keep = false;
                    return;
                }
                window.flags &= !WindowFlags::MAXIMIZED;
                window.flags |= WindowFlags::MINIMIZED;
            }
            EventType::WINDOW_MAXIMIZED => {
                if window.flags.contains(WindowFlags::MAXIMIZED) {
                    keep = false;
                    return;
                }
                window.flags &= !WindowFlags::MINIMIZED;
                window.flags |= WindowFlags::MAXIMIZED;
            }
            EventType::WINDOW_RESTORED => {
                if !window
                    .flags
                    .intersects(WindowFlags::MINIMIZED | WindowFlags::MAXIMIZED)
                {
                    keep = false;
                    return;
                }
                window.flags &= !(WindowFlags::MINIMIZED | WindowFlags::MAXIMIZED);
            }
            EventType::WINDOW_MOUSE_ENTER => {
                if window.flags.contains(WindowFlags::MOUSE_FOCUS) {
                    keep = false;
                    return;
                }
                window.flags |= WindowFlags::MOUSE_FOCUS;
            }
            EventType::WINDOW_MOUSE_LEAVE => {
                if !window.flags.contains(WindowFlags::MOUSE_FOCUS) {
                    keep = false;
                    return;
                }
                window.flags &= !WindowFlags::MOUSE_FOCUS;
            }
            EventType::WINDOW_FOCUS_GAINED => {
                if window.flags.contains(WindowFlags::INPUT_FOCUS) {
                    keep = false;
                    return;
                }
                window.flags |= WindowFlags::INPUT_FOCUS;
            }
            EventType::WINDOW_FOCUS_LOST => {
                if !window.flags.contains(WindowFlags::INPUT_FOCUS) {
                    keep = false;
                    return;
                }
                window.flags &= !WindowFlags::INPUT_FOCUS;
            }
            EventType::WINDOW_DISPLAY_CHANGED => {
                if data1 == 0 || data1 as DisplayID == window.display_id {
                    keep = false;
                    return;
                }
                window.update_fullscreen_on_display_changed = true;
                window.display_id = data1 as DisplayID;
            }
            EventType::WINDOW_OCCLUDED => {
                if window.flags.contains(WindowFlags::OCCLUDED) {
                    keep = false;
                    return;
                }
                window.flags |= WindowFlags::OCCLUDED;
            }
            EventType::WINDOW_ENTER_FULLSCREEN => {
                if window.flags.contains(WindowFlags::FULLSCREEN) {
                    keep = false;
                    return;
                }
                window.flags |= WindowFlags::FULLSCREEN;
            }
            EventType::WINDOW_LEAVE_FULLSCREEN => {
                if !window.flags.contains(WindowFlags::FULLSCREEN) {
                    keep = false;
                    return;
                }
                window.flags &= !WindowFlags::FULLSCREEN;
            }
            _ => {}
        }

        if window.is_destroying && windowevent != EventType::WINDOW_DESTROYED {
            keep = false;
            return;
        }

        // The window might be destroyed in an event handler, so cache the topmost status first.
        window_is_topmost = !window.has_parent;
    });
    if !found {
        return false;
    }
    if check_pixel_size {
        video.check_window_pixel_size_changed(window_id);
    }
    if !keep {
        return false;
    }

    // Only post if we are not currently quitting
    if video.is_quitting() {
        post_event = false;
    }

    // Post the event, if desired
    let event = Event::Window(WindowEvent {
        event_type: windowevent,
        timestamp: Duration::ZERO,
        window_id,
        data1,
        data2,
    });

    WINDOW_EVENT_WATCHERS[WindowEventWatchPriority::Early as usize].dispatch(&event);
    WINDOW_EVENT_WATCHERS[WindowEventWatchPriority::Normal as usize].dispatch(&event);

    if post_event && queue::event_enabled(windowevent) {
        // Fixes queue overflow with move/resize events that aren't processed
        if matches!(
            windowevent,
            EventType::WINDOW_MOVED
                | EventType::WINDOW_RESIZED
                | EventType::WINDOW_PIXEL_SIZE_CHANGED
                | EventType::WINDOW_SAFE_AREA_CHANGED
                | EventType::WINDOW_EXPOSED
                | EventType::WINDOW_OCCLUDED
        ) {
            // RemoveSupersededWindowEvents
            queue::filter_events(
                |e| !matches!(e, Event::Window(w) if w.event_type == windowevent && w.window_id == window_id),
            );
        }
        posted = queue::push(event).unwrap_or(false);
    }

    // Ensure that the window is still valid, as it may have been destroyed in an event handler.
    let window_exists = video.with_window(window_id, &mut |_| {});

    if window_exists
        && matches!(
            windowevent,
            EventType::WINDOW_SHOWN
                | EventType::WINDOW_HIDDEN
                | EventType::WINDOW_MOVED
                | EventType::WINDOW_RESIZED
                | EventType::WINDOW_PIXEL_SIZE_CHANGED
                | EventType::WINDOW_MINIMIZED
                | EventType::WINDOW_MAXIMIZED
                | EventType::WINDOW_RESTORED
                | EventType::WINDOW_MOUSE_ENTER
                | EventType::WINDOW_MOUSE_LEAVE
                | EventType::WINDOW_FOCUS_GAINED
                | EventType::WINDOW_FOCUS_LOST
                | EventType::WINDOW_DISPLAY_CHANGED
        )
    {
        video.on_window_event(window_id, windowevent);
    }

    if windowevent == EventType::WINDOW_CLOSE_REQUESTED
        && window_is_topmost
        && !video.has_active_trays()
    {
        let mut count = if window_exists { 0 } else { 1 };
        count += video.visible_toplevel_window_count();

        if count <= 1 && hints::get_bool(hints::QUIT_ON_LAST_WINDOW_CLOSE, true) {
            queue::send_quit(); // This is the last window in the list, so send the SDL_EVENT_QUIT event
        }
    }

    posted
}

// ---------------------------------------------------------------------------
// Display events (SDL_displayevents.c)
// ---------------------------------------------------------------------------

/// Translation of `SDL_SendDisplayEvent()`.
pub fn send_display_event(display_id: DisplayID, displayevent: EventType, data1: i32, data2: i32) {
    let mut post_event = true;

    if display_id == 0 {
        return;
    }
    let Some(video) = video() else {
        return;
    };

    let mut keep = true;
    let found = video.with_display(display_id, &mut |display| {
        if displayevent == EventType::DISPLAY_ORIENTATION {
            if data1 == ORIENTATION_UNKNOWN || data1 == display.current_orientation {
                keep = false;
                return;
            }
            display.current_orientation = data1;
        }
    });
    if !found || !keep {
        return;
    }

    // Only post if we are not currently quitting
    if video.is_quitting() {
        post_event = false;
    }

    // Post the event, if desired
    if post_event && queue::event_enabled(displayevent) {
        let _ = queue::push(Event::Display(DisplayEvent {
            event_type: displayevent,
            timestamp: Duration::ZERO,
            display_id,
            data1,
            data2,
        }));
    }

    if matches!(
        displayevent,
        EventType::DISPLAY_ADDED | EventType::DISPLAY_MOVED
    ) {
        video.on_display_event(display_id, displayevent);
    }
}

// ---------------------------------------------------------------------------
// Clipboard events (SDL_clipboardevents.c)
// ---------------------------------------------------------------------------

/// Translation of `SDL_SendClipboardUpdate()`.
pub fn send_clipboard_update(owner: bool, mime_types: Vec<String>) {
    if !owner {
        if let Some(video) = video() {
            video.clipboard_lost(&mime_types);
        }
    }

    if queue::event_enabled(EventType::CLIPBOARD_UPDATE) {
        let _ = queue::push(Event::Clipboard(ClipboardEvent {
            timestamp: Duration::ZERO,
            owner,
            mime_types,
        }));
    }
}

// ---------------------------------------------------------------------------
// Drag and drop events (SDL_dropevents.c)
// ---------------------------------------------------------------------------

struct DropState {
    app_is_dropping: bool,
    last_drop_x: f32,
    last_drop_y: f32,
}

/// The `static` locals of `SDL_SendDrop()`.
static DROP_STATE: Mutex<DropState> = Mutex::new(DropState {
    app_is_dropping: false,
    last_drop_x: 0.0,
    last_drop_y: 0.0,
});

/// Translation of `SDL_SendDrop()`. `window` is `None` for app-wide drops.
fn send_drop(
    window: Option<WindowID>,
    evtype: EventType,
    source: Option<&str>,
    data: Option<&str>,
    x: f32,
    y: f32,
) -> bool {
    let mut posted = false;

    // Post the event, if desired
    if queue::event_enabled(evtype) {
        let window_id = window.unwrap_or(0);
        let window_is_dropping =
            |id: WindowID| -> Option<bool> { self::window(id).map(|w| w.is_dropping) };
        let set_window_dropping = |id: WindowID, dropping: bool| {
            if let Some(video) = video() {
                video.with_window(id, &mut |w| w.is_dropping = dropping);
            }
        };

        let need_begin = match window {
            Some(id) => !window_is_dropping(id).unwrap_or(false),
            None => {
                !DROP_STATE
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .app_is_dropping
            }
        };
        if need_begin {
            posted = queue::push(Event::Drop(DropEvent {
                event_type: EventType::DROP_BEGIN,
                timestamp: Duration::ZERO,
                window_id,
                ..Default::default()
            }))
            .unwrap_or(false);
            if !posted {
                return false;
            }
            match window {
                Some(id) => set_window_dropping(id, true),
                None => {
                    DROP_STATE
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .app_is_dropping = true
                }
            }
        }

        let (last_x, last_y) = {
            let mut s = DROP_STATE.lock().unwrap_or_else(|e| e.into_inner());
            if evtype == EventType::DROP_POSITION {
                s.last_drop_x = x;
                s.last_drop_y = y;
            }
            (s.last_drop_x, s.last_drop_y)
        };
        posted = queue::push(Event::Drop(DropEvent {
            event_type: evtype,
            timestamp: Duration::ZERO,
            window_id,
            x: last_x,
            y: last_y,
            source: source.map(str::to_owned),
            data: data.map(str::to_owned),
        }))
        .unwrap_or(false);
        if posted && evtype == EventType::DROP_COMPLETE {
            let mut s = DROP_STATE.lock().unwrap_or_else(|e| e.into_inner());
            match window {
                Some(id) => set_window_dropping(id, false),
                None => s.app_is_dropping = false,
            }
            s.last_drop_x = 0.0;
            s.last_drop_y = 0.0;
        }
    }
    posted
}

/// Translation of `SDL_SendDropFile()`.
pub fn send_drop_file(window: Option<WindowID>, source: Option<&str>, file: &str) -> bool {
    send_drop(window, EventType::DROP_FILE, source, Some(file), 0.0, 0.0)
}

/// Translation of `SDL_SendDropPosition()`.
pub fn send_drop_position(window: Option<WindowID>, x: f32, y: f32) -> bool {
    send_drop(window, EventType::DROP_POSITION, None, None, x, y)
}

/// Translation of `SDL_SendDropText()`.
pub fn send_drop_text(window: Option<WindowID>, text: &str) -> bool {
    send_drop(window, EventType::DROP_TEXT, None, Some(text), 0.0, 0.0)
}

/// Translation of `SDL_SendDropComplete()`.
pub fn send_drop_complete(window: Option<WindowID>) -> bool {
    send_drop(window, EventType::DROP_COMPLETE, None, None, 0.0, 0.0)
}

// ---------------------------------------------------------------------------
// Notification events (SDL_notificationevents.c)
// ---------------------------------------------------------------------------

/// Translation of `SDL_SendNotificationAction()`.
pub fn send_notification_action(notification_id: NotificationID, action_id: &str) -> bool {
    if queue::event_enabled(EventType::NOTIFICATION_ACTION_INVOKED) {
        return queue::push(Event::Notification(NotificationEvent {
            timestamp: Duration::ZERO,
            which: notification_id,
            action_id: action_id.to_owned(),
        }))
        .unwrap_or(false);
    }
    false
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::init;
    use std::collections::HashMap;

    /// A minimal in-memory video backend for tests.
    #[derive(Default)]
    pub(crate) struct FakeVideo {
        pub windows: Mutex<HashMap<WindowID, WindowCore>>,
        pub displays: Mutex<HashMap<DisplayID, DisplayCore>>,
        pub callbacks: Mutex<Vec<(WindowID, EventType)>>,
    }

    impl FakeVideo {
        pub fn with_windows(ids: &[WindowID]) -> Arc<Self> {
            let v = FakeVideo::default();
            for &id in ids {
                let mut w = WindowCore::new(id);
                w.w = 640;
                w.h = 480;
                v.windows.lock().unwrap().insert(id, w);
            }
            Arc::new(v)
        }
    }

    impl VideoHooks for FakeVideo {
        fn with_window(&self, id: WindowID, f: &mut dyn FnMut(&mut WindowCore)) -> bool {
            match self.windows.lock().unwrap().get_mut(&id) {
                Some(w) => {
                    f(w);
                    true
                }
                None => false,
            }
        }
        fn with_display(&self, id: DisplayID, f: &mut dyn FnMut(&mut DisplayCore)) -> bool {
            match self.displays.lock().unwrap().get_mut(&id) {
                Some(d) => {
                    f(d);
                    true
                }
                None => false,
            }
        }
        fn on_window_event(&self, id: WindowID, t: EventType) {
            self.callbacks.lock().unwrap().push((id, t));
        }
        fn visible_toplevel_window_count(&self) -> usize {
            self.windows
                .lock()
                .unwrap()
                .values()
                .filter(|w| !w.has_parent && !w.flags.contains(WindowFlags::HIDDEN))
                .count()
        }
    }

    /// Unregisters the fake video and quits events even if the test panics.
    struct CleanupOnDrop;
    impl Drop for CleanupOnDrop {
        fn drop(&mut self) {
            set_video(None);
            init::quit_subsystem(init::InitFlags::EVENTS);
        }
    }

    pub(crate) fn with_video<R>(ids: &[WindowID], f: impl FnOnce(&Arc<FakeVideo>) -> R) -> R {
        let _guard = init::tests::TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        init::init(init::InitFlags::EVENTS).unwrap();
        let video = FakeVideo::with_windows(ids);
        set_video(Some(video.clone()));
        let _cleanup = CleanupOnDrop;
        f(&video)
    }

    #[test]
    fn flags_debug_and_ops() {
        let f = WindowFlags::FULLSCREEN | WindowFlags::RESIZABLE;
        assert_eq!(format!("{f:?}"), "WindowFlags(FULLSCREEN | RESIZABLE)");
        assert_eq!(format!("{:?}", WindowFlags::NONE), "WindowFlags(0x0)");
        assert!(f.contains(WindowFlags::RESIZABLE));
        assert!(!f.contains(WindowFlags::HIDDEN));
        assert_eq!(WindowFlags::NOT_FOCUSABLE.0, 0x80000000);
        assert_eq!(WindowFlags::FILL_DOCUMENT.0, 0x200000);
    }

    #[test]
    fn window_events_update_state_and_dedupe() {
        with_video(&[1], |video| {
            // Shown clears HIDDEN; a second SHOWN is a no-op.
            assert!(send_window_event(1, EventType::WINDOW_SHOWN, 0, 0));
            assert!(!send_window_event(1, EventType::WINDOW_SHOWN, 0, 0));
            assert!(!video.window(1).unwrap().flags.contains(WindowFlags::HIDDEN));

            // Moves to the same position are dropped; superseded moves are replaced.
            assert!(send_window_event(1, EventType::WINDOW_MOVED, 10, 20));
            assert!(!send_window_event(1, EventType::WINDOW_MOVED, 10, 20));
            assert!(send_window_event(1, EventType::WINDOW_MOVED, 30, 40));
            let moves: Vec<_> =
                queue::get_events(EventType::WINDOW_MOVED, EventType::WINDOW_MOVED, 10).unwrap();
            assert_eq!(moves.len(), 1);
            assert!(matches!(
                moves[0],
                Event::Window(WindowEvent {
                    data1: 30,
                    data2: 40,
                    ..
                })
            ));
            let w = video.window(1).unwrap();
            assert_eq!((w.x, w.y, w.floating.x, w.floating.y), (30, 40, 30, 40));

            // Resize updates the size and runs the video callback.
            assert!(send_window_event(1, EventType::WINDOW_RESIZED, 800, 600));
            assert!(video
                .callbacks
                .lock()
                .unwrap()
                .contains(&(1, EventType::WINDOW_RESIZED)));
            assert_eq!(video.window(1).unwrap().windowed.w, 800);

            // Unknown windows are ignored.
            assert!(!send_window_event(99, EventType::WINDOW_SHOWN, 0, 0));
        });
    }

    #[test]
    fn close_last_window_sends_quit() {
        with_video(&[1], |_video| {
            send_window_event(1, EventType::WINDOW_SHOWN, 0, 0);
            queue::flush_events(EventType::FIRST, EventType::LAST);
            send_window_event(1, EventType::WINDOW_CLOSE_REQUESTED, 0, 0);
            assert!(queue::has_event(EventType::WINDOW_CLOSE_REQUESTED));
            assert!(queue::has_event(EventType::QUIT));
        });
    }

    #[test]
    fn early_watchers_see_window_events() {
        with_video(&[1], |_video| {
            let seen = Arc::new(Mutex::new(Vec::new()));
            let s2 = seen.clone();
            let _w = add_window_event_watch(WindowEventWatchPriority::Early, move |e| {
                s2.lock().unwrap().push(e.event_type())
            });
            send_window_event(1, EventType::WINDOW_EXPOSED, 0, 0);
            assert_eq!(*seen.lock().unwrap(), vec![EventType::WINDOW_EXPOSED]);
        });
    }

    #[test]
    fn drop_and_notification_events() {
        with_video(&[1], |_video| {
            assert!(send_drop_position(Some(1), 5.0, 6.0));
            assert!(send_drop_file(Some(1), Some("app"), "/tmp/x"));
            assert!(send_drop_complete(Some(1)));
            let events =
                queue::get_events(EventType::DROP_FIRST, EventType::DROP_LAST, 10).unwrap();
            let types: Vec<_> = events.iter().map(|e| e.event_type()).collect();
            assert_eq!(
                types,
                vec![
                    EventType::DROP_BEGIN,
                    EventType::DROP_POSITION,
                    EventType::DROP_FILE,
                    EventType::DROP_COMPLETE
                ]
            );
            match &events[2] {
                Event::Drop(d) => {
                    assert_eq!((d.x, d.y), (5.0, 6.0));
                    assert_eq!(d.source.as_deref(), Some("app"));
                    assert_eq!(d.data.as_deref(), Some("/tmp/x"));
                }
                _ => unreachable!(),
            }
            // App-wide drop (no window) tracks its own begin state.
            assert!(send_drop_text(None, "hello"));
            let events =
                queue::get_events(EventType::DROP_FIRST, EventType::DROP_LAST, 10).unwrap();
            assert_eq!(events.len(), 2);
            assert!(send_drop_complete(None));
            let events =
                queue::get_events(EventType::DROP_FIRST, EventType::DROP_LAST, 10).unwrap();
            assert_eq!(events.len(), 1);
            assert_eq!(events[0].event_type(), EventType::DROP_COMPLETE);

            assert!(send_notification_action(7, "reply"));
            match queue::poll() {
                Some(Event::Notification(n)) => {
                    assert_eq!(n.which, 7);
                    assert_eq!(n.action_id, "reply");
                }
                other => panic!("unexpected {other:?}"),
            }

            send_clipboard_update(true, vec!["text/plain".into()]);
            assert!(queue::has_event(EventType::CLIPBOARD_UPDATE));
        });
    }

    #[test]
    fn display_orientation_dedupes() {
        with_video(&[], |video| {
            video.displays.lock().unwrap().insert(
                3,
                DisplayCore {
                    id: 3,
                    current_orientation: 0,
                },
            );
            send_display_event(3, EventType::DISPLAY_ORIENTATION, ORIENTATION_UNKNOWN, 0);
            assert!(!queue::has_event(EventType::DISPLAY_ORIENTATION));
            send_display_event(3, EventType::DISPLAY_ORIENTATION, 2, 0);
            assert!(queue::has_event(EventType::DISPLAY_ORIENTATION));
            assert_eq!(video.displays.lock().unwrap()[&3].current_orientation, 2);
            send_display_event(0, EventType::DISPLAY_ADDED, 0, 0);
            assert!(!queue::has_event(EventType::DISPLAY_ADDED));
        });
    }
}
