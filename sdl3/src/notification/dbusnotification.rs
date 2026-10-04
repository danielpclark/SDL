// Rust translation of src/notification/unix/SDL_dbusnotification.c from
// Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Notifications over D-Bus: `org.freedesktop.Notifications` outside a
//! sandbox, the `org.freedesktop.portal.Notification` portal inside one
//! (each falling back to the other). Action invocations arrive as signals,
//! handled while the event loop pumps D-Bus.

use std::io::Read;
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::Duration;

use super::{Notification, NotificationAction, NotificationID, NotificationPriority};
use crate::core::linux::dbus::{
    self, Arg, Context, FilterHandle, HandlerResult, Message, Value, Writer,
};
use crate::error::{Error, Result};
use crate::log::Category;
use crate::video::pixels::PixelFormat;
use crate::video::surface::Surface;

const NOTIFICATION_PORTAL_NODE: &str = "org.freedesktop.portal.Desktop";
const NOTIFICATION_PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const NOTIFICATION_PORTAL_INTERFACE: &str = "org.freedesktop.portal.Notification";

const NOTIFICATION_CORE_NODE: &str = "org.freedesktop.Notifications";
const NOTIFICATION_CORE_PATH: &str = "/org/freedesktop/Notifications";
const NOTIFICATION_CORE_INTERFACE: &str = "org.freedesktop.Notifications";

const NOTIFICATION_ACTION_SIGNAL_NAME: &str = "ActionInvoked";
const NOTIFICATION_CLOSED_SIGNAL_NAME: &str = "NotificationClosed";
const NOTIFICATION_ACTIVATION_TOKEN_SIGNAL_NAME: &str = "ActivationToken";

const SDL_NOTIFICATION_PREAMBLE: &str = "SDL_LocalNotification-";

const ACTIVATION_TOKEN_LIFETIME: Duration = Duration::from_secs(1);

fn core_match(member: &str) -> String {
    format!("type='signal', interface='{NOTIFICATION_CORE_INTERFACE}',member='{member}'")
}

fn portal_match() -> String {
    format!(
        "type='signal', interface='{NOTIFICATION_PORTAL_INTERFACE}',member='{NOTIFICATION_ACTION_SIGNAL_NAME}'"
    )
}

/// The file's statics.
#[derive(Default)]
struct State {
    activation_token_time: Duration,
    activation_token: Option<String>,

    icon_uri: Option<String>,
    session_id: u64,

    core_id_list: Vec<u32>,

    interface_version: u32,

    /// The filter of the initialized interface, with the connection it is on.
    core_interface_initialized: Option<(Weak<Context>, FilterHandle)>,
    portal_interface_initialized: Option<(Weak<Context>, FilterHandle)>,

    /// `interface_unavailable` of `InitCoreSignalListener()` and
    /// `InitPortalSignalListener()`.
    core_interface_unavailable: bool,
    portal_interface_unavailable: bool,
}

/// `core_id_list` holds this many IDs.
const CORE_ID_LIST_LEN: usize = 32;

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut guard: MutexGuard<'_, Option<State>> = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(guard.get_or_insert_with(State::default))
}

/// Translation of `GetRandom()`.
fn get_random(dst: &mut [u8]) {
    let file = std::fs::File::open("/dev/urandom").or_else(|_| std::fs::File::open("/dev/random"));
    match file {
        Ok(mut f) => {
            // FIXME (upstream): this retries forever if the device can't
            // give the full amount.
            loop {
                if let Ok(n) = f.read(dst) {
                    if n == dst.len() {
                        break;
                    }
                }
            }
        }
        Err(_) => {
            let mut written = 0;
            while written < dst.len() {
                let tval = crate::timer::ticks_ns().to_ne_bytes();
                let towrite = (dst.len() - written).min(tval.len());
                dst[written..written + towrite].copy_from_slice(&tval[..towrite]);
                written += towrite;
            }
        }
    }
}

fn random_u64() -> u64 {
    let mut b = [0u8; 8];
    get_random(&mut b);
    u64::from_ne_bytes(b)
}

fn random_u32() -> u32 {
    let mut b = [0u8; 4];
    get_random(&mut b);
    u32::from_ne_bytes(b)
}

/// Translation of `AppendStringOption()`.
fn append_string_option(options: &mut Writer<'_>, key: &str, value: &str) -> bool {
    options.dict_entry(key, "s", |v| v.append(&Arg::Str(value)))
}

/// Translation of `AppendTargetString()`.
fn append_target_string(options: &mut Writer<'_>, session_id: u64, key: &str) -> bool {
    let target_val = session_id.to_string();
    options.dict_entry(key, "v", |target_variant| {
        target_variant.variant("s", |target_string| {
            target_string.append(&Arg::Str(&target_val))
        })
    })
}

/// Translation of `RemoveIDFromListAtIndex()`.
fn remove_id_from_list_at_index(state: &mut State, index: usize) {
    state.core_id_list.remove(index);
}

fn exists(path: &str) -> bool {
    std::fs::metadata(path).is_ok()
}

/// Translation of `HasDesktopFile()`.
fn has_desktop_file(app_id: &str) -> bool {
    // FIXME (upstream): with XDG_DATA_HOME set, the second check looks in
    // "$XDG_DATA_HOME/.local/share/applications"; without it, it repeats
    // the "$HOME/.local/share/applications" check. Kept.
    let mut xdg_data_home = crate::stdlib::getenv("XDG_DATA_HOME");
    if let Some(dir) = &xdg_data_home {
        if exists(&format!("{dir}/applications/{app_id}.desktop")) {
            return true;
        }
    } else {
        xdg_data_home = crate::stdlib::getenv("HOME");
        if let Some(dir) = &xdg_data_home {
            if exists(&format!("{dir}/.local/share/applications/{app_id}.desktop")) {
                return true;
            }
        }
    }

    if let Some(dir) = &xdg_data_home {
        if exists(&format!("{dir}/.local/share/applications/{app_id}.desktop")) {
            return true;
        }
    }

    if exists(&format!("/usr/share/local/applications/{app_id}.desktop")) {
        return true;
    }

    if exists(&format!("/usr/share/applications/{app_id}.desktop")) {
        return true;
    }

    false
}

/// Send the action event (`SDL_SendNotificationAction()`), outside the state
/// lock.
fn send_action(id: u32, action: &str) {
    crate::events::window::send_notification_action(id, action);
}

/// org.freedesktop.Notifications, used when not running inside a container.
/// Translation of `CoreNotificationFilter()`.
fn core_notification_filter(msg: &Message) -> HandlerResult {
    if msg.is_signal(NOTIFICATION_CORE_NODE, NOTIFICATION_ACTION_SIGNAL_NAME) {
        let args = msg.args();
        // Check if the parameters are what we expect
        let Some(Value::U32(id)) = args.first() else {
            return HandlerResult::NotYetHandled;
        };
        let id = *id;

        // See if this signal is for this client.
        let own_id = with_state(
            |state| match state.core_id_list.iter().position(|&i| i == id) {
                Some(i) => {
                    remove_id_from_list_at_index(state, i);
                    true
                }
                None => false,
            },
        );
        if !own_id {
            return HandlerResult::NotYetHandled;
        }

        let Some(Value::Str(button)) = args.get(1) else {
            return HandlerResult::NotYetHandled;
        };
        send_action(id, button);

        return HandlerResult::Handled;
    } else if msg.is_signal(NOTIFICATION_CORE_NODE, NOTIFICATION_CLOSED_SIGNAL_NAME) {
        let args = msg.args();
        // Check if the parameters are what we expect
        let (Some(Value::U32(id)), Some(Value::U32(reason))) = (args.first(), args.get(1)) else {
            return HandlerResult::NotYetHandled;
        };
        if *id != 0 && *reason != 0 {
            let removed =
                with_state(
                    |state| match state.core_id_list.iter().position(|i| i == id) {
                        Some(i) => {
                            remove_id_from_list_at_index(state, i);
                            true
                        }
                        None => false,
                    },
                );
            if removed {
                return HandlerResult::Handled;
            }
        }
    } else if msg.is_signal(
        NOTIFICATION_CORE_NODE,
        NOTIFICATION_ACTIVATION_TOKEN_SIGNAL_NAME,
    ) {
        let args = msg.args();
        // Check if the parameters are what we expect
        let (Some(Value::U32(id)), Some(Value::Str(token))) = (args.first(), args.get(1)) else {
            return HandlerResult::NotYetHandled;
        };
        if *id != 0 {
            let stored = with_state(|state| {
                if state.core_id_list.contains(id) {
                    state.activation_token = Some(token.clone());
                    state.activation_token_time = crate::timer::ticks();
                    true
                } else {
                    false
                }
            });
            if stored {
                return HandlerResult::Handled;
            }
        }
    }

    HandlerResult::NotYetHandled
}

/// Translation of `SetCoreImage()`.
fn set_core_image(iter_init: &mut Writer<'_>, surface: &Surface<'_>) -> bool {
    let alpha = true;
    let bpp = 8;
    // Channels (always 4 with alpha)
    let channels = 4;
    let len = (surface.pitch() * surface.height()).max(0) as usize;
    let pixels = surface.pixels().unwrap_or_default();
    let pixels = &pixels[..len.min(pixels.len())];

    iter_init.dict_entry("image-data", "(iiibiiay)", |value| {
        value.container(b'r', None, |iter| {
            // Width, Height, Pitch, Alpha yes/no, BPP, Channels
            iter.append(&Arg::I32(surface.width()))
                && iter.append(&Arg::I32(surface.height()))
                && iter.append(&Arg::I32(surface.pitch()))
                && iter.append(&Arg::Bool(alpha))
                && iter.append(&Arg::I32(bpp))
                && iter.append(&Arg::I32(channels))
                // Raw image bytes
                && iter.container(b'a', Some("y"), |array| {
                    pixels.iter().all(|&b| array.append(&Arg::Byte(b)))
                })
        })
    })
}

/// The image as ABGR8888 (upstream converts only when it isn't already).
fn abgr_image(image: &Surface<'_>) -> Option<Surface<'static>> {
    image.convert(PixelFormat::ABGR8888).ok()
}

/// Translation of `SetCoreHints()`.
fn set_core_hints(iter_init: &mut Writer<'_>, notification: &Notification<'_>) -> bool {
    let app_id = crate::init::app_metadata_property(crate::init::AppMetadata::Identifier);
    let sound = notification.sound.as_deref().unwrap_or("default");
    let priority = notification.priority;
    let transient = notification.transient;

    iter_init.container(b'a', Some("{sv}"), |iter_dict| {
        let dbus_priority: u8 = match priority {
            NotificationPriority::Normal | NotificationPriority::High => 1,
            NotificationPriority::Low => 0,
            NotificationPriority::Critical => 2,
        };

        if !iter_dict.dict_entry("urgency", "y", |v| v.append(&Arg::Byte(dbus_priority))) {
            return false;
        }

        if let Some(app_id) = &app_id {
            if has_desktop_file(app_id)
                && !iter_dict.dict_entry("desktop-entry", "s", |v| v.append(&Arg::Str(app_id)))
            {
                return false;
            }
        }

        if !iter_dict.dict_entry("transient", "b", |v| v.append(&Arg::Bool(transient))) {
            return false;
        }

        if sound == "default" {
            if !append_string_option(iter_dict, "sound-name", "dialog-information") {
                return false;
            }
        } else if sound != "silent" {
            match std::fs::canonicalize(sound) {
                Ok(sound_path) => {
                    if !append_string_option(iter_dict, "sound-file", &sound_path.to_string_lossy())
                    {
                        return false;
                    }
                }
                Err(_) => {
                    crate::log::error!(
                        Category::Application,
                        "Notification sound '{}' not found",
                        sound
                    );

                    // Play the default if the custom sound is not found.
                    if !append_string_option(iter_dict, "sound-name", "dialog-information") {
                        return false;
                    }
                }
            }
        }

        if let Some(image_surface) = notification.image.and_then(abgr_image) {
            // (as upstream, a failure here is ignored)
            let _ = set_core_image(iter_dict, &image_surface);
        }

        true
    })
}

/// Translation of `InitCoreSignalListener()`.
fn init_core_signal_listener(ctx: &Arc<Context>) -> bool {
    if with_state(|s| s.core_interface_unavailable) {
        return false;
    }
    let conn = &ctx.session_conn;

    // Query the server information to see if the notification interface is available.
    let Some(msg) = conn.new_method_call(
        NOTIFICATION_CORE_NODE,
        NOTIFICATION_CORE_PATH,
        NOTIFICATION_CORE_INTERFACE,
        "GetServerInformation",
    ) else {
        return false;
    };
    if conn.send_with_reply_and_block(&msg, -1).is_err() {
        // Mark the interface as unavailable.
        with_state(|s| s.core_interface_unavailable = true);
        return false;
    }

    with_state(|s| {
        if s.session_id == 0 {
            s.session_id = random_u64();
        }
    });

    let action = core_match(NOTIFICATION_ACTION_SIGNAL_NAME);
    let closed = core_match(NOTIFICATION_CLOSED_SIGNAL_NAME);
    let token = core_match(NOTIFICATION_ACTIVATION_TOKEN_SIGNAL_NAME);

    if let Err(e) = conn.add_match(&action) {
        crate::log::debug!(
            Category::Application,
            "Failed to register DBus notification filter: {}",
            e
        );
        return false;
    }

    // On failure, undo all registrations.
    if let Err(e) = conn.add_match(&closed) {
        crate::log::debug!(
            Category::Application,
            "Failed to register DBus notification filter: {}",
            e
        );
        let _ = conn.remove_match(&action);
        return false;
    }

    if let Err(e) = conn.add_match(&token) {
        crate::log::debug!(
            Category::Application,
            "Failed to register DBus notification filter: {}",
            e
        );
        let _ = conn.remove_match(&closed);
        let _ = conn.remove_match(&action);
        return false;
    }

    match conn.add_owned_filter(core_notification_filter) {
        Some(handle) => {
            conn.flush();
            crate::log::debug!(
                Category::Application,
                "Registered DBus portal notification filter"
            );
            with_state(|s| s.core_interface_initialized = Some((Arc::downgrade(ctx), handle)));
            true
        }
        None => {
            crate::log::debug!(
                Category::Application,
                "Failed to register DBus notification filter"
            );
            let _ = conn.remove_match(&token);
            let _ = conn.remove_match(&closed);
            let _ = conn.remove_match(&action);
            false
        }
    }
}

/// Translation of `GetIconURI()`.
fn get_icon_uri(state: &mut State) -> Option<String> {
    if state.icon_uri.is_some() {
        return state.icon_uri.clone();
    }

    let props = crate::properties::Properties::global();
    let icon = props.get_string(super::PROP_GLOBAL_NOTIFICATION_HEADER_ICON_STRING);
    if let Some(icon) = icon {
        state.icon_uri = Some(match std::fs::canonicalize(&icon) {
            Ok(full_path) => format!("file://{}", full_path.to_string_lossy()),
            // If the path can't be retrieved, assume it is the system name of an icon.
            Err(_) => icon,
        });
    }

    state.icon_uri.clone()
}

/// Translation of `ShowCoreNotification()`.
fn show_core_notification(
    ctx: &Context,
    notification: &Notification<'_>,
) -> Result<NotificationID> {
    let conn = &ctx.session_conn;
    let timeout: i32 = -1;
    let failure =
        || Error::new("Failed to dispatch org.freedesktop.Notifications request (Out of memory?)");

    let replaces = notification.replaces.unwrap_or(0);
    let title = notification.title.as_deref().unwrap_or_default();
    let message = notification.message.as_deref();

    // Call org.freedesktop.Notifications.Notify()
    let mut msg = conn
        .new_method_call(
            NOTIFICATION_CORE_NODE,
            NOTIFICATION_CORE_PATH,
            NOTIFICATION_CORE_INTERFACE,
            "Notify",
        )
        .ok_or_else(failure)?;

    let icon_uri = with_state(get_icon_uri);
    let ok = {
        let mut iter = msg.writer();
        // App ID
        // FIXME (upstream): without a name, SDL_GetAppID()'s result is
        // dropped and NULL is appended (the name always has a default
        // here, so this doesn't happen).
        let app_name = crate::init::app_metadata_property(crate::init::AppMetadata::Name)
            .unwrap_or_else(crate::core::unix::app_id);
        iter.append(&Arg::Str(&app_name))
            // Replaces id
            && iter.append(&Arg::U32(replaces))
            // Icon URI
            && iter.append(&Arg::Str(icon_uri.as_deref().unwrap_or("")))
            // Summary
            && iter.append(&Arg::Str(title))
            // Body
            // FIXME (upstream): the message pointer itself is appended,
            // not the "" fallback computed for it (NULL crashes libdbus);
            // the fallback is appended here.
            && iter.append(&Arg::Str(message.unwrap_or("")))
            // Actions
            && iter.container(b'a', Some("s"), |array| {
                // Add the default action
                let tmpstr = "default";
                if !array.append(&Arg::Str(tmpstr)) || !array.append(&Arg::Str(tmpstr)) {
                    return false;
                }

                // Add the actions
                notification.actions.iter().all(|action| match action {
                    NotificationAction::Button {
                        action_id,
                        action_label,
                    } => array.append(&Arg::Str(action_id)) && array.append(&Arg::Str(action_label)),
                })
            })
            // Hints
            && set_core_hints(&mut iter, notification)
            // Timeout
            && iter.append(&Arg::I32(timeout))
    };
    if !ok {
        return Err(failure());
    }

    let reply = conn
        .send_with_reply_and_block(&msg, -1)
        .map_err(|e| Error::new(format!("Notification failed: {}", error_message(&e))))?;
    // FIXME (upstream): when the reply isn't a UINT32, the request message
    // is released a second time (a double unref); RAII releases it once.
    let message_id = match reply.args().first() {
        Some(Value::U32(id)) => *id,
        _ => return Err(failure()),
    };

    with_state(|state| {
        if state.core_id_list.len() == CORE_ID_LIST_LEN {
            remove_id_from_list_at_index(state, 0);
        }
        state.core_id_list.push(message_id);
    });

    Ok(message_id)
}

/// The `message` part of a D-Bus error ("name: message").
fn error_message(e: &Error) -> &str {
    let m = e.message();
    m.split_once(": ").map_or(m, |(_, message)| message)
}

/// Translation of `RemoveCoreNotification()`.
fn remove_core_notification(ctx: &Context, id: NotificationID) -> Result<()> {
    if id == 0 {
        return Err(Error::invalid_param("id"));
    }

    // Call org.freedesktop.Notifications.CloseNotification()
    let mut msg = ctx
        .session_conn
        .new_method_call(
            NOTIFICATION_CORE_NODE,
            NOTIFICATION_CORE_PATH,
            NOTIFICATION_CORE_INTERFACE,
            "CloseNotification",
        )
        .ok_or_else(Error::out_of_memory)?;

    if !msg.append_args(&[Arg::U32(id)]) {
        return Err(Error::out_of_memory());
    }
    if !ctx.session_conn.send_no_flush(&msg) {
        return Err(Error::new("Failed to send notification removal request"));
    }
    Ok(())
}

/// The notification ID in a portal notification's ID string.
///
/// FIXME (upstream): the number is read from one character past the
/// preamble (`sizeof` counts the NUL), so its first digit is skipped. Kept.
fn parse_portal_id(s: &str) -> Option<u32> {
    if !s.starts_with(SDL_NOTIFICATION_PREAMBLE) {
        return None;
    }
    let rest = s.get(SDL_NOTIFICATION_PREAMBLE.len() + 1..).unwrap_or("");
    let id = crate::stdlib::string::strtoul(rest, 10).0 as u32;
    (id != 0).then_some(id)
}

/// org.freedesktop.portal.Notification interface, used when running in a
/// Flatpak or SNAP container. Translation of `PortalNotificationFilter()`.
fn portal_notification_filter(msg: &Message) -> HandlerResult {
    if !msg.is_signal(
        NOTIFICATION_PORTAL_INTERFACE,
        NOTIFICATION_ACTION_SIGNAL_NAME,
    ) {
        return HandlerResult::NotYetHandled;
    }
    let args = msg.args();

    // Check if the parameters are what we expect
    let Some(Value::Str(id_str)) = args.first() else {
        return HandlerResult::NotYetHandled;
    };

    // Parse the ID.
    let Some(id) = parse_portal_id(id_str) else {
        return HandlerResult::NotYetHandled;
    };

    let Some(Value::Str(action)) = args.get(1) else {
        return HandlerResult::NotYetHandled;
    };

    // Check for the target and optional XDG activation parameter.
    let mut target = None;
    let mut token = None;

    if let Some(Value::Array(params)) = args.get(2) {
        let mut params = params.iter().peekable();

        // The order of parameters in the array is defined: first the target, then the activation ID.
        if let Some(Value::Variant(target_variant)) = params.peek() {
            if let Value::Variant(target_string) = &**target_variant {
                if let Value::Str(s) = &**target_string {
                    target = Some(s.as_str());
                }
            }
            params.next();
        }

        // System properties array.
        // FIXME (upstream): the dictionary entries are entered from the
        // array's iterator instead of the dictionary's, so libdbus reads a
        // container as a basic value; the entries are read here.
        if let Some(Value::Array(pdata)) = params.peek() {
            for entry in pdata {
                let Value::DictEntry(key, value) = entry else {
                    break;
                };
                // Get the activation token string.
                if key.as_str() == Some("activation-token") {
                    if let Value::Variant(v) = &**value {
                        if let Value::Str(s) = &**v {
                            token = Some(s.clone());
                        }
                    }

                    // Found the activation token, nothing else to do.
                    break;
                }
            }
        }
    }

    let session_id = with_state(|s| s.session_id);
    if let Some(target) = target {
        if crate::stdlib::string::strtoull(target, 10).0 == session_id {
            if let Some(token) = token {
                with_state(|s| {
                    s.activation_token = Some(token);
                    s.activation_token_time = crate::timer::ticks();
                });
            }
            send_action(id, action);

            return HandlerResult::Handled;
        }
    }

    HandlerResult::NotYetHandled
}

/// Write `data` to a new sealable memory file, at its start. Translation
/// of the `memfd_create()` parts of `SetPortalImage()`.
#[cfg(target_os = "linux")]
fn memfd_with(name: &str, data: &[u8], cloexec: bool) -> Option<std::os::fd::OwnedFd> {
    use std::os::fd::FromRawFd;
    let name = std::ffi::CString::new(name.replace('\0', "")).ok()?;
    let mut flags = libc::MFD_ALLOW_SEALING;
    if cloexec {
        flags |= libc::MFD_CLOEXEC;
    }
    // SAFETY: name is NUL-terminated.
    let fd = unsafe { libc::memfd_create(name.as_ptr(), flags) };
    if fd < 0 {
        return None;
    }
    // SAFETY: fd is a new descriptor we own.
    let fd = unsafe { std::os::fd::OwnedFd::from_raw_fd(fd) };
    let mut file = std::fs::File::from(fd);
    std::io::Write::write_all(&mut file, data).ok()?;
    Some(file.into())
}

/// Translation of `SetPortalImage()`.
fn set_portal_image(
    iter_init: &mut Writer<'_>,
    interface_version: u32,
    surface: &mut Surface<'_>,
) -> bool {
    let fd_string = "file-descriptor";
    let key = "icon";

    let mut png = crate::io::IoStream::from_dynamic_mem();
    if surface.save_png_io(&mut png).is_err() {
        return false;
    }
    let Some(png_data) = png.dynamic_memory() else {
        return false;
    };

    /* Version 2 of the portal interface wants images passed as a sealable file descriptor,
     * which is only possible with memfd_create().
     */
    #[cfg(target_os = "linux")]
    let fd = if interface_version >= 2 {
        memfd_with("SDL_NotificationImage", png_data, false)
    } else {
        None
    };
    #[cfg(not(target_os = "linux"))]
    let fd: Option<std::os::fd::OwnedFd> = {
        let _ = interface_version;
        None
    };

    match &fd {
        Some(fd) => iter_init.dict_entry(key, "(sv)", |variant_iter| {
            variant_iter.container(b'r', None, |struct_iter| {
                struct_iter.append(&Arg::Str(fd_string))
                    && struct_iter.variant("h", |fd_variant_iter| {
                        fd_variant_iter.append(&Arg::UnixFd(std::os::fd::AsRawFd::as_raw_fd(fd)))
                    })
            })
        }),
        None => {
            let bytes_string = "bytes";
            iter_init.dict_entry(key, "(sv)", |variant_iter| {
                variant_iter.container(b'r', None, |struct_iter| {
                    struct_iter.append(&Arg::Str(bytes_string))
                        && struct_iter.variant("ay", |byte_array_iter| {
                            byte_array_iter.container(b'a', Some("y"), |array_iter| {
                                png_data.iter().all(|&b| array_iter.append(&Arg::Byte(b)))
                            })
                        })
                })
            })
        }
    }
}

/// Translation of `SetFileSize()`.
#[cfg(target_os = "linux")]
fn set_file_size(fd: std::os::fd::RawFd, size: libc::off_t) -> bool {
    /* SIGALRM can potentially block a large posix_fallocate() operation
     * from succeeding, so block it.
     */
    // SAFETY: the sigsets are plain data initialized by sigemptyset.
    let mut set: libc::sigset_t = unsafe { std::mem::zeroed() };
    // SAFETY: as above.
    let mut old_set: libc::sigset_t = unsafe { std::mem::zeroed() };
    // SAFETY: set and old_set are valid sigsets; fd is open.
    let ret = unsafe {
        libc::sigemptyset(&mut set);
        libc::sigaddset(&mut set, libc::SIGALRM);
        libc::pthread_sigmask(libc::SIG_BLOCK, &set, &mut old_set);

        let mut ret;
        loop {
            ret = libc::posix_fallocate(fd, 0, size);
            if ret != libc::EINTR {
                break;
            }
        }

        libc::pthread_sigmask(libc::SIG_SETMASK, &old_set, std::ptr::null_mut());
        ret
    };

    if ret == 0 {
        return true;
    } else if ret != libc::EINVAL
        && std::io::Error::last_os_error().raw_os_error() != Some(libc::EOPNOTSUPP)
    {
        return false;
    }

    // SAFETY: fd is open.
    unsafe { libc::ftruncate(fd, size) >= 0 }
}

/// Translation of `SetPortalSound()`.
fn set_portal_sound(iter_init: &mut Writer<'_>, sound: &str) -> bool {
    let key = "sound";

    if sound == "default" {
        return append_string_option(iter_init, key, "default");
    } else if sound == "silent" {
        return true;
    }

    // Passing sound files to a portal must be done via a sealable file descriptor, which is only possible with memfd_create.
    #[cfg(target_os = "linux")]
    {
        use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
        use std::os::unix::fs::FileExt;

        let fd_string = "file-descriptor";
        let Ok(st) = std::fs::metadata(sound) else {
            // Log an error if the sound file can't be found, but this is not fatal.
            crate::log::error!(
                Category::Application,
                "Notification sound file '{}' not found",
                sound
            );
            return append_string_option(iter_init, key, "default");
        };
        // Copy the sound file to a memfd.
        let Ok(mut read_file) = std::fs::File::open(sound) else {
            // Log an error if the sound file can't be opened, but this is not fatal.
            crate::log::error!(
                Category::Application,
                "Notification sound file '{}' cannot be opened; check file permissions",
                sound
            );
            return append_string_option(iter_init, key, "default");
        };
        let Ok(name) = std::ffi::CString::new(sound) else {
            return false;
        };
        // SAFETY: name is NUL-terminated.
        let raw = unsafe {
            libc::memfd_create(name.as_ptr(), libc::MFD_ALLOW_SEALING | libc::MFD_CLOEXEC)
        };
        if raw < 0 {
            return false;
        }
        // SAFETY: raw is a new descriptor we own.
        let mem_fd = std::fs::File::from(unsafe { OwnedFd::from_raw_fd(raw) });

        let size = st.len();
        if !set_file_size(mem_fd.as_raw_fd(), size as libc::off_t) {
            return false;
        }

        // (upstream maps the memfd and reads the file into it; this writes
        // the same bytes at the same place, leaving the offset at 0)
        let mut data = Vec::with_capacity(size as usize);
        let res = read_file.read_to_end(&mut data);
        if res.ok() != Some(size as usize) || mem_fd.write_all_at(&data, 0).is_err() {
            return false;
        }

        // Set the tuple values (sv).
        iter_init.dict_entry(key, "(sv)", |variant_iter| {
            variant_iter.container(b'r', None, |struct_iter| {
                struct_iter.append(&Arg::Str(fd_string))
                    && struct_iter.variant("h", |fd_variant_iter| {
                        fd_variant_iter.append(&Arg::UnixFd(mem_fd.as_raw_fd()))
                    })
            })
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        true
    }
}

/// Translation of `AddPortalActions()`.
fn add_portal_actions(
    iter_init: &mut Writer<'_>,
    session_id: u64,
    actions: &[NotificationAction],
) -> bool {
    iter_init.dict_entry("buttons", "aa{sv}", |options_value| {
        options_value.container(b'a', Some("a{sv}"), |button_array| {
            actions.iter().all(|action| match action {
                NotificationAction::Button {
                    action_id,
                    action_label,
                } => button_array.container(b'a', Some("{sv}"), |properties_array| {
                    append_string_option(properties_array, "action", action_id)
                        && append_string_option(properties_array, "label", action_label)
                        && append_target_string(properties_array, session_id, "target")
                }),
            })
        })
    })
}

/// Translation of `SetPortalDisplayHints()`.
fn set_portal_display_hints(iter_init: &mut Writer<'_>, notification: &Notification<'_>) -> bool {
    let transient = notification.transient;
    iter_init.dict_entry("display-hint", "(as)", |options_value| {
        options_value.container(b'r', None, |var_struct| {
            var_struct.container(b'a', Some("s"), |string_array| {
                !transient || string_array.append(&Arg::Str("transient"))
            })
        })
    })
}

/// Translation of `InitPortalSignalListener()`.
fn init_portal_signal_listener(ctx: &Arc<Context>) -> bool {
    if with_state(|s| s.portal_interface_unavailable) {
        return false;
    }
    let conn = &ctx.session_conn;

    let version = dbus::query_property_on_connection(
        conn,
        NOTIFICATION_PORTAL_NODE,
        NOTIFICATION_PORTAL_PATH,
        NOTIFICATION_PORTAL_INTERFACE,
        "version",
    );
    let Some(Value::U32(version)) = version else {
        // Mark the interface as unavailable.
        with_state(|s| s.portal_interface_unavailable = true);
        return false;
    };

    with_state(|s| {
        s.interface_version = version;
        if s.session_id == 0 {
            s.session_id = random_u64();
        }
    });

    let rule = portal_match();
    if let Err(e) = conn.add_match(&rule) {
        crate::log::debug!(
            Category::Application,
            "Failed to register DBus portal notification filter: {}",
            e
        );
        return false;
    }

    match conn.add_owned_filter(portal_notification_filter) {
        Some(handle) => {
            conn.flush();
            crate::log::debug!(
                Category::Application,
                "Registered DBus portal notification filter"
            );
            with_state(|s| s.portal_interface_initialized = Some((Arc::downgrade(ctx), handle)));
            true
        }
        None => {
            let _ = conn.remove_match(&rule);
            false
        }
    }
}

/// Translation of `ShowPortalNotification()`.
fn show_portal_notification(
    ctx: &Context,
    notification: &Notification<'_>,
) -> Result<NotificationID> {
    let conn = &ctx.session_conn;
    let failure = || {
        Error::new(
            "Failed to dispatch org.freedesktop.portal.Notification request (Out of memory?)",
        )
    };

    let replaces = notification.replaces.unwrap_or(0);
    let title = notification.title.as_deref().unwrap_or_default();
    let message = notification.message.as_deref();
    let sound = notification.sound.as_deref().unwrap_or("default");
    let priority = notification.priority;
    let (session_id, interface_version) = with_state(|s| (s.session_id, s.interface_version));

    // Call Notification.AddNotification()
    let mut msg = conn
        .new_method_call(
            NOTIFICATION_PORTAL_NODE,
            NOTIFICATION_PORTAL_PATH,
            NOTIFICATION_PORTAL_INTERFACE,
            "AddNotification",
        )
        .ok_or_else(failure)?;

    // Notification ID
    let new_id = if replaces == 0 {
        random_u32()
    } else {
        replaces
    };

    let mut image = notification.image.and_then(abgr_image);
    let ok = {
        let mut iter = msg.writer();
        let id = format!("{SDL_NOTIFICATION_PREAMBLE}{new_id}");
        iter.append(&Arg::Str(&id));

        // Parameters
        iter.container(b'a', Some("{sv}"), |array| {
            let priority_str = match priority {
                NotificationPriority::Normal => "normal",
                NotificationPriority::Low => "low",
                NotificationPriority::High => "high",
                NotificationPriority::Critical => "urgent",
            };

            append_string_option(array, "title", title)
                // FIXME (upstream): a missing message is appended as a NULL
                // string (crashing libdbus); "" is appended here.
                && append_string_option(array, "body", message.unwrap_or(""))
                && append_string_option(array, "default-action", "default")
                && append_target_string(array, session_id, "default-action-target")
                && set_portal_sound(array, sound)
                && append_string_option(array, "priority", priority_str)
                && set_portal_display_hints(array, notification)
                && image
                    .as_mut()
                    .is_none_or(|image| set_portal_image(array, interface_version, image))
                && (notification.actions.is_empty()
                    || add_portal_actions(array, session_id, &notification.actions))
        })
    };
    if !ok {
        return Err(failure());
    }

    // FIXME (upstream): on a failed call the request message is released a
    // second time (a double unref); RAII releases it once.
    conn.send_with_reply_and_block(&msg, -1)
        .map_err(|e| Error::new(format!("Notification failed: {}", error_message(&e))))?;
    Ok(new_id)
}

/// Translation of `RemovePortalNotification()`.
fn remove_portal_notification(ctx: &Context, id: NotificationID) -> Result<()> {
    if id == 0 {
        return Err(Error::invalid_param("id"));
    }

    // Call org.freedesktop.Notifications.CloseNotification()
    let mut msg = ctx
        .session_conn
        .new_method_call(
            NOTIFICATION_PORTAL_NODE,
            NOTIFICATION_PORTAL_PATH,
            NOTIFICATION_PORTAL_INTERFACE,
            "RemoveNotification",
        )
        .ok_or_else(Error::out_of_memory)?;

    let id_str = format!("{SDL_NOTIFICATION_PREAMBLE}{id}");
    if !msg.append_args(&[Arg::Str(&id_str)]) {
        return Err(Error::out_of_memory());
    }
    if !ctx.session_conn.send_no_flush(&msg) {
        return Err(Error::new("Failed to send notification removal request"));
    }
    Ok(())
}

/// Which interface is in use.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Interface {
    Core,
    Portal,
}

fn initialized_interface() -> Option<Interface> {
    with_state(|s| {
        if s.portal_interface_initialized.is_some() {
            Some(Interface::Portal)
        } else if s.core_interface_initialized.is_some() {
            Some(Interface::Core)
        } else {
            None
        }
    })
}

/// Translation of `CheckInitNotifications()`.
fn check_init_notifications(ctx: Option<&Arc<Context>>) -> Result<Interface> {
    let Some(ctx) = ctx else {
        return Err(Error::new("D-Bus not available"));
    };

    if let Some(interface) = initialized_interface() {
        return Ok(interface);
    }

    type Init = fn(&Arc<Context>) -> bool;
    let order: [Init; 2] = if crate::init::sandbox() != crate::init::Sandbox::None {
        [init_portal_signal_listener, init_core_signal_listener]
    } else {
        [init_core_signal_listener, init_portal_signal_listener]
    };
    let ret = order.iter().any(|init| init(ctx));

    match (ret, initialized_interface()) {
        (true, Some(interface)) => Ok(interface),
        _ => Err(Error::new("Notification interface not available")),
    }
}

/// Translation of `SDL_SYS_ShowNotification()`.
pub(super) fn show_notification(notification: &Notification<'_>) -> Result<NotificationID> {
    let ctx = dbus::context();
    let interface = check_init_notifications(ctx.as_ref())?;
    let ctx = ctx.expect("checked");

    // The portal is only used if inside a container, or the app association can be wrong.
    match interface {
        Interface::Portal => show_portal_notification(&ctx, notification),
        Interface::Core => show_core_notification(&ctx, notification),
    }
}

/// Translation of `SDL_RemoveNotification()`.
pub(super) fn remove_notification(notification: NotificationID) -> Result<()> {
    let ctx = dbus::context();
    let interface = check_init_notifications(ctx.as_ref())?;
    let ctx = ctx.expect("checked");

    match interface {
        Interface::Portal => remove_portal_notification(&ctx, notification),
        Interface::Core => remove_core_notification(&ctx, notification),
    }
}

/// Translation of `SDL_CleanupNotifications()`.
pub(super) fn cleanup_notifications() {
    let (portal, core) = with_state(|s| {
        (
            s.portal_interface_initialized.take(),
            s.core_interface_initialized.take(),
        )
    });

    // (upstream fetches the D-Bus context, connecting if needed, before
    // checking whether there is anything to undo)
    if let Some((ctx, handle)) = portal {
        if let Some(ctx) = ctx.upgrade() {
            let conn = &ctx.session_conn;
            conn.remove_owned_filter(handle);
            let _ = conn.remove_match(&portal_match());
            conn.flush();
        }
    }
    if let Some((ctx, handle)) = core {
        if let Some(ctx) = ctx.upgrade() {
            let conn = &ctx.session_conn;
            conn.remove_owned_filter(handle);
            let _ = conn.remove_match(&core_match(NOTIFICATION_ACTION_SIGNAL_NAME));
            let _ = conn.remove_match(&core_match(NOTIFICATION_CLOSED_SIGNAL_NAME));
            let _ = conn.remove_match(&core_match(NOTIFICATION_ACTIVATION_TOKEN_SIGNAL_NAME));
            conn.flush();
        }
    }

    with_state(|s| {
        s.icon_uri = None;
        s.activation_token = None;
    });
}

/// Translation of `SDL_RequestNotificationPermission()`.
pub(super) fn request_notification_permission() -> Result<()> {
    // No API (yet) to ask permission; just make sure that notifications are available.
    check_init_notifications(dbus::context().as_ref()).map(|_| ())
}

/// The XDG activation token a notification action brought, if it arrived
/// within the last second. Translation of
/// `SDL_GetNotificationActivationToken()`.
pub(super) fn notification_activation_token() -> Option<String> {
    with_state(|s| {
        // Track the lifetime to avoid returning a stale token.
        let token = s.activation_token.as_ref()?;
        if crate::timer::ticks().saturating_sub(s.activation_token_time) < ACTIVATION_TOKEN_LIFETIME
        {
            s.activation_token_time = Duration::ZERO;
            return Some(token.clone());
        }
        None
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::linux::dbus::test_bus::{self, peer, Bus};
    use crate::core::linux::dbus::Connection;
    use crate::events::{Event, EventType};

    type Calls = Arc<Mutex<Vec<(String, Vec<Value>)>>>;

    fn reset() {
        *STATE.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    fn dict_get<'v>(dict: &'v [Value], key: &str) -> Option<&'v Value> {
        dict.iter().find_map(|e| match e {
            Value::DictEntry(k, v) if k.as_str() == Some(key) => Some(v.unvariant()),
            _ => None,
        })
    }

    fn actions_invoked() -> Vec<(u32, String)> {
        crate::events::get_events(
            EventType::NOTIFICATION_ACTION_INVOKED,
            EventType::NOTIFICATION_ACTION_INVOKED,
            16,
        )
        .unwrap()
        .into_iter()
        .filter_map(|e| match e {
            Event::Notification(n) => Some((n.which, n.action_id)),
            _ => None,
        })
        .collect()
    }

    fn wait_for_action() -> Vec<(u32, String)> {
        let mut got = Vec::new();
        test_bus::wait_for(dbus::pump_events, || {
            got.extend(actions_invoked());
            !got.is_empty()
        });
        got
    }

    fn emit(
        address: &str,
        path: &str,
        interface: &str,
        name: &str,
        fill: impl FnOnce(&mut Writer<'_>),
    ) {
        let conn = Connection::open_address(address).unwrap();
        let mut signal = conn.new_signal(path, interface, name).unwrap();
        fill(&mut signal.writer());
        assert!(conn.send(&signal));
    }

    #[test]
    fn portal_ids() {
        // (the first digit is skipped, as upstream)
        assert_eq!(parse_portal_id("SDL_LocalNotification-1234"), Some(234));
        assert_eq!(parse_portal_id("SDL_LocalNotification-5"), None);
        assert_eq!(parse_portal_id("Other-1234"), None);
    }

    #[test]
    fn core_notifications() {
        let _l = crate::test_support::test_lock();
        let Some(bus) = Bus::start() else { return };
        reset();

        // An org.freedesktop.Notifications server.
        let calls: Calls = Arc::default();
        let c2 = calls.clone();
        let server = peer(&bus.address, NOTIFICATION_CORE_NODE, move |_, msg| {
            if msg.is_method_call(NOTIFICATION_CORE_INTERFACE, "GetServerInformation") {
                let mut reply = msg.new_method_return()?;
                reply.append_args(&[
                    Arg::Str("mock"),
                    Arg::Str("libsdl"),
                    Arg::Str("1"),
                    Arg::Str("1.2"),
                ]);
                return Some(reply);
            }
            for method in ["Notify", "CloseNotification"] {
                if msg.is_method_call(NOTIFICATION_CORE_INTERFACE, method) {
                    let mut calls = c2.lock().unwrap();
                    calls.push((method.to_owned(), msg.args()));
                    let mut reply = msg.new_method_return()?;
                    if method == "Notify" {
                        reply.append_args(&[Arg::U32(40 + calls.len() as u32)]);
                    }
                    return Some(reply);
                }
            }
            None
        });
        test_bus::use_as_session(&bus);
        crate::init::init(crate::init::InitFlags::EVENTS).unwrap();

        super::super::request_notification_permission().unwrap();
        let mut image = Surface::new(1, 1, PixelFormat::ABGR8888).unwrap();
        image.pixels_mut().unwrap()[..4].copy_from_slice(&[1, 2, 3, 4]);
        let notification = Notification {
            title: Some("T".into()),
            image: Some(&image),
            actions: vec![NotificationAction::Button {
                action_id: "yes".into(),
                action_label: "Yes".into(),
            }],
            priority: NotificationPriority::Critical,
            sound: Some("silent".into()),
            transient: true,
            ..Notification::default()
        };
        assert_eq!(
            super::super::show_notification_with(&notification).unwrap(),
            41
        );
        {
            let calls = calls.lock().unwrap();
            let (method, args) = &calls[0];
            assert_eq!(method, "Notify");
            let app = crate::init::app_metadata_property(crate::init::AppMetadata::Name).unwrap();
            assert_eq!(
                args[..5],
                [
                    Value::Str(app),
                    Value::U32(0),
                    Value::Str(String::new()),
                    Value::Str("T".into()),
                    Value::Str(String::new()),
                ]
            );
            let strs =
                |v: &[&str]| Value::Array(v.iter().map(|s| Value::Str((*s).into())).collect());
            assert_eq!(args[5], strs(&["default", "default", "yes", "Yes"]));
            let Value::Array(hints) = &args[6] else {
                panic!("{args:?}");
            };
            assert_eq!(dict_get(hints, "urgency"), Some(&Value::Byte(2)));
            assert_eq!(dict_get(hints, "transient"), Some(&Value::Bool(true)));
            assert_eq!(dict_get(hints, "sound-name"), None);
            let Some(Value::Struct(img)) = dict_get(hints, "image-data") else {
                panic!("{hints:?}");
            };
            assert_eq!(
                img[..6],
                [
                    Value::I32(1),
                    Value::I32(1),
                    Value::I32(image.pitch()),
                    Value::Bool(true),
                    Value::I32(8),
                    Value::I32(4),
                ]
            );
            assert_eq!(
                img[6],
                Value::Array([1, 2, 3, 4].into_iter().map(Value::Byte).collect())
            );
            assert_eq!(args[7], Value::I32(-1));
        }

        // An action of ours, then the same one again (no longer ours).
        let invoke = |id: u32| {
            emit(
                &bus.address,
                NOTIFICATION_CORE_PATH,
                NOTIFICATION_CORE_INTERFACE,
                NOTIFICATION_ACTION_SIGNAL_NAME,
                |w| {
                    w.append(&Arg::U32(id));
                    w.append(&Arg::Str("yes"));
                },
            )
        };
        invoke(41);
        assert_eq!(wait_for_action(), [(41, "yes".to_owned())]);
        invoke(41);
        std::thread::sleep(std::time::Duration::from_millis(50));
        dbus::pump_events();
        assert!(actions_invoked().is_empty());

        // An activation token, then removal.
        let simple = Notification {
            title: Some("U".into()),
            message: Some("body".into()),
            ..Notification::default()
        };
        assert_eq!(super::super::show_notification_with(&simple).unwrap(), 42);
        assert!(calls.lock().unwrap()[1]
            .1
            .contains(&Value::Str("body".into())));
        emit(
            &bus.address,
            NOTIFICATION_CORE_PATH,
            NOTIFICATION_CORE_INTERFACE,
            NOTIFICATION_ACTIVATION_TOKEN_SIGNAL_NAME,
            |w| {
                w.append(&Arg::U32(42));
                w.append(&Arg::Str("tok"));
            },
        );
        assert!(test_bus::wait_for(dbus::pump_events, || with_state(|s| s
            .activation_token
            .is_some())));
        assert_eq!(notification_activation_token().as_deref(), Some("tok"));
        super::super::remove_notification(42).unwrap();
        assert!(super::super::remove_notification(0).is_err());
        assert!(test_bus::wait_for(
            || (),
            || calls.lock().unwrap().len() == 3
        ));
        assert_eq!(
            calls.lock().unwrap()[2],
            ("CloseNotification".to_owned(), vec![Value::U32(42)])
        );

        crate::init::quit();
        reset();
        test_bus::release_session();
        server.stop();
    }

    #[test]
    fn portal_notifications() {
        let _l = crate::test_support::test_lock();
        let Some(bus) = Bus::start() else { return };
        reset();

        // A notification portal (and no org.freedesktop.Notifications).
        let calls: Calls = Arc::default();
        let c2 = calls.clone();
        let portal = peer(&bus.address, NOTIFICATION_PORTAL_NODE, move |_, msg| {
            if msg.is_method_call("org.freedesktop.DBus.Properties", "Get") {
                let mut reply = msg.new_method_return()?;
                reply.writer().variant("u", |v| v.append(&Arg::U32(2)));
                return Some(reply);
            }
            for method in ["AddNotification", "RemoveNotification"] {
                if msg.is_method_call(NOTIFICATION_PORTAL_INTERFACE, method) {
                    c2.lock().unwrap().push((method.to_owned(), msg.args()));
                    return msg.new_method_return();
                }
            }
            None
        });
        test_bus::use_as_session(&bus);
        crate::init::init(crate::init::InitFlags::EVENTS).unwrap();

        let image = Surface::new(2, 2, PixelFormat::RGBA8888).unwrap();
        let notification = Notification {
            title: Some("T".into()),
            message: Some("M".into()),
            image: Some(&image),
            actions: vec![NotificationAction::Button {
                action_id: "yes".into(),
                action_label: "Yes".into(),
            }],
            priority: NotificationPriority::High,
            replaces: Some(1234),
            ..Notification::default()
        };
        assert_eq!(
            super::super::show_notification_with(&notification).unwrap(),
            1234
        );
        let session_id = with_state(|s| s.session_id).to_string();
        {
            let calls = calls.lock().unwrap();
            let (method, args) = &calls[0];
            assert_eq!(method, "AddNotification");
            assert_eq!(args[0], Value::Str("SDL_LocalNotification-1234".into()));
            let Value::Array(dict) = &args[1] else {
                panic!("{args:?}");
            };
            let s = |v: &str| Some(Value::Str(v.into()));
            assert_eq!(dict_get(dict, "title").cloned_str(), s("T"));
            assert_eq!(dict_get(dict, "body").cloned_str(), s("M"));
            assert_eq!(dict_get(dict, "default-action").cloned_str(), s("default"));
            assert_eq!(
                dict_get(dict, "default-action-target"),
                Some(&Value::Str(session_id.clone()))
            );
            assert_eq!(dict_get(dict, "sound").cloned_str(), s("default"));
            assert_eq!(dict_get(dict, "priority").cloned_str(), s("high"));
            assert_eq!(
                dict_get(dict, "display-hint"),
                Some(&Value::Struct(vec![Value::Array(vec![])]))
            );
            let Some(Value::Struct(icon)) = dict_get(dict, "icon") else {
                panic!("{dict:?}");
            };
            assert_eq!(icon[0], Value::Str("file-descriptor".into()));
            assert!(matches!(icon[1].unvariant(), Value::UnixFd(_)), "{icon:?}");
            let Some(Value::Array(buttons)) = dict_get(dict, "buttons") else {
                panic!("{dict:?}");
            };
            let Value::Array(button) = &buttons[0] else {
                panic!("{buttons:?}");
            };
            assert_eq!(dict_get(button, "action").cloned_str(), s("yes"));
            assert_eq!(dict_get(button, "label").cloned_str(), s("Yes"));
            assert_eq!(
                dict_get(button, "target"),
                Some(&Value::Str(session_id.clone()))
            );
        }

        // The portal reports the action with our session as the target.
        let invoke = |target: &str| {
            emit(
                &bus.address,
                NOTIFICATION_PORTAL_PATH,
                NOTIFICATION_PORTAL_INTERFACE,
                NOTIFICATION_ACTION_SIGNAL_NAME,
                |w| {
                    w.append(&Arg::Str("SDL_LocalNotification-1234"));
                    w.append(&Arg::Str("yes"));
                    w.container(b'a', Some("v"), |a| {
                        a.variant("v", |v| v.variant("s", |s| s.append(&Arg::Str(target))))
                    });
                },
            )
        };
        invoke("1");
        invoke(&session_id);
        // (the first digit of the ID is lost, as upstream)
        assert_eq!(wait_for_action(), [(234, "yes".to_owned())]);

        super::super::remove_notification(1234).unwrap();
        assert!(test_bus::wait_for(
            || (),
            || calls.lock().unwrap().len() == 2
        ));
        assert_eq!(
            calls.lock().unwrap()[1],
            (
                "RemoveNotification".to_owned(),
                vec![Value::Str("SDL_LocalNotification-1234".into())]
            )
        );

        crate::init::quit();
        reset();
        test_bus::release_session();
        portal.stop();
    }

    trait ClonedStr {
        fn cloned_str(self) -> Option<Value>;
    }

    impl ClonedStr for Option<&Value> {
        fn cloned_str(self) -> Option<Value> {
            self.and_then(Value::as_str)
                .map(|s| Value::Str(s.to_owned()))
        }
    }
}
