// Tests of the UIKit video driver's clipboard: the general pasteboard's
// text and its changes.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The UIKit video driver only starts in an app, so this is a program of
//! its own (`harness = false`), run as an app on the iOS Simulator by
//! `tools/ios_run.py` (the `__DATA,__sdl3_runner` section below), as
//! `uikit_main` is. Its main function empties the general pasteboard
//! (what another app put there would ask to be pasted), then traces the
//! clipboard (whether it has text, its text and mime types), the general
//! `UIPasteboard` itself (whether it has strings, its string, how many
//! times it changed) and the clipboard's events, through:
//!
//! * the clipboard's text set before the video subsystem starts;
//! * text set and read back: ASCII, characters outside ASCII and outside
//!   the Basic Multilingual Plane, control characters and quotes, the
//!   same text again, and empty text (twice);
//! * the pasteboard changed in the program through `UIPasteboard` (a
//!   string, an empty one, no items, two strings), which sends clipboard
//!   updates the app doesn't own and drops its clipboard data, and
//!   `UIPasteboardChangedNotification` posted with nothing changed;
//! * clipboard data (`sdl3::video::clipboard::set_clipboard_data()`):
//!   with text among its mime types (which goes to the pasteboard, asked
//!   of the callback, whose calls are traced) and without, read back by
//!   mime type, replaced by a change through `UIPasteboard`, cleared, and
//!   given bad parameters;
//! * the text left in the pasteboard when SDL quits, the pasteboard
//!   changed while SDL isn't running (nothing follows it then), and the
//!   video subsystem started again.
//!
//! Only the clipboard's events are traced (each step's, the program
//! pumping for half a second). The pasteboard posts its change
//! notification at once, from the setter: while the driver sets it (so
//! that SDL's own text sends no update it doesn't own), and twice for a
//! string set or the items emptied through `UIPasteboard`. The expected trace is upstream's C SDL's,
//! from the same program in C run the same way on the same simulator (an
//! iPhone 16 with iOS 18.5). Elsewhere this program does nothing.

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
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use sdl3::events::{self, Event};
    use sdl3::init::{self, InitFlags};
    use sdl3::video::{self, clipboard};

    use super::uikit_support::objc::{self, Id};
    use super::uikit_support::{check_trace, checked, elapsed, exit};

    /// Upstream's trace (the C program, on an iPhone 16 simulator with
    /// iOS 18.5).
    const EXPECTED: &str = "\
start: pasteboard: has strings=no
before init: set: error: Video subsystem has not been initialized
before init: get: \"\"; has: no
init video: ok
video driver: uikit
start: events: (none)
start: has text=no text=\"\"
start: mime types=[] pasteboard: has strings=no string=nil change count=+0
ascii: set \"Hello, clipboard!\": ok
ascii: events: CLIPBOARD_UPDATE(owner=1 mime_types=[text/plain;charset=utf-8])
ascii: has text=yes text=\"Hello, clipboard!\"
ascii: mime types=[text/plain;charset=utf-8] pasteboard: has strings=yes string=\"Hello, clipboard!\" change count=+1
unicode: set \"Grüße, 世界 🎮\": ok
unicode: events: CLIPBOARD_UPDATE(owner=1 mime_types=[text/plain;charset=utf-8])
unicode: has text=yes text=\"Grüße, 世界 🎮\"
unicode: mime types=[text/plain;charset=utf-8] pasteboard: has strings=yes string=\"Grüße, 世界 🎮\" change count=+2
again: set \"Hello, clipboard!\": ok
again: events: CLIPBOARD_UPDATE(owner=1 mime_types=[text/plain;charset=utf-8])
again: has text=yes text=\"Hello, clipboard!\"
again: mime types=[text/plain;charset=utf-8] pasteboard: has strings=yes string=\"Hello, clipboard!\" change count=+3
control: set \"tab\\x09here\\x0aline \\x22quoted\\x22 back\\x5cslash\": ok
control: events: CLIPBOARD_UPDATE(owner=1 mime_types=[text/plain;charset=utf-8])
control: has text=yes text=\"tab\\x09here\\x0aline \\x22quoted\\x22 back\\x5cslash\"
control: mime types=[text/plain;charset=utf-8] pasteboard: has strings=yes string=\"tab\\x09here\\x0aline \\x22quoted\\x22 back\\x5cslash\" change count=+4
empty: set \"\": ok
empty: events: CLIPBOARD_UPDATE(owner=1 mime_types=[])
empty: has text=no text=\"\"
empty: mime types=[] pasteboard: has strings=no string=nil change count=+5
empty again: set \"\": ok
empty again: events: CLIPBOARD_UPDATE(owner=1 mime_types=[])
empty again: has text=no text=\"\"
empty again: mime types=[] pasteboard: has strings=no string=nil change count=+6
pasteboard: string set
pasteboard string: events: CLIPBOARD_UPDATE(owner=0 mime_types=[]), CLIPBOARD_UPDATE(owner=0 mime_types=[])
pasteboard string: has text=yes text=\"set through UIPasteboard\"
pasteboard string: mime types=[] pasteboard: has strings=yes string=\"set through UIPasteboard\" change count=+7
sdl after pasteboard: set \"SDL's again\": ok
sdl after pasteboard: events: CLIPBOARD_UPDATE(owner=1 mime_types=[text/plain;charset=utf-8])
sdl after pasteboard: has text=yes text=\"SDL's again\"
sdl after pasteboard: mime types=[text/plain;charset=utf-8] pasteboard: has strings=yes string=\"SDL's again\" change count=+8
pasteboard: empty string set
pasteboard empty string: events: CLIPBOARD_UPDATE(owner=0 mime_types=[])
pasteboard empty string: has text=yes text=\"\"
pasteboard empty string: mime types=[] pasteboard: has strings=yes string=\"\" change count=+9
pasteboard: items emptied
pasteboard emptied: events: CLIPBOARD_UPDATE(owner=0 mime_types=[]), CLIPBOARD_UPDATE(owner=0 mime_types=[])
pasteboard emptied: has text=no text=\"\"
pasteboard emptied: mime types=[] pasteboard: has strings=no string=nil change count=+10
pasteboard: two strings set
pasteboard strings: events: CLIPBOARD_UPDATE(owner=0 mime_types=[]), CLIPBOARD_UPDATE(owner=0 mime_types=[])
pasteboard strings: has text=yes text=\"first\"
pasteboard strings: mime types=[] pasteboard: has strings=yes string=\"first\" change count=+11
before post: set \"posted over\": ok
before post: events: CLIPBOARD_UPDATE(owner=1 mime_types=[text/plain;charset=utf-8])
before post: has text=yes text=\"posted over\"
before post: mime types=[text/plain;charset=utf-8] pasteboard: has strings=yes string=\"posted over\" change count=+12
posted: UIPasteboardChangedNotification
posted: events: CLIPBOARD_UPDATE(owner=0 mime_types=[])
posted: has text=yes text=\"posted over\"
posted: mime types=[] pasteboard: has strings=yes string=\"posted over\" change count=+12
posted: UIPasteboardChangedNotification (no object)
posted without object: events: CLIPBOARD_UPDATE(owner=0 mime_types=[])
posted without object: has text=yes text=\"posted over\"
posted without object: mime types=[] pasteboard: has strings=yes string=\"posted over\" change count=+12
callback: text/plain;charset=utf-8
data: set: ok
data: events: CLIPBOARD_UPDATE(owner=1 mime_types=[image/png, text/plain;charset=utf-8, text/plain])
data: has text=yes text=\"text from the callback\"
data: mime types=[image/png, text/plain;charset=utf-8, text/plain] pasteboard: has strings=yes string=\"text from the callback\" change count=+13
callback: image/png
data: data image/png: has=yes size=4 data=89 50 4e 47
data: data text/plain;charset=utf-8: has=yes size=22 data=\"text from the callback\"
data: data text/plain: has=yes size=22 data=\"text from the callback\"
data: data text/html: has=yes size=22 data=\"text from the callback\"
data: data application/octet-stream: has=no size=0 data=none
png only: set: ok
png only: events: CLIPBOARD_UPDATE(owner=1 mime_types=[image/png])
png only: has text=no text=\"\"
png only: mime types=[image/png] pasteboard: has strings=no string=nil change count=+14
callback: image/png
png only: data image/png: has=yes size=4 data=89 50 4e 47
png only: data text/plain;charset=utf-8: has=no size=0 data=none
pasteboard: string set over the app's data
over data: events: CLIPBOARD_UPDATE(owner=0 mime_types=[]), CLIPBOARD_UPDATE(owner=0 mime_types=[])
over data: has text=yes text=\"over the app's data\"
over data: mime types=[] pasteboard: has strings=yes string=\"over the app's data\" change count=+15
over data: data image/png: has=no size=0 data=none
callback: text/plain;charset=utf-8
data: set again: ok
data again: events: CLIPBOARD_UPDATE(owner=1 mime_types=[image/png, text/plain;charset=utf-8, text/plain])
clear: ok
clear: events: CLIPBOARD_UPDATE(owner=1 mime_types=[])
clear: has text=no text=\"\"
clear: mime types=[] pasteboard: has strings=no string=nil change count=+17
invalid: error: Invalid parameters
invalid: error: Invalid parameters
before quit: set \"kept after quit\": ok
before quit: events: CLIPBOARD_UPDATE(owner=1 mime_types=[text/plain;charset=utf-8])
before quit: has text=yes text=\"kept after quit\"
before quit: mime types=[text/plain;charset=utf-8] pasteboard: has strings=yes string=\"kept after quit\" change count=+18
quit
after quit: pasteboard string=\"kept after quit\"
after quit: pasteboard string set
again: init video: ok
again: events: (none)
again: has text=yes text=\"changed after quit\"
again: mime types=[] pasteboard: has strings=yes string=\"changed after quit\" change count=+19
again: set \"set again\": ok
again: events: CLIPBOARD_UPDATE(owner=1 mime_types=[text/plain;charset=utf-8])
again: has text=yes text=\"set again\"
again: mime types=[text/plain;charset=utf-8] pasteboard: has strings=yes string=\"set again\" change count=+20
quit again
after quit again: pasteboard string=\"set again\"
done
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

    /// A text as the trace quotes it: bytes below 0x20, 0x7f, quotes and
    /// backslashes escaped.
    fn quoted(text: &str) -> String {
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

    // ---- the general pasteboard

    fn pb() -> Id {
        // SAFETY: UIPasteboard's class property.
        unsafe { objc::send0(objc::class("UIPasteboard"), "generalPasteboard") }
    }

    fn pb_string() -> Option<String> {
        // SAFETY: a UIPasteboard property, an NSString or nil.
        objc::string(unsafe { objc::send0(pb(), "string") })
    }

    fn pb_has_strings() -> bool {
        // SAFETY: a UIPasteboard property.
        unsafe { objc::send0(pb(), "hasStrings") }
    }

    fn pb_change_count() -> isize {
        // SAFETY: a UIPasteboard property.
        unsafe { objc::send0(pb(), "changeCount") }
    }

    /// An `NSArray` of these objects.
    fn ns_array(objects: &[Id]) -> Id {
        // SAFETY: NSArray's class method, with the objects' count.
        unsafe {
            objc::send2(
                objc::class("NSArray"),
                "arrayWithObjects:count:",
                objects.as_ptr(),
                objects.len(),
            )
        }
    }

    fn pb_set_string(s: &str) {
        // SAFETY: a UIPasteboard property, an NSString.
        unsafe {
            let () = objc::send1(pb(), "setString:", objc::ns_string(s));
        }
    }

    fn pb_set_strings(strings: &[&str]) {
        let strings: Vec<Id> = strings.iter().map(|s| objc::ns_string(s)).collect();
        // SAFETY: a UIPasteboard property, an NSArray of NSStrings.
        unsafe {
            let () = objc::send1(pb(), "setStrings:", ns_array(&strings));
        }
    }

    fn pb_empty() {
        // SAFETY: a UIPasteboard property, an empty NSArray.
        unsafe {
            let () = objc::send1(pb(), "setItems:", ns_array(&[]));
        }
    }

    fn pb_string_shown() -> String {
        pb_string().map_or_else(|| "nil".to_string(), |s| quoted(&s))
    }

    #[link(name = "UIKit", kind = "framework")]
    unsafe extern "C" {
        static UIPasteboardChangedNotification: Id;
    }

    static START_COUNT: Mutex<isize> = Mutex::new(0);

    // ---- the clipboard

    /// Pump for `ms` milliseconds (10 ms steps): the clipboard's events.
    fn events(label: &str, ms: u64) {
        let mut seen = Vec::new();
        let take = |seen: &mut Vec<String>| {
            events::pump();
            while let Some(event) = events::poll() {
                if let Event::Clipboard(e) = event {
                    seen.push(format!(
                        "CLIPBOARD_UPDATE(owner={} mime_types=[{}])",
                        e.owner as i32,
                        e.mime_types.join(", ")
                    ));
                }
            }
        };
        take(&mut seen);
        for _ in 0..ms / 10 {
            std::thread::sleep(Duration::from_millis(10));
            take(&mut seen);
        }
        if seen.is_empty() {
            t(format!("{label}: events: (none)"));
        } else {
            t(format!("{label}: events: {}", seen.join(", ")));
        }
    }

    fn state(label: &str) {
        let text = clipboard::clipboard_text().unwrap_or_default();
        t(format!(
            "{label}: has text={} text={}",
            yes_no(clipboard::has_clipboard_text().unwrap_or(false)),
            quoted(&text)
        ));
        let types = clipboard::clipboard_mime_types().unwrap_or_default();
        t(format!(
            "{label}: mime types=[{}] pasteboard: has strings={} string={} change count=+{}",
            types.join(", "),
            yes_no(pb_has_strings()),
            pb_string_shown(),
            pb_change_count() - *START_COUNT.lock().unwrap()
        ));
    }

    fn set_text(label: &str, text: &str) {
        t(format!(
            "{label}: set {}: {}",
            quoted(text),
            ok(&clipboard::set_clipboard_text(text))
        ));
        events(label, 500);
        state(label);
    }

    const CB_TEXT: &str = "text from the callback";

    /// The clipboard data's callback (each call traced).
    fn data_cb() -> clipboard::ClipboardDataCallback {
        Arc::new(|mime_type: &str| {
            t(format!("callback: {mime_type}"));
            if mime_type == "image/png" {
                return Some(b"\x89PNG".to_vec());
            }
            if mime_type.starts_with("text") {
                return Some(CB_TEXT.as_bytes().to_vec());
            }
            None
        })
    }

    fn data_state(label: &str, mime_type: &str) {
        let data = clipboard::clipboard_data(mime_type).ok().flatten();
        let size = data.as_ref().map_or(0, Vec::len);
        let shown = match &data {
            None => "none".to_string(),
            Some(d) if mime_type.starts_with("text") => quoted(&String::from_utf8_lossy(d)),
            Some(d) => d
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<Vec<_>>()
                .join(" "),
        };
        t(format!(
            "{label}: data {mime_type}: has={} size={size} data={shown}",
            yes_no(clipboard::has_clipboard_data(mime_type).unwrap_or(false))
        ));
    }

    fn application_state() -> isize {
        // SAFETY: a UIApplication property.
        unsafe { objc::send0(objc::shared_application(), "applicationState") }
    }

    fn post_changed(object: Id) {
        // SAFETY: UIKit's constant string.
        objc::post_notification_for(unsafe { UIPasteboardChangedNotification }, object);
    }

    pub(super) fn main() {
        elapsed();
        let status = sdl3::app::run_app(|_args| {
            let status = checked(main_function);
            exit(status);
        });
        println!("uikit_clipboard: run_app returned {status}, which it never does");
        exit(1);
    }

    fn main_function() {
        println!("uikit_clipboard: {:.3} s: main", elapsed());
        // The pasteboard emptied first (by the program: what another app
        // put there would ask to be pasted)
        pb_empty();
        *START_COUNT.lock().unwrap() = pb_change_count();
        t(format!(
            "start: pasteboard: has strings={}",
            yes_no(pb_has_strings())
        ));
        t(format!(
            "before init: set: {}",
            ok(&clipboard::set_clipboard_text("early"))
        ));
        t(format!(
            "before init: get: {}; has: {}",
            quoted(&clipboard::clipboard_text().unwrap_or_default()),
            yes_no(clipboard::has_clipboard_text().unwrap_or(false))
        ));
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
        events("start", 500);
        state("start");

        set_text("ascii", "Hello, clipboard!");
        set_text("unicode", "Grüße, 世界 🎮");
        set_text("again", "Hello, clipboard!");
        set_text("control", "tab\there\nline \"quoted\" back\\slash");
        set_text("empty", "");
        set_text("empty again", "");

        // Changes through UIPasteboard (not SDL's)
        pb_set_string("set through UIPasteboard");
        t("pasteboard: string set");
        events("pasteboard string", 500);
        state("pasteboard string");
        set_text("sdl after pasteboard", "SDL's again");
        pb_set_string("");
        t("pasteboard: empty string set");
        events("pasteboard empty string", 500);
        state("pasteboard empty string");
        pb_empty();
        t("pasteboard: items emptied");
        events("pasteboard emptied", 500);
        state("pasteboard emptied");
        pb_set_strings(&["first", "second"]);
        t("pasteboard: two strings set");
        events("pasteboard strings", 500);
        state("pasteboard strings");

        // The notification posted (nothing changed)
        set_text("before post", "posted over");
        post_changed(pb());
        t("posted: UIPasteboardChangedNotification");
        events("posted", 500);
        state("posted");
        post_changed(std::ptr::null_mut());
        t("posted: UIPasteboardChangedNotification (no object)");
        events("posted without object", 500);
        state("posted without object");

        // Clipboard data: the text goes to the pasteboard, the rest stays
        // the app's
        let types = ["image/png", "text/plain;charset=utf-8", "text/plain"];
        t(format!(
            "data: set: {}",
            ok(&clipboard::set_clipboard_data(Some(data_cb()), &types))
        ));
        events("data", 500);
        state("data");
        data_state("data", "image/png");
        data_state("data", "text/plain;charset=utf-8");
        data_state("data", "text/plain");
        data_state("data", "text/html");
        data_state("data", "application/octet-stream");
        t(format!(
            "png only: set: {}",
            ok(&clipboard::set_clipboard_data(
                Some(data_cb()),
                &["image/png"]
            ))
        ));
        events("png only", 500);
        state("png only");
        data_state("png only", "image/png");
        data_state("png only", "text/plain;charset=utf-8");
        pb_set_string("over the app's data");
        t("pasteboard: string set over the app's data");
        events("over data", 500);
        state("over data");
        data_state("over data", "image/png");
        t(format!(
            "data: set again: {}",
            ok(&clipboard::set_clipboard_data(Some(data_cb()), &types))
        ));
        events("data again", 500);
        t(format!("clear: {}", ok(&clipboard::clear_clipboard_data())));
        events("clear", 500);
        state("clear");
        t(format!(
            "invalid: {}",
            ok(&clipboard::set_clipboard_data(Some(data_cb()), &[]))
        ));
        t(format!(
            "invalid: {}",
            ok(&clipboard::set_clipboard_data(None, &types[..1]))
        ));

        // What's left after quitting
        set_text("before quit", "kept after quit");
        init::quit();
        t("quit");
        t(format!(
            "after quit: pasteboard string={}",
            pb_string_shown()
        ));
        pb_set_string("changed after quit");
        t("after quit: pasteboard string set");
        t(format!(
            "again: init video: {}",
            ok(&init::init(InitFlags::VIDEO))
        ));
        events("again", 500);
        state("again");
        set_text("again", "set again");
        init::quit();
        t("quit again");
        t(format!(
            "after quit again: pasteboard string={}",
            pb_string_shown()
        ));
        pb_empty();
        t("done");

        let trace = TRACE.lock().unwrap().clone();
        check_trace(&trace, EXPECTED);
    }
}
