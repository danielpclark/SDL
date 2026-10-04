// Rust translation of src/tray/SDL_tray_utils.c, SDL_tray_utils.h,
// src/tray/dummy/SDL_tray.c and include/SDL3/SDL_tray.h from Simple
// DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! System tray icons and their menus.
//!
//! On Unix systems other than Apple's and Android, trays are
//! StatusNotifierItems over D-Bus with a dbusmenu menu (see `unix/`);
//! elsewhere the dummy backend reports that the operation is unsupported.
//!
//! A [`Tray`] owns its icon: dropping it is `SDL_DestroyTray()`. Menus and
//! entries are handles ([`TrayMenu`], [`TrayEntry`], and [`TrayRef`] for
//! the tray itself) that report an invalid parameter once what they refer
//! to is gone, where upstream's pointers would dangle. Callbacks are
//! closures instead of function pointers with userdata.

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex, Weak};

use crate::error::{Error, Result};
use crate::video::surface::Surface;

#[cfg(all(unix, not(target_vendor = "apple"), not(target_os = "android")))]
mod unix;

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

/// A callback run when a tray entry is selected. Translation of
/// `SDL_TrayCallback` (the userdata is whatever the closure captures).
pub type TrayCallback = Box<dyn FnMut(&TrayEntry) + Send>;

/// A callback run when the tray icon is clicked; it returns whether to show
/// the tray menu afterwards (only used for left and right clicks).
/// Translation of `SDL_TrayClickCallback`.
pub type TrayClickCallback = Box<dyn FnMut(&TrayRef) -> bool + Send>;

/// The options of a new tray. Translation of the `SDL_PROP_TRAY_CREATE_*`
/// properties of `SDL_CreateTrayWithProperties()` (the userdata pointer is
/// whatever the callbacks capture).
#[derive(Default)]
pub struct TrayOptions<'a> {
    /// The icon (`SDL_PROP_TRAY_CREATE_ICON_POINTER`).
    pub icon: Option<&'a Surface<'a>>,
    /// The tooltip, where supported (`SDL_PROP_TRAY_CREATE_TOOLTIP_STRING`).
    pub tooltip: Option<String>,
    /// Run on a left click, where supported
    /// (`SDL_PROP_TRAY_CREATE_LEFTCLICK_CALLBACK_POINTER`).
    pub left_click: Option<TrayClickCallback>,
    /// Run on a right click, where supported
    /// (`SDL_PROP_TRAY_CREATE_RIGHTCLICK_CALLBACK_POINTER`).
    pub right_click: Option<TrayClickCallback>,
    /// Run on a middle click, where supported
    /// (`SDL_PROP_TRAY_CREATE_MIDDLECLICK_CALLBACK_POINTER`).
    pub middle_click: Option<TrayClickCallback>,
}

impl std::fmt::Debug for TrayOptions<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TrayOptions")
            .field("icon", &self.icon.is_some())
            .field("tooltip", &self.tooltip)
            .finish_non_exhaustive()
    }
}

/// The key of an entry inside its tray's backend.
pub(crate) type EntryKey = u64;

/// What a tray backend implements (upstream's `SDL_TrayDriver` functions
/// that take a tray, menu or entry). Menus are named by their parent entry
/// (`None` for the tray's menu).
pub(crate) trait TrayBackend: Send + Sync {
    fn update(&self);
    fn destroy(&self);
    fn set_icon(&self, icon: Option<&Surface<'_>>);
    fn set_tooltip(&self, tooltip: Option<&str>);
    fn create_menu(&self) -> Result<()>;
    fn has_menu(&self) -> bool;
    fn insert_entry(
        &self,
        menu: Option<EntryKey>,
        pos: i32,
        label: Option<&str>,
        flags: TrayEntryFlags,
    ) -> Result<EntryKey>;
    fn create_submenu(&self, entry: EntryKey) -> Result<()>;
    fn has_submenu(&self, entry: EntryKey) -> Option<bool>;
    fn entries(&self, menu: Option<EntryKey>) -> Result<Vec<EntryKey>>;
    /// The menu holding `entry` (`None` when there's no such entry).
    fn entry_parent(&self, entry: EntryKey) -> Option<Option<EntryKey>>;
    fn remove_entry(&self, entry: EntryKey);
    fn set_label(&self, entry: EntryKey, label: Option<&str>);
    fn label(&self, entry: EntryKey) -> Option<String>;
    fn set_checked(&self, entry: EntryKey, checked: bool);
    fn checked(&self, entry: EntryKey) -> bool;
    fn set_enabled(&self, entry: EntryKey, enabled: bool);
    fn enabled(&self, entry: EntryKey) -> bool;
    fn set_callback(&self, entry: EntryKey, callback: TrayCallback);
    #[allow(dead_code)] // (unreachable: see TrayEntry::click)
    fn click(&self, entry: EntryKey);
}

/// A system tray icon. Translation of `SDL_Tray *`; dropping it is
/// `SDL_DestroyTray()`. It dereferences to its [`TrayRef`].
pub struct Tray {
    inner: Arc<dyn TrayBackend>,
    tray: TrayRef,
}

impl std::fmt::Debug for Tray {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tray").finish_non_exhaustive()
    }
}

/// A handle to a tray, as passed to the click callbacks and returned by
/// [`TrayMenu::parent_tray`]; its methods fail (or do nothing) once the
/// tray is destroyed.
#[derive(Clone)]
pub struct TrayRef {
    inner: Weak<dyn TrayBackend>,
}

impl std::fmt::Debug for TrayRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TrayRef").finish_non_exhaustive()
    }
}

impl PartialEq for TrayRef {
    fn eq(&self, other: &TrayRef) -> bool {
        Weak::ptr_eq(&self.inner, &other.inner)
    }
}

/// A menu of a tray or of a tray entry. Translation of `SDL_TrayMenu *`.
#[derive(Clone, Debug, PartialEq)]
pub struct TrayMenu {
    tray: TrayRef,
    parent: Option<EntryKey>,
}

/// An entry of a tray menu. Translation of `SDL_TrayEntry *`.
#[derive(Clone, Debug, PartialEq)]
pub struct TrayEntry {
    tray: TrayRef,
    key: EntryKey,
}

impl Tray {
    /// Create an icon to be placed in the operating system's tray, or
    /// equivalent; call it on the main thread. Translation of
    /// `SDL_CreateTray()`.
    pub fn create(icon: Option<&Surface<'_>>, tooltip: Option<&str>) -> Result<Tray> {
        Tray::create_with(TrayOptions {
            icon,
            tooltip: tooltip.map(str::to_owned),
            ..TrayOptions::default()
        })
    }

    /// Create a tray with options; call it on the main thread. Translation
    /// of `SDL_CreateTrayWithProperties()`.
    pub fn create_with(options: TrayOptions<'_>) -> Result<Tray> {
        let inner = sys::create_tray(options)?;
        Ok(Tray::from_backend(inner))
    }

    fn from_backend(inner: Arc<dyn TrayBackend>) -> Tray {
        register_tray(&inner);
        let tray = TrayRef {
            inner: Arc::downgrade(&inner),
        };
        Tray { inner, tray }
    }

    /// A handle to the tray.
    pub fn handle(&self) -> TrayRef {
        self.tray.clone()
    }
}

impl std::ops::Deref for Tray {
    type Target = TrayRef;
    fn deref(&self) -> &TrayRef {
        &self.tray
    }
}

impl Drop for Tray {
    /// Translation of `SDL_DestroyTray()`.
    fn drop(&mut self) {
        destroy_tray(&self.inner);
    }
}

impl TrayRef {
    fn backend(&self) -> Option<Arc<dyn TrayBackend>> {
        let inner = self.inner.upgrade()?;
        is_tray_valid(&inner).then_some(inner)
    }

    fn valid(&self) -> Result<Arc<dyn TrayBackend>> {
        self.backend().ok_or_else(|| Error::invalid_param("tray"))
    }

    /// Update the icon. Translation of `SDL_SetTrayIcon()`.
    pub fn set_icon(&self, icon: Option<&Surface<'_>>) {
        if let Some(b) = self.backend() {
            b.set_icon(icon);
        }
    }

    /// Update the tooltip. Translation of `SDL_SetTrayTooltip()`.
    pub fn set_tooltip(&self, tooltip: Option<&str>) {
        if let Some(b) = self.backend() {
            b.set_tooltip(tooltip);
        }
    }

    /// Create the menu of the tray. Translation of `SDL_CreateTrayMenu()`.
    pub fn create_menu(&self) -> Result<TrayMenu> {
        self.valid()?.create_menu()?;
        Ok(TrayMenu {
            tray: self.clone(),
            parent: None,
        })
    }

    /// The tray's menu, if it has one. Translation of `SDL_GetTrayMenu()`.
    pub fn menu(&self) -> Option<TrayMenu> {
        self.backend()?.has_menu().then(|| TrayMenu {
            tray: self.clone(),
            parent: None,
        })
    }
}

impl TrayMenu {
    fn valid(&self) -> Result<Arc<dyn TrayBackend>> {
        self.tray
            .backend()
            .ok_or_else(|| Error::invalid_param("menu"))
    }

    /// The entries of the menu, in order. Translation of
    /// `SDL_GetTrayEntries()`.
    pub fn entries(&self) -> Result<Vec<TrayEntry>> {
        Ok(self
            .valid()?
            .entries(self.parent)?
            .into_iter()
            .map(|key| TrayEntry {
                tray: self.tray.clone(),
                key,
            })
            .collect())
    }

    /// Insert an entry at a position (-1 for the end); a `None` label makes
    /// a separator. Translation of `SDL_InsertTrayEntryAt()`.
    pub fn insert_entry(
        &self,
        pos: i32,
        label: Option<&str>,
        flags: TrayEntryFlags,
    ) -> Result<TrayEntry> {
        let key = self.valid()?.insert_entry(self.parent, pos, label, flags)?;
        Ok(TrayEntry {
            tray: self.tray.clone(),
            key,
        })
    }

    /// The entry the menu is a submenu of (`None` for the tray's menu).
    /// Translation of `SDL_GetTrayMenuParentEntry()`.
    pub fn parent_entry(&self) -> Option<TrayEntry> {
        self.parent.map(|key| TrayEntry {
            tray: self.tray.clone(),
            key,
        })
    }

    /// The tray the menu belongs to. Translation of
    /// `SDL_GetTrayMenuParentTray()`.
    pub fn parent_tray(&self) -> TrayRef {
        self.tray.clone()
    }
}

impl TrayEntry {
    fn valid(&self) -> Result<Arc<dyn TrayBackend>> {
        let b = self
            .tray
            .backend()
            .ok_or_else(|| Error::invalid_param("entry"))?;
        b.entry_parent(self.key)
            .ok_or_else(|| Error::invalid_param("entry"))?;
        Ok(b)
    }

    /// Create a submenu for the entry. Translation of
    /// `SDL_CreateTraySubmenu()`.
    pub fn create_submenu(&self) -> Result<TrayMenu> {
        self.valid()?.create_submenu(self.key)?;
        Ok(TrayMenu {
            tray: self.tray.clone(),
            parent: Some(self.key),
        })
    }

    /// The entry's submenu, if it has one. Translation of
    /// `SDL_GetTraySubmenu()`.
    pub fn submenu(&self) -> Option<TrayMenu> {
        let b = self.valid().ok()?;
        b.has_submenu(self.key)?.then(|| TrayMenu {
            tray: self.tray.clone(),
            parent: Some(self.key),
        })
    }

    /// Remove the entry (and its submenu) from its menu. Translation of
    /// `SDL_RemoveTrayEntry()`.
    pub fn remove(self) {
        if let Ok(b) = self.valid() {
            b.remove_entry(self.key);
        }
    }

    /// Set the label. Translation of `SDL_SetTrayEntryLabel()`.
    pub fn set_label(&self, label: Option<&str>) {
        if let Ok(b) = self.valid() {
            b.set_label(self.key, label);
        }
    }

    /// The label (`None` for a separator). Translation of
    /// `SDL_GetTrayEntryLabel()`.
    pub fn label(&self) -> Result<Option<String>> {
        Ok(self.valid()?.label(self.key))
    }

    /// Check or uncheck a checkbox. Translation of `SDL_SetTrayEntryChecked()`.
    pub fn set_checked(&self, checked: bool) {
        if let Ok(b) = self.valid() {
            b.set_checked(self.key, checked);
        }
    }

    /// Whether a checkbox is checked. Translation of
    /// `SDL_GetTrayEntryChecked()`.
    pub fn checked(&self) -> Result<bool> {
        Ok(self.valid()?.checked(self.key))
    }

    /// Enable or disable the entry. Translation of `SDL_SetTrayEntryEnabled()`.
    pub fn set_enabled(&self, enabled: bool) {
        if let Ok(b) = self.valid() {
            b.set_enabled(self.key, enabled);
        }
    }

    /// Whether the entry is enabled. Translation of
    /// `SDL_GetTrayEntryEnabled()`.
    pub fn enabled(&self) -> Result<bool> {
        Ok(self.valid()?.enabled(self.key))
    }

    /// Set the callback run when the entry is selected (it doesn't run
    /// while it is already running). Translation of
    /// `SDL_SetTrayEntryCallback()`.
    pub fn set_callback(&self, callback: impl FnMut(&TrayEntry) + Send + 'static) {
        if let Ok(b) = self.valid() {
            b.set_callback(self.key, Box::new(callback));
        }
    }

    /// Simulate a click on the entry. Translation of `SDL_ClickTrayEntry()`.
    pub fn click(&self) {
        if let Ok(b) = self.valid() {
            // FIXME (upstream): SDL_ClickTrayEntry() calls the driver's
            // GetTrayEntryEnabled() instead of its ClickTrayEntry(), so a
            // simulated click does nothing.
            let _ = b.enabled(self.key);
        }
    }

    /// The menu the entry is in. Translation of `SDL_GetTrayEntryParent()`.
    pub fn parent(&self) -> Result<TrayMenu> {
        let parent = self
            .valid()?
            .entry_parent(self.key)
            .ok_or_else(|| Error::invalid_param("entry"))?;
        Ok(TrayMenu {
            tray: self.tray.clone(),
            parent,
        })
    }
}

/// Update the trays: called by the event loop, or by the application if it
/// doesn't use one. Translation of `SDL_UpdateTrays()`.
pub fn update_trays() {
    sys::update_trays();
}

#[cfg(all(unix, not(target_vendor = "apple"), not(target_os = "android")))]
use unix as sys;

#[cfg(not(all(unix, not(target_vendor = "apple"), not(target_os = "android"))))]
mod sys {
    //! Translation of `src/tray/dummy/SDL_tray.c`.

    use super::{TrayBackend, TrayOptions};
    use crate::error::{Error, Result};
    use std::sync::Arc;

    /// Translation of `SDL_CreateTrayWithProperties()` (dummy).
    pub(super) fn create_tray(_options: TrayOptions<'_>) -> Result<Arc<dyn TrayBackend>> {
        Err(Error::unsupported())
    }

    /// Translation of `SDL_UpdateTrays()` (dummy).
    pub(super) fn update_trays() {}

    /// (no driver to destroy)
    pub(super) fn destroy_driver() {}
}

// * * * SDL_tray_utils.c

/// Translation of `active_trays`.
static ACTIVE_TRAYS: AtomicI32 = AtomicI32::new(0);

/// The valid trays (upstream's `SDL_OBJECT_TYPE_TRAY` objects).
static TRAYS: Mutex<Vec<Weak<dyn TrayBackend>>> = Mutex::new(Vec::new());

fn trays() -> std::sync::MutexGuard<'static, Vec<Weak<dyn TrayBackend>>> {
    TRAYS.lock().unwrap_or_else(|e| e.into_inner())
}

/// The data pointer of a tray, to compare trays by identity.
fn tray_addr(tray: &Weak<dyn TrayBackend>) -> *const () {
    Weak::as_ptr(tray) as *const ()
}

/// Translation of `SDL_ObjectValid(tray, SDL_OBJECT_TYPE_TRAY)`.
fn is_tray_valid(tray: &Arc<dyn TrayBackend>) -> bool {
    let addr = Arc::as_ptr(tray) as *const ();
    trays().iter().any(|t| tray_addr(t) == addr)
}

/// The valid trays, for `SDL_GetObjects(SDL_OBJECT_TYPE_TRAY)`.
#[allow(dead_code)] // (used by the platform backends)
pub(crate) fn valid_trays() -> Vec<Arc<dyn TrayBackend>> {
    trays().iter().filter_map(Weak::upgrade).collect()
}

/// Count a newly created tray. Translation of `SDL_RegisterTray()`.
fn register_tray(tray: &Arc<dyn TrayBackend>) {
    trays().push(Arc::downgrade(tray));

    ACTIVE_TRAYS.fetch_add(1, Ordering::AcqRel);
}

/// Count a destroyed tray, and quit if it was the last thing keeping the
/// app alive. Translation of `SDL_UnregisterTray()`.
fn unregister_tray(tray: &Arc<dyn TrayBackend>) {
    let addr = Arc::as_ptr(tray) as *const ();
    trays().retain(|t| tray_addr(t) != addr);

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

/// Translation of `SDL_DestroyTray()`.
fn destroy_tray(tray: &Arc<dyn TrayBackend>) {
    if !is_tray_valid(tray) {
        return;
    }

    unregister_tray(tray);
    tray.destroy();

    if active_tray_count() == 0 {
        sys::destroy_driver();
    }
}

/// Destroy the remaining trays at shutdown (the application's [`Tray`]s
/// then do nothing when dropped). Translation of `SDL_CleanupTrays()`.
pub(crate) fn cleanup_trays() {
    if ACTIVE_TRAYS.load(Ordering::Acquire) == 0 {
        return;
    }

    for tray in valid_trays() {
        destroy_tray(&tray);
    }
}

/// Whether any trays exist. Translation of `SDL_HasActiveTrays()`.
pub(crate) fn has_active_trays() -> bool {
    ACTIVE_TRAYS.load(Ordering::Acquire) > 0
}

/// The number of trays. Translation of `SDL_GetActiveTrayCount()`.
pub(crate) fn active_tray_count() -> i32 {
    ACTIVE_TRAYS.load(Ordering::Acquire)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(all(unix, not(target_vendor = "apple"), not(target_os = "android"))))]
    #[test]
    fn dummy_tray() {
        assert_eq!(
            Tray::create(None, Some("tip")).unwrap_err().to_string(),
            "That operation is not supported"
        );
        assert!(!has_active_trays());
    }

    #[test]
    fn entry_flags() {
        assert!(
            (TrayEntryFlags::CHECKBOX | TrayEntryFlags::CHECKED).contains(TrayEntryFlags::CHECKED)
        );
        assert!(!TrayEntryFlags::BUTTON.contains(TrayEntryFlags::CHECKBOX));
    }
}
