// Rust translation of the base of src/video/uikit/SDL_uikitview.m and
// SDL_uikitview.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! `SDL_uikitview`, the `UIView` a window's view controller shows: the
//! window it belongs to ([`set_sdl_window`], which makes it the view
//! controller's view and keeps it in the window's views, and
//! [`get_sdl_window`]), its size following the window's (it resizes with
//! its superview), and its safe area, which becomes the window's
//! ([`set_window_safe_area_insets`]).
//!
//! Not translated yet: the touches (with the pinch gesture), the hardware
//! keys (`UIPress`), the iPad's pointer (`UIPointerInteraction` and its
//! hover gesture) and the Pencil's hover; until then the view leaves
//! them to `UIView` (the next responder). Upstream also adds the direct
//! touch device when the view is made, which comes with the touches.
//!
//! [`set_window_safe_area_insets`]: crate::video::core::set_window_safe_area_insets

use crate::core::apple::foundation::CGRect;
use crate::core::apple::objc::{
    class, imp, ivar_ptr, msg_send, msg_send_super, sel, Class, ClassBuilder, DeclaredClass, Id,
    Sel, StrongPtr, NIL, YES,
};
use crate::events::WindowID;

use super::sys::*;
use super::window::window_data;

static SDL_UIKITVIEW: DeclaredClass = DeclaredClass::new();

/// The `SDL_uikitview` class (`@interface SDL_uikitview : UIView`), with
/// its `sdlwindow` instance variable (the window's ID, 0 for none).
pub(crate) fn uikitview_class() -> Class {
    SDL_UIKITVIEW.get(|| {
        let mut b = ClassBuilder::new(c"SDL_uikitview", class!(UIView)).expect("SDL_uikitview");
        b.add_ivar::<u32>(c"sdlwindow", c"I");
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

/// A new view (owned). Translation of `[[SDL_uikitview alloc]
/// initWithFrame:frame]`.
pub(crate) fn new_view(frame: CGRect) -> StrongPtr {
    // SAFETY: the class's alloc and initializer (owned).
    unsafe {
        let view: Id = msg_send![uikitview_class(), alloc];
        StrongPtr::from_owned(msg_send![view, initWithFrame: frame])
    }
}

/// Translation of `-[SDL_uikitview initWithFrame:]`.
extern "C" fn view_init_with_frame(this: Id, _sel: Sel, frame: CGRect) -> Id {
    // SAFETY: UIView's initializer and properties.
    unsafe {
        let this: Id = msg_send_super![this, uikitview_class(), initWithFrame: frame];
        if !this.is_null() {
            // (tvOS's swipe gestures, the pinch gesture, the touch IDs,
            // multipleTouchEnabled, the direct touch device, the Pencil's
            // and the pointer's hover gestures and the pointer interaction
            // come with the touches: see the module's description)

            let () = msg_send![this, setAutoresizingMask: UIViewAutoresizingFlexibleWidth | UIViewAutoresizingFlexibleHeight];
            let () = msg_send![this, setAutoresizesSubviews: YES];
            *sdlwindow_ivar(this) = 0;
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
