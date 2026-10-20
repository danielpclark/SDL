// Rust translation of src/dialog/windows/SDL_windowsdialog.c from Simple
// DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Windows file dialogs: `IFileDialog` (the "modern" dialogs, for
//! files and folders), falling back to `GetOpenFileNameW()` /
//! `GetSaveFileNameW()` and `SHBrowseForFolderW()` when it can't be set up.
//! Each dialog runs on its own detached thread, which calls the callback.
//! The COM interfaces aren't in `windows-sys`, so their vtables (from
//! `shobjidl.h`) are declared by hand.

use std::ffi::c_void;

use windows_sys::core::{GUID, HRESULT, PCWSTR, PWSTR};
use windows_sys::Win32::Foundation::{FreeLibrary, HWND, LPARAM, MAX_PATH};
use windows_sys::Win32::Globalization::{MultiByteToWideChar, CP_UTF8, MB_ERR_INVALID_CHARS};
use windows_sys::Win32::System::Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_INPROC_SERVER};
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryW};
use windows_sys::Win32::UI::Controls::Dialogs::{
    OFN_ALLOWMULTISELECT, OFN_EXPLORER, OFN_HIDEREADONLY, OFN_NOCHANGEDIR, OFN_OVERWRITEPROMPT,
    OPENFILENAMEW,
};
use windows_sys::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
use windows_sys::Win32::UI::Shell::{
    SHBrowseForFolderW, SHGetPathFromIDListW, BFFM_INITIALIZED, BFFM_SETSELECTIONW, BROWSEINFOW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::SendMessageW;

use super::{
    convert_filters, DialogFileCallback, DialogFileFilter, FileDialogOptions, FileDialogType,
};
use crate::core::windows::com::{ComObject, ComPtr, IUnknownVtbl};
use crate::core::windows::{
    co_initialize, co_uninitialize, set_error, utf8_to_wide, wide_char_to_multi_byte, wide_to_utf8,
};
use crate::error::{Error, Result};
use crate::events::WindowID;
use crate::video::window::PROP_WINDOW_WIN32_HWND_POINTER;

type Method = unsafe extern "system" fn();

/// `IShellItemVtbl`.
#[repr(C)]
struct IShellItemVtbl {
    base: IUnknownVtbl,
    bind_to_handler: Method,
    get_parent: Method,
    get_display_name: unsafe extern "system" fn(*mut IShellItem, i32, *mut PWSTR) -> HRESULT,
    get_attributes: Method,
    compare: Method,
}
type IShellItem = ComObject<IShellItemVtbl>;

/// `IShellItemArrayVtbl`.
#[repr(C)]
struct IShellItemArrayVtbl {
    base: IUnknownVtbl,
    bind_to_handler: Method,
    get_property_store: Method,
    get_property_description_list: Method,
    get_attributes: Method,
    get_count: unsafe extern "system" fn(*mut IShellItemArray, *mut u32) -> HRESULT,
    get_item_at:
        unsafe extern "system" fn(*mut IShellItemArray, u32, *mut *mut IShellItem) -> HRESULT,
    enum_items: Method,
}
type IShellItemArray = ComObject<IShellItemArrayVtbl>;

/// `IFileDialogVtbl`.
#[repr(C)]
struct IFileDialogVtbl {
    base: IUnknownVtbl,
    show: unsafe extern "system" fn(*mut IFileDialog, HWND) -> HRESULT,
    set_file_types:
        unsafe extern "system" fn(*mut IFileDialog, u32, *const COMDLG_FILTERSPEC) -> HRESULT,
    set_file_type_index: Method,
    get_file_type_index: unsafe extern "system" fn(*mut IFileDialog, *mut u32) -> HRESULT,
    advise: Method,
    unadvise: Method,
    set_options: unsafe extern "system" fn(*mut IFileDialog, u32) -> HRESULT,
    get_options: unsafe extern "system" fn(*mut IFileDialog, *mut u32) -> HRESULT,
    set_default_folder: Method,
    set_folder: unsafe extern "system" fn(*mut IFileDialog, *mut IShellItem) -> HRESULT,
    get_folder: Method,
    get_current_selection: Method,
    set_file_name: unsafe extern "system" fn(*mut IFileDialog, PCWSTR) -> HRESULT,
    get_file_name: Method,
    set_title: unsafe extern "system" fn(*mut IFileDialog, PCWSTR) -> HRESULT,
    set_ok_button_label: unsafe extern "system" fn(*mut IFileDialog, PCWSTR) -> HRESULT,
    set_file_name_label: Method,
    get_result: unsafe extern "system" fn(*mut IFileDialog, *mut *mut IShellItem) -> HRESULT,
    add_place: Method,
    set_default_extension: Method,
    close: Method,
    set_client_guid: Method,
    clear_client_data: Method,
    set_filter: Method,
}
type IFileDialog = ComObject<IFileDialogVtbl>;

/// `IFileOpenDialogVtbl`.
#[repr(C)]
struct IFileOpenDialogVtbl {
    base: IFileDialogVtbl,
    get_results:
        unsafe extern "system" fn(*mut IFileOpenDialog, *mut *mut IShellItemArray) -> HRESULT,
    get_selected_items: Method,
}
type IFileOpenDialog = ComObject<IFileOpenDialogVtbl>;

/// `IFileDialog2Vtbl` (the IFileDialog methods are called through the
/// IFileDialog pointer).
#[repr(C)]
struct IFileDialog2Vtbl {
    base: IUnknownVtbl,
    show: Method,
    set_file_types: Method,
    set_file_type_index: Method,
    get_file_type_index: Method,
    advise: Method,
    unadvise: Method,
    set_options: unsafe extern "system" fn(*mut IFileDialog2, u32) -> HRESULT,
    get_options: unsafe extern "system" fn(*mut IFileDialog2, *mut u32) -> HRESULT,
    set_default_folder: Method,
    set_folder: Method,
    get_folder: Method,
    get_current_selection: Method,
    set_file_name: Method,
    get_file_name: Method,
    set_title: Method,
    set_ok_button_label: Method,
    set_file_name_label: Method,
    get_result: Method,
    add_place: Method,
    set_default_extension: Method,
    close: Method,
    set_client_guid: Method,
    clear_client_data: Method,
    set_filter: Method,
    set_cancel_button_label: unsafe extern "system" fn(*mut IFileDialog2, PCWSTR) -> HRESULT,
    set_navigation_root: Method,
}
type IFileDialog2 = ComObject<IFileDialog2Vtbl>;

// Flags/GUIDs defined for compatibility with pre-Vista headers
const FOS_OVERWRITEPROMPT: u32 = 0x2;
const FOS_NOCHANGEDIR: u32 = 0x8;
const FOS_PICKFOLDERS: u32 = 0x20;
const FOS_ALLOWMULTISELECT: u32 = 0x200;
const SIGDN_FILESYSPATH: i32 = 0x80058000_u32 as i32;

/// `BIF_USENEWUI` (`BIF_NEWDIALOGSTYLE | BIF_EDITBOX`).
const BIF_USENEWUI: u32 = 0x0040 | 0x0010;

const fn guid(data1: u32, data2: u16, data3: u16, data4: [u8; 8]) -> GUID {
    GUID {
        data1,
        data2,
        data3,
        data4,
    }
}

const SDL_CLSID_FILE_OPEN_DIALOG: GUID = guid(
    0xdc1c5a9c,
    0xe88a,
    0x4dde,
    [0xa5, 0xa1, 0x60, 0xf8, 0x2a, 0x20, 0xae, 0xf7],
);
const SDL_CLSID_FILE_SAVE_DIALOG: GUID = guid(
    0xc0b4e2f3,
    0xba21,
    0x4773,
    [0x8d, 0xba, 0x33, 0x5e, 0xc9, 0x46, 0xeb, 0x8b],
);

const SDL_IID_IFILE_DIALOG: GUID = guid(
    0x42f85136,
    0xdb7e,
    0x439c,
    [0x85, 0xf1, 0xe4, 0x07, 0x5d, 0x13, 0x5f, 0xc8],
);
const SDL_IID_IFILE_DIALOG2: GUID = guid(
    0x61744fc7,
    0x85b5,
    0x4791,
    [0xa9, 0xb0, 0x27, 0x22, 0x76, 0x30, 0x9b, 0x13],
);
const SDL_IID_IFILE_OPEN_DIALOG: GUID = guid(
    0xd57c7288,
    0xd4ad,
    0x4768,
    [0xbe, 0x02, 0x9d, 0x96, 0x95, 0x32, 0xd9, 0x60],
);
/// `IID_IShellItem`.
const IID_ISHELL_ITEM: GUID = guid(
    0x43826d1e,
    0xe718,
    0x42ee,
    [0xbc, 0x55, 0xa1, 0xe2, 0x61, 0xc3, 0x7b, 0xfe],
);

/// `HRESULT_FROM_WIN32(ERROR_CANCELLED)`.
const HRESULT_CANCELLED: HRESULT = 0x800704C7_u32 as HRESULT;

// If this number is too small, selecting too many files will give an error
const SELECTLIST_SIZE: usize = 65536;

/// Translation of `winArgs`.
struct WinArgs {
    is_save: bool,
    filters_str: Option<Vec<u16>>,
    nfilters: usize,
    default_file: Option<String>,
    parent: Option<WindowID>,
    flags: u32,
    allow_many: bool,
    callback: DialogFileCallback,
    title: Option<String>,
    accept: Option<String>,
    cancel: Option<String>,
}

/// Translation of `winFArgs`.
struct WinFArgs {
    parent: Option<WindowID>,
    allow_many: bool,
    callback: DialogFileCallback,
    default_folder: Option<String>,
    title: Option<String>,
    accept: Option<String>,
    cancel: Option<String>,
}

/// Converts dialog.nFilterIndex to SDL-compatible value.
/// Translation of `getFilterIndex()` (-1, no filter, is `None`).
fn get_filter_index(as_reported_by_windows: i32) -> Option<usize> {
    usize::try_from(as_reported_by_windows - 1).ok()
}

/// Translation of `clear_filt_names()`.
fn clear_filt_names(filt: &str) -> String {
    /* 0x01 bytes are used as temporary replacement for the various 0x00
    bytes required by Win32 (one null byte between each filter, two at
    the end of the filters). Filter out these bytes from the filter names
    to avoid early-ending the filters if someone puts two consecutive
    0x01 bytes in their filter names. */
    filt.replace('\x01', " ")
}

/// The window's `HWND` (`SDL_PROP_WINDOW_WIN32_HWND_POINTER`), or NULL.
fn window_hwnd(window: Option<WindowID>) -> HWND {
    window
        .and_then(|id| crate::video::Window::from_id(id).ok())
        .and_then(|w| w.properties().ok())
        .and_then(|props| props.get_number(PROP_WINDOW_WIN32_HWND_POINTER))
        .unwrap_or(0) as HWND
}

/// The name and spec of each filter in a filter string from
/// [`win_get_filters`] (offsets of their NUL-terminated strings).
fn filter_specs(filter_wchar: &[u16], nfilters: usize) -> Vec<(usize, usize)> {
    let mut filter_ptr = 0usize;
    let wcslen = |at: usize| {
        filter_wchar[at..]
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(filter_wchar.len() - at)
    };
    let mut specs = Vec::with_capacity(nfilters);
    for _ in 0..nfilters {
        let name = filter_ptr;
        filter_ptr += wcslen(filter_ptr) + 1;
        let spec = filter_ptr;
        filter_ptr += wcslen(filter_ptr) + 1;
        specs.push((name, spec));
    }

    // assert(*filter_ptr == L'\0');
    specs
}

/// The folder and file name parts of a default location: the folder is
/// everything before the last separator (none without one), the file what
/// follows it (none if empty).
fn split_default_location(default_file: &str) -> (Option<Vec<u16>>, Option<Vec<u16>>) {
    let default_folder_w = utf8_to_wide(default_file);
    let len = default_folder_w.len() - 1;
    let last = default_folder_w[..len]
        .iter()
        .rposition(|&c| c == u16::from(b'/') || c == u16::from(b'\\'));

    match last {
        None => (None, Some(default_folder_w)),
        Some(i) => {
            let mut folder = default_folder_w[..i].to_vec();
            folder.push(0);
            let file = &default_folder_w[i + 1..];
            let file = (file.len() > 1).then(|| file.to_vec());
            (Some(folder), file)
        }
    }
}

/// The path of a shell item (`SIGDN_FILESYSPATH`).
fn shell_item_path(item: &ComPtr<IShellItemVtbl>) -> std::result::Result<String, HRESULT> {
    let mut file_path: PWSTR = std::ptr::null_mut();
    // SAFETY: the item is live; the out-pointer receives a CoTaskMemAlloc()ed string.
    let hr =
        unsafe { (item.vtbl().get_display_name)(item.as_ptr(), SIGDN_FILESYSPATH, &mut file_path) };
    if hr < 0 {
        return Err(hr);
    }
    // SAFETY: the string is NUL-terminated, and ours to free.
    unsafe {
        let len = (0..).take_while(|&i| *file_path.add(i) != 0).count();
        let path = wide_to_utf8(std::slice::from_raw_parts(file_path, len));
        CoTaskMemFree(file_path.cast());
        Ok(path)
    }
}

/// Translation of `windows_ShowModernFileFolderDialog()`.
///
/// This function returns NOT success or error, but rather whether the callback
/// was invoked or not (and if it was, no fallback should be attempted to prevent
/// calling the callback twice). See https://github.com/libsdl-org/SDL/issues/15194
/// (here: the callback back when it wasn't invoked).
#[allow(clippy::too_many_arguments)]
fn show_modern_file_folder_dialog(
    dialog_type: FileDialogType,
    default_file: Option<&str>,
    parent: HWND,
    mut allow_many: bool,
    callback: DialogFileCallback,
    title: Option<&str>,
    accept: Option<&str>,
    cancel: Option<&str>,
    filter_wchar: Option<&[u16]>,
    nfilters: usize,
) -> Option<DialogFileCallback> {
    let is_save = dialog_type == FileDialogType::SaveFile;
    let is_folder = dialog_type == FileDialogType::OpenFolder;

    if is_save {
        // Just in case; the code relies on that
        allow_many = false;
    }

    type PfnSHCreateItemFromParsingName =
        unsafe extern "system" fn(PCWSTR, *mut c_void, *const GUID, *mut *mut c_void) -> HRESULT;

    let mut call_callback_on_error = false;
    let mut co_init = false;
    let mut callback = Some(callback);

    // (the body up to `quit:`; Err means `goto quit`)
    let _ = (|| -> std::result::Result<(), ()> {
        // We can assume shell32 is already loaded here.
        let shell32_name = utf8_to_wide("shell32.dll");
        // SAFETY: the name is NUL-terminated.
        let shell32_handle = unsafe { GetModuleHandleW(shell32_name.as_ptr()) };
        if shell32_handle.is_null() {
            return Err(());
        }

        // SAFETY: the name is NUL-terminated; the export has this signature.
        let p_sh_create_item_from_parsing_name: PfnSHCreateItemFromParsingName = unsafe {
            match GetProcAddress(
                shell32_handle,
                c"SHCreateItemFromParsingName".as_ptr().cast(),
            ) {
                Some(f) => std::mem::transmute::<
                    unsafe extern "system" fn() -> isize,
                    PfnSHCreateItemFromParsingName,
                >(f),
                None => return Err(()),
            }
        };

        let mut filter_data: Vec<COMDLG_FILTERSPEC> = Vec::new();
        if let Some(filter_wchar) = filter_wchar.filter(|_| nfilters > 0) {
            for (name, spec) in filter_specs(filter_wchar, nfilters) {
                filter_data.push(COMDLG_FILTERSPEC {
                    pszName: filter_wchar[name..].as_ptr(),
                    pszSpec: filter_wchar[spec..].as_ptr(),
                });
            }
        }

        let title_w = title.map(utf8_to_wide);
        let accept_w = accept.map(utf8_to_wide);
        let cancel_w = cancel.map(utf8_to_wide);

        let (default_folder_w, default_file_w) = match default_file {
            Some(default_file) => split_default_location(default_file),
            None => (None, None),
        };

        macro_rules! check {
            ($op:expr) => {
                if $op < 0 {
                    return Err(());
                }
            };
        }

        check!(co_initialize());

        co_init = true;

        // SAFETY: the out-pointer receives an IFileDialog on success.
        let p_file_dialog = unsafe {
            ComPtr::<IFileDialogVtbl>::from_out(|out| {
                CoCreateInstance(
                    if is_save {
                        &SDL_CLSID_FILE_SAVE_DIALOG
                    } else {
                        &SDL_CLSID_FILE_OPEN_DIALOG
                    },
                    std::ptr::null_mut(),
                    CLSCTX_INPROC_SERVER,
                    &SDL_IID_IFILE_DIALOG,
                    out.cast(),
                )
            })
        }
        .map_err(|_| ())?;
        let p_file_dialog2 = p_file_dialog
            .query::<IFileDialog2Vtbl>(&SDL_IID_IFILE_DIALOG2)
            .map_err(|_| ())?;

        let p_file_open_dialog = if allow_many {
            Some(
                p_file_dialog
                    .query::<IFileOpenDialogVtbl>(&SDL_IID_IFILE_OPEN_DIALOG)
                    .map_err(|_| ())?,
            )
        } else {
            None
        };

        let fd = p_file_dialog.as_ptr();
        let fd_vtbl = p_file_dialog.vtbl();
        let fd2 = p_file_dialog2.as_ptr();
        let fd2_vtbl = p_file_dialog2.vtbl();

        let mut pfos = 0u32;
        // SAFETY: the dialog is live; pfos is valid.
        check!(unsafe { (fd2_vtbl.get_options)(fd2, &mut pfos) });

        pfos |= FOS_NOCHANGEDIR;
        if allow_many {
            pfos |= FOS_ALLOWMULTISELECT;
        }
        if is_save {
            pfos |= FOS_OVERWRITEPROMPT;
        }
        if is_folder {
            pfos |= FOS_PICKFOLDERS;
        }

        // SAFETY: the dialog is live; the strings are NUL-terminated and
        // the filter specs point into filter_wchar, all outliving the calls.
        unsafe {
            check!((fd2_vtbl.set_options)(fd2, pfos));

            if let Some(cancel_w) = &cancel_w {
                check!((fd2_vtbl.set_cancel_button_label)(fd2, cancel_w.as_ptr()));
            }

            if let Some(accept_w) = &accept_w {
                check!((fd_vtbl.set_ok_button_label)(fd, accept_w.as_ptr()));
            }

            if let Some(title_w) = &title_w {
                check!((fd_vtbl.set_title)(fd, title_w.as_ptr()));
            }

            if !filter_data.is_empty() {
                check!((fd_vtbl.set_file_types)(
                    fd,
                    nfilters as u32,
                    filter_data.as_ptr()
                ));
            }

            if let Some(default_folder_w) = &default_folder_w {
                let p_folder_item = ComPtr::<IShellItemVtbl>::from_out(|out| {
                    p_sh_create_item_from_parsing_name(
                        default_folder_w.as_ptr(),
                        std::ptr::null_mut(),
                        &IID_ISHELL_ITEM,
                        out.cast(),
                    )
                })
                .map_err(|_| ())?;
                check!((fd_vtbl.set_folder)(fd, p_folder_item.as_ptr()));
            }

            if let Some(default_file_w) = &default_file_w {
                check!((fd_vtbl.set_file_name)(fd, default_file_w.as_ptr()));
            }
        }

        // Right after this, a dialog is shown. No fallback should be attempted on
        // error to prevent showing two dialogs to the user.
        call_callback_on_error = true;

        // (upstream has the same code for a parent window and none, with
        // a NULL owner for none; the caller looks up the parent's HWND)
        let window = parent;

        // SAFETY: the dialog is live; the owner is a window or NULL.
        let hr = unsafe { (fd_vtbl.show)(fd, window) };

        if hr == HRESULT_CANCELLED {
            let mut selected_filter = 0u32;

            // This is a one-based index, not zero-based. Doc link in similar comment below
            // SAFETY: the dialog is live.
            check!(unsafe { (fd_vtbl.get_file_type_index)(fd, &mut selected_filter) });
            let callback = callback.take().expect("not called yet");
            callback(Ok(Vec::new()), get_filter_index(selected_filter as i32));
            return Ok(());
        } else if hr < 0 {
            return Err(());
        }

        if let Some(p_file_open_dialog) = p_file_open_dialog {
            let mut n_results = 0u32;
            let mut selected_filter = 0u32;

            // SAFETY: the dialogs are live; the out-pointers are valid.
            let p_item_array = unsafe {
                ComPtr::<IShellItemArrayVtbl>::from_out(|out| {
                    (p_file_open_dialog.vtbl().get_results)(p_file_open_dialog.as_ptr(), out)
                })
            }
            .map_err(|_| ())?;
            // SAFETY: as above.
            unsafe {
                check!((fd_vtbl.get_file_type_index)(fd, &mut selected_filter));
                check!((p_item_array.vtbl().get_count)(
                    p_item_array.as_ptr(),
                    &mut n_results
                ));
            }

            let mut files = Vec::with_capacity(n_results as usize);

            for i in 0..n_results {
                // SAFETY: the array is live; the out-pointer receives an item.
                let p_item = unsafe {
                    ComPtr::<IShellItemVtbl>::from_out(|out| {
                        (p_item_array.vtbl().get_item_at)(p_item_array.as_ptr(), i, out)
                    })
                }
                .map_err(|_| ())?;
                files.push(shell_item_path(&p_item).map_err(|_| ())?);
            }

            let callback = callback.take().expect("not called yet");
            callback(Ok(files), get_filter_index(selected_filter as i32));
        } else {
            // This is a one-based index, not zero-based.
            // https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-ifiledialog-getfiletypeindex#parameters
            let mut selected_filter = 0u32;

            // SAFETY: the dialog is live; the out-pointers are valid.
            let p_item =
                unsafe { ComPtr::<IShellItemVtbl>::from_out(|out| (fd_vtbl.get_result)(fd, out)) }
                    .map_err(|_| ())?;
            // SAFETY: as above.
            check!(unsafe { (fd_vtbl.get_file_type_index)(fd, &mut selected_filter) });
            let file = shell_item_path(&p_item).map_err(|_| ())?;

            let callback = callback.take().expect("not called yet");
            callback(Ok(vec![file]), get_filter_index(selected_filter as i32));
        }

        Ok(())
    })();

    // quit:
    // (the COM objects were released at the end of the body, before
    // CoUninitialize() here, where upstream releases them after it)
    if call_callback_on_error {
        if let Some(cb) = callback.take() {
            cb(Err(set_error("dialog")), None);
        }
    }

    if co_init {
        co_uninitialize();
    }

    callback
}

/// Translation of `windows_ShowFileDialog()`.
// TODO: The new version of file dialogs
fn show_file_dialog_thread(args: WinArgs) {
    let WinArgs {
        is_save,
        filters_str: filter_wchar,
        nfilters,
        default_file,
        parent,
        flags,
        allow_many,
        callback,
        title,
        accept,
        cancel,
    } = args;

    let Some(callback) = show_modern_file_folder_dialog(
        if is_save {
            FileDialogType::SaveFile
        } else {
            FileDialogType::OpenFile
        },
        default_file.as_deref(),
        window_hwnd(parent),
        allow_many,
        callback,
        title.as_deref(),
        accept.as_deref(),
        cancel.as_deref(),
        filter_wchar.as_deref(),
        nfilters,
    ) else {
        return;
    };

    /* GetOpenFileName and GetSaveFileName have the same signature
    (yes, LPOPENFILENAMEW even for the save dialog) */
    type PfnGetAnyFileNameW = unsafe extern "system" fn(*mut OPENFILENAMEW) -> i32;
    type PfnCommDlgExtendedError = unsafe extern "system" fn() -> u32;
    let lib_name = utf8_to_wide("Comdlg32.dll");
    // SAFETY: the name is NUL-terminated.
    let lib = unsafe { LoadLibraryW(lib_name.as_ptr()) };

    if lib.is_null() {
        return callback(Err(Error::new("Couldn't load Comdlg32.dll")), None);
    }
    // (upstream never frees the library)
    struct Library(windows_sys::Win32::Foundation::HMODULE);
    impl Drop for Library {
        fn drop(&mut self) {
            // SAFETY: balances the LoadLibraryW() above.
            unsafe { FreeLibrary(self.0) };
        }
    }
    let lib = Library(lib);

    // SAFETY: the names are NUL-terminated; the exports have these signatures.
    let (p_get_any_file_name, p_comm_dlg_extended_error) = unsafe {
        (
            GetProcAddress(
                lib.0,
                if is_save {
                    c"GetSaveFileNameW".as_ptr().cast()
                } else {
                    c"GetOpenFileNameW".as_ptr().cast()
                },
            )
            .map(|f| {
                std::mem::transmute::<unsafe extern "system" fn() -> isize, PfnGetAnyFileNameW>(f)
            }),
            GetProcAddress(lib.0, c"CommDlgExtendedError".as_ptr().cast()).map(|f| {
                std::mem::transmute::<unsafe extern "system" fn() -> isize, PfnCommDlgExtendedError>(
                    f,
                )
            }),
        )
    };

    let Some(p_get_any_file_name) = p_get_any_file_name else {
        return callback(
            Err(Error::new(
                "Couldn't load GetOpenFileName/GetSaveFileName from library",
            )),
            None,
        );
    };

    let Some(p_comm_dlg_extended_error) = p_comm_dlg_extended_error else {
        return callback(
            Err(Error::new(
                "Couldn't load CommDlgExtendedError from library",
            )),
            None,
        );
    };

    let window = window_hwnd(parent);

    // Necessary for the return code below
    let mut filebuffer = vec![0u16; SELECTLIST_SIZE]; // lpstrFile
    let mut initfolder = [0u16; MAX_PATH as usize]; // lpstrInitialDir

    if let Some(default_file) = default_file.as_deref().filter(|f| !f.is_empty()) {
        // Note (upstream): an empty default location reads the byte before
        // the string; it is no default location here.
        legacy_initial_paths(default_file, &mut initfolder, &mut filebuffer);
    }

    let title_w = title.as_deref().map(utf8_to_wide);

    // SAFETY: OPENFILENAMEW is plain data; all zeroes is valid.
    let mut dialog: OPENFILENAMEW = unsafe { std::mem::zeroed() };
    dialog.lStructSize = size_of::<OPENFILENAMEW>() as u32;
    dialog.hwndOwner = window;
    dialog.hInstance = std::ptr::null_mut();
    dialog.lpstrFilter = filter_wchar
        .as_ref()
        .map_or(std::ptr::null(), |f| f.as_ptr());
    dialog.lpstrCustomFilter = std::ptr::null_mut();
    dialog.nMaxCustFilter = 0;
    dialog.nFilterIndex = 0;
    dialog.lpstrFile = filebuffer.as_mut_ptr();
    dialog.nMaxFile = SELECTLIST_SIZE as u32;
    dialog.lpstrFileTitle = std::ptr::null_mut();
    dialog.lpstrInitialDir = if initfolder[0] != 0 {
        initfolder.as_ptr()
    } else {
        std::ptr::null()
    };
    dialog.lpstrTitle = title_w.as_ref().map_or(std::ptr::null(), |t| t.as_ptr());
    dialog.Flags = flags | OFN_EXPLORER | OFN_HIDEREADONLY | OFN_NOCHANGEDIR;
    dialog.nFileOffset = 0;
    dialog.nFileExtension = 0;
    dialog.lpstrDefExt = std::ptr::null();
    dialog.lCustData = 0;
    dialog.lpfnHook = None;
    dialog.lpTemplateName = std::ptr::null();
    // Skipped many mac-exclusive and reserved members
    dialog.FlagsEx = 0;

    // SAFETY: the structure and the buffers it points to outlive the call.
    let result = unsafe { p_get_any_file_name(&mut dialog) };

    if result != 0 {
        if flags & OFN_ALLOWMULTISELECT == 0 {
            // File is a C string stored in dialog.lpstrFile
            let chosen_file = wide_to_utf8(&filebuffer);
            callback(
                Ok(vec![chosen_file]),
                get_filter_index(dialog.nFilterIndex as i32),
            );
        } else {
            match parse_multiselect(&filebuffer) {
                Ok(files) => callback(Ok(files), get_filter_index(dialog.nFilterIndex as i32)),
                Err(e) => callback(Err(e), None),
            }
        }
    } else {
        // SAFETY: CommDlgExtendedError takes no arguments.
        let error = unsafe { p_comm_dlg_extended_error() };
        // Error code 0 means the user clicked the cancel button.
        if error == 0 {
            /* Unlike SDL's handling of errors, Windows does reset the error
            code to 0 after calling GetOpenFileName if another Windows
            function before set a different error code, so it's safe to
            check for success. */
            callback(Ok(Vec::new()), get_filter_index(dialog.nFilterIndex as i32));
        } else {
            // SAFETY: as above.
            let error = unsafe { p_comm_dlg_extended_error() };
            callback(
                Err(Error::new(format!(
                    "Windows error, CommDlgExtendedError: {error}"
                ))),
                None,
            );
        }
    }
}

/// The starting point of `GetOpenFileNameW()`: a location ending with a
/// separator goes in `initfolder` (`lpstrInitialDir`), anything else in
/// `filebuffer` (`lpstrFile`) with forward slashes made backslashes; both
/// hold at most `MAX_PATH` characters (an invalid or too long location
/// leaves them empty, as `MultiByteToWideChar()` does).
fn legacy_initial_paths(default_file: &str, initfolder: &mut [u16], filebuffer: &mut [u16]) {
    /* On Windows 10, 11 and possibly others, lpstrFile can be initialized
    with a path and the dialog will start at that location, but *only if
    the path contains a filename*. If it ends with a folder (directory
    separator), it fails with 0x3002 (12290) FNERR_INVALIDFILENAME. For
    that specific case, lpstrInitialDir must be used instead, but just
    for that case, because lpstrInitialDir doesn't support file names.

    On top of that, lpstrInitialDir hides a special algorithm that
    decides which folder to actually use as starting point, which may or
    may not be the one provided, or some other unrelated folder. Also,
    the algorithm changes between platforms. Assuming the documentation
    is correct, the algorithm is there under 'lpstrInitialDir':

    https://learn.microsoft.com/en-us/windows/win32/api/commdlg/ns-commdlg-openfilenamew

    Finally, lpstrFile does not support forward slashes. lpstrInitialDir
    does, though. */

    let last_c = default_file.as_bytes()[default_file.len() - 1];
    let mut source = default_file.as_bytes().to_vec();
    source.push(0);

    let target = if last_c == b'\\' || last_c == b'/' {
        &mut initfolder[..MAX_PATH as usize]
    } else {
        &mut filebuffer[..MAX_PATH as usize]
    };
    // SAFETY: the source is NUL-terminated; the target holds MAX_PATH
    // characters.
    unsafe {
        MultiByteToWideChar(
            CP_UTF8,
            MB_ERR_INVALID_CHARS,
            source.as_ptr(),
            -1,
            target.as_mut_ptr(),
            MAX_PATH as i32,
        );
    }

    if !(last_c == b'\\' || last_c == b'/') {
        for c in filebuffer.iter_mut() {
            if *c == u16::from(b'/') {
                *c = u16::from(b'\\');
            }
        }
    }
}

/// The files of a multiple-selection `GetOpenFileNameW()` result.
fn parse_multiselect(file_buffer: &[u16]) -> Result<Vec<String>> {
    /* File is either a C string if the user chose a single file, else
    it's a series of strings formatted like:

        "C:\\path\\to\\folder\0filename1.ext\0filename2.ext\0\0"

    The code below will only stop on a double NULL in all cases, so
    it is important that the rest of the buffer has been zeroed. */
    let wcslen = |at: usize| {
        file_buffer[at..]
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(file_buffer.len() - at)
    };
    let mut file_ptr = 0usize;
    let mut chosen_files_list = Vec::new();

    let path_error = || Error::new("Path too long or invalid character in path");

    // (WideCharToMultiByte() into a MAX_PATH buffer, with its NUL)
    let to_utf8 = |w: &[u16], room: usize| -> Option<String> {
        let mut with_nul = w.to_vec();
        with_nul.push(0);
        let mut bytes = wide_char_to_multi_byte(CP_UTF8, 0, &with_nul)?;
        if bytes.len() > room {
            return None;
        }
        bytes.pop();
        String::from_utf8(bytes).ok()
    };

    let folder_len = wcslen(file_ptr);
    let chosen_folder =
        to_utf8(&file_buffer[..folder_len], MAX_PATH as usize).ok_or_else(path_error)?;

    let chosen_folder_size = chosen_folder.len();
    let mut chosen_file = chosen_folder.clone();
    chosen_file.push('\\');

    file_ptr += folder_len + 1;

    while file_ptr < file_buffer.len() && file_buffer[file_ptr] != 0 {
        let diff = chosen_folder_size + 1;
        let len = wcslen(file_ptr);

        let name = to_utf8(
            &file_buffer[file_ptr..file_ptr + len],
            MAX_PATH as usize - diff,
        )
        .ok_or_else(path_error)?;

        file_ptr += len + 1;

        chosen_file.truncate(diff);
        chosen_file.push_str(&name);
        chosen_files_list.push(chosen_file.clone());
    }

    // If the user chose only one file, it's all just one string
    if chosen_files_list.is_empty() {
        chosen_files_list.push(chosen_folder);
    }

    Ok(chosen_files_list)
}

/// Translation of `windows_file_dialog_thread()`.
fn windows_file_dialog_thread(args: WinArgs) -> i32 {
    show_file_dialog_thread(args);
    0
}

/// Translation of `browse_callback_proc()`.
unsafe extern "system" fn browse_callback_proc(
    hwnd: HWND,
    u_msg: u32,
    _l_param: LPARAM,
    lp_data: LPARAM,
) -> i32 {
    // (BFFM_SELCHANGED, BFFM_VALIDATEFAILED and the rest: nothing)
    if u_msg == BFFM_INITIALIZED && lp_data != 0 {
        // SAFETY: lp_data is the NUL-terminated folder passed below.
        unsafe { SendMessageW(hwnd, BFFM_SETSELECTIONW, 1, lp_data) };
    }
    0
}

/// Translation of `windows_ShowFolderDialog()`.
fn show_folder_dialog_thread(args: WinFArgs) {
    let WinFArgs {
        parent: window,
        allow_many,
        callback,
        default_folder,
        title,
        accept,
        cancel,
    } = args;

    let Some(callback) = show_modern_file_folder_dialog(
        FileDialogType::OpenFolder,
        default_folder.as_deref(),
        window_hwnd(window),
        allow_many,
        callback,
        title.as_deref(),
        accept.as_deref(),
        cancel.as_deref(),
        None,
        0,
    ) else {
        return;
    };

    let parent = window_hwnd(window);

    let title_w = title.as_deref().map(utf8_to_wide);

    let mut buffer = [0u16; MAX_PATH as usize];

    // Note (upstream): the default folder is sent as UTF-8 to
    // BFFM_SETSELECTION, which (in a Unicode build) is BFFM_SETSELECTIONW
    // and reads UTF-16; it is sent as UTF-16 here.
    let default_folder_w = default_folder.as_deref().map(utf8_to_wide);

    // SAFETY: BROWSEINFOW is plain data; all zeroes is valid.
    let mut dialog: BROWSEINFOW = unsafe { std::mem::zeroed() };
    dialog.hwndOwner = parent;
    dialog.pidlRoot = std::ptr::null_mut();
    dialog.pszDisplayName = buffer.as_mut_ptr();
    dialog.lpszTitle = title_w.as_ref().map_or(std::ptr::null(), |t| t.as_ptr());
    dialog.ulFlags = BIF_USENEWUI;
    dialog.lpfn = Some(browse_callback_proc);
    dialog.lParam = default_folder_w
        .as_ref()
        .map_or(0, |f| f.as_ptr() as LPARAM);
    dialog.iImage = 0;

    // SAFETY: the structure and the buffers it points to outlive the call.
    let lp_item = unsafe { SHBrowseForFolderW(&dialog) };

    if !lp_item.is_null() {
        // SAFETY: lp_item is the chosen item, ours to free; buffer holds
        // MAX_PATH characters.
        unsafe {
            SHGetPathFromIDListW(lp_item, buffer.as_mut_ptr());
            // Note (upstream): the item list is never freed.
            CoTaskMemFree(lp_item.cast());
        }
        let chosen_file = wide_to_utf8(&buffer);
        callback(Ok(vec![chosen_file]), None);
    } else {
        callback(Ok(Vec::new()), None);
    }
}

/// Translation of `windows_folder_dialog_thread()`.
fn windows_folder_dialog_thread(args: WinFArgs) -> i32 {
    show_folder_dialog_thread(args);
    0
}

/// Translation of `win_get_filters()`: the filters as Win32 wants them,
/// `name\0*.ext;*.ext\0...\0\0` in UTF-16 (`None` without filters).
fn win_get_filters(filters: &[DialogFileFilter]) -> Result<Option<Vec<u16>>> {
    if filters.is_empty() {
        // (upstream: a NULL filter list)
        return Ok(None);
    }

    // '\x01' is used in place of a null byte
    // suffix needs two null bytes in case the filter list is empty
    let filterlist = convert_filters(
        filters,
        Some(clear_filt_names),
        "",
        "",
        "\x01\x01",
        "",
        "\x01",
        "\x01",
        "*.",
        ";*.",
        "",
        false,
    )?;

    let filterlist = filterlist.replace('\x01', "\0");

    Ok(Some(filterlist.encode_utf16().collect()))
}

/// Run `f(args)` on a detached thread named `name`, or call the callback
/// with an error if the thread can't be created.
fn spawn_dialog_thread<A: Send + 'static>(
    name: &str,
    args: A,
    f: fn(A) -> i32,
    take_callback: fn(A) -> DialogFileCallback,
) {
    let slot = std::sync::Arc::new(std::sync::Mutex::new(Some(args)));
    let thread_slot = slot.clone();
    let spawned = std::thread::Builder::new()
        .name(name.into())
        .spawn(move || {
            let args = thread_slot.lock().unwrap_or_else(|e| e.into_inner()).take();
            if let Some(args) = args {
                f(args);
            }
        });

    if spawned.is_err() {
        // The thread won't have run, therefore the data won't have been freed
        let args = slot.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(args) = args {
            take_callback(args)(Err(Error::new("Couldn't create thread")), None);
        }
    }
    // (detached: the JoinHandle is dropped, SDL_DetachThread())
}

/// Translation of `ShowFileDialog()`.
#[allow(clippy::too_many_arguments)]
fn show_file_dialog_impl(
    callback: DialogFileCallback,
    window: Option<WindowID>,
    filters: &[DialogFileFilter],
    default_location: Option<String>,
    allow_many: bool,
    is_save: bool,
    title: Option<String>,
    accept: Option<String>,
    cancel: Option<String>,
) {
    if crate::hints::get(crate::hints::FILE_DIALOG_DRIVER).is_some() {
        return callback(Err(Error::new("File dialog driver unsupported")), None);
    }

    let filters_str = match win_get_filters(filters) {
        Ok(filters_str) => filters_str,
        Err(e) => return callback(Err(e), None),
    };

    let mut flags = 0u32;
    if allow_many {
        flags |= OFN_ALLOWMULTISELECT;
    }
    if is_save {
        flags |= OFN_OVERWRITEPROMPT;
    }

    let args = WinArgs {
        is_save,
        filters_str,
        nfilters: filters.len(),
        default_file: default_location,
        parent: window,
        flags,
        allow_many,
        callback,
        title,
        accept,
        cancel,
    };

    spawn_dialog_thread(
        "SDL_Windows_ShowFileDialog",
        args,
        windows_file_dialog_thread,
        |args| args.callback,
    );
}

/// Translation of `ShowFolderDialog()`.
#[allow(clippy::too_many_arguments)]
fn show_folder_dialog_impl(
    callback: DialogFileCallback,
    window: Option<WindowID>,
    default_location: Option<String>,
    allow_many: bool,
    title: Option<String>,
    accept: Option<String>,
    cancel: Option<String>,
) {
    if crate::hints::get(crate::hints::FILE_DIALOG_DRIVER).is_some() {
        return callback(Err(Error::new("File dialog driver unsupported")), None);
    }

    let args = WinFArgs {
        parent: window,
        allow_many,
        callback,
        default_folder: default_location,
        title,
        accept,
        cancel,
    };

    spawn_dialog_thread(
        "SDL_Windows_ShowFolderDialog",
        args,
        windows_folder_dialog_thread,
        |args| args.callback,
    );
}

/// Translation of `SDL_SYS_ShowFileDialogWithProperties()` (Windows).
pub(super) fn show_file_dialog(
    dialog_type: FileDialogType,
    callback: DialogFileCallback,
    options: FileDialogOptions,
) {
    /* The internal functions will start threads, and the properties may be freed as soon as this function returns.
    Save a copy of what we need before invoking the functions and starting the threads. */
    let FileDialogOptions {
        filters,
        window,
        location: default_location,
        many: allow_many,
        title,
        accept,
        cancel,
    } = options;

    match dialog_type {
        FileDialogType::SaveFile | FileDialogType::OpenFile => show_file_dialog_impl(
            callback,
            window,
            &filters,
            default_location,
            allow_many,
            dialog_type == FileDialogType::SaveFile,
            title,
            accept,
            cancel,
        ),

        FileDialogType::OpenFolder => show_folder_dialog_impl(
            callback,
            window,
            default_location,
            allow_many,
            title,
            accept,
            cancel,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn filter_strings() {
        let filters = [
            DialogFileFilter::new("Images\x01x", "png;jpg"),
            DialogFileFilter::new("All", "*"),
        ];
        let w = win_get_filters(&filters).unwrap().unwrap();
        assert_eq!(w, wide("Images x\0*.png;*.jpg\0All\0*.*\0\0\0"));
        let specs = filter_specs(&w, 2);
        let at = |i: usize| wide_to_utf8(&w[i..]);
        assert_eq!(at(specs[0].0), "Images x");
        assert_eq!(at(specs[0].1), "*.png;*.jpg");
        assert_eq!(at(specs[1].0), "All");
        assert_eq!(at(specs[1].1), "*.*");
        assert_eq!(win_get_filters(&[]).unwrap(), None);
        assert_eq!(
            win_get_filters(&[DialogFileFilter::new("Bad", "a b")])
                .unwrap_err()
                .to_string(),
            "Invalid character ' ' in pattern (Only [a-zA-Z0-9_.-] allowed, or a single *)"
        );
        assert_eq!(clear_filt_names("a\x01\x01b"), "a  b");
    }

    #[test]
    fn default_locations() {
        let s = |w: Option<Vec<u16>>| w.map(|w| wide_to_utf8(&w));
        let split = |p: &str| {
            let (folder, file) = split_default_location(p);
            (s(folder), s(file))
        };
        assert_eq!(split("file.txt"), (None, Some("file.txt".into())));
        assert_eq!(
            split("C:\\dir/sub\\file.txt"),
            (Some("C:\\dir/sub".into()), Some("file.txt".into()))
        );
        assert_eq!(split("C:\\dir\\"), (Some("C:\\dir".into()), None));

        let mut initfolder = [0u16; MAX_PATH as usize];
        let mut filebuffer = vec![0u16; SELECTLIST_SIZE];
        legacy_initial_paths("C:/dir/file.txt", &mut initfolder, &mut filebuffer);
        assert_eq!(wide_to_utf8(&filebuffer), "C:\\dir\\file.txt");
        assert_eq!(initfolder[0], 0);
        let mut filebuffer = vec![0u16; SELECTLIST_SIZE];
        legacy_initial_paths("C:/dir/", &mut initfolder, &mut filebuffer);
        assert_eq!(wide_to_utf8(&initfolder), "C:/dir/");
        assert_eq!(filebuffer[0], 0);

        assert_eq!(get_filter_index(0), None);
        assert_eq!(get_filter_index(1), Some(0));
        assert_eq!(get_filter_index(3), Some(2));
    }

    #[test]
    fn multiselect_results() {
        let mut buffer = wide("C:\\dir\0a.txt\0b.png\0\0");
        buffer.resize(64, 0);
        assert_eq!(
            parse_multiselect(&buffer).unwrap(),
            ["C:\\dir\\a.txt", "C:\\dir\\b.png"]
        );
        // A single file is one string.
        let mut buffer = wide("C:\\dir\\only.txt\0\0");
        buffer.resize(64, 0);
        assert_eq!(parse_multiselect(&buffer).unwrap(), ["C:\\dir\\only.txt"]);
        // Paths are limited to MAX_PATH.
        let mut buffer = wide("C:\\dir\0");
        buffer.extend(wide(&"x".repeat(300)));
        buffer.resize(buffer.len() + 2, 0);
        assert_eq!(
            parse_multiselect(&buffer).unwrap_err().to_string(),
            "Path too long or invalid character in path"
        );
    }

    /// With `SDL_HINT_FILE_DIALOG_DRIVER` set, no dialog is shown: the
    /// callback reports the error at once.
    #[test]
    fn driver_hint_is_unsupported() {
        let _l = crate::test_support::test_lock();
        crate::hints::set(crate::hints::FILE_DIALOG_DRIVER, "zenity").unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        for dialog_type in [
            FileDialogType::OpenFile,
            FileDialogType::SaveFile,
            FileDialogType::OpenFolder,
        ] {
            let tx = tx.clone();
            super::super::show_file_dialog(
                dialog_type,
                FileDialogOptions::default(),
                move |result, filter| {
                    tx.send((result.map_err(|e| e.to_string()), filter))
                        .unwrap();
                },
            );
            assert_eq!(
                rx.try_recv().unwrap(),
                (Err("File dialog driver unsupported".to_string()), None)
            );
        }
        crate::hints::reset(crate::hints::FILE_DIALOG_DRIVER);
    }

    /// A dialog whose owner window is gone fails to show, and the callback
    /// gets the error (the dialog never appears).
    #[test]
    fn invalid_parent_fails() {
        use windows_sys::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow};

        let class = utf8_to_wide("STATIC");
        // SAFETY: a plain hidden window, destroyed at once: its handle is
        // then invalid.
        let stale = unsafe {
            let hwnd = CreateWindowExW(
                0,
                class.as_ptr(),
                std::ptr::null(),
                0,
                0,
                0,
                10,
                10,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            );
            assert!(!hwnd.is_null());
            assert_ne!(DestroyWindow(hwnd), 0);
            hwnd
        };

        // (if a dialog did appear, the test would wait for it: fail instead)
        let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let watched = done.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(60));
            if !watched.load(std::sync::atomic::Ordering::Relaxed) {
                eprintln!("invalid_parent_fails: a dialog was shown");
                std::process::exit(101);
            }
        });

        let filters = win_get_filters(&[DialogFileFilter::new("Text", "txt")])
            .unwrap()
            .unwrap();
        // (an existing folder, which the dialog is set to)
        let location = format!("{}\\name.txt", std::env::temp_dir().display());
        for dialog_type in [
            FileDialogType::OpenFile,
            FileDialogType::SaveFile,
            FileDialogType::OpenFolder,
        ] {
            let (tx, rx) = std::sync::mpsc::channel();
            // (no filters for a folder dialog, as show_folder_dialog()
            // passes none: IFileDialog::SetFileTypes() fails with
            // E_UNEXPECTED on a folder picker)
            let (filters, nfilters) = if dialog_type == FileDialogType::OpenFolder {
                (None, 0)
            } else {
                (Some(&filters[..]), 1)
            };
            let left = show_modern_file_folder_dialog(
                dialog_type,
                Some(&location),
                stale,
                dialog_type == FileDialogType::OpenFile,
                Box::new(move |result, filter| {
                    tx.send((result.map_err(|e| e.to_string()), filter))
                        .unwrap();
                }),
                Some("Title"),
                Some("Accept"),
                Some("Cancel"),
                filters,
                nfilters,
            );
            if left.is_some() {
                // (the dialog couldn't be set up: the caller would fall
                // back to the older dialogs)
                crate::test_support::skip("desktop", "IFileDialog could not be set up");
                continue;
            }
            let (result, filter) = rx.try_recv().unwrap();
            let e = result.unwrap_err();
            assert!(e.starts_with("dialog: "), "{e}");
            assert_eq!(filter, None);
        }
        done.store(true, std::sync::atomic::Ordering::Relaxed);
    }

    /// The method slots, as `offsetof()` in `shobjidl.h` gives them
    /// (checked with a mingw-w64 C harness).
    #[test]
    fn vtable_layouts() {
        use std::mem::offset_of;
        let slot = |offset: usize| offset / size_of::<usize>();
        assert_eq!(slot(offset_of!(IShellItemVtbl, get_display_name)), 5);
        assert_eq!(slot(offset_of!(IShellItemArrayVtbl, get_count)), 7);
        assert_eq!(slot(offset_of!(IShellItemArrayVtbl, get_item_at)), 8);
        assert_eq!(slot(offset_of!(IFileDialogVtbl, show)), 3);
        assert_eq!(slot(offset_of!(IFileDialogVtbl, set_file_types)), 4);
        assert_eq!(slot(offset_of!(IFileDialogVtbl, get_file_type_index)), 6);
        assert_eq!(slot(offset_of!(IFileDialogVtbl, set_options)), 9);
        assert_eq!(slot(offset_of!(IFileDialogVtbl, get_options)), 10);
        assert_eq!(slot(offset_of!(IFileDialogVtbl, set_folder)), 12);
        assert_eq!(slot(offset_of!(IFileDialogVtbl, set_file_name)), 15);
        assert_eq!(slot(offset_of!(IFileDialogVtbl, set_title)), 17);
        assert_eq!(slot(offset_of!(IFileDialogVtbl, set_ok_button_label)), 18);
        assert_eq!(slot(offset_of!(IFileDialogVtbl, get_result)), 20);
        assert_eq!(slot(size_of::<IFileDialogVtbl>()), 27);
        assert_eq!(slot(offset_of!(IFileOpenDialogVtbl, get_results)), 27);
        assert_eq!(slot(offset_of!(IFileDialog2Vtbl, set_options)), 9);
        assert_eq!(slot(offset_of!(IFileDialog2Vtbl, get_options)), 10);
        assert_eq!(
            slot(offset_of!(IFileDialog2Vtbl, set_cancel_button_label)),
            27
        );
        assert_eq!(size_of::<COMDLG_FILTERSPEC>(), 16);
    }
}
