// Rust translation of src/video/uikit/SDL_UIKitBridge.m,
// SDL_UIKitBridge-objc.h and SDL_UIKitBridge-swift.h from Simple
// DirectMedia Layer, with stand-ins for the Swift code
// (SDL_uikitviewcontroller.swift, SDL_CurvedContentHosting.swift and the
// rest).
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! visionOS's bridge between the driver and upstream's Swift code: the
//! curved windows (a RealityKit surface a SwiftUI hosting controller
//! presents, which the Metal renderer draws to in place of the layer's
//! drawable), their pointer mode, and their settings and HDR headroom
//! sliders.
//!
//! Note: the Swift code isn't translated (SwiftUI and RealityKit have no
//! Objective-C interface; curved windows and immersive spaces are out of
//! scope), so a window's curved content hosting object, which only it
//! sets (`SDL_VisionOS_SetWindowRealityKitHosting()`), is never there, and
//! every window is a flat UIKit window: the functions the driver and the
//! Metal renderer call are stand-ins with what upstream's answer without
//! that object (no curved window, no pointer mode, no texture), and
//! [`initialize_visionos_curved_ui`] (the Swift extension of the view
//! controller) does nothing. The bridge's functions only the Swift code
//! calls (`SDL_VisionOS_SendSizeChanged()`, `SDL_VisionOS_GetWindowSettings()`,
//! `SDL_VisionOS_SendWindowSettings()`, `SDL_VisionOS_SendPointerMode()`,
//! `SDL_VisionOS_ShouldShowHeadroomUI()`, `SDL_VisionOS_SendHeadroom()`,
//! `SDL_VisionOS_SendTouch()` and `SDL_VisionOS_SetWindowRealityKitHosting()`)
//! have no caller, so they aren't translated. A window's settings
//! (`SDL_PROP_WINDOW_CREATE_VISIONOS_SETTINGS_STRING`) are kept and given
//! back as `SDL_PROP_WINDOW_VISIONOS_SETTINGS_STRING`, as upstream's, but
//! ask for a curved window in vain.

use crate::core::apple::objc::{Id, NSUInteger, NIL};
use crate::events::WindowID;

/// Whether the curved content's pointer mode is on. Translation of
/// `SDL_VisionOS_PointerModeEnabled()`. Note: a stand-in, always false:
/// only the Swift code turns it on (`SDL_VisionOS_SendPointerMode()`).
pub(crate) fn pointer_mode_enabled() -> bool {
    false
}

/// Whether the focused window is a curved one. Translation of
/// `SDL_UIKit_HasCurvedWindow()`. Note: a stand-in, always false (see the
/// module's description).
pub(crate) fn has_curved_window() -> bool {
    let Some(window) = crate::video::core::toplevel_for_keyboard_focus() else {
        return false;
    };
    is_curved_window(window)
}

/// Whether a window is a curved one (has a curved content hosting
/// object). Translation of `SDL_UIKit_IsCurvedWindow()`. Note: a
/// stand-in, always false (see the module's description).
pub(crate) fn is_curved_window(window: WindowID) -> bool {
    let _ = window;
    false
}

/// Dismiss a window's curved content. Translation of
/// `SDL_UIKit_HideCurvedWindow()`. Note: a stand-in that does nothing, as
/// upstream's does for a window without curved content. FIXME (upstream):
/// upstream's returns `nil` from this `void` function, which clang
/// rejects (`-Wreturn-mismatch`), so its CMake visionOS build doesn't
/// compile as it is.
pub(crate) fn hide_curved_window(window: WindowID) {
    let _ = window;
}

/// The curved content's texture to draw to (nil for a flat window).
/// Translation of `SDL_UIKit_GetCurvedDisplayTexture()`. Note: a stand-in,
/// always nil (see the module's description).
pub(crate) fn get_curved_display_texture(
    window: WindowID,
    command_buffer: Id,
    width: i32,
    height: i32,
    pixel_format: NSUInteger,
) -> Id {
    let _ = (window, command_buffer, width, height, pixel_format);
    NIL
}

/// Present a window's curved content from its view controller (visionOS
/// 26, for a window made with settings). Translation of
/// `-[SDL_uikitviewcontroller initializeVisionOSCurvedUI]`, the Swift
/// code's. Note: a stand-in that does nothing (see the module's
/// description), so the window stays a flat one.
pub(crate) fn initialize_visionos_curved_ui(viewcontroller: Id) {
    let _ = viewcontroller;
}
