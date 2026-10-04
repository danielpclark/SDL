// Rust translation of src/video/x11/SDL_x11video.c and SDL_x11video.h from
// Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The X11 video device: opening the display, the driver's private data
//! (`SDL_VideoData`), initialization and shutdown, and the table of entry
//! points (the [`VideoDriver`] implementation).
//!
//! The display connection is shared (`Arc<X11Display>`) with the objects
//! that outlive a driver call and need it to free X resources, such as
//! cursors; it is closed when the last of them goes.

use std::cell::RefCell;
use std::ffi::{c_int, c_uchar, c_ulong, CStr, CString};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use super::clipboard::ClipboardData;
use super::keyboard::KeyboardData;
use super::settings::SettingsData;
use super::sys::*;
use super::x11dyn::{load_symbols, unload_symbols, X11Syms};
use crate::error::{Error, Result};
use crate::events::mouse::{Cursor, CursorFrame, MouseButtonFlags, MouseFeature, SystemCursor};
use crate::events::{keyboard, mouse, DisplayID, KeyboardID, WindowID};
use crate::hints;
use crate::properties::Properties;
use crate::thread::ReentrantMutex;
use crate::video::messagebox::MessageBoxData;
use crate::video::sysvideo::{
    DeviceCaps, DisplayMode, FlashOperation, FullscreenOp, FullscreenResult, VideoBootStrap,
    VideoDriver,
};
use crate::video::window::WindowOp;
use crate::video::{Point, Rect, Surface};

/// An open connection to an X server (`Display *`), closed on drop.
pub(crate) struct X11Display {
    pub(crate) x: Arc<X11Syms>,
    pub(crate) display: *mut Display,
}

// SAFETY: XInitThreads() is called before any display is opened, so Xlib
// locks the connection around every call; the pointer itself is only
// handed to Xlib.
unsafe impl Send for X11Display {}
// SAFETY: as above.
unsafe impl Sync for X11Display {}

impl Drop for X11Display {
    fn drop(&mut self) {
        // SAFETY: the display was opened by XOpenDisplay and is closed once,
        // here, after every user of it is gone.
        unsafe {
            (self.x.XCloseDisplay)(self.display);
        }
    }
}

/// Useful atoms (the `atoms` member of `SDL_VideoData`).
#[allow(non_snake_case)]
#[derive(Clone, Copy, Default)]
pub(crate) struct Atoms {
    pub(crate) WM_PROTOCOLS: Atom,
    pub(crate) WM_DELETE_WINDOW: Atom,
    #[allow(dead_code)] // (looked up but unused, as upstream)
    pub(crate) WM_NAME: Atom,
    pub(crate) WM_TRANSIENT_FOR: Atom,
    pub(crate) WM_STATE: Atom,
    pub(crate) _NET_WM_STATE: Atom,
    pub(crate) _NET_WM_STATE_HIDDEN: Atom,
    pub(crate) _NET_WM_STATE_FOCUSED: Atom,
    pub(crate) _NET_WM_STATE_MAXIMIZED_VERT: Atom,
    pub(crate) _NET_WM_STATE_MAXIMIZED_HORZ: Atom,
    pub(crate) _NET_WM_STATE_FULLSCREEN: Atom,
    pub(crate) _NET_WM_STATE_ABOVE: Atom,
    pub(crate) _NET_WM_STATE_SKIP_TASKBAR: Atom,
    pub(crate) _NET_WM_STATE_SKIP_PAGER: Atom,
    pub(crate) _NET_WM_STATE_MODAL: Atom,
    pub(crate) _NET_WM_MOVERESIZE: Atom,
    pub(crate) _NET_WM_ALLOWED_ACTIONS: Atom,
    pub(crate) _NET_WM_ACTION_FULLSCREEN: Atom,
    pub(crate) _NET_WM_NAME: Atom,
    pub(crate) _NET_WM_ICON_NAME: Atom,
    pub(crate) _NET_WM_ICON: Atom,
    pub(crate) _NET_WM_PING: Atom,
    pub(crate) _NET_WM_SYNC_REQUEST: Atom,
    pub(crate) _NET_WM_SYNC_REQUEST_COUNTER: Atom,
    pub(crate) _NET_WM_WINDOW_OPACITY: Atom,
    pub(crate) _NET_WM_USER_TIME: Atom,
    pub(crate) _NET_ACTIVE_WINDOW: Atom,
    pub(crate) _NET_FRAME_EXTENTS: Atom,
    pub(crate) _SDL_WAKEUP: Atom,
    pub(crate) UTF8_STRING: Atom,
    pub(crate) PRIMARY: Atom,
    pub(crate) CLIPBOARD: Atom,
    pub(crate) INCR: Atom,
    pub(crate) SDL_SELECTION: Atom,
    pub(crate) TARGETS: Atom,
    pub(crate) SDL_FORMATS: Atom,
    #[allow(dead_code)] // (looked up but unused, as upstream)
    pub(crate) RESOURCE_MANAGER: Atom,
    pub(crate) XdndAware: Atom,
    pub(crate) XdndEnter: Atom,
    pub(crate) XdndLeave: Atom,
    pub(crate) XdndPosition: Atom,
    pub(crate) XdndStatus: Atom,
    pub(crate) XdndTypeList: Atom,
    pub(crate) XdndActionCopy: Atom,
    pub(crate) XdndDrop: Atom,
    pub(crate) XdndFinished: Atom,
    pub(crate) XdndSelection: Atom,
    pub(crate) XKLAVIER_STATE: Atom,

    // Pen atoms (these have names that don't map well to C symbols)
    pub(crate) pen_atom_device_product_id: Atom,
    pub(crate) pen_atom_abs_pressure: Atom,
    pub(crate) pen_atom_abs_tilt_x: Atom,
    pub(crate) pen_atom_abs_tilt_y: Atom,
    pub(crate) pen_atom_wacom_serial_ids: Atom,
    pub(crate) pen_atom_wacom_tool_type: Atom,
}

/// An entry of `windowlist`: the SDL window and its X window (upstream's
/// list of `SDL_WindowData *`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct WindowListEntry {
    pub(crate) window: WindowID,
    pub(crate) xwindow: Window,
}

/// The mutable part of the private display data. Translation of
/// `struct SDL_VideoData` (the display connections, the atoms and the
/// flags fixed at creation live directly in [`X11Video`]).
pub(crate) struct VideoData {
    pub(crate) pid: libc::pid_t,
    pub(crate) im: XIM,
    pub(crate) screensaver_activity: u64,
    pub(crate) windowlist: Vec<WindowListEntry>,
    pub(crate) window_group: XID,
    pub(crate) clipboard_window: Window,
    pub(crate) clipboard: ClipboardData,
    pub(crate) primary_selection: ClipboardData,
    pub(crate) active_cursor_confined_window: Option<WindowID>,
    pub(crate) xsettings_window: Window,
    pub(crate) xsettings_data: SettingsData,

    /// This is true for ICCCM2.0-compliant window managers
    pub(crate) net_wm: bool,

    pub(crate) selection_waiting: bool,
    pub(crate) selection_incr_waiting: bool,

    /// true if XGrabPointer seems unreliable.
    pub(crate) broken_pointer_grab: bool,

    pub(crate) last_mode_change_deadline: u64,

    pub(crate) global_mouse_changed: bool,
    pub(crate) global_mouse_position: Point,
    pub(crate) global_mouse_buttons: u32,

    pub(crate) xinput_last_button_serial: c_ulong,
    pub(crate) xinput_last_key_serial: c_ulong,
    pub(crate) xinput_last_keyboard_device: KeyboardID,
    pub(crate) xinput_master_pointer_device: c_int,
    pub(crate) xinput_hierarchy_changed: bool,

    pub(crate) xrandr_event_base: c_int,
    pub(crate) keyboard: KeyboardData,

    /// The cursors of `SDL_x11mouse.c` (its statics `sys_cursors`,
    /// `x11_empty_cursor` and `x11_cursor_visible`).
    pub(crate) mouse: super::mouse::MouseData,

    /// Vulkan variables only valid if the Vulkan loader is loaded
    pub(crate) vulkan: super::vulkan::VulkanData,
}

// SAFETY: the raw pointers (the input method, the XKB description) are
// Xlib objects of the display, which Xlib locks (XInitThreads).
unsafe impl Send for VideoData {}

/// The X11 video device: `SDL_VideoDevice` with its `SDL_VideoData`.
pub(crate) struct X11Video {
    /// The connection everything happens on (`display`).
    pub(crate) conn: Arc<X11Display>,
    /// A second connection for wakeup events (`request_display`).
    pub(crate) request_conn: Arc<X11Display>,
    /// The loaded X11 functions (also reachable through `conn`).
    pub(crate) x: Arc<X11Syms>,
    /// `data->display` (the pointer of `conn`).
    pub(crate) display: *mut Display,
    /// The atoms (written by `X11_VideoInit()` and `X11_InitPen()`).
    pub(crate) atoms: RwLock<Atoms>,
    caps: DeviceCaps,
    /// Used to interact with the on-screen keyboard
    pub(crate) use_steam_screen_keyboard: bool,
    pub(crate) is_xwayland: bool,
    data: ReentrantMutex<RefCell<VideoData>>,
}

// SAFETY: see X11Display; the mutable state is behind a lock.
unsafe impl Send for X11Video {}
// SAFETY: as above.
unsafe impl Sync for X11Video {}

impl X11Video {
    /// The atoms looked up by [`x11_video_init`](Self::x11_video_init)
    /// (all `None` before).
    pub(crate) fn atoms(&self) -> Atoms {
        *self.atoms.read().unwrap_or_else(|e| e.into_inner())
    }

    /// Run `f` on the private data. Never call back into SDL (or into
    /// another `with_data`) from `f`.
    pub(crate) fn with_data<R>(&self, f: impl FnOnce(&mut VideoData) -> R) -> R {
        let guard = self.data.lock();
        let mut data = guard.borrow_mut();
        f(&mut data)
    }

    /// Intern an atom (`X11_XInternAtom(display, name, only_if_exists)`).
    pub(crate) fn intern_atom(&self, name: &str, only_if_exists: bool) -> Atom {
        intern_atom(&self.x, self.display, name, only_if_exists)
    }
}

/// `X11_XInternAtom()` with a Rust string.
pub(crate) fn intern_atom(
    x: &X11Syms,
    display: *mut Display,
    name: &str,
    only_if_exists: bool,
) -> Atom {
    let Ok(name) = CString::new(name) else {
        return None;
    };
    // SAFETY: the display is open; the name is NUL-terminated.
    unsafe { (x.XInternAtom)(display, name.as_ptr(), only_if_exists as Bool) }
}

/// `X11_XGetAtomName()` as a Rust string (freeing Xlib's copy).
pub(crate) fn atom_name(x: &X11Syms, display: *mut Display, atom: Atom) -> Option<String> {
    // SAFETY: the display is open; XGetAtomName returns NULL or a
    // NUL-terminated string to be freed with XFree.
    unsafe {
        let name = (x.XGetAtomName)(display, atom);
        if name.is_null() {
            return Option::None;
        }
        let s = CStr::from_ptr(name).to_string_lossy().into_owned();
        (x.XFree)(name.cast());
        Some(s)
    }
}

impl Drop for X11Video {
    /// Translation of `X11_DeleteDevice()`.
    fn drop(&mut self) {
        let loaded = self.with_data(|d| d.vulkan.loader_loaded());
        if loaded {
            self.x11_vulkan_unload_library();
        }
        // (the displays close when the last reference to them goes)
        unload_symbols();
    }
}

/// Translation of `X11_IsXWayland()`.
fn x11_is_xwayland(x: &X11Syms, d: *mut Display) -> bool {
    let mut opcode = 0;
    let mut event = 0;
    let mut error = 0;
    // SAFETY: the display is open; the out-parameters are valid.
    unsafe {
        (x.XQueryExtension)(d, c"XWAYLAND".as_ptr(), &mut opcode, &mut event, &mut error) == True
    }
}

/// Translation of `X11_IsWSL()`.
fn x11_is_wsl() -> bool {
    if cfg!(target_os = "linux") {
        // if either of these exist, we're on WSL.
        if crate::filesystem::get_path_info("/proc/sys/fs/binfmt_misc/WSLInterop").is_ok()
            || crate::filesystem::get_path_info("/run/WSL").is_ok()
        {
            return true;
        }
    }
    false
}

/// Translation of `X11_CreateDevice()`.
fn x11_create_device() -> Option<Arc<dyn VideoDriver>> {
    let x = load_symbols()?;

    /* Need for threading gl calls. This is also required for the proprietary
    nVidia driver to be threaded. */
    // SAFETY: XInitThreads has no preconditions.
    unsafe {
        (x.XInitThreads)();
    }

    // Open the display first to be sure that X11 is available
    // (NULL: use the DISPLAY environment variable)
    // SAFETY: a NULL display name selects $DISPLAY.
    let x11_display = unsafe { (x.XOpenDisplay)(std::ptr::null()) };

    if x11_display.is_null() {
        unload_symbols();

        let session = crate::stdlib::getenv("XDG_SESSION_TYPE");
        if session.is_some_and(|s| s.eq_ignore_ascii_case("wayland")) {
            crate::debug!(
                crate::log::Category::Video,
                "Failed to connect to the X11 (XWayland) display server"
            );
        } else {
            crate::debug!(
                crate::log::Category::Video,
                "Failed to connect to the X11 display server"
            );
        }

        return Option::None;
    }
    let conn = Arc::new(X11Display {
        x: x.clone(),
        display: x11_display,
    });

    // SAFETY: as above.
    let request_display = unsafe { (x.XOpenDisplay)(std::ptr::null()) };
    if request_display.is_null() {
        drop(conn);
        unload_symbols();
        return Option::None;
    }
    let request_conn = Arc::new(X11Display {
        x: x.clone(),
        display: request_display,
    });

    /* Steam Deck will have an on-screen keyboard, so check their environment
     * variable so we can make use of SDL_StartTextInput.
     */
    let use_steam_screen_keyboard = hints::get_bool(hints::ENABLE_STEAM_SCREEN_KEYBOARD, false);

    let mut device_caps = DeviceCaps::HAS_POPUP_WINDOW_SUPPORT | DeviceCaps::SLOW_FRAMEBUFFER;

    let is_xwayland = x11_is_xwayland(&x, x11_display);
    if is_xwayland {
        crate::info!(crate::log::Category::Video, "Detected XWayland");

        device_caps = device_caps
            | DeviceCaps::MODE_SWITCHING_EMULATED
            | DeviceCaps::SENDS_FULLSCREEN_DIMENSIONS;
    }
    if x11_is_wsl() {
        // On WSL, direct X11 is faster than using OpenGL for window framebuffers, so try to detect WSL and avoid texture framebuffer.
        device_caps = DeviceCaps(device_caps.0 & !DeviceCaps::SLOW_FRAMEBUFFER.0);
    }

    let data = VideoData {
        pid: 0,
        im: std::ptr::null_mut(),
        screensaver_activity: 0,
        windowlist: Vec::new(),
        window_group: 0,
        clipboard_window: None,
        clipboard: ClipboardData::default(),
        primary_selection: ClipboardData::default(),
        active_cursor_confined_window: Option::None,
        xsettings_window: None,
        xsettings_data: SettingsData::default(),
        net_wm: false,
        selection_waiting: false,
        selection_incr_waiting: false,
        broken_pointer_grab: false,
        last_mode_change_deadline: 0,
        global_mouse_changed: true,
        global_mouse_position: Point::default(),
        global_mouse_buttons: 0,
        xinput_last_button_serial: 0,
        xinput_last_key_serial: 0,
        xinput_last_keyboard_device: 0,
        xinput_master_pointer_device: 0,
        xinput_hierarchy_changed: false,
        xrandr_event_base: 0,
        keyboard: KeyboardData::default(),
        mouse: super::mouse::MouseData::default(),
        vulkan: super::vulkan::VulkanData::default(),
    };

    // (X11_DEBUG would XSynchronize() the display here)

    // The function pointers are the VideoDriver implementation below; the
    // system theme comes with the D-Bus layer (SDL_SystemTheme_Init()).
    Some(Arc::new(X11Video {
        conn,
        request_conn,
        x,
        display: x11_display,
        atoms: RwLock::new(Atoms::default()),
        caps: device_caps,
        use_steam_screen_keyboard,
        is_xwayland,
        data: ReentrantMutex::new(RefCell::new(data)),
    }))
}

/// Translation of `X11_bootstrap`.
pub(crate) static X11_BOOTSTRAP: VideoBootStrap = VideoBootStrap {
    name: "x11",
    desc: "SDL X11 video driver",
    create: x11_create_device,
    show_message_box: Some(super::messagebox::x11_show_message_box),
    is_preferred: false,
};

/// The error handler `X11_CheckWindowManager()` replaced (its `handler`).
static CHECK_WM_HANDLER: std::sync::Mutex<XErrorHandler> = std::sync::Mutex::new(Option::None);

/// Translation of `X11_CheckWindowManagerErrorHandler()`.
unsafe extern "C" fn x11_check_window_manager_error_handler(
    d: *mut Display,
    e: *mut XErrorEvent,
) -> c_int {
    // SAFETY: Xlib passes a valid error event.
    if unsafe { (*e).error_code } == BadWindow {
        0
    } else {
        let handler = *CHECK_WM_HANDLER.lock().unwrap_or_else(|e| e.into_inner());
        match handler {
            // SAFETY: the previous handler, called as Xlib would.
            Some(h) => unsafe { h(d, e) },
            Option::None => 0,
        }
    }
}

impl X11Video {
    /// Translation of `X11_CheckWindowManager()`.
    fn x11_check_window_manager(&self) {
        let x = &self.x;
        let display = self.display;
        let mut real_type: Atom = 0;
        let mut real_format: c_int = 0;
        let mut items_read: c_ulong = 0;
        let mut items_left: c_ulong = 0;
        let mut propdata: *mut c_uchar = std::ptr::null_mut();
        let mut wm_window: Window = 0;

        // Set up a handler to gracefully catch errors
        // SAFETY: the display is open; the handler is a valid extern fn.
        unsafe {
            (x.XSync)(display, False);
            let previous = (x.XSetErrorHandler)(Some(x11_check_window_manager_error_handler));
            *CHECK_WM_HANDLER.lock().unwrap_or_else(|e| e.into_inner()) = previous;
        }

        let _NET_SUPPORTING_WM_CHECK = self.intern_atom("_NET_SUPPORTING_WM_CHECK", false);
        // SAFETY: the display is open; the out-parameters are valid, and
        // the property data (1 window when items_read > 0) is freed here.
        unsafe {
            let status = (x.XGetWindowProperty)(
                display,
                DefaultRootWindow(display),
                _NET_SUPPORTING_WM_CHECK,
                0,
                1,
                False,
                XA_WINDOW,
                &mut real_type,
                &mut real_format,
                &mut items_read,
                &mut items_left,
                &mut propdata,
            );
            if status == Success {
                if items_read != 0 {
                    wm_window = *(propdata as *const Window);
                }
                if !propdata.is_null() {
                    (x.XFree)(propdata.cast());
                    propdata = std::ptr::null_mut();
                }
            }

            if wm_window != 0 {
                let status = (x.XGetWindowProperty)(
                    display,
                    wm_window,
                    _NET_SUPPORTING_WM_CHECK,
                    0,
                    1,
                    False,
                    XA_WINDOW,
                    &mut real_type,
                    &mut real_format,
                    &mut items_read,
                    &mut items_left,
                    &mut propdata,
                );
                if status != Success || items_read == 0 || wm_window != *(propdata as *const Window)
                {
                    wm_window = None;
                }
                if status == Success && !propdata.is_null() {
                    (x.XFree)(propdata.cast());
                }
            }

            // Reset the error handler, we're done checking
            (x.XSync)(display, False);
            let handler = *CHECK_WM_HANDLER.lock().unwrap_or_else(|e| e.into_inner());
            (x.XSetErrorHandler)(handler);
        }

        if wm_window == 0 {
            // (DEBUG_WINDOW_MANAGER: "Couldn't get _NET_SUPPORTING_WM_CHECK property")
            return;
        }
        self.with_data(|d| d.net_wm = true);

        // (DEBUG_WINDOW_MANAGER logs the window manager's name here)
    }

    /// Translation of `X11_VideoInit()`.
    fn x11_video_init(&self) -> Result<()> {
        // Get the process PID to be associated to the window
        // SAFETY: getpid() has no preconditions.
        let pid = unsafe { libc::getpid() };
        self.with_data(|d| {
            d.pid = pid;

            // I have no idea how random this actually is, or has to be.
            d.window_group = (pid as usize ^ (self as *const X11Video as usize)) as XID;
        });

        // Look up some useful Atoms
        let display = self.display;
        let x = &self.x;
        macro_rules! atoms {
            ($($name:ident),* $(,)?) => {
                Atoms {
                    $($name: intern_atom(x, display, stringify!($name), false),)*
                    ..Atoms::default()
                }
            };
        }
        let atoms = atoms!(
            WM_PROTOCOLS,
            WM_DELETE_WINDOW,
            WM_NAME,
            WM_TRANSIENT_FOR,
            WM_STATE,
            _NET_WM_STATE,
            _NET_WM_STATE_HIDDEN,
            _NET_WM_STATE_FOCUSED,
            _NET_WM_STATE_MAXIMIZED_VERT,
            _NET_WM_STATE_MAXIMIZED_HORZ,
            _NET_WM_STATE_FULLSCREEN,
            _NET_WM_STATE_ABOVE,
            _NET_WM_STATE_SKIP_TASKBAR,
            _NET_WM_STATE_SKIP_PAGER,
            _NET_WM_MOVERESIZE,
            _NET_WM_STATE_MODAL,
            _NET_WM_ALLOWED_ACTIONS,
            _NET_WM_ACTION_FULLSCREEN,
            _NET_WM_NAME,
            _NET_WM_ICON_NAME,
            _NET_WM_ICON,
            _NET_WM_PING,
            _NET_WM_SYNC_REQUEST,
            _NET_WM_SYNC_REQUEST_COUNTER,
            _NET_WM_WINDOW_OPACITY,
            _NET_WM_USER_TIME,
            _NET_ACTIVE_WINDOW,
            _NET_FRAME_EXTENTS,
            _SDL_WAKEUP,
            UTF8_STRING,
            PRIMARY,
            CLIPBOARD,
            INCR,
            SDL_SELECTION,
            TARGETS,
            SDL_FORMATS,
            RESOURCE_MANAGER,
            XdndAware,
            XdndEnter,
            XdndLeave,
            XdndPosition,
            XdndStatus,
            XdndTypeList,
            XdndActionCopy,
            XdndDrop,
            XdndFinished,
            XdndSelection,
            XKLAVIER_STATE,
        );
        // (the pen atoms are added by X11_InitPen())
        *self.atoms.write().unwrap_or_else(|e| e.into_inner()) = atoms;

        // Detect the window manager
        self.x11_check_window_manager();

        self.x11_init_modes()?;

        if !self.x11_init_xinput2() {
            // Assume a mouse and keyboard are attached
            keyboard::add_keyboard(keyboard::DEFAULT_KEYBOARD_ID, Option::None);
            mouse::add_mouse(mouse::DEFAULT_MOUSE_ID, Option::None);
        }

        self.x11_init_xfixes();

        self.x11_init_xsettings();

        self.x11_init_xsync();

        self.x11_init_xtest();

        self.x11_init_keyboard()?;
        self.x11_init_mouse();

        self.x11_init_touch();

        self.x11_init_pen();

        // Request currently available mime-types in the clipboard.
        let atoms = self.atoms();
        let window = self.get_window();
        // SAFETY: the display is open; the window is the clipboard window.
        unsafe {
            (self.x.XConvertSelection)(
                self.display,
                atoms.CLIPBOARD,
                atoms.TARGETS,
                atoms.SDL_FORMATS,
                window,
                CurrentTime,
            );
        }

        Ok(())
    }

    /// Translation of `X11_VideoQuit()`.
    fn x11_video_quit(&self) {
        let (clipboard_window, xsettings_window) =
            self.with_data(|d| (d.clipboard_window, d.xsettings_window));

        // SAFETY: the display is open; the windows are ours.
        unsafe {
            if clipboard_window != 0 {
                (self.x.XDestroyWindow)(self.display, clipboard_window);
            }

            if xsettings_window != 0 {
                (self.x.XDestroyWindow)(self.display, xsettings_window);
            }
        }

        self.x11_quit_xinput2();
        self.x11_quit_modes();
        self.x11_quit_keyboard();
        self.x11_quit_mouse();
        self.x11_quit_touch();
        self.x11_quit_pen();
        self.x11_quit_clipboard();
        self.x11_quit_xsettings();
    }
}

/// Translation of `X11_UseDirectColorVisuals()`.
pub(crate) fn x11_use_direct_color_visuals() -> bool {
    if hints::get_bool(hints::VIDEO_X11_NODIRECTCOLOR, false) {
        return false;
    }
    true
}

/// `timeoutNS` for `X11_WaitEventTimeout()`: -1 waits forever.
fn timeout_ns(timeout: Option<Duration>) -> i64 {
    match timeout {
        Some(d) => i64::try_from(d.as_nanos()).unwrap_or(i64::MAX),
        Option::None => -1,
    }
}

/// The entry points (`device->VideoInit = X11_VideoInit;` ...).
impl VideoDriver for X11Video {
    fn caps(&self) -> DeviceCaps {
        self.caps
    }

    fn video_init(&self) -> Result<()> {
        self.x11_video_init()
    }

    fn video_quit(&self) {
        self.x11_video_quit()
    }

    fn reset_touch(&self) -> Option<()> {
        self.x11_reset_touch();
        Some(())
    }

    fn display_modes(&self, display: DisplayID) -> Option<()> {
        let _ = self.x11_get_display_modes(display);
        Some(())
    }

    fn display_bounds(&self, display: DisplayID) -> Option<Result<Rect>> {
        Some(self.x11_get_display_bounds(display))
    }

    fn display_usable_bounds(&self, display: DisplayID) -> Option<Result<Rect>> {
        Some(self.x11_get_display_usable_bounds(display))
    }

    fn window_icc_profile(&self, window: WindowID) -> Option<Result<Vec<u8>>> {
        Some(self.x11_get_window_icc_profile(window))
    }

    fn set_display_mode(&self, display: DisplayID, mode: &DisplayMode) -> Option<Result<()>> {
        Some(self.x11_set_display_mode(display, mode))
    }

    fn suspend_screen_saver(&self, suspend: bool) -> Option<Result<()>> {
        Some(self.x11_suspend_screen_saver(suspend))
    }

    fn pump_events(&self) -> Option<()> {
        self.x11_pump_events();
        Some(())
    }

    fn wait_event_timeout(&self, timeout: Option<Duration>) -> Option<i32> {
        Some(self.x11_wait_event_timeout(timeout_ns(timeout)))
    }

    fn send_wakeup_event(&self, window: WindowID) -> Option<()> {
        self.x11_send_wakeup_event(window);
        Some(())
    }

    fn can_wait_events(&self) -> bool {
        true
    }

    fn create_window(&self, window: WindowID, create_props: &Properties) -> Option<Result<()>> {
        Some(self.x11_create_window(window, create_props))
    }

    fn set_window_title(&self, window: WindowID) -> Option<()> {
        self.x11_set_window_title(window);
        Some(())
    }

    fn set_window_icon(&self, window: WindowID, icon: &Surface<'static>) -> Option<Result<()>> {
        Some(self.x11_set_window_icon(window, Some(icon)))
    }

    fn set_window_position(&self, window: WindowID) -> Option<Result<()>> {
        Some(self.x11_set_window_position(window))
    }

    fn set_window_size(&self, window: WindowID) -> Option<()> {
        self.x11_set_window_size(window);
        Some(())
    }

    fn set_window_minimum_size(&self, window: WindowID) -> Option<()> {
        self.x11_set_window_minimum_size(window);
        Some(())
    }

    fn set_window_maximum_size(&self, window: WindowID) -> Option<()> {
        self.x11_set_window_maximum_size(window);
        Some(())
    }

    fn set_window_aspect_ratio(&self, window: WindowID) -> Option<()> {
        self.x11_set_window_aspect_ratio(window);
        Some(())
    }

    fn window_borders_size(&self, window: WindowID) -> Option<Result<(i32, i32, i32, i32)>> {
        Some(self.x11_get_window_borders_size(window))
    }

    fn set_window_opacity(&self, window: WindowID, opacity: f32) -> Option<Result<()>> {
        Some(self.x11_set_window_opacity(window, opacity))
    }

    fn set_window_parent(&self, window: WindowID, parent: Option<WindowID>) -> Option<Result<()>> {
        Some(self.x11_set_window_parent(window, parent))
    }

    fn set_window_modal(&self, window: WindowID, modal: bool) -> Option<Result<()>> {
        Some(self.x11_set_window_modal(window, modal))
    }

    fn show_window(&self, window: WindowID) -> Option<()> {
        self.x11_show_window(window);
        Some(())
    }

    fn hide_window(&self, window: WindowID) -> Option<()> {
        self.x11_hide_window(window);
        Some(())
    }

    fn raise_window(&self, window: WindowID) -> Option<()> {
        self.x11_raise_window(window);
        Some(())
    }

    fn maximize_window(&self, window: WindowID) -> Option<()> {
        self.x11_maximize_window(window);
        Some(())
    }

    fn minimize_window(&self, window: WindowID) -> Option<()> {
        self.x11_minimize_window(window);
        Some(())
    }

    fn restore_window(&self, window: WindowID) -> Option<()> {
        self.x11_restore_window(window);
        Some(())
    }

    fn set_window_bordered(&self, window: WindowID, bordered: bool) -> Option<()> {
        self.x11_set_window_bordered(window, bordered);
        Some(())
    }

    fn set_window_resizable(&self, window: WindowID, resizable: bool) -> Option<()> {
        self.x11_set_window_resizable(window, resizable);
        Some(())
    }

    fn set_window_always_on_top(&self, window: WindowID, on_top: bool) -> Option<()> {
        self.x11_set_window_always_on_top(window, on_top);
        Some(())
    }

    fn set_window_fullscreen(
        &self,
        window: WindowID,
        display: DisplayID,
        fullscreen: FullscreenOp,
    ) -> Option<FullscreenResult> {
        Some(self.x11_set_window_fullscreen(window, display, fullscreen))
    }

    fn set_window_mouse_grab(&self, window: WindowID, grabbed: bool) -> Option<Result<()>> {
        Some(self.x11_set_window_mouse_grab(window, grabbed))
    }

    fn set_window_keyboard_grab(&self, window: WindowID, grabbed: bool) -> Option<Result<()>> {
        Some(self.x11_set_window_keyboard_grab(window, grabbed))
    }

    fn set_window_mouse_rect(&self, window: WindowID) -> Option<Result<()>> {
        Some(self.x11_set_window_mouse_rect(window))
    }

    fn destroy_window(&self, window: WindowID) -> Option<()> {
        self.x11_destroy_window(window);
        Some(())
    }

    fn implements_window_framebuffer(&self) -> bool {
        true
    }

    fn create_window_framebuffer(
        &self,
        window: WindowID,
        _w: i32,
        _h: i32,
    ) -> Option<Result<Surface<'static>>> {
        Some(self.x11_create_window_framebuffer(window))
    }

    fn update_window_framebuffer(
        &self,
        window: WindowID,
        surface: &Surface<'static>,
        rects: &[Rect],
    ) -> Option<Result<()>> {
        Some(self.x11_update_window_framebuffer(window, surface, rects))
    }

    fn destroy_window_framebuffer(&self, window: WindowID) -> Option<()> {
        self.x11_destroy_window_framebuffer(window);
        Some(())
    }

    fn set_window_hit_test(&self, window: WindowID, enabled: bool) -> Option<Result<()>> {
        Some(super::window::x11_set_window_hit_test(window, enabled))
    }

    fn accept_drag_and_drop(&self, window: WindowID, accept: bool) -> Option<()> {
        self.x11_accept_drag_and_drop(window, accept);
        Some(())
    }

    fn update_window_shape(
        &self,
        window: WindowID,
        shape: Option<&Surface<'static>>,
    ) -> Option<Result<()>> {
        Some(self.x11_update_window_shape(window, shape))
    }

    fn flash_window(&self, window: WindowID, operation: FlashOperation) -> Option<Result<()>> {
        Some(self.x11_flash_window(window, operation))
    }

    // (ApplyWindowProgress is DBUS_ApplyWindowProgress, which comes with
    // the D-Bus layer)

    fn show_window_system_menu(&self, window: WindowID, x: i32, y: i32) -> Option<()> {
        self.x11_show_window_system_menu(window, x, y);
        Some(())
    }

    fn set_window_focusable(&self, window: WindowID, focusable: bool) -> Option<Result<()>> {
        Some(self.x11_set_window_focusable(window, focusable))
    }

    fn sync_window(&self, window: WindowID) -> Option<Result<()>> {
        Some(self.x11_sync_window(window))
    }

    fn implements_window_op(&self, op: WindowOp) -> bool {
        matches!(
            op,
            WindowOp::SetBordered
                | WindowOp::SetResizable
                | WindowOp::SetAlwaysOnTop
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

    fn implements_sync_window(&self) -> bool {
        true
    }

    // * * * OpenGL: GLX and EGL come with the OpenGL front end (see
    // `opengl.rs`); Vulkan:

    fn implements_vulkan_surfaces(&self) -> bool {
        true
    }

    fn vulkan_load_library(&self, path: Option<&str>) -> Option<Result<()>> {
        Some(self.x11_vulkan_load_library(path))
    }

    fn vulkan_unload_library(&self) -> Option<()> {
        self.x11_vulkan_unload_library();
        Some(())
    }

    fn vulkan_get_instance_proc_addr(&self) -> Option<usize> {
        self.x11_vulkan_get_instance_proc_addr()
    }

    fn vulkan_instance_extensions(&self) -> Option<Vec<&'static str>> {
        Some(self.x11_vulkan_get_instance_extensions())
    }

    fn vulkan_create_surface(
        &self,
        window: WindowID,
        instance: usize,
        allocator: usize,
    ) -> Option<Result<u64>> {
        Some(self.x11_vulkan_create_surface(window, instance, allocator))
    }

    fn vulkan_destroy_surface(
        &self,
        instance: usize,
        surface: u64,
        allocator: usize,
    ) -> Option<()> {
        self.x11_vulkan_destroy_surface(instance, surface, allocator);
        Some(())
    }

    fn vulkan_presentation_support(
        &self,
        instance: usize,
        physical_device: usize,
        queue_family_index: u32,
    ) -> Option<bool> {
        Some(self.x11_vulkan_get_presentation_support(
            instance,
            physical_device,
            queue_family_index,
        ))
    }

    // * * * Clipboard

    fn text_mime_types(&self) -> Option<Vec<String>> {
        Some(super::clipboard::x11_get_text_mime_types())
    }

    fn set_clipboard_data(&self) -> Option<Result<()>> {
        Some(self.x11_set_clipboard_data())
    }

    fn clipboard_data(&self, mime_type: &str) -> Option<Option<Vec<u8>>> {
        Some(self.x11_get_clipboard_data(mime_type))
    }

    fn has_clipboard_data(&self, mime_type: &str) -> Option<bool> {
        Some(self.x11_has_clipboard_data(mime_type))
    }

    fn set_primary_selection_text(&self, text: &str) -> Option<Result<()>> {
        Some(self.x11_set_primary_selection_text(text))
    }

    fn primary_selection_text(&self) -> Option<String> {
        Some(self.x11_get_primary_selection_text())
    }

    fn has_primary_selection_text(&self) -> Option<bool> {
        Some(self.x11_has_primary_selection_text())
    }

    // * * * Text input

    fn start_text_input(&self, window: WindowID, props: Option<&Properties>) -> Option<Result<()>> {
        Some(self.x11_start_text_input(window, props))
    }

    fn stop_text_input(&self, window: WindowID) -> Option<Result<()>> {
        Some(self.x11_stop_text_input(window))
    }

    fn update_text_input_area(&self, window: WindowID) -> Option<Result<()>> {
        Some(self.x11_update_text_input_area(window))
    }

    fn has_screen_keyboard_support(&self) -> Option<bool> {
        Some(self.x11_has_screen_keyboard_support())
    }

    fn show_screen_keyboard(&self, window: WindowID, props: Option<&Properties>) -> Option<()> {
        self.x11_show_screen_keyboard(window, props);
        Some(())
    }

    fn hide_screen_keyboard(&self, window: WindowID) -> Option<()> {
        self.x11_hide_screen_keyboard(window);
        Some(())
    }

    // * * * Message boxes

    fn show_message_box(&self, data: &MessageBoxData) -> Option<Result<i32>> {
        // (the device has no ShowMessageBox; the bootstrap's is used)
        let _ = data;
        Option::None
    }

    // * * * The SDL_Mouse entry points (X11_InitMouse())

    fn warp_mouse(&self, window: WindowID, x: f32, y: f32) -> Option<Result<()>> {
        Some(self.x11_warp_mouse(window, x, y))
    }

    fn warp_mouse_global(&self, x: f32, y: f32) -> Option<Result<()>> {
        Some(self.x11_warp_mouse_global(x, y))
    }

    fn set_relative_mouse_mode(&self, enabled: bool) -> Option<Result<()>> {
        Some(self.x11_set_relative_mouse_mode(enabled))
    }

    fn capture_mouse(&self, window: Option<WindowID>) -> Option<Result<()>> {
        Some(self.x11_capture_mouse(window))
    }

    fn global_mouse_state(&self) -> Option<(f32, f32, MouseButtonFlags)> {
        Some(self.x11_get_global_mouse_state())
    }

    fn create_cursor(
        &self,
        surface: &Surface<'_>,
        hot_x: i32,
        hot_y: i32,
    ) -> Option<Result<Cursor>> {
        Some(self.x11_create_cursor(surface, hot_x, hot_y))
    }

    fn create_animated_cursor(
        &self,
        frames: &[CursorFrame<'_>],
        hot_x: i32,
        hot_y: i32,
    ) -> Option<Result<Cursor>> {
        Some(self.x11_create_animated_cursor(frames, hot_x, hot_y))
    }

    fn create_system_cursor(&self, id: SystemCursor) -> Option<Result<Cursor>> {
        Some(
            self.x11_create_system_cursor(id)
                .ok_or_else(|| Error::new("Couldn't create the system cursor")),
        )
    }

    fn show_cursor(&self, cursor: Option<&Cursor>) -> Option<Result<()>> {
        Some(self.x11_show_cursor(cursor))
    }

    fn implements_mouse_feature(&self, feature: MouseFeature) -> bool {
        matches!(
            feature,
            MouseFeature::WarpMouse
                | MouseFeature::WarpMouseGlobal
                | MouseFeature::SetRelativeMouseMode
                | MouseFeature::CaptureMouse
                | MouseFeature::GetGlobalMouseState
        )
    }
}
