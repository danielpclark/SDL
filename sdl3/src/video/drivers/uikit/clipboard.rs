// Rust translation of src/video/uikit/SDL_uikitclipboard.m and
// SDL_uikitclipboard.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The clipboard: the text of the general `UIPasteboard`, set (it outlives
//! the app), read and tested directly (the clipboard front end offers the
//! app's other data only to itself), and the pasteboard's changes
//! (`UIPasteboardChangedNotification`) made other than through
//! [`set_clipboard_text`], sent as clipboard updates that the app doesn't
//! own. tvOS has no pasteboard.

#[cfg(not(target_os = "tvos"))]
use std::sync::atomic::Ordering;
use std::sync::Arc;

#[cfg(not(target_os = "tvos"))]
use crate::core::apple::block::RcBlock;
#[cfg(not(target_os = "tvos"))]
use crate::core::apple::foundation::{ns_string, string_from_ns};
use crate::core::apple::objc::{autoreleasepool, class, msg_send, Id};
#[cfg(not(target_os = "tvos"))]
use crate::core::apple::objc::{Bool, StrongPtr, NIL};
#[cfg(target_os = "tvos")]
use crate::error::Error;
use crate::error::Result;
#[cfg(not(target_os = "tvos"))]
use crate::events::window::send_clipboard_update;

use super::VideoData;

/// `[UIPasteboard generalPasteboard]`.
#[cfg(not(target_os = "tvos"))]
fn general_pasteboard() -> Id {
    // SAFETY: UIPasteboard's class property.
    unsafe { msg_send![class!(UIPasteboard), generalPasteboard] }
}

/// Put text into the pasteboard (an empty one empties it). Translation of
/// `UIKit_SetClipboardText()`.
pub(crate) fn set_clipboard_text(data: &VideoData, text: &str) -> Result<()> {
    #[cfg(target_os = "tvos")]
    {
        let _ = (data, text);
        Err(Error::new("The clipboard is not available on tvOS"))
    }
    #[cfg(not(target_os = "tvos"))]
    autoreleasepool(|| {
        // (upstream's text is a C string: it ends at a NUL)
        let text = text.split('\0').next().unwrap_or_default();
        data.setting_clipboard.store(true, Ordering::SeqCst);
        // SAFETY: a UIPasteboard property, an NSString (or nil).
        unsafe {
            if !text.is_empty() {
                let string = ns_string(text);
                let () = msg_send![general_pasteboard(), setString: string.get()];
            } else {
                let () = msg_send![general_pasteboard(), setString: NIL];
            }
        }
        data.setting_clipboard.store(false, Ordering::SeqCst);
        Ok(())
    })
}

/// The pasteboard's text (empty if it has none). Translation of
/// `UIKit_GetClipboardText()`.
pub(crate) fn get_clipboard_text() -> String {
    #[cfg(target_os = "tvos")]
    {
        String::new() // Unsupported.
    }
    #[cfg(not(target_os = "tvos"))]
    autoreleasepool(|| {
        let pasteboard = general_pasteboard();
        // SAFETY: a UIPasteboard property, an NSString (or nil).
        unsafe {
            let string: Id = msg_send![pasteboard, string];

            string_from_ns(string).unwrap_or_default()
        }
    })
}

/// Whether the pasteboard has text. Translation of
/// `UIKit_HasClipboardText()`.
pub(crate) fn has_clipboard_text() -> bool {
    autoreleasepool(|| {
        #[cfg(not(target_os = "tvos"))]
        {
            // SAFETY: a UIPasteboard property.
            let has: Bool = unsafe { msg_send![general_pasteboard(), hasStrings] };
            if has.is_true() {
                return true;
            }
        }
        false
    })
}

/// Start sending the pasteboard's changes as clipboard updates.
/// Translation of `UIKit_InitClipboard()`.
pub(crate) fn init_clipboard(data: &Arc<VideoData>) {
    #[cfg(not(target_os = "tvos"))]
    autoreleasepool(|| {
        // SAFETY: the notification center, with a notification block of
        // the documented signature.
        unsafe {
            let center: Id = msg_send![class!(NSNotificationCenter), defaultCenter];

            let block_data = Arc::clone(data);
            let block = RcBlock::new1(move |_note: Id| {
                if !block_data.setting_clipboard.load(Ordering::SeqCst) {
                    // TODO: compute mime types
                    send_clipboard_update(false, Vec::new());
                }
            });
            let observer: Id = msg_send![center,
                addObserverForName: super::sys::UIPasteboardChangedNotification
                object: NIL
                queue: NIL
                usingBlock: block.as_ptr()];

            // (upstream's property doesn't retain the observer, which the
            // notification center keeps until it's removed)
            *data.pasteboard_observer() = StrongPtr::retain(observer);
            data.setting_clipboard.store(false, Ordering::SeqCst);
        }
    });
    #[cfg(target_os = "tvos")]
    let _ = data;
}

/// Stop sending the pasteboard's changes. Translation of
/// `UIKit_QuitClipboard()`.
pub(crate) fn quit_clipboard(data: &VideoData) {
    autoreleasepool(|| {
        let observer = std::mem::take(&mut *data.pasteboard_observer());

        if !observer.is_nil() {
            // SAFETY: the notification center, and the observer it gave
            // init_clipboard().
            unsafe {
                let center: Id = msg_send![class!(NSNotificationCenter), defaultCenter];
                let () = msg_send![center, removeObserver: observer.get()];
            }
        }
    })
}
