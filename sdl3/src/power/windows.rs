// Rust translation of src/power/windows/SDL_syspower.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Windows power backend: `GetSystemPowerStatus()`.

use super::{PowerInfo, PowerState};
use windows_sys::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};

/// Translation of `SDL_GetPowerInfo_Windows()`.
pub(super) fn get_power_info_windows() -> Option<PowerInfo> {
    // SAFETY: SYSTEM_POWER_STATUS is plain data; all zeroes is a valid value.
    let mut status: SYSTEM_POWER_STATUS = unsafe { std::mem::zeroed() };

    // This API should exist back to Win95.
    // SAFETY: status is a valid out-parameter.
    let ok = unsafe { GetSystemPowerStatus(&mut status) } != 0;
    Some(power_info_from_status(ok.then_some(&status)))
}

/// The answer for a `GetSystemPowerStatus()` result (`None`: the call
/// failed). Always the definitive answer on Windows.
pub(super) fn power_info_from_status(status: Option<&SYSTEM_POWER_STATUS>) -> PowerInfo {
    let mut need_details = false;

    let state = match status {
        None => {
            // !!! FIXME: push GetLastError() into SDL_GetError()
            PowerState::Unknown
        }
        Some(status) if status.BatteryFlag == 0xFF => PowerState::Unknown, // unknown state
        Some(status) if status.BatteryFlag & (1 << 7) != 0 => PowerState::NoBattery, // no battery
        Some(status) if status.BatteryFlag & (1 << 3) != 0 => {
            // charging
            need_details = true;
            PowerState::Charging
        }
        Some(status) if status.ACLineStatus == 1 => {
            need_details = true;
            PowerState::Charged // on AC, not charging.
        }
        Some(_) => {
            need_details = true;
            PowerState::OnBattery // not on AC.
        }
    };

    let mut percent = None;
    let mut time_left = None;
    if let (true, Some(status)) = (need_details, status) {
        let pct = status.BatteryLifePercent;
        let secs = status.BatteryLifeTime;

        if pct != 255 {
            // 255 == unknown
            percent = Some(pct.min(100)); // clamp between 0%, 100%
        }
        if secs != 0xFFFFFFFF {
            // ((DWORD)-1) == unknown
            // (upstream stores it in an int: above i32::MAX, it's negative,
            // which reads as unknown.)
            time_left = i32::try_from(secs)
                .ok()
                .map(|s| std::time::Duration::from_secs(s as u64));
        }
    }

    PowerInfo {
        state,
        time_left,
        percent,
    }
}
