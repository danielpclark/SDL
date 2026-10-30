// Rust translation of src/power/android/SDL_syspower.c from Simple
// DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Android power backend: the battery's sticky intent
//! (`ACTION_BATTERY_CHANGED`: plugged, status, present, level and scale),
//! which the app's activity gives (`core::android::get_power_info()`,
//! `Android_JNI_GetPowerInfo()`); the time left is never known. A program
//! that isn't an app (no Java VM) gets [`PowerState::Unknown`], as when the
//! intent can't be read.

use std::time::Duration;

use super::{PowerInfo, PowerState};

/// Translation of `SDL_GetPowerInfo_Android()`: always definitive.
pub(super) fn get_power_info_android() -> Option<PowerInfo> {
    let state;
    let mut seconds;
    let mut percent;

    match crate::core::android::get_power_info() {
        Some(info) => {
            seconds = info.seconds;
            percent = info.percent;
            if info.plugged {
                if info.charged {
                    state = PowerState::Charged;
                } else if info.battery {
                    state = PowerState::Charging;
                } else {
                    state = PowerState::NoBattery;
                    seconds = -1;
                    percent = -1;
                }
            } else {
                state = PowerState::OnBattery;
            }
        }
        None => {
            state = PowerState::Unknown;
            seconds = -1;
            percent = -1;
        }
    }

    Some(PowerInfo {
        state,
        time_left: u64::try_from(seconds).ok().map(Duration::from_secs),
        percent: u8::try_from(percent).ok(),
    })
}
