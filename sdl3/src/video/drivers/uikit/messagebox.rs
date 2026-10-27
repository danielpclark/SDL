// Rust translation of src/video/uikit/SDL_uikitmessagebox.m and
// SDL_uikitmessagebox.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Message boxes, as `UIAlertController`s: presented on the window's root
//! view controller (or on a window of their own, at the alerts' level),
//! on the main thread, which runs its run loop until a button is chosen.
//! While one is up the windows' animation callbacks don't run
//! ([`showing_message_box`]).

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use crate::core::apple::block::RcBlock;
use crate::core::apple::foundation::{available, is_main_thread, ns_string};
use crate::core::apple::objc::{autoreleasepool, class, msg_send, Id, StrongPtr, NIL, YES};
use crate::core::apple::uikit::shared_application;
use crate::error::{Error, Result};
use crate::video::messagebox::{MessageBoxButtonFlags, MessageBoxData, MessageBoxFlags};

use super::sys::{
    NSDefaultRunLoopMode, UIAlertActionStyleCancel, UIAlertActionStyleDefault,
    UIAlertControllerStyleAlert, UIWindowLevelAlert,
};

// Display a UIKit message box

/// Translation of `s_showingMessageBox`.
static SHOWING_MESSAGE_BOX: AtomicBool = AtomicBool::new(false);

/// Whether a message box is up. Translation of `UIKit_ShowingMessageBox()`.
pub(crate) fn showing_message_box() -> bool {
    SHOWING_MESSAGE_BOX.load(Ordering::SeqCst)
}

/// Translation of `UIKit_WaitUntilMessageBoxClosed()` (the index of the
/// button clicked is `numbuttons` until one is).
fn wait_until_message_box_closed(numbuttons: usize, clickedindex: &AtomicUsize) {
    autoreleasepool(|| {
        // Run the main event loop until the alert has finished
        // Note that this needs to be done on the main thread
        SHOWING_MESSAGE_BOX.store(true, Ordering::SeqCst);
        while clickedindex.load(Ordering::SeqCst) == numbuttons {
            // SAFETY: the current NSRunLoop, run in the default mode until
            // a source has been handled; NSDate's class property.
            unsafe {
                let run_loop: Id = msg_send![class!(NSRunLoop), currentRunLoop];
                let date: Id = msg_send![class!(NSDate), distantFuture];
                let _: crate::core::apple::objc::Bool =
                    msg_send![run_loop, runMode: NSDefaultRunLoopMode beforeDate: date];
            }
        }
        SHOWING_MESSAGE_BOX.store(false, Ordering::SeqCst);
    })
}

/// Translation of `UIKit_ShowMessageBoxAlertController()`: the ID of the
/// button chosen, or `None` if there's no `UIAlertController`.
fn show_message_box_alert_controller(messageboxdata: &MessageBoxData) -> Option<i32> {
    let buttons = &messageboxdata.buttons;
    let numbuttons = buttons.len();
    let clickedindex = Arc::new(AtomicUsize::new(numbuttons));
    let mut window: Id = NIL;
    let mut alertwindow = StrongPtr::nil();

    if class!(UIAlertController).is_null() {
        return None;
    }

    // SAFETY: UIAlertController's and UIAlertAction's class methods
    // (autoreleased) and properties, with NSStrings and blocks of the
    // documented signature; UIWindow's and UIViewController's
    // initializers (owned) and properties.
    unsafe {
        let title = ns_string(&messageboxdata.title);
        let message = ns_string(&messageboxdata.message);
        let alert: Id = msg_send![class!(UIAlertController),
            alertControllerWithTitle: title.get()
            message: message.get()
            preferredStyle: UIAlertControllerStyleAlert];

        // FIXME (upstream): UIAlertController raises an Objective-C
        // exception for a second action of the cancel style, so a message
        // box with two escape key default buttons ends the app (as
        // upstream's: the exception isn't caught).
        for i in 0..numbuttons {
            let mut style = UIAlertActionStyleDefault;

            let index = if messageboxdata
                .flags
                .contains(MessageBoxFlags::BUTTONS_RIGHT_TO_LEFT)
            {
                numbuttons - 1 - i
            } else {
                i
            };
            let sdl_button = &buttons[index];

            if sdl_button
                .flags
                .contains(MessageBoxButtonFlags::ESCAPEKEY_DEFAULT)
            {
                style = UIAlertActionStyleCancel;
            }

            let clicked = Arc::clone(&clickedindex);
            let handler = RcBlock::new1(move |_alert_action: Id| {
                clicked.store(index, Ordering::SeqCst);
            });
            let text = ns_string(&sdl_button.text);
            let action: Id = msg_send![class!(UIAlertAction),
                actionWithTitle: text.get()
                style: style
                handler: handler.as_ptr()];
            let () = msg_send![alert, addAction: action];

            if sdl_button
                .flags
                .contains(MessageBoxButtonFlags::RETURNKEY_DEFAULT)
            {
                let () = msg_send![alert, setPreferredAction: action];
            }
        }

        if let Some(data) = messageboxdata
            .window
            .as_ref()
            .and_then(|w| super::window::window_data(w.id()))
        {
            window = data.uiwindow();
        }

        let root: Id = if window.is_null() {
            NIL
        } else {
            msg_send![window, rootViewController]
        };
        if window.is_null() || root.is_null() {
            if available!(ios(13, 0), tvos(13, 0)) {
                let scene = super::get_active_window_scene();
                if !scene.is_null() {
                    let w: Id = msg_send![class!(UIWindow), alloc];
                    alertwindow = StrongPtr::from_owned(msg_send![w, initWithWindowScene: scene]);
                }
            }
            if alertwindow.is_nil() {
                #[cfg(target_os = "visionos")]
                {
                    let w: Id = msg_send![class!(UIWindow), alloc];
                    alertwindow = StrongPtr::from_owned(msg_send![w, init]);
                }
                #[cfg(not(target_os = "visionos"))]
                {
                    let screen: Id = msg_send![class!(UIScreen), mainScreen];
                    let bounds: crate::core::apple::foundation::CGRect = msg_send![screen, bounds];
                    let w: Id = msg_send![class!(UIWindow), alloc];
                    alertwindow = StrongPtr::from_owned(msg_send![w, initWithFrame: bounds]);
                }
            }
            let root = StrongPtr::from_owned(msg_send![class!(UIViewController), new]);
            let () = msg_send![alertwindow.get(), setRootViewController: root.get()];
            let () = msg_send![alertwindow.get(), setWindowLevel: UIWindowLevelAlert];

            window = alertwindow.get();

            let () = msg_send![alertwindow.get(), makeKeyAndVisible];
        }

        let root: Id = msg_send![window, rootViewController];
        let () = msg_send![root, presentViewController: alert
                                              animated: YES
                                            completion: NIL];
    }
    wait_until_message_box_closed(numbuttons, &clickedindex);

    if !alertwindow.is_nil() {
        // SAFETY: a UIView property.
        unsafe {
            let () = msg_send![alertwindow.get(), setHidden: YES];
        }
    }

    super::force_update_home_indicator();

    // (the wait ends only once a button is chosen: a message box without
    // buttons never closes, as upstream's, which would then read past
    // them)
    buttons
        .get(clickedindex.load(Ordering::SeqCst))
        .map(|b| b.button_id)
}

/// Display a UIKit message box: the ID of the button chosen. Translation
/// of `UIKit_ShowMessageBox()` (the bootstrap's `ShowMessageBox`), which
/// shows it on the main thread (`SDL_RunOnMainThread()`, waiting).
pub(crate) fn show_message_box(messageboxdata: &MessageBoxData) -> Result<i32> {
    let result: Arc<Mutex<Option<i32>>> = Arc::new(Mutex::new(None));
    let data = messageboxdata.clone();
    let callback_result = Arc::clone(&result);
    // Translation of `UIKit_ShowMessageBoxMainThreadCallback()`.
    crate::events::run_on_main_thread(
        move || {
            autoreleasepool(|| {
                // Note: upstream always runs in UIApplicationMain(), on the
                // main thread (SDL's main thread is the app's there); with
                // no app (a program run as a plain process of a simulated
                // device), or on another thread, there's no UIKit to show
                // the alert, as the driver doesn't start then either.
                if !is_main_thread() || shared_application().is_null() {
                    return;
                }
                *callback_result.lock().unwrap_or_else(|e| e.into_inner()) =
                    show_message_box_alert_controller(&data);
            })
        },
        true,
    )?;
    let result = *result.lock().unwrap_or_else(|e| e.into_inner());
    result.ok_or_else(|| Error::new("Could not show message box."))
}
