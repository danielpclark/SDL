// The camera2 NDK (<camera/NdkCameraDevice.h>, <camera/NdkCameraManager.h>,
// API 24 and later) and media NDK (<media/NdkImage.h>,
// <media/NdkImageReader.h>) declarations the Android camera driver uses,
// loaded at run time from libcamera2ndk.so and libmediandk.so as upstream
// does.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

#![allow(non_camel_case_types, non_snake_case)]

use std::ffi::{c_char, c_int, c_void};
use std::mem::ManuallyDrop;

use crate::core::android::ndk::ANativeWindow;
use crate::loadso::SharedObject;

macro_rules! opaque {
    ($($name:ident),* $(,)?) => {
        $(
            #[doc = concat!("`", stringify!($name), "` (opaque).")]
            #[repr(C)]
            pub(super) struct $name {
                _private: [u8; 0],
            }
        )*
    };
}

opaque!(
    ACameraManager,
    ACameraDevice,
    ACameraCaptureSession,
    ACaptureRequest,
    ACameraOutputTarget,
    ACaptureSessionOutputContainer,
    ACaptureSessionOutput,
    ACameraMetadata,
    ACameraCaptureSession_captureCallbacks,
    AImage,
    AImageReader,
);

/// `ACameraWindowType` (the NDK's, not the VNDK's).
pub(super) type ACameraWindowType = ANativeWindow;

pub(super) type camera_status_t = c_int;
pub(super) type media_status_t = c_int;
/// `ACameraDevice_request_template`.
pub(super) type ACameraDevice_request_template = c_int;

pub(super) const ACAMERA_OK: camera_status_t = 0;
pub(super) const ACAMERA_ERROR_UNKNOWN: camera_status_t = -10000;
pub(super) const ACAMERA_ERROR_INVALID_PARAMETER: camera_status_t = -10001;
pub(super) const ACAMERA_ERROR_CAMERA_DISCONNECTED: camera_status_t = -10002;
pub(super) const ACAMERA_ERROR_NOT_ENOUGH_MEMORY: camera_status_t = -10003;
pub(super) const ACAMERA_ERROR_METADATA_NOT_FOUND: camera_status_t = -10004;
pub(super) const ACAMERA_ERROR_CAMERA_DEVICE: camera_status_t = -10005;
pub(super) const ACAMERA_ERROR_CAMERA_SERVICE: camera_status_t = -10006;
pub(super) const ACAMERA_ERROR_SESSION_CLOSED: camera_status_t = -10007;
pub(super) const ACAMERA_ERROR_INVALID_OPERATION: camera_status_t = -10008;
pub(super) const ACAMERA_ERROR_STREAM_CONFIGURE_FAIL: camera_status_t = -10009;
pub(super) const ACAMERA_ERROR_CAMERA_IN_USE: camera_status_t = -10010;
pub(super) const ACAMERA_ERROR_MAX_CAMERA_IN_USE: camera_status_t = -10011;
pub(super) const ACAMERA_ERROR_CAMERA_DISABLED: camera_status_t = -10012;
pub(super) const ACAMERA_ERROR_PERMISSION_DENIED: camera_status_t = -10013;
pub(super) const ACAMERA_ERROR_UNSUPPORTED_OPERATION: camera_status_t = -10014;

pub(super) const AMEDIA_OK: media_status_t = 0;
pub(super) const AMEDIACODEC_ERROR_INSUFFICIENT_RESOURCE: media_status_t = 1100;
pub(super) const AMEDIACODEC_ERROR_RECLAIMED: media_status_t = 1101;
pub(super) const AMEDIA_ERROR_UNKNOWN: media_status_t = -10000;
pub(super) const AMEDIA_ERROR_MALFORMED: media_status_t = -10001;
pub(super) const AMEDIA_ERROR_UNSUPPORTED: media_status_t = -10002;
pub(super) const AMEDIA_ERROR_INVALID_OBJECT: media_status_t = -10003;
pub(super) const AMEDIA_ERROR_INVALID_PARAMETER: media_status_t = -10004;
pub(super) const AMEDIA_ERROR_INVALID_OPERATION: media_status_t = -10005;
pub(super) const AMEDIA_ERROR_END_OF_STREAM: media_status_t = -10006;
pub(super) const AMEDIA_ERROR_IO: media_status_t = -10007;
pub(super) const AMEDIA_ERROR_WOULD_BLOCK: media_status_t = -10008;
pub(super) const AMEDIA_DRM_NOT_PROVISIONED: media_status_t = -20001;
pub(super) const AMEDIA_DRM_RESOURCE_BUSY: media_status_t = -20002;
pub(super) const AMEDIA_DRM_DEVICE_REVOKED: media_status_t = -20003;
pub(super) const AMEDIA_DRM_SHORT_BUFFER: media_status_t = -20004;
pub(super) const AMEDIA_DRM_SESSION_NOT_OPENED: media_status_t = -20005;
pub(super) const AMEDIA_DRM_TAMPER_DETECTED: media_status_t = -20006;
pub(super) const AMEDIA_DRM_VERIFY_FAILED: media_status_t = -20007;
pub(super) const AMEDIA_DRM_NEED_KEY: media_status_t = -20008;
pub(super) const AMEDIA_DRM_LICENSE_EXPIRED: media_status_t = -20009;
pub(super) const AMEDIA_IMGREADER_NO_BUFFER_AVAILABLE: media_status_t = -30001;
pub(super) const AMEDIA_IMGREADER_MAX_IMAGES_ACQUIRED: media_status_t = -30002;
pub(super) const AMEDIA_IMGREADER_CANNOT_LOCK_IMAGE: media_status_t = -30003;
pub(super) const AMEDIA_IMGREADER_CANNOT_UNLOCK_IMAGE: media_status_t = -30004;
pub(super) const AMEDIA_IMGREADER_IMAGE_NOT_LOCKED: media_status_t = -30005;

/// `AIMAGE_FORMATS` (the ones SDL knows).
pub(super) const AIMAGE_FORMAT_RGBA_8888: i32 = 0x1;
pub(super) const AIMAGE_FORMAT_RGBX_8888: i32 = 0x2;
pub(super) const AIMAGE_FORMAT_RGB_888: i32 = 0x3;
pub(super) const AIMAGE_FORMAT_RGB_565: i32 = 0x4;
pub(super) const AIMAGE_FORMAT_RGBA_FP16: i32 = 0x16;
pub(super) const AIMAGE_FORMAT_YUV_420_888: i32 = 0x23;

/// `TEMPLATE_RECORD` (`ACameraDevice_request_template`).
pub(super) const TEMPLATE_RECORD: ACameraDevice_request_template = 3;

// (`acamera_metadata_tag`: a section's start, `ACAMERA_..._START`, is its
// index << 16)
/// `ACAMERA_LENS_FACING` (`ACAMERA_LENS_START + 5`): a byte.
pub(super) const ACAMERA_LENS_FACING: u32 = (8 << 16) + 5;
/// `ACAMERA_SCALER_AVAILABLE_STREAM_CONFIGURATIONS`
/// (`ACAMERA_SCALER_START + 10`): (format, width, height, input) int32s.
pub(super) const ACAMERA_SCALER_AVAILABLE_STREAM_CONFIGURATIONS: u32 = (13 << 16) + 10;
/// `ACAMERA_SCALER_AVAILABLE_MIN_FRAME_DURATIONS` (`ACAMERA_SCALER_START + 11`).
pub(super) const ACAMERA_SCALER_AVAILABLE_MIN_FRAME_DURATIONS: u32 = (13 << 16) + 11;
/// `ACAMERA_SENSOR_ORIENTATION` (`ACAMERA_SENSOR_START + 14`): an int32.
pub(super) const ACAMERA_SENSOR_ORIENTATION: u32 = (14 << 16) + 14;
/// `ACAMERA_INFO_VERSION` (`ACAMERA_INFO_START + 1`): bytes.
pub(super) const ACAMERA_INFO_VERSION: u32 = (21 << 16) + 1;

/// `ACAMERA_LENS_FACING_FRONT`.
pub(super) const ACAMERA_LENS_FACING_FRONT: u8 = 0;
/// `ACAMERA_LENS_FACING_BACK`.
pub(super) const ACAMERA_LENS_FACING_BACK: u8 = 1;
/// `ACAMERA_SCALER_AVAILABLE_STREAM_CONFIGURATIONS_INPUT`.
pub(super) const ACAMERA_SCALER_AVAILABLE_STREAM_CONFIGURATIONS_INPUT: i32 = 1;

/// `ACameraIdList`.
#[repr(C)]
pub(super) struct ACameraIdList {
    /// Number of camera device Ids
    pub(super) numCameras: c_int,
    /// list of camera device Ids
    pub(super) cameraIds: *const *const c_char,
}

/// `ACameraMetadata_const_entry`: its `data` union as a pointer, read as
/// the entry's type.
#[repr(C)]
pub(super) struct ACameraMetadata_const_entry {
    pub(super) tag: u32,
    pub(super) type_: u8,
    /// the number of values (of the type), not of bytes
    pub(super) count: u32,
    pub(super) data: *const c_void,
}

/// `ACameraManager_AvailabilityCallback`.
pub(super) type ACameraManager_AvailabilityCallback =
    Option<unsafe extern "C" fn(context: *mut c_void, cameraId: *const c_char)>;

/// `ACameraManager_AvailabilityCallbacks`.
#[repr(C)]
pub(super) struct ACameraManager_AvailabilityCallbacks {
    pub(super) context: *mut c_void,
    pub(super) onCameraAvailable: ACameraManager_AvailabilityCallback,
    pub(super) onCameraUnavailable: ACameraManager_AvailabilityCallback,
}

/// `ACameraDevice_StateCallbacks`.
#[repr(C)]
pub(super) struct ACameraDevice_StateCallbacks {
    pub(super) context: *mut c_void,
    pub(super) onDisconnected:
        Option<unsafe extern "C" fn(context: *mut c_void, device: *mut ACameraDevice)>,
    pub(super) onError: Option<
        unsafe extern "C" fn(context: *mut c_void, device: *mut ACameraDevice, error: c_int),
    >,
}

/// `ACameraCaptureSession_stateCallback`.
pub(super) type ACameraCaptureSession_stateCallback =
    Option<unsafe extern "C" fn(context: *mut c_void, session: *mut ACameraCaptureSession)>;

/// `ACameraCaptureSession_stateCallbacks`.
#[repr(C)]
pub(super) struct ACameraCaptureSession_stateCallbacks {
    pub(super) context: *mut c_void,
    pub(super) onClosed: ACameraCaptureSession_stateCallback,
    pub(super) onReady: ACameraCaptureSession_stateCallback,
    pub(super) onActive: ACameraCaptureSession_stateCallback,
}

/// `AImageReader_ImageListener`.
#[repr(C)]
pub(super) struct AImageReader_ImageListener {
    pub(super) context: *mut c_void,
    pub(super) onImageAvailable:
        Option<unsafe extern "C" fn(context: *mut c_void, reader: *mut AImageReader)>,
}

macro_rules! ndk_procs {
    (
        $(#[$meta:meta])*
        $lib:ident ($file:literal) { $($name:ident: fn($($arg:ty),*) $(-> $ret:ty)?;)* }
    ) => {
        $(#[$meta])*
        #[allow(dead_code)] // (all are loaded, as upstream does, though not every one is called)
        pub(super) struct $lib {
            $(pub(super) $name: unsafe extern "C" fn($($arg),*) $(-> $ret)?,)*
            /// Never closed (see `camera/android.rs`).
            _handle: ManuallyDrop<SharedObject>,
        }

        impl $lib {
            /// Upstream's `LOADSYM`s of the library's functions, the first
            /// one missing logged.
            pub(super) fn load(handle: SharedObject) -> Option<$lib> {
                Some($lib {
                    $(
                        // SAFETY: the declared type is the function's C
                        // signature from the NDK's headers, and it is only
                        // called while `_handle` keeps the library loaded.
                        $name: match unsafe { handle.function(stringify!($name)) } {
                            Ok(f) => f,
                            Err(e) => {
                                crate::log!(
                                    "CAMERA: symbol '{}' can't be found in {}: {}",
                                    stringify!($name),
                                    $file,
                                    e.message()
                                );
                                return None;
                            }
                        },
                    )*
                    _handle: ManuallyDrop::new(handle),
                })
            }
        }
    };
}

ndk_procs! {
    /// libcamera2ndk's functions (upstream's `pACamera...` pointers).
    Camera2Ndk ("libcamera2ndk.so") {
    ACameraManager_create: fn() -> *mut ACameraManager;
    ACameraManager_registerAvailabilityCallback: fn(*mut ACameraManager, *const ACameraManager_AvailabilityCallbacks) -> camera_status_t;
    ACameraManager_unregisterAvailabilityCallback: fn(*mut ACameraManager, *const ACameraManager_AvailabilityCallbacks) -> camera_status_t;
    ACameraManager_getCameraIdList: fn(*mut ACameraManager, *mut *mut ACameraIdList) -> camera_status_t;
    ACameraManager_deleteCameraIdList: fn(*mut ACameraIdList);
    ACameraCaptureSession_close: fn(*mut ACameraCaptureSession);
    ACaptureRequest_free: fn(*mut ACaptureRequest);
    ACameraOutputTarget_free: fn(*mut ACameraOutputTarget);
    ACameraDevice_close: fn(*mut ACameraDevice) -> camera_status_t;
    ACameraManager_delete: fn(*mut ACameraManager);
    ACaptureSessionOutputContainer_free: fn(*mut ACaptureSessionOutputContainer);
    ACaptureSessionOutput_free: fn(*mut ACaptureSessionOutput);
    ACameraManager_openCamera: fn(*mut ACameraManager, *const c_char, *mut ACameraDevice_StateCallbacks, *mut *mut ACameraDevice) -> camera_status_t;
    ACameraDevice_createCaptureRequest: fn(*const ACameraDevice, ACameraDevice_request_template, *mut *mut ACaptureRequest) -> camera_status_t;
    ACameraDevice_createCaptureSession: fn(*mut ACameraDevice, *const ACaptureSessionOutputContainer, *const ACameraCaptureSession_stateCallbacks, *mut *mut ACameraCaptureSession) -> camera_status_t;
    ACameraManager_getCameraCharacteristics: fn(*mut ACameraManager, *const c_char, *mut *mut ACameraMetadata) -> camera_status_t;
    ACameraMetadata_free: fn(*mut ACameraMetadata);
    ACameraMetadata_getConstEntry: fn(*const ACameraMetadata, u32, *mut ACameraMetadata_const_entry) -> camera_status_t;
    ACameraCaptureSession_setRepeatingRequest: fn(*mut ACameraCaptureSession, *mut ACameraCaptureSession_captureCallbacks, c_int, *mut *mut ACaptureRequest, *mut c_int) -> camera_status_t;
    ACameraOutputTarget_create: fn(*mut ACameraWindowType, *mut *mut ACameraOutputTarget) -> camera_status_t;
    ACaptureRequest_addTarget: fn(*mut ACaptureRequest, *const ACameraOutputTarget) -> camera_status_t;
    ACaptureSessionOutputContainer_add: fn(*mut ACaptureSessionOutputContainer, *const ACaptureSessionOutput) -> camera_status_t;
    ACaptureSessionOutputContainer_create: fn(*mut *mut ACaptureSessionOutputContainer) -> camera_status_t;
    ACaptureSessionOutput_create: fn(*mut ACameraWindowType, *mut *mut ACaptureSessionOutput) -> camera_status_t;
    }
}

ndk_procs! {
    /// libmediandk's functions (upstream's `pAImage...` pointers).
    MediaNdk ("libmediandk.so") {
    AImage_delete: fn(*mut AImage);
    AImage_getTimestamp: fn(*const AImage, *mut i64) -> media_status_t;
    AImage_getNumberOfPlanes: fn(*const AImage, *mut i32) -> media_status_t;
    AImage_getPlaneRowStride: fn(*const AImage, c_int, *mut i32) -> media_status_t;
    AImage_getPlaneData: fn(*const AImage, c_int, *mut *mut u8, *mut c_int) -> media_status_t;
    AImageReader_acquireNextImage: fn(*mut AImageReader, *mut *mut AImage) -> media_status_t;
    AImageReader_delete: fn(*mut AImageReader);
    AImageReader_setImageListener: fn(*mut AImageReader, *mut AImageReader_ImageListener) -> media_status_t;
    AImageReader_getWindow: fn(*mut AImageReader, *mut *mut ANativeWindow) -> media_status_t;
    AImageReader_new: fn(i32, i32, i32, i32, *mut *mut AImageReader) -> media_status_t;
    AImage_getWidth: fn(*const AImage, *mut i32) -> media_status_t;
    AImage_getHeight: fn(*const AImage, *mut i32) -> media_status_t;
    }
}
