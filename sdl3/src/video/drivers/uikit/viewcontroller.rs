// Rust translation of the window parts of
// src/video/uikit/SDL_uikitviewcontroller.m and
// SDL_uikitviewcontroller.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see
// LICENSE.txt.

//! `SDL_uikitviewcontroller`, a window's root view controller: it
//! reports the view's size as the window's ([`EventType::WINDOW_RESIZED`]
//! after each layout), the orientations the window supports
//! ([`get_supported_orientations`]), whether the status bar is hidden
//! (fullscreen and borderless windows), the home indicator and the screen
//! edges that defer the system's gestures (`SDL_HINT_IOS_HIDE_HOME_INDICATOR`),
//! the system theme when the traits change, and the animation callback
//! ([`set_animation_callback`], `SDL_SetiOSAnimationCallback()`), which a
//! `CADisplayLink` calls once every `interval` frames.
//!
//! Not translated yet: the on-screen keyboard and text input (the hidden
//! `SDLUITextField` and its delegate, the keyboard's notifications, the
//! text input properties and area, the key commands for the arrows and
//! escape, and the rotation flag `viewWillTransitionToSize:` keeps for
//! the keyboard), which come with the keyboard; `prefersPointerLocked`
//! (the game controllers' mouse's relative mode), which comes with the
//! mouse (until then UIKit's default, unlocked, as upstream's with no
//! relative mode); and tvOS's base class (`GCEventViewController`) and
//! hint (`SDL_HINT_APPLE_TV_CONTROLLER_UI_EVENTS`), which come with tvOS.
//! The view controller keeps the window's ID; once the window is gone it
//! answers as for a window with no flags.
//!
//! [`get_supported_orientations`]: super::window::get_supported_orientations
//! [`EventType::WINDOW_RESIZED`]: crate::events::EventType::WINDOW_RESIZED

use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::sync::atomic::{AtomicI32, Ordering};

use crate::core::apple::foundation::{available, CGRect};
#[cfg(not(target_os = "tvos"))]
use crate::core::apple::objc::encoding;
use crate::core::apple::objc::{
    class, imp, ivar_ptr, msg_send, msg_send_super, sel, Class, ClassBuilder, DeclaredClass, Id,
    NSInteger, Sel, StrongPtr, NIL,
};
#[cfg(not(target_os = "tvos"))]
use crate::core::apple::objc::{Bool, NSUInteger, NO, YES};
use crate::events::window::{send_window_event, WindowFlags};
use crate::events::{EventType, WindowID};
use crate::video::core::with_window;

use super::sys::*;

/// The instance variables of `SDL_uikitviewcontroller` (through its
/// `_state` variable). Only the main thread uses them, but for
/// `homeIndicatorHidden`, which the hint's callback sets.
struct State {
    /// `window` (the SDL window's ID).
    window: WindowID,
    /// `displayLink` (nil when there's none).
    display_link: RefCell<StrongPtr>,
    animation_interval: Cell<i32>,
    /// `animationCallback` and `animationCallbackParam`.
    animation_callback: RefCell<Option<Box<dyn FnMut()>>>,
    /// Bumped each time the animation callback is set, so that a callback
    /// that sets a new one (or none) while it runs isn't put back.
    animation_generation: Cell<u64>,
    /// `homeIndicatorHidden`: -1 (the hint is unset), or the hint's value.
    #[cfg(not(target_os = "tvos"))]
    home_indicator_hidden: AtomicI32,
    /// The `SDL_HINT_IOS_HIDE_HOME_INDICATOR` callback (removed with the
    /// view controller).
    #[cfg(not(target_os = "tvos"))]
    hint_callback: RefCell<Option<crate::hints::Callback>>,
}

static SDL_UIKITVIEWCONTROLLER: DeclaredClass = DeclaredClass::new();

/// The `SDL_uikitviewcontroller` class (`@interface SDL_uikitviewcontroller
/// : UIViewController`, `GCEventViewController` on tvOS, see the module's
/// description).
pub(crate) fn viewcontroller_class() -> Class {
    SDL_UIKITVIEWCONTROLLER.get(|| {
        let mut b = ClassBuilder::new(c"SDL_uikitviewcontroller", class!(UIViewController))
            .expect("SDL_uikitviewcontroller");
        b.add_ivar::<*mut c_void>(c"_state", c"^v");
        // SAFETY: the implementations have the signatures of the encodings.
        unsafe {
            b.add_method(
                sel!(dealloc),
                imp!(vc_dealloc as extern "C" fn(Id, Sel)),
                c"v@:",
            );
            b.add_method(
                sel!(traitCollectionDidChange:),
                imp!(vc_trait_collection_did_change as extern "C" fn(Id, Sel, Id)),
                c"v@:@",
            );
            b.add_method(
                sel!(doLoop:),
                imp!(vc_do_loop as extern "C" fn(Id, Sel, Id)),
                c"v@:@",
            );
            b.add_method(
                sel!(loadView),
                imp!(vc_load_view as extern "C" fn(Id, Sel)),
                c"v@:",
            );
            b.add_method(
                sel!(viewDidLayoutSubviews),
                imp!(vc_view_did_layout_subviews as extern "C" fn(Id, Sel)),
                c"v@:",
            );
            b.add_method(
                sel!(setView:),
                imp!(vc_set_view as extern "C" fn(Id, Sel, Id)),
                c"v@:@",
            );
            #[cfg(not(target_os = "tvos"))]
            {
                b.add_method(
                    sel!(supportedInterfaceOrientations),
                    imp!(
                        vc_supported_interface_orientations as extern "C" fn(Id, Sel) -> NSUInteger
                    ),
                    c"Q@:",
                );
                b.add_method(
                    sel!(prefersStatusBarHidden),
                    imp!(vc_prefers_status_bar_hidden as extern "C" fn(Id, Sel) -> Bool),
                    &encoding("B@:"),
                );
                b.add_method(
                    sel!(prefersHomeIndicatorAutoHidden),
                    imp!(vc_prefers_home_indicator_auto_hidden as extern "C" fn(Id, Sel) -> Bool),
                    &encoding("B@:"),
                );
                b.add_method(
                    sel!(preferredScreenEdgesDeferringSystemGestures),
                    imp!(
                        vc_preferred_screen_edges_deferring_system_gestures
                            as extern "C" fn(Id, Sel) -> UIRectEdge
                    ),
                    c"Q@:",
                );
            }
        }
        b.register()
    })
}

/// The `_state` variable.
fn state_ivar(this: Id) -> *mut *mut State {
    // SAFETY: an SDL_uikitviewcontroller, which has the variable.
    unsafe { ivar_ptr::<*mut State>(this, viewcontroller_class(), c"_state") }
}

/// The view controller's state (`None` before init and after dealloc).
fn state<'a>(this: Id) -> Option<&'a State> {
    // SAFETY: the box the initializer made, alive until dealloc (the
    // view controller is alive while its methods run).
    unsafe { (*state_ivar(this)).as_ref() }
}

/// The SDL window of a view controller.
pub(crate) fn window_of(this: Id) -> Option<WindowID> {
    state(this).map(|s| s.window)
}

/// `window->flags` (none once the window is gone).
#[cfg(not(target_os = "tvos"))]
fn window_flags(this: Id) -> WindowFlags {
    window_of(this)
        .and_then(|w| with_window(w, |w| w.flags()).ok())
        .unwrap_or_default()
}

/// Translation of `SDL_HideHomeIndicatorHintChanged()`.
#[cfg(not(target_os = "tvos"))]
fn hide_home_indicator_hint_changed(viewcontroller: Id, hint: Option<&str>) {
    let Some(state) = state(viewcontroller) else {
        return;
    };
    let hidden = match hint {
        Some(h) if !h.is_empty() => crate::stdlib::atoi(h),
        _ => -1,
    };
    state.home_indicator_hidden.store(hidden, Ordering::Relaxed);
    // SAFETY: UIViewController's methods (iOS 11).
    unsafe {
        let () = msg_send![viewcontroller, setNeedsUpdateOfHomeIndicatorAutoHidden];
        let () = msg_send![
            viewcontroller,
            setNeedsUpdateOfScreenEdgesDeferringSystemGestures
        ];
    }
}

/// A new view controller for a window (owned). Translation of
/// `-[SDL_uikitviewcontroller initWithSDLWindow:]`.
pub(crate) fn new_view_controller(window: WindowID) -> StrongPtr {
    // SAFETY: UIViewController's alloc and initializer (owned).
    let this = unsafe {
        let this: Id = msg_send![viewcontroller_class(), alloc];
        StrongPtr::from_owned(msg_send![this, initWithNibName: NIL bundle: NIL])
    };
    if this.is_nil() {
        return this;
    }
    let new_state = Box::new(State {
        window,
        display_link: RefCell::new(StrongPtr::nil()),
        animation_interval: Cell::new(0),
        animation_callback: RefCell::new(None),
        animation_generation: Cell::new(0),
        #[cfg(not(target_os = "tvos"))]
        home_indicator_hidden: AtomicI32::new(0),
        #[cfg(not(target_os = "tvos"))]
        hint_callback: RefCell::new(None),
    });
    // SAFETY: the view controller's variable, owning the box until dealloc.
    unsafe { *state_ivar(this.get()) = Box::into_raw(new_state) };
    let state = state(this.get()).expect("the view controller's state");

    // (SDL_IPHONE_KEYBOARD's initKeyboard: see the module's description)

    #[cfg(not(target_os = "tvos"))]
    {
        // (the callback has the view controller unretained, as upstream's
        // userdata; it's removed in dealloc)
        let vc = this.get() as usize;
        let callback = crate::hints::watch(crate::hints::IOS_HIDE_HOME_INDICATOR, move |change| {
            crate::core::apple::objc::autoreleasepool(|| {
                hide_home_indicator_hint_changed(vc as Id, change.new_value)
            });
        });
        *state.hint_callback.borrow_mut() = callback.ok();
    }

    // Enable high refresh rates on iOS
    // To enable this on phones, you should add the following line to Info.plist:
    // <key>CADisableMinimumFrameDurationOnPhone</key> <true/>
    if available!(ios(15, 0), tvos(15, 0)) {
        let mode = crate::video::primary_display()
            .and_then(crate::video::desktop_display_mode)
            .ok();
        if let Some(mode) = mode.filter(|mode| mode.refresh_rate > 60.0) {
            let frame_rate = mode.refresh_rate as i32;
            // SAFETY: CADisplayLink's class method (autoreleased, retained
            // here) targeting this view controller's doLoop:, its
            // properties, and the current run loop.
            unsafe {
                let display_link: Id = msg_send![class!(CADisplayLink),
                    displayLinkWithTarget: this.get()
                    selector: sel!(doLoop:)];
                let display_link = StrongPtr::retain(display_link);
                let () = msg_send![display_link.get(), setPreferredFrameRateRange: CAFrameRateRangeMake(
                    ((frame_rate * 2) / 3) as f32,
                    frame_rate as f32,
                    frame_rate as f32,
                )];
                let run_loop: Id = msg_send![class!(NSRunLoop), currentRunLoop];
                let () = msg_send![display_link.get(), addToRunLoop: run_loop forMode: NSDefaultRunLoopMode];
                *state.display_link.borrow_mut() = display_link;
            }
        }
    }

    this
}

/// Translation of `-[SDL_uikitviewcontroller dealloc]`.
extern "C" fn vc_dealloc(this: Id, _sel: Sel) {
    // (SDL_IPHONE_KEYBOARD's deinitKeyboard: see the module's description)

    // SAFETY: the box new_view_controller() made (taken once); dropping
    // it removes the hint's callback and releases the display link.
    unsafe {
        let slot = state_ivar(this);
        let state = std::mem::replace(&mut *slot, std::ptr::null_mut());
        if !state.is_null() {
            drop(Box::from_raw(state));
        }
        let () = msg_send_super![this, viewcontroller_class(), dealloc];
    }
}

/// Translation of `-[SDL_uikitviewcontroller traitCollectionDidChange:]`.
extern "C" fn vc_trait_collection_did_change(_this: Id, _sel: Sel, _previous: Id) {
    crate::video::core::set_system_theme(super::get_system_theme());
}

/// Call `callback` once every `interval` frames (of the window's screen)
/// from now on, in place of the one before; `None` stops it. Translation of
/// `-[SDL_uikitviewcontroller setAnimationCallback:callback:callbackParam:]`.
pub(crate) fn set_animation_callback(
    this: Id,
    mut interval: i32,
    callback: Option<Box<dyn FnMut()>>,
) {
    stop_animation(this);

    let Some(state) = state(this) else {
        return;
    };
    if interval <= 0 {
        interval = 1;
    }
    state.animation_interval.set(interval);
    let has_callback = callback.is_some();
    *state.animation_callback.borrow_mut() = callback;
    state
        .animation_generation
        .set(state.animation_generation.get().wrapping_add(1));

    if has_callback {
        start_animation(this);
    }
}

/// Translation of `-[SDL_uikitviewcontroller startAnimation]`.
pub(crate) fn start_animation(this: Id) {
    let Some(state) = state(this) else {
        return;
    };
    // SAFETY: CADisplayLink's class method (autoreleased, retained into the
    // state) targeting this view controller's doLoop:, its properties, the
    // window's screen, and the current run loop.
    unsafe {
        let display_link: Id = msg_send![class!(CADisplayLink),
            displayLinkWithTarget: this
            selector: sel!(doLoop:)];
        let display_link = StrongPtr::retain(display_link);

        #[cfg(target_os = "visionos")]
        {
            let fps: NSInteger = 90 / state.animation_interval.get() as NSInteger; //TODO: Get frame max frame rate on visionOS
            let () = msg_send![display_link.get(), setPreferredFramesPerSecond: fps];
        }
        #[cfg(not(target_os = "visionos"))]
        {
            let uiwindow =
                super::window::window_data(state.window).map_or(NIL, |data| data.uiwindow());
            let screen: Id = msg_send![uiwindow, screen];
            let maximum: NSInteger = msg_send![screen, maximumFramesPerSecond];
            let fps = maximum / state.animation_interval.get() as NSInteger;
            let () = msg_send![display_link.get(), setPreferredFramesPerSecond: fps];
        }

        let run_loop: Id = msg_send![class!(NSRunLoop), currentRunLoop];
        let () =
            msg_send![display_link.get(), addToRunLoop: run_loop forMode: NSDefaultRunLoopMode];
        let old = state.display_link.replace(display_link);
        drop(old);
    }
}

/// Translation of `-[SDL_uikitviewcontroller stopAnimation]`.
pub(crate) fn stop_animation(this: Id) {
    let Some(state) = state(this) else {
        return;
    };
    let display_link = state.display_link.replace(StrongPtr::nil());
    // SAFETY: a CADisplayLink (or nil); invalidating it lets go of this
    // view controller.
    unsafe {
        let () = msg_send![display_link.get(), invalidate];
    }
    drop(display_link);
}

/// Translation of `-[SDL_uikitviewcontroller doLoop:]`.
extern "C" fn vc_do_loop(this: Id, _sel: Sel, _sender: Id) {
    // (kept while the callback runs, which may destroy the window, and so
    // the view controller)
    // SAFETY: a valid object, retained.
    let _this = unsafe { StrongPtr::retain(this) };
    let Some(state) = state(this) else {
        return;
    };

    // Don't run the game loop while a messagebox is up
    // (message boxes aren't translated yet)
    let Some(mut callback) = state.animation_callback.borrow_mut().take() else {
        return;
    };
    // See the comment in the function definition.
    // (UIKit_GL_RestoreCurrentContext() comes with OpenGL ES, which isn't
    // translated yet)

    let generation = state.animation_generation.get();
    callback();
    // (put back, unless the callback set another one or none)
    if state.animation_generation.get() == generation {
        *state.animation_callback.borrow_mut() = Some(callback);
    }
}

/// Translation of `-[SDL_uikitviewcontroller loadView]`.
extern "C" fn vc_load_view(_this: Id, _sel: Sel) {
    // Do nothing.
}

/// Translation of `-[SDL_uikitviewcontroller viewDidLayoutSubviews]`.
extern "C" fn vc_view_did_layout_subviews(this: Id, _sel: Sel) {
    // SAFETY: UIViewController's view and its bounds.
    let bounds: CGRect = unsafe {
        let view: Id = msg_send![this, view];
        msg_send![view, bounds]
    };
    let w = bounds.size.width as i32;
    let h = bounds.size.height as i32;

    if let Some(window) = window_of(this) {
        send_window_event(window, EventType::WINDOW_RESIZED, w, h);
    }
}

/// Translation of `-[SDL_uikitviewcontroller setView:]` (the game
/// controllers' interaction with the view; the hidden text field the
/// keyboard adds to it isn't translated yet).
extern "C" fn vc_set_view(this: Id, _sel: Sel, view: Id) {
    // SAFETY: UIViewController's setView:.
    unsafe {
        let () = msg_send_super![this, viewcontroller_class(), setView: view];
    }

    if crate::init::was_init(crate::init::InitFlags::JOYSTICK)
        .contains(crate::init::InitFlags::JOYSTICK)
    {
        super::set_view_game_controller_interaction(view, true);
    }

    // (the text field: see the module's description)
}

/// Translation of `-[SDL_uikitviewcontroller supportedInterfaceOrientations]`.
#[cfg(not(target_os = "tvos"))]
extern "C" fn vc_supported_interface_orientations(this: Id, _sel: Sel) -> NSUInteger {
    match window_of(this) {
        Some(window) => super::window::get_supported_orientations(window),
        None => UIInterfaceOrientationMaskAll,
    }
}

/// Translation of `-[SDL_uikitviewcontroller prefersStatusBarHidden]`.
#[cfg(not(target_os = "tvos"))]
extern "C" fn vc_prefers_status_bar_hidden(this: Id, _sel: Sel) -> Bool {
    let hidden = window_flags(this).intersects(WindowFlags::FULLSCREEN | WindowFlags::BORDERLESS);
    Bool::from(hidden)
}

/// Translation of `-[SDL_uikitviewcontroller prefersHomeIndicatorAutoHidden]`.
#[cfg(not(target_os = "tvos"))]
extern "C" fn vc_prefers_home_indicator_auto_hidden(this: Id, _sel: Sel) -> Bool {
    let mut hidden = NO;
    if state(this).is_some_and(|s| s.home_indicator_hidden.load(Ordering::Relaxed) == 1) {
        hidden = YES;
    }
    hidden
}

/// Translation of
/// `-[SDL_uikitviewcontroller preferredScreenEdgesDeferringSystemGestures]`.
#[cfg(not(target_os = "tvos"))]
extern "C" fn vc_preferred_screen_edges_deferring_system_gestures(
    this: Id,
    _sel: Sel,
) -> UIRectEdge {
    let home_indicator_hidden =
        state(this).map_or(-1, |s| s.home_indicator_hidden.load(Ordering::Relaxed));
    if home_indicator_hidden >= 0 {
        if home_indicator_hidden == 2 {
            return UIRectEdgeAll;
        } else {
            return UIRectEdgeNone;
        }
    }

    // By default, fullscreen and borderless windows get all screen gestures
    if window_flags(this).intersects(WindowFlags::FULLSCREEN | WindowFlags::BORDERLESS) {
        UIRectEdgeAll
    } else {
        UIRectEdgeNone
    }
}
