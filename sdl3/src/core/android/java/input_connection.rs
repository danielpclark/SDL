// Rust translation of android-project/app/src/main/java/org/libsdl/app/
// SDLInputConnection.java from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! `class SDLInputConnection extends BaseInputConnection`: the input
//! connection of the activity's text edit ([`super::dummy_edit`]), what
//! the input method commits and composes into. Its text is an `EditText`'s
//! (`getEditable()`, of `mEditText`, which no window shows); after each
//! change (`commitText()`, `setComposingText()`, `deleteSurroundingText()`)
//! `updateText()` compares it with what it last sent (`mCommittedText`):
//! a backspace for each character that is no longer there, then the new
//! characters as key presses (those below 128, unless a key event is being
//! dispatched, which sends its own; a newline may end text input:
//! `SDLActivity.onNativeSoftReturnKey()`) and as text
//! (`nativeCommitText()`). The return key's events end text input the same
//! way (`sendKeyEvent()`), and a deletion before the cursor alone is that
//! many backspaces.
//!
//! The constructor's code (`mEditText`, `mCommittedText`) runs in
//! [`new_sdl_input_connection`], after the DEX file's constructor, which
//! only calls `BaseInputConnection`'s; the fields are the DEX file's
//! (`super::super::classes`), and `updateText()` is a Rust function.
//! `nativeCommitText()` and `nativeGenerateScancodeForUnichar()` are the
//! class's natives, which this file calls as Rust functions.

use super::super::env::{Arg, Env, JniResult, Local, Obj};
use super::super::jni::*;
use super::{activity, sdl};

/// `KeyEvent.KEYCODE_ENTER`.
const KEYCODE_ENTER: jint = 66;

/// `mEditText`'s and `mCommittedText`'s descriptors.
const EDIT_TEXT: &std::ffi::CStr = c"Landroid/widget/EditText;";
const STRING: &std::ffi::CStr = c"Ljava/lang/String;";

/// `android.view.inputmethod.BaseInputConnection`, the superclass, for
/// `super.method()`.
fn base_input_connection<'a>(env: Env<'a>) -> JniResult<'a, Local<'a>> {
    env.find_class(c"android/view/inputmethod/BaseInputConnection")
}

/// `super.name(args)` of a method returning a boolean.
fn call_super_boolean<'a>(
    env: Env<'a>,
    this: Obj<'_>,
    name: &std::ffi::CStr,
    descriptor: &std::ffi::CStr,
    args: &[Arg<'_>],
) -> JniResult<'a, bool> {
    let superclass = base_input_connection(env)?;
    Ok(env
        .call_nonvirtual_method(this, superclass.obj(), name, descriptor, args)?
        .boolean())
}

/// A native method's body returning a boolean (false when it throws, the
/// exception left pending for the caller, as from Java).
fn native_boolean<'a>(result: JniResult<'a, bool>) -> jboolean {
    match result {
        Ok(result) => result as jboolean,
        Err(thrown) => {
            // (pending, for the caller)
            drop(thrown);
            JNI_FALSE
        }
    }
}

/// `new SDLInputConnection(targetView, fullEditor)`: the DEX file's
/// constructor (`super(targetView, fullEditor)`), then the constructor's
/// code and the fields' initial values.
pub(crate) fn new_sdl_input_connection<'a>(
    env: Env<'a>,
    target_view: Obj<'_>,
    full_editor: bool,
) -> JniResult<'a, Local<'a>> {
    let class = super::super::env::app_class(super::super::classes::SDL_INPUT_CONNECTION)
        .expect("JNI_OnLoad() found SDLInputConnection");
    let this = env.new_object(
        class.obj(),
        c"(Landroid/view/View;Z)V",
        &[Arg::Object(target_view), Arg::Boolean(full_editor)],
    )?;
    let empty = env.new_string("")?;
    env.set_field(
        this.obj(),
        c"mCommittedText",
        STRING,
        Arg::Object(empty.obj()),
    )?;

    let edit_text_class = env.find_class(c"android/widget/EditText")?;
    let context = sdl::get_context(env);
    let edit_text = env.new_object(
        edit_text_class.obj(),
        c"(Landroid/content/Context;)V",
        &[Arg::Object(context.as_ref().map_or(Obj::NULL, Local::obj))],
    )?;
    env.set_field(
        this.obj(),
        c"mEditText",
        EDIT_TEXT,
        Arg::Object(edit_text.obj()),
    )?;
    Ok(this)
}

/// Translation of `getEditable()`.
pub(crate) extern "system" fn get_editable(env: *mut JNIEnv, this: jobject) -> jobject {
    // SAFETY: a native method's environment and object.
    let (env, this) = unsafe { (Env::from_raw(env), Obj::from_raw(this)) };
    let editable: JniResult<'_, Option<Local<'_>>> = (|| {
        let Some(edit_text) = env.get_field(this, c"mEditText", EDIT_TEXT)?.object() else {
            let npe = env.find_class(c"java/lang/NullPointerException")?;
            env.throw_new(npe.obj(), c"SDLInputConnection.mEditText is null");
            env.pending()?;
            unreachable!("ThrowNew() threw");
        };
        Ok(env
            .call_method(
                edit_text.obj(),
                c"getEditableText",
                c"()Landroid/text/Editable;",
                &[],
            )?
            .object())
    })();
    match editable {
        Ok(Some(editable)) => editable.into_raw(),
        Ok(None) => std::ptr::null_mut(),
        Err(thrown) => {
            // (pending, for the caller)
            drop(thrown);
            std::ptr::null_mut()
        }
    }
}

/// Translation of `sendKeyEvent()`.
pub(crate) extern "system" fn send_key_event(
    env: *mut JNIEnv,
    this: jobject,
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
    native_boolean((|| {
        /*
         * This used to handle the keycodes from soft keyboard (and IME-translated input from hardkeyboard)
         * However, as of Ice Cream Sandwich and later, almost all soft keyboard doesn't generate key presses
         * and so we need to generate them ourselves in commitText.  To avoid duplicates on the handful of keys
         * that still do, we empty this out.
         */

        /*
         * Return DOES still generate a key event, however.  So rather than using it as the 'click a button' key
         * as we do with physical keyboards, let's just use it to hide the keyboard.
         */

        if env.call_method(event, c"getKeyCode", c"()I", &[])?.int() == KEYCODE_ENTER
            && super::super::on_soft_return_key()
        {
            return Ok(true);
        }

        call_super_boolean(
            env,
            this,
            c"sendKeyEvent",
            c"(Landroid/view/KeyEvent;)Z",
            &[Arg::Object(event)],
        )
    })())
}

/// Translation of `commitText()`.
pub(crate) extern "system" fn commit_text(
    env: *mut JNIEnv,
    this: jobject,
    text: jobject,
    new_cursor_position: jint,
) -> jboolean {
    // SAFETY: a native method's environment and arguments.
    let (env, this, text) =
        unsafe { (Env::from_raw(env), Obj::from_raw(this), Obj::from_raw(text)) };
    native_boolean((|| {
        if !call_super_boolean(
            env,
            this,
            c"commitText",
            c"(Ljava/lang/CharSequence;I)Z",
            &[Arg::Object(text), Arg::Int(new_cursor_position)],
        )? {
            return Ok(false);
        }
        update_text(env, this)?;
        Ok(true)
    })())
}

/// Translation of `setComposingText()`.
pub(crate) extern "system" fn set_composing_text(
    env: *mut JNIEnv,
    this: jobject,
    text: jobject,
    new_cursor_position: jint,
) -> jboolean {
    // SAFETY: a native method's environment and arguments.
    let (env, this, text) =
        unsafe { (Env::from_raw(env), Obj::from_raw(this), Obj::from_raw(text)) };
    native_boolean((|| {
        if !call_super_boolean(
            env,
            this,
            c"setComposingText",
            c"(Ljava/lang/CharSequence;I)Z",
            &[Arg::Object(text), Arg::Int(new_cursor_position)],
        )? {
            return Ok(false);
        }
        update_text(env, this)?;
        Ok(true)
    })())
}

/// Translation of `deleteSurroundingText()`.
pub(crate) extern "system" fn delete_surrounding_text(
    env: *mut JNIEnv,
    this: jobject,
    before_length: jint,
    after_length: jint,
) -> jboolean {
    // SAFETY: a native method's environment and object.
    let (env, this) = unsafe { (Env::from_raw(env), Obj::from_raw(this)) };
    native_boolean((|| {
        // Workaround to capture backspace key. Ref: http://stackoverflow.com/questions>/14560344/android-backspace-in-webview-baseinputconnection
        // and https://bugzilla.libsdl.org/show_bug.cgi?id=2265
        if before_length > 0 && after_length == 0 {
            // backspace(s)
            for _ in 0..before_length {
                super::super::generate_scancode_for_unichar(b'\x08' as jchar);
            }
            return Ok(true);
        }

        if !call_super_boolean(
            env,
            this,
            c"deleteSurroundingText",
            c"(II)Z",
            &[Arg::Int(before_length), Arg::Int(after_length)],
        )? {
            return Ok(false);
        }
        update_text(env, this)?;
        Ok(true)
    })())
}

/// Java's `String.codePointAt(index)` of UTF-16 units: a surrogate pair's
/// code point, else the unit's.
fn code_point_at(units: &[jchar], index: usize) -> u32 {
    let high = units[index];
    if (0xd800..0xdc00).contains(&high) {
        if let Some(&low) = units.get(index + 1) {
            if (0xdc00..0xe000).contains(&low) {
                return 0x10000 + ((u32::from(high) - 0xd800) << 10) + (u32::from(low) - 0xdc00);
            }
        }
    }
    u32::from(high)
}

/// Java's `Character.charCount(codePoint)`.
fn char_count(code_point: u32) -> usize {
    if code_point >= 0x10000 {
        2
    } else {
        1
    }
}

/// Translation of `updateText()`.
fn update_text<'a>(env: Env<'a>, this: Obj<'_>) -> JniResult<'a, ()> {
    // (getEditable(), a virtual call, as Java's)
    let content = env
        .call_method(this, c"getEditable", c"()Landroid/text/Editable;", &[])?
        .object();
    let Some(content) = content else {
        return Ok(());
    };

    let text = env
        .call_method(content.obj(), c"toString", c"()Ljava/lang/String;", &[])?
        .object()
        .expect("toString() isn't null");
    let text_units = env.get_string_utf16(text.obj())?;
    let committed = env.get_field(this, c"mCommittedText", STRING)?.object();
    let committed_units = match &committed {
        Some(committed) => env.get_string_utf16(committed.obj())?,
        None => {
            let npe = env.find_class(c"java/lang/NullPointerException")?;
            env.throw_new(npe.obj(), c"SDLInputConnection.mCommittedText is null");
            env.pending()?;
            unreachable!("ThrowNew() threw");
        }
    };
    let compare_length = text_units.len().min(committed_units.len());

    /* Backspace over characters that are no longer in the string */
    let mut match_length = 0;
    while match_length < compare_length {
        let code_point = code_point_at(&committed_units, match_length);
        if code_point != code_point_at(&text_units, match_length) {
            break;
        }
        match_length += char_count(code_point);
    }
    /* FIXME: This doesn't handle graphemes, like '🌬️' */
    let mut offset = match_length;
    while offset < committed_units.len() {
        let code_point = code_point_at(&committed_units, offset);
        super::super::generate_scancode_for_unichar(b'\x08' as jchar);
        offset += char_count(code_point);
    }

    if match_length < text_units.len() {
        let pending_text = &text_units[match_length..];
        if !activity::dispatching_key_event() {
            let mut offset = 0;
            while offset < pending_text.len() {
                let code_point = code_point_at(pending_text, offset);
                if code_point == u32::from(b'\n') && super::super::on_soft_return_key() {
                    return Ok(());
                }
                /* Higher code points don't generate simulated scancodes */
                if code_point > 0 && code_point < 128 {
                    super::super::generate_scancode_for_unichar(code_point as jchar);
                }
                offset += char_count(code_point);
            }
        }
        // SDLInputConnection.nativeCommitText(pendingText, 0)
        super::super::commit_text(&String::from_utf16_lossy(pending_text));
    }
    env.set_field(this, c"mCommittedText", STRING, Arg::Object(text.obj()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_points() {
        let units: Vec<jchar> = "a\u{e9}\u{1f600}".encode_utf16().collect();
        assert_eq!(code_point_at(&units, 0), 0x61);
        assert_eq!(code_point_at(&units, 1), 0xe9);
        assert_eq!(code_point_at(&units, 2), 0x1f600);
        assert_eq!(code_point_at(&units, 3), 0xde00);
        assert_eq!(char_count(0x1f600), 2);
        // (an unpaired high surrogate at the end)
        assert_eq!(code_point_at(&[0xd83d], 0), 0xd83d);
    }
}
