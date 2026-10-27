// Rust translation of the lifecycle, event pump and mouse parts of
// src/video/uikit/SDL_uikitevents.m and SDL_uikitevents.h from Simple
// DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The app's lifecycle and the event pump: `SDL_LifecycleObserver`, which
//! turns UIApplication's notifications into the application events
//! (`SDL_OnApplication*()`, observed while the event pump is on, the main
//! callbacks run or a window exists), and the pump, which runs the main
//! run loop briefly ([`set_ios_event_pump`] turns it off); the events'
//! timestamps on SDL's clock ([`get_event_timestamp`]); and the game
//! controllers' mice (`GCMouse`, iOS 14.1): each connected one is a mouse
//! whose buttons and wheel send their events, and whose motion is the
//! relative mode's, which locks the pointer
//! ([`super::window::update_pointer_lock`]). The game controllers'
//! keyboards (`GCKeyboard`) come with the keyboard.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use crate::core::apple::block::{dispatch_get_global_queue, dispatch_queue_create, RcBlock};
use crate::core::apple::foundation::{
    available, cf_string, kCFRunLoopDefaultMode, ns_array_items, ns_string, CFRunLoopRunInMode,
    CFStringRef, NSTimeInterval, K_CF_RUN_LOOP_RUN_HANDLED_SOURCE,
};
use crate::core::apple::objc::{
    autoreleasepool, class, imp, msg_send, sel, Bool, Class, ClassBuilder, DeclaredClass, Id, Sel,
    StrongPtr, NIL,
};
use crate::events::mouse::{
    add_mouse, mouse_focus, remove_mouse, send_mouse_button, send_mouse_motion, send_mouse_wheel,
    MouseID, MouseWheelDirection, BUTTON_LEFT, BUTTON_MIDDLE, BUTTON_RIGHT, BUTTON_X1,
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

/// Translation of `timestamp_offset` (`UIKit_GetEventTimestamp()`'s).
static TIMESTAMP_OFFSET: AtomicU64 = AtomicU64::new(0);

/// A UIKit event's timestamp (seconds since the system started) on SDL's
/// clock, never later than now. Translation of `UIKit_GetEventTimestamp()`.
pub(crate) fn get_event_timestamp(ns_timestamp: NSTimeInterval) -> Duration {
    let mut timestamp = (ns_timestamp * 1_000_000_000.0) as u64;
    let now = crate::timer::ticks_ns();

    let mut timestamp_offset = TIMESTAMP_OFFSET.load(Ordering::Relaxed);
    if timestamp_offset == 0 {
        timestamp_offset = now.wrapping_sub(timestamp);
    }
    timestamp = timestamp.wrapping_add(timestamp_offset);

    if timestamp > now {
        timestamp_offset = timestamp_offset.wrapping_sub(timestamp - now);
        timestamp = now;
    }
    TIMESTAMP_OFFSET.store(timestamp_offset, Ordering::Relaxed);
    Duration::from_nanos(timestamp)
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

// ---------------------------------------------------------------------------
// GCMouse

/// The `GCMouseDidConnectNotification` and `GCMouseDidDisconnectNotification`
/// observers (`mouse_connect_observer`, `mouse_disconnect_observer`).
static MOUSE_OBSERVERS: Mutex<(StrongPtr, StrongPtr)> =
    Mutex::new((StrongPtr::nil(), StrongPtr::nil()));
/// Translation of `mouse_relative_mode`.
static MOUSE_RELATIVE_MODE: AtomicBool = AtomicBool::new(false);
/// Translation of `mouse_scroll_direction` (true: `SDL_MOUSEWHEEL_FLIPPED`).
static MOUSE_SCROLL_FLIPPED: AtomicBool = AtomicBool::new(false);
/// Whether `SDL_GetMouse()->SetRelativeMouseMode` is
/// [`set_gc_mouse_relative_mode`] (from [`init_gc_mouse`] to
/// [`quit_gc_mouse`]).
static RELATIVE_MODE_HOOKED: AtomicBool = AtomicBool::new(false);

/// Translation of `UpdateScrollDirection()`.
fn update_scroll_direction() {
    // (upstream's `#if 0` reads NSUserDefaults instead: "This code doesn't
    // work for some reason")
    let key = cf_string("com.apple.swipescrolldirection");
    let mut key_exists_and_has_valid_format = 0;
    // SAFETY: a CFString key, the constant application ID and an out
    // parameter.
    let mut natural_scroll_direction = unsafe {
        CFPreferencesGetAppBooleanValue(
            key.get() as CFStringRef,
            kCFPreferencesAnyApplication,
            &mut key_exists_and_has_valid_format,
        )
    } != 0;
    if key_exists_and_has_valid_format == 0 {
        // Couldn't read the preference, assume natural scrolling direction
        natural_scroll_direction = true;
    }
    MOUSE_SCROLL_FLIPPED.store(natural_scroll_direction, Ordering::Relaxed);
}

/// Translation of `UpdatePointerLock()`.
fn update_pointer_lock() {
    for window in crate::video::core::window_ids() {
        super::window::update_pointer_lock(window);
    }
}

/// Translation of `SetGCMouseRelativeMode()`.
pub(crate) fn set_gc_mouse_relative_mode(enabled: bool) -> crate::Result<()> {
    MOUSE_RELATIVE_MODE.store(enabled, Ordering::Relaxed);
    update_pointer_lock();
    Ok(())
}

/// Whether `SDL_GetMouse()->SetRelativeMouseMode` is
/// [`set_gc_mouse_relative_mode`]: from [`init_gc_mouse`] (when the app
/// asks for the pointer's events) to [`quit_gc_mouse`].
pub(crate) fn has_gc_mouse_relative_mode() -> bool {
    RELATIVE_MODE_HOOKED.load(Ordering::Relaxed)
}

/// Translation of `OnGCMouseButtonChanged()`.
fn on_gc_mouse_button_changed(mouse_id: MouseID, button: u8, pressed: bool) {
    let timestamp = crate::timer::ticks();

    // (visionOS's pointer mode and curved windows come with visionOS)
    send_mouse_button(timestamp, mouse_focus(), mouse_id, button, pressed);
}

/// `button.pressedChangedHandler = handler` (nil for `None`).
///
/// # Safety
///
/// `input` must be a `GCControllerButtonInput`.
unsafe fn set_pressed_changed_handler(input: Id, handler: Option<RcBlock>) {
    let block = handler
        .as_ref()
        .map_or(std::ptr::null_mut(), |b| b.as_ptr());
    // SAFETY: the caller's contract; the input copies the block.
    let () = unsafe { msg_send![input, setPressedChangedHandler: block] };
}

/// Translation of `OnGCMouseConnected()`.
fn on_gc_mouse_connected(mouse: Id) {
    let mouse_id = mouse as usize as MouseID;

    add_mouse(mouse_id, None);

    // `^(GCControllerButtonInput *button, float value, BOOL pressed) {...}`
    let button_handler = |button: u8| {
        RcBlock::new3(move |_button: Id, _value: f32, pressed: Bool| {
            on_gc_mouse_button_changed(mouse_id, button, pressed.is_true());
        })
    };

    // SAFETY: a GCMouse and its inputs; the handlers are blocks of the
    // documented signatures, which the inputs copy.
    unsafe {
        let mouse_input: Id = msg_send![mouse, mouseInput];
        let left: Id = msg_send![mouse_input, leftButton];
        set_pressed_changed_handler(left, Some(button_handler(BUTTON_LEFT)));
        let middle: Id = msg_send![mouse_input, middleButton];
        set_pressed_changed_handler(middle, Some(button_handler(BUTTON_MIDDLE)));
        let right: Id = msg_send![mouse_input, rightButton];
        set_pressed_changed_handler(right, Some(button_handler(BUTTON_RIGHT)));

        let mut auxiliary_button = BUTTON_X1;
        let auxiliary: Id = msg_send![mouse_input, auxiliaryButtons];
        for btn in ns_array_items(auxiliary) {
            set_pressed_changed_handler(btn, Some(button_handler(auxiliary_button)));
            auxiliary_button = auxiliary_button.wrapping_add(1);
        }

        // `^(GCMouseInput *mouseInput, float deltaX, float deltaY) {...}`
        let moved = RcBlock::new3(move |_mouse_input: Id, delta_x: f32, delta_y: f32| {
            let timestamp = crate::timer::ticks();

            let send_motion = gc_mouse_relative_mode();
            // (visionOS's pointer mode comes with visionOS)
            if send_motion {
                send_mouse_motion(timestamp, mouse_focus(), mouse_id, true, delta_x, -delta_y);
            }
        });
        let () = msg_send![mouse_input, setMouseMovedHandler: moved.as_ptr()];

        // `^(GCControllerDirectionPad *dpad, float xValue, float yValue) {...}`
        let scrolled = RcBlock::new3(move |_dpad: Id, x_value: f32, y_value: f32| {
            let timestamp = crate::timer::ticks();

            #[cfg(target_os = "visionos")]
            let (mut vertical, mut horizontal) = {
                /* Mouse scroll values on visionOS have swapped axes compared to other platforms.
                 * There is also an acceleration ramp applied, so clamp to a single tick per event.
                 */
                let tick = |v: f32| {
                    if v < 0.0 {
                        -1.0
                    } else if v > 0.0 {
                        1.0
                    } else {
                        0.0
                    }
                };
                (tick(y_value), tick(x_value))
            };
            #[cfg(not(target_os = "visionos"))]
            let (mut vertical, mut horizontal) = {
                /* Raw scroll values come in here, vertical values in the first axis, horizontal values in the second axis.
                 * The vertical values are negative moving the mouse wheel up and positive moving it down.
                 * The horizontal values are negative moving the mouse wheel left and positive moving it right.
                 * The vertical values are inverted compared to SDL, and the horizontal values are as expected.
                 */
                (-x_value, y_value)
            };

            let flipped = MOUSE_SCROLL_FLIPPED.load(Ordering::Relaxed);
            if flipped {
                // Since these are raw values, we need to flip them ourselves
                vertical = -vertical;
                horizontal = -horizontal;
            }
            let direction = if flipped {
                MouseWheelDirection::Flipped
            } else {
                MouseWheelDirection::Normal
            };
            send_mouse_wheel(
                timestamp,
                mouse_focus(),
                mouse_id,
                horizontal,
                vertical,
                direction,
            );
        });
        let scroll: Id = msg_send![mouse_input, scroll];
        let () = msg_send![scroll, setValueChangedHandler: scrolled.as_ptr()];
        update_scroll_direction();

        #[link(name = "System", kind = "dylib")]
        unsafe extern "C" {
            fn dispatch_set_target_queue(
                object: *mut std::ffi::c_void,
                queue: *mut std::ffi::c_void,
            );
        }
        /// `DISPATCH_QUEUE_PRIORITY_HIGH`.
        const DISPATCH_QUEUE_PRIORITY_HIGH: isize = 2;
        let queue = dispatch_queue_create(
            c"org.libsdl.input.mouse".as_ptr(),
            crate::core::apple::block::DISPATCH_QUEUE_SERIAL,
        );
        dispatch_set_target_queue(
            queue,
            dispatch_get_global_queue(DISPATCH_QUEUE_PRIORITY_HIGH, 0),
        );
        let () = msg_send![mouse, setHandlerQueue: queue];
        // (the mouse keeps its queue)
        crate::core::apple::block::dispatch_release(queue);
    }

    update_pointer_lock();
}

/// Translation of `OnGCMouseDisconnected()`.
fn on_gc_mouse_disconnected(mouse: Id) {
    let mouse_id = mouse as usize as MouseID;

    // SAFETY: a GCMouse and its inputs.
    unsafe {
        let mouse_input: Id = msg_send![mouse, mouseInput];
        let () =
            msg_send![mouse_input, setMouseMovedHandler: std::ptr::null_mut::<std::ffi::c_void>()];

        let left: Id = msg_send![mouse_input, leftButton];
        set_pressed_changed_handler(left, None);
        let middle: Id = msg_send![mouse_input, middleButton];
        set_pressed_changed_handler(middle, None);
        let right: Id = msg_send![mouse_input, rightButton];
        set_pressed_changed_handler(right, None);

        let auxiliary: Id = msg_send![mouse_input, auxiliaryButtons];
        for button in ns_array_items(auxiliary) {
            set_pressed_changed_handler(button, None);
        }
        // (upstream leaves the scroll wheel's handler)
    }

    update_pointer_lock();

    remove_mouse(mouse_id);
}

/// Start following the game controllers' mice (from iOS 14.1, in an app
/// whose Info.plist has `UIApplicationSupportsIndirectInputEvents`),
/// adding those connected, and make the relative mode theirs. Translation
/// of `SDL_InitGCMouse()`.
pub(crate) fn init_gc_mouse() {
    autoreleasepool(|| {
        // There is a bug where mouse accumulates duplicate deltas over time in iOS 14.0
        if !available!(ios(14, 1), tvos(14, 1)) {
            return;
        }
        // (GameController is loaded at run time here: without it there's no
        // mouse to follow)
        let connect_name = GCMouseDidConnectNotification();
        let disconnect_name = GCMouseDidDisconnectNotification();
        if connect_name.is_null() || disconnect_name.is_null() || class!(GCMouse).is_null() {
            return;
        }
        // SAFETY: the main bundle's Info.plist (a dictionary), the
        // notification center with notification blocks of the documented
        // signature, and GCMouse's class method.
        unsafe {
            /* iOS will not send the new pointer touch events if you don't have this key,
             * and we need them to differentiate between mouse events and real touch events.
             */
            let bundle: Id = msg_send![class!(NSBundle), mainBundle];
            let info: Id = msg_send![bundle, infoDictionary];
            let key = ns_string("UIApplicationSupportsIndirectInputEvents");
            let value: Id = msg_send![info, objectForKey: key.get()];
            // ([nil boolValue] is NO)
            let indirect_input_available = !value.is_null() && {
                let b: Bool = msg_send![value, boolValue];
                b.is_true()
            };
            if indirect_input_available {
                let center: Id = msg_send![class!(NSNotificationCenter), defaultCenter];

                let connect = RcBlock::new1(|note: Id| {
                    // (a notification whose object is the GCMouse)
                    let mouse: Id = msg_send![note, object];
                    on_gc_mouse_connected(mouse);
                });
                let connect_observer: Id = msg_send![center,
                    addObserverForName: connect_name
                    object: NIL
                    queue: NIL
                    usingBlock: connect.as_ptr()];

                let disconnect = RcBlock::new1(|note: Id| {
                    // (a notification whose object is the GCMouse)
                    let mouse: Id = msg_send![note, object];
                    on_gc_mouse_disconnected(mouse);
                });
                let disconnect_observer: Id = msg_send![center,
                    addObserverForName: disconnect_name
                    object: NIL
                    queue: NIL
                    usingBlock: disconnect.as_ptr()];

                *MOUSE_OBSERVERS.lock().unwrap_or_else(|e| e.into_inner()) = (
                    StrongPtr::retain(connect_observer),
                    StrongPtr::retain(disconnect_observer),
                );

                let mice: Id = msg_send![class!(GCMouse), mice];
                for mouse in ns_array_items(mice) {
                    on_gc_mouse_connected(mouse);
                }

                RELATIVE_MODE_HOOKED.store(true, Ordering::Relaxed);
            } else {
                crate::log::ns_log(
                    "",
                    "You need UIApplicationSupportsIndirectInputEvents in your Info.plist for mouse support",
                );
            }
        }
    })
}

/// Whether the relative mode is on. Translation of
/// `SDL_GCMouseRelativeMode()`.
pub(crate) fn gc_mouse_relative_mode() -> bool {
    MOUSE_RELATIVE_MODE.load(Ordering::Relaxed)
}

/// Stop following the game controllers' mice, removing those connected.
/// Translation of `SDL_QuitGCMouse()`.
pub(crate) fn quit_gc_mouse() {
    autoreleasepool(|| {
        if !available!(ios(14, 1), tvos(14, 1)) {
            return;
        }
        // SAFETY: the observers init_gc_mouse() registered, the
        // notification center and GCMouse's class method.
        unsafe {
            let center: Id = msg_send![class!(NSNotificationCenter), defaultCenter];
            let (connect, disconnect) =
                std::mem::take(&mut *MOUSE_OBSERVERS.lock().unwrap_or_else(|e| e.into_inner()));

            if !connect.is_nil() {
                let () = msg_send![center, removeObserver: connect.get()
                                                     name: GCMouseDidConnectNotification()
                                                   object: NIL];
            }

            if !disconnect.is_nil() {
                let () = msg_send![center, removeObserver: disconnect.get()
                                                     name: GCMouseDidDisconnectNotification()
                                                   object: NIL];
            }

            // (no GCMouse class without GameController)
            if !class!(GCMouse).is_null() {
                let mice: Id = msg_send![class!(GCMouse), mice];
                for mouse in ns_array_items(mice) {
                    on_gc_mouse_disconnected(mouse);
                }
            }
        }

        RELATIVE_MODE_HOOKED.store(false, Ordering::Relaxed);
    })
}
