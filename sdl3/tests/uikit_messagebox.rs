// Tests of the UIKit video driver's message boxes: alert controllers,
// answered in the program and through idb.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The UIKit video driver only starts in an app, so this is a program of
//! its own (`harness = false`), run as an app on the iOS Simulator by
//! `tools/ios_run.py` (the `__DATA,__sdl3_runner` section below), as
//! `uikit_main` is. Its main function makes a window, sets its animation
//! callback (which counts frames), and shows message boxes, each answered
//! from the main queue while the box runs the main run loop: the program
//! finds the alert controller (the one a window's root view controller
//! presents), traces it (its title, message and style, its actions with
//! their styles and the preferred one, and what presents it), counts the
//! animation's frames for half a second (none while a box is up), then
//! chooses an action as a tap would: the alert dismissed and the
//! action's handler called (`handler`, UIAlertAction's private property:
//! test programs only). The boxes:
//!
//! * a simple message box (`show_simple_message_box()`);
//! * three buttons (the return and escape key defaults, and another),
//!   left to right and right to left, with the error and warning flags;
//! * no window (the box gets a window of its own, at the alerts' level,
//!   hidden once it's answered);
//! * empty title and message;
//! * one shown from another thread (on the main thread, as it pumps);
//! * a real tap on a button, through idb (the runner's `tap` request, at
//!   the middle of the button's label, when it drives idb: else skipped,
//!   `note: skipping, idb unavailable`, and the trace is compared without
//!   those lines).
//!
//! Each box's result (the button's ID), the events of the half second
//! after it (the devices' events aren't traced: the keyboard's comes
//! when it comes), and whether the frames resume are traced too. The
//! expected trace is upstream's C SDL's, from the same program in C run
//! the same way on the same simulator (an iPhone 16 with iOS 18.5).
//! Elsewhere this program does nothing.

#[cfg(not(target_os = "ios"))]
fn main() {}

/// The runner's words: run as an app.
#[cfg(target_os = "ios")]
#[used]
#[link_section = "__DATA,__sdl3_runner"]
static RUNNER: [u8; 4] = *b"app\0";

#[cfg(target_os = "ios")]
mod uikit_support;

#[cfg(target_os = "ios")]
fn main() {
    ios::main()
}

#[cfg(target_os = "ios")]
mod ios {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    use sdl3::events::window::WindowFlags;
    use sdl3::events::{self, Event, WindowID};
    use sdl3::init::{self, InitFlags};
    use sdl3::video::messagebox::{
        show_message_box, show_simple_message_box, MessageBoxButtonData, MessageBoxButtonFlags,
        MessageBoxData, MessageBoxFlags,
    };
    use sdl3::video::{self, Window};

    use super::uikit_support::objc::{self, Id};
    use super::uikit_support::{check_trace, checked, elapsed, event_name, exit, idb, request};

    /// Upstream's trace (the C program, on an iPhone 16 simulator with
    /// iOS 18.5).
    const EXPECTED: &str = "\
init video: ok
video driver: uikit
create: ok
created: events: DISPLAY_ORIENTATION(3), W:WINDOW_SAFE_AREA_CHANGED(0,0), W:WINDOW_MOUSE_ENTER(0,0), W:WINDOW_FOCUS_GAINED(0,0), W:WINDOW_SHOWN(0,0), W:WINDOW_PIXEL_SIZE_CHANGED(393,852)
animation: ok
animation: events: (none)
animation: animation frames resumed: yes
simple: alert: title=\"Simple\" message=\"A simple message box\" style=1
simple: action 0: \"OK\" style=1 enabled=yes preferred
simple: presented on: the window's SDL_uikitviewcontroller
simple: animation frames while up: 0
simple: answered: action 0 \"OK\"
simple: result: ok
simple: events: (none)
simple: animation frames resumed: yes
left to right: alert: title=\"Left to right\" message=\"Three buttons\" style=1
left to right: action 0: \"Yes\" style=0 enabled=yes preferred
left to right: action 1: \"No\" style=1 enabled=yes
left to right: action 2: \"Maybe\" style=0 enabled=yes
left to right: presented on: the window's SDL_uikitviewcontroller
left to right: animation frames while up: 0
left to right: answered: action 2 \"Maybe\"
left to right: result: ok id=30
left to right: events: (none)
left to right: animation frames resumed: yes
right to left: alert: title=\"Right to left\" message=\"Three buttons, reversed\" style=1
right to left: action 0: \"Maybe\" style=0 enabled=yes
right to left: action 1: \"No\" style=1 enabled=yes
right to left: action 2: \"Yes\" style=0 enabled=yes preferred
right to left: presented on: the window's SDL_uikitviewcontroller
right to left: animation frames while up: 0
right to left: answered: action 0 \"Maybe\"
right to left: result: ok id=30
right to left: events: (none)
right to left: animation frames resumed: yes
right to left, the cancel one: alert: title=\"Right to left\" message=\"Three buttons, reversed\" style=1
right to left, the cancel one: action 0: \"Maybe\" style=0 enabled=yes
right to left, the cancel one: action 1: \"No\" style=1 enabled=yes
right to left, the cancel one: action 2: \"Yes\" style=0 enabled=yes preferred
right to left, the cancel one: presented on: the window's SDL_uikitviewcontroller
right to left, the cancel one: animation frames while up: 0
right to left, the cancel one: answered: action 1 \"No\"
right to left, the cancel one: result: ok id=20
right to left, the cancel one: events: (none)
right to left, the cancel one: animation frames resumed: yes
no window: alert: title=\"No window\" message=\"On a window of its own\" style=1
no window: action 0: \"First\" style=0 enabled=yes
no window: action 1: \"Second\" style=0 enabled=yes preferred
no window: presented on: a UIWindow (level 2000, key yes, hidden no), root UIViewController
no window: animation frames while up: 0
no window: answered: action 1 \"Second\"
no window: result: ok id=8
no window: events: (none)
no window: animation frames resumed: yes
no window: its window: hidden yes, key no; the SDL window: key yes
empty texts: alert: title=\"\" message=\"\" style=1
empty texts: action 0: \"Only\" style=0 enabled=yes
empty texts: presented on: the window's SDL_uikitviewcontroller
empty texts: animation frames while up: 0
empty texts: answered: action 0 \"Only\"
empty texts: result: ok id=5
empty texts: events: (none)
empty texts: animation frames resumed: yes
thread: alert: title=\"Thread\" message=\"From another thread\" style=1
thread: action 0: \"Thread A\" style=1 enabled=yes
thread: action 1: \"Thread B\" style=0 enabled=yes
thread: presented on: the window's SDL_uikitviewcontroller
thread: animation frames while up: 0
thread: answered: action 1 \"Thread B\"
thread: result: ok id=200
thread: events: (none)
thread: animation frames resumed: yes
idb: tap: alert: title=\"Tap\" message=\"Tapped through idb\" style=1
idb: tap: action 0: \"Left\" style=0 enabled=yes
idb: tap: action 1: \"Right\" style=0 enabled=yes
idb: tap: presented on: the window's SDL_uikitviewcontroller
idb: tap: animation frames while up: 0
idb: tap: result: ok id=2
idb: tap: events: (none)
idb: tap: animation frames resumed: yes
animation stopped: ok
destroyed
destroyed: events: W:WINDOW_DESTROYED(0,0)
quit
";

    static TRACE: Mutex<Vec<String>> = Mutex::new(Vec::new());

    fn t(line: impl Into<String>) {
        let line = line.into();
        println!("trace: {line}");
        TRACE.lock().unwrap().push(line);
    }

    fn yes_no(b: bool) -> &'static str {
        if b {
            "yes"
        } else {
            "no"
        }
    }

    fn ok<T>(r: &sdl3::Result<T>) -> String {
        match r {
            Ok(_) => "ok".into(),
            Err(e) => format!("error: {e}"),
        }
    }

    /// An `NSString` as the trace quotes it (`nil` for nil): bytes below
    /// 0x20, 0x7f, quotes and backslashes escaped.
    fn quoted_ns(s: Id) -> String {
        let Some(text) = objc::string(s) else {
            return "nil".into();
        };
        let mut out = String::from("\"");
        for ch in text.chars() {
            if (ch as u32) < 0x20 || ch == '\x7f' || ch == '"' || ch == '\\' {
                out.push_str(&format!("\\x{:02x}", ch as u32));
            } else {
                out.push(ch);
            }
        }
        out.push('"');
        out
    }

    static WINDOW: Mutex<WindowID> = Mutex::new(0);

    /// An event as the trace names it (the C program's names), `None` for
    /// the devices' events.
    fn describe(event: &Event) -> Option<String> {
        let n = event_name(event.event_type());
        Some(match event {
            Event::MouseDevice(_) | Event::KeyboardDevice(_) => return None,
            Event::Display(d) => format!("{n}({})", d.data1),
            Event::Window(w) => format!(
                "{}:{n}({},{})",
                if w.window_id == *WINDOW.lock().unwrap() {
                    "W"
                } else {
                    "?"
                },
                w.data1,
                w.data2
            ),
            _ => n,
        })
    }

    fn take_queued(out: &mut Vec<String>) {
        events::pump();
        while let Some(event) = events::poll() {
            out.extend(describe(&event));
        }
    }

    /// Pump for `ms` milliseconds (10 ms steps), and trace the events.
    fn events(label: &str, ms: u64) {
        let mut seen = Vec::new();
        take_queued(&mut seen);
        for _ in 0..ms / 10 {
            std::thread::sleep(Duration::from_millis(10));
            take_queued(&mut seen);
        }
        if seen.is_empty() {
            t(format!("{label}: events: (none)"));
        } else {
            t(format!("{label}: events: {}", seen.join(", ")));
        }
    }

    // ---- the animation callback, counting frames

    static FRAMES: AtomicUsize = AtomicUsize::new(0);

    fn frames() -> usize {
        FRAMES.load(Ordering::SeqCst)
    }

    // ---- the alerts, found and answered from the main queue while the
    // box runs its run loop

    /// The objects of an `NSArray`.
    fn items(array: Id) -> Vec<Id> {
        if array.is_null() {
            return Vec::new();
        }
        // SAFETY: an NSArray's count and objects.
        unsafe {
            let count: usize = objc::send0(array, "count");
            (0..count)
                .map(|i| objc::send1(array, "objectAtIndex:", i))
                .collect()
        }
    }

    fn is_kind_of(obj: Id, cls: &str) -> bool {
        // SAFETY: NSObject's method.
        !obj.is_null() && unsafe { objc::send1(obj, "isKindOfClass:", objc::class(cls)) }
    }

    /// The alert controller a window's root view controller presents, and
    /// that window.
    fn find_alert() -> Option<(Id, Id)> {
        // SAFETY: UIApplication's connected scenes (an NSSet), the window
        // scenes' windows, and UIViewController's properties.
        unsafe {
            let scenes: Id = objc::send0(objc::shared_application(), "connectedScenes");
            for scene in items(objc::send0(scenes, "allObjects")) {
                if !is_kind_of(scene, "UIWindowScene") {
                    continue;
                }
                for w in items(objc::send0(scene, "windows")) {
                    let root: Id = objc::send0(w, "rootViewController");
                    let p: Id = objc::send0(root, "presentedViewController");
                    if is_kind_of(p, "UIAlertController") {
                        return Some((p, w));
                    }
                }
            }
        }
        None
    }

    /// The label (a `UILabel`) showing `text` among a view's subviews.
    fn find_label(v: Id, text: &str) -> Option<Id> {
        if is_kind_of(v, "UILabel") {
            // SAFETY: a UILabel property.
            let label_text = objc::string(unsafe { objc::send0(v, "text") });
            if label_text.as_deref() == Some(text) {
                return Some(v);
            }
        }
        // SAFETY: a UIView property.
        items(unsafe { objc::send0(v, "subviews") })
            .into_iter()
            .find_map(|s| find_label(s, text))
    }

    /// The SDL window's `UIWindow` (its property, a number here).
    fn uiwindow_of(w: &Window) -> Id {
        w.properties()
            .ok()
            .and_then(|p| p.get_number(video::window::PROP_WINDOW_UIKIT_WINDOW_POINTER))
            .map_or(std::ptr::null_mut(), |p| p as usize as Id)
    }

    static SDL_UIWINDOW: AtomicUsize = AtomicUsize::new(0);
    /// The window of the box without a window (retained).
    static ALERT_WINDOW: AtomicUsize = AtomicUsize::new(0);

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct CGRect {
        x: f64,
        y: f64,
        w: f64,
        h: f64,
    }

    /// Choose an action as a tap would: dismiss the alert, and call the
    /// action's handler.
    fn act(label: &str, a: Id, index: usize) {
        let action = items(
            // SAFETY: a UIAlertController property.
            unsafe { objc::send0(a, "actions") },
        )[index];
        // SAFETY: UIAlertAction's properties (its handler, a private one,
        // a block taking the action), and UIViewController's method.
        unsafe {
            t(format!(
                "{label}: answered: action {index} {}",
                quoted_ns(objc::send0(action, "title"))
            ));
            let handler: Id = objc::send1(action, "valueForKey:", objc::ns_string("handler"));
            let () = objc::send2(
                a,
                "dismissViewControllerAnimated:completion:",
                false,
                std::ptr::null_mut::<std::ffi::c_void>(),
            );
            objc::call_block1(handler, action);
        }
    }

    fn answer_later(label: &'static str, index: usize, tap: Option<&'static str>) {
        objc::after(0.2, move || answer_step(label, index, tap));
    }

    fn answer_step(label: &'static str, index: usize, tap: Option<&'static str>) {
        let found = find_alert().filter(|&(a, _)| {
            // SAFETY: UIViewController's properties.
            unsafe {
                let presenting: bool = objc::send0(a, "isBeingPresented");
                let view: Id = objc::send0(a, "view");
                let window: Id = objc::send0(view, "window");
                !presenting && !window.is_null()
            }
        });
        let Some((a, presenter)) = found else {
            objc::after(0.1, move || answer_step(label, index, tap));
            return;
        };
        println!("uikit_messagebox: {:.3} s: {label}: up", elapsed());
        // SAFETY: UIAlertController's, UIAlertAction's, UIViewController's
        // and UIWindow's properties.
        unsafe {
            let style: isize = objc::send0(a, "preferredStyle");
            t(format!(
                "{label}: alert: title={} message={} style={style}",
                quoted_ns(objc::send0(a, "title")),
                quoted_ns(objc::send0(a, "message"))
            ));
            let preferred: Id = objc::send0(a, "preferredAction");
            for (i, action) in items(objc::send0(a, "actions")).into_iter().enumerate() {
                let style: isize = objc::send0(action, "style");
                let enabled: bool = objc::send0(action, "isEnabled");
                t(format!(
                    "{label}: action {i}: {} style={style} enabled={}{}",
                    quoted_ns(objc::send0(action, "title")),
                    yes_no(enabled),
                    if action == preferred {
                        " preferred"
                    } else {
                        ""
                    }
                ));
            }
            if presenter as usize == SDL_UIWINDOW.load(Ordering::SeqCst) {
                t(format!(
                    "{label}: presented on: the window's {}",
                    objc::class_name(objc::send0(a, "presentingViewController"))
                ));
            } else {
                let level: f64 = objc::send0(presenter, "windowLevel");
                let key: bool = objc::send0(presenter, "isKeyWindow");
                let hidden: bool = objc::send0(presenter, "isHidden");
                t(format!(
                    "{label}: presented on: a {} (level {level}, key {}, hidden {}), root {}",
                    objc::class_name(presenter),
                    yes_no(key),
                    yes_no(hidden),
                    objc::class_name(objc::send0(presenter, "rootViewController"))
                ));
                let _: Id = objc::send0(presenter, "retain");
                ALERT_WINDOW.store(presenter as usize, Ordering::SeqCst);
            }
        }
        let frames0 = frames();
        objc::after(0.5, move || {
            t(format!(
                "{label}: animation frames while up: {}",
                frames() - frames0
            ));
            let Some(tap) = tap else {
                act(label, a, index);
                return;
            };
            // SAFETY: UIViewController's and UIView's properties and
            // methods.
            let r: CGRect = unsafe {
                let Some(l) = find_label(objc::send0(a, "view"), tap) else {
                    t(format!("{label}: no label \"{tap}\""));
                    act(label, a, index);
                    return;
                };
                let bounds: CGRect = objc::send0(l, "bounds");
                objc::send2(
                    l,
                    "convertRect:toView:",
                    bounds,
                    std::ptr::null_mut::<std::ffi::c_void>(),
                )
            };
            let req = format!(
                "tap {} {}",
                (r.x + r.w / 2.0).round() as i64,
                (r.y + r.h / 2.0).round() as i64
            );
            println!(
                "uikit_messagebox: {:.3} s: {label}: request {req}",
                elapsed()
            );
            request(&req);
            // (if the tap doesn't close it in a minute, it's answered
            // here: a different answer)
            objc::after(60.0, move || {
                if find_alert().map(|(found, _)| found) == Some(a) {
                    t(format!("{label}: the tap didn't close it"));
                    act(label, a, index);
                }
            });
        });
    }

    fn frames_resumed(label: &str) {
        let f0 = frames();
        events(label, 500);
        t(format!(
            "{label}: animation frames resumed: {}",
            yes_no(frames() > f0)
        ));
    }

    fn show(label: &'static str, data: &MessageBoxData, index: usize) {
        answer_later(label, index, None);
        let r = show_message_box(data);
        t(format!(
            "{label}: result: {} id={}",
            ok(&r),
            r.as_ref().map_or(-1, |&id| id)
        ));
        frames_resumed(label);
    }

    fn button(flags: MessageBoxButtonFlags, button_id: i32, text: &str) -> MessageBoxButtonData {
        MessageBoxButtonData {
            flags,
            button_id,
            text: text.into(),
        }
    }

    fn application_state() -> isize {
        // SAFETY: a UIApplication property.
        unsafe { objc::send0(objc::shared_application(), "applicationState") }
    }

    pub(super) fn main() {
        elapsed();
        let status = sdl3::app::run_app(|_args| {
            let status = checked(main_function);
            exit(status);
        });
        println!("uikit_messagebox: run_app returned {status}, which it never does");
        exit(1);
    }

    fn main_function() {
        println!("uikit_messagebox: {:.3} s: main", elapsed());
        let r = init::init(InitFlags::VIDEO);
        t(format!("init video: {}", ok(&r)));
        r.unwrap();
        t(format!(
            "video driver: {}",
            video::current_video_driver().unwrap()
        ));
        let end = Instant::now() + Duration::from_secs(30);
        while application_state() != 0 && Instant::now() < end {
            events::pump();
            std::thread::sleep(Duration::from_millis(10));
        }
        let r = Window::create("message boxes", 320, 480, WindowFlags::NONE);
        t(format!("create: {}", ok(&r)));
        let w = r.unwrap();
        *WINDOW.lock().unwrap() = w.id();
        SDL_UIWINDOW.store(uiwindow_of(&w) as usize, Ordering::SeqCst);
        events("created", 500);
        t(format!(
            "animation: {}",
            ok(&w.set_ios_animation_callback(
                1,
                Some(Box::new(|| {
                    FRAMES.fetch_add(1, Ordering::SeqCst);
                }))
            ))
        ));
        frames_resumed("animation");

        // The simple one
        answer_later("simple", 0, None);
        t(format!(
            "simple: result: {}",
            ok(&show_simple_message_box(
                MessageBoxFlags::INFORMATION,
                "Simple",
                "A simple message box",
                Some(&w)
            ))
        ));
        frames_resumed("simple");

        // Three buttons, left to right and right to left
        let three = vec![
            button(MessageBoxButtonFlags::RETURNKEY_DEFAULT, 10, "Yes"),
            button(MessageBoxButtonFlags::ESCAPEKEY_DEFAULT, 20, "No"),
            button(MessageBoxButtonFlags::NONE, 30, "Maybe"),
        ];
        let ltr = MessageBoxData {
            flags: MessageBoxFlags::ERROR,
            window: Some(w),
            title: "Left to right".into(),
            message: "Three buttons".into(),
            buttons: three.clone(),
            color_scheme: None,
        };
        show("left to right", &ltr, 2);
        let rtl = MessageBoxData {
            flags: MessageBoxFlags::WARNING | MessageBoxFlags::BUTTONS_RIGHT_TO_LEFT,
            window: Some(w),
            title: "Right to left".into(),
            message: "Three buttons, reversed".into(),
            buttons: three,
            color_scheme: None,
        };
        show("right to left", &rtl, 0);
        show("right to left, the cancel one", &rtl, 1);

        // Without a window: a window of its own
        let nowindow = MessageBoxData {
            flags: MessageBoxFlags::INFORMATION,
            window: None,
            title: "No window".into(),
            message: "On a window of its own".into(),
            buttons: vec![
                button(MessageBoxButtonFlags::NONE, 7, "First"),
                button(MessageBoxButtonFlags::RETURNKEY_DEFAULT, 8, "Second"),
            ],
            color_scheme: None,
        };
        show("no window", &nowindow, 1);
        let alert_window = ALERT_WINDOW.swap(0, Ordering::SeqCst) as Id;
        if !alert_window.is_null() {
            // SAFETY: UIWindow's properties; the window retained in
            // answer_step(), released.
            unsafe {
                let hidden: bool = objc::send0(alert_window, "isHidden");
                let key: bool = objc::send0(alert_window, "isKeyWindow");
                let sdl_key: bool = objc::send0(uiwindow_of(&w), "isKeyWindow");
                t(format!(
                    "no window: its window: hidden {}, key {}; the SDL window: key {}",
                    yes_no(hidden),
                    yes_no(key),
                    yes_no(sdl_key)
                ));
                let () = objc::send0(alert_window, "release");
            }
        }

        // Empty texts
        let empty = MessageBoxData {
            flags: MessageBoxFlags::NONE,
            window: Some(w),
            title: String::new(),
            message: String::new(),
            buttons: vec![button(MessageBoxButtonFlags::NONE, 5, "Only")],
            color_scheme: None,
        };
        show("empty texts", &empty, 0);

        // From another thread: shown on the main thread, as it pumps
        let fromthread = MessageBoxData {
            flags: MessageBoxFlags::INFORMATION,
            window: Some(w),
            title: "Thread".into(),
            message: "From another thread".into(),
            buttons: vec![
                button(MessageBoxButtonFlags::ESCAPEKEY_DEFAULT, 100, "Thread A"),
                button(MessageBoxButtonFlags::NONE, 200, "Thread B"),
            ],
            color_scheme: None,
        };
        answer_later("thread", 1, None);
        let th = std::thread::spawn(move || show_message_box(&fromthread));
        let end = Instant::now() + Duration::from_secs(60);
        while !th.is_finished() && Instant::now() < end {
            events::pump();
            std::thread::sleep(Duration::from_millis(10));
        }
        let r = th.join().unwrap();
        t(format!(
            "thread: result: {} id={}",
            if r.is_ok() { "ok" } else { "error" },
            r.as_ref().map_or(-1, |&id| id)
        ));
        frames_resumed("thread");

        // A real tap, through idb
        let idb = idb();
        match &idb {
            Ok(()) => {
                let tapped = MessageBoxData {
                    flags: MessageBoxFlags::INFORMATION,
                    window: Some(w),
                    title: "Tap".into(),
                    message: "Tapped through idb".into(),
                    buttons: vec![
                        button(MessageBoxButtonFlags::NONE, 1, "Left"),
                        button(MessageBoxButtonFlags::NONE, 2, "Right"),
                    ],
                    color_scheme: None,
                };
                answer_later("idb: tap", 0, Some("Right"));
                let r = show_message_box(&tapped);
                t(format!(
                    "idb: tap: result: {} id={}",
                    ok(&r),
                    r.as_ref().map_or(-1, |&id| id)
                ));
                frames_resumed("idb: tap");
            }
            Err(why) => println!("note: skipping, idb unavailable: {why}"),
        }

        t(format!(
            "animation stopped: {}",
            ok(&w.set_ios_animation_callback(1, None))
        ));
        w.destroy();
        t("destroyed");
        events("destroyed", 500);
        init::quit();
        t("quit");

        let trace = TRACE.lock().unwrap().clone();
        let expected: String = if idb.is_ok() {
            EXPECTED.to_string()
        } else {
            EXPECTED
                .lines()
                .filter(|l| !l.starts_with("idb: "))
                .map(|l| format!("{l}\n"))
                .collect()
        };
        check_trace(&trace, &expected);
    }
}
