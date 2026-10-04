// Rust translation of src/camera/mediafoundation/SDL_camera_mediafoundation.c
// from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Windows Media Foundation camera driver: video capture devices from
//! `MFEnumDeviceSources()`, the native media types of their selected
//! streams collected into specs, and one `IMFSourceReader` per opened
//! camera, read synchronously on the device thread. mf.dll, mfplat.dll and
//! mfreadwrite.dll are loaded at run time, as upstream does; the COM
//! interfaces SDL calls are declared here (only the vtable slots it uses
//! are typed, the rest are padding).
//!
//! Upstream builds without `KEEP_ACQUIRED_BUFFERS_LOCKED`, copying every
//! sample into a frame of its own, and so does this (into the frame's
//! `Vec`); the sample is released as soon as it's copied.

use std::ffi::c_void;
use std::ptr;
use std::sync::{Arc, Mutex, MutexGuard};

use windows_sys::core::{GUID, HRESULT};
use windows_sys::Win32::System::Com::CoTaskMemFree;

use super::{
    add_camera, camera_permission_outcome, find_physical_camera_by_callback, AcquiredFrame,
    CameraBackend, CameraBootStrap, CameraDevice, CameraDriverImpl, CameraFrameResult,
    CameraPosition, CameraSpec,
};
use crate::core::windows::com::{ComPtr, IUnknownVtbl};
use crate::core::windows::{error_from_hresult, utf8_to_wide, wide_to_utf8};
use crate::error::{Error, Result};
use crate::loadso::SharedObject;
use crate::video::{
    define_colorspace, ChromaLocation, ColorPrimaries, ColorRange, ColorType, Colorspace,
    MatrixCoefficients, PixelFormat, TransferCharacteristics,
};

// --- GUIDs ---

const IID_IMFMEDIASOURCE: GUID = GUID::from_u128(0x279a808d_aec7_40c8_9c6b_a6b492c78a66);
const IID_IMF2DBUFFER: GUID = GUID::from_u128(0x7dc9d5f9_9ed9_44ec_9bbf_0600bb589fbb);
const IID_IMF2DBUFFER2: GUID = GUID::from_u128(0x33ae5ea6_4316_436f_8ddd_d73d22f829ec);
const MF_MT_DEFAULT_STRIDE: GUID = GUID::from_u128(0x644b4e48_1e02_4516_b0eb_c01ca9d49ac6);
const MF_MT_MAJOR_TYPE: GUID = GUID::from_u128(0x48eba18e_f8c9_4687_bf11_0a74c9f96a8f);
const MF_MT_SUBTYPE: GUID = GUID::from_u128(0xf7e34c9a_42e8_4714_b74b_cb29d72c35e5);
const MF_MT_VIDEO_NOMINAL_RANGE: GUID = GUID::from_u128(0xc21b8ee5_b956_4071_8daf_325edf5cab11);
const MF_MT_VIDEO_PRIMARIES: GUID = GUID::from_u128(0xdbfbe4d7_0740_4ee0_8192_850ab0e21935);
const MF_MT_TRANSFER_FUNCTION: GUID = GUID::from_u128(0x5fb0fce9_be5c_4935_a811_ec838f8eed93);
const MF_MT_YUV_MATRIX: GUID = GUID::from_u128(0x3e23d450_2c75_4d25_a00e_b91670d12327);
const MF_MT_VIDEO_CHROMA_SITING: GUID = GUID::from_u128(0x65df2370_c773_4c33_aa64_843e068efb0c);
const MF_MT_FRAME_SIZE: GUID = GUID::from_u128(0x1652c33d_d6b2_4012_b834_72030849a37d);
const MF_MT_FRAME_RATE: GUID = GUID::from_u128(0xc459a2e8_3d2c_4e44_b132_fee5156c7bb0);
const MFMEDIATYPE_VIDEO: GUID = GUID::from_u128(0x73646976_0000_0010_8000_00aa00389b71);
const MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME: GUID =
    GUID::from_u128(0x60d0e559_52f8_4fa2_bbce_acdb34a8ec01);
const MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE: GUID =
    GUID::from_u128(0xc60ac5fe_252a_478f_a0ef_bc8fa5f7cad3);
const MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK: GUID =
    GUID::from_u128(0x58f0aad8_22bf_4f8a_bb3d_d2c4978c6e2f);
const MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID: GUID =
    GUID::from_u128(0x8ac3587a_4ae7_42d8_99e0_0a6013eef90f);

/// `FCC()` of a multi-character constant: the four characters in memory
/// order.
const fn fcc(ch4: &[u8; 4]) -> u32 {
    u32::from_le_bytes(*ch4)
}

/// `SDL_DEFINE_MEDIATYPE_GUID()`: a media subtype GUID from its format
/// code.
const fn mediatype_guid(fmt: u32) -> GUID {
    GUID {
        data1: fmt,
        data2: 0x0000,
        data3: 0x0010,
        data4: [0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71],
    }
}

const MFVIDEOFORMAT_RGB555: GUID = mediatype_guid(24);
const MFVIDEOFORMAT_RGB565: GUID = mediatype_guid(23);
const MFVIDEOFORMAT_RGB24: GUID = mediatype_guid(20);
const MFVIDEOFORMAT_RGB32: GUID = mediatype_guid(22);
const MFVIDEOFORMAT_ARGB32: GUID = mediatype_guid(21);
const MFVIDEOFORMAT_A2R10G10B10: GUID = mediatype_guid(31);
const MFVIDEOFORMAT_YV12: GUID = mediatype_guid(fcc(b"YV12"));
const MFVIDEOFORMAT_IYUV: GUID = mediatype_guid(fcc(b"IYUV"));
const MFVIDEOFORMAT_YUY2: GUID = mediatype_guid(fcc(b"YUY2"));
const MFVIDEOFORMAT_UYVY: GUID = mediatype_guid(fcc(b"UYVY"));
const MFVIDEOFORMAT_YVYU: GUID = mediatype_guid(fcc(b"YVYU"));
const MFVIDEOFORMAT_NV12: GUID = mediatype_guid(fcc(b"NV12"));
const MFVIDEOFORMAT_NV21: GUID = mediatype_guid(fcc(b"NV21"));
const MFVIDEOFORMAT_MJPG: GUID = mediatype_guid(fcc(b"MJPG"));

// --- the other constants of the Media Foundation headers SDL uses ---

/// `MF_VERSION` (`MF_SDK_VERSION << 16 | MF_API_VERSION`).
const MF_VERSION: u32 = 0x0002_0070;
const MFSTARTUP_LITE: u32 = 1;
/// `MF_SOURCE_READER_FIRST_VIDEO_STREAM`.
const MF_SOURCE_READER_FIRST_VIDEO_STREAM: u32 = 0xffff_fffc;
const MF_SOURCE_READERF_ERROR: u32 = 0x1;
const MF_SOURCE_READERF_ENDOFSTREAM: u32 = 0x2;
/// `MF2DBuffer_LockFlags_Read`.
const MF2DBUFFER_LOCKFLAGS_READ: u32 = 1;
/// `E_POINTER`.
const E_POINTER: HRESULT = 0x8000_4003_u32 as HRESULT;

// enum _MFNominalRange
const MF_NOMINAL_RANGE_0_255: u32 = 1;
const MF_NOMINAL_RANGE_16_235: u32 = 2;

// enum _MFVideoPrimaries
const MF_VIDEO_PRIMARIES_BT709: u32 = 2;
const MF_VIDEO_PRIMARIES_BT470_2_SYSM: u32 = 3;
const MF_VIDEO_PRIMARIES_BT470_2_SYSBG: u32 = 4;
const MF_VIDEO_PRIMARIES_SMPTE170M: u32 = 5;
const MF_VIDEO_PRIMARIES_SMPTE240M: u32 = 6;
const MF_VIDEO_PRIMARIES_EBU3213: u32 = 7;
const MF_VIDEO_PRIMARIES_BT2020: u32 = 9;
const MF_VIDEO_PRIMARIES_XYZ: u32 = 10;
const MF_VIDEO_PRIMARIES_DCI_P3: u32 = 11;

// enum _MFVideoTransferFunction
const MF_VIDEO_TRANS_FUNC_10: u32 = 1;
const MF_VIDEO_TRANS_FUNC_22: u32 = 4;
const MF_VIDEO_TRANS_FUNC_709: u32 = 5;
const MF_VIDEO_TRANS_FUNC_240M: u32 = 6;
const MF_VIDEO_TRANS_FUNC_SRGB: u32 = 7;
const MF_VIDEO_TRANS_FUNC_28: u32 = 8;
const MF_VIDEO_TRANS_FUNC_LOG_100: u32 = 9;
const MF_VIDEO_TRANS_FUNC_2084: u32 = 15;
const MF_VIDEO_TRANS_FUNC_HLG: u32 = 16;

// enum _MFVideoTransferMatrix
const MF_VIDEO_TRANSFER_MATRIX_BT709: u32 = 1;
const MF_VIDEO_TRANSFER_MATRIX_BT601: u32 = 2;
const MF_VIDEO_TRANSFER_MATRIX_SMPTE240M: u32 = 3;
const MF_VIDEO_TRANSFER_MATRIX_BT2020_10: u32 = 4;

// enum _MFVideoChromaSubsampling
const MF_VIDEO_CHROMA_SUBSAMPLING_MPEG2: u32 = 0x4 | 0x1;
const MF_VIDEO_CHROMA_SUBSAMPLING_MPEG1: u32 = 0x1;
const MF_VIDEO_CHROMA_SUBSAMPLING_DV_PAL: u32 = 0x4 | 0x2;

// --- the COM interfaces SDL calls ---

/// `HRESULT` to `Result` (`SUCCEEDED()`).
fn check_hr(hr: HRESULT) -> std::result::Result<(), HRESULT> {
    if hr < 0 {
        Err(hr)
    } else {
        Ok(())
    }
}

/// An unused vtable slot.
type Slot = usize;

/// `IMFAttributesVtbl`, which `IMFMediaType`'s, `IMFActivate`'s,
/// `IMFPresentationDescriptor`'s, `IMFStreamDescriptor`'s and
/// `IMFSample`'s start with. (SDL calls no `IMFMediaType` methods of its
/// own, so a media type is a `ComPtr<IMFAttributesVtbl>` too.)
#[repr(C)]
struct IMFAttributesVtbl {
    base: IUnknownVtbl,
    /// GetItem, GetItemType, CompareItem, Compare
    _get_item: [Slot; 4],
    get_uint32: unsafe extern "system" fn(*mut c_void, *const GUID, *mut u32) -> HRESULT,
    get_uint64: unsafe extern "system" fn(*mut c_void, *const GUID, *mut u64) -> HRESULT,
    _get_double: Slot,
    get_guid: unsafe extern "system" fn(*mut c_void, *const GUID, *mut GUID) -> HRESULT,
    /// GetStringLength, GetString
    _get_string: [Slot; 2],
    get_allocated_string:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut u16, *mut u32) -> HRESULT,
    /// GetBlobSize, GetBlob, GetAllocatedBlob, GetUnknown, SetItem,
    /// DeleteItem, DeleteAllItems
    _get_blob: [Slot; 7],
    set_uint32: unsafe extern "system" fn(*mut c_void, *const GUID, u32) -> HRESULT,
    set_uint64: unsafe extern "system" fn(*mut c_void, *const GUID, u64) -> HRESULT,
    _set_double: Slot,
    set_guid: unsafe extern "system" fn(*mut c_void, *const GUID, *const GUID) -> HRESULT,
    set_string: unsafe extern "system" fn(*mut c_void, *const GUID, *const u16) -> HRESULT,
    /// SetBlob, SetUnknown, LockStore, UnlockStore, GetCount,
    /// GetItemByIndex, CopyAllItems
    _set_blob: [Slot; 7],
}

/// `IMFActivateVtbl`.
#[repr(C)]
struct IMFActivateVtbl {
    attributes: IMFAttributesVtbl,
    activate_object:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    shutdown_object: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    _detach_object: Slot,
}

/// `IMFMediaSourceVtbl`.
#[repr(C)]
struct IMFMediaSourceVtbl {
    base: IUnknownVtbl,
    /// IMFMediaEventGenerator's GetEvent, BeginGetEvent, EndGetEvent,
    /// QueueEvent; GetCharacteristics
    _get_event: [Slot; 5],
    create_presentation_descriptor:
        unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    /// Start, Stop, Pause
    _start: [Slot; 3],
    shutdown: unsafe extern "system" fn(*mut c_void) -> HRESULT,
}

/// `IMFPresentationDescriptorVtbl` (the part SDL uses).
#[repr(C)]
struct IMFPresentationDescriptorVtbl {
    attributes: IMFAttributesVtbl,
    get_stream_descriptor_count: unsafe extern "system" fn(*mut c_void, *mut u32) -> HRESULT,
    get_stream_descriptor_by_index:
        unsafe extern "system" fn(*mut c_void, u32, *mut i32, *mut *mut c_void) -> HRESULT,
}

/// `IMFStreamDescriptorVtbl`.
#[repr(C)]
struct IMFStreamDescriptorVtbl {
    attributes: IMFAttributesVtbl,
    _get_stream_identifier: Slot,
    get_media_type_handler: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
}

/// `IMFMediaTypeHandlerVtbl` (the part SDL uses).
#[repr(C)]
struct IMFMediaTypeHandlerVtbl {
    base: IUnknownVtbl,
    _is_media_type_supported: Slot,
    get_media_type_count: unsafe extern "system" fn(*mut c_void, *mut u32) -> HRESULT,
    get_media_type_by_index:
        unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> HRESULT,
}

/// `IMFSourceReaderVtbl` (the part SDL uses).
#[repr(C)]
struct IMFSourceReaderVtbl {
    base: IUnknownVtbl,
    /// GetStreamSelection, SetStreamSelection, GetNativeMediaType,
    /// GetCurrentMediaType
    _get_stream_selection: [Slot; 4],
    set_current_media_type:
        unsafe extern "system" fn(*mut c_void, u32, *mut u32, *mut c_void) -> HRESULT,
    _set_current_position: Slot,
    read_sample: unsafe extern "system" fn(
        *mut c_void,
        u32,
        u32,
        *mut u32,
        *mut u32,
        *mut i64,
        *mut *mut c_void,
    ) -> HRESULT,
}

/// `IMFSampleVtbl` (the part SDL uses).
#[repr(C)]
struct IMFSampleVtbl {
    attributes: IMFAttributesVtbl,
    /// GetSampleFlags, SetSampleFlags
    _get_sample_flags: [Slot; 2],
    get_sample_time: unsafe extern "system" fn(*mut c_void, *mut i64) -> HRESULT,
    /// SetSampleTime, GetSampleDuration, SetSampleDuration, GetBufferCount,
    /// GetBufferByIndex
    _set_sample_time: [Slot; 5],
    convert_to_contiguous_buffer:
        unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
}

/// `IMFMediaBufferVtbl` (the part SDL uses).
#[repr(C)]
struct IMFMediaBufferVtbl {
    base: IUnknownVtbl,
    lock: unsafe extern "system" fn(*mut c_void, *mut *mut u8, *mut u32, *mut u32) -> HRESULT,
    unlock: unsafe extern "system" fn(*mut c_void) -> HRESULT,
}

/// `IMF2DBufferVtbl`.
#[repr(C)]
struct IMF2DBufferVtbl {
    base: IUnknownVtbl,
    lock_2d: unsafe extern "system" fn(*mut c_void, *mut *mut u8, *mut i32) -> HRESULT,
    unlock_2d: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    /// GetScanline0AndPitch, IsContiguousFormat, GetContiguousLength,
    /// ContiguousCopyTo, ContiguousCopyFrom
    _get_scanline0_and_pitch: [Slot; 5],
}

/// `IMF2DBuffer2Vtbl` (the part SDL uses).
#[repr(C)]
struct IMF2DBuffer2Vtbl {
    buffer_2d: IMF2DBufferVtbl,
    lock_2d_size: unsafe extern "system" fn(
        *mut c_void,
        u32,
        *mut *mut u8,
        *mut i32,
        *mut *mut u8,
        *mut u32,
    ) -> HRESULT,
}

/// A vtable that starts with `IMFAttributes`'.
trait HasAttributes {
    fn attributes(&self) -> &IMFAttributesVtbl;
}

impl HasAttributes for IMFAttributesVtbl {
    fn attributes(&self) -> &IMFAttributesVtbl {
        self
    }
}

macro_rules! has_attributes {
    ($($vtbl:ty),*) => {
        $(impl HasAttributes for $vtbl {
            fn attributes(&self) -> &IMFAttributesVtbl {
                &self.attributes
            }
        })*
    };
}
has_attributes!(
    IMFActivateVtbl,
    IMFPresentationDescriptorVtbl,
    IMFStreamDescriptorVtbl,
    IMFSampleVtbl
);

/// An `IMFAttributes` or `IMFMediaType`.
type Attributes = ComPtr<IMFAttributesVtbl>;

/// The attribute reads (and the one write) the format mapping and
/// `GetDefaultStride()` make on a media type, so that they can be tested
/// against a fake one.
trait MediaTypeAttributes {
    /// `IMFAttributes_GetUINT32()`
    fn get_uint32(&self, key: &GUID) -> std::result::Result<u32, HRESULT>;
    /// `IMFAttributes_GetUINT64()`
    fn get_uint64(&self, key: &GUID) -> std::result::Result<u64, HRESULT>;
    /// `IMFAttributes_GetGUID()`
    fn get_guid(&self, key: &GUID) -> std::result::Result<GUID, HRESULT>;
    /// `IMFAttributes_SetUINT32()`
    fn set_uint32(&self, key: &GUID, value: u32) -> std::result::Result<(), HRESULT>;
}

impl<V: HasAttributes> MediaTypeAttributes for ComPtr<V> {
    fn get_uint32(&self, key: &GUID) -> std::result::Result<u32, HRESULT> {
        let mut value = 0;
        // SAFETY: a live object; GetUINT32 writes a UINT32.
        check_hr(unsafe {
            (self.vtbl().attributes().get_uint32)(self.as_ptr().cast(), key, &mut value)
        })?;
        Ok(value)
    }

    fn get_uint64(&self, key: &GUID) -> std::result::Result<u64, HRESULT> {
        let mut value = 0;
        // SAFETY: a live object; GetUINT64 writes a UINT64.
        check_hr(unsafe {
            (self.vtbl().attributes().get_uint64)(self.as_ptr().cast(), key, &mut value)
        })?;
        Ok(value)
    }

    fn get_guid(&self, key: &GUID) -> std::result::Result<GUID, HRESULT> {
        let mut value = GUID::from_u128(0);
        // SAFETY: a live object; GetGUID writes a GUID.
        check_hr(unsafe {
            (self.vtbl().attributes().get_guid)(self.as_ptr().cast(), key, &mut value)
        })?;
        Ok(value)
    }

    fn set_uint32(&self, key: &GUID, value: u32) -> std::result::Result<(), HRESULT> {
        // SAFETY: a live object and a valid key.
        check_hr(unsafe { (self.vtbl().attributes().set_uint32)(self.as_ptr().cast(), key, value) })
    }
}

/// The other `IMFAttributes` calls SDL makes.
trait AttributesExt {
    /// `IMFAttributes_SetUINT64()`
    fn set_uint64(&self, key: &GUID, value: u64) -> std::result::Result<(), HRESULT>;
    /// `IMFAttributes_SetGUID()`
    fn set_guid(&self, key: &GUID, value: &GUID) -> std::result::Result<(), HRESULT>;
    /// `IMFAttributes_SetString()` of a NUL-terminated UTF-16 string.
    fn set_string(&self, key: &GUID, value: &[u16]) -> std::result::Result<(), HRESULT>;
    /// `IMFAttributes_GetAllocatedString()`, converted to UTF-8 (and the
    /// string freed).
    fn get_allocated_string(&self, key: &GUID) -> std::result::Result<String, HRESULT>;
}

impl<V: HasAttributes> AttributesExt for ComPtr<V> {
    fn set_uint64(&self, key: &GUID, value: u64) -> std::result::Result<(), HRESULT> {
        // SAFETY: a live object and a valid key.
        check_hr(unsafe { (self.vtbl().attributes().set_uint64)(self.as_ptr().cast(), key, value) })
    }

    fn set_guid(&self, key: &GUID, value: &GUID) -> std::result::Result<(), HRESULT> {
        // SAFETY: a live object and valid GUIDs.
        check_hr(unsafe { (self.vtbl().attributes().set_guid)(self.as_ptr().cast(), key, value) })
    }

    fn set_string(&self, key: &GUID, value: &[u16]) -> std::result::Result<(), HRESULT> {
        assert_eq!(
            value.last(),
            Some(&0),
            "set_string() takes a NUL-terminated string"
        );
        // SAFETY: a live object, a valid key and a NUL-terminated string
        // (which the object copies).
        check_hr(unsafe {
            (self.vtbl().attributes().set_string)(self.as_ptr().cast(), key, value.as_ptr())
        })
    }

    fn get_allocated_string(&self, key: &GUID) -> std::result::Result<String, HRESULT> {
        let mut wstr: *mut u16 = ptr::null_mut();
        let mut wlen = 0u32;
        // SAFETY: a live object; on success GetAllocatedString stores a
        // CoTaskMemAlloc'd string of `wlen` characters (plus a NUL).
        check_hr(unsafe {
            (self.vtbl().attributes().get_allocated_string)(
                self.as_ptr().cast(),
                key,
                &mut wstr,
                &mut wlen,
            )
        })?;
        if wstr.is_null() {
            return Err(E_POINTER);
        }
        // SAFETY: as above; the string is freed once, after the copy.
        let utf8str = unsafe {
            let s = wide_to_utf8(std::slice::from_raw_parts(wstr, wlen as usize));
            CoTaskMemFree(wstr.cast());
            s
        };
        Ok(utf8str)
    }
}

/// Call a method that returns an interface through its last parameter.
///
/// # Safety
///
/// On success, `f` must store NULL or an owned reference to a `W`.
unsafe fn out_ptr<W>(
    f: impl FnOnce(*mut *mut c_void) -> HRESULT,
) -> std::result::Result<ComPtr<W>, HRESULT> {
    // SAFETY: the caller's contract.
    unsafe { ComPtr::from_out(|out: *mut *mut _| f(out.cast())) }
}

// --- the format table ---

/// An entry of `fmtmappings`.
struct FormatMapping {
    guid: GUID,
    format: PixelFormat,
    colorspace: Colorspace,
}

const fn mapping(guid: GUID, format: PixelFormat, colorspace: Colorspace) -> FormatMapping {
    FormatMapping {
        guid,
        format,
        colorspace,
    }
}

/// Translation of `fmtmappings`.
// This is not every possible format, just popular ones that SDL can reasonably handle.
// (and we should probably trim this list more.)
static FMTMAPPINGS: [FormatMapping; 14] = [
    mapping(
        MFVIDEOFORMAT_RGB555,
        PixelFormat::XRGB1555,
        Colorspace::SRGB,
    ),
    mapping(MFVIDEOFORMAT_RGB565, PixelFormat::RGB565, Colorspace::SRGB),
    mapping(MFVIDEOFORMAT_RGB24, PixelFormat::RGB24, Colorspace::SRGB),
    mapping(MFVIDEOFORMAT_RGB32, PixelFormat::XRGB8888, Colorspace::SRGB),
    mapping(
        MFVIDEOFORMAT_ARGB32,
        PixelFormat::ARGB8888,
        Colorspace::SRGB,
    ),
    mapping(
        MFVIDEOFORMAT_A2R10G10B10,
        PixelFormat::ARGB2101010,
        Colorspace::SRGB,
    ),
    mapping(
        MFVIDEOFORMAT_YV12,
        PixelFormat::YV12,
        Colorspace::BT709_LIMITED,
    ),
    mapping(
        MFVIDEOFORMAT_IYUV,
        PixelFormat::IYUV,
        Colorspace::BT709_LIMITED,
    ),
    mapping(
        MFVIDEOFORMAT_YUY2,
        PixelFormat::YUY2,
        Colorspace::BT709_LIMITED,
    ),
    mapping(
        MFVIDEOFORMAT_UYVY,
        PixelFormat::UYVY,
        Colorspace::BT709_LIMITED,
    ),
    mapping(
        MFVIDEOFORMAT_YVYU,
        PixelFormat::YVYU,
        Colorspace::BT709_LIMITED,
    ),
    mapping(
        MFVIDEOFORMAT_NV12,
        PixelFormat::NV12,
        Colorspace::BT709_LIMITED,
    ),
    mapping(
        MFVIDEOFORMAT_NV21,
        PixelFormat::NV21,
        Colorspace::BT709_LIMITED,
    ),
    mapping(MFVIDEOFORMAT_MJPG, PixelFormat::MJPG, Colorspace::SRGB),
];

fn same_guid(a: &GUID, b: &GUID) -> bool {
    crate::core::windows::is_equal_guid(a, b)
}

/// Translation of `GetMediaTypeColorspace()`.
fn media_type_colorspace(
    mediatype: &impl MediaTypeAttributes,
    default_colorspace: Colorspace,
) -> Colorspace {
    let mut colorspace = default_colorspace;

    if colorspace.color_type() == ColorType::Ycbcr {
        let range = match mediatype.get_uint32(&MF_MT_VIDEO_NOMINAL_RANGE) {
            Ok(MF_NOMINAL_RANGE_0_255) => ColorRange::Full,
            Ok(MF_NOMINAL_RANGE_16_235) => ColorRange::Limited,
            _ => default_colorspace.range(),
        };

        let primaries = match mediatype.get_uint32(&MF_MT_VIDEO_PRIMARIES) {
            Ok(MF_VIDEO_PRIMARIES_BT709) => ColorPrimaries::Bt709,
            Ok(MF_VIDEO_PRIMARIES_BT470_2_SYSM) => ColorPrimaries::Bt470m,
            Ok(MF_VIDEO_PRIMARIES_BT470_2_SYSBG) => ColorPrimaries::Bt470bg,
            Ok(MF_VIDEO_PRIMARIES_SMPTE170M) => ColorPrimaries::Bt601,
            Ok(MF_VIDEO_PRIMARIES_SMPTE240M) => ColorPrimaries::Smpte240,
            Ok(MF_VIDEO_PRIMARIES_EBU3213) => ColorPrimaries::Ebu3213,
            Ok(MF_VIDEO_PRIMARIES_BT2020) => ColorPrimaries::Bt2020,
            Ok(MF_VIDEO_PRIMARIES_XYZ) => ColorPrimaries::Xyz,
            Ok(MF_VIDEO_PRIMARIES_DCI_P3) => ColorPrimaries::Smpte432,
            _ => default_colorspace.primaries(),
        };

        let transfer = match mediatype.get_uint32(&MF_MT_TRANSFER_FUNCTION) {
            Ok(MF_VIDEO_TRANS_FUNC_10) => TransferCharacteristics::Linear,
            Ok(MF_VIDEO_TRANS_FUNC_22) => TransferCharacteristics::Gamma22,
            Ok(MF_VIDEO_TRANS_FUNC_709) => TransferCharacteristics::Bt709,
            Ok(MF_VIDEO_TRANS_FUNC_240M) => TransferCharacteristics::Smpte240,
            Ok(MF_VIDEO_TRANS_FUNC_SRGB) => TransferCharacteristics::Srgb,
            Ok(MF_VIDEO_TRANS_FUNC_28) => TransferCharacteristics::Gamma28,
            Ok(MF_VIDEO_TRANS_FUNC_LOG_100) => TransferCharacteristics::Log100,
            Ok(MF_VIDEO_TRANS_FUNC_2084) => TransferCharacteristics::Pq,
            Ok(MF_VIDEO_TRANS_FUNC_HLG) => TransferCharacteristics::Hlg,
            Ok(18 /* MFVideoTransFunc_BT1361_ECG */) => TransferCharacteristics::Bt1361,
            Ok(19 /* MFVideoTransFunc_SMPTE428 */) => TransferCharacteristics::Smpte428,
            _ => default_colorspace.transfer(),
        };

        let matrix = match mediatype.get_uint32(&MF_MT_YUV_MATRIX) {
            Ok(MF_VIDEO_TRANSFER_MATRIX_BT709) => MatrixCoefficients::Bt709,
            Ok(MF_VIDEO_TRANSFER_MATRIX_BT601) => MatrixCoefficients::Bt601,
            Ok(MF_VIDEO_TRANSFER_MATRIX_SMPTE240M) => MatrixCoefficients::Smpte240,
            Ok(MF_VIDEO_TRANSFER_MATRIX_BT2020_10) => MatrixCoefficients::Bt2020Ncl,
            Ok(6 /* MFVideoTransferMatrix_Identity */) => MatrixCoefficients::Identity,
            Ok(7 /* MFVideoTransferMatrix_FCC47 */) => MatrixCoefficients::Fcc,
            Ok(8 /* MFVideoTransferMatrix_YCgCo */) => MatrixCoefficients::Ycgco,
            Ok(9 /* MFVideoTransferMatrix_SMPTE2085 */) => MatrixCoefficients::Smpte2085,
            Ok(10 /* MFVideoTransferMatrix_Chroma */) => MatrixCoefficients::ChromaDerivedNcl,
            Ok(11 /* MFVideoTransferMatrix_Chroma_const */) => MatrixCoefficients::ChromaDerivedCl,
            Ok(12 /* MFVideoTransferMatrix_ICtCp */) => MatrixCoefficients::Ictcp,
            _ => default_colorspace.matrix(),
        };

        let chroma = match mediatype.get_uint32(&MF_MT_VIDEO_CHROMA_SITING) {
            Ok(MF_VIDEO_CHROMA_SUBSAMPLING_MPEG2) => ChromaLocation::Left,
            Ok(MF_VIDEO_CHROMA_SUBSAMPLING_MPEG1) => ChromaLocation::Center,
            Ok(MF_VIDEO_CHROMA_SUBSAMPLING_DV_PAL) => ChromaLocation::TopLeft,
            _ => default_colorspace.chroma(),
        };

        colorspace =
            define_colorspace(ColorType::Ycbcr, range, primaries, transfer, matrix, chroma);
    }
    colorspace
}

/// The SDL format and colorspace of a media type (`UNKNOWN` for both if
/// SDL doesn't handle it). Translation of `MediaTypeToSDLFmt()`.
fn media_type_to_sdl_fmt(mediatype: &impl MediaTypeAttributes) -> (PixelFormat, Colorspace) {
    if let Ok(ty) = mediatype.get_guid(&MF_MT_SUBTYPE) {
        if let Some(m) = FMTMAPPINGS.iter().find(|m| same_guid(&ty, &m.guid)) {
            return (m.format, media_type_colorspace(mediatype, m.colorspace));
        }
    }
    // (upstream logs the unknown type's FourCC when built with DEBUG_CAMERA)
    (PixelFormat::UNKNOWN, Colorspace::UNKNOWN)
}

/// Translation of `SDLFmtToMFVidFmtGuid()`.
fn sdl_fmt_to_mf_vid_fmt_guid(format: PixelFormat) -> Option<&'static GUID> {
    FMTMAPPINGS
        .iter()
        .find(|m| m.format == format)
        .map(|m| &m.guid)
}

/// Two `UINT32`s packed in a `UINT64` attribute (`MF_MT_FRAME_SIZE`: width
/// and height, `MF_MT_FRAME_RATE`: numerator and denominator), as
/// `MFSetAttributeSize()`/`MFSetAttributeRatio()` do. Upstream converts the
/// `int`s straight to `UINT64`, so this does too.
fn pack_attribute_pair(high: i32, low: i32) -> u64 {
    ((high as u64) << 32) | (low as u64)
}

/// The two `UINT32`s of a packed `UINT64` attribute (high, low).
fn unpack_attribute_pair(val: u64) -> (u32, u32) {
    ((val >> 32) as u32, val as u32)
}

/// The spec of one of a stream's media types, if SDL can use it: a video
/// type of a format SDL handles, with a size and a frame rate. The inner
/// loop of `GatherCameraSpecs()`.
fn media_type_spec(mediatype: &impl MediaTypeAttributes) -> Option<CameraSpec> {
    let ty = mediatype.get_guid(&MF_MT_MAJOR_TYPE).ok()?;
    if !same_guid(&ty, &MFMEDIATYPE_VIDEO) {
        return None;
    }
    let (sdlfmt, colorspace) = media_type_to_sdl_fmt(mediatype);
    if sdlfmt == PixelFormat::UNKNOWN {
        return None;
    }
    let (w, h) = unpack_attribute_pair(mediatype.get_uint64(&MF_MT_FRAME_SIZE).ok()?);
    if w == 0 || h == 0 {
        return None;
    }
    let (framerate_numerator, framerate_denominator) =
        unpack_attribute_pair(mediatype.get_uint64(&MF_MT_FRAME_RATE).ok()?);
    if framerate_numerator == 0 || framerate_denominator == 0 {
        return None;
    }
    Some(CameraSpec {
        format: sdlfmt,
        colorspace,
        width: w as i32,
        height: h as i32,
        framerate_numerator: framerate_numerator as i32,
        framerate_denominator: framerate_denominator as i32,
    })
}

/// The default stride (pitch) of a media type: its `MF_MT_DEFAULT_STRIDE`,
/// or else one computed from its subtype and width (with `stride_for`,
/// `MFGetStrideForBitmapInfoHeader()`) and stored in it for later
/// reference. Translation of `GetDefaultStride()`.
// this function is from https://learn.microsoft.com/en-us/windows/win32/medfound/uncompressed-video-buffers
fn get_default_stride(
    ptype: &impl MediaTypeAttributes,
    stride_for: impl FnOnce(u32, u32) -> std::result::Result<i32, HRESULT>,
) -> std::result::Result<i32, HRESULT> {
    // Try to get the default stride from the media type.
    if let Ok(lstride) = ptype.get_uint32(&MF_MT_DEFAULT_STRIDE) {
        return Ok(lstride as i32);
    }

    // Attribute not set. Try to calculate the default stride.

    // Get the subtype and the image size.
    let subtype = ptype.get_guid(&MF_MT_SUBTYPE)?;
    let (width, _height) = unpack_attribute_pair(ptype.get_uint64(&MF_MT_FRAME_SIZE)?);

    let lstride = stride_for(subtype.data1, width)?;

    // Set the attribute for later reference.
    let _ = ptype.set_uint32(&MF_MT_DEFAULT_STRIDE, lstride as u32);

    Ok(lstride)
}

/// A sample's time (in 100-nanosecond increments) in nanoseconds.
fn sample_time_to_ns(timestamp_100ns: i64) -> u64 {
    // the timestamps are in 100-nanosecond increments; move to full nanoseconds.
    (timestamp_100ns as u64).wrapping_mul(100)
}

/// A copy of a locked buffer's frame and its pitch. `buffer` is the whole
/// locked buffer, `top` the offset of the image's top row in it, and
/// `buflen` how much upstream copies from there. Translation of
/// `MEDIAFOUNDATION_CopyFrame()`.
///
/// Note (upstream): for a negative pitch (rows bottom-up in memory),
/// upstream moves its start a further `-pitch * (h - 1)` bytes on from the
/// top row, copies `buflen` bytes from there (reading past the end of the
/// buffer) and keeps the negative pitch. Here the rows are copied top-down
/// with the positive pitch, as the front end expects; and a copy that
/// would read past the buffer stops at its end.
fn copy_frame(
    buffer: &[u8],
    top: usize,
    pitch: i32,
    h: i32,
    buflen: usize,
) -> Option<(Vec<u8>, i32)> {
    if pitch >= 0 {
        let src = buffer.get(top..)?;
        return Some((src[..buflen.min(src.len())].to_vec(), pitch));
    }

    // image rows are reversed.
    let row_len = pitch.unsigned_abs() as usize;
    let rows = h.max(0) as usize;
    let mut pixels = Vec::with_capacity(row_len * rows);
    for row in 0..rows {
        let start = top.checked_sub(row * row_len)?;
        pixels.extend_from_slice(buffer.get(start..start + row_len)?);
    }
    Some((pixels, row_len as i32))
}

/// A buffer locked for reading, as a slice.
///
/// # Safety
///
/// `start` must be NULL with `len` 0, or `len` readable bytes that stay
/// valid while the slice is used.
unsafe fn locked_bytes<'a>(start: *const u8, len: usize) -> &'a [u8] {
    if start.is_null() || len == 0 {
        &[]
    } else {
        // SAFETY: the caller's contract.
        unsafe { std::slice::from_raw_parts(start, len) }
    }
}

// --- the libraries ---

/// The Media Foundation functions SDL uses, with the libraries they came
/// from (`libmf`, `libmfplat`, `libmfreadwrite` and the `p*` function
/// pointers). Loaded at run time: mf.dll, mfplat.dll and mfreadwrite.dll
/// are only on Vista and later.
struct MfLib {
    // mf.dll ...
    enum_device_sources:
        unsafe extern "system" fn(*mut c_void, *mut *mut *mut c_void, *mut u32) -> HRESULT,
    create_device_source: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    // mfplat.dll ...
    startup: unsafe extern "system" fn(u32, u32) -> HRESULT,
    shutdown: unsafe extern "system" fn() -> HRESULT,
    create_attributes: unsafe extern "system" fn(*mut *mut c_void, u32) -> HRESULT,
    create_media_type: unsafe extern "system" fn(*mut *mut c_void) -> HRESULT,
    get_stride_for_bitmap_info_header: unsafe extern "system" fn(u32, u32, *mut i32) -> HRESULT,
    // mfreadwrite.dll ...
    create_source_reader_from_media_source:
        unsafe extern "system" fn(*mut c_void, *mut c_void, *mut *mut c_void) -> HRESULT,
    // (dropped in this order, as MEDIAFOUNDATION_Deinitialize() frees them)
    _libmfreadwrite: SharedObject,
    _libmfplat: SharedObject,
    _libmf: SharedObject,
}

impl MfLib {
    /// The library and symbol loading of `MEDIAFOUNDATION_Init()`.
    fn load() -> Result<MfLib> {
        // !!! FIXME: slide this off into a subroutine
        let mf = SharedObject::load("Mf.dll")?; // this library is available in Vista and later, but also can be on XP with service packs and Windows
        let mfplat = SharedObject::load("Mfplat.dll")?; // this library is available in Vista and later. No WinXP, so have to LoadLibrary to use it for now!
        let mfreadwrite = SharedObject::load("Mfreadwrite.dll")?; // this library is available in Vista and later. No WinXP, so have to LoadLibrary to use it for now!

        macro_rules! loadsym {
            ($lib:expr, $name:literal) => {
                // SAFETY: the symbol has the prototype of the field it fills
                // (from the Media Foundation headers); the libraries stay
                // loaded with the functions.
                unsafe { $lib.function($name)? }
            };
        }

        Ok(MfLib {
            enum_device_sources: loadsym!(mf, "MFEnumDeviceSources"),
            create_device_source: loadsym!(mf, "MFCreateDeviceSource"),
            startup: loadsym!(mfplat, "MFStartup"),
            shutdown: loadsym!(mfplat, "MFShutdown"),
            create_attributes: loadsym!(mfplat, "MFCreateAttributes"),
            create_media_type: loadsym!(mfplat, "MFCreateMediaType"),
            get_stride_for_bitmap_info_header: loadsym!(mfplat, "MFGetStrideForBitmapInfoHeader"),
            create_source_reader_from_media_source: loadsym!(
                mfreadwrite,
                "MFCreateSourceReaderFromMediaSource"
            ),
            _libmfreadwrite: mfreadwrite,
            _libmfplat: mfplat,
            _libmf: mf,
        })
    }

    /// `MFCreateAttributes()`
    fn create_attributes(&self, initial_size: u32) -> std::result::Result<Attributes, HRESULT> {
        // SAFETY: MFCreateAttributes stores a new IMFAttributes.
        unsafe { out_ptr(|out| (self.create_attributes)(out, initial_size)) }
    }

    /// `MFCreateMediaType()`
    fn create_media_type(&self) -> std::result::Result<Attributes, HRESULT> {
        // SAFETY: MFCreateMediaType stores a new IMFMediaType.
        unsafe { out_ptr(|out| (self.create_media_type)(out)) }
    }

    /// `MFGetStrideForBitmapInfoHeader()`
    fn get_stride_for_bitmap_info_header(
        &self,
        format: u32,
        width: u32,
    ) -> std::result::Result<i32, HRESULT> {
        let mut stride = 0;
        // SAFETY: the function writes a LONG.
        check_hr(unsafe { (self.get_stride_for_bitmap_info_header)(format, width, &mut stride) })?;
        Ok(stride)
    }

    /// `MFEnumDeviceSources()`: the activation objects (the array they
    /// come in is freed).
    fn enum_device_sources(
        &self,
        attrs: &Attributes,
    ) -> std::result::Result<Vec<ComPtr<IMFActivateVtbl>>, HRESULT> {
        let mut activations: *mut *mut c_void = ptr::null_mut();
        let mut total = 0u32;
        // SAFETY: a live attribute store; on success the function stores a
        // CoTaskMemAlloc'd array of `total` owned IMFActivate references.
        check_hr(unsafe {
            (self.enum_device_sources)(attrs.as_ptr().cast(), &mut activations, &mut total)
        })?;
        if activations.is_null() {
            return Ok(Vec::new());
        }
        // SAFETY: as above; each reference is taken once, and the array is
        // freed once.
        unsafe {
            let list = (0..total as usize)
                .filter_map(|i| ComPtr::from_raw((*activations.add(i)).cast()))
                .collect();
            CoTaskMemFree(activations.cast());
            Ok(list)
        }
    }

    /// `MFCreateDeviceSource()`
    fn create_device_source(
        &self,
        attrs: &Attributes,
    ) -> std::result::Result<ComPtr<IMFMediaSourceVtbl>, HRESULT> {
        // SAFETY: a live attribute store; the function stores a new
        // IMFMediaSource.
        unsafe { out_ptr(|out| (self.create_device_source)(attrs.as_ptr().cast(), out)) }
    }

    /// `MFCreateSourceReaderFromMediaSource()` (without attributes)
    fn create_source_reader_from_media_source(
        &self,
        source: &ComPtr<IMFMediaSourceVtbl>,
    ) -> std::result::Result<ComPtr<IMFSourceReaderVtbl>, HRESULT> {
        // SAFETY: a live media source; the function stores a new
        // IMFSourceReader.
        unsafe {
            out_ptr(|out| {
                (self.create_source_reader_from_media_source)(
                    source.as_ptr().cast(),
                    ptr::null_mut(),
                    out,
                )
            })
        }
    }
}

// --- an opened camera ---

/// Translation of `SDL_PrivateCameraData`. (Media Foundation's objects are
/// free-threaded: the reader is opened on the app's thread and read on the
/// device thread, as upstream does.)
struct Hidden {
    srcreader: ComPtr<IMFSourceReaderVtbl>,
    current_sample: Option<ComPtr<IMFSampleVtbl>>,
    pitch: i32,
    /// The device's format and height (`frame->format` and `frame->h`).
    format: PixelFormat,
    height: i32,
}

/// The opened-device interface (`device->hidden` and the per-device
/// `MEDIAFOUNDATION_*` functions).
struct MfCamera {
    hidden: Mutex<Option<Hidden>>,
    /// Keeps the libraries loaded while their objects are alive.
    _lib: Arc<MfLib>,
}

impl MfCamera {
    fn hidden(&self) -> MutexGuard<'_, Option<Hidden>> {
        self.hidden.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// `IMFSourceReader_ReadSample()` of the first video stream: the sample
/// (if there is one) and the stream flags.
fn read_sample(
    srcreader: &ComPtr<IMFSourceReaderVtbl>,
) -> std::result::Result<(Option<ComPtr<IMFSampleVtbl>>, u32), HRESULT> {
    let mut stream_flags = 0u32;
    let mut sample: *mut c_void = ptr::null_mut();
    // SAFETY: a live source reader; ReadSample stores NULL or an owned
    // IMFSample.
    unsafe {
        check_hr((srcreader.vtbl().read_sample)(
            srcreader.as_ptr().cast(),
            MF_SOURCE_READER_FIRST_VIDEO_STREAM,
            0,
            ptr::null_mut(),
            &mut stream_flags,
            ptr::null_mut(),
            &mut sample,
        ))?;
        Ok((ComPtr::from_raw(sample.cast()), stream_flags))
    }
}

/// The `IMF2DBuffer2` path of `MEDIAFOUNDATION_AcquireFrame()`.
fn copy_from_2d_buffer2(
    buffer2d2: &ComPtr<IMF2DBuffer2Vtbl>,
    format: PixelFormat,
    h: i32,
) -> Option<(Vec<u8>, i32)> {
    let vtbl = buffer2d2.vtbl();
    let this = buffer2d2.as_ptr().cast();
    let mut pixels: *mut u8 = ptr::null_mut();
    let mut pitch = 0i32;
    let mut bufstart: *mut u8 = ptr::null_mut();
    let mut buflen = 0u32;
    // SAFETY: a live buffer; Lock2DSize writes the four out parameters.
    let ret = unsafe {
        (vtbl.lock_2d_size)(
            this,
            MF2DBUFFER_LOCKFLAGS_READ,
            &mut pixels,
            &mut pitch,
            &mut bufstart,
            &mut buflen,
        )
    };
    check_hr(ret).ok()?;
    if format == PixelFormat::MJPG {
        pitch = buflen as i32;
    }
    // SAFETY: while locked, the buffer is the `buflen` bytes at `bufstart`.
    let buffer = unsafe { locked_bytes(bufstart, buflen as usize) };
    let result = (pixels as usize)
        .checked_sub(bufstart as usize)
        .and_then(|top| copy_frame(buffer, top, pitch, h, buflen as usize));
    // SAFETY: balances the successful lock above.
    unsafe { (vtbl.buffer_2d.unlock_2d)(this) };
    result
}

/// The `IMF2DBuffer` path of `MEDIAFOUNDATION_AcquireFrame()`.
fn copy_from_2d_buffer(buffer2d: &ComPtr<IMF2DBufferVtbl>, h: i32) -> Option<(Vec<u8>, i32)> {
    let vtbl = buffer2d.vtbl();
    let this = buffer2d.as_ptr().cast();
    let mut pixels: *mut u8 = ptr::null_mut();
    let mut pitch = 0i32;
    // SAFETY: a live buffer; Lock2D writes the two out parameters.
    check_hr(unsafe { (vtbl.lock_2d)(this, &mut pixels, &mut pitch) }).ok()?;
    // FIXME (upstream): this is the size of the first plane only, so the
    // chroma planes of the planar formats (NV12, YV12, ...) aren't copied
    // (and upstream's frame is too short for them).
    let row_len = pitch.unsigned_abs() as usize;
    let rows = h.max(0) as usize;
    let buflen = row_len * rows;
    // The rows run from the top row (`pixels`) in the pitch's direction.
    let back = if pitch < 0 {
        row_len * rows.saturating_sub(1)
    } else {
        0
    };
    // SAFETY: while locked, the `rows` rows of `row_len` bytes from the top
    // row in the pitch's direction are the buffer's.
    let buffer = unsafe { locked_bytes(pixels.wrapping_sub(back), buflen) };
    let result = copy_frame(buffer, back, pitch, h, buflen);
    // SAFETY: balances the successful lock above.
    unsafe { (vtbl.unlock_2d)(this) };
    result
}

/// The `IMFMediaBuffer` path of `MEDIAFOUNDATION_AcquireFrame()`.
fn copy_from_media_buffer(
    buffer: &ComPtr<IMFMediaBufferVtbl>,
    format: PixelFormat,
    h: i32,
    default_pitch: i32,
) -> Option<(Vec<u8>, i32)> {
    let vtbl = buffer.vtbl();
    let this = buffer.as_ptr().cast();
    let mut pixels: *mut u8 = ptr::null_mut();
    let mut maxlen = 0u32;
    let mut buflen = 0u32;
    // SAFETY: a live buffer; Lock writes the three out parameters.
    check_hr(unsafe { (vtbl.lock)(this, &mut pixels, &mut maxlen, &mut buflen) }).ok()?;
    let pitch = if format == PixelFormat::MJPG {
        buflen as i32
    } else {
        default_pitch
    };
    // The buffer starts with the bottom row when the rows are reversed.
    let top = if pitch < 0 {
        pitch.unsigned_abs() as usize * (h.max(1) as usize - 1)
    } else {
        0
    };
    // SAFETY: while locked, the buffer is the `buflen` bytes at `pixels`.
    let bytes = unsafe { locked_bytes(pixels, buflen as usize) };
    let result = copy_frame(bytes, top, pitch, h, buflen as usize);
    // SAFETY: balances the successful lock above.
    unsafe { (vtbl.unlock)(this) };
    result
}

impl CameraBackend for MfCamera {
    fn wait_device(&self, device: &CameraDevice) -> bool {
        // Translation of `MEDIAFOUNDATION_WaitDevice()`.
        // (the reader is used without the lock, which close_device() only
        // takes once this thread is done)
        let srcreader = {
            let guard = self.hidden();
            let Some(hidden) = guard.as_ref() else {
                return false;
            };
            crate::sdl_assert!(hidden.current_sample.is_none());
            hidden.srcreader.clone()
        };
        let mut sample = None;

        while !device.shutting_down() {
            let Ok((s, stream_flags)) = read_sample(&srcreader) else {
                return false; // ruh roh.
            };

            // we currently ignore stream_flags format changes, but my _hope_ is that IMFSourceReader is handling this and
            // will continue to give us the explicitly-specified format we requested when opening the device, though, and
            // we don't have to manually deal with it.

            if s.is_some() {
                sample = s;
                break;
            } else if stream_flags & (MF_SOURCE_READERF_ERROR | MF_SOURCE_READERF_ENDOFSTREAM) != 0
            {
                return false; // apparently this camera has gone down.  :/
            }

            // otherwise, there was some minor burp, probably; just try again.
        }

        if let Some(hidden) = self.hidden().as_mut() {
            hidden.current_sample = sample;
        }

        true
    }

    fn acquire_frame(&self, _device: &CameraDevice) -> CameraFrameResult {
        // Translation of `MEDIAFOUNDATION_AcquireFrame()`.
        let (sample, format, h, default_pitch) = {
            let mut guard = self.hidden();
            let Some(hidden) = guard.as_mut() else {
                return CameraFrameResult::Error;
            };
            crate::sdl_assert!(hidden.current_sample.is_some());
            let Some(sample) = hidden.current_sample.take() else {
                return CameraFrameResult::Error;
            };
            (sample, hidden.format, hidden.height, hidden.pitch)
        };

        // FIXME (upstream): a failed GetSampleTime() sets the result to
        // SDL_CAMERA_FRAME_ERROR, but the `result < 0` check that should
        // then skip the copy is never true (SDL_CAMERA_FRAME_ERROR is 0),
        // and the copy's result replaces it: the frame is delivered anyway,
        // with a timestamp of 0. So it is here.
        let mut timestamp_100ns = 0i64;
        // SAFETY: a live sample; GetSampleTime writes a LONGLONG.
        let _ = unsafe {
            (sample.vtbl().get_sample_time)(sample.as_ptr().cast(), &mut timestamp_100ns)
        };
        let timestamp_ns = sample_time_to_ns(timestamp_100ns);

        // SAFETY: a live sample; the method stores a new IMFMediaBuffer.
        let buffer = unsafe {
            out_ptr::<IMFMediaBufferVtbl>(|out| {
                (sample.vtbl().convert_to_contiguous_buffer)(sample.as_ptr().cast(), out)
            })
        }; // IMFSample_GetBufferByIndex(sample, 0, &buffer);

        let copied = buffer.ok().and_then(|buffer| {
            if let Ok(buffer2d2) = buffer.query::<IMF2DBuffer2Vtbl>(&IID_IMF2DBUFFER2) {
                return copy_from_2d_buffer2(&buffer2d2, format, h);
            }
            if format != PixelFormat::MJPG {
                if let Ok(buffer2d) = buffer.query::<IMF2DBufferVtbl>(&IID_IMF2DBUFFER) {
                    return copy_from_2d_buffer(&buffer2d, h);
                }
            }
            copy_from_media_buffer(&buffer, format, h, default_pitch)
        });

        drop(sample);

        match copied {
            Some((pixels, pitch)) => CameraFrameResult::Ready(AcquiredFrame {
                pixels,
                pitch,
                timestamp_ns,
                rotation: 0.0,
            }),
            None => CameraFrameResult::Error,
        }
    }

    fn release_frame(&self, _device: &CameraDevice, _pixels: Vec<u8>) {
        // Translation of `MEDIAFOUNDATION_ReleaseFrame()`: the copy is
        // freed (dropped).
    }

    fn close_device(&self, _device: &CameraDevice) {
        // Translation of `MEDIAFOUNDATION_CloseDevice()`: the reader, then
        // any sample still held, are released.
        *self.hidden() = None;
    }
}

// --- the driver ---

/// The device handle: the device's symbolic link.
struct MfDeviceHandle {
    symlink: String,
}

/// A device's `MfDeviceHandle`.
fn handle_of(device: &CameraDevice) -> Option<&MfDeviceHandle> {
    device.handle.downcast_ref::<MfDeviceHandle>()
}

/// Translation of `QueryActivationObjectString()`.
fn query_activation_object_string(
    activation: &ComPtr<IMFActivateVtbl>,
    pguid: &GUID,
) -> Option<String> {
    activation.get_allocated_string(pguid).ok()
}

/// The specs of a device's media source. Translation of
/// `GatherCameraSpecs()`.
fn gather_camera_specs(source: &ComPtr<IMFMediaSourceVtbl>) -> Vec<CameraSpec> {
    // this has like a thousand steps.  :/

    let mut specs = Vec::new();

    // SAFETY: a live source; the method stores a new descriptor.
    let presentdesc = unsafe {
        out_ptr::<IMFPresentationDescriptorVtbl>(|out| {
            (source.vtbl().create_presentation_descriptor)(source.as_ptr().cast(), out)
        })
    };
    let Ok(presentdesc) = presentdesc else {
        return specs;
    };
    let pd = presentdesc.vtbl();

    let mut num_streams = 0u32;
    // SAFETY: a live descriptor; the method writes a DWORD.
    let ret =
        unsafe { (pd.get_stream_descriptor_count)(presentdesc.as_ptr().cast(), &mut num_streams) };
    if check_hr(ret).is_err() {
        num_streams = 0;
    }

    for i in 0..num_streams {
        let mut selected = 0i32;
        // SAFETY: a live descriptor; the method writes a BOOL and stores a
        // new stream descriptor.
        let streamdesc = unsafe {
            out_ptr::<IMFStreamDescriptorVtbl>(|out| {
                (pd.get_stream_descriptor_by_index)(
                    presentdesc.as_ptr().cast(),
                    i,
                    &mut selected,
                    out,
                )
            })
        };
        let Ok(streamdesc) = streamdesc else {
            continue;
        };

        if selected != 0 {
            // SAFETY: a live stream descriptor; the method stores a new
            // handler.
            let handler = unsafe {
                out_ptr::<IMFMediaTypeHandlerVtbl>(|out| {
                    (streamdesc.vtbl().get_media_type_handler)(streamdesc.as_ptr().cast(), out)
                })
            };
            if let Ok(handler) = handler {
                let hv = handler.vtbl();
                let mut num_mediatype = 0u32;
                // SAFETY: a live handler; the method writes a DWORD.
                let ret = unsafe {
                    (hv.get_media_type_count)(handler.as_ptr().cast(), &mut num_mediatype)
                };
                if check_hr(ret).is_err() {
                    num_mediatype = 0;
                }

                for j in 0..num_mediatype {
                    // SAFETY: a live handler; the method stores a new media
                    // type.
                    let mediatype = unsafe {
                        out_ptr::<IMFAttributesVtbl>(|out| {
                            (hv.get_media_type_by_index)(handler.as_ptr().cast(), j, out)
                        })
                    };
                    if let Some(spec) = mediatype.ok().as_ref().and_then(media_type_spec) {
                        specs.push(spec);
                    }
                }
            }
        }
    }

    specs
}

/// Translation of `FindMediaFoundationCameraBySymlink()`.
fn find_media_foundation_camera_by_symlink(device: &CameraDevice, symlink: &str) -> bool {
    handle_of(device).is_some_and(|h| h.symlink == symlink)
}

/// Translation of `MaybeAddDevice()`.
fn maybe_add_device(activation: &ComPtr<IMFActivateVtbl>) {
    let symlink = query_activation_object_string(
        activation,
        &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK,
    );

    // Note (upstream): upstream looks a NULL symlink up too, strcmp()ing it
    // against every device's; without one there's nothing to find here.
    if let Some(symlink) = &symlink {
        if find_physical_camera_by_callback(|d| find_media_foundation_camera_by_symlink(d, symlink))
            .is_ok()
        {
            return; // already have this one.
        }
    }

    let name = query_activation_object_string(activation, &MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME);
    if let (Some(name), Some(symlink)) = (name, symlink) {
        // "activating" here only creates an object, it doesn't open the actual camera hardware or start recording.
        // SAFETY: a live activation object; ActivateObject stores an owned
        // reference to the IMFMediaSource interface asked for.
        let source = unsafe {
            out_ptr::<IMFMediaSourceVtbl>(|out| {
                (activation.vtbl().activate_object)(
                    activation.as_ptr().cast(),
                    &IID_IMFMEDIASOURCE,
                    out,
                )
            })
        };
        if let Ok(source) = source {
            let specs = gather_camera_specs(&source);
            if !specs.is_empty() {
                add_camera(
                    &name,
                    CameraPosition::Unknown,
                    &specs,
                    Box::new(MfDeviceHandle { symlink }),
                );
            }
            // SAFETY: a live activation object whose object was created.
            unsafe { (activation.vtbl().shutdown_object)(activation.as_ptr().cast()) };
            drop(source);
        }
    }
}

/// `WIN_SetErrorFromHRESULT(what " failed", r)` for a failed call.
fn check<T>(what: &str, r: std::result::Result<T, HRESULT>) -> Result<T> {
    r.map_err(|hr| error_from_hresult(Some(&format!("{what} failed")), hr))
}

/// The steps of `MEDIAFOUNDATION_OpenDevice()` once the media source
/// exists: its source reader, set to `spec`, and the default stride.
fn open_source_reader(
    lib: &MfLib,
    source: &ComPtr<IMFMediaSourceVtbl>,
    spec: &CameraSpec,
) -> Result<(ComPtr<IMFSourceReaderVtbl>, i32)> {
    // !!! FIXME: I think it'd be nice to do this without an IMFSourceReader,
    // since it's just utility code that has to handle more complex media streams
    // than we're dealing with, but this will do for now. The docs are slightly
    // insistent that you should use one, though...Maybe it's extremely hard
    // to handle directly at the IMFMediaSource layer...?
    let srcreader = check(
        "MFCreateSourceReaderFromMediaSource",
        lib.create_source_reader_from_media_source(source),
    )?;

    // !!! FIXME: do we actually have to find the media type object in the source reader or can we just roll our own like this?
    let mediatype = check("MFCreateMediaType", lib.create_media_type())?;

    check(
        "IMFMediaType_SetGUID(major_type)",
        mediatype.set_guid(&MF_MT_MAJOR_TYPE, &MFMEDIATYPE_VIDEO),
    )?;

    // Note (upstream): upstream passes a NULL GUID for a format that isn't
    // in the table; that's E_POINTER here. (The specs only list formats
    // from the table, though.)
    let subtype = sdl_fmt_to_mf_vid_fmt_guid(spec.format).ok_or(E_POINTER);
    check(
        "IMFMediaType_SetGUID(subtype)",
        subtype.and_then(|g| mediatype.set_guid(&MF_MT_SUBTYPE, g)),
    )?;

    check(
        "MFSetAttributeSize(frame_size)",
        mediatype.set_uint64(
            &MF_MT_FRAME_SIZE,
            pack_attribute_pair(spec.width, spec.height),
        ),
    )?;

    check(
        "MFSetAttributeRatio(frame_rate)",
        mediatype.set_uint64(
            &MF_MT_FRAME_RATE,
            pack_attribute_pair(spec.framerate_numerator, spec.framerate_denominator),
        ),
    )?;

    // SAFETY: a live reader and media type (which the reader copies).
    let ret = unsafe {
        (srcreader.vtbl().set_current_media_type)(
            srcreader.as_ptr().cast(),
            MF_SOURCE_READER_FIRST_VIDEO_STREAM,
            ptr::null_mut(),
            mediatype.as_ptr().cast(),
        )
    };
    check("IMFSourceReader_SetCurrentMediaType", check_hr(ret))?;

    // (upstream has an untested, disabled (#if 0) start of the media source
    // without an IMFSourceReader here.)

    let lstride = check(
        "GetDefaultStride",
        get_default_stride(&mediatype, |format, width| {
            lib.get_stride_for_bitmap_info_header(format, width)
        }),
    )?;

    Ok((srcreader, lstride))
}

/// The driver (`MEDIAFOUNDATION_Init()`'s function table).
struct MediaFoundationDriver {
    lib: Arc<MfLib>,
}

impl CameraDriverImpl for MediaFoundationDriver {
    fn detect_devices(&self) {
        // Translation of `MEDIAFOUNDATION_DetectDevices()`.
        // !!! FIXME: use CM_Register_Notification (Win8+) to get device notifications.
        // !!! FIXME: Earlier versions can use RegisterDeviceNotification, but I'm not bothering: no hotplug for you!
        let Ok(attrs) = self.lib.create_attributes(1) else {
            return; // oh well, no cameras for you.
        };

        if attrs
            .set_guid(
                &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE,
                &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID,
            )
            .is_err()
        {
            return; // oh well, no cameras for you.
        }

        let activations = self.lib.enum_device_sources(&attrs);
        drop(attrs);
        let Ok(activations) = activations else {
            return; // oh well, no cameras for you.
        };

        for activation in activations {
            maybe_add_device(&activation);
        }
    }

    fn open_device(
        &self,
        device: &Arc<CameraDevice>,
        spec: &CameraSpec,
    ) -> Result<Arc<dyn CameraBackend>> {
        // Translation of `MEDIAFOUNDATION_OpenDevice()`.
        let Some(handle) = handle_of(device) else {
            return Err(Error::new("Not a Media Foundation device"));
        };
        let lib = &self.lib;

        // (upstream logs "CAMERA: opening device with symlink of '%s'" when
        // built with DEBUG_CAMERA)

        let wstrsymlink = utf8_to_wide(&handle.symlink);

        let attrs = check("MFCreateAttributes", lib.create_attributes(1))?;

        check(
            "IMFAttributes_SetGUID(srctype)",
            attrs.set_guid(
                &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE,
                &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID,
            ),
        )?;

        check(
            "IMFAttributes_SetString(symlink)",
            attrs.set_string(
                &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK,
                &wstrsymlink,
            ),
        )?;

        let source = check("MFCreateDeviceSource", lib.create_device_source(&attrs))?;

        drop(attrs);
        drop(wstrsymlink);

        let (srcreader, lstride) = match open_source_reader(lib, &source, spec) {
            Ok(opened) => opened,
            Err(e) => {
                // (the reader is already released)
                // SAFETY: a live media source, shut down once.
                unsafe { (source.vtbl().shutdown)(source.as_ptr().cast()) };
                return Err(e);
            }
        };

        let backend = Arc::new(MfCamera {
            hidden: Mutex::new(Some(Hidden {
                srcreader,
                current_sample: None,
                pitch: lstride,
                format: spec.format,
                height: spec.height,
            })),
            _lib: lib.clone(),
        });
        drop(source); // srcreader is holding a reference to this.

        // There is no user permission prompt for camera access (I think?)
        camera_permission_outcome(device, true);

        Ok(backend)
    }

    fn free_device_handle(&self, _device: &CameraDevice) {
        // Translation of `MEDIAFOUNDATION_FreeDeviceHandle()`: the device's
        // symlink string is freed with the device.
    }

    fn deinitialize(&self) {
        // Translation of `MEDIAFOUNDATION_Deinitialize()` (the libraries are
        // unloaded with the last reference to them).
        // SAFETY: balances the MFStartup() of mediafoundation_init().
        unsafe { (self.lib.shutdown)() };
    }
}

/// Translation of `MEDIAFOUNDATION_Init()`. (Upstream fails without
/// setting an error; this reports what failed.)
fn mediafoundation_init() -> Result<Arc<dyn CameraDriverImpl>> {
    let lib = MfLib::load()?;

    // SAFETY: MFStartup takes a version and flags.
    let ret = unsafe { (lib.startup)(MF_VERSION, MFSTARTUP_LITE) };
    check("MFStartup", check_hr(ret))?;

    Ok(Arc::new(MediaFoundationDriver { lib: Arc::new(lib) }))
}

/// Translation of `MEDIAFOUNDATION_bootstrap`.
pub(super) static MEDIAFOUNDATION_BOOTSTRAP: CameraBootStrap = CameraBootStrap {
    name: "mediafoundation",
    desc: "SDL Windows Media Foundation camera driver",
    init: mediafoundation_init,
    demand_only: false,
};

#[cfg(test)]
mod tests;
