// Rust translation of src/test/SDL_test_log.c and include/SDL3/SDL_test_log.h
// from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Logging related functions of SDL test framework.
//!
//! Wrapper to log in the TEST category: messages get a local timestamp
//! (`08/23/01 14:55:02`), and those less important than errors are slightly
//! indented. Formatting uses the [`log_message!`](crate::log_message),
//! [`log!`](crate::log!) and [`log_error!`](crate::log_error) macros.
//!
//! Used by the test framework and test cases.

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};

use sdl3::log::Category;

/// The priorities of [`log_message()`], from `sdl3`. Translation of `SDL_LogPriority`.
pub use sdl3::log::Priority;

use crate::internal::{isprint, truncate};
use crate::MAX_LOGMESSAGE_LENGTH;

/// Whether messages get a timestamp. Translation of `SDLTest_Time`.
static TIME: AtomicBool = AtomicBool::new(true);

/// Turn the timestamps of the messages on or off (on by default; the
/// common options' `--no-time` turns them off). Translation of setting
/// `SDLTest_Time`.
pub fn set_timestamps(enabled: bool) {
    TIME.store(enabled, Ordering::Relaxed);
}

/// Whether the messages get timestamps. Translation of reading `SDLTest_Time`.
pub fn timestamps() -> bool {
    TIME.load(Ordering::Relaxed)
}

/// Turn the colors of the harness's and the assertions' messages on or
/// off (on by default). Translation of setting `SDLTest_Color`.
pub fn set_color(enabled: bool) {
    crate::internal::set_color(enabled);
}

/// Converts the current time to its ascii representation in localtime.
///
/// Returns an ascii representation of the timestamp in localtime in the
/// format '08/23/01 14:55:02' (what `strftime()` makes of `"%x %X"` in the
/// C locale), or an empty string when timestamps are off or the local time
/// is not known. Translation of `SDLTest_TimestampToString(time(NULL))`.
fn timestamp_to_string() -> String {
    if !timestamps() {
        return String::new();
    }

    let Some((year, month, day, hour, minute, second)) = local_time() else {
        return String::new();
    };
    format!(
        "{:02}/{:02}/{:02} {:02}:{:02}:{:02}",
        month,
        day,
        year.rem_euclid(100),
        hour,
        minute,
        second
    )
}

/// The local time now (year, month, day, hour, minute, second), from
/// `localtime(time(NULL))`.
#[cfg(unix)]
fn local_time() -> Option<(i32, u32, u32, u32, u32, u32)> {
    // SAFETY: time() and localtime_r() only write the values passed.
    unsafe {
        let copy = libc::time(std::ptr::null_mut());
        let mut local: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&copy, &mut local).is_null() {
            return None;
        }
        Some((
            local.tm_year + 1900,
            (local.tm_mon + 1) as u32,
            local.tm_mday as u32,
            local.tm_hour as u32,
            local.tm_min as u32,
            local.tm_sec as u32,
        ))
    }
}

/// The local time now (year, month, day, hour, minute, second), from
/// `GetLocalTime()` (what `localtime(time(NULL))` comes to).
#[cfg(windows)]
fn local_time() -> Option<(i32, u32, u32, u32, u32, u32)> {
    let mut local: windows_sys::Win32::Foundation::SYSTEMTIME =
        // SAFETY: plain data, zeroed.
        unsafe { std::mem::zeroed() };
    // SAFETY: GetLocalTime() only fills the structure.
    unsafe { windows_sys::Win32::System::SystemInformation::GetLocalTime(&mut local) };
    Some((
        local.wYear as i32,
        local.wMonth as u32,
        local.wDay as u32,
        local.wHour as u32,
        local.wMinute as u32,
        local.wSecond as u32,
    ))
}

/// The time now (year, month, day, hour, minute, second), in UTC where
/// there is no local time.
#[cfg(not(any(unix, windows)))]
fn local_time() -> Option<(i32, u32, u32, u32, u32, u32)> {
    let now = sdl3::time::Time::now().ok()?;
    let local = now
        .to_local_date_time()
        .unwrap_or_else(|_| now.to_utc_date_time());
    Some((
        local.year,
        local.month as u32,
        local.day as u32,
        local.hour as u32,
        local.minute as u32,
        local.second as u32,
    ))
}

/// Prints given message with a timestamp in the TEST category and given
/// priority. Translation of `SDLTest_LogMessageV()`.
fn log_message_v(priority: Priority, message: String) {
    /* Print log message into a buffer */
    let log_message = truncate(message, MAX_LOGMESSAGE_LENGTH);

    /* Log with timestamp and newline. Messages with lower priority are slightly indented. */
    // (through the log macros' entry point, which installs the default
    // output on first use as SDL_LogMessage() does)
    if priority > Priority::Info {
        sdl3::log::__log(
            Category::Test,
            priority,
            format_args!("{}: {}", timestamp_to_string(), log_message),
        );
    } else {
        sdl3::log::__log(
            Category::Test,
            priority,
            format_args!(" {}: {}", timestamp_to_string(), log_message),
        );
    }
}

/// Prints given message with a timestamp in the TEST category and given
/// priority. Translation of `SDLTest_LogMessage()` (with the text already
/// formatted; see [`log_message!`](crate::log_message)).
pub fn log_message(priority: Priority, message: &str) {
    log_message_v(priority, message.to_owned());
}

/// Prints given message with a timestamp in the TEST category and INFO
/// priority. Translation of `SDLTest_Log()` (see [`log!`](crate::log!)).
pub fn log(message: &str) {
    log_message_v(Priority::Info, message.to_owned());
}

/// Prints given message with a timestamp in the TEST category and the
/// ERROR priority. Translation of `SDLTest_LogError()` (see
/// [`log_error!`](crate::log_error)).
pub fn log_error(message: &str) {
    log_message_v(Priority::Error, message.to_owned());
}

#[doc(hidden)]
pub fn __log_message(priority: Priority, args: fmt::Arguments<'_>) {
    log_message_v(priority, fmt::format(args));
}

/// Log a formatted message with a timestamp in the TEST category:
/// `log_message!(priority, fmt, ...)`. Translation of `SDLTest_LogMessage()`.
#[macro_export]
macro_rules! log_message {
    ($priority:expr, $($arg:tt)*) => {
        $crate::log::__log_message($priority, ::std::format_args!($($arg)*))
    };
}

/// Log a formatted message with a timestamp in the TEST category and INFO
/// priority: `log!(fmt, ...)`. Translation of `SDLTest_Log()`.
#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => {
        $crate::log::__log_message($crate::log::Priority::Info, ::std::format_args!($($arg)*))
    };
}

/// Log a formatted message with a timestamp in the TEST category and ERROR
/// priority: `log_error!(fmt, ...)`. Translation of `SDLTest_LogError()`.
#[macro_export]
macro_rules! log_error {
    ($($arg:tt)*) => {
        $crate::log::__log_message($crate::log::Priority::Error, ::std::format_args!($($arg)*))
    };
}

pub use crate::{log as log_info, log_error, log_message};

fn nibble_to_char(nibble: u8) -> u8 {
    if nibble < 0xa {
        b'0' + nibble
    } else {
        b'a' + nibble - 10
    }
}

/// Prints given prefix and buffer. Non-printable characters in the raw data
/// are substituted by printable alternatives (`\0`, `\"`, `\n`, `\r`, `\t`,
/// `\f`, `\b`, `\\` and `\xNN`), quoted; `None` prints `(nil)`. Data that
/// doesn't fit in a message ends in `...`. Translation of
/// `SDLTest_LogEscapedString()`.
pub fn log_escaped_string(prefix: &str, buffer: Option<&[u8]>) {
    let log_message = match buffer {
        Some(data) => escape(data),
        None => "(nil)".to_owned(),
    };

    log_message_v(Priority::Info, format!("{prefix}{log_message}"));
}

/// The escaping of [`log_escaped_string`], into a buffer of
/// [`MAX_LOGMESSAGE_LENGTH`] bytes.
fn escape(data: &[u8]) -> String {
    const SIZE: usize = MAX_LOGMESSAGE_LENGTH;
    let mut log_message: Vec<u8> = Vec::with_capacity(SIZE);

    let mut i = 0;
    log_message.push(b'"');
    while i < data.len() {
        let c = data[i];
        let pos_start = log_message.len();
        // NEED_X_CHARS(N): when there is no room for N more, nothing is
        // written (C's `break` leaves the switch) and the loop ends below.
        let need = |n: usize, pos: usize| pos + n <= SIZE - 2;
        let escaped: Option<&[u8]> = match c {
            b'\0' => Some(b"\\0"),
            b'"' => Some(b"\\\""),
            b'\n' => Some(b"\\n"),
            b'\r' => Some(b"\\r"),
            b'\t' => Some(b"\\t"),
            0x0c => Some(b"\\f"),
            0x08 => Some(b"\\b"),
            b'\\' => Some(b"\\\\"),
            _ => None,
        };
        match escaped {
            Some(escaped) => {
                if need(2, log_message.len()) {
                    log_message.extend_from_slice(escaped);
                }
            }
            None => {
                if isprint(c) {
                    if need(1, log_message.len()) {
                        log_message.push(c);
                    }
                } else if need(4, log_message.len()) {
                    log_message.extend_from_slice(&[
                        b'\\',
                        b'x',
                        nibble_to_char(c >> 4),
                        nibble_to_char(c & 0xf),
                    ]);
                }
            }
        }
        if log_message.len() == pos_start {
            break;
        }
        i += 1;
    }
    if i < data.len() {
        // Note (upstream): the C code writes the dots over the last three
        // bytes of its buffer, leaving the bytes between the end of the
        // text and the dots uninitialized; here the text is cut there.
        log_message.truncate(SIZE - 4);
        log_message.extend_from_slice(b"...");
    } else {
        log_message.push(b'"');
    }

    // Every byte written is printable ASCII.
    String::from_utf8(log_message).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{test_lock, Capture};

    #[test]
    fn indentation_by_priority() {
        let _l = test_lock();
        let capture = Capture::new();
        crate::log!("hello {}", 42);
        crate::log_error!("bad {}", "thing");
        crate::log_message!(Priority::Warn, "warned");
        crate::log_message!(Priority::Debug, "debugged");
        log("plain");
        assert_eq!(
            capture.lines(),
            [
                (Category::Test, Priority::Info, " : hello 42".to_owned()),
                (Category::Test, Priority::Error, ": bad thing".to_owned()),
                (Category::Test, Priority::Warn, ": warned".to_owned()),
                (Category::Test, Priority::Debug, " : debugged".to_owned()),
                (Category::Test, Priority::Info, " : plain".to_owned()),
            ]
        );
    }

    #[test]
    fn timestamp_format() {
        let _l = test_lock();
        let capture = Capture::new();
        set_timestamps(true);
        log("x");
        let message = &capture.messages()[0];
        // " MM/DD/YY HH:MM:SS: x"
        let bytes = message.as_bytes();
        assert_eq!(message.len(), " 08/23/01 14:55:02: x".len(), "{message}");
        for (i, &b) in bytes.iter().enumerate() {
            match i {
                0 | 9 => assert_eq!(b, b' '),
                3 | 6 => assert_eq!(b, b'/'),
                12 | 15 | 18 => assert_eq!(b, b':'),
                19 => assert_eq!(b, b' '),
                20 => assert_eq!(b, b'x'),
                _ => assert!(b.is_ascii_digit(), "{message}"),
            }
        }
    }

    #[test]
    fn long_messages_are_cut() {
        let _l = test_lock();
        let capture = Capture::new();
        log(&"a".repeat(5000));
        assert_eq!(
            capture.messages()[0],
            format!(" : {}", "a".repeat(MAX_LOGMESSAGE_LENGTH - 1))
        );
    }

    #[test]
    fn escaped_strings() {
        let _l = test_lock();
        let capture = Capture::new();
        log_escaped_string("data: ", Some(b"a\"b\\c\n\r\t\x0c\x08\0\x01\x7f\xff z\0"));
        log_escaped_string("none: ", None);
        log_escaped_string("empty: ", Some(b""));
        assert_eq!(
            capture.messages(),
            [
                r#" : data: "a\"b\\c\n\r\t\f\b\0\x01\x7f\xff z\0""#, // (as upstream's C logs it)
                " : none: (nil)",
                r#" : empty: """#,
            ]
        );
    }

    #[test]
    fn escaped_string_overflow() {
        // 3581 plain bytes fill the buffer up to the limit (one quote + 3581
        // = 3582 = sizeof - 2); the next one doesn't fit.
        let data = vec![b'x'; 4000];
        let escaped = escape(&data);
        assert_eq!(escaped.len(), MAX_LOGMESSAGE_LENGTH - 1);
        assert!(escaped.starts_with("\"xxx"));
        assert!(escaped.ends_with("xx..."));

        // Exactly what fits: no dots, the closing quote is added.
        let data = vec![b'x'; MAX_LOGMESSAGE_LENGTH - 3];
        let escaped = escape(&data);
        assert_eq!(escaped.len(), MAX_LOGMESSAGE_LENGTH - 1);
        assert!(escaped.ends_with("x\""));

        // An escape that doesn't fit stops the output before it.
        let mut data = vec![b'x'; MAX_LOGMESSAGE_LENGTH - 4];
        data.push(0x01);
        let escaped = escape(&data);
        assert_eq!(escaped.len(), MAX_LOGMESSAGE_LENGTH - 1);
        assert!(escaped.ends_with("x..."));
    }
}
