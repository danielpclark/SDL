// Rust translation of src/video/uikit/SDL_uikitappdelegate.m and
// SDL_uikitappdelegate.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! `SDL_RunApp()` on UIKit ([`run_app`]): `UIApplicationMain()` with SDL's
//! app delegate, which calls the app's main function once the app has
//! launched, from the main run loop.
//!
//! From iOS 13 (tvOS 13) the delegate is `SDLUIKitSceneDelegate`, which
//! also delegates the app's window scene: when the scene connects, it
//! shows the launch screen (the `UILaunchStoryboardName` storyboard, else
//! the launch image) in a window of its own until the app's first events
//! are pumped, makes the bundle's resource directory the working
//! directory, keeps the URLs the app was launched with (sent as drop
//! events once the main function pumps events, as are the URLs the app is
//! asked to open later), calls `SDL_SetMainReady()` and, on the next turn
//! of the run loop, `postFinishLaunch`, which runs the main function with
//! SDL's event pump on ([`set_ios_event_pump`]). The scene's activation
//! and background changes are the application events
//! ([`on_application_did_enter_foreground`] and the others). Before iOS
//! 13 the delegate is `SDLUIKitDelegate`, which does the same from
//! `application:didFinishLaunchingWithOptions:`.
//!
//! As upstream, when the main function returns the app keeps running (the
//! run loop goes on, for apps that set up in their main function and then
//! let UIKit's event loop run): `UIApplicationMain()` never returns, so
//! neither does [`run_app`], and the process ends with `exit()`
//! (`std::process::exit`), or when the system ends it. The main function
//! is called through a pointer kept in a thread-local variable, which
//! `run_app`'s frame, alive until the process ends, owns.
//!
//! [`set_ios_event_pump`]: crate::video::set_ios_event_pump
//! [`on_application_did_enter_foreground`]: crate::video::on_application_did_enter_foreground

use std::cell::Cell;
use std::ffi::{c_char, c_int, CString};
use std::sync::atomic::{AtomicI32, Ordering};

use crate::core::apple::block::RcBlock;
use crate::core::apple::foundation::{
    available, ns_array_items, ns_string, string_from_ns, CGRect,
};
#[cfg(not(target_os = "tvos"))]
use crate::core::apple::objc::NSUInteger;
use crate::core::apple::objc::{
    autoreleasepool, class, encoding, imp, ivar_ptr, msg_send, msg_send_super, sel, Bool, Class,
    ClassBuilder, DeclaredClass, Id, Sel, StrongPtr, NIL, NO, YES,
};
use crate::events::window::{send_drop_complete, send_drop_file};

use super::sys::*;

/// The app's main function, set by [`run_app`]: a pointer to a closure on
/// `run_app`'s stack, which lives as long as the process (upstream's
/// `forward_main`, `forward_argc` and `forward_argv`; the arguments are
/// the process's). It's taken by the first call.
type ForwardMain = *mut (dyn FnMut() -> i32 + 'static);

thread_local! {
    static FORWARD_MAIN: Cell<Option<ForwardMain>> = const { Cell::new(None) };
}

/// Translation of `exit_status`.
static EXIT_STATUS: AtomicI32 = AtomicI32::new(0);

/// Initialize SDL's main thread state and run the app: `UIApplicationMain()`
/// with SDL's app delegate, which calls `main_function` with the program's
/// arguments once the app has launched (see the module's description).
/// `UIApplicationMain()` never returns, so neither does this, even once
/// the main function has returned. Translation of `SDL_RunApp()` (UIKit).
pub(crate) fn run_app(main_function: impl FnOnce(&[String]) -> i32) -> i32 {
    // store arguments
    let mut main_function = Some(main_function);
    let mut forward = move || -> i32 {
        match main_function.take() {
            Some(f) => crate::app::call_main_function(f),
            None => 0,
        }
    };
    let forward: &mut (dyn FnMut() -> i32 + '_) = &mut forward;
    // SAFETY: only the lifetime bound is changed: the pointer is cleared
    // (by `_clear`) before `forward` goes away, and the delegate calls it
    // from within UIApplicationMain(), below, on this thread.
    let forward: ForwardMain =
        unsafe { std::mem::transmute::<*mut (dyn FnMut() -> i32 + '_), ForwardMain>(forward) };
    FORWARD_MAIN.with(|f| f.set(Some(forward)));
    struct Clear;
    impl Drop for Clear {
        fn drop(&mut self) {
            FORWARD_MAIN.with(|f| f.set(None));
        }
    }
    let _clear = Clear;

    use std::os::unix::ffi::OsStringExt;
    let args: Vec<CString> = std::env::args_os()
        .map(|a| {
            let mut bytes = a.into_vec();
            bytes.retain(|&b| b != 0);
            CString::new(bytes).unwrap_or_default()
        })
        .collect();
    let mut argv: Vec<*mut c_char> = args
        .iter()
        .map(|a| a.as_ptr() as *mut c_char)
        .chain(std::iter::once(std::ptr::null_mut()))
        .collect();

    // (the classes UIKit looks up by name)
    launch_screen_controller_class();
    let scene_delegate = scene_delegate_class();
    let app_delegate = app_delegate_class();

    // Give over control to run loop, SDLUIKitDelegate will handle most things from here
    autoreleasepool(|| {
        let mut name: Id = NIL;

        if available!(ios(13, 0), tvos(13, 0)) {
            // SAFETY: the class method below, answering an NSString.
            name = unsafe { msg_send![scene_delegate, getSceneDelegateClassName] };
        }
        if name.is_null() {
            // SAFETY: as above.
            name = unsafe { msg_send![app_delegate, getAppDelegateClassName] };
        }
        // SAFETY: the process's arguments, NULL-terminated, which outlive
        // the call; the delegate's class name.
        unsafe {
            UIApplicationMain((argv.len() - 1) as c_int, argv.as_mut_ptr(), NIL, name);
        }
    });

    EXIT_STATUS.load(Ordering::Relaxed)
}

/// Run the app's main function, the first time (`exit_status =
/// SDL_CallMainFunction(forward_argc, forward_argv, forward_main)`).
fn call_forward_main() {
    if let Some(forward) = FORWARD_MAIN.with(|f| f.get()) {
        // SAFETY: run_app's closure, alive while UIApplicationMain() runs
        // (on this thread, the main thread), not otherwise borrowed.
        let status = unsafe { (*forward)() };
        EXIT_STATUS.store(status, Ordering::Relaxed);
    }
}

// ---------------------------------------------------------------------------
// Instance variables holding objects (strong references)

/// The object in an instance variable (`self->name`).
fn get_ivar(this: Id, cls: Class, name: &std::ffi::CStr) -> Id {
    // SAFETY: an instance of `cls`, which has the object variable `name`.
    unsafe { *ivar_ptr::<Id>(this, cls, name) }
}

/// `self->name = value` under ARC: `value` retained, the old one released.
fn set_ivar(this: Id, cls: Class, name: &std::ffi::CStr, value: Id) {
    // SAFETY: as above; the variable owns its object.
    unsafe {
        let slot = ivar_ptr::<Id>(this, cls, name);
        let old = *slot;
        *slot = crate::core::apple::objc::retain(value);
        crate::core::apple::objc::release(old);
    }
}

/// `[[NSBundle mainBundle] objectForInfoDictionaryKey:key]`.
fn info_dictionary_value(bundle: Id, key: &str) -> Id {
    let key = ns_string(key);
    // SAFETY: an NSBundle method taking an NSString.
    unsafe { msg_send![bundle, objectForInfoDictionaryKey: key.get()] }
}

/// `[NSBundle mainBundle]`.
fn main_bundle() -> Id {
    // SAFETY: NSBundle's class method.
    unsafe { msg_send![class!(NSBundle), mainBundle] }
}

/// `[UIImage imageNamed:name]`.
#[cfg(not(any(target_os = "tvos", target_os = "visionos")))]
fn image_named(name: &str) -> Id {
    let name = ns_string(name);
    // SAFETY: UIImage's class method taking an NSString.
    unsafe { msg_send![class!(UIImage), imageNamed: name.get()] }
}

// ---------------------------------------------------------------------------
// The launch screen

/// Load a launch image using the old UILaunchImageFile-era naming rules.
/// Translation of `SDL_LoadLaunchImageNamed()`.
#[cfg(not(any(target_os = "tvos", target_os = "visionos")))]
fn load_launch_image_named(name: &str, screenh: i32) -> Id {
    let curorient = super::get_interface_orientation();
    let idiom = crate::core::apple::uikit::user_interface_idiom();
    let mut image = NIL;

    if idiom == UIUserInterfaceIdiomPhone && screenh == 568 {
        // The image name for the iPhone 5 uses its height as a suffix.
        image = image_named(&format!("{name}-568h"));
    } else if idiom == UIUserInterfaceIdiomPad {
        // iPad apps can launch in any orientation.
        if UIInterfaceOrientationIsLandscape(curorient) {
            if curorient == UIInterfaceOrientationLandscapeLeft {
                image = image_named(&format!("{name}-LandscapeLeft"));
            } else {
                image = image_named(&format!("{name}-LandscapeRight"));
            }
            if image.is_null() {
                image = image_named(&format!("{name}-Landscape"));
            }
        } else {
            if curorient == UIInterfaceOrientationPortraitUpsideDown {
                image = image_named(&format!("{name}-PortraitUpsideDown"));
            }
            if image.is_null() {
                image = image_named(&format!("{name}-Portrait"));
            }
        }
    }

    if image.is_null() {
        image = image_named(name);
    }

    image
}

#[cfg(not(any(target_os = "tvos", target_os = "visionos")))]
static SDL_LAUNCH_STORYBOARD_VIEW_CONTROLLER: DeclaredClass = DeclaredClass::new();

/// The `SDLLaunchStoryboardViewController` class (`@interface
/// SDLLaunchStoryboardViewController : UIViewController`), with its
/// `storyboardViewController` (strong) property as an instance variable.
#[cfg(not(any(target_os = "tvos", target_os = "visionos")))]
fn launch_storyboard_view_controller_class() -> Class {
    SDL_LAUNCH_STORYBOARD_VIEW_CONTROLLER.get(|| {
        let mut b = ClassBuilder::new(
            c"SDLLaunchStoryboardViewController",
            class!(UIViewController),
        )
        .expect("SDLLaunchStoryboardViewController");
        b.add_ivar::<Id>(c"storyboardViewController", c"@");
        // SAFETY: the implementations have the signatures of the encodings.
        unsafe {
            b.add_method(
                sel!(initWithStoryboardViewController:),
                imp!(storyboard_init as extern "C" fn(Id, Sel, Id) -> Id),
                c"@@:@",
            );
            b.add_method(
                sel!(dealloc),
                imp!(storyboard_dealloc as extern "C" fn(Id, Sel)),
                c"v@:",
            );
            b.add_method(
                sel!(viewDidLoad),
                imp!(storyboard_view_did_load as extern "C" fn(Id, Sel)),
                c"v@:",
            );
            b.add_method(
                sel!(prefersStatusBarHidden),
                imp!(storyboard_prefers_status_bar_hidden as extern "C" fn(Id, Sel) -> Bool),
                &encoding("B@:"),
            );
            b.add_method(
                sel!(preferredStatusBarStyle),
                imp!(
                    storyboard_preferred_status_bar_style
                        as extern "C" fn(Id, Sel) -> UIStatusBarStyle
                ),
                c"q@:",
            );
        }
        b.register()
    })
}

/// Translation of
/// `-[SDLLaunchStoryboardViewController initWithStoryboardViewController:]`.
#[cfg(not(any(target_os = "tvos", target_os = "visionos")))]
extern "C" fn storyboard_init(this: Id, _sel: Sel, storyboard_view_controller: Id) -> Id {
    let cls = launch_storyboard_view_controller_class();
    // SAFETY: UIViewController's init.
    let this: Id = unsafe { msg_send_super![this, cls, init] };
    if !this.is_null() {
        set_ivar(
            this,
            cls,
            c"storyboardViewController",
            storyboard_view_controller,
        );
    }
    this
}

/// `-[SDLLaunchStoryboardViewController dealloc]` (ARC's).
#[cfg(not(any(target_os = "tvos", target_os = "visionos")))]
extern "C" fn storyboard_dealloc(this: Id, _sel: Sel) {
    let cls = launch_storyboard_view_controller_class();
    set_ivar(this, cls, c"storyboardViewController", NIL);
    // SAFETY: UIViewController's dealloc.
    let () = unsafe { msg_send_super![this, cls, dealloc] };
}

/// Translation of `-[SDLLaunchStoryboardViewController viewDidLoad]`.
#[cfg(not(any(target_os = "tvos", target_os = "visionos")))]
extern "C" fn storyboard_view_did_load(this: Id, _sel: Sel) {
    let cls = launch_storyboard_view_controller_class();
    // SAFETY: UIViewController's methods and UIView's properties, on this
    // controller and its child.
    unsafe {
        let () = msg_send_super![this, cls, viewDidLoad];

        let child = get_ivar(this, cls, c"storyboardViewController");
        let () = msg_send![this, addChildViewController: child];
        let view: Id = msg_send![this, view];
        let child_view: Id = msg_send![child, view];
        let () = msg_send![view, addSubview: child_view];
        let () = msg_send![child_view, setAutoresizingMask: UIViewAutoresizingFlexibleWidth | UIViewAutoresizingFlexibleHeight];
        let bounds: CGRect = msg_send![view, bounds];
        let () = msg_send![child_view, setFrame: bounds];
        let () = msg_send![child, didMoveToParentViewController: this];

        // (deprecated since iOS 9)
        let app = crate::core::apple::uikit::shared_application();
        let hidden: Bool = msg_send![this, prefersStatusBarHidden];
        let () = msg_send![app, setStatusBarHidden: hidden];
        let style: UIStatusBarStyle = msg_send![this, preferredStatusBarStyle];
        let () = msg_send![app, setStatusBarStyle: style];
    }
}

/// Translation of `-[SDLLaunchStoryboardViewController prefersStatusBarHidden]`.
#[cfg(not(any(target_os = "tvos", target_os = "visionos")))]
extern "C" fn storyboard_prefers_status_bar_hidden(_this: Id, _sel: Sel) -> Bool {
    let value = info_dictionary_value(main_bundle(), "UIStatusBarHidden");
    // SAFETY: an NSNumber (or nil, answering NO).
    unsafe { msg_send![value, boolValue] }
}

/// Translation of `-[SDLLaunchStoryboardViewController preferredStatusBarStyle]`.
#[cfg(not(any(target_os = "tvos", target_os = "visionos")))]
extern "C" fn storyboard_preferred_status_bar_style(_this: Id, _sel: Sel) -> UIStatusBarStyle {
    let status_bar_style = info_dictionary_value(main_bundle(), "UIStatusBarStyle");
    // SAFETY: an NSString (or nil).
    let status_bar_style = unsafe { string_from_ns(status_bar_style) };
    if status_bar_style.as_deref() == Some("UIStatusBarStyleLightContent") {
        return UIStatusBarStyleLightContent;
    }
    if available!(ios(13, 0)) && status_bar_style.as_deref() == Some("UIStatusBarStyleDarkContent")
    {
        return UIStatusBarStyleDarkContent;
    }
    UIStatusBarStyleDefault
}

static SDL_LAUNCH_SCREEN_CONTROLLER: DeclaredClass = DeclaredClass::new();

/// The `SDLLaunchScreenController` class (`@interface
/// SDLLaunchScreenController : UIViewController`).
fn launch_screen_controller_class() -> Class {
    SDL_LAUNCH_SCREEN_CONTROLLER.get(|| {
        let mut b = ClassBuilder::new(c"SDLLaunchScreenController", class!(UIViewController))
            .expect("SDLLaunchScreenController");
        // SAFETY: the implementations have the signatures of the encodings.
        unsafe {
            b.add_method(
                sel!(init),
                imp!(launch_screen_init as extern "C" fn(Id, Sel) -> Id),
                c"@@:",
            );
            b.add_method(
                sel!(initWithNibName:bundle:),
                imp!(launch_screen_init_with_nib as extern "C" fn(Id, Sel, Id, Id) -> Id),
                c"@@:@@",
            );
            b.add_method(
                sel!(loadView),
                imp!(launch_screen_load_view as extern "C" fn(Id, Sel)),
                c"v@:",
            );
            #[cfg(not(target_os = "tvos"))]
            b.add_method(
                sel!(shouldAutorotate),
                imp!(launch_screen_should_autorotate as extern "C" fn(Id, Sel) -> Bool),
                &encoding("B@:"),
            );
            #[cfg(not(target_os = "tvos"))]
            b.add_method(
                sel!(supportedInterfaceOrientations),
                imp!(launch_screen_supported_orientations as extern "C" fn(Id, Sel) -> NSUInteger),
                c"Q@:",
            );
        }
        b.register()
    })
}

/// Translation of `-[SDLLaunchScreenController init]`.
extern "C" fn launch_screen_init(this: Id, _sel: Sel) -> Id {
    // SAFETY: this class's initializer, with the main bundle.
    unsafe { msg_send![this, initWithNibName: NIL bundle: main_bundle()] }
}

/// Whether `[bundle loadNibNamed:name owner:owner options:nil][0]` can be
/// done (the nib is in the bundle, and has objects): the view, or nil if
/// it can't.
///
/// Note (upstream): upstream loads the nib in `@try`, returning nil from
/// the initializer when it throws (the nib isn't in the bundle, or is
/// empty); Objective-C exceptions can't be caught here, so that's checked
/// first.
fn load_nib_view(bundle: Id, name: Id, owner: Id) -> Option<Id> {
    // SAFETY: NSBundle's methods with NSStrings; the nib's objects, an
    // NSArray.
    unsafe {
        let nib = ns_string("nib");
        let path: Id = msg_send![bundle, pathForResource: name ofType: nib.get()];
        if path.is_null() {
            return None;
        }
        let objects: Id = msg_send![bundle, loadNibNamed: name owner: owner options: NIL];
        ns_array_items(objects).first().copied()
    }
}

/// Translation of `-[SDLLaunchScreenController initWithNibName:bundle:]`.
extern "C" fn launch_screen_init_with_nib(
    this: Id,
    _sel: Sel,
    nib_name_or_nil: Id,
    nib_bundle_or_nil: Id,
) -> Id {
    let cls = launch_screen_controller_class();
    // SAFETY: UIViewController's initializer.
    let this: Id = unsafe { msg_send_super![this, cls, initWithNibName: NIL bundle: NIL] };
    if this.is_null() {
        return NIL;
    }

    let screenname = nib_name_or_nil;
    let bundle = nib_bundle_or_nil;

    // A launch screen may not exist. Fall back to launch images in that case.
    if !screenname.is_null() {
        match load_nib_view(bundle, screenname, this) {
            // SAFETY: a UIViewController property.
            Some(view) => unsafe {
                let () = msg_send![this, setView: view];
            },
            None => {
                /* If a launch screen name is specified but it fails to load, iOS
                 * displays a blank screen rather than falling back to an image. */
                // SAFETY: the reference init answered (ARC releases it).
                unsafe { crate::core::apple::objc::release(this) };
                return NIL;
            }
        }
    }

    // SAFETY: UIViewController's view (loadView does nothing: nil unless
    // set).
    let view: Id = unsafe { msg_send![this, view] };
    if view.is_null() {
        load_launch_image(this, bundle);
    }

    this
}

/// The launch image part of `-[SDLLaunchScreenController
/// initWithNibName:bundle:]`.
fn load_launch_image(this: Id, bundle: Id) {
    let launchimages = info_dictionary_value(bundle, "UILaunchImages");
    let mut imagename: Id = NIL;
    let mut image: Id = NIL;

    #[cfg(target_os = "visionos")]
    let (screenw, screenh) = (
        super::modes::SDL_XR_SCREENWIDTH,
        super::modes::SDL_XR_SCREENHEIGHT,
    );
    #[cfg(not(target_os = "visionos"))]
    #[cfg_attr(target_os = "tvos", allow(unused_mut))]
    let (mut screenw, mut screenh) = {
        // SAFETY: UIScreen's main screen's bounds.
        let bounds: CGRect = unsafe { msg_send![super::modes::main_screen(), bounds] };
        (
            (bounds.size.width + 0.5) as i32,
            (bounds.size.height + 0.5) as i32,
        )
    };

    #[cfg(not(any(target_os = "tvos", target_os = "visionos")))]
    let curorient = super::get_interface_orientation();

    // We always want portrait-oriented size, to match UILaunchImageSize.
    #[cfg(not(any(target_os = "tvos", target_os = "visionos")))]
    if screenw > screenh {
        std::mem::swap(&mut screenw, &mut screenh);
    }

    // Xcode 5 introduced a dictionary of launch images in Info.plist.
    if !launchimages.is_null() {
        // SAFETY: an NSArray of NSDictionary, with NSString values.
        unsafe {
            for dict in ns_array_items(launchimages) {
                let value = |key: &str| -> Id {
                    let key = ns_string(key);
                    msg_send![dict, objectForKey: key.get()]
                };
                let minversion = value("UILaunchImageMinimumOSVersion");
                let sizestring = value("UILaunchImageSize");

                // Ignore this image if the current version is too low.
                if !minversion.is_null() {
                    let version: f64 = msg_send![minversion, doubleValue];
                    if !super::is_system_version_at_least(version) {
                        continue;
                    }
                }

                // Ignore this image if the size doesn't match.
                if !sizestring.is_null() {
                    let size = CGSizeFromString(sizestring);
                    if (size.width + 0.5) as i32 != screenw || (size.height + 0.5) as i32 != screenh
                    {
                        continue;
                    }
                }

                #[cfg(not(any(target_os = "tvos", target_os = "visionos")))]
                {
                    let mut orientmask = UIInterfaceOrientationMaskPortrait
                        | UIInterfaceOrientationMaskPortraitUpsideDown;
                    let orientstring = string_from_ns(value("UILaunchImageOrientation"));

                    match orientstring.as_deref() {
                        Some("PortraitUpsideDown") => {
                            orientmask = UIInterfaceOrientationMaskPortraitUpsideDown
                        }
                        Some("Landscape") => orientmask = UIInterfaceOrientationMaskLandscape,
                        Some("LandscapeLeft") => {
                            orientmask = UIInterfaceOrientationMaskLandscapeLeft
                        }
                        Some("LandscapeRight") => {
                            orientmask = UIInterfaceOrientationMaskLandscapeRight
                        }
                        _ => {}
                    }

                    // Ignore this image if the orientation doesn't match.
                    if (orientmask & (1usize << (curorient as u32 & 63))) == 0 {
                        continue;
                    }
                }

                imagename = value("UILaunchImageName");
            }

            if !imagename.is_null() {
                image = msg_send![class!(UIImage), imageNamed: imagename];
            }
        }
    } else {
        #[cfg(not(any(target_os = "tvos", target_os = "visionos")))]
        {
            imagename = info_dictionary_value(bundle, "UILaunchImageFile");

            // SAFETY: an NSString (or nil).
            if let Some(name) = unsafe { string_from_ns(imagename) } {
                image = load_launch_image_named(&name, screenh);
            }

            if image.is_null() {
                image = load_launch_image_named("Default", screenh);
            }
        }
    }
    let _ = (imagename, screenw);

    if !image.is_null() {
        #[cfg(target_os = "visionos")]
        let view_frame = CGRect::new(0.0, 0.0, screenw as f64, screenh as f64);
        // SAFETY: UIScreen's main screen's bounds.
        #[cfg(not(target_os = "visionos"))]
        let view_frame: CGRect = unsafe { msg_send![super::modes::main_screen(), bounds] };
        #[cfg_attr(any(target_os = "tvos", target_os = "visionos"), allow(unused_mut))]
        let mut imageorient = UIImageOrientationUp;

        // Bugs observed / workaround tested in iOS 8.3.
        #[cfg(not(any(target_os = "tvos", target_os = "visionos")))]
        if UIInterfaceOrientationIsLandscape(curorient) {
            // SAFETY: a UIImage property.
            let size: crate::core::apple::foundation::CGSize = unsafe { msg_send![image, size] };
            if size.width < size.height {
                /* On iOS 8, portrait launch images displayed in forced-
                 * landscape mode (e.g. a standard Default.png on an iPhone
                 * when Info.plist only supports landscape orientations) need
                 * to be rotated to display in the expected orientation. */
                if curorient == UIInterfaceOrientationLandscapeLeft {
                    imageorient = UIImageOrientationRight;
                } else if curorient == UIInterfaceOrientationLandscapeRight {
                    imageorient = UIImageOrientationLeft;
                }
            }
        }

        // SAFETY: UIImageView and UIImage initializers (owned results),
        // with the image's CGImage and scale; the controller's view.
        unsafe {
            let view: Id = msg_send![class!(UIImageView), alloc];
            let view = StrongPtr::from_owned(msg_send![view, initWithFrame: view_frame]);

            // Create the properly oriented image.
            let cgimage: *mut std::ffi::c_void = msg_send![image, CGImage];
            let scale: crate::core::apple::foundation::CGFloat = msg_send![image, scale];
            let oriented: Id = msg_send![class!(UIImage), alloc];
            let oriented = StrongPtr::from_owned(
                msg_send![oriented, initWithCGImage: cgimage scale: scale orientation: imageorient],
            );
            let () = msg_send![view.get(), setImage: oriented.get()];

            let () = msg_send![this, setView: view.get()];
        }
    }
}

/// Translation of `-[SDLLaunchScreenController loadView]`.
extern "C" fn launch_screen_load_view(_this: Id, _sel: Sel) {
    // Do nothing.
}

/// Translation of `-[SDLLaunchScreenController shouldAutorotate]`.
#[cfg(not(target_os = "tvos"))]
extern "C" fn launch_screen_should_autorotate(_this: Id, _sel: Sel) -> Bool {
    // If YES, the launch image will be incorrectly rotated in some cases.
    NO
}

/// Translation of `-[SDLLaunchScreenController supportedInterfaceOrientations]`.
#[cfg(not(target_os = "tvos"))]
extern "C" fn launch_screen_supported_orientations(_this: Id, _sel: Sel) -> NSUInteger {
    /* We keep the supported orientations unrestricted to avoid the case where
     * there are no common orientations between the ones set in Info.plist and
     * the ones set here (it will cause an exception in that case.) */
    UIInterfaceOrientationMaskAll
}

/// The launch screen's view controller: the launch storyboard's (not on
/// tvOS or visionOS), else an `SDLLaunchScreenController` (owned, nil if
/// it failed). Upstream repeats this in both delegates.
fn launch_view_controller(bundle: Id) -> StrongPtr {
    #[cfg_attr(any(target_os = "tvos", target_os = "visionos"), allow(unused_mut))]
    let mut vc = StrongPtr::nil();

    // tvOS only uses a plain launch image.
    #[cfg(any(target_os = "tvos", target_os = "visionos"))]
    let screenname: Id = NIL;
    #[cfg(not(any(target_os = "tvos", target_os = "visionos")))]
    let screenname = info_dictionary_value(bundle, "UILaunchStoryboardName");
    #[cfg(not(any(target_os = "tvos", target_os = "visionos")))]
    {
        if !screenname.is_null() {
            /* The launch storyboard is actually a nib in some older versions of
             * Xcode. We'll try to load it as a storyboard first, as it's more
             * modern. */
            // Note (upstream): upstream loads the storyboard in @try,
            // going on to the nib when it throws (the storyboard isn't in
            // the bundle); Objective-C exceptions can't be caught here, so
            // that's checked first.
            // SAFETY: NSBundle's, UIStoryboard's and this file's
            // controller's methods; the controller is owned.
            unsafe {
                let storyboardc = ns_string("storyboardc");
                let path: Id =
                    msg_send![bundle, pathForResource: screenname ofType: storyboardc.get()];
                if !path.is_null() {
                    let storyboard: Id = msg_send![class!(UIStoryboard), storyboardWithName: screenname bundle: bundle];
                    let storyboard_vc: Id = msg_send![storyboard, instantiateInitialViewController];
                    let controller: Id =
                        msg_send![launch_storyboard_view_controller_class(), alloc];
                    vc = StrongPtr::from_owned(
                        msg_send![controller, initWithStoryboardViewController: storyboard_vc],
                    );
                }
            }
        }
    }

    if vc.is_nil() {
        // SAFETY: this file's controller (owned).
        unsafe {
            let controller: Id = msg_send![launch_screen_controller_class(), alloc];
            vc = StrongPtr::from_owned(
                msg_send![controller, initWithNibName: screenname bundle: bundle],
            );
        }
    }
    vc
}

/// Show the launch screen's view controller in a window above the app's
/// (not key: events go to the app's windows): the delegate's
/// `launchWindow` (owned), made by `make_window` (owned too).
fn show_launch_window(
    vc: &StrongPtr,
    view_frame: CGRect,
    make_window: impl FnOnce() -> StrongPtr,
) -> StrongPtr {
    let launch_window = make_window();
    // SAFETY: UIWindow's properties.
    unsafe {
        let () = msg_send![launch_window.get(), setFrame: view_frame];

        /* We don't want the launch window immediately hidden when a real SDL
         * window is shown - we fade it out ourselves when we're ready. */
        let () = msg_send![launch_window.get(), setWindowLevel: UIWindowLevelNormal + 1.0];

        /* Show the window but don't make it key. Events should always go to
         * other windows when possible. */
        let () = msg_send![launch_window.get(), setHidden: NO];

        let () = msg_send![launch_window.get(), setRootViewController: vc.get()];
    }
    launch_window
}

/// Set working directory to resource path.
fn change_to_resource_directory(bundle: Id) {
    // SAFETY: NSFileManager's and NSBundle's methods.
    unsafe {
        let manager: Id = msg_send![class!(NSFileManager), defaultManager];
        let path: Id = msg_send![bundle, resourcePath];
        let _: Bool = msg_send![manager, changeCurrentDirectoryPath: path];
    }
}

/// Fade the launch window out and hide it (the delegates'
/// `hideLaunchScreen`), taking it from the `launchWindow` instance
/// variable.
fn hide_launch_screen(this: Id, cls: Class) {
    let window = get_ivar(this, cls, c"launchWindow");

    if window.is_null() {
        return;
    }
    // SAFETY: a UIView property.
    let hidden: Bool = unsafe { msg_send![window, isHidden] };
    if hidden.is_true() {
        return;
    }

    // SAFETY: the window, kept by the blocks.
    let window = unsafe { StrongPtr::retain(window) };
    set_ivar(this, cls, c"launchWindow", NIL);

    // Do a nice animated fade-out (roughly matches the real launch behavior.)
    // (the blocks keep the window, and UIKit calls them on the main thread)
    let fading = window.clone();
    let animations = RcBlock::new0(move || {
        // SAFETY: a UIView property.
        let () = unsafe { msg_send![fading.get(), setAlpha: 0.0f64] };
    });
    let hiding = window;
    let completion = RcBlock::new1(move |_finished: Bool| {
        // SAFETY: a UIView property.
        let () = unsafe { msg_send![hiding.get(), setHidden: YES] };
        super::force_update_home_indicator(); // Wait for launch screen to hide so settings are applied to the actual view controller.
    });
    // SAFETY: UIView's class method; UIKit copies the blocks.
    unsafe {
        let () = msg_send![class!(UIView), animateWithDuration: 0.2f64
                                                   animations: animations.as_ptr()
                                                   completion: completion.as_ptr()];
    }
}

/// `[self performSelector:selector withObject:nil afterDelay:0.0]`: on the
/// next turn of the run loop.
fn perform_later(this: Id, selector: Sel) {
    // SAFETY: an NSObject method; `selector` is one of this object's
    // methods taking no argument.
    unsafe {
        let () = msg_send![this, performSelector: selector withObject: NIL afterDelay: 0.0f64];
    }
}

/// Send a URL as a drop event, a file's as its path (shared by
/// `-[SDLUIKitSceneDelegate handleURL:]` and `-[SDLUIKitDelegate
/// sendDropFileForURL:fromSourceApplication:]`).
fn send_drop_file_for_url(url: Id, source_application: Option<&str>) {
    // SAFETY: NSURL's properties, NSStrings.
    let path = unsafe {
        let file_url: Id = msg_send![url, filePathURL];
        if !file_url.is_null() {
            string_from_ns(msg_send![file_url, path])
        } else {
            string_from_ns(msg_send![url, absoluteString])
        }
    };
    send_drop_file(
        None,
        source_application,
        path.as_deref().unwrap_or_default(),
    );
    send_drop_complete(None);
}

/// Run the app's main function with SDL's event pump on (the delegates'
/// `postFinishLaunch`), then hide the launch window if it's still up.
fn run_main_function(this: Id, cls: Class) {
    crate::video::set_ios_event_pump(true);
    call_forward_main();
    crate::video::set_ios_event_pump(false);

    let launch_window = get_ivar(this, cls, c"launchWindow");
    if !launch_window.is_null() {
        // SAFETY: a UIView property.
        let () = unsafe { msg_send![launch_window, setHidden: YES] };
        set_ivar(this, cls, c"launchWindow", NIL);
    }
}

// ---------------------------------------------------------------------------
// SDLUIKitSceneDelegate

static SDL_UIKIT_SCENE_DELEGATE: DeclaredClass = DeclaredClass::new();

/// The `SDLUIKitSceneDelegate` class (`@interface SDLUIKitSceneDelegate :
/// NSObject <UIApplicationDelegate, UIWindowSceneDelegate>`, iOS 13), with
/// its `launchWindow` and `launchURLs` instance variables.
pub(crate) fn scene_delegate_class() -> Class {
    SDL_UIKIT_SCENE_DELEGATE.get(|| {
        let mut b = ClassBuilder::new(c"SDLUIKitSceneDelegate", class!(NSObject))
            .expect("SDLUIKitSceneDelegate");
        b.add_protocol(c"UIApplicationDelegate");
        b.add_protocol(c"UIWindowSceneDelegate");
        b.add_ivar::<Id>(c"launchWindow", c"@");
        b.add_ivar::<Id>(c"launchURLs", c"@");
        // SAFETY: the implementations have the signatures of the encodings.
        unsafe {
            b.add_class_method(
                sel!(getSceneDelegateClassName),
                imp!(scene_get_class_name as extern "C" fn(Id, Sel) -> Id),
                c"@@:",
            );
            b.add_method(
                sel!(dealloc),
                imp!(scene_dealloc as extern "C" fn(Id, Sel)),
                c"v@:",
            );
            b.add_method(
                sel!(scene:willConnectToSession:options:),
                imp!(scene_will_connect as extern "C" fn(Id, Sel, Id, Id, Id)),
                c"v@:@@@",
            );
            #[cfg(not(any(target_os = "tvos", target_os = "visionos")))]
            b.add_method(
                sel!(windowScene:didUpdateEffectiveGeometry:),
                imp!(scene_did_update_effective_geometry as extern "C" fn(Id, Sel, Id, Id)),
                c"v@:@@",
            );
            b.add_method(
                sel!(scene:openURLContexts:),
                imp!(scene_open_url_contexts as extern "C" fn(Id, Sel, Id, Id)),
                c"v@:@@",
            );
            b.add_method(
                sel!(sceneDidBecomeActive:),
                imp!(scene_did_become_active as extern "C" fn(Id, Sel, Id)),
                c"v@:@",
            );
            b.add_method(
                sel!(sceneWillResignActive:),
                imp!(scene_will_resign_active as extern "C" fn(Id, Sel, Id)),
                c"v@:@",
            );
            b.add_method(
                sel!(sceneWillEnterForeground:),
                imp!(scene_will_enter_foreground as extern "C" fn(Id, Sel, Id)),
                c"v@:@",
            );
            b.add_method(
                sel!(sceneDidEnterBackground:),
                imp!(scene_did_enter_background as extern "C" fn(Id, Sel, Id)),
                c"v@:@",
            );
            b.add_method(
                sel!(handleURL:),
                imp!(scene_handle_url as extern "C" fn(Id, Sel, Id)),
                c"v@:@",
            );
            b.add_method(
                sel!(hideLaunchScreen),
                imp!(scene_hide_launch_screen as extern "C" fn(Id, Sel)),
                c"v@:",
            );
            b.add_method(
                sel!(postFinishLaunch),
                imp!(scene_post_finish_launch as extern "C" fn(Id, Sel)),
                c"v@:",
            );
            b.add_method(
                sel!(processLaunchURLs),
                imp!(scene_process_launch_urls as extern "C" fn(Id, Sel)),
                c"v@:",
            );
            b.add_method(
                sel!(application:configurationForConnectingSceneSession:options:),
                imp!(scene_configuration as extern "C" fn(Id, Sel, Id, Id, Id) -> Id),
                c"@@:@@@",
            );
        }
        b.register()
    })
}

/// Translation of `+[SDLUIKitSceneDelegate getSceneDelegateClassName]`.
extern "C" fn scene_get_class_name(_cls: Id, _sel: Sel) -> Id {
    ns_string("SDLUIKitSceneDelegate").autorelease()
}

/// `-[SDLUIKitSceneDelegate dealloc]` (ARC's).
extern "C" fn scene_dealloc(this: Id, _sel: Sel) {
    let cls = scene_delegate_class();
    set_ivar(this, cls, c"launchWindow", NIL);
    set_ivar(this, cls, c"launchURLs", NIL);
    // SAFETY: NSObject's dealloc.
    let () = unsafe { msg_send_super![this, cls, dealloc] };
}

/// The objects of an `NSSet` (unretained).
///
/// # Safety
///
/// `set` must be nil or a valid `NSSet`.
unsafe fn ns_set_items(set: Id) -> Vec<Id> {
    // SAFETY: the caller's contract; allObjects is an NSArray.
    unsafe { ns_array_items(msg_send![set, allObjects]) }
}

/// Translation of `-[SDLUIKitSceneDelegate scene:willConnectToSession:options:]`.
extern "C" fn scene_will_connect(
    this: Id,
    _sel: Sel,
    scene: Id,
    _session: Id,
    connection_options: Id,
) {
    let cls = scene_delegate_class();
    // SAFETY: a UIScene.
    if !unsafe { crate::core::apple::objc::is_kind_of(scene, class!(UIWindowScene)) } {
        return;
    }

    let window_scene = scene;
    // SAFETY: a UIScene property.
    let () = unsafe { msg_send![window_scene, setDelegate: this] };

    let bundle = main_bundle();

    let vc = launch_view_controller(bundle);

    // SAFETY: a UIViewController property (nil for nil).
    let vc_view: Id = unsafe { msg_send![vc.get(), view] };
    if !vc_view.is_null() {
        #[cfg(target_os = "visionos")]
        let view_frame = CGRect::new(
            0.0,
            0.0,
            super::modes::SDL_XR_SCREENWIDTH as f64,
            super::modes::SDL_XR_SCREENHEIGHT as f64,
        );
        // SAFETY: UIWindowScene's coordinate space, a UICoordinateSpace.
        #[cfg(not(target_os = "visionos"))]
        let view_frame: CGRect = unsafe {
            let space: Id = msg_send![window_scene, coordinateSpace];
            msg_send![space, bounds]
        };
        let launch_window = show_launch_window(&vc, view_frame, || {
            // SAFETY: UIWindow's initializer (owned).
            unsafe {
                let window: Id = msg_send![class!(UIWindow), alloc];
                StrongPtr::from_owned(msg_send![window, initWithWindowScene: window_scene])
            }
        });
        set_ivar(this, cls, c"launchWindow", launch_window.get());
    }

    change_to_resource_directory(bundle);

    // SAFETY: NSMutableArray (owned, then kept by the variable);
    // UISceneConnectionOptions' sets of NSUserActivity and
    // UIOpenURLContext, with their NSURLs.
    unsafe {
        let launch_urls = StrongPtr::from_owned(msg_send![class!(NSMutableArray), new]);
        set_ivar(this, cls, c"launchURLs", launch_urls.get());

        let activities: Id = msg_send![connection_options, userActivities];
        for activity in ns_set_items(activities) {
            let url: Id = msg_send![activity, webpageURL];
            if !url.is_null() {
                let () = msg_send![launch_urls.get(), addObject: url];
            }
        }

        let contexts: Id = msg_send![connection_options, URLContexts];
        for url_context in ns_set_items(contexts) {
            let url: Id = msg_send![url_context, URL];
            let () = msg_send![launch_urls.get(), addObject: url];
        }
    }

    crate::init::set_main_ready();
    perform_later(this, sel!(postFinishLaunch));
}

/// Translation of `-[SDLUIKitSceneDelegate windowScene:didUpdateEffectiveGeometry:]`
/// (iOS 26; acted on from iOS 27).
#[cfg(not(any(target_os = "tvos", target_os = "visionos")))]
extern "C" fn scene_did_update_effective_geometry(
    _this: Id,
    _sel: Sel,
    window_scene: Id,
    previous_effective_geometry: Id,
) {
    if available!(ios(27, 0)) {
        // SAFETY: UIWindowSceneGeometry properties.
        let orientationchanged = unsafe {
            let geometry: Id = msg_send![window_scene, effectiveGeometry];
            let now: UIInterfaceOrientation = msg_send![geometry, interfaceOrientation];
            let before: UIInterfaceOrientation =
                msg_send![previous_effective_geometry, interfaceOrientation];
            now != before
        };
        if orientationchanged {
            let _ = crate::video::on_application_did_change_status_bar_orientation();
        }
    }
}

/// Translation of `-[SDLUIKitSceneDelegate scene:openURLContexts:]`.
extern "C" fn scene_open_url_contexts(this: Id, _sel: Sel, _scene: Id, url_contexts: Id) {
    // SAFETY: an NSSet of UIOpenURLContext, with their NSURLs.
    unsafe {
        for context in ns_set_items(url_contexts) {
            let url: Id = msg_send![context, URL];
            let () = msg_send![this, handleURL: url];
        }
    }
}

/// Translation of `-[SDLUIKitSceneDelegate sceneDidBecomeActive:]`.
extern "C" fn scene_did_become_active(_this: Id, _sel: Sel, _scene: Id) {
    crate::video::on_application_did_enter_foreground();
}

/// Translation of `-[SDLUIKitSceneDelegate sceneWillResignActive:]`.
extern "C" fn scene_will_resign_active(_this: Id, _sel: Sel, _scene: Id) {
    crate::video::on_application_will_enter_background();
}

/// Translation of `-[SDLUIKitSceneDelegate sceneWillEnterForeground:]`.
extern "C" fn scene_will_enter_foreground(_this: Id, _sel: Sel, _scene: Id) {
    crate::video::on_application_will_enter_foreground();
}

/// Translation of `-[SDLUIKitSceneDelegate sceneDidEnterBackground:]`.
extern "C" fn scene_did_enter_background(_this: Id, _sel: Sel, _scene: Id) {
    crate::video::on_application_did_enter_background();
}

/// Translation of `-[SDLUIKitSceneDelegate handleURL:]`.
extern "C" fn scene_handle_url(_this: Id, _sel: Sel, url: Id) {
    send_drop_file_for_url(url, None);
}

/// Translation of `-[SDLUIKitSceneDelegate hideLaunchScreen]`.
extern "C" fn scene_hide_launch_screen(this: Id, _sel: Sel) {
    hide_launch_screen(this, scene_delegate_class());
}

/// Translation of `-[SDLUIKitSceneDelegate postFinishLaunch]`.
extern "C" fn scene_post_finish_launch(this: Id, _sel: Sel) {
    perform_later(this, sel!(hideLaunchScreen));
    perform_later(this, sel!(processLaunchURLs));

    run_main_function(this, scene_delegate_class());
}

/// Translation of `-[SDLUIKitSceneDelegate processLaunchURLs]`.
extern "C" fn scene_process_launch_urls(this: Id, _sel: Sel) {
    let cls = scene_delegate_class();
    // SAFETY: the NSMutableArray of NSURLs (kept while it's walked).
    unsafe {
        let launch_urls = StrongPtr::retain(get_ivar(this, cls, c"launchURLs"));
        for url in ns_array_items(launch_urls.get()) {
            let () = msg_send![this, handleURL: url];
        }
    }
    set_ivar(this, cls, c"launchURLs", NIL);
}

/// Translation of `-[SDLUIKitSceneDelegate
/// application:configurationForConnectingSceneSession:options:]`.
extern "C" fn scene_configuration(
    _this: Id,
    _sel: Sel,
    _application: Id,
    connecting_scene_session: Id,
    _options: Id,
) -> Id {
    // This doesn't appear to be called, but it needs to be implemented to signal that we support the UIScene life cycle
    // SAFETY: UISceneConfiguration's initializer (owned, answered
    // autoreleased), with the session's role.
    unsafe {
        let name = ns_string("SDLSceneConfiguration");
        let role: Id = msg_send![connecting_scene_session, role];
        let config: Id = msg_send![class!(UISceneConfiguration), alloc];
        let config: Id = msg_send![config, initWithName: name.get() sessionRole: role];
        let () = msg_send![config, setDelegateClass: scene_delegate_class()];
        crate::core::apple::objc::autorelease(config)
    }
}

// ---------------------------------------------------------------------------
// SDLUIKitDelegate

static SDL_UIKIT_DELEGATE: DeclaredClass = DeclaredClass::new();

/// The `SDLUIKitDelegate` class (`@interface SDLUIKitDelegate : NSObject
/// <UIApplicationDelegate>`, the app delegate before iOS 13), with its
/// `launchWindow` instance variable.
pub(crate) fn app_delegate_class() -> Class {
    SDL_UIKIT_DELEGATE.get(|| {
        let mut b =
            ClassBuilder::new(c"SDLUIKitDelegate", class!(NSObject)).expect("SDLUIKitDelegate");
        b.add_protocol(c"UIApplicationDelegate");
        b.add_ivar::<Id>(c"launchWindow", c"@");
        // SAFETY: the implementations have the signatures of the encodings.
        unsafe {
            b.add_class_method(
                sel!(sharedAppDelegate),
                imp!(delegate_shared_app_delegate as extern "C" fn(Id, Sel) -> Id),
                c"@@:",
            );
            b.add_class_method(
                sel!(getAppDelegateClassName),
                imp!(delegate_get_class_name as extern "C" fn(Id, Sel) -> Id),
                c"@@:",
            );
            b.add_method(
                sel!(dealloc),
                imp!(delegate_dealloc as extern "C" fn(Id, Sel)),
                c"v@:",
            );
            b.add_method(
                sel!(hideLaunchScreen),
                imp!(delegate_hide_launch_screen as extern "C" fn(Id, Sel)),
                c"v@:",
            );
            b.add_method(
                sel!(postFinishLaunch),
                imp!(delegate_post_finish_launch as extern "C" fn(Id, Sel)),
                c"v@:",
            );
            b.add_method(
                sel!(application:didFinishLaunchingWithOptions:),
                imp!(delegate_did_finish_launching as extern "C" fn(Id, Sel, Id, Id) -> Bool),
                &encoding("B@:@@"),
            );
            b.add_method(
                sel!(window),
                imp!(delegate_window as extern "C" fn(Id, Sel) -> Id),
                c"@@:",
            );
            b.add_method(
                sel!(setWindow:),
                imp!(delegate_set_window as extern "C" fn(Id, Sel, Id)),
                c"v@:@",
            );
            b.add_method(
                sel!(sendDropFileForURL:fromSourceApplication:),
                imp!(delegate_send_drop_file_for_url as extern "C" fn(Id, Sel, Id, Id)),
                c"v@:@@",
            );
            b.add_method(
                sel!(application:openURL:options:),
                imp!(delegate_open_url as extern "C" fn(Id, Sel, Id, Id, Id) -> Bool),
                &encoding("B@:@@@"),
            );
        }
        b.register()
    })
}

/// convenience method. Translation of `+[SDLUIKitDelegate sharedAppDelegate]`.
extern "C" fn delegate_shared_app_delegate(_cls: Id, _sel: Sel) -> Id {
    /* the delegate is set in UIApplicationMain(), which is guaranteed to be
     * called before this method */
    // SAFETY: a UIApplication property.
    unsafe { msg_send![crate::core::apple::uikit::shared_application(), delegate] }
}

/// Translation of `+[SDLUIKitDelegate getAppDelegateClassName]`.
extern "C" fn delegate_get_class_name(_cls: Id, _sel: Sel) -> Id {
    /* subclassing notice: when you subclass this appdelegate, make sure to add
     * a category to override this method and return the actual name of the
     * delegate */
    ns_string("SDLUIKitDelegate").autorelease()
}

/// `-[SDLUIKitDelegate dealloc]` (ARC's).
extern "C" fn delegate_dealloc(this: Id, _sel: Sel) {
    let cls = app_delegate_class();
    set_ivar(this, cls, c"launchWindow", NIL);
    // SAFETY: NSObject's dealloc.
    let () = unsafe { msg_send_super![this, cls, dealloc] };
}

/// Translation of `-[SDLUIKitDelegate hideLaunchScreen]`.
extern "C" fn delegate_hide_launch_screen(this: Id, _sel: Sel) {
    hide_launch_screen(this, app_delegate_class());
}

/// Translation of `-[SDLUIKitDelegate postFinishLaunch]`.
extern "C" fn delegate_post_finish_launch(this: Id, _sel: Sel) {
    /* Hide the launch screen the next time the run loop is run. SDL apps will
     * have a chance to load resources while the launch screen is still up. */
    perform_later(this, sel!(hideLaunchScreen));

    // run the user's application, passing argc and argv
    run_main_function(this, app_delegate_class());

    // exit, passing the return status from the user's application
    /* We don't actually exit to support applications that do setup in their
     * main function and then allow the Cocoa event loop to run. */
    // exit(exit_status);
}

/// Translation of `-[SDLUIKitDelegate application:didFinishLaunchingWithOptions:]`.
extern "C" fn delegate_did_finish_launching(
    this: Id,
    _sel: Sel,
    _application: Id,
    _launch_options: Id,
) -> Bool {
    let bundle = main_bundle();

    /* The normal launch screen is displayed until didFinishLaunching returns,
     * but SDL_main is called after that happens and there may be a noticeable
     * delay between the start of SDL_main and when the first real frame is
     * displayed (e.g. if resources are loaded before SDL_GL_SwapWindow is
     * called), so we show the launch screen programmatically until the first
     * time events are pumped. */
    let vc = launch_view_controller(bundle);

    // SAFETY: a UIViewController property (nil for nil).
    let vc_view: Id = unsafe { msg_send![vc.get(), view] };
    if !vc_view.is_null() {
        #[cfg(target_os = "visionos")]
        let view_frame = CGRect::new(
            0.0,
            0.0,
            super::modes::SDL_XR_SCREENWIDTH as f64,
            super::modes::SDL_XR_SCREENHEIGHT as f64,
        );
        // SAFETY: UIScreen's main screen's bounds.
        #[cfg(not(target_os = "visionos"))]
        let view_frame: CGRect = unsafe { msg_send![super::modes::main_screen(), bounds] };
        let launch_window = show_launch_window(&vc, view_frame, || {
            // SAFETY: UIWindow's initializer (owned).
            unsafe {
                let window: Id = msg_send![class!(UIWindow), alloc];
                StrongPtr::from_owned(msg_send![window, initWithFrame: view_frame])
            }
        });
        set_ivar(
            this,
            app_delegate_class(),
            c"launchWindow",
            launch_window.get(),
        );
    }

    change_to_resource_directory(bundle);

    crate::init::set_main_ready();
    perform_later(this, sel!(postFinishLaunch));

    YES
}

/// Translation of `-[SDLUIKitDelegate window]`: the first SDL window's
/// `UIWindow`. This property is marked as optional, and is only intended
/// to be used when the app's UI is storyboard-based. SDL is not
/// storyboard-based, however several major third-party ad APIs (e.g.
/// Google admob) incorrectly assume this property always exists, and will
/// crash if it doesn't.
extern "C" fn delegate_window(_this: Id, _sel: Sel) -> Id {
    // (the windows, the most recently made first; none without video)
    for window in crate::video::core::window_ids() {
        if let Some(data) = super::window::window_data(window) {
            return data.uiwindow();
        }
    }
    NIL
}

/// Translation of `-[SDLUIKitDelegate setWindow:]`.
extern "C" fn delegate_set_window(_this: Id, _sel: Sel, _window: Id) {
    // Do nothing.
}

/// Translation of `-[SDLUIKitDelegate sendDropFileForURL:fromSourceApplication:]`.
extern "C" fn delegate_send_drop_file_for_url(
    _this: Id,
    _sel: Sel,
    url: Id,
    source_application: Id,
) {
    // SAFETY: an NSString (or nil).
    let source = unsafe { string_from_ns(source_application) };
    send_drop_file_for_url(url, source.as_deref());
}

/// Translation of `-[SDLUIKitDelegate application:openURL:options:]`.
extern "C" fn delegate_open_url(this: Id, _sel: Sel, _app: Id, url: Id, _options: Id) -> Bool {
    // TODO: Handle options
    // SAFETY: this class's method.
    unsafe {
        let () = msg_send![this, sendDropFileForURL: url fromSourceApplication: NIL];
    }
    YES
}
