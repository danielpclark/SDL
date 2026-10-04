// Rust translation of src/video/x11/SDL_x11window.c and SDL_x11window.h
// from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! X11 windows: creation (or wrapping an existing X window), the window
//! manager hints and EWMH state, position/size with the asynchronous
//! window manager, show/hide, fullscreen, grabs, icons and titles.
//!
//! The per-window data (`SDL_WindowData`) is [`X11WindowData`], kept in the
//! window's `internal` slot; [`with_x11_window`] reaches a window and its
//! data together.

use std::ffi::{c_char, c_int, c_long, c_uchar, c_ulong, CStr, CString};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use super::modes::{display_driver_data_for_window, with_x11_display, X11DisplayData};
use super::sys::*;
use super::video::{intern_atom, WindowListEntry, X11Video};
use super::xfixes::x11_xfixes_is_initialized;
use crate::error::{Error, Result};
use crate::events::window::{send_window_event, WindowFlags};
use crate::events::{keyboard, mouse, DisplayID, EventType, WindowID};
use crate::hints;
use crate::properties::Properties;
use crate::video::core::{update_fullscreen_mode, with_window};
use crate::video::display::{display_bounds, display_usable_bounds, primary_display};
use crate::video::sysvideo::{
    DisplayMode, FlashOperation, FullscreenOp, FullscreenResult, HitTestResult, WindowData,
};
use crate::video::window::{
    global_to_relative_for_window, relative_to_global_for_window, should_focus_popup,
    should_relinquish_popup_focus, PROP_WINDOW_CREATE_X11_WINDOW_NUMBER,
    PROP_WINDOW_X11_DISPLAY_POINTER, PROP_WINDOW_X11_SCREEN_NUMBER, PROP_WINDOW_X11_WINDOW_NUMBER,
};
use crate::video::{Point, Rect, Surface};

/// We need to queue the focus in/out changes because they may occur during
/// video mode changes and we can respond to them by triggering more mode
/// changes. Translation of `PENDING_FOCUS_TIME`.
pub(crate) const PENDING_FOCUS_TIME: u64 = 200;

const _NET_WM_STATE_REMOVE: c_long = 0;
const _NET_WM_STATE_ADD: c_long = 1;

/// Translation of `PendingFocusEnum`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum PendingFocus {
    #[default]
    None,
    In,
    Out,
}

// The `pending_operation` flags of `SDL_WindowData`.
pub(crate) const X11_PENDING_OP_NONE: u32 = 0x00;
pub(crate) const X11_PENDING_OP_RESTORE: u32 = 0x01;
pub(crate) const X11_PENDING_OP_MINIMIZE: u32 = 0x02;
pub(crate) const X11_PENDING_OP_MAXIMIZE: u32 = 0x04;
pub(crate) const X11_PENDING_OP_FULLSCREEN: u32 = 0x08;
pub(crate) const X11_PENDING_OP_MOVE: u32 = 0x10;
pub(crate) const X11_PENDING_OP_RESIZE: u32 = 0x20;

// The `size_move_event_flags` of `SDL_WindowData`.
/// Events are completely disabled.
pub(crate) const X11_SIZE_MOVE_EVENTS_DISABLE: u32 = 0x01;
/// Events are disabled until a _NET_FRAME_EXTENTS event arrives.
pub(crate) const X11_SIZE_MOVE_EVENTS_WAIT_FOR_BORDERS: u32 = 0x02;

/// What the input context callbacks are given as `client_data` (upstream
/// passes the `SDL_WindowData *`): the device and the window.
pub(crate) struct IcClientData {
    pub(crate) video: *const X11Video,
    pub(crate) window: WindowID,
}

/// The driver data of a window. Translation of `struct SDL_WindowData`.
pub(crate) struct X11WindowData {
    pub(crate) xwindow: Window,
    pub(crate) visual: *mut Visual,
    pub(crate) colormap: Colormap,
    // MIT shared memory extension information
    pub(crate) use_mitshm: bool,
    /// The shared memory segment of the framebuffer (`shminfo`), kept alive
    /// by the framebuffer surface too.
    pub(crate) shminfo: Option<std::sync::Arc<super::framebuffer::ShmSegment>>,
    pub(crate) ximage: *mut XImage,
    pub(crate) gc: GC,
    pub(crate) ic: XIC,
    #[allow(dead_code)] // (kept as upstream, which doesn't read it either)
    pub(crate) created: bool,
    pub(crate) border_left: c_int,
    pub(crate) border_right: c_int,
    pub(crate) border_top: c_int,
    pub(crate) border_bottom: c_int,
    pub(crate) xinput2_mouse_enabled: bool,
    pub(crate) xinput2_keyboard_enabled: bool,
    pub(crate) mouse_grabbed: bool,
    pub(crate) last_focus_event_time: u64,
    pub(crate) ignore_button_press_serial: c_ulong,
    pub(crate) pending_focus: PendingFocus,
    pub(crate) pending_focus_time: u64,
    pub(crate) pending_move: bool,
    pub(crate) pending_move_point: Point,
    pub(crate) last_xconfigure: XConfigureEvent,
    pub(crate) pending_xconfigure: XConfigureEvent,
    pub(crate) user_time: c_ulong,
    pub(crate) xdnd_req: Atom,
    pub(crate) xdnd_source: Window,
    pub(crate) flashing_window: bool,
    pub(crate) flash_cancel_time: u64,
    pub(crate) pointer_barrier_active: bool,
    pub(crate) barrier: [PointerBarrier; 4],
    pub(crate) barrier_rect: Rect,
    pub(crate) resize_counter: XSyncCounter,
    pub(crate) resize_id: XSyncValue,
    pub(crate) resize_in_progress: bool,

    pub(crate) expected: Rect,
    pub(crate) requested_fullscreen_mode: DisplayMode,

    pub(crate) pending_operation: u32,
    pub(crate) size_move_event_flags: u32,

    pub(crate) pending_size: bool,
    pub(crate) pending_position: bool,
    pub(crate) fs_repositioned: bool,
    pub(crate) window_was_maximized: bool,
    pub(crate) previous_borders_nonzero: bool,
    pub(crate) toggle_borders: bool,
    pub(crate) fullscreen_borders_forced_on: bool,
    pub(crate) was_shown: bool,
    pub(crate) emit_size_move_after_property_notify: bool,
    pub(crate) hit_test_result: HitTestResult,
    pub(crate) pending_grab: bool,

    pub(crate) xim_spot: XPoint,
    pub(crate) preedit_text: Option<String>,
    pub(crate) preedit_feedback: Vec<XIMFeedback>,
    pub(crate) preedit_length: c_int,
    pub(crate) preedit_cursor: c_int,
    pub(crate) ime_needs_clear_composition: bool,

    /// The `client_data` of the input context callbacks (stable address).
    pub(crate) ic_client: Box<IcClientData>,

    /// The GLES window surface (`EGLSurface`, NULL when none).
    pub(crate) egl_surface: *mut std::ffi::c_void,
}

// SAFETY: the raw pointers are Xlib objects of the display (locked by Xlib
// after XInitThreads()) and the device owning this window.
unsafe impl Send for X11WindowData {}

impl X11WindowData {
    fn new(video: &X11Video, window: WindowID, xwindow: Window) -> X11WindowData {
        X11WindowData {
            xwindow,
            visual: std::ptr::null_mut(),
            colormap: 0,
            use_mitshm: false,
            shminfo: Option::None,
            ximage: std::ptr::null_mut(),
            gc: std::ptr::null_mut(),
            ic: std::ptr::null_mut(),
            created: false,
            border_left: 0,
            border_right: 0,
            border_top: 0,
            border_bottom: 0,
            xinput2_mouse_enabled: false,
            xinput2_keyboard_enabled: false,
            mouse_grabbed: false,
            last_focus_event_time: 0,
            ignore_button_press_serial: 0,
            pending_focus: PendingFocus::None,
            pending_focus_time: 0,
            pending_move: false,
            pending_move_point: Point::default(),
            last_xconfigure: XConfigureEvent::default(),
            pending_xconfigure: XConfigureEvent::default(),
            user_time: 0,
            xdnd_req: 0,
            xdnd_source: 0,
            flashing_window: false,
            flash_cancel_time: 0,
            pointer_barrier_active: false,
            barrier: [0; 4],
            barrier_rect: Rect::default(),
            resize_counter: 0,
            resize_id: XSyncValue::default(),
            resize_in_progress: false,
            expected: Rect::default(),
            requested_fullscreen_mode: DisplayMode::default(),
            pending_operation: X11_PENDING_OP_NONE,
            size_move_event_flags: 0,
            pending_size: false,
            pending_position: false,
            fs_repositioned: false,
            window_was_maximized: false,
            previous_borders_nonzero: false,
            toggle_borders: false,
            fullscreen_borders_forced_on: false,
            was_shown: false,
            emit_size_move_after_property_notify: false,
            hit_test_result: HitTestResult::Normal,
            pending_grab: false,
            xim_spot: XPoint::default(),
            preedit_text: Option::None,
            preedit_feedback: Vec::new(),
            preedit_length: 0,
            preedit_cursor: 0,
            ime_needs_clear_composition: false,
            ic_client: Box::new(IcClientData {
                video: video as *const X11Video,
                window,
            }),
            egl_surface: std::ptr::null_mut(),
        }
    }
}

/// Run `f` on a window and its X11 data (`window` and `window->internal`).
/// Fails like `CHECK_WINDOW_DATA()`.
///
/// `f` runs with the video device borrowed: it must not call back into SDL.
pub(crate) fn with_x11_window<R>(
    window: WindowID,
    f: impl FnOnce(&mut WindowData, &mut X11WindowData) -> R,
) -> Result<R> {
    with_window(window, |w| {
        let mut internal = w.internal.take();
        let r = match internal
            .as_mut()
            .and_then(|b| b.downcast_mut::<X11WindowData>())
        {
            Some(data) => Ok(f(w, data)),
            Option::None => Err(Error::new("Invalid window driver data")),
        };
        w.internal = internal;
        r
    })?
}

/// Translation of `SDL_WINDOW_IS_POPUP()` for a window ID.
fn is_popup(window: WindowID) -> bool {
    with_window(window, |w| w.is_popup()).unwrap_or(false)
}

/// Translation of `isMapNotify()` (of `SDL_x11window.c`).
unsafe extern "C" fn is_map_notify(_dpy: *mut Display, ev: *mut XEvent, win: XPointer) -> Bool {
    // SAFETY: Xlib passes a valid event; `win` points at the Window waited for.
    unsafe {
        ((*ev).get_type() == MapNotify && (*ev).map().window == *(win as *const Window)) as Bool
    }
}

/// Translation of `isUnmapNotify()`.
unsafe extern "C" fn is_unmap_notify(_dpy: *mut Display, ev: *mut XEvent, win: XPointer) -> Bool {
    // SAFETY: as above.
    unsafe {
        ((*ev).get_type() == UnmapNotify && (*ev).unmap().window == *(win as *const Window)) as Bool
    }
}

/// Translation of `X11_CheckCurrentDesktop()`.
fn x11_check_current_desktop(name: &str) -> bool {
    if let Some(desktop_var) = crate::stdlib::getenv("DESKTOP_SESSION") {
        if desktop_var.eq_ignore_ascii_case(name) {
            return true;
        }
    }

    if let Some(desktop_var) = crate::stdlib::getenv("XDG_CURRENT_DESKTOP") {
        if crate::stdlib::string::strcasestr(&desktop_var, name).is_some() {
            return true;
        }
    }

    false
}

/// Translation of `X11_IsDisplayOk()`.
fn x11_is_display_ok(display: *mut Display) -> bool {
    // SAFETY: the display is open.
    if unsafe { DisplayFlags(display) } & XlibDisplayIOError != 0 {
        return false;
    }
    true
}

/// A property value as longs (format-32 data, which Xlib hands over and
/// takes as `long`s).
fn as_prop(data: &[c_long]) -> *const c_uchar {
    data.as_ptr() as *const c_uchar
}

/// An all-zero `XClientMessageEvent` in an `XEvent` (`SDL_zero(e)`).
fn client_message(window: Window, message_type: Atom, format: c_int) -> XEvent {
    let mut e = XEvent::zeroed();
    let c = e.client_mut();
    c.type_ = ClientMessage;
    c.message_type = message_type;
    c.format = format;
    c.window = window;
    e
}

/// The display data of a window, or the default one (screen 0) when there
/// is none (`displaydata ? displaydata->screen : 0`).
fn screen_of(displaydata: &Option<X11DisplayData>) -> c_int {
    displaydata.as_ref().map(|d| d.screen).unwrap_or(0)
}

/// `caught_x11_error`.
static CAUGHT_X11_ERROR: AtomicBool = AtomicBool::new(false);

/// Translation of `X11_CatchAnyError()`.
unsafe extern "C" fn x11_catch_any_error(_d: *mut Display, _e: *mut XErrorEvent) -> c_int {
    /* this may happen during tumultuous times when we are polling anyhow,
    so just note we had an error and return control. */
    CAUGHT_X11_ERROR.store(true, Ordering::Relaxed);
    0
}

/// Translation of `X11_SetWindowHitTest()`.
pub(crate) fn x11_set_window_hit_test(_window: WindowID, _enabled: bool) -> Result<()> {
    Ok(()) // just succeed, the real work is done elsewhere.
}

/// Translation of `SetWindowBordered()`.
fn set_window_bordered(
    x: &super::x11dyn::X11Syms,
    display: *mut Display,
    screen: c_int,
    window: Window,
    border: bool,
) {
    /*
     * this code used to check for KWM_WIN_DECORATION, but KDE hasn't
     *  supported it for years and years. It now respects _MOTIF_WM_HINTS.
     *  Gnome is similar: just use the Motif atom.
     */

    let wm_hints = intern_atom(x, display, "_MOTIF_WM_HINTS", true);
    if wm_hints != None {
        // Hints used by Motif compliant window managers
        #[repr(C)]
        struct MwmHints {
            flags: c_ulong,
            functions: c_ulong,
            decorations: c_ulong,
            input_mode: c_long,
            status: c_ulong,
        }
        let mwm_hints = MwmHints {
            flags: 1 << 1,
            functions: 0,
            decorations: if border { 1 } else { 0 },
            input_mode: 0,
            status: 0,
        };

        // SAFETY: the display is open and the window one of its windows; the
        // hints are 5 longs.
        unsafe {
            (x.XChangeProperty)(
                display,
                window,
                wm_hints,
                wm_hints,
                32,
                PropModeReplace,
                &mwm_hints as *const MwmHints as *const c_uchar,
                (size_of::<MwmHints>() / size_of::<c_long>()) as c_int,
            );
        }
    } else {
        // set the transient hints instead, if necessary
        // SAFETY: as above.
        unsafe {
            (x.XSetTransientForHint)(display, window, RootWindow(display, screen));
        }
    }
}

/// Translation of `SDL_X11_SetWindowTitle()`.
pub(crate) fn sdl_x11_set_window_title(
    x: &super::x11dyn::X11Syms,
    display: *mut Display,
    xwindow: Window,
    title: &str,
) -> Result<()> {
    let _NET_WM_NAME = intern_atom(x, display, "_NET_WM_NAME", false);
    let title_c = CString::new(title).unwrap_or_default();
    let mut title_ptr = title_c.as_ptr() as *mut c_char;
    let mut titleprop = XTextProperty::default();
    // SAFETY: the display is open; the title list has one NUL-terminated
    // string; titleprop is a valid out-parameter.
    let conv = unsafe {
        (x.XmbTextListToTextProperty)(display, &mut title_ptr, 1, XTextStyle, &mut titleprop)
    };

    // SAFETY: XSupportsLocale has no preconditions.
    if unsafe { (x.XSupportsLocale)() } != True {
        return Err(Error::new(
            "Current locale not supported by X server, cannot continue.",
        ));
    }

    if conv == 0 {
        // SAFETY: the display is open; titleprop came from Xlib and its
        // value is freed once.
        unsafe {
            (x.XSetTextProperty)(display, xwindow, &mut titleprop, XA_WM_NAME);
            (x.XFree)(titleprop.value.cast());
        }
        // we know this can't be a locale error as we checked X locale validity
    } else if conv < 0 {
        return Err(Error::out_of_memory());
    } else {
        // conv > 0
        crate::debug!(
            crate::log::Category::Video,
            "{} characters were not convertible to the current locale!",
            conv
        );
        return Ok(());
    }

    if let Some(utf8) = &x.utf8 {
        // SAFETY: as above.
        let status = unsafe {
            (utf8.Xutf8TextListToTextProperty)(
                display,
                &mut title_ptr,
                1,
                XUTF8StringStyle,
                &mut titleprop,
            )
        };
        if status == Success {
            // SAFETY: as above.
            unsafe {
                (x.XSetTextProperty)(display, xwindow, &mut titleprop, _NET_WM_NAME);
                (x.XFree)(titleprop.value.cast());
            }
        } else {
            return Err(Error::new(format!(
                "Failed to convert title to UTF8! Bad encoding, or bad Xorg encoding? Window title: «{title}»"
            )));
        }
    }

    // SAFETY: the display is open.
    unsafe {
        (x.XFlush)(display);
    }
    Ok(())
}

/// Translation of `SDL_x11Prop` (of `SDL_x11window.c`).
pub(crate) struct X11Prop {
    pub(crate) data: *mut c_uchar,
    pub(crate) format: c_int,
    pub(crate) count: c_ulong,
    #[allow(dead_code)] // (kept as upstream, which doesn't read it either)
    pub(crate) type_: Atom,
}

/// Reads property. Must call X11_XFree on results. Translation of
/// `X11_ReadProperty()`.
///
/// # Safety
///
/// `disp` must be an open display and `w` one of its windows.
pub(crate) unsafe fn x11_read_property(
    x: &super::x11dyn::X11Syms,
    disp: *mut Display,
    w: Window,
    prop: Atom,
) -> X11Prop {
    let mut ret: *mut c_uchar = std::ptr::null_mut();
    let mut type_: Atom = 0;
    let mut fmt: c_int = 0;
    let mut count: c_ulong = 0;
    let mut bytes_left: c_ulong = 0;
    let mut bytes_fetch: c_long = 0;

    loop {
        // SAFETY: the caller's contract; the out-parameters are valid; the
        // previous result is freed before it is replaced.
        unsafe {
            if !ret.is_null() {
                (x.XFree)(ret.cast());
            }
            (x.XGetWindowProperty)(
                disp,
                w,
                prop,
                0,
                bytes_fetch,
                False,
                AnyPropertyType,
                &mut type_,
                &mut fmt,
                &mut count,
                &mut bytes_left,
                &mut ret,
            );
        }
        bytes_fetch += bytes_left as c_long;
        if bytes_left == 0 {
            break;
        }
    }

    X11Prop {
        data: ret,
        format: fmt,
        count,
        type_,
    }
}

impl X11Video {
    /// Translation of `X11_IsWindowMapped()`.
    fn x11_is_window_mapped(&self, xwindow: Window) -> bool {
        // SAFETY: the display is open; attr is a valid out-parameter.
        unsafe {
            let mut attr: XWindowAttributes = std::mem::zeroed();
            (self.x.XGetWindowAttributes)(self.display, xwindow, &mut attr);
            attr.map_state != IsUnmapped
        }
    }

    /// Translation of `X11_FlushPendingEvents()`.
    fn x11_flush_pending_events(&self, window: WindowID) {
        // Serialize and restore the pending flags, as they may be overwritten while flushing.
        let Ok((last_position_pending, last_size_pending)) = with_window(window, |w| {
            (w.core.last_position_pending, w.core.last_size_pending)
        }) else {
            return;
        };

        let _ = self.x11_sync_window(window);

        let _ = with_window(window, |w| {
            w.core.last_position_pending = last_position_pending;
            w.core.last_size_pending = last_size_pending;
        });
    }

    /// Translation of `X11_SetNetWMState()`.
    pub(crate) fn x11_set_net_wm_state(&self, xwindow: Window, flags: WindowFlags) {
        let display = self.display;
        // !!! FIXME: just dereference videodata below instead of copying to locals.
        let a = self.atoms();
        let mut atoms: Vec<c_long> = Vec::with_capacity(16);

        /* The window manager sets this property, we shouldn't set it.
           If we did, this would indicate to the window manager that we don't
           actually want to be mapped during X11_XMapRaised(), which would be bad.
         *
        if ((flags & SDL_WINDOW_HIDDEN) != 0) {
            atoms[count++] = _NET_WM_STATE_HIDDEN;
        }
        */

        if flags.contains(WindowFlags::ALWAYS_ON_TOP) {
            atoms.push(a._NET_WM_STATE_ABOVE as c_long);
        }
        if flags.contains(WindowFlags::UTILITY) {
            atoms.push(a._NET_WM_STATE_SKIP_TASKBAR as c_long);
            atoms.push(a._NET_WM_STATE_SKIP_PAGER as c_long);
        }
        if flags.contains(WindowFlags::INPUT_FOCUS) {
            atoms.push(a._NET_WM_STATE_FOCUSED as c_long);
        }
        if flags.contains(WindowFlags::MAXIMIZED) {
            atoms.push(a._NET_WM_STATE_MAXIMIZED_VERT as c_long);
            atoms.push(a._NET_WM_STATE_MAXIMIZED_HORZ as c_long);
        }
        if flags.contains(WindowFlags::FULLSCREEN) {
            atoms.push(a._NET_WM_STATE_FULLSCREEN as c_long);
        }
        if flags.contains(WindowFlags::MODAL) {
            atoms.push(a._NET_WM_STATE_MODAL as c_long);
        }

        crate::sdl_assert!(atoms.len() <= 16);

        // SAFETY: the display is open and the window ours; the atoms are
        // longs (format 32).
        unsafe {
            if !atoms.is_empty() {
                (self.x.XChangeProperty)(
                    display,
                    xwindow,
                    a._NET_WM_STATE,
                    XA_ATOM,
                    32,
                    PropModeReplace,
                    as_prop(&atoms),
                    atoms.len() as c_int,
                );
            } else {
                (self.x.XDeleteProperty)(display, xwindow, a._NET_WM_STATE);
            }
        }
    }
}

/// Translation of `X11_ConstrainPopup()`.
fn x11_constrain_popup(window: WindowID, output_to_pending: bool) {
    // Clamp popup windows to the output borders
    let Ok((popup, last_position_pending, pending, floating, constrain, w, h, parent)) =
        with_window(window, |win| {
            (
                win.is_popup(),
                win.core.last_position_pending,
                win.pending,
                win.core.floating,
                win.constrain_popup,
                win.core.w,
                win.core.h,
                win.parent,
            )
        })
    else {
        return;
    };
    if !popup {
        return;
    }

    let mut abs_x = if last_position_pending {
        pending.x
    } else {
        floating.x
    };
    let mut abs_y = if last_position_pending {
        pending.y
    } else {
        floating.y
    };
    let mut offset_x = 0;
    let mut offset_y = 0;

    if constrain {
        // Calculate the total offset from the parents
        let mut cur = parent;
        while let Some(p) = cur {
            let Ok((px, py, p_popup, p_parent)) =
                with_window(p, |pw| (pw.core.x, pw.core.y, pw.is_popup(), pw.parent))
            else {
                break;
            };
            if !p_popup {
                break;
            }
            offset_x += px;
            offset_y += py;
            cur = p_parent;
        }

        let toplevel = cur;
        if let Some(t) = toplevel {
            if let Ok((tx, ty)) = with_window(t, |tw| (tw.core.x, tw.core.y)) {
                offset_x += tx;
                offset_y += ty;
            }
        }
        abs_x += offset_x;
        abs_y += offset_y;

        let display_id = toplevel
            .and_then(|t| crate::video::window::display_for_window(t).ok())
            .unwrap_or(0);

        let rect = display_bounds(display_id).unwrap_or_default();
        if abs_x + w > rect.x + rect.w {
            abs_x -= (abs_x + w) - (rect.x + rect.w);
        }
        if abs_y + h > rect.y + rect.h {
            abs_y -= (abs_y + h) - (rect.y + rect.h);
        }
        abs_x = abs_x.max(rect.x);
        abs_y = abs_y.max(rect.y);
    }

    let _ = with_window(window, |win| {
        if output_to_pending {
            win.pending.x = abs_x - offset_x;
            win.pending.y = abs_y - offset_y;
        } else {
            win.core.windowed.x = abs_x - offset_x;
            win.core.floating.x = abs_x - offset_x;
            win.core.windowed.y = abs_y - offset_y;
            win.core.floating.y = abs_y - offset_y;
        }
    });
}

/// Translation of `X11_SetKeyboardFocus()`.
pub(crate) fn x11_set_keyboard_focus(window: Option<WindowID>, set_active_focus: bool) {
    let Some(window) = window else {
        return;
    };
    let mut toplevel = window;

    // Find the toplevel parent
    while let Ok((true, Some(parent))) = with_window(toplevel, |w| (w.is_popup(), w.parent)) {
        toplevel = parent;
    }

    let _ = with_window(toplevel, |w| w.keyboard_focus = Some(window));

    let (is_hiding, is_destroying) =
        with_window(window, |w| (w.is_hiding, w.core.is_destroying)).unwrap_or((true, true));
    if set_active_focus && !is_hiding && !is_destroying {
        let _ = keyboard::set_keyboard_focus(Some(window));
    }
}

impl X11Video {
    /// Translation of `X11_GetNetWMState()`.
    pub(crate) fn x11_get_net_wm_state(&self, window: WindowID, xwindow: Window) -> WindowFlags {
        let display = self.display;
        let a = self.atoms();
        let mut actual_type: Atom = 0;
        let mut actual_format: c_int = 0;
        let mut num_items: c_ulong = 0;
        let mut bytes_after: c_ulong = 0;
        let mut property_value: *mut c_uchar = std::ptr::null_mut();
        let max_length: c_long = 1024;
        let mut flags = WindowFlags::NONE;
        let net_wm = self.with_data(|d| d.net_wm);
        let window_flags = with_window(window, |w| w.flags()).unwrap_or_default();

        // SAFETY: the display is open; the out-parameters are valid; the
        // property holds `num_items` atoms (as longs) and is freed here.
        unsafe {
            if (self.x.XGetWindowProperty)(
                display,
                xwindow,
                a._NET_WM_STATE,
                0,
                max_length,
                False,
                XA_ATOM,
                &mut actual_type,
                &mut actual_format,
                &mut num_items,
                &mut bytes_after,
                &mut property_value,
            ) == Success
            {
                let atoms: &[Atom] = if property_value.is_null() {
                    &[]
                } else {
                    std::slice::from_raw_parts(property_value as *const Atom, num_items as usize)
                };
                let mut maximized = 0;
                let mut fullscreen = 0;

                for &atom in atoms {
                    if atom == a._NET_WM_STATE_HIDDEN {
                        flags |= WindowFlags::MINIMIZED | WindowFlags::OCCLUDED;
                    } else if atom == a._NET_WM_STATE_FOCUSED {
                        flags |= WindowFlags::INPUT_FOCUS;
                    } else if atom == a._NET_WM_STATE_MAXIMIZED_VERT {
                        maximized |= 1;
                    } else if atom == a._NET_WM_STATE_MAXIMIZED_HORZ {
                        maximized |= 2;
                    } else if atom == a._NET_WM_STATE_FULLSCREEN {
                        fullscreen = 1;
                    }
                }

                if fullscreen == 1 {
                    flags |= WindowFlags::FULLSCREEN;
                } else if !net_wm {
                    // If there's no window manager, use whatever the window reports.
                    flags |= window_flags & WindowFlags::FULLSCREEN;
                }

                if maximized == 3 {
                    /* Fullscreen windows are maximized on some window managers,
                       and this is functional behavior - if maximized is removed,
                       the windows remain floating centered and not covering the
                       rest of the desktop. So we just won't change the maximize
                       state for fullscreen windows here, otherwise SDL would think
                       we're always maximized when fullscreen and not restore the
                       correct state when leaving fullscreen.
                    */
                    if fullscreen != 0 {
                        flags |= window_flags & WindowFlags::MAXIMIZED;
                    } else {
                        flags |= WindowFlags::MAXIMIZED;
                    }
                }

                /* If the window is unmapped, numItems will be zero and _NET_WM_STATE_HIDDEN
                 * will not be set. Do an additional check to see if the window is unmapped
                 * and mark it as SDL_WINDOW_HIDDEN if it is.
                 */
                {
                    let mut attr: XWindowAttributes = std::mem::zeroed();
                    (self.x.XGetWindowAttributes)(display, xwindow, &mut attr);
                    if attr.map_state == IsUnmapped {
                        flags |= WindowFlags::HIDDEN;
                    }
                }
                (self.x.XFree)(property_value.cast());
            }
        }

        // FIXME, check the size hints for resizable
        // flags |= SDL_WINDOW_RESIZABLE;

        flags
    }

    /// Translation of `SetupWindowData()`.
    fn setup_window_data(&self, window: WindowID, w: Window) -> Result<()> {
        let displaydata = display_driver_data_for_window(window);
        let display = self.display;

        // Allocate the window data
        let mut data = X11WindowData::new(self, window, w);
        data.hit_test_result = HitTestResult::Normal;

        self.x11_create_input_context(&mut data);

        // Associate the data with the window

        // Fill in the SDL window with the window data
        let border_top = data.border_top;
        // SAFETY: the display is open; attrib is a valid out-parameter.
        let attrib = unsafe {
            let mut attrib: XWindowAttributes = std::mem::zeroed();
            (self.x.XGetWindowAttributes)(display, w, &mut attrib);
            attrib
        };
        with_window(window, |win| {
            if !win.is_popup() {
                win.core.x = attrib.x;
                win.core.windowed.x = attrib.x;
                win.core.floating.x = attrib.x;
                win.core.y = attrib.y - border_top;
                win.core.windowed.y = attrib.y - border_top;
                win.core.floating.y = attrib.y - border_top;
            }
            win.core.w = attrib.width;
            win.core.windowed.w = attrib.width;
            win.core.floating.w = attrib.width;
            win.core.h = attrib.height;
            win.core.windowed.h = attrib.height;
            win.core.floating.h = attrib.height;
            if attrib.map_state != IsUnmapped {
                win.core.flags &= !WindowFlags::HIDDEN;
            } else {
                win.core.flags |= WindowFlags::HIDDEN;
            }
        })?;
        data.visual = attrib.visual;
        data.colormap = attrib.colormap;

        let state = self.x11_get_net_wm_state(window, w);
        with_window(window, |win| win.core.flags |= state)?;

        {
            let mut focal_window: Window = 0;
            let mut revert_to = 0;
            // SAFETY: the display is open; the out-parameters are valid.
            unsafe {
                (self.x.XGetInputFocus)(display, &mut focal_window, &mut revert_to);
            }
            let flags = with_window(window, |win| {
                if focal_window == w {
                    win.core.flags |= WindowFlags::INPUT_FOCUS;
                }
                win.flags()
            })?;

            if flags.contains(WindowFlags::INPUT_FOCUS) {
                // (the window has no driver data yet, like upstream's)
                let _ = keyboard::set_keyboard_focus(Some(window));
            }

            if flags.contains(WindowFlags::MOUSE_GRABBED) {
                // Tell x11 to clip mouse
            }
        }

        if with_window(window, |win| win.flags().contains(WindowFlags::EXTERNAL))? {
            // Query the title from the existing window
            let title = self.x11_get_window_title(w);
            with_window(window, |win| win.title = Some(title))?;
        }

        let props: Properties = with_window(window, |win| win.properties())?;
        let screen = screen_of(&displaydata);
        let _ = props.set(PROP_WINDOW_X11_DISPLAY_POINTER, display as usize as i64);
        let _ = props.set(PROP_WINDOW_X11_SCREEN_NUMBER, screen as i64);
        let _ = props.set(PROP_WINDOW_X11_WINDOW_NUMBER, w as i64);

        if with_window(window, |win| win.flags().contains(WindowFlags::OPENGL))?
            && self.x11_gl_window_uses_egl()
        {
            if !crate::video::egl::is_loaded() {
                // FIXME (upstream): this fails without setting an error.
                self.x11_destroy_input_context(&mut data);
                return Err(Error::new("EGL not initialized"));
            }

            // Create the GLES window surface
            // FIXME (upstream): the surface is never destroyed with the
            // window (only eglTerminate() frees it, when the library is
            // unloaded).
            match crate::video::egl::create_surface(Some(window), w as usize as *mut _) {
                Ok(surface) => data.egl_surface = surface,
                Err(_) => {
                    self.x11_destroy_input_context(&mut data);
                    return Err(Error::new("Could not create GLES window surface"));
                }
            }
        }

        // All done!
        with_window(window, |win| win.internal = Some(Box::new(data)))?;
        self.with_data(|d| d.windowlist.push(WindowListEntry { window, xwindow: w }));
        Ok(())
    }

    /// Translation of `SetupWindowInput()`.
    fn setup_window_input(&self, window: WindowID) {
        let mut fevent: c_long = 0;
        let Ok((xwindow, ic)) = with_x11_window(window, |_, d| (d.xwindow, d.ic)) else {
            return;
        };

        if let (Some(utf8), false) = (&self.x.utf8, ic.is_null()) {
            // SAFETY: the input context is valid; XNFilterEvents takes a
            // pointer to a long; the list ends with NULL.
            unsafe {
                (utf8.XGetICValues)(
                    ic,
                    XNFilterEvents.as_ptr(),
                    &mut fevent as *mut c_long,
                    std::ptr::null_mut::<c_char>(),
                );
            }
        }

        self.x11_xinput2_select(window);

        {
            let mut x11_keyboard_events = KeyPressMask | KeyReleaseMask;
            let mut x11_pointer_events = ButtonPressMask | ButtonReleaseMask | PointerMotionMask;

            self.x11_xinput2_select_mouse_and_keyboard(window);

            // If XInput2 can handle pointer and keyboard events, we don't track them here
            let (keyboard_enabled, mouse_enabled) = with_x11_window(window, |_, d| {
                (d.xinput2_keyboard_enabled, d.xinput2_mouse_enabled)
            })
            .unwrap_or((false, false));
            if keyboard_enabled {
                x11_keyboard_events = 0;
            }
            if mouse_enabled {
                x11_pointer_events = 0;
            }

            // SAFETY: the display is open and the window ours.
            unsafe {
                (self.x.XSelectInput)(
                    self.display,
                    xwindow,
                    FocusChangeMask
                        | EnterWindowMask
                        | LeaveWindowMask
                        | ExposureMask
                        | x11_keyboard_events
                        | x11_pointer_events
                        | PropertyChangeMask
                        | StructureNotifyMask
                        | KeymapStateMask
                        | fevent,
                );
            }
        }
    }

    /// Translation of `X11_CreateWindow()`.
    pub(crate) fn x11_create_window(
        &self,
        window: WindowID,
        create_props: &Properties,
    ) -> Result<()> {
        let w = create_props
            .get_number(PROP_WINDOW_CREATE_X11_WINDOW_NUMBER)
            .or_else(|| create_props.get_number("sdl2-compat.external_window"))
            .unwrap_or(0) as Window;
        if w != 0 {
            with_window(window, |win| win.core.flags |= WindowFlags::EXTERNAL)?;

            self.setup_window_data(window, w)?;

            if hints::get_bool(hints::VIDEO_X11_EXTERNAL_WINDOW_INPUT, true) {
                self.setup_window_input(window);
            }
            return Ok(());
        }

        let Some(displaydata) = display_driver_data_for_window(window) else {
            return Err(Error::new("Could not find display info"));
        };

        let force_override_redirect = hints::get_bool(hints::X11_FORCE_OVERRIDE_REDIRECT, false);
        let window_flags = with_window(window, |win| win.flags())?;
        let use_resize_sync = hints::get_bool(hints::VIDEO_X11_ENABLE_XSYNC_EXT, false)
            && window_flags.contains(WindowFlags::OPENGL); // Doesn't work well with Vulkan
        let display = self.display;
        let x = &self.x;
        let screen = displaydata.screen;
        let mut compositor: c_long;

        let transparent = window_flags.contains(WindowFlags::TRANSPARENT);
        let forced_visual_id =
            hints::get(hints::VIDEO_X11_WINDOW_VISUALID).filter(|h| !h.is_empty());
        let display_visual_id = hints::get(hints::VIDEO_X11_VISUALID).filter(|h| !h.is_empty());

        let (visual, depth) = if let Some(forced_visual_id) = forced_visual_id {
            let mut template = XVisualInfo {
                visualid: crate::stdlib::string::strtol(&forced_visual_id, 0).0 as VisualID,
                ..XVisualInfo::default()
            };
            let mut nvis: c_int = 0;
            // SAFETY: the display is open; the result is freed with XFree.
            unsafe {
                let vi = (x.XGetVisualInfo)(display, VisualIDMask, &mut template, &mut nvis);
                if vi.is_null() {
                    // FIXME (upstream): this fails without setting an error.
                    return Err(Error::new("Couldn't find the forced X11 visual"));
                }
                let r = ((*vi).visual, (*vi).depth);
                (x.XFree)(vi.cast());
                r
            }
        } else if window_flags.contains(WindowFlags::OPENGL) && display_visual_id.is_none() {
            let vinfo = if self.x11_gl_window_uses_egl() {
                self.x11_gles_get_visual(display, screen, transparent)?
            } else {
                self.x11_gl_get_visual(display, screen, transparent)?
            };
            // SAFETY: the visual info came from Xlib/GLX and is freed here.
            unsafe {
                let r = ((*vinfo.as_ptr()).visual, (*vinfo.as_ptr()).depth);
                (x.XFree)(vinfo.as_ptr().cast());
                r
            }
        } else {
            (displaydata.visual, displaydata.depth)
        };

        // SAFETY: an all-zero XSetWindowAttributes is valid (plain data).
        let mut xattr: XSetWindowAttributes = unsafe { std::mem::zeroed() };
        xattr.override_redirect = (window_flags.contains(WindowFlags::TOOLTIP)
            || window_flags.contains(WindowFlags::POPUP_MENU)
            || force_override_redirect) as Bool;
        xattr.backing_store = NotUseful;
        xattr.background_pixmap = None;
        xattr.border_pixel = 0;

        // SAFETY: the visual of an open display.
        let visual_ref = unsafe { &*visual };
        if visual_ref.class == DirectColor {
            // SAFETY: the display is open; the screen and visual are its.
            xattr.colormap = unsafe {
                (x.XCreateColormap)(display, RootWindow(display, screen), visual, AllocAll)
            };

            // If we can't create a colormap, then we must die
            if xattr.colormap == 0 {
                return Err(Error::new("Could not create writable colormap"));
            }

            // OK, we got a colormap, now fill it in as best as we can
            let ncolors = visual_ref.map_entries;
            let mut colorcells = vec![XColor::default(); ncolors.max(0) as usize];
            let rmax: u32 = 0xffff;
            let gmax: u32 = 0xffff;
            let bmax: u32 = 0xffff;

            let mut rshift = 0;
            let mut rmask = visual_ref.red_mask as u32;
            while 0 == (rmask & 1) {
                rshift += 1;
                rmask >>= 1;
            }

            let mut gshift = 0;
            let mut gmask = visual_ref.green_mask as u32;
            while 0 == (gmask & 1) {
                gshift += 1;
                gmask >>= 1;
            }

            let mut bshift = 0;
            let mut bmask = visual_ref.blue_mask as u32;
            while 0 == (bmask & 1) {
                bshift += 1;
                bmask >>= 1;
            }

            // build the color table pixel values
            let n1 = (ncolors - 1) as u32;
            for (i, cell) in colorcells.iter_mut().enumerate() {
                let i = i as u32;
                let red = (rmax * i) / n1;
                let green = (gmax * i) / n1;
                let blue = (bmax * i) / n1;

                let rbits = (rmask * i) / n1;
                let gbits = (gmask * i) / n1;
                let bbits = (bmask * i) / n1;

                let pix = (rbits << rshift) | (gbits << gshift) | (bbits << bshift);

                cell.pixel = pix as c_ulong;

                cell.red = red as u16;
                cell.green = green as u16;
                cell.blue = blue as u16;

                cell.flags = DoRed | DoGreen | DoBlue;
            }

            // SAFETY: the display is open; the cells array has ncolors entries.
            unsafe {
                (x.XStoreColors)(display, xattr.colormap, colorcells.as_mut_ptr(), ncolors);
            }
        } else {
            // SAFETY: as above.
            xattr.colormap = unsafe {
                (x.XCreateColormap)(display, RootWindow(display, screen), visual, AllocNone)
            };
        }

        let (undefined_x, undefined_y, display_id) = with_window(window, |win| {
            (
                win.core.undefined_x,
                win.core.undefined_y,
                win.core.display_id,
            )
        })?;
        let undefined_position = undefined_x && undefined_y && Ok(display_id) == primary_display();

        if is_popup(window) {
            x11_constrain_popup(window, false);
        }
        let (floating, pending_flags) =
            with_window(window, |win| (win.core.floating, win.core.pending_flags))?;
        let (win_x, win_y) = relative_to_global_for_window(window, floating.x, floating.y);

        /* Always create this with the window->floating.* fields; if we're creating a windowed mode window,
         * that's fine. If we're creating a maximized or fullscreen window, the window manager will want to
         * know these values so it can use them if we go _back_ to the base floating windowed mode. SDL manages
         * migration to fullscreen after CreateSDLWindow returns, which will put all the SDL_Window fields and
         * system state as expected.
         */
        // SAFETY: the display is open; xattr is fully initialized.
        let w = unsafe {
            (x.XCreateWindow)(
                display,
                RootWindow(display, screen),
                win_x,
                win_y,
                floating.w as u32,
                floating.h as u32,
                0,
                depth,
                InputOutput,
                visual,
                CWOverrideRedirect | CWBackPixmap | CWBorderPixel | CWBackingStore | CWColormap,
                &mut xattr,
            )
        };
        if w == 0 {
            return Err(Error::new("Couldn't create window"));
        }

        /* Don't set the borderless flag if we're about to go fullscreen.
         * This prevents the window manager from moving a full-screen borderless
         * window to a different display before we actually go fullscreen.
         */
        if !pending_flags.contains(WindowFlags::FULLSCREEN) {
            set_window_bordered(
                x,
                display,
                screen,
                w,
                !window_flags.contains(WindowFlags::BORDERLESS),
            );
        }

        let (window_group, pid) = self.with_data(|d| (d.window_group, d.pid));
        let exe_name =
            CString::new(crate::filesystem::get_exe_name().unwrap_or_default()).unwrap_or_default();
        let app_id = CString::new(crate::core::unix::app_id()).unwrap_or_default();

        // SAFETY: the display is open and w is our new window; the hints
        // are allocated by Xlib and freed here; the class hint strings
        // outlive the call.
        unsafe {
            let sizehints = (x.XAllocSizeHints)();
            // Setup the normal size hints
            (*sizehints).flags = 0;
            if !window_flags.contains(WindowFlags::RESIZABLE) {
                (*sizehints).min_width = floating.w;
                (*sizehints).max_width = floating.w;
                (*sizehints).min_height = floating.h;
                (*sizehints).max_height = floating.h;
                (*sizehints).flags |= PMaxSize | PMinSize;
            }
            if !undefined_position {
                (*sizehints).x = win_x;
                (*sizehints).y = win_y;
                (*sizehints).flags |= USPosition;
            }

            // Setup the input hints so we get keyboard input
            let wmhints = (x.XAllocWMHints)();
            (*wmhints).input = (!window_flags.contains(WindowFlags::NOT_FOCUSABLE)) as Bool;
            (*wmhints).window_group = window_group;
            (*wmhints).flags = InputHint | WindowGroupHint;

            // Setup the class hints so we can get an icon (AfterStep)
            let classhints = (x.XAllocClassHint)();
            (*classhints).res_name = exe_name.as_ptr() as *mut c_char;
            (*classhints).res_class = app_id.as_ptr() as *mut c_char;

            // Set the size, input and class hints, and define WM_CLIENT_MACHINE and WM_LOCALE_NAME
            (x.XSetWMProperties)(
                display,
                w,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                0,
                sizehints,
                wmhints,
                classhints,
            );

            (x.XFree)(sizehints.cast());
            (x.XFree)(wmhints.cast());
            (x.XFree)(classhints.cast());
            // Set the PID related to the window for the given hostname, if possible
            if pid > 0 {
                let pid = [pid as c_long];
                let _NET_WM_PID = intern_atom(x, display, "_NET_WM_PID", false);
                (x.XChangeProperty)(
                    display,
                    w,
                    _NET_WM_PID,
                    XA_CARDINAL,
                    32,
                    PropModeReplace,
                    as_prop(&pid),
                    1,
                );
            }
        }

        // Set the window manager state
        self.x11_set_net_wm_state(w, window_flags);

        compositor = 2; // don't disable compositing except for "normal" windows
        let hint = hints::get(hints::X11_WINDOW_TYPE);
        let wintype_name = if window_flags.contains(WindowFlags::UTILITY) {
            "_NET_WM_WINDOW_TYPE_UTILITY".to_owned()
        } else if window_flags.contains(WindowFlags::TOOLTIP) {
            "_NET_WM_WINDOW_TYPE_TOOLTIP".to_owned()
        } else if window_flags.contains(WindowFlags::POPUP_MENU) {
            "_NET_WM_WINDOW_TYPE_POPUP_MENU".to_owned()
        } else if let Some(hint) = hint.filter(|h| !h.is_empty()) {
            hint
        } else {
            compositor = 1; // disable compositing for "normal" windows
            "_NET_WM_WINDOW_TYPE_NORMAL".to_owned()
        };

        // Let the window manager know what type of window we are.
        let _NET_WM_WINDOW_TYPE = intern_atom(x, display, "_NET_WM_WINDOW_TYPE", false);
        let wintype = [intern_atom(x, display, &wintype_name, false) as c_long];
        // SAFETY: the display is open and the window ours; format-32 data
        // as longs.
        unsafe {
            (x.XChangeProperty)(
                display,
                w,
                _NET_WM_WINDOW_TYPE,
                XA_ATOM,
                32,
                PropModeReplace,
                as_prop(&wintype),
                1,
            );
            if hints::get_bool(hints::VIDEO_X11_NET_WM_BYPASS_COMPOSITOR, true) {
                let _NET_WM_BYPASS_COMPOSITOR =
                    intern_atom(x, display, "_NET_WM_BYPASS_COMPOSITOR", false);
                let compositor = [compositor];
                (x.XChangeProperty)(
                    display,
                    w,
                    _NET_WM_BYPASS_COMPOSITOR,
                    XA_CARDINAL,
                    32,
                    PropModeReplace,
                    as_prop(&compositor),
                    1,
                );
            }
        }

        {
            let atoms = self.atoms();
            let mut protocols: Vec<Atom> = Vec::with_capacity(4);

            protocols.push(atoms.WM_DELETE_WINDOW); // Allow window to be deleted by the WM

            // Default to using ping if there is no hint
            if hints::get_bool(hints::VIDEO_X11_NET_WM_PING, true) {
                protocols.push(atoms._NET_WM_PING); // Respond so WM knows we're alive
            }

            if use_resize_sync {
                protocols.push(atoms._NET_WM_SYNC_REQUEST); // Respond after completing resize
            }

            crate::sdl_assert!(protocols.len() <= 4);

            // SAFETY: the display is open and the window ours.
            unsafe {
                (x.XSetWMProtocols)(display, w, protocols.as_mut_ptr(), protocols.len() as c_int);
            }
        }

        if let Err(e) = self.setup_window_data(window, w) {
            // SAFETY: the display is open; w is our window.
            unsafe {
                (x.XDestroyWindow)(display, w);
            }
            return Err(e);
        }

        // Set the parent if this is a non-popup window.
        let parent = with_window(window, |win| {
            if win.is_popup() {
                Option::None
            } else {
                win.parent
            }
        })?;
        if let Some(parent) = parent {
            if let Ok(parent_xwindow) = with_x11_window(parent, |_, d| d.xwindow) {
                // SAFETY: the display is open; both windows are ours.
                unsafe {
                    (x.XSetTransientForHint)(display, w, parent_xwindow);
                }
            }
        }

        // Set the flag if the borders were forced on when creating a fullscreen window for later removal.
        with_x11_window(window, |win, d| {
            d.fullscreen_borders_forced_on =
                win.core.pending_flags.contains(WindowFlags::FULLSCREEN)
                    && win.flags().contains(WindowFlags::BORDERLESS);
        })?;

        if use_resize_sync {
            let _ = self.x11_init_resize_sync(window);
        }

        // Tooltips do not receive input
        if window_flags.contains(WindowFlags::TOOLTIP) {
            // FIXME (upstream): XShapeCombineRegion is called without
            // checking SDL_X11_HAVE_XSHAPE (skipped here without it).
            if let Some(xshape) = &x.xshape {
                // SAFETY: the display is open; the region is created and
                // destroyed here.
                unsafe {
                    let region = (x.XCreateRegion)();
                    (xshape.XShapeCombineRegion)(display, w, ShapeInput, 0, 0, region, ShapeSet);
                    (x.XDestroyRegion)(region);
                }
            }
        }

        self.setup_window_input(window);

        // For _ICC_PROFILE.
        // SAFETY: the display is open.
        unsafe {
            (x.XSelectInput)(display, RootWindow(display, screen), PropertyChangeMask);

            (x.XFlush)(display);
        }

        Ok(())
    }

    /// Translation of `X11_GetWindowTitle()`.
    pub(crate) fn x11_get_window_title(&self, xwindow: Window) -> String {
        let display = self.display;
        let atoms = self.atoms();
        let mut real_format: c_int = 0;
        let mut real_type: Atom = 0;
        let mut items_read: c_ulong = 0;
        let mut items_left: c_ulong = 0;
        let mut propdata: *mut c_uchar = std::ptr::null_mut();

        // SAFETY: the display is open; the out-parameters are valid; the
        // property data is NUL-terminated and freed here.
        unsafe {
            let status = (self.x.XGetWindowProperty)(
                display,
                xwindow,
                atoms._NET_WM_NAME,
                0,
                8192,
                False,
                atoms.UTF8_STRING,
                &mut real_type,
                &mut real_format,
                &mut items_read,
                &mut items_left,
                &mut propdata,
            );
            if status == Success && !propdata.is_null() {
                let title = CStr::from_ptr(propdata.cast())
                    .to_string_lossy()
                    .into_owned();
                (self.x.XFree)(propdata.cast());
                title
            } else {
                let status = (self.x.XGetWindowProperty)(
                    display,
                    xwindow,
                    XA_WM_NAME,
                    0,
                    8192,
                    False,
                    XA_STRING,
                    &mut real_type,
                    &mut real_format,
                    &mut items_read,
                    &mut items_left,
                    &mut propdata,
                );
                if status == Success && !propdata.is_null() {
                    let bytes = std::slice::from_raw_parts(propdata, items_read as usize + 1);
                    let title = crate::stdlib::iconv::iconv_string("UTF-8", "", bytes)
                        .map(|b| {
                            let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
                            String::from_utf8_lossy(&b[..end]).into_owned()
                        })
                        .unwrap_or_default();
                    crate::debug!(
                        crate::log::Category::Video,
                        "Failed to convert WM_NAME title expecting UTF8! Title: {}",
                        title
                    );
                    (self.x.XFree)(propdata.cast());
                    title
                } else {
                    crate::debug!(
                        crate::log::Category::Video,
                        "Could not get any window title response from Xorg, returning empty string!"
                    );
                    String::new()
                }
            }
        }
    }

    /// Translation of `X11_SetWindowTitle()`.
    pub(crate) fn x11_set_window_title(&self, window: WindowID) {
        let Ok((xwindow, title)) = with_x11_window(window, |w, d| {
            (d.xwindow, w.title.clone().unwrap_or_default())
        }) else {
            return;
        };

        let _ = sdl_x11_set_window_title(&self.x, self.display, xwindow, &title);
    }

    /// Translation of `X11_ExternalResizeMoveSync()`.
    fn x11_external_resize_move_sync(&self, window: WindowID) {
        let display = self.display;
        let x = &self.x;
        let mut child_count: u32 = 0;
        let mut child_return: Window = 0;
        let mut root: Window = 0;
        let mut parent: Window = 0;
        let mut children: *mut Window = std::ptr::null_mut();
        // SAFETY: an all-zero XWindowAttributes is valid (plain data).
        let mut attrs: XWindowAttributes = unsafe { std::mem::zeroed() };
        let mut wx = 0;
        let mut wy = 0;
        let Ok((xwindow, pending_operation)) =
            with_x11_window(window, |_, d| (d.xwindow, d.pending_operation))
        else {
            return;
        };
        let send_move = pending_operation & X11_PENDING_OP_MOVE != 0;
        let send_resize = pending_operation & X11_PENDING_OP_RESIZE != 0;

        // SAFETY: the display is open and the window ours; the handler is a
        // valid extern fn.
        let prev_handler = unsafe {
            (x.XSync)(display, False);
            (x.XQueryTree)(
                display,
                xwindow,
                &mut root,
                &mut parent,
                &mut children,
                &mut child_count,
            );
            (x.XSetErrorHandler)(Some(x11_catch_any_error))
        };
        if !children.is_null() {
            // (upstream leaks the children list)
            // SAFETY: the list from XQueryTree, freed once.
            unsafe {
                (x.XFree)(children.cast());
            }
        }

        /* Wait a brief time to see if the window manager decided to let the move or resize happen.
         * If the window changes at all, even to an unexpected value, we break out.
         */
        let timeout = crate::timer::ticks_ns() + 100_000_000;
        loop {
            CAUGHT_X11_ERROR.store(false, Ordering::Relaxed);
            // SAFETY: as above.
            unsafe {
                (x.XSync)(display, False);
                (x.XGetWindowAttributes)(display, xwindow, &mut attrs);
                (x.XTranslateCoordinates)(
                    display,
                    parent,
                    DefaultRootWindow(display),
                    attrs.x,
                    attrs.y,
                    &mut wx,
                    &mut wy,
                    &mut child_return,
                );
            }
            (wx, wy) = global_to_relative_for_window(window, wx, wy);

            if !CAUGHT_X11_ERROR.load(Ordering::Relaxed) {
                let done = with_x11_window(window, |_, data| {
                    if (data.pending_operation & X11_PENDING_OP_MOVE) != 0
                        && wx == data.expected.x + data.border_left
                        && wy == data.expected.y + data.border_top
                    {
                        data.pending_operation &= !X11_PENDING_OP_MOVE;
                    }
                    if (data.pending_operation & X11_PENDING_OP_RESIZE) != 0
                        && attrs.width == data.expected.w
                        && attrs.height == data.expected.h
                    {
                        data.pending_operation &= !X11_PENDING_OP_RESIZE;
                    }

                    data.pending_operation == X11_PENDING_OP_NONE
                })
                .unwrap_or(true);
                if done {
                    break;
                }
            }

            if crate::timer::ticks_ns() >= timeout {
                // Timed out without the expected values. Update the requested data so future sync calls won't block.
                let _ = with_x11_window(window, |_, data| {
                    data.pending_operation &= !(X11_PENDING_OP_MOVE | X11_PENDING_OP_RESIZE);
                    data.expected.x = wx;
                    data.expected.y = wy;
                    data.expected.w = attrs.width;
                    data.expected.h = attrs.height;
                });
                break;
            }

            crate::timer::delay(Duration::from_millis(10));
        }

        if !CAUGHT_X11_ERROR.load(Ordering::Relaxed) {
            if send_move {
                send_window_event(window, EventType::WINDOW_MOVED, wx, wy);
            }
            if send_resize {
                send_window_event(window, EventType::WINDOW_RESIZED, attrs.width, attrs.height);
            }
        }

        // SAFETY: restoring the previous handler.
        unsafe {
            (x.XSetErrorHandler)(prev_handler);
        }
        CAUGHT_X11_ERROR.store(false, Ordering::Relaxed);
    }

    /// Wait a brief time, or not, to see if the window manager decided to move/resize the window.
    /// Send MOVED and RESIZED window events. Translation of `X11_SyncWindowTimeout()`.
    fn x11_sync_window_timeout(&self, window: WindowID, param_timeout: u64) -> bool {
        let display = self.display;
        let x = &self.x;
        let mut timeout = 0;
        let mut force_exit = false;
        let mut result = true;

        // SAFETY: the display is open; the handler is a valid extern fn.
        let prev_handler = unsafe {
            (x.XSync)(display, False);
            (x.XSetErrorHandler)(Some(x11_catch_any_error))
        };

        if param_timeout != 0 {
            timeout = crate::timer::ticks_ns() + param_timeout;
        }

        loop {
            // SAFETY: the display is open.
            unsafe {
                (x.XSync)(display, False);
            }
            self.x11_pump_events();

            let state = with_x11_window(window, |w, data| {
                if (data.pending_operation & X11_PENDING_OP_MOVE) != 0
                    && w.core.x == data.expected.x + data.border_left
                    && w.core.y == data.expected.y + data.border_top
                {
                    data.pending_operation &= !X11_PENDING_OP_MOVE;
                }
                if (data.pending_operation & X11_PENDING_OP_RESIZE) != 0
                    && w.core.w == data.expected.w
                    && w.core.h == data.expected.h
                {
                    data.pending_operation &= !X11_PENDING_OP_RESIZE;
                }

                let in_expected_state = w.core.x == data.expected.x + data.border_left
                    && w.core.y == data.expected.y + data.border_top
                    && w.core.w == data.expected.w
                    && w.core.h == data.expected.h;
                (
                    data.pending_operation == X11_PENDING_OP_NONE,
                    in_expected_state,
                )
            });
            let Ok((nothing_pending, in_expected_state)) = state else {
                break;
            };

            if nothing_pending {
                if force_exit || in_expected_state {
                    // The window is in the expected state and nothing is pending. Done.
                    break;
                }

                /* No operations are pending, but the window still isn't in the expected state.
                 * Try one more time before exiting.
                 */
                force_exit = true;
            }

            if crate::timer::ticks_ns() >= timeout {
                // Timed out without the expected values. Update the requested data so future sync calls won't block.
                let _ = with_x11_window(window, |w, data| {
                    data.expected.x = w.core.x;
                    data.expected.y = w.core.y;
                    data.expected.w = w.core.w;
                    data.expected.h = w.core.h;
                });

                result = false;
                break;
            }

            crate::timer::delay(Duration::from_millis(10));
        }

        let _ = with_x11_window(window, |_, data| {
            data.pending_operation = X11_PENDING_OP_NONE
        });

        if !CAUGHT_X11_ERROR.load(Ordering::Relaxed) {
            self.x11_pump_events();
        } else {
            result = false;
        }

        // SAFETY: restoring the previous handler.
        unsafe {
            (x.XSetErrorHandler)(prev_handler);
        }
        CAUGHT_X11_ERROR.store(false, Ordering::Relaxed);

        result
    }

    /// Translation of `X11_SetWindowIcon()`.
    pub(crate) fn x11_set_window_icon(
        &self,
        window: WindowID,
        icon: Option<&Surface<'static>>,
    ) -> Result<()> {
        let display = self.display;
        let x = &self.x;
        let _NET_WM_ICON = self.atoms()._NET_WM_ICON;
        let mut prev_handler: XErrorHandler = Option::None;
        let mut result = Ok(());
        let xwindow = with_x11_window(window, |_, d| d.xwindow)?;

        if let Some(icon) = icon {
            // Set the _NET_WM_ICON property
            crate::sdl_assert!(icon.format() == crate::video::PixelFormat::ARGB8888);
            let (w, h) = (icon.width(), icon.height());
            let propsize = 2 + (w * h) as usize;
            let mut propdata: Vec<c_long> = Vec::with_capacity(propsize);

            // SAFETY: the display is open; the handler is a valid extern fn.
            unsafe {
                (x.XSync)(display, False);
                prev_handler = (x.XSetErrorHandler)(Some(x11_catch_any_error));
            }

            propdata.push(w as c_long);
            propdata.push(h as c_long);

            let pixels = icon.pixels().unwrap_or(&[]);
            let pitch = icon.pitch() as usize;
            for y in 0..h as usize {
                let row = &pixels[y * pitch..];
                for xx in 0..w as usize {
                    let p = u32::from_ne_bytes([
                        row[xx * 4],
                        row[xx * 4 + 1],
                        row[xx * 4 + 2],
                        row[xx * 4 + 3],
                    ]);
                    propdata.push(p as c_long);
                }
            }

            // SAFETY: the display is open and the window ours; format-32
            // data as longs.
            unsafe {
                (x.XChangeProperty)(
                    display,
                    xwindow,
                    _NET_WM_ICON,
                    XA_CARDINAL,
                    32,
                    PropModeReplace,
                    as_prop(&propdata),
                    propsize as c_int,
                );
            }

            if CAUGHT_X11_ERROR.load(Ordering::Relaxed) {
                result = Err(Error::new(
                    "An error occurred while trying to set the window's icon",
                ));
            }
        }

        // SAFETY: the display is open.
        unsafe {
            (x.XFlush)(display);
        }

        if prev_handler.is_some() {
            // SAFETY: restoring the previous handler.
            unsafe {
                (x.XSetErrorHandler)(prev_handler);
            }
            CAUGHT_X11_ERROR.store(false, Ordering::Relaxed);
        }

        result
    }

    /// Translation of `X11_UpdateWindowPosition()`.
    pub(crate) fn x11_update_window_position(&self, window: WindowID, use_current_position: bool) {
        let display = self.display;
        let Ok((rel_x, rel_y, border_left, border_top, hidden)) =
            with_x11_window(window, |w, data| {
                (
                    if use_current_position {
                        w.core.x
                    } else {
                        w.pending.x
                    },
                    if use_current_position {
                        w.core.y
                    } else {
                        w.pending.y
                    },
                    data.border_left,
                    data.border_top,
                    w.flags().contains(WindowFlags::HIDDEN),
                )
            })
        else {
            return;
        };

        let (ex, ey) =
            relative_to_global_for_window(window, rel_x - border_left, rel_y - border_top);

        let _ = with_x11_window(window, |_, data| {
            data.expected.x = ex;
            data.expected.y = ey;

            // Attempt to move the window
            if hidden {
                data.pending_position = true;
            } else {
                data.pending_operation |= X11_PENDING_OP_MOVE;
                // SAFETY: the display is open and the window ours.
                unsafe {
                    (self.x.XMoveWindow)(display, data.xwindow, data.expected.x, data.expected.y);
                }
            }
        });
    }

    /// Translation of `X11_SetWindowPosition()`.
    pub(crate) fn x11_set_window_position(&self, window: WindowID) -> Result<()> {
        // Sync any pending fullscreen or maximize events.
        let pending_operation = with_x11_window(window, |_, d| d.pending_operation)?;
        if pending_operation & (X11_PENDING_OP_FULLSCREEN | X11_PENDING_OP_MAXIMIZE) != 0 {
            self.x11_flush_pending_events(window);
        }

        // Set the position as pending if the window is maximized with a restore pending.
        let flags = with_window(window, |w| w.flags())?;
        if flags.contains(WindowFlags::MAXIMIZED) {
            with_x11_window(window, |_, d| {
                if d.pending_operation & X11_PENDING_OP_RESTORE != 0 {
                    d.pending_position = true;
                }
            })?;
            return Ok(());
        }

        if !flags.contains(WindowFlags::FULLSCREEN) {
            if is_popup(window) {
                x11_constrain_popup(window, true);
            }
            self.x11_update_window_position(window, false);
        } else {
            with_x11_window(window, |_, d| d.fs_repositioned = true)?;
            let _ = update_fullscreen_mode(window, FullscreenOp::Update, true);
        }
        Ok(())
    }

    /// Translation of `X11_SetWindowMinMax()`.
    pub(crate) fn x11_set_window_min_max(&self, window: WindowID, use_current: bool) {
        let display = self.display;
        let x = &self.x;
        let _ = with_x11_window(window, |w, data| {
            let mut hint_flags: c_long = 0;
            // SAFETY: the display is open and the window ours; the hints
            // are allocated by Xlib and freed here.
            unsafe {
                let sizehints = (x.XAllocSizeHints)();

                (x.XGetWMNormalHints)(display, data.xwindow, sizehints, &mut hint_flags);
                (*sizehints).flags &= !(PMinSize | PMaxSize | PAspect);

                if w.flags().contains(WindowFlags::RESIZABLE) {
                    if w.core.min_w != 0 || w.core.min_h != 0 {
                        (*sizehints).flags |= PMinSize;
                        (*sizehints).min_width = w.core.min_w;
                        (*sizehints).min_height = w.core.min_h;
                    }
                    if w.core.max_w != 0 || w.core.max_h != 0 {
                        (*sizehints).flags |= PMaxSize;
                        (*sizehints).max_width = w.core.max_w;
                        (*sizehints).max_height = w.core.max_h;
                    }
                    if w.min_aspect > 0.0 || w.max_aspect > 0.0 {
                        (*sizehints).flags |= PAspect;
                        let (mx, my) = crate::utils::approximate_fraction(w.min_aspect);
                        (*sizehints).min_aspect = XAspect { x: mx, y: my };
                        let (mx, my) = crate::utils::approximate_fraction(w.max_aspect);
                        (*sizehints).max_aspect = XAspect { x: mx, y: my };
                    }
                } else {
                    // Set the min/max to the same values to make the window non-resizable
                    (*sizehints).flags |= PMinSize | PMaxSize;
                    let base = if use_current {
                        w.core.floating
                    } else {
                        w.core.windowed
                    };
                    let width = if w.core.last_size_pending {
                        w.pending.w
                    } else {
                        base.w
                    };
                    let height = if w.core.last_size_pending {
                        w.pending.h
                    } else {
                        base.h
                    };
                    (*sizehints).min_width = width;
                    (*sizehints).max_width = width;
                    (*sizehints).min_height = height;
                    (*sizehints).max_height = height;
                }

                (x.XSetWMNormalHints)(display, data.xwindow, sizehints);
                (x.XFree)(sizehints.cast());
            }
        });
    }

    /// The `X11_SetWindowMinimumSize()`, `X11_SetWindowMaximumSize()` and
    /// `X11_SetWindowAspectRatio()` body.
    fn x11_update_size_limits(&self, window: WindowID) {
        let pending_operation = with_x11_window(window, |_, d| d.pending_operation).unwrap_or(0);
        if pending_operation & X11_PENDING_OP_FULLSCREEN != 0 {
            let _ = self.x11_sync_window(window);
        }

        if !with_window(window, |w| w.flags().contains(WindowFlags::FULLSCREEN)).unwrap_or(true) {
            self.x11_set_window_min_max(window, true);
        }
    }

    /// Translation of `X11_SetWindowMinimumSize()`.
    pub(crate) fn x11_set_window_minimum_size(&self, window: WindowID) {
        self.x11_update_size_limits(window);
    }

    /// Translation of `X11_SetWindowMaximumSize()`.
    pub(crate) fn x11_set_window_maximum_size(&self, window: WindowID) {
        self.x11_update_size_limits(window);
    }

    /// Translation of `X11_SetWindowAspectRatio()`.
    pub(crate) fn x11_set_window_aspect_ratio(&self, window: WindowID) {
        self.x11_update_size_limits(window);
    }

    /// Translation of `X11_SetWindowSize()`.
    pub(crate) fn x11_set_window_size(&self, window: WindowID) {
        let display = self.display;
        let x = &self.x;

        /* Wait for pending maximize and fullscreen operations to complete, as these windows
         * don't get size changes.
         */
        let Ok(pending_operation) = with_x11_window(window, |_, d| d.pending_operation) else {
            return;
        };
        if pending_operation & (X11_PENDING_OP_MAXIMIZE | X11_PENDING_OP_FULLSCREEN) != 0 {
            self.x11_flush_pending_events(window);
        }

        let Ok(flags) = with_window(window, |w| w.flags()) else {
            return;
        };

        // Set the size as pending if the window is being restored.
        if flags.intersects(WindowFlags::MAXIMIZED | WindowFlags::FULLSCREEN) {
            // New size will be set when the window is restored.
            let _ = with_x11_window(window, |w, data| {
                if data.pending_operation & X11_PENDING_OP_RESTORE != 0 {
                    data.pending_size = true;
                } else {
                    // Can't resize the window.
                    w.core.last_size_pending = false;
                }
            });
            return;
        }

        if !flags.contains(WindowFlags::RESIZABLE) {
            if !flags.contains(WindowFlags::FULLSCREEN) {
                /* Apparently, if the X11 Window is set to a 'non-resizable' window, you cannot resize it using the X11_XResizeWindow, thus
                 * we must set the size hints to adjust the window size.
                 */
                let Ok((xwindow, pending, border_left, border_top, xy)) =
                    with_x11_window(window, |w, data| {
                        let mut userhints: c_long = 0;
                        // SAFETY: the display is open and the window ours; the
                        // hints are allocated by Xlib and freed here.
                        unsafe {
                            let sizehints = (x.XAllocSizeHints)();

                            (x.XGetWMNormalHints)(display, data.xwindow, sizehints, &mut userhints);

                            data.expected.w = w.pending.w;
                            (*sizehints).min_width = w.pending.w;
                            (*sizehints).max_width = w.pending.w;
                            data.expected.h = w.pending.h;
                            (*sizehints).min_height = w.pending.h;
                            (*sizehints).max_height = w.pending.h;
                            (*sizehints).flags |= PMinSize | PMaxSize;
                            data.pending_operation |= X11_PENDING_OP_RESIZE;

                            (x.XSetWMNormalHints)(display, data.xwindow, sizehints);
                            (x.XFree)(sizehints.cast());
                        }
                        let xy = if w.core.last_position_pending {
                            (w.pending.x, w.pending.y)
                        } else {
                            (w.core.x, w.core.y)
                        };
                        (
                            data.xwindow,
                            w.pending,
                            data.border_left,
                            data.border_top,
                            xy,
                        )
                    })
                else {
                    return;
                };

                /* From Pierre-Loup:
                  WMs each have their little quirks with that.  When you change the
                  size hints, they get a ConfigureNotify event with the
                  WM_NORMAL_SIZE_HINTS Atom.  They all save the hints then, but they
                  don't all resize the window right away to enforce the new hints.

                  Some of them resize only after:
                   - A user-initiated move or resize
                   - A code-initiated move or resize
                   - Hiding & showing window (Unmap & map)

                  The following move & resize seems to help a lot of WMs that didn't
                  properly update after the hints were changed. We don't do a
                  hide/show, because there are supposedly subtle problems with doing so
                  and transitioning from windowed to fullscreen in Unity.
                */
                let (dest_x, dest_y) =
                    relative_to_global_for_window(window, xy.0 - border_left, xy.1 - border_top);
                // SAFETY: the display is open and the window ours.
                unsafe {
                    (x.XResizeWindow)(display, xwindow, pending.w as u32, pending.h as u32);
                    (x.XMoveWindow)(display, xwindow, dest_x, dest_y);
                    (x.XRaiseWindow)(display, xwindow);
                }
            }
        } else {
            let _ = with_x11_window(window, |w, data| {
                data.expected.w = w.pending.w;
                data.expected.h = w.pending.h;
                data.pending_operation |= X11_PENDING_OP_RESIZE;
                // SAFETY: the display is open and the window ours.
                unsafe {
                    (x.XResizeWindow)(
                        display,
                        data.xwindow,
                        data.expected.w as u32,
                        data.expected.h as u32,
                    );
                }
            });
        }

        /* External windows may call this to update the renderer size, but not pump SDL events,
         * so the size event needs to be synthesized for external windows. If it is wrong, the
         * true size will be sent if/when events are processed.
         */
        if flags.contains(WindowFlags::EXTERNAL) {
            if let Ok(pending) = with_window(window, |w| w.pending) {
                send_window_event(window, EventType::WINDOW_RESIZED, pending.w, pending.h);
            }
        }
    }

    /// Translation of `X11_GetWindowBordersSize()`: (top, left, bottom, right).
    pub(crate) fn x11_get_window_borders_size(
        &self,
        window: WindowID,
    ) -> Result<(i32, i32, i32, i32)> {
        with_x11_window(window, |_, data| {
            (
                data.border_top,
                data.border_left,
                data.border_bottom,
                data.border_right,
            )
        })
    }

    /// Translation of `X11_SetWindowOpacity()`.
    pub(crate) fn x11_set_window_opacity(&self, window: WindowID, opacity: f32) -> Result<()> {
        let display = self.display;
        let _NET_WM_WINDOW_OPACITY = self.atoms()._NET_WM_WINDOW_OPACITY;
        let xwindow = with_x11_window(window, |_, d| d.xwindow)?;

        // SAFETY: the display is open and the window ours; format-32 data
        // as longs.
        unsafe {
            if opacity == 1.0 {
                (self.x.XDeleteProperty)(display, xwindow, _NET_WM_WINDOW_OPACITY);
            } else {
                const FULLY_OPAQUE: u32 = 0xFFFFFFFF;
                let alpha = [(opacity as f64 * FULLY_OPAQUE as f64) as c_long];
                (self.x.XChangeProperty)(
                    display,
                    xwindow,
                    _NET_WM_WINDOW_OPACITY,
                    XA_CARDINAL,
                    32,
                    PropModeReplace,
                    as_prop(&alpha),
                    1,
                );
            }
        }

        Ok(())
    }

    /// Translation of `X11_SetWindowParent()`.
    pub(crate) fn x11_set_window_parent(
        &self,
        window: WindowID,
        parent: Option<WindowID>,
    ) -> Result<()> {
        let xwindow = with_x11_window(window, |_, d| d.xwindow)?;
        let parent_xwindow = parent.and_then(|p| with_x11_window(p, |_, d| d.xwindow).ok());
        let display = self.display;

        // SAFETY: the display is open and the windows ours.
        unsafe {
            if let Some(parent_xwindow) = parent_xwindow {
                (self.x.XSetTransientForHint)(display, xwindow, parent_xwindow);
            } else {
                (self.x.XDeleteProperty)(display, xwindow, self.atoms().WM_TRANSIENT_FOR);
            }
        }

        Ok(())
    }

    /// Translation of `X11_SetWindowModal()`.
    pub(crate) fn x11_set_window_modal(&self, window: WindowID, modal: bool) -> Result<()> {
        let displaydata = display_driver_data_for_window(window);
        let display = self.display;
        let atoms = self.atoms();
        let (xwindow, mut flags) = with_x11_window(window, |w, d| (d.xwindow, w.flags()))?;

        if modal {
            flags |= WindowFlags::MODAL;
        } else {
            flags &= !WindowFlags::MODAL;
            // SAFETY: the display is open and the window ours.
            unsafe {
                (self.x.XDeleteProperty)(display, xwindow, atoms.WM_TRANSIENT_FOR);
            }
        }

        if self.x11_is_window_mapped(xwindow) {
            let mut e = client_message(xwindow, atoms._NET_WM_STATE, 32);
            let c = e.client_mut();
            c.data.l[0] = if modal {
                _NET_WM_STATE_ADD
            } else {
                _NET_WM_STATE_REMOVE
            };
            c.data.l[1] = atoms._NET_WM_STATE_MODAL as c_long;
            c.data.l[3] = 0;

            // SAFETY: the display is open; the event is a valid client message.
            unsafe {
                (self.x.XSendEvent)(
                    display,
                    RootWindow(display, screen_of(&displaydata)),
                    0,
                    SubstructureNotifyMask | SubstructureRedirectMask,
                    &mut e,
                );
            }
        } else {
            self.x11_set_net_wm_state(xwindow, flags);
        }

        // SAFETY: the display is open.
        unsafe {
            (self.x.XFlush)(display);
        }

        Ok(())
    }

    /// Translation of `X11_SetWindowBordered()`.
    pub(crate) fn x11_set_window_bordered(&self, window: WindowID, bordered: bool) {
        let Ok((focused, visible, pending_operation)) = with_x11_window(window, |w, d| {
            (
                w.flags().contains(WindowFlags::INPUT_FOCUS),
                !w.flags().contains(WindowFlags::HIDDEN) && !w.is_hiding,
                d.pending_operation,
            )
        }) else {
            return;
        };
        let displaydata = display_driver_data_for_window(window);
        let display = self.display;
        let x = &self.x;

        if pending_operation & X11_PENDING_OP_FULLSCREEN != 0 {
            let _ = self.x11_sync_window(window);
        }

        // If the window is fullscreen, the resize capability will be set/cleared when it is returned to windowed mode.
        let Ok((fullscreen, xwindow)) = with_x11_window(window, |w, d| {
            (w.flags().contains(WindowFlags::FULLSCREEN), d.xwindow)
        }) else {
            return;
        };
        if !fullscreen {
            set_window_bordered(x, display, screen_of(&displaydata), xwindow, bordered);
            // SAFETY: the display is open.
            unsafe {
                (x.XFlush)(display);
            }

            if visible {
                // SAFETY: the display is open and the window ours.
                unsafe {
                    let mut attr: XWindowAttributes = std::mem::zeroed();
                    loop {
                        (x.XSync)(display, False);
                        (x.XGetWindowAttributes)(display, xwindow, &mut attr);
                        if attr.map_state == IsViewable {
                            break;
                        }
                    }

                    if focused {
                        (x.XSetInputFocus)(display, xwindow, RevertToParent, CurrentTime);
                    }
                }
            }

            // make sure these don't make it to the real event queue if they fired here.
            let mut event = XEvent::zeroed();
            let mut w = xwindow;
            // SAFETY: the display is open; the predicates read `w`.
            unsafe {
                (x.XSync)(display, False);
                (x.XCheckIfEvent)(
                    display,
                    &mut event,
                    Some(is_unmap_notify),
                    &mut w as *mut Window as XPointer,
                );
                (x.XCheckIfEvent)(
                    display,
                    &mut event,
                    Some(is_map_notify),
                    &mut w as *mut Window as XPointer,
                );
            }

            // Turning the borders off doesn't send an extent event, so they must be cleared here.
            self.x11_get_border_values(window);

            // Make sure the window manager didn't resize our window for the difference.
            if let Ok(floating) = with_window(window, |w| w.core.floating) {
                // SAFETY: the display is open and the window ours.
                unsafe {
                    (x.XResizeWindow)(display, xwindow, floating.w as u32, floating.h as u32);
                    (x.XSync)(display, False);
                }
            }
        } else {
            // If fullscreen, set a flag to toggle the borders when returning to windowed mode.
            let _ = with_x11_window(window, |_, d| {
                d.toggle_borders = true;
                d.fullscreen_borders_forced_on = false;
            });
        }
    }

    /// Translation of `X11_SetWindowResizable()`.
    pub(crate) fn x11_set_window_resizable(&self, window: WindowID, _resizable: bool) {
        self.x11_update_size_limits(window);
    }

    /// Translation of `X11_SetWindowAlwaysOnTop()`.
    pub(crate) fn x11_set_window_always_on_top(&self, window: WindowID, on_top: bool) {
        let displaydata = display_driver_data_for_window(window);
        let display = self.display;
        let atoms = self.atoms();
        let Ok((xwindow, flags)) = with_x11_window(window, |w, d| (d.xwindow, w.flags())) else {
            return;
        };

        if self.x11_is_window_mapped(xwindow) {
            let mut e = client_message(xwindow, atoms._NET_WM_STATE, 32);
            let c = e.client_mut();
            c.data.l[0] = if on_top {
                _NET_WM_STATE_ADD
            } else {
                _NET_WM_STATE_REMOVE
            };
            c.data.l[1] = atoms._NET_WM_STATE_ABOVE as c_long;
            c.data.l[3] = 0;

            // SAFETY: the display is open; the event is a valid client message.
            unsafe {
                (self.x.XSendEvent)(
                    display,
                    RootWindow(display, screen_of(&displaydata)),
                    0,
                    SubstructureNotifyMask | SubstructureRedirectMask,
                    &mut e,
                );
            }
        } else {
            self.x11_set_net_wm_state(xwindow, flags);
        }
        // SAFETY: the display is open.
        unsafe {
            (self.x.XFlush)(display);
        }
    }

    /// Translation of `X11_ShowWindow()`.
    pub(crate) fn x11_show_window(&self, window: WindowID) {
        let display = self.display;
        let x = &self.x;
        let b_activate = hints::get_bool(hints::WINDOW_ACTIVATE_WHEN_SHOWN, true);
        let mut set_position = false;
        let mut event = XEvent::zeroed();

        let Ok((was_shown, xwindow)) = with_x11_window(window, |_, d| (d.was_shown, d.xwindow))
        else {
            return;
        };

        // If the window was previously shown, pump events to avoid possible positioning issues.
        if was_shown {
            self.x11_pump_events();
        }

        if is_popup(window) {
            // Update the position in case the parent moved while we were hidden
            x11_constrain_popup(window, true);
            let _ = with_x11_window(window, |_, d| d.pending_position = true);
            set_position = true;
        }

        /* Whether XMapRaised focuses the window is based on the window type and it is
         * wm specific. There isn't much we can do here */
        let _ = b_activate;

        if !self.x11_is_window_mapped(xwindow) {
            let mut w = xwindow;
            // SAFETY: the display is open and the window ours.
            unsafe {
                (x.XMapRaised)(display, xwindow);
            }
            /* Blocking wait for "MapNotify" event.
             * We use X11_XIfEvent because pXWindowEvent takes a mask rather than a type,
             * and XCheckTypedWindowEvent doesn't block */
            let external =
                with_window(window, |w| w.flags().contains(WindowFlags::EXTERNAL)).unwrap_or(true);
            if !external && x11_is_display_ok(display) {
                // SAFETY: the predicate reads `w`; the event is an out-parameter.
                unsafe {
                    (x.XIfEvent)(
                        display,
                        &mut event,
                        Some(is_map_notify),
                        &mut w as *mut Window as XPointer,
                    );
                }
            }
            // SAFETY: the display is open.
            unsafe {
                (x.XFlush)(display);
            }
            set_position = with_x11_window(window, |win, d| {
                d.pending_position
                    || (!win.flags().contains(WindowFlags::BORDERLESS)
                        && !win.core.undefined_x
                        && !win.core.undefined_y)
            })
            .unwrap_or(false);
        }

        if !self.with_data(|d| d.net_wm) {
            // no WM means no FocusIn event, which confuses us. Force it.
            // SAFETY: the display is open and the window ours.
            unsafe {
                (x.XSync)(display, False);
                (x.XSetInputFocus)(display, xwindow, RevertToNone, CurrentTime);
                (x.XFlush)(display);
            }
        }

        // Grabbing popup menus get keyboard focus.
        let flags = with_window(window, |w| w.flags()).unwrap_or_default();
        if flags.contains(WindowFlags::POPUP_MENU) && !flags.contains(WindowFlags::NOT_FOCUSABLE) {
            x11_set_keyboard_focus(Some(window), true);
        }

        // Get some valid border values, if we haven't received them yet
        let no_borders = with_x11_window(window, |_, d| {
            d.border_left == 0 && d.border_right == 0 && d.border_top == 0 && d.border_bottom == 0
        })
        .unwrap_or(false);
        if no_borders {
            self.x11_get_border_values(window);
        }

        if set_position {
            // Apply the window position, accounting for offsets due to the borders appearing, but only when initially mapping.
            if let Ok((tx, ty)) = with_x11_window(window, |w, d| {
                let tx = (if d.pending_position {
                    w.pending.x
                } else {
                    w.core.x
                }) - (if d.was_shown { 0 } else { d.border_left });
                let ty = (if d.pending_position {
                    w.pending.y
                } else {
                    w.core.y
                }) - (if d.was_shown { 0 } else { d.border_top });
                (tx, ty)
            }) {
                let (wx, wy) = relative_to_global_for_window(window, tx, ty);

                let _ = with_x11_window(window, |_, d| d.pending_position = false);
                // SAFETY: the display is open and the window ours.
                unsafe {
                    (x.XMoveWindow)(display, xwindow, wx, wy);
                }
            }
        }

        /* XMonad ignores size hints and shrinks the client area to overlay borders on fixed-size windows,
         * even if no borders were requested, resulting in the window client area being smaller than
         * requested. Calling XResizeWindow after mapping seems to fix it, even though resizing fixed-size
         * windows in this manner doesn't work on any other window manager.
         */
        if !flags.contains(WindowFlags::RESIZABLE) && x11_check_current_desktop("xmonad") {
            if let Ok((w, h)) = with_window(window, |w| (w.core.w, w.core.h)) {
                // SAFETY: the display is open and the window ours.
                unsafe {
                    (x.XResizeWindow)(display, xwindow, w as u32, h as u32);
                }
            }
        }

        /* Some window managers can send garbage coordinates while mapping the window, so don't emit size and position
         * events during the initial configure events.
         */
        let _ = with_x11_window(window, |_, d| {
            d.size_move_event_flags = X11_SIZE_MOVE_EVENTS_DISABLE
        });
        // SAFETY: the display is open.
        unsafe {
            (x.XSync)(display, False);
        }
        self.x11_pump_events();
        let _ = with_x11_window(window, |_, d| d.size_move_event_flags = 0);

        /* A MapNotify or PropertyNotify may not have arrived, so ensure that the shown event is dispatched
         * to apply pending state before clearing the flag.
         */
        send_window_event(window, EventType::WINDOW_SHOWN, 0, 0);
        let _ = with_x11_window(window, |_, d| d.was_shown = true);

        // If a configure event was received (type is non-zero), send the final window size and coordinates.
        if let Ok(last) = with_x11_window(window, |_, d| d.last_xconfigure) {
            if last.type_ != 0 {
                let (lx, ly) = global_to_relative_for_window(window, last.x, last.y);
                send_window_event(window, EventType::WINDOW_RESIZED, last.width, last.height);
                send_window_event(window, EventType::WINDOW_MOVED, lx, ly);
            }
        }
    }

    /// Translation of `X11_HideWindow()`.
    pub(crate) fn x11_hide_window(&self, window: WindowID) {
        let displaydata = display_driver_data_for_window(window);
        let screen = screen_of(&displaydata);
        let display = self.display;
        let x = &self.x;
        let mut event = XEvent::zeroed();
        let Ok(xwindow) = with_x11_window(window, |_, d| d.xwindow) else {
            return;
        };

        /* Some window managers will mark minimized or offscreen windows as unmapped, so we
         * must not block while waiting for an UnmapNotify event that will never arrive.
         */
        let is_mapped = self.x11_is_window_mapped(xwindow);
        let flags = with_window(window, |w| w.flags()).unwrap_or_default();
        let mut w = xwindow;
        // SAFETY: the display is open and the window ours; the predicate
        // reads `w`.
        unsafe {
            (x.XWithdrawWindow)(display, xwindow, screen);
            if is_mapped && !flags.contains(WindowFlags::EXTERNAL) && x11_is_display_ok(display) {
                // Blocking wait for "UnmapNotify" event.
                (x.XIfEvent)(
                    display,
                    &mut event,
                    Some(is_unmap_notify),
                    &mut w as *mut Window as XPointer,
                );
            }
            (x.XFlush)(display);
        }

        // Transfer keyboard focus back to the parent
        if flags.contains(WindowFlags::POPUP_MENU) && !flags.contains(WindowFlags::NOT_FOCUSABLE) {
            if let Ok((new_focus, set_focus)) = should_relinquish_popup_focus(window) {
                x11_set_keyboard_focus(new_focus, set_focus);
            }
        }

        // SAFETY: the display is open.
        unsafe {
            (x.XSync)(display, False);
        }
        self.x11_pump_events();
    }

    /// Translation of `X11_SetWindowActive()`.
    fn x11_set_window_active(&self, window: WindowID) -> Result<()> {
        let (xwindow, user_time) = with_x11_window(window, |_, d| (d.xwindow, d.user_time))?;
        let displaydata = display_driver_data_for_window(window);
        let display = self.display;
        let _NET_ACTIVE_WINDOW = self.atoms()._NET_ACTIVE_WINDOW;

        if self.x11_is_window_mapped(xwindow) {
            // printf("SDL Window %p: sending _NET_ACTIVE_WINDOW with timestamp %lu\n", window, data->user_time);

            let mut e = client_message(xwindow, _NET_ACTIVE_WINDOW, 32);
            let c = e.client_mut();
            c.data.l[0] = 1; // source indication. 1 = application
            c.data.l[1] = user_time as c_long;
            c.data.l[2] = 0;

            // SAFETY: the display is open; the event is a valid client message.
            unsafe {
                (self.x.XSendEvent)(
                    display,
                    RootWindow(display, screen_of(&displaydata)),
                    0,
                    SubstructureNotifyMask | SubstructureRedirectMask,
                    &mut e,
                );

                (self.x.XFlush)(display);
            }
        }
        Ok(())
    }

    /// Translation of `X11_RaiseWindow()`.
    pub(crate) fn x11_raise_window(&self, window: WindowID) {
        let display = self.display;
        let b_activate = hints::get_bool(hints::WINDOW_ACTIVATE_WHEN_RAISED, true);
        let Ok(xwindow) = with_x11_window(window, |_, d| d.xwindow) else {
            return;
        };

        // SAFETY: the display is open and the window ours.
        unsafe {
            (self.x.XRaiseWindow)(display, xwindow);
        }
        if b_activate {
            let _ = self.x11_set_window_active(window);
        }
        // SAFETY: the display is open.
        unsafe {
            (self.x.XFlush)(display);
        }
    }

    /// Translation of `X11_SetWindowMaximized()`.
    fn x11_set_window_maximized(&self, window: WindowID, maximized: bool) -> Result<()> {
        let (xwindow, flags) = with_x11_window(window, |w, d| (d.xwindow, w.flags()))?;
        let displaydata = display_driver_data_for_window(window);
        let display = self.display;
        let atoms = self.atoms();

        if flags.contains(WindowFlags::FULLSCREEN) {
            /* Fullscreen windows are maximized on some window managers,
              and this is functional behavior, so don't remove that state
              now, we'll take care of it when we leave fullscreen mode.
            */
            return Ok(());
        }

        if self.x11_is_window_mapped(xwindow) {
            let mut e = client_message(xwindow, atoms._NET_WM_STATE, 32);
            let c = e.client_mut();
            c.data.l[0] = if maximized {
                _NET_WM_STATE_ADD
            } else {
                _NET_WM_STATE_REMOVE
            };
            c.data.l[1] = atoms._NET_WM_STATE_MAXIMIZED_VERT as c_long;
            c.data.l[2] = atoms._NET_WM_STATE_MAXIMIZED_HORZ as c_long;
            c.data.l[3] = 0;

            if maximized {
                let display_id = crate::video::window::display_for_window(window).unwrap_or(0);
                let bounds = display_usable_bounds(display_id).unwrap_or_default();

                with_x11_window(window, |_, data| {
                    data.expected.x = bounds.x + data.border_left;
                    data.expected.y = bounds.y + data.border_top;
                    data.expected.w = bounds.w - (data.border_left + data.border_right);
                    data.expected.h = bounds.h - (data.border_top + data.border_bottom);
                })?;
            } else {
                with_x11_window(window, |w, data| {
                    data.expected.x = w.core.floating.x;
                    data.expected.y = w.core.floating.y;
                    data.expected.w = w.core.floating.w;
                    data.expected.h = w.core.floating.h;
                })?;
            }

            // SAFETY: the display is open; the event is a valid client message.
            unsafe {
                (self.x.XSendEvent)(
                    display,
                    RootWindow(display, screen_of(&displaydata)),
                    0,
                    SubstructureNotifyMask | SubstructureRedirectMask,
                    &mut e,
                );
            }
        } else {
            self.x11_set_net_wm_state(xwindow, flags);
        }
        // SAFETY: the display is open.
        unsafe {
            (self.x.XFlush)(display);
        }

        Ok(())
    }

    /// Translation of `X11_MaximizeWindow()`.
    pub(crate) fn x11_maximize_window(&self, window: WindowID) {
        let Ok(pending_operation) = with_x11_window(window, |_, d| d.pending_operation) else {
            return;
        };
        if pending_operation & (X11_PENDING_OP_FULLSCREEN | X11_PENDING_OP_MINIMIZE) != 0 {
            let _ = crate::video::window::sync_window(window);
        }

        let flags = with_window(window, |w| w.flags()).unwrap_or_default();
        if flags.contains(WindowFlags::FULLSCREEN) {
            // If fullscreen, just toggle the restored state.
            let _ = with_x11_window(window, |_, d| d.window_was_maximized = true);
            return;
        }

        if !flags.contains(WindowFlags::MINIMIZED) {
            let _ = with_x11_window(window, |_, d| {
                d.pending_operation |= X11_PENDING_OP_MAXIMIZE
            });
            let _ = self.x11_set_window_maximized(window, true);
        }
    }

    /// Translation of `X11_MinimizeWindow()`.
    pub(crate) fn x11_minimize_window(&self, window: WindowID) {
        let displaydata = display_driver_data_for_window(window);
        let display = self.display;

        let Ok(pending_operation) = with_x11_window(window, |_, d| d.pending_operation) else {
            return;
        };
        // FIXME (upstream): pending_operation is tested against the window
        // flag SDL_WINDOW_FULLSCREEN (0x1, which is X11_PENDING_OP_RESTORE)
        // instead of X11_PENDING_OP_FULLSCREEN.
        if pending_operation & (WindowFlags::FULLSCREEN.0 as u32) != 0 {
            let _ = crate::video::window::sync_window(window);
        }

        let _ = with_x11_window(window, |w, data| {
            data.pending_operation |= X11_PENDING_OP_MINIMIZE;
            if !w.flags().contains(WindowFlags::FULLSCREEN) {
                data.window_was_maximized = w.flags().contains(WindowFlags::MAXIMIZED);
            }
            // SAFETY: the display is open and the window ours.
            unsafe {
                (self.x.XIconifyWindow)(display, data.xwindow, screen_of(&displaydata));
                (self.x.XFlush)(display);
            }
        });
    }

    /// Translation of `X11_RestoreWindow()`.
    pub(crate) fn x11_restore_window(&self, window: WindowID) {
        // Don't restore the window the first time it is being shown.
        let Ok((was_shown, pending_operation)) =
            with_x11_window(window, |_, d| (d.was_shown, d.pending_operation))
        else {
            return;
        };
        if !was_shown {
            return;
        }

        if pending_operation
            & (X11_PENDING_OP_FULLSCREEN | X11_PENDING_OP_MAXIMIZE | X11_PENDING_OP_MINIMIZE)
            != 0
        {
            let _ = crate::video::window::sync_window(window);
        }

        let flags = with_window(window, |w| w.flags()).unwrap_or_default();
        if flags.contains(WindowFlags::FULLSCREEN) && !flags.contains(WindowFlags::MINIMIZED) {
            // If fullscreen and not minimized, just toggle the restored state.
            let _ = with_x11_window(window, |_, d| d.window_was_maximized = false);
            return;
        }

        let maximize = with_x11_window(window, |_, d| {
            if flags.intersects(WindowFlags::MINIMIZED | WindowFlags::MAXIMIZED)
                || (d.pending_operation & X11_PENDING_OP_MINIMIZE) != 0
            {
                d.pending_operation |= X11_PENDING_OP_RESTORE;
            }

            // If the window was minimized while maximized, restore as maximized.
            flags.contains(WindowFlags::MINIMIZED) && d.window_was_maximized
        })
        .unwrap_or(false);
        let _ = self.x11_set_window_maximized(window, maximize);
        self.x11_show_window(window);
        let _ = self.x11_set_window_active(window);
    }

    /// This asks the Window Manager to handle fullscreen for us. This is the modern way.
    /// Translation of `X11_SetWindowFullscreenViaWM()`.
    fn x11_set_window_fullscreen_via_wm(
        &self,
        window: WindowID,
        sdl_display: DisplayID,
        fullscreen: FullscreenOp,
    ) -> FullscreenResult {
        let Some(displaydata) = super::modes::display_driver_data(sdl_display) else {
            return FullscreenResult::Failed;
        };
        let display = self.display;
        let x = &self.x;
        let atoms = self.atoms();
        let Ok((was_shown, xwindow)) = with_x11_window(window, |_, d| (d.was_shown, d.xwindow))
        else {
            return FullscreenResult::Failed;
        };
        let entering = fullscreen != FullscreenOp::Leave;

        if !was_shown && fullscreen == FullscreenOp::Leave {
            return FullscreenResult::Succeeded;
        }

        if self.x11_is_window_mapped(xwindow) {
            // Flush any pending fullscreen events.
            let pending_operation =
                with_x11_window(window, |_, d| d.pending_operation).unwrap_or(0);
            if pending_operation
                & (X11_PENDING_OP_FULLSCREEN | X11_PENDING_OP_MAXIMIZE | X11_PENDING_OP_MOVE)
                != 0
            {
                let _ = self.x11_sync_window(window);
            }

            let flags = with_window(window, |w| w.flags()).unwrap_or_default();
            if !flags.contains(WindowFlags::FULLSCREEN) {
                if fullscreen == FullscreenOp::Update {
                    // Request was out of date; set -1 to signal the video core to undo a mode switch.
                    return FullscreenResult::Failed;
                } else if fullscreen == FullscreenOp::Leave {
                    // Nothing to do.
                    return FullscreenResult::Succeeded;
                }
            }

            if entering && !flags.contains(WindowFlags::RESIZABLE) {
                /* Compiz refuses fullscreen toggle if we're not resizable, so update the hints so we
                can be resized to the fullscreen resolution (or reset so we're not resizable again) */
                let mut hint_flags: c_long = 0;
                // SAFETY: the display is open and the window ours; the hints
                // are allocated by Xlib and freed here.
                unsafe {
                    let sizehints = (x.XAllocSizeHints)();
                    (x.XGetWMNormalHints)(display, xwindow, sizehints, &mut hint_flags);
                    // we are going fullscreen so turn the flags off
                    (*sizehints).flags &= !(PMinSize | PMaxSize | PAspect);
                    (x.XSetWMNormalHints)(display, xwindow, sizehints);
                    (x.XFree)(sizehints.cast());
                }
            }

            let mut e = client_message(xwindow, atoms._NET_WM_STATE, 32);
            let c = e.client_mut();
            c.data.l[0] = if entering {
                _NET_WM_STATE_ADD
            } else {
                _NET_WM_STATE_REMOVE
            };
            c.data.l[1] = atoms._NET_WM_STATE_FULLSCREEN as c_long;
            c.data.l[3] = 0;

            // SAFETY: the display is open; the event is a valid client message.
            unsafe {
                (x.XSendEvent)(
                    display,
                    RootWindow(display, displaydata.screen),
                    0,
                    SubstructureNotifyMask | SubstructureRedirectMask,
                    &mut e,
                );
            }

            // Only set the pending flag if the fullscreen state actually changed.
            if flags.contains(WindowFlags::FULLSCREEN) != entering {
                let _ = with_x11_window(window, |_, d| {
                    d.pending_operation |= X11_PENDING_OP_FULLSCREEN
                });
            }

            // Set the position so the window will be on the target display
            if entering {
                let current = crate::video::core::display_for_window_position(window).unwrap_or(0);
                let current_mode =
                    crate::video::display::current_display_mode(sdl_display).unwrap_or_default();
                let mut moved = false;
                let _ = with_x11_window(window, |w, data| {
                    data.requested_fullscreen_mode = w.current_fullscreen_mode;
                    if entering != w.flags().contains(WindowFlags::FULLSCREEN) {
                        data.window_was_maximized = w.flags().contains(WindowFlags::MAXIMIZED);
                    }
                    data.expected.x = displaydata.x;
                    data.expected.y = displaydata.y;
                    data.expected.w = current_mode.w;
                    data.expected.h = current_mode.h;

                    // Only move the window if it isn't already on the target display.
                    if current == 0 || current != sdl_display {
                        moved = true;
                        data.pending_operation |= X11_PENDING_OP_MOVE;
                    }
                });
                if moved {
                    // SAFETY: the display is open and the window ours.
                    unsafe {
                        (x.XMoveWindow)(display, xwindow, displaydata.x, displaydata.y);
                    }
                }
            } else {
                let window_was_maximized = with_x11_window(window, |_, data| {
                    data.requested_fullscreen_mode = DisplayMode::default();
                    data.pending_position = data.fs_repositioned;
                    data.fs_repositioned = false;
                    data.window_was_maximized
                })
                .unwrap_or(false);

                /* Fullscreen windows sometimes end up being marked maximized by
                 * window managers. Force it back to how we expect it to be.
                 */
                let mut e = client_message(xwindow, atoms._NET_WM_STATE, 32);
                let c = e.client_mut();
                if window_was_maximized {
                    c.data.l[0] = _NET_WM_STATE_ADD;
                } else {
                    c.data.l[0] = _NET_WM_STATE_REMOVE;
                }
                c.data.l[1] = atoms._NET_WM_STATE_MAXIMIZED_VERT as c_long;
                c.data.l[2] = atoms._NET_WM_STATE_MAXIMIZED_HORZ as c_long;
                c.data.l[3] = 0;
                // SAFETY: as above.
                unsafe {
                    (x.XSendEvent)(
                        display,
                        RootWindow(display, displaydata.screen),
                        0,
                        SubstructureNotifyMask | SubstructureRedirectMask,
                        &mut e,
                    );
                }
            }
        } else {
            let mut flags = with_window(window, |w| w.flags()).unwrap_or_default();
            if entering {
                flags |= WindowFlags::FULLSCREEN;
            } else {
                flags &= !WindowFlags::FULLSCREEN;
            }
            self.x11_set_net_wm_state(xwindow, flags);
        }

        let (visual, colormap) = with_x11_window(window, |_, d| (d.visual, d.colormap))
            .unwrap_or((std::ptr::null_mut(), 0));
        // SAFETY: the window's visual is a visual of the display.
        if !visual.is_null() && unsafe { (*visual).class } == DirectColor {
            // SAFETY: the display is open; the colormap is the window's.
            unsafe {
                if entering {
                    (x.XInstallColormap)(display, colormap);
                } else {
                    (x.XUninstallColormap)(display, colormap);
                }
            }
        }

        FullscreenResult::Pending
    }

    /// Translation of `X11_SetWindowFullscreenLegacy()`.
    fn x11_set_window_fullscreen_legacy(
        &self,
        window: WindowID,
        sdl_display: DisplayID,
        fullscreen: bool,
    ) -> FullscreenResult {
        let Some(displaydata) = super::modes::display_driver_data(sdl_display) else {
            return FullscreenResult::Failed;
        };
        let display = self.display;
        let x = &self.x;
        let screen = displaydata.screen;
        let Ok((was_shown, xwindow, pending_operation)) =
            with_x11_window(window, |_, d| (d.was_shown, d.xwindow, d.pending_operation))
        else {
            return FullscreenResult::Failed;
        };

        // (upstream compares the bool with SDL_FULLSCREEN_OP_LEAVE, i.e. 0)
        if !was_shown && !fullscreen {
            return FullscreenResult::Succeeded;
        }

        // Flush any pending fullscreen events.
        if pending_operation & (X11_PENDING_OP_RESIZE | X11_PENDING_OP_MOVE) != 0 {
            let _ = self.x11_sync_window(window);
        }

        // No window manager? Just make the window the size of the display.
        send_window_event(
            window,
            if fullscreen {
                EventType::WINDOW_ENTER_FULLSCREEN
            } else {
                EventType::WINDOW_LEAVE_FULLSCREEN
            },
            0,
            0,
        );
        let flags = with_window(window, |w| w.flags()).unwrap_or_default();
        if fullscreen {
            if !flags.contains(WindowFlags::RESIZABLE) {
                /* Compiz refuses fullscreen toggle if we're not resizable, so update the hints so we
                 * can be resized to the fullscreen resolution (or reset so we're not resizable again).
                 */
                let mut hint_flags: c_long = 0;
                // SAFETY: the display is open and the window ours; the hints
                // are allocated by Xlib and freed here.
                unsafe {
                    let sizehints = (x.XAllocSizeHints)();
                    (x.XGetWMNormalHints)(display, xwindow, sizehints, &mut hint_flags);
                    // we are going fullscreen so turn the flags off
                    (*sizehints).flags &= !(PMinSize | PMaxSize | PAspect);
                    (x.XSetWMNormalHints)(display, xwindow, sizehints);
                    (x.XFree)(sizehints.cast());
                }
            }

            set_window_bordered(x, display, screen, xwindow, false);

            let rect = self.x11_get_display_bounds(sdl_display).unwrap_or_default();

            let _ = with_x11_window(window, |_, data| {
                data.pending_operation |= X11_PENDING_OP_MOVE | X11_PENDING_OP_RESIZE;
                data.expected = rect;
            });
            // SAFETY: the display is open and the window ours.
            unsafe {
                (x.XMoveWindow)(display, xwindow, rect.x, rect.y);
                (x.XResizeWindow)(display, xwindow, rect.w as u32, rect.h as u32);
                (x.XRaiseWindow)(display, xwindow);
            }
        } else {
            let windowed = with_window(window, |w| w.core.windowed).unwrap_or_default();
            let _ = with_x11_window(window, |_, data| {
                data.pending_operation |= X11_PENDING_OP_MOVE | X11_PENDING_OP_RESIZE;
                data.expected = windowed;
            });
            // SAFETY: the display is open and the window ours.
            unsafe {
                (x.XResizeWindow)(display, xwindow, windowed.w as u32, windowed.h as u32);
                (x.XMoveWindow)(display, xwindow, windowed.x, windowed.y);
            }

            set_window_bordered(
                x,
                display,
                screen,
                xwindow,
                !flags.contains(WindowFlags::BORDERLESS),
            );
            self.x11_set_window_min_max(window, false);
        }

        // SAFETY: the display is open.
        unsafe {
            (x.XFlush)(display);
        }

        FullscreenResult::Pending
    }

    /// Translation of `X11_SetWindowFullscreen()`.
    pub(crate) fn x11_set_window_fullscreen(
        &self,
        window: WindowID,
        sdl_display: DisplayID,
        fullscreen: FullscreenOp,
    ) -> FullscreenResult {
        // CHECK_WINDOW_DATA / CHECK_DISPLAY_DATA
        if with_x11_window(window, |_, _| ()).is_err() {
            return FullscreenResult::Failed;
        }
        if with_x11_display(sdl_display, |_, _| ()).is_none() {
            return FullscreenResult::Failed;
        }

        if self.with_data(|d| d.net_wm) {
            return self.x11_set_window_fullscreen_via_wm(window, sdl_display, fullscreen);
        }

        self.x11_set_window_fullscreen_legacy(
            window,
            sdl_display,
            fullscreen != FullscreenOp::Leave,
        )
    }

    /// Translation of `X11_GetWindowICCProfile()`.
    pub(crate) fn x11_get_window_icc_profile(&self, window: WindowID) -> Result<Vec<u8>> {
        let display = self.display;
        let x = &self.x;
        let xwindow = with_x11_window(window, |_, d| d.xwindow)?;

        // SAFETY: the display is open and the window ours; the attributes
        // are valid out-parameters; the property data has `count` bytes
        // and is freed here.
        unsafe {
            let mut attributes: XWindowAttributes = std::mem::zeroed();
            (x.XGetWindowAttributes)(display, xwindow, &mut attributes);
            let screen_number = (x.XScreenNumberOfScreen)(attributes.screen);
            let icc_atom_string = if screen_number > 0 {
                format!("_ICC_PROFILE_{screen_number}")
            } else {
                "_ICC_PROFILE".to_owned()
            };
            let root = RootWindowOfScreen(attributes.screen);
            (x.XGetWindowAttributes)(display, root, &mut attributes);

            let icc_profile_atom = intern_atom(x, display, &icc_atom_string, true);
            if icc_profile_atom == None {
                return Err(Error::new("Screen is not calibrated."));
            }

            let atom_prop = x11_read_property(
                x,
                display,
                RootWindowOfScreen(attributes.screen),
                icc_profile_atom,
            );
            let real_format = atom_prop.format;
            let real_nitems = atom_prop.count;
            let icc_profile_data = atom_prop.data;
            if real_format == None as c_int {
                // FIXME (upstream): the property data (if any) leaks here.
                return Err(Error::new("Screen is not calibrated."));
            }

            let ret = if icc_profile_data.is_null() {
                Vec::new()
            } else {
                std::slice::from_raw_parts(icc_profile_data, real_nitems as usize).to_vec()
            };
            (x.XFree)(icc_profile_data.cast());

            Ok(ret)
        }
    }

    /// Translation of `X11_SetWindowMouseGrab()`.
    pub(crate) fn x11_set_window_mouse_grab(&self, window: WindowID, grabbed: bool) -> Result<()> {
        let display = self.display;
        let x = &self.x;
        let (xwindow, xinput2_mouse_enabled, flags) = with_x11_window(window, |w, data| {
            data.mouse_grabbed = false;
            data.pending_grab = false;
            (data.xwindow, data.xinput2_mouse_enabled, w.flags())
        })
        .map_err(|_| Error::new("Invalid window data"))?;

        if grabbed {
            /* If the window is unmapped, XGrab calls return GrabNotViewable,
            so when we get a MapNotify later, we'll try to update the grab as
            appropriate. */
            if flags.contains(WindowFlags::HIDDEN) {
                return Ok(());
            }

            /* If XInput2 is enabled, it will grab the pointer on button presses,
             * which results in XGrabPointer returning AlreadyGrabbed. If buttons
             * are currently pressed, clear any existing grabs before attempting
             * the confinement grab.
             */
            if xinput2_mouse_enabled && mouse::mouse_state().2 .0 != 0 {
                with_x11_window(window, |_, d| d.pending_grab = true)?;
                return Ok(());
            }

            // Try to grab the mouse
            if !self.with_data(|d| d.broken_pointer_grab) {
                let mask =
                    (ButtonPressMask | ButtonReleaseMask | PointerMotionMask | FocusChangeMask)
                        as u32;
                let mut result = 0;

                // Try for up to 5000ms (5s) to grab. If it still fails, stop trying.
                for _attempts in 0..100 {
                    // SAFETY: the display is open and the window ours.
                    result = unsafe {
                        (x.XGrabPointer)(
                            display,
                            xwindow,
                            False,
                            mask,
                            GrabModeAsync,
                            GrabModeAsync,
                            xwindow,
                            None,
                            CurrentTime,
                        )
                    };
                    if result == GrabSuccess {
                        with_x11_window(window, |_, d| d.mouse_grabbed = true)?;
                        break;
                    }
                    crate::timer::delay(Duration::from_millis(50));
                }

                if result != GrabSuccess {
                    self.with_data(|d| d.broken_pointer_grab = true); // don't try again.
                }
            }

            self.x11_xinput2_grab_touch(window);

            // Raise the window if we grab the mouse
            // SAFETY: the display is open and the window ours.
            unsafe {
                (x.XRaiseWindow)(display, xwindow);
            }
        } else {
            // SAFETY: the display is open.
            unsafe {
                (x.XUngrabPointer)(display, CurrentTime);
            }

            self.x11_xinput2_ungrab_touch(window);
        }
        // SAFETY: the display is open.
        unsafe {
            (x.XSync)(display, False);
        }

        if !self.with_data(|d| d.broken_pointer_grab) {
            Ok(())
        } else {
            Err(Error::new(
                "The X server refused to let us grab the mouse. You might experience input bugs.",
            ))
        }
    }

    /// Translation of `X11_SetWindowKeyboardGrab()`.
    pub(crate) fn x11_set_window_keyboard_grab(
        &self,
        window: WindowID,
        grabbed: bool,
    ) -> Result<()> {
        let display = self.display;
        let x = &self.x;
        let (xwindow, flags) = with_x11_window(window, |w, d| (d.xwindow, w.flags()))
            .map_err(|_| Error::new("Invalid window data"))?;

        if grabbed {
            /* If the window is unmapped, XGrab calls return GrabNotViewable,
            so when we get a MapNotify later, we'll try to update the grab as
            appropriate. */
            if flags.contains(WindowFlags::HIDDEN) {
                return Ok(());
            }

            /* GNOME needs the _XWAYLAND_MAY_GRAB_KEYBOARD message on XWayland:
             *
             * - message_type set to "_XWAYLAND_MAY_GRAB_KEYBOARD"
             * - window set to the xid of the window on which the grab is to be issued
             * - data.l[0] to a non-zero value
             *
             * The dconf setting `org/gnome/mutter/wayland/xwayland-allow-grabs` must be enabled as well.
             *
             * https://gitlab.gnome.org/GNOME/mutter/-/commit/5f132f39750f684c3732b4346dec810cd218d609
             */
            if self.is_xwayland {
                let _XWAYLAND_MAY_GRAB_ATOM =
                    self.intern_atom("_XWAYLAND_MAY_GRAB_KEYBOARD", false);

                if _XWAYLAND_MAY_GRAB_ATOM != None {
                    // (upstream leaves the other fields of the message
                    // uninitialized; they are zero here)
                    let mut client_message = client_message(xwindow, _XWAYLAND_MAY_GRAB_ATOM, 32);
                    let c = client_message.client_mut();
                    c.data.l[0] = 1;
                    c.data.l[1] = CurrentTime as c_long;

                    // SAFETY: the display is open; the event is a valid
                    // client message.
                    unsafe {
                        (x.XSendEvent)(
                            display,
                            DefaultRootWindow(display),
                            False,
                            SubstructureNotifyMask | SubstructureRedirectMask,
                            &mut client_message,
                        );
                        (x.XFlush)(display);
                    }
                }
            }

            // SAFETY: the display is open and the window ours.
            unsafe {
                (x.XGrabKeyboard)(
                    display,
                    xwindow,
                    True,
                    GrabModeAsync,
                    GrabModeAsync,
                    CurrentTime,
                );
            }
        } else {
            // SAFETY: the display is open.
            unsafe {
                (x.XUngrabKeyboard)(display, CurrentTime);
            }
        }
        // SAFETY: the display is open.
        unsafe {
            (x.XSync)(display, False);
        }

        Ok(())
    }

    /// Translation of `X11_DestroyWindow()`.
    pub(crate) fn x11_destroy_window(&self, window: WindowID) {
        let display = self.display;
        let Ok((xwindow, external)) = with_x11_window(window, |w, d| {
            (d.xwindow, w.flags().contains(WindowFlags::EXTERNAL))
        }) else {
            let _ = with_window(window, |w| w.internal = Option::None);
            return;
        };

        self.with_data(|d| {
            if let Some(i) = d.windowlist.iter().position(|e| e.window == window) {
                // (moves the last entry into the slot, like upstream)
                d.windowlist.swap_remove(i);
            }
        });

        let _ = with_x11_window(window, |_, data| self.x11_destroy_input_context(data));

        self.x11_term_resize_sync(window);

        if !external {
            // SAFETY: the display is open and the window ours.
            unsafe {
                (self.x.XDestroyWindow)(display, xwindow);
                (self.x.XFlush)(display);
            }
        }

        // If the pointer barriers are active for this, deactivate it.
        // (upstream checks this after freeing the window data; the
        // barriers are destroyed with the data still attached here)
        if self.with_data(|d| d.active_cursor_confined_window) == Some(window) {
            self.x11_destroy_pointer_barrier(Some(window));
        }
        let _ = with_window(window, |w| w.internal = Option::None);
    }

    /// Translation of `X11_AcceptDragAndDrop()`.
    pub(crate) fn x11_accept_drag_and_drop(&self, window: WindowID, accept: bool) {
        let display = self.display;
        let xdnd_aware = self.atoms().XdndAware;
        let Ok(xwindow) = with_x11_window(window, |_, d| d.xwindow) else {
            return;
        };

        // SAFETY: the display is open and the window ours; format-32 data as
        // longs.
        unsafe {
            if accept {
                let xdnd_version = [5 as c_long];
                (self.x.XChangeProperty)(
                    display,
                    xwindow,
                    xdnd_aware,
                    XA_ATOM,
                    32,
                    PropModeReplace,
                    as_prop(&xdnd_version),
                    1,
                );
            } else {
                (self.x.XDeleteProperty)(display, xwindow, xdnd_aware);
            }
        }
    }

    /// Translation of `X11_FlashWindow()`.
    pub(crate) fn x11_flash_window(
        &self,
        window: WindowID,
        operation: FlashOperation,
    ) -> Result<()> {
        let display = self.display;
        let (xwindow, flags) = with_x11_window(window, |w, d| (d.xwindow, w.flags()))?;

        // SAFETY: the display is open and the window ours.
        let wmhints = unsafe { (self.x.XGetWMHints)(display, xwindow) };
        if wmhints.is_null() {
            return Err(Error::new("Couldn't get WM hints"));
        }

        let mut flashing_window = false;
        let mut flash_cancel_time = 0;
        // SAFETY: wmhints came from XGetWMHints.
        unsafe {
            (*wmhints).flags &= !XUrgencyHint;
        }

        match operation {
            FlashOperation::Cancel => {
                // Taken care of above
            }
            FlashOperation::Briefly => {
                if !flags.contains(WindowFlags::INPUT_FOCUS) {
                    // SAFETY: as above.
                    unsafe {
                        (*wmhints).flags |= XUrgencyHint;
                    }
                    flashing_window = true;
                    // On Ubuntu 21.04 this causes a dialog to pop up, so leave it up for a full second so users can see it
                    flash_cancel_time = crate::timer::ticks_ms() + 1000;
                }
            }
            FlashOperation::UntilFocused => {
                if !flags.contains(WindowFlags::INPUT_FOCUS) {
                    // SAFETY: as above.
                    unsafe {
                        (*wmhints).flags |= XUrgencyHint;
                    }
                    flashing_window = true;
                }
            }
        }
        with_x11_window(window, |_, d| {
            d.flashing_window = flashing_window;
            d.flash_cancel_time = flash_cancel_time;
        })?;

        // SAFETY: the display is open; wmhints came from Xlib and is freed
        // once.
        unsafe {
            (self.x.XSetWMHints)(display, xwindow, wmhints);
            (self.x.XFree)(wmhints.cast());
        }
        Ok(())
    }

    /// Translation of `X11_ShowWindowSystemMenu()`.
    pub(crate) fn x11_show_window_system_menu(&self, window: WindowID, x: i32, y: i32) {
        let displaydata = display_driver_data_for_window(window);
        let display = self.display;
        let Ok(xwindow) = with_x11_window(window, |_, d| d.xwindow) else {
            return;
        };
        let mut child_return: Window = 0;
        let mut wx = 0;
        let mut wy = 0;

        // SAFETY: the display is open and the window ours.
        unsafe {
            let root = RootWindow(display, screen_of(&displaydata));
            (self.x.XTranslateCoordinates)(
                display,
                xwindow,
                root,
                x,
                y,
                &mut wx,
                &mut wy,
                &mut child_return,
            );

            let mut e = client_message(
                xwindow,
                self.intern_atom("_GTK_SHOW_WINDOW_MENU", false),
                32,
            );
            let c = e.client_mut();
            c.data.l[0] = 0; // GTK device ID (unused)
            c.data.l[1] = wx as c_long; // X coordinate relative to root
            c.data.l[2] = wy as c_long; // Y coordinate relative to root

            (self.x.XSendEvent)(
                display,
                root,
                False,
                SubstructureRedirectMask | SubstructureNotifyMask,
                &mut e,
            );
            (self.x.XFlush)(display);
        }
    }

    /// Translation of `X11_SyncWindow()`.
    pub(crate) fn x11_sync_window(&self, window: WindowID) -> Result<()> {
        let (external, pending_operation) = with_x11_window(window, |w, d| {
            (
                w.flags().contains(WindowFlags::EXTERNAL),
                d.pending_operation,
            )
        })?;

        // If the window is external and has only a pending resize or move event, use the special external sync path to avoid processing events.
        if external
            && (pending_operation & !(X11_PENDING_OP_RESIZE | X11_PENDING_OP_MOVE))
                == X11_PENDING_OP_NONE
        {
            self.x11_external_resize_move_sync(window);
            return Ok(());
        }

        let current_time = crate::timer::ticks_ns();
        let mut timeout = 0;

        // Allow time for any pending mode switches to complete.
        for display in crate::video::display::displays().unwrap_or_default() {
            let deadline = with_x11_display(display, |_, d| d.mode_switch_deadline_ns).unwrap_or(0);
            if deadline != 0 && current_time < deadline {
                timeout = (deadline - current_time).max(timeout);
            }
        }

        /* 100ms is fine for most cases, but, for some reason, maximizing
         * a window can take a very long time.
         */
        timeout += if pending_operation & X11_PENDING_OP_MAXIMIZE != 0 {
            1_000_000_000
        } else {
            100_000_000
        };

        if self.x11_sync_window_timeout(window, timeout) {
            Ok(())
        } else {
            Err(Error::new("Timed out waiting for the window to be updated"))
        }
    }

    /// Translation of `X11_SetWindowFocusable()`.
    pub(crate) fn x11_set_window_focusable(&self, window: WindowID, focusable: bool) -> Result<()> {
        let flags = with_window(window, |w| w.flags())?;
        if !is_popup(window) {
            let display = self.display;
            let xwindow = with_x11_window(window, |_, d| d.xwindow)?;

            // SAFETY: the display is open and the window ours.
            let wmhints = unsafe { (self.x.XGetWMHints)(display, xwindow) };
            if wmhints.is_null() {
                return Err(Error::new("Couldn't get WM hints"));
            }

            // SAFETY: wmhints came from XGetWMHints and is freed once.
            unsafe {
                (*wmhints).input = focusable as Bool;
                (*wmhints).flags |= InputHint;

                (self.x.XSetWMHints)(display, xwindow, wmhints);
                (self.x.XFree)(wmhints.cast());
            }
        } else if flags.contains(WindowFlags::POPUP_MENU) {
            if !flags.contains(WindowFlags::HIDDEN) {
                if !focusable && flags.contains(WindowFlags::INPUT_FOCUS) {
                    if let Ok((new_focus, set_focus)) = should_relinquish_popup_focus(window) {
                        x11_set_keyboard_focus(new_focus, set_focus);
                    }
                } else if focusable && should_focus_popup(window) {
                    x11_set_keyboard_focus(Some(window), true);
                }
            }

            return Ok(());
        }

        Ok(())
    }

    /// The window whose X window is `xwindow`, for the XFixes barrier code
    /// (`active_cursor_confined_window`).
    #[allow(dead_code)]
    pub(crate) fn xfixes_barriers_supported(&self) -> bool {
        x11_xfixes_is_initialized() && self.x.xfixes.is_some()
    }
}
