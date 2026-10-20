// Rust translation of src/notification/windows/SDL_windowsnotification.c
// from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Windows notifications: toasts through the Windows Runtime
//! (`Windows.UI.Notifications`, Windows 10 or later), described by toast
//! XML, with the app's name and icon registered under
//! `HKCU\SOFTWARE\Classes\AppUserModelId`. The WinRT interfaces aren't in
//! `windows-sys`, so their vtables are declared by hand.

// (the Win32 and WinRT names, as upstream spells them)
#![allow(non_camel_case_types, clippy::upper_case_acronyms)]

use std::ffi::c_void;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Mutex;

use windows_sys::core::{GUID, HRESULT, PCWSTR};
use windows_sys::Win32::Foundation::{
    ERROR_INSUFFICIENT_BUFFER, E_NOINTERFACE, E_POINTER, HWND, MAX_PATH, S_OK,
};
use windows_sys::Win32::Globalization::{MultiByteToWideChar, CP_UTF8};
use windows_sys::Win32::Storage::FileSystem::{
    DeleteFileW, GetFileAttributesExW, GetFileExInfoStandard, GetFullPathNameW, GetTempFileNameW,
    GetTempPathW, WIN32_FILE_ATTRIBUTE_DATA,
};
use windows_sys::Win32::System::LibraryLoader::{
    GetModuleFileNameW, GetModuleHandleW, GetProcAddress, LoadLibraryExW,
    LOAD_LIBRARY_SEARCH_SYSTEM32,
};
use windows_sys::Win32::System::Registry::{
    RegDeleteKeyValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{KillTimer, SetTimer};

use super::{Notification, NotificationAction, NotificationID, NotificationPriority};
use crate::core::windows::com::{ComObject, ComPtr, IUnknownVtbl, IID_IAGILEOBJECT, IID_IUNKNOWN};
use crate::core::windows::{
    co_initialize, co_uninitialize, error_from_hresult, is_equal_guid, load_combase_function,
    ro_initialize, ro_uninitialize, set_error, utf8_to_wide, wide_char_to_multi_byte, wide_to_utf8,
};
use crate::error::{Error, Result};
use crate::video::surface::Surface;

/// `HSTRING`
type HSTRING = *mut c_void;

/// `HSTRING_HEADER` (24 bytes on 64-bit Windows, 20 on 32-bit)
#[repr(C)]
struct HSTRING_HEADER {
    reserved: [usize; if cfg!(target_pointer_width = "64") {
        3
    } else {
        5
    }],
}

/// `EventRegistrationToken`
#[repr(C)]
#[derive(Default)]
struct EventRegistrationToken {
    value: i64,
}

type RoGetActivationFactory_t =
    unsafe extern "system" fn(HSTRING, *const GUID, *mut *mut c_void) -> HRESULT;
type RoActivateInstance_t = unsafe extern "system" fn(HSTRING, *mut *mut IInspectable) -> HRESULT;
type WindowsCreateString_t = unsafe extern "system" fn(PCWSTR, u32, *mut HSTRING) -> HRESULT;
type WindowsCreateStringReference_t =
    unsafe extern "system" fn(PCWSTR, u32, *mut HSTRING_HEADER, *mut HSTRING) -> HRESULT;
type WindowsDeleteString_t = unsafe extern "system" fn(HSTRING) -> HRESULT;
type WindowsGetStringRawBuffer_t = unsafe extern "system" fn(HSTRING, *mut u32) -> PCWSTR;
type BCryptGenRandom_t = unsafe extern "system" fn(*mut c_void, *mut u8, u32, u32) -> i32;
type GetCurrentPackageFullName_t = unsafe extern "system" fn(*mut u32, *mut u16) -> i32;

/// The combase functions (`WIN_RoGetActivationFactory` and the others).
#[derive(Clone, Copy)]
struct WinRt {
    ro_get_activation_factory: RoGetActivationFactory_t,
    ro_activate_instance: RoActivateInstance_t,
    windows_create_string: WindowsCreateString_t,
    windows_create_string_reference: WindowsCreateStringReference_t,
    windows_delete_string: WindowsDeleteString_t,
    windows_get_string_raw_buffer: WindowsGetStringRawBuffer_t,
}

// The registry key base needed to register the app instance so notifications can be sent.
const REG_KEY_BASE: &str = "SOFTWARE\\Classes\\AppUserModelId\\";
const GROUP_ID_STR: &str = "SDL_LocalNotification";

const RUNTIME_CLASS_TOAST_NOTIFICATION_MANAGER: &str =
    "Windows.UI.Notifications.ToastNotificationManager";
const RUNTIME_CLASS_TOAST_NOTIFICATION: &str = "Windows.UI.Notifications.ToastNotification";
const RUNTIME_CLASS_XML_DOCUMENT: &str = "Windows.Data.Xml.Dom.XmlDocument";

const fn guid(data1: u32, data2: u16, data3: u16, data4: [u8; 8]) -> GUID {
    GUID {
        data1,
        data2,
        data3,
        data4,
    }
}

// IIDs for the interfaces.
const IID_ITOAST_NOTIFICATION_MANAGER_STATICS: GUID = guid(
    0x50ac103f,
    0xd235,
    0x4598,
    [0xbb, 0xef, 0x98, 0xfe, 0x4d, 0x1a, 0x3a, 0xd4],
);
const IID_ITOAST_NOTIFICATION_MANAGER_STATICS2: GUID = guid(
    0x7ab93c52,
    0x0e48,
    0x4750,
    [0xba, 0x9d, 0x1a, 0x41, 0x13, 0x98, 0x18, 0x47],
);
const IID_ITOAST_NOTIFICATION_FACTORY: GUID = guid(
    0x04124b20,
    0x82c6,
    0x4229,
    [0xb1, 0x09, 0xfd, 0x9e, 0xd4, 0x66, 0x2b, 0x53],
);
const IID_ITOAST_NOTIFICATION2: GUID = guid(
    0x9dfb9fd1,
    0x143a,
    0x490e,
    [0x90, 0xbf, 0xb9, 0xfb, 0xa7, 0x13, 0x2d, 0xe7],
);
const IID_ITOAST_NOTIFICATION4: GUID = guid(
    0x15154935,
    0x28ea,
    0x4727,
    [0x88, 0xe9, 0xc5, 0x86, 0x80, 0xe2, 0xd1, 0x18],
);
#[allow(dead_code)] // (as upstream: defined, unused)
const IID_INOTIFICATION_ACTIVATION_CALLBACK: GUID = guid(
    0x53e31837,
    0x6600,
    0x4a81,
    [0x93, 0x95, 0x75, 0xcf, 0xfe, 0x74, 0x6f, 0x94],
);
const IID_IXML_DOCUMENT: GUID = guid(
    0xf7f3a506,
    0x1e87,
    0x42d6,
    [0xbc, 0xfb, 0xb8, 0xc8, 0x09, 0xfa, 0x54, 0x94],
);
const IID_IXML_DOCUMENT_IO: GUID = guid(
    0x6cd0e74e,
    0xee65,
    0x4489,
    [0x9e, 0xbf, 0xca, 0x43, 0xe8, 0x7b, 0xa6, 0x37],
);
const IID_ITOAST_ACTIVATED_EVENT_ARGS: GUID = guid(
    0xe3bf92f3,
    0xc197,
    0x436f,
    [0x82, 0x65, 0x06, 0x25, 0x82, 0x4f, 0x8d, 0xac],
);
const IID_ITOAST_ACTIVATED_EVENT_HANDLER: GUID = guid(
    0xab54de2d,
    0x97d9,
    0x5528,
    [0xb6, 0xad, 0x10, 0x5a, 0xfe, 0x15, 0x65, 0x30],
);
const IID_ITOAST_DISMISSED_EVENT_HANDLER: GUID = guid(
    0x61c2402f,
    0x0ed0,
    0x5a18,
    [0xab, 0x69, 0x59, 0xf4, 0xaa, 0x99, 0xa3, 0x68],
);

/// A method SDL doesn't call.
type Unused = unsafe extern "system" fn();

/// `IInspectableVtbl`
#[repr(C)]
struct IInspectableVtbl {
    base: IUnknownVtbl,
    get_iids: Unused,
    get_runtime_class_name: Unused,
    get_trust_level: Unused,
}
type IInspectable = ComObject<IInspectableVtbl>;

/// `IToastNotificationManagerStaticsVtbl`
#[repr(C)]
struct IToastNotificationManagerStaticsVtbl {
    base: IInspectableVtbl,
    create_toast_notifier: Unused,
    create_toast_notifier_with_id: unsafe extern "system" fn(
        *mut IToastNotificationManagerStatics,
        HSTRING,
        *mut *mut IToastNotifier,
    ) -> HRESULT,
    get_template_content: Unused,
}
type IToastNotificationManagerStatics = ComObject<IToastNotificationManagerStaticsVtbl>;

/// `IToastNotificationManagerStatics2Vtbl`
#[repr(C)]
struct IToastNotificationManagerStatics2Vtbl {
    base: IInspectableVtbl,
    get_history: unsafe extern "system" fn(
        *mut IToastNotificationManagerStatics2,
        *mut *mut IToastNotificationHistory,
    ) -> HRESULT,
}
type IToastNotificationManagerStatics2 = ComObject<IToastNotificationManagerStatics2Vtbl>;

/// `IToastNotificationHistoryVtbl`
#[repr(C)]
struct IToastNotificationHistoryVtbl {
    base: IInspectableVtbl,
    remove_group: Unused,
    remove_group_with_id: Unused,
    remove_grouped_tag_with_id: unsafe extern "system" fn(
        *mut IToastNotificationHistory,
        HSTRING,
        HSTRING,
        HSTRING,
    ) -> HRESULT,
    remove_grouped_tag: Unused,
    remove: Unused,
    clear: Unused,
    clear_with_id: Unused,
}
type IToastNotificationHistory = ComObject<IToastNotificationHistoryVtbl>;

/// `IToastNotifierVtbl`
#[repr(C)]
struct IToastNotifierVtbl {
    base: IInspectableVtbl,
    show: unsafe extern "system" fn(*mut IToastNotifier, *mut IToastNotification) -> HRESULT,
    hide: unsafe extern "system" fn(*mut IToastNotifier, *mut IToastNotification) -> HRESULT,
    get_setting: Unused,
    add_to_schedule: Unused,
    remove_from_schedule: Unused,
    get_scheduled_toast_notifications: Unused,
}
type IToastNotifier = ComObject<IToastNotifierVtbl>;

/// `IToastNotificationFactoryVtbl`
#[repr(C)]
struct IToastNotificationFactoryVtbl {
    base: IInspectableVtbl,
    create_toast_notification: unsafe extern "system" fn(
        *mut IToastNotificationFactory,
        *mut IInspectable,
        *mut *mut IToastNotification,
    ) -> HRESULT,
}
type IToastNotificationFactory = ComObject<IToastNotificationFactoryVtbl>;

/// `IToastNotificationVtbl`
#[repr(C)]
struct IToastNotificationVtbl {
    base: IInspectableVtbl,
    get_content: Unused,
    put_expiration_time: Unused,
    get_expiration_time: Unused,
    add_dismissed: unsafe extern "system" fn(
        *mut IToastNotification,
        *mut ToastEventHandler,
        *mut EventRegistrationToken,
    ) -> HRESULT,
    remove_dismissed: Unused,
    add_activated: unsafe extern "system" fn(
        *mut IToastNotification,
        *mut ToastEventHandler,
        *mut EventRegistrationToken,
    ) -> HRESULT,
    remove_activated: Unused,
    add_failed: Unused,
    remove_failed: Unused,
}
type IToastNotification = ComObject<IToastNotificationVtbl>;

/// `IToastNotification2Vtbl`
#[repr(C)]
struct IToastNotification2Vtbl {
    base: IInspectableVtbl,
    put_tag: unsafe extern "system" fn(*mut IToastNotification2, HSTRING) -> HRESULT,
    get_tag: unsafe extern "system" fn(*mut IToastNotification2, *mut HSTRING) -> HRESULT,
    put_group: unsafe extern "system" fn(*mut IToastNotification2, HSTRING) -> HRESULT,
    get_group: Unused,
    put_suppress_popup: Unused,
    get_suppress_popup: Unused,
}
type IToastNotification2 = ComObject<IToastNotification2Vtbl>;

/// `IToastNotification4Vtbl`
#[repr(C)]
struct IToastNotification4Vtbl {
    base: IInspectableVtbl,
    get_data: Unused,
    put_data: Unused,
    get_priority: Unused,
    put_priority: unsafe extern "system" fn(*mut IToastNotification4, i32) -> HRESULT,
}
type IToastNotification4 = ComObject<IToastNotification4Vtbl>;

/// `IXmlDocumentIOVtbl`
#[repr(C)]
struct IXmlDocumentIOVtbl {
    base: IInspectableVtbl,
    load_xml: unsafe extern "system" fn(*mut IXmlDocumentIO, HSTRING) -> HRESULT,
    load_xml_with_settings: Unused,
    save_to_file_async: Unused,
}
type IXmlDocumentIO = ComObject<IXmlDocumentIOVtbl>;

/// `IToastActivatedEventArgsVtbl`
#[repr(C)]
struct IToastActivatedEventArgsVtbl {
    base: IInspectableVtbl,
    get_arguments:
        unsafe extern "system" fn(*mut IToastActivatedEventArgs, *mut HSTRING) -> HRESULT,
}
type IToastActivatedEventArgs = ComObject<IToastActivatedEventArgsVtbl>;

/// `IToastDismissedEventArgsVtbl`
#[repr(C)]
struct IToastDismissedEventArgsVtbl {
    base: IInspectableVtbl,
    get_reason: unsafe extern "system" fn(*mut IToastDismissedEventArgs, *mut i32) -> HRESULT,
}
type IToastDismissedEventArgs = ComObject<IToastDismissedEventArgsVtbl>;

/// `ToastDismissalReason`
const TOAST_DISMISSAL_REASON_USER_CANCELED: i32 = 0;
const TOAST_DISMISSAL_REASON_TIMED_OUT: i32 = 2;

/// `ToastNotificationPriority`
const TOAST_NOTIFICATION_PRIORITY_DEFAULT: i32 = 0;
const TOAST_NOTIFICATION_PRIORITY_HIGH: i32 = 1;

/// `APPMODEL_ERROR_NO_PACKAGE`
const APPMODEL_ERROR_NO_PACKAGE: i32 = 15700;
/// `BCRYPT_USE_SYSTEM_PREFERRED_RNG`
const BCRYPT_USE_SYSTEM_PREFERRED_RNG: u32 = 0x00000002;

/// An owned `HSTRING`, deleted on drop.
struct HString(HSTRING, WindowsDeleteString_t);

// SAFETY: an HSTRING is an immutable, reference-counted string, usable from
// any thread.
unsafe impl Send for HString {}

impl Drop for HString {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: the string is ours, deleted once.
            unsafe { (self.1)(self.0) };
        }
    }
}

/// Translation of the `initialization_status` values.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum InitializationStatus {
    Failed,
    Uninitialized,
    Initialized,
}

/// The toast system's globals.
struct State {
    winrt: Option<WinRt>,
    hs_group_id: Option<HString>,
    hs_app_id: Option<HString>,

    // (released in this order: the reverse of their creation)
    toast_notification_history: Option<ComPtr<IToastNotificationHistoryVtbl>>,
    toast_notification_manager_statics2: Option<ComPtr<IToastNotificationManagerStatics2Vtbl>>,
    notification_factory: Option<ComPtr<IToastNotificationFactoryVtbl>>,
    toast_notifier: Option<ComPtr<IToastNotifierVtbl>>,
    toast_notification_manager: Option<ComPtr<IToastNotificationManagerStaticsVtbl>>,

    app_reg_key: Option<Vec<u16>>,
    app_icon_path: Option<Vec<u16>>,

    initialization_status: InitializationStatus,
    ro_initialized: bool,
    co_initialized: bool,

    /// Translation of `toast_icons` (the files to delete).
    toast_icons: Vec<Vec<u16>>,
    cleanup_timer_id: usize,
}

static STATE: Mutex<State> = Mutex::new(State {
    winrt: None,
    hs_group_id: None,
    hs_app_id: None,
    toast_notification_history: None,
    toast_notification_manager_statics2: None,
    notification_factory: None,
    toast_notifier: None,
    toast_notification_manager: None,
    app_reg_key: None,
    app_icon_path: None,
    initialization_status: InitializationStatus::Uninitialized,
    ro_initialized: false,
    co_initialized: false,
    toast_icons: Vec::new(),
    cleanup_timer_id: 0,
});

fn state() -> std::sync::MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

/// `WindowsCreateStringReference()` over `s` (NUL-terminated), whose
/// header and buffer must outlive the returned string.
///
/// # Safety
///
/// `s` must be NUL-terminated and outlive the string, as `header` must.
unsafe fn string_reference(
    winrt: &WinRt,
    s: &[u16],
    header: &mut HSTRING_HEADER,
) -> std::result::Result<HSTRING, HRESULT> {
    let mut hstring: HSTRING = std::ptr::null_mut();
    // SAFETY: the caller's contract.
    let hr = unsafe {
        (winrt.windows_create_string_reference)(
            s.as_ptr(),
            (s.len() - 1) as u32,
            header,
            &mut hstring,
        )
    };
    if hr < 0 {
        return Err(hr);
    }
    Ok(hstring)
}

fn new_header() -> HSTRING_HEADER {
    HSTRING_HEADER {
        reserved: [0; if cfg!(target_pointer_width = "64") {
            3
        } else {
            5
        }],
    }
}

// IUnknown implementation

/// Translation of `Impl_IGeneric`: a static event handler with a
/// reference count.
#[repr(C)]
struct ToastEventHandler {
    lp_vtbl: &'static ToastEventHandlerVtbl,
    ref_count: AtomicI32,
}

/// The vtable of a `TypedEventHandler<ToastNotification, T>`.
#[repr(C)]
struct ToastEventHandlerVtbl {
    query_interface:
        unsafe extern "system" fn(*mut ToastEventHandler, *const GUID, *mut *mut c_void) -> HRESULT,
    add_ref: unsafe extern "system" fn(*mut ToastEventHandler) -> u32,
    release: unsafe extern "system" fn(*mut ToastEventHandler) -> u32,
    invoke: unsafe extern "system" fn(
        *mut ToastEventHandler,
        *mut IToastNotification,
        *mut IInspectable,
    ) -> HRESULT,
}

impl ToastEventHandler {
    fn as_ptr(&'static self) -> *mut ToastEventHandler {
        (self as *const ToastEventHandler).cast_mut()
    }
}

/// Translation of `Impl_IGeneric_AddRef()`.
unsafe extern "system" fn impl_igeneric_add_ref(this: *mut ToastEventHandler) -> u32 {
    // SAFETY: `this` is one of the static handlers.
    (unsafe { &*this }.ref_count.fetch_add(1, Ordering::AcqRel) as u32).wrapping_add(1)
}

/// Translation of `Impl_IGeneric_Release()`.
unsafe extern "system" fn impl_igeneric_release(this: *mut ToastEventHandler) -> u32 {
    // SAFETY: `this` is one of the static handlers.
    (unsafe { &*this }.ref_count.fetch_sub(1, Ordering::AcqRel) as u32).wrapping_sub(1)
}

/// The `QueryInterface()` of a handler whose own IID is `iid`.
///
/// # Safety
///
/// `this` must be one of the static handlers; `ppv_object` NULL or valid.
unsafe fn handler_query_interface(
    this: *mut ToastEventHandler,
    riid: *const GUID,
    ppv_object: *mut *mut c_void,
    iid: &GUID,
) -> HRESULT {
    if ppv_object.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's contract; COM passes a valid IID.
    unsafe {
        if is_equal_guid(&*riid, iid)
            || is_equal_guid(&*riid, &IID_IAGILEOBJECT)
            || is_equal_guid(&*riid, &IID_IUNKNOWN)
        {
            *ppv_object = this.cast();
            ((*this).lp_vtbl.add_ref)(this);
            return S_OK;
        }
    }
    E_NOINTERFACE
}

// OnActivated interface

/// Translation of `Impl_OnActivated_QueryInterface()`.
unsafe extern "system" fn impl_on_activated_query_interface(
    this: *mut ToastEventHandler,
    riid: *const GUID,
    ppv_object: *mut *mut c_void,
) -> HRESULT {
    // SAFETY: as COM calls it.
    unsafe { handler_query_interface(this, riid, ppv_object, &IID_ITOAST_ACTIVATED_EVENT_HANDLER) }
}

/// The text of an `HSTRING` (`WindowsGetStringRawBuffer()`).
///
/// # Safety
///
/// `hstring` must be a live string.
unsafe fn hstring_text(winrt: &WinRt, hstring: HSTRING) -> Vec<u16> {
    let mut len = 0u32;
    // SAFETY: the caller's contract; the buffer holds len characters.
    unsafe {
        let p = (winrt.windows_get_string_raw_buffer)(hstring, &mut len);
        if p.is_null() {
            return Vec::new();
        }
        std::slice::from_raw_parts(p, len as usize).to_vec()
    }
}

/// `SDL_wcstoul(tag, NULL, 10)`: leading white space, an optional sign and
/// decimal digits (0 for none).
fn wcstoul(tag: &[u16]) -> u32 {
    let text = wide_to_utf8(tag);
    let text = text.trim_start();
    let (negative, digits) = match text.as_bytes().first() {
        Some(b'-') => (true, &text[1..]),
        Some(b'+') => (false, &text[1..]),
        _ => (false, text),
    };
    let end = digits
        .bytes()
        .position(|c| !c.is_ascii_digit())
        .unwrap_or(digits.len());
    let value = digits[..end].parse::<u64>().unwrap_or(u64::MAX);
    let value = if end == 0 { 0 } else { value };
    // (an unsigned long is 32 bits on Windows; out of range saturates)
    let value = u32::try_from(value).unwrap_or(u32::MAX);
    if negative {
        value.wrapping_neg()
    } else {
        value
    }
}

/// Translation of `Impl_OnActivated_Invoke()`.
unsafe extern "system" fn impl_on_activated_invoke(
    _this: *mut ToastEventHandler,
    sender: *mut IToastNotification,
    args: *mut IInspectable,
) -> HRESULT {
    let Some(winrt) = state().winrt else {
        return S_OK;
    };
    // SAFETY: the notification passes live objects, which we borrow
    // (ComPtr::clone() takes our own reference).
    let (sender, args) = unsafe {
        let (Some(sender), Some(args)) = (ComPtr::from_raw(sender), ComPtr::from_raw(args)) else {
            return S_OK;
        };
        let borrowed = (sender.clone(), args.clone());
        std::mem::forget(sender);
        std::mem::forget(args);
        borrowed
    };

    // Note (upstream): pEventArgs and the tag and arguments strings are
    // never released; here they are.
    let Ok(p_event_args) =
        args.query::<IToastActivatedEventArgsVtbl>(&IID_ITOAST_ACTIVATED_EVENT_ARGS)
    else {
        return S_OK;
    };
    let mut id: NotificationID = 0;

    if let Ok(p_toast_notification2) =
        sender.query::<IToastNotification2Vtbl>(&IID_ITOAST_NOTIFICATION2)
    {
        let mut hs_tag: HSTRING = std::ptr::null_mut();
        // SAFETY: the notification is live; the out-pointer is valid.
        let hr = unsafe {
            (p_toast_notification2.vtbl().get_tag)(p_toast_notification2.as_ptr(), &mut hs_tag)
        };
        if hr >= 0 {
            let hs_tag = HString(hs_tag, winrt.windows_delete_string);
            // SAFETY: the tag is a live string.
            let tag = unsafe { hstring_text(&winrt, hs_tag.0) };
            id = wcstoul(&tag);
        }
    }

    if id != 0 {
        let mut hs_event_string: HSTRING = std::ptr::null_mut();
        // SAFETY: the arguments are live; the out-pointer is valid.
        let hr = unsafe {
            (p_event_args.vtbl().get_arguments)(p_event_args.as_ptr(), &mut hs_event_string)
        };
        if hr >= 0 {
            let hs_event_string = HString(hs_event_string, winrt.windows_delete_string);
            // SAFETY: a live string.
            let mut wstr = unsafe { hstring_text(&winrt, hs_event_string.0) };
            wstr.push(0);
            // (into a 512-byte buffer)
            if let Some(mut tmp) = wide_char_to_multi_byte(CP_UTF8, 0, &wstr) {
                if !tmp.is_empty() && tmp.len() <= 512 {
                    tmp.pop();
                    crate::events::window::send_notification_action(
                        id,
                        &String::from_utf8_lossy(&tmp),
                    );
                }
            }
        }
    }

    S_OK
}

static WINDOWS_TOAST_ON_ACTIVATED_VTBL: ToastEventHandlerVtbl = ToastEventHandlerVtbl {
    query_interface: impl_on_activated_query_interface,
    add_ref: impl_igeneric_add_ref,
    release: impl_igeneric_release,
    invoke: impl_on_activated_invoke,
};

static ON_ACTIVATED: ToastEventHandler = ToastEventHandler {
    lp_vtbl: &WINDOWS_TOAST_ON_ACTIVATED_VTBL,
    ref_count: AtomicI32::new(1),
};

// OnDismissed interface

/// Translation of `Impl_OnDismissed_QueryInterface()`.
unsafe extern "system" fn impl_on_dismissed_query_interface(
    this: *mut ToastEventHandler,
    riid: *const GUID,
    ppv_object: *mut *mut c_void,
) -> HRESULT {
    // SAFETY: as COM calls it.
    unsafe { handler_query_interface(this, riid, ppv_object, &IID_ITOAST_DISMISSED_EVENT_HANDLER) }
}

/// Translation of `Impl_OnDismissed_Invoke()`.
unsafe extern "system" fn impl_on_dismissed_invoke(
    _this: *mut ToastEventHandler,
    sender: *mut IToastNotification,
    args: *mut IInspectable,
) -> HRESULT {
    let args = args.cast::<IToastDismissedEventArgs>();
    let mut reason = 0i32;
    // SAFETY: the notification passes live dismissal arguments.
    unsafe { (ComObject::vtbl(args).get_reason)(args, &mut reason) };

    /* Remove transient notifications that were cancelled or timed out,
     * so they won't persist in the notification center.
     */
    match reason {
        TOAST_DISMISSAL_REASON_TIMED_OUT | TOAST_DISMISSAL_REASON_USER_CANCELED => {
            let notifier = state().toast_notifier.clone();
            if let Some(notifier) = notifier {
                // SAFETY: the notifier and the sender are live.
                unsafe { (notifier.vtbl().hide)(notifier.as_ptr(), sender) };
            }
        }

        _ => {}
    }

    S_OK
}

static WINDOWS_TOAST_ON_DISMISSED_VTBL: ToastEventHandlerVtbl = ToastEventHandlerVtbl {
    query_interface: impl_on_dismissed_query_interface,
    add_ref: impl_igeneric_add_ref,
    release: impl_igeneric_release,
    invoke: impl_on_dismissed_invoke,
};

static ON_DISMISSED: ToastEventHandler = ToastEventHandler {
    lp_vtbl: &WINDOWS_TOAST_ON_DISMISSED_VTBL,
    ref_count: AtomicI32::new(1),
};

/// Translation of `IsInPackage()`.
fn is_in_package() -> bool {
    let name = utf8_to_wide("kernel32.dll");
    // SAFETY: the names are NUL-terminated; the export has this prototype.
    let get_current_package_full_name: Option<GetCurrentPackageFullName_t> = unsafe {
        let kernel32 = GetModuleHandleW(name.as_ptr());
        GetProcAddress(kernel32, c"GetCurrentPackageFullName".as_ptr().cast()).map(|f| {
            std::mem::transmute::<unsafe extern "system" fn() -> isize, GetCurrentPackageFullName_t>(
                f,
            )
        })
    };

    if let Some(get_current_package_full_name) = get_current_package_full_name {
        let mut length = 0u32;
        // SAFETY: a NULL buffer of length 0 asks for the length.
        let rc = unsafe { get_current_package_full_name(&mut length, std::ptr::null_mut()) };
        if rc != ERROR_INSUFFICIENT_BUFFER as i32 {
            return rc != APPMODEL_ERROR_NO_PACKAGE;
        }
    }

    false
}

/// Translation of `GetExePath()`.
fn get_exe_path() -> Option<Vec<u16>> {
    let mut buflen = MAX_PATH;
    let mut path;
    let len;

    loop {
        path = vec![0u16; buflen as usize];

        // SAFETY: path holds buflen characters.
        let n = unsafe { GetModuleFileNameW(std::ptr::null_mut(), path.as_mut_ptr(), buflen) };
        // If this was truncated, then len >= buflen - 1
        if n < buflen - 1 {
            len = n;
            break;
        }

        // buffer too small? Try again.
        buflen *= 2;
    }

    if len == 0 {
        return None;
    }

    path.truncate(len as usize + 1);
    Some(path)
}

/* There is no way to pass an image as a byte stream when creating Windows
 * notifications, so surfaces are saved as PNG files to temporary storage
 * while the notification is being shown, then cleaned up a few seconds later.
 */

/// Translation of `CleanupIcons()`.
fn cleanup_icons(state: &mut State) {
    // SAFETY: KillTimer accepts any timer ID.
    unsafe { KillTimer(std::ptr::null_mut(), state.cleanup_timer_id) };
    state.cleanup_timer_id = 0;

    for icon_file in state.toast_icons.drain(..) {
        // SAFETY: the path is NUL-terminated.
        unsafe { DeleteFileW(icon_file.as_ptr()) };
    }
}

/// Translation of `IconCleanupCallback()`.
unsafe extern "system" fn icon_cleanup_callback(
    _hwnd: HWND,
    _u_msg: u32,
    _id_event: usize,
    _dw_time: u32,
) {
    cleanup_icons(&mut state());
}

/// Translation of `GetFullPath()`: the full path of an existing file.
fn get_full_path(path: &str) -> Option<Vec<u16>> {
    let mut source = path.as_bytes().to_vec();
    source.push(0);
    let conv_len = source.len();

    let mut conv_path = vec![0u16; conv_len];
    // SAFETY: the source is NUL-terminated; the target holds conv_len characters.
    unsafe {
        MultiByteToWideChar(
            CP_UTF8,
            0,
            source.as_ptr(),
            -1,
            conv_path.as_mut_ptr(),
            conv_len as i32,
        )
    };

    // SAFETY: conv_path is NUL-terminated; a NULL buffer asks for the length.
    let full_len = unsafe {
        GetFullPathNameW(
            conv_path.as_ptr(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if full_len == 0 {
        return None;
    }

    let mut full_path = vec![0u16; full_len as usize];
    // SAFETY: full_path holds full_len characters.
    unsafe {
        GetFullPathNameW(
            conv_path.as_ptr(),
            full_len,
            full_path.as_mut_ptr(),
            std::ptr::null_mut(),
        )
    };
    let end = full_path
        .iter()
        .position(|&c| c == 0)
        .unwrap_or(full_path.len());
    full_path.truncate(end);
    full_path.push(0);

    // Make sure the file actually exists.
    // SAFETY: plain data; the path is NUL-terminated.
    let exists = unsafe {
        let mut attr: WIN32_FILE_ATTRIBUTE_DATA = std::mem::zeroed();
        GetFileAttributesExW(
            full_path.as_ptr(),
            GetFileExInfoStandard,
            (&mut attr as *mut WIN32_FILE_ATTRIBUTE_DATA).cast(),
        ) != 0
    };
    exists.then_some(full_path)
}

/// Translation of `GetToastIconPath()`.
fn get_toast_icon_path(state: &mut State) -> Option<Vec<u16>> {
    if state.app_icon_path.is_some() {
        return state.app_icon_path.clone();
    }

    let icon = crate::properties::Properties::global()
        .get_string(super::PROP_GLOBAL_NOTIFICATION_HEADER_ICON_STRING);
    if let Some(icon) = icon {
        state.app_icon_path = get_full_path(&icon);
    }

    state.app_icon_path.clone()
}

/// Translation of `SaveToastImage()`.
fn save_toast_image(state: &mut State, icon: Option<&Surface<'_>>) -> Option<Vec<u16>> {
    let icon = icon?;

    // The documentation states that the buffers for these should be MAX_PATH.

    // Stop any timers so they won't fire in the middle of this.
    if state.cleanup_timer_id != 0 {
        // SAFETY: KillTimer accepts any timer ID.
        unsafe { KillTimer(std::ptr::null_mut(), state.cleanup_timer_id) };
    }

    // SAFETY: a NULL buffer asks for the length.
    let path_len = unsafe { GetTempPathW(0, std::ptr::null_mut()) };
    if path_len == 0 {
        return None;
    }
    let mut temp_path = vec![0u16; path_len as usize + 1];
    // SAFETY: temp_path holds path_len + 1 characters.
    let path_len = unsafe { GetTempPathW(path_len + 1, temp_path.as_mut_ptr()) };
    if path_len == 0 {
        return None;
    }

    let mut file_name = [0u16; MAX_PATH as usize];
    let prefix = utf8_to_wide("SDL");
    // SAFETY: the strings are NUL-terminated; file_name holds MAX_PATH
    // characters.
    let name_ret = unsafe {
        GetTempFileNameW(
            temp_path.as_ptr(),
            prefix.as_ptr(),
            0,
            file_name.as_mut_ptr(),
        )
    };
    if name_ret == 0 {
        return None;
    }
    let name_len = file_name
        .iter()
        .position(|&c| c == 0)
        .unwrap_or(file_name.len());
    let mut path_buf = file_name[..name_len].to_vec();
    path_buf.extend(".png".encode_utf16());
    path_buf.push(0);

    let png_path = wide_to_utf8(&path_buf);
    if let Ok(mut copy) = icon.duplicate() {
        let _ = copy.save_png(&png_path);
    }

    state.toast_icons.push(path_buf.clone());
    // Note (upstream): GetTempFileNameW() creates the file it names, which
    // is never deleted (only the ".png" next to it is). Here it keeps the
    // name taken while the image is in use, and goes after it.
    let mut placeholder = file_name[..name_len].to_vec();
    placeholder.push(0);
    state.toast_icons.push(placeholder);

    // Schedule a cleanup of icons 5 seconds from now.
    // SAFETY: a thread timer with a valid callback.
    state.cleanup_timer_id =
        unsafe { SetTimer(std::ptr::null_mut(), 0, 5000, Some(icon_cleanup_callback)) };

    // Duplicate the path, since the source object will be destroyed by a timer.
    Some(path_buf)
}

/// Translation of `GetAppMetadata()`: the metadata, or the executable's
/// file name.
fn get_app_metadata(metadata_name: crate::init::AppMetadata) -> Option<Vec<u16>> {
    // (SDL_GetAppMetadataProperty() with upstream's fallbacks: the name
    // falls back to the executable's name there too)
    let app_metadata = crate::init::app_metadata_property(metadata_name);
    if let Some(app_metadata) = app_metadata.filter(|m| !m.is_empty()) {
        return Some(utf8_to_wide(&app_metadata));
    }

    exe_file_name()
}

/// The executable's file name (the fallback of `GetAppMetadata()`).
fn exe_file_name() -> Option<Vec<u16>> {
    let wsz_exe_path = get_exe_path()?;
    let len = wsz_exe_path.len() - 1;
    let start = wsz_exe_path[..len]
        .iter()
        .rposition(|&c| c == u16::from(b'/') || c == u16::from(b'\\'))
        .map_or(0, |i| i + 1);
    Some(wsz_exe_path[start..].to_vec())
}

/// Translation of `QuitToastSystem()`.
fn quit_toast_system(state: &mut State) {
    state.toast_notification_history = None;
    state.toast_notification_manager_statics2 = None;
    state.notification_factory = None;
    state.toast_notifier = None;
    state.toast_notification_manager = None;
    state.hs_app_id = None;
    state.hs_group_id = None;

    cleanup_icons(state);

    if state.ro_initialized {
        ro_uninitialize();
        state.ro_initialized = false;
    }
    if state.co_initialized {
        co_uninitialize();
        state.co_initialized = false;
    }

    state.app_reg_key = None;

    state.app_icon_path = None;
}

/// `HRESULT_FROM_WIN32()`.
fn hresult_from_win32(x: u32) -> HRESULT {
    if x as i32 <= 0 {
        x as HRESULT
    } else {
        ((x & 0x0000_FFFF) | 0x8007_0000) as HRESULT
    }
}

/// Load a combase function as type `T`.
///
/// # Safety
///
/// `T` must be the function's prototype.
unsafe fn resolve<T: Copy>(name: &std::ffi::CStr) -> Result<T> {
    let Some(f) = load_combase_function(name) else {
        return Err(set_error(&format!(
            "GetProcAddress failed for {}",
            name.to_string_lossy()
        )));
    };
    // SAFETY: `T` is a function pointer type of the function's prototype.
    Ok(unsafe { std::mem::transmute_copy::<unsafe extern "system" fn() -> isize, T>(&f) })
}

/// The `REG_SZ` value `name` of the app's key.
fn reg_set_string(app_reg_key: &[u16], name: &str, value: &[u16]) -> HRESULT {
    let name = utf8_to_wide(name);
    let len = value.iter().position(|&c| c == 0).unwrap_or(value.len());
    // FIXME (upstream): the size of a REG_SZ should count its terminating
    // NUL.
    // SAFETY: the key and name are NUL-terminated; value holds len characters.
    hresult_from_win32(unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            app_reg_key.as_ptr(),
            name.as_ptr(),
            REG_SZ,
            value.as_ptr().cast(),
            (len * 2) as u32,
        )
    })
}

/// Translation of `InitToastSystem()`.
fn init_toast_system(state: &mut State) -> Result<()> {
    // Only try to initialize once.
    if state.initialization_status != InitializationStatus::Uninitialized {
        return if state.initialization_status == InitializationStatus::Initialized {
            Ok(())
        } else {
            Err(Error::new("The notification system failed to initialize"))
        };
    }

    // Failure of anything results in an early-out, so the initial value is failure.
    state.initialization_status = InitializationStatus::Failed;

    // SAFETY: each function is loaded with its prototype.
    let winrt = unsafe {
        WinRt {
            ro_get_activation_factory: resolve(c"RoGetActivationFactory")?,
            ro_activate_instance: resolve(c"RoActivateInstance")?,
            windows_create_string: resolve(c"WindowsCreateString")?,
            windows_create_string_reference: resolve(c"WindowsCreateStringReference")?,
            windows_get_string_raw_buffer: resolve(c"WindowsGetStringRawBuffer")?,
            windows_delete_string: resolve(c"WindowsDeleteString")?,
        }
    };
    state.winrt = Some(winrt);

    // Initialize COM and Windows runtime with the same threading model.
    let hr = co_initialize();
    if hr < 0 {
        return Err(error_from_hresult(Some("CoInitialize"), hr));
    }
    state.co_initialized = true;

    let hr = ro_initialize();
    if hr < 0 {
        return Err(error_from_hresult(Some("RoInitialize"), hr));
    }
    state.ro_initialized = true;

    let result = (|| -> std::result::Result<(), HRESULT> {
        // Get the application ID and name.
        let app_id = get_app_metadata(crate::init::AppMetadata::Identifier).unwrap_or(vec![0]);
        let app_name = get_app_metadata(crate::init::AppMetadata::Name).unwrap_or(vec![0]);

        let create_string = |s: &[u16]| -> std::result::Result<HString, HRESULT> {
            let mut hstring: HSTRING = std::ptr::null_mut();
            // SAFETY: s holds len - 1 characters before its NUL.
            let hr = unsafe {
                (winrt.windows_create_string)(s.as_ptr(), (s.len() - 1) as u32, &mut hstring)
            };
            if hr < 0 {
                return Err(hr);
            }
            Ok(HString(hstring, winrt.windows_delete_string))
        };

        // Create the persistent appID string.
        state.hs_app_id = Some(create_string(&app_id)?);

        // Create the persistent groupID string.
        state.hs_group_id = Some(create_string(&utf8_to_wide(GROUP_ID_STR))?);

        // Build the registry key.
        let mut app_reg_key: Vec<u16> = REG_KEY_BASE.encode_utf16().collect();
        app_reg_key.extend_from_slice(&app_id);
        state.app_reg_key = Some(app_reg_key.clone());

        // Set the app name and icon.
        if let Some(icon_path) = get_toast_icon_path(state) {
            let hr = reg_set_string(&app_reg_key, "IconUri", &icon_path);
            if hr < 0 {
                return Err(hr);
            }
        } else {
            // This will "fail" if the key already doesn't exist, which is fine.
            let name = utf8_to_wide("IconUri");
            // SAFETY: the key and name are NUL-terminated.
            unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, app_reg_key.as_ptr(), name.as_ptr()) };
        }

        let hr = reg_set_string(&app_reg_key, "DisplayName", &app_name);
        if hr < 0 {
            return Err(hr);
        }

        let class_name = utf8_to_wide(RUNTIME_CLASS_TOAST_NOTIFICATION_MANAGER);
        let mut header = new_header();
        // SAFETY: the name and header outlive the reference.
        let hs_toast_notification_manager =
            unsafe { string_reference(&winrt, &class_name, &mut header)? };

        // SAFETY: the out-pointer receives the factory on success.
        let manager = unsafe {
            ComPtr::<IToastNotificationManagerStaticsVtbl>::from_out(|out| {
                (winrt.ro_get_activation_factory)(
                    hs_toast_notification_manager,
                    &IID_ITOAST_NOTIFICATION_MANAGER_STATICS,
                    out.cast(),
                )
            })?
        };
        let manager = state.toast_notification_manager.insert(manager).clone();

        let hs_app_id = state
            .hs_app_id
            .as_ref()
            .map_or(std::ptr::null_mut(), |h| h.0);
        // SAFETY: the manager is live; the out-pointer receives a notifier.
        let notifier = unsafe {
            ComPtr::<IToastNotifierVtbl>::from_out(|out| {
                (manager.vtbl().create_toast_notifier_with_id)(manager.as_ptr(), hs_app_id, out)
            })?
        };
        state.toast_notifier = Some(notifier);

        let class_name = utf8_to_wide(RUNTIME_CLASS_TOAST_NOTIFICATION);
        let mut header = new_header();
        // SAFETY: the name and header outlive the reference.
        let hs_toast_notification = unsafe { string_reference(&winrt, &class_name, &mut header)? };

        // SAFETY: the out-pointer receives the factory on success.
        let factory = unsafe {
            ComPtr::<IToastNotificationFactoryVtbl>::from_out(|out| {
                (winrt.ro_get_activation_factory)(
                    hs_toast_notification,
                    &IID_ITOAST_NOTIFICATION_FACTORY,
                    out.cast(),
                )
            })?
        };
        state.notification_factory = Some(factory);

        // This acts as a version check, as notification history is only available on Win10+.
        let statics2 = manager.query::<IToastNotificationManagerStatics2Vtbl>(
            &IID_ITOAST_NOTIFICATION_MANAGER_STATICS2,
        )?;

        // SAFETY: the statics are live; the out-pointer receives the history.
        let history = unsafe {
            ComPtr::<IToastNotificationHistoryVtbl>::from_out(|out| {
                (statics2.vtbl().get_history)(statics2.as_ptr(), out)
            })?
        };
        state.toast_notification_manager_statics2 = Some(statics2);
        state.toast_notification_history = Some(history);

        Ok(())
    })();

    // cleanup:
    // (the string references need no deleting)
    match result {
        Ok(()) => {
            state.initialization_status = InitializationStatus::Initialized;
            Ok(())
        }
        Err(hr) => {
            quit_toast_system(state);
            Err(error_from_hresult(None, hr))
        }
    }
}

/// Translation of `AppendXmlAudio()`.
fn append_xml_audio(dst: &mut Vec<u16>, sound: &str) {
    const DEFAULT_SOUND_PATH: &str = "ms-winsoundevent:Notification.Default";

    let mut silent_str = "false";
    let mut sound_path = DEFAULT_SOUND_PATH.to_string();

    if sound == "silent" {
        silent_str = "true";
    } else if sound != "default" {
        /* Windows currently only loads custom notification sounds when the app is
         * in an MSIX package. We'll prepend a 'file:///' prefix when not in a package
         * in case this changes in the future, but for now, it just plays the default
         * sound.
         */
        if is_in_package() {
            let prefix = "ms-appx:///";
            let sound = sound.strip_prefix('/').unwrap_or(sound);
            sound_path = format!("{prefix}{sound}");
        } else {
            crate::log::log(
                crate::log::Category::Application,
                crate::log::Priority::Warn,
                "Windows does not support custom notification sounds outside an MSIX package",
            );

            if let Some(path) = get_full_path(sound) {
                let prefix = "file:///";
                sound_path = format!("{prefix}{}", wide_to_utf8(&path));
            }
        }
    }

    // FIXME (upstream): the attribute values (here and in the actions)
    // aren't XML-escaped.
    dst.extend(
        format!("<audio src=\"{sound_path}\" loop=\"false\" silent=\"{silent_str}\"/>")
            .encode_utf16(),
    );
}

/// Translation of `AppendXmlAction()`.
fn append_xml_action(dst: &mut Vec<u16>, label: &str, action: &str) {
    dst.extend(
        format!(
            "<action content=\"{label}\" activationType=\"foreground\" arguments=\"{action}\"/>"
        )
        .encode_utf16(),
    );
}

/// Translation of `AppendXmlImage()`.
fn append_xml_image(dst: &mut Vec<u16>, image_path: Option<&[u16]>) {
    // FIXME (upstream): this is called without an image too, whose NULL
    // path SDL_swprintf() prints as "(null)".
    let image_path = image_path.map_or_else(|| "(null)".to_string(), wide_to_utf8);
    dst.extend(
        format!(
            "<image id=\"1\" placement=\"appLogoOverride\" src=\"file:///{image_path}\"></image>"
        )
        .encode_utf16(),
    );
}

const XML_TOAST_OPENING_STR: &str = concat!(
    "<toast scenario=\"default\" activationType=\"foreground\" launch=\"default\">",
    "<visual>",
    "<binding template=\"ToastGeneric\">"
);
const XML_TOAST_CLOSING_STR: &str = "</toast>";
const XML_TEXT_OPENING_STR: &str = "<text><![CDATA[";
const XML_TEXT_CLOSING_STR: &str = "]]></text>";
const XML_ACTIONS_OPENING_STR: &str = "<actions>";
const XML_ACTIONS_CLOSING_STR: &str = "</actions>";
const XML_VISUAL_CLOSING_STR: &str = "</binding></visual>";

/// Translation of `AppendXmlText()`.
fn append_xml_text(dst: &mut Vec<u16>, text: Option<&str>) -> bool {
    // FIXME (upstream): a missing text (a notification without a message)
    // fails the conversion, and so the notification.
    let Some(text) = text else {
        return false;
    };

    dst.extend(XML_TEXT_OPENING_STR.encode_utf16());
    dst.extend(text.encode_utf16());
    dst.extend(XML_TEXT_CLOSING_STR.encode_utf16());

    true
}

/// Translation of `BuildNotificationXml()`: the toast XML, NUL-terminated
/// (for `WindowsCreateStringReference()`).
fn build_notification_xml(
    notification: &Notification<'_>,
    icon_path: Option<&[u16]>,
) -> Option<Vec<u16>> {
    let mut dst = Vec::new();

    let title = notification.title.as_deref();
    let message = notification.message.as_deref();
    let sound = notification.sound.as_deref().unwrap_or("default");
    let actions = &notification.actions;

    dst.extend(XML_TOAST_OPENING_STR.encode_utf16());
    append_xml_image(&mut dst, icon_path);
    if !append_xml_text(&mut dst, title) {
        return None;
    }
    if !append_xml_text(&mut dst, message) {
        return None;
    }

    dst.extend(XML_VISUAL_CLOSING_STR.encode_utf16());

    append_xml_audio(&mut dst, sound);

    if !actions.is_empty() {
        dst.extend(XML_ACTIONS_OPENING_STR.encode_utf16());

        for action in actions {
            let NotificationAction::Button {
                action_id,
                action_label,
            } = action;
            append_xml_action(&mut dst, action_label, action_id);
        }

        dst.extend(XML_ACTIONS_CLOSING_STR.encode_utf16());
    }

    // The XML string *must* be null-terminated for WindowsCreateStringReference().
    dst.extend(XML_TOAST_CLOSING_STR.encode_utf16());
    dst.push(0);

    Some(dst)
}

/// The decimal tag of a notification ID.
fn notification_tag(id: NotificationID) -> Vec<u16> {
    utf8_to_wide(&id.to_string())
}

/// Translation of `ClearNotificationWithID()`.
fn clear_notification_with_id(state: &State, id: NotificationID) {
    let (Some(winrt), Some(history)) = (state.winrt, &state.toast_notification_history) else {
        return;
    };
    let tag = notification_tag(id);
    let mut header = new_header();

    // SAFETY: the tag and header outlive the reference.
    let Ok(hs_tag) = (unsafe { string_reference(&winrt, &tag, &mut header) }) else {
        return;
    };

    let group = state
        .hs_group_id
        .as_ref()
        .map_or(std::ptr::null_mut(), |h| h.0);
    let app = state
        .hs_app_id
        .as_ref()
        .map_or(std::ptr::null_mut(), |h| h.0);
    // SAFETY: the history and strings are live.
    unsafe { (history.vtbl().remove_grouped_tag_with_id)(history.as_ptr(), hs_tag, group, app) };
}

/// Translation of `WIN_BCryptGenRandom()`.
fn bcrypt_gen_random(buffer: &mut [u8], flags: u32) -> i32 {
    struct Loaded(Option<BCryptGenRandom_t>);
    static P_BCRYPT_GEN_RANDOM: std::sync::OnceLock<Loaded> = std::sync::OnceLock::new();

    let loaded = P_BCRYPT_GEN_RANDOM.get_or_init(|| {
        let name = utf8_to_wide("bcrypt.dll");
        // SAFETY: the names are NUL-terminated; the export has this prototype.
        unsafe {
            let s_h_bcrypt = LoadLibraryExW(
                name.as_ptr(),
                std::ptr::null_mut(),
                LOAD_LIBRARY_SEARCH_SYSTEM32,
            );
            if s_h_bcrypt.is_null() {
                return Loaded(None);
            }
            Loaded(
                GetProcAddress(s_h_bcrypt, c"BCryptGenRandom".as_ptr().cast()).map(|f| {
                    std::mem::transmute::<unsafe extern "system" fn() -> isize, BCryptGenRandom_t>(
                        f,
                    )
                }),
            )
        }
    });

    match loaded.0 {
        // SAFETY: the buffer holds its length.
        Some(f) => unsafe {
            f(
                std::ptr::null_mut(),
                buffer.as_mut_ptr(),
                buffer.len() as u32,
                flags,
            )
        },
        None => -1,
    }
}

const INIT_ERROR: &str =
    "Failed to initialize the notification system (requires Windows 10 or higher)";

/// Translation of `SDL_SYS_ShowNotification()`.
pub(super) fn show_notification(notification: &Notification<'_>) -> Result<NotificationID> {
    let mut state = state();

    // Need Win10 or higher for notifications.
    if init_toast_system(&mut state).is_err() {
        return Err(Error::new(INIT_ERROR));
    }
    let winrt = state.winrt.expect("initialized");

    let image = notification.image;
    let replaces = notification.replaces.unwrap_or(0);
    let priority = notification.priority;
    let transient = notification.transient;

    if replaces != 0 {
        clear_notification_with_id(&state, replaces);
    }

    // Generate a unique notification ID.
    let mut bytes = [0u8; 4];
    let new_id = if bcrypt_gen_random(&mut bytes, BCRYPT_USE_SYSTEM_PREFERRED_RNG) != 0 {
        // STATUS_SUCCESS == 0
        // No RNG? Use the low 32 bits of the current time.
        crate::timer::ticks_ns() as NotificationID
    } else {
        NotificationID::from_ne_bytes(bytes)
    };

    // Windows notifications load images from disk, so save it to temporary storage.
    let image_path = save_toast_image(&mut state, image);

    // Build the XML description for the notification.
    let Some(xml) = build_notification_xml(notification, image_path.as_deref()) else {
        return Err(Error::new("Couldn't build the notification"));
    };

    let result = (|| -> std::result::Result<(), HRESULT> {
        let class_name = utf8_to_wide(RUNTIME_CLASS_XML_DOCUMENT);
        let mut hsh_xml_document = new_header();
        // SAFETY: the name and header outlive the reference.
        let hs_xml_document =
            unsafe { string_reference(&winrt, &class_name, &mut hsh_xml_document)? };

        let mut hsh_banner = new_header();
        // SAFETY: the XML and header outlive the reference.
        let hs_banner = unsafe { string_reference(&winrt, &xml, &mut hsh_banner)? };

        // SAFETY: the out-pointer receives an instance on success.
        let p_inspectable = unsafe {
            ComPtr::<IInspectableVtbl>::from_out(|out| {
                (winrt.ro_activate_instance)(hs_xml_document, out)
            })?
        };

        let p_xml_document = p_inspectable.query::<IInspectableVtbl>(&IID_IXML_DOCUMENT)?;

        let p_xml_document_io =
            p_xml_document.query::<IXmlDocumentIOVtbl>(&IID_IXML_DOCUMENT_IO)?;

        // SAFETY: the document and string are live.
        let hr =
            unsafe { (p_xml_document_io.vtbl().load_xml)(p_xml_document_io.as_ptr(), hs_banner) };
        if hr < 0 {
            return Err(hr);
        }

        let factory = state.notification_factory.clone().ok_or(E_POINTER)?;
        // SAFETY: the factory and document are live; the out-pointer
        // receives a notification.
        let p_toast_notification = unsafe {
            ComPtr::<IToastNotificationVtbl>::from_out(|out| {
                (factory.vtbl().create_toast_notification)(
                    factory.as_ptr(),
                    p_xml_document.as_ptr(),
                    out,
                )
            })?
        };
        let tn = p_toast_notification.as_ptr();
        let tn_vtbl = p_toast_notification.vtbl();

        // Register the OnDismissed notifier to clear transient notifications when cancelled or timed out.
        if transient {
            let mut dismissed_token = EventRegistrationToken::default();
            // SAFETY: the notification is live; the handler is static.
            let hr =
                unsafe { (tn_vtbl.add_dismissed)(tn, ON_DISMISSED.as_ptr(), &mut dismissed_token) };
            if hr < 0 {
                return Err(hr);
            }
        }
        {
            let mut activated_token = EventRegistrationToken::default();
            // SAFETY: as above.
            let hr =
                unsafe { (tn_vtbl.add_activated)(tn, ON_ACTIVATED.as_ptr(), &mut activated_token) };
            if hr < 0 {
                return Err(hr);
            }
        }

        // Tag with the ID for future removal or replacement.
        {
            let p_toast_notification2 =
                p_toast_notification.query::<IToastNotification2Vtbl>(&IID_ITOAST_NOTIFICATION2)?;

            let tag = notification_tag(new_id);
            let mut hsh_tag = new_header();
            // SAFETY: the tag and header outlive the reference.
            let hs_tag = unsafe { string_reference(&winrt, &tag, &mut hsh_tag)? };

            let group = state
                .hs_group_id
                .as_ref()
                .map_or(std::ptr::null_mut(), |h| h.0);
            // SAFETY: the notification and strings are live.
            unsafe {
                let hr =
                    (p_toast_notification2.vtbl().put_group)(p_toast_notification2.as_ptr(), group);
                if hr < 0 {
                    return Err(hr);
                }

                let hr =
                    (p_toast_notification2.vtbl().put_tag)(p_toast_notification2.as_ptr(), hs_tag);
                if hr < 0 {
                    return Err(hr);
                }
            }
        }

        /* Set the priority.
         * All levels except for SDL_NOTIFICATION_PRIORITY_CRITICAL map to normal on Windows, as selecting
         * high priority can wake the screen, and should only be done when absolutely necessary.
         */
        {
            let toast_priority = if priority != NotificationPriority::Critical {
                TOAST_NOTIFICATION_PRIORITY_DEFAULT
            } else {
                TOAST_NOTIFICATION_PRIORITY_HIGH
            };
            let p_toast_notification4 =
                p_toast_notification.query::<IToastNotification4Vtbl>(&IID_ITOAST_NOTIFICATION4)?;

            // SAFETY: the notification is live.
            let hr = unsafe {
                (p_toast_notification4.vtbl().put_priority)(
                    p_toast_notification4.as_ptr(),
                    toast_priority,
                )
            };
            if hr < 0 {
                return Err(hr);
            }
        }

        // Finally, show the notification.
        let notifier = state.toast_notifier.clone().ok_or(E_POINTER)?;
        // SAFETY: the notifier and notification are live.
        let hr = unsafe { (notifier.vtbl().show)(notifier.as_ptr(), tn) };
        if hr < 0 {
            return Err(hr);
        }
        Ok(())
    })();

    // cleanup:
    match result {
        Ok(()) => Ok(new_id),
        Err(hr) => Err(error_from_hresult(
            Some("Couldn't show the notification"),
            hr,
        )),
    }
}

/// Translation of `SDL_RemoveNotification()` (Windows).
pub(super) fn remove_notification(notification: NotificationID) -> Result<()> {
    let mut state = state();
    if init_toast_system(&mut state).is_err() {
        return Err(Error::new(INIT_ERROR));
    }

    clear_notification_with_id(&state, notification);
    Ok(())
}

/// Translation of `SDL_CleanupNotifications()` (Windows).
pub(super) fn cleanup_notifications() {
    let mut state = state();
    quit_toast_system(&mut state);
    state.initialization_status = InitializationStatus::Uninitialized;
}

/// Translation of `SDL_RequestNotificationPermission()` (Windows).
pub(super) fn request_notification_permission() -> Result<()> {
    // Notifications are supported on Win10 or higher.
    init_toast_system(&mut state()).map_err(|_| Error::new(INIT_ERROR))
}

/// (Windows has no activation tokens)
pub(super) fn notification_activation_token() -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(w: &[u16]) -> String {
        String::from_utf16(w.strip_suffix(&[0]).unwrap_or(w)).unwrap()
    }

    #[test]
    fn toast_xml() {
        let notification = Notification {
            title: Some("Title <&>".into()),
            message: Some("Body".into()),
            actions: vec![NotificationAction::Button {
                action_id: "ok".into(),
                action_label: "OK".into(),
            }],
            sound: Some("silent".into()),
            ..Notification::default()
        };
        let icon = utf8_to_wide("C:\\tmp\\SDL1.tmp.png");
        let xml = build_notification_xml(&notification, Some(&icon)).unwrap();
        assert_eq!(xml.last(), Some(&0));
        assert_eq!(
            text(&xml),
            concat!(
                "<toast scenario=\"default\" activationType=\"foreground\" launch=\"default\">",
                "<visual><binding template=\"ToastGeneric\">",
                "<image id=\"1\" placement=\"appLogoOverride\" src=\"file:///C:\\tmp\\SDL1.tmp.png\"></image>",
                "<text><![CDATA[Title <&>]]></text><text><![CDATA[Body]]></text>",
                "</binding></visual>",
                "<audio src=\"ms-winsoundevent:Notification.Default\" loop=\"false\" silent=\"true\"/>",
                "<actions><action content=\"OK\" activationType=\"foreground\" arguments=\"ok\"/></actions>",
                "</toast>"
            )
        );

        // The default sound, no actions, no image.
        let notification = Notification {
            title: Some("T".into()),
            message: Some("M".into()),
            ..Notification::default()
        };
        let xml = text(&build_notification_xml(&notification, None).unwrap());
        assert!(xml.contains("src=\"file:///(null)\""), "{xml}");
        assert!(xml.contains(
            "<audio src=\"ms-winsoundevent:Notification.Default\" loop=\"false\" silent=\"false\"/>"
        ));
        assert!(!xml.contains("<actions>"));

        // (Note (upstream): a notification without a message fails; see
        // the FIXME in append_xml_text())
        let notification = Notification {
            title: Some("T".into()),
            ..Notification::default()
        };
        assert!(build_notification_xml(&notification, None).is_none());
    }

    #[test]
    fn toast_images_are_cleaned_up() {
        let _l = crate::test_support::test_lock();
        let mut state = state();
        let icon = Surface::new(4, 4, crate::video::pixels::PixelFormat::ARGB8888).unwrap();
        let png = text(&save_toast_image(&mut state, Some(&icon)).unwrap());
        let temp = png.strip_suffix(".png").unwrap().to_owned();
        let exists = |p: &str| std::path::Path::new(p).exists();
        assert!(exists(&png), "{png}");
        assert!(exists(&temp), "{temp}");
        cleanup_icons(&mut state);
        assert!(!exists(&png));
        // (upstream leaves GetTempFileNameW()'s file behind)
        assert!(!exists(&temp));
    }

    #[test]
    fn helpers() {
        assert_eq!(wcstoul(&utf8_to_wide("1234")), 1234);
        assert_eq!(wcstoul(&utf8_to_wide("  42x")), 42);
        assert_eq!(wcstoul(&utf8_to_wide("x")), 0);
        assert_eq!(wcstoul(&utf8_to_wide("99999999999")), u32::MAX);
        assert_eq!(text(&notification_tag(4000000000)), "4000000000");
        assert_eq!(hresult_from_win32(0), 0);
        assert_eq!(hresult_from_win32(5), 0x80070005_u32 as HRESULT);

        // A test binary isn't in an MSIX package.
        assert!(!is_in_package());
        // (the app metadata may be set by other tests)
        let exe = text(&exe_file_name().unwrap());
        assert!(exe.ends_with(".exe") && !exe.contains('\\'), "{exe}");
        assert!(get_full_path("C:\\nonexistent\\file.wav").is_none());
        let current = std::env::current_exe().unwrap();
        let full = get_full_path(&current.to_string_lossy()).unwrap();
        assert_eq!(
            text(&full).to_lowercase(),
            current.to_string_lossy().to_lowercase()
        );

        let mut random = [0u8; 16];
        assert_eq!(
            bcrypt_gen_random(&mut random, BCRYPT_USE_SYSTEM_PREFERRED_RNG),
            0
        );
        assert_ne!(random, [0; 16]);

        // Custom sounds aren't supported outside a package: the default
        // sound plays, or the file's URI if it exists.
        let mut audio = Vec::new();
        append_xml_audio(&mut audio, "nonexistent.wav");
        assert_eq!(
            text(&audio),
            "<audio src=\"ms-winsoundevent:Notification.Default\" loop=\"false\" silent=\"false\"/>"
        );
        let mut audio = Vec::new();
        append_xml_audio(&mut audio, &current.to_string_lossy());
        assert!(text(&audio).starts_with("<audio src=\"file:///"));
    }

    #[test]
    fn handlers_answer_query_interface() {
        let mut out: *mut c_void = std::ptr::null_mut();
        for (handler, iid, other) in [
            (
                &ON_ACTIVATED,
                IID_ITOAST_ACTIVATED_EVENT_HANDLER,
                IID_ITOAST_DISMISSED_EVENT_HANDLER,
            ),
            (
                &ON_DISMISSED,
                IID_ITOAST_DISMISSED_EVENT_HANDLER,
                IID_ITOAST_ACTIVATED_EVENT_HANDLER,
            ),
        ] {
            let this = handler.as_ptr();
            let vtbl = handler.lp_vtbl;
            // SAFETY: the handlers are static objects; the IIDs and
            // out-pointer are valid.
            unsafe {
                for good in [iid, IID_IAGILEOBJECT, IID_IUNKNOWN] {
                    assert_eq!((vtbl.query_interface)(this, &good, &mut out), S_OK);
                    assert_eq!(out, this.cast());
                    (vtbl.release)(this);
                }
                assert_eq!(
                    (vtbl.query_interface)(this, &other, &mut out),
                    E_NOINTERFACE
                );
                assert_eq!(
                    (vtbl.query_interface)(this, &iid, std::ptr::null_mut()),
                    E_POINTER
                );
            }
        }
    }

    /// Whatever the system supports: toasts on Windows 10 or later (shown
    /// and removed at once), an error without them (Wine).
    #[test]
    fn show_and_remove() {
        let _l = crate::test_support::test_lock();
        let notification = Notification {
            title: Some("SDL test".into()),
            message: Some("A test notification".into()),
            transient: true,
            ..Notification::default()
        };
        match request_notification_permission() {
            Ok(()) => {
                // (it may still fail to show, without a desktop)
                if let Ok(id) = show_notification(&notification) {
                    remove_notification(id).unwrap();
                }
            }
            Err(e) => {
                assert_eq!(e.to_string(), INIT_ERROR);
                assert_eq!(
                    show_notification(&notification).unwrap_err().to_string(),
                    INIT_ERROR
                );
                assert_eq!(remove_notification(1).unwrap_err().to_string(), INIT_ERROR);
            }
        }
        cleanup_notifications();
    }
}
