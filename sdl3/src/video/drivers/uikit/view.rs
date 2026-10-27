// Rust translation of src/video/uikit/SDL_uikitview.m and SDL_uikitview.h
// from Simple DirectMedia Layer (but the hardware keys).
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! `SDL_uikitview`, the `UIView` a window's view controller shows: the
//! window it belongs to ([`set_sdl_window`], which makes it the view
//! controller's view and keeps it in the window's views, and
//! [`get_sdl_window`]), its size following the window's (it resizes with
//! its superview), and its safe area, which becomes the window's
//! ([`set_window_safe_area_insets`]).
//!
//! It turns UIKit's touches into SDL's: fingers on the screen are the
//! direct touch device's (ID 1, added when the view is made), touches
//! elsewhere (the Siri Remote's surface) the indirect one's (ID 2, added
//! with its first touch), each finger's ID its `UITouch`'s address, its
//! position normalized to the view's size and its pressure the touch's
//! force (the touch core emulates the mouse with the first finger,
//! `SDL_HINT_TOUCH_MOUSE_EVENTS`); the Apple Pencil's touches and hovering
//! are the pen's ([`super::pen`]); the pointer of a trackpad or mouse
//! (iOS 13.4) moves the mouse and, with no game controller's mouse
//! (`GCMouse`, [`super::events`]), presses its buttons, and its
//! `UIPointerInteraction` hides the system's pointer while SDL's cursor is
//! hidden; and the pinch gesture is SDL's pinch events. The pinch,
//! hovering and the pointer aren't on tvOS. The methods upstream calls
//! only from these handlers (`touchTypeForTouch:`, `touchIdForType:`,
//! `touchLocation:shouldNormalize:`, `indirectPointerPressed:fromEvent:`,
//! ...) are Rust functions here.
//!
//! Not translated yet: the hardware keys (`UIPress`), which come with the
//! keyboard, and tvOS's swipe gestures, which come with tvOS; until then
//! the view leaves the presses to `UIView` (the next responder).
//!
//! [`set_window_safe_area_insets`]: crate::video::core::set_window_safe_area_insets

#[cfg(not(target_os = "tvos"))]
use crate::core::apple::foundation::available;
use crate::core::apple::foundation::{CGFloat, CGPoint, CGRect, NSTimeInterval};
use crate::core::apple::objc::{
    class, imp, ivar_ptr, msg_send, msg_send_super, sel, Class, ClassBuilder, DeclaredClass, Id,
    Sel, StrongPtr, NIL, YES,
};
#[cfg(not(target_os = "tvos"))]
use crate::core::apple::objc::{NSInteger, NO};
use crate::events::touch::{
    add_touch, send_touch, send_touch_motion, FingerID, TouchDeviceType, TouchID,
};
use crate::events::{EventType, WindowID};

use super::events::get_event_timestamp;
use super::sys::*;
use super::window::window_data;

static SDL_UIKITVIEW: DeclaredClass = DeclaredClass::new();

/// The `SDL_uikitview` class (`@interface SDL_uikitview : UIView`, a
/// `UIPointerInteractionDelegate` but on tvOS), with its instance
/// variables: `sdlwindow` (the window's ID, 0 for none),
/// `directTouchId`, `indirectTouchId` and `pinch_scale`. (Upstream also
/// keeps the pointer interaction, which the view's interactions keep
/// here.)
pub(crate) fn uikitview_class() -> Class {
    SDL_UIKITVIEW.get(|| {
        let mut b = ClassBuilder::new(c"SDL_uikitview", class!(UIView)).expect("SDL_uikitview");
        b.add_ivar::<u32>(c"sdlwindow", c"I");
        b.add_ivar::<TouchID>(c"directTouchId", c"Q");
        b.add_ivar::<TouchID>(c"indirectTouchId", c"Q");
        b.add_ivar::<f32>(c"pinch_scale", c"f");
        #[cfg(not(target_os = "tvos"))]
        b.add_protocol(c"UIPointerInteractionDelegate");
        // SAFETY: the implementations have the signatures of the encodings.
        unsafe {
            b.add_method(
                sel!(initWithFrame:),
                imp!(view_init_with_frame as extern "C" fn(Id, Sel, CGRect) -> Id),
                c"@@:{CGRect={CGPoint=dd}{CGSize=dd}}",
            );
            b.add_method(
                sel!(safeAreaInsetsDidChange),
                imp!(view_safe_area_insets_did_change as extern "C" fn(Id, Sel)),
                c"v@:",
            );
            b.add_method(
                sel!(touchesBegan:withEvent:),
                imp!(view_touches_began as extern "C" fn(Id, Sel, Id, Id)),
                c"v@:@@",
            );
            b.add_method(
                sel!(touchesEnded:withEvent:),
                imp!(view_touches_ended as extern "C" fn(Id, Sel, Id, Id)),
                c"v@:@@",
            );
            b.add_method(
                sel!(touchesCancelled:withEvent:),
                imp!(view_touches_cancelled as extern "C" fn(Id, Sel, Id, Id)),
                c"v@:@@",
            );
            b.add_method(
                sel!(touchesMoved:withEvent:),
                imp!(view_touches_moved as extern "C" fn(Id, Sel, Id, Id)),
                c"v@:@@",
            );
            #[cfg(not(target_os = "tvos"))]
            {
                b.add_method(
                    sel!(pointerInteraction:regionForRequest:defaultRegion:),
                    imp!(view_pointer_region as extern "C" fn(Id, Sel, Id, Id, Id) -> Id),
                    c"@@:@@@",
                );
                b.add_method(
                    sel!(pointerInteraction:styleForRegion:),
                    imp!(view_pointer_style as extern "C" fn(Id, Sel, Id, Id) -> Id),
                    c"@@:@@",
                );
                b.add_method(
                    sel!(indirectPointerHovering:),
                    imp!(view_indirect_pointer_hovering as extern "C" fn(Id, Sel, Id)),
                    c"v@:@",
                );
                b.add_method(
                    sel!(pencilHovering:),
                    imp!(view_pencil_hovering as extern "C" fn(Id, Sel, Id)),
                    c"v@:@",
                );
                b.add_method(
                    sel!(sdlPinchGesture:),
                    imp!(view_pinch_gesture as extern "C" fn(Id, Sel, Id)),
                    c"v@:@",
                );
            }
        }
        b.register()
    })
}

/// `sdlwindow`'s variable.
fn sdlwindow_ivar(view: Id) -> *mut u32 {
    // SAFETY: an SDL_uikitview (or a subclass's instance), which has the
    // variable.
    unsafe { ivar_ptr::<u32>(view, uikitview_class(), c"sdlwindow") }
}

/// A `TouchID` variable of the view (`directTouchId`, `indirectTouchId`).
fn touch_id_ivar(view: Id, name: &std::ffi::CStr) -> *mut TouchID {
    // SAFETY: as above.
    unsafe { ivar_ptr::<TouchID>(view, uikitview_class(), name) }
}

/// `pinch_scale`'s variable.
#[cfg(not(target_os = "tvos"))]
fn pinch_scale_ivar(view: Id) -> *mut f32 {
    // SAFETY: as above.
    unsafe { ivar_ptr::<f32>(view, uikitview_class(), c"pinch_scale") }
}

/// A new view (owned). Translation of `[[SDL_uikitview alloc]
/// initWithFrame:frame]`.
pub(crate) fn new_view(frame: CGRect) -> StrongPtr {
    // SAFETY: the class's alloc and initializer (owned).
    unsafe {
        let view: Id = msg_send![uikitview_class(), alloc];
        StrongPtr::from_owned(msg_send![view, initWithFrame: frame])
    }
}

/// `@[@(touchType)]`, for a gesture recognizer's `allowedTouchTypes`
/// (autoreleased).
#[cfg(not(target_os = "tvos"))]
fn touch_types(touch_type: UITouchType) -> Id {
    // SAFETY: NSNumber's and NSArray's class methods (autoreleased).
    unsafe {
        let number: Id = msg_send![class!(NSNumber), numberWithInteger: touch_type as NSInteger];
        msg_send![class!(NSArray), arrayWithObject: number]
    }
}

/// `[[UIHoverGestureRecognizer alloc] initWithTarget:view action:action]`
/// for touches of one type only, added to the view.
#[cfg(not(target_os = "tvos"))]
fn add_hover_recognizer(view: Id, action: Sel, touch_type: UITouchType) {
    // SAFETY: UIHoverGestureRecognizer's alloc and initializer (owned,
    // the view keeps it), its property and UIView's method.
    unsafe {
        let recognizer: Id = msg_send![class!(UIHoverGestureRecognizer), alloc];
        let recognizer =
            StrongPtr::from_owned(msg_send![recognizer, initWithTarget: view action: action]);
        let () = msg_send![recognizer.get(), setAllowedTouchTypes: touch_types(touch_type)];
        let () = msg_send![view, addGestureRecognizer: recognizer.get()];
    }
}

/// Translation of `-[SDL_uikitview initWithFrame:]`.
extern "C" fn view_init_with_frame(this: Id, _sel: Sel, frame: CGRect) -> Id {
    // SAFETY: UIView's initializer and properties; UIPinchGestureRecognizer's
    // and UIPointerInteraction's alloc and initializers (owned: the view
    // keeps them).
    unsafe {
        let this: Id = msg_send_super![this, uikitview_class(), initWithFrame: frame];
        if !this.is_null() {
            // (tvOS's swipe gestures for the Apple TV Remote's touch surface
            // come with tvOS)

            #[cfg(not(target_os = "tvos"))]
            {
                /* Pinch gestures */
                let pinch_gesture: Id = msg_send![class!(UIPinchGestureRecognizer), alloc];
                let pinch_gesture = StrongPtr::from_owned(msg_send![pinch_gesture,
                    initWithTarget: this
                    action: sel!(sdlPinchGesture:)]);
                let () = msg_send![pinch_gesture.get(), setCancelsTouchesInView: NO];
                let () = msg_send![pinch_gesture.get(), setDelaysTouchesBegan: NO];
                let () = msg_send![pinch_gesture.get(), setDelaysTouchesEnded: NO];
                let () = msg_send![this, addGestureRecognizer: pinch_gesture.get()];
            }

            let () = msg_send![this, setAutoresizingMask: UIViewAutoresizingFlexibleWidth | UIViewAutoresizingFlexibleHeight];
            let () = msg_send![this, setAutoresizesSubviews: YES];
            *sdlwindow_ivar(this) = 0;

            *touch_id_ivar(this, c"directTouchId") = 1;
            *touch_id_ivar(this, c"indirectTouchId") = 2;

            #[cfg(not(target_os = "tvos"))]
            {
                let () = msg_send![this, setMultipleTouchEnabled: YES];
                add_touch(
                    *touch_id_ivar(this, c"directTouchId"),
                    TouchDeviceType::Direct,
                    "",
                );

                if available!(ios(13, 0)) {
                    add_hover_recognizer(this, sel!(pencilHovering:), UITouchTypePencil);
                }

                if available!(ios(13, 4)) {
                    let interaction: Id = msg_send![class!(UIPointerInteraction), alloc];
                    let interaction =
                        StrongPtr::from_owned(msg_send![interaction, initWithDelegate: this]);
                    let () = msg_send![this, addInteraction: interaction.get()];

                    add_hover_recognizer(
                        this,
                        sel!(indirectPointerHovering:),
                        UITouchTypeIndirectPointer,
                    );
                }
            }
        }
        this
    }
}

/// Move a view to a window (or to none): it becomes the window's view
/// controller's view (the one before it comes back when it leaves).
/// Translation of `-[SDL_uikitview setSDLWindow:]`.
pub(crate) fn set_sdl_window(view: Id, window: Option<WindowID>) {
    let current = get_sdl_window(view);
    if window == current {
        return;
    }
    // (kept while it moves: the old window's views may hold the last
    // reference)
    // SAFETY: a valid view, retained.
    let _view = unsafe { StrongPtr::retain(view) };

    // Remove ourself from the old window.
    if let Some(old) = current {
        if let Some(data) = window_data(old) {
            let next = {
                let mut views = data.views();
                views.retain(|v| v.get() != view);
                // Restore the next-oldest view in the old window.
                views.last().map_or(NIL, |v| v.get())
            };

            // SAFETY: UIView's, UIViewController's and UIWindow's methods
            // and properties.
            unsafe {
                let () = msg_send![view, removeFromSuperview];

                let () = msg_send![data.viewcontroller(), setView: next];

                let () = msg_send![data.uiwindow(), setRootViewController: NIL];
                let () = msg_send![data.uiwindow(), setRootViewController: data.viewcontroller()];

                let () = msg_send![data.uiwindow(), layoutIfNeeded];
            }
        }
    }

    // SAFETY: this view's variable.
    unsafe { *sdlwindow_ivar(view) = window.unwrap_or(0) };

    // Add ourself to the new window.
    if let Some(new) = window {
        if let Some(data) = window_data(new) {
            // Make sure the SDL window has a strong reference to this view.
            // SAFETY: a valid view, retained.
            data.views().push(unsafe { StrongPtr::retain(view) });

            // SAFETY: as above.
            unsafe {
                // Replace the view controller's old view with this one.
                let old_view: Id = msg_send![data.viewcontroller(), view];
                let () = msg_send![old_view, removeFromSuperview];
                let () = msg_send![data.viewcontroller(), setView: view];

                /* The root view controller handles rotation and the status bar.
                 * Assigning it also adds the controller's view to the window. We
                 * explicitly re-set it to make sure the view is properly attached to
                 * the window. Just adding the sub-view if the root view controller is
                 * already correct causes orientation issues on iOS 7 and below. */
                let () = msg_send![data.uiwindow(), setRootViewController: NIL];
                let () = msg_send![data.uiwindow(), setRootViewController: data.viewcontroller()];

                /* The view's bounds may not be correct until the next event cycle. That
                 * might happen after the current dimensions are queried, so we force a
                 * layout now to immediately update the bounds. */
                let () = msg_send![data.uiwindow(), layoutIfNeeded];
            }
        }
    }
}

/// The window of a view (none if it has left it). Translation of
/// `-[SDL_uikitview getSDLWindow]`.
pub(crate) fn get_sdl_window(view: Id) -> Option<WindowID> {
    // SAFETY: this view's variable.
    let id = unsafe { *sdlwindow_ivar(view) };
    (id != 0).then_some(id)
}

// ---------------------------------------------------------------------------
// The pointer and the Pencil (not on tvOS)

/// Translation of
/// `-[SDL_uikitview pointerInteraction:regionForRequest:defaultRegion:]`.
#[cfg(not(target_os = "tvos"))]
extern "C" fn view_pointer_region(
    this: Id,
    _sel: Sel,
    _interaction: Id,
    _request: Id,
    _default_region: Id,
) -> Id {
    // SAFETY: UIView's bounds and UIPointerRegion's class method
    // (autoreleased, as UIKit takes it).
    unsafe {
        let bounds: CGRect = msg_send![this, bounds];
        msg_send![class!(UIPointerRegion), regionWithRect: bounds identifier: NIL]
    }
}

/// Translation of `-[SDL_uikitview pointerInteraction:styleForRegion:]`.
#[cfg(not(target_os = "tvos"))]
extern "C" fn view_pointer_style(_this: Id, _sel: Sel, _interaction: Id, _region: Id) -> Id {
    if crate::events::mouse::cursor_visible() {
        NIL
    } else {
        // SAFETY: UIPointerStyle's class method (autoreleased).
        unsafe { msg_send![class!(UIPointerStyle), hiddenPointerStyle] }
    }
}

/// Translation of `-[SDL_uikitview indirectPointerHovering:]`.
#[cfg(not(target_os = "tvos"))]
extern "C" fn view_indirect_pointer_hovering(this: Id, _sel: Sel, recognizer: Id) {
    // SAFETY: UIGestureRecognizer's state and location.
    let state: UIGestureRecognizerState = unsafe { msg_send![recognizer, state] };
    match state {
        UIGestureRecognizerStateBegan | UIGestureRecognizerStateChanged => {
            // SAFETY: as above.
            let point: CGPoint = unsafe { msg_send![recognizer, locationInView: this] };
            crate::events::mouse::send_mouse_motion(
                std::time::Duration::ZERO,
                get_sdl_window(this),
                crate::events::mouse::GLOBAL_MOUSE_ID,
                false,
                point.x as f32,
                point.y as f32,
            );
        }
        _ => {}
    }
}

/// Translation of `-[SDL_uikitview indirectPointerMoving:]`.
#[cfg(not(target_os = "tvos"))]
fn indirect_pointer_moving(this: Id, touch: Id) {
    let location_in_view = touch_location(this, touch, false);
    crate::events::mouse::send_mouse_motion(
        std::time::Duration::ZERO,
        get_sdl_window(this),
        crate::events::mouse::GLOBAL_MOUSE_ID,
        false,
        location_in_view.x as f32,
        location_in_view.y as f32,
    );
}

/// The maximum number of mouse buttons we support. Translation of
/// `MAX_MOUSE_BUTTONS`.
#[cfg(not(target_os = "tvos"))]
const MAX_MOUSE_BUTTONS: u8 = 5;

/// A touch's timestamp (`[touch timestamp]`) on SDL's clock.
#[cfg(not(target_os = "tvos"))]
fn touch_timestamp(touch: Id) -> std::time::Duration {
    // SAFETY: a UITouch property.
    let timestamp: NSTimeInterval = unsafe { msg_send![touch, timestamp] };
    get_event_timestamp(timestamp)
}

/// Translation of `-[SDL_uikitview indirectPointerPressed:fromEvent:]`.
#[cfg(not(target_os = "tvos"))]
fn indirect_pointer_pressed(this: Id, touch: Id, event: Id) {
    use crate::events::mouse::{BUTTON_LEFT, BUTTON_MIDDLE, BUTTON_RIGHT};

    if !crate::events::mouse::has_mouse() {
        // SAFETY: a UIEvent property (iOS 13.4).
        let button_mask: UIEventButtonMask = unsafe { msg_send![event, buttonMask] };
        for i in 1..=MAX_MOUSE_BUTTONS {
            if button_mask & (1 << (i - 1)) != 0 {
                let button = match i {
                    1 => BUTTON_LEFT,
                    2 => BUTTON_RIGHT,
                    3 => BUTTON_MIDDLE,
                    _ => i,
                };
                crate::events::mouse::send_mouse_button(
                    touch_timestamp(touch),
                    get_sdl_window(this),
                    crate::events::mouse::GLOBAL_MOUSE_ID,
                    button,
                    true,
                );
            }
        }
    }
}

/// Translation of `-[SDL_uikitview indirectPointerReleased:fromEvent:]`.
#[cfg(not(target_os = "tvos"))]
fn indirect_pointer_released(this: Id, touch: Id, _event: Id) {
    if !crate::events::mouse::has_mouse() {
        let buttons = crate::events::mouse::mouse_state().2;

        for i in 1..=MAX_MOUSE_BUTTONS {
            if buttons.0 & (1 << (i - 1)) != 0 {
                crate::events::mouse::send_mouse_button(
                    touch_timestamp(touch),
                    get_sdl_window(this),
                    crate::events::mouse::GLOBAL_MOUSE_ID,
                    i,
                    false,
                );
            }
        }
    }
}

/// Translation of `-[SDL_uikitview pencilHovering:]`.
#[cfg(not(target_os = "tvos"))]
extern "C" fn view_pencil_hovering(this: Id, _sel: Sel, recognizer: Id) {
    // SAFETY: UIGestureRecognizer's state.
    let state: UIGestureRecognizerState = unsafe { msg_send![recognizer, state] };
    match state {
        UIGestureRecognizerStateBegan | UIGestureRecognizerStateChanged => {
            super::pen::handle_pen_hover(this, recognizer);
        }

        UIGestureRecognizerStateEnded | UIGestureRecognizerStateCancelled => {
            // we track touches elsewhere, so if a hover "ends" we'll deal with that there.
        }

        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Touches

/// Translation of `-[SDL_uikitview touchTypeForTouch:]`.
fn touch_type_for_touch(touch: Id) -> TouchDeviceType {
    // SAFETY: a UITouch property (iOS 9).
    let touch_type: UITouchType = unsafe { msg_send![touch, type] };
    if touch_type == UITouchTypeIndirect {
        return TouchDeviceType::IndirectRelative;
    }
    TouchDeviceType::Direct
}

/// Translation of `-[SDL_uikitview touchIdForType:]`.
fn touch_id_for_type(this: Id, touch_type: TouchDeviceType) -> TouchID {
    // SAFETY: this view's variables.
    unsafe {
        match touch_type {
            TouchDeviceType::IndirectRelative => *touch_id_ivar(this, c"indirectTouchId"),
            _ => *touch_id_ivar(this, c"directTouchId"),
        }
    }
}

/// A touch's position in a view, in points or (`normalize`) as a fraction
/// of the view's size. Translation of
/// `-[SDL_uikitview touchLocation:shouldNormalize:]`.
pub(crate) fn touch_location(this: Id, touch: Id, normalize: bool) -> CGPoint {
    // SAFETY: UITouch's location and UIView's bounds.
    unsafe {
        let mut point: CGPoint = msg_send![touch, locationInView: this];

        if normalize {
            let bounds: CGRect = msg_send![this, bounds];
            point.x /= bounds.size.width;
            point.y /= bounds.size.height;
        }

        point
    }
}

/// Translation of `-[SDL_uikitview pressureForTouch:]`.
fn pressure_for_touch(touch: Id) -> f32 {
    // SAFETY: a UITouch property.
    let force: CGFloat = unsafe { msg_send![touch, force] };
    force as f32
}

/// The touches of a set (`for (UITouch *touch in touches)`, through its
/// enumerator).
fn each_touch(touches: Id, mut f: impl FnMut(Id)) {
    // SAFETY: an NSSet's enumerator (autoreleased) and its objects.
    unsafe {
        let enumerator: Id = msg_send![touches, objectEnumerator];
        loop {
            let touch: Id = msg_send![enumerator, nextObject];
            if touch.is_null() {
                break;
            }
            f(touch);
        }
    }
}

/// What happened to the touches (which `touches*:withEvent:`).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Began,
    Moved,
    Ended,
    Cancelled,
}

/// The Pencil's and the pointer's touches, handled apart (not on tvOS):
/// whether `touch` was one.
#[cfg(not(target_os = "tvos"))]
fn handle_pencil_or_pointer(this: Id, touch: Id, event: Id, phase: Phase) -> bool {
    // SAFETY: a UITouch property (iOS 9).
    let touch_type: UITouchType = unsafe { msg_send![touch, type] };
    if available!(ios(13, 0)) && touch_type == UITouchTypePencil {
        match phase {
            Phase::Began => super::pen::handle_pen_press(this, touch),
            Phase::Moved => super::pen::handle_pen_motion(this, touch),
            Phase::Ended | Phase::Cancelled => super::pen::handle_pen_release(this, touch),
        }
        return true;
    }

    if available!(ios(13, 4)) && touch_type == UITouchTypeIndirectPointer {
        match phase {
            Phase::Began => indirect_pointer_pressed(this, touch, event),
            Phase::Moved => indirect_pointer_moving(this, touch),
            Phase::Ended | Phase::Cancelled => indirect_pointer_released(this, touch, event),
        }
        return true;
    }
    false
}

/// `-[SDL_uikitview touchesBegan:withEvent:]` and the others: each finger's
/// touch event (the Pencil's and the pointer's apart).
fn handle_touches(this: Id, touches: Id, event: Id, phase: Phase) {
    // SAFETY: a UIEvent property.
    let event_timestamp: NSTimeInterval = unsafe { msg_send![event, timestamp] };
    each_touch(touches, |touch| {
        #[cfg(not(target_os = "tvos"))]
        if handle_pencil_or_pointer(this, touch, event, phase) {
            return;
        }

        let touch_type = touch_type_for_touch(touch);
        let touch_id = touch_id_for_type(this, touch_type);
        let pressure = pressure_for_touch(touch);

        // (upstream skips the touch if SDL_AddTouch() fails, which only
        // running out of memory makes it do)
        add_touch(touch_id, touch_type, "");

        // FIXME, need to send: int clicks = (int) touch.tapCount; ?

        let location_in_view = touch_location(this, touch, true);
        let timestamp = get_event_timestamp(event_timestamp);
        let finger_id = touch as usize as FingerID;
        let (x, y) = (location_in_view.x as f32, location_in_view.y as f32);
        let window = get_sdl_window(this);
        match phase {
            Phase::Began => send_touch(
                timestamp,
                touch_id,
                finger_id,
                window,
                EventType::FINGER_DOWN,
                x,
                y,
                pressure,
            ),
            Phase::Ended => send_touch(
                timestamp,
                touch_id,
                finger_id,
                window,
                EventType::FINGER_UP,
                x,
                y,
                pressure,
            ),
            Phase::Cancelled => send_touch(
                timestamp,
                touch_id,
                finger_id,
                window,
                EventType::FINGER_CANCELED,
                x,
                y,
                pressure,
            ),
            Phase::Moved => {
                send_touch_motion(timestamp, touch_id, finger_id, window, x, y, pressure)
            }
        }
    });
}

/// Translation of `-[SDL_uikitview touchesBegan:withEvent:]`.
extern "C" fn view_touches_began(this: Id, _sel: Sel, touches: Id, event: Id) {
    handle_touches(this, touches, event, Phase::Began);
}

/// Translation of `-[SDL_uikitview touchesEnded:withEvent:]`.
extern "C" fn view_touches_ended(this: Id, _sel: Sel, touches: Id, event: Id) {
    handle_touches(this, touches, event, Phase::Ended);
}

/// Translation of `-[SDL_uikitview touchesCancelled:withEvent:]`.
extern "C" fn view_touches_cancelled(this: Id, _sel: Sel, touches: Id, event: Id) {
    handle_touches(this, touches, event, Phase::Cancelled);
}

/// Translation of `-[SDL_uikitview touchesMoved:withEvent:]`.
extern "C" fn view_touches_moved(this: Id, _sel: Sel, touches: Id, event: Id) {
    handle_touches(this, touches, event, Phase::Moved);
}

/// Translation of `-[SDL_uikitview safeAreaInsetsDidChange]`.
extern "C" fn view_safe_area_insets_did_change(this: Id, _sel: Sel) {
    // Update the safe area insets
    // (no window: SDL_SetWindowSafeAreaInsets() refuses a NULL one)
    if let Some(window) = get_sdl_window(this) {
        // SAFETY: a UIView property.
        let insets: UIEdgeInsets = unsafe { msg_send![this, safeAreaInsets] };
        crate::video::core::set_window_safe_area_insets(
            window,
            (insets.left as f32).ceil() as i32,
            (insets.right as f32).ceil() as i32,
            (insets.top as f32).ceil() as i32,
            (insets.bottom as f32).ceil() as i32,
        );
    }
}

/// Translation of `-[SDL_uikitview sdlPinchGesture:]`.
#[cfg(not(target_os = "tvos"))]
extern "C" fn view_pinch_gesture(this: Id, _sel: Sel, sender: Id) {
    use crate::core::apple::objc::NSUInteger;
    use crate::events::touch::send_pinch;

    // SAFETY: UIPinchGestureRecognizer's properties and methods, and
    // UIView's bounds.
    let (scale, state, pinch_has_two_points, point1, point2, bounds) = unsafe {
        let scale: CGFloat = msg_send![sender, scale];
        let state: UIGestureRecognizerState = msg_send![sender, state];
        let touches: NSUInteger = msg_send![sender, numberOfTouches];
        let pinch_has_two_points = touches == 2;
        let point1: CGPoint = msg_send![sender, locationOfTouch: 0 as NSUInteger inView: this];
        let point2: CGPoint = if pinch_has_two_points {
            msg_send![sender, locationOfTouch: 1 as NSUInteger inView: this]
        } else {
            point1
        };
        let bounds: CGRect = msg_send![this, bounds];
        (scale, state, pinch_has_two_points, point1, point2, bounds)
    };
    let mut focus_x = (point1.x + point2.x) / 2.0;
    let mut focus_y = (point1.y + point2.y) / 2.0;
    let mut span_x = (point1.x - point2.x).abs();
    let mut span_y = (point1.y - point2.y).abs();
    focus_x /= bounds.size.width;
    focus_y /= bounds.size.height;
    span_x /= bounds.size.width;
    span_y /= bounds.size.height;

    let window = get_sdl_window(this).unwrap_or(0);
    let pinch_scale = pinch_scale_ivar(this);
    // SAFETY: this view's variable.
    unsafe {
        match state {
            UIGestureRecognizerStateBegan => {
                *pinch_scale = 1.0;
                send_pinch(
                    EventType::PINCH_BEGIN,
                    std::time::Duration::ZERO,
                    window,
                    0.0,
                    span_x as f32,
                    span_y as f32,
                    focus_x as f32,
                    focus_y as f32,
                );
            }

            UIGestureRecognizerStateChanged => {
                if *pinch_scale > 0.0 && pinch_has_two_points {
                    send_pinch(
                        EventType::PINCH_UPDATE,
                        std::time::Duration::ZERO,
                        window,
                        (scale / *pinch_scale as CGFloat) as f32,
                        span_x as f32,
                        span_y as f32,
                        focus_x as f32,
                        focus_y as f32,
                    );
                }
                *pinch_scale = scale as f32;
            }

            UIGestureRecognizerStateFailed
            | UIGestureRecognizerStateEnded
            | UIGestureRecognizerStateCancelled => {
                send_pinch(
                    EventType::PINCH_END,
                    std::time::Duration::ZERO,
                    window,
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
}
