// Rust translation of src/core/windows/SDL_immdevice.c and
// src/core/windows/SDL_immdevice.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The MMDevice API glue the WASAPI driver uses: endpoint enumeration,
//! device names and formats, hotplug notifications (an
//! `IMMNotificationClient` implemented here) and the handles SDL keeps for
//! each endpoint. The COM interfaces aren't in `windows-sys`, so their
//! vtables are declared by hand.

use std::collections::HashMap;
use std::ffi::c_void;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicPtr, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use windows_sys::core::{GUID, HRESULT, PCWSTR, PWSTR};
use windows_sys::Win32::Foundation::{E_NOINTERFACE, PROPERTYKEY, S_OK};
use windows_sys::Win32::Media::Audio::{
    eCapture, eConsole, eRender, EDataFlow, ERole, DEVICE_STATE_ACTIVE, WAVEFORMATEXTENSIBLE,
};
use windows_sys::Win32::System::Com::StructuredStorage::{PropVariantClear, PROPVARIANT};
use windows_sys::Win32::System::Com::{
    CLSIDFromString, CoCreateInstance, CoTaskMemFree, CLSCTX_INPROC_SERVER, STGM_READ,
};

use super::{co_initialize, co_uninitialize, error_from_hresult, is_equal_guid, wide_to_utf8};
use crate::audio::device::{
    add_audio_device, audio_device_disconnected, default_audio_device_changed,
    find_physical_audio_device_by_callback, PhysicalDevice,
};
use crate::audio::{AudioFormat, AudioSpec};
use crate::error::{Error, Result};

/// `SUCCEEDED()`.
pub(crate) fn succeeded(hr: HRESULT) -> bool {
    hr >= 0
}

/// `FAILED()`.
pub(crate) fn failed(hr: HRESULT) -> bool {
    hr < 0
}

/// `E_NOTFOUND` from <mmdeviceapi.h>: `HRESULT_FROM_WIN32(ERROR_NOT_FOUND)`.
pub(crate) const E_NOTFOUND: HRESULT = 0x8007_0490_u32 as HRESULT;

// --- the COM interfaces SDL uses (<unknwn.h>, <mmdeviceapi.h>, <propsys.h>) ---

/// A COM object: a pointer to its vtable.
#[repr(C)]
pub(crate) struct ComObject<V> {
    pub(crate) vtbl: *const V,
}

impl<V> ComObject<V> {
    /// The object's vtable.
    ///
    /// # Safety
    ///
    /// `this` must be a live COM object whose vtable is a `V`.
    pub(crate) unsafe fn vtbl<'a>(this: *mut ComObject<V>) -> &'a V {
        // SAFETY: the caller's contract.
        unsafe { &*(*this).vtbl }
    }
}

/// `IUnknownVtbl`.
#[repr(C)]
pub(crate) struct IUnknownVtbl {
    pub(crate) query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    pub(crate) add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(crate) release: unsafe extern "system" fn(*mut c_void) -> u32,
}

/// `IMMDeviceEnumeratorVtbl`.
#[repr(C)]
pub(crate) struct IMMDeviceEnumeratorVtbl {
    pub(crate) base: IUnknownVtbl,
    pub(crate) enum_audio_endpoints: unsafe extern "system" fn(
        *mut IMMDeviceEnumerator,
        EDataFlow,
        u32,
        *mut *mut IMMDeviceCollection,
    ) -> HRESULT,
    pub(crate) get_default_audio_endpoint: unsafe extern "system" fn(
        *mut IMMDeviceEnumerator,
        EDataFlow,
        ERole,
        *mut *mut IMMDevice,
    ) -> HRESULT,
    pub(crate) get_device:
        unsafe extern "system" fn(*mut IMMDeviceEnumerator, PCWSTR, *mut *mut IMMDevice) -> HRESULT,
    pub(crate) register_endpoint_notification_callback:
        unsafe extern "system" fn(*mut IMMDeviceEnumerator, *mut IMMNotificationClient) -> HRESULT,
    pub(crate) unregister_endpoint_notification_callback:
        unsafe extern "system" fn(*mut IMMDeviceEnumerator, *mut IMMNotificationClient) -> HRESULT,
}
pub(crate) type IMMDeviceEnumerator = ComObject<IMMDeviceEnumeratorVtbl>;

/// `IMMDeviceCollectionVtbl`.
#[repr(C)]
pub(crate) struct IMMDeviceCollectionVtbl {
    pub(crate) base: IUnknownVtbl,
    pub(crate) get_count: unsafe extern "system" fn(*mut IMMDeviceCollection, *mut u32) -> HRESULT,
    pub(crate) item:
        unsafe extern "system" fn(*mut IMMDeviceCollection, u32, *mut *mut IMMDevice) -> HRESULT,
}
pub(crate) type IMMDeviceCollection = ComObject<IMMDeviceCollectionVtbl>;

/// `IMMDeviceVtbl`.
#[repr(C)]
pub(crate) struct IMMDeviceVtbl {
    pub(crate) base: IUnknownVtbl,
    pub(crate) activate: unsafe extern "system" fn(
        *mut IMMDevice,
        *const GUID,
        u32,
        *mut PROPVARIANT,
        *mut *mut c_void,
    ) -> HRESULT,
    pub(crate) open_property_store:
        unsafe extern "system" fn(*mut IMMDevice, u32, *mut *mut IPropertyStore) -> HRESULT,
    pub(crate) get_id: unsafe extern "system" fn(*mut IMMDevice, *mut PWSTR) -> HRESULT,
    pub(crate) get_state: unsafe extern "system" fn(*mut IMMDevice, *mut u32) -> HRESULT,
}
pub(crate) type IMMDevice = ComObject<IMMDeviceVtbl>;

/// `IMMEndpointVtbl`.
#[repr(C)]
pub(crate) struct IMMEndpointVtbl {
    pub(crate) base: IUnknownVtbl,
    pub(crate) get_data_flow:
        unsafe extern "system" fn(*mut IMMEndpoint, *mut EDataFlow) -> HRESULT,
}
pub(crate) type IMMEndpoint = ComObject<IMMEndpointVtbl>;

/// `IPropertyStoreVtbl`.
#[repr(C)]
pub(crate) struct IPropertyStoreVtbl {
    pub(crate) base: IUnknownVtbl,
    pub(crate) get_count: unsafe extern "system" fn(*mut IPropertyStore, *mut u32) -> HRESULT,
    pub(crate) get_at:
        unsafe extern "system" fn(*mut IPropertyStore, u32, *mut PROPERTYKEY) -> HRESULT,
    pub(crate) get_value: unsafe extern "system" fn(
        *mut IPropertyStore,
        *const PROPERTYKEY,
        *mut PROPVARIANT,
    ) -> HRESULT,
    pub(crate) set_value: unsafe extern "system" fn(
        *mut IPropertyStore,
        *const PROPERTYKEY,
        *const PROPVARIANT,
    ) -> HRESULT,
    pub(crate) commit: unsafe extern "system" fn(*mut IPropertyStore) -> HRESULT,
}
pub(crate) type IPropertyStore = ComObject<IPropertyStoreVtbl>;

/// `IMMNotificationClientVtbl`.
#[repr(C)]
pub(crate) struct IMMNotificationClientVtbl {
    pub(crate) query_interface: unsafe extern "system" fn(
        *mut IMMNotificationClient,
        *const GUID,
        *mut *mut c_void,
    ) -> HRESULT,
    pub(crate) add_ref: unsafe extern "system" fn(*mut IMMNotificationClient) -> u32,
    pub(crate) release: unsafe extern "system" fn(*mut IMMNotificationClient) -> u32,
    pub(crate) on_device_state_changed:
        unsafe extern "system" fn(*mut IMMNotificationClient, PCWSTR, u32) -> HRESULT,
    pub(crate) on_device_added:
        unsafe extern "system" fn(*mut IMMNotificationClient, PCWSTR) -> HRESULT,
    pub(crate) on_device_removed:
        unsafe extern "system" fn(*mut IMMNotificationClient, PCWSTR) -> HRESULT,
    pub(crate) on_default_device_changed:
        unsafe extern "system" fn(*mut IMMNotificationClient, EDataFlow, ERole, PCWSTR) -> HRESULT,
    pub(crate) on_property_value_changed:
        unsafe extern "system" fn(*mut IMMNotificationClient, PCWSTR, PROPERTYKEY) -> HRESULT,
}
pub(crate) type IMMNotificationClient = ComObject<IMMNotificationClientVtbl>;

/// `IUnknown_Release()` on any COM object.
///
/// # Safety
///
/// `this` must be a live COM object; the caller gives up one reference.
pub(crate) unsafe fn release<V>(this: *mut ComObject<V>) -> u32 {
    // SAFETY: every COM vtable starts with IUnknown's (the caller's contract).
    unsafe {
        let base = &*(*this).vtbl.cast::<IUnknownVtbl>();
        (base.release)(this.cast())
    }
}

/// `IUnknown_QueryInterface()` on any COM object.
///
/// # Safety
///
/// `this` must be a live COM object.
pub(crate) unsafe fn query_interface<V, W>(
    this: *mut ComObject<V>,
    iid: &GUID,
    out: *mut *mut ComObject<W>,
) -> HRESULT {
    // SAFETY: every COM vtable starts with IUnknown's (the caller's contract).
    unsafe {
        let base = &*(*this).vtbl.cast::<IUnknownVtbl>();
        (base.query_interface)(this.cast(), iid, out.cast())
    }
}

/// The length of a NUL-terminated wide string (`SDL_wcslen()`).
///
/// # Safety
///
/// `s` must be a NUL-terminated wide string.
pub(crate) unsafe fn wcslen(s: PCWSTR) -> usize {
    let mut n = 0;
    // SAFETY: we stop at the NUL (the caller's contract).
    while unsafe { *s.add(n) } != 0 {
        n += 1;
    }
    n
}

/// A wide string with its NUL (`SDL_wcsdup()`).
///
/// # Safety
///
/// `s` must be a NUL-terminated wide string.
pub(crate) unsafe fn wcsdup(s: PCWSTR) -> Vec<u16> {
    // SAFETY: the caller's contract.
    let n = unsafe { wcslen(s) };
    // SAFETY: `s` has `n` units and a NUL.
    unsafe { std::slice::from_raw_parts(s, n + 1) }.to_vec()
}

/// A NUL-terminated wide string as UTF-8 (`WIN_StringToUTF8W()`), `None` for NULL.
///
/// # Safety
///
/// `s` must be NULL or a NUL-terminated wide string.
unsafe fn string_to_utf8(s: PCWSTR) -> Option<String> {
    if s.is_null() {
        return None;
    }
    // SAFETY: the caller's contract.
    Some(wide_to_utf8(&unsafe { wcsdup(s) }))
}

/// Translation of `SDL_IMMDevice_HandleData`.
#[derive(Clone)]
struct HandleData {
    /// The endpoint id, NUL-terminated (`None` once a zombie device came back).
    immdevice_id: Option<Vec<u16>>,
    directsound_guid: GUID,
}

/// !!! FIXME: should this be eMultimedia? Should be a hint?
const SDL_IMMDEVICE_ROLE: ERole = eConsole;

/// The callbacks for lost devices and default device changes. Translation
/// of `SDL_IMMDevice_callbacks`.
#[derive(Clone, Copy)]
pub(crate) struct ImmDeviceCallbacks {
    pub(crate) audio_device_disconnected: fn(Option<Arc<PhysicalDevice>>),
    pub(crate) default_audio_device_changed: fn(Option<Arc<PhysicalDevice>>),
}

/// This is global to the WASAPI target, to handle hotplug and default device lookup.
static ENUMERATOR: AtomicPtr<IMMDeviceEnumerator> = AtomicPtr::new(ptr::null_mut());
/// `immcallbacks`
static IMMCALLBACKS: Mutex<Option<ImmDeviceCallbacks>> = Mutex::new(None);
/// What each device handle given to `add_audio_device` stands for (the
/// `SDL_IMMDevice_HandleData *` upstream), until `FreeDeviceHandle`.
static HANDLES: Mutex<Option<HashMap<usize, HandleData>>> = Mutex::new(None);
static NEXT_HANDLE: AtomicUsize = AtomicUsize::new(1);

fn enumerator() -> *mut IMMDeviceEnumerator {
    ENUMERATOR.load(Ordering::Acquire)
}

fn callbacks() -> Option<ImmDeviceCallbacks> {
    *IMMCALLBACKS.lock().unwrap_or_else(|e| e.into_inner())
}

fn with_handles<R>(f: impl FnOnce(&mut HashMap<usize, HandleData>) -> R) -> R {
    let mut h = HANDLES.lock().unwrap_or_else(|e| e.into_inner());
    f(h.get_or_insert_with(HashMap::new))
}

fn handle_data(device: &PhysicalDevice) -> Option<HandleData> {
    with_handles(|h| h.get(&device.handle).cloned())
}

// Some GUIDs we need to know without linking to libraries that aren't available before Vista.
pub(crate) const SDL_CLSID_MMDEVICEENUMERATOR: GUID = GUID {
    data1: 0xbcde0395,
    data2: 0xe52f,
    data3: 0x467c,
    data4: [0x8e, 0x3d, 0xc4, 0x57, 0x92, 0x91, 0x69, 0x2e],
};
pub(crate) const SDL_IID_IMMDEVICEENUMERATOR: GUID = GUID {
    data1: 0xa95664d2,
    data2: 0x9614,
    data3: 0x4f35,
    data4: [0xa7, 0x46, 0xde, 0x8d, 0xb6, 0x36, 0x17, 0xe6],
};
pub(crate) const SDL_IID_IMMNOTIFICATIONCLIENT: GUID = GUID {
    data1: 0x7991eec9,
    data2: 0x7e89,
    data3: 0x4d85,
    data4: [0x83, 0x90, 0x6c, 0x70, 0x3c, 0xec, 0x60, 0xc0],
};
pub(crate) const SDL_IID_IMMENDPOINT: GUID = GUID {
    data1: 0x1be09788,
    data2: 0x6894,
    data3: 0x4089,
    data4: [0x85, 0x86, 0x9a, 0x2a, 0x6c, 0x26, 0x5a, 0xc5],
};
pub(crate) const IID_IUNKNOWN: GUID = GUID {
    data1: 0x00000000,
    data2: 0x0000,
    data3: 0x0000,
    data4: [0xc0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46],
};
pub(crate) const SDL_PKEY_DEVICE_FRIENDLYNAME: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID {
        data1: 0xa45c254e,
        data2: 0xdf1c,
        data3: 0x4efd,
        data4: [0x80, 0x20, 0x67, 0xd1, 0x46, 0xa8, 0x50, 0xe0],
    },
    pid: 14,
};
pub(crate) const SDL_PKEY_AUDIOENGINE_DEVICEFORMAT: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID {
        data1: 0xf19f064d,
        data2: 0x82c,
        data3: 0x4e27,
        data4: [0xbc, 0x73, 0x68, 0x82, 0xa1, 0xbb, 0x8e, 0x4c],
    },
    pid: 0,
};
pub(crate) const SDL_PKEY_AUDIOENDPOINT_GUID: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID {
        data1: 0x1da5d803,
        data2: 0xd492,
        data3: 0x4edd,
        data4: [0x8c, 0x23, 0xe0, 0xc0, 0xff, 0xee, 0x7f, 0x0e],
    },
    pid: 4,
};
pub(crate) const SDL_PKEY_AUDIOENDPOINT_STABLEID: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID {
        data1: 0x1da5d803,
        data2: 0xd492,
        data3: 0x4edd,
        data4: [0x8c, 0x23, 0xe0, 0xc0, 0xff, 0xee, 0x7f, 0x0e],
    },
    pid: 12,
};

/// Wide-string equality up to the NUL (`SDL_wcscmp() == 0`).
fn wcs_eq(a: &[u16], b: &[u16]) -> bool {
    let end = |s: &[u16]| s.iter().position(|&c| c == 0).unwrap_or(s.len());
    a[..end(a)] == b[..end(b)]
}

/// Translation of `FindByDevIDCallback()`.
fn find_by_dev_id_callback(device: &PhysicalDevice, devid: &[u16]) -> bool {
    handle_data(device)
        .and_then(|h| h.immdevice_id)
        .is_some_and(|id| wcs_eq(&id, devid))
}

/// Translation of `SDL_IMMDevice_FindByDevID()`.
fn sdl_immdevice_find_by_dev_id(devid: &[u16]) -> Option<Arc<PhysicalDevice>> {
    find_physical_audio_device_by_callback(|d| find_by_dev_id_callback(d, devid)).ok()
}

/// Translation of `SDL_IMMDevice_GetDirectSoundGUID()`.
pub(crate) fn get_direct_sound_guid(device: &PhysicalDevice) -> Option<GUID> {
    handle_data(device).map(|h| h.directsound_guid)
}

/// Translation of `SDL_IMMDevice_GetDevID()`: the NUL-terminated endpoint id.
pub(crate) fn get_dev_id(device: &PhysicalDevice) -> Option<Vec<u16>> {
    handle_data(device).and_then(|h| h.immdevice_id)
}

/// What `GetMMDeviceInfo()` reads about an endpoint.
struct MMDeviceInfo {
    utf8dev: Option<String>,
    fmt: WAVEFORMATEXTENSIBLE,
    guid: GUID,
    unique_id: Option<String>,
}

/// Translation of `GetMMDeviceInfo()`.
///
/// # Safety
///
/// `device` must be a live `IMMDevice`.
unsafe fn get_mm_device_info(device: *mut IMMDevice, guid: GUID) -> MMDeviceInfo {
    /* PKEY_Device_FriendlyName gives you "Speakers (SoundBlaster Pro)" which drives me nuts. I'd rather it be
    "SoundBlaster Pro (Speakers)" but I guess that's developers vs users. Windows uses the FriendlyName in
    its own UIs, like Volume Control, etc. */
    let mut devid: PWSTR = ptr::null_mut();
    let mut props: *mut IPropertyStore = ptr::null_mut();
    let mut info = MMDeviceInfo {
        utf8dev: None,
        // SAFETY: plain data; all zeroes is valid (SDL_zerop(fmt)).
        fmt: unsafe { std::mem::zeroed() },
        guid,
        unique_id: None,
    };
    // SAFETY (the COM calls below): `device` is live; the property store
    // and the PROPVARIANTs are ours, cleared and released once.
    unsafe {
        if succeeded((IMMDevice::vtbl(device).open_property_store)(
            device, STGM_READ, &mut props,
        )) {
            let ps = IPropertyStore::vtbl(props);
            let mut var: PROPVARIANT = std::mem::zeroed(); // PropVariantInit(&var)
            if succeeded((ps.get_value)(
                props,
                &SDL_PKEY_DEVICE_FRIENDLYNAME,
                &mut var,
            )) {
                info.utf8dev = string_to_utf8(var.Anonymous.Anonymous.Anonymous.pwszVal);
            }
            PropVariantClear(&mut var);
            if succeeded((ps.get_value)(
                props,
                &SDL_PKEY_AUDIOENGINE_DEVICEFORMAT,
                &mut var,
            )) {
                let blob = var.Anonymous.Anonymous.Anonymous.blob;
                let n = (blob.cbSize as usize).min(size_of::<WAVEFORMATEXTENSIBLE>());
                if !blob.pBlobData.is_null() {
                    ptr::copy_nonoverlapping(
                        blob.pBlobData,
                        ptr::from_mut(&mut info.fmt).cast::<u8>(),
                        n,
                    );
                }
            }
            PropVariantClear(&mut var);
            if succeeded((ps.get_value)(
                props,
                &SDL_PKEY_AUDIOENDPOINT_GUID,
                &mut var,
            )) {
                let _ = CLSIDFromString(var.Anonymous.Anonymous.Anonymous.pwszVal, &mut info.guid);
            }

            PropVariantClear(&mut var);
            if succeeded((ps.get_value)(
                props,
                &SDL_PKEY_AUDIOENDPOINT_STABLEID,
                &mut var,
            )) && !var.Anonymous.Anonymous.Anonymous.pwszVal.is_null()
            {
                // this was introduced in Windows 11, with stronger promises than IMMDevice_GetId().
                info.unique_id = string_to_utf8(var.Anonymous.Anonymous.Anonymous.pwszVal);
            } else if succeeded((IMMDevice::vtbl(device).get_id)(device, &mut devid)) {
                info.unique_id = string_to_utf8(devid);
                CoTaskMemFree(devid.cast());
            }

            PropVariantClear(&mut var);
            release(props);
        }
    }
    info
}

/// Translation of `SDL_IMMDevice_FreeDeviceHandle()`, by the device's
/// handle (WASAPI calls it from its management thread).
pub(crate) fn free_device_handle(handle: usize) {
    with_handles(|h| h.remove(&handle));
}

/// Translation of `SDL_IMMDevice_Add()`.
#[allow(clippy::too_many_arguments)]
fn sdl_immdevice_add(
    recording: bool,
    devname: Option<&str>,
    fmt: &WAVEFORMATEXTENSIBLE,
    devid: &[u16],
    dsoundguid: &GUID,
    unique_id: Option<&str>,
    force_format: AudioFormat,
    supports_recording_playback_devices: bool,
) -> Option<Arc<PhysicalDevice>> {
    /* You can have multiple endpoints on a device that are mutually exclusive ("Speakers" vs "Line Out" or whatever).
    In a perfect world, things that are unplugged won't be in this collection. The only gotcha is probably for
    phones and tablets, where you might have an internal speaker and a headphone jack and expect both to be
    available and switch automatically. (!!! FIXME...?) */

    let devname = devname?;

    // see if we already have this one first.
    let mut device = sdl_immdevice_find_by_dev_id(devid);
    if let Some(d) = &device {
        if d.is_zombie() {
            // whoa, it came back! This can happen if you unplug and replug USB headphones while we're still keeping the SDL object alive.
            // Kill this device's IMMDevice id; the device will go away when the app closes it, or maybe a new default device is chosen
            // (possibly this reconnected device), so we just want to make sure IMMDevice doesn't try to find the old device by the existing ID string.
            let handle = d.handle;
            with_handles(|h| {
                if let Some(data) = h.get_mut(&handle) {
                    data.immdevice_id = None;
                }
            });
            device = None; // add a new device, below.
        }
    }

    if device.is_none() {
        // handle is freed by SDL_IMMDevice_FreeDeviceHandle!
        let handle = NEXT_HANDLE.fetch_add(1, Ordering::Relaxed);
        with_handles(|h| {
            h.insert(
                handle,
                HandleData {
                    immdevice_id: Some(devid.to_vec()),
                    directsound_guid: *dsoundguid,
                },
            )
        });

        // SAFETY: a zeroed WAVEFORMATEXTENSIBLE or one copied from the
        // endpoint's format property: a whole extensible struct either way.
        let wave_format = unsafe { super::wave_format_ex_to_sdl_format(&fmt.Format) };
        let spec = AudioSpec::new(
            if force_format != AudioFormat::UNKNOWN {
                force_format
            } else {
                wave_format.unwrap_or(AudioFormat::UNKNOWN)
            },
            fmt.Format.nChannels as u8 as i32,
            fmt.Format.nSamplesPerSec as i32,
        );

        device = add_audio_device(recording, devname, unique_id, Some(&spec), handle);

        if !recording && supports_recording_playback_devices {
            // handle is freed by SDL_IMMDevice_FreeDeviceHandle!
            let recording_handle = NEXT_HANDLE.fetch_add(1, Ordering::Relaxed);
            with_handles(|h| {
                h.insert(
                    recording_handle,
                    HandleData {
                        immdevice_id: Some(devid.to_vec()),
                        directsound_guid: *dsoundguid,
                    },
                )
            });

            if add_audio_device(true, devname, unique_id, Some(&spec), recording_handle).is_none() {
                with_handles(|h| h.remove(&recording_handle));
            }
        }

        if device.is_none() {
            with_handles(|h| h.remove(&handle));
        }
    }

    device
}

/* We need a COM subclass of IMMNotificationClient for hotplug support, which is
easy in C++, but we have to tapdance more to make work in C.
Thanks to this page for coaching on how to make this work:
  https://www.codeproject.com/Articles/13601/COM-in-plain-C */

/// Translation of `SDLMMNotificationClient`.
#[repr(C)]
struct SdlMmNotificationClient {
    lp_vtbl: *const IMMNotificationClientVtbl,
    refcount: AtomicI32,
    force_format: AtomicU32,
    supports_recording_playback_devices: AtomicBool,
}

// SAFETY: the vtable pointer is to a static and never changes; the rest is atomic.
unsafe impl Sync for SdlMmNotificationClient {}

/// Translation of `SDLMMNotificationClient_QueryInterface()`.
unsafe extern "system" fn sdlmm_notification_client_query_interface(
    client: *mut IMMNotificationClient,
    iid: *const GUID,
    ppv: *mut *mut c_void,
) -> HRESULT {
    // SAFETY: COM passes a valid IID and out-pointer; `client` is ours.
    unsafe {
        if is_equal_guid(&*iid, &IID_IUNKNOWN)
            || is_equal_guid(&*iid, &SDL_IID_IMMNOTIFICATIONCLIENT)
        {
            *ppv = client.cast();
            ((*(*client).vtbl).add_ref)(client);
            return S_OK;
        }

        *ppv = ptr::null_mut();
    }
    E_NOINTERFACE
}

/// The notification client behind an `IMMNotificationClient` pointer.
///
/// # Safety
///
/// `iclient` must be [`NOTIFICATION_CLIENT`] (the only one handed out).
unsafe fn sdl_client<'a>(iclient: *mut IMMNotificationClient) -> &'a SdlMmNotificationClient {
    // SAFETY: the caller's contract; it's a static.
    unsafe { &*iclient.cast::<SdlMmNotificationClient>() }
}

/// Translation of `SDLMMNotificationClient_AddRef()`.
unsafe extern "system" fn sdlmm_notification_client_add_ref(
    iclient: *mut IMMNotificationClient,
) -> u32 {
    // SAFETY: only our client is handed to COM.
    let client = unsafe { sdl_client(iclient) };
    (client.refcount.fetch_add(1, Ordering::AcqRel) + 1) as u32
}

/// Translation of `SDLMMNotificationClient_Release()`.
unsafe extern "system" fn sdlmm_notification_client_release(
    iclient: *mut IMMNotificationClient,
) -> u32 {
    // client is a static object; we don't ever free it.
    // SAFETY: only our client is handed to COM.
    let client = unsafe { sdl_client(iclient) };
    // FIXME (upstream): SDL_AtomicDecRef() returns whether the count hit
    // zero, not the count, so releasing while others hold references zeroes
    // the count, and both paths return 0.
    let rc = u32::from(client.refcount.fetch_sub(1, Ordering::AcqRel) == 1); // SDL_AtomicDecRef()
    if rc == 0 {
        client.refcount.store(0, Ordering::Release); // uhh...
        return 0;
    }
    rc - 1
}

// These are the entry points called when WASAPI device endpoints change.

/// Translation of `SDLMMNotificationClient_OnDefaultDeviceChanged()`.
unsafe extern "system" fn sdlmm_notification_client_on_default_device_changed(
    _iclient: *mut IMMNotificationClient,
    _flow: EDataFlow,
    role: ERole,
    pwstr_device_id: PCWSTR,
) -> HRESULT {
    if role == SDL_IMMDEVICE_ROLE {
        // FIXME (upstream): the device id is NULL when there's no default
        // device left; SDL_IMMDevice_FindByDevID() then finds nothing.
        let device = if pwstr_device_id.is_null() {
            None
        } else {
            // SAFETY: a NUL-terminated id from MMDevice.
            sdl_immdevice_find_by_dev_id(&unsafe { wcsdup(pwstr_device_id) })
        };
        if let Some(cb) = callbacks() {
            (cb.default_audio_device_changed)(device);
        }
    }
    S_OK
}

/// Translation of `SDLMMNotificationClient_OnDeviceAdded()`.
unsafe extern "system" fn sdlmm_notification_client_on_device_added(
    _iclient: *mut IMMNotificationClient,
    _pwstr_device_id: PCWSTR,
) -> HRESULT {
    /* we ignore this; devices added here then progress to ACTIVE, if appropriate, in
    OnDeviceStateChange, making that a better place to deal with device adds. More
    importantly: the first time you plug in a USB audio device, this callback will
    fire, but when you unplug it, it isn't removed (it's state changes to NOTPRESENT).
    Plugging it back in won't fire this callback again. */
    S_OK
}

/// Translation of `SDLMMNotificationClient_OnDeviceRemoved()`.
unsafe extern "system" fn sdlmm_notification_client_on_device_removed(
    _iclient: *mut IMMNotificationClient,
    _pwstr_device_id: PCWSTR,
) -> HRESULT {
    S_OK // See notes in OnDeviceAdded handler about why we ignore this.
}

/// Translation of `SDLMMNotificationClient_OnDeviceStateChanged()`.
unsafe extern "system" fn sdlmm_notification_client_on_device_state_changed(
    iclient: *mut IMMNotificationClient,
    pwstr_device_id: PCWSTR,
    dw_new_state: u32,
) -> HRESULT {
    // SAFETY: only our client is handed to COM.
    let client = unsafe { sdl_client(iclient) };
    let mut device: *mut IMMDevice = ptr::null_mut();
    let enumerator = enumerator();
    if enumerator.is_null() || pwstr_device_id.is_null() {
        return S_OK; // (upstream assumes neither is NULL; MMDevice only calls this while registered)
    }

    // SAFETY (the COM calls below): the enumerator is live while
    // registered; the device and endpoint are ours, released once.
    unsafe {
        if succeeded((IMMDeviceEnumerator::vtbl(enumerator).get_device)(
            enumerator,
            pwstr_device_id,
            &mut device,
        )) {
            let mut endpoint: *mut IMMEndpoint = ptr::null_mut();
            if succeeded(query_interface(device, &SDL_IID_IMMENDPOINT, &mut endpoint)) {
                let mut flow: EDataFlow = 0;
                if succeeded((IMMEndpoint::vtbl(endpoint).get_data_flow)(
                    endpoint, &mut flow,
                )) {
                    let recording = flow == eCapture;
                    let devid = wcsdup(pwstr_device_id);
                    if dw_new_state == DEVICE_STATE_ACTIVE {
                        // FIXME (upstream): `dsoundguid` isn't initialized here, so an
                        // endpoint without a GUID property gets stack garbage; zeroed here.
                        let info = get_mm_device_info(device, zero_guid());
                        if info.utf8dev.is_some() {
                            sdl_immdevice_add(
                                recording,
                                info.utf8dev.as_deref(),
                                &info.fmt,
                                &devid,
                                &info.guid,
                                info.unique_id.as_deref(),
                                AudioFormat(client.force_format.load(Ordering::Acquire)),
                                client
                                    .supports_recording_playback_devices
                                    .load(Ordering::Acquire),
                            );
                        }
                    } else if let Some(cb) = callbacks() {
                        (cb.audio_device_disconnected)(sdl_immdevice_find_by_dev_id(&devid));
                    }
                }
                release(endpoint);
            }
            release(device);
        }
    }

    S_OK
}

/// Translation of `SDLMMNotificationClient_OnPropertyValueChanged()`.
unsafe extern "system" fn sdlmm_notification_client_on_property_value_changed(
    _client: *mut IMMNotificationClient,
    _pwstr_device_id: PCWSTR,
    _key: PROPERTYKEY,
) -> HRESULT {
    S_OK // we don't care about these.
}

static NOTIFICATION_CLIENT_VTBL: IMMNotificationClientVtbl = IMMNotificationClientVtbl {
    query_interface: sdlmm_notification_client_query_interface,
    add_ref: sdlmm_notification_client_add_ref,
    release: sdlmm_notification_client_release,
    on_device_state_changed: sdlmm_notification_client_on_device_state_changed,
    on_device_added: sdlmm_notification_client_on_device_added,
    on_device_removed: sdlmm_notification_client_on_device_removed,
    on_default_device_changed: sdlmm_notification_client_on_default_device_changed,
    on_property_value_changed: sdlmm_notification_client_on_property_value_changed,
};

static NOTIFICATION_CLIENT: SdlMmNotificationClient = SdlMmNotificationClient {
    lp_vtbl: &NOTIFICATION_CLIENT_VTBL,
    refcount: AtomicI32::new(1),
    force_format: AtomicU32::new(AudioFormat::UNKNOWN.0),
    supports_recording_playback_devices: AtomicBool::new(false),
};

fn notification_client() -> *mut IMMNotificationClient {
    ptr::from_ref(&NOTIFICATION_CLIENT).cast_mut().cast()
}

const fn zero_guid() -> GUID {
    GUID {
        data1: 0,
        data2: 0,
        data3: 0,
        data4: [0; 8],
    }
}

/// `SDL_AudioDeviceDisconnected()` as an IMMDevice callback.
fn default_disconnected(device: Option<Arc<PhysicalDevice>>) {
    if let Some(device) = device {
        audio_device_disconnected(&device);
    }
}

/// `SDL_DefaultAudioDeviceChanged()` as an IMMDevice callback.
fn default_changed(device: Option<Arc<PhysicalDevice>>) {
    if let Some(device) = device {
        default_audio_device_changed(&device);
    }
}

/// Translation of `SDL_IMMDevice_Init()`.
pub(crate) fn init(callbacks: Option<ImmDeviceCallbacks>) -> Result<()> {
    // just skip the discussion with COM here.
    if !super::is_windows_vista_or_greater() {
        return Err(Error::new(
            "IMMDevice support requires Windows Vista or later",
        ));
    }

    if failed(co_initialize()) {
        return Err(Error::new("IMMDevice: CoInitialize() failed"));
    }

    let mut enumerator: *mut IMMDeviceEnumerator = ptr::null_mut();
    // SAFETY: valid CLSID/IID and out-pointer.
    let ret = unsafe {
        CoCreateInstance(
            &SDL_CLSID_MMDEVICEENUMERATOR,
            ptr::null_mut(),
            CLSCTX_INPROC_SERVER,
            &SDL_IID_IMMDEVICEENUMERATOR,
            ptr::from_mut(&mut enumerator).cast(),
        )
    };
    if failed(ret) {
        co_uninitialize();
        return Err(error_from_hresult(
            Some("IMMDevice CoCreateInstance(MMDeviceEnumerator)"),
            ret,
        ));
    }
    ENUMERATOR.store(enumerator, Ordering::Release);

    // (missing callbacks fall back to SDL_AudioDeviceDisconnected() and SDL_DefaultAudioDeviceChanged())
    *IMMCALLBACKS.lock().unwrap_or_else(|e| e.into_inner()) =
        Some(callbacks.unwrap_or(ImmDeviceCallbacks {
            audio_device_disconnected: default_disconnected,
            default_audio_device_changed: default_changed,
        }));

    Ok(())
}

/// Translation of `SDL_IMMDevice_Quit()`.
pub(crate) fn quit() {
    let enumerator = ENUMERATOR.swap(ptr::null_mut(), Ordering::AcqRel);
    if !enumerator.is_null() {
        // SAFETY: our live enumerator, released once.
        unsafe {
            (IMMDeviceEnumerator::vtbl(enumerator).unregister_endpoint_notification_callback)(
                enumerator,
                notification_client(),
            );
            release(enumerator);
        }
    }

    *IMMCALLBACKS.lock().unwrap_or_else(|e| e.into_inner()) = None;

    co_uninitialize();
}

/// Translation of `SDL_IMMDevice_Get()`: the endpoint, which the caller releases.
pub(crate) fn get(device: &PhysicalDevice, _recording: bool) -> Result<*mut IMMDevice> {
    let timeout = crate::timer::ticks() + Duration::from_millis(8000); // intel's audio drivers can fail for up to EIGHT SECONDS after a device is connected or we wake from sleep.

    let devid = get_dev_id(device);
    crate::sdl_assert!(devid.is_some());
    let devid = devid.unwrap_or_else(|| vec![0]);
    let enumerator = enumerator();
    if enumerator.is_null() {
        return Err(Error::new("IMMDevice isn't initialized"));
    }

    let mut immdevice: *mut IMMDevice = ptr::null_mut();
    let ret = loop {
        // SAFETY: the enumerator is live; the id is NUL-terminated.
        let ret = unsafe {
            (IMMDeviceEnumerator::vtbl(enumerator).get_device)(
                enumerator,
                devid.as_ptr(),
                &mut immdevice,
            )
        };
        if ret != E_NOTFOUND {
            break ret;
        }
        let now = crate::timer::ticks();
        if timeout > now {
            let ticksleft = timeout - now;
            crate::timer::delay(ticksleft.min(Duration::from_millis(300))); // wait awhile and try again.
            continue;
        }
        break ret;
    };

    if !succeeded(ret) {
        return Err(error_from_hresult(
            Some("WASAPI can't find requested audio endpoint"),
            ret,
        ));
    }
    Ok(immdevice)
}

/// Translation of `SDL_IMMDevice_GetIsCapture()`.
///
/// # Safety
///
/// `device` must be a live `IMMDevice`.
pub(crate) unsafe fn get_is_capture(device: *mut IMMDevice) -> bool {
    let mut iscapture = false;
    let mut endpoint: *mut IMMEndpoint = ptr::null_mut();
    // SAFETY: `device` is live; the endpoint is ours.
    unsafe {
        if succeeded(query_interface(device, &SDL_IID_IMMENDPOINT, &mut endpoint)) {
            let mut flow: EDataFlow = 0;

            if succeeded((IMMEndpoint::vtbl(endpoint).get_data_flow)(
                endpoint, &mut flow,
            )) {
                iscapture = flow == eCapture;
            }
        }

        // FIXME (upstream): this releases `endpoint` even when the query
        // failed and it's NULL; here it's only released when it was got.
        if !endpoint.is_null() {
            release(endpoint);
        }
    }
    iscapture
}

/// Translation of `EnumerateEndpointsForFlow()`.
fn enumerate_endpoints_for_flow(
    recording: bool,
    want_default: bool,
    force_format: AudioFormat,
    supports_recording_playback_devices: bool,
) -> Option<Arc<PhysicalDevice>> {
    /* Note that WASAPI separates "adapter devices" from "audio endpoint devices"
    ...one adapter device ("SoundBlaster Pro") might have multiple endpoint devices ("Speakers", "Line-Out"). */

    let enumerator = enumerator();
    if enumerator.is_null() {
        return None;
    }
    let mut default_device = None;
    let mut collection: *mut IMMDeviceCollection = ptr::null_mut();
    // SAFETY (the COM calls below): the enumerator is live; every object
    // we get is released once; CoTaskMemFree() frees the ids it hands out.
    unsafe {
        let ev = IMMDeviceEnumerator::vtbl(enumerator);
        if failed((ev.enum_audio_endpoints)(
            enumerator,
            if recording { eCapture } else { eRender },
            DEVICE_STATE_ACTIVE,
            &mut collection,
        )) {
            return None;
        }

        let mut total: u32 = 0;
        if failed((IMMDeviceCollection::vtbl(collection).get_count)(
            collection, &mut total,
        )) {
            release(collection);
            return None;
        }

        let mut default_devid: Option<Vec<u16>> = None;
        if want_default {
            let mut default_immdevice: *mut IMMDevice = ptr::null_mut();
            let dataflow = if recording { eCapture } else { eRender };
            if succeeded((ev.get_default_audio_endpoint)(
                enumerator,
                dataflow,
                SDL_IMMDEVICE_ROLE,
                &mut default_immdevice,
            )) {
                let mut devid: PWSTR = ptr::null_mut();
                if succeeded((IMMDevice::vtbl(default_immdevice).get_id)(
                    default_immdevice,
                    &mut devid,
                )) {
                    default_devid = Some(wcsdup(devid)); // if this fails, oh well.
                    CoTaskMemFree(devid.cast());
                }
                release(default_immdevice);
            }
        }

        for i in 0..total {
            let mut immdevice: *mut IMMDevice = ptr::null_mut();
            if succeeded((IMMDeviceCollection::vtbl(collection).item)(
                collection,
                i,
                &mut immdevice,
            )) {
                let mut devid: PWSTR = ptr::null_mut();
                if succeeded((IMMDevice::vtbl(immdevice).get_id)(immdevice, &mut devid)) {
                    let info = get_mm_device_info(immdevice, zero_guid());
                    if info.utf8dev.is_some() {
                        let id = wcsdup(devid);
                        let sdldevice = sdl_immdevice_add(
                            recording,
                            info.utf8dev.as_deref(),
                            &info.fmt,
                            &id,
                            &info.guid,
                            info.unique_id.as_deref(),
                            force_format,
                            supports_recording_playback_devices,
                        );
                        if want_default && default_devid.as_deref().is_some_and(|d| wcs_eq(d, &id))
                        {
                            default_device = sdldevice;
                        }
                    }
                    CoTaskMemFree(devid.cast());
                }
                release(immdevice);
            }
        }

        release(collection);
    }
    default_device
}

/// Translation of `SDL_IMMDevice_EnumerateEndpoints()`: the default playback
/// and recording devices, if there are any.
pub(crate) fn enumerate_endpoints(
    force_format: AudioFormat,
    supports_recording_playback_devices: bool,
) -> (Option<Arc<PhysicalDevice>>, Option<Arc<PhysicalDevice>>) {
    let default_playback = enumerate_endpoints_for_flow(
        false,
        true,
        force_format,
        supports_recording_playback_devices,
    );
    let default_recording = enumerate_endpoints_for_flow(
        true,
        true,
        force_format,
        supports_recording_playback_devices,
    );

    NOTIFICATION_CLIENT
        .force_format
        .store(force_format.0, Ordering::Release);
    NOTIFICATION_CLIENT
        .supports_recording_playback_devices
        .store(supports_recording_playback_devices, Ordering::Release);

    // if this fails, we just won't get hotplug events. Carry on anyhow.
    let enumerator = enumerator();
    if !enumerator.is_null() {
        // SAFETY: the enumerator is live; the client is a static.
        unsafe {
            (IMMDeviceEnumerator::vtbl(enumerator).register_endpoint_notification_callback)(
                enumerator,
                notification_client(),
            )
        };
    }

    (default_playback, default_recording)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notification_client_com_basics() {
        let _l = crate::test_support::test_lock();
        let client = notification_client();
        let mut out: *mut c_void = ptr::null_mut();
        // SAFETY: our static client and valid IIDs and out-pointers.
        unsafe {
            assert_eq!(
                sdlmm_notification_client_query_interface(
                    client,
                    &SDL_IID_IMMNOTIFICATIONCLIENT,
                    &mut out
                ),
                S_OK
            );
            assert_eq!(out, client.cast());
            assert_eq!(
                sdlmm_notification_client_query_interface(client, &IID_IUNKNOWN, &mut out),
                S_OK
            );
            assert_eq!(
                sdlmm_notification_client_query_interface(client, &SDL_IID_IMMENDPOINT, &mut out),
                E_NOINTERFACE
            );
            assert!(out.is_null());
            // (two references taken above)
            assert_eq!(NOTIFICATION_CLIENT.refcount.load(Ordering::Acquire), 3);
            // Upstream's release zeroes the count when it isn't the last one...
            assert_eq!(sdlmm_notification_client_release(client), 0);
            assert_eq!(NOTIFICATION_CLIENT.refcount.load(Ordering::Acquire), 0);
            assert_eq!(sdlmm_notification_client_add_ref(client), 1);
            // ...and returns 0 when it is.
            assert_eq!(sdlmm_notification_client_release(client), 0);
            assert_eq!(NOTIFICATION_CLIENT.refcount.load(Ordering::Acquire), 0);
            NOTIFICATION_CLIENT.refcount.store(1, Ordering::Release);
        }
    }

    #[test]
    fn wide_string_helpers() {
        let a: Vec<u16> = "{0.0.0.00000000}.{abc}\0".encode_utf16().collect();
        let b: Vec<u16> = "{0.0.0.00000000}.{abc}".encode_utf16().collect();
        assert!(wcs_eq(&a, &b));
        assert!(!wcs_eq(&a, &b[..5]));
        // SAFETY: NUL-terminated.
        unsafe {
            assert_eq!(wcslen(a.as_ptr()), a.len() - 1);
            assert_eq!(wcsdup(a.as_ptr()), a);
            assert_eq!(
                string_to_utf8(a.as_ptr()).unwrap(),
                "{0.0.0.00000000}.{abc}"
            );
            assert_eq!(string_to_utf8(ptr::null()), None);
        }
        assert_eq!(E_NOTFOUND as u32, 0x80070490);
    }

    #[test]
    fn vtable_layouts() {
        let p = size_of::<usize>();
        assert_eq!(size_of::<IMMDeviceEnumeratorVtbl>(), 8 * p);
        assert_eq!(size_of::<IMMDeviceVtbl>(), 7 * p);
        assert_eq!(size_of::<IMMDeviceCollectionVtbl>(), 5 * p);
        assert_eq!(size_of::<IMMEndpointVtbl>(), 4 * p);
        assert_eq!(size_of::<IPropertyStoreVtbl>(), 8 * p);
        assert_eq!(size_of::<IMMNotificationClientVtbl>(), 8 * p);
    }
}
