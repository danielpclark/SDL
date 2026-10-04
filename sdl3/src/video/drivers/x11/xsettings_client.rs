// Rust translation of src/video/x11/xsettings-client.c and
// xsettings-client.h from Simple DirectMedia Layer.
// Copyright © 2001, 2007 Red Hat, Inc.
// Copyright 2024 Igalia S.L.
//
// Permission to use, copy, modify, distribute, and sell this software and its
// documentation for any purpose is hereby granted without fee, provided that
// the above copyright notice appear in all copies and that both that
// copyright notice and this permission notice appear in supporting
// documentation, and that the name of Red Hat not be used in advertising or
// publicity pertaining to distribution of the software without specific,
// written prior permission.  Red Hat makes no representations about the
// suitability of this software for any purpose.  It is provided "as is"
// without express or implied warranty.
//
// RED HAT DISCLAIMS ALL WARRANTIES WITH REGARD TO THIS SOFTWARE, INCLUDING ALL
// IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS, IN NO EVENT SHALL RED HAT
// BE LIABLE FOR ANY SPECIAL, INDIRECT OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
// WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN AN ACTION
// OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF OR IN
// CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.
//
// Author:  Owen Taylor, Red Hat, Inc.
//
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! A client of the XSETTINGS protocol (desktop settings such as the DPI,
//! published by the settings manager on a window property).
//!
//! Upstream calls the notify function from inside
//! `xsettings_client_process_event()`; here the notifications are queued
//! on the client and the caller delivers them (see
//! [`XSettingsClient::take_notifications`]) once it no longer holds the
//! client, so the notify function may read the client's settings.

#![allow(dead_code)] // (the list API of xsettings-client.h is kept whole)

use std::ffi::{c_int, c_long, c_uchar, c_ulong};
use std::sync::Arc;

use super::sys::*;
use super::video::intern_atom;
use super::x11dyn::X11Syms;

/// Translation of `XSettingsType`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum XSettingsType {
    Int = 0,
    String = 1,
    Color = 2,
}

/// Translation of `XSettingsResult`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum XSettingsResult {
    Success,
    NoMem,
    Access,
    Failed,
    NoEntry,
    DuplicateEntry,
}

/// Translation of `XSettingsColor`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) struct XSettingsColor {
    pub(crate) red: u16,
    pub(crate) green: u16,
    pub(crate) blue: u16,
    pub(crate) alpha: u16,
}

/// The value of a setting (the `type` and `data` union of
/// `XSettingsSetting`; unknown types keep their number and no data).
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) enum XSettingsValue {
    Int(i32),
    String(String),
    Color(XSettingsColor),
    Unknown(u8),
}

/// Translation of `XSettingsSetting`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct XSettingsSetting {
    pub(crate) name: String,
    pub(crate) data: XSettingsValue,
    pub(crate) last_change_serial: c_ulong,
}

impl XSettingsSetting {
    /// The setting's type (`setting->type`).
    pub(crate) fn type_(&self) -> u8 {
        match self.data {
            XSettingsValue::Int(_) => XSettingsType::Int as u8,
            XSettingsValue::String(_) => XSettingsType::String as u8,
            XSettingsValue::Color(_) => XSettingsType::Color as u8,
            XSettingsValue::Unknown(t) => t,
        }
    }
}

/// Translation of `XSettingsAction`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum XSettingsAction {
    New,
    Changed,
    Deleted,
}

/// A notification the client produced: `XSettingsNotifyFunc`'s arguments.
pub(crate) type XSettingsNotification = (String, XSettingsAction, Option<XSettingsSetting>);

/// Translation of `xsettings_setting_equal()`.
pub(crate) fn xsettings_setting_equal(
    setting_a: &XSettingsSetting,
    setting_b: &XSettingsSetting,
) -> bool {
    if setting_a.type_() != setting_b.type_() {
        return false;
    }

    if setting_a.name != setting_b.name {
        return false;
    }

    match (&setting_a.data, &setting_b.data) {
        (XSettingsValue::Int(a), XSettingsValue::Int(b)) => a == b,
        (XSettingsValue::Color(a), XSettingsValue::Color(b)) => {
            a.red == b.red && a.green == b.green && a.blue == b.blue && a.alpha == b.alpha
        }
        (XSettingsValue::String(a), XSettingsValue::String(b)) => a == b,
        _ => false,
    }
}

/// Translation of `xsettings_list_insert()`: the list is kept sorted by
/// name (`strcmp()` order).
pub(crate) fn xsettings_list_insert(
    list: &mut Vec<XSettingsSetting>,
    setting: XSettingsSetting,
) -> XSettingsResult {
    let mut index = list.len();
    for (i, iter) in list.iter().enumerate() {
        match setting.name.as_bytes().cmp(iter.name.as_bytes()) {
            std::cmp::Ordering::Less => {
                index = i;
                break;
            }
            std::cmp::Ordering::Equal => return XSettingsResult::DuplicateEntry,
            std::cmp::Ordering::Greater => {}
        }
    }

    list.insert(index, setting);

    XSettingsResult::Success
}

/// Translation of `xsettings_list_delete()`.
pub(crate) fn xsettings_list_delete(
    list: &mut Vec<XSettingsSetting>,
    name: &str,
) -> XSettingsResult {
    match list.iter().position(|s| s.name == name) {
        Some(i) => {
            list.remove(i);
            XSettingsResult::Success
        }
        Option::None => XSettingsResult::Failed,
    }
}

/// Translation of `xsettings_list_lookup()`.
pub(crate) fn xsettings_list_lookup<'a>(
    list: &'a [XSettingsSetting],
    name: &str,
) -> Option<&'a XSettingsSetting> {
    list.iter().find(|s| s.name == name)
}

/// Translation of `xsettings_byte_order()`.
pub(crate) fn xsettings_byte_order() -> c_int {
    if cfg!(target_endian = "big") {
        MSBFirst
    } else {
        LSBFirst
    }
}

/// Translation of `XSETTINGS_PAD()`.
const fn xsettings_pad(n: usize, m: usize) -> usize {
    (n + m - 1) & !(m - 1)
}

/// Translation of `XSettingsBuffer`.
struct XSettingsBuffer<'a> {
    byte_order: c_int,
    data: &'a [u8],
    pos: usize,
}

impl XSettingsBuffer<'_> {
    /// Translation of `BYTES_LEFT()` (negative once `pos` ran past the end).
    fn bytes_left(&self) -> isize {
        self.data.len() as isize - self.pos as isize
    }

    /// Translation of `fetch_card16()`.
    fn fetch_card16(&mut self) -> Result<u16, XSettingsResult> {
        if self.bytes_left() < 2 {
            return Err(XSettingsResult::Access);
        }

        let x = u16::from_ne_bytes([self.data[self.pos], self.data[self.pos + 1]]);
        self.pos += 2;

        if self.byte_order == xsettings_byte_order() {
            Ok(x)
        } else {
            Ok(x.rotate_left(8))
        }
    }

    /// Translation of `fetch_ushort()`.
    fn fetch_ushort(&mut self) -> Result<u16, XSettingsResult> {
        self.fetch_card16()
    }

    /// Translation of `fetch_card32()`.
    fn fetch_card32(&mut self) -> Result<u32, XSettingsResult> {
        if self.bytes_left() < 4 {
            return Err(XSettingsResult::Access);
        }

        let p = self.pos;
        let x = u32::from_ne_bytes([
            self.data[p],
            self.data[p + 1],
            self.data[p + 2],
            self.data[p + 3],
        ]);
        self.pos += 4;

        if self.byte_order == xsettings_byte_order() {
            Ok(x)
        } else {
            Ok((x << 24) | ((x & 0xff00) << 8) | ((x & 0xff0000) >> 8) | (x >> 24))
        }
    }

    /// Translation of `fetch_card8()`.
    fn fetch_card8(&mut self) -> Result<u8, XSettingsResult> {
        if self.bytes_left() < 1 {
            return Err(XSettingsResult::Access);
        }

        let x = self.data[self.pos];
        self.pos += 1;

        Ok(x)
    }
}

/// Translation of `parse_settings()`.
fn parse_settings(data: &[u8]) -> Option<Vec<XSettingsSetting>> {
    let mut buffer = XSettingsBuffer {
        byte_order: 0,
        data,
        pos: 0,
    };
    let mut settings = Vec::new();

    let result = (|| -> Result<(), XSettingsResult> {
        let buffer_byte_order = buffer.fetch_card8().map(|b| b as c_int).unwrap_or(0);
        if buffer_byte_order != MSBFirst && buffer_byte_order != LSBFirst {
            eprintln!("Invalid byte order in XSETTINGS property");
            return Err(XSettingsResult::Failed);
        }

        buffer.byte_order = buffer_byte_order;
        buffer.pos += 3;

        let _serial = buffer.fetch_card32()?;

        let n_entries = buffer.fetch_card32()?;

        for _ in 0..n_entries {
            let type_ = buffer.fetch_card8()?;

            buffer.pos += 1;

            let name_len = buffer.fetch_card16()? as usize;

            let pad_len = xsettings_pad(name_len, 4);
            if buffer.bytes_left() < pad_len as isize {
                return Err(XSettingsResult::Access);
            }

            let name = String::from_utf8_lossy(&buffer.data[buffer.pos..buffer.pos + name_len])
                .into_owned();
            buffer.pos += pad_len;

            let last_change_serial = buffer.fetch_card32()? as c_ulong;

            let value = match type_ {
                t if t == XSettingsType::Int as u8 => {
                    XSettingsValue::Int(buffer.fetch_card32()? as i32)
                }
                t if t == XSettingsType::String as u8 => {
                    let v_int = buffer.fetch_card32()?;

                    let pad_len = xsettings_pad(v_int as usize, 4);
                    if v_int.wrapping_add(1) == 0 /* Guard against wrap-around */
                        || buffer.bytes_left() < pad_len as isize
                    {
                        return Err(XSettingsResult::Access);
                    }

                    let s = String::from_utf8_lossy(
                        &buffer.data[buffer.pos..buffer.pos + v_int as usize],
                    )
                    .into_owned();
                    buffer.pos += pad_len;
                    XSettingsValue::String(s)
                }
                t if t == XSettingsType::Color as u8 => XSettingsValue::Color(XSettingsColor {
                    red: buffer.fetch_ushort()?,
                    green: buffer.fetch_ushort()?,
                    blue: buffer.fetch_ushort()?,
                    alpha: buffer.fetch_ushort()?,
                }),
                // Quietly ignore unknown types
                t => XSettingsValue::Unknown(t),
            };

            let setting = XSettingsSetting {
                name,
                data: value,
                last_change_serial,
            };
            let duplicate_name = setting.name.clone();
            let result = xsettings_list_insert(&mut settings, setting);
            if result != XSettingsResult::Success {
                if result == XSettingsResult::DuplicateEntry {
                    eprintln!("Duplicate XSETTINGS entry for '{duplicate_name}'");
                }
                return Err(result);
            }
        }
        Ok(())
    })();

    match result {
        Ok(()) => Some(settings),
        Err(result) => {
            match result {
                XSettingsResult::NoMem => eprintln!("Out of memory reading XSETTINGS property"),
                XSettingsResult::Access => eprintln!("Invalid XSETTINGS property (read off end)"),
                _ => {}
            }
            Option::None
        }
    }
}

/// Translation of `ignore_errors()`.
unsafe extern "C" fn ignore_errors(_display: *mut Display, _event: *mut XErrorEvent) -> c_int {
    True
}

/// Translation of `struct _XSettingsClient` (without the watch and grab
/// functions, which SDL doesn't use: their `NULL` paths are translated).
pub(crate) struct XSettingsClient {
    x: Arc<X11Syms>,
    display: *mut Display,
    screen: c_int,
    /// Whether there is a notify function (`client->notify`).
    notify: bool,

    manager_window: Window,
    manager_atom: Atom,
    selection_atom: Atom,
    xsettings_atom: Atom,

    settings: Vec<XSettingsSetting>,
    /// The `notify()` calls not delivered yet.
    notifications: Vec<XSettingsNotification>,
}

// SAFETY: the display is an Xlib connection (locked by Xlib after
// XInitThreads()).
unsafe impl Send for XSettingsClient {}

impl XSettingsClient {
    /// Translation of `notify_changes()`.
    fn notify_changes(&mut self, old_list: &[XSettingsSetting]) {
        if !self.notify {
            return;
        }

        let mut old_iter = old_list.iter().peekable();
        let mut new_iter = self.settings.iter().peekable();

        while old_iter.peek().is_some() || new_iter.peek().is_some() {
            let cmp = match (old_iter.peek(), new_iter.peek()) {
                (Some(old), Some(new)) => old.name.as_bytes().cmp(new.name.as_bytes()),
                (Some(_), Option::None) => std::cmp::Ordering::Less,
                _ => std::cmp::Ordering::Greater,
            };

            match cmp {
                std::cmp::Ordering::Less => {
                    let old = old_iter.peek().expect("checked above");
                    self.notifications.push((
                        old.name.clone(),
                        XSettingsAction::Deleted,
                        Option::None,
                    ));
                }
                std::cmp::Ordering::Equal => {
                    let (old, new) = (
                        old_iter.peek().expect("checked"),
                        new_iter.peek().expect("checked"),
                    );
                    if !xsettings_setting_equal(old, new) {
                        self.notifications.push((
                            old.name.clone(),
                            XSettingsAction::Changed,
                            Some((*new).clone()),
                        ));
                    }
                }
                std::cmp::Ordering::Greater => {
                    let new = new_iter.peek().expect("checked above");
                    self.notifications.push((
                        new.name.clone(),
                        XSettingsAction::New,
                        Some((*new).clone()),
                    ));
                }
            }

            // FIXME (upstream): both lists advance even when only one side
            // matched (a deleted or new name), so later entries are
            // compared against the wrong neighbors.
            old_iter.next();
            new_iter.next();
        }
    }

    /// Translation of `read_settings()`.
    fn read_settings(&mut self) {
        let old_list = std::mem::take(&mut self.settings);

        if self.manager_window != 0 {
            let x = &self.x;
            let mut type_: Atom = 0;
            let mut format: c_int = 0;
            let mut n_items: c_ulong = 0;
            let mut bytes_after: c_ulong = 0;
            let mut data: *mut c_uchar = std::ptr::null_mut();

            // SAFETY: the display is open; the out-parameters are valid;
            // the property data has `n_items` bytes (format 8) and is freed
            // here.
            unsafe {
                let old_handler = (x.XSetErrorHandler)(Some(ignore_errors));
                let result = (x.XGetWindowProperty)(
                    self.display,
                    self.manager_window,
                    self.xsettings_atom,
                    0,
                    c_long::MAX,
                    False,
                    self.xsettings_atom,
                    &mut type_,
                    &mut format,
                    &mut n_items,
                    &mut bytes_after,
                    &mut data,
                );
                (x.XSetErrorHandler)(old_handler);

                if result == Success && type_ != None {
                    if type_ != self.xsettings_atom {
                        eprint!("Invalid type for XSETTINGS property");
                    } else if format != 8 {
                        eprint!("Invalid format for XSETTINGS property {format}");
                    } else if !data.is_null() {
                        self.settings =
                            parse_settings(std::slice::from_raw_parts(data, n_items as usize))
                                .unwrap_or_default();
                    }

                    if !data.is_null() {
                        (x.XFree)(data.cast());
                    }
                }
            }
        }

        self.notify_changes(&old_list);
    }

    /// Translation of `check_manager_window()`.
    fn check_manager_window(&mut self) {
        let x = &self.x;
        // (no watch function)

        // SAFETY: the display is open; the atoms are valid.
        unsafe {
            (x.XGrabServer)(self.display);

            self.manager_window = (x.XGetSelectionOwner)(self.display, self.selection_atom);
            if self.manager_window != 0 {
                (x.XSelectInput)(
                    self.display,
                    self.manager_window,
                    PropertyChangeMask | StructureNotifyMask,
                );
            }

            (x.XUngrabServer)(self.display);

            (x.XFlush)(self.display);
        }

        self.read_settings();
    }

    /// Translation of `xsettings_client_new()` (with SDL's arguments: no
    /// watch function; `notify` says whether there is a notify function).
    pub(crate) fn new(
        x: Arc<X11Syms>,
        display: *mut Display,
        screen: c_int,
        notify: bool,
    ) -> XSettingsClient {
        let selection_atom = intern_atom(&x, display, &format!("_XSETTINGS_S{screen}"), false);
        let xsettings_atom = intern_atom(&x, display, "_XSETTINGS_SETTINGS", false);
        let manager_atom = intern_atom(&x, display, "MANAGER", false);

        let mut client = XSettingsClient {
            x,
            display,
            screen,
            notify,
            manager_window: None,
            manager_atom,
            selection_atom,
            xsettings_atom,
            settings: Vec::new(),
            notifications: Vec::new(),
        };

        // Select on StructureNotify so we get MANAGER events
        // SAFETY: the display is open and the screen one of its screens.
        unsafe {
            add_events(
                &client.x,
                display,
                RootWindow(display, screen),
                StructureNotifyMask,
            );
        }

        client.check_manager_window();

        client
    }

    /// The notifications produced since the last call (to deliver to the
    /// notify function, in order).
    pub(crate) fn take_notifications(&mut self) -> Vec<XSettingsNotification> {
        std::mem::take(&mut self.notifications)
    }

    /// Translation of `xsettings_client_get_setting()`.
    pub(crate) fn get_setting(&self, name: &str) -> Result<XSettingsSetting, XSettingsResult> {
        xsettings_list_lookup(&self.settings, name)
            .cloned()
            .ok_or(XSettingsResult::NoEntry)
    }

    /// Translation of `xsettings_client_process_event()`.
    pub(crate) fn process_event(&mut self, xev: &XEvent) -> bool {
        /* The checks here will not unlikely cause us to reread
         * the properties from the manager window a number of
         * times when the manager changes from A->B. But manager changes
         * are going to be pretty rare.
         */
        let any = xev.any();
        // SAFETY: the display is open and the screen one of its screens.
        let root = unsafe { RootWindow(self.display, self.screen) };
        if any.window == root {
            let client = xev.client();
            if any.type_ == ClientMessage
                && client.message_type == self.manager_atom
                && client.data.l[1] as Atom == self.selection_atom
            {
                self.check_manager_window();
                return true;
            }
        } else if any.window == self.manager_window {
            if any.type_ == DestroyNotify {
                self.check_manager_window();
                return false;
            } else if any.type_ == PropertyNotify {
                self.read_settings();
                return true;
            }
        }

        false
    }
}

/// Translation of `add_events()`.
///
/// # Safety
///
/// `display` must be open and `window` one of its windows.
unsafe fn add_events(x: &X11Syms, display: *mut Display, window: Window, mask: c_long) {
    // SAFETY: the caller's contract; attr is a valid out-parameter.
    unsafe {
        let mut attr: XWindowAttributes = std::mem::zeroed();
        (x.XGetWindowAttributes)(display, window, &mut attr);
        (x.XSelectInput)(display, window, attr.your_event_mask | mask);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn property(byte_order: u8, entries: &[(u8, &str, &[u8])]) -> Vec<u8> {
        let mut out = vec![byte_order, 0, 0, 0];
        let push32 = |out: &mut Vec<u8>, v: u32| {
            if byte_order == MSBFirst as u8 {
                out.extend_from_slice(&v.to_be_bytes());
            } else {
                out.extend_from_slice(&v.to_le_bytes());
            }
        };
        push32(&mut out, 7); // serial
        push32(&mut out, entries.len() as u32);
        for (t, name, value) in entries {
            out.push(*t);
            out.push(0);
            let len = name.len() as u16;
            if byte_order == MSBFirst as u8 {
                out.extend_from_slice(&len.to_be_bytes());
            } else {
                out.extend_from_slice(&len.to_le_bytes());
            }
            out.extend_from_slice(name.as_bytes());
            out.resize(out.len() + xsettings_pad(name.len(), 4) - name.len(), 0);
            push32(&mut out, 3); // last change serial
            out.extend_from_slice(value);
        }
        out
    }

    #[test]
    fn parses_settings_in_both_byte_orders() {
        for byte_order in [LSBFirst as u8, MSBFirst as u8] {
            let int = |v: u32| {
                if byte_order == MSBFirst as u8 {
                    v.to_be_bytes().to_vec()
                } else {
                    v.to_le_bytes().to_vec()
                }
            };
            let mut s = int(5);
            s.extend_from_slice(b"Adwa\0\0\0\0");
            s.truncate(4 + 8);
            let data = property(
                byte_order,
                &[
                    (0, "Xft/DPI", &int(98304)),
                    (1, "Net/ThemeName", &s),
                    (0, "Gdk/WindowScalingFactor", &int(2)),
                ],
            );
            let settings = parse_settings(&data).unwrap();
            let names: Vec<&str> = settings.iter().map(|s| s.name.as_str()).collect();
            assert_eq!(
                names,
                ["Gdk/WindowScalingFactor", "Net/ThemeName", "Xft/DPI"]
            );
            assert_eq!(settings[2].data, XSettingsValue::Int(98304));
            assert_eq!(settings[2].last_change_serial, 3);
            assert_eq!(
                settings[1].data,
                XSettingsValue::String("Adwa\0".to_owned())
            );
        }
    }

    #[test]
    fn rejects_bad_properties() {
        assert!(parse_settings(&[]).is_none());
        assert!(parse_settings(&[7, 0, 0, 0]).is_none());
        let mut data = property(LSBFirst as u8, &[(0, "A", &[1, 0, 0, 0])]);
        data.truncate(data.len() - 2);
        assert!(parse_settings(&data).is_none());
        let dup = property(
            LSBFirst as u8,
            &[(0, "A", &[1, 0, 0, 0]), (0, "A", &[2, 0, 0, 0])],
        );
        assert!(parse_settings(&dup).is_none());
    }

    #[test]
    fn list_order_and_lookup() {
        let mut list = Vec::new();
        for name in ["b", "a", "c"] {
            let s = XSettingsSetting {
                name: name.to_owned(),
                data: XSettingsValue::Int(1),
                last_change_serial: 0,
            };
            assert_eq!(
                xsettings_list_insert(&mut list, s.clone()),
                XSettingsResult::Success
            );
        }
        assert_eq!(
            list.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
            ["a", "b", "c"]
        );
        assert!(xsettings_list_lookup(&list, "b").is_some());
        assert_eq!(
            xsettings_list_delete(&mut list, "b"),
            XSettingsResult::Success
        );
        assert_eq!(
            xsettings_list_delete(&mut list, "b"),
            XSettingsResult::Failed
        );
    }
}
