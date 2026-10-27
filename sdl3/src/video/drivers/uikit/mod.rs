// Rust translation of src/video/uikit/SDL_uikitvideo.m and
// SDL_uikitvideo.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The UIKit video driver (iOS, tvOS, visionOS): the lifecycle
//! notifications and the event pump ([`events`]), and the displays
//! ([`modes`]).
//!
//! Upstream writes the driver in Objective-C; its classes
//! (`SDL_LifecycleObserver`, `SDL_DisplayWatch`, ...) are declared at run
//! time here, their methods implemented in Rust (see
//! [`crate::core::apple::objc`]), as the Cocoa driver's are.
//!
//! The driver only starts in an app: on the main thread of a process whose
//! `UIApplicationMain()` runs. Note: upstream
//! never runs outside `UIApplicationMain()`; here a program that isn't an
//! app (a test program run as a plain process of a simulated device)
//! falls through to the next driver, as one on another thread does with
//! the Cocoa driver.
//!
//! Not translated yet: the app delegate, windows and their views and view
//! controllers, so no window can be made yet, the fullscreen display
//! modes and usable bounds, touches, the keyboard (`GCKeyboard`, the
//! on-screen one), the mouse (`GCMouse`), pens, the clipboard, message
//! boxes, OpenGL ES, Metal views and Vulkan.
//! `SDL_NSLog()`, which upstream copies here from the Cocoa driver, is
//! [`crate::log`]'s on every Apple platform.

#![allow(non_upper_case_globals, non_snake_case)] // (the SDK's names are kept)

pub(crate) mod events;
pub(crate) mod modes;
pub(crate) mod sys;

use std::sync::Arc;

use crate::core::apple::objc::{autoreleasepool, class, msg_send, Bool, Id};
use crate::core::apple::uikit::shared_application;
use crate::error::Result;
use crate::video::sysvideo::{DeviceCaps, SystemTheme, VideoBootStrap, VideoDriver};

#[cfg(not(any(target_os = "tvos", target_os = "visionos")))]
use sys::UIInterfaceOrientation;
use sys::{
    UISceneActivationStateForegroundActive, UISceneActivationStateForegroundInactive,
    UIUserInterfaceStyleDark, UIUserInterfaceStyleLight,
};

/// The UIKit video driver (the function pointers of the `SDL_VideoDevice`
/// that `UIKit_CreateDevice()` fills in). Upstream's `SDL_UIKitVideoData`
/// holds the clipboard's state, which isn't translated yet.
pub(crate) struct UIKitVideo;

/// Translation of `UIKit_CreateDevice()`.
fn create_device() -> Option<Arc<dyn VideoDriver>> {
    autoreleasepool(|| {
        // Note: upstream always runs in UIApplicationMain(), on the main
        // thread; outside an app (or on another thread) there's no UIKit
        // to drive, so the next driver gets its chance.
        if !crate::core::apple::foundation::is_main_thread() || shared_application().is_null() {
            return None; // this doesn't SDL_SetError() because SDL_InitVideo is just going to overwrite it.
        }

        let video: Arc<dyn VideoDriver> = Arc::new(UIKitVideo);
        Some(video)
    })
}

/// Translation of `UIKIT_bootstrap`.
pub(crate) static UIKIT_BOOTSTRAP: VideoBootStrap = VideoBootStrap {
    name: "uikit",
    desc: "SDL UIKit video driver",
    create: create_device,
    // (UIKit_ShowMessageBox() isn't translated yet)
    show_message_box: None,
    is_preferred: false,
};

/// Translation of `UIKit_VideoInit()`.
fn video_init() -> Result<()> {
    let _ = crate::video::core::with_device(|v| v.gl_driver_loaded = 1);

    if !modes::init_modes() {
        return Err(crate::error::Error::new(
            "The UIKit video driver couldn't add its displays",
        ));
    }

    // (SDL_InitGCKeyboard(), SDL_InitGCMouse() and UIKit_InitClipboard()
    // come with the keyboard, the mouse and the clipboard, which aren't
    // translated yet)

    Ok(())
}

/// Translation of `UIKit_VideoQuit()`.
fn video_quit() {
    // (UIKit_QuitClipboard(), SDL_QuitGCKeyboard(), SDL_QuitGCMouse() and
    // UIKit_QuitPen(): see video_init())

    modes::quit_modes();
}

/// Translation of `UIKit_SuspendScreenSaver()`.
fn suspend_screen_saver(suspend: bool) -> Result<()> {
    autoreleasepool(|| {
        let app = shared_application();

        // Prevent the display from dimming and going to sleep.
        // SAFETY: a UIApplication property.
        unsafe {
            let () = msg_send![app, setIdleTimerDisabled: Bool::from(suspend)];
        }
    });
    Ok(())
}

/// The system theme, from the main screen's traits. Translation of
/// `UIKit_GetSystemTheme()`.
pub(crate) fn get_system_theme() -> SystemTheme {
    #[cfg(not(target_os = "visionos"))]
    if crate::core::apple::foundation::available!(ios(12, 0), tvos(10, 0)) {
        // SAFETY: UIScreen's main screen and its trait collection.
        let style: sys::UIUserInterfaceStyle = unsafe {
            let screen: Id = msg_send![class!(UIScreen), mainScreen];
            let traits: Id = msg_send![screen, traitCollection];
            msg_send![traits, userInterfaceStyle]
        };
        match style {
            UIUserInterfaceStyleDark => return SystemTheme::Dark,
            UIUserInterfaceStyleLight => return SystemTheme::Light,
            _ => {}
        }
    }
    SystemTheme::Unknown
}

/// The interface orientation of the app (its active window scene's, from
/// iOS 27). Translation of `UIKit_GetInterfaceOrientation()`.
#[cfg(not(any(target_os = "tvos", target_os = "visionos")))]
pub(crate) fn get_interface_orientation() -> UIInterfaceOrientation {
    if crate::core::apple::foundation::available!(ios(27, 0)) {
        let window_scene = get_active_window_scene();
        if !window_scene.is_null() {
            // SAFETY: UIWindowScene's effective geometry (iOS 26).
            return unsafe {
                let geometry: Id = msg_send![window_scene, effectiveGeometry];
                msg_send![geometry, interfaceOrientation]
            };
        }
        return sys::UIInterfaceOrientationUnknown;
    }

    // (deprecated since iOS 13)
    // SAFETY: a UIApplication property.
    unsafe { msg_send![shared_application(), statusBarOrientation] }
}

/// The app's active window scene, else a foreground one, else any (nil if
/// it has none, and before iOS 13). Translation of
/// `UIKit_GetActiveWindowScene()`.
pub(crate) fn get_active_window_scene() -> Id {
    if crate::core::apple::foundation::available!(ios(13, 0), tvos(13, 0)) {
        // SAFETY: UIApplication's connected scenes (an NSSet), and the
        // scenes' properties.
        unsafe {
            let connected: Id = msg_send![shared_application(), connectedScenes];
            let all: Id = msg_send![connected, allObjects];
            let window_scenes: Vec<Id> = crate::core::apple::foundation::ns_array_items(all)
                .into_iter()
                .filter(|&scene| crate::core::apple::objc::is_kind_of(scene, class!(UIWindowScene)))
                .collect();
            let state = |scene: Id| -> crate::core::apple::objc::NSInteger {
                msg_send![scene, activationState]
            };

            // First, try to find an active foreground scene
            if let Some(&scene) = window_scenes
                .iter()
                .find(|&&s| state(s) == UISceneActivationStateForegroundActive)
            {
                return scene;
            }

            // If no active scene, return any foreground scene
            if let Some(&scene) = window_scenes
                .iter()
                .find(|&&s| state(s) == UISceneActivationStateForegroundInactive)
            {
                return scene;
            }

            // Last resort: return first window scene
            if let Some(&scene) = window_scenes.first() {
                return scene;
            }
        }
    }

    crate::core::apple::objc::NIL
}

impl VideoDriver for UIKitVideo {
    fn caps(&self) -> DeviceCaps {
        DeviceCaps::SENDS_FULLSCREEN_DIMENSIONS
    }

    fn video_init(&self) -> Result<()> {
        // (UIKit_CreateDevice() sets device->system_theme, which exists
        // once the driver is chosen; no event is sent. It also sets
        // gl_config.accelerated to 1, which SDL_VideoInit() resets.)
        let theme = autoreleasepool(get_system_theme);
        let _ = crate::video::core::with_device(|v| v.system_theme = theme);
        video_init()
    }

    fn video_quit(&self) {
        video_quit()
    }

    fn pump_events(&self) -> Option<()> {
        events::pump_events();
        Some(())
    }

    fn suspend_screen_saver(&self, suspend: bool) -> Option<Result<()>> {
        Some(suspend_screen_saver(suspend))
    }
}
