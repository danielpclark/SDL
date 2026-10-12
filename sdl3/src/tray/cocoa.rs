// Rust translation of src/tray/cocoa/SDL_tray.m from Simple DirectMedia
// Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The macOS tray: an `NSStatusItem` in the system status bar, whose button
//! sends left and right clicks to an `SDLTrayClickHandler` (a class
//! declared at run time) and whose middle clicks are caught by a local
//! event monitor; the menus are `NSMenu`s popped up from the click handler.
//! Selecting a menu item reaches the entry through the Cocoa video driver's
//! app delegate (`-[SDL3AppDelegate menu:]`, which calls
//! `SDL_ClickTrayEntry()` with the item's represented object), so the
//! entries carry their keys there.

use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};

use super::{
    EntryKey, TrayBackend, TrayCallback, TrayClickCallback, TrayEntry, TrayEntryFlags, TrayOptions,
    TrayRef,
};
use crate::core::apple::block::RcBlock;
use crate::core::apple::foundation::{ns_string, string_from_ns, CGFloat, NSPoint, NSRect, NSSize};
use crate::core::apple::objc::{
    autoreleasepool, class, imp, ivar_ptr, msg_send, sel, Bool, ClassBuilder, DeclaredClass, Id,
    NSInteger, Sel, StrongPtr, NO, YES,
};
use crate::error::{Error, Result};
use crate::video::pixels::PixelFormat;
use crate::video::surface::Surface;

/// `NSVariableStatusItemLength`.
const NS_VARIABLE_STATUS_ITEM_LENGTH: CGFloat = -1.0;
/// `NSControlStateValueOn`.
const NS_CONTROL_STATE_VALUE_ON: NSInteger = 1;
/// `NSControlStateValueOff`.
const NS_CONTROL_STATE_VALUE_OFF: NSInteger = 0;
/// `NSEventMaskLeftMouseUp`.
const NS_EVENT_MASK_LEFT_MOUSE_UP: u64 = 1 << 2;
/// `NSEventMaskRightMouseUp`.
const NS_EVENT_MASK_RIGHT_MOUSE_UP: u64 = 1 << 4;
/// `NSEventMaskOtherMouseUp`.
const NS_EVENT_MASK_OTHER_MOUSE_UP: u64 = 1 << 26;

#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {
    static NSCalibratedRGBColorSpace: Id;
}

/// Translation of `struct SDL_TrayMenu` (its parent tray is the tray that
/// holds it, when `parent_entry` is `None`).
struct Menu {
    nsmenu: StrongPtr,
    entries: Vec<Entry>,
    parent_entry: Option<EntryKey>,
}

/// Translation of `struct SDL_TrayEntry` (its parent menu is the menu that
/// holds it).
struct Entry {
    key: EntryKey,
    nsitem: StrongPtr,
    flags: TrayEntryFlags,
    callback: Option<TrayCallback>,
    submenu: Option<Box<Menu>>,
}

/// Translation of the rest of `struct SDL_Tray`.
struct TrayState {
    status_item: StrongPtr,
    menu: Option<Box<Menu>>,
    click_handler: StrongPtr,

    left_click_callback: Option<TrayClickCallback>,
    right_click_callback: Option<TrayClickCallback>,
    middle_click_callback: Option<TrayClickCallback>,
}

/// Translation of `struct SDL_Tray`.
pub(super) struct TrayCocoa {
    this: Weak<TrayCocoa>,
    state: Mutex<TrayState>,
}

impl TrayCocoa {
    fn state(&self) -> std::sync::MutexGuard<'_, TrayState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn tray_ref(&self) -> TrayRef {
        let inner: Weak<dyn TrayBackend> = self.this.clone();
        TrayRef { inner }
    }

    fn entry_ref(&self, key: EntryKey) -> TrayEntry {
        TrayEntry {
            tray: self.tray_ref(),
            key,
        }
    }

    /// Run an entry's callback (without the lock, which the callback may
    /// need; it doesn't run again while it runs).
    fn run_entry_callback(&self, key: EntryKey) {
        let callback = find_entry(&mut self.state(), key).and_then(|e| e.callback.take());
        if let Some(mut callback) = callback {
            callback(&self.entry_ref(key));
            if let Some(entry) = find_entry(&mut self.state(), key) {
                // (unless the callback set a new one)
                if entry.callback.is_none() {
                    entry.callback = Some(callback);
                }
            }
        }
    }

    /// Run a click callback, by which of the tray's callbacks it is
    /// (`None` when there is none).
    fn run_click_callback(
        &self,
        which: fn(&mut TrayState) -> &mut Option<TrayClickCallback>,
    ) -> Option<bool> {
        let mut callback = which(&mut self.state()).take()?;
        let show_menu = callback(&self.tray_ref());
        let mut state = self.state();
        let slot = which(&mut state);
        if slot.is_none() {
            *slot = Some(callback);
        }
        Some(show_menu)
    }
}

/// The keys of the entries, unique in the process.
fn next_key() -> EntryKey {
    static NEXT_KEY: AtomicU64 = AtomicU64::new(0);
    NEXT_KEY.fetch_add(1, Ordering::Relaxed) + 1
}

/// The menu holding the entry `key`, searched from `menu`.
fn find_parent_in_menu(menu: &mut Menu, key: EntryKey) -> Option<&mut Menu> {
    if menu.entries.iter().any(|e| e.key == key) {
        return Some(menu);
    }
    for entry in &mut menu.entries {
        if let Some(submenu) = &mut entry.submenu {
            if let Some(m) = find_parent_in_menu(submenu, key) {
                return Some(m);
            }
        }
    }
    None
}

/// The entry `key` of the tray.
fn find_entry(state: &mut TrayState, key: EntryKey) -> Option<&mut Entry> {
    let menu = state.menu.as_deref_mut()?;
    find_parent_in_menu(menu, key)?
        .entries
        .iter_mut()
        .find(|e| e.key == key)
}

/// The tray's menu (`None`) or an entry's submenu.
fn find_menu(state: &mut TrayState, parent: Option<EntryKey>) -> Option<&mut Menu> {
    match parent {
        None => state.menu.as_deref_mut(),
        Some(key) => find_entry(state, key)?.submenu.as_deref_mut(),
    }
}

// * * * SDLTrayClickHandler

/// The instance variable holding the click handler's tray (a leaked
/// `Box<Weak<TrayCocoa>>`, or null): upstream's `tray` property.
const IVAR_TRAY: &std::ffi::CStr = c"_tray";
/// The instance variable holding the middle click monitor (retained, or
/// nil): upstream's `middleClickMonitor` property.
const IVAR_MONITOR: &std::ffi::CStr = c"_middleClickMonitor";

static CLICK_HANDLER_CLASS: DeclaredClass = DeclaredClass::new();

/// The `SDLTrayClickHandler` class: "Objective-C helper class to handle
/// status item button clicks".
fn click_handler_class() -> crate::core::apple::objc::Class {
    CLICK_HANDLER_CLASS.get(|| {
        let mut b = ClassBuilder::new(c"SDLTrayClickHandler", class!(NSObject))
            .expect("SDLTrayClickHandler is declared once");
        b.add_ivar::<*mut c_void>(IVAR_TRAY, c"^v");
        b.add_ivar::<Id>(IVAR_MONITOR, c"@");
        // SAFETY: the implementations have the signatures the encodings say.
        unsafe {
            b.add_method(
                sel!(handleClick:),
                imp!(handle_click as extern "C" fn(Id, Sel, Id)),
                c"v@:@",
            );
            b.add_method(
                sel!(dealloc),
                imp!(click_handler_dealloc as extern "C" fn(Id, Sel)),
                c"v@:",
            );
        }
        b.register()
    })
}

/// The click handler's `tray` slot.
///
/// # Safety
///
/// `this` must be an `SDLTrayClickHandler`.
unsafe fn tray_slot(this: Id) -> *mut *mut Weak<TrayCocoa> {
    // SAFETY: the caller's contract; the ivar holds a pointer.
    unsafe { ivar_ptr::<*mut Weak<TrayCocoa>>(this, click_handler_class(), IVAR_TRAY) }
}

/// The click handler's `middleClickMonitor` slot.
///
/// # Safety
///
/// `this` must be an `SDLTrayClickHandler`.
unsafe fn monitor_slot(this: Id) -> *mut Id {
    // SAFETY: the caller's contract; the ivar holds an object.
    unsafe { ivar_ptr::<Id>(this, click_handler_class(), IVAR_MONITOR) }
}

/// The handler's tray, if it still has one (`self.tray`).
///
/// # Safety
///
/// `this` must be an `SDLTrayClickHandler`.
unsafe fn handler_tray(this: Id) -> Option<Arc<TrayCocoa>> {
    // SAFETY: the caller's contract; the slot is null or a live box.
    unsafe {
        let p = *tray_slot(this);
        if p.is_null() {
            None
        } else {
            (*p).upgrade()
        }
    }
}

/// `self.tray = tray` (NULL to detach the handler).
///
/// # Safety
///
/// `this` must be an `SDLTrayClickHandler`.
unsafe fn set_handler_tray(this: Id, tray: Option<Weak<TrayCocoa>>) {
    // SAFETY: the caller's contract; the old box is freed once.
    unsafe {
        let slot = tray_slot(this);
        let old = *slot;
        *slot = match tray {
            Some(t) => Box::into_raw(Box::new(t)),
            None => std::ptr::null_mut(),
        };
        if !old.is_null() {
            drop(Box::from_raw(old));
        }
    }
}

/// The instances' cleanup (what ARC does for the properties).
extern "C" fn click_handler_dealloc(this: Id, _sel: Sel) {
    // SAFETY: an SDLTrayClickHandler being freed: its slots are released,
    // then NSObject's dealloc runs.
    unsafe {
        set_handler_tray(this, None);
        let monitor = std::mem::replace(&mut *monitor_slot(this), std::ptr::null_mut());
        crate::core::apple::objc::release(monitor);
        let _: () = crate::core::apple::objc::msg_send_super![this, click_handler_class(), dealloc];
    }
}

/// Translation of `-[SDLTrayClickHandler handleClick:]`.
extern "C" fn handle_click(this: Id, _sel: Sel, _sender: Id) {
    // SAFETY: called by the status item's button on the main thread.
    let Some(tray) = (unsafe { handler_tray(this) }) else {
        return;
    };

    // SAFETY: NSApp exists (it delivered the click).
    let button_number: NSInteger = unsafe {
        let app: Id = msg_send![class!(NSApplication), sharedApplication];
        let event: Id = msg_send![app, currentEvent];
        msg_send![event, buttonNumber]
    };

    let mut show_menu = false;

    if button_number == 0 {
        /* Left click */
        show_menu = tray
            .run_click_callback(|s| &mut s.left_click_callback)
            .unwrap_or(true);
    } else if button_number == 1 {
        /* Right click */
        show_menu = tray
            .run_click_callback(|s| &mut s.right_click_callback)
            .unwrap_or(true);
    } else if button_number == 2 {
        /* Middle click */
        let _ = tray.run_click_callback(|s| &mut s.middle_click_callback);
    }

    if show_menu {
        let (status_item, nsmenu) = {
            let state = tray.state();
            (
                state.status_item.clone(),
                state.menu.as_ref().map(|m| m.nsmenu.clone()),
            )
        };
        if let Some(nsmenu) = nsmenu {
            // SAFETY: the status item and the menu, retained for the call
            // (which tracks the menu until it closes).
            let _: () = unsafe { msg_send![status_item, popUpStatusItemMenu: nsmenu.get()] };
        }
    }
}

/// Translation of `-[SDLTrayClickHandler startMonitoringMiddleClicks]`.
///
/// # Safety
///
/// `this` must be an `SDLTrayClickHandler`, on the main thread.
unsafe fn start_monitoring_middle_clicks(this: Id) {
    // SAFETY: the caller's contract.
    if !unsafe { *monitor_slot(this) }.is_null() {
        return;
    }

    // (`__weak SDLTrayClickHandler *weakSelf`: the block holds the tray
    // weakly, which is what it reaches through the handler)
    // SAFETY: the caller's contract.
    let weak_tray = match unsafe { handler_tray(this) } {
        Some(t) => Arc::downgrade(&t),
        None => Weak::new(),
    };
    let block = RcBlock::new1(move |event: Id| -> Id {
        let Some(tray) = weak_tray.upgrade() else {
            return event;
        };
        // SAFETY: the monitor's event, and the tray's status item, on the
        // main thread.
        unsafe {
            let button_number: NSInteger = msg_send![event, buttonNumber];
            if button_number != 2 {
                return event;
            }

            /* Check if the click is within the status item's button bounds */
            let click_location: NSPoint = msg_send![event, locationInWindow];
            let status_item = tray.state().status_item.clone();
            let button: Id = msg_send![status_item, button];
            let status_item_window: Id = msg_send![button, window];
            let event_window: Id = msg_send![event, window];

            if !status_item_window.is_null() && event_window == status_item_window {
                let local_point: NSPoint = msg_send![button,
                    convertPoint: click_location
                    fromView: std::ptr::null_mut::<c_void>()];
                let bounds: NSRect = msg_send![button, bounds];
                // (NSPointInRect)
                if bounds.contains(local_point) {
                    let _ = tray.run_click_callback(|s| &mut s.middle_click_callback);
                }
            }
        }

        event
    });
    // SAFETY: NSEvent copies the block; the handler keeps the monitor.
    unsafe {
        let monitor: Id = msg_send![class!(NSEvent),
            addLocalMonitorForEventsMatchingMask: NS_EVENT_MASK_OTHER_MOUSE_UP
            handler: block.as_ptr()];
        *monitor_slot(this) = crate::core::apple::objc::retain(monitor);
    }
}

/// Translation of `-[SDLTrayClickHandler stopMonitoringMiddleClicks]`.
///
/// # Safety
///
/// `this` must be an `SDLTrayClickHandler`, on the main thread.
unsafe fn stop_monitoring_middle_clicks(this: Id) {
    // SAFETY: the caller's contract; the monitor is released once.
    unsafe {
        let monitor = *monitor_slot(this);
        if !monitor.is_null() {
            let _: () = msg_send![class!(NSEvent), removeMonitor: monitor];
            *monitor_slot(this) = std::ptr::null_mut();
            crate::core::apple::objc::release(monitor);
        }
    }
}

// * * *

/// Translation of `DestroySDLMenu()`: `owner` is what holds the menu, the
/// parent menu and item of a submenu, or the tray's status item.
fn destroy_sdl_menu(menu: Menu, owner: MenuOwner) {
    let Menu {
        nsmenu, entries, ..
    } = menu;
    for entry in entries {
        if let Some(submenu) = entry.submenu {
            destroy_sdl_menu(
                *submenu,
                MenuOwner::Entry {
                    parent: nsmenu.get(),
                    item: entry.nsitem.get(),
                },
            );
        }
    }

    // SAFETY: live menus, items and status item.
    unsafe {
        match owner {
            MenuOwner::Entry { parent, item } => {
                let _: () = msg_send![parent,
                    setSubmenu: std::ptr::null_mut::<c_void>()
                    forItem: item];
            }
            MenuOwner::Tray(status_item) => {
                let _: () = msg_send![status_item, setMenu: std::ptr::null_mut::<c_void>()];
            }
        }
    }
}

/// What holds a menu being destroyed.
enum MenuOwner {
    Entry { parent: Id, item: Id },
    Tray(Id),
}

/// The 22x22 status bar image of an icon converted to RGBA32. Upstream's
/// code in `SDL_CreateTrayWithProperties()` and `SDL_SetTrayIcon()`.
///
/// This function assumes that it's called from within an autorelease pool.
fn status_bar_image(icon: &mut Surface<'_>) -> Option<StrongPtr> {
    let (w, h, pitch) = (icon.width(), icon.height(), icon.pitch());
    let mut pixels = icon.pixels_mut()?.as_mut_ptr();
    // SAFETY: the bitmap uses the surface's pixels (it doesn't copy them)
    // only while the 22x22 image is drawn from it, before this returns.
    unsafe {
        let bitmap: Id = msg_send![class!(NSBitmapImageRep), alloc];
        let bitmap: Id = msg_send![bitmap,
            initWithBitmapDataPlanes: &mut pixels as *mut *mut u8
            pixelsWide: w as NSInteger
            pixelsHigh: h as NSInteger
            bitsPerSample: 8 as NSInteger
            samplesPerPixel: 4 as NSInteger
            hasAlpha: YES
            isPlanar: NO
            colorSpaceName: NSCalibratedRGBColorSpace
            bytesPerRow: pitch as NSInteger
            bitsPerPixel: 32 as NSInteger];
        let bitmap = StrongPtr::from_owned(bitmap);
        let iconimg: Id = msg_send![class!(NSImage), alloc];
        let iconimg: Id = msg_send![iconimg, initWithSize: NSSize::new(w as CGFloat, h as CGFloat)];
        let iconimg = StrongPtr::from_owned(iconimg);
        let _: () = msg_send![iconimg, addRepresentation: bitmap.get()];

        /* A typical icon size is 22x22 on macOS. Failing to resize the icon
        may give oversized status bar buttons. */
        let iconimg22: Id = msg_send![class!(NSImage), alloc];
        let iconimg22: Id = msg_send![iconimg22, initWithSize: NSSize::new(22.0, 22.0)];
        let iconimg22 = StrongPtr::from_owned(iconimg22);
        let _: () = msg_send![iconimg22, lockFocus];
        let _: () = msg_send![iconimg, setSize: NSSize::new(22.0, 22.0)];
        let _: () = msg_send![iconimg, drawInRect: NSRect::new(0.0, 0.0, 22.0, 22.0)];
        let _: () = msg_send![iconimg22, unlockFocus];

        Some(iconimg22)
    }
}

/// `tray->statusItem.button`.
///
/// # Safety
///
/// `status_item` must be an `NSStatusItem`.
unsafe fn status_button(status_item: &StrongPtr) -> Id {
    // SAFETY: the caller's contract.
    unsafe { msg_send![status_item, button] }
}

/// `tray->statusItem.button.toolTip = ...`.
fn set_button_tooltip(status_item: &StrongPtr, tooltip: Option<&str>) {
    // SAFETY: a live status item; a nil tooltip removes it.
    unsafe {
        let button = status_button(status_item);
        match tooltip {
            Some(tooltip) => {
                let _: () = msg_send![button, setToolTip: ns_string(tooltip).get()];
            }
            None => {
                let _: () = msg_send![button, setToolTip: std::ptr::null_mut::<c_void>()];
            }
        }
    }
}

/// `tray->statusItem.button.image = ...`.
fn set_button_image(status_item: &StrongPtr, image: Option<&StrongPtr>) {
    let image = image.map_or(std::ptr::null_mut(), StrongPtr::get);
    // SAFETY: a live status item; nil removes the image.
    let _: () = unsafe { msg_send![status_button(status_item), setImage: image] };
}

/// Translation of `SDL_UpdateTrays()`.
pub(super) fn update_trays() {}

/// What `-[SDL3AppDelegate menu:]` reaches with a menu item's represented
/// object (the entry's key): `SDL_ClickTrayEntry()` on the tray that has
/// the entry.
fn click_tray_entry_from_menu(entry: *mut c_void) {
    let key = entry as usize as EntryKey;
    for tray in super::valid_trays() {
        if tray.entry_parent(key).is_some() {
            tray.click(key);
            return;
        }
    }
}

/// Translation of `SDL_CreateTrayWithProperties()` (the caller registers
/// the tray).
pub(super) fn create_tray(options: TrayOptions<'_>) -> Result<Arc<dyn TrayBackend>> {
    if !crate::init::is_main_thread() {
        return Err(Error::new(
            "This function should be called on the main thread",
        ));
    }

    let TrayOptions {
        icon,
        tooltip,
        left_click,
        right_click,
        middle_click,
    } = options;

    let mut icon = match icon {
        Some(icon) => Some(icon.convert(PixelFormat::RGBA32)?),
        None => None,
    };

    *crate::video::drivers::cocoa::events::TRAY_MENU_HANDLER
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = Some(click_tray_entry_from_menu);

    autoreleasepool(|| {
        // SAFETY: NSStatusBar and NSApplication as documented, on the main
        // thread.
        let status_item = unsafe {
            let status_bar: Id = msg_send![class!(NSStatusBar), systemStatusBar];
            let item: Id =
                msg_send![status_bar, statusItemWithLength: NS_VARIABLE_STATUS_ITEM_LENGTH];
            let app: Id = msg_send![class!(NSApplication), sharedApplication];
            let _: () = msg_send![app, activateIgnoringOtherApps: YES];
            StrongPtr::retain(item)
        };

        set_button_tooltip(&status_item, tooltip.as_deref());

        if let Some(icon) = &mut icon {
            let image = status_bar_image(icon);
            set_button_image(&status_item, image.as_ref());
        }

        /* Create click handler and set up button to receive clicks */
        // SAFETY: a new instance of the handler class.
        let click_handler = unsafe {
            let h: Id = msg_send![click_handler_class(), alloc];
            StrongPtr::from_owned(msg_send![h, init])
        };

        let tray = Arc::new_cyclic(|this: &Weak<TrayCocoa>| TrayCocoa {
            this: this.clone(),
            state: Mutex::new(TrayState {
                status_item,
                menu: None,
                click_handler,
                left_click_callback: left_click,
                right_click_callback: right_click,
                middle_click_callback: middle_click,
            }),
        });

        {
            let state = tray.state();
            // SAFETY: the handler is an SDLTrayClickHandler; the button
            // keeps its target unretained, the tray keeps the handler.
            unsafe {
                set_handler_tray(state.click_handler.get(), Some(Arc::downgrade(&tray)));

                let button = status_button(&state.status_item);
                let _: () = msg_send![button, setTarget: state.click_handler.get()];
                let _: () = msg_send![button, setAction: sel!(handleClick:)];
                let _: NSInteger = msg_send![button,
                    sendActionOn: NS_EVENT_MASK_LEFT_MOUSE_UP | NS_EVENT_MASK_RIGHT_MOUSE_UP];

                /* Start monitoring for middle clicks since status items don't receive them via the normal action mechanism */
                start_monitoring_middle_clicks(state.click_handler.get());
            }
        }

        let tray: Arc<dyn TrayBackend> = tray;
        Ok(tray)
    })
}

/// A new `NSMenu` that doesn't enable its items by itself.
fn new_nsmenu() -> StrongPtr {
    // SAFETY: a new menu.
    unsafe {
        let nsmenu: Id = msg_send![class!(NSMenu), alloc];
        let nsmenu = StrongPtr::from_owned(msg_send![nsmenu, init]);
        let _: () = msg_send![nsmenu, setAutoenablesItems: NO];
        nsmenu
    }
}

impl TrayBackend for TrayCocoa {
    fn update(&self) {}

    /// Translation of `SDL_DestroyTray()` (after `SDL_UnregisterTray()`).
    fn destroy(&self) {
        let mut state = self.state();

        autoreleasepool(|| {
            // SAFETY: the status bar and the tray's status item.
            unsafe {
                let status_bar: Id = msg_send![class!(NSStatusBar), systemStatusBar];
                let _: () = msg_send![status_bar, removeStatusItem: state.status_item.get()];
            }

            if let Some(menu) = state.menu.take() {
                destroy_sdl_menu(*menu, MenuOwner::Tray(state.status_item.get()));
            }

            if !state.click_handler.is_nil() {
                // SAFETY: the tray's SDLTrayClickHandler.
                unsafe {
                    stop_monitoring_middle_clicks(state.click_handler.get());
                    set_handler_tray(state.click_handler.get(), None);
                }
                state.click_handler = StrongPtr::nil();
            }
        });
    }

    /// Translation of `SDL_SetTrayIcon()`.
    fn set_icon(&self, icon: Option<&Surface<'_>>) {
        let state = self.state();
        autoreleasepool(|| {
            let Some(icon) = icon else {
                set_button_image(&state.status_item, None);
                return;
            };

            let Ok(mut icon) = icon.convert(PixelFormat::RGBA32) else {
                set_button_image(&state.status_item, None);
                return;
            };

            let image = status_bar_image(&mut icon);
            set_button_image(&state.status_item, image.as_ref());
        });
    }

    /// Translation of `SDL_SetTrayTooltip()`.
    fn set_tooltip(&self, tooltip: Option<&str>) {
        let state = self.state();
        autoreleasepool(|| set_button_tooltip(&state.status_item, tooltip));
    }

    /// Translation of `SDL_CreateTrayMenu()`.
    fn create_menu(&self) -> Result<()> {
        let mut state = self.state();

        /* Don't set menu on statusItem - we handle menu display manually in the click handler */

        // FIXME (upstream): a previous menu is leaked; here it is destroyed.
        if let Some(old) = state.menu.take() {
            destroy_sdl_menu(*old, MenuOwner::Tray(state.status_item.get()));
        }
        state.menu = Some(Box::new(Menu {
            nsmenu: new_nsmenu(),
            entries: Vec::new(),
            parent_entry: None,
        }));
        Ok(())
    }

    /// Translation of `SDL_GetTrayMenu()`.
    fn has_menu(&self) -> bool {
        self.state().menu.is_some()
    }

    /// Translation of `SDL_InsertTrayEntryAt()`.
    fn insert_entry(
        &self,
        menu: Option<EntryKey>,
        mut pos: i32,
        label: Option<&str>,
        flags: TrayEntryFlags,
    ) -> Result<EntryKey> {
        let mut state = self.state();
        let Some(menu) = find_menu(&mut state, menu) else {
            return Err(Error::invalid_param("menu"));
        };
        let n_entries = menu.entries.len() as i32;

        if pos < -1 || pos > n_entries {
            return Err(Error::invalid_param("pos"));
        }

        if pos == -1 {
            pos = n_entries;
        }

        let key = next_key();

        let nsitem = autoreleasepool(|| {
            // SAFETY: new menu items, inserted into a live menu.
            unsafe {
                let nsitem = match label {
                    None => StrongPtr::retain(msg_send![class!(NSMenuItem), separatorItem]),
                    Some(label) => {
                        // FIXME (upstream): the action is `menu`, which the
                        // responder chain answers with NSResponder's `menu`
                        // property (on NSApp) instead of the app delegate's
                        // `menu:`, so choosing the item doesn't reach the
                        // entry; only SDL_ClickTrayEntry() does.
                        let item: Id = msg_send![class!(NSMenuItem), alloc];
                        let item: Id = msg_send![item,
                            initWithTitle: ns_string(label).get()
                            action: sel!(menu)
                            keyEquivalent: ns_string("").get()];
                        let item = StrongPtr::from_owned(item);
                        let _: () = msg_send![item,
                            setEnabled: if flags.contains(TrayEntryFlags::DISABLED) { NO } else { YES }];
                        let _: () = msg_send![item,
                        setState: if flags.contains(TrayEntryFlags::CHECKED) {
                            NS_CONTROL_STATE_VALUE_ON
                        } else {
                            NS_CONTROL_STATE_VALUE_OFF
                        }];
                        let value: Id = msg_send![class!(NSValue),
                            valueWithPointer: key as usize as *const c_void];
                        let _: () = msg_send![item, setRepresentedObject: value];
                        item
                    }
                };

                let _: () = msg_send![menu.nsmenu,
                    insertItem: nsitem.get()
                    atIndex: pos as NSInteger];
                nsitem
            }
        });

        menu.entries.insert(
            pos as usize,
            Entry {
                key,
                nsitem,
                flags,
                callback: None,
                submenu: None,
            },
        );

        Ok(key)
    }

    /// Translation of `SDL_CreateTraySubmenu()`.
    fn create_submenu(&self, entry: EntryKey) -> Result<()> {
        let mut state = self.state();
        let Some(menu) = state
            .menu
            .as_deref_mut()
            .and_then(|m| find_parent_in_menu(m, entry))
        else {
            return Err(Error::invalid_param("entry"));
        };
        let parent_nsmenu = menu.nsmenu.clone();
        let Some(entry) = menu.entries.iter_mut().find(|e| e.key == entry) else {
            return Err(Error::invalid_param("entry"));
        };

        if entry.submenu.is_some() {
            return Err(Error::new("Tray entry submenu already exists"));
        }

        if !entry.flags.contains(TrayEntryFlags::SUBMENU) {
            return Err(Error::new(
                "Cannot create submenu for entry not created with SDL_TRAYENTRY_SUBMENU",
            ));
        }

        let nsmenu = new_nsmenu();
        // SAFETY: live menus and item.
        let _: () = unsafe {
            msg_send![parent_nsmenu, setSubmenu: nsmenu.get() forItem: entry.nsitem.get()]
        };
        entry.submenu = Some(Box::new(Menu {
            nsmenu,
            entries: Vec::new(),
            parent_entry: Some(entry.key),
        }));

        Ok(())
    }

    /// Translation of `SDL_GetTraySubmenu()`.
    fn has_submenu(&self, entry: EntryKey) -> Option<bool> {
        find_entry(&mut self.state(), entry).map(|e| e.submenu.is_some())
    }

    /// Translation of `SDL_GetTrayEntries()`.
    fn entries(&self, menu: Option<EntryKey>) -> Result<Vec<EntryKey>> {
        let mut state = self.state();
        let menu = find_menu(&mut state, menu).ok_or_else(|| Error::invalid_param("menu"))?;
        Ok(menu.entries.iter().map(|e| e.key).collect())
    }

    /// Translation of `SDL_GetTrayEntryParent()`.
    fn entry_parent(&self, entry: EntryKey) -> Option<Option<EntryKey>> {
        let mut state = self.state();
        let menu = state.menu.as_deref_mut()?;
        find_parent_in_menu(menu, entry).map(|m| m.parent_entry)
    }

    /// Translation of `SDL_RemoveTrayEntry()`.
    fn remove_entry(&self, entry: EntryKey) {
        let mut state = self.state();
        let Some(menu) = state
            .menu
            .as_deref_mut()
            .and_then(|m| find_parent_in_menu(m, entry))
        else {
            return;
        };

        let Some(i) = menu.entries.iter().position(|e| e.key == entry) else {
            return;
        };
        let mut entry = menu.entries.remove(i);

        autoreleasepool(|| {
            if let Some(submenu) = entry.submenu.take() {
                destroy_sdl_menu(
                    *submenu,
                    MenuOwner::Entry {
                        parent: menu.nsmenu.get(),
                        item: entry.nsitem.get(),
                    },
                );
            }

            // SAFETY: a live menu and its item.
            let _: () = unsafe { msg_send![menu.nsmenu, removeItem: entry.nsitem.get()] };
        });
    }

    /// Translation of `SDL_SetTrayEntryLabel()`.
    fn set_label(&self, entry: EntryKey, label: Option<&str>) {
        let mut state = self.state();
        let Some(entry) = find_entry(&mut state, entry) else {
            return;
        };

        // Note (upstream): a NULL label makes a nil title, which NSMenuItem
        // refuses with an exception; an empty title here.
        let title = ns_string(label.unwrap_or(""));
        // SAFETY: a live item.
        let _: () = unsafe { msg_send![entry.nsitem, setTitle: title.get()] };
    }

    /// Translation of `SDL_GetTrayEntryLabel()`.
    fn label(&self, entry: EntryKey) -> Option<String> {
        let mut state = self.state();
        let entry = find_entry(&mut state, entry)?;
        autoreleasepool(|| {
            // SAFETY: a live item; its title is an NSString.
            unsafe {
                let title: Id = msg_send![entry.nsitem, title];
                string_from_ns(title)
            }
        })
    }

    /// Translation of `SDL_SetTrayEntryChecked()`.
    fn set_checked(&self, entry: EntryKey, checked: bool) {
        let mut state = self.state();
        let Some(entry) = find_entry(&mut state, entry) else {
            return;
        };

        let value = if checked {
            NS_CONTROL_STATE_VALUE_ON
        } else {
            NS_CONTROL_STATE_VALUE_OFF
        };
        // SAFETY: a live item.
        let _: () = unsafe { msg_send![entry.nsitem, setState: value] };
    }

    /// Translation of `SDL_GetTrayEntryChecked()`.
    fn checked(&self, entry: EntryKey) -> bool {
        let mut state = self.state();
        let Some(entry) = find_entry(&mut state, entry) else {
            return false;
        };

        // SAFETY: a live item.
        let value: NSInteger = unsafe { msg_send![entry.nsitem, state] };
        value == NS_CONTROL_STATE_VALUE_ON
    }

    /// Translation of `SDL_SetTrayEntryEnabled()` (upstream only changes
    /// checkboxes).
    fn set_enabled(&self, entry: EntryKey, enabled: bool) {
        let mut state = self.state();
        let Some(entry) = find_entry(&mut state, entry) else {
            return;
        };
        if !entry.flags.contains(TrayEntryFlags::CHECKBOX) {
            return;
        }

        // SAFETY: a live item.
        let _: () = unsafe { msg_send![entry.nsitem, setEnabled: Bool::from(enabled)] };
    }

    /// Translation of `SDL_GetTrayEntryEnabled()` (upstream reports `false`
    /// for anything but a checkbox).
    fn enabled(&self, entry: EntryKey) -> bool {
        let mut state = self.state();
        let Some(entry) = find_entry(&mut state, entry) else {
            return false;
        };
        if !entry.flags.contains(TrayEntryFlags::CHECKBOX) {
            return false;
        }

        // SAFETY: a live item.
        let enabled: Bool = unsafe { msg_send![entry.nsitem, isEnabled] };
        enabled.is_true()
    }

    /// Translation of `SDL_SetTrayEntryCallback()`.
    fn set_callback(&self, entry: EntryKey, callback: TrayCallback) {
        if let Some(entry) = find_entry(&mut self.state(), entry) {
            entry.callback = Some(callback);
        }
    }

    /// Translation of `SDL_ClickTrayEntry()`.
    fn click(&self, entry: EntryKey) {
        let is_checkbox = find_entry(&mut self.state(), entry)
            .map(|e| e.flags.contains(TrayEntryFlags::CHECKBOX));
        let Some(is_checkbox) = is_checkbox else {
            return;
        };

        if is_checkbox {
            self.set_checked(entry, !self.checked(entry));
        }

        self.run_entry_callback(entry);
    }
}

/// (the Cocoa trays have no driver)
pub(super) fn destroy_driver() {}
