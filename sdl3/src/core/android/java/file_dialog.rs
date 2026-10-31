// Rust translation of android-project/app/src/main/java/org/libsdl/app/
// SDLActivity.java from Simple DirectMedia Layer (part: the file dialogs,
// `showFileDialog()`, `onActivityResult()` and `SDLFileDialogState`).
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! SDL's file dialogs: the system's documents' picker (the Storage Access
//! Framework), started for a result ([`show_file_dialog`]):
//! `ACTION_OPEN_DOCUMENT` (open a file: one, or several), whose filters'
//! extensions are turned into MIME types (`MimeTypeMap`; `*` alone is
//! `*/*`), `ACTION_CREATE_DOCUMENT` (save a file, the initial path's last
//! part its suggested name) and `ACTION_OPEN_DOCUMENT_TREE` (open a
//! folder, its access made persistent when
//! `SDL_ANDROID_ALLOW_PERSISTENT_FOLDER_ACCESS` says so), the initial path
//! a URI (`DocumentsContract.EXTRA_INITIAL_URI`). The picker's result
//! comes to the activity's `onActivityResult()` ([`on_activity_result`])
//! with the request's code: the chosen documents' URIs (none when it was
//! cancelled) go to SDL (`onNativeFileDialog()`), with no filter (-1).
//!
//! `SDLFileDialogState`, the dialog being shown (a static class of plain
//! fields), is a Rust struct.

use super::super::env::{Arg, Env, JniResult, Local, Obj};
use super::super::jni::*;
use super::super::ndk::LogPriority;
use super::activity;
use std::sync::Mutex;

// File dialog types
const SDL_FILEDIALOG_OPENFILE: jint = 0;
const SDL_FILEDIALOG_SAVEFILE: jint = 1;
const SDL_FILEDIALOG_OPENFOLDER: jint = 2;

/// `Activity.RESULT_OK`.
const RESULT_OK: jint = -1;
/// `Intent.FLAG_GRANT_READ_URI_PERMISSION`, `_WRITE_`, `_PERSISTABLE_`,
/// `_PREFIX_`.
const FLAG_GRANT_READ_URI_PERMISSION: jint = 0x00000001;
const FLAG_GRANT_WRITE_URI_PERMISSION: jint = 0x00000002;
const FLAG_GRANT_PERSISTABLE_URI_PERMISSION: jint = 0x00000040;
const FLAG_GRANT_PREFIX_URI_PERMISSION: jint = 0x00000080;

/// Internal class used to track active file dialog. Translation of
/// `SDLFileDialogState`.
#[derive(Clone, Copy, Debug)]
struct SdlFileDialogState {
    request_code: jint,
    r#type: jint,
    persistable: bool,
}

/// `mFileDialogState`.
static FILE_DIALOG_STATE: Mutex<Option<SdlFileDialogState>> = Mutex::new(None);

fn file_dialog_state() -> std::sync::MutexGuard<'static, Option<SdlFileDialogState>> {
    FILE_DIALOG_STATE.lock().unwrap_or_else(|e| e.into_inner())
}

fn log(priority: LogPriority, message: &str) {
    super::super::ndk::log_write(priority, "SDL", message);
}

/// The activity's result of a request: the picker's, when it's the file
/// dialog's (the documents' URIs to SDL, as strings). Translation of
/// `onActivityResult()`.
pub(crate) extern "system" fn on_activity_result(
    env: *mut JNIEnv,
    this: jobject,
    request_code: jint,
    result_code: jint,
    data: jobject,
) {
    // SAFETY: a native method's environment and arguments.
    let (env, this, data) =
        unsafe { (Env::from_raw(env), Obj::from_raw(this), Obj::from_raw(data)) };
    if let Err(thrown) = activity_result(env, this, request_code, result_code, data) {
        // (pending, for the framework)
        drop(thrown);
    }
}

fn activity_result<'a>(
    env: Env<'a>,
    this: Obj<'_>,
    request_code: jint,
    result_code: jint,
    data: Obj<'_>,
) -> JniResult<'a, ()> {
    activity::call_super(
        env,
        this,
        c"onActivityResult",
        c"(IILandroid/content/Intent;)V",
        &[
            Arg::Int(request_code),
            Arg::Int(result_code),
            Arg::Object(data),
        ],
    )?;

    let Some(state) = *file_dialog_state() else {
        return Ok(());
    };
    if state.request_code != request_code {
        return Ok(());
    }
    /* This is our file dialog */
    let filelist: Vec<String>;

    if !data.is_null() && result_code == RESULT_OK {
        let single_file_uri = env
            .call_method(data, c"getData", c"()Landroid/net/Uri;", &[])?
            .object();

        match single_file_uri {
            None => {
                /* Use Intent.getClipData to get multiple choices */
                let clip_data = env
                    .call_method(data, c"getClipData", c"()Landroid/content/ClipData;", &[])?
                    .object();
                // (assert clipData != null: assertions are off in an app)
                let clip_data = clip_data.as_ref().map_or(Obj::NULL, Local::obj);
                let count = env
                    .call_method(clip_data, c"getItemCount", c"()I", &[])?
                    .int();

                let mut list = Vec::with_capacity(count.max(0) as usize);
                for i in 0..count {
                    let item = env
                        .call_method(
                            clip_data,
                            c"getItemAt",
                            c"(I)Landroid/content/ClipData$Item;",
                            &[Arg::Int(i)],
                        )?
                        .object();
                    let uri = env
                        .call_method(
                            item.as_ref().map_or(Obj::NULL, Local::obj),
                            c"getUri",
                            c"()Landroid/net/Uri;",
                            &[],
                        )?
                        .object();
                    list.push(uri_to_string(
                        env,
                        uri.as_ref().map_or(Obj::NULL, Local::obj),
                    )?);
                }
                filelist = list;
            }
            Some(single_file_uri) => {
                /* If the user selected a directory and the persistent permission hint has been set,
                make the permission persistable */
                if state.r#type == SDL_FILEDIALOG_OPENFOLDER && state.persistable {
                    let singleton = activity::singleton(env);
                    let resolver = env
                        .call_method(
                            singleton.as_ref().map_or(Obj::NULL, Local::obj),
                            c"getContentResolver",
                            c"()Landroid/content/ContentResolver;",
                            &[],
                        )?
                        .object();
                    env.call_method(
                        resolver.as_ref().map_or(Obj::NULL, Local::obj),
                        c"takePersistableUriPermission",
                        c"(Landroid/net/Uri;I)V",
                        &[
                            Arg::Object(single_file_uri.obj()),
                            Arg::Int(
                                FLAG_GRANT_READ_URI_PERMISSION | FLAG_GRANT_WRITE_URI_PERMISSION,
                            ),
                        ],
                    )?;
                }
                /* Only one file is selected. */
                filelist = vec![uri_to_string(env, single_file_uri.obj())?];
            }
        }
    } else {
        /* User cancelled the request. */
        filelist = Vec::new();
    }

    // TODO: Detect the file MIME type and pass the filter value accordingly.
    super::super::on_file_dialog(request_code, Some(filelist), -1);
    *file_dialog_state() = None;
    Ok(())
}

/// `uri.toString()`.
fn uri_to_string<'a>(env: Env<'a>, uri: Obj<'_>) -> JniResult<'a, String> {
    let string = env
        .call_method(uri, c"toString", c"()Ljava/lang/String;", &[])?
        .object()
        .expect("a URI's string");
    env.get_string(string.obj())
}

/// This method is called by SDL using JNI: the picker of `type` (0: open a
/// file, 1: save one, 2: open a folder) started for the result
/// `request_code`, with the MIME types of `filters`' extensions, several
/// documents when `allow_multiple` (when opening files), at the URI
/// `initial_path`; false without an activity, or when no activity answers
/// the intent. Translation of `showFileDialog()`.
pub(crate) fn show_file_dialog<'a>(
    env: Env<'a>,
    filters: Option<&[String]>,
    mut allow_multiple: bool,
    r#type: jint,
    initial_path: Option<&str>,
    request_code: jint,
) -> JniResult<'a, bool> {
    let Some(singleton) = activity::singleton(env) else {
        return Ok(false);
    };

    /* Convert string list of extensions to their respective MIME types (not needed for folder selection) */
    let mut mimes: Vec<String> = Vec::new();
    let mime_type_map_class = env.find_class(c"android/webkit/MimeTypeMap")?;
    let mime_type_map = env
        .call_static_method(
            mime_type_map_class.obj(),
            c"getSingleton",
            c"()Landroid/webkit/MimeTypeMap;",
            &[],
        )?
        .object()
        .expect("the MIME type map");
    if let Some(filters) = filters.filter(|_| r#type != SDL_FILEDIALOG_OPENFOLDER) {
        for pattern in filters {
            let extensions = java_split(pattern, ';');

            if extensions.len() == 1 && extensions[0] == "*" {
                /* Handle "*" special case */
                mimes.push("*/*".to_string());
            } else {
                for ext in extensions {
                    let ext = env.new_string(ext)?;
                    let mime = env
                        .call_method(
                            mime_type_map.obj(),
                            c"getMimeTypeFromExtension",
                            c"(Ljava/lang/String;)Ljava/lang/String;",
                            &[Arg::Object(ext.obj())],
                        )?
                        .object();
                    if let Some(mime) = mime {
                        mimes.push(env.get_string(mime.obj())?);
                    }
                }
            }
        }
    }

    /* Handle the initial path, if set */
    let mut initial_path_uri = None;

    if let Some(initial_path) = initial_path.filter(|p| !p.is_empty()) {
        let uri_class = env.find_class(c"android/net/Uri")?;
        let path = env.new_string(initial_path)?;
        match env.call_static_method(
            uri_class.obj(),
            c"parse",
            c"(Ljava/lang/String;)Landroid/net/Uri;",
            &[Arg::Object(path.obj())],
        ) {
            Ok(uri) => initial_path_uri = uri.object(),
            Err(thrown) => {
                // catch (Exception e)
                drop(thrown.catch_exception()?);
                log(
                    LogPriority::Error,
                    "Failed to parse initial path URI, ignoring initial path",
                );
            }
        }
    }

    let persistable =
        super::super::get_hint_boolean("SDL_ANDROID_ALLOW_PERSISTENT_FOLDER_ACCESS", false);

    /* Select the intent based on the type */
    let action = match r#type {
        SDL_FILEDIALOG_OPENFILE => "android.intent.action.OPEN_DOCUMENT", // Intent.ACTION_OPEN_DOCUMENT
        SDL_FILEDIALOG_SAVEFILE => {
            allow_multiple = false;
            "android.intent.action.CREATE_DOCUMENT" // Intent.ACTION_CREATE_DOCUMENT
        }
        SDL_FILEDIALOG_OPENFOLDER => "android.intent.action.OPEN_DOCUMENT_TREE", // Intent.ACTION_OPEN_DOCUMENT_TREE
        _ => {
            log(
                LogPriority::Error,
                &format!("Unsupported file dialog type: {}", r#type),
            );
            return Ok(false);
        }
    };

    /* Prepare the intent with the proper values */
    let intent_class = env.find_class(c"android/content/Intent")?;
    let action = env.new_string(action)?;
    let intent = env.new_object(
        intent_class.obj(),
        c"(Ljava/lang/String;)V",
        &[Arg::Object(action.obj())],
    )?;
    let put_string = |name: &str, value: Obj<'_>| -> JniResult<'a, ()> {
        let name = env.new_string(name)?;
        env.call_method(
            intent.obj(),
            c"putExtra",
            c"(Ljava/lang/String;Ljava/lang/String;)Landroid/content/Intent;",
            &[Arg::Object(name.obj()), Arg::Object(value)],
        )?;
        Ok(())
    };
    let set_type = |mime: &str| -> JniResult<'a, ()> {
        let mime = env.new_string(mime)?;
        env.call_method(
            intent.obj(),
            c"setType",
            c"(Ljava/lang/String;)Landroid/content/Intent;",
            &[Arg::Object(mime.obj())],
        )?;
        Ok(())
    };
    if r#type != SDL_FILEDIALOG_OPENFOLDER {
        let category = env.new_string("android.intent.category.OPENABLE")?; // Intent.CATEGORY_OPENABLE
        env.call_method(
            intent.obj(),
            c"addCategory",
            c"(Ljava/lang/String;)Landroid/content/Intent;",
            &[Arg::Object(category.obj())],
        )?;
        let name = env.new_string("android.intent.extra.ALLOW_MULTIPLE")?; // Intent.EXTRA_ALLOW_MULTIPLE
        env.call_method(
            intent.obj(),
            c"putExtra",
            c"(Ljava/lang/String;Z)Landroid/content/Intent;",
            &[Arg::Object(name.obj()), Arg::Boolean(allow_multiple)],
        )?;
        match mimes.len() {
            0 => set_type("*/*")?,
            1 => set_type(&mimes[0])?,
            _ => {
                set_type("*/*")?;
                let string = env.find_class(c"java/lang/String")?;
                let array = env.new_object_array(mimes.len() as jsize, string.obj(), Obj::NULL)?;
                for (i, mime) in mimes.iter().enumerate() {
                    let mime = env.new_string(mime)?;
                    env.set_object_array_element(array.obj(), i as jsize, mime.obj())?;
                }
                let name = env.new_string("android.intent.extra.MIME_TYPES")?; // Intent.EXTRA_MIME_TYPES
                env.call_method(
                    intent.obj(),
                    c"putExtra",
                    c"(Ljava/lang/String;[Ljava/lang/String;)Landroid/content/Intent;",
                    &[Arg::Object(name.obj()), Arg::Object(array.obj())],
                )?;
            }
        }
    } else {
        let mut intent_flags = FLAG_GRANT_READ_URI_PERMISSION | FLAG_GRANT_WRITE_URI_PERMISSION;
        if persistable {
            intent_flags |=
                FLAG_GRANT_PREFIX_URI_PERMISSION | FLAG_GRANT_PERSISTABLE_URI_PERMISSION;
        }
        env.call_method(
            intent.obj(),
            c"addFlags",
            c"(I)Landroid/content/Intent;",
            &[Arg::Int(intent_flags)],
        )?;
    }

    if let Some(initial_path_uri) = &initial_path_uri {
        let name = env.new_string("android.provider.extra.INITIAL_URI")?; // DocumentsContract.EXTRA_INITIAL_URI
        env.call_method(
            intent.obj(),
            c"putExtra",
            c"(Ljava/lang/String;Landroid/os/Parcelable;)Landroid/content/Intent;",
            &[Arg::Object(name.obj()), Arg::Object(initial_path_uri.obj())],
        )?;
    }

    /* Handle a suggested filename when saving */
    if let Some(initial_path) = initial_path.filter(|p| {
        r#type == SDL_FILEDIALOG_SAVEFILE
            && !p.is_empty()
            && !p.ends_with('/')
            && !p.ends_with('\\')
    }) {
        let mut title = initial_path;
        let last_separator = title.rfind('/').max(title.rfind('\\'));
        if let Some(last_separator) = last_separator {
            title = &title[last_separator + 1..];
        }
        if !title.is_empty() {
            let title = env.new_string(title)?;
            put_string("android.intent.extra.TITLE", title.obj())?; // Intent.EXTRA_TITLE
        }
    }

    /* Display the file/folder dialog */
    if let Err(thrown) = env.call_method(
        singleton.obj(),
        c"startActivityForResult",
        c"(Landroid/content/Intent;I)V",
        &[Arg::Object(intent.obj()), Arg::Int(request_code)],
    ) {
        // catch (ActivityNotFoundException e)
        drop(thrown.catch_any(&[c"android/content/ActivityNotFoundException"])?);
        log(LogPriority::Error, "Unable to open dialog.");
        return Ok(false);
    }

    /* Save current dialog state */
    *file_dialog_state() = Some(SdlFileDialogState {
        request_code,
        r#type,
        persistable,
    });

    Ok(true)
}

/// Java's `String.split(regex)` of a one-character, non-special regex:
/// the parts between the separators, the trailing empty ones removed
/// (an empty string gives one empty part).
fn java_split(s: &str, separator: char) -> Vec<&str> {
    if s.is_empty() {
        return vec![""];
    }
    let mut parts: Vec<&str> = s.split(separator).collect();
    while parts.len() > 1 && parts.last() == Some(&"") {
        parts.pop();
    }
    if parts.len() == 1 && parts[0].is_empty() {
        // ("" from ";;": all trailing, Java gives an empty array)
        parts.clear();
    }
    parts
}

#[cfg(test)]
mod tests {
    #[test]
    fn java_split() {
        assert_eq!(super::java_split("png;jpg", ';'), ["png", "jpg"]);
        assert_eq!(super::java_split("*", ';'), ["*"]);
        assert_eq!(super::java_split("a;;b;;", ';'), ["a", "", "b"]);
        assert_eq!(super::java_split("", ';'), [""]);
        assert_eq!(super::java_split(";;", ';'), Vec::<&str>::new());
    }
}
