// Rust translation of android-project/app/src/main/java/org/libsdl/app/
// SDLActivity.java from Simple DirectMedia Layer (part: the clipboard,
// `class SDLClipboardHandler` and SDLActivity's `mClipboardHandler`,
// `clipboardHasText()`, `clipboardGetText()` and `clipboardSetText()`).
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! `class SDLClipboardHandler implements
//! ClipboardManager.OnPrimaryClipChangedListener`: the system's clipboard
//! (the `ClipboardManager`'s primary clip), its text read, tested and set
//! (an empty one clears it), and its changes, which the handler (a primary
//! clip listener, made by the activity's `onCreate()` when video is
//! compiled: [`create`]) passes on to SDL (`onNativeClipboardChanged()`).
//! While it sets the clip, the handler isn't a listener, so that SDL's own
//! changes aren't sent back to it (but they are: see
//! [`clipboard_set_text`]).
//!
//! The class is the DEX file's (`super::super::classes`), with its field
//! `mClipMgr`; its constructor's code runs in [`create`], after the DEX
//! file's constructor (`Object`'s). SDLActivity's static methods that call
//! it are [`clipboard_has_text`], [`clipboard_get_text`] and
//! [`clipboard_set_text`].

use super::super::env::{Arg, Env, Global, JniResult, Local, Obj};
use super::super::jni::*;
use super::sdl;
use std::sync::Mutex;

/// SDLActivity's `mClipboardHandler`.
static CLIPBOARD_HANDLER: Mutex<Option<Global>> = Mutex::new(None);

/// `mClipMgr`'s descriptor.
const CLIPBOARD_MANAGER: &std::ffi::CStr = c"Landroid/content/ClipboardManager;";

/// `mClipboardHandler = null` (SDLActivity's `initialize()`).
pub(crate) fn reset() {
    *CLIPBOARD_HANDLER.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// `mClipboardHandler = new SDLClipboardHandler()` (SDLActivity's
/// `onCreate()`): the DEX file's constructor, then the constructor's code
/// (the context's `ClipboardManager`, which it listens to).
pub(crate) fn create<'a>(env: Env<'a>) -> JniResult<'a, ()> {
    let class = super::super::env::app_class(super::super::classes::SDL_CLIPBOARD_HANDLER)
        .expect("JNI_OnLoad() found SDLClipboardHandler");
    let this = env.new_object(class.obj(), c"()V", &[])?;

    let context = sdl::get_context(env);
    let service = env.new_string("clipboard")?; // Context.CLIPBOARD_SERVICE
    let clip_mgr = env
        .call_method(
            context.as_ref().map_or(Obj::NULL, Local::obj),
            c"getSystemService",
            c"(Ljava/lang/String;)Ljava/lang/Object;",
            &[Arg::Object(service.obj())],
        )?
        .object();
    let clip_mgr_obj = clip_mgr.as_ref().map_or(Obj::NULL, Local::obj);
    env.set_field(
        this.obj(),
        c"mClipMgr",
        CLIPBOARD_MANAGER,
        Arg::Object(clip_mgr_obj),
    )?;
    add_primary_clip_changed_listener(env, clip_mgr_obj, this.obj())?;

    *CLIPBOARD_HANDLER.lock().unwrap_or_else(|e| e.into_inner()) = Global::new(env, this.obj());
    Ok(())
}

/// `mClipboardHandler`, or the `NullPointerException` of using it null.
fn handler<'a>(env: Env<'a>) -> JniResult<'a, Local<'a>> {
    let handler = CLIPBOARD_HANDLER
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .and_then(|h| env.new_local_ref(h.obj()));
    match handler {
        Some(handler) => Ok(handler),
        None => {
            let npe = env.find_class(c"java/lang/NullPointerException")?;
            env.throw_new(npe.obj(), c"SDLActivity.mClipboardHandler is null");
            env.pending()?;
            unreachable!("ThrowNew() threw")
        }
    }
}

/// The handler's `mClipMgr`.
fn clip_mgr<'a>(env: Env<'a>, this: Obj<'_>) -> JniResult<'a, Option<Local<'a>>> {
    Ok(env
        .get_field(this, c"mClipMgr", CLIPBOARD_MANAGER)?
        .object())
}

fn add_primary_clip_changed_listener<'a>(
    env: Env<'a>,
    clip_mgr: Obj<'_>,
    listener: Obj<'_>,
) -> JniResult<'a, ()> {
    env.call_method(
        clip_mgr,
        c"addPrimaryClipChangedListener",
        c"(Landroid/content/ClipboardManager$OnPrimaryClipChangedListener;)V",
        &[Arg::Object(listener)],
    )?;
    Ok(())
}

/// This method is called by SDL using JNI. Translation of
/// `SDLActivity.clipboardHasText()` and `SDLClipboardHandler.clipboardHasText()`:
/// from API 28, whether there's a primary clip (of any kind).
pub(crate) fn clipboard_has_text<'a>(env: Env<'a>) -> JniResult<'a, bool> {
    let this = handler(env)?;
    let clip_mgr = clip_mgr(env, this.obj())?;
    let clip_mgr = clip_mgr.as_ref().map_or(Obj::NULL, Local::obj);
    let method = if super::super::sdk_int(env) >= 28
    /* Android 9 (P) */
    {
        c"hasPrimaryClip"
    } else {
        c"hasText"
    };
    Ok(env.call_method(clip_mgr, method, c"()Z", &[])?.boolean())
}

/// This method is called by SDL using JNI: the primary clip's first
/// item's text, `None` without one. Translation of
/// `SDLActivity.clipboardGetText()` and `SDLClipboardHandler.clipboardGetText()`.
pub(crate) fn clipboard_get_text<'a>(env: Env<'a>) -> JniResult<'a, Option<String>> {
    let this = handler(env)?;
    let clip_mgr = clip_mgr(env, this.obj())?;
    let clip = env
        .call_method(
            clip_mgr.as_ref().map_or(Obj::NULL, Local::obj),
            c"getPrimaryClip",
            c"()Landroid/content/ClipData;",
            &[],
        )?
        .object();
    if let Some(clip) = clip {
        let item = env
            .call_method(
                clip.obj(),
                c"getItemAt",
                c"(I)Landroid/content/ClipData$Item;",
                &[Arg::Int(0)],
            )?
            .object();
        if let Some(item) = item {
            let text = env
                .call_method(item.obj(), c"getText", c"()Ljava/lang/CharSequence;", &[])?
                .object();
            if let Some(text) = text {
                let string = env
                    .call_method(text.obj(), c"toString", c"()Ljava/lang/String;", &[])?
                    .object()
                    .expect("a CharSequence's string");
                return env.get_string(string.obj()).map(Some);
            }
        }
    }
    Ok(None)
}

/// This method is called by SDL using JNI: the primary clip set to the
/// text (a plain text clip without a label), or cleared when it's empty
/// (before API 28: set to an empty one), while the handler doesn't
/// listen. Translation of `SDLActivity.clipboardSetText()` and
/// `SDLClipboardHandler.clipboardSetText()`. FIXME (upstream): the
/// clipboard service tells its listeners asynchronously (a one-way call),
/// so the handler, a listener again by then, hears of SDL's own change
/// too: a clipboard update the app doesn't own follows SDL's, and the
/// clipboard front end forgets the app's MIME types (traced by
/// `android_clipboard`).
pub(crate) fn clipboard_set_text<'a>(env: Env<'a>, string: &str) -> JniResult<'a, ()> {
    let this = handler(env)?;
    let clip_mgr = clip_mgr(env, this.obj())?;
    let clip_mgr = clip_mgr.as_ref().map_or(Obj::NULL, Local::obj);
    env.call_method(
        clip_mgr,
        c"removePrimaryClipChangedListener",
        c"(Landroid/content/ClipboardManager$OnPrimaryClipChangedListener;)V",
        &[Arg::Object(this.obj())],
    )?;
    let clip_data = env.find_class(c"android/content/ClipData")?;
    let new_plain_text = |text: &str| -> JniResult<'a, Local<'a>> {
        let text = env.new_string(text)?;
        Ok(env
            .call_static_method(
                clip_data.obj(),
                c"newPlainText",
                c"(Ljava/lang/CharSequence;Ljava/lang/CharSequence;)Landroid/content/ClipData;",
                &[Arg::Object(Obj::NULL), Arg::Object(text.obj())],
            )?
            .object()
            .expect("a clip"))
    };
    if string.is_empty() {
        if super::super::sdk_int(env) >= 28
        /* Android 9 (P) */
        {
            env.call_method(clip_mgr, c"clearPrimaryClip", c"()V", &[])?;
        } else {
            let clip = new_plain_text("")?;
            env.call_method(
                clip_mgr,
                c"setPrimaryClip",
                c"(Landroid/content/ClipData;)V",
                &[Arg::Object(clip.obj())],
            )?;
        }
    } else {
        let clip = new_plain_text(string)?;
        env.call_method(
            clip_mgr,
            c"setPrimaryClip",
            c"(Landroid/content/ClipData;)V",
            &[Arg::Object(clip.obj())],
        )?;
    }
    add_primary_clip_changed_listener(env, clip_mgr, this.obj())
}

/// The primary clip changed (not by [`clipboard_set_text`]). Translation
/// of `SDLClipboardHandler.onPrimaryClipChanged()`.
pub(crate) extern "system" fn on_primary_clip_changed(_env: *mut JNIEnv, _this: jobject) {
    super::super::on_clipboard_changed();
}
