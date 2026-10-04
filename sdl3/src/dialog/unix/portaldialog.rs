// Rust translation of src/dialog/unix/SDL_portaldialog.c from Simple
// DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! File dialogs through the XDG desktop portal
//! (`org.freedesktop.portal.FileChooser`): the request is a method call,
//! and the result arrives later as a `Response` signal, picked up while the
//! event loop pumps D-Bus.

use std::sync::atomic::{AtomicI8, AtomicU32, Ordering};
use std::sync::Mutex;

use super::super::{
    validate_filters, DialogFileCallback, DialogFileFilter, FileDialogOptions, FileDialogType,
};
use crate::core::linux::dbus::{self, Arg, HandlerResult, Message, Value, Writer};
use crate::error::Error;

const PORTAL_DESTINATION: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const PORTAL_INTERFACE: &str = "org.freedesktop.portal.FileChooser";

const SIGNAL_SENDER: &str = "org.freedesktop.portal.Desktop";
const SIGNAL_INTERFACE: &str = "org.freedesktop.portal.Request";
const SIGNAL_NAME: &str = "Response";

const HANDLE_LEN: usize = 10;

const WAYLAND_HANDLE_PREFIX: &str = "wayland:";
const X11_HANDLE_PREFIX: &str = "x11:";

/// Translation of `SDL_PROP_WINDOW_WAYLAND_XDG_TOPLEVEL_EXPORT_HANDLE_STRING`
/// (set by the Wayland video driver).
const PROP_WINDOW_WAYLAND_XDG_TOPLEVEL_EXPORT_HANDLE_STRING: &str =
    "SDL.window.wayland.xdg_toplevel_export_handle";
/// Translation of `SDL_PROP_WINDOW_X11_WINDOW_NUMBER` (set by the X11 video
/// driver).
const PROP_WINDOW_X11_WINDOW_NUMBER: &str = "SDL.window.x11.window";

/// The match rule for the Response signal on a request path
/// (`SIGNAL_FILTER` followed by the path).
fn signal_filter(path: &str) -> String {
    format!(
        "type='signal', sender='{SIGNAL_SENDER}', interface='{SIGNAL_INTERFACE}', member='{SIGNAL_NAME}', path='{path}'"
    )
}

/// Translation of `DBus_AppendStringOption()`.
fn dbus_append_string_option(options: &mut Writer<'_>, key: &str, value: &str) {
    options.dict_entry(key, "s", |v| v.append(&Arg::Str(value)));
}

/// Translation of `DBus_AppendBoolOption()`.
fn dbus_append_bool_option(options: &mut Writer<'_>, key: &str, value: bool) {
    options.dict_entry(key, "b", |v| v.append(&Arg::Bool(value)));
}

/// The glob patterns of a filter's extension list: case-insensitive
/// (`*.[pP][nN][gG]` for `png`), one per extension. Part of
/// `DBus_AppendFilter()`.
fn filter_glob_patterns(pattern: &str) -> Vec<String> {
    /* Copy the filter string, converting to a case-insensitive version.
     * For example, for case-insensitive matching of '*.png', the pattern '*.[pP][nN][gG]' is used.
     */
    let mut patterns = String::with_capacity(pattern.len() * 4);
    for c in pattern.chars() {
        if c.is_ascii_alphabetic() {
            patterns.push('[');
            patterns.push(c.to_ascii_lowercase());
            patterns.push(c.to_ascii_uppercase());
            patterns.push(']');
        } else {
            patterns.push(c);
        }
    }

    // (strtok_r skips empty tokens)
    patterns
        .split(';')
        .filter(|p| !p.is_empty())
        .map(|pattern| {
            /* Special case: The '*' filter doesn't need to be rewritten */
            if pattern == "*" {
                "*".to_owned()
            } else {
                format!("*.{pattern}")
            }
        })
        .collect()
}

/// Translation of `DBus_AppendFilter()`.
fn dbus_append_filter(parent: &mut Writer<'_>, filter: &DialogFileFilter) {
    let zero = 0;
    parent.container(b'r', None, |filter_entry| {
        filter_entry.append(&Arg::Str(&filter.name))
            && filter_entry.container(b'a', Some("(us)"), |filter_array| {
                filter_glob_patterns(&filter.pattern).iter().all(|glob| {
                    filter_array.container(b'r', None, |filter_array_entry| {
                        filter_array_entry.append(&Arg::U32(zero))
                            && filter_array_entry.append(&Arg::Str(glob))
                    })
                })
            })
    });
}

/// Translation of `DBus_AppendFilters()`.
fn dbus_append_filters(options: &mut Writer<'_>, filters: &[DialogFileFilter]) {
    options.dict_entry("filters", "a(sa(us))", |options_value| {
        options_value.container(b'a', Some("(sa(us))"), |options_value_array| {
            for filter in filters {
                dbus_append_filter(options_value_array, filter);
            }
            true
        })
    });
}

/// Translation of `DBus_AppendByteArray()` (the bytes include the
/// terminating NUL).
fn dbus_append_byte_array(options: &mut Writer<'_>, key: &str, value: &str) {
    let mut bytes = value.as_bytes().to_vec();
    bytes.push(0);
    options.dict_entry(key, "ay", |v| v.append_bytes(&bytes));
}

/// What the Response signal says. Translation of the parsing in
/// `DBus_MessageFilter()`; `None` for "not our signal".
fn parse_response(msg: &Message) -> Option<crate::error::Result<Vec<String>>> {
    let args = msg.args();
    // Check if the parameters are what we expect
    let result = match args.first() {
        Some(Value::U32(r)) => *r,
        _ => return None,
    };

    if result == 1 || result == 2 {
        // cancelled
        return Some(Ok(Vec::new())); // TODO: Set this to the last selected filter
    } else if result != 0 {
        // some error occurred
        // (upstream reports it without setting an error message)
        return Some(Err(Error::new(format!(
            "Portal dialogs: the request failed ({result})"
        ))));
    }

    let Some(Value::Array(results)) = args.get(1) else {
        return None;
    };

    // FIXME (upstream): with an empty results dictionary, upstream reads
    // an uninitialized iterator; here (as for a dictionary without "uris")
    // the signal is not taken as ours, and the dialog never completes.
    let uris = results.iter().find_map(|entry| match entry {
        Value::DictEntry(key, value) if key.as_str() == Some("uris") => Some(&**value),
        _ => None,
    })?;

    let Value::Variant(uris) = uris else {
        return None;
    };
    let Value::Array(uris) = &**uris else {
        return None;
    };

    let mut paths = Vec::new();
    for uri in uris {
        let Value::Str(uri) = uri else {
            break;
        };

        // https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.FileChooser.html
        // Returned paths will always start with 'file://'; SDL_URIToLocal() truncates it.
        match crate::utils::uri_to_local(uri) {
            Some(path) => paths.push(String::from_utf8_lossy(&path).into_owned()),
            None => {
                return Some(Err(Error::new(format!(
                    "Portal dialogs: Unsupported protocol: {uri}"
                ))));
            }
        }
    }
    Some(Ok(paths)) // TODO: Fetch the index of the filter that was used
}

/// POSIX `dirname()`.
fn dirname(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        return if path.is_empty() { "." } else { "/" }.to_owned();
    }
    match trimmed.rfind('/') {
        None => ".".to_owned(),
        Some(i) => {
            let dir = trimmed[..i].trim_end_matches('/');
            if dir.is_empty() {
                "/".to_owned()
            } else {
                dir.to_owned()
            }
        }
    }
}

/// POSIX `basename()`.
fn basename(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        return if path.is_empty() { "." } else { "/" }.to_owned();
    }
    match trimmed.rfind('/') {
        None => trimmed.to_owned(),
        Some(i) => trimmed[i + 1..].to_owned(),
    }
}

/// The portal's parent window handle for a window: `wayland:<export
/// handle>` or `x11:<hex XID>`, else empty.
fn parent_window_handle(window: Option<crate::events::WindowID>) -> String {
    static DEFAULT_PARENT_WINDOW: &str = "";

    let window_props = window
        .and_then(|id| crate::video::Window::from_id(id).ok())
        .and_then(|w| w.properties().ok());
    if let Some(window_props) = window_props {
        if let Some(parent_handle) =
            window_props.get_string(PROP_WINDOW_WAYLAND_XDG_TOPLEVEL_EXPORT_HANDLE_STRING)
        {
            return format!("{WAYLAND_HANDLE_PREFIX}{parent_handle}");
        }
        let xid = window_props
            .get_number(PROP_WINDOW_X11_WINDOW_NUMBER)
            .unwrap_or(0) as u64;
        if xid != 0 {
            // The portal wants X11 window ID numbers in hex.
            return format!("{X11_HANDLE_PREFIX}{xid:x}");
        }
    }
    DEFAULT_PARENT_WINDOW.to_owned()
}

/// Translation of `SDL_Portal_ShowFileDialogWithProperties()`.
pub(super) fn portal_show_file_dialog(
    dialog_type: FileDialogType,
    callback: DialogFileCallback,
    options: FileDialogOptions,
) {
    let Some(ctx) = dbus::context() else {
        // FIXME (upstream): dbus->error_init() is called before the check
        // for a missing D-Bus context (a NULL dereference).
        return callback(Err(Error::new("Failed to connect to DBus")), None);
    };
    portal_show_file_dialog_on(&ctx.session_conn, dialog_type, callback, options);
}

fn portal_show_file_dialog_on(
    conn: &dbus::Connection,
    dialog_type: FileDialogType,
    callback: DialogFileCallback,
    options: FileDialogOptions,
) {
    static HANDLE_ID: AtomicU32 = AtomicU32::new(0);

    let FileDialogOptions {
        filters,
        window,
        location: default_location,
        many: allow_many,
        title,
        accept,
        cancel: _,
    } = options;
    let mut location_name = None;
    let mut location_folder = None;
    let mut open_folders = false;
    let mut save_file_existing = false;
    let mut save_file_new_named = false;

    let (method, method_title) = match dialog_type {
        FileDialogType::OpenFile => ("OpenFile", title.unwrap_or_else(|| "Open File".into())),
        FileDialogType::SaveFile => {
            if let Some(default_location) = &default_location {
                match std::fs::metadata(default_location) {
                    Ok(statbuf) => save_file_existing = statbuf.is_file(),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        let folder = dirname(default_location);
                        save_file_new_named =
                            std::fs::metadata(&folder).is_ok_and(|statbuf| statbuf.is_dir());
                        location_folder = Some(folder);
                    }
                    Err(_) => {}
                }

                if save_file_existing || save_file_new_named {
                    location_name = Some(basename(default_location));
                }
            }
            ("SaveFile", title.unwrap_or_else(|| "Save File".into()))
        }
        FileDialogType::OpenFolder => {
            open_folders = true;
            ("OpenFile", title.unwrap_or_else(|| "Open Folder".into()))
        }
    };

    if let Some(err_msg) = validate_filters(&filters) {
        return callback(Err(Error::new(err_msg)), None);
    }

    let Some(mut msg) =
        conn.new_method_call(PORTAL_DESTINATION, PORTAL_PATH, PORTAL_INTERFACE, method)
    else {
        return callback(Err(Error::new("Failed to send message to portal")), None);
    };

    {
        let mut params = msg.writer();
        let handle_str = parent_window_handle(window);
        params.append(&Arg::Str(&handle_str));
        params.append(&Arg::Str(&method_title));
        params.container(b'a', Some("{sv}"), |options| {
            // (snprintf into HANDLE_LEN bytes keeps HANDLE_LEN - 1 characters)
            let mut handle_str =
                (HANDLE_ID.fetch_add(1, Ordering::Relaxed).wrapping_add(1)).to_string();
            handle_str.truncate(HANDLE_LEN - 1);
            dbus_append_string_option(options, "handle_token", &handle_str);

            dbus_append_bool_option(options, "modal", window.is_some());
            if allow_many {
                dbus_append_bool_option(options, "multiple", true);
            }
            if open_folders {
                dbus_append_bool_option(options, "directory", true);
            }
            if !filters.is_empty() {
                dbus_append_filters(options, &filters);
            }
            if let Some(default_location) = &default_location {
                if let (true, Some(name)) = (save_file_existing, &location_name) {
                    /* Open a save dialog at an existing file */
                    dbus_append_byte_array(options, "current_file", default_location);
                    /* Setting "current_name" should not be necessary however the kde-desktop-portal sets the filename without an extension.
                     * An alternative would be to match the extension to a filter and set "current_filter".
                     */
                    dbus_append_string_option(options, "current_name", name);
                } else if let (true, Some(folder), Some(name)) =
                    (save_file_new_named, &location_folder, &location_name)
                {
                    /* Open a save dialog at a location with a suggested name */
                    dbus_append_byte_array(options, "current_folder", folder);
                    dbus_append_string_option(options, "current_name", name);
                } else {
                    dbus_append_byte_array(options, "current_folder", default_location);
                }
            }
            if let Some(accept) = &accept {
                dbus_append_string_option(options, "accept_label", accept);
            }
            true
        });
    }

    let reply = match conn.send_with_reply_and_block(&msg, dbus::TIMEOUT_INFINITE) {
        Ok(reply) => reply,
        Err(e) => {
            return callback(
                Err(Error::new(format!(
                    "Failed to open dialog via DBus, {}",
                    e.message()
                ))),
                None,
            );
        }
    };

    let signal_id = match reply.args().into_iter().next() {
        Some(Value::ObjectPath(path)) => path,
        _ => return callback(Err(Error::new("Invalid response received by DBus")), None),
    };

    // FIXME (upstream): the match rule is never removed, so one stays on the
    // bus for each dialog shown. Kept.
    if let Err(e) = conn.add_match(&signal_filter(&signal_id)) {
        return callback(
            Err(Error::new(format!(
                "Failed to set up DBus listener for dialog, {}",
                e.message()
            ))),
            None,
        );
    }

    /* TODO: This should be registered before opening the portal, or the filter will not catch
             the message if it is sent before we register the filter.
    */
    // (translation of `DBus_MessageFilter()`; the SignalCallback data is
    // what the closure captures)
    let callback = Mutex::new(Some(callback));
    conn.add_oneshot_filter(move |msg| {
        if !msg.is_signal(SIGNAL_INTERFACE, SIGNAL_NAME) || !msg.has_path(&signal_id) {
            return None;
        }
        let result = parse_response(msg)?;
        let callback = callback.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(callback) = callback {
            callback(result, None);
        }
        Some(HandlerResult::Handled)
    });
    conn.flush();
}

/// Translation of `SDL_Portal_detect()`.
pub(super) fn portal_detect() -> bool {
    /// -1: not checked yet.
    static PORTAL_PRESENT: AtomicI8 = AtomicI8::new(-1);

    // No need for this if the result is cached.
    let cached = PORTAL_PRESENT.load(Ordering::Acquire);
    if cached != -1 {
        return cached > 0;
    }

    PORTAL_PRESENT.store(0, Ordering::Release);

    let Some(ctx) = dbus::context() else {
        // ("Failed to connect to DBus!")
        return false;
    };

    let present = portal_detect_on(&ctx.session_conn);
    if present {
        PORTAL_PRESENT.store(1, Ordering::Release); // Found it!
    }
    present
}

fn portal_detect_on(conn: &dbus::Connection) -> bool {
    // Use introspection to get the available services.
    let Some(msg) = conn.new_method_call(
        PORTAL_DESTINATION,
        PORTAL_PATH,
        "org.freedesktop.DBus.Introspectable",
        "Introspect",
    ) else {
        return false;
    };

    let Ok(reply) = conn.send_with_reply_and_block(&msg, -1) else {
        return false;
    };

    /* Introspection gives us a dump of all the services on the destination in XML format, so search the
     * giant string for the file chooser protocol.
     */
    match reply.args().first() {
        Some(Value::Str(reply_str)) => reply_str.contains(PORTAL_INTERFACE),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::linux::dbus::test_bus::{self, peer, Bus};
    use std::collections::VecDeque;
    use std::sync::Arc;

    #[test]
    fn helpers() {
        assert_eq!(
            filter_glob_patterns("png;JPG;*"),
            ["*.[pP][nN][gG]", "*.[jJ][pP][gG]", "*"]
        );
        assert_eq!(filter_glob_patterns("tar.gz"), ["*.[tT][aA][rR].[gG][zZ]"]);
        for (path, dir, base) in [
            ("/a/b", "/a", "b"),
            ("/a/b/", "/a", "b"),
            ("b", ".", "b"),
            ("/b", "/", "b"),
            ("/", "/", "/"),
            ("", ".", "."),
            ("a//b", "a", "b"),
        ] {
            assert_eq!(dirname(path), dir, "{path}");
            assert_eq!(basename(path), base, "{path}");
        }
        assert_eq!(
            signal_filter("/r/1"),
            "type='signal', sender='org.freedesktop.portal.Desktop', interface='org.freedesktop.portal.Request', member='Response', path='/r/1'"
        );
    }

    fn dict_get<'v>(dict: &'v [Value], key: &str) -> Option<&'v Value> {
        dict.iter().find_map(|e| match e {
            Value::DictEntry(k, v) if k.as_str() == Some(key) => Some(v.unvariant()),
            _ => None,
        })
    }

    fn bytes_of(v: Option<&Value>) -> Vec<u8> {
        match v {
            Some(Value::Array(bytes)) => bytes
                .iter()
                .map(|b| match b {
                    Value::Byte(b) => *b,
                    other => panic!("{other:?}"),
                })
                .collect(),
            other => panic!("{other:?}"),
        }
    }

    type Outcome = (std::result::Result<Vec<String>, String>, Option<usize>);

    #[test]
    fn file_chooser_portal() {
        let _l = crate::test_support::test_lock();
        let Some(bus) = Bus::start() else { return };

        // A FileChooser portal: each request gets its handle back, then the
        // queued response a moment later.
        type Call = (String, Vec<Value>);
        let calls: Arc<Mutex<Vec<Call>>> = Arc::default();
        type Response = (u32, Vec<&'static str>);
        let responses: Arc<Mutex<VecDeque<Response>>> = Arc::default();
        let (c2, r2) = (calls.clone(), responses.clone());
        let portal = peer(&bus.address, PORTAL_DESTINATION, move |conn, msg| {
            if msg.is_method_call("org.freedesktop.DBus.Introspectable", "Introspect") {
                let mut reply = msg.new_method_return()?;
                reply.append_args(&[Arg::Str(
                    "<node><interface name=\"org.freedesktop.portal.FileChooser\"/></node>",
                )]);
                return Some(reply);
            }
            for method in ["OpenFile", "SaveFile"] {
                if msg.is_method_call(PORTAL_INTERFACE, method) {
                    let mut calls = c2.lock().unwrap();
                    calls.push((method.to_owned(), msg.args()));
                    let path = format!(
                        "/org/freedesktop/portal/desktop/request/1_1/t{}",
                        calls.len()
                    );
                    let (code, uris) = r2.lock().unwrap().pop_front().unwrap();
                    let (conn, p2) = (conn.clone(), path.clone());
                    std::thread::spawn(move || {
                        std::thread::sleep(std::time::Duration::from_millis(300));
                        let mut signal =
                            conn.new_signal(&p2, SIGNAL_INTERFACE, SIGNAL_NAME).unwrap();
                        let mut w = signal.writer();
                        w.append(&Arg::U32(code));
                        w.container(b'a', Some("{sv}"), |d| {
                            d.dict_entry("uris", "as", |v| v.append_str_array(&uris))
                        });
                        conn.send(&signal);
                    });
                    let mut reply = msg.new_method_return()?;
                    reply.append_args(&[Arg::ObjectPath(&path)]);
                    return Some(reply);
                }
            }
            None
        });
        test_bus::use_as_session(&bus);
        let ctx = dbus::context().unwrap();
        assert!(portal_detect_on(&ctx.session_conn));

        let show = |dialog_type, options: FileDialogOptions| -> Outcome {
            let result = Arc::new(Mutex::new(None));
            let r2 = result.clone();
            portal_show_file_dialog(
                dialog_type,
                Box::new(move |files, filter| {
                    *r2.lock().unwrap() = Some((files.map_err(|e| e.to_string()), filter));
                }),
                options,
            );
            assert!(test_bus::wait_for(dbus::pump_events, || result
                .lock()
                .unwrap()
                .is_some()));
            let outcome = result.lock().unwrap().take().unwrap();
            outcome
        };

        // Open files.
        responses
            .lock()
            .unwrap()
            .push_back((0, vec!["file:///tmp/a%20b", "file:///c"]));
        let result = show(
            FileDialogType::OpenFile,
            FileDialogOptions {
                filters: vec![
                    DialogFileFilter::new("Images", "png;jpg"),
                    DialogFileFilter::new("All", "*"),
                ],
                location: Some("/tmp".into()),
                many: true,
                accept: Some("Go".into()),
                ..FileDialogOptions::default()
            },
        );
        assert_eq!(result, (Ok(vec!["/tmp/a b".into(), "/c".into()]), None));
        {
            let calls = calls.lock().unwrap();
            let (method, args) = &calls[0];
            assert_eq!(method, "OpenFile");
            assert_eq!(args[0], Value::Str(String::new()));
            assert_eq!(args[1], Value::Str("Open File".into()));
            let Value::Array(dict) = &args[2] else {
                panic!("{args:?}");
            };
            assert_eq!(
                dict_get(dict, "handle_token").and_then(Value::as_str),
                Some("1")
            );
            assert_eq!(dict_get(dict, "modal"), Some(&Value::Bool(false)));
            assert_eq!(dict_get(dict, "multiple"), Some(&Value::Bool(true)));
            assert_eq!(dict_get(dict, "directory"), None);
            assert_eq!(bytes_of(dict_get(dict, "current_folder")), b"/tmp\0");
            assert_eq!(
                dict_get(dict, "accept_label").and_then(Value::as_str),
                Some("Go")
            );
            let filter = |name: &str, globs: &[&str]| {
                Value::Struct(vec![
                    Value::Str(name.into()),
                    Value::Array(
                        globs
                            .iter()
                            .map(|g| Value::Struct(vec![Value::U32(0), Value::Str((*g).into())]))
                            .collect(),
                    ),
                ])
            };
            assert_eq!(
                dict_get(dict, "filters"),
                Some(&Value::Array(vec![
                    filter("Images", &["*.[pP][nN][gG]", "*.[jJ][pP][gG]"]),
                    filter("All", &["*"]),
                ]))
            );
        }

        // A canceled folder dialog.
        responses.lock().unwrap().push_back((1, vec![]));
        let result = show(FileDialogType::OpenFolder, FileDialogOptions::default());
        assert_eq!(result, (Ok(vec![]), None));
        {
            let calls = calls.lock().unwrap();
            let (method, args) = &calls[1];
            assert_eq!(method, "OpenFile");
            assert_eq!(args[1], Value::Str("Open Folder".into()));
            let Value::Array(dict) = &args[2] else {
                panic!("{args:?}");
            };
            assert_eq!(dict_get(dict, "directory"), Some(&Value::Bool(true)));
            assert_eq!(dict_get(dict, "multiple"), None);
        }

        // Saving a new file, then an existing one; an error response.
        let dir = crate::test_support::TempDir::new("portal");
        let existing = dir.path("old.txt");
        std::fs::write(&existing, b"x").unwrap();
        responses
            .lock()
            .unwrap()
            .push_back((0, vec!["file:///x/new.txt"]));
        responses.lock().unwrap().push_back((0, vec!["ftp://nope"]));
        responses.lock().unwrap().push_back((3, vec![]));
        let result = show(
            FileDialogType::SaveFile,
            FileDialogOptions {
                location: Some(dir.path("new.txt")),
                title: Some("Keep".into()),
                ..FileDialogOptions::default()
            },
        );
        assert_eq!(result, (Ok(vec!["/x/new.txt".into()]), None));
        let result = show(
            FileDialogType::SaveFile,
            FileDialogOptions {
                location: Some(existing.clone()),
                ..FileDialogOptions::default()
            },
        );
        assert_eq!(
            result,
            (
                Err("Portal dialogs: Unsupported protocol: ftp://nope".into()),
                None
            )
        );
        let result = show(FileDialogType::SaveFile, FileDialogOptions::default());
        assert!(result.0.is_err());
        {
            let calls = calls.lock().unwrap();
            let (method, args) = &calls[2];
            assert_eq!(method, "SaveFile");
            assert_eq!(args[1], Value::Str("Keep".into()));
            let Value::Array(dict) = &args[2] else {
                panic!("{args:?}");
            };
            assert_eq!(
                bytes_of(dict_get(dict, "current_folder")),
                format!("{}\0", dir.0).as_bytes()
            );
            assert_eq!(
                dict_get(dict, "current_name").and_then(Value::as_str),
                Some("new.txt")
            );
            let Value::Array(dict) = &calls[3].1[2] else {
                panic!("{calls:?}");
            };
            assert_eq!(
                bytes_of(dict_get(dict, "current_file")),
                format!("{existing}\0").as_bytes()
            );
            assert_eq!(
                dict_get(dict, "current_name").and_then(Value::as_str),
                Some("old.txt")
            );
            assert_eq!(calls[4].1[1], Value::Str("Save File".into()));
        }

        drop(ctx);
        test_bus::release_session();
        portal.stop();
    }
}
