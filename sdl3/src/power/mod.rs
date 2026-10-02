// Rust translation of src/power/SDL_power.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Power management: battery state queries.
//!
//! Direct translation of `SDL_power.c`. The platform backends
//! (`src/power/linux`, `windows`, `macos`, ...) are not translated yet, so
//! the dispatcher behaves like an `SDL_POWER_DISABLED` build.

use std::time::Duration;

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

/// The ordered list of platform implementations to try (empty until the
/// platform backends are translated; see docs/ROADMAP.md).
static IMPLEMENTATIONS: &[GetPowerInfoImpl] = &[];

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
mod tests {
    use super::*;

    #[test]
    fn no_backends_yet() {
        let info = power_info();
        assert_eq!(info.state, PowerState::Unknown);
        assert_eq!(info.time_left, None);
        assert_eq!(info.percent, None);
    }
}
