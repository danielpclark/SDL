// Rust translation of src/hidapi/windows/hid.c, hidapi_cfgmgr32.h,
// hidapi_hidclass.h, hidapi_hidpi.h, hidapi_hidsdi.h, hidapi_winapi.h and
// src/hidapi/SDL_hidapi_windows.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.
//
// HIDAPI - Multi-Platform library for communication with HID devices.
// Alan Ott, Signal 11 Software; libusb/hidapi Team. Copyright 2022, All
// Rights Reserved. At the discretion of the user of this library, this
// software may be licensed under the terms of the GNU General Public
// License v3, a BSD-Style license, or the original HIDAPI license as
// outlined in the LICENSE.txt, LICENSE-gpl3.txt, LICENSE-bsd.txt, and
// LICENSE-orig.txt files located at the root of the source distribution.
// These files may also be found in the public source code repository
// located at: https://github.com/libusb/hidapi .

//! The Windows backend: devices are the interfaces of the HID device class
//! (listed with `cfgmgr32.dll`) and are used through the HID class driver
//! (`hid.dll`), both loaded at run time like upstream does without the DDK.
//! Reads and writes are overlapped I/O on the device handle.
//!
//! Errors are returned (upstream, built into SDL, sets them with
//! `SDL_SetError()`), so the error strings `hid_error()` keeps don't exist.

#![allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clippy::upper_case_acronyms
)]

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use windows_sys::core::GUID;
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_IO_PENDING, ERROR_OPERATION_ABORTED, GENERIC_READ,
    GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, ReadFile, WriteFile, FILE_FLAG_OVERLAPPED, FILE_SHARE_READ, FILE_SHARE_WRITE,
    OPEN_EXISTING,
};
use windows_sys::Win32::System::Diagnostics::Debug::{
    FormatMessageW, FORMAT_MESSAGE_FROM_SYSTEM, FORMAT_MESSAGE_IGNORE_INSERTS,
};
use windows_sys::Win32::System::Threading::{
    CreateEventW, ResetEvent, WaitForSingleObject, INFINITE,
};
use windows_sys::Win32::System::IO::{
    CancelIo, CancelIoEx, DeviceIoControl, GetOverlappedResult, OVERLAPPED,
};

use super::descriptor_reconstruct::{
    preparsed_data_size, reconstruct_pp_data, PP_DATA_CAPS_OFFSET,
};
use super::{BusType, DeviceInfo};
use crate::error::{Error, Result};
use crate::loadso::SharedObject;

/// MAXIMUM_USB_STRING_LENGTH from usbspec.h is 255;
/// BLUETOOTH_DEVICE_NAME_SIZE from bluetoothapis.h is 256 (`MAX_STRING_WCHARS`).
const MAX_STRING_WCHARS: usize = 256;

/// For certain USB devices, using a buffer larger or equal to 127 wchars
/// results in successful completion of HID API functions, but a broken
/// string is stored in the output buffer. This behaviour persists even if
/// HID API is bypassed and HID IOCTLs are passed to the HID driver
/// directly. Therefore, for USB devices, the buffer MUST NOT exceed 126
/// WCHARs (`MAX_STRING_WCHARS_USB`).
const MAX_STRING_WCHARS_USB: usize = 126;

// hidapi_cfgmgr32.h

/// `PROPERTYKEY`/`DEVPROPKEY`
#[repr(C)]
#[derive(Clone, Copy)]
struct DEVPROPKEY {
    fmtid: GUID,
    pid: u32,
}

type CONFIGRET = u32;
type DEVINST = u32;
type DEVPROPTYPE = u32;

const CR_SUCCESS: CONFIGRET = 0x00000000;
const CR_BUFFER_SMALL: CONFIGRET = 0x0000001A;
const CR_FAILURE: CONFIGRET = 0x00000013;

const CM_LOCATE_DEVNODE_NORMAL: u32 = 0x00000000;
const CM_GET_DEVICE_INTERFACE_LIST_PRESENT: u32 = 0x00000000;

/// `DEVPROP_TYPE_GUID`
const DEVPROP_TYPE_GUID: DEVPROPTYPE = 0x0000000D;
/// `DEVPROP_TYPE_STRING`
const DEVPROP_TYPE_STRING: DEVPROPTYPE = 0x00000012;
/// `DEVPROP_TYPE_STRING_LIST` (`DEVPROP_TYPE_STRING | DEVPROP_TYPEMOD_LIST`)
const DEVPROP_TYPE_STRING_LIST: DEVPROPTYPE = 0x00002012;

const fn devpropkey(data1: u32, data2: u16, data3: u16, data4: [u8; 8], pid: u32) -> DEVPROPKEY {
    DEVPROPKEY {
        fmtid: GUID {
            data1,
            data2,
            data3,
            data4,
        },
        pid,
    }
}

// from devpkey.h
const DEVPKEY_NAME: DEVPROPKEY = devpropkey(
    0xb725f130,
    0x47ef,
    0x101a,
    [0xa5, 0xf1, 0x02, 0x60, 0x8c, 0x9e, 0xeb, 0xac],
    10,
); // DEVPROP_TYPE_STRING
const DEVPKEY_Device_Manufacturer: DEVPROPKEY = devpropkey(
    0xa45c254e,
    0xdf1c,
    0x4efd,
    [0x80, 0x20, 0x67, 0xd1, 0x46, 0xa8, 0x50, 0xe0],
    13,
); // DEVPROP_TYPE_STRING
const DEVPKEY_Device_InstanceId: DEVPROPKEY = devpropkey(
    0x78c34fc8,
    0x104a,
    0x4aca,
    [0x9e, 0xa4, 0x52, 0x4d, 0x52, 0x99, 0x6e, 0x57],
    256,
); // DEVPROP_TYPE_STRING
const DEVPKEY_Device_HardwareIds: DEVPROPKEY = devpropkey(
    0xa45c254e,
    0xdf1c,
    0x4efd,
    [0x80, 0x20, 0x67, 0xd1, 0x46, 0xa8, 0x50, 0xe0],
    3,
); // DEVPROP_TYPE_STRING_LIST
const DEVPKEY_Device_CompatibleIds: DEVPROPKEY = devpropkey(
    0xa45c254e,
    0xdf1c,
    0x4efd,
    [0x80, 0x20, 0x67, 0xd1, 0x46, 0xa8, 0x50, 0xe0],
    4,
); // DEVPROP_TYPE_STRING_LIST
const DEVPKEY_Device_ContainerId: DEVPROPKEY = devpropkey(
    0x8c7ed206,
    0x3f8a,
    0x4827,
    [0xb3, 0xab, 0xae, 0x9e, 0x1f, 0xae, 0xfc, 0x6c],
    2,
); // DEVPROP_TYPE_GUID

// from propkey.h
const PKEY_DeviceInterface_Bluetooth_DeviceAddress: DEVPROPKEY = devpropkey(
    0x2bd67d8b,
    0x8beb,
    0x48d5,
    [0x87, 0xe0, 0x6c, 0xda, 0x34, 0x28, 0x04, 0x0a],
    1,
); // DEVPROP_TYPE_STRING
const PKEY_DeviceInterface_Bluetooth_Manufacturer: DEVPROPKEY = devpropkey(
    0x2bd67d8b,
    0x8beb,
    0x48d5,
    [0x87, 0xe0, 0x6c, 0xda, 0x34, 0x28, 0x04, 0x0a],
    4,
); // DEVPROP_TYPE_STRING
const PKEY_DeviceInterface_Bluetooth_ModelNumber: DEVPROPKEY = devpropkey(
    0x2BD67D8B,
    0x8BEB,
    0x48D5,
    [0x87, 0xE0, 0x6C, 0xDA, 0x34, 0x28, 0x04, 0x0A],
    5,
); // DEVPROP_TYPE_STRING

type CM_Locate_DevNodeW_ = unsafe extern "system" fn(
    pdnDevInst: *mut DEVINST,
    pDeviceID: *const u16,
    ulFlags: u32,
) -> CONFIGRET;
type CM_Get_Parent_ = unsafe extern "system" fn(
    pdnDevInst: *mut DEVINST,
    dnDevInst: DEVINST,
    ulFlags: u32,
) -> CONFIGRET;
type CM_Get_DevNode_PropertyW_ = unsafe extern "system" fn(
    dnDevInst: DEVINST,
    PropertyKey: *const DEVPROPKEY,
    PropertyType: *mut DEVPROPTYPE,
    PropertyBuffer: *mut u8,
    PropertyBufferSize: *mut u32,
    ulFlags: u32,
) -> CONFIGRET;
type CM_Get_Device_Interface_PropertyW_ = unsafe extern "system" fn(
    pszDeviceInterface: *const u16,
    PropertyKey: *const DEVPROPKEY,
    PropertyType: *mut DEVPROPTYPE,
    PropertyBuffer: *mut u8,
    PropertyBufferSize: *mut u32,
    ulFlags: u32,
) -> CONFIGRET;
type CM_Get_Device_Interface_List_SizeW_ = unsafe extern "system" fn(
    pulLen: *mut u32,
    InterfaceClassGuid: *const GUID,
    pDeviceID: *const u16,
    ulFlags: u32,
) -> CONFIGRET;
type CM_Get_Device_Interface_ListW_ = unsafe extern "system" fn(
    InterfaceClassGuid: *const GUID,
    pDeviceID: *const u16,
    Buffer: *mut u16,
    BufferLen: u32,
    ulFlags: u32,
) -> CONFIGRET;

// hidapi_hidsdi.h and hidapi_hidpi.h

/// `HIDD_ATTRIBUTES`
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct HIDD_ATTRIBUTES {
    Size: u32,
    VendorID: u16,
    ProductID: u16,
    VersionNumber: u16,
}

/// `HIDP_CAPS`
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct HIDP_CAPS {
    Usage: u16,
    UsagePage: u16,
    InputReportByteLength: u16,
    OutputReportByteLength: u16,
    FeatureReportByteLength: u16,
    Reserved: [u16; 17],
    NumberLinkCollectionNodes: u16,
    NumberInputButtonCaps: u16,
    NumberInputValueCaps: u16,
    NumberInputDataIndices: u16,
    NumberOutputButtonCaps: u16,
    NumberOutputValueCaps: u16,
    NumberOutputDataIndices: u16,
    NumberFeatureButtonCaps: u16,
    NumberFeatureValueCaps: u16,
    NumberFeatureDataIndices: u16,
}

/// `PHIDP_PREPARSED_DATA` (opaque, see `descriptor_reconstruct`)
type PHIDP_PREPARSED_DATA = *mut c_void;

/// `BOOLEAN`
type BOOLEAN = u8;

/// `HIDP_STATUS_SUCCESS`
const HIDP_STATUS_SUCCESS: i32 = 0x00110000;

type HidD_GetHidGuid_ = unsafe extern "system" fn(hid_guid: *mut GUID);
type HidD_GetAttributes_ =
    unsafe extern "system" fn(device: HANDLE, attrib: *mut HIDD_ATTRIBUTES) -> BOOLEAN;
type HidD_GetString_ =
    unsafe extern "system" fn(device: HANDLE, buffer: *mut c_void, buffer_len: u32) -> BOOLEAN;
type HidD_SetFeature_ =
    unsafe extern "system" fn(handle: HANDLE, data: *mut c_void, length: u32) -> BOOLEAN;
type HidD_GetIndexedString_ = unsafe extern "system" fn(
    handle: HANDLE,
    string_index: u32,
    buffer: *mut c_void,
    buffer_len: u32,
) -> BOOLEAN;
type HidD_GetPreparsedData_ =
    unsafe extern "system" fn(handle: HANDLE, preparsed_data: *mut PHIDP_PREPARSED_DATA) -> BOOLEAN;
type HidD_FreePreparsedData_ =
    unsafe extern "system" fn(preparsed_data: PHIDP_PREPARSED_DATA) -> BOOLEAN;
type HidP_GetCaps_ =
    unsafe extern "system" fn(preparsed_data: PHIDP_PREPARSED_DATA, caps: *mut HIDP_CAPS) -> i32;
type HidD_SetNumInputBuffers_ =
    unsafe extern "system" fn(handle: HANDLE, number_buffers: u32) -> BOOLEAN;
type HidD_SetOutputReport_ = unsafe extern "system" fn(
    HidDeviceObject: HANDLE,
    ReportBuffer: *mut c_void,
    ReportBufferLength: u32,
) -> BOOLEAN;

// hidapi_hidclass.h

/// `FILE_DEVICE_KEYBOARD`
const FILE_DEVICE_KEYBOARD: u32 = 0x0000000b;
/// `METHOD_OUT_DIRECT`
const METHOD_OUT_DIRECT: u32 = 2;
/// `FILE_ANY_ACCESS`
const FILE_ANY_ACCESS: u32 = 0;

/// `HID_OUT_CTL_CODE(id)`: `CTL_CODE(FILE_DEVICE_KEYBOARD, (id), METHOD_OUT_DIRECT, FILE_ANY_ACCESS)`
const fn hid_out_ctl_code(id: u32) -> u32 {
    (FILE_DEVICE_KEYBOARD << 16) | (FILE_ANY_ACCESS << 14) | (id << 2) | METHOD_OUT_DIRECT
}

/// `IOCTL_HID_GET_FEATURE`
const IOCTL_HID_GET_FEATURE: u32 = hid_out_ctl_code(100);
/// `IOCTL_HID_GET_INPUT_REPORT`
const IOCTL_HID_GET_INPUT_REPORT: u32 = hid_out_ctl_code(104);

// The library functions

/// The functions `lookup_functions()` resolves, with the libraries that
/// hold them.
struct Functions {
    _hid_lib_handle: SharedObject,
    _cfgmgr32_lib_handle: SharedObject,
    HidD_GetHidGuid: HidD_GetHidGuid_,
    HidD_GetAttributes: HidD_GetAttributes_,
    HidD_GetSerialNumberString: HidD_GetString_,
    HidD_GetManufacturerString: HidD_GetString_,
    HidD_GetProductString: HidD_GetString_,
    HidD_SetFeature: HidD_SetFeature_,
    HidD_GetIndexedString: HidD_GetIndexedString_,
    HidD_GetPreparsedData: HidD_GetPreparsedData_,
    HidD_FreePreparsedData: HidD_FreePreparsedData_,
    HidP_GetCaps: HidP_GetCaps_,
    HidD_SetNumInputBuffers: HidD_SetNumInputBuffers_,
    HidD_SetOutputReport: HidD_SetOutputReport_,
    CM_Locate_DevNodeW: CM_Locate_DevNodeW_,
    CM_Get_Parent: CM_Get_Parent_,
    CM_Get_DevNode_PropertyW: CM_Get_DevNode_PropertyW_,
    CM_Get_Device_Interface_PropertyW: CM_Get_Device_Interface_PropertyW_,
    CM_Get_Device_Interface_List_SizeW: CM_Get_Device_Interface_List_SizeW_,
    CM_Get_Device_Interface_ListW: CM_Get_Device_Interface_ListW_,
}

/// Translation of the function pointers, `hid_lib_handle`,
/// `cfgmgr32_lib_handle` and `hidapi_initialized` (open devices keep the
/// libraries loaded).
static FUNCTIONS: Mutex<Option<Arc<Functions>>> = Mutex::new(None);

/// Translation of `lookup_functions()`.
fn lookup_functions() -> Result<Functions> {
    let hid_lib_handle = SharedObject::load("hid.dll")?;
    let cfgmgr32_lib_handle = SharedObject::load("cfgmgr32.dll")?;

    macro_rules! resolve {
        ($lib:expr, $name:literal) => {
            // SAFETY: the symbol has the prototype of the field it fills
            // (declared above from the Windows headers); the libraries stay
            // loaded with the functions.
            unsafe { $lib.function($name)? }
        };
    }

    Ok(Functions {
        HidD_GetHidGuid: resolve!(hid_lib_handle, "HidD_GetHidGuid"),
        HidD_GetAttributes: resolve!(hid_lib_handle, "HidD_GetAttributes"),
        HidD_GetSerialNumberString: resolve!(hid_lib_handle, "HidD_GetSerialNumberString"),
        HidD_GetManufacturerString: resolve!(hid_lib_handle, "HidD_GetManufacturerString"),
        HidD_GetProductString: resolve!(hid_lib_handle, "HidD_GetProductString"),
        HidD_SetFeature: resolve!(hid_lib_handle, "HidD_SetFeature"),
        // (HidD_GetFeature and HidD_GetInputReport are resolved upstream
        // but not used)
        HidD_GetIndexedString: resolve!(hid_lib_handle, "HidD_GetIndexedString"),
        HidD_GetPreparsedData: resolve!(hid_lib_handle, "HidD_GetPreparsedData"),
        HidD_FreePreparsedData: resolve!(hid_lib_handle, "HidD_FreePreparsedData"),
        HidP_GetCaps: resolve!(hid_lib_handle, "HidP_GetCaps"),
        HidD_SetNumInputBuffers: resolve!(hid_lib_handle, "HidD_SetNumInputBuffers"),
        HidD_SetOutputReport: resolve!(hid_lib_handle, "HidD_SetOutputReport"),
        CM_Locate_DevNodeW: resolve!(cfgmgr32_lib_handle, "CM_Locate_DevNodeW"),
        CM_Get_Parent: resolve!(cfgmgr32_lib_handle, "CM_Get_Parent"),
        CM_Get_DevNode_PropertyW: resolve!(cfgmgr32_lib_handle, "CM_Get_DevNode_PropertyW"),
        CM_Get_Device_Interface_PropertyW: resolve!(
            cfgmgr32_lib_handle,
            "CM_Get_Device_Interface_PropertyW"
        ),
        CM_Get_Device_Interface_List_SizeW: resolve!(
            cfgmgr32_lib_handle,
            "CM_Get_Device_Interface_List_SizeW"
        ),
        CM_Get_Device_Interface_ListW: resolve!(
            cfgmgr32_lib_handle,
            "CM_Get_Device_Interface_ListW"
        ),
        _hid_lib_handle: hid_lib_handle,
        _cfgmgr32_lib_handle: cfgmgr32_lib_handle,
    })
}

fn functions() -> Result<Arc<Functions>> {
    FUNCTIONS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .ok_or_else(|| Error::new("hidapi isn't initialized"))
}

// Errors

/// Translation of `register_winapi_error_to_buffer()`: "op: (0xCODE)
/// system message" for `GetLastError()`.
fn winapi_error(op: &str) -> Error {
    // SAFETY: GetLastError has no preconditions.
    let error_code = unsafe { GetLastError() };
    winapi_error_code(op, error_code)
}

fn winapi_error_code(op: &str, error_code: u32) -> Error {
    let mut system_err_buf = [0u16; 1024];
    // SAFETY: the buffer holds the given number of UTF-16 units; no
    // source or arguments are used.
    let system_err_len = unsafe {
        FormatMessageW(
            FORMAT_MESSAGE_FROM_SYSTEM | FORMAT_MESSAGE_IGNORE_INSERTS,
            std::ptr::null(),
            error_code,
            0x0400, // MAKELANGID(LANG_NEUTRAL, SUBLANG_DEFAULT)
            system_err_buf.as_mut_ptr(),
            system_err_buf.len() as u32,
            std::ptr::null(),
        )
    } as usize;
    let system_err =
        String::from_utf16_lossy(&system_err_buf[..system_err_len.min(system_err_buf.len())]);

    let msg = format!("{op}: (0x{error_code:08X}) {system_err}");

    /* Get rid of the CR and LF that FormatMessage() sticks at the
    end of the message. Thanks Microsoft! */
    Error::new(msg.trim_end_matches(['\r', '\n', ' ']).to_owned())
}

/// Translation of `open_device()`.
fn open_device(path: &[u16], open_rw: bool) -> HANDLE {
    let desired_access = if open_rw {
        GENERIC_WRITE | GENERIC_READ
    } else {
        0
    };
    let share_mode = FILE_SHARE_READ | FILE_SHARE_WRITE;

    // SAFETY: path is NUL-terminated.
    unsafe {
        CreateFileW(
            path.as_ptr(),
            desired_access,
            share_mode,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_OVERLAPPED, /*FILE_ATTRIBUTE_NORMAL,*/
            std::ptr::null_mut(),
        )
    }
}

/// Translation of `hid_init()`.
pub(super) fn hid_init() -> Result<()> {
    let mut functions = FUNCTIONS.lock().unwrap_or_else(|e| e.into_inner());
    if functions.is_none() {
        match lookup_functions() {
            Ok(f) => *functions = Some(Arc::new(f)),
            Err(e) => {
                return Err(Error::new(format!(
                    "resolve DLL functions: {}",
                    e.message()
                )));
            }
        }
    }
    Ok(())
}

/// Translation of `hid_exit()`.
pub(super) fn hid_exit() -> Result<()> {
    *FUNCTIONS.lock().unwrap_or_else(|e| e.into_inner()) = None;
    Ok(())
}

/// A NUL-terminated wide string up to its NUL (the whole slice if none).
fn wide_str(w: &[u16]) -> &[u16] {
    &w[..w.iter().position(|&c| c == 0).unwrap_or(w.len())]
}

/// The strings of a `REG_MULTI_SZ`-style list (double NUL terminated).
fn wide_list(w: &[u16]) -> impl Iterator<Item = &[u16]> {
    let mut rest = w;
    std::iter::from_fn(move || {
        let s = wide_str(rest);
        if s.is_empty() {
            return None;
        }
        rest = rest.get(s.len() + 1..).unwrap_or(&[]);
        Some(s)
    })
}

/// Translation of `hid_internal_towupper()` (`SDL_toupper`, ASCII only).
fn towupper(s: &[u16]) -> Vec<u16> {
    s.iter()
        .map(|&c| {
            if (u16::from(b'a')..=u16::from(b'z')).contains(&c) {
                c - 32
            } else {
                c
            }
        })
        .collect()
}

/// `wcsstr()`: the index of `needle` in `haystack`.
fn wcsstr(haystack: &[u16], needle: &str) -> Option<usize> {
    let needle: Vec<u16> = needle.encode_utf16().collect();
    haystack
        .windows(needle.len())
        .position(|w| w == needle.as_slice())
}

/// Translation of `hid_internal_extract_int_token_value()`: the hex number
/// after `token` in `string`, or -1.
fn hid_internal_extract_int_token_value(string: &[u16], token: &str) -> i32 {
    let Some(start) = wcsstr(string, token) else {
        return -1;
    };

    let rest = &string[start + token.encode_utf16().count()..];
    // (wcstol on the wide string; only ASCII matters for hex digits)
    let narrow: String = rest
        .iter()
        .map(|&c| {
            char::from_u32(u32::from(c))
                .filter(char::is_ascii)
                .unwrap_or('\u{1}')
        })
        .collect();
    let (token_value, len) = crate::stdlib::string::strtol(&narrow, 16);
    if len == 0 {
        return -1;
    }

    token_value as i32
}

impl Functions {
    /// Translation of `hid_internal_get_devnode_property()`.
    fn get_devnode_property(
        &self,
        dev_node: DEVINST,
        property_key: &DEVPROPKEY,
        expected_property_type: DEVPROPTYPE,
    ) -> Option<Vec<u8>> {
        let mut len: u32 = 0;
        let mut property_type: DEVPROPTYPE = 0;

        // SAFETY: a size query with valid out-parameters.
        let cr = unsafe {
            (self.CM_Get_DevNode_PropertyW)(
                dev_node,
                property_key,
                &mut property_type,
                std::ptr::null_mut(),
                &mut len,
                0,
            )
        };
        if cr != CR_BUFFER_SMALL || property_type != expected_property_type {
            return None;
        }

        let mut property_value = vec![0u8; len as usize];
        // SAFETY: the buffer holds len bytes.
        let cr = unsafe {
            (self.CM_Get_DevNode_PropertyW)(
                dev_node,
                property_key,
                &mut property_type,
                property_value.as_mut_ptr(),
                &mut len,
                0,
            )
        };
        if cr != CR_SUCCESS {
            return None;
        }

        Some(property_value)
    }

    /// Translation of `hid_internal_get_device_interface_property()`.
    fn get_device_interface_property(
        &self,
        interface_path: &[u16],
        property_key: &DEVPROPKEY,
        expected_property_type: DEVPROPTYPE,
    ) -> Option<Vec<u8>> {
        let mut len: u32 = 0;
        let mut property_type: DEVPROPTYPE = 0;

        // SAFETY: a size query with valid out-parameters; the path is
        // NUL-terminated.
        let cr = unsafe {
            (self.CM_Get_Device_Interface_PropertyW)(
                interface_path.as_ptr(),
                property_key,
                &mut property_type,
                std::ptr::null_mut(),
                &mut len,
                0,
            )
        };
        if cr != CR_BUFFER_SMALL || property_type != expected_property_type {
            return None;
        }

        let mut property_value = vec![0u8; len as usize];
        // SAFETY: the buffer holds len bytes.
        let cr = unsafe {
            (self.CM_Get_Device_Interface_PropertyW)(
                interface_path.as_ptr(),
                property_key,
                &mut property_type,
                property_value.as_mut_ptr(),
                &mut len,
                0,
            )
        };
        if cr != CR_SUCCESS {
            return None;
        }

        Some(property_value)
    }

    /// A string property (UTF-16 units, NUL-terminated).
    fn devnode_string(
        &self,
        dev_node: DEVINST,
        key: &DEVPROPKEY,
        kind: DEVPROPTYPE,
    ) -> Option<Vec<u16>> {
        self.get_devnode_property(dev_node, key, kind)
            .map(|bytes| bytes_to_wide(&bytes))
    }

    /// `CM_Get_Parent()`
    fn parent(&self, dev_node: DEVINST) -> Option<DEVINST> {
        let mut parent: DEVINST = 0;
        // SAFETY: a valid out-parameter.
        (unsafe { (self.CM_Get_Parent)(&mut parent, dev_node, 0) } == CR_SUCCESS).then_some(parent)
    }

    /// Translation of `hid_internal_get_usb_info()`.
    fn get_usb_info(&self, dev: &mut DeviceInfo, mut dev_node: DEVINST) {
        let Some(device_id) =
            self.devnode_string(dev_node, &DEVPKEY_Device_InstanceId, DEVPROP_TYPE_STRING)
        else {
            return;
        };

        // Normalize to upper case
        let device_id = towupper(wide_str(&device_id));

        /* Check for Xbox Common Controller class (XUSB) device.
        https://docs.microsoft.com/windows/win32/xinput/directinput-and-xusb-devices
        https://docs.microsoft.com/windows/win32/xinput/xinput-and-directinput
        */
        if hid_internal_extract_int_token_value(&device_id, "IG_") != -1 {
            // Get devnode parent to reach out USB device.
            match self.parent(dev_node) {
                Some(parent) => dev_node = parent,
                None => return,
            }
        }

        // Get the hardware ids from devnode
        let Some(hardware_ids) = self.devnode_string(
            dev_node,
            &DEVPKEY_Device_HardwareIds,
            DEVPROP_TYPE_STRING_LIST,
        ) else {
            return;
        };

        /* Get additional information from USB device's Hardware ID
        https://docs.microsoft.com/windows-hardware/drivers/install/standard-usb-identifiers
        https://docs.microsoft.com/windows-hardware/drivers/usbcon/enumeration-of-interfaces-not-grouped-in-collections
        */
        for hardware_id in wide_list(&hardware_ids) {
            // Normalize to upper case
            let hardware_id = towupper(hardware_id);

            if dev.release_number == 0 {
                // USB_DEVICE_DESCRIPTOR.bcdDevice value.
                let release_number = hid_internal_extract_int_token_value(&hardware_id, "REV_");
                if release_number != -1 {
                    dev.release_number = release_number as u16;
                }
            }

            if dev.interface_number == -1 {
                // USB_INTERFACE_DESCRIPTOR.bInterfaceNumber value.
                let interface_number = hid_internal_extract_int_token_value(&hardware_id, "MI_");
                if interface_number != -1 {
                    dev.interface_number = interface_number;
                }
            }
        }

        // Try to get USB device manufacturer string if not provided by HidD_GetManufacturerString.
        if dev.manufacturer_string.as_deref().unwrap_or("").is_empty() {
            if let Some(manufacturer_string) =
                self.devnode_string(dev_node, &DEVPKEY_Device_Manufacturer, DEVPROP_TYPE_STRING)
            {
                dev.manufacturer_string = Some(wide_to_string(&manufacturer_string));
            }
        }

        // Try to get USB device serial number if not provided by HidD_GetSerialNumberString.
        if dev.serial_number.as_deref().unwrap_or("").is_empty() {
            let mut usb_dev_node = dev_node;
            if dev.interface_number != -1 {
                /* Get devnode parent to reach out composite parent USB device.
                https://docs.microsoft.com/windows-hardware/drivers/usbcon/enumeration-of-the-composite-parent-device
                */
                match self.parent(dev_node) {
                    Some(parent) => usb_dev_node = parent,
                    None => return,
                }
            }

            // Get the device id of the USB device.
            let Some(device_id) = self.devnode_string(
                usb_dev_node,
                &DEVPKEY_Device_InstanceId,
                DEVPROP_TYPE_STRING,
            ) else {
                return;
            };
            let device_id = wide_str(&device_id);

            /* Extract substring after last '\\' of Instance ID.
            For USB devices it may contain device's serial number.
            https://docs.microsoft.com/windows-hardware/drivers/install/instance-ids
            */
            // (upstream starts at the terminating NUL and stops before the
            // first character)
            for ptr in (1..=device_id.len()).rev() {
                let c = device_id.get(ptr).copied().unwrap_or(0);
                /* Instance ID is unique only within the scope of the bus.
                For USB devices it means that serial number is not available. Skip. */
                if c == u16::from(b'&') {
                    break;
                }

                if c == u16::from(b'\\') {
                    dev.serial_number = Some(wide_to_string(&device_id[ptr + 1..]));
                    break;
                }
            }
        }

        // If we can't get the interface number, it means that there is only one interface.
        if dev.interface_number == -1 {
            dev.interface_number = 0;
        }
    }

    /// Translation of `hid_internal_get_ble_info()`:
    /// HidD_GetProductString/HidD_GetManufacturerString/HidD_GetSerialNumberString
    /// is not working for BLE HID devices. Request this info via dev node
    /// properties instead.
    /// <https://docs.microsoft.com/answers/questions/401236/hidd-getproductstring-with-ble-hid-device.html>
    fn get_ble_info(&self, dev: &mut DeviceInfo, dev_node: DEVINST) {
        if dev.manufacturer_string.as_deref().unwrap_or("").is_empty() {
            // Manufacturer Name String (UUID: 0x2A29)
            if let Some(manufacturer_string) = self.devnode_string(
                dev_node,
                &PKEY_DeviceInterface_Bluetooth_Manufacturer,
                DEVPROP_TYPE_STRING,
            ) {
                dev.manufacturer_string = Some(wide_to_string(&manufacturer_string));
            }
        }

        if dev.serial_number.as_deref().unwrap_or("").is_empty() {
            // Serial Number String (UUID: 0x2A25)
            if let Some(serial_number) = self.devnode_string(
                dev_node,
                &PKEY_DeviceInterface_Bluetooth_DeviceAddress,
                DEVPROP_TYPE_STRING,
            ) {
                dev.serial_number = Some(wide_to_string(&serial_number));
            }
        }

        if dev.product_string.as_deref().unwrap_or("").is_empty() {
            // Model Number String (UUID: 0x2A24)
            let mut product_string = self.devnode_string(
                dev_node,
                &PKEY_DeviceInterface_Bluetooth_ModelNumber,
                DEVPROP_TYPE_STRING,
            );
            if product_string.is_none() {
                // Fallback: Get devnode grandparent to reach out Bluetooth LE device node
                if let Some(parent_dev_node) = self.parent(dev_node) {
                    // Device Name (UUID: 0x2A00)
                    product_string =
                        self.devnode_string(parent_dev_node, &DEVPKEY_NAME, DEVPROP_TYPE_STRING);
                }
            }

            if let Some(product_string) = product_string {
                dev.product_string = Some(wide_to_string(&product_string));
            }
        }
    }

    /// Translation of `hid_internal_detect_bus_type()` (and of
    /// `get_bus_type()`, which is the same without the flags).
    fn detect_bus_type(&self, interface_path: &[u16]) -> DetectBusTypeResult {
        let mut result = DetectBusTypeResult::default();

        // Get the device id from interface path
        let Some(device_id) = self
            .get_device_interface_property(
                interface_path,
                &DEVPKEY_Device_InstanceId,
                DEVPROP_TYPE_STRING,
            )
            .map(|bytes| bytes_to_wide(&bytes))
        else {
            return result;
        };

        // Open devnode from device id
        let mut dev_node: DEVINST = 0;
        // SAFETY: the device id is NUL-terminated (with the terminator
        // bytes_to_wide() keeps).
        let cr = unsafe {
            (self.CM_Locate_DevNodeW)(&mut dev_node, device_id.as_ptr(), CM_LOCATE_DEVNODE_NORMAL)
        };
        if cr != CR_SUCCESS {
            return result;
        }

        // Get devnode parent
        let Some(dev_node) = self.parent(dev_node) else {
            return result;
        };

        // Get the compatible ids from parent devnode
        let Some(compatible_ids) = self.devnode_string(
            dev_node,
            &DEVPKEY_Device_CompatibleIds,
            DEVPROP_TYPE_STRING_LIST,
        ) else {
            return result;
        };

        // Now we can parse parent's compatible IDs to find out the device bus type
        for compatible_id in wide_list(&compatible_ids) {
            // Normalize to upper case
            let compatible_id = towupper(compatible_id);

            /* USB devices
            https://docs.microsoft.com/windows-hardware/drivers/hid/plug-and-play-support
            https://docs.microsoft.com/windows-hardware/drivers/install/standard-usb-identifiers */
            if wcsstr(&compatible_id, "USB").is_some() {
                result.bus_type = BusType::Usb;
                break;
            }

            /* Bluetooth devices
            https://docs.microsoft.com/windows-hardware/drivers/bluetooth/installing-a-bluetooth-device */
            if wcsstr(&compatible_id, "BTHENUM").is_some() {
                result.bus_type = BusType::Bluetooth;
                break;
            }

            // Bluetooth LE devices
            if wcsstr(&compatible_id, "BTHLEDEVICE").is_some() {
                result.bus_type = BusType::Bluetooth;
                result.ble = true;
                break;
            }

            /* I2C devices
            https://docs.microsoft.com/windows-hardware/drivers/hid/plug-and-play-support-and-power-management */
            if wcsstr(&compatible_id, "PNP0C50").is_some() {
                result.bus_type = BusType::I2c;
                break;
            }

            /* SPI devices
            https://docs.microsoft.com/windows-hardware/drivers/hid/plug-and-play-for-spi */
            if wcsstr(&compatible_id, "PNP0C51").is_some() {
                result.bus_type = BusType::Spi;
                break;
            }
        }

        result.dev_node = dev_node;
        result
    }

    /// Read one of the `HidD_Get*String()` strings.
    fn hid_string(&self, f: HidD_GetString_, handle: HANDLE, len: usize) -> String {
        let mut string = [0u16; MAX_STRING_WCHARS + 1];
        let size = (len * size_of::<u16>()) as u32;
        // SAFETY: the buffer holds more than size bytes.
        unsafe {
            f(handle, string.as_mut_ptr().cast(), size);
        }
        wide_to_string(&string[..len])
    }

    /// Translation of `hid_internal_get_device_info()`.
    fn get_device_info(&self, path: &[u16], handle: HANDLE) -> DeviceInfo {
        // Fill out the record
        let mut dev = DeviceInfo {
            path: hid_internal_utf16_to_utf8(path),
            interface_number: -1,
            ..DeviceInfo::default()
        };

        let mut attrib = HIDD_ATTRIBUTES {
            Size: size_of::<HIDD_ATTRIBUTES>() as u32,
            ..HIDD_ATTRIBUTES::default()
        };
        // SAFETY: the handle is open and attrib is writable.
        if unsafe { (self.HidD_GetAttributes)(handle, &mut attrib) } != 0 {
            // VID/PID
            dev.vendor_id = attrib.VendorID;
            dev.product_id = attrib.ProductID;

            // Release Number
            dev.release_number = attrib.VersionNumber;
        }

        // Get the Usage Page and Usage for this device.
        if let Some(caps) = self.caps(handle) {
            dev.usage_page = caps.UsagePage;
            dev.usage = caps.Usage;
        }

        // detect bus type before reading string descriptors
        let detect_bus_type_result = self.detect_bus_type(path);
        dev.bus_type = detect_bus_type_result.bus_type;

        let len = if dev.bus_type == BusType::Usb {
            MAX_STRING_WCHARS_USB
        } else {
            MAX_STRING_WCHARS
        };

        // Serial Number
        dev.serial_number = Some(self.hid_string(self.HidD_GetSerialNumberString, handle, len));

        // Manufacturer String
        dev.manufacturer_string =
            Some(self.hid_string(self.HidD_GetManufacturerString, handle, len));

        // Product String
        dev.product_string = Some(self.hid_string(self.HidD_GetProductString, handle, len));

        // now, the portion that depends on string descriptors
        match dev.bus_type {
            BusType::Usb => self.get_usb_info(&mut dev, detect_bus_type_result.dev_node),
            BusType::Bluetooth => {
                if detect_bus_type_result.ble {
                    self.get_ble_info(&mut dev, detect_bus_type_result.dev_node);
                }
            }
            BusType::Unknown | BusType::Spi | BusType::I2c => {}
        }

        dev
    }

    /// The `HIDP_CAPS` of a device (`HidD_GetPreparsedData()`,
    /// `HidP_GetCaps()`, `HidD_FreePreparsedData()`), if they are available.
    fn caps(&self, handle: HANDLE) -> Option<HIDP_CAPS> {
        let mut pp_data: PHIDP_PREPARSED_DATA = std::ptr::null_mut();
        // SAFETY: the handle is open and pp_data is writable.
        if unsafe { (self.HidD_GetPreparsedData)(handle, &mut pp_data) } == 0 {
            return None;
        }
        let mut caps = HIDP_CAPS::default();
        // SAFETY: pp_data came from HidD_GetPreparsedData and is freed once.
        let status = unsafe { (self.HidP_GetCaps)(pp_data, &mut caps) };
        // SAFETY: as above.
        unsafe {
            (self.HidD_FreePreparsedData)(pp_data);
        }
        (status == HIDP_STATUS_SUCCESS).then_some(caps)
    }
}

/// Translation of `hid_internal_detect_bus_type_result` (the BLE flag is
/// `HID_API_BUS_FLAG_BLE`): unfortunately, `HID_API_BUS_xxx` constants
/// alone aren't enough to distinguish between BLUETOOTH and BLE.
#[derive(Clone, Copy, Default)]
struct DetectBusTypeResult {
    dev_node: DEVINST,
    bus_type: BusType,
    ble: bool,
}

/// Property bytes as UTF-16 units (keeping a NUL terminator).
fn bytes_to_wide(bytes: &[u8]) -> Vec<u16> {
    let mut wide: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    wide.push(0);
    wide
}

/// A wide string up to its NUL, as UTF-8 (unpaired surrogates replaced).
fn wide_to_string(w: &[u16]) -> String {
    String::from_utf16_lossy(wide_str(w))
}

/// Translation of `hid_internal_UTF16toUTF8()`: `None` for invalid UTF-16
/// (`WC_ERR_INVALID_CHARS`).
fn hid_internal_utf16_to_utf8(src: &[u16]) -> Option<String> {
    String::from_utf16(wide_str(src)).ok()
}

/// Translation of `hid_blacklist()`.
fn hid_blacklist(vendor_id: u16, product_id: u16) -> bool {
    const KNOWN_BAD: [(u16, u16); 8] = [
        (0x045E, 0x0822), // Microsoft Precision Mouse - causes deadlock asking for device details
        (0x0738, 0x2217), // SPEEDLINK COMPETITION PRO - turns into an Android controller when enumerated
        (0x0D8C, 0x0014), // Sharkoon Skiller SGH2 headset - causes deadlock asking for device details
        (0x1532, 0x0109), // Razer Lycosa Gaming keyboard - causes deadlock asking for device details
        (0x1532, 0x010B), // Razer Arctosa Gaming keyboard - causes deadlock asking for device details
        (0x1532, 0x0227), // Razer Huntsman Gaming keyboard - long delay asking for device details
        (0x1B1C, 0x1B3D), // Corsair Gaming keyboard - causes deadlock asking for device details
        (0x1CCF, 0x0000), // All Konami Amusement Devices - causes deadlock asking for device details
    ];

    KNOWN_BAD
        .iter()
        .any(|&(vid, pid)| vendor_id == vid && (product_id == pid || pid == 0x0000))
}

/// A device handle closed on drop.
struct OwnedHandle(HANDLE);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if self.0 != INVALID_HANDLE_VALUE {
            // SAFETY: the handle is ours and closed once.
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}

/// Translation of `hid_enumerate()`.
pub(super) fn hid_enumerate(vendor_id: u16, product_id: u16) -> Vec<DeviceInfo> {
    let Ok(f) = functions() else {
        return Vec::new();
    };

    /* Retrieve HID Interface Class GUID
    https://docs.microsoft.com/windows-hardware/drivers/install/guid-devinterface-hid */
    let mut interface_class_guid = GUID {
        data1: 0,
        data2: 0,
        data3: 0,
        data4: [0; 8],
    };
    // SAFETY: a valid out-parameter.
    unsafe {
        (f.HidD_GetHidGuid)(&mut interface_class_guid);
    }

    /* Get the list of all device interfaces belonging to the HID class. */
    /* Retry in case of list was changed between calls to
    CM_Get_Device_Interface_List_SizeW and CM_Get_Device_Interface_ListW */
    let mut device_interface_list: Vec<u16>;
    loop {
        let mut len: u32 = 0;
        // SAFETY: valid out-parameters.
        let cr = unsafe {
            (f.CM_Get_Device_Interface_List_SizeW)(
                &mut len,
                &interface_class_guid,
                std::ptr::null(),
                CM_GET_DEVICE_INTERFACE_LIST_PRESENT,
            )
        };
        if cr != CR_SUCCESS {
            // "Failed to get size of HID device interface list"
            return Vec::new();
        }

        device_interface_list = vec![0u16; len as usize];
        // SAFETY: the buffer holds len units.
        let cr = unsafe {
            (f.CM_Get_Device_Interface_ListW)(
                &interface_class_guid,
                std::ptr::null(),
                device_interface_list.as_mut_ptr(),
                len,
                CM_GET_DEVICE_INTERFACE_LIST_PRESENT,
            )
        };
        if cr == CR_BUFFER_SMALL {
            continue;
        }
        if cr != CR_SUCCESS {
            // "Failed to get HID device interface list"
            return Vec::new();
        }
        break;
    }

    let mut root = Vec::new();

    // Iterate over each device interface in the HID class, looking for the right one.
    for device_interface in wide_list(&device_interface_list) {
        // XInput devices don't get real HID reports and are better handled by the raw input driver
        if wcsstr(device_interface, "&IG_").is_some() {
            continue;
        }

        let path: Vec<u16> = device_interface
            .iter()
            .copied()
            .chain(std::iter::once(0))
            .collect();

        // Open read-only handle to the device
        let device_handle = OwnedHandle(open_device(&path, false));

        // Check validity of device_handle.
        if device_handle.0 == INVALID_HANDLE_VALUE {
            // Unable to open the device.
            continue;
        }

        // Get the Vendor ID and Product ID for this device.
        let mut attrib = HIDD_ATTRIBUTES {
            Size: size_of::<HIDD_ATTRIBUTES>() as u32,
            ..HIDD_ATTRIBUTES::default()
        };
        // SAFETY: the handle is open and attrib is writable.
        if unsafe { (f.HidD_GetAttributes)(device_handle.0, &mut attrib) } == 0 {
            continue;
        }

        // See if there are any devices we should skip in enumeration
        let bus_type = f.detect_bus_type(&path).bus_type;
        let caps = f.caps(device_handle.0).unwrap_or_default();
        if super::should_ignore_device(
            bus_type,
            attrib.VendorID,
            attrib.ProductID,
            caps.UsagePage,
            caps.Usage,
            false,
            false,
        ) {
            continue;
        }

        /* Check the VID/PID to see if we should add this
        device to the enumeration list. */
        if (vendor_id == 0x0 || attrib.VendorID == vendor_id)
            && (product_id == 0x0 || attrib.ProductID == product_id)
            && !hid_blacklist(attrib.VendorID, attrib.ProductID)
        {
            // VID/PID match. Create the record.
            root.push(f.get_device_info(&path, device_handle.0));
        }
    }

    // (with no devices, upstream sets "No HID devices found in the system."
    // or "No HID devices with requested VID/PID found in the system.")
    root
}

/// Translation of `hid_open()`.
pub(super) fn open(vendor_id: u16, product_id: u16, serial_number: Option<&str>) -> Result<Device> {
    let devs = hid_enumerate(vendor_id, product_id);
    if devs.is_empty() {
        return Err(Error::new(if vendor_id == 0 && product_id == 0 {
            "No HID devices found in the system."
        } else {
            "No HID devices with requested VID/PID found in the system."
        }));
    }

    let path_to_open = devs
        .iter()
        .find(|cur_dev| {
            cur_dev.vendor_id == vendor_id
                && cur_dev.product_id == product_id
                && serial_number
                    .is_none_or(|serial| cur_dev.serial_number.as_deref() == Some(serial))
        })
        .map(|cur_dev| cur_dev.path.clone());

    match path_to_open {
        // Open the device
        Some(Some(path)) => open_path(&path),
        _ => Err(Error::new(
            "Device with requested VID/PID/(SerialNumber) not found",
        )),
    }
}

/// An `OVERLAPPED` with its own auto-reset event, at a stable address.
struct Overlapped(Box<OVERLAPPED>);

impl Overlapped {
    fn new() -> Overlapped {
        // SAFETY: OVERLAPPED is plain data, all-zero is its initial state.
        let mut ol: Box<OVERLAPPED> = Box::new(unsafe { std::mem::zeroed() });
        // SAFETY: an unnamed auto-reset event, initially nonsignaled.
        ol.hEvent = unsafe {
            CreateEventW(
                std::ptr::null(),
                0,
                0, /*initial state f=nonsignaled*/
                std::ptr::null(),
            )
        };
        Overlapped(ol)
    }
}

impl Drop for Overlapped {
    fn drop(&mut self) {
        // SAFETY: the event was created by Overlapped::new and is closed
        // once, after any I/O using it is finished (see Device::drop).
        unsafe {
            CloseHandle(self.0.hEvent);
        }
    }
}

/// The state of the overlapped reads (`read_pending`, `read_buf`, `ol`).
struct ReadState {
    pending: bool,
    buf: Box<[u8]>,
    ol: Overlapped,
}

/// The state of the overlapped writes (`write_buf`, `write_ol`).
struct WriteState {
    buf: Box<[u8]>,
    ol: Overlapped,
}

/// An open HID device. Translation of `struct hid_device_`.
pub(super) struct Device {
    functions: Arc<Functions>,
    device_handle: HANDLE,
    blocking: AtomicBool,
    output_report_length: u16,
    input_report_length: usize,
    feature_report_length: u16,
    /// `feature_buf`, made on first use.
    feature_buf: Mutex<Vec<u8>>,
    read: Mutex<ReadState>,
    write: Mutex<WriteState>,
    device_info: Option<DeviceInfo>,
    use_hid_write_output_report: bool,
}

// SAFETY: the device handle is a kernel handle usable from any thread; the
// buffers and OVERLAPPED structures the kernel reads and writes are behind
// the mutexes, which serialize their use.
unsafe impl Send for Device {}
// SAFETY: as above.
unsafe impl Sync for Device {}

impl std::fmt::Debug for Device {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Device")
            .field("device_info", &self.device_info)
            .finish_non_exhaustive()
    }
}

/// Translation of `hid_open_path()`.
pub(super) fn open_path(path: &str) -> Result<Device> {
    let f = functions()?;

    let interface_path = crate::core::windows::utf8_to_wide(path);

    // Open a handle to the device
    let mut device_handle = OwnedHandle(open_device(&interface_path, true));

    // Check validity of write_handle.
    if device_handle.0 == INVALID_HANDLE_VALUE {
        /* System devices, such as keyboards and mice, cannot be opened in
        read-write mode, because the system takes exclusive control over
        them.  This is to prevent keyloggers.  However, feature reports
        can still be sent and received.  Retry opening the device, but
        without read/write access. */
        device_handle = OwnedHandle(open_device(&interface_path, false));

        // Check the validity of the limited device_handle.
        if device_handle.0 == INVALID_HANDLE_VALUE {
            return Err(winapi_error("open_device"));
        }
    }

    // Set the Input Report buffer size to 64 reports.
    // SAFETY: the handle is open.
    if unsafe { (f.HidD_SetNumInputBuffers)(device_handle.0, 64) } == 0 {
        return Err(winapi_error("set input buffers"));
    }

    // Get the Input Report length for the device.
    let mut pp_data: PHIDP_PREPARSED_DATA = std::ptr::null_mut();
    // SAFETY: the handle is open and pp_data is writable.
    if unsafe { (f.HidD_GetPreparsedData)(device_handle.0, &mut pp_data) } == 0 {
        return Err(winapi_error("get preparsed data"));
    }
    let mut caps = HIDP_CAPS::default();
    // SAFETY: pp_data came from HidD_GetPreparsedData and is freed once.
    let status = unsafe { (f.HidP_GetCaps)(pp_data, &mut caps) };
    // SAFETY: as above.
    unsafe {
        (f.HidD_FreePreparsedData)(pp_data);
    }
    if status != HIDP_STATUS_SUCCESS {
        return Err(Error::new("HidP_GetCaps"));
    }

    let handle = std::mem::replace(&mut device_handle.0, INVALID_HANDLE_VALUE);
    let output_report_length = caps.OutputReportByteLength;
    let input_report_length = usize::from(caps.InputReportByteLength);
    let device_info = Some(f.get_device_info(&interface_path, handle));

    // On Windows 7, we need to use hid_write_output_report() over Bluetooth
    let use_hid_write_output_report =
        output_report_length > 512 && !crate::core::windows::is_windows_8_or_greater();

    Ok(Device {
        functions: f,
        device_handle: handle,
        blocking: AtomicBool::new(true),
        output_report_length,
        input_report_length,
        feature_report_length: caps.FeatureReportByteLength,
        feature_buf: Mutex::new(Vec::new()),
        read: Mutex::new(ReadState {
            pending: false,
            buf: vec![0u8; input_report_length].into_boxed_slice(),
            ol: Overlapped::new(),
        }),
        write: Mutex::new(WriteState {
            buf: vec![0u8; usize::from(output_report_length)].into_boxed_slice(),
            ol: Overlapped::new(),
        }),
        device_info,
        use_hid_write_output_report,
    })
}

impl Device {
    /// Translation of `hid_write_output_report()`.
    fn write_output_report(&self, data: &[u8]) -> Result<usize> {
        let mut buf = data.to_vec();
        // SAFETY: the buffer holds data.len() bytes.
        let res = unsafe {
            (self.functions.HidD_SetOutputReport)(
                self.device_handle,
                buf.as_mut_ptr().cast(),
                buf.len() as u32,
            )
        };
        if res != 0 {
            Ok(data.len())
        } else {
            // FIXME (upstream): the failure is returned with the error
            // cleared, so it has no message.
            Err(Error::new(""))
        }
    }

    /// Translation of `hid_write()`.
    pub(super) fn write(&self, data: &[u8]) -> Result<usize> {
        if data.is_empty() {
            return Err(Error::new("Zero buffer/length"));
        }

        if self.use_hid_write_output_report {
            return self.write_output_report(data);
        }

        let mut state = self.write.lock().unwrap_or_else(|e| e.into_inner());
        let state = &mut *state;

        /* Make sure the right number of bytes are passed to WriteFile. Windows
        expects the number of bytes which are in the _longest_ report (plus
        one for the report number) bytes even if the data is a report
        which is shorter than that. Windows gives us this value in
        caps.OutputReportByteLength. If a user passes in fewer bytes than this,
        use cached temporary buffer which is the proper size. */
        // (the data is always copied into the device's buffer, which outlives
        // a write that is still in flight; upstream writes from the caller's
        // buffer when it is long enough)
        let length = data.len().max(usize::from(self.output_report_length));
        if state.buf.len() < length {
            state.buf = vec![0u8; length].into_boxed_slice();
        }
        state.buf[..data.len()].copy_from_slice(data);
        state.buf[data.len()..length].fill(0);

        let mut bytes_written: u32 = 0;
        // SAFETY: the buffer and the OVERLAPPED are owned by the device and
        // stay in place until the write is finished (waited for below, or
        // cancelled).
        let res = unsafe {
            WriteFile(
                self.device_handle,
                state.buf.as_ptr(),
                length as u32,
                &mut bytes_written,
                &mut *state.ol.0,
            )
        };

        if res == 0 {
            // SAFETY: GetLastError has no preconditions.
            if unsafe { GetLastError() } != ERROR_IO_PENDING {
                // WriteFile() failed. Return error.
                return Err(winapi_error("WriteFile"));
            }
        } else {
            // WriteFile() succeeded synchronously.
            return Ok(bytes_written as usize);
        }

        /* Wait for the transaction to complete. This makes
        hid_write() synchronous. */
        // SAFETY: the event belongs to the OVERLAPPED.
        let res = unsafe { WaitForSingleObject(state.ol.0.hEvent, 1000) };
        if res != WAIT_OBJECT_0 {
            // There was a Timeout.
            let error = winapi_error("hid_write/WaitForSingleObject");
            // FIXME (upstream): the timed out write is left in flight,
            // reading the buffer and using the OVERLAPPED after hid_write()
            // returned; here it is cancelled and finished first.
            // SAFETY: the OVERLAPPED is the one of the pending write.
            unsafe {
                CancelIoEx(self.device_handle, &*state.ol.0);
                GetOverlappedResult(self.device_handle, &*state.ol.0, &mut bytes_written, 1);
            }
            return Err(error);
        }

        // Get the result.
        // SAFETY: the write is complete; valid out-parameter.
        let res = unsafe {
            GetOverlappedResult(
                self.device_handle,
                &*state.ol.0,
                &mut bytes_written,
                0, /*wait*/
            )
        };
        if res != 0 {
            Ok(bytes_written as usize)
        } else {
            // The Write operation failed.
            Err(winapi_error("hid_write/GetOverlappedResult"))
        }
    }

    /// Translation of `hid_read_timeout()`.
    pub(super) fn read_timeout(&self, data: &mut [u8], milliseconds: i32) -> Result<usize> {
        if data.is_empty() {
            return Err(Error::new("Zero buffer/length"));
        }

        let mut state = self.read.lock().unwrap_or_else(|e| e.into_inner());
        let state = &mut *state;

        let mut bytes_read: u32 = 0;
        let mut copy_len = 0;
        let mut res: i32 = 0;
        let overlapped;

        // Copy the handle for convenience.
        let ev = state.ol.0.hEvent;

        if !state.pending {
            // Start an Overlapped I/O read.
            state.pending = true;
            state.buf.fill(0);
            // SAFETY: the event belongs to the OVERLAPPED; the buffer and the
            // OVERLAPPED are owned by the device and stay in place while the
            // read is pending (Device::drop finishes it).
            res = unsafe {
                ResetEvent(ev);
                ReadFile(
                    self.device_handle,
                    state.buf.as_mut_ptr(),
                    self.input_report_length as u32,
                    &mut bytes_read,
                    &mut *state.ol.0,
                )
            };

            if res == 0 {
                // SAFETY: GetLastError has no preconditions.
                if unsafe { GetLastError() } != ERROR_IO_PENDING {
                    /* ReadFile() has failed.
                    Clean up and return error. */
                    let error = winapi_error("ReadFile");
                    // SAFETY: the handle is open.
                    unsafe {
                        CancelIo(self.device_handle);
                    }
                    state.pending = false;
                    return Err(error);
                }
                overlapped = true;
            } else {
                overlapped = false;
            }
        } else {
            overlapped = true;
        }

        if overlapped {
            // See if there is any data yet.
            let wait = if milliseconds >= 0 {
                milliseconds as u32
            } else {
                INFINITE
            };
            // SAFETY: the event belongs to the pending read.
            if unsafe { WaitForSingleObject(ev, wait) } != WAIT_OBJECT_0 {
                /* There was no data this time. Return zero bytes available,
                but leave the Overlapped I/O running. */
                return Ok(0);
            }

            /* Get the number of bytes read. The actual data has been copied to the data[]
            array which was passed to ReadFile(). We must not wait here because we've
            already waited on our event above, and since it's auto-reset, it will have
            been reset back to unsignalled by now. */
            // SAFETY: the read is complete; valid out-parameter.
            res = unsafe {
                GetOverlappedResult(
                    self.device_handle,
                    &*state.ol.0,
                    &mut bytes_read,
                    0, /*don't wait now - already did on the prev step*/
                )
            };
        }
        // Set pending back to false, even if GetOverlappedResult() returned error.
        state.pending = false;

        if res != 0 && bytes_read > 0 {
            let bytes_read = (bytes_read as usize).min(state.buf.len());
            if state.buf[0] == 0x0 {
                /* If report numbers aren't being used, but Windows sticks a report
                number (0x0) on the beginning of the report anyway. To make this
                work like the other platforms, and to make it work more like the
                HID spec, we'll skip over this byte. */
                copy_len = data.len().min(bytes_read - 1);
                data[..copy_len].copy_from_slice(&state.buf[1..1 + copy_len]);
            } else {
                // Copy the whole buffer, report number and all.
                copy_len = data.len().min(bytes_read);
                data[..copy_len].copy_from_slice(&state.buf[..copy_len]);
            }
        }
        if res == 0 {
            // SAFETY: GetLastError has no preconditions.
            let error = unsafe { GetLastError() };
            if error == ERROR_OPERATION_ABORTED {
                /* The read request was issued on another thread.
                This is harmless, so just ignore it. */
                return Ok(0);
            }
            return Err(winapi_error_code(
                "hid_read_timeout/GetOverlappedResult",
                error,
            ));
        }

        Ok(copy_len)
    }

    /// Translation of `hid_read()`.
    pub(super) fn read(&self, data: &mut [u8]) -> Result<usize> {
        let milliseconds = if self.blocking.load(Ordering::Relaxed) {
            -1
        } else {
            0
        };
        self.read_timeout(data, milliseconds)
    }

    /// Translation of `hid_set_nonblocking()`.
    pub(super) fn set_nonblocking(&self, nonblock: bool) -> Result<()> {
        self.blocking.store(!nonblock, Ordering::Relaxed);
        Ok(()) // Success
    }

    /// Translation of `hid_send_feature_report()`.
    pub(super) fn send_feature_report(&self, data: &[u8]) -> Result<usize> {
        if data.is_empty() {
            return Err(Error::new("Zero buffer/length"));
        }

        /* Windows expects at least caps.FeatureReportByteLength bytes passed
        to HidD_SetFeature(), even if the report is shorter. Any less sent and
        the function fails with error ERROR_INVALID_PARAMETER set. Any more
        and HidD_SetFeature() silently truncates the data sent in the report
        to caps.FeatureReportByteLength. */
        let mut feature_buf = self.feature_buf.lock().unwrap_or_else(|e| e.into_inner());
        let length_to_send = data.len().max(usize::from(self.feature_report_length));
        feature_buf.clear();
        feature_buf.extend_from_slice(data);
        feature_buf.resize(length_to_send, 0);

        // SAFETY: the buffer holds length_to_send bytes.
        let res = unsafe {
            (self.functions.HidD_SetFeature)(
                self.device_handle,
                feature_buf.as_mut_ptr().cast(),
                length_to_send as u32,
            )
        };

        if res == 0 {
            return Err(winapi_error("HidD_SetFeature"));
        }

        Ok(data.len())
    }

    /// Translation of `hid_get_report()`.
    fn get_report(&self, report_type: u32, data: &mut [u8]) -> Result<usize> {
        if data.is_empty() {
            return Err(Error::new("Zero buffer/length"));
        }

        let mut bytes_returned: u32 = 0;
        // SAFETY: OVERLAPPED is plain data, all-zero is its initial state.
        let mut ol: OVERLAPPED = unsafe { std::mem::zeroed() };

        // SAFETY: data is readable and writable for its length; the
        // OVERLAPPED and the buffer outlive the I/O, which is waited for
        // below.
        let res = unsafe {
            DeviceIoControl(
                self.device_handle,
                report_type,
                data.as_ptr().cast(),
                data.len() as u32,
                data.as_mut_ptr().cast(),
                data.len() as u32,
                &mut bytes_returned,
                &mut ol,
            )
        };

        if res == 0 {
            // SAFETY: GetLastError has no preconditions.
            if unsafe { GetLastError() } != ERROR_IO_PENDING {
                // DeviceIoControl() failed. Return error.
                return Err(winapi_error("Get Input/Feature Report DeviceIoControl"));
            }
        }

        /* Wait here until the write is done. This makes
        hid_get_feature_report() synchronous. */
        // SAFETY: the OVERLAPPED is the one of the I/O started above.
        let res = unsafe {
            GetOverlappedResult(
                self.device_handle,
                &ol,
                &mut bytes_returned,
                1, /*wait*/
            )
        };
        if res == 0 {
            // The operation failed.
            return Err(winapi_error("Get Input/Feature Report GetOverLappedResult"));
        }

        /* When numbered reports aren't used,
        bytes_returned seem to include only what is actually received from the device
        (not including the first byte with 0, as an indication "no numbered reports"). */
        let mut bytes_returned = bytes_returned as usize;
        if data[0] == 0x0 {
            bytes_returned += 1;
        }

        // FIXME (upstream): with the extra report number byte the length
        // can be one more than the buffer; here it stays within it.
        Ok(bytes_returned.min(data.len()))
    }

    /// Translation of `hid_get_feature_report()`.
    pub(super) fn get_feature_report(&self, data: &mut [u8]) -> Result<usize> {
        // We could use HidD_GetFeature() instead, but it doesn't give us an actual length, unfortunately
        self.get_report(IOCTL_HID_GET_FEATURE, data)
    }

    /// Translation of `hid_get_input_report()`.
    pub(super) fn get_input_report(&self, data: &mut [u8]) -> Result<usize> {
        // We could use HidD_GetInputReport() instead, but it doesn't give us an actual length, unfortunately
        self.get_report(IOCTL_HID_GET_INPUT_REPORT, data)
    }

    fn info(&self) -> Result<&DeviceInfo> {
        self.device_info
            .as_ref()
            .ok_or_else(|| Error::new("NULL device info"))
    }

    /// Translation of `hid_get_manufacturer_string()`.
    pub(super) fn get_manufacturer_string(&self) -> Result<String> {
        Ok(self.info()?.manufacturer_string.clone().unwrap_or_default())
    }

    /// Translation of `hid_get_product_string()`.
    pub(super) fn get_product_string(&self) -> Result<String> {
        Ok(self.info()?.product_string.clone().unwrap_or_default())
    }

    /// Translation of `hid_get_serial_number_string()`.
    pub(super) fn get_serial_number_string(&self) -> Result<String> {
        Ok(self.info()?.serial_number.clone().unwrap_or_default())
    }

    /// Translation of `hid_get_device_info()`.
    pub(super) fn get_device_info(&self) -> Result<DeviceInfo> {
        self.info().cloned()
    }

    /// Translation of `hid_get_indexed_string()` (with a buffer of
    /// `MAX_STRING_WCHARS`).
    pub(super) fn get_indexed_string(&self, string_index: i32) -> Result<String> {
        let mut string = [0u16; MAX_STRING_WCHARS + 1];
        let mut maxlen = MAX_STRING_WCHARS;

        if self
            .device_info
            .as_ref()
            .is_some_and(|info| info.bus_type == BusType::Usb)
            && maxlen > MAX_STRING_WCHARS_USB
        {
            string[MAX_STRING_WCHARS_USB] = 0;
            maxlen = MAX_STRING_WCHARS_USB;
        }

        // SAFETY: the buffer holds more than maxlen units.
        let res = unsafe {
            (self.functions.HidD_GetIndexedString)(
                self.device_handle,
                string_index as u32,
                string.as_mut_ptr().cast(),
                (maxlen * size_of::<u16>()) as u32,
            )
        };
        if res == 0 {
            return Err(winapi_error("HidD_GetIndexedString"));
        }

        Ok(wide_to_string(&string[..maxlen]))
    }

    /// The container ID of the device. Translation of
    /// `hid_winapi_get_container_id()`.
    #[allow(dead_code)] // (part of hidapi's Windows extensions; SDL doesn't use it)
    pub(super) fn get_container_id(&self) -> Result<GUID> {
        let f = &self.functions;
        let path = self
            .device_info
            .as_ref()
            .and_then(|info| info.path.as_deref())
            .ok_or_else(|| Error::new("Path conversion failure"))?;
        let interface_path = crate::core::windows::utf8_to_wide(path);

        // Get the device id from interface path
        let device_id = f
            .get_device_interface_property(
                &interface_path,
                &DEVPKEY_Device_InstanceId,
                DEVPROP_TYPE_STRING,
            )
            .map(|bytes| bytes_to_wide(&bytes))
            .ok_or_else(|| Error::new("Failed to get device interface property InstanceId"))?;

        // Open devnode from device id
        let mut dev_node: DEVINST = 0;
        // SAFETY: the device id is NUL-terminated.
        let cr = unsafe {
            (f.CM_Locate_DevNodeW)(&mut dev_node, device_id.as_ptr(), CM_LOCATE_DEVNODE_NORMAL)
        };
        if cr != CR_SUCCESS {
            return Err(Error::new("Failed to locate device node"));
        }

        // Get the container id from devnode
        let mut container_id = GUID {
            data1: 0,
            data2: 0,
            data3: 0,
            data4: [0; 8],
        };
        let mut len = size_of::<GUID>() as u32;
        let mut property_type: DEVPROPTYPE = 0;
        // SAFETY: the buffer holds len bytes.
        let mut cr = unsafe {
            (f.CM_Get_DevNode_PropertyW)(
                dev_node,
                &DEVPKEY_Device_ContainerId,
                &mut property_type,
                (&mut container_id as *mut GUID).cast(),
                &mut len,
                0,
            )
        };
        if cr == CR_SUCCESS && property_type != DEVPROP_TYPE_GUID {
            cr = CR_FAILURE;
        }

        if cr != CR_SUCCESS {
            return Err(Error::new(
                "Failed to read ContainerId property from device node",
            ));
        }
        Ok(container_id)
    }

    /// Translation of `hid_get_report_descriptor()`.
    pub(super) fn get_report_descriptor(&self, buf: &mut [u8]) -> Result<usize> {
        let f = &self.functions;
        let mut pp_data: PHIDP_PREPARSED_DATA = std::ptr::null_mut();

        // SAFETY: the handle is open and pp_data is writable.
        if unsafe { (f.HidD_GetPreparsedData)(self.device_handle, &mut pp_data) } == 0
            || pp_data.is_null()
        {
            return Err(Error::new("HidD_GetPreparsedData"));
        }

        // (the size of the structure is in its header)
        // SAFETY: the preparsed data starts with its header of
        // PP_DATA_CAPS_OFFSET bytes...
        let header =
            unsafe { std::slice::from_raw_parts(pp_data.cast::<u8>(), PP_DATA_CAPS_OFFSET) };
        let size = preparsed_data_size(header).unwrap_or(PP_DATA_CAPS_OFFSET);
        // SAFETY: ...and holds the caps and link collections it describes,
        // which end at the size computed from it.
        let bytes = unsafe { std::slice::from_raw_parts(pp_data.cast::<u8>(), size) };
        let res = reconstruct_pp_data(bytes, buf);

        // SAFETY: pp_data came from HidD_GetPreparsedData and is freed once.
        unsafe {
            (f.HidD_FreePreparsedData)(pp_data);
        }

        res.map_err(|()| Error::new("Couldn't reconstruct the report descriptor"))
    }
}

impl Drop for Device {
    /// Translation of `hid_close()`.
    fn drop(&mut self) {
        let state = self.read.get_mut().unwrap_or_else(|e| e.into_inner());
        // SAFETY: the handle is open; cancelling and then waiting for the
        // pending read finishes the kernel's use of the buffer and the
        // OVERLAPPED before they are freed.
        unsafe {
            CancelIoEx(self.device_handle, std::ptr::null());
            if state.pending {
                let mut bytes_read: u32 = 0;
                GetOverlappedResult(
                    self.device_handle,
                    &*state.ol.0,
                    &mut bytes_read,
                    1, /*wait*/
                );
            }
            CloseHandle(self.device_handle);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ioctl_codes() {
        // (from the Windows DDK's hidclass.h)
        assert_eq!(IOCTL_HID_GET_FEATURE, 0x000B0192);
        assert_eq!(IOCTL_HID_GET_INPUT_REPORT, 0x000B01A2);
        assert_eq!(size_of::<HIDP_CAPS>(), 64);
        assert_eq!(size_of::<HIDD_ATTRIBUTES>(), 12);
    }

    #[test]
    fn hardware_id_tokens() {
        let id: Vec<u16> = r"USB\VID_045E&PID_028E&REV_0114&MI_02"
            .encode_utf16()
            .collect();
        assert_eq!(hid_internal_extract_int_token_value(&id, "REV_"), 0x114);
        assert_eq!(hid_internal_extract_int_token_value(&id, "MI_"), 2);
        assert_eq!(hid_internal_extract_int_token_value(&id, "IG_"), -1);
        let bad: Vec<u16> = "REV_XYZ".encode_utf16().collect();
        assert_eq!(hid_internal_extract_int_token_value(&bad, "REV_"), -1);
        let lower: Vec<u16> = "hid\\vid_045e".encode_utf16().collect();
        assert_eq!(
            towupper(&lower),
            "HID\\VID_045E".encode_utf16().collect::<Vec<_>>()
        );
        let list: Vec<u16> = "A\0BC\0\0".encode_utf16().collect();
        let items: Vec<String> = wide_list(&list).map(String::from_utf16_lossy).collect();
        assert_eq!(items, ["A", "BC"]);
        assert!(hid_blacklist(0x1CCF, 0x1234));
        assert!(!hid_blacklist(0x045E, 0x028E));
    }

    #[test]
    fn library_loads() {
        let _l = crate::test_support::test_lock();
        // (hid.dll and cfgmgr32.dll exist under Wine; there are no devices)
        match hid_init() {
            Ok(()) => {
                let devices = hid_enumerate(0, 0);
                println!("note: {} HID devices", devices.len());
                assert!(open_path(r"\\?\nonexistent").is_err());
                hid_exit().unwrap();
            }
            Err(e) => println!("note: hid.dll unavailable ({}), skipping", e.message()),
        }
        let error = winapi_error_code("op", 2);
        assert!(
            error.message().starts_with("op: (0x00000002)"),
            "{}",
            error.message()
        );
        assert!(!error.message().ends_with('\n'));
    }
}
