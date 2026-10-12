// Rust translation of src/video/cocoa/SDL_cocoamessagebox.m and
// SDL_cocoamessagebox.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Message boxes, as `NSAlert`s: run modally, or as a sheet on the parent
//! window, always on the main thread.

use super::events::{ns_app, register_app};
use super::sys::*;
use crate::core::apple::block::{run_on_main_sync, RcBlock};
use crate::core::apple::foundation::{is_main_thread, ns_string};
use crate::core::apple::objc::{autoreleasepool, class, msg_send, Id, NSInteger, StrongPtr};
use crate::error::{Error, Result};
use crate::video::messagebox::{MessageBoxButtonFlags, MessageBoxData, MessageBoxFlags};

/// Translation of `-[SDL3MessageBoxPresenter showAlert:]` (the presenter's
/// `nswindow` is the parent window, retained while the alert shows).
fn show_alert(alert: Id, nswindow: Option<StrongPtr>) -> NSInteger {
    // SAFETY: an NSAlert, NSApp and (if any) the parent NSWindow.
    unsafe {
        if let Some(nswindow) = nswindow {
            let handler = RcBlock::new1(|return_code: NSInteger| {
                // (NSApp)
                let _: () = msg_send![ns_app(), stopModalWithCode: return_code];
            });
            let _: () = msg_send![alert, beginSheetModalForWindow: nswindow.get()
                                               completionHandler: handler.as_ptr()];
            msg_send![ns_app(), runModalForWindow: nswindow.get()]
        } else {
            msg_send![alert, runModal]
        }
    }
}

/// Translation of `Cocoa_ShowMessageBoxImpl()`.
fn show_message_box_impl(messageboxdata: &MessageBoxData) -> Result<i32> {
    let buttons = &messageboxdata.buttons;
    register_app();

    // SAFETY: NSAlert and its buttons, as documented.
    unsafe {
        let alert = StrongPtr::from_owned(msg_send![super::alloc(class!(NSAlert)), init]);

        let style = if messageboxdata.flags.contains(MessageBoxFlags::ERROR) {
            NSAlertStyleCritical
        } else if messageboxdata.flags.contains(MessageBoxFlags::WARNING) {
            NSAlertStyleWarning
        } else {
            NSAlertStyleInformational
        };
        let _: () = msg_send![alert, setAlertStyle: style];

        let title = ns_string(&messageboxdata.title);
        let _: () = msg_send![alert, setMessageText: title.get()];
        let message = ns_string(&messageboxdata.message);
        let _: () = msg_send![alert, setInformativeText: message.get()];

        let right_to_left = messageboxdata
            .flags
            .contains(MessageBoxFlags::BUTTONS_RIGHT_TO_LEFT);
        let numbuttons = buttons.len();
        for i in 0..numbuttons {
            let sdl_button = if right_to_left {
                &buttons[numbuttons - 1 - i]
            } else {
                &buttons[i]
            };

            let text = ns_string(&sdl_button.text);
            let button: Id = msg_send![alert, addButtonWithTitle: text.get()];
            let key = if sdl_button
                .flags
                .contains(MessageBoxButtonFlags::RETURNKEY_DEFAULT)
            {
                "\r"
            } else if sdl_button
                .flags
                .contains(MessageBoxButtonFlags::ESCAPEKEY_DEFAULT)
            {
                "\u{1b}"
            } else {
                ""
            };
            let key = ns_string(key);
            let _: () = msg_send![button, setKeyEquivalent: key.get()];
        }

        // Retain the NSWindow because we'll show the alert later on the main thread
        let nswindow = messageboxdata
            .window
            .as_ref()
            .and_then(|w| super::window::window_data(w.id()))
            .map(|d| d.nswindow.clone());

        let mut clicked = show_alert(alert.get(), nswindow);

        if clicked >= NSAlertFirstButtonReturn {
            clicked -= NSAlertFirstButtonReturn;
            if right_to_left {
                clicked = numbuttons as NSInteger - 1 - clicked;
            }
            match buttons.get(clicked as usize) {
                Some(b) => Ok(b.button_id),
                None => Err(Error::new(format!(
                    "Did not get a valid `clicked button' id: {clicked}"
                ))),
            }
        } else {
            Err(Error::new(format!(
                "Did not get a valid `clicked button' id: {clicked}"
            )))
        }
    }
}

/// Display a Cocoa message box. Translation of `Cocoa_ShowMessageBox()`.
pub(crate) fn show_message_box(messageboxdata: &MessageBoxData) -> Result<i32> {
    autoreleasepool(|| {
        if is_main_thread() {
            show_message_box_impl(messageboxdata)
        } else {
            let data = messageboxdata.clone();
            run_on_main_sync(move || autoreleasepool(|| show_message_box_impl(&data)))
        }
    })
}
