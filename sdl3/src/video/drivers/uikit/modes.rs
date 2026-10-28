// Rust translation of src/video/uikit/SDL_uikitmodes.m and
// SDL_uikitmodes.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The displays: one per `UIScreen` (the fake 1280x720 one on visionOS),
//! added at video init and when a screen connects (`SDL_DisplayWatch`),
//! removed when it disconnects; their modes (the screen's modes, each in
//! both orientations on the main screen, in points with the screen's
//! native scale as the pixel density, at the screen's maximum frame rate),
//! mode changes, usable bounds (the screen's bounds), and the main
//! screen's orientation (`SDL_OnApplicationDidChangeStatusBarOrientation()`),
//! which the desktop and fullscreen modes follow.
//!
//! Upstream keeps each mode's `UIScreenMode` in the mode
//! (`SDL_UIKitDisplayModeData`, the mode's `internal`), for
//! `UIKit_SetDisplayMode()`. The video core's modes carry no driver data
//! here, so the display's driver data keeps them, by the mode as the core
//! finalized it (and turned with it), as the X11 driver keeps its modes'.

use crate::core::apple::foundation::CGRect;
#[cfg(not(target_os = "visionos"))]
use crate::core::apple::foundation::{CGFloat, CGSize};
use crate::core::apple::objc::{autoreleasepool, StrongPtr};
#[cfg(not(target_os = "visionos"))]
use crate::core::apple::objc::{class, msg_send, Id};
#[cfg(not(target_os = "visionos"))]
use crate::error::Error;
use crate::error::Result;
use crate::events::DisplayID;
#[cfg(not(any(target_os = "tvos", target_os = "visionos")))]
use crate::events::EventType;
use crate::video::core::with_display;
#[cfg(not(target_os = "visionos"))]
use crate::video::display::finalize_display_mode;
#[cfg(not(target_os = "visionos"))]
use crate::video::display::{add_fullscreen_display_mode, del_video_display};
use crate::video::display::{add_video_display, display_bounds, displays};
#[cfg(not(any(target_os = "tvos", target_os = "visionos")))]
use crate::video::display::{primary_display, set_desktop_display_mode};
use crate::video::sysvideo::{DisplayOrientation, VideoDisplay};
use crate::video::{DisplayMode, PixelFormat, Rect};

#[cfg(not(any(target_os = "tvos", target_os = "visionos")))]
use super::sys::*;

/// because visionOS does not have a screen
/// we create a fake display to maintain compatibility.
/// By default, a window measures 1280x720 pt.
/// <https://developer.apple.com/design/human-interface-guidelines/windows#visionOS>
#[cfg(target_os = "visionos")]
pub(crate) const SDL_XR_SCREENWIDTH: i32 = 1280;
#[cfg(target_os = "visionos")]
pub(crate) const SDL_XR_SCREENHEIGHT: i32 = 720;

/// Translation of `SDL_UIKitDisplayData` (the display's `internal`), with
/// the `SDL_UIKitDisplayModeData` of its modes (see the module's
/// description).
pub(crate) struct DisplayData {
    /// `uiscreen` (nil on visionOS).
    #[cfg_attr(target_os = "visionos", allow(dead_code))]
    pub(crate) uiscreen: StrongPtr,
    /// The desktop mode's `uiscreenmode` (`desktop_mode.internal`; nil
    /// once lost, see `on_application_did_change_status_bar_orientation()`).
    #[cfg(not(target_os = "visionos"))]
    #[cfg_attr(target_os = "tvos", allow(dead_code))] // (as upstream's)
    desktop_uiscreenmode: StrongPtr,
    /// The fullscreen modes' `uiscreenmode`s, by the mode as the video core
    /// keeps it.
    #[cfg(not(target_os = "visionos"))]
    modes: Vec<(DisplayMode, StrongPtr)>,
}

/// Run `f` on a display's driver data, if it's one of this driver's.
#[cfg(not(target_os = "visionos"))]
fn with_display_data<R>(display: DisplayID, f: impl FnOnce(&mut DisplayData) -> R) -> Option<R> {
    with_display(display, |d| {
        d.internal
            .as_mut()
            .and_then(|i| i.downcast_mut::<DisplayData>())
            .map(f)
    })
    .flatten()
}

/// The `UIScreen` of a display, if it's one of this driver's.
#[cfg(not(target_os = "visionos"))]
pub(crate) fn display_screen(display: DisplayID) -> Option<Id> {
    with_display_data(display, |data| data.uiscreen.get())
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
// Modes

/// Translation of `UIKit_GetDisplayModeRefreshRate()`.
#[cfg(not(target_os = "visionos"))]
fn get_display_mode_refresh_rate(uiscreen: Id) -> f32 {
    // SAFETY: a UIScreen property.
    let fps: crate::core::apple::objc::NSInteger =
        unsafe { msg_send![uiscreen, maximumFramesPerSecond] };
    fps as f32
}

/// Add a fullscreen mode of a display, keeping its `UIScreenMode`.
/// Translation of `UIKit_AddSingleDisplayMode()` (with
/// `UIKit_AllocateDisplayModeData()`).
#[cfg(not(target_os = "visionos"))]
fn add_single_display_mode(
    display: DisplayID,
    w: i32,
    h: i32,
    uiscreen: Id,
    uiscreenmode: Id,
) -> bool {
    // SAFETY: a UIScreen property.
    let native_scale: CGFloat = unsafe { msg_send![uiscreen, nativeScale] };
    let mode = DisplayMode {
        w,
        h,
        pixel_density: native_scale as f32,
        refresh_rate: get_display_mode_refresh_rate(uiscreen),
        format: PixelFormat::ABGR8888,
        ..DisplayMode::default()
    };

    if add_fullscreen_display_mode(display, &mode) {
        // (the mode as the video core keeps it)
        let mut kept = mode;
        kept.display_id = display;
        finalize_display_mode(&mut kept);
        // SAFETY: the screen's mode (nil allowed).
        let data = unsafe { StrongPtr::retain(uiscreenmode) };
        with_display_data(display, |d| d.modes.push((kept, data)));
        true
    } else {
        false
    }
}

/// Translation of `UIKit_AddDisplayMode()`.
#[cfg(not(target_os = "visionos"))]
fn add_display_mode(
    display: DisplayID,
    w: i32,
    h: i32,
    uiscreen: Id,
    uiscreenmode: Id,
    add_rotation: bool,
) -> bool {
    if !add_single_display_mode(display, w, h, uiscreen, uiscreenmode) {
        return false;
    }

    if add_rotation {
        // Add the rotated version
        if !add_single_display_mode(display, h, w, uiscreen, uiscreenmode) {
            return false;
        }
    }

    true
}

/// Translation of `GetUIScreenModeSize()`.
#[cfg(not(target_os = "visionos"))]
fn get_uiscreen_mode_size(uiscreen: Id, mode: Id) -> CGSize {
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
        let mut size: CGSize = msg_send![mode, size];
        let scale: CGFloat = msg_send![uiscreen, scale];

        size.width = crate::stdlib::math::round(size.width / scale);
        size.height = crate::stdlib::math::round(size.height / scale);

        size
    }
}

// ---------------------------------------------------------------------------
// Displays

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

        let native_scale: CGFloat = msg_send![uiscreen, nativeScale];
        let mode = DisplayMode {
            w: size.width as i32,
            h: size.height as i32,
            pixel_density: native_scale as f32,
            format: PixelFormat::ABGR8888,
            refresh_rate: get_display_mode_refresh_rate(uiscreen),
            ..DisplayMode::default()
        };

        let desktop_uiscreenmode = StrongPtr::retain(uiscreenmode);

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
            let current: CGFloat = msg_send![uiscreen, currentEDRHeadroom];
            if current > 1.0 {
                display.hdr.hdr_headroom = current as f32;
            } else {
                let potential: CGFloat = msg_send![uiscreen, potentialEDRHeadroom];
                display.hdr.hdr_headroom = potential as f32;
            }
        }

        // Allocate the display data
        display.internal = Some(Box::new(DisplayData {
            uiscreen: StrongPtr::retain(uiscreen),
            desktop_uiscreenmode,
            modes: Vec::new(),
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

    // FIXME (upstream): upstream zeroes the mode but not the display, so
    // the display's other fields (its name, content scale, current
    // orientation) are whatever its stack held; here they're the
    // defaults, as the other UIKit platforms' SDL_zero() gives them.
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
            // Note (upstream): upstream releases only the display's data
            // here, and SDL_DelVideoDisplay() SDL_free()s its modes' (the
            // SDL_UIKitDisplayModeData objects); here dropping the
            // display's data releases them all.
            let _ = with_display(id, |d| d.internal = None);
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
    let bounds: CGRect = unsafe { msg_send![uiscreen, bounds] };
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

/// Add a display's fullscreen modes. Translation of
/// `UIKit_GetDisplayModes()`.
pub(crate) fn get_display_modes(display: DisplayID) {
    #[cfg(not(target_os = "visionos"))]
    autoreleasepool(|| {
        let Some(uiscreen) = display_screen(display) else {
            return;
        };

        let is_landscape = is_display_landscape(uiscreen);
        let add_rotation: bool;
        let available_modes: Vec<Id>;

        #[cfg(target_os = "tvos")]
        {
            add_rotation = false;
            // SAFETY: a UIScreen property.
            available_modes = vec![unsafe { msg_send![uiscreen, currentMode] }];
        }
        #[cfg(not(target_os = "tvos"))]
        {
            add_rotation = uiscreen == main_screen();
            // SAFETY: a UIScreen property, an array of UIScreenModes.
            available_modes = unsafe {
                let modes: Id = msg_send![uiscreen, availableModes];
                crate::core::apple::foundation::ns_array_items(modes)
            };
        }

        for uimode in available_modes {
            let size = get_uiscreen_mode_size(uiscreen, uimode);
            let mut w = size.width as i32;
            let mut h = size.height as i32;

            // Make sure the width/height are oriented correctly
            if is_landscape != (w > h) {
                std::mem::swap(&mut w, &mut h);
            }

            add_display_mode(display, w, h, uiscreen, uimode, add_rotation);
        }
    });
    #[cfg(target_os = "visionos")]
    let _ = display;
}

/// The `UIScreenMode` of one of a display's modes (`((SDL_UIKitDisplayModeData
/// *)mode->internal).uiscreenmode`; nil for a mode it doesn't have).
/// Upstream takes the mode's own data; the desktop mode is the one the
/// video core passes from `display->desktop_mode`.
#[cfg(not(any(target_os = "tvos", target_os = "visionos")))]
pub(crate) fn screen_mode_of(display: DisplayID, mode: &DisplayMode) -> StrongPtr {
    let desktop_mode = with_display(display, |d| d.desktop_mode);
    with_display_data(display, |d| {
        if desktop_mode == Some(*mode) {
            d.desktop_uiscreenmode.clone()
        } else {
            d.modes
                .iter()
                .find(|(m, _)| m == mode)
                .map(|(_, data)| data.clone())
                .unwrap_or_default()
        }
    })
    .unwrap_or_default()
}

/// Make one of a display's fullscreen modes (as the video core keeps them)
/// its current mode (`SDL_SetCurrentDisplayMode(display, &mode)`, with a
/// mode from `SDL_GetClosestFullscreenDisplayMode()`).
#[cfg(not(any(target_os = "tvos", target_os = "visionos")))]
pub(crate) fn set_current_display_mode_to(display: DisplayID, mode: &DisplayMode) {
    use crate::video::sysvideo::CurrentMode;
    let current = with_display(display, |d| {
        d.fullscreen_modes
            .iter()
            .position(|m| m == mode)
            .map_or(CurrentMode::Other(*mode), CurrentMode::Fullscreen)
    });
    if let Some(current) = current {
        crate::video::display::set_current_display_mode(display, current);
    }
}

/// Translation of `UIKit_SetDisplayMode()`.
pub(crate) fn set_display_mode(display: DisplayID, mode: &DisplayMode) -> Result<()> {
    #[cfg(not(target_os = "visionos"))]
    {
        autoreleasepool(|| {
            let Some(uiscreen) = display_screen(display) else {
                return Ok(());
            };

            #[cfg(not(target_os = "tvos"))]
            {
                let modedata = screen_mode_of(display, mode);
                // SAFETY: a UIScreen property (nil, as upstream's nil mode
                // data has, is UIKit's to handle).
                unsafe {
                    let () = msg_send![uiscreen, setCurrentMode: modedata.get()];
                }
            }

            if uiscreen == main_screen() {
                /* [UIApplication setStatusBarOrientation:] no longer works reliably
                 * in recent iOS versions, so we can't rotate the screen when setting
                 * the display mode. */
                if mode.w > mode.h {
                    if !is_display_landscape(uiscreen) {
                        return Err(Error::new(
                            "Screen orientation does not match display mode size",
                        ));
                    }
                } else if mode.w < mode.h && is_display_landscape(uiscreen) {
                    return Err(Error::new(
                        "Screen orientation does not match display mode size",
                    ));
                }
            }
            Ok(())
        })
    }
    #[cfg(target_os = "visionos")]
    {
        let _ = (display, mode);
        Ok(())
    }
}

/// Translation of `UIKit_GetDisplayUsableBounds()`.
pub(crate) fn get_display_usable_bounds(display: DisplayID) -> Result<Rect> {
    autoreleasepool(|| {
        #[cfg(target_os = "visionos")]
        let frame = CGRect::new(
            0.0,
            0.0,
            SDL_XR_SCREENWIDTH as f64,
            SDL_XR_SCREENHEIGHT as f64,
        );
        #[cfg(not(target_os = "visionos"))]
        let frame: CGRect = {
            let uiscreen = display_screen(display).ok_or_else(|| Error::new("Invalid display"))?;
            // SAFETY: a UIScreen property.
            unsafe { msg_send![uiscreen, bounds] }
        };

        /* the default function iterates displays to make a fake offset,
        as if all the displays were side-by-side, which is fine for iOS. */
        let mut rect = display_bounds(display)?;

        rect.x += frame.origin.x as i32;
        rect.y += frame.origin.y as i32;
        rect.w = frame.size.width as i32;
        rect.h = frame.size.height as i32;

        Ok(rect)
    })
}

/// Translation of `UIKit_QuitModes()`.
pub(crate) fn quit_modes() {
    #[cfg(not(target_os = "visionos"))]
    watch::stop();

    // Release Objective-C objects, so higher level doesn't free() them.
    // (the display's data holds its modes' too)
    autoreleasepool(|| {
        for id in displays().unwrap_or_default() {
            let _ = with_display(id, |d| d.internal = None);
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
    let Some((desktop_mode, fullscreen_active)) =
        with_display(display, |d| (d.desktop_mode, d.fullscreen_active))
    else {
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

        // Make sure we don't free the current display mode data
        // (the desktop mode's data goes with new_mode)
        // FIXME (upstream): SDL_SetDesktopDisplayMode() doesn't take
        // new_mode while a fullscreen window is active, so the desktop mode
        // is left with no data (nil), and restoring it sets the screen's
        // current mode to nil.
        if fullscreen_active {
            with_display_data(display, |d| d.desktop_uiscreenmode = StrongPtr::nil());
        }

        set_desktop_display_mode(display, &new_mode);
    }

    // Same deal with the fullscreen modes
    // (and their data, kept by the mode)
    let turn = |mode: &mut DisplayMode| {
        if is_landscape != (mode.w > mode.h) {
            std::mem::swap(&mut mode.w, &mut mode.h);
        }
    };
    with_display(display, |d| d.fullscreen_modes.iter_mut().for_each(turn));
    with_display_data(display, |d| {
        d.modes.iter_mut().for_each(|(mode, _)| turn(mode))
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
