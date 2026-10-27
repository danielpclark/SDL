// Rust translation of src/power/SDL_power.c and SDL_syspower.h from Simple
// DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Power management: battery state queries.
//!
//! Direct translation of `SDL_power.c`, with the Linux backends (UPower
//! over D-Bus, `/sys/class/power_supply`, `/proc/acpi` and `/proc/apm`,
//! tried in that order), the Windows one (`GetSystemPowerStatus()`) and
//! the UIKit one (UIDevice's battery monitoring; no battery on tvOS), the
//! macOS one (IOKit's power sources) and the Emscripten one (the
//! browser's Battery Status API). Elsewhere (the Android, ... backends are
//! not translated) no backend is compiled in, so the answer is
//! [`PowerState::Unknown`], as in an `SDL_POWER_DISABLED` build.

use std::time::Duration;

#[cfg(target_os = "emscripten")]
mod emscripten;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
/// (`update_battery_monitoring()` is for the UIKit video driver.)
#[cfg(all(target_vendor = "apple", not(target_os = "macos")))]
pub(crate) mod uikit;
#[cfg(windows)]
mod windows;

/// The basic state for the system's power supply. Translation of `SDL_PowerState`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum PowerState {
    /// error determining power status
    Error,
    /// cannot determine power status
    Unknown,
    /// Not plugged in, running on the battery
    OnBattery,
    /// Plugged in, no battery available
    NoBattery,
    /// Plugged in, charging battery
    Charging,
    /// Plugged in, battery charged
    Charged,
}

/// Result of [`power_info`]. Translation of `SDL_GetPowerInfo()`'s return value
/// and its `seconds`/`percent` out-parameters (`-1` becomes `None`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PowerInfo {
    pub state: PowerState,
    /// Battery life left, if known and running on battery.
    pub time_left: Option<Duration>,
    /// Battery charge left in percent, if known and running on battery.
    pub percent: Option<u8>,
}

/// A platform implementation: returns `Some` if it has a definitive answer,
/// `None` to try the next implementation. Translation of `SDL_GetPowerInfo_Impl`.
pub(crate) type GetPowerInfoImpl = fn() -> Option<PowerInfo>;

/// The ordered list of platform implementations to try. Translation of
/// `implementations[]`.
static IMPLEMENTATIONS: &[GetPowerInfoImpl] = &[
    // in order of preference. More than could work.
    #[cfg(target_os = "linux")]
    linux::get_power_info_linux_org_freedesktop_upower,
    #[cfg(target_os = "linux")]
    linux::get_power_info_linux_sys_class_power_supply,
    #[cfg(target_os = "linux")]
    linux::get_power_info_linux_proc_acpi,
    #[cfg(target_os = "linux")]
    linux::get_power_info_linux_proc_apm,
    // handles Win32, Win64, PocketPC.
    #[cfg(windows)]
    windows::get_power_info_windows,
    // handles iPhone/iPad/etc
    #[cfg(all(target_vendor = "apple", not(target_os = "macos")))]
    uikit::get_power_info_uikit,
    // handles macOS, Darwin.
    #[cfg(target_os = "macos")]
    macos::get_power_info_macosx,
    // handles Emscripten
    #[cfg(target_os = "emscripten")]
    emscripten::get_power_info_emscripten,
];

/// Get the current power supply details. Translation of `SDL_GetPowerInfo()`.
pub fn power_info() -> PowerInfo {
    for imp in IMPLEMENTATIONS {
        if let Some(info) = imp() {
            return info;
        }
    }
    // nothing was definitive.
    PowerInfo {
        state: PowerState::Unknown,
        time_left: None,
        percent: None,
    }
}

#[cfg(test)]
mod tests;
