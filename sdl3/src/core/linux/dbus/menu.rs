// Rust translation of the dbusmenu layer of src/core/linux/SDL_dbus.c and
// of src/SDL_menu.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The `com.canonical.dbusmenu` export of a menu tree, used by the D-Bus
//! tray (`SDL_DBus_ExportMenu()` and friends).
//!
//! Upstream keeps the menu in an `SDL_ListNode` list of `SDL_MenuItem`s
//! extended to `SDL_DBusMenuItem`s; here a [`Menu`] is a `Vec` of
//! [`MenuItem`]s. The menu's owner implements [`MenuHost`], through which the
//! registered handler reaches the menu and reports activations, instead of
//! the list head and the callbacks stored in the items.

/* Special thanks to the kind Hayden Gray (thag_iceman/A1029384756) from the SDL community for his help! */

use super::{Connection, HandlerResult, Message, Value, Writer};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Weak;

const DBUS_MENU_INTERFACE: &str = "com.canonical.dbusmenu";
pub(crate) const DBUS_MENU_OBJECT_PATH: &str = "/StatusNotifierItem/menu";

/// The flags of [`update_menu`] (`SDL_DBUS_UPDATE_MENU_FLAG_*`).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) struct UpdateMenuFlags(pub(crate) u8);

impl UpdateMenuFlags {
    /// `SDL_DBUS_UPDATE_MENU_FLAGS_NONE`.
    pub(crate) const NONE: UpdateMenuFlags = UpdateMenuFlags(0);
    /// `SDL_DBUS_UPDATE_MENU_FLAG_DO_NOT_REPLACE`.
    pub(crate) const DO_NOT_REPLACE: UpdateMenuFlags = UpdateMenuFlags(1 << 0);
}

const MENU_INTROSPECT: &str = "<?xml version=\"1.0\"?><node name=\"/\"><interface name=\"com.canonical.dbusmenu\"><property name=\"Version\" type=\"u\" access=\"read\"></property><property name=\"TextDirection\" type=\"s\" access=\"read\"></property><property name=\"Status\" type=\"s\" access=\"read\"></property><property name=\"IconThemePath\" type=\"as\" access=\"read\"></property><method name=\"GetLayout\"><arg type=\"i\" name=\"parentId\" direction=\"in\"></arg><arg type=\"i\" name=\"recursionDepth\" direction=\"in\"></arg><arg type=\"as\" name=\"propertyNames\" direction=\"in\"></arg><arg type=\"u\" name=\"revision\" direction=\"out\"></arg><arg type=\"(ia{sv}av)\" name=\"layout\" direction=\"out\"></arg></method><method name=\"GetGroupProperties\"><arg type=\"ai\" name=\"ids\" direction=\"in\"></arg><arg type=\"as\" name=\"propertyNames\" direction=\"in\"></arg><arg type=\"a(ia{sv})\" name=\"properties\" direction=\"out\"></arg></method><method name=\"GetProperty\"><arg type=\"i\" name=\"id\" direction=\"in\"></arg><arg type=\"s\" name=\"name\" direction=\"in\"></arg><arg type=\"v\" name=\"value\" direction=\"out\"></arg></method><method name=\"Event\"><arg type=\"i\" name=\"id\" direction=\"in\"></arg><arg type=\"s\" name=\"eventId\" direction=\"in\"></arg><arg type=\"v\" name=\"data\" direction=\"in\"></arg><arg type=\"u\" name=\"timestamp\" direction=\"in\"></arg></method><method name=\"EventGroup\"><arg type=\"a(isvu)\" name=\"events\" direction=\"in\"></arg><arg type=\"ai\" name=\"idErrors\" direction=\"out\"></arg></method><method name=\"AboutToShow\"><arg type=\"i\" name=\"id\" direction=\"in\"></arg><arg type=\"b\" name=\"needUpdate\" direction=\"out\"></arg></method><method name=\"AboutToShowGroup\"><arg type=\"ai\" name=\"ids\" direction=\"in\"></arg><arg type=\"ai\" name=\"updatesNeeded\" direction=\"out\"></arg><arg type=\"ai\" name=\"idErrors\" direction=\"out\"></arg></method><signal name=\"ItemsPropertiesUpdated\"><arg type=\"a(ia{sv})\" name=\"updatedProps\" direction=\"out\"/><arg type=\"a(ias)\" name=\"removedProps\" direction=\"out\"/></signal><signal name=\"LayoutUpdated\"><arg type=\"u\" name=\"revision\" direction=\"out\"></arg><arg type=\"i\" name=\"parent\" direction=\"out\"></arg></signal><signal name=\"ItemActivationRequested\"><arg type=\"i\" name=\"id\" direction=\"out\"></arg><arg type=\"u\" name=\"timestamp\" direction=\"out\"></arg></signal></interface></node>";

/// The kind of a menu item. Translation of `SDL_MenuItemType`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum MenuItemType {
    Normal,
    Separator,
    Checkbox,
}

/// Translation of `SDL_MenuItemFlags`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) struct MenuItemFlags(pub(crate) u32);

impl MenuItemFlags {
    pub(crate) const NONE: MenuItemFlags = MenuItemFlags(0);
    pub(crate) const DISABLED: MenuItemFlags = MenuItemFlags(1 << 0);
    pub(crate) const CHECKED: MenuItemFlags = MenuItemFlags(1 << 1);
    #[allow(dead_code)] // (part of the upstream enum)
    pub(crate) const BAR_ITEM: MenuItemFlags = MenuItemFlags(1 << 2);

    pub(crate) fn contains(self, other: MenuItemFlags) -> bool {
        self.0 & other.0 == other.0
    }

    pub(crate) fn set(&mut self, other: MenuItemFlags, on: bool) {
        if on {
            self.0 |= other.0;
        } else {
            self.0 &= !other.0;
        }
    }
}

/// A menu: the items in order (upstream's `SDL_ListNode *` of items).
pub(crate) type Menu = Vec<MenuItem>;

/// A menu item. Translation of `SDL_MenuItem` with the fields of
/// `SDL_DBusMenuItem`.
#[derive(Debug)]
pub(crate) struct MenuItem {
    /* Basic properties */
    pub(crate) utf8: Option<String>,
    pub(crate) kind: MenuItemType,
    pub(crate) flags: MenuItemFlags,

    /// Whether the owner wants [`MenuHost::item_activated`] calls for this
    /// item (upstream's `cb` is set).
    pub(crate) has_callback: bool,

    /// The submenu's items, if a submenu was made. Upstream's `sub_menu`
    /// list is NULL until it gets its first item, which is what
    /// [`MenuItem::has_sub_menu`] reports.
    pub(crate) sub_menu: Option<Menu>,

    /// The owner's key for this item (upstream's `udata`).
    pub(crate) udata: u64,

    /* D-Bus fields */
    id: i32,
    revision: u32,
    /// Whether the right click (menu open) handler is registered here
    /// (upstream's `cb`/`cbdata` of `SDL_DBusMenuItem`; it lives on the
    /// first item of the menu).
    open_callback: bool,
    /// Identifies the item, standing in for the list node's address when
    /// upstream compares the exported list head.
    serial: u64,
}

impl MenuItem {
    /// Whether the item has a (non-empty) submenu.
    pub(crate) fn has_sub_menu(&self) -> bool {
        self.sub_menu.as_ref().is_some_and(|m| !m.is_empty())
    }
}

/// What the registered handler needs from the menu's owner.
pub(crate) trait MenuHost: Send + Sync {
    /// Run `f` on the exported menu (not at all if it's gone).
    fn with_menu(&self, f: &mut dyn FnMut(&Connection, &mut Menu));
    /// The item with key `udata` was clicked (upstream's `item->cb`); called
    /// without the menu borrowed.
    fn item_activated(&self, udata: u64);
    /// The menu is about to be shown and an open callback is registered;
    /// called without the menu borrowed.
    fn menu_opened(&self);
}

/// Translation of `MenuGetItemById()`.
fn menu_get_item_by_id(menu: &mut [MenuItem], id: i32) -> Option<&mut MenuItem> {
    for item in menu.iter_mut() {
        if item.id == id {
            return Some(item);
        }
        if item.has_sub_menu() {
            if let Some(found) = item
                .sub_menu
                .as_deref_mut()
                .and_then(|sub| menu_get_item_by_id(sub, id))
            {
                return Some(found);
            }
        }
    }
    None
}

fn label_value(item: &MenuItem) -> &str {
    item.utf8.as_deref().unwrap_or("")
}

fn type_value(item: &MenuItem) -> &'static str {
    if item.kind == MenuItemType::Separator {
        "separator"
    } else {
        "standard"
    }
}

fn toggle_type_value(item: &MenuItem) -> &'static str {
    if item.kind == MenuItemType::Checkbox {
        "checkmark"
    } else {
        ""
    }
}

fn toggle_state_value(item: &MenuItem) -> i32 {
    if item.flags.contains(MenuItemFlags::CHECKED) {
        1
    } else {
        0
    }
}

fn children_display_value(item: &MenuItem) -> &'static str {
    if item.has_sub_menu() {
        "submenu"
    } else {
        ""
    }
}

/// Append one of the known properties to an open `a{sv}`; false if `prop`
/// isn't one (and nothing was appended).
fn append_item_property(dict: &mut Writer<'_>, item: &MenuItem, prop: &str) -> Option<bool> {
    let enabled = !item.flags.contains(MenuItemFlags::DISABLED);
    Some(match prop {
        "label" => dict.dict_entry(prop, "s", |v| v.append(&super::Arg::Str(label_value(item)))),
        "type" => dict.dict_entry(prop, "s", |v| v.append(&super::Arg::Str(type_value(item)))),
        "enabled" => dict.dict_entry(prop, "b", |v| v.append(&super::Arg::Bool(enabled))),
        "visible" => dict.dict_entry(prop, "b", |v| v.append(&super::Arg::Bool(true))),
        "toggle-type" => dict.dict_entry(prop, "s", |v| {
            v.append(&super::Arg::Str(toggle_type_value(item)))
        }),
        "toggle-state" => dict.dict_entry(prop, "i", |v| {
            v.append(&super::Arg::I32(toggle_state_value(item)))
        }),
        "children-display" => dict.dict_entry(prop, "s", |v| {
            v.append(&super::Arg::Str(children_display_value(item)))
        }),
        _ => return None,
    })
}

/// Translation of `MenuAppendItemProperties()`.
fn menu_append_item_properties(dict: &mut Writer<'_>, item: &MenuItem) -> bool {
    [
        "label",
        "type",
        "enabled",
        "visible",
        "toggle-type",
        "toggle-state",
        "children-display",
    ]
    .iter()
    .all(|prop| append_item_property(dict, item, prop) == Some(true))
}

/// Append the `(ia{sv}av)` structure of an item, its children down to
/// `depth` levels. Translation of `MenuAppendItem()`.
fn menu_append_item(w: &mut Writer<'_>, item: &MenuItem, depth: i32) -> bool {
    w.container(b'r', None, |st| {
        st.append(&super::Arg::I32(item.id))
            && st.container(b'a', Some("{sv}"), |dict| {
                menu_append_item_properties(dict, item)
            })
            && st.container(b'a', Some("v"), |children| {
                if item.has_sub_menu() && depth > 0 {
                    for child in item.sub_menu.iter().flatten() {
                        if !children
                            .variant("(ia{sv}av)", |v| menu_append_item(v, child, depth - 1))
                        {
                            return false;
                        }
                    }
                }
                true
            })
    })
}

/// Append the children of a level of GetLayout's reply. (Upstream has this
/// loop twice, for the root and for a parent item.)
fn menu_append_layout_children(
    children: &mut Writer<'_>,
    items: &[MenuItem],
    recursion_depth: i32,
) -> bool {
    items.iter().all(|item| {
        children.variant("(ia{sv}av)", |cvariant| {
            cvariant.container(b'r', None, |item_struct| {
                item_struct.append(&super::Arg::I32(item.id))
                    && item_struct.container(b'a', Some("{sv}"), |item_dict| {
                        menu_append_item_properties(item_dict, item)
                    })
                    && item_struct.container(b'a', Some("v"), |item_children| {
                        if item.has_sub_menu() && recursion_depth != 0 {
                            for child in item.sub_menu.iter().flatten() {
                                if !item_children.variant("(ia{sv}av)", |v| {
                                    menu_append_item(v, child, recursion_depth - 1)
                                }) {
                                    return false;
                                }
                            }
                        }
                        true
                    })
            })
        })
    })
}

/// Translation of `MenuHandleGetLayout()`.
fn menu_handle_get_layout(conn: &Connection, menu: &mut Menu, msg: &Message) -> HandlerResult {
    let args = msg.args();
    let parent_id = match args.first() {
        Some(Value::I32(v)) => *v,
        _ => return HandlerResult::NotYetHandled,
    };
    let mut recursion_depth = match args.get(1) {
        Some(Value::I32(v)) => *v,
        _ => return HandlerResult::NotYetHandled,
    };
    if recursion_depth == -1 {
        recursion_depth = 100;
    }

    let Some(mut reply) = msg.new_method_return() else {
        return HandlerResult::Handled;
    };

    let revision = menu.first().map_or(0, |head| head.revision);

    let mut w = reply.writer();
    w.append(&super::Arg::U32(revision));
    w.container(b'r', None, |st| {
        let root_id = 0;
        st.append(&super::Arg::I32(root_id))
            && st.container(b'a', Some("{sv}"), |dict| {
                dict.dict_entry("children-display", "s", |v| {
                    v.append(&super::Arg::Str("submenu"))
                })
            })
            && st.container(b'a', Some("v"), |children| {
                if parent_id == 0 && !menu.is_empty() {
                    menu_append_layout_children(children, menu, recursion_depth)
                } else if parent_id != 0 {
                    match menu_get_item_by_id(menu, parent_id) {
                        Some(parent) if parent.has_sub_menu() => menu_append_layout_children(
                            children,
                            parent.sub_menu.as_deref().unwrap_or_default(),
                            recursion_depth,
                        ),
                        _ => true,
                    }
                } else {
                    true
                }
            })
    });

    conn.send_no_flush(&reply);
    HandlerResult::Handled
}

/// Toggle a clicked checkbox and report whether its owner wants to hear
/// about it (the part of `MenuHandleEvent()` and `MenuHandleEventGroup()`
/// done with the menu borrowed).
fn menu_click(
    conn: &Connection,
    menu: &mut Menu,
    weak: &Weak<dyn MenuHost>,
    id: i32,
) -> Option<u64> {
    let item = menu_get_item_by_id(menu, id)?;
    let toggled = item.kind == MenuItemType::Checkbox;
    if toggled {
        item.flags.0 ^= MenuItemFlags::CHECKED.0;
    }
    let notify = item.has_callback.then_some(item.udata);
    if toggled {
        update_menu(
            conn,
            menu,
            None,
            weak,
            None,
            UpdateMenuFlags::DO_NOT_REPLACE,
        );
    }
    notify
}

/// Translation of `MenuHandleEvent()`.
fn menu_handle_event(
    host: &dyn MenuHost,
    weak: &Weak<dyn MenuHost>,
    conn: &Connection,
    msg: &Message,
) -> HandlerResult {
    let args = msg.args();
    // (upstream reads the id and the event id without checking their types)
    let (id, event_id) = match args.as_slice() {
        [Value::I32(id), Value::Str(event_id), ..] => (*id, event_id.as_str()),
        _ => (0, ""),
    };

    let clicked = event_id == "clicked";

    if let Some(reply) = msg.new_method_return() {
        conn.send_no_flush(&reply);
    }

    if clicked {
        let mut notify = None;
        host.with_menu(&mut |conn, menu| notify = menu_click(conn, menu, weak, id));
        if let Some(udata) = notify {
            host.item_activated(udata);
        }
    }

    HandlerResult::Handled
}

/// Translation of `MenuHandleEventGroup()`.
fn menu_handle_event_group(
    host: &dyn MenuHost,
    weak: &Weak<dyn MenuHost>,
    conn: &Connection,
    msg: &Message,
) -> HandlerResult {
    if let Some(Value::Array(events)) = msg.args().first() {
        for event in events {
            let Value::Struct(fields) = event else {
                break;
            };
            if let [Value::I32(id), Value::Str(event_id), ..] = fields.as_slice() {
                if event_id == "clicked" {
                    let mut notify = None;
                    host.with_menu(&mut |conn, menu| notify = menu_click(conn, menu, weak, *id));
                    if let Some(udata) = notify {
                        host.item_activated(udata);
                    }
                }
            }
        }
    }

    if let Some(mut reply) = msg.new_method_return() {
        reply.writer().container(b'a', Some("i"), |_| true);
        conn.send_no_flush(&reply);
    }
    HandlerResult::Handled
}

/// Translation of `MenuHandleGetProperty()`.
fn menu_handle_get_property(conn: &Connection, menu: &mut Menu, msg: &Message) -> HandlerResult {
    let args = msg.args();
    let id = match args.first() {
        Some(Value::I32(v)) => *v,
        _ => return HandlerResult::NotYetHandled,
    };
    let property = match args.get(1) {
        Some(Value::Str(v)) => v.as_str(),
        _ => return HandlerResult::NotYetHandled,
    };

    let Some(item) = menu_get_item_by_id(menu, id) else {
        if let Some(error) = msg.new_error("com.canonical.dbusmenu.Error", "Item not found") {
            conn.send_no_flush(&error);
        }
        return HandlerResult::Handled;
    };

    let Some(mut reply) = msg.new_method_return() else {
        return HandlerResult::Handled;
    };
    let mut w = reply.writer();
    let enabled = !item.flags.contains(MenuItemFlags::DISABLED);
    match property {
        "label" => w.variant("s", |v| v.append(&super::Arg::Str(label_value(item)))),
        "enabled" => w.variant("b", |v| v.append(&super::Arg::Bool(enabled))),
        "visible" => w.variant("b", |v| v.append(&super::Arg::Bool(true))),
        "type" => w.variant("s", |v| v.append(&super::Arg::Str(type_value(item)))),
        "toggle-type" => w.variant("s", |v| v.append(&super::Arg::Str(toggle_type_value(item)))),
        "toggle-state" => w.variant("i", |v| {
            v.append(&super::Arg::I32(toggle_state_value(item)))
        }),
        "children-display" => w.variant("s", |v| {
            v.append(&super::Arg::Str(children_display_value(item)))
        }),
        _ => w.variant("s", |v| v.append(&super::Arg::Str(""))),
    };

    conn.send_no_flush(&reply);
    HandlerResult::Handled
}

/// Translation of `MenuHandleGetGroupProperties()`.
fn menu_handle_get_group_properties(
    conn: &Connection,
    menu: &mut Menu,
    msg: &Message,
) -> HandlerResult {
    const FILTER_PROPS_SZ: usize = 32;

    let args = msg.args();
    let ids: Vec<i32> = match args.first() {
        Some(Value::Array(ids)) => ids
            .iter()
            .map_while(|v| match v {
                Value::I32(id) => Some(*id),
                _ => None,
            })
            .collect(),
        _ => return HandlerResult::NotYetHandled,
    };
    let filter_props: Vec<&str> = match args.get(1) {
        Some(Value::Array(props)) => props
            .iter()
            .map_while(|v| match v {
                Value::Str(s) => Some(s.as_str()),
                _ => None,
            })
            .take(FILTER_PROPS_SZ)
            .collect(),
        _ => return HandlerResult::NotYetHandled,
    };

    let Some(mut reply) = msg.new_method_return() else {
        return HandlerResult::Handled;
    };
    reply
        .writer()
        .container(b'a', Some("(ia{sv})"), |reply_array| {
            for &id in &ids {
                let Some(item) = menu_get_item_by_id(menu, id) else {
                    continue;
                };
                let item = &*item;
                reply_array.container(b'r', None, |st| {
                    st.append(&super::Arg::I32(id))
                        && st.container(b'a', Some("{sv}"), |dict| {
                            if filter_props.is_empty() {
                                menu_append_item_properties(dict, item)
                            } else {
                                for prop in &filter_props {
                                    let _ = append_item_property(dict, item, prop);
                                }
                                true
                            }
                        })
                });
            }
            true
        });
    conn.send_no_flush(&reply);
    HandlerResult::Handled
}

/// Translation of `MenuGetMaxItemId()`.
fn menu_get_max_item_id(menu: &[MenuItem]) -> i32 {
    let mut max_id = 0;
    for item in menu {
        if item.id > max_id {
            max_id = item.id;
        }
        if item.has_sub_menu() {
            let sub_max = menu_get_max_item_id(item.sub_menu.as_deref().unwrap_or_default());
            if sub_max > max_id {
                max_id = sub_max;
            }
        }
    }
    max_id
}

/// Translation of `MenuAssignItemIds()`.
fn menu_assign_item_ids(menu: &mut [MenuItem], next_id: &mut i32) {
    for item in menu.iter_mut() {
        if item.id == 0 {
            item.id = *next_id;
            *next_id += 1;
        }
        if item.has_sub_menu() {
            if let Some(sub) = item.sub_menu.as_deref_mut() {
                menu_assign_item_ids(sub, next_id);
            }
        }
    }
}

/// A new item, all fields zeroed. Translation of `SDL_DBus_CreateMenuItem()`.
pub(crate) fn create_menu_item() -> MenuItem {
    static SERIAL: AtomicU64 = AtomicU64::new(1);
    MenuItem {
        utf8: None,
        kind: MenuItemType::Normal,
        flags: MenuItemFlags::NONE,
        has_callback: false,
        sub_menu: None,
        udata: 0,
        id: 0,
        revision: 0,
        open_callback: false,
        serial: SERIAL.fetch_add(1, Ordering::Relaxed),
    }
}

/// The tag a menu is registered with: its first item (upstream registers
/// the list head as the object path's user data).
fn menu_tag(menu: &[MenuItem]) -> u64 {
    menu.first().map_or(0, |head| head.serial)
}

/// Translation of `MenuMessageHandler()`.
fn menu_message_handler(
    host: &dyn MenuHost,
    weak: &Weak<dyn MenuHost>,
    conn: &Connection,
    msg: &Message,
) -> HandlerResult {
    /// What the handler does with the menu borrowed.
    enum Step {
        Done(HandlerResult),
        Event,
        EventGroup,
        AboutToShow { open_callback: bool },
    }

    let mut step = Step::Done(HandlerResult::NotYetHandled);
    host.with_menu(&mut |_, menu| {
        if menu.is_empty() {
            return;
        }

        step = if msg.is_method_call(DBUS_MENU_INTERFACE, "GetLayout") {
            Step::Done(menu_handle_get_layout(conn, menu, msg))
        } else if msg.is_method_call(DBUS_MENU_INTERFACE, "Event") {
            Step::Event
        } else if msg.is_method_call(DBUS_MENU_INTERFACE, "EventGroup") {
            Step::EventGroup
        } else if msg.is_method_call(DBUS_MENU_INTERFACE, "AboutToShow") {
            Step::AboutToShow {
                open_callback: menu[0].open_callback,
            }
        } else if msg.is_method_call(DBUS_MENU_INTERFACE, "AboutToShowGroup") {
            if let Some(mut reply) = msg.new_method_return() {
                let mut w = reply.writer();
                w.container(b'a', Some("i"), |_| true);
                w.container(b'a', Some("i"), |_| true);
                conn.send_no_flush(&reply);
            }
            Step::Done(HandlerResult::Handled)
        } else if msg.is_method_call(DBUS_MENU_INTERFACE, "GetGroupProperties") {
            Step::Done(menu_handle_get_group_properties(conn, menu, msg))
        } else if msg.is_method_call(DBUS_MENU_INTERFACE, "GetProperty") {
            Step::Done(menu_handle_get_property(conn, menu, msg))
        } else if msg.is_method_call("org.freedesktop.DBus.Properties", "Get") {
            Step::Done(menu_handle_properties_get(conn, msg))
        } else if msg.is_method_call("org.freedesktop.DBus.Properties", "GetAll") {
            Step::Done(menu_handle_properties_get_all(conn, msg))
        } else if msg.is_method_call("org.freedesktop.DBus.Introspectable", "Introspect") {
            if let Some(mut reply) = msg.new_method_return() {
                reply.append_args(&[super::Arg::Str(MENU_INTROSPECT)]);
                conn.send_no_flush(&reply);
            }
            Step::Done(HandlerResult::Handled)
        } else {
            Step::Done(HandlerResult::NotYetHandled)
        };
    });

    match step {
        Step::Done(result) => result,
        Step::Event => menu_handle_event(host, weak, conn, msg),
        Step::EventGroup => menu_handle_event_group(host, weak, conn, msg),
        Step::AboutToShow { open_callback } => {
            if open_callback {
                host.menu_opened();
            }

            let need_update = false;
            if let Some(mut reply) = msg.new_method_return() {
                reply.append_args(&[super::Arg::Bool(need_update)]);
                conn.send_no_flush(&reply);
            }
            HandlerResult::Handled
        }
    }
}

/// `org.freedesktop.DBus.Properties.Get` of `MenuMessageHandler()`.
fn menu_handle_properties_get(conn: &Connection, msg: &Message) -> HandlerResult {
    let args = msg.args();
    let interface_name = match args.first() {
        Some(Value::Str(v)) => v.as_str(),
        _ => return HandlerResult::NotYetHandled,
    };
    let property_name = match args.get(1) {
        Some(Value::Str(v)) => v.as_str(),
        _ => return HandlerResult::NotYetHandled,
    };

    if interface_name != DBUS_MENU_INTERFACE {
        return HandlerResult::NotYetHandled;
    }
    let Some(mut reply) = msg.new_method_return() else {
        return HandlerResult::Handled;
    };
    let mut w = reply.writer();
    match property_name {
        "Version" => w.variant("u", |v| v.append(&super::Arg::U32(3))),
        "Status" => w.variant("s", |v| v.append(&super::Arg::Str("normal"))),
        "TextDirection" => w.variant("s", |v| v.append(&super::Arg::Str("ltr"))),
        "IconThemePath" => w.variant("as", |v| v.append_str_array(&[])),
        _ => return HandlerResult::NotYetHandled,
    };
    conn.send_no_flush(&reply);
    HandlerResult::Handled
}

/// `org.freedesktop.DBus.Properties.GetAll` of `MenuMessageHandler()`.
fn menu_handle_properties_get_all(conn: &Connection, msg: &Message) -> HandlerResult {
    let interface_name = match msg.args().into_iter().next() {
        Some(Value::Str(v)) => v,
        _ => return HandlerResult::NotYetHandled,
    };

    if interface_name != DBUS_MENU_INTERFACE {
        return HandlerResult::NotYetHandled;
    }
    let Some(mut reply) = msg.new_method_return() else {
        return HandlerResult::Handled;
    };
    reply.writer().container(b'a', Some("{sv}"), |dict| {
        dict.dict_entry("Version", "u", |v| v.append(&super::Arg::U32(3)))
            && dict.dict_entry("Status", "s", |v| v.append(&super::Arg::Str("normal")))
            && dict.dict_entry("TextDirection", "s", |v| v.append(&super::Arg::Str("ltr")))
            && dict.dict_entry("IconThemePath", "as", |v| v.append_str_array(&[]))
    });
    conn.send_no_flush(&reply);
    HandlerResult::Handled
}

fn register_menu(
    conn: &Connection,
    path: &str,
    menu: &[MenuItem],
    host: &Weak<dyn MenuHost>,
) -> bool {
    let host = host.clone();
    conn.try_register_object_path(path, menu_tag(menu), move |conn, msg| {
        match host.upgrade() {
            Some(strong) => menu_message_handler(&*strong, &host, conn, msg),
            None => HandlerResult::NotYetHandled,
        }
    })
    .is_ok()
}

/// Export `menu` at the dbusmenu object path; the path, or `None` on
/// failure. Translation of `SDL_DBus_ExportMenu()`.
pub(crate) fn export_menu(
    conn: &Connection,
    menu: &mut Menu,
    host: &Weak<dyn MenuHost>,
) -> Option<&'static str> {
    if menu.is_empty() {
        return None;
    }

    let mut next_id = 1;
    menu_assign_item_ids(menu, &mut next_id);

    if let Some(head) = menu.first_mut() {
        head.revision += 1;
    }

    if !register_menu(conn, DBUS_MENU_OBJECT_PATH, menu, host) {
        return None;
    }

    Some(DBUS_MENU_OBJECT_PATH)
}

/// Tell the host about changes to the menu (LayoutUpdated), re-exporting it
/// first unless `flags` says not to; `cb` hears about a re-export (with the
/// new path when the menu wasn't exported before). Translation of
/// `SDL_DBus_UpdateMenu()`.
pub(crate) fn update_menu(
    conn: &Connection,
    menu: &mut Menu,
    path: Option<&str>,
    host: &Weak<dyn MenuHost>,
    mut cb: Option<&mut dyn FnMut(Option<&'static str>)>,
    flags: UpdateMenuFlags,
) {
    // FIXME (upstream): with an empty menu, `revision` is sent in the
    // LayoutUpdated signal without having been set (indeterminate in C);
    // it is 0 here.
    let mut revision: u32 = 0;

    'send_signal: {
        if !menu.is_empty() {
            let mut next_id = menu_get_max_item_id(menu) + 1;
            menu_assign_item_ids(menu, &mut next_id);

            revision = 0;
            if let Some(head) = menu.first_mut() {
                head.revision += 1;
                revision = head.revision;
            }

            if flags.0 & UpdateMenuFlags::DO_NOT_REPLACE.0 != 0 {
                break 'send_signal;
            }
        }

        // REPLACE_MENU:
        if let Some(path) = path {
            // (an unregistered path reads as NULL user data)
            if conn.object_path_tag(path).unwrap_or(0) != menu_tag(menu) {
                conn.unregister_object_path(path);
                register_menu(conn, path, menu, host);
                conn.flush();

                if let Some(cb) = &mut cb {
                    cb(None);
                }
            }
        } else {
            if menu.is_empty() {
                break 'send_signal;
            }

            let mut next_id = menu_get_max_item_id(menu) + 1;
            menu_assign_item_ids(menu, &mut next_id);
            revision = 0;
            if let Some(head) = menu.first_mut() {
                head.revision += 1;
                revision = head.revision;
            }

            register_menu(conn, DBUS_MENU_OBJECT_PATH, menu, host);
            conn.flush();

            if let Some(cb) = &mut cb {
                cb(Some(DBUS_MENU_OBJECT_PATH));
            }
            conn.flush();
        }
    }

    // SEND_SIGNAL:
    let signal = conn.new_signal(
        path.unwrap_or(DBUS_MENU_OBJECT_PATH),
        DBUS_MENU_INTERFACE,
        "LayoutUpdated",
    );
    if let Some(mut signal) = signal {
        let parent = 0;
        signal.append_args(&[super::Arg::U32(revision), super::Arg::I32(parent)]);
        conn.send_no_flush(&signal);
        conn.flush();
    }
}

/// Have [`MenuHost::menu_opened`] called when the menu is about to be
/// shown. Translation of `SDL_DBus_RegisterMenuOpenCallback()`.
pub(crate) fn register_menu_open_callback(menu: &mut Menu) {
    if let Some(head) = menu.first_mut() {
        head.open_callback = true;
    }
}

/// Translation of `SDL_DBus_TransferMenuItemProperties()`.
pub(crate) fn transfer_menu_item_properties(src: &MenuItem, dst: &mut MenuItem) {
    dst.revision = src.revision;
    dst.open_callback = src.open_callback;
}

/// Stop exporting the menu at `path`. Translation of `SDL_DBus_RetractMenu()`.
pub(crate) fn retract_menu(conn: &Connection, path: &mut Option<&'static str>) {
    if let Some(p) = path.take() {
        conn.unregister_object_path(p);
    }
}
