// Rust translation of android-project/app/src/main/java/org/libsdl/app/
// SDLAudioManager.java from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! `class SDLAudioManager`: the system's audio devices (the
//! `AudioManager`'s, API 24 and later), their hotplug, and the audio
//! threads' names and priority (`audioSetThreadPriority()`, which SDL
//! calls from each device's thread).
//!
//! The class is the DEX file's (`super::super::classes`), with its
//! natives (`nativeSetupJNI()`, `nativeAddAudioDevice()`,
//! `nativeRemoveAudioDevice()`); its static methods, which SDL calls
//! through JNI upstream, are Rust functions here, and its statics this
//! module's. Upstream's anonymous `AudioDeviceCallback` subclass is the DEX
//! file's `org.libsdl.app.SDLAudioDeviceCallback` (Note: another name than
//! upstream's `SDLAudioManager$1`), whose overrides are
//! [`on_audio_devices_added`] and [`on_audio_devices_removed`].
//!
//! Note (upstream): its callbacks pass `isSink()` (true for a playback
//! device) as `nativeAddAudioDevice()`'s and `nativeRemoveAudioDevice()`'s
//! `recording`; those natives do nothing while SDL_android.h's
//! `ALLOW_MULTIPLE_ANDROID_AUDIO_DEVICES` is 0 (as in 3.5.0), and SDL never
//! registers the callback then.

use std::sync::Mutex;

use super::super::classes::SDL_AUDIO_DEVICE_CALLBACK;
use super::super::env::{self, Arg, Env, Global, JniResult, Local, Obj};
use super::super::jni::*;
use super::super::ndk::LogPriority;

const TAG: &str = "SDLAudio";

/// `mContext`.
static CONTEXT: Mutex<Option<Global>> = Mutex::new(None);

/// `mAudioDeviceCallback`.
static AUDIO_DEVICE_CALLBACK: Mutex<Option<Global>> = Mutex::new(None);

/// `Context.AUDIO_SERVICE`.
const AUDIO_SERVICE: &str = "audio";
/// `AudioManager.GET_DEVICES_INPUTS`.
const GET_DEVICES_INPUTS: jint = 1;
/// `AudioManager.GET_DEVICES_OUTPUTS`.
const GET_DEVICES_OUTPUTS: jint = 2;
/// `AudioDeviceInfo.TYPE_TELEPHONY`.
const TYPE_TELEPHONY: jint = 18;
/// `android.os.Process.THREAD_PRIORITY_AUDIO`.
const THREAD_PRIORITY_AUDIO: jint = -16;

fn log(priority: LogPriority, message: &str) {
    super::super::ndk::log_write(priority, TAG, message);
}

/// Translation of `initialize()`.
pub(crate) fn initialize<'a>(env: Env<'a>) -> JniResult<'a, ()> {
    *AUDIO_DEVICE_CALLBACK
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = None;

    if super::super::sdk_int(env) >= 24
    /* Android 7.0 (N) */
    {
        let class = env::app_class(SDL_AUDIO_DEVICE_CALLBACK)
            .expect("JNI_OnLoad() found SDLAudioDeviceCallback");
        let callback = env.new_object(class.obj(), c"()V", &[])?;
        *AUDIO_DEVICE_CALLBACK
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Global::new(env, callback.obj());
    }
    Ok(())
}

/// Translation of `setContext()`.
pub(crate) fn set_context(env: Env<'_>, context: Option<Obj<'_>>) {
    *CONTEXT.lock().unwrap_or_else(|e| e.into_inner()) = context.and_then(|c| Global::new(env, c));
}

/// Translation of `release()`.
pub(crate) fn release(_env: Env<'_>, _context: Obj<'_>) {
    // no-op atm
}

/// `mAudioDeviceCallback`, as a local reference (null when there's none).
fn audio_device_callback<'a>(env: Env<'a>) -> Option<Local<'a>> {
    AUDIO_DEVICE_CALLBACK
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .and_then(|c| env.new_local_ref(c.obj()))
}

/// `(AudioManager) mContext.getSystemService(Context.AUDIO_SERVICE)` (the
/// `NullPointerException` of a null context, as Java's).
fn audio_manager<'a>(env: Env<'a>) -> JniResult<'a, Local<'a>> {
    let context = CONTEXT
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .and_then(|c| env.new_local_ref(c.obj()));
    let service = env.new_string(AUDIO_SERVICE)?;
    let manager = env
        .call_method(
            context.as_ref().map_or(Obj::NULL, Local::obj),
            c"getSystemService",
            c"(Ljava/lang/String;)Ljava/lang/Object;",
            &[Arg::Object(service.obj())],
        )?
        .object();
    match manager {
        Some(manager) => Ok(manager),
        None => {
            let npe = env.find_class(c"java/lang/NullPointerException")?;
            env.throw_new(npe.obj(), c"getSystemService(AUDIO_SERVICE) is null");
            env.pending()?;
            unreachable!("ThrowNew() threw")
        }
    }
}

/// `audioManager.getDevices(flags)`: each `AudioDeviceInfo`.
fn get_devices<'a>(env: Env<'a>, manager: Obj<'_>, flags: jint) -> JniResult<'a, Vec<Local<'a>>> {
    let array = env
        .call_method(
            manager,
            c"getDevices",
            c"(I)[Landroid/media/AudioDeviceInfo;",
            &[Arg::Int(flags)],
        )?
        .object();
    elements(env, array.as_ref().map_or(Obj::NULL, Local::obj))
}

/// The elements of an `AudioDeviceInfo[]` (none for null).
fn elements<'a>(env: Env<'a>, array: Obj<'_>) -> JniResult<'a, Vec<Local<'a>>> {
    let mut out = Vec::new();
    if array.is_null() {
        return Ok(out);
    }
    for i in 0..env.get_array_length(array) {
        if let Some(element) = env.get_object_array_element(array, i)? {
            out.push(element);
        }
    }
    Ok(out)
}

/// `deviceInfo.getId()`.
fn id<'a>(env: Env<'a>, device_info: Obj<'_>) -> JniResult<'a, jint> {
    Ok(env.call_method(device_info, c"getId", c"()I", &[])?.int())
}

/// `deviceInfo.isSink()`.
fn is_sink<'a>(env: Env<'a>, device_info: Obj<'_>) -> JniResult<'a, bool> {
    Ok(env
        .call_method(device_info, c"isSink", c"()Z", &[])?
        .boolean())
}

/// `deviceInfo.getProductName().toString()`.
fn product_name<'a>(env: Env<'a>, device_info: Obj<'_>) -> JniResult<'a, String> {
    let name = env
        .call_method(
            device_info,
            c"getProductName",
            c"()Ljava/lang/CharSequence;",
            &[],
        )?
        .object();
    let string = env
        .call_method(
            name.as_ref().map_or(Obj::NULL, Local::obj),
            c"toString",
            c"()Ljava/lang/String;",
            &[],
        )?
        .object();
    env.get_string(string.as_ref().map_or(Obj::NULL, Local::obj))
}

/// `nativeAddAudioDevice(deviceInfo.isSink(), deviceInfo.getProductName().toString(), deviceInfo.getId())`.
fn add_audio_device<'a>(env: Env<'a>, device_info: Obj<'_>) -> JniResult<'a, ()> {
    let recording = is_sink(env, device_info)?;
    let name = product_name(env, device_info)?;
    let device_id = id(env, device_info)?;
    super::super::add_audio_device(recording, &name, device_id);
    Ok(())
}

/// `AudioDeviceCallback.onAudioDevicesAdded()` of upstream's anonymous
/// subclass.
pub(crate) extern "system" fn on_audio_devices_added(
    env: *mut JNIEnv,
    _this: jobject,
    added_devices: jobject,
) {
    // SAFETY: a native method's environment and arguments.
    let (env, added_devices) = unsafe { (Env::from_raw(env), Obj::from_raw(added_devices)) };
    native(env, |env| {
        for device_info in elements(env, added_devices)? {
            add_audio_device(env, device_info.obj())?;
        }
        Ok(())
    });
}

/// `AudioDeviceCallback.onAudioDevicesRemoved()` of upstream's anonymous
/// subclass.
pub(crate) extern "system" fn on_audio_devices_removed(
    env: *mut JNIEnv,
    _this: jobject,
    removed_devices: jobject,
) {
    // SAFETY: a native method's environment and arguments.
    let (env, removed_devices) = unsafe { (Env::from_raw(env), Obj::from_raw(removed_devices)) };
    native(env, |env| {
        for device_info in elements(env, removed_devices)? {
            let recording = is_sink(env, device_info.obj())?;
            let device_id = id(env, device_info.obj())?;
            super::super::remove_audio_device(recording, device_id);
        }
        Ok(())
    });
}

/// A native method's body: an exception it throws stays pending, for the
/// framework.
fn native<'a>(env: Env<'a>, body: impl FnOnce(Env<'a>) -> JniResult<'a, ()>) {
    if let Err(thrown) = body(env) {
        // (pending, for the framework)
        drop(thrown);
    }
}

// Audio

/// The input device of that ID, if there's one. Translation of
/// `getInputAudioDeviceInfo()` (private, and unused upstream too).
#[allow(dead_code)]
fn get_input_audio_device_info<'a>(
    env: Env<'a>,
    device_id: jint,
) -> JniResult<'a, Option<Local<'a>>> {
    find_audio_device_info(env, GET_DEVICES_INPUTS, device_id)
}

/// The playback device of that ID, if there's one. Translation of
/// `getPlaybackAudioDeviceInfo()` (private, and unused upstream too).
#[allow(dead_code)]
fn get_playback_audio_device_info<'a>(
    env: Env<'a>,
    device_id: jint,
) -> JniResult<'a, Option<Local<'a>>> {
    find_audio_device_info(env, GET_DEVICES_OUTPUTS, device_id)
}

#[allow(dead_code)]
fn find_audio_device_info<'a>(
    env: Env<'a>,
    flags: jint,
    device_id: jint,
) -> JniResult<'a, Option<Local<'a>>> {
    if super::super::sdk_int(env) >= 24
    /* Android 7.0 (N) */
    {
        let audio_manager = audio_manager(env)?;
        for device_info in get_devices(env, audio_manager.obj(), flags)? {
            if id(env, device_info.obj())? == device_id {
                return Ok(Some(device_info));
            }
        }
    }
    Ok(None)
}

/// Translation of `registerAudioDeviceCallback()`: the devices there are
/// now passed to `nativeAddAudioDevice()` (the outputs but the telephony
/// one, which can't be opened, then the inputs), then the callback
/// registered (on the main thread's looper).
pub(crate) fn register_audio_device_callback<'a>(env: Env<'a>) -> JniResult<'a, ()> {
    if super::super::sdk_int(env) >= 24
    /* Android 7.0 (N) */
    {
        let audio_manager = audio_manager(env)?;
        // get an initial list now, before hotplug callbacks fire.
        for dev in get_devices(env, audio_manager.obj(), GET_DEVICES_OUTPUTS)? {
            if env.call_method(dev.obj(), c"getType", c"()I", &[])?.int() == TYPE_TELEPHONY {
                continue; // Device cannot be opened
            }
            add_audio_device(env, dev.obj())?;
        }
        for dev in get_devices(env, audio_manager.obj(), GET_DEVICES_INPUTS)? {
            add_audio_device(env, dev.obj())?;
        }
        let callback = audio_device_callback(env);
        env.call_method(
            audio_manager.obj(),
            c"registerAudioDeviceCallback",
            c"(Landroid/media/AudioDeviceCallback;Landroid/os/Handler;)V",
            &[
                Arg::Object(callback.as_ref().map_or(Obj::NULL, Local::obj)),
                Arg::Object(Obj::NULL),
            ],
        )?;
    }
    Ok(())
}

/// Translation of `unregisterAudioDeviceCallback()`.
pub(crate) fn unregister_audio_device_callback<'a>(env: Env<'a>) -> JniResult<'a, ()> {
    if super::super::sdk_int(env) >= 24
    /* Android 7.0 (N) */
    {
        let audio_manager = audio_manager(env)?;
        let callback = audio_device_callback(env);
        env.call_method(
            audio_manager.obj(),
            c"unregisterAudioDeviceCallback",
            c"(Landroid/media/AudioDeviceCallback;)V",
            &[Arg::Object(callback.as_ref().map_or(Obj::NULL, Local::obj))],
        )?;
    }
    Ok(())
}

/// This method is called by SDL using JNI: the calling thread (a device's)
/// named `SDLAudioC<device_id>` (recording) or `SDLAudioP<device_id>`, at
/// the audio priority. Translation of `audioSetThreadPriority()`.
pub(crate) fn audio_set_thread_priority<'a>(
    env: Env<'a>,
    recording: bool,
    device_id: jint,
) -> JniResult<'a, ()> {
    let body = || -> JniResult<'a, ()> {
        /* Set thread name */
        let thread_class = env.find_class(c"java/lang/Thread")?;
        let thread = env
            .call_static_method(
                thread_class.obj(),
                c"currentThread",
                c"()Ljava/lang/Thread;",
                &[],
            )?
            .object();
        let name = if recording {
            format!("SDLAudioC{device_id}")
        } else {
            format!("SDLAudioP{device_id}")
        };
        let name = env.new_string(&name)?;
        env.call_method(
            thread.as_ref().map_or(Obj::NULL, Local::obj),
            c"setName",
            c"(Ljava/lang/String;)V",
            &[Arg::Object(name.obj())],
        )?;

        /* Set thread priority */
        let process = env.find_class(c"android/os/Process")?;
        env.call_static_method(
            process.obj(),
            c"setThreadPriority",
            c"(I)V",
            &[Arg::Int(THREAD_PRIORITY_AUDIO)],
        )?;
        Ok(())
    };
    // try { ... } catch (Exception e) { ... }
    let Err(thrown) = body() else {
        return Ok(());
    };
    let e = thrown.catch_exception()?;
    let text = env
        .call_method(e.obj(), c"toString", c"()Ljava/lang/String;", &[])?
        .object();
    let text = env.get_string(text.as_ref().map_or(Obj::NULL, Local::obj))?;
    log(
        LogPriority::Verbose,
        &format!("modify thread properties failed {text}"),
    );
    Ok(())
}

// The static methods SDL calls (Rust functions, which the DEX file's class
// declares `native` too, so that Java calls them as it calls upstream's)

/// `SDLAudioManager.registerAudioDeviceCallback()` called from Java.
pub(crate) extern "system" fn register_audio_device_callback_java(env: *mut JNIEnv, _cls: jclass) {
    // SAFETY: a native method's environment.
    let env = unsafe { Env::from_raw(env) };
    native(env, register_audio_device_callback);
}

/// `SDLAudioManager.unregisterAudioDeviceCallback()` called from Java.
pub(crate) extern "system" fn unregister_audio_device_callback_java(
    env: *mut JNIEnv,
    _cls: jclass,
) {
    // SAFETY: a native method's environment.
    let env = unsafe { Env::from_raw(env) };
    native(env, unregister_audio_device_callback);
}

/// `SDLAudioManager.audioSetThreadPriority()` called from Java.
pub(crate) extern "system" fn audio_set_thread_priority_java(
    env: *mut JNIEnv,
    _cls: jclass,
    recording: jboolean,
    device_id: jint,
) {
    // SAFETY: a native method's environment.
    let env = unsafe { Env::from_raw(env) };
    native(env, |env| {
        audio_set_thread_priority(env, recording != 0, device_id)
    });
}
