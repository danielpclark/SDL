// UIKit for the Apple mobile backends: UIDevice and UIApplication, with
// SDL_GetUIKitDeviceFormFactor() from src/video/uikit/SDL_uikitvideo.m of
// Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The UIKit pieces that backends other than the video driver share
//! (`SDL_PLATFORM_IOS`: every UIKit platform). Upstream's power and URL
//! code import UIKit themselves, and its `SDL_GetUIKitDeviceFormFactor()`
//! sits in the video driver ("This doesn't really have anything to do
//! with the interfaces of the SDL video subsystem, but we need to stuff
//! this into an Objective-C source code file"); here they are reached
//! through this module.
//!
//! UIKit is linked, as upstream links it on these platforms. Its classes
//! are looked up by name, so nothing references the framework's symbols.

use super::objc::{class, msg_send, Id, NSInteger};
use crate::init::FormFactor;

#[link(name = "UIKit", kind = "framework")]
unsafe extern "C" {}

/// `UIUserInterfaceIdiom`.
pub(crate) type UIUserInterfaceIdiom = NSInteger;
/// `UIUserInterfaceIdiomUnspecified`.
#[cfg(test)]
pub(crate) const UI_USER_INTERFACE_IDIOM_UNSPECIFIED: UIUserInterfaceIdiom = -1;
/// `UIUserInterfaceIdiomPhone`.
pub(crate) const UI_USER_INTERFACE_IDIOM_PHONE: UIUserInterfaceIdiom = 0;
/// `UIUserInterfaceIdiomPad`.
pub(crate) const UI_USER_INTERFACE_IDIOM_PAD: UIUserInterfaceIdiom = 1;
/// `UIUserInterfaceIdiomTV`.
pub(crate) const UI_USER_INTERFACE_IDIOM_TV: UIUserInterfaceIdiom = 2;
/// `UIUserInterfaceIdiomCarPlay`.
pub(crate) const UI_USER_INTERFACE_IDIOM_CAR_PLAY: UIUserInterfaceIdiom = 3;
/// `UIUserInterfaceIdiomMac`.
pub(crate) const UI_USER_INTERFACE_IDIOM_MAC: UIUserInterfaceIdiom = 5;
/// `UIUserInterfaceIdiomVision`.
pub(crate) const UI_USER_INTERFACE_IDIOM_VISION: UIUserInterfaceIdiom = 6;

/// `UIDeviceBatteryState` (not on tvOS).
#[cfg(not(target_os = "tvos"))]
pub(crate) type UIDeviceBatteryState = NSInteger;
/// `UIDeviceBatteryStateUnknown`.
#[cfg(not(target_os = "tvos"))]
pub(crate) const UI_DEVICE_BATTERY_STATE_UNKNOWN: UIDeviceBatteryState = 0;
/// `UIDeviceBatteryStateUnplugged`: on battery, discharging.
#[cfg(not(target_os = "tvos"))]
pub(crate) const UI_DEVICE_BATTERY_STATE_UNPLUGGED: UIDeviceBatteryState = 1;
/// `UIDeviceBatteryStateCharging`: plugged in, less than 100%.
#[cfg(not(target_os = "tvos"))]
pub(crate) const UI_DEVICE_BATTERY_STATE_CHARGING: UIDeviceBatteryState = 2;
/// `UIDeviceBatteryStateFull`: plugged in, at 100%.
#[cfg(not(target_os = "tvos"))]
pub(crate) const UI_DEVICE_BATTERY_STATE_FULL: UIDeviceBatteryState = 3;

/// `UIDeviceOrientation` (iOS: the CoreMedia camera's rotation). The
/// others are `UIDeviceOrientationUnknown` (0), `FaceUp` (5) and
/// `FaceDown` (6).
#[cfg(target_os = "ios")]
pub(crate) type UIDeviceOrientation = NSInteger;
/// `UIDeviceOrientationPortrait`: upright, the home button at the bottom.
#[cfg(target_os = "ios")]
pub(crate) const UI_DEVICE_ORIENTATION_PORTRAIT: UIDeviceOrientation = 1;
/// `UIDeviceOrientationPortraitUpsideDown`.
#[cfg(target_os = "ios")]
pub(crate) const UI_DEVICE_ORIENTATION_PORTRAIT_UPSIDE_DOWN: UIDeviceOrientation = 2;
/// `UIDeviceOrientationLandscapeLeft`: the home button on the right.
#[cfg(target_os = "ios")]
pub(crate) const UI_DEVICE_ORIENTATION_LANDSCAPE_LEFT: UIDeviceOrientation = 3;
/// `UIDeviceOrientationLandscapeRight`: the home button on the left.
#[cfg(target_os = "ios")]
pub(crate) const UI_DEVICE_ORIENTATION_LANDSCAPE_RIGHT: UIDeviceOrientation = 4;

/// `UIDeviceOrientationIsValidInterfaceOrientation()`: one of the four
/// the interface can have (not face up or down, nor unknown).
#[cfg(target_os = "ios")]
pub(crate) fn device_orientation_is_valid_interface_orientation(
    orientation: UIDeviceOrientation,
) -> bool {
    matches!(
        orientation,
        UI_DEVICE_ORIENTATION_PORTRAIT
            | UI_DEVICE_ORIENTATION_PORTRAIT_UPSIDE_DOWN
            | UI_DEVICE_ORIENTATION_LANDSCAPE_LEFT
            | UI_DEVICE_ORIENTATION_LANDSCAPE_RIGHT
    )
}

/// `[UIDevice currentDevice]`.
pub(crate) fn current_device() -> Id {
    // SAFETY: UIDevice's class method, answering the shared instance.
    unsafe { msg_send![class!(UIDevice), currentDevice] }
}

/// `[UIApplication sharedApplication]`: nil in a process that isn't an
/// app (no `UIApplicationMain()`), where messages to it answer zero.
pub(crate) fn shared_application() -> Id {
    // SAFETY: UIApplication's class method, answering the app's instance
    // (or nil).
    unsafe { msg_send![class!(UIApplication), sharedApplication] }
}

/// `[UIDevice currentDevice].userInterfaceIdiom`.
pub(crate) fn user_interface_idiom() -> UIUserInterfaceIdiom {
    // SAFETY: a UIDevice property.
    unsafe { msg_send![current_device(), userInterfaceIdiom] }
}

/// The form factor of a `UIUserInterfaceIdiom`, from
/// `SDL_GetUIKitDeviceFormFactor()`.
pub(crate) fn form_factor_from_idiom(idiom: UIUserInterfaceIdiom) -> FormFactor {
    // TODO: Apple Watch
    match idiom {
        UI_USER_INTERFACE_IDIOM_PHONE => FormFactor::Phone,
        UI_USER_INTERFACE_IDIOM_PAD => FormFactor::Tablet,
        UI_USER_INTERFACE_IDIOM_TV => FormFactor::Tv,
        UI_USER_INTERFACE_IDIOM_CAR_PLAY => FormFactor::Car,
        UI_USER_INTERFACE_IDIOM_MAC => FormFactor::Desktop,
        UI_USER_INTERFACE_IDIOM_VISION => FormFactor::Headset,
        _ => FormFactor::Unknown,
    }
}

/// iOS Tablet, etc, detection. Translation of
/// `SDL_GetUIKitDeviceFormFactor()`.
pub(crate) fn device_form_factor() -> FormFactor {
    form_factor_from_idiom(user_interface_idiom())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Upstream's switch, case by case.
    #[test]
    fn form_factors_of_idioms() {
        let cases = [
            (UI_USER_INTERFACE_IDIOM_UNSPECIFIED, FormFactor::Unknown),
            (UI_USER_INTERFACE_IDIOM_PHONE, FormFactor::Phone),
            (UI_USER_INTERFACE_IDIOM_PAD, FormFactor::Tablet),
            (UI_USER_INTERFACE_IDIOM_TV, FormFactor::Tv),
            (UI_USER_INTERFACE_IDIOM_CAR_PLAY, FormFactor::Car),
            (4, FormFactor::Unknown),
            (UI_USER_INTERFACE_IDIOM_MAC, FormFactor::Desktop),
            (UI_USER_INTERFACE_IDIOM_VISION, FormFactor::Headset),
            (7, FormFactor::Unknown),
        ];
        for (idiom, form_factor) in cases {
            assert_eq!(form_factor_from_idiom(idiom), form_factor, "{idiom}");
        }
    }

    /// The SDK's `UIDeviceOrientationIsValidInterfaceOrientation()`, over
    /// unknown (0), the four, face up (5) and down (6), and beyond.
    #[cfg(target_os = "ios")]
    #[test]
    fn valid_interface_orientations() {
        let valid: Vec<bool> = (-1..=7)
            .map(device_orientation_is_valid_interface_orientation)
            .collect();
        assert_eq!(
            valid,
            [false, false, true, true, true, true, false, false, false]
        );
        assert_eq!(
            [
                UI_DEVICE_ORIENTATION_PORTRAIT,
                UI_DEVICE_ORIENTATION_PORTRAIT_UPSIDE_DOWN,
                UI_DEVICE_ORIENTATION_LANDSCAPE_LEFT,
                UI_DEVICE_ORIENTATION_LANDSCAPE_RIGHT
            ],
            [1, 2, 3, 4]
        );
    }

    /// The simulated device's idiom is its kind's (an iPhone's or an
    /// iPad's, as the simulator names its model), also in a process that
    /// isn't an app.
    #[cfg(target_abi = "sim")]
    #[test]
    fn simulated_device_form_factor() {
        let model = std::env::var("SIMULATOR_MODEL_IDENTIFIER").unwrap_or_default();
        let idiom = user_interface_idiom();
        let expected = if model.starts_with("iPhone") {
            FormFactor::Phone
        } else if model.starts_with("iPad") {
            FormFactor::Tablet
        } else if model.starts_with("AppleTV") {
            FormFactor::Tv
        } else if model.starts_with("RealityDevice") {
            // (the Apple Vision Pro)
            FormFactor::Headset
        } else {
            panic!("unexpected simulator model {model:?} (idiom {idiom})");
        };
        assert_eq!(device_form_factor(), expected, "{model}: idiom {idiom}");
        assert_eq!(crate::init::device_form_factor(), expected);
    }

    /// Outside an app (a test program under `simctl spawn`) there's no
    /// shared application.
    #[cfg(target_abi = "sim")]
    #[test]
    fn no_shared_application_outside_an_app() {
        assert!(shared_application().is_null());
    }
}
