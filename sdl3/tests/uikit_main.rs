// Tests of SDL_RunApp() on UIKit: the app delegate, the lifecycle and the
// event pump.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! On UIKit `run_app()` is `UIApplicationMain()`, which calls the main
//! function from SDL's app delegate once the app has launched and never
//! returns, so this is a program of its own (`harness = false`), run as an
//! app on the iOS Simulator by `tools/ios_run.py` (the `__DATA,__sdl3_runner`
//! section below). Its main function checks what the delegate has done by
//! then (the scene and its delegate, the working directory), what the app
//! sees (the form factor, the battery, the UIKit video driver and its
//! display), the application events UIKit's notifications become (posted
//! in-process, and a real trip to the background and back, which the
//! runner makes: `request background`, `request foreground`), the event
//! pump turned off and on, URLs the scene delegate is given and the app
//! opens (its own, as a drop event; an unknown scheme's), then returns:
//! the app keeps running, as upstream's does, with the pump off, and a
//! block on the main queue finishes the checks and ends the process.
//!
//! The expected trace is upstream's C SDL's, from the same program in C
//! run the same way on the same simulator (an iPhone 16 with iOS 18.5).
//! The mice's device events are left out of it. It runs on the tvOS
//! Simulator too (an Apple TV 4K (3rd generation) with tvOS 18.5: the
//! runner's background is the Apple TV's Settings app), without the
//! battery monitoring and the status bar's notification, which tvOS
//! hasn't, and on the visionOS Simulator (an Apple Vision Pro with
//! visionOS 2.5: its background is the Settings app), without the status
//! bar's notification, each against upstream's C there. Elsewhere this
//! program does nothing.

#[cfg(not(any(target_os = "ios", target_os = "tvos", target_os = "visionos")))]
fn main() {}

/// The runner's words: run as an app.
#[cfg(any(target_os = "ios", target_os = "tvos", target_os = "visionos"))]
#[used]
#[link_section = "__DATA,__sdl3_runner"]
static RUNNER: [u8; 4] = *b"app\0";

#[cfg(any(target_os = "ios", target_os = "tvos", target_os = "visionos"))]
mod uikit_support;

#[cfg(any(target_os = "ios", target_os = "tvos", target_os = "visionos"))]
fn main() {
    ios::main()
}

#[cfg(any(target_os = "ios", target_os = "tvos", target_os = "visionos"))]
mod ios {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use sdl3::events::{self, Event, EventType, EventWatch};
    use sdl3::init::{self, InitFlags};
    use sdl3::power::PowerState;
    use sdl3::video;

    use super::uikit_support::objc::{self, Id};
    use super::uikit_support::{check_trace, checked, elapsed, event_name, exit, request};

    /// Upstream's trace (the C program, on an iPhone 16 simulator with
    /// iOS 18.5).
    #[cfg(target_os = "ios")]
    const EXPECTED: &str = "\
main: argc=1
main thread: yes
before run_app: yes
app delegate: SDLUIKitSceneDelegate
scenes: 1 UIWindowScene, delegate SDLUIKitSceneDelegate, the app delegate: no
cwd is the bundle's resource path: yes
form factor: SDL_FORMFACTOR_PHONE
power: state=0 secs=-1 pct=-1
battery monitoring: no
platform: iOS
init video: ok
video driver: uikit
system theme: 1
displays: 1
display: bounds=0,0 393x852 natural=3 current=3
desktop mode: 393x852 fmt=SDL_PIXELFORMAT_ABGR8888 density=3 refresh=60
events: DISPLAY_ORIENTATION(3), KEYBOARD_ADDED
activation: application state 0, (none)
posted will resign active: WILL_ENTER_BACKGROUND
posted did enter background: DID_ENTER_BACKGROUND
posted will enter foreground: WILL_ENTER_FOREGROUND
posted did become active: DID_ENTER_FOREGROUND
posted memory warning: LOW_MEMORY
posted will terminate: TERMINATING
posted status bar orientation: (none)
pump off: main queue ran: no
pump off: posted memory warning: (none)
pump on: main queue ran: yes
pump on: posted memory warning: LOW_MEMORY
handleURL file: DROP_BEGIN, DROP_FILE(/tmp/sdl3 drop.txt), DROP_COMPLETE
handleURL https: DROP_BEGIN, DROP_FILE(https://libsdl.org/?q=1), DROP_COMPLETE
open_url unknown scheme: No handler registered for this type of URL
open_url own scheme: ok
own scheme opened: DROP_BEGIN, DROP_FILE(SCHEME://open-url-test), DROP_COMPLETE
background and back: WILL_ENTER_BACKGROUND, WILL_ENTER_BACKGROUND, DID_ENTER_BACKGROUND, DID_ENTER_BACKGROUND, WILL_ENTER_FOREGROUND, WILL_ENTER_FOREGROUND, DID_ENTER_FOREGROUND, DID_ENTER_FOREGROUND
application state: 0
main returns
after main returned: application state: 0
after main returned: posted memory warning: (none)
quit
";

    /// Upstream's trace on tvOS (the C program, on an Apple TV 4K (3rd
    /// generation) simulator with tvOS 18.5).
    #[cfg(target_os = "tvos")]
    const EXPECTED: &str = "\
main: argc=1
main thread: yes
before run_app: yes
app delegate: SDLUIKitSceneDelegate
scenes: 1 UIWindowScene, delegate SDLUIKitSceneDelegate, the app delegate: no
cwd is the bundle's resource path: yes
form factor: SDL_FORMFACTOR_TV
power: state=2 secs=-1 pct=-1
platform: iOS
init video: ok
video driver: uikit
system theme: 2
displays: 1
display: bounds=0,0 1920x1080 natural=1 current=1
desktop mode: 1920x1080 fmt=SDL_PIXELFORMAT_ABGR8888 density=2 refresh=60
events: KEYBOARD_ADDED
activation: application state 0, (none)
posted will resign active: WILL_ENTER_BACKGROUND
posted did enter background: DID_ENTER_BACKGROUND
posted will enter foreground: WILL_ENTER_FOREGROUND
posted did become active: DID_ENTER_FOREGROUND
posted memory warning: LOW_MEMORY
posted will terminate: TERMINATING
pump off: main queue ran: no
pump off: posted memory warning: (none)
pump on: main queue ran: yes
pump on: posted memory warning: LOW_MEMORY
handleURL file: DROP_BEGIN, DROP_FILE(/tmp/sdl3 drop.txt), DROP_COMPLETE
handleURL https: DROP_BEGIN, DROP_FILE(https://libsdl.org/?q=1), DROP_COMPLETE
open_url unknown scheme: No handler registered for this type of URL
open_url own scheme: ok
own scheme opened: DROP_BEGIN, DROP_FILE(SCHEME://open-url-test), DROP_COMPLETE
background and back: WILL_ENTER_BACKGROUND, WILL_ENTER_BACKGROUND, DID_ENTER_BACKGROUND, DID_ENTER_BACKGROUND, WILL_ENTER_FOREGROUND, WILL_ENTER_FOREGROUND, DID_ENTER_FOREGROUND, DID_ENTER_FOREGROUND
application state: 0
main returns
after main returned: application state: 0
after main returned: posted memory warning: (none)
quit
";

    /// Upstream's trace on visionOS (the C program, on an Apple Vision Pro
    /// simulator with visionOS 2.5).
    #[cfg(target_os = "visionos")]
    const EXPECTED: &str = "\
main: argc=1
main thread: yes
before run_app: yes
app delegate: SDLUIKitSceneDelegate
scenes: 1 UIWindowScene, delegate SDLUIKitSceneDelegate, the app delegate: no
cwd is the bundle's resource path: yes
form factor: SDL_FORMFACTOR_HEADSET
power: state=0 secs=-1 pct=-1
battery monitoring: no
platform: visionOS
init video: ok
video driver: uikit
system theme: 0
displays: 1
display: bounds=0,0 1280x720 natural=1 current=1
desktop mode: 1280x720 fmt=SDL_PIXELFORMAT_ABGR8888 density=2 refresh=90
events: KEYBOARD_ADDED
activation: application state 0, (none)
posted will resign active: WILL_ENTER_BACKGROUND
posted did enter background: DID_ENTER_BACKGROUND
posted will enter foreground: WILL_ENTER_FOREGROUND
posted did become active: DID_ENTER_FOREGROUND
posted memory warning: LOW_MEMORY
posted will terminate: TERMINATING
pump off: main queue ran: no
pump off: posted memory warning: (none)
pump on: main queue ran: yes
pump on: posted memory warning: LOW_MEMORY
handleURL file: DROP_BEGIN, DROP_FILE(/tmp/sdl3 drop.txt), DROP_COMPLETE
handleURL https: DROP_BEGIN, DROP_FILE(https://libsdl.org/?q=1), DROP_COMPLETE
open_url unknown scheme: No handler registered for this type of URL
open_url own scheme: ok
own scheme opened: DROP_BEGIN, DROP_FILE(SCHEME://open-url-test), DROP_COMPLETE
background and back: (none)
application state: 0
main returns
after main returned: application state: 0
after main returned: posted memory warning: (none)
quit
";

    static TRACE: Mutex<Vec<String>> = Mutex::new(Vec::new());
    /// The application events the watch saw (they aren't queued).
    static LIFECYCLE: Mutex<Vec<EventType>> = Mutex::new(Vec::new());
    static WATCH: Mutex<Option<EventWatch>> = Mutex::new(None);
    static BEFORE_RUN_APP: AtomicBool = AtomicBool::new(false);
    static MAIN_RETURNED: AtomicBool = AtomicBool::new(false);

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

    /// An event as the trace names it (the C program's names), `None` for
    /// the mice's device events (see the module's description).
    fn describe(event: &Event, scheme: &str) -> Option<String> {
        let ty = event.event_type();
        let name = event_name(ty);
        Some(match event {
            Event::MouseDevice(_) => return None,
            Event::Display(d) => format!("{name}({})", d.data1),
            Event::Drop(d) if ty == EventType::DROP_FILE => {
                let data = d.data.as_deref().unwrap_or_default();
                match data.strip_prefix(scheme).filter(|_| !scheme.is_empty()) {
                    Some(rest) => format!("{name}(SCHEME{rest})"),
                    None => format!("{name}({data})"),
                }
            }
            _ => name,
        })
    }

    fn list(items: Vec<String>) -> String {
        if items.is_empty() {
            "(none)".into()
        } else {
            items.join(", ")
        }
    }

    /// The application events seen since the last call.
    fn take_lifecycle() -> Vec<String> {
        std::mem::take(&mut *LIFECYCLE.lock().unwrap())
            .iter()
            .map(|&ty| event_name(ty))
            .collect()
    }

    /// Pump, and the queued events.
    fn take_queued(scheme: &str) -> Vec<String> {
        events::pump();
        let mut out = Vec::new();
        while let Some(event) = events::poll() {
            out.extend(describe(&event, scheme));
        }
        out
    }

    /// The application events, then the queued ones, after posting a
    /// notification.
    fn post(name: Id) -> String {
        objc::post_notification(name);
        let mut seen = take_lifecycle();
        seen.extend(take_queued(""));
        list(seen)
    }

    /// Pump events (in a loop with short sleeps) until `done` says so, or
    /// for at most `limit`; the queued events.
    fn pump_until(
        limit: Duration,
        scheme: &str,
        mut done: impl FnMut(&[String]) -> bool,
    ) -> Vec<String> {
        let end = Instant::now() + limit;
        let mut seen = Vec::new();
        while Instant::now() < end {
            seen.extend(take_queued(scheme));
            if done(&seen) {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        seen
    }

    /// The scene delegate of the app's (one) scene.
    fn scene_and_delegate() -> (Id, Id) {
        // SAFETY: UIApplication's connected scenes (an NSSet) and the
        // scene's delegate.
        unsafe {
            let scenes: Id = objc::send0(objc::shared_application(), "connectedScenes");
            let all: Id = objc::send0(scenes, "allObjects");
            let scene: Id = objc::send0(all, "firstObject");
            (scene, objc::send0(scene, "delegate"))
        }
    }

    fn application_state() -> isize {
        // SAFETY: a UIApplication property.
        unsafe { objc::send0(objc::shared_application(), "applicationState") }
    }

    fn power_state_number(state: PowerState) -> i32 {
        match state {
            PowerState::Error => -1,
            PowerState::Unknown => 0,
            PowerState::OnBattery => 1,
            PowerState::NoBattery => 2,
            PowerState::Charging => 3,
            PowerState::Charged => 4,
        }
    }

    pub(super) fn main() {
        elapsed();
        BEFORE_RUN_APP.store(true, Ordering::SeqCst);
        let status = sdl3::app::run_app(|args| {
            let status = checked(|| main_function(args));
            if status != 0 {
                exit(status);
            }
            // Back to the run loop, which goes on (upstream doesn't exit
            // when the main function returns); this ends the process.
            objc::after(0.5, after_main);
            t("main returns");
            MAIN_RETURNED.store(true, Ordering::SeqCst);
            0
        });
        println!("uikit_main: run_app returned {status}, which it never does");
        exit(1);
    }

    fn main_function(args: &[String]) {
        println!("uikit_main: {:.3} s: main", elapsed());
        t(format!("main: argc={}", args.len()));
        // SAFETY: NSThread's class method.
        let main_thread: bool =
            unsafe { objc::send0::<i8>(objc::class("NSThread"), "isMainThread") } != 0;
        t(format!("main thread: {}", yes_no(main_thread)));
        t(format!(
            "before run_app: {}",
            yes_no(BEFORE_RUN_APP.load(Ordering::SeqCst))
        ));
        println!(
            "uikit_main: {:.3} s: application state {}",
            elapsed(),
            application_state()
        );

        // The delegates
        // SAFETY: a UIApplication property.
        let app_delegate: Id = unsafe { objc::send0(objc::shared_application(), "delegate") };
        t(format!("app delegate: {}", objc::class_name(app_delegate)));
        // SAFETY: as in scene_and_delegate().
        let count: usize = unsafe {
            let scenes: Id = objc::send0(objc::shared_application(), "connectedScenes");
            objc::send0(scenes, "count")
        };
        let (scene, scene_delegate) = scene_and_delegate();
        t(format!(
            "scenes: {count} {}, delegate {}, the app delegate: {}",
            objc::class_name(scene),
            objc::class_name(scene_delegate),
            yes_no(scene_delegate == app_delegate)
        ));

        // The working directory
        // SAFETY: NSBundle's main bundle and its resource path.
        let resources = unsafe {
            let bundle: Id = objc::send0(objc::class("NSBundle"), "mainBundle");
            objc::string(objc::send0(bundle, "resourcePath")).unwrap()
        };
        let cwd = std::env::current_dir().unwrap();
        t(format!(
            "cwd is the bundle's resource path: {}",
            yes_no(cwd.canonicalize().ok() == std::path::Path::new(&resources).canonicalize().ok())
        ));

        // The device
        t(format!(
            "form factor: {}",
            init::device_form_factor().name()
        ));
        let power = sdl3::power::power_info();
        t(format!(
            "power: state={} secs={} pct={}",
            power_state_number(power.state),
            power.time_left.map_or(-1, |s| s.as_secs() as i64),
            power.percent.map_or(-1, i32::from)
        ));
        // (SDL turns UIDevice's battery monitoring on for the query; the
        // simulator keeps it off, in an app too; tvOS has none)
        #[cfg(not(target_os = "tvos"))]
        {
            // SAFETY: UIDevice's current device and its property.
            let monitoring: bool = unsafe {
                let device: Id = objc::send0(objc::class("UIDevice"), "currentDevice");
                objc::send0::<i8>(device, "isBatteryMonitoringEnabled") != 0
            };
            t(format!("battery monitoring: {}", yes_no(monitoring)));
        }
        t(format!("platform: {}", init::platform()));

        // Video
        *WATCH.lock().unwrap() = Some(events::add_watch(|event| {
            let ty = event.event_type();
            if (EventType::TERMINATING.0..=EventType::DID_ENTER_FOREGROUND.0).contains(&ty.0) {
                println!("uikit_main: {:.3} s: {}", elapsed(), event_name(ty));
                LIFECYCLE.lock().unwrap().push(ty);
            }
        }));
        let r = init::init(InitFlags::VIDEO);
        t(format!(
            "init video: {}",
            r.as_ref().map_or_else(|e| e.to_string(), |_| "ok".into())
        ));
        r.unwrap();
        t(format!(
            "video driver: {}",
            video::current_video_driver().unwrap()
        ));
        t(format!("system theme: {}", video::system_theme() as i32));
        let displays = video::displays().unwrap();
        t(format!("displays: {}", displays.len()));
        let display = displays[0];
        let b = video::display_bounds(display).unwrap();
        t(format!(
            "display: bounds={},{} {}x{} natural={} current={}",
            b.x,
            b.y,
            b.w,
            b.h,
            video::natural_display_orientation(display) as i32,
            video::current_display_orientation(display) as i32
        ));
        let m = video::desktop_display_mode(display).unwrap();
        t(format!(
            "desktop mode: {}x{} fmt={} density={} refresh={}",
            m.w,
            m.h,
            m.format.name(),
            m.pixel_density,
            m.refresh_rate
        ));
        // (the simulator's keyboard is GameController's coalesced keyboard:
        // added when video starts if GameController has found it by then,
        // else when its connect notification comes, a moment later)
        let queued = pump_until(Duration::from_secs(5), "", |seen| {
            seen.iter().any(|event| event == "KEYBOARD_ADDED")
        });
        t(format!("events: {}", list(queued)));

        // The app becomes active around when the main function starts
        // (before it, or when it pumps events), with the application
        // events of the scene's activation and UIKit's notification, which
        // the trace leaves out (they may come before SDL is up)
        let end = Instant::now() + Duration::from_secs(30);
        let mut seen = Vec::new();
        while application_state() != 0 && Instant::now() < end {
            seen.extend(take_queued(""));
            std::thread::sleep(Duration::from_millis(10));
        }
        println!(
            "uikit_main: {:.3} s: application state {}",
            elapsed(),
            application_state()
        );
        // (the rest of the transition)
        for _ in 0..10 {
            seen.extend(take_queued(""));
            std::thread::sleep(Duration::from_millis(10));
        }
        take_lifecycle();
        t(format!(
            "activation: application state {}, {}",
            application_state(),
            list(seen)
        ));

        // UIKit's notifications, posted in-process
        // SAFETY: UIKit's notification names.
        let notifications = unsafe {
            [
                (
                    "will resign active",
                    objc::UIApplicationWillResignActiveNotification,
                ),
                (
                    "did enter background",
                    objc::UIApplicationDidEnterBackgroundNotification,
                ),
                (
                    "will enter foreground",
                    objc::UIApplicationWillEnterForegroundNotification,
                ),
                (
                    "did become active",
                    objc::UIApplicationDidBecomeActiveNotification,
                ),
                (
                    "memory warning",
                    objc::UIApplicationDidReceiveMemoryWarningNotification,
                ),
                (
                    "will terminate",
                    objc::UIApplicationWillTerminateNotification,
                ),
                // (tvOS and visionOS have no status bar)
                #[cfg(not(any(target_os = "tvos", target_os = "visionos")))]
                (
                    "status bar orientation",
                    objc::UIApplicationDidChangeStatusBarOrientationNotification,
                ),
            ]
        };
        for (label, name) in notifications {
            t(format!("posted {label}: {}", post(name)));
        }
        // SAFETY: as above.
        let memory_warning = unsafe { objc::UIApplicationDidReceiveMemoryWarningNotification };

        // The event pump, off (the main run loop doesn't run, and the
        // notifications aren't observed) and on
        let ran = Arc::new(AtomicBool::new(false));
        video::set_ios_event_pump(false);
        let r = ran.clone();
        objc::on_main_queue(move || r.store(true, Ordering::SeqCst));
        events::pump();
        t(format!(
            "pump off: main queue ran: {}",
            yes_no(ran.load(Ordering::SeqCst))
        ));
        t(format!(
            "pump off: posted memory warning: {}",
            post(memory_warning)
        ));
        video::set_ios_event_pump(true);
        events::pump();
        t(format!(
            "pump on: main queue ran: {}",
            yes_no(ran.load(Ordering::SeqCst))
        ));
        t(format!(
            "pump on: posted memory warning: {}",
            post(memory_warning)
        ));

        // URLs the scene delegate handles: drop events
        // SAFETY: NSURL's class methods, and the scene delegate's method
        // taking one.
        unsafe {
            let file: Id = objc::send1(
                objc::class("NSURL"),
                "fileURLWithPath:",
                objc::ns_string("/tmp/sdl3 drop.txt"),
            );
            let () = objc::send1(scene_delegate, "handleURL:", file);
            t(format!("handleURL file: {}", list(take_queued(""))));
            let https: Id = objc::send1(
                objc::class("NSURL"),
                "URLWithString:",
                objc::ns_string("https://libsdl.org/?q=1"),
            );
            let () = objc::send1(scene_delegate, "handleURL:", https);
            t(format!("handleURL https: {}", list(take_queued(""))));
        }

        // Opening URLs: an unknown scheme's, and the app's own (the runner
        // registers its bundle identifier as a URL scheme), which UIKit
        // gives back to the scene delegate
        t(format!(
            "open_url unknown scheme: {}",
            sdl3::misc::open_url("sdl-no-such-scheme-for-tests://x")
                .map_or_else(|e| e.to_string(), |_| "ok".into())
        ));
        // SAFETY: NSBundle's main bundle and its identifier.
        let scheme = unsafe {
            let bundle: Id = objc::send0(objc::class("NSBundle"), "mainBundle");
            objc::string(objc::send0(bundle, "bundleIdentifier")).unwrap()
        };
        t(format!(
            "open_url own scheme: {}",
            sdl3::misc::open_url(&format!("{scheme}://open-url-test"))
                .map_or_else(|e| e.to_string(), |_| "ok".into())
        ));
        println!("uikit_main: {:.3} s: opened", elapsed());
        let seen = pump_until(Duration::from_secs(10), &scheme, |seen| {
            seen.iter().any(|e| e == "DROP_COMPLETE")
        });
        println!("uikit_main: {:.3} s: drop events", elapsed());
        t(format!("own scheme opened: {}", list(seen)));
        // (anything the trip may have caused)
        take_lifecycle();

        // To the background and back (the runner brings another app to
        // the foreground, then this one back)
        request("background");
        let mut lifecycle = Vec::new();
        let saw = |lifecycle: &mut Vec<String>, what: &str, limit: Duration| {
            let end = Instant::now() + limit;
            let mut report = Instant::now();
            while Instant::now() < end {
                if report.elapsed() >= Duration::from_secs(1) {
                    report = Instant::now();
                    println!(
                        "uikit_main: {:.3} s: waiting for {what}, application state {}",
                        elapsed(),
                        application_state()
                    );
                }
                take_queued("");
                lifecycle.extend(take_lifecycle());
                if lifecycle.iter().any(|e| e == what) {
                    return true;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            false
        };
        let backgrounded = saw(
            &mut lifecycle,
            "DID_ENTER_BACKGROUND",
            Duration::from_secs(60),
        );
        request("foreground");
        let foregrounded = backgrounded
            && saw(
                &mut lifecycle,
                "DID_ENTER_FOREGROUND",
                Duration::from_secs(60),
            );
        if foregrounded {
            // (the rest of the transition)
            saw(&mut lifecycle, "-", Duration::from_secs(1));
        }
        t(format!("background and back: {}", list(lifecycle)));
        t(format!("application state: {}", application_state()));
    }

    /// From the main queue, once the main function has returned to
    /// UIKit's run loop.
    fn after_main() {
        let status = checked(|| {
            assert!(MAIN_RETURNED.load(Ordering::SeqCst));
            t(format!(
                "after main returned: application state: {}",
                application_state()
            ));
            // (the event pump is off since the main function returned, so
            // the lifecycle notifications aren't observed)
            // SAFETY: UIKit's notification name.
            let memory_warning = unsafe { objc::UIApplicationDidReceiveMemoryWarningNotification };
            objc::post_notification(memory_warning);
            t(format!(
                "after main returned: posted memory warning: {}",
                list(take_lifecycle())
            ));

            drop(WATCH.lock().unwrap().take());
            init::quit();
            t("quit");

            check_trace(&TRACE.lock().unwrap(), EXPECTED);
        });
        exit(status);
    }
}
