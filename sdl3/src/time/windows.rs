// Rust translation of src/time/windows/SDL_systime.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Windows clock and calendar: `GetSystemTimePreciseAsFileTime()`,
//! `FileTimeToSystemTime()` and `SystemTimeToTzSpecificLocalTime()`, and
//! the user locale's date and time formats from `GetLocaleInfoW()`.

use super::{DateFormat, DateTime, Time, TimeFormat, Weekday};
use crate::err;
use crate::error::Result;
use crate::timer::NS_PER_SECOND;
use std::sync::OnceLock;
use windows_sys::Win32::Foundation::{GetLastError, FILETIME, SYSTEMTIME};
use windows_sys::Win32::Globalization::{
    GetLocaleInfoW, LOCALE_SSHORTDATE, LOCALE_STIMEFORMAT, LOCALE_USER_DEFAULT,
};
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows_sys::Win32::System::SystemInformation::GetSystemTimeAsFileTime;
use windows_sys::Win32::System::Time::{
    FileTimeToSystemTime, SystemTimeToFileTime, SystemTimeToTzSpecificLocalTime,
};

type PfnGetSystemTimePreciseAsFileTime = unsafe extern "system" fn(*mut FILETIME);

/// One `GetLocaleInfoW()` string of the user locale, or `None` on failure.
fn locale_info(lctype: u32) -> Option<Vec<u16>> {
    let mut str = [0u16; 80]; // Per the docs, the time and short date format strings can be a max of 80 characters.
                              // SAFETY: str holds the given number of UTF-16 units.
    let rc = unsafe {
        GetLocaleInfoW(
            LOCALE_USER_DEFAULT,
            lctype,
            str.as_mut_ptr(),
            str.len() as i32,
        )
    };
    if rc == 0 {
        return None;
    }
    let end = str.iter().position(|&c| c == 0).unwrap_or(str.len());
    Some(str[..end].to_vec())
}

/// Translation of `SDL_GetSystemTimeLocalePreferences()` (Windows).
pub(super) fn get_system_time_locale_preferences(df: &mut DateFormat, tf: &mut TimeFormat) {
    if let Some(s) = locale_info(LOCALE_SSHORTDATE) {
        for c in s {
            match c {
                c if c == u16::from(b'y') => {
                    *df = DateFormat::YearMonthDay;
                    break;
                }
                c if c == u16::from(b'd') => {
                    *df = DateFormat::DayMonthYear;
                    break;
                }
                c if c == u16::from(b'M') => {
                    *df = DateFormat::MonthDayYear;
                    break;
                }
                _ => {}
            }
        }
    }

    // Figure out the preferred system date format.
    if let Some(s) = locale_info(LOCALE_STIMEFORMAT) {
        for c in s {
            match c {
                c if c == u16::from(b'H') => {
                    *tf = TimeFormat::Hour24;
                    return;
                }
                c if c == u16::from(b'h') => {
                    *tf = TimeFormat::Hour12;
                    return;
                }
                _ => {}
            }
        }
    }
}

/// `pGetSystemTimePreciseAsFileTime`: only available in Win8/Server 2012
/// or higher, so looked up once.
fn p_get_system_time_precise_as_file_time() -> Option<PfnGetSystemTimePreciseAsFileTime> {
    static P: OnceLock<Option<PfnGetSystemTimePreciseAsFileTime>> = OnceLock::new();
    *P.get_or_init(|| {
        let name = crate::core::windows::utf8_to_wide("kernel32.dll");
        // SAFETY: name is NUL-terminated; kernel32 stays loaded.
        unsafe {
            let kernel32 = GetModuleHandleW(name.as_ptr());
            if kernel32.is_null() {
                return None;
            }
            let f = GetProcAddress(kernel32, c"GetSystemTimePreciseAsFileTime".as_ptr().cast())?;
            // SAFETY: the export has this signature.
            Some(std::mem::transmute::<
                unsafe extern "system" fn() -> isize,
                PfnGetSystemTimePreciseAsFileTime,
            >(f))
        }
    })
}

/// `SDL_TimeFromWindows()` of a `FILETIME`.
fn time_from_filetime(ft: &FILETIME) -> Time {
    Time::from_windows_filetime(((ft.dwHighDateTime as u64) << 32) | ft.dwLowDateTime as u64)
}

/// Translation of `SDL_GetCurrentTime()` (Windows).
pub(super) fn get_current_time() -> Result<Time> {
    let mut ft = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };

    match p_get_system_time_precise_as_file_time() {
        // SAFETY: ft is a valid out-parameter.
        Some(f) => unsafe { f(&mut ft) },
        // SAFETY: as above.
        None => unsafe { GetSystemTimeAsFileTime(&mut ft) },
    }

    Ok(time_from_filetime(&ft))
}

/// Translation of `SDL_TimeToDateTime()` (Windows).
pub(super) fn time_to_date_time(ticks: Time, local_time: bool) -> Result<DateTime> {
    let mut ticks = ticks.as_nanos();
    let mut rem_ns = (ticks % NS_PER_SECOND) as i32;
    if rem_ns < 0 {
        // Prevent rounding errors if the remaining nanoseconds are less than one unit of system time.
        if -rem_ns < 100 {
            ticks -= 100;
        }
        rem_ns += NS_PER_SECOND as i32;
    }

    let wtime = Time::from_nanos(ticks).to_windows_filetime();
    let ft = FILETIME {
        dwLowDateTime: wtime as u32,
        dwHighDateTime: (wtime >> 32) as u32,
    };

    // SAFETY: SYSTEMTIME is plain data; all zeroes is a valid value.
    let mut utc_st: SYSTEMTIME = unsafe { std::mem::zeroed() };
    // SAFETY: as above.
    let mut local_st: SYSTEMTIME = unsafe { std::mem::zeroed() };

    // SAFETY: valid in and out pointers.
    if unsafe { FileTimeToSystemTime(&ft, &mut utc_st) } != 0 {
        let mut st = None;
        let mut utc_offset = 0;
        if local_time {
            // SAFETY: valid pointers; NULL means the current time zone.
            if unsafe { SystemTimeToTzSpecificLocalTime(std::ptr::null(), &utc_st, &mut local_st) }
                != 0
            {
                // Calculate the difference for the UTC offset.
                let mut local_ft = FILETIME {
                    dwLowDateTime: 0,
                    dwHighDateTime: 0,
                };
                // SAFETY: valid in and out pointers.
                unsafe { SystemTimeToFileTime(&local_st, &mut local_ft) };
                let local_ticks = time_from_filetime(&local_ft).as_nanos();
                // Note (upstream): C subtracts ticks, but local_st only has
                // millisecond precision, so with a sub-millisecond part in
                // ticks and a time zone east of UTC this truncates to one
                // second less than the offset. Here the local time is
                // compared with utc_st, which has the same precision.
                let mut utc_ft = FILETIME {
                    dwLowDateTime: 0,
                    dwHighDateTime: 0,
                };
                // SAFETY: valid in and out pointers.
                unsafe { SystemTimeToFileTime(&utc_st, &mut utc_ft) };
                let utc_ticks = time_from_filetime(&utc_ft).as_nanos();
                utc_offset = ((local_ticks - utc_ticks) / NS_PER_SECOND) as i32;
                st = Some(&local_st);
            }
        } else {
            utc_offset = 0;
            st = Some(&utc_st);
        }

        if let Some(st) = st {
            return Ok(DateTime {
                year: st.wYear as i32,
                month: st.wMonth as u8,
                day: st.wDay as u8,
                hour: st.wHour as u8,
                minute: st.wMinute as u8,
                second: st.wSecond as u8,
                nanosecond: rem_ns as u32,
                day_of_week: Weekday::from_index(st.wDayOfWeek as i32).unwrap_or_default(),
                utc_offset,
            });
        }
    }

    // SAFETY: GetLastError has no preconditions.
    let code = unsafe { GetLastError() };
    Err(err!("SDL_DateTime conversion failed ({code})"))
}
