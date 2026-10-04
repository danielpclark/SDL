// Rust translation of src/core/linux/SDL_system_theme.c from Simple
// DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The system theme from the settings portal's `color-scheme` (followed
//! through its SettingChanged signal), else Ubuntu Touch's theme file. The
//! X11 and Wayland video drivers call [`init`] and [`get`].

use std::io::BufRead;
use std::sync::Mutex;

use super::dbus::{self, Arg, HandlerResult, Message, Value};
use crate::video::SystemTheme;

const PORTAL_DESTINATION: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const PORTAL_INTERFACE: &str = "org.freedesktop.portal.Settings";
const PORTAL_METHOD: &str = "Read";

const SIGNAL_INTERFACE: &str = "org.freedesktop.portal.Settings";
const SIGNAL_NAMESPACE: &str = "org.freedesktop.appearance";
const SIGNAL_NAME: &str = "SettingChanged";
const SIGNAL_KEY: &str = "color-scheme";

/// Translation of `system_theme_data` (its D-Bus context is
/// [`dbus::context`]).
static SYSTEM_THEME_DATA: Mutex<SystemTheme> = Mutex::new(SystemTheme::Unknown);

fn set_theme(theme: SystemTheme) {
    *SYSTEM_THEME_DATA.lock().unwrap_or_else(|e| e.into_inner()) = theme;
}

fn theme() -> SystemTheme {
    *SYSTEM_THEME_DATA.lock().unwrap_or_else(|e| e.into_inner())
}

/// The theme in a `color-scheme` variant; `None` if it isn't a UINT32
/// variant, the theme unchanged for unknown values. Translation of
/// `DBus_ExtractThemeVariant()`.
fn dbus_extract_theme_variant(value: &Value, theme: SystemTheme) -> Option<SystemTheme> {
    let Value::Variant(inner) = value else {
        return None;
    };
    let Value::U32(color_scheme) = **inner else {
        return None;
    };
    Some(match color_scheme {
        0 => SystemTheme::Unknown,
        1 => SystemTheme::Dark,
        2 => SystemTheme::Light,
        _ => theme,
    })
}

/// Translation of `DBus_MessageFilter()`.
fn dbus_message_filter(msg: &Message) -> HandlerResult {
    if msg.is_signal(SIGNAL_INTERFACE, SIGNAL_NAME) {
        let args = msg.args();
        // Check if the parameters are what we expect
        let (Some(Value::Str(namespace)), Some(Value::Str(key)), Some(value)) =
            (args.first(), args.get(1), args.get(2))
        else {
            return HandlerResult::NotYetHandled;
        };
        if namespace != SIGNAL_NAMESPACE || key != SIGNAL_KEY {
            return HandlerResult::NotYetHandled;
        }

        let Some(new_theme) = dbus_extract_theme_variant(value, theme()) else {
            return HandlerResult::NotYetHandled;
        };
        set_theme(new_theme);

        crate::video::core::set_system_theme(new_theme);
        return HandlerResult::Handled;
    }
    HandlerResult::NotYetHandled
}

/// Read the theme from the settings portal and follow its changes; false
/// without D-Bus. Translation of `SDL_SystemTheme_Init()`.
pub(crate) fn init() -> bool {
    set_theme(SystemTheme::Unknown);
    let Some(ctx) = dbus::context() else {
        return false;
    };
    let conn = &ctx.session_conn;

    if let Some(mut msg) = conn.new_method_call(
        PORTAL_DESTINATION,
        PORTAL_PATH,
        PORTAL_INTERFACE,
        PORTAL_METHOD,
    ) {
        if msg.append_args(&[Arg::Str(SIGNAL_NAMESPACE), Arg::Str(SIGNAL_KEY)]) {
            if let Ok(reply) = conn.send_with_reply_and_block(&msg, 300) {
                // The response has signature <<u>>
                if let Some(Value::Variant(variant_outer)) = reply.args().first() {
                    if let Some(t) = dbus_extract_theme_variant(variant_outer, theme()) {
                        set_theme(t);
                    }
                }
            }
        }
    }

    let _ = conn.add_match(&format!(
        "type='signal', interface='{SIGNAL_INTERFACE}',member='{SIGNAL_NAME}', arg0='{SIGNAL_NAMESPACE}',arg1='{SIGNAL_KEY}'"
    ));
    // (as upstream, each call adds another filter, and none is removed)
    let _ = conn.add_owned_filter(dbus_message_filter);
    conn.flush();
    true
}

/// The theme set in an Ubuntu Touch `theme.ini`. Part of
/// `UbuntuTouch_GetSystemTheme()`.
fn ubuntu_touch_parse_theme(config_file: impl BufRead) -> SystemTheme {
    let mut theme = SystemTheme::Unknown;
    let mut is_in_general_category = false;

    // (lines keep their "\n", as getline() leaves it)
    let mut lines = config_file;
    let mut line = String::new();
    loop {
        line.clear();
        match lines.read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        if line.starts_with('[') {
            is_in_general_category = line == "[General]\n";
        } else if is_in_general_category && line.starts_with("theme=") {
            theme = match line.as_str() {
                "theme=Lomiri.Components.Themes.SuruDark\n"
                | "theme=Ubuntu.Components.Themes.SuruDark\n" => SystemTheme::Dark,
                "theme=Lomiri.Components.Themes.Ambiance\n"
                | "theme=Ubuntu.Components.Themes.Ambiance\n" => SystemTheme::Light,
                _ => SystemTheme::Unknown,
            };
        }
    }

    theme
}

/// Translation of `UbuntuTouch_GetSystemTheme()`.
fn ubuntu_touch_get_system_theme() -> SystemTheme {
    // "Lomiri": Ubuntu Touch 20.04+
    // "Ubuntu": Ubuntu Touch 16.04
    let config_file = std::fs::File::open("/home/phablet/.config/lomiri-ui-toolkit/theme.ini")
        .or_else(|_| std::fs::File::open("/home/phablet/.config/ubuntu-ui-toolkit/theme.ini"));
    match config_file {
        Ok(f) => ubuntu_touch_parse_theme(std::io::BufReader::new(f)),
        Err(_) => SystemTheme::Unknown,
    }
}

/// The system theme. Translation of `SDL_SystemTheme_Get()`.
pub(crate) fn get() -> SystemTheme {
    let theme = theme();
    if theme == SystemTheme::Unknown {
        // TODO: Use inotify to watch for changes, so that the config file
        // doesn't need to be checked each time.
        return ubuntu_touch_get_system_theme();
    }

    theme
}

/// A settings portal on `bus` that answers the color-scheme read with
/// `scheme` (0 unknown, 1 dark, 2 light), for tests of the theme's users.
#[cfg(test)]
pub(crate) fn serve_test_portal(bus: &dbus::test_bus::Bus, scheme: u32) -> dbus::test_bus::Peer {
    dbus::test_bus::peer(&bus.address, PORTAL_DESTINATION, move |_, msg| {
        if !msg.is_method_call(PORTAL_INTERFACE, PORTAL_METHOD) {
            return None;
        }
        let mut reply = msg.new_method_return()?;
        reply
            .writer()
            .variant("v", |v| v.variant("u", |u| u.append(&Arg::U32(scheme))));
        Some(reply)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::linux::dbus::test_bus::{self, peer, Bus};
    use crate::core::linux::dbus::Connection;

    #[test]
    fn ubuntu_touch_theme_file() {
        let parse = |s: &str| ubuntu_touch_parse_theme(s.as_bytes());
        assert_eq!(
            parse("[General]\ntheme=Lomiri.Components.Themes.SuruDark\n"),
            SystemTheme::Dark
        );
        assert_eq!(
            parse("[General]\ntheme=Ubuntu.Components.Themes.Ambiance\n"),
            SystemTheme::Light
        );
        assert_eq!(
            parse("[Other]\ntheme=Lomiri.Components.Themes.SuruDark\n"),
            SystemTheme::Unknown
        );
        // (the last line needs its newline, as upstream)
        assert_eq!(
            parse("[General]\ntheme=Lomiri.Components.Themes.SuruDark"),
            SystemTheme::Unknown
        );
        assert_eq!(parse("[General]\ntheme=Mine\n"), SystemTheme::Unknown);
    }

    #[test]
    fn settings_portal() {
        let _l = crate::test_support::test_lock();
        let Some(bus) = Bus::start() else { return };
        let portal = peer(&bus.address, PORTAL_DESTINATION, |_, msg| {
            if !msg.is_method_call(PORTAL_INTERFACE, PORTAL_METHOD) {
                return None;
            }
            assert_eq!(
                msg.args(),
                [
                    Value::Str(SIGNAL_NAMESPACE.into()),
                    Value::Str(SIGNAL_KEY.into())
                ]
            );
            let mut reply = msg.new_method_return()?;
            reply
                .writer()
                .variant("v", |v| v.variant("u", |u| u.append(&Arg::U32(1))));
            Some(reply)
        });
        test_bus::use_as_session(&bus);

        assert!(init());
        assert_eq!(get(), SystemTheme::Dark);

        let emitter = Connection::open_address(&bus.address).unwrap();
        let changed = |namespace: &str, scheme: u32| {
            let mut signal = emitter
                .new_signal(PORTAL_PATH, SIGNAL_INTERFACE, SIGNAL_NAME)
                .unwrap();
            let mut w = signal.writer();
            w.append(&Arg::Str(namespace));
            w.append(&Arg::Str(SIGNAL_KEY));
            w.variant("u", |v| v.append(&Arg::U32(scheme)));
            assert!(emitter.send(&signal));
        };
        changed("org.example.other", 2);
        changed(SIGNAL_NAMESPACE, 2);
        assert!(test_bus::wait_for(dbus::pump_events, || theme() == SystemTheme::Light));
        changed(SIGNAL_NAMESPACE, 7); // (unknown: unchanged)
        changed(SIGNAL_NAMESPACE, 0);
        assert!(test_bus::wait_for(dbus::pump_events, || theme() == SystemTheme::Unknown));

        test_bus::release_session();
        portal.stop();
    }
}
