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

use crate::core::apple::foundation::{Boolean, CFStringRef, CGFloat, CGPoint, CGSize};
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

/// `UIContainerBackgroundStyle` (visionOS).
#[cfg(target_os = "visionos")]
pub(crate) type UIContainerBackgroundStyle = NSInteger;
#[cfg(target_os = "visionos")]
pub(crate) const UIContainerBackgroundStyleAutomatic: UIContainerBackgroundStyle = 0;
#[cfg(target_os = "visionos")]
pub(crate) const UIContainerBackgroundStyleHidden: UIContainerBackgroundStyle = 2;

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
#[cfg(not(any(target_os = "tvos", target_os = "visionos")))]
pub(crate) use crate::core::apple::uikit::UI_USER_INTERFACE_IDIOM_PAD as UIUserInterfaceIdiomPad;
#[cfg(not(target_os = "tvos"))]
pub(crate) use crate::core::apple::uikit::UI_USER_INTERFACE_IDIOM_PHONE as UIUserInterfaceIdiomPhone;

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

/// `UISwipeGestureRecognizerDirection` (tvOS's swipes on the Siri Remote).
#[cfg(target_os = "tvos")]
pub(crate) type UISwipeGestureRecognizerDirection = crate::core::apple::objc::NSUInteger;
#[cfg(target_os = "tvos")]
pub(crate) const UISwipeGestureRecognizerDirectionRight: UISwipeGestureRecognizerDirection = 1 << 0;
#[cfg(target_os = "tvos")]
pub(crate) const UISwipeGestureRecognizerDirectionLeft: UISwipeGestureRecognizerDirection = 1 << 1;
#[cfg(target_os = "tvos")]
pub(crate) const UISwipeGestureRecognizerDirectionUp: UISwipeGestureRecognizerDirection = 1 << 2;
#[cfg(target_os = "tvos")]
pub(crate) const UISwipeGestureRecognizerDirectionDown: UISwipeGestureRecognizerDirection = 1 << 3;

/// `UIEventButtonMask` (iOS 13.4): the buttons of the pointer pressed
/// during an event (`UIEventButtonMaskPrimary` is bit 0, and so on).
pub(crate) type UIEventButtonMask = NSInteger;

// ---------------------------------------------------------------------------
// The keyboard and text input

#[link(name = "UIKit", kind = "framework")]
unsafe extern "C" {
    pub(crate) static UITextFieldTextDidChangeNotification: Id;

    /// The `UIKeyCommand` inputs of the arrow keys and Escape.
    pub(crate) static UIKeyInputUpArrow: Id;
    pub(crate) static UIKeyInputDownArrow: Id;
    pub(crate) static UIKeyInputLeftArrow: Id;
    pub(crate) static UIKeyInputRightArrow: Id;
    pub(crate) static UIKeyInputEscape: Id;

    /// The `UITextContentType`s (iOS 10; the one-time code's is
    /// [`UITextContentTypeOneTimeCode`]).
    pub(crate) static UITextContentTypeName: Id;
    pub(crate) static UITextContentTypeEmailAddress: Id;
    pub(crate) static UITextContentTypeUsername: Id;
    pub(crate) static UITextContentTypePassword: Id;
}

#[cfg(not(target_os = "tvos"))]
#[link(name = "UIKit", kind = "framework")]
unsafe extern "C" {
    pub(crate) static UIKeyboardWillShowNotification: Id;
    pub(crate) static UIKeyboardDidShowNotification: Id;
    pub(crate) static UIKeyboardWillHideNotification: Id;
    pub(crate) static UIKeyboardDidHideNotification: Id;
    /// The `userInfo` key of the keyboard's frame (an `NSValue` of a
    /// `CGRect`, in the screen's coordinates) once it's shown or hidden.
    pub(crate) static UIKeyboardFrameEndUserInfoKey: Id;
}

/// `UITextContentTypeOneTimeCode` (iOS 12, tvOS 12; nil before): looked
/// up when it's asked for, so that a program still starts on an older
/// system, whose UIKit doesn't have it (upstream links it weakly).
pub(crate) fn UITextContentTypeOneTimeCode() -> Id {
    // SAFETY: the symbol, when there, is UIKit's `NSString *const`.
    unsafe {
        let p =
            libc::dlsym(libc::RTLD_DEFAULT, c"UITextContentTypeOneTimeCode".as_ptr()) as *const Id;
        if p.is_null() {
            std::ptr::null_mut()
        } else {
            *p
        }
    }
}

/// `UIKeyboardType`.
pub(crate) type UIKeyboardType = NSInteger;
pub(crate) const UIKeyboardTypeDefault: UIKeyboardType = 0;
pub(crate) const UIKeyboardTypeNumberPad: UIKeyboardType = 4;
pub(crate) const UIKeyboardTypeEmailAddress: UIKeyboardType = 7;
pub(crate) const UIKeyboardTypeDecimalPad: UIKeyboardType = 8;

/// `UITextAutocapitalizationType`.
pub(crate) type UITextAutocapitalizationType = NSInteger;
pub(crate) const UITextAutocapitalizationTypeNone: UITextAutocapitalizationType = 0;
pub(crate) const UITextAutocapitalizationTypeWords: UITextAutocapitalizationType = 1;
pub(crate) const UITextAutocapitalizationTypeSentences: UITextAutocapitalizationType = 2;
pub(crate) const UITextAutocapitalizationTypeAllCharacters: UITextAutocapitalizationType = 3;

/// `UITextAutocorrectionType`.
pub(crate) type UITextAutocorrectionType = NSInteger;
pub(crate) const UITextAutocorrectionTypeNo: UITextAutocorrectionType = 1;
pub(crate) const UITextAutocorrectionTypeYes: UITextAutocorrectionType = 2;

/// `UITextSpellCheckingType`.
pub(crate) type UITextSpellCheckingType = NSInteger;
pub(crate) const UITextSpellCheckingTypeNo: UITextSpellCheckingType = 1;
pub(crate) const UITextSpellCheckingTypeYes: UITextSpellCheckingType = 2;

/// `UIKeyModifierFlags` (`kNilOptions`: none).
pub(crate) type UIKeyModifierFlags = NSInteger;

/// `UIPressType`: what a press without a key (`UIPress.key`) is, the
/// Apple TV Remote's and the game controllers' buttons.
pub(crate) type UIPressType = NSInteger;
pub(crate) const UIPressTypeUpArrow: UIPressType = 0;
pub(crate) const UIPressTypeDownArrow: UIPressType = 1;
pub(crate) const UIPressTypeLeftArrow: UIPressType = 2;
pub(crate) const UIPressTypeRightArrow: UIPressType = 3;
pub(crate) const UIPressTypeSelect: UIPressType = 4;
pub(crate) const UIPressTypeMenu: UIPressType = 5;
pub(crate) const UIPressTypePlayPause: UIPressType = 6;

/// `UIKeyboardHIDUsage` (iOS 13.4): a key's USB HID usage (page 7), which
/// SDL's scancodes are.
pub(crate) type UIKeyboardHIDUsage = NSInteger;

/// `CGAffineTransform`.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub(crate) struct CGAffineTransform {
    pub(crate) a: CGFloat,
    pub(crate) b: CGFloat,
    pub(crate) c: CGFloat,
    pub(crate) d: CGFloat,
    pub(crate) tx: CGFloat,
    pub(crate) ty: CGFloat,
}

/// `CGPointApplyAffineTransform()` (an inline function of the SDK's).
pub(crate) fn CGPointApplyAffineTransform(point: CGPoint, t: CGAffineTransform) -> CGPoint {
    CGPoint::new(
        t.a * point.x + t.c * point.y + t.tx,
        t.b * point.x + t.d * point.y + t.ty,
    )
}

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

/// Load GameController (once); whether it's there.
#[cfg(target_os = "tvos")]
pub(crate) fn load_game_controller() -> bool {
    game_controller_framework().is_some()
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

/// `GCKeyboardDidConnectNotification` (iOS 14), nil without GameController.
pub(crate) fn GCKeyboardDidConnectNotification() -> Id {
    game_controller_string(c"GCKeyboardDidConnectNotification")
}

/// `GCKeyboardDidDisconnectNotification` (iOS 14), nil without
/// GameController.
pub(crate) fn GCKeyboardDidDisconnectNotification() -> Id {
    game_controller_string(c"GCKeyboardDidDisconnectNotification")
}

/// `GCKeyCode` (iOS 14): a key's USB HID usage (page 7).
pub(crate) type GCKeyCode = NSInteger;

// ---------------------------------------------------------------------------
// The pasteboard and alerts

// (tvOS has no pasteboard)
#[cfg(not(target_os = "tvos"))]
#[link(name = "UIKit", kind = "framework")]
unsafe extern "C" {
    /// `UIPasteboardChangedNotification`: a pasteboard's contents changed
    /// (posted by the pasteboard, in the process that changed them).
    pub(crate) static UIPasteboardChangedNotification: Id;
}

#[link(name = "UIKit", kind = "framework")]
unsafe extern "C" {
    /// `UIWindowLevelAlert`.
    pub(crate) static UIWindowLevelAlert: CGFloat;
}

/// `UIAlertControllerStyle`.
pub(crate) type UIAlertControllerStyle = NSInteger;
pub(crate) const UIAlertControllerStyleActionSheet: UIAlertControllerStyle = 0;
pub(crate) const UIAlertControllerStyleAlert: UIAlertControllerStyle = 1;

/// `UIAlertActionStyle`.
pub(crate) type UIAlertActionStyle = NSInteger;
pub(crate) const UIAlertActionStyleDefault: UIAlertActionStyle = 0;
pub(crate) const UIAlertActionStyleCancel: UIAlertActionStyle = 1;
pub(crate) const UIAlertActionStyleDestructive: UIAlertActionStyle = 2;

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

// ---------------------------------------------------------------------------
// OpenGL ES (<OpenGLES/EAGL.h>, <OpenGLES/EAGLDrawable.h>, <OpenGLES/ES3/gl.h>
// and <OpenGLES/ES2/glext.h>; iOS and tvOS, not visionOS or Mac Catalyst)

#[cfg(all(any(target_os = "ios", target_os = "tvos"), not(target_abi = "macabi")))]
pub(crate) use gles::*;

#[cfg(all(any(target_os = "ios", target_os = "tvos"), not(target_abi = "macabi")))]
mod gles {
    use crate::core::apple::objc::{Id, NSUInteger};

    /// `EAGLRenderingAPI`: the values are the major versions of OpenGL ES.
    pub(crate) type EAGLRenderingAPI = NSUInteger;
    pub(crate) const kEAGLRenderingAPIOpenGLES1: EAGLRenderingAPI = 1;
    pub(crate) const kEAGLRenderingAPIOpenGLES2: EAGLRenderingAPI = 2;
    pub(crate) const kEAGLRenderingAPIOpenGLES3: EAGLRenderingAPI = 3;

    #[link(name = "OpenGLES", kind = "framework")]
    unsafe extern "C" {
        /// `kEAGLDrawablePropertyRetainedBacking`: an NSNumber (a BOOL),
        /// whether the drawable's contents are kept once they're shown.
        pub(crate) static kEAGLDrawablePropertyRetainedBacking: Id;
        /// `kEAGLDrawablePropertyColorFormat`: one of the color formats.
        pub(crate) static kEAGLDrawablePropertyColorFormat: Id;
        /// `kEAGLColorFormatRGBA8`.
        pub(crate) static kEAGLColorFormatRGBA8: Id;
        /// `kEAGLColorFormatRGB565`.
        pub(crate) static kEAGLColorFormatRGB565: Id;
        /// `kEAGLColorFormatSRGBA8`.
        pub(crate) static kEAGLColorFormatSRGBA8: Id;
    }

    pub(crate) type GLenum = u32;
    pub(crate) type GLuint = u32;
    pub(crate) type GLint = i32;
    pub(crate) type GLsizei = i32;
    pub(crate) type GLbitfield = u32;
    pub(crate) type GLchar = std::ffi::c_char;

    pub(crate) const GL_NEAREST: GLenum = 0x2600;
    pub(crate) const GL_COLOR_BUFFER_BIT: GLbitfield = 0x0000_4000;
    pub(crate) const GL_RGBA8: GLenum = 0x8058;
    pub(crate) const GL_DEPTH_COMPONENT24_OES: GLenum = 0x81A6;
    pub(crate) const GL_DEPTH24_STENCIL8_OES: GLenum = 0x88F0;
    pub(crate) const GL_SRGB8_ALPHA8: GLenum = 0x8C43;
    pub(crate) const GL_RENDERBUFFER_BINDING: GLenum = 0x8CA7;
    pub(crate) const GL_READ_FRAMEBUFFER: GLenum = 0x8CA8;
    pub(crate) const GL_DRAW_FRAMEBUFFER: GLenum = 0x8CA9;
    pub(crate) const GL_FRAMEBUFFER_COMPLETE: GLenum = 0x8CD5;
    pub(crate) const GL_COLOR_ATTACHMENT0: GLenum = 0x8CE0;
    pub(crate) const GL_DEPTH_ATTACHMENT: GLenum = 0x8D00;
    pub(crate) const GL_STENCIL_ATTACHMENT: GLenum = 0x8D20;
    pub(crate) const GL_FRAMEBUFFER: GLenum = 0x8D40;
    pub(crate) const GL_RENDERBUFFER: GLenum = 0x8D41;
    pub(crate) const GL_RENDERBUFFER_WIDTH: GLenum = 0x8D42;
    pub(crate) const GL_RENDERBUFFER_HEIGHT: GLenum = 0x8D43;
    pub(crate) const GL_MAX_SAMPLES: GLenum = 0x8D57;
    pub(crate) const GL_RGB565: GLenum = 0x8D62;

    // (OpenGLES.framework's functions, linked as upstream links them: the
    // view's framebuffer is made with them whatever the context's version)
    #[link(name = "OpenGLES", kind = "framework")]
    unsafe extern "C" {
        pub(crate) fn glGetIntegerv(pname: GLenum, data: *mut GLint);
        pub(crate) fn glGenRenderbuffers(n: GLsizei, renderbuffers: *mut GLuint);
        pub(crate) fn glBindRenderbuffer(target: GLenum, renderbuffer: GLuint);
        pub(crate) fn glDeleteRenderbuffers(n: GLsizei, renderbuffers: *const GLuint);
        pub(crate) fn glGenFramebuffers(n: GLsizei, framebuffers: *mut GLuint);
        pub(crate) fn glBindFramebuffer(target: GLenum, framebuffer: GLuint);
        pub(crate) fn glDeleteFramebuffers(n: GLsizei, framebuffers: *const GLuint);
        pub(crate) fn glFramebufferRenderbuffer(
            target: GLenum,
            attachment: GLenum,
            renderbuffertarget: GLenum,
            renderbuffer: GLuint,
        );
        pub(crate) fn glGetRenderbufferParameteriv(
            target: GLenum,
            pname: GLenum,
            params: *mut GLint,
        );
        pub(crate) fn glCheckFramebufferStatus(target: GLenum) -> GLenum;
        pub(crate) fn glRenderbufferStorage(
            target: GLenum,
            internalformat: GLenum,
            width: GLsizei,
            height: GLsizei,
        );
        pub(crate) fn glRenderbufferStorageMultisample(
            target: GLenum,
            samples: GLsizei,
            internalformat: GLenum,
            width: GLsizei,
            height: GLsizei,
        );
        #[allow(clippy::too_many_arguments)]
        pub(crate) fn glBlitFramebuffer(
            src_x0: GLint,
            src_y0: GLint,
            src_x1: GLint,
            src_y1: GLint,
            dst_x0: GLint,
            dst_y0: GLint,
            dst_x1: GLint,
            dst_y1: GLint,
            mask: GLbitfield,
            filter: GLenum,
        );
        pub(crate) fn glInvalidateFramebuffer(
            target: GLenum,
            num_attachments: GLsizei,
            attachments: *const GLenum,
        );
        /// `GL_APPLE_framebuffer_multisample`'s resolve.
        pub(crate) fn glResolveMultisampleFramebufferAPPLE();
        /// `GL_EXT_discard_framebuffer`'s discard.
        pub(crate) fn glDiscardFramebufferEXT(
            target: GLenum,
            num_attachments: GLsizei,
            attachments: *const GLenum,
        );
        /// `GL_EXT_debug_label`'s label.
        pub(crate) fn glLabelObjectEXT(
            type_: GLenum,
            object: GLuint,
            length: GLsizei,
            label: *const GLchar,
        );
    }
}
