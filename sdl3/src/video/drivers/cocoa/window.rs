// Rust translation of src/video/cocoa/SDL_cocoawindow.m and
// SDL_cocoawindow.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Windows: `SDL3Window` (the `NSWindow` subclass, with drag and drop),
//! `SDL3Cocoa_WindowListener` (its delegate, which turns window
//! notifications and mouse, touch and tablet events into SDL events) and
//! `SDL3View` (the content view), and the window entry points: creation
//! and destruction, position and size, fullscreen (in a Space, or
//! exclusive), minimize/maximize/restore, borders, levels, grabs, parents
//! and modality, opacity, hit testing and attention requests.
//!
//! The driver data of a window ([`WindowData`], `SDL_CocoaWindowData`)
//! lives in `window->internal`; the Objective-C objects find it by the
//! window ID they keep, as upstream's `__weak` references find theirs (or
//! nil, once the window is gone).

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use super::events::{encoding, get_event_timestamp, ns_app, BOOL_ENC};
use super::sys::*;
use super::{mouse, pen, video_data, VideoData};
use crate::core::apple::block::{run_on_main_async, RcBlock};
use crate::core::apple::foundation::{ns_array_items, ns_string, NSPoint, NSRect, NSSize};
use crate::core::apple::objc::{
    autoreleasepool, class, imp, is_kind_of, ivar_ptr, msg_send, msg_send_super, sel, Bool, Class,
    ClassBuilder, DeclaredClass, Id, NSInteger, NSUInteger, Sel, StrongPtr, NIL, NO, YES,
};
use crate::error::{Error, Result};
use crate::events::mouse::{
    send_mouse_button, send_mouse_motion, set_mouse_focus, with_mouse, BUTTON_LEFT, BUTTON_MIDDLE,
    BUTTON_RIGHT, DEFAULT_MOUSE_ID, GLOBAL_MOUSE_ID,
};
use crate::events::touch::{
    add_touch, send_pinch, send_touch, send_touch_motion, touch_fingers, TouchDeviceType,
    MOUSE_TOUCH_ID,
};
use crate::events::window::{
    send_drop_complete, send_drop_file, send_drop_position, send_drop_text, send_window_event,
    WindowFlags,
};
use crate::events::{DisplayID, EventType, WindowID};
use crate::hints;
use crate::log::Category;
use crate::properties::Properties;
use crate::video::core::{window_ids, with_window};
use crate::video::sysvideo::{FlashOperation, FullscreenOp, FullscreenResult, HitTestResult};
use crate::video::window::{
    global_to_relative_for_window, relative_to_global_for_window, should_allow_topmost,
    should_focus_popup, should_relinquish_popup_focus, Window,
    PROP_WINDOW_COCOA_METAL_VIEW_TAG_NUMBER, PROP_WINDOW_COCOA_WINDOW_POINTER,
    PROP_WINDOW_CREATE_COCOA_VIEW_POINTER, PROP_WINDOW_CREATE_COCOA_WINDOW_POINTER,
    PROP_WINDOW_SHAPE_POINTER,
};
use crate::video::{Point, Rect, Surface};

/// Translation of `SDL_METALVIEW_TAG`.
pub(crate) const SDL_METALVIEW_TAG: NSInteger = 255;

/// Translation of `PendingWindowOperation`.
pub(crate) type PendingWindowOperation = u32;
pub(crate) const PENDING_OPERATION_NONE: PendingWindowOperation = 0x00;
pub(crate) const PENDING_OPERATION_ENTER_FULLSCREEN: PendingWindowOperation = 0x01;
pub(crate) const PENDING_OPERATION_LEAVE_FULLSCREEN: PendingWindowOperation = 0x02;
pub(crate) const PENDING_OPERATION_MINIMIZE: PendingWindowOperation = 0x04;
pub(crate) const PENDING_OPERATION_ZOOM: PendingWindowOperation = 0x08;

/// The instance variables of `SDL3Cocoa_WindowListener`.
pub(crate) struct ListenerState {
    observing_visible: bool,
    was_ctrl_left: bool,
    was_visible: bool,
    is_fullscreen_space: bool,
    in_fullscreen_transition: bool,
    pending_window_operation: PendingWindowOperation,
    is_moving: bool,
    is_miniaturizing: bool,
    focus_click_pending: NSInteger,
    pending_window_warp_x: f32,
    pending_window_warp_y: f32,
    is_drag_area_running: bool,
    live_resize_timer: StrongPtr,
}

impl ListenerState {
    fn new() -> ListenerState {
        ListenerState {
            observing_visible: false,
            was_ctrl_left: false,
            was_visible: false,
            is_fullscreen_space: false,
            in_fullscreen_transition: false,
            pending_window_operation: PENDING_OPERATION_NONE,
            is_moving: false,
            is_miniaturizing: false,
            focus_click_pending: 0,
            pending_window_warp_x: f32::MAX,
            pending_window_warp_y: f32::MAX,
            is_drag_area_running: false,
            live_resize_timer: StrongPtr::nil(),
        }
    }
}

/// The mutable part of `SDL_CocoaWindowData`.
pub(crate) struct WindowState {
    pub(crate) viewport: NSRect,
    /// The window's OpenGL contexts (`nscontexts`, filled by
    /// `SDL_cocoaopengl.m`'s contexts as they attach to the window).
    pub(crate) nscontexts: Vec<StrongPtr>,
    pub(crate) in_blocking_transition: bool,
    pub(crate) fullscreen_space_requested: bool,
    pub(crate) was_zoomed: bool,
    pub(crate) window_number: NSInteger,
    pub(crate) flash_request: NSInteger,
    pub(crate) modal_session: NSModalSession,
    pub(crate) pending_size: bool,
    pub(crate) pending_position: bool,
    pub(crate) border_toggled: bool,
    pub(crate) has_modal_dialog: bool,
    /// The window's `EGLSurface` (`SDL_cocoaopengles.m`).
    #[allow(dead_code)] // (used by the OpenGL ES windows)
    pub(crate) egl_surface: *mut c_void,
}

/// The driver data of a window. Translation of `SDL_CocoaWindowData`.
pub(crate) struct WindowData {
    pub(crate) window: WindowID,
    pub(crate) nswindow: StrongPtr,
    pub(crate) sdl_content_view: StrongPtr,
    /// The `SDL3Cocoa_WindowListener` (nil once the window is destroyed).
    listener: Mutex<StrongPtr>,
    pub(crate) videodata: Arc<VideoData>,
    state: Mutex<WindowState>,
    listener_state: Mutex<ListenerState>,
}

// SAFETY: the raw pointers (the modal session, the EGL surface) are
// opaque tokens used on the main thread; the rest is Send.
unsafe impl Send for WindowState {}

impl WindowData {
    /// The window state. Never call into AppKit or SDL's event functions
    /// while holding it.
    pub(crate) fn state(&self) -> MutexGuard<'_, WindowState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The listener's instance variables (as [`WindowData::state`]).
    fn ls(&self) -> MutexGuard<'_, ListenerState> {
        self.listener_state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    /// The `SDL3Cocoa_WindowListener` (`data.listener`).
    pub(crate) fn listener(&self) -> Id {
        self.listener
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get()
    }

    pub(crate) fn nswindow(&self) -> Id {
        self.nswindow.get()
    }

    /// `[data.listener isInFullscreenSpace]`.
    pub(crate) fn is_in_fullscreen_space(&self) -> bool {
        self.ls().is_fullscreen_space
    }

    /// `[data.listener isInFullscreenSpaceTransition]`.
    pub(crate) fn is_in_fullscreen_space_transition(&self) -> bool {
        self.ls().in_fullscreen_transition
    }

    /// `[data.listener isMovingOrFocusClickPending]`.
    pub(crate) fn is_moving_or_focus_click_pending(&self) -> bool {
        let ls = self.ls();
        ls.is_moving || ls.focus_click_pending != 0
    }

    /// `[data.listener isMoving]`.
    pub(crate) fn is_moving(&self) -> bool {
        self.ls().is_moving
    }

    /// `[data.listener setPendingMoveX:x Y:y]`.
    pub(crate) fn set_pending_move(&self, x: f32, y: f32) {
        let mut ls = self.ls();
        ls.pending_window_warp_x = x;
        ls.pending_window_warp_y = y;
    }

    /// `[data.listener setFocusClickPending:button]`.
    pub(crate) fn set_focus_click_pending(&self, button: NSInteger) {
        self.ls().focus_click_pending |= 1 << button;
    }

    /// `[data.listener clearFocusClickPending:button]`.
    pub(crate) fn clear_focus_click_pending(self: &Arc<Self>, button: NSInteger) {
        let cleared = {
            let mut ls = self.ls();
            if ls.focus_click_pending & (1 << button) != 0 {
                ls.focus_click_pending &= !(1 << button);
                ls.focus_click_pending == 0
            } else {
                false
            }
        };
        if cleared {
            on_moving_or_focus_click_pending_state_cleared(self);
        }
    }

    /// `[data.listener clearPendingWindowOperation:operation]`.
    fn clear_pending_window_operation(&self, operation: PendingWindowOperation) {
        self.ls().pending_window_operation &= !operation;
    }

    /// `[data.listener clearAllPendingWindowOperations]`.
    fn clear_all_pending_window_operations(&self) {
        self.ls().pending_window_operation = PENDING_OPERATION_NONE;
    }

    /// `[data.listener addPendingWindowOperation:operation]`.
    pub(crate) fn add_pending_window_operation(&self, operation: PendingWindowOperation) {
        self.ls().pending_window_operation |= operation;
    }

    /// `[data.listener windowOperationIsPending:operation]`.
    fn window_operation_is_pending(&self, operation: PendingWindowOperation) -> bool {
        self.ls().pending_window_operation & operation != 0
    }

    /// `[data.listener hasPendingWindowOperation]`.
    fn has_pending_window_operation(&self) -> bool {
        let ls = self.ls();
        // A pending zoom may be deferred until leaving fullscreen, so don't block on it.
        (ls.pending_window_operation & !PENDING_OPERATION_ZOOM) != PENDING_OPERATION_NONE
            || ls.is_miniaturizing
            || ls.in_fullscreen_transition
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

/// The SDL window of an `NSWindow` (`-[SDL3Window findSDLWindow]`,
/// `FindSDLWindowForNSWindow()`).
pub(crate) fn find_sdl_window(nswindow: Id) -> Option<WindowID> {
    // !!! FIXME: is there a better way to do this?
    window_ids()
        .into_iter()
        .find(|&w| window_data(w).is_some_and(|d| d.nswindow() == nswindow))
}

/// A window's flags (empty for a window that's gone).
fn flags(window: WindowID) -> WindowFlags {
    with_window(window, |w| w.flags()).unwrap_or_default()
}

/// `window->x, window->y, window->w, window->h`.
fn geometry(window: WindowID) -> (i32, i32, i32, i32) {
    with_window(window, |w| (w.core.x, w.core.y, w.core.w, w.core.h)).unwrap_or_default()
}

/// Translation of `SDL_WINDOW_IS_POPUP()`.
fn is_popup(window: WindowID) -> bool {
    with_window(window, |w| w.is_popup()).unwrap_or(false)
}

/// `window->parent`.
fn parent_of(window: WindowID) -> Option<WindowID> {
    with_window(window, |w| w.parent).ok().flatten()
}

/// `SDL_SendWindowEvent()` ignoring whether it was posted.
fn send(window: WindowID, event: EventType, data1: i32, data2: i32) {
    send_window_event(window, event, data1, data2);
}

// ---------------------------------------------------------------------------
// SDL3Window

static SDL3_WINDOW: DeclaredClass = DeclaredClass::new();

/// The `SDL3Window` class (`@interface SDL3Window : NSWindow
/// <NSDraggingDestination>`).
pub(crate) fn sdl3_window_class() -> Class {
    SDL3_WINDOW.get(|| {
        let mut b = ClassBuilder::new(c"SDL3Window", class!(NSWindow)).expect("SDL3Window");
        b.add_protocol(c"NSDraggingDestination");
        // SAFETY: the implementations have the signatures of the encodings.
        unsafe {
            b.add_method(
                sel!(validateMenuItem:),
                imp!(window_validate_menu_item as extern "C" fn(Id, Sel, Id) -> Bool),
                &encoding("B@:@"),
            );
            b.add_method(
                sel!(canBecomeKeyWindow),
                imp!(window_can_become_key_window as extern "C" fn(Id, Sel) -> Bool),
                &encoding("B@:"),
            );
            b.add_method(
                sel!(canBecomeMainWindow),
                imp!(window_can_become_main_window as extern "C" fn(Id, Sel) -> Bool),
                &encoding("B@:"),
            );
            b.add_method(
                sel!(sendEvent:),
                imp!(window_send_event as extern "C" fn(Id, Sel, Id)),
                c"v@:@",
            );
            b.add_method(
                sel!(doCommandBySelector:),
                imp!(do_command_by_selector as extern "C" fn(Id, Sel, Sel)),
                c"v@::",
            );
            b.add_method(
                sel!(draggingEntered:),
                imp!(window_dragging_entered as extern "C" fn(Id, Sel, Id) -> NSUInteger),
                c"Q@:@",
            );
            b.add_method(
                sel!(draggingExited:),
                imp!(window_dragging_exited as extern "C" fn(Id, Sel, Id)),
                c"v@:@",
            );
            b.add_method(
                sel!(draggingUpdated:),
                imp!(window_dragging_updated as extern "C" fn(Id, Sel, Id) -> NSUInteger),
                c"Q@:@",
            );
            b.add_method(
                sel!(performDragOperation:),
                imp!(window_perform_drag_operation as extern "C" fn(Id, Sel, Id) -> Bool),
                &encoding("B@:@"),
            );
            b.add_method(
                sel!(wantsPeriodicDraggingUpdates),
                imp!(window_wants_periodic_dragging_updates as extern "C" fn(Id, Sel) -> Bool),
                &encoding("B@:"),
            );
        }
        b.register()
    })
}

/// Translation of `-[SDL3Window validateMenuItem:]`.
extern "C" fn window_validate_menu_item(this: Id, _sel: Sel, menu_item: Id) -> Bool {
    /* Only allow using the macOS native fullscreen toggle menubar item if the
     * window is resizable and not in a SDL fullscreen mode.
     */
    // SAFETY: an NSMenuItem.
    let action: Sel = unsafe { msg_send![menu_item, action] };
    if action == sel!(toggleFullScreen:) {
        let Some(window) = find_sdl_window(this) else {
            return NO;
        };

        let data = window_data(window);
        let f = flags(window);
        if (f.contains(WindowFlags::FULLSCREEN)
            && !data.as_ref().is_some_and(|d| d.is_in_fullscreen_space()))
            || !f.contains(WindowFlags::RESIZABLE)
        {
            return NO;
        }
    }
    // SAFETY: NSWindow's implementation.
    unsafe { msg_send_super![this, sdl3_window_class(), validateMenuItem: menu_item] }
}

/// Translation of `-[SDL3Window canBecomeKeyWindow]`.
extern "C" fn window_can_become_key_window(this: Id, _sel: Sel) -> Bool {
    match find_sdl_window(this) {
        Some(window)
            if !flags(window).intersects(WindowFlags::TOOLTIP | WindowFlags::NOT_FOCUSABLE) =>
        {
            YES
        }
        _ => NO,
    }
}

/// Translation of `-[SDL3Window canBecomeMainWindow]`.
extern "C" fn window_can_become_main_window(this: Id, _sel: Sel) -> Bool {
    match find_sdl_window(this) {
        Some(window)
            if !flags(window).intersects(WindowFlags::TOOLTIP | WindowFlags::NOT_FOCUSABLE)
                && !is_popup(window) =>
        {
            YES
        }
        _ => NO,
    }
}

/// Translation of `-[SDL3Window sendEvent:]`.
extern "C" fn window_send_event(this: Id, _sel: Sel, event: Id) {
    // SAFETY: NSWindow's implementation, then an NSEvent and the window's
    // delegate.
    unsafe {
        let _: () = msg_send_super![this, sdl3_window_class(), sendEvent: event];

        let ty: NSUInteger = msg_send![event, type];
        if ty != NSEventTypeLeftMouseUp {
            return;
        }

        let delegate: Id = msg_send![this, delegate];
        if !is_kind_of(delegate, sdl3_cocoa_window_listener_class()) {
            return;
        }

        let moving: Bool = msg_send![delegate, isMoving];
        if moving.is_true() {
            let _: () = msg_send![delegate, windowDidFinishMoving];
        }
    }
}

/// We'll respond to selectors by doing nothing so we don't beep.
/// The escape key gets converted to a "cancel" selector, etc.
/// Translation of `-[SDL3Window doCommandBySelector:]` (and the listener's).
extern "C" fn do_command_by_selector(_this: Id, _sel: Sel, _a_selector: Sel) {
    // NSLog(@"doCommandBySelector: %@\n", NSStringFromSelector(aSelector));
}

/// `[sender draggingSourceOperationMask]`.
fn dragging_source_operation_mask(sender: Id) -> NSUInteger {
    // SAFETY: an NSDraggingInfo.
    unsafe { msg_send![sender, draggingSourceOperationMask] }
}

/// Translation of `-[SDL3Window draggingEntered:]`.
extern "C" fn window_dragging_entered(_this: Id, _sel: Sel, sender: Id) -> NSUInteger {
    let mask = dragging_source_operation_mask(sender);
    if (mask & NSDragOperationGeneric) == NSDragOperationGeneric {
        return NSDragOperationGeneric;
    } else if (mask & NSDragOperationCopy) == NSDragOperationCopy {
        return NSDragOperationCopy;
    }

    NSDragOperationNone // no idea what to do with this, reject it.
}

/// Translation of `-[SDL3Window draggingExited:]`.
extern "C" fn window_dragging_exited(this: Id, _sel: Sel, _sender: Id) {
    let sdlwindow = find_sdl_window(this);
    send_drop_complete(sdlwindow);
}

/// `SDL_SendDropPosition()` at `[sender draggingLocation]`.
fn send_dragging_position(this: Id, sender: Id) {
    let Some(sdlwindow) = find_sdl_window(this) else {
        return;
    };
    // SAFETY: an NSDraggingInfo.
    let point: NSPoint = unsafe { msg_send![sender, draggingLocation] };
    let (_, _, _, h) = geometry(sdlwindow);
    let x = point.x as f32;
    let y = (h as f64 - point.y) as f32;
    send_drop_position(Some(sdlwindow), x, y);
}

/// Translation of `-[SDL3Window draggingUpdated:]`.
extern "C" fn window_dragging_updated(this: Id, _sel: Sel, sender: Id) -> NSUInteger {
    let mask = dragging_source_operation_mask(sender);
    if (mask & NSDragOperationGeneric) == NSDragOperationGeneric {
        send_dragging_position(this, sender);
        return NSDragOperationGeneric;
    } else if (mask & NSDragOperationCopy) == NSDragOperationCopy {
        send_dragging_position(this, sender);
        return NSDragOperationCopy;
    }

    NSDragOperationNone // no idea what to do with this, reject it.
}

/// Translation of `-[SDL3Window performDragOperation:]`.
extern "C" fn window_perform_drag_operation(this: Id, _sel: Sel, sender: Id) -> Bool {
    crate::log::trace!(
        Category::Input,
        ". [SDL] In performDragOperation, draggingSourceOperationMask {:x}, expected Generic {:x}, others Copy {:x}, Link {:x}, Private {:x}, Move {:x}, Delete {:x}\n",
        dragging_source_operation_mask(sender),
        NSDragOperationGeneric,
        NSDragOperationCopy,
        NSDragOperationLink,
        NSDragOperationPrivate,
        NSDragOperationMove,
        NSDragOperationDelete
    );
    // SAFETY: the dragging pasteboard and its NSString/NSArray contents.
    unsafe {
        let pasteboard: Id = msg_send![sender, draggingPasteboard];
        if !pasteboard.is_null() {
            let name: Id = msg_send![pasteboard, name];
            let class_name: Id = msg_send![name, className];
            let description: Id = msg_send![name, description];
            let change_count: NSInteger = msg_send![pasteboard, changeCount];
            crate::log::trace!(
                Category::Input,
                ". [SDL] In performDragOperation, valid draggingPasteboard, name [{}] '{}', changeCount {}\n",
                super::ns_to_string(class_name),
                super::ns_to_string(description),
                change_count
            );
        }
        autoreleasepool(|| {
            let types = [NSFilenamesPboardType, NSPasteboardTypeString];
            let type_array: Id = msg_send![class!(NSArray),
                arrayWithObjects: types.as_ptr() count: types.len() as NSUInteger];
            let desired_type: Id = msg_send![pasteboard, availableTypeFromArray: type_array];
            let sdlwindow = find_sdl_window(this);

            let all_types: Id = msg_send![pasteboard, types];
            for supported_type in ns_array_items(all_types) {
                let type_string: Id = msg_send![pasteboard, stringForType: supported_type];
                let type_description: Id = msg_send![type_string, description];
                let len: NSUInteger = if type_description.is_null() {
                    0
                } else {
                    msg_send![type_description, length]
                };
                let supported_description: Id = msg_send![supported_type, description];
                crate::log::trace!(
                    Category::Input,
                    ". [SDL] In performDragOperation, Pasteboard type '{}', stringForType ({}) '{}'\n",
                    super::ns_to_string(supported_description),
                    len,
                    super::ns_to_string(type_description)
                );
            }

            if desired_type.is_null() {
                return NO; // can't accept anything that's being dropped here.
            }
            let pboard_data: Id = msg_send![pasteboard, dataForType: desired_type];
            if pboard_data.is_null() {
                return NO;
            }
            let is_filenames: Bool =
                msg_send![desired_type, isEqualToString: NSFilenamesPboardType];
            let is_string: Bool = msg_send![desired_type, isEqualToString: NSPasteboardTypeString];
            crate::sdl_assert!(is_filenames.is_true() || is_string.is_true());

            let pboard_string: Id = msg_send![pasteboard, stringForType: desired_type];
            let pboard_plist: Id = msg_send![pasteboard, propertyListForType: desired_type];

            // Use SendDropPosition to update the mouse location
            let point: NSPoint = msg_send![sender, draggingLocation];
            let (_, _, w, h) = sdlwindow.map(geometry).unwrap_or_default();
            let x = point.x as f32;
            let y = (h as f64 - point.y) as f32;
            if x >= 0.0 && x < w as f32 && y >= 0.0 && y < h as f32 {
                send_drop_position(sdlwindow, x, y);
            }
            // Use SendDropPosition to update the mouse location

            if is_filenames.is_true() {
                for path in ns_array_items(pboard_plist) {
                    let mut file_url: Id = msg_send![class!(NSURL), fileURLWithPath: path];
                    let mut is_alias: Id = NIL;

                    let _: Bool = msg_send![file_url, getResourceValue: &mut is_alias as *mut Id
                                                               forKey: NSURLIsAliasFileKey
                                                                error: std::ptr::null_mut::<Id>()];

                    // If the URL is an alias, resolve it.
                    let alias: Bool = if is_alias.is_null() {
                        NO
                    } else {
                        msg_send![is_alias, boolValue]
                    };
                    if alias.is_true() {
                        let opts = NSURLBookmarkResolutionWithoutMounting
                            | NSURLBookmarkResolutionWithoutUI;
                        let bookmark: Id = msg_send![class!(NSURL),
                            bookmarkDataWithContentsOfURL: file_url
                            error: std::ptr::null_mut::<Id>()];
                        if !bookmark.is_null() {
                            let resolved_url: Id = msg_send![class!(NSURL),
                                URLByResolvingBookmarkData: bookmark
                                options: opts
                                relativeToURL: NIL
                                bookmarkDataIsStale: std::ptr::null_mut::<Bool>()
                                error: std::ptr::null_mut::<Id>()];
                            if !resolved_url.is_null() {
                                file_url = resolved_url;
                            }
                        }
                    }
                    let file_path: Id = msg_send![file_url, path];
                    let file_description: Id = msg_send![file_path, description];
                    let file = super::ns_to_string(file_description);
                    let desired_description: Id = msg_send![desired_type, description];
                    crate::log::trace!(
                        Category::Input,
                        ". [SDL] In performDragOperation, desiredType '{}', Submitting DropFile as ({}) '{}'\n",
                        super::ns_to_string(desired_description),
                        file.len(),
                        file
                    );
                    if !send_drop_file(sdlwindow, None, &file) {
                        return NO;
                    }
                }
            } else if is_string.is_true() {
                let description: Id = msg_send![pboard_string, description];
                let buffer = super::ns_to_string(description);
                let desired_description: Id = msg_send![desired_type, description];
                let desired_description = super::ns_to_string(desired_description);
                for token in buffer.split(['\r', '\n']).filter(|t| !t.is_empty()) {
                    crate::log::trace!(
                        Category::Input,
                        ". [SDL] In performDragOperation, desiredType '{}', Submitting DropText as ({}) '{}'\n",
                        desired_description,
                        token.len(),
                        token
                    );
                    if !send_drop_text(sdlwindow, token) {
                        return NO;
                    }
                }
            }

            send_drop_complete(sdlwindow);
            YES
        })
    }
}

/// Translation of `-[SDL3Window wantsPeriodicDraggingUpdates]`.
extern "C" fn window_wants_periodic_dragging_updates(_this: Id, _sel: Sel) -> Bool {
    NO
}

/// Translation of `b_inModeTransition`.
pub(crate) static B_IN_MODE_TRANSITION: AtomicBool = AtomicBool::new(false);

fn in_mode_transition() -> bool {
    B_IN_MODE_TRANSITION.load(Ordering::Relaxed)
}

/// Translation of `SqDistanceToRect()`.
fn sq_distance_to_rect(point: &NSPoint, rect: &NSRect) -> f64 {
    let mut edge = *point;
    let left = rect.min_x();
    let right = rect.max_x();
    // FIXME (upstream): the bottom edge is NSMinX(), not NSMinY().
    let bottom = rect.min_x();
    let top = rect.max_y();

    if point.x < left {
        edge.x = left;
    } else if point.x > right {
        edge.x = right;
    }

    if point.y < bottom {
        edge.y = bottom;
    } else if point.y > top {
        edge.y = top;
    }

    let delta = NSPoint::new(edge.x - point.x, edge.y - point.y);
    delta.x * delta.x + delta.y * delta.y
}

/// `[screen frame]`.
fn screen_frame(screen: Id) -> NSRect {
    // SAFETY: an NSScreen (nil gives a zero rectangle).
    unsafe { msg_send![screen, frame] }
}

/// Translation of `ScreenForPoint()`.
fn screen_for_point(point: &NSPoint) -> Id {
    // SAFETY: NSScreen's class method.
    let screens = unsafe { ns_array_items(msg_send![class!(NSScreen), screens]) };
    let mut screen: Id = NIL;

    // Do a quick check first to see if the point lies on a specific screen
    for &candidate in &screens {
        if screen_frame(candidate).contains(*point) {
            screen = candidate;
            break;
        }
    }

    // Find the screen the point is closest to
    if screen.is_null() {
        let mut closest = f64::from(f32::MAX);
        for &candidate in &screens {
            let screen_rect = screen_frame(candidate);

            let sqdist = sq_distance_to_rect(point, &screen_rect);
            if sqdist < closest {
                screen = candidate;
                closest = sqdist;
            }
        }
    }

    screen
}

/// Translation of `Cocoa_IsWindowInFullscreenSpace()`.
pub(crate) fn is_window_in_fullscreen_space(window: WindowID) -> bool {
    window_data(window).is_some_and(|d| d.is_in_fullscreen_space())
}

/// Translation of `Cocoa_IsWindowInFullscreenSpaceTransition()`.
pub(crate) fn is_window_in_fullscreen_space_transition(window: WindowID) -> bool {
    window_data(window).is_some_and(|d| d.is_in_fullscreen_space_transition())
}

/// Translation of `Cocoa_IsWindowZoomed()`.
pub(crate) fn is_window_zoomed(window: WindowID) -> bool {
    let Some(data) = window_data(window) else {
        return false;
    };
    let nswindow = data.nswindow();
    let mut zoomed = false;

    let f = flags(window);
    // isZoomed always returns true if the window is not resizable or the window is fullscreen
    // SAFETY: an NSWindow.
    let is_zoomed: Bool = unsafe { msg_send![nswindow, isZoomed] };
    if f.contains(WindowFlags::RESIZABLE)
        && is_zoomed.is_true()
        && !f.contains(WindowFlags::FULLSCREEN)
        && !is_window_in_fullscreen_space(window)
    {
        // If we are at our desired floating area, then we're not zoomed
        let floating = with_window(window, |w| {
            w.core.x == w.core.floating.x
                && w.core.y == w.core.floating.y
                && w.core.w == w.core.floating.w
                && w.core.h == w.core.floating.h
        })
        .unwrap_or(true);
        if !floating {
            zoomed = true;
        }
    }
    zoomed
}

/// Translation of `Cocoa_IsShowingModalDialog()`.
pub(crate) fn is_showing_modal_dialog(window: WindowID) -> bool {
    window_data(window).is_some_and(|d| d.state().has_modal_dialog)
}

/// Translation of `Cocoa_SetWindowHasModalDialog()` (for the dialog
/// backend).
#[allow(dead_code)]
pub(crate) fn set_window_has_modal_dialog(window: WindowID, has_modal: bool) {
    if let Some(data) = window_data(window) {
        data.state().has_modal_dialog = has_modal;
    }
}

/// Translation of `CocoaMenuVisibility`.
const COCOA_MENU_VISIBILITY_AUTO: u8 = 0;
const COCOA_MENU_VISIBILITY_NEVER: u8 = 1;
const COCOA_MENU_VISIBILITY_ALWAYS: u8 = 2;

/// Translation of `menu_visibility_hint`.
static MENU_VISIBILITY_HINT: AtomicU8 = AtomicU8::new(COCOA_MENU_VISIBILITY_AUTO);

/// `[NSMenu setMenuBarVisible:visible]`.
fn set_menu_bar_visible(visible: bool) {
    // SAFETY: NSMenu's class method.
    let _: () = unsafe { msg_send![class!(NSMenu), setMenuBarVisible: Bool::from(visible)] };
}

/// `[NSMenu menuBarVisible]`.
fn menu_bar_visible() -> bool {
    // SAFETY: NSMenu's class method.
    let r: Bool = unsafe { msg_send![class!(NSMenu), menuBarVisible] };
    r.is_true()
}

/// Translation of `Cocoa_ToggleFullscreenSpaceMenuVisibility()`.
fn toggle_fullscreen_space_menu_visibility(window: Option<WindowID>) {
    let Some(window) = window else {
        return;
    };
    if is_window_in_fullscreen_space(window) {
        let Some(data) = window_data(window) else {
            return;
        };
        let hint = MENU_VISIBILITY_HINT.load(Ordering::Relaxed);

        // 'Auto' sets the menu to visible if fullscreen wasn't explicitly entered via SDL_SetWindowFullscreen().
        if (hint == COCOA_MENU_VISIBILITY_AUTO && !data.state().fullscreen_space_requested)
            || hint == COCOA_MENU_VISIBILITY_ALWAYS
        {
            set_menu_bar_visible(true);
        } else {
            set_menu_bar_visible(false);
        }
    }
}

/// Translation of `Cocoa_MenuVisibilityCallback()`.
pub(crate) fn menu_visibility_callback(new_value: Option<&str>) {
    let hint = match new_value {
        Some(v) => {
            if v.starts_with('0') || v.eq_ignore_ascii_case("false") {
                COCOA_MENU_VISIBILITY_NEVER
            } else if v.starts_with('1') || v.eq_ignore_ascii_case("true") {
                COCOA_MENU_VISIBILITY_ALWAYS
            } else {
                COCOA_MENU_VISIBILITY_AUTO
            }
        }
        None => COCOA_MENU_VISIBILITY_AUTO,
    };
    MENU_VISIBILITY_HINT.store(hint, Ordering::Relaxed);

    // Update the current menu visibility.
    if crate::core::apple::foundation::is_main_thread() {
        toggle_fullscreen_space_menu_visibility(crate::events::keyboard::keyboard_focus());
    }
}

/// Translation of `ScreenForRect()`.
fn screen_for_rect(rect: &NSRect) -> Id {
    let center = NSPoint::new(
        rect.origin.x + rect.size.width / 2.0,
        rect.origin.y + rect.size.height / 2.0,
    );
    screen_for_point(&center)
}

/// Flip a rectangle between SDL's top-down and Cocoa's bottom-up
/// coordinates. Translation of `ConvertNSRect()`.
pub(crate) fn convert_ns_rect(r: &mut NSRect) {
    let main_display_height = video_data().map_or(0.0, |d| d.main_display_height());
    r.origin.y = main_display_height - r.origin.y - r.size.height;
}

/// Translation of `ScheduleContextUpdates()`.
pub(crate) fn schedule_context_updates(data: &WindowData) {
    // We still support OpenGL as long as Apple offers it, deprecated or not.
    let contexts: Vec<StrongPtr> = data.state().nscontexts.clone();
    if contexts.is_empty() {
        return;
    }

    // SAFETY: NSOpenGLContext's class method and the contexts' methods.
    unsafe {
        let current_context: Id = msg_send![class!(NSOpenGLContext), currentContext];
        // (@synchronized(contexts): the list was copied under its lock)
        for context in &contexts {
            if context.get() == current_context {
                let _: () = msg_send![context, update];
            } else {
                let _: () = msg_send![context, scheduleUpdate];
            }
        }
    }
}

/// !!! FIXME: this should use a hint callback.
/// Translation of `GetHintCtrlClickEmulateRightClick()`.
fn get_hint_ctrl_click_emulate_right_click() -> bool {
    hints::get_bool(hints::MAC_CTRL_CLICK_EMULATE_RIGHT_CLICK, false)
}

/// Translation of `GetWindowWindowedStyle()`.
fn get_window_windowed_style(window: WindowID) -> NSUInteger {
    /* IF YOU CHANGE ANY FLAGS IN HERE, PLEASE READ
    the NSWindowStyleMaskBorderless comments in SetupWindowData()! */

    /* always allow miniaturization, otherwise you can't programmatically
    minimize the window, whether there's a title bar or not */
    let mut style = NSWindowStyleMaskMiniaturizable;
    let f = flags(window);

    if !is_popup(window) {
        if f.contains(WindowFlags::BORDERLESS) {
            style |= NSWindowStyleMaskBorderless;
        } else {
            style |= NSWindowStyleMaskTitled | NSWindowStyleMaskClosable;
        }
        if f.contains(WindowFlags::RESIZABLE) {
            style |= NSWindowStyleMaskResizable;
        }
    } else {
        style |= NSWindowStyleMaskBorderless;
    }
    style
}

/// Translation of `GetWindowStyle()`.
fn get_window_style(window: WindowID) -> NSUInteger {
    if flags(window).contains(WindowFlags::FULLSCREEN) {
        NSWindowStyleMaskBorderless
    } else {
        get_window_windowed_style(window)
    }
}

/// `[view nextResponder]`.
fn next_responder(view: Id) -> Id {
    // SAFETY: an NSResponder.
    unsafe { msg_send![view, nextResponder] }
}

/// `[view setNextResponder:responder]`.
fn set_next_responder(view: Id, responder: Id) {
    // SAFETY: two NSResponders (or nil).
    let _: () = unsafe { msg_send![view, setNextResponder: responder] };
}

/// Translation of `SetWindowStyle()`.
fn set_window_style(data: &WindowData, style: NSUInteger) -> bool {
    let nswindow = data.nswindow();
    let view = data.sdl_content_view.get();
    let listener = data.listener();

    // The view responder chain gets messed with during setStyleMask
    if next_responder(view) == listener {
        set_next_responder(view, NIL);
    }

    // SAFETY: an NSWindow.
    let _: () = unsafe { msg_send![nswindow, setStyleMask: style] };

    // The view responder chain gets messed with during setStyleMask
    if next_responder(view) != listener {
        set_next_responder(view, listener);
    }

    true
}

/// Translation of `ShouldAdjustCoordinatesForGrab()`.
fn should_adjust_coordinates_for_grab(window: WindowID) -> bool {
    let Some(data) = window_data(window) else {
        return false;
    };
    if data.is_moving_or_focus_click_pending() {
        return false;
    }

    let (f, mouse_rect) =
        with_window(window, |w| (w.flags(), w.core.mouse_rect)).unwrap_or_default();
    if !f.contains(WindowFlags::INPUT_FOCUS) {
        return false;
    }

    f.contains(WindowFlags::MOUSE_GRABBED) || (mouse_rect.w > 0 && mouse_rect.h > 0)
}

/// Translation of `AdjustCoordinatesForGrab()`.
fn adjust_coordinates_for_grab(window: WindowID, x: f32, y: f32) -> Option<NSPoint> {
    let (wx, wy, ww, wh) = geometry(window);
    let (f, mouse_rect) =
        with_window(window, |w| (w.flags(), w.core.mouse_rect)).unwrap_or_default();

    if mouse_rect.w > 0 && mouse_rect.h > 0 {
        let window_rect = Rect::new(0, 0, ww, wh);

        if let Some(mouse_rect) = mouse_rect.intersection(&window_rect) {
            let left = wx as f32 + mouse_rect.x as f32;
            let right = left + mouse_rect.w as f32 - 1.0;
            let top = wy as f32 + mouse_rect.y as f32;
            let bottom = top + mouse_rect.h as f32 - 1.0;
            if x < left || x > right || y < top || y > bottom {
                return Some(NSPoint::new(
                    x.clamp(left, right) as f64,
                    y.clamp(top, bottom) as f64,
                ));
            }
            return None;
        }
    }

    if f.contains(WindowFlags::MOUSE_GRABBED) {
        let left = wx as f32;
        let right = left + ww as f32 - 1.0;
        let top = wy as f32;
        let bottom = top + wh as f32 - 1.0;
        if x < left || x > right || y < top || y > bottom {
            return Some(NSPoint::new(
                x.clamp(left, right) as f64,
                y.clamp(top, bottom) as f64,
            ));
        }
    }
    None
}

/// Translation of `Cocoa_UpdateClipCursor()`.
pub(crate) fn update_clip_cursor(window: WindowID) {
    let Some(data) = window_data(window) else {
        return;
    };

    if appkit_version_number() >= NSAppKitVersionNumber10_13_2 {
        let nswindow = data.nswindow();
        let mut mouse_rect = Rect::default();

        if should_adjust_coordinates_for_grab(window) {
            let (f, wrect, ww, wh) = with_window(window, |w| {
                (w.flags(), w.core.mouse_rect, w.core.w, w.core.h)
            })
            .unwrap_or_default();
            let window_rect = Rect::new(0, 0, ww, wh);

            if wrect.w > 0 && wrect.h > 0 {
                mouse_rect = wrect.intersection(&window_rect).unwrap_or_default();
            }

            if f.contains(WindowFlags::MOUSE_GRABBED) && mouse_rect.is_empty() {
                mouse_rect = window_rect;
            }
        }

        // SAFETY: an NSWindow; mouseConfinementRect is available as of
        // 10.13.2, but isn't in public headers.
        unsafe {
            if mouse_rect.is_empty() {
                let _: () = msg_send![nswindow, setMouseConfinementRect: NSRect::default()];
            } else {
                let layout: NSRect = msg_send![nswindow, contentLayoutRect];
                let rect = NSRect::new(
                    mouse_rect.x as f64,
                    layout.size.height - mouse_rect.y as f64 - mouse_rect.h as f64,
                    mouse_rect.w as f64,
                    mouse_rect.h as f64,
                );
                let _: () = msg_send![nswindow, setMouseConfinementRect: rect];
            }
        }
    } else {
        // Move the cursor to the nearest point in the window
        if should_adjust_coordinates_for_grab(window) {
            let (x, y, _) = crate::events::mouse::global_mouse_state();
            if let Some(cgpoint) = adjust_coordinates_for_grab(window, x, y) {
                mouse::handle_mouse_warp(cgpoint.x, cgpoint.y);
                // SAFETY: a point on the main display.
                unsafe { CGDisplayMoveCursorToPoint(kCGDirectMainDisplay(), cgpoint) };
            }
        }
    }
}

/// Translation of `GetParentToplevelWindow()`.
fn get_parent_toplevel_window(window: WindowID) -> WindowID {
    let mut toplevel = window;

    // Find the topmost parent
    while is_popup(toplevel) {
        match parent_of(toplevel) {
            Some(p) => toplevel = p,
            None => break,
        }
    }

    toplevel
}

/// Translation of `Cocoa_SetKeyboardFocus()`.
fn set_keyboard_focus(window: Option<WindowID>, set_active_focus: bool) {
    let Some(window) = window else {
        if set_active_focus {
            let _ = crate::events::keyboard::set_keyboard_focus(None);
        }
        return;
    };
    let toplevel = get_parent_toplevel_window(window);
    let _ = with_window(toplevel, |w| w.keyboard_focus = Some(window));

    let (is_hiding, is_destroying) =
        with_window(window, |w| (w.is_hiding, w.core.is_destroying)).unwrap_or((true, true));
    if set_active_focus && !is_hiding && !is_destroying {
        let _ = crate::events::keyboard::set_keyboard_focus(Some(window));
    }
}

/// Translation of `Cocoa_SendExposedEventIfVisible()`.
fn send_exposed_event_if_visible(window: WindowID) {
    let Some(data) = window_data(window) else {
        return;
    };
    // SAFETY: an NSWindow.
    let occlusion: NSUInteger = unsafe { msg_send![data.nswindow(), occlusionState] };
    if occlusion & NSWindowOcclusionStateVisible != 0 {
        send(window, EventType::WINDOW_EXPOSED, 0, 0);
    }
}

/// Translation of `Cocoa_WaitForMiniaturizable()`.
fn wait_for_miniaturizable(window: WindowID) {
    let Some(data) = window_data(window) else {
        return;
    };
    // SAFETY: an NSWindow and its button.
    let button: Id =
        unsafe { msg_send![data.nswindow(), standardWindowButton: NSWindowMiniaturizeButton] };
    if !button.is_null() {
        let mut iterations = 0;
        loop {
            // SAFETY: an NSButton.
            let enabled: Bool = unsafe { msg_send![button, isEnabled] };
            if enabled.is_true() || iterations >= 100 {
                break;
            }
            crate::timer::delay(std::time::Duration::from_millis(10));
            crate::events::pump();
            iterations += 1;
        }
    }
}

/// Translation of `Cocoa_IncrementCursorFrame()`.
pub(crate) fn increment_cursor_frame() {
    let cur_cursor = with_mouse(|m| m.cur_cursor.clone());

    if let Some(cursor) = cur_cursor {
        if let Some(cdata) = mouse::cursor_data(&cursor) {
            cdata.next_frame();

            if let Some(focus) = crate::events::mouse::mouse_focus() {
                if let Some(data) = window_data(focus) {
                    // SAFETY: an NSWindow and its content view.
                    let _: () = unsafe {
                        msg_send![data.nswindow(), invalidateCursorRectsForView: data.sdl_content_view.get()]
                    };
                }
            }
        }
    }
}

/// Translation of `Cocoa_GetDesiredCursor()`.
pub(crate) fn get_desired_cursor() -> Id {
    let (visible, cur_cursor, relative) =
        with_mouse(|m| (m.cursor_visible, m.cur_cursor.clone(), m.relative_mode));

    if visible && !relative {
        if let Some(cursor) = cur_cursor {
            if let Some(cdata) = mouse::cursor_data(&cursor) {
                cdata.schedule_frame_timer();
                return cdata.current_cursor();
            }
        }
    }

    mouse::invisible_cursor()
}

// ---------------------------------------------------------------------------
// SDL3Cocoa_WindowListener

static SDL3_COCOA_WINDOW_LISTENER: DeclaredClass = DeclaredClass::new();

/// The `SDL3Cocoa_WindowListener` class (`@interface
/// SDL3Cocoa_WindowListener : NSResponder <NSWindowDelegate>`), keeping the
/// window ID in `_sdlWindowID`.
pub(crate) fn sdl3_cocoa_window_listener_class() -> Class {
    SDL3_COCOA_WINDOW_LISTENER.get(|| {
        let mut b = ClassBuilder::new(c"SDL3Cocoa_WindowListener", class!(NSResponder))
            .expect("SDL3Cocoa_WindowListener");
        b.add_protocol(c"NSWindowDelegate");
        b.add_ivar::<u32>(c"_sdlWindowID", c"I");

        type Notify = extern "C" fn(Id, Sel, Id);
        let notifications: &[(Sel, Notify)] = &[
            (sel!(windowDidExpose:), listener_window_did_expose),
            (
                sel!(windowDidChangeOcclusionState:),
                listener_window_did_change_occlusion_state,
            ),
            (
                sel!(windowWillStartLiveResize:),
                listener_window_will_start_live_resize,
            ),
            (
                sel!(windowDidEndLiveResize:),
                listener_window_did_end_live_resize,
            ),
            (sel!(windowWillMove:), listener_window_will_move),
            (sel!(windowDidMove:), listener_window_did_move),
            (sel!(windowDidResize:), listener_window_did_resize),
            (
                sel!(windowWillMiniaturize:),
                listener_window_will_miniaturize,
            ),
            (sel!(windowDidMiniaturize:), listener_window_did_miniaturize),
            (
                sel!(windowDidDeminiaturize:),
                listener_window_did_deminiaturize,
            ),
            (sel!(windowDidBecomeKey:), listener_window_did_become_key),
            (sel!(windowDidResignKey:), listener_window_did_resign_key),
            (
                sel!(windowDidChangeBackingProperties:),
                listener_window_did_change_backing_properties,
            ),
            (
                sel!(windowDidChangeScreenProfile:),
                listener_window_did_change_screen_profile,
            ),
            (
                sel!(windowDidChangeScreen:),
                listener_window_did_change_screen,
            ),
            (
                sel!(windowWillEnterFullScreen:),
                listener_window_will_enter_full_screen,
            ),
            (
                sel!(windowDidFailToEnterFullScreen:),
                listener_window_did_fail_to_enter_full_screen,
            ),
            (
                sel!(windowDidEnterFullScreen:),
                listener_window_did_enter_full_screen,
            ),
            (
                sel!(windowWillExitFullScreen:),
                listener_window_will_exit_full_screen,
            ),
            (
                sel!(windowDidFailToExitFullScreen:),
                listener_window_did_fail_to_exit_full_screen,
            ),
            (
                sel!(windowDidExitFullScreen:),
                listener_window_did_exit_full_screen,
            ),
            // Window event handling
            (sel!(flagsChanged:), listener_flags_changed),
            (sel!(keyDown:), listener_key_down),
            (sel!(keyUp:), listener_key_up),
            (sel!(mouseDown:), listener_mouse_down),
            (sel!(rightMouseDown:), listener_mouse_down),
            (sel!(otherMouseDown:), listener_mouse_down),
            (sel!(mouseUp:), listener_mouse_up),
            (sel!(rightMouseUp:), listener_mouse_up),
            (sel!(otherMouseUp:), listener_mouse_up),
            (sel!(mouseMoved:), listener_mouse_moved),
            (sel!(mouseDragged:), listener_mouse_moved),
            (sel!(rightMouseDragged:), listener_mouse_moved),
            (sel!(otherMouseDragged:), listener_mouse_moved),
            (sel!(scrollWheel:), listener_scroll_wheel),
            (sel!(touchesBeganWithEvent:), listener_touches_began),
            (sel!(touchesMovedWithEvent:), listener_touches_moved),
            (sel!(touchesEndedWithEvent:), listener_touches_ended),
            (sel!(touchesCancelledWithEvent:), listener_touches_cancelled),
            (sel!(magnifyWithEvent:), listener_magnify),
            (sel!(tabletProximity:), listener_tablet_event),
            (sel!(tabletPoint:), listener_tablet_event),
        ];

        // SAFETY: the implementations have the signatures of the encodings.
        unsafe {
            for &(sel, f) in notifications {
                b.add_method(sel, imp!(f), c"v@:@");
            }
            b.add_method(
                sel!(observeValueForKeyPath:ofObject:change:context:),
                imp!(listener_observe_value as extern "C" fn(Id, Sel, Id, Id, Id, *mut c_void)),
                c"v@:@@@^v",
            );
            b.add_method(
                sel!(windowShouldClose:),
                imp!(listener_window_should_close as extern "C" fn(Id, Sel, Id) -> Bool),
                &encoding("B@:@"),
            );
            b.add_method(
                sel!(windowWillResize:toSize:),
                imp!(listener_window_will_resize as extern "C" fn(Id, Sel, Id, NSSize) -> NSSize),
                c"{CGSize=dd}@:@{CGSize=dd}",
            );
            b.add_method(
                sel!(window:willUseFullScreenPresentationOptions:),
                imp!(
                    listener_will_use_full_screen_presentation_options
                        as extern "C" fn(Id, Sel, Id, NSUInteger) -> NSUInteger
                ),
                c"Q@:@Q",
            );
            b.add_method(
                sel!(doCommandBySelector:),
                imp!(do_command_by_selector as extern "C" fn(Id, Sel, Sel)),
                c"v@::",
            );
            b.add_method(
                sel!(isMoving),
                imp!(listener_is_moving as extern "C" fn(Id, Sel) -> Bool),
                &encoding("B@:"),
            );
            b.add_method(
                sel!(windowDidFinishMoving),
                imp!(listener_window_did_finish_moving as extern "C" fn(Id, Sel)),
                c"v@:",
            );
        }
        b.register()
    })
}

/// The window data of a listener (`_data`, nil once the window is gone).
fn listener_data(this: Id) -> Option<Arc<WindowData>> {
    // SAFETY: an SDL3Cocoa_WindowListener, which has the variable.
    let id = unsafe { *ivar_ptr::<u32>(this, sdl3_cocoa_window_listener_class(), c"_sdlWindowID") };
    window_data(id)
}

/// The notification names the listener observes when the window has
/// another delegate.
fn observed_notifications() -> Vec<(Sel, Id)> {
    // SAFETY: notification name constants AppKit exports.
    unsafe {
        vec![
            (sel!(windowDidExpose:), NSWindowDidExposeNotification),
            (
                sel!(windowDidChangeOcclusionState:),
                NSWindowDidChangeOcclusionStateNotification,
            ),
            (
                sel!(windowWillStartLiveResize:),
                NSWindowWillStartLiveResizeNotification,
            ),
            (
                sel!(windowDidEndLiveResize:),
                NSWindowDidEndLiveResizeNotification,
            ),
            (sel!(windowWillMove:), NSWindowWillMoveNotification),
            (sel!(windowDidMove:), NSWindowDidMoveNotification),
            (sel!(windowDidResize:), NSWindowDidResizeNotification),
            (
                sel!(windowWillMiniaturize:),
                NSWindowWillMiniaturizeNotification,
            ),
            (
                sel!(windowDidMiniaturize:),
                NSWindowDidMiniaturizeNotification,
            ),
            (
                sel!(windowDidDeminiaturize:),
                NSWindowDidDeminiaturizeNotification,
            ),
            (sel!(windowDidBecomeKey:), NSWindowDidBecomeKeyNotification),
            (sel!(windowDidResignKey:), NSWindowDidResignKeyNotification),
            (
                sel!(windowDidChangeBackingProperties:),
                NSWindowDidChangeBackingPropertiesNotification,
            ),
            (
                sel!(windowDidChangeScreenProfile:),
                NSWindowDidChangeScreenProfileNotification,
            ),
            (
                sel!(windowDidChangeScreen:),
                NSWindowDidChangeScreenNotification,
            ),
            (
                sel!(windowWillEnterFullScreen:),
                NSWindowWillEnterFullScreenNotification,
            ),
            (
                sel!(windowDidEnterFullScreen:),
                NSWindowDidEnterFullScreenNotification,
            ),
            (
                sel!(windowWillExitFullScreen:),
                NSWindowWillExitFullScreenNotification,
            ),
            (
                sel!(windowDidExitFullScreen:),
                NSWindowDidExitFullScreenNotification,
            ),
        ]
    }
}

/// Translation of `-[SDL3Cocoa_WindowListener listen:]`.
fn listen(data: &Arc<WindowData>) {
    let window = data.nswindow();
    let view = data.sdl_content_view.get();
    let listener = data.listener();

    {
        // SAFETY: an NSWindow.
        let visible: Bool = unsafe { msg_send![window, isVisible] };
        let mut ls = data.ls();
        *ls = ListenerState::new();
        ls.observing_visible = true;
        ls.was_visible = visible.is_true();
    }

    // SAFETY: notification and KVO registrations of the listener's
    // methods, on the window it outlives (close removes them).
    unsafe {
        let center: Id = msg_send![class!(NSNotificationCenter), defaultCenter];

        let delegate: Id = msg_send![window, delegate];
        if !delegate.is_null() {
            for (selector, name) in observed_notifications() {
                let _: () = msg_send![center, addObserver: listener
                                                 selector: selector
                                                     name: name
                                                   object: window];
            }
            let fail_enter = ns_string("NSWindowDidFailToEnterFullScreenNotification");
            let _: () = msg_send![center, addObserver: listener
                                             selector: sel!(windowDidFailToEnterFullScreen:)
                                                 name: fail_enter.get()
                                               object: window];
            let fail_exit = ns_string("NSWindowDidFailToExitFullScreenNotification");
            let _: () = msg_send![center, addObserver: listener
                                             selector: sel!(windowDidFailToExitFullScreen:)
                                                 name: fail_exit.get()
                                               object: window];
        } else {
            let _: () = msg_send![window, setDelegate: listener];
        }

        /* Haven't found a delegate / notification that triggers when the window is
         * ordered out (is not visible any more). You can be ordered out without
         * minimizing, so DidMiniaturize doesn't work. (e.g. -[NSWindow orderOut:])
         */
        let visible = ns_string("visible");
        let _: () = msg_send![window, addObserver: listener
                                      forKeyPath: visible.get()
                                         options: NSKeyValueObservingOptionNew
                                         context: std::ptr::null_mut::<c_void>()];

        let _: () = msg_send![window, setNextResponder: listener];
        let _: () = msg_send![window, setAcceptsMouseMovedEvents: YES];

        let _: () = msg_send![view, setNextResponder: listener];

        let _: () = msg_send![view, setAcceptsTouchEvents: YES];
    }
}

/// Translation of `-[SDL3Cocoa_WindowListener observeValueForKeyPath:ofObject:change:context:]`.
extern "C" fn listener_observe_value(
    this: Id,
    _sel: Sel,
    key_path: Id,
    object: Id,
    change: Id,
    _context: *mut c_void,
) {
    let Some(data) = listener_data(this) else {
        return;
    };
    if !data.ls().observing_visible {
        return;
    }

    // SAFETY: a KVO change dictionary of NSNumbers.
    unsafe {
        let visible = ns_string("visible");
        let is_visible_key: Bool = msg_send![key_path, isEqualToString: visible.get()];
        if object == data.nswindow() && is_visible_key.is_true() {
            let new_key = ns_string("new");
            let new_value: Id = msg_send![change, objectForKey: new_key.get()];
            let new_visibility: i32 = msg_send![new_value, intValue];
            if new_visibility != 0 {
                send(data.window, EventType::WINDOW_SHOWN, 0, 0);
            } else {
                let miniaturized: Bool = msg_send![data.nswindow(), isMiniaturized];
                if !miniaturized.is_true() {
                    send(data.window, EventType::WINDOW_HIDDEN, 0, 0);
                }
            }
        }
    }
}

/// Translation of `-[SDL3Cocoa_WindowListener pauseVisibleObservation]`.
fn pause_visible_observation(data: &WindowData) {
    // SAFETY: an NSWindow.
    let visible: Bool = unsafe { msg_send![data.nswindow(), isVisible] };
    let mut ls = data.ls();
    ls.observing_visible = false;
    ls.was_visible = visible.is_true();
}

/// Translation of `-[SDL3Cocoa_WindowListener resumeVisibleObservation]`.
fn resume_visible_observation(data: &WindowData) {
    // SAFETY: an NSWindow.
    let is_visible: Bool = unsafe { msg_send![data.nswindow(), isVisible] };
    let is_visible = is_visible.is_true();
    let was_visible = {
        let mut ls = data.ls();
        ls.observing_visible = true;
        ls.was_visible
    };
    if was_visible != is_visible {
        if is_visible {
            send(data.window, EventType::WINDOW_SHOWN, 0, 0);
        } else {
            // SAFETY: an NSWindow.
            let miniaturized: Bool = unsafe { msg_send![data.nswindow(), isMiniaturized] };
            if !miniaturized.is_true() {
                send(data.window, EventType::WINDOW_HIDDEN, 0, 0);
            }
        }

        data.ls().was_visible = is_visible;
    }
}

/// Translation of `-[SDL3Cocoa_WindowListener setFullscreenSpace:]`.
fn set_fullscreen_space(data: &WindowData, state: bool) -> bool {
    let window = data.window;
    let nswindow = data.nswindow();
    let allow_spaces = data.videodata.state().allow_spaces;
    let (fullscreen_exclusive, last_fullscreen_exclusive_display) = with_window(window, |w| {
        (w.fullscreen_exclusive, w.last_fullscreen_exclusive_display)
    })
    .unwrap_or_default();

    {
        let mut ls = data.ls();
        if !allow_spaces {
            return false; // Spaces are forcibly disabled.
        } else if state && fullscreen_exclusive {
            return false; // we only allow you to make a Space on fullscreen desktop windows.
        } else if !state && last_fullscreen_exclusive_display != 0 {
            return false; // we only handle leaving the Space on windows that were previously fullscreen desktop.
        } else if state == ls.is_fullscreen_space && !ls.in_fullscreen_transition {
            return true; // already there.
        }

        if ls.in_fullscreen_transition {
            if state {
                ls.pending_window_operation &= !PENDING_OPERATION_LEAVE_FULLSCREEN;
                ls.pending_window_operation |= PENDING_OPERATION_ENTER_FULLSCREEN;
            } else {
                ls.pending_window_operation &= !PENDING_OPERATION_ENTER_FULLSCREEN;
                ls.pending_window_operation |= PENDING_OPERATION_LEAVE_FULLSCREEN;
            }
            return true;
        }
        ls.in_fullscreen_transition = true;
    }

    // you need to be FullScreenPrimary, or toggleFullScreen doesn't work. Unset it again in windowDidExitFullScreen.
    // SAFETY: an NSWindow.
    unsafe {
        let _: () =
            msg_send![nswindow, setCollectionBehavior: NSWindowCollectionBehaviorFullScreenPrimary];
        let _: () = msg_send![nswindow, performSelectorOnMainThread: sel!(toggleFullScreen:)
                                                         withObject: nswindow
                                                      waitUntilDone: NO];
    }
    true
}

/// Translation of `-[SDL3Cocoa_WindowListener close]`.
fn close_listener(data: &WindowData) {
    let window = data.nswindow();
    let listener = data.listener();
    // SAFETY: undoes the registrations of listen().
    unsafe {
        let view: Id = msg_send![window, contentView];

        let center: Id = msg_send![class!(NSNotificationCenter), defaultCenter];

        let delegate: Id = msg_send![window, delegate];
        if delegate != listener {
            for (_, name) in observed_notifications() {
                let _: () = msg_send![center, removeObserver: listener name: name object: window];
            }
            let fail_enter = ns_string("NSWindowDidFailToEnterFullScreenNotification");
            let _: () =
                msg_send![center, removeObserver: listener name: fail_enter.get() object: window];
            let fail_exit = ns_string("NSWindowDidFailToExitFullScreenNotification");
            let _: () =
                msg_send![center, removeObserver: listener name: fail_exit.get() object: window];
        } else {
            let _: () = msg_send![window, setDelegate: NIL];
        }

        let visible = ns_string("visible");
        let _: () = msg_send![window, removeObserver: listener forKeyPath: visible.get()];

        if next_responder(window) == listener {
            set_next_responder(window, NIL);
        }
        if !view.is_null() && next_responder(view) == listener {
            set_next_responder(view, NIL);
        }
    }
}

/// Translation of `-[SDL3Cocoa_WindowListener isMoving]`.
extern "C" fn listener_is_moving(this: Id, _sel: Sel) -> Bool {
    Bool::from(listener_data(this).is_some_and(|d| d.is_moving()))
}

/// Translation of `-[SDL3Cocoa_WindowListener windowDidFinishMoving]`.
extern "C" fn listener_window_did_finish_moving(this: Id, _sel: Sel) {
    let Some(data) = listener_data(this) else {
        return;
    };
    let was_moving = std::mem::replace(&mut data.ls().is_moving, false);
    if was_moving {
        on_moving_or_focus_click_pending_state_cleared(&data);
    }
}

/// Translation of `-[SDL3Cocoa_WindowListener updateIgnoreMouseState:]`.
fn update_ignore_mouse_state(data: &WindowData, the_event: Id) {
    let window = data.window;
    let shape = Window::from_raw(window)
        .properties()
        .ok()
        .and_then(|p| p.get_any::<Surface<'static>>(PROP_WINDOW_SHAPE_POINTER));
    let mut ignores_mouse_events = NO;

    if let Some(shape) = shape {
        // SAFETY: an NSEvent and the window's content view.
        unsafe {
            let point: NSPoint = msg_send![the_event, locationInWindow];
            let content_view: Id = msg_send![data.nswindow(), contentView];
            let window_rect: NSRect = msg_send![content_view, frame];
            let in_rect: Bool = msg_send![content_view, mouse: point inRect: window_rect];
            if in_rect.is_true() {
                let (_, _, w, h) = geometry(window);
                let x =
                    ((point.x as f32 / (w - 1) as f32) * (shape.width() - 1) as f32).round() as i32;
                let y = (((h as f64 - point.y) as f32 / (h - 1) as f32)
                    * (shape.height() - 1) as f32)
                    .round() as i32;

                match shape.read_pixel(x, y) {
                    Ok(c) if c.a != 0 => {}
                    _ => ignores_mouse_events = YES,
                }
            }
        }
    }
    // SAFETY: an NSWindow.
    let _: () = unsafe { msg_send![data.nswindow(), setIgnoresMouseEvents: ignores_mouse_events] };
}

/// Translation of `-[SDL3Cocoa_WindowListener onMovingOrFocusClickPendingStateCleared]`.
fn on_moving_or_focus_click_pending_state_cleared(data: &Arc<WindowData>) {
    if !data.is_moving_or_focus_click_pending() {
        let (x, y) = {
            let mut ls = data.ls();
            let xy = (ls.pending_window_warp_x, ls.pending_window_warp_y);
            if xy.0 != f32::MAX && xy.1 != f32::MAX {
                ls.pending_window_warp_x = f32::MAX;
                ls.pending_window_warp_y = f32::MAX;
            }
            xy
        };
        if x != f32::MAX && y != f32::MAX {
            let _ = mouse::warp_mouse_global(x, y);
        }
        let (relative_mode, focus) = with_mouse(|m| (m.relative_mode, m.focus));
        if relative_mode && focus == Some(data.window) {
            // Move the cursor to the nearest point in the window
            {
                let (x, y, _) = crate::events::mouse::mouse_state();
                let (wx, wy, _, _) = geometry(data.window);
                let cgpoint = NSPoint::new(wx as f64 + x as f64, wy as f64 + y as f64);

                mouse::handle_mouse_warp(cgpoint.x, cgpoint.y);

                // DLog("Returning cursor to (%g, %g)", cgpoint.x, cgpoint.y);
                // SAFETY: a point on the main display.
                unsafe { CGDisplayMoveCursorToPoint(kCGDirectMainDisplay(), cgpoint) };
            }

            let _ = mouse::set_relative_mouse_mode(true);
        } else {
            update_clip_cursor(data.window);
        }
    }
}

/// Translation of `-[SDL3Cocoa_WindowListener windowShouldClose:]`.
extern "C" fn listener_window_should_close(this: Id, _sel: Sel, _sender: Id) -> Bool {
    if let Some(data) = listener_data(this) {
        send(data.window, EventType::WINDOW_CLOSE_REQUESTED, 0, 0);
    }
    NO
}

/// Translation of `-[SDL3Cocoa_WindowListener windowDidExpose:]`.
extern "C" fn listener_window_did_expose(this: Id, _sel: Sel, _notification: Id) {
    if let Some(data) = listener_data(this) {
        send_exposed_event_if_visible(data.window);
    }
}

/// Translation of `-[SDL3Cocoa_WindowListener windowDidChangeOcclusionState:]`.
extern "C" fn listener_window_did_change_occlusion_state(this: Id, _sel: Sel, _notification: Id) {
    let Some(data) = listener_data(this) else {
        return;
    };
    // SAFETY: an NSWindow.
    let occlusion: NSUInteger = unsafe { msg_send![data.nswindow(), occlusionState] };
    if occlusion & NSWindowOcclusionStateVisible != 0 {
        send(data.window, EventType::WINDOW_EXPOSED, 0, 0);
    } else {
        send(data.window, EventType::WINDOW_OCCLUDED, 0, 0);
    }
}

/// Translation of `-[SDL3Cocoa_WindowListener windowWillStartLiveResize:]`.
extern "C" fn listener_window_will_start_live_resize(this: Id, _sel: Sel, _notification: Id) {
    let Some(data) = listener_data(this) else {
        return;
    };
    // We'll try to maintain 60 FPS during live resizing
    let interval = 1.0f64 / 60.0;
    let window = data.window;
    let block = RcBlock::new1(move |_unused_timer: Id| {
        crate::video::core::on_window_live_resize_update(window);
    });
    // SAFETY: NSTimer and the current run loop; the timer keeps its block.
    unsafe {
        let timer: Id = msg_send![class!(NSTimer), scheduledTimerWithTimeInterval: interval
                                                                           repeats: YES
                                                                             block: block.as_ptr()];
        let run_loop: Id = msg_send![class!(NSRunLoop), currentRunLoop];
        let _: () = msg_send![run_loop, addTimer: timer forMode: NSRunLoopCommonModes];
        data.ls().live_resize_timer = StrongPtr::retain(timer);
    }
}

/// Translation of `-[SDL3Cocoa_WindowListener windowDidEndLiveResize:]`.
extern "C" fn listener_window_did_end_live_resize(this: Id, _sel: Sel, _notification: Id) {
    let Some(data) = listener_data(this) else {
        return;
    };
    let timer = std::mem::take(&mut data.ls().live_resize_timer);
    // SAFETY: an NSTimer (or nil).
    let _: () = unsafe { msg_send![timer, invalidate] };
}

/// Translation of `-[SDL3Cocoa_WindowListener windowWillMove:]`.
extern "C" fn listener_window_will_move(this: Id, _sel: Sel, _notification: Id) {
    let Some(data) = listener_data(this) else {
        return;
    };
    // SAFETY: an NSWindow.
    if unsafe { is_kind_of(data.nswindow(), sdl3_window_class()) } {
        let mut ls = data.ls();
        ls.pending_window_warp_x = f32::MAX;
        ls.pending_window_warp_y = f32::MAX;
        ls.is_moving = true;
    }
}

/// The content rectangle of the window in SDL coordinates.
fn content_rect(nswindow: Id) -> NSRect {
    // SAFETY: an NSWindow.
    let mut rect: NSRect = unsafe {
        let frame: NSRect = msg_send![nswindow, frame];
        msg_send![nswindow, contentRectForFrameRect: frame]
    };
    convert_ns_rect(&mut rect);
    rect
}

/// Translation of `-[SDL3Cocoa_WindowListener windowDidMove:]`.
fn window_did_move(data: &WindowData) {
    let window = data.window;
    let rect = content_rect(data.nswindow());

    if data.ls().in_fullscreen_transition || in_mode_transition() {
        // We'll take care of this at the end of the transition
        return;
    }

    let x = rect.origin.x as i32;
    let y = rect.origin.y as i32;

    schedule_context_updates(data);

    // Get the parent-relative coordinates for child windows.
    let (x, y) = global_to_relative_for_window(window, x, y);
    send(window, EventType::WINDOW_MOVED, x, y);
}

extern "C" fn listener_window_did_move(this: Id, _sel: Sel, _notification: Id) {
    if let Some(data) = listener_data(this) {
        window_did_move(&data);
    }
}

/// Translation of `-[SDL3Cocoa_WindowListener windowWillResize:toSize:]`.
extern "C" fn listener_window_will_resize(
    this: Id,
    _sel: Sel,
    _sender: Id,
    mut frame_size: NSSize,
) -> NSSize {
    let Some(data) = listener_data(this) else {
        return frame_size;
    };
    let window = data.window;
    let (min_aspect, max_aspect) =
        with_window(window, |w| (w.min_aspect, w.max_aspect)).unwrap_or_default();

    if min_aspect != max_aspect {
        let nswindow = data.nswindow();
        // SAFETY: an NSWindow.
        let new_content_rect: NSRect = unsafe {
            msg_send![nswindow, contentRectForFrameRect:
                NSRect::new(0.0, 0.0, frame_size.width, frame_size.height)]
        };
        let mut new_size = new_content_rect.size;
        let min_aspect_ratio = min_aspect as f64;
        let max_aspect_ratio = max_aspect as f64;

        if new_size.height > 0.0 {
            let aspect_ratio = new_size.width / new_size.height;

            if max_aspect_ratio > 0.0 && aspect_ratio > max_aspect_ratio {
                new_size.width = ((new_size.height * max_aspect_ratio) as f32).round() as f64;
            } else if min_aspect_ratio > 0.0 && aspect_ratio < min_aspect_ratio {
                new_size.height = ((new_size.width / min_aspect_ratio) as f32).round() as f64;
            }

            // SAFETY: an NSWindow.
            let new_frame_rect: NSRect = unsafe {
                msg_send![nswindow, frameRectForContentRect:
                    NSRect::new(0.0, 0.0, new_size.width, new_size.height)]
            };
            frame_size = new_frame_rect.size;
        }
    }
    frame_size
}

/// Translation of `-[SDL3Cocoa_WindowListener windowDidResize:]`.
fn window_did_resize(data: &Arc<WindowData>) {
    if data.ls().in_fullscreen_transition || in_mode_transition() {
        // We'll take care of this at the end of the transition
        return;
    }

    let had_focus_click = std::mem::take(&mut data.ls().focus_click_pending) != 0;
    if had_focus_click {
        on_moving_or_focus_click_pending_state_cleared(data);
    }
    let window = data.window;
    let nswindow = data.nswindow();
    let rect = content_rect(nswindow);
    let x = rect.origin.x as i32;
    let y = rect.origin.y as i32;
    let w = rect.size.width as i32;
    let h = rect.size.height as i32;

    let f = flags(window);
    // SAFETY: the content view.
    let viewport: NSRect = unsafe {
        let bounds: NSRect = msg_send![data.sdl_content_view, bounds];
        if f.contains(WindowFlags::HIGH_PIXEL_DENSITY) {
            // This gives us the correct viewport for a Retina-enabled view.
            msg_send![data.sdl_content_view, convertRectToBacking: bounds]
        } else {
            bounds
        }
    };
    data.state().viewport = viewport;

    schedule_context_updates(data);

    /* The OS can resize the window automatically if the display density
     *  changes while the window is miniaturized or hidden.
     */
    // SAFETY: an NSWindow.
    let visible: Bool = unsafe { msg_send![nswindow, isVisible] };
    if visible.is_true() {
        /* isZoomed always returns true if the window is not resizable
         * and fullscreen windows are considered zoomed.
         */
        // SAFETY: an NSWindow.
        let is_zoomed: Bool = unsafe { msg_send![nswindow, isZoomed] };
        let zoomed = f.contains(WindowFlags::RESIZABLE)
            && is_zoomed.is_true()
            && !f.contains(WindowFlags::FULLSCREEN)
            && !data.is_in_fullscreen_space();
        if !zoomed {
            send(window, EventType::WINDOW_RESTORED, 0, 0);
        } else {
            send(window, EventType::WINDOW_MAXIMIZED, 0, 0);
            if data.window_operation_is_pending(PENDING_OPERATION_MINIMIZE) {
                // SAFETY: an NSWindow.
                let _: () = unsafe { msg_send![nswindow, miniaturize: NIL] };
            }
        }
    }

    /* The window can move during a resize event, such as when maximizing
    or resizing from a corner */
    let (x, y) = global_to_relative_for_window(window, x, y);
    send(window, EventType::WINDOW_MOVED, x, y);
    send(window, EventType::WINDOW_RESIZED, w, h);
}

extern "C" fn listener_window_did_resize(this: Id, _sel: Sel, _notification: Id) {
    if let Some(data) = listener_data(this) {
        window_did_resize(&data);
    }
}

/// Translation of `-[SDL3Cocoa_WindowListener windowWillMiniaturize:]`.
extern "C" fn listener_window_will_miniaturize(this: Id, _sel: Sel, _notification: Id) {
    let Some(data) = listener_data(this) else {
        return;
    };
    data.ls().is_miniaturizing = true;
    wait_for_miniaturizable(data.window);
}

/// Translation of `-[SDL3Cocoa_WindowListener windowDidMiniaturize:]`.
extern "C" fn listener_window_did_miniaturize(this: Id, _sel: Sel, _notification: Id) {
    let Some(data) = listener_data(this) else {
        return;
    };
    let had_focus_click = std::mem::take(&mut data.ls().focus_click_pending) != 0;
    if had_focus_click {
        on_moving_or_focus_click_pending_state_cleared(&data);
    }
    data.ls().is_miniaturizing = false;
    data.clear_pending_window_operation(PENDING_OPERATION_MINIMIZE);
    send(data.window, EventType::WINDOW_MINIMIZED, 0, 0);
}

/// Translation of `-[SDL3Cocoa_WindowListener windowDidDeminiaturize:]`.
extern "C" fn listener_window_did_deminiaturize(this: Id, _sel: Sel, _notification: Id) {
    let Some(data) = listener_data(this) else {
        return;
    };
    // Always send restored before maximized.
    send(data.window, EventType::WINDOW_RESTORED, 0, 0);

    if is_window_zoomed(data.window) {
        send(data.window, EventType::WINDOW_MAXIMIZED, 0, 0);
    }

    if data.window_operation_is_pending(PENDING_OPERATION_ENTER_FULLSCREEN) {
        let _ = crate::video::core::update_fullscreen_mode(data.window, FullscreenOp::Enter, true);
    }
}

/// Translation of `-[SDL3Cocoa_WindowListener windowDidBecomeKey:]`.
extern "C" fn listener_window_did_become_key(this: Id, _sel: Sel, _notification: Id) {
    let Some(data) = listener_data(this) else {
        return;
    };
    let window = data.window;

    // We're going to get keyboard events, since we're key.
    // This needs to be done before restoring the relative mouse mode.
    let keyboard_focus = with_window(window, |w| w.keyboard_focus).ok().flatten();
    set_keyboard_focus(Some(keyboard_focus.unwrap_or(window)), true);

    // If we just gained focus we need the updated mouse position
    let f = flags(window);
    if !f.contains(WindowFlags::MOUSE_RELATIVE_MODE) {
        // SAFETY: an NSWindow.
        let point: NSPoint =
            unsafe { msg_send![data.nswindow(), mouseLocationOutsideOfEventStream] };
        let (_, _, w, h) = geometry(window);
        let x = point.x as f32;
        let y = (h as f64 - point.y) as f32;

        if x >= 0.0 && x < w as f32 && y >= 0.0 && y < h as f32 {
            send_mouse_motion(
                std::time::Duration::ZERO,
                Some(window),
                GLOBAL_MOUSE_ID,
                false,
                x,
                y,
            );
        }
    }

    // Check to see if someone updated the clipboard
    super::clipboard::check_clipboard_update(&data.videodata);

    let fullscreen_exclusive = with_window(window, |w| w.fullscreen_exclusive).unwrap_or(false);
    if data.is_in_fullscreen_space() && !fullscreen_exclusive {
        toggle_fullscreen_space_menu_visibility(Some(window));
    }
    {
        // SAFETY: NSEvent's class method.
        let modifier_flags: NSUInteger = unsafe { msg_send![class!(NSEvent), modifierFlags] };
        let newflags = (modifier_flags & NSEventModifierFlagCapsLock) as u32;
        {
            let mut st = data.videodata.state();
            st.modifier_flags =
                (st.modifier_flags & !(NSEventModifierFlagCapsLock as u32)) | newflags;
        }
        crate::events::keyboard::toggle_mod_state(
            crate::events::keyboard::Keymod::CAPS,
            newflags != 0,
        );
    }

    /* Restore fullscreen mode unless the window is deminiaturizing.
     * If it is, fullscreen will be restored when deminiaturization is complete.
     */
    if !flags(window).contains(WindowFlags::MINIMIZED)
        && data.window_operation_is_pending(PENDING_OPERATION_ENTER_FULLSCREEN)
    {
        let _ = crate::video::core::update_fullscreen_mode(window, FullscreenOp::Enter, true);
    }
}

/// Translation of `-[SDL3Cocoa_WindowListener windowDidResignKey:]`.
extern "C" fn listener_window_did_resign_key(this: Id, _sel: Sel, _notification: Id) {
    let Some(data) = listener_data(this) else {
        return;
    };
    // Some other window will get mouse events, since we're not key.
    if crate::events::mouse::mouse_focus() == Some(data.window) {
        set_mouse_focus(None);
    }

    // Some other window will get keyboard events, since we're not key.
    if crate::events::keyboard::keyboard_focus() == Some(data.window) {
        let _ = crate::events::keyboard::set_keyboard_focus(None);
    }

    if data.is_in_fullscreen_space() {
        set_menu_bar_visible(true);
    }
}

/// Translation of `-[SDL3Cocoa_WindowListener windowDidChangeBackingProperties:]`.
extern "C" fn listener_window_did_change_backing_properties(this: Id, _sel: Sel, notification: Id) {
    let Some(data) = listener_data(this) else {
        return;
    };
    let content_view = data.sdl_content_view.get();
    // SAFETY: the notification's user info dictionary of NSNumbers.
    let oldscale: Id = unsafe {
        let user_info: Id = msg_send![notification, userInfo];
        msg_send![user_info, objectForKey: NSBackingPropertyOldScaleFactorKey]
    };

    if data.ls().in_fullscreen_transition {
        return;
    }

    // SAFETY: an NSNumber (nil gives 0), an NSWindow and a view's layer.
    unsafe {
        let old: f64 = msg_send![oldscale, doubleValue];
        let backing: f64 = msg_send![data.nswindow(), backingScaleFactor];
        if old != backing {
            // Update the content scale on the window layer
            // This is required to keep content scale in sync with ANGLE
            let layer: Id = msg_send![content_view, layer];
            let _: () = msg_send![layer, setContentsScale: backing];
            // Send a resize event when the backing scale factor changes.
            window_did_resize(&data);
        }
    }
}

/// Translation of `-[SDL3Cocoa_WindowListener windowDidChangeScreenProfile:]`.
extern "C" fn listener_window_did_change_screen_profile(this: Id, _sel: Sel, _notification: Id) {
    if let Some(data) = listener_data(this) {
        send(data.window, EventType::WINDOW_ICCPROF_CHANGED, 0, 0);
    }
}

/// Translation of `-[SDL3Cocoa_WindowListener windowDidChangeScreen:]`.
extern "C" fn listener_window_did_change_screen(this: Id, _sel: Sel, _notification: Id) {
    // printf("WINDOWDIDCHANGESCREEN\n");
    let Some(data) = listener_data(this) else {
        return;
    };
    let contexts: Vec<StrongPtr> = data.state().nscontexts.clone();
    for context in &contexts {
        // SAFETY: an SDL3OpenGLContext.
        let _: () = unsafe { msg_send![context, movedToNewScreen] };
    }
}

/// Translation of `-[SDL3Cocoa_WindowListener windowWillEnterFullScreen:]`.
extern "C" fn listener_window_will_enter_full_screen(this: Id, _sel: Sel, _notification: Id) {
    let Some(data) = listener_data(this) else {
        return;
    };
    let window = data.window;
    let flags_mask = NSWindowStyleMaskClosable
        | NSWindowStyleMaskMiniaturizable
        | NSWindowStyleMaskResizable
        | NSWindowStyleMaskTitled;

    /* For some reason, the fullscreen window won't get any mouse button events
     * without the NSWindowStyleMaskTitled flag being set when entering fullscreen,
     * so it's needed even if the window is borderless.
     */
    set_window_style(&data, flags_mask);

    data.state().was_zoomed = flags(window).contains(WindowFlags::MAXIMIZED);

    let mut ls = data.ls();
    ls.is_fullscreen_space = true;
    ls.in_fullscreen_transition = true;
}

/// This is usually sent after an unexpected windowDidExitFullscreen if the window
/// failed to become fullscreen, but not always, so ensure that the state variables
/// are cleared.
///
/// Since something went wrong and the current state is unknown, dump any pending events.
///
/// For testing purposes, this error can usually be induced by starting something that
/// immedately enters a fullscreen space from a terminal that is in a fullscreen space.
///
/// Translation of `-[SDL3Cocoa_WindowListener windowDidFailToEnterFullScreen:]`.
extern "C" fn listener_window_did_fail_to_enter_full_screen(
    this: Id,
    _sel: Sel,
    _notification: Id,
) {
    let Some(data) = listener_data(this) else {
        return;
    };
    let mut ls = data.ls();
    ls.in_fullscreen_transition = false;
    ls.is_fullscreen_space = false;
    ls.pending_window_operation = PENDING_OPERATION_NONE;
}

/// Force the move and resize events after a fullscreen transition.
fn force_size_change_events(data: &Arc<WindowData>) {
    /* Force the size change event in case it was delivered earlier
      while the window was still animating into place.
    */
    let _ = with_window(data.window, |w| {
        w.core.w = 0;
        w.core.h = 0;
    });
    window_did_move(data);
    window_did_resize(data);
}

/// Translation of `-[SDL3Cocoa_WindowListener windowDidEnterFullScreen:]`.
extern "C" fn listener_window_did_enter_full_screen(this: Id, _sel: Sel, _notification: Id) {
    let Some(data) = listener_data(this) else {
        return;
    };
    let window = data.window;

    {
        let mut ls = data.ls();
        ls.in_fullscreen_transition = false;
        ls.is_fullscreen_space = true;
        ls.pending_window_operation &= !PENDING_OPERATION_ENTER_FULLSCREEN;
    }

    if data.window_operation_is_pending(PENDING_OPERATION_LEAVE_FULLSCREEN) {
        set_fullscreen_space(&data, false);
    } else {
        toggle_fullscreen_space_menu_visibility(Some(window));

        /* Don't recurse back into UpdateFullscreenMode() if this was hit in
         * a blocking transition, as the caller is already waiting in
         * UpdateFullscreenMode().
         */
        if !data.state().in_blocking_transition {
            let _ = crate::video::core::update_fullscreen_mode(window, FullscreenOp::Enter, false);
        }
        send(window, EventType::WINDOW_ENTER_FULLSCREEN, 0, 0);

        data.state().pending_size = false;

        force_size_change_events(&data);

        update_clip_cursor(window);
    }
}

/// Translation of `-[SDL3Cocoa_WindowListener windowWillExitFullScreen:]`.
extern "C" fn listener_window_will_exit_full_screen(this: Id, _sel: Sel, _notification: Id) {
    let Some(data) = listener_data(this) else {
        return;
    };
    let window = data.window;

    /* If the windowed mode borders were toggled on while in a fullscreen space,
     * NSWindowStyleMaskTitled has to be cleared here, or the window can end up
     * in a weird, semi-decorated state upon returning to windowed mode.
     */
    let border_toggled = data.state().border_toggled;
    if border_toggled && !flags(window).contains(WindowFlags::BORDERLESS) {
        let flags_mask = NSWindowStyleMaskClosable
            | NSWindowStyleMaskMiniaturizable
            | NSWindowStyleMaskResizable;

        set_window_style(&data, flags_mask);
        data.state().border_toggled = false;
    }

    let mut ls = data.ls();
    ls.is_fullscreen_space = false;
    ls.in_fullscreen_transition = true;
}

/// This may be sent before windowDidExitFullscreen to signal that the window was
/// dumped out of fullscreen with no animation.
///
/// Since something went wrong and the state is unknown, dump any pending events.
///
/// Translation of `-[SDL3Cocoa_WindowListener windowDidFailToExitFullScreen:]`.
extern "C" fn listener_window_did_fail_to_exit_full_screen(this: Id, _sel: Sel, _notification: Id) {
    if let Some(data) = listener_data(this) {
        data.clear_all_pending_window_operations();
    }
}

/// Translation of `-[SDL3Cocoa_WindowListener windowDidExitFullScreen:]`.
extern "C" fn listener_window_did_exit_full_screen(this: Id, _sel: Sel, _notification: Id) {
    let Some(data) = listener_data(this) else {
        return;
    };
    let window = data.window;
    let nswindow = data.nswindow();

    {
        let mut ls = data.ls();
        ls.in_fullscreen_transition = false;
        ls.is_fullscreen_space = false;
    }
    data.state().fullscreen_space_requested = false;

    /* As of macOS 10.15, the window decorations can go missing sometimes after
    certain fullscreen-desktop->exclusive-fullscreen->windowed mode flows
    sometimes. Making sure the style mask always uses the windowed mode style
    when returning to windowed mode from a space (instead of using a pending
    fullscreen mode style mask) seems to work around that issue.
     */
    set_window_style(&data, get_window_windowed_style(window));

    /* This can happen if the window failed to enter fullscreen, as this
     * may be called *before* windowDidFailToEnterFullScreen in that case.
     */
    if !flags(window).contains(WindowFlags::FULLSCREEN) {
        data.clear_all_pending_window_operations();
    }

    /* Don't recurse back into UpdateFullscreenMode() if this was hit in
     * a blocking transition, as the caller is already waiting in
     * UpdateFullscreenMode().
     */
    send(window, EventType::WINDOW_LEAVE_FULLSCREEN, 0, 0);
    if !data.state().in_blocking_transition {
        let _ = crate::video::core::update_fullscreen_mode(window, FullscreenOp::Leave, false);
    }

    // SAFETY: an NSWindow.
    unsafe {
        if flags(window).contains(WindowFlags::ALWAYS_ON_TOP) {
            let _: () = msg_send![nswindow, setLevel: NSFloatingWindowLevel];
        } else {
            let _: () = msg_send![nswindow, setLevel: kCGNormalWindowLevel];
        }
    }

    data.clear_pending_window_operation(PENDING_OPERATION_LEAVE_FULLSCREEN);

    if data.window_operation_is_pending(PENDING_OPERATION_ENTER_FULLSCREEN) {
        set_fullscreen_space(&data, true);
    } else if data.window_operation_is_pending(PENDING_OPERATION_MINIMIZE) {
        /* There's some state that isn't quite back to normal when
         * windowDidExitFullScreen triggers. For example, the minimize button on
         * the title bar doesn't actually enable for another 200 milliseconds or
         * so on this MacBook. Camp here and wait for that to happen before
         * going on, in case we're exiting fullscreen to minimize, which need
         * that window state to be normal before it will work.
         */
        wait_for_miniaturizable(window);
        data.add_pending_window_operation(PENDING_OPERATION_ENTER_FULLSCREEN);
        // SAFETY: an NSWindow.
        let _: () = unsafe { msg_send![nswindow, miniaturize: NIL] };
    } else {
        // SAFETY: an NSWindow.
        unsafe {
            // Adjust the fullscreen toggle button and re-add menu now that we're here.
            if flags(window).contains(WindowFlags::RESIZABLE) {
                // resizable windows are Spaces-friendly: they get the "go fullscreen" toggle button on their titlebar.
                let _: () = msg_send![nswindow, setCollectionBehavior: NSWindowCollectionBehaviorFullScreenPrimary];
            } else {
                let _: () =
                    msg_send![nswindow, setCollectionBehavior: NSWindowCollectionBehaviorManaged];
            }
        }

        if !menu_bar_visible() {
            set_menu_bar_visible(true);
        }

        // Toggle zoom, if changed while fullscreen.
        if data.window_operation_is_pending(PENDING_OPERATION_ZOOM) {
            data.clear_pending_window_operation(PENDING_OPERATION_ZOOM);
            // SAFETY: an NSWindow.
            let _: () = unsafe { msg_send![nswindow, zoom: NIL] };
            let mut st = data.state();
            st.was_zoomed = !st.was_zoomed;
        }

        let (was_zoomed, pending_position, pending_size) = {
            let st = data.state();
            (st.was_zoomed, st.pending_position, st.pending_size)
        };
        if !was_zoomed {
            // Apply a pending window size, if not zoomed.
            let (pending, floating) =
                with_window(window, |w| (w.pending, w.core.floating)).unwrap_or_default();
            let mut rect = NSRect::new(
                if pending_position {
                    pending.x
                } else {
                    floating.x
                } as f64,
                if pending_position {
                    pending.y
                } else {
                    floating.y
                } as f64,
                if pending_size { pending.w } else { floating.w } as f64,
                if pending_size { pending.h } else { floating.h } as f64,
            );
            convert_ns_rect(&mut rect);

            // SAFETY: an NSWindow.
            unsafe {
                if pending_size {
                    let _: () = msg_send![nswindow, setContentSize: rect.size];
                }
                if pending_position {
                    let _: () = msg_send![nswindow, setFrameOrigin: rect.origin];
                }
            }
        }

        {
            let mut st = data.state();
            st.pending_size = false;
            st.pending_position = false;
            st.was_zoomed = false;
        }

        force_size_change_events(&data);

        // FIXME: Why does the window get hidden?
        if !flags(window).contains(WindowFlags::HIDDEN) {
            show_window(window);
        }

        update_clip_cursor(window);
    }
}

/// Translation of `-[SDL3Cocoa_WindowListener window:willUseFullScreenPresentationOptions:]`.
extern "C" fn listener_will_use_full_screen_presentation_options(
    this: Id,
    _sel: Sel,
    _window: Id,
    proposed_options: NSUInteger,
) -> NSUInteger {
    let exclusive = listener_data(this)
        .and_then(|d| with_window(d.window, |w| w.fullscreen_exclusive).ok())
        .unwrap_or(false);
    if exclusive {
        NSApplicationPresentationFullScreen
            | NSApplicationPresentationHideDock
            | NSApplicationPresentationHideMenuBar
    } else {
        proposed_options
    }
}

/// We'll respond to key events by mostly doing nothing so we don't beep.
/// We could handle key messages here, but we lose some in the NSApp dispatch,
/// where they get converted to action messages, etc.
///
/// Translation of `-[SDL3Cocoa_WindowListener flagsChanged:]`.
extern "C" fn listener_flags_changed(_this: Id, _sel: Sel, the_event: Id) {
    // Cocoa_HandleKeyEvent(SDL_GetVideoDevice(), theEvent);

    /* Catch capslock in here as a special case:
    https://developer.apple.com/library/archive/qa/qa1519/_index.html
    Note that technote's check of keyCode doesn't work. At least on the
    10.15 beta, capslock comes through here as keycode 255, but it's safe
    to send duplicate key events; SDL filters them out quickly in
    SDL_SendKeyboardKey(). */

    /* Also note that SDL_SendKeyboardKey expects all capslock events to be
    keypresses; it won't toggle the mod state if you send a keyrelease.  */
    // SAFETY: an NSEvent.
    let modifier_flags: NSUInteger = unsafe { msg_send![the_event, modifierFlags] };
    let osenabled = (modifier_flags & NSEventModifierFlagCapsLock) != 0;
    let sdlenabled =
        crate::events::keyboard::mod_state().contains(crate::events::keyboard::Keymod::CAPS);
    if osenabled ^ sdlenabled {
        let keyboard = crate::events::keyboard::DEFAULT_KEYBOARD_ID;
        let caps = crate::events::keyboard::Scancode::CAPSLOCK;
        crate::events::keyboard::send_keyboard_key(
            std::time::Duration::ZERO,
            keyboard,
            0,
            caps,
            true,
        );
        crate::events::keyboard::send_keyboard_key(
            std::time::Duration::ZERO,
            keyboard,
            0,
            caps,
            false,
        );
    }
}

/// Translation of `-[SDL3Cocoa_WindowListener keyDown:]`.
extern "C" fn listener_key_down(_this: Id, _sel: Sel, _the_event: Id) {
    // Cocoa_HandleKeyEvent(SDL_GetVideoDevice(), theEvent);
}

/// Translation of `-[SDL3Cocoa_WindowListener keyUp:]`.
extern "C" fn listener_key_up(_this: Id, _sel: Sel, _the_event: Id) {
    // Cocoa_HandleKeyEvent(SDL_GetVideoDevice(), theEvent);
}

/// Translation of `-[SDL3Cocoa_WindowListener updateHitTest]`.
fn update_hit_test(data: &WindowData) {
    let window = data.window;
    let mut draggable = false;

    let hit_test = with_window(window, |w| w.hit_test.clone()).ok().flatten();
    if let Some(hit_test) = hit_test {
        let (x, y, _) = crate::events::mouse::global_mouse_state();
        let (wx, wy, ww, wh) = geometry(window);
        let point = Point::new(
            (x - wx as f32).round() as i32,
            (y - wy as f32).round() as i32,
        );
        if point.x >= 0
            && point.x < ww
            && point.y >= 0
            && point.y < wh
            && hit_test(window, point) == HitTestResult::Draggable
        {
            draggable = true;
        }
    }

    let changed = {
        let mut ls = data.ls();
        let changed = ls.is_drag_area_running != draggable;
        ls.is_drag_area_running = draggable;
        changed
    };
    if changed {
        // SAFETY: an NSWindow.
        let _: () = unsafe {
            msg_send![data.nswindow(), setMovableByWindowBackground: Bool::from(draggable)]
        };
    }
}

/// Translation of `-[SDL3Cocoa_WindowListener processHitTest:]`.
fn process_hit_test(data: &WindowData, the_event: Id) -> bool {
    let window = data.window;

    let hit_test = with_window(window, |w| w.hit_test.clone()).ok().flatten();
    if let Some(hit_test) = hit_test {
        // if no hit-test, skip this.
        // SAFETY: an NSEvent.
        let location: NSPoint = unsafe { msg_send![the_event, locationInWindow] };
        let (_, _, _, h) = geometry(window);
        let point = Point::new(location.x as i32, h - ((location.y as i32) - 1));
        let rc = hit_test(window, point);
        if rc == HitTestResult::Draggable {
            let start = !std::mem::replace(&mut data.ls().is_drag_area_running, true);
            if start {
                // SAFETY: an NSWindow.
                let _: () =
                    unsafe { msg_send![data.nswindow(), setMovableByWindowBackground: YES] };
            }
            return true; // dragging!
        } else {
            let was_running = std::mem::replace(&mut data.ls().is_drag_area_running, false);
            if was_running {
                // SAFETY: an NSWindow.
                let _: () = unsafe { msg_send![data.nswindow(), setMovableByWindowBackground: NO] };
                return true; // was dragging, drop event.
            }
        }
    }

    false // not a special area, carry on.
}

/// Translation of `Cocoa_SendMouseButtonClicks()`.
fn send_mouse_button_clicks(the_event: Id, window: WindowID, button: u8, down: bool) {
    // GCMouse handles button events directly, skip NSEvent path to avoid duplicates
    if mouse::has_gc_mouse() {
        return;
    }

    let mouse_id = DEFAULT_MOUSE_ID;
    //const int clicks = (int)[theEvent clickCount];
    let focus = crate::events::keyboard::keyboard_focus();
    // SAFETY: an NSEvent.
    let (event_window, timestamp): (Id, f64) = unsafe {
        (
            msg_send![the_event, window],
            msg_send![the_event, timestamp],
        )
    };
    let timestamp = get_event_timestamp(timestamp);

    // macOS will send non-left clicks to background windows without raising them, so we need to
    //  temporarily adjust the mouse position when this happens, as `mouse` will be tracking
    //  the position in the currently-focused window. We don't (currently) send a mousemove
    //  event for the background window, this just makes sure the button is reported at the
    //  correct position in its own event.
    if focus
        .and_then(window_data)
        .is_some_and(|d| d.nswindow() == event_window)
    {
        //SDL_SendMouseButtonClicks(Cocoa_GetEventTimestamp([theEvent timestamp]), window, mouseID, button, down, clicks);
        send_mouse_button(timestamp, Some(window), mouse_id, button, down);
    } else {
        // SAFETY: an NSEvent.
        let point: NSPoint = unsafe { msg_send![the_event, locationInWindow] };
        let (_, _, _, h) = geometry(window);
        let (orig_x, orig_y) = with_mouse(|m| {
            let orig = (m.x, m.y);
            m.x = (point.x as i32) as f32;
            m.y = ((h as f64 - point.y) as i32) as f32;
            orig
        });
        //SDL_SendMouseButtonClicks(Cocoa_GetEventTimestamp([theEvent timestamp]), window, mouseID, button, down, clicks);
        send_mouse_button(timestamp, Some(window), mouse_id, button, down);
        with_mouse(|m| {
            m.x = orig_x;
            m.y = orig_y;
        });
    }
}

/// `NSMouseInRect([event locationInWindow], [[[event window] contentView] frame], NO)`
/// (true for an event without a window).
fn event_in_content_view(the_event: Id) -> bool {
    // SAFETY: an NSEvent and its window's content view.
    unsafe {
        let window: Id = msg_send![the_event, window];
        if window.is_null() {
            return true;
        }
        let content_view: Id = msg_send![window, contentView];
        let window_rect: NSRect = msg_send![content_view, frame];
        let location: NSPoint = msg_send![the_event, locationInWindow];
        // (NSMouseInRect() for an unflipped view is NSPointInRect())
        window_rect.contains(location)
    }
}

/// Translation of `-[SDL3Cocoa_WindowListener mouseDown:]` (and of
/// `rightMouseDown:` and `otherMouseDown:`, which call it).
extern "C" fn listener_mouse_down(this: Id, _sel: Sel, the_event: Id) {
    let Some(data) = listener_data(this) else {
        return;
    };
    if pen::handle_pen_event(&data, the_event) {
        return; // pen code handled it.
    }

    // Ignore events that aren't inside the client area (i.e. title bar.)
    if !event_in_content_view(the_event) {
        return;
    }

    // SAFETY: an NSEvent.
    let (button_number, modifier_flags): (NSInteger, NSUInteger) = unsafe {
        (
            msg_send![the_event, buttonNumber],
            msg_send![the_event, modifierFlags],
        )
    };
    let button = match button_number {
        0 => {
            if (modifier_flags & NSEventModifierFlagControl) != 0
                && get_hint_ctrl_click_emulate_right_click()
            {
                data.ls().was_ctrl_left = true;
                BUTTON_RIGHT
            } else {
                data.ls().was_ctrl_left = false;
                BUTTON_LEFT
            }
        }
        1 => BUTTON_RIGHT,
        2 => BUTTON_MIDDLE,
        n => (n + 1) as u8,
    };

    if button == BUTTON_LEFT && process_hit_test(&data, the_event) {
        send(data.window, EventType::WINDOW_HIT_TEST, 0, 0);
        return; // dragging, drop event.
    }

    send_mouse_button_clicks(the_event, data.window, button, true);
}

/// Translation of `-[SDL3Cocoa_WindowListener mouseUp:]` (and of
/// `rightMouseUp:` and `otherMouseUp:`, which call it).
extern "C" fn listener_mouse_up(this: Id, _sel: Sel, the_event: Id) {
    let Some(data) = listener_data(this) else {
        return;
    };
    if pen::handle_pen_event(&data, the_event) {
        return; // pen code handled it.
    }

    // SAFETY: an NSEvent.
    let button_number: NSInteger = unsafe { msg_send![the_event, buttonNumber] };
    let button = match button_number {
        0 => {
            if std::mem::replace(&mut data.ls().was_ctrl_left, false) {
                BUTTON_RIGHT
            } else {
                BUTTON_LEFT
            }
        }
        1 => BUTTON_RIGHT,
        2 => BUTTON_MIDDLE,
        n => (n + 1) as u8,
    };

    if button == BUTTON_LEFT && process_hit_test(&data, the_event) {
        send(data.window, EventType::WINDOW_HIT_TEST, 0, 0);
        return; // stopped dragging, drop event.
    }

    send_mouse_button_clicks(the_event, data.window, button, false);
}

/// Translation of `-[SDL3Cocoa_WindowListener mouseMoved:]` (and of the
/// `*MouseDragged:` methods, which call it).
extern "C" fn listener_mouse_moved(this: Id, _sel: Sel, the_event: Id) {
    let Some(data) = listener_data(this) else {
        return;
    };
    if pen::handle_pen_event(&data, the_event) {
        return; // pen code handled it.
    }

    let mouse_id = DEFAULT_MOUSE_ID;

    if mouse::get_mouse_focus().is_null() {
        // The mouse is no longer over any window in the application
        set_mouse_focus(None);
        return;
    }

    let window = data.window;
    let content_view = data.sdl_content_view.get();
    // SAFETY: an NSEvent and the content view.
    let point: NSPoint = unsafe { msg_send![the_event, locationInWindow] };

    // SAFETY: the content view and NSCursor's class method.
    unsafe {
        let converted: NSPoint = msg_send![content_view, convertPoint: point fromView: NIL];
        let bounds: NSRect = msg_send![content_view, bounds];
        let in_rect: Bool = msg_send![content_view, mouse: converted inRect: bounds];
        let current: Id = msg_send![class!(NSCursor), currentCursor];
        if in_rect.is_true() && current != get_desired_cursor() {
            // The wrong cursor is on screen, fix it. This fixes an macOS bug that is only known to
            // occur in fullscreen windows on the built-in displays of newer MacBooks with camera
            // notches. When the mouse is moved near the top of such a window (within about 44 units)
            // and then moved back down, the cursor rects aren't respected.
            let _: () = msg_send![data.nswindow(), invalidateCursorRectsForView: content_view];
        }
    }

    if flags(window).contains(WindowFlags::TRANSPARENT) {
        update_ignore_mouse_state(&data, the_event);
    }

    if process_hit_test(&data, the_event) {
        send(window, EventType::WINDOW_HIT_TEST, 0, 0);
        return; // dragging, drop event.
    }

    if with_mouse(|m| m.relative_mode) {
        return;
    }

    let (wx, wy, _, h) = geometry(window);
    let mut x = point.x as f32;
    let mut y = (h as f64 - point.y) as f32;

    // On macOS 26 if you move away from a space and then back, mouse motion events will have incorrect
    // values at the top of the screen. Apparently non-fullscreen windows suffer from this from time
    // to time when mouse leaves/enters a window and/or focus changes, too. The global mouse position query
    // is still correct, so we'll fall back to that until this is fixed by Apple. Mouse button events are
    // interestingly not affected.
    if macos_available(26, 0) {
        // do for all windows for now, not just fullscreen spaces. And hopefully remove this code entirely soon.
        //if ([_data.listener isInFullscreenSpace]) {
        let (posx, posy) = Window::from_raw(window).position().unwrap_or((0, 0));
        let (posx, posy) = relative_to_global_for_window(window, posx, posy);
        let (gx, gy, _) = crate::events::mouse::global_mouse_state();
        x = gx - posx as f32;
        y = gy - posy as f32;
        //}
    }

    if appkit_version_number() >= NSAppKitVersionNumber10_13_2 {
        // Mouse grab is taken care of by the confinement rect
    } else if should_adjust_coordinates_for_grab(window) {
        if let Some(cgpoint) = adjust_coordinates_for_grab(window, wx as f32 + x, wy as f32 + y) {
            mouse::handle_mouse_warp(cgpoint.x, cgpoint.y);
            // SAFETY: a point on the main display.
            unsafe {
                CGDisplayMoveCursorToPoint(kCGDirectMainDisplay(), cgpoint);
                CGAssociateMouseAndMouseCursorPosition(1);
            }
        }
    }

    // SAFETY: an NSEvent.
    let timestamp: f64 = unsafe { msg_send![the_event, timestamp] };
    send_mouse_motion(
        get_event_timestamp(timestamp),
        Some(window),
        mouse_id,
        false,
        x,
        y,
    );
}

/// Translation of `-[SDL3Cocoa_WindowListener scrollWheel:]`.
extern "C" fn listener_scroll_wheel(this: Id, _sel: Sel, the_event: Id) {
    if let Some(data) = listener_data(this) {
        mouse::handle_mouse_wheel(data.window, the_event);
    }
}

/// Translation of `-[SDL3Cocoa_WindowListener isTouchFromTrackpad:]`.
fn is_touch_from_trackpad(data: &WindowData, the_event: Id) -> bool {
    /* if this a MacBook trackpad, we'll make input look like a synthesized
    event. This is backwards from reality, but better matches user
    expectations. You can make it look like a generic touch device instead
    with the SDL_HINT_TRACKPAD_IS_TOUCH_ONLY hint. */
    let mut istrackpad = false;
    if !data.videodata.state().trackpad_is_touch_only {
        // Note (upstream): upstream catches the NSInternalInconsistencyException
        // that -[NSEvent subtype] throws for events without a subtype (such as
        // NSEventTypeBeginGesture on macOS 10.5 to 10.10); Objective-C
        // exceptions can't be caught here, and the touch events this is called
        // for have a subtype on the macOS versions this runs on.
        // SAFETY: an NSEvent.
        let subtype: i16 = unsafe { msg_send![the_event, subtype] };
        istrackpad = subtype == NSEventSubtypeMouseEvent;
    }
    istrackpad
}

/// `[theEvent touchesMatchingPhase:phase inView:nil]` as a list.
fn touches_matching_phase(the_event: Id, phase: NSUInteger) -> Vec<Id> {
    // SAFETY: an NSEvent; the set's objects are NSTouches.
    unsafe {
        let touches: Id = msg_send![the_event, touchesMatchingPhase: phase inView: NIL];
        let all: Id = msg_send![touches, allObjects];
        ns_array_items(all)
    }
}

/// Translation of `-[SDL3Cocoa_WindowListener touchesBeganWithEvent:]`.
extern "C" fn listener_touches_began(this: Id, _sel: Sel, the_event: Id) {
    let Some(data) = listener_data(this) else {
        return;
    };
    let istrackpad = is_touch_from_trackpad(&data, the_event);

    let touches = touches_matching_phase(the_event, NSTouchPhaseAny);
    let touch_id = if istrackpad {
        MOUSE_TOUCH_ID
    } else {
        // SAFETY: an NSTouch.
        touches.first().map_or(0, |&t| {
            let device: Id = unsafe { msg_send![t, device] };
            device as usize as u64
        })
    };
    let mut existing_touch_count = 0;

    for &touch in &touches {
        // SAFETY: an NSTouch.
        let phase: NSUInteger = unsafe { msg_send![touch, phase] };
        if phase != NSTouchPhaseBegan {
            existing_touch_count += 1;
        }
    }
    if existing_touch_count == 0 {
        if let Ok(fingers) = touch_fingers(touch_id) {
            // DLog("Reset Lost Fingers: %d", numFingers);
            // SAFETY: an NSEvent.
            let timestamp: f64 = unsafe { msg_send![the_event, timestamp] };
            for finger in fingers.iter().rev() {
                /* trackpad touches have no window. If we really wanted one we could
                 * use the window that has mouse or keyboard focus.
                 * Sending a null window currently also prevents synthetic mouse
                 * events from being generated from touch events.
                 */
                send_touch(
                    get_event_timestamp(timestamp),
                    touch_id,
                    finger.id,
                    None,
                    EventType::FINGER_CANCELED,
                    0.0,
                    0.0,
                    0.0,
                );
            }
        }
    }

    // DLog("Began Fingers: %lu .. existing: %d", (unsigned long)[touches count], existingTouchCount);
    handle_touches(&data, NSTouchPhaseBegan, the_event);
}

extern "C" fn listener_touches_moved(this: Id, _sel: Sel, the_event: Id) {
    if let Some(data) = listener_data(this) {
        handle_touches(&data, NSTouchPhaseMoved, the_event);
    }
}

extern "C" fn listener_touches_ended(this: Id, _sel: Sel, the_event: Id) {
    if let Some(data) = listener_data(this) {
        handle_touches(&data, NSTouchPhaseEnded, the_event);
    }
}

extern "C" fn listener_touches_cancelled(this: Id, _sel: Sel, the_event: Id) {
    if let Some(data) = listener_data(this) {
        handle_touches(&data, NSTouchPhaseCancelled, the_event);
    }
}

/// Translation of `-[SDL3Cocoa_WindowListener magnifyWithEvent:]`.
extern "C" fn listener_magnify(_this: Id, _sel: Sel, the_event: Id) {
    // SAFETY: an NSEvent.
    let (phase, timestamp): (NSUInteger, f64) =
        unsafe { (msg_send![the_event, phase], msg_send![the_event, timestamp]) };
    let timestamp = get_event_timestamp(timestamp);
    match phase {
        NSEventPhaseBegan => {
            send_pinch(
                EventType::PINCH_BEGIN,
                timestamp,
                0,
                0.0,
                -1.0,
                -1.0,
                -1.0,
                -1.0,
            );
        }
        NSEventPhaseChanged => {
            // SAFETY: an NSEvent.
            let magnification: f64 = unsafe { msg_send![the_event, magnification] };
            let scale = 1.0 + magnification;
            send_pinch(
                EventType::PINCH_UPDATE,
                timestamp,
                0,
                scale as f32,
                -1.0,
                -1.0,
                -1.0,
                -1.0,
            );
        }
        NSEventPhaseEnded | NSEventPhaseCancelled => {
            send_pinch(
                EventType::PINCH_END,
                timestamp,
                0,
                0.0,
                -1.0,
                -1.0,
                -1.0,
                -1.0,
            );
        }
        _ => {}
    }
}

/// Translation of `-[SDL3Cocoa_WindowListener handleTouches:withEvent:]`.
fn handle_touches(data: &WindowData, phase: NSUInteger, the_event: Id) {
    let touches = touches_matching_phase(the_event, phase);
    let istrackpad = is_touch_from_trackpad(data, the_event);
    // SAFETY: an NSEvent.
    let timestamp: f64 = unsafe { msg_send![the_event, timestamp] };
    let timestamp = get_event_timestamp(timestamp);

    for touch in touches {
        // SAFETY: an NSTouch.
        let (device, ty, identity, normalized): (Id, NSInteger, Id, NSPoint) = unsafe {
            (
                msg_send![touch, device],
                msg_send![touch, type],
                msg_send![touch, identity],
                msg_send![touch, normalizedPosition],
            )
        };
        let touch_id = if istrackpad {
            MOUSE_TOUCH_ID
        } else {
            device as usize as u64
        };
        let devtype = TouchDeviceType::IndirectAbsolute;

        /* trackpad touches have no window. If we really wanted one we could
         * use the window that has mouse or keyboard focus.
         * Sending a null window currently also prevents synthetic mouse events
         * from being generated from touch events.
         */
        let window: Option<WindowID> = None;

        /* TODO: Before implementing direct touch support here, we need to
         * figure out whether the OS generates mouse events from them on its
         * own. If it does, we should prevent SendTouch from generating
         * synthetic mouse events for these touches itself (while also
         * sending a window.) It will also need to use normalized window-
         * relative coordinates via [touch locationInView:].
         */
        if ty == NSTouchTypeDirect {
            continue;
        }

        add_touch(touch_id, devtype, "");

        let finger_id = identity as usize as u64;
        let x = normalized.x as f32;
        let mut y = normalized.y as f32;
        // Make the origin the upper left instead of the lower left
        y = 1.0 - y;

        match phase {
            NSTouchPhaseBegan => send_touch(
                timestamp,
                touch_id,
                finger_id,
                window,
                EventType::FINGER_DOWN,
                x,
                y,
                1.0,
            ),
            NSTouchPhaseEnded => send_touch(
                timestamp,
                touch_id,
                finger_id,
                window,
                EventType::FINGER_UP,
                x,
                y,
                1.0,
            ),
            NSTouchPhaseCancelled => send_touch(
                timestamp,
                touch_id,
                finger_id,
                window,
                EventType::FINGER_CANCELED,
                x,
                y,
                1.0,
            ),
            NSTouchPhaseMoved => {
                send_touch_motion(timestamp, touch_id, finger_id, window, x, y, 1.0)
            }
            _ => {}
        }
    }
}

/// Translation of `-[SDL3Cocoa_WindowListener tabletProximity:]` and
/// `tabletPoint:`.
extern "C" fn listener_tablet_event(this: Id, _sel: Sel, the_event: Id) {
    if let Some(data) = listener_data(this) {
        pen::handle_pen_event(&data, the_event);
    }
}

// ---------------------------------------------------------------------------
// SDL3View

static SDL3_VIEW: DeclaredClass = DeclaredClass::new();

/// The `SDL3View` class (`@interface SDL3View : NSView`), keeping the
/// window ID in `_sdlWindowID` and its tracking area (retained) in
/// `_trackingArea`.
pub(crate) fn sdl3_view_class() -> Class {
    SDL3_VIEW.get(|| {
        let mut b = ClassBuilder::new(c"SDL3View", class!(NSView)).expect("SDL3View");
        b.add_ivar::<u32>(c"_sdlWindowID", c"I");
        b.add_ivar::<Id>(c"_trackingArea", c"@");
        // SAFETY: the implementations have the signatures of the encodings.
        unsafe {
            b.add_method(
                sel!(drawRect:),
                imp!(view_draw_rect as extern "C" fn(Id, Sel, NSRect)),
                c"v@:{CGRect={CGPoint=dd}{CGSize=dd}}",
            );
            b.add_method(
                sel!(wantsUpdateLayer),
                imp!(view_wants_update_layer as extern "C" fn(Id, Sel) -> Bool),
                &encoding("B@:"),
            );
            b.add_method(
                sel!(updateLayer),
                imp!(view_update_layer as extern "C" fn(Id, Sel)),
                c"v@:",
            );
            b.add_method(
                sel!(rightMouseDown:),
                imp!(view_right_mouse_down as extern "C" fn(Id, Sel, Id)),
                c"v@:@",
            );
            b.add_method(
                sel!(mouseDownCanMoveWindow),
                imp!(view_mouse_down_can_move_window as extern "C" fn(Id, Sel) -> Bool),
                &encoding("B@:"),
            );
            b.add_method(
                sel!(resetCursorRects),
                imp!(view_reset_cursor_rects as extern "C" fn(Id, Sel)),
                c"v@:",
            );
            b.add_method(
                sel!(acceptsFirstMouse:),
                imp!(view_accepts_first_mouse as extern "C" fn(Id, Sel, Id) -> Bool),
                &encoding("B@:@"),
            );
            b.add_method(
                sel!(updateTrackingAreas),
                imp!(view_update_tracking_areas as extern "C" fn(Id, Sel)),
                c"v@:",
            );
            b.add_method(
                sel!(dealloc),
                imp!(view_dealloc as extern "C" fn(Id, Sel)),
                c"v@:",
            );
        }
        b.register()
    })
}

/// The SDL window of an `SDL3View` (`_sdlWindow`).
fn view_window(this: Id) -> WindowID {
    // SAFETY: an SDL3View, which has the variable.
    unsafe { *ivar_ptr::<u32>(this, sdl3_view_class(), c"_sdlWindowID") }
}

/// Translation of `-[SDL3View setSDLWindow:]`.
fn view_set_sdl_window(this: Id, window: WindowID) {
    // SAFETY: an SDL3View, which has the variable.
    unsafe { *ivar_ptr::<u32>(this, sdl3_view_class(), c"_sdlWindowID") = window };
}

/// The layer background for the window's transparency.
fn background_color(window: WindowID) -> CGColorRef {
    let transparent = flags(window).contains(WindowFlags::TRANSPARENT);
    // SAFETY: CoreGraphics' constant colors.
    unsafe {
        let color = if transparent {
            kCGColorClear
        } else {
            kCGColorBlack
        };
        CGColorGetConstantColor(color)
    }
}

/// this is used on older macOS revisions, and newer ones which emulate old
/// NSOpenGLContext behaviour while still using a layer under the hood. 10.8 and
/// later use updateLayer, up until 10.14.2 or so, which uses drawRect without
/// a GraphicsContext and with a layer active instead (for OpenGL contexts).
///
/// Translation of `-[SDL3View drawRect:]`.
extern "C" fn view_draw_rect(this: Id, _sel: Sel, dirty_rect: NSRect) {
    let window = view_window(this);
    /* Force the graphics context to clear to black so we don't get a flash of
    white until the app is ready to draw. In practice on modern macOS, this
    only gets called for window creation and other extraordinary events. */
    let transparent = flags(window).contains(WindowFlags::TRANSPARENT);
    // SAFETY: NSGraphicsContext, NSColor and the view's layer.
    unsafe {
        let current: Id = msg_send![class!(NSGraphicsContext), currentContext];
        let layer: Id = msg_send![this, layer];
        if !current.is_null() {
            let fill_color: Id = if transparent {
                msg_send![class!(NSColor), clearColor]
            } else {
                msg_send![class!(NSColor), blackColor]
            };
            let _: () = msg_send![fill_color, setFill];
            NSRectFill(dirty_rect);
        } else if !layer.is_null() {
            let _: () = msg_send![layer, setBackgroundColor: background_color(window)];
        }
    }

    send_exposed_event_if_visible(window);
}

/// Translation of `-[SDL3View wantsUpdateLayer]`.
extern "C" fn view_wants_update_layer(_this: Id, _sel: Sel) -> Bool {
    YES
}

/// This is also called when a Metal layer is active.
/// Translation of `-[SDL3View updateLayer]`.
extern "C" fn view_update_layer(this: Id, _sel: Sel) {
    let window = view_window(this);
    /* Force the graphics context to clear to black so we don't get a flash of
    white until the app is ready to draw. In practice on modern macOS, this
    only gets called for window creation and other extraordinary events. */
    // SAFETY: the view's layer.
    unsafe {
        let layer: Id = msg_send![this, layer];
        let _: () = msg_send![layer, setBackgroundColor: background_color(window)];
    }
    if let Some(data) = window_data(window) {
        schedule_context_updates(&data);
    }
    send_exposed_event_if_visible(window);
}

/// The default implementation doesn't pass rightMouseDown to responder chain
/// Translation of `-[SDL3View rightMouseDown:]`.
extern "C" fn view_right_mouse_down(this: Id, _sel: Sel, the_event: Id) {
    // SAFETY: the view's next responder.
    let _: () = unsafe { msg_send![next_responder(this), rightMouseDown: the_event] };
}

/// Translation of `-[SDL3View mouseDownCanMoveWindow]`.
extern "C" fn view_mouse_down_can_move_window(_this: Id, _sel: Sel) -> Bool {
    /* Always say YES, but this doesn't do anything until we call
    -[NSWindow setMovableByWindowBackground:YES], which we ninja-toggle
    during mouse events when we're using a drag area. */
    YES
}

/// Translation of `-[SDL3View resetCursorRects]`.
extern "C" fn view_reset_cursor_rects(this: Id, _sel: Sel) {
    // SAFETY: NSView's implementation and the view's bounds.
    unsafe {
        let _: () = msg_send_super![this, sdl3_view_class(), resetCursorRects];
        let bounds: NSRect = msg_send![this, bounds];
        let _: () = msg_send![this, addCursorRect: bounds cursor: get_desired_cursor()];
    }
}

/// Translation of `-[SDL3View acceptsFirstMouse:]`.
extern "C" fn view_accepts_first_mouse(this: Id, _sel: Sel, _the_event: Id) -> Bool {
    let window = view_window(this);
    if flags(window).contains(WindowFlags::POPUP_MENU) {
        YES
    } else if hints::get(hints::MOUSE_FOCUS_CLICKTHROUGH).is_some() {
        Bool::from(hints::get_bool(hints::MOUSE_FOCUS_CLICKTHROUGH, false))
    } else {
        Bool::from(hints::get_bool("SDL_MAC_MOUSE_FOCUS_CLICKTHROUGH", false))
    }
}

/// NSTrackingArea is how Cocoa tells you when the mouse cursor has entered or
///  left certain regions. We put one over our entire window so we know when
///  it has "mouse focus."
/// Translation of `-[SDL3View updateTrackingAreas]`.
extern "C" fn view_update_tracking_areas(this: Id, _sel: Sel) {
    // SAFETY: NSView's implementation, then a tracking area owned through
    // the instance variable.
    unsafe {
        let _: () = msg_send_super![this, sdl3_view_class(), updateTrackingAreas];

        let Some(windata) = window_data(view_window(this)) else {
            return;
        };
        let tracking = ivar_ptr::<Id>(this, sdl3_view_class(), c"_trackingArea");
        if !(*tracking).is_null() {
            let _: () = msg_send![this, removeTrackingArea: *tracking];
            crate::core::apple::objc::release(*tracking);
            *tracking = NIL;
        }
        let bounds: NSRect = msg_send![this, bounds];
        let area: Id = msg_send![super::alloc(class!(NSTrackingArea)),
            initWithRect: bounds
            options: (NSTrackingMouseEnteredAndExited | NSTrackingActiveAlways)
            owner: windata.listener()
            userInfo: NIL];
        *tracking = area;
        let _: () = msg_send![this, addTrackingArea: area];
    }
}

/// Release the tracking area (ARC's `-dealloc`).
extern "C" fn view_dealloc(this: Id, _sel: Sel) {
    // SAFETY: the instance variable we own, then NSView's dealloc.
    unsafe {
        let tracking = ivar_ptr::<Id>(this, sdl3_view_class(), c"_trackingArea");
        crate::core::apple::objc::release(*tracking);
        *tracking = NIL;
        let _: () = msg_send_super![this, sdl3_view_class(), dealloc];
    }
}

/// Translation of `Cocoa_UpdateMouseFocus()`.
fn update_mouse_focus() {
    // SAFETY: NSEvent's class method.
    let mouse_location: NSPoint = unsafe { msg_send![class!(NSEvent), mouseLocation] };

    // Find the topmost window under the pointer and send a motion event if it is an SDL window.
    let block = RcBlock::new2(move |nswin: Id, stop: *mut Bool| {
        // SAFETY: an NSWindow and the stop flag of the enumeration.
        unsafe {
            let frame: NSRect = msg_send![nswin, frame];
            let r: NSRect = msg_send![nswin, contentRectForFrameRect: frame];
            if r.contains(mouse_location) {
                let sdlwindow = find_sdl_window(nswin);
                *stop = YES;
                if let Some(sdlwindow) = sdlwindow {
                    let main_display_height = video_data().map_or(0.0, |d| d.main_display_height());
                    let (x, y, _, _) = geometry(sdlwindow);
                    let (wx, wy) = relative_to_global_for_window(sdlwindow, x, y);

                    // Calculate the cursor coordinates relative to the window.
                    let dx = (mouse_location.x - wx as f64) as f32;
                    let dy = ((main_display_height - mouse_location.y) - wy as f64) as f32;
                    send_mouse_motion(
                        std::time::Duration::ZERO,
                        Some(sdlwindow),
                        GLOBAL_MOUSE_ID,
                        false,
                        dx,
                        dy,
                    );
                }
            }
        }
    });
    // SAFETY: NSApp's window enumeration with a block of the documented
    // signature, called synchronously.
    let _: () = unsafe {
        msg_send![ns_app(), enumerateWindowsWithOptions: NSWindowListOrderedFrontToBack
                                             usingBlock: block.as_ptr()]
    };
}

/// Translation of `SetupWindowData()`.
fn setup_window_data(
    videodata: &Arc<VideoData>,
    window: WindowID,
    nswindow: Id,
    nsview: Id,
) -> Result<()> {
    autoreleasepool(|| {
        // SAFETY: the window, its view and a new listener, all retained by
        // the window data.
        unsafe {
            let high_density = flags(window).contains(WindowFlags::HIGH_PIXEL_DENSITY);
            let viewport: NSRect = {
                let bounds: NSRect = msg_send![nsview, bounds];
                if high_density {
                    // This gives us the correct viewport for a Retina-enabled view.
                    msg_send![nsview, convertRectToBacking: bounds]
                } else {
                    bounds
                }
            };
            let window_number: NSInteger = msg_send![nswindow, windowNumber];

            // Create an event listener for the window
            let listener: Id = msg_send![super::alloc(sdl3_cocoa_window_listener_class()), init];
            *ivar_ptr::<u32>(
                listener,
                sdl3_cocoa_window_listener_class(),
                c"_sdlWindowID",
            ) = window;

            // Allocate the window data
            let data = Arc::new(WindowData {
                window,
                nswindow: StrongPtr::retain(nswindow),
                sdl_content_view: StrongPtr::retain(nsview),
                listener: Mutex::new(StrongPtr::from_owned(listener)),
                videodata: videodata.clone(),
                state: Mutex::new(WindowState {
                    viewport,
                    nscontexts: Vec::new(),
                    in_blocking_transition: false,
                    fullscreen_space_requested: false,
                    was_zoomed: false,
                    window_number,
                    flash_request: 0,
                    modal_session: std::ptr::null_mut(),
                    pending_size: false,
                    pending_position: false,
                    border_toggled: false,
                    has_modal_dialog: false,
                    egl_surface: std::ptr::null_mut(),
                }),
                listener_state: Mutex::new(ListenerState::new()),
            });
            with_window(window, |w| w.internal = Some(Box::new(data.clone())))?;

            // Fill in the SDL window with the window data
            {
                let rect = content_rect(nswindow);
                let (x, y) = global_to_relative_for_window(
                    window,
                    rect.origin.x as i32,
                    rect.origin.y as i32,
                );
                with_window(window, |w| {
                    w.core.x = x;
                    w.core.y = y;
                    w.core.w = rect.size.width as i32;
                    w.core.h = rect.size.height as i32;
                })?;
            }

            // Set up the listener after we create the view
            listen(&data);

            let visible: Bool = msg_send![nswindow, isVisible];
            let style: NSUInteger = msg_send![nswindow, styleMask];
            let is_zoomed: Bool = msg_send![nswindow, isZoomed];
            let miniaturized: Bool = msg_send![nswindow, isMiniaturized];
            with_window(window, |w| {
                let f = &mut w.core.flags;
                if visible.is_true() {
                    *f &= !WindowFlags::HIDDEN;
                } else {
                    *f |= WindowFlags::HIDDEN;
                }

                /* NSWindowStyleMaskBorderless is zero, and it's possible to be
                Resizeable _and_ borderless, so we can't do a simple bitwise AND
                of NSWindowStyleMaskBorderless here. */
                if (style & !(NSWindowStyleMaskResizable | NSWindowStyleMaskMiniaturizable))
                    == NSWindowStyleMaskBorderless
                {
                    *f |= WindowFlags::BORDERLESS;
                } else {
                    *f &= !WindowFlags::BORDERLESS;
                }
                if style & NSWindowStyleMaskResizable != 0 {
                    *f |= WindowFlags::RESIZABLE;
                } else {
                    *f &= !WindowFlags::RESIZABLE;
                }

                // isZoomed always returns true if the window is not resizable
                if f.contains(WindowFlags::RESIZABLE) && is_zoomed.is_true() {
                    *f |= WindowFlags::MAXIMIZED;
                } else {
                    *f &= !WindowFlags::MAXIMIZED;
                }

                if miniaturized.is_true() {
                    *f |= WindowFlags::MINIMIZED;
                } else {
                    *f &= !WindowFlags::MINIMIZED;
                }
            })?;

            if let Some(parent) = parent_of(window) {
                if let Some(parent_data) = window_data(parent) {
                    let _: () = msg_send![parent_data.nswindow(), addChildWindow: nswindow ordered: NSWindowAbove];
                }

                /* FIXME: Should not need to call addChildWindow then orderOut.
                Attaching a hidden child window to a hidden parent window will cause the child window
                to show when the parent does. We therefore shouldn't attach the child window here as we're
                going to do so when the child window is explicitly shown later but skipping the addChildWindow
                entirely causes the child window to not get key focus correctly the first time it's shown. Adding
                then immediately ordering out (removing) the window does work. */
                if flags(window).contains(WindowFlags::HIDDEN) {
                    let _: () = msg_send![nswindow, orderOut: NIL];
                }
            }

            let f = flags(window);
            if !is_popup(window) {
                let is_key: Bool = msg_send![nswindow, isKeyWindow];
                if is_key.is_true() {
                    with_window(window, |w| w.core.flags |= WindowFlags::INPUT_FOCUS)?;
                    set_keyboard_focus(Some(data.window), true);
                }
            } else if f.contains(WindowFlags::TOOLTIP) {
                let _: () = msg_send![nswindow, setIgnoresMouseEvents: YES];
                let _: () = msg_send![nswindow, setAcceptsMouseMovedEvents: NO];
            } else if f.contains(WindowFlags::POPUP_MENU) && !f.contains(WindowFlags::HIDDEN) {
                if !f.contains(WindowFlags::NOT_FOCUSABLE) {
                    set_keyboard_focus(Some(window), true);
                }
                update_mouse_focus();
            }

            let opaque: Bool = msg_send![nswindow, isOpaque];
            with_window(window, |w| {
                if opaque.is_true() {
                    w.core.flags &= !WindowFlags::TRANSPARENT;
                } else {
                    w.core.flags |= WindowFlags::TRANSPARENT;
                }
            })?;

            /* SDL_CocoaWindowData will be holding a strong reference to the NSWindow, and
             * it will also call [NSWindow close] in DestroyWindow before releasing the
             * NSWindow, so the extra release provided by releasedWhenClosed isn't
             * necessary. */
            let _: () = msg_send![nswindow, setReleasedWhenClosed: NO];

            /* Prevents the window's "window device" from being destroyed when it is
             * hidden. See http://www.mikeash.com/pyblog/nsopenglcontext-and-one-shot.html
             */
            let _: () = msg_send![nswindow, setOneShot: NO];

            if f.contains(WindowFlags::EXTERNAL) {
                // Query the title from the existing window
                let title: Id = msg_send![nswindow, title];
                if let Some(title) = crate::core::apple::foundation::string_from_ns(title) {
                    with_window(window, |w| w.title = Some(title))?;
                }
            }

            let props = Window::from_raw(window).properties()?;
            let _ = props.set(PROP_WINDOW_COCOA_WINDOW_POINTER, nswindow as usize as i64);
            let _ = props.set(
                PROP_WINDOW_COCOA_METAL_VIEW_TAG_NUMBER,
                SDL_METALVIEW_TAG as i64,
            );

            // All done!
            Ok(())
        }
    })
}

/// Translation of `Cocoa_CreateWindow()`.
pub(crate) fn create_window(
    videodata: &Arc<VideoData>,
    window: WindowID,
    create_props: &Properties,
) -> Result<()> {
    autoreleasepool(|| {
        let pointer = |name: &str| -> Id {
            create_props
                .get_number(name)
                .map_or(NIL, |p| p as usize as Id)
        };
        let data = pointer("sdl2-compat.external_window");
        let mut nswindow: Id;
        let mut nsview: Id;

        // SAFETY: objects the caller passed (an NSWindow or NSView), or a
        // new SDL3Window and SDL3View, retained by the window data.
        unsafe {
            if !data.is_null() {
                nswindow = NIL;
                nsview = NIL;
                if is_kind_of(data, class!(NSWindow)) {
                    nswindow = data;
                } else if is_kind_of(data, class!(NSView)) {
                    nsview = data;
                } else {
                    crate::sdl_assert!(false);
                }
            } else {
                nswindow = pointer(PROP_WINDOW_CREATE_COCOA_WINDOW_POINTER);
                nsview = pointer(PROP_WINDOW_CREATE_COCOA_VIEW_POINTER);
            }
            if !nswindow.is_null() && nsview.is_null() {
                nsview = msg_send![nswindow, contentView];
            }
            if !nsview.is_null() && nswindow.is_null() {
                nswindow = msg_send![nsview, window];
            }
            let _owned_window: StrongPtr;
            let _owned_view: StrongPtr;
            if !nswindow.is_null() {
                with_window(window, |w| w.core.flags |= WindowFlags::EXTERNAL)?;
            } else {
                let (wx, wy, ww, wh) =
                    with_window(window, |w| (w.core.x, w.core.y, w.core.w, w.core.h))?;
                let (x, y) = relative_to_global_for_window(window, wx, wy);
                let mut rect = NSRect::new(x as f64, y as f64, ww as f64, wh as f64);
                convert_ns_rect(&mut rect);

                let style = get_window_style(window);

                // Figure out which screen to place this window
                let screen = screen_for_rect(&rect);
                let screen_rect = screen_frame(screen);
                rect.origin.x -= screen_rect.origin.x;
                rect.origin.y -= screen_rect.origin.y;

                // Constrain the popup
                let constrain_popup = with_window(window, |w| w.constrain_popup)?;
                if is_popup(window) && constrain_popup {
                    if rect.origin.x + rect.size.width
                        > screen_rect.origin.x + screen_rect.size.width
                    {
                        rect.origin.x -= (rect.origin.x + rect.size.width)
                            - (screen_rect.origin.x + screen_rect.size.width);
                    }
                    if rect.origin.y + rect.size.height
                        > screen_rect.origin.y + screen_rect.size.height
                    {
                        rect.origin.y -= (rect.origin.y + rect.size.height)
                            - (screen_rect.origin.y + screen_rect.size.height);
                    }
                    rect.origin.x = rect.origin.x.max(screen_rect.origin.x);
                    rect.origin.y = rect.origin.y.max(screen_rect.origin.y);
                }

                // Note (upstream): upstream catches an exception thrown here
                // and reports its reason; Objective-C exceptions can't be
                // caught in Rust.
                nswindow = msg_send![super::alloc(sdl3_window_class()),
                    initWithContentRect: rect
                    styleMask: style
                    backing: NSBackingStoreBuffered
                    defer: NO
                    screen: screen];
                if nswindow.is_null() {
                    return Err(Error::new("Couldn't create the NSWindow"));
                }
                _owned_window = StrongPtr::from_owned(nswindow);

                let srgb: Id = msg_send![class!(NSColorSpace), sRGBColorSpace];
                let _: () = msg_send![nswindow, setColorSpace: srgb];

                let _: () = msg_send![nswindow, setTabbingMode: NSWindowTabbingModeDisallowed];

                // we put fullscreen desktop windows in their own Space, without a toggle button or menubar, later
                if flags(window).contains(WindowFlags::RESIZABLE) && videodata.state().allow_spaces
                {
                    // resizable windows are Spaces-friendly: they get the "go fullscreen" toggle button on their titlebar.
                    let _: () = msg_send![nswindow, setCollectionBehavior: NSWindowCollectionBehaviorFullScreenPrimary];
                } else {
                    let _: () = msg_send![nswindow, setCollectionBehavior: NSWindowCollectionBehaviorFullScreenNone];
                }

                // Create a default view for this window
                let rect: NSRect = {
                    let frame: NSRect = msg_send![nswindow, frame];
                    msg_send![nswindow, contentRectForFrameRect: frame]
                };
                let content_view: Id =
                    msg_send![super::alloc(sdl3_view_class()), initWithFrame: rect];
                _owned_view = StrongPtr::from_owned(content_view);
                view_set_sdl_window(content_view, window);
                nsview = content_view;
            }

            let f = flags(window);
            if f.contains(WindowFlags::ALWAYS_ON_TOP) {
                let _: () = msg_send![nswindow, setLevel: NSFloatingWindowLevel];
            }

            if f.contains(WindowFlags::TRANSPARENT) {
                let _: () = msg_send![nswindow, setOpaque: NO];
                let _: () = msg_send![nswindow, setHasShadow: NO];
                let clear: Id = msg_send![class!(NSColor), clearColor];
                let _: () = msg_send![nswindow, setBackgroundColor: clear];
            }

            // We still support OpenGL as long as Apple offers it, deprecated or not.
            /* Note: as of the macOS 10.15 SDK, this defaults to YES instead of NO when
             * the NSHighResolutionCapable boolean is set in Info.plist. */
            let highdpi = Bool::from(f.contains(WindowFlags::HIGH_PIXEL_DENSITY));
            let _: () = msg_send![nsview, setWantsBestResolutionOpenGLSurface: highdpi];

            let gles = crate::video::gl::gl_config().profile_mask
                == crate::video::gl::GL_CONTEXT_PROFILE_ES;
            if f.contains(WindowFlags::OPENGL) && gles {
                let _: () = msg_send![nsview, setWantsLayer: YES];
                let layer: Id = msg_send![nsview, layer];
                if f.contains(WindowFlags::HIGH_PIXEL_DENSITY) {
                    let screen: Id = msg_send![nswindow, screen];
                    let scale: crate::core::apple::foundation::CGFloat =
                        msg_send![screen, backingScaleFactor];
                    let _: () = msg_send![layer, setContentsScale: scale];
                } else {
                    let _: () = msg_send![layer, setContentsScale: 1.0f64];
                }
            }
            let _: () = msg_send![nswindow, setContentView: nsview];

            setup_window_data(videodata, window, nswindow, nsview)?;

            if !f.contains(WindowFlags::OPENGL) {
                return Ok(());
            }

            // The rest of this macro mess is for OpenGL or OpenGL ES windows
            if gles {
                if let Err(e) = super::opengles::gles_setup_window(window) {
                    destroy_window(window);
                    return Err(e);
                }
                return Ok(());
            }
            Ok(())
        }
    })
}

/// Translation of `Cocoa_SetWindowTitle()`.
pub(crate) fn set_window_title(window: WindowID) {
    autoreleasepool(|| {
        let title = with_window(window, |w| w.title.clone())
            .ok()
            .flatten()
            .unwrap_or_default();
        let Some(data) = window_data(window) else {
            return;
        };
        let string = ns_string(&title);
        // SAFETY: an NSWindow.
        let _: () = unsafe { msg_send![data.nswindow(), setTitle: string.get()] };
    })
}

/// Translation of `Cocoa_SetWindowIcon()`.
pub(crate) fn set_window_icon(_window: WindowID, icon: &Surface<'static>) -> Result<()> {
    autoreleasepool(|| {
        if let Some(nsimage) = super::create_image(icon) {
            // SAFETY: NSApp.
            let _: () = unsafe { msg_send![ns_app(), setApplicationIconImage: nsimage.get()] };

            return Ok(());
        }

        Err(Error::new("Unable to set the window's icon"))
    })
}

/// Translation of `Cocoa_SetWindowPosition()`.
pub(crate) fn set_window_position(window: WindowID) -> Result<()> {
    autoreleasepool(|| {
        let Some(windata) = window_data(window) else {
            return Ok(());
        };
        let nswindow = windata.nswindow();
        // SAFETY: an NSWindow.
        let mut rect: NSRect = unsafe {
            let frame: NSRect = msg_send![nswindow, frame];
            msg_send![nswindow, contentRectForFrameRect: frame]
        };
        let f = flags(window);
        let fullscreen = f.contains(WindowFlags::FULLSCREEN);

        if fullscreen || windata.is_in_fullscreen_space_transition() {
            windata.state().pending_position = true;
            return Ok(());
        }

        if !f.contains(WindowFlags::MAXIMIZED) {
            if fullscreen {
                let display = crate::video::core::display_for_fullscreen_window(window)?;
                let r = crate::video::display::display_bounds(display)?;

                rect.origin.x = r.x as f64;
                rect.origin.y = r.y as f64;
            } else {
                let pending = with_window(window, |w| w.pending)?;
                let (x, y) = relative_to_global_for_window(window, pending.x, pending.y);
                rect.origin.x = x as f64;
                rect.origin.y = y as f64;
            }
            convert_ns_rect(&mut rect);

            // Position and constrain the popup
            let constrain_popup = with_window(window, |w| w.constrain_popup)?;
            if is_popup(window) && constrain_popup {
                let screen_rect = screen_frame(screen_for_rect(&rect));

                if rect.origin.x + rect.size.width > screen_rect.origin.x + screen_rect.size.width {
                    rect.origin.x -= (rect.origin.x + rect.size.width)
                        - (screen_rect.origin.x + screen_rect.size.width);
                }
                if rect.origin.y + rect.size.height > screen_rect.origin.y + screen_rect.size.height
                {
                    rect.origin.y -= (rect.origin.y + rect.size.height)
                        - (screen_rect.origin.y + screen_rect.size.height);
                }
                rect.origin.x = rect.origin.x.max(screen_rect.origin.x);
                rect.origin.y = rect.origin.y.max(screen_rect.origin.y);
            }

            // SAFETY: an NSWindow.
            let _: () = unsafe { msg_send![nswindow, setFrameOrigin: rect.origin] };

            schedule_context_updates(&windata);
        }
        Ok(())
    })
}

/// Translation of `Cocoa_SetWindowSize()`.
pub(crate) fn set_window_size(window: WindowID) {
    autoreleasepool(|| {
        let Some(windata) = window_data(window) else {
            return;
        };
        let nswindow = windata.nswindow();

        if windata.is_in_fullscreen_space() || windata.is_in_fullscreen_space_transition() {
            windata.state().pending_size = true;
            return;
        }

        if !is_window_zoomed(window) {
            let (floating, pending) =
                with_window(window, |w| (w.core.floating, w.pending)).unwrap_or_default();

            /* Cocoa will resize the window from the bottom-left rather than the
             * top-left when -[nswindow setContentSize:] is used, so we must set the
             * entire frame based on the new size, in order to preserve the position.
             */
            let (x, y) = relative_to_global_for_window(window, floating.x, floating.y);
            let mut rect = NSRect::new(x as f64, y as f64, pending.w as f64, pending.h as f64);
            convert_ns_rect(&mut rect);

            // SAFETY: an NSWindow.
            unsafe {
                let frame: NSRect = msg_send![nswindow, frameRectForContentRect: rect];
                let _: () = msg_send![nswindow, setFrame: frame display: YES];
            }
            schedule_context_updates(&windata);
        } else {
            // Can't set the window size.
            let _ = with_window(window, |w| w.core.last_size_pending = false);
        }
    })
}

/// Translation of `Cocoa_SetWindowMinimumSize()`.
pub(crate) fn set_window_minimum_size(window: WindowID) {
    autoreleasepool(|| {
        let Some(windata) = window_data(window) else {
            return;
        };
        let (min_w, min_h) =
            with_window(window, |w| (w.core.min_w, w.core.min_h)).unwrap_or_default();

        let min_size = NSSize::new(min_w as f64, min_h as f64);

        // SAFETY: an NSWindow.
        let _: () = unsafe { msg_send![windata.nswindow(), setContentMinSize: min_size] };
    })
}

/// Translation of `Cocoa_SetWindowMaximumSize()`.
pub(crate) fn set_window_maximum_size(window: WindowID) {
    autoreleasepool(|| {
        let Some(windata) = window_data(window) else {
            return;
        };
        let (max_w, max_h) =
            with_window(window, |w| (w.core.max_w, w.core.max_h)).unwrap_or_default();

        let max_size = NSSize::new(
            if max_w != 0 { max_w as f64 } else { f64::MAX },
            if max_h != 0 { max_h as f64 } else { f64::MAX },
        );

        // SAFETY: an NSWindow.
        let _: () = unsafe { msg_send![windata.nswindow(), setContentMaxSize: max_size] };
    })
}

/// Translation of `Cocoa_SetWindowAspectRatio()`.
pub(crate) fn set_window_aspect_ratio(window: WindowID) {
    autoreleasepool(|| {
        let Some(windata) = window_data(window) else {
            return;
        };
        let (min_aspect, max_aspect) =
            with_window(window, |w| (w.min_aspect, w.max_aspect)).unwrap_or_default();

        let ratio = if min_aspect > 0.0 && min_aspect == max_aspect {
            let (numerator, denominator) = crate::utils::approximate_fraction(max_aspect);
            NSSize::new(numerator as f64, denominator as f64)
        } else {
            NSSize::new(0.0, 0.0)
        };
        // SAFETY: an NSWindow.
        let _: () = unsafe { msg_send![windata.nswindow(), setContentAspectRatio: ratio] };
    })
}

/// Translation of `Cocoa_GetWindowSizeInPixels()`.
pub(crate) fn get_window_size_in_pixels(window: WindowID) -> Option<(i32, i32)> {
    let windata = window_data(window)?;
    let viewport = windata.state().viewport;
    Some((viewport.size.width as i32, viewport.size.height as i32))
}

/// Translation of `Cocoa_ShowWindow()`.
pub(crate) fn show_window(window: WindowID) {
    autoreleasepool(|| {
        let Some(window_data_) = window_data(window) else {
            return;
        };
        let nswindow = window_data_.nswindow();
        let b_activate = hints::get_bool(hints::WINDOW_ACTIVATE_WHEN_SHOWN, true);

        // SAFETY: NSWindows and NSApp.
        unsafe {
            let miniaturized: Bool = msg_send![nswindow, isMiniaturized];
            if !miniaturized.is_true() {
                pause_visible_observation(&window_data_);
                if let Some(parent) = parent_of(window) {
                    if let Some(parent_data) = self::window_data(parent) {
                        let _: () = msg_send![parent_data.nswindow(), addChildWindow: nswindow
                                                                             ordered: NSWindowAbove];
                    }

                    if flags(window).contains(WindowFlags::MODAL) {
                        let _ = set_window_modal(window, true);
                    }
                }
                let f = flags(window);
                if !is_popup(window) {
                    if b_activate {
                        let _: () = msg_send![nswindow, makeKeyAndOrderFront: NIL];
                    } else {
                        // Order this window below the key window if we're not activating it
                        let key_window: Id = msg_send![ns_app(), keyWindow];
                        if !key_window.is_null() {
                            let number: NSInteger = msg_send![key_window, windowNumber];
                            let _: () =
                                msg_send![nswindow, orderWindow: NSWindowBelow relativeTo: number];
                        }
                    }
                } else if f.contains(WindowFlags::POPUP_MENU) {
                    if !f.contains(WindowFlags::NOT_FOCUSABLE) {
                        set_keyboard_focus(Some(window), true);
                    }
                    update_mouse_focus();
                }
            }
            let _: () = msg_send![nswindow, setIsVisible: YES];
        }
        resume_visible_observation(&window_data_);
    })
}

/// Translation of `Cocoa_HideWindow()`.
pub(crate) fn hide_window(window: WindowID) {
    autoreleasepool(|| {
        let Some(data) = window_data(window) else {
            return;
        };
        let nswindow = data.nswindow();
        // SAFETY: an NSWindow.
        unsafe {
            let waskey: Bool = msg_send![nswindow, isKeyWindow];

            /* orderOut has no effect on miniaturized windows, so close must be used to remove
             * the window from the desktop and window list in this case.
             *
             * SDL holds a strong reference to the window (oneShot/releasedWhenClosed are 'NO'),
             * and calling 'close' doesn't send a 'windowShouldClose' message, so it's safe to
             * use for this purpose as nothing is implicitly released.
             */
            let miniaturized: Bool = msg_send![nswindow, isMiniaturized];
            if !miniaturized.is_true() {
                let _: () = msg_send![nswindow, orderOut: NIL];
            } else {
                let _: () = msg_send![nswindow, close];
            }

            /* If this window is the source of a modal session, end it when
             * hidden, or other windows will be prevented from closing.
             */
            let _ = set_window_modal(window, false);

            // Transfer keyboard focus back to the parent when closing a popup menu
            let f = flags(window);
            if f.contains(WindowFlags::POPUP_MENU) && !f.contains(WindowFlags::NOT_FOCUSABLE) {
                let (new_focus, set_focus) =
                    should_relinquish_popup_focus(window).unwrap_or((None, false));
                set_keyboard_focus(new_focus, set_focus);
                update_mouse_focus();
            } else if let Some(parent) = parent_of(window) {
                if waskey.is_true() {
                    /* Key status is not automatically set on the parent when a child is hidden. Check if the
                     * child window was key, and set the first visible parent to be key if so.
                     */
                    let mut new_focus = parent;

                    while let Ok(Some(grand)) = with_window(new_focus, |w| {
                        w.parent.filter(|_| w.is_hiding || w.core.is_destroying)
                    }) {
                        new_focus = grand;
                    }

                    // FIXME (upstream): the window made key is the hidden child
                    // (window->internal), not the new_focus parent found above.
                    let _ = new_focus;
                    let _: () = msg_send![nswindow, makeKeyAndOrderFront: NIL];
                }
            }
        }
    })
}

/// Translation of `Cocoa_RaiseWindow()`.
pub(crate) fn raise_window(window: WindowID) {
    autoreleasepool(|| {
        let Some(window_data_) = window_data(window) else {
            return;
        };
        let nswindow = window_data_.nswindow();
        let b_activate = hints::get_bool(hints::WINDOW_ACTIVATE_WHEN_RAISED, true);

        /* makeKeyAndOrderFront: has the side-effect of deminiaturizing and showing
        a minimized or hidden window, so check for that before showing it.
         */
        pause_visible_observation(&window_data_);
        // SAFETY: NSWindows and NSApp.
        unsafe {
            let miniaturized: Bool = msg_send![nswindow, isMiniaturized];
            let visible: Bool = msg_send![nswindow, isVisible];
            if !miniaturized.is_true() && visible.is_true() {
                if let Some(parent) = parent_of(window) {
                    if let Some(parent_data) = self::window_data(parent) {
                        let _: () = msg_send![parent_data.nswindow(), addChildWindow: nswindow
                                                                             ordered: NSWindowAbove];
                    }
                }
                if !is_popup(window) {
                    if b_activate {
                        let _: () = msg_send![ns_app(), activateIgnoringOtherApps: YES];
                        let _: () = msg_send![nswindow, makeKeyAndOrderFront: NIL];
                    } else {
                        let _: () = msg_send![nswindow, orderFront: NIL];
                    }
                } else if b_activate {
                    let _: () = msg_send![nswindow, makeKeyWindow];
                }
            }
        }
        resume_visible_observation(&window_data_);
    })
}

/// Translation of `Cocoa_MaximizeWindow()`.
pub(crate) fn maximize_window(window: WindowID) {
    autoreleasepool(|| {
        let Some(windata) = window_data(window) else {
            return;
        };
        let nswindow = windata.nswindow();

        if windata.window_operation_is_pending(
            PENDING_OPERATION_ENTER_FULLSCREEN | PENDING_OPERATION_LEAVE_FULLSCREEN,
        ) || windata.is_in_fullscreen_space_transition()
        {
            sync_window(window);
        }

        if !flags(window).contains(WindowFlags::FULLSCREEN)
            && !windata.is_in_fullscreen_space_transition()
            && !windata.is_in_fullscreen_space()
        {
            // SAFETY: an NSWindow.
            let _: () = unsafe { msg_send![nswindow, zoom: NIL] };
            schedule_context_updates(&windata);
        } else if !windata.state().was_zoomed {
            windata.add_pending_window_operation(PENDING_OPERATION_ZOOM);
        } else {
            windata.clear_pending_window_operation(PENDING_OPERATION_ZOOM);
        }
    })
}

/// Translation of `Cocoa_MinimizeWindow()`.
pub(crate) fn minimize_window(window: WindowID) {
    autoreleasepool(|| {
        let Some(data) = window_data(window) else {
            return;
        };
        let nswindow = data.nswindow();

        data.add_pending_window_operation(PENDING_OPERATION_MINIMIZE);
        if data.is_in_fullscreen_space() || flags(window).contains(WindowFlags::FULLSCREEN) {
            data.add_pending_window_operation(PENDING_OPERATION_LEAVE_FULLSCREEN);
            let _ = crate::video::core::update_fullscreen_mode(window, FullscreenOp::Leave, true);
        } else if data.is_in_fullscreen_space_transition() {
            data.add_pending_window_operation(PENDING_OPERATION_LEAVE_FULLSCREEN);
        } else {
            // SAFETY: an NSWindow.
            let _: () = unsafe { msg_send![nswindow, miniaturize: NIL] };
        }
    })
}

/// Translation of `Cocoa_RestoreWindow()`.
pub(crate) fn restore_window(window: WindowID) {
    autoreleasepool(|| {
        let Some(data) = window_data(window) else {
            return;
        };
        let nswindow = data.nswindow();
        // SAFETY: an NSWindow.
        let miniaturized = || -> bool {
            let r: Bool = unsafe { msg_send![nswindow, isMiniaturized] };
            r.is_true()
        };

        if (data.window_operation_is_pending(
            PENDING_OPERATION_ENTER_FULLSCREEN | PENDING_OPERATION_LEAVE_FULLSCREEN,
        ) && !miniaturized())
            || data.is_in_fullscreen_space_transition()
        {
            sync_window(window);
        }

        data.clear_pending_window_operation(PENDING_OPERATION_MINIMIZE);

        if !flags(window).contains(WindowFlags::FULLSCREEN)
            && !data.is_in_fullscreen_space_transition()
            && !data.is_in_fullscreen_space()
        {
            // SAFETY: an NSWindow.
            unsafe {
                if miniaturized() {
                    let _: () = msg_send![nswindow, deminiaturize: NIL];
                } else if is_window_zoomed(window) {
                    let _: () = msg_send![nswindow, zoom: NIL];
                }
            }
        } else if data.state().was_zoomed {
            data.add_pending_window_operation(PENDING_OPERATION_ZOOM);
        } else {
            data.clear_pending_window_operation(PENDING_OPERATION_ZOOM);
        }
    })
}

/// Translation of `Cocoa_SetWindowBordered()`.
pub(crate) fn set_window_bordered(window: WindowID, bordered: bool) {
    autoreleasepool(|| {
        let Some(data) = window_data(window) else {
            return;
        };

        // If the window is in or transitioning to/from fullscreen, this will be set on leave.
        if !flags(window).contains(WindowFlags::FULLSCREEN)
            && !data.is_in_fullscreen_space_transition()
        {
            if set_window_style(&data, get_window_style(window)) && bordered {
                set_window_title(window); // this got blanked out.
            }
        } else {
            data.state().border_toggled = true;
        }
        update_clip_cursor(window);
    })
}

/// Translation of `Cocoa_SetWindowResizable()`.
pub(crate) fn set_window_resizable(window: WindowID, resizable: bool) {
    autoreleasepool(|| {
        /* Don't set this if we're in or transitioning to/from a space!
         * The window will get permanently stuck if resizable is false.
         * -flibit
         */
        let Some(data) = window_data(window) else {
            return;
        };
        let nswindow = data.nswindow();
        if !data.is_in_fullscreen_space() && !data.is_in_fullscreen_space_transition() {
            set_window_style(&data, get_window_style(window));
        }

        let allow_spaces = data.videodata.state().allow_spaces;
        // SAFETY: an NSWindow.
        unsafe {
            if resizable && allow_spaces {
                // resizable windows are Spaces-friendly: they get the "go fullscreen" toggle button on their titlebar.
                let _: () = msg_send![nswindow, setCollectionBehavior: NSWindowCollectionBehaviorFullScreenPrimary];
            } else {
                let _: () = msg_send![nswindow, setCollectionBehavior: NSWindowCollectionBehaviorFullScreenNone];
            }
        }
    })
}

/// Translation of `Cocoa_SetWindowAlwaysOnTop()`.
pub(crate) fn set_window_always_on_top(window: WindowID, on_top: bool) {
    autoreleasepool(|| {
        let Some(data) = window_data(window) else {
            return;
        };
        let nswindow = data.nswindow();

        // If the window is in or transitioning to/from fullscreen, this will be set on leave.
        if !flags(window).contains(WindowFlags::FULLSCREEN)
            && !data.is_in_fullscreen_space_transition()
        {
            // SAFETY: an NSWindow.
            unsafe {
                if on_top {
                    let _: () = msg_send![nswindow, setLevel: NSFloatingWindowLevel];
                } else {
                    let _: () = msg_send![nswindow, setLevel: kCGNormalWindowLevel];
                }
            }
        }
    })
}

/// `NSEdgeInsets`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
struct NSEdgeInsets {
    top: f64,
    left: f64,
    bottom: f64,
    right: f64,
}

/// Translation of `Cocoa_SetWindowFullscreen()`.
pub(crate) fn set_window_fullscreen(
    window: WindowID,
    display: DisplayID,
    fullscreen: FullscreenOp,
) -> FullscreenResult {
    let fullscreen = fullscreen != FullscreenOp::Leave;
    autoreleasepool(|| {
        let Some(data) = window_data(window) else {
            return FullscreenResult::Failed;
        };
        let nswindow = data.nswindow();
        let view = data.sdl_content_view.get();
        let mut rect: NSRect;

        // This is a synchronous operation, so always clear the pending flags.
        data.clear_pending_window_operation(
            PENDING_OPERATION_ENTER_FULLSCREEN | PENDING_OPERATION_LEAVE_FULLSCREEN,
        );

        // The view responder chain gets messed with during setStyleMask
        if next_responder(view) == data.listener() {
            set_next_responder(view, NIL);
        }

        // SAFETY: an NSWindow and its screen.
        unsafe {
            if fullscreen {
                if !flags(window).contains(WindowFlags::FULLSCREEN) {
                    data.state().was_zoomed = flags(window).contains(WindowFlags::MAXIMIZED);
                }

                send(window, EventType::WINDOW_ENTER_FULLSCREEN, 0, 0);
                let bounds = super::modes::get_display_bounds(display).unwrap_or_default();
                rect = NSRect::new(
                    bounds.x as f64,
                    bounds.y as f64,
                    bounds.w as f64,
                    bounds.h as f64,
                );
                convert_ns_rect(&mut rect);

                /* Hack to fix origin on macOS 10.4
                This is no longer needed as of macOS 10.15, according to bug 4822.
                 */
                if appkit_version_number().floor() <= NSAppKitVersionNumber10_14 {
                    let screen: Id = msg_send![nswindow, screen];
                    let screen_rect = screen_frame(screen);
                    if screen_rect.size.height >= 1.0 {
                        rect.origin.y += screen_rect.size.height - rect.size.height;
                    }
                }

                let _: () = msg_send![nswindow, setStyleMask: NSWindowStyleMaskBorderless];
            } else {
                send(window, EventType::WINDOW_LEAVE_FULLSCREEN, 0, 0);

                let (pending_position, was_zoomed) = {
                    let st = data.state();
                    (st.pending_position, st.was_zoomed)
                };
                let (pending, windowed, floating) =
                    with_window(window, |w| (w.pending, w.core.windowed, w.core.floating))
                        .unwrap_or_default();
                rect = NSRect::default();
                if pending_position {
                    rect.origin.x = pending.x as f64;
                    rect.origin.y = pending.y as f64;
                } else {
                    rect.origin.x = if was_zoomed { windowed.x } else { floating.x } as f64;
                    rect.origin.y = if was_zoomed { windowed.y } else { floating.y } as f64;
                }
                rect.size.width = if was_zoomed { windowed.w } else { floating.w } as f64;
                rect.size.height = if was_zoomed { windowed.h } else { floating.h } as f64;

                convert_ns_rect(&mut rect);

                /* The window is not meant to be fullscreen, but its flags might have a
                 * fullscreen bit set if it's scheduled to go fullscreen immediately
                 * after. Always using the windowed mode style here works around bugs in
                 * macOS 10.15 where the window doesn't properly restore the windowed
                 * mode decorations after exiting fullscreen-desktop, when the window
                 * was created as fullscreen-desktop. */
                let _: () = msg_send![nswindow, setStyleMask: get_window_windowed_style(window)];

                // Hack to restore window decorations on macOS 10.10
                let frame_rect: NSRect = msg_send![nswindow, frame];
                let wider = NSRect::new(
                    frame_rect.origin.x,
                    frame_rect.origin.y,
                    frame_rect.size.width + 1.0,
                    frame_rect.size.height,
                );
                let _: () = msg_send![nswindow, setFrame: wider display: NO];
                let _: () = msg_send![nswindow, setFrame: frame_rect display: NO];
            }

            // The view responder chain gets messed with during setStyleMask
            if next_responder(view) != data.listener() {
                set_next_responder(view, data.listener());
            }

            let _: () = msg_send![nswindow, setContentSize: rect.size];
            let _: () = msg_send![nswindow, setFrameOrigin: rect.origin];

            // Disable the window shadow in fullscreen to avoid a visible 1px border on Tahoe
            let has_shadow = !fullscreen && !flags(window).contains(WindowFlags::TRANSPARENT);
            let _: () = msg_send![nswindow, setHasShadow: Bool::from(has_shadow)];

            // When the window style changes the title is cleared
            if !fullscreen {
                set_window_title(window);
                data.state().was_zoomed = false;
                if data.window_operation_is_pending(PENDING_OPERATION_ZOOM) {
                    data.clear_pending_window_operation(PENDING_OPERATION_ZOOM);
                    let _: () = msg_send![nswindow, zoom: NIL];
                }
            }

            if should_allow_topmost() && fullscreen {
                // OpenGL is rendering to the window, so make it visible!
                let _: () = msg_send![nswindow, setLevel: kCGMainMenuWindowLevel + 1];
            } else if flags(window).contains(WindowFlags::ALWAYS_ON_TOP) {
                let _: () = msg_send![nswindow, setLevel: NSFloatingWindowLevel];
            } else {
                let _: () = msg_send![nswindow, setLevel: kCGNormalWindowLevel];
            }

            let visible: Bool = msg_send![nswindow, isVisible];
            if visible.is_true() || fullscreen {
                pause_visible_observation(&data);
                let _: () = msg_send![nswindow, makeKeyAndOrderFront: NIL];
                resume_visible_observation(&data);
            }

            // Update the safe area insets
            // The view never seems to reflect the safe area, so we'll use the screen instead
            if macos_available(12, 0) {
                if fullscreen {
                    let screen: Id = msg_send![nswindow, screen];
                    let insets: NSEdgeInsets = if screen.is_null() {
                        NSEdgeInsets::default()
                    } else {
                        msg_send![screen, safeAreaInsets]
                    };

                    crate::video::core::set_window_safe_area_insets(
                        data.window,
                        (insets.left as f32).ceil() as i32,
                        (insets.right as f32).ceil() as i32,
                        (insets.top as f32).ceil() as i32,
                        (insets.bottom as f32).ceil() as i32,
                    );
                } else {
                    crate::video::core::set_window_safe_area_insets(data.window, 0, 0, 0, 0);
                }
            }

            /* When coming out of fullscreen to minimize, this needs to happen after the window
             * is made key again, or it won't minimize on 15.0 (Sequoia).
             */
            if !fullscreen && data.window_operation_is_pending(PENDING_OPERATION_MINIMIZE) {
                wait_for_miniaturizable(window);
                data.add_pending_window_operation(PENDING_OPERATION_ENTER_FULLSCREEN);
                data.clear_pending_window_operation(PENDING_OPERATION_MINIMIZE);
                let _: () = msg_send![nswindow, miniaturize: NIL];
            }
        }

        schedule_context_updates(&data);
        sync_window(window);
        update_clip_cursor(window);

        FullscreenResult::Succeeded
    })
}

/// Translation of `Cocoa_GetWindowICCProfile()`.
pub(crate) fn get_window_icc_profile(window: WindowID) -> Result<Vec<u8>> {
    autoreleasepool(|| {
        let data = window_data(window).ok_or_else(|| Error::new("Window data not set"))?;
        // SAFETY: an NSWindow, its NSScreen, NSColorSpace and NSData.
        unsafe {
            let screen: Id = msg_send![data.nswindow(), screen];

            if screen.is_null() {
                return Err(Error::new("Could not get screen of window."));
            }

            let color_space: Id = msg_send![screen, colorSpace];
            if color_space.is_null() {
                return Err(Error::new(
                    "Could not get colorspace information of screen.",
                ));
            }

            let icc_profile_data: Id = msg_send![color_space, ICCProfileData];
            if icc_profile_data.is_null() {
                return Err(Error::new("Could not get ICC profile data."));
            }

            let length: NSUInteger = msg_send![icc_profile_data, length];
            let mut ret = vec![0u8; length];
            let _: () = msg_send![icc_profile_data, getBytes: ret.as_mut_ptr() as *mut c_void
                                                          length: length];
            Ok(ret)
        }
    })
}

/// Translation of `Cocoa_GetDisplayForWindow()`.
pub(crate) fn get_display_for_window(window: WindowID) -> DisplayID {
    autoreleasepool(|| {
        // Not recognized via CHECK_WINDOW_MAGIC
        let Some(data) = window_data(window) else {
            // Don't set the error here, it hides other errors and is ignored anyway
            // return SDL_SetError("Window data not set");
            return 0;
        };

        // NSWindow.screen may be nil when the window is off-screen.
        // SAFETY: an NSWindow.
        let screen: Id = unsafe { msg_send![data.nswindow(), screen] };

        if !screen.is_null() {
            // https://developer.apple.com/documentation/appkit/nsscreen/1388360-devicedescription?language=objc
            // SAFETY: an NSScreen.
            let displayid = unsafe { super::modes::screen_display_id(screen) };
            if let Some(display) = super::modes::find_sdl_display_by_cg_direct_display_id(displayid)
            {
                return display;
            }
        }

        // The higher level code will use other logic to find the display
        0
    })
}

/// Translation of `Cocoa_SetWindowMouseRect()`.
pub(crate) fn set_window_mouse_rect(window: WindowID) -> Result<()> {
    update_clip_cursor(window);
    Ok(())
}

/// Translation of `Cocoa_SetWindowMouseGrab()`.
pub(crate) fn set_window_mouse_grab(window: WindowID, _grabbed: bool) -> Result<()> {
    autoreleasepool(|| {
        update_clip_cursor(window);

        let f = flags(window);
        if let Some(data) = window_data(window).filter(|_| f.contains(WindowFlags::FULLSCREEN)) {
            // SAFETY: an NSWindow.
            unsafe {
                if should_allow_topmost()
                    && f.contains(WindowFlags::INPUT_FOCUS)
                    && !data.is_in_fullscreen_space()
                {
                    // OpenGL is rendering to the window, so make it visible!
                    // Doing this in 10.11 while in a Space breaks things (bug #3152)
                    let _: () = msg_send![data.nswindow(), setLevel: kCGMainMenuWindowLevel + 1];
                } else if f.contains(WindowFlags::ALWAYS_ON_TOP) {
                    let _: () = msg_send![data.nswindow(), setLevel: NSFloatingWindowLevel];
                } else {
                    let _: () = msg_send![data.nswindow(), setLevel: kCGNormalWindowLevel];
                }
            }
        }
        Ok(())
    })
}

/// Translation of `Cocoa_DestroyWindow()`.
pub(crate) fn destroy_window(window: WindowID) {
    autoreleasepool(|| {
        let data = with_window(window, |w| {
            w.internal
                .take()
                .and_then(|i| i.downcast::<Arc<WindowData>>().ok())
                .map(|b| *b)
        })
        .ok()
        .flatten();

        if let Some(data) = data {
            let topmost = get_parent_toplevel_window(window);

            /* Reset the input focus of the root window if this window is still set as keyboard focus.
             * SDL_DestroyWindow will have already taken care of reassigning focus if this is the SDL
             * keyboard focus, this ensures that an inactive window with this window set as input focus
             * does not try to reference it the next time it gains focus.
             */
            if with_window(topmost, |w| w.keyboard_focus).ok().flatten() == Some(window) {
                let mut new_focus = window;
                while let Ok(Some(parent)) = with_window(new_focus, |w| {
                    w.parent
                        .filter(|_| w.is_popup() && (w.is_hiding || w.core.is_destroying))
                }) {
                    new_focus = parent;
                }

                let _ = with_window(topmost, |w| w.keyboard_focus = Some(new_focus));
            }

            if data.is_in_fullscreen_space() {
                set_menu_bar_visible(true);
            }
            close_listener(&data);
            let listener =
                std::mem::take(&mut *data.listener.lock().unwrap_or_else(|e| e.into_inner()));
            drop(listener);

            if !flags(window).contains(WindowFlags::EXTERNAL) {
                // SAFETY: the window we created.
                unsafe {
                    // Release the content view to avoid further updateLayer callbacks
                    let _: () = msg_send![data.nswindow(), setContentView: NIL];
                    let _: () = msg_send![data.nswindow(), close];
                }
            }

            let contexts: Vec<StrongPtr> = data.state().nscontexts.clone();
            for context in &contexts {
                // Calling setWindow:NULL causes the context to remove itself from the context list.
                // SAFETY: an SDL3OpenGLContext.
                let _: () =
                    unsafe { msg_send![context, setWindow: std::ptr::null_mut::<c_void>()] };
            }
        }
    })
}

/// Translation of `Cocoa_SetWindowFullscreenSpace()`.
pub(crate) fn set_window_fullscreen_space(window: WindowID, state: bool, blocking: bool) -> bool {
    autoreleasepool(|| {
        let mut succeeded = false;
        let Some(data) = window_data(window) else {
            return false;
        };

        {
            let mut st = data.state();
            if state {
                st.fullscreen_space_requested = true;
            }
            st.in_blocking_transition = blocking;
        }
        if set_fullscreen_space(&data, state) {
            if blocking {
                let maxattempts = 3;
                let mut attempt = 0;
                loop {
                    attempt += 1;
                    if attempt > maxattempts {
                        break;
                    }
                    /* Wait for the transition to complete, so application changes
                    take effect properly (e.g. setting the window size, etc.)
                     */
                    let limit = 10000;
                    let mut count = 0;
                    while data.is_in_fullscreen_space_transition() {
                        count += 1;
                        if count == limit {
                            // Uh oh, transition isn't completing. Should we assert?
                            break;
                        }
                        crate::timer::delay(std::time::Duration::from_millis(1));
                        crate::events::pump();
                    }
                    if data.is_in_fullscreen_space() == state {
                        break;
                    }
                    // Try again, the last attempt was interrupted by user gestures
                    if !set_fullscreen_space(&data, state) {
                        break; // ???
                    }
                }
            }

            // Return TRUE to prevent non-space fullscreen logic from running
            succeeded = true;
        }

        data.state().in_blocking_transition = false;
        succeeded
    })
}

/// Translation of `Cocoa_SetWindowHitTest()`.
pub(crate) fn set_window_hit_test(window: WindowID, _enabled: bool) -> Result<()> {
    if let Some(data) = window_data(window) {
        update_hit_test(&data);
    }
    Ok(())
}

/// Translation of `Cocoa_AcceptDragAndDrop()`.
pub(crate) fn accept_drag_and_drop(window: WindowID, accept: bool) {
    autoreleasepool(|| {
        let Some(data) = window_data(window) else {
            return;
        };
        // SAFETY: an NSWindow and an array of UTI strings.
        unsafe {
            if accept {
                let types = [cf_as_id(kUTTypeFileURL), cf_as_id(kUTTypeUTF8PlainText)];
                let array: Id = msg_send![class!(NSArray),
                    arrayWithObjects: types.as_ptr() count: types.len() as NSUInteger];
                let _: () = msg_send![data.nswindow(), registerForDraggedTypes: array];
            } else {
                let _: () = msg_send![data.nswindow(), unregisterDraggedTypes];
            }
        }
    })
}

/// Translation of `Cocoa_SetWindowParent()`.
pub(crate) fn set_window_parent(window: WindowID, parent: Option<WindowID>) -> Result<()> {
    autoreleasepool(|| {
        let Some(child_data) = window_data(window) else {
            return Ok(());
        };

        // SAFETY: NSWindows.
        unsafe {
            // Remove an existing parent.
            let parent_window: Id = msg_send![child_data.nswindow(), parentWindow];
            if !parent_window.is_null() {
                if let Some(old) = parent_of(window).and_then(window_data) {
                    let _: () = msg_send![old.nswindow(), removeChildWindow: child_data.nswindow()];
                }
            }

            if let Some(parent_data) = parent.and_then(window_data) {
                let _: () = msg_send![parent_data.nswindow(), addChildWindow: child_data.nswindow()
                                                                     ordered: NSWindowAbove];
            }
        }
        Ok(())
    })
}

/// Translation of `Cocoa_SetWindowModal()`.
pub(crate) fn set_window_modal(window: WindowID, modal: bool) -> Result<()> {
    autoreleasepool(|| {
        let Some(data) = window_data(window) else {
            return Ok(());
        };

        let session = std::mem::replace(&mut data.state().modal_session, std::ptr::null_mut());
        // SAFETY: NSApp's modal sessions.
        unsafe {
            if !session.is_null() {
                let _: () = msg_send![ns_app(), endModalSession: session];
            }

            if modal {
                let session: NSModalSession =
                    msg_send![ns_app(), beginModalSessionForWindow: data.nswindow()];
                data.state().modal_session = session;
            }
        }
        Ok(())
    })
}

/// Translation of `Cocoa_FlashWindow()`.
pub(crate) fn flash_window(window: WindowID, operation: FlashOperation) -> Result<()> {
    autoreleasepool(|| {
        // Note that this is app-wide and not window-specific!
        let Some(data) = window_data(window) else {
            return Ok(());
        };

        let request = std::mem::take(&mut data.state().flash_request);
        // SAFETY: NSApp's attention requests.
        unsafe {
            if request != 0 {
                let _: () = msg_send![ns_app(), cancelUserAttentionRequest: request];
            }

            match operation {
                FlashOperation::Cancel => {
                    // Canceled above
                }
                FlashOperation::Briefly => {
                    let r: NSInteger =
                        msg_send![ns_app(), requestUserAttention: NSInformationalRequest];
                    data.state().flash_request = r;
                }
                FlashOperation::UntilFocused => {
                    let r: NSInteger = msg_send![ns_app(), requestUserAttention: NSCriticalRequest];
                    data.state().flash_request = r;
                }
            }
        }
        Ok(())
    })
}

/// Translation of `Cocoa_SetWindowFocusable()`.
pub(crate) fn set_window_focusable(window: WindowID, focusable: bool) -> Result<()> {
    let f = flags(window);
    if f.contains(WindowFlags::POPUP_MENU) && !f.contains(WindowFlags::HIDDEN) {
        if !focusable && f.contains(WindowFlags::INPUT_FOCUS) {
            let (new_focus, set_focus) =
                should_relinquish_popup_focus(window).unwrap_or((None, false));
            set_keyboard_focus(new_focus, set_focus);
        } else if focusable && should_focus_popup(window) {
            set_keyboard_focus(Some(window), true);
        }
    }

    Ok(()) // just succeed, the real work is done elsewhere.
}

/// Translation of `Cocoa_SetWindowOpacity()`.
pub(crate) fn set_window_opacity(window: WindowID, opacity: f32) -> Result<()> {
    autoreleasepool(|| {
        if let Some(data) = window_data(window) {
            // SAFETY: an NSWindow.
            let _: () = unsafe { msg_send![data.nswindow(), setAlphaValue: opacity as f64] };
        }
        Ok(())
    })
}

/// Pump events until the window's pending operations are done (for up to
/// 2.5 seconds). Translation of `Cocoa_SyncWindow()`.
pub(crate) fn sync_window(window: WindowID) -> bool {
    let mut result = false;

    autoreleasepool(|| {
        let timeout = crate::timer::ticks_ns() + 2_500_000_000;
        let Some(data) = window_data(window) else {
            result = true;
            return;
        };

        loop {
            crate::events::pump();

            result = !data.has_pending_window_operation();
            if result || crate::timer::ticks_ns() >= timeout {
                break;
            }

            // Small delay before going again.
            crate::timer::delay(std::time::Duration::from_millis(10));
        }
    });

    result
}

/// Invalidate the cursor rectangles of every window, on the main thread
/// (`Cocoa_ShowCursor()`'s `dispatch_async()`).
pub(crate) fn invalidate_cursor_rects_async() {
    for window in window_ids() {
        if let Some(data) = window_data(window) {
            let retained = data.nswindow.clone();
            run_on_main_async(move || {
                let nswindow = retained.get();
                // SAFETY: the window, kept alive by the retained reference.
                unsafe {
                    let view: Id = msg_send![nswindow, contentView];
                    let _: () = msg_send![nswindow, invalidateCursorRectsForView: view];
                }
            });
        }
    }
}

/// The `BOOL` type encoding, re-exported for the other classes.
#[allow(dead_code)]
pub(crate) const BOOL_TYPE: &str = BOOL_ENC;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distance_to_rect_keeps_upstream_bottom_edge() {
        // Note (upstream): the bottom edge is the rectangle's minimum x (see
        // the FIXME in sq_distance_to_rect()), so a rectangle from x = 100
        // has its "bottom" at y = 100.
        let r = NSRect::new(100.0, 0.0, 50.0, 50.0);
        // Inside the real rectangle, 75 units under that bottom.
        assert_eq!(
            sq_distance_to_rect(&NSPoint::new(120.0, 25.0), &r),
            75.0 * 75.0
        );
        // Above the top (50) but under the bottom (100): measured to y = 100.
        assert_eq!(
            sq_distance_to_rect(&NSPoint::new(120.0, 60.0), &r),
            40.0 * 40.0
        );
        // Left of the rectangle and above both: to (100, 50).
        assert_eq!(
            sq_distance_to_rect(&NSPoint::new(90.0, 120.0), &r),
            100.0 + 70.0 * 70.0
        );
        // Where x is the same as the bottom, nothing changes.
        let square = NSRect::new(0.0, 0.0, 10.0, 10.0);
        assert_eq!(sq_distance_to_rect(&NSPoint::new(5.0, 5.0), &square), 0.0);
        assert_eq!(
            sq_distance_to_rect(&NSPoint::new(13.0, 14.0), &square),
            9.0 + 16.0
        );
    }

    #[test]
    fn menu_visibility_hint_values() {
        for (value, expected) in [
            (None, COCOA_MENU_VISIBILITY_AUTO),
            (Some("0"), COCOA_MENU_VISIBILITY_NEVER),
            (Some("false"), COCOA_MENU_VISIBILITY_NEVER),
            (Some("1"), COCOA_MENU_VISIBILITY_ALWAYS),
            (Some("TRUE"), COCOA_MENU_VISIBILITY_ALWAYS),
            (Some("auto"), COCOA_MENU_VISIBILITY_AUTO),
        ] {
            // (off the main thread, so only the hint is stored)
            menu_visibility_callback(value);
            assert_eq!(
                MENU_VISIBILITY_HINT.load(Ordering::Relaxed),
                expected,
                "{value:?}"
            );
        }
        menu_visibility_callback(None);
    }
}
