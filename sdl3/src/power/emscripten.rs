// Rust translation of src/power/emscripten/SDL_syspower.c from Simple
// DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Emscripten power backend: the browser's Battery Status API
//! (`navigator.getBattery()`), through `emscripten_get_battery_status()`.
//!
//! Without the API (Firefox, Safari, node) there's no answer, and
//! [`power_info`](super::power_info) reports [`PowerState::Unknown`]. With
//! it, emscripten asks for the battery on the first call, which then has
//! no data yet (`getBattery()` answers through a promise, once control is
//! back in the event loop); later calls read the battery it got.

use std::time::Duration;

use sdl3_emscripten::sys::{self, EmscriptenBatteryEvent};

use super::{PowerInfo, PowerState};

/// Translation of `SDL_GetPowerInfo_Emscripten()`.
pub(super) fn get_power_info_emscripten() -> Option<PowerInfo> {
    // FIXME (upstream): EMSCRIPTEN_RESULT_NO_DATA, the first call's answer
    // while getBattery()'s promise is pending, isn't checked: the state is
    // read from a structure nothing filled in. In C it's uninitialized; here
    // it's zeroed, which reads as a battery at 0%, discharging, 0 s left
    // (what upstream's C reported under node, where its stack was zeroes).
    let mut battery_state = EmscriptenBatteryEvent {
        chargingTime: 0.0,
        dischargingTime: 0.0,
        level: 0.0,
        charging: false,
    };

    // SAFETY: a valid structure for emscripten to fill in.
    if unsafe { sys::emscripten_get_battery_status(&mut battery_state) }
        == sys::EMSCRIPTEN_RESULT_NOT_SUPPORTED
    {
        return None;
    }

    Some(power_info_from_battery(&battery_state))
}

/// The answer for a battery's state, from `SDL_GetPowerInfo_Emscripten()`.
fn power_info_from_battery(battery_state: &EmscriptenBatteryEvent) -> PowerInfo {
    let have_battery =
        battery_state.level != 1.0 || !battery_state.charging || battery_state.chargingTime != 0.0;

    if !have_battery {
        return PowerInfo {
            state: PowerState::NoBattery,
            time_left: None,
            percent: None,
        };
    }

    let state = if battery_state.charging {
        if battery_state.chargingTime == 0.0 {
            PowerState::Charged
        } else {
            PowerState::Charging
        }
    } else {
        PowerState::OnBattery
    };

    // (C's (int) conversion of a double out of range, which emcc's wasm
    // makes saturating, as Rust's `as`: Infinity, the time left while
    // charging, is INT_MAX, and NaN is 0)
    let seconds = battery_state.dischargingTime as i32;
    // FIXME (upstream): the level (0 to 1) is converted to an int before
    // it's multiplied, so the percentage is 0 unless the battery is full.
    let percent = (battery_state.level as i32) * 100;

    PowerInfo {
        state,
        // (a negative time, which the API never reports, is unknown here)
        time_left: u64::try_from(seconds).ok().map(Duration::from_secs),
        percent: u8::try_from(percent).ok(),
    }
}
