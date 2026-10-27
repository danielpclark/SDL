// Rust translation of src/video/uikit/SDL_uikitwindow.m and
// SDL_uikitwindow.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Windows: one `UIWindow` per display (in the app's active window scene,
//! or the scene the app names), whose root view controller
//! ([`super::viewcontroller`]) shows the window's view
//! ([`super::view`]); showing (key and visible, with the mouse and
//! keyboard focus) and hiding, the status bar of fullscreen and
//! borderless windows, fullscreen (the view controller's business: the
//! window covers its screen either way), the size in pixels (the view's
//! bounds at the screen's native scale, for high density windows), the
//! orientations a window supports (`SDL_HINT_ORIENTATIONS`, or those its
//! shape suits), the animation callback
//! ([`crate::video::Window::set_ios_animation_callback`]), and the
//! pointer's lock ([`update_pointer_lock`]).
//!
//! The driver data of a window ([`WindowData`], `SDL_UIKitWindowData`)
//! lives in `window->internal`; the view controller and the views keep
//! the window's ID and find it there.

use std::sync::{Arc, Mutex, MutexGuard};

#[cfg(not(target_os = "visionos"))]
use crate::core::apple::foundation::CGSize;
use crate::core::apple::foundation::{available, ns_string};
use crate::core::apple::objc::{autoreleasepool, class, msg_send, Id, NSInteger, StrongPtr, NIL};
#[cfg(not(any(target_os = "tvos", target_os = "visionos")))]
use crate::core::apple::objc::{responds_to, sel, Bool, NSUInteger};
use crate::error::{Error, Result};
use crate::events::window::{send_window_event, WindowFlags};
use crate::events::{EventType, WindowID};
use crate::properties::Properties;
use crate::video::core::{window_ids, with_window};
use crate::video::sysvideo::{FullscreenOp, FullscreenResult};
use crate::video::window::{
    display_for_window, Window, PROP_WINDOW_CREATE_WINDOWSCENE_POINTER,
    PROP_WINDOW_UIKIT_METAL_VIEW_TAG_NUMBER, PROP_WINDOW_UIKIT_WINDOW_POINTER,
};

#[cfg(not(any(target_os = "tvos", target_os = "visionos")))]
use super::sys::*;
use super::{view, viewcontroller};

/// Translation of `SDL_METALVIEW_TAG`.
pub(crate) const SDL_METALVIEW_TAG: NSInteger = 255;

/// The driver data of a window. Translation of `SDL_UIKitWindowData`.
pub(crate) struct WindowData {
    uiwindow: StrongPtr,
    viewcontroller: Mutex<StrongPtr>,
    /// Array of SDL_uikitviews owned by this window.
    views: Mutex<Vec<StrongPtr>>,
}

impl WindowData {
    /// `data.uiwindow`.
    pub(crate) fn uiwindow(&self) -> Id {
        self.uiwindow.get()
    }

    /// `data.viewcontroller` (nil until the window is set up).
    pub(crate) fn viewcontroller(&self) -> Id {
        self.viewcontroller
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get()
    }

    /// `data.views`. Never message UIKit while holding it.
    pub(crate) fn views(&self) -> MutexGuard<'_, Vec<StrongPtr>> {
        self.views.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// The driver data of a window (`window->internal`).
pub(crate) fn window_data(window: WindowID) -> Option<Arc<WindowData>> {
    with_window(window, |w| {
        w.internal
            .as_ref()
            .and_then(|i| i.downcast_ref::<Arc<WindowData>>())
            .cloned()
    })
    .ok()
    .flatten()
}

/// `window->flags` (none once the window is gone).
fn flags(window: WindowID) -> WindowFlags {
    with_window(window, |w| w.flags()).unwrap_or_default()
}

/// The `UIScreen` of the display a window is on (nil if none).
#[cfg(not(target_os = "visionos"))]
fn window_display_screen(window: WindowID) -> Id {
    display_for_window(window)
        .ok()
        .and_then(super::modes::display_screen)
        .unwrap_or(NIL)
}

/// Translation of `SetupWindowData()`.
fn setup_window_data(
    window: WindowID,
    uiwindow: StrongPtr,
    _create_props: &Properties,
) -> Result<()> {
    #[cfg(target_os = "visionos")]
    let frame = super::compute_view_frame(window);
    #[cfg(not(target_os = "visionos"))]
    let screen = window_display_screen(window);
    #[cfg(not(target_os = "visionos"))]
    let frame = super::compute_view_frame(window, screen);

    #[allow(unused_mut)]
    let mut width = frame.size.width as i32;
    #[allow(unused_mut)]
    let mut height = frame.size.height as i32;

    let data = Arc::new(WindowData {
        uiwindow,
        viewcontroller: Mutex::new(StrongPtr::nil()),
        views: Mutex::new(Vec::new()),
    });

    with_window(window, |w| w.internal = Some(Box::new(data.clone())))?;

    #[cfg(not(target_os = "visionos"))]
    if screen != super::modes::main_screen() {
        with_window(window, |w| {
            w.core.flags &= !WindowFlags::RESIZABLE; // window is NEVER resizable
            w.core.flags &= !WindowFlags::INPUT_FOCUS; // never has input focus
            w.core.flags |= WindowFlags::BORDERLESS; // never has a status bar.
        })?;
    }

    #[cfg(not(any(target_os = "tvos", target_os = "visionos")))]
    if screen == super::modes::main_screen() {
        let orients = get_supported_orientations(window);
        let supports_landscape = (orients & UIInterfaceOrientationMaskLandscape) != 0;
        let supports_portrait = (orients
            & (UIInterfaceOrientationMaskPortrait | UIInterfaceOrientationMaskPortraitUpsideDown))
            != 0;

        // Make sure the width/height are oriented correctly
        if (width > height && !supports_landscape) || (height > width && !supports_portrait) {
            std::mem::swap(&mut width, &mut height);
        }
    }

    // (upstream doesn't set the x/y position, it's already placed on a display)
    with_window(window, |w| {
        w.core.w = width;
        w.core.h = height;
    })?;

    let props = Window::from_raw(window).properties()?;
    // (a pointer, kept as a number, as the other drivers keep theirs)
    let _ = props.set(
        PROP_WINDOW_UIKIT_WINDOW_POINTER,
        data.uiwindow() as usize as i64,
    );
    let _ = props.set(
        PROP_WINDOW_UIKIT_METAL_VIEW_TAG_NUMBER,
        SDL_METALVIEW_TAG as i64,
    );

    // (visionOS's settings string comes with visionOS)

    /* The View Controller will handle rotating the view when the device
     * orientation changes. This will trigger resize events, if appropriate. */
    let viewcontroller = viewcontroller::new_view_controller(window);
    *data
        .viewcontroller
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = viewcontroller;

    /* The window will initially contain a generic view so resizes, touch events,
     * etc. can be handled without an active OpenGL view/context. */
    let view = view::new_view(frame);

    /* Sets this view as the controller's view, and adds the view to the window
     * hierarchy. */
    view::set_sdl_window(view.get(), Some(window));

    Ok(())
}

/// Translation of `UIKit_CreateWindow()`.
pub(crate) fn create_window(window: WindowID, create_props: &Properties) -> Result<()> {
    autoreleasepool(|| {
        let display = display_for_window(window).unwrap_or(0);

        // We currently only handle a single window per display on iOS
        for other in window_ids() {
            if other != window && display_for_window(other).unwrap_or(0) == display {
                return Err(Error::new("Only one window allowed per display."));
            }
        }

        #[cfg(not(target_os = "visionos"))]
        let uiscreen = super::modes::display_screen(display).unwrap_or(NIL);

        /* If monitor has a resolution of 0x0 (hasn't been explicitly set by the
         * user, so it's in standby), try to force the display to a resolution
         * that most closely matches the desired window size. */
        #[cfg(not(any(target_os = "tvos", target_os = "visionos")))]
        {
            // SAFETY: UIScreen's current mode, a UIScreenMode, and its size.
            let origsize: CGSize = unsafe {
                let current: Id = msg_send![uiscreen, currentMode];
                msg_send![current, size]
            };
            if origsize.width == 0.0 && origsize.height == 0.0 {
                let (w, h, window_flags) =
                    with_window(window, |w| (w.core.w, w.core.h, w.flags()))?;
                let include_high_density_modes =
                    window_flags.contains(WindowFlags::HIGH_PIXEL_DENSITY);
                if let Ok(bestmode) = crate::video::closest_fullscreen_display_mode(
                    display,
                    w,
                    h,
                    0.0,
                    include_high_density_modes,
                ) {
                    let modedata = super::modes::screen_mode_of(display, &bestmode);
                    // SAFETY: a UIScreen property.
                    unsafe {
                        let () = msg_send![uiscreen, setCurrentMode: modedata.get()];
                    }

                    /* desktop_mode doesn't change here (the higher level will
                     * use it to set all the screens back to their defaults
                     * upon window destruction, SDL_Quit(), etc. */
                    super::modes::set_current_display_mode_to(display, &bestmode);
                }
            }

            if uiscreen == super::modes::main_screen() {
                if available!(ios(13, 0)) {
                    // iOS 13+ uses view controller's prefersStatusBarHidden
                } else {
                    let hidden =
                        flags(window).intersects(WindowFlags::FULLSCREEN | WindowFlags::BORDERLESS);
                    // SAFETY: a UIApplication property (deprecated in iOS 9).
                    unsafe {
                        let () = msg_send![crate::core::apple::uikit::shared_application(), setStatusBarHidden: Bool::from(hidden)];
                    }
                }
            }
        }

        let mut uiwindow = StrongPtr::nil();
        if available!(ios(13, 0), tvos(13, 0)) {
            let mut scene: Id = create_props
                .get_number(PROP_WINDOW_CREATE_WINDOWSCENE_POINTER)
                .map_or(NIL, |p| p as usize as Id);
            if scene.is_null() {
                scene = super::get_active_window_scene();
            }
            if !scene.is_null() {
                // SAFETY: UIWindow's initializer (owned), on a UIWindowScene.
                uiwindow = unsafe {
                    let w: Id = msg_send![class!(UIWindow), alloc];
                    StrongPtr::from_owned(msg_send![w, initWithWindowScene: scene])
                };

                // (visionOS's initial geometry request comes with visionOS)
            }
        }
        if uiwindow.is_nil() {
            // ignore the size user requested, and make a fullscreen window
            #[cfg(target_os = "visionos")]
            let frame = crate::core::apple::foundation::CGRect::new(
                0.0,
                0.0,
                super::modes::SDL_XR_SCREENWIDTH as f64,
                super::modes::SDL_XR_SCREENHEIGHT as f64,
            );
            // SAFETY: a UIScreen property.
            #[cfg(not(target_os = "visionos"))]
            let frame: crate::core::apple::foundation::CGRect =
                unsafe { msg_send![uiscreen, bounds] };
            // SAFETY: UIWindow's initializer (owned).
            uiwindow = unsafe {
                let w: Id = msg_send![class!(UIWindow), alloc];
                StrongPtr::from_owned(msg_send![w, initWithFrame: frame])
            };
        }

        // put the window on an external display if appropriate.
        #[cfg(not(target_os = "visionos"))]
        if uiscreen != super::modes::main_screen() {
            if available!(ios(13, 0), tvos(13, 0)) {
                // iOS 13+ uses UIWindowScene to manage screen association
            } else {
                // SAFETY: a UIWindow property (deprecated in iOS 13).
                unsafe {
                    let () = msg_send![uiwindow.get(), setScreen: uiscreen];
                }
            }
        }

        setup_window_data(window, uiwindow, create_props)
    })
}

/// Translation of `UIKit_SetWindowTitle()`.
pub(crate) fn set_window_title(window: WindowID) {
    autoreleasepool(|| {
        let Some(data) = window_data(window) else {
            return;
        };
        let title = with_window(window, |w| w.title.clone())
            .ok()
            .flatten()
            .unwrap_or_default();
        let title = ns_string(&title);
        // SAFETY: a UIViewController property.
        unsafe {
            let () = msg_send![data.viewcontroller(), setTitle: title.get()];
        }
    });
}

/// Translation of `UIKit_SetWindowSize()`: windows have their screen's
/// size, but on visionOS (whose geometry request comes with visionOS).
pub(crate) fn set_window_size(_window: WindowID) {}

/// Translation of `UIKit_ShowWindow()`.
pub(crate) fn show_window(window: WindowID) {
    autoreleasepool(|| {
        let Some(data) = window_data(window) else {
            return;
        };
        // SAFETY: a UIWindow method.
        unsafe {
            let () = msg_send![data.uiwindow(), makeKeyAndVisible];
        }

        // Make this window the current mouse focus for touch input
        #[cfg(not(target_os = "visionos"))]
        let on_main_screen = window_display_screen(window) == super::modes::main_screen();
        #[cfg(target_os = "visionos")]
        let on_main_screen = true;
        if on_main_screen {
            crate::events::mouse::set_mouse_focus(Some(window));
            let _ = crate::events::keyboard::set_keyboard_focus(Some(window));
        }
    });
}

/// Translation of `UIKit_HideWindow()`.
pub(crate) fn hide_window(window: WindowID) {
    autoreleasepool(|| {
        if let Some(data) = window_data(window) {
            // SAFETY: a UIView property.
            unsafe {
                let () = msg_send![data.uiwindow(), setHidden: crate::core::apple::objc::YES];
            }
        }
    });
}

/// Translation of `UIKit_RaiseWindow()`.
pub(crate) fn raise_window(_window: WindowID) {
    /* We don't currently offer a concept of "raising" the SDL window, since
     * we only allow one per display, in the iOS fashion.
     * However, we use this entry point to rebind the context to the view
     * during OnWindowRestored processing. */
    // (with OpenGL ES, which isn't translated yet)
}

/// Translation of `UIKit_UpdateWindowBorder()`.
fn update_window_border(window: WindowID) {
    let Some(data) = window_data(window) else {
        return;
    };
    let viewcontroller = data.viewcontroller();

    // SAFETY: UIApplication's, UIWindow's, UIViewController's and UIView's
    // properties and methods.
    unsafe {
        #[cfg(not(any(target_os = "tvos", target_os = "visionos")))]
        {
            let screen: Id = msg_send![data.uiwindow(), screen];
            if screen == super::modes::main_screen() {
                if available!(ios(13, 0)) {
                    // iOS 13+ uses view controller's prefersStatusBarHidden
                } else {
                    let hidden =
                        flags(window).intersects(WindowFlags::FULLSCREEN | WindowFlags::BORDERLESS);
                    let () = msg_send![crate::core::apple::uikit::shared_application(), setStatusBarHidden: Bool::from(hidden)];
                }

                let () = msg_send![viewcontroller, setNeedsStatusBarAppearanceUpdate];
            }

            // Update the view's frame to account for the status bar change.
            let view: Id = msg_send![viewcontroller, view];
            let () = msg_send![view, setFrame: super::compute_view_frame(window, screen)];
        }

        // (SDL_IPHONE_KEYBOARD's updateKeyboard comes with the keyboard)

        let view: Id = msg_send![viewcontroller, view];
        let () = msg_send![view, setNeedsLayout];
        let () = msg_send![view, layoutIfNeeded];
    }
}

/// Translation of `UIKit_SetWindowBordered()`.
pub(crate) fn set_window_bordered(window: WindowID, bordered: bool) {
    autoreleasepool(|| {
        let _ = with_window(window, |w| {
            if bordered {
                w.core.flags &= !WindowFlags::BORDERLESS;
            } else {
                w.core.flags |= WindowFlags::BORDERLESS;
            }
        });
        update_window_border(window);
    });
}

/// Translation of `UIKit_SetWindowFullscreen()`.
pub(crate) fn set_window_fullscreen(
    window: WindowID,
    fullscreen: FullscreenOp,
) -> FullscreenResult {
    autoreleasepool(|| {
        send_window_event(
            window,
            if fullscreen != FullscreenOp::Leave {
                EventType::WINDOW_ENTER_FULLSCREEN
            } else {
                EventType::WINDOW_LEAVE_FULLSCREEN
            },
            0,
            0,
        );
        update_window_border(window);
    });
    FullscreenResult::Succeeded
}

/// Make a window's view controller tell UIKit again whether it wants the
/// pointer locked (the game controllers' mice's relative mode, iOS 14).
/// Translation of `UIKit_UpdatePointerLock()`.
pub(crate) fn update_pointer_lock(window: WindowID) {
    #[cfg(not(target_os = "tvos"))]
    autoreleasepool(|| {
        let Some(data) = window_data(window) else {
            return;
        };
        let viewcontroller = data.viewcontroller();
        if available!(ios(14, 0)) {
            // SAFETY: a UIViewController method (iOS 14).
            unsafe {
                let () = msg_send![viewcontroller, setNeedsUpdateOfPrefersPointerLocked];
            }
        }
    });
    #[cfg(target_os = "tvos")]
    let _ = window;
}

/// Translation of `UIKit_DestroyWindow()`.
pub(crate) fn destroy_window(window: WindowID) {
    autoreleasepool(|| {
        let Some(data) = window_data(window) else {
            return;
        };

        viewcontroller::stop_animation(data.viewcontroller());

        // (visionOS's curved window comes with visionOS)

        /* Detach all views from this window. We use a copy of the array
         * because setSDLWindow will remove the object from the original
         * array, which would be undesirable if we were iterating over it. */
        let views: Vec<StrongPtr> = data.views().clone();
        for v in &views {
            view::set_sdl_window(v.get(), None);
        }
        drop(views);

        /* iOS may still hold a reference to the window after we release it.
         * We want to make sure the SDL view controller isn't accessed in
         * that case, because it would contain an invalid pointer to the old
         * SDL window. */
        // SAFETY: UIWindow's properties.
        unsafe {
            let () = msg_send![data.uiwindow(), setRootViewController: NIL];
            let () = msg_send![data.uiwindow(), setHidden: crate::core::apple::objc::YES];
        }

        drop(data);
        let _ = with_window(window, |w| w.internal = None);
    });
}

/// Translation of `UIKit_GetWindowSizeInPixels()`.
pub(crate) fn get_window_size_in_pixels(window: WindowID) -> (i32, i32) {
    autoreleasepool(|| {
        let Some(windata) = window_data(window) else {
            return (0, 0);
        };
        // SAFETY: UIViewController's view, UIView's bounds and UIScreen's
        // native scale.
        unsafe {
            let view: Id = msg_send![windata.viewcontroller(), view];
            let bounds: crate::core::apple::foundation::CGRect = msg_send![view, bounds];
            let size = bounds.size;
            let mut scale = 1.0;

            if flags(window).contains(WindowFlags::HIGH_PIXEL_DENSITY) {
                #[cfg(not(target_os = "visionos"))]
                {
                    let screen: Id = msg_send![windata.uiwindow(), screen];
                    scale = msg_send![screen, nativeScale];
                }
                #[cfg(target_os = "visionos")]
                {
                    scale = 2.0;
                }
            }

            /* Integer truncation of fractional values matches SDL_uikitmetalview and
             * SDL_uikitopenglview. */
            ((size.width * scale) as i32, (size.height * scale) as i32)
        }
    })
}

/// The orientations a window supports: those of `SDL_HINT_ORIENTATIONS`,
/// else any for a resizable window, else those its floating shape suits;
/// never upside down on a phone, and only the app's. Translation of
/// `UIKit_GetSupportedOrientations()`.
#[cfg(not(target_os = "tvos"))]
pub(crate) fn get_supported_orientations(
    window: WindowID,
) -> super::sys::UIInterfaceOrientationMask {
    #[cfg(target_os = "visionos")]
    {
        let _ = window;
        super::sys::UIInterfaceOrientationMaskAll
    }
    #[cfg(not(target_os = "visionos"))]
    {
        let hint = crate::hints::get(crate::hints::ORIENTATIONS);
        let mut orientation_mask: NSUInteger = 0;

        autoreleasepool(|| {
            let uiwindow = window_data(window).map_or(NIL, |d| d.uiwindow());
            let app = crate::core::apple::uikit::shared_application();

            /* Get all possible valid orientations. If the app delegate doesn't tell
             * us, we get the orientations from Info.plist via UIApplication. */
            // SAFETY: UIApplication's delegate and methods.
            let valid_orientations: NSUInteger = unsafe {
                let delegate: Id = msg_send![app, delegate];
                if responds_to(
                    delegate,
                    sel!(application:supportedInterfaceOrientationsForWindow:),
                ) {
                    msg_send![delegate, application: app supportedInterfaceOrientationsForWindow: uiwindow]
                } else {
                    msg_send![app, supportedInterfaceOrientationsForWindow: uiwindow]
                }
            };

            if let Some(hint) = &hint {
                // (componentsSeparatedByString:@" "'s parts)
                let orientations: Vec<&str> = hint.split(' ').collect();

                if orientations.contains(&"LandscapeLeft") {
                    orientation_mask |= UIInterfaceOrientationMaskLandscapeLeft;
                }
                if orientations.contains(&"LandscapeRight") {
                    orientation_mask |= UIInterfaceOrientationMaskLandscapeRight;
                }
                if orientations.contains(&"Portrait") {
                    orientation_mask |= UIInterfaceOrientationMaskPortrait;
                }
                if orientations.contains(&"PortraitUpsideDown") {
                    orientation_mask |= UIInterfaceOrientationMaskPortraitUpsideDown;
                }
            }

            let (window_flags, floating) =
                with_window(window, |w| (w.flags(), w.core.floating)).unwrap_or_default();

            if orientation_mask == 0 && window_flags.contains(WindowFlags::RESIZABLE) {
                // any orientation is okay.
                orientation_mask = UIInterfaceOrientationMaskAll;
            }

            if orientation_mask == 0 {
                if floating.w >= floating.h {
                    orientation_mask |= UIInterfaceOrientationMaskLandscape;
                }
                if floating.h >= floating.w {
                    orientation_mask |= UIInterfaceOrientationMaskPortrait
                        | UIInterfaceOrientationMaskPortraitUpsideDown;
                }
            }

            // Don't allow upside-down orientation on phones, so answering calls is in the natural orientation
            if crate::core::apple::uikit::user_interface_idiom() == UIUserInterfaceIdiomPhone {
                orientation_mask &= !UIInterfaceOrientationMaskPortraitUpsideDown;
            }

            /* If none of the specified orientations are actually supported by the
             * app, we'll revert to what the app supports. An exception would be
             * thrown by the system otherwise. */
            if (valid_orientations & orientation_mask) == 0 {
                orientation_mask = valid_orientations;
            }
        });

        orientation_mask
    }
}

/// Translation of `SDL_SetiOSAnimationCallback()` (see
/// [`Window::set_ios_animation_callback`]).
pub(crate) fn set_ios_animation_callback(
    window: WindowID,
    interval: i32,
    callback: Option<Box<dyn FnMut()>>,
) -> Result<()> {
    let Some(data) = window_data(window) else {
        return Err(Error::new("Invalid window"));
    };

    autoreleasepool(|| {
        viewcontroller::set_animation_callback(data.viewcontroller(), interval, callback);
    });

    Ok(())
}
