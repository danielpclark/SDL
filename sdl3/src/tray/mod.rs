// Rust translation of src/tray/SDL_tray_utils.c, SDL_tray_utils.h,
// src/tray/dummy/SDL_tray.c and include/SDL3/SDL_tray.h from Simple
// DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! System tray icons and their menus.
//!
//! Only the dummy backend exists so far: creating a tray reports that the
//! operation is unsupported. The bookkeeping the event loop uses (the
//! active tray count, quitting when the last tray goes away) is
//! translated, for the backends (Windows, Cocoa, AppIndicator) that arrive
//! with the platform layer.

use std::sync::atomic::{AtomicI32, Ordering};

use crate::error::{Error, Result};
use crate::video::surface::Surface;

/// Flags that control the creation of system tray entries. Some of these
/// flags are required; exactly one of them must be specified at the time a
/// tray entry is created. Other flags are optional; zero or more of those
/// can be OR'ed together with the required flag.
/// Translation of `SDL_TrayEntryFlags`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct TrayEntryFlags(pub u32);

impl TrayEntryFlags {
    /// Make the entry a simple button. Required.
    pub const BUTTON: TrayEntryFlags = TrayEntryFlags(0x00000001);
    /// Make the entry a checkbox. Required.
    pub const CHECKBOX: TrayEntryFlags = TrayEntryFlags(0x00000002);
    /// Prepare the entry to have a submenu. Required
    pub const SUBMENU: TrayEntryFlags = TrayEntryFlags(0x00000004);
    /// Make the entry disabled. Optional.
    pub const DISABLED: TrayEntryFlags = TrayEntryFlags(0x80000000);
    /// Make the entry checked. This is valid only for checkboxes. Optional.
    pub const CHECKED: TrayEntryFlags = TrayEntryFlags(0x40000000);

    /// Whether all of `other`'s flags are set.
    pub fn contains(self, other: TrayEntryFlags) -> bool {
        self.0 & other.0 == other.0
    }
}

impl std::ops::BitOr for TrayEntryFlags {
    type Output = TrayEntryFlags;
    fn bitor(self, rhs: TrayEntryFlags) -> TrayEntryFlags {
        TrayEntryFlags(self.0 | rhs.0)
    }
}

/// A system tray icon. Translation of `SDL_Tray *`; dropping it is
/// `SDL_DestroyTray()`.
///
/// (No backend can create one yet.)
#[derive(Debug)]
pub struct Tray {
    _private: (),
}

/// A menu of a tray or of a tray entry. Translation of `SDL_TrayMenu *`.
#[derive(Debug)]
pub struct TrayMenu {
    _private: (),
}

/// An entry of a tray menu. Translation of `SDL_TrayEntry *`.
#[derive(Debug)]
pub struct TrayEntry {
    _private: (),
}

impl Tray {
    /// Create an icon to be placed in the operating system's tray, or
    /// equivalent. Translation of `SDL_CreateTray()` (dummy).
    pub fn create(_icon: Option<&Surface<'_>>, _tooltip: Option<&str>) -> Result<Tray> {
        Err(Error::unsupported())
    }

    /// Update the icon. Translation of `SDL_SetTrayIcon()` (dummy).
    pub fn set_icon(&mut self, _icon: Option<&Surface<'_>>) {}

    /// Update the tooltip. Translation of `SDL_SetTrayTooltip()` (dummy).
    pub fn set_tooltip(&mut self, _tooltip: Option<&str>) {}

    /// Create a menu for the tray. Translation of `SDL_CreateTrayMenu()` (dummy).
    pub fn create_menu(&mut self) -> Result<TrayMenu> {
        Err(Error::invalid_param("tray"))
    }

    /// The tray's menu. Translation of `SDL_GetTrayMenu()` (dummy).
    pub fn menu(&self) -> Result<TrayMenu> {
        Err(Error::invalid_param("tray"))
    }
}

impl TrayMenu {
    /// The entries of the menu. Translation of `SDL_GetTrayEntries()` (dummy).
    pub fn entries(&self) -> Result<Vec<TrayEntry>> {
        Err(Error::invalid_param("menu"))
    }

    /// Insert a tray entry at a position (-1 for the end).
    /// Translation of `SDL_InsertTrayEntryAt()` (dummy).
    pub fn insert_entry(
        &mut self,
        _pos: i32,
        _label: Option<&str>,
        _flags: TrayEntryFlags,
    ) -> Result<TrayEntry> {
        Err(Error::invalid_param("menu"))
    }

    /// The entry the menu is a submenu of.
    /// Translation of `SDL_GetTrayMenuParentEntry()` (dummy).
    pub fn parent_entry(&self) -> Result<TrayEntry> {
        Err(Error::invalid_param("menu"))
    }

    /// The tray the menu belongs to.
    /// Translation of `SDL_GetTrayMenuParentTray()` (dummy).
    pub fn parent_tray(&self) -> Result<Tray> {
        Err(Error::invalid_param("menu"))
    }
}

impl TrayEntry {
    /// Create a submenu for the entry.
    /// Translation of `SDL_CreateTraySubmenu()` (dummy).
    pub fn create_submenu(&mut self) -> Result<TrayMenu> {
        Err(Error::invalid_param("entry"))
    }

    /// The entry's submenu. Translation of `SDL_GetTraySubmenu()` (dummy).
    pub fn submenu(&self) -> Option<TrayMenu> {
        None
    }

    /// Remove the entry from its menu. Translation of `SDL_RemoveTrayEntry()` (dummy).
    pub fn remove(self) {}

    /// Set the label. Translation of `SDL_SetTrayEntryLabel()` (dummy).
    pub fn set_label(&mut self, _label: Option<&str>) {}

    /// The label. Translation of `SDL_GetTrayEntryLabel()` (dummy).
    pub fn label(&self) -> Result<Option<String>> {
        Err(Error::invalid_param("entry"))
    }

    /// Check or uncheck a checkbox. Translation of `SDL_SetTrayEntryChecked()` (dummy).
    pub fn set_checked(&mut self, _checked: bool) {}

    /// Whether a checkbox is checked. Translation of `SDL_GetTrayEntryChecked()` (dummy).
    pub fn checked(&self) -> Result<bool> {
        Err(Error::invalid_param("entry"))
    }

    /// Enable or disable the entry. Translation of `SDL_SetTrayEntryEnabled()` (dummy).
    pub fn set_enabled(&mut self, _enabled: bool) {}

    /// Whether the entry is enabled. Translation of `SDL_GetTrayEntryEnabled()` (dummy).
    pub fn enabled(&self) -> Result<bool> {
        Err(Error::invalid_param("entry"))
    }

    /// Set the callback run when the entry is selected.
    /// Translation of `SDL_SetTrayEntryCallback()` (dummy).
    pub fn set_callback(&mut self, _callback: impl FnMut(&TrayEntry) + Send + 'static) {}

    /// Simulate a click on the entry. Translation of `SDL_ClickTrayEntry()` (dummy).
    pub fn click(&mut self) {}

    /// The menu the entry is in. Translation of `SDL_GetTrayEntryParent()` (dummy).
    pub fn parent(&self) -> Result<TrayMenu> {
        Err(Error::invalid_param("entry"))
    }
}

/// Update the trays: called by the event loop, or by the application if it
/// doesn't use one. Translation of `SDL_UpdateTrays()` (dummy).
pub fn update_trays() {}

/// Translation of `active_trays`.
static ACTIVE_TRAYS: AtomicI32 = AtomicI32::new(0);

/// Count a newly created tray. Translation of `SDL_RegisterTray()`.
#[allow(dead_code)] // (used by the platform backends)
pub(crate) fn register_tray() {
    ACTIVE_TRAYS.fetch_add(1, Ordering::AcqRel);
}

/// Count a destroyed tray, and quit if it was the last thing keeping the
/// app alive. Translation of `SDL_UnregisterTray()`.
#[allow(dead_code)] // (used by the platform backends)
pub(crate) fn unregister_tray() {
    if ACTIVE_TRAYS.fetch_sub(1, Ordering::AcqRel) - 1 > 0 {
        return;
    }

    if !crate::hints::get_bool(crate::hints::QUIT_ON_LAST_WINDOW_CLOSE, true) {
        return;
    }

    let toplevel_count =
        crate::events::window::video().map_or(0, |v| v.visible_toplevel_window_count());

    if toplevel_count == 0 {
        crate::events::queue::send_quit();
    }
}

/// Destroy the remaining trays at shutdown. Translation of `SDL_CleanupTrays()`.
pub(crate) fn cleanup_trays() {
    // (trays are owned by the application; the backends drop theirs)
}

/// Whether any trays exist. Translation of `SDL_HasActiveTrays()`.
pub(crate) fn has_active_trays() -> bool {
    ACTIVE_TRAYS.load(Ordering::Acquire) > 0
}

/// The number of trays. Translation of `SDL_GetActiveTrayCount()`.
#[allow(dead_code)] // (used by the platform backends)
pub(crate) fn active_tray_count() -> i32 {
    ACTIVE_TRAYS.load(Ordering::Acquire)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dummy_tray() {
        assert_eq!(
            Tray::create(None, Some("tip")).unwrap_err().to_string(),
            "That operation is not supported"
        );
        assert!(!has_active_trays());
        assert!(
            (TrayEntryFlags::CHECKBOX | TrayEntryFlags::CHECKED).contains(TrayEntryFlags::CHECKED)
        );
    }
}
