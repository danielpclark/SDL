// Rust translation of the lifecycle and event pump parts of
// src/video/uikit/SDL_uikitevents.m and SDL_uikitevents.h from Simple
// DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The app's lifecycle and the event pump: `SDL_LifecycleObserver`, which
//! turns UIApplication's notifications into the application events
//! (`SDL_OnApplication*()`, observed while the event pump is on, the main
//! callbacks run or a window exists), and the pump, which runs the main
//! run loop briefly ([`set_ios_event_pump`] turns it off). The game
//! controllers' keyboards and mice (`GCKeyboard`, `GCMouse`) aren't
//! translated yet.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use crate::core::apple::foundation::{
    kCFRunLoopDefaultMode, CFRunLoopRunInMode, CFStringRef, K_CF_RUN_LOOP_RUN_HANDLED_SOURCE,
};
use crate::core::apple::objc::{
    class, imp, msg_send, sel, Class, ClassBuilder, DeclaredClass, Id, Sel, StrongPtr, NIL,
};

use super::sys::*;

/// Translation of `UIKit_EventPumpEnabled`.
static UIKIT_EVENT_PUMP_ENABLED: AtomicBool = AtomicBool::new(true);

// ---------------------------------------------------------------------------
// SDL_LifecycleObserver

static SDL_LIFECYCLE_OBSERVER: DeclaredClass = DeclaredClass::new();

/// The `SDL_LifecycleObserver` class (`@interface SDL_LifecycleObserver :
/// NSObject`). Its `isObservingNotifications` property is
/// [`IS_OBSERVING_NOTIFICATIONS`]: there's one observer.
fn lifecycle_observer_class() -> Class {
    SDL_LIFECYCLE_OBSERVER.get(|| {
        let mut b = ClassBuilder::new(c"SDL_LifecycleObserver", class!(NSObject))
            .expect("SDL_LifecycleObserver");
        // SAFETY: the implementations have the signatures of the encodings.
        unsafe {
            b.add_method(
                sel!(applicationDidBecomeActive),
                imp!(application_did_become_active as extern "C" fn(Id, Sel)),
                c"v@:",
            );
            b.add_method(
                sel!(applicationWillResignActive),
                imp!(application_will_resign_active as extern "C" fn(Id, Sel)),
                c"v@:",
            );
            b.add_method(
                sel!(applicationDidEnterBackground),
                imp!(application_did_enter_background as extern "C" fn(Id, Sel)),
                c"v@:",
            );
            b.add_method(
                sel!(applicationWillEnterForeground),
                imp!(application_will_enter_foreground as extern "C" fn(Id, Sel)),
                c"v@:",
            );
            b.add_method(
                sel!(applicationWillTerminate),
                imp!(application_will_terminate as extern "C" fn(Id, Sel)),
                c"v@:",
            );
            b.add_method(
                sel!(applicationDidReceiveMemoryWarning),
                imp!(application_did_receive_memory_warning as extern "C" fn(Id, Sel)),
                c"v@:",
            );
            #[cfg(not(any(target_os = "tvos", target_os = "visionos")))]
            b.add_method(
                sel!(applicationDidChangeStatusBarOrientation),
                imp!(application_did_change_status_bar_orientation as extern "C" fn(Id, Sel)),
                c"v@:",
            );
        }
        b.register()
    })
}

/// `isObservingNotifications` of the observer.
static IS_OBSERVING_NOTIFICATIONS: AtomicBool = AtomicBool::new(false);

/// Translation of `-[SDL_LifecycleObserver update]`.
fn update(this: Id) {
    let mut wants_observation =
        UIKIT_EVENT_PUMP_ENABLED.load(Ordering::Relaxed) || crate::app::has_main_callbacks();
    if !wants_observation {
        // Make sure no windows have active animation callbacks
        if crate::video::core::with_device(|v| !v.windows.is_empty()).unwrap_or(false) {
            wants_observation = true;
        }
    }
    let observing = IS_OBSERVING_NOTIFICATIONS.load(Ordering::Relaxed);
    // SAFETY: the default notification center, and this observer's
    // methods, which take no argument.
    unsafe {
        let notification_center: Id = msg_send![class!(NSNotificationCenter), defaultCenter];
        if wants_observation && !observing {
            IS_OBSERVING_NOTIFICATIONS.store(true, Ordering::Relaxed);
            let observe = |selector: Sel, name: Id| {
                let () = msg_send![notification_center, addObserver: this
                                                              selector: selector
                                                                  name: name
                                                                object: NIL];
            };
            observe(
                sel!(applicationDidBecomeActive),
                UIApplicationDidBecomeActiveNotification,
            );
            observe(
                sel!(applicationWillResignActive),
                UIApplicationWillResignActiveNotification,
            );
            observe(
                sel!(applicationDidEnterBackground),
                UIApplicationDidEnterBackgroundNotification,
            );
            observe(
                sel!(applicationWillEnterForeground),
                UIApplicationWillEnterForegroundNotification,
            );
            observe(
                sel!(applicationWillTerminate),
                UIApplicationWillTerminateNotification,
            );
            observe(
                sel!(applicationDidReceiveMemoryWarning),
                UIApplicationDidReceiveMemoryWarningNotification,
            );
            #[cfg(not(any(target_os = "tvos", target_os = "visionos")))]
            observe(
                sel!(applicationDidChangeStatusBarOrientation),
                UIApplicationDidChangeStatusBarOrientationNotification,
            );
        } else if !wants_observation && observing {
            IS_OBSERVING_NOTIFICATIONS.store(false, Ordering::Relaxed);
            let () = msg_send![notification_center, removeObserver: this];
        }
    }
}

/// Translation of `-[SDL_LifecycleObserver applicationDidBecomeActive]`.
extern "C" fn application_did_become_active(_this: Id, _sel: Sel) {
    crate::video::on_application_did_enter_foreground();
}

/// Translation of `-[SDL_LifecycleObserver applicationWillResignActive]`.
extern "C" fn application_will_resign_active(_this: Id, _sel: Sel) {
    crate::video::on_application_will_enter_background();
}

/// Translation of `-[SDL_LifecycleObserver applicationDidEnterBackground]`.
extern "C" fn application_did_enter_background(_this: Id, _sel: Sel) {
    crate::video::on_application_did_enter_background();
}

/// Translation of `-[SDL_LifecycleObserver applicationWillEnterForeground]`.
extern "C" fn application_will_enter_foreground(_this: Id, _sel: Sel) {
    crate::video::on_application_will_enter_foreground();
}

/// Translation of `-[SDL_LifecycleObserver applicationWillTerminate]`.
extern "C" fn application_will_terminate(_this: Id, _sel: Sel) {
    crate::video::on_application_will_terminate();
}

/// Translation of `-[SDL_LifecycleObserver applicationDidReceiveMemoryWarning]`.
extern "C" fn application_did_receive_memory_warning(_this: Id, _sel: Sel) {
    crate::video::on_application_did_receive_memory_warning();
}

/// Translation of
/// `-[SDL_LifecycleObserver applicationDidChangeStatusBarOrientation]`.
#[cfg(not(any(target_os = "tvos", target_os = "visionos")))]
extern "C" fn application_did_change_status_bar_orientation(_this: Id, _sel: Sel) {
    let _ = crate::video::on_application_did_change_status_bar_orientation();
}

/// Start or stop observing the lifecycle notifications, as the event pump,
/// the main callbacks and the windows need. Translation of
/// `SDL_UpdateLifecycleObserver()`.
pub(crate) fn update_lifecycle_observer() {
    static LIFECYCLE_OBSERVER: OnceLock<StrongPtr> = OnceLock::new();
    let observer = LIFECYCLE_OBSERVER.get_or_init(|| {
        // SAFETY: `new` answers an owned instance.
        unsafe { StrongPtr::from_owned(msg_send![lifecycle_observer_class(), new]) }
    });
    update(observer.get());
}

/// Turn SDL's event pump on or off on iOS: off, pumping events doesn't
/// run the main run loop, for apps that run it themselves. It's on while
/// the app's main function runs (and off after it returns). Translation
/// of `SDL_SetiOSEventPump()`.
///
/// With the main callbacks, SDL manages this itself. This should only be
/// called on the main thread.
pub fn set_ios_event_pump(enabled: bool) {
    UIKIT_EVENT_PUMP_ENABLED.store(enabled, Ordering::Relaxed);

    update_lifecycle_observer();
}

/// Translation of `UIKit_PumpEvents()`.
pub(crate) fn pump_events() {
    if !UIKIT_EVENT_PUMP_ENABLED.load(Ordering::Relaxed) {
        return;
    }

    /* Let the run loop run for a short amount of time: long enough for
       touch events to get processed (which is important to get certain
       elements of Game Center's GKLeaderboardViewController to respond
       to touch input), but not long enough to introduce a significant
       delay in the rest of the app.
    */
    let seconds = 0.000002;

    // SAFETY: runs the main thread's run loop (the pump runs there) in the
    // default and the tracking modes, briefly.
    unsafe {
        // Pump most event types.
        loop {
            let result = CFRunLoopRunInMode(kCFRunLoopDefaultMode, seconds, 1);
            if result != K_CF_RUN_LOOP_RUN_HANDLED_SOURCE {
                break;
            }
        }

        // Make sure UIScrollView objects scroll properly.
        loop {
            let result = CFRunLoopRunInMode(UITrackingRunLoopMode as CFStringRef, seconds, 1);
            if result != K_CF_RUN_LOOP_RUN_HANDLED_SOURCE {
                break;
            }
        }
    }

    // (UIKit_GL_RestoreCurrentContext() comes with OpenGL ES, which isn't
    // translated yet)
}
