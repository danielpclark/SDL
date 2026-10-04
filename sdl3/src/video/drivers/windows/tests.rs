// Tests of the Windows video driver (run natively or under Wine).
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Each test starts the `windows` driver; with no desktop to connect to,
//! it prints a note and returns.

use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{HWND, POINT, RECT};
use windows_sys::Win32::Graphics::Gdi::{
    BitBlt, ClientToScreen, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC,
    GetDIBits, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
    SRCCOPY,
};
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, SetClipboardData,
};
use windows_sys::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows_sys::Win32::System::Ole::CF_UNICODETEXT;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_A, VK_RIGHT};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetClientRect, GetIconInfo, GetSystemMetrics, GetWindowTextW, IsWindow, IsWindowVisible,
    PostMessageW, ICONINFO, SM_CXSCREEN, SM_CYSCREEN, WM_CHAR, WM_KEYDOWN, WM_KEYUP,
    WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE,
};

use super::clipboard::check_clipboard_update;
use super::events::windows_scan_code_to_sdl_scan_code;
use super::mouse::current_cursor;
use super::video_data;
use crate::events::keyboard::Scancode;
use crate::events::mouse::{self, CursorFrame, SystemCursor, BUTTON_LEFT};
use crate::events::queue::{get_events, pump};
use crate::events::window::WindowFlags;
use crate::events::{Event, EventType};
use crate::hints;
use crate::init::{self, InitFlags};

use crate::video::clipboard;
use crate::video::display::{
    desktop_display_mode, display_bounds, display_content_scale, display_name, display_properties,
    display_usable_bounds, displays, fullscreen_display_modes, primary_display,
    PROP_DISPLAY_WINDOWS_HMONITOR_POINTER,
};
use crate::video::window::{Window, PROP_WINDOW_WIN32_HWND_POINTER};
use crate::video::{current_video_driver, PixelFormat, Rect, Surface};

/// Quits video when a test ends (even by panicking).
struct Session;

impl Drop for Session {
    fn drop(&mut self) {
        init::quit();
    }
}

/// Start the windows driver, or `None` (with a note) if there's no desktop.
fn start() -> Option<Session> {
    init::quit(); // (in case an earlier test failed halfway)
    hints::set(hints::VIDEO_DRIVER, "windows").unwrap();
    let result = init::init(InitFlags::VIDEO);
    hints::reset(hints::VIDEO_DRIVER);
    match result {
        Ok(()) => {
            let session = Session;
            pump();
            let _ = get_events(EventType::FIRST, EventType::LAST, 100000);
            Some(session)
        }
        Err(e) => {
            println!("note: no desktop for the windows video driver, skipping ({e})");
            None
        }
    }
}

/// Pump events until `done` is satisfied by the events seen so far (or a
/// few seconds pass); all the events seen.
fn pump_until(done: impl Fn(&[Event]) -> bool) -> Vec<Event> {
    let start = Instant::now();
    let mut seen = Vec::new();
    loop {
        pump();
        seen.extend(get_events(EventType::FIRST, EventType::LAST, 100000).unwrap());
        if done(&seen) || start.elapsed() > Duration::from_secs(5) {
            return seen;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn has_window_event(events: &[Event], ty: EventType) -> bool {
    events
        .iter()
        .any(|e| matches!(e, Event::Window(w) if w.event_type == ty))
}

fn hwnd_of(window: &Window) -> HWND {
    let props = window.properties().unwrap();
    props
        .get_number(PROP_WINDOW_WIN32_HWND_POINTER)
        .unwrap_or(0) as HWND
}

fn client_size(hwnd: HWND) -> (i32, i32) {
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    // SAFETY: hwnd is a live window.
    unsafe { GetClientRect(hwnd, &mut rect) };
    (rect.right - rect.left, rect.bottom - rect.top)
}

fn client_origin(hwnd: HWND) -> (i32, i32) {
    let mut pt = POINT { x: 0, y: 0 };
    // SAFETY: hwnd is a live window.
    unsafe { ClientToScreen(hwnd, &mut pt) };
    (pt.x, pt.y)
}

fn window_text(hwnd: HWND) -> String {
    let mut buf = [0u16; 256];
    // SAFETY: the buffer holds 256 WCHARs.
    let n = unsafe { GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

#[test]
fn driver_init_displays_and_modes() {
    let _l = crate::test_support::test_lock();
    let Some(_session) = start() else { return };

    assert_eq!(current_video_driver().unwrap(), "windows");

    let ds = displays().unwrap();
    assert!(!ds.is_empty());
    let primary = primary_display().unwrap();
    assert!(ds.contains(&primary));
    for &d in &ds {
        let bounds = display_bounds(d).unwrap();
        assert!(bounds.w > 0 && bounds.h > 0, "{bounds:?}");
        let usable = display_usable_bounds(d).unwrap();
        assert!(usable.w > 0 && usable.h > 0 && usable.w <= bounds.w && usable.h <= bounds.h);
        assert!(!display_name(d).unwrap().is_empty());
        assert!(display_content_scale(d).unwrap() > 0.0);

        let desktop = desktop_display_mode(d).unwrap();
        assert!(desktop.w > 0 && desktop.h > 0 && desktop.format != PixelFormat::UNKNOWN);
        assert!(desktop.refresh_rate >= 0.0);
        let modes = fullscreen_display_modes(d).unwrap();
        assert!(!modes.is_empty());
        for mode in &modes {
            assert!(mode.w > 0 && mode.h > 0);
        }

        let hmonitor = display_properties(d)
            .unwrap()
            .get_number(PROP_DISPLAY_WINDOWS_HMONITOR_POINTER)
            .unwrap_or(0);
        assert_ne!(hmonitor, 0);
    }

    // The primary display is the one at the origin, of the screen's size
    let bounds = display_bounds(primary).unwrap();
    // SAFETY: GetSystemMetrics has no preconditions.
    let (sw, sh) = unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) };
    assert_eq!(bounds, Rect::new(0, 0, sw, sh));
}

#[test]
fn window_lifecycle_and_events() {
    let _l = crate::test_support::test_lock();
    let Some(_session) = start() else { return };

    let window = Window::create("SDL window test", 200, 150, WindowFlags::HIDDEN).unwrap();
    let id = window.id();
    let hwnd = hwnd_of(&window);
    // SAFETY: IsWindow/IsWindowVisible accept any handle.
    unsafe {
        assert_ne!(IsWindow(hwnd), 0);
        assert_eq!(IsWindowVisible(hwnd), 0);
    }
    assert_eq!(window_text(hwnd), "SDL window test");
    assert_eq!(client_size(hwnd), (200, 150));

    window.show().unwrap();
    let events = pump_until(|e| has_window_event(e, EventType::WINDOW_SHOWN));
    assert!(
        has_window_event(&events, EventType::WINDOW_SHOWN),
        "{events:?}"
    );
    // SAFETY: as above.
    assert_ne!(unsafe { IsWindowVisible(hwnd) }, 0);
    assert!(!window.flags().unwrap().contains(WindowFlags::HIDDEN));

    window.set_position(50, 60).unwrap();
    let events = pump_until(|e| {
        e.iter().any(|e| matches!(e, Event::Window(w) if w.event_type == EventType::WINDOW_MOVED && w.data1 == 50 && w.data2 == 60))
    });
    assert!(
        events.iter().any(|e| matches!(e, Event::Window(w)
            if w.event_type == EventType::WINDOW_MOVED && w.window_id == id && (w.data1, w.data2) == (50, 60))),
        "{events:?}"
    );
    assert_eq!(client_origin(hwnd), (50, 60));
    assert_eq!(window.position().unwrap(), (50, 60));

    window.set_size(320, 240).unwrap();
    let events = pump_until(|e| {
        e.iter().any(|e| matches!(e, Event::Window(w) if w.event_type == EventType::WINDOW_RESIZED && w.data1 == 320))
    });
    assert!(
        events.iter().any(|e| matches!(e, Event::Window(w)
            if w.event_type == EventType::WINDOW_RESIZED && (w.data1, w.data2) == (320, 240))),
        "{events:?}"
    );
    assert_eq!(client_size(hwnd), (320, 240));
    assert_eq!(window.size().unwrap(), (320, 240));

    window.set_title("Renamed ünïcödé").unwrap();
    assert_eq!(window_text(hwnd), "Renamed ünïcödé");

    window.hide().unwrap();
    let events = pump_until(|e| has_window_event(e, EventType::WINDOW_HIDDEN));
    assert!(
        has_window_event(&events, EventType::WINDOW_HIDDEN),
        "{events:?}"
    );
    // SAFETY: as above.
    assert_eq!(unsafe { IsWindowVisible(hwnd) }, 0);

    window.destroy();
    // SAFETY: as above.
    assert_eq!(unsafe { IsWindow(hwnd) }, 0);
}

/// The 32-bit pixels of the window's client area, read with BitBlt() and
/// GetDIBits().
fn read_client_pixels(hwnd: HWND, w: i32, h: i32) -> Vec<u32> {
    let mut pixels = vec![0u32; (w * h) as usize];
    // SAFETY: the GDI objects are created, used and released in order; the
    // buffer holds w * h pixels.
    unsafe {
        let hdc = GetDC(hwnd);
        let mdc = CreateCompatibleDC(hdc);
        let mut info: BITMAPINFO = std::mem::zeroed();
        info.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
        info.bmiHeader.biWidth = w;
        info.bmiHeader.biHeight = -h; // top-down
        info.bmiHeader.biPlanes = 1;
        info.bmiHeader.biBitCount = 32;
        info.bmiHeader.biCompression = BI_RGB;
        let mut bits = std::ptr::null_mut();
        let hbm = CreateDIBSection(
            hdc,
            &info,
            DIB_RGB_COLORS,
            &mut bits,
            std::ptr::null_mut(),
            0,
        );
        let old = SelectObject(mdc, hbm);
        BitBlt(mdc, 0, 0, w, h, hdc, 0, 0, SRCCOPY);
        SelectObject(mdc, old);
        GetDIBits(
            mdc,
            hbm,
            0,
            h as u32,
            pixels.as_mut_ptr().cast(),
            &mut info,
            DIB_RGB_COLORS,
        );
        DeleteObject(hbm);
        DeleteDC(mdc);
        ReleaseDC(hwnd, hdc);
    }
    pixels
}

#[test]
fn framebuffer_update_and_readback() {
    let _l = crate::test_support::test_lock();
    let Some(_session) = start() else { return };

    let window = Window::create("SDL framebuffer test", 64, 48, WindowFlags::BORDERLESS).unwrap();
    window.set_position(10, 10).unwrap();
    window.raise().unwrap();
    let _ = pump_until(|e| has_window_event(e, EventType::WINDOW_EXPOSED));
    let hwnd = hwnd_of(&window);

    let surface = window.surface().unwrap();
    {
        let mut s = surface.lock();
        assert_eq!((s.width(), s.height()), (64, 48));
        let red = s.map_rgb(0xFF, 0x00, 0x00);
        let blue = s.map_rgb(0x00, 0x00, 0xFF);
        s.fill_rect(None, red).unwrap();
        s.fill_rect(Some(&Rect::new(32, 0, 32, 48)), blue).unwrap();
    }
    surface.update().unwrap();
    pump();

    let pixels = read_client_pixels(hwnd, 64, 48);
    let at = |x: i32, y: i32| pixels[(y * 64 + x) as usize] & 0x00FF_FFFF;
    assert_eq!(at(5, 5), 0x00FF_0000, "left half is red");
    assert_eq!(at(40, 30), 0x0000_00FF, "right half is blue");

    // Only the updated rectangle changes
    {
        let mut s = surface.lock();
        let green = s.map_rgb(0x00, 0xFF, 0x00);
        s.fill_rect(None, green).unwrap();
    }
    surface.update_rects(&[Rect::new(0, 0, 16, 16)]).unwrap();
    pump();
    let pixels = read_client_pixels(hwnd, 64, 48);
    let at = |x: i32, y: i32| pixels[(y * 64 + x) as usize] & 0x00FF_FFFF;
    assert_eq!(at(4, 4), 0x0000_FF00);
    assert_eq!(at(20, 20), 0x00FF_0000);
    assert_eq!(at(40, 30), 0x0000_00FF);

    window.destroy();
}

/// The `CF_UNICODETEXT` text on the clipboard, read directly.
fn raw_clipboard_text() -> Option<String> {
    // SAFETY: the clipboard is opened and closed around the read; the
    // locked data is a NUL-terminated UTF-16 string.
    unsafe {
        if OpenClipboard(std::ptr::null_mut()) == 0 {
            return None;
        }
        let h = GetClipboardData(CF_UNICODETEXT as u32);
        let mut text = None;
        if !h.is_null() {
            let p = GlobalLock(h).cast::<u16>();
            if !p.is_null() {
                let mut n = 0;
                while *p.add(n) != 0 {
                    n += 1;
                }
                text = Some(String::from_utf16_lossy(std::slice::from_raw_parts(p, n)));
                GlobalUnlock(h);
            }
        }
        CloseClipboard();
        text
    }
}

/// Put `CF_UNICODETEXT` text on the clipboard directly.
fn set_raw_clipboard_text(text: &str) {
    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: the clipboard is opened and closed around the write; it takes
    // ownership of the filled memory block.
    unsafe {
        assert_ne!(OpenClipboard(std::ptr::null_mut()), 0);
        EmptyClipboard();
        let h = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2);
        let p = GlobalLock(h).cast::<u16>();
        std::ptr::copy_nonoverlapping(wide.as_ptr(), p, wide.len());
        GlobalUnlock(h);
        SetClipboardData(CF_UNICODETEXT as u32, h);
        CloseClipboard();
    }
}

#[test]
fn clipboard_text_round_trip() {
    let _l = crate::test_support::test_lock();
    let Some(_session) = start() else { return };
    let _window = Window::create("SDL clipboard test", 32, 32, WindowFlags::HIDDEN).unwrap();

    clipboard::set_clipboard_text("Hello from SDL, ünïcödé").unwrap();
    assert!(clipboard::has_clipboard_text().unwrap());
    assert_eq!(
        clipboard::clipboard_text().unwrap(),
        "Hello from SDL, ünïcödé"
    );
    assert_eq!(
        raw_clipboard_text().as_deref(),
        Some("Hello from SDL, ünïcödé")
    );

    // Line feeds become CRLF on the Windows clipboard
    clipboard::set_clipboard_text("line 1\nline 2\r\nline 3").unwrap();
    assert_eq!(
        raw_clipboard_text().as_deref(),
        Some("line 1\r\nline 2\r\nline 3")
    );
    assert_eq!(
        clipboard::clipboard_text().unwrap(),
        "line 1\r\nline 2\r\nline 3"
    );

    // Text another application put there, seen when a window gains focus
    set_raw_clipboard_text("from elsewhere");
    assert_eq!(clipboard::clipboard_text().unwrap(), "from elsewhere");
    let _ = get_events(EventType::FIRST, EventType::LAST, 100000);
    check_clipboard_update(&video_data().unwrap());
    let events = get_events(EventType::FIRST, EventType::LAST, 100000).unwrap();
    let update = events.iter().find_map(|e| match e {
        Event::Clipboard(c) if !c.owner => Some(c.clone()),
        _ => None,
    });
    let update = update.expect("a clipboard update event");
    assert!(update
        .mime_types
        .iter()
        .any(|m| m == "text/plain;charset=utf-8"));

    clipboard::clear_clipboard_data().unwrap();
}

#[test]
fn cursors() {
    let _l = crate::test_support::test_lock();
    let Some(_session) = start() else { return };
    // (without mouse focus, the default cursor is shown)
    let window = Window::create("SDL cursor test", 64, 64, WindowFlags::HIDDEN).unwrap();
    mouse::set_mouse_focus(Some(window.id()));

    for i in 0..SystemCursor::COUNT as i32 {
        let id = SystemCursor::from_index(i).unwrap();
        let cursor = mouse::create_system_cursor(id).unwrap();
        mouse::set_cursor(Some(&cursor)).unwrap();
        assert!(!current_cursor().is_null(), "{id:?}");
    }

    // A color cursor is built as an animated cursor resource of one frame
    let mut image = Surface::new(32, 32, PixelFormat::ARGB8888).unwrap();
    image
        .fill_rect(Some(&Rect::new(8, 8, 16, 16)), 0xFF00FF00)
        .unwrap();
    let color = mouse::create_color_cursor(&image, 3, 4).unwrap();
    mouse::set_cursor(Some(&color)).unwrap();
    let hcursor = current_cursor();
    assert!(!hcursor.is_null());
    // SAFETY: plain data, filled by GetIconInfo; its bitmaps are released.
    unsafe {
        let mut info: ICONINFO = std::mem::zeroed();
        assert_ne!(GetIconInfo(hcursor, &mut info), 0);
        assert_eq!(info.fIcon, 0, "a cursor, not an icon");
        assert_eq!((info.xHotspot, info.yHotspot), (3, 4));
        if !info.hbmColor.is_null() {
            DeleteObject(info.hbmColor);
        }
        if !info.hbmMask.is_null() {
            DeleteObject(info.hbmMask);
        }
    }

    // An animated cursor
    let mut second = Surface::new(32, 32, PixelFormat::ARGB8888).unwrap();
    second.fill_rect(None, 0xFFFF0000).unwrap();
    let frames = [
        CursorFrame {
            surface: &image,
            duration: 100,
        },
        CursorFrame {
            surface: &second,
            duration: 100,
        },
    ];
    let animated = mouse::create_animated_cursor(&frames, 0, 0).unwrap();
    mouse::set_cursor(Some(&animated)).unwrap();
    assert!(!current_cursor().is_null());
    assert_ne!(current_cursor(), hcursor);

    // A monochrome cursor
    let data = [0xFFu8; 4 * 32];
    let mask = [0xFFu8; 4 * 32];
    let mono = mouse::create_cursor(&data, &mask, 32, 32, 0, 0).unwrap();
    mouse::set_cursor(Some(&mono)).unwrap();
    assert!(!current_cursor().is_null());

    mouse::hide_cursor();
    assert!(current_cursor().is_null());
    mouse::show_cursor();
    assert!(!current_cursor().is_null());

    mouse::set_mouse_focus(None);
    window.destroy();
}

/// `lParam` of a key message: repeat count 1, the scan code and the
/// extended, previous-state and transition bits.
fn key_lparam(scancode: u32, extended: bool, up: bool) -> isize {
    let mut l = 1 | (scancode << 16);
    if extended {
        l |= 1 << 24;
    }
    if up {
        l |= (1 << 30) | (1 << 31);
    }
    l as i32 as isize
}

#[test]
fn keyboard_scancodes() {
    let _l = crate::test_support::test_lock();
    let Some(_session) = start() else { return };

    // The mapping itself
    let (code, raw, _) =
        windows_scan_code_to_sdl_scan_code(key_lparam(0x1E, false, false), VK_A as usize);
    assert_eq!((code, raw), (Scancode::A, 0x1E));
    let (code, raw, _) =
        windows_scan_code_to_sdl_scan_code(key_lparam(0x4D, true, false), VK_RIGHT as usize);
    assert_eq!((code, raw), (Scancode::RIGHT, 0xE04D));

    // Key messages through the window procedure and the event queue
    let window = Window::create("SDL keyboard test", 64, 64, WindowFlags::default()).unwrap();
    let hwnd = hwnd_of(&window);
    let _ = pump_until(|e| has_window_event(e, EventType::WINDOW_SHOWN));
    let _ = get_events(EventType::FIRST, EventType::LAST, 100000);

    let keys = [
        (VK_A, 0x1E, false, Scancode::A),
        (VK_RIGHT, 0x4D, true, Scancode::RIGHT),
    ];
    for &(vk, sc, ext, expected) in &keys {
        // SAFETY: hwnd is a live window of this thread.
        unsafe {
            PostMessageW(hwnd, WM_KEYDOWN, vk as usize, key_lparam(sc, ext, false));
            PostMessageW(hwnd, WM_KEYUP, vk as usize, key_lparam(sc, ext, true));
        }
        let events = pump_until(|e| {
            e.iter()
                .any(|e| matches!(e, Event::Key(k) if !k.down && k.scancode == expected))
        });
        let keys: Vec<(Scancode, bool)> = events
            .iter()
            .filter_map(|e| match e {
                Event::Key(k) => Some((k.scancode, k.down)),
                _ => None,
            })
            .collect();
        assert_eq!(
            keys,
            vec![(expected, true), (expected, false)],
            "{events:?}"
        );
    }

    window.destroy();
}

#[test]
fn mouse_messages() {
    let _l = crate::test_support::test_lock();
    let Some(_session) = start() else { return };

    let window = Window::create("SDL mouse test", 100, 100, WindowFlags::default()).unwrap();
    let hwnd = hwnd_of(&window);
    let _ = pump_until(|e| has_window_event(e, EventType::WINDOW_SHOWN));
    let _ = get_events(EventType::FIRST, EventType::LAST, 100000);

    let lparam = |x: i32, y: i32| ((y << 16) | (x & 0xFFFF)) as isize;
    // SAFETY: hwnd is a live window of this thread.
    unsafe {
        PostMessageW(hwnd, WM_MOUSEMOVE, 0, lparam(20, 30));
        PostMessageW(hwnd, WM_LBUTTONDOWN, 1, lparam(20, 30));
        PostMessageW(hwnd, WM_LBUTTONUP, 0, lparam(20, 30));
    }
    let events = pump_until(|e| {
        e.iter()
            .any(|e| matches!(e, Event::MouseButton(b) if !b.down))
    });
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::MouseMotion(m) if (m.x, m.y) == (20.0, 30.0))),
        "{events:?}"
    );
    let buttons: Vec<(u8, bool)> = events
        .iter()
        .filter_map(|e| match e {
            Event::MouseButton(b) => Some((b.button, b.down)),
            _ => None,
        })
        .collect();
    assert_eq!(
        buttons,
        vec![(BUTTON_LEFT, true), (BUTTON_LEFT, false)],
        "{events:?}"
    );

    window.destroy();
}

#[test]
fn warping_and_relative_mode() {
    let _l = crate::test_support::test_lock();
    let Some(_session) = start() else { return };

    // Global warps move the system cursor
    mouse::warp_mouse_global(30.0, 40.0).unwrap();
    let (x, y, _) = mouse::global_mouse_state();
    assert_eq!((x, y), (30.0, 40.0));

    let window =
        Window::create("SDL relative mouse test", 100, 100, WindowFlags::default()).unwrap();
    let _ = pump_until(|e| has_window_event(e, EventType::WINDOW_SHOWN));

    // Relative mode starts the raw input thread (and stops it again)
    window.set_relative_mouse_mode(true).unwrap();
    assert!(window.relative_mouse_mode().unwrap());
    for _ in 0..10 {
        pump();
        std::thread::sleep(Duration::from_millis(10));
    }
    window.set_relative_mouse_mode(false).unwrap();
    assert!(!window.relative_mouse_mode().unwrap());

    window.destroy();
}

#[test]
fn text_input() {
    let _l = crate::test_support::test_lock();
    let Some(_session) = start() else { return };

    let window = Window::create("SDL text input test", 100, 100, WindowFlags::default()).unwrap();
    let hwnd = hwnd_of(&window);
    let _ = pump_until(|e| has_window_event(e, EventType::WINDOW_SHOWN));
    crate::events::keyboard::set_keyboard_focus(Some(window.id())).unwrap();

    // (this sets up the input method context)
    window.start_text_input().unwrap();
    window
        .set_text_input_area(Some(&Rect::new(10, 20, 50, 16)), 3)
        .unwrap();
    window.clear_composition().unwrap();
    let _ = get_events(EventType::FIRST, EventType::LAST, 100000);

    // A character, then one outside the BMP as a surrogate pair
    // SAFETY: hwnd is a live window of this thread.
    unsafe {
        PostMessageW(hwnd, WM_CHAR, 0xE9, 1);
        PostMessageW(hwnd, WM_CHAR, 0xD83D, 1);
        PostMessageW(hwnd, WM_CHAR, 0xDE00, 1);
    }
    let texts = |events: &[Event]| -> Vec<String> {
        events
            .iter()
            .filter_map(|e| match e {
                Event::TextInput(t) => Some(t.text.clone()),
                _ => None,
            })
            .collect()
    };
    let events = pump_until(|e| texts(e).len() >= 2);
    assert_eq!(texts(&events), vec!["\u{e9}", "\u{1F600}"], "{events:?}");

    window.stop_text_input().unwrap();
    // SAFETY: as above.
    unsafe { PostMessageW(hwnd, WM_CHAR, u16::from(b'x') as usize, 1) };
    let mut events = Vec::new();
    for _ in 0..20 {
        pump();
        events.extend(get_events(EventType::FIRST, EventType::LAST, 100000).unwrap());
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(texts(&events).is_empty(), "{events:?}");

    window.destroy();
}
