// Tests of the android video driver's clipboard: the system's primary
// clip, set, read and tested through SDL, and changed from outside.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! An app (`harness = false`), run by `tools/android_run.py`, whose main
//! function makes a window (Android lets only the focused app read the
//! clipboard) and traces the clipboard:
//!
//! * before `init()` (the clipboard needs the video subsystem);
//! * text set through SDL (ASCII, other characters, control characters,
//!   empty, which clears the system's clip), each with its
//!   `CLIPBOARD_UPDATE` events, `has_clipboard_text()`,
//!   `clipboard_text()`, `clipboard_mime_types()`, and the system's clip
//!   as the `ClipboardManager` has it (its label, MIME types and text);
//! * the app's own data in another MIME type (`set_clipboard_data()`),
//!   alone (the system's clip is cleared: upstream gives it text only)
//!   and with text, and `clipboard_data()` and `has_clipboard_data()` of
//!   each type;
//! * changes made to the system's clip other than through SDL: plain
//!   text, HTML with its text, a clear, other characters (stand-ins for
//!   another app's: this program's own calls of the `ClipboardManager`,
//!   on a thread of its own, as Android lets only the focused app and the
//!   input method use the clipboard), each sent by the activity's primary
//!   clip listener as an update the app doesn't own (no MIME types; the
//!   app's own data is forgotten);
//! * SDL's text again, and the data cleared.
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
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use sdl3::events::window::WindowFlags;
    use sdl3::events::{self, Event, EventType, EventWatch};
    use sdl3::init::{self, InitFlags};
    use sdl3::video::clipboard;
    use sdl3::video::{self, Window};

    use super::android_support::check_trace;
    use super::android_support::events::pump_for;
    use super::android_support::jni::{self, JNIEnv, Jobject, Jvalue};

    /// Upstream's trace (the C program, on the emulator: API 35, Google
    /// APIs, x86-64).
    const EXPECTED: &str = "\
before init: set: error: Video subsystem has not been initialized
before init: get: \"\"; has: no
init: ok
video driver: android
window: ok
start: events: (none)
emptied: set \"\": ok
emptied: events: CLIPBOARD_UPDATE(owner=1 mime_types=[])
emptied: has text=no text=\"\"
emptied: mime types=[]
emptied: system: has primary clip=no clip=null
ascii: set \"Hello, clipboard!\": ok
ascii: events: CLIPBOARD_UPDATE(owner=1 mime_types=[text/plain;charset=utf-8]), CLIPBOARD_UPDATE(owner=0 mime_types=[])
ascii: has text=yes text=\"Hello, clipboard!\"
ascii: mime types=[]
ascii: system: has primary clip=yes label=null mime types=[text/plain] items=1 text=\"Hello, clipboard!\"
unicode: set \"Grüße, 世界 🎮\": ok
unicode: events: CLIPBOARD_UPDATE(owner=1 mime_types=[text/plain;charset=utf-8]), CLIPBOARD_UPDATE(owner=0 mime_types=[])
unicode: has text=yes text=\"Grüße, 世界 🎮\"
unicode: mime types=[]
unicode: system: has primary clip=yes label=null mime types=[text/plain] items=1 text=\"Grüße, 世界 🎮\"
again: set \"Hello, clipboard!\": ok
again: events: CLIPBOARD_UPDATE(owner=1 mime_types=[text/plain;charset=utf-8]), CLIPBOARD_UPDATE(owner=0 mime_types=[])
again: has text=yes text=\"Hello, clipboard!\"
again: mime types=[]
again: system: has primary clip=yes label=null mime types=[text/plain] items=1 text=\"Hello, clipboard!\"
control: set \"tab\\x09here\\x0aline \\x22quoted\\x22 back\\x5cslash\": ok
control: events: CLIPBOARD_UPDATE(owner=1 mime_types=[text/plain;charset=utf-8]), CLIPBOARD_UPDATE(owner=0 mime_types=[])
control: has text=yes text=\"tab\\x09here\\x0aline \\x22quoted\\x22 back\\x5cslash\"
control: mime types=[]
control: system: has primary clip=yes label=null mime types=[text/plain] items=1 text=\"tab\\x09here\\x0aline \\x22quoted\\x22 back\\x5cslash\"
empty: set \"\": ok
empty: events: CLIPBOARD_UPDATE(owner=1 mime_types=[])
empty: has text=no text=\"\"
empty: mime types=[]
empty: system: has primary clip=no clip=null
empty again: set \"\": ok
empty again: events: CLIPBOARD_UPDATE(owner=1 mime_types=[])
empty again: has text=no text=\"\"
empty again: mime types=[]
empty again: system: has primary clip=no clip=null
custom: set data: ok
custom: events: CLIPBOARD_UPDATE(owner=1 mime_types=[application/x-sdl3-test])
custom: has text=no text=\"\"
custom: mime types=[application/x-sdl3-test]
custom: system: has primary clip=no clip=null
custom: data application/x-sdl3-test: has=yes \"custom data\" (11 bytes)
custom: data text/plain: has=no nil
custom and text: set data: ok
custom and text: events: CLIPBOARD_UPDATE(owner=1 mime_types=[application/x-sdl3-test,text/plain;charset=utf-8]), CLIPBOARD_UPDATE(owner=0 mime_types=[])
custom and text: has text=yes text=\"as text!!\"
custom and text: mime types=[]
custom and text: system: has primary clip=yes label=null mime types=[text/plain] items=1 text=\"as text!!\"
custom and text: data application/x-sdl3-test: has=no nil
custom and text: data text/plain;charset=utf-8: has=yes \"as text!!\" (9 bytes)
outside: system's clip changed (text \"set outside SDL\")
outside: events: CLIPBOARD_UPDATE(owner=0 mime_types=[]), CLIPBOARD_UPDATE(owner=0 mime_types=[])
outside: has text=yes text=\"set outside SDL\"
outside: mime types=[]
outside: system: has primary clip=yes label=\"sdl3 test\" mime types=[text/plain] items=1 text=\"set outside SDL\"
outside: data application/x-sdl3-test: has=no nil
outside html: system's clip changed (html \"plain part\")
outside html: events: CLIPBOARD_UPDATE(owner=0 mime_types=[]), CLIPBOARD_UPDATE(owner=0 mime_types=[])
outside html: has text=yes text=\"plain part\"
outside html: mime types=[]
outside html: system: has primary clip=yes label=\"sdl3 test\" mime types=[text/html] items=1 text=\"plain part\"
outside clear: system's clip changed (clear)
outside clear: events: CLIPBOARD_UPDATE(owner=0 mime_types=[])
outside clear: has text=no text=\"\"
outside clear: mime types=[]
outside clear: system: has primary clip=no clip=null
outside unicode: system's clip changed (text \"été 😀\")
outside unicode: events: CLIPBOARD_UPDATE(owner=0 mime_types=[]), CLIPBOARD_UPDATE(owner=0 mime_types=[])
outside unicode: has text=yes text=\"été 😀\"
outside unicode: mime types=[]
outside unicode: system: has primary clip=yes label=\"sdl3 test\" mime types=[text/plain] items=1 text=\"été 😀\"
sdl after outside: set \"SDL's again\": ok
sdl after outside: events: CLIPBOARD_UPDATE(owner=1 mime_types=[text/plain;charset=utf-8]), CLIPBOARD_UPDATE(owner=0 mime_types=[])
sdl after outside: has text=yes text=\"SDL's again\"
sdl after outside: mime types=[]
sdl after outside: system: has primary clip=yes label=null mime types=[text/plain] items=1 text=\"SDL's again\"
clear data: ok
cleared: events: CLIPBOARD_UPDATE(owner=1 mime_types=[])
cleared: has text=no text=\"\"
cleared: mime types=[]
cleared: system: has primary clip=no clip=null
main returns
";

    static TRACE: Mutex<Vec<String>> = Mutex::new(Vec::new());

    fn t(line: impl Into<String>) {
        let line = line.into();
        println!("trace: {line}");
        TRACE.lock().unwrap().push(line);
    }

    fn result<T>(r: &sdl3::Result<T>) -> String {
        match r {
            Ok(_) => "ok".to_string(),
            Err(e) => format!("error: {e}"),
        }
    }

    fn yes(b: bool) -> &'static str {
        if b {
            "yes"
        } else {
            "no"
        }
    }

    /// Text as the trace writes it: control characters, the quote and the
    /// backslash as `\xNN`, the rest as it is.
    fn quoted(s: &str) -> String {
        let mut out = String::from("\"");
        // (upstream's C strings end at a NUL)
        for c in s.split('\0').next().unwrap_or_default().chars() {
            if (c as u32) < 0x20 || c == '\x7f' || c == '"' || c == '\\' {
                out += &format!("\\x{:02x}", c as u32);
            } else {
                out.push(c);
            }
        }
        out.push('"');
        out
    }

    /// The clipboard updates since the last [`trace_updates`].
    static UPDATES: Mutex<Vec<String>> = Mutex::new(Vec::new());

    fn watch() -> EventWatch {
        events::add_watch(|event| {
            if let Event::Clipboard(e) = event {
                UPDATES.lock().unwrap().push(format!(
                    "CLIPBOARD_UPDATE(owner={} mime_types=[{}])",
                    i32::from(e.owner),
                    e.mime_types.join(",")
                ));
            }
        })
    }

    /// Pump until `n` updates came (up to 3 s), then a second more (the
    /// system notifies its listeners asynchronously, of SDL's own changes
    /// too).
    fn pump_updates(n: usize) {
        let end = Instant::now() + Duration::from_millis(3000);
        while Instant::now() < end && UPDATES.lock().unwrap().len() < n {
            events::pump();
            events::flush_events(EventType::FIRST, EventType::LAST);
            std::thread::sleep(Duration::from_millis(10));
        }
        pump_for(1000);
    }

    fn trace_updates(step: &str) {
        let updates = std::mem::take(&mut *UPDATES.lock().unwrap());
        t(format!(
            "{step}: events: {}",
            if updates.is_empty() {
                "(none)".to_string()
            } else {
                updates.join(", ")
            }
        ));
    }

    /// The JNI environment of this thread, and the activity.
    struct Java {
        env: JNIEnv,
        activity: Jobject,
    }

    fn java() -> Java {
        let env = sdl3::android::jni_env()
            .expect("a JNI environment")
            .as_ptr();
        let activity = sdl3::android::activity().expect("the activity").as_ptr();
        Java { env, activity }
    }

    impl Java {
        fn clipboard_manager(&self) -> Jobject {
            let name = jni::new_string(self.env, "clipboard");
            let manager = jni::call_object(
                self.env,
                self.activity,
                "getSystemService",
                "(Ljava/lang/String;)Ljava/lang/Object;",
                &[Jvalue { l: name }],
            );
            jni::delete_local_ref(self.env, name);
            manager
        }

        /// A `CharSequence`, as the trace writes it (`null` for null).
        fn char_sequence(&self, s: Jobject) -> String {
            if s.is_null() {
                return "null".to_string();
            }
            let string = jni::call_object(self.env, s, "toString", "()Ljava/lang/String;", &[]);
            let text = quoted(&jni::string(self.env, string));
            jni::delete_local_ref(self.env, string);
            text
        }
    }

    impl Drop for Java {
        fn drop(&mut self) {
            jni::delete_local_ref(self.env, self.activity);
        }
    }

    /// The system's clip, as the ClipboardManager has it.
    fn trace_system(step: &str) {
        let j = java();
        let env = j.env;
        let manager = j.clipboard_manager();
        let has = jni::call_boolean(env, manager, "hasPrimaryClip", "()Z", &[]);
        let clip = jni::call_object(
            env,
            manager,
            "getPrimaryClip",
            "()Landroid/content/ClipData;",
            &[],
        );
        if clip.is_null() {
            t(format!(
                "{step}: system: has primary clip={} clip=null",
                yes(has)
            ));
        } else {
            let desc = jni::call_object(
                env,
                clip,
                "getDescription",
                "()Landroid/content/ClipDescription;",
                &[],
            );
            let ntypes = jni::call_int(env, desc, "getMimeTypeCount", "()I", &[]);
            let mut types = Vec::new();
            for i in 0..ntypes {
                let m = jni::call_object(
                    env,
                    desc,
                    "getMimeType",
                    "(I)Ljava/lang/String;",
                    &[Jvalue { i }],
                );
                types.push(jni::string(env, m));
                jni::delete_local_ref(env, m);
            }
            let l = jni::call_object(env, desc, "getLabel", "()Ljava/lang/CharSequence;", &[]);
            let label = j.char_sequence(l);
            let items = jni::call_int(env, clip, "getItemCount", "()I", &[]);
            let mut text = "none".to_string();
            if items > 0 {
                let item = jni::call_object(
                    env,
                    clip,
                    "getItemAt",
                    "(I)Landroid/content/ClipData$Item;",
                    &[Jvalue { i: 0 }],
                );
                let s = jni::call_object(env, item, "getText", "()Ljava/lang/CharSequence;", &[]);
                text = j.char_sequence(s);
                if !s.is_null() {
                    jni::delete_local_ref(env, s);
                }
                jni::delete_local_ref(env, item);
            }
            t(format!(
                "{step}: system: has primary clip={} label={label} mime types=[{}] items={items} text={text}",
                yes(has),
                types.join(",")
            ));
            if !l.is_null() {
                jni::delete_local_ref(env, l);
            }
            jni::delete_local_ref(env, desc);
            jni::delete_local_ref(env, clip);
        }
        jni::delete_local_ref(env, manager);
    }

    fn trace_mime_types(step: &str) {
        let types = clipboard::clipboard_mime_types().unwrap_or_default();
        t(format!("{step}: mime types=[{}]", types.join(",")));
    }

    fn trace_state(step: &str) {
        let text = clipboard::clipboard_text().unwrap_or_default();
        t(format!(
            "{step}: has text={} text={}",
            yes(clipboard::has_clipboard_text().unwrap_or(false)),
            quoted(&text)
        ));
        trace_mime_types(step);
        trace_system(step);
    }

    /// SDL's text set: `updates` are the updates it brings (SDL's own,
    /// then the system's for a clip of text; SDL's only for none).
    fn set_text(step: &str, text: &str, updates: usize) {
        t(format!(
            "{step}: set {}: {}",
            quoted(text),
            result(&clipboard::set_clipboard_text(text))
        ));
        pump_updates(updates);
        trace_updates(step);
        trace_state(step);
    }

    const CUSTOM: &str = "application/x-sdl3-test";

    /// The app's own data (`set_clipboard_data()`'s callback).
    fn data_callback() -> clipboard::ClipboardDataCallback {
        Arc::new(|mime_type: &str| {
            if mime_type == CUSTOM {
                Some(b"custom data".to_vec())
            } else if mime_type.starts_with("text/") {
                Some(b"as text!!".to_vec())
            } else {
                None
            }
        })
    }

    fn trace_data(step: &str, mime_type: &str) {
        let data = match clipboard::clipboard_data(mime_type) {
            Ok(Some(data)) => format!(
                "{} ({} bytes)",
                quoted(&String::from_utf8_lossy(&data)),
                data.len()
            ),
            _ => "nil".to_string(),
        };
        t(format!(
            "{step}: data {mime_type}: has={} {data}",
            yes(clipboard::has_clipboard_data(mime_type).unwrap_or(false))
        ));
    }

    /// The system's clip changed on a thread of its own (through the
    /// ClipboardManager, as another app's change would come).
    fn change(step: &str, what: &'static str, text: Option<&'static str>, updates: usize) {
        std::thread::Builder::new()
            .name("clip change".into())
            .spawn(move || {
                let j = java();
                let env = j.env;
                let manager = j.clipboard_manager();
                if what == "clear" {
                    jni::call_void(env, manager, "clearPrimaryClip", "()V", &[]);
                } else {
                    let cc = jni::find_class(env, "android/content/ClipData");
                    let label = jni::new_string(env, "sdl3 test");
                    let text = jni::new_string(env, text.unwrap_or_default());
                    let clip = if what == "html" {
                        let html = jni::new_string(env, "<b>bold</b> text");
                        let clip = jni::call_static_object(
                            env,
                            cc,
                            "newHtmlText",
                            "(Ljava/lang/CharSequence;Ljava/lang/CharSequence;Ljava/lang/String;)Landroid/content/ClipData;",
                            &[Jvalue { l: label }, Jvalue { l: text }, Jvalue { l: html }],
                        );
                        jni::delete_local_ref(env, html);
                        clip
                    } else {
                        jni::call_static_object(
                            env,
                            cc,
                            "newPlainText",
                            "(Ljava/lang/CharSequence;Ljava/lang/CharSequence;)Landroid/content/ClipData;",
                            &[Jvalue { l: label }, Jvalue { l: text }],
                        )
                    };
                    jni::call_void(
                        env,
                        manager,
                        "setPrimaryClip",
                        "(Landroid/content/ClipData;)V",
                        &[Jvalue { l: clip }],
                    );
                    jni::delete_local_ref(env, clip);
                    jni::delete_local_ref(env, text);
                    jni::delete_local_ref(env, label);
                    jni::delete_local_ref(env, cc);
                }
                jni::delete_local_ref(env, manager);
            })
            .unwrap()
            .join()
            .expect("the change's thread");
        t(format!(
            "{step}: system's clip changed ({what}{}{})",
            if text.is_some() { " " } else { "" },
            text.map(quoted).unwrap_or_default()
        ));
        pump_updates(updates);
        trace_updates(step);
        trace_state(step);
    }

    pub(super) fn main() {
        let text = clipboard::clipboard_text().unwrap_or_default();
        t(format!(
            "before init: set: {}",
            result(&clipboard::set_clipboard_text("too early"))
        ));
        t(format!(
            "before init: get: {}; has: {}",
            quoted(&text),
            yes(clipboard::has_clipboard_text().unwrap_or(false))
        ));

        t(format!("init: {}", result(&init::init(InitFlags::VIDEO))));
        t(format!(
            "video driver: {}",
            video::current_video_driver().unwrap_or("(none)")
        ));
        let watch = watch();

        // (the clipboard is the focused app's: a window, focused)
        let win = Window::create("SDL clipboard", 320, 240, WindowFlags::default());
        t(format!("window: {}", result(&win)));
        pump_for(1500);
        trace_updates("start");

        // (whatever the device had: emptied)
        set_text("emptied", "", 1);
        set_text("ascii", "Hello, clipboard!", 2);
        set_text("unicode", "Grüße, 世界 🎮", 2);
        set_text("again", "Hello, clipboard!", 2);
        set_text("control", "tab\there\nline \"quoted\" back\\slash", 2);
        set_text("empty", "", 1);
        set_text("empty again", "", 1);

        // The app's own data: another MIME type only (the system's clip is
        // emptied: no text to give it), then with text too
        t(format!(
            "custom: set data: {}",
            result(&clipboard::set_clipboard_data(
                Some(data_callback()),
                &[CUSTOM]
            ))
        ));
        pump_updates(1);
        trace_updates("custom");
        trace_state("custom");
        trace_data("custom", CUSTOM);
        trace_data("custom", "text/plain");
        t(format!(
            "custom and text: set data: {}",
            result(&clipboard::set_clipboard_data(
                Some(data_callback()),
                &[CUSTOM, "text/plain;charset=utf-8"]
            ))
        ));
        pump_updates(2);
        trace_updates("custom and text");
        trace_state("custom and text");
        trace_data("custom and text", CUSTOM);
        trace_data("custom and text", "text/plain;charset=utf-8");

        // Changes from outside SDL
        change("outside", "text", Some("set outside SDL"), 2);
        trace_data("outside", CUSTOM);
        change("outside html", "html", Some("plain part"), 2);
        change("outside clear", "clear", None, 1);
        change("outside unicode", "text", Some("été 😀"), 2);
        set_text("sdl after outside", "SDL's again", 2);
        t(format!(
            "clear data: {}",
            result(&clipboard::clear_clipboard_data())
        ));
        pump_updates(1);
        trace_updates("cleared");
        trace_state("cleared");

        drop(watch);
        if let Ok(w) = win {
            w.destroy();
        }
        init::quit();
        t("main returns");
        let trace = TRACE.lock().unwrap().clone();
        check_trace(&trace, EXPECTED);
    }
}
