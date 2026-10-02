// Rust translation of src/time/SDL_time.c and include/SDL3/SDL_time.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Realtime clock and calendar date/time routines.
//!
//! [`Time`] is a point in time as nanoseconds since the Unix epoch (`SDL_Time`);
//! [`DateTime`] breaks one down into human-understandable components
//! (`SDL_DateTime`). The calendar algorithms are Howard Hinnant's public-domain
//! ones, exactly as upstream uses them.

use std::fmt;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::err;
use crate::error::{Error, Result};
use crate::timer::NS_PER_SECOND;

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
        Time::try_from(SystemTime::now())
    }

    /// Break down into calendar components in UTC.
    /// Translation of `SDL_TimeToDateTime(ticks, dt, false)`.
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
    /// Upstream implements this per platform (`localtime_r`,
    /// `FileTimeToSystemTime`, ...). Until that platform layer is translated
    /// this returns [`ErrorKind::Unsupported`](crate::ErrorKind::Unsupported).
    pub fn to_local_date_time(self) -> Result<DateTime> {
        Err(Error::unsupported())
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
/// Without a platform backend this is the documented default: ISO 8601 date
/// order (unambiguous) and 24 hour time.
pub fn locale_preferences() -> (DateFormat, TimeFormat) {
    (DateFormat::YearMonthDay, TimeFormat::Hour24)
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
        assert_eq!(
            Time::UNIX_EPOCH.to_local_date_time().unwrap_err().kind(),
            crate::ErrorKind::Unsupported
        );
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
        assert_eq!(
            locale_preferences(),
            (DateFormat::YearMonthDay, TimeFormat::Hour24)
        );
    }
}
