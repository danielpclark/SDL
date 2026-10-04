// Rust translation of src/tray/unix/SDL_dbustray.c from Simple DirectMedia
// Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The D-Bus tray: a StatusNotifierItem on a private session bus
//! connection, registered with the StatusNotifierWatcher, with its menu
//! exported through the dbusmenu layer.

/* Special thanks to the kind Hayden Gray (thag_iceman/A1029384756) from the SDL community for his help! */

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};

use super::super::{
    EntryKey, TrayBackend, TrayCallback, TrayClickCallback, TrayEntry, TrayEntryFlags, TrayOptions,
    TrayRef,
};
use super::TrayDriver;
use crate::core::linux::dbus::menu::{
    self, Menu, MenuHost, MenuItem, MenuItemFlags, MenuItemType, UpdateMenuFlags,
};
use crate::core::linux::dbus::{self, Arg, Connection, HandlerResult, Message, Value, Writer};
use crate::error::{Error, Result};
use crate::video::pixels::PixelFormat;
use crate::video::surface::Surface;

const SNI_INTERFACE: &str = "org.kde.StatusNotifierItem";
const SNI_WATCHER_SERVICE: &str = "org.kde.StatusNotifierWatcher";
const SNI_WATCHER_PATH: &str = "/StatusNotifierWatcher";
const SNI_WATCHER_INTERFACE: &str = "org.kde.StatusNotifierWatcher";
const SNI_OBJECT_PATH: &str = "/StatusNotifierItem";
const SNI_INTROSPECT: &str = "<!DOCTYPE node PUBLIC \"-//freedesktop//DTD D-BUS Object Introspection 1.0//EN\" \"http://www.freedesktop.org/standards/dbus/1.0/introspect.dtd\">\r\n<node>\r\n  <interface name=\"org.kde.StatusNotifierItem\">\r\n\r\n    <property name=\"Category\" type=\"s\" access=\"read\"/>\r\n    <property name=\"Id\" type=\"s\" access=\"read\"/>\r\n    <property name=\"Title\" type=\"s\" access=\"read\"/>\r\n    <property name=\"Status\" type=\"s\" access=\"read\"/>\r\n    <property name=\"WindowId\" type=\"i\" access=\"read\"/>\r\n\r\n    <!-- An additional path to add to the theme search path to find the icons specified above. -->\r\n    <property name=\"IconThemePath\" type=\"s\" access=\"read\"/>\r\n    <property name=\"Menu\" type=\"o\" access=\"read\"/>\r\n    <property name=\"ItemIsMenu\" type=\"b\" access=\"read\"/>\r\n\r\n\r\n    <!-- main icon -->\r\n    <!-- names are preferred over pixmaps -->\r\n    <property name=\"IconName\" type=\"s\" access=\"read\"/>\r\n\r\n    <!--struct containing width, height and image data-->\r\n    <property name=\"IconPixmap\" type=\"a(iiay)\" access=\"read\">\r\n      <annotation name=\"org.qtproject.QtDBus.QtTypeName\" value=\"KDbusImageVector\"/>\r\n    </property>\r\n\r\n    <property name=\"OverlayIconName\" type=\"s\" access=\"read\"/>\r\n\r\n    <property name=\"OverlayIconPixmap\" type=\"a(iiay)\" access=\"read\">\r\n      <annotation name=\"org.qtproject.QtDBus.QtTypeName\" value=\"KDbusImageVector\"/>\r\n    </property>\r\n\r\n\r\n    <!-- Requesting attention icon -->\r\n    <property name=\"AttentionIconName\" type=\"s\" access=\"read\"/>\r\n\r\n    <!--same definition as image-->\r\n    <property name=\"AttentionIconPixmap\" type=\"a(iiay)\" access=\"read\">\r\n      <annotation name=\"org.qtproject.QtDBus.QtTypeName\" value=\"KDbusImageVector\"/>\r\n    </property>\r\n\r\n    <property name=\"AttentionMovieName\" type=\"s\" access=\"read\"/>\r\n\r\n\r\n\r\n    <!-- tooltip data -->\r\n\r\n    <!--(iiay) is an image-->\r\n    <property name=\"ToolTip\" type=\"(sa(iiay)ss)\" access=\"read\">\r\n      <annotation name=\"org.qtproject.QtDBus.QtTypeName\" value=\"KDbusToolTipStruct\"/>\r\n    </property>\r\n\r\n    <method name=\"ProvideXdgActivationToken\">\r\n        <arg name=\"token\" type=\"s\" direction=\"in\"/>\r\n    </method>\r\n\r\n    <!-- interaction: the systemtray wants the application to do something -->\r\n    <method name=\"ContextMenu\">\r\n        <!-- we\'re passing the coordinates of the icon, so the app knows where to put the popup window -->\r\n        <arg name=\"x\" type=\"i\" direction=\"in\"/>\r\n        <arg name=\"y\" type=\"i\" direction=\"in\"/>\r\n    </method>\r\n\r\n    <method name=\"Activate\">\r\n        <arg name=\"x\" type=\"i\" direction=\"in\"/>\r\n        <arg name=\"y\" type=\"i\" direction=\"in\"/>\r\n    </method>\r\n\r\n    <method name=\"SecondaryActivate\">\r\n        <arg name=\"x\" type=\"i\" direction=\"in\"/>\r\n        <arg name=\"y\" type=\"i\" direction=\"in\"/>\r\n    </method>\r\n\r\n    <method name=\"Scroll\">\r\n      <arg name=\"delta\" type=\"i\" direction=\"in\"/>\r\n      <arg name=\"orientation\" type=\"s\" direction=\"in\"/>\r\n    </method>\r\n\r\n    <!-- Signals: the client wants to change something in the status-->\r\n    <signal name=\"NewTitle\">\r\n    </signal>\r\n\r\n    <signal name=\"NewIcon\">\r\n    </signal>\r\n\r\n    <signal name=\"NewAttentionIcon\">\r\n    </signal>\r\n\r\n    <signal name=\"NewOverlayIcon\">\r\n    </signal>\r\n\r\n    <signal name=\"NewMenu\">\r\n    </signal>\r\n\r\n    <signal name=\"NewToolTip\">\r\n    </signal>\r\n\r\n    <signal name=\"NewStatus\">\r\n      <arg name=\"status\" type=\"s\"/>\r\n    </signal>\r\n\r\n  </interface>\r\n</node>";

/// The icon, converted to ARGB32 (upstream keeps the converted surface).
#[derive(Debug)]
struct Icon {
    w: i32,
    h: i32,
    pitch: i32,
    pixels: Vec<u8>,
}

/// Translation of `SDL_ConvertSurface(icon, SDL_PIXELFORMAT_ARGB32)`.
fn convert_icon(icon: Option<&Surface<'_>>) -> Option<Icon> {
    let surface = icon?.convert(PixelFormat::ARGB32).ok()?;
    let len = (surface.pitch() * surface.height()).max(0) as usize;
    let pixels = surface.pixels()?.get(..len)?.to_vec();
    Some(Icon {
        w: surface.width(),
        h: surface.height(),
        pitch: surface.pitch(),
        pixels,
    })
}

type SharedCallback = Arc<Mutex<TrayCallback>>;
type SharedClickCallback = Arc<Mutex<TrayClickCallback>>;

/// The tray's menu. Translation of `SDL_TrayMenuDBus` (for the tray's own
/// menu; submenus are the items' `sub_menu` lists).
#[derive(Debug, Default)]
struct TrayMenuDBus {
    menu: Menu,
    menu_path: Option<&'static str>,
}

/// The mutable part of `SDL_TrayDBus`.
#[derive(Default)]
struct TrayState {
    tooltip: Option<String>,
    surface: Option<Icon>,

    l_cb: Option<SharedClickCallback>,
    r_cb: Option<SharedClickCallback>,
    m_cb: Option<SharedClickCallback>,

    /// `tray->menu`.
    menu: Option<TrayMenuDBus>,
    /// The entries' callbacks (upstream's `udata2`/`cb_data` of the items).
    callbacks: HashMap<EntryKey, SharedCallback>,
    next_key: EntryKey,
}

/// A D-Bus tray. Translation of `SDL_TrayDBus`.
pub(super) struct TrayDBus {
    this: Weak<TrayDBus>,
    connection: Connection,
    service_name: String,

    state: Mutex<TrayState>,

    block: AtomicBool,
}

impl TrayDBus {
    fn state(&self) -> MutexGuard<'_, TrayState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn tray_ref(&self) -> TrayRef {
        let inner: Weak<dyn TrayBackend> = self.this.clone();
        TrayRef { inner }
    }

    fn host(&self) -> Weak<dyn MenuHost> {
        self.this.clone()
    }

    /// Run a click callback (without the state borrowed); false without one.
    fn run_click(&self, cb: Option<SharedClickCallback>) -> bool {
        let Some(cb) = cb else {
            return false;
        };
        let tray = self.tray_ref();
        // (a callback doesn't run again from inside itself)
        let result = match cb.try_lock() {
            Ok(mut cb) => cb(&tray),
            Err(_) => false,
        };
        result
    }

    /// Re-export the menu and tell the host (the `SDL_DBus_UpdateMenu()`
    /// calls with `TrayNewMenuOnMenuUpdateCallback`).
    fn update_main_menu(&self, state: &mut TrayState) {
        let host = self.host();
        if let Some(main) = state.menu.as_mut() {
            let path = main.menu_path;
            let menu_path = &mut main.menu_path;
            let conn = &self.connection;
            menu::update_menu(
                conn,
                &mut main.menu,
                path,
                &host,
                Some(&mut |new_path| tray_send_new_menu(conn, menu_path, new_path)),
                UpdateMenuFlags::NONE,
            );
        }
    }
}

/// Send a reply or signal on the tray connection (as upstream, ignoring
/// failures).
fn send(conn: &Connection, msg: Option<Message>) {
    if let Some(msg) = msg {
        conn.send_no_flush(&msg);
    }
}

fn append_string_variant(w: &mut Writer<'_>, value: &str) -> bool {
    w.variant("s", |v| v.append(&Arg::Str(value)))
}

/// The IconPixmap value, `a(iiay)`, inside an open variant.
fn append_icon_pixmap(v: &mut Writer<'_>, icon: &Icon) -> bool {
    v.container(b'a', Some("(iiay)"), |pixmap_array| {
        pixmap_array.container(b'r', None, |pixmap_struct| {
            pixmap_struct.append(&Arg::I32(icon.w))
                && pixmap_struct.append(&Arg::I32(icon.h))
                && pixmap_struct.container(b'a', Some("y"), |bytes| {
                    let len = (icon.pitch * icon.h).max(0) as usize;
                    bytes.append_fixed_bytes(&icon.pixels[..len.min(icon.pixels.len())])
                })
        })
    })
}

/// The ToolTip value, `(sa(iiay)ss)`, inside an open variant.
fn append_tooltip(v: &mut Writer<'_>, tooltip: &str) -> bool {
    let empty = "";
    v.container(b'r', None, |st| {
        st.append(&Arg::Str(empty))
            && st.container(b'a', Some("(iiay)"), |_| true)
            && st.append(&Arg::Str(tooltip))
            && st.append(&Arg::Str(empty))
    })
}

/// Translation of `TrayHandleGetAllProps()`.
fn tray_handle_get_all_props(tray: &TrayDBus, msg: &Message) -> HandlerResult {
    let empty = "";
    let app_id = crate::core::unix::app_id();
    let Some(mut reply) = msg.new_method_return() else {
        return HandlerResult::Handled;
    };

    {
        let state = tray.state();
        let menu_path = state.menu.as_ref().and_then(|m| m.menu_path);
        // FIXME (upstream): the dictionary is only closed when the tray has
        // a tooltip, which leaves the reply malformed otherwise (libdbus
        // refuses to send it with checks enabled); it is always closed here.
        reply.writer().container(b'a', Some("{sv}"), |dict| {
            let mut ok = dict.dict_entry("Category", "s", |v| {
                v.append(&Arg::Str("ApplicationStatus"))
            }) && dict.dict_entry("Id", "s", |v| v.append(&Arg::Str(&app_id)))
                && dict.dict_entry("Title", "s", |v| v.append(&Arg::Str(empty)))
                && dict.dict_entry("Status", "s", |v| v.append(&Arg::Str("Active")))
                && dict.dict_entry("IconName", "s", |v| v.append(&Arg::Str(empty)))
                && dict.dict_entry("WindowId", "i", |v| v.append(&Arg::I32(0)))
                && dict.dict_entry("ItemIsMenu", "b", |v| {
                    v.append(&Arg::Bool(menu_path.is_some()))
                })
                && dict.dict_entry("Menu", "o", |v| {
                    v.append(&Arg::ObjectPath(menu_path.unwrap_or("/NO_DBUSMENU")))
                });

            if let Some(icon) = &state.surface {
                ok =
                    ok && dict.dict_entry("IconPixmap", "a(iiay)", |v| append_icon_pixmap(v, icon));
            }

            if let Some(tooltip) = &state.tooltip {
                ok = ok
                    && dict.dict_entry("ToolTip", "(sa(iiay)ss)", |v| append_tooltip(v, tooltip));
            }
            ok
        });
    }

    send(&tray.connection, Some(reply));
    HandlerResult::Handled
}

/// Translation of `TrayHandleGetProp()`.
fn tray_handle_get_prop(tray: &TrayDBus, msg: &Message) -> HandlerResult {
    let empty = "";
    let args = msg.args();
    // (upstream reads the interface and property without checking them)
    let property = args.get(1).and_then(Value::as_str).unwrap_or_default();

    let Some(mut reply) = msg.new_method_return() else {
        return HandlerResult::Handled;
    };

    let known = {
        let state = tray.state();
        let menu_path = state.menu.as_ref().and_then(|m| m.menu_path);
        let mut w = reply.writer();
        match property {
            "Category" => Some(append_string_variant(&mut w, "ApplicationStatus")),
            "Id" => Some(append_string_variant(&mut w, &crate::core::unix::app_id())),
            "Title" => Some(append_string_variant(&mut w, empty)),
            "Status" => Some(append_string_variant(&mut w, "Active")),
            "IconName" => Some(append_string_variant(&mut w, empty)),
            "ItemIsMenu" => Some(w.variant("b", |v| v.append(&Arg::Bool(menu_path.is_some())))),
            "Menu" => Some(w.variant("o", |v| {
                v.append(&Arg::ObjectPath(menu_path.unwrap_or("/NO_DBUSMENU")))
            })),
            "IconPixmap" if state.surface.is_some() => {
                let icon = state.surface.as_ref().expect("checked");
                Some(w.variant("a(iiay)", |v| append_icon_pixmap(v, icon)))
            }
            "ToolTip" if state.tooltip.is_some() => {
                let tooltip = state.tooltip.as_deref().expect("checked");
                Some(w.variant("(sa(iiay)ss)", |v| append_tooltip(v, tooltip)))
            }
            "WindowId" => Some(w.variant("i", |v| v.append(&Arg::I32(0)))),
            _ => None,
        }
    };

    let reply = match known {
        Some(_) => Some(reply),
        None => msg.new_error(dbus::ERROR_UNKNOWN_PROPERTY, "Unknown property"),
    };
    send(&tray.connection, reply);
    HandlerResult::Handled
}

/// Translation of `TrayMessageHandler()`.
fn tray_message_handler(tray: &TrayDBus, msg: &Message) -> HandlerResult {
    if msg.is_method_call("org.freedesktop.DBus.Properties", "Get") {
        return tray_handle_get_prop(tray, msg);
    } else if msg.is_method_call("org.freedesktop.DBus.Properties", "GetAll") {
        return tray_handle_get_all_props(tray, msg);
    } else if msg.is_method_call("org.freedesktop.DBus.Introspectable", "Introspect") {
        let reply = msg.new_method_return().map(|mut reply| {
            reply.append_args(&[Arg::Str(SNI_INTROSPECT)]);
            reply
        });
        send(&tray.connection, reply);
        return HandlerResult::Handled;
    }

    let cb = if msg.is_method_call(SNI_INTERFACE, "ContextMenu") {
        tray.state().r_cb.clone()
    } else if msg.is_method_call(SNI_INTERFACE, "Activate") {
        tray.state().l_cb.clone()
    } else if msg.is_method_call(SNI_INTERFACE, "SecondaryActivate") {
        tray.state().m_cb.clone()
    } else if msg.is_method_call(SNI_INTERFACE, "Scroll") {
        if let [Value::I32(_delta), Value::Str(_orientation), ..] = msg.args().as_slice() {
            /* Scroll callback support will come later :) */
        }
        None
    } else {
        return HandlerResult::NotYetHandled;
    };

    tray.run_click(cb);

    send(&tray.connection, msg.new_method_return());
    HandlerResult::Handled
}

/// Translation of `CreateTray()`.
pub(super) fn create_tray(
    driver: &mut TrayDriver,
    options: TrayOptions<'_>,
) -> Result<Arc<dyn TrayBackend>> {
    /* Get properties */
    let TrayOptions {
        icon,
        tooltip,
        left_click,
        right_click,
        middle_click,
    } = options;

    /* Connect */
    let connection = Connection::session_private()
        .map_err(|e| Error::new(format!("Unable to create tray: {}", e.message())))?;

    /* Request name */
    driver.count += 1;
    let service_name = if crate::init::sandbox() == crate::init::Sandbox::Flatpak {
        format!("{}.tray{}", crate::core::unix::app_id(), driver.count)
    } else {
        format!(
            "org.kde.StatusNotifierItem-{}-{}",
            std::process::id(),
            driver.count
        )
    };
    let status = connection
        .request_name(&service_name, dbus::NAME_FLAG_REPLACE_EXISTING)
        .map_err(|e| Error::new(format!("Unable to create tray: {}", e.message())))?;
    if status != dbus::REQUEST_NAME_REPLY_PRIMARY_OWNER {
        return Err(Error::new(
            "Unable to create tray: unable to request a unique name!",
        ));
    }

    /* Populate */
    let tray = Arc::new_cyclic(|this| TrayDBus {
        this: this.clone(),
        connection,
        service_name,
        state: Mutex::new(TrayState {
            tooltip,
            surface: convert_icon(icon),
            next_key: 1,
            ..TrayState::default()
        }),
        block: AtomicBool::new(false),
    });

    /* Create object */
    let weak = tray.this.clone();
    tray.connection
        .try_register_object_path(SNI_OBJECT_PATH, 0, move |_, msg| match weak.upgrade() {
            Some(tray) => tray_message_handler(&tray, msg),
            None => HandlerResult::NotYetHandled,
        })
        .map_err(|e| {
            if e.message().is_empty() {
                Error::new("Unable to create tray: unable to register object path!")
            } else {
                Error::new(format!("Unable to create tray: {}", e.message()))
            }
        })?;

    /* Register */
    if !dbus::call_void_method_on_connection(
        &tray.connection,
        SNI_WATCHER_SERVICE,
        SNI_WATCHER_PATH,
        SNI_WATCHER_INTERFACE,
        "RegisterStatusNotifierItem",
        &[Arg::Str(&tray.service_name)],
    ) {
        return Err(Error::new(
            "Unable to create tray: unable to register status notifier item!",
        ));
    }

    /* Icon mouse event callbacks */
    {
        let mut state = tray.state();
        state.l_cb = left_click.map(|cb| Arc::new(Mutex::new(cb)));
        state.r_cb = right_click.map(|cb| Arc::new(Mutex::new(cb)));
        state.m_cb = middle_click.map(|cb| Arc::new(Mutex::new(cb)));
    }

    Ok(tray)
}

/// Translation of `DestroyDriver()`.
pub(super) fn destroy_driver(_driver: TrayDriver) {
    // FIXME (upstream): this closes SDL's shared D-Bus connections, which
    // the rest of SDL (portal dialogs, notifications, the screensaver...)
    // may still be using; they reconnect on next use, but lose their
    // filters.
    dbus::quit();
}

/// Find an item by its key in a menu and its submenus.
fn find_item(menu: &mut [MenuItem], key: EntryKey) -> Option<&mut MenuItem> {
    for item in menu.iter_mut() {
        if item.udata == key {
            return Some(item);
        }
        if let Some(found) = item
            .sub_menu
            .as_deref_mut()
            .and_then(|sub| find_item(sub, key))
        {
            return Some(found);
        }
    }
    None
}

/// The menu holding the item `key`: `Some(None)` for the top level.
fn find_parent(menu: &[MenuItem], key: EntryKey) -> Option<Option<EntryKey>> {
    for item in menu {
        if item.udata == key {
            return Some(None);
        }
        if let Some(sub) = item.sub_menu.as_deref() {
            match find_parent(sub, key) {
                Some(None) => return Some(Some(item.udata)),
                Some(found) => return Some(found),
                None => {}
            }
        }
    }
    None
}

/// The keys of the items in a menu and all its submenus.
fn collect_keys(menu: &[MenuItem], out: &mut Vec<EntryKey>) {
    for item in menu {
        out.push(item.udata);
        if let Some(sub) = item.sub_menu.as_deref() {
            collect_keys(sub, out);
        }
    }
}

/// Where `SDL_ListInsertAtPosition()` puts an item.
///
/// FIXME (upstream): for positions from 2 on, the item lands one place
/// earlier than asked (the cursor stops one node short), and any other
/// negative position than -1 inserts at 1. Position 0 inserts the same item
/// twice (the head insertion doesn't return), which would alias the item
/// here, so it is inserted once.
fn list_insert_index(len: usize, pos: i32) -> usize {
    if pos == -1 || len == 0 {
        return len;
    }
    if pos == 0 {
        return 0;
    }
    let advances = if pos >= 2 { (pos - 2) as usize } else { 0 };
    if advances < len {
        advances + 1
    } else {
        len
    }
}

/// Translation of `TraySendNewMenu()`.
fn tray_send_new_menu(
    conn: &Connection,
    menu_path: &mut Option<&'static str>,
    new_path: Option<&'static str>,
) {
    conn.flush();

    if let Some(mut signal) = conn.new_signal(
        SNI_OBJECT_PATH,
        "org.freedesktop.DBus.Properties",
        "PropertiesChanged",
    ) {
        let iface = SNI_INTERFACE;
        let path = match new_path {
            Some(p) => {
                *menu_path = Some(p);
                p
            }
            None => menu_path.unwrap_or_default(),
        };
        let bool_val = true;
        let mut w = signal.writer();
        let _ = w.append(&Arg::Str(iface))
            && w.container(b'a', Some("{sv}"), |dict| {
                dict.dict_entry("Menu", "o", |v| v.append(&Arg::ObjectPath(path)))
                    && dict.dict_entry("ItemIsMenu", "b", |v| v.append(&Arg::Bool(bool_val)))
            })
            && w.append_str_array(&[]);
        conn.send_no_flush(&signal);
        conn.flush();
    }

    if let Some(signal) = conn.new_signal(SNI_OBJECT_PATH, SNI_INTERFACE, "NewMenu") {
        conn.send_no_flush(&signal);
        conn.flush();
    }

    conn.flush();
}

impl MenuHost for TrayDBus {
    fn with_menu(&self, f: &mut dyn FnMut(&Connection, &mut Menu)) {
        let mut state = self.state();
        if let Some(main) = state.menu.as_mut() {
            f(&self.connection, &mut main.menu);
        }
    }

    /// Translation of `EntryCallback()`.
    fn item_activated(&self, udata: u64) {
        let cb = self.state().callbacks.get(&udata).cloned();
        if let Some(cb) = cb {
            let entry = TrayEntry {
                tray: self.tray_ref(),
                key: udata,
            };
            // (a callback doesn't run again from inside itself)
            if let Ok(mut cb) = cb.try_lock() {
                cb(&entry);
            }
        }
    }

    /// Translation of `TrayRightClickHandler()`.
    fn menu_opened(&self) {
        let cb = self.state().r_cb.clone();
        self.run_click(cb);
    }
}

impl TrayBackend for TrayDBus {
    /// Translation of `UpdateTray()`.
    fn update(&self) {
        // (SDL_ObjectValid(tray, SDL_OBJECT_TYPE_TRAY))
        let valid = || {
            self.this.upgrade().is_some_and(|tray| {
                let tray: Arc<dyn TrayBackend> = tray;
                super::super::is_tray_valid(&tray)
            })
        };
        if !valid() {
            return;
        }

        if self.block.load(Ordering::Acquire) {
            return;
        }

        self.connection.read_write(0);
        while self.connection.dispatch() {
            if !valid() {
                break;
            }

            if self.block.load(Ordering::Acquire) {
                break;
            }

            crate::timer::delay(std::time::Duration::from_micros(10));
        }
    }

    /// Translation of `DestroyTray()` (and `DestroyMenu()`).
    fn destroy(&self) {
        /* Destroy connection */
        self.connection.flush();
        self.block.store(true, Ordering::Release);
        self.connection.close();

        /* Destroy icon and tooltip, the menus and entries */
        let mut state = self.state();
        state.tooltip = None;
        state.surface = None;
        state.menu = None;
        state.callbacks.clear();
    }

    /// Translation of `SetTrayIcon()`.
    fn set_icon(&self, icon: Option<&Surface<'_>>) {
        self.state().surface = convert_icon(icon);

        if let Some(signal) = self
            .connection
            .new_signal(SNI_OBJECT_PATH, SNI_INTERFACE, "NewIcon")
        {
            self.connection.send(&signal);
        }
    }

    /// Translation of `SetTrayTooltip()`.
    fn set_tooltip(&self, text: Option<&str>) {
        self.state().tooltip = text.map(str::to_owned);

        if let Some(signal) =
            self.connection
                .new_signal(SNI_OBJECT_PATH, SNI_INTERFACE, "NewToolTip")
        {
            self.connection.send(&signal);
        }
    }

    /// Translation of `CreateTrayMenu()`.
    fn create_menu(&self) -> Result<()> {
        // (as upstream, a second call replaces the menu)
        self.state().menu = Some(TrayMenuDBus::default());
        Ok(())
    }

    fn has_menu(&self) -> bool {
        self.state().menu.is_some()
    }

    /// Translation of `InsertTrayEntryAt()`.
    fn insert_entry(
        &self,
        parent: Option<EntryKey>,
        pos: i32,
        label: Option<&str>,
        flags: TrayEntryFlags,
    ) -> Result<EntryKey> {
        let mut guard = self.state();
        let state = &mut *guard;
        let key = state.next_key;

        let mut item = menu::create_menu_item();
        // FIXME (upstream): the label pointer is stored without a copy (it
        // dangles once the caller frees it); it is copied here.
        item.utf8 = label.map(str::to_owned);
        item.kind = if label.is_none() {
            MenuItemType::Separator
        } else if flags.contains(TrayEntryFlags::CHECKBOX) {
            MenuItemType::Checkbox
        } else {
            MenuItemType::Normal
        };
        // (as upstream, the DISABLED and CHECKED creation flags are ignored)
        item.flags = MenuItemFlags::NONE;
        item.has_callback = false;
        item.sub_menu = None;
        item.udata = key;

        let main = state
            .menu
            .as_mut()
            .ok_or_else(|| Error::invalid_param("menu"))?;
        let list: &mut Menu = match parent {
            None => &mut main.menu,
            Some(parent_key) => find_item(&mut main.menu, parent_key)
                .and_then(|p| p.sub_menu.as_mut())
                .ok_or_else(|| Error::invalid_param("menu"))?,
        };

        let mut update = !list.is_empty();

        if parent.is_some() {
            update = true;
        }

        let index = list_insert_index(list.len(), pos);
        list.insert(index, item);
        state.next_key += 1;

        // (the parent entry's item holds the submenu list itself)
        // FIXME (upstream): the parent item keeps a pointer to the
        // submenu's first list node, set on each insertion but not on
        // removal, so removing a submenu's first entry leaves it dangling
        // (and an emptied submenu still reads as one); the current list is
        // always used here, and an empty one is no submenu.

        let host = self.host();
        if update {
            self.update_main_menu(state);
        } else if let Some(main) = state.menu.as_mut() {
            main.menu_path = menu::export_menu(&self.connection, &mut main.menu, &host);

            if main.menu_path.is_some() {
                let mut menu_path = main.menu_path;
                tray_send_new_menu(&self.connection, &mut menu_path, None);
                main.menu_path = menu_path;
            }
        }

        if parent.is_none() && state.r_cb.is_some() {
            if let Some(main) = state.menu.as_mut() {
                menu::register_menu_open_callback(&mut main.menu);
            }
        }

        Ok(key)
    }

    /// Translation of `CreateTraySubmenu()`.
    fn create_submenu(&self, entry: EntryKey) -> Result<()> {
        let mut state = self.state();
        let item = state
            .menu
            .as_mut()
            .and_then(|m| find_item(&mut m.menu, entry))
            .ok_or_else(|| Error::invalid_param("entry"))?;
        // (as upstream, a second call replaces the submenu)
        item.sub_menu = Some(Vec::new());
        Ok(())
    }

    /// Translation of `GetTraySubmenu()`.
    fn has_submenu(&self, entry: EntryKey) -> Option<bool> {
        let mut state = self.state();
        let item = state
            .menu
            .as_mut()
            .and_then(|m| find_item(&mut m.menu, entry))?;
        Some(item.sub_menu.is_some())
    }

    /// Translation of `GetTrayEntries()`.
    fn entries(&self, parent: Option<EntryKey>) -> Result<Vec<EntryKey>> {
        let mut state = self.state();
        let main = state
            .menu
            .as_mut()
            .ok_or_else(|| Error::invalid_param("menu"))?;
        let list: &Menu = match parent {
            None => &main.menu,
            Some(key) => find_item(&mut main.menu, key)
                .and_then(|p| p.sub_menu.as_ref())
                .ok_or_else(|| Error::invalid_param("menu"))?,
        };
        Ok(list.iter().map(|item| item.udata).collect())
    }

    fn entry_parent(&self, entry: EntryKey) -> Option<Option<EntryKey>> {
        let state = self.state();
        find_parent(&state.menu.as_ref()?.menu, entry)
    }

    /// Translation of `RemoveTrayEntry()`.
    fn remove_entry(&self, entry: EntryKey) {
        let mut guard = self.state();
        let state = &mut *guard;
        let Some(main) = state.menu.as_mut() else {
            return;
        };
        let Some(parent) = find_parent(&main.menu, entry) else {
            return;
        };

        self.block.store(true, Ordering::Release);

        let old_path = if main.menu.len() == 1 {
            main.menu_path
        } else {
            None
        };

        self.connection.flush();

        let list: &mut Menu = match parent {
            None => &mut main.menu,
            Some(key) => match find_item(&mut main.menu, key).and_then(|p| p.sub_menu.as_mut()) {
                Some(list) => list,
                None => return,
            },
        };
        if let Some(index) = list.iter().position(|item| item.udata == entry) {
            if index == 0 && list.len() > 1 {
                let (head, rest) = list.split_at_mut(1);
                menu::transfer_menu_item_properties(&head[0], &mut rest[0]);
            }
            /* DestroyMenu() of the submenu, then the entry */
            let removed = list.remove(index);
            let mut keys = Vec::new();
            collect_keys(std::slice::from_ref(&removed), &mut keys);
            for key in keys {
                state.callbacks.remove(&key);
            }
        }

        if old_path.is_some() {
            if let Some(main) = state.menu.as_mut() {
                menu::retract_menu(&self.connection, &mut main.menu_path);
            }
        }
        self.update_main_menu(state);
        self.connection.flush();
        self.block.store(false, Ordering::Release);
    }

    /// Translation of `SetTrayEntryLabel()`.
    fn set_label(&self, entry: EntryKey, label: Option<&str>) {
        let mut state = self.state();
        if let Some(item) = state
            .menu
            .as_mut()
            .and_then(|m| find_item(&mut m.menu, entry))
        {
            // FIXME (upstream): the label pointer is stored without a copy;
            // it is copied here.
            item.utf8 = label.map(str::to_owned);
        }
        self.update_main_menu(&mut state);
    }

    /// Translation of `GetTrayEntryLabel()`.
    fn label(&self, entry: EntryKey) -> Option<String> {
        let mut state = self.state();
        let item = state
            .menu
            .as_mut()
            .and_then(|m| find_item(&mut m.menu, entry))?;
        item.utf8.clone()
    }

    /// Translation of `SetTrayEntryChecked()`.
    fn set_checked(&self, entry: EntryKey, val: bool) {
        let mut state = self.state();
        if let Some(item) = state
            .menu
            .as_mut()
            .and_then(|m| find_item(&mut m.menu, entry))
        {
            item.flags.set(MenuItemFlags::CHECKED, val);
        }
        self.update_main_menu(&mut state);
    }

    /// Translation of `GetTrayEntryChecked()`.
    fn checked(&self, entry: EntryKey) -> bool {
        let mut state = self.state();
        state
            .menu
            .as_mut()
            .and_then(|m| find_item(&mut m.menu, entry))
            .is_some_and(|item| item.flags.contains(MenuItemFlags::CHECKED))
    }

    /// Translation of `SetTrayEntryEnabled()`.
    fn set_enabled(&self, entry: EntryKey, val: bool) {
        let mut state = self.state();
        if let Some(item) = state
            .menu
            .as_mut()
            .and_then(|m| find_item(&mut m.menu, entry))
        {
            item.flags.set(MenuItemFlags::DISABLED, !val);
        }
        self.update_main_menu(&mut state);
    }

    /// Translation of `GetTrayEntryEnabled()`.
    fn enabled(&self, entry: EntryKey) -> bool {
        let mut state = self.state();
        state
            .menu
            .as_mut()
            .and_then(|m| find_item(&mut m.menu, entry))
            .is_some_and(|item| !item.flags.contains(MenuItemFlags::DISABLED))
    }

    /// Translation of `SetTrayEntryCallback()`.
    fn set_callback(&self, entry: EntryKey, callback: TrayCallback) {
        let mut state = self.state();
        let Some(item) = state
            .menu
            .as_mut()
            .and_then(|m| find_item(&mut m.menu, entry))
        else {
            return;
        };
        item.has_callback = true;
        state
            .callbacks
            .insert(entry, Arc::new(Mutex::new(callback)));

        self.update_main_menu(&mut state);
    }

    /// Translation of `ClickTrayEntry()`.
    fn click(&self, entry: EntryKey) {
        {
            let mut state = self.state();
            let is_checkbox = state
                .menu
                .as_mut()
                .and_then(|m| find_item(&mut m.menu, entry))
                .map(|item| {
                    if item.kind == MenuItemType::Checkbox {
                        item.flags.0 ^= MenuItemFlags::CHECKED.0;
                        true
                    } else {
                        false
                    }
                });
            if is_checkbox == Some(true) {
                self.update_main_menu(&mut state);
            }
        }
        // FIXME (upstream): the callback is called even when none is set
        // (a NULL function pointer call); it is skipped here.
        self.item_activated(entry);
    }
}

/// Translation of `SDL_Tray_CreateDBusDriver()`.
pub(super) fn create_dbus_driver() -> Result<TrayDriver> {
    /* Init DBus and get context */
    dbus::init();
    let Some(_ctx) = dbus::context() else {
        return Err(Error::new("Unable to create tray: D-Bus is not available"));
    };

    /* SNI support detection */
    let mut sni_supported = false;

    if let Some(reply) = dbus::call_method(
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
        "ListNames",
        &[],
    ) {
        if let Some(Value::Array(paths)) = reply.args().first() {
            let watcher_found = paths
                .iter()
                .any(|p| p.as_str() == Some(SNI_WATCHER_SERVICE));

            if watcher_found {
                let host_registered = dbus::query_property(
                    SNI_WATCHER_SERVICE,
                    SNI_WATCHER_PATH,
                    SNI_WATCHER_INTERFACE,
                    "IsStatusNotifierHostRegistered",
                );
                if host_registered == Some(Value::Bool(true)) {
                    sni_supported = true;
                }
            }
        }
    }

    if !sni_supported {
        // FIXME (upstream): this closes SDL's shared D-Bus connections too
        // (see destroy_driver).
        dbus::quit();
        return Err(Error::new("Unable to create tray: no SNI support!"));
    }

    /* Populate */
    Ok(TrayDriver {
        name: "dbus",
        count: 0,
    })
}

#[cfg(test)]
mod tests {
    use super::super::super::{Tray, TrayEntryFlags, TrayOptions};
    use super::*;
    use crate::core::linux::dbus::test_bus::{self, peer, Bus};
    use std::sync::atomic::AtomicU32;

    #[test]
    fn insert_positions() {
        // (upstream's list insertion, off by one from 2 on)
        assert_eq!(list_insert_index(0, 5), 0);
        assert_eq!(list_insert_index(3, -1), 3);
        assert_eq!(list_insert_index(3, 0), 0);
        assert_eq!(list_insert_index(3, 1), 1);
        assert_eq!(list_insert_index(3, 2), 1);
        assert_eq!(list_insert_index(3, 3), 2);
        assert_eq!(list_insert_index(3, 4), 3);
        assert_eq!(list_insert_index(3, 9), 3);
        assert_eq!(list_insert_index(3, -5), 1);
    }

    /// Run `host` on another thread while this one updates the trays.
    fn as_host<T: Send>(address: &str, host: impl FnOnce(&Connection) -> T + Send) -> T {
        std::thread::scope(|s| {
            let t = s.spawn(|| {
                let conn = Connection::open_address(address).unwrap();
                host(&conn)
            });
            while !t.is_finished() {
                crate::tray::update_trays();
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            t.join().unwrap()
        })
    }

    fn call(
        conn: &Connection,
        dest: &str,
        path: &str,
        iface: &str,
        method: &str,
        args: &[Arg<'_>],
    ) -> Result<Vec<Value>> {
        let mut msg = conn.new_method_call(dest, path, iface, method).unwrap();
        assert!(msg.append_args(args));
        Ok(conn.send_with_reply_and_block(&msg, 5000)?.args())
    }

    /// (id, label, children) of a GetLayout node.
    fn node(v: &Value) -> (i32, String, &[Value]) {
        let Value::Struct(fields) = v.unvariant() else {
            panic!("{v:?}");
        };
        let [Value::I32(id), Value::Array(props), Value::Array(children)] = fields.as_slice()
        else {
            panic!("{fields:?}");
        };
        let label = props
            .iter()
            .find_map(|p| match p {
                Value::DictEntry(k, v) if k.as_str() == Some("label") => {
                    v.unvariant().as_str().map(str::to_owned)
                }
                _ => None,
            })
            .unwrap_or_default();
        (*id, label, children.as_slice())
    }

    fn dict_get<'v>(dict: &'v [Value], key: &str) -> Option<&'v Value> {
        dict.iter().find_map(|e| match e {
            Value::DictEntry(k, v) if k.as_str() == Some(key) => Some(v.unvariant()),
            _ => None,
        })
    }

    #[test]
    fn status_notifier_item_and_menu() {
        let _l = crate::test_support::test_lock();
        let Some(bus) = Bus::start() else { return };

        // A StatusNotifierWatcher with a host registered.
        let items = Arc::new(Mutex::new(Vec::new()));
        let i2 = items.clone();
        let watcher = peer(&bus.address, SNI_WATCHER_SERVICE, move |_, msg| {
            if msg.is_method_call("org.freedesktop.DBus.Properties", "Get") {
                let mut reply = msg.new_method_return()?;
                reply.writer().variant("b", |v| v.append(&Arg::Bool(true)));
                return Some(reply);
            }
            if msg.is_method_call(SNI_WATCHER_INTERFACE, "RegisterStatusNotifierItem") {
                if let Some(Value::Str(name)) = msg.args().first() {
                    i2.lock().unwrap().push(name.clone());
                }
                return msg.new_method_return();
            }
            None
        });
        test_bus::use_as_session(&bus);

        let mut icon = Surface::new(2, 2, PixelFormat::ARGB32).unwrap();
        let pitch = icon.pitch() as usize;
        icon.pixels_mut().unwrap()[..4].copy_from_slice(&[1, 2, 3, 4]);
        let clicks = Arc::new(AtomicU32::new(0));
        let c2 = clicks.clone();
        let opened = Arc::new(AtomicU32::new(0));
        let o2 = opened.clone();
        let backend = super::super::create_tray_on_any_thread(TrayOptions {
            icon: Some(&icon),
            tooltip: Some("tip".into()),
            left_click: Some(Box::new(move |_| {
                c2.fetch_add(1, Ordering::SeqCst);
                true
            })),
            right_click: Some(Box::new(move |_| {
                o2.fetch_add(1, Ordering::SeqCst);
                false
            })),
            middle_click: None,
        })
        .unwrap();
        let tray = Tray::from_backend(backend);
        assert!(crate::tray::has_active_trays());

        let service = format!("org.kde.StatusNotifierItem-{}-1", std::process::id());
        assert!(test_bus::wait_for(
            || (),
            || items.lock().unwrap().len() == 1
        ));
        assert_eq!(items.lock().unwrap()[0], service);

        let sni = |conn: &Connection, method: &str, args: &[Arg<'_>]| {
            call(conn, &service, SNI_OBJECT_PATH, SNI_INTERFACE, method, args)
        };
        let props = |conn: &Connection, method: &str, args: &[Arg<'_>]| {
            call(
                conn,
                &service,
                SNI_OBJECT_PATH,
                "org.freedesktop.DBus.Properties",
                method,
                args,
            )
        };

        // The item's properties, a property, an unknown one, a click.
        let (all, tooltip, unknown) = as_host(&bus.address, |conn| {
            let all = props(conn, "GetAll", &[Arg::Str(SNI_INTERFACE)]).unwrap();
            let tooltip =
                props(conn, "Get", &[Arg::Str(SNI_INTERFACE), Arg::Str("ToolTip")]).unwrap();
            let unknown =
                props(conn, "Get", &[Arg::Str(SNI_INTERFACE), Arg::Str("Nope")]).unwrap_err();
            sni(conn, "Activate", &[Arg::I32(0), Arg::I32(0)]).unwrap();
            (all, tooltip, unknown.message().to_owned())
        });
        let [Value::Array(dict)] = all.as_slice() else {
            panic!("{all:?}");
        };
        assert_eq!(
            dict_get(dict, "Category").and_then(Value::as_str),
            Some("ApplicationStatus")
        );
        assert_eq!(
            dict_get(dict, "Id").and_then(Value::as_str),
            Some(crate::core::unix::app_id().as_str())
        );
        assert_eq!(dict_get(dict, "ItemIsMenu"), Some(&Value::Bool(false)));
        assert_eq!(
            dict_get(dict, "Menu").and_then(Value::as_str),
            Some("/NO_DBUSMENU")
        );
        let Some(Value::Array(pixmaps)) = dict_get(dict, "IconPixmap") else {
            panic!("{dict:?}");
        };
        let Value::Struct(pixmap) = &pixmaps[0] else {
            panic!("{pixmaps:?}");
        };
        assert_eq!(pixmap[0], Value::I32(2));
        assert_eq!(pixmap[1], Value::I32(2));
        let Value::Array(bytes) = &pixmap[2] else {
            panic!("{pixmap:?}");
        };
        assert_eq!(bytes.len(), pitch * 2);
        assert_eq!(
            bytes[..4],
            [
                Value::Byte(1),
                Value::Byte(2),
                Value::Byte(3),
                Value::Byte(4)
            ]
        );
        let tooltip_value = Value::Struct(vec![
            Value::Str(String::new()),
            Value::Array(vec![]),
            Value::Str("tip".into()),
            Value::Str(String::new()),
        ]);
        assert_eq!(dict_get(dict, "ToolTip"), Some(&tooltip_value));
        assert_eq!(tooltip, vec![Value::Variant(Box::new(tooltip_value))]);
        assert!(
            unknown.starts_with(dbus::ERROR_UNKNOWN_PROPERTY),
            "{unknown}"
        );
        assert_eq!(clicks.load(Ordering::SeqCst), 1);

        // A menu: A, a separator, a checkbox C, S with the submenu S1.
        let menu = tray.create_menu().unwrap();
        assert_eq!(tray.menu().unwrap(), menu);
        let a = menu
            .insert_entry(-1, Some("A"), TrayEntryFlags::BUTTON)
            .unwrap();
        let _sep = menu.insert_entry(-1, None, TrayEntryFlags::BUTTON).unwrap();
        let c = menu
            .insert_entry(-1, Some("C"), TrayEntryFlags::CHECKBOX)
            .unwrap();
        let s = menu
            .insert_entry(-1, Some("S"), TrayEntryFlags::SUBMENU)
            .unwrap();
        let sub = s.create_submenu().unwrap();
        assert_eq!(s.submenu().unwrap(), sub);
        assert_eq!(sub.parent_entry().unwrap(), s);
        assert_eq!(sub.parent_tray(), tray.handle());
        let s1 = sub
            .insert_entry(-1, Some("S1"), TrayEntryFlags::BUTTON)
            .unwrap();
        assert_eq!(s1.parent().unwrap(), sub);
        assert_eq!(a.parent().unwrap(), menu);
        assert_eq!(
            menu.entries().unwrap(),
            vec![a.clone(), _sep.clone(), c.clone(), s.clone()]
        );
        assert_eq!(a.label().unwrap().as_deref(), Some("A"));
        assert_eq!(_sep.label().unwrap(), None);

        let activated = Arc::new(Mutex::new(Vec::new()));
        for e in [&a, &c, &s1] {
            let act = activated.clone();
            e.set_callback(move |entry| {
                act.lock()
                    .unwrap()
                    .push(entry.label().unwrap().unwrap_or_default());
            });
        }

        let mpath = menu::DBUS_MENU_OBJECT_PATH;
        let dbusmenu = |conn: &Connection, method: &str, args: &[Arg<'_>]| {
            call(
                conn,
                &service,
                mpath,
                "com.canonical.dbusmenu",
                method,
                args,
            )
        };
        let (menu_prop, layout) = as_host(&bus.address, |conn| {
            let menu_prop =
                props(conn, "Get", &[Arg::Str(SNI_INTERFACE), Arg::Str("Menu")]).unwrap();
            let layout = dbusmenu(conn, "GetLayout", &[Arg::I32(0), Arg::I32(-1)]).unwrap();
            (menu_prop, layout)
        });
        assert_eq!(
            menu_prop,
            vec![Value::Variant(Box::new(Value::ObjectPath(mpath.into())))]
        );
        let [Value::U32(revision), root] = layout.as_slice() else {
            panic!("{layout:?}");
        };
        assert!(*revision > 0);
        let (root_id, _, top) = node(root);
        assert_eq!(root_id, 0);
        let top: Vec<_> = top.iter().map(node).collect();
        let labels: Vec<&str> = top.iter().map(|n| n.1.as_str()).collect();
        assert_eq!(labels, ["A", "", "C", "S"]);
        let (s_id, _, s_children) = &top[3];
        let s1_node = node(&s_children[0]);
        assert_eq!(s1_node.1, "S1");
        let (a_id, c_id, s1_id) = (top[0].0, top[2].0, s1_node.0);

        // Clicks, properties, the menu opening.
        let (c_label, group, sub_layout) = as_host(&bus.address, |conn| {
            for id in [c_id, a_id, s1_id] {
                let mut msg = conn
                    .new_method_call(&service, mpath, "com.canonical.dbusmenu", "Event")
                    .unwrap();
                let mut w = msg.writer();
                w.append(&Arg::I32(id));
                w.append(&Arg::Str("clicked"));
                w.variant("i", |v| v.append(&Arg::I32(0)));
                w.append(&Arg::U32(0));
                conn.send_with_reply_and_block(&msg, 5000).unwrap();
            }
            let c_label =
                dbusmenu(conn, "GetProperty", &[Arg::I32(c_id), Arg::Str("label")]).unwrap();
            let mut msg = conn
                .new_method_call(
                    &service,
                    mpath,
                    "com.canonical.dbusmenu",
                    "GetGroupProperties",
                )
                .unwrap();
            let mut w = msg.writer();
            w.append_i32_array(&[c_id, 9999]);
            w.append_str_array(&["toggle-state", "label"]);
            let group = conn.send_with_reply_and_block(&msg, 5000).unwrap().args();
            dbusmenu(conn, "AboutToShow", &[Arg::I32(0)]).unwrap();
            let sub_layout = dbusmenu(conn, "GetLayout", &[Arg::I32(*s_id), Arg::I32(1)]).unwrap();
            (c_label, group, sub_layout)
        });
        assert_eq!(*activated.lock().unwrap(), ["C", "A", "S1"]);
        assert!(c.checked().unwrap(), "a clicked checkbox toggles");
        assert!(!a.checked().unwrap());
        assert_eq!(
            c_label,
            vec![Value::Variant(Box::new(Value::Str("C".into())))]
        );
        let entry = |k: &str, v: Value| {
            Value::DictEntry(
                Box::new(Value::Str(k.into())),
                Box::new(Value::Variant(Box::new(v))),
            )
        };
        assert_eq!(
            group,
            vec![Value::Array(vec![Value::Struct(vec![
                Value::I32(c_id),
                Value::Array(vec![
                    entry("toggle-state", Value::I32(1)),
                    entry("label", Value::Str("C".into()))
                ]),
            ])])]
        );
        assert_eq!(
            opened.load(Ordering::SeqCst),
            1,
            "the right click callback runs"
        );
        let (_, _, sub_children) = node(&sub_layout[1]);
        assert_eq!(node(&sub_children[0]).1, "S1");

        // Changing and removing entries.
        a.set_label(Some("A2"));
        assert_eq!(a.label().unwrap().as_deref(), Some("A2"));
        c.set_enabled(false);
        assert!(!c.enabled().unwrap());
        c.set_checked(false);
        assert!(!c.checked().unwrap());
        c.click(); // (does nothing, as upstream)
        assert!(!c.checked().unwrap());
        s.clone().remove();
        assert!(s1.label().is_err(), "removed with its parent");
        assert!(s.submenu().is_none());
        let first = menu
            .insert_entry(0, Some("First"), TrayEntryFlags::BUTTON)
            .unwrap();
        assert_eq!(
            menu.entries().unwrap(),
            vec![first, a.clone(), _sep, c.clone()]
        );
        let layout = as_host(&bus.address, |conn| {
            dbusmenu(conn, "GetLayout", &[Arg::I32(0), Arg::I32(-1)]).unwrap()
        });
        let (_, _, top) = node(&layout[1]);
        let labels: Vec<String> = top.iter().map(|n| node(n).1).collect();
        assert_eq!(labels, ["First", "A2", "", "C"]);

        tray.set_tooltip(None);
        let gone = as_host(&bus.address, |conn| {
            props(conn, "Get", &[Arg::Str(SNI_INTERFACE), Arg::Str("ToolTip")]).unwrap_err()
        });
        assert!(error_name_is(&gone, dbus::ERROR_UNKNOWN_PROPERTY));

        let handle = tray.handle();
        drop(tray);
        assert!(!crate::tray::has_active_trays());
        assert!(handle.menu().is_none());
        assert!(a.label().is_err());

        test_bus::release_session();
        watcher.stop();
    }

    fn error_name_is(e: &Error, name: &str) -> bool {
        e.message().starts_with(name)
    }

    #[test]
    fn no_status_notifier_watcher() {
        let _l = crate::test_support::test_lock();
        let Some(bus) = Bus::start() else { return };
        test_bus::use_as_session(&bus);
        let Err(err) = super::super::create_tray_on_any_thread(TrayOptions::default()) else {
            panic!("no watcher on the bus");
        };
        assert_eq!(err.to_string(), "Unable to create tray: no SNI support!");
        assert!(!crate::tray::has_active_trays());
        test_bus::release_session();
    }
}
