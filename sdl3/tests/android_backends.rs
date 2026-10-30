// Tests of Android's small backends in an app: the filesystem paths, the
// locale, the battery, URLs, toasts, permissions, the assets' and the
// content resolver's file I/O, and the generic storage on them.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! An app (as android_main.rs is), run by `tools/android_run.py`, with
//! assets of its own in its package (the word `assets=tests/android_assets`)
//! and two permissions in its manifest (`permissions=...`), which checks
//! what an app sees of the backends:
//!
//! * the paths: the base path (the assets, `assets://`), the pref path and
//!   the internal storage, the external storage's path and state (the
//!   state logged too: the runner reads the device's log back), the cache,
//!   the user folders (none on Android), the app's name (the executable's:
//!   the package's) and the current directory;
//! * the locale, as the app's locales change (`cmd locale
//!   set-app-locales`: with and without a country, Indonesian, a script);
//! * the battery, as the emulator's changes (`adb emu power`: on battery,
//!   charging, charged, no battery);
//! * the assets through `IoStream::from_file()` (relative paths, `./`,
//!   `assets://`; size, seeks, reads to the end, a write's error, the
//!   `AAsset` property, missing assets), the internal storage's files that
//!   shadow them, the file operations on both (enumerations, path info,
//!   globs, a copy, directories, a rename), and a `content://` URI of a
//!   file the program puts in `MediaStore`'s downloads (written and read
//!   through the content resolver, then deleted);
//! * the title storage on the assets and the user storage on the pref path;
//! * a toast (whether the system shows it), and URLs (a custom scheme no
//!   app answers, and an http one, with the activity that answers it);
//! * permissions: `pm revoke` and `pm grant` (requests of the runner's),
//!   and `RequestAndroidPermission()` answered at once (granted already),
//!   by the system's dialog (denied, then allowed: the runner taps its
//!   button, found with `uiautomator`), and for a permission the manifest
//!   doesn't ask for (denied without a dialog).
//!
//! The expected trace is upstream's C SDL's, from the same program in C
//! (built with upstream's android-project, Gradle and CMake, with the same
//! assets and permissions, run by the runner's `--run-apk`) on the same
//! emulator (API 35, Google APIs, x86-64), with the package's name and the
//! `MediaStore` file's ID as `PKG` and `N`. Elsewhere this program does
//! nothing.

#[cfg(not(target_os = "android"))]
fn main() {}

/// The runner's words: run as an app, with these assets and permissions.
#[cfg(target_os = "android")]
#[used]
#[link_section = ".sdl3_runner"]
static RUNNER: [u8; 72] =
    *b"app assets=tests/android_assets permissions=READ_CONTACTS,READ_CALENDAR\0";

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
    use std::ops::ControlFlow;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use sdl3::events::{self, EventType, EventWatch};
    use sdl3::filesystem::{self, Folder, GlobFlags, PathType};
    use sdl3::init::{self, AppMetadata, InitFlags};
    use sdl3::io::{IoStatus, IoStream, IoWhence, PROP_IOSTREAM_ANDROID_AASSET_POINTER};
    use sdl3::power::{self, PowerState};
    use sdl3::storage::Storage;
    use sdl3::{android, locale, misc};

    use super::android_support::jni::{self, Jobject, Jvalue};
    use super::android_support::{process_name, request};

    /// Upstream's trace (the C program, on the emulator: API 35, Google
    /// APIs, x86-64).
    const EXPECTED: &str = "\
init: ok
base: assets://
pref: /data/data/PKG/files/
pref, no org: /data/data/PKG/files/
internal: /data/data/PKG/files
external state: 3
reply: I/SDL: external storage state: mounted
external: /storage/emulated/0/Android/data/PKG/files
cache: /data/user/0/PKG/cache
folder HOME: error 'That operation is not supported'
folder DESKTOP: error 'That operation is not supported'
folder DOCUMENTS: error 'That operation is not supported'
folder DOWNLOADS: error 'That operation is not supported'
folder MUSIC: error 'That operation is not supported'
folder PICTURES: error 'That operation is not supported'
folder PUBLICSHARE: error 'That operation is not supported'
folder SAVEDGAMES: error 'That operation is not supported'
folder SCREENSHOTS: error 'That operation is not supported'
folder TEMPLATES: error 'That operation is not supported'
folder VIDEOS: error 'That operation is not supported'
app name: PKG
current directory: /
locales: en_US
locale fr-FR: LOCALE_CHANGED; locales: fr_FR, en_US
locale fr: LOCALE_CHANGED; locales: fr, en_US
locale id-ID: LOCALE_CHANGED; locales: id_ID, en_US
locale zh-Hant-TW,de-DE: LOCALE_CHANGED; locales: zh_TW, de_DE, en_US
locale reset: LOCALE_CHANGED; locales: en_US
power: CHARGING seconds -1 percent 100
power ac off, status discharging, capacity 42: ON_BATTERY seconds -1 percent 42
power ac on, status charging, capacity 50: CHARGING seconds -1 percent 50
power status full, capacity 100: CHARGED seconds -1 percent 100
power status charging, present false: NO_BATTERY seconds -1 percent -1
power restored: CHARGING seconds -1 percent 100
open hello.txt: ok, size 14
read 5: 5 \"Hello\"
tell: 5
seek 7: 7
read 5: 5 \"world\"
seek -1 from the end: 13
read 4: 1 \"\" status EOF
seek 100: error ''
seek -5 from the start: error ''
aasset property: yes
close: ok
write: 0 status ERROR error 'Cannot write to Android package filesystem'
file assets://dir/inner.txt: size 6, 6 \"inner\" status EOF
file dir/sub/deep.bin: size 9, 9 \"deep data\" status EOF
file ./hello.txt: size 14, 14 \"Hello, world!\" status EOF
file assets:///hello.txt: size 14, 14 \"Hello, world!\" status EOF
file missing.txt: error 'Couldn't open asset 'missing.txt''
file assets://dir: error 'Couldn't open asset 'dir''
file /data/local/tmp/sdl3-missing: error 'Couldn't open asset 'data/local/tmp/sdl3-missing''
write note.txt: 9 bytes, close ok
file note.txt: size 9, 9 \"internal\" status EOF
write hello.txt: 9 bytes, close ok
file hello.txt: size 9, 9 \"shadowed\" status EOF
file assets://hello.txt: size 14, 14 \"Hello, world!\" status EOF
info hello.txt: type 1 size 9
remove hello.txt: ok
file hello.txt: size 14, 14 \"Hello, world!\" status EOF
enumerate assets://: [assets://|dir] [assets://|hello.txt]
enumerate assets://dir: [assets://dir/|inner.txt] [assets://dir/|sub]
enumerate assets://dir//: [assets://dir//|inner.txt] [assets://dir//|sub]
enumerate assets://hello.txt: error 'No such directory'
enumerate assets://nope: error 'No such directory'
enumerate dir: error 'Can't open directory: No such file or directory'
enumerate : error 'No such file or directory'
asset time: 347245262
asset times equal: yes
info assets://hello.txt: type 1 size 14
info assets://dir: type 2 size 0
info assets://: type 2 size 0
info hello.txt: type 1 size 14
info ./dir: type 2 size 0
info dir/sub/deep.bin: type 1 size 9
info assets://nope: error 'No such file or directory'
info assets://hello.txt/x: error 'No such file or directory'
info : error 'No such file or directory'
info /data/local/tmp/sdl3-missing: error 'No such file or directory'
glob assets:// *.txt: [hello.txt]
glob assets:// (null): [dir/inner.txt] [dir/sub/deep.bin] [dir/sub] [dir] [hello.txt]
glob assets://dir */*: [sub/deep.bin]
glob assets://dir *.TXT: [inner.txt]
copy assets://hello.txt copy.txt: ok
file copy.txt: size 14, 14 \"Hello, world!\" status EOF
mkdir newdir: ok
mkdir newdir again: error 'Can't create directory: File exists'
info newdir: type 2 size 0
rename note.txt note2.txt: ok
enumerate the internal storage: [/data/data/PKG/files/|copy.txt] [/data/data/PKG/files/|newdir] [/data/data/PKG/files/|note2.txt]
remove copy.txt: ok
remove note2.txt: ok
remove newdir: ok
enumerate the internal storage:\x20
content insert: content://media/external/downloads/N
content write: 8 bytes, close ok
content read: size 8, 8 \"content\" status EOF
content delete: 1
content read deleted: error 'Unspecified error in JNI'
title storage: ready yes
title size hello.txt: 14
title read hello.txt: \"Hello, world!\"
title info dir: type 2
title enumerate (null): [|dir] [|hello.txt]
title enumerate dir: [dir/|inner.txt] [dir/|sub]
title glob *.txt: [hello.txt]
title glob (null): [dir/inner.txt] [dir/sub/deep.bin] [dir/sub] [dir] [hello.txt]
title write: error 'That operation is not supported'
title space: error 'That operation is not supported'
title close: ok
user storage: ready yes
user write save.txt: ok
user read save.txt: \"1\"
the pref path's save.txt: type 1 size 1
user enumerate (null): [|save.txt]
user space: 18446744073709551615
user remove save.txt: ok
user close: ok
file storage info hello.txt: type 1 size 14
file storage close: ok
toast: ok
reply: toast: shown
toast centered: ok
open sdl3-backends-test://hello: error ''
open https://www.libsdl.org/: ok
reply: top: com.android.chrome
reply: top: the app
request android.permission.READ_CONTACTS: ok
reply: dialog: tapped permission_deny_button
callback: android.permission.READ_CONTACTS denied, at once: no
request android.permission.READ_CONTACTS: ok
callback: android.permission.READ_CONTACTS granted, at once: yes
request android.permission.READ_CALENDAR: ok
reply: dialog: tapped permission_allow_button
callback: android.permission.READ_CALENDAR granted, at once: no
request android.permission.READ_SMS: ok
callback: android.permission.READ_SMS denied, at once: no
main returns
";

    static TRACE: Mutex<Vec<String>> = Mutex::new(Vec::new());
    static LOCALE_EVENTS: Mutex<Vec<EventType>> = Mutex::new(Vec::new());

    /// The trace's line, with the package's name as `PKG`.
    fn t(line: impl Into<String>) {
        let line = line.into().replace(&process_name(), "PKG");
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

    /// A runner's request, its answer's lines in the trace (`reply: LINE`,
    /// as the C reference's runner prints them).
    fn ask(what: &str) {
        for line in request(what) {
            t(format!("reply: {line}"));
        }
    }

    /// A result as the C program prints it: its value, or `error 'MESSAGE'`.
    fn shown<T>(r: sdl3::Result<T>, f: impl FnOnce(T) -> String) -> String {
        match r {
            Ok(v) => f(v),
            Err(e) => format!("error '{}'", e.message()),
        }
    }

    fn pump() {
        events::pump();
        events::flush_events(EventType::FIRST, EventType::LAST);
    }

    /// Pump until a `LOCALE_CHANGED` event, for `ms` at most, then 300 ms
    /// more for the rest: those events.
    fn locale_events(ms: u64) -> String {
        let end = Instant::now() + Duration::from_millis(ms);
        while Instant::now() < end && LOCALE_EVENTS.lock().unwrap().is_empty() {
            pump();
            std::thread::sleep(Duration::from_millis(10));
        }
        let end = Instant::now() + Duration::from_millis(300);
        while Instant::now() < end {
            pump();
            std::thread::sleep(Duration::from_millis(10));
        }
        let seen = std::mem::take(&mut *LOCALE_EVENTS.lock().unwrap());
        if seen.is_empty() {
            "(none)".to_string()
        } else {
            vec!["LOCALE_CHANGED"; seen.len()].join(", ")
        }
    }

    fn locales() -> String {
        shown(locale::preferred_locales(), |l| {
            let names: Vec<String> = l
                .iter()
                .map(|l| match &l.country {
                    Some(c) => format!("{}_{c}", l.language),
                    None => l.language.clone(),
                })
                .collect();
            if names.is_empty() {
                "(none)".to_string()
            } else {
                names.join(", ")
            }
        })
    }

    fn power_name(state: PowerState) -> &'static str {
        match state {
            PowerState::Error => "ERROR",
            PowerState::Unknown => "UNKNOWN",
            PowerState::OnBattery => "ON_BATTERY",
            PowerState::NoBattery => "NO_BATTERY",
            PowerState::Charging => "CHARGING",
            PowerState::Charged => "CHARGED",
        }
    }

    /// The battery, as `STATE seconds S percent P` (-1: not known), once it
    /// is `want` (state and percent), or 10 s have gone.
    fn power_until(want: Option<(PowerState, i32)>) -> String {
        let end = Instant::now() + Duration::from_secs(10);
        loop {
            let info = power::power_info();
            let percent = info.percent.map_or(-1, i32::from);
            let seconds = info.time_left.map_or(-1, |d| d.as_secs() as i64);
            if want.is_none_or(|w| w == (info.state, percent)) || Instant::now() > end {
                return format!(
                    "{} seconds {seconds} percent {percent}",
                    power_name(info.state)
                );
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    fn status_name(s: IoStatus) -> &'static str {
        match s {
            IoStatus::Ready => "READY",
            IoStatus::Error => "ERROR",
            IoStatus::Eof => "EOF",
            IoStatus::NotReady => "NOT_READY",
            IoStatus::ReadOnly => "READONLY",
            IoStatus::WriteOnly => "WRITEONLY",
        }
    }

    /// Up to `n` bytes read, as `COUNT "TEXT"` (the text without its line
    /// end), with the status when the read was short.
    fn read(io: &mut IoStream<'_>, n: usize) -> String {
        let mut buf = vec![0u8; n];
        let got = io.read(&mut buf);
        let text = String::from_utf8_lossy(&buf[..got]);
        let mut line = format!("{got} \"{}\"", text.trim_end_matches('\n'));
        if got < n {
            line += &format!(" status {}", status_name(io.status()));
        }
        line
    }

    /// A file's whole text (its line end dropped), or its error.
    fn file_text(path: &str) -> String {
        shown(IoStream::from_file(path, "rb"), |mut io| {
            let size = io.size().unwrap_or(-1);
            format!("size {size}, {}", read(&mut io, 64))
        })
    }

    /// A file written with `text`.
    fn write_file(path: &str, mode: &str, text: &str) -> String {
        shown(IoStream::from_file(path, mode), |mut io| {
            let n = io.write(text.as_bytes());
            let closed = shown(io.close(), |()| "ok".to_string());
            format!("{n} bytes, close {closed}")
        })
    }

    fn info(path: &str) -> String {
        shown(filesystem::get_path_info(path), |i| {
            let kind = match i.kind {
                PathType::None => 0,
                PathType::File => 1,
                PathType::Directory => 2,
                PathType::Other => 3,
            };
            format!("type {kind} size {}", i.size)
        })
    }

    /// A directory's entries (`DIRNAME|NAME`, sorted: their order is the
    /// package's), or its error.
    fn enumerate(path: &str) -> String {
        let mut seen = Vec::new();
        let r = filesystem::enumerate_directory(path, |dir, name| {
            seen.push(format!("[{dir}|{name}]"));
            Ok(ControlFlow::Continue(()))
        });
        seen.sort();
        shown(r, |()| seen.join(" "))
    }

    fn list(r: sdl3::Result<Vec<String>>) -> String {
        shown(r, |v| {
            // (sorted as the C program sorts them: in their brackets)
            let mut v: Vec<String> = v.iter().map(|p| format!("[{p}]")).collect();
            v.sort();
            v.join(" ")
        })
    }

    fn ok(r: sdl3::Result<()>) -> String {
        shown(r, |()| "ok".to_string())
    }

    fn env() -> jni::JNIEnv {
        android::jni_env().expect("a JNI environment").as_ptr()
    }

    /// The content resolver's new file in `MediaStore`'s downloads (as the
    /// C program makes it): its `content://` URI.
    fn insert_download(env: jni::JNIEnv, activity: Jobject) -> (Jobject, Jobject, String) {
        let resolver = jni::call_object(
            env,
            activity,
            "getContentResolver",
            "()Landroid/content/ContentResolver;",
            &[],
        );
        let values_class = jni::find_class(env, "android/content/ContentValues");
        let values = jni::new_object(env, values_class, "()V", &[]);
        for (key, value) in [
            ("_display_name", "sdl3-android-backends.txt"),
            ("mime_type", "text/plain"),
        ] {
            let (k, v) = (jni::new_string(env, key), jni::new_string(env, value));
            jni::call_void(
                env,
                values,
                "put",
                "(Ljava/lang/String;Ljava/lang/String;)V",
                &[Jvalue { l: k }, Jvalue { l: v }],
            );
            jni::delete_local_ref(env, k);
            jni::delete_local_ref(env, v);
        }
        let downloads = jni::find_class(env, "android/provider/MediaStore$Downloads");
        let collection = jni::get_static_object_field(
            env,
            downloads,
            "EXTERNAL_CONTENT_URI",
            "Landroid/net/Uri;",
        );
        let uri = jni::call_object(
            env,
            resolver,
            "insert",
            "(Landroid/net/Uri;Landroid/content/ContentValues;)Landroid/net/Uri;",
            &[Jvalue { l: collection }, Jvalue { l: values }],
        );
        assert!(!uri.is_null(), "ContentResolver.insert() gave null");
        let string = jni::call_object(env, uri, "toString", "()Ljava/lang/String;", &[]);
        let text = jni::string(env, string);
        for r in [string, collection, downloads, values, values_class] {
            jni::delete_local_ref(env, r);
        }
        (resolver, uri, text)
    }

    /// `content://.../ID` with the ID as `N`.
    fn normalized_uri(uri: &str) -> String {
        match uri.rsplit_once('/') {
            Some((head, id)) if !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()) => {
                format!("{head}/N")
            }
            _ => uri.to_string(),
        }
    }

    /// A permission's request, and its answer (`callback: ...`), within 20
    /// s, with `answer` (a runner's request) once it's submitted.
    fn permission(name: &str, answer: Option<&str>) {
        let result: Arc<Mutex<Option<(String, bool)>>> = Arc::new(Mutex::new(None));
        let returned = Arc::new(AtomicBool::new(false));
        let at_once = Arc::new(AtomicBool::new(false));
        let (r, ret, once) = (result.clone(), returned.clone(), at_once.clone());
        let submitted = android::request_permission(name, move |permission, granted| {
            once.store(!ret.load(Ordering::SeqCst), Ordering::SeqCst);
            *r.lock().unwrap() = Some((permission.to_string(), granted));
        });
        returned.store(true, Ordering::SeqCst);
        t(format!("request {name}: {}", ok(submitted)));
        if let Some(answer) = answer {
            ask(answer);
        }
        let end = Instant::now() + Duration::from_secs(20);
        while result.lock().unwrap().is_none() && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(10));
        }
        let answered = result.lock().unwrap().clone();
        match answered {
            Some((permission, granted)) => t(format!(
                "callback: {permission} {}, at once: {}",
                if granted { "granted" } else { "denied" },
                yes(at_once.load(Ordering::SeqCst))
            )),
            None => t("callback: none in 20 s"),
        }
    }

    pub(super) fn main() {
        t(format!("init: {}", ok(init::init(InitFlags::EVENTS))));
        let _watch: EventWatch = events::add_watch(|event| {
            if event.event_type() == EventType::LOCALE_CHANGED {
                LOCALE_EVENTS
                    .lock()
                    .unwrap()
                    .push(EventType::LOCALE_CHANGED);
            }
        });

        // The paths
        let path = |r: sdl3::Result<String>| shown(r, |p| p);
        t(format!("base: {}", path(filesystem::get_base_path())));
        t(format!(
            "pref: {}",
            path(filesystem::get_pref_path(Some("SDL3 Test"), "Backends"))
        ));
        t(format!(
            "pref, no org: {}",
            path(filesystem::get_pref_path(None, "Other"))
        ));
        t(format!(
            "internal: {}",
            path(android::internal_storage_path())
        ));
        t(format!(
            "external state: {}",
            android::external_storage_state()
        ));
        ask("logcat SDL:I --grep external storage state");
        t(format!(
            "external: {}",
            path(android::external_storage_path())
        ));
        t(format!("cache: {}", path(android::cache_path())));
        for (folder, name) in [
            (Folder::Home, "HOME"),
            (Folder::Desktop, "DESKTOP"),
            (Folder::Documents, "DOCUMENTS"),
            (Folder::Downloads, "DOWNLOADS"),
            (Folder::Music, "MUSIC"),
            (Folder::Pictures, "PICTURES"),
            (Folder::PublicShare, "PUBLICSHARE"),
            (Folder::SavedGames, "SAVEDGAMES"),
            (Folder::Screenshots, "SCREENSHOTS"),
            (Folder::Templates, "TEMPLATES"),
            (Folder::Videos, "VIDEOS"),
        ] {
            t(format!(
                "folder {name}: {}",
                path(filesystem::get_user_folder(folder))
            ));
        }
        t(format!(
            "app name: {}",
            init::app_metadata_property(AppMetadata::Name).unwrap_or_default()
        ));
        t(format!(
            "current directory: {}",
            path(filesystem::get_current_directory())
        ));

        // The locale
        t(format!("locales: {}", locales()));
        for (tags, label) in [
            ("fr-FR", "fr-FR"),
            ("fr", "fr"),
            ("id-ID", "id-ID"),
            ("zh-Hant-TW,de-DE", "zh-Hant-TW,de-DE"),
            ("", "reset"),
        ] {
            ask(format!("locale {tags}").trim_end());
            let events = locale_events(10_000);
            t(format!("locale {label}: {events}; locales: {}", locales()));
        }

        // The battery
        t(format!("power: {}", power_until(None)));
        for (requests, label, want) in [
            (
                &["ac off", "status discharging", "capacity 42"][..],
                "ac off, status discharging, capacity 42",
                (PowerState::OnBattery, 42),
            ),
            (
                &["ac on", "status charging", "capacity 50"][..],
                "ac on, status charging, capacity 50",
                (PowerState::Charging, 50),
            ),
            (
                &["status full", "capacity 100"][..],
                "status full, capacity 100",
                (PowerState::Charged, 100),
            ),
            (
                &["status charging", "present false"][..],
                "status charging, present false",
                (PowerState::NoBattery, -1),
            ),
            (
                &["present true", "ac on", "status charging", "capacity 100"][..],
                "restored",
                (PowerState::Charging, 100),
            ),
        ] {
            for r in requests {
                ask(&format!("power {r}"));
            }
            t(format!("power {label}: {}", power_until(Some(want))));
        }

        // The assets, through SDL_IOFromFile()
        match IoStream::from_file("hello.txt", "rb") {
            Err(e) => t(format!("open hello.txt: error '{}'", e.message())),
            Ok(mut io) => {
                t(format!(
                    "open hello.txt: ok, size {}",
                    shown(io.size(), |s| s.to_string())
                ));
                t(format!("read 5: {}", read(&mut io, 5)));
                t(format!("tell: {}", shown(io.tell(), |p| p.to_string())));
                t(format!(
                    "seek 7: {}",
                    shown(io.seek(7, IoWhence::Set), |p| p.to_string())
                ));
                t(format!("read 5: {}", read(&mut io, 5)));
                t(format!(
                    "seek -1 from the end: {}",
                    shown(io.seek(-1, IoWhence::End), |p| p.to_string())
                ));
                t(format!("read 4: {}", read(&mut io, 4)));
                t(format!(
                    "seek 100: {}",
                    shown(io.seek(100, IoWhence::Set), |p| p.to_string())
                ));
                t(format!(
                    "seek -5 from the start: {}",
                    shown(io.seek(-5, IoWhence::Set), |p| p.to_string())
                ));
                let pointer = io
                    .properties()
                    .ok()
                    .and_then(|p| p.get_number(PROP_IOSTREAM_ANDROID_AASSET_POINTER));
                t(format!(
                    "aasset property: {}",
                    yes(pointer.is_some_and(|p| p != 0))
                ));
                t(format!("close: {}", ok(io.close())));
            }
        }
        match IoStream::from_file("assets://hello.txt", "wb") {
            Err(e) => t(format!(
                "open assets://hello.txt wb: error '{}'",
                e.message()
            )),
            Ok(mut io) => {
                let n = io.write(b"x");
                t(format!(
                    "write: {n} status {} error '{}'",
                    status_name(io.status()),
                    io.last_error().map_or("", |e| e.message())
                ));
            }
        }
        for file in [
            "assets://dir/inner.txt",
            "dir/sub/deep.bin",
            "./hello.txt",
            "assets:///hello.txt",
            "missing.txt",
            "assets://dir",
            "/data/local/tmp/sdl3-missing",
        ] {
            t(format!("file {file}: {}", file_text(file)));
        }

        // The internal storage's files, which shadow the assets
        t(format!(
            "write note.txt: {}",
            write_file("note.txt", "wb", "internal\n")
        ));
        t(format!("file note.txt: {}", file_text("note.txt")));
        t(format!(
            "write hello.txt: {}",
            write_file("hello.txt", "wb", "shadowed\n")
        ));
        t(format!("file hello.txt: {}", file_text("hello.txt")));
        t(format!(
            "file assets://hello.txt: {}",
            file_text("assets://hello.txt")
        ));
        t(format!("info hello.txt: {}", info("hello.txt")));
        t(format!(
            "remove hello.txt: {}",
            ok(filesystem::remove_path("hello.txt"))
        ));
        t(format!("file hello.txt: {}", file_text("hello.txt")));

        // The file operations
        for p in [
            "assets://",
            "assets://dir",
            "assets://dir//",
            "assets://hello.txt",
            "assets://nope",
            "dir",
            "",
        ] {
            t(format!("enumerate {p}: {}", enumerate(p)));
        }
        let asset = filesystem::get_path_info("assets://hello.txt").ok();
        t(format!(
            "asset time: {}",
            asset.map_or(-1, |i| i.modify_time.as_nanos())
        ));
        t(format!(
            "asset times equal: {}",
            yes(asset
                .is_some_and(|i| i.create_time == i.modify_time && i.modify_time == i.access_time))
        ));
        for p in [
            "assets://hello.txt",
            "assets://dir",
            "assets://",
            "hello.txt",
            "./dir",
            "dir/sub/deep.bin",
            "assets://nope",
            "assets://hello.txt/x",
            "",
            "/data/local/tmp/sdl3-missing",
        ] {
            t(format!("info {p}: {}", info(p)));
        }
        for (p, pattern, flags) in [
            ("assets://", Some("*.txt"), GlobFlags::NONE),
            ("assets://", None, GlobFlags::NONE),
            ("assets://dir", Some("*/*"), GlobFlags::NONE),
            ("assets://dir", Some("*.TXT"), GlobFlags::CASE_INSENSITIVE),
        ] {
            t(format!(
                "glob {p} {}: {}",
                pattern.unwrap_or("(null)"),
                list(filesystem::glob_directory(p, pattern, flags))
            ));
        }
        t(format!(
            "copy assets://hello.txt copy.txt: {}",
            ok(filesystem::copy_file("assets://hello.txt", "copy.txt"))
        ));
        t(format!("file copy.txt: {}", file_text("copy.txt")));
        t(format!(
            "mkdir newdir: {}",
            ok(filesystem::create_directory("newdir"))
        ));
        t(format!(
            "mkdir newdir again: {}",
            ok(filesystem::create_directory("newdir"))
        ));
        t(format!("info newdir: {}", info("newdir")));
        t(format!(
            "rename note.txt note2.txt: {}",
            ok(filesystem::rename_path("note.txt", "note2.txt"))
        ));
        let internal = android::internal_storage_path().unwrap_or_default();
        t(format!(
            "enumerate the internal storage: {}",
            enumerate(&internal)
        ));
        for p in ["copy.txt", "note2.txt", "newdir"] {
            t(format!("remove {p}: {}", ok(filesystem::remove_path(p))));
        }
        t(format!(
            "enumerate the internal storage: {}",
            enumerate(&internal)
        ));

        // A content:// URI
        let env = env();
        let activity = android::activity().expect("the activity").as_ptr();
        let (resolver, uri, text) = insert_download(env, activity);
        t(format!("content insert: {}", normalized_uri(&text)));
        t(format!(
            "content write: {}",
            write_file(&text, "wb", "content\n")
        ));
        t(format!("content read: {}", file_text(&text)));
        let deleted = jni::call_int(
            env,
            resolver,
            "delete",
            "(Landroid/net/Uri;Ljava/lang/String;[Ljava/lang/String;)I",
            &[
                Jvalue { l: uri },
                Jvalue {
                    l: std::ptr::null_mut(),
                },
                Jvalue {
                    l: std::ptr::null_mut(),
                },
            ],
        );
        t(format!("content delete: {deleted}"));
        t(format!("content read deleted: {}", file_text(&text)));
        jni::delete_local_ref(env, uri);
        jni::delete_local_ref(env, resolver);
        jni::delete_local_ref(env, activity);

        // The storage
        match Storage::open_title(None, None) {
            Err(e) => t(format!("title storage: error '{}'", e.message())),
            Ok(title) => {
                t(format!("title storage: ready {}", yes(title.ready())));
                t(format!(
                    "title size hello.txt: {}",
                    shown(title.file_size("hello.txt"), |s| s.to_string())
                ));
                t(format!(
                    "title read hello.txt: {}",
                    shown(title.load_file("hello.txt"), |d| format!(
                        "\"{}\"",
                        String::from_utf8_lossy(&d).trim_end_matches('\n')
                    ))
                ));
                t(format!(
                    "title info dir: {}",
                    shown(title.path_info("dir"), |i| format!(
                        "type {}",
                        if i.kind == PathType::Directory { 2 } else { 1 }
                    ))
                ));
                for p in [None, Some("dir")] {
                    let mut seen = Vec::new();
                    let r = title.enumerate_directory(p, |dir, name| {
                        seen.push(format!("[{dir}|{name}]"));
                        Ok(ControlFlow::Continue(()))
                    });
                    seen.sort();
                    t(format!(
                        "title enumerate {}: {}",
                        p.unwrap_or("(null)"),
                        shown(r, |()| seen.join(" "))
                    ));
                }
                for pattern in [Some("*.txt"), None] {
                    t(format!(
                        "title glob {}: {}",
                        pattern.unwrap_or("(null)"),
                        list(title.glob_directory(None, pattern, GlobFlags::NONE))
                    ));
                }
                t(format!(
                    "title write: {}",
                    ok(title.write_file("x.txt", b"x"))
                ));
                t(format!(
                    "title space: {}",
                    shown(title.space_remaining(), |s| s.to_string())
                ));
                t(format!("title close: {}", ok(title.close())));
            }
        }
        match Storage::open_user(Some("SDL3 Test"), "Backends", None) {
            Err(e) => t(format!("user storage: error '{}'", e.message())),
            Ok(user) => {
                t(format!("user storage: ready {}", yes(user.ready())));
                t(format!(
                    "user write save.txt: {}",
                    ok(user.write_file("save.txt", b"1"))
                ));
                t(format!(
                    "user read save.txt: {}",
                    shown(user.load_file("save.txt"), |d| format!(
                        "\"{}\"",
                        String::from_utf8_lossy(&d)
                    ))
                ));
                let pref =
                    filesystem::get_pref_path(Some("SDL3 Test"), "Backends").unwrap_or_default();
                t(format!(
                    "the pref path's save.txt: {}",
                    info(&format!("{pref}save.txt"))
                ));
                let mut seen = Vec::new();
                let r = user.enumerate_directory(None, |dir, name| {
                    seen.push(format!("[{dir}|{name}]"));
                    Ok(ControlFlow::Continue(()))
                });
                seen.sort();
                t(format!(
                    "user enumerate (null): {}",
                    shown(r, |()| seen.join(" "))
                ));
                t(format!(
                    "user space: {}",
                    shown(user.space_remaining(), |s| s.to_string())
                ));
                t(format!(
                    "user remove save.txt: {}",
                    ok(user.remove_path("save.txt"))
                ));
                t(format!("user close: {}", ok(user.close())));
            }
        }
        match Storage::open_file(None) {
            Err(e) => t(format!("file storage: error '{}'", e.message())),
            Ok(file) => {
                t(format!(
                    "file storage info hello.txt: {}",
                    shown(file.path_info("hello.txt"), |i| format!(
                        "type {} size {}",
                        if i.kind == PathType::Directory { 2 } else { 1 },
                        i.size
                    ))
                ));
                t(format!("file storage close: {}", ok(file.close())));
            }
        }

        // A toast
        t(format!(
            "toast: {}",
            ok(android::show_toast("SDL3 toast test", 1, -1, 0, 0))
        ));
        ask("toast");
        t(format!(
            "toast centered: {}",
            ok(android::show_toast(
                "SDL3 toast test, centered",
                0,
                17,
                0,
                100
            ))
        ));

        // URLs (an http one leaves the app for the activity that views it:
        // no pumping meanwhile, which would wait there while the app is
        // paused)
        t(format!(
            "open sdl3-backends-test://hello: {}",
            ok(misc::open_url("sdl3-backends-test://hello"))
        ));
        let opened = misc::open_url("https://www.libsdl.org/");
        let left = opened.is_ok();
        t(format!("open https://www.libsdl.org/: {}", ok(opened)));
        if left {
            ask("top-activity other");
            // (the app launched again until it stays on top: the browser's
            // first start can put an activity of its own over the app's
            // after the app's launch; the C program launches it once, its
            // trace the same as this one's when it stays)
            let mut top = Vec::new();
            for _ in 0..5 {
                request("foreground");
                top = request("top-activity app");
                if top.iter().any(|line| line == "top: the app") {
                    std::thread::sleep(Duration::from_secs(2));
                    top = request("top-activity app");
                    if top.iter().any(|line| line == "top: the app") {
                        break;
                    }
                }
            }
            for line in top {
                t(format!("reply: {line}"));
            }
        }
        pump();

        // Permissions
        ask("permission revoke READ_CONTACTS");
        permission(
            "android.permission.READ_CONTACTS",
            Some("permission-dialog deny"),
        );
        ask("permission grant READ_CONTACTS");
        permission("android.permission.READ_CONTACTS", None);
        permission(
            "android.permission.READ_CALENDAR",
            Some("permission-dialog allow"),
        );
        permission("android.permission.READ_SMS", None);

        t("main returns");
        init::quit();
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
