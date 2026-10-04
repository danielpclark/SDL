// Rust translation of src/core/linux/SDL_progressbar.c from Simple
// DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Window progress in the taskbar through the Unity LauncherAPI
//! (`com.canonical.Unity.LauncherEntry.Update`), for the X11 and Wayland
//! video drivers' `ApplyWindowProgress`.

use super::dbus::{self, Arg};
use crate::events::WindowID;
use crate::video::ProgressState;

const UNITY_LAUNCHER_API_DBUS_INTERFACE: &str = "com.canonical.Unity.LauncherEntry";
const UNITY_LAUNCHER_API_DBUS_SIGNAL: &str = "Update";

/// Translation of `GetDBUSObjectPath()`.
fn get_dbus_object_path() -> String {
    let app_id = crate::core::unix::app_id();

    // Sanitize exe_name to make it a legal D-Bus path element
    let mut sanitized: String = app_id
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() {
                b as char
            } else {
                '_'
            }
        })
        .collect();

    // Ensure it starts with a letter or underscore
    if !sanitized.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_') {
        sanitized.insert(0, '_');
    }

    // Create full path
    format!("/org/libsdl/{}_{}", sanitized, std::process::id())
}

/// Translation of `GetAppDesktopPath()`.
fn get_app_desktop_path() -> String {
    format!("{}.desktop", crate::core::unix::app_id())
}

/// Translation of `ShouldShowProgress()` (a `ProgressState` here is never
/// `SDL_PROGRESS_STATE_INVALID`).
fn should_show_progress(progress_state: ProgressState) -> bool {
    if progress_state == ProgressState::None {
        return false;
    }

    // Unity LauncherAPI only supports "normal" display of progress
    true
}

/// Show a window's progress state and value on its taskbar entry; false
/// without D-Bus. Translation of `DBUS_ApplyWindowProgress()`.
pub(crate) fn apply_window_progress(window: WindowID) -> bool {
    let Ok(window) = crate::video::Window::from_id(window) else {
        return false;
    };
    let (Ok(state), Ok(value)) = (window.progress_state(), window.progress_value()) else {
        return false;
    };
    send_progress(state, value)
}

/// The body of `DBUS_ApplyWindowProgress()`.
fn send_progress(progress_state: ProgressState, progress_value: f32) -> bool {
    // Signal signature:
    // signal com.canonical.Unity.LauncherEntry.Update (in s app_uri, in a{sv} properties)

    let Some(ctx) = dbus::context() else {
        return false;
    };
    let conn = &ctx.session_conn;

    let object_path = get_dbus_object_path();

    let Some(mut msg) = conn.new_signal(
        &object_path,
        UNITY_LAUNCHER_API_DBUS_INTERFACE,
        UNITY_LAUNCHER_API_DBUS_SIGNAL,
    ) else {
        return false;
    };

    let desktop_path = get_app_desktop_path();

    let progress_visible = should_show_progress(progress_state);
    let progress = f64::from(progress_value);

    let mut args = msg.writer();
    args.append(&Arg::Str(&desktop_path)); // Setup app_uri parameter
    args.container(b'a', Some("{sv}"), |props| {
        // Setup properties parameter
        // Set progress visible property
        props.dict_entry("progress-visible", "b", |v| v.append(&Arg::Bool(progress_visible)))
            // Set progress value property
            && props.dict_entry("progress", "d", |v| v.append(&Arg::F64(progress)))
    });

    conn.send_no_flush(&msg);

    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::linux::dbus::test_bus::{self, Bus};
    use crate::core::linux::dbus::{Connection, HandlerResult, Value};
    use std::sync::{Arc, Mutex};

    #[test]
    fn launcher_entry_update() {
        let _l = crate::test_support::test_lock();
        let Some(bus) = Bus::start() else { return };
        test_bus::use_as_session(&bus);

        let listener = Connection::open_address(&bus.address).unwrap();
        listener
            .add_match(&format!(
                "type='signal',interface='{UNITY_LAUNCHER_API_DBUS_INTERFACE}'"
            ))
            .unwrap();
        let got = Arc::new(Mutex::new(Vec::new()));
        let g2 = got.clone();
        let _filter = listener.add_filter(move |msg| {
            if msg.is_signal(
                UNITY_LAUNCHER_API_DBUS_INTERFACE,
                UNITY_LAUNCHER_API_DBUS_SIGNAL,
            ) {
                g2.lock().unwrap().push(msg.args());
            }
            HandlerResult::NotYetHandled
        });

        assert!(send_progress(ProgressState::Normal, 0.5));
        assert!(send_progress(ProgressState::None, 0.0));
        // (flushed by the next pump)
        dbus::pump_events();
        assert!(test_bus::wait_for(
            || listener.pump(),
            || got.lock().unwrap().len() == 2
        ));
        let entry = |k: &str, v: Value| {
            Value::DictEntry(
                Box::new(Value::Str(k.into())),
                Box::new(Value::Variant(Box::new(v))),
            )
        };
        let desktop = Value::Str(get_app_desktop_path());
        let got = got.lock().unwrap();
        assert_eq!(got[0][0], desktop);
        assert_eq!(
            got[0][1],
            Value::Array(vec![
                entry("progress-visible", Value::Bool(true)),
                entry("progress", Value::F64(0.5)),
            ])
        );
        assert_eq!(
            got[1][1],
            Value::Array(vec![
                entry("progress-visible", Value::Bool(false)),
                entry("progress", Value::F64(0.0)),
            ])
        );

        let path = get_dbus_object_path();
        assert!(path.starts_with("/org/libsdl/"), "{path}");
        assert!(path.ends_with(&format!("_{}", std::process::id())));
        let element = &path["/org/libsdl/".len()..];
        assert!(element
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_'));
        assert!(!element.starts_with(|c: char| c.is_ascii_digit()));

        test_bus::release_session();
    }
}
