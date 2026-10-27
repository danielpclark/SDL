// Rust translation of the display parts of src/video/uikit/SDL_uikitmodes.m
// and SDL_uikitmodes.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The displays: one per `UIScreen` (the fake 1280x720 one on visionOS),
//! with its desktop mode, added at video init and when a screen connects
//! (`SDL_DisplayWatch`), and the main screen's orientation
//! (`SDL_OnApplicationDidChangeStatusBarOrientation()`).
//!
//! This is the part of upstream's file the video init needs (it fails
//! without a display). The fullscreen modes (`UIKit_GetDisplayModes()`),
//! mode changes (`UIKit_SetDisplayMode()`) and the usable bounds
//! (`UIKit_GetDisplayUsableBounds()`) aren't translated yet: displays have
//! only their desktop mode, and the generic usable bounds (the display's
//! bounds). Upstream keeps each mode's `UIScreenMode` in the mode, for
//! `UIKit_SetDisplayMode()`; it isn't kept here until that is translated.

use crate::core::apple::objc::{autoreleasepool, StrongPtr};
#[cfg(not(target_os = "visionos"))]
use crate::core::apple::objc::{class, msg_send, Id};
use crate::events::DisplayID;
#[cfg(not(target_os = "visionos"))]
use crate::events::EventType;
#[cfg(not(target_os = "visionos"))]
use crate::video::display::del_video_display;
use crate::video::display::{add_video_display, displays};
#[cfg(not(any(target_os = "tvos", target_os = "visionos")))]
use crate::video::display::{primary_display, set_desktop_display_mode};
use crate::video::sysvideo::{DisplayOrientation, VideoDisplay};
use crate::video::{DisplayMode, PixelFormat};

#[cfg(not(target_os = "visionos"))]
use super::sys::*;

/// because visionOS does not have a screen
/// we create a fake display to maintain compatibility.
/// By default, a window measures 1280x720 pt.
/// <https://developer.apple.com/design/human-interface-guidelines/windows#visionOS>
#[cfg(target_os = "visionos")]
pub(crate) const SDL_XR_SCREENWIDTH: i32 = 1280;
#[cfg(target_os = "visionos")]
pub(crate) const SDL_XR_SCREENHEIGHT: i32 = 720;

/// Translation of `SDL_UIKitDisplayData` (the display's `internal`).
pub(crate) struct DisplayData {
    /// `uiscreen` (nil on visionOS).
    #[cfg_attr(target_os = "visionos", allow(dead_code))]
    pub(crate) uiscreen: StrongPtr,
}

/// The `UIScreen` of a display, if it's one of this driver's.
#[cfg(not(target_os = "visionos"))]
pub(crate) fn display_screen(display: DisplayID) -> Option<Id> {
    crate::video::core::with_display(display, |d| {
        d.internal
            .as_ref()
            .and_then(|i| i.downcast_ref::<DisplayData>())
            .map(|data| data.uiscreen.get())
    })
    .flatten()
}

// ---------------------------------------------------------------------------
// SDL_DisplayWatch

#[cfg(not(target_os = "visionos"))]
mod watch {
    use super::{add_display, del_display};
    use crate::core::apple::objc::{
        class, imp, msg_send, sel, Class, ClassBuilder, DeclaredClass, Id, Sel, NIL,
    };
    use crate::video::drivers::uikit::sys::{
        UIScreenDidConnectNotification, UIScreenDidDisconnectNotification,
    };

    static SDL_DISPLAY_WATCH: DeclaredClass = DeclaredClass::new();

    /// The `SDL_DisplayWatch` class (`@interface SDL_DisplayWatch :
    /// NSObject`), whose class methods observe the screens.
    fn display_watch_class() -> Class {
        SDL_DISPLAY_WATCH.get(|| {
            let mut b =
                ClassBuilder::new(c"SDL_DisplayWatch", class!(NSObject)).expect("SDL_DisplayWatch");
            // SAFETY: the implementations have the signatures of the encodings.
            unsafe {
                b.add_class_method(
                    sel!(screenConnected:),
                    imp!(screen_connected as extern "C" fn(Id, Sel, Id)),
                    c"v@:@",
                );
                b.add_class_method(
                    sel!(screenDisconnected:),
                    imp!(screen_disconnected as extern "C" fn(Id, Sel, Id)),
                    c"v@:@",
                );
            }
            b.register()
        })
    }

    /// Translation of `+[SDL_DisplayWatch start]`.
    pub(super) fn start() {
        let this = display_watch_class() as Id;
        // SAFETY: the class's methods taking a notification, for the
        // screens' notifications.
        unsafe {
            let center: Id = msg_send![class!(NSNotificationCenter), defaultCenter];

            let () = msg_send![center, addObserver: this
                                          selector: sel!(screenConnected:)
                                              name: UIScreenDidConnectNotification
                                            object: NIL];
            let () = msg_send![center, addObserver: this
                                          selector: sel!(screenDisconnected:)
                                              name: UIScreenDidDisconnectNotification
                                            object: NIL];
        }
    }

    /// Translation of `+[SDL_DisplayWatch stop]`.
    pub(super) fn stop() {
        let this = display_watch_class() as Id;
        // SAFETY: removing the observations start() added.
        unsafe {
            let center: Id = msg_send![class!(NSNotificationCenter), defaultCenter];

            let () = msg_send![center, removeObserver: this
                                                 name: UIScreenDidConnectNotification
                                               object: NIL];
            let () = msg_send![center, removeObserver: this
                                                 name: UIScreenDidDisconnectNotification
                                               object: NIL];
        }
    }

    /// Translation of `+[SDL_DisplayWatch screenConnected:]`.
    extern "C" fn screen_connected(_this: Id, _sel: Sel, notification: Id) {
        // SAFETY: the notification's object is the screen.
        let uiscreen: Id = unsafe { msg_send![notification, object] };
        add_display(uiscreen, true);
    }

    /// Translation of `+[SDL_DisplayWatch screenDisconnected:]`.
    extern "C" fn screen_disconnected(_this: Id, _sel: Sel, notification: Id) {
        // SAFETY: as above.
        let uiscreen: Id = unsafe { msg_send![notification, object] };
        del_display(uiscreen, true);
    }
}

// ---------------------------------------------------------------------------
// Displays

/// Translation of `UIKit_GetDisplayModeRefreshRate()`.
#[cfg(not(target_os = "visionos"))]
fn get_display_mode_refresh_rate(uiscreen: Id) -> f32 {
    // SAFETY: a UIScreen property.
    let fps: crate::core::apple::objc::NSInteger =
        unsafe { msg_send![uiscreen, maximumFramesPerSecond] };
    fps as f32
}

/// Translation of `GetUIScreenModeSize()`.
#[cfg(not(target_os = "visionos"))]
fn get_uiscreen_mode_size(uiscreen: Id, mode: Id) -> crate::core::apple::foundation::CGSize {
    /* For devices such as iPhone 6/7/8 Plus, the UIScreenMode reported by iOS
     * isn't the physical pixels of the display, but rather the point size times
     * the scale. For example, on iOS 12.2 on iPhone 8 Plus the physical pixel
     * resolution is 1080x1920, the size reported by mode.size is 1242x2208,
     * the size in points is 414x736, the scale property is 3.0, and the
     * nativeScale property is ~2.6087 (ie 1920.0 / 736.0).
     *
     * What we want for the mode size is the point size, and the pixel density
     * is the native scale.
     *
     * Note that the iOS Simulator doesn't have this behavior for those devices.
     * https://github.com/libsdl-org/SDL/issues/3220
     */
    // SAFETY: UIScreenMode's and UIScreen's properties.
    unsafe {
        let mut size: crate::core::apple::foundation::CGSize = msg_send![mode, size];
        let scale: crate::core::apple::foundation::CGFloat = msg_send![uiscreen, scale];

        size.width = crate::stdlib::math::round(size.width / scale);
        size.height = crate::stdlib::math::round(size.height / scale);

        size
    }
}

/// Add the display of a screen. Translation of `UIKit_AddDisplay()`.
#[cfg(not(target_os = "visionos"))]
pub(crate) fn add_display(uiscreen: Id, send_event: bool) -> bool {
    // SAFETY: UIScreen's properties.
    unsafe {
        let uiscreenmode: Id = msg_send![uiscreen, currentMode];
        let mut size = get_uiscreen_mode_size(uiscreen, uiscreenmode);

        // Make sure the width/height are oriented correctly
        if is_display_landscape(uiscreen) != (size.width > size.height) {
            std::mem::swap(&mut size.width, &mut size.height);
        }

        let native_scale: crate::core::apple::foundation::CGFloat =
            msg_send![uiscreen, nativeScale];
        let mode = DisplayMode {
            w: size.width as i32,
            h: size.height as i32,
            pixel_density: native_scale as f32,
            format: PixelFormat::ABGR8888,
            refresh_rate: get_display_mode_refresh_rate(uiscreen),
            ..DisplayMode::default()
        };

        let mut display = VideoDisplay::new();
        #[cfg(not(target_os = "tvos"))]
        let is_main = uiscreen == main_screen();
        #[cfg(target_os = "tvos")]
        let is_main = false;
        if is_main {
            // The natural orientation (used by sensors) is portrait
            display.natural_orientation = DisplayOrientation::Portrait;
        } else if is_display_landscape(uiscreen) {
            display.natural_orientation = DisplayOrientation::Landscape;
        } else {
            display.natural_orientation = DisplayOrientation::Portrait;
        }
        display.desktop_mode = mode;

        display.hdr.sdr_white_level = 1.0;
        display.hdr.hdr_headroom = 1.0;

        #[cfg(not(target_os = "tvos"))]
        if crate::core::apple::foundation::available!(ios(16, 0)) {
            let current: crate::core::apple::foundation::CGFloat =
                msg_send![uiscreen, currentEDRHeadroom];
            if current > 1.0 {
                display.hdr.hdr_headroom = current as f32;
            } else {
                let potential: crate::core::apple::foundation::CGFloat =
                    msg_send![uiscreen, potentialEDRHeadroom];
                display.hdr.hdr_headroom = potential as f32;
            }
        }

        // Allocate the display data
        display.internal = Some(Box::new(DisplayData {
            uiscreen: StrongPtr::retain(uiscreen),
        }));
        add_video_display(display, send_event) != 0
    }
}

/// Add the visionOS display. Translation of `UIKit_AddDisplay()`
/// (visionOS).
#[cfg(target_os = "visionos")]
pub(crate) fn add_display(send_event: bool) -> bool {
    let mode = DisplayMode {
        w: SDL_XR_SCREENWIDTH,
        h: SDL_XR_SCREENHEIGHT,
        pixel_density: 2.0,
        format: PixelFormat::ABGR8888,
        refresh_rate: 90.0,
        ..DisplayMode::default()
    };

    let mut display = VideoDisplay::new();
    display.natural_orientation = DisplayOrientation::Landscape;

    display.desktop_mode = mode;

    // There isn't currently a way to query the EDR headeroom on visionOS.
    // The range is 1 - 16, so we'll assume a current value of 2 for now.
    display.hdr.sdr_white_level = 1.0;
    display.hdr.hdr_headroom = 2.0;

    display.internal = Some(Box::new(DisplayData {
        uiscreen: StrongPtr::nil(),
    }));
    add_video_display(display, send_event) != 0
}

/// Remove the display of a screen. Translation of `UIKit_DelDisplay()`.
#[cfg(not(target_os = "visionos"))]
pub(crate) fn del_display(uiscreen: Id, send_event: bool) {
    for id in displays().unwrap_or_default() {
        if display_screen(id) == Some(uiscreen) {
            let _ = crate::video::core::with_display(id, |d| d.internal = None);
            del_video_display(id, send_event);
            break;
        }
    }
}

/// `[UIScreen mainScreen]`.
#[cfg(not(target_os = "visionos"))]
pub(crate) fn main_screen() -> Id {
    // SAFETY: UIScreen's class method.
    unsafe { msg_send![class!(UIScreen), mainScreen] }
}

/// Translation of `UIKit_IsDisplayLandscape()`.
#[cfg(not(target_os = "visionos"))]
pub(crate) fn is_display_landscape(uiscreen: Id) -> bool {
    #[cfg(not(target_os = "tvos"))]
    if uiscreen == main_screen() {
        return UIInterfaceOrientationIsLandscape(super::get_interface_orientation());
    }
    // SAFETY: a UIScreen property.
    let bounds: crate::core::apple::foundation::CGRect = unsafe { msg_send![uiscreen, bounds] };
    bounds.size.width > bounds.size.height
}

/// Translation of `UIKit_InitModes()`.
pub(crate) fn init_modes() -> bool {
    autoreleasepool(|| {
        #[cfg(target_os = "visionos")]
        add_display(false);
        #[cfg(not(target_os = "visionos"))]
        {
            // SAFETY: UIScreen's class method, an array of screens.
            let screens = unsafe {
                let screens: Id = msg_send![class!(UIScreen), screens];
                crate::core::apple::foundation::ns_array_items(screens)
            };
            for uiscreen in screens {
                if !add_display(uiscreen, false) {
                    return false;
                }
            }
        }

        #[cfg(not(any(target_os = "tvos", target_os = "visionos")))]
        on_application_did_change_status_bar_orientation();

        #[cfg(not(target_os = "visionos"))]
        watch::start();

        true
    })
}

/// Translation of `UIKit_QuitModes()`.
pub(crate) fn quit_modes() {
    #[cfg(not(target_os = "visionos"))]
    watch::stop();

    // Release Objective-C objects, so higher level doesn't free() them.
    autoreleasepool(|| {
        for id in displays().unwrap_or_default() {
            let _ = crate::video::core::with_display(id, |d| d.internal = None);
        }
    });
}

/// Translation of `SDL_OnApplicationDidChangeStatusBarOrientation()`
/// (iOS).
#[cfg(not(any(target_os = "tvos", target_os = "visionos")))]
pub(crate) fn on_application_did_change_status_bar_orientation() {
    let interface_orientation = super::get_interface_orientation();
    let is_landscape = UIInterfaceOrientationIsLandscape(interface_orientation);
    let Ok(display) = primary_display() else {
        return;
    };
    let Some(desktop_mode) = crate::video::core::with_display(display, |d| d.desktop_mode) else {
        return;
    };

    /* The desktop display mode should be kept in sync with the screen
     * orientation so that updating a window's fullscreen state to
     * fullscreen desktop keeps the window dimensions in the
     * correct orientation. */
    if is_landscape != (desktop_mode.w > desktop_mode.h) {
        let mut new_mode = desktop_mode;
        new_mode.w = desktop_mode.h;
        new_mode.h = desktop_mode.w;

        set_desktop_display_mode(display, &new_mode);
    }

    // Same deal with the fullscreen modes
    crate::video::core::with_display(display, |d| {
        for mode in &mut d.fullscreen_modes {
            if is_landscape != (mode.w > mode.h) {
                std::mem::swap(&mut mode.w, &mut mode.h);
            }
        }
    });

    let orientation = match interface_orientation {
        UIInterfaceOrientationPortrait => DisplayOrientation::Portrait,
        UIInterfaceOrientationPortraitUpsideDown => DisplayOrientation::PortraitFlipped,
        // Bug: UIInterfaceOrientationLandscapeLeft/Right are reversed - http://openradar.appspot.com/7216046
        UIInterfaceOrientationLandscapeLeft => DisplayOrientation::LandscapeFlipped,
        // Bug: UIInterfaceOrientationLandscapeLeft/Right are reversed - http://openradar.appspot.com/7216046
        UIInterfaceOrientationLandscapeRight => DisplayOrientation::Landscape,
        _ => DisplayOrientation::Unknown,
    };
    crate::events::window::send_display_event(
        display,
        EventType::DISPLAY_ORIENTATION,
        orientation as i32,
        0,
    );
}
