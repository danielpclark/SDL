// Rust translation of src/video/uikit/SDL_uikitviewcontroller.m and
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
//! the system theme when the traits change, whether the pointer is locked
//! (in the game controllers' mice's relative mode), and the animation
//! callback ([`set_animation_callback`], `SDL_SetiOSAnimationCallback()`),
//! which a `CADisplayLink` calls once every `interval` frames.
//!
//! It also does text input, through a hidden text field (`SDLUITextField`,
//! a subview of the window's view) whose delegate it is: starting text
//! input makes the field the first responder, which shows the on-screen
//! keyboard (when there's no hardware keyboard), and stopping it lets go
//! ([`start_text_input`], [`stop_text_input`]). The text input properties
//! become the field's ([`set_text_field_properties`]: keyboard type,
//! content type, secure entry, autocapitalization, autocorrection and
//! spell checking, and whether the return key enables itself). What is
//! typed in the field is SDL's text input: the field starts with 64
//! spaces, so that deleting has something to delete, and each change
//! becomes backspaces for what's gone (with no hardware keyboard), a key
//! press for each new character (when no hardware key is down) and the
//! new text; the return key is a press of Return, which stops text input
//! with `SDL_HINT_RETURN_KEY_HIDES_IME`. The keyboard's notifications tell
//! whether the on-screen keyboard is shown (the
//! `SCREEN_KEYBOARD_SHOWN`/`HIDDEN` events) and its height, by which the
//! view moves up when it would hide the text input area
//! ([`update_keyboard`], [`crate::video::Window::set_text_input_area`]).
//! Its key commands are the arrow keys and Escape, pressed and released.
//!
//! On tvOS the view controller is a `GCEventViewController` (upstream's
//! `SDLRootViewController`), whose game controllers' events go to UIKit
//! with `SDL_HINT_APPLE_TV_CONTROLLER_UI_EVENTS` only
//! (`controllerUserInteractionEnabled`). GameController is loaded at run
//! time here, so without it the base class is `UIViewController` (`Note`:
//! upstream links GameController).
//! The view controller keeps the window's ID; once the window is gone it
//! answers as for a window with no flags.
//!
//! [`get_supported_orientations`]: super::window::get_supported_orientations
//! [`EventType::WINDOW_RESIZED`]: crate::events::EventType::WINDOW_RESIZED

use std::cell::{Cell, RefCell};
use std::ffi::{c_char, c_void, CStr};
#[cfg(not(target_os = "tvos"))]
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::Duration;

use crate::core::apple::block::RcBlock;
use crate::core::apple::foundation::{
    available, ns_string, CGFloat, CGPoint, CGRect, CGSize, NSRange,
};
use crate::core::apple::objc::{
    autoreleasepool, class, encoding, imp, ivar_ptr, msg_send, msg_send_super, sel, Bool, Class,
    ClassBuilder, DeclaredClass, Id, NSInteger, NSUInteger, Sel, StrongPtr, NIL, NO, YES,
};
use crate::error::{Error, Result};
use crate::events::keyboard::{
    hardware_keyboard_key_pressed, has_keyboard, send_keyboard_key, send_keyboard_key_auto_release,
    send_keyboard_text, send_keyboard_unicode_code, Scancode, GLOBAL_KEYBOARD_ID,
};
use crate::events::window::send_window_event;
#[cfg(not(target_os = "tvos"))]
use crate::events::window::WindowFlags;
use crate::events::{EventType, WindowID};
use crate::properties::Properties;
use crate::video::core::with_window;
use crate::video::textinput::{
    send_screen_keyboard_hidden, send_screen_keyboard_shown, text_input_autocorrect,
    text_input_capitalization, text_input_multiline, text_input_type, Capitalization,
    TextInputType,
};
use crate::video::Rect;

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
    /// The `SDL_HINT_APPLE_TV_CONTROLLER_UI_EVENTS` callback (removed with
    /// the view controller).
    #[cfg(target_os = "tvos")]
    controller_ui_hint_callback: RefCell<Option<crate::hints::Callback>>,

    // SDL_IPHONE_KEYBOARD
    /// `textField` (an `SDLUITextField`).
    text_field: StrongPtr,
    /// `rotatingOrientation`: while the view's size changes with the
    /// interface's orientation (from `viewWillTransitionToSize:` to the
    /// transition's end). (Upstream keeps it, but reads it nowhere.)
    rotating_orientation: Cell<bool>,
    /// `committedText`: the text field's text as SDL last saw it.
    committed_text: RefCell<StrongPtr>,
    /// `obligateForBackspace`: the 64 spaces the text field holds, so
    /// that deleting has something to delete.
    obligate_for_backspace: StrongPtr,
    /// `isOTPMode`: the text input is a PIN (a one-time code), whose text
    /// isn't reset.
    is_otp_mode: Cell<bool>,
    /// `textInputRect` (the window's text input area).
    text_input_rect: Cell<Rect>,
    /// `keyboardHeight` (the on-screen keyboard's, in the view's points).
    keyboard_height: Cell<i32>,
    /// `textFieldFocused`.
    text_field_focused: Cell<bool>,
}

static SDLUITEXTFIELD: DeclaredClass = DeclaredClass::new();

/// The `SDLUITextField` class (`@interface SDLUITextField : UITextField`),
/// which can't paste.
fn text_field_class() -> Class {
    SDLUITEXTFIELD.get(|| {
        let mut b =
            ClassBuilder::new(c"SDLUITextField", class!(UITextField)).expect("SDLUITextField");
        // SAFETY: the implementation has the signature of the encoding.
        unsafe {
            b.add_method(
                sel!(canPerformAction:withSender:),
                imp!(text_field_can_perform_action as extern "C" fn(Id, Sel, Sel, Id) -> Bool),
                &encoding("B@::@"),
            );
        }
        b.register()
    })
}

/// Translation of `-[SDLUITextField canPerformAction:withSender:]`.
extern "C" fn text_field_can_perform_action(this: Id, _sel: Sel, action: Sel, sender: Id) -> Bool {
    if action == sel!(paste:) {
        return NO;
    }

    // SAFETY: UIResponder's method.
    unsafe {
        msg_send_super![this, text_field_class(), canPerformAction: action withSender: sender]
    }
}

static SDL_UIKITVIEWCONTROLLER: DeclaredClass = DeclaredClass::new();

/// The root view controller's base class (`SDLRootViewController`):
/// `UIViewController`, `GCEventViewController` on tvOS (when GameController
/// loads).
fn root_view_controller_class() -> Class {
    #[cfg(target_os = "tvos")]
    if super::sys::load_game_controller() {
        let class = class!(GCEventViewController);
        if !class.is_null() {
            return class;
        }
    }
    class!(UIViewController)
}

/// The `SDL_uikitviewcontroller` class (`@interface SDL_uikitviewcontroller
/// : SDLRootViewController`: `UIViewController`, `GCEventViewController` on
/// tvOS, see the module's description).
pub(crate) fn viewcontroller_class() -> Class {
    SDL_UIKITVIEWCONTROLLER.get(|| {
        let mut b = ClassBuilder::new(c"SDL_uikitviewcontroller", root_view_controller_class())
            .expect("SDL_uikitviewcontroller");
        b.add_ivar::<*mut c_void>(c"_state", c"^v");
        b.add_protocol(c"UITextFieldDelegate");
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
            // SDL_IPHONE_KEYBOARD
            b.add_method(
                sel!(keyCommands),
                imp!(vc_key_commands as extern "C" fn(Id, Sel) -> Id),
                c"@@:",
            );
            b.add_method(
                sel!(handleCommand:),
                imp!(vc_handle_command as extern "C" fn(Id, Sel, Id)),
                c"v@:@",
            );
            b.add_method(
                sel!(viewWillTransitionToSize:withTransitionCoordinator:),
                imp!(vc_view_will_transition_to_size as extern "C" fn(Id, Sel, CGSize, Id)),
                c"v@:{CGSize=dd}@",
            );
            b.add_method(
                sel!(keyboardWillShow:),
                imp!(vc_keyboard_will_show as extern "C" fn(Id, Sel, Id)),
                c"v@:@",
            );
            b.add_method(
                sel!(keyboardDidShow:),
                imp!(vc_keyboard_did_show as extern "C" fn(Id, Sel, Id)),
                c"v@:@",
            );
            b.add_method(
                sel!(keyboardWillHide:),
                imp!(vc_keyboard_will_hide as extern "C" fn(Id, Sel, Id)),
                c"v@:@",
            );
            b.add_method(
                sel!(keyboardDidHide:),
                imp!(vc_keyboard_did_hide as extern "C" fn(Id, Sel, Id)),
                c"v@:@",
            );
            b.add_method(
                sel!(textFieldTextDidChange:),
                imp!(vc_text_field_text_did_change as extern "C" fn(Id, Sel, Id)),
                c"v@:@",
            );
            b.add_method(
                sel!(textField:shouldChangeCharactersInRange:replacementString:),
                imp!(
                    vc_should_change_characters as extern "C" fn(Id, Sel, Id, NSRange, Id) -> Bool
                ),
                &encoding("B@:@{_NSRange=QQ}@"),
            );
            b.add_method(
                sel!(textFieldShouldReturn:),
                imp!(vc_text_field_should_return as extern "C" fn(Id, Sel, Id) -> Bool),
                &encoding("B@:@"),
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
                b.add_method(
                    sel!(prefersPointerLocked),
                    imp!(vc_prefers_pointer_locked as extern "C" fn(Id, Sel) -> Bool),
                    &encoding("B@:"),
                );
            }
            #[cfg(target_os = "visionos")]
            b.add_method(
                sel!(preferredContainerBackgroundStyle),
                imp!(
                    vc_preferred_container_background_style as extern "C" fn(Id, Sel) -> NSInteger
                ),
                c"q@:",
            );
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
    // (initKeyboard's text field, which the state keeps)
    // SAFETY: SDLUITextField's alloc and UIView's initializer (owned).
    let text_field = unsafe {
        let text_field: Id = msg_send![text_field_class(), alloc];
        StrongPtr::from_owned(msg_send![text_field, initWithFrame: CGRect::new(0.0, 0.0, 0.0, 0.0)])
    };
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
        #[cfg(target_os = "tvos")]
        controller_ui_hint_callback: RefCell::new(None),
        text_field,
        rotating_orientation: Cell::new(false),
        committed_text: RefCell::new(StrongPtr::nil()),
        obligate_for_backspace: ns_string(&" ".repeat(64)), // 64 space
        is_otp_mode: Cell::new(false),
        text_input_rect: Cell::new(Rect::default()),
        keyboard_height: Cell::new(0),
        text_field_focused: Cell::new(false),
    });
    // SAFETY: the view controller's variable, owning the box until dealloc.
    unsafe { *state_ivar(this.get()) = Box::into_raw(new_state) };
    let state = state(this.get()).expect("the view controller's state");

    // SDL_IPHONE_KEYBOARD
    init_keyboard(this.get(), state);
    state.rotating_orientation.set(false);

    #[cfg(target_os = "tvos")]
    {
        // (the callback has the view controller unretained, as upstream's
        // userdata; it's removed in dealloc)
        let vc = this.get() as usize;
        let callback =
            crate::hints::watch(crate::hints::APPLE_TV_CONTROLLER_UI_EVENTS, move |change| {
                crate::core::apple::objc::autoreleasepool(|| {
                    apple_tv_controller_ui_hint_changed(vc as Id, change.new_value)
                });
            });
        *state.controller_ui_hint_callback.borrow_mut() = callback.ok();
    }

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

    #[cfg(target_os = "visionos")]
    if available!(visionos(26, 0)) {
        let settings = super::window::window_data(window).and_then(|data| data.settings());
        if settings.is_some() {
            super::bridge::initialize_visionos_curved_ui(this.get());
        }
    }
    this
}

/// Translation of `-[SDL_uikitviewcontroller
/// preferredContainerBackgroundStyle]` (visionOS): hidden for a curved
/// window (no glass behind its content), else UIKit's default. (Never a
/// curved one here: see [`super::bridge`].)
#[cfg(target_os = "visionos")]
extern "C" fn vc_preferred_container_background_style(this: Id, _sel: Sel) -> NSInteger {
    if let Some(window) = window_of(this) {
        if super::window::window_data(window).is_some() && super::bridge::is_curved_window(window) {
            return UIContainerBackgroundStyleHidden;
        }
    }
    UIContainerBackgroundStyleAutomatic
}

/// Translation of `SDL_AppleTVControllerUIHintChanged()`: whether the game
/// controllers' events go to UIKit (`GCEventViewController`'s property;
/// nothing without GameController).
#[cfg(target_os = "tvos")]
fn apple_tv_controller_ui_hint_changed(viewcontroller: Id, hint: Option<&str>) {
    let enabled = hint.is_some_and(|h| !h.starts_with('0'));
    let class = class!(GCEventViewController);
    // SAFETY: the view controller, and GCEventViewController's property.
    unsafe {
        if !class.is_null() && crate::core::apple::objc::is_kind_of(viewcontroller, class) {
            let () =
                msg_send![viewcontroller, setControllerUserInteractionEnabled: Bool::from(enabled)];
        }
    }
}

/// Translation of `-[SDL_uikitviewcontroller dealloc]`.
extern "C" fn vc_dealloc(this: Id, _sel: Sel) {
    // SDL_IPHONE_KEYBOARD
    deinit_keyboard(this);

    // SAFETY: the box new_view_controller() made (taken once); dropping
    // it removes the hint's callback and releases the display link.
    unsafe {
        let slot = state_ivar(this);
        let state = *slot;
        *slot = std::ptr::null_mut();
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
    if super::messagebox::showing_message_box() {
        return;
    }
    let Some(mut callback) = state.animation_callback.borrow_mut().take() else {
        return;
    };
    // See the comment in the function definition.
    #[cfg(all(any(target_os = "ios", target_os = "tvos"), not(target_abi = "macabi")))]
    super::opengles::restore_current_context();

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

/// Translation of `-[SDL_uikitviewcontroller setView:]`.
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

    let Some(state) = state(this) else {
        return;
    };
    // SAFETY: UIView's method (nil does nothing).
    unsafe {
        let () = msg_send![view, addSubview: state.text_field.get()];
    }

    if state.text_field_focused.get() {
        /* startTextInput has been called before the text field was added to the view,
         * call it again for the text field to actually become first responder. */
        start_text_input(this);
    }
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

/// Translation of `-[SDL_uikitviewcontroller prefersPointerLocked]`.
#[cfg(not(target_os = "tvos"))]
extern "C" fn vc_prefers_pointer_locked(_this: Id, _sel: Sel) -> Bool {
    if super::events::gc_mouse_relative_mode() {
        YES
    } else {
        NO
    }
}

// ---------------------------------------------------------------------------
// ---- Keyboard related functionality below this line ----
// (SDL_IPHONE_KEYBOARD: every UIKit platform)

/// The UTF-8 bytes of an `NSString` (`[string UTF8String]`; `None` for
/// nil or a string that has none).
///
/// # Safety
///
/// `string` must be nil or an `NSString`.
unsafe fn utf8_bytes(string: Id) -> Option<Vec<u8>> {
    if string.is_null() {
        return None;
    }
    // SAFETY: the caller's contract; the buffer lives as long as the
    // autorelease pool, and is copied at once.
    unsafe {
        let p: *const c_char = msg_send![string, UTF8String];
        (!p.is_null()).then(|| CStr::from_ptr(p).to_bytes().to_vec())
    }
}

/// `[string length]` (0 for nil).
///
/// # Safety
///
/// As [`utf8_bytes`].
unsafe fn length(string: Id) -> NSUInteger {
    // SAFETY: the caller's contract.
    unsafe { msg_send![string, length] }
}

/// Set ourselves up as a UITextFieldDelegate. Translation of
/// `-[SDL_uikitviewcontroller initKeyboard]` (the text field and the
/// obligatory spaces are made with the state).
fn init_keyboard(this: Id, state: &State) {
    let text_field = state.text_field.get();
    // SAFETY: UITextField's properties, and the default notification
    // center with this view controller's methods (which take the
    // notification).
    unsafe {
        let () = msg_send![text_field, setDelegate: this];
        // placeholder so there is something to delete!
        let () = msg_send![text_field, setText: state.obligate_for_backspace.get()];
        let text: Id = msg_send![text_field, text];
        *state.committed_text.borrow_mut() = StrongPtr::retain(text);

        let () = msg_send![text_field, setHidden: YES];
        state.text_field_focused.set(false);
        state.is_otp_mode.set(false);

        let center: Id = msg_send![class!(NSNotificationCenter), defaultCenter];
        let observe = |selector: Sel, name: Id| {
            let () = msg_send![center, addObserver: this
                                          selector: selector
                                              name: name
                                            object: NIL];
        };
        #[cfg(not(target_os = "tvos"))]
        {
            observe(sel!(keyboardWillShow:), UIKeyboardWillShowNotification);
            observe(sel!(keyboardDidShow:), UIKeyboardDidShowNotification);
            observe(sel!(keyboardWillHide:), UIKeyboardWillHideNotification);
            observe(sel!(keyboardDidHide:), UIKeyboardDidHideNotification);
        }
        observe(
            sel!(textFieldTextDidChange:),
            UITextFieldTextDidChangeNotification,
        );
    }
}

/// Translation of `-[SDL_uikitviewcontroller keyCommands]`.
extern "C" fn vc_key_commands(_this: Id, _sel: Sel) -> Id {
    // SAFETY: NSMutableArray's alloc and init (owned), UIKeyCommand's
    // class method (autoreleased) and NSArray's (autoreleased, as the
    // method answers); UIKit's constant strings.
    unsafe {
        let commands: Id = msg_send![class!(NSMutableArray), alloc];
        let commands = StrongPtr::from_owned(msg_send![commands, init]);
        for input in [
            UIKeyInputUpArrow,
            UIKeyInputDownArrow,
            UIKeyInputLeftArrow,
            UIKeyInputRightArrow,
            UIKeyInputEscape,
        ] {
            let command: Id = msg_send![class!(UIKeyCommand),
                keyCommandWithInput: input
                modifierFlags: 0 as UIKeyModifierFlags // kNilOptions
                action: sel!(handleCommand:)];
            let () = msg_send![commands.get(), addObject: command];
        }
        msg_send![class!(NSArray), arrayWithArray: commands.get()]
    }
}

/// Translation of `-[SDL_uikitviewcontroller handleCommand:]`.
extern "C" fn vc_handle_command(_this: Id, _sel: Sel, key_command: Id) {
    // SAFETY: a UIKeyCommand property.
    let input: Id = unsafe { msg_send![key_command, input] };

    // (the same strings, as upstream compares them)
    // SAFETY: UIKit's constant strings.
    let scancode = unsafe {
        if input == UIKeyInputUpArrow {
            Scancode::UP
        } else if input == UIKeyInputDownArrow {
            Scancode::DOWN
        } else if input == UIKeyInputLeftArrow {
            Scancode::LEFT
        } else if input == UIKeyInputRightArrow {
            Scancode::RIGHT
        } else if input == UIKeyInputEscape {
            Scancode::ESCAPE
        } else {
            Scancode::UNKNOWN
        }
    };

    if scancode != Scancode::UNKNOWN {
        send_keyboard_key_auto_release(Duration::ZERO, scancode);
    }
}

/// Translation of
/// `-[SDL_uikitviewcontroller viewWillTransitionToSize:withTransitionCoordinator:]`.
extern "C" fn vc_view_will_transition_to_size(this: Id, _sel: Sel, size: CGSize, coordinator: Id) {
    // SAFETY: UIViewController's method.
    unsafe {
        let () = msg_send_super![this, viewcontroller_class(),
            viewWillTransitionToSize: size
            withTransitionCoordinator: coordinator];
    }
    let Some(state) = state(this) else {
        return;
    };
    state.rotating_orientation.set(true);
    // (the completion keeps the view controller, as upstream's block does)
    // SAFETY: a valid object, retained.
    let vc = unsafe { StrongPtr::retain(this) };
    let along = RcBlock::new1(|_context: Id| {});
    let completion = RcBlock::new1(move |_context: Id| {
        if let Some(state) = self::state(vc.get()) {
            state.rotating_orientation.set(false);
        }
    });
    // SAFETY: the transition coordinator's method, which copies the blocks.
    unsafe {
        let _: Bool = msg_send![coordinator,
            animateAlongsideTransition: along.as_ptr()
            completion: completion.as_ptr()];
    }
}

/// Translation of `-[SDL_uikitviewcontroller deinitKeyboard]`.
fn deinit_keyboard(this: Id) {
    // SAFETY: the default notification center.
    unsafe {
        let center: Id = msg_send![class!(NSNotificationCenter), defaultCenter];
        let forget = |name: Id| {
            let () = msg_send![center, removeObserver: this name: name object: NIL];
        };
        #[cfg(not(target_os = "tvos"))]
        {
            forget(UIKeyboardWillShowNotification);
            forget(UIKeyboardDidShowNotification);
            forget(UIKeyboardWillHideNotification);
            forget(UIKeyboardDidHideNotification);
        }
        forget(UITextFieldTextDidChangeNotification);
    }
}

/// Make the text input properties the text field's. Translation of
/// `-[SDL_uikitviewcontroller setTextFieldProperties:]`.
pub(crate) fn set_text_field_properties(this: Id, props: Option<&Properties>) {
    let Some(state) = state(this) else {
        return;
    };
    let text_field = state.text_field.get();
    let ty = text_input_type(props);
    // SAFETY: UITextField's properties (UITextInputTraits) and UIView's;
    // UIKit's constant strings.
    unsafe {
        let () = msg_send![text_field, setSecureTextEntry: NO];

        let (keyboard_type, content_type) = match ty {
            TextInputType::Text => (UIKeyboardTypeDefault, NIL),
            TextInputType::TextName => (UIKeyboardTypeDefault, UITextContentTypeName),
            TextInputType::TextEmail => (UIKeyboardTypeEmailAddress, UITextContentTypeEmailAddress),
            TextInputType::TextUsername => (UIKeyboardTypeDefault, UITextContentTypeUsername),
            TextInputType::TextPasswordHidden | TextInputType::TextPasswordVisible => {
                (UIKeyboardTypeDefault, UITextContentTypePassword)
            }
            TextInputType::Number => (UIKeyboardTypeDecimalPad, NIL),
            TextInputType::NumberPasswordHidden | TextInputType::NumberPasswordVisible => {
                let content_type = if available!(ios(12, 0), tvos(12, 0)) {
                    UITextContentTypeOneTimeCode()
                } else {
                    NIL
                };
                (UIKeyboardTypeNumberPad, content_type)
            }
        };
        let () = msg_send![text_field, setKeyboardType: keyboard_type];
        let () = msg_send![text_field, setTextContentType: content_type];
        if matches!(
            ty,
            TextInputType::TextPasswordHidden | TextInputType::NumberPasswordHidden
        ) {
            let () = msg_send![text_field, setSecureTextEntry: YES];
        }

        let capitalization = match text_input_capitalization(props) {
            Capitalization::None => UITextAutocapitalizationTypeNone,
            Capitalization::Letters => UITextAutocapitalizationTypeAllCharacters,
            Capitalization::Words => UITextAutocapitalizationTypeWords,
            Capitalization::Sentences => UITextAutocapitalizationTypeSentences,
        };
        let () = msg_send![text_field, setAutocapitalizationType: capitalization];

        if text_input_autocorrect(props) {
            let () = msg_send![text_field, setAutocorrectionType: UITextAutocorrectionTypeYes];
            let () = msg_send![text_field, setSpellCheckingType: UITextSpellCheckingTypeYes];
        } else {
            let () = msg_send![text_field, setAutocorrectionType: UITextAutocorrectionTypeNo];
            let () = msg_send![text_field, setSpellCheckingType: UITextSpellCheckingTypeNo];
        }

        if text_input_multiline(props) {
            let () = msg_send![text_field, setEnablesReturnKeyAutomatically: YES];
        } else {
            let () = msg_send![text_field, setEnablesReturnKeyAutomatically: NO];
        }

        let window: Id = msg_send![text_field, window];
        if window.is_null() {
            /* textField has not been added to the view yet,
            we don't have to do anything. */
            return;
        }

        // the text field needs to be re-added to the view in order to update correctly.
        // (the superview kept while the text field is out of it)
        let superview = StrongPtr::retain(msg_send![text_field, superview]);
        let () = msg_send![text_field, removeFromSuperview];
        let () = msg_send![superview.get(), addSubview: text_field];

        if window_of(this).is_some_and(crate::events::keyboard::text_input_active) {
            let _: Bool = msg_send![text_field, becomeFirstResponder];
        }
    }

    state.is_otp_mode.set(matches!(
        ty,
        TextInputType::NumberPasswordHidden | TextInputType::NumberPasswordVisible
    ));
}

/// Make the text field the first responder and accept text input (which
/// also shows the on-screen keyboard if no hardware keyboard is
/// attached): whether it is. Translation of
/// `-[SDL_uikitviewcontroller startTextInput]`.
pub(crate) fn start_text_input(this: Id) -> bool {
    let Some(state) = state(this) else {
        return false;
    };
    if !state.text_field_focused.get() {
        state.text_field_focused.set(true);
        send_screen_keyboard_shown();
    }

    let text_field = state.text_field.get();
    // SAFETY: UIView's, UITextField's and UIResponder's properties and
    // methods; NSString's.
    unsafe {
        let window: Id = msg_send![text_field, window];
        if window.is_null() {
            /* textField has not been added to the view yet,
             * we will try again when that happens. */
            return true;
        }

        if state.is_otp_mode.get() {
            let text: Id = msg_send![text_field, text];
            let empty = ns_string("");
            let space = ns_string(" ");
            let padding: Id = msg_send![empty.get(),
                stringByPaddingToLength: 64 as NSUInteger
                withString: space.get()
                startingAtIndex: 0 as NSUInteger];
            if length(text) == 64 {
                let equal: Bool = msg_send![text, isEqualToString: padding];
                if equal.is_true() {
                    let () = msg_send![text_field, setText: empty.get()];
                    *state.committed_text.borrow_mut() = empty;
                }
            }
        }
        let first_responder: Bool = msg_send![text_field, becomeFirstResponder];
        first_responder.is_true()
    }
}

/// Let the text field lose focus and stop accepting text input (which
/// also hides the on-screen keyboard if no hardware keyboard is
/// attached): whether it did. Translation of
/// `-[SDL_uikitviewcontroller stopTextInput]`.
pub(crate) fn stop_text_input(this: Id) -> bool {
    let Some(state) = state(this) else {
        return false;
    };
    if state.text_field_focused.get() {
        state.text_field_focused.set(false);
        send_screen_keyboard_hidden();
    }

    let text_field = state.text_field.get();
    // SAFETY: UIView's property.
    let window: Id = unsafe { msg_send![text_field, window] };
    if window.is_null() {
        /* textField has not been added to the view yet,
         * we will try again when that happens. */
        return true;
    }

    reset_text_state(this);
    // SAFETY: UIResponder's method.
    let resigned: Bool = unsafe { msg_send![text_field, resignFirstResponder] };
    resigned.is_true()
}

/// Translation of `-[SDL_uikitviewcontroller keyboardWillShow:]`.
extern "C" fn vc_keyboard_will_show(this: Id, _sel: Sel, notification: Id) {
    #[cfg(not(target_os = "tvos"))]
    {
        // SAFETY: NSNotification's user info (a dictionary), whose frame
        // is an NSValue of a CGRect; UIView's method.
        let height = unsafe {
            let info: Id = msg_send![notification, userInfo];
            let value: Id = msg_send![info, objectForKey: UIKeyboardFrameEndUserInfoKey];
            // ([nil CGRectValue] is a zero rectangle)
            let kbrect: CGRect = if value.is_null() {
                CGRect::new(0.0, 0.0, 0.0, 0.0)
            } else {
                msg_send![value, CGRectValue]
            };

            /* The keyboard rect is in the coordinate space of the screen/window, but we
             * want its height in the coordinate space of the view. */
            let view: Id = msg_send![this, view];
            let kbrect: CGRect = if view.is_null() {
                // ([nil convertRect:fromView:] is a zero rectangle)
                CGRect::new(0.0, 0.0, 0.0, 0.0)
            } else {
                msg_send![view, convertRect: kbrect fromView: NIL]
            };
            kbrect.size.height as i32
        };

        set_keyboard_height(this, height);
    }
    #[cfg(target_os = "tvos")]
    let _ = (this, notification);
}

/// Translation of `-[SDL_uikitviewcontroller keyboardDidShow:]`.
extern "C" fn vc_keyboard_did_show(_this: Id, _sel: Sel, _notification: Id) {
    send_screen_keyboard_shown();
}

/// Translation of `-[SDL_uikitviewcontroller keyboardWillHide:]`.
extern "C" fn vc_keyboard_will_hide(this: Id, _sel: Sel, _notification: Id) {
    set_keyboard_height(this, 0);
}

/// Translation of `-[SDL_uikitviewcontroller keyboardDidHide:]`.
extern "C" fn vc_keyboard_did_hide(_this: Id, _sel: Sel, _notification: Id) {
    send_screen_keyboard_hidden();
}

/// Translation of `-[SDL_uikitviewcontroller textFieldTextDidChange:]`.
extern "C" fn vc_text_field_text_did_change(this: Id, _sel: Sel, _notification: Id) {
    let Some(state) = state(this) else {
        return;
    };
    let text_field = state.text_field.get();
    // SAFETY: UITextField's properties, and NSString's methods (with
    // indices below the strings' lengths).
    unsafe {
        let marked: Id = msg_send![text_field, markedTextRange];
        if !marked.is_null() {
            return;
        }
        // (both kept: the events sent below may change the text field's
        // text and the committed one)
        let text = StrongPtr::retain(msg_send![text_field, text]);
        let committed = state.committed_text.borrow().clone();
        let text_length = length(text.get());
        let committed_length = length(committed.get());

        if state.is_otp_mode.get()
            && (text_length as NSInteger - committed_length as NSInteger).abs() != 1
        {
            return;
        }

        let compare_length = text_length.min(committed_length);
        let mut match_length: NSUInteger = 0;

        // Backspace over characters that are no longer in the string
        while match_length < compare_length {
            let a: u16 = msg_send![committed.get(), characterAtIndex: match_length];
            let b: u16 = msg_send![text.get(), characterAtIndex: match_length];
            if a != b {
                break;
            }
            match_length += 1;
        }
        if match_length < committed_length && !has_keyboard() {
            let gone: Id = msg_send![committed.get(), substringFromIndex: match_length];
            // Note: upstream counts a string that has no UTF-8 (a lone
            // surrogate's) from a NULL pointer; here there's nothing to
            // delete then.
            let mut delete_length =
                utf8_bytes(gone).map_or(0, |b| crate::stdlib::string::utf8strlen(&b));
            while delete_length > 0 {
                // Send distinct down and up events for each backspace action
                send_keyboard_key(
                    Duration::ZERO,
                    GLOBAL_KEYBOARD_ID,
                    0,
                    Scancode::BACKSPACE,
                    true,
                );
                send_keyboard_key(
                    Duration::ZERO,
                    GLOBAL_KEYBOARD_ID,
                    0,
                    Scancode::BACKSPACE,
                    false,
                );
                delete_length -= 1;
            }
        }

        if match_length < text_length {
            let pending: Id = msg_send![text.get(), substringFromIndex: match_length];
            if !hardware_keyboard_key_pressed() {
                /* Go through all the characters in the string we've been sent and
                 * convert them to key presses */
                for i in 0..length(pending) {
                    let ch: u16 = msg_send![pending, characterAtIndex: i];
                    send_keyboard_unicode_code(Duration::ZERO, ch as u32);
                }
            }
            // (no UTF-8 is no text, as for upstream's SDL_SendKeyboardText())
            if let Some(bytes) = utf8_bytes(pending) {
                send_keyboard_text(&String::from_utf8_lossy(&bytes));
            }
        }
        *state.committed_text.borrow_mut() = text;
    }
}

/// Move the view up when the on-screen keyboard would hide the text input
/// area (or back). Translation of `-[SDL_uikitviewcontroller updateKeyboard]`.
pub(crate) fn update_keyboard(this: Id) {
    let Some(state) = state(this) else {
        return;
    };
    // SAFETY: UIViewController's view, UIView's properties, and UIWindow's.
    unsafe {
        let view: Id = msg_send![this, view];
        if view.is_null() {
            // (upstream's messages to nil do nothing)
            return;
        }
        let mut t: CGAffineTransform = msg_send![view, transform];
        let mut offset = CGPoint::new(0.0, 0.0);
        #[cfg(target_os = "visionos")]
        let mut frame = super::compute_view_frame(state.window);
        #[cfg(not(target_os = "visionos"))]
        let mut frame = {
            let uiwindow =
                super::window::window_data(state.window).map_or(NIL, |data| data.uiwindow());
            let screen: Id = msg_send![uiwindow, screen];
            super::compute_view_frame(state.window, screen)
        };

        let rect = state.text_input_rect.get();
        let keyboard_height = state.keyboard_height.get();
        if keyboard_height != 0 && rect.h != 0 {
            let rectbottom = rect.y.wrapping_add(rect.h);
            let bounds: CGRect = msg_send![view, bounds];
            let keybottom = (bounds.size.height - keyboard_height as CGFloat) as i32;
            if keybottom < rectbottom {
                offset.y = keybottom.wrapping_sub(rectbottom) as CGFloat;
            }
        }

        /* Apply this view's transform (except any translation) to the offset, in
         * order to orient it correctly relative to the frame's coordinate space. */
        t.tx = 0.0;
        t.ty = 0.0;
        offset = CGPointApplyAffineTransform(offset, t);

        // Apply the updated offset to the view's frame.
        frame.origin.x += offset.x;
        frame.origin.y += offset.y;

        let () = msg_send![view, setFrame: frame];
    }
}

/// Translation of `-[SDL_uikitviewcontroller setKeyboardHeight:]`.
fn set_keyboard_height(this: Id, height: i32) {
    let Some(state) = state(this) else {
        return;
    };
    state.keyboard_height.set(height);
    update_keyboard(this);
}

/// UITextFieldDelegate method.  Invoked when user types something.
/// Translation of
/// `-[SDL_uikitviewcontroller textField:shouldChangeCharactersInRange:replacementString:]`.
extern "C" fn vc_should_change_characters(
    this: Id,
    _sel: Sel,
    _text_field: Id,
    _range: NSRange,
    string: Id,
) -> Bool {
    let Some(state) = state(this) else {
        return YES;
    };
    if !state.is_otp_mode.get() {
        let text_field = state.text_field.get();
        // SAFETY: UITextField's properties and NSString's.
        let reset = unsafe {
            let marked: Id = msg_send![text_field, markedTextRange];
            let text: Id = msg_send![text_field, text];
            marked.is_null() && length(string) == 0 && length(text) < 16
        };
        if reset {
            reset_text_state(this);
        }
    }
    YES
}

/// Terminates the editing session. Translation of
/// `-[SDL_uikitviewcontroller textFieldShouldReturn:]`.
extern "C" fn vc_text_field_should_return(this: Id, _sel: Sel, _text_field: Id) -> Bool {
    send_keyboard_key_auto_release(Duration::ZERO, Scancode::RETURN);
    if state(this).is_some_and(|s| s.text_field_focused.get())
        && crate::hints::get_bool(crate::hints::RETURN_KEY_HIDES_IME, false)
    {
        if let Some(window) = window_of(this).and_then(|w| crate::video::Window::from_id(w).ok()) {
            let _ = window.stop_text_input();
        }
    }
    YES
}

/// Translation of `-[SDL_uikitviewcontroller resetTextState]`.
fn reset_text_state(this: Id) {
    let Some(state) = state(this) else {
        return;
    };
    if !state.is_otp_mode.get() {
        let text_field = state.text_field.get();
        // SAFETY: UITextField's property.
        unsafe {
            let () = msg_send![text_field, setText: state.obligate_for_backspace.get()];
            let text: Id = msg_send![text_field, text];
            *state.committed_text.borrow_mut() = StrongPtr::retain(text);
        }
    }
}

// iPhone keyboard addition functions

/// The view controller of a window. Translation of
/// `GetWindowViewController()`.
fn get_window_view_controller(window: WindowID) -> Result<Id> {
    super::window::window_data(window)
        .map(|data| data.viewcontroller())
        .ok_or_else(|| Error::new("Invalid window"))
}

/// Translation of `UIKit_HasScreenKeyboardSupport()`.
pub(crate) fn has_screen_keyboard_support() -> bool {
    true
}

/// Translation of `UIKit_StartTextInput()`.
pub(crate) fn uikit_start_text_input(window: WindowID, _props: Option<&Properties>) -> Result<()> {
    autoreleasepool(|| {
        let vc = get_window_view_controller(window)?;
        if start_text_input(vc) {
            Ok(())
        } else {
            // Note: upstream fails without an error of its own.
            Err(Error::new(
                "The text field couldn't become the first responder",
            ))
        }
    })
}

/// Translation of `UIKit_StopTextInput()`.
pub(crate) fn uikit_stop_text_input(window: WindowID) -> Result<()> {
    autoreleasepool(|| {
        let vc = get_window_view_controller(window)?;
        if stop_text_input(vc) {
            Ok(())
        } else {
            // Note: as above.
            Err(Error::new(
                "The text field couldn't resign being the first responder",
            ))
        }
    })
}

/// Translation of `UIKit_SetTextInputProperties()`.
pub(crate) fn uikit_set_text_input_properties(window: WindowID, props: Option<&Properties>) {
    autoreleasepool(|| {
        if let Ok(vc) = get_window_view_controller(window) {
            set_text_field_properties(vc, props);
        }
    })
}

/// Translation of `UIKit_UpdateTextInputArea()`.
pub(crate) fn uikit_update_text_input_area(window: WindowID) -> Result<()> {
    autoreleasepool(|| {
        if let Ok(vc) = get_window_view_controller(window) {
            if let Some(state) = state(vc) {
                let rect = with_window(window, |w| w.text_input_rect).unwrap_or_default();
                state.text_input_rect.set(rect);

                if state.text_field_focused.get() {
                    update_keyboard(vc);
                }
            }
        }
    });
    Ok(())
}
