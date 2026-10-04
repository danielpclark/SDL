// Rust translation of src/core/linux/SDL_fcitx.c and SDL_fcitx.h from
// Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Fcitx 5 input method through its D-Bus portal on the session bus.

use std::sync::{Mutex, MutexGuard, Weak};

use super::dbus::{self, Arg, Context, HandlerResult, Message, Value};
use crate::events::keyboard::{self, Keymod};
use crate::events::WindowID;
use crate::hints;
use crate::video::rect::Rect;

const FCITX_DBUS_SERVICE: &str = "org.freedesktop.portal.Fcitx";

const FCITX_IM_DBUS_PATH: &str = "/org/freedesktop/portal/inputmethod";

const FCITX_IM_DBUS_INTERFACE: &str = "org.fcitx.Fcitx.InputMethod1";
const FCITX_IC_DBUS_INTERFACE: &str = "org.fcitx.Fcitx.InputContext1";

#[allow(dead_code)] // (unused upstream too)
const DBUS_TIMEOUT: i32 = 500;

/// Translation of `FcitxClient`.
struct FcitxClient {
    dbus: Option<Weak<Context>>,

    ic_path: Option<String>,

    #[allow(dead_code)] // (as upstream: unused)
    id: i32,

    cursor_rect: Rect,

    /// The `SDL_HINT_IME_IMPLEMENTED_UI` callbacks added.
    capabilities_watches: Vec<hints::Callback>,
}

static FCITX_CLIENT: Mutex<FcitxClient> = Mutex::new(FcitxClient {
    dbus: None,
    ic_path: None,
    id: 0,
    cursor_rect: Rect {
        x: 0,
        y: 0,
        w: 0,
        h: 0,
    },
    capabilities_watches: Vec::new(),
});

fn client() -> MutexGuard<'static, FcitxClient> {
    FCITX_CLIENT.lock().unwrap_or_else(|e| e.into_inner())
}

fn ic_path() -> Option<String> {
    client().ic_path.clone()
}

/// Translation of `GetAppName()`.
// !!! FIXME: should this just be dumped for src/core/unix's SDL_GetAppID()?
fn get_app_name() -> String {
    crate::filesystem::get_exe_name().unwrap_or_else(|_| "SDL_App".to_owned())
}

/// The preedit text and its highlighted range (in characters; -1 when
/// unknown). Translation of `Fcitx_GetPreeditString()`.
fn fcitx_get_preedit_string(args: &[Value]) -> (Option<String>, i32, i32) {
    let mut text = None;
    let mut p_start_pos: i32 = -1;
    let mut p_end_pos: i32 = -1;

    // Message type is a(si)i, we only need string part
    if let Some(Value::Array(array)) = args.first() {
        let mut pos: i32 = 0;
        let mut joined = String::new();
        // First pass: calculate string length
        for item in array {
            let Value::Struct(sub) = item else {
                break;
            };
            let subtext = match sub.first() {
                Some(Value::Str(s)) if !s.is_empty() => Some(s.as_str()),
                _ => None,
            };
            if let Some(Value::I32(ty)) = sub.get(1) {
                if p_end_pos == -1 {
                    // Type is a bit field defined as follows:
                    // bit 3: Underline, bit 4: HighLight, bit 5: DontCommit,
                    // bit 6: Bold,      bit 7: Strike,    bit 8: Italic
                    // We only consider highlight
                    if ty & (1 << 4) != 0 {
                        if p_start_pos == -1 {
                            p_start_pos = pos;
                        }
                    } else if p_start_pos != -1 && p_end_pos == -1 {
                        p_end_pos = pos;
                    }
                }
            }
            if let Some(subtext) = subtext {
                pos += crate::stdlib::string::utf8strlen(subtext) as i32;
                // Second pass: join all the sub string
                joined.push_str(subtext);
            }
        }
        if p_start_pos != -1 && p_end_pos == -1 {
            p_end_pos = pos;
        }
        if !joined.is_empty() {
            text = Some(joined);
        }
    }

    (text, p_start_pos, p_end_pos)
}

/// Translation of `Fcitx_GetPreeditCursorByte()`.
fn fcitx_get_preedit_cursor_byte(args: &[Value]) -> i32 {
    match args.get(1) {
        Some(Value::I32(byte)) => *byte,
        _ => -1,
    }
}

/// Translation of `DBus_MessageFilter()`.
fn dbus_message_filter(msg: &Message) -> HandlerResult {
    if msg.is_signal(FCITX_IC_DBUS_INTERFACE, "CommitString") {
        // (upstream reads the text without checking its type)
        if let Some(Value::Str(text)) = msg.args().first() {
            keyboard::send_keyboard_text(text);
        }

        return HandlerResult::Handled;
    }

    if msg.is_signal(FCITX_IC_DBUS_INTERFACE, "UpdateFormattedPreedit") {
        let args = msg.args();
        let (text, mut start_pos, end_pos) = fcitx_get_preedit_string(&args);
        match text {
            Some(text) => {
                if start_pos == -1 {
                    let byte_pos = fcitx_get_preedit_cursor_byte(&args);
                    start_pos = if byte_pos >= 0 {
                        crate::stdlib::string::utf8strnlen(&text, byte_pos as usize) as i32
                    } else {
                        -1
                    };
                }
                keyboard::send_editing_text(
                    &text,
                    start_pos,
                    if end_pos >= 0 {
                        end_pos - start_pos
                    } else {
                        -1
                    },
                );
            }
            None => keyboard::send_editing_text("", 0, 0),
        }

        update_text_input_area(keyboard::keyboard_focus());
        return HandlerResult::Handled;
    }

    HandlerResult::NotYetHandled
}

/// Translation of `FcitxClientICCallMethod()`.
fn fcitx_client_ic_call_method(method: &str) {
    let Some(ic_path) = ic_path() else {
        return;
    };
    dbus::call_void_method(
        FCITX_DBUS_SERVICE,
        &ic_path,
        FCITX_IC_DBUS_INTERFACE,
        method,
        &[],
    );
}

/// Translation of `Fcitx_SetCapabilities()`.
fn fcitx_set_capabilities(hint: Option<&str>) {
    let mut caps: u64 = 0;
    let Some(ic_path) = ic_path() else {
        return;
    };

    if hint.is_some_and(|h| h.contains("composition")) {
        caps |= 1 << 1; // Preedit Flag
        caps |= 1 << 4; // Formatted Preedit Flag
    }
    if hint.is_some_and(|h| h.contains("candidates")) {
        // FIXME, turn off native candidate rendering
    }

    dbus::call_void_method(
        FCITX_DBUS_SERVICE,
        &ic_path,
        FCITX_IC_DBUS_INTERFACE,
        "SetCapability",
        &[Arg::U64(caps)],
    );
}

/// Translation of `FcitxCreateInputContext()`.
fn fcitx_create_input_context(ctx: &Context, appname: &str) -> Option<String> {
    let program = "program";
    let conn = &ctx.session_conn;
    let mut msg = conn.new_method_call(
        FCITX_DBUS_SERVICE,
        FCITX_IM_DBUS_PATH,
        FCITX_IM_DBUS_INTERFACE,
        "CreateInputContext",
    )?;
    msg.writer().container(b'a', Some("(ss)"), |array| {
        array.container(b'r', None, |sub| {
            sub.append(&Arg::Str(program)) && sub.append(&Arg::Str(appname))
        })
    });
    let reply = conn.send_with_reply_and_block(&msg, 300).ok()?;
    match reply.args().into_iter().next() {
        Some(Value::ObjectPath(path)) => Some(path),
        _ => None,
    }
}

/// Translation of `FcitxClientCreateIC()`.
fn fcitx_client_create_ic(ctx: Option<&std::sync::Arc<Context>>) -> bool {
    let appname = get_app_name();

    // SDL_DBus_CallMethod cannot handle a(ss) type, call dbus function directly
    let Some(ctx) = ctx else {
        return false;
    };
    let Some(ic_path) = fcitx_create_input_context(ctx, &appname) else {
        return false;
    };

    client().ic_path = Some(ic_path);

    let conn = &ctx.session_conn;
    let _ = conn.add_match("type='signal', interface='org.fcitx.Fcitx.InputContext1'");
    // FIXME (upstream): the filter and the hint callback are never removed,
    // and each Init adds another.
    let _ = conn.add_owned_filter(dbus_message_filter);
    conn.flush();

    // (it runs at once, without our lock)
    if let Ok(watch) = hints::watch(hints::IME_IMPLEMENTED_UI, |change| {
        fcitx_set_capabilities(change.new_value)
    }) {
        client().capabilities_watches.push(watch);
    }
    true
}

/// Translation of `Fcitx_ModState()`.
fn fcitx_mod_state() -> u32 {
    let mut fcitx_mods = 0;
    let sdl_mods = keyboard::mod_state();

    if sdl_mods.intersects(Keymod::SHIFT) {
        fcitx_mods |= 1 << 0;
    }
    if sdl_mods.intersects(Keymod::CAPS) {
        fcitx_mods |= 1 << 1;
    }
    if sdl_mods.intersects(Keymod::CTRL) {
        fcitx_mods |= 1 << 2;
    }
    if sdl_mods.intersects(Keymod::ALT) {
        fcitx_mods |= 1 << 3;
    }
    if sdl_mods.intersects(Keymod::NUM) {
        fcitx_mods |= 1 << 4;
    }
    if sdl_mods.intersects(Keymod::MODE) {
        fcitx_mods |= 1 << 7;
    }
    if sdl_mods.intersects(Keymod::LGUI) {
        fcitx_mods |= 1 << 6;
    }
    if sdl_mods.intersects(Keymod::RGUI) {
        fcitx_mods |= 1 << 28;
    }

    fcitx_mods
}

/// Create the Fcitx input context; false without Fcitx. Translation of
/// `SDL_Fcitx_Init()`.
pub(crate) fn init() -> bool {
    let ctx = dbus::context();
    {
        let mut c = client();
        c.dbus = ctx.as_ref().map(std::sync::Arc::downgrade);

        c.cursor_rect = Rect {
            x: -1,
            y: -1,
            w: 0,
            h: 0,
        };
    }

    fcitx_client_create_ic(ctx.as_ref())
}

/// Translation of `SDL_Fcitx_Quit()`.
pub(crate) fn quit() {
    fcitx_client_ic_call_method("DestroyIC");
    client().ic_path = None;
}

/// Translation of `SDL_Fcitx_SetFocus()`.
pub(crate) fn set_focus(focused: bool) {
    if focused {
        fcitx_client_ic_call_method("FocusIn");
    } else {
        fcitx_client_ic_call_method("FocusOut");
    }
}

/// Translation of `SDL_Fcitx_Reset()`.
pub(crate) fn reset() {
    fcitx_client_ic_call_method("Reset");
}

/// Translation of `SDL_Fcitx_ProcessKeyEvent()`.
pub(crate) fn process_key_event(keysym: u32, keycode: u32, down: bool) -> bool {
    let mod_state = fcitx_mod_state();
    let is_release = !down;
    let event_time: u32 = 0;

    let Some(ic_path) = ic_path() else {
        return false;
    };

    let reply = dbus::call_method(
        FCITX_DBUS_SERVICE,
        &ic_path,
        FCITX_IC_DBUS_INTERFACE,
        "ProcessKeyEvent",
        &[
            Arg::U32(keysym),
            Arg::U32(keycode),
            Arg::U32(mod_state),
            Arg::Bool(is_release),
            Arg::U32(event_time),
        ],
    );
    if let Some(reply) = reply {
        if let [Value::Bool(true), ..] = reply.args().as_slice() {
            update_text_input_area(keyboard::keyboard_focus());
            return true;
        }
    }

    false
}

/// Translation of `SDL_Fcitx_UpdateTextInputArea()`.
pub(crate) fn update_text_input_area(window: Option<WindowID>) {
    let Some(window) = window else {
        return;
    };
    let Some((new_cursor, (mut x, mut y))) = super::ime::window_input_cursor(window) else {
        return;
    };

    let (cursor, ic_path) = {
        let mut c = client();
        c.cursor_rect = new_cursor;
        let cursor = &mut c.cursor_rect;

        if cursor.x == -1 && cursor.y == -1 && cursor.w == 0 && cursor.h == 0 {
            // move to bottom left
            let (_w, h) = crate::video::Window::from_id(window)
                .and_then(|w| w.size())
                .unwrap_or((0, 0));
            cursor.x = 0;
            cursor.y = h;
        }
        (c.cursor_rect, c.ic_path.clone())
    };

    x += cursor.x;
    y += cursor.y;

    // FIXME (upstream): the call is made even without an input context,
    // with a NULL path; it is skipped here.
    let Some(ic_path) = ic_path else {
        return;
    };
    dbus::call_void_method(
        FCITX_DBUS_SERVICE,
        &ic_path,
        FCITX_IC_DBUS_INTERFACE,
        "SetCursorRect",
        &[
            Arg::I32(x),
            Arg::I32(y),
            Arg::I32(cursor.w),
            Arg::I32(cursor.h),
        ],
    );
}

/// Translation of `SDL_Fcitx_PumpEvents()`.
pub(crate) fn pump_events() {
    let ctx = client().dbus.as_ref().and_then(Weak::upgrade);
    let Some(ctx) = ctx else {
        return;
    };
    let conn = &ctx.session_conn;

    conn.read_write(0);

    while conn.dispatch() {
        // Do nothing, actual work happens in DBus_MessageFilter
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::linux::dbus::test_bus::{self, peer, Bus};
    use crate::core::linux::dbus::{Connection, Writer};
    use crate::events::{Event, EventType};
    use std::sync::Arc;

    fn spans(w: &mut Writer<'_>, spans: &[(&str, i32)]) -> bool {
        w.container(b'a', Some("(si)"), |a| {
            spans.iter().all(|(text, ty)| {
                a.container(b'r', None, |st| {
                    st.append(&Arg::Str(text)) && st.append(&Arg::I32(*ty))
                })
            })
        })
    }

    #[test]
    fn preedit_strings() {
        let s = |v: &[(&str, i32)]| {
            Value::Array(
                v.iter()
                    .map(|(t, ty)| Value::Struct(vec![Value::Str((*t).into()), Value::I32(*ty)]))
                    .collect(),
            )
        };
        assert_eq!(
            fcitx_get_preedit_string(&[s(&[("ab", 0), ("cd", 16), ("é", 0)])]),
            (Some("abcdé".into()), 2, 4)
        );
        assert_eq!(
            fcitx_get_preedit_string(&[s(&[("ab", 16), ("cd", 16)])]),
            (Some("abcd".into()), 0, 4)
        );
        assert_eq!(fcitx_get_preedit_string(&[s(&[("", 16)])]), (None, 0, 0));
        assert_eq!(fcitx_get_preedit_string(&[]), (None, -1, -1));
        assert_eq!(fcitx_get_preedit_cursor_byte(&[s(&[]), Value::I32(3)]), 3);
        assert_eq!(fcitx_get_preedit_cursor_byte(&[s(&[])]), -1);
    }

    #[test]
    fn fcitx_portal() {
        crate::events::window::tests::with_video(&[1], |video| {
            let Some(bus) = Bus::start() else { return };
            const IC_PATH: &str = "/org/freedesktop/portal/inputcontext/1";
            let calls = Arc::new(Mutex::new(Vec::new()));
            let c2 = calls.clone();
            let portal = peer(&bus.address, FCITX_DBUS_SERVICE, move |_, msg| {
                if msg.is_method_call(FCITX_IM_DBUS_INTERFACE, "CreateInputContext") {
                    c2.lock().unwrap().push(("CreateInputContext", msg.args()));
                    let mut reply = msg.new_method_return()?;
                    reply.append_args(&[Arg::ObjectPath(IC_PATH)]);
                    return Some(reply);
                }
                for method in [
                    "SetCapability",
                    "FocusIn",
                    "FocusOut",
                    "Reset",
                    "ProcessKeyEvent",
                    "SetCursorRect",
                    "DestroyIC",
                ] {
                    if msg.is_method_call(FCITX_IC_DBUS_INTERFACE, method) {
                        c2.lock().unwrap().push((method, msg.args()));
                        let mut reply = msg.new_method_return()?;
                        if method == "ProcessKeyEvent" {
                            reply.append_args(&[Arg::Bool(true)]);
                        }
                        return Some(reply);
                    }
                }
                None
            });
            test_bus::use_as_session(&bus);
            keyboard::init_keyboard().unwrap();
            video
                .windows
                .lock()
                .unwrap()
                .get_mut(&1)
                .unwrap()
                .text_input_active = true;
            keyboard::set_keyboard_focus(Some(1)).unwrap();
            crate::events::flush_events(EventType::FIRST, EventType::LAST);

            assert!(init());
            assert_eq!(ic_path().as_deref(), Some(IC_PATH));
            set_focus(true);
            assert!(process_key_event(0x61, 38, true));
            reset();
            assert!(test_bus::wait_for(
                || (),
                || calls.lock().unwrap().len() == 5
            ));
            {
                let calls = calls.lock().unwrap();
                let names: Vec<&str> = calls.iter().map(|c| c.0).collect();
                assert_eq!(
                    names,
                    [
                        "CreateInputContext",
                        "SetCapability",
                        "FocusIn",
                        "ProcessKeyEvent",
                        "Reset"
                    ]
                );
                assert_eq!(
                    calls[0].1,
                    [Value::Array(vec![Value::Struct(vec![
                        Value::Str("program".into()),
                        Value::Str(get_app_name()),
                    ])])]
                );
                assert_eq!(calls[1].1, [Value::U64(0)]);
                assert_eq!(
                    calls[3].1,
                    [
                        Value::U32(0x61),
                        Value::U32(38),
                        Value::U32(0),
                        Value::Bool(false),
                        Value::U32(0)
                    ]
                );
            }

            // Signals on the input context.
            let emitter = Connection::open_address(&bus.address).unwrap();
            let signal = |name: &str, fill: &dyn Fn(&mut Writer<'_>)| {
                let mut s = emitter
                    .new_signal(IC_PATH, FCITX_IC_DBUS_INTERFACE, name)
                    .unwrap();
                fill(&mut s.writer());
                assert!(emitter.send(&s));
            };
            signal("CommitString", &|w| {
                w.append(&Arg::Str("hi"));
            });
            signal("UpdateFormattedPreedit", &|w| {
                spans(w, &[("ab", 0), ("cd", 16), ("e", 0)]);
                w.append(&Arg::I32(1));
            });
            signal("UpdateFormattedPreedit", &|w| {
                spans(w, &[("xyz", 0)]);
                w.append(&Arg::I32(2));
            });
            signal("UpdateFormattedPreedit", &|w| {
                spans(w, &[]);
                w.append(&Arg::I32(0));
            });
            let mut events = Vec::new();
            assert!(test_bus::wait_for(pump_events, || {
                events.extend(
                    crate::events::get_events(EventType::TEXT_EDITING, EventType::TEXT_INPUT, 16)
                        .unwrap(),
                );
                events.len() >= 4
            }));
            let summary: Vec<(String, i32, i32)> = events
                .iter()
                .map(|e| match e {
                    Event::TextInput(t) => (t.text.clone(), -2, -2),
                    Event::TextEditing(t) => (t.text.clone(), t.start, t.length),
                    other => panic!("{other:?}"),
                })
                .collect();
            assert_eq!(
                summary,
                [
                    ("hi".to_owned(), -2, -2),
                    ("abcde".to_owned(), 2, 2),
                    ("xyz".to_owned(), 2, -1),
                    (String::new(), 0, 0),
                ]
            );

            quit();
            assert_eq!(ic_path(), None);
            assert!(test_bus::wait_for(
                || (),
                || calls
                    .lock()
                    .unwrap()
                    .last()
                    .is_some_and(|c| c.0 == "DestroyIC")
            ));
            test_bus::release_session();
            portal.stop();
        });
    }
}
