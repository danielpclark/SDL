// Tests of the X11 video driver against a real X server.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Each test runs against its own Xvfb server (started on a free display
//! number with `-displayfd`), or the server in `DISPLAY` if one can be
//! opened. Without either the tests print a note and pass.

use std::ffi::{c_int, c_uchar, c_uint, c_ulong, OsString};
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::sys::*;
use super::x11dyn::{load_symbols, unload_symbols, X11Syms};
use crate::events::keyboard::{self, Keycode, Keymod, Scancode};
use crate::events::mouse::{self, SystemCursor};
use crate::events::window::WindowFlags;
use crate::events::{Event, EventType};
use crate::hints;
use crate::init::{self, InitFlags};
use crate::loadso::SharedObject;
use crate::test_support::TEST_LOCK;
use crate::video::pixels::Color;
use crate::video::window::{Window as SdlWindow, PROP_WINDOW_X11_WINDOW_NUMBER};
use crate::video::{self, PixelFormat, Rect, Surface};

/// An X client connection of the test itself (to check what SDL did).
struct Client {
    x: Arc<X11Syms>,
    dpy: *mut Display,
}

impl Client {
    fn open() -> Option<Client> {
        let x = load_symbols()?;
        // SAFETY: a NULL name opens $DISPLAY.
        let dpy = unsafe { (x.XOpenDisplay)(std::ptr::null()) };
        if dpy.is_null() {
            unload_symbols();
            return Option::None;
        }
        Some(Client { x, dpy })
    }

    fn atom(&self, name: &str) -> Atom {
        super::video::intern_atom(&self.x, self.dpy, name, false)
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        // SAFETY: the display was opened by Client::open.
        unsafe {
            (self.x.XCloseDisplay)(self.dpy);
        }
        unload_symbols();
    }
}

/// The X server of a test: an Xvfb started for it, or the one in
/// `DISPLAY`. Restores `DISPLAY` when dropped.
struct XServer {
    child: Option<Child>,
    old_display: Option<OsString>,
}

impl XServer {
    /// The server in `DISPLAY`, or a new Xvfb.
    fn start() -> Option<XServer> {
        let old_display = std::env::var_os("DISPLAY");
        if old_display.is_some() && Client::open().is_some() {
            return Some(XServer {
                child: Option::None,
                old_display,
            });
        }
        XServer::xvfb(&[])
    }

    /// A new Xvfb with extra arguments.
    fn xvfb(extra: &[&str]) -> Option<XServer> {
        let old_display = std::env::var_os("DISPLAY");
        let child = Command::new("Xvfb")
            .args([
                "-displayfd",
                "1",
                "-screen",
                "0",
                "1024x768x24",
                "-nolisten",
                "tcp",
                "-noreset",
            ])
            .args(extra)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn();
        let mut child = match child {
            Ok(child) => child,
            Err(_) => {
                eprintln!("note: no X server (DISPLAY can't be opened and Xvfb isn't available); skipping");
                return Option::None;
            }
        };
        // Xvfb writes its display number once it accepts connections
        let mut line = String::new();
        let read = child
            .stdout
            .take()
            .map(|out| BufReader::new(out).read_line(&mut line).unwrap_or(0))
            .unwrap_or(0);
        let number = line.trim();
        if read == 0 || number.is_empty() {
            let _ = child.kill();
            let _ = child.wait();
            eprintln!("note: Xvfb didn't start; skipping");
            return Option::None;
        }
        std::env::set_var("DISPLAY", format!(":{number}"));
        let server = XServer {
            child: Some(child),
            old_display,
        };
        if Client::open().is_none() {
            eprintln!("note: can't connect to Xvfb; skipping");
            return Option::None;
        }
        Some(server)
    }
}

impl Drop for XServer {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        match &self.old_display {
            Some(d) => std::env::set_var("DISPLAY", d),
            Option::None => std::env::remove_var("DISPLAY"),
        }
    }
}

/// Video initialized with the X11 driver; quits video when dropped (before
/// the server goes away).
struct Video;

impl Video {
    fn init() -> Video {
        init::quit();
        hints::set(hints::VIDEO_DRIVER, "x11").unwrap();
        init::init(InitFlags::VIDEO).unwrap();
        Video
    }
}

impl Drop for Video {
    fn drop(&mut self) {
        init::quit();
        hints::reset(hints::VIDEO_DRIVER);
    }
}

/// Pump events until `pred` matches one (returned), for up to two seconds.
fn wait_for(mut pred: impl FnMut(&Event) -> bool) -> Option<Event> {
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        crate::events::pump();
        while let Some(e) = crate::events::poll() {
            if pred(&e) {
                return Some(e);
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    Option::None
}

/// Drain the queue.
fn drain() {
    crate::events::pump();
    while crate::events::poll().is_some() {}
}

fn xwindow_of(w: &SdlWindow) -> Window {
    w.properties()
        .unwrap()
        .get_number(PROP_WINDOW_X11_WINDOW_NUMBER)
        .unwrap() as Window
}

/// Load libXtst for XTestFakeKeyEvent/XTestFakeButtonEvent/XTestFakeMotionEvent.
struct XTest {
    _lib: SharedObject,
    key: unsafe extern "C" fn(*mut Display, c_uint, Bool, c_ulong) -> c_int,
    button: unsafe extern "C" fn(*mut Display, c_uint, Bool, c_ulong) -> c_int,
    motion: unsafe extern "C" fn(*mut Display, c_int, c_int, c_int, c_ulong) -> c_int,
}

impl XTest {
    fn load() -> Option<XTest> {
        let lib = SharedObject::load("libXtst.so.6").ok()?;
        // SAFETY: the XTest functions have these signatures.
        unsafe {
            Some(XTest {
                key: lib.function("XTestFakeKeyEvent").ok()?,
                button: lib.function("XTestFakeButtonEvent").ok()?,
                motion: lib.function("XTestFakeMotionEvent").ok()?,
                _lib: lib,
            })
        }
    }
}

#[test]
fn x11_init_and_modes() {
    let _l = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let Some(_server) = XServer::start() else {
        return;
    };
    let _video = Video::init();
    assert_eq!(video::current_video_driver().unwrap(), "x11");
    // (the screen size: 1024x768 for our Xvfb)
    let (sw, sh) = {
        let client = Client::open().unwrap();
        // SAFETY: the client display is open.
        unsafe {
            let screen = DefaultScreen(client.dpy);
            (
                DisplayWidth(client.dpy, screen),
                DisplayHeight(client.dpy, screen),
            )
        }
    };

    let displays = video::displays().unwrap();
    assert!(!displays.is_empty());
    let primary = video::primary_display().unwrap();
    let bounds = video::display_bounds(primary).unwrap();
    assert_eq!((bounds.w, bounds.h), (sw, sh));
    let usable = video::display_usable_bounds(primary).unwrap();
    assert!(usable.w > 0 && usable.h > 0);
    assert!(!video::display_name(primary).unwrap().is_empty());

    let desktop = video::desktop_display_mode(primary).unwrap();
    assert_eq!((desktop.w, desktop.h), (sw, sh));
    assert_ne!(desktop.format, PixelFormat::UNKNOWN);
    let current = video::current_display_mode(primary).unwrap();
    assert_eq!((current.w, current.h), (sw, sh));
    let modes = video::fullscreen_display_modes(primary).unwrap();
    assert!(modes.iter().any(|m| m.w == sw && m.h == sh), "{modes:?}");
    assert!(video::display_content_scale(primary).unwrap() >= 1.0);
}

#[test]
fn x11_windows() {
    let _l = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let Some(_server) = XServer::start() else {
        return;
    };
    let _video = Video::init();
    let client = Client::open().unwrap();

    let w = SdlWindow::create(
        "X11 test",
        320,
        240,
        WindowFlags::HIDDEN | WindowFlags::RESIZABLE,
    )
    .unwrap();
    let id = w.id();
    let xwin = xwindow_of(&w);
    assert_ne!(xwin, 0);
    assert_eq!(w.title().unwrap(), "X11 test");
    drain();

    w.show().unwrap();
    assert!(
        wait_for(|e| matches!(e, Event::Window(we) if we.event_type == EventType::WINDOW_SHOWN && we.window_id == id))
            .is_some()
            || !w.flags().unwrap().contains(WindowFlags::HIDDEN)
    );
    // SAFETY: the client display is open; attrs is an out-parameter.
    let attrs = unsafe {
        let mut attrs: XWindowAttributes = std::mem::zeroed();
        (client.x.XGetWindowAttributes)(client.dpy, xwin, &mut attrs);
        attrs
    };
    assert_eq!(attrs.map_state, IsViewable);
    assert_eq!((attrs.width, attrs.height), (320, 240));

    // Resize and move, through the server
    w.set_size(400, 300).unwrap();
    w.set_position(50, 60).unwrap();
    w.sync().unwrap();
    assert_eq!(w.size().unwrap(), (400, 300));
    assert_eq!(w.position().unwrap(), (50, 60));
    // SAFETY: as above.
    let attrs = unsafe {
        let mut attrs: XWindowAttributes = std::mem::zeroed();
        (client.x.XGetWindowAttributes)(client.dpy, xwin, &mut attrs);
        attrs
    };
    assert_eq!((attrs.width, attrs.height), (400, 300));

    // The resize shows up in the event queue
    drain();
    w.set_size(200, 150).unwrap();
    w.sync().unwrap();
    let resized =
        wait_for(|e| matches!(e, Event::Window(we) if we.event_type == EventType::WINDOW_RESIZED))
            .map(|e| match e {
                Event::Window(we) => (we.data1, we.data2),
                _ => (0, 0),
            });
    assert!(
        resized == Some((200, 150)) || w.size().unwrap() == (200, 150),
        "{resized:?}"
    );

    // The title is the window's WM_NAME / _NET_WM_NAME
    w.set_title("Renamed ✓").unwrap();
    let net_wm_name = client.atom("_NET_WM_NAME");
    let utf8 = client.atom("UTF8_STRING");
    let mut actual_type: Atom = 0;
    let mut format: c_int = 0;
    let mut nitems: c_ulong = 0;
    let mut after: c_ulong = 0;
    let mut data: *mut c_uchar = std::ptr::null_mut();
    // SAFETY: the client display is open; the out-parameters are valid;
    // the data is freed below.
    let title = unsafe {
        (client.x.XGetWindowProperty)(
            client.dpy,
            xwin,
            net_wm_name,
            0,
            1024,
            False,
            utf8,
            &mut actual_type,
            &mut format,
            &mut nitems,
            &mut after,
            &mut data,
        );
        let t = std::slice::from_raw_parts(data, nitems as usize).to_vec();
        (client.x.XFree)(data.cast());
        String::from_utf8(t).unwrap()
    };
    assert_eq!(title, "Renamed ✓");

    // Hide, then destroy
    w.hide().unwrap();
    assert!(w.flags().unwrap().contains(WindowFlags::HIDDEN));
    // SAFETY: as above.
    let attrs = unsafe {
        let mut attrs: XWindowAttributes = std::mem::zeroed();
        (client.x.XGetWindowAttributes)(client.dpy, xwin, &mut attrs);
        attrs
    };
    assert_eq!(attrs.map_state, IsUnmapped);
    w.destroy();
    drain();
}

/// Draw into a window's framebuffer and read the window back.
fn framebuffer_roundtrip() {
    let _video = Video::init();
    let client = Client::open().unwrap();
    let w = SdlWindow::create("framebuffer", 64, 48, WindowFlags::default()).unwrap();
    let xwin = xwindow_of(&w);
    drain();

    let surface = w.surface().unwrap();
    {
        let mut s = surface.lock();
        let format = s.format();
        let pitch = s.pitch() as usize;
        let details = crate::video::pixels::PixelFormatDetails::new(format).unwrap();
        let red = details
            .map_rgb(
                Option::None,
                Color {
                    r: 255,
                    g: 0,
                    b: 0,
                    a: 255,
                },
            )
            .unwrap();
        let blue = details
            .map_rgb(
                Option::None,
                Color {
                    r: 0,
                    g: 0,
                    b: 255,
                    a: 255,
                },
            )
            .unwrap();
        let bpp = format.bytes_per_pixel() as usize;
        let pixels = s.pixels_mut().unwrap();
        for y in 0..48 {
            for x in 0..64 {
                let v = if x < 32 { red } else { blue };
                let o = y * pitch + x * bpp;
                pixels[o..o + bpp].copy_from_slice(&v.to_ne_bytes()[..bpp]);
            }
        }
    }
    w.update_surface().unwrap();

    // Read the window back from the server
    // SAFETY: the client display is open and the window viewable; the
    // image is destroyed after use.
    let (left, right) = unsafe {
        (client.x.XSync)(client.dpy, False);
        let image = (client.x.XGetImage)(client.dpy, xwin, 0, 0, 64, 48, !0, ZPixmap);
        assert!(!image.is_null());
        let left = XGetPixel(image, 10, 10);
        let right = XGetPixel(image, 50, 40);
        XDestroyImage(image);
        (left, right)
    };
    // (a 24-bit TrueColor visual: 0xRRGGBB)
    assert_eq!(left & 0xffffff, 0xff0000, "left {left:#x}");
    assert_eq!(right & 0xffffff, 0x0000ff, "right {right:#x}");

    // A partial update
    {
        let mut s = surface.lock();
        let pitch = s.pitch() as usize;
        let pixels = s.pixels_mut().unwrap();
        for y in 0..48 {
            for b in &mut pixels[y * pitch..y * pitch + 8 * 4] {
                *b = 0xff;
            }
        }
    }
    w.update_surface_rects(&[Rect::new(0, 0, 8, 8)]).unwrap();
    // SAFETY: as above.
    let (corner, below) = unsafe {
        (client.x.XSync)(client.dpy, False);
        let image = (client.x.XGetImage)(client.dpy, xwin, 0, 0, 16, 16, !0, ZPixmap);
        let corner = XGetPixel(image, 2, 2);
        let below = XGetPixel(image, 2, 12);
        XDestroyImage(image);
        (corner, below)
    };
    assert_eq!(corner & 0xffffff, 0xffffff);
    // (only the rectangle was updated)
    assert_eq!(below & 0xffffff, 0xff0000);

    // Resizing makes a new framebuffer
    w.set_size(80, 60).unwrap();
    w.sync().unwrap();
    drain();
    let surface = w.surface().unwrap();
    assert_eq!(surface.lock().width(), 80);
    w.update_surface().unwrap();
    w.destroy_surface().unwrap();
    w.destroy();
}

#[test]
fn x11_framebuffer_shm() {
    let _l = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let Some(_server) = XServer::start() else {
        return;
    };
    framebuffer_roundtrip();
}

#[test]
fn x11_framebuffer_putimage() {
    let _l = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // (a server without MIT-SHM: the framebuffer goes through XPutImage)
    let Some(_server) = XServer::xvfb(&["-extension", "MIT-SHM"]) else {
        return;
    };
    framebuffer_roundtrip();
}

#[test]
fn x11_clipboard() {
    let _l = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let Some(_server) = XServer::start() else {
        return;
    };
    let _video = Video::init();

    // Within the app
    crate::video::clipboard::set_clipboard_text("hello from SDL").unwrap();
    assert!(crate::video::clipboard::has_clipboard_text().unwrap());
    assert_eq!(
        crate::video::clipboard::clipboard_text().unwrap(),
        "hello from SDL"
    );

    // Another client reads it: SDL answers the SelectionRequest
    let client = Client::open().unwrap();
    let clipboard = client.atom("CLIPBOARD");
    let utf8 = client.atom("UTF8_STRING");
    let prop = client.atom("TEST_PROP");
    // SAFETY: the client display is open; the window is the client's.
    let win = unsafe {
        let root = DefaultRootWindow(client.dpy);
        let mut attrs: XSetWindowAttributes = std::mem::zeroed();
        let win = (client.x.XCreateWindow)(
            client.dpy,
            root,
            0,
            0,
            1,
            1,
            0,
            CopyFromParent as c_int,
            InputOnly as c_uint,
            std::ptr::null_mut(),
            0,
            &mut attrs,
        );
        (client.x.XConvertSelection)(client.dpy, clipboard, utf8, prop, win, CurrentTime);
        (client.x.XFlush)(client.dpy);
        win
    };
    let mut got = Option::None;
    let deadline = Instant::now() + Duration::from_secs(2);
    while got.is_none() && Instant::now() < deadline {
        crate::events::pump();
        let mut ev = XEvent::zeroed();
        // SAFETY: the client display is open.
        unsafe {
            while (client.x.XPending)(client.dpy) != 0 {
                (client.x.XCheckIfEvent)(
                    client.dpy,
                    &mut ev,
                    Some(any_event),
                    std::ptr::null_mut(),
                );
                if ev.get_type() == SelectionNotify && ev.selection().property == prop {
                    let mut actual_type: Atom = 0;
                    let mut format: c_int = 0;
                    let mut nitems: c_ulong = 0;
                    let mut after: c_ulong = 0;
                    let mut data: *mut c_uchar = std::ptr::null_mut();
                    (client.x.XGetWindowProperty)(
                        client.dpy,
                        win,
                        prop,
                        0,
                        1024,
                        False,
                        utf8,
                        &mut actual_type,
                        &mut format,
                        &mut nitems,
                        &mut after,
                        &mut data,
                    );
                    got = Some(
                        String::from_utf8_lossy(std::slice::from_raw_parts(data, nitems as usize))
                            .into_owned(),
                    );
                    (client.x.XFree)(data.cast());
                }
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(got.as_deref(), Some("hello from SDL"));

    // The other client takes the clipboard: SDL converts it (answered by
    // a thread serving the other client's selection)
    let owner = std::thread::spawn(serve_selection);
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut text = String::new();
    while Instant::now() < deadline {
        crate::events::pump();
        if crate::video::clipboard::has_clipboard_text().unwrap_or(false) {
            text = crate::video::clipboard::clipboard_text().unwrap();
            if text == "hello from X" {
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let _ = owner.join();
    assert_eq!(text, "hello from X");

    // The primary selection
    crate::video::clipboard::set_primary_selection_text("primary").unwrap();
    assert_eq!(
        crate::video::clipboard::primary_selection_text().unwrap(),
        "primary"
    );
    assert!(crate::video::clipboard::has_primary_selection_text().unwrap());
}

/// `isAnyEvent()` for the test client.
unsafe extern "C" fn any_event(_d: *mut Display, _e: *mut XEvent, _a: XPointer) -> Bool {
    True
}

/// Own CLIPBOARD from another client and answer conversion requests for a
/// second (until two requests were answered or two seconds passed).
fn serve_selection() {
    let client = Client::open().unwrap();
    let clipboard = client.atom("CLIPBOARD");
    let targets = client.atom("TARGETS");
    let utf8 = client.atom("UTF8_STRING");
    let text = b"hello from X";
    // SAFETY: the client display is open; the window is the client's.
    let win = unsafe {
        let root = DefaultRootWindow(client.dpy);
        let mut attrs: XSetWindowAttributes = std::mem::zeroed();
        let win = (client.x.XCreateWindow)(
            client.dpy,
            root,
            0,
            0,
            1,
            1,
            0,
            CopyFromParent as c_int,
            InputOnly as c_uint,
            std::ptr::null_mut(),
            0,
            &mut attrs,
        );
        (client.x.XSetSelectionOwner)(client.dpy, clipboard, win, CurrentTime);
        (client.x.XFlush)(client.dpy);
        win
    };
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut answered = 0;
    while Instant::now() < deadline && answered < 3 {
        let mut ev = XEvent::zeroed();
        // SAFETY: the client display is open; the replies go to the
        // requestor's property.
        unsafe {
            if (client.x.XPending)(client.dpy) == 0 {
                std::thread::sleep(Duration::from_millis(2));
                continue;
            }
            (client.x.XCheckIfEvent)(client.dpy, &mut ev, Some(any_event), std::ptr::null_mut());
            if ev.get_type() != SelectionRequest {
                continue;
            }
            let req = *ev.selectionrequest();
            let mut reply = XEvent::zeroed();
            {
                let s = reply.selection_mut();
                s.type_ = SelectionNotify;
                s.requestor = req.requestor;
                s.selection = req.selection;
                s.target = req.target;
                s.property = req.property;
                s.time = req.time;
            }
            if req.target == targets {
                let list: [Atom; 2] = [targets, utf8];
                (client.x.XChangeProperty)(
                    client.dpy,
                    req.requestor,
                    req.property,
                    XA_ATOM,
                    32,
                    PropModeReplace,
                    list.as_ptr() as *const c_uchar,
                    2,
                );
            } else if req.target == utf8 {
                (client.x.XChangeProperty)(
                    client.dpy,
                    req.requestor,
                    req.property,
                    utf8,
                    8,
                    PropModeReplace,
                    text.as_ptr(),
                    text.len() as c_int,
                );
            } else {
                reply.selection_mut().property = None;
            }
            (client.x.XSendEvent)(client.dpy, req.requestor, False, 0, &mut reply);
            (client.x.XFlush)(client.dpy);
            answered += 1;
        }
    }
    let _ = win;
}

#[test]
fn x11_cursors() {
    let _l = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let Some(_server) = XServer::start() else {
        return;
    };
    let _video = Video::init();
    let w = SdlWindow::create("cursors", 100, 100, WindowFlags::default()).unwrap();

    for id in [
        SystemCursor::Default,
        SystemCursor::Text,
        SystemCursor::Wait,
        SystemCursor::Pointer,
        SystemCursor::NwseResize,
    ] {
        let c = mouse::create_system_cursor(id).unwrap();
        mouse::set_cursor(Some(&c)).unwrap();
        mouse::destroy_cursor(c);
    }

    let mut s = Surface::new(16, 16, PixelFormat::ARGB8888).unwrap();
    {
        let p = s.pixels_mut().unwrap();
        for (i, px) in p.chunks_mut(4).enumerate() {
            let v: u32 = if (i % 16) < 8 { 0xffff0000 } else { 0x00000000 };
            px.copy_from_slice(&v.to_ne_bytes());
        }
    }
    let c = mouse::create_color_cursor(&s, 2, 3).unwrap();
    mouse::set_cursor(Some(&c)).unwrap();
    mouse::hide_cursor();
    assert!(!mouse::cursor_visible());
    mouse::show_cursor();
    assert!(mouse::cursor_visible());
    mouse::set_cursor(mouse::default_cursor().as_ref()).unwrap();
    mouse::destroy_cursor(c);

    // Warping the pointer moves it on the server
    w.raise().unwrap();
    mouse::warp_mouse_in_window(Some(w.id()), 30.0, 40.0);
    let (x, y, _) = mouse::global_mouse_state();
    let (wx, wy) = w.position().unwrap();
    assert_eq!((x as i32, y as i32), (wx + 30, wy + 40));
    w.destroy();
}

#[test]
fn x11_keyboard_and_mouse_input() {
    let _l = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let Some(_server) = XServer::start() else {
        return;
    };
    let _video = Video::init();

    // The keymap of the server's (us) layout
    assert_eq!(
        keyboard::key_from_scancode(Scancode::A, Keymod::NONE, false),
        Keycode::A
    );
    assert_eq!(
        keyboard::key_from_scancode(Scancode::N1, Keymod::NONE, false),
        Keycode::N1
    );
    assert_eq!(
        keyboard::key_from_scancode(Scancode::RETURN, Keymod::NONE, false),
        Keycode::RETURN
    );
    assert_eq!(
        keyboard::key_from_scancode(Scancode::LSHIFT, Keymod::NONE, false),
        Keycode::LSHIFT
    );
    assert_eq!(keyboard::scancode_from_key(Keycode::Z).0, Scancode::Z);

    let Some(xtest) = XTest::load() else {
        eprintln!("note: libXtst not available; skipping the input part");
        return;
    };
    let client = Client::open().unwrap();
    let w = SdlWindow::create("input", 200, 200, WindowFlags::default()).unwrap();
    let id = w.id();
    w.raise().unwrap();
    drain();
    let _ = wait_for(
        |e| matches!(e, Event::Window(we) if we.event_type == EventType::WINDOW_FOCUS_GAINED),
    );
    assert_eq!(keyboard::keyboard_focus(), Some(id));

    // SAFETY: the client display is open; the key code is the server's.
    let keycode = unsafe {
        (client.x.XKeysymToKeycode)(client.dpy, 0x61 /* XK_a */)
    } as c_uint;
    // SAFETY: as above.
    unsafe {
        (xtest.key)(client.dpy, keycode, True, 0);
        (client.x.XFlush)(client.dpy);
    }
    let down = wait_for(|e| matches!(e, Event::Key(k) if k.down)).expect("no key down event");
    match down {
        Event::Key(k) => {
            assert_eq!(k.scancode, Scancode::A);
            assert_eq!(k.key, Keycode::A);
            assert_eq!(k.window_id, id);
        }
        _ => unreachable!(),
    }
    // SAFETY: as above.
    unsafe {
        (xtest.key)(client.dpy, keycode, False, 0);
        (client.x.XFlush)(client.dpy);
    }
    assert!(
        wait_for(|e| matches!(e, Event::Key(k) if !k.down && k.scancode == Scancode::A)).is_some()
    );

    // Mouse motion and buttons
    let (wx, wy) = w.position().unwrap();
    // SAFETY: as above.
    unsafe {
        (xtest.motion)(client.dpy, 0, wx + 20, wy + 25, 0);
        (client.x.XFlush)(client.dpy);
    }
    let motion = wait_for(|e| matches!(e, Event::MouseMotion(m) if m.x == 20.0 && m.y == 25.0));
    assert!(motion.is_some());
    // SAFETY: as above.
    unsafe {
        (xtest.button)(client.dpy, 1, True, 0);
        (xtest.button)(client.dpy, 1, False, 0);
        (client.x.XFlush)(client.dpy);
    }
    let button = wait_for(
        |e| matches!(e, Event::MouseButton(b) if b.down && b.button == mouse::BUTTON_LEFT),
    );
    assert!(button.is_some());
    assert!(wait_for(|e| matches!(e, Event::MouseButton(b) if !b.down)).is_some());
    // SAFETY: as above (button 4 is the wheel).
    unsafe {
        (xtest.button)(client.dpy, 4, True, 0);
        (xtest.button)(client.dpy, 4, False, 0);
        (client.x.XFlush)(client.dpy);
    }
    let wheel = wait_for(|e| matches!(e, Event::MouseWheel(_)));
    match wheel {
        Some(Event::MouseWheel(m)) => assert_eq!(m.y, 1.0),
        other => panic!("no wheel event: {other:?}"),
    }
    w.destroy();
}

#[test]
fn x11_vulkan() {
    let _l = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let Some(_server) = XServer::start() else {
        return;
    };
    let _video = Video::init();
    match crate::video::vulkan::vulkan_load_library(Option::None) {
        Ok(()) => {
            let ext = crate::video::vulkan::vulkan_instance_extensions().unwrap();
            assert_eq!(ext[0], "VK_KHR_surface");
            assert!(ext[1] == "VK_KHR_xlib_surface" || ext[1] == "VK_KHR_xcb_surface");
            assert_ne!(
                crate::video::vulkan::vulkan_get_vk_get_instance_proc_addr().unwrap(),
                0
            );
            crate::video::vulkan::vulkan_unload_library();
        }
        Err(e) => eprintln!(
            "note: no Vulkan loader with surface support: {}",
            e.message()
        ),
    }
}

#[test]
fn x11_wait_and_wakeup() {
    let _l = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let Some(_server) = XServer::start() else {
        return;
    };
    let _video = Video::init();
    let w = SdlWindow::create("wait", 50, 50, WindowFlags::default()).unwrap();
    drain();

    // Waiting with nothing to do times out
    let start = Instant::now();
    assert!(crate::events::wait_timeout(Some(Duration::from_millis(50)))
        .unwrap()
        .is_none());
    assert!(start.elapsed() >= Duration::from_millis(40));

    // An event pushed from another thread wakes the waiter up (through an
    // _SDL_WAKEUP client message sent on the request display)
    let user = crate::events::register_events(1).unwrap();
    let pusher = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        crate::events::push(Event::User(crate::events::UserEvent {
            event_type: user,
            code: 42,
            ..Default::default()
        }))
        .unwrap();
    });
    let start = Instant::now();
    let mut got = false;
    while start.elapsed() < Duration::from_secs(3) {
        match crate::events::wait_timeout(Some(Duration::from_secs(3))).unwrap() {
            Some(Event::User(u)) if u.code == 42 => {
                got = true;
                break;
            }
            _ => {}
        }
    }
    pusher.join().unwrap();
    assert!(got);
    assert!(
        start.elapsed() < Duration::from_secs(2),
        "{:?}",
        start.elapsed()
    );
    w.destroy();
}

#[test]
fn x11_fullscreen_and_popups() {
    let _l = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let Some(_server) = XServer::start() else {
        return;
    };
    let _video = Video::init();
    let primary = video::primary_display().unwrap();
    let bounds = video::display_bounds(primary).unwrap();

    let w = SdlWindow::create("fullscreen", 300, 200, WindowFlags::RESIZABLE).unwrap();
    drain();
    w.set_fullscreen(true).unwrap();
    w.sync().unwrap();
    drain();
    assert!(w.flags().unwrap().contains(WindowFlags::FULLSCREEN));
    assert_eq!(w.size().unwrap(), (bounds.w, bounds.h));
    w.set_fullscreen(false).unwrap();
    w.sync().unwrap();
    drain();
    assert!(!w.flags().unwrap().contains(WindowFlags::FULLSCREEN));
    assert_eq!(w.size().unwrap(), (300, 200));

    // A popup is placed relative to its parent
    w.set_position(100, 100).unwrap();
    w.sync().unwrap();
    let popup = SdlWindow::create_popup(&w, 10, 20, 50, 40, WindowFlags::POPUP_MENU).unwrap();
    popup.sync().unwrap();
    drain();
    let (px, py) = popup.position().unwrap();
    assert_eq!((px, py), (10, 20));
    let client = Client::open().unwrap();
    let xpopup = xwindow_of(&popup);
    // SAFETY: the client display is open; the out-parameters are valid.
    let (rx, ry) = unsafe {
        let mut rx = 0;
        let mut ry = 0;
        let mut child: Window = 0;
        (client.x.XTranslateCoordinates)(
            client.dpy,
            xpopup,
            DefaultRootWindow(client.dpy),
            0,
            0,
            &mut rx,
            &mut ry,
            &mut child,
        );
        (rx, ry)
    };
    let (wx, wy) = w.position().unwrap();
    assert_eq!((rx, ry), (wx + 10, wy + 20));
    popup.destroy();
    w.destroy();
}
