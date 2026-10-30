// Tests of an Android app's main and lifecycle: SDL_RunApp() from
// SDLActivity's SDLThread, the lifecycle events, the event pump, the log.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! An app (as android_runner.rs is), run by `tools/android_run.py`, whose
//! main function, which upstream's `SDLActivity` runs on its "SDLThread"
//! thread through `nativeRunMain()`, checks what such a program sees (its
//! arguments, its thread, which isn't SDL's main thread until SDL is
//! initialized, the device: its API level and form factor, the JNI
//! environment and the activity), the logcat output of `SDL_Log` at each
//! priority (the runner reads the device's log back: `request logcat`), the
//! application events the activity's lifecycle becomes: the lifecycle
//! natives called in-process (low memory, a pause and a resume that cancel
//! each other, the locale and the dark mode), a pause with
//! `SDL_HINT_ANDROID_BLOCK_ON_PAUSE` on (the event pump blocks until the
//! resume, which a thread of SDL's sends) and off (it doesn't), and the
//! system's: memory trimmed (`am send-trim-memory`), trips to the
//! background (the HOME key, then the activity launched again, asked for
//! from the event watch, as the pump blocks) with the hint on and off
//! (whether the pump that saw the app go returned only once it was back:
//! it does with the hint, it doesn't without, as the app resumes once its
//! surface is ready and has the focus), the system's dark mode (`cmd
//! uimode night`), the app's locale (`cmd locale set-app-locales`),
//! `SDL_SendAndroidMessage()`, and the back button: `onBackPressed()` with
//! the trap's hint, the BACK key with the trap's callback (which the
//! android video driver registers when the hint changes), and
//! `SDL_SendAndroidBackButton()`, the system's back, which finishes the
//! activity while main runs (`SDL_EVENT_TERMINATING`). Video is the
//! android driver's, without a window.
//!
//! The expected trace is upstream's C SDL's, from the same program in C
//! (built with upstream's android-project, Gradle and CMake, run by the
//! runner's `--run-apk`) on the same emulator (API 35, Google APIs,
//! x86-64). Elsewhere this program does nothing.

#[cfg(not(target_os = "android"))]
fn main() {}

/// The runner's words: run as an app.
#[cfg(target_os = "android")]
#[used]
#[link_section = ".sdl3_runner"]
static RUNNER: [u8; 4] = *b"app\0";

#[cfg(target_os = "android")]
mod android_support;

#[cfg(target_os = "android")]
fn main() -> std::process::ExitCode {
    android_support::start();
    let status = android_support::checked(app::main);
    android_support::finish(status)
}

#[cfg(target_os = "android")]
mod app {
    use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    use sdl3::events::{self, EventType, EventWatch};
    use sdl3::init::{self, InitFlags};
    use sdl3::log::{self, Category, Priority};
    use sdl3::{android, hints, video};

    use super::android_support::jni::{self, Jobject, Jvalue};
    use super::android_support::{os_thread_name, request, request_without_waiting};

    /// Upstream's trace (the C program, on the emulator: API 35, Google
    /// APIs, x86-64).
    const EXPECTED: &str = "\
main: argc=1 argv0=app_process
thread: SDLThread
main thread before init: no
sdk: 35
tv: no, chromebook: no, dex: no, tablet: no
form factor: SDL_FORMFACTOR_PHONE
jni env: yes
activity: org.libsdl.app.SDLActivity
init: ok
main thread after init: yes
video driver: android
system theme: 1
events: (none)
logcat: V/SDL/TEST: trace
logcat: V/SDL/TEST: verbose
logcat: D/SDL/TEST: debug
logcat: I/SDL/TEST: info
logcat: W/SDL/TEST: warn
logcat: E/SDL/TEST: error
logcat: F/SDL/TEST: critical
logcat: E/SDL/RESERVED: reserved
logcat: E/SDL/CUSTOM: custom
in-process low memory: LOW_MEMORY
in-process low memory twice: LOW_MEMORY
in-process pause then resume: (none)
in-process locale changed: LOCALE_CHANGED
in-process dark mode: SYSTEM_THEME_CHANGED, theme 2
in-process dark mode again: (none), theme 2
in-process light mode: SYSTEM_THEME_CHANGED, theme 1
block on pause: (unset)
in-process pause, blocking: WILL_ENTER_BACKGROUND, DID_ENTER_BACKGROUND, WILL_ENTER_FOREGROUND, DID_ENTER_FOREGROUND, the pump returned after the resume: yes
an SDL thread is attached: yes
send message 5: Parameter 'command' is invalid
send message 0x8000: ok
logcat: E/SDL: error handling message, command is 32768
trim memory: LOW_MEMORY
in-process pause, not blocking: WILL_ENTER_BACKGROUND, DID_ENTER_BACKGROUND, the pump returned after the resume: no
then: WILL_ENTER_FOREGROUND, DID_ENTER_FOREGROUND
background and back, not blocking: WILL_ENTER_BACKGROUND, DID_ENTER_BACKGROUND, WILL_ENTER_FOREGROUND, DID_ENTER_FOREGROUND; back in the same pump: no
background and back, blocking: WILL_ENTER_BACKGROUND, DID_ENTER_BACKGROUND, WILL_ENTER_FOREGROUND, DID_ENTER_FOREGROUND; back in the same pump: yes
night yes: SYSTEM_THEME_CHANGED, theme 2
night no: SYSTEM_THEME_CHANGED, theme 1
locale fr-FR: LOCALE_CHANGED
locale reset: LOCALE_CHANGED
onBackPressed with the trap's hint: (none)
back with the trap: (none)
SDL_SendAndroidBackButton: QUIT, TERMINATING
finish: (finished)
main returns
";

    static TRACE: Mutex<Vec<String>> = Mutex::new(Vec::new());
    /// The events the watch saw (the application's).
    static SEEN: Mutex<Vec<EventType>> = Mutex::new(Vec::new());
    static WATCH: Mutex<Option<EventWatch>> = Mutex::new(None);
    static PUMPS: AtomicI32 = AtomicI32::new(0);
    static BACKGROUND_PUMP: AtomicI32 = AtomicI32::new(-1);
    static FOREGROUND_PUMP: AtomicI32 = AtomicI32::new(-1);
    static FOREGROUND_ON_BACKGROUND: AtomicBool = AtomicBool::new(false);
    static THREAD_ATTACHED: AtomicBool = AtomicBool::new(false);

    /// The activity's class (a global reference), for the threads.
    #[derive(Clone, Copy)]
    struct Class(Jobject);
    // SAFETY: a global reference, which any thread may use.
    unsafe impl Send for Class {}
    unsafe impl Sync for Class {}
    static ACTIVITY_CLASS: Mutex<Option<Class>> = Mutex::new(None);

    fn t(line: impl Into<String>) {
        let line = line.into();
        println!("trace: {line}");
        TRACE.lock().unwrap().push(line);
    }

    fn yes(b: bool) -> &'static str {
        if b {
            "yes"
        } else {
            "no"
        }
    }

    /// The C program's names of the events the watch keeps.
    fn event_name(ty: EventType) -> Option<&'static str> {
        Some(match ty {
            EventType::QUIT => "QUIT",
            EventType::TERMINATING => "TERMINATING",
            EventType::LOW_MEMORY => "LOW_MEMORY",
            EventType::WILL_ENTER_BACKGROUND => "WILL_ENTER_BACKGROUND",
            EventType::DID_ENTER_BACKGROUND => "DID_ENTER_BACKGROUND",
            EventType::WILL_ENTER_FOREGROUND => "WILL_ENTER_FOREGROUND",
            EventType::DID_ENTER_FOREGROUND => "DID_ENTER_FOREGROUND",
            EventType::LOCALE_CHANGED => "LOCALE_CHANGED",
            EventType::SYSTEM_THEME_CHANGED => "SYSTEM_THEME_CHANGED",
            _ => return None,
        })
    }

    fn add_watch() {
        *WATCH.lock().unwrap() = Some(events::add_watch(|event| {
            let ty = event.event_type();
            if event_name(ty).is_none() {
                return;
            }
            SEEN.lock().unwrap().push(ty);
            let pumps = PUMPS.load(Ordering::SeqCst);
            if ty == EventType::DID_ENTER_BACKGROUND {
                BACKGROUND_PUMP.store(pumps, Ordering::SeqCst);
            }
            if ty == EventType::DID_ENTER_FOREGROUND {
                FOREGROUND_PUMP.store(pumps, Ordering::SeqCst);
            }
            // (from the watch, before the pump blocks while in the
            // background; not waited for, as the C's)
            if ty == EventType::DID_ENTER_BACKGROUND
                && FOREGROUND_ON_BACKGROUND.load(Ordering::SeqCst)
            {
                request_without_waiting("foreground");
            }
        }));
    }

    /// The events the watch saw since the last call, but those of `skip`.
    fn list(skip: Option<EventType>) -> String {
        let seen = std::mem::take(&mut *SEEN.lock().unwrap());
        let names: Vec<&str> = seen
            .into_iter()
            .filter(|&ty| Some(ty) != skip)
            .filter_map(event_name)
            .collect();
        if names.is_empty() {
            "(none)".to_string()
        } else {
            names.join(", ")
        }
    }

    fn saw(ty: EventType) -> bool {
        SEEN.lock().unwrap().contains(&ty)
    }

    fn pump() {
        PUMPS.fetch_add(1, Ordering::SeqCst);
        events::pump();
        events::flush_events(EventType::FIRST, EventType::LAST);
    }

    /// Pump until the watch sees `ty` (`None`: none), for `ms` at most, then
    /// 200 ms more for the rest.
    fn pump_until(ty: Option<EventType>, ms: u64) {
        let end = Instant::now() + Duration::from_millis(ms);
        while Instant::now() < end {
            pump();
            if ty.is_some_and(saw) {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let end = Instant::now() + Duration::from_millis(200);
        while ty.is_some() && Instant::now() < end {
            pump();
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn env() -> jni::JNIEnv {
        android::jni_env().expect("a JNI environment").as_ptr()
    }

    fn activity_class() -> Jobject {
        ACTIVITY_CLASS
            .lock()
            .unwrap()
            .expect("the activity's class")
            .0
    }

    /// One of the activity's (static) natives, called in-process, as the
    /// C program calls them (with a boolean argument when `sig` has one).
    fn call_native(name: &str, sig: &str, z: bool) {
        jni::call_static_void(env(), activity_class(), name, sig, &[Jvalue { z: z as u8 }]);
    }

    /// `nativePause()`, then `nativeResume()` from a thread of SDL's
    /// 500 ms later, and a pump meanwhile: its events, and whether it
    /// returned after the resume.
    fn pause_and_resume_later(label: &str, vm: jni::JavaVm) {
        call_native("nativePause", "()V", false);
        let class = Class(activity_class());
        let thread = sdl3::thread::Thread::builder()
            .name("resume")
            .spawn(move || {
                THREAD_ATTACHED.store(jni::is_attached(vm), Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(500));
                let env = android::jni_env().expect("a JNI environment").as_ptr();
                let class = class;
                jni::call_static_void(env, class.0, "nativeResume", "()V", &[]);
                0
            })
            .expect("a thread");
        let start = Instant::now();
        pump();
        t(format!(
            "{label}: {}, the pump returned after the resume: {}",
            list(None),
            yes(start.elapsed() >= Duration::from_millis(400))
        ));
        thread.wait();
    }

    fn init_video() {
        hints::set(hints::VIDEO_DRIVER, "android").unwrap();
        if let Err(e) = init::init(InitFlags::VIDEO) {
            t(format!("init: {e}"));
        }
        add_watch();
        pump_until(None, 100);
        list(None);
    }

    fn quit_video() {
        drop(WATCH.lock().unwrap().take());
        init::quit();
    }

    /// A trip to the background (asked for with `what`) and back, and
    /// whether the pump that saw the activity go returned only once it was
    /// back (not blocking, the resume comes once the activity's surface is
    /// ready and has the focus, long after the pump's 100 ms).
    fn trip(label: &str, what: &str) {
        FOREGROUND_ON_BACKGROUND.store(true, Ordering::SeqCst);
        BACKGROUND_PUMP.store(-1, Ordering::SeqCst);
        FOREGROUND_PUMP.store(-1, Ordering::SeqCst);
        request(what);
        pump_until(Some(EventType::DID_ENTER_FOREGROUND), 30_000);
        FOREGROUND_ON_BACKGROUND.store(false, Ordering::SeqCst);
        let background = BACKGROUND_PUMP.load(Ordering::SeqCst);
        let foreground = FOREGROUND_PUMP.load(Ordering::SeqCst);
        let events = list(Some(EventType::LOW_MEMORY));
        t(format!(
            "{label}: {events}; back in the same pump: {}",
            yes(background >= 0 && background == foreground)
        ));
    }

    /// The events from `QUIT` on (a pause may come before the activity is
    /// destroyed, or not: it stops, then is destroyed, the pause dropped
    /// unless a pump came between).
    fn from_quit(events: &str) -> &str {
        events.find("QUIT").map_or(events, |i| &events[i..])
    }

    /// The runner's answer to `logcat TAGS`, in the trace.
    fn logcat(tags: &str) {
        for line in request(&format!("logcat {tags}")) {
            t(format!("logcat: {line}"));
        }
    }

    fn theme() -> i32 {
        video::system_theme() as i32
    }

    pub(super) fn main() {
        let args: Vec<String> = std::env::args().collect();
        t(format!(
            "main: argc={} argv0={}",
            args.len(),
            args.first().map_or("", String::as_str)
        ));
        t(format!("thread: {}", os_thread_name()));
        t(format!(
            "main thread before init: {}",
            yes(init::is_main_thread())
        ));
        t(format!("sdk: {}", android::sdk_version()));
        t(format!(
            "tv: {}, chromebook: {}, dex: {}, tablet: {}",
            yes(init::is_tv()),
            yes(android::is_chromebook()),
            yes(android::is_dex_mode()),
            yes(init::is_tablet())
        ));
        t(format!(
            "form factor: {}",
            init::device_form_factor().name()
        ));

        let env = android::jni_env();
        t(format!("jni env: {}", yes(env.is_some())));
        let env = env.unwrap().as_ptr();
        let vm = jni::java_vm(env);
        let activity = android::activity().expect("the activity").as_ptr();
        t(format!("activity: {}", jni::class_name(env, activity)));
        jni::delete_local_ref(env, activity);
        let class = jni::find_class(env, "org/libsdl/app/SDLActivity");
        *ACTIVITY_CLASS.lock().unwrap() = Some(Class(jni::new_global_ref(env, class)));
        jni::delete_local_ref(env, class);

        // Video (the android driver), and the events
        hints::set(hints::VIDEO_DRIVER, "android").unwrap();
        t(format!(
            "init: {}",
            init::init(InitFlags::VIDEO).map_or_else(|e| e.to_string(), |()| "ok".to_string())
        ));
        add_watch();
        t(format!(
            "main thread after init: {}",
            yes(init::is_main_thread())
        ));
        t(format!(
            "video driver: {}",
            video::current_video_driver().unwrap_or("(none)")
        ));
        t(format!("system theme: {}", theme()));
        pump_until(None, 100);
        t(format!("events: {}", list(None)));

        // Android's log
        log::set_priority(Category::Test, Priority::Trace);
        log::log(Category::Test, Priority::Trace, "trace");
        log::log(Category::Test, Priority::Verbose, "verbose");
        log::log(Category::Test, Priority::Debug, "debug");
        log::log(Category::Test, Priority::Info, "info");
        log::log(Category::Test, Priority::Warn, "warn");
        log::log(Category::Test, Priority::Error, "error");
        log::log(Category::Test, Priority::Critical, "critical");
        log::log(Category::Reserved(10), Priority::Error, "reserved");
        log::log(
            Category::Custom(Category::CUSTOM_BASE),
            Priority::Error,
            "custom",
        );
        logcat("SDL/TEST SDL/RESERVED SDL/CUSTOM");

        // The lifecycle's natives, called in-process
        call_native("nativeLowMemory", "()V", false);
        pump_until(None, 100);
        t(format!("in-process low memory: {}", list(None)));
        call_native("nativeLowMemory", "()V", false);
        call_native("nativeLowMemory", "()V", false);
        pump_until(None, 100);
        t(format!("in-process low memory twice: {}", list(None)));
        call_native("nativePause", "()V", false);
        call_native("nativeResume", "()V", false);
        pump_until(None, 100);
        t(format!("in-process pause then resume: {}", list(None)));
        call_native("onNativeLocaleChanged", "()V", false);
        pump_until(None, 100);
        t(format!("in-process locale changed: {}", list(None)));
        call_native("onNativeDarkModeChanged", "(Z)V", true);
        pump_until(None, 100);
        t(format!(
            "in-process dark mode: {}, theme {}",
            list(None),
            theme()
        ));
        call_native("onNativeDarkModeChanged", "(Z)V", true);
        pump_until(None, 100);
        t(format!(
            "in-process dark mode again: {}, theme {}",
            list(None),
            theme()
        ));
        call_native("onNativeDarkModeChanged", "(Z)V", false);
        pump_until(None, 100);
        t(format!(
            "in-process light mode: {}, theme {}",
            list(None),
            theme()
        ));

        // Pausing, with SDL_HINT_ANDROID_BLOCK_ON_PAUSE unset (blocking)
        t(format!(
            "block on pause: {}",
            hints::get(hints::ANDROID_BLOCK_ON_PAUSE).unwrap_or_else(|| "(unset)".to_string())
        ));
        pause_and_resume_later("in-process pause, blocking", vm);
        t(format!(
            "an SDL thread is attached: {}",
            yes(THREAD_ATTACHED.load(Ordering::SeqCst))
        ));

        // The command handler
        let sent = |command, param| {
            android::send_message(command, param)
                .map_or_else(|e| e.to_string(), |()| "ok".to_string())
        };
        t(format!("send message 5: {}", sent(5, 1)));
        t(format!("send message 0x8000: {}", sent(0x8000, 7)));
        std::thread::sleep(Duration::from_millis(500));
        logcat("SDL:E");

        // Low memory, from the system
        request("trim-memory RUNNING_LOW");
        pump_until(Some(EventType::LOW_MEMORY), 10_000);
        t(format!("trim memory: {}", list(None)));

        // With SDL_HINT_ANDROID_BLOCK_ON_PAUSE 0
        quit_video();
        hints::set(hints::ANDROID_BLOCK_ON_PAUSE, "0").unwrap();
        init_video();
        pause_and_resume_later("in-process pause, not blocking", vm);
        pump_until(Some(EventType::DID_ENTER_FOREGROUND), 10_000);
        t(format!("then: {}", list(None)));
        trip("background and back, not blocking", "background");

        // Blocking again
        quit_video();
        init_video();
        trip("background and back, blocking", "background");

        // The system's dark mode
        request("night yes");
        pump_until(Some(EventType::SYSTEM_THEME_CHANGED), 10_000);
        t(format!("night yes: {}, theme {}", list(None), theme()));
        request("night no");
        pump_until(Some(EventType::SYSTEM_THEME_CHANGED), 10_000);
        t(format!("night no: {}, theme {}", list(None), theme()));

        // The app's locale
        request("locale fr-FR");
        pump_until(Some(EventType::LOCALE_CHANGED), 10_000);
        t(format!("locale fr-FR: {}", list(None)));
        request("locale");
        pump_until(Some(EventType::LOCALE_CHANGED), 10_000);
        t(format!("locale reset: {}", list(None)));

        // The back button, with the trap's hint (which the android video
        // driver passes on: SDLActivity.setBackButtonTrapEnabled(true)):
        // onBackPressed() (called here: from API 33, with
        // enableOnBackInvokedCallback, the BACK key doesn't call it), then
        // the BACK key, which the trap's callback takes
        hints::set(hints::ANDROID_TRAP_BACK_BUTTON, "1").unwrap();
        let activity = android::activity().expect("the activity").as_ptr();
        jni::call_void(env, activity, "onBackPressed", "()V", &[]);
        jni::delete_local_ref(env, activity);
        pump_until(None, 1000);
        t(format!(
            "onBackPressed with the trap's hint: {}",
            list(None)
        ));
        request("back");
        pump_until(None, 3000);
        t(format!("back with the trap: {}", list(None)));
        hints::reset(hints::ANDROID_TRAP_BACK_BUTTON);

        // SDL_SendAndroidBackButton(): the system's back (the task moves to
        // the back, or the activity finishes), with the pause not blocking
        // (whichever it is, the pump returns)
        quit_video();
        hints::set(hints::ANDROID_BLOCK_ON_PAUSE, "0").unwrap();
        init_video();
        android::send_back_button();
        pump_until(Some(EventType::TERMINATING), 5000);
        let terminated = saw(EventType::TERMINATING);
        t(format!(
            "SDL_SendAndroidBackButton: {}",
            from_quit(&list(None))
        ));

        // The activity finishes
        if terminated {
            t("finish: (finished)");
        } else {
            request("foreground");
            pump_until(Some(EventType::DID_ENTER_FOREGROUND), 10_000);
            t(format!("foreground: {}", list(None)));
            let activity = android::activity().expect("the activity").as_ptr();
            jni::call_void(env, activity, "finish", "()V", &[]);
            jni::delete_local_ref(env, activity);
            pump_until(Some(EventType::TERMINATING), 10_000);
            t(format!("finish: {}", from_quit(&list(None))));
        }

        t("main returns");
        let trace = TRACE.lock().unwrap().clone();
        check_trace(&trace, EXPECTED);
    }

    /// Compare the trace with upstream's, line by line.
    fn check_trace(trace: &[String], expected: &str) {
        let expected: Vec<&str> = expected.lines().collect();
        println!("--- trace ({} lines)", trace.len());
        for line in trace {
            println!("  {line}");
        }
        let first_difference = (0..trace.len().max(expected.len()))
            .find(|&i| trace.get(i).map(String::as_str) != expected.get(i).copied());
        if let Some(i) = first_difference {
            panic!(
                "the trace differs from upstream's at line {}: {:?}, expected {:?}",
                i + 1,
                trace.get(i),
                expected.get(i)
            );
        }
        println!("--- the trace matches upstream's");
    }
}
