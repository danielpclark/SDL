// Tests of the console keyboard: its tables, and its key handlers on the
// built-in tables without a console (as when SDL isn't run on a virtual
// console), whose text arrives as SDL_EVENT_TEXT_INPUT.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

use super::*;
use crate::events::window::WindowFlags;
use crate::events::Event;
use crate::init::{self, InitFlags};
use crate::video::Window;

// <linux/input-event-codes.h>
const KEY_ESC: c_uint = 1;
const KEY_1: c_uint = 2;
const KEY_E: c_uint = 18;
const KEY_O: c_uint = 24;
const KEY_ENTER: c_uint = 28;
const KEY_LEFTCTRL: c_uint = 29;
const KEY_A: c_uint = 30;
const KEY_S: c_uint = 31;
const KEY_N: c_uint = 49;
const KEY_Q: c_uint = 16;
const KEY_APOSTROPHE: c_uint = 40;
const KEY_GRAVE: c_uint = 41;
const KEY_LEFTSHIFT: c_uint = 42;
const KEY_SPACE: c_uint = 57;
const KEY_CAPSLOCK: c_uint = 58;
const KEY_NUMLOCK: c_uint = 69;
const KEY_KP7: c_uint = 71;
const KEY_KP4: c_uint = 75;
const KEY_KPPLUS: c_uint = 78;
const KEY_KP1: c_uint = 79;
const KEY_102ND: c_uint = 86;
const KEY_KPENTER: c_uint = 96;
const KEY_COMPOSE: c_uint = 127;
const KEY_MUTE: c_uint = 113;
const KEY_PLAYPAUSE: c_uint = 164;

const UP: c_int = 0;
const DOWN: c_int = 1;
const REPEAT: c_int = 2;

#[test]
fn tables_match_upstreams_headers() {
    // (SDL_evdev_kbd_default_keymap.h: default_key_maps, without
    // INCLUDE_EXTENDED_KEYMAP)
    for (i, map) in DEFAULT_KEY_MAPS.iter().enumerate() {
        assert_eq!(map.is_some(), i < 4, "keymap {i}");
    }
    let map = |i: usize| DEFAULT_KEY_MAPS[i].unwrap();
    assert_eq!(map(0)[0], 0xf200);
    assert_eq!(map(0)[KEY_ESC as usize], 0xf01b);
    assert_eq!(map(0)[KEY_1 as usize], 0xf031);
    assert_eq!(map(1)[KEY_1 as usize], 0xf021);
    assert_eq!(map(0)[KEY_A as usize], 0xfb61);
    assert_eq!(map(1)[KEY_A as usize], 0xfb41);
    assert_eq!(map(0)[KEY_ENTER as usize], 0xf201);
    assert_eq!(map(0)[KEY_LEFTCTRL as usize], 0xf702);
    assert_eq!(map(0)[KEY_LEFTSHIFT as usize], 0xf700);
    assert_eq!(map(0)[KEY_CAPSLOCK as usize], 0xfa06);
    assert_eq!(map(0)[KEY_NUMLOCK as usize], 0xf208);
    assert_eq!(map(1)[70], 0xf203);
    assert_eq!(map(0)[KEY_KP7 as usize], 0xf307);
    assert_eq!(map(2)[KEY_KP7 as usize], 0xf911);
    assert_eq!(map(0)[KEY_KPENTER as usize], 0xf30e);
    assert_eq!(map(0)[100], 0xf703);
    assert_eq!(map(3)[0], 0xf27e);
    assert_eq!(map(3)[KEY_102ND as usize], 0x00a6);
    assert_eq!(map(0)[128], 0x0000);
    assert_eq!(map(0)[NR_KEYS - 1], 0x0000);
    assert_eq!(map(3)[NR_KEYS - 1], 0xf200);

    // (SDL_evdev_kbd_default_accents.h: default_accents)
    assert_eq!(DEFAULT_ACCENTS.kb_cnt, 68);
    let entry = |i: usize| {
        let e = DEFAULT_ACCENTS.kbdiacr[i];
        (e.diacr, e.base, e.result)
    };
    assert_eq!(entry(0), (0x60, 0x41, 0xc0));
    assert_eq!(entry(1), (0x60, 0x61, 0xe0));
    assert_eq!(entry(15), (0x61, 0x61, 0xe5));
    assert_eq!(entry(64), (0x73, 0x73, 0xdf));
    assert_eq!(entry(67), (0x69, 0x6a, 0xff));
    assert!((68..MAX_DIACR).all(|i| entry(i) == (0, 0, 0)));
}

fn without_console() -> EvdevKeyboardState {
    EvdevKeyboardState::without_console()
}

/// The built-in keymaps, with dead keys and Compose on keymap 0 (which
/// they have none of): Grave a dead grave, Apostrophe a dead acute (Shift:
/// a dead diaeresis), 102nd a dead tilde (`KT_DEAD2`), Compose the
/// compose key.
fn with_dead_keys() -> &'static [Option<&'static [u16; NR_KEYS]>; MAX_NR_KEYMAPS] {
    let mut plain = *DEFAULT_KEY_MAPS[0].unwrap();
    plain[KEY_GRAVE as usize] = 0xf000 ^ K_DGRAVE;
    plain[KEY_APOSTROPHE as usize] = 0xf000 ^ K_DACUTE;
    plain[KEY_102ND as usize] = 0xf000 ^ k(KT_DEAD2, b'~');
    plain[KEY_COMPOSE as usize] = 0xf000 ^ K_COMPOSE;
    let mut shifted = *DEFAULT_KEY_MAPS[1].unwrap();
    shifted[KEY_APOSTROPHE as usize] = 0xf000 ^ K_DDIERE;
    let mut maps = DEFAULT_KEY_MAPS;
    maps[0] = Some(Box::leak(Box::new(plain)));
    maps[1] = Some(Box::leak(Box::new(shifted)));
    Box::leak(Box::new(maps))
}

/// The dummy video driver with a shown window (which gets the keyboard
/// focus) taking text input, for the text the keyboard sends.
struct TextSession {
    _lock: std::sync::MutexGuard<'static, ()>,
    window: Window,
}

impl TextSession {
    fn new() -> TextSession {
        let lock = crate::test_support::test_lock();
        crate::hints::set(crate::hints::VIDEO_DRIVER, "dummy").unwrap();
        init::init(InitFlags::VIDEO).unwrap();
        let window = Window::create("evdev_kbd", 64, 48, WindowFlags::default()).unwrap();
        window.start_text_input().unwrap();
        while crate::events::poll().is_some() {}
        TextSession {
            _lock: lock,
            window,
        }
    }

    /// Send the keys (key code, value) and return the text events.
    fn keys(&self, kbd: &mut EvdevKeyboardState, keys: &[(c_uint, c_int)]) -> Vec<String> {
        for &(code, value) in keys {
            kbd.keycode(code, value);
        }
        let mut text = Vec::new();
        while let Some(event) = crate::events::poll() {
            if let Event::TextInput(t) = event {
                assert_eq!(t.window_id, self.window.id());
                text.push(t.text);
            }
        }
        text
    }
}

impl Drop for TextSession {
    fn drop(&mut self) {
        self.window.destroy();
        init::quit();
        crate::hints::reset_all();
    }
}

fn press(code: c_uint) -> [(c_uint, c_int); 2] {
    [(code, DOWN), (code, UP)]
}

#[test]
fn shift_and_releases() {
    let s = TextSession::new();
    let mut kbd = without_console();
    assert_eq!(s.keys(&mut kbd, &press(KEY_A)), ["a"]);
    assert_eq!(
        s.keys(&mut kbd, &[(KEY_A, DOWN), (KEY_A, REPEAT), (KEY_A, REPEAT)]),
        ["a", "a", "a"]
    );
    assert_eq!(s.keys(&mut kbd, &[(KEY_A, UP)]), Vec::<String>::new());
    assert_eq!(
        s.keys(
            &mut kbd,
            &[
                (KEY_LEFTSHIFT, DOWN),
                (KEY_A, DOWN),
                (KEY_A, UP),
                (KEY_1, DOWN),
                (KEY_1, UP)
            ]
        ),
        ["A", "!"]
    );
    assert_eq!(kbd.shift_state, 1 << KG_SHIFT);
    // (a repeating Shift changes nothing)
    s.keys(&mut kbd, &[(KEY_LEFTSHIFT, REPEAT)]);
    assert_eq!(kbd.shift_down[usize::from(KG_SHIFT)], 1);
    assert_eq!(
        s.keys(&mut kbd, &[(KEY_LEFTSHIFT, UP)]),
        Vec::<String>::new()
    );
    assert_eq!(kbd.shift_state, 0);
    assert_eq!(s.keys(&mut kbd, &press(KEY_A)), ["a"]);
    // Two Shifts down, one up: still shifted
    assert_eq!(
        s.keys(
            &mut kbd,
            &[
                (KEY_LEFTSHIFT, DOWN),
                (54, DOWN),
                (54, UP),
                (KEY_A, DOWN),
                (KEY_LEFTSHIFT, UP),
                (KEY_A, DOWN)
            ]
        ),
        ["A", "a"]
    );
    // Space, and the keys whose text is a control character (Escape,
    // Enter) or nothing (keysym 0) have no text event; nor does a key past
    // NR_KEYS
    assert_eq!(s.keys(&mut kbd, &press(KEY_SPACE)), [" "]);
    assert_eq!(
        s.keys(
            &mut kbd,
            &[
                (KEY_ESC, DOWN),
                (KEY_ENTER, DOWN),
                (KEY_PLAYPAUSE, DOWN),
                (KEY_MUTE, DOWN),
                (300, DOWN)
            ]
        ),
        Vec::<String>::new()
    );
}

#[test]
fn caps_lock_from_the_led_state() {
    let s = TextSession::new();
    let mut kbd = without_console();
    // (as KDGKBLED reports it on a console)
    kbd.ledflagstate = K_CAPSLOCK;
    assert_eq!(s.keys(&mut kbd, &press(KEY_A)), ["A"]);
    // (only letters)
    assert_eq!(s.keys(&mut kbd, &press(KEY_1)), ["1"]);
    assert_eq!(
        s.keys(
            &mut kbd,
            &[
                (KEY_LEFTSHIFT, DOWN),
                (KEY_A, DOWN),
                (KEY_1, DOWN),
                (KEY_LEFTSHIFT, UP)
            ]
        ),
        ["a", "!"]
    );
    kbd.ledflagstate = 0;
    assert_eq!(s.keys(&mut kbd, &press(KEY_A)), ["a"]);
}

#[test]
fn built_in_lock_and_modifier_keys() {
    let s = TextSession::new();
    let mut kbd = without_console();
    // The built-in Caps Lock is CtrlL_Lock: its keymap (0x40) isn't
    // built in, so the next key event (its own release, here) resets the
    // shift and lock state, and has no text
    assert_eq!(
        s.keys(&mut kbd, &[(KEY_CAPSLOCK, DOWN)]),
        Vec::<String>::new()
    );
    assert_eq!(kbd.lockstate, 1 << KG_CTRLL);
    assert_eq!(s.keys(&mut kbd, &[(KEY_A, DOWN)]), Vec::<String>::new());
    assert_eq!(kbd.lockstate, 0);
    assert_eq!(
        s.keys(&mut kbd, &[(KEY_A, DOWN), (KEY_CAPSLOCK, UP)]),
        ["a"]
    );
    assert_eq!(
        s.keys(&mut kbd, &[(KEY_CAPSLOCK, DOWN), (KEY_CAPSLOCK, UP)]),
        Vec::<String>::new()
    );
    assert_eq!(kbd.lockstate, 0);
    assert_eq!(s.keys(&mut kbd, &press(KEY_A)), ["a"]);
    // Control (keymap 4) and Alt (keymap 8) likewise, held down
    assert_eq!(
        s.keys(&mut kbd, &[(KEY_LEFTCTRL, DOWN), (KEY_A, DOWN)]),
        Vec::<String>::new()
    );
    assert_eq!(kbd.shift_state, 0);
    assert_eq!(
        s.keys(&mut kbd, &[(KEY_A, DOWN), (KEY_LEFTCTRL, UP)]),
        ["a"]
    );
    assert_eq!(
        s.keys(
            &mut kbd,
            &[(56, DOWN), (KEY_A, DOWN), (KEY_A, DOWN), (56, UP)]
        ),
        ["a"]
    );
}

#[test]
fn numeric_pad() {
    let s = TextSession::new();
    let mut kbd = without_console();
    // Without Num Lock: nothing
    assert_eq!(s.keys(&mut kbd, &press(KEY_KP7)), Vec::<String>::new());
    s.keys(&mut kbd, &press(KEY_NUMLOCK));
    assert_eq!(kbd.ledflagstate, K_NUMLOCK);
    // (a repeating Num Lock doesn't toggle it)
    s.keys(
        &mut kbd,
        &[
            (KEY_NUMLOCK, DOWN),
            (KEY_NUMLOCK, REPEAT),
            (KEY_NUMLOCK, REPEAT),
        ],
    );
    assert_eq!(kbd.ledflagstate, 0);
    s.keys(
        &mut kbd,
        &[(KEY_NUMLOCK, UP), (KEY_NUMLOCK, DOWN), (KEY_NUMLOCK, UP)],
    );
    assert_eq!(kbd.ledflagstate, K_NUMLOCK);
    assert_eq!(
        s.keys(
            &mut kbd,
            &[press(KEY_KP7), press(KEY_KPPLUS), press(KEY_KP1)].concat()
        ),
        ["7", "+", "1"]
    );
    // (the pad's Enter is a carriage return: a control character)
    assert_eq!(s.keys(&mut kbd, &press(KEY_KPENTER)), Vec::<String>::new());
    s.keys(&mut kbd, &press(KEY_NUMLOCK));
    assert_eq!(s.keys(&mut kbd, &press(KEY_KP7)), Vec::<String>::new());
}

#[test]
fn unicode_and_codes_typed_on_the_pad() {
    let s = TextSession::new();
    let mut kbd = without_console();
    // AltGr (keymap 2), as TIOCLINUX reports it on a console; then Shift
    // (keymap 3), whose 102nd key is U+00A6
    kbd.shift_state = 1 << KG_ALTGR;
    assert_eq!(
        s.keys(
            &mut kbd,
            &[
                (KEY_LEFTSHIFT, DOWN),
                (KEY_102ND, DOWN),
                (KEY_102ND, UP),
                (KEY_LEFTSHIFT, UP)
            ]
        ),
        ["\u{a6}"]
    );
    // AltGr's pad is hexadecimal input (KT_ASCII): 4, 1, then a shift
    // key's release sends the code typed
    assert_eq!(
        s.keys(&mut kbd, &[press(KEY_KP4), press(KEY_KP1)].concat()),
        Vec::<String>::new()
    );
    assert_eq!(kbd.npadch, 0x41);
    assert_eq!(
        s.keys(&mut kbd, &[(KEY_LEFTSHIFT, DOWN)]),
        Vec::<String>::new()
    );
    assert_eq!(s.keys(&mut kbd, &[(KEY_LEFTSHIFT, UP)]), ["A"]);
    assert_eq!(kbd.npadch, -1);
    // A code past U+FFFF, and a surrogate (which has no text)
    kbd.npadch = 0x1f600;
    assert_eq!(
        s.keys(&mut kbd, &[(KEY_LEFTSHIFT, DOWN), (KEY_LEFTSHIFT, UP)]),
        ["\u{1f600}"]
    );
    kbd.npadch = 0xd800;
    assert_eq!(
        s.keys(&mut kbd, &[(KEY_LEFTSHIFT, DOWN), (KEY_LEFTSHIFT, UP)]),
        Vec::<String>::new()
    );
}

#[test]
fn dead_keys_and_compose() {
    let s = TextSession::new();
    let mut kbd = without_console();
    kbd.key_maps = with_dead_keys();
    // A dead key and a letter of the accent table
    assert_eq!(
        s.keys(&mut kbd, &[press(KEY_GRAVE), press(KEY_A)].concat()),
        ["\u{e0}"]
    );
    assert_eq!(
        s.keys(&mut kbd, &[press(KEY_APOSTROPHE), press(KEY_E)].concat()),
        ["\u{e9}"]
    );
    assert_eq!(
        s.keys(
            &mut kbd,
            &[
                (KEY_LEFTSHIFT, DOWN),
                (KEY_APOSTROPHE, DOWN),
                (KEY_APOSTROPHE, UP),
                (KEY_LEFTSHIFT, UP),
                (KEY_O, DOWN)
            ]
        ),
        ["\u{f6}"]
    );
    // KT_DEAD2: the accent itself
    assert_eq!(
        s.keys(&mut kbd, &[press(KEY_102ND), press(KEY_N)].concat()),
        ["\u{f1}"]
    );
    // A space, or the dead key again: the accent alone
    assert_eq!(
        s.keys(&mut kbd, &[press(KEY_GRAVE), press(KEY_SPACE)].concat()),
        ["`"]
    );
    assert_eq!(
        s.keys(&mut kbd, &[press(KEY_GRAVE), press(KEY_GRAVE)].concat()),
        Vec::<String>::new()
    );
    assert_eq!(kbd.diacr, c_uint::from(b'`'));
    assert_eq!(s.keys(&mut kbd, &press(KEY_E)), ["\u{e8}"]);
    // A letter that doesn't combine: the accent, then the letter
    assert_eq!(
        s.keys(&mut kbd, &[press(KEY_GRAVE), press(KEY_Q)].concat()),
        ["`q"]
    );
    // Enter after a dead key: the accent
    assert_eq!(
        s.keys(&mut kbd, &[press(KEY_GRAVE), press(KEY_ENTER)].concat()),
        ["`"]
    );
    assert_eq!(kbd.diacr, 0);
    // Compose: the next character is the accent
    assert_eq!(
        s.keys(
            &mut kbd,
            &[press(KEY_COMPOSE), press(KEY_S), press(KEY_S)].concat()
        ),
        ["\u{df}"]
    );
    assert_eq!(
        s.keys(
            &mut kbd,
            &[press(KEY_COMPOSE), press(KEY_A), press(KEY_Q)].concat()
        ),
        ["aq"]
    );
}

#[test]
fn muting_installs_the_emergency_restore() {
    let _lock = crate::test_support::test_lock();
    let handler = |sig: c_int| {
        // SAFETY: reading the current action.
        unsafe {
            let mut action: libc::sigaction = std::mem::zeroed();
            libc::sigaction(sig, std::ptr::null(), &mut action);
            action.sa_sigaction
        }
    };
    let before: Vec<_> = FATAL_SIGNALS.iter().map(|&sig| handler(sig)).collect();

    let mut kbd = without_console();
    kbd.set_muted(true);
    assert!(kbd.muted);
    assert!(KBD_CLEANUP_STATE.active.load(Ordering::SeqCst));
    assert!(KBD_CLEANUP_ATEXIT_INSTALLED.load(Ordering::SeqCst));
    for (&sig, &old) in FATAL_SIGNALS.iter().zip(&before) {
        // (Rust's runtime ignores SIGPIPE: that one is left alone)
        if (sig == libc::SIGHUP || sig == libc::SIGPIPE) && old != libc::SIG_DFL {
            assert_eq!(handler(sig), old, "signal {sig}");
        } else {
            assert_eq!(
                handler(sig),
                cleanup_signal_action_address(),
                "signal {sig}"
            );
        }
    }
    // (again: nothing changes)
    kbd.set_muted(true);
    kbd.set_muted(false);
    assert!(!kbd.muted);
    assert!(!KBD_CLEANUP_STATE.active.load(Ordering::SeqCst));
    let after: Vec<_> = FATAL_SIGNALS.iter().map(|&sig| handler(sig)).collect();
    assert_eq!(after, before);

    // Not with SDL_HINT_NO_SIGNAL_HANDLERS, nor with
    // SDL_HINT_MUTE_CONSOLE_KEYBOARD off (which doesn't mute either)
    crate::hints::set(crate::hints::NO_SIGNAL_HANDLERS, "1").unwrap();
    kbd.set_muted(true);
    assert!(!KBD_CLEANUP_STATE.active.load(Ordering::SeqCst));
    assert_eq!(handler(libc::SIGSEGV), before[5]);
    kbd.set_muted(false);
    crate::hints::reset_all();
    crate::hints::set(crate::hints::MUTE_CONSOLE_KEYBOARD, "0").unwrap();
    kbd.set_muted(true);
    assert!(kbd.muted);
    assert!(!KBD_CLEANUP_STATE.active.load(Ordering::SeqCst));
    kbd.set_muted(false);
    crate::hints::reset_all();
}

#[test]
fn vt_switch_signals_without_a_console() {
    let _lock = crate::test_support::test_lock();
    // Without a console, VT_SETMODE fails: the signals are given back
    assert!(!kbd_vt_init(-1));
    assert_eq!(VT_RELEASE_SIGNAL.load(Ordering::SeqCst), 0);
    assert_eq!(VT_ACQUIRE_SIGNAL.load(Ordering::SeqCst), 0);
    let first = libc::SIGRTMIN() + 2;
    // SAFETY: reading the current action.
    let current = unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        libc::sigaction(first, std::ptr::null(), &mut action);
        action.sa_sigaction
    };
    assert_eq!(current, libc::SIG_DFL);

    // The signal handlers, raised: the callbacks run from update() (with
    // VT_RELDISP on no console)
    let release = find_free_signal(kbd_vt_release_signal_action);
    let acquire = find_free_signal(kbd_vt_acquire_signal_action);
    assert_eq!((release, acquire), (first, first + 1));
    VT_RELEASE_SIGNAL.store(release, Ordering::SeqCst);
    VT_ACQUIRE_SIGNAL.store(acquire, Ordering::SeqCst);
    let calls = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut kbd = without_console();
    let (r, a) = (calls.clone(), calls.clone());
    kbd.set_vt_switch_callbacks(
        Some(Box::new(move || r.lock().unwrap().push("release"))),
        Some(Box::new(move || a.lock().unwrap().push("acquire"))),
    );
    kbd.update();
    assert!(calls.lock().unwrap().is_empty());
    for (sig, name) in [(release, "release"), (acquire, "acquire")] {
        // SAFETY: the handler only stores an atomic.
        unsafe {
            libc::raise(sig);
        }
        kbd.update();
        assert_eq!(calls.lock().unwrap().pop(), Some(name));
        assert_eq!(VT_SIGNAL_PENDING.load(Ordering::SeqCst), VT_SIGNAL_NONE);
    }
    kbd.set_vt_switch_callbacks(None, None);
    kbd_vt_quit(-1);
    assert_eq!(VT_RELEASE_SIGNAL.load(Ordering::SeqCst), 0);
    // SAFETY: reading the current action.
    let current = unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        libc::sigaction(release, std::ptr::null(), &mut action);
        action.sa_sigaction
    };
    assert_eq!(current, libc::SIG_DFL);
}

/// What `the_emergency_restore_hands_faults_on` runs in a child (this test
/// binary again, with this variable set).
const FAULT_CHILD: &str = "SDL3_TEST_EVDEV_KBD_FAULT";

/// The handler installed before SDL's in the `chained` child: whether the
/// keyboard was restored first, and whether the SIGSEGV it gets is the
/// fault's own; then the default action (the fault, made again by the
/// return, ends the child).
extern "C" fn previous_segv_handler(sig: c_int, info: *mut libc::siginfo_t, _context: *mut c_void) {
    // SAFETY: the kernel's siginfo; write and sigaction are
    // async-signal-safe.
    unsafe {
        let restored = !KBD_CLEANUP_STATE.active.load(Ordering::SeqCst);
        let own = (*info).si_code == 1 /* SEGV_MAPERR */ && (*info).si_addr().addr() == 16;
        let line: &[u8] = match (restored, own) {
            (true, true) => b"previous handler: restored, the fault's own siginfo\n",
            (true, false) => b"previous handler: restored, another siginfo\n",
            (false, _) => b"previous handler: not restored\n",
        };
        libc::write(2, line.as_ptr().cast(), line.len());
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = libc::SIG_DFL;
        libc::sigaction(sig, &action, std::ptr::null_mut());
    }
}

/// A recursion that overflows the stack.
fn recurse(depth: u64) -> u64 {
    let frame = std::hint::black_box([depth; 512]);
    if frame[0] == u64::MAX {
        return 0;
    }
    recurse(frame[1] + 1) + frame[2]
}

/// The emergency restore's handler, with the console muted, hands a fault
/// on to the handler it replaced with the fault's own siginfo (`Note
/// (upstream)` in `kbd_cleanup_signal_action()`): a handler of the
/// program's sees the faulting address, after the restore; Rust std's tells
/// a stack overflow (and aborts, through SDL's handler of SIGABRT); a
/// SIGABRT is raised again, as upstream. In children (this test binary
/// again), as the faults end them.
#[test]
fn the_emergency_restore_hands_faults_on() {
    let name = format!(
        "{}::the_emergency_restore_hands_faults_on",
        module_path!().split_once("::").unwrap().1
    );
    if let Ok(what) = std::env::var(FAULT_CHILD) {
        if what == "chained" {
            // SAFETY: the handler is async-signal-safe and doesn't unwind.
            unsafe {
                let mut action: libc::sigaction = std::mem::zeroed();
                action.sa_sigaction = previous_segv_handler
                    as extern "C" fn(c_int, *mut libc::siginfo_t, *mut c_void)
                    as libc::sighandler_t;
                action.sa_flags = libc::SA_SIGINFO;
                libc::sigaction(libc::SIGSEGV, &action, std::ptr::null_mut());
            }
        }
        let mut kbd = without_console();
        kbd.set_muted(true);
        assert!(KBD_CLEANUP_STATE.active.load(Ordering::SeqCst));
        match what.as_str() {
            // SAFETY: the fault is the point.
            "chained" | "segv" => unsafe {
                std::ptr::without_provenance_mut::<u8>(16).write_volatile(1);
            },
            "stack-overflow" => {
                std::hint::black_box(recurse(0));
            }
            "abort" => std::process::abort(),
            _ => unreachable!("{what}"),
        }
        unreachable!("the child {what} is still running");
    }

    let exe = std::env::current_exe().unwrap();
    for (what, signal, stderr_has) in [
        (
            "chained",
            libc::SIGSEGV,
            "previous handler: restored, the fault's own siginfo\n",
        ),
        ("segv", libc::SIGSEGV, ""),
        ("stack-overflow", libc::SIGABRT, "has overflowed its stack"),
        ("abort", libc::SIGABRT, ""),
    ] {
        use std::os::unix::process::ExitStatusExt;
        let output = std::process::Command::new(&exe)
            .args([name.as_str(), "--exact", "--nocapture", "--test-threads=1"])
            .env(FAULT_CHILD, what)
            .env_remove("SDL_NO_SIGNAL_HANDLERS")
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(
            output.status.signal(),
            Some(signal),
            "{what}: {:?}, stderr {stderr:?}",
            output.status
        );
        assert!(stderr.contains(stderr_has), "{what}: stderr {stderr:?}");
    }
}
