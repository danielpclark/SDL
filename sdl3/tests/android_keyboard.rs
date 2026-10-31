// Tests of the android video driver's keyboard: keys, text input, the
// screen keyboard and the input method's connection.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! An app (`harness = false`), run by `tools/android_run.py`, whose main
//! function makes a window and traces the keyboard's events (keys, text,
//! the screen keyboard shown and hidden) and the state of text input,
//! from:
//!
//! * real keys, through the runner's `input` requests (`adb shell input
//!   keyevent`, `keycombination` and `text`), with text input off and on:
//!   letters, a digit, a long press, the modifiers, caps lock, the
//!   arrows, Enter, Tab, Space, the deletions, Escape, a function key, the
//!   numeric keypad, a key without a scancode, one Android keeps (zoom), a
//!   game controller's button (of the virtual keyboard, so not a
//!   joystick's), the mouse's forward key, and BACK (the keyboard's and
//!   the mouse's) with the back button's trap on
//!   (`SDL_HINT_ANDROID_TRAP_BACK_BUTTON`: its `OnBackInvokedCallback`'s
//!   key), and with text input on too (the keyboard's focus lost). (BACK
//!   untrapped, the mouse's too, finishes the activity: from API 33, with
//!   the manifest's `enableOnBackInvokedCallback`, the views don't get it,
//!   upstream's C as here);
//! * the screen keyboard, shown and hidden by `start_text_input()` and
//!   `stop_text_input()` (with the input method's keyboard shown though
//!   the virtual device has a hardware keyboard: the runner's
//!   `ime-with-hard-keyboard yes`), each time traced as SDL has it
//!   (`text_input_active()`, `screen_keyboard_shown()`), as the input
//!   method has it (the runner's `ime` request: `dumpsys input_method`'s
//!   mInputShown), as the window's insets have it
//!   (`WindowInsets.isVisible(Type.ime())`), and by the focused view (the
//!   surface, or the text edit, with its place over the text input area);
//!   restored with the surface (the text input area moved, then the
//!   display turned and back: the text edit follows the area); and after
//!   a trip to the background and back (hidden with the app, so not
//!   restored; the pump doesn't block while the app is paused,
//!   `SDL_HINT_ANDROID_BLOCK_ON_PAUSE` off);
//! * what an input method does (commits, compositions, deletions, key
//!   events), done in the program: the text edit's input connection
//!   (`SDLDummyEdit.onCreateInputConnection()`, of a new `EditorInfo`),
//!   called on SDL's thread (`commitText()`, `setComposingText()`,
//!   `finishComposingText()`, `deleteSurroundingText()`, `sendKeyEvent()`),
//!   each with its result, its events (`TEXT_INPUT`, the keys it presses;
//!   upstream sends no `TEXT_EDITING` on Android) and the connection's
//!   text (`getEditable()`, `mCommittedText`);
//! * the return key with `SDL_HINT_RETURN_KEY_HIDES_IME` off and on (a
//!   newline committed, the return key's event sent, the real key);
//! * the text input properties: each `TextInputType`, capitalization,
//!   autocorrection, multiline, the hint's default, and
//!   `PROP_TEXTINPUT_ANDROID_INPUTTYPE_NUMBER`, as the input type the text
//!   edit gives an input method (`EditorInfo.inputType`), with the input
//!   method's keyboard off (`ime-with-hard-keyboard no`: its animations
//!   would make SDL's shown and hidden a matter of timing).
//!
//! The expected trace is upstream's C SDL's, from the same program in C
//! (built with upstream's android-project, Gradle and CMake, run by the
//! runner's `--run-apk`) on the same emulator (API 35, Google APIs, x86-64,
//! a Pixel 6's screen with a hardware keyboard, cold booted). Elsewhere
//! this program does nothing.

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
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    use sdl3::android;
    use sdl3::events::keyboard;
    use sdl3::events::window::WindowFlags;
    use sdl3::events::{self, Event, EventType, WindowID};
    use sdl3::hints;
    use sdl3::init::{self, InitFlags};
    use sdl3::properties::Properties;
    use sdl3::video::textinput::{self, Capitalization, TextInputType};
    use sdl3::video::{self, Rect, Window};

    use super::android_support::jni::{self, JNIEnv, Jobject, Jvalue};
    use super::android_support::{check_trace, event_name, request};

    /// Upstream's trace (the C program, on the emulator: API 35, Google
    /// APIs, x86-64).
    const EXPECTED: &str = "\
init: ok
video driver: android
screen keyboard support: yes
start: keyboards=0 has keyboard=no mod state=0x0
create: ok
created: events: (none)
created: text input active=no screen keyboard shown=no
logcat: ime: mInputShown=false
created: insets: ime visible=no
created: focused view: org.libsdl.app.SDLSurface
created: keyboards=0 has keyboard=no mod state=0x0
a: event: KEY_DOWN(which=1 scancode=4 key=0x61 mod=0x0 raw=29 repeat=0 w=W)
a: event: KEY_UP(which=1 scancode=4 key=0x61 mod=0x0 raw=29 repeat=0 w=W)
5: event: KEY_DOWN(which=1 scancode=34 key=0x35 mod=0x0 raw=12 repeat=0 w=W)
5: event: KEY_UP(which=1 scancode=34 key=0x35 mod=0x0 raw=12 repeat=0 w=W)
long press a: event: KEY_DOWN(which=1 scancode=4 key=0x61 mod=0x0 raw=29 repeat=0 w=W)
long press a: event: KEY_DOWN(which=1 scancode=4 key=0x61 mod=0x0 raw=29 repeat=1 w=W)
long press a: event: KEY_UP(which=1 scancode=4 key=0x61 mod=0x0 raw=29 repeat=0 w=W)
shift b: event: KEY_DOWN(which=1 scancode=225 key=0x400000e1 mod=0x1 raw=59 repeat=0 w=W)
shift b: event: KEY_DOWN(which=1 scancode=5 key=0x62 mod=0x1 raw=30 repeat=0 w=W)
shift b: event: KEY_UP(which=1 scancode=225 key=0x400000e1 mod=0x0 raw=59 repeat=0 w=W)
shift b: event: KEY_UP(which=1 scancode=5 key=0x62 mod=0x0 raw=30 repeat=0 w=W)
ctrl c: event: KEY_DOWN(which=1 scancode=224 key=0x400000e0 mod=0x40 raw=113 repeat=0 w=W)
ctrl c: event: KEY_DOWN(which=1 scancode=6 key=0x63 mod=0x40 raw=31 repeat=0 w=W)
ctrl c: event: KEY_UP(which=1 scancode=224 key=0x400000e0 mod=0x0 raw=113 repeat=0 w=W)
ctrl c: event: KEY_UP(which=1 scancode=6 key=0x63 mod=0x0 raw=31 repeat=0 w=W)
alt x: event: KEY_DOWN(which=1 scancode=226 key=0x400000e2 mod=0x100 raw=57 repeat=0 w=W)
alt x: event: KEY_DOWN(which=1 scancode=27 key=0x78 mod=0x100 raw=52 repeat=0 w=W)
alt x: event: KEY_UP(which=1 scancode=226 key=0x400000e2 mod=0x0 raw=57 repeat=0 w=W)
alt x: event: KEY_UP(which=1 scancode=27 key=0x78 mod=0x0 raw=52 repeat=0 w=W)
shift right: event: KEY_DOWN(which=1 scancode=229 key=0x400000e5 mod=0x2 raw=60 repeat=0 w=W)
shift right: event: KEY_UP(which=1 scancode=229 key=0x400000e5 mod=0x0 raw=60 repeat=0 w=W)
caps lock: event: KEY_DOWN(which=1 scancode=57 key=0x40000039 mod=0x2000 raw=115 repeat=0 w=W)
caps lock: event: KEY_UP(which=1 scancode=57 key=0x40000039 mod=0x2000 raw=115 repeat=0 w=W)
caps lock: keyboards=0 has keyboard=no mod state=0x2000
caps lock a: event: KEY_DOWN(which=1 scancode=4 key=0x61 mod=0x2000 raw=29 repeat=0 w=W)
caps lock a: event: KEY_UP(which=1 scancode=4 key=0x61 mod=0x2000 raw=29 repeat=0 w=W)
caps lock again: event: KEY_DOWN(which=1 scancode=57 key=0x40000039 mod=0x0 raw=115 repeat=0 w=W)
caps lock again: event: KEY_UP(which=1 scancode=57 key=0x40000039 mod=0x0 raw=115 repeat=0 w=W)
caps lock again: keyboards=0 has keyboard=no mod state=0x0
left: event: KEY_DOWN(which=1 scancode=80 key=0x40000050 mod=0x0 raw=21 repeat=0 w=W)
left: event: KEY_UP(which=1 scancode=80 key=0x40000050 mod=0x0 raw=21 repeat=0 w=W)
right: event: KEY_DOWN(which=1 scancode=79 key=0x4000004f mod=0x0 raw=22 repeat=0 w=W)
right: event: KEY_UP(which=1 scancode=79 key=0x4000004f mod=0x0 raw=22 repeat=0 w=W)
up: event: KEY_DOWN(which=1 scancode=82 key=0x40000052 mod=0x0 raw=19 repeat=0 w=W)
up: event: KEY_UP(which=1 scancode=82 key=0x40000052 mod=0x0 raw=19 repeat=0 w=W)
down: event: KEY_DOWN(which=1 scancode=81 key=0x40000051 mod=0x0 raw=20 repeat=0 w=W)
down: event: KEY_UP(which=1 scancode=81 key=0x40000051 mod=0x0 raw=20 repeat=0 w=W)
dpad center: event: KEY_DOWN(which=1 scancode=40 key=0xd mod=0x0 raw=23 repeat=0 w=W)
dpad center: event: KEY_UP(which=1 scancode=40 key=0xd mod=0x0 raw=23 repeat=0 w=W)
enter: event: KEY_DOWN(which=1 scancode=40 key=0xd mod=0x0 raw=66 repeat=0 w=W)
enter: event: KEY_UP(which=1 scancode=40 key=0xd mod=0x0 raw=66 repeat=0 w=W)
numpad enter: event: KEY_DOWN(which=1 scancode=88 key=0x40000058 mod=0x0 raw=160 repeat=0 w=W)
numpad enter: event: KEY_UP(which=1 scancode=88 key=0x40000058 mod=0x0 raw=160 repeat=0 w=W)
tab: event: KEY_DOWN(which=1 scancode=43 key=0x9 mod=0x0 raw=61 repeat=0 w=W)
tab: event: KEY_UP(which=1 scancode=43 key=0x9 mod=0x0 raw=61 repeat=0 w=W)
space: event: KEY_DOWN(which=1 scancode=44 key=0x20 mod=0x0 raw=62 repeat=0 w=W)
space: event: KEY_UP(which=1 scancode=44 key=0x20 mod=0x0 raw=62 repeat=0 w=W)
del: event: KEY_DOWN(which=1 scancode=42 key=0x8 mod=0x0 raw=67 repeat=0 w=W)
del: event: KEY_UP(which=1 scancode=42 key=0x8 mod=0x0 raw=67 repeat=0 w=W)
forward del: event: KEY_DOWN(which=1 scancode=76 key=0x7f mod=0x0 raw=112 repeat=0 w=W)
forward del: event: KEY_UP(which=1 scancode=76 key=0x7f mod=0x0 raw=112 repeat=0 w=W)
escape: event: KEY_DOWN(which=1 scancode=41 key=0x1b mod=0x0 raw=111 repeat=0 w=W)
escape: event: KEY_UP(which=1 scancode=41 key=0x1b mod=0x0 raw=111 repeat=0 w=W)
f1: event: KEY_DOWN(which=1 scancode=58 key=0x4000003a mod=0x0 raw=131 repeat=0 w=W)
f1: event: KEY_UP(which=1 scancode=58 key=0x4000003a mod=0x0 raw=131 repeat=0 w=W)
numpad 5: event: KEY_DOWN(which=1 scancode=93 key=0x4000005d mod=0x0 raw=149 repeat=0 w=W)
numpad 5: event: KEY_UP(which=1 scancode=93 key=0x4000005d mod=0x0 raw=149 repeat=0 w=W)
page down: event: KEY_DOWN(which=1 scancode=78 key=0x4000004e mod=0x0 raw=93 repeat=0 w=W)
page down: event: KEY_UP(which=1 scancode=78 key=0x4000004e mod=0x0 raw=93 repeat=0 w=W)
star (no scancode): event: KEY_DOWN(which=1 scancode=0 key=0x0 mod=0x0 raw=17 repeat=0 w=W)
star (no scancode): event: KEY_UP(which=1 scancode=0 key=0x0 mod=0x0 raw=17 repeat=0 w=W)
zoom in (Android's): events: (none)
button a: event: KEY_DOWN(which=1 scancode=0 key=0x0 mod=0x0 raw=96 repeat=0 w=W)
button a: event: KEY_UP(which=1 scancode=0 key=0x0 mod=0x0 raw=96 repeat=0 w=W)
gamepad button a: event: KEY_DOWN(which=1 scancode=0 key=0x0 mod=0x0 raw=96 repeat=0 w=W)
gamepad button a: event: KEY_UP(which=1 scancode=0 key=0x0 mod=0x0 raw=96 repeat=0 w=W)
mouse forward: events: (none)
back, trapped: event: KEY_DOWN(which=1 scancode=282 key=0x4000011a mod=0x0 raw=4 repeat=0 w=W)
back, trapped: event: KEY_UP(which=1 scancode=282 key=0x4000011a mod=0x0 raw=4 repeat=0 w=W)
mouse back, trapped: event: KEY_DOWN(which=1 scancode=282 key=0x4000011a mod=0x0 raw=4 repeat=0 w=W)
mouse back, trapped: event: KEY_UP(which=1 scancode=282 key=0x4000011a mod=0x0 raw=4 repeat=0 w=W)
text: event: KEY_DOWN(which=1 scancode=225 key=0x400000e1 mod=0x1 raw=59 repeat=0 w=W)
text: event: KEY_DOWN(which=1 scancode=11 key=0x68 mod=0x1 raw=36 repeat=0 w=W)
text: event: KEY_UP(which=1 scancode=11 key=0x68 mod=0x1 raw=36 repeat=0 w=W)
text: event: KEY_UP(which=1 scancode=225 key=0x400000e1 mod=0x0 raw=59 repeat=0 w=W)
text: event: KEY_DOWN(which=1 scancode=12 key=0x69 mod=0x0 raw=37 repeat=0 w=W)
text: event: KEY_UP(which=1 scancode=12 key=0x69 mod=0x0 raw=37 repeat=0 w=W)
text: event: KEY_DOWN(which=1 scancode=225 key=0x400000e1 mod=0x1 raw=59 repeat=0 w=W)
text: event: KEY_DOWN(which=1 scancode=30 key=0x31 mod=0x1 raw=8 repeat=0 w=W)
text: event: KEY_UP(which=1 scancode=30 key=0x31 mod=0x1 raw=8 repeat=0 w=W)
text: event: KEY_UP(which=1 scancode=225 key=0x400000e1 mod=0x0 raw=59 repeat=0 w=W)
logcat: I/SDL: Unknown keycode 17
logcat: I/SDL: Unknown keycode 17
keys: keyboards=0 has keyboard=no mod state=0x0
text input area: ok
text input on: start text input: ok
text input on: event: SCREEN_KEYBOARD_SHOWN
text input on: text input active=yes screen keyboard shown=yes
logcat: ime: mInputShown=true
text input on: insets: ime visible=yes
text input on: focused view: org.libsdl.app.SDLDummyEdit
text input on: text edit: left=100 top=200 width=300 height=55 visibility=0
on: a: event: KEY_DOWN(which=1 scancode=4 key=0x61 mod=0x0 raw=29 repeat=0 w=W)
on: a: event: TEXT_INPUT(text=\"a\" w=W)
on: a: event: KEY_UP(which=1 scancode=4 key=0x61 mod=0x0 raw=29 repeat=0 w=W)
on: shift a: event: KEY_DOWN(which=1 scancode=225 key=0x400000e1 mod=0x1 raw=59 repeat=0 w=W)
on: shift a: event: KEY_DOWN(which=1 scancode=4 key=0x61 mod=0x1 raw=29 repeat=0 w=W)
on: shift a: event: TEXT_INPUT(text=\"A\" w=W)
on: shift a: event: KEY_UP(which=1 scancode=225 key=0x400000e1 mod=0x0 raw=59 repeat=0 w=W)
on: shift a: event: KEY_UP(which=1 scancode=4 key=0x61 mod=0x0 raw=29 repeat=0 w=W)
on: ctrl a: event: KEY_DOWN(which=1 scancode=224 key=0x400000e0 mod=0x40 raw=113 repeat=0 w=W)
on: ctrl a: event: KEY_DOWN(which=1 scancode=4 key=0x61 mod=0x40 raw=29 repeat=0 w=W)
on: ctrl a: event: KEY_UP(which=1 scancode=224 key=0x400000e0 mod=0x0 raw=113 repeat=0 w=W)
on: ctrl a: event: KEY_UP(which=1 scancode=4 key=0x61 mod=0x0 raw=29 repeat=0 w=W)
on: space: event: KEY_DOWN(which=1 scancode=44 key=0x20 mod=0x0 raw=62 repeat=0 w=W)
on: space: event: TEXT_INPUT(text=\" \" w=W)
on: space: event: KEY_UP(which=1 scancode=44 key=0x20 mod=0x0 raw=62 repeat=0 w=W)
on: enter: event: KEY_DOWN(which=1 scancode=40 key=0xd mod=0x0 raw=66 repeat=0 w=W)
on: enter: event: KEY_UP(which=1 scancode=40 key=0xd mod=0x0 raw=66 repeat=0 w=W)
on: del: event: KEY_DOWN(which=1 scancode=42 key=0x8 mod=0x0 raw=67 repeat=0 w=W)
on: del: event: KEY_UP(which=1 scancode=42 key=0x8 mod=0x0 raw=67 repeat=0 w=W)
on: text: event: KEY_DOWN(which=1 scancode=11 key=0x68 mod=0x0 raw=36 repeat=0 w=W)
on: text: event: TEXT_INPUT(text=\"h\" w=W)
on: text: event: KEY_UP(which=1 scancode=11 key=0x68 mod=0x0 raw=36 repeat=0 w=W)
on: text: event: KEY_DOWN(which=1 scancode=8 key=0x65 mod=0x0 raw=33 repeat=0 w=W)
on: text: event: TEXT_INPUT(text=\"e\" w=W)
on: text: event: KEY_UP(which=1 scancode=8 key=0x65 mod=0x0 raw=33 repeat=0 w=W)
on: text: event: KEY_DOWN(which=1 scancode=15 key=0x6c mod=0x0 raw=40 repeat=0 w=W)
on: text: event: TEXT_INPUT(text=\"l\" w=W)
on: text: event: KEY_UP(which=1 scancode=15 key=0x6c mod=0x0 raw=40 repeat=0 w=W)
on: text: event: KEY_DOWN(which=1 scancode=15 key=0x6c mod=0x0 raw=40 repeat=0 w=W)
on: text: event: TEXT_INPUT(text=\"l\" w=W)
on: text: event: KEY_UP(which=1 scancode=15 key=0x6c mod=0x0 raw=40 repeat=0 w=W)
on: text: event: KEY_DOWN(which=1 scancode=18 key=0x6f mod=0x0 raw=43 repeat=0 w=W)
on: text: event: TEXT_INPUT(text=\"o\" w=W)
on: text: event: KEY_UP(which=1 scancode=18 key=0x6f mod=0x0 raw=43 repeat=0 w=W)
on: text input active=yes screen keyboard shown=yes
logcat: ime: mInputShown=true
on: insets: ime visible=yes
on: focused view: org.libsdl.app.SDLDummyEdit
ime: input connection: org.libsdl.app.SDLInputConnection inputType=0x3c001 imeOptions=0x12000000
ime: editable=\"\" committed=\"\"
commit hello: true
commit hello: event: KEY_DOWN(which=0 scancode=11 key=0x68 mod=0x0 raw=0 repeat=0 w=W)
commit hello: event: KEY_UP(which=0 scancode=11 key=0x68 mod=0x0 raw=0 repeat=0 w=W)
commit hello: event: KEY_DOWN(which=0 scancode=8 key=0x65 mod=0x0 raw=0 repeat=0 w=W)
commit hello: event: KEY_UP(which=0 scancode=8 key=0x65 mod=0x0 raw=0 repeat=0 w=W)
commit hello: event: KEY_DOWN(which=0 scancode=15 key=0x6c mod=0x0 raw=0 repeat=0 w=W)
commit hello: event: KEY_UP(which=0 scancode=15 key=0x6c mod=0x0 raw=0 repeat=0 w=W)
commit hello: event: KEY_DOWN(which=0 scancode=15 key=0x6c mod=0x0 raw=0 repeat=0 w=W)
commit hello: event: KEY_UP(which=0 scancode=15 key=0x6c mod=0x0 raw=0 repeat=0 w=W)
commit hello: event: KEY_DOWN(which=0 scancode=18 key=0x6f mod=0x0 raw=0 repeat=0 w=W)
commit hello: event: KEY_UP(which=0 scancode=18 key=0x6f mod=0x0 raw=0 repeat=0 w=W)
commit hello: event: TEXT_INPUT(text=\"hello\" w=W)
commit hello: editable=\"hello\" committed=\"hello\"
commit Hi!: true
commit Hi!: event: KEY_DOWN(which=0 scancode=225 key=0x400000e1 mod=0x1 raw=0 repeat=0 w=W)
commit Hi!: event: KEY_DOWN(which=0 scancode=11 key=0x68 mod=0x1 raw=0 repeat=0 w=W)
commit Hi!: event: KEY_UP(which=0 scancode=11 key=0x68 mod=0x1 raw=0 repeat=0 w=W)
commit Hi!: event: KEY_UP(which=0 scancode=225 key=0x400000e1 mod=0x0 raw=0 repeat=0 w=W)
commit Hi!: event: KEY_DOWN(which=0 scancode=12 key=0x69 mod=0x0 raw=0 repeat=0 w=W)
commit Hi!: event: KEY_UP(which=0 scancode=12 key=0x69 mod=0x0 raw=0 repeat=0 w=W)
commit Hi!: event: KEY_DOWN(which=0 scancode=225 key=0x400000e1 mod=0x1 raw=0 repeat=0 w=W)
commit Hi!: event: KEY_DOWN(which=0 scancode=30 key=0x31 mod=0x1 raw=0 repeat=0 w=W)
commit Hi!: event: KEY_UP(which=0 scancode=30 key=0x31 mod=0x1 raw=0 repeat=0 w=W)
commit Hi!: event: KEY_UP(which=0 scancode=225 key=0x400000e1 mod=0x0 raw=0 repeat=0 w=W)
commit Hi!: event: TEXT_INPUT(text=\"Hi!\" w=W)
commit Hi!: editable=\"helloHi!\" committed=\"helloHi!\"
commit e acute, grinning face: true
commit e acute, grinning face: event: TEXT_INPUT(text=\"\\xc3\\xa9\\xf0\\x9f\\x98\\x80\" w=W)
commit e acute, grinning face: editable=\"helloHi!\\u00e9\\ud83d\\ude00\" committed=\"helloHi!\\u00e9\\ud83d\\ude00\"
compose wor: true
compose wor: event: KEY_DOWN(which=0 scancode=26 key=0x77 mod=0x0 raw=0 repeat=0 w=W)
compose wor: event: KEY_UP(which=0 scancode=26 key=0x77 mod=0x0 raw=0 repeat=0 w=W)
compose wor: event: KEY_DOWN(which=0 scancode=18 key=0x6f mod=0x0 raw=0 repeat=0 w=W)
compose wor: event: KEY_UP(which=0 scancode=18 key=0x6f mod=0x0 raw=0 repeat=0 w=W)
compose wor: event: KEY_DOWN(which=0 scancode=21 key=0x72 mod=0x0 raw=0 repeat=0 w=W)
compose wor: event: KEY_UP(which=0 scancode=21 key=0x72 mod=0x0 raw=0 repeat=0 w=W)
compose wor: event: TEXT_INPUT(text=\"wor\" w=W)
compose wor: editable=\"helloHi!\\u00e9\\ud83d\\ude00wor\" committed=\"helloHi!\\u00e9\\ud83d\\ude00wor\"
compose worl: true
compose worl: event: KEY_DOWN(which=0 scancode=15 key=0x6c mod=0x0 raw=0 repeat=0 w=W)
compose worl: event: KEY_UP(which=0 scancode=15 key=0x6c mod=0x0 raw=0 repeat=0 w=W)
compose worl: event: TEXT_INPUT(text=\"l\" w=W)
compose worl: editable=\"helloHi!\\u00e9\\ud83d\\ude00worl\" committed=\"helloHi!\\u00e9\\ud83d\\ude00worl\"
compose wox: true
compose wox: event: KEY_DOWN(which=0 scancode=42 key=0x8 mod=0x0 raw=0 repeat=0 w=W)
compose wox: event: KEY_UP(which=0 scancode=42 key=0x8 mod=0x0 raw=0 repeat=0 w=W)
compose wox: event: KEY_DOWN(which=0 scancode=42 key=0x8 mod=0x0 raw=0 repeat=0 w=W)
compose wox: event: KEY_UP(which=0 scancode=42 key=0x8 mod=0x0 raw=0 repeat=0 w=W)
compose wox: event: KEY_DOWN(which=0 scancode=27 key=0x78 mod=0x0 raw=0 repeat=0 w=W)
compose wox: event: KEY_UP(which=0 scancode=27 key=0x78 mod=0x0 raw=0 repeat=0 w=W)
compose wox: event: TEXT_INPUT(text=\"x\" w=W)
compose wox: editable=\"helloHi!\\u00e9\\ud83d\\ude00wox\" committed=\"helloHi!\\u00e9\\ud83d\\ude00wox\"
finish composing: true
finish composing: events: (none)
finish composing: editable=\"helloHi!\\u00e9\\ud83d\\ude00wox\" committed=\"helloHi!\\u00e9\\ud83d\\ude00wox\"
delete 1 0: true
delete 1 0: event: KEY_DOWN(which=0 scancode=42 key=0x8 mod=0x0 raw=0 repeat=0 w=W)
delete 1 0: event: KEY_UP(which=0 scancode=42 key=0x8 mod=0x0 raw=0 repeat=0 w=W)
delete 1 0: editable=\"helloHi!\\u00e9\\ud83d\\ude00wox\" committed=\"helloHi!\\u00e9\\ud83d\\ude00wox\"
delete 0 1: true
delete 0 1: events: (none)
delete 0 1: editable=\"helloHi!\\u00e9\\ud83d\\ude00wox\" committed=\"helloHi!\\u00e9\\ud83d\\ude00wox\"
delete 3 1: true
delete 3 1: event: KEY_DOWN(which=0 scancode=42 key=0x8 mod=0x0 raw=0 repeat=0 w=W)
delete 3 1: event: KEY_UP(which=0 scancode=42 key=0x8 mod=0x0 raw=0 repeat=0 w=W)
delete 3 1: event: KEY_DOWN(which=0 scancode=42 key=0x8 mod=0x0 raw=0 repeat=0 w=W)
delete 3 1: event: KEY_UP(which=0 scancode=42 key=0x8 mod=0x0 raw=0 repeat=0 w=W)
delete 3 1: event: KEY_DOWN(which=0 scancode=42 key=0x8 mod=0x0 raw=0 repeat=0 w=W)
delete 3 1: event: KEY_UP(which=0 scancode=42 key=0x8 mod=0x0 raw=0 repeat=0 w=W)
delete 3 1: editable=\"helloHi!\\u00e9\\ud83d\\ude00\" committed=\"helloHi!\\u00e9\\ud83d\\ude00\"
compose ab: true
compose ab: event: KEY_DOWN(which=0 scancode=4 key=0x61 mod=0x0 raw=0 repeat=0 w=W)
compose ab: event: KEY_UP(which=0 scancode=4 key=0x61 mod=0x0 raw=0 repeat=0 w=W)
compose ab: event: KEY_DOWN(which=0 scancode=5 key=0x62 mod=0x0 raw=0 repeat=0 w=W)
compose ab: event: KEY_UP(which=0 scancode=5 key=0x62 mod=0x0 raw=0 repeat=0 w=W)
compose ab: event: TEXT_INPUT(text=\"ab\" w=W)
compose ab: editable=\"helloHi!\\u00e9\\ud83d\\ude00ab\" committed=\"helloHi!\\u00e9\\ud83d\\ude00ab\"
commit abc: true
commit abc: event: KEY_DOWN(which=0 scancode=6 key=0x63 mod=0x0 raw=0 repeat=0 w=W)
commit abc: event: KEY_UP(which=0 scancode=6 key=0x63 mod=0x0 raw=0 repeat=0 w=W)
commit abc: event: TEXT_INPUT(text=\"c\" w=W)
commit abc: editable=\"helloHi!\\u00e9\\ud83d\\ude00abc\" committed=\"helloHi!\\u00e9\\ud83d\\ude00abc\"
key enter down: false
key enter down: event: KEY_DOWN(which=1 scancode=40 key=0xd mod=0x0 raw=66 repeat=0 w=W)
key enter down: editable=\"helloHi!\\u00e9\\ud83d\\ude00abc\" committed=\"helloHi!\\u00e9\\ud83d\\ude00abc\"
key enter up: false
key enter up: event: KEY_UP(which=1 scancode=40 key=0xd mod=0x0 raw=66 repeat=0 w=W)
key enter up: editable=\"helloHi!\\u00e9\\ud83d\\ude00abc\" committed=\"helloHi!\\u00e9\\ud83d\\ude00abc\"
key a down: false
key a down: event: KEY_DOWN(which=1 scancode=4 key=0x61 mod=0x0 raw=29 repeat=0 w=W)
key a down: event: TEXT_INPUT(text=\"a\" w=W)
key a down: editable=\"helloHi!\\u00e9\\ud83d\\ude00abca\" committed=\"helloHi!\\u00e9\\ud83d\\ude00abca\"
key a up: false
key a up: event: KEY_UP(which=1 scancode=4 key=0x61 mod=0x0 raw=29 repeat=0 w=W)
key a up: editable=\"helloHi!\\u00e9\\ud83d\\ude00abca\" committed=\"helloHi!\\u00e9\\ud83d\\ude00abca\"
commit newline: true
commit newline: event: KEY_DOWN(which=0 scancode=40 key=0xd mod=0x0 raw=0 repeat=0 w=W)
commit newline: event: KEY_UP(which=0 scancode=40 key=0xd mod=0x0 raw=0 repeat=0 w=W)
commit newline: editable=\"helloHi!\\u00e9\\ud83d\\ude00abca\\u000a\" committed=\"helloHi!\\u00e9\\ud83d\\ude00abca\\u000a\"
delete 2 0: true
delete 2 0: event: KEY_DOWN(which=0 scancode=42 key=0x8 mod=0x0 raw=0 repeat=0 w=W)
delete 2 0: event: KEY_UP(which=0 scancode=42 key=0x8 mod=0x0 raw=0 repeat=0 w=W)
delete 2 0: event: KEY_DOWN(which=0 scancode=42 key=0x8 mod=0x0 raw=0 repeat=0 w=W)
delete 2 0: event: KEY_UP(which=0 scancode=42 key=0x8 mod=0x0 raw=0 repeat=0 w=W)
delete 2 0: editable=\"helloHi!\\u00e9\\ud83d\\ude00abca\\u000a\" committed=\"helloHi!\\u00e9\\ud83d\\ude00abca\\u000a\"
ime: text input active=yes screen keyboard shown=yes
logcat: ime: mInputShown=true
ime: insets: ime visible=yes
ime: focused view: org.libsdl.app.SDLDummyEdit
hides: commit x newline y: true
hides: commit x newline y: event: KEY_DOWN(which=0 scancode=27 key=0x78 mod=0x0 raw=0 repeat=0 w=W)
hides: commit x newline y: event: KEY_UP(which=0 scancode=27 key=0x78 mod=0x0 raw=0 repeat=0 w=W)
hides: commit x newline y: event: SCREEN_KEYBOARD_HIDDEN
hides: commit x newline y: editable=\"helloHi!\\u00e9\\ud83d\\ude00abca\\u000ax\\u000ay\" committed=\"helloHi!\\u00e9\\ud83d\\ude00abca\\u000a\"
hides: commit x newline y: events: (none)
hides: commit x newline y: text input active=no screen keyboard shown=no
logcat: ime: mInputShown=false
hides: commit x newline y: insets: ime visible=no
hides: commit x newline y: focused view: org.libsdl.app.SDLSurface
hides: again: start text input: ok
hides: again: event: SCREEN_KEYBOARD_SHOWN
hides: again: text input active=yes screen keyboard shown=yes
logcat: ime: mInputShown=true
hides: again: insets: ime visible=yes
hides: again: focused view: org.libsdl.app.SDLDummyEdit
hides: again: text edit: left=100 top=200 width=300 height=55 visibility=0
hides: again: input connection: org.libsdl.app.SDLInputConnection inputType=0x1c001 imeOptions=0x12000000
hides: key enter down: true
hides: key enter down: event: SCREEN_KEYBOARD_HIDDEN
hides: key enter down: editable=\"\" committed=\"\"
hides: key enter down: events: (none)
hides: key enter down: text input active=no screen keyboard shown=no
logcat: ime: mInputShown=false
hides: key enter down: insets: ime visible=no
hides: key enter down: focused view: org.libsdl.app.SDLSurface
hides: once more: start text input: ok
hides: once more: event: SCREEN_KEYBOARD_SHOWN
hides: once more: text input active=yes screen keyboard shown=yes
logcat: ime: mInputShown=true
hides: once more: insets: ime visible=yes
hides: once more: focused view: org.libsdl.app.SDLDummyEdit
hides: once more: text edit: left=100 top=200 width=300 height=55 visibility=0
hides: enter: event: KEY_DOWN(which=1 scancode=40 key=0xd mod=0x0 raw=66 repeat=0 w=W)
hides: enter: event: KEY_UP(which=1 scancode=40 key=0xd mod=0x0 raw=66 repeat=0 w=W)
hides: enter: text input active=yes screen keyboard shown=yes
logcat: ime: mInputShown=true
hides: enter: insets: ime visible=yes
hides: enter: focused view: org.libsdl.app.SDLDummyEdit
back, text input on: event: SCREEN_KEYBOARD_HIDDEN
back, text input on: events: (none)
back, text input on: text input active=yes screen keyboard shown=no
logcat: ime: mInputShown=false
back, text input on: insets: ime visible=no
back, text input on: focused view: org.libsdl.app.SDLDummyEdit
restore: start text input: ok
restore: event: SCREEN_KEYBOARD_SHOWN
restore: text input active=yes screen keyboard shown=yes
logcat: ime: mInputShown=true
restore: insets: ime visible=yes
restore: focused view: org.libsdl.app.SDLDummyEdit
restore: text edit: left=100 top=200 width=300 height=55 visibility=0
restore: text input area: ok
restore: area moved: text edit: left=100 top=200 width=300 height=55 visibility=0
restore: text input active=yes screen keyboard shown=yes
logcat: ime: mInputShown=true
restore: insets: ime visible=yes
restore: focused view: org.libsdl.app.SDLDummyEdit
restore: text edit: left=50 top=300 width=200 height=45 visibility=0
stopped: stop text input: ok
stopped: event: SCREEN_KEYBOARD_HIDDEN
stopped: text input active=no screen keyboard shown=no
logcat: ime: mInputShown=false
stopped: insets: ime visible=no
stopped: focused view: org.libsdl.app.SDLSurface
props none: ok inputType=0x3c001
props text: ok inputType=0x3c001
props text name: ok inputType=0x3a061
props text email: ok inputType=0x38021
props text username: ok inputType=0x38001
props text password hidden: ok inputType=0x38081
props text password visible: ok inputType=0x38091
props number: ok inputType=0x38002
props number password hidden: ok inputType=0x38012
props number password visible: ok inputType=0x38002
props capitalize none: ok inputType=0x38001
props capitalize sentences: ok inputType=0x3c001
props capitalize words: ok inputType=0x3a001
props capitalize letters: ok inputType=0x39001
props autocorrect off: ok inputType=0x24001
props multiline off: ok inputType=0x1c001
props return key hides: ok inputType=0x1c001
props android input type 0x21: ok inputType=0x21
properties: text input active=no screen keyboard shown=no
logcat: ime: mInputShown=false
properties: insets: ime visible=no
properties: focused view: org.libsdl.app.SDLSurface
trip: start text input: ok
trip: event: SCREEN_KEYBOARD_SHOWN
trip: text input active=yes screen keyboard shown=yes
logcat: ime: mInputShown=true
trip: insets: ime visible=yes
trip: focused view: org.libsdl.app.SDLDummyEdit
trip: text edit: left=50 top=300 width=200 height=45 visibility=0
trip: event: SCREEN_KEYBOARD_HIDDEN
trip: text input active=yes screen keyboard shown=no
logcat: ime: mInputShown=false
trip: insets: ime visible=no
trip: focused view: org.libsdl.app.SDLSurface
trip: text edit: none (org.libsdl.app.SDLSurface)
after the trip: stop text input: ok
after the trip: events: (none)
after the trip: text input active=no screen keyboard shown=no
logcat: ime: mInputShown=false
after the trip: insets: ime visible=no
after the trip: focused view: org.libsdl.app.SDLSurface
end: keyboards=0 has keyboard=no mod state=0x0
quit
";

    static TRACE: Mutex<Vec<String>> = Mutex::new(Vec::new());

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

    fn result<T>(r: &sdl3::Result<T>) -> String {
        match r {
            Ok(_) => "ok".to_string(),
            Err(e) => format!("error: {e}"),
        }
    }

    // JNI: the activity, the classes the program makes objects of

    struct Jni {
        env: JNIEnv,
        activity: Jobject,
        editor_info: Jobject,
        key_event: Jobject,
        window_insets_type: Jobject,
        /// The text edit, a global reference (null: none).
        edit: Jobject,
        /// Its input connection, a global reference (null: none).
        ic: Jobject,
    }

    // SAFETY: the program's one thread (SDL's) uses it.
    unsafe impl Send for Jni {}

    static JNI: Mutex<Option<Jni>> = Mutex::new(None);

    fn with_jni<R>(f: impl FnOnce(&mut Jni) -> R) -> R {
        f(JNI.lock().unwrap().as_mut().expect("the JNI state"))
    }

    fn global_class(env: JNIEnv, name: &str) -> Jobject {
        let class = jni::find_class(env, name);
        let global = jni::new_global_ref(env, class);
        jni::delete_local_ref(env, class);
        global
    }

    /// A Java string as the trace writes it: printable ASCII as it is (with
    /// \" and \\), the other UTF-16 units as \uXXXX.
    fn java_text(env: JNIEnv, s: Jobject) -> String {
        if s.is_null() {
            return "null".into();
        }
        let mut out = String::new();
        for u in jni::string_utf16(env, s) {
            match u {
                0x22 | 0x5c => {
                    out.push('\\');
                    out.push(u as u8 as char);
                }
                0x20..0x7f => out.push(u as u8 as char),
                _ => out.push_str(&format!("\\u{u:04x}")),
            }
        }
        out
    }

    /// UTF-8 as the trace writes it: printable ASCII as it is (with \" and
    /// \\), the other bytes as \xNN.
    fn utf8_text(s: &str) -> String {
        let mut out = String::new();
        for &b in s.as_bytes() {
            match b {
                b'"' | b'\\' => {
                    out.push('\\');
                    out.push(b as char);
                }
                0x20..0x7f => out.push(b as char),
                _ => out.push_str(&format!("\\x{b:02x}")),
            }
        }
        out
    }

    /// `String.valueOf(obj)`, as the trace writes it.
    fn to_string(env: JNIEnv, obj: Jobject) -> String {
        if obj.is_null() {
            return "null".into();
        }
        let s = jni::call_object(env, obj, "toString", "()Ljava/lang/String;", &[]);
        let text = java_text(env, s);
        if !s.is_null() {
            jni::delete_local_ref(env, s);
        }
        text
    }

    fn focus_view(j: &Jni) -> Jobject {
        jni::call_object(
            j.env,
            j.activity,
            "getCurrentFocus",
            "()Landroid/view/View;",
            &[],
        )
    }

    fn class_name_of(env: JNIEnv, obj: Jobject) -> String {
        if obj.is_null() {
            return "null".into();
        }
        let c = jni::call_object(env, obj, "getClass", "()Ljava/lang/Class;", &[]);
        let n = jni::call_object(env, c, "getName", "()Ljava/lang/String;", &[]);
        let text = java_text(env, n);
        jni::delete_local_ref(env, n);
        jni::delete_local_ref(env, c);
        text
    }

    fn focused_name() -> String {
        with_jni(|j| {
            let v = focus_view(j);
            let name = class_name_of(j.env, v);
            if !v.is_null() {
                jni::delete_local_ref(j.env, v);
            }
            name
        })
    }

    /// Whether the activity's window insets have the input method visible
    /// (its decor view's root insets: `isVisible(WindowInsets.Type.ime())`).
    fn insets_ime_visible() -> &'static str {
        with_jni(|j| {
            let env = j.env;
            let window =
                jni::call_object(env, j.activity, "getWindow", "()Landroid/view/Window;", &[]);
            let decor = jni::call_object(env, window, "getDecorView", "()Landroid/view/View;", &[]);
            let insets = jni::call_object(
                env,
                decor,
                "getRootWindowInsets",
                "()Landroid/view/WindowInsets;",
                &[],
            );
            let mut r = "null";
            if !insets.is_null() {
                let ty = jni::call_static_int(env, j.window_insets_type, "ime", "()I", &[]);
                r = yes(jni::call_boolean(
                    env,
                    insets,
                    "isVisible",
                    "(I)Z",
                    &[Jvalue { i: ty }],
                ));
                jni::delete_local_ref(env, insets);
            }
            jni::delete_local_ref(env, decor);
            jni::delete_local_ref(env, window);
            r
        })
    }

    // Events

    struct Events {
        lines: Vec<String>,
        seen: Vec<EventType>,
    }

    static EVENTS: Mutex<Events> = Mutex::new(Events {
        lines: Vec::new(),
        seen: Vec::new(),
    });
    static WINDOW: Mutex<WindowID> = Mutex::new(0);

    fn wl(id: WindowID) -> &'static str {
        if id == *WINDOW.lock().unwrap() {
            "W"
        } else if id == 0 {
            "0"
        } else {
            "?"
        }
    }

    /// An event as the trace names it (the C program's names).
    fn describe(event: &Event) -> String {
        let n = event_name(event.event_type());
        match event {
            Event::Key(e) => format!(
                "{n}(which={} scancode={} key={:#x} mod={:#x} raw={} repeat={} w={})",
                e.which,
                e.scancode.0,
                e.key.0,
                e.modifiers.0,
                e.raw,
                e.repeat as i32,
                wl(e.window_id)
            ),
            Event::TextInput(e) => {
                format!("{n}(text=\"{}\" w={})", utf8_text(&e.text), wl(e.window_id))
            }
            Event::TextEditing(e) => format!(
                "{n}(text=\"{}\" start={} length={} w={})",
                utf8_text(&e.text),
                e.start,
                e.length,
                wl(e.window_id)
            ),
            Event::KeyboardDevice(e) => format!("{n}(which={})", e.which),
            _ => n,
        }
    }

    /// Whether the trace keeps an event: the keyboard's.
    fn traced(ty: EventType) -> bool {
        (EventType::KEY_DOWN.0..=EventType::SCREEN_KEYBOARD_HIDDEN.0).contains(&ty.0)
    }

    fn take_queued() {
        events::pump();
        while let Some(event) = events::poll() {
            let ty = event.event_type();
            let mut e = EVENTS.lock().unwrap();
            e.seen.push(ty);
            if traced(ty) {
                e.lines.push(describe(&event));
            }
        }
    }

    fn saw_type(ty: EventType) -> bool {
        EVENTS.lock().unwrap().seen.contains(&ty)
    }

    /// Pump until an event of `ty` (none: `None`), up to 5 s, then `ms`
    /// milliseconds.
    fn settle(ty: Option<EventType>, ms: u64) {
        let end = Instant::now() + Duration::from_millis(5000);
        take_queued();
        while ty.is_some_and(|ty| !saw_type(ty)) && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(10));
            take_queued();
        }
        let end = Instant::now() + Duration::from_millis(ms);
        while Instant::now() < end {
            std::thread::sleep(Duration::from_millis(10));
            take_queued();
        }
        EVENTS.lock().unwrap().seen.clear();
    }

    fn clear_events() {
        let mut e = EVENTS.lock().unwrap();
        e.lines.clear();
        e.seen.clear();
    }

    /// The events since the last trace, one line each.
    fn trace_events(label: &str) {
        let lines = std::mem::take(&mut EVENTS.lock().unwrap().lines);
        if lines.is_empty() {
            t(format!("{label}: events: (none)"));
        }
        for line in lines {
            t(format!("{label}: event: {line}"));
        }
    }

    /// Poll (pumping) until the focused view is of the class, up to 5 s.
    fn wait_focus(class_name: &str) {
        let end = Instant::now() + Duration::from_millis(5000);
        loop {
            if focused_name() == class_name || Instant::now() >= end {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
            take_queued();
        }
    }

    /// A request of the runner's, and the lines of its answer in the trace
    /// (as the C reference's job puts them there).
    fn ask(what: &str) {
        for line in request(what) {
            t(format!("logcat: {line}"));
        }
    }

    /// A request of the runner's `input`, and its events (until `ty`, then
    /// 300 ms).
    fn real(label: &str, args: &str, ty: Option<EventType>) {
        ask(&format!("input {args}"));
        settle(ty, 300);
        trace_events(label);
    }

    const SURFACE: &str = "org.libsdl.app.SDLSurface";
    const DUMMY_EDIT: &str = "org.libsdl.app.SDLDummyEdit";

    /// The state of text input and the screen keyboard: SDL's, the input
    /// method's (the runner's `ime` request, once it is as `ime`), the
    /// window insets', the focused view.
    fn state(w: &Window, label: &str, ime: &str) {
        t(format!(
            "{label}: text input active={} screen keyboard shown={}",
            yes(w.text_input_active().unwrap_or(false)),
            yes(w.screen_keyboard_shown().unwrap_or(false))
        ));
        ask(&format!("ime {ime}"));
        t(format!(
            "{label}: insets: ime visible={}",
            insets_ime_visible()
        ));
        t(format!("{label}: focused view: {}", focused_name()));
    }

    /// The text edit (the focused view once text input has started), kept.
    fn take_edit(label: &str) {
        let line = with_jni(|j| {
            let env = j.env;
            if !j.edit.is_null() {
                jni::delete_global_ref(env, j.edit);
                j.edit = std::ptr::null_mut();
            }
            let v = focus_view(j);
            let name = class_name_of(env, v);
            let line = if !v.is_null() && name == DUMMY_EDIT {
                j.edit = jni::new_global_ref(env, v);
                let int = |m: &str| jni::call_int(env, j.edit, m, "()I", &[]);
                format!(
                    "{label}: text edit: left={} top={} width={} height={} visibility={}",
                    int("getLeft"),
                    int("getTop"),
                    int("getWidth"),
                    int("getHeight"),
                    int("getVisibility")
                )
            } else {
                format!("{label}: text edit: none ({name})")
            };
            if !v.is_null() {
                jni::delete_local_ref(env, v);
            }
            line
        });
        t(line);
    }

    /// `view.onCreateInputConnection(new EditorInfo())`: the connection (a
    /// local reference), with the EditorInfo's inputType and imeOptions.
    fn create_input_connection(j: &Jni, view: Jobject) -> (Jobject, i32, i32) {
        let env = j.env;
        let info = jni::new_object(env, j.editor_info, "()V", &[]);
        let c = jni::call_object(
            env,
            view,
            "onCreateInputConnection",
            "(Landroid/view/inputmethod/EditorInfo;)Landroid/view/inputmethod/InputConnection;",
            &[Jvalue { l: info }],
        );
        let input_type = jni::get_int_field(env, j.editor_info, info, "inputType");
        let ime_options = jni::get_int_field(env, j.editor_info, info, "imeOptions");
        jni::delete_local_ref(env, info);
        (c, input_type, ime_options)
    }

    /// The text edit's input connection, kept: `onCreateInputConnection()`
    /// of a new EditorInfo, whose inputType and imeOptions it sets.
    fn new_ic(label: &str) {
        let line = with_jni(|j| {
            let env = j.env;
            if !j.ic.is_null() {
                jni::delete_global_ref(env, j.ic);
                j.ic = std::ptr::null_mut();
            }
            if j.edit.is_null() {
                return format!("{label}: input connection: no text edit");
            }
            let (c, input_type, ime_options) = create_input_connection(j, j.edit);
            let name = class_name_of(env, c);
            if !c.is_null() {
                j.ic = jni::new_global_ref(env, c);
                jni::delete_local_ref(env, c);
            }
            format!("{label}: input connection: {name} inputType={input_type:#x} imeOptions={ime_options:#x}")
        });
        t(line);
    }

    /// The input connection's text (its `getEditable()`) and what it last
    /// sent (`mCommittedText`).
    fn editable(label: &str) {
        let line = with_jni(|j| {
            let env = j.env;
            let e = jni::call_object(env, j.ic, "getEditable", "()Landroid/text/Editable;", &[]);
            let text = to_string(env, e);
            if !e.is_null() {
                jni::delete_local_ref(env, e);
            }
            let c = jni::object_class(env, j.ic);
            let s = jni::get_object_field(env, c, j.ic, "mCommittedText", "Ljava/lang/String;");
            jni::delete_local_ref(env, c);
            let committed = java_text(env, s);
            if !s.is_null() {
                jni::delete_local_ref(env, s);
            }
            format!("{label}: editable=\"{text}\" committed=\"{committed}\"")
        });
        t(line);
    }

    /// One of the input connection's calls (what an input method does).
    #[derive(Clone, Copy)]
    enum Op<'a> {
        Commit(&'a str),
        Compose(&'a str),
        Finish,
        Delete(i32, i32),
        Key(i32, i32),
    }

    /// An input connection's call, its result, then its events and the
    /// text.
    fn ime_call(label: &str, op: Op<'_>) {
        let r = with_jni(|j| {
            let env = j.env;
            let ic = j.ic;
            match op {
                Op::Commit(text) | Op::Compose(text) => {
                    let s = jni::new_string(env, text);
                    let name = if matches!(op, Op::Commit(_)) {
                        "commitText"
                    } else {
                        "setComposingText"
                    };
                    let r = jni::call_boolean(
                        env,
                        ic,
                        name,
                        "(Ljava/lang/CharSequence;I)Z",
                        &[Jvalue { l: s }, Jvalue { i: 1 }],
                    );
                    jni::delete_local_ref(env, s);
                    r
                }
                Op::Finish => jni::call_boolean(env, ic, "finishComposingText", "()Z", &[]),
                Op::Delete(before, after) => jni::call_boolean(
                    env,
                    ic,
                    "deleteSurroundingText",
                    "(II)Z",
                    &[Jvalue { i: before }, Jvalue { i: after }],
                ),
                Op::Key(action, code) => {
                    let ev = jni::new_object(
                        env,
                        j.key_event,
                        "(II)V",
                        &[Jvalue { i: action }, Jvalue { i: code }],
                    );
                    let r = jni::call_boolean(
                        env,
                        ic,
                        "sendKeyEvent",
                        "(Landroid/view/KeyEvent;)Z",
                        &[Jvalue { l: ev }],
                    );
                    jni::delete_local_ref(env, ev);
                    r
                }
            }
        });
        t(format!("{label}: {r}"));
        settle(None, 300);
        trace_events(label);
        editable(label);
    }

    /// `start_text_input()`, then its events once the text edit has the
    /// focus (and 1 s), and the state.
    fn start(w: &Window, label: &str, props: Option<&Properties>) {
        let r = match props {
            Some(p) => w.start_text_input_with_properties(Some(p)),
            None => w.start_text_input(),
        };
        t(format!("{label}: start text input: {}", result(&r)));
        wait_focus(DUMMY_EDIT);
        settle(None, 1000);
        trace_events(label);
        state(w, label, "shown");
        take_edit(label);
    }

    /// `stop_text_input()`, then its events once the surface has the focus
    /// (and 1 s), and the state.
    fn stop(w: &Window, label: &str) {
        t(format!(
            "{label}: stop text input: {}",
            result(&w.stop_text_input())
        ));
        wait_focus(SURFACE);
        settle(None, 1000);
        trace_events(label);
        state(w, label, "hidden");
    }

    /// The text edit's input type (its `input_type` field).
    fn edit_input_type() -> i32 {
        with_jni(|j| {
            let c = jni::object_class(j.env, j.edit);
            let r = jni::get_int_field(j.env, c, j.edit, "input_type");
            jni::delete_local_ref(j.env, c);
            r
        })
    }

    fn set_edit_input_type(value: i32) {
        with_jni(|j| {
            let c = jni::object_class(j.env, j.edit);
            jni::set_int_field(j.env, c, j.edit, "input_type", value);
            jni::delete_local_ref(j.env, c);
        })
    }

    /// Pump until SDL has had the screen keyboard hidden for 500 ms (up to
    /// 5 s), hiding it again (`stop_text_input()`) whenever SDL has it
    /// shown: the text edit's task, or the window insets, may tell it
    /// shown after the stop, and a start while SDL has it shown doesn't
    /// show it again.
    fn keyboard_hidden(w: &Window) {
        let end = Instant::now() + Duration::from_millis(5000);
        let mut since = Instant::now();
        while Instant::now() < end {
            std::thread::sleep(Duration::from_millis(10));
            take_queued();
            if w.screen_keyboard_shown().unwrap_or(false) {
                let _ = w.stop_text_input();
                since = Instant::now();
            } else if since.elapsed() >= Duration::from_millis(500) {
                break;
            }
        }
    }

    /// A text input properties' case: started with them (once the text
    /// edit has its input type: its field is -1 until then), the input type
    /// the text edit gives an input method, stopped (until SDL has the
    /// screen keyboard hidden: [`keyboard_hidden`]; its events not traced).
    /// The input method doesn't show its keyboard meanwhile (the hardware
    /// keyboard's setting off), so that the window insets don't tell it
    /// shown and hidden whenever its animations end.
    fn props_case(w: &Window, label: &str, props: Option<&Properties>) {
        if with_jni(|j| j.edit.is_null()) {
            t(format!("props {label}: no text edit"));
            return;
        }
        set_edit_input_type(-1);
        let r = match props {
            Some(p) => w.start_text_input_with_properties(Some(p)),
            None => w.start_text_input(),
        };
        let end = Instant::now() + Duration::from_millis(5000);
        while edit_input_type() == -1 && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(10));
            take_queued();
        }
        let input_type = with_jni(|j| {
            let (c, input_type, _) = create_input_connection(j, j.edit);
            if !c.is_null() {
                jni::delete_local_ref(j.env, c);
            }
            input_type
        });
        t(format!(
            "props {label}: {} inputType={input_type:#x}",
            result(&r)
        ));
        let _ = w.stop_text_input();
        keyboard_hidden(w);
        clear_events();
    }

    fn props_type(w: &Window, label: &str, ty: TextInputType) {
        let p = Properties::new();
        p.set(textinput::PROP_TEXTINPUT_TYPE_NUMBER, ty as i64)
            .unwrap();
        props_case(w, label, Some(&p));
    }

    fn props_capitalization(w: &Window, label: &str, capitalization: Capitalization) {
        let p = Properties::new();
        p.set(
            textinput::PROP_TEXTINPUT_TYPE_NUMBER,
            TextInputType::Text as i64,
        )
        .unwrap();
        p.set(
            textinput::PROP_TEXTINPUT_CAPITALIZATION_NUMBER,
            capitalization as i64,
        )
        .unwrap();
        props_case(w, label, Some(&p));
    }

    fn props_bool(w: &Window, label: &str, name: &str, value: bool) {
        let p = Properties::new();
        p.set(name, value).unwrap();
        props_case(w, label, Some(&p));
    }

    /// The keyboards SDL knows.
    fn keyboards(label: &str) {
        t(format!(
            "{label}: keyboards={} has keyboard={} mod state={:#x}",
            keyboard::keyboards().len(),
            yes(keyboard::has_keyboard()),
            keyboard::mod_state().0
        ));
    }

    const ACTION_DOWN: i32 = 0;
    const ACTION_UP: i32 = 1;
    const KEYCODE_A: i32 = 29;
    const KEYCODE_ENTER: i32 = 66;

    pub(super) fn main() {
        // (the pump doesn't block while the app is paused: the trip below
        // traces the events in the background)
        let _ = hints::set(hints::ANDROID_BLOCK_ON_PAUSE, "0");
        t(format!("init: {}", result(&init::init(InitFlags::VIDEO))));
        t(format!(
            "video driver: {}",
            video::current_video_driver().unwrap_or("(none)")
        ));
        t(format!(
            "screen keyboard support: {}",
            yes(textinput::has_screen_keyboard_support())
        ));
        keyboards("start");

        let env = android::jni_env().expect("a JNI environment").as_ptr();
        let activity = android::activity().expect("the activity").as_ptr();
        *JNI.lock().unwrap() = Some(Jni {
            env,
            activity: jni::new_global_ref(env, activity),
            editor_info: global_class(env, "android/view/inputmethod/EditorInfo"),
            key_event: global_class(env, "android/view/KeyEvent"),
            window_insets_type: global_class(env, "android/view/WindowInsets$Type"),
            edit: std::ptr::null_mut(),
            ic: std::ptr::null_mut(),
        });
        jni::delete_local_ref(env, activity);

        // (the virtual device has a hardware keyboard: the input method
        // shows its keyboard all the same)
        ask("ime-with-hard-keyboard yes");

        let created = Window::create("SDL keyboard", 320, 240, WindowFlags::RESIZABLE);
        t(format!("create: {}", result(&created)));
        let w = created.expect("the window");
        *WINDOW.lock().unwrap() = w.id();
        // (until the activity's surface is ready and has the focus)
        settle(Some(EventType::WINDOW_FOCUS_GAINED), 2000);
        trace_events("created");
        state(&w, "created", "hidden");
        keyboards("created");

        let up = Some(EventType::KEY_UP);

        // Real keys, text input off
        real("a", "keyevent KEYCODE_A", up);
        real("5", "keyevent KEYCODE_5", up);
        real("long press a", "keyevent --longpress KEYCODE_A", up);
        real("shift b", "keycombination KEYCODE_SHIFT_LEFT KEYCODE_B", up);
        real("ctrl c", "keycombination KEYCODE_CTRL_LEFT KEYCODE_C", up);
        real("alt x", "keycombination KEYCODE_ALT_LEFT KEYCODE_X", up);
        real("shift right", "keyevent KEYCODE_SHIFT_RIGHT", up);
        real("caps lock", "keyevent KEYCODE_CAPS_LOCK", up);
        keyboards("caps lock");
        real("caps lock a", "keyevent KEYCODE_A", up);
        real("caps lock again", "keyevent KEYCODE_CAPS_LOCK", up);
        keyboards("caps lock again");
        real("left", "keyevent KEYCODE_DPAD_LEFT", up);
        real("right", "keyevent KEYCODE_DPAD_RIGHT", up);
        real("up", "keyevent KEYCODE_DPAD_UP", up);
        real("down", "keyevent KEYCODE_DPAD_DOWN", up);
        real("dpad center", "dpad keyevent KEYCODE_DPAD_CENTER", up);
        real("enter", "keyevent KEYCODE_ENTER", up);
        real("numpad enter", "keyevent KEYCODE_NUMPAD_ENTER", up);
        real("tab", "keyevent KEYCODE_TAB", up);
        real("space", "keyevent KEYCODE_SPACE", up);
        real("del", "keyevent KEYCODE_DEL", up);
        real("forward del", "keyevent KEYCODE_FORWARD_DEL", up);
        real("escape", "keyevent KEYCODE_ESCAPE", up);
        real("f1", "keyevent KEYCODE_F1", up);
        real("numpad 5", "keyevent KEYCODE_NUMPAD_5", up);
        real("page down", "keyevent KEYCODE_PAGE_DOWN", up);
        real("star (no scancode)", "keyevent KEYCODE_STAR", up);
        real("zoom in (Android's)", "keyevent KEYCODE_ZOOM_IN", None);
        real("button a", "keyevent KEYCODE_BUTTON_A", up);
        real("gamepad button a", "gamepad keyevent KEYCODE_BUTTON_A", up);
        real("mouse forward", "mouse keyevent KEYCODE_FORWARD", None);
        // (BACK, untrapped, finishes the activity, the mouse's too: from
        // API 33, with the manifest's enableOnBackInvokedCallback, the
        // views don't get it; the trap stays on until the text edit's back
        // key)
        let _ = hints::set(hints::ANDROID_TRAP_BACK_BUTTON, "1");
        real("back, trapped", "keyevent KEYCODE_BACK", up);
        real("mouse back, trapped", "mouse keyevent KEYCODE_BACK", up);
        real("text", "text Hi!", up);
        ask("logcat SDL --grep Unknown keycode");
        keyboards("keys");

        // The screen keyboard: shown
        let area = Rect {
            x: 100,
            y: 200,
            w: 300,
            h: 40,
        };
        t(format!(
            "text input area: {}",
            result(&w.set_text_input_area(Some(&area), 0))
        ));
        start(&w, "text input on", None);

        // Real keys, text input on
        real("on: a", "keyevent KEYCODE_A", up);
        real(
            "on: shift a",
            "keycombination KEYCODE_SHIFT_LEFT KEYCODE_A",
            up,
        );
        real(
            "on: ctrl a",
            "keycombination KEYCODE_CTRL_LEFT KEYCODE_A",
            up,
        );
        real("on: space", "keyevent KEYCODE_SPACE", up);
        real("on: enter", "keyevent KEYCODE_ENTER", up);
        real("on: del", "keyevent KEYCODE_DEL", up);
        real("on: text", "text hello", up);
        state(&w, "on", "shown");

        // What an input method does, through the input connection
        new_ic("ime");
        editable("ime");
        ime_call("commit hello", Op::Commit("hello"));
        ime_call("commit Hi!", Op::Commit("Hi!"));
        ime_call(
            "commit e acute, grinning face",
            Op::Commit("\u{e9}\u{1f600}"),
        );
        ime_call("compose wor", Op::Compose("wor"));
        ime_call("compose worl", Op::Compose("worl"));
        ime_call("compose wox", Op::Compose("wox"));
        ime_call("finish composing", Op::Finish);
        ime_call("delete 1 0", Op::Delete(1, 0));
        ime_call("delete 0 1", Op::Delete(0, 1));
        ime_call("delete 3 1", Op::Delete(3, 1));
        ime_call("compose ab", Op::Compose("ab"));
        ime_call("commit abc", Op::Commit("abc"));
        ime_call("key enter down", Op::Key(ACTION_DOWN, KEYCODE_ENTER));
        ime_call("key enter up", Op::Key(ACTION_UP, KEYCODE_ENTER));
        ime_call("key a down", Op::Key(ACTION_DOWN, KEYCODE_A));
        ime_call("key a up", Op::Key(ACTION_UP, KEYCODE_A));
        ime_call("commit newline", Op::Commit("\n"));
        ime_call("delete 2 0", Op::Delete(2, 0));
        state(&w, "ime", "shown");

        // The return key, SDL_HINT_RETURN_KEY_HIDES_IME on
        let _ = hints::set(hints::RETURN_KEY_HIDES_IME, "1");
        ime_call("hides: commit x newline y", Op::Commit("x\ny"));
        wait_focus(SURFACE);
        settle(None, 1000);
        trace_events("hides: commit x newline y");
        state(&w, "hides: commit x newline y", "hidden");
        start(&w, "hides: again", None);
        new_ic("hides: again");
        ime_call("hides: key enter down", Op::Key(ACTION_DOWN, KEYCODE_ENTER));
        wait_focus(SURFACE);
        settle(None, 1000);
        trace_events("hides: key enter down");
        state(&w, "hides: key enter down", "hidden");
        start(&w, "hides: once more", None);
        real("hides: enter", "keyevent KEYCODE_ENTER", up);
        state(&w, "hides: enter", "any");
        hints::reset(hints::RETURN_KEY_HIDES_IME);

        // The back key with the text edit focused: the keyboard's focus lost
        real("back, text input on", "keyevent KEYCODE_BACK", None);
        wait_focus(SURFACE);
        settle(None, 1000);
        trace_events("back, text input on");
        state(&w, "back, text input on", "hidden");
        let _ = hints::set(hints::ANDROID_TRAP_BACK_BUTTON, "0");

        // The screen keyboard restored with the surface: the text input
        // area moved (the text edit follows it only when it's shown
        // again), then the display turned and back, which changes the
        // surface (its events not traced)
        start(&w, "restore", None);
        let area = Rect {
            x: 50,
            y: 300,
            w: 200,
            h: 30,
        };
        t(format!(
            "restore: text input area: {}",
            result(&w.set_text_input_area(Some(&area), 0))
        ));
        settle(None, 500);
        take_edit("restore: area moved");
        ask("rotate 1");
        settle(None, 2000);
        ask("rotate 0");
        settle(None, 2000);
        clear_events();
        state(&w, "restore", "shown");
        take_edit("restore");

        // Stopped
        stop(&w, "stopped");

        // The text input properties' input types
        ask("ime-with-hard-keyboard no");
        props_case(&w, "none", None);
        props_type(&w, "text", TextInputType::Text);
        props_type(&w, "text name", TextInputType::TextName);
        props_type(&w, "text email", TextInputType::TextEmail);
        props_type(&w, "text username", TextInputType::TextUsername);
        props_type(
            &w,
            "text password hidden",
            TextInputType::TextPasswordHidden,
        );
        props_type(
            &w,
            "text password visible",
            TextInputType::TextPasswordVisible,
        );
        props_type(&w, "number", TextInputType::Number);
        props_type(
            &w,
            "number password hidden",
            TextInputType::NumberPasswordHidden,
        );
        props_type(
            &w,
            "number password visible",
            TextInputType::NumberPasswordVisible,
        );
        props_capitalization(&w, "capitalize none", Capitalization::None);
        props_capitalization(&w, "capitalize sentences", Capitalization::Sentences);
        props_capitalization(&w, "capitalize words", Capitalization::Words);
        props_capitalization(&w, "capitalize letters", Capitalization::Letters);
        props_bool(
            &w,
            "autocorrect off",
            textinput::PROP_TEXTINPUT_AUTOCORRECT_BOOLEAN,
            false,
        );
        props_bool(
            &w,
            "multiline off",
            textinput::PROP_TEXTINPUT_MULTILINE_BOOLEAN,
            false,
        );
        let _ = hints::set(hints::RETURN_KEY_HIDES_IME, "1");
        props_case(&w, "return key hides", None);
        hints::reset(hints::RETURN_KEY_HIDES_IME);
        {
            let p = Properties::new();
            p.set(
                textinput::PROP_TEXTINPUT_TYPE_NUMBER,
                TextInputType::Number as i64,
            )
            .unwrap();
            p.set(textinput::PROP_TEXTINPUT_ANDROID_INPUTTYPE_NUMBER, 0x21i64)
                .unwrap();
            props_case(&w, "android input type 0x21", Some(&p));
        }
        settle(None, 1500);
        clear_events();
        state(&w, "properties", "hidden");
        ask("ime-with-hard-keyboard yes");

        // A trip to the background and back with text input on: the
        // screen keyboard hidden with the app (the window insets tell it,
        // in the background or once back), and so not restored with the
        // surface; the events of the whole trip (when the insets tell it
        // is a matter of timing)
        start(&w, "trip", None);
        ask("background");
        settle(Some(EventType::DID_ENTER_BACKGROUND), 2000);
        ask("foreground");
        settle(Some(EventType::DID_ENTER_FOREGROUND), 2000);
        trace_events("trip");
        state(&w, "trip", "any");
        take_edit("trip");
        stop(&w, "after the trip");
        keyboards("end");

        with_jni(|j| {
            for r in [j.ic, j.edit] {
                if !r.is_null() {
                    jni::delete_global_ref(j.env, r);
                }
            }
            j.ic = std::ptr::null_mut();
            j.edit = std::ptr::null_mut();
        });
        w.destroy();
        settle(None, 200);
        clear_events();
        init::quit();
        t("quit");
        let trace = TRACE.lock().unwrap().clone();
        check_trace(&trace, EXPECTED);
    }
}
