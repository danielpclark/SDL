// Rust translation of src/video/cocoa/SDL_cocoaevents.m and
// SDL_cocoaevents.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The application: `SDL3Application` (the `NSApplication` subclass that
//! dispatches events to SDL), `SDL3AppDelegate` (activation, open-file and
//! URL events, theme and locale changes), the default menus, and the event
//! pump.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::sys::*;
use super::{keyboard, mouse, window, VideoData};
use crate::core::apple::foundation::{
    available, cf_string, ns_number_bool, ns_string, NSPoint, NSTimeInterval,
};
use crate::core::apple::objc::{
    autoreleasepool, class, encoding, imp, msg_send, msg_send_super, sel, Bool, Class,
    ClassBuilder, DeclaredClass, Id, NSInteger, NSUInteger, Sel, StrongPtr, NIL, NO, YES,
};
use crate::error::Result;
use crate::events::window::{send_drop_complete, send_drop_file};
use crate::events::WindowID;
use crate::hints;

/// Translation of `FindSDLWindowForNSWindow()`.
fn find_sdl_window_for_nswindow(win: Id) -> Option<WindowID> {
    window::find_sdl_window(win)
}

/// Translation of `s_bShouldHandleEventsInSDLApplication`.
static S_SHOULD_HANDLE_EVENTS_IN_SDL_APPLICATION: AtomicBool = AtomicBool::new(false);

/// Translation of `Cocoa_DispatchEvent()`.
fn dispatch_event(the_event: Id) {
    // SAFETY: an NSEvent.
    let ty: NSUInteger = unsafe { msg_send![the_event, type] };
    match ty {
        NSEventTypeLeftMouseDown
        | NSEventTypeOtherMouseDown
        | NSEventTypeRightMouseDown
        | NSEventTypeLeftMouseUp
        | NSEventTypeOtherMouseUp
        | NSEventTypeRightMouseUp
        | NSEventTypeLeftMouseDragged
        | NSEventTypeRightMouseDragged
        | NSEventTypeOtherMouseDragged // usually middle mouse dragged
        | NSEventTypeMouseMoved
        | NSEventTypeScrollWheel
        | NSEventTypeMouseEntered
        | NSEventTypeMouseExited => {
            mouse::handle_mouse_event(the_event);
        }
        NSEventTypeKeyDown | NSEventTypeKeyUp | NSEventTypeFlagsChanged => {
            keyboard::handle_key_event(the_event);
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// SDL3Application

static SDL3_APPLICATION: DeclaredClass = DeclaredClass::new();

/// The `SDL3Application` class (`@interface SDL3Application : NSApplication`).
fn sdl3_application_class() -> Class {
    SDL3_APPLICATION.get(|| {
        let mut b =
            ClassBuilder::new(c"SDL3Application", class!(NSApplication)).expect("SDL3Application");
        // SAFETY: the implementations have the signatures of the encodings.
        unsafe {
            b.add_method(
                sel!(terminate:),
                imp!(app_terminate as extern "C" fn(Id, Sel, Id)),
                c"v@:@",
            );
            b.add_method(
                sel!(sendEvent:),
                imp!(app_send_event as extern "C" fn(Id, Sel, Id)),
                c"v@:@",
            );
            b.add_class_method(
                sel!(registerUserDefaults),
                imp!(app_register_user_defaults as extern "C" fn(Id, Sel)),
                c"v@:",
            );
        }
        b.register()
    })
}

/// Override terminate to handle Quit and System Shutdown smoothly.
/// Translation of `-[SDL3Application terminate:]`.
extern "C" fn app_terminate(_this: Id, _sel: Sel, _sender: Id) {
    crate::events::send_quit();
}

/// Dispatch events here so that we can handle events caught by
/// nextEventMatchingMask in SDL, as well as events caught by other
/// processes (such as CEF) that are passed down to NSApp.
/// Translation of `-[SDL3Application sendEvent:]`.
extern "C" fn app_send_event(this: Id, _sel: Sel, the_event: Id) {
    if S_SHOULD_HANDLE_EVENTS_IN_SDL_APPLICATION.load(Ordering::Relaxed) {
        dispatch_event(the_event);
    }

    // SAFETY: NSApplication's own implementation.
    let _: () = unsafe { msg_send_super![this, sdl3_application_class(), sendEvent: the_event] };
}

/// Translation of `+[SDL3Application registerUserDefaults]`.
extern "C" fn app_register_user_defaults(_cls: Id, _sel: Sel) {
    register_user_defaults();
}

/// Translation of `+[SDL3Application registerUserDefaults]`.
fn register_user_defaults() {
    let momentum_scroll_supported = hints::get_bool(hints::MAC_SCROLL_MOMENTUM, false);
    let press_and_hold_enabled = hints::get_bool(hints::MAC_PRESS_AND_HOLD, true);

    // SAFETY: NSDictionary and NSUserDefaults as documented.
    unsafe {
        let keys = [
            ns_string("AppleMomentumScrollSupported"),
            ns_string("ApplePressAndHoldEnabled"),
            ns_string("ApplePersistenceIgnoreState"),
        ];
        let values = [
            ns_number_bool(momentum_scroll_supported),
            ns_number_bool(press_and_hold_enabled),
            ns_number_bool(true),
        ];
        let key_ids: Vec<Id> = keys.iter().map(|k| k.get()).collect();
        let app_defaults: Id = msg_send![class!(NSDictionary),
            dictionaryWithObjects: values.as_ptr()
            forKeys: key_ids.as_ptr()
            count: values.len() as NSUInteger];
        let defaults: Id = msg_send![class!(NSUserDefaults), standardUserDefaults];
        let _: () = msg_send![defaults, registerDefaults: app_defaults];
    }
}

// ---------------------------------------------------------------------------
// SDL3AppDelegate

static SDL3_APP_DELEGATE: DeclaredClass = DeclaredClass::new();

/// The `SDL3AppDelegate` class (`@interface SDL3AppDelegate : NSObject
/// <NSApplicationDelegate>`), with its `seenFirstActivate` instance
/// variable.
fn sdl3_app_delegate_class() -> Class {
    SDL3_APP_DELEGATE.get(|| {
        let mut b =
            ClassBuilder::new(c"SDL3AppDelegate", class!(NSObject)).expect("SDL3AppDelegate");
        b.add_protocol(c"NSApplicationDelegate");
        b.add_ivar::<Bool>(c"seenFirstActivate", &encoding("B"));
        // SAFETY: the implementations have the signatures of the encodings.
        unsafe {
            b.add_method(
                sel!(init),
                imp!(delegate_init as extern "C" fn(Id, Sel) -> Id),
                c"@@:",
            );
            b.add_method(
                sel!(dealloc),
                imp!(delegate_dealloc as extern "C" fn(Id, Sel)),
                c"v@:",
            );
            b.add_method(
                sel!(windowWillClose:),
                imp!(delegate_window_will_close as extern "C" fn(Id, Sel, Id)),
                c"v@:@",
            );
            b.add_method(
                sel!(focusSomeWindow:),
                imp!(delegate_focus_some_window as extern "C" fn(Id, Sel, Id)),
                c"v@:@",
            );
            b.add_method(
                sel!(screenParametersChanged:),
                imp!(delegate_screen_parameters_changed as extern "C" fn(Id, Sel, Id)),
                c"v@:@",
            );
            b.add_method(
                sel!(localeDidChange:),
                imp!(delegate_locale_did_change as extern "C" fn(Id, Sel, Id)),
                c"v@:@",
            );
            b.add_method(
                sel!(observeValueForKeyPath:ofObject:change:context:),
                imp!(delegate_observe_value as extern "C" fn(Id, Sel, Id, Id, Id, *mut c_void)),
                c"v@:@@@^v",
            );
            b.add_method(
                sel!(application:openFile:),
                imp!(delegate_open_file as extern "C" fn(Id, Sel, Id, Id) -> Bool),
                &encoding("B@:@@"),
            );
            b.add_method(
                sel!(applicationDidFinishLaunching:),
                imp!(delegate_did_finish_launching as extern "C" fn(Id, Sel, Id)),
                c"v@:@",
            );
            b.add_method(
                sel!(handleURLEvent:withReplyEvent:),
                imp!(delegate_handle_url_event as extern "C" fn(Id, Sel, Id, Id)),
                c"v@:@@",
            );
            b.add_method(
                sel!(applicationSupportsSecureRestorableState:),
                imp!(
                    delegate_supports_secure_restorable_state as extern "C" fn(Id, Sel, Id) -> Bool
                ),
                &encoding("B@:@"),
            );
            b.add_method(
                sel!(menu:),
                imp!(delegate_menu as extern "C" fn(Id, Sel, Id)),
                c"v@:@",
            );
        }
        b.register()
    })
}

/// The `seenFirstActivate` instance variable of an `SDL3AppDelegate`.
fn seen_first_activate(this: Id) -> *mut Bool {
    // SAFETY: an SDL3AppDelegate, which has the variable.
    unsafe {
        crate::core::apple::objc::ivar_ptr::<Bool>(
            this,
            sdl3_app_delegate_class(),
            c"seenFirstActivate",
        )
    }
}

/// Add `this` to the default notification center for `name`.
///
/// # Safety
///
/// `name` must be an `NSString` and `selector` a method of `this` taking a
/// notification.
unsafe fn observe(this: Id, selector: Sel, name: Id, object: Id) {
    // SAFETY: the caller's contract.
    unsafe {
        let center: Id = msg_send![class!(NSNotificationCenter), defaultCenter];
        let _: () =
            msg_send![center, addObserver: this selector: selector name: name object: object];
    }
}

/// Translation of `-[SDL3AppDelegate init]`.
extern "C" fn delegate_init(this: Id, _sel: Sel) -> Id {
    // SAFETY: NSObject's init, then notification registrations of this
    // object's methods.
    unsafe {
        let this: Id = msg_send_super![this, sdl3_app_delegate_class(), init];
        if !this.is_null() {
            let register_activation_handlers =
                hints::get_bool("SDL_MAC_REGISTER_ACTIVATION_HANDLERS", true);

            *seen_first_activate(this) = NO;

            if register_activation_handlers {
                observe(
                    this,
                    sel!(windowWillClose:),
                    NSWindowWillCloseNotification,
                    NIL,
                );
                observe(
                    this,
                    sel!(focusSomeWindow:),
                    NSApplicationDidBecomeActiveNotification,
                    NIL,
                );
                observe(
                    this,
                    sel!(screenParametersChanged:),
                    NSApplicationDidChangeScreenParametersNotification,
                    NIL,
                );
            }

            observe(
                this,
                sel!(localeDidChange:),
                NSCurrentLocaleDidChangeNotification,
                NIL,
            );

            let app: Id = msg_send![class!(NSApplication), sharedApplication];
            let key = ns_string("effectiveAppearance");
            let _: () = msg_send![app, addObserver: this
                                          forKeyPath: key.get()
                                             options: NSKeyValueObservingOptionInitial
                                             context: std::ptr::null_mut::<c_void>()];
        }
        this
    }
}

/// Translation of `-[SDL3AppDelegate dealloc]`.
extern "C" fn delegate_dealloc(this: Id, _sel: Sel) {
    // SAFETY: undoes the registrations of init, then NSObject's dealloc.
    unsafe {
        let center: Id = msg_send![class!(NSNotificationCenter), defaultCenter];
        for name in [
            NSWindowWillCloseNotification,
            NSApplicationDidBecomeActiveNotification,
            NSApplicationDidChangeScreenParametersNotification,
            NSCurrentLocaleDidChangeNotification,
        ] {
            let _: () = msg_send![center, removeObserver: this name: name object: NIL];
        }
        let app: Id = msg_send![class!(NSApplication), sharedApplication];
        let key = ns_string("effectiveAppearance");
        let _: () = msg_send![app, removeObserver: this forKeyPath: key.get()];

        // Remove our URL event handler only if we set it
        let delegate: Id = msg_send![app, delegate];
        if delegate == this {
            let manager: Id = msg_send![class!(NSAppleEventManager), sharedAppleEventManager];
            let _: () = msg_send![manager, removeEventHandlerForEventClass: kInternetEventClass
                                                                andEventID: kAEGetURL];
        }

        let _: () = msg_send_super![this, sdl3_app_delegate_class(), dealloc];
    }
}

/// Translation of `-[SDL3AppDelegate windowWillClose:]`.
extern "C" fn delegate_window_will_close(this: Id, _sel: Sel, notification: Id) {
    // SAFETY: the notification of a closing NSWindow, and NSApp's windows.
    unsafe {
        let win: Id = msg_send![notification, object];

        let is_key: Bool = msg_send![win, isKeyWindow];
        if !is_key.is_true() {
            return;
        }

        // Don't do anything if this was not an SDL window that was closed
        if find_sdl_window_for_nswindow(win).is_none() {
            return;
        }

        /* HACK: Make the next window in the z-order key when the key window is
         * closed. The custom event loop and/or windowing code we have seems to
         * prevent the normal behavior: https://bugzilla.libsdl.org/show_bug.cgi?id=1825
         */

        /* +[NSApp orderedWindows] never includes the 'About' window, but we still
         * want to try its list first since the behavior in other apps is to only
         * make the 'About' window key if no other windows are on-screen.
         */
        let app: Id = msg_send![class!(NSApplication), sharedApplication];
        let ordered: Id = msg_send![app, orderedWindows];
        for window in crate::core::apple::foundation::ns_array_items(ordered) {
            let can: Bool = msg_send![window, canBecomeKeyWindow];
            if window != win && can.is_true() {
                let active: Bool = msg_send![window, isOnActiveSpace];
                if !active.is_true() {
                    continue;
                }
                let _: () = msg_send![window, makeKeyAndOrderFront: this];
                return;
            }
        }

        /* If a window wasn't found above, iterate through all visible windows in
         * the active Space in z-order (including the 'About' window, if it's shown)
         * and make the first one key.
         */
        let numbers: Id = msg_send![class!(NSWindow), windowNumbersWithOptions: 0 as NSUInteger];
        for num in crate::core::apple::foundation::ns_array_items(numbers) {
            let n: NSInteger = msg_send![num, integerValue];
            let window: Id = msg_send![app, windowWithWindowNumber: n];
            if !window.is_null() && window != win {
                let can: Bool = msg_send![window, canBecomeKeyWindow];
                if can.is_true() {
                    let _: () = msg_send![window, makeKeyAndOrderFront: this];
                    return;
                }
            }
        }
    }
}

/// Translation of `-[SDL3AppDelegate focusSomeWindow:]`.
extern "C" fn delegate_focus_some_window(this: Id, _sel: Sel, _notification: Id) {
    /* HACK: Ignore the first call. The application gets a
     * applicationDidBecomeActive: a little bit after the first window is
     * created, and if we don't ignore it, a window that has been created with
     * SDL_WINDOW_MINIMIZED will ~immediately be restored.
     */
    // SAFETY: the delegate's instance variable.
    unsafe {
        let seen = seen_first_activate(this);
        if !(*seen).is_true() {
            *seen = YES;
            return;
        }
    }

    /* Don't do anything if the application already has a key window
     * that is not an SDL window.
     */
    // SAFETY: NSApp's key window.
    let key_window: Id = unsafe {
        let app: Id = msg_send![class!(NSApplication), sharedApplication];
        msg_send![app, keyWindow]
    };
    if !key_window.is_null() && find_sdl_window_for_nswindow(key_window).is_none() {
        return;
    }

    // Restore any fullscreen window
    let fullscreen_window = crate::video::core::with_device(|v| {
        if v.windows.is_empty() {
            return None;
        }
        v.displays.iter().find_map(|d| d.fullscreen_window)
    })
    .ok()
    .flatten();
    if let Some(fullscreen_window) = fullscreen_window {
        let minimized = crate::video::core::with_window(fullscreen_window, |w| {
            w.flags()
                .contains(crate::events::window::WindowFlags::MINIMIZED)
        })
        .unwrap_or(false);
        if minimized {
            let _ = crate::video::window::Window::from_raw(fullscreen_window).restore();
        }
    }
}

/// Translation of `-[SDL3AppDelegate screenParametersChanged:]`.
extern "C" fn delegate_screen_parameters_changed(_this: Id, _sel: Sel, _notification: Id) {
    if crate::video::core::initialized() && super::is_current_driver() {
        super::modes::update_displays();
    }
}

/// Translation of `-[SDL3AppDelegate localeDidChange:]`.
extern "C" fn delegate_locale_did_change(_this: Id, _sel: Sel, _notification: Id) {
    crate::events::send_locale_changed_event();
}

/// Translation of `-[SDL3AppDelegate observeValueForKeyPath:ofObject:change:context:]`.
extern "C" fn delegate_observe_value(
    _this: Id,
    _sel: Sel,
    _key_path: Id,
    _object: Id,
    _change: Id,
    _context: *mut c_void,
) {
    if crate::video::core::initialized() && super::is_current_driver() {
        crate::video::core::set_system_theme(super::get_system_theme());
    }
}

/// Translation of `-[SDL3AppDelegate application:openFile:]`.
extern "C" fn delegate_open_file(_this: Id, _sel: Sel, _app: Id, filename: Id) -> Bool {
    // SAFETY: an NSString.
    let filename = unsafe { super::ns_to_string(filename) };
    Bool::from(send_drop_file(None, None, &filename) && send_drop_complete(None))
}

/// Translation of `-[SDL3AppDelegate applicationDidFinishLaunching:]`.
extern "C" fn delegate_did_finish_launching(_this: Id, _sel: Sel, _notification: Id) {
    if !hints::get_bool("SDL_MAC_REGISTER_ACTIVATION_HANDLERS", true) {
        return;
    }

    /* The menu bar of SDL apps which don't have the typical .app bundle
     * structure fails to work the first time a window is created (until it's
     * de-focused and re-focused), if this call is in Cocoa_RegisterApp instead
     * of here. https://github.com/libsdl-org/SDL/issues/1913
     */

    /* this apparently became unnecessary on macOS 14.0, and will addition pop up a
    hidden dock if you're moving the mouse during launch, so change the default
    behaviour there.  https://github.com/libsdl-org/SDL/issues/10340
    (13.6 still needs it, presumably 13.7 does, too.) */
    let mut background_app_default = false;
    if available!(macos(14, 0)) {
        background_app_default = true; /* by default, don't explicitly activate the dock and then us again to force to foreground */
    }

    if !hints::get_bool(hints::MAC_BACKGROUND_APP, background_app_default) {
        // Get more aggressive for Catalina: activate the Dock first so we definitely reset all activation state.
        // SAFETY: NSRunningApplication and NSApp as documented.
        unsafe {
            let dock = ns_string("com.apple.dock");
            let apps: Id = msg_send![class!(NSRunningApplication),
                runningApplicationsWithBundleIdentifier: dock.get()];
            if let Some(&i) = crate::core::apple::foundation::ns_array_items(apps).first() {
                let _: Bool =
                    msg_send![i, activateWithOptions: NSApplicationActivateIgnoringOtherApps];
            }
        }
        crate::timer::delay(Duration::from_millis(300)); // !!! FIXME: this isn't right.
                                                         // SAFETY: NSApp.
        unsafe {
            let app: Id = msg_send![class!(NSApplication), sharedApplication];
            let _: () = msg_send![app, activateIgnoringOtherApps: YES];
        }
    }

    /* If we call this before NSApp activation, macOS might print a complaint
     * about ApplePersistenceIgnoreState. */
    register_user_defaults();
}

/// Translation of `-[SDL3AppDelegate handleURLEvent:withReplyEvent:]`.
extern "C" fn delegate_handle_url_event(_this: Id, _sel: Sel, event: Id, _reply_event: Id) {
    // SAFETY: an NSAppleEventDescriptor.
    let path = unsafe {
        let param: Id = msg_send![event, paramDescriptorForKeyword: keyDirectObject];
        let s: Id = msg_send![param, stringValue];
        super::ns_to_string(s)
    };
    send_drop_file(None, None, &path);
    send_drop_complete(None);
}

/// Translation of `-[SDL3AppDelegate applicationSupportsSecureRestorableState:]`.
extern "C" fn delegate_supports_secure_restorable_state(_this: Id, _sel: Sel, _app: Id) -> Bool {
    // This just tells Cocoa that we didn't do any custom save state magic for the app,
    // so the system is safe to use NSSecureCoding internally, instead of using unencrypted
    // save states for backwards compatibility. If we don't return YES here, we'll get a
    // warning on the console at startup:
    //
    // ```
    // WARNING: Secure coding is not enabled for restorable state! Enable secure coding by implementing NSApplicationDelegate.applicationSupportsSecureRestorableState: and returning YES.
    // ```
    //
    // More-detailed explanation:
    // https://stackoverflow.com/questions/77283578/sonoma-and-nsapplicationdelegate-applicationsupportssecurerestorablestate/77320845#77320845
    YES
}

/// What `-[SDL3AppDelegate menu:]` calls with the `pointerValue` of the
/// sender's `representedObject` (`SDL_ClickTrayEntry()`): set by the Cocoa
/// tray backend, which puts its entries in those menu items.
pub(crate) static TRAY_MENU_HANDLER: Mutex<Option<fn(*mut c_void)>> = Mutex::new(None);

/// Translation of `-[SDL3AppDelegate menu:]`.
extern "C" fn delegate_menu(_this: Id, _sel: Sel, sender: Id) {
    // SAFETY: an NSMenuItem whose represented object is an NSValue.
    let entry: *mut c_void = unsafe {
        let value: Id = msg_send![sender, representedObject];
        msg_send![value, pointerValue]
    };

    let handler = *TRAY_MENU_HANDLER.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(handler) = handler {
        handler(entry);
    }
}

/// Translation of `appDelegate`.
static APP_DELEGATE: AtomicPtr<crate::core::apple::objc::Object> =
    AtomicPtr::new(std::ptr::null_mut());

/// The name of the application, owned. Translation of `GetApplicationName()`.
pub(crate) fn get_application_name() -> StrongPtr {
    // SAFETY: NSBundle and NSProcessInfo as documented; the objects are
    // retained before the pool goes.
    unsafe {
        let mut app_name: StrongPtr = StrongPtr::nil();

        let props = crate::properties::Properties::global();
        if let Some(metaname) = props.get_string(crate::init::AppMetadata::Name.property_name()) {
            if !metaname.is_empty() {
                app_name = ns_string(&metaname);
            }
        }

        // Determine the application name
        if app_name.is_nil() {
            let bundle: Id = msg_send![class!(NSBundle), mainBundle];
            let key = ns_string("CFBundleDisplayName");
            let name: Id = msg_send![bundle, objectForInfoDictionaryKey: key.get()];
            app_name = StrongPtr::retain(name);
            if app_name.is_nil() {
                let key = ns_string("CFBundleName");
                let name: Id = msg_send![bundle, objectForInfoDictionaryKey: key.get()];
                app_name = StrongPtr::retain(name);
            }
        }

        let len: NSUInteger = if app_name.is_nil() {
            0
        } else {
            msg_send![app_name, length]
        };
        if len == 0 {
            let info: Id = msg_send![class!(NSProcessInfo), processInfo];
            let name: Id = msg_send![info, processName];
            app_name = StrongPtr::retain(name);
        }

        app_name
    }
}

/// Translation of `LoadMainMenuNibIfAvailable()`.
fn load_main_menu_nib_if_available() -> bool {
    // SAFETY: NSBundle as documented.
    unsafe {
        let mut success = false;

        let bundle: Id = msg_send![class!(NSBundle), mainBundle];
        let info_dict: Id = msg_send![bundle, infoDictionary];
        if !info_dict.is_null() {
            let key = ns_string("NSMainNibFile");
            let main_nib_file_name: Id = msg_send![info_dict, valueForKey: key.get()];

            if !main_nib_file_name.is_null() {
                let app: Id = msg_send![class!(NSApplication), sharedApplication];
                let r: Bool = msg_send![bundle, loadNibNamed: main_nib_file_name
                                                         owner: app
                                               topLevelObjects: std::ptr::null_mut::<Id>()];
                success = r.is_true();
            }
        }

        success
    }
}

/// `[menu addItemWithTitle:title action:action keyEquivalent:key]`.
///
/// # Safety
///
/// `menu` must be an `NSMenu`.
unsafe fn add_item(menu: Id, title: Id, action: Option<Sel>, key: &str) -> Id {
    let key = ns_string(key);
    let action = action.unwrap_or(Sel::NULL);
    // SAFETY: the caller's contract.
    unsafe { msg_send![menu, addItemWithTitle: title action: action keyEquivalent: key.get()] }
}

/// `[@"prefix" stringByAppendingString:name]`.
///
/// # Safety
///
/// `name` must be an `NSString`.
unsafe fn prefixed(prefix: &str, name: Id) -> Id {
    let prefix = ns_string(prefix);
    // SAFETY: the caller's contract.
    unsafe { msg_send![prefix, stringByAppendingString: name] }
}

/// Translation of `CreateApplicationMenus()`.
fn create_application_menus() {
    // SAFETY: NSMenu and NSMenuItem as documented; the menus are owned by
    // NSApp once set.
    unsafe {
        let app: Id = msg_send![class!(NSApplication), sharedApplication];
        if app.is_null() {
            return;
        }

        let main_menu = StrongPtr::from_owned(msg_send![super::alloc(class!(NSMenu)), init]);

        // Create the main menu bar
        let _: () = msg_send![app, setMainMenu: main_menu.get()];

        // Create the application menu
        let app_name = get_application_name();
        let empty = ns_string("");
        let apple_menu = StrongPtr::from_owned(msg_send![super::alloc(class!(NSMenu)),
            initWithTitle: empty.get()]);

        // Add menu items
        let title = prefixed("About ", app_name.get());

        // !!! FIXME: Menu items can't take parameters, just a basic selector, so this should instead call a selector
        // !!! FIXME: that itself calls -[NSApplication orderFrontStandardAboutPanelWithOptions:optionsDictionary],
        // !!! FIXME: filling in that NSDictionary with SDL_GetAppMetadataProperty()
        add_item(
            apple_menu.get(),
            title,
            Some(sel!(orderFrontStandardAboutPanel:)),
            "",
        );

        let separator: Id = msg_send![class!(NSMenuItem), separatorItem];
        let _: () = msg_send![apple_menu, addItem: separator];

        let preferences = ns_string("Preferences…");
        add_item(apple_menu.get(), preferences.get(), None, ",");

        let separator: Id = msg_send![class!(NSMenuItem), separatorItem];
        let _: () = msg_send![apple_menu, addItem: separator];

        let service_menu = StrongPtr::from_owned(msg_send![super::alloc(class!(NSMenu)),
            initWithTitle: empty.get()]);
        let services = ns_string("Services");
        let menu_item = add_item(apple_menu.get(), services.get(), None, "");
        let _: () = msg_send![menu_item, setSubmenu: service_menu.get()];

        let _: () = msg_send![app, setServicesMenu: service_menu.get()];

        let separator: Id = msg_send![class!(NSMenuItem), separatorItem];
        let _: () = msg_send![apple_menu, addItem: separator];

        let title = prefixed("Hide ", app_name.get());
        add_item(apple_menu.get(), title, Some(sel!(hide:)), "h");

        let hide_others = ns_string("Hide Others");
        let menu_item = add_item(
            apple_menu.get(),
            hide_others.get(),
            Some(sel!(hideOtherApplications:)),
            "h",
        );
        let _: () = msg_send![menu_item, setKeyEquivalentModifierMask:
            (NSEventModifierFlagOption | NSEventModifierFlagCommand)];

        let show_all = ns_string("Show All");
        add_item(
            apple_menu.get(),
            show_all.get(),
            Some(sel!(unhideAllApplications:)),
            "",
        );

        let separator: Id = msg_send![class!(NSMenuItem), separatorItem];
        let _: () = msg_send![apple_menu, addItem: separator];

        let title = prefixed("Quit ", app_name.get());
        add_item(apple_menu.get(), title, Some(sel!(terminate:)), "q");

        // Put menu into the menubar
        let menu_item = StrongPtr::from_owned(msg_send![super::alloc(class!(NSMenuItem)),
            initWithTitle: empty.get() action: Sel::NULL keyEquivalent: empty.get()]);
        let _: () = msg_send![menu_item, setSubmenu: apple_menu.get()];
        let main: Id = msg_send![app, mainMenu];
        let _: () = msg_send![main, addItem: menu_item.get()];

        // Tell the application object that this is now the application menu
        let _: () = msg_send![app, setAppleMenu: apple_menu.get()];

        // Create the window menu
        let window_title = ns_string("Window");
        let window_menu = StrongPtr::from_owned(msg_send![super::alloc(class!(NSMenu)),
            initWithTitle: window_title.get()]);

        // Add menu items
        let close = ns_string("Close");
        add_item(
            window_menu.get(),
            close.get(),
            Some(sel!(performClose:)),
            "w",
        );

        let minimize = ns_string("Minimize");
        add_item(
            window_menu.get(),
            minimize.get(),
            Some(sel!(performMiniaturize:)),
            "m",
        );

        let zoom = ns_string("Zoom");
        add_item(window_menu.get(), zoom.get(), Some(sel!(performZoom:)), "");

        // Add the fullscreen toggle menu option.
        /* Cocoa should update the title to Enter or Exit Full Screen automatically.
         * But if not, then just fallback to Toggle Full Screen.
         */
        let toggle = ns_string("Toggle Full Screen");
        let f = ns_string("f");
        let menu_item = StrongPtr::from_owned(msg_send![super::alloc(class!(NSMenuItem)),
            initWithTitle: toggle.get() action: sel!(toggleFullScreen:) keyEquivalent: f.get()]);
        let _: () = msg_send![menu_item, setKeyEquivalentModifierMask:
            (NSEventModifierFlagControl | NSEventModifierFlagCommand)];
        let _: () = msg_send![window_menu, addItem: menu_item.get()];

        // Put menu into the menubar
        let menu_item = StrongPtr::from_owned(msg_send![super::alloc(class!(NSMenuItem)),
            initWithTitle: window_title.get() action: Sel::NULL keyEquivalent: empty.get()]);
        let _: () = msg_send![menu_item, setSubmenu: window_menu.get()];
        let main: Id = msg_send![app, mainMenu];
        let _: () = msg_send![main, addItem: menu_item.get()];

        // Tell the application object that this is now the window menu
        let _: () = msg_send![app, setWindowsMenu: window_menu.get()];
    }
}

/// `NSApp` (nil before `[NSApplication sharedApplication]`).
pub(crate) fn ns_app() -> Id {
    #[link(name = "AppKit", kind = "framework")]
    unsafe extern "C" {
        static NSApp: Id;
    }
    // SAFETY: the global AppKit keeps the shared application in.
    unsafe { std::ptr::read_volatile(std::ptr::addr_of!(NSApp)) }
}

/// Set up the shared application (as an `SDL3Application` unless the
/// program made its own), the menus and the app delegate. Translation of
/// `Cocoa_RegisterApp()`.
pub(crate) fn register_app() {
    autoreleasepool(|| {
        // This can get called more than once! Be careful what you initialize!

        // SAFETY: NSApplication setup as documented, on the main thread.
        unsafe {
            if ns_app().is_null() {
                let _: Id = msg_send![sdl3_application_class(), sharedApplication];
                crate::sdl_assert!(!ns_app().is_null());

                S_SHOULD_HANDLE_EVENTS_IN_SDL_APPLICATION.store(true, Ordering::Relaxed);

                if !hints::get_bool(hints::MAC_BACKGROUND_APP, false) {
                    let _: Bool = msg_send![ns_app(), setActivationPolicy: NSApplicationActivationPolicyRegular];
                }

                /* If there aren't already menus in place, look to see if there's
                 * a nib we should use. If not, then manually create the basic
                 * menus we meed.
                 */
                let main_menu: Id = msg_send![ns_app(), mainMenu];
                if main_menu.is_null() {
                    let nib_loaded = load_main_menu_nib_if_available();
                    if !nib_loaded {
                        create_application_menus();
                    }
                }
                let _: () = msg_send![ns_app(), finishLaunching];
                let delegate: Id = msg_send![ns_app(), delegate];
                if !delegate.is_null() {
                    /* The SDL app delegate calls this in didFinishLaunching if it's
                     * attached to the NSApp, otherwise we need to call it manually.
                     */
                    register_user_defaults();
                }
            }
            if !ns_app().is_null() && APP_DELEGATE.load(Ordering::Acquire).is_null() {
                let app_delegate: Id = msg_send![super::alloc(sdl3_app_delegate_class()), init];
                APP_DELEGATE.store(app_delegate, Ordering::Release);

                /* If someone else has an app delegate, it means we can't turn a
                 * termination into SDL_Quit, and we can't handle application:openFile:
                 */
                let delegate: Id = msg_send![ns_app(), delegate];
                if delegate.is_null() {
                    /* Only register the URL event handler if we are being set as the
                     * app delegate to avoid replacing any existing event handler.
                     */
                    let manager: Id =
                        msg_send![class!(NSAppleEventManager), sharedAppleEventManager];
                    let _: () = msg_send![manager,
                        setEventHandler: app_delegate
                        andSelector: sel!(handleURLEvent:withReplyEvent:)
                        forEventClass: kInternetEventClass
                        andEventID: kAEGetURL];

                    let _: () = msg_send![ns_app(), setDelegate: app_delegate];
                } else {
                    *seen_first_activate(app_delegate) = YES;
                }
            }
        }
    })
}

/// Translation of `timestamp_offset`.
static TIMESTAMP_OFFSET: AtomicU64 = AtomicU64::new(0);

/// An event's timestamp on SDL's clock. Translation of
/// `Cocoa_GetEventTimestamp()`.
pub(crate) fn get_event_timestamp(ns_timestamp: NSTimeInterval) -> Duration {
    let mut timestamp = (ns_timestamp * 1_000_000_000.0) as u64;
    let now = crate::timer::ticks_ns();

    let mut timestamp_offset = TIMESTAMP_OFFSET.load(Ordering::Relaxed);
    if timestamp_offset == 0 {
        timestamp_offset = now.wrapping_sub(timestamp);
    }
    timestamp = timestamp.wrapping_add(timestamp_offset);

    if timestamp > now {
        timestamp_offset = timestamp_offset.wrapping_sub(timestamp - now);
        timestamp = now;
    }
    TIMESTAMP_OFFSET.store(timestamp_offset, Ordering::Relaxed);
    Duration::from_nanos(timestamp)
}

/// Translation of `Cocoa_PumpEventsUntilDate()`.
fn pump_events_until_date(expiration: Id, accumulate: bool) -> i32 {
    // Run any existing modal sessions.
    for w in crate::video::core::window_ids() {
        if let Some(data) = window::window_data(w) {
            let session = data.state().modal_session;
            if !session.is_null() {
                // SAFETY: a modal session NSApp began.
                let _: NSInteger = unsafe { msg_send![ns_app(), runModalSession: session] };
            }
        }
    }

    loop {
        // SAFETY: NSApp's event queue.
        let event: Id = unsafe {
            msg_send![ns_app(), nextEventMatchingMask: NSEventMaskAny
                                             untilDate: expiration
                                                inMode: NSDefaultRunLoopMode
                                               dequeue: YES]
        };
        if event.is_null() {
            return 0;
        }

        if !S_SHOULD_HANDLE_EVENTS_IN_SDL_APPLICATION.load(Ordering::Relaxed) {
            dispatch_event(event);
        }

        // Pass events down to SDL3Application to be handled in sendEvent:
        // SAFETY: an event from the queue.
        let _: () = unsafe { msg_send![ns_app(), sendEvent: event] };
        if !accumulate {
            break;
        }
    }
    1
}

/// Translation of `Cocoa_WaitEventTimeout()` (`None` waits forever).
pub(crate) fn wait_event_timeout(timeout: Option<Duration>) -> i32 {
    autoreleasepool(|| {
        // SAFETY: NSDate's class methods.
        unsafe {
            match timeout {
                Some(t) if !t.is_zero() => {
                    let limit_date: Id = msg_send![class!(NSDate),
                        dateWithTimeIntervalSinceNow: t.as_secs_f64()];
                    pump_events_until_date(limit_date, false)
                }
                Some(_) => {
                    let past: Id = msg_send![class!(NSDate), distantPast];
                    pump_events_until_date(past, false)
                }
                None => {
                    loop {
                        let future: Id = msg_send![class!(NSDate), distantFuture];
                        if pump_events_until_date(future, false) != 0 {
                            break;
                        }
                    }
                    1
                }
            }
        }
    })
}

/// Translation of `Cocoa_PumpEvents()`.
pub(crate) fn pump_events() {
    autoreleasepool(|| {
        // SAFETY: NSDate's class method.
        let past: Id = unsafe { msg_send![class!(NSDate), distantPast] };
        pump_events_until_date(past, true);
    });
}

/// Translation of `Cocoa_SendWakeupEvent()`.
pub(crate) fn send_wakeup_event(window: WindowID) {
    autoreleasepool(|| {
        let window_number = window::window_data(window)
            .map(|d| d.state().window_number)
            .unwrap_or(0);
        // SAFETY: an application-defined event, posted to NSApp's queue.
        unsafe {
            let event: Id = msg_send![class!(NSEvent),
                otherEventWithType: NSEventTypeApplicationDefined
                location: NSPoint::new(0.0, 0.0)
                modifierFlags: 0 as NSUInteger
                timestamp: 0.0f64
                windowNumber: window_number
                context: NIL
                subtype: 0i16
                data1: 0 as NSInteger
                data2: 0 as NSInteger];

            let _: () = msg_send![ns_app(), postEvent: event atStart: YES];
        }
    });
}

/// Translation of `Cocoa_SuspendScreenSaver()`.
pub(crate) fn suspend_screen_saver(data: &Arc<VideoData>, suspend: bool) -> Result<()> {
    autoreleasepool(|| {
        let assertion = std::mem::replace(
            &mut data.state().screensaver_assertion,
            kIOPMNullAssertionID,
        );
        if assertion != kIOPMNullAssertionID {
            // SAFETY: an assertion we created.
            unsafe { IOPMAssertionRelease(assertion) };
        }

        if suspend {
            /* FIXME: this should ideally describe the real reason why the game
             * called SDL_DisableScreenSaver. Note that the name is only meant to be
             * seen by macOS power users. there's an additional optional human-readable
             * (localized) reason parameter which we don't set.
             */
            let mut assertion = kIOPMNullAssertionID;
            // SAFETY: NSString/CFString are toll-free bridged; the out
            // pointer is valid.
            unsafe {
                let name: Id = prefixed_suffix(
                    get_application_name().get(),
                    " using SDL_DisableScreenSaver",
                );
                let ty = cf_string(kIOPMAssertPreventUserIdleDisplaySleep);
                IOPMAssertionCreateWithDescription(
                    ty.get(),
                    name as *const c_void,
                    std::ptr::null(),
                    std::ptr::null(),
                    std::ptr::null(),
                    0.0,
                    std::ptr::null(),
                    &mut assertion,
                );
            }
            data.state().screensaver_assertion = assertion;
        }
    });
    Ok(())
}

/// `[name stringByAppendingString:@"suffix"]`.
///
/// # Safety
///
/// `name` must be an `NSString`.
unsafe fn prefixed_suffix(name: Id, suffix: &str) -> Id {
    let suffix = ns_string(suffix);
    // SAFETY: the caller's contract.
    unsafe { msg_send![name, stringByAppendingString: suffix.get()] }
}
