// Tests of the console input reader on a virtual console: its keymap, its
// keyboard mute and the restore of it, its VT switches.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! A program (`harness = false`), run as root by `tools/linux_console_run.py
//! --vt 3` on the virtual console tty3 (its controlling terminal and stdin,
//! made the active console), on the dummy video driver's `evdev` variant
//! (`SDL_VIDEO_DRIVER=evdev`), tracing tty3's keyboard and VT modes (asked
//! of the runner: `vt modes`) and the events of a uinput keyboard:
//!
//! * the console keyboard muted (`K_OFF`) while a keyboard is there (the
//!   machine's own keyboards count too: the runner's has one), and restored
//!   on quit, with the VT in `VT_PROCESS` while SDL runs;
//! * keys and their text, through `KDGKBENT` from tty3's keymap (letters,
//!   Shift, Caps Lock, the pad with Num Lock, AltGr, Control and Alt,
//!   Compose with the console's accent table);
//! * a switch away from tty3, which waits for SDL's `VT_RELDISP` from the
//!   event pump, and back (`VT_ACKACQ`);
//! * the keyboard unplugged, then SDL quit;
//! * with `SDL_MUTE_CONSOLE_KEYBOARD=0`: not muted, the same keys;
//! * children of this program (`--child WHAT`, tty3 their controlling
//!   terminal too) with the console muted that end without `SDL_Quit()`:
//!   killed by `SIGABRT` (`abort()`), `SIGSEGV` (a write to an unmapped
//!   address) and `SIGBUS` (a read past a mapped file's end), or by
//!   `SIGSEGV` with a handler of their own installed before SDL's
//!   (`chained`: which SDL's hands the fault on to), or calling `exit()`:
//!   the keyboard mode is restored each time (the emergency handlers, and
//!   the `atexit()` one), and the VT is left in `VT_PROCESS` for the dead
//!   process (as upstream: the kernel gives the VT back at the next
//!   switch, which a switch here shows); with `SDL_NO_SIGNAL_HANDLERS`,
//!   `abort()` leaves the keyboard muted (the runner restores it);
//! * SDL started and quit again, which leaves the modes as they were.
//!
//! The expected trace is upstream's C SDL's, from the same program in C
//! (upstream built with CMake at ea7a2da, with the fix of the emergency
//! handler marked `Note (upstream)` in `evdev_kbd.rs`, run by the same
//! runner on the same CI runner, tty3 and keymap). The Rust program also
//! checks, apart from the trace (lines `rust: `), a child that overflows
//! its stack: Rust std's SIGSEGV handler, which SDL's chains to, reports
//! the overflow and aborts, and the keyboard is restored.
//!
//! Without the runner, it runs itself under it as root when tty3 and
//! /dev/uinput are there, and skips (capabilities `vt`, `uinput`)
//! otherwise, unless `SDL3_TEST_REQUIRE` asks for them. Elsewhere than
//! Linux it does nothing.

#[cfg(not(target_os = "linux"))]
fn main() {}

#[cfg(target_os = "linux")]
fn main() -> std::process::ExitCode {
    app::main()
}

#[cfg(target_os = "linux")]
mod app {
    use std::collections::HashMap;
    use std::ffi::c_int;
    use std::io::{BufRead, BufReader, Write};
    use std::os::fd::FromRawFd;
    use std::os::unix::process::ExitStatusExt;
    use std::process::{Command, ExitCode, ExitStatus, Stdio};
    use std::sync::{Mutex, OnceLock};
    use std::time::{Duration, Instant};

    use sdl3::events::keyboard;
    use sdl3::events::window::WindowFlags;
    use sdl3::events::{self, Event, EventType};
    use sdl3::hints;
    use sdl3::init::{self, InitFlags};
    use sdl3::video::{self, Window};

    /// Upstream's trace (the C program, by the runner, on the CI runner).
    const EXPECTED: &str = "\
";

    const PREFIX: &str = "sdl3-console-runner: ";
    /// `K_COMPOSE` of `<linux/keyboard.h>`
    const K_COMPOSE: u16 = 0x020e;
    const KEYBOARD: &str = "SDL3 evdev vt keyboard";
    /// The console the runner starts the program on
    const VT: &str = "3";
    /// The console switched to and back from
    const OTHER_VT: &str = "1";

    /// Whether `SDL3_TEST_REQUIRE` makes a capability mandatory.
    fn required(capability: &str) -> bool {
        std::env::var("SDL3_TEST_REQUIRE")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .any(|c| c == "all" || c == capability)
    }

    /// Skip (all of) the program, as the library's tests do.
    fn skip(capability: &str, reason: &str) -> ExitCode {
        assert!(
            !required(capability),
            "required capability {capability} unavailable: {reason}"
        );
        eprintln!("note: skipping, {capability} unavailable: {reason}");
        if let Ok(path) = std::env::var("SDL3_TEST_SKIP_LOG") {
            if let Ok(mut file) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
            {
                let _ = writeln!(file, "{capability}\tevdev_vt\t{reason}");
            }
        }
        ExitCode::SUCCESS
    }

    /// The runner's answers: the pipe it gives (stdin is the console).
    fn replies() -> &'static Mutex<BufReader<std::fs::File>> {
        static REPLIES: OnceLock<Mutex<BufReader<std::fs::File>>> = OnceLock::new();
        REPLIES.get_or_init(|| {
            let fd: c_int = std::env::var("SDL3_CONSOLE_REPLY_FD")
                .expect("SDL3_CONSOLE_REPLY_FD (tools/linux_console_run.py --vt)")
                .parse()
                .expect("a descriptor");
            // SAFETY: the runner's pipe, inherited, which nothing else owns
            // (left open across exec, for the children).
            Mutex::new(BufReader::new(unsafe { std::fs::File::from_raw_fd(fd) }))
        })
    }

    /// Ask the runner, and return its answer's lines.
    fn request(line: &str) -> Vec<String> {
        println!("{PREFIX}request {line}");
        std::io::stdout().flush().unwrap();
        let mut replies = replies().lock().unwrap();
        let mut lines = Vec::new();
        loop {
            let mut answer = String::new();
            if replies.read_line(&mut answer).expect("the runner's answer") == 0 {
                panic!("the runner ended without answering {line:?}");
            }
            let answer = answer.trim_end_matches('\n');
            if answer == format!("{PREFIX}done") {
                return lines;
            }
            if let Some(reply) = answer.strip_prefix(&format!("{PREFIX}reply ")) {
                lines.push(reply.to_owned());
            }
        }
    }

    /// A trace line, printed (by a child too: its parent waits for it).
    fn print_line(line: &str) {
        println!("trace: {line}");
        std::io::stdout().flush().unwrap();
    }

    /// The trace, printed as it's made.
    struct Trace {
        lines: Vec<String>,
        /// The names of the keyboards by ID (kept after they're removed)
        keyboards: HashMap<u32, String>,
        window: events::WindowID,
    }

    impl Trace {
        fn line(&mut self, line: String) {
            print_line(&line);
            self.lines.push(line);
        }

        /// The answer's lines, traced.
        fn ask(&mut self, line: &str) {
            for reply in request(line) {
                self.line(reply);
            }
        }

        fn ours(name: &str) -> bool {
            name.starts_with("SDL3 evdev")
        }

        fn window(&self, id: events::WindowID) -> &'static str {
            if id == self.window {
                "window"
            } else if id == 0 {
                "none"
            } else {
                "another"
            }
        }

        /// The event's line, if it's one of the traced ones (of this
        /// keyboard: the machine's own may be read too, as root).
        fn describe(&mut self, event: &Event) -> Option<String> {
            match event {
                Event::KeyboardDevice(d) => {
                    let added = d.event_type == EventType::KEYBOARD_ADDED;
                    if added {
                        let name = keyboard::keyboard_name(d.which).unwrap_or_default();
                        self.keyboards.insert(d.which, name);
                    }
                    let name = self.keyboards.get(&d.which).cloned().unwrap_or_default();
                    Self::ours(&name).then(|| {
                        format!(
                            "keyboard {}: {name}",
                            if added { "added" } else { "removed" }
                        )
                    })
                }
                Event::Key(k) => {
                    let name = self.keyboards.get(&k.which)?;
                    Self::ours(name).then(|| {
                        format!(
                            "key {} {} key {} raw {} mod 0x{:04x} repeat {} {}",
                            if k.down { "down" } else { "up" },
                            k.scancode.name(),
                            k.key.name(),
                            k.raw,
                            k.modifiers.0,
                            k.repeat as i32,
                            self.window(k.window_id),
                        )
                    })
                }
                Event::TextInput(t) => {
                    Some(format!("text \"{}\" {}", t.text, self.window(t.window_id)))
                }
                _ => None,
            }
        }

        /// Pump once, and trace the events (all of a request's events are
        /// read by then).
        fn pump(&mut self) {
            events::pump();
            while let Some(event) = events::poll() {
                if let Some(line) = self.describe(&event) {
                    self.line(line);
                }
            }
        }

        /// Pump until the line comes (the hotplug events, through udev's
        /// monitor), tracing the events, for up to 10 s.
        fn wait_for(&mut self, line: &str) {
            let end = Instant::now() + Duration::from_secs(10);
            loop {
                events::pump();
                while let Some(event) = events::poll() {
                    if let Some(l) = self.describe(&event) {
                        let done = l == line;
                        self.line(l);
                        if done {
                            return;
                        }
                    }
                }
                if Instant::now() >= end {
                    self.line(format!("timeout: no {line:?} in 10 s"));
                    return;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }

        /// A request of the keyboard's events (shell words: TYPE CODE VALUE
        /// ...), then the events they make.
        fn keys(&mut self, events: &str) {
            for reply in request(&format!("uinput events '{KEYBOARD}' {events}")) {
                if !reply.starts_with("uinput: sent ") {
                    self.line(reply);
                }
            }
            self.pump();
        }

        /// The keys typed, one request (and one pump) a line: their events
        /// and text.
        fn typing(&mut self) {
            self.keys(&format!(
                "{} {} {}",
                tap("KEY_A"),
                tap("KEY_1"),
                tap("KEY_SPACE")
            ));
            self.keys(&held(
                "KEY_LEFTSHIFT",
                &["KEY_A", "KEY_1", "KEY_APOSTROPHE", "KEY_SLASH", "KEY_102ND"],
            ));
            self.keys(&format!(
                "{} {} {} {}",
                tap("KEY_GRAVE"),
                tap("KEY_BACKSLASH"),
                tap("KEY_102ND"),
                tap("KEY_SEMICOLON")
            ));
            self.keys(&format!(
                "{} {} {} {}",
                tap("KEY_CAPSLOCK"),
                tap("KEY_A"),
                held("KEY_LEFTSHIFT", &["KEY_A"]),
                tap("KEY_CAPSLOCK")
            ));
            self.keys(&format!(
                "{} {} {} {} {}",
                tap("KEY_KP7"),
                tap("KEY_NUMLOCK"),
                tap("KEY_KP7"),
                tap("KEY_KPDOT"),
                tap("KEY_NUMLOCK")
            ));
            self.keys(&held("KEY_RIGHTALT", &["KEY_E", "KEY_8", "KEY_102ND"]));
            self.keys(&format!(
                "EV_KEY KEY_RIGHTALT 1 EV_SYN SYN_REPORT 0 {} EV_KEY KEY_RIGHTALT 0 EV_SYN SYN_REPORT 0",
                held("KEY_LEFTSHIFT", &["KEY_A"])
            ));
            // (not Control+C: on a console that isn't muted, tty3's line
            // discipline would make it a SIGINT of the program)
            self.keys(&held("KEY_LEFTCTRL", &["KEY_B"]));
            self.keys(&held("KEY_LEFTALT", &["KEY_A"]));
            self.keys(&format!(
                "{} {} {} {}",
                tap("KEY_COMPOSE"),
                tap("KEY_APOSTROPHE"),
                tap("KEY_E"),
                tap("KEY_B")
            ));
        }

        /// Init SDL's video (the evdev variant of the dummy driver), and
        /// its window with text input; false when it fails.
        fn init(&mut self) -> Option<Window> {
            hints::set(hints::VIDEO_DRIVER, "evdev").unwrap();
            if let Err(e) = init::init(InitFlags::VIDEO) {
                self.line(format!("init: {e}"));
                return None;
            }
            let driver = video::current_video_driver().unwrap_or("(none)");
            self.line(format!("init: ok ({driver})"));
            let window = Window::create("evdev_vt", 1024, 768, WindowFlags::default()).unwrap();
            self.window = window.id();
            window.start_text_input().unwrap();

            // The keyboards found (sorted: udev lists them by their paths),
            // and how many of the machine's
            let mut found = Vec::new();
            while let Some(event) = events::poll() {
                if let Some(line) = self.describe(&event) {
                    found.push(line);
                }
            }
            found.sort();
            for line in found {
                self.line(line);
            }
            let others = keyboard::keyboards()
                .into_iter()
                .filter(|&id| !Self::ours(&keyboard::keyboard_name(id).unwrap_or_default()))
                .count();
            self.line(format!("other keyboards: {others}"));
            Some(window)
        }

        fn quit(&mut self, window: Window) {
            window.destroy();
            init::quit();
            self.line("quit".into());
        }

        /// A child (this program, `--child WHAT`), waited for: how it
        /// ended.
        fn child(&mut self, what: &str, env: &[(&str, &str)]) {
            self.line(format!("-- child {what}"));
            let status = Command::new(std::env::current_exe().unwrap())
                .args(["--child", what])
                .envs(env.iter().copied())
                .status()
                .expect("the child runs");
            self.line(format!("child {what}: {}", ended(status)));
            self.ask("vt modes");
        }
    }

    /// How a process ended.
    fn ended(status: ExitStatus) -> String {
        match (status.code(), status.signal()) {
            (Some(code), _) => format!("exited with {code}"),
            (None, Some(sig)) => format!("killed by {}", signal_name(sig)),
            _ => format!("{status}"),
        }
    }

    fn signal_name(sig: c_int) -> String {
        match sig {
            libc::SIGABRT => "SIGABRT".into(),
            libc::SIGSEGV => "SIGSEGV".into(),
            libc::SIGBUS => "SIGBUS".into(),
            libc::SIGKILL => "SIGKILL".into(),
            _ => format!("signal {sig}"),
        }
    }

    /// A key's press and release (and SYN_REPORT after each).
    fn tap(key: &str) -> String {
        format!("EV_KEY {key} 1 EV_SYN SYN_REPORT 0 EV_KEY {key} 0 EV_SYN SYN_REPORT 0")
    }

    /// Keys tapped with a modifier held.
    fn held(modifier: &str, keys: &[&str]) -> String {
        let keys: Vec<String> = keys.iter().map(|k| tap(k)).collect();
        format!(
            "EV_KEY {modifier} 1 EV_SYN SYN_REPORT 0 {} EV_KEY {modifier} 0 EV_SYN SYN_REPORT 0",
            keys.join(" ")
        )
    }

    /// The handler of `--child chained`, installed before SDL's: which
    /// `SIGSEGV` it gets (the fault's own, or one raised), then the default
    /// action (the fault, made again by the return, ends the child).
    extern "C" fn previous_handler(
        sig: c_int,
        info: *mut libc::siginfo_t,
        _context: *mut std::ffi::c_void,
    ) {
        // SAFETY: the kernel's siginfo; write and sigaction are
        // async-signal-safe.
        unsafe {
            let code = (*info).si_code;
            let line: &[u8] = if code == 1 /* SEGV_MAPERR */ && (*info).si_addr().addr() == 16 {
                b"trace: child: the previous handler: SIGSEGV at the fault's address\n"
            } else if code == libc::SI_TKILL {
                b"trace: child: the previous handler: SIGSEGV raised again\n"
            } else {
                b"trace: child: the previous handler: SIGSEGV from elsewhere\n"
            };
            libc::write(1, line.as_ptr().cast(), line.len());
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

    /// `--child WHAT`: SDL started with the console muted (the keyboard is
    /// there), then the end WHAT, without `SDL_Quit()`.
    fn child(what: &str) -> ExitCode {
        // (no core files)
        let no_core = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        // SAFETY: setrlimit reads the limit.
        unsafe {
            libc::setrlimit(libc::RLIMIT_CORE, &no_core);
        }
        if what == "chained" {
            // SAFETY: the handler is async-signal-safe and doesn't unwind.
            unsafe {
                let mut action: libc::sigaction = std::mem::zeroed();
                action.sa_sigaction = previous_handler
                    as extern "C" fn(c_int, *mut libc::siginfo_t, *mut std::ffi::c_void)
                    as libc::sighandler_t;
                action.sa_flags = libc::SA_SIGINFO;
                libc::sigaction(libc::SIGSEGV, &action, std::ptr::null_mut());
            }
        }
        hints::set(hints::VIDEO_DRIVER, "evdev").unwrap();
        match init::init(InitFlags::VIDEO) {
            Ok(()) => print_line("child: init: ok"),
            Err(e) => {
                print_line(&format!("child: init: {e}"));
                return ExitCode::FAILURE;
            }
        }
        for reply in request("vt modes") {
            print_line(&format!("child: {reply}"));
        }
        // SAFETY: the faults are the point; nothing runs after them.
        unsafe {
            match what {
                "abort" | "no-signal-handlers" => libc::abort(),
                "segv" | "chained" => {
                    std::ptr::without_provenance_mut::<u8>(16).write_volatile(1);
                }
                "bus" => {
                    let fd = libc::memfd_create(c"evdev_vt".as_ptr(), 0);
                    assert!(fd >= 0);
                    assert_eq!(libc::ftruncate(fd, 4096), 0);
                    let map = libc::mmap(
                        std::ptr::null_mut(),
                        4096,
                        libc::PROT_READ,
                        libc::MAP_SHARED,
                        fd,
                        0,
                    );
                    assert_ne!(map, libc::MAP_FAILED);
                    assert_eq!(libc::ftruncate(fd, 0), 0);
                    std::hint::black_box(map.cast::<u8>().read_volatile());
                }
                "exit" => libc::exit(0),
                "stack-overflow" => {
                    std::hint::black_box(recurse(0));
                }
                _ => panic!("unknown child {what:?}"),
            }
        }
        print_line(&format!("child: {what}: still running"));
        ExitCode::FAILURE
    }

    /// The Rust-only check: a stack overflow in a child, whose SIGSEGV
    /// SDL's handler hands on to Rust std's (which tells the overflow by
    /// the fault's address).
    fn stack_overflow() -> Result<(), String> {
        let output = Command::new(std::env::current_exe().unwrap())
            .args(["--child", "stack-overflow"])
            .stdout(Stdio::inherit())
            .stderr(Stdio::piped())
            .output()
            .expect("the child runs");
        let stderr = String::from_utf8_lossy(&output.stderr);
        let how = ended(output.status);
        println!("rust: child stack-overflow: {how}");
        let modes = request("vt modes");
        println!("rust: {modes:?}");
        if how != "killed by SIGABRT" || !stderr.contains("has overflowed its stack") {
            return Err(format!(
                "the stack overflow wasn't std's: {how}, stderr {stderr:?}"
            ));
        }
        if modes.first().map(String::as_str) != Some("vt: keyboard K_UNICODE mode VT_PROCESS") {
            return Err(format!("the keyboard wasn't restored: {modes:?}"));
        }
        Ok(())
    }

    /// Whether the controlling terminal is a virtual console (as the
    /// console keyboard finds it: `KDGKBTYPE`).
    fn on_a_vt() -> bool {
        // SAFETY: open and close of a NUL-terminated path; KDGKBTYPE writes
        // one char.
        unsafe {
            let fd = libc::open(c"/dev/tty".as_ptr(), libc::O_RDONLY | libc::O_CLOEXEC);
            if fd < 0 {
                return false;
            }
            let mut kbtype: std::ffi::c_char = 0;
            let ok = libc::ioctl(fd, 0x4B33 /* KDGKBTYPE */, &mut kbtype) == 0;
            libc::close(fd);
            ok
        }
    }

    pub(super) fn main() -> ExitCode {
        let args: Vec<String> = std::env::args().collect();
        if args.len() == 3 && args[1] == "--child" {
            return child(&args[2]);
        }
        if std::env::var_os("SDL3_CONSOLE_RUNNER").is_none() {
            // SAFETY: geteuid has no preconditions.
            if unsafe { libc::geteuid() } != 0 {
                return skip("vt", "not root (the program runs on tty3 as root)");
            }
            let tty = format!("/dev/tty{VT}");
            if let Err(e) = std::fs::File::open(&tty) {
                return skip("vt", &format!("{tty}: {e}"));
            }
            // SAFETY: access() reads the NUL-terminated path.
            if unsafe { libc::access(c"/dev/uinput".as_ptr(), libc::W_OK) } != 0 {
                return skip("uinput", "/dev/uinput isn't writable");
            }
            let manifest = std::env::var("CARGO_MANIFEST_DIR")
                .unwrap_or_else(|_| env!("CARGO_MANIFEST_DIR").to_owned());
            let runner = std::path::Path::new(&manifest).join("../tools/linux_console_run.py");
            let status = Command::new("python3")
                .arg(runner)
                .args(["--vt", VT])
                .arg(std::env::current_exe().unwrap())
                .args(std::env::args_os().skip(1))
                .status()
                .expect("python3 runs tools/linux_console_run.py");
            return ExitCode::from(status.code().unwrap_or(1) as u8);
        }
        if std::env::var_os("SDL3_CONSOLE_VT").is_none() || !on_a_vt() {
            return skip(
                "vt",
                "the controlling terminal isn't a virtual console (linux_console_run.py --vt)",
            );
        }

        let mut t = Trace {
            lines: Vec::new(),
            keyboards: HashMap::new(),
            window: 0,
        };
        t.ask("vt active");
        t.ask("vt modes");
        // Two keys of the keymap (all the consoles', put back by the
        // runner): Compose on the Compose key (the runner's keymap has it on
        // none), whose sequences come from the console's accent table, and
        // a Unicode keysym, the euro sign (U+20AC), on 102nd (unshifted)
        t.ask(&format!("vt setkey 0 127 {K_COMPOSE:#06x}"));
        t.ask(&format!("vt setkey 0 86 {:#06x}", 0x20ac ^ 0xf000));

        t.line("-- init".into());
        let Some(window) = t.init() else {
            return finish(t, Ok(()));
        };
        t.ask("vt modes");
        for reply in request(&format!("uinput add '{KEYBOARD}' keyboard")) {
            if reply.starts_with("uinput: unreadable ") {
                return skip("uinput", &format!("{reply}: the reader can't open it"));
            }
            t.line(reply);
        }
        t.wait_for(&format!("keyboard added: {KEYBOARD}"));
        t.ask("vt modes");

        t.line("-- keys".into());
        t.typing();

        t.line("-- vt switch".into());
        // Away: the kernel waits for SDL's VT_RELDISP, which the event pump
        // makes (on the release signal)
        t.ask(&format!("vt activate {OTHER_VT}"));
        let end = Instant::now() + Duration::from_secs(10);
        let away = format!("vt: active {OTHER_VT}");
        loop {
            events::pump();
            let active = request("vt active");
            if active.first() == Some(&away) || Instant::now() >= end {
                for line in active {
                    t.line(line);
                }
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        t.ask("vt modes");
        // Back: done at once, the acquire signal acknowledged by the next
        // pump
        t.ask(&format!("vt switch {VT}"));
        t.pump();
        t.ask("vt modes");
        t.keys(&tap("KEY_Z"));

        t.line("-- unplugged".into());
        t.ask(&format!("uinput remove '{KEYBOARD}'"));
        t.wait_for(&format!("keyboard removed: {KEYBOARD}"));
        t.ask("vt modes");
        t.quit(window);
        t.ask("vt modes");

        t.line("-- SDL_MUTE_CONSOLE_KEYBOARD=0".into());
        t.ask(&format!("uinput add '{KEYBOARD}' keyboard"));
        hints::set(hints::MUTE_CONSOLE_KEYBOARD, "0").unwrap();
        let Some(window) = t.init() else {
            return finish(t, Ok(()));
        };
        t.ask("vt modes");
        t.typing();
        t.quit(window);
        hints::reset(hints::MUTE_CONSOLE_KEYBOARD);
        t.ask("vt modes");
        // (the keys reached tty3's input too)
        t.ask("vt flush");

        // Children ended without SDL_Quit(), the keyboard there
        t.child("abort", &[]);
        // (the VT left in VT_PROCESS: a switch gives it back)
        t.ask(&format!("vt switch {OTHER_VT}"));
        t.ask("vt modes");
        t.ask(&format!("vt switch {VT}"));
        t.child("segv", &[]);
        t.child("bus", &[]);
        t.child("chained", &[]);
        t.child("exit", &[]);
        t.child("no-signal-handlers", &[(hints::NO_SIGNAL_HANDLERS, "1")]);
        t.ask("vt restore");

        let rust_only = stack_overflow();

        t.line("-- init and quit".into());
        let Some(window) = t.init() else {
            return finish(t, rust_only);
        };
        t.ask("vt modes");
        t.quit(window);
        t.ask("vt modes");
        t.ask(&format!("uinput remove '{KEYBOARD}'"));
        finish(t, rust_only)
    }

    /// Compare the trace with upstream's.
    fn finish(t: Trace, rust_only: Result<(), String>) -> ExitCode {
        let mut ok = true;
        if let Err(e) = rust_only {
            println!("evdev_vt: {e}");
            ok = false;
        }
        let actual = t.lines.join("\n") + "\n";
        if actual == EXPECTED {
            println!(
                "evdev_vt: the trace is upstream's ({} lines)",
                t.lines.len()
            );
            return if ok {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            };
        }
        let expected: Vec<&str> = EXPECTED.lines().collect();
        for (i, line) in t.lines.iter().enumerate() {
            if expected.get(i) != Some(&line.as_str()) {
                println!(
                    "evdev_vt: line {}: {line:?}, upstream's {:?}",
                    i + 1,
                    expected.get(i)
                );
                break;
            }
        }
        if expected.len() != t.lines.len() {
            println!(
                "evdev_vt: {} lines, upstream's {}",
                t.lines.len(),
                expected.len()
            );
        }
        ExitCode::FAILURE
    }
}
