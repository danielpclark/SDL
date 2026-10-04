// Rust translation of src/video/windows/SDL_windowsvideo.c and
// SDL_windowsvideo.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Windows video driver: Win32 windows, the message pump, GDI
//! framebuffers, displays and modes, keyboard (with the IMM32 input method
//! editor), mouse (cursors, raw input), the clipboard, message boxes, drag
//! and drop, window shapes and Vulkan surfaces.
//!
//! Everything newer than Windows XP that upstream loads at run time
//! (per-monitor DPI functions, `shcore.dll`, `dwmapi.dll`, the pointer and
//! touch functions, `TaskDialogIndirect`...) is loaded at run time here too,
//! so a program using this driver still starts on older Windows.
//!
//! Not translated yet (each needs large COM interface declarations or a
//! subsystem that isn't translated):
//!
//! * `SDL_windowsgameinput.cpp` (the GameInput keyboard/mouse backend, C++
//!   COM); this driver behaves like a build without `HAVE_GAMEINPUT_H`.
//! * The DXGI parts (`HAVE_DXGI_H`/`HAVE_DXGI1_6_H`): refresh rates come
//!   from `EnumDisplaySettings` and displays report no HDR, like a build
//!   without `dxgi.h`; `SDL_GetDXGIOutputInfo()` and
//!   `SDL_GetDirect3D9AdapterIndex()` (Direct3D 9 COM) come with the
//!   Direct3D renderers.
//! * WGL and EGL contexts (`SDL_windowsopengl.c`, `SDL_windowsopengles.c`):
//!   the driver behaves as a build without OpenGL, so OpenGL windows can't
//!   be created ("not available in current SDL video driver").
//! * The Text Services Framework UI (`SDL_msctf.h`), which upstream no longer
//!   compiles in.

#![allow(clippy::upper_case_acronyms)]

pub(crate) mod clipboard;
pub(crate) mod events;
pub(crate) mod framebuffer;
pub(crate) mod keyboard;
pub(crate) mod messagebox;
pub(crate) mod modes;
pub(crate) mod mouse;
pub(crate) mod rawinput;
pub(crate) mod shape;
pub(crate) mod vulkan;
pub(crate) mod window;

#[cfg(test)]
mod tests;

use std::cell::RefCell;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use windows_sys::core::{BOOL, HRESULT};
use windows_sys::Win32::Devices::Display::{
    DISPLAYCONFIG_DEVICE_INFO_HEADER, DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_PATH_INFO,
    DISPLAYCONFIG_TOPOLOGY_ID,
};
use windows_sys::Win32::Foundation::{ERROR_SUCCESS, HANDLE, HWND, RECT};
use windows_sys::Win32::Graphics::Gdi::{HMONITOR, HRGN};
use windows_sys::Win32::System::Ole::{OleInitialize, OleUninitialize};
use windows_sys::Win32::System::Power::{
    SetThreadExecutionState, ES_CONTINUOUS, ES_DISPLAY_REQUIRED,
};
use windows_sys::Win32::System::Registry::{
    RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, KEY_READ, REG_DWORD,
};
use windows_sys::Win32::UI::HiDpi::{
    DPI_AWARENESS, DPI_AWARENESS_CONTEXT, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE,
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, DPI_AWARENESS_CONTEXT_SYSTEM_AWARE,
    DPI_AWARENESS_CONTEXT_UNAWARE, MONITOR_DPI_TYPE, PROCESS_DPI_AWARENESS, PROCESS_DPI_UNAWARE,
    PROCESS_PER_MONITOR_DPI_AWARE, PROCESS_SYSTEM_DPI_AWARE,
};
use windows_sys::Win32::UI::Input::Pointer::POINTER_PEN_INFO;
use windows_sys::Win32::UI::Input::Touch::{HTOUCHINPUT, TOUCHINPUT};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    RegisterWindowMessageA, POINTER_INPUT_TYPE, WINDOW_LONG_PTR_INDEX,
};

use crate::core::windows::{co_initialize, co_uninitialize, is_windows_7_or_greater};
use crate::error::{Error, Result};
use crate::events::mouse::{
    Cursor, CursorFrame, MouseButtonFlags, MouseFeature, MouseID, SystemCursor,
};
use crate::events::{DisplayID, WindowID};
use crate::hints;
use crate::loadso::SharedObject;
use crate::log::Category;
use crate::properties::Properties;
use crate::thread::ReentrantMutex;
use crate::video::sysvideo::{
    DeviceCaps, DisplayMode, FlashOperation, FullscreenOp, FullscreenResult, SystemTheme,
    VideoBootStrap, VideoDriver,
};
use crate::video::window::WindowOp;
use crate::video::{Rect, Surface};

/// Translation of `USER_DEFAULT_SCREEN_DPI`.
pub(crate) const USER_DEFAULT_SCREEN_DPI: u32 = 96;

// Hints

/// Translation of `g_WindowsEnableMessageLoop`.
pub(crate) static G_WINDOWS_ENABLE_MESSAGE_LOOP: AtomicBool = AtomicBool::new(true);
/// Translation of `g_WindowsEnableMenuMnemonics`.
pub(crate) static G_WINDOWS_ENABLE_MENU_MNEMONICS: AtomicBool = AtomicBool::new(false);
/// Translation of `g_WindowFrameUsableWhileCursorHidden`.
pub(crate) static G_WINDOW_FRAME_USABLE_WHILE_CURSOR_HIDDEN: AtomicBool = AtomicBool::new(true);

/// State shared by the driver entry points, the window procedure and the
/// raw input thread (upstream's plain struct fields, reached through
/// `SDL_GetVideoDevice()->internal` and `window->internal`).
///
/// The lock is recursive, but the borrow is not: never call into Win32
/// functions that send window messages, or into SDL's event functions,
/// while inside [`Shared::with`] (copy what's needed out first).
pub(crate) struct Shared<T>(ReentrantMutex<RefCell<T>>);

impl<T> Shared<T> {
    pub(crate) const fn new(value: T) -> Shared<T> {
        Shared(ReentrantMutex::new(RefCell::new(value)))
    }

    /// Run `f` on the state.
    pub(crate) fn with<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        let guard = self.0.lock();
        let mut state = guard.borrow_mut();
        f(&mut state)
    }
}

// Function types of what upstream loads from user32.dll, shcore.dll and dwmapi.dll

type GetDisplayConfigBufferSizesFn = unsafe extern "system" fn(u32, *mut u32, *mut u32) -> i32;
type QueryDisplayConfigFn = unsafe extern "system" fn(
    u32,
    *mut u32,
    *mut DISPLAYCONFIG_PATH_INFO,
    *mut u32,
    *mut DISPLAYCONFIG_MODE_INFO,
    *mut DISPLAYCONFIG_TOPOLOGY_ID,
) -> i32;
type DisplayConfigGetDeviceInfoFn =
    unsafe extern "system" fn(*mut DISPLAYCONFIG_DEVICE_INFO_HEADER) -> i32;
type CloseTouchInputHandleFn = unsafe extern "system" fn(HTOUCHINPUT) -> BOOL;
type GetTouchInputInfoFn =
    unsafe extern "system" fn(HTOUCHINPUT, u32, *mut TOUCHINPUT, i32) -> BOOL;
type RegisterTouchWindowFn = unsafe extern "system" fn(HWND, u32) -> BOOL;
type DwmFlushFn = unsafe extern "system" fn() -> HRESULT;
type DwmEnableBlurBehindWindowFn = unsafe extern "system" fn(HWND, *const DwmBlurBehind) -> HRESULT;
type DwmSetWindowAttributeFn = unsafe extern "system" fn(HWND, u32, *const c_void, u32) -> HRESULT;
type GetPointerTypeFn = unsafe extern "system" fn(u32, *mut POINTER_INPUT_TYPE) -> BOOL;
type GetPointerPenInfoFn = unsafe extern "system" fn(u32, *mut POINTER_PEN_INFO) -> BOOL;
type GetPointerDeviceRectsFn = unsafe extern "system" fn(HANDLE, *mut RECT, *mut RECT) -> BOOL;
type SetProcessDPIAwareFn = unsafe extern "system" fn() -> BOOL;
type SetProcessDpiAwarenessContextFn = unsafe extern "system" fn(DPI_AWARENESS_CONTEXT) -> BOOL;
type SetThreadDpiAwarenessContextFn =
    unsafe extern "system" fn(DPI_AWARENESS_CONTEXT) -> DPI_AWARENESS_CONTEXT;
type GetThreadDpiAwarenessContextFn = unsafe extern "system" fn() -> DPI_AWARENESS_CONTEXT;
type GetAwarenessFromDpiAwarenessContextFn =
    unsafe extern "system" fn(DPI_AWARENESS_CONTEXT) -> DPI_AWARENESS;
type EnableNonClientDpiScalingFn = unsafe extern "system" fn(HWND) -> BOOL;
type AdjustWindowRectExForDpiFn = unsafe extern "system" fn(*mut RECT, u32, BOOL, u32, u32) -> BOOL;
type GetDpiForWindowFn = unsafe extern "system" fn(HWND) -> u32;
type AreDpiAwarenessContextsEqualFn =
    unsafe extern "system" fn(DPI_AWARENESS_CONTEXT, DPI_AWARENESS_CONTEXT) -> BOOL;
type IsValidDpiAwarenessContextFn = unsafe extern "system" fn(DPI_AWARENESS_CONTEXT) -> BOOL;
type GetDpiForMonitorFn =
    unsafe extern "system" fn(HMONITOR, MONITOR_DPI_TYPE, *mut u32, *mut u32) -> HRESULT;
type SetProcessDpiAwarenessFn = unsafe extern "system" fn(PROCESS_DPI_AWARENESS) -> HRESULT;

// Corner rounding support  (Win 11+)
/// Translation of `DWMWA_WINDOW_CORNER_PREFERENCE`.
pub(crate) const DWMWA_WINDOW_CORNER_PREFERENCE: u32 = 33;
/// Translation of `DWM_WINDOW_CORNER_PREFERENCE`'s values.
pub(crate) const DWMWCP_DEFAULT: i32 = 0;
pub(crate) const DWMWCP_DONOTROUND: i32 = 1;

// Border Color support (Win 11+)
/// Translation of `DWMWA_BORDER_COLOR`.
pub(crate) const DWMWA_BORDER_COLOR: u32 = 34;
/// Translation of `DWMWA_COLOR_DEFAULT`.
pub(crate) const DWMWA_COLOR_DEFAULT: u32 = 0xFFFFFFFF;
/// Translation of `DWMWA_COLOR_NONE`.
pub(crate) const DWMWA_COLOR_NONE: u32 = 0xFFFFFFFE;

// Transparent window support
/// Translation of `DWM_BB_ENABLE`.
pub(crate) const DWM_BB_ENABLE: u32 = 0x00000001;
/// Translation of `DWM_BB_BLURREGION`.
pub(crate) const DWM_BB_BLURREGION: u32 = 0x00000002;

/// Translation of `DWM_BLURBEHIND`.
#[repr(C)]
pub(crate) struct DwmBlurBehind {
    pub(crate) flags: u32,
    pub(crate) enable: BOOL,
    pub(crate) blur_region: HRGN,
    pub(crate) transition_on_maxed: BOOL,
}

/// The `ITaskbarList3` COM interface, as far as SDL calls it.
#[repr(C)]
pub(crate) struct ITaskbarList3 {
    pub(crate) vtbl: *const ITaskbarList3Vtbl,
}

/// The vtable of `ITaskbarList3` (unused slots are opaque).
#[repr(C)]
pub(crate) struct ITaskbarList3Vtbl {
    pub(crate) query_interface: usize,
    pub(crate) add_ref: usize,
    pub(crate) release: unsafe extern "system" fn(*mut ITaskbarList3) -> u32,
    pub(crate) hr_init: unsafe extern "system" fn(*mut ITaskbarList3) -> HRESULT,
    pub(crate) add_tab: usize,
    pub(crate) delete_tab: usize,
    pub(crate) activate_tab: usize,
    pub(crate) set_active_alt: usize,
    pub(crate) mark_fullscreen_window: usize,
    pub(crate) set_progress_value:
        unsafe extern "system" fn(*mut ITaskbarList3, HWND, u64, u64) -> HRESULT,
    pub(crate) set_progress_state:
        unsafe extern "system" fn(*mut ITaskbarList3, HWND, i32) -> HRESULT,
}

/// The mutable part of `struct SDL_VideoData`.
pub(crate) struct VideoState {
    pub(crate) coinitialized: bool,
    pub(crate) oleinitialized: bool,

    pub(crate) clipboard_count: u32,

    pub(crate) cleared: bool,

    pub(crate) detect_device_hotplug: bool,

    pub(crate) raw_mouse_enabled: bool,
    pub(crate) raw_mouse_flag_nolegacy: bool,
    pub(crate) raw_keyboard_enabled: bool,
    pub(crate) raw_keyboard_flag_nohotkeys: bool,
    pub(crate) raw_keyboard_flag_inputsink: bool,

    pub(crate) pre_hook_key_state: [u8; 256],
    /// `_SDL_WAKEUP`
    pub(crate) sdl_wakeup: u32,

    pub(crate) wm_taskbar_button_created: u32,
    pub(crate) taskbar_list: *mut ITaskbarList3,

    /// The hint callbacks `WIN_VideoInit()` adds (removed on quit).
    hint_callbacks: Vec<hints::Callback>,
}

// SAFETY: the COM pointer is only used on the video thread, inside the
// Shared lock.
unsafe impl Send for VideoState {}

/// Translation of `struct SDL_VideoData`: the libraries and functions the
/// driver loads at run time, and the driver-wide state.
pub(crate) struct VideoData {
    // DisplayConfig functions
    pub(crate) get_display_config_buffer_sizes: Option<GetDisplayConfigBufferSizesFn>,
    pub(crate) query_display_config: Option<QueryDisplayConfigFn>,
    pub(crate) display_config_get_device_info: Option<DisplayConfigGetDeviceInfoFn>,

    // Touch input functions
    pub(crate) close_touch_input_handle: Option<CloseTouchInputHandleFn>,
    pub(crate) get_touch_input_info: Option<GetTouchInputInfoFn>,
    pub(crate) register_touch_window: Option<RegisterTouchWindowFn>,

    // DWM functions
    #[allow(dead_code)] // (its only use is disabled upstream, see WM_TIMER)
    pub(crate) dwm_flush: Option<DwmFlushFn>,
    pub(crate) dwm_enable_blur_behind_window: Option<DwmEnableBlurBehindWindowFn>,
    pub(crate) dwm_set_window_attribute: Option<DwmSetWindowAttributeFn>,

    // Pen input functions
    pub(crate) get_pointer_type: Option<GetPointerTypeFn>,
    pub(crate) get_pointer_pen_info: Option<GetPointerPenInfoFn>,
    pub(crate) get_pointer_device_rects: Option<GetPointerDeviceRectsFn>,

    // DPI functions
    pub(crate) set_process_dpi_aware: Option<SetProcessDPIAwareFn>,
    pub(crate) set_process_dpi_awareness_context: Option<SetProcessDpiAwarenessContextFn>,
    #[allow(dead_code)] // (loaded like upstream, which doesn't call it either)
    pub(crate) set_thread_dpi_awareness_context: Option<SetThreadDpiAwarenessContextFn>,
    pub(crate) get_thread_dpi_awareness_context: Option<GetThreadDpiAwarenessContextFn>,
    #[allow(dead_code)] // (loaded like upstream, which doesn't call it either)
    pub(crate) get_awareness_from_dpi_awareness_context:
        Option<GetAwarenessFromDpiAwarenessContextFn>,
    #[allow(dead_code)] // (loaded like upstream, which doesn't call it either)
    pub(crate) enable_non_client_dpi_scaling: Option<EnableNonClientDpiScalingFn>,
    pub(crate) adjust_window_rect_ex_for_dpi: Option<AdjustWindowRectExForDpiFn>,
    pub(crate) get_dpi_for_window: Option<GetDpiForWindowFn>,
    pub(crate) are_dpi_awareness_contexts_equal: Option<AreDpiAwarenessContextsEqualFn>,
    #[allow(dead_code)] // (loaded like upstream, which doesn't call it either)
    pub(crate) is_valid_dpi_awareness_context: Option<IsValidDpiAwarenessContextFn>,
    pub(crate) get_dpi_for_monitor: Option<GetDpiForMonitorFn>,
    pub(crate) set_process_dpi_awareness: Option<SetProcessDpiAwarenessFn>,

    pub(crate) state: Shared<VideoState>,
    pub(crate) raw: Shared<events::RawInputData>,
    pub(crate) ime: Shared<keyboard::ImeData>,

    // (dropped last: the functions above point into these)
    _user_dll: Option<SharedObject>,
    _shcore_dll: Option<SharedObject>,
    _dwmapi_dll: Option<SharedObject>,
}

/// The device of the running driver (`SDL_GetVideoDevice()->internal`, for
/// the window procedure, hooks and the raw input thread).
static VIDEO_DATA: Mutex<Option<Arc<VideoData>>> = Mutex::new(None);

/// The driver data of the running Windows video driver, if it is the
/// current one.
pub(crate) fn video_data() -> Option<Arc<VideoData>> {
    VIDEO_DATA.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// `GetWindowLongPtr()`, which is `GetWindowLong()` on 32-bit Windows.
///
/// # Safety
///
/// `hwnd` must be a window handle (or null, for which Windows fails).
pub(crate) unsafe fn get_window_long_ptr(hwnd: HWND, index: WINDOW_LONG_PTR_INDEX) -> isize {
    #[cfg(target_pointer_width = "64")]
    {
        // SAFETY: as the caller promises.
        unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetWindowLongPtrW(hwnd, index) }
    }
    #[cfg(target_pointer_width = "32")]
    {
        // SAFETY: as the caller promises.
        unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetWindowLongW(hwnd, index) as isize }
    }
}

/// `SetWindowLongPtr()`, which is `SetWindowLong()` on 32-bit Windows.
///
/// # Safety
///
/// `hwnd` must be a window handle, and `value` valid for `index` (a window
/// procedure for `GWLP_WNDPROC`, say).
pub(crate) unsafe fn set_window_long_ptr(
    hwnd: HWND,
    index: WINDOW_LONG_PTR_INDEX,
    value: isize,
) -> isize {
    #[cfg(target_pointer_width = "64")]
    {
        // SAFETY: as the caller promises.
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::SetWindowLongPtrW(hwnd, index, value)
        }
    }
    #[cfg(target_pointer_width = "32")]
    {
        // SAFETY: as the caller promises.
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::SetWindowLongW(hwnd, index, value as i32)
                as isize
        }
    }
}

/// Load a function from a library, `None` if it (or the library) is
/// missing. Translation of `SDL_LoadFunction()` as upstream casts it.
///
/// # Safety
///
/// `F` must be the function's real signature.
unsafe fn load_function<F: Copy>(lib: Option<&SharedObject>, name: &str) -> Option<F> {
    // SAFETY: as the caller promises; the library outlives the pointer
    // (both live in VideoData).
    lib.and_then(|lib| unsafe { lib.function::<F>(name) }.ok())
}

/// Translation of `UpdateWindowsRawKeyboard()`.
fn update_windows_raw_keyboard(data: &Arc<VideoData>, new_value: Option<&str>) {
    let enabled = hints::string_to_bool(new_value, false);
    let _ = rawinput::set_raw_keyboard_enabled(data, enabled);
}

/// Translation of `UpdateWindowsRawKeyboardNoHotkeys()`.
fn update_windows_raw_keyboard_no_hotkeys(data: &Arc<VideoData>, new_value: Option<&str>) {
    let enabled = hints::string_to_bool(new_value, false);
    let _ = rawinput::set_raw_keyboard_flag_no_hotkeys(data, enabled);
}

/// Translation of `UpdateWindowsRawKeyboardInputsink()`.
fn update_windows_raw_keyboard_inputsink(data: &Arc<VideoData>, new_value: Option<&str>) {
    let enabled = hints::string_to_bool(new_value, false);
    let _ = rawinput::set_raw_keyboard_flag_inputsink(data, enabled);
}

/// Translation of `UpdateWindowsRawMouseNoLegacy()`.
fn update_windows_raw_mouse_no_legacy(data: &Arc<VideoData>, new_value: Option<&str>) {
    let enabled = hints::string_to_bool(new_value, false);
    let _ = rawinput::set_raw_mouse_flag_no_legacy(data, enabled);
}

/// Translation of `UpdateWindowsEnableMessageLoop()`.
fn update_windows_enable_message_loop(new_value: Option<&str>) {
    G_WINDOWS_ENABLE_MESSAGE_LOOP.store(hints::string_to_bool(new_value, true), Ordering::Relaxed);
}

/// Translation of `UpdateWindowsEnableMenuMnemonics()`.
fn update_windows_enable_menu_mnemonics(new_value: Option<&str>) {
    G_WINDOWS_ENABLE_MENU_MNEMONICS
        .store(hints::string_to_bool(new_value, false), Ordering::Relaxed);
}

/// Translation of `UpdateWindowFrameUsableWhileCursorHidden()`.
fn update_window_frame_usable_while_cursor_hidden(new_value: Option<&str>) {
    G_WINDOW_FRAME_USABLE_WHILE_CURSOR_HIDDEN
        .store(hints::string_to_bool(new_value, true), Ordering::Relaxed);
}

/// Translation of `WIN_SuspendScreenSaver()`.
fn suspend_screen_saver(suspend: bool) -> Result<()> {
    // SAFETY: SetThreadExecutionState takes flags only.
    let result = unsafe {
        if suspend {
            SetThreadExecutionState(ES_CONTINUOUS | ES_DISPLAY_REQUIRED)
        } else {
            SetThreadExecutionState(ES_CONTINUOUS)
        }
    };
    if result == 0 {
        return Err(Error::new("SetThreadExecutionState() failed"));
    }
    Ok(())
}

// Windows driver bootstrap functions

/// The Windows video driver (the function pointers of the
/// `SDL_VideoDevice` that `WIN_CreateDevice()` fills in).
pub(crate) struct WindowsVideo {
    data: Arc<VideoData>,
}

impl Drop for WindowsVideo {
    /// Translation of `WIN_DeleteDevice()` (the libraries are unloaded
    /// when the last reference to the driver data goes).
    fn drop(&mut self) {
        events::unregister_app();
        let mut current = VIDEO_DATA.lock().unwrap_or_else(|e| e.into_inner());
        if current.as_ref().is_some_and(|d| Arc::ptr_eq(d, &self.data)) {
            *current = None;
        }
    }
}

/// Translation of `WIN_CreateDevice()`.
fn create_device() -> Option<Arc<dyn VideoDriver>> {
    let _ = events::register_app(None, 0, None);

    let user_dll = SharedObject::load("USER32.DLL").ok();
    let shcore_dll = SharedObject::load("SHCORE.DLL").ok();
    let dwmapi_dll = SharedObject::load("DWMAPI.DLL").ok();

    // SAFETY: each function type is the documented signature of the export.
    let data = unsafe {
        let user = user_dll.as_ref();
        let shcore = shcore_dll.as_ref();
        let dwmapi = dwmapi_dll.as_ref();
        VideoData {
            close_touch_input_handle: load_function(user, "CloseTouchInputHandle"),
            get_touch_input_info: load_function(user, "GetTouchInputInfo"),
            register_touch_window: load_function(user, "RegisterTouchWindow"),
            set_process_dpi_aware: load_function(user, "SetProcessDPIAware"),
            set_process_dpi_awareness_context: load_function(user, "SetProcessDpiAwarenessContext"),
            set_thread_dpi_awareness_context: load_function(user, "SetThreadDpiAwarenessContext"),
            get_thread_dpi_awareness_context: load_function(user, "GetThreadDpiAwarenessContext"),
            get_awareness_from_dpi_awareness_context: load_function(
                user,
                "GetAwarenessFromDpiAwarenessContext",
            ),
            enable_non_client_dpi_scaling: load_function(user, "EnableNonClientDpiScaling"),
            adjust_window_rect_ex_for_dpi: load_function(user, "AdjustWindowRectExForDpi"),
            get_dpi_for_window: load_function(user, "GetDpiForWindow"),
            are_dpi_awareness_contexts_equal: load_function(user, "AreDpiAwarenessContextsEqual"),
            is_valid_dpi_awareness_context: load_function(user, "IsValidDpiAwarenessContext"),
            get_display_config_buffer_sizes: load_function(user, "GetDisplayConfigBufferSizes"),
            query_display_config: load_function(user, "QueryDisplayConfig"),
            display_config_get_device_info: load_function(user, "DisplayConfigGetDeviceInfo"),
            get_pointer_type: load_function(user, "GetPointerType"),
            get_pointer_pen_info: load_function(user, "GetPointerPenInfo"),
            get_pointer_device_rects: load_function(user, "GetPointerDeviceRects"),

            get_dpi_for_monitor: load_function(shcore, "GetDpiForMonitor"),
            set_process_dpi_awareness: load_function(shcore, "SetProcessDpiAwareness"),

            dwm_flush: load_function(dwmapi, "DwmFlush"),
            dwm_enable_blur_behind_window: load_function(dwmapi, "DwmEnableBlurBehindWindow"),
            dwm_set_window_attribute: load_function(dwmapi, "DwmSetWindowAttribute"),

            state: Shared::new(VideoState {
                coinitialized: false,
                oleinitialized: false,
                clipboard_count: 0,
                cleared: false,
                detect_device_hotplug: false,
                raw_mouse_enabled: false,
                raw_mouse_flag_nolegacy: false,
                raw_keyboard_enabled: false,
                raw_keyboard_flag_nohotkeys: false,
                raw_keyboard_flag_inputsink: false,
                pre_hook_key_state: [0; 256],
                sdl_wakeup: 0,
                wm_taskbar_button_created: 0,
                taskbar_list: std::ptr::null_mut(),
                hint_callbacks: Vec::new(),
            }),
            raw: Shared::new(events::RawInputData::new()),
            ime: Shared::new(keyboard::ImeData::new()),

            _user_dll: user_dll,
            _shcore_dll: shcore_dll,
            _dwmapi_dll: dwmapi_dll,
        }
    };
    let data = Arc::new(data);
    *VIDEO_DATA.lock().unwrap_or_else(|e| e.into_inner()) = Some(data.clone());

    Some(Arc::new(WindowsVideo { data }))
}

/// Translation of `WINDOWS_bootstrap`.
pub(crate) static WINDOWS_BOOTSTRAP: VideoBootStrap = VideoBootStrap {
    name: "windows",
    desc: "SDL Windows video driver",
    create: create_device,
    show_message_box: Some(messagebox::show_message_box),
    is_preferred: false,
};

/// Translation of `WIN_DeclareDPIAwareUnaware()`.
fn declare_dpi_aware_unaware(data: &VideoData) -> bool {
    // SAFETY: the functions were loaded with these signatures.
    unsafe {
        if let Some(f) = data.set_process_dpi_awareness_context {
            return f(DPI_AWARENESS_CONTEXT_UNAWARE) != 0;
        } else if let Some(f) = data.set_process_dpi_awareness {
            // Windows 8.1
            return f(PROCESS_DPI_UNAWARE) >= 0;
        }
    }
    false
}

/// Translation of `WIN_DeclareDPIAwareSystem()`.
fn declare_dpi_aware_system(data: &VideoData) -> bool {
    // SAFETY: the functions were loaded with these signatures.
    unsafe {
        if let Some(f) = data.set_process_dpi_awareness_context {
            // Windows 10, version 1607
            return f(DPI_AWARENESS_CONTEXT_SYSTEM_AWARE) != 0;
        } else if let Some(f) = data.set_process_dpi_awareness {
            // Windows 8.1
            return f(PROCESS_SYSTEM_DPI_AWARE) >= 0;
        } else if let Some(f) = data.set_process_dpi_aware {
            // Windows Vista
            return f() != 0;
        }
    }
    false
}

/// Translation of `WIN_DeclareDPIAwarePerMonitor()`.
fn declare_dpi_aware_per_monitor(data: &VideoData) -> bool {
    // SAFETY: the functions were loaded with these signatures.
    unsafe {
        if let Some(f) = data.set_process_dpi_awareness_context {
            // Windows 10, version 1607
            f(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE) != 0
        } else if let Some(f) = data.set_process_dpi_awareness {
            // Windows 8.1
            f(PROCESS_PER_MONITOR_DPI_AWARE) >= 0
        } else {
            // Older OS: fall back to system DPI aware
            declare_dpi_aware_system(data)
        }
    }
}

/// Translation of `WIN_DeclareDPIAwarePerMonitorV2()`.
fn declare_dpi_aware_per_monitor_v2(data: &VideoData) -> bool {
    // Declare DPI aware (may have been done in external code or a manifest, as well)
    if let Some(f) = data.set_process_dpi_awareness_context {
        // Windows 10, version 1607

        /* NOTE: SetThreadDpiAwarenessContext doesn't work here with OpenGL - the OpenGL contents
          end up still getting OS scaled. (tested on Windows 10 21H1 19043.1348, NVIDIA 496.49)

          NOTE: Enabling DPI awareness through Windows Explorer
          (right click .exe -> Properties -> Compatibility -> High DPI Settings ->
          check "Override high DPI Scaling behaviour", select Application) gives
          a DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE context (at least on Windows 10 21H1), and
          setting DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2 will fail.

          NOTE: Entering exclusive fullscreen in a DPI_AWARENESS_CONTEXT_UNAWARE process
          appears to cause Windows to change the .exe manifest to DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE
          on future launches. This means attempting to use DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2
          will fail in the future until you manually clear the "Override high DPI Scaling behaviour"
          setting in Windows Explorer (tested on Windows 10 21H2).
        */
        // SAFETY: the function was loaded with this signature.
        if unsafe { f(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) } != 0 {
            true
        } else {
            declare_dpi_aware_per_monitor(data)
        }
    } else {
        // Older OS: fall back to per-monitor (or system)
        declare_dpi_aware_per_monitor(data)
    }
}

/// Translation of `WIN_InitDPIAwareness()`.
fn init_dpi_awareness(data: &VideoData) {
    let hint = hints::get("SDL_WINDOWS_DPI_AWARENESS");

    match hint.as_deref() {
        None | Some("permonitorv2") => {
            declare_dpi_aware_per_monitor_v2(data);
        }
        Some("permonitor") => {
            declare_dpi_aware_per_monitor(data);
        }
        Some("system") => {
            declare_dpi_aware_system(data);
        }
        Some("unaware") => {
            declare_dpi_aware_unaware(data);
        }
        Some(_) => {}
    }
}

/// Translation of `WIN_VideoInit()`.
fn video_init(data: &Arc<VideoData>) -> Result<()> {
    let hr = co_initialize();
    if hr >= 0 {
        data.state.with(|s| s.coinitialized = true);

        // SAFETY: OleInitialize takes a reserved NULL.
        let hr = unsafe { OleInitialize(std::ptr::null_mut()) };
        if hr >= 0 {
            data.state.with(|s| s.oleinitialized = true);
        } else {
            crate::log::info!(
                Category::Video,
                "OleInitialize() failed: 0x{:08x}, using fallback drag-n-drop functionality",
                hr as u32
            );
        }
    } else {
        crate::log::info!(
            Category::Video,
            "CoInitialize() failed: 0x{:08x}, using fallback drag-n-drop functionality",
            hr as u32
        );
    }

    init_dpi_awareness(data);

    if hints::get_bool(hints::WINDOWS_GAMEINPUT, false) {
        // (WIN_InitGameInput(): GameInput isn't translated, which is what
        // upstream does without HAVE_GAMEINPUT_H)
    }

    modes::init_modes(data)?;

    let detect = hints::get_bool("SDL_WINDOWS_DETECT_DEVICE_HOTPLUG", true);
    data.state.with(|s| s.detect_device_hotplug = detect);

    keyboard::init_keyboard(data);
    mouse::init_mouse(data);
    crate::core::windows::hid::init_device_notification();

    let mut callbacks = Vec::new();
    type HintFn = Box<dyn Fn(Option<&str>) + Send + Sync>;
    let watch = |name: &str, f: HintFn| hints::watch(name, move |change| f(change.new_value)).ok();
    let d = data.clone();
    callbacks.extend(watch(
        hints::WINDOWS_RAW_KEYBOARD,
        Box::new(move |v| update_windows_raw_keyboard(&d, v)),
    ));
    let d = data.clone();
    callbacks.extend(watch(
        hints::WINDOWS_RAW_KEYBOARD_EXCLUDE_HOTKEYS,
        Box::new(move |v| update_windows_raw_keyboard_no_hotkeys(&d, v)),
    ));
    let d = data.clone();
    callbacks.extend(watch(
        hints::WINDOWS_RAW_KEYBOARD_INPUTSINK,
        Box::new(move |v| update_windows_raw_keyboard_inputsink(&d, v)),
    ));
    let d = data.clone();
    callbacks.extend(watch(
        hints::WINDOWS_RAW_MOUSE_NOLEGACY,
        Box::new(move |v| update_windows_raw_mouse_no_legacy(&d, v)),
    ));
    callbacks.extend(watch(
        hints::WINDOWS_ENABLE_MESSAGELOOP,
        Box::new(update_windows_enable_message_loop),
    ));
    callbacks.extend(watch(
        hints::WINDOWS_ENABLE_MENU_MNEMONICS,
        Box::new(update_windows_enable_menu_mnemonics),
    ));
    callbacks.extend(watch(
        hints::WINDOW_FRAME_USABLE_WHILE_CURSOR_HIDDEN,
        Box::new(update_window_frame_usable_while_cursor_hidden),
    ));

    // SAFETY: the names are NUL-terminated.
    let (wakeup, taskbar) = unsafe {
        (
            RegisterWindowMessageA(c"_SDL_WAKEUP".as_ptr().cast()),
            if is_windows_7_or_greater() {
                RegisterWindowMessageA(c"TaskbarButtonCreated".as_ptr().cast())
            } else {
                0
            },
        )
    };
    data.state.with(|s| {
        s.hint_callbacks = callbacks;
        s.sdl_wakeup = wakeup;
        s.wm_taskbar_button_created = taskbar;
    });

    Ok(())
}

/// Translation of `WIN_VideoQuit()`.
fn video_quit(data: &Arc<VideoData>) {
    // (the raw mouse "no legacy" callback is removed too: upstream leaks it)
    // FIXME (upstream): WIN_VideoQuit() never removes the
    // SDL_HINT_WINDOWS_RAW_MOUSE_NOLEGACY callback it added.
    let callbacks = data.state.with(|s| std::mem::take(&mut s.hint_callbacks));
    drop(callbacks);

    rawinput::quit_raw_input(data);
    // (WIN_QuitGameInput(): nothing to do, see above)

    modes::quit_modes(data);
    crate::core::windows::hid::quit_device_notification();
    keyboard::quit_keyboard(data);
    mouse::quit_mouse(data);

    let (taskbar_list, oleinitialized, coinitialized) = data.state.with(|s| {
        let t = std::mem::replace(&mut s.taskbar_list, std::ptr::null_mut());
        let o = std::mem::take(&mut s.oleinitialized);
        let c = std::mem::take(&mut s.coinitialized);
        (t, o, c)
    });
    if !taskbar_list.is_null() {
        // SAFETY: taskbar_list is a live ITaskbarList3 we own a reference to.
        unsafe { ((*(*taskbar_list).vtbl).release)(taskbar_list) };
    }

    if oleinitialized {
        // SAFETY: balances the successful OleInitialize() in video_init().
        unsafe { OleUninitialize() };
    }

    if coinitialized {
        co_uninitialize();
    }
}

/// The system theme, from the apps' light theme preference. Translation of
/// `WIN_GetSystemTheme()`.
pub(crate) fn get_system_theme() -> SystemTheme {
    let mut theme = SystemTheme::Light;
    let mut hkey: HKEY = std::ptr::null_mut();
    let mut dw_type = REG_DWORD;
    let mut value: u32 = !0u32;
    let mut length = size_of::<u32>() as u32;

    // Technically this isn't the system theme, but it's the preference for applications
    let key = crate::core::windows::utf8_to_wide(
        "Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize",
    );
    let name = crate::core::windows::utf8_to_wide("AppsUseLightTheme");
    // SAFETY: the strings are NUL-terminated; value holds length bytes.
    unsafe {
        if RegOpenKeyExW(HKEY_CURRENT_USER, key.as_ptr(), 0, KEY_READ, &mut hkey) == ERROR_SUCCESS {
            if RegQueryValueExW(
                hkey,
                name.as_ptr(),
                std::ptr::null(),
                &mut dw_type,
                (&mut value as *mut u32).cast(),
                &mut length,
            ) == ERROR_SUCCESS
                && value == 0
            {
                theme = SystemTheme::Dark;
            }
            RegCloseKey(hkey);
        }
    }
    theme
}

/// Whether the thread runs with per-monitor v2 DPI awareness. Translation of
/// `WIN_IsPerMonitorV2DPIAware()`.
pub(crate) fn is_per_monitor_v2_dpi_aware(data: &VideoData) -> bool {
    if let (Some(equal), Some(get)) = (
        data.are_dpi_awareness_contexts_equal,
        data.get_thread_dpi_awareness_context,
    ) {
        // Windows 10, version 1607
        // SAFETY: the functions were loaded with these signatures.
        return unsafe { equal(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, get()) } != 0;
    }
    false
}

/// `WIN_IsPerMonitorV2DPIAware(SDL_GetVideoDevice())`: false when the
/// Windows driver isn't running.
pub(crate) fn current_is_per_monitor_v2_dpi_aware() -> bool {
    video_data().is_some_and(|d| is_per_monitor_v2_dpi_aware(&d))
}

impl VideoDriver for WindowsVideo {
    fn caps(&self) -> DeviceCaps {
        DeviceCaps::HAS_POPUP_WINDOW_SUPPORT | DeviceCaps::SENDS_FULLSCREEN_DIMENSIONS
    }

    fn video_init(&self) -> Result<()> {
        // (WIN_CreateDevice() sets device->system_theme, which exists
        // once the driver is chosen; no event is sent)
        let theme = get_system_theme();
        let _ = crate::video::core::with_device(|v| v.system_theme = theme);
        video_init(&self.data)
    }

    fn video_quit(&self) {
        video_quit(&self.data)
    }

    // Display functions

    fn refresh_displays(&self) -> Option<()> {
        modes::refresh_displays(&self.data);
        Some(())
    }

    fn display_bounds(&self, display: DisplayID) -> Option<Result<Rect>> {
        Some(modes::get_display_bounds(display))
    }

    fn display_usable_bounds(&self, display: DisplayID) -> Option<Result<Rect>> {
        Some(modes::get_display_usable_bounds(display))
    }

    fn display_modes(&self, display: DisplayID) -> Option<()> {
        modes::get_display_modes(&self.data, display);
        Some(())
    }

    fn set_display_mode(&self, display: DisplayID, mode: &DisplayMode) -> Option<Result<()>> {
        Some(modes::set_display_mode(&self.data, display, mode))
    }

    // Window functions

    fn create_window(&self, window: WindowID, create_props: &Properties) -> Option<Result<()>> {
        Some(window::create_window(&self.data, window, create_props))
    }
    fn set_window_title(&self, window: WindowID) -> Option<()> {
        window::set_window_title(window);
        Some(())
    }
    fn set_window_icon(&self, window: WindowID, icon: &Surface<'static>) -> Option<Result<()>> {
        Some(window::set_window_icon(window, icon))
    }
    fn set_window_position(&self, window: WindowID) -> Option<Result<()>> {
        Some(window::set_window_position(window))
    }
    fn set_window_size(&self, window: WindowID) -> Option<()> {
        window::set_window_size(window);
        Some(())
    }
    fn window_borders_size(&self, window: WindowID) -> Option<Result<(i32, i32, i32, i32)>> {
        Some(window::get_window_borders_size(window))
    }
    fn window_size_in_pixels(&self, window: WindowID) -> Option<(i32, i32)> {
        window::get_window_size_in_pixels(window)
    }
    fn set_window_opacity(&self, window: WindowID, opacity: f32) -> Option<Result<()>> {
        Some(window::set_window_opacity(window, opacity))
    }
    fn set_window_parent(&self, window: WindowID, parent: Option<WindowID>) -> Option<Result<()>> {
        Some(window::set_window_parent(window, parent))
    }
    fn set_window_modal(&self, window: WindowID, modal: bool) -> Option<Result<()>> {
        Some(window::set_window_modal(window, modal))
    }
    fn show_window(&self, window: WindowID) -> Option<()> {
        window::show_window(window);
        Some(())
    }
    fn hide_window(&self, window: WindowID) -> Option<()> {
        window::hide_window(window);
        Some(())
    }
    fn raise_window(&self, window: WindowID) -> Option<()> {
        window::raise_window(window);
        Some(())
    }
    fn maximize_window(&self, window: WindowID) -> Option<()> {
        window::maximize_window(window);
        Some(())
    }
    fn minimize_window(&self, window: WindowID) -> Option<()> {
        window::minimize_window(window);
        Some(())
    }
    fn restore_window(&self, window: WindowID) -> Option<()> {
        window::restore_window(window);
        Some(())
    }
    fn set_window_bordered(&self, window: WindowID, bordered: bool) -> Option<()> {
        window::set_window_bordered(window, bordered);
        Some(())
    }
    fn set_window_resizable(&self, window: WindowID, resizable: bool) -> Option<()> {
        window::set_window_resizable(window, resizable);
        Some(())
    }
    fn set_window_always_on_top(&self, window: WindowID, on_top: bool) -> Option<()> {
        window::set_window_always_on_top(window, on_top);
        Some(())
    }
    fn set_window_fullscreen(
        &self,
        window: WindowID,
        display: DisplayID,
        fullscreen: FullscreenOp,
    ) -> Option<FullscreenResult> {
        Some(window::set_window_fullscreen(
            &self.data, window, display, fullscreen,
        ))
    }
    fn window_icc_profile(&self, window: WindowID) -> Option<Result<Vec<u8>>> {
        Some(window::get_window_icc_profile(window))
    }
    fn set_window_mouse_rect(&self, window: WindowID) -> Option<Result<()>> {
        Some(window::set_window_mouse_rect(window))
    }
    fn set_window_mouse_grab(&self, window: WindowID, grabbed: bool) -> Option<Result<()>> {
        Some(window::set_window_mouse_grab(window, grabbed))
    }
    fn set_window_keyboard_grab(&self, window: WindowID, grabbed: bool) -> Option<Result<()>> {
        Some(window::set_window_keyboard_grab(
            &self.data, window, grabbed,
        ))
    }
    fn destroy_window(&self, window: WindowID) -> Option<()> {
        window::destroy_window(window);
        Some(())
    }
    fn create_window_framebuffer(
        &self,
        window: WindowID,
        w: i32,
        h: i32,
    ) -> Option<Result<Surface<'static>>> {
        Some(framebuffer::create_window_framebuffer(window, w, h))
    }
    fn update_window_framebuffer(
        &self,
        window: WindowID,
        surface: &Surface<'static>,
        rects: &[Rect],
    ) -> Option<Result<()>> {
        Some(framebuffer::update_window_framebuffer(
            window, surface, rects,
        ))
    }
    fn destroy_window_framebuffer(&self, window: WindowID) -> Option<()> {
        framebuffer::destroy_window_framebuffer(window);
        Some(())
    }
    fn on_window_enter(&self, window: WindowID) -> Option<()> {
        window::on_window_enter(window);
        Some(())
    }
    fn update_window_shape(
        &self,
        window: WindowID,
        shape: Option<&Surface<'static>>,
    ) -> Option<Result<()>> {
        Some(shape::update_window_shape(window, shape))
    }
    fn flash_window(&self, window: WindowID, operation: FlashOperation) -> Option<Result<()>> {
        Some(window::flash_window(window, operation))
    }
    fn apply_window_progress(&self, window: WindowID) -> Option<Result<()>> {
        Some(window::apply_window_progress(&self.data, window))
    }
    fn set_window_focusable(&self, window: WindowID, focusable: bool) -> Option<Result<()>> {
        Some(window::set_window_focusable(window, focusable))
    }

    // Vulkan support

    fn implements_vulkan_surfaces(&self) -> bool {
        true
    }
    fn vulkan_load_library(&self, path: Option<&str>) -> Option<Result<()>> {
        Some(vulkan::vulkan_load_library(path))
    }
    fn vulkan_unload_library(&self) -> Option<()> {
        vulkan::vulkan_unload_library();
        Some(())
    }
    fn vulkan_get_instance_proc_addr(&self) -> Option<usize> {
        vulkan::vk_get_instance_proc_addr()
    }
    fn vulkan_instance_extensions(&self) -> Option<Vec<&'static str>> {
        Some(vulkan::vulkan_get_instance_extensions())
    }
    fn vulkan_create_surface(
        &self,
        window: WindowID,
        instance: usize,
        allocator: usize,
    ) -> Option<Result<u64>> {
        Some(vulkan::vulkan_create_surface(window, instance, allocator))
    }
    fn vulkan_destroy_surface(
        &self,
        instance: usize,
        surface: u64,
        allocator: usize,
    ) -> Option<()> {
        vulkan::vulkan_destroy_surface(instance, surface, allocator);
        Some(())
    }
    fn vulkan_presentation_support(
        &self,
        instance: usize,
        physical_device: usize,
        queue_family_index: u32,
    ) -> Option<bool> {
        Some(vulkan::vulkan_get_presentation_support(
            instance,
            physical_device,
            queue_family_index,
        ))
    }

    // Event manager functions

    fn wait_event_timeout(&self, timeout: Option<Duration>) -> Option<i32> {
        Some(events::wait_event_timeout(timeout))
    }
    fn send_wakeup_event(&self, window: WindowID) -> Option<()> {
        events::send_wakeup_event(&self.data, window);
        Some(())
    }
    fn pump_events(&self) -> Option<()> {
        events::pump_events(&self.data);
        Some(())
    }

    fn suspend_screen_saver(&self, suspend: bool) -> Option<Result<()>> {
        Some(suspend_screen_saver(suspend))
    }

    // Hit-testing, drag and drop, the system menu

    fn set_window_hit_test(&self, window: WindowID, enabled: bool) -> Option<Result<()>> {
        Some(window::set_window_hit_test(window, enabled))
    }
    fn accept_drag_and_drop(&self, window: WindowID, accept: bool) -> Option<()> {
        window::accept_drag_and_drop(&self.data, window, accept);
        Some(())
    }
    fn show_window_system_menu(&self, window: WindowID, x: i32, y: i32) -> Option<()> {
        window::show_window_system_menu(window, x, y);
        Some(())
    }

    // The SDL_Mouse entry points

    fn warp_mouse(&self, window: WindowID, x: f32, y: f32) -> Option<Result<()>> {
        Some(mouse::warp_mouse(window, x, y))
    }
    fn warp_mouse_global(&self, x: f32, y: f32) -> Option<Result<()>> {
        Some(mouse::warp_mouse_global(x, y))
    }
    fn set_relative_mouse_mode(&self, enabled: bool) -> Option<Result<()>> {
        Some(mouse::set_relative_mouse_mode(&self.data, enabled))
    }
    fn capture_mouse(&self, window: Option<WindowID>) -> Option<Result<()>> {
        Some(mouse::capture_mouse(window))
    }
    fn global_mouse_state(&self) -> Option<(f32, f32, MouseButtonFlags)> {
        Some(mouse::get_global_mouse_state())
    }
    fn apply_system_scale(
        &self,
        timestamp: Duration,
        window: Option<WindowID>,
        mouse_id: MouseID,
        x: f32,
        y: f32,
    ) -> Option<(f32, f32)> {
        Some(mouse::apply_system_scale(
            &self.data, timestamp, window, mouse_id, x, y,
        ))
    }
    fn create_cursor(
        &self,
        surface: &Surface<'_>,
        hot_x: i32,
        hot_y: i32,
    ) -> Option<Result<Cursor>> {
        Some(mouse::create_cursor(surface, hot_x, hot_y))
    }
    fn create_animated_cursor(
        &self,
        frames: &[CursorFrame<'_>],
        hot_x: i32,
        hot_y: i32,
    ) -> Option<Result<Cursor>> {
        Some(mouse::create_animated_cursor(frames, hot_x, hot_y))
    }
    fn create_system_cursor(&self, id: SystemCursor) -> Option<Result<Cursor>> {
        Some(mouse::create_system_cursor(id))
    }
    fn show_cursor(&self, cursor: Option<&Cursor>) -> Option<Result<()>> {
        Some(mouse::show_cursor(cursor))
    }
    fn implements_mouse_feature(&self, feature: MouseFeature) -> bool {
        matches!(
            feature,
            MouseFeature::WarpMouse
                | MouseFeature::WarpMouseGlobal
                | MouseFeature::SetRelativeMouseMode
                | MouseFeature::CaptureMouse
                | MouseFeature::GetGlobalMouseState
                | MouseFeature::ApplySystemScale
        )
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
    fn implements_window_framebuffer(&self) -> bool {
        true
    }
    fn can_wait_events(&self) -> bool {
        true
    }

    // Text input

    fn start_text_input(&self, window: WindowID, props: Option<&Properties>) -> Option<Result<()>> {
        Some(keyboard::start_text_input(&self.data, window, props))
    }
    fn stop_text_input(&self, window: WindowID) -> Option<Result<()>> {
        Some(keyboard::stop_text_input(&self.data, window))
    }
    fn update_text_input_area(&self, window: WindowID) -> Option<Result<()>> {
        Some(keyboard::update_text_input_area(&self.data, window))
    }
    fn clear_composition(&self, window: WindowID) -> Option<Result<()>> {
        Some(keyboard::clear_composition(&self.data, window))
    }

    // Clipboard

    fn set_clipboard_data(&self) -> Option<Result<()>> {
        Some(clipboard::set_clipboard_data(&self.data))
    }
    fn clipboard_data(&self, mime_type: &str) -> Option<Option<Vec<u8>>> {
        Some(clipboard::get_clipboard_data(mime_type))
    }
    fn has_clipboard_data(&self, mime_type: &str) -> Option<bool> {
        Some(clipboard::has_clipboard_data(mime_type))
    }
}
