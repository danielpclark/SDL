// Rust translation of android-project/app/src/main/java/org/libsdl/app/
// SDLDummyEdit.java from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! `public class SDLDummyEdit extends View implements View.OnKeyListener`:
//! a fake invisible editor view that receives the input and defines the
//! pan&scan region. The activity puts it over the window's text input area
//! when text input starts (`SDLActivity.showTextInput()`) and focuses it,
//! so that the input method connects to it: it's a text editor
//! (`onCheckIsTextEditor()`), whose input connection
//! (`onCreateInputConnection()`) is an [`super::input_connection`], of the
//! input type the text input properties give (`setInputType()`); its keys
//! go to `SDLActivity.handleKeyEvent()` with that connection, and the back
//! key, before the input method has it, ends text input
//! (`onKeyPreIme()`: `onNativeKeyboardFocusLost()`).
//!
//! The constructor's code (the focus, the key listener) runs in
//! [`new_sdl_dummy_edit`], after the DEX file's constructor, which only
//! calls `View`'s; the fields are the DEX file's
//! (`super::super::classes`), and `setInputType()` is a Rust function
//! ([`set_input_type`]).

use super::super::env::{Arg, Env, JniResult, Local, Obj};
use super::super::jni::*;
use super::{activity, input_connection};

/// `View.VISIBLE`.
const VISIBLE: jint = 0;
/// `KeyEvent.ACTION_UP`.
const ACTION_UP: jint = 1;
/// `KeyEvent.KEYCODE_BACK`.
const KEYCODE_BACK: jint = 4;
/// `EditorInfo.IME_FLAG_NO_EXTRACT_UI`.
const IME_FLAG_NO_EXTRACT_UI: jint = 0x1000_0000;
/// `EditorInfo.IME_FLAG_NO_FULLSCREEN` (API 11).
const IME_FLAG_NO_FULLSCREEN: jint = 0x0200_0000;

/// `ic`'s descriptor.
const INPUT_CONNECTION: &std::ffi::CStr = c"Landroid/view/inputmethod/InputConnection;";

/// `new SDLDummyEdit(context)`: the DEX file's constructor (`super(context)`),
/// then the constructor's code.
pub(crate) fn new_sdl_dummy_edit<'a>(env: Env<'a>, context: Obj<'_>) -> JniResult<'a, Local<'a>> {
    let class = super::super::env::app_class(super::super::classes::SDL_DUMMY_EDIT)
        .expect("JNI_OnLoad() found SDLDummyEdit");
    let this = env.new_object(
        class.obj(),
        c"(Landroid/content/Context;)V",
        &[Arg::Object(context)],
    )?;
    env.call_method(
        this.obj(),
        c"setFocusableInTouchMode",
        c"(Z)V",
        &[Arg::Boolean(true)],
    )?;
    env.call_method(this.obj(), c"setFocusable", c"(Z)V", &[Arg::Boolean(true)])?;
    env.call_method(
        this.obj(),
        c"setOnKeyListener",
        c"(Landroid/view/View$OnKeyListener;)V",
        &[Arg::Object(this.obj())],
    )?;
    Ok(this)
}

/// Translation of `setInputType()`.
pub(crate) fn set_input_type<'a>(
    env: Env<'a>,
    this: Obj<'_>,
    input_type: jint,
) -> JniResult<'a, ()> {
    env.set_field(this, c"input_type", c"I", Arg::Int(input_type))
}

/// Translation of `onCheckIsTextEditor()`.
pub(crate) extern "system" fn on_check_is_text_editor(
    _env: *mut JNIEnv,
    _this: jobject,
) -> jboolean {
    JNI_TRUE
}

/// Translation of `onKey()`.
pub(crate) extern "system" fn on_key(
    env: *mut JNIEnv,
    this: jobject,
    v: jobject,
    key_code: jint,
    event: jobject,
) -> jboolean {
    // SAFETY: a native method's environment and arguments.
    let (env, this, v, event) = unsafe {
        (
            Env::from_raw(env),
            Obj::from_raw(this),
            Obj::from_raw(v),
            Obj::from_raw(event),
        )
    };
    let result: JniResult<'_, bool> = (|| {
        let ic = env.get_field(this, c"ic", INPUT_CONNECTION)?.object();
        activity::handle_key_event(
            env,
            v,
            key_code,
            event,
            ic.as_ref().map_or(Obj::NULL, Local::obj),
        )
    })();
    match result {
        Ok(handled) => handled as jboolean,
        Err(thrown) => {
            // (pending, for the framework)
            drop(thrown);
            JNI_FALSE
        }
    }
}

/// Translation of `onKeyPreIme()`.
pub(crate) extern "system" fn on_key_pre_ime(
    env: *mut JNIEnv,
    this: jobject,
    key_code: jint,
    event: jobject,
) -> jboolean {
    // SAFETY: a native method's environment and arguments.
    let (env, this, event) = unsafe {
        (
            Env::from_raw(env),
            Obj::from_raw(this),
            Obj::from_raw(event),
        )
    };
    let result: JniResult<'_, bool> = (|| {
        // As seen on StackOverflow: http://stackoverflow.com/questions/7634346/keyboard-hide-event
        // FIXME: Discussion at http://bugzilla.libsdl.org/show_bug.cgi?id=1639
        // FIXME: This is not a 100% effective solution to the problem of detecting if the keyboard is showing or not
        // FIXME: A more effective solution would be to assume our Layout to be RelativeLayout or LinearLayout
        // FIXME: And determine the keyboard presence doing this: http://stackoverflow.com/questions/2150078/how-to-check-visibility-of-software-keyboard-in-android
        // FIXME: An even more effective way would be if Android provided this out of the box, but where would the fun be in that :)
        if env.call_method(event, c"getAction", c"()I", &[])?.int() == ACTION_UP
            && key_code == KEYCODE_BACK
        {
            if let Some(text_edit) = activity::text_edit(env) {
                if env
                    .call_method(text_edit.obj(), c"getVisibility", c"()I", &[])?
                    .int()
                    == VISIBLE
                {
                    super::super::on_keyboard_focus_lost();
                }
            }
        }
        let view = env.find_class(c"android/view/View")?;
        Ok(env
            .call_nonvirtual_method(
                this,
                view.obj(),
                c"onKeyPreIme",
                c"(ILandroid/view/KeyEvent;)Z",
                &[Arg::Int(key_code), Arg::Object(event)],
            )?
            .boolean())
    })();
    match result {
        Ok(result) => result as jboolean,
        Err(thrown) => {
            // (pending, for the framework)
            drop(thrown);
            JNI_FALSE
        }
    }
}

/// Translation of `onCreateInputConnection()`.
pub(crate) extern "system" fn on_create_input_connection(
    env: *mut JNIEnv,
    this: jobject,
    out_attrs: jobject,
) -> jobject {
    // SAFETY: a native method's environment and arguments.
    let (env, this, out_attrs) = unsafe {
        (
            Env::from_raw(env),
            Obj::from_raw(this),
            Obj::from_raw(out_attrs),
        )
    };
    let result: JniResult<'_, Local<'_>> = (|| {
        let ic = input_connection::new_sdl_input_connection(env, this, true)?;
        env.set_field(this, c"ic", INPUT_CONNECTION, Arg::Object(ic.obj()))?;

        let input_type = env.get_field(this, c"input_type", c"I")?.int();
        env.set_field(out_attrs, c"inputType", c"I", Arg::Int(input_type))?;
        env.set_field(
            out_attrs,
            c"imeOptions",
            c"I",
            Arg::Int(
                IME_FLAG_NO_EXTRACT_UI | IME_FLAG_NO_FULLSCREEN, /* API 11 */
            ),
        )?;

        Ok(ic)
    })();
    match result {
        Ok(ic) => ic.into_raw(),
        Err(thrown) => {
            // (pending, for the framework)
            drop(thrown);
            std::ptr::null_mut()
        }
    }
}
