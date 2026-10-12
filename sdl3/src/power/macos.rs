// Rust translation of src/power/macos/SDL_syspower.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The macOS power backend: IOKit's power sources (`IOPSCopyPowerSourcesInfo()`).

use std::ffi::c_void;

use super::{PowerInfo, PowerState};
use crate::core::apple::foundation::{
    cf_string, kCFBooleanFalse, kCFBooleanTrue, CFArrayGetCount, CFArrayGetValueAtIndex,
    CFArrayRef, CFDictionaryGetValueIfPresent, CFDictionaryRef, CFNumberGetValue, CFNumberRef,
    CFOptionFlags, CFStringRef, CFTypeRef, CfRef, K_CF_NUMBER_SINT32_TYPE,
};

/// `kCFCompareEqualTo`.
const K_CF_COMPARE_EQUAL_TO: isize = 0;

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOPSCopyPowerSourcesInfo() -> CFTypeRef;
    fn IOPSCopyPowerSourcesList(blob: CFTypeRef) -> CFArrayRef;
    fn IOPSGetPowerSourceDescription(blob: CFTypeRef, ps: CFTypeRef) -> CFDictionaryRef;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFStringCompare(a: CFStringRef, b: CFStringRef, options: CFOptionFlags) -> isize;
}

// (IOPSKeys.h)
const K_IOPS_IS_PRESENT_KEY: &str = "Is Present";
const K_IOPS_POWER_SOURCE_STATE_KEY: &str = "Power Source State";
const K_IOPS_AC_POWER_VALUE: &str = "AC Power";
const K_IOPS_BATTERY_POWER_VALUE: &str = "Battery Power";
const K_IOPS_IS_CHARGING_KEY: &str = "Is Charging";
const K_IOPS_MAX_CAPACITY_KEY: &str = "Max Capacity";
const K_IOPS_TIME_TO_EMPTY_KEY: &str = "Time to Empty";
const K_IOPS_CURRENT_CAPACITY_KEY: &str = "Current Capacity";

/// What `checkps()` accumulates over the power sources.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Accumulated {
    have_ac: bool,
    have_battery: bool,
    charging: bool,
    seconds: i32,
    percent: i32,
}

impl Default for Accumulated {
    fn default() -> Accumulated {
        Accumulated {
            have_ac: false,
            have_battery: false,
            charging: false,
            seconds: -1,
            percent: -1,
        }
    }
}

// CoreFoundation is so verbose...
/// `STRMATCH()`.
///
/// # Safety
///
/// `a` must be a valid `CFString`.
unsafe fn strmatch(a: CFStringRef, b: &str) -> bool {
    let b = cf_string(b);
    // SAFETY: two valid strings (the caller's contract).
    unsafe { CFStringCompare(a, b.get(), 0) == K_CF_COMPARE_EQUAL_TO }
}

/// `GETVAL()`: the value of `key` in `dict`, if present.
///
/// # Safety
///
/// `dict` must be a valid `CFDictionary`.
unsafe fn getval(dict: CFDictionaryRef, key: &str) -> Option<*const c_void> {
    let key = cf_string(key);
    let mut value = std::ptr::null();
    // SAFETY: the caller's contract; `value` is a valid out-parameter.
    let found = unsafe { CFDictionaryGetValueIfPresent(dict, key.get(), &mut value) } != 0;
    found.then_some(value)
}

/// `CFNumberGetValue(numval, kCFNumberSInt32Type, &val)` with `val`
/// starting at -1.
///
/// # Safety
///
/// `numval` must be a valid `CFNumber`.
unsafe fn sint32(numval: CFNumberRef) -> i32 {
    let mut val: i32 = -1;
    // SAFETY: the caller's contract; the out-parameter is of the type named.
    unsafe {
        CFNumberGetValue(
            numval,
            K_CF_NUMBER_SINT32_TYPE,
            &mut val as *mut i32 as *mut c_void,
        )
    };
    val
}

/// Note that AC power sources also include a laptop battery it is charging.
/// Translation of `checkps()`.
///
/// # Safety
///
/// `dict` must be a valid power source description (a `CFDictionary`
/// with `CFString` keys, whose values have IOPSKeys.h's types).
unsafe fn checkps(dict: CFDictionaryRef, acc: &mut Accumulated) {
    // SAFETY: the caller's contract, for every lookup and value.
    unsafe {
        let mut charge = false;
        let mut choose = false;
        let mut is_ac = false;
        let mut secs = -1;
        let mut maxpct = -1;
        let mut pct = -1;

        if let Some(bval) = getval(dict, K_IOPS_IS_PRESENT_KEY) {
            if bval == kCFBooleanFalse {
                return; // nothing to see here.
            }
        }

        let Some(strval) = getval(dict, K_IOPS_POWER_SOURCE_STATE_KEY) else {
            return;
        };

        if strmatch(strval, K_IOPS_AC_POWER_VALUE) {
            is_ac = true;
            acc.have_ac = true;
        } else if !strmatch(strval, K_IOPS_BATTERY_POWER_VALUE) {
            return; // not a battery?
        }

        if getval(dict, K_IOPS_IS_CHARGING_KEY) == Some(kCFBooleanTrue) {
            charge = true;
        }

        if let Some(numval) = getval(dict, K_IOPS_MAX_CAPACITY_KEY) {
            let val = sint32(numval);
            if val > 0 {
                acc.have_battery = true;
                maxpct = val;
            }
        }

        // (upstream reads kIOPSMaxCapacityKey a second time, to the same
        // effect)
        if let Some(numval) = getval(dict, K_IOPS_MAX_CAPACITY_KEY) {
            let val = sint32(numval);
            if val > 0 {
                acc.have_battery = true;
                maxpct = val;
            }
        }

        if let Some(numval) = getval(dict, K_IOPS_TIME_TO_EMPTY_KEY) {
            let mut val = sint32(numval);

            // macOS reports 0 minutes until empty if you're plugged in. :(
            if val == 0 && is_ac {
                val = -1; // !!! FIXME: calc from timeToFull and capacity?
            }

            secs = val;
            if secs > 0 {
                secs = secs.wrapping_mul(60); // value is in minutes, so convert to seconds.
            }
        }

        if let Some(numval) = getval(dict, K_IOPS_CURRENT_CAPACITY_KEY) {
            pct = sint32(numval);
        }

        if pct > 0 && maxpct > 0 {
            pct = ((pct as f64 / maxpct as f64) * 100.0) as i32;
        }

        if pct > 100 {
            pct = 100;
        }

        /*
         * We pick the battery that claims to have the most minutes left.
         *  (failing a report of minutes, we'll take the highest percent.)
         */
        if secs < 0 && acc.seconds < 0 {
            if pct < 0 && acc.percent < 0 {
                choose = true; // at least we know there's a battery.
            }
            if pct > acc.percent {
                choose = true;
            }
        } else if secs > acc.seconds {
            choose = true;
        }

        if choose {
            acc.seconds = secs;
            acc.percent = pct;
            acc.charging = charge;
        }
    }
}

/// The state for what the power sources showed.
fn power_state(acc: &Accumulated) -> PowerState {
    if !acc.have_battery {
        PowerState::NoBattery
    } else if acc.charging {
        PowerState::Charging
    } else if acc.have_ac {
        PowerState::Charged
    } else {
        PowerState::OnBattery
    }
}

/// `state/seconds/percent` as a [`PowerInfo`] (-1 is unknown).
fn power_info(state: PowerState, seconds: i32, percent: i32) -> PowerInfo {
    PowerInfo {
        state,
        time_left: u64::try_from(seconds)
            .ok()
            .map(std::time::Duration::from_secs),
        percent: u8::try_from(percent).ok(),
    }
}

/// Translation of `SDL_GetPowerInfo_MacOSX()`.
pub(super) fn get_power_info_macosx() -> Option<PowerInfo> {
    // SAFETY: IOKit returns an owned snapshot (or NULL).
    let blob = unsafe { CfRef::from_owned(IOPSCopyPowerSourcesInfo()) };

    let mut seconds = -1;
    let mut percent = -1;
    let mut state = PowerState::Unknown;

    if let Some(blob) = blob {
        // SAFETY: a valid snapshot; the list is owned.
        let list = unsafe { CfRef::from_owned(IOPSCopyPowerSourcesList(blob.get())) };
        if let Some(list) = list {
            // don't CFRelease() the list items, or dictionaries!
            let mut acc = Accumulated::default();
            // SAFETY: a valid array of power source handles of `blob`.
            unsafe {
                let total = CFArrayGetCount(list.get());
                for i in 0..total {
                    let ps = CFArrayGetValueAtIndex(list.get(), i);
                    let dict = IOPSGetPowerSourceDescription(blob.get(), ps);
                    if !dict.is_null() {
                        checkps(dict, &mut acc);
                    }
                }
            }

            seconds = acc.seconds;
            percent = acc.percent;
            state = power_state(&acc);
        }
    }

    Some(power_info(state, seconds, percent)) // always the definitive answer on macOS.
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::apple::foundation::{cf_dictionary, cf_number_i32};

    /// Run checkps() over descriptions given as (key, value) pairs.
    fn check(sources: &[&[(&str, Value)]]) -> (PowerState, i32, i32) {
        let mut acc = Accumulated::default();
        for pairs in sources {
            let keys: Vec<CfRef> = pairs.iter().map(|(k, _)| cf_string(k)).collect();
            let values: Vec<CfRef> = pairs
                .iter()
                .map(|(_, v)| match v {
                    Value::Str(s) => cf_string(s),
                    Value::Int(n) => cf_number_i32(*n),
                    Value::Bool(b) => unsafe {
                        CfRef::retain(if *b { kCFBooleanTrue } else { kCFBooleanFalse }).unwrap()
                    },
                })
                .collect();
            let dict = cf_dictionary(
                &keys
                    .iter()
                    .zip(&values)
                    .map(|(k, v)| (k.get(), v.get()))
                    .collect::<Vec<_>>(),
            );
            unsafe { checkps(dict.get(), &mut acc) };
        }
        (power_state(&acc), acc.seconds, acc.percent)
    }

    enum Value {
        Str(&'static str),
        Int(i32),
        Bool(bool),
    }
    use Value::*;

    #[test]
    fn power_sources() {
        // No sources: no battery.
        assert_eq!(check(&[]), (PowerState::NoBattery, -1, -1));
        // A laptop on battery: 75 of 150, 90 minutes left.
        let battery: &[(&str, Value)] = &[
            ("Is Present", Bool(true)),
            ("Power Source State", Str("Battery Power")),
            ("Is Charging", Bool(false)),
            ("Max Capacity", Int(150)),
            ("Current Capacity", Int(75)),
            ("Time to Empty", Int(90)),
        ];
        assert_eq!(check(&[battery]), (PowerState::OnBattery, 5400, 50));
        // Plugged in, charging: 0 minutes to empty is unknown on AC.
        let charging: &[(&str, Value)] = &[
            ("Power Source State", Str("AC Power")),
            ("Is Charging", Bool(true)),
            ("Max Capacity", Int(100)),
            ("Current Capacity", Int(120)),
            ("Time to Empty", Int(0)),
        ];
        assert_eq!(check(&[charging]), (PowerState::Charging, -1, 100));
        // Plugged in, full, not charging.
        let full: &[(&str, Value)] = &[
            ("Power Source State", Str("AC Power")),
            ("Max Capacity", Int(100)),
            ("Current Capacity", Int(100)),
        ];
        assert_eq!(check(&[full]), (PowerState::Charged, -1, 100));
        // A source that isn't present, or isn't a battery, is ignored.
        let absent: &[(&str, Value)] = &[
            ("Is Present", Bool(false)),
            ("Power Source State", Str("Battery Power")),
            ("Max Capacity", Int(100)),
        ];
        let ups: &[(&str, Value)] = &[
            ("Power Source State", Str("Off Line")),
            ("Max Capacity", Int(100)),
        ];
        assert_eq!(check(&[absent, ups]), (PowerState::NoBattery, -1, -1));
        // Two batteries: the one with the most minutes left wins.
        let other: &[(&str, Value)] = &[
            ("Power Source State", Str("Battery Power")),
            ("Max Capacity", Int(100)),
            ("Current Capacity", Int(90)),
            ("Time to Empty", Int(30)),
        ];
        assert_eq!(check(&[battery, other]), (PowerState::OnBattery, 5400, 50));
        assert_eq!(check(&[other, battery]), (PowerState::OnBattery, 5400, 50));
    }

    #[test]
    fn this_machine() {
        // Always definitive; whatever the hardware.
        let info = get_power_info_macosx().unwrap();
        assert_ne!(info.state, PowerState::Unknown);
        assert_ne!(info.state, PowerState::Error);
        if info.state == PowerState::NoBattery {
            assert_eq!(info.percent, None);
        }
    }
}
