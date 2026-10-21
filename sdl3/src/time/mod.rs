// Rust translation of src/time/SDL_time.c, SDL_time_c.h,
// src/time/dummy/SDL_systime.c and include/SDL3/SDL_time.h from Simple
// DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Realtime clock and calendar date/time routines.
//!
//! [`Time`] is a point in time as nanoseconds since the Unix epoch (`SDL_Time`);
//! [`DateTime`] breaks one down into human-understandable components
//! (`SDL_DateTime`). The calendar algorithms are Howard Hinnant's public-domain
//! ones, exactly as upstream uses them.
//!
//! The platform layer (the clock, local time and the locale's date and time
//! formats) is the Unix backend (`clock_gettime()`, `localtime_r()`,
//! `nl_langinfo()`) or the Windows one (`GetSystemTimePreciseAsFileTime()`,
//! `SystemTimeToTzSpecificLocalTime()`, `GetLocaleInfoW()`); elsewhere the
//! clock is `std`'s and there is no local time (upstream's dummy backend).

use std::fmt;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::err;
use crate::error::{Error, Result};
use crate::timer::NS_PER_SECOND;

#[cfg(unix)]
mod unix;
#[cfg(unix)]
use unix as sys;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as sys;

/// The dummy backend (`src/time/dummy/SDL_systime.c`).
#[cfg(not(any(unix, windows)))]
mod sys {
    use super::{DateFormat, DateTime, Time, TimeFormat};
    use crate::error::{Error, Result};

    /// Translation of `SDL_GetSystemTimeLocalePreferences()` (dummy).
    pub(super) fn get_system_time_locale_preferences(_df: &mut DateFormat, _tf: &mut TimeFormat) {
        // NOP
    }

    /// `SDL_GetCurrentTime()`: upstream's dummy backend reports that it is
    /// unsupported; `std`'s clock is used instead.
    pub(super) fn get_current_time() -> Result<Time> {
        Time::try_from(std::time::SystemTime::now())
    }

    /// Translation of `SDL_TimeToDateTime()` (dummy).
    pub(super) fn time_to_date_time(_ticks: Time, _local_time: bool) -> Result<DateTime> {
        Err(Error::unsupported())
    }
}

const SECONDS_PER_DAY: i64 = 86400;

/// Nanoseconds since the Unix epoch (Jan 1, 1970), signed. Translation of `SDL_Time`.
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Time(i64);

impl Time {
    /// Translation of `SDL_MAX_TIME`.
    pub const MAX: Time = Time(i64::MAX);
    /// Translation of `SDL_MIN_TIME`.
    pub const MIN: Time = Time(i64::MIN);
    /// The Unix epoch itself.
    pub const UNIX_EPOCH: Time = Time(0);

    /// From nanoseconds since the epoch.
    pub const fn from_nanos(nanos: i64) -> Time {
        Time(nanos)
    }

    /// Nanoseconds since the epoch.
    pub const fn as_nanos(self) -> i64 {
        self.0
    }

    /// Whole seconds since the epoch (rounded toward negative infinity).
    pub const fn as_secs(self) -> i64 {
        self.0.div_euclid(NS_PER_SECOND)
    }

    /// The current value of the system realtime clock in UTC.
    /// Translation of `SDL_GetCurrentTime()`.
    pub fn now() -> Result<Time> {
        sys::get_current_time()
    }

    /// Break down into calendar components in UTC.
    /// Translation of `SDL_TimeToDateTime(ticks, dt, false)`.
    ///
    /// Upstream asks the platform (`gmtime_r()`, `FileTimeToSystemTime()`),
    /// which can't fail for a [`Time`]; this computes the same result with
    /// the calendar algorithms below, on every platform (the tests check it
    /// against the platform's answer).
    pub fn to_utc_date_time(self) -> DateTime {
        let mut secs = self.0.div_euclid(NS_PER_SECOND);
        let nanos = self.0.rem_euclid(NS_PER_SECOND);
        let days = secs.div_euclid(SECONDS_PER_DAY);
        secs = secs.rem_euclid(SECONDS_PER_DAY);
        let (year, month, day) = civil_from_days(days);
        let (_, day_of_week, _) = days_from_civil(year, month as i32, day as i32);
        DateTime {
            year,
            month,
            day,
            hour: (secs / 3600) as u8,
            minute: ((secs % 3600) / 60) as u8,
            second: (secs % 60) as u8,
            nanosecond: nanos as u32,
            day_of_week,
            utc_offset: 0,
        }
    }

    /// Break down into calendar components in the local time zone.
    /// Translation of `SDL_TimeToDateTime(ticks, dt, true)`.
    ///
    /// Without a platform backend (other than on Unix and Windows) this
    /// returns [`ErrorKind::Unsupported`](crate::ErrorKind::Unsupported).
    pub fn to_local_date_time(self) -> Result<DateTime> {
        sys::time_to_date_time(self, true)
    }

    /// Convert to a Windows `FILETIME`: 100-nanosecond intervals since
    /// January 1, 1601. Translation of `SDL_TimeToWindows()`.
    pub const fn to_windows_filetime(self) -> u64 {
        /* Convert nanoseconds to Win32 ticks.
         * SDL_Time has a range of roughly 292 years, so even SDL_MIN_TIME can't underflow the Win32 epoch.
         */
        ((self.0 / 100) + DELTA_EPOCH_1601_100NS) as u64
    }

    /// Convert from a Windows `FILETIME`, clamping to the representable range.
    /// Translation of `SDL_TimeFromWindows()`.
    pub const fn from_windows_filetime(filetime: u64) -> Time {
        let wintime_min = ((Time::MIN.0 / 100) + DELTA_EPOCH_1601_100NS) as u64;
        let wintime_max = ((Time::MAX.0 / 100) + DELTA_EPOCH_1601_100NS) as u64;
        // Clamp the windows time range to the SDL_Time min/max
        let wtime = if filetime < wintime_min {
            wintime_min
        } else if filetime > wintime_max {
            wintime_max
        } else {
            filetime
        };
        Time(((wtime as i64) - DELTA_EPOCH_1601_100NS) * 100)
    }
}

/// 100 ns units between 1601-01-01 and 1970-01-01, 11644473600 seconds.
const DELTA_EPOCH_1601_100NS: i64 = 11644473600 * 10000000;

impl fmt::Debug for Time {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Time({} ns)", self.0)
    }
}

impl TryFrom<SystemTime> for Time {
    type Error = Error;

    fn try_from(t: SystemTime) -> Result<Time> {
        let nanos: i128 = match t.duration_since(UNIX_EPOCH) {
            Ok(d) => d.as_nanos() as i128,
            Err(e) => -(e.duration().as_nanos() as i128),
        };
        i64::try_from(nanos)
            .map(Time)
            .map_err(|_| err!("Time out of range for SDL_Time representation"))
    }
}

impl From<Time> for SystemTime {
    fn from(t: Time) -> SystemTime {
        if t.0 >= 0 {
            UNIX_EPOCH + Duration::from_nanos(t.0 as u64)
        } else {
            UNIX_EPOCH - Duration::from_nanos(t.0.unsigned_abs())
        }
    }
}

/// Day of the week, Sunday first (SDL numbers them 0–6 with 0 = Sunday).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum Weekday {
    #[default]
    Sunday = 0,
    Monday,
    Tuesday,
    Wednesday,
    Thursday,
    Friday,
    Saturday,
}

impl Weekday {
    /// From SDL's 0 (Sunday) – 6 (Saturday) numbering.
    pub const fn from_index(i: i32) -> Option<Weekday> {
        Some(match i {
            0 => Weekday::Sunday,
            1 => Weekday::Monday,
            2 => Weekday::Tuesday,
            3 => Weekday::Wednesday,
            4 => Weekday::Thursday,
            5 => Weekday::Friday,
            6 => Weekday::Saturday,
            _ => return None,
        })
    }
}

/// A calendar date and time broken down into its components. Translation of `SDL_DateTime`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct DateTime {
    /// Year
    pub year: i32,
    /// Month [1-12]
    pub month: u8,
    /// Day of the month [1-31]
    pub day: u8,
    /// Hour [0-23]
    pub hour: u8,
    /// Minute [0-59]
    pub minute: u8,
    /// Seconds [0-60] (60 accounts for a possible leap second)
    pub second: u8,
    /// Nanoseconds [0-999999999]
    pub nanosecond: u32,
    /// Day of the week (ignored when converting to a [`Time`])
    pub day_of_week: Weekday,
    /// Seconds east of UTC
    pub utc_offset: i32,
}

impl DateTime {
    /// Translation of the internal `SDL_DateTimeIsValid()`.
    fn validate(&self) -> Result<()> {
        if self.month < 1 || self.month > 12 {
            return Err(err!(
                "Malformed SDL_DateTime: month out of range [1-12], current: {}",
                self.month
            ));
        }
        let days_in_month = days_in_month(self.year, self.month)?;
        if self.day < 1 || self.day > days_in_month {
            return Err(err!(
                "Malformed SDL_DateTime: day of month out of range [1-{}], current: {}",
                days_in_month,
                self.month
            ));
        }
        if self.hour > 23 {
            return Err(err!(
                "Malformed SDL_DateTime: hour out of range [0-23], current: {}",
                self.hour
            ));
        }
        if self.minute > 59 {
            return Err(err!(
                "Malformed SDL_DateTime: minute out of range [0-59], current: {}",
                self.minute
            ));
        }
        if self.second > 60 {
            return Err(err!(
                "Malformed SDL_DateTime: second out of range [0-60], current: {}",
                self.second
            ));
        }
        if self.nanosecond as i64 >= NS_PER_SECOND {
            return Err(err!(
                "Malformed SDL_DateTime: nanosecond out of range [0-999999999], current: {}",
                self.nanosecond
            ));
        }
        Ok(())
    }

    /// Convert to a [`Time`]. `day_of_week` is ignored. Translation of `SDL_DateTimeToTime()`.
    ///
    /// Upstream clamps dates outside the `Time` range and reports an error; here
    /// the error is returned and no clamped value is produced.
    pub fn to_time(&self) -> Result<Time> {
        let max_seconds: i64 = Time::MAX.0 / NS_PER_SECOND - 1;
        let min_seconds: i64 = Time::MIN.0 / NS_PER_SECOND + 1;

        self.validate()?;

        let (days, _, _) = days_from_civil(self.year, self.month as i32, self.day as i32);
        let mut ticks = days * SECONDS_PER_DAY;
        ticks += (((self.hour as i64 * 60) + self.minute as i64) * 60) + self.second as i64
            - self.utc_offset as i64;
        if ticks > max_seconds || ticks < min_seconds {
            return Err(err!(
                "Date out of range for SDL_Time representation; SDL_Time value clamped"
            ));
        }
        Ok(Time(ticks * NS_PER_SECOND + self.nanosecond as i64))
    }
}

impl TryFrom<DateTime> for Time {
    type Error = Error;
    fn try_from(dt: DateTime) -> Result<Time> {
        dt.to_time()
    }
}

impl From<Time> for DateTime {
    fn from(t: Time) -> DateTime {
        t.to_utc_date_time()
    }
}

/// The preferred date format of the current system locale. Translation of `SDL_DateFormat`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum DateFormat {
    /// Year/Month/Day
    #[default]
    YearMonthDay,
    /// Day/Month/Year
    DayMonthYear,
    /// Month/Day/Year
    MonthDayYear,
}

/// The preferred time format of the current system locale. Translation of `SDL_TimeFormat`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum TimeFormat {
    #[default]
    Hour24,
    Hour12,
}

/// The current system locale's preferred date and time formats.
/// Translation of `SDL_GetDateTimeLocalePreferences()`.
///
/// On Unix this is what the current `LC_TIME` locale's formats (from
/// `nl_langinfo()`) start with: a program that never calls `setlocale()`
/// is in the "C" locale, whose `%m/%d/%y` is month first. On Windows it is
/// the user locale's short date and time formats.
pub fn locale_preferences() -> (DateFormat, TimeFormat) {
    // Default to ISO 8061 date format, as it is unambiguous, and 24 hour time.
    let mut date_format = DateFormat::YearMonthDay;
    let mut time_format = TimeFormat::Hour24;

    sys::get_system_time_locale_preferences(&mut date_format, &mut time_format);

    (date_format, time_format)
}

/* The following algorithms are based on those of Howard Hinnant and are in the public domain.
 *
 * http://howardhinnant.github.io/date_algorithms.html
 */

/// Given a calendar date, returns `(days since 1970-01-01, day of week, day of year [0-365])`.
/// Translation of the internal `SDL_CivilToDays()`.
pub fn days_from_civil(mut year: i32, month: i32, day: i32) -> (i64, Weekday, u16) {
    year -= (month <= 2) as i32;
    let era = (if year >= 0 { year } else { year - 399 }) / 400;
    let yoe = (year - era * 400) as u32; // [0, 399]
    let doy = ((153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1) as u32; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    let z = (era as i64) * 146097 + (doe as i64) - 719468;

    let dow = (if z >= -4 {
        (z + 4) % 7
    } else {
        (z + 5) % 7 + 6
    }) as i32;

    // This algorithm considers March 1 to be the first day of the year, so offset by Jan + Feb.
    let day_of_year = if doy > 305 {
        // Day 0 is the first day of the year.
        doy - 306
    } else {
        let doy_offset = 59 + ((year % 4 == 0) && ((year % 100 != 0) || (year % 400 == 0))) as u32;
        doy + doy_offset
    };

    (
        z,
        Weekday::from_index(dow).unwrap_or_default(),
        day_of_year as u16,
    )
}

/// Inverse of [`days_from_civil`]: days since 1970-01-01 to `(year, month, day)`.
pub fn civil_from_days(z: i64) -> (i32, u8, u8) {
    let z = z + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = (z - era * 146097) as u32; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u8; // [1, 31]
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u8; // [1, 12]
    ((y + (m <= 2) as i64) as i32, m, d)
}

/// Number of days in a month for a given year. Translation of `SDL_GetDaysInMonth()`.
pub fn days_in_month(year: i32, month: u8) -> Result<u8> {
    const DAYS_IN_MONTH: [u8; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

    if !(1..=12).contains(&month) {
        return Err(err!("Month out of range [1-12], requested: {month}"));
    }
    let mut days = DAYS_IN_MONTH[(month - 1) as usize];

    /* A leap year occurs every 4 years...
     * but not every 100 years...
     * except for every 400 years.
     */
    if month == 2 && (year % 4 == 0) && ((year % 100 != 0) || (year % 400 == 0)) {
        days += 1;
    }
    Ok(days)
}

fn check_day(year: i32, month: u8, day: u8) -> Result<()> {
    if !(1..=12).contains(&month) {
        return Err(err!("Month out of range [1-12], requested: {month}"));
    }
    let dim = days_in_month(year, month)?;
    if day < 1 || day > dim {
        // (upstream prints the month here, not the day; kept verbatim)
        return Err(err!("Day out of range [1-{dim}], requested: {month}"));
    }
    Ok(())
}

/// Day of year [0-365] for a calendar date. Translation of `SDL_GetDayOfYear()`.
pub fn day_of_year(year: i32, month: u8, day: u8) -> Result<u16> {
    check_day(year, month, day)?;
    Ok(days_from_civil(year, month as i32, day as i32).2)
}

/// Day of week for a calendar date. Translation of `SDL_GetDayOfWeek()`.
pub fn day_of_week(year: i32, month: u8, day: u8) -> Result<Weekday> {
    check_day(year, month, day)?;
    Ok(days_from_civil(year, month as i32, day as i32).1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_is_day_zero() {
        let (z, dow, doy) = days_from_civil(1970, 1, 1);
        assert_eq!(z, 0);
        assert_eq!(dow, Weekday::Thursday);
        assert_eq!(doy, 0);
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
    }

    #[test]
    fn months_and_leap_years() {
        assert_eq!(days_in_month(2024, 2), Ok(29));
        assert_eq!(days_in_month(1900, 2), Ok(28));
        assert_eq!(days_in_month(2000, 2), Ok(29));
        assert!(days_in_month(2023, 13).is_err());
        assert_eq!(day_of_year(2024, 12, 31), Ok(365));
        assert_eq!(day_of_year(2023, 12, 31), Ok(364));
        assert_eq!(day_of_year(2024, 3, 1), Ok(60));
        assert_eq!(day_of_week(2026, 10, 2), Ok(Weekday::Friday));
        assert!(day_of_week(2026, 2, 30).is_err());
    }

    #[test]
    fn datetime_roundtrip() {
        let dt = DateTime {
            year: 2026,
            month: 10,
            day: 2,
            hour: 17,
            minute: 3,
            second: 9,
            nanosecond: 123,
            ..Default::default()
        };
        let t = dt.to_time().unwrap();
        let back = DateTime::from(t);
        assert_eq!((back.year, back.month, back.day), (2026, 10, 2));
        assert_eq!(
            (back.hour, back.minute, back.second, back.nanosecond),
            (17, 3, 9, 123)
        );
        assert_eq!(back.day_of_week, Weekday::Friday);
        let t2 = Time::try_from(DateTime {
            utc_offset: 3600,
            ..dt
        })
        .unwrap();
        assert_eq!(t.as_nanos() - t2.as_nanos(), 3600 * NS_PER_SECOND);
        assert_eq!(
            Time::UNIX_EPOCH.to_utc_date_time(),
            DateTime {
                year: 1970,
                month: 1,
                day: 1,
                day_of_week: Weekday::Thursday,
                ..Default::default()
            }
        );
        // before the epoch
        let early = Time::from_nanos(-1).to_utc_date_time();
        assert_eq!(
            (
                early.year,
                early.month,
                early.day,
                early.hour,
                early.minute,
                early.second,
                early.nanosecond
            ),
            (1969, 12, 31, 23, 59, 59, 999_999_999)
        );
    }

    #[test]
    fn invalid_datetime() {
        let dt = DateTime {
            year: 2026,
            month: 2,
            day: 30,
            ..Default::default()
        };
        let e = dt.to_time().unwrap_err();
        assert!(e.message().contains("day of month"));
        let far = DateTime {
            year: 3000,
            month: 1,
            day: 1,
            ..Default::default()
        };
        assert!(far
            .to_time()
            .unwrap_err()
            .message()
            .contains("out of range"));
    }

    #[test]
    fn windows_filetime_and_system_time() {
        assert_eq!(
            Time::from_windows_filetime(Time::UNIX_EPOCH.to_windows_filetime()),
            Time::UNIX_EPOCH
        );
        let t = Time::from_nanos(1_700_000_000_000_000_000);
        assert_eq!(Time::from_windows_filetime(t.to_windows_filetime()), t);
        let v = Time::from_windows_filetime(u64::MAX);
        assert!(v <= Time::MAX && v > Time::UNIX_EPOCH);
        let st: SystemTime = t.into();
        assert_eq!(Time::try_from(st).unwrap(), t);
        let neg = Time::from_nanos(-5_000_000_000);
        assert_eq!(Time::try_from(SystemTime::from(neg)).unwrap(), neg);
        assert_eq!(neg.as_secs(), -5);
    }

    #[test]
    fn now_is_recent() {
        let t = Time::now().unwrap();
        assert!(t.as_secs() > 1_600_000_000); // after 2020
        let std_now = Time::try_from(SystemTime::now()).unwrap();
        assert!((std_now.as_secs() - t.as_secs()).abs() < 60);
    }

    /// Times spread over the whole range, from a fixed sequence (and the
    /// edges), multiples of `unit` nanoseconds.
    fn sample_times(unit: i64) -> Vec<Time> {
        let mut out = vec![
            Time::MAX,
            Time::from_nanos(-1),
            Time::from_nanos(-150),
            Time::from_nanos(-1_000_000_050),
            Time::UNIX_EPOCH,
            Time::from_nanos(i64::MIN + 2 * NS_PER_SECOND),
        ];
        let mut x: u64 = 0x2545_f491_4f6c_dd1d;
        for _ in 0..2000 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            out.push(Time::from_nanos(x as i64));
        }
        // (upstream's Unix backend wraps around within a second of
        // SDL_MIN_TIME, see unix::time_to_date_time)
        out.retain(|t| t.as_nanos() > i64::MIN + NS_PER_SECOND);
        for t in &mut out {
            *t = Time::from_nanos(t.as_nanos() - t.as_nanos().rem_euclid(unit));
        }
        out
    }

    /// The calendar algorithms give what the platform's UTC conversion
    /// (`gmtime_r()`, `FileTimeToSystemTime()`) gives.
    #[cfg(any(unix, windows))]
    #[test]
    fn utc_matches_platform() {
        for t in sample_times(1) {
            assert_eq!(
                sys::time_to_date_time(t, false).unwrap(),
                t.to_utc_date_time(),
                "{t:?}"
            );
        }
    }

    /// A local date and time, with its UTC offset, is the same point in time.
    #[cfg(any(unix, windows))]
    #[test]
    fn local_time_round_trips() {
        let _l = crate::test_support::test_lock();
        for t in sample_times(1) {
            let dt = t.to_local_date_time().unwrap();
            if !(1678..=2261).contains(&dt.year) {
                continue; // (the ends of the range don't convert back)
            }
            assert_eq!(dt.to_time().unwrap(), t, "{dt:?}");
            assert_eq!(
                dt.day_of_week,
                day_of_week(dt.year, dt.month, dt.day).unwrap()
            );
            assert!(dt.utc_offset.abs() <= 14 * 3600 + 3600, "{dt:?}");
        }
        let now = Time::now().unwrap();
        assert_eq!(now.to_local_date_time().unwrap().to_time().unwrap(), now);
    }

    /// " Y-M-D h:m:s.ns wDOW oOFFSET", as the C reference prints it.
    #[cfg(any(target_os = "linux", target_vendor = "apple"))]
    fn format_dt(dt: Result<DateTime>) -> String {
        match dt {
            Ok(dt) => format!(
                " {}-{}-{} {}:{}:{}.{} w{} o{}",
                dt.year,
                dt.month,
                dt.day,
                dt.hour,
                dt.minute,
                dt.second,
                dt.nanosecond,
                dt.day_of_week as i32,
                dt.utc_offset
            ),
            Err(_) => " err".to_owned(),
        }
    }

    /// `testdata/unix.txt` is upstream's `SDL_TimeToDateTime()` (from
    /// libSDL3 built from the C source, on glibc) for these times in these
    /// time zones, then `SDL_GetDateTimeLocalePreferences()` in these
    /// locales (compiled with `localedef`, used with `uselocale()`).
    /// Upstream built for macOS converts the times identically there.
    #[cfg(any(target_os = "linux", target_vendor = "apple"))]
    const ZONES: [&str; 9] = [
        "UTC",
        "America/New_York",
        "Europe/Berlin",
        "Australia/Lord_Howe",
        "Asia/Kolkata",
        "America/St_Johns",
        "Pacific/Chatham",
        "XYZ-5:30",
        "",
    ];
    #[cfg(any(target_os = "linux", target_vendor = "apple"))]
    const TICKS: [i64; 23] = [
        0,
        -1,
        1,
        -1500000000,
        999999999,
        -999999999,
        -1000000000,
        -1000000001,
        1710054000000000000 - 1,
        1710054000000000000,
        1730613600000000000 - 1,
        1730613600000000000,
        1711846800000000000,
        1704067200123456789,
        1719792000000500000,
        1743865200000000000,
        7258118400000000000,
        -2208988800000000000,
        -5364662400000000000,
        i64::MAX,
        i64::MIN,
        i64::MIN + 1,
        -9223372036000000000,
    ];

    #[cfg(any(target_os = "linux", target_vendor = "apple"))]
    extern "C" {
        /// POSIX `tzset()` (the libc crate doesn't declare it).
        fn tzset();
    }

    #[cfg(any(target_os = "linux", target_vendor = "apple"))]
    #[test]
    fn unix_local_time_matches_upstream() {
        let _l = crate::test_support::test_lock();
        if !std::path::Path::new("/usr/share/zoneinfo/America/New_York").exists() {
            crate::test_support::skip("tzdata", "no /usr/share/zoneinfo");
            return;
        }
        let expected: Vec<&str> = include_str!("testdata/unix.txt").lines().collect();
        let saved = std::env::var("TZ").ok();
        let mut i = 0;
        for zone in ZONES {
            crate::stdlib::setenv_unsafe("TZ", zone, true).unwrap();
            // SAFETY: tzset() rereads TZ; the test lock keeps other tests
            // from converting times meanwhile.
            unsafe { tzset() };
            for ticks in TICKS {
                let t = Time::from_nanos(ticks);
                let got = format!(
                    "{zone} {ticks}:{} |{}",
                    format_dt(t.to_local_date_time()),
                    format_dt(sys::time_to_date_time(t, false))
                );
                assert_eq!(got, expected[i]);
                if ticks > i64::MIN + 1 {
                    // (upstream wraps around next to SDL_MIN_TIME; the
                    // calendar algorithms don't)
                    assert!(got.ends_with(&format_dt(Ok(t.to_utc_date_time()))));
                }
                i += 1;
            }
        }
        match saved {
            Some(tz) => crate::stdlib::setenv_unsafe("TZ", &tz, true).unwrap(),
            None => crate::stdlib::unsetenv_unsafe("TZ").unwrap(),
        }
        // SAFETY: as above.
        unsafe { tzset() };
    }

    /// The preferences in `locale` (`None`: it isn't installed), used on
    /// this thread only.
    #[cfg(any(target_os = "linux", target_vendor = "apple"))]
    fn preferences_in(locale: &str) -> Option<(DateFormat, TimeFormat)> {
        let name = std::ffi::CString::new(locale).unwrap();
        // SAFETY: name is NUL-terminated; the locale object is freed after
        // the thread switches back to the one it used before.
        unsafe {
            let l = libc::newlocale(libc::LC_ALL_MASK, name.as_ptr(), std::ptr::null_mut());
            if l.is_null() {
                return None;
            }
            let old = libc::uselocale(l);
            let prefs = locale_preferences();
            libc::uselocale(old);
            libc::freelocale(l);
            Some(prefs)
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn unix_locale_preferences_match_upstream() {
        let _l = crate::test_support::test_lock();
        let raw = |prefs: (DateFormat, TimeFormat)| {
            let df = match prefs.0 {
                DateFormat::YearMonthDay => 0,
                DateFormat::DayMonthYear => 1,
                DateFormat::MonthDayYear => 2,
            };
            format!("{} {}", df, prefs.1 as i32)
        };
        let expected = |locale: &str| {
            include_str!("testdata/unix.txt")
                .lines()
                .find_map(|l| l.strip_prefix(&format!("locale {locale}: ")))
                .unwrap()
        };
        // Built into glibc.
        for locale in ["C", "POSIX"] {
            assert_eq!(raw(preferences_in(locale).unwrap()), expected(locale));
        }

        // Compiled for the test.
        let tmp = crate::test_support::TempDir::new("locales");
        let locales = ["en_US", "en_GB", "de_DE", "ja_JP", "en_DK"];
        for l in locales {
            let status = std::process::Command::new("localedef")
                .args(["-i", l, "-f", "UTF-8", &tmp.path(&format!("{l}.UTF-8"))])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
            if !status.is_ok_and(|s| s.success()) {
                crate::test_support::skip("locales", format!("localedef can't compile {l}"));
                return;
            }
        }
        let saved = std::env::var("LOCPATH").ok();
        crate::stdlib::setenv_unsafe("LOCPATH", &tmp.0, true).unwrap();
        for l in locales {
            let name = format!("{l}.UTF-8");
            let prefs = preferences_in(&name);
            assert_eq!(prefs.map(raw).as_deref(), Some(expected(&name)), "{name}");
        }
        match saved {
            Some(p) => crate::stdlib::setenv_unsafe("LOCPATH", &p, true).unwrap(),
            None => crate::stdlib::unsetenv_unsafe("LOCPATH").unwrap(),
        }
    }

    /// What upstream built for macOS reports in the locales macOS ships
    /// (its en_US time format is 24-hour, `%H:%M:%S`).
    #[cfg(target_vendor = "apple")]
    #[test]
    fn macos_locale_preferences_match_upstream() {
        let _l = crate::test_support::test_lock();
        let expected = [
            ("C", (DateFormat::MonthDayYear, TimeFormat::Hour24)),
            ("POSIX", (DateFormat::MonthDayYear, TimeFormat::Hour24)),
            ("C.UTF-8", (DateFormat::MonthDayYear, TimeFormat::Hour24)),
            (
                "en_US.UTF-8",
                (DateFormat::MonthDayYear, TimeFormat::Hour24),
            ),
            (
                "en_GB.UTF-8",
                (DateFormat::DayMonthYear, TimeFormat::Hour24),
            ),
            (
                "de_DE.UTF-8",
                (DateFormat::DayMonthYear, TimeFormat::Hour24),
            ),
            (
                "ja_JP.UTF-8",
                (DateFormat::YearMonthDay, TimeFormat::Hour24),
            ),
        ];
        for (locale, prefs) in expected {
            assert_eq!(preferences_in(locale), Some(prefs), "{locale}");
        }
    }

    /// A local time's UTC offset doesn't depend on the time's fraction of
    /// a second. (It only did in a time zone east of UTC: under Wine, run
    /// with `TZ` set to one, such as `TZ=Asia/Kolkata`.)
    #[cfg(windows)]
    #[test]
    fn windows_utc_offset_is_whole_seconds() {
        let base = 1704067200000000000; // 2024-01-01 00:00:00 UTC
        let offset = |ns: i64| {
            sys::time_to_date_time(Time::from_nanos(base + ns), true)
                .unwrap()
                .utc_offset
        };
        let whole = offset(0);
        assert_eq!(whole % 60, 0, "{whole}");
        for ns in [1, 999, 500_000, 999_999, 123_456_789, 999_999_999] {
            assert_eq!(offset(ns), whole, "{ns} ns");
        }
    }

    /// The preferences follow the first `y`/`d`/`M` of the user locale's
    /// short date format and the first `H`/`h` of its time format.
    #[cfg(windows)]
    #[test]
    fn windows_locale_preferences() {
        use windows_sys::Win32::Globalization::{
            GetLocaleInfoW, LOCALE_SSHORTDATE, LOCALE_STIMEFORMAT, LOCALE_USER_DEFAULT,
        };
        let info = |lctype| {
            let mut buf = [0u16; 80];
            // SAFETY: buf holds the given number of units.
            let n = unsafe { GetLocaleInfoW(LOCALE_USER_DEFAULT, lctype, buf.as_mut_ptr(), 80) };
            String::from_utf16_lossy(&buf[..(n.max(1) - 1) as usize])
        };
        let date = info(LOCALE_SSHORTDATE);
        let time = info(LOCALE_STIMEFORMAT);
        let df = match date.chars().find(|c| matches!(c, 'y' | 'd' | 'M')) {
            Some('d') => DateFormat::DayMonthYear,
            Some('M') => DateFormat::MonthDayYear,
            _ => DateFormat::YearMonthDay,
        };
        let tf = match time.chars().find(|c| matches!(c, 'H' | 'h')) {
            Some('h') => TimeFormat::Hour12,
            _ => TimeFormat::Hour24,
        };
        assert!(!date.is_empty() && !time.is_empty());
        assert_eq!(locale_preferences(), (df, tf), "{date:?} {time:?}");
    }
}
