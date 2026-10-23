// Rust translation of src/notification/cocoa/SDL_cocoanotification.m from
// Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! macOS notifications through UserNotifications (`UNUserNotificationCenter`),
//! with an `SDLNotificationDelegate` (a class declared at run time) that
//! shows them while the app is in front and turns the user's responses
//! into `NOTIFICATION_ACTION_INVOKED` events.
//!
//! UserNotifications only works in an application bundle (outside of one,
//! it raises an exception deep inside the system), so outside of one the
//! notification center is never set up, and the functions report that they
//! are unsupported there, as upstream does. The delegate is registered when
//! a subsystem is initialized ([`register_notification_delegate`]).

use std::ffi::c_void;
use std::sync::Mutex;

use super::{Notification, NotificationAction, NotificationID, NotificationPriority};
use crate::core::apple::block::RcBlock;
use crate::core::apple::foundation::{
    available, ns_string, string_from_ns, CFBundleGetMainBundle, CFRelease, CFStringRef, CFTypeRef,
    CFURLRef, CfRef,
};
use crate::core::apple::objc::{
    autoreleasepool, class, imp, msg_send, sel, Bool, ClassBuilder, DeclaredClass, Id, NSInteger,
    NSUInteger, Sel, StrongPtr,
};
use crate::error::{Error, Result};
use crate::video::surface::Surface;

/// `UNNotificationPresentationOptions`.
type UNNotificationPresentationOptions = NSUInteger;
/// `UNNotificationPresentationOptionSound`.
const UN_NOTIFICATION_PRESENTATION_OPTION_SOUND: UNNotificationPresentationOptions = 1 << 1;
/// `UNNotificationPresentationOptionAlert`.
const UN_NOTIFICATION_PRESENTATION_OPTION_ALERT: UNNotificationPresentationOptions = 1 << 2;
/// `UNNotificationPresentationOptionBanner`.
const UN_NOTIFICATION_PRESENTATION_OPTION_BANNER: UNNotificationPresentationOptions = 1 << 4;

/// `UNAuthorizationStatusNotDetermined`.
const UN_AUTHORIZATION_STATUS_NOT_DETERMINED: NSInteger = 0;
/// `UNAuthorizationOptionSound`.
const UN_AUTHORIZATION_OPTION_SOUND: NSUInteger = 1 << 1;
/// `UNAuthorizationOptionAlert`.
const UN_AUTHORIZATION_OPTION_ALERT: NSUInteger = 1 << 2;

/// `UNNotificationActionOptionNone`.
const UN_NOTIFICATION_ACTION_OPTION_NONE: NSUInteger = 0;
/// `UNNotificationCategoryOptionNone`.
const UN_NOTIFICATION_CATEGORY_OPTION_NONE: NSUInteger = 0;

/// `UNNotificationInterruptionLevelPassive`.
const UN_NOTIFICATION_INTERRUPTION_LEVEL_PASSIVE: NSUInteger = 0;
/// `UNNotificationInterruptionLevelActive`.
const UN_NOTIFICATION_INTERRUPTION_LEVEL_ACTIVE: NSUInteger = 1;
/// `UNNotificationInterruptionLevelCritical`.
const UN_NOTIFICATION_INTERRUPTION_LEVEL_CRITICAL: NSUInteger = 3;

/// `errSecSuccess`.
const ERR_SEC_SUCCESS: i32 = 0;

/// The prefix of the identifiers of SDL's notification requests.
const SDL_IDENTIFIER: &str = "SDL_LocalNotification-";

#[link(name = "UserNotifications", kind = "framework")]
unsafe extern "C" {
    static UNNotificationDefaultActionIdentifier: Id;
}

#[link(name = "CoreServices", kind = "framework")]
unsafe extern "C" {
    static kUTTypeApplicationBundle: CFStringRef;
    fn UTTypeConformsTo(in_uti: CFStringRef, in_conforms_to_uti: CFStringRef) -> u8;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFURLTypeIdentifierKey: CFStringRef;
    fn CFBundleCopyBundleURL(bundle: *mut c_void) -> CFURLRef;
    fn CFURLCopyResourcePropertyForKey(
        url: CFURLRef,
        key: CFStringRef,
        property_value_type_ref_ptr: *mut CFTypeRef,
        error: *mut *mut c_void,
    ) -> u8;
}

#[link(name = "Security", kind = "framework")]
unsafe extern "C" {
    static kSecRandomDefault: *const c_void;
    fn SecRandomCopyBytes(rnd: *const c_void, count: usize, bytes: *mut c_void) -> i32;
}

/// The head of `Block_layout`, with the invoke function of type `F`.
#[repr(C)]
struct BlockHead<F> {
    isa: *const c_void,
    flags: i32,
    reserved: i32,
    invoke: F,
}

/// Call a block taking one argument (`completionHandler(value)`).
///
/// # Safety
///
/// `block` must be a block taking an `A` and returning nothing.
unsafe fn call_block1<A>(block: *mut c_void, value: A) {
    type Invoke<A> = unsafe extern "C" fn(*mut c_void, A);
    // SAFETY: the caller's contract; a block's invoke function takes the
    // block itself first.
    unsafe { ((*(block as *mut BlockHead<Invoke<A>>)).invoke)(block, value) }
}

/// Call a block taking no argument (`completionHandler()`).
///
/// # Safety
///
/// `block` must be a block taking nothing and returning nothing.
unsafe fn call_block0(block: *mut c_void) {
    type Invoke = unsafe extern "C" fn(*mut c_void);
    // SAFETY: as above.
    unsafe { ((*(block as *mut BlockHead<Invoke>)).invoke)(block) }
}

// * * * SDLNotificationDelegate

/// Translation of
/// `-[SDLNotificationDelegate userNotificationCenter:willPresentNotification:withCompletionHandler:]`.
extern "C" fn will_present_notification(
    _this: Id,
    _sel: Sel,
    _center: Id,
    _notification: Id,
    completion_handler: *mut c_void,
) {
    let options = if available!(macos(11), ios(14)) {
        UN_NOTIFICATION_PRESENTATION_OPTION_BANNER + UN_NOTIFICATION_PRESENTATION_OPTION_SOUND
    } else {
        UN_NOTIFICATION_PRESENTATION_OPTION_ALERT + UN_NOTIFICATION_PRESENTATION_OPTION_SOUND
    };
    // SAFETY: the system's completion handler takes the presentation options.
    unsafe { call_block1(completion_handler, options) };
}

/// The notification ID in an identifier `SDL_LocalNotification-<id>`
/// (`integerValue` of what follows, so 0 when it isn't one of SDL's).
fn notification_id_from_identifier(identifier: &str) -> NotificationID {
    match identifier.strip_prefix(SDL_IDENTIFIER) {
        // (integerValue: leading digits, 0 when there are none; the
        // truncation to SDL_NotificationID as upstream's cast)
        Some(rest) => {
            let rest = rest.trim_start();
            let (sign, digits) = match rest.strip_prefix('-') {
                Some(d) => (-1i64, d),
                None => (1, rest.strip_prefix('+').unwrap_or(rest)),
            };
            let digits: String = digits.chars().take_while(char::is_ascii_digit).collect();
            let value =
                digits
                    .parse::<i64>()
                    .unwrap_or(if digits.is_empty() { 0 } else { i64::MAX });
            (sign * value) as NotificationID
        }
        None => 0,
    }
}

/// Translation of
/// `-[SDLNotificationDelegate userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:]`.
extern "C" fn did_receive_notification_response(
    _this: Id,
    _sel: Sel,
    _center: Id,
    response: Id,
    completion_handler: *mut c_void,
) {
    autoreleasepool(|| {
        // SAFETY: the response's notification, request and identifiers are
        // NSStrings.
        unsafe {
            let notification: Id = msg_send![response, notification];
            let request: Id = msg_send![notification, request];
            let identifier: Id = msg_send![request, identifier];
            let identifier = string_from_ns(identifier).unwrap_or_default();
            // const char *identifier = [[[[response notification] request] identifier] UTF8String];
            let id = notification_id_from_identifier(&identifier);

            if id != 0 {
                let action_id: Id = msg_send![response, actionIdentifier];
                if !action_id.is_null() {
                    let is_default: Bool = msg_send![action_id, isEqualToString: UNNotificationDefaultActionIdentifier];
                    if is_default.is_true() {
                        crate::events::window::send_notification_action(id, "default");
                    } else {
                        let length: NSUInteger = msg_send![action_id, length];
                        if length != 0 {
                            let action_id = string_from_ns(action_id).unwrap_or_default();
                            crate::events::window::send_notification_action(id, &action_id);
                        }
                    }
                }
            }
        }
    });

    // SAFETY: the system's completion handler takes nothing.
    unsafe { call_block0(completion_handler) };
}

static DELEGATE_CLASS: DeclaredClass = DeclaredClass::new();

/// The `SDLNotificationDelegate` class (`<UNUserNotificationCenterDelegate>`).
fn delegate_class() -> crate::core::apple::objc::Class {
    DELEGATE_CLASS.get(|| {
        let mut b = ClassBuilder::new(c"SDLNotificationDelegate", class!(NSObject))
            .expect("SDLNotificationDelegate is declared once");
        b.add_protocol(c"UNUserNotificationCenterDelegate");
        // SAFETY: the implementations have the signatures the encodings say.
        unsafe {
            b.add_method(
                sel!(userNotificationCenter:willPresentNotification:withCompletionHandler:),
                imp!(will_present_notification as extern "C" fn(Id, Sel, Id, Id, *mut c_void)),
                c"v@:@@@?",
            );
            b.add_method(
                sel!(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:),
                imp!(
                    did_receive_notification_response
                        as extern "C" fn(Id, Sel, Id, Id, *mut c_void)
                ),
                c"v@:@@@?",
            );
        }
        b.register()
    })
}

/// Translation of `center` and `delegate`.
struct State {
    center: StrongPtr,
    delegate: StrongPtr,
}

static STATE: Mutex<State> = Mutex::new(State {
    center: StrongPtr::nil(),
    delegate: StrongPtr::nil(),
});

fn state() -> std::sync::MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Translation of `ShouldEnableNotifications()`.
fn should_enable_notifications() -> bool {
    /* Notifications outside of an app bundle are unsupported, and will crash with an
     * unhandled exception error, deep within a system library.
     *
     * FIXME: These functions are deprecated, find a modern way.
     */
    // SAFETY: CoreFoundation and LaunchServices queries; what they create
    // is released here.
    unsafe {
        let bundle = CFBundleGetMainBundle();
        // Note (upstream): the bundle URL and the type identifier are
        // leaked; they are released here.
        let Some(bundle_url) = CfRef::from_owned(CFBundleCopyBundleURL(bundle)) else {
            return false;
        };

        let mut uti: CFTypeRef = std::ptr::null();
        let ok = CFURLCopyResourcePropertyForKey(
            bundle_url.get(),
            kCFURLTypeIdentifierKey,
            &mut uti,
            std::ptr::null_mut(),
        ) != 0;
        let conforms = ok && !uti.is_null() && UTTypeConformsTo(uti, kUTTypeApplicationBundle) != 0;
        if !uti.is_null() {
            CFRelease(uti);
        }
        conforms
    }
}

/// The notification center, when notifications are set up.
fn center() -> Option<StrongPtr> {
    let state = state();
    (!state.center.is_nil()).then(|| state.center.clone())
}

/// Translation of `SaveTempImage()`.
fn save_temp_image(image: &Surface<'_>) -> Option<StrongPtr> {
    autoreleasepool(|| {
        let pixels = image.pixels()?;
        let len = (image.pitch() as usize * image.height() as usize).min(pixels.len());
        let hash = crate::stdlib::murmur3::murmur3_32(&pixels[..len], 0);
        // SAFETY: Foundation's temporary directory and path functions.
        let path = unsafe {
            let temp_file_name = ns_string(&format!("SDL_tmpimage-{hash}.png"));
            let temp_dir: Id = NSTemporaryDirectory();
            let temp_file_path: Id =
                msg_send![temp_dir, stringByAppendingPathComponent: temp_file_name.get()];
            StrongPtr::retain(temp_file_path)
        };

        // SAFETY: an NSString.
        let file = unsafe { string_from_ns(path.get()) }?;
        image.duplicate().ok()?.save_png(&file).ok()?;

        // SAFETY: NSURL's class method, with an NSString path.
        unsafe {
            let url: Id = msg_send![class!(NSURL), fileURLWithPath: path.get()];
            Some(StrongPtr::retain(url))
        }
    })
}

#[link(name = "Foundation", kind = "framework")]
unsafe extern "C" {
    fn NSTemporaryDirectory() -> Id;
}

/// Register the notification delegate early, as Apple platforms require.
/// Translation of `Cocoa_RegisterNotificationDelegate()`.
pub(crate) fn register_notification_delegate() {
    if !should_enable_notifications() {
        return;
    }

    if class!(UNUserNotificationCenter).is_null() {
        // (macOS before 10.14)
        return;
    }

    autoreleasepool(|| {
        let mut state = state();
        // SAFETY: the current notification center, and a new delegate it
        // keeps unretained (the state keeps it).
        unsafe {
            if state.center.is_nil() {
                state.center = StrongPtr::retain(msg_send![
                    class!(UNUserNotificationCenter),
                    currentNotificationCenter
                ]);
            }
            if state.delegate.is_nil() {
                state.delegate = StrongPtr::from_owned(msg_send![delegate_class(), new]);
                let _: () = msg_send![state.center, setDelegate: state.delegate.get()];
            }
        }
    });
}

/// Translation of `SDL_RequestNotificationPermission()`.
pub(super) fn request_notification_permission() -> Result<()> {
    autoreleasepool(|| {
        // Notifications not initialized (not in a bundle).
        let Some(center) = center() else {
            return Err(Error::new(
                "macOS notifications not supported outside an application bundle",
            ));
        };

        // Check authorization to send notifications, and request it if necessary.
        let c = center.clone();
        let handler = RcBlock::new1(move |settings: Id| {
            // SAFETY: the settings the system passes, and the center.
            unsafe {
                let status: NSInteger = msg_send![settings, authorizationStatus];
                if status == UN_AUTHORIZATION_STATUS_NOT_DETERMINED {
                    let options = UN_AUTHORIZATION_OPTION_ALERT + UN_AUTHORIZATION_OPTION_SOUND;
                    let done = RcBlock::new2(|_granted: Bool, _error: Id| {});
                    let _: () = msg_send![c,
                        requestAuthorizationWithOptions: options
                        completionHandler: done.as_ptr()];
                }
            }
        });
        // SAFETY: the center copies the block.
        let _: () = unsafe {
            msg_send![center, getNotificationSettingsWithCompletionHandler: handler.as_ptr()]
        };

        Ok(())
    })
}

/// `@[ identifier ]` for SDL's notification `id`.
///
/// This function assumes that it's called from within an autorelease pool.
fn identifier_array(id: NotificationID) -> Id {
    let identifier = ns_string(&format!("{SDL_IDENTIFIER}{id}"));
    // SAFETY: NSArray's class method.
    unsafe { msg_send![class!(NSArray), arrayWithObject: identifier.get()] }
}

/// Translation of `SDL_RemoveNotification()`.
pub(super) fn remove_notification(notification: NotificationID) -> Result<()> {
    autoreleasepool(|| {
        // Notifications not initialized (not in a bundle).
        let Some(center) = center() else {
            return Err(Error::new(
                "macOS notifications not supported outside an application bundle",
            ));
        };

        let identifiers = identifier_array(notification);
        // SAFETY: the center, with an array of NSStrings.
        unsafe {
            let _: () =
                msg_send![center, removePendingNotificationRequestsWithIdentifiers: identifiers];
            let _: () = msg_send![center, removeDeliveredNotificationsWithIdentifiers: identifiers];
        }
        Ok(())
    })
}

/// A random notification ID (`SecRandomCopyBytes()`), or the low bits of
/// the tick count when there is no randomness.
fn new_notification_id() -> NotificationID {
    let mut new_id: NotificationID = 0;
    // SAFETY: four bytes into a u32.
    let r = unsafe {
        SecRandomCopyBytes(
            kSecRandomDefault,
            size_of::<NotificationID>(),
            &mut new_id as *mut NotificationID as *mut c_void,
        )
    };
    if r != ERR_SEC_SUCCESS {
        new_id = crate::timer::ticks_ns() as NotificationID;
    }
    new_id
}

/// Translation of `SDL_SYS_ShowNotification()`.
pub(super) fn show_notification(notification: &Notification<'_>) -> Result<NotificationID> {
    request_notification_permission()?;
    let center = center().ok_or_else(|| {
        Error::new("macOS notifications not supported outside an application bundle")
    })?;

    autoreleasepool(|| {
        // Get the notification properties.
        let title = notification.title.as_deref().unwrap_or("");
        let message = notification.message.as_deref().unwrap_or("");
        let sound = notification.sound.as_deref().unwrap_or("default");
        let image = notification.image;
        let replaces = notification.replaces.unwrap_or(0);
        let priority = notification.priority;
        let sdlactions = &notification.actions;
        let transient = notification.transient;

        // Generate a new ID.
        let new_id = if replaces != 0 {
            replaces
        } else {
            new_notification_id()
        };

        // SAFETY: the UserNotifications objects as documented; the arrays
        // hold the types their methods take.
        unsafe {
            // Build the action array.
            let mut actions: Id = std::ptr::null_mut();
            if !sdlactions.is_empty() {
                actions = msg_send![class!(NSMutableArray), array];
                for (i, a) in sdlactions.iter().enumerate() {
                    match a {
                        NotificationAction::Button {
                            action_id,
                            action_label,
                        } => {
                            let action: Id = msg_send![class!(UNNotificationAction),
                                actionWithIdentifier: ns_string(action_id).get()
                                title: ns_string(action_label).get()
                                options: UN_NOTIFICATION_ACTION_OPTION_NONE];
                            let _: () = msg_send![actions,
                                setObject: action
                                atIndexedSubscript: i as NSUInteger];
                        }
                    }
                }
            }

            // Create the category.
            let mut category_id: Id = std::ptr::null_mut();
            let count: NSUInteger = if actions.is_null() {
                0
            } else {
                msg_send![actions, count]
            };
            if count != 0 {
                // Create the notification category.
                let uuid = StrongPtr::from_owned(msg_send![class!(NSUUID), new]);
                category_id = msg_send![uuid, UUIDString];
                let no_intents: Id = msg_send![class!(NSArray), array];
                let category: Id = msg_send![class!(UNNotificationCategory),
                    categoryWithIdentifier: category_id
                    actions: actions
                    intentIdentifiers: no_intents
                    options: UN_NOTIFICATION_CATEGORY_OPTION_NONE];
                let categories: Id = msg_send![class!(NSSet), setWithObject: category];
                let _: () = msg_send![center, setNotificationCategories: categories];
            }

            // Configure the content.
            let content =
                StrongPtr::from_owned(msg_send![class!(UNMutableNotificationContent), new]);
            let _: () = msg_send![content, setTitle: ns_string(title).get()];
            let _: () = msg_send![content, setBody: ns_string(message).get()];
            let _: () = msg_send![content, setCategoryIdentifier: category_id];

            let sound_class = class!(UNNotificationSound);
            let critical = priority == NotificationPriority::Critical;
            if sound == "default" {
                // defaultCriticalSound is only in iOS 12+
                // (upstream's @available(iOS 12, *) is true on macOS)
                let s: Id = if critical {
                    msg_send![sound_class, defaultCriticalSound]
                } else {
                    msg_send![sound_class, defaultSound]
                };
                let _: () = msg_send![content, setSound: s];
            } else if sound != "silent" {
                let name = ns_string(sound);
                let s: Id = if critical {
                    msg_send![sound_class, criticalSoundNamed: name.get()]
                } else {
                    msg_send![sound_class, soundNamed: name.get()]
                };
                let _: () = msg_send![content, setSound: s];
            }

            if available!(macos(12), ios(15)) {
                let level = match priority {
                    NotificationPriority::Low => UN_NOTIFICATION_INTERRUPTION_LEVEL_PASSIVE,
                    NotificationPriority::Critical => UN_NOTIFICATION_INTERRUPTION_LEVEL_CRITICAL,
                    NotificationPriority::Normal | NotificationPriority::High => {
                        UN_NOTIFICATION_INTERRUPTION_LEVEL_ACTIVE
                    }
                };
                let _: () = msg_send![content, setInterruptionLevel: level];
            }

            // Notifications load images from file paths, so save it to a temporary location.
            if let Some(image) = image {
                if let Some(url) = save_temp_image(image) {
                    let attach: Id = msg_send![class!(UNNotificationAttachment),
                        attachmentWithIdentifier: ns_string("").get()
                        URL: url.get()
                        options: std::ptr::null_mut::<c_void>()
                        error: std::ptr::null_mut::<Id>()];
                    let attachments: Id = msg_send![class!(NSArray), arrayWithObject: attach];
                    let _: () = msg_send![content, setAttachments: attachments];
                }
            }

            let identifier = ns_string(&format!("{SDL_IDENTIFIER}{new_id}"));
            let request: Id = msg_send![class!(UNNotificationRequest),
                requestWithIdentifier: identifier.get()
                content: content.get()
                trigger: std::ptr::null_mut::<c_void>()];

            let done = RcBlock::new1(|error: Id| {
                if !error.is_null() {
                    // (SDL_SetError() from the completion handler: nothing
                    // reads it)
                    let _ = Error::new("Failed to show notification");
                }
            });
            let _: () = msg_send![center,
                addNotificationRequest: request
                withCompletionHandler: done.as_ptr()];

            /* There is no way to tell if the notification was ignored or wound up
             * in the notification center, so just remove transient notifications
             * after a brief period via a timer.
             */
            if transient {
                let remove = RcBlock::new1(move |_timer: Id| {
                    let _ = remove_notification(new_id);
                });
                let _: Id = msg_send![class!(NSTimer),
                    scheduledTimerWithTimeInterval: 7.0f64
                    repeats: Bool::NO
                    block: remove.as_ptr()];
            }
        }

        Ok(new_id)
    })
}

/// Translation of `SDL_CleanupNotifications()`.
pub(super) fn cleanup_notifications() {
    // TODO: Anything to do here?
}

/// (no activation tokens on macOS)
pub(super) fn notification_activation_token() -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers() {
        assert_eq!(
            notification_id_from_identifier("SDL_LocalNotification-42"),
            42
        );
        assert_eq!(
            notification_id_from_identifier("SDL_LocalNotification-4294967295"),
            u32::MAX
        );
        assert_eq!(
            notification_id_from_identifier("SDL_LocalNotification-x"),
            0
        );
        assert_eq!(notification_id_from_identifier("Other-42"), 0);
        assert_eq!(
            notification_id_from_identifier("SDL_LocalNotification-7abc"),
            7
        );
    }

    /// A test binary isn't an application bundle: no notification center,
    /// and the functions say why.
    #[test]
    fn unsupported_outside_a_bundle() {
        assert!(!should_enable_notifications());
        register_notification_delegate();
        let msg = "macOS notifications not supported outside an application bundle";
        assert_eq!(
            request_notification_permission().unwrap_err().to_string(),
            msg
        );
        assert_eq!(remove_notification(1).unwrap_err().to_string(), msg);
        let n = Notification {
            title: Some("t".into()),
            ..Notification::default()
        };
        assert_eq!(show_notification(&n).unwrap_err().to_string(), msg);
    }

    #[test]
    fn delegate_class_methods() {
        let cls = delegate_class();
        assert!(crate::core::apple::objc::class_responds_to(
            cls,
            sel!(userNotificationCenter:willPresentNotification:withCompletionHandler:)
        ));
    }
}
