// What the Android test programs share.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Android programs that run as apps, through `tools/android_run.py`'s
//! app mode (each marks itself with a `.sdl3_runner` section holding
//! `app`), and their spawned twins. An app's stdout and stderr go nowhere,
//! so a program first calls [`start`], which connects them to the
//! runner's sockets (`SDL3_ANDROID_RUNNER_STDIO`, set through the app's
//! manifest, which SDL's environment reads first); adb doesn't give an
//! app's exit status, so it ends with [`finish`], which prints the line
//! the runner takes it from, and returns it from `main` (upstream's
//! SDLMain then finishes the activity; the runner stops the app). Never
//! `std::process::exit()` in an app: libc's `exit()` ends the process
//! under the framework's threads, which abort. [`request`] asks the runner
//! for something done from outside the app (`background`, `foreground`,
//! `back`, `input ARGUMENTS`, `trim-memory LEVEL`, `night MODE`, `locale
//! TAGS`, `logcat TAGS`, ...: see tools/android_run.py), and gives its
//! answer. [`jni`] has the few JNI calls the programs make themselves, as
//! an app's C would (through `sdl3::android::jni_env()`).
//!
//! A panic in a native method (the framework's callbacks) aborts the
//! process, so the programs run their checks in [`checked`] and end with
//! the status it gives.

#![allow(dead_code)] // (each program uses what it needs)

pub mod events;
pub mod jni;

use std::io::Write;
use std::os::fd::AsRawFd;
use std::process::ExitCode;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Whether the program runs as an app of the runner's (it gave the
/// program its sockets).
pub fn is_app() -> bool {
    stdio().is_some()
}

/// The runner's sockets, `OUT,ERR` (through SDL's environment, which sets
/// the manifest's variables first, as upstream's does).
fn stdio() -> Option<String> {
    sdl3::stdlib::getenv_unsafe("SDL3_ANDROID_RUNNER_STDIO")
}

/// Connect stdout and stderr to the runner's sockets, when it gave some.
pub fn start() {
    let Some(ports) = stdio() else {
        return;
    };
    for (fd, port) in [1, 2].into_iter().zip(ports.split(',')) {
        let port: u16 = port.parse().expect("SDL3_ANDROID_RUNNER_STDIO's ports");
        let stream = std::net::TcpStream::connect(("127.0.0.1", port))
            .unwrap_or_else(|e| panic!("can't connect to the runner's port {port}: {e}"));
        // SAFETY: both are open descriptors; the socket's stays open as
        // `fd` (the stream is forgotten, not closed).
        let result = unsafe { libc::dup2(stream.as_raw_fd(), fd) };
        assert_eq!(result, fd, "dup2(): {}", std::io::Error::last_os_error());
        std::mem::forget(stream);
    }
}

/// The program's arguments: the runner's (`SDL3_ANDROID_RUNNER_ARGS`, `N:`
/// then the arguments percent-encoded, separated by commas) in an app,
/// where `main`'s are upstream's `app_process` alone; else `main`'s after
/// the program's name.
pub fn args() -> Vec<String> {
    let Some(encoded) = std::env::var("SDL3_ANDROID_RUNNER_ARGS")
        .ok()
        .filter(|_| is_app())
    else {
        return std::env::args().skip(1).collect();
    };
    let (count, rest) = encoded
        .split_once(':')
        .expect("SDL3_ANDROID_RUNNER_ARGS: N:...");
    let count: usize = count.parse().expect("SDL3_ANDROID_RUNNER_ARGS's count");
    if count == 0 {
        return Vec::new();
    }
    let args: Vec<String> = rest.split(',').map(percent_decode).collect();
    assert_eq!(args.len(), count, "SDL3_ANDROID_RUNNER_ARGS {encoded:?}");
    args
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).expect("a percent escape");
            out.push(u8::from_str_radix(hex, 16).expect("a percent escape"));
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).expect("UTF-8 arguments")
}

/// End the program with `status`: in an app, the runner's exit line
/// (`sdl3-android-runner: exit STATUS`), flushed; `main` returns the
/// status.
pub fn finish(status: u8) -> ExitCode {
    if is_app() {
        println!("sdl3-android-runner: exit {status}");
    }
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().flush();
    ExitCode::from(status)
}

/// End the program with `status` once its process has ended by itself
/// (the activity recreated after `main` returns ends it: upstream's
/// `System.exit(0)`): the runner's line for that (`sdl3-android-runner:
/// exit STATUS once the process ends`), flushed; `main` returns the status.
pub fn finish_once_the_process_ends(status: u8) -> ExitCode {
    if is_app() {
        println!("sdl3-android-runner: exit {status} once the process ends");
    }
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().flush();
    ExitCode::from(status)
}

/// An event type's name as the C programs write it (`WINDOW_RESIZED`).
pub fn event_name(ty: sdl3::events::EventType) -> String {
    format!("{ty:?}")
        .trim_start_matches("SDL_EVENT_")
        .to_string()
}

/// Compare a trace with upstream's (both printed, with the first line
/// that differs).
pub fn check_trace(trace: &[String], expected: &str) {
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

/// Ask the runner for something (`sdl3-android-runner: request WHAT`), and
/// wait until it's done: the lines of its answer (`sdl3-android-runner:
/// reply LINE`, until `sdl3-android-runner: done`, on the stdout socket).
/// Not in an app: nothing done, no answer.
pub fn request(what: &str) -> Vec<String> {
    println!("sdl3-android-runner: request {what}");
    let _ = std::io::stdout().flush();
    if !is_app() {
        return Vec::new();
    }
    // (the answers of the requests not waited for, first)
    while UNANSWERED.load(Ordering::SeqCst) > 0 {
        UNANSWERED.fetch_sub(1, Ordering::SeqCst);
        loop {
            match read_stdout_line().as_str() {
                "sdl3-android-runner: done" => break,
                "" => panic!("the runner's socket closed during a request"),
                _ => {}
            }
        }
    }
    let mut lines = Vec::new();
    loop {
        let line = read_stdout_line();
        if line == "sdl3-android-runner: done" {
            return lines;
        }
        match line.strip_prefix("sdl3-android-runner: reply ") {
            Some(reply) => lines.push(reply.to_string()),
            None if line.is_empty() => panic!("the runner's socket closed during a request"),
            None => panic!("the runner's answer to {what:?}: {line:?}"),
        }
    }
}

/// The requests sent without waiting ([`request_without_waiting()`]),
/// whose answers the next [`request()`] reads first.
static UNANSWERED: AtomicUsize = AtomicUsize::new(0);

/// Ask the runner for something, as [`request()`], without waiting for it
/// (as the C references' requests from an event watch).
pub fn request_without_waiting(what: &str) {
    println!("sdl3-android-runner: request {what}");
    let _ = std::io::stdout().flush();
    if is_app() {
        UNANSWERED.fetch_add(1, Ordering::SeqCst);
    }
}

/// A line the runner sent on the stdout socket (fd 1), without its end.
fn read_stdout_line() -> String {
    let mut line = Vec::new();
    loop {
        let mut byte = 0u8;
        // SAFETY: one byte into a byte (fd 1 is the runner's socket).
        let n = unsafe { libc::read(1, (&mut byte as *mut u8).cast(), 1) };
        if n < 0 && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
            continue;
        }
        if n <= 0 || byte == b'\n' {
            break;
        }
        line.push(byte);
    }
    String::from_utf8_lossy(&line).into_owned()
}

/// Run `checks`, catching a panic (printed by the panic hook): the exit
/// status for the runner, 0 if they passed, 101 (as libtest's) if not.
pub fn checked(checks: impl FnOnce()) -> u8 {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(checks)) {
        Ok(()) => 0,
        Err(_) => 101,
    }
}

/// The current thread's name as the system has it (`/proc`'s `comm`).
pub fn os_thread_name() -> String {
    // SAFETY: no arguments.
    let tid = unsafe { libc::gettid() };
    std::fs::read_to_string(format!("/proc/self/task/{tid}/comm"))
        .unwrap_or_default()
        .trim()
        .to_string()
}

/// The process's command line (an app's: its package name).
pub fn process_name() -> String {
    let cmdline = std::fs::read("/proc/self/cmdline").unwrap_or_default();
    String::from_utf8_lossy(cmdline.split(|&b| b == 0).next().unwrap_or_default()).into_owned()
}
