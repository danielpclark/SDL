// What the KMS/DRM video driver's test programs share: the capability
// checks, the runner's requests, and the trace.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The programs (`harness = false`) run as root on the virtual console
//! tty3, through `tools/linux_console_run.py --vt 3` (which they start
//! themselves under), on a vkms card (`SDL_KMSDRM_DEVICE_INDEX`, or the
//! first card whose DRM driver is vkms), with `SDL_VIDEO_DRIVER=kmsdrm`.
//! The card's index, and its connectors' numbers, depend on the machine's
//! other cards: the traces name neither (see [`connector_name`]). The runner
//! answers their requests: uinput devices, the console's modes and
//! switches, and the card's state as a second client sees it (`drm
//! state`, `drm crc`, `drm pixels`, `drm master ...`).
//!
//! Capabilities: `kmsdrm` (root, for DRM master; a vkms card with a
//! connected connector; libdrm and libgbm), `egl` (libEGL), `vt` (tty3),
//! and `uinput` (for the programs that make devices). A program skips
//! without them, unless `SDL3_TEST_REQUIRE` names them.
//!
//! The trace is compared with upstream's, from the same program in C on
//! the same runner. Its first line, the versions of the kernel, Mesa and
//! libdrm (`versions: ...`), is the reference's preamble: not compared,
//! but printed when the trace differs.

#![allow(dead_code)] // (each program uses some of it)

use std::ffi::c_int;
use std::io::{BufRead, BufReader, Write};
use std::os::fd::FromRawFd;
use std::process::{Command, ExitCode};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

pub const PREFIX: &str = "sdl3-console-runner: ";
/// The console the runner starts the program on
pub const VT: &str = "3";
/// The console switched to and back from
pub const OTHER_VT: &str = "1";

/// Whether `SDL3_TEST_REQUIRE` makes a capability mandatory.
pub fn required(capability: &str) -> bool {
    std::env::var("SDL3_TEST_REQUIRE")
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .any(|c| c == "all" || c == capability)
}

/// Skip (all of) the program, as the library's tests do.
pub fn skip(program: &str, capability: &str, reason: &str) -> ExitCode {
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
            let _ = writeln!(file, "{capability}\t{program}\t{reason}");
        }
    }
    ExitCode::SUCCESS
}

/// Whether a library can be loaded (`dlopen()`).
fn loadable(name: &std::ffi::CStr) -> bool {
    // SAFETY: dlopen of a NUL-terminated name; the handle is closed at
    // once.
    unsafe {
        let handle = libc::dlopen(name.as_ptr(), libc::RTLD_LAZY | libc::RTLD_LOCAL);
        if handle.is_null() {
            return false;
        }
        libc::dlclose(handle);
    }
    true
}

/// `struct drm_version` (`<drm/drm.h>`), for `DRM_IOCTL_VERSION`.
#[repr(C)]
struct DrmVersion {
    version_major: c_int,
    version_minor: c_int,
    version_patchlevel: c_int,
    name_len: usize,
    name: *mut std::ffi::c_char,
    date_len: usize,
    date: *mut std::ffi::c_char,
    desc_len: usize,
    desc: *mut std::ffi::c_char,
}

/// The name of the DRM driver of `/dev/dri/card{index}`
/// (`DRM_IOCTL_VERSION`'s: `vkms` for every vkms device, whatever the
/// device's own name).
fn drm_driver(index: u32) -> Option<String> {
    use std::os::fd::AsRawFd;
    let card = std::fs::File::open(format!("/dev/dri/card{index}")).ok()?;
    let mut name = [0u8; 64];
    let mut version = DrmVersion {
        version_major: 0,
        version_minor: 0,
        version_patchlevel: 0,
        name_len: name.len(),
        name: name.as_mut_ptr().cast(),
        date_len: 0,
        date: std::ptr::null_mut(),
        desc_len: 0,
        desc: std::ptr::null_mut(),
    };
    // _IOWR('d', 0x00, struct drm_version)
    let request = (3 << 30) | (std::mem::size_of::<DrmVersion>() << 16) | ((b'd' as usize) << 8);
    // SAFETY: DRM_IOCTL_VERSION on an open card writes the structure and at
    // most `name_len` bytes of the name into `name`, which outlives the
    // call (the date and description have no room: not written).
    if unsafe { libc::ioctl(card.as_raw_fd(), request as _, &mut version) } != 0 {
        return None;
    }
    let len = version.name_len.min(name.len());
    Some(String::from_utf8_lossy(&name[..len]).into_owned())
}

/// The index of the vkms card: `SDL_KMSDRM_DEVICE_INDEX` (if it's one),
/// or the first card whose DRM driver is vkms. (Which index that is
/// depends on the machine: vkms is card0 on some CI runners, card1 after
/// the firmware framebuffer's simpledrm card0 on others.)
fn vkms_card() -> Result<u32, String> {
    let is_vkms = |index: u32| drm_driver(index).is_some_and(|d| d == "vkms");
    if let Ok(index) = std::env::var("SDL_KMSDRM_DEVICE_INDEX") {
        let index: u32 = index
            .parse()
            .map_err(|_| format!("SDL_KMSDRM_DEVICE_INDEX={index:?} isn't an index"))?;
        return if is_vkms(index) {
            Ok(index)
        } else {
            Err(format!("card{index} isn't vkms (modprobe vkms)"))
        };
    }
    let mut cards: Vec<u32> = std::fs::read_dir("/sys/class/drm")
        .map_err(|e| format!("/sys/class/drm: {e}"))?
        .filter_map(|e| {
            let name = e.ok()?.file_name().into_string().ok()?;
            name.strip_prefix("card")?.parse().ok()
        })
        .collect();
    cards.sort();
    cards
        .into_iter()
        .find(|&i| is_vkms(i))
        .ok_or_else(|| "no vkms card (modprobe vkms)".to_owned())
}

/// The card the programs run on: `SDL_KMSDRM_DEVICE_INDEX`'s (which the
/// runner runs them with), or the first vkms card (the runner's default).
pub fn card() -> u32 {
    std::env::var("SDL_KMSDRM_DEVICE_INDEX")
        .ok()
        .and_then(|index| index.parse().ok())
        .or_else(|| vkms_card().ok())
        .unwrap_or(0)
}

/// A display's name (its connector's, `TYPE-N`) as the traces give it,
/// `TYPE-#K`: the card's `K`th connector of that type, by number (`TYPE-#?`
/// when `card` has no connector of that name). The kernel numbers the
/// connectors of a type across all the machine's cards, in the order they
/// are made, so `N` depends on the cards before the vkms card: on Azure's
/// runners the Hyper-V card's Virtual-1 makes vkms's connector Virtual-2,
/// on others (with no other virtual connector) it is Virtual-1. The C
/// twins mask the names the same way.
pub fn connector_name(card: u32, name: &str) -> String {
    let Some((kind, number)) = name
        .rsplit_once('-')
        .and_then(|(kind, n)| Some((kind, n.parse::<u32>().ok()?)))
    else {
        return name.to_owned();
    };
    let prefix = format!("card{card}-{kind}-");
    let numbers: Vec<u32> = std::fs::read_dir("/sys/class/drm")
        .map(|d| {
            d.filter_map(|e| {
                let entry = e.ok()?.file_name().into_string().ok()?;
                entry.strip_prefix(&prefix)?.parse().ok()
            })
            .collect()
        })
        .unwrap_or_default();
    if numbers.contains(&number) {
        let rank = numbers.iter().filter(|&&n| n < number).count() + 1;
        format!("{kind}-#{rank}")
    } else {
        format!("{kind}-#?")
    }
}

/// Check the capabilities, then run the program as root on tty3 under the
/// runner (`Ok`: running there already).
pub fn run_on_tty3(program: &str, uinput: bool) -> Result<(), ExitCode> {
    if std::env::var_os("SDL3_CONSOLE_RUNNER").is_some() {
        if std::env::var_os("SDL3_CONSOLE_VT").is_none() || !on_a_vt() {
            return Err(skip(
                program,
                "vt",
                "the controlling terminal isn't a virtual console (linux_console_run.py --vt)",
            ));
        }
        return Ok(());
    }
    // SAFETY: geteuid has no preconditions.
    if unsafe { libc::geteuid() } != 0 {
        return Err(skip(
            program,
            "kmsdrm",
            "not root (the program runs on tty3 as root, for DRM master)",
        ));
    }
    let card = match vkms_card() {
        Ok(card) => card,
        Err(e) => return Err(skip(program, "kmsdrm", &e)),
    };
    let connected = std::fs::read_dir("/sys/class/drm")
        .map(|d| {
            d.filter_map(|e| e.ok()).any(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                name.starts_with(&format!("card{card}-"))
                    && std::fs::read_to_string(e.path().join("status"))
                        .is_ok_and(|s| s.trim() == "connected")
            })
        })
        .unwrap_or(false);
    if !connected {
        return Err(skip(
            program,
            "kmsdrm",
            &format!("card{card} has no connected connector"),
        ));
    }
    for lib in [c"libdrm.so.2", c"libgbm.so.1"] {
        if !loadable(lib) {
            return Err(skip(
                program,
                "kmsdrm",
                &format!("{} can't be loaded", lib.to_string_lossy()),
            ));
        }
    }
    if !loadable(c"libEGL.so.1") {
        return Err(skip(program, "egl", "libEGL.so.1 can't be loaded"));
    }
    let tty = format!("/dev/tty{VT}");
    if let Err(e) = std::fs::File::open(&tty) {
        return Err(skip(program, "vt", &format!("{tty}: {e}")));
    }
    // SAFETY: access() reads the NUL-terminated path.
    if uinput && unsafe { libc::access(c"/dev/uinput".as_ptr(), libc::W_OK) } != 0 {
        return Err(skip(program, "uinput", "/dev/uinput isn't writable"));
    }
    let manifest = std::env::var("CARGO_MANIFEST_DIR")
        .unwrap_or_else(|_| env!("CARGO_MANIFEST_DIR").to_owned());
    let runner = std::path::Path::new(&manifest).join("../tools/linux_console_run.py");
    let status = Command::new("python3")
        .arg(runner)
        .args(["--vt", VT])
        .arg(std::env::current_exe().unwrap())
        .args(std::env::args_os().skip(1))
        .env("SDL_KMSDRM_DEVICE_INDEX", card.to_string())
        .env("SDL_VIDEO_DRIVER", "kmsdrm")
        .status()
        .expect("python3 runs tools/linux_console_run.py");
    Err(ExitCode::from(status.code().unwrap_or(1) as u8))
}

/// Whether the controlling terminal is a virtual console (as the
/// console keyboard finds it: `KDGKBTYPE`).
pub fn on_a_vt() -> bool {
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

/// The runner's answers: the pipe it gives (stdin is the console).
fn replies() -> &'static Mutex<BufReader<std::fs::File>> {
    static REPLIES: OnceLock<Mutex<BufReader<std::fs::File>>> = OnceLock::new();
    REPLIES.get_or_init(|| {
        let fd: c_int = std::env::var("SDL3_CONSOLE_REPLY_FD")
            .expect("SDL3_CONSOLE_REPLY_FD (tools/linux_console_run.py --vt)")
            .parse()
            .expect("a descriptor");
        // SAFETY: the runner's pipe, inherited, which nothing else owns.
        Mutex::new(BufReader::new(unsafe { std::fs::File::from_raw_fd(fd) }))
    })
}

/// Ask the runner, and return its answer's lines.
pub fn request(line: &str) -> Vec<String> {
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

/// The trace, printed as it's made.
pub struct Trace {
    pub lines: Vec<String>,
    /// Rust-only failures (lines `rust: `), apart from the trace
    pub rust_failures: Vec<String>,
}

impl Default for Trace {
    fn default() -> Trace {
        Trace::new()
    }
}

impl Trace {
    pub fn new() -> Trace {
        let mut t = Trace {
            lines: Vec::new(),
            rust_failures: Vec::new(),
        };
        // (the runner's `drm versions`: the reference's preamble)
        t.ask("drm versions");
        t
    }

    pub fn line(&mut self, line: impl Into<String>) {
        let line = line.into();
        println!("trace: {line}");
        std::io::stdout().flush().unwrap();
        self.lines.push(line);
    }

    /// The answer's lines, traced.
    pub fn ask(&mut self, line: &str) {
        for reply in request(line) {
            self.line(reply);
        }
    }

    /// A Rust-only check's outcome (not in the trace).
    pub fn rust(&mut self, ok: bool, line: impl Into<String>) {
        let line = line.into();
        println!("rust: {line}");
        if !ok {
            self.rust_failures.push(line);
        }
    }

    /// Compare the trace with upstream's.
    pub fn finish(self, program: &str, expected: &str) -> ExitCode {
        let mut ok = true;
        for f in &self.rust_failures {
            println!("{program}: {f}");
            ok = false;
        }
        let expected: Vec<&str> = expected.lines().collect();
        let actual: Vec<&str> = self.lines.iter().map(String::as_str).collect();
        // (the preamble isn't compared)
        if actual.get(1..) == expected.get(1..) {
            println!(
                "{program}: the trace is upstream's ({} lines)",
                self.lines.len()
            );
            return if ok {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            };
        }
        if expected.first() != actual.first() {
            println!(
                "{program}: the reference's {:?}, here {:?}",
                expected.first(),
                actual.first()
            );
        }
        for (i, line) in actual.iter().enumerate().skip(1) {
            if expected.get(i) != Some(line) {
                println!(
                    "{program}: line {}: {line:?}, upstream's {:?}",
                    i + 1,
                    expected.get(i)
                );
                break;
            }
        }
        if expected.len() != actual.len() {
            println!(
                "{program}: {} lines, upstream's {}",
                actual.len(),
                expected.len()
            );
        }
        ExitCode::FAILURE
    }
}

/// Pump until `done` says so (or 10 s passed: false), sleeping 10 ms
/// between the pumps.
pub fn pump_until(mut done: impl FnMut() -> bool) -> bool {
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        sdl3::events::pump();
        if done() {
            return true;
        }
        if Instant::now() >= end {
            return false;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// An error's message, or "ok".
pub fn result<T>(r: &sdl3::Result<T>) -> String {
    match r {
        Ok(_) => "ok".to_owned(),
        Err(e) => format!("error: {e}"),
    }
}

/// The mode as traced: its size, refresh rate (and its fraction), format.
pub fn mode_line(m: &sdl3::video::DisplayMode) -> String {
    format!(
        "{}x{}@{:.2} ({}/{}) density {:.2} {}",
        m.w,
        m.h,
        m.refresh_rate,
        m.refresh_rate_numerator,
        m.refresh_rate_denominator,
        m.pixel_density,
        m.format.name()
    )
}

/// Raw OpenGL (ES): `glClearColor()` then `glClear()` of the current
/// context.
pub fn gl_clear(r: f32, g: f32, b: f32) {
    type ClearColor = unsafe extern "C" fn(f32, f32, f32, f32);
    type Clear = unsafe extern "C" fn(u32);
    let clear_color = sdl3::video::gl::gl_get_proc_address("glClearColor").expect("glClearColor");
    let clear = sdl3::video::gl::gl_get_proc_address("glClear").expect("glClear");
    // SAFETY: the GL functions of the current context, of these types.
    unsafe {
        std::mem::transmute::<*const std::ffi::c_void, ClearColor>(clear_color)(r, g, b, 1.0);
        std::mem::transmute::<*const std::ffi::c_void, Clear>(clear)(
            0x4000, /* GL_COLOR_BUFFER_BIT */
        );
    }
}
