// Rust translation of src/video/windows/SDL_windowswindow.c and
// SDL_windowswindow.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Win32 windows: styles, creation and destruction, position and size
//! (with the per-monitor DPI frame sizes), fullscreen, icons, input grabs
//! and cursor clipping, opacity, OLE drag and drop, the taskbar progress
//! bar.
//!
//! The driver data of a window ([`WindowData`], `SDL_WindowData`) is
//! shared between the video core (`window->internal`), the window property
//! `SDL_WindowData` the window procedure falls back to, and the raw input
//! thread; its mutable fields are behind a [`Shared`] lock that is never
//! held across a call that sends window messages.

use std::ffi::c_void;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use windows_sys::core::{GUID, HRESULT};
use windows_sys::Win32::Foundation::{
    GetLastError, E_INVALIDARG, E_NOINTERFACE, HINSTANCE, HWND, LPARAM, POINT, POINTL, RECT, S_OK,
    WPARAM,
};
use windows_sys::Win32::Graphics::Gdi::GetMonitorInfoW;
use windows_sys::Win32::Graphics::Gdi::{
    ClientToScreen, CreateDCW, CreateRectRgn, DeleteDC, DeleteObject, GetDC, IntersectRect,
    PtInRect, ReleaseDC, ScreenToClient, BITMAPINFOHEADER, BI_RGB, HBITMAP, HDC, MONITORINFO,
};
use windows_sys::Win32::Graphics::OpenGL::{
    DescribePixelFormat, GetPixelFormat, SetPixelFormat, PIXELFORMATDESCRIPTOR,
};
use windows_sys::Win32::System::Com::{
    CoCreateInstance, CLSCTX_ALL, DVASPECT_CONTENT, FORMATETC, STGMEDIUM, TYMED_HGLOBAL,
};
use windows_sys::Win32::System::DataExchange::{GetClipboardFormatNameA, RegisterClipboardFormatW};
use windows_sys::Win32::System::LibraryLoader::{
    GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
    GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
};
use windows_sys::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
use windows_sys::Win32::System::Ole::{
    RegisterDragDrop, ReleaseStgMedium, RevokeDragDrop, CF_BITMAP, CF_DIB, CF_DIBV5, CF_DIF,
    CF_DSPBITMAP, CF_DSPENHMETAFILE, CF_DSPMETAFILEPICT, CF_DSPTEXT, CF_ENHMETAFILE, CF_HDROP,
    CF_LOCALE, CF_METAFILEPICT, CF_OEMTEXT, CF_OWNERDISPLAY, CF_PALETTE, CF_PENDATA, CF_RIFF,
    CF_SYLK, CF_TEXT, CF_TIFF, CF_UNICODETEXT, CF_WAVE, DROPEFFECT_COPY,
};
use windows_sys::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows_sys::Win32::UI::ColorSystem::GetICMProfileW;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetFocus;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    EnableWindow, GetKeyboardState, SetActiveWindow, SetFocus,
};
use windows_sys::Win32::UI::Shell::{DragAcceptFiles, DragQueryFileW, HDROP};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AdjustWindowRectEx, ClipCursor, CreateIconFromResource, CreateWindowExW, DestroyIcon,
    DestroyWindow, FlashWindowEx, GetClientRect, GetClipCursor, GetForegroundWindow, GetMenu,
    GetPropW, GetSystemMetrics, GetWindowRect, GetWindowTextLengthW, GetWindowTextW,
    GetWindowThreadProcessId, RemovePropW, SendMessageW, SetForegroundWindow,
    SetLayeredWindowAttributes, SetParent, SetPropW, SetWindowPos, SetWindowTextW,
    SetWindowsHookExW, ShowWindow, UnhookWindowsHookEx, FLASHWINFO, FLASHW_STOP, FLASHW_TIMERNOFG,
    FLASHW_TRAY, GWLP_HINSTANCE, GWLP_HWNDPARENT, GWLP_WNDPROC, GWL_EXSTYLE, GWL_STYLE, HHOOK,
    HICON, HWND_NOTOPMOST, HWND_TOP, HWND_TOPMOST, ICON_BIG, ICON_SMALL, LWA_ALPHA,
    SM_REMOTESESSION, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOCOPYBITS, SWP_NOMOVE,
    SWP_NOOWNERZORDER, SWP_NOSIZE, SWP_NOZORDER, SWP_SHOWWINDOW, SW_HIDE, SW_MAXIMIZE, SW_MINIMIZE,
    SW_RESTORE, SW_SHOW, SW_SHOWMINNOACTIVE, WH_KEYBOARD_LL, WM_SETICON, WNDPROC, WS_CAPTION,
    WS_CHILD, WS_CHILDWINDOW, WS_CLIPCHILDREN, WS_CLIPSIBLINGS, WS_EX_LAYERED, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_MAXIMIZE, WS_MAXIMIZEBOX, WS_MINIMIZE, WS_MINIMIZEBOX, WS_OVERLAPPED,
    WS_POPUP, WS_SYSMENU, WS_THICKFRAME, WS_VISIBLE,
};

use super::events::{
    app_instance, app_name, keyboard_hook_proc, pump_events_for_hwnd, window_proc,
};
use super::modes::with_display_data;
use super::opengl::win_gl_use_egl;
use super::{
    get_window_long_ptr, set_window_long_ptr, video_data, DwmBlurBehind, ITaskbarList3, Shared,
    VideoData, DWMWA_BORDER_COLOR, DWMWA_COLOR_DEFAULT, DWMWA_COLOR_NONE,
    DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DEFAULT, DWMWCP_DONOTROUND, DWM_BB_BLURREGION,
    DWM_BB_ENABLE, USER_DEFAULT_SCREEN_DPI,
};
use crate::core::windows::{
    is_equal_guid, is_windows_11_or_greater, set_error, update_dark_mode_for_hwnd, utf8_to_wide,
    wide_to_utf8, window_rect_valid,
};
use crate::error::{Error, Result};
use crate::events::window::{
    send_drop_complete, send_drop_file, send_drop_position, send_drop_text, send_window_event,
    WindowFlags,
};
use crate::events::{keyboard, mouse, DisplayID, EventType, WindowID};
use crate::hints;
use crate::log::Category;
use crate::properties::Properties;
use crate::video::core::{with_device, with_window};
use crate::video::gl::{gl_config, GL_CONTEXT_PROFILE_ES};
use crate::video::sysvideo::{FlashOperation, FullscreenOp, FullscreenResult, ProgressState};
use crate::video::window::{
    display_for_window, relative_to_global_for_window, should_allow_topmost, should_focus_popup,
    should_relinquish_popup_focus, PROP_WINDOW_CREATE_WIN32_HWND_POINTER,
    PROP_WINDOW_CREATE_WIN32_PIXEL_FORMAT_HWND_POINTER, PROP_WINDOW_CREATE_WIN32_STYLE_EX_NUMBER,
    PROP_WINDOW_WIN32_HDC_POINTER, PROP_WINDOW_WIN32_HWND_POINTER,
    PROP_WINDOW_WIN32_INSTANCE_POINTER,
};
use crate::video::{Point, Surface};

/// Translation of `SDL_WindowRect`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum WindowRect {
    Current,
    Windowed,
    Floating,
    Pending,
}

/// Translation of `SDL_WindowEraseBackgroundMode`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum EraseBackgroundMode {
    Never,
    Initial,
    Always,
}

/// The mutable part of `struct SDL_WindowData`.
pub(crate) struct WindowState {
    pub(crate) mdc: HDC,
    pub(crate) hbm: HBITMAP,
    /// The DIB section's pixels (`CreateDIBSection()`'s `ppvBits`).
    pub(crate) bits: *mut c_void,
    /// The DIB section's pitch and height.
    pub(crate) bits_pitch: usize,
    pub(crate) bits_h: usize,
    pub(crate) hicon: HICON,
    pub(crate) wndproc: WNDPROC,
    pub(crate) keyboard_hook: HHOOK,
    pub(crate) mouse_button_flags: WPARAM,
    pub(crate) last_pointer_update: LPARAM,
    pub(crate) high_surrogate: u16,
    pub(crate) initializing: bool,
    pub(crate) expected_resize: bool,
    pub(crate) in_border_change: bool,
    pub(crate) in_title_click: bool,
    pub(crate) focus_click_pending: u8,
    pub(crate) postpone_clipcursor: bool,
    pub(crate) clipcursor_queued: bool,
    pub(crate) windowed_mode_was_maximized: bool,
    pub(crate) in_window_deactivation: bool,
    pub(crate) force_ws_maximizebox: bool,
    pub(crate) disable_move_size_events: bool,
    pub(crate) showing_window: bool,
    pub(crate) in_modal_loop: i32,
    pub(crate) last_modal_width: i32,
    pub(crate) last_modal_height: i32,
    pub(crate) initial_size_rect: RECT,
    /// last successfully committed clipping rect for this window
    pub(crate) cursor_clipped_rect: RECT,
    pub(crate) mouse_tracked: bool,
    pub(crate) destroy_parent_with_window: bool,
    pub(crate) icm_file_name: Option<Vec<u16>>,
    pub(crate) taskbar_button_created: bool,
    pub(crate) drop_target: *mut DropTarget,
    /// The window's EGL surface (`EGL_NO_SURFACE` without EGL).
    pub(crate) egl_surface: *mut c_void,
}

// SAFETY: the handles are process-wide tokens; the window's own handles
// are only used on its thread, inside the Shared lock.
unsafe impl Send for WindowState {}

/// The driver data of a window. Translation of `struct SDL_WindowData`.
pub(crate) struct WindowData {
    pub(crate) window: WindowID,
    pub(crate) hwnd: HWND,
    pub(crate) parent: HWND,
    pub(crate) hdc: HDC,
    pub(crate) hinstance: HINSTANCE,
    /// this is Windows-specific, but probably does not need to be per-window
    pub(crate) cursor_ctrlock_rect: RECT,
    pub(crate) hint_erase_background_mode: EraseBackgroundMode,
    /// Whether we retain the content of the window when changing state
    pub(crate) copybits_flag: u32,
    pub(crate) state: Shared<WindowState>,
}

// SAFETY: the handles are process-wide tokens; see WindowState.
unsafe impl Send for WindowData {}
// SAFETY: as above; the mutable state is behind the Shared lock.
unsafe impl Sync for WindowData {}

/// The name of the window property upstream stores the window data in.
const WINDOW_DATA_PROP: &str = "SDL_WindowData";

/// The driver data of a window (`window->internal`).
pub(crate) fn window_data(window: WindowID) -> Option<Arc<WindowData>> {
    with_window(window, |w| {
        w.internal
            .as_ref()
            .and_then(|i| i.downcast_ref::<Arc<WindowData>>())
            .cloned()
    })
    .ok()
    .flatten()
}

/// The window flags (none if the window is gone).
pub(crate) fn window_flags(window: WindowID) -> WindowFlags {
    with_window(window, |w| w.core.flags).unwrap_or_default()
}

/// `SDL_WINDOW_IS_POPUP(window)`.
pub(crate) fn is_popup(window: WindowID) -> bool {
    with_window(window, |w| w.is_popup()).unwrap_or(false)
}

/// The window's parent (`window->parent`).
fn window_parent(window: WindowID) -> Option<WindowID> {
    with_window(window, |w| w.parent).ok().flatten()
}

/* For borderless Windows, still want the following flag:
  - WS_MINIMIZEBOX: window will respond to Windows minimize commands sent to all windows, such as windows key + m, shaking title bar, etc.
  Additionally, non-fullscreen windows can add:
  - WS_CAPTION: this seems to enable the Windows minimize animation
  - WS_SYSMENU: enables system context menu on task bar
  This will also cause the task bar to overlap the window and other windowed behaviors, so only use this for windows that shouldn't appear to be fullscreen
  - WS_THICKFRAME: allows hit-testing to resize window (doesn't actually add a frame to a borderless window).
  - WS_MAXIMIZEBOX: window will respond to Windows maximize commands sent to all windows, and the window will fill the usable desktop area rather than the whole screen
*/

pub(crate) const STYLE_BASIC: u32 = WS_CLIPSIBLINGS | WS_CLIPCHILDREN;
const STYLE_FULLSCREEN: u32 = WS_POPUP | WS_MINIMIZEBOX;
const STYLE_BORDERLESS: u32 = WS_POPUP | WS_MINIMIZEBOX;
const STYLE_BORDERLESS_WINDOWED: u32 = WS_POPUP | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;
const STYLE_NORMAL: u32 = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;
const STYLE_RESIZABLE: u32 = WS_THICKFRAME | WS_MAXIMIZEBOX;
const STYLE_MASK: u32 = STYLE_FULLSCREEN | STYLE_BORDERLESS | STYLE_NORMAL | STYLE_RESIZABLE;

/* An undocumented message to create a popup system menu
 * - wParam is always 0
 * - lParam = MAKELONG(x, y) where x and y are the screen coordinates where the menu should be displayed
 */
const WM_POPUPSYSTEMMENU: u32 = 0x313;

/// The window style for the SDL window's flags. Translation of
/// `GetWindowStyle()`.
pub(crate) fn get_window_style(window: WindowID) -> u32 {
    let mut style = 0;
    let flags = window_flags(window);

    if is_popup(window) {
        style |= WS_POPUP;
    } else if flags.contains(WindowFlags::FULLSCREEN) {
        style |= STYLE_FULLSCREEN;
    } else {
        if flags.contains(WindowFlags::BORDERLESS) {
            /* This behavior more closely matches other platform where the window is borderless
              but still interacts with the window manager (e.g. task bar shows above it, it can
              be resized to fit within usable desktop area, etc.)
            */
            if hints::get_bool("SDL_BORDERLESS_WINDOWED_STYLE", true) {
                style |= STYLE_BORDERLESS_WINDOWED;
            } else {
                style |= STYLE_BORDERLESS;
            }
        } else {
            style |= STYLE_NORMAL;
        }

        /* The WS_MAXIMIZEBOX style flag needs to be retained for as long as the window is maximized,
         * or restoration from minimized can fail, and leaving maximized can result in an odd size.
         */
        if flags.contains(WindowFlags::RESIZABLE) {
            /* You can have a borderless resizable window, but Windows doesn't always draw it correctly,
              see https://bugzilla.libsdl.org/show_bug.cgi?id=4466
            */
            if !flags.contains(WindowFlags::BORDERLESS)
                || hints::get_bool("SDL_BORDERLESS_RESIZABLE_STYLE", true)
            {
                style |= STYLE_RESIZABLE;
            }
        }

        if window_data(window).is_some_and(|d| d.state.with(|s| s.force_ws_maximizebox)) {
            /* Even if the resizable flag is cleared, WS_MAXIMIZEBOX is still needed as long
             * as the window is maximized, or de-maximizing or minimizing and restoring the
             * maximized window can result in the window disappearing or being the wrong size.
             */
            style |= WS_MAXIMIZEBOX;
        }

        // Need to set initialize minimize style, or when we call ShowWindow with WS_MINIMIZE it will activate a random window
        if flags.contains(WindowFlags::MINIMIZED) {
            style |= WS_MINIMIZE;
        }
    }
    style
}

/// Translation of `GetWindowStyleEx()`.
fn get_window_style_ex(window: WindowID) -> u32 {
    let mut style = 0;
    let flags = window_flags(window);

    if is_popup(window) || flags.contains(WindowFlags::UTILITY) {
        style |= WS_EX_TOOLWINDOW;
    }
    if is_popup(window) || flags.contains(WindowFlags::NOT_FOCUSABLE) {
        style |= WS_EX_NOACTIVATE;
    }
    style
}

/// `CLSID_TaskbarList`
const CLSID_TASKBAR_LIST: GUID = GUID {
    data1: 0x56FDF344,
    data2: 0xFD6D,
    data3: 0x11d0,
    data4: [0x95, 0x8A, 0x00, 0x60, 0x97, 0xC9, 0xA0, 0x90],
};
/// `IID_ITaskbarList3`
const IID_ITASKBAR_LIST3: GUID = GUID {
    data1: 0xEA1AFB91,
    data2: 0x9E28,
    data3: 0x4B86,
    data4: [0x90, 0xE9, 0x9E, 0x9F, 0x8A, 0x5E, 0xEF, 0xAF],
};

/// The taskbar list, created on first use. Translation of `GetTaskbarList()`.
fn get_taskbar_list(videodata: &VideoData, data: &WindowData) -> Result<*mut ITaskbarList3> {
    crate::sdl_assert!(data.state.with(|s| s.taskbar_button_created));
    let existing = videodata.state.with(|s| s.taskbar_list);
    if !existing.is_null() {
        return Ok(existing);
    }
    let mut taskbar_list: *mut ITaskbarList3 = std::ptr::null_mut();
    // SAFETY: the GUIDs are the taskbar list's; taskbar_list receives the
    // interface pointer.
    let ret = unsafe {
        CoCreateInstance(
            &CLSID_TASKBAR_LIST,
            std::ptr::null_mut(),
            CLSCTX_ALL,
            &IID_ITASKBAR_LIST3,
            (&mut taskbar_list as *mut *mut ITaskbarList3).cast(),
        )
    };
    if ret < 0 {
        return Err(crate::core::windows::error_from_hresult(
            Some("Unable to create taskbar list"),
            ret,
        ));
    }
    // SAFETY: taskbar_list is a live ITaskbarList3.
    let ret = unsafe { ((*(*taskbar_list).vtbl).hr_init)(taskbar_list) };
    if ret < 0 {
        // SAFETY: as above; we own the reference.
        unsafe { ((*(*taskbar_list).vtbl).release)(taskbar_list) };
        return Err(crate::core::windows::error_from_hresult(
            Some("Unable to initialize taskbar list"),
            ret,
        ));
    }
    videodata.state.with(|s| s.taskbar_list = taskbar_list);
    Ok(taskbar_list)
}

/// A window rectangle of the SDL window, in global coordinates.
fn sdl_window_rect(window: WindowID, rect_type: WindowRect) -> (i32, i32, i32, i32) {
    // Client rect, in points
    let (x, y, w, h) = with_window(window, |w| match rect_type {
        WindowRect::Current => (w.core.x, w.core.y, w.core.w, w.core.h),
        WindowRect::Windowed => (
            w.core.windowed.x,
            w.core.windowed.y,
            w.core.windowed.w,
            w.core.windowed.h,
        ),
        WindowRect::Floating => (
            w.core.floating.x,
            w.core.floating.y,
            w.core.floating.w,
            w.core.floating.h,
        ),
        WindowRect::Pending => (w.pending.x, w.pending.y, w.pending.w, w.pending.h),
    })
    .unwrap_or_default();
    let (x, y) = relative_to_global_for_window(window, x, y);
    (x, y, w, h)
}

/// The arguments to pass to `SetWindowPos()`: the window rect, including
/// the frame, in Windows coordinates (`x`, `y`, `width`, `height`). Can be
/// called before there is an HWND. Translation of
/// `WIN_AdjustWindowRectWithStyle()`.
fn adjust_window_rect_with_style(
    window: WindowID,
    style: u32,
    style_ex: u32,
    menu: bool,
    rect_type: WindowRect,
) -> Result<(i32, i32, i32, i32)> {
    let videodata = video_data();

    let (mut x, mut y, width, height) = sdl_window_rect(window, rect_type);

    /* Copy the client size in pixels into this rect structure,
    which we'll then adjust with AdjustWindowRectEx */
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: width,
        bottom: height,
    };

    /* borderless windows will have WM_NCCALCSIZE return 0 for the non-client area. When this happens, it looks like windows will send a resize message
      expanding the window client area to the previous window + chrome size, so shouldn't need to adjust the window size for the set styles.
    */
    if !window_flags(window).contains(WindowFlags::BORDERLESS) && !is_popup(window) {
        let per_monitor_v2 = videodata
            .as_deref()
            .is_some_and(super::is_per_monitor_v2_dpi_aware);
        match (per_monitor_v2, videodata.as_deref()) {
            (true, Some(videodata)) => {
                /* With per-monitor v2, the window border/titlebar size depend on the DPI, so we need to call AdjustWindowRectExForDpi instead of
                AdjustWindowRectEx. */
                let data = window_data(window);
                let frame_dpi = match (data.as_deref(), videodata.get_dpi_for_window) {
                    // SAFETY: the function was loaded with this signature.
                    (Some(data), Some(get_dpi)) => unsafe { get_dpi(data.hwnd) },
                    _ => USER_DEFAULT_SCREEN_DPI,
                };
                // (per-monitor v2 awareness needs Windows 10 1607, which has
                // AdjustWindowRectExForDpi)
                if let Some(adjust) = videodata.adjust_window_rect_ex_for_dpi {
                    // SAFETY: the function was loaded with this signature.
                    if unsafe { adjust(&mut rect, style, menu as i32, style_ex, frame_dpi) } == 0 {
                        return Err(set_error("AdjustWindowRectExForDpi()"));
                    }
                }
            }
            _ => {
                // SAFETY: rect is a valid RECT.
                if unsafe { AdjustWindowRectEx(&mut rect, style, menu as i32, style_ex) } == 0 {
                    return Err(set_error("AdjustWindowRectEx()"));
                }
            }
        }
    }

    // Final rect in Windows screen space, including the frame
    x += rect.left;
    y += rect.top;
    Ok((x, y, rect.right - rect.left, rect.bottom - rect.top))
}

/// Whether a window has a menu (`(style & WS_CHILDWINDOW) ? FALSE : (GetMenu(hwnd) != NULL)`).
fn has_menu(hwnd: HWND, style: u32) -> bool {
    // SAFETY: GetMenu accepts any window handle.
    (style & WS_CHILDWINDOW) == 0 && !unsafe { GetMenu(hwnd) }.is_null()
}

/// The frame rect of an existing window. Translation of
/// `WIN_AdjustWindowRect()`.
pub(crate) fn adjust_window_rect(
    window: WindowID,
    rect_type: WindowRect,
) -> Result<(i32, i32, i32, i32)> {
    let Some(data) = window_data(window) else {
        return Err(Error::new("Invalid window"));
    };
    let hwnd = data.hwnd;

    // SAFETY: hwnd is the window's handle.
    let (style, style_ex) = unsafe {
        (
            get_window_long_ptr(hwnd, GWL_STYLE) as u32,
            get_window_long_ptr(hwnd, GWL_EXSTYLE) as u32,
        )
    };
    let menu = has_menu(hwnd, style);
    adjust_window_rect_with_style(window, style, style_ex, menu, rect_type)
}

/// Grow a client rect into the frame rect of `hwnd`, at `frame_dpi` (0:
/// the window's) with per-monitor v2 awareness. Translation of
/// `WIN_AdjustWindowRectForHWND()`.
pub(crate) fn adjust_window_rect_for_hwnd(
    hwnd: HWND,
    rect: &mut RECT,
    frame_dpi: u32,
) -> Result<()> {
    let videodata = video_data();

    // SAFETY: hwnd is a window handle.
    let (style, style_ex) = unsafe {
        (
            get_window_long_ptr(hwnd, GWL_STYLE) as u32,
            get_window_long_ptr(hwnd, GWL_EXSTYLE) as u32,
        )
    };
    let menu = has_menu(hwnd, style);

    match videodata.as_deref() {
        Some(videodata) if super::is_per_monitor_v2_dpi_aware(videodata) => {
            // With per-monitor v2, the window border/titlebar size depend on the DPI, so we need to call AdjustWindowRectExForDpi instead of AdjustWindowRectEx.
            let mut frame_dpi = frame_dpi;
            if frame_dpi == 0 {
                frame_dpi = match videodata.get_dpi_for_window {
                    // SAFETY: the function was loaded with this signature.
                    Some(get_dpi) => unsafe { get_dpi(hwnd) },
                    None => USER_DEFAULT_SCREEN_DPI,
                };
            }
            if let Some(adjust) = videodata.adjust_window_rect_ex_for_dpi {
                // SAFETY: the function was loaded with this signature.
                if unsafe { adjust(rect, style, menu as i32, style_ex, frame_dpi) } == 0 {
                    return Err(set_error("AdjustWindowRectExForDpi()"));
                }
            }
        }
        _ => {
            // SAFETY: rect is a valid RECT.
            if unsafe { AdjustWindowRectEx(rect, style, menu as i32, style_ex) } == 0 {
                return Err(set_error("AdjustWindowRectEx()"));
            }
        }
    }
    Ok(())
}

/// Move/size the window (and its children) to one of the SDL window's
/// rects. Translation of `WIN_SetWindowPositionInternal()`.
pub(crate) fn set_window_position_internal(
    window: WindowID,
    flags: u32,
    rect_type: WindowRect,
) -> Result<()> {
    let Some(data) = window_data(window) else {
        return Err(Error::new("Invalid window"));
    };
    let hwnd = data.hwnd;
    let mut result = Ok(());

    // Figure out what the window area will be
    let top = if should_allow_topmost() && window_flags(window).contains(WindowFlags::ALWAYS_ON_TOP)
    {
        HWND_TOPMOST
    } else {
        HWND_NOTOPMOST
    };

    let (x, y, w, h) = adjust_window_rect(window, rect_type).unwrap_or_default();

    data.state.with(|s| s.expected_resize = true);
    // SAFETY: hwnd is the window's handle.
    if unsafe { SetWindowPos(hwnd, top, x, y, w, h, flags) } == 0 {
        result = Err(set_error("SetWindowPos()"));
    }
    data.state.with(|s| s.expected_resize = false);

    // Update any child windows
    let children = with_window(window, |w| w.children.clone()).unwrap_or_default();
    for child_window in children {
        if window_data(child_window).is_none() {
            // This child window is not yet fully initialized.
            continue;
        }
        if let Err(e) = set_window_position_internal(child_window, flags, WindowRect::Current) {
            result = Err(e);
        }
    }
    result
}

/// Translation of `GetEraseBackgroundModeHint()`.
fn get_erase_background_mode_hint() -> EraseBackgroundMode {
    let Some(hint) = hints::get(hints::WINDOWS_ERASE_BACKGROUND_MODE) else {
        return EraseBackgroundMode::Initial;
    };

    if hint.contains("never") {
        return EraseBackgroundMode::Never;
    }

    if hint.contains("initial") {
        return EraseBackgroundMode::Initial;
    }

    if hint.contains("always") {
        return EraseBackgroundMode::Always;
    }

    match hints::string_to_integer(Some(&hint), 1) {
        0 => EraseBackgroundMode::Never,
        1 => EraseBackgroundMode::Initial,
        2 => EraseBackgroundMode::Always,
        _ => {
            crate::log::info!(
                "GetEraseBackgroundModeHint: invalid value for SDL_HINT_WINDOWS_ERASE_BACKGROUND_MODE. Fallback to default"
            );
            EraseBackgroundMode::Initial
        }
    }
}

/// Create the driver data of a window and fill in the SDL window from the
/// HWND's state. Translation of `SetupWindowData()`.
fn setup_window_data(
    videodata: &VideoData,
    window: WindowID,
    hwnd: HWND,
    parent: HWND,
) -> Result<()> {
    // WIN_WarpCursor() jitters by +1, and remote desktop warp wobble is +/- 1
    // SAFETY: GetSystemMetrics takes an index.
    let remote_desktop_adjustment = if unsafe { GetSystemMetrics(SM_REMOTESESSION) } != 0 {
        2
    } else {
        0
    };

    // Allocate the window data
    // SAFETY: hwnd is the window's handle.
    let (hdc, hinstance) = unsafe {
        (
            GetDC(hwnd),
            get_window_long_ptr(hwnd, GWLP_HINSTANCE) as HINSTANCE,
        )
    };
    let data = Arc::new(WindowData {
        window,
        hwnd,
        parent,
        hdc,
        hinstance,
        cursor_ctrlock_rect: RECT {
            left: -remote_desktop_adjustment,
            top: 0,
            right: 1 + remote_desktop_adjustment,
            bottom: 1,
        },
        hint_erase_background_mode: get_erase_background_mode_hint(),
        copybits_flag: if hints::get_bool("SDL_WINDOW_RETAIN_CONTENT", false) {
            0
        } else {
            SWP_NOCOPYBITS
        },
        state: Shared::new(WindowState {
            mdc: std::ptr::null_mut(),
            hbm: std::ptr::null_mut(),
            bits: std::ptr::null_mut(),
            bits_pitch: 0,
            bits_h: 0,
            hicon: std::ptr::null_mut(),
            wndproc: None,
            keyboard_hook: std::ptr::null_mut(),
            mouse_button_flags: usize::MAX,
            last_pointer_update: -1,
            high_surrogate: 0,
            initializing: true,
            expected_resize: false,
            in_border_change: false,
            in_title_click: false,
            focus_click_pending: 0,
            postpone_clipcursor: false,
            clipcursor_queued: false,
            windowed_mode_was_maximized: false,
            in_window_deactivation: false,
            force_ws_maximizebox: false,
            disable_move_size_events: false,
            showing_window: false,
            in_modal_loop: 0,
            last_modal_width: 0,
            last_modal_height: 0,
            initial_size_rect: RECT::default(),
            cursor_clipped_rect: RECT::default(),
            mouse_tracked: false,
            destroy_parent_with_window: false,
            icm_file_name: None,
            taskbar_button_created: false,
            drop_target: std::ptr::null_mut(),
            egl_surface: std::ptr::null_mut(),
        }),
    });

    // Associate the data with the window
    let prop = utf8_to_wide(WINDOW_DATA_PROP);
    // SAFETY: the property holds a pointer to the data, which stays alive
    // until cleanup_window_data() removes the property.
    if unsafe { SetPropW(hwnd, prop.as_ptr(), Arc::as_ptr(&data) as *mut c_void) } == 0 {
        // SAFETY: hdc came from GetDC(hwnd).
        unsafe { ReleaseDC(hwnd, hdc) };
        return Err(set_error("SetProp() failed"));
    }

    with_window(window, |w| w.internal = Some(Box::new(data.clone())))?;

    let flags = window_flags(window);
    // Set up the window proc function
    if flags.contains(WindowFlags::EXTERNAL) {
        // SAFETY: hwnd is a window handle; a window procedure is stored in
        // GWLP_WNDPROC.
        unsafe {
            let current = get_window_long_ptr(hwnd, GWLP_WNDPROC);
            let wndproc: WNDPROC = std::mem::transmute::<isize, WNDPROC>(current);
            data.state.with(|s| s.wndproc = wndproc);
            let ours = window_proc as *const () as isize;
            if current != ours {
                set_window_long_ptr(hwnd, GWLP_WNDPROC, ours);
            }
        }
    } else {
        // We set up our window proc function at window creation.
        // If someone has set hooks to modify it, leave it alone.
    }

    // Fill in the SDL window with the window state
    {
        // SAFETY: hwnd is the window's handle.
        let style = unsafe { get_window_long_ptr(hwnd, GWL_STYLE) } as u32;
        let _ = with_window(window, |w| {
            let flags = &mut w.core.flags;
            flags.set(WindowFlags::HIDDEN, (style & WS_VISIBLE) == 0);
            flags.set(WindowFlags::BORDERLESS, (style & WS_POPUP) != 0);
            if (style & WS_THICKFRAME) != 0 {
                *flags |= WindowFlags::RESIZABLE;
            } else if (style & WS_POPUP) == 0 {
                *flags &= !WindowFlags::RESIZABLE;
            }
            flags.set(WindowFlags::MAXIMIZED, (style & WS_MAXIMIZE) != 0);
            flags.set(WindowFlags::MINIMIZED, (style & WS_MINIMIZE) != 0);
        });
    }
    if !window_flags(window).contains(WindowFlags::MINIMIZED) {
        let mut rect = RECT::default();
        // SAFETY: rect is a valid RECT.
        if unsafe { GetClientRect(hwnd, &mut rect) } != 0 && window_rect_valid(&rect) {
            let w = rect.right;
            let h = rect.bottom;

            let resize = with_window(window, |win| {
                if win.flags().contains(WindowFlags::EXTERNAL) {
                    win.core.floating.w = w;
                    win.core.windowed.w = w;
                    win.core.w = w;
                    win.core.floating.h = h;
                    win.core.windowed.h = h;
                    win.core.h = h;
                    false
                } else if (win.core.windowed.w != 0 && win.core.windowed.w != w)
                    || (win.core.windowed.h != 0 && win.core.windowed.h != h)
                {
                    true
                } else {
                    win.core.w = w;
                    win.core.h = h;
                    false
                }
            })
            .unwrap_or(false);
            if resize {
                // We tried to create a window larger than the desktop and Windows didn't allow it.  Override!
                // Figure out what the window area will be
                let (x, y, w, h) =
                    adjust_window_rect(window, WindowRect::Floating).unwrap_or_default();
                data.state.with(|s| s.expected_resize = true);
                // SAFETY: hwnd is the window's handle.
                unsafe {
                    SetWindowPos(
                        hwnd,
                        std::ptr::null_mut(),
                        x,
                        y,
                        w,
                        h,
                        data.copybits_flag | SWP_NOZORDER | SWP_NOOWNERZORDER | SWP_NOACTIVATE,
                    )
                };
                data.state.with(|s| s.expected_resize = false);
            }
        }
    }
    if !window_flags(window).contains(WindowFlags::MINIMIZED) {
        let mut point = POINT { x: 0, y: 0 };
        // SAFETY: point is a valid POINT.
        if unsafe { ClientToScreen(hwnd, &mut point) } != 0 {
            let _ = with_window(window, |w| {
                if w.flags().contains(WindowFlags::EXTERNAL) {
                    w.core.floating.x = point.x;
                    w.core.windowed.x = point.x;
                    w.core.floating.y = point.y;
                    w.core.windowed.y = point.y;
                }
                w.core.x = point.x;
                w.core.y = point.y;
            });
        }
    }

    update_window_icc_profile(window, false);

    // SAFETY: GetFocus has no preconditions.
    if unsafe { GetFocus() } == hwnd {
        let _ = with_window(window, |w| w.core.flags |= WindowFlags::INPUT_FOCUS);
        let _ = keyboard::set_keyboard_focus(Some(window));
        update_clip_cursor(window);
    }

    if window_flags(window).contains(WindowFlags::ALWAYS_ON_TOP) {
        set_window_always_on_top(window, true);
    } else {
        set_window_always_on_top(window, false);
    }

    // Enable multi-touch
    if let Some(register_touch_window) = videodata.register_touch_window {
        const TWF_FINETOUCH: u32 = 1;
        const TWF_WANTPALM: u32 = 2;
        // SAFETY: the function was loaded with this signature.
        unsafe { register_touch_window(hwnd, TWF_FINETOUCH | TWF_WANTPALM) };
    }

    if !parent.is_null() && window_parent(window).is_none() {
        data.state.with(|s| s.destroy_parent_with_window = true);
    }

    data.state.with(|s| s.initializing = false);

    if window_flags(window).contains(WindowFlags::EXTERNAL) {
        // Query the title from the existing window
        // SAFETY: hwnd is a window handle; title holds titleLen + 1 units.
        let title = unsafe {
            let title_len = GetWindowTextLengthW(hwnd);
            let mut title = vec![0u16; title_len.max(0) as usize + 1];
            let title_len = GetWindowTextW(hwnd, title.as_mut_ptr(), title_len + 1);
            if title_len > 0 {
                Some(wide_to_utf8(&title[..title_len as usize]))
            } else {
                None
            }
        };
        if let Some(title) = title {
            let _ = with_window(window, |w| w.title = Some(title));
        }
    }

    if let Ok(props) = with_window(window, |w| w.properties()) {
        let _ = props.set(PROP_WINDOW_WIN32_HWND_POINTER, hwnd as i64);
        let _ = props.set(PROP_WINDOW_WIN32_HDC_POINTER, hdc as i64);
        let _ = props.set(PROP_WINDOW_WIN32_INSTANCE_POINTER, hinstance as i64);
    }

    // All done!
    Ok(())
}

/// Release a window's driver data and destroy the HWND (or give an external
/// one its window procedure back). Translation of `CleanupWindowData()`.
fn cleanup_window_data(window: WindowID) {
    if let Some(data) = window_data(window) {
        if !data.state.with(|s| s.drop_target).is_null() {
            if let Some(videodata) = video_data() {
                accept_drag_and_drop(&videodata, window, false);
            }
        }
        let (keyboard_hook, hicon, wndproc) = data.state.with(|s| {
            s.icm_file_name = None;
            (
                std::mem::replace(&mut s.keyboard_hook, std::ptr::null_mut()),
                std::mem::replace(&mut s.hicon, std::ptr::null_mut()),
                s.wndproc,
            )
        });
        let prop = utf8_to_wide(WINDOW_DATA_PROP);
        // SAFETY: the handles belong to this window and are released once.
        unsafe {
            if !keyboard_hook.is_null() {
                UnhookWindowsHookEx(keyboard_hook);
            }
            if !hicon.is_null() {
                DestroyIcon(hicon);
            }
            ReleaseDC(data.hwnd, data.hdc);
            RemovePropW(data.hwnd, prop.as_ptr());
        }
        if !window_flags(window).contains(WindowFlags::EXTERNAL) {
            let destroy_parent = data.state.with(|s| s.destroy_parent_with_window);
            // SAFETY: the windows are ours to destroy.
            unsafe {
                DestroyWindow(data.hwnd);
                if destroy_parent && !data.parent.is_null() {
                    DestroyWindow(data.parent);
                }
            }
        } else {
            // Restore any original event handler...
            if let Some(wndproc) = wndproc {
                // SAFETY: wndproc was the window's procedure.
                unsafe { set_window_long_ptr(data.hwnd, GWLP_WNDPROC, wndproc as usize as isize) };
            }
        }
    }
    let _ = with_window(window, |w| w.internal = None);
}

/// Possibly clamp popup windows to the output borders. Translation of
/// `WIN_ConstrainPopup()`.
fn constrain_popup(window: WindowID, output_to_pending: bool) {
    if !is_popup(window) {
        return;
    }
    let Ok((mut abs_x, mut abs_y, width, height, constrain, parent)) = with_window(window, |w| {
        (
            if w.core.last_position_pending {
                w.pending.x
            } else {
                w.core.floating.x
            },
            if w.core.last_position_pending {
                w.pending.y
            } else {
                w.core.floating.y
            },
            if w.core.last_size_pending {
                w.pending.w
            } else {
                w.core.floating.w
            },
            if w.core.last_size_pending {
                w.pending.h
            } else {
                w.core.floating.h
            },
            w.constrain_popup,
            w.parent,
        )
    }) else {
        return;
    };
    let mut offset_x = 0;
    let mut offset_y = 0;

    if constrain {
        // Calculate the total offset from the parents
        let mut w = parent;
        while let Some(p) = w.filter(|&p| is_popup(p)) {
            let (x, y, next) =
                with_window(p, |p| (p.core.x, p.core.y, p.parent)).unwrap_or((0, 0, None));
            offset_x += x;
            offset_y += y;
            w = next;
        }

        let toplevel = w;
        let (x, y) = toplevel
            .and_then(|t| with_window(t, |t| (t.core.x, t.core.y)).ok())
            .unwrap_or((0, 0));
        offset_x += x;
        offset_y += y;
        abs_x += offset_x;
        abs_y += offset_y;

        // Constrain the popup window to the display of the toplevel parent
        let display_id = toplevel
            .and_then(|t| display_for_window(t).ok())
            .unwrap_or(0);
        let rect = crate::video::display::display_bounds(display_id).unwrap_or_default();
        if abs_x + width > rect.x + rect.w {
            abs_x -= (abs_x + width) - (rect.x + rect.w);
        }
        if abs_y + height > rect.y + rect.h {
            abs_y -= (abs_y + height) - (rect.y + rect.h);
        }
        abs_x = abs_x.max(rect.x);
        abs_y = abs_y.max(rect.y);
    }

    let _ = with_window(window, |w| {
        if output_to_pending {
            w.pending.x = abs_x - offset_x;
            w.pending.y = abs_y - offset_y;
            w.pending.w = width;
            w.pending.h = height;
        } else {
            w.core.floating.x = abs_x - offset_x;
            w.core.floating.y = abs_y - offset_y;
            w.core.floating.w = width;
            w.core.floating.h = height;
        }
    });
}

/// Record the keyboard focus in the toplevel window, and take it if asked.
/// Translation of `WIN_SetKeyboardFocus()`.
fn set_keyboard_focus(window: WindowID, set_active_focus: bool) {
    let mut toplevel = window;

    // Find the topmost parent
    while is_popup(toplevel) {
        match window_parent(toplevel) {
            Some(parent) => toplevel = parent,
            None => break,
        }
    }

    let _ = with_window(toplevel, |t| t.keyboard_focus = Some(window));

    let (is_hiding, is_destroying) =
        with_window(window, |w| (w.is_hiding, w.core.is_destroying)).unwrap_or((true, true));
    if set_active_focus && !is_hiding && !is_destroying {
        let _ = keyboard::set_keyboard_focus(Some(window));
    }
}

/// A handle from a pointer property: a number (or `None`).
fn handle_property(props: &Properties, name: &str) -> Option<HWND> {
    props
        .get_number(name)
        .filter(|&h| h != 0)
        .map(|h| h as isize as HWND)
}

/// Create the HWND for a window (or adopt the one in
/// `PROP_WINDOW_CREATE_WIN32_HWND_POINTER`). Translation of
/// `WIN_CreateWindow()`.
pub(crate) fn create_window(
    videodata: &VideoData,
    window: WindowID,
    create_props: &Properties,
) -> Result<()> {
    let mut hwnd = handle_property(create_props, PROP_WINDOW_CREATE_WIN32_HWND_POINTER)
        .or_else(|| handle_property(create_props, "sdl2-compat.external_window"));
    let mut parent: HWND = std::ptr::null_mut();
    if let Some(hwnd) = hwnd {
        with_window(window, |w| w.core.flags |= WindowFlags::EXTERNAL)?;

        setup_window_data(videodata, window, hwnd, parent)?;
    } else {
        let mut style = STYLE_BASIC;
        let mut style_ex = 0;

        let flags = window_flags(window);
        if flags.contains(WindowFlags::UTILITY) {
            let empty = utf8_to_wide("");
            // SAFETY: the class is registered (register_app()); the
            // strings are NUL-terminated.
            parent = unsafe {
                CreateWindowExW(
                    0,
                    app_name().as_ptr(),
                    empty.as_ptr(),
                    STYLE_BASIC,
                    0,
                    0,
                    32,
                    32,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    app_instance(),
                    std::ptr::null(),
                )
            };
        } else if let Some(p) = window_parent(window) {
            parent = window_data(p).map_or(std::ptr::null_mut(), |d| d.hwnd);
        }

        style |= get_window_style(window);
        style_ex |= create_props
            .get_number(PROP_WINDOW_CREATE_WIN32_STYLE_EX_NUMBER)
            .map_or_else(|| get_window_style_ex(window), |v| v as u32);

        // Figure out what the window area will be
        constrain_popup(window, false);
        let (x, y, w, h) =
            adjust_window_rect_with_style(window, style, style_ex, false, WindowRect::Floating)
                .unwrap_or_default();

        let empty = utf8_to_wide("");
        // SAFETY: the class is registered (register_app()); the strings are
        // NUL-terminated.
        let new_hwnd = unsafe {
            CreateWindowExW(
                style_ex,
                app_name().as_ptr(),
                empty.as_ptr(),
                style,
                x,
                y,
                w,
                h,
                parent,
                std::ptr::null_mut(),
                app_instance(),
                std::ptr::null(),
            )
        };
        if new_hwnd.is_null() {
            return Err(set_error("Couldn't create window"));
        }
        hwnd = Some(new_hwnd);
        let hwnd = new_hwnd;

        update_dark_mode_for_hwnd(hwnd);

        pump_events_for_hwnd(hwnd);

        if let Err(e) = setup_window_data(videodata, window, hwnd, parent) {
            // SAFETY: the windows are ours to destroy.
            unsafe {
                DestroyWindow(hwnd);
                if !parent.is_null() {
                    DestroyWindow(parent);
                }
            }
            return Err(e);
        }

        // Ensure that the IME isn't active on the new window until explicitly requested.
        let _ = super::keyboard::stop_text_input(videodata, window);

        // Inform Windows of the frame change so we can respond to WM_NCCALCSIZE
        // SAFETY: hwnd is the window's handle.
        unsafe {
            SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                0,
                0,
                0,
                0,
                SWP_FRAMECHANGED
                    | SWP_NOSIZE
                    | SWP_NOMOVE
                    | SWP_NOZORDER
                    | SWP_NOOWNERZORDER
                    | SWP_NOACTIVATE,
            )
        };

        if window_flags(window).contains(WindowFlags::MINIMIZED) {
            /* TODO: We have to clear SDL_WINDOW_HIDDEN here to ensure the window flags match the window state. The
               window is already shown after this and windows with WS_MINIMIZE do not generate a WM_SHOWWINDOW. This
               means you can't currently create a window that is initially hidden and is minimized when shown.
            */
            with_window(window, |w| w.core.flags &= !WindowFlags::HIDDEN)?;
            // SAFETY: hwnd is the window's handle.
            unsafe { ShowWindow(hwnd, SW_SHOWMINNOACTIVE) };
        }
    }
    let hwnd = hwnd.unwrap_or(std::ptr::null_mut());

    // FIXME: does not work on all hardware configurations with different renders (i.e. hybrid GPUs)
    if window_flags(window).contains(WindowFlags::TRANSPARENT) {
        if let Some(enable_blur) = videodata.dwm_enable_blur_behind_window {
            /* The region indicates which part of the window will be blurred and rest will be transparent. This
               is because the alpha value of the window will be used for non-blurred areas
               We can use (-1, -1, 0, 0) boundary to make sure no pixels are being blurred
            */
            // SAFETY: the function was loaded with this signature; the
            // region is deleted after use.
            unsafe {
                let rgn = CreateRectRgn(-1, -1, 0, 0);
                let bb = DwmBlurBehind {
                    flags: DWM_BB_ENABLE | DWM_BB_BLURREGION,
                    enable: 1,
                    blur_region: rgn,
                    transition_on_maxed: 0,
                };
                enable_blur(hwnd, &bb);
                DeleteObject(rgn);
            }
        }
    }

    if let Some(share_hwnd) = handle_property(
        create_props,
        PROP_WINDOW_CREATE_WIN32_PIXEL_FORMAT_HWND_POINTER,
    ) {
        let hdc = window_data(window).map_or(std::ptr::null_mut(), |d| d.hdc);
        // SAFETY: share_hwnd is the app's window; the DC is released after
        // the query; pfd is sized.
        let ok = unsafe {
            let share_hdc = GetDC(share_hwnd);
            let pixel_format = GetPixelFormat(share_hdc);
            let mut pfd: PIXELFORMATDESCRIPTOR = std::mem::zeroed();
            DescribePixelFormat(
                share_hdc,
                pixel_format,
                size_of::<PIXELFORMATDESCRIPTOR>() as u32,
                &mut pfd,
            );
            ReleaseDC(share_hwnd, share_hdc);

            SetPixelFormat(hdc, pixel_format, &pfd) != 0
        };
        if !ok {
            let e = set_error("SetPixelFormat()");
            destroy_window(window);
            return Err(e);
        }
    } else {
        if !window_flags(window).contains(WindowFlags::OPENGL) {
            return Ok(());
        }

        // The rest of this macro mess is for OpenGL or OpenGL ES windows
        let config = gl_config();
        if (config.profile_mask == GL_CONTEXT_PROFILE_ES
            || hints::get_bool(hints::VIDEO_FORCE_EGL, false))
            && videodata
                .wgl_data()
                .is_none_or(|wgl| win_gl_use_egl(&wgl, &config))
        {
            if let Err(e) = videodata.win_gles_setup_window(window) {
                destroy_window(window);
                return Err(e);
            }
            return Ok(());
        }

        if let Err(e) = videodata.win_gl_setup_window(window) {
            destroy_window(window);
            return Err(e);
        }
    }

    Ok(())
}

/// Translation of `WIN_SetWindowTitle()`.
pub(crate) fn set_window_title(window: WindowID) {
    let Some(data) = window_data(window) else {
        return;
    };
    let title = with_window(window, |w| w.title.clone())
        .ok()
        .flatten()
        .unwrap_or_default();
    let title = utf8_to_wide(&title);
    // SAFETY: title is NUL-terminated.
    unsafe { SetWindowTextW(data.hwnd, title.as_ptr()) };
}

/// Set the window icon from an ARGB8888 surface. Translation of
/// `WIN_SetWindowIcon()`.
pub(crate) fn set_window_icon(window: WindowID, icon: &Surface<'static>) -> Result<()> {
    let Some(data) = window_data(window) else {
        return Err(Error::new("Invalid window"));
    };
    let hwnd = data.hwnd;

    // Create temporary buffer for ICONIMAGE structure
    const _: () = assert!(size_of::<BITMAPINFOHEADER>() == 40);
    let (w, h) = (icon.width(), icon.height());
    let mask_len = (h * (w + 7) / 8) as usize;
    let icon_len = size_of::<BITMAPINFOHEADER>() + (h * w) as usize * size_of::<u32>() + mask_len;
    let mut icon_bmp = vec![0u8; icon_len];

    // Write the BITMAPINFO header
    let bmi = BITMAPINFOHEADER {
        biSize: (size_of::<BITMAPINFOHEADER>() as u32).to_le(),
        biWidth: w.to_le(),
        biHeight: (h * 2).to_le(),
        biPlanes: 1u16.to_le(),
        biBitCount: 32u16.to_le(),
        biCompression: BI_RGB.to_le(),
        biSizeImage: ((h * w) as u32 * size_of::<u32>() as u32).to_le(),
        biXPelsPerMeter: 0,
        biYPelsPerMeter: 0,
        biClrUsed: 0,
        biClrImportant: 0,
    };
    // SAFETY: BITMAPINFOHEADER is plain data of the size copied.
    let header = unsafe {
        std::slice::from_raw_parts(
            (&bmi as *const BITMAPINFOHEADER).cast::<u8>(),
            size_of::<BITMAPINFOHEADER>(),
        )
    };
    icon_bmp[..header.len()].copy_from_slice(header);

    // Write the pixels upside down into the bitmap buffer
    crate::sdl_assert!(icon.format() == crate::video::PixelFormat::ARGB8888);
    let pixels = icon.pixels().unwrap_or(&[]);
    let pitch = icon.pitch() as usize;
    let row_len = w as usize * size_of::<u32>();
    let mut dst = size_of::<BITMAPINFOHEADER>();
    for y in (0..h as usize).rev() {
        if let Some(src) = pixels.get(y * pitch..y * pitch + row_len) {
            icon_bmp[dst..dst + row_len].copy_from_slice(src);
        }
        dst += row_len;
    }

    // Write the mask
    for b in &mut icon_bmp[icon_len - mask_len..] {
        *b = 0xFF;
    }

    // SAFETY: icon_bmp holds an ICONIMAGE of icon_len bytes.
    let hicon =
        unsafe { CreateIconFromResource(icon_bmp.as_ptr(), icon_len as u32, 1, 0x00030000) };

    if hicon.is_null() {
        // SAFETY: GetLastError has no preconditions.
        let error = unsafe { GetLastError() };
        return Err(Error::new(format!(
            "SetWindowIcon() failed, error {error:08X}"
        )));
    }

    let old = data.state.with(|s| std::mem::replace(&mut s.hicon, hicon));
    // SAFETY: the old icon is ours; hwnd is the window's handle.
    unsafe {
        if !old.is_null() {
            DestroyIcon(old);
        }

        // Set the icon for the window
        SendMessageW(hwnd, WM_SETICON, ICON_SMALL as WPARAM, hicon as LPARAM);

        // Set the icon in the task manager (should we do this?)
        SendMessageW(hwnd, WM_SETICON, ICON_BIG as WPARAM, hicon as LPARAM);
    }
    Ok(())
}

/// Translation of `WIN_SetWindowPosition()`.
pub(crate) fn set_window_position(window: WindowID) -> Result<()> {
    /* HighDPI support: removed SWP_NOSIZE. If the move results in a DPI change, we need to allow
     * the window to resize (e.g. AdjustWindowRectExForDpi frame sizes are different).
     */
    let flags = window_flags(window);
    if !flags.contains(WindowFlags::FULLSCREEN) {
        if !flags.intersects(WindowFlags::MAXIMIZED | WindowFlags::MINIMIZED) {
            constrain_popup(window, true);
            let copybits = window_data(window).map_or(0, |d| d.copybits_flag);
            return set_window_position_internal(
                window,
                copybits | SWP_NOZORDER | SWP_NOOWNERZORDER | SWP_NOSIZE | SWP_NOACTIVATE,
                WindowRect::Pending,
            );
        }
    } else {
        return crate::video::core::update_fullscreen_mode(window, FullscreenOp::Enter, true);
    }

    Ok(())
}

/// Translation of `WIN_SetWindowSize()`.
pub(crate) fn set_window_size(window: WindowID) {
    if !window_flags(window).intersects(WindowFlags::FULLSCREEN | WindowFlags::MAXIMIZED) {
        let copybits = window_data(window).map_or(0, |d| d.copybits_flag);
        let _ = set_window_position_internal(
            window,
            copybits | SWP_NOMOVE | SWP_NOZORDER | SWP_NOOWNERZORDER | SWP_NOACTIVATE,
            WindowRect::Pending,
        );
    } else {
        // Can't resize the window
        let _ = with_window(window, |w| w.core.last_size_pending = false);
    }
}

/// The frame sizes around the client area: (top, left, bottom, right).
/// Translation of `WIN_GetWindowBordersSize()`.
pub(crate) fn get_window_borders_size(window: WindowID) -> Result<(i32, i32, i32, i32)> {
    let Some(data) = window_data(window) else {
        return Err(Error::new("Invalid window"));
    };
    let hwnd = data.hwnd;
    let mut rc_client = RECT::default();
    let mut rc_window = RECT::default();

    let last_error = || {
        // SAFETY: GetLastError has no preconditions.
        unsafe { GetLastError() }
    };

    /* rcClient stores the size of the inner window, while rcWindow stores the outer size relative to the top-left
     * screen position; so the top/left values of rcClient are always {0,0} and bottom/right are {height,width} */
    // SAFETY: the rects and points are valid.
    unsafe {
        if GetClientRect(hwnd, &mut rc_client) == 0 {
            return Err(Error::new(format!(
                "GetClientRect() failed, error {:08X}",
                last_error()
            )));
        }

        if GetWindowRect(hwnd, &mut rc_window) == 0 {
            return Err(Error::new(format!(
                "GetWindowRect() failed, error {:08X}",
                last_error()
            )));
        }

        /* convert the top/left values to make them relative to
         * the window; they will end up being slightly negative */
        let mut pt_diff = POINT {
            x: rc_window.left,
            y: rc_window.top,
        };

        if ScreenToClient(hwnd, &mut pt_diff) == 0 {
            return Err(Error::new(format!(
                "ScreenToClient() failed, error {:08X}",
                last_error()
            )));
        }

        rc_window.top = pt_diff.y;
        rc_window.left = pt_diff.x;

        /* convert the bottom/right values to make them relative to the window,
         * these will be slightly bigger than the inner width/height */
        pt_diff.y = rc_window.bottom;
        pt_diff.x = rc_window.right;

        if ScreenToClient(hwnd, &mut pt_diff) == 0 {
            return Err(Error::new(format!(
                "ScreenToClient() failed, error {:08X}",
                last_error()
            )));
        }

        rc_window.bottom = pt_diff.y;
        rc_window.right = pt_diff.x;
    }

    /* Now that both the inner and outer rects use the same coordinate system we can subtract them to get the border size.
     * Keep in mind that the top/left coordinates of rcWindow are negative because the border lies slightly before {0,0},
     * so switch them around because SDL3 wants them in positive. */
    Ok((
        rc_client.top - rc_window.top,
        rc_client.left - rc_window.left,
        rc_window.bottom - rc_client.bottom,
        rc_window.right - rc_client.right,
    ))
}

/// Translation of `WIN_GetWindowSizeInPixels()`.
pub(crate) fn get_window_size_in_pixels(window: WindowID) -> Option<(i32, i32)> {
    let data = window_data(window)?;
    let mut rect = RECT::default();

    // SAFETY: rect is a valid RECT.
    if unsafe { GetClientRect(data.hwnd, &mut rect) } != 0 && window_rect_valid(&rect) {
        return Some((rect.right, rect.bottom));
    }
    with_window(window, |w| {
        if w.core.last_pixel_w != 0 && w.core.last_pixel_h != 0 {
            (w.core.last_pixel_w, w.core.last_pixel_h)
        } else {
            // Probably created minimized, use the restored size
            (w.core.floating.w, w.core.floating.h)
        }
    })
    .ok()
}

/// Translation of `WIN_ShowWindow()`.
pub(crate) fn show_window(window: WindowID) {
    let Some(data) = window_data(window) else {
        return;
    };
    let hwnd = data.hwnd;

    let mut b_activate = hints::get_bool(hints::WINDOW_ACTIVATE_WHEN_SHOWN, true);

    if is_popup(window) {
        // Update our position in case our parent moved while we were hidden
        let _ = set_window_position(window);
    }

    // If the window isn't borderless and will be fullscreen, use the borderless style to hide the initial borders.
    let pending_fullscreen = with_window(window, |w| {
        w.core.pending_flags.contains(WindowFlags::FULLSCREEN)
    })
    .unwrap_or(false);
    if pending_fullscreen && !window_flags(window).contains(WindowFlags::BORDERLESS) {
        let _ = with_window(window, |w| w.core.flags |= WindowFlags::BORDERLESS);
        // SAFETY: hwnd is the window's handle.
        let mut style = unsafe { get_window_long_ptr(hwnd, GWL_STYLE) } as u32;
        style &= !STYLE_MASK;
        style |= get_window_style(window);
        // SAFETY: as above.
        unsafe { set_window_long_ptr(hwnd, GWL_STYLE, style as i32 as isize) };
        let _ = with_window(window, |w| w.core.flags &= !WindowFlags::BORDERLESS);
    }
    // SAFETY: hwnd is the window's handle.
    let style = unsafe { get_window_long_ptr(hwnd, GWL_EXSTYLE) } as u32;
    if (style & WS_EX_NOACTIVATE) != 0 {
        b_activate = false;
    }

    data.state.with(|s| s.showing_window = true);
    // SAFETY: hwnd is the window's handle.
    unsafe {
        if b_activate {
            ShowWindow(hwnd, SW_SHOW);
        } else {
            // Use SetWindowPos instead of ShowWindow to avoid activating the parent window if this is a child window
            SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                0,
                0,
                0,
                0,
                data.copybits_flag
                    | SWP_SHOWWINDOW
                    | SWP_NOACTIVATE
                    | SWP_NOMOVE
                    | SWP_NOSIZE
                    | SWP_NOZORDER
                    | SWP_NOOWNERZORDER,
            );
        }
    }
    data.state.with(|s| s.showing_window = false);

    let flags = window_flags(window);
    if flags.contains(WindowFlags::POPUP_MENU)
        && !flags.contains(WindowFlags::NOT_FOCUSABLE)
        && b_activate
    {
        set_keyboard_focus(window, true);
    }
    if flags.contains(WindowFlags::MODAL) {
        let _ = set_window_modal(window, true);
    }
}

/// Translation of `WIN_HideWindow()`.
pub(crate) fn hide_window(window: WindowID) {
    let Some(data) = window_data(window) else {
        return;
    };
    let hwnd = data.hwnd;

    if window_flags(window).contains(WindowFlags::MODAL) {
        let _ = set_window_modal(window, false);
    }

    // SAFETY: hwnd is the window's handle.
    unsafe { ShowWindow(hwnd, SW_HIDE) };

    // Transfer keyboard focus back to the parent from a grabbing popup.
    let flags = window_flags(window);
    if flags.contains(WindowFlags::POPUP_MENU) && !flags.contains(WindowFlags::NOT_FOCUSABLE) {
        if let Ok((Some(new_focus), set_focus)) = should_relinquish_popup_focus(window) {
            set_keyboard_focus(new_focus, set_focus);
        }
    }
}

/// Translation of `WIN_RaiseWindow()`.
pub(crate) fn raise_window(window: WindowID) {
    /* If desired, raise the window more forcefully.
     * Technique taken from http://stackoverflow.com/questions/916259/ .
     * Specifically, http://stackoverflow.com/a/34414846 .
     *
     * The issue is that Microsoft has gone through a lot of trouble to make it
     * nearly impossible to programmatically move a window to the foreground,
     * for "security" reasons. Apparently, the following song-and-dance gets
     * around their objections. */
    let b_force = hints::get_bool(hints::FORCE_RAISEWINDOW, false);
    let b_activate = hints::get_bool(hints::WINDOW_ACTIVATE_WHEN_RAISED, true);

    let mut dw_my_id: u32 = 0;
    let mut dw_cur_id: u32 = 0;

    let Some(data) = window_data(window) else {
        return;
    };
    let hwnd = data.hwnd;
    // SAFETY: hwnd is the window's handle; the thread IDs are ours and the
    // foreground window's.
    unsafe {
        if b_force {
            let h_cur_wnd = GetForegroundWindow();
            dw_my_id = GetCurrentThreadId();
            dw_cur_id = GetWindowThreadProcessId(h_cur_wnd, std::ptr::null_mut());
            ShowWindow(hwnd, SW_RESTORE);
            AttachThreadInput(dw_cur_id, dw_my_id, 1);
            SetWindowPos(hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOSIZE | SWP_NOMOVE);
            if !should_allow_topmost() || !window_flags(window).contains(WindowFlags::ALWAYS_ON_TOP)
            {
                SetWindowPos(hwnd, HWND_NOTOPMOST, 0, 0, 0, 0, SWP_NOSIZE | SWP_NOMOVE);
            }
        }
    }
    if b_activate {
        // SAFETY: hwnd is the window's handle.
        unsafe { SetForegroundWindow(hwnd) };
        let flags = window_flags(window);
        if flags.contains(WindowFlags::POPUP_MENU) && !flags.contains(WindowFlags::NOT_FOCUSABLE) {
            set_keyboard_focus(window, window_parent(window) == keyboard::keyboard_focus());
        }
    } else {
        // SAFETY: hwnd is the window's handle.
        unsafe {
            SetWindowPos(
                hwnd,
                HWND_TOP,
                0,
                0,
                0,
                0,
                data.copybits_flag | SWP_NOMOVE | SWP_NOSIZE | SWP_NOOWNERZORDER | SWP_NOACTIVATE,
            )
        };
    }
    if b_force {
        // SAFETY: as above.
        unsafe {
            AttachThreadInput(dw_cur_id, dw_my_id, 0);
            SetFocus(hwnd);
            SetActiveWindow(hwnd);
        }
    }
}

/// Translation of `WIN_MaximizeWindow()`.
pub(crate) fn maximize_window(window: WindowID) {
    let Some(data) = window_data(window) else {
        return;
    };

    if !window_flags(window).contains(WindowFlags::FULLSCREEN) {
        let hwnd = data.hwnd;
        data.state.with(|s| s.expected_resize = true);
        // SAFETY: hwnd is the window's handle.
        unsafe { ShowWindow(hwnd, SW_MAXIMIZE) };
        data.state.with(|s| s.expected_resize = false);

        /* Clamp the maximized window size to the max window size.
         * This is automatic if maximizing from the window controls.
         */
        let clamp = with_window(window, |w| {
            if w.core.max_w != 0 || w.core.max_h != 0 {
                w.core.windowed.w = if w.core.max_w != 0 {
                    w.core.w.min(w.core.max_w)
                } else {
                    w.core.windowed.w
                };
                w.core.windowed.h = if w.core.max_h != 0 {
                    w.core.h.min(w.core.max_h)
                } else {
                    w.core.windowed.h
                };
                true
            } else {
                false
            }
        })
        .unwrap_or(false);
        if clamp {
            let (fx, fy, fw, fh) =
                adjust_window_rect(window, WindowRect::Windowed).unwrap_or_default();

            data.state.with(|s| s.expected_resize = true);
            // SAFETY: hwnd is the window's handle.
            unsafe {
                SetWindowPos(
                    hwnd,
                    HWND_TOP,
                    fx,
                    fy,
                    fw,
                    fh,
                    data.copybits_flag | SWP_NOMOVE | SWP_NOOWNERZORDER | SWP_NOACTIVATE,
                )
            };
            data.state.with(|s| s.expected_resize = false);
        }
    } else {
        data.state.with(|s| s.windowed_mode_was_maximized = true);
    }
}

/// Translation of `WIN_MinimizeWindow()`.
pub(crate) fn minimize_window(window: WindowID) {
    if let Some(data) = window_data(window) {
        // SAFETY: hwnd is the window's handle.
        unsafe { ShowWindow(data.hwnd, SW_MINIMIZE) };
    }
}

/// Translation of `WIN_SetWindowBordered()`.
pub(crate) fn set_window_bordered(window: WindowID, _bordered: bool) {
    let Some(data) = window_data(window) else {
        return;
    };
    let hwnd = data.hwnd;

    // SAFETY: hwnd is the window's handle.
    let mut style = unsafe { get_window_long_ptr(hwnd, GWL_STYLE) } as u32;
    style &= !STYLE_MASK;
    style |= get_window_style(window);

    data.state.with(|s| s.in_border_change = true);
    // SAFETY: as above.
    unsafe { set_window_long_ptr(hwnd, GWL_STYLE, style as i32 as isize) };
    let _ = set_window_position_internal(
        window,
        data.copybits_flag | SWP_FRAMECHANGED | SWP_NOZORDER | SWP_NOOWNERZORDER | SWP_NOACTIVATE,
        WindowRect::Current,
    );
    data.state.with(|s| s.in_border_change = false);
}

/// Translation of `WIN_SetWindowResizable()`.
pub(crate) fn set_window_resizable(window: WindowID, _resizable: bool) {
    let Some(data) = window_data(window) else {
        return;
    };
    let hwnd = data.hwnd;

    // SAFETY: hwnd is the window's handle.
    let mut style = unsafe { get_window_long_ptr(hwnd, GWL_STYLE) } as u32;
    style &= !STYLE_MASK;
    style |= get_window_style(window);

    // SAFETY: as above.
    unsafe { set_window_long_ptr(hwnd, GWL_STYLE, style as i32 as isize) };
}

/// Translation of `WIN_SetWindowAlwaysOnTop()`.
pub(crate) fn set_window_always_on_top(window: WindowID, _on_top: bool) {
    let _ = set_window_position_internal(
        window,
        SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
        WindowRect::Current,
    );
}

/// Translation of `WIN_RestoreWindow()`.
pub(crate) fn restore_window(window: WindowID) {
    let Some(data) = window_data(window) else {
        return;
    };
    let flags = window_flags(window);
    if !flags.contains(WindowFlags::FULLSCREEN) {
        if !data.state.with(|s| s.showing_window)
            || flags.intersects(WindowFlags::MAXIMIZED | WindowFlags::MINIMIZED)
        {
            let hwnd = data.hwnd;
            data.state.with(|s| s.expected_resize = true);
            // SAFETY: hwnd is the window's handle.
            unsafe { ShowWindow(hwnd, SW_RESTORE) };
            data.state.with(|s| s.expected_resize = false);
        }
    } else {
        data.state.with(|s| s.windowed_mode_was_maximized = false);
    }
}

/// Translation of `WIN_UpdateCornerRoundingForHWND()`.
fn update_corner_rounding_for_hwnd(videodata: &VideoData, hwnd: HWND, corner_pref: i32) {
    if let Some(set_attribute) = videodata.dwm_set_window_attribute {
        if is_windows_11_or_greater() {
            // SAFETY: the function was loaded with this signature; the
            // attribute value is an int.
            unsafe {
                set_attribute(
                    hwnd,
                    DWMWA_WINDOW_CORNER_PREFERENCE,
                    (&corner_pref as *const i32).cast(),
                    size_of::<i32>() as u32,
                )
            };
        }
    }
}

/// Translation of `WIN_UpdateBorderColorForHWND()`.
fn update_border_color_for_hwnd(videodata: &VideoData, hwnd: HWND, color_ref: u32) {
    if let Some(set_attribute) = videodata.dwm_set_window_attribute {
        if is_windows_11_or_greater() {
            // SAFETY: the function was loaded with this signature; the
            // attribute value is a COLORREF.
            unsafe {
                set_attribute(
                    hwnd,
                    DWMWA_BORDER_COLOR,
                    (&color_ref as *const u32).cast(),
                    size_of::<u32>() as u32,
                )
            };
        }
    }
}

/// Reconfigures the window to fill the given display, if fullscreen is
/// true, otherwise restores the window. Translation of
/// `WIN_SetWindowFullscreen()`.
pub(crate) fn set_window_fullscreen(
    videodata: &VideoData,
    window: WindowID,
    display: DisplayID,
    fullscreen: FullscreenOp,
) -> FullscreenResult {
    let data = window_data(window);
    let hwnd = data.as_ref().map_or(std::ptr::null_mut(), |d| d.hwnd);
    let mut enter_maximized = false;
    let entering = fullscreen != FullscreenOp::Leave;

    /* Early out if already not in fullscreen, or the styling on
     * external windows may end up being overridden.
     */
    if !window_flags(window).contains(WindowFlags::FULLSCREEN) && !entering {
        return FullscreenResult::Succeeded;
    }

    let top = if should_allow_topmost() && window_flags(window).contains(WindowFlags::ALWAYS_ON_TOP)
    {
        HWND_TOPMOST
    } else {
        HWND_NOTOPMOST
    };

    /* Use GetMonitorInfo instead of WIN_GetDisplayBounds because we want the
    monitor bounds in Windows coordinates (pixels) rather than SDL coordinates (points). */
    let mut minfo = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    let monitor = with_display_data(display, |d| d.monitor_handle).unwrap_or(std::ptr::null_mut());
    // SAFETY: minfo is sized.
    if unsafe { GetMonitorInfoW(monitor, &mut minfo) } == 0 {
        // SDL_SetError("GetMonitorInfo failed")
        return FullscreenResult::Failed;
    }
    let Some(data) = data else {
        return FullscreenResult::Failed;
    };

    send_window_event(
        window,
        if entering {
            EventType::WINDOW_ENTER_FULLSCREEN
        } else {
            EventType::WINDOW_LEAVE_FULLSCREEN
        },
        0,
        0,
    );
    // SAFETY: hwnd is the window's handle.
    let mut style = unsafe { get_window_long_ptr(hwnd, GWL_STYLE) } as u32;
    style &= !STYLE_MASK;
    style |= get_window_style(window);
    // SAFETY: as above.
    let style_ex = unsafe { get_window_long_ptr(hwnd, GWL_EXSTYLE) } as u32;

    let (mut x, mut y, w, h);
    if entering {
        x = minfo.rcMonitor.left;
        y = minfo.rcMonitor.top;
        w = minfo.rcMonitor.right - minfo.rcMonitor.left;
        h = minfo.rcMonitor.bottom - minfo.rcMonitor.top;

        /* Unset the maximized flag.  This fixes
        https://bugzilla.libsdl.org/show_bug.cgi?id=3215
        */
        if (style & WS_MAXIMIZE) != 0 {
            data.state.with(|s| s.windowed_mode_was_maximized = true);
            style &= !WS_MAXIMIZE;
        }

        // Disable corner rounding & border color (Windows 11+) so the window fills the full screen
        update_corner_rounding_for_hwnd(videodata, hwnd, DWMWCP_DONOTROUND);
        update_border_color_for_hwnd(videodata, hwnd, DWMWA_COLOR_NONE);
    } else {
        update_corner_rounding_for_hwnd(videodata, hwnd, DWMWCP_DEFAULT);
        update_border_color_for_hwnd(videodata, hwnd, DWMWA_COLOR_DEFAULT);

        /* Restore window-maximization state, as applicable.
        Special care is taken to *not* do this if and when we're
        alt-tab'ing away (to some other window; as indicated by
        in_window_deactivation), otherwise
        https://bugzilla.libsdl.org/show_bug.cgi?id=3215 can reproduce!
        */
        data.state.with(|s| {
            if s.windowed_mode_was_maximized && !s.in_window_deactivation {
                enter_maximized = true;
                s.disable_move_size_events = true;
            }
        });

        let menu = has_menu(hwnd, style);
        (x, y, w, h) =
            adjust_window_rect_with_style(window, style, style_ex, menu, WindowRect::Floating)
                .unwrap_or_default();
        data.state.with(|s| s.windowed_mode_was_maximized = false);

        /* A window may have been maximized by dragging it to the top of another display, in which case the floating
         * position may be out-of-date. If the window is being restored to maximized, and the maximized and floating
         * position are on different displays, try to center the window on the maximized display for restoration, which
         * mimics native Windows behavior.
         */
        if enter_maximized {
            let (windowed_point, floating_point) = with_window(window, |win| {
                (
                    Point {
                        x: win.core.windowed.x,
                        y: win.core.windowed.y,
                    },
                    Point {
                        x: win.core.floating.x,
                        y: win.core.floating.y,
                    },
                )
            })
            .unwrap_or_default();
            let floating_display =
                crate::video::display::display_for_point(floating_point).unwrap_or(0);
            let windowed_display =
                crate::video::display::display_for_point(windowed_point).unwrap_or(0);

            if floating_display != windowed_display {
                let bounds = crate::video::display::display_usable_bounds(windowed_display)
                    .unwrap_or_default();
                if w < bounds.w {
                    x = bounds.x + (bounds.w - w) / 2;
                } else {
                    x = bounds.x;
                }
                if h < bounds.h {
                    y = bounds.y + (bounds.h - h) / 2;
                } else {
                    y = bounds.y;
                }
            }
        }
    }

    /* Always reset the window to the base floating size before possibly re-applying the maximized state,
     * otherwise, the base floating size can seemingly be lost in some cases.
     */
    // SAFETY: hwnd is the window's handle.
    unsafe { set_window_long_ptr(hwnd, GWL_STYLE, style as i32 as isize) };
    data.state.with(|s| s.expected_resize = true);
    // SAFETY: as above.
    unsafe { SetWindowPos(hwnd, top, x, y, w, h, data.copybits_flag | SWP_NOACTIVATE) };
    data.state.with(|s| {
        s.expected_resize = false;
        s.disable_move_size_events = false;
    });

    if enter_maximized {
        maximize_window(window);
    }

    let hicon = data.state.with(|s| s.hicon);
    if !entering && !hicon.is_null() {
        // Reset the icon for the window when returning from fullscreen mode
        // SAFETY: hwnd is the window's handle; hicon is its icon.
        unsafe {
            SendMessageW(hwnd, WM_SETICON, ICON_SMALL as WPARAM, 0);
            SendMessageW(hwnd, WM_SETICON, ICON_SMALL as WPARAM, hicon as LPARAM);
        }
    }

    FullscreenResult::Succeeded
}

/// Remember the ICC profile of the window's display, optionally sending
/// `WINDOW_ICCPROF_CHANGED` when it changed. Translation of
/// `WIN_UpdateWindowICCProfile()`.
pub(crate) fn update_window_icc_profile(window: WindowID, send_event: bool) {
    let Some(data) = window_data(window) else {
        return;
    };
    let display = display_for_window(window).unwrap_or(0);
    let Some(device_name) = with_display_data(display, |d| d.device_name) else {
        return;
    };

    // SAFETY: device_name is NUL-terminated.
    let hdc = unsafe {
        CreateDCW(
            device_name.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
        )
    };
    if hdc.is_null() {
        return;
    }
    const MAX_PATH: usize = 260;
    let mut file_name = [0u16; MAX_PATH];
    let mut file_name_size = MAX_PATH as u32;
    // SAFETY: file_name holds file_name_size units; hdc is deleted after.
    let ok = unsafe { GetICMProfileW(hdc, &mut file_name_size, file_name.as_mut_ptr()) } != 0;
    if ok {
        // fileNameSize includes '\0' on return
        let end = file_name.iter().position(|&c| c == 0).unwrap_or(MAX_PATH);
        let new_name = file_name[..end].to_vec();
        let changed = data.state.with(|s| {
            if s.icm_file_name.as_deref() != Some(&new_name[..]) {
                s.icm_file_name = Some(new_name);
                true
            } else {
                false
            }
        });
        if changed && send_event {
            send_window_event(window, EventType::WINDOW_ICCPROF_CHANGED, 0, 0);
        }
    }
    // SAFETY: hdc came from CreateDCW.
    unsafe { DeleteDC(hdc) };
}

/// The contents of the ICC profile file of the window's display.
/// Translation of `WIN_GetWindowICCProfile()`.
pub(crate) fn get_window_icc_profile(window: WindowID) -> Result<Vec<u8>> {
    let Some(data) = window_data(window) else {
        return Err(Error::new("Invalid window"));
    };
    let Some(file_name) = data.state.with(|s| s.icm_file_name.clone()) else {
        // (upstream returns NULL here without setting an error)
        return Err(Error::new("No ICC profile"));
    };
    let filename_utf8 = wide_to_utf8(&file_name);
    std::fs::read(filename_utf8).map_err(|_| Error::new("Could not open ICC profile"))
}

/// Install the low-level keyboard hook. Translation of `WIN_GrabKeyboard()`.
fn grab_keyboard(videodata: &VideoData, window: WindowID) {
    let Some(data) = window_data(window) else {
        return;
    };

    if !data.state.with(|s| s.keyboard_hook).is_null() {
        return;
    }

    /* SetWindowsHookEx() needs to know which module contains the hook we
    want to install. This is complicated by the fact that SDL can be
    linked statically or dynamically. Fortunately XP and later provide
    this nice API that will go through the loaded modules and find the
    one containing our code.
    */
    let mut module = std::ptr::null_mut();
    // SAFETY: the address is our hook procedure; module receives a handle.
    if unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT | GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
            keyboard_hook_proc as *const () as usize as *const u16,
            &mut module,
        )
    } == 0
    {
        return;
    }

    // Capture a snapshot of the current keyboard state before the hook
    let mut key_state = [0u8; 256];
    // SAFETY: key_state holds the 256 bytes GetKeyboardState writes.
    if unsafe { GetKeyboardState(key_state.as_mut_ptr()) } == 0 {
        return;
    }
    videodata.state.with(|s| s.pre_hook_key_state = key_state);

    /* To grab the keyboard, we have to install a low-level keyboard hook to
    intercept keys that would normally be captured by the OS. Intercepting
    all key events on the system is rather invasive, but it's what Microsoft
    actually documents that you do to capture these.
    */
    // SAFETY: the hook procedure lives in module.
    let hook = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook_proc), module, 0) };
    data.state.with(|s| s.keyboard_hook = hook);
}

/// Translation of `WIN_UngrabKeyboard()`.
pub(crate) fn ungrab_keyboard(window: WindowID) {
    let Some(data) = window_data(window) else {
        return;
    };

    let hook = data
        .state
        .with(|s| std::mem::replace(&mut s.keyboard_hook, std::ptr::null_mut()));
    if !hook.is_null() {
        // SAFETY: the hook was installed by grab_keyboard().
        unsafe { UnhookWindowsHookEx(hook) };
    }
}

/// Translation of `WIN_SetWindowMouseRect()`.
pub(crate) fn set_window_mouse_rect(window: WindowID) -> Result<()> {
    update_clip_cursor(window);
    Ok(())
}

/// Translation of `WIN_SetWindowMouseGrab()`.
pub(crate) fn set_window_mouse_grab(window: WindowID, _grabbed: bool) -> Result<()> {
    update_clip_cursor(window);
    Ok(())
}

/// Translation of `WIN_SetWindowKeyboardGrab()`.
pub(crate) fn set_window_keyboard_grab(
    videodata: &VideoData,
    window: WindowID,
    grabbed: bool,
) -> Result<()> {
    if grabbed {
        grab_keyboard(videodata, window);
    } else {
        ungrab_keyboard(window);
    }

    Ok(())
}

/// Translation of `WIN_DestroyWindow()`.
pub(crate) fn destroy_window(window: WindowID) {
    cleanup_window_data(window);
}

/// Translation of `WIN_OnWindowEnter()`.
pub(crate) fn on_window_enter(window: WindowID) {
    let Some(data) = window_data(window) else {
        // The window wasn't fully initialized
        return;
    };
    if data.hwnd.is_null() {
        // The window wasn't fully initialized
        return;
    }

    if window_flags(window).contains(WindowFlags::ALWAYS_ON_TOP) {
        let _ = set_window_position_internal(
            window,
            data.copybits_flag | SWP_NOSIZE | SWP_NOACTIVATE,
            WindowRect::Current,
        );
    }
}

/// The client area in screen coordinates. Translation of
/// `GetClientScreenRect()`.
fn get_client_screen_rect(hwnd: HWND, rect: &mut RECT) -> bool {
    // SAFETY: rect is a RECT, laid out as two POINTs.
    unsafe {
        let points = (rect as *mut RECT).cast::<POINT>();
        GetClientRect(hwnd, rect) != 0 // RECT( left , top , right , bottom )
            && ClientToScreen(hwnd, points) != 0 // POINT( left , top )
            && ClientToScreen(hwnd, points.add(1)) != 0 // POINT( right , bottom )
    }
}

/// Translation of `WIN_UnclipCursorForWindow()`.
pub(crate) fn unclip_cursor_for_window(window: WindowID) {
    let Some(data) = window_data(window) else {
        return;
    };
    let mut rect = RECT::default();
    let clipped = data.state.with(|s| s.cursor_clipped_rect);
    // SAFETY: rect is a valid RECT.
    if unsafe { GetClipCursor(&mut rect) } != 0 && rect_eq(&rect, &clipped) {
        // SAFETY: a NULL rect releases the clip.
        unsafe { ClipCursor(std::ptr::null()) };
        data.state.with(|s| s.cursor_clipped_rect = RECT::default());
    }
}

/// `SDL_memcmp(a, b, sizeof(RECT)) == 0`.
fn rect_eq(a: &RECT, b: &RECT) -> bool {
    a.left == b.left && a.top == b.top && a.right == b.right && a.bottom == b.bottom
}

/// Confine the cursor to the window (or release it) as its focus, grab,
/// mouse rect and relative mode ask. Translation of `WIN_UpdateClipCursor()`.
pub(crate) fn update_clip_cursor(window: WindowID) {
    let Some(data) = window_data(window) else {
        return;
    };
    let (in_title_click, focus_click_pending, postpone_clipcursor, cursor_clipped_rect) =
        data.state.with(|s| {
            (
                s.in_title_click,
                s.focus_click_pending,
                s.postpone_clipcursor,
                s.cursor_clipped_rect,
            )
        });
    if in_title_click || focus_click_pending != 0 || postpone_clipcursor {
        return;
    }

    let Ok((mouse_rect, flags)) = with_window(window, |w| (w.core.mouse_rect, w.core.flags)) else {
        return;
    };
    let win_mouse_rect = mouse_rect.w > 0 && mouse_rect.h > 0;
    let win_have_focus = flags.contains(WindowFlags::INPUT_FOCUS);
    let win_is_grabbed = flags.contains(WindowFlags::MOUSE_GRABBED);
    let win_in_relmode = flags.contains(WindowFlags::MOUSE_RELATIVE_MODE);
    let cursor_confine = win_in_relmode || win_is_grabbed || win_mouse_rect;

    // This is verbatim translation of the old logic,
    // but I don't quite get what it's trying to do.
    // A clean-room implementation according to MSDN
    // documentation of GetClipCursor is provided in
    // a commented-out block below.
    if !win_have_focus || !cursor_confine {
        let mut current = RECT::default();
        // SAFETY: current is a valid RECT.
        if unsafe { GetClipCursor(&mut current) } == 0 {
            return;
        }
        if let Ok(desktop_bounds) = with_device(|v| v.desktop_bounds) {
            if current.left != desktop_bounds.x || current.top != desktop_bounds.y {
                let first = POINT {
                    x: current.left,
                    y: current.top,
                };
                let second = POINT {
                    x: current.right - 1,
                    y: current.bottom - 1,
                };
                // SAFETY: PtInRect reads the rect.
                if unsafe {
                    PtInRect(&cursor_clipped_rect, first) == 0
                        || PtInRect(&cursor_clipped_rect, second) == 0
                } {
                    return;
                }
            }
        }
        // SAFETY: a NULL rect releases the clip.
        unsafe { ClipCursor(std::ptr::null()) };
        data.state.with(|s| s.cursor_clipped_rect = RECT::default());
        return;
    }

    // if (!win_have_focus || !cursor_confine) {
    //     RECT current;
    //     SDL_VideoDevice *videodevice = SDL_GetVideoDevice();
    //     if (GetClipCursor(&current) && (!videodevice ||
    //         current.left   != videodevice->desktop_bounds.x ||
    //         current.top    != videodevice->desktop_bounds.y ||
    //         current.right  != videodevice->desktop_bounds.x + videodevice->desktop_bounds.w ||
    //         current.bottom != videodevice->desktop_bounds.y + videodevice->desktop_bounds.h )) {
    //         ClipCursor(NULL);
    //         SDL_zero(data->cursor_clipped_rect);
    //     }
    //     return;
    // }

    let lock_to_ctr = mouse::relative_mode_enabled() && mouse::relative_mode_center();

    let mut client = RECT::default();
    if !get_client_screen_rect(data.hwnd, &mut client) {
        return;
    }

    let mut target = client;
    if lock_to_ctr {
        let cx = (client.left + client.right) / 2;
        let cy = (client.top + client.bottom) / 2;
        target = data.cursor_ctrlock_rect;
        target.left += cx;
        target.right += cx;
        target.top += cy;
        target.bottom += cy;
    } else if win_mouse_rect {
        let custom = RECT {
            left: client.left + mouse_rect.x,
            top: client.top + mouse_rect.y,
            right: client.left + mouse_rect.x + mouse_rect.w,
            bottom: client.top + mouse_rect.y + mouse_rect.h,
        };
        let mut overlap = RECT::default();
        // SAFETY: the rects are valid.
        if unsafe { IntersectRect(&mut overlap, &client, &custom) } != 0 {
            target = overlap;
        } else if !win_is_grabbed {
            unclip_cursor_for_window(window);
            return;
        }
    }

    // SAFETY: the rects are valid.
    unsafe {
        if GetClipCursor(&mut client) != 0 && !rect_eq(&target, &client) && ClipCursor(&target) != 0
        {
            data.state.with(|s| s.cursor_clipped_rect = target); // ClipCursor may fail if rect beyond screen
        }
    }
}

/// Translation of `WIN_SetWindowHitTest()`.
pub(crate) fn set_window_hit_test(_window: WindowID, _enabled: bool) -> Result<()> {
    Ok(()) // just succeed, the real work is done elsewhere.
}

/// Translation of `WIN_SetWindowOpacity()`.
pub(crate) fn set_window_opacity(window: WindowID, opacity: f32) -> Result<()> {
    let Some(data) = window_data(window) else {
        return Err(Error::new("Invalid window"));
    };
    let hwnd = data.hwnd;
    // SAFETY: hwnd is the window's handle.
    let style = unsafe { get_window_long_ptr(hwnd, GWL_EXSTYLE) } as u32;

    if opacity == 1.0 {
        // want it fully opaque, just mark it unlayered if necessary.
        if (style & WS_EX_LAYERED) != 0 {
            // SAFETY: as above.
            if unsafe {
                set_window_long_ptr(hwnd, GWL_EXSTYLE, (style & !WS_EX_LAYERED) as i32 as isize)
            } == 0
            {
                return Err(set_error("SetWindowLong()"));
            }
        }
    } else {
        let alpha = (opacity * 255.0) as i32 as u8;
        // want it transparent, mark it layered if necessary.
        if (style & WS_EX_LAYERED) == 0 {
            // SAFETY: as above.
            if unsafe {
                set_window_long_ptr(hwnd, GWL_EXSTYLE, (style | WS_EX_LAYERED) as i32 as isize)
            } == 0
            {
                return Err(set_error("SetWindowLong()"));
            }
        }

        // SAFETY: as above.
        if unsafe { SetLayeredWindowAttributes(hwnd, 0, alpha, LWA_ALPHA) } == 0 {
            return Err(set_error("SetLayeredWindowAttributes()"));
        }
    }

    Ok(())
}

/// A clipboard format's name, for the drop target's trace. Translation of
/// `SDLGetClipboardFormatName()`.
fn get_clipboard_format_name(cf: u32) -> Option<String> {
    let name = match cf as u16 {
        CF_TEXT => "CF_TEXT",
        CF_BITMAP => "CF_BITMAP",
        CF_METAFILEPICT => "CF_METAFILEPICT",
        CF_SYLK => "CF_SYLK",
        CF_DIF => "CF_DIF",
        CF_TIFF => "CF_TIFF",
        CF_OEMTEXT => "CF_OEMTEXT",
        CF_DIB => "CF_DIB",
        CF_PALETTE => "CF_PALETTE",
        CF_PENDATA => "CF_PENDATA",
        CF_RIFF => "CF_RIFF",
        CF_WAVE => "CF_WAVE",
        CF_UNICODETEXT => "CF_UNICODETEXT",
        CF_ENHMETAFILE => "CF_ENHMETAFILE",
        CF_HDROP => "CF_HDROP",
        CF_LOCALE => "CF_LOCALE",
        CF_DIBV5 => "CF_DIBV5",
        CF_OWNERDISPLAY => "CF_OWNERDISPLAY",
        CF_DSPTEXT => "CF_DSPTEXT",
        CF_DSPBITMAP => "CF_DSPBITMAP",
        CF_DSPMETAFILEPICT => "CF_DSPMETAFILEPICT",
        CF_DSPENHMETAFILE => "CF_DSPENHMETAFILE",
        _ => {
            let mut text = [0u8; 257];
            // SAFETY: text holds 256 bytes plus a terminator.
            let n = unsafe { GetClipboardFormatNameA(cf, text.as_mut_ptr(), 256) };
            if n > 0 {
                return Some(String::from_utf8_lossy(&text[..n as usize]).into_owned());
            } else {
                return None;
            }
        }
    };
    Some(name.to_owned())
}

// ---------------------------------------------------------------------------
// OLE drag and drop: an IDropTarget, and the IDataObject methods it calls
// ---------------------------------------------------------------------------

/// The `IDataObject` COM interface (the methods SDL calls).
#[repr(C)]
struct IDataObject {
    vtbl: *const IDataObjectVtbl,
}

#[repr(C)]
struct IDataObjectVtbl {
    query_interface: usize,
    add_ref: usize,
    release: usize,
    get_data:
        unsafe extern "system" fn(*mut IDataObject, *const FORMATETC, *mut STGMEDIUM) -> HRESULT,
    get_data_here: usize,
    query_get_data: unsafe extern "system" fn(*mut IDataObject, *const FORMATETC) -> HRESULT,
    get_canonical_format_etc: usize,
    set_data: usize,
    enum_format_etc:
        unsafe extern "system" fn(*mut IDataObject, u32, *mut *mut IEnumFormatEtc) -> HRESULT,
}

/// The `IEnumFORMATETC` COM interface (the methods SDL calls).
#[repr(C)]
struct IEnumFormatEtc {
    vtbl: *const IEnumFormatEtcVtbl,
}

#[repr(C)]
struct IEnumFormatEtcVtbl {
    query_interface: usize,
    add_ref: usize,
    release: usize,
    next: unsafe extern "system" fn(*mut IEnumFormatEtc, u32, *mut FORMATETC, *mut u32) -> HRESULT,
}

/// `IID_IUnknown`
const IID_IUNKNOWN: GUID = GUID {
    data1: 0x00000000,
    data2: 0x0000,
    data3: 0x0000,
    data4: [0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46],
};
/// `IID_IDropTarget`
const IID_IDROP_TARGET: GUID = GUID {
    data1: 0x00000122,
    data2: 0x0000,
    data3: 0x0000,
    data4: [0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46],
};
/// `DATADIR_GET`
const DATADIR_GET: u32 = 1;

/// SDL's `IDropTarget` implementation. Translation of `SDLDropTarget`.
#[repr(C)]
pub(crate) struct DropTarget {
    lp_vtbl: *const IDropTargetVtbl,
    refcount: AtomicI32,
    window: WindowID,
    hwnd: HWND,
    format_text: u32,
    format_file: u32,
}

#[repr(C)]
struct IDropTargetVtbl {
    query_interface:
        unsafe extern "system" fn(*mut DropTarget, *const GUID, *mut *mut c_void) -> HRESULT,
    add_ref: unsafe extern "system" fn(*mut DropTarget) -> u32,
    release: unsafe extern "system" fn(*mut DropTarget) -> u32,
    drag_enter: unsafe extern "system" fn(
        *mut DropTarget,
        *mut IDataObject,
        u32,
        POINTL,
        *mut u32,
    ) -> HRESULT,
    drag_over: unsafe extern "system" fn(*mut DropTarget, u32, POINTL, *mut u32) -> HRESULT,
    drag_leave: unsafe extern "system" fn(*mut DropTarget) -> HRESULT,
    drop: unsafe extern "system" fn(
        *mut DropTarget,
        *mut IDataObject,
        u32,
        POINTL,
        *mut u32,
    ) -> HRESULT,
}

/// Translation of `SDLDropTarget_AddRef()`.
unsafe extern "system" fn drop_target_add_ref(target: *mut DropTarget) -> u32 {
    // SAFETY: OLE passes our live object.
    let target = unsafe { &*target };
    (target.refcount.fetch_add(1, Ordering::AcqRel) + 1) as u32
}

/// Translation of `SDLDropTarget_Release()`.
unsafe extern "system" fn drop_target_release(target: *mut DropTarget) -> u32 {
    // SAFETY: OLE (or accept_drag_and_drop) passes our live object.
    let refcount = unsafe { (*target).refcount.fetch_sub(1, Ordering::AcqRel) } - 1;
    if refcount == 0 {
        // SAFETY: the object came from Box::into_raw and this was the last
        // reference.
        drop(unsafe { Box::from_raw(target) });
        return 0;
    }
    refcount as u32
}

/// Translation of `SDLDropTarget_QueryInterface()`.
unsafe extern "system" fn drop_target_query_interface(
    target: *mut DropTarget,
    riid: *const GUID,
    ppv: *mut *mut c_void,
) -> HRESULT {
    if ppv.is_null() {
        return E_INVALIDARG;
    }

    // SAFETY: ppv is a valid out pointer; riid is the IID OLE asks for.
    unsafe {
        *ppv = std::ptr::null_mut();
        if is_equal_guid(&*riid, &IID_IUNKNOWN) || is_equal_guid(&*riid, &IID_IDROP_TARGET) {
            *ppv = target.cast();
        }
        if !(*ppv).is_null() {
            drop_target_add_ref(target);
            return S_OK;
        }
    }
    E_NOINTERFACE
}

/// Report a drop position in client coordinates (shared by the drag
/// methods).
fn send_drop_position_for(target: &DropTarget, pt: POINTL, what: &str) -> bool {
    let mut pnt = POINT { x: pt.x, y: pt.y };
    // SAFETY: pnt is a valid POINT.
    if unsafe { ScreenToClient(target.hwnd, &mut pnt) } != 0 {
        crate::log::trace!(
            Category::Input,
            ". In {} at {}, {} => window {} at {}, {}",
            what,
            pt.x,
            pt.y,
            target.window,
            pnt.x,
            pnt.y
        );
        send_drop_position(Some(target.window), pnt.x as f32, pnt.y as f32);
        true
    } else {
        false
    }
}

/// Translation of `SDLDropTarget_DragEnter()`.
unsafe extern "system" fn drop_target_drag_enter(
    target: *mut DropTarget,
    _p_data_object: *mut IDataObject,
    _grf_key_state: u32,
    pt: POINTL,
    pdw_effect: *mut u32,
) -> HRESULT {
    // SAFETY: OLE passes our live object and a valid effect pointer.
    let target = unsafe { &*target };
    crate::log::trace!(Category::Input, ". In DragEnter at {}, {}", pt.x, pt.y);
    // SAFETY: as above.
    unsafe { *pdw_effect = DROPEFFECT_COPY };
    if !send_drop_position_for(target, pt, "DragEnter") {
        crate::log::trace!(
            Category::Input,
            ". In DragEnter at {}, {} => nil, nil",
            pt.x,
            pt.y
        );
    }
    S_OK
}

/// Translation of `SDLDropTarget_DragOver()`.
unsafe extern "system" fn drop_target_drag_over(
    target: *mut DropTarget,
    _grf_key_state: u32,
    pt: POINTL,
    pdw_effect: *mut u32,
) -> HRESULT {
    // SAFETY: OLE passes our live object and a valid effect pointer.
    let target = unsafe { &*target };
    crate::log::trace!(Category::Input, ". In DragOver at {}, {}", pt.x, pt.y);
    // SAFETY: as above.
    unsafe { *pdw_effect = DROPEFFECT_COPY };
    if !send_drop_position_for(target, pt, "DragOver") {
        crate::log::trace!(
            Category::Input,
            ". In DragOver at {}, {} => nil, nil",
            pt.x,
            pt.y
        );
    }
    S_OK
}

/// Translation of `SDLDropTarget_DragLeave()`.
unsafe extern "system" fn drop_target_drag_leave(target: *mut DropTarget) -> HRESULT {
    // SAFETY: OLE passes our live object.
    let target = unsafe { &*target };
    crate::log::trace!(Category::Input, ". In DragLeave");
    send_drop_complete(Some(target.window));
    S_OK
}

/// Split dropped text at line breaks (`SDL_strtok_r(text, "\r\n", ...)`).
fn tokens(text: &str) -> impl Iterator<Item = &str> {
    text.split(['\r', '\n']).filter(|t| !t.is_empty())
}

/// Get one format of the drop's data as `HGLOBAL` and run `f` on its
/// bytes; false if the data object doesn't offer it.
///
/// # Safety
///
/// `data_object` must be the live data object of the drop.
unsafe fn with_drop_data(
    data_object: *mut IDataObject,
    cf_format: u16,
    format_mime: &str,
    what: &str,
    f: impl FnOnce(&[u8]),
) -> bool {
    let fetc = FORMATETC {
        cfFormat: cf_format,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT,
        lindex: -1,
        tymed: TYMED_HGLOBAL as u32,
    };
    // SAFETY: data_object is live (the caller's contract); the medium is
    // released after use.
    unsafe {
        let vtbl = &*(*data_object).vtbl;
        if (vtbl.query_get_data)(data_object, &fetc) < 0 {
            return false;
        }
        crate::log::trace!(
            Category::Input,
            ". In Drop {} for QueryGetData, format {:08x} '{}', success",
            what,
            fetc.cfFormat,
            format_mime
        );
        let mut med: STGMEDIUM = std::mem::zeroed();
        let hres = (vtbl.get_data)(data_object, &fetc, &mut med);
        crate::log::trace!(
            Category::Input,
            ". In Drop {} for      GetData, format {:08x} '{}', HRESULT is {:08x}",
            what,
            fetc.cfFormat,
            format_mime,
            hres
        );
        if hres < 0 {
            return false;
        }
        let bsize = GlobalSize(med.u.hGlobal);
        let buffer = GlobalLock(med.u.hGlobal);
        crate::log::trace!(
            Category::Input,
            ". In Drop {} for   GlobalLock, format {:08x} '{}', memory ({}) {:p}",
            what,
            fetc.cfFormat,
            format_mime,
            bsize,
            buffer
        );
        if !buffer.is_null() {
            f(std::slice::from_raw_parts(buffer.cast::<u8>(), bsize));
        }
        GlobalUnlock(med.u.hGlobal);
        ReleaseStgMedium(&mut med);
    }
    true
}

/// The text up to the first NUL (the C code copies the buffer and reads it
/// as a C string).
fn c_text(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

/// Translation of `SDLDropTarget_Drop()`.
unsafe extern "system" fn drop_target_drop(
    target: *mut DropTarget,
    p_data_object: *mut IDataObject,
    _grf_key_state: u32,
    pt: POINTL,
    pdw_effect: *mut u32,
) -> HRESULT {
    // SAFETY: OLE passes our live object and a valid effect pointer.
    let target = unsafe { &*target };
    // SAFETY: as above.
    unsafe { *pdw_effect = DROPEFFECT_COPY };
    if !send_drop_position_for(target, pt, "Drop") {
        crate::log::trace!(
            Category::Input,
            ". In Drop at {}, {} => nil, nil",
            pt.x,
            pt.y
        );
    }
    let window = Some(target.window);

    // SAFETY: p_data_object is the live data object of the drop.
    unsafe {
        let mut p_enum_format_etc: *mut IEnumFormatEtc = std::ptr::null_mut();
        let hres = ((*(*p_data_object).vtbl).enum_format_etc)(
            p_data_object,
            DATADIR_GET,
            &mut p_enum_format_etc,
        );
        crate::log::trace!(
            Category::Input,
            ". In Drop for EnumFormatEtc, HRESULT is {:08x}",
            hres
        );
        if hres == S_OK {
            let mut fetc: FORMATETC = std::mem::zeroed();
            while ((*(*p_enum_format_etc).vtbl).next)(
                p_enum_format_etc,
                1,
                &mut fetc,
                std::ptr::null_mut(),
            ) == S_OK
            {
                match get_clipboard_format_name(fetc.cfFormat as u32) {
                    Some(cfnm) => crate::log::trace!(
                        Category::Input,
                        ". In Drop, Supported format is {:08x}, '{}'",
                        fetc.cfFormat,
                        cfnm
                    ),
                    None => crate::log::trace!(
                        Category::Input,
                        ". In Drop, Supported format is {:08x}, Predefined",
                        fetc.cfFormat
                    ),
                }
            }
            // FIXME (upstream): SDLDropTarget_Drop() never releases the
            // IEnumFORMATETC it gets from EnumFormatEtc(), leaking it.
        }
    }

    // SAFETY: as above, for each format.
    unsafe {
        if with_drop_data(
            p_data_object,
            target.format_file as u16,
            "text/uri-list",
            "File",
            |bytes| {
                let text = c_text(bytes);
                for token in tokens(&text) {
                    if let Some(file) = crate::utils::uri_to_local(token) {
                        let file = String::from_utf8_lossy(&file);
                        crate::log::trace!(
                            Category::Input,
                            ". In Drop File, file ({} of {}) '{}'",
                            file.len(),
                            bytes.len(),
                            file
                        );
                        send_drop_file(window, None, &file);
                    }
                }
            },
        ) {
            send_drop_complete(window);
            return S_OK;
        }

        if with_drop_data(
            p_data_object,
            target.format_text as u16,
            "text/plain;charset=utf-8",
            "Text",
            |bytes| {
                let text = c_text(bytes);
                for token in tokens(&text) {
                    crate::log::trace!(
                        Category::Input,
                        ". In Drop Text, text ({} of {}) '{}'",
                        token.len(),
                        bytes.len(),
                        token
                    );
                    send_drop_text(window, token);
                }
            },
        ) {
            send_drop_complete(window);
            return S_OK;
        }

        if with_drop_data(
            p_data_object,
            CF_UNICODETEXT,
            "CF_UNICODETEXT",
            "Text",
            |bytes| {
                let wide: Vec<u16> = bytes
                    .chunks_exact(2)
                    .map(|c| u16::from_le_bytes([c[0], c[1]]))
                    .collect();
                let buffer = wide_to_utf8(&wide);
                crate::log::trace!(
                    Category::Input,
                    ". In Drop Text for StringToUTF8, format {:08x} '{}', memory ({})",
                    CF_UNICODETEXT,
                    "CF_UNICODETEXT",
                    buffer.len()
                );
                for token in tokens(&buffer) {
                    crate::log::trace!(
                        Category::Input,
                        ". In Drop Text, text ({} of {}) '{}'",
                        token.len(),
                        buffer.len(),
                        token
                    );
                    send_drop_text(window, token);
                }
            },
        ) {
            send_drop_complete(window);
            return S_OK;
        }

        if with_drop_data(p_data_object, CF_TEXT, "CF_TEXT", "Text", |bytes| {
            let text = c_text(bytes);
            for token in tokens(&text) {
                crate::log::trace!(
                    Category::Input,
                    ". In Drop Text, text ({} of {}) '{}'",
                    token.len(),
                    bytes.len(),
                    token
                );
                send_drop_text(window, token);
            }
        }) {
            send_drop_complete(window);
            return S_OK;
        }

        if with_drop_data(p_data_object, CF_HDROP, "CF_HDROP", "File", |bytes| {
            let drop = bytes.as_ptr() as HDROP;
            let count = DragQueryFileW(drop, 0xFFFFFFFF, std::ptr::null_mut(), 0);
            for i in 0..count {
                let size = DragQueryFileW(drop, i, std::ptr::null_mut(), 0) + 1;
                let mut buffer = vec![0u16; size as usize];
                if DragQueryFileW(drop, i, buffer.as_mut_ptr(), size) != 0 {
                    let file = wide_to_utf8(&buffer);
                    crate::log::trace!(
                        Category::Input,
                        ". In Drop File, file ({} of {}) '{}'",
                        file.len(),
                        bytes.len(),
                        file
                    );
                    send_drop_file(window, None, &file);
                }
            }
        }) {
            send_drop_complete(window);
            return S_OK;
        }
    }

    send_drop_complete(window);
    S_OK
}

/// Translation of `vtDropTarget`.
static VT_DROP_TARGET: IDropTargetVtbl = IDropTargetVtbl {
    query_interface: drop_target_query_interface,
    add_ref: drop_target_add_ref,
    release: drop_target_release,
    drag_enter: drop_target_drag_enter,
    drag_over: drop_target_drag_over,
    drag_leave: drop_target_drag_leave,
    drop: drop_target_drop,
};

/// Turn drag and drop on or off: an OLE drop target if OLE is up, else
/// `WM_DROPFILES`. Translation of `WIN_AcceptDragAndDrop()`.
pub(crate) fn accept_drag_and_drop(videodata: &VideoData, window: WindowID, accept: bool) {
    let Some(data) = window_data(window) else {
        return;
    };
    if videodata.state.with(|s| s.oleinitialized) {
        let current = data.state.with(|s| s.drop_target);
        if accept && current.is_null() {
            let uri_list = utf8_to_wide("text/uri-list");
            let plain_text = utf8_to_wide("text/plain;charset=utf-8");
            // SAFETY: the format names are NUL-terminated.
            let (format_file, format_text) = unsafe {
                (
                    RegisterClipboardFormatW(uri_list.as_ptr()),
                    RegisterClipboardFormatW(plain_text.as_ptr()),
                )
            };
            let drop_target = Box::into_raw(Box::new(DropTarget {
                lp_vtbl: &VT_DROP_TARGET,
                refcount: AtomicI32::new(0),
                window,
                hwnd: data.hwnd,
                format_text,
                format_file,
            }));
            data.state.with(|s| s.drop_target = drop_target);
            // SAFETY: drop_target is a live COM object; OLE takes its own
            // reference.
            unsafe {
                drop_target_add_ref(drop_target);
                RegisterDragDrop(data.hwnd, drop_target.cast());
            }
            crate::log::trace!(
                Category::Input,
                ". In Accept Drag and Drop, window {}, enabled Full OLE IDropTarget",
                window
            );
        } else if !accept && !current.is_null() {
            // SAFETY: current is our live drop target; we drop our reference.
            unsafe {
                RevokeDragDrop(data.hwnd);
                drop_target_release(current);
            }
            data.state.with(|s| s.drop_target = std::ptr::null_mut());
            crate::log::trace!(
                Category::Input,
                ". In Accept Drag and Drop, window {}, disabled Full OLE IDropTarget",
                window
            );
        }
    } else {
        // SAFETY: hwnd is the window's handle.
        unsafe { DragAcceptFiles(data.hwnd, accept as i32) };
        crate::log::trace!(
            Category::Input,
            ". In Accept Drag and Drop, window {}, {} Fallback WM_DROPFILES",
            window,
            if accept { "enabled" } else { "disabled" }
        );
    }
}

/// Translation of `WIN_FlashWindow()`.
pub(crate) fn flash_window(window: WindowID, operation: FlashOperation) -> Result<()> {
    let Some(data) = window_data(window) else {
        return Err(Error::new("Invalid window"));
    };
    let mut desc = FLASHWINFO {
        cbSize: size_of::<FLASHWINFO>() as u32,
        hwnd: data.hwnd,
        ..Default::default()
    };
    match operation {
        FlashOperation::Cancel => {
            desc.dwFlags = FLASHW_STOP;
        }
        FlashOperation::Briefly => {
            desc.dwFlags = FLASHW_TRAY;
            desc.uCount = 1;
        }
        FlashOperation::UntilFocused => {
            desc.dwFlags = FLASHW_TRAY | FLASHW_TIMERNOFG;
        }
    }

    // SAFETY: desc is sized.
    unsafe { FlashWindowEx(&desc) };

    Ok(())
}

/// Show the window's progress state in its taskbar button. Translation of
/// `WIN_ApplyWindowProgress()`.
pub(crate) fn apply_window_progress(videodata: &VideoData, window: WindowID) -> Result<()> {
    let Some(data) = window_data(window) else {
        return Err(Error::new("Invalid window"));
    };
    if !data.state.with(|s| s.taskbar_button_created) {
        return Ok(());
    }

    let (progress_state, progress_value) =
        with_window(window, |w| (w.progress_state, w.progress_value))?;
    if progress_state == ProgressState::None && videodata.state.with(|s| s.taskbar_list).is_null() {
        return Ok(());
    }

    let taskbar_list = get_taskbar_list(videodata, &data)?;

    const TBPF_NOPROGRESS: i32 = 0x0;
    const TBPF_INDETERMINATE: i32 = 0x1;
    const TBPF_NORMAL: i32 = 0x2;
    const TBPF_ERROR: i32 = 0x4;
    const TBPF_PAUSED: i32 = 0x8;
    let tbp_flags = match progress_state {
        ProgressState::None => TBPF_NOPROGRESS,
        ProgressState::Indeterminate => TBPF_INDETERMINATE,
        ProgressState::Normal => TBPF_NORMAL,
        ProgressState::Paused => TBPF_PAUSED,
        ProgressState::Error => TBPF_ERROR,
    };

    // SAFETY: taskbar_list is a live ITaskbarList3.
    let ret =
        unsafe { ((*(*taskbar_list).vtbl).set_progress_state)(taskbar_list, data.hwnd, tbp_flags) };
    if ret < 0 {
        return Err(crate::core::windows::error_from_hresult(
            Some("ITaskbarList3::SetProgressState()"),
            ret,
        ));
    }

    if matches!(
        progress_state,
        ProgressState::Normal | ProgressState::Paused | ProgressState::Error
    ) {
        // SAFETY: as above.
        let ret = unsafe {
            ((*(*taskbar_list).vtbl).set_progress_value)(
                taskbar_list,
                data.hwnd,
                (progress_value * 10000.0) as u64,
                10000,
            )
        };
        if ret < 0 {
            return Err(crate::core::windows::error_from_hresult(
                Some("ITaskbarList3::SetProgressValue()"),
                ret,
            ));
        }
    }
    Ok(())
}

/// Translation of `WIN_ShowWindowSystemMenu()`.
pub(crate) fn show_window_system_menu(window: WindowID, x: i32, y: i32) {
    let Some(data) = window_data(window) else {
        return;
    };
    let mut pt = POINT { x, y };

    // SAFETY: pt is a valid POINT; hwnd is the window's handle.
    unsafe {
        ClientToScreen(data.hwnd, &mut pt);
        SendMessageW(data.hwnd, WM_POPUPSYSTEMMENU, 0, make_lparam(pt.x, pt.y));
    }
}

/// `MAKELPARAM(lo, hi)`.
pub(crate) fn make_lparam(lo: i32, hi: i32) -> LPARAM {
    ((lo as u16 as u32) | ((hi as u16 as u32) << 16)) as i32 as LPARAM
}

/// Translation of `WIN_SetWindowFocusable()`.
pub(crate) fn set_window_focusable(window: WindowID, focusable: bool) -> Result<()> {
    let Some(data) = window_data(window) else {
        return Err(Error::new("Invalid window"));
    };
    if !is_popup(window) {
        let hwnd = data.hwnd;
        // SAFETY: hwnd is the window's handle.
        let style = unsafe { get_window_long_ptr(hwnd, GWL_EXSTYLE) } as u32;

        crate::sdl_assert!(style != 0);

        if focusable {
            if (style & WS_EX_NOACTIVATE) != 0 {
                // SAFETY: as above.
                if unsafe {
                    set_window_long_ptr(
                        hwnd,
                        GWL_EXSTYLE,
                        (style & !WS_EX_NOACTIVATE) as i32 as isize,
                    )
                } == 0
                {
                    return Err(set_error("SetWindowLong()"));
                }
            }
        } else if (style & WS_EX_NOACTIVATE) == 0 {
            // SAFETY: as above.
            if unsafe {
                set_window_long_ptr(
                    hwnd,
                    GWL_EXSTYLE,
                    (style | WS_EX_NOACTIVATE) as i32 as isize,
                )
            } == 0
            {
                return Err(set_error("SetWindowLong()"));
            }
        }
    } else if window_flags(window).contains(WindowFlags::POPUP_MENU) {
        let flags = window_flags(window);
        if !flags.contains(WindowFlags::HIDDEN) {
            if !focusable && flags.contains(WindowFlags::INPUT_FOCUS) {
                if let Ok((Some(new_focus), set_focus)) = should_relinquish_popup_focus(window) {
                    set_keyboard_focus(new_focus, set_focus);
                }
            } else if focusable && should_focus_popup(window) {
                set_keyboard_focus(window, true);
            }
        }

        return Ok(());
    }

    Ok(())
}

/// Translation of `WIN_SetWindowParent()`.
pub(crate) fn set_window_parent(window: WindowID, parent: Option<WindowID>) -> Result<()> {
    let Some(child_data) = window_data(window) else {
        return Err(Error::new("Invalid window"));
    };
    let parent_hwnd = parent
        .and_then(window_data)
        .map_or(std::ptr::null_mut(), |d| d.hwnd);
    // SAFETY: the handles are windows (or NULL for no parent).
    unsafe {
        let style = get_window_long_ptr(child_data.hwnd, GWL_STYLE) as u32;

        if (style & WS_CHILD) == 0 {
            /* Despite the name, this changes the *owner* of a toplevel window, not
             * the parent of a child window.
             *
             * https://devblogs.microsoft.com/oldnewthing/20100315-00/?p=14613
             */
            set_window_long_ptr(child_data.hwnd, GWLP_HWNDPARENT, parent_hwnd as isize);
        } else {
            SetParent(child_data.hwnd, parent_hwnd);
        }
    }

    Ok(())
}

/// Translation of `WIN_SetWindowModal()`.
pub(crate) fn set_window_modal(window: WindowID, modal: bool) -> Result<()> {
    let parent_hwnd = window_parent(window)
        .and_then(window_data)
        .map_or(std::ptr::null_mut(), |d| d.hwnd);

    // SAFETY: EnableWindow accepts any window handle.
    unsafe {
        if modal {
            // Disable the parent window.
            EnableWindow(parent_hwnd, 0);
        } else if !window_flags(window).contains(WindowFlags::HIDDEN) {
            // Re-enable the parent window
            EnableWindow(parent_hwnd, 1);
        }
    }

    Ok(())
}

/// The window data a window property points at (the window procedure's
/// fallback for windows not in the video core's list yet).
pub(crate) fn window_data_from_prop(hwnd: HWND) -> Option<Arc<WindowData>> {
    let prop = utf8_to_wide(WINDOW_DATA_PROP);
    // SAFETY: the property, if set, points at a WindowData kept alive by the
    // window's internal data until cleanup_window_data() removes the
    // property (on this thread).
    unsafe {
        let ptr = GetPropW(hwnd, prop.as_ptr()) as *const WindowData;
        if ptr.is_null() {
            return None;
        }
        Arc::increment_strong_count(ptr);
        Some(Arc::from_raw(ptr))
    }
}
