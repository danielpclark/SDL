// Tests for the video core: a scripted session on the dummy and offscreen
// drivers whose trace is compared with the same session run by upstream's
// C (testdata/video_trace.txt, from a program built against SDL with only
// those two drivers).

use std::fmt::Write as _;

use crate::error::Result;
use crate::events::window::WindowFlags;
use crate::events::{get_events, pump, DisplayID, Event, EventType};
use crate::hints;
use crate::init::{self, InitFlags};
use crate::properties::Properties;
use crate::video::clipboard;
use crate::video::gl::{self, GlAttr};
use crate::video::messagebox::{show_simple_message_box, MessageBoxFlags};
use crate::video::*;

/// The trace being built, with IDs renumbered by first appearance.
struct Trace {
    out: String,
    ids: Vec<u32>,
}

/// `%g` for the values the session prints.
fn g(v: f32) -> String {
    if v == v.trunc() && v.abs() < 1e9 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

impl Trace {
    fn rid(&mut self, id: u32) -> usize {
        if id == 0 {
            return 0;
        }
        if let Some(i) = self.ids.iter().position(|&x| x == id) {
            return i + 1;
        }
        self.ids.push(id);
        self.ids.len()
    }

    fn line(&mut self, s: impl AsRef<str>) {
        self.out.push_str(s.as_ref());
        self.out.push('\n');
    }

    fn res<T>(&mut self, label: &str, r: Result<T>) {
        match r {
            Ok(_) => self.line(format!("{label}: ok")),
            Err(e) => self.line(format!("{label}: err: {}", e.message())),
        }
    }

    fn events(&mut self) {
        pump();
        let events = get_events(EventType::FIRST, EventType::LAST, 10000).unwrap();
        for e in events {
            match e {
                Event::Window(w) => {
                    let mut d1 = w.data1;
                    if w.event_type == EventType::WINDOW_DISPLAY_CHANGED {
                        d1 = self.rid(d1 as u32) as i32;
                    }
                    let win = self.rid(w.window_id);
                    self.line(format!(
                        "  event 0x{:x} win={} {} {}",
                        w.event_type.0, win, d1, w.data2
                    ));
                }
                Event::Display(d) => {
                    let id = self.rid(d.display_id);
                    self.line(format!(
                        "  event 0x{:x} display={} {} {}",
                        d.event_type.0, id, d.data1, d.data2
                    ));
                }
                Event::Clipboard(c) => {
                    self.line(format!(
                        "  event 0x{:x} owner={} n={}",
                        EventType::CLIPBOARD_UPDATE.0,
                        c.owner as i32,
                        c.mime_types.len()
                    ));
                    for m in &c.mime_types {
                        self.line(format!("    {m}"));
                    }
                }
                other => {
                    self.line(format!("  event 0x{:x}", other.event_type().0));
                }
            }
        }
    }

    fn display(&mut self, d: DisplayID) {
        let id = self.rid(d);
        // (the default name is the display ID, renumbered like the others)
        let mut name = display_name(d).unwrap();
        if name == d.to_string() {
            name = id.to_string();
        }
        self.line(format!("display {id} name={name}"));
        if let Ok(r) = display_bounds(d) {
            self.line(format!("  bounds {} {} {} {}", r.x, r.y, r.w, r.h));
        }
        if let Ok(r) = display_usable_bounds(d) {
            self.line(format!("  usable {} {} {} {}", r.x, r.y, r.w, r.h));
        }
        let m = desktop_display_mode(d).unwrap();
        let mid = self.rid(m.display_id);
        self.line(format!(
            "  desktop {} {}x{} fmt=0x{:x} density={} rate={} {}/{}",
            mid,
            m.w,
            m.h,
            m.format.0,
            g(m.pixel_density),
            g(m.refresh_rate),
            m.refresh_rate_numerator,
            m.refresh_rate_denominator
        ));
        let m = current_display_mode(d).unwrap();
        self.line(format!("  current {}x{} fmt=0x{:x}", m.w, m.h, m.format.0));
        self.line(format!(
            "  scale={} natural={} current={}",
            g(display_content_scale(d).unwrap()),
            natural_display_orientation(d) as i32,
            current_display_orientation(d) as i32
        ));
        self.line(format!(
            "  modes={}",
            fullscreen_display_modes(d).unwrap().len()
        ));
    }

    fn window(&mut self, w: &Window) {
        let (x, y) = w.position().unwrap();
        let (ww, hh) = w.size().unwrap();
        let (pw, ph) = w.size_in_pixels().unwrap();
        let (minw, minh) = w.minimum_size().unwrap();
        let (maxw, maxh) = w.maximum_size().unwrap();
        let (mina, maxa) = w.aspect_ratio().unwrap();
        let r = w.safe_area().unwrap();
        let fm = w.fullscreen_mode().unwrap();
        let id = self.rid(w.id());
        self.line(format!(
            "window {} flags=0x{:x} pos={},{} size={}x{} pixels={}x{}",
            id,
            w.flags().unwrap().0,
            x,
            y,
            ww,
            hh,
            pw,
            ph
        ));
        let display = self.rid(w.display().unwrap());
        self.line(format!(
            "  density={} scale={} display={} fmt=0x{:x} title='{}' opacity={}",
            g(w.pixel_density().unwrap()),
            g(w.display_scale().unwrap()),
            display,
            w.pixel_format().unwrap().0,
            w.title().unwrap(),
            g(w.opacity().unwrap())
        ));
        let parent = match w.parent().unwrap() {
            Some(p) => self.rid(p.id()),
            None => 0,
        };
        self.line(format!(
            "  min={}x{} max={}x{} aspect={},{} safe={},{},{},{} surface={} kgrab={} mgrab={} text={} fsmode={} parent={}",
            minw,
            minh,
            maxw,
            maxh,
            g(mina),
            g(maxa),
            r.x,
            r.y,
            r.w,
            r.h,
            w.has_surface().unwrap() as i32,
            w.keyboard_grab().unwrap() as i32,
            w.mouse_grab().unwrap() as i32,
            w.text_input_active().unwrap() as i32,
            if fm.is_some() { "yes" } else { "none" },
            parent
        ));
    }

    fn created(&mut self, label: &str, w: &Result<Window>) {
        match w {
            Ok(_) => self.line(format!("{label} 1: ")),
            Err(e) => self.line(format!("{label} 0: {}", e.message())),
        }
    }
}

fn session(t: &mut Trace, driver: &str) {
    t.line(format!("==== driver {driver}"));
    hints::set(hints::VIDEO_DRIVER, driver).unwrap();
    t.res("init", init::init(InitFlags::VIDEO));
    t.line(format!(
        "current={} screensaver={} theme={}",
        current_video_driver().unwrap(),
        screen_saver_enabled() as i32,
        system_theme() as i32
    ));
    let ds = displays().unwrap();
    let primary = t.rid(primary_display().unwrap());
    t.line(format!("displays={} primary={}", ds.len(), primary));
    t.display(ds[0]);
    let for_point = t.rid(display_for_point(Point { x: 2000, y: 300 }).unwrap());
    let for_rect = t.rid(display_for_rect(&Rect::new(10, 10, 1, 1)).unwrap());
    t.line(format!("for point={for_point} for rect={for_rect}"));
    t.res(
        "closest",
        closest_fullscreen_display_mode(ds[0], 640, 480, 0.0, false),
    );
    t.events();

    t.line("== create A");
    let a = Window::create("Window A", 640, 480, WindowFlags::RESIZABLE).unwrap();
    t.line("created 1");
    t.events();
    t.window(&a);

    t.line("== position");
    t.res("setpos", a.set_position(100, 50));
    t.events();
    t.window(&a);
    t.res(
        "setpos centered",
        a.set_position(WINDOWPOS_CENTERED, WINDOWPOS_UNDEFINED),
    );
    t.events();
    t.window(&a);

    t.line("== size");
    t.res("setsize", a.set_size(800, 600));
    t.events();
    t.window(&a);
    t.res("setsize 0", a.set_size(0, 10));
    t.res("minsize", a.set_minimum_size(200, 100));
    t.events();
    t.res("maxsize", a.set_maximum_size(1000, 900));
    t.events();
    t.res("maxsize bad", a.set_maximum_size(100, 900));
    t.res("setsize clamp", a.set_size(50, 5000));
    t.events();
    t.window(&a);
    t.res("aspect", a.set_aspect_ratio(1.0, 1.5));
    t.events();
    t.res("setsize aspect", a.set_size(900, 300));
    t.events();
    t.window(&a);
    t.res("aspect off", a.set_aspect_ratio(0.0, 0.0));
    t.events();

    t.line("== states");
    t.res("title", a.set_title("Renamed"));
    t.res("bordered", a.set_bordered(false));
    t.res("resizable", a.set_resizable(false));
    t.res("on top", a.set_always_on_top(true));
    t.res("maximize", a.maximize());
    t.res("minimize", a.minimize());
    t.res("restore", a.restore());
    t.res("opacity", a.set_opacity(0.5));
    t.res("modal", a.set_modal(true));
    t.res("parent self", a.set_parent(Some(&a)));
    t.res("flash", a.flash(FlashOperation::Briefly));
    t.res("raise", a.raise());
    t.res("sync", a.sync());
    t.res("focusable", a.set_focusable(false));
    t.res("mouse rect", a.set_mouse_rect(None));
    t.res("hit test", a.set_hit_test(None));
    t.res("shape", a.set_shape(None));
    t.res("progress", a.set_progress_value(2.0));
    t.line(format!(
        "progress={} state={}",
        g(a.progress_value().unwrap()),
        a.progress_state().unwrap() as i32
    ));
    t.res("system menu", a.show_system_menu(0, 0));
    t.res("borders", a.borders_size());
    t.events();
    t.window(&a);

    t.line("== hide/show");
    t.res("hide", a.hide());
    t.events();
    t.window(&a);
    t.res("fullscreen hidden", a.set_fullscreen(true));
    t.res("kgrab hidden", a.set_keyboard_grab(true));
    t.window(&a);
    t.res("show", a.show());
    t.events();
    t.window(&a);

    t.line("== fullscreen");
    t.res("leave", a.set_fullscreen(false));
    t.events();
    t.window(&a);
    t.res("enter", a.set_fullscreen(true));
    t.events();
    t.window(&a);
    let mode = DisplayMode {
        w: 640,
        h: 480,
        ..Default::default()
    };
    t.res("fsmode", a.set_fullscreen_mode(Some(&mode)));
    t.res("fsmode none", a.set_fullscreen_mode(None));
    t.res("leave", a.set_fullscreen(false));
    t.events();
    t.window(&a);

    t.line("== grabs");
    t.res("kgrab", a.set_keyboard_grab(false));
    t.res("mgrab", a.set_mouse_grab(true));
    let grabbed = match grabbed_window() {
        Some(w) => t.rid(w.id()),
        None => 0,
    };
    t.line(format!("grabbed={grabbed}"));
    t.res("relative", a.set_relative_mouse_mode(true));
    t.line(format!(
        "relative={}",
        a.relative_mouse_mode().unwrap() as i32
    ));
    t.res("relative off", a.set_relative_mouse_mode(false));
    t.res("mgrab off", a.set_mouse_grab(false));
    t.events();
    t.window(&a);

    t.line("== surface");
    let s = a.surface().unwrap();
    {
        let guard = s.lock();
        t.line(format!(
            "surface {}x{} fmt=0x{:x} pitch={}",
            guard.width(),
            guard.height(),
            guard.format().0,
            guard.pitch()
        ));
    }
    t.res("fill", s.lock().fill_rect(None, 0x00ff00));
    t.res("update", a.update_surface());
    t.res(
        "update rects",
        a.update_surface_rects(&[Rect::new(1, 2, 3, 4)]),
    );
    t.res("vsync", a.set_surface_vsync(1));
    t.res("resize", a.set_size(320, 200));
    t.events();
    t.res("update stale", s.update());
    let s = a.surface().unwrap();
    {
        let guard = s.lock();
        t.line(format!(
            "surface {}x{} fmt=0x{:x} pitch={}",
            guard.width(),
            guard.height(),
            guard.format().0,
            guard.pitch()
        ));
    }
    t.res("destroy surface", a.destroy_surface());
    t.window(&a);

    t.line("== text input");
    t.res("start text", a.start_text_input());
    t.res(
        "text area",
        a.set_text_input_area(Some(&Rect::new(5, 6, 7, 8)), 3),
    );
    let (tr, cursor) = a.text_input_area().unwrap();
    t.line(format!(
        "area {} {} {} {} cursor {} shown={} support={}",
        tr.x,
        tr.y,
        tr.w,
        tr.h,
        cursor,
        a.screen_keyboard_shown().unwrap() as i32,
        textinput::has_screen_keyboard_support() as i32
    ));
    t.res("clear composition", a.clear_composition());
    t.res("stop text", a.stop_text_input());
    t.events();
    t.window(&a);

    t.line("== more windows");
    let bw = Window::create("B", 300, 200, WindowFlags::HIDDEN | WindowFlags::FULLSCREEN).unwrap();
    t.events();
    t.window(&bw);
    let c = Window::create_popup(&a, 10, 10, 50, 50, WindowFlags::TOOLTIP);
    t.created("popup", &c);
    let c = Window::create_popup(&a, 10, 10, 50, 50, WindowFlags::NONE);
    t.created("popup", &c);
    let c = Window::create("C", 10, 10, WindowFlags::MODAL);
    t.created("modal", &c);
    let c = Window::create("C", 10, 10, WindowFlags::OPENGL);
    t.created("gl", &c);
    let c = Window::create("C", 10, 10, WindowFlags::UTILITY | WindowFlags::MODAL);
    t.created("conflict", &c);
    let props = Properties::new();
    props
        .set(window::PROP_WINDOW_CREATE_TITLE_STRING, "D")
        .unwrap();
    props
        .set(
            window::PROP_WINDOW_CREATE_X_NUMBER,
            WINDOWPOS_CENTERED as i64,
        )
        .unwrap();
    props
        .set(window::PROP_WINDOW_CREATE_Y_NUMBER, 30i64)
        .unwrap();
    props
        .set(window::PROP_WINDOW_CREATE_WIDTH_NUMBER, 2000i64)
        .unwrap();
    props
        .set(window::PROP_WINDOW_CREATE_HEIGHT_NUMBER, 100i64)
        .unwrap();
    props
        .set(window::PROP_WINDOW_CREATE_FOCUSABLE_BOOLEAN, false)
        .unwrap();
    props
        .set(window::PROP_WINDOW_CREATE_BORDERLESS_BOOLEAN, true)
        .unwrap();
    let d = Window::create_with_properties(&props).unwrap();
    t.events();
    t.window(&d);
    let ws = windows().unwrap();
    let mut line = format!("windows={}:", ws.len());
    for w in &ws {
        let id = t.rid(w.id());
        let _ = write!(line, " {id}");
    }
    t.line(line);
    let same = Window::from_id(d.id()).ok() == Some(d);
    let bad = Window::from_id(12345);
    t.line(format!(
        "from id={} bad={} {}",
        same as i32,
        bad.is_ok() as i32,
        bad.err()
            .map(|e| e.message().to_owned())
            .unwrap_or_default()
    ));
    t.res("show B", bw.show());
    t.events();
    t.window(&bw);
    t.window(&a);

    t.line("== clipboard");
    t.line(format!(
        "has text={}",
        clipboard::has_clipboard_text().unwrap() as i32
    ));
    t.res("set text", clipboard::set_clipboard_text("hello"));
    t.events();
    t.line(format!(
        "text='{}' has={}",
        clipboard::clipboard_text().unwrap(),
        clipboard::has_clipboard_text().unwrap() as i32
    ));
    let callback: clipboard::ClipboardDataCallback =
        std::sync::Arc::new(|mime: &str| Some(format!("data for {mime}").into_bytes()));
    t.res(
        "set data",
        clipboard::set_clipboard_data(Some(callback), &["text/plain", "image/png"]),
    );
    t.events();
    let data = clipboard::clipboard_data("image/png").unwrap();
    t.line(format!(
        "data='{}' size={} has png={} has jpg={}",
        data.as_deref()
            .map(String::from_utf8_lossy)
            .unwrap_or_default(),
        data.as_ref().map_or(0, Vec::len),
        clipboard::has_clipboard_data("image/png").unwrap() as i32,
        clipboard::has_clipboard_data("image/jpeg").unwrap() as i32
    ));
    t.line(format!("text='{}'", clipboard::clipboard_text().unwrap()));
    t.line(format!(
        "mime types={}",
        clipboard::clipboard_mime_types().unwrap().len()
    ));
    t.res(
        "bad data",
        clipboard::set_clipboard_data(None, &["text/plain", "image/png"]),
    );
    t.res("clear", clipboard::clear_clipboard_data());
    t.events();
    t.res("primary", clipboard::set_primary_selection_text("sel"));
    t.events();
    t.line(format!(
        "primary='{}' has={}",
        clipboard::primary_selection_text().unwrap(),
        clipboard::has_primary_selection_text().unwrap() as i32
    ));

    t.line("== screensaver");
    t.res("enable", enable_screen_saver());
    t.line(format!("enabled={}", screen_saver_enabled() as i32));
    t.res("disable", disable_screen_saver());
    t.line(format!("enabled={}", screen_saver_enabled() as i32));

    t.line("== message box");
    t.res(
        "msgbox",
        show_simple_message_box(MessageBoxFlags::ERROR, "t", "m", Some(&a)),
    );

    t.line("== gl");
    t.res("gl attr", gl::gl_set_attribute(GlAttr::RedSize, 8));
    t.res("gl load", gl::gl_load_library(None));
    match gl::gl_create_context(&a) {
        Ok(_) => t.line("gl ctx=1: "),
        Err(e) => t.line(format!("gl ctx=0: {}", e.message())),
    }
    t.res("vk load", vulkan::vulkan_load_library(None));

    t.line("== destroy");
    bw.destroy();
    t.events();
    d.destroy();
    t.events();
    a.destroy();
    t.events();
    match a.flags() {
        Ok(_) => t.line("valid after destroy: 1 "),
        Err(e) => t.line(format!("valid after destroy: 0 {}", e.message())),
    }
    init::quit();
    match current_video_driver() {
        Ok(_) => t.line("after quit: still"),
        Err(e) => t.line(format!("after quit: {}", e.message())),
    }
}

#[test]
fn session_matches_c() {
    let _l = crate::test_support::test_lock();
    init::quit(); // (in case an earlier test failed halfway)
    let mut t = Trace {
        out: String::new(),
        ids: Vec::new(),
    };
    session(&mut t, "dummy");
    // The C reference's offscreen driver was built without EGL: here EGL is
    // made to fail, and its error stands in for "not available" (below).
    #[cfg(all(unix, not(target_vendor = "apple")))]
    hints::set(hints::EGL_LIBRARY, "libSDL-test-no-such-EGL.so").unwrap();
    session(&mut t, "offscreen");
    #[cfg(all(unix, not(target_vendor = "apple")))]
    {
        hints::reset(hints::EGL_LIBRARY);
        let offscreen = t.out.find("==== driver offscreen").unwrap();
        let (before, after) = t.out.split_at(offscreen);
        let mut out = before.to_owned();
        for line in after.lines() {
            if line.starts_with("gl 0: ") {
                out.push_str("gl 0: OpenGL support is either not configured in SDL or not available in current SDL video driver (offscreen) or platform");
            } else if line.starts_with("gl load: err: ") {
                out.push_str("gl load: err: No dynamic OpenGL support in current SDL video driver (offscreen)");
            } else {
                out.push_str(line);
            }
            out.push('\n');
        }
        t.out = out;
    }
    hints::set(hints::VIDEO_DRIVER, "nonexistent").unwrap();
    t.res("init bad", init::init(InitFlags::VIDEO));
    hints::reset(hints::VIDEO_DRIVER);
    let no_display = crate::test_support::NoDisplay::new();
    t.res("init none", init::init(InitFlags::VIDEO));
    drop(no_display);
    init::quit();

    let expected = include_str!("testdata/video_trace.txt");
    // (on Windows the windows driver is available without asking for it)
    #[cfg(windows)]
    let expected_windows =
        expected.replace("init none: err: No available video device", "init none: ok");
    #[cfg(windows)]
    let expected: &str = &expected_windows;
    if t.out != expected {
        let out = std::env::temp_dir().join("rust_video_trace.txt");
        std::fs::write(&out, &t.out).unwrap();
        for (i, (a, b)) in t.out.lines().zip(expected.lines()).enumerate() {
            if a != b {
                panic!(
                    "trace differs at line {}:\n  rust: {a}\n  C:    {b}\n(full trace in {})",
                    i + 1,
                    out.display()
                );
            }
        }
        panic!(
            "trace lengths differ: {} vs {} lines (full trace in {})",
            t.out.lines().count(),
            expected.lines().count(),
            out.display()
        );
    }
}

// ---------------------------------------------------------------------------
// A test driver for what the dummy driver can't reach: two displays with
// fullscreen modes, mode switching, popups, parents and modal windows.
// ---------------------------------------------------------------------------

use std::sync::{Arc, Mutex};

use crate::events::WindowID;
use crate::video::core::with_window;
use crate::video::display::{add_fullscreen_display_mode, add_video_display};
use crate::video::sysvideo::{DeviceCaps, VideoBootStrap, VideoDisplay, VideoDriver};
use crate::video::window::WindowOp;

/// The display modes the test driver was asked to switch to.
static MODE_SWITCHES: Mutex<Vec<(i32, i32)>> = Mutex::new(Vec::new());

struct TestVideo;

fn mode(w: i32, h: i32, rate: f32) -> DisplayMode {
    DisplayMode {
        format: PixelFormat::XRGB8888,
        w,
        h,
        refresh_rate: rate,
        ..Default::default()
    }
}

impl VideoDriver for TestVideo {
    fn caps(&self) -> DeviceCaps {
        DeviceCaps::HAS_POPUP_WINDOW_SUPPORT
    }

    fn video_init(&self) -> Result<()> {
        let mut primary = VideoDisplay::new();
        primary.name = Some("Primary".into());
        primary.desktop_mode = mode(1920, 1080, 60.0);
        add_video_display(primary, false);
        let mut second = VideoDisplay::new();
        second.desktop_mode = mode(1280, 1024, 75.0);
        second.content_scale = 2.0;
        add_video_display(second, false);
        Ok(())
    }

    fn video_quit(&self) {}

    fn display_modes(&self, display: DisplayID) -> Option<()> {
        if display == primary_display().ok()? {
            for m in [
                mode(800, 600, 60.0),
                mode(1920, 1080, 60.0),
                mode(1280, 720, 60.0),
                mode(1280, 720, 30.0),
                mode(1280, 720, 60.0),
            ] {
                add_fullscreen_display_mode(display, &m);
            }
        }
        Some(())
    }

    fn set_display_mode(&self, _display: DisplayID, mode: &DisplayMode) -> Option<Result<()>> {
        MODE_SWITCHES.lock().unwrap().push((mode.w, mode.h));
        Some(Ok(()))
    }

    fn set_window_position(&self, window: WindowID) -> Option<Result<()>> {
        let pending = with_window(window, |w| w.pending).ok()?;
        crate::events::window::send_window_event(
            window,
            EventType::WINDOW_MOVED,
            pending.x,
            pending.y,
        );
        Some(Ok(()))
    }

    fn set_window_size(&self, window: WindowID) -> Option<()> {
        let pending = with_window(window, |w| w.pending).ok()?;
        crate::events::window::send_window_event(
            window,
            EventType::WINDOW_RESIZED,
            pending.w,
            pending.h,
        );
        Some(())
    }

    fn set_window_parent(
        &self,
        _window: WindowID,
        _parent: Option<WindowID>,
    ) -> Option<Result<()>> {
        Some(Ok(()))
    }

    fn set_window_modal(&self, _window: WindowID, _modal: bool) -> Option<Result<()>> {
        Some(Ok(()))
    }

    fn implements_window_op(&self, op: WindowOp) -> bool {
        matches!(op, WindowOp::SetParent | WindowOp::SetModal)
    }

    fn create_cursor(
        &self,
        surface: &Surface<'_>,
        hot_x: i32,
        hot_y: i32,
    ) -> Option<Result<crate::events::mouse::Cursor>> {
        let pixels = (0..surface.height())
            .flat_map(|y| (0..surface.width()).map(move |x| (x, y)))
            .map(|(x, y)| {
                let c = surface.read_pixel(x, y).unwrap();
                u32::from_be_bytes([c.a, c.r, c.g, c.b])
            })
            .collect();
        let mut cursors = CURSORS.lock().unwrap();
        cursors.push(CursorImage {
            format: surface.format(),
            w: surface.width(),
            h: surface.height(),
            hot: (hot_x, hot_y),
            pixels,
        });
        Some(Ok(crate::events::mouse::Cursor::with_internal(
            cursors.len() - 1,
        )))
    }

    fn show_cursor(&self, cursor: Option<&crate::events::mouse::Cursor>) -> Option<Result<()>> {
        SHOWN_CURSORS
            .lock()
            .unwrap()
            .push(cursor.and_then(|c| c.internal::<usize>().copied()));
        Some(Ok(()))
    }
}

/// A cursor image the test driver was given.
struct CursorImage {
    format: PixelFormat,
    w: i32,
    h: i32,
    hot: (i32, i32),
    pixels: Vec<u32>,
}

/// The cursors the test driver created, and the ones it was asked to show
/// (by index; `None` hides the cursor).
static CURSORS: Mutex<Vec<CursorImage>> = Mutex::new(Vec::new());
static SHOWN_CURSORS: Mutex<Vec<Option<usize>>> = Mutex::new(Vec::new());

fn testvideo_create() -> Option<Arc<dyn VideoDriver>> {
    crate::video::drivers::dummy::available("testvideo")
        .then(|| Arc::new(TestVideo) as Arc<dyn VideoDriver>)
}

pub(super) static TESTVIDEO_BOOTSTRAP: VideoBootStrap = VideoBootStrap {
    name: "testvideo",
    desc: "SDL test video driver",
    create: testvideo_create,
    show_message_box: None,
    is_preferred: false,
};

fn start_test_driver() {
    init::quit(); // (in case an earlier test failed halfway)
    hints::set(hints::VIDEO_DRIVER, "testvideo").unwrap();
    init::init(InitFlags::VIDEO).unwrap();
    MODE_SWITCHES.lock().unwrap().clear();
    let _ = get_events(EventType::FIRST, EventType::LAST, 10000);
}

fn window_events() -> Vec<(EventType, i32, i32)> {
    pump();
    get_events(EventType::WINDOW_FIRST, EventType::WINDOW_LAST, 10000)
        .unwrap()
        .into_iter()
        .filter_map(|e| match e {
            Event::Window(w) => Some((w.event_type, w.data1, w.data2)),
            _ => None,
        })
        .collect()
}

#[test]
fn displays_and_modes() {
    let _l = crate::test_support::test_lock();
    start_test_driver();

    let ds = displays().unwrap();
    assert_eq!(ds.len(), 2);
    assert_eq!(display_name(ds[0]).unwrap(), "Primary");
    assert_eq!(display_name(ds[1]).unwrap(), ds[1].to_string());
    // Displays are assumed to be left to right.
    assert_eq!(
        display_bounds(ds[1]).unwrap(),
        Rect::new(1920, 0, 1280, 1024)
    );
    assert_eq!(display_for_point(Point { x: 2000, y: 10 }).unwrap(), ds[1]);
    // (closest to the center of a rect off every display)
    assert_eq!(
        display_for_rect(&Rect::new(-500, 500, 10, 10)).unwrap(),
        ds[0]
    );
    assert_eq!(display_content_scale(ds[1]).unwrap(), 2.0);
    assert_eq!(
        desktop_display_mode(ds[1]).unwrap().refresh_rate_numerator,
        75
    );

    // Sorted largest first, duplicates dropped.
    let modes: Vec<(i32, i32, f32)> = fullscreen_display_modes(ds[0])
        .unwrap()
        .iter()
        .map(|m| (m.w, m.h, m.refresh_rate))
        .collect();
    assert_eq!(
        modes,
        vec![
            (1920, 1080, 60.0),
            (1280, 720, 60.0),
            (1280, 720, 30.0),
            (800, 600, 60.0)
        ]
    );
    let closest = closest_fullscreen_display_mode(ds[0], 1000, 700, 0.0, false).unwrap();
    assert_eq!(
        (closest.w, closest.h, closest.refresh_rate),
        (1280, 720, 60.0)
    );
    let closest = closest_fullscreen_display_mode(ds[0], 1000, 700, 25.0, false).unwrap();
    assert_eq!(closest.refresh_rate, 30.0);
    assert!(closest_fullscreen_display_mode(ds[0], 4000, 700, 0.0, false).is_err());

    // A window moved onto the second display changes display, and gets its scale.
    let w = Window::create("moving", 640, 480, WindowFlags::NONE).unwrap();
    assert_eq!(w.display().unwrap(), ds[0]);
    assert_eq!(w.display_scale().unwrap(), 1.0);
    window_events();
    w.set_position(2000, 100).unwrap();
    let events = window_events();
    assert!(events.contains(&(EventType::WINDOW_DISPLAY_CHANGED, ds[1] as i32, 0)));
    assert!(events.contains(&(EventType::WINDOW_DISPLAY_SCALE_CHANGED, 0, 0)));
    assert_eq!(w.display().unwrap(), ds[1]);
    assert_eq!(w.display_scale().unwrap(), 2.0);
    // Mostly on the first display: not committed to the new one yet.
    w.set_position(1920 - 600, 100).unwrap();
    assert_eq!(w.display().unwrap(), ds[0]);
    w.destroy();
    init::quit();
}

#[test]
fn exclusive_fullscreen() {
    let _l = crate::test_support::test_lock();
    start_test_driver();
    let primary = primary_display().unwrap();

    let w = Window::create("fs", 640, 480, WindowFlags::NONE).unwrap();
    // (a mode must match a listed one, format included, as upstream)
    let partial = DisplayMode {
        w: 1280,
        h: 720,
        refresh_rate: 60.0,
        ..Default::default()
    };
    assert!(w.set_fullscreen_mode(Some(&partial)).is_err());
    let mode = fullscreen_display_modes(primary).unwrap()[1];
    assert_eq!((mode.w, mode.h, mode.refresh_rate), (1280, 720, 60.0));
    w.set_fullscreen_mode(Some(&mode)).unwrap();
    assert_eq!(
        w.fullscreen_mode().unwrap().map(|m| (m.w, m.h)),
        Some((1280, 720))
    );
    window_events();
    pump();
    let _ = get_events(EventType::FIRST, EventType::LAST, 10000);

    w.set_fullscreen(true).unwrap();
    assert_eq!(*MODE_SWITCHES.lock().unwrap(), vec![(1280, 720)]);
    assert_eq!(current_display_mode(primary).unwrap().w, 1280);
    assert_eq!(w.size().unwrap(), (1280, 720));
    // A mode added (sorted in ahead of the current one) while it is in use
    // leaves the current mode alone.
    assert!(add_fullscreen_display_mode(
        primary,
        &DisplayMode {
            w: 2560,
            h: 1440,
            ..mode
        }
    ));
    assert_eq!(fullscreen_display_modes(primary).unwrap()[2].w, 1280);
    assert_eq!(
        current_display_mode(primary).unwrap().w,
        1280,
        "the current mode moved"
    );
    assert!(w.flags().unwrap().contains(WindowFlags::FULLSCREEN));
    pump();
    let display_events: Vec<EventType> =
        get_events(EventType::DISPLAY_FIRST, EventType::DISPLAY_LAST, 100)
            .unwrap()
            .iter()
            .map(|e| e.event_type())
            .collect();
    assert_eq!(
        display_events,
        vec![EventType::DISPLAY_CURRENT_MODE_CHANGED]
    );

    // Leaving restores the desktop mode and the windowed size.
    w.set_fullscreen(false).unwrap();
    assert_eq!(
        *MODE_SWITCHES.lock().unwrap(),
        vec![(1280, 720), (1920, 1080)]
    );
    assert_eq!(current_display_mode(primary).unwrap().w, 1920);
    assert_eq!(w.size().unwrap(), (640, 480));

    // A mode the display doesn't have is refused.
    let bad = DisplayMode {
        w: 1234,
        h: 567,
        ..Default::default()
    };
    assert_eq!(
        w.set_fullscreen_mode(Some(&bad)).unwrap_err().message(),
        "Invalid fullscreen display mode"
    );
    w.destroy();
    init::quit();
}

#[test]
fn popups_parents_and_modals() {
    let _l = crate::test_support::test_lock();
    start_test_driver();

    let parent = Window::create("parent", 640, 480, WindowFlags::NONE).unwrap();
    parent.set_position(100, 200).unwrap();
    let popup = Window::create_popup(&parent, 10, 20, 50, 50, WindowFlags::POPUP_MENU).unwrap();
    assert_eq!(popup.parent().unwrap(), Some(parent));
    assert_eq!(popup.position().unwrap(), (10, 20));
    assert_eq!(
        crate::video::window::relative_to_global_for_window(popup.id(), 10, 20),
        (110, 220)
    );
    assert_eq!(
        popup.set_title("no").unwrap_err().message(),
        "Operation invalid on popup windows"
    );

    // Hiding the parent hides the popup, and showing it restores the popup.
    parent.hide().unwrap();
    assert!(popup.flags().unwrap().contains(WindowFlags::HIDDEN));
    parent.show().unwrap();
    assert!(!popup.flags().unwrap().contains(WindowFlags::HIDDEN));

    // Modal windows need a parent and can't change it.
    let child = Window::create("child", 100, 100, WindowFlags::NONE).unwrap();
    assert!(child.set_modal(true).is_err());
    child.set_parent(Some(&parent)).unwrap();
    child.set_modal(true).unwrap();
    assert!(child.flags().unwrap().contains(WindowFlags::MODAL));
    assert!(child.set_parent(None).is_err());
    child.set_modal(false).unwrap();
    child.set_parent(None).unwrap();
    assert_eq!(child.parent().unwrap(), None);

    // Destroying the parent destroys its popups.
    parent.destroy();
    assert!(!popup.is_valid());
    assert!(child.is_valid());
    assert_eq!(windows().unwrap(), vec![child]);
    init::quit();
}

// ---------------------------------------------------------------------------
// Cursors
// ---------------------------------------------------------------------------

use crate::events::mouse::{self as mousemod, CursorFrame, SystemCursor};

/// The cursor calls of a session upstream's C was run through on the
/// dummy driver, with its output.
#[test]
fn cursors_match_c() {
    let _l = crate::test_support::test_lock();
    init::quit(); // (in case an earlier test failed halfway)
    hints::set(hints::VIDEO_DRIVER, "dummy").unwrap();
    init::init(InitFlags::VIDEO).unwrap();

    let mut out = String::new();
    let mut cr = |label: &str, c: &Result<mousemod::Cursor>| {
        let r = match c {
            Ok(_) => "ok".to_string(),
            Err(e) => e.message().to_string(),
        };
        writeln!(out, "{label}: {r}").unwrap();
    };

    let def = mousemod::default_cursor();
    let data = [0xF0, 0x0F, 0xAA, 0x55, 0xFF, 0x00];
    let mask = [0xFF, 0xFF, 0x0F, 0xF0, 0x00, 0xFF];
    let c1 = mousemod::create_cursor(&data, &mask, 12, 3, 1, 1);
    cr("mono", &c1);
    cr(
        "mono hot out",
        &mousemod::create_cursor(&data, &mask, 12, 3, 16, 0),
    );
    cr(
        "mono hot in padding",
        &mousemod::create_cursor(&data, &mask, 12, 3, 15, 2),
    );
    cr(
        "mono hot neg",
        &mousemod::create_cursor(&data, &mask, 12, 3, -1, 0),
    );
    let mut s = Surface::new(8, 8, PixelFormat::RGB565).unwrap();
    cr("color 565", &mousemod::create_color_cursor(&s, 7, 7));
    cr("color out", &mousemod::create_color_cursor(&s, 8, 0));
    s.properties()
        .set(crate::video::surface::PROP_SURFACE_HOTSPOT_X_NUMBER, 20i64)
        .unwrap();
    cr("color prop out", &mousemod::create_color_cursor(&s, 0, 0));
    s.properties()
        .set(crate::video::surface::PROP_SURFACE_HOTSPOT_X_NUMBER, 3i64)
        .unwrap();
    cr("color prop in", &mousemod::create_color_cursor(&s, 50, 0));
    let s2 = Surface::new(8, 4, PixelFormat::ARGB8888).unwrap();
    let s3 = Surface::new(8, 8, PixelFormat::ABGR8888).unwrap();
    let frame = |surface, duration| CursorFrame { surface, duration };
    let frames = [frame(&s, 10), frame(&s3, 20), frame(&s, 0)];
    let anim = mousemod::create_animated_cursor(&frames, 0, 0);
    cr("animated", &anim);
    cr(
        "animated one",
        &mousemod::create_animated_cursor(&frames[..1], 0, 0),
    );
    cr(
        "animated zero",
        &mousemod::create_animated_cursor(&[], 0, 0),
    );
    cr(
        "animated sizes",
        &mousemod::create_animated_cursor(&[frame(&s, 10), frame(&s2, 20)], 0, 0),
    );
    cr(
        "animated hot",
        &mousemod::create_animated_cursor(&[frame(&s2, 10), frame(&s2, 20)], 0, 5),
    );
    let anim = anim.unwrap();
    writeln!(
        out,
        "set anim: {}",
        mousemod::set_cursor(Some(&anim)).is_ok() as i32
    )
    .unwrap();
    writeln!(
        out,
        "cur is anim={}",
        (mousemod::cursor() == Some(anim.clone())) as i32
    )
    .unwrap();
    pump();
    mousemod::destroy_cursor(anim);
    writeln!(
        out,
        "after destroy cur is default={}",
        (mousemod::cursor() == def) as i32
    )
    .unwrap();
    let c1 = c1.unwrap();
    writeln!(
        out,
        "set destroyed c1 ok={}",
        mousemod::set_cursor(Some(&c1)).is_ok() as i32
    )
    .unwrap();
    mousemod::destroy_cursor(c1.clone());
    match mousemod::set_cursor(Some(&c1)) {
        Ok(()) => writeln!(out, "set destroyed: 1").unwrap(),
        Err(e) => writeln!(out, "set destroyed: 0 {}", e.message()).unwrap(),
    }
    mousemod::destroy_cursor(def.clone().unwrap());
    writeln!(
        out,
        "default still={}",
        (mousemod::default_cursor() == def) as i32
    )
    .unwrap();
    let mut cr = |label: &str, c: &Result<mousemod::Cursor>| {
        let r = match c {
            Ok(_) => "ok".to_string(),
            Err(e) => e.message().to_string(),
        };
        writeln!(out, "{label}: {r}").unwrap();
    };
    cr(
        "system",
        &mousemod::create_system_cursor(SystemCursor::Wait),
    );
    init::quit();
    hints::reset(hints::VIDEO_DRIVER);

    assert_eq!(
        out,
        "mono: ok
mono hot out: Cursor hot spot doesn't lie within cursor
mono hot in padding: ok
mono hot neg: Cursor hot spot doesn't lie within cursor
color 565: ok
color out: Cursor hot spot doesn't lie within cursor
color prop out: Cursor hot spot doesn't lie within cursor
color prop in: ok
animated: ok
animated one: ok
animated zero: Parameter 'frame_count' is invalid
animated sizes: All frames in an animated sequence must have the same dimensions
animated hot: Cursor hot spot doesn't lie within cursor
set anim: 1
cur is anim=1
after destroy cur is default=1
set destroyed c1 ok=1
set destroyed: 0 Cursor not associated with the current mouse
default still=1
system: CreateSystemCursor is not currently supported
"
    );
}

#[test]
fn cursor_images_and_animation() {
    let _l = crate::test_support::test_lock();
    CURSORS.lock().unwrap().clear();
    start_test_driver();

    // The default cursor: a transparent 1x1 color cursor
    {
        let cursors = CURSORS.lock().unwrap();
        assert_eq!(cursors.len(), 1);
        assert_eq!((cursors[0].w, cursors[0].h, cursors[0].hot), (1, 1, (0, 0)));
        assert_eq!(cursors[0].pixels, vec![0]);
    }

    // A mono cursor: width rounded up to 8, one byte of data and mask per 8 pixels
    let data = [0b1100_0000, 0b1010_0000];
    let mask = [0b1010_0000, 0b1100_0000];
    let mono = mousemod::create_cursor(&data, &mask, 3, 2, 2, 1).unwrap();
    {
        let cursors = CURSORS.lock().unwrap();
        let c = cursors.last().unwrap();
        assert_eq!(
            (c.format, c.w, c.h, c.hot),
            (PixelFormat::ARGB8888, 8, 2, (2, 1))
        );
        let inverted = if cfg!(windows) {
            0x00FFFFFF
        } else {
            0xFF000000
        };
        let mut row0 = vec![0xFF000000, inverted, 0xFFFFFFFF];
        let mut row1 = vec![0xFF000000, 0xFFFFFFFF, inverted];
        row0.resize(8, 0);
        row1.resize(8, 0);
        assert_eq!(c.pixels, [row0, row1].concat());
    }
    assert!(mousemod::create_cursor(&data[..1], &mask, 3, 2, 0, 0).is_err());

    // Color cursors are converted to ARGB8888; the hotspot properties win
    let mut s = Surface::new(2, 2, PixelFormat::RGB565).unwrap();
    s.fill_rect(None, 0xF800).unwrap();
    s.properties()
        .set(crate::video::surface::PROP_SURFACE_HOTSPOT_Y_NUMBER, 1i64)
        .unwrap();
    let _color = mousemod::create_color_cursor(&s, 1, 0).unwrap();
    {
        let cursors = CURSORS.lock().unwrap();
        let c = cursors.last().unwrap();
        assert_eq!((c.format, c.hot), (PixelFormat::ARGB8888, (1, 1)));
        assert_eq!(c.pixels, vec![0xFFFF0000; 4]);
    }

    // An animated cursor is animated by the mouse code: a cursor per frame
    let a = Surface::new(2, 2, PixelFormat::ARGB8888).unwrap();
    let b = Surface::new(2, 2, PixelFormat::ABGR8888).unwrap();
    let first = CURSORS.lock().unwrap().len();
    let anim = mousemod::create_animated_cursor(
        &[
            CursorFrame {
                surface: &a,
                duration: 20,
            },
            CursorFrame {
                surface: &b,
                duration: 20,
            },
            CursorFrame {
                surface: &a,
                duration: 0,
            },
        ],
        0,
        0,
    )
    .unwrap();
    assert!(anim.is_animated());
    assert_eq!(CURSORS.lock().unwrap().len(), first + 3);

    // Without focus, the default cursor is shown and nothing animates
    SHOWN_CURSORS.lock().unwrap().clear();
    mousemod::set_cursor(Some(&anim)).unwrap();
    assert_eq!(*SHOWN_CURSORS.lock().unwrap(), vec![Some(0)]);

    let w = Window::create("cursor", 100, 100, WindowFlags::default()).unwrap();
    mousemod::set_mouse_focus(Some(w.id()));
    SHOWN_CURSORS.lock().unwrap().clear();
    mousemod::set_cursor(None).unwrap(); // (a redraw)
    assert_eq!(*SHOWN_CURSORS.lock().unwrap(), vec![Some(first)]);

    // Frames advance as events are pumped, and stop on a 0 duration
    for _ in 0..3 {
        std::thread::sleep(std::time::Duration::from_millis(25));
        pump();
    }
    let shown = SHOWN_CURSORS.lock().unwrap().clone();
    assert_eq!(shown, vec![Some(first), Some(first + 1), Some(first + 2)]);

    // Hiding shows no cursor
    mousemod::hide_cursor();
    assert_eq!(SHOWN_CURSORS.lock().unwrap().last(), Some(&None));
    mousemod::show_cursor();

    // Destroying the animation destroys its frames
    mousemod::destroy_cursor(anim.clone());
    assert!(mousemod::set_cursor(Some(&anim)).is_err());
    assert_eq!(mousemod::cursor(), mousemod::default_cursor());
    mousemod::destroy_cursor(mono);

    w.destroy();
    init::quit();
    hints::reset(hints::VIDEO_DRIVER);
}
