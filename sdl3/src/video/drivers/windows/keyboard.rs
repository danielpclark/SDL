// Rust translation of src/video/windows/SDL_windowskeyboard.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Keymaps, dead keys and the IMM32 input method support of the Windows
//! video driver.
//!
//! The IME state (`videodata->ime_*`) lives in [`ImeData`]. Many IMM
//! functions send window messages synchronously, and the window procedure
//! calls back into this module, so the state is only ever borrowed briefly
//! and never across an IMM call or an SDL event function.

use std::sync::Mutex;

use windows_sys::Win32::Foundation::{HWND, LPARAM, POINT, RECT, WPARAM};
use windows_sys::Win32::Globalization::{CompareStringA, NORM_IGNORECASE};
use windows_sys::Win32::Graphics::Gdi::LOGFONTW;
use windows_sys::Win32::Storage::FileSystem::{
    GetFileVersionInfoA, GetFileVersionInfoSizeA, VerQueryValueA, VS_FIXEDFILEINFO,
};
use windows_sys::Win32::UI::Input::Ime::{
    ImmAssociateContext, ImmGetCandidateListW, ImmGetCompositionFontW, ImmGetCompositionStringW,
    ImmGetContext, ImmGetIMEFileNameA, ImmNotifyIME, ImmReleaseContext, ImmSetCandidateWindow,
    ImmSetCompositionStringW, ImmSetCompositionWindow, ATTR_TARGET_CONVERTED,
    ATTR_TARGET_NOTCONVERTED, CANDIDATEFORM, CANDIDATELIST, CFS_EXCLUDE, CFS_RECT, COMPOSITIONFORM,
    CPS_CANCEL, GCS_COMPATTR, GCS_COMPSTR, GCS_CURSORPOS, GCS_RESULTSTR, HIMC, HIMCC,
    IMN_CHANGECANDIDATE, IMN_CLOSECANDIDATE, IMN_OPENCANDIDATE, IMN_PRIVATE, IMN_SETCANDIDATEPOS,
    IMN_SETCOMPOSITIONFONT, IMN_SETCOMPOSITIONWINDOW, IMN_SETCONVERSIONMODE, IMN_SETOPENSTATUS,
    INPUTCONTEXT, ISC_SHOWUIALL, ISC_SHOWUIALLCANDIDATEWINDOW, ISC_SHOWUICOMPOSITIONWINDOW,
    NI_CLOSECANDIDATE, NI_COMPOSITIONSTR, SCS_SETSTR,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, GetKeyboardLayout, GetKeyboardState, MapVirtualKeyW, ToUnicode, HKL,
    MAPVK_VK_TO_VSC, MAPVK_VSC_TO_VK, VK_BACK, VK_CAPITAL, VK_CONTROL, VK_MENU, VK_NUMLOCK,
    VK_PROCESSKEY, VK_SCROLL, VK_SHIFT, VK_SPACE,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    WM_IME_COMPOSITION, WM_IME_ENDCOMPOSITION, WM_IME_NOTIFY, WM_IME_SETCONTEXT,
    WM_IME_STARTCOMPOSITION, WM_INPUTLANGCHANGE, WM_KEYDOWN, WM_SYSKEYDOWN,
};

use super::window::window_data;
use super::VideoData;
use crate::core::windows::wide_to_utf8;
use crate::error::Result;
use crate::events::keyboard::{self, Keycode, Keymap, Keymod, Scancode};
use crate::events::scancodes_windows::WINDOWS_SCANCODE_TABLE;
use crate::events::WindowID;
use crate::hints;
use crate::loadso::SharedObject;
use crate::properties::Properties;
use crate::video::core::with_window;
use crate::video::Rect;

/// Translation of `MAX_CANDLIST`.
const MAX_CANDLIST: usize = 10;

type ImmLockImcFn = unsafe extern "system" fn(HIMC) -> *mut INPUTCONTEXT;
type ImmUnlockImcFn = unsafe extern "system" fn(HIMC) -> i32;
type ImmLockImccFn = unsafe extern "system" fn(HIMCC) -> *mut std::ffi::c_void;
type ImmUnlockImccFn = unsafe extern "system" fn(HIMCC) -> i32;
type GetReadingStringFn =
    unsafe extern "system" fn(HIMC, u32, *mut u16, *mut i32, *mut i32, *mut u32) -> u32;
type ShowReadingWindowFn = unsafe extern "system" fn(HIMC, i32) -> i32;

/// The IME fields of `SDL_VideoData` (`ime_*`, `ImmLockIMC`... and
/// `GetReadingString`/`ShowReadingWindow`), plus `IME_GetId()`'s statics.
pub(crate) struct ImeData {
    initialized: bool,
    enabled: bool,
    available: bool,
    internal_composition: bool,
    internal_candidates: bool,
    hwnd_main: HWND,
    hwnd_current: HWND,
    needs_clear_composition: bool,
    himc: HIMC,

    /// `ime_composition`, a buffer of WCHARs
    composition: Vec<u16>,
    /// `ime_composition_length`, in bytes
    composition_length: i32,
    readingstring: [u16; 16],
    cursor: i32,
    selected_start: i32,
    selected_length: i32,

    candidates_open: bool,
    update_candidates: bool,
    candidates: [Option<String>; MAX_CANDLIST],
    candcount: i32,
    candsel: u32,
    candlistindexbase: i32,
    horizontal_candidates: bool,

    composition_area: COMPOSITIONFORM,
    candidate_area: CANDIDATEFORM,

    hkl: HKL,
    himm32: Option<SharedObject>,
    imm_lock_imc: Option<ImmLockImcFn>,
    imm_unlock_imc: Option<ImmUnlockImcFn>,
    imm_lock_imcc: Option<ImmLockImccFn>,
    imm_unlock_imcc: Option<ImmUnlockImccFn>,
    get_reading_string: Option<GetReadingStringFn>,
    show_reading_window: Option<ShowReadingWindowFn>,

    /// `IME_GetId()`'s `hklprev` and `dwRet`
    id_hklprev: HKL,
    id_ret: [u32; 2],
}

// SAFETY: the handles are process-wide tokens, only used on the thread
// that owns the windows; the struct is behind the Shared lock.
unsafe impl Send for ImeData {}

const ZERO_POINT: POINT = POINT { x: 0, y: 0 };
const ZERO_RECT: RECT = RECT {
    left: 0,
    top: 0,
    right: 0,
    bottom: 0,
};
const ZERO_COMPOSITIONFORM: COMPOSITIONFORM = COMPOSITIONFORM {
    dwStyle: 0,
    ptCurrentPos: ZERO_POINT,
    rcArea: ZERO_RECT,
};
const ZERO_CANDIDATEFORM: CANDIDATEFORM = CANDIDATEFORM {
    dwIndex: 0,
    dwStyle: 0,
    ptCurrentPos: ZERO_POINT,
    rcArea: ZERO_RECT,
};

impl ImeData {
    /// The zeroed fields of a new `SDL_VideoData`.
    pub(crate) fn new() -> ImeData {
        ImeData {
            initialized: false,
            enabled: false,
            available: false,
            internal_composition: false,
            internal_candidates: false,
            hwnd_main: std::ptr::null_mut(),
            hwnd_current: std::ptr::null_mut(),
            needs_clear_composition: false,
            himc: std::ptr::null_mut(),
            composition: Vec::new(),
            composition_length: 0,
            readingstring: [0; 16],
            cursor: 0,
            selected_start: 0,
            selected_length: 0,
            candidates_open: false,
            update_candidates: false,
            candidates: Default::default(),
            candcount: 0,
            candsel: 0,
            candlistindexbase: 0,
            horizontal_candidates: false,
            composition_area: ZERO_COMPOSITIONFORM,
            candidate_area: ZERO_CANDIDATEFORM,
            hkl: std::ptr::null_mut(),
            himm32: None,
            imm_lock_imc: None,
            imm_unlock_imc: None,
            imm_lock_imcc: None,
            imm_unlock_imcc: None,
            get_reading_string: None,
            show_reading_window: None,
            id_hklprev: std::ptr::null_mut(),
            id_ret: [0; 2],
        }
    }
}

/// Run `f` on the IME state (briefly: see the module docs).
fn ime<R>(videodata: &VideoData, f: impl FnOnce(&mut ImeData) -> R) -> R {
    videodata.ime.with(f)
}

/// An `HKL` as the number upstream compares it with.
fn hkl_bits(hkl: HKL) -> usize {
    hkl as usize
}

/* Building keymaps is expensive, so keep a reasonably-sized LRU cache to
 * enable fast switching between commonly used ones.
 */
/// Translation of `keymap_cache` (`keyboard_layout` as a number).
static KEYMAP_CACHE: Mutex<Vec<(usize, Keymap)>> = Mutex::new(Vec::new());
const KEYMAP_CACHE_SIZE: usize = 4;

/// Translation of `WIN_GetCachedKeymap()`.
fn get_cached_keymap(layout: usize) -> Option<Keymap> {
    let mut cache = KEYMAP_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    let i = cache.iter().position(|(l, _)| *l == layout)?;
    // Move the map to the front of the list.
    if i != 0 {
        let entry = cache.remove(i);
        cache.insert(0, entry);
    }
    Some(cache[0].1.clone())
}

/// Translation of `WIN_CacheKeymap()`.
fn cache_keymap(layout: usize, keymap: Keymap) {
    let mut cache = KEYMAP_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    // If the cache is full, evict the last keymap.
    if cache.len() == KEYMAP_CACHE_SIZE {
        cache.pop();
    }

    // Move all elements down by one.
    cache.insert(0, (layout, keymap));
}

/// `LOBYTE()`
fn lobyte(v: u32) -> u32 {
    v & 0xff
}

/// Translation of `WIN_BuildKeymap()`.
fn build_keymap() -> Keymap {
    let mut keyboard_state = [0u8; 256];
    let mut buffer = [0u16; 16];
    let mods = [
        Keymod::NONE,
        Keymod::SHIFT,
        Keymod::CAPS,
        Keymod::SHIFT | Keymod::CAPS,
        Keymod::MODE,
        Keymod::MODE | Keymod::SHIFT,
        Keymod::MODE | Keymod::CAPS,
        Keymod::MODE | Keymod::SHIFT | Keymod::CAPS,
    ];

    reset_dead_keys();

    let mut keymap = Keymap::new();

    for &m in &mods {
        for (i, &scancode) in WINDOWS_SCANCODE_TABLE.iter().enumerate() {
            let i = i as u32;
            // Make sure this scancode is a valid character scancode
            if scancode == Scancode::UNKNOWN
                || scancode == Scancode::DELETE
                || (Keymap::keycode_in(None, scancode, Keymod::NONE).0 & Keycode::SCANCODE_MASK)
                    != 0
            {
                // The Colemak mapping swaps Backspace and CapsLock
                if m == Keymod::NONE
                    && (scancode == Scancode::CAPSLOCK || scancode == Scancode::BACKSPACE)
                {
                    // SAFETY: MapVirtualKeyW is a pure lookup.
                    let vk = lobyte(unsafe { MapVirtualKeyW(i, MAPVK_VSC_TO_VK) });
                    if vk == VK_CAPITAL as u32 {
                        keymap.set_entry(scancode, m, Keycode::CAPSLOCK);
                    } else if vk == VK_BACK as u32 {
                        keymap.set_entry(scancode, m, Keycode::BACKSPACE);
                    }
                }
                continue;
            }

            // Unpack the single byte index to make the scan code.
            let sc = (i & 0x7f) | (if (i & 0x80) != 0 { 0xe0 } else { 0x00 } << 8);
            // SAFETY: as above.
            let vk = lobyte(unsafe { MapVirtualKeyW(sc, MAPVK_VSC_TO_VK) });
            if vk == 0 {
                continue;
            }

            // Update the keyboard state for the modifiers
            keyboard_state[VK_SHIFT as usize] = if m.intersects(Keymod::SHIFT) {
                0x80
            } else {
                0x00
            };
            keyboard_state[VK_CAPITAL as usize] = if m.intersects(Keymod::CAPS) {
                0x01
            } else {
                0x00
            };
            keyboard_state[VK_CONTROL as usize] = if m.intersects(Keymod::MODE) {
                0x80
            } else {
                0x00
            };
            keyboard_state[VK_MENU as usize] = if m.intersects(Keymod::MODE) {
                0x80
            } else {
                0x00
            };

            // SAFETY: the state has 256 entries and the buffer 16 WCHARs.
            let result =
                unsafe { ToUnicode(vk, sc, keyboard_state.as_ptr(), buffer.as_mut_ptr(), 16, 0) };
            let n = (result.unsigned_abs() as usize).min(15);
            buffer[n] = 0;

            // Convert UTF-16 to UTF-32 code points
            let utf16: Vec<u8> = buffer[..=n].iter().flat_map(|c| c.to_le_bytes()).collect();
            match crate::stdlib::iconv::iconv_string("UTF-32LE", "UTF-16LE", &utf16) {
                Ok(ch) => {
                    let at = |k: usize| {
                        ch.get(k * 4..k * 4 + 4)
                            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                            .unwrap_or(0)
                    };
                    /* Windows keyboard layouts can emit several UTF-32 code points on a single key press.
                     * Use <U+FFFD REPLACEMENT CHARACTER> since we cannot fit into single SDL_Keycode value in SDL keymap.
                     * See https://kbdlayout.info/features/ligatures for a list of such keys. */
                    let code = if at(1) == 0 { at(0) } else { 0xfffd };
                    keymap.set_entry(scancode, m, Keycode(code));
                }
                Err(_) => {
                    // The default keymap doesn't have any SDL_KMOD_MODE entries, so we don't need to override them
                    if !m.intersects(Keymod::MODE) {
                        keymap.set_entry(scancode, m, Keycode::UNKNOWN);
                    }
                }
            }

            if result < 0 {
                reset_dead_keys();
            }
        }
    }

    keymap
}

/// Build (or fetch from the cache) and bind the keymap of the current
/// keyboard layout. Translation of `WIN_UpdateKeymap()`.
pub(crate) fn update_keymap(send_event: bool) {
    // SAFETY: GetKeyboardLayout has no preconditions.
    let layout = hkl_bits(unsafe { GetKeyboardLayout(0) });
    let keymap = match get_cached_keymap(layout) {
        Some(keymap) => keymap,
        None => {
            let keymap = build_keymap();
            cache_keymap(layout, keymap.clone());
            keymap
        }
    };

    keyboard::set_keymap(Some(keymap), send_event);
}

// Alphabetic scancodes for PC keyboards
/// Translation of `WIN_InitKeyboard()`.
pub(crate) fn init_keyboard(data: &VideoData) {
    ime(data, |d| {
        d.candlistindexbase = 1;
        d.composition_length = 32 * 2;
        d.composition = vec![0; d.composition_length as usize];
    });

    // Build and bind the current keymap.
    update_keymap(false);

    let _ = Scancode::APPLICATION.set_name(Some("Menu"));
    let _ = Scancode::LGUI.set_name(Some("Left Windows"));
    let _ = Scancode::RGUI.set_name(Some("Right Windows"));

    // Are system caps/num/scroll lock active? Set our state to match.
    // SAFETY: GetKeyState has no preconditions.
    unsafe {
        keyboard::toggle_mod_state(Keymod::CAPS, (GetKeyState(VK_CAPITAL as i32) & 0x0001) != 0);
        keyboard::toggle_mod_state(Keymod::NUM, (GetKeyState(VK_NUMLOCK as i32) & 0x0001) != 0);
        keyboard::toggle_mod_state(
            Keymod::SCROLL,
            (GetKeyState(VK_SCROLL as i32) & 0x0001) != 0,
        );
    }
}

/// Translation of `WIN_QuitKeyboard()`.
pub(crate) fn quit_keyboard(data: &VideoData) {
    ime_quit(data);

    ime(data, |d| d.composition = Vec::new());

    KEYMAP_CACHE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clear();
}

/// Translation of `WIN_ResetDeadKeys()`.
pub(crate) fn reset_dead_keys() {
    /*
    if a deadkey has been typed, but not the next character (which the deadkey might modify),
    this tries to undo the effect pressing the deadkey.
    see: http://archives.miloush.net/michkap/archive/2006/09/10/748775.html
    */
    let mut keyboard_state = [0u8; 256];
    let mut buffer = [0u16; 16];

    // SAFETY: the state buffer has the 256 entries GetKeyboardState fills.
    if unsafe { GetKeyboardState(keyboard_state.as_mut_ptr()) } == 0 {
        return;
    }

    let vk = VK_SPACE as u32;
    // SAFETY: MapVirtualKeyW is a pure lookup.
    let sc = unsafe { MapVirtualKeyW(vk, MAPVK_VK_TO_VSC) };
    if sc == 0 {
        // the keyboard doesn't have this key
        return;
    }

    for _ in 0..5 {
        // SAFETY: the state has 256 entries and the buffer 16 WCHARs.
        let result =
            unsafe { ToUnicode(vk, sc, keyboard_state.as_ptr(), buffer.as_mut_ptr(), 16, 0) };
        if result > 0 {
            // success
            return;
        }
    }
}

/// The window's `HWND` (null if it has no driver data).
fn window_hwnd(window: WindowID) -> HWND {
    window_data(window).map_or(std::ptr::null_mut(), |d| d.hwnd)
}

/// The window's text input area and cursor.
fn text_input_area(window: WindowID) -> (Rect, i32) {
    with_window(window, |w| (w.text_input_rect, w.text_input_cursor)).unwrap_or_default()
}

/// Translation of `WIN_StartTextInput()`.
pub(crate) fn start_text_input(
    videodata: &VideoData,
    window: WindowID,
    _props: Option<&Properties>,
) -> Result<()> {
    reset_dead_keys();

    let hwnd = window_hwnd(window);
    ime_init(videodata, window);
    ime_enable(videodata, hwnd);

    update_text_input_area(videodata, window)?;

    Ok(())
}

/// Translation of `WIN_StopTextInput()`.
pub(crate) fn stop_text_input(videodata: &VideoData, window: WindowID) -> Result<()> {
    reset_dead_keys();

    let hwnd = window_hwnd(window);
    ime_init(videodata, window);
    ime_disable(videodata, hwnd);

    Ok(())
}

/// Translation of `WIN_UpdateTextInputArea()`.
pub(crate) fn update_text_input_area(videodata: &VideoData, window: WindowID) -> Result<()> {
    let hwnd = window_hwnd(window);
    let (rect, cursor) = text_input_area(window);
    ime_set_text_input_area(videodata, hwnd, &rect, cursor);
    Ok(())
}

/// Translation of `WIN_ClearComposition()`.
pub(crate) fn clear_composition(videodata: &VideoData, _window: WindowID) -> Result<()> {
    ime_clear_composition(videodata);
    Ok(())
}

/// `MAKELANGID()`
const fn makelangid(primary: u32, sub: u32) -> u32 {
    (sub << 10) | primary
}
const LANG_CHINESE: u32 = 0x04;
const LANG_KOREAN: u32 = 0x12;
const LANG_CHT: u32 = makelangid(LANG_CHINESE, 0x01); // SUBLANG_CHINESE_TRADITIONAL
const LANG_CHS: u32 = makelangid(LANG_CHINESE, 0x02); // SUBLANG_CHINESE_SIMPLIFIED

/// `MAKEIMEVERSION()`
const fn makeimeversion(major: u32, minor: u32) -> u32 {
    ((major & 0xff) << 24) | ((minor & 0xff) << 16)
}

const CHT_HKL_DAYI: usize = 0xE0060404;
const CHT_HKL_NEW_PHONETIC: usize = 0xE0080404;
const CHT_HKL_NEW_CHANG_JIE: usize = 0xE0090404;
const CHT_HKL_NEW_QUICK: usize = 0xE00A0404;
const CHT_HKL_HK_CANTONESE: usize = 0xE00B0404;
const CHT_IMEFILENAME1: &std::ffi::CStr = c"TINTLGNT.IME";
const CHT_IMEFILENAME2: &std::ffi::CStr = c"CINTLGNT.IME";
const CHT_IMEFILENAME3: &std::ffi::CStr = c"MSTCIPHA.IME";
const IMEID_CHT_VER42: u32 = LANG_CHT | makeimeversion(4, 2);
const IMEID_CHT_VER43: u32 = LANG_CHT | makeimeversion(4, 3);
const IMEID_CHT_VER44: u32 = LANG_CHT | makeimeversion(4, 4);
const IMEID_CHT_VER50: u32 = LANG_CHT | makeimeversion(5, 0);
const IMEID_CHT_VER51: u32 = LANG_CHT | makeimeversion(5, 1);
const IMEID_CHT_VER52: u32 = LANG_CHT | makeimeversion(5, 2);
const IMEID_CHT_VER60: u32 = LANG_CHT | makeimeversion(6, 0);
const IMEID_CHT_VER_VISTA: u32 = LANG_CHT | makeimeversion(7, 0);

const CHS_HKL: usize = 0xE00E0804;
const CHS_IMEFILENAME1: &std::ffi::CStr = c"PINTLGNT.IME";
const CHS_IMEFILENAME2: &std::ffi::CStr = c"MSSCIPYA.IME";
const IMEID_CHS_VER41: u32 = LANG_CHS | makeimeversion(4, 1);
const IMEID_CHS_VER42: u32 = LANG_CHS | makeimeversion(4, 2);
const IMEID_CHS_VER53: u32 = LANG_CHS | makeimeversion(5, 3);

/// `LANG()`
fn lang(d: &ImeData) -> u32 {
    (hkl_bits(d.hkl) & 0xffff) as u32
}
/// `PRIMLANG()`
fn primlang(d: &ImeData) -> u32 {
    lang(d) & 0x3ff
}

/// Translation of `IME_Init()`.
fn ime_init(videodata: &VideoData, window: WindowID) -> bool {
    let hwnd = window_hwnd(window);

    if ime(videodata, |d| d.initialized) {
        return true;
    }

    let hint = hints::get(hints::IME_IMPLEMENTED_UI);
    ime(videodata, |d| {
        if hint.as_deref().is_some_and(|h| h.contains("composition")) {
            d.internal_composition = true;
        }
        if hint.as_deref().is_some_and(|h| h.contains("candidates")) {
            d.internal_candidates = true;
        }

        d.hwnd_main = hwnd;
        d.initialized = true;
    });
    let Ok(himm32) = SharedObject::load("imm32.dll") else {
        ime(videodata, |d| d.available = false);
        // (SDL_ClearError(): the load error was returned, not set)
        return true;
    };
    // SAFETY: the types match the IMM32 exports.
    unsafe {
        let lock_imc = himm32.function::<ImmLockImcFn>("ImmLockIMC").ok();
        let unlock_imc = himm32.function::<ImmUnlockImcFn>("ImmUnlockIMC").ok();
        let lock_imcc = himm32.function::<ImmLockImccFn>("ImmLockIMCC").ok();
        let unlock_imcc = himm32.function::<ImmUnlockImccFn>("ImmUnlockIMCC").ok();
        ime(videodata, |d| {
            d.imm_lock_imc = lock_imc;
            d.imm_unlock_imc = unlock_imc;
            d.imm_lock_imcc = lock_imcc;
            d.imm_unlock_imcc = unlock_imcc;
            d.himm32 = Some(himm32);
        });
    }

    ime_set_window(videodata, window);
    // SAFETY: hwnd is a window handle (or null, for which IMM fails).
    let himc = unsafe {
        let himc = ImmGetContext(hwnd);
        ImmReleaseContext(hwnd, himc);
        himc
    };
    ime(videodata, |d| d.himc = himc);
    if himc.is_null() {
        ime(videodata, |d| d.available = false);
        ime_disable(videodata, hwnd);
        return true;
    }
    ime(videodata, |d| d.available = true);
    ime_update_input_locale(videodata);
    ime_setup_api(videodata);
    ime_update_input_locale(videodata);
    ime_disable(videodata, hwnd);
    true
}

/// Translation of `IME_Enable()`.
fn ime_enable(videodata: &VideoData, hwnd: HWND) {
    let (initialized, current, available, main, himc) = ime(videodata, |d| {
        (
            d.initialized,
            d.hwnd_current,
            d.available,
            d.hwnd_main,
            d.himc,
        )
    });
    if !initialized || current.is_null() {
        return;
    }

    if !available {
        ime_disable(videodata, hwnd);
        return;
    }
    if current == main {
        // SAFETY: current is a window handle and himc its input context.
        unsafe { ImmAssociateContext(current, himc) };
    }

    ime(videodata, |d| d.enabled = true);
    ime_update_input_locale(videodata);
}

/// Translation of `IME_Disable()`.
fn ime_disable(videodata: &VideoData, _hwnd: HWND) {
    let (initialized, current, main) =
        ime(videodata, |d| (d.initialized, d.hwnd_current, d.hwnd_main));
    if !initialized || current.is_null() {
        return;
    }

    ime_clear_composition(videodata);
    if current == main {
        // SAFETY: current is a window handle.
        unsafe { ImmAssociateContext(current, std::ptr::null_mut()) };
    }

    ime(videodata, |d| d.enabled = false);
}

/// Translation of `IME_Quit()`.
fn ime_quit(videodata: &VideoData) {
    let (initialized, main, himc) = ime(videodata, |d| (d.initialized, d.hwnd_main, d.himc));
    if !initialized {
        return;
    }

    if !main.is_null() {
        // SAFETY: main is a window handle and himc its input context.
        unsafe { ImmAssociateContext(main, himc) };
    }

    let himm32 = ime(videodata, |d| {
        d.hwnd_main = std::ptr::null_mut();
        d.himc = std::ptr::null_mut();
        d.imm_lock_imc = None;
        d.imm_unlock_imc = None;
        d.imm_lock_imcc = None;
        d.imm_unlock_imcc = None;
        let himm32 = d.himm32.take();
        for i in 0..d.candcount as usize {
            d.candidates[i] = None;
        }
        d.initialized = false;
        himm32
    });
    drop(himm32);
}

/// Copy at most `max - 1` WCHARs of a NUL-terminated string (`SDL_wcslcpy()`
/// into a buffer of `max`).
fn wcslcpy(dst: &mut [u16], src: impl IntoIterator<Item = u16>, max: usize) {
    let mut n = 0;
    if max == 0 {
        return;
    }
    for c in src {
        if c == 0 || n + 1 >= max || n + 1 >= dst.len() {
            break;
        }
        dst[n] = c;
        n += 1;
    }
    dst[n] = 0;
}

/// The NUL-terminated WCHAR string at `p`, up to `max` WCHARs.
///
/// # Safety
///
/// `p` must be readable up to its terminator or `max` WCHARs.
unsafe fn read_wide(p: *const u16, max: usize) -> Vec<u16> {
    let mut out = Vec::new();
    for i in 0..max {
        // SAFETY: as the caller promises.
        let c = unsafe { p.add(i).read_unaligned() };
        if c == 0 {
            break;
        }
        out.push(c);
    }
    out
}

/// Translation of `IME_GetReadingString()` (the private data offsets are
/// written as upstream writes them).
#[allow(clippy::identity_op)]
fn ime_get_reading_string(videodata: &VideoData, hwnd: HWND) {
    let mut buffer = [0u16; 16];
    let mut err: i32 = 0;
    let mut vertical: i32 = 0;
    let mut maxuilen: u32 = 0;

    ime(videodata, |d| d.readingstring[0] = 0);

    let id = ime_get_id(videodata, 0);
    if id == 0 {
        return;
    }

    // SAFETY: hwnd is a window handle (or null, for which IMM fails).
    let himc = unsafe { ImmGetContext(hwnd) };
    if himc.is_null() {
        return;
    }

    let (get_reading_string, lock_imc, unlock_imc, lock_imcc, unlock_imcc) = ime(videodata, |d| {
        (
            d.get_reading_string,
            d.imm_lock_imc,
            d.imm_unlock_imc,
            d.imm_lock_imcc,
            d.imm_unlock_imcc,
        )
    });
    if let Some(get_reading_string) = get_reading_string {
        // SAFETY: the IME's GetReadingString, with a buffer of the given size.
        let len = unsafe {
            let mut len = get_reading_string(
                himc,
                0,
                std::ptr::null_mut(),
                &mut err,
                &mut vertical,
                &mut maxuilen,
            );
            if len != 0 {
                if len as usize > buffer.len() {
                    len = buffer.len() as u32;
                }

                len = get_reading_string(
                    himc,
                    len,
                    buffer.as_mut_ptr(),
                    &mut err,
                    &mut vertical,
                    &mut maxuilen,
                );
            }
            len
        };
        // FIXME (upstream): the buffer GetReadingString() fills isn't NUL-terminated, and is read uninitialized when len is 0; here reading stops at its end.
        ime(videodata, |d| {
            wcslcpy(&mut d.readingstring, buffer.iter().copied(), len as usize)
        });
    } else {
        // FIXME (upstream): the ImmLockIMC()... pointers and the locked context are used unchecked; here a missing one skips the reading string.
        let (Some(lock_imc), Some(unlock_imc), Some(lock_imcc), Some(unlock_imcc)) =
            (lock_imc, unlock_imc, lock_imcc, unlock_imcc)
        else {
            // SAFETY: himc came from ImmGetContext(hwnd).
            unsafe { ImmReleaseContext(hwnd, himc) };
            return;
        };
        let id1 = if id == IMEID_CHS_VER41 {
            ime_get_id(videodata, 1)
        } else {
            0
        };
        // SAFETY: the private IME data layouts of these IME versions, as
        // upstream reads them (unaligned reads of pointers and DWORDs).
        unsafe {
            let lpimc = lock_imc(himc);
            if lpimc.is_null() {
                ImmReleaseContext(hwnd, himc);
                return;
            }
            let private = (*lpimc).hPrivate;
            let base = lock_imcc(private) as *const u8;
            let ptr_at = |p: *const u8, off: usize| -> *const u8 {
                p.add(off).cast::<*const u8>().read_unaligned()
            };
            let dword_at =
                |p: *const u8, off: usize| -> u32 { p.add(off).cast::<u32>().read_unaligned() };
            let mut len: u32 = 0;
            let mut s: *const u16 = std::ptr::null();
            match id {
                IMEID_CHT_VER42 | IMEID_CHT_VER43 | IMEID_CHT_VER44 => {
                    let p = ptr_at(base, 24);
                    if !p.is_null() {
                        len = dword_at(p, 7 * 4 + 32 * 4);
                        s = p.add(56).cast();
                    }
                }
                IMEID_CHT_VER51 | IMEID_CHT_VER52 | IMEID_CHS_VER53 => {
                    let p = ptr_at(base, 4);
                    if !p.is_null() {
                        let p = ptr_at(p, 1 * 4 + 5 * 4);
                        if !p.is_null() {
                            len = dword_at(p, 1 * 4 + (16 * 2 + 2 * 4) + 5 * 4 + 16 * 2);
                            s = p.add(1 * 4 + (16 * 2 + 2 * 4) + 5 * 4).cast();
                        }
                    }
                }
                IMEID_CHS_VER41 => {
                    let offset = if id1 >= 0x00000002 { 8 } else { 7 };
                    let p = ptr_at(base, offset * 4);
                    if !p.is_null() {
                        len = dword_at(p, 7 * 4 + 16 * 2 * 4);
                        s = p.add(6 * 4 + 16 * 2 * 1).cast();
                    }
                }
                IMEID_CHS_VER42 => {
                    let p = ptr_at(base, 1 * 4 + 1 * 4 + 6 * 4);
                    if !p.is_null() {
                        len = dword_at(p, 1 * 4 + (16 * 2 + 2 * 4) + 5 * 4 + 16 * 2);
                        s = p.add(1 * 4 + (16 * 2 + 2 * 4) + 5 * 4).cast();
                    }
                }
                _ => {}
            }
            if !s.is_null() {
                let size = (len as usize + 1).min(16);
                let text = read_wide(s, size);
                ime(videodata, |d| wcslcpy(&mut d.readingstring, text, size));
            }

            unlock_imcc(private);
            unlock_imc(himc);
        }
    }
    // SAFETY: himc came from ImmGetContext(hwnd).
    unsafe { ImmReleaseContext(hwnd, himc) };
    ime_send_editing_event(videodata);
}

/// Translation of `IME_InputLangChanged()`.
fn ime_input_lang_changed(videodata: &VideoData) {
    let lang = ime(videodata, |d| primlang(d));
    ime_update_input_locale(videodata);

    ime_setup_api(videodata);
    if lang != ime(videodata, |d| primlang(d)) {
        ime_clear_composition(videodata);
    }
}

/// `CompareStringA(LCID_INVARIANT, NORM_IGNORECASE, a, -1, b, -1) != CSTR_EQUAL`
fn names_differ(a: &[u8], b: &std::ffi::CStr) -> bool {
    // LCID_INVARIANT: MAKELCID(MAKELANGID(LANG_ENGLISH, SUBLANG_ENGLISH_US), SORT_DEFAULT)
    const LCID_INVARIANT: u32 = 0x0409;
    // SAFETY: both strings are NUL-terminated.
    unsafe {
        CompareStringA(
            LCID_INVARIANT,
            NORM_IGNORECASE,
            a.as_ptr().cast(),
            -1,
            b.as_ptr().cast(),
            -1,
        ) != 2
    }
}

/// Translation of `IME_GetId()`.
fn ime_get_id(videodata: &VideoData, index: usize) -> u32 {
    crate::sdl_assert!(index < 2);

    let (hkl, hklprev, ret, internal_candidates, has_get_reading_string) = ime(videodata, |d| {
        (
            d.hkl,
            d.id_hklprev,
            d.id_ret,
            d.internal_candidates,
            d.get_reading_string.is_some(),
        )
    });
    if hklprev == hkl {
        return ret[index];
    }
    let set = |r0: u32, r1: u32| {
        ime(videodata, |d| {
            d.id_hklprev = hkl;
            d.id_ret = [r0, r1];
        });
        r0
    };

    crate::sdl_assert!(index == 0);
    let lang = (hkl_bits(hkl) & 0xffff) as u32;
    // FIXME: What does this do?
    if internal_candidates && lang == LANG_CHT {
        return set(IMEID_CHT_VER_VISTA, 0);
    }
    let h = hkl_bits(hkl);
    if h != CHT_HKL_NEW_PHONETIC
        && h != CHT_HKL_NEW_CHANG_JIE
        && h != CHT_HKL_NEW_QUICK
        && h != CHT_HKL_HK_CANTONESE
        && h != CHS_HKL
    {
        return set(0, 0);
    }
    let mut temp = [0u8; 256];
    // SAFETY: the buffer holds the size passed.
    if unsafe { ImmGetIMEFileNameA(hkl, temp.as_mut_ptr(), (temp.len() - 1) as u32) } == 0 {
        return set(0, 0);
    }
    if !has_get_reading_string {
        if names_differ(&temp, CHT_IMEFILENAME1)
            && names_differ(&temp, CHT_IMEFILENAME2)
            && names_differ(&temp, CHT_IMEFILENAME3)
            && names_differ(&temp, CHS_IMEFILENAME1)
            && names_differ(&temp, CHS_IMEFILENAME2)
        {
            return set(0, 0);
        }
        // SAFETY: temp is NUL-terminated; the version buffer has the size
        // asked for, and VerQueryValueA points into it.
        unsafe {
            let ver_size = GetFileVersionInfoSizeA(temp.as_ptr(), std::ptr::null_mut());
            if ver_size != 0 {
                let mut ver_buffer = vec![0u8; ver_size as usize];
                if GetFileVersionInfoA(temp.as_ptr(), 0, ver_size, ver_buffer.as_mut_ptr().cast())
                    != 0
                {
                    let mut ver_data: *mut std::ffi::c_void = std::ptr::null_mut();
                    let mut cb_ver_data: u32 = 0;
                    if VerQueryValueA(
                        ver_buffer.as_ptr().cast(),
                        c"\\".as_ptr().cast(),
                        &mut ver_data,
                        &mut cb_ver_data,
                    ) != 0
                    {
                        let info = ver_data.cast::<VS_FIXEDFILEINFO>().read_unaligned();
                        let mut ver = info.dwFileVersionMS;
                        ver = ((ver & 0x00ff0000) << 8) | ((ver & 0x000000ff) << 16);
                        if has_get_reading_string
                            || (lang == LANG_CHT
                                && (ver == makeimeversion(4, 2)
                                    || ver == makeimeversion(4, 3)
                                    || ver == makeimeversion(4, 4)
                                    || ver == makeimeversion(5, 0)
                                    || ver == makeimeversion(5, 1)
                                    || ver == makeimeversion(5, 2)
                                    || ver == makeimeversion(6, 0)))
                            || (lang == LANG_CHS
                                && (ver == makeimeversion(4, 1)
                                    || ver == makeimeversion(4, 2)
                                    || ver == makeimeversion(5, 3)))
                        {
                            return set(ver | lang, info.dwFileVersionLS);
                        }
                    }
                }
            }
        }
    }
    set(0, 0)
}

/// Translation of `IME_SetupAPI()`.
fn ime_setup_api(videodata: &VideoData) {
    let mut ime_file = [0u8; 260 + 1];
    let hkl = ime(videodata, |d| {
        d.get_reading_string = None;
        d.show_reading_window = None;
        d.hkl
    });

    // SAFETY: the buffer holds the size passed.
    if unsafe { ImmGetIMEFileNameA(hkl, ime_file.as_mut_ptr(), (ime_file.len() - 1) as u32) } == 0 {
        return;
    }

    let len = ime_file.iter().position(|&c| c == 0).unwrap_or(0);
    let name = String::from_utf8_lossy(&ime_file[..len]).into_owned();
    let Ok(hime) = SharedObject::load(&name) else {
        return;
    };

    // SAFETY: the types match the IME exports.
    let (get_reading_string, show_reading_window) = unsafe {
        (
            hime.function::<GetReadingStringFn>("GetReadingString").ok(),
            hime.function::<ShowReadingWindowFn>("ShowReadingWindow")
                .ok(),
        )
    };
    // FIXME (upstream): the IME module is never unloaded.
    std::mem::forget(hime);
    let current = ime(videodata, |d| {
        d.get_reading_string = get_reading_string;
        d.show_reading_window = show_reading_window;
        d.hwnd_current
    });

    if let Some(show_reading_window) = show_reading_window {
        // SAFETY: current is a window handle; the context is released.
        unsafe {
            let himc = ImmGetContext(current);
            if !himc.is_null() {
                show_reading_window(himc, 0);
                ImmReleaseContext(current, himc);
            }
        }
    }
}

/// Translation of `IME_SetWindow()`.
fn ime_set_window(videodata: &VideoData, window: WindowID) {
    let hwnd = window_hwnd(window);

    ime(videodata, |d| {
        if hwnd != d.hwnd_current {
            d.hwnd_current = hwnd;
            d.composition_area = ZERO_COMPOSITIONFORM;
            d.candidate_area = ZERO_CANDIDATEFORM;
        }
    });

    let (rect, cursor) = text_input_area(window);
    ime_set_text_input_area(videodata, hwnd, &rect, cursor);
}

fn point_eq(a: &POINT, b: &POINT) -> bool {
    a.x == b.x && a.y == b.y
}
fn rect_eq(a: &RECT, b: &RECT) -> bool {
    a.left == b.left && a.top == b.top && a.right == b.right && a.bottom == b.bottom
}

/// Translation of `IME_SetTextInputArea()`.
fn ime_set_text_input_area(videodata: &VideoData, hwnd: HWND, rect: &Rect, cursor: i32) {
    // SAFETY: hwnd is a window handle (or null, for which IMM fails).
    let himc = unsafe { ImmGetContext(hwnd) };
    if !himc.is_null() {
        let mut font_height = rect.h;

        // SAFETY: LOGFONTW is plain data.
        let mut font: LOGFONTW = unsafe { std::mem::zeroed() };
        // SAFETY: himc is a valid input context.
        if unsafe { ImmGetCompositionFontW(himc, &mut font) } != 0 {
            font_height = font.lfHeight;
        }

        let mut cof = ZERO_COMPOSITIONFORM;
        cof.dwStyle = CFS_RECT;
        cof.ptCurrentPos.x = rect.x + cursor;
        cof.ptCurrentPos.y = rect.y + (rect.h - font_height) / 2;
        cof.rcArea.left = rect.x;
        cof.rcArea.right = rect.x + rect.w;
        cof.rcArea.top = rect.y;
        cof.rcArea.bottom = rect.y + rect.h;
        let changed = ime(videodata, |d| {
            let a = &d.composition_area;
            let same = a.dwStyle == cof.dwStyle
                && point_eq(&a.ptCurrentPos, &cof.ptCurrentPos)
                && rect_eq(&a.rcArea, &cof.rcArea);
            if !same {
                d.composition_area = cof;
            }
            !same
        });
        if changed {
            // SAFETY: himc is a valid input context.
            unsafe { ImmSetCompositionWindow(himc, &cof) };
        }

        let mut caf = ZERO_CANDIDATEFORM;
        caf.dwIndex = 0;
        caf.dwStyle = CFS_EXCLUDE;
        caf.ptCurrentPos.x = rect.x + cursor;
        caf.ptCurrentPos.y = rect.y;
        caf.rcArea.left = rect.x;
        caf.rcArea.right = rect.x + rect.w;
        caf.rcArea.top = rect.y;
        caf.rcArea.bottom = rect.y + rect.h;
        let changed = ime(videodata, |d| {
            let a = &d.candidate_area;
            let same = a.dwIndex == caf.dwIndex
                && a.dwStyle == caf.dwStyle
                && point_eq(&a.ptCurrentPos, &caf.ptCurrentPos)
                && rect_eq(&a.rcArea, &caf.rcArea);
            if !same {
                d.candidate_area = caf;
            }
            !same
        });
        if changed {
            // SAFETY: himc is a valid input context.
            unsafe { ImmSetCandidateWindow(himc, &caf) };
        }

        // SAFETY: himc came from ImmGetContext(hwnd).
        unsafe { ImmReleaseContext(hwnd, himc) };
    }
}

/// Translation of `IME_UpdateInputLocale()`.
fn ime_update_input_locale(videodata: &VideoData) {
    // SAFETY: GetKeyboardLayout has no preconditions.
    let hklnext = unsafe { GetKeyboardLayout(0) };

    ime(videodata, |d| {
        if hklnext == d.hkl {
            return;
        }

        d.hkl = hklnext;
        d.horizontal_candidates = primlang(d) == LANG_KOREAN || lang(d) == LANG_CHS;
        d.candlistindexbase = if hkl_bits(d.hkl) == CHT_HKL_DAYI {
            0
        } else {
            1
        };
    });
}

/// Translation of `IME_ClearComposition()`.
fn ime_clear_composition(videodata: &VideoData) {
    let (initialized, current) = ime(videodata, |d| (d.initialized, d.hwnd_current));
    if !initialized {
        return;
    }

    // SAFETY: current is a window handle (or null, for which IMM fails).
    let himc = unsafe { ImmGetContext(current) };
    if himc.is_null() {
        return;
    }

    let empty: [u16; 1] = [0];
    // SAFETY: himc is a valid input context; the strings are one WCHAR.
    unsafe {
        ImmNotifyIME(himc, NI_COMPOSITIONSTR, CPS_CANCEL, 0);
        ImmSetCompositionStringW(
            himc,
            SCS_SETSTR,
            empty.as_ptr().cast(),
            2,
            empty.as_ptr().cast(),
            2,
        );

        ImmNotifyIME(himc, NI_CLOSECANDIDATE, 0, 0);
        ImmReleaseContext(current, himc);
    }
    ime_send_clear_composition(videodata);
}

/// `wcslen()` of a WCHAR buffer.
fn wcslen(s: &[u16]) -> usize {
    s.iter().position(|&c| c == 0).unwrap_or(s.len())
}

/// Translation of `IME_GetCompositionString()`.
fn ime_get_composition_string(videodata: &VideoData, himc: HIMC, l_param: &LPARAM, string: u32) {
    let hkl = ime(videodata, |d| d.hkl);
    let lang = (hkl_bits(hkl) & 0xffff) as u32;

    if (*l_param as u32 & GCS_CURSORPOS) != 0 {
        // SAFETY: himc is a valid input context; nothing is written.
        let cursor =
            unsafe { ImmGetCompositionStringW(himc, GCS_CURSORPOS, std::ptr::null_mut(), 0) };
        ime(videodata, |d| d.cursor = (cursor as u32 & 0xffff) as i32);
    }
    ime(videodata, |d| {
        d.selected_start = 0;
        d.selected_length = 0;
    });

    // SAFETY: himc is a valid input context; nothing is written.
    let mut length = unsafe { ImmGetCompositionStringW(himc, string, std::ptr::null_mut(), 0) };
    let (mut composition, composition_length) = ime(videodata, |d| {
        if length > 0 && d.composition_length < length {
            d.composition = vec![0; length as usize / 2 + 1];
            d.composition_length = length;
        }
        (std::mem::take(&mut d.composition), d.composition_length)
    });

    // SAFETY: the buffer holds composition_length bytes (plus a WCHAR).
    length = unsafe {
        ImmGetCompositionStringW(
            himc,
            string,
            composition.as_mut_ptr().cast(),
            composition_length as u32,
        )
    };
    if length < 0 {
        length = 0;
    }
    length /= 2;

    ime(videodata, |d| {
        if (*l_param as u32 & GCS_CURSORPOS) == 0 {
            // If the IME doesn't support GCS_CURSORPOS, default the cursor to the end of the composition.
            d.cursor = length;
        }

        if (lang == LANG_CHT || lang == LANG_CHS)
            && d.cursor > 0
            && d.cursor < composition_length / 2
            && (composition[0] == 0x3000 || composition[0] == 0x0020)
        {
            // Traditional Chinese IMEs add a placeholder U+3000
            // Simplified Chinese IMEs seem to add a placeholder U+0020 sometimes
            for i in (d.cursor + 1)..length {
                composition[(i - 1) as usize] = composition[i as usize];
            }
            length -= 1;
        }

        composition[length.max(0) as usize] = 0;
        d.composition = composition;
    });

    // SAFETY: himc is a valid input context; nothing is written.
    let length = unsafe { ImmGetCompositionStringW(himc, GCS_COMPATTR, std::ptr::null_mut(), 0) };
    if length > 0 {
        let mut attributes = vec![0u8; length as usize];
        // SAFETY: the buffer holds `length` bytes.
        let mut length = unsafe {
            ImmGetCompositionStringW(
                himc,
                GCS_COMPATTR,
                attributes.as_mut_ptr().cast(),
                length as u32,
            )
        };
        if length < 0 {
            length = 0;
        }
        let attributes = &attributes[..length as usize];

        let is_target =
            |a: u8| a as u32 == ATTR_TARGET_CONVERTED || a as u32 == ATTR_TARGET_NOTCONVERTED;
        let start = attributes
            .iter()
            .position(|&a| is_target(a))
            .unwrap_or(attributes.len());
        let end = attributes[start..]
            .iter()
            .position(|&a| !is_target(a))
            .map_or(attributes.len(), |e| start + e);

        if end > start {
            ime(videodata, |d| {
                d.selected_start = start as i32;
                d.selected_length = (end - start) as i32;
            });
        }
    }
}

/// Translation of `IME_SendInputEvent()`.
fn ime_send_input_event(videodata: &VideoData) {
    let s = ime(videodata, |d| wide_to_utf8(&d.composition));
    keyboard::send_keyboard_text(&s);

    ime(videodata, |d| {
        if let Some(c) = d.composition.first_mut() {
            *c = 0;
        }
        d.readingstring[0] = 0;
        d.cursor = 0;
    });
}

/// Translation of `IME_SendEditingEvent()`.
fn ime_send_editing_event(videodata: &VideoData) {
    let (buffer, reading_len, cursor, selected_start, selected_length) = ime(videodata, |d| {
        let comp_len = wcslen(&d.composition);
        let mut buffer: Vec<u16>;
        if d.readingstring[0] != 0 {
            let len = comp_len.min(d.cursor.max(0) as usize);

            buffer = d.composition[..len].to_vec();
            buffer.extend_from_slice(&d.readingstring[..wcslen(&d.readingstring)]);
            buffer.extend_from_slice(&d.composition[len..comp_len]);
        } else {
            buffer = d.composition[..comp_len].to_vec();
        }
        (
            buffer,
            wcslen(&d.readingstring),
            d.cursor,
            d.selected_start,
            d.selected_length,
        )
    });

    let s = wide_to_utf8(&buffer);
    if reading_len != 0 {
        keyboard::send_editing_text(&s, cursor, reading_len as i32);
    } else if cursor == selected_start {
        keyboard::send_editing_text(&s, selected_start, selected_length);
    } else {
        keyboard::send_editing_text(&s, cursor, 0);
    }
    if !s.is_empty() {
        ime(videodata, |d| d.needs_clear_composition = true);
    }
}

/// Translation of `IME_SendClearComposition()`.
fn ime_send_clear_composition(videodata: &VideoData) {
    let needs = ime(videodata, |d| {
        std::mem::take(&mut d.needs_clear_composition)
    });
    if needs {
        keyboard::send_editing_text("", 0, 0);
    }
}

/// Translation of `IME_OpenCandidateList()`.
fn ime_open_candidate_list(d: &mut ImeData) -> bool {
    d.candidates_open = true;
    d.candcount = 0;
    true
}

/// Translation of `IME_AddCandidate()`.
fn ime_add_candidate(d: &mut ImeData, i: u32, candidate: &[u16]) {
    let candidate_utf8 = wide_to_utf8(candidate);
    d.candidates[i as usize] = Some(format!(
        "{} {}",
        (i as i32 + d.candlistindexbase) % 10,
        candidate_utf8
    ));

    d.candcount = i as i32 + 1;
}

/// Translation of `IME_SendCandidateList()`.
fn ime_send_candidate_list(videodata: &VideoData) {
    let (candidates, sel, horizontal) = ime(videodata, |d| {
        let list: Vec<String> = d.candidates[..d.candcount as usize]
            .iter()
            .map(|c| c.clone().unwrap_or_default())
            .collect();
        (list, d.candsel, d.horizontal_candidates)
    });
    keyboard::send_editing_text_candidates(&candidates, sel as i32, horizontal);
}

/// Translation of `IME_CloseCandidateList()`.
fn ime_close_candidate_list(videodata: &VideoData) {
    let had = ime(videodata, |d| {
        d.candidates_open = false;

        if d.candcount > 0 {
            for i in 0..d.candcount as usize {
                d.candidates[i] = None;
            }
            d.candcount = 0;
            true
        } else {
            false
        }
    });
    if had {
        keyboard::send_editing_text_candidates(&[], -1, false);
    }
}

/// The NUL-terminated WCHAR string at byte offset `off` of a candidate list.
fn candidate_at(bytes: &[u8], off: usize) -> Vec<u16> {
    bytes
        .get(off..)
        .unwrap_or(&[])
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .take_while(|&c| c != 0)
        .collect()
}

/// Translation of `IME_GetCandidateList()`.
fn ime_get_candidate_list(videodata: &VideoData, hwnd: HWND) {
    let mut has_candidates = false;

    // SAFETY: hwnd is a window handle (or null, for which IMM fails).
    let himc = unsafe { ImmGetContext(hwnd) };
    if !himc.is_null() {
        // SAFETY: himc is a valid input context; nothing is written.
        let size = unsafe { ImmGetCandidateListW(himc, 0, std::ptr::null_mut(), 0) };
        if size != 0 {
            // (u32s, for the alignment of CANDIDATELIST)
            let mut storage = vec![0u32; (size as usize).div_ceil(4)];
            // SAFETY: the buffer holds `size` bytes.
            let size = unsafe { ImmGetCandidateListW(himc, 0, storage.as_mut_ptr().cast(), size) };
            if size as usize >= size_of::<CANDIDATELIST>() {
                let bytes: Vec<u8> = storage.iter().flat_map(|w| w.to_ne_bytes()).collect();
                let field = |i: usize| storage[i];
                let dw_count = field(2);
                let dw_selection = field(3);
                let dw_page_size = field(5);
                let offset = |i: u32| storage.get(6 + i as usize).copied().unwrap_or(0) as usize;

                let lang_chs = ime(videodata, |d| lang(d) == LANG_CHS);
                let ime_id = if lang_chs {
                    ime_get_id(videodata, 0)
                } else {
                    0
                };
                ime(videodata, |d| {
                    if ime_open_candidate_list(d) {
                        let mut i: u32;
                        let mut page_start: u32 = 0;
                        let page_size: u32;

                        d.candsel = dw_selection;

                        if lang_chs && ime_id != 0 {
                            let maxcandchar = 18usize;
                            let mut cchars = 0usize;

                            i = 0;
                            while i < dw_count {
                                let len = candidate_at(&bytes, offset(i)).len() + 1;
                                if len + cchars > maxcandchar {
                                    if i > dw_selection {
                                        break;
                                    }

                                    page_start = i;
                                    cchars = len;
                                } else {
                                    cchars += len;
                                }
                                i += 1;
                            }
                            page_size = i - page_start;
                        } else {
                            page_size = (if dw_page_size == 0 {
                                MAX_CANDLIST as u32
                            } else {
                                dw_page_size
                            })
                            .min(MAX_CANDLIST as u32);
                            page_start = (dw_selection / page_size) * page_size;
                        }
                        i = page_start;
                        let mut j = 0u32;
                        while i < dw_count && j < page_size {
                            let candidate = candidate_at(&bytes, offset(i));
                            ime_add_candidate(d, j, &candidate);
                            i += 1;
                            j += 1;
                        }

                        has_candidates = true;
                    }
                });
                if has_candidates {
                    ime_send_candidate_list(videodata);
                }
            }
        }
        // SAFETY: himc came from ImmGetContext(hwnd).
        unsafe { ImmReleaseContext(hwnd, himc) };
    }

    if !has_candidates {
        ime_close_candidate_list(videodata);
    }
}

/// Let the IME handle a window message; `true` traps it. Translation of
/// `WIN_HandleIMEMessage()`.
pub(crate) fn handle_ime_message(
    hwnd: HWND,
    msg: u32,
    w_param: WPARAM,
    l_param: &mut LPARAM,
    videodata: &VideoData,
) -> bool {
    let mut trap = false;

    if msg == WM_IME_SETCONTEXT {
        let mut element_mask = ISC_SHOWUIALL as LPARAM;
        let (internal_composition, internal_candidates) = ime(videodata, |d| {
            (d.internal_composition, d.internal_candidates)
        });
        if internal_composition {
            element_mask &= !(ISC_SHOWUICOMPOSITIONWINDOW as LPARAM);
        }
        if internal_candidates {
            element_mask &= !(ISC_SHOWUIALLCANDIDATEWINDOW as LPARAM);
        }
        *l_param &= element_mask;

        return false;
    } else if msg == WM_IME_STARTCOMPOSITION {
        let internal_composition = ime(videodata, |d| {
            if d.internal_composition {
                d.cursor = 0;
            }
            d.internal_composition
        });
        if internal_composition {
            // Windows may still display a composition dialog even with
            // ISC_SHOWUICOMPOSITIONWINDOW cleared, so trap the message
            // here to prevent that (even when the IME is disabled).
            return true;
        }
    }

    let (initialized, available, enabled, internal_composition, internal_candidates) =
        ime(videodata, |d| {
            (
                d.initialized,
                d.available,
                d.enabled,
                d.internal_composition,
                d.internal_candidates,
            )
        });
    if !initialized || !available || !enabled {
        return false;
    }

    match msg {
        WM_KEYDOWN | WM_SYSKEYDOWN => {
            if w_param == VK_PROCESSKEY as WPARAM {
                trap = true;
            }
        }
        WM_INPUTLANGCHANGE => {
            ime_input_lang_changed(videodata);
        }
        WM_IME_COMPOSITION => {
            if internal_composition {
                trap = true;
                // SAFETY: hwnd is the window receiving the message.
                let himc = unsafe { ImmGetContext(hwnd) };
                if (*l_param as u32 & GCS_RESULTSTR) != 0 {
                    ime_get_composition_string(videodata, himc, l_param, GCS_RESULTSTR);
                    ime_send_clear_composition(videodata);
                    ime_send_input_event(videodata);
                }
                if (*l_param as u32 & GCS_COMPSTR) != 0 {
                    ime(videodata, |d| d.readingstring[0] = 0);
                    ime_get_composition_string(videodata, himc, l_param, GCS_COMPSTR);
                    ime_send_editing_event(videodata);
                }
                // SAFETY: himc came from ImmGetContext(hwnd).
                unsafe { ImmReleaseContext(hwnd, himc) };
            }
        }
        WM_IME_ENDCOMPOSITION => {
            if internal_composition {
                trap = true;
                ime(videodata, |d| {
                    if let Some(c) = d.composition.first_mut() {
                        *c = 0;
                    }
                    d.readingstring[0] = 0;
                    d.cursor = 0;
                    d.selected_start = 0;
                    d.selected_length = 0;
                });
                ime_send_clear_composition(videodata);
            }
        }
        WM_IME_NOTIFY => match w_param as u32 {
            IMN_SETCOMPOSITIONWINDOW | IMN_SETCOMPOSITIONFONT | IMN_SETCANDIDATEPOS => {}
            IMN_SETCONVERSIONMODE | IMN_SETOPENSTATUS => {
                ime_update_input_locale(videodata);
            }
            IMN_OPENCANDIDATE | IMN_CHANGECANDIDATE => {
                if internal_candidates {
                    trap = true;
                    ime(videodata, |d| d.update_candidates = true);
                }
            }
            IMN_CLOSECANDIDATE => {
                if internal_candidates {
                    trap = true;
                    ime(videodata, |d| d.update_candidates = false);
                    ime_close_candidate_list(videodata);
                }
            }
            IMN_PRIVATE => {
                let id = ime_get_id(videodata, 0);
                ime_get_reading_string(videodata, hwnd);
                match id {
                    IMEID_CHT_VER42 | IMEID_CHT_VER43 | IMEID_CHT_VER44 | IMEID_CHS_VER41
                    | IMEID_CHS_VER42 => {
                        if *l_param == 1 || *l_param == 2 {
                            trap = true;
                        }
                    }
                    IMEID_CHT_VER50 | IMEID_CHT_VER51 | IMEID_CHT_VER52 | IMEID_CHT_VER60
                    | IMEID_CHS_VER53 => {
                        if matches!(*l_param, 16 | 17 | 26 | 27 | 28) {
                            trap = true;
                        }
                    }
                    _ => {}
                }
            }
            _ => {
                trap = true;
            }
        },
        _ => {}
    }
    trap
}

/// Translation of `WIN_UpdateIMECandidates()`.
pub(crate) fn update_ime_candidates(videodata: &VideoData) {
    let (update, current) = ime(videodata, |d| (d.update_candidates, d.hwnd_current));
    if update {
        ime_get_candidate_list(videodata, current);
        ime(videodata, |d| d.update_candidates = false);
    }
}
