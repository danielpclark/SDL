// Rust translation of the iOS-specific section of
// src/audio/coreaudio/SDL_coreaudio.m from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The CoreAudio driver's `AVAudioSession` management on the UIKit
//! platforms (upstream's `!MACOSX_COREAUDIO` parts):
//!
//! * `UpdateAudioSession()`, run when a device opens and when one closes:
//!   the session's category and options, from
//!   [`hints::AUDIO_CATEGORY`](crate::hints::AUDIO_CATEGORY),
//!   [`hints::AUDIO_DUCK_OTHERS`](crate::hints::AUDIO_DUCK_OTHERS) and what
//!   is open (play and record when both a playback and a recording device
//!   are), set with the open devices' queues paused and the session
//!   inactive, then the session activated (the queues resumed) while a
//!   device is open;
//! * the session's preferred sample rate, the device's own rate then
//!   (`OpenDevice`), and on tvOS its channel counts;
//! * `SDLInterruptionListener`: an `AVAudioSessionInterruptionNotification`
//!   that begins pauses the device's queue, one that ends (or the app
//!   becoming active, or coming to the foreground) starts it again.
//!
//! Upstream finds the open devices (`CountOpenAudioDevices()`,
//! `PauseAudioDevices()`, `ResumeAudioDevices()`) among all physical
//! devices, as those whose `hidden` data exists, from the start of
//! `OpenDevice` to the end of `CloseDevice`; here each device's data is
//! listed while it exists (the same span), as the core's devices aren't
//! reachable from inside an open (their locks are held). Note (upstream):
//! a closing device is still counted when `CloseDevice` updates the
//! session, so closing the last device leaves the session active, in the
//! category it had.
//!
//! The microphone permission has no handling of its own (as upstream): the
//! system asks for it when a recording queue starts, and without it the
//! queue records silence.
//!
//! AVFoundation is linked, as upstream links it.

use std::ffi::c_int;
use std::ptr;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use super::sys::{AudioQueuePause, AudioQueueStart};
use super::{CoreAudioDevice, Shared};
use crate::audio::device::PhysState;
use crate::core::apple::foundation::string_from_ns;
use crate::core::apple::objc::{
    autoreleasepool, class, imp, ivar_ptr, msg_send, sel, Bool, Class, ClassBuilder, DeclaredClass,
    Id, NSInteger, NSUInteger, Sel, StrongPtr, NIL,
};
use crate::error::{Error, Result};
use crate::stdlib::string::strcasecmp;

#[link(name = "AVFoundation", kind = "framework")]
unsafe extern "C" {
    static AVAudioSessionCategoryAmbient: Id;
    static AVAudioSessionCategorySoloAmbient: Id;
    static AVAudioSessionCategoryPlayback: Id;
    static AVAudioSessionCategoryRecord: Id;
    static AVAudioSessionCategoryPlayAndRecord: Id;
    static AVAudioSessionModeDefault: Id;
    static AVAudioSessionInterruptionNotification: Id;
    static AVAudioSessionInterruptionTypeKey: Id;
}

#[link(name = "UIKit", kind = "framework")]
unsafe extern "C" {
    static UIApplicationDidBecomeActiveNotification: Id;
    static UIApplicationWillEnterForegroundNotification: Id;
}

unsafe extern "C" {
    /// Foundation's version (`NSFoundationVersionNumber`).
    static NSFoundationVersionNumber: f64;
}

#[link(name = "objc")]
unsafe extern "C" {
    fn objc_sync_enter(obj: Id) -> c_int;
    fn objc_sync_exit(obj: Id) -> c_int;
}

/// `NSFoundationVersionNumber_iOS_7_1`.
const NS_FOUNDATION_VERSION_NUMBER_IOS_7_1: f64 = 1047.25;

/// `AVAudioSessionCategoryOptions` (`NSUInteger`).
pub(super) const AV_AUDIO_SESSION_CATEGORY_OPTION_MIX_WITH_OTHERS: NSUInteger = 0x1;
pub(super) const AV_AUDIO_SESSION_CATEGORY_OPTION_DUCK_OTHERS: NSUInteger = 0x2;
/// `AVAudioSessionCategoryOptionAllowBluetooth`, which upstream spells
/// `0x4` (see `session_category()`).
pub(super) const AV_AUDIO_SESSION_CATEGORY_OPTION_ALLOW_BLUETOOTH: NSUInteger = 0x4;
/// (not on tvOS)
#[cfg(not(target_os = "tvos"))]
pub(super) const AV_AUDIO_SESSION_CATEGORY_OPTION_DEFAULT_TO_SPEAKER: NSUInteger = 0x8;
pub(super) const AV_AUDIO_SESSION_CATEGORY_OPTION_ALLOW_BLUETOOTH_A2DP: NSUInteger = 0x20;
pub(super) const AV_AUDIO_SESSION_CATEGORY_OPTION_ALLOW_AIR_PLAY: NSUInteger = 0x40;

/// `AVAudioSessionInterruptionTypeBegan` (`Ended` is 0).
pub(super) const AV_AUDIO_SESSION_INTERRUPTION_TYPE_BEGAN: NSUInteger = 1;
/// `AVAudioSessionErrorCodeNone`.
const AV_AUDIO_SESSION_ERROR_CODE_NONE: i32 = 0;
/// `AVAudioSessionErrorCodeResourceNotAvailable` (`'!res'`).
pub(super) const AV_AUDIO_SESSION_ERROR_CODE_RESOURCE_NOT_AVAILABLE: NSInteger = 0x2172_6573;

/// Whether Foundation is iOS 7.1's or older (`PrepareAudioQueue()`'s
/// test for older iOS hardware).
pub(super) fn older_ios_hardware() -> bool {
    // SAFETY: Foundation's constant.
    crate::stdlib::math::floor(unsafe { NSFoundationVersionNumber })
        <= NS_FOUNDATION_VERSION_NUMBER_IOS_7_1
}

/// Translation of `session_active`.
static SESSION_ACTIVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The devices whose `hidden` exists (see the module notes).
static HIDDEN: Mutex<Vec<Arc<CoreAudioDevice>>> = Mutex::new(Vec::new());

fn hidden_devices() -> std::sync::MutexGuard<'static, Vec<Arc<CoreAudioDevice>>> {
    HIDDEN.lock().unwrap_or_else(|e| e.into_inner())
}

/// `device->hidden` was allocated (the start of `OpenDevice`).
pub(super) fn hidden_allocated(hidden: &Arc<CoreAudioDevice>) {
    hidden_devices().push(hidden.clone());
}

/// `SDL_free(device->hidden)` (the end of `CloseDevice`).
pub(super) fn free_hidden(hidden: &CoreAudioDevice) {
    hidden_devices().retain(|d| !ptr::eq(Arc::as_ptr(d), hidden));
}

/// The queue data of the devices whose `hidden` exists, once their queue
/// thread has some.
fn hidden_queues() -> Vec<Arc<Shared>> {
    hidden_devices()
        .iter()
        .filter_map(|d| d.shared.get().cloned())
        .collect()
}

/// Translation of `PauseOneAudioDevice()`.
fn pause_one_audio_device(shared: &Shared) {
    let queue = shared.queue();
    if !queue.is_null() && !shared.interrupted.load(Ordering::Acquire) {
        // SAFETY: a live queue of ours.
        unsafe { AudioQueuePause(queue) };
    }
}

/// Translation of `PauseAudioDevices()`.
fn pause_audio_devices() {
    for shared in hidden_queues() {
        pause_one_audio_device(&shared); // keep enumerating devices until we've paused them all.
    }
}

/// Translation of `ResumeOneAudioDevice()`.
fn resume_one_audio_device(shared: &Shared) {
    let queue = shared.queue();
    if !queue.is_null() && !shared.interrupted.load(Ordering::Acquire) {
        // SAFETY: a live queue of ours.
        unsafe { AudioQueueStart(queue, ptr::null()) };
    }
}

/// Translation of `ResumeAudioDevices()`.
fn resume_audio_devices() {
    for shared in hidden_queues() {
        resume_one_audio_device(&shared); // keep enumerating devices until we've resumed them all.
    }
}

/// Translation of `InterruptionBegin()`.
fn interruption_begin(device: Option<&Shared>) {
    if let Some(shared) = device {
        let queue = shared.queue();
        if !queue.is_null() {
            shared.interrupted.store(true, Ordering::Release);
            // SAFETY: a live queue of ours.
            unsafe { AudioQueuePause(queue) };
        }
    }
}

/// Translation of `InterruptionEnd()`.
fn interruption_end(device: Option<&Shared>) {
    if let Some(shared) = device {
        let queue = shared.queue();
        // SAFETY: a live queue of ours.
        if !queue.is_null()
            && shared.interrupted.load(Ordering::Acquire)
            && unsafe { AudioQueueStart(queue, ptr::null()) } == AV_AUDIO_SESSION_ERROR_CODE_NONE
        {
            shared.interrupted.store(false, Ordering::Release);
        }
    }
}

// ---------------------------------------------------------------------------
// SDLInterruptionListener

static SDL_INTERRUPTION_LISTENER: DeclaredClass = DeclaredClass::new();

/// The `SDLInterruptionListener` class (`@interface SDLInterruptionListener
/// : NSObject`). Its `device` property is the instance variable `device`,
/// the device's queue data (a reference of an `Arc<Shared>`, or null).
fn interruption_listener_class() -> Class {
    SDL_INTERRUPTION_LISTENER.get(|| {
        let mut b = ClassBuilder::new(c"SDLInterruptionListener", class!(NSObject))
            .expect("SDLInterruptionListener");
        b.add_ivar::<*const Shared>(c"device", c"^v");
        // SAFETY: the implementations have the signatures of the encodings.
        unsafe {
            b.add_method(
                sel!(audioSessionInterruption:),
                imp!(audio_session_interruption as extern "C" fn(Id, Sel, Id)),
                c"v@:@",
            );
            b.add_method(
                sel!(applicationBecameActive:),
                imp!(application_became_active as extern "C" fn(Id, Sel, Id)),
                c"v@:@",
            );
        }
        b.register()
    })
}

/// The listener's `device` instance variable.
///
/// # Safety
///
/// `listener` must be an `SDLInterruptionListener`.
unsafe fn listener_device(listener: Id) -> *mut *const Shared {
    // SAFETY: the caller's contract.
    unsafe { ivar_ptr::<*const Shared>(listener, interruption_listener_class(), c"device") }
}

/// `@synchronized(listener) { f(listener.device) }`.
///
/// # Safety
///
/// As [`listener_device`].
unsafe fn synchronized<T>(listener: Id, f: impl FnOnce(&mut *const Shared) -> T) -> T {
    struct Exit(Id);
    impl Drop for Exit {
        fn drop(&mut self) {
            // SAFETY: the object entered below.
            unsafe { objc_sync_exit(self.0) };
        }
    }
    // SAFETY: the caller's contract; the ivar is only touched under the
    // object's lock.
    unsafe {
        objc_sync_enter(listener);
        let _exit = Exit(listener);
        f(&mut *listener_device(listener))
    }
}

/// The listener's device, if any (inside [`synchronized`]).
fn device_of(device: &*const Shared) -> Option<&Shared> {
    // SAFETY: a non-null `device` holds a reference of an `Arc<Shared>`
    // (dropped only when it is set back to null, under the lock).
    unsafe { device.as_ref() }
}

/// Translation of `-[SDLInterruptionListener audioSessionInterruption:]`.
extern "C" fn audio_session_interruption(this: Id, _sel: Sel, note: Id) {
    // SAFETY: this is a listener; an NSNotification, its user info (an
    // NSDictionary or nil) and its value for the key (an NSNumber or nil).
    unsafe {
        synchronized(this, |device| {
            let user_info: Id = msg_send![note, userInfo];
            let ty: Id =
                msg_send![user_info, objectForKeyedSubscript: AVAudioSessionInterruptionTypeKey];
            let began = !ty.is_null() && {
                let value: NSUInteger = msg_send![ty, unsignedIntegerValue];
                value == AV_AUDIO_SESSION_INTERRUPTION_TYPE_BEGAN
            };
            if began {
                interruption_begin(device_of(device));
            } else {
                interruption_end(device_of(device));
            }
        })
    }
}

/// Translation of `-[SDLInterruptionListener applicationBecameActive:]`.
extern "C" fn application_became_active(this: Id, _sel: Sel, _note: Id) {
    // SAFETY: this is a listener.
    unsafe {
        synchronized(this, |device| {
            interruption_end(device_of(device));
        })
    }
}

/// The listener `OpenDevice` adds once the queue runs (into
/// `interruption_listener`).
pub(super) fn add_interruption_listener(hidden: &CoreAudioDevice, shared: &Arc<Shared>) {
    // SAFETY: a new listener, given a reference of the queue data, and
    // registrations of its methods (which take the notification); the
    // notification names are UIKit's and AVFoundation's constants.
    unsafe {
        let center: Id = msg_send![class!(NSNotificationCenter), defaultCenter];
        let listener = StrongPtr::from_owned(msg_send![interruption_listener_class(), new]);
        *listener_device(listener.get()) = Arc::into_raw(shared.clone());

        let session: Id = msg_send![class!(AVAudioSession), sharedInstance];
        let () = msg_send![center, addObserver: listener.get()
                                      selector: sel!(audioSessionInterruption:)
                                          name: AVAudioSessionInterruptionNotification
                                        object: session];

        /* An interruption end notification is not guaranteed to be sent if
        we were previously interrupted... resuming if needed when the app
        becomes active seems to be the way to go. */
        // Note: object: below needs to be nil, as otherwise it filters by the object, and session doesn't send foreground / active notifications.
        let () = msg_send![center, addObserver: listener.get()
                                      selector: sel!(applicationBecameActive:)
                                          name: UIApplicationDidBecomeActiveNotification
                                        object: NIL];

        let () = msg_send![center, addObserver: listener.get()
                                      selector: sel!(applicationBecameActive:)
                                          name: UIApplicationWillEnterForegroundNotification
                                        object: NIL];

        *hidden
            .interruption_listener
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = listener;
    }
}

/// `CloseDevice`'s removal of the listener (if there is one).
pub(super) fn remove_interruption_listener(listener: &mut StrongPtr) {
    let listener = std::mem::take(listener);
    if listener.is_nil() {
        return;
    }
    // SAFETY: our listener: its registrations go, then (under its lock)
    // its device, whose reference is dropped.
    unsafe {
        let center: Id = msg_send![class!(NSNotificationCenter), defaultCenter];
        let () = msg_send![center, removeObserver: listener.get()];
        let device = synchronized(listener.get(), |device| {
            let taken = *device;
            *device = ptr::null();
            taken
        });
        if !device.is_null() {
            drop(Arc::from_raw(device));
        }
    }
}

// ---------------------------------------------------------------------------
// UpdateAudioSession

/// The `AVAudioSessionCategory` constants upstream chooses from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Category {
    Ambient,
    SoloAmbient,
    Playback,
    Record,
    PlayAndRecord,
}

impl Category {
    /// The `NSString` constant.
    fn ns_string(self) -> Id {
        // SAFETY: AVFoundation's constants.
        unsafe {
            match self {
                Category::Ambient => AVAudioSessionCategoryAmbient,
                Category::SoloAmbient => AVAudioSessionCategorySoloAmbient,
                Category::Playback => AVAudioSessionCategoryPlayback,
                Category::Record => AVAudioSessionCategoryRecord,
                Category::PlayAndRecord => AVAudioSessionCategoryPlayAndRecord,
            }
        }
    }
}

/// The category and options `UpdateAudioSession()` sets for the
/// [`hints::AUDIO_CATEGORY`](crate::hints::AUDIO_CATEGORY) hint, the
/// [`hints::AUDIO_DUCK_OTHERS`](crate::hints::AUDIO_DUCK_OTHERS) hint's
/// value (`duck_others`) and how many playback and recording devices are
/// open; `None` where it gives up ("We already failed play and record").
pub(super) fn session_category(
    hint: Option<&str>,
    duck_others: bool,
    playback: usize,
    recording: usize,
    allow_playandrecord: bool,
) -> Option<(Category, NSUInteger)> {
    let is = |a: &str, b: &str| strcasecmp(a, b) == std::cmp::Ordering::Equal;
    let mut category = Category::Playback;
    let mut options = AV_AUDIO_SESSION_CATEGORY_OPTION_MIX_WITH_OTHERS;

    if let Some(hint) = hint {
        if is(hint, "AVAudioSessionCategoryAmbient") || is(hint, "ambient") {
            category = Category::Ambient;
        } else if is(hint, "AVAudioSessionCategorySoloAmbient") {
            category = Category::SoloAmbient;
            options &= !AV_AUDIO_SESSION_CATEGORY_OPTION_MIX_WITH_OTHERS;
        } else if is(hint, "AVAudioSessionCategoryPlayback") || is(hint, "playback") {
            category = Category::Playback;
            options &= !AV_AUDIO_SESSION_CATEGORY_OPTION_MIX_WITH_OTHERS;
        } else if (is(hint, "AVAudioSessionCategoryPlayAndRecord") || is(hint, "playandrecord"))
            && allow_playandrecord
        {
            category = Category::PlayAndRecord;
        }
    } else if playback != 0 && recording != 0 {
        if allow_playandrecord {
            category = Category::PlayAndRecord;
        } else {
            // We already failed play and record with AVAudioSessionErrorCodeResourceNotAvailable
            return None;
        }
    } else if recording != 0 {
        category = Category::Record;
    }

    #[cfg(not(target_os = "tvos"))]
    if category == Category::PlayAndRecord {
        options |= AV_AUDIO_SESSION_CATEGORY_OPTION_DEFAULT_TO_SPEAKER;
    }
    if category == Category::Record || category == Category::PlayAndRecord {
        /* AVAudioSessionCategoryOptionAllowBluetooth isn't available in the SDK for
          Apple TV but is still needed in order to output to Bluetooth devices.
        */
        options |= AV_AUDIO_SESSION_CATEGORY_OPTION_ALLOW_BLUETOOTH;
    }
    if category == Category::PlayAndRecord {
        options |= AV_AUDIO_SESSION_CATEGORY_OPTION_ALLOW_BLUETOOTH_A2DP
            | AV_AUDIO_SESSION_CATEGORY_OPTION_ALLOW_AIR_PLAY;
    }
    if (category == Category::Playback || category == Category::PlayAndRecord) && duck_others {
        options |= AV_AUDIO_SESSION_CATEGORY_OPTION_DUCK_OTHERS;
    }

    Some((category, options))
}

/// Translation of `CountOpenAudioDevices()`: the playback and recording
/// devices whose `hidden` exists (assume it's open if hidden != NULL).
fn count_open_audio_devices() -> (usize, usize) {
    let hidden = hidden_devices();
    let recording = hidden.iter().filter(|d| d.recording).count();
    (hidden.len() - recording, recording)
}

/// An `NSError`'s description (`err.description.UTF8String`).
///
/// # Safety
///
/// `err` must be nil or an `NSError`.
unsafe fn error_description(err: Id) -> String {
    // SAFETY: the caller's contract.
    unsafe {
        let desc: Id = msg_send![err, description];
        string_from_ns(desc).unwrap_or_else(|| "(null)".into())
    }
}

/// Translation of `UpdateAudioSession()`. (Upstream's `device` and `open`
/// parameters are unused, apart from being passed on.)
pub(super) fn update_audio_session(allow_playandrecord: bool) -> Result<()> {
    autoreleasepool(|| {
        // SAFETY: AVAudioSession's shared instance, its properties and
        // methods, with an out NSError.
        unsafe {
            let session: Id = msg_send![class!(AVAudioSession), sharedInstance];

            let mode = AVAudioSessionModeDefault;
            let mut err: Id = NIL;

            let (playback, recording) = count_open_audio_devices();

            let hint = crate::hints::get(crate::hints::AUDIO_CATEGORY);
            let duck_others = crate::hints::get_bool(crate::hints::AUDIO_DUCK_OTHERS, false);
            let Some((category, options)) = session_category(
                hint.as_deref(),
                duck_others,
                playback,
                recording,
                allow_playandrecord,
            ) else {
                // (no error is set upstream: only the retry below gets here, and it sets its own)
                return Err(Error::new("Could not activate Audio Session"));
            };
            let category_name = category.ns_string();

            let current: Id = msg_send![session, category];
            let same: Bool = msg_send![current, isEqualToString: category_name];
            let current_options: NSUInteger = msg_send![session, categoryOptions];
            if !same.is_true() || current_options != options {
                // Stop the current session so we don't interrupt other application audio
                pause_audio_devices();
                let _: Bool = msg_send![session, setActive: Bool::NO error: NIL];
                SESSION_ACTIVE.store(false, Ordering::Release);

                let ok: Bool = msg_send![session, setCategory: category_name
                                                         mode: mode
                                                      options: options
                                                        error: &mut err as *mut Id];
                if !ok.is_true() {
                    return Err(Error::new(format!(
                        "Could not set Audio Session category: {}",
                        error_description(err)
                    )));
                }
            }

            if (playback != 0 || recording != 0) && !SESSION_ACTIVE.load(Ordering::Acquire) {
                let ok: Bool = msg_send![session, setActive: Bool::YES error: &mut err as *mut Id];
                if !ok.is_true() {
                    let code: NSInteger = msg_send![err, code];
                    if code == AV_AUDIO_SESSION_ERROR_CODE_RESOURCE_NOT_AVAILABLE
                        && category == Category::PlayAndRecord
                    {
                        if update_audio_session(false).is_ok() {
                            return Ok(());
                        } else {
                            return Err(Error::new(
                                "Could not activate Audio Session: Resource not available",
                            ));
                        }
                    }

                    return Err(Error::new(format!(
                        "Could not activate Audio Session: {}",
                        error_description(err)
                    )));
                }
                SESSION_ACTIVE.store(true, Ordering::Release);
                resume_audio_devices();
            } else if playback == 0 && recording == 0 && SESSION_ACTIVE.load(Ordering::Acquire) {
                pause_audio_devices();
                let _: Bool = msg_send![session, setActive: Bool::NO error: NIL];
                SESSION_ACTIVE.store(false, Ordering::Release);
            }
            Ok(())
        }
    })
}

/// The first steps of `COREAUDIO_OpenDevice()` on the UIKit platforms:
/// the session updated for the device, then its preferred sample rate,
/// which the device takes on (and on tvOS its channel counts).
pub(super) fn open_session(recording: bool, state: &mut PhysState) -> Result<()> {
    update_audio_session(true)?;

    // Stop CoreAudio from doing expensive audio rate conversion
    autoreleasepool(|| {
        // SAFETY: AVAudioSession's shared instance and its properties.
        unsafe {
            let session: Id = msg_send![class!(AVAudioSession), sharedInstance];
            let _: Bool =
                msg_send![session, setPreferredSampleRate: state.spec.freq as f64 error: NIL];
            let rate: f64 = msg_send![session, sampleRate];
            state.spec.freq = rate as i32;
            #[cfg(not(target_os = "tvos"))]
            let _ = recording;
            #[cfg(target_os = "tvos")]
            {
                if recording {
                    let _: Bool = msg_send![session, setPreferredInputNumberOfChannels: state.spec.channels as NSInteger error: NIL];
                    let n: NSInteger = msg_send![session, preferredInputNumberOfChannels];
                    state.spec.channels = n as i32;
                } else {
                    let _: Bool = msg_send![session, setPreferredOutputNumberOfChannels: state.spec.channels as NSInteger error: NIL];
                    let n: NSInteger = msg_send![session, preferredOutputNumberOfChannels];
                    state.spec.channels = n as i32;
                }
            }
            // (Calling setPreferredOutputNumberOfChannels seems to break audio output on iOS)
        }
    });
    Ok(())
}
