// Rust translation of src/power/uikit/SDL_syspower.m and SDL_syspower.h
// from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The UIKit power backend: UIDevice's battery monitoring, turned on for
//! a query and off again once the app hasn't asked for a while. tvOS
//! devices have no battery.
//!
//! The iOS Simulator has no battery either: UIDevice reports its state as
//! unknown, and so does this backend there (as upstream's C does); its
//! battery monitoring stays off.

use super::{PowerInfo, PowerState};

#[cfg(not(target_os = "tvos"))]
mod battery {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::{PowerInfo, PowerState};
    use crate::core::apple::objc::{autoreleasepool, msg_send, Bool, NO, YES};
    use crate::core::apple::uikit::{
        current_device, UIDeviceBatteryState, UI_DEVICE_BATTERY_STATE_CHARGING,
        UI_DEVICE_BATTERY_STATE_FULL, UI_DEVICE_BATTERY_STATE_UNPLUGGED,
    };

    /// turn off the battery monitor if it's been more than X ms since last check.
    const BATTERY_MONITORING_TIMEOUT: u64 = 3000;
    /// `SDL_UIKitLastPowerInfoQuery`: the ticks of the last query, 0 while
    /// monitoring is off.
    pub(super) static SDL_UIKIT_LAST_POWER_INFO_QUERY: AtomicU64 = AtomicU64::new(0);

    /// `uidev.isBatteryMonitoringEnabled`.
    pub(super) fn is_battery_monitoring_enabled(uidev: crate::core::apple::objc::Id) -> bool {
        // SAFETY: a UIDevice property.
        let enabled: Bool = unsafe { msg_send![uidev, isBatteryMonitoringEnabled] };
        enabled.is_true()
    }

    /// Translation of `SDL_UIKit_UpdateBatteryMonitoring()`.
    // (the UIKit video driver's OpenGL ES swap calls it: not on visionOS
    // or Mac Catalyst, which have no OpenGL ES)
    #[cfg_attr(
        all(not(test), any(target_os = "visionos", target_abi = "macabi")),
        allow(dead_code)
    )]
    pub(crate) fn update_battery_monitoring() {
        let last = SDL_UIKIT_LAST_POWER_INFO_QUERY.load(Ordering::Relaxed);
        if last != 0 && crate::timer::ticks_ms() >= last + BATTERY_MONITORING_TIMEOUT {
            let uidev = current_device();
            // FIXME (upstream): the iOS Simulator's UIDevice keeps battery
            // monitoring off whatever it's told, so this fails there, once
            // the last query is older than the timeout.
            crate::sdl_assert!(is_battery_monitoring_enabled(uidev));
            // SAFETY: a UIDevice property.
            unsafe {
                let () = msg_send![uidev, setBatteryMonitoringEnabled: NO];
            }
            SDL_UIKIT_LAST_POWER_INFO_QUERY.store(0, Ordering::Relaxed);
        }
    }

    /// Translation of `SDL_GetPowerInfo_UIKit()` (not tvOS).
    pub(super) fn get_power_info_uikit() -> Option<PowerInfo> {
        autoreleasepool(|| {
            let uidev = current_device();

            if SDL_UIKIT_LAST_POWER_INFO_QUERY.load(Ordering::Relaxed) == 0 {
                crate::sdl_assert!(!is_battery_monitoring_enabled(uidev));
                // SAFETY: a UIDevice property.
                unsafe {
                    let () = msg_send![uidev, setBatteryMonitoringEnabled: YES];
                }
            }

            /* UIKit_GL_SwapWindow() (etc) will check this and disable the battery
             *  monitoring if the app hasn't queried it in the last X seconds.
             *  Apparently monitoring the battery burns battery life.  :)
             *  Apple's docs say not to monitor the battery unless you need it.
             */
            SDL_UIKIT_LAST_POWER_INFO_QUERY.store(crate::timer::ticks_ms(), Ordering::Relaxed);

            // SAFETY: UIDevice properties.
            let (battery_state, level): (UIDeviceBatteryState, f32) = unsafe {
                (
                    msg_send![uidev, batteryState],
                    msg_send![uidev, batteryLevel],
                )
            };
            Some(power_info_from_battery(battery_state, level))
        })
    }

    /// The answer for UIDevice's battery state and level (0 to 1, or -1
    /// when unknown), from `SDL_GetPowerInfo_UIKit()`.
    pub(super) fn power_info_from_battery(
        battery_state: UIDeviceBatteryState,
        level: f32,
    ) -> PowerInfo {
        let state = match battery_state {
            UI_DEVICE_BATTERY_STATE_CHARGING => PowerState::Charging,
            UI_DEVICE_BATTERY_STATE_FULL => PowerState::Charged,
            UI_DEVICE_BATTERY_STATE_UNPLUGGED => PowerState::OnBattery,
            // UIDeviceBatteryStateUnknown, and anything else
            _ => PowerState::Unknown,
        };

        let percent = if level < 0.0 {
            -1
        } else {
            ((level * 100.0) + 0.5) as i32
        };

        PowerInfo {
            state,
            time_left: None, // no API to estimate this in UIKit.
            percent: u8::try_from(percent).ok(),
        }
    }
}

#[cfg(not(target_os = "tvos"))]
#[cfg_attr(
    all(not(test), any(target_os = "visionos", target_abi = "macabi")),
    allow(unused_imports)
)]
// (for the UIKit video driver's OpenGL ES swap)
pub(crate) use battery::update_battery_monitoring;

/// Translation of `SDL_UIKit_UpdateBatteryMonitoring()` (tvOS).
#[cfg(target_os = "tvos")]
pub(crate) fn update_battery_monitoring() {
    // Do nothing.
}

/// Translation of `SDL_GetPowerInfo_UIKit()`: always the definitive
/// answer on iOS.
pub(super) fn get_power_info_uikit() -> Option<PowerInfo> {
    #[cfg(target_os = "tvos")]
    {
        Some(PowerInfo {
            state: PowerState::NoBattery,
            time_left: None,
            percent: None,
        })
    }
    #[cfg(not(target_os = "tvos"))]
    battery::get_power_info_uikit()
}

#[cfg(all(test, not(target_os = "tvos")))]
mod tests {
    use std::sync::atomic::Ordering;

    use super::battery::{
        is_battery_monitoring_enabled, power_info_from_battery, SDL_UIKIT_LAST_POWER_INFO_QUERY,
    };
    use super::{get_power_info_uikit, update_battery_monitoring, PowerState};
    use crate::core::apple::uikit::*;

    /// Upstream's mapping of UIDevice's state and level.
    #[test]
    fn battery_states_and_levels() {
        let info = |state, level| {
            let i = power_info_from_battery(state, level);
            (i.state, i.time_left, i.percent)
        };
        use PowerState::*;
        assert_eq!(
            info(UI_DEVICE_BATTERY_STATE_UNKNOWN, -1.0),
            (Unknown, None, None)
        );
        assert_eq!(
            info(UI_DEVICE_BATTERY_STATE_UNPLUGGED, 0.5),
            (OnBattery, None, Some(50))
        );
        assert_eq!(
            info(UI_DEVICE_BATTERY_STATE_CHARGING, 0.004),
            (Charging, None, Some(0))
        );
        assert_eq!(
            info(UI_DEVICE_BATTERY_STATE_CHARGING, 0.005),
            (Charging, None, Some(1))
        );
        assert_eq!(
            info(UI_DEVICE_BATTERY_STATE_FULL, 1.0),
            (Charged, None, Some(100))
        );
        // (the level, rounded to the nearest percent, even with an unknown state)
        assert_eq!(info(7, 0.333), (Unknown, None, Some(33)));
        assert_eq!(info(-1, 0.996), (Unknown, None, Some(100)));
    }

    /// A query turns UIDevice's battery monitoring on; it's turned off
    /// once the last query is older than the timeout. The simulator has no
    /// battery: its state is unknown, as upstream's C reports it there
    /// (`state=0 secs=-1 pct=-1`), and its UIDevice keeps the monitoring
    /// off (on the main thread or another, in an app's bundle or not).
    #[test]
    fn battery_monitoring() {
        let _l = crate::test_support::test_lock();
        let uidev = current_device();
        let info = get_power_info_uikit().unwrap();
        let enabled = is_battery_monitoring_enabled(uidev);
        let last = SDL_UIKIT_LAST_POWER_INFO_QUERY.load(Ordering::Relaxed);
        assert_ne!(last, 0);
        #[cfg(target_abi = "sim")]
        {
            assert_eq!(
                (info.state, info.time_left, info.percent),
                (PowerState::Unknown, None, None)
            );
            assert!(!enabled);
        }
        #[cfg(not(target_abi = "sim"))]
        {
            assert_eq!(info.time_left, None);
            assert!(enabled);
        }

        // Asked again: the dispatcher gives the same answer, and the
        // monitoring stays as it is until the timeout.
        assert_eq!(super::super::power_info(), info);
        update_battery_monitoring();
        assert_eq!(is_battery_monitoring_enabled(uidev), enabled);
        assert_ne!(SDL_UIKIT_LAST_POWER_INFO_QUERY.load(Ordering::Relaxed), 0);

        if !enabled {
            // (past the timeout the update would fail its assertion: see
            // update_battery_monitoring())
            SDL_UIKIT_LAST_POWER_INFO_QUERY.store(0, Ordering::Relaxed);
            return;
        }

        // The last query long ago: turned off (once SDL's ticks, which
        // start at their first use, are past the timeout).
        while crate::timer::ticks_ms() <= 3000 {
            crate::timer::delay(std::time::Duration::from_millis(10));
        }
        SDL_UIKIT_LAST_POWER_INFO_QUERY.store(1, Ordering::Relaxed);
        update_battery_monitoring();
        assert!(!is_battery_monitoring_enabled(uidev));
        assert_eq!(SDL_UIKIT_LAST_POWER_INFO_QUERY.load(Ordering::Relaxed), 0);
    }
}
