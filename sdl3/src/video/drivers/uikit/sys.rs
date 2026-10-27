// The UIKit and QuartzCore declarations the UIKit video driver uses, from
// the iOS SDK headers.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! What `<UIKit/UIKit.h>` and `<QuartzCore/QuartzCore.h>` give upstream's
//! UIKit driver and its main callbacks (`src/main/ios/`): the constants
//! (from the SDK headers) and the C functions and global objects (declared
//! by hand, the frameworks linked as upstream links them). The
//! Objective-C classes are looked up at run time ([`class!`]). What the
//! backends besides the video driver share is in
//! [`crate::core::apple::uikit`].
//!
//! [`class!`]: crate::core::apple::objc::class

#![allow(non_camel_case_types, non_snake_case, non_upper_case_globals)]
#![allow(dead_code)] // (a header: not every declaration is used)

use std::ffi::{c_char, c_int};

use crate::core::apple::foundation::{Boolean, CFStringRef, CGFloat, CGSize};
use crate::core::apple::objc::{Id, NSInteger, NSUInteger};

// ---------------------------------------------------------------------------
// UIApplication

#[link(name = "UIKit", kind = "framework")]
unsafe extern "C" {
    /// `UIApplicationMain()`: creates the application and its delegate
    /// (the classes named), and runs the main run loop; it never returns.
    pub(crate) fn UIApplicationMain(
        argc: c_int,
        argv: *mut *mut c_char,
        principalClassName: Id,
        delegateClassName: Id,
    ) -> c_int;

    /// `CGSizeFromString()`: a `{w, h}` string's size (zero if it isn't one).
    pub(crate) fn CGSizeFromString(string: Id) -> CGSize;

    pub(crate) static UIApplicationDidBecomeActiveNotification: Id;
    pub(crate) static UIApplicationWillResignActiveNotification: Id;
    pub(crate) static UIApplicationDidEnterBackgroundNotification: Id;
    pub(crate) static UIApplicationWillEnterForegroundNotification: Id;
    pub(crate) static UIApplicationWillTerminateNotification: Id;
    pub(crate) static UIApplicationDidReceiveMemoryWarningNotification: Id;

    /// `UITrackingRunLoopMode`: the run loop's mode while a control tracks
    /// a touch (a `UIScrollView` scrolling).
    pub(crate) static UITrackingRunLoopMode: Id;

    /// `UIWindowLevelNormal`.
    pub(crate) static UIWindowLevelNormal: CGFloat;
}

#[cfg(not(any(target_os = "tvos", target_os = "visionos")))]
#[link(name = "UIKit", kind = "framework")]
unsafe extern "C" {
    // (deprecated in iOS 13: the window scene's interface orientation)
    pub(crate) static UIApplicationDidChangeStatusBarOrientationNotification: Id;
}

#[cfg(not(target_os = "visionos"))]
#[link(name = "UIKit", kind = "framework")]
unsafe extern "C" {
    pub(crate) static UIScreenDidConnectNotification: Id;
    pub(crate) static UIScreenDidDisconnectNotification: Id;
}

/// `UIInterfaceOrientation`.
pub(crate) type UIInterfaceOrientation = NSInteger;
pub(crate) const UIInterfaceOrientationUnknown: UIInterfaceOrientation = 0;
pub(crate) const UIInterfaceOrientationPortrait: UIInterfaceOrientation = 1;
pub(crate) const UIInterfaceOrientationPortraitUpsideDown: UIInterfaceOrientation = 2;
pub(crate) const UIInterfaceOrientationLandscapeLeft: UIInterfaceOrientation = 4;
pub(crate) const UIInterfaceOrientationLandscapeRight: UIInterfaceOrientation = 3;

/// `UIInterfaceOrientationIsLandscape()`.
pub(crate) fn UIInterfaceOrientationIsLandscape(orientation: UIInterfaceOrientation) -> bool {
    orientation == UIInterfaceOrientationLandscapeLeft
        || orientation == UIInterfaceOrientationLandscapeRight
}

/// `UIInterfaceOrientationMask`.
pub(crate) type UIInterfaceOrientationMask = NSUInteger;
pub(crate) const UIInterfaceOrientationMaskPortrait: UIInterfaceOrientationMask =
    1 << UIInterfaceOrientationPortrait;
pub(crate) const UIInterfaceOrientationMaskLandscapeLeft: UIInterfaceOrientationMask =
    1 << UIInterfaceOrientationLandscapeLeft;
pub(crate) const UIInterfaceOrientationMaskLandscapeRight: UIInterfaceOrientationMask =
    1 << UIInterfaceOrientationLandscapeRight;
pub(crate) const UIInterfaceOrientationMaskPortraitUpsideDown: UIInterfaceOrientationMask =
    1 << UIInterfaceOrientationPortraitUpsideDown;
pub(crate) const UIInterfaceOrientationMaskLandscape: UIInterfaceOrientationMask =
    UIInterfaceOrientationMaskLandscapeLeft | UIInterfaceOrientationMaskLandscapeRight;
pub(crate) const UIInterfaceOrientationMaskAll: UIInterfaceOrientationMask =
    UIInterfaceOrientationMaskPortrait
        | UIInterfaceOrientationMaskLandscapeLeft
        | UIInterfaceOrientationMaskLandscapeRight
        | UIInterfaceOrientationMaskPortraitUpsideDown;

/// `UIUserInterfaceStyle`.
pub(crate) type UIUserInterfaceStyle = NSInteger;
pub(crate) const UIUserInterfaceStyleUnspecified: UIUserInterfaceStyle = 0;
pub(crate) const UIUserInterfaceStyleLight: UIUserInterfaceStyle = 1;
pub(crate) const UIUserInterfaceStyleDark: UIUserInterfaceStyle = 2;

/// `UIStatusBarStyle`.
pub(crate) type UIStatusBarStyle = NSInteger;
pub(crate) const UIStatusBarStyleDefault: UIStatusBarStyle = 0;
pub(crate) const UIStatusBarStyleLightContent: UIStatusBarStyle = 1;
pub(crate) const UIStatusBarStyleDarkContent: UIStatusBarStyle = 3;

/// `UIViewAutoresizing`.
pub(crate) type UIViewAutoresizing = NSUInteger;
pub(crate) const UIViewAutoresizingFlexibleWidth: UIViewAutoresizing = 1 << 1;
pub(crate) const UIViewAutoresizingFlexibleHeight: UIViewAutoresizing = 1 << 4;

/// `UIRectEdge`.
pub(crate) type UIRectEdge = NSUInteger;
pub(crate) const UIRectEdgeNone: UIRectEdge = 0;
pub(crate) const UIRectEdgeTop: UIRectEdge = 1 << 0;
pub(crate) const UIRectEdgeLeft: UIRectEdge = 1 << 1;
pub(crate) const UIRectEdgeBottom: UIRectEdge = 1 << 2;
pub(crate) const UIRectEdgeRight: UIRectEdge = 1 << 3;
pub(crate) const UIRectEdgeAll: UIRectEdge =
    UIRectEdgeTop | UIRectEdgeLeft | UIRectEdgeBottom | UIRectEdgeRight;

/// `UIEdgeInsets`.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub(crate) struct UIEdgeInsets {
    pub(crate) top: CGFloat,
    pub(crate) left: CGFloat,
    pub(crate) bottom: CGFloat,
    pub(crate) right: CGFloat,
}

/// `UIImageOrientation`.
pub(crate) type UIImageOrientation = NSInteger;
pub(crate) const UIImageOrientationUp: UIImageOrientation = 0;
pub(crate) const UIImageOrientationLeft: UIImageOrientation = 2;
pub(crate) const UIImageOrientationRight: UIImageOrientation = 3;

/// `UIUserInterfaceIdiom`'s values are in [`crate::core::apple::uikit`].
pub(crate) use crate::core::apple::uikit::{
    UI_USER_INTERFACE_IDIOM_PAD as UIUserInterfaceIdiomPad,
    UI_USER_INTERFACE_IDIOM_PHONE as UIUserInterfaceIdiomPhone,
};

/// `UISceneActivationState`.
pub(crate) type UISceneActivationState = NSInteger;
pub(crate) const UISceneActivationStateUnattached: UISceneActivationState = -1;
pub(crate) const UISceneActivationStateForegroundActive: UISceneActivationState = 0;
pub(crate) const UISceneActivationStateForegroundInactive: UISceneActivationState = 1;
pub(crate) const UISceneActivationStateBackground: UISceneActivationState = 2;

// ---------------------------------------------------------------------------
// Touches, gestures and the pointer

/// `UITouchType` (iOS 9).
pub(crate) type UITouchType = NSInteger;
/// A touch of a finger on the screen.
pub(crate) const UITouchTypeDirect: UITouchType = 0;
/// A touch not on the screen (the Siri Remote's touch surface).
pub(crate) const UITouchTypeIndirect: UITouchType = 1;
/// The Apple Pencil (`UITouchTypeStylus`).
pub(crate) const UITouchTypePencil: UITouchType = 2;
/// The pointer of a trackpad or a mouse (iOS 13.4).
pub(crate) const UITouchTypeIndirectPointer: UITouchType = 3;

/// `UIGestureRecognizerState`.
pub(crate) type UIGestureRecognizerState = NSInteger;
pub(crate) const UIGestureRecognizerStatePossible: UIGestureRecognizerState = 0;
pub(crate) const UIGestureRecognizerStateBegan: UIGestureRecognizerState = 1;
pub(crate) const UIGestureRecognizerStateChanged: UIGestureRecognizerState = 2;
pub(crate) const UIGestureRecognizerStateEnded: UIGestureRecognizerState = 3;
pub(crate) const UIGestureRecognizerStateCancelled: UIGestureRecognizerState = 4;
pub(crate) const UIGestureRecognizerStateFailed: UIGestureRecognizerState = 5;

/// `UIEventButtonMask` (iOS 13.4): the buttons of the pointer pressed
/// during an event (`UIEventButtonMaskPrimary` is bit 0, and so on).
pub(crate) type UIEventButtonMask = NSInteger;

// ---------------------------------------------------------------------------
// CoreFoundation's preferences

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    /// `kCFPreferencesAnyApplication`.
    pub(crate) static kCFPreferencesAnyApplication: CFStringRef;

    /// `CFPreferencesGetAppBooleanValue()`: a preference's boolean value
    /// (false when it isn't set or isn't a boolean, which
    /// `keyExistsAndHasValidFormat` tells apart).
    pub(crate) fn CFPreferencesGetAppBooleanValue(
        key: CFStringRef,
        applicationID: CFStringRef,
        keyExistsAndHasValidFormat: *mut Boolean,
    ) -> Boolean;
}

// ---------------------------------------------------------------------------
// GameController (loaded at run time)

/// `GCUIEventTypes` (iOS 18, visionOS 2).
pub(crate) type GCUIEventTypes = NSUInteger;
pub(crate) const GCUIEventTypeGamepad: GCUIEventTypes = 1 << 0;

/// The GameController framework (`dlopen()`, kept loaded for the life of
/// the process, as a weakly linked framework is), `None` without it.
fn game_controller_framework() -> Option<usize> {
    static HANDLE: std::sync::OnceLock<Option<usize>> = std::sync::OnceLock::new();
    *HANDLE.get_or_init(|| {
        // SAFETY: a NUL-terminated path; the handle is never closed.
        let handle = unsafe {
            libc::dlopen(
                c"/System/Library/Frameworks/GameController.framework/GameController".as_ptr(),
                libc::RTLD_LAZY | libc::RTLD_GLOBAL,
            )
        };
        (!handle.is_null()).then_some(handle as usize)
    })
}

/// An `NSString *const` GameController exports (nil without the framework
/// or the symbol).
fn game_controller_string(name: &std::ffi::CStr) -> Id {
    let Some(handle) = game_controller_framework() else {
        return std::ptr::null_mut();
    };
    // SAFETY: the symbol, when there, is an exported `NSString *const`.
    unsafe {
        let p = libc::dlsym(handle as *mut std::ffi::c_void, name.as_ptr()) as *const Id;
        if p.is_null() {
            std::ptr::null_mut()
        } else {
            *p
        }
    }
}

/// `GCMouseDidConnectNotification` (iOS 14), nil without GameController.
pub(crate) fn GCMouseDidConnectNotification() -> Id {
    game_controller_string(c"GCMouseDidConnectNotification")
}

/// `GCMouseDidDisconnectNotification` (iOS 14), nil without GameController.
pub(crate) fn GCMouseDidDisconnectNotification() -> Id {
    game_controller_string(c"GCMouseDidDisconnectNotification")
}

// ---------------------------------------------------------------------------
// Foundation

#[link(name = "Foundation", kind = "framework")]
unsafe extern "C" {
    /// `NSDefaultRunLoopMode`.
    pub(crate) static NSDefaultRunLoopMode: Id;
}

// ---------------------------------------------------------------------------
// QuartzCore

#[link(name = "QuartzCore", kind = "framework")]
unsafe extern "C" {}

/// `CAFrameRateRange` (iOS 15): a display link's preferred frame rates.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) struct CAFrameRateRange {
    pub(crate) minimum: f32,
    pub(crate) maximum: f32,
    pub(crate) preferred: f32,
}

/// `CAFrameRateRangeMake()`.
pub(crate) fn CAFrameRateRangeMake(minimum: f32, maximum: f32, preferred: f32) -> CAFrameRateRange {
    CAFrameRateRange {
        minimum,
        maximum,
        preferred,
    }
}
