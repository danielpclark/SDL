// Rust translation of src/time/unix/SDL_systime.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Unix clock and calendar: `clock_gettime()`, `localtime_r()` and
//! `gmtime_r()`, and the locale's date and time formats from
//! `nl_langinfo()`.

use super::{DateFormat, DateTime, Time, TimeFormat, Weekday};
use crate::err;
use crate::error::Result;
use crate::timer::NS_PER_SECOND;

/// The OS error code (`errno`) of the last call.
fn errno() -> i32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

/// `nl_langinfo(item)`'s bytes (empty for NULL).
///
/// This *should* be well-supported aside from very old legacy systems, but
/// apparently Android didn't add this until SDK version 26 (upstream's
/// `HAVE_NL_LANGINFO`), so it isn't used there.
#[cfg(not(target_os = "android"))]
fn langinfo(item: libc::nl_item) -> Vec<u8> {
    // SAFETY: nl_langinfo returns a NUL-terminated string (or NULL) that
    // stays valid until the next call on this thread; it is copied at once.
    unsafe {
        let s = libc::nl_langinfo(item);
        if s.is_null() {
            return Vec::new();
        }
        std::ffi::CStr::from_ptr(s).to_bytes().to_vec()
    }
}

/// Translation of `SDL_GetSystemTimeLocalePreferences()` (Unix).
pub(super) fn get_system_time_locale_preferences(df: &mut DateFormat, tf: &mut TimeFormat) {
    #[cfg(not(target_os = "android"))]
    {
        // Figure out the preferred system date format from the first format character.
        for &c in &langinfo(libc::D_FMT) {
            match c {
                b'Y' | b'y' | b'F' | b'C' => {
                    *df = DateFormat::YearMonthDay;
                    break;
                }
                b'd' | b'e' => {
                    *df = DateFormat::DayMonthYear;
                    break;
                }
                b'b' | b'D' | b'h' | b'm' => {
                    *df = DateFormat::MonthDayYear;
                    break;
                }
                _ => {}
            }
        }

        // Figure out the preferred system date format.
        for &c in &langinfo(libc::T_FMT) {
            match c {
                b'H' | b'k' | b'T' => {
                    *tf = TimeFormat::Hour24;
                    return;
                }
                b'I' | b'l' | b'r' => {
                    *tf = TimeFormat::Hour12;
                    return;
                }
                _ => {}
            }
        }
    }
    #[cfg(target_os = "android")]
    let _ = (df, tf);
}

/// Translation of `SDL_GetCurrentTime()` (Unix, `clock_gettime()`).
pub(super) fn get_current_time() -> Result<Time> {
    let mut tp = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: tp is a valid out-parameter.
    if unsafe { libc::clock_gettime(libc::CLOCK_REALTIME, &mut tp) } == 0 {
        //tp.tv_sec = SDL_min(tp.tv_sec, SDL_NS_TO_SECONDS(SDL_MAX_TIME) - 1);
        #[allow(clippy::unnecessary_cast)] // (time_t and long are 32-bit on some targets)
        return Ok(Time::from_nanos(
            (tp.tv_sec as i64)
                .wrapping_mul(NS_PER_SECOND)
                .wrapping_add(tp.tv_nsec as i64),
        ));
    }

    Err(err!("Failed to retrieve system time ({})", errno()))
}

/// Translation of `SDL_TimeToDateTime()` (Unix).
pub(super) fn time_to_date_time(ticks: Time, local_time: bool) -> Result<DateTime> {
    let mut ticks = ticks.as_nanos();
    let mut rem_ns = (ticks % NS_PER_SECOND) as i32;
    if rem_ns < 0 {
        // Prevent rounding errors if the remaining nanoseconds are less than one unit of system time.
        // Note (upstream): within a second of SDL_MIN_TIME this overflows
        // (undefined behaviour in C; it wraps around, as here).
        ticks = ticks.wrapping_sub(NS_PER_SECOND);
        rem_ns += NS_PER_SECOND as i32;
    }
    let tval = (ticks / NS_PER_SECOND) as libc::time_t;

    // SAFETY: tm is plain data; all zeroes is a valid value.
    let mut tm_storage: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: both pointers are valid; the result points into tm_storage or is NULL.
    let tm = unsafe {
        if local_time {
            libc::localtime_r(&tval, &mut tm_storage)
        } else {
            libc::gmtime_r(&tval, &mut tm_storage)
        }
    };

    if tm.is_null() {
        return Err(err!("SDL_DateTime conversion failed ({})", errno()));
    }
    let tm = &tm_storage;

    /* tm_gmtoff wasn't formally standardized until POSIX.1-2024, but practically it has been available on desktop
     * *nix platforms such as Linux/glibc, FreeBSD, OpenBSD, NetBSD, OSX/macOS, and others since the 1990s.
     *
     * The notable exception is Solaris, where the timezone offset must still be retrieved in the strictly POSIX.1-2008
     * compliant way.
     */
    #[cfg(not(any(target_os = "solaris", target_os = "illumos")))]
    let utc_offset = tm.tm_gmtoff as i32;
    #[cfg(any(target_os = "solaris", target_os = "illumos"))]
    let utc_offset = if local_time {
        extern "C" {
            static timezone: libc::c_long;
            fn tzset();
        }
        // FIXME (upstream): `timezone` counts seconds *west* of UTC, and
        // ignores daylight saving time; utc_offset is seconds east.
        // SAFETY: tzset() initializes `timezone`, which is then read.
        unsafe {
            tzset();
            timezone as i32
        }
    } else {
        0
    };

    Ok(DateTime {
        year: tm.tm_year + 1900,
        month: (tm.tm_mon + 1) as u8,
        day: tm.tm_mday as u8,
        hour: tm.tm_hour as u8,
        minute: tm.tm_min as u8,
        second: tm.tm_sec as u8,
        nanosecond: rem_ns as u32,
        day_of_week: Weekday::from_index(tm.tm_wday).unwrap_or_default(),
        utc_offset,
    })
}
