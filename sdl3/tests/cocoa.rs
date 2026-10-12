// Tests of the Cocoa video driver.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Cocoa video driver only starts on the main thread (as upstream's
//! `Cocoa_CreateDevice()`), and the test harness runs tests on other
//! threads, so this is a program of its own (`harness = false`) whose
//! `main` drives the driver through the public API: displays and modes,
//! windows (geometry, visibility, flags, fullscreen, popups), text input,
//! the clipboard, cursors, the keymap, mouse warping and message boxes.
//!
//! Without a window server session (capability `cocoa`) it skips, unless
//! `SDL3_TEST_REQUIRE` asks for `cocoa` (or `all`).

#[cfg(not(target_os = "macos"))]
fn main() {}

#[cfg(target_os = "macos")]
fn main() {
    macos::main();
}

#[cfg(target_os = "macos")]
mod macos {
    use std::ffi::{c_char, c_void};
    use std::time::{Duration, Instant};

    use sdl3::events::keyboard::{key_from_scancode, Keycode, Keymod, Scancode};
    use sdl3::events::window::WindowFlags;
    use sdl3::events::{Event, EventType};
    use sdl3::hints;
    use sdl3::init::{self, InitFlags};
    use sdl3::video::messagebox::{
        show_message_box, MessageBoxButtonData, MessageBoxButtonFlags, MessageBoxData,
        MessageBoxFlags,
    };
    use sdl3::video::{self, PixelFormat, Surface, Window};

    /// Whether `SDL3_TEST_REQUIRE` makes the `cocoa` capability mandatory.
    fn required() -> bool {
        std::env::var("SDL3_TEST_REQUIRE")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .any(|c| c == "all" || c == "cocoa")
    }

    fn skip(reason: &str) {
        if required() {
            panic!("required capability cocoa unavailable: {reason}");
        }
        println!("note: skipping, cocoa unavailable: {reason}");
    }

    /// Pump events for a while, returning what came in.
    fn pump_for(ms: u64) -> Vec<Event> {
        let end = Instant::now() + Duration::from_millis(ms);
        let mut events = Vec::new();
        while Instant::now() < end {
            while let Some(e) = sdl3::events::poll() {
                events.push(e);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        events
    }

    fn has_window_event(events: &[Event], window: &Window, ty: EventType) -> bool {
        events.iter().any(|e| match e {
            Event::Window(w) => w.window_id == window.id() && w.event_type == ty,
            _ => false,
        })
    }

    // The bits of the Objective-C runtime and GCD the message box check
    // needs to answer the alert from outside.
    unsafe extern "C" {
        fn objc_getClass(name: *const c_char) -> *mut c_void;
        fn sel_registerName(name: *const c_char) -> *mut c_void;
        fn objc_msgSend();
        static _dispatch_main_q: c_void;
        fn dispatch_async_f(
            queue: *const c_void,
            context: *mut c_void,
            work: unsafe extern "C" fn(*mut c_void),
        );
    }

    /// `[NSApp stopModalWithCode:1001]` (the second button), run by the main
    /// queue inside the alert's modal loop.
    unsafe extern "C" fn stop_modal_with_second_button(_ctx: *mut c_void) {
        // SAFETY: NSApplication's class method and instance method, with
        // their signatures.
        unsafe {
            let send0: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
                std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
            let send1: unsafe extern "C" fn(*mut c_void, *mut c_void, isize) =
                std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
            let app = send0(
                objc_getClass(c"NSApplication".as_ptr()),
                sel_registerName(c"sharedApplication".as_ptr()),
            );
            send1(app, sel_registerName(c"stopModalWithCode:".as_ptr()), 1001);
        }
    }

    pub(super) fn main() {
        // A hung transition shouldn't hang the test run.
        std::thread::spawn(|| {
            std::thread::sleep(Duration::from_secs(240));
            eprintln!("cocoa test timed out");
            std::process::exit(1);
        });

        hints::set(hints::VIDEO_DRIVER, "cocoa").unwrap();
        if let Err(e) = init::init(InitFlags::VIDEO) {
            skip(e.message());
            return;
        }
        assert_eq!(video::current_video_driver().unwrap(), "cocoa");
        println!("cocoa: video initialized");

        displays_and_modes();
        windows();
        popups();
        fullscreen();
        text_input();
        clipboard();
        cursors();
        keymap();
        mouse_warp();
        message_box();

        init::quit();
        println!("cocoa: all checks passed");
    }

    fn displays_and_modes() {
        let displays = video::displays().unwrap();
        assert!(!displays.is_empty(), "no displays");
        let primary = video::primary_display().unwrap();
        assert_eq!(primary, displays[0]);
        for &d in &displays {
            let name = video::display_name(d).unwrap();
            let bounds = video::display_bounds(d).unwrap();
            let usable = video::display_usable_bounds(d).unwrap();
            let desktop = video::desktop_display_mode(d).unwrap();
            let current = video::current_display_mode(d).unwrap();
            let scale = video::display_content_scale(d).unwrap();
            let modes = video::fullscreen_display_modes(d).unwrap();
            println!(
                "cocoa: display {d} {name:?} {bounds:?} usable {usable:?} desktop {}x{}@{} x{} {:?}, {} modes, content scale {scale}",
                desktop.w,
                desktop.h,
                desktop.refresh_rate,
                desktop.pixel_density,
                desktop.format,
                modes.len()
            );
            assert!(!name.is_empty());
            assert!(bounds.w > 0 && bounds.h > 0);
            assert!(usable.w > 0 && usable.h > 0);
            assert!(usable.x >= bounds.x && usable.y >= bounds.y);
            assert!(usable.x + usable.w <= bounds.x + bounds.w);
            assert!(usable.y + usable.h <= bounds.y + bounds.h);
            assert_eq!((desktop.w, desktop.h), (bounds.w, bounds.h));
            assert!(desktop.pixel_density >= 1.0);
            assert!(matches!(
                desktop.format,
                PixelFormat::ARGB8888 | PixelFormat::ARGB2101010 | PixelFormat::ARGB1555
            ));
            assert_eq!(current, desktop);
            assert!(!modes.is_empty());
            for m in &modes {
                assert!(m.w > 0 && m.h > 0 && m.pixel_density >= 1.0);
                assert_eq!(m.display_id, d);
            }
            assert!(scale >= 1.0);
        }
    }

    fn windows() {
        let window = Window::create(
            "SDL Cocoa test",
            320,
            240,
            WindowFlags::RESIZABLE | WindowFlags::HIGH_PIXEL_DENSITY,
        )
        .unwrap();
        let events = pump_for(300);
        let flags = window.flags().unwrap();
        assert!(!flags.contains(WindowFlags::HIDDEN), "{flags:?}");
        assert!(flags.contains(WindowFlags::RESIZABLE));
        assert!(has_window_event(&events, &window, EventType::WINDOW_SHOWN));
        assert_eq!(window.size().unwrap(), (320, 240));
        let density = window.pixel_density().unwrap();
        let (pw, ph) = window.size_in_pixels().unwrap();
        assert_eq!(
            (pw, ph),
            ((320.0 * density) as i32, (240.0 * density) as i32)
        );
        assert!(window.display().unwrap() != 0);
        println!(
            "cocoa: window {:?} density {density}",
            window.position().unwrap()
        );

        // Title
        window.set_title("SDL Cocoa test ✓").unwrap();
        assert_eq!(window.title().unwrap(), "SDL Cocoa test ✓");

        // Position and size round trips
        window.set_position(120, 160).unwrap();
        window.sync().unwrap();
        let events = pump_for(100);
        assert_eq!(window.position().unwrap(), (120, 160));
        let _ = events;
        window.set_size(400, 300).unwrap();
        window.sync().unwrap();
        let events = pump_for(100);
        assert_eq!(window.size().unwrap(), (400, 300));
        assert_eq!(window.position().unwrap(), (120, 160));
        let _ = events;

        // Minimum and maximum sizes
        window.set_minimum_size(200, 150).unwrap();
        window.set_maximum_size(800, 600).unwrap();
        window.set_size(100, 100).unwrap();
        window.sync().unwrap();
        pump_for(100);
        let (w, h) = window.size().unwrap();
        assert!(w >= 200 && h >= 150, "{w}x{h}");
        window.set_minimum_size(0, 0).unwrap();
        window.set_maximum_size(0, 0).unwrap();

        // Hide and show
        window.hide().unwrap();
        let events = pump_for(200);
        assert!(window.flags().unwrap().contains(WindowFlags::HIDDEN));
        assert!(has_window_event(&events, &window, EventType::WINDOW_HIDDEN));
        window.show().unwrap();
        let events = pump_for(200);
        assert!(!window.flags().unwrap().contains(WindowFlags::HIDDEN));
        assert!(has_window_event(&events, &window, EventType::WINDOW_SHOWN));

        // Borders, always on top, opacity
        window.set_bordered(false).unwrap();
        pump_for(50);
        assert!(window.flags().unwrap().contains(WindowFlags::BORDERLESS));
        window.set_bordered(true).unwrap();
        pump_for(50);
        assert!(!window.flags().unwrap().contains(WindowFlags::BORDERLESS));
        window.set_always_on_top(true).unwrap();
        assert!(window.flags().unwrap().contains(WindowFlags::ALWAYS_ON_TOP));
        window.set_always_on_top(false).unwrap();
        window.set_opacity(0.5).unwrap();
        assert_eq!(window.opacity().unwrap(), 0.5);
        window.set_opacity(1.0).unwrap();
        window.set_resizable(false).unwrap();
        assert!(!window.flags().unwrap().contains(WindowFlags::RESIZABLE));
        window.set_resizable(true).unwrap();

        // Minimize and restore (animated: sync waits for them)
        window.minimize().unwrap();
        window.sync().unwrap();
        let events = pump_for(200);
        assert!(window.flags().unwrap().contains(WindowFlags::MINIMIZED));
        assert!(has_window_event(
            &events,
            &window,
            EventType::WINDOW_MINIMIZED
        ));
        window.restore().unwrap();
        window.sync().unwrap();
        let events = pump_for(500);
        assert!(!window.flags().unwrap().contains(WindowFlags::MINIMIZED));
        assert!(has_window_event(
            &events,
            &window,
            EventType::WINDOW_RESTORED
        ));

        // Maximize and restore
        window.maximize().unwrap();
        window.sync().unwrap();
        pump_for(300);
        assert!(window.flags().unwrap().contains(WindowFlags::MAXIMIZED));
        window.restore().unwrap();
        window.sync().unwrap();
        pump_for(300);
        assert!(!window.flags().unwrap().contains(WindowFlags::MAXIMIZED));

        // A second window, raised over the first
        let other = Window::create("SDL Cocoa test 2", 200, 100, WindowFlags::NONE).unwrap();
        pump_for(200);
        other.raise().unwrap();
        pump_for(100);
        other.destroy();
        pump_for(100);

        // The window's properties
        let props = window.properties().unwrap();
        assert!(props.get_number("SDL.window.cocoa.window").unwrap_or(0) != 0);
        assert_eq!(
            props.get_number("SDL.window.cocoa.metal_view_tag"),
            Some(255)
        );

        window.destroy();
        pump_for(100);
        println!("cocoa: windows ok");
    }

    fn popups() {
        let parent = Window::create("SDL Cocoa popup parent", 300, 200, WindowFlags::NONE).unwrap();
        parent.set_position(200, 200).unwrap();
        parent.sync().unwrap();
        pump_for(200);
        let tooltip = Window::create_popup(&parent, 10, 20, 100, 30, WindowFlags::TOOLTIP).unwrap();
        pump_for(200);
        assert!(tooltip.flags().unwrap().contains(WindowFlags::TOOLTIP));
        assert_eq!(tooltip.position().unwrap(), (10, 20));
        let menu = Window::create_popup(&parent, 30, 40, 120, 80, WindowFlags::POPUP_MENU).unwrap();
        pump_for(200);
        assert!(menu.flags().unwrap().contains(WindowFlags::POPUP_MENU));
        menu.destroy();
        tooltip.destroy();
        parent.destroy();
        pump_for(100);
        println!("cocoa: popups ok");
    }

    fn fullscreen() {
        let window =
            Window::create("SDL Cocoa fullscreen", 320, 240, WindowFlags::RESIZABLE).unwrap();
        pump_for(300);
        let display = window.display().unwrap();
        let bounds = video::display_bounds(display).unwrap();

        // Desktop fullscreen (a Space, animated)
        window.set_fullscreen(true).unwrap();
        window.sync().unwrap();
        let events = pump_for(500);
        assert!(window.flags().unwrap().contains(WindowFlags::FULLSCREEN));
        assert!(has_window_event(
            &events,
            &window,
            EventType::WINDOW_ENTER_FULLSCREEN
        ));
        let (w, h) = window.size().unwrap();
        println!("cocoa: fullscreen {w}x{h} on {}x{}", bounds.w, bounds.h);
        assert_eq!(w, bounds.w);
        assert!(h > 0 && h <= bounds.h);

        window.set_fullscreen(false).unwrap();
        window.sync().unwrap();
        let events = pump_for(500);
        assert!(!window.flags().unwrap().contains(WindowFlags::FULLSCREEN));
        assert!(has_window_event(
            &events,
            &window,
            EventType::WINDOW_LEAVE_FULLSCREEN
        ));
        assert_eq!(window.size().unwrap(), (320, 240));

        window.destroy();
        pump_for(200);
        println!("cocoa: fullscreen ok");
    }

    fn text_input() {
        let window = Window::create("SDL Cocoa text input", 200, 100, WindowFlags::NONE).unwrap();
        pump_for(200);
        window.start_text_input().unwrap();
        assert!(window.text_input_active().unwrap());
        window
            .set_text_input_area(Some(&video::Rect::new(10, 20, 30, 40)), 5)
            .unwrap();
        window.stop_text_input().unwrap();
        assert!(!window.text_input_active().unwrap());
        window.destroy();
        pump_for(100);
        println!("cocoa: text input ok");
    }

    fn clipboard() {
        // (the user's clipboard text is put back afterwards)
        let saved = video::clipboard::clipboard_text().ok();
        video::clipboard::set_clipboard_text("héllo from SDL ✓").unwrap();
        assert!(video::clipboard::has_clipboard_text().unwrap());
        assert_eq!(
            video::clipboard::clipboard_text().unwrap(),
            "héllo from SDL ✓"
        );
        assert!(video::clipboard::has_clipboard_data("text/plain;charset=utf-8").unwrap());
        // Data offered by mime type, produced when asked for
        let png = vec![0x89u8, b'P', b'N', b'G', 1, 2, 3];
        let data = png.clone();
        video::clipboard::set_clipboard_data(
            Some(std::sync::Arc::new(move |mime: &str| {
                (mime == "image/png").then(|| data.clone())
            })),
            &["image/png"],
        )
        .unwrap();
        assert!(video::clipboard::has_clipboard_data("image/png").unwrap());
        assert_eq!(
            video::clipboard::clipboard_data("image/png")
                .unwrap()
                .as_deref(),
            Some(&png[..])
        );
        if let Some(saved) = saved.filter(|s| !s.is_empty()) {
            let _ = video::clipboard::set_clipboard_text(&saved);
        }
        println!("cocoa: clipboard ok");
    }

    fn cursors() {
        use sdl3::events::mouse::*;
        let window = Window::create("SDL Cocoa cursors", 200, 100, WindowFlags::NONE).unwrap();
        pump_for(100);
        for id in [
            SystemCursor::Default,
            SystemCursor::Text,
            SystemCursor::Wait,
            SystemCursor::Crosshair,
            SystemCursor::Progress,
            SystemCursor::NwseResize,
            SystemCursor::NeswResize,
            SystemCursor::EwResize,
            SystemCursor::NsResize,
            SystemCursor::Move,
            SystemCursor::NotAllowed,
            SystemCursor::Pointer,
            SystemCursor::NwResize,
            SystemCursor::NResize,
            SystemCursor::NeResize,
            SystemCursor::EResize,
            SystemCursor::SeResize,
            SystemCursor::SResize,
            SystemCursor::SwResize,
            SystemCursor::WResize,
        ] {
            let cursor = create_system_cursor(id).unwrap();
            set_cursor(Some(&cursor)).unwrap();
            pump_for(10);
            destroy_cursor(cursor);
        }
        let mut surface = Surface::new(16, 16, PixelFormat::ARGB8888).unwrap();
        surface.fill_rect(None, 0xff00ff00).unwrap();
        let cursor = create_color_cursor(&surface, 8, 8).unwrap();
        set_cursor(Some(&cursor)).unwrap();
        hide_cursor();
        assert!(!cursor_visible());
        show_cursor();
        assert!(cursor_visible());
        pump_for(50);
        set_cursor(default_cursor().as_ref()).unwrap();
        destroy_cursor(cursor);
        window.destroy();
        pump_for(50);
        println!("cocoa: cursors ok");
    }

    fn keymap() {
        // Layout-independent keys
        assert_eq!(
            key_from_scancode(Scancode::RETURN, Keymod::NONE, false),
            Keycode(b'\r' as u32)
        );
        assert_eq!(
            key_from_scancode(Scancode::ESCAPE, Keymod::NONE, false),
            Keycode(0x1b)
        );
        // The letter keys type something on any layout
        let a = key_from_scancode(Scancode::A, Keymod::NONE, false);
        assert!(
            a != Keycode::UNKNOWN && a.0 & Keycode::SCANCODE_MASK == 0,
            "{a:?}"
        );
        assert_eq!(Scancode::LALT.name(), "Left Option");
        assert_eq!(Scancode::RGUI.name(), "Right Command");
        println!("cocoa: keymap ok (A types {:?})", char::from_u32(a.0));
    }

    fn mouse_warp() {
        use sdl3::events::mouse::*;
        let window = Window::create("SDL Cocoa warp", 300, 200, WindowFlags::NONE).unwrap();
        window.set_position(150, 150).unwrap();
        window.sync().unwrap();
        pump_for(300);
        let (ox, oy, _) = global_mouse_state();
        warp_mouse_in_window(Some(window.id()), 40.0, 30.0);
        pump_for(100);
        let (x, y, _) = global_mouse_state();
        let (wx, wy) = window.position().unwrap();
        println!("cocoa: warped to ({x}, {y}), window at ({wx}, {wy})");
        assert!((x - (wx + 40) as f32).abs() <= 1.0 && (y - (wy + 30) as f32).abs() <= 1.0);
        let _ = warp_mouse_global(ox, oy);
        window.destroy();
        pump_for(50);
        println!("cocoa: mouse warp ok");
    }

    fn message_box() {
        let data = MessageBoxData {
            flags: MessageBoxFlags::WARNING,
            window: None,
            title: "SDL Cocoa test".into(),
            message: "This alert closes itself.".into(),
            buttons: vec![
                MessageBoxButtonData {
                    flags: MessageBoxButtonFlags::RETURNKEY_DEFAULT,
                    button_id: 7,
                    text: "First".into(),
                },
                MessageBoxButtonData {
                    flags: MessageBoxButtonFlags::ESCAPEKEY_DEFAULT,
                    button_id: 42,
                    text: "Second".into(),
                },
            ],
            color_scheme: None,
        };
        // Answer with the second button from the main queue, which the
        // alert's modal loop runs.
        // SAFETY: the main queue and a function taking no context.
        unsafe {
            dispatch_async_f(
                std::ptr::addr_of!(_dispatch_main_q),
                std::ptr::null_mut(),
                stop_modal_with_second_button,
            );
        }
        assert_eq!(show_message_box(&data).unwrap(), 42);
        println!("cocoa: message box ok");
    }
}
