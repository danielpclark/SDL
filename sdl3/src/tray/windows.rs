// Rust translation of src/tray/windows/SDL_tray.c from Simple DirectMedia
// Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Windows tray: a notification area icon (`Shell_NotifyIconW()`)
//! owned by a hidden window whose window procedure handles the icon's
//! clicks and the menu's commands, with the menus as Win32 popup menus.

use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Weak};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIM_ADD, NIM_DELETE,
    NIM_MODIFY, NIM_SETVERSION, NOTIFYICONDATAW, NOTIFYICON_VERSION_4,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CheckMenuItem, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DeleteMenu, DestroyIcon,
    DestroyMenu, DestroyWindow, EnableMenuItem, GetClassInfoExW, GetMenuItemInfoW,
    GetWindowLongPtrW, InsertMenuW, LoadIconW, RegisterClassExW, RegisterWindowMessageW,
    SetForegroundWindow, SetMenuItemInfoW, SetWindowLongPtrW, TrackPopupMenu, UnregisterClassW,
    CW_USEDEFAULT, GWLP_USERDATA, HICON, HMENU, IDI_APPLICATION, MENUITEMINFOW, MFS_CHECKED,
    MFS_ENABLED, MF_BYCOMMAND, MF_BYPOSITION, MF_CHECKED, MF_DISABLED, MF_ENABLED, MF_GRAYED,
    MF_POPUP, MF_SEPARATOR, MF_STRING, MF_UNCHECKED, MIIM_STATE, MIIM_STRING, TPM_BOTTOMALIGN,
    TPM_RIGHTALIGN, WM_COMMAND, WM_CONTEXTMENU, WM_LBUTTONUP, WM_MBUTTONUP, WM_SETTINGCHANGE,
    WM_USER, WNDCLASSEXW, WS_OVERLAPPEDWINDOW,
};

use super::{
    EntryKey, TrayBackend, TrayCallback, TrayClickCallback, TrayEntry, TrayEntryFlags, TrayOptions,
    TrayRef,
};
use crate::core::windows::{create_icon_from_surface, update_dark_mode_for_hwnd, utf8_to_wide};
use crate::error::{Error, Result};
use crate::properties::Properties;
use crate::video::surface::Surface;

const WM_TRAYICON: u32 = WM_USER + 1;

/// Translation of `SDL_PROP_TRAY_CLEANUP`.
const PROP_TRAY_CLEANUP: &str = "SDL.internal.tray.cleanup";

/// The size of an entry's `label_cache`.
const LABEL_CACHE_SIZE: usize = 4096;

/// Translation of `struct SDL_TrayMenu` (its parent tray is the tray that
/// holds it, when `parent_entry` is `None`).
struct Menu {
    h_menu: HMENU,
    entries: Vec<Entry>,
    parent_entry: Option<EntryKey>,
}

/// Translation of `struct SDL_TrayEntry` (its parent menu is the menu that
/// holds it).
struct Entry {
    id: usize,
    label_cache: String,
    flags: TrayEntryFlags,
    callback: Option<TrayCallback>,
    submenu: Option<Box<Menu>>,
}

/// Translation of the rest of `struct SDL_Tray`.
struct TrayState {
    nid: NOTIFYICONDATAW,
    hwnd: HWND,
    icon: HICON,
    menu: Option<Box<Menu>>,

    left_click_callback: Option<TrayClickCallback>,
    right_click_callback: Option<TrayClickCallback>,
    middle_click_callback: Option<TrayClickCallback>,
}

// SAFETY: the window, menu and icon handles are process-wide user object
// handles, used behind the mutex; the window procedure runs on the window's
// thread and takes the mutex too.
unsafe impl Send for TrayState {}

/// Translation of `struct SDL_Tray`.
pub(super) struct TrayWin {
    this: Weak<TrayWin>,
    state: Mutex<TrayState>,
}

impl TrayWin {
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
    fn run_entry_callback(&self, id: usize) {
        let callback = find_entry_in_state(&mut self.state(), id).and_then(|e| e.callback.take());
        if let Some(mut callback) = callback {
            callback(&self.entry_ref(id as EntryKey));
            if let Some(entry) = find_entry_in_state(&mut self.state(), id) {
                // (unless the callback set a new one)
                if entry.callback.is_none() {
                    entry.callback = Some(callback);
                }
            }
        }
    }

    /// Run a click callback, by which of the tray's callbacks it is.
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

/// Translation of `get_next_id()`.
fn get_next_id() -> usize {
    static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
    NEXT_ID.fetch_add(1, Ordering::Relaxed) + 1
}

/// The menu holding the entry `id`, searched from `menu`.
/// Translation of `find_entry_in_menu()`, returning the parent.
fn find_parent_in_menu(menu: &mut Menu, id: usize) -> Option<&mut Menu> {
    if menu.entries.iter().any(|e| e.id == id) {
        return Some(menu);
    }
    for entry in &mut menu.entries {
        if let Some(submenu) = &mut entry.submenu {
            if let Some(m) = find_parent_in_menu(submenu, id) {
                return Some(m);
            }
        }
    }
    None
}

/// Translation of `find_entry_with_id()`.
fn find_entry_in_state(state: &mut TrayState, id: usize) -> Option<&mut Entry> {
    let menu = state.menu.as_deref_mut()?;
    find_parent_in_menu(menu, id)?
        .entries
        .iter_mut()
        .find(|e| e.id == id)
}

/// The tray's menu (`None`) or an entry's submenu.
fn find_menu(state: &mut TrayState, parent: Option<EntryKey>) -> Option<&mut Menu> {
    match parent {
        None => state.menu.as_deref_mut(),
        Some(key) => find_entry_in_state(state, key as usize)?
            .submenu
            .as_deref_mut(),
    }
}

/// The entry `id` and the menu handle of its parent.
fn find_entry_and_parent(state: &mut TrayState, id: usize) -> Option<(&mut Entry, HMENU)> {
    let menu = state.menu.as_deref_mut()?;
    let parent = find_parent_in_menu(menu, id)?;
    let h_menu = parent.h_menu;
    let entry = parent.entries.iter_mut().find(|e| e.id == id)?;
    Some((entry, h_menu))
}

/// Translation of `TrayWindowProc()`.
unsafe extern "system" fn tray_window_proc(
    hwnd: HWND,
    u_msg: u32,
    w_param: WPARAM,
    l_param: LPARAM,
) -> LRESULT {
    // SAFETY: the user data is NULL or the Weak<TrayWin> set at creation,
    // which lives until the window is destroyed.
    let tray = unsafe {
        let data = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Weak<TrayWin>;
        data.as_ref().and_then(Weak::upgrade)
    };

    let Some(tray) = tray else {
        // SAFETY: the default handling of the window's message.
        return unsafe { DefWindowProcW(hwnd, u_msg, w_param, l_param) };
    };

    static S_TASKBAR_RESTART: AtomicU32 = AtomicU32::new(0);
    if S_TASKBAR_RESTART.load(Ordering::Relaxed) == 0 {
        let name = utf8_to_wide("TaskbarCreated");
        // SAFETY: the name is NUL-terminated.
        let message = unsafe { RegisterWindowMessageW(name.as_ptr()) };
        S_TASKBAR_RESTART.store(message, Ordering::Relaxed);
    }
    if u_msg == S_TASKBAR_RESTART.load(Ordering::Relaxed) {
        let state = tray.state();
        // SAFETY: the icon data is valid for the calls.
        unsafe {
            Shell_NotifyIconW(NIM_ADD, &state.nid);
            Shell_NotifyIconW(NIM_SETVERSION, &state.nid);
        }
    }

    match u_msg {
        WM_TRAYICON => {
            let mut show_menu = false;

            match (l_param & 0xFFFF) as u32 {
                WM_LBUTTONUP => {
                    show_menu = tray
                        .run_click_callback(|s| &mut s.left_click_callback)
                        .unwrap_or(true);
                }

                WM_CONTEXTMENU => {
                    show_menu = tray
                        .run_click_callback(|s| &mut s.right_click_callback)
                        .unwrap_or(true);
                }

                WM_MBUTTONUP => {
                    tray.run_click_callback(|s| &mut s.middle_click_callback);
                }
                _ => {}
            }

            let h_menu = tray.state().menu.as_ref().map(|m| m.h_menu);
            if let (true, Some(h_menu)) = (show_menu, h_menu) {
                // (GET_X_LPARAM() and GET_Y_LPARAM() of wParam)
                let x = i32::from(w_param as u16 as i16);
                let y = i32::from((w_param >> 16) as u16 as i16);
                // SAFETY: the window and menu are live.
                unsafe {
                    SetForegroundWindow(hwnd);
                    TrackPopupMenu(
                        h_menu,
                        TPM_BOTTOMALIGN | TPM_RIGHTALIGN,
                        x,
                        y,
                        0,
                        hwnd,
                        std::ptr::null(),
                    );
                }
            }
        }

        WM_COMMAND => {
            let id = w_param & 0xFFFF;
            let is_checkbox = find_entry_in_state(&mut tray.state(), id)
                .map(|e| e.flags.contains(TrayEntryFlags::CHECKBOX));

            if is_checkbox == Some(true) {
                tray.set_checked(id as EntryKey, !tray.checked(id as EntryKey));
            }

            if is_checkbox.is_some() {
                tray.run_entry_callback(id);
            }
        }

        WM_SETTINGCHANGE => {
            if w_param == 0 && l_param != 0 {
                let setting = utf8_to_wide("ImmersiveColorSet");
                // SAFETY: for WM_SETTINGCHANGE, a nonzero lParam is a
                // NUL-terminated string.
                let matches = unsafe {
                    let p = l_param as *const u16;
                    setting.iter().enumerate().all(|(i, &c)| *p.add(i) == c)
                };
                if matches {
                    update_dark_mode_for_hwnd(hwnd);
                }
            }
        }

        _ => {
            // SAFETY: the default handling of the window's message.
            return unsafe { DefWindowProcW(hwnd, u_msg, w_param, l_param) };
        }
    }
    0
}

/// Translation of `DestroySDLMenu()`.
fn destroy_sdl_menu(menu: Menu) {
    let Menu {
        h_menu, entries, ..
    } = menu;
    for entry in entries {
        if let Some(submenu) = entry.submenu {
            destroy_sdl_menu(*submenu);
        }
    }
    // SAFETY: the menu handle was made by CreatePopupMenu() and is
    // destroyed once.
    unsafe { DestroyMenu(h_menu) };
}

/// The label with each `&` doubled (Win32 menus underline the character
/// after a single one), in UTF-16. Translation of `escape_label()`.
fn escape_label(input: &str) -> Vec<u16> {
    utf8_to_wide(&input.replace('&', "&&"))
}

/// `SDL_snprintf(entry->label_cache, sizeof(entry->label_cache), "%s", label)`.
fn label_cache(label: &str) -> String {
    let mut end = label.len().min(LABEL_CACHE_SIZE - 1);
    while !label.is_char_boundary(end) {
        end -= 1;
    }
    label[..end].to_string()
}

/// Translation of `load_default_icon()`.
fn load_default_icon() -> HICON {
    // SAFETY: GetModuleHandleW(NULL) is this executable; LoadIconW takes a
    // module and a resource ID (MAKEINTRESOURCE).
    unsafe {
        let h_instance = GetModuleHandleW(std::ptr::null());
        if h_instance.is_null() {
            return LoadIconW(std::ptr::null_mut(), IDI_APPLICATION);
        }

        for hint in [
            crate::hints::WINDOWS_INTRESOURCE_ICON_SMALL,
            crate::hints::WINDOWS_INTRESOURCE_ICON,
        ] {
            if let Some(hint) = crate::hints::get(hint).filter(|h| !h.is_empty()) {
                let id = crate::stdlib::atoi(&hint) as u16 as usize;
                let icon = LoadIconW(h_instance, id as *const u16);
                return if icon.is_null() {
                    LoadIconW(std::ptr::null_mut(), IDI_APPLICATION)
                } else {
                    icon
                };
            }
        }

        LoadIconW(std::ptr::null_mut(), IDI_APPLICATION)
    }
}

/// Translation of `SDL_UpdateTrays()`.
pub(super) fn update_trays() {}

/// The tray window class, unregistered when the global properties go
/// away. Translation of `SDL_PropTrayCleanupCb()`.
struct TrayClassCleanup(Vec<u16>);

impl Drop for TrayClassCleanup {
    fn drop(&mut self) {
        // SAFETY: WNDCLASSEXW is plain data; the class name is
        // NUL-terminated.
        unsafe {
            let mut wcex: WNDCLASSEXW = std::mem::zeroed();
            // FIXME (upstream): wcex.cbSize isn't set for GetClassInfoEx().
            let h = GetModuleHandleW(std::ptr::null());
            if GetClassInfoExW(h, self.0.as_ptr(), &mut wcex) != 0 {
                UnregisterClassW(self.0.as_ptr(), h);
            }
        }
    }
}

/// Translation of `SDL_RegisterTrayClass()`.
fn register_tray_class(class_name: &[u16]) -> Result<()> {
    let props = Properties::global();
    let mut locked = props.lock();
    if locked.get(PROP_TRAY_CLEANUP).is_some() {
        return Ok(());
    }

    // SAFETY: WNDCLASSEXW is plain data; all zeroes is valid.
    let mut wcex: WNDCLASSEXW = unsafe { std::mem::zeroed() };

    wcex.cbSize = size_of::<WNDCLASSEXW>() as u32;
    wcex.lpszClassName = class_name.as_ptr();
    wcex.style = 0;
    wcex.lpfnWndProc = Some(tray_window_proc);
    wcex.hInstance = std::ptr::null_mut();
    wcex.cbClsExtra = 0;
    wcex.cbWndExtra = 0;

    // SAFETY: the class name outlives the call (the system copies it).
    if unsafe { RegisterClassExW(&wcex) } == 0 {
        return Err(Error::new("Couldn't register tray class"));
    }

    locked.set(
        PROP_TRAY_CLEANUP,
        crate::properties::Value::any(TrayClassCleanup(class_name.to_vec())),
    )
}

/// Translation of `SDL_CreateTrayWithProperties()` (the caller checks the
/// thread and registers the tray).
pub(super) fn create_tray(options: TrayOptions<'_>) -> Result<Arc<dyn TrayBackend>> {
    if !crate::init::is_main_thread() {
        return Err(Error::new(
            "This function should be called on the main thread",
        ));
    }

    create_tray_on_any_thread(options)
}

/// [`create_tray`] without the main thread check.
fn create_tray_on_any_thread(options: TrayOptions<'_>) -> Result<Arc<dyn TrayBackend>> {
    let TrayOptions {
        icon,
        tooltip,
        left_click,
        right_click,
        middle_click,
    } = options;

    let class_name = utf8_to_wide("SDL_TRAY");
    if register_tray_class(&class_name).is_err() {
        return Err(Error::new("Failed to register SDL_TRAY window class"));
    }
    // SAFETY: the class is registered; the class name is NUL-terminated.
    let hwnd = unsafe {
        CreateWindowExW(
            0,
            class_name.as_ptr(),
            std::ptr::null(),
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            0,
            CW_USEDEFAULT,
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null(),
        )
    };
    // (upstream doesn't check for failure)

    update_dark_mode_for_hwnd(hwnd);

    // SAFETY: NOTIFYICONDATAW is plain data; all zeroes is valid.
    let mut nid: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
    nid.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = hwnd;
    nid.uID = get_next_id() as u32;
    nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP | NIF_SHOWTIP;
    nid.uCallbackMessage = WM_TRAYICON;
    nid.Anonymous.uVersion = NOTIFYICON_VERSION_4;
    if let Some(tooltip) = &tooltip {
        set_tip(&mut nid, &utf8_to_wide(tooltip));
    }

    nid.hIcon = match icon {
        Some(icon) => create_icon_from_surface(icon).unwrap_or_else(load_default_icon),
        None => load_default_icon(),
    };
    let tray_icon = nid.hIcon;

    // SAFETY: the icon data is valid for the calls.
    unsafe {
        Shell_NotifyIconW(NIM_ADD, &nid);
        Shell_NotifyIconW(NIM_SETVERSION, &nid);
    }

    let tray = Arc::new_cyclic(|this| TrayWin {
        this: this.clone(),
        state: Mutex::new(TrayState {
            nid,
            hwnd,
            icon: tray_icon,
            menu: None,
            left_click_callback: left_click,
            right_click_callback: right_click,
            middle_click_callback: middle_click,
        }),
    });

    // (the window's user data is a Weak<TrayWin>, freed in destroy())
    let data = Box::into_raw(Box::new(Arc::downgrade(&tray)));
    // SAFETY: the window is ours; the data lives until destroy().
    unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, data as isize) };

    Ok(tray)
}

/// `SDL_wcslcpy(nid.szTip, tooltipw, SDL_arraysize(nid.szTip))`.
fn set_tip(nid: &mut NOTIFYICONDATAW, tooltip: &[u16]) {
    let len = tooltip
        .iter()
        .position(|&c| c == 0)
        .unwrap_or(tooltip.len())
        .min(nid.szTip.len() - 1);
    nid.szTip[..len].copy_from_slice(&tooltip[..len]);
    nid.szTip[len] = 0;
}

/// A zeroed `MENUITEMINFOW` with its size set and `mask`.
fn menu_item_info(mask: u32) -> MENUITEMINFOW {
    // SAFETY: MENUITEMINFOW is plain data; all zeroes is valid.
    let mut mii: MENUITEMINFOW = unsafe { std::mem::zeroed() };
    mii.cbSize = size_of::<MENUITEMINFOW>() as u32;
    mii.fMask = mask;
    mii
}

/// The state of a menu item, by command (`GetMenuItemInfoW()` with
/// `MIIM_STATE`).
fn menu_item_state(h_menu: HMENU, id: usize) -> u32 {
    let mut mii = menu_item_info(MIIM_STATE);
    // SAFETY: the menu is live; mii is valid for the call.
    unsafe { GetMenuItemInfoW(h_menu, id as u32, 0, &mut mii) };
    mii.fState
}

impl TrayBackend for TrayWin {
    fn update(&self) {}

    /// Translation of `SDL_DestroyTray()` (after `SDL_UnregisterTray()`).
    fn destroy(&self) {
        let mut state = self.state();

        // SAFETY: the icon data is valid for the call.
        unsafe { Shell_NotifyIconW(NIM_DELETE, &state.nid) };

        if let Some(menu) = state.menu.take() {
            destroy_sdl_menu(*menu);
        }

        if !state.icon.is_null() {
            // SAFETY: the icon is ours (or a shared one, which this leaves).
            unsafe { DestroyIcon(state.icon) };
            state.icon = std::ptr::null_mut();
        }

        if !state.hwnd.is_null() {
            let hwnd = state.hwnd;
            state.hwnd = std::ptr::null_mut();
            drop(state);
            // SAFETY: the window is ours; its user data is the Weak set at
            // creation, freed once the window can't use it any more.
            unsafe {
                let data = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Weak<TrayWin>;
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                DestroyWindow(hwnd);
                if !data.is_null() {
                    drop(Box::from_raw(data));
                }
            }
        }
    }

    /// Translation of `SDL_SetTrayIcon()`.
    fn set_icon(&self, icon: Option<&Surface<'_>>) {
        let mut state = self.state();
        if !state.icon.is_null() {
            // SAFETY: the old icon is ours.
            unsafe { DestroyIcon(state.icon) };
        }

        state.nid.hIcon = match icon {
            Some(icon) => create_icon_from_surface(icon).unwrap_or_else(load_default_icon),
            None => load_default_icon(),
        };
        state.icon = state.nid.hIcon;

        // SAFETY: the icon data is valid for the call.
        unsafe { Shell_NotifyIconW(NIM_MODIFY, &state.nid) };
    }

    /// Translation of `SDL_SetTrayTooltip()`.
    fn set_tooltip(&self, tooltip: Option<&str>) {
        let mut state = self.state();
        match tooltip {
            Some(tooltip) => set_tip(&mut state.nid, &utf8_to_wide(tooltip)),
            None => state.nid.szTip[0] = 0,
        }

        // SAFETY: the icon data is valid for the call.
        unsafe { Shell_NotifyIconW(NIM_MODIFY, &state.nid) };
    }

    /// Translation of `SDL_CreateTrayMenu()`.
    fn create_menu(&self) -> Result<()> {
        let mut state = self.state();
        // FIXME (upstream): the previous menu (if any) is leaked; here it
        // is destroyed.
        if let Some(old) = state.menu.take() {
            destroy_sdl_menu(*old);
        }
        state.menu = Some(Box::new(Menu {
            // SAFETY: CreatePopupMenu has no preconditions.
            h_menu: unsafe { CreatePopupMenu() },
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

        let mut windows_compatible_pos = pos;

        if pos == -1 {
            pos = n_entries;
        } else if pos == n_entries {
            windows_compatible_pos = -1;
        }

        let label_w = label.map(escape_label);

        let mut entry = Entry {
            id: 0,
            label_cache: label_cache(label.unwrap_or("")),
            flags,
            callback: None,
            submenu: None,
        };

        if label.is_some() && flags.contains(TrayEntryFlags::SUBMENU) {
            // SAFETY: CreatePopupMenu has no preconditions.
            let h_menu = unsafe { CreatePopupMenu() };
            entry.id = h_menu as usize;
            entry.submenu = Some(Box::new(Menu {
                h_menu,
                entries: Vec::new(),
                parent_entry: Some(entry.id as EntryKey),
            }));
        } else {
            entry.id = get_next_id();
        }
        let id = entry.id;
        menu.entries.insert(pos as usize, entry);

        // SAFETY: the menu is live; the label is NUL-terminated and
        // outlives the call (the system copies it).
        unsafe {
            match &label_w {
                None => {
                    InsertMenuW(
                        menu.h_menu,
                        windows_compatible_pos as u32,
                        MF_SEPARATOR | MF_BYPOSITION,
                        id,
                        std::ptr::null(),
                    );
                }
                Some(label_w) => {
                    let mut mf = MF_STRING | MF_BYPOSITION;
                    if flags.contains(TrayEntryFlags::SUBMENU) {
                        // FIXME (upstream): this drops MF_BYPOSITION, so
                        // the position is taken as a command ID.
                        mf = MF_POPUP;
                    }

                    if flags.contains(TrayEntryFlags::DISABLED) {
                        mf |= MF_DISABLED | MF_GRAYED;
                    }

                    if flags.contains(TrayEntryFlags::CHECKED) {
                        mf |= MF_CHECKED;
                    }

                    InsertMenuW(
                        menu.h_menu,
                        windows_compatible_pos as u32,
                        mf,
                        id,
                        label_w.as_ptr(),
                    );
                }
            }
        }

        Ok(id as EntryKey)
    }

    /// Translation of `SDL_CreateTraySubmenu()`.
    fn create_submenu(&self, entry: EntryKey) -> Result<()> {
        let mut state = self.state();
        let Some(entry) = find_entry_in_state(&mut state, entry as usize) else {
            return Err(Error::invalid_param("entry"));
        };

        if entry.submenu.is_none() {
            return Err(Error::new(
                "Cannot create submenu for entry not created with SDL_TRAYENTRY_SUBMENU",
            ));
        }
        Ok(())
    }

    /// Translation of `SDL_GetTraySubmenu()`.
    fn has_submenu(&self, entry: EntryKey) -> Option<bool> {
        find_entry_in_state(&mut self.state(), entry as usize).map(|e| e.submenu.is_some())
    }

    /// Translation of `SDL_GetTrayEntries()`.
    fn entries(&self, menu: Option<EntryKey>) -> Result<Vec<EntryKey>> {
        let mut state = self.state();
        let menu = find_menu(&mut state, menu).ok_or_else(|| Error::invalid_param("menu"))?;
        Ok(menu.entries.iter().map(|e| e.id as EntryKey).collect())
    }

    /// Translation of `SDL_GetTrayEntryParent()`.
    fn entry_parent(&self, entry: EntryKey) -> Option<Option<EntryKey>> {
        let mut state = self.state();
        let menu = state.menu.as_deref_mut()?;
        find_parent_in_menu(menu, entry as usize).map(|m| m.parent_entry)
    }

    /// Translation of `SDL_RemoveTrayEntry()`.
    fn remove_entry(&self, entry: EntryKey) {
        let mut state = self.state();
        let id = entry as usize;
        let Some(menu) = state
            .menu
            .as_deref_mut()
            .and_then(|m| find_parent_in_menu(m, id))
        else {
            return;
        };

        let Some(i) = menu.entries.iter().position(|e| e.id == id) else {
            return;
        };
        let mut entry = menu.entries.remove(i);

        if let Some(submenu) = entry.submenu.take() {
            destroy_sdl_menu(*submenu);
        }

        // SAFETY: the menu is live.
        if unsafe { DeleteMenu(menu.h_menu, id as u32, MF_BYCOMMAND) } == 0 {
            /* This is somewhat useless since we don't return anything, but might help with eventual bugs */
            let _ = Error::new("Couldn't destroy tray entry");
        }
    }

    /// Translation of `SDL_SetTrayEntryLabel()`.
    fn set_label(&self, entry: EntryKey, label: Option<&str>) {
        let mut state = self.state();
        let Some((entry, h_menu)) = find_entry_and_parent(&mut state, entry as usize) else {
            return;
        };

        // (upstream formats a NULL label as "%s" and escapes it, which
        // isn't defined; an empty label here)
        let label = label.unwrap_or("");
        entry.label_cache = label_cache(label);

        let mut label_w = escape_label(label);

        let mut mii = menu_item_info(MIIM_STRING);

        mii.dwTypeData = label_w.as_mut_ptr();
        mii.cch = (label_w.len() - 1) as u32;

        // SAFETY: the menu is live; the label outlives the call.
        if unsafe { SetMenuItemInfoW(h_menu, entry.id as u32, 0, &mii) } == 0 {
            let _ = Error::new("Couldn't update tray entry label");
        }
    }

    /// Translation of `SDL_GetTrayEntryLabel()`.
    fn label(&self, entry: EntryKey) -> Option<String> {
        find_entry_in_state(&mut self.state(), entry as usize).map(|e| e.label_cache.clone())
    }

    /// Translation of `SDL_SetTrayEntryChecked()`.
    fn set_checked(&self, entry: EntryKey, checked: bool) {
        let mut state = self.state();
        let Some((entry, h_menu)) = find_entry_and_parent(&mut state, entry as usize) else {
            return;
        };
        if !entry.flags.contains(TrayEntryFlags::CHECKBOX) {
            return;
        }

        // SAFETY: the menu is live.
        unsafe {
            CheckMenuItem(
                h_menu,
                entry.id as u32,
                if checked { MF_CHECKED } else { MF_UNCHECKED },
            )
        };
    }

    /// Translation of `SDL_GetTrayEntryChecked()`.
    fn checked(&self, entry: EntryKey) -> bool {
        let mut state = self.state();
        let Some((entry, h_menu)) = find_entry_and_parent(&mut state, entry as usize) else {
            return false;
        };
        if !entry.flags.contains(TrayEntryFlags::CHECKBOX) {
            return false;
        }

        menu_item_state(h_menu, entry.id) & MFS_CHECKED != 0
    }

    /// Translation of `SDL_SetTrayEntryEnabled()`.
    fn set_enabled(&self, entry: EntryKey, enabled: bool) {
        let mut state = self.state();
        let Some((entry, h_menu)) = find_entry_and_parent(&mut state, entry as usize) else {
            return;
        };

        // SAFETY: the menu is live.
        unsafe {
            EnableMenuItem(
                h_menu,
                entry.id as u32,
                MF_BYCOMMAND
                    | if enabled {
                        MF_ENABLED
                    } else {
                        MF_DISABLED | MF_GRAYED
                    },
            )
        };
    }

    /// Translation of `SDL_GetTrayEntryEnabled()`.
    #[allow(clippy::bad_bit_mask)] // (see the FIXME)
    fn enabled(&self, entry: EntryKey) -> bool {
        let mut state = self.state();
        let Some((entry, h_menu)) = find_entry_and_parent(&mut state, entry as usize) else {
            return false;
        };

        // FIXME (upstream): MFS_ENABLED is 0, so this always reports the
        // entry disabled.
        menu_item_state(h_menu, entry.id) & MFS_ENABLED != 0
    }

    /// Translation of `SDL_SetTrayEntryCallback()`.
    fn set_callback(&self, entry: EntryKey, callback: TrayCallback) {
        if let Some(entry) = find_entry_in_state(&mut self.state(), entry as usize) {
            entry.callback = Some(callback);
        }
    }

    /// Translation of `SDL_ClickTrayEntry()`.
    fn click(&self, entry: EntryKey) {
        let is_checkbox = find_entry_in_state(&mut self.state(), entry as usize)
            .map(|e| e.flags.contains(TrayEntryFlags::CHECKBOX));
        let Some(is_checkbox) = is_checkbox else {
            return;
        };

        if is_checkbox {
            self.set_checked(entry, !self.checked(entry));
        }

        self.run_entry_callback(entry as usize);
    }
}

/// (the Windows trays have no driver)
pub(super) fn destroy_driver() {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tray::Tray;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetMenuItemCount, GetMenuStringW, GetSubMenu, IsWindow, IsWindowVisible, SendMessageW,
        MFS_DISABLED,
    };

    /// A tray made off the main thread (as the tests run).
    fn test_tray(options: TrayOptions<'_>) -> Tray {
        Tray::from_backend(create_tray_on_any_thread(options).unwrap())
    }

    fn hwnd(tray: &Tray) -> HWND {
        backend(tray).state().hwnd
    }

    fn backend(tray: &Tray) -> Arc<TrayWin> {
        let b = tray.inner.clone();
        // SAFETY: the test trays are TrayWins.
        unsafe { Arc::from_raw(Arc::into_raw(b).cast::<TrayWin>()) }
    }

    fn h_menu(tray: &Tray, parent: Option<EntryKey>) -> HMENU {
        let b = backend(tray);
        let mut state = b.state();
        find_menu(&mut state, parent).unwrap().h_menu
    }

    /// The text of the item at `pos` of a Win32 menu.
    fn menu_string(h_menu: HMENU, pos: u32) -> String {
        let mut buf = [0u16; 256];
        // SAFETY: the menu is live; the buffer holds its length.
        let n = unsafe {
            GetMenuStringW(
                h_menu,
                pos,
                buf.as_mut_ptr(),
                buf.len() as i32,
                MF_BYPOSITION,
            )
        };
        String::from_utf16_lossy(&buf[..n.max(0) as usize])
    }

    #[test]
    fn labels_and_tips() {
        assert_eq!(escape_label("a&b&&c"), utf8_to_wide("a&&b&&&&c"));
        assert_eq!(escape_label(""), [0]);
        let long = "é".repeat(3000);
        let cached = label_cache(&long);
        assert_eq!(cached.len(), 4094);
        assert_eq!(label_cache("x"), "x");

        // SAFETY: NOTIFYICONDATAW is plain data.
        let mut nid: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
        set_tip(&mut nid, &utf8_to_wide("tip"));
        assert_eq!(&nid.szTip[..4], &utf8_to_wide("tip")[..]);
        set_tip(&mut nid, &utf8_to_wide(&"x".repeat(200)));
        assert_eq!(nid.szTip[126], u16::from(b'x'));
        assert_eq!(nid.szTip[127], 0);
    }

    #[test]
    fn main_thread_only() {
        if crate::init::is_main_thread() {
            return;
        }
        assert_eq!(
            Tray::create(None, Some("tip")).unwrap_err().to_string(),
            "This function should be called on the main thread"
        );
    }

    #[test]
    fn tray_menus_and_entries() {
        // (SDL_Quit() destroys the trays)
        let _l = crate::test_support::test_lock();
        let tray = test_tray(TrayOptions {
            tooltip: Some("SDL tray test".to_string()),
            ..TrayOptions::default()
        });
        let window = hwnd(&tray);
        // SAFETY: IsWindow and IsWindowVisible accept any handle.
        unsafe {
            assert!(IsWindow(window) != 0);
            assert!(IsWindowVisible(window) == 0);
        }
        tray.set_tooltip(Some("new tip"));
        tray.set_tooltip(None);
        let icon = Surface::new(16, 16, crate::video::pixels::PixelFormat::ARGB8888).unwrap();
        tray.set_icon(Some(&icon));
        tray.set_icon(None);

        assert!(tray.menu().is_none());
        let menu = tray.create_menu().unwrap();
        assert_eq!(tray.menu().unwrap(), menu);
        assert_eq!(menu.parent_tray(), tray.handle());
        assert!(menu.parent_entry().is_none());

        let quit = menu
            .insert_entry(-1, Some("Quit & exit"), TrayEntryFlags::BUTTON)
            .unwrap();
        let check = menu
            .insert_entry(
                0,
                Some("Check"),
                TrayEntryFlags::CHECKBOX | TrayEntryFlags::CHECKED,
            )
            .unwrap();
        let sep = menu.insert_entry(1, None, TrayEntryFlags::BUTTON).unwrap();
        let disabled = menu
            .insert_entry(
                3,
                Some("Disabled"),
                TrayEntryFlags::BUTTON | TrayEntryFlags::DISABLED,
            )
            .unwrap();
        assert_eq!(
            menu.insert_entry(9, Some("x"), TrayEntryFlags::BUTTON)
                .unwrap_err()
                .to_string(),
            "Parameter 'pos' is invalid"
        );
        assert_eq!(
            menu.entries().unwrap(),
            [check.clone(), sep.clone(), quit.clone(), disabled.clone()]
        );

        // The Win32 menu matches.
        let hm = h_menu(&tray, None);
        // SAFETY: the menu is live.
        assert_eq!(unsafe { GetMenuItemCount(hm) }, 4);
        assert_eq!(menu_string(hm, 0), "Check");
        assert_eq!(menu_string(hm, 2), "Quit && exit");
        assert_eq!(menu_string(hm, 3), "Disabled");
        assert_eq!(quit.label().unwrap().as_deref(), Some("Quit & exit"));
        assert_eq!(sep.label().unwrap().as_deref(), Some(""));
        assert_ne!(menu_item_state(hm, disabled.key as usize) & MFS_DISABLED, 0);
        assert_eq!(menu_item_state(hm, quit.key as usize) & MFS_DISABLED, 0);
        // (FIXME (upstream): enabled() always reports false)
        assert!(!quit.enabled().unwrap());
        disabled.set_enabled(true);
        assert_eq!(menu_item_state(hm, disabled.key as usize) & MFS_DISABLED, 0);
        quit.set_enabled(false);
        assert_ne!(menu_item_state(hm, quit.key as usize) & MFS_DISABLED, 0);

        quit.set_label(Some("Leave"));
        assert_eq!(menu_string(hm, 2), "Leave");
        assert_eq!(quit.label().unwrap().as_deref(), Some("Leave"));

        // Checkboxes.
        assert!(check.checked().unwrap());
        check.set_checked(false);
        assert!(!check.checked().unwrap());
        assert!(!quit.checked().unwrap());
        quit.set_checked(true);
        assert!(!quit.checked().unwrap());

        // Clicks, simulated and through WM_COMMAND.
        let clicks = Arc::new(AtomicU32::new(0));
        let c = clicks.clone();
        let check2 = check.clone();
        check.set_callback(move |entry| {
            assert_eq!(*entry, check2);
            // (the entry can be used from its callback)
            let _ = entry.checked();
            c.fetch_add(1, Ordering::Relaxed);
        });
        check.click();
        assert!(check.checked().unwrap());
        assert_eq!(clicks.load(Ordering::Relaxed), 1);
        // SAFETY: the window is ours; WM_COMMAND with an entry's ID.
        unsafe { SendMessageW(window, WM_COMMAND, check.key as usize, 0) };
        assert!(!check.checked().unwrap());
        assert_eq!(clicks.load(Ordering::Relaxed), 2);
        // An unknown command is ignored.
        // SAFETY: as above.
        unsafe { SendMessageW(window, WM_COMMAND, 0xFFFF, 0) };
        assert_eq!(clicks.load(Ordering::Relaxed), 2);

        // Submenus.
        assert_eq!(
            quit.create_submenu().unwrap_err().to_string(),
            "Cannot create submenu for entry not created with SDL_TRAYENTRY_SUBMENU"
        );
        assert!(quit.submenu().is_none());
        let more = menu
            .insert_entry(-1, Some("More"), TrayEntryFlags::SUBMENU)
            .unwrap();
        let submenu = more.create_submenu().unwrap();
        assert_eq!(more.submenu().unwrap(), submenu);
        assert_eq!(submenu.parent_entry().unwrap(), more);
        let inner = submenu
            .insert_entry(-1, Some("Inner"), TrayEntryFlags::CHECKBOX)
            .unwrap();
        assert_eq!(inner.parent().unwrap(), submenu);
        assert_eq!(check.parent().unwrap(), menu);
        // SAFETY: the menus are live.
        unsafe {
            assert_eq!(GetMenuItemCount(hm), 5);
            assert_eq!(GetSubMenu(hm, 4), h_menu(&tray, Some(more.key)));
            assert_eq!(GetMenuItemCount(h_menu(&tray, Some(more.key))), 1);
        }
        assert_eq!(menu_string(hm, 4), "More");
        inner.click();
        assert!(inner.checked().unwrap());

        // Removal (with the submenu).
        more.clone().remove();
        assert!(inner.label().is_err());
        assert!(more.label().is_err());
        sep.remove();
        // SAFETY: the menu is live.
        assert_eq!(unsafe { GetMenuItemCount(hm) }, 3);
        assert_eq!(menu.entries().unwrap(), [check.clone(), quit, disabled]);

        drop(tray);
        assert!(check.label().is_err());
        assert!(menu.entries().is_err());
        // SAFETY: IsWindow accepts any handle.
        assert!(unsafe { IsWindow(window) } == 0);
    }

    #[test]
    fn click_callbacks() {
        let _l = crate::test_support::test_lock();
        let left = Arc::new(AtomicU32::new(0));
        let right = Arc::new(AtomicU32::new(0));
        let middle = Arc::new(AtomicU32::new(0));
        let (l, r, m) = (left.clone(), right.clone(), middle.clone());
        let tray = test_tray(TrayOptions {
            left_click: Some(Box::new(move |tray| {
                // (the tray can be used from its callbacks)
                tray.set_tooltip(Some("clicked"));
                l.fetch_add(1, Ordering::Relaxed);
                false
            })),
            right_click: Some(Box::new(move |_| {
                r.fetch_add(1, Ordering::Relaxed);
                false
            })),
            middle_click: Some(Box::new(move |_| {
                m.fetch_add(1, Ordering::Relaxed);
                true
            })),
            ..TrayOptions::default()
        });
        tray.create_menu().unwrap();
        let window = hwnd(&tray);
        // SAFETY: the window is ours; WM_TRAYICON as the shell sends it
        // (NOTIFYICON_VERSION_4: the event in LOWORD(lParam)). No menu is
        // shown: the callbacks return false, or it's the middle button.
        unsafe {
            SendMessageW(window, WM_TRAYICON, 0, WM_LBUTTONUP as isize);
            SendMessageW(window, WM_TRAYICON, 0, WM_CONTEXTMENU as isize);
            SendMessageW(window, WM_TRAYICON, 0, WM_MBUTTONUP as isize);
            SendMessageW(window, WM_TRAYICON, 0, WM_MBUTTONUP as isize);
        }
        assert_eq!(left.load(Ordering::Relaxed), 1);
        assert_eq!(right.load(Ordering::Relaxed), 1);
        assert_eq!(middle.load(Ordering::Relaxed), 2);
        let tip = backend(&tray).state().nid.szTip;
        assert_eq!(&tip[..8], &utf8_to_wide("clicked")[..]);
    }
}
