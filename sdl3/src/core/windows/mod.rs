// Rust translation of src/core/windows/SDL_windows.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Windows glue shared by the backends: errors from `GetLastError()` and
//! `HRESULT`s, COM/WinRT initialization, OS version checks, string
//! conversion, the DirectInput helper window, icons from surfaces and the
//! audio device name lookup.
//!
//! `WIN_CheckDefaultArgcArgv()` has no counterpart: Rust programs get their
//! arguments from `std::env::args()`, already converted from the wide
//! command line.

#![allow(clippy::upper_case_acronyms)]

pub(crate) mod immdevice;

use crate::audio::AudioFormat;
use crate::error::{Error, Result};
use crate::video::pixels::PixelFormat;
use crate::video::rect::Rect;
use crate::video::surface::Surface;
use std::ffi::c_void;
use std::sync::{Mutex, OnceLock};
use windows_sys::core::{GUID, HRESULT};
use windows_sys::Win32::Foundation::{
    FreeLibrary, GetLastError, ERROR_CLASS_ALREADY_EXISTS, ERROR_SUCCESS, E_NOINTERFACE, HMODULE,
    HWND, RECT, RPC_E_CHANGED_MODE, S_FALSE, S_OK,
};
use windows_sys::Win32::Globalization::{WideCharToMultiByte, WC_ERR_INVALID_CHARS};
use windows_sys::Win32::Graphics::Gdi::{
    CreateBitmap, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC,
    SelectObject, SetPixel, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
};
use windows_sys::Win32::Media::Audio::WAVEFORMATEX;
use windows_sys::Win32::System::Com::{
    CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED, COINIT_MULTITHREADED,
};
use windows_sys::Win32::System::Diagnostics::Debug::{FormatMessageW, FORMAT_MESSAGE_FROM_SYSTEM};
use windows_sys::Win32::System::LibraryLoader::{
    GetModuleFileNameW, GetModuleHandleW, GetProcAddress, LoadLibraryExW, LoadLibraryW,
    LOAD_LIBRARY_SEARCH_SYSTEM32,
};
use windows_sys::Win32::System::Registry::{
    RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_LOCAL_MACHINE, KEY_QUERY_VALUE,
};
use windows_sys::Win32::System::SystemInformation::{
    VerSetConditionMask, VerifyVersionInfoW, OSVERSIONINFOEXW, VER_MAJORVERSION, VER_MINORVERSION,
    VER_SERVICEPACKMAJOR,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateIconIndirect, CreateWindowExW, DefWindowProcW, DestroyWindow, RegisterClassW, SetPropW,
    UnregisterClassW, CW_USEDEFAULT, HICON, HWND_MESSAGE, ICONINFO, WNDCLASSW, WS_OVERLAPPED,
};

pub(crate) mod hid;
pub(crate) mod xinput;

const VER_GREATER_EQUAL: u8 = 3;

/// What `GetProcAddress()` returns, before the cast to the real signature.
type RawProc = unsafe extern "system" fn() -> isize;

/// UTF-8 to NUL-terminated UTF-16 (`WIN_UTF8ToStringW()`).
pub(crate) fn utf8_to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// UTF-16, up to the first NUL if there is one, to UTF-8
/// (`WIN_StringToUTF8W()`). Unpaired surrogates become U+FFFD.
pub(crate) fn wide_to_utf8(w: &[u16]) -> String {
    let end = w.iter().position(|&c| c == 0).unwrap_or(w.len());
    String::from_utf16_lossy(&w[..end])
}

/// An error from an `HRESULT` or Win32 error code, as
/// "`prefix`: `system message`". Translation of `WIN_SetErrorFromHRESULT()`.
pub(crate) fn error_from_hresult(prefix: Option<&str>, hr: HRESULT) -> Error {
    let mut buffer = [0u16; 1024];
    // SAFETY: buffer holds the given number of UTF-16 units; no source or
    // arguments are used with FORMAT_MESSAGE_FROM_SYSTEM alone.
    let c = unsafe {
        FormatMessageW(
            FORMAT_MESSAGE_FROM_SYSTEM,
            std::ptr::null(),
            hr as u32,
            0,
            buffer.as_mut_ptr(),
            buffer.len() as u32,
            std::ptr::null(),
        )
    } as usize;
    let text = &buffer[..c.min(buffer.len() - 1)];
    // kill CR/LF that FormatMessage() sticks at the end
    let end = text
        .iter()
        .position(|&ch| ch == u16::from(b'\r'))
        .unwrap_or(text.len());
    let message = wide_to_utf8(&text[..end]);
    match prefix {
        Some(prefix) => Error::new(format!("{prefix}: {message}")),
        None => Error::new(message),
    }
}

/// An error from `GetLastError()`. Translation of `WIN_SetError()`.
pub(crate) fn set_error(prefix: &str) -> Error {
    // SAFETY: GetLastError has no preconditions.
    let code = unsafe { GetLastError() };
    error_from_hresult(Some(prefix), code as HRESULT)
}

/// Initialize COM on this thread, apartment-threaded if possible, else
/// multithreaded. `S_FALSE` (already initialized) counts as success; every
/// success still needs a [`co_uninitialize`]. Translation of
/// `WIN_CoInitialize()`.
pub(crate) fn co_initialize() -> HRESULT {
    /* SDL handles any threading model, so initialize with the default, which
       is compatible with OLE and if that doesn't work, try multi-threaded mode.

       If you need multi-threaded mode, call CoInitializeEx() before SDL_Init()
    */
    // SAFETY: CoInitializeEx takes a reserved NULL and a mode flag.
    let mut hr = unsafe { CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32) };
    if hr == RPC_E_CHANGED_MODE {
        // SAFETY: as above.
        hr = unsafe { CoInitializeEx(std::ptr::null(), COINIT_MULTITHREADED as u32) };
    }

    // S_FALSE means success, but someone else already initialized.
    // You still need to call CoUninitialize in this case!
    if hr == S_FALSE {
        return S_OK;
    }
    hr
}

/// Translation of `WIN_CoUninitialize()`.
pub(crate) fn co_uninitialize() {
    // SAFETY: balances a successful co_initialize() on this thread.
    unsafe { CoUninitialize() }
}

struct Module(HMODULE);
// SAFETY: a module handle is a process-wide token, usable from any thread.
unsafe impl Send for Module {}
// SAFETY: as above.
unsafe impl Sync for Module {}

/// A function exported by combase.dll, loaded once from System32.
/// Translation of `WIN_LoadComBaseFunction()`.
fn load_combase_function(name: &std::ffi::CStr) -> Option<unsafe extern "system" fn() -> isize> {
    static COMBASE: OnceLock<Module> = OnceLock::new();
    let combase = COMBASE.get_or_init(|| {
        let name = utf8_to_wide("combase.dll");
        // SAFETY: name is NUL-terminated; no file handle is passed.
        Module(unsafe {
            LoadLibraryExW(
                name.as_ptr(),
                std::ptr::null_mut(),
                LOAD_LIBRARY_SEARCH_SYSTEM32,
            )
        })
    });
    if combase.0.is_null() {
        return None;
    }
    // SAFETY: the module stays loaded for the life of the process.
    unsafe { GetProcAddress(combase.0, name.as_ptr().cast()) }
}

/// Initialize the Windows Runtime on this thread (single-threaded if
/// possible). Translation of `WIN_RoInitialize()`.
pub(crate) fn ro_initialize() -> HRESULT {
    type RoInitializeFn = unsafe extern "system" fn(i32) -> HRESULT;
    const RO_INIT_SINGLETHREADED: i32 = 0;
    const RO_INIT_MULTITHREADED: i32 = 1;
    let Some(f) = load_combase_function(c"RoInitialize") else {
        return E_NOINTERFACE;
    };
    // SAFETY: RoInitialize has this signature.
    let ro_initialize: RoInitializeFn = unsafe { std::mem::transmute(f) };
    // RO_INIT_SINGLETHREADED is equivalent to COINIT_APARTMENTTHREADED
    // SAFETY: RoInitialize takes a mode flag.
    let mut hr = unsafe { ro_initialize(RO_INIT_SINGLETHREADED) };
    if hr == RPC_E_CHANGED_MODE {
        // SAFETY: as above.
        hr = unsafe { ro_initialize(RO_INIT_MULTITHREADED) };
    }

    // S_FALSE means success, but someone else already initialized.
    // You still need to call RoUninitialize in this case!
    if hr == S_FALSE {
        return S_OK;
    }
    hr
}

/// Translation of `WIN_RoUninitialize()`.
pub(crate) fn ro_uninitialize() {
    type RoUninitializeFn = unsafe extern "system" fn();
    if let Some(f) = load_combase_function(c"RoUninitialize") {
        // SAFETY: RoUninitialize takes no arguments; it balances a
        // successful ro_initialize() on this thread.
        unsafe {
            let ro_uninitialize: RoUninitializeFn = std::mem::transmute(f);
            ro_uninitialize();
        }
    }
}

/// Translation of `IsWindowsVersionOrGreater()` from versionhelpers.h.
fn is_windows_version_or_greater(major: u32, minor: u32, service_pack_major: u16) -> bool {
    // SAFETY: VerSetConditionMask is a pure function of its arguments.
    let condition_mask = unsafe {
        VerSetConditionMask(
            VerSetConditionMask(
                VerSetConditionMask(0, VER_MAJORVERSION, VER_GREATER_EQUAL),
                VER_MINORVERSION,
                VER_GREATER_EQUAL,
            ),
            VER_SERVICEPACKMAJOR,
            VER_GREATER_EQUAL,
        )
    };
    // SAFETY: OSVERSIONINFOEXW is plain data; all zeroes is valid.
    let mut osvi: OSVERSIONINFOEXW = unsafe { std::mem::zeroed() };
    osvi.dwOSVersionInfoSize = size_of::<OSVERSIONINFOEXW>() as u32;
    osvi.dwMajorVersion = major;
    osvi.dwMinorVersion = minor;
    osvi.wServicePackMajor = service_pack_major;
    // SAFETY: osvi is a valid, sized OSVERSIONINFOEXW.
    unsafe {
        VerifyVersionInfoW(
            &mut osvi,
            VER_MAJORVERSION | VER_MINORVERSION | VER_SERVICEPACKMAJOR,
            condition_mask,
        ) != 0
    }
}

/// The Windows build number, from `RtlGetVersion()` (0 if unknown).
fn windows_build_number() -> u32 {
    static BUILD: OnceLock<u32> = OnceLock::new();
    *BUILD.get_or_init(|| {
        #[repr(C)]
        struct NtOsVersionInfoW {
            os_version_info_size: u32,
            major_version: u32,
            minor_version: u32,
            build_number: u32,
            platform_id: u32,
            csd_version: [u16; 128],
        }
        type RtlGetVersionFn = unsafe extern "system" fn(*mut NtOsVersionInfoW);
        let name = utf8_to_wide("ntdll.dll");
        // SAFETY: name is NUL-terminated.
        let ntdll = unsafe { LoadLibraryW(name.as_ptr()) };
        if ntdll.is_null() {
            return 0;
        }
        // There is no function to get Windows build number, so let's get it here via RtlGetVersion
        let mut os_info = NtOsVersionInfoW {
            os_version_info_size: size_of::<NtOsVersionInfoW>() as u32,
            major_version: 0,
            minor_version: 0,
            build_number: 0,
            platform_id: 0,
            csd_version: [0; 128],
        };
        // SAFETY: ntdll is loaded; RtlGetVersion has this signature and
        // fills the sized struct.
        unsafe {
            if let Some(f) = GetProcAddress(ntdll, c"RtlGetVersion".as_ptr().cast()) {
                let rtl_get_version: RtlGetVersionFn = std::mem::transmute(f);
                rtl_get_version(&mut os_info);
            }
            FreeLibrary(ntdll);
        }
        os_info.build_number & !0xF000_0000
    })
}

/// Translation of `IsWindowsBuildVersionAtLeast()`.
fn is_windows_build_version_at_least(build_number: u32) -> bool {
    windows_build_number() >= build_number
}

/// Whether the process runs under Wine (ntdll exports `wine_get_version`).
/// Translation of `WIN_IsWine()`.
pub(crate) fn is_wine() -> bool {
    static IS_WINE: OnceLock<bool> = OnceLock::new();
    *IS_WINE.get_or_init(|| {
        let name = utf8_to_wide("ntdll.dll");
        // SAFETY: name is NUL-terminated; the module is freed after the lookup.
        unsafe {
            let ntdll = LoadLibraryW(name.as_ptr());
            if ntdll.is_null() {
                return false;
            }
            let found = GetProcAddress(ntdll, c"wine_get_version".as_ptr().cast()).is_some();
            FreeLibrary(ntdll);
            found
        }
    })
}

macro_rules! check_win_ver {
    ($(#[$m:meta])* $name:ident, $test:expr) => {
        $(#[$m])*
        pub(crate) fn $name() -> bool {
            static RESULT: OnceLock<bool> = OnceLock::new();
            *RESULT.get_or_init(|| $test)
        }
    };
}

check_win_ver!(
    /// Windows XP, the oldest version SDL runs on. Translation of `WIN_IsWindowsXP()`.
    is_windows_xp,
    !is_windows_vista_or_greater() && is_windows_version_or_greater(5, 1, 0)
);
check_win_ver!(
    /// Translation of `WIN_IsWindowsVistaOrGreater()`.
    is_windows_vista_or_greater,
    is_windows_version_or_greater(6, 0, 0)
);
check_win_ver!(
    /// Translation of `WIN_IsWindows7OrGreater()`.
    is_windows_7_or_greater,
    is_windows_version_or_greater(6, 1, 0)
);
check_win_ver!(
    /// Translation of `WIN_IsWindows8OrGreater()`.
    is_windows_8_or_greater,
    is_windows_version_or_greater(6, 2, 0)
);
check_win_ver!(
    /// Translation of `WIN_IsWindows81OrGreater()`.
    is_windows_81_or_greater,
    is_windows_version_or_greater(6, 3, 0)
);
check_win_ver!(
    /// Translation of `WIN_IsWindows10OrGreater()`.
    is_windows_10_or_greater,
    is_windows_version_or_greater(10, 0, 0)
);

/// Translation of `WIN_IsWindows11OrGreater()`.
pub(crate) fn is_windows_11_or_greater() -> bool {
    is_windows_build_version_at_least(22000)
}

/// The full name of an audio endpoint from the registry
/// (`HKLM\System\CurrentControlSet\Control\MediaCategories\{guid}\Name`),
/// falling back to `name`. Translation of `WIN_LookupAudioDeviceName()`.
///
/// WAVExxxCAPS truncates device names to 31 characters; since WinXP the name
/// GUID of WAVExxxCAPS2 points into the registry, where the full name is.
/// Drivers can report GUID_NULL, in which case Windows makes a best effort in
/// the usual place. WASAPI doesn't need this; DirectSound and WinMM do.
pub(crate) fn lookup_audio_device_name(name: &[u16], guid: &GUID) -> String {
    let fallback = || wide_to_utf8(name);
    let g = guid_bytes(guid);
    if g == [0; 16] {
        return fallback(); // No GUID, go with what we've got.
    }
    let keystr = format!(
        "System\\CurrentControlSet\\Control\\MediaCategories\\{{{:02X}{:02X}{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}}}",
        g[3], g[2], g[1], g[0], g[5], g[4], g[7], g[6], g[8], g[9], g[10], g[11], g[12], g[13], g[14], g[15]
    );
    let strw = utf8_to_wide(&keystr);
    let mut hkey: HKEY = std::ptr::null_mut();
    // SAFETY: strw is NUL-terminated; hkey receives the opened key.
    if unsafe {
        RegOpenKeyExW(
            HKEY_LOCAL_MACHINE,
            strw.as_ptr(),
            0,
            KEY_QUERY_VALUE,
            &mut hkey,
        )
    } != ERROR_SUCCESS
    {
        return fallback(); // oh well.
    }
    let value = utf8_to_wide("Name");
    let mut len: u32 = 0;
    // SAFETY: hkey is open; a NULL data pointer asks for the size only.
    let rc = unsafe {
        RegQueryValueExW(
            hkey,
            value.as_ptr(),
            std::ptr::null(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut len,
        )
    };
    if rc != ERROR_SUCCESS {
        // SAFETY: hkey is open and closed once.
        unsafe { RegCloseKey(hkey) };
        return fallback(); // oh well.
    }
    let mut strw = vec![0u16; len as usize / 2 + 1];
    // SAFETY: strw has room for len bytes plus a terminator.
    let rc = unsafe {
        RegQueryValueExW(
            hkey,
            value.as_ptr(),
            std::ptr::null(),
            std::ptr::null_mut(),
            strw.as_mut_ptr().cast(),
            &mut len,
        )
    };
    // SAFETY: hkey is open and closed once.
    unsafe { RegCloseKey(hkey) };
    if rc != ERROR_SUCCESS {
        return fallback(); // oh well.
    }
    strw.truncate(len as usize / 2); // make sure it's null-terminated.
    wide_to_utf8(&strw)
}

/// A GUID's bytes in memory order.
pub(crate) fn guid_bytes(g: &GUID) -> [u8; 16] {
    let mut b = [0u8; 16];
    b[0..4].copy_from_slice(&g.data1.to_le_bytes());
    b[4..6].copy_from_slice(&g.data2.to_le_bytes());
    b[6..8].copy_from_slice(&g.data3.to_le_bytes());
    b[8..16].copy_from_slice(&g.data4);
    b
}

/// Translation of `WIN_IsEqualGUID()` and `WIN_IsEqualIID()`.
pub(crate) fn is_equal_guid(a: &GUID, b: &GUID) -> bool {
    guid_bytes(a) == guid_bytes(b)
}

/// Translation of `WIN_RECTToRect()` (inclusive right/bottom).
pub(crate) fn rect_from_win(winrect: &RECT) -> Rect {
    Rect::new(
        winrect.left,
        winrect.top,
        (winrect.right - winrect.left) + 1,
        (winrect.bottom - winrect.top) + 1,
    )
}

/// Translation of `WIN_RectToRECT()`.
pub(crate) fn rect_to_win(sdlrect: &Rect) -> RECT {
    RECT {
        left: sdlrect.x,
        right: sdlrect.x + sdlrect.w - 1,
        top: sdlrect.y,
        bottom: sdlrect.y + sdlrect.h - 1,
    }
}

/// Translation of `WIN_WindowRectValid()`.
pub(crate) fn window_rect_valid(rect: &RECT) -> bool {
    // A window can be resized to zero height, but not zero width
    rect.right > 0
}

/// Opt a window into the dark title bar when the system uses dark mode,
/// through uxtheme's unnamed exports. Translation of
/// `WIN_UpdateDarkModeForHWND()`.
pub(crate) fn update_dark_mode_for_hwnd(hwnd: HWND) {
    type ShouldAppsUseDarkModeFn = unsafe extern "system" fn() -> bool;
    type AllowDarkModeForWindowFn = unsafe extern "system" fn(HWND, bool);
    type AllowDarkModeForAppFn = unsafe extern "system" fn(bool);
    type RefreshImmersiveColorPolicyStateFn = unsafe extern "system" fn();
    type SetPreferredAppModeFn = unsafe extern "system" fn(i32) -> i32;
    #[repr(C)]
    struct WindowCompositionAttribData {
        attrib: i32,
        data: *mut c_void,
        size: usize,
    }
    type SetWindowCompositionAttributeFn =
        unsafe extern "system" fn(HWND, *const WindowCompositionAttribData) -> i32;
    const UXTHEME_APPMODE_ALLOW_DARK: i32 = 1;
    const WCA_USEDARKMODECOLORS: i32 = 26;

    if !is_windows_build_version_at_least(17763) {
        // Too old to support dark mode
        return;
    }
    let name = utf8_to_wide("uxtheme.dll");
    // SAFETY: the ordinals are uxtheme's documented-by-use exports with
    // these signatures; the module is freed after use.
    unsafe {
        let uxtheme = LoadLibraryW(name.as_ptr());
        if uxtheme.is_null() {
            return;
        }
        let ordinal = |n: usize| GetProcAddress(uxtheme, n as *const u8);
        let refresh_immersive_color_policy_state = ordinal(104);
        let should_apps_use_dark_mode = ordinal(132);
        let allow_dark_mode_for_window = ordinal(133);
        if let Some(f) = ordinal(135) {
            if !is_windows_build_version_at_least(18362) {
                std::mem::transmute::<RawProc, AllowDarkModeForAppFn>(f)(true);
            } else {
                std::mem::transmute::<RawProc, SetPreferredAppModeFn>(f)(
                    UXTHEME_APPMODE_ALLOW_DARK,
                );
            }
        }
        if let Some(f) = refresh_immersive_color_policy_state {
            std::mem::transmute::<RawProc, RefreshImmersiveColorPolicyStateFn>(f)();
        }
        if let Some(f) = allow_dark_mode_for_window {
            std::mem::transmute::<RawProc, AllowDarkModeForWindowFn>(f)(hwnd, true);
        }
        // Check dark mode using ShouldAppsUseDarkMode, but use SDL_GetSystemTheme as a fallback
        let mut value: i32 = match should_apps_use_dark_mode {
            Some(f) => std::mem::transmute::<RawProc, ShouldAppsUseDarkModeFn>(f)() as i32,
            None => (crate::video::core::system_theme() == crate::video::SystemTheme::Dark) as i32,
        };
        FreeLibrary(uxtheme);
        if !is_windows_build_version_at_least(18362) {
            let prop = utf8_to_wide("UseImmersiveDarkModeColors");
            SetPropW(hwnd, prop.as_ptr(), value as isize as *mut c_void);
        } else {
            let user32_name = utf8_to_wide("user32.dll");
            let user32 = GetModuleHandleW(user32_name.as_ptr());
            if !user32.is_null() {
                if let Some(f) =
                    GetProcAddress(user32, c"SetWindowCompositionAttribute".as_ptr().cast())
                {
                    let data = WindowCompositionAttribData {
                        attrib: WCA_USEDARKMODECOLORS,
                        data: (&mut value as *mut i32).cast(),
                        size: size_of::<i32>(),
                    };
                    std::mem::transmute::<RawProc, SetWindowCompositionAttributeFn>(f)(hwnd, &data);
                }
            }
        }
    }
}

/// An icon from a surface: the pixels as ARGB8888, with a mask that is set
/// where alpha is zero. Translation of `WIN_CreateIconFromSurface()`.
pub(crate) fn create_icon_from_surface(surface: &Surface<'_>) -> Option<HICON> {
    let s = surface.convert(PixelFormat::ARGB8888).ok()?;
    let width = s.width();
    let height = s.height();
    let pitch = s.pitch() as usize;
    let pixels = s.pixels()?;

    // SAFETY: BITMAPINFO is plain data; all zeroes is valid.
    let mut bmp_info: BITMAPINFO = unsafe { std::mem::zeroed() };
    bmp_info.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
    bmp_info.bmiHeader.biWidth = width;
    bmp_info.bmiHeader.biHeight = -height; /* Top-down bitmap */
    bmp_info.bmiHeader.biPlanes = 1;
    bmp_info.bmiHeader.biBitCount = 32;
    bmp_info.bmiHeader.biCompression = BI_RGB;

    // SAFETY: the GDI objects are created, used and released in order; the
    // DIB section's bits hold width * height * 4 bytes.
    unsafe {
        let hdc = GetDC(std::ptr::null_mut());
        let mut p_bits: *mut c_void = std::ptr::null_mut();
        let h_bitmap = CreateDIBSection(
            hdc,
            &bmp_info,
            DIB_RGB_COLORS,
            &mut p_bits,
            std::ptr::null_mut(),
            0,
        );
        if h_bitmap.is_null() {
            ReleaseDC(std::ptr::null_mut(), hdc);
            return None;
        }
        let row = width as usize * 4;
        let bits = std::slice::from_raw_parts_mut(p_bits.cast::<u8>(), row * height as usize);
        for y in 0..height as usize {
            bits[y * row..(y + 1) * row].copy_from_slice(&pixels[y * pitch..y * pitch + row]);
        }

        let h_mask = CreateBitmap(width, height, 1, 1, std::ptr::null());
        if h_mask.is_null() {
            DeleteObject(h_bitmap);
            ReleaseDC(std::ptr::null_mut(), hdc);
            return None;
        }

        let hdc_mem = CreateCompatibleDC(hdc);
        let old_bitmap = SelectObject(hdc_mem, h_mask);

        for y in 0..height {
            for x in 0..width {
                let alpha = bits[(y as usize * width as usize + x as usize) * 4 + 3];
                let mask_color = if alpha == 0 { 0x00FF_FFFF } else { 0 };
                SetPixel(hdc_mem, x, y, mask_color);
            }
        }

        let icon_info = ICONINFO {
            fIcon: 1,
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: h_mask,
            hbmColor: h_bitmap,
        };
        let h_icon = CreateIconIndirect(&icon_info);

        SelectObject(hdc_mem, old_bitmap);
        DeleteDC(hdc_mem);
        DeleteObject(h_bitmap);
        DeleteObject(h_mask);
        ReleaseDC(std::ptr::null_mut(), hdc);

        (!h_icon.is_null()).then_some(h_icon)
    }
}

// Some GUIDs we need to know without linking to libraries that aren't available before Vista.
const KSDATAFORMAT_SUBTYPE_PCM: GUID = GUID {
    data1: 0x0000_0001,
    data2: 0x0000,
    data3: 0x0010,
    data4: [0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71],
};
const KSDATAFORMAT_SUBTYPE_IEEE_FLOAT: GUID = GUID {
    data1: 0x0000_0003,
    data2: 0x0000,
    data3: 0x0010,
    data4: [0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71],
};
const WAVE_FORMAT_PCM: u16 = 1;
const WAVE_FORMAT_IEEE_FLOAT: u16 = 3;
const WAVE_FORMAT_EXTENSIBLE: u16 = 0xFFFE;

/// The SDL audio format of a `WAVEFORMATEX` (and its `WAVEFORMATEXTENSIBLE`
/// sub-format), or `None` for one SDL can't use directly. Translation of
/// `SDL_WaveFormatExToSDLFormat()`.
///
/// # Safety
///
/// If `waveformat.wFormatTag` is `WAVE_FORMAT_EXTENSIBLE`, `waveformat` must
/// be the head of a whole `WAVEFORMATEXTENSIBLE`.
pub(crate) unsafe fn wave_format_ex_to_sdl_format(
    waveformat: &WAVEFORMATEX,
) -> Option<AudioFormat> {
    let tag = waveformat.wFormatTag;
    let bits = waveformat.wBitsPerSample;
    if tag == WAVE_FORMAT_IEEE_FLOAT && bits == 32 {
        Some(AudioFormat::F32)
    } else if tag == WAVE_FORMAT_PCM && bits == 16 {
        Some(AudioFormat::S16)
    } else if tag == WAVE_FORMAT_PCM && bits == 32 {
        Some(AudioFormat::S32)
    } else if tag == WAVE_FORMAT_EXTENSIBLE {
        // SAFETY: an extensible format is a whole WAVEFORMATEXTENSIBLE (the
        // caller's contract); the struct is packed, so read it unaligned.
        let sub_format = unsafe {
            let ext = (waveformat as *const WAVEFORMATEX)
                .cast::<windows_sys::Win32::Media::Audio::WAVEFORMATEXTENSIBLE>();
            std::ptr::addr_of!((*ext).SubFormat).read_unaligned()
        };
        if is_equal_guid(&sub_format, &KSDATAFORMAT_SUBTYPE_IEEE_FLOAT) && bits == 32 {
            Some(AudioFormat::F32)
        } else if is_equal_guid(&sub_format, &KSDATAFORMAT_SUBTYPE_PCM) && bits == 16 {
            Some(AudioFormat::S16)
        } else if is_equal_guid(&sub_format, &KSDATAFORMAT_SUBTYPE_PCM) && bits == 32 {
            Some(AudioFormat::S32)
        } else {
            None
        }
    } else {
        None
    }
}

/// UTF-16 to a code page with `WideCharToMultiByte()`, dropping
/// `WC_ERR_INVALID_CHARS` on Windows XP, which doesn't support it.
/// Translation of `WIN_WideCharToMultiByte()`.
pub(crate) fn wide_char_to_multi_byte(
    code_page: u32,
    mut flags: u32,
    wide: &[u16],
) -> Option<Vec<u8>> {
    if is_windows_xp() {
        flags &= !WC_ERR_INVALID_CHARS; // not supported before Vista. Without this flag, it will just replace bogus chars with U+FFFD. You're on your own, WinXP.
    }
    // SAFETY: the first call only measures; the second writes into a buffer
    // of the measured size.
    unsafe {
        let n = WideCharToMultiByte(
            code_page,
            flags,
            wide.as_ptr(),
            wide.len() as i32,
            std::ptr::null_mut(),
            0,
            std::ptr::null(),
            std::ptr::null_mut(),
        );
        if n <= 0 {
            return if wide.is_empty() {
                Some(Vec::new())
            } else {
                None
            };
        }
        let mut out = vec![0u8; n as usize];
        let n = WideCharToMultiByte(
            code_page,
            flags,
            wide.as_ptr(),
            wide.len() as i32,
            out.as_mut_ptr(),
            n,
            std::ptr::null(),
            std::ptr::null_mut(),
        );
        out.truncate(n.max(0) as usize);
        Some(out)
    }
}

/// The path of a loaded module (`None`: the executable). Translation of
/// `WIN_GetModulePath()`.
pub(crate) fn get_module_path(handle: HMODULE) -> Result<String> {
    let mut buflen: u32 = 128;
    loop {
        let mut path = vec![0u16; buflen as usize];
        // SAFETY: path holds buflen UTF-16 units.
        let len = unsafe { GetModuleFileNameW(handle, path.as_mut_ptr(), buflen) };
        // if it truncated, then len >= buflen - 1
        // if there was enough room (or failure), len < buflen - 1
        if len < buflen - 1 {
            if len == 0 {
                return Err(set_error("Couldn't locate module"));
            }
            return Ok(wide_to_utf8(&path[..len as usize]));
        }
        // buffer too small? Try again.
        buflen *= 2;
    }
}

/// Whether the broken 64-bit EZFRD64.DLL (a USB vibration driver that
/// crashes once loaded, which querying device capabilities does implicitly)
/// is installed, unless the `SDL_CHECK_BROKEN_EZFRD64` hint is false.
/// Translation of `WIN_HasBrokenEZFRD64DLL()`.
pub(crate) fn has_broken_ezfrd64_dll() -> bool {
    static BROKEN: OnceLock<bool> = OnceLock::new();
    *BROKEN.get_or_init(|| {
        if !cfg!(target_pointer_width = "64") || !crate::hints::get_bool("SDL_CHECK_BROKEN_EZFRD64", true) {
            return false;
        }
        // The 64-bit version of EZFRD64.DLL crashes after being loaded,
        // which happens implicitly when querying the device capabilities,
        // so make sure we don't do that if there's a possibility of crashing
        const DIRECTORIES: [&str; 2] = ["C:/Windows/USB_Vibration", "C:/Windows/USB Vibration"];
        for dir in DIRECTORIES {
            let files = crate::filesystem::glob_directory(
                dir,
                Some("*/EZFRD64.DLL"),
                crate::filesystem::GlobFlags::CASE_INSENSITIVE,
            );
            if files.is_ok_and(|f| !f.is_empty()) {
                crate::log::warn!(
                    crate::log::Category::Input,
                    "Broken EZFRD64.DLL detected, disabling GameInput and DirectInput force feedback"
                );
                return true;
            }
        }
        false
    })
}

/// The message-only window DirectInput needs for its cooperative level.
/// Translation of `SDL_HelperWindow`, `SDL_HelperWindowCreate()` and
/// `SDL_HelperWindowDestroy()`.
/// `hid.dll` and device notifications (SDL_hid.c).
pub(crate) mod hid;

pub(crate) mod helper_window {
    use super::*;

    struct State {
        hwnd: HWND,
        class: u16,
    }
    // SAFETY: the window handle is only used through the Mutex.
    unsafe impl Send for State {}

    static STATE: Mutex<State> = Mutex::new(State {
        hwnd: std::ptr::null_mut(),
        class: 0,
    });

    const CLASS_NAME: &str = "SDLHelperWindowInputCatcher";
    const WINDOW_NAME: &str = "SDLHelperWindowInputMsgWindow";

    /// The helper window, if created.
    pub(crate) fn hwnd() -> Option<HWND> {
        let s = STATE.lock().unwrap_or_else(|e| e.into_inner());
        (!s.hwnd.is_null()).then_some(s.hwnd)
    }

    /// Creates a HelperWindow used for DirectInput.
    pub(crate) fn create() -> Result<()> {
        let mut s = STATE.lock().unwrap_or_else(|e| e.into_inner());
        // Make sure window isn't created twice.
        if !s.hwnd.is_null() {
            return Ok(());
        }
        let class_name = utf8_to_wide(CLASS_NAME);
        let window_name = utf8_to_wide(WINDOW_NAME);
        // SAFETY: the class and window names are NUL-terminated and outlive
        // the calls; DefWindowProcW is a valid window procedure.
        unsafe {
            let h_instance = GetModuleHandleW(std::ptr::null());

            // Create the class.
            let wce = WNDCLASSW {
                style: 0,
                lpfnWndProc: Some(DefWindowProcW),
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: h_instance,
                hIcon: std::ptr::null_mut(),
                hCursor: std::ptr::null_mut(),
                hbrBackground: std::ptr::null_mut(),
                lpszMenuName: std::ptr::null(),
                lpszClassName: class_name.as_ptr(),
            };

            // Register the class.
            s.class = RegisterClassW(&wce);
            if s.class == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS {
                return Err(set_error("Unable to create Helper Window Class"));
            }

            // Create the window.
            s.hwnd = CreateWindowExW(
                0,
                class_name.as_ptr(),
                window_name.as_ptr(),
                WS_OVERLAPPED,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                HWND_MESSAGE,
                std::ptr::null_mut(),
                h_instance,
                std::ptr::null(),
            );
            if s.hwnd.is_null() {
                UnregisterClassW(class_name.as_ptr(), h_instance);
                return Err(set_error("Unable to create Helper Window"));
            }
        }
        Ok(())
    }

    /// Destroys the HelperWindow previously created with [`create`].
    pub(crate) fn destroy() -> Result<()> {
        let mut s = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let class_name = utf8_to_wide(CLASS_NAME);
        // SAFETY: the handles were created by create(); each is released once.
        unsafe {
            let h_instance = GetModuleHandleW(std::ptr::null());

            // Destroy the window.
            if !s.hwnd.is_null() {
                if DestroyWindow(s.hwnd) == 0 {
                    return Err(set_error("Unable to destroy Helper Window"));
                }
                s.hwnd = std::ptr::null_mut();
            }

            // Unregister the class.
            if s.class != 0 {
                if UnregisterClassW(class_name.as_ptr(), h_instance) == 0 {
                    return Err(set_error("Unable to destroy Helper Window Class"));
                }
                s.class = 0;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strings_round_trip() {
        let w = utf8_to_wide("héllo 🎮");
        assert_eq!(*w.last().unwrap(), 0);
        assert_eq!(wide_to_utf8(&w), "héllo 🎮");
        assert_eq!(wide_to_utf8(&[0x61, 0xD800, 0x62]), "a\u{FFFD}b");
        let utf8 = wide_char_to_multi_byte(
            windows_sys::Win32::Globalization::CP_UTF8,
            0,
            &w[..w.len() - 1],
        )
        .unwrap();
        assert_eq!(utf8, "héllo 🎮".as_bytes());
    }

    #[test]
    fn errors_carry_the_system_message() {
        // ERROR_FILE_NOT_FOUND
        let e = error_from_hresult(Some("Opening"), 2);
        assert!(e.message().starts_with("Opening: "), "{}", e.message());
        assert!(!e.message().ends_with('\n') && !e.message().contains('\r'));
    }

    #[test]
    fn version_checks() {
        assert!(is_windows_vista_or_greater());
        assert!(is_windows_7_or_greater());
        assert!(!is_windows_xp());
        let _ = is_windows_11_or_greater();
        let _ = is_wine();
    }

    #[test]
    fn com_initializes() {
        let hr = std::thread::spawn(|| {
            let hr = co_initialize();
            if hr >= 0 {
                co_uninitialize();
            }
            hr
        })
        .join()
        .unwrap();
        assert!(hr >= 0, "{hr:#x}");
    }

    #[test]
    fn rect_conversions() {
        let r = Rect::new(10, 20, 30, 40);
        let w = rect_to_win(&r);
        assert_eq!((w.left, w.top, w.right, w.bottom), (10, 20, 39, 59));
        assert_eq!(rect_from_win(&w), r);
        assert!(window_rect_valid(&w));
    }

    #[test]
    fn wave_formats() {
        let mut wf = WAVEFORMATEX {
            wFormatTag: WAVE_FORMAT_IEEE_FLOAT,
            nChannels: 2,
            nSamplesPerSec: 48000,
            nAvgBytesPerSec: 0,
            nBlockAlign: 8,
            wBitsPerSample: 32,
            cbSize: 0,
        };
        // SAFETY: the tag isn't WAVE_FORMAT_EXTENSIBLE.
        assert_eq!(
            unsafe { wave_format_ex_to_sdl_format(&wf) },
            Some(AudioFormat::F32)
        );
        wf.wFormatTag = WAVE_FORMAT_PCM;
        wf.wBitsPerSample = 16;
        // SAFETY: as above.
        assert_eq!(
            unsafe { wave_format_ex_to_sdl_format(&wf) },
            Some(AudioFormat::S16)
        );
        wf.wBitsPerSample = 8;
        // SAFETY: as above.
        assert_eq!(unsafe { wave_format_ex_to_sdl_format(&wf) }, None);

        // SAFETY: WAVEFORMATEXTENSIBLE is plain data.
        let mut ext: windows_sys::Win32::Media::Audio::WAVEFORMATEXTENSIBLE =
            unsafe { std::mem::zeroed() };
        ext.Format.wFormatTag = WAVE_FORMAT_EXTENSIBLE;
        ext.Format.wBitsPerSample = 32;
        ext.SubFormat = KSDATAFORMAT_SUBTYPE_PCM;
        // SAFETY: ext is a whole WAVEFORMATEXTENSIBLE.
        assert_eq!(
            unsafe { wave_format_ex_to_sdl_format(&ext.Format) },
            Some(AudioFormat::S32)
        );
    }

    #[test]
    fn module_path_and_helper_window() {
        let exe = get_module_path(std::ptr::null_mut()).unwrap();
        assert!(exe.to_ascii_lowercase().ends_with(".exe"), "{exe}");
        helper_window::create().unwrap();
        helper_window::create().unwrap();
        assert!(helper_window::hwnd().is_some());
        helper_window::destroy().unwrap();
        assert!(helper_window::hwnd().is_none());
    }
}
