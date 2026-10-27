// Rust translation of src/main/ios/SDL_sysmain_callbacks.m from Simple
// DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The main callbacks on UIKit (iOS, tvOS, visionOS): a `CADisplayLink`
//! on the main run loop calls the iterations, once per frame of the
//! display, after the app's main function (which [`run_app`] runs from
//! `UIApplicationMain()`) has returned to that run loop. When an iteration
//! (or an event) ends the app, SDL quits and the process exits, with 1
//! for [`AppResult::Failure`] and 0 otherwise.
//!
//! [`run_app`]: super::run_app

use std::io::Write;
use std::sync::Mutex;

use super::{
    init_main_callbacks, iterate_main_callbacks, quit_main_callbacks, AppCallbacks, AppResult,
};
use crate::core::apple::foundation::available;
use crate::core::apple::objc::{
    class, imp, ivar_ptr, msg_send, sel, Class, ClassBuilder, DeclaredClass, Id, Sel, StrongPtr,
    NIL,
};
use crate::video::drivers::uikit::events::update_lifecycle_observer;
use crate::video::drivers::uikit::sys::{CAFrameRateRangeMake, NSDefaultRunLoopMode};

/// Translation of `globalDisplayLink`.
static GLOBAL_DISPLAY_LINK: Mutex<Option<StrongPtr>> = Mutex::new(None);

static SDL_IOS_MAIN_CALLBACKS_DISPLAY_LINK: DeclaredClass = DeclaredClass::new();

/// The `SDLIosMainCallbacksDisplayLink` class (`@interface
/// SDLIosMainCallbacksDisplayLink : NSObject`), with its `displayLink`
/// (retained) property as an instance variable.
fn display_link_class() -> Class {
    SDL_IOS_MAIN_CALLBACKS_DISPLAY_LINK.get(|| {
        let mut b = ClassBuilder::new(c"SDLIosMainCallbacksDisplayLink", class!(NSObject))
            .expect("SDLIosMainCallbacksDisplayLink");
        b.add_ivar::<Id>(c"displayLink", c"@");
        // SAFETY: the implementation has the signature of the encoding.
        unsafe {
            b.add_method(
                sel!(appIteration:),
                imp!(app_iteration as extern "C" fn(Id, Sel, Id)),
                c"v@:@",
            );
        }
        b.register()
    })
}

/// `self.displayLink`'s variable (the display link it owns, or nil).
fn display_link_ivar(this: Id) -> *mut Id {
    // SAFETY: an SDLIosMainCallbacksDisplayLink, which has the variable.
    unsafe { ivar_ptr::<Id>(this, display_link_class(), c"displayLink") }
}

/// Translation of `-[SDLIosMainCallbacksDisplayLink init:quitfunc:]`
/// (owned; the callbacks are the main callbacks' own state here).
fn new_display_link() -> StrongPtr {
    // SAFETY: NSObject's alloc and init (owned); CADisplayLink's class
    // method (autoreleased, retained into the variable) targeting this
    // object's appIteration:, its properties, and the current run loop.
    unsafe {
        let this: Id = msg_send![display_link_class(), alloc];
        let this = StrongPtr::from_owned(msg_send![this, init]);
        if this.is_nil() {
            return this;
        }
        let display_link: Id = msg_send![class!(CADisplayLink),
            displayLinkWithTarget: this.get()
            selector: sel!(appIteration:)];
        *display_link_ivar(this.get()) = crate::core::apple::objc::retain(display_link);
        // Enable high refresh rates on iOS
        // To enable this on phones, you should add the following line to Info.plist:
        // <key>CADisableMinimumFrameDurationOnPhone</key> <true/>
        // If main callbacks are used then this CADisplayLink will affect framerate, not one in SDL_uikitviewcontroller.
        if available!(ios(15, 0), tvos(15, 0)) {
            let mode = crate::video::primary_display()
                .and_then(crate::video::desktop_display_mode)
                .ok();
            if let Some(mode) = mode.filter(|mode| mode.refresh_rate > 60.0) {
                let frame_rate = mode.refresh_rate as i32;
                let () = msg_send![display_link, setPreferredFrameRateRange: CAFrameRateRangeMake(
                    ((frame_rate * 2) / 3) as f32,
                    frame_rate as f32,
                    frame_rate as f32,
                )];
            }
        }
        let run_loop: Id = msg_send![class!(NSRunLoop), currentRunLoop];
        let () = msg_send![display_link, addToRunLoop: run_loop forMode: NSDefaultRunLoopMode];
        this
    }
}

/// `exit((rc == SDL_APP_FAILURE) ? 1 : 0)`, with what was printed flushed.
fn exit_with(rc: AppResult) -> ! {
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().flush();
    std::process::exit(if rc == AppResult::Failure { 1 } else { 0 })
}

/// Translation of `-[SDLIosMainCallbacksDisplayLink appIteration:]`.
extern "C" fn app_iteration(this: Id, _sel: Sel, _sender: Id) {
    let rc = iterate_main_callbacks(true);
    if rc != AppResult::Continue {
        // SAFETY: the display link this object owns, invalidated (it lets
        // go of its target) then released.
        unsafe {
            let slot = display_link_ivar(this);
            let display_link = std::mem::replace(&mut *slot, NIL);
            let () = msg_send![display_link, invalidate];
            crate::core::apple::objc::release(display_link);
        }
        // (`this` is gone after this)
        let global = GLOBAL_DISPLAY_LINK
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        drop(global);
        quit_main_callbacks(rc);
        update_lifecycle_observer();
        exit_with(rc);
    }
}

/// Translation of `SDL_EnterAppMainCallbacks()` (UIKit): set up the app,
/// and the display link that iterates it once the caller (the app's main
/// function) returns to the run loop. Returns 0 then; if `appinit` ended
/// the app (or the display link can't be made), SDL quits and the process
/// exits at once.
///
/// [`run_app`](super::run_app) lands in `UIApplicationMain()`, which calls
/// the main function from `postFinishLaunch`, which calls this. When we
/// return from here, we're living in the run loop, and a `CADisplayLink`
/// is firing regularly for us.
pub(super) fn enter_app_main_callbacks<A: AppCallbacks>(
    args: &[String],
    appinit: impl FnOnce(&[String]) -> (A, AppResult),
) -> i32 {
    let mut rc = init_main_callbacks(args, appinit);
    if rc == AppResult::Continue {
        let display_link = new_display_link();
        if display_link.is_nil() {
            rc = AppResult::Failure;
        } else {
            *GLOBAL_DISPLAY_LINK
                .lock()
                .unwrap_or_else(|e| e.into_inner()) = Some(display_link);
            return 0; // this will fall all the way out of SDL_main, where UIApplicationMain will keep running the RunLoop.
        }
    }

    // appinit requested quit, just bounce out now.
    quit_main_callbacks(rc);
    exit_with(rc);
}
