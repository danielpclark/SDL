/* FluidSynth - A Software Synthesizer
 *
 * Copyright (C) 2003  Peter Hanappe and others.
 *
 * This library is free software; you can redistribute it and/or
 * modify it under the terms of the GNU Lesser General Public License
 * as published by the Free Software Foundation; either version 2.1 of
 * the License, or (at your option) any later version.
 *
 * This library is distributed in the hope that it will be useful, but
 * WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the GNU
 * Lesser General Public License for more details.
 *
 * You should have received a copy of the GNU Lesser General Public
 * License along with this library; if not, see
 * <https://www.gnu.org/licenses/>.
 */
// Modified 2026-10-17: translated into Rust from the logging and file
// functions of fluid_sys.c and fluid_sys.h (fluid_set_log_function(),
// fluid_default_log_function(), fluid_log(), fluid_file_open(),
// fluid_file_read(), fluid_file_seek(), fluid_file_tell(), and
// fluid_curtime() with the C++11 OS abstraction's fluid_utime()) in
// SDL_mixer 3.3.0's FluidSynth 2.5.7 (see NOTICE). The threads, timers,
// sockets, profiling and shell parsing of fluid_sys.c are not reached by
// SDL_mixer (whose build has no threads, network or shell).

use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

/// The synth can't function correctly any more (`FLUID_PANIC`)
pub const FLUID_PANIC: i32 = 0;
/// Serious error occurred (`FLUID_ERR`)
pub const FLUID_ERR: i32 = 1;
/// Warning (`FLUID_WARN`)
pub const FLUID_WARN: i32 = 2;
/// Verbose informational messages (`FLUID_INFO`)
pub const FLUID_INFO: i32 = 3;
/// Debugging messages (`FLUID_DBG`)
pub const FLUID_DBG: i32 = 4;
/// The number of log levels (`LAST_LOG_LEVEL`).
pub const LAST_LOG_LEVEL: i32 = 5;

/// Translation of `fluid_log_function_t`: a log function (the level and
/// the message; the C `data` pointer is whatever the closure captures).
pub type FluidLogFunction = Arc<dyn Fn(i32, &str) + Send + Sync>;

/// `fluid_log_function[]`: `None` for a level means the default function,
/// `Some(None)` no function.
type LogTable = [Option<Option<FluidLogFunction>>; LAST_LOG_LEVEL as usize];

static FLUID_LOG_FUNCTION: Mutex<LogTable> = Mutex::new([None, None, None, None, None]);

const FLUID_LIBNAME: &str = "fluidsynth";

/// The default log function of a level (`fluid_default_log_function` for
/// all but `FLUID_DBG`, which has none in a build without DEBUG).
fn default_for(level: i32) -> Option<FluidLogFunction> {
    if level == FLUID_DBG {
        None
    } else {
        Some(Arc::new(fluid_default_log_function))
    }
}

/// Installs a new log function for a specified log level (`None` for none),
/// returning the previously installed one. Translation of
/// `fluid_set_log_function()`.
pub fn set_log_function(level: i32, fun: Option<FluidLogFunction>) -> Option<FluidLogFunction> {
    let mut old = None;

    if (0..LAST_LOG_LEVEL).contains(&level) {
        let mut table = FLUID_LOG_FUNCTION.lock().unwrap_or_else(|e| e.into_inner());
        let slot = &mut table[level as usize];
        old = match slot.take() {
            None => default_for(level),
            Some(f) => f,
        };
        *slot = Some(fun);
    }

    old
}

/// Default log function which prints to the stderr (stdout on Windows).
/// Translation of `fluid_default_log_function()`.
pub fn fluid_default_log_function(level: i32, message: &str) {
    let line = match level {
        FLUID_PANIC => format!("{FLUID_LIBNAME}: panic: {message}\n"),
        FLUID_ERR => format!("{FLUID_LIBNAME}: error: {message}\n"),
        FLUID_WARN => format!("{FLUID_LIBNAME}: warning: {message}\n"),
        FLUID_INFO => format!("{FLUID_LIBNAME}: {message}\n"),
        FLUID_DBG => format!("{FLUID_LIBNAME}: debug: {message}\n"),
        _ => format!("{FLUID_LIBNAME}: {message}\n"),
    };

    if cfg!(windows) {
        let mut out = std::io::stdout().lock();
        let _ = out.write_all(line.as_bytes());
        let _ = out.flush();
    } else {
        let mut out = std::io::stderr().lock();
        let _ = out.write_all(line.as_bytes());
        let _ = out.flush();
    }
}

/// The log function installed for a level, if any (the
/// `fluid_log_function[level]` lookup of `fluid_log()`).
pub(crate) fn fluid_log_function(level: i32) -> Option<FluidLogFunction> {
    if (0..LAST_LOG_LEVEL).contains(&level) {
        let table = FLUID_LOG_FUNCTION.lock().unwrap_or_else(|e| e.into_inner());
        match &table[level as usize] {
            None => default_for(level),
            Some(f) => f.clone(),
        }
    } else {
        None
    }
}

/// The part of `fluid_log()` after the lookup: hands the message
/// (truncated as `char errbuf[1024]` and vsnprintf() truncate it) to the
/// log function.
pub(crate) fn fluid_log_with(fun: &FluidLogFunction, level: i32, message: &str) {
    let mut end = message.len().min(1023);
    while !message.is_char_boundary(end) {
        end -= 1;
    }
    fun(level, &message[..end]);
}

/// Translation of `fluid_log()`: print a message to the log. (Always
/// returns #FLUID_FAILED upstream.)
pub(crate) fn fluid_log(level: i32, message: &str) {
    if let Some(fun) = fluid_log_function(level) {
        fluid_log_with(&fun, level, message);
    }
}

/// `FLUID_LOG(level, ...)` with a format string (formatted only if the
/// level has a log function, as upstream's vsnprintf()).
macro_rules! flog {
    ($level:expr, $($arg:tt)*) => {{
        let level = $level;
        if let Some(fun) = $crate::utils::sys::fluid_log_function(level) {
            $crate::utils::sys::fluid_log_with(&fun, level, &format!($($arg)*));
        }
    }};
}
pub(crate) use flog;

/// Translation of `fluid_utime()` (the C++11 OS abstraction's): the time
/// in microseconds, from a monotonic clock.
pub(crate) fn fluid_utime() -> f64 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_secs_f64() * 1_000_000.0
}

/// Translation of `fluid_curtime()`: get time in milliseconds to be used in
/// relative timing operations.
pub(crate) fn fluid_curtime() -> u32 {
    static INITIAL_TIME: OnceLock<f64> = OnceLock::new();
    let initial_time = *INITIAL_TIME.get_or_init(fluid_utime);

    let now = fluid_utime();

    ((now - initial_time) / 1000.0) as u32
}

/// Translation of `fluid_file_open()`: opens a file for reading (`"rb"`),
/// or tells why it can't.
pub(crate) fn fluid_file_open(path: &str) -> Result<File, &'static str> {
    const ERR_EXIST: &str = "File does not exist.";
    const ERR_REGULAR: &str = "File is not regular, refusing to open it.";
    const ERR_NULL: &str = "File does not exists or insufficient permissions to open it.";

    match std::fs::metadata(path) {
        Err(_) => Err(ERR_EXIST),
        Ok(m) if !m.is_file() => Err(ERR_REGULAR),
        Ok(_) => File::open(path).map_err(|_| ERR_NULL),
    }
}

/// Translation of `fluid_file_tell()`.
pub(crate) fn fluid_file_tell(f: &mut File) -> i64 {
    f.stream_position().map(|p| p as i64).unwrap_or(-1)
}

/// Translation of `fluid_file_read()`: reads exactly `buf.len()` bytes.
pub(crate) fn fluid_file_read(buf: &mut [u8], fd: &mut File) -> bool {
    // (fread() of count bytes, 1 item: a short read fails)
    let mut done = 0;
    let mut eof = false;
    let mut failed = false;
    while done < buf.len() {
        match fd.read(&mut buf[done..]) {
            Ok(0) => {
                eof = true;
                break;
            }
            Ok(n) => done += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => {
                failed = true;
                break;
            }
        }
    }

    if done != buf.len() {
        if eof && !failed {
            flog!(
                FLUID_ERR,
                "EOF while attempting to read {} bytes",
                buf.len()
            );
        } else {
            fluid_log(FLUID_ERR, "File read failed");
        }

        return false;
    }

    true
}

/// `SEEK_SET`
pub const SEEK_SET: i32 = 0;
/// `SEEK_CUR`
pub const SEEK_CUR: i32 = 1;
/// `SEEK_END`
pub const SEEK_END: i32 = 2;

/// Translation of `fluid_file_seek()`.
pub(crate) fn fluid_file_seek(fd: &mut File, ofs: i64, whence: i32) -> bool {
    let pos = match whence {
        SEEK_SET if ofs >= 0 => Some(SeekFrom::Start(ofs as u64)),
        SEEK_CUR => Some(SeekFrom::Current(ofs)),
        SEEK_END => Some(SeekFrom::End(ofs)),
        _ => None,
    };

    if pos.and_then(|p| fd.seek(p).ok()).is_none() {
        flog!(
            FLUID_ERR,
            "File seek failed with offset = {} and whence = {}",
            ofs,
            whence
        );
        return false;
    }

    true
}
