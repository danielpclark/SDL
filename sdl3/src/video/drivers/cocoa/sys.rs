// The AppKit, CoreGraphics, Carbon, CoreServices, CoreVideo and IOKit
// declarations the Cocoa video driver uses, from the macOS SDK headers.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! What `<Cocoa/Cocoa.h>`, `<Carbon/Carbon.h>`, `<IOKit/pwr_mgt/IOPMLib.h>`
//! and `<CoreVideo/CVDisplayLink.h>` give upstream's Cocoa driver: the
//! constants (from the SDK headers) and the C functions and global objects
//! (declared by hand, the frameworks linked as upstream links them). The
//! Objective-C classes are looked up at run time ([`class!`]).
//!
//! [`class!`]: crate::core::apple::objc::class

#![allow(non_camel_case_types, non_snake_case, non_upper_case_globals)]
#![allow(dead_code)] // (a header: not every declaration is used)

use std::ffi::{c_char, c_void};

use crate::core::apple::foundation::{
    CFArrayRef, CFDataRef, CFDictionaryKeyCallBacks, CFDictionaryRef, CFStringRef, CFTypeRef,
    CGPoint, CGRect,
};
use crate::core::apple::objc::{Id, NSInteger, NSUInteger};

// ---------------------------------------------------------------------------
// AppKit

/// `NSWindowStyleMask`.
pub(crate) const NSWindowStyleMaskBorderless: NSUInteger = 0;
pub(crate) const NSWindowStyleMaskTitled: NSUInteger = 1 << 0;
pub(crate) const NSWindowStyleMaskClosable: NSUInteger = 1 << 1;
pub(crate) const NSWindowStyleMaskMiniaturizable: NSUInteger = 1 << 2;
pub(crate) const NSWindowStyleMaskResizable: NSUInteger = 1 << 3;

/// `NSBackingStoreBuffered`.
pub(crate) const NSBackingStoreBuffered: NSUInteger = 2;

/// `NSWindowCollectionBehavior`.
pub(crate) const NSWindowCollectionBehaviorManaged: NSUInteger = 1 << 2;
pub(crate) const NSWindowCollectionBehaviorFullScreenPrimary: NSUInteger = 1 << 7;
pub(crate) const NSWindowCollectionBehaviorFullScreenNone: NSUInteger = 1 << 9;

/// `NSWindowTabbingModeDisallowed`.
pub(crate) const NSWindowTabbingModeDisallowed: NSInteger = 2;

/// Window levels (`CGWindowLevel.h`).
pub(crate) const kCGNormalWindowLevel: NSInteger = 0;
pub(crate) const NSFloatingWindowLevel: NSInteger = 3;
pub(crate) const kCGMainMenuWindowLevel: NSInteger = 24;

/// `NSWindowOrderingMode`.
pub(crate) const NSWindowAbove: NSInteger = 1;
pub(crate) const NSWindowBelow: NSInteger = -1;

/// `NSWindowMiniaturizeButton`.
pub(crate) const NSWindowMiniaturizeButton: NSUInteger = 1;

/// `NSWindowOcclusionStateVisible`.
pub(crate) const NSWindowOcclusionStateVisible: NSUInteger = 1 << 1;

/// `NSWindowListOrderedFrontToBack`.
pub(crate) const NSWindowListOrderedFrontToBack: NSInteger = 1 << 0;

/// `NSApplicationActivationPolicyRegular`.
pub(crate) const NSApplicationActivationPolicyRegular: NSInteger = 0;

/// `NSApplicationActivateIgnoringOtherApps`.
pub(crate) const NSApplicationActivateIgnoringOtherApps: NSUInteger = 1 << 1;

/// `NSApplicationPresentationOptions`.
pub(crate) const NSApplicationPresentationHideDock: NSUInteger = 1 << 1;
pub(crate) const NSApplicationPresentationHideMenuBar: NSUInteger = 1 << 3;
pub(crate) const NSApplicationPresentationFullScreen: NSUInteger = 1 << 10;

/// `NSRequestUserAttentionType`.
pub(crate) const NSCriticalRequest: NSUInteger = 0;
pub(crate) const NSInformationalRequest: NSUInteger = 10;

/// `NSEventType`.
pub(crate) const NSEventTypeLeftMouseDown: NSUInteger = 1;
pub(crate) const NSEventTypeLeftMouseUp: NSUInteger = 2;
pub(crate) const NSEventTypeRightMouseDown: NSUInteger = 3;
pub(crate) const NSEventTypeRightMouseUp: NSUInteger = 4;
pub(crate) const NSEventTypeMouseMoved: NSUInteger = 5;
pub(crate) const NSEventTypeLeftMouseDragged: NSUInteger = 6;
pub(crate) const NSEventTypeRightMouseDragged: NSUInteger = 7;
pub(crate) const NSEventTypeMouseEntered: NSUInteger = 8;
pub(crate) const NSEventTypeMouseExited: NSUInteger = 9;
pub(crate) const NSEventTypeKeyDown: NSUInteger = 10;
pub(crate) const NSEventTypeKeyUp: NSUInteger = 11;
pub(crate) const NSEventTypeFlagsChanged: NSUInteger = 12;
pub(crate) const NSEventTypeApplicationDefined: NSUInteger = 15;
pub(crate) const NSEventTypeScrollWheel: NSUInteger = 22;
pub(crate) const NSEventTypeTabletPoint: NSUInteger = 23;
pub(crate) const NSEventTypeTabletProximity: NSUInteger = 24;
pub(crate) const NSEventTypeOtherMouseDown: NSUInteger = 25;
pub(crate) const NSEventTypeOtherMouseUp: NSUInteger = 26;
pub(crate) const NSEventTypeOtherMouseDragged: NSUInteger = 27;

/// `NSEventMaskAny`.
pub(crate) const NSEventMaskAny: u64 = u64::MAX;

/// `NSEventSubtype`.
pub(crate) const NSEventSubtypeMouseEvent: i16 = 0;
pub(crate) const NSEventSubtypeTabletPoint: i16 = 1;
pub(crate) const NSEventSubtypeTabletProximity: i16 = 2;

/// `NSEventModifierFlags`.
pub(crate) const NSEventModifierFlagCapsLock: NSUInteger = 1 << 16;
pub(crate) const NSEventModifierFlagShift: NSUInteger = 1 << 17;
pub(crate) const NSEventModifierFlagControl: NSUInteger = 1 << 18;
pub(crate) const NSEventModifierFlagOption: NSUInteger = 1 << 19;
pub(crate) const NSEventModifierFlagCommand: NSUInteger = 1 << 20;

/// The device-dependent modifier masks of `<IOKit/hidsystem/IOLLEvent.h>`.
pub(crate) const NX_DEVICELCTLKEYMASK: u32 = 0x0000_0001;
pub(crate) const NX_DEVICELSHIFTKEYMASK: u32 = 0x0000_0002;
pub(crate) const NX_DEVICERSHIFTKEYMASK: u32 = 0x0000_0004;
pub(crate) const NX_DEVICELCMDKEYMASK: u32 = 0x0000_0008;
pub(crate) const NX_DEVICERCMDKEYMASK: u32 = 0x0000_0010;
pub(crate) const NX_DEVICELALTKEYMASK: u32 = 0x0000_0020;
pub(crate) const NX_DEVICERALTKEYMASK: u32 = 0x0000_0040;
pub(crate) const NX_DEVICERCTLKEYMASK: u32 = 0x0000_2000;
pub(crate) const NX_SHIFTMASK: u32 = 0x0002_0000;
pub(crate) const NX_CONTROLMASK: u32 = 0x0004_0000;
pub(crate) const NX_ALTERNATEMASK: u32 = 0x0008_0000;
pub(crate) const NX_COMMANDMASK: u32 = 0x0010_0000;

/// `NSEventPhase`.
pub(crate) const NSEventPhaseBegan: NSUInteger = 0x1 << 0;
pub(crate) const NSEventPhaseChanged: NSUInteger = 0x1 << 2;
pub(crate) const NSEventPhaseEnded: NSUInteger = 0x1 << 3;
pub(crate) const NSEventPhaseCancelled: NSUInteger = 0x1 << 4;

/// `NSTouchPhase`.
pub(crate) const NSTouchPhaseBegan: NSUInteger = 1 << 0;
pub(crate) const NSTouchPhaseMoved: NSUInteger = 1 << 1;
pub(crate) const NSTouchPhaseStationary: NSUInteger = 1 << 2;
pub(crate) const NSTouchPhaseEnded: NSUInteger = 1 << 3;
pub(crate) const NSTouchPhaseCancelled: NSUInteger = 1 << 4;
pub(crate) const NSTouchPhaseAny: NSUInteger = NSUInteger::MAX;

/// `NSTouchTypeDirect`.
pub(crate) const NSTouchTypeDirect: NSInteger = 0;

/// `NSPointingDeviceType`.
pub(crate) const NSPointingDeviceTypePen: NSUInteger = 1;
pub(crate) const NSPointingDeviceTypeEraser: NSUInteger = 3;

/// `NSEventButtonMask`.
pub(crate) const NSEventButtonMaskPenTip: NSUInteger = 1;
pub(crate) const NSEventButtonMaskPenLowerSide: NSUInteger = 2;
pub(crate) const NSEventButtonMaskPenUpperSide: NSUInteger = 4;

/// `NSTrackingAreaOptions`.
pub(crate) const NSTrackingMouseEnteredAndExited: NSUInteger = 0x01;
pub(crate) const NSTrackingActiveAlways: NSUInteger = 0x80;

/// `NSDragOperation`.
pub(crate) const NSDragOperationNone: NSUInteger = 0;
pub(crate) const NSDragOperationCopy: NSUInteger = 1;
pub(crate) const NSDragOperationLink: NSUInteger = 2;
pub(crate) const NSDragOperationGeneric: NSUInteger = 4;
pub(crate) const NSDragOperationPrivate: NSUInteger = 8;
pub(crate) const NSDragOperationMove: NSUInteger = 16;
pub(crate) const NSDragOperationDelete: NSUInteger = 32;

/// `NSKeyValueObservingOptions`.
pub(crate) const NSKeyValueObservingOptionNew: NSUInteger = 0x01;
pub(crate) const NSKeyValueObservingOptionInitial: NSUInteger = 0x04;

/// `NSAlertStyle`.
pub(crate) const NSAlertStyleWarning: NSUInteger = 0;
pub(crate) const NSAlertStyleInformational: NSUInteger = 1;
pub(crate) const NSAlertStyleCritical: NSUInteger = 2;

/// `NSAlertFirstButtonReturn`.
pub(crate) const NSAlertFirstButtonReturn: NSInteger = 1000;

/// `NSCompositingOperationCopy`.
pub(crate) const NSCompositingOperationCopy: NSUInteger = 1;

/// `NSURLBookmarkResolutionOptions`.
pub(crate) const NSURLBookmarkResolutionWithoutUI: NSUInteger = 1 << 8;
pub(crate) const NSURLBookmarkResolutionWithoutMounting: NSUInteger = 1 << 9;

/// `NSUTF32StringEncoding`.
pub(crate) const NSUTF32StringEncoding: NSUInteger = 0x8c00_0100;

/// `NSModalSession` (an opaque pointer).
pub(crate) type NSModalSession = *mut c_void;

/// `NSAppKitVersionNumber10_13_2`.
pub(crate) const NSAppKitVersionNumber10_13_2: f64 = 1561.2;
/// `NSAppKitVersionNumber10_14`.
pub(crate) const NSAppKitVersionNumber10_14: f64 = 1671.0;

/// The four-character codes of the Apple Event Manager.
pub(crate) const kInternetEventClass: u32 = u32::from_be_bytes(*b"GURL");
pub(crate) const kAEGetURL: u32 = u32::from_be_bytes(*b"GURL");
pub(crate) const keyDirectObject: u32 = u32::from_be_bytes(*b"----");

#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {
    pub(crate) static NSAppKitVersionNumber: f64;

    pub(crate) static NSWindowDidExposeNotification: Id;
    pub(crate) static NSWindowDidChangeOcclusionStateNotification: Id;
    pub(crate) static NSWindowWillStartLiveResizeNotification: Id;
    pub(crate) static NSWindowDidEndLiveResizeNotification: Id;
    pub(crate) static NSWindowWillMoveNotification: Id;
    pub(crate) static NSWindowDidMoveNotification: Id;
    pub(crate) static NSWindowDidResizeNotification: Id;
    pub(crate) static NSWindowWillMiniaturizeNotification: Id;
    pub(crate) static NSWindowDidMiniaturizeNotification: Id;
    pub(crate) static NSWindowDidDeminiaturizeNotification: Id;
    pub(crate) static NSWindowDidBecomeKeyNotification: Id;
    pub(crate) static NSWindowDidResignKeyNotification: Id;
    pub(crate) static NSWindowDidChangeBackingPropertiesNotification: Id;
    pub(crate) static NSWindowDidChangeScreenProfileNotification: Id;
    pub(crate) static NSWindowDidChangeScreenNotification: Id;
    pub(crate) static NSWindowWillEnterFullScreenNotification: Id;
    pub(crate) static NSWindowDidEnterFullScreenNotification: Id;
    pub(crate) static NSWindowWillExitFullScreenNotification: Id;
    pub(crate) static NSWindowDidExitFullScreenNotification: Id;
    pub(crate) static NSWindowWillCloseNotification: Id;
    pub(crate) static NSApplicationDidBecomeActiveNotification: Id;
    pub(crate) static NSApplicationDidChangeScreenParametersNotification: Id;
    pub(crate) static NSBackingPropertyOldScaleFactorKey: Id;
    pub(crate) static NSPasteboardTypeString: Id;
    pub(crate) static NSFilenamesPboardType: Id;
    pub(crate) static NSDeviceRGBColorSpace: Id;

    pub(crate) fn NSRectFill(rect: CGRect);
}

#[link(name = "Foundation", kind = "framework")]
unsafe extern "C" {
    pub(crate) static NSCurrentLocaleDidChangeNotification: Id;
    pub(crate) static NSURLIsAliasFileKey: Id;
    pub(crate) static NSDefaultRunLoopMode: Id;
    pub(crate) static NSRunLoopCommonModes: Id;
}

/// `NSAppKitVersionNumber`.
pub(crate) fn appkit_version_number() -> f64 {
    // SAFETY: a constant AppKit exports.
    unsafe { NSAppKitVersionNumber }
}

// ---------------------------------------------------------------------------
// CoreGraphics

/// `CGDirectDisplayID`.
pub(crate) type CGDirectDisplayID = u32;
/// `CGDisplayCount`.
pub(crate) type CGDisplayCount = u32;
/// `CGError`.
pub(crate) type CGError = i32;
/// `CGDisplayModeRef`.
pub(crate) type CGDisplayModeRef = *const c_void;
/// `CGDisplayChangeSummaryFlags`.
pub(crate) type CGDisplayChangeSummaryFlags = u32;
/// `CGDisplayFadeReservationToken`.
pub(crate) type CGDisplayFadeReservationToken = u32;
/// `CGDisplayFadeInterval`, `CGDisplayReservationInterval`.
pub(crate) type CGDisplayFadeInterval = f32;
/// `CGDisplayBlendFraction`.
pub(crate) type CGDisplayBlendFraction = f32;
/// `CGColorRef`.
pub(crate) type CGColorRef = *const c_void;
/// `io_service_t`.
pub(crate) type io_service_t = u32;

/// `kCGNullDirectDisplay`.
pub(crate) const kCGNullDirectDisplay: CGDirectDisplayID = 0;

/// `CGError` values.
pub(crate) const kCGErrorSuccess: CGError = 0;
pub(crate) const kCGErrorFailure: CGError = 1000;
pub(crate) const kCGErrorIllegalArgument: CGError = 1001;
pub(crate) const kCGErrorInvalidConnection: CGError = 1002;
pub(crate) const kCGErrorInvalidContext: CGError = 1003;
pub(crate) const kCGErrorCannotComplete: CGError = 1004;
pub(crate) const kCGErrorNotImplemented: CGError = 1006;
pub(crate) const kCGErrorRangeCheck: CGError = 1007;
pub(crate) const kCGErrorTypeCheck: CGError = 1008;
pub(crate) const kCGErrorInvalidOperation: CGError = 1010;
pub(crate) const kCGErrorNoneAvailable: CGError = 1011;

/// `CGDisplayChangeSummaryFlags` values.
pub(crate) const kCGDisplayBeginConfigurationFlag: u32 = 1 << 0;
pub(crate) const kCGDisplayMovedFlag: u32 = 1 << 1;
pub(crate) const kCGDisplaySetMainFlag: u32 = 1 << 2;
pub(crate) const kCGDisplaySetModeFlag: u32 = 1 << 3;
pub(crate) const kCGDisplayAddFlag: u32 = 1 << 4;
pub(crate) const kCGDisplayRemoveFlag: u32 = 1 << 5;
pub(crate) const kCGDisplayEnabledFlag: u32 = 1 << 8;
pub(crate) const kCGDisplayDisabledFlag: u32 = 1 << 9;
pub(crate) const kCGDisplayMirrorFlag: u32 = 1 << 10;
pub(crate) const kCGDisplayUnMirrorFlag: u32 = 1 << 11;
pub(crate) const kCGDisplayDesktopShapeChangedFlag: u32 = 1 << 12;

/// `kCGDisplayFadeReservationInvalidToken`.
pub(crate) const kCGDisplayFadeReservationInvalidToken: CGDisplayFadeReservationToken = 0;
/// `kCGDisplayBlendNormal`.
pub(crate) const kCGDisplayBlendNormal: CGDisplayBlendFraction = 0.0;
/// `kCGDisplayBlendSolidColor`.
pub(crate) const kCGDisplayBlendSolidColor: CGDisplayBlendFraction = 1.0;

/// The display mode IO flags of `<IOKit/graphics/IOGraphicsTypes.h>`.
pub(crate) const kDisplayModeValidFlag: u32 = 0x0000_0001;
pub(crate) const kDisplayModeSafeFlag: u32 = 0x0000_0002;
pub(crate) const kDisplayModeInterlacedFlag: u32 = 0x0000_0040;
pub(crate) const kDisplayModeNeverShowFlag: u32 = 0x0000_0080;
pub(crate) const kDisplayModeNotGraphicsQualityFlag: u32 = 0x0000_1000;

/// `IO32BitDirectPixels`.
pub(crate) const IO32BitDirectPixels: &str = "--------RRRRRRRRGGGGGGGGBBBBBBBB";
/// `IO16BitDirectPixels`.
pub(crate) const IO16BitDirectPixels: &str = "-RRRRRGGGGGBBBBB";
/// `kIO30BitDirectPixels`.
pub(crate) const kIO30BitDirectPixels: &str = "--RRRRRRRRRRGGGGGGGGGGBBBBBBBBBB";

/// `kIODisplayOnlyPreferredName`.
pub(crate) const kIODisplayOnlyPreferredName: u32 = 0x0000_0200;
/// `kDisplayProductName`.
pub(crate) const kDisplayProductName: &str = "DisplayProductName";

/// `CGDisplayReconfigurationCallBack`.
pub(crate) type CGDisplayReconfigurationCallBack = unsafe extern "C" fn(
    display: CGDirectDisplayID,
    flags: CGDisplayChangeSummaryFlags,
    user_info: *mut c_void,
);

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    pub(crate) static kCGColorClear: CFStringRef;
    pub(crate) static kCGColorBlack: CFStringRef;
    pub(crate) static kCGDisplayShowDuplicateLowResolutionModes: CFStringRef;

    pub(crate) fn CGMainDisplayID() -> CGDirectDisplayID;
    pub(crate) fn CGGetOnlineDisplayList(
        max_displays: u32,
        online_displays: *mut CGDirectDisplayID,
        display_count: *mut CGDisplayCount,
    ) -> CGError;
    pub(crate) fn CGDisplayIsMain(display: CGDirectDisplayID) -> u32;
    pub(crate) fn CGDisplayMirrorsDisplay(display: CGDirectDisplayID) -> CGDirectDisplayID;
    pub(crate) fn CGDisplayBounds(display: CGDirectDisplayID) -> CGRect;
    pub(crate) fn CGDisplayPixelsWide(display: CGDirectDisplayID) -> usize;
    pub(crate) fn CGDisplayPixelsHigh(display: CGDirectDisplayID) -> usize;
    pub(crate) fn CGDisplayCopyDisplayMode(display: CGDirectDisplayID) -> CGDisplayModeRef;
    pub(crate) fn CGDisplayCopyAllDisplayModes(
        display: CGDirectDisplayID,
        options: CFDictionaryRef,
    ) -> CFArrayRef;
    pub(crate) fn CGDisplaySetDisplayMode(
        display: CGDirectDisplayID,
        mode: CGDisplayModeRef,
        options: CFDictionaryRef,
    ) -> CGError;
    pub(crate) fn CGDisplayModeRelease(mode: CGDisplayModeRef);
    pub(crate) fn CGDisplayModeGetWidth(mode: CGDisplayModeRef) -> usize;
    pub(crate) fn CGDisplayModeGetHeight(mode: CGDisplayModeRef) -> usize;
    pub(crate) fn CGDisplayModeGetPixelWidth(mode: CGDisplayModeRef) -> usize;
    pub(crate) fn CGDisplayModeGetPixelHeight(mode: CGDisplayModeRef) -> usize;
    pub(crate) fn CGDisplayModeGetRefreshRate(mode: CGDisplayModeRef) -> f64;
    pub(crate) fn CGDisplayModeGetIOFlags(mode: CGDisplayModeRef) -> u32;
    pub(crate) fn CGDisplayModeIsUsableForDesktopGUI(mode: CGDisplayModeRef) -> bool;
    pub(crate) fn CGDisplayModeCopyPixelEncoding(mode: CGDisplayModeRef) -> CFStringRef;
    pub(crate) fn CGDisplayRegisterReconfigurationCallback(
        callback: CGDisplayReconfigurationCallBack,
        user_info: *mut c_void,
    ) -> CGError;
    pub(crate) fn CGDisplayRemoveReconfigurationCallback(
        callback: CGDisplayReconfigurationCallBack,
        user_info: *mut c_void,
    ) -> CGError;
    pub(crate) fn CGAcquireDisplayFadeReservation(
        seconds: CGDisplayFadeInterval,
        token: *mut CGDisplayFadeReservationToken,
    ) -> CGError;
    pub(crate) fn CGDisplayFade(
        token: CGDisplayFadeReservationToken,
        duration: CGDisplayFadeInterval,
        start_blend: CGDisplayBlendFraction,
        end_blend: CGDisplayBlendFraction,
        red: f32,
        green: f32,
        blue: f32,
        synchronous: u32,
    ) -> CGError;
    pub(crate) fn CGReleaseDisplayFadeReservation(token: CGDisplayFadeReservationToken) -> CGError;
    pub(crate) fn CGWarpMouseCursorPosition(new_cursor_position: CGPoint) -> CGError;
    pub(crate) fn CGAssociateMouseAndMouseCursorPosition(connected: u32) -> CGError;
    pub(crate) fn CGDisplayMoveCursorToPoint(display: CGDirectDisplayID, point: CGPoint)
        -> CGError;
    pub(crate) fn CGColorGetConstantColor(color_name: CFStringRef) -> CGColorRef;
}

/// `kCGDirectMainDisplay` (a macro for `CGMainDisplayID()`).
pub(crate) fn kCGDirectMainDisplay() -> CGDirectDisplayID {
    // SAFETY: no arguments.
    unsafe { CGMainDisplayID() }
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    pub(crate) static kCFCopyStringDictionaryKeyCallBacks: CFDictionaryKeyCallBacks;
    pub(crate) fn CFArrayExchangeValuesAtIndices(array: *mut c_void, idx1: isize, idx2: isize);
    pub(crate) fn CFStringCompare(a: CFStringRef, b: CFStringRef, options: usize) -> isize;
}

/// `kCFCompareCaseInsensitive`.
pub(crate) const kCFCompareCaseInsensitive: usize = 1;
/// `kCFCompareEqualTo`.
pub(crate) const kCFCompareEqualTo: isize = 0;

// ---------------------------------------------------------------------------
// CoreVideo

/// `CVDisplayLinkRef`.
pub(crate) type CVDisplayLinkRef = *mut c_void;

/// `CVTime`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct CVTime {
    pub(crate) time_value: i64,
    pub(crate) time_scale: i32,
    pub(crate) flags: i32,
}

/// `kCVTimeIsIndefinite`.
pub(crate) const kCVTimeIsIndefinite: i32 = 1 << 0;

#[link(name = "CoreVideo", kind = "framework")]
unsafe extern "C" {
    pub(crate) fn CVDisplayLinkCreateWithCGDisplay(
        display_id: CGDirectDisplayID,
        display_link_out: *mut CVDisplayLinkRef,
    ) -> i32;
    pub(crate) fn CVDisplayLinkGetNominalOutputVideoRefreshPeriod(link: CVDisplayLinkRef)
        -> CVTime;
    pub(crate) fn CVDisplayLinkRelease(link: CVDisplayLinkRef);
}

// ---------------------------------------------------------------------------
// IOKit power management

/// `IOPMAssertionID`.
pub(crate) type IOPMAssertionID = u32;
/// `kIOPMNullAssertionID`.
pub(crate) const kIOPMNullAssertionID: IOPMAssertionID = 0;
/// `kIOPMAssertPreventUserIdleDisplaySleep` (a `CFSTR()` macro).
pub(crate) const kIOPMAssertPreventUserIdleDisplaySleep: &str = "PreventUserIdleDisplaySleep";

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    pub(crate) fn IOPMAssertionCreateWithDescription(
        assertion_type: CFStringRef,
        name: CFStringRef,
        details: CFStringRef,
        human_readable_reason: CFStringRef,
        localization_bundle_path: CFStringRef,
        timeout: f64,
        timeout_action: CFStringRef,
        assertion_id: *mut IOPMAssertionID,
    ) -> i32;
    pub(crate) fn IOPMAssertionRelease(assertion_id: IOPMAssertionID) -> i32;
    pub(crate) fn IODisplayCreateInfoDictionary(
        framebuffer: io_service_t,
        options: u32,
    ) -> CFDictionaryRef;
}

// ---------------------------------------------------------------------------
// Carbon (HIToolbox) and CoreServices

/// `TISInputSourceRef`.
pub(crate) type TISInputSourceRef = *const c_void;
/// `UCKeyboardLayout` (opaque).
pub(crate) type UCKeyboardLayout = c_void;
/// `UniChar`.
pub(crate) type UniChar = u16;
/// `UniCharCount`.
pub(crate) type UniCharCount = std::ffi::c_ulong;
/// `OSStatus`.
pub(crate) type OSStatus = i32;
/// `PhysicalKeyboardLayoutType`.
pub(crate) type PhysicalKeyboardLayoutType = u32;

/// `noErr`.
pub(crate) const noErr: OSStatus = 0;
/// `kUCKeyActionDown`.
pub(crate) const kUCKeyActionDown: u16 = 0;
/// `kUCKeyTranslateNoDeadKeysMask` (`1 << kUCKeyTranslateNoDeadKeysBit`).
pub(crate) const kUCKeyTranslateNoDeadKeysMask: u32 = 1 << 0;
/// The modifier bits of `<HIToolbox/Events.h>`.
pub(crate) const shiftKey: u32 = 1 << 9;
pub(crate) const alphaLock: u32 = 1 << 10;
pub(crate) const optionKey: u32 = 1 << 11;
/// `kKeyboardISO`.
pub(crate) const kKeyboardISO: PhysicalKeyboardLayoutType = u32::from_be_bytes(*b"ISO ");

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    pub(crate) static kTISPropertyUnicodeKeyLayoutData: CFStringRef;
    pub(crate) fn TISCopyCurrentKeyboardLayoutInputSource() -> TISInputSourceRef;
    pub(crate) fn TISGetInputSourceProperty(
        input_source: TISInputSourceRef,
        property_key: CFStringRef,
    ) -> CFDataRef;
    pub(crate) fn LMGetKbdType() -> u8;
    pub(crate) fn KBGetLayoutType(keyboard_type: i16) -> PhysicalKeyboardLayoutType;
    pub(crate) fn UCKeyTranslate(
        key_layout_ptr: *const UCKeyboardLayout,
        virtual_key_code: u16,
        key_action: u16,
        modifier_key_state: u32,
        keyboard_type: u32,
        key_translate_options: u32,
        dead_key_state: *mut u32,
        max_string_length: UniCharCount,
        actual_string_length: *mut UniCharCount,
        unicode_string: *mut UniChar,
    ) -> OSStatus;
}

#[link(name = "CoreServices", kind = "framework")]
unsafe extern "C" {
    pub(crate) static kUTTypeFileURL: CFStringRef;
    pub(crate) static kUTTypeUTF8PlainText: CFStringRef;
    pub(crate) static kUTTagClassMIMEType: CFStringRef;
    pub(crate) fn UTTypeCreatePreferredIdentifierForTag(
        tag_class: CFStringRef,
        tag: CFStringRef,
        conforming_to_uti: CFStringRef,
    ) -> CFStringRef;
    pub(crate) fn UTTypeCopyPreferredTagWithClass(
        uti: CFStringRef,
        tag_class: CFStringRef,
    ) -> CFStringRef;
}

// ---------------------------------------------------------------------------
// Frameworks upstream links weakly, loaded at run time

/// Load a system framework's binary (`dlopen()`), kept for the process'
/// life as a weakly linked framework is; `None` if it isn't there.
pub(crate) fn load_framework(path: &std::ffi::CStr) -> Option<usize> {
    // SAFETY: a NUL-terminated path; the handle is never closed.
    let handle = unsafe { libc::dlopen(path.as_ptr(), libc::RTLD_LAZY | libc::RTLD_GLOBAL) };
    if handle.is_null() {
        None
    } else {
        Some(handle as usize)
    }
}

/// An exported global object of a framework loaded with
/// [`load_framework`] (`dlsym()`, dereferenced), nil if absent.
///
/// # Safety
///
/// `name` must name an exported `id` variable.
pub(crate) unsafe fn framework_object(handle: usize, name: &std::ffi::CStr) -> Id {
    // SAFETY: the caller's contract; the symbol is the variable's address.
    unsafe {
        let p = libc::dlsym(handle as *mut c_void, name.as_ptr()) as *const Id;
        if p.is_null() {
            std::ptr::null_mut()
        } else {
            *p
        }
    }
}

/// A C string that lives as long as the autorelease pool (`[NSString
/// UTF8String]`) as `&str`, or "".
///
/// # Safety
///
/// `p` must be null or a NUL-terminated string valid for the borrow.
pub(crate) unsafe fn cstr_or_empty<'a>(p: *const c_char) -> &'a str {
    if p.is_null() {
        return "";
    }
    // SAFETY: the caller's contract.
    unsafe { std::ffi::CStr::from_ptr(p) }
        .to_str()
        .unwrap_or("")
}

/// `CFStringRef` constants are toll-free bridged `NSString`s.
pub(crate) fn cf_as_id(s: CFTypeRef) -> Id {
    s as Id
}
