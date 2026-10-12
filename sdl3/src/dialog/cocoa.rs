// Rust translation of src/dialog/cocoa/SDL_cocoadialog.m from Simple
// DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The macOS file dialogs: `NSOpenPanel` (files and folders) and
//! `NSSavePanel`, as a sheet on the parent window when there is one (the
//! callback runs from its completion handler), or else run app-modally
//! (the callback runs before the dialog function returns). Call them on the
//! main thread, as AppKit requires.
//!
//! The filters become `UTType`s on macOS 11 and later (UniformTypeIdentifiers,
//! which upstream links weakly, is loaded at run time here) or file
//! extension strings before.

use std::sync::{Mutex, OnceLock};

use super::{DialogFileCallback, DialogFileFilter, FileDialogOptions, FileDialogType};
use crate::core::apple::block::RcBlock;
use crate::core::apple::foundation::{ns_string, string_from_ns};
use crate::core::apple::objc::{
    autoreleasepool, class, msg_send, Bool, Id, NSInteger, NSUInteger, StrongPtr, NO, YES,
};
use crate::error::Error;
use crate::events::WindowID;
use crate::video::window::PROP_WINDOW_COCOA_WINDOW_POINTER;

#[cfg(test)]
mod tests;

/// `NSModalResponseOK`.
const NS_MODAL_RESPONSE_OK: NSInteger = 1;
/// `NSModalResponseCancel`.
const NS_MODAL_RESPONSE_CANCEL: NSInteger = 0;

#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {}

/// Load UniformTypeIdentifiers (macOS 11 and later; upstream links it
/// weakly), so that its `UTType` class is known to the runtime: the
/// `@available(macOS 11.0, *)` checks become "is there a `UTType` class".
fn ut_type_class() -> crate::core::apple::objc::Class {
    static FRAMEWORK: OnceLock<Option<crate::loadso::SharedObject>> = OnceLock::new();
    FRAMEWORK.get_or_init(|| {
        crate::loadso::SharedObject::load(
            "/System/Library/Frameworks/UniformTypeIdentifiers.framework/UniformTypeIdentifiers",
        )
        .ok()
    });
    class!(UTType)
}

/// Add the allowed type of one extension of a filter pattern to `types`, as
/// the panel takes them (a `UTType`, or the extension string before macOS
/// 11). Translation of `AddFileExtensionType()`.
///
/// This function assumes that it's called from within an autorelease pool.
fn add_file_extension_type(types: Id, pattern: &str) {
    if pattern.is_empty() {
        return; // in case the string had an extra ';' at the end.
    }

    // -[UTType typeWithFilenameExtension] will return nil if there's a period in the string. It's better to
    //  allow too many files than not allow the one the user actually needs, so just take the part after the '.'
    let ext = match pattern.rfind('.') {
        Some(dot) => &pattern[dot + 1..],
        None => pattern,
    };
    let extstr = ns_string(ext);
    let uttype_class = ut_type_class();
    // SAFETY: `types` is an NSMutableArray; UTType's class method takes an
    // NSString.
    unsafe {
        if !uttype_class.is_null() {
            let uttype: Id = msg_send![uttype_class, typeWithFilenameExtension: extstr.get()];
            if !uttype.is_null() {
                // still failed? Don't add the pattern. This is what the pre-macOS11 path does internally anyhow.
                let _: () = msg_send![types, addObject: uttype];
            }
        } else {
            let _: () = msg_send![types, addObject: extstr.get()];
        }
    }
}

/// The extensions of each filter, split at `;` as upstream walks them, and
/// whether any filter has a `*` ("all files").
fn filter_extensions(filters: &[DialogFileFilter]) -> (Vec<String>, bool) {
    let mut extensions = Vec::new();
    let mut has_all_files = false;
    for f in filters {
        if f.pattern.contains('*') {
            has_all_files = true;
        }
        extensions.extend(f.pattern.split(';').map(str::to_owned));
    }
    (extensions, has_all_files)
}

/// Bring the application back to the front after a panel closes.
/// Translation of `ReactivateAfterDialog()`.
fn reactivate_after_dialog() {
    // SAFETY: NSRunningApplication and NSApplication as documented.
    unsafe {
        let dock = ns_string("com.apple.dock");
        let apps: Id = msg_send![class!(NSRunningApplication),
            runningApplicationsWithBundleIdentifier: dock.get()];
        let count: NSUInteger = msg_send![apps, count];
        if count > 0 {
            let i: Id = msg_send![apps, objectAtIndex: 0 as NSUInteger];
            let _: Bool = msg_send![i, activateWithOptions: 0 as NSUInteger];
        }
        let app: Id = msg_send![class!(NSApplication), sharedApplication];
        let _: () = msg_send![app, activateIgnoringOtherApps: YES];
    }
}

/// The paths a panel that was accepted selected: every URL of an open
/// panel, the one URL of a save panel.
///
/// # Safety
///
/// `dialog` must be the panel, and `dialog_as_open` it as an open panel or
/// nil.
unsafe fn selected_paths(dialog: Id, dialog_as_open: Id) -> Vec<String> {
    // SAFETY: the caller's contract; NSURL's path is an NSString.
    unsafe {
        if !dialog_as_open.is_null() {
            let urls: Id = msg_send![dialog_as_open, URLs];
            let count: NSUInteger = msg_send![urls, count];
            (0..count)
                .map(|i| {
                    let url: Id = msg_send![urls, objectAtIndex: i];
                    let path: Id = msg_send![url, path];
                    string_from_ns(path).unwrap_or_default()
                })
                .collect()
        } else {
            let url: Id = msg_send![dialog, URL];
            let path: Id = msg_send![url, path];
            vec![string_from_ns(path).unwrap_or_default()]
        }
    }
}

/// The parent window's `NSWindow` (`SDL_PROP_WINDOW_COCOA_WINDOW_POINTER`),
/// or nil.
fn window_nswindow(window: Option<WindowID>) -> Id {
    window
        .and_then(|id| crate::video::Window::from_id(id).ok())
        .and_then(|w| w.properties().ok())
        .and_then(|props| props.get_number(PROP_WINDOW_COCOA_WINDOW_POINTER))
        .unwrap_or(0) as usize as Id
}

/// Translation of `Cocoa_SetWindowHasModalDialog()` (a no-op without the
/// Cocoa video driver, as upstream's macro).
fn set_window_has_modal_dialog(window: Option<WindowID>, has_modal: bool) {
    if let Some(window) = window {
        crate::video::drivers::cocoa::window::set_window_has_modal_dialog(window, has_modal);
    }
}

/// Translation of `SDL_SYS_ShowFileDialogWithProperties()`.
pub(super) fn show_file_dialog(
    dialog_type: FileDialogType,
    callback: DialogFileCallback,
    options: FileDialogOptions,
) {
    let FileDialogOptions {
        filters,
        window,
        location: default_location,
        many: allow_many,
        title,
        accept,
        ..
    } = options;

    // (the filters were validated by show_file_dialog())

    if crate::hints::get(crate::hints::FILE_DIALOG_DRIVER).is_some() {
        callback(
            Err(Error::new(
                "File dialog driver unsupported (don't set SDL_HINT_FILE_DIALOG_DRIVER)",
            )),
            None,
        );
        return;
    }

    autoreleasepool(|| {
        // SAFETY: the panels and their properties as AppKit documents them,
        // on the main thread (the caller's duty, as upstream's).
        unsafe {
            // NSOpenPanel inherits from NSSavePanel
            let mut dialog_as_open: Id = std::ptr::null_mut();

            let dialog: Id = match dialog_type {
                FileDialogType::SaveFile => msg_send![class!(NSSavePanel), savePanel],
                FileDialogType::OpenFile => {
                    dialog_as_open = msg_send![class!(NSOpenPanel), openPanel];
                    let _: () = msg_send![dialog_as_open,
                        setAllowsMultipleSelection: if allow_many { YES } else { NO }];
                    dialog_as_open
                }
                FileDialogType::OpenFolder => {
                    dialog_as_open = msg_send![class!(NSOpenPanel), openPanel];
                    let _: () = msg_send![dialog_as_open, setCanChooseFiles: NO];
                    let _: () = msg_send![dialog_as_open, setCanChooseDirectories: YES];
                    let _: () = msg_send![dialog_as_open,
                        setAllowsMultipleSelection: if allow_many { YES } else { NO }];
                    dialog_as_open
                }
            };

            if let Some(title) = &title {
                let _: () = msg_send![dialog, setTitle: ns_string(title).get()];
            }

            if let Some(accept) = &accept {
                let _: () = msg_send![dialog, setPrompt: ns_string(accept).get()];
            }

            if !filters.is_empty() {
                // On macOS 11.0 and up, this is an array of UTType. Prior to that, it's an array of NSString
                let types: Id = msg_send![class!(NSMutableArray), alloc];
                let types: Id = msg_send![types, initWithCapacity: filters.len() as NSUInteger];
                let types = StrongPtr::from_owned(types);

                let (extensions, has_all_files) = filter_extensions(&filters);
                for ext in &extensions {
                    add_file_extension_type(types.get(), ext);
                }

                if !has_all_files {
                    if !ut_type_class().is_null() {
                        let _: () = msg_send![dialog, setAllowedContentTypes: types.get()];
                    } else {
                        let _: () = msg_send![dialog, setAllowedFileTypes: types.get()];
                    }
                }
            }

            // Keep behavior consistent with other platforms
            let _: () = msg_send![dialog, setAllowsOtherFileTypes: YES];

            if let Some(default_location) = default_location.as_deref().filter(|l| !l.is_empty()) {
                let url: Id = msg_send![class!(NSURL),
                    fileURLWithPath: ns_string(default_location).get()];
                if default_location.ends_with('/') {
                    let _: () = msg_send![dialog, setDirectoryURL: url];
                } else {
                    let dir: Id = msg_send![url, URLByDeletingLastPathComponent];
                    let _: () = msg_send![dialog, setDirectoryURL: dir];
                    let name: Id = msg_send![url, lastPathComponent];
                    let _: () = msg_send![dialog, setNameFieldStringValue: name];
                }
            }

            let w = window_nswindow(window);
            if !w.is_null() {
                set_window_has_modal_dialog(window, true);
            }

            if !w.is_null() {
                let panel = dialog;
                let dialog = StrongPtr::retain(dialog);
                let dialog_as_open = StrongPtr::retain(dialog_as_open);
                let callback = Mutex::new(Some(callback));
                let handler = RcBlock::new1(move |result: NSInteger| {
                    let callback = callback.lock().unwrap_or_else(|e| e.into_inner()).take();
                    if let Some(callback) = callback {
                        if result == NS_MODAL_RESPONSE_OK {
                            // (the panel and its open-panel view, kept
                            // alive by the block)
                            let files = selected_paths(dialog.get(), dialog_as_open.get());
                            callback(Ok(files), None);
                        } else if result == NS_MODAL_RESPONSE_CANCEL {
                            callback(Ok(Vec::new()), None);
                        }
                        // Note (upstream): any other response (an aborted
                        // sheet) never calls the callback; here it is
                        // dropped with the block.
                    }

                    set_window_has_modal_dialog(window, false);
                    reactivate_after_dialog();
                });
                // [dialog beginWithCompletionHandler:^(NSInteger result) {
                let _: () = msg_send![panel,
                    beginSheetModalForWindow: w
                    completionHandler: handler.as_ptr()];
            } else {
                let response: NSInteger = msg_send![dialog, runModal];
                if response == NS_MODAL_RESPONSE_OK {
                    callback(Ok(selected_paths(dialog, dialog_as_open)), None);
                } else {
                    callback(Ok(Vec::new()), None);
                }

                reactivate_after_dialog();
            }
        }
    });
}
