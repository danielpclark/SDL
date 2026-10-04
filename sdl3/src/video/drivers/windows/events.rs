// Rust translation of src/video/windows/SDL_windowsevents.c and
// SDL_windowsevents.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The window procedure and the message pump: window state, focus, mouse,
//! keyboard, text, pen, touch, drag and drop and DPI messages become SDL
//! events; raw input is polled in bulk; the window class is registered for
//! the application (`SDL_RegisterApp()`).

use std::any::Any;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
    SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInfo, SetupDiGetClassDevsA,
    SetupDiGetDeviceInstanceIdA, SetupDiGetDeviceRegistryPropertyW, DIGCF_ALLCLASSES,
    DIGCF_PRESENT, HDEVINFO, SPDRP_DEVICEDESC, SP_DEVINFO_DATA,
};
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_INSUFFICIENT_BUFFER, ERROR_NO_MORE_ITEMS, HANDLE, HINSTANCE,
    HWND, INVALID_HANDLE_VALUE, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM,
};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, ClientToScreen, CreateSolidBrush, DeleteObject, EndPaint, FillRect, GetDC,
    GetMonitorInfoW, GetUpdateRect, MonitorFromPoint, MonitorFromWindow, ScreenToClient,
    ValidateRect, MONITORINFO, MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTONULL, PAINTSTRUCT,
};
use windows_sys::Win32::Graphics::Gdi::{SetRectEmpty, SC_SCREENSAVE};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileA, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::System::LibraryLoader::EnumResourceNamesW;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::SystemInformation::GetTickCount;
use windows_sys::Win32::System::SystemServices::{
    MK_LBUTTON, MK_MBUTTON, MK_RBUTTON, MK_XBUTTON1, MK_XBUTTON2,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, IsWow64Process, INFINITE};
use windows_sys::Win32::UI::Controls::WM_MOUSELEAVE;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, GetKeyState, MapVirtualKeyW, TrackMouseEvent, MAPVK_VK_TO_VSC,
    MAPVK_VK_TO_VSC_EX, TME_LEAVE, TRACKMOUSEEVENT, VK_CAPITAL, VK_CONTROL, VK_ESCAPE, VK_LBUTTON,
    VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_LWIN, VK_MBUTTON, VK_MENU, VK_NUMLOCK, VK_RBUTTON,
    VK_RCONTROL, VK_RMENU, VK_RSHIFT, VK_RWIN, VK_SCROLL, VK_SNAPSHOT, VK_TAB, VK_XBUTTON1,
    VK_XBUTTON2,
};
use windows_sys::Win32::UI::Input::Pointer::POINTER_PEN_INFO;
use windows_sys::Win32::UI::Input::Touch::{
    HTOUCHINPUT, TOUCHEVENTF_DOWN, TOUCHEVENTF_MOVE, TOUCHEVENTF_UP, TOUCHINPUT,
};
use windows_sys::Win32::UI::Input::{
    GetRawInputBuffer, GetRawInputDeviceInfoA, GetRawInputDeviceList, RAWINPUTDEVICELIST,
    RAWINPUTHEADER, RAWKEYBOARD, RAWMOUSE, RIDI_DEVICEINFO, RIDI_DEVICENAME, RID_DEVICE_INFO,
    RIM_TYPEKEYBOARD, RIM_TYPEMOUSE,
};
use windows_sys::Win32::UI::Shell::{DragFinish, DragQueryFileW, HDROP};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AdjustWindowRectEx, CallNextHookEx, CallWindowProcW, DefWindowProcW, DestroyIcon,
    DispatchMessageW, GetClassInfoExW, GetClientRect, GetCursorPos, GetMenu, GetMessageExtraInfo,
    GetMessagePos, GetMessageTime, GetSystemMetrics, GetWindowPlacement, GetWindowRect, IsIconic,
    IsZoomed, KillTimer, LoadIconW, MsgWaitForMultipleObjects, PeekMessageW, PostMessageW,
    RegisterClassExW, SendMessageW, SetCursor, SetTimer, SetWindowPos, TranslateMessage,
    UnregisterClassW, CS_BYTEALIGNCLIENT, CS_OWNDC, GWL_EXSTYLE, GWL_STYLE, HC_ACTION, HTBOTTOM,
    HTBOTTOMLEFT, HTBOTTOMRIGHT, HTCAPTION, HTCLIENT, HTLEFT, HTRIGHT, HTTOP, HTTOPLEFT,
    HTTOPRIGHT, HTTRANSPARENT, KBDLLHOOKSTRUCT, KF_EXTENDED, MA_NOACTIVATE, MINMAXINFO, MSG,
    NCCALCSIZE_PARAMS, PM_NOREMOVE, PM_REMOVE, QS_ALLINPUT, RI_KEY_BREAK, RI_KEY_E0, RI_KEY_E1,
    RI_MOUSE_BUTTON_4_DOWN, RI_MOUSE_BUTTON_4_UP, RI_MOUSE_BUTTON_5_DOWN, RI_MOUSE_BUTTON_5_UP,
    RI_MOUSE_HWHEEL, RI_MOUSE_LEFT_BUTTON_DOWN, RI_MOUSE_LEFT_BUTTON_UP,
    RI_MOUSE_MIDDLE_BUTTON_DOWN, RI_MOUSE_MIDDLE_BUTTON_UP, RI_MOUSE_RIGHT_BUTTON_DOWN,
    RI_MOUSE_RIGHT_BUTTON_UP, RI_MOUSE_WHEEL, RT_GROUP_ICON, SC_KEYMENU, SC_MONITORPOWER,
    SM_CXSCREEN, SM_CXVIRTUALSCREEN, SM_CYSCREEN, SM_CYVIRTUALSCREEN, SM_REMOTESESSION,
    SM_SWAPBUTTON, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, SPI_SETMOUSE, SPI_SETMOUSESPEED,
    SPI_SETWORKAREA, SWP_HIDEWINDOW, SWP_NOACTIVATE, SWP_NOCOPYBITS, SWP_NOSIZE, SWP_NOZORDER,
    SWP_SHOWWINDOW, SW_MAXIMIZE, UNICODE_NOCHAR, WHEEL_DELTA, WINDOWPLACEMENT, WINDOWPOS,
    WMSZ_BOTTOM, WMSZ_BOTTOMLEFT, WMSZ_BOTTOMRIGHT, WMSZ_LEFT, WMSZ_RIGHT, WMSZ_TOP, WMSZ_TOPLEFT,
    WMSZ_TOPRIGHT, WM_ACTIVATE, WM_CAPTURECHANGED, WM_CHAR, WM_CLOSE, WM_DISPLAYCHANGE,
    WM_DPICHANGED, WM_DROPFILES, WM_ENTERIDLE, WM_ENTERMENULOOP, WM_ENTERSIZEMOVE, WM_ERASEBKGND,
    WM_EXITMENULOOP, WM_EXITSIZEMOVE, WM_GETDPISCALEDSIZE, WM_GETMINMAXINFO, WM_INPUTLANGCHANGE,
    WM_KEYDOWN, WM_KEYUP, WM_KILLFOCUS, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_MBUTTONDBLCLK, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEACTIVATE, WM_MOUSEHWHEEL, WM_MOUSEMOVE,
    WM_MOUSEWHEEL, WM_NCACTIVATE, WM_NCCALCSIZE, WM_NCHITTEST, WM_NCLBUTTONDOWN, WM_PAINT,
    WM_POINTERCAPTURECHANGED, WM_POINTERDOWN, WM_POINTERENTER, WM_POINTERLEAVE, WM_POINTERUP,
    WM_POINTERUPDATE, WM_RBUTTONDBLCLK, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SETCURSOR, WM_SETFOCUS,
    WM_SETTINGCHANGE, WM_SHOWWINDOW, WM_SIZING, WM_SYSCOMMAND, WM_SYSKEYDOWN, WM_SYSKEYUP,
    WM_TIMER, WM_TOUCH, WM_UNICHAR, WM_WINDOWPOSCHANGED, WM_WINDOWPOSCHANGING, WM_XBUTTONDBLCLK,
    WM_XBUTTONDOWN, WM_XBUTTONUP, WNDCLASSEXW,
};

use super::window::{
    adjust_window_rect_for_hwnd, is_popup, set_window_position_internal, set_window_resizable,
    unclip_cursor_for_window, update_clip_cursor, update_window_icc_profile, window_data,
    window_data_from_prop, window_flags, WindowData, WindowRect,
};
use super::{
    clipboard, get_system_theme, keyboard, modes, mouse as winmouse, video_data, VideoData,
    G_WINDOWS_ENABLE_MENU_MNEMONICS, G_WINDOWS_ENABLE_MESSAGE_LOOP,
    G_WINDOW_FRAME_USABLE_WHILE_CURSOR_HIDDEN,
};
use crate::core::windows::{
    is_windows_xp, update_dark_mode_for_hwnd, utf8_to_wide, wide_char_to_multi_byte, wide_to_utf8,
    window_rect_valid,
};
use crate::error::{Error, Result};
use crate::events::keyboard::{Keymod, Scancode, GLOBAL_KEYBOARD_ID};
use crate::events::mouse::{
    MouseButtonFlags, MouseWheelDirection, BUTTON_LEFT, BUTTON_MIDDLE, BUTTON_RIGHT, BUTTON_X1,
    BUTTON_X2, GLOBAL_MOUSE_ID, TOUCH_MOUSE_ID,
};
use crate::events::pen::{self, PenAxis, PenCapabilityFlags, PenID, PenInfo, PenSubtype};
use crate::events::scancodes_windows::WINDOWS_SCANCODE_TABLE;
use crate::events::touch::{self, TouchDeviceType};
use crate::events::window::{send_drop_complete, send_drop_file, send_window_event, WindowFlags};
use crate::events::{keyboard as sdl_keyboard, mouse, EventType, WindowID};
use crate::hints;
use crate::video::core::{self, with_device};
use crate::video::sysvideo::HitTestResult;
use crate::video::window::{global_to_relative_for_window, Window};
use crate::video::Point;

// Undocumented window messages
const WM_NCUAHDRAWCAPTION: u32 = 0xAE;
const WM_NCUAHDRAWFRAME: u32 = 0xAF;

// For WM_TABLET_QUERYSYSTEMGESTURESTATUS et. al. (tpcshrd.h)
const WM_TABLET_QUERYSYSTEMGESTURESTATUS: u32 = 0x02C0 + 12;
const TABLET_DISABLE_PRESSANDHOLD: isize = 0x00000001;
const TABLET_DISABLE_PENTAPFEEDBACK: isize = 0x00000008;
const TABLET_DISABLE_PENBARRELFEEDBACK: isize = 0x00000010;
const TABLET_DISABLE_TOUCHUIFORCEON: isize = 0x00000100;
const TABLET_DISABLE_TOUCHUIFORCEOFF: isize = 0x00000200;
const TABLET_DISABLE_TOUCHSWITCH: isize = 0x00008000;
const TABLET_DISABLE_FLICKS: isize = 0x00010000;
const TABLET_DISABLE_SMOOTHSCROLLING: isize = 0x00080000;
const TABLET_DISABLE_FLICKFALLBACKKEYS: isize = 0x00100000;

const TOUCHEVENTF_PEN: u32 = 0x0040;

// Pen input definitions (WINVER < _WIN32_WINNT_WIN8)
const POINTER_MESSAGE_FLAG_INCONTACT: u32 = 0x00000004;
const POINTER_MESSAGE_FLAG_FIRSTBUTTON: u32 = 0x00000010;
const PT_POINTER: i32 = 1;
const PT_PEN: i32 = 3;
const PT_MOUSE: i32 = 4;
const PEN_FLAG_BARREL: u32 = 0x00000001;
const PEN_FLAG_INVERTED: u32 = 0x00000002;
const PEN_FLAG_ERASER: u32 = 0x00000004;
const PEN_MASK_PRESSURE: u32 = 0x00000001;
const PEN_MASK_ROTATION: u32 = 0x00000002;
const PEN_MASK_TILT_X: u32 = 0x00000004;
const PEN_MASK_TILT_Y: u32 = 0x00000008;

/// Translation of `USER_TIMER_MINIMUM`.
const USER_TIMER_MINIMUM: u32 = 0x0000000A;

/// The timer upstream identifies by `SDL_IterateMainCallbacks`'s address.
fn main_callbacks_timer_id() -> usize {
    crate::app::iterate_main_callbacks as *const () as usize
}

/// `LOWORD()`
fn loword(v: usize) -> u16 {
    v as u16
}
/// `HIWORD()`
fn hiword(v: usize) -> u16 {
    (v >> 16) as u16
}
/// `GET_X_LPARAM()`
fn get_x_lparam(lp: isize) -> i32 {
    lp as u16 as i16 as i32
}
/// `GET_Y_LPARAM()`
fn get_y_lparam(lp: isize) -> i32 {
    (lp >> 16) as u16 as i16 as i32
}
/// `GET_POINTERID_WPARAM()`
fn get_pointerid_wparam(wparam: WPARAM) -> u32 {
    loword(wparam) as u32
}
/// `IS_POINTER_FLAG_SET_WPARAM()`
fn is_pointer_flag_set_wparam(wparam: WPARAM, flag: u32) -> bool {
    (hiword(wparam) as u32 & flag) == flag
}

/// Used to compare Windows message timestamps. Translation of
/// `SDL_TICKS_PASSED()`.
fn ticks_passed(a: u32, b: u32) -> bool {
    (b.wrapping_sub(a) as i32) <= 0
}

/// Translation of `SDL_processing_messages`.
static PROCESSING_MESSAGES: AtomicBool = AtomicBool::new(false);
/// Translation of `message_tick`.
static MESSAGE_TICK: AtomicU32 = AtomicU32::new(0);
/// Translation of `timestamp_offset`.
static TIMESTAMP_OFFSET: AtomicU64 = AtomicU64::new(0);

/// Translation of `WIN_SetMessageTick()`.
fn set_message_tick(tick: u32) {
    MESSAGE_TICK.store(tick, Ordering::Relaxed);
}

/// The SDL timestamp of the message being processed (zero: now).
/// Translation of `WIN_GetEventTimestamp()`.
fn get_event_timestamp() -> Duration {
    const TIMESTAMP_WRAP_OFFSET: u64 = 0x100000000u64 * 1_000_000;

    if !PROCESSING_MESSAGES.load(Ordering::Relaxed) {
        // message_tick isn't valid, just use the current time
        return Duration::ZERO;
    }

    let now = crate::timer::ticks_ns();
    let mut timestamp = MESSAGE_TICK.load(Ordering::Relaxed) as u64 * 1_000_000;
    let mut timestamp_offset = TIMESTAMP_OFFSET.load(Ordering::Relaxed);
    timestamp = timestamp.wrapping_add(timestamp_offset);
    if timestamp_offset == 0 {
        // Initializing timestamp offset
        //SDL_Log("Initializing timestamp offset");
        timestamp_offset = now.wrapping_sub(timestamp);
        timestamp = now;
    } else if (now
        .wrapping_sub(timestamp)
        .wrapping_sub(TIMESTAMP_WRAP_OFFSET) as i64)
        >= 0
    {
        // The windows message tick wrapped
        //SDL_Log("Adjusting timestamp offset for wrapping tick");
        timestamp_offset = timestamp_offset.wrapping_add(TIMESTAMP_WRAP_OFFSET);
        timestamp = timestamp.wrapping_add(TIMESTAMP_WRAP_OFFSET);
    } else if timestamp > now {
        // We got a newer timestamp, but it can't be newer than now, so adjust our offset
        //SDL_Log("Adjusting timestamp offset, %.2f ms newer", (double)(timestamp - now) / SDL_NS_PER_MS);
        timestamp_offset = timestamp_offset.wrapping_sub(timestamp - now);
        timestamp = now;
    }
    TIMESTAMP_OFFSET.store(timestamp_offset, Ordering::Relaxed);
    Duration::from_nanos(timestamp)
}

/// A callback run on every message before `TranslateMessage()`; it may
/// change the message, and returns `false` to drop it. Translation of
/// `SDL_WindowsMessageHook`.
pub type WindowsMessageHook = Arc<dyn Fn(&mut MSG) -> bool + Send + Sync>;

/// A message hook called before TranslateMessage(). Translation of
/// `g_WindowsMessageHook` (its `userdata` is the closure's).
static G_WINDOWS_MESSAGE_HOOK: Mutex<Option<WindowsMessageHook>> = Mutex::new(None);

/// Install (or, with `None`, remove) a callback for every Windows message
/// the event pump sees, before `TranslateMessage()`. Translation of
/// `SDL_SetWindowsMessageHook()`.
pub fn set_windows_message_hook(callback: Option<WindowsMessageHook>) {
    *G_WINDOWS_MESSAGE_HOOK
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = callback;
}

fn windows_message_hook() -> Option<WindowsMessageHook> {
    G_WINDOWS_MESSAGE_HOOK
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

/// The SDL scancode of a key message, with the raw scan code and whether
/// the key came without one (a virtual key). Translation of
/// `WindowsScanCodeToSDLScanCode()`.
pub(crate) fn windows_scan_code_to_sdl_scan_code(
    l_param: LPARAM,
    w_param: WPARAM,
) -> (Scancode, u16, bool) {
    let key_flags = hiword(l_param as usize);
    let mut scan_code = key_flags & 0xFF;

    /* On-Screen Keyboard can send wrong scan codes with high-order bit set (key break code).
     * Strip high-order bit. */
    scan_code &= !0x80;

    let virtual_key = scan_code == 0;

    if scan_code != 0 {
        if (key_flags as u32 & KF_EXTENDED) == KF_EXTENDED {
            scan_code |= 0xe0 << 8;
        } else if scan_code == 0x45 {
            // Pause
            scan_code = 0xe046;
        }
    } else {
        let vk_code = loword(w_param);

        /* Windows may not report scan codes for some buttons (multimedia buttons etc).
         * Get scan code from the VK code.*/
        // SAFETY: MapVirtualKeyW is a pure lookup.
        scan_code = unsafe {
            MapVirtualKeyW(
                vk_code as u32,
                if is_windows_xp() {
                    MAPVK_VK_TO_VSC
                } else {
                    MAPVK_VK_TO_VSC_EX
                },
            )
        } as u16;

        /* Pause/Break key have a special scan code with 0xe1 prefix.
         * Use Pause scan code that is used in Win32. */
        if scan_code == 0xe11d {
            scan_code = 0xe046;
        }
    }

    // Pack scan code into one byte to make the index.
    let index = (scan_code & 0xFF) as u8 | if (scan_code >> 8) != 0 { 0x80 } else { 0x00 };
    let code = WINDOWS_SCANCODE_TABLE[index as usize];

    (code, scan_code, virtual_key)
}

/// `SDL_BUTTON_MASK(button)`.
fn button_mask(button: u8) -> u8 {
    1u8 << (button - 1)
}

/// Translation of `WIN_ShouldIgnoreFocusClick()`.
fn should_ignore_focus_click(data: &WindowData) -> bool {
    !is_popup(data.window) && !hints::get_bool(hints::MOUSE_FOCUS_CLICKTHROUGH, false)
}

/// Translation of `WIN_CheckWParamMouseButton()`.
fn check_wparam_mouse_button(
    timestamp: Duration,
    b_wparam_mouse_pressed: bool,
    mouse_flags: MouseButtonFlags,
    b_swap_buttons: bool,
    data: &WindowData,
    mut button: u8,
    mouse_id: mouse::MouseID,
) {
    if b_swap_buttons {
        if button == BUTTON_LEFT {
            button = BUTTON_RIGHT;
        } else if button == BUTTON_RIGHT {
            button = BUTTON_LEFT;
        }
    }

    let mask = button_mask(button);
    if (data.state.with(|s| s.focus_click_pending) & mask) != 0 {
        // Ignore the button click for activation
        if !b_wparam_mouse_pressed {
            data.state.with(|s| s.focus_click_pending &= !mask);
            update_clip_cursor(data.window);
        }
        return;
    }

    let pressed = (mouse_flags.0 & mask as u32) != 0;
    if b_wparam_mouse_pressed && !pressed {
        mouse::send_mouse_button(timestamp, Some(data.window), mouse_id, button, true);
    } else if !b_wparam_mouse_pressed && pressed {
        mouse::send_mouse_button(timestamp, Some(data.window), mouse_id, button, false);
    }
}

/*
 * Some windows systems fail to send a WM_LBUTTONDOWN sometimes, but each mouse move contains the current button state also
 *  so this function reconciles our view of the world with the current buttons reported by windows
 */
/// Translation of `WIN_CheckWParamMouseButtons()`.
fn check_wparam_mouse_buttons(
    timestamp: Duration,
    w_param: WPARAM,
    data: &WindowData,
    mouse_id: mouse::MouseID,
) {
    if w_param != data.state.with(|s| s.mouse_button_flags) {
        let mouse_flags = mouse::mouse_state().2;

        // WM_LBUTTONDOWN and friends handle button swapping for us. No need to check SM_SWAPBUTTON here.
        let w = w_param as u32;
        check_wparam_mouse_button(
            timestamp,
            (w & MK_LBUTTON) != 0,
            mouse_flags,
            false,
            data,
            BUTTON_LEFT,
            mouse_id,
        );
        check_wparam_mouse_button(
            timestamp,
            (w & MK_MBUTTON) != 0,
            mouse_flags,
            false,
            data,
            BUTTON_MIDDLE,
            mouse_id,
        );
        check_wparam_mouse_button(
            timestamp,
            (w & MK_RBUTTON) != 0,
            mouse_flags,
            false,
            data,
            BUTTON_RIGHT,
            mouse_id,
        );
        check_wparam_mouse_button(
            timestamp,
            (w & MK_XBUTTON1) != 0,
            mouse_flags,
            false,
            data,
            BUTTON_X1,
            mouse_id,
        );
        check_wparam_mouse_button(
            timestamp,
            (w & MK_XBUTTON2) != 0,
            mouse_flags,
            false,
            data,
            BUTTON_X2,
            mouse_id,
        );

        data.state.with(|s| s.mouse_button_flags = w_param);
    }
}

/// `GetAsyncKeyState()`.
fn async_key_state(vk: u16) -> i16 {
    // SAFETY: GetAsyncKeyState takes a key code.
    unsafe { GetAsyncKeyState(vk as i32) }
}

/// `GetSystemMetrics(SM_SWAPBUTTON) != 0`.
fn swap_buttons() -> bool {
    // SAFETY: GetSystemMetrics takes an index.
    unsafe { GetSystemMetrics(SM_SWAPBUTTON) != 0 }
}

/// Translation of `WIN_CheckAsyncMouseRelease()`.
fn check_async_mouse_release(timestamp: Duration, data: &WindowData) {
    let mouse_id = GLOBAL_MOUSE_ID;

    /* mouse buttons may have changed state here, we need to resync them,
       but we will get a WM_MOUSEMOVE right away which will fix things up if in non raw mode also
    */
    let mouse_flags = mouse::mouse_state().2;
    let swap = swap_buttons();

    let released = |vk: u16| (async_key_state(vk) as u16 & 0x8000) == 0;
    if released(VK_LBUTTON) {
        check_wparam_mouse_button(
            timestamp,
            false,
            mouse_flags,
            swap,
            data,
            BUTTON_LEFT,
            mouse_id,
        );
    }
    if released(VK_RBUTTON) {
        check_wparam_mouse_button(
            timestamp,
            false,
            mouse_flags,
            swap,
            data,
            BUTTON_RIGHT,
            mouse_id,
        );
    }
    if released(VK_MBUTTON) {
        check_wparam_mouse_button(
            timestamp,
            false,
            mouse_flags,
            swap,
            data,
            BUTTON_MIDDLE,
            mouse_id,
        );
    }
    if released(VK_XBUTTON1) {
        check_wparam_mouse_button(
            timestamp,
            false,
            mouse_flags,
            swap,
            data,
            BUTTON_X1,
            mouse_id,
        );
    }
    if released(VK_XBUTTON2) {
        check_wparam_mouse_button(
            timestamp,
            false,
            mouse_flags,
            swap,
            data,
            BUTTON_X2,
            mouse_id,
        );
    }
    data.state.with(|s| s.mouse_button_flags = usize::MAX);
}

/// Follow the foreground window: take or give up the keyboard focus.
/// Translation of `WIN_UpdateFocus()`.
fn update_focus(videodata: &VideoData, data: &WindowData, expect_focus: bool, pos: POINT) {
    let window = data.window;
    let hwnd = data.hwnd;
    let had_focus = sdl_keyboard::keyboard_focus() == Some(window);
    // SAFETY: GetForegroundWindow has no preconditions.
    let has_focus =
        unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow() } == hwnd;

    if had_focus == has_focus || has_focus != expect_focus {
        return;
    }

    if has_focus {
        if should_ignore_focus_click(data)
            && !window_flags(window).contains(WindowFlags::MOUSE_CAPTURE)
        {
            let swap = swap_buttons();
            let mut pending = 0u8;
            if async_key_state(VK_LBUTTON) != 0 {
                pending |= if !swap { 0x01 } else { 0x04 }; // SDL_BUTTON_LMASK : SDL_BUTTON_RMASK
            }
            if async_key_state(VK_RBUTTON) != 0 {
                pending |= if !swap { 0x04 } else { 0x01 }; // SDL_BUTTON_RMASK : SDL_BUTTON_LMASK
            }
            if async_key_state(VK_MBUTTON) != 0 {
                pending |= 0x02; // SDL_BUTTON_MMASK
            }
            if async_key_state(VK_XBUTTON1) != 0 {
                pending |= 0x08; // SDL_BUTTON_X1MASK
            }
            if async_key_state(VK_XBUTTON2) != 0 {
                pending |= 0x10; // SDL_BUTTON_X2MASK
            }
            data.state.with(|s| s.focus_click_pending |= pending);
        }

        let keyboard_focus = core::with_window(window, |w| w.keyboard_focus)
            .ok()
            .flatten();
        let _ = sdl_keyboard::set_keyboard_focus(Some(keyboard_focus.unwrap_or(window)));

        // In relative mode we are guaranteed to have mouse focus if we have keyboard focus
        if !mouse::relative_mode_enabled() {
            let mut cursor_pos = pos;
            // SAFETY: cursor_pos is a valid POINT.
            unsafe { ScreenToClient(hwnd, &mut cursor_pos) };
            mouse::send_mouse_motion(
                get_event_timestamp(),
                Some(window),
                GLOBAL_MOUSE_ID,
                false,
                cursor_pos.x as f32,
                cursor_pos.y as f32,
            );
        }

        check_async_mouse_release(get_event_timestamp(), data);
        update_clip_cursor(window);

        /*
         * FIXME: Update keyboard state
         */
        clipboard::check_clipboard_update(videodata);

        // SAFETY: GetKeyState takes a key code.
        unsafe {
            sdl_keyboard::toggle_mod_state(
                Keymod::CAPS,
                (GetKeyState(VK_CAPITAL as i32) & 0x0001) != 0,
            );
            sdl_keyboard::toggle_mod_state(
                Keymod::NUM,
                (GetKeyState(VK_NUMLOCK as i32) & 0x0001) != 0,
            );
            sdl_keyboard::toggle_mod_state(
                Keymod::SCROLL,
                (GetKeyState(VK_SCROLL as i32) & 0x0001) != 0,
            );
        }

        update_window_icc_profile(window, true);
    } else {
        data.state.with(|s| s.in_window_deactivation = true);

        let _ = sdl_keyboard::set_keyboard_focus(None);
        // In relative mode we are guaranteed to not have mouse focus if we don't have keyboard focus
        if mouse::relative_mode_enabled() {
            mouse::set_mouse_focus(None);
        }
        keyboard::reset_dead_keys();

        unclip_cursor_for_window(window);

        data.state.with(|s| s.in_window_deactivation = false);
    }
}

/// Translation of `ShouldGenerateWindowCloseOnAltF4()`.
fn should_generate_window_close_on_alt_f4() -> bool {
    hints::get_bool(hints::WINDOWS_CLOSE_ON_ALT_F4, true)
}

/// Translation of `ShouldClearWindowOnEraseBackground()`.
fn should_clear_window_on_erase_background(videodata: &VideoData, data: &WindowData) -> bool {
    use super::window::EraseBackgroundMode;
    match data.hint_erase_background_mode {
        EraseBackgroundMode::Never => false,
        EraseBackgroundMode::Initial => !videodata.state.with(|s| s.cleared),
        EraseBackgroundMode::Always => true,
    }
}

// We want to generate mouse events from mouse and pen, and touch events from touchscreens
const MI_WP_SIGNATURE: u32 = 0xFF515700;
const MI_WP_SIGNATURE_MASK: u32 = 0xFFFFFF00;

/// Translation of `SDL_MOUSE_EVENT_SOURCE`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum MouseEventSource {
    #[allow(dead_code)] // (never produced, like upstream)
    Unknown,
    Mouse,
    Touch,
    Pen,
}

/// Translation of `GetMouseMessageSource()`.
fn get_mouse_message_source(extrainfo: u32) -> MouseEventSource {
    // Mouse data (ignoring synthetic mouse events generated for touchscreens)
    /* Versions below Vista will set the low 7 bits to the Mouse ID and don't use bit 7:
    Check bits 8-31 for the signature (which will indicate a Tablet PC Pen or Touch Device).
    Only check bit 7 when Vista and up(Cleared=Pen, Set=Touch(which we need to filter out)),
    when the signature is set. The Mouse ID will be zero for an actual mouse. */
    if (extrainfo & MI_WP_SIGNATURE_MASK) == MI_WP_SIGNATURE {
        if (extrainfo & 0x80) != 0 {
            return MouseEventSource::Touch;
        } else {
            return MouseEventSource::Pen;
        }
    }
    MouseEventSource::Mouse
}

/// The data of the SDL window with this HWND. Translation of
/// `WIN_GetWindowDataFromHWND()`.
fn get_window_data_from_hwnd(hwnd: HWND) -> Option<Arc<WindowData>> {
    with_device(|v| {
        v.windows.iter().find_map(|w| {
            w.internal
                .as_ref()
                .and_then(|i| i.downcast_ref::<Arc<WindowData>>())
                .filter(|d| d.hwnd == hwnd)
                .cloned()
        })
    })
    .ok()
    .flatten()
}

/// The low-level keyboard hook of keyboard grabs. Translation of
/// `WIN_KeyboardHookProc()`.
pub(crate) unsafe extern "system" fn keyboard_hook_proc(
    n_code: i32,
    w_param: WPARAM,
    l_param: LPARAM,
) -> LRESULT {
    let Some(data) = video_data() else {
        // SAFETY: passing the hook call on.
        return unsafe { CallNextHookEx(std::ptr::null_mut(), n_code, w_param, l_param) };
    };

    if n_code < 0 || n_code != HC_ACTION as i32 {
        // SAFETY: passing the hook call on.
        return unsafe { CallNextHookEx(std::ptr::null_mut(), n_code, w_param, l_param) };
    }
    // SAFETY: for HC_ACTION, lParam points at a KBDLLHOOKSTRUCT.
    let hook_data = unsafe { &*(l_param as *const KBDLLHOOKSTRUCT) };
    if hook_data.scanCode == 0x21d {
        // Skip fake LCtrl when RAlt is pressed
        return 1;
    }

    let scan_code = match hook_data.vkCode as u16 {
        VK_LWIN => Scancode::LGUI,
        VK_RWIN => Scancode::RGUI,
        VK_LMENU => Scancode::LALT,
        VK_RMENU => Scancode::RALT,
        VK_LCONTROL => Scancode::LCTRL,
        VK_RCONTROL => Scancode::RCTRL,
        VK_SNAPSHOT => Scancode::PRINTSCREEN,

        // These are required to intercept Alt+Tab and Alt+Esc on Windows 7
        VK_TAB => Scancode::TAB,
        VK_ESCAPE => Scancode::ESCAPE,

        _ => {
            // SAFETY: passing the hook call on.
            return unsafe { CallNextHookEx(std::ptr::null_mut(), n_code, w_param, l_param) };
        }
    };

    let raw_keyboard_enabled = data.state.with(|s| s.raw_keyboard_enabled);
    if w_param == WM_KEYDOWN as usize || w_param == WM_SYSKEYDOWN as usize {
        if !raw_keyboard_enabled {
            sdl_keyboard::send_keyboard_key(
                Duration::ZERO,
                GLOBAL_KEYBOARD_ID,
                hook_data.scanCode as i32,
                scan_code,
                true,
            );
        }
    } else {
        if !raw_keyboard_enabled {
            sdl_keyboard::send_keyboard_key(
                Duration::ZERO,
                GLOBAL_KEYBOARD_ID,
                hook_data.scanCode as i32,
                scan_code,
                false,
            );
        }

        /* If the key was down prior to our hook being installed, allow the
        key up message to pass normally the first time. This ensures other
        windows have a consistent view of the key state, and avoids keys
        being stuck down in those windows if they are down when the grab
        happens and raised while grabbed. */
        let vk = hook_data.vkCode as usize;
        let was_down = vk <= 0xFF
            && data.state.with(|s| {
                let down = s.pre_hook_key_state[vk] != 0;
                if down {
                    s.pre_hook_key_state[vk] = 0;
                }
                down
            });
        if was_down {
            // SAFETY: passing the hook call on.
            return unsafe { CallNextHookEx(std::ptr::null_mut(), n_code, w_param, l_param) };
        }
    }

    1
}

/// Translation of `WIN_SwapButtons()`.
fn win_swap_buttons(h_device: HANDLE) -> bool {
    if h_device.is_null() {
        // Touchpad, already has buttons swapped
        return false;
    }
    swap_buttons()
}

/// The pens of the pointer messages (upstream's handle `(void *)1`) and of
/// absolute raw mouse input (`(void *)-1`).
static POINTER_PEN_HANDLE: LazyLock<Arc<dyn Any + Send + Sync>> =
    LazyLock::new(|| Arc::new(1usize));
static RAW_INPUT_PEN_HANDLE: LazyLock<Arc<dyn Any + Send + Sync>> =
    LazyLock::new(|| Arc::new(usize::MAX));

/// The raw input state of the driver (the `rawinput*` and raw mouse fields
/// of `struct SDL_VideoData`), used by the raw input thread.
pub(crate) struct RawInputData {
    /// The raw input buffer (`rawinput`, `rawinput_size`), 8-byte aligned.
    rawinput: Vec<u64>,
    rawinput_offset: u32,
    #[allow(dead_code)] // (counted, like upstream)
    rawinput_count: u32,
    pub(crate) last_rawinput_poll: u64,
    last_raw_mouse_position: Point,
    pending_e1_key_sequence: bool,
    pub(crate) raw_input_fake_pen_id: PenID,
}

impl RawInputData {
    pub(crate) fn new() -> RawInputData {
        RawInputData {
            rawinput: Vec::new(),
            rawinput_offset: 0,
            rawinput_count: 0,
            last_rawinput_poll: 0,
            last_raw_mouse_position: Point::default(),
            pending_e1_key_sequence: false,
            raw_input_fake_pen_id: 0,
        }
    }
}

/// Raw mouse buttons. Translation of `raw_buttons`.
const RAW_BUTTONS: [(u32, u8, bool); 10] = [
    (RI_MOUSE_LEFT_BUTTON_DOWN, BUTTON_LEFT, true),
    (RI_MOUSE_LEFT_BUTTON_UP, BUTTON_LEFT, false),
    (RI_MOUSE_RIGHT_BUTTON_DOWN, BUTTON_RIGHT, true),
    (RI_MOUSE_RIGHT_BUTTON_UP, BUTTON_RIGHT, false),
    (RI_MOUSE_MIDDLE_BUTTON_DOWN, BUTTON_MIDDLE, true),
    (RI_MOUSE_MIDDLE_BUTTON_UP, BUTTON_MIDDLE, false),
    (RI_MOUSE_BUTTON_4_DOWN, BUTTON_X1, true),
    (RI_MOUSE_BUTTON_4_UP, BUTTON_X1, false),
    (RI_MOUSE_BUTTON_5_DOWN, BUTTON_X2, true),
    (RI_MOUSE_BUTTON_5_UP, BUTTON_X2, false),
];

/// Translation of the static `wobble` of `WIN_HandleRawMouseInput()`.
static WOBBLE: AtomicI32 = AtomicI32::new(0);

/// Translation of `WIN_HandleRawMouseInput()`.
fn handle_raw_mouse_input(
    timestamp: Duration,
    data: &VideoData,
    h_device: HANDLE,
    rawmouse: &RAWMOUSE,
) {
    const MOUSE_MOVE_ABSOLUTE: u16 = 0x01;
    const MOUSE_VIRTUAL_DESKTOP: u16 = 0x02;

    let dx = rawmouse.lLastX;
    let dy = rawmouse.lLastY;
    let have_motion = dx != 0 || dy != 0;
    // SAFETY: the buttons member of the union is the one raw input fills.
    let (us_button_flags, us_button_data) = unsafe {
        (
            rawmouse.Anonymous.Anonymous.usButtonFlags,
            rawmouse.Anonymous.Anonymous.usButtonData,
        )
    };
    let have_button = us_button_flags != 0;
    let is_absolute = (rawmouse.usFlags & MOUSE_MOVE_ABSOLUTE) != 0;
    let mouse_id = h_device as usize as mouse::MouseID;

    // Check whether relative mode should also receive events from the rawinput stream
    if !data.state.with(|s| s.raw_mouse_enabled) {
        return;
    }

    // Relative mouse motion is delivered to the window with keyboard focus
    let Some(window) = sdl_keyboard::keyboard_focus() else {
        return;
    };

    if get_mouse_message_source(rawmouse.ulExtraInformation) != MouseEventSource::Mouse
        || (touch::touch_devices_available() && (rawmouse.ulExtraInformation & 0x80) == 0x80)
    {
        return;
    }

    let Some(windowdata) = window_data(window) else {
        return;
    };

    if have_motion && windowdata.state.with(|s| s.in_modal_loop) == 0 {
        if !is_absolute {
            mouse::send_mouse_motion(
                timestamp,
                Some(window),
                mouse_id,
                true,
                dx as f32,
                dy as f32,
            );
        } else {
            /* This is absolute motion, either using a tablet or mouse over RDP

                Notes on how RDP appears to work, as of Windows 10 2004:
                - SetCursorPos() calls are cached, with multiple calls coalesced into a single call that's sent to the RDP client. If the last call to SetCursorPos() has the same value as the last one that was sent to the client, it appears to be ignored and not sent. This means that we need to jitter the SetCursorPos() position slightly in order for the recentering to work correctly.
                - User mouse motion is coalesced with SetCursorPos(), so the WM_INPUT positions we see will not necessarily match the position we requested with SetCursorPos().
                - SetCursorPos() outside of the bounds of the focus window appears not to do anything.
                - SetCursorPos() while the cursor is NULL doesn't do anything

                We handle this by creating a safe area within the application window, and when the mouse leaves that safe area, we warp back to the opposite side. Any single motion > 50% of the safe area is assumed to be a warp and ignored.
            */
            // SAFETY: GetSystemMetrics takes an index.
            let metrics = |i| unsafe { GetSystemMetrics(i) };
            let remote_desktop = metrics(SM_REMOTESESSION) == 1;
            let virtual_desktop = (rawmouse.usFlags & MOUSE_VIRTUAL_DESKTOP) != 0;
            let raw_coordinates = (rawmouse.usFlags & 0x40) != 0;
            let w = metrics(if virtual_desktop {
                SM_CXVIRTUALSCREEN
            } else {
                SM_CXSCREEN
            });
            let h = metrics(if virtual_desktop {
                SM_CYVIRTUALSCREEN
            } else {
                SM_CYSCREEN
            });
            let x = if raw_coordinates {
                dx
            } else {
                ((dx as f32 / 65535.0) * w as f32) as i32
            };
            let y = if raw_coordinates {
                dy
            } else {
                ((dy as f32 / 65535.0) * h as f32) as i32
            };

            // Calculate relative motion
            let (rel_x, rel_y) = data.raw.with(|r| {
                if r.last_raw_mouse_position.x == 0 && r.last_raw_mouse_position.y == 0 {
                    r.last_raw_mouse_position.x = x;
                    r.last_raw_mouse_position.y = y;
                }
                (
                    x - r.last_raw_mouse_position.x,
                    y - r.last_raw_mouse_position.y,
                )
            });

            if remote_desktop {
                let (in_title_click, focus_click_pending, rect) = windowdata.state.with(|s| {
                    (
                        s.in_title_click,
                        s.focus_click_pending,
                        s.cursor_clipped_rect,
                    )
                });
                if !in_title_click && focus_click_pending == 0 {
                    let float_x = x as f32 / w as f32;
                    let float_y = y as f32 / h as f32;

                    // See if the mouse is at the edge of the screen, or in the RDP title bar area
                    if float_x <= 0.01
                        || float_x >= 0.99
                        || float_y <= 0.01
                        || float_y >= 0.99
                        || y < 32
                    {
                        // Wobble the cursor position so it's not ignored if the last warp didn't have any effect
                        let wobble = WOBBLE.load(Ordering::Relaxed);
                        let warp_x = rect.left + ((rect.right - rect.left) / 2) + wobble;
                        let warp_y = rect.top + ((rect.bottom - rect.top) / 2);

                        winmouse::set_cursor_pos(warp_x, warp_y);

                        let mut wobble = wobble + 1;
                        if wobble > 1 {
                            wobble = -1;
                        }
                        WOBBLE.store(wobble, Ordering::Relaxed);
                    } else {
                        /* Send relative motion if we didn't warp last frame (had good position data)
                          We also sometimes get large deltas due to coalesced mouse motion and warping,
                          so ignore those.
                        */
                        let max_relative_motion = h / 6;
                        if rel_x.abs() < max_relative_motion && rel_y.abs() < max_relative_motion {
                            mouse::send_mouse_motion(
                                timestamp,
                                Some(window),
                                mouse_id,
                                true,
                                rel_x as f32,
                                rel_y as f32,
                            );
                        }
                    }
                }
            } else if mouse::with_mouse(|m| m.pen_mouse_events) {
                const MAXIMUM_TABLET_RELATIVE_MOTION: i32 = 32;
                if rel_x.abs() > MAXIMUM_TABLET_RELATIVE_MOTION
                    || rel_y.abs() > MAXIMUM_TABLET_RELATIVE_MOTION
                {
                    // Ignore this motion, probably a pen lift and drop
                } else {
                    mouse::send_mouse_motion(
                        timestamp,
                        Some(window),
                        mouse_id,
                        true,
                        rel_x as f32,
                        rel_y as f32,
                    );
                }
            } else {
                let screen_x = if virtual_desktop {
                    metrics(SM_XVIRTUALSCREEN)
                } else {
                    0
                };
                let screen_y = if virtual_desktop {
                    metrics(SM_YVIRTUALSCREEN)
                } else {
                    0
                };

                let mut pen_id = data.raw.with(|r| r.raw_input_fake_pen_id);
                if pen_id == 0 {
                    pen_id = pen::add_pen_device(
                        timestamp,
                        Some("raw mouse input"),
                        Some(window),
                        None,
                        RAW_INPUT_PEN_HANDLE.clone(),
                        true,
                    );
                    data.raw.with(|r| r.raw_input_fake_pen_id = pen_id);
                }
                let (wx, wy) =
                    core::with_window(window, |w| (w.core.x, w.core.y)).unwrap_or((0, 0));
                pen::send_pen_motion(
                    timestamp,
                    pen_id,
                    Some(window),
                    (x + screen_x - wx) as f32,
                    (y + screen_y - wy) as f32,
                );
            }

            data.raw.with(|r| {
                r.last_raw_mouse_position.x = x;
                r.last_raw_mouse_position.y = y;
            });
        }
    }

    if have_button {
        for &(flags, raw_button, down) in &RAW_BUTTONS {
            if (us_button_flags as u32 & flags) != 0 {
                let mut button = raw_button;

                if button == BUTTON_LEFT {
                    if win_swap_buttons(h_device) {
                        button = BUTTON_RIGHT;
                    }
                } else if button == BUTTON_RIGHT && win_swap_buttons(h_device) {
                    button = BUTTON_LEFT;
                }

                let mask = button_mask(button);
                if (windowdata.state.with(|s| s.focus_click_pending) & mask) != 0 {
                    // Ignore the button click for activation
                    if !down {
                        windowdata.state.with(|s| s.focus_click_pending &= !mask);
                        update_clip_cursor(window);
                    }
                    continue;
                }

                mouse::send_mouse_button(timestamp, Some(window), mouse_id, button, down);
            }
        }

        if (us_button_flags as u32 & RI_MOUSE_WHEEL) != 0 {
            let amount = us_button_data as i16;
            let f_amount = amount as f32 / WHEEL_DELTA as f32;
            mouse::send_mouse_wheel(
                timestamp,
                Some(window),
                mouse_id,
                0.0,
                f_amount,
                MouseWheelDirection::Normal,
            );
        } else if (us_button_flags as u32 & RI_MOUSE_HWHEEL) != 0 {
            let amount = us_button_data as i16;
            let f_amount = amount as f32 / WHEEL_DELTA as f32;
            mouse::send_mouse_wheel(
                timestamp,
                Some(window),
                mouse_id,
                f_amount,
                0.0,
                MouseWheelDirection::Normal,
            );
        }

        /* Invalidate the mouse button flags. If we don't do this then disabling raw input
        will cause held down mouse buttons to persist when released. */
        windowdata.state.with(|s| s.mouse_button_flags = usize::MAX);
    }
}

/// Translation of `WIN_HandleRawKeyboardInput()`.
fn handle_raw_keyboard_input(
    timestamp: Duration,
    data: &VideoData,
    h_device: HANDLE,
    rawkeyboard: &mut RAWKEYBOARD,
) {
    let keyboard_id = h_device as usize as crate::events::KeyboardID;

    let (raw_keyboard_enabled, inputsink) = data
        .state
        .with(|s| (s.raw_keyboard_enabled, s.raw_keyboard_flag_inputsink));
    if !raw_keyboard_enabled {
        return;
    }

    if (rawkeyboard.Flags as u32 & RI_KEY_E1) != 0 {
        // First key in a Ctrl+{key} sequence
        data.raw.with(|r| r.pending_e1_key_sequence = true);
        return;
    }

    if (rawkeyboard.Flags as u32 & RI_KEY_E0) != 0 && rawkeyboard.MakeCode == 0x2A {
        // 0xE02A make code prefix, ignored
        return;
    }

    if rawkeyboard.MakeCode == 0 {
        // SAFETY: MapVirtualKeyW is a pure lookup.
        rawkeyboard.MakeCode = unsafe {
            MapVirtualKeyW(
                rawkeyboard.VKey as u32,
                if is_windows_xp() {
                    MAPVK_VK_TO_VSC
                } else {
                    MAPVK_VK_TO_VSC_EX
                },
            )
        } as u16;
    }
    if rawkeyboard.MakeCode == 0 {
        return;
    }

    let down = (rawkeyboard.Flags as u32 & RI_KEY_BREAK) == 0;
    let mut rawcode = rawkeyboard.MakeCode;
    let pending_e1 = data
        .raw
        .with(|r| std::mem::take(&mut r.pending_e1_key_sequence));
    let code = if pending_e1 {
        rawcode |= 0xE100;
        if rawkeyboard.MakeCode == 0x45 {
            // Ctrl+NumLock == Pause
            Scancode::PAUSE
        } else {
            // Ctrl+ScrollLock == Break (no SDL scancode?)
            Scancode::UNKNOWN
        }
    } else {
        // The code is in the lower 7 bits, the high bit is set for the E0 prefix
        let mut index = rawkeyboard.MakeCode as u8;
        if (rawkeyboard.Flags as u32 & RI_KEY_E0) != 0 {
            rawcode |= 0xE000;
            index |= 0x80;
        }
        WINDOWS_SCANCODE_TABLE[index as usize]
    };

    if down {
        let focus = sdl_keyboard::keyboard_focus();
        // With input sink flag we want to receive input even if not focused
        let text_input_active = focus
            .is_some_and(|f| core::with_window(f, |w| w.core.text_input_active).unwrap_or(false));
        if (!inputsink && focus.is_none()) || text_input_active {
            return;
        }
    }

    sdl_keyboard::send_keyboard_key(timestamp, keyboard_id, rawcode as i32, code, down);
}

/// The alignment of raw input blocks (`RAWINPUT_ALIGN()`).
const RAWINPUT_ALIGNMENT: usize = size_of::<usize>();

/// `NEXTRAWINPUTBLOCK()` on a byte offset.
fn next_raw_input_block(buffer: &[u8], offset: usize) -> usize {
    let dw_size = u32::from_ne_bytes(
        buffer[offset + 4..offset + 8]
            .try_into()
            .expect("RAWINPUTHEADER.dwSize"),
    ) as usize;
    (offset + dw_size + RAWINPUT_ALIGNMENT - 1) & !(RAWINPUT_ALIGNMENT - 1)
}

/// Read every pending raw input block and handle it, spreading the mouse
/// timestamps over the poll interval. Translation of `WIN_PollRawInput()`.
pub(crate) fn poll_raw_input(data: &VideoData, poll_start: u64) {
    let mut total: u32 = 0;
    let mut poll_finish: u64;

    if data.raw.with(|r| r.rawinput_offset) == 0 {
        let mut offset = size_of::<RAWINPUTHEADER>() as u32;
        let mut is_wow64 = 0;
        // SAFETY: is_wow64 is a valid out pointer.
        if unsafe { IsWow64Process(GetCurrentProcess(), &mut is_wow64) } != 0 && is_wow64 != 0 {
            // We're going to get 64-bit data, so use the 64-bit RAWINPUTHEADER size
            offset += 8;
        }
        data.raw.with(|r| r.rawinput_offset = offset);
    }

    // Get all available events
    // (the buffer is taken out of the shared state while the handlers run)
    let (mut buffer, rawinput_offset) = data
        .raw
        .with(|r| (std::mem::take(&mut r.rawinput), r.rawinput_offset));
    let mut input: usize = 0; // byte offset of the next block
    loop {
        let rawinput_size = buffer.len() * 8;
        let mut size = (rawinput_size - input) as u32;
        // SAFETY: the buffer has `size` bytes after `input` (or is empty,
        // in which case Windows reports the size it needs).
        let count = unsafe {
            let ptr = if buffer.is_empty() {
                std::ptr::null_mut()
            } else {
                buffer.as_mut_ptr().cast::<u8>().add(input)
            };
            GetRawInputBuffer(ptr.cast(), &mut size, size_of::<RAWINPUTHEADER>() as u32)
        };
        poll_finish = crate::timer::ticks_ns();
        if count == 0 || count == u32::MAX {
            // SAFETY: GetLastError has no preconditions.
            if buffer.is_empty()
                || (count == u32::MAX && unsafe { GetLastError() } == ERROR_INSUFFICIENT_BUFFER)
            {
                const RAWINPUT_BUFFER_SIZE_INCREMENT: usize = 96; // 2 64-bit raw mouse packets
                buffer.resize(buffer.len() + RAWINPUT_BUFFER_SIZE_INCREMENT / 8, 0);
            } else {
                break;
            }
        } else {
            total += count;

            // Advance input to the end of the buffer
            // SAFETY: u64s viewed as bytes.
            let bytes = unsafe {
                std::slice::from_raw_parts(buffer.as_ptr().cast::<u8>(), buffer.len() * 8)
            };
            for _ in 0..count {
                input = next_raw_input_block(bytes, input);
            }
        }
    }

    if total > 0 {
        let delta = poll_finish.wrapping_sub(poll_start);
        // SAFETY: u64s viewed as bytes.
        let bytes =
            unsafe { std::slice::from_raw_parts(buffer.as_ptr().cast::<u8>(), buffer.len() * 8) };
        let header_at = |offset: usize| -> RAWINPUTHEADER {
            // SAFETY: Windows wrote a RAWINPUTHEADER at each block offset.
            unsafe {
                bytes
                    .as_ptr()
                    .add(offset)
                    .cast::<RAWINPUTHEADER>()
                    .read_unaligned()
            }
        };
        let mut mouse_total: u64 = 0;
        let mut input = 0usize;
        for _ in 0..total {
            if header_at(input).dwType == RIM_TYPEMOUSE {
                mouse_total += 1;
            }
            input = next_raw_input_block(bytes, input);
        }
        let mut mouse_index: u64 = 0;
        let mut input = 0usize;
        for _ in 0..total {
            let header = header_at(input);
            let payload = input + rawinput_offset as usize;
            if header.dwType == RIM_TYPEMOUSE {
                mouse_index += 1; // increment first so that it starts at one
                                  // SAFETY: a RAWMOUSE follows the (possibly 64-bit) header.
                let rawmouse: RAWMOUSE = unsafe {
                    bytes
                        .as_ptr()
                        .add(payload)
                        .cast::<RAWMOUSE>()
                        .read_unaligned()
                };
                let time = poll_finish - (delta * (mouse_total - mouse_index)) / mouse_total;
                handle_raw_mouse_input(Duration::from_nanos(time), data, header.hDevice, &rawmouse);
            } else if header.dwType == RIM_TYPEKEYBOARD {
                // SAFETY: a RAWKEYBOARD follows the (possibly 64-bit) header.
                let mut rawkeyboard: RAWKEYBOARD = unsafe {
                    bytes
                        .as_ptr()
                        .add(payload)
                        .cast::<RAWKEYBOARD>()
                        .read_unaligned()
                };
                handle_raw_keyboard_input(
                    Duration::from_nanos(poll_finish),
                    data,
                    header.hDevice,
                    &mut rawkeyboard,
                );
            }
            input = next_raw_input_block(bytes, input);
        }
    }
    data.raw.with(|r| {
        r.rawinput = buffer;
        r.last_rawinput_poll = poll_finish;
    });
}

/// `SDL_sscanf(instance, "HID\\VID_%X&PID_%X&", &vendor, &product)`.
fn scan_vid_pid(instance: &str) -> (i32, i32) {
    let hex = |s: &str| -> Option<(i32, usize)> {
        let n = s.bytes().take_while(|b| b.is_ascii_hexdigit()).count();
        if n == 0 {
            return None;
        }
        Some((
            u32::from_str_radix(&s[..n], 16).unwrap_or(u32::MAX) as i32,
            n,
        ))
    };
    let mut vendor = 0;
    let mut product = 0;
    if let Some(rest) = instance.strip_prefix("HID\\VID_") {
        if let Some((v, n)) = hex(rest) {
            vendor = v;
            if let Some(rest) = rest[n..].strip_prefix("&PID_") {
                if let Some((p, _)) = hex(rest) {
                    product = p;
                }
            }
        }
    }
    (vendor, product)
}

/// The name of a raw input device: from HID, or the device's description
/// in SetupAPI, or `default_name`. Translation of `GetDeviceName()`.
fn get_device_name(
    h_device: HANDLE,
    devinfo: HDEVINFO,
    instance: &str,
    vendor: u16,
    product: u16,
    default_name: &str,
    hid_loaded: bool,
) -> Option<String> {
    let mut vendor_name = None;
    let mut product_name = None;

    // These are 126 for USB, but can be longer for Bluetooth devices
    let mut vend = [0u16; 256];
    let mut prod = [0u16; 256];

    if hid_loaded {
        const MAX_PATH: usize = 260;
        let mut dev_name = [0u8; MAX_PATH + 1];
        let mut cap = (dev_name.len() - 1) as u32;
        // SAFETY: dev_name holds cap bytes plus a terminator.
        let len = unsafe {
            GetRawInputDeviceInfoA(
                h_device,
                RIDI_DEVICENAME,
                dev_name.as_mut_ptr().cast(),
                &mut cap,
            )
        };
        if len != u32::MAX {
            dev_name[(len as usize).min(MAX_PATH)] = 0;

            // important: for devices with exclusive access mode as per
            // https://learn.microsoft.com/en-us/windows-hardware/drivers/hid/top-level-collections-opened-by-windows-for-system-use
            // they can only be opened with a desired access of none instead of generic read.
            // SAFETY: dev_name is NUL-terminated; the handle is closed after use.
            unsafe {
                let h_file = CreateFileA(
                    dev_name.as_ptr(),
                    0,
                    FILE_SHARE_READ | FILE_SHARE_WRITE,
                    std::ptr::null(),
                    OPEN_EXISTING,
                    0,
                    std::ptr::null_mut(),
                );
                if h_file != INVALID_HANDLE_VALUE {
                    crate::core::windows::hid::hidd_get_manufacturer_string(h_file, &mut vend);
                    crate::core::windows::hid::hidd_get_product_string(h_file, &mut prod);
                    CloseHandle(h_file);
                }
            }
        }
    }

    if vend[0] != 0 {
        vendor_name = Some(wide_to_utf8(&vend));
    }

    if prod[0] != 0 {
        product_name = Some(wide_to_utf8(&prod));
    } else {
        // SAFETY: plain data; all zeroes is valid.
        let mut data: SP_DEVINFO_DATA = unsafe { std::mem::zeroed() };
        data.cbSize = size_of::<SP_DEVINFO_DATA>() as u32;
        let mut i: u32 = 0;
        loop {
            // SAFETY: devinfo is the set from SetupDiGetClassDevsA; data is sized.
            if unsafe { SetupDiEnumDeviceInfo(devinfo, i, &mut data) } == 0 {
                // SAFETY: GetLastError has no preconditions.
                if unsafe { GetLastError() } == ERROR_NO_MORE_ITEMS {
                    break;
                } else {
                    i += 1;
                    continue;
                }
            }
            i += 1;

            let mut device_instance_id = [0u8; 64];
            // SAFETY: the buffer holds 64 bytes.
            if unsafe {
                SetupDiGetDeviceInstanceIdA(
                    devinfo,
                    &data,
                    device_instance_id.as_mut_ptr(),
                    device_instance_id.len() as u32,
                    std::ptr::null_mut(),
                )
            } == 0
            {
                continue;
            }
            let end = device_instance_id
                .iter()
                .position(|&b| b == 0)
                .unwrap_or(64);
            let device_instance_id = String::from_utf8_lossy(&device_instance_id[..end]);

            if instance.eq_ignore_ascii_case(&device_instance_id) {
                let mut size: u32 = 0;
                // SAFETY: prod holds its size in bytes.
                if unsafe {
                    SetupDiGetDeviceRegistryPropertyW(
                        devinfo,
                        &data,
                        SPDRP_DEVICEDESC,
                        std::ptr::null_mut(),
                        prod.as_mut_ptr().cast(),
                        size_of_val(&prod) as u32,
                        &mut size,
                    )
                } != 0
                {
                    // Make sure the device description is null terminated
                    let mut size = size as usize / size_of::<u16>();
                    if size >= prod.len() {
                        // Truncated description...
                        size = prod.len() - 1;
                    }
                    prod[size] = 0;

                    if vendor != 0 || product != 0 {
                        product_name = Some(format!(
                            "{} (0x{:04x}/0x{:04x})",
                            wide_to_utf8(&prod),
                            vendor,
                            product
                        ));
                    } else {
                        product_name = Some(wide_to_utf8(&prod));
                    }
                }
                break;
            }
        }
    }

    if product_name.is_none() && (vendor != 0 || product != 0) {
        product_name = Some(format!("{default_name} (0x{vendor:04x}/0x{product:04x})"));
    }
    crate::utils::create_device_name(
        vendor,
        product,
        vendor_name.as_deref(),
        product_name.as_deref(),
        Some(default_name),
    )
}

/// Add the keyboards and mice that appeared and remove the ones that went
/// away (on the device hotplug thread). Translation of
/// `WIN_CheckKeyboardAndMouseHotplug()`.
pub(crate) fn check_keyboard_and_mouse_hotplug(hid_loaded: bool) {
    let Some(videodata) = video_data() else {
        return;
    };
    if core::current_video_driver().ok() != Some("windows")
        || !videodata.state.with(|s| s.detect_device_hotplug)
        || super::gameinput::has_game_input(&videodata)
    {
        return;
    }

    let mut raw_device_count: u32 = 0;
    // SAFETY: a NULL list asks for the count only.
    if unsafe {
        GetRawInputDeviceList(
            std::ptr::null_mut(),
            &mut raw_device_count,
            size_of::<RAWINPUTDEVICELIST>() as u32,
        )
    } == u32::MAX
        || raw_device_count == 0
    {
        return; // oh well.
    }

    // SAFETY: plain data; all zeroes is valid.
    let mut raw_devices: Vec<RAWINPUTDEVICELIST> =
        vec![unsafe { std::mem::zeroed() }; raw_device_count as usize];

    // SAFETY: raw_devices holds raw_device_count entries.
    raw_device_count = unsafe {
        GetRawInputDeviceList(
            raw_devices.as_mut_ptr(),
            &mut raw_device_count,
            size_of::<RAWINPUTDEVICELIST>() as u32,
        )
    };
    if raw_device_count == u32::MAX {
        return; // oh well.
    }

    // SAFETY: NULL filters list all present devices.
    let devinfo = unsafe {
        SetupDiGetClassDevsA(
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null_mut(),
            DIGCF_ALLCLASSES | DIGCF_PRESENT,
        )
    };

    let old_keyboards = sdl_keyboard::keyboards();
    let old_mice = mouse::mice();
    let mut new_keyboards = Vec::new();
    let mut new_mice = Vec::new();

    for device in raw_devices.iter().take(raw_device_count as usize) {
        const MAX_PATH: usize = 260;
        // SAFETY: plain data; all zeroes is valid.
        let mut rdi: RID_DEVICE_INFO = unsafe { std::mem::zeroed() };
        let mut dev_name = [0u8; MAX_PATH];
        let mut rdi_size = size_of::<RID_DEVICE_INFO>() as u32;
        let mut name_size = dev_name.len() as u32;
        let dw_type = device.dwType;

        if dw_type != RIM_TYPEKEYBOARD && dw_type != RIM_TYPEMOUSE {
            continue;
        }

        rdi.cbSize = size_of::<RID_DEVICE_INFO>() as u32;
        // SAFETY: the buffers hold the sizes passed.
        if unsafe {
            GetRawInputDeviceInfoA(
                device.hDevice,
                RIDI_DEVICEINFO,
                (&mut rdi as *mut RID_DEVICE_INFO).cast(),
                &mut rdi_size,
            ) == u32::MAX
                || GetRawInputDeviceInfoA(
                    device.hDevice,
                    RIDI_DEVICENAME,
                    dev_name.as_mut_ptr().cast(),
                    &mut name_size,
                ) == u32::MAX
        } {
            continue;
        }

        // Extract the device instance
        let end = dev_name.iter().position(|&b| b == 0).unwrap_or(MAX_PATH);
        let mut name: Vec<u8> = dev_name[..end].to_vec();
        let start = name
            .iter()
            .position(|&c| c != b'\\' && c != b'?')
            .unwrap_or(name.len());
        let mut ptr = start;
        while ptr < name.len() {
            if name[ptr] == b'#' {
                name[ptr] = b'\\';
            }
            if name[ptr] == b'{' {
                if ptr > start && name[ptr - 1] == b'\\' {
                    ptr -= 1;
                }
                break;
            }
            ptr += 1;
        }
        let instance = String::from_utf8_lossy(&name[start..ptr]).into_owned();

        let (vendor, product) = scan_vid_pid(&instance);

        match dw_type {
            RIM_TYPEKEYBOARD => {
                // SAFETY: the keyboard member is the one filled for keyboards.
                let num_keys = unsafe { rdi.Anonymous.keyboard.dwNumberOfKeysTotal };
                if sdl_keyboard::is_keyboard(vendor as u16, product as u16, num_keys as i32) {
                    let keyboard_id = device.hDevice as usize as u32;
                    new_keyboards.push(keyboard_id);
                    if !old_keyboards.contains(&keyboard_id) {
                        let name = get_device_name(
                            device.hDevice,
                            devinfo,
                            &instance,
                            vendor as u16,
                            product as u16,
                            "Keyboard",
                            hid_loaded,
                        );
                        sdl_keyboard::add_keyboard(keyboard_id, name.as_deref());
                    }
                }
            }
            RIM_TYPEMOUSE if mouse::is_mouse(vendor as u16, product as u16) => {
                let mouse_id = device.hDevice as usize as u32;
                new_mice.push(mouse_id);
                if !old_mice.contains(&mouse_id) {
                    let name = get_device_name(
                        device.hDevice,
                        devinfo,
                        &instance,
                        vendor as u16,
                        product as u16,
                        "Mouse",
                        hid_loaded,
                    );
                    mouse::add_mouse(mouse_id, name.as_deref());
                }
            }
            _ => {}
        }
    }

    for &id in old_keyboards.iter().rev() {
        if !new_keyboards.contains(&id) {
            sdl_keyboard::remove_keyboard(id);
        }
    }

    for &id in old_mice.iter().rev() {
        if !new_mice.contains(&id) {
            mouse::remove_mouse(id);
        }
    }

    // SAFETY: devinfo came from SetupDiGetClassDevsA.
    unsafe { SetupDiDestroyDeviceInfoList(devinfo) };
}

/// Return true if spurious LCtrl is pressed: LCtrl is sent when RAltGR is
/// pressed. Translation of `SkipAltGrLeftControl()`.
fn skip_alt_gr_left_control(w_param: WPARAM, l_param: LPARAM) -> bool {
    if w_param != VK_CONTROL as usize {
        return false;
    }

    // Is this an extended key (i.e. right key)?
    if (l_param & 0x01000000) != 0 {
        return false;
    }

    // Here is a trick: "Alt Gr" sends LCTRL, then RALT. We only
    // want the RALT message, so we try to see if the next message
    // is a RALT message. In that case, this is a false LCTRL!
    // SAFETY: next_msg is a valid MSG; PM_NOREMOVE leaves the queue as is.
    unsafe {
        let mut next_msg: MSG = std::mem::zeroed();
        let msg_time = GetMessageTime() as u32;
        if PeekMessageW(&mut next_msg, std::ptr::null_mut(), 0, 0, PM_NOREMOVE) != 0
            && (next_msg.message == WM_KEYDOWN || next_msg.message == WM_SYSKEYDOWN)
            && next_msg.wParam == VK_MENU as usize
            && (next_msg.lParam & 0x01000000) != 0
            && next_msg.time == msg_time
        {
            // Next message is a RALT down message, which means that this is NOT a proper LCTRL message!
            return true;
        }
    }

    false
}

/// Run the message hook on a message from a modal loop. Translation of
/// `DispatchModalLoopMessageHook()`.
fn dispatch_modal_loop_message_hook(
    hook: &WindowsMessageHook,
    hwnd: HWND,
    msg: &mut u32,
    w_param: &mut WPARAM,
    l_param: &mut LPARAM,
) -> bool {
    // SAFETY: MSG is plain data; all zeroes is valid.
    let mut dummy: MSG = unsafe { std::mem::zeroed() };
    dummy.hwnd = hwnd;
    dummy.message = *msg;
    dummy.wParam = *w_param;
    dummy.lParam = *l_param;
    if hook(&mut dummy) {
        // Can't modify the hwnd, but everything else is fair game
        *msg = dummy.message;
        *w_param = dummy.wParam;
        *l_param = dummy.lParam;
        return true;
    }
    false
}

/// `GetMessagePos()` as a point.
fn message_pos() -> POINT {
    // SAFETY: GetMessagePos has no preconditions.
    let pos = unsafe { GetMessagePos() } as isize;
    POINT {
        x: get_x_lparam(pos),
        y: get_y_lparam(pos),
    }
}

/// `CallWindowProc(DefWindowProc, ...)`.
///
/// # Safety
///
/// The message must be valid for the window.
unsafe fn def_window_proc(hwnd: HWND, msg: u32, w_param: WPARAM, l_param: LPARAM) -> LRESULT {
    // SAFETY: as the caller promises.
    unsafe { CallWindowProcW(Some(DefWindowProcW), hwnd, msg, w_param, l_param) }
}

/// The window procedure of SDL's windows. Translation of `WIN_WindowProc()`.
pub(crate) unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    w_param: WPARAM,
    l_param: LPARAM,
) -> LRESULT {
    // Get the window data for the window
    let data = get_window_data_from_hwnd(hwnd).or_else(|| {
        // Fallback
        window_data_from_prop(hwnd)
    });
    let (Some(data), Some(videodata)) = (data, video_data()) else {
        // SAFETY: Windows passed the message for this window.
        return unsafe { def_window_proc(hwnd, msg, w_param, l_param) };
    };
    // SAFETY: Windows passed the message for this window.
    unsafe { window_proc_for(&videodata, &data, hwnd, msg, w_param, l_param) }
}

/// The body of [`window_proc`] once the window data is known.
///
/// # Safety
///
/// The message must be valid for `hwnd`, `data`'s window.
unsafe fn window_proc_for(
    videodata: &VideoData,
    data: &WindowData,
    hwnd: HWND,
    mut msg: u32,
    mut w_param: WPARAM,
    mut l_param: LPARAM,
) -> LRESULT {
    let mut return_code: LRESULT = -1;
    let window = data.window;

    // (WMMSG_DEBUG logging of every message is not translated)

    if let Some(hook) = windows_message_hook() {
        if data.state.with(|s| s.in_modal_loop) != 0 {
            // Synthesize a message for window hooks so they can modify the message if desired
            if !dispatch_modal_loop_message_hook(&hook, hwnd, &mut msg, &mut w_param, &mut l_param)
            {
                return 0;
            }
        }
    }

    if keyboard::handle_ime_message(hwnd, msg, w_param, &mut l_param, videodata) {
        return 0;
    }

    match msg {
        WM_SHOWWINDOW => {
            if w_param != 0 {
                send_window_event(window, EventType::WINDOW_SHOWN, 0, 0);
            } else {
                send_window_event(window, EventType::WINDOW_HIDDEN, 0, 0);
            }
        }

        WM_NCACTIVATE => {
            // Don't immediately clip the cursor in case we're clicking minimize/maximize buttons
            data.state.with(|s| {
                s.postpone_clipcursor = true;
                s.clipcursor_queued = true;
            });

            /* Update the focus here, since it's possible to get WM_ACTIVATE and WM_SETFOCUS without
            actually being the foreground window, but this appears to get called in all cases where
            the global foreground window changes to and from this window.

            The current cursor position is needed here, as the message position contains old
            coordinates if the pointer moved while an overlay was active. */
            let mut pos = POINT { x: 0, y: 0 };
            // SAFETY: pos is a valid POINT.
            unsafe { GetCursorPos(&mut pos) };
            update_focus(videodata, data, w_param != 0, pos);

            /* Handle borderless windows; this event is intended for drawing the titlebar, so we need
            to stop that from happening. */
            if window_flags(window).contains(WindowFlags::BORDERLESS) {
                l_param = -1; // According to MSDN, DefWindowProc will draw a title bar if lParam != -1
            }
        }

        WM_NCUAHDRAWCAPTION | WM_NCUAHDRAWFRAME => {
            /* These messages are undocumented. They are responsible for redrawing the window frame and
            caption. Notably, WM_NCUAHDRAWCAPTION is sent when calling SetWindowText on a window.
            For borderless windows, we don't want to draw a frame or caption, so we should stop
            that from happening. */
            if window_flags(window).contains(WindowFlags::BORDERLESS) {
                return_code = 0;
            }
        }

        WM_ACTIVATE => {
            // Update the focus in case we changed focus to a child window and then away from the application
            update_focus(videodata, data, loword(w_param) != 0, message_pos());
        }

        WM_MOUSEACTIVATE => {
            if is_popup(window) {
                return MA_NOACTIVATE as LRESULT;
            }

            // Check parents to see if they are in relative mouse mode and focused
            let mut parent = core::with_window(window, |w| w.parent).ok().flatten();
            while let Some(p) = parent {
                let flags = window_flags(p);
                if flags.contains(WindowFlags::INPUT_FOCUS)
                    && flags.contains(WindowFlags::MOUSE_RELATIVE_MODE)
                {
                    return MA_NOACTIVATE as LRESULT;
                }
                parent = core::with_window(p, |w| w.parent).ok().flatten();
            }
        }

        WM_SETFOCUS => {
            // Update the focus in case it's changing between top-level windows in the same application
            update_focus(videodata, data, true, message_pos());
        }

        WM_KILLFOCUS | WM_ENTERIDLE => {
            // Update the focus in case it's changing between top-level windows in the same application
            update_focus(videodata, data, false, message_pos());
        }

        WM_POINTERENTER => {
            // NOTE: GET_POINTERID_WPARAM(wParam) is not a tool ID! It changes for each new WM_POINTERENTER, like a finger ID on a touch display. We can't identify a specific pen through these events.
            let pointerid = get_pointerid_wparam(w_param);
            let mut pointer_type = PT_POINTER;
            let Some(get_pointer_type) = videodata.get_pointer_type else {
                // Not on Windows8 or later? We shouldn't get this event, but just in case...
                // SAFETY: the arguments of this window procedure call.
                return unsafe {
                    finish(videodata, data, hwnd, msg, w_param, l_param, return_code)
                };
            };
            // SAFETY: the function was loaded with this signature.
            if unsafe { get_pointer_type(pointerid, &mut pointer_type) } == 0 {
                // oh well.
            } else if pointer_type != PT_PEN {
                // we only care about pens here.
            } else {
                let hpointer = &*POINTER_PEN_HANDLE; // just something > 0. We're using this one ID any possible pen.
                let pen = pen::find_pen_by_handle(hpointer);
                if pen != 0 {
                    pen::send_pen_proximity(get_event_timestamp(), pen, Some(window), true, true);
                } else {
                    // one can use GetPointerPenInfo() to get the current state of the pen, and check POINTER_PEN_INFO::penMask,
                    //  but the docs aren't clear if these masks are _always_ set for pens with specific features, or if they
                    //  could be unset at this moment because Windows is still deciding what capabilities the pen has, and/or
                    //  doesn't yet have valid data for them. As such, just say everything that the interface supports is
                    //  available...we don't expose this information through the public API at the moment anyhow.
                    let info = PenInfo {
                        capabilities: PenCapabilityFlags(
                            PenCapabilityFlags::PRESSURE.0
                                | PenCapabilityFlags::XTILT.0
                                | PenCapabilityFlags::YTILT.0
                                | PenCapabilityFlags::DISTANCE.0
                                | PenCapabilityFlags::ROTATION.0
                                | PenCapabilityFlags::ERASER.0,
                        ),
                        max_tilt: 90,
                        num_buttons: 1,
                        subtype: PenSubtype::Pencil,
                        ..Default::default()
                    };
                    pen::add_pen_device(
                        get_event_timestamp(),
                        None,
                        Some(window),
                        Some(&info),
                        hpointer.clone(),
                        true,
                    );
                }
                return_code = 0;
            }
        }

        WM_POINTERCAPTURECHANGED | WM_POINTERLEAVE => {
            // NOTE: GET_POINTERID_WPARAM(wParam) is not a tool ID! It changes for each new WM_POINTERENTER, like a finger ID on a touch display. We can't identify a specific pen through these events.
            let pointerid = get_pointerid_wparam(w_param);
            let mut pointer_type = PT_POINTER;
            if let Some(get_pointer_type) = videodata.get_pointer_type {
                // SAFETY: the function was loaded with this signature.
                if unsafe { get_pointer_type(pointerid, &mut pointer_type) } != 0
                    && pointer_type == PT_PEN
                {
                    let hpointer = &*POINTER_PEN_HANDLE; // just something > 0. We're using this one ID any possible pen.
                    let pen = pen::find_pen_by_handle(hpointer);
                    if pen != 0 {
                        // if this just left the _window_, we don't care. If this is no longer visible to the tablet, time to remove it!
                        if msg == WM_POINTERCAPTURECHANGED
                            || !is_pointer_flag_set_wparam(w_param, POINTER_MESSAGE_FLAG_INCONTACT)
                        {
                            // technically this isn't just _proximity_ but maybe just leaving the window. Good enough. WinTab apparently has real proximity info.
                            pen::send_pen_proximity(
                                get_event_timestamp(),
                                pen,
                                Some(window),
                                false,
                                false,
                            );
                        }
                        return_code = 0;
                    }
                }
            }
        }

        WM_POINTERDOWN | WM_POINTERUP | WM_POINTERUPDATE => 'pointer: {
            // NOTE: GET_POINTERID_WPARAM(wParam) is not a tool ID! It changes for each new WM_POINTERENTER, like a finger ID on a touch display. We can't identify a specific pen through these events.
            let pointerid = get_pointerid_wparam(w_param);
            let mut pointer_type = PT_POINTER;
            let Some(get_pointer_type) = videodata.get_pointer_type else {
                break 'pointer; // oh well.
            };
            // SAFETY: the function was loaded with this signature.
            if unsafe { get_pointer_type(pointerid, &mut pointer_type) } == 0 {
                break 'pointer; // oh well.
            } else if msg == WM_POINTERUPDATE && pointer_type == PT_MOUSE {
                data.state.with(|s| s.last_pointer_update = l_param);
                return_code = 0;
                break 'pointer;
            } else if pointer_type != PT_PEN {
                break 'pointer; // we only care about pens here.
            }

            let hpointer = &*POINTER_PEN_HANDLE; // just something > 0. We're using this one ID any possible pen.
            let pen = pen::find_pen_by_handle(hpointer);
            // SAFETY: plain data; all zeroes is valid.
            let mut pen_info: POINTER_PEN_INFO = unsafe { std::mem::zeroed() };
            if pen == 0 {
                break 'pointer; // not a pen, or not a pen we already knew about.
            }
            let Some(get_pointer_pen_info) = videodata.get_pointer_pen_info else {
                break 'pointer; // oh well.
            };
            // SAFETY: the function was loaded with this signature.
            if unsafe { get_pointer_pen_info(pointerid, &mut pen_info) } == 0 {
                break 'pointer; // oh well.
            }

            let timestamp = get_event_timestamp();
            let w = Some(window);

            let istouching = is_pointer_flag_set_wparam(w_param, POINTER_MESSAGE_FLAG_INCONTACT)
                && is_pointer_flag_set_wparam(w_param, POINTER_MESSAGE_FLAG_FIRSTBUTTON);

            // if lifting off, do it first, so any motion changes don't cause app issues.
            if !istouching {
                pen::send_pen_touch(
                    timestamp,
                    pen,
                    w,
                    (pen_info.penFlags & PEN_FLAG_INVERTED) != 0,
                    false,
                );
            }

            let pointer_info = &pen_info.pointerInfo;
            let mut tablet_bounds = RECT::default();
            let mut tablet_mapping = RECT::default();
            let (fx, fy);

            // try to get a more-precise position than is stored in lParam...GetPointerDeviceRects is available starting in Windows 8.
            // we might need to cache this somewhere (and if we cache it, we will need to update it if the display changes)...for now we'll see if GetPointerDeviceRect is fast enough.
            let have_rects = videodata.get_pointer_device_rects.is_some_and(|f| {
                // SAFETY: the function was loaded with this signature.
                unsafe {
                    f(
                        pointer_info.sourceDevice,
                        &mut tablet_bounds,
                        &mut tablet_mapping,
                    ) != 0
                }
            });
            if !have_rects {
                let mut position = POINT {
                    x: get_x_lparam(l_param),
                    y: get_y_lparam(l_param),
                };
                // SAFETY: position is a valid POINT.
                unsafe { ScreenToClient(data.hwnd, &mut position) };
                fx = position.x as f32;
                fy = position.y as f32;
            } else {
                let (ix, iy) = Window::from_raw(window).position().unwrap_or((0, 0));
                let window_pos = (ix as f32, iy as f32);

                let fac_x =
                    pointer_info.ptHimetricLocationRaw.x as f32 / tablet_bounds.right as f32;
                let fac_y =
                    pointer_info.ptHimetricLocationRaw.y as f32 / tablet_bounds.bottom as f32;

                let w = (tablet_mapping.right - tablet_mapping.left) as f32;
                let h = (tablet_mapping.bottom - tablet_mapping.top) as f32;

                fx = (tablet_mapping.left as f32 + (fac_x * w)) - window_pos.0;
                fy = (tablet_mapping.top as f32 + (fac_y * h)) - window_pos.1;
            }

            pen::send_pen_motion(timestamp, pen, w, fx, fy);
            pen::send_pen_button(
                timestamp,
                pen,
                w,
                1,
                (pen_info.penFlags & PEN_FLAG_BARREL) != 0,
            );
            pen::send_pen_button(
                timestamp,
                pen,
                w,
                2,
                (pen_info.penFlags & PEN_FLAG_ERASER) != 0,
            );

            if (pen_info.penMask & PEN_MASK_PRESSURE) != 0 {
                pen::send_pen_axis(
                    timestamp,
                    pen,
                    w,
                    PenAxis::Pressure,
                    pen_info.pressure as f32 / 1024.0,
                ); // pen_info.pressure is in the range 0..1024.
            }

            if (pen_info.penMask & PEN_MASK_ROTATION) != 0 {
                pen::send_pen_axis(
                    timestamp,
                    pen,
                    w,
                    PenAxis::Rotation,
                    pen_info.rotation as f32,
                ); // it's already in the range of 0 to 359.
            }

            if (pen_info.penMask & PEN_MASK_TILT_X) != 0 {
                pen::send_pen_axis(timestamp, pen, w, PenAxis::XTilt, pen_info.tiltX as f32);
                // it's already in the range of -90 to 90..
            }

            if (pen_info.penMask & PEN_MASK_TILT_Y) != 0 {
                pen::send_pen_axis(timestamp, pen, w, PenAxis::YTilt, pen_info.tiltY as f32);
                // it's already in the range of -90 to 90..
            }

            // if setting down, do it last, so the pen is positioned correctly from the first contact.
            if istouching {
                pen::send_pen_touch(
                    timestamp,
                    pen,
                    w,
                    (pen_info.penFlags & PEN_FLAG_INVERTED) != 0,
                    true,
                );
            }

            return_code = 0;
        }

        WM_MOUSEMOVE => {
            let flags = window_flags(window);

            if flags.contains(WindowFlags::INPUT_FOCUS) {
                let mouse_rect =
                    core::with_window(window, |w| w.core.mouse_rect).unwrap_or_default();
                let wish_clip_cursor = flags
                    .intersects(WindowFlags::MOUSE_RELATIVE_MODE | WindowFlags::MOUSE_GRABBED)
                    || (mouse_rect.w > 0 && mouse_rect.h > 0);
                if wish_clip_cursor {
                    // queue clipcursor refresh on pump finish
                    data.state.with(|s| s.clipcursor_queued = true);
                }
            }

            if !data.state.with(|s| s.mouse_tracked) {
                let mut track_mouse_event = TRACKMOUSEEVENT {
                    cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: data.hwnd,
                    dwHoverTime: 0,
                };

                // SAFETY: track_mouse_event is sized.
                if unsafe { TrackMouseEvent(&mut track_mouse_event) } != 0 {
                    data.state.with(|s| s.mouse_tracked = true);
                }

                check_async_mouse_release(get_event_timestamp(), data);
            }

            if !videodata.state.with(|s| s.raw_mouse_enabled) {
                // Only generate mouse events for real mouse
                // SAFETY: GetMessageExtraInfo has no preconditions.
                let extra = unsafe { GetMessageExtraInfo() } as u32;
                if get_mouse_message_source(extra) == MouseEventSource::Mouse
                    && l_param != data.state.with(|s| s.last_pointer_update)
                {
                    mouse::send_mouse_motion(
                        get_event_timestamp(),
                        Some(window),
                        GLOBAL_MOUSE_ID,
                        false,
                        get_x_lparam(l_param) as f32,
                        get_y_lparam(l_param) as f32,
                    );
                }
            }
        }

        WM_LBUTTONUP | WM_RBUTTONUP | WM_MBUTTONUP | WM_XBUTTONUP | WM_LBUTTONDOWN
        | WM_LBUTTONDBLCLK | WM_RBUTTONDOWN | WM_RBUTTONDBLCLK | WM_MBUTTONDOWN
        | WM_MBUTTONDBLCLK | WM_XBUTTONDOWN | WM_XBUTTONDBLCLK => {
            /* SDL_Mouse *mouse = SDL_GetMouse(); */
            if !videodata.state.with(|s| s.raw_mouse_enabled) {
                // SAFETY: GetMessageExtraInfo has no preconditions.
                let extra = unsafe { GetMessageExtraInfo() } as u32;
                if get_mouse_message_source(extra) == MouseEventSource::Mouse
                    && l_param != data.state.with(|s| s.last_pointer_update)
                {
                    check_wparam_mouse_buttons(
                        get_event_timestamp(),
                        w_param,
                        data,
                        GLOBAL_MOUSE_ID,
                    );
                }
            }
        }

        // (WM_INPUT: upstream handles raw input all at once instead of
        // using a syscall for each mouse event, see poll_raw_input())
        WM_MOUSEWHEEL | WM_MOUSEHWHEEL => {
            if !videodata.state.with(|s| s.raw_mouse_enabled) {
                let amount = hiword(w_param) as i16; // GET_WHEEL_DELTA_WPARAM
                let f_amount = amount as f32 / WHEEL_DELTA as f32;
                if msg == WM_MOUSEWHEEL {
                    mouse::send_mouse_wheel(
                        get_event_timestamp(),
                        Some(window),
                        GLOBAL_MOUSE_ID,
                        0.0,
                        f_amount,
                        MouseWheelDirection::Normal,
                    );
                } else {
                    mouse::send_mouse_wheel(
                        get_event_timestamp(),
                        Some(window),
                        GLOBAL_MOUSE_ID,
                        f_amount,
                        0.0,
                        MouseWheelDirection::Normal,
                    );
                }
            }
        }

        WM_MOUSELEAVE => {
            if !window_flags(window).contains(WindowFlags::MOUSE_CAPTURE) {
                // SAFETY: hwnd is the window's handle.
                if mouse::mouse_focus() == Some(window)
                    && !mouse::relative_mode_enabled()
                    && unsafe { IsIconic(hwnd) } == 0
                {
                    let mut cursor_pos = message_pos();
                    // SAFETY: cursor_pos is a valid POINT.
                    unsafe { ScreenToClient(hwnd, &mut cursor_pos) };
                    let (was_touch, touch_mouse_events) = mouse::with_mouse(|m| {
                        let was = m.was_touch_mouse_events;
                        if was {
                            m.was_touch_mouse_events = false; // not anymore
                        }
                        (was, m.touch_mouse_events)
                    });
                    if !was_touch {
                        // we're not a touch handler causing a mouse leave?
                        mouse::send_mouse_motion(
                            get_event_timestamp(),
                            Some(window),
                            GLOBAL_MOUSE_ID,
                            false,
                            cursor_pos.x as f32,
                            cursor_pos.y as f32,
                        );
                    } else if touch_mouse_events {
                        // touch handling? convert touch to mouse events
                        mouse::send_mouse_motion(
                            get_event_timestamp(),
                            Some(window),
                            TOUCH_MOUSE_ID,
                            false,
                            cursor_pos.x as f32,
                            cursor_pos.y as f32,
                        );
                    } else {
                        // normal handling
                        mouse::send_mouse_motion(
                            get_event_timestamp(),
                            Some(window),
                            GLOBAL_MOUSE_ID,
                            false,
                            cursor_pos.x as f32,
                            cursor_pos.y as f32,
                        );
                    }
                }

                if !mouse::relative_mode_enabled() {
                    // When WM_MOUSELEAVE is fired we can be assured that the cursor has left the window
                    mouse::set_mouse_focus(None);
                }
            }

            // Once we get WM_MOUSELEAVE we're guaranteed that the window is no longer tracked
            data.state.with(|s| s.mouse_tracked = false);

            return_code = 0;
        }

        WM_KEYDOWN | WM_SYSKEYDOWN => {
            if skip_alt_gr_left_control(w_param, l_param) {
                return_code = 0;
            } else {
                let (code, rawcode, virtual_key) =
                    windows_scan_code_to_sdl_scan_code(l_param, w_param);

                // Detect relevant keyboard shortcuts
                if code == Scancode::F4 && sdl_keyboard::mod_state().intersects(Keymod::ALT) {
                    // ALT+F4: Close window
                    if should_generate_window_close_on_alt_f4() {
                        send_window_event(window, EventType::WINDOW_CLOSE_REQUESTED, 0, 0);
                    }
                }

                let text_input_active =
                    core::with_window(window, |w| w.core.text_input_active).unwrap_or(false);
                if virtual_key
                    || !videodata.state.with(|s| s.raw_keyboard_enabled)
                    || text_input_active
                {
                    sdl_keyboard::send_keyboard_key(
                        get_event_timestamp(),
                        GLOBAL_KEYBOARD_ID,
                        rawcode as i32,
                        code,
                        true,
                    );
                }

                return_code = 0;
            }
        }

        WM_SYSKEYUP | WM_KEYUP => {
            if skip_alt_gr_left_control(w_param, l_param) {
                return_code = 0;
            } else {
                let (code, rawcode, virtual_key) =
                    windows_scan_code_to_sdl_scan_code(l_param, w_param);
                let text_input_active =
                    core::with_window(window, |w| w.core.text_input_active).unwrap_or(false);

                if virtual_key
                    || !videodata.state.with(|s| s.raw_keyboard_enabled)
                    || text_input_active
                {
                    if code == Scancode::PRINTSCREEN && !sdl_keyboard::is_pressed(code) {
                        sdl_keyboard::send_keyboard_key(
                            get_event_timestamp(),
                            GLOBAL_KEYBOARD_ID,
                            rawcode as i32,
                            code,
                            true,
                        );
                    }
                    sdl_keyboard::send_keyboard_key(
                        get_event_timestamp(),
                        GLOBAL_KEYBOARD_ID,
                        rawcode as i32,
                        code,
                        false,
                    );
                }
                return_code = 0;
            }
        }

        WM_UNICHAR => {
            if w_param == UNICODE_NOCHAR as usize {
                return_code = 1;
            } else {
                if sdl_keyboard::text_input_active(window) {
                    if let Some(c) = char::from_u32(w_param as u32) {
                        let mut text = [0u8; 4];
                        sdl_keyboard::send_keyboard_text(c.encode_utf8(&mut text));
                    }
                }
                return_code = 0;
            }
        }

        WM_CHAR => {
            if sdl_keyboard::text_input_active(window) {
                /* Characters outside Unicode Basic Multilingual Plane (BMP)
                 * are coded as so called "surrogate pair" in two separate UTF-16 character events.
                 * Cache high surrogate until next character event. */
                let ch = w_param as u16;
                if (0xd800..=0xdbff).contains(&ch) {
                    data.state.with(|s| s.high_surrogate = ch);
                } else {
                    let high_surrogate = data.state.with(|s| std::mem::take(&mut s.high_surrogate));
                    let utf16: Vec<u16> = if high_surrogate != 0 {
                        vec![high_surrogate, ch]
                    } else {
                        vec![ch]
                    };

                    if let Some(utf8) = wide_char_to_multi_byte(
                        windows_sys::Win32::Globalization::CP_UTF8,
                        windows_sys::Win32::Globalization::WC_ERR_INVALID_CHARS,
                        &utf16,
                    ) {
                        if !utf8.is_empty() {
                            sdl_keyboard::send_keyboard_text(&String::from_utf8_lossy(&utf8));
                        }
                    }
                }
            } else {
                data.state.with(|s| s.high_surrogate = 0);
            }

            return_code = 0;
        }

        WM_INPUTLANGCHANGE => {
            keyboard::update_keymap(true);
            return_code = 1;
        }

        WM_NCLBUTTONDOWN => {
            data.state.with(|s| s.in_title_click = true);

            // Fix for 500ms hang after user clicks on the title bar, but before moving mouse
            // Reference: https://gamedev.net/forums/topic/672094-keeping-things-moving-during-win32-moveresize-events/5254386/
            // SAFETY: hwnd is the window's handle.
            if unsafe { SendMessageW(hwnd, WM_NCHITTEST, w_param, l_param) } == HTCAPTION as LRESULT
            {
                let mut cursor_pos = POINT::default();
                // SAFETY: cursor_pos is a valid POINT.
                unsafe {
                    GetCursorPos(&mut cursor_pos); // want the most current pos so as to not cause position change
                    ScreenToClient(hwnd, &mut cursor_pos);
                    PostMessageW(
                        hwnd,
                        WM_MOUSEMOVE,
                        0,
                        (cursor_pos.x as u32 | ((cursor_pos.y as i16 as u32) << 16)) as LPARAM,
                    );
                }
            }
        }

        WM_CAPTURECHANGED => {
            data.state.with(|s| s.in_title_click = false);

            // The mouse may have been released during a modal loop
            check_async_mouse_release(get_event_timestamp(), data);
        }

        WM_GETMINMAXINFO => 'minmax: {
            // If this is an expected size change, allow it
            if data.state.with(|s| s.expected_resize) {
                break 'minmax;
            }

            // Get the current position of our window
            let mut size = RECT::default();
            // SAFETY: size is a valid RECT.
            unsafe { GetWindowRect(hwnd, &mut size) };
            let x = size.left;
            let y = size.top;

            // Calculate current size of our window
            let sdl_window = Window::from_raw(window);
            let (mut w, mut h) = sdl_window.size().unwrap_or((0, 0));
            let (mut min_w, mut min_h) = sdl_window.minimum_size().unwrap_or((0, 0));
            let (mut max_w, mut max_h) = sdl_window.maximum_size().unwrap_or((0, 0));
            let flags = sdl_window.flags().unwrap_or_default();

            /* Store in min_w and min_h difference between current size and minimal
            size so we don't need to call AdjustWindowRectEx twice */
            min_w -= w;
            min_h -= h;
            let constrain_max_size = if max_w != 0 && max_h != 0 {
                max_w -= w;
                max_h -= h;
                true
            } else {
                false
            };

            if !flags.contains(WindowFlags::BORDERLESS) && !is_popup(window) {
                size.top = 0;
                size.left = 0;
                size.bottom = h;
                size.right = w;
                let _ = adjust_window_rect_for_hwnd(hwnd, &mut size, 0);
                w = size.right - size.left;
                h = size.bottom - size.top;
            }

            // Fix our size to the current size
            // SAFETY: for WM_GETMINMAXINFO, lParam points at a MINMAXINFO.
            let info = unsafe { &mut *(l_param as *mut MINMAXINFO) };
            if flags.contains(WindowFlags::RESIZABLE) {
                if flags.contains(WindowFlags::BORDERLESS) {
                    // SAFETY: GetSystemMetrics takes an index.
                    let (screen_w, screen_h) =
                        unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) };
                    info.ptMaxSize.x = w.max(screen_w);
                    info.ptMaxSize.y = h.max(screen_h);
                    info.ptMaxPosition.x = 0.min((screen_w - w) / 2);
                    info.ptMaxPosition.y = 0.min((screen_h - h) / 2);
                }
                info.ptMinTrackSize.x = w + min_w;
                info.ptMinTrackSize.y = h + min_h;
                if constrain_max_size {
                    info.ptMaxTrackSize.x = w + max_w;
                    info.ptMaxTrackSize.y = h + max_h;
                }
            } else {
                info.ptMaxSize.x = w;
                info.ptMaxSize.y = h;
                info.ptMaxPosition.x = x;
                info.ptMaxPosition.y = y;
                info.ptMinTrackSize.x = w;
                info.ptMinTrackSize.y = h;
                info.ptMaxTrackSize.x = w;
                info.ptMaxTrackSize.y = h;
            }
            return_code = 0;
        }

        WM_WINDOWPOSCHANGING => {
            let (expected_resize, in_modal_loop) =
                data.state.with(|s| (s.expected_resize, s.in_modal_loop));
            if expected_resize {
                return_code = 0;
            } else if in_modal_loop != 0 {
                // SAFETY: for WM_WINDOWPOSCHANGING, lParam points at a WINDOWPOS.
                let windowpos = unsafe { &mut *(l_param as *mut WINDOWPOS) };

                /* While in a modal loop, the size may only be updated if the window is being resized interactively.
                 * Set the SWP_NOSIZE flag if the reported size hasn't changed from the last WM_WINDOWPOSCHANGING
                 * event, or a size set programmatically may end up being overwritten by old size data.
                 */
                data.state.with(|s| {
                    if s.last_modal_width == windowpos.cx && s.last_modal_height == windowpos.cy {
                        windowpos.flags |= SWP_NOSIZE;
                    }

                    s.last_modal_width = windowpos.cx;
                    s.last_modal_height = windowpos.cy;
                });

                return_code = 0;
            }
        }

        WM_WINDOWPOSCHANGED => 'changed: {
            let original_display_id = core::with_window(window, |w| w.core.display_id).unwrap_or(0);
            // SAFETY: for WM_WINDOWPOSCHANGED, lParam points at a WINDOWPOS.
            let windowpos_flags = unsafe { (*(l_param as *const WINDOWPOS)).flags };

            if (windowpos_flags & SWP_SHOWWINDOW) != 0 {
                send_window_event(window, EventType::WINDOW_SHOWN, 0, 0);
            }

            // These must be set after sending SDL_EVENT_WINDOW_SHOWN as that may apply pending
            // window operations that change the window state.
            // SAFETY: hwnd is the window's handle.
            let iconic = unsafe { IsIconic(hwnd) } != 0;
            // SAFETY: as above.
            let zoomed = unsafe { IsZoomed(hwnd) } != 0;

            if iconic {
                send_window_event(window, EventType::WINDOW_MINIMIZED, 0, 0);
            } else if zoomed {
                if window_flags(window).contains(WindowFlags::MINIMIZED) {
                    // If going from minimized to maximized, send the restored event first.
                    send_window_event(window, EventType::WINDOW_RESTORED, 0, 0);
                }
                send_window_event(window, EventType::WINDOW_MAXIMIZED, 0, 0);
                data.state.with(|s| s.force_ws_maximizebox = true);
            } else if window_flags(window)
                .intersects(WindowFlags::MAXIMIZED | WindowFlags::MINIMIZED)
            {
                send_window_event(window, EventType::WINDOW_RESTORED, 0, 0);

                /* If resizable was forced on for the maximized window, clear the style flags now,
                 * but not if the window is fullscreen, as this needs to be preserved in that case.
                 */
                let flags = window_flags(window);
                if !flags.contains(WindowFlags::FULLSCREEN) {
                    data.state.with(|s| s.force_ws_maximizebox = false);
                    set_window_resizable(window, flags.contains(WindowFlags::RESIZABLE));
                }
            }

            if (windowpos_flags & SWP_HIDEWINDOW) != 0 {
                send_window_event(window, EventType::WINDOW_HIDDEN, 0, 0);
            }

            // When the window is minimized it's resized to the dock icon size, ignore this
            if iconic {
                break 'changed;
            }

            if data.state.with(|s| s.initializing) {
                break 'changed;
            }

            if !data.state.with(|s| s.disable_move_size_events) {
                let mut rect = RECT::default();
                // SAFETY: rect is a RECT, laid out as two POINTs.
                if unsafe { GetClientRect(hwnd, &mut rect) } != 0 && window_rect_valid(&rect) {
                    // SAFETY: as above.
                    unsafe {
                        let points = (&mut rect as *mut RECT).cast::<POINT>();
                        ClientToScreen(hwnd, points);
                        ClientToScreen(hwnd, points.add(1));
                    }

                    let (x, y) = global_to_relative_for_window(window, rect.left, rect.top);
                    send_window_event(window, EventType::WINDOW_MOVED, x, y);
                }

                // Moving the window from one display to another can change the size of the window (in the handling of SDL_EVENT_WINDOW_MOVED), so we need to re-query the bounds
                // SAFETY: rect is a valid RECT.
                if unsafe { GetClientRect(hwnd, &mut rect) } != 0 && window_rect_valid(&rect) {
                    let w = rect.right;
                    let h = rect.bottom;

                    send_window_event(window, EventType::WINDOW_RESIZED, w, h);
                }
            }

            update_clip_cursor(window);

            // Update the window display position
            if core::with_window(window, |w| w.core.display_id).unwrap_or(0) != original_display_id
            {
                // Display changed, check ICC profile
                update_window_icc_profile(window, true);
            }

            // Update the position of any child windows
            let children = core::with_window(window, |w| w.children.clone()).unwrap_or_default();
            for win in children {
                // Don't update hidden child popup windows, their relative position doesn't change
                if is_popup(win) && !window_flags(win).contains(WindowFlags::HIDDEN) {
                    let _ = set_window_position_internal(
                        win,
                        SWP_NOCOPYBITS | SWP_NOACTIVATE,
                        WindowRect::Current,
                    );
                }
            }
        }

        WM_ENTERSIZEMOVE | WM_ENTERMENULOOP => {
            if let Some(hook) = windows_message_hook() {
                if !dispatch_modal_loop_message_hook(
                    &hook,
                    hwnd,
                    &mut msg,
                    &mut w_param,
                    &mut l_param,
                ) {
                    return 0;
                }
            }

            let in_modal_loop = data.state.with(|s| {
                s.in_modal_loop += 1;
                s.in_modal_loop
            });
            if in_modal_loop == 1 {
                let mut rect = RECT::default();
                // SAFETY: rect is a valid RECT.
                unsafe { GetWindowRect(data.hwnd, &mut rect) };
                let (x, y, w, h) =
                    core::with_window(window, |w| (w.core.x, w.core.y, w.core.w, w.core.h))
                        .unwrap_or_default();
                data.state.with(|s| {
                    s.last_modal_width = rect.right - rect.left;
                    s.last_modal_height = rect.bottom - rect.top;

                    s.initial_size_rect.left = x;
                    s.initial_size_rect.right = x + w;
                    s.initial_size_rect.top = y;
                    s.initial_size_rect.bottom = y + h;
                });

                // SAFETY: hwnd is the window's handle; no timer procedure.
                unsafe { SetTimer(hwnd, main_callbacks_timer_id(), USER_TIMER_MINIMUM, None) };

                // Reset the keyboard, as we won't get any key up events during the modal loop
                sdl_keyboard::reset_keyboard();
            }
        }

        WM_TIMER => {
            if w_param == main_callbacks_timer_id() {
                core::on_window_live_resize_update(window);

                // (DwmFlush() is disabled upstream: it locks up the Windows
                // compositor when called by Steam)
                return 0;
            }
        }

        WM_EXITSIZEMOVE | WM_EXITMENULOOP => {
            let in_modal_loop = data.state.with(|s| {
                s.in_modal_loop -= 1;
                s.in_modal_loop
            });
            if in_modal_loop == 0 {
                // SAFETY: hwnd is the window's handle.
                unsafe { KillTimer(hwnd, main_callbacks_timer_id()) };
            }
        }

        WM_SIZING => 'sizing: {
            let edge = w_param as u32;
            // SAFETY: for WM_SIZING, lParam points at the drag RECT.
            let drag_rect = unsafe { &mut *(l_param as *mut RECT) };
            let mut client_drag_rect = *drag_rect;
            let Ok((min_aspect, max_aspect)) =
                core::with_window(window, |w| (w.min_aspect, w.max_aspect))
            else {
                break 'sizing;
            };
            let lock_aspect_ratio = max_aspect == min_aspect;
            let mut rc = RECT::default();
            let roundf = crate::stdlib::math::roundf;

            // if aspect ratio constraints are not enabled then skip this message
            if min_aspect <= 0.0 && max_aspect <= 0.0 {
                break 'sizing;
            }

            // unadjust the dragRect from the window rect to the client rect
            // SAFETY: rc is a valid RECT; hwnd is the window's handle.
            let (style, style_ex, menu) = unsafe {
                SetRectEmpty(&mut rc);
                (
                    super::get_window_long_ptr(hwnd, GWL_STYLE) as u32,
                    super::get_window_long_ptr(hwnd, GWL_EXSTYLE) as u32,
                    !GetMenu(hwnd).is_null(),
                )
            };
            // SAFETY: rc is a valid RECT.
            if unsafe { AdjustWindowRectEx(&mut rc, style, menu as i32, style_ex) } == 0 {
                break 'sizing;
            }

            client_drag_rect.left -= rc.left;
            client_drag_rect.top -= rc.top;
            client_drag_rect.right -= rc.right;
            client_drag_rect.bottom -= rc.bottom;

            let mut w = client_drag_rect.right - client_drag_rect.left;
            let mut h = client_drag_rect.bottom - client_drag_rect.top;
            let new_aspect = w as f32 / h as f32;

            // handle the special case in which the min ar and max ar are the same so the window can size symmetrically
            if lock_aspect_ratio {
                match edge {
                    WMSZ_LEFT | WMSZ_RIGHT => {
                        h = roundf(w as f32 / max_aspect) as i32;
                    }
                    _ => {
                        // resizing via corners or top or bottom
                        w = roundf(h as f32 * max_aspect) as i32;
                    }
                }
            } else {
                match edge {
                    WMSZ_LEFT | WMSZ_RIGHT => {
                        if max_aspect > 0.0 && new_aspect > max_aspect {
                            w = roundf(h as f32 * max_aspect) as i32;
                        } else if min_aspect > 0.0 && new_aspect < min_aspect {
                            w = roundf(h as f32 * min_aspect) as i32;
                        }
                    }
                    WMSZ_TOP | WMSZ_BOTTOM => {
                        if min_aspect > 0.0 && new_aspect < min_aspect {
                            h = roundf(w as f32 / min_aspect) as i32;
                        } else if max_aspect > 0.0 && new_aspect > max_aspect {
                            h = roundf(w as f32 / max_aspect) as i32;
                        }
                    }

                    _ => {
                        // resizing via corners
                        if max_aspect > 0.0 && new_aspect > max_aspect {
                            w = roundf(h as f32 * max_aspect) as i32;
                        } else if min_aspect > 0.0 && new_aspect < min_aspect {
                            h = roundf(w as f32 / min_aspect) as i32;
                        }
                    }
                }
            }

            let initial_size_rect = data.state.with(|s| s.initial_size_rect);
            match edge {
                WMSZ_LEFT => {
                    client_drag_rect.left = client_drag_rect.right - w;
                    if lock_aspect_ratio {
                        client_drag_rect.top =
                            (initial_size_rect.bottom + initial_size_rect.top - h) / 2;
                    }
                    client_drag_rect.bottom = h + client_drag_rect.top;
                }
                WMSZ_BOTTOMLEFT => {
                    client_drag_rect.left = client_drag_rect.right - w;
                    client_drag_rect.bottom = h + client_drag_rect.top;
                }
                WMSZ_RIGHT => {
                    client_drag_rect.right = w + client_drag_rect.left;
                    if lock_aspect_ratio {
                        client_drag_rect.top =
                            (initial_size_rect.bottom + initial_size_rect.top - h) / 2;
                    }
                    client_drag_rect.bottom = h + client_drag_rect.top;
                }
                WMSZ_TOPRIGHT => {
                    client_drag_rect.right = w + client_drag_rect.left;
                    client_drag_rect.top = client_drag_rect.bottom - h;
                }
                WMSZ_TOP => {
                    if lock_aspect_ratio {
                        client_drag_rect.left =
                            (initial_size_rect.right + initial_size_rect.left - w) / 2;
                    }
                    client_drag_rect.right = w + client_drag_rect.left;
                    client_drag_rect.top = client_drag_rect.bottom - h;
                }
                WMSZ_TOPLEFT => {
                    client_drag_rect.left = client_drag_rect.right - w;
                    client_drag_rect.top = client_drag_rect.bottom - h;
                }
                WMSZ_BOTTOM => {
                    if lock_aspect_ratio {
                        client_drag_rect.left =
                            (initial_size_rect.right + initial_size_rect.left - w) / 2;
                    }
                    client_drag_rect.right = w + client_drag_rect.left;
                    client_drag_rect.bottom = h + client_drag_rect.top;
                }
                WMSZ_BOTTOMRIGHT => {
                    client_drag_rect.right = w + client_drag_rect.left;
                    client_drag_rect.bottom = h + client_drag_rect.top;
                }
                _ => {}
            }

            // convert the client rect to a window rect
            // SAFETY: client_drag_rect is a valid RECT.
            if unsafe { AdjustWindowRectEx(&mut client_drag_rect, style, menu as i32, style_ex) }
                == 0
            {
                break 'sizing;
            }

            *drag_rect = client_drag_rect;
        }

        WM_SETCURSOR => {
            let hittest = loword(l_param as usize);
            let cursor = winmouse::current_cursor();
            if hittest == HTCLIENT as u16 {
                // SAFETY: cursor is a cursor handle (or NULL to hide it).
                unsafe { SetCursor(cursor) };
                return_code = 1;
            } else if !G_WINDOW_FRAME_USABLE_WHILE_CURSOR_HIDDEN.load(Ordering::Relaxed)
                && cursor.is_null()
            {
                // SAFETY: NULL hides the cursor.
                unsafe { SetCursor(std::ptr::null_mut()) };
                return_code = 1;
            }
        }

        // We were occluded, refresh our display
        WM_PAINT => {
            let mut rect = RECT::default();
            // SAFETY: rect is a valid RECT; hwnd is the window's handle.
            if unsafe { GetUpdateRect(hwnd, &mut rect, 0) } != 0 {
                // SAFETY: as above.
                let style = unsafe { super::get_window_long_ptr(hwnd, GWL_EXSTYLE) } as u32;

                /* Composited windows will continue to receive WM_PAINT messages for update
                regions until the window is actually painted through Begin/EndPaint */
                if (style & windows_sys::Win32::UI::WindowsAndMessaging::WS_EX_COMPOSITED) != 0 {
                    // SAFETY: ps is a valid PAINTSTRUCT.
                    unsafe {
                        let mut ps: PAINTSTRUCT = std::mem::zeroed();
                        BeginPaint(hwnd, &mut ps);
                        EndPaint(hwnd, &ps);
                    }
                }

                // SAFETY: NULL validates the whole client area.
                unsafe { ValidateRect(hwnd, std::ptr::null()) };
                send_window_event(window, EventType::WINDOW_EXPOSED, 0, 0);
            }
            return_code = 0;
        }

        // We'll do our own drawing, prevent flicker
        WM_ERASEBKGND => {
            if should_clear_window_on_erase_background(videodata, data) {
                videodata.state.with(|s| s.cleared = true);
                let mut client_rect = RECT::default();
                // SAFETY: the brush is deleted after use.
                // FIXME (upstream): the DC from GetDC(hwnd) is never released.
                unsafe {
                    GetClientRect(hwnd, &mut client_rect);
                    let brush = CreateSolidBrush(0);
                    FillRect(GetDC(hwnd), &client_rect, brush);
                    DeleteObject(brush);
                }
            }
            return 1;
        }

        WM_SYSCOMMAND => {
            if !G_WINDOWS_ENABLE_MENU_MNEMONICS.load(Ordering::Relaxed)
                && (w_param & 0xFFF0) == SC_KEYMENU as usize
            {
                return 0;
            }

            // Don't start the screensaver or blank the monitor in fullscreen apps
            if ((w_param & 0xFFF0) == SC_SCREENSAVE as usize
                || (w_param & 0xFFF0) == SC_MONITORPOWER as usize)
                && with_device(|v| v.suspend_screensaver).unwrap_or(false)
            {
                return 0;
            }
        }

        WM_CLOSE => {
            send_window_event(window, EventType::WINDOW_CLOSE_REQUESTED, 0, 0);
            return_code = 0;
        }

        WM_TOUCH => 'touch: {
            let (Some(get_touch_input_info), Some(close_touch_input_handle)) = (
                videodata.get_touch_input_info,
                videodata.close_touch_input_handle,
            ) else {
                break 'touch;
            };
            let num_inputs = loword(w_param) as usize;
            // SAFETY: plain data; all zeroes is valid.
            let mut inputs: Vec<TOUCHINPUT> = vec![unsafe { std::mem::zeroed() }; num_inputs];
            // SAFETY: the function was loaded with this signature; inputs
            // holds num_inputs entries.
            if unsafe {
                get_touch_input_info(
                    l_param as HTOUCHINPUT,
                    num_inputs as u32,
                    inputs.as_mut_ptr(),
                    size_of::<TOUCHINPUT>() as i32,
                )
            } != 0
            {
                let mut rect = RECT::default();

                // SAFETY: rect is a valid RECT.
                if unsafe { GetClientRect(hwnd, &mut rect) } == 0 || !window_rect_valid(&rect) {
                    break 'touch;
                }
                // SAFETY: rect is a RECT, laid out as two POINTs.
                unsafe {
                    let points = (&mut rect as *mut RECT).cast::<POINT>();
                    ClientToScreen(hwnd, points);
                    ClientToScreen(hwnd, points.add(1));
                }
                rect.top *= 100;
                rect.left *= 100;
                rect.bottom *= 100;
                rect.right *= 100;

                for input in &inputs {
                    let w = rect.right - rect.left;
                    let h = rect.bottom - rect.top;

                    let touch_id = input.hSource as usize as touch::TouchID;
                    let finger_id = input.dwID as touch::FingerID + 1;

                    /* TODO: Can we use GetRawInputDeviceInfo and HID info to
                    determine if this is a direct or indirect touch device?
                    */
                    touch::add_touch(
                        touch_id,
                        TouchDeviceType::Direct,
                        if (input.dwFlags & TOUCHEVENTF_PEN) == TOUCHEVENTF_PEN {
                            "pen"
                        } else {
                            "touch"
                        },
                    );

                    // Get the normalized coordinates for the window
                    let x = if w <= 1 {
                        0.5
                    } else {
                        (input.x - rect.left) as f32 / (w - 1) as f32
                    };
                    let y = if h <= 1 {
                        0.5
                    } else {
                        (input.y - rect.top) as f32 / (h - 1) as f32
                    };

                    // FIXME: Should we use the input->dwTime field for the tick source of the timestamp?
                    if (input.dwFlags & TOUCHEVENTF_DOWN) != 0 {
                        touch::send_touch(
                            get_event_timestamp(),
                            touch_id,
                            finger_id,
                            Some(window),
                            EventType::FINGER_DOWN,
                            x,
                            y,
                            1.0,
                        );
                    }
                    if (input.dwFlags & TOUCHEVENTF_MOVE) != 0 {
                        touch::send_touch_motion(
                            get_event_timestamp(),
                            touch_id,
                            finger_id,
                            Some(window),
                            x,
                            y,
                            1.0,
                        );
                    }
                    if (input.dwFlags & TOUCHEVENTF_UP) != 0 {
                        touch::send_touch(
                            get_event_timestamp(),
                            touch_id,
                            finger_id,
                            Some(window),
                            EventType::FINGER_UP,
                            x,
                            y,
                            1.0,
                        );
                    }
                }
            }

            // SAFETY: the function was loaded with this signature.
            unsafe { close_touch_input_handle(l_param as HTOUCHINPUT) };
            return 0;
        }

        WM_TABLET_QUERYSYSTEMGESTURESTATUS => {
            /* See https://msdn.microsoft.com/en-us/library/windows/desktop/bb969148(v=vs.85).aspx .
             * If we're handling our own touches, we don't want any gestures.
             * Not all of these settings are documented.
             * The use of the undocumented ones was suggested by https://github.com/bjarkeck/GCGJ/blob/master/Monogame/Windows/WinFormsGameForm.cs . */
            return TABLET_DISABLE_PRESSANDHOLD // disables press and hold (right-click) gesture
                | TABLET_DISABLE_PENTAPFEEDBACK // disables UI feedback on pen up (waves)
                | TABLET_DISABLE_PENBARRELFEEDBACK // disables UI feedback on pen button down (circle)
                | TABLET_DISABLE_TOUCHUIFORCEON
                | TABLET_DISABLE_TOUCHUIFORCEOFF
                | TABLET_DISABLE_TOUCHSWITCH
                | TABLET_DISABLE_FLICKS // disables pen flicks (back, forward, drag down, drag up)
                | TABLET_DISABLE_SMOOTHSCROLLING
                | TABLET_DISABLE_FLICKFALLBACKKEYS;
        }

        WM_DROPFILES => {
            let drop = w_param as HDROP;
            // SAFETY: wParam is the HDROP of the drop; buffers hold the
            // sizes passed.
            unsafe {
                let count = DragQueryFileW(drop, 0xFFFFFFFF, std::ptr::null_mut(), 0);
                for i in 0..count {
                    let size = DragQueryFileW(drop, i, std::ptr::null_mut(), 0) + 1;
                    let mut buffer = vec![0u16; size as usize];
                    if DragQueryFileW(drop, i, buffer.as_mut_ptr(), size) != 0 {
                        let file = wide_to_utf8(&buffer);
                        send_drop_file(Some(window), None, &file);
                    }
                }
            }
            send_drop_complete(Some(window));
            // SAFETY: the drop is finished with.
            unsafe { DragFinish(drop) };
            return 0;
        }

        WM_DISPLAYCHANGE => {
            // Reacquire displays if any were added or removed
            modes::refresh_displays(videodata);
        }

        WM_NCCALCSIZE => {
            let window_flags = window_flags(window);
            if w_param == 1
                && window_flags.contains(WindowFlags::BORDERLESS)
                && !window_flags.contains(WindowFlags::FULLSCREEN)
            {
                // When borderless, need to tell windows that the size of the non-client area is 0
                // SAFETY: for WM_NCCALCSIZE with wParam TRUE, lParam points at
                // an NCCALCSIZE_PARAMS.
                let params = unsafe { &mut *(l_param as *mut NCCALCSIZE_PARAMS) };
                // SAFETY: plain data; all zeroes is valid.
                let mut placement: WINDOWPLACEMENT = unsafe { std::mem::zeroed() };
                // FIXME (upstream): placement.length is never set before
                // GetWindowPlacement().
                // SAFETY: placement is a valid WINDOWPLACEMENT.
                if unsafe { GetWindowPlacement(hwnd, &mut placement) } != 0
                    && placement.showCmd == SW_MAXIMIZE as u32
                {
                    // Maximized borderless windows should use the monitor work area.
                    // SAFETY: hwnd is the window's handle.
                    let mut h_monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONULL) };
                    if h_monitor.is_null() {
                        // The returned monitor can be null when restoring from minimized, so use the last coordinates.
                        let (x, y) =
                            core::with_window(window, |w| (w.core.windowed.x, w.core.windowed.y))
                                .unwrap_or((0, 0));
                        let pt = POINT { x, y };
                        // SAFETY: MonitorFromPoint takes a point by value.
                        h_monitor = unsafe { MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST) };
                    }
                    if !h_monitor.is_null() {
                        let mut info = MONITORINFO {
                            cbSize: size_of::<MONITORINFO>() as u32,
                            ..Default::default()
                        };
                        // SAFETY: info is sized.
                        if unsafe { GetMonitorInfoW(h_monitor, &mut info) } != 0 {
                            params.rgrc[0] = info.rcWork;
                        }
                    }
                } else if !window_flags.contains(WindowFlags::RESIZABLE)
                    && !data.state.with(|s| s.force_ws_maximizebox)
                {
                    let (w, h) = core::with_window(window, |w| {
                        if w.core.last_size_pending {
                            (w.pending.w, w.pending.h)
                        } else {
                            (w.core.floating.w, w.core.floating.h)
                        }
                    })
                    .unwrap_or((0, 0));
                    params.rgrc[0].right = params.rgrc[0].left + w;
                    params.rgrc[0].bottom = params.rgrc[0].top + h;
                }
                return 0;
            }
        }

        WM_NCHITTEST => {
            let flags = window_flags(window);

            if flags.contains(WindowFlags::TOOLTIP) {
                return HTTRANSPARENT as LRESULT;
            }

            let has_hit_test = core::with_window(window, |w| w.hit_test.is_some()).unwrap_or(false);
            if has_hit_test {
                let mut winpoint = POINT {
                    x: get_x_lparam(l_param),
                    y: get_y_lparam(l_param),
                };
                // SAFETY: winpoint is a valid POINT.
                if unsafe { ScreenToClient(hwnd, &mut winpoint) } != 0 {
                    let point = Point {
                        x: winpoint.x,
                        y: winpoint.y,
                    };
                    let rc = Window::from_raw(window).hit_test(point);
                    let post_hit_test = |ret: u32| -> LRESULT {
                        send_window_event(window, EventType::WINDOW_HIT_TEST, 0, 0);
                        ret as LRESULT
                    };
                    match rc {
                        Some(HitTestResult::Draggable) => {
                            /* If the mouse button state is something other than none or left button down,
                             * return HTCLIENT, or Windows will eat the button press.
                             */
                            let button_state = mouse::global_mouse_state().2;
                            if button_state.0 != 0 && (button_state.0 & 0x01) == 0 {
                                // Set focus in case it was lost while previously moving over a draggable area.
                                mouse::set_mouse_focus(Some(window));
                                return HTCLIENT as LRESULT;
                            }

                            return post_hit_test(HTCAPTION);
                        }
                        Some(HitTestResult::ResizeTopLeft) => return post_hit_test(HTTOPLEFT),
                        Some(HitTestResult::ResizeTop) => return post_hit_test(HTTOP),
                        Some(HitTestResult::ResizeTopRight) => return post_hit_test(HTTOPRIGHT),
                        Some(HitTestResult::ResizeRight) => return post_hit_test(HTRIGHT),
                        Some(HitTestResult::ResizeBottomRight) => {
                            return post_hit_test(HTBOTTOMRIGHT)
                        }
                        Some(HitTestResult::ResizeBottom) => return post_hit_test(HTBOTTOM),
                        Some(HitTestResult::ResizeBottomLeft) => {
                            return post_hit_test(HTBOTTOMLEFT)
                        }
                        Some(HitTestResult::ResizeLeft) => return post_hit_test(HTLEFT),
                        Some(HitTestResult::Normal) => return HTCLIENT as LRESULT,
                        None => {}
                    }
                }
                // If we didn't return, this will call DefWindowProc below.
            }
        }

        WM_GETDPISCALEDSIZE => {
            // Windows 10 Creators Update+
            /* Documented as only being sent to windows that are per-monitor V2 DPI aware.

            Experimentation shows it's only sent during interactive dragging, not in response to
            SetWindowPos. */
            if let (Some(get_dpi_for_window), Some(_)) = (
                videodata.get_dpi_for_window,
                videodata.adjust_window_rect_ex_for_dpi,
            ) {
                /* Windows expects applications to scale their window rects linearly
                when dragging between monitors with different DPI's.
                e.g. a 100x100 window dragged to a 200% scaled monitor
                becomes 200x200.

                For SDL, we instead want the client size to scale linearly.
                This is not the same as the window rect scaling linearly,
                because Windows doesn't scale the non-client area (titlebar etc.)
                linearly. So, we need to handle this message to request custom
                scaling. */

                let next_dpi = w_param as u32;
                // SAFETY: the function was loaded with this signature.
                let prev_dpi = unsafe { get_dpi_for_window(hwnd) };
                // SAFETY: for WM_GETDPISCALEDSIZE, lParam points at a SIZE.
                let size_in_out = unsafe { &mut *(l_param as *mut SIZE) };
                let flags = window_flags(window);

                // Subtract the window frame size that would have been used at prevDPI
                let (query_client_w_win, query_client_h_win) = {
                    let mut rect = RECT::default();

                    if !flags.contains(WindowFlags::BORDERLESS) && !is_popup(window) {
                        let _ = adjust_window_rect_for_hwnd(hwnd, &mut rect, prev_dpi);
                    }

                    let frame_w = -rect.left + rect.right;
                    let frame_h = -rect.top + rect.bottom;

                    (size_in_out.cx - frame_w, size_in_out.cy - frame_h)
                };

                // Add the window frame size that would be used at nextDPI
                {
                    let mut rect = RECT {
                        left: 0,
                        top: 0,
                        right: query_client_w_win,
                        bottom: query_client_h_win,
                    };

                    if !flags.contains(WindowFlags::BORDERLESS) && !is_popup(window) {
                        let _ = adjust_window_rect_for_hwnd(hwnd, &mut rect, next_dpi);
                    }

                    // This is supposed to control the suggested rect param of WM_DPICHANGED
                    size_in_out.cx = rect.right - rect.left;
                    size_in_out.cy = rect.bottom - rect.top;
                }

                return 1;
            }
        }

        WM_DPICHANGED => {
            // Windows 8.1+
            let new_dpi = hiword(w_param) as u32;
            // SAFETY: for WM_DPICHANGED, lParam points at the suggested RECT.
            let suggested_rect = unsafe { *(l_param as *const RECT) };

            if data.state.with(|s| s.expected_resize) {
                /* This DPI change is coming from an explicit SetWindowPos call within SDL.
                Assume all call sites are calculating the DPI-aware frame correctly, so
                we don't need to do any further adjustment. */
                return 0;
            }

            // Interactive user-initiated resizing/movement
            let (w, h) = {
                /* Calculate the new frame w/h such that
                the client area size is maintained. */
                let (ww, wh) =
                    core::with_window(window, |w| (w.core.w, w.core.h)).unwrap_or((0, 0));
                let mut rect = RECT {
                    left: 0,
                    top: 0,
                    right: ww,
                    bottom: wh,
                };

                if !window_flags(window).contains(WindowFlags::BORDERLESS) {
                    let _ = adjust_window_rect_for_hwnd(hwnd, &mut rect, new_dpi);
                }

                (rect.right - rect.left, rect.bottom - rect.top)
            };

            data.state.with(|s| s.expected_resize = true);
            // SAFETY: hwnd is the window's handle.
            unsafe {
                SetWindowPos(
                    hwnd,
                    std::ptr::null_mut(),
                    suggested_rect.left,
                    suggested_rect.top,
                    w,
                    h,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                )
            };
            data.state.with(|s| s.expected_resize = false);
            return 0;
        }

        WM_SETTINGCHANGE => {
            if w_param == 0 && l_param != 0 {
                // SAFETY: for WM_SETTINGCHANGE, a non-zero lParam is a
                // NUL-terminated string.
                let name = unsafe {
                    let p = l_param as *const u16;
                    let mut len = 0;
                    while *p.add(len) != 0 {
                        len += 1;
                    }
                    std::slice::from_raw_parts(p, len)
                };
                if name == utf8_to_wide("ImmersiveColorSet").split_last().unwrap().1 {
                    core::set_system_theme(get_system_theme());
                    update_dark_mode_for_hwnd(hwnd);
                }
            }
            if w_param == SPI_SETMOUSE as usize || w_param == SPI_SETMOUSESPEED as usize {
                winmouse::update_mouse_system_scale();
            }
            if w_param == SPI_SETWORKAREA as usize {
                modes::update_display_usable_bounds();
            }
        }

        _ => {}
    }

    // SAFETY: as the caller promises.
    unsafe { finish(videodata, data, hwnd, msg, w_param, l_param, return_code) }
}

/// The end of `WIN_WindowProc()`: the taskbar message, and the default or
/// original window procedure.
///
/// # Safety
///
/// The message must be valid for `hwnd`.
unsafe fn finish(
    videodata: &VideoData,
    data: &WindowData,
    hwnd: HWND,
    msg: u32,
    w_param: WPARAM,
    l_param: LPARAM,
    return_code: LRESULT,
) -> LRESULT {
    let taskbar_button_created = videodata.state.with(|s| s.wm_taskbar_button_created);
    if msg != 0 && msg == taskbar_button_created {
        data.state.with(|s| s.taskbar_button_created = true);
        let _ = super::window::apply_window_progress(videodata, data.window);
    }

    // If there's a window proc, assume it's going to handle messages
    if let Some(wndproc) = data.state.with(|s| s.wndproc) {
        // SAFETY: the original window procedure of the external window.
        unsafe { CallWindowProcW(Some(wndproc), hwnd, msg, w_param, l_param) }
    } else if return_code >= 0 {
        return_code
    } else {
        // SAFETY: as the caller promises.
        unsafe { def_window_proc(hwnd, msg, w_param, l_param) }
    }
}

/// Wait for a message (or the timeout; `None` waits forever): 1 got one,
/// 0 timed out, -1 can't wait. Translation of `WIN_WaitEventTimeout()`.
pub(crate) fn wait_event_timeout(timeout: Option<Duration>) -> i32 {
    if G_WINDOWS_ENABLE_MESSAGE_LOOP.load(Ordering::Relaxed) {
        let timeout = match timeout {
            None => INFINITE,
            Some(t) => t.as_millis().min(u32::MAX as u128 - 1) as u32,
        };
        // SAFETY: no handles; wakes on any input.
        let ret =
            unsafe { MsgWaitForMultipleObjects(0, std::ptr::null(), 0, timeout, QS_ALLINPUT) };
        if ret == windows_sys::Win32::Foundation::WAIT_OBJECT_0 {
            1
        } else {
            0
        }
    } else {
        // Fail the wait so the caller falls back to polling
        -1
    }
}

/// Translation of `WIN_SendWakeupEvent()`.
pub(crate) fn send_wakeup_event(videodata: &VideoData, window: WindowID) {
    if let Some(data) = window_data(window) {
        let wakeup = videodata.state.with(|s| s.sdl_wakeup);
        // SAFETY: hwnd is the window's handle.
        unsafe { PostMessageW(data.hwnd, wakeup, 0, 0) };
    }
}

/// Simplified event pump for using when creating and destroying windows.
/// Translation of `WIN_PumpEventsForHWND()`.
pub(crate) fn pump_events_for_hwnd(hwnd: HWND) {
    if G_WINDOWS_ENABLE_MESSAGE_LOOP.load(Ordering::Relaxed) {
        PROCESSING_MESSAGES.store(true, Ordering::Relaxed);

        // SAFETY: msg is a valid MSG; the messages are this window's.
        unsafe {
            let mut msg: MSG = std::mem::zeroed();
            while PeekMessageW(&mut msg, hwnd, 0, 0, PM_REMOVE) != 0 {
                set_message_tick(msg.time);

                // Always translate the message in case it's a non-SDL window (e.g. with Qt integration)
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }

        PROCESSING_MESSAGES.store(false, Ordering::Relaxed);
    }
}

/// Dispatch the pending messages of this thread, then fix up the key state
/// Windows loses and the queued cursor clipping. Translation of
/// `WIN_PumpEvents()`.
pub(crate) fn pump_events(videodata: &VideoData) {
    // We explicitly want to use GetTickCount(), not GetTickCount64()
    // SAFETY: GetTickCount has no preconditions.
    let end_ticks = unsafe { GetTickCount() }.wrapping_add(1);
    let mut new_messages = 0;

    if super::gameinput::has_game_input(videodata) {
        super::gameinput::update_game_input(videodata);
    }

    if G_WINDOWS_ENABLE_MESSAGE_LOOP.load(Ordering::Relaxed) {
        PROCESSING_MESSAGES.store(true, Ordering::Relaxed);

        // SAFETY: msg is a valid MSG.
        let mut msg: MSG = unsafe { std::mem::zeroed() };
        // SAFETY: as above.
        while unsafe { PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) } != 0 {
            if let Some(hook) = windows_message_hook() {
                if !hook(&mut msg) {
                    continue;
                }
            }

            // Don't dispatch any mouse motion queued prior to or including the last mouse warp
            let last_warp_time = winmouse::LAST_WARP_TIME.load(Ordering::Relaxed);
            if msg.message == WM_MOUSEMOVE && last_warp_time != 0 {
                if !ticks_passed(msg.time, last_warp_time.wrapping_add(1)) {
                    continue;
                }

                // This mouse message happened after the warp
                winmouse::LAST_WARP_TIME.store(0, Ordering::Relaxed);
            }

            set_message_tick(msg.time);

            // Always translate the message in case it's a non-SDL window (e.g. with Qt integration)
            // SAFETY: msg came from PeekMessageW.
            unsafe {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }

            // Make sure we don't busy loop here forever if there are lots of events coming in
            if ticks_passed(msg.time, end_ticks) {
                /* We might get a few new messages generated by the Steam overlay or other application hooks
                  In this case those messages will be processed before any pending input, so we want to continue after those messages.
                  (thanks to Peter Deayton for his investigation here)
                */
                const MAX_NEW_MESSAGES: i32 = 3;
                new_messages += 1;
                if new_messages > MAX_NEW_MESSAGES {
                    break;
                }
            }
        }

        PROCESSING_MESSAGES.store(false, Ordering::Relaxed);
    }

    /* Windows loses a shift KEYUP event when you have both pressed at once and let go of one.
    You won't get a KEYUP until both are released, and that keyup will only be for the second
    key you released. Take heroic measures and check the keystate as of the last handled event,
    and if we think a key is pressed when Windows doesn't, unstick it in SDL's state. */
    // SAFETY: GetKeyState takes a key code.
    let key_up = |vk: u16| (unsafe { GetKeyState(vk as i32) } as u16 & 0x8000) == 0;
    if sdl_keyboard::is_pressed(Scancode::LSHIFT) && key_up(VK_LSHIFT) {
        sdl_keyboard::send_keyboard_key(
            Duration::ZERO,
            GLOBAL_KEYBOARD_ID,
            0,
            Scancode::LSHIFT,
            false,
        );
    }
    if sdl_keyboard::is_pressed(Scancode::RSHIFT) && key_up(VK_RSHIFT) {
        sdl_keyboard::send_keyboard_key(
            Duration::ZERO,
            GLOBAL_KEYBOARD_ID,
            0,
            Scancode::RSHIFT,
            false,
        );
    }

    /* The Windows key state gets lost when using Windows+Space or Windows+G shortcuts and
    not grabbing the keyboard. Note: If we *are* grabbing the keyboard, GetKeyState()
    will return inaccurate results for VK_LWIN and VK_RWIN but we don't need it anyway. */
    let focus_window = sdl_keyboard::keyboard_focus();
    if !focus_window.is_some_and(|w| window_flags(w).contains(WindowFlags::KEYBOARD_GRABBED)) {
        if sdl_keyboard::is_pressed(Scancode::LGUI) && key_up(VK_LWIN) {
            sdl_keyboard::send_keyboard_key(
                Duration::ZERO,
                GLOBAL_KEYBOARD_ID,
                0,
                Scancode::LGUI,
                false,
            );
        }
        if sdl_keyboard::is_pressed(Scancode::RGUI) && key_up(VK_RWIN) {
            sdl_keyboard::send_keyboard_key(
                Duration::ZERO,
                GLOBAL_KEYBOARD_ID,
                0,
                Scancode::RGUI,
                false,
            );
        }
    }

    // fire queued clipcursor refreshes
    for window in core::window_ids() {
        let mut refresh_clipcursor = false;
        if let Some(data) = window_data(window) {
            refresh_clipcursor = data.state.with(|s| {
                let queued = s.clipcursor_queued;
                s.clipcursor_queued = false; // Must be cleared unconditionally.
                s.postpone_clipcursor = false; // Must be cleared unconditionally.
                                               // Must happen before UpdateClipCursor.
                                               // Although its occurrence currently
                                               // always coincides with the queuing of
                                               // clipcursor, it is logically distinct
                                               // and this coincidence might no longer
                                               // be true in the future.
                                               // Ergo this placement concordantly
                                               // conveys its unconditionality
                                               // vis-a-vis the queuing of clipcursor.
                queued
            });
        }
        if refresh_clipcursor {
            update_clip_cursor(window);
        }
    }

    // Synchronize internal mouse capture state to the most current cursor state
    // since for whatever reason we are not depending exclusively on SetCapture/
    // ReleaseCapture to pipe in out-of-window mouse events.
    // Formerly WIN_UpdateMouseCapture().
    // TODO: can this go before clipcursor?
    let focus_window = sdl_keyboard::keyboard_focus();
    if let Some(focus_window) =
        focus_window.filter(|&w| window_flags(w).contains(WindowFlags::MOUSE_CAPTURE))
    {
        if let Some(data) = window_data(focus_window) {
            if !data.state.with(|s| s.mouse_tracked) {
                let mut cursor_pos = POINT::default();

                // SAFETY: cursor_pos is a valid POINT.
                if unsafe { GetCursorPos(&mut cursor_pos) } != 0
                    && unsafe { ScreenToClient(data.hwnd, &mut cursor_pos) } != 0
                {
                    let swap = swap_buttons();
                    let mouse_id = GLOBAL_MOUSE_ID;
                    let w = Some(data.window);
                    let down = |vk: u16| (async_key_state(vk) as u16 & 0x8000) != 0;

                    mouse::send_mouse_motion(
                        get_event_timestamp(),
                        w,
                        mouse_id,
                        false,
                        cursor_pos.x as f32,
                        cursor_pos.y as f32,
                    );
                    mouse::send_mouse_button(
                        get_event_timestamp(),
                        w,
                        mouse_id,
                        if !swap { BUTTON_LEFT } else { BUTTON_RIGHT },
                        down(VK_LBUTTON),
                    );
                    mouse::send_mouse_button(
                        get_event_timestamp(),
                        w,
                        mouse_id,
                        if !swap { BUTTON_RIGHT } else { BUTTON_LEFT },
                        down(VK_RBUTTON),
                    );
                    mouse::send_mouse_button(
                        get_event_timestamp(),
                        w,
                        mouse_id,
                        BUTTON_MIDDLE,
                        down(VK_MBUTTON),
                    );
                    mouse::send_mouse_button(
                        get_event_timestamp(),
                        w,
                        mouse_id,
                        BUTTON_X1,
                        down(VK_XBUTTON1),
                    );
                    mouse::send_mouse_button(
                        get_event_timestamp(),
                        w,
                        mouse_id,
                        BUTTON_X2,
                        down(VK_XBUTTON2),
                    );
                }
            }
        }
    }

    keyboard::update_ime_candidates(videodata);
}

/// The application's window class. Translation of `app_registered`,
/// `SDL_Appname`, `SDL_Appstyle` and `SDL_Instance`.
struct App {
    registered: i32,
    name: Option<Vec<u16>>,
    style: u32,
    instance: HINSTANCE,
}

// SAFETY: the instance handle is a process-wide token.
unsafe impl Send for App {}

static APP: Mutex<App> = Mutex::new(App {
    registered: 0,
    name: None,
    style: 0,
    instance: std::ptr::null_mut(),
});

fn app() -> std::sync::MutexGuard<'static, App> {
    APP.lock().unwrap_or_else(|e| e.into_inner())
}

/// The window class name (`SDL_Appname`), NUL-terminated.
pub(crate) fn app_name() -> Vec<u16> {
    app().name.clone().unwrap_or_else(|| vec![0])
}

/// The instance the class is registered for (`SDL_Instance`).
pub(crate) fn app_instance() -> HINSTANCE {
    app().instance
}

/// Translation of `WIN_CleanRegisterApp()`.
fn clean_register_app(app: &mut App, wcex: &WNDCLASSEXW) {
    // SAFETY: the icons, if any, were loaded for the class.
    unsafe {
        if !wcex.hIcon.is_null() {
            DestroyIcon(wcex.hIcon);
        }
        if !wcex.hIconSm.is_null() {
            DestroyIcon(wcex.hIconSm);
        }
    }
    app.name = None;
}

/// Translation of `WIN_ResourceNameCallback()`.
unsafe extern "system" fn resource_name_callback(
    h_module: windows_sys::Win32::Foundation::HMODULE,
    _lp_type: windows_sys::core::PCWSTR,
    lp_name: windows_sys::core::PCWSTR,
    l_param: isize,
) -> i32 {
    // SAFETY: l_param is the WNDCLASSEXW register_app() passed.
    let wcex = unsafe { &mut *(l_param as *mut WNDCLASSEXW) };

    // (void)lpType; // We already know that the resource type is RT_GROUP_ICON.

    /* We leave hIconSm as NULL as it will allow Windows to automatically
    choose the appropriate small icon size to suit the current DPI. */
    // SAFETY: lp_name names an icon resource of h_module.
    wcex.hIcon = unsafe { LoadIconW(h_module, lp_name) };

    // Do not bother enumerating any more.
    0
}

/// Register the window class for this application (counted).
/// Translation of `SDL_RegisterApp()`.
pub fn register_app(name: Option<&str>, style: u32, h_inst: Option<HINSTANCE>) -> Result<()> {
    let mut app = app();

    // Only do this once...
    if app.registered != 0 {
        app.registered += 1;
        return Ok(());
    }
    crate::sdl_assert!(app.name.is_none());
    let (name, style) = match name {
        Some(name) => (name, style),
        None => ("SDL_app", CS_BYTEALIGNCLIENT | CS_OWNDC),
    };
    app.name = Some(utf8_to_wide(name));
    app.style = style;
    // SAFETY: GetModuleHandleW(NULL) is the executable.
    app.instance = h_inst.unwrap_or_else(|| unsafe { GetModuleHandleW(std::ptr::null()) });

    // Register the application class
    // SAFETY: plain data; all zeroes is valid.
    let mut wcex: WNDCLASSEXW = unsafe { std::mem::zeroed() };
    wcex.cbSize = size_of::<WNDCLASSEXW>() as u32;
    wcex.lpszClassName = app.name.as_ref().map_or(std::ptr::null(), |n| n.as_ptr());
    wcex.style = app.style;
    wcex.lpfnWndProc = Some(window_proc);
    wcex.hInstance = app.instance;

    match hints::get(hints::WINDOWS_INTRESOURCE_ICON).filter(|h| !h.is_empty()) {
        Some(hint) => {
            let id = crate::stdlib::atoi(&hint) as u16;
            // SAFETY: MAKEINTRESOURCE(id).
            wcex.hIcon = unsafe { LoadIconW(app.instance, id as usize as *const u16) };

            if let Some(hint) =
                hints::get(hints::WINDOWS_INTRESOURCE_ICON_SMALL).filter(|h| !h.is_empty())
            {
                let id = crate::stdlib::atoi(&hint) as u16;
                // SAFETY: as above.
                wcex.hIconSm = unsafe { LoadIconW(app.instance, id as usize as *const u16) };
            }
        }
        None => {
            // Use the first icon as a default icon, like in the Explorer.
            // SAFETY: the callback gets a pointer to wcex, alive for the call.
            unsafe {
                EnumResourceNamesW(
                    app.instance,
                    RT_GROUP_ICON,
                    Some(resource_name_callback),
                    &mut wcex as *mut WNDCLASSEXW as isize,
                )
            };
        }
    }

    // SAFETY: wcex is a complete class description whose strings outlive it.
    if unsafe { RegisterClassExW(&wcex) } == 0 {
        clean_register_app(&mut app, &wcex);
        return Err(Error::new("Couldn't register application class"));
    }

    app.registered = 1;
    Ok(())
}

/// Unregisters the windowclass registered in [`register_app`] (when the
/// last registration goes). Translation of `SDL_UnregisterApp()`.
pub fn unregister_app() {
    let mut app = app();

    // SDL_RegisterApp might not have been called before
    if app.registered == 0 {
        return;
    }
    app.registered -= 1;
    if app.registered == 0 {
        // Ensure the icons are initialized.
        // SAFETY: plain data; all zeroes is valid.
        let mut wcex: WNDCLASSEXW = unsafe { std::mem::zeroed() };
        // FIXME (upstream): wcex.cbSize is never set before GetClassInfoEx(), so the call can fail and leave the class registered.
        // Check for any registered window classes.
        let name = app.name.clone().unwrap_or_else(|| vec![0]);
        // SAFETY: name is NUL-terminated; wcex receives the class.
        unsafe {
            if GetClassInfoExW(app.instance, name.as_ptr(), &mut wcex) != 0 {
                UnregisterClassW(name.as_ptr(), app.instance);
            }
        }
        clean_register_app(&mut app, &wcex);
    }
}
