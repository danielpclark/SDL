// Rust translation of src/core/linux/SDL_ibus.c and SDL_ibus.h from Simple
// DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The IBus input method over D-Bus: through the IBus portal on the
//! session bus when there is one, else on IBus's own bus, whose address is
//! read from the file IBus writes (watched with inotify for restarts).
//! The `IBUS_*` constants of `<ibus.h>` are declared here.

use std::io::BufRead;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::sync::{Arc, Mutex, MutexGuard};

use super::dbus::{self, Arg, Connection, Context, HandlerResult, Message, Value};
use crate::events::keyboard::{self, Keymod};
use crate::events::WindowID;
use crate::hints;
use crate::video::rect::Rect;

const IBUS_PATH: &str = "/org/freedesktop/IBus";

const IBUS_SERVICE: &str = "org.freedesktop.IBus";
const IBUS_INTERFACE: &str = "org.freedesktop.IBus";
const IBUS_INPUT_INTERFACE: &str = "org.freedesktop.IBus.InputContext";

const IBUS_PORTAL_SERVICE: &str = "org.freedesktop.portal.IBus";
const IBUS_PORTAL_INTERFACE: &str = "org.freedesktop.IBus.Portal";
const IBUS_PORTAL_INPUT_INTERFACE: &str = "org.freedesktop.IBus.InputContext";

// IBusModifierType (ibustypes.h)
const IBUS_SHIFT_MASK: u32 = 1 << 0;
const IBUS_LOCK_MASK: u32 = 1 << 1;
const IBUS_CONTROL_MASK: u32 = 1 << 2;
const IBUS_MOD1_MASK: u32 = 1 << 3;
const IBUS_MOD2_MASK: u32 = 1 << 4;
const IBUS_MOD5_MASK: u32 = 1 << 7;
const IBUS_SUPER_MASK: u32 = 1 << 26;
const IBUS_META_MASK: u32 = 1 << 28;
const IBUS_RELEASE_MASK: u32 = 1 << 30;

// IBusCapabilite (ibustypes.h)
const IBUS_CAP_PREEDIT_TEXT: u32 = 1 << 0;
const IBUS_CAP_FOCUS: u32 = 1 << 3;

/// The connection IBus is reached through.
#[derive(Clone)]
enum IBusConn {
    /// The session bus, for the portal (upstream references it).
    Session(Arc<Context>),
    /// A private connection to IBus's own bus.
    Private(Arc<Connection>),
}

impl IBusConn {
    fn conn(&self) -> &Connection {
        match self {
            IBusConn::Session(ctx) => &ctx.session_conn,
            IBusConn::Private(conn) => conn,
        }
    }
}

/// The file's statics.
struct State {
    ibus_service: &'static str,
    ibus_interface: &'static str,
    ibus_input_interface: &'static str,
    input_ctx_path: Option<String>,
    ibus_cursor_rect: Rect,
    ibus_conn: Option<IBusConn>,
    ibus_is_portal_interface: bool,
    ibus_addr_file: Option<String>,
    inotify_fd: Option<OwnedFd>,
    inotify_wd: i32,
    /// The `SDL_HINT_IME_IMPLEMENTED_UI` callback.
    capabilities_watch: Option<hints::Callback>,
}

static STATE: Mutex<State> = Mutex::new(State {
    ibus_service: "",
    ibus_interface: "",
    ibus_input_interface: "",
    input_ctx_path: None,
    ibus_cursor_rect: Rect {
        x: 0,
        y: 0,
        w: 0,
        h: 0,
    },
    ibus_conn: None,
    ibus_is_portal_interface: false,
    ibus_addr_file: None,
    inotify_fd: None,
    inotify_wd: -1,
    capabilities_watch: None,
});

fn state() -> MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

/// What a call on the input context needs.
struct Target {
    conn: IBusConn,
    service: &'static str,
    path: Option<String>,
    input_interface: &'static str,
}

fn target() -> Option<Target> {
    let s = state();
    Some(Target {
        conn: s.ibus_conn.clone()?,
        service: s.ibus_service,
        path: s.input_ctx_path.clone(),
        input_interface: s.ibus_input_interface,
    })
}

impl Target {
    fn call_void(&self, method: &str, args: &[Arg<'_>]) {
        if let Some(path) = &self.path {
            dbus::call_void_method_on_connection(
                self.conn.conn(),
                self.service,
                path,
                self.input_interface,
                method,
                args,
            );
        }
    }
}

/// Translation of `IBus_ModState()`.
fn ibus_mod_state() -> u32 {
    let mut ibus_mods = 0;
    let sdl_mods = keyboard::mod_state();

    // Not sure about MOD3, MOD4 and HYPER mappings
    if sdl_mods.intersects(Keymod::LSHIFT) {
        ibus_mods |= IBUS_SHIFT_MASK;
    }
    if sdl_mods.intersects(Keymod::CAPS) {
        ibus_mods |= IBUS_LOCK_MASK;
    }
    if sdl_mods.intersects(Keymod::LCTRL) {
        ibus_mods |= IBUS_CONTROL_MASK;
    }
    if sdl_mods.intersects(Keymod::LALT) {
        ibus_mods |= IBUS_MOD1_MASK;
    }
    if sdl_mods.intersects(Keymod::NUM) {
        ibus_mods |= IBUS_MOD2_MASK;
    }
    if sdl_mods.intersects(Keymod::MODE) {
        ibus_mods |= IBUS_MOD5_MASK;
    }
    if sdl_mods.intersects(Keymod::LGUI) {
        ibus_mods |= IBUS_SUPER_MASK;
    }
    if sdl_mods.intersects(Keymod::RGUI) {
        ibus_mods |= IBUS_META_MASK;
    }

    ibus_mods
}

/// The fields of an `IBusText`-style structure in a variant. Translation of
/// `IBus_EnterVariant()`.
///
/// FIXME (upstream): the structure's name is compared with itself, so any
/// name is accepted. Kept.
fn ibus_enter_variant<'v>(value: Option<&'v Value>, _struct_id: &str) -> Option<&'v [Value]> {
    let Some(Value::Variant(sub)) = value else {
        return None;
    };
    let Value::Struct(inside) = &**sub else {
        return None;
    };
    match inside.first() {
        Some(Value::Str(_)) => Some(inside),
        _ => None,
    }
}

/// Translation of `IBus_GetDecorationPosition()`.
fn ibus_get_decoration_position(args: &[Value]) -> Option<(u32, u32)> {
    let sub1 = ibus_enter_variant(args.first(), "IBusText")?;

    let sub2 = ibus_enter_variant(sub1.get(3), "IBusAttrList")?;

    let Some(Value::Array(array)) = sub2.get(2) else {
        return None;
    };

    for element in array {
        if !matches!(element, Value::Variant(_)) {
            break;
        }
        if let Some(sub) = ibus_enter_variant(Some(element), "IBusAttribute") {
            // From here on, the structure looks like this:
            // Uint32 type: 1=underline, 2=foreground, 3=background
            // Uint32 value: for underline it's 0=NONE, 1=SINGLE, 2=DOUBLE,
            // 3=LOW,  4=ERROR
            // for foreground and background it's a color
            // Uint32 start_index: starting position for the style (utf8-char)
            // Uint32 end_index: end position for the style (utf8-char)

            // We only use the background type to determine the selection
            // (upstream reads the type without checking that it is one)
            if let Some(Value::U32(3)) = sub.get(2) {
                if let Some(Value::U32(start)) = sub.get(4) {
                    if let Some(Value::U32(end_pos)) = sub.get(5) {
                        return Some((*start, *end_pos));
                    }
                }
            }
        }
    }
    None
}

/// Translation of `IBus_GetVariantText()`.
fn ibus_get_variant_text(args: &[Value]) -> Option<&str> {
    // The text we need is nested weirdly, use dbus-monitor to see the structure better
    let sub = ibus_enter_variant(args.first(), "IBusText")?;
    match sub.get(2) {
        Some(Value::Str(text)) => Some(text),
        _ => None,
    }
}

/// Translation of `IBus_GetVariantCursorPos()`.
fn ibus_get_variant_cursor_pos(args: &[Value]) -> Option<u32> {
    match args.get(1) {
        Some(Value::U32(pos)) => Some(*pos),
        _ => None,
    }
}

/// Translation of `IBus_MessageHandler()`.
fn ibus_message_handler(msg: &Message) -> HandlerResult {
    let ibus_input_interface = state().ibus_input_interface;

    if msg.is_signal(ibus_input_interface, "CommitText") {
        let args = msg.args();
        if let Some(text) = ibus_get_variant_text(&args) {
            keyboard::send_keyboard_text(text);
        }

        return HandlerResult::Handled;
    }

    if msg.is_signal(ibus_input_interface, "UpdatePreeditText") {
        let args = msg.args();
        if let Some(text) = ibus_get_variant_text(&args) {
            let dec_pos = ibus_get_decoration_position(&args);
            let pos = if dec_pos.is_none() {
                ibus_get_variant_cursor_pos(&args)
            } else {
                None
            };

            if let Some((start_pos, end_pos)) = dec_pos {
                keyboard::send_editing_text(
                    text,
                    start_pos as i32,
                    end_pos.wrapping_sub(start_pos) as i32,
                );
            } else if let Some(pos) = pos {
                keyboard::send_editing_text(text, pos as i32, -1);
            } else {
                keyboard::send_editing_text(text, -1, -1);
            }
        }

        update_text_input_area(keyboard::keyboard_focus());

        return HandlerResult::Handled;
    }

    if msg.is_signal(ibus_input_interface, "HidePreeditText") {
        keyboard::send_editing_text("", 0, 0);
        return HandlerResult::Handled;
    }

    HandlerResult::NotYetHandled
}

/// Translation of `IBus_ReadAddressFromFile()`.
fn ibus_read_address_from_file(file_path: &str) -> Option<String> {
    const PREFIX: &[u8] = b"IBUS_ADDRESS=";
    let addr_file = std::fs::File::open(file_path).ok()?;

    let mut reader = std::io::BufReader::new(addr_file);
    let mut addr_buf = Vec::new();
    loop {
        addr_buf.clear();
        match reader.read_until(b'\n', &mut addr_buf) {
            Ok(0) | Err(_) => return None,
            Ok(_) => {}
        }
        if addr_buf.starts_with(PREFIX) {
            let sz = addr_buf.len();
            let mut end = sz;
            if addr_buf[sz - 1] == b'\n' {
                end = sz - 1;
            }
            if addr_buf[sz - 2] == b'\r' {
                end = end.min(sz - 2);
            }
            return Some(String::from_utf8_lossy(&addr_buf[PREFIX.len()..end]).into_owned());
        }
    }
}

/// The address file's path for a display, session type, config dirs and
/// machine ID. The computation of `IBus_GetDBusAddressFilename()`.
fn ibus_address_filename(
    disp_env: Option<&str>,
    session: Option<&str>,
    conf_env: Option<&str>,
    home_env: Option<&str>,
    machine_id: impl FnOnce() -> Option<String>,
) -> Option<String> {
    /* Otherwise, we have to get the hostname, display, machine id, config dir
    and look up the address from a filepath using all those bits, eek. */
    let mut display: Vec<u8> = match disp_env {
        Some(d) if !d.is_empty() => d.as_bytes().to_vec(),
        _ => b":0.0".to_vec(),
    };

    let disp_num = display.iter().rposition(|&c| c == b':')?;
    let screen_num = display.iter().rposition(|&c| c == b'.');

    // (the string is cut with NULs where it is split)
    display[disp_num] = 0;

    if let Some(screen_num) = screen_num {
        display[screen_num] = 0;
    }
    let cstr = |from: usize| {
        let s = &display[from..];
        let end = s.iter().position(|&c| c == 0).unwrap_or(s.len());
        String::from_utf8_lossy(&s[..end]).into_owned()
    };
    let mut host = cstr(0);
    let disp_num = cstr(disp_num + 1);

    if host.is_empty() {
        host = if session == Some("wayland") {
            "unix-wayland".to_owned()
        } else {
            "unix".to_owned()
        };
    }

    let config_dir = match conf_env {
        Some(c) if !c.is_empty() => c.to_owned(),
        _ => match home_env {
            Some(h) if !h.is_empty() => format!("{h}/.config"),
            _ => return None,
        },
    };

    let key = machine_id()?;

    Some(format!("{config_dir}/ibus/bus/{key}-{host}-{disp_num}"))
}

/// Translation of `IBus_GetDBusAddressFilename()`.
fn ibus_get_dbus_address_filename() -> Option<String> {
    if let Some(file) = &state().ibus_addr_file {
        return Some(file.clone());
    }

    dbus::context()?;

    // Use this environment variable if it exists.
    // FIXME (upstream): this is an address, not the name of a file holding
    // one, so reading it as a file fails.
    if let Some(addr) = crate::stdlib::getenv("IBUS_ADDRESS").filter(|a| !a.is_empty()) {
        return Some(addr);
    }

    let env = crate::stdlib::getenv;
    ibus_address_filename(
        env("DISPLAY").as_deref(),
        env("XDG_SESSION_TYPE").as_deref(),
        env("XDG_CONFIG_HOME").as_deref(),
        env("HOME").as_deref(),
        || dbus::get_local_machine_id().ok(),
    )
}

/// Translation of `IBus_SetCapabilities()`.
fn ibus_set_capabilities(hint: Option<&str>) {
    if ibus_check_connection() {
        let mut caps = IBUS_CAP_FOCUS;

        if hint.is_some_and(|h| h.contains("composition")) {
            caps |= IBUS_CAP_PREEDIT_TEXT;
        }
        if hint.is_some_and(|h| h.contains("candidates")) {
            // FIXME, turn off native candidate rendering
        }

        if let Some(t) = target() {
            t.call_void("SetCapabilities", &[Arg::U32(caps)]);
        }
    }
}

/// Create the input context; its object path. A step of
/// `IBus_SetupConnection()`.
fn create_input_context(conn: &Connection, service: &str, interface: &str) -> Option<String> {
    let client_name = "SDL3_Application";
    let reply = dbus::call_method_on_connection(
        conn,
        service,
        IBUS_PATH,
        interface,
        "CreateInputContext",
        &[Arg::Str(client_name)],
    )?;
    match reply.args().into_iter().next() {
        Some(Value::ObjectPath(path)) => Some(path),
        _ => None,
    }
}

/// Translation of `IBus_SetupConnection()`.
fn ibus_setup_connection(ctx: &Arc<Context>, addr: &str) -> bool {
    /* try the portal interface first. Modern systems have this in general,
    and sandbox things like FlakPak and Snaps, etc, require it. */
    let mut path = create_input_context(
        &ctx.session_conn,
        IBUS_PORTAL_SERVICE,
        IBUS_PORTAL_INTERFACE,
    );

    let conn = if path.is_some() {
        // reusing dbus->session_conn
        let mut s = state();
        s.ibus_is_portal_interface = true;
        s.ibus_service = IBUS_PORTAL_SERVICE;
        s.ibus_interface = IBUS_PORTAL_INTERFACE;
        s.ibus_input_interface = IBUS_PORTAL_INPUT_INTERFACE;
        IBusConn::Session(ctx.clone())
    } else {
        {
            let mut s = state();
            s.ibus_is_portal_interface = false;
            s.ibus_service = IBUS_SERVICE;
            s.ibus_interface = IBUS_INTERFACE;
            s.ibus_input_interface = IBUS_INPUT_INTERFACE;
            s.ibus_conn = None;
        }
        // (connection_open_private() and bus_register())
        let Ok(conn) = Connection::open_address(addr) else {
            return false; // oh well.
        };
        conn.flush();

        path = create_input_context(&conn, IBUS_SERVICE, IBUS_INTERFACE);
        IBusConn::Private(Arc::new(conn))
    };

    let input_interface = {
        let mut s = state();
        s.ibus_conn = Some(conn.clone());
        s.ibus_input_interface
    };

    let result = path.is_some();
    if let Some(path) = path {
        let matchstr = format!("type='signal',interface='{input_interface}'");
        state().input_ctx_path = Some(path.clone());

        // (adding the callback replaces the previous one, as upstream's
        // SDL_AddHintCallback() does; it runs at once, without our lock)
        let watch = hints::watch(hints::IME_IMPLEMENTED_UI, |change| {
            ibus_set_capabilities(change.new_value)
        })
        .ok();
        let old = std::mem::replace(&mut state().capabilities_watch, watch);
        drop(old);

        let c = conn.conn();
        let _ = c.add_match(&matchstr);
        let _ = c.try_register_object_path(&path, 0, |_, msg| ibus_message_handler(msg));
        c.flush();
    }

    let window = keyboard::keyboard_focus();
    if window.is_some_and(keyboard::text_input_active) {
        set_focus(true);
        update_text_input_area(window);
    } else {
        set_focus(false);
    }
    result
}

/// The names in the inotify events waiting on `fd`.
fn inotify_event_names(fd: &OwnedFd) -> Vec<Vec<u8>> {
    const HEADER: usize = std::mem::size_of::<libc::inotify_event>();
    let mut buf = [0u8; 1024];
    // SAFETY: buf is writable for its length; fd is open (and non-blocking).
    let readsize = unsafe { libc::read(fd.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len()) };
    if readsize <= 0 {
        return Vec::new();
    }
    let buf = &buf[..readsize as usize];

    let mut names = Vec::new();
    let mut p = 0;
    while p + HEADER <= buf.len() {
        // (struct inotify_event: wd, mask, cookie, len, then the name)
        let len = u32::from_ne_bytes([buf[p + 12], buf[p + 13], buf[p + 14], buf[p + 15]]) as usize;
        let name = buf.get(p + HEADER..p + HEADER + len).unwrap_or_default();
        let end = name.iter().position(|&c| c == 0).unwrap_or(name.len());
        names.push(name[..end].to_vec());
        p += HEADER + len;
    }
    names
}

/// Translation of `IBus_CheckConnection()`.
fn ibus_check_connection() -> bool {
    let Some(ctx) = dbus::context() else {
        return false;
    };

    let addr_file = {
        let s = state();
        if s.ibus_conn
            .as_ref()
            .is_some_and(|c| c.conn().is_connected())
        {
            return true;
        }

        let Some(fd) = s.inotify_fd.as_ref().filter(|fd| fd.as_raw_fd() > 0) else {
            return false;
        };
        if s.inotify_wd <= 0 {
            return false;
        }

        let names = inotify_event_names(fd);
        if names.is_empty() {
            return false;
        }
        let addr_file = s.ibus_addr_file.clone().unwrap_or_default();
        let Some(slash) = addr_file.rfind('/') else {
            return false;
        };
        let addr_file_no_path = &addr_file.as_bytes()[slash + 1..];
        let file_updated = names
            .iter()
            .any(|name| !name.is_empty() && name.as_slice() == addr_file_no_path);
        if !file_updated {
            return false;
        }
        addr_file
    };

    match ibus_read_address_from_file(&addr_file) {
        Some(addr) => ibus_setup_connection(&ctx, &addr),
        None => false,
    }
}

/// Connect to IBus; false if it isn't running. Translation of
/// `SDL_IBus_Init()`.
pub(crate) fn init() -> bool {
    let Some(ctx) = dbus::context() else {
        return false;
    };

    let Some(addr_file) = ibus_get_dbus_address_filename() else {
        return false;
    };

    let Some(addr) = ibus_read_address_from_file(&addr_file) else {
        return false;
    };

    {
        let mut s = state();
        s.ibus_addr_file = Some(addr_file.clone());

        if s.inotify_fd.is_none() {
            // SAFETY: plain system calls; a new descriptor is owned below.
            let fd = unsafe { libc::inotify_init() };
            if fd >= 0 {
                // SAFETY: fd is open.
                unsafe { libc::fcntl(fd, libc::F_SETFL, libc::O_NONBLOCK) };
                // SAFETY: fd is a new descriptor we own.
                s.inotify_fd = Some(unsafe { OwnedFd::from_raw_fd(fd) });
            }
        }

        let addr_file_dir = match addr_file.rfind('/') {
            Some(i) => &addr_file[..i],
            None => addr_file.as_str(),
        };

        s.inotify_wd = match (&s.inotify_fd, std::ffi::CString::new(addr_file_dir)) {
            (Some(fd), Ok(dir)) => {
                // SAFETY: fd is open; dir is NUL-terminated.
                unsafe {
                    libc::inotify_add_watch(
                        fd.as_raw_fd(),
                        dir.as_ptr(),
                        libc::IN_CREATE | libc::IN_MODIFY,
                    )
                }
            }
            _ => -1,
        };
    }

    let result = ibus_setup_connection(&ctx, &addr);

    // don't use the addr_file if using the portal interface.
    let mut s = state();
    if result && s.ibus_is_portal_interface {
        if let Some(fd) = s.inotify_fd.take() {
            if fd.as_raw_fd() > 0 && s.inotify_wd > 0 {
                // SAFETY: fd is open; the watch was added to it.
                unsafe { libc::inotify_rm_watch(fd.as_raw_fd(), s.inotify_wd) };
                s.inotify_wd = -1;
            }
            drop(fd);
        }
    }

    result
}

/// Translation of `SDL_IBus_Quit()`.
pub(crate) fn quit() {
    let (conn, watch) = {
        let mut s = state();
        s.input_ctx_path = None;
        s.ibus_addr_file = None;

        let conn = s.ibus_conn.take();
        s.ibus_service = "";
        s.ibus_interface = "";
        s.ibus_input_interface = "";
        s.ibus_is_portal_interface = false;

        if let Some(fd) = &s.inotify_fd {
            if fd.as_raw_fd() > 0 && s.inotify_wd > 0 {
                // SAFETY: fd is open; the watch was added to it.
                unsafe { libc::inotify_rm_watch(fd.as_raw_fd(), s.inotify_wd) };
                s.inotify_wd = -1;
            }
        }

        // !!! FIXME: should we close(inotify_fd) here?

        s.ibus_cursor_rect = Rect::default();
        (conn, s.capabilities_watch.take())
    };

    // if using portal, ibus_conn == session_conn; don't release it here.
    if let Some(IBusConn::Private(conn)) = &conn {
        conn.close();
    }
    drop(conn);
    // (SDL_RemoveHintCallback(), outside our lock)
    drop(watch);
}

/// Translation of `IBus_SimpleMessage()`.
fn ibus_simple_message(method: &str) {
    if state().input_ctx_path.is_some() && ibus_check_connection() {
        if let Some(t) = target() {
            t.call_void(method, &[]);
        }
    }
}

/// Let the IBus server know about changes in window focus. Translation of
/// `SDL_IBus_SetFocus()`.
pub(crate) fn set_focus(focused: bool) {
    let method = if focused { "FocusIn" } else { "FocusOut" };
    ibus_simple_message(method);
}

/// Close the candidate list and reset any text currently being edited.
/// Translation of `SDL_IBus_Reset()`.
pub(crate) fn reset() {
    ibus_simple_message("Reset");
}

/// Send a keypress event to IBus; true if IBus used it to update its
/// candidate list or change input methods. Translation of
/// `SDL_IBus_ProcessKeyEvent()`.
pub(crate) fn process_key_event(keysym: u32, keycode: u32, down: bool) -> bool {
    let mut result = false;

    if ibus_check_connection() {
        let mut mods = ibus_mod_state();
        let ibus_keycode = keycode.wrapping_sub(8);
        if !down {
            mods |= IBUS_RELEASE_MASK;
        }
        if let Some(Target {
            conn,
            service,
            path: Some(path),
            input_interface,
        }) = target()
        {
            let reply = dbus::call_method_on_connection(
                conn.conn(),
                service,
                &path,
                input_interface,
                "ProcessKeyEvent",
                &[Arg::U32(keysym), Arg::U32(ibus_keycode), Arg::U32(mods)],
            );
            result = matches!(
                reply.map(|r| r.args()).as_deref(),
                Some([Value::Bool(true), ..])
            );
        }
    }

    update_text_input_area(keyboard::keyboard_focus());

    result
}

/// Update the position of IBus' candidate list. Translation of
/// `SDL_IBus_UpdateTextInputArea()`.
pub(crate) fn update_text_input_area(window: Option<WindowID>) {
    let Some(window) = window else {
        return;
    };
    let Some((cursor, (mut x, mut y))) = super::ime::window_input_cursor(window) else {
        return;
    };

    state().ibus_cursor_rect = cursor;

    x += cursor.x;
    y += cursor.y;

    if ibus_check_connection() {
        if let Some(t) = target() {
            t.call_void(
                "SetCursorLocation",
                &[
                    Arg::I32(x),
                    Arg::I32(y),
                    Arg::I32(cursor.w),
                    Arg::I32(cursor.h),
                ],
            );
        }
    }
}

/// Check D-Bus for new IBus events, delivering their text input and
/// editing. Translation of `SDL_IBus_PumpEvents()`.
pub(crate) fn pump_events() {
    if ibus_check_connection() {
        if let Some(t) = target() {
            let conn = t.conn.conn();
            conn.read_write(0);

            while conn.dispatch() {
                // Do nothing, actual work happens in IBus_MessageHandler
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::linux::dbus::test_bus::{self, peer, Bus};
    use crate::core::linux::dbus::Writer;
    use crate::events::{Event, EventType};

    #[test]
    fn address_file_name() {
        let id = || Some("mid".to_owned());
        let name = |display: Option<&str>, session: Option<&str>| {
            ibus_address_filename(display, session, Some("/cfg"), None, id)
        };
        assert_eq!(
            name(Some(":1.0"), None).unwrap(),
            "/cfg/ibus/bus/mid-unix-1"
        );
        assert_eq!(
            name(None, Some("wayland")).unwrap(),
            "/cfg/ibus/bus/mid-unix-wayland-0"
        );
        assert_eq!(
            name(Some("host:2"), None).unwrap(),
            "/cfg/ibus/bus/mid-host-2"
        );
        // (a dot in the host name cuts it, as upstream)
        assert_eq!(
            name(Some("my.host:3"), None).unwrap(),
            "/cfg/ibus/bus/mid-my-3"
        );
        assert_eq!(name(Some("nocolon"), None), None);
        assert_eq!(
            ibus_address_filename(Some(":0"), None, None, Some("/h"), id).unwrap(),
            "/h/.config/ibus/bus/mid-unix-0"
        );
        assert_eq!(
            ibus_address_filename(Some(":0"), None, None, None, id),
            None
        );
        assert_eq!(
            ibus_address_filename(Some(":0"), None, Some("/c"), None, || None),
            None
        );
    }

    #[test]
    fn address_from_file() {
        let dir = crate::test_support::TempDir::new("ibus");
        let file = dir.path("addr");
        std::fs::write(
            &file,
            "# comment\nIBUS_ADDRESS=unix:path=/x,guid=1\r\nIBUS_DAEMON_PID=1\n",
        )
        .unwrap();
        assert_eq!(
            ibus_read_address_from_file(&file).as_deref(),
            Some("unix:path=/x,guid=1")
        );
        std::fs::write(&file, "IBUS_DAEMON_PID=1\n").unwrap();
        assert_eq!(ibus_read_address_from_file(&file), None);
        assert_eq!(ibus_read_address_from_file(&dir.path("none")), None);
    }

    /// An IBusText variant, with background attributes for `selection`.
    fn ibus_text(w: &mut Writer<'_>, text: &str, selection: Option<(u32, u32)>) -> bool {
        w.variant("(sa{sv}sv)", |v| {
            v.container(b'r', None, |st| {
                st.append(&Arg::Str("IBusText"))
                    && st.container(b'a', Some("{sv}"), |_| true)
                    && st.append(&Arg::Str(text))
                    && st.variant("(sa{sv}av)", |attrs| {
                        attrs.container(b'r', None, |list| {
                            list.append(&Arg::Str("IBusAttrList"))
                                && list.container(b'a', Some("{sv}"), |_| true)
                                && list.container(b'a', Some("v"), |a| {
                                    let Some((start, end)) = selection else {
                                        return true;
                                    };
                                    a.variant("(sa{sv}uuuu)", |attr| {
                                        attr.container(b'r', None, |f| {
                                            f.append(&Arg::Str("IBusAttribute"))
                                                && f.container(b'a', Some("{sv}"), |_| true)
                                                && f.append(&Arg::U32(3))
                                                && f.append(&Arg::U32(0xffffff))
                                                && f.append(&Arg::U32(start))
                                                && f.append(&Arg::U32(end))
                                        })
                                    })
                                })
                        })
                    })
            })
        })
    }

    #[test]
    fn ibus_portal() {
        crate::events::window::tests::with_video(&[1], |video| {
            let Some(bus) = Bus::start() else { return };
            const CTX_PATH: &str = "/org/freedesktop/IBus/InputContext_7";
            let calls = Arc::new(Mutex::new(Vec::new()));
            let c2 = calls.clone();
            let portal = peer(&bus.address, IBUS_PORTAL_SERVICE, move |_, msg| {
                if msg.is_method_call(IBUS_PORTAL_INTERFACE, "CreateInputContext") {
                    let mut reply = msg.new_method_return()?;
                    reply.append_args(&[Arg::ObjectPath(CTX_PATH)]);
                    return Some(reply);
                }
                for method in [
                    "SetCapabilities",
                    "FocusIn",
                    "FocusOut",
                    "Reset",
                    "ProcessKeyEvent",
                    "SetCursorLocation",
                ] {
                    if msg.is_method_call(IBUS_PORTAL_INPUT_INTERFACE, method) {
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

            let ctx = dbus::context().unwrap();
            assert!(ibus_setup_connection(&ctx, "unused"));
            assert!(state().ibus_is_portal_interface);
            assert!(process_key_event(0x61, 38, true));
            assert!(process_key_event(0x61, 38, false));
            reset();
            // (no SetCursorLocation: the fake video has no text input area)
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
                        "SetCapabilities",
                        "FocusIn",
                        "ProcessKeyEvent",
                        "ProcessKeyEvent",
                        "Reset"
                    ]
                );
                assert_eq!(calls[0].1, [Value::U32(IBUS_CAP_FOCUS)]);
                assert_eq!(
                    calls[2].1,
                    [Value::U32(0x61), Value::U32(30), Value::U32(0)]
                );
                assert_eq!(
                    calls[3].1,
                    [
                        Value::U32(0x61),
                        Value::U32(30),
                        Value::U32(IBUS_RELEASE_MASK)
                    ]
                );
            }

            // Signals on the input context: text, preedit, hidden preedit.
            let emitter = Connection::open_address(&bus.address).unwrap();
            let signal = |name: &str, fill: &dyn Fn(&mut Writer<'_>)| {
                let mut s = emitter
                    .new_signal(CTX_PATH, IBUS_PORTAL_INPUT_INTERFACE, name)
                    .unwrap();
                fill(&mut s.writer());
                assert!(emitter.send(&s));
            };
            signal("CommitText", &|w| {
                ibus_text(w, "hello", None);
            });
            signal("UpdatePreeditText", &|w| {
                ibus_text(w, "abc", None);
                w.append(&Arg::U32(2));
                w.append(&Arg::Bool(true));
            });
            signal("UpdatePreeditText", &|w| {
                ibus_text(w, "wxyz", Some((1, 3)));
                w.append(&Arg::U32(0));
                w.append(&Arg::Bool(true));
            });
            signal("HidePreeditText", &|_| {});
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
                    ("hello".to_owned(), -2, -2),
                    ("abc".to_owned(), 2, -1),
                    ("wxyz".to_owned(), 1, 2),
                    (String::new(), 0, 0),
                ]
            );

            quit();
            assert!(state().ibus_conn.is_none());
            drop(ctx);
            test_bus::release_session();
            portal.stop();
        });
    }
}
