// Tests of the file dialogs on Android (the system's documents' picker,
// answered by real taps), and of the "Open with" drop.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! An app (`harness = false`), run by `tools/android_run.py`, launched as
//! another app's "Open with" launches it (its runner word `data=URI`: the
//! intent `ACTION_VIEW` of a file's URI), whose main function makes a
//! window and traces:
//!
//! * the drop of the launch's file (`onCreate()` sends it, before `main`
//!   runs), its `DROP_*` events and the activity's log of it, and a view
//!   of another file while the app runs (the runner's `view`: the
//!   singleTop activity's `onNewIntent()`);
//! * the file dialogs (in a directory of the shared storage's Download the
//!   runner makes, its `files` request), each answered in the documents'
//!   picker (DocumentsUI) by the runner: real taps on the files and buttons
//!   it finds in uiautomator's dump of the picker (`ui-tap`), whose dump
//!   (`ui-dump`) also shows which files the filters let through, or BACK
//!   (`back-to-app`, the cancel path): open (a filter of one MIME type),
//!   open several (filters of several, and `*`: a long press, then a tap,
//!   then Select), open cancelled, a second dialog while one is shown,
//!   save (at the folder's URI; then with a suggested name), save
//!   cancelled, a folder (with and without
//!   `SDL_HINT_ANDROID_ALLOW_PERSISTENT_FOLDER_ACCESS`: the persisted URI
//!   permissions), a folder cancelled, `SDL_HINT_FILE_DIALOG_DRIVER` set,
//!   and an invalid filter. Each callback's file list (the documents'
//!   `content://` URIs), filter, error and thread (Java's name for it),
//!   and the app's and window's events meanwhile, are traced.
//!
//! The expected trace is upstream's C SDL's, from the same program in C
//! (built with upstream's android-project, Gradle and CMake, run by the
//! runner's `--run-apk`, with the same data) on the same emulator (API
//! 35, Google APIs, x86-64). Elsewhere this program does nothing.

#[cfg(not(target_os = "android"))]
fn main() {}

/// The runner's words: run as an app, launched with the file's URI.
#[cfg(target_os = "android")]
#[used]
#[link_section = ".sdl3_runner"]
static RUNNER: [u8; 57] = *b"app data=file:///sdcard/Download/sdl3_dialog/dropped.txt\0";

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
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Mutex;

    use sdl3::dialog::{self, DialogFileFilter};
    use sdl3::events::window::WindowFlags;
    use sdl3::events::{self, Event, EventType, EventWatch};
    use sdl3::hints;
    use sdl3::init::{self, InitFlags};
    use sdl3::video::{self, Window};

    use super::android_support::events::{self as ev, pump_for, pump_until};
    use super::android_support::jni;
    use super::android_support::{check_trace, request};

    /// Upstream's trace (the C program, on the emulator: API 35, Google
    /// APIs, x86-64).
    const EXPECTED: &str = "\
init: ok
video driver: android
window: ok
launch: drop events: (none)
logcat: V/SDL: Got filename: /sdcard/Download/sdl3_dialog/dropped.txt
view: drop events: (none)
logcat: ui: TextView \"sub\" [188,713][248,764]
logcat: ui: TextView \"a.txt\" [196,1310][268,1361]
logcat: ui: TextView \"b.png\" [683,1310][776,1361] disabled
logcat: ui: TextView \"c.jpg\" [196,1933][276,1984] disabled
logcat: ui: TextView \"notes.md\" [683,1933][838,1984] disabled
logcat: ui: tapped a.txt
open: callback: files=[content://com.android.externalstorage.documents/document/primary%3ADownload%2Fsdl3_dialog%2Fa.txt] filter=-1 thread=SDLActivity
open: focus events: WINDOW_FOCUS_GAINED, WINDOW_FOCUS_LOST
logcat: ui: TextView \"sub\" [188,713][248,764]
logcat: ui: TextView \"a.txt\" [196,1310][268,1361]
logcat: ui: TextView \"b.png\" [683,1310][776,1361]
logcat: ui: TextView \"c.jpg\" [196,1933][276,1984]
logcat: ui: TextView \"notes.md\" [683,1933][838,1984]
logcat: ui: long-pressed b.png
logcat: ui: tapped c.jpg
logcat: ui: tapped Select
open many: callback: files=[content://com.android.externalstorage.documents/document/primary%3ADownload%2Fsdl3_dialog%2Fb.png, content://com.android.externalstorage.documents/document/primary%3ADownload%2Fsdl3_dialog%2Fc.jpg] filter=-1 thread=SDLActivity
open many: focus events: WINDOW_FOCUS_GAINED, WINDOW_FOCUS_LOST
logcat: ui: TextView \"sub\" [188,713][248,764]
logcat: ui: TextView \"a.txt\" [196,1310][268,1361]
logcat: ui: TextView \"b.png\" [683,1310][776,1361]
logcat: ui: TextView \"c.jpg\" [196,1933][276,1984]
logcat: ui: TextView \"notes.md\" [683,1933][838,1984]
logcat: top: the app
open cancelled: callback: files=[] filter=-1 thread=SDLActivity
open cancelled: focus events: WINDOW_FOCUS_GAINED, WINDOW_FOCUS_LOST
second: called at once: yes
second: callback: error \"Only one file dialog can be run at a time.\" filter=-1 thread=SDLThread
logcat: top: the app
first: callback: files=[] filter=-1 thread=SDLActivity
first: focus events: WINDOW_FOCUS_GAINED, WINDOW_FOCUS_LOST
logcat: ui: EditText \"\" [189,2215][801,2333]
logcat: ui: tapped Save
save: callback: files=[content://com.android.externalstorage.documents/document/primary%3ADownload%2Fsdl3_dialog%2F(invalid).txt] filter=-1 thread=SDLActivity
save: focus events: WINDOW_FOCUS_GAINED, WINDOW_FOCUS_LOST
logcat: file: (invalid).txt 0
logcat: file: a.txt 6
logcat: file: b.png 6
logcat: file: c.jpg 6
logcat: file: notes.md 9
logcat: ui: EditText \"suggested.txt\" [189,2215][801,2333]
logcat: ui: tapped Save
save suggested: callback: files=[content://com.android.externalstorage.documents/document/primary%3ADownload%2Fsdl3_dialog%2Fsuggested.txt] filter=-1 thread=SDLActivity
save suggested: focus events: WINDOW_FOCUS_GAINED, WINDOW_FOCUS_LOST
logcat: file: (invalid).txt 0
logcat: file: a.txt 6
logcat: file: b.png 6
logcat: file: c.jpg 6
logcat: file: notes.md 9
logcat: file: suggested.txt 0
logcat: top: the app
save cancelled: callback: files=[] filter=-1 thread=SDLActivity
save cancelled: focus events: WINDOW_FOCUS_GAINED, WINDOW_FOCUS_LOST
persisted permissions: 0
logcat: ui: TextView \"sdk_gphone64_x86_64\" [0,254][331,380] disabled
logcat: ui: TextView \"Download\" [394,254][600,380] disabled
logcat: ui: TextView \"sdl3_dialog\" [663,254][893,380] disabled
logcat: ui: TextView \"sub\" [956,254][1080,380]
logcat: ui: tapped Use this folder
logcat: ui: Button \"CANCEL\"
logcat: ui: Button \"ALLOW\"
logcat: ui: tapped Allow
folder: callback: files=[content://com.android.externalstorage.documents/tree/primary%3ADownload%2Fsdl3_dialog%2Fsub] filter=-1 thread=SDLActivity
folder: focus events: WINDOW_FOCUS_GAINED, WINDOW_FOCUS_LOST
persisted permissions: 0
logcat: ui: tapped Use this folder
logcat: ui: tapped Allow
folder persistent: callback: files=[content://com.android.externalstorage.documents/tree/primary%3ADownload%2Fsdl3_dialog%2Fsub] filter=-1 thread=SDLActivity
folder persistent: focus events: WINDOW_FOCUS_GAINED, WINDOW_FOCUS_LOST
persisted permissions: 1
logcat: top: the app
folder cancelled: callback: files=[] filter=-1 thread=SDLActivity
folder cancelled: focus events: WINDOW_FOCUS_GAINED, WINDOW_FOCUS_LOST
driver hint: called at once: yes
driver hint: callback: error \"File dialog driver unsupported (don't set SDL_HINT_FILE_DIALOG_DRIVER)\" filter=-1 thread=SDLThread
driver hint: focus events: (none)
bad filter: callback: error \"Invalid dialog file filters: Invalid character in pattern (Only [a-zA-Z0-9_.-] allowed, or a single *)\" filter=-1 thread=SDLThread
bad filter: focus events: (none)
main returns
";

    const DIR_URI: &str =
        "content://com.android.externalstorage.documents/document/primary%3ADownload%2Fsdl3_dialog";

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

    /// Answers of the runner's requests, traced as the C program's are
    /// (the runner's lines, `logcat: LINE`).
    fn ask(what: &str) {
        for line in request(what) {
            t(format!("logcat: {line}"));
        }
    }

    /// The drops since the last [`trace_drops`].
    static DROPS: Mutex<Vec<String>> = Mutex::new(Vec::new());

    fn drop_watch() -> EventWatch {
        events::add_watch(|event| {
            if let Event::Drop(e) = event {
                let name = match e.event_type {
                    EventType::DROP_FILE => "DROP_FILE",
                    EventType::DROP_TEXT => "DROP_TEXT",
                    EventType::DROP_BEGIN => "DROP_BEGIN",
                    EventType::DROP_COMPLETE => "DROP_COMPLETE",
                    _ => "DROP_POSITION",
                };
                DROPS.lock().unwrap().push(format!(
                    "{name}(window={} source={} data={})",
                    if e.window_id != 0 { "W" } else { "0" },
                    e.source.as_deref().unwrap_or("nil"),
                    e.data.as_deref().unwrap_or("nil")
                ));
            }
        })
    }

    fn trace_drops(step: &str) {
        let drops = std::mem::take(&mut *DROPS.lock().unwrap());
        t(format!(
            "{step}: drop events: {}",
            if drops.is_empty() {
                "(none)".to_string()
            } else {
                drops.join(", ")
            }
        ));
    }

    /// The current thread's Java name (`Thread.currentThread().getName()`).
    fn java_thread_name() -> String {
        let env = sdl3::android::jni_env()
            .expect("a JNI environment")
            .as_ptr();
        let c = jni::find_class(env, "java/lang/Thread");
        let thread = jni::call_static_object(env, c, "currentThread", "()Ljava/lang/Thread;", &[]);
        let name = jni::call_object(env, thread, "getName", "()Ljava/lang/String;", &[]);
        let text = jni::string(env, name);
        jni::delete_local_ref(env, name);
        jni::delete_local_ref(env, thread);
        jni::delete_local_ref(env, c);
        text
    }

    /// The callback's
    static CALLED: AtomicBool = AtomicBool::new(false);
    static ANSWER: Mutex<String> = Mutex::new(String::new());

    fn callback(files: sdl3::Result<Vec<String>>, filter: Option<usize>) {
        let thread = java_thread_name();
        let filter = filter.map_or(-1, |f| f as i64);
        *ANSWER.lock().unwrap() = match files {
            Err(e) => format!("error \"{e}\" filter={filter} thread={thread}"),
            Ok(files) => format!(
                "files=[{}] filter={filter} thread={thread}",
                files.join(", ")
            ),
        };
        CALLED.store(true, Ordering::SeqCst);
    }

    /// Wait for the callback (up to 15 s, then 2 s more: the picker gone
    /// before the next one), then trace it and the events since the dialog
    /// was shown.
    fn answered(step: &str) {
        let came = pump_until(|| CALLED.load(Ordering::SeqCst), 15_000, 2000);
        t(format!(
            "{step}: callback: {}",
            if came {
                ANSWER.lock().unwrap().clone()
            } else {
                "none in 15 s".to_string()
            }
        ));
        CALLED.store(false, Ordering::SeqCst);
        // (the focus only: the pause and resume the picker makes cancel out
        // in the lifecycle queue when nothing pumped between them, so the
        // application and other window events are a matter of timing)
        let events = ev::take();
        t(format!(
            "{step}: focus events: {}",
            ev::names(&events, |ty| ty == EventType::WINDOW_FOCUS_GAINED
                || ty == EventType::WINDOW_FOCUS_LOST)
        ));
    }

    fn persisted() -> i32 {
        let env = sdl3::android::jni_env()
            .expect("a JNI environment")
            .as_ptr();
        let activity = sdl3::android::activity().expect("the activity").as_ptr();
        let resolver = jni::call_object(
            env,
            activity,
            "getContentResolver",
            "()Landroid/content/ContentResolver;",
            &[],
        );
        let list = jni::call_object(
            env,
            resolver,
            "getPersistedUriPermissions",
            "()Ljava/util/List;",
            &[],
        );
        let n = jni::call_int(env, list, "size", "()I", &[]);
        jni::delete_local_ref(env, list);
        jni::delete_local_ref(env, resolver);
        jni::delete_local_ref(env, activity);
        n
    }

    fn filter(name: &str, pattern: &str) -> DialogFileFilter {
        DialogFileFilter::new(name, pattern)
    }

    pub(super) fn main() {
        // (the pump doesn't block while the picker hides the app)
        let _ = hints::set(hints::ANDROID_BLOCK_ON_PAUSE, "0");
        t(format!("init: {}", result(&init::init(InitFlags::VIDEO))));
        t(format!(
            "video driver: {}",
            video::current_video_driver().unwrap_or("(none)")
        ));
        let drops = drop_watch();
        ev::watch();
        let win = Window::create("SDL dialogs", 320, 240, WindowFlags::default());
        t(format!("window: {}", result(&win)));
        let window = win.as_ref().ok().map(|w| w.id());
        pump_for(1500);
        ev::take();

        // The "Open with" launch's drop (sent by onCreate(), before init())
        trace_drops("launch");
        ask("logcat SDL --grep ^Got filename");
        // Another file viewed while the app runs (onNewIntent())
        ask("view file:///sdcard/Download/sdl3_dialog/viewed.txt");
        pump_for(2000);
        trace_drops("view");
        ev::take();

        ask("files reset a.txt b.png c.jpg notes.md sub/");

        // Open, one file: a filter of one MIME type
        let text = [filter("Text", "txt")];
        dialog::show_open_file_dialog(callback, window, &text, Some(DIR_URI), false);
        ask("ui-dump top --id android:id/title");
        ask("ui-tap --in top a.txt");
        answered("open");

        // Open, several files: filters of several MIME types (and "*")
        let images = [filter("Images", "png;jpg"), filter("All", "*")];
        dialog::show_open_file_dialog(callback, window, &images, Some(DIR_URI), true);
        ask("ui-dump top --id android:id/title");
        ask("ui-tap --long --in top b.png");
        ask("ui-tap --in top c.jpg");
        ask("ui-tap --in top Select");
        answered("open many");

        // Open, cancelled (BACK in the picker)
        dialog::show_open_file_dialog(callback, window, &[], Some(DIR_URI), false);
        ask("ui-dump top --id android:id/title");
        ask("back-to-app");
        answered("open cancelled");

        // One at a time
        dialog::show_open_file_dialog(callback, window, &[], Some(DIR_URI), false);
        {
            dialog::show_open_file_dialog(callback, window, &[], Some(DIR_URI), false);
            // (the second's callback, at once, on this thread)
            t(format!(
                "second: called at once: {}",
                yes(CALLED.load(Ordering::SeqCst))
            ));
            let first = ANSWER.lock().unwrap().clone();
            CALLED.store(false, Ordering::SeqCst);
            t(format!("second: callback: {first}"));
        }
        ask("back-to-app");
        answered("first");

        // Save: a filter of one MIME type, the folder's URI (ending with a
        // slash: no name suggested)
        let folder = format!("{DIR_URI}/");
        dialog::show_save_file_dialog(callback, window, &text, Some(&folder));
        ask("ui-dump top --class EditText");
        ask("ui-tap --in top Save");
        answered("save");
        ask("files list");

        // Save with a suggested name (the location's last part)
        dialog::show_save_file_dialog(callback, window, &text, Some("suggested.txt"));
        ask("ui-dump top --class EditText");
        ask("ui-tap --in top Save");
        answered("save suggested");
        ask("files list");

        // Save, cancelled
        dialog::show_save_file_dialog(callback, window, &[], Some(&folder));
        ask("back-to-app");
        answered("save cancelled");

        // A folder, without persistent access, then with it
        let sub = format!("{DIR_URI}%2Fsub");
        t(format!("persisted permissions: {}", persisted()));
        dialog::show_open_folder_dialog(callback, window, Some(&sub), false);
        ask("ui-dump top --id breadcrumb_text");
        ask("ui-tap --in top Use this folder");
        ask("ui-dump top --class Button --no-bounds");
        ask("ui-tap --in top Allow");
        answered("folder");
        t(format!("persisted permissions: {}", persisted()));
        let _ = hints::set(hints::ANDROID_ALLOW_PERSISTENT_FOLDER_ACCESS, "1");
        dialog::show_open_folder_dialog(callback, window, Some(&sub), false);
        ask("ui-tap --in top Use this folder");
        ask("ui-tap --in top Allow");
        answered("folder persistent");
        t(format!("persisted permissions: {}", persisted()));

        // A folder, cancelled
        dialog::show_open_folder_dialog(callback, window, Some(DIR_URI), false);
        ask("back-to-app");
        answered("folder cancelled");

        // SDL_HINT_FILE_DIALOG_DRIVER set
        let _ = hints::set(hints::FILE_DIALOG_DRIVER, "portal");
        dialog::show_open_file_dialog(callback, window, &[], None, false);
        t(format!(
            "driver hint: called at once: {}",
            yes(CALLED.load(Ordering::SeqCst))
        ));
        answered("driver hint");
        hints::reset(hints::FILE_DIALOG_DRIVER);

        // Invalid filters (SDL_dialog.c's)
        let bad = [filter("Bad", "a b")];
        dialog::show_open_file_dialog(callback, window, &bad, None, false);
        answered("bad filter");

        ev::unwatch();
        drop(drops);
        if let Ok(w) = win {
            w.destroy();
        }
        init::quit();
        t("main returns");
        let trace = TRACE.lock().unwrap().clone();
        check_trace(&trace, EXPECTED);
    }
}
