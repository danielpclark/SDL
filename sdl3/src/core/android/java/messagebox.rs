// Rust translation of android-project/app/src/main/java/org/libsdl/app/
// SDLActivity.java from Simple DirectMedia Layer (part: the message boxes,
// `messageboxShowMessageBox()` and `messageboxCreateAndShow()`).
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! SDL's message boxes: an `AlertDialog` of the activity, made and shown on
//! the main thread ([`messagebox_create_and_show`]: its title, the message
//! in a `TextView`, a row of `Button`s, the color scheme's colors where it
//! has one, the keys of the buttons that are the return and escape keys'
//! defaults), while the calling thread (SDL's) waits until it's dismissed
//! ([`messagebox_show_message_box`]): a button dismisses it, and its ID is
//! the result (-1 if it's dismissed otherwise).
//!
//! The dialog's anonymous listeners (`DialogInterface.OnDismissListener`,
//! `View.OnClickListener`, `Dialog.OnKeyListener`) are proxies
//! (`super::anonymous::listener`), and its `Runnable` an
//! `SDLNativeRunnable`. Note: upstream passes the arguments to the main
//! thread in a `Bundle`; here they're a Rust struct ([`MessageBoxArgs`]).
//! Note: upstream's calling thread waits on `messageboxSelection`'s
//! monitor (`wait()`, which returns on a spurious wakeup too, with -1, and
//! would miss a dismissal that came before it waits); here it waits until
//! the dismissal's notification has come.

use super::super::env::{Arg, Env, Global, JniResult, Local, Obj};
use super::super::jni::*;
use super::anonymous;
use std::sync::{Condvar, Mutex};

/// `Color.TRANSPARENT`.
const TRANSPARENT: jint = 0;
/// `Gravity.CENTER`.
const GRAVITY_CENTER: jint = 17;
/// `LinearLayout.HORIZONTAL`, `LinearLayout.VERTICAL`.
const HORIZONTAL: jint = 0;
const VERTICAL: jint = 1;
/// `KeyEvent.KEYCODE_ENTER`, `KeyEvent.KEYCODE_ESCAPE` (API 11).
const KEYCODE_ENTER: jint = 66;
const KEYCODE_ESCAPE: jint = 111;
/// `KeyEvent.ACTION_UP`.
const ACTION_UP: jint = 1;

/// The result of the current messagebox (`messageboxSelection[0]`), and
/// whether its dismissal has woken the calling thread.
struct Selection {
    value: jint,
    dismissed: bool,
}

/// `messageboxSelection`, also used for blocking the calling thread.
static MESSAGEBOX_SELECTION: Mutex<Selection> = Mutex::new(Selection {
    value: -1,
    dismissed: false,
});
static MESSAGEBOX_DISMISSED: Condvar = Condvar::new();

fn selection() -> std::sync::MutexGuard<'static, Selection> {
    MESSAGEBOX_SELECTION
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// The arguments for the dialog (upstream's `Bundle`): the flags, the
/// title and message, the buttons' flags, IDs and texts, and the colors
/// (none for the defaults, else the color scheme's five, as ARGB).
pub(crate) struct MessageBoxArgs {
    pub(crate) flags: jint,
    pub(crate) title: String,
    pub(crate) message: String,
    pub(crate) button_flags: Vec<jint>,
    pub(crate) button_ids: Vec<jint>,
    pub(crate) button_texts: Vec<String>,
    pub(crate) colors: Option<Vec<jint>>,
}

/// This method is called by SDL using JNI. Shows the messagebox from UI
/// thread and block calling thread. `button_flags`, `button_ids` and
/// `button_texts` must have same length; the result is the button's ID,
/// or -1. Translation of `messageboxShowMessageBox()`.
pub(crate) fn messagebox_show_message_box<'a>(
    env: Env<'a>,
    this: Obj<'_>,
    args: MessageBoxArgs,
) -> JniResult<'a, jint> {
    {
        let mut s = selection();
        s.value = -1;
        s.dismissed = false;
    }

    // sanity checks

    // Note (upstream): `&&` where `||` was meant: lengths that differ
    // pass unless all three do.
    if args.button_flags.len() != args.button_ids.len()
        && args.button_ids.len() != args.button_texts.len()
    {
        return Ok(-1); // implementation broken
    }

    // collect arguments for Dialog

    // (the flags aren't used: see messagebox_create_and_show())
    let _ = args.flags;

    // trigger Dialog creation on UI thread

    let activity = Global::new(env, this).expect("a global reference");
    let runnable = anonymous::runnable(env, move |env| {
        if let Err(thrown) = messagebox_create_and_show(env, activity.obj(), &args) {
            // (pending, for the looper)
            drop(thrown);
        }
    })?;
    env.call_method(
        this,
        c"runOnUiThread",
        c"(Ljava/lang/Runnable;)V",
        &[Arg::Object(runnable.obj())],
    )?;

    // block the calling thread

    let mut s = selection();
    while !s.dismissed {
        s = MESSAGEBOX_DISMISSED
            .wait(s)
            .unwrap_or_else(|e| e.into_inner());
    }

    // return selected value

    Ok(s.value)
}

/// `new CLASS(context)`.
fn new_with_context<'a>(
    env: Env<'a>,
    class: &std::ffi::CStr,
    context: Obj<'_>,
) -> JniResult<'a, Local<'a>> {
    let class = env.find_class(class)?;
    env.new_object(
        class.obj(),
        c"(Landroid/content/Context;)V",
        &[Arg::Object(context)],
    )
}

/// `Boolean.valueOf(b)`, a listener's result.
fn boxed_boolean<'a>(env: Env<'a>, b: bool) -> JniResult<'a, Option<Local<'a>>> {
    let boolean = env.find_class(c"java/lang/Boolean")?;
    Ok(env
        .call_static_method(
            boolean.obj(),
            c"valueOf",
            c"(Z)Ljava/lang/Boolean;",
            &[Arg::Boolean(b)],
        )?
        .object())
}

/// The dialog, made and shown (on the main thread). Translation of
/// `messageboxCreateAndShow()`.
fn messagebox_create_and_show<'a>(
    env: Env<'a>,
    this: Obj<'_>,
    args: &MessageBoxArgs,
) -> JniResult<'a, ()> {
    // TODO set values from "flags" to messagebox dialog

    // get colors

    let colors = args.colors.as_deref();
    let (
        background_color,
        text_color,
        button_border_color,
        button_background_color,
        button_selected_color,
    ) = match colors {
        Some(colors) => {
            let color = |i: usize| -> JniResult<'a, jint> {
                match colors.get(i) {
                    Some(&c) => Ok(c),
                    None => {
                        // (colors[++i] past the array's end)
                        let e = env.find_class(c"java/lang/ArrayIndexOutOfBoundsException")?;
                        env.throw_new(e.obj(), c"colors");
                        env.pending()?;
                        unreachable!("ThrowNew() threw")
                    }
                }
            };
            (color(0)?, color(1)?, color(2)?, color(3)?, color(4)?)
        }
        None => (
            TRANSPARENT,
            TRANSPARENT,
            TRANSPARENT,
            TRANSPARENT,
            TRANSPARENT,
        ),
    };

    // create dialog with title and a listener to wake up calling thread

    let builder = new_with_context(env, c"android/app/AlertDialog$Builder", this)?;
    let dialog = env
        .call_method(
            builder.obj(),
            c"create",
            c"()Landroid/app/AlertDialog;",
            &[],
        )?
        .object()
        .expect("a dialog");
    let title = env.new_string(&args.title)?;
    env.call_method(
        dialog.obj(),
        c"setTitle",
        c"(Ljava/lang/CharSequence;)V",
        &[Arg::Object(title.obj())],
    )?;
    env.call_method(
        dialog.obj(),
        c"setCancelable",
        c"(Z)V",
        &[Arg::Boolean(false)],
    )?;
    let on_dismiss = anonymous::listener(
        env,
        c"android/content/DialogInterface$OnDismissListener",
        |_env, method, _args| {
            if method == "onDismiss" {
                let mut s = selection();
                s.dismissed = true;
                MESSAGEBOX_DISMISSED.notify_all();
            }
            Ok(None)
        },
    )?;
    env.call_method(
        dialog.obj(),
        c"setOnDismissListener",
        c"(Landroid/content/DialogInterface$OnDismissListener;)V",
        &[Arg::Object(on_dismiss.obj())],
    )?;

    // create text

    let message = new_with_context(env, c"android/widget/TextView", this)?;
    env.call_method(
        message.obj(),
        c"setGravity",
        c"(I)V",
        &[Arg::Int(GRAVITY_CENTER)],
    )?;
    let text = env.new_string(&args.message)?;
    env.call_method(
        message.obj(),
        c"setText",
        c"(Ljava/lang/CharSequence;)V",
        &[Arg::Object(text.obj())],
    )?;
    if text_color != TRANSPARENT {
        env.call_method(
            message.obj(),
            c"setTextColor",
            c"(I)V",
            &[Arg::Int(text_color)],
        )?;
    }

    // create buttons

    let sparse_array = env.find_class(c"android/util/SparseArray")?;
    let mapping = env.new_object(sparse_array.obj(), c"()V", &[])?;

    let buttons = new_with_context(env, c"android/widget/LinearLayout", this)?;
    env.call_method(
        buttons.obj(),
        c"setOrientation",
        c"(I)V",
        &[Arg::Int(HORIZONTAL)],
    )?;
    env.call_method(
        buttons.obj(),
        c"setGravity",
        c"(I)V",
        &[Arg::Int(GRAVITY_CENTER)],
    )?;
    for (i, button_text) in args.button_texts.iter().enumerate() {
        let button = new_with_context(env, c"android/widget/Button", this)?;
        let id = array_element(env, &args.button_ids, i)?;
        let dialog_ref = Global::new(env, dialog.obj()).expect("a global reference");
        let on_click = anonymous::listener(
            env,
            c"android/view/View$OnClickListener",
            move |env, method, _args| {
                if method == "onClick" {
                    selection().value = id;
                    env.call_method(dialog_ref.obj(), c"dismiss", c"()V", &[])?;
                }
                Ok(None)
            },
        )?;
        env.call_method(
            button.obj(),
            c"setOnClickListener",
            c"(Landroid/view/View$OnClickListener;)V",
            &[Arg::Object(on_click.obj())],
        )?;
        let button_flags = array_element(env, &args.button_flags, i)?;
        if button_flags != 0 {
            // see SDL_messagebox.h
            let put = |keycode: jint| {
                env.call_method(
                    mapping.obj(),
                    c"put",
                    c"(ILjava/lang/Object;)V",
                    &[Arg::Int(keycode), Arg::Object(button.obj())],
                )
            };
            if (button_flags & 0x00000001) != 0 {
                put(KEYCODE_ENTER)?;
            }
            if (button_flags & 0x00000002) != 0 {
                put(KEYCODE_ESCAPE)?; /* API 11 */
            }
        }
        let label = env.new_string(button_text)?;
        env.call_method(
            button.obj(),
            c"setText",
            c"(Ljava/lang/CharSequence;)V",
            &[Arg::Object(label.obj())],
        )?;
        if text_color != TRANSPARENT {
            env.call_method(
                button.obj(),
                c"setTextColor",
                c"(I)V",
                &[Arg::Int(text_color)],
            )?;
        }
        if button_border_color != TRANSPARENT {
            // TODO set color for border of messagebox button
        }
        if button_background_color != TRANSPARENT {
            let drawable = env
                .call_method(
                    button.obj(),
                    c"getBackground",
                    c"()Landroid/graphics/drawable/Drawable;",
                    &[],
                )?
                .object();
            match drawable {
                None => {
                    // setting the color this way removes the style
                    env.call_method(
                        button.obj(),
                        c"setBackgroundColor",
                        c"(I)V",
                        &[Arg::Int(button_background_color)],
                    )?;
                }
                Some(drawable) => {
                    // setting the color this way keeps the style (gradient, padding, etc.)
                    let mode_class = env.find_class(c"android/graphics/PorterDuff$Mode")?;
                    let multiply = env
                        .get_static_field(
                            mode_class.obj(),
                            c"MULTIPLY",
                            c"Landroid/graphics/PorterDuff$Mode;",
                        )?
                        .object()
                        .expect("PorterDuff.Mode.MULTIPLY");
                    env.call_method(
                        drawable.obj(),
                        c"setColorFilter",
                        c"(ILandroid/graphics/PorterDuff$Mode;)V",
                        &[
                            Arg::Int(button_background_color),
                            Arg::Object(multiply.obj()),
                        ],
                    )?;
                }
            }
        }
        if button_selected_color != TRANSPARENT {
            // TODO set color for selected messagebox button
        }
        env.call_method(
            buttons.obj(),
            c"addView",
            c"(Landroid/view/View;)V",
            &[Arg::Object(button.obj())],
        )?;
    }

    // create content

    let content = new_with_context(env, c"android/widget/LinearLayout", this)?;
    env.call_method(
        content.obj(),
        c"setOrientation",
        c"(I)V",
        &[Arg::Int(VERTICAL)],
    )?;
    env.call_method(
        content.obj(),
        c"addView",
        c"(Landroid/view/View;)V",
        &[Arg::Object(message.obj())],
    )?;
    env.call_method(
        content.obj(),
        c"addView",
        c"(Landroid/view/View;)V",
        &[Arg::Object(buttons.obj())],
    )?;
    if background_color != TRANSPARENT {
        env.call_method(
            content.obj(),
            c"setBackgroundColor",
            c"(I)V",
            &[Arg::Int(background_color)],
        )?;
    }

    // add content to dialog and return

    env.call_method(
        dialog.obj(),
        c"setView",
        c"(Landroid/view/View;)V",
        &[Arg::Object(content.obj())],
    )?;
    let mapping = Global::new(env, mapping.obj()).expect("a global reference");
    let on_key = anonymous::listener(
        env,
        c"android/content/DialogInterface$OnKeyListener",
        move |env, method, args| {
            if method != "onKey" {
                return Ok(None);
            }
            // onKey(DialogInterface d, int keyCode, KeyEvent event)
            let key_code = env
                .get_object_array_element(args, 1)?
                .expect("onKey()'s key code");
            let key_code = env
                .call_method(key_code.obj(), c"intValue", c"()I", &[])?
                .int();
            let event = env
                .get_object_array_element(args, 2)?
                .expect("onKey()'s event");
            let button = env
                .call_method(
                    mapping.obj(),
                    c"get",
                    c"(I)Ljava/lang/Object;",
                    &[Arg::Int(key_code)],
                )?
                .object();
            if let Some(button) = button {
                let action = env
                    .call_method(event.obj(), c"getAction", c"()I", &[])?
                    .int();
                if action == ACTION_UP {
                    env.call_method(button.obj(), c"performClick", c"()Z", &[])?;
                }
                return boxed_boolean(env, true); // also for ignored actions
            }
            boxed_boolean(env, false)
        },
    )?;
    env.call_method(
        dialog.obj(),
        c"setOnKeyListener",
        c"(Landroid/content/DialogInterface$OnKeyListener;)V",
        &[Arg::Object(on_key.obj())],
    )?;

    env.call_method(dialog.obj(), c"show", c"()V", &[])?;
    Ok(())
}

/// `array[i]`, or Java's `ArrayIndexOutOfBoundsException`.
fn array_element<'a>(env: Env<'a>, array: &[jint], i: usize) -> JniResult<'a, jint> {
    match array.get(i) {
        Some(&value) => Ok(value),
        None => {
            let e = env.find_class(c"java/lang/ArrayIndexOutOfBoundsException")?;
            env.throw_new(e.obj(), c"messagebox");
            env.pending()?;
            unreachable!("ThrowNew() threw")
        }
    }
}
