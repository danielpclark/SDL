// Rust translation of src/video/cocoa/SDL_cocoamouse.m and
// SDL_cocoamouse.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The mouse: cursors (images, animated ones with a frame timer, the system
//! cursors including the ones HIServices keeps as PDFs), warping, relative
//! mode, the global mouse state, relative motion from `NSEvent`s, the
//! scroll wheel, and raw input from `GCMouse` (GameController, loaded at run
//! time as upstream links it weakly).

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use super::events::get_event_timestamp;
use super::sys::*;
use super::window::{find_sdl_window, window_data};
use super::{video_data, VideoData};
use crate::core::apple::block::{dispatch_get_global_queue, dispatch_queue_create, RcBlock};
use crate::core::apple::foundation::{
    available, ns_array_items, ns_string, NSPoint, NSRect, NSSize,
};
use crate::core::apple::objc::{
    autoreleasepool, class, msg_send, Bool, Id, NSInteger, NSUInteger, StrongPtr, NIL, YES,
};
use crate::error::{Error, Result};
use crate::events::mouse::{
    add_mouse, mouse_focus, remove_mouse, send_mouse_button, send_mouse_motion, send_mouse_wheel,
    set_default_cursor, set_mouse_focus, with_mouse, Cursor, CursorFrame, MouseButtonFlags,
    MouseID, MouseWheelDirection, SystemCursor, BUTTON_LEFT, BUTTON_MIDDLE, BUTTON_RIGHT,
    BUTTON_X1, DEFAULT_MOUSE_ID, GLOBAL_MOUSE_ID,
};
use crate::events::WindowID;
use crate::hints;
use crate::video::Surface;

/// The frames of a cursor and its animation state. Translation of
/// `struct SDL_CursorData`.
pub(crate) struct CursorData {
    /// The `NSCursor`s and their durations (`frames`, `num_cursors`).
    frames: Vec<(StrongPtr, u32)>,
    /// `frameTimer` and `current_frame`.
    state: Mutex<(StrongPtr, usize)>,
}

impl CursorData {
    fn new(frames: Vec<(StrongPtr, u32)>) -> Arc<CursorData> {
        Arc::new(CursorData {
            frames,
            state: Mutex::new((StrongPtr::nil(), 0)),
        })
    }

    fn st(&self) -> std::sync::MutexGuard<'_, (StrongPtr, usize)> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The `NSCursor` being shown.
    pub(crate) fn current_cursor(&self) -> Id {
        let frame = self.st().1;
        self.frames.get(frame).map_or(NIL, |f| f.0.get())
    }

    /// Advance to the next frame (`Cocoa_IncrementCursorFrame()`).
    pub(crate) fn next_frame(&self) {
        let mut st = self.st();
        st.1 = (st.1 + 1) % self.frames.len().max(1);
    }

    /// Start the timer of the current frame of an animated cursor, if it
    /// isn't running (`Cocoa_GetDesiredCursor()`).
    pub(crate) fn schedule_frame_timer(self: &Arc<Self>) {
        let (running, frame) = {
            let st = self.st();
            (!st.0.is_nil(), st.1)
        };
        let duration = self.frames.get(frame).map_or(0, |f| f.1);
        if self.frames.len() > 1 && duration != 0 && !running {
            let interval = duration as f64 * 0.001;
            let weak = Arc::downgrade(self);
            let block = RcBlock::new1(move |_timer: Id| {
                if let Some(cdata) = weak.upgrade() {
                    cdata.st().0 = StrongPtr::nil();
                    super::window::increment_cursor_frame();
                }
            });
            // SAFETY: NSTimer and the current run loop; the timer keeps its
            // block and the run loop the timer.
            unsafe {
                let timer: Id = msg_send![class!(NSTimer), timerWithTimeInterval: interval
                                                                         repeats: false
                                                                           block: block.as_ptr()];
                self.st().0 = StrongPtr::retain(timer);

                let run_loop: Id = msg_send![class!(NSRunLoop), currentRunLoop];
                let _: () = msg_send![run_loop, addTimer: timer forMode: NSRunLoopCommonModes];
            }
        }
    }

    /// Stop the frame timer and go back to the first frame.
    fn reset(&self) {
        let timer = {
            let mut st = self.st();
            st.1 = 0;
            std::mem::take(&mut st.0)
        };
        if !timer.is_nil() {
            // SAFETY: an NSTimer.
            let _: () = unsafe { msg_send![timer, invalidate] };
        }
    }
}

impl Drop for CursorData {
    /// Translation of `Cocoa_FreeCursor()` (the cursors are released with
    /// the frames).
    fn drop(&mut self) {
        let timer = std::mem::take(&mut self.st().0);
        if !timer.is_nil() {
            // SAFETY: an NSTimer.
            let _: () = unsafe { msg_send![timer, invalidate] };
        }
    }
}

/// The driver data of a cursor.
pub(crate) fn cursor_data(cursor: &Cursor) -> Option<Arc<CursorData>> {
    cursor.internal::<Arc<CursorData>>().cloned()
}

/// Translation of `+[NSCursor invisibleCursor]`.
pub(crate) fn invisible_cursor() -> Id {
    static INVISIBLE_CURSOR: OnceLock<StrongPtr> = OnceLock::new();
    INVISIBLE_CURSOR
        .get_or_init(|| {
            let size = 32;
            // SAFETY: NSImage, NSBitmapImageRep and NSCursor as documented.
            unsafe {
                let cursor_image: Id = msg_send![super::alloc(class!(NSImage)),
                    initWithSize: NSSize::new(size as f64, size as f64)];
                let cursor_image = StrongPtr::from_owned(cursor_image);
                let imgrep: Id = msg_send![super::alloc(class!(NSBitmapImageRep)),
                    initWithBitmapDataPlanes: std::ptr::null_mut::<*mut u8>()
                    pixelsWide: size as NSInteger
                    pixelsHigh: size as NSInteger
                    bitsPerSample: 8 as NSInteger
                    samplesPerPixel: 4 as NSInteger
                    hasAlpha: YES
                    isPlanar: false
                    colorSpaceName: NSDeviceRGBColorSpace
                    bytesPerRow: (size * 4) as NSInteger
                    bitsPerPixel: 32 as NSInteger];
                let imgrep = StrongPtr::from_owned(imgrep);
                let _: () = msg_send![cursor_image, addRepresentation: imgrep.get()];

                let cursor: Id = msg_send![super::alloc(class!(NSCursor)),
                    initWithImage: cursor_image.get() hotSpot: NSPoint::new(0.0, 0.0)];
                StrongPtr::from_owned(cursor)
            }
        })
        .get()
}

/// Translation of `Cocoa_CreateAnimatedCursor()`.
pub(crate) fn create_animated_cursor(
    frames: &[CursorFrame<'_>],
    hot_x: i32,
    hot_y: i32,
) -> Result<Cursor> {
    autoreleasepool(|| {
        let mut cursors = Vec::with_capacity(frames.len());

        for frame in frames {
            let nscursor = super::create_image(frame.surface).map(|nsimage| {
                // SAFETY: NSCursor's initializer with an image.
                unsafe {
                    let c: Id = msg_send![super::alloc(class!(NSCursor)),
                        initWithImage: nsimage.get()
                        hotSpot: NSPoint::new(hot_x as f64, hot_y as f64)];
                    StrongPtr::from_owned(c)
                }
            });

            match nscursor {
                Some(nscursor) if !nscursor.is_nil() => {
                    cursors.push((nscursor, frame.duration));
                }
                _ => {
                    // (the cursors made so far are released with the vector)
                    // Note (upstream): the C cleanup releases frames[i], the
                    // failed frame, i times instead of each earlier frame;
                    // here the vector releases each one once.
                    return Err(Error::new("Couldn't create the cursor"));
                }
            }
        }

        Ok(Cursor::with_internal(CursorData::new(cursors)))
    })
}

/// Translation of `Cocoa_CreateCursor()`.
pub(crate) fn create_cursor(surface: &Surface<'_>, hot_x: i32, hot_y: i32) -> Result<Cursor> {
    let frame = CursorFrame {
        surface,
        duration: 0,
    };

    create_animated_cursor(&[frame], hot_x, hot_y)
}

/// `[NSCursor fallback]`.
fn named_cursor(fallback: &str) -> Id {
    // SAFETY: NSCursor's class methods returning cursors.
    unsafe {
        match fallback {
            "arrowCursor" => msg_send![class!(NSCursor), arrowCursor],
            "closedHandCursor" => msg_send![class!(NSCursor), closedHandCursor],
            "resizeLeftRightCursor" => msg_send![class!(NSCursor), resizeLeftRightCursor],
            "resizeUpDownCursor" => msg_send![class!(NSCursor), resizeUpDownCursor],
            "zoomInCursor" => {
                // (private before macOS 15: answers only where it exists)
                let cls = class!(NSCursor);
                if crate::core::apple::objc::has_class_method(
                    cls,
                    crate::core::apple::objc::sel!(zoomInCursor),
                ) {
                    msg_send![cls, zoomInCursor]
                } else {
                    msg_send![cls, arrowCursor]
                }
            }
            "zoomOutCursor" => {
                let cls = class!(NSCursor);
                if crate::core::apple::objc::has_class_method(
                    cls,
                    crate::core::apple::objc::sel!(zoomOutCursor),
                ) {
                    msg_send![cls, zoomOutCursor]
                } else {
                    msg_send![cls, arrowCursor]
                }
            }
            _ => msg_send![class!(NSCursor), arrowCursor],
        }
    }
}

/// there are .pdf files of some of the cursors we need, installed by default on macOS, but not available through NSCursor.
/// If we can load them ourselves, use them, otherwise fallback to something standard but not super-great.
/// Since these are under /System, they should be available even to sandboxed apps.
/// Translation of `LoadHiddenSystemCursor()`.
fn load_hidden_system_cursor(cursor_name: &str, fallback: &str) -> StrongPtr {
    // SAFETY: NSString, NSDictionary, NSImage and NSCursor as documented.
    unsafe {
        let base = ns_string("/System/Library/Frameworks/ApplicationServices.framework/Versions/A/Frameworks/HIServices.framework/Versions/A/Resources/cursors");
        let name = ns_string(cursor_name);
        let cursor_path: Id = msg_send![base, stringByAppendingPathComponent: name.get()];
        let plist = ns_string("info.plist");
        let info_path: Id = msg_send![cursor_path, stringByAppendingPathComponent: plist.get()];
        let info: Id = msg_send![class!(NSDictionary), dictionaryWithContentsOfFile: info_path];
        // we can't do animation atm.  :/
        let frames_key = ns_string("frames");
        let frames_value: Id = msg_send![info, valueForKey: frames_key.get()];
        let frames: NSInteger = msg_send![frames_value, integerValue];
        let pdf = ns_string("cursor.pdf");
        let pdf_path: Id = msg_send![cursor_path, stringByAppendingPathComponent: pdf.get()];
        let mut image = StrongPtr::from_owned(
            msg_send![super::alloc(class!(NSImage)), initWithContentsOfFile: pdf_path],
        );
        let valid: Bool = if image.is_nil() {
            false.into()
        } else {
            msg_send![image, isValid]
        };
        if !valid.is_true() {
            return StrongPtr::retain(named_cursor(fallback));
        }

        if frames > 1 {
            let operation = NSCompositingOperationCopy;
            let size: NSSize = msg_send![image, size];
            let cropped_size =
                NSSize::new(size.width, ((size.height / frames as f64) as i32) as f64);
            let cropped = StrongPtr::from_owned(
                msg_send![super::alloc(class!(NSImage)), initWithSize: cropped_size],
            );
            if cropped.is_nil() {
                return StrongPtr::retain(named_cursor(fallback));
            }

            let _: () = msg_send![cropped, lockFocus];
            {
                let cropped_rect = NSRect::new(0.0, 0.0, cropped_size.width, cropped_size.height);
                let _: () = msg_send![image, drawInRect: cropped_rect
                                                fromRect: cropped_rect
                                               operation: operation
                                                fraction: 1.0f64];
            }
            let _: () = msg_send![cropped, unlockFocus];
            image = cropped;
        }

        let hotx_key = ns_string("hotx");
        let hoty_key = ns_string("hoty");
        let hotx_value: Id = msg_send![info, valueForKey: hotx_key.get()];
        let hoty_value: Id = msg_send![info, valueForKey: hoty_key.get()];
        let hotx: f64 = msg_send![hotx_value, doubleValue];
        let hoty: f64 = msg_send![hoty_value, doubleValue];
        StrongPtr::from_owned(msg_send![super::alloc(class!(NSCursor)),
            initWithImage: image.get() hotSpot: NSPoint::new(hotx, hoty)])
    }
}

/// Translation of `Cocoa_CreateSystemCursor()`.
pub(crate) fn create_system_cursor(id: SystemCursor) -> Result<Cursor> {
    autoreleasepool(|| {
        // SAFETY: NSCursor's class methods returning cursors.
        let nscursor: StrongPtr = unsafe {
            let named = |f: &str| StrongPtr::retain(named_cursor(f));
            match id {
                SystemCursor::Default => named("arrowCursor"),
                SystemCursor::Text => StrongPtr::retain(msg_send![class!(NSCursor), IBeamCursor]),
                SystemCursor::Crosshair => {
                    StrongPtr::retain(msg_send![class!(NSCursor), crosshairCursor])
                }
                SystemCursor::Wait => {
                    // !!! FIXME: this is more like WAITARROW
                    load_hidden_system_cursor("busybutclickable", "arrowCursor")
                }
                SystemCursor::Progress => {
                    // !!! FIXME: this is meant to be animated
                    load_hidden_system_cursor("busybutclickable", "arrowCursor")
                }
                SystemCursor::NwseResize => {
                    load_hidden_system_cursor("resizenorthwestsoutheast", "closedHandCursor")
                }
                SystemCursor::NeswResize => {
                    load_hidden_system_cursor("resizenortheastsouthwest", "closedHandCursor")
                }
                SystemCursor::EwResize => {
                    load_hidden_system_cursor("resizeeastwest", "resizeLeftRightCursor")
                }
                SystemCursor::NsResize => {
                    load_hidden_system_cursor("resizenorthsouth", "resizeUpDownCursor")
                }
                SystemCursor::Move | SystemCursor::AllScroll => {
                    load_hidden_system_cursor("move", "closedHandCursor")
                }
                SystemCursor::NotAllowed | SystemCursor::NoDrop => {
                    StrongPtr::retain(msg_send![class!(NSCursor), operationNotAllowedCursor])
                }
                SystemCursor::Pointer => {
                    StrongPtr::retain(msg_send![class!(NSCursor), pointingHandCursor])
                }
                SystemCursor::NwResize => {
                    load_hidden_system_cursor("resizenorthwestsoutheast", "closedHandCursor")
                }
                SystemCursor::NResize => {
                    load_hidden_system_cursor("resizenorthsouth", "resizeUpDownCursor")
                }
                SystemCursor::NeResize => {
                    load_hidden_system_cursor("resizenortheastsouthwest", "closedHandCursor")
                }
                SystemCursor::EResize => {
                    load_hidden_system_cursor("resizeeastwest", "resizeLeftRightCursor")
                }
                SystemCursor::SeResize => {
                    load_hidden_system_cursor("resizenorthwestsoutheast", "closedHandCursor")
                }
                SystemCursor::SResize => {
                    load_hidden_system_cursor("resizenorthsouth", "resizeUpDownCursor")
                }
                SystemCursor::SwResize => {
                    load_hidden_system_cursor("resizenortheastsouthwest", "closedHandCursor")
                }
                SystemCursor::WResize => {
                    load_hidden_system_cursor("resizeeastwest", "resizeLeftRightCursor")
                }
                SystemCursor::ContextMenu => {
                    StrongPtr::retain(msg_send![class!(NSCursor), contextualMenuCursor])
                }
                SystemCursor::Help => load_hidden_system_cursor("help", "arrowCursor"),
                SystemCursor::Cell => load_hidden_system_cursor("cell", "arrowCursor"),
                SystemCursor::VerticalText => {
                    StrongPtr::retain(msg_send![class!(NSCursor), IBeamCursorForVerticalLayout])
                }
                SystemCursor::Alias => {
                    StrongPtr::retain(msg_send![class!(NSCursor), dragLinkCursor])
                }
                SystemCursor::Copy => {
                    StrongPtr::retain(msg_send![class!(NSCursor), dragCopyCursor])
                }
                SystemCursor::Grab => {
                    StrongPtr::retain(msg_send![class!(NSCursor), openHandCursor])
                }
                SystemCursor::Grabbing => {
                    StrongPtr::retain(msg_send![class!(NSCursor), closedHandCursor])
                }
                SystemCursor::ColResize => {
                    StrongPtr::retain(msg_send![class!(NSCursor), resizeLeftRightCursor])
                }
                SystemCursor::RowResize => {
                    StrongPtr::retain(msg_send![class!(NSCursor), resizeUpDownCursor])
                }
                SystemCursor::ZoomIn => load_hidden_system_cursor("zoomin", "zoomInCursor"),
                SystemCursor::ZoomOut => load_hidden_system_cursor("zoomout", "zoomOutCursor"),
            }
        };

        if nscursor.is_nil() {
            return Err(Error::new("Couldn't create the system cursor"));
        }
        // We'll free it later, so retain it here
        Ok(Cursor::with_internal(CursorData::new(vec![(nscursor, 0)])))
    })
}

/// Translation of `Cocoa_CreateDefaultCursor()`.
fn create_default_cursor() -> Option<Cursor> {
    let id = crate::events::mouse::default_system_cursor();
    create_system_cursor(id).ok()
}

// GCMouse support for raw (unaccelerated) mouse input on macOS 11.0+

/// The `GCMouseDidConnectNotification`/`GCMouseDidDisconnectNotification`
/// observers (`cocoa_mouse_connect_observer`, `cocoa_mouse_disconnect_observer`).
static COCOA_MOUSE_OBSERVERS: Mutex<(StrongPtr, StrongPtr)> =
    Mutex::new((StrongPtr::nil(), StrongPtr::nil()));
/// Atomic for thread-safe access during high-frequency mouse input
/// (`cocoa_gcmouse_relative_mode`).
static COCOA_GCMOUSE_RELATIVE_MODE: AtomicI32 = AtomicI32::new(0);
/// Translation of `cocoa_has_gcmouse`.
static COCOA_HAS_GCMOUSE: AtomicBool = AtomicBool::new(false);

/// The GameController framework, loaded on first use (upstream links it
/// weakly).
fn game_controller_framework() -> Option<usize> {
    static HANDLE: OnceLock<Option<usize>> = OnceLock::new();
    *HANDLE.get_or_init(|| {
        load_framework(c"/System/Library/Frameworks/GameController.framework/GameController")
    })
}

/// Translation of `Cocoa_SetGCMouseRelativeMode()`.
fn set_gc_mouse_relative_mode(enabled: bool) -> bool {
    COCOA_GCMOUSE_RELATIVE_MODE.store(enabled as i32, Ordering::Relaxed);
    true
}

/// Translation of `Cocoa_OnGCMouseButtonChanged()`.
fn on_gc_mouse_button_changed(mouse_id: MouseID, button: u8, pressed: bool) {
    let timestamp = crate::timer::ticks();
    send_mouse_button(timestamp, mouse_focus(), mouse_id, button, pressed);
}

/// `button.pressedChangedHandler = ^(GCControllerButtonInput *button, float value, BOOL pressed) {...}`.
///
/// # Safety
///
/// `input` must be a `GCControllerButtonInput`.
unsafe fn set_pressed_changed_handler(input: Id, handler: Option<RcBlock>) {
    let block = handler
        .as_ref()
        .map_or(std::ptr::null_mut(), |b| b.as_ptr());
    // SAFETY: the caller's contract; the input copies the block.
    let _: () = unsafe { msg_send![input, setPressedChangedHandler: block] };
}

/// Translation of `Cocoa_OnGCMouseConnected()`.
fn on_gc_mouse_connected(mouse: Id) {
    let mouse_id = mouse as usize as MouseID;

    add_mouse(mouse_id, None);
    COCOA_HAS_GCMOUSE.store(true, Ordering::Relaxed);

    // Sync with SDL's current relative mode state (may have been set before
    // GCMouse connected)
    if with_mouse(|m| m.relative_mode) {
        COCOA_GCMOUSE_RELATIVE_MODE.store(1, Ordering::Relaxed);
    }

    let button_handler = |button: u8| {
        RcBlock::new3(move |_button: Id, _value: f32, pressed: Bool| {
            on_gc_mouse_button_changed(mouse_id, button, pressed.is_true());
        })
    };

    // SAFETY: a GCMouse and its inputs; the handlers are blocks of the
    // documented signatures, copied by the inputs.
    unsafe {
        let mouse_input: Id = msg_send![mouse, mouseInput];
        let left: Id = msg_send![mouse_input, leftButton];
        set_pressed_changed_handler(left, Some(button_handler(BUTTON_LEFT)));
        let middle: Id = msg_send![mouse_input, middleButton];
        set_pressed_changed_handler(middle, Some(button_handler(BUTTON_MIDDLE)));
        let right: Id = msg_send![mouse_input, rightButton];
        set_pressed_changed_handler(right, Some(button_handler(BUTTON_RIGHT)));

        let auxiliary: Id = msg_send![mouse_input, auxiliaryButtons];
        for (i, btn) in ns_array_items(auxiliary).into_iter().enumerate() {
            let current_button = BUTTON_X1 + i as u8;
            set_pressed_changed_handler(btn, Some(button_handler(current_button)));
        }

        let moved = RcBlock::new3(move |_mouse_input: Id, delta_x: f32, delta_y: f32| {
            if gc_mouse_relative_mode() {
                // Skip raw input if user wants system-scaled (accelerated) deltas
                if with_mouse(|m| m.enable_relative_system_scale) {
                    return;
                }
                let timestamp = crate::timer::ticks();
                send_mouse_motion(timestamp, mouse_focus(), mouse_id, true, delta_x, -delta_y);
            }
        });
        let _: () = msg_send![mouse_input, setMouseMovedHandler: moved.as_ptr()];

        // (USE_GCMOUSE_SCROLL isn't defined upstream: the scroll wheel stays
        // with the NSEvent path)

        // Use high-priority queue for low-latency input
        #[link(name = "System", kind = "dylib")]
        unsafe extern "C" {
            fn dispatch_set_target_queue(object: *mut c_void, queue: *mut c_void);
        }
        let queue = dispatch_queue_create(
            c"org.libsdl.input.mouse".as_ptr(),
            crate::core::apple::block::DISPATCH_QUEUE_SERIAL,
        );
        /// `DISPATCH_QUEUE_PRIORITY_HIGH`.
        const DISPATCH_QUEUE_PRIORITY_HIGH: isize = 2;
        dispatch_set_target_queue(
            queue,
            dispatch_get_global_queue(DISPATCH_QUEUE_PRIORITY_HIGH, 0),
        );
        let _: () = msg_send![mouse, setHandlerQueue: queue];
        crate::core::apple::block::dispatch_release(queue);
    }
}

/// Translation of `Cocoa_OnGCMouseDisconnected()`.
fn on_gc_mouse_disconnected(mouse: Id) {
    let mouse_id = mouse as usize as MouseID;

    // SAFETY: a GCMouse and its inputs.
    unsafe {
        let mouse_input: Id = msg_send![mouse, mouseInput];
        let _: () = msg_send![mouse_input, setMouseMovedHandler: std::ptr::null_mut::<c_void>()];
        let left: Id = msg_send![mouse_input, leftButton];
        set_pressed_changed_handler(left, None);
        let middle: Id = msg_send![mouse_input, middleButton];
        set_pressed_changed_handler(middle, None);
        let right: Id = msg_send![mouse_input, rightButton];
        set_pressed_changed_handler(right, None);
        let scroll: Id = msg_send![mouse_input, scroll];
        let _: () = msg_send![scroll, setValueChangedHandler: std::ptr::null_mut::<c_void>()];

        let auxiliary: Id = msg_send![mouse_input, auxiliaryButtons];
        for button in ns_array_items(auxiliary) {
            set_pressed_changed_handler(button, None);
        }
    }

    remove_mouse(mouse_id);

    // Check if any GCMouse devices remain
    if available!(macos(11, 0)) {
        // SAFETY: GCMouse's class method.
        let count = unsafe {
            let mice: Id = msg_send![class!(GCMouse), mice];
            crate::core::apple::foundation::ns_array_count(mice)
        };
        COCOA_HAS_GCMOUSE.store(count > 0, Ordering::Relaxed);
    }
}

/// Translation of `Cocoa_InitGCMouse()`.
pub(crate) fn init_gc_mouse() {
    if !hints::get_bool(hints::MAC_USE_GCMOUSE, true) {
        return;
    }

    autoreleasepool(|| {
        // These APIs are available starting in macOS Big Sur, but we don't enable
        // GCMouse until Sonoma due to broken motion and button events on MacBooks
        // running Monterey and Ventura.
        if available!(macos(14, 0)) {
            let Some(framework) = game_controller_framework() else {
                return;
            };
            // SAFETY: the notification names GameController exports, and
            // notification blocks of the documented signature.
            unsafe {
                let connect_name = framework_object(framework, c"GCMouseDidConnectNotification");
                let disconnect_name =
                    framework_object(framework, c"GCMouseDidDisconnectNotification");
                if connect_name.is_null() || disconnect_name.is_null() || class!(GCMouse).is_null()
                {
                    return;
                }
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

                *COCOA_MOUSE_OBSERVERS
                    .lock()
                    .unwrap_or_else(|e| e.into_inner()) = (
                    StrongPtr::retain(connect_observer),
                    StrongPtr::retain(disconnect_observer),
                );

                // Enumerate already-connected mice
                let mice: Id = msg_send![class!(GCMouse), mice];
                for mouse in ns_array_items(mice) {
                    on_gc_mouse_connected(mouse);
                }
            }
        }
    })
}

/// Translation of `Cocoa_GCMouseRelativeMode()`.
pub(crate) fn gc_mouse_relative_mode() -> bool {
    COCOA_GCMOUSE_RELATIVE_MODE.load(Ordering::Relaxed) != 0
}

/// Translation of `Cocoa_HasGCMouse()`.
pub(crate) fn has_gc_mouse() -> bool {
    COCOA_HAS_GCMOUSE.load(Ordering::Relaxed)
}

/// Translation of `Cocoa_QuitGCMouse()`.
pub(crate) fn quit_gc_mouse() {
    autoreleasepool(|| {
        if available!(macos(14, 0)) && game_controller_framework().is_some() {
            let Some(framework) = game_controller_framework() else {
                return;
            };
            // SAFETY: the observers registered by init_gc_mouse() and the
            // GCMouse class method.
            unsafe {
                let center: Id = msg_send![class!(NSNotificationCenter), defaultCenter];
                let (connect, disconnect) = std::mem::take(
                    &mut *COCOA_MOUSE_OBSERVERS
                        .lock()
                        .unwrap_or_else(|e| e.into_inner()),
                );

                if !connect.is_nil() {
                    let name = framework_object(framework, c"GCMouseDidConnectNotification");
                    let _: () =
                        msg_send![center, removeObserver: connect.get() name: name object: NIL];
                }

                if !disconnect.is_nil() {
                    let name = framework_object(framework, c"GCMouseDidDisconnectNotification");
                    let _: () =
                        msg_send![center, removeObserver: disconnect.get() name: name object: NIL];
                }

                if !class!(GCMouse).is_null() {
                    let mice: Id = msg_send![class!(GCMouse), mice];
                    for mouse in ns_array_items(mice) {
                        on_gc_mouse_disconnected(mouse);
                    }
                }

                COCOA_HAS_GCMOUSE.store(false, Ordering::Relaxed);
                COCOA_GCMOUSE_RELATIVE_MODE.store(0, Ordering::Relaxed);
            }
        }
    })
}

/// Translation of `Cocoa_ShowCursor()`.
pub(crate) fn show_cursor(cursor: Option<&Cursor>) -> Result<()> {
    autoreleasepool(|| {
        if let Some(cdata) = cursor.and_then(cursor_data) {
            cdata.reset();
        }

        super::window::invalidate_cursor_rects_async();
        Ok(())
    })
}

/// Translation of `SDL_FindWindowAtPoint()`.
fn find_window_at_point(x: f32, y: f32) -> Option<WindowID> {
    crate::video::core::window_ids().into_iter().find(|&id| {
        crate::video::core::with_window(id, |w| {
            x >= w.core.x as f32
                && x < (w.core.x + w.core.w) as f32
                && y >= w.core.y as f32
                && y < (w.core.y + w.core.h) as f32
        })
        .unwrap_or(false)
    })
}

/// Translation of `Cocoa_WarpMouseGlobal()`.
pub(crate) fn warp_mouse_global(x: f32, y: f32) -> Result<()> {
    if let Some(focus) = mouse_focus() {
        if let Some(data) = window_data(focus) {
            if data.is_moving_or_focus_click_pending() {
                // DLog("Postponing warp, window being moved or focused.");
                data.set_pending_move(x, y);
                return Ok(());
            }
        }
    }
    let point = NSPoint::new(x as f64, y as f64);

    handle_mouse_warp(point.x, point.y);

    // SAFETY: CoreGraphics' cursor functions.
    unsafe { CGWarpMouseCursorPosition(point) };

    let relative_mode = with_mouse(|m| m.relative_mode);
    /* CGWarpMouse causes a short delay by default, which is preventable by
     * Calling this directly after. CGSetLocalEventsSuppressionInterval can also
     * prevent it, but it's deprecated as macOS 10.6.
     */
    if !relative_mode {
        // SAFETY: as above.
        unsafe { CGAssociateMouseAndMouseCursorPosition(1) };
    }

    /* CGWarpMouseCursorPosition doesn't generate a window event, unlike our
     * other implementations' APIs. Send what's appropriate.
     */
    if !relative_mode {
        let win = find_window_at_point(x, y);
        set_mouse_focus(win);
        if let Some(win) = win {
            crate::sdl_assert!(Some(win) == mouse_focus());
            let (wx, wy) =
                crate::video::core::with_window(win, |w| (w.core.x, w.core.y)).unwrap_or_default();
            send_mouse_motion(
                Duration::ZERO,
                Some(win),
                GLOBAL_MOUSE_ID,
                false,
                x - wx as f32,
                y - wy as f32,
            );
        }
    }

    Ok(())
}

/// Translation of `Cocoa_WarpMouse()`.
pub(crate) fn warp_mouse(window: WindowID, x: f32, y: f32) -> Result<()> {
    let (wx, wy) = crate::video::core::with_window(window, |w| (w.core.x, w.core.y))?;
    warp_mouse_global(wx as f32 + x, wy as f32 + y)
}

/// Translation of `Cocoa_SetRelativeMouseMode()`.
pub(crate) fn set_relative_mouse_mode(enabled: bool) -> Result<()> {
    // Update GCMouse relative mode state if available
    if has_gc_mouse() {
        set_gc_mouse_relative_mode(enabled);
    }

    let result = if enabled {
        if let Some(window) = crate::events::keyboard::keyboard_focus() {
            /* We will re-apply the relative mode when the window finishes
             * being moved, if it is being moved right now.
             */
            if let Some(data) = window_data(window) {
                if data.is_moving_or_focus_click_pending() {
                    return Ok(());
                }
            }

            // Make sure the mouse isn't at the corner of the window, as this
            // can confuse things if macOS thinks a window resize is happening
            // on the first click.
            let (x, y, w, h) = crate::video::core::with_window(window, |w| {
                (w.core.x, w.core.y, w.core.w, w.core.h)
            })
            .unwrap_or_default();
            let point = NSPoint::new((x + (w / 2)) as f32 as f64, (y + (h / 2)) as f32 as f64);
            handle_mouse_warp(point.x, point.y);
            // SAFETY: CoreGraphics' cursor function.
            unsafe { CGWarpMouseCursorPosition(point) };
        }
        // DLog("Turning on.");
        // SAFETY: as above.
        unsafe { CGAssociateMouseAndMouseCursorPosition(0) }
    } else {
        // DLog("Turning off.");
        // SAFETY: as above.
        unsafe { CGAssociateMouseAndMouseCursorPosition(1) }
    };
    if result != kCGErrorSuccess {
        return Err(Error::new(
            "CGAssociateMouseAndMouseCursorPosition() failed",
        ));
    }

    /* The hide/unhide calls are redundant most of the time, but they fix
     * https://bugzilla.libsdl.org/show_bug.cgi?id=2550
     */
    // SAFETY: NSCursor's class methods.
    unsafe {
        if enabled {
            let _: () = msg_send![class!(NSCursor), hide];
        } else {
            let _: () = msg_send![class!(NSCursor), unhide];
        }
    }
    Ok(())
}

/// Translation of `Cocoa_CaptureMouse()`.
pub(crate) fn capture_mouse(_window: Option<WindowID>) -> Result<()> {
    /* our Cocoa event code already tracks the mouse outside the window,
    so all we have to do here is say "okay" and do what we always do. */
    Ok(())
}

/// Translation of `Cocoa_GetGlobalMouseState()`.
pub(crate) fn get_global_mouse_state() -> (f32, f32, MouseButtonFlags) {
    // SAFETY: NSEvent's class methods.
    let (cocoa_buttons, cocoa_location): (NSUInteger, NSPoint) = unsafe {
        (
            msg_send![class!(NSEvent), pressedMouseButtons],
            msg_send![class!(NSEvent), mouseLocation],
        )
    };
    let main_display_height = video_data().map_or(0.0, |d| d.main_display_height());
    let mut result = MouseButtonFlags::default();

    let x = cocoa_location.x as f32;
    let y = (main_display_height - cocoa_location.y) as f32;

    if cocoa_buttons & (1 << 0) != 0 {
        result |= MouseButtonFlags::LMASK;
    }
    if cocoa_buttons & (1 << 1) != 0 {
        result |= MouseButtonFlags::RMASK;
    }
    if cocoa_buttons & (1 << 2) != 0 {
        result |= MouseButtonFlags::MMASK;
    }
    if cocoa_buttons & (1 << 3) != 0 {
        result |= MouseButtonFlags::X1MASK;
    }
    if cocoa_buttons & (1 << 4) != 0 {
        result |= MouseButtonFlags::X2MASK;
    }

    (x, y, result)
}

/// Translation of `SDL_MouseData`.
#[derive(Clone, Copy, Default)]
struct MouseData {
    /// Whether we've seen a cursor warp since the last move event.
    seen_warp: bool,
    /// What location our last cursor warp was to.
    last_warp_x: f64,
    last_warp_y: f64,
    /// What location we last saw the cursor move to.
    last_move_x: f64,
    last_move_y: f64,
}

/// `mouse->internal` (`None` before `Cocoa_InitMouse()` and after quit).
static MOUSE_DATA: Mutex<Option<MouseData>> = Mutex::new(None);

/// Translation of `Cocoa_InitMouse()`.
pub(crate) fn init_mouse(_data: &Arc<VideoData>) -> Result<()> {
    // (the SDL_Mouse entry points are the driver's VideoDriver methods)

    set_default_cursor(create_default_cursor());

    // SAFETY: NSEvent's class method.
    let location: NSPoint = unsafe { msg_send![class!(NSEvent), mouseLocation] };
    *MOUSE_DATA.lock().unwrap_or_else(|e| e.into_inner()) = Some(MouseData {
        last_move_x: location.x,
        last_move_y: location.y,
        ..MouseData::default()
    });
    Ok(())
}

/// Translation of `Cocoa_HandleTitleButtonEvent()`.
fn handle_title_button_event(event: Id) {
    // SAFETY: an NSEvent.
    let (nswindow, ty, button_number): (Id, NSUInteger, NSInteger) = unsafe {
        (
            msg_send![event, window],
            msg_send![event, type],
            msg_send![event, buttonNumber],
        )
    };

    /* You might land in this function before SDL_Init if showing a message box.
    Don't dereference a NULL pointer if that happens. */
    if !crate::video::core::initialized() {
        return;
    }

    if let Some(window) = find_sdl_window(nswindow) {
        if let Some(data) = window_data(window) {
            match ty {
                NSEventTypeLeftMouseDown
                | NSEventTypeRightMouseDown
                | NSEventTypeOtherMouseDown => {
                    data.set_focus_click_pending(button_number);
                }
                NSEventTypeLeftMouseUp | NSEventTypeRightMouseUp | NSEventTypeOtherMouseUp => {
                    data.clear_focus_click_pending(button_number);
                }
                _ => {}
            }
        }
    }
}

/// Translation of `Cocoa_MouseFocus`.
static COCOA_MOUSE_FOCUS: AtomicUsize = AtomicUsize::new(0);

/// Translation of `Cocoa_GetMouseFocus()`: the `NSWindow` the mouse was
/// last seen in (nil once it exited).
pub(crate) fn get_mouse_focus() -> Id {
    COCOA_MOUSE_FOCUS.load(Ordering::Relaxed) as Id
}

/// Translation of `Cocoa_ReconcileButtonState()`.
fn reconcile_button_state(event: Id) {
    // Send mouse up events for any buttons that are no longer pressed
    let (_, _, buttons) = crate::events::mouse::mouse_state();
    let mut buttons = buttons.0;
    // SAFETY: NSEvent's class method.
    let pressed: NSUInteger = unsafe { msg_send![class!(NSEvent), pressedMouseButtons] };
    if buttons != 0 && pressed == 0 {
        // SAFETY: an NSEvent.
        let timestamp: f64 = unsafe { msg_send![event, timestamp] };
        let mut button = BUTTON_LEFT;
        while buttons != 0 {
            if buttons & 0x01 != 0 {
                send_mouse_button(
                    get_event_timestamp(timestamp),
                    mouse_focus(),
                    GLOBAL_MOUSE_ID,
                    button,
                    false,
                );
            }
            button += 1;
            buttons >>= 1;
        }
    }
}

/// `NSMouseInRect([event locationInWindow], [[[event window] contentView] frame], NO)`
/// for an event with a window.
fn in_content_view(event: Id, window: Id) -> bool {
    // SAFETY: an NSEvent and its window's content view.
    unsafe {
        let content_view: Id = msg_send![window, contentView];
        let window_rect: NSRect = msg_send![content_view, frame];
        let location: NSPoint = msg_send![event, locationInWindow];
        window_rect.contains(location)
    }
}

/// Translation of `Cocoa_HandleMouseEvent()`.
pub(crate) fn handle_mouse_event(event: Id) {
    let mouse_id = DEFAULT_MOUSE_ID;

    // All events except NSEventTypeMouseExited can only happen if the window
    // has mouse focus, so we'll always set the focus even if we happen to miss
    // NSEventTypeMouseEntered, which apparently happens if the window is
    // created under the mouse on macOS 12.7.  But, only set the focus if
    // the event actually has a non-NULL window, otherwise what would happen
    // is that after an NSEventTypeMouseEntered there would sometimes be
    // NSEventTypeMouseMoved without a window causing us to suppress subsequent
    // mouse move events.
    // SAFETY: an NSEvent.
    let (event_type, event_window): (NSUInteger, Id) =
        unsafe { (msg_send![event, type], msg_send![event, window]) };
    if event_type == NSEventTypeMouseExited {
        COCOA_MOUSE_FOCUS.store(0, Ordering::Relaxed);
    } else if !event_window.is_null() {
        COCOA_MOUSE_FOCUS.store(event_window as usize, Ordering::Relaxed);
        if crate::video::core::initialized() {
            reconcile_button_state(event);
        }
    }

    match event_type {
        NSEventTypeMouseEntered | NSEventTypeMouseExited => {
            // Focus is handled above
            return;
        }

        NSEventTypeMouseMoved
        | NSEventTypeLeftMouseDragged
        | NSEventTypeRightMouseDragged
        | NSEventTypeOtherMouseDragged => {}

        NSEventTypeLeftMouseDown
        | NSEventTypeLeftMouseUp
        | NSEventTypeRightMouseDown
        | NSEventTypeRightMouseUp
        | NSEventTypeOtherMouseDown
        | NSEventTypeOtherMouseUp => {
            if !event_window.is_null() && !in_content_view(event, event_window) {
                handle_title_button_event(event);
                return;
            }
            return;
        }

        _ => {
            // Ignore any other events.
            return;
        }
    }

    let Some(videodata) = video_data() else {
        return;
    };
    let (seen_warp, last_move_x, last_move_y, last_warp_x, last_warp_y) = {
        let mut guard = MOUSE_DATA.lock().unwrap_or_else(|e| e.into_inner());
        let Some(data) = guard.as_mut() else {
            return; // can happen when returning from fullscreen Space on shutdown
        };

        let seen_warp = std::mem::take(&mut data.seen_warp);

        // SAFETY: NSEvent's class method.
        let location: NSPoint = unsafe { msg_send![class!(NSEvent), mouseLocation] };
        let last = (data.last_move_x, data.last_move_y);
        data.last_move_x = location.x;
        data.last_move_y = location.y;
        // DLog("Last seen mouse: (%g, %g)", location.x, location.y);
        (
            seen_warp,
            last.0,
            last.1,
            data.last_warp_x,
            data.last_warp_y,
        )
    };

    let (relative_mode, system_scale, focus) =
        with_mouse(|m| (m.relative_mode, m.enable_relative_system_scale, m.focus));

    // Non-relative movement is handled in -[SDL3Cocoa_WindowListener mouseMoved:]
    if !relative_mode {
        return;
    }

    // When GCMouse is active in relative mode, it handles motion events
    // directly with raw (unaccelerated) deltas. Skip NSEvent-based motion
    // unless the user wants system-scaled (accelerated) input.
    if has_gc_mouse() && gc_mouse_relative_mode() && !system_scale {
        // GCMouse is providing raw input, skip NSEvent deltas
        return;
        // SYSTEM_SCALE is enabled: use NSEvent accelerated deltas instead
    }

    // Ignore events that aren't inside the client area (i.e. title bar.)
    if !event_window.is_null() && !in_content_view(event, event_window) {
        return;
    }

    // SAFETY: an NSEvent.
    let (mut delta_x, mut delta_y, timestamp) = unsafe {
        let dx: f64 = msg_send![event, deltaX];
        let dy: f64 = msg_send![event, deltaY];
        let t: f64 = msg_send![event, timestamp];
        (dx as f32, dy as f32, t)
    };

    if seen_warp {
        delta_x += (last_move_x - last_warp_x) as f32;
        delta_y += ((videodata.main_display_height() - last_move_y) - last_warp_y) as f32;

        // DLog("Motion was (%g, %g), offset to (%g, %g)", [event deltaX],
        //      [event deltaY], deltaX, deltaY);
    }

    send_mouse_motion(
        get_event_timestamp(timestamp),
        focus,
        mouse_id,
        true,
        delta_x,
        delta_y,
    );
}

/// Translation of `Cocoa_HandleMouseWheel()`.
pub(crate) fn handle_mouse_wheel(window: WindowID, event: Id) {
    // (USE_GCMOUSE_SCROLL isn't defined upstream)

    let mouse_id = DEFAULT_MOUSE_ID;
    let mut direction = MouseWheelDirection::Normal;

    // SAFETY: an NSEvent.
    unsafe {
        let inverted: Bool = msg_send![event, isDirectionInvertedFromDevice];
        if inverted.is_true() {
            direction = MouseWheelDirection::Flipped;
        }

        let precise: Bool = msg_send![event, hasPreciseScrollingDeltas];
        let (x, y): (f64, f64) = if precise.is_true() {
            let sx: f64 = msg_send![event, scrollingDeltaX];
            let sy: f64 = msg_send![event, scrollingDeltaY];
            (-sx * 0.1f32 as f64, sy * 0.1f32 as f64)
        } else {
            let dx: f64 = msg_send![event, deltaX];
            let dy: f64 = msg_send![event, deltaY];
            let mut x = -dx;
            let mut y = dy;
            if x > 0.0 {
                x = x.ceil();
            } else if x < 0.0 {
                x = x.floor();
            }
            if y > 0.0 {
                y = y.ceil();
            } else if y < 0.0 {
                y = y.floor();
            }
            (x, y)
        };

        let timestamp: f64 = msg_send![event, timestamp];
        send_mouse_wheel(
            get_event_timestamp(timestamp),
            Some(window),
            mouse_id,
            x as f32,
            y as f32,
            direction,
        );
    }
}

/// Translation of `Cocoa_HandleMouseWarp()`.
pub(crate) fn handle_mouse_warp(x: f64, y: f64) {
    /* This makes Cocoa_HandleMouseEvent ignore the delta caused by the warp,
     * since it gets included in the next movement event.
     */
    if let Some(data) = MOUSE_DATA
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_mut()
    {
        data.last_warp_x = x;
        data.last_warp_y = y;
        data.seen_warp = true;
    }

    // DLog("(%g, %g)", x, y);
}

/// Translation of `Cocoa_QuitMouse()`.
pub(crate) fn quit_mouse() {
    *MOUSE_DATA.lock().unwrap_or_else(|e| e.into_inner()) = None;
}
