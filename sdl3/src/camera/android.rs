// Rust translation of src/camera/android/SDL_camera_android.c from Simple
// DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Android camera driver: the camera2 NDK (`ACameraManager`,
//! `ACameraDevice`, a capture session) with an `AImageReader` for the
//! frames, both libraries (libcamera2ndk.so and libmediandk.so, API 24 and
//! later) loaded at run time, as upstream loads them, so that the driver
//! fails to start rather than the program to load on older systems.
//!
//! The app's manifest needs the `CAMERA` permission (upstream's
//! android-project has it commented out, with the camera feature).
//!
//! The cameras are the camera manager's (`ACameraManager_getCameraIdList()`)
//! that have output formats SDL knows (each at 30 frames per second: the
//! minimum frame durations aren't used), named by their
//! `ACAMERA_INFO_VERSION`, else by their lens's facing ("Front-facing
//! camera", "Back-facing camera"), else "Generic camera"; the camera
//! manager's availability callbacks add a camera that becomes available
//! and disconnect one that becomes unavailable while it isn't open (as
//! when another app opens it: it comes back, a new device, once that app
//! closes it). Opening a camera asks for the `CAMERA` permission
//! (`SDL_RequestAndroidPermission()`); the camera is only opened once it's
//! granted (at once when it was already): the camera device, an image
//! reader of the requested spec (ten buffers) whose window is the capture
//! session's output and the target of a repeating `TEMPLATE_RECORD`
//! request. A denial is the camera's permission outcome; a failure to open
//! it then disconnects the camera. The driver provides its own "thread"
//! (`ProvidesOwnCallbackThread`): the image reader's listener runs the
//! camera's iteration for each image, which copies the image's planes (Y,
//! and NV12's interleaved U and V as one) into the frame, with its
//! rotation: the display's current orientation (subtracted for a
//! back-facing camera) plus the sensor's.
//!
//! The callbacks' context is the camera's instance ID, which they look the
//! device up by (as the CoreMedia, PipeWire and Emscripten drivers'
//! callbacks do), where upstream's is the `SDL_Camera *`.
//!
//! Note (upstream), what the driver does, kept as it is:
//!
//! * a planar frame's planes are copied with the Y plane's row stride as
//!   theirs (asserted: the emulator's YUV frames' chroma planes have half
//!   of it, so there the assertion fails, and a release build, without
//!   it, copies their rows as if they had the Y plane's), and one of three
//!   planes is dropped only when the spec the app asked for (not the
//!   camera's) is NV12;
//! * the thread priority isn't set (the core's `SDL_CameraThreadSetup()`
//!   leaves Android's to a TODO).
//!
//! Note (upstream), what upstream does that is done differently here:
//!
//! * `GatherCameraSpecs()` walks the stream configurations as if the
//!   entry's `count` were the number of configurations, when it is the
//!   number of `int32_t` values (four per configuration): it reads three
//!   times as far again past the entry's data, and adds what it finds there
//!   that looks like an output format. Here only the configurations are
//!   read (`FIXME (upstream)`);
//! * `ANDROIDCAMERA_Init()` goes on when a function is missing (it closes
//!   both libraries, then calls their functions); here the driver fails to
//!   start (`FIXME (upstream)`);
//! * the image reader's listener calls `SDL_CameraThreadIterate()`, which
//!   waits for the camera's lock, while closing the camera holds that lock
//!   and deletes the reader, which waits for the listener's thread: an
//!   image that arrives as the camera closes deadlocks both. Here the
//!   listener waits for the lock only while the camera isn't shutting down
//!   (`Note`);
//! * the availability callbacks read whether a camera is open
//!   (`device->hidden`) without its lock, which here they take: so a camera
//!   being opened is seen as open (a camera manager's "unavailable" for the
//!   app's own open waits until the open is done);
//! * the camera manager is deleted once nothing uses it: by
//!   `ANDROIDCAMERA_Deinitialize()`, or by an availability callback that was
//!   running then;
//! * the libraries are never closed (upstream `dlclose()`s them when the
//!   driver quits): libcamera_client keeps libcamera2ndk's vendor tag cache
//!   in a static, whose destructor, at the process's `exit()`, calls into
//!   the closed library, which aborts (a CFI check fails).

mod sys;

use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::ptr;
use std::sync::{Arc, Mutex, MutexGuard, RwLock};
use std::time::Duration;

use sys::*;

use super::{
    add_camera, add_camera_format, camera_disconnected, camera_permission_outcome,
    camera_thread_iterate, find_physical_camera_by_callback, ref_physical_camera,
    unref_physical_camera, AcquiredFrame, CameraBackend, CameraBootStrap, CameraDevice,
    CameraDriverImpl, CameraFrameResult, CameraID, CameraPosition, CameraSpec,
};
use crate::core::android::ndk::ANativeWindow;
use crate::error::{Error, Result};
use crate::loadso::SharedObject;
use crate::video::{Colorspace, DisplayOrientation, PixelFormat};

/// The driver: the libraries (`libcamera2ndk`, `libmediandk` and their
/// functions) and the camera manager (`cameraMgr`), which the camera
/// manager's callbacks use too ([`DRIVER`]).
struct AndroidCamera {
    camera2: Camera2Ndk,
    media: MediaNdk,
    camera_mgr: *mut ACameraManager,
}

// SAFETY: the camera manager is thread-safe (the NDK's callbacks use it
// from their own threads, as upstream's do).
unsafe impl Send for AndroidCamera {}
unsafe impl Sync for AndroidCamera {}

impl Drop for AndroidCamera {
    /// Translation of `DestroyCameraManager()`. Note: the libraries stay
    /// loaded, where `ANDROIDCAMERA_Deinitialize()` `dlclose()`s them (see
    /// the module's documentation).
    fn drop(&mut self) {
        if !self.camera_mgr.is_null() {
            // SAFETY: the manager, deleted once.
            unsafe { (self.camera2.ACameraManager_delete)(self.camera_mgr) };
            self.camera_mgr = ptr::null_mut();
        }
    }
}

/// The running driver, for the camera manager's availability callbacks.
static DRIVER: RwLock<Option<Arc<AndroidCamera>>> = RwLock::new(None);

fn driver() -> Option<Arc<AndroidCamera>> {
    DRIVER.read().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Translation of `struct SDL_PrivateCameraData`.
struct PrivateCameraData {
    device: *mut ACameraDevice,
    reader: *mut AImageReader,
    window: *mut ANativeWindow,
    session_output: *mut ACaptureSessionOutput,
    session_output_container: *mut ACaptureSessionOutputContainer,
    output_target: *mut ACameraOutputTarget,
    request: *mut ACaptureRequest,
    session: *mut ACameraCaptureSession,
    requested_spec: CameraSpec,
    /// degrees to rotate clockwise to get from camera's static orientation to device's native orientation. Apply this plus current phone rotation to get upright image!
    rotation: i32,
}

// SAFETY: the camera2 and media NDK's objects are used from the threads
// their callbacks run on, as upstream uses them.
unsafe impl Send for PrivateCameraData {}

/// An opened camera: its `device->hidden`, `None` once closed.
struct AndroidCameraDevice {
    driver: Arc<AndroidCamera>,
    hidden: Mutex<Option<PrivateCameraData>>,
}

impl AndroidCameraDevice {
    fn hidden(&self) -> MutexGuard<'_, Option<PrivateCameraData>> {
        self.hidden.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Translation of `SetErrorStr()`.
fn set_error_str(what: &str, errstr: Option<&str>, rc: c_int) -> Error {
    let errbuf;
    let errstr = match errstr {
        Some(errstr) => errstr,
        None => {
            errbuf = format!("Unknown error #{rc}");
            &errbuf
        }
    };
    Error::new(format!("{what}: {errstr}"))
}

/// Translation of `CameraStatusStr()`.
fn camera_status_str(rc: camera_status_t) -> Option<&'static str> {
    Some(match rc {
        ACAMERA_OK => "no error",
        ACAMERA_ERROR_UNKNOWN => "unknown error",
        ACAMERA_ERROR_INVALID_PARAMETER => "invalid parameter",
        ACAMERA_ERROR_CAMERA_DISCONNECTED => "camera disconnected",
        ACAMERA_ERROR_NOT_ENOUGH_MEMORY => "not enough memory",
        ACAMERA_ERROR_METADATA_NOT_FOUND => "metadata not found",
        ACAMERA_ERROR_CAMERA_DEVICE => "camera device error",
        ACAMERA_ERROR_CAMERA_SERVICE => "camera service error",
        ACAMERA_ERROR_SESSION_CLOSED => "session closed",
        ACAMERA_ERROR_INVALID_OPERATION => "invalid operation",
        ACAMERA_ERROR_STREAM_CONFIGURE_FAIL => "configure failure",
        ACAMERA_ERROR_CAMERA_IN_USE => "camera in use",
        ACAMERA_ERROR_MAX_CAMERA_IN_USE => "max cameras in use",
        ACAMERA_ERROR_CAMERA_DISABLED => "camera disabled",
        ACAMERA_ERROR_PERMISSION_DENIED => "permission denied",
        ACAMERA_ERROR_UNSUPPORTED_OPERATION => "unsupported operation",
        _ => return None, // unknown error
    })
}

/// Translation of `SetCameraError()`.
fn set_camera_error(what: &str, rc: camera_status_t) -> Error {
    set_error_str(what, camera_status_str(rc), rc)
}

/// Translation of `MediaStatusStr()`.
fn media_status_str(rc: media_status_t) -> Option<&'static str> {
    Some(match rc {
        AMEDIA_OK => "no error",
        AMEDIACODEC_ERROR_INSUFFICIENT_RESOURCE => "insufficient resources",
        AMEDIACODEC_ERROR_RECLAIMED => "reclaimed",
        AMEDIA_ERROR_UNKNOWN => "unknown error",
        AMEDIA_ERROR_MALFORMED => "malformed",
        AMEDIA_ERROR_UNSUPPORTED => "unsupported",
        AMEDIA_ERROR_INVALID_OBJECT => "invalid object",
        AMEDIA_ERROR_INVALID_PARAMETER => "invalid parameter",
        AMEDIA_ERROR_INVALID_OPERATION => "invalid operation",
        AMEDIA_ERROR_END_OF_STREAM => "end of stream",
        AMEDIA_ERROR_IO => "i/o error",
        AMEDIA_ERROR_WOULD_BLOCK => "operation would block",
        AMEDIA_DRM_NOT_PROVISIONED => "DRM not provisioned",
        AMEDIA_DRM_RESOURCE_BUSY => "DRM resource busy",
        AMEDIA_DRM_DEVICE_REVOKED => "DRM device revoked",
        AMEDIA_DRM_SHORT_BUFFER => "DRM short buffer",
        AMEDIA_DRM_SESSION_NOT_OPENED => "DRM session not opened",
        AMEDIA_DRM_TAMPER_DETECTED => "DRM tampering detected",
        AMEDIA_DRM_VERIFY_FAILED => "DRM verify failed",
        AMEDIA_DRM_NEED_KEY => "DRM need key",
        AMEDIA_DRM_LICENSE_EXPIRED => "DRM license expired",
        AMEDIA_IMGREADER_NO_BUFFER_AVAILABLE => "no buffer available",
        AMEDIA_IMGREADER_MAX_IMAGES_ACQUIRED => "maximum images acquired",
        AMEDIA_IMGREADER_CANNOT_LOCK_IMAGE => "cannot lock image",
        AMEDIA_IMGREADER_CANNOT_UNLOCK_IMAGE => "cannot unlock image",
        AMEDIA_IMGREADER_IMAGE_NOT_LOCKED => "image not locked",
        _ => return None, // unknown error
    })
}

/// Translation of `SetMediaError()`.
fn set_media_error(what: &str, rc: media_status_t) -> Error {
    set_error_str(what, media_status_str(rc), rc)
}

/// Translation of `format_android_to_sdl()`.
fn format_android_to_sdl(fmt: i32) -> (PixelFormat, Colorspace) {
    match fmt {
        AIMAGE_FORMAT_YUV_420_888 => (PixelFormat::NV12, Colorspace::BT709_LIMITED),
        AIMAGE_FORMAT_RGB_565 => (PixelFormat::RGB565, Colorspace::SRGB),
        AIMAGE_FORMAT_RGB_888 => (PixelFormat::XRGB8888, Colorspace::SRGB),
        AIMAGE_FORMAT_RGBA_8888 => (PixelFormat::RGBA8888, Colorspace::SRGB),
        AIMAGE_FORMAT_RGBX_8888 => (PixelFormat::RGBX8888, Colorspace::SRGB),
        AIMAGE_FORMAT_RGBA_FP16 => (PixelFormat::RGBA64_FLOAT, Colorspace::SRGB),
        _ => (PixelFormat::UNKNOWN, Colorspace::UNKNOWN),
    }
}

/// Translation of `format_sdl_to_android()`.
fn format_sdl_to_android(fmt: PixelFormat) -> i32 {
    match fmt {
        PixelFormat::NV12 => AIMAGE_FORMAT_YUV_420_888,
        PixelFormat::RGB565 => AIMAGE_FORMAT_RGB_565,
        PixelFormat::XRGB8888 => AIMAGE_FORMAT_RGB_888,
        PixelFormat::RGBA8888 => AIMAGE_FORMAT_RGBA_8888,
        PixelFormat::RGBX8888 => AIMAGE_FORMAT_RGBX_8888,
        _ => 0,
    }
}

/// The context of a camera's callbacks: its instance ID.
fn device_context(device: &CameraDevice) -> *mut c_void {
    device.instance_id as usize as *mut c_void
}

/// The camera of a callback's context.
fn context_device(context: *mut c_void) -> Option<Arc<CameraDevice>> {
    let devid = context as usize as CameraID;
    find_physical_camera_by_callback(|d| d.instance_id == devid).ok()
}

/// The open camera's backend (`device->hidden`), from the core's state.
fn device_backend(device: &CameraDevice) -> Option<Arc<dyn CameraBackend>> {
    device.lock().borrow().hidden.clone()
}

impl CameraBackend for AndroidCameraDevice {
    /// Translation of `ANDROIDCAMERA_WaitDevice()`.
    fn wait_device(&self, _device: &CameraDevice) -> bool {
        true // this isn't used atm, since we run our own thread via onImageAvailable callbacks.
    }

    /// Translation of `ANDROIDCAMERA_AcquireFrame()`.
    fn acquire_frame(&self, device: &CameraDevice) -> CameraFrameResult {
        let media = &self.driver.media;
        let (reader, hidden_rotation) = match self.hidden().as_ref() {
            Some(hidden) => (hidden.reader, hidden.rotation),
            None => return CameraFrameResult::Error,
        };
        let (frame_h, spec_format) = {
            let guard = device.lock();
            let st = guard.borrow();
            (st.actual_spec.height, st.spec.format)
        };

        let mut image: *mut AImage = ptr::null_mut();
        // SAFETY: the open camera's reader.
        let res = unsafe { (media.AImageReader_acquireNextImage)(reader, &mut image) };
        // We could also use this one:
        //res = AImageReader_acquireLatestImage(device->hidden->reader, &image);

        crate::sdl_assert!(res != AMEDIA_IMGREADER_NO_BUFFER_AVAILABLE); // we should only be here if onImageAvailable was called.

        if res != AMEDIA_OK {
            // (SetMediaError(): the message, SDL_GetError()'s upstream, goes
            // nowhere here)
            let _ = set_media_error("Error AImageReader_acquireNextImage", res);
            return CameraFrameResult::Error;
        }

        let mut atimestamp: i64 = 0;
        // SAFETY: the acquired image (until it's deleted below).
        let timestamp_ns =
            if unsafe { (media.AImage_getTimestamp)(image, &mut atimestamp) } == AMEDIA_OK {
                atimestamp as u64
            } else {
                0
            };

        // !!! FIXME: this currently copies the data to the surface (see FIXME about non-contiguous planar surfaces, but in theory we could just keep this locked until ReleaseFrame...
        let mut num_planes: i32 = 0;
        // SAFETY: as above.
        unsafe { (media.AImage_getNumberOfPlanes)(image, &mut num_planes) };

        if num_planes == 3 && spec_format == PixelFormat::NV12 {
            num_planes -= 1; // treat the interleaved planes as one.
        }

        let mut pitch: i32 = 0;
        // SAFETY: as above.
        unsafe { (media.AImage_getPlaneRowStride)(image, 0, &mut pitch) };
        let expected_len = |i: i32| -> i32 {
            if i == 0 {
                pitch * frame_h
            } else {
                pitch * (frame_h + 1) / 2
            }
        };
        let planes = num_planes.clamp(0, 3);
        let buflen: usize = (0..planes).map(|i| expected_len(i).max(0) as usize).sum();

        let mut pixels = Vec::new();
        let result = if pixels.try_reserve_exact(buflen).is_err() {
            None
        } else {
            pixels.resize(buflen, 0);
            let mut dst = 0usize;

            for i in 0..planes {
                let mut data: *mut u8 = ptr::null_mut();
                let mut datalen: c_int = 0;
                let expected = expected_len(i).max(0) as usize;
                // SAFETY: as above.
                unsafe { (media.AImage_getPlaneData)(image, i, &mut data, &mut datalen) };

                let mut row_stride: i32 = 0;
                // SAFETY: as above.
                unsafe { (media.AImage_getPlaneRowStride)(image, i, &mut row_stride) };
                crate::sdl_assert!(row_stride == pitch);
                let n = expected.min(datalen.max(0) as usize);
                if !data.is_null() && n > 0 {
                    // SAFETY: the plane holds `datalen` bytes; `pixels`
                    // has room for `expected` at `dst`.
                    unsafe { ptr::copy_nonoverlapping(data, pixels.as_mut_ptr().add(dst), n) };
                }
                dst += expected;
            }
            Some(pixels)
        };

        // SAFETY: the acquired image, deleted once.
        unsafe { (media.AImage_delete)(image) };

        let mut dev_rotation = match crate::core::android::display_current_orientation() {
            DisplayOrientation::Portrait => 0,
            DisplayOrientation::Landscape => 90,
            DisplayOrientation::PortraitFlipped => 180,
            DisplayOrientation::LandscapeFlipped => 270,
            _ => {
                crate::sdl_assert!(!"Unexpected device rotation!");
                0
            }
        };

        if device.position == CameraPosition::BackFacing {
            dev_rotation = -dev_rotation; // we want to subtract this value, instead of add, if back-facing.
        }

        let rotation = (dev_rotation + hidden_rotation) as f32; // current phone orientation, static camera orientation in relation to phone.

        match result {
            Some(pixels) => CameraFrameResult::Ready(AcquiredFrame {
                pixels,
                pitch,
                timestamp_ns,
                rotation,
            }),
            None => CameraFrameResult::Error,
        }
    }

    /// Translation of `ANDROIDCAMERA_ReleaseFrame()`.
    fn release_frame(&self, _device: &CameraDevice, pixels: Vec<u8>) {
        // !!! FIXME: this currently copies the data to the surface, but in theory we could just keep the AImage until ReleaseFrame...
        drop(pixels);
    }

    /// Translation of `ANDROIDCAMERA_CloseDevice()`.
    fn close_device(&self, _device: &CameraDevice) {
        let Some(hidden) = self.hidden().take() else {
            return;
        };
        let camera2 = &self.driver.camera2;
        let media = &self.driver.media;

        // SAFETY: the open camera's objects, each freed once, in upstream's
        // order.
        unsafe {
            if !hidden.reader.is_null() {
                (media.AImageReader_setImageListener)(hidden.reader, ptr::null_mut());
            }

            if !hidden.session.is_null() {
                (camera2.ACameraCaptureSession_close)(hidden.session);
            }

            if !hidden.request.is_null() {
                (camera2.ACaptureRequest_free)(hidden.request);
            }

            if !hidden.output_target.is_null() {
                (camera2.ACameraOutputTarget_free)(hidden.output_target);
            }

            if !hidden.session_output_container.is_null() {
                (camera2.ACaptureSessionOutputContainer_free)(hidden.session_output_container);
            }

            if !hidden.session_output.is_null() {
                (camera2.ACaptureSessionOutput_free)(hidden.session_output);
            }

            // we don't free hidden->window here, it'll be cleaned up by AImageReader_delete.

            if !hidden.reader.is_null() {
                (media.AImageReader_delete)(hidden.reader);
            }

            if !hidden.device.is_null() {
                (camera2.ACameraDevice_close)(hidden.device);
            }
        }
    }
}

/// Translation of `onImageAvailable()`.
unsafe extern "C" fn on_image_available(context: *mut c_void, _reader: *mut AImageReader) {
    let Some(device) = context_device(context) else {
        return;
    };
    // Note: SDL_CameraThreadIterate() waits for the camera's lock, which
    // closing the camera holds while it deletes this reader, and that
    // waits for this thread (see the module's documentation): the lock is
    // waited for only while the camera isn't shutting down.
    loop {
        if device.shutting_down() {
            return;
        }
        if let Some(_guard) = device.try_lock() {
            camera_thread_iterate(&device);
            return;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// Translation of `onDisconnected()`.
unsafe extern "C" fn on_disconnected(context: *mut c_void, _device: *mut ACameraDevice) {
    if let Some(device) = context_device(context) {
        camera_disconnected(&device);
    }
}

/// Translation of `onError()`.
unsafe extern "C" fn on_error(context: *mut c_void, _device: *mut ACameraDevice, _error: c_int) {
    if let Some(device) = context_device(context) {
        camera_disconnected(&device);
    }
}

/// Translation of `onClosed()`.
unsafe extern "C" fn on_closed(_context: *mut c_void, _session: *mut ACameraCaptureSession) {}

/// Translation of `onReady()`.
unsafe extern "C" fn on_ready(_context: *mut c_void, _session: *mut ACameraCaptureSession) {}

/// Translation of `onActive()`.
unsafe extern "C" fn on_active(_context: *mut c_void, _session: *mut ACameraCaptureSession) {}

/// A camera's ID (its handle).
fn device_id(device: &CameraDevice) -> &str {
    device
        .handle
        .downcast_ref::<String>()
        .map_or("", String::as_str)
}

/// this is where the "opening" of the camera happens, after permission is
/// granted. Translation of `PrepareCamera()`.
fn prepare_camera(
    driver: &AndroidCamera,
    device: &CameraDevice,
    hidden: &mut PrivateCameraData,
) -> Result<()> {
    let camera2 = &driver.camera2;
    let media = &driver.media;

    let mut dev_callbacks = ACameraDevice_StateCallbacks {
        context: device_context(device),
        onDisconnected: Some(on_disconnected),
        onError: Some(on_error),
    };

    let capture_callbacks = ACameraCaptureSession_stateCallbacks {
        context: device_context(device),
        onClosed: Some(on_closed),
        onReady: Some(on_ready),
        onActive: Some(on_active),
    };

    let mut imglistener = AImageReader_ImageListener {
        context: device_context(device),
        onImageAvailable: Some(on_image_available),
    };

    let devid = CString::new(device_id(device)).unwrap_or_default();

    hidden.rotation = 0;
    let mut metadata: *mut ACameraMetadata = ptr::null_mut();
    // SAFETY: the manager; the metadata is freed once.
    unsafe {
        if (camera2.ACameraManager_getCameraCharacteristics)(
            driver.camera_mgr,
            devid.as_ptr(),
            &mut metadata,
        ) == ACAMERA_OK
        {
            let mut orientationentry: ACameraMetadata_const_entry = std::mem::zeroed();
            if (camera2.ACameraMetadata_getConstEntry)(
                metadata,
                ACAMERA_SENSOR_ORIENTATION,
                &mut orientationentry,
            ) == ACAMERA_OK
            {
                hidden.rotation = *orientationentry.data.cast::<i32>() % 360;
            }
            (camera2.ACameraMetadata_free)(metadata);
        }
    }

    // just in case SDL_OpenCamera is overwriting device->spec as CameraPermissionCallback runs, we work from a different copy.
    let spec = hidden.requested_spec;

    // SAFETY: the NDK's calls in upstream's order, each on what the ones
    // before made; the callbacks' structures are copied by the NDK.
    unsafe {
        let res = (camera2.ACameraManager_openCamera)(
            driver.camera_mgr,
            devid.as_ptr(),
            &mut dev_callbacks,
            &mut hidden.device,
        );
        if res != ACAMERA_OK {
            return Err(set_camera_error("Failed to open camera", res));
        }
        let res2 = (media.AImageReader_new)(
            spec.width,
            spec.height,
            format_sdl_to_android(spec.format),
            10, /* nb buffers */
            &mut hidden.reader,
        );
        if res2 != AMEDIA_OK {
            return Err(set_media_error("Error AImageReader_new", res2));
        }
        let res2 = (media.AImageReader_getWindow)(hidden.reader, &mut hidden.window);
        if res2 != AMEDIA_OK {
            return Err(set_media_error("Error AImageReader_getWindow", res2));
        }
        let res = (camera2.ACaptureSessionOutput_create)(hidden.window, &mut hidden.session_output);
        if res != ACAMERA_OK {
            return Err(set_camera_error("Error ACaptureSessionOutput_create", res));
        }
        let res =
            (camera2.ACaptureSessionOutputContainer_create)(&mut hidden.session_output_container);
        if res != ACAMERA_OK {
            return Err(set_camera_error(
                "Error ACaptureSessionOutputContainer_create",
                res,
            ));
        }
        let res = (camera2.ACaptureSessionOutputContainer_add)(
            hidden.session_output_container,
            hidden.session_output,
        );
        if res != ACAMERA_OK {
            return Err(set_camera_error(
                "Error ACaptureSessionOutputContainer_add",
                res,
            ));
        }
        let res = (camera2.ACameraOutputTarget_create)(hidden.window, &mut hidden.output_target);
        if res != ACAMERA_OK {
            return Err(set_camera_error("Error ACameraOutputTarget_create", res));
        }
        let res = (camera2.ACameraDevice_createCaptureRequest)(
            hidden.device,
            TEMPLATE_RECORD,
            &mut hidden.request,
        );
        if res != ACAMERA_OK {
            return Err(set_camera_error(
                "Error ACameraDevice_createCaptureRequest",
                res,
            ));
        }
        let res = (camera2.ACaptureRequest_addTarget)(hidden.request, hidden.output_target);
        if res != ACAMERA_OK {
            return Err(set_camera_error("Error ACaptureRequest_addTarget", res));
        }
        let res = (camera2.ACameraDevice_createCaptureSession)(
            hidden.device,
            hidden.session_output_container,
            &capture_callbacks,
            &mut hidden.session,
        );
        if res != ACAMERA_OK {
            return Err(set_camera_error(
                "Error ACameraDevice_createCaptureSession",
                res,
            ));
        }
        let res = (camera2.ACameraCaptureSession_setRepeatingRequest)(
            hidden.session,
            ptr::null_mut(),
            1,
            &mut hidden.request,
            ptr::null_mut(),
        );
        if res != ACAMERA_OK {
            return Err(set_camera_error(
                "Error ACameraCaptureSession_setRepeatingRequest",
                res,
            ));
        }
        let res2 = (media.AImageReader_setImageListener)(hidden.reader, &mut imglistener);
        if res2 != AMEDIA_OK {
            return Err(set_media_error("Error AImageReader_setImageListener", res2));
        }
    }

    Ok(())
}

/// Translation of `CameraPermissionCallback()`.
fn camera_permission_callback(
    backend: &AndroidCameraDevice,
    device: &Arc<CameraDevice>,
    granted: bool,
) {
    let mut hidden = backend.hidden();
    if let Some(data) = hidden.as_mut() {
        // if device was already closed, don't send an event.
        if !granted {
            drop(hidden);
            camera_permission_outcome(device, false); // sorry, permission denied.
        } else if let Err(_e) = prepare_camera(&backend.driver, device, data) {
            // permission given? Actually open the camera now.
            drop(hidden);
            // (the error, SDL_GetError()'s upstream, goes nowhere here)
            // uhoh, setup failed; since the app thinks we already "opened" the device, mark it as disconnected and don't report the permission.
            camera_disconnected(device);
        } else {
            drop(hidden);
            // okay! We have permission to use the camera _and_ opening the hardware worked out, report that the camera is usable!
            camera_permission_outcome(device, true); // go go go!
        }
    }

    unref_physical_camera(device); // we ref'd this in OpenDevice, release the extra reference.
}

/// Translation of `GatherCameraSpecs()`: the formats, the name and the
/// position of a camera.
fn gather_camera_specs(
    driver: &AndroidCamera,
    devid: &CStr,
    position: &mut CameraPosition,
) -> (Vec<CameraSpec>, Option<String>) {
    let camera2 = &driver.camera2;
    let mut add_data = Vec::new();

    let mut metadata: *mut ACameraMetadata = ptr::null_mut();
    // SAFETY: zero is a valid entry.
    let mut cfgentry: ACameraMetadata_const_entry = unsafe { std::mem::zeroed() };
    // SAFETY: as above.
    let mut durentry: ACameraMetadata_const_entry = unsafe { std::mem::zeroed() };
    // SAFETY: as above.
    let mut infoentry: ACameraMetadata_const_entry = unsafe { std::mem::zeroed() };

    // This can fail with an "unknown error" (with `adb logcat` reporting "no such file or directory")
    // for "LEGACY" level cameras. I saw this happen on a 30-dollar budget phone I have for testing
    // (but a different brand budget phone worked, so it's not strictly the low-end of Android devices).
    // LEGACY devices are seen by onCameraAvailable, but are not otherwise accessible through
    // libcamera2ndk. The Java camera2 API apparently _can_ access these cameras, but we're going on
    // without them here for now, in hopes that such hardware is a dying breed.
    // SAFETY: the manager; the metadata is freed once, and its entries read
    // before.
    unsafe {
        if (camera2.ACameraManager_getCameraCharacteristics)(
            driver.camera_mgr,
            devid.as_ptr(),
            &mut metadata,
        ) != ACAMERA_OK
        {
            return (add_data, None); // oh well.
        } else if (camera2.ACameraMetadata_getConstEntry)(
            metadata,
            ACAMERA_SCALER_AVAILABLE_STREAM_CONFIGURATIONS,
            &mut cfgentry,
        ) != ACAMERA_OK
            || (camera2.ACameraMetadata_getConstEntry)(
                metadata,
                ACAMERA_SCALER_AVAILABLE_MIN_FRAME_DURATIONS,
                &mut durentry,
            ) != ACAMERA_OK
        {
            (camera2.ACameraMetadata_free)(metadata);
            return (add_data, None); // oh well.
        }

        let mut fullname = None;
        if (camera2.ACameraMetadata_getConstEntry)(metadata, ACAMERA_INFO_VERSION, &mut infoentry)
            == ACAMERA_OK
        {
            // (SDL_strlcpy() of `count` bytes: up to a NUL)
            let bytes =
                std::slice::from_raw_parts(infoentry.data.cast::<u8>(), infoentry.count as usize);
            let bytes = bytes.split(|&b| b == 0).next().unwrap_or_default();
            fullname = Some(String::from_utf8_lossy(bytes).into_owned());
        }

        // SAFETY: zero is a valid entry.
        let mut posentry: ACameraMetadata_const_entry = std::mem::zeroed();
        if (camera2.ACameraMetadata_getConstEntry)(metadata, ACAMERA_LENS_FACING, &mut posentry)
            == ACAMERA_OK
        {
            // ignore this if it fails.
            let facing = *posentry.data.cast::<u8>();
            if facing == ACAMERA_LENS_FACING_FRONT {
                *position = CameraPosition::FrontFacing;
                if fullname.is_none() {
                    fullname = Some("Front-facing camera".to_owned());
                }
            } else if facing == ACAMERA_LENS_FACING_BACK {
                *position = CameraPosition::BackFacing;
                if fullname.is_none() {
                    fullname = Some("Back-facing camera".to_owned());
                }
            }
        }

        if fullname.is_none() {
            fullname = Some("Generic camera".to_owned()); // we tried.
        }

        // FIXME (upstream): upstream counts `cfgentry.count` configurations,
        // four int32s each, past the entry's data; `count` is the number of
        // int32s, so here a quarter as many (see the module's
        // documentation).
        let configs = std::slice::from_raw_parts(
            cfgentry.data.cast::<i32>(),
            cfgentry.count as usize / 4 * 4,
        );
        for config in configs.chunks_exact(4) {
            let fmt = config[0];
            let w = config[1];
            let h = config[2];
            let type_ = config[3];

            if type_ == ACAMERA_SCALER_AVAILABLE_STREAM_CONFIGURATIONS_INPUT || w <= 0 || h <= 0 {
                continue;
            }
            let (sdlfmt, colorspace) = format_android_to_sdl(fmt);
            if sdlfmt == PixelFormat::UNKNOWN {
                continue;
            }

            // !!! FIXME: these all come out with 0 durations on my test phone.  :(
            add_camera_format(&mut add_data, sdlfmt, colorspace, w, h, 30, 1);
        }

        (camera2.ACameraMetadata_free)(metadata);
        (add_data, fullname)
    }
}

/// Translation of `FindAndroidCameraByID()`.
fn find_android_camera_by_id(devid: &str) -> Option<Arc<CameraDevice>> {
    find_physical_camera_by_callback(|device| device_id(device) == devid).ok()
}

/// Translation of `MaybeAddDevice()`.
fn maybe_add_device(driver: &AndroidCamera, devid: &CStr) {
    let devid_str = devid.to_string_lossy();
    if find_android_camera_by_id(&devid_str).is_some() {
        return; // already have this one.
    }

    let mut position = CameraPosition::Unknown;
    let (specs, fullname) = gather_camera_specs(driver, devid, &mut position);
    if !specs.is_empty() {
        let namecpy = devid_str.into_owned();
        add_camera(
            fullname.as_deref().unwrap_or_default(),
            position,
            &specs,
            Box::new(namecpy),
        );
    }
}

// note that camera "availability" covers both hotplugging and whether another
//  has the device opened, but for something like Android, it's probably fine
//  to treat both unplugging and loss of access as disconnection events. When
//  the other app closes the camera, we get an available event as if it was
//  just plugged back in.

/// Translation of `onCameraAvailable()`.
unsafe extern "C" fn on_camera_available(_context: *mut c_void, camera_id: *const c_char) {
    crate::sdl_assert!(!camera_id.is_null());
    let Some(driver) = driver() else {
        return;
    };
    if camera_id.is_null() {
        return;
    }
    // SAFETY: the camera manager's NUL-terminated ID.
    maybe_add_device(&driver, unsafe { CStr::from_ptr(camera_id) });
}

/// Translation of `onCameraUnavailable()`.
unsafe extern "C" fn on_camera_unavailable(_context: *mut c_void, camera_id: *const c_char) {
    crate::sdl_assert!(!camera_id.is_null());
    if camera_id.is_null() {
        return;
    }
    // SAFETY: the camera manager's NUL-terminated ID.
    let camera_id = unsafe { CStr::from_ptr(camera_id) }.to_string_lossy();

    // THIS CALLBACK FIRES WHEN YOU OPEN THE DEVICE YOURSELF.  :(
    // Make sure we don't have the device opened, in which case onDisconnected will fire instead if actually lost.
    // (with the camera's lock: see the module's documentation)
    if let Some(device) = find_android_camera_by_id(&camera_id) {
        if device_backend(&device).is_none() {
            camera_disconnected(&device);
        }
    }
}

/// The availability callbacks (`camera_availability_listener`).
struct AvailabilityListener(ACameraManager_AvailabilityCallbacks);

// SAFETY: a null context and functions.
unsafe impl Sync for AvailabilityListener {}

static CAMERA_AVAILABILITY_LISTENER: AvailabilityListener =
    AvailabilityListener(ACameraManager_AvailabilityCallbacks {
        context: ptr::null_mut(),
        onCameraAvailable: Some(on_camera_available),
        onCameraUnavailable: Some(on_camera_unavailable),
    });

impl CameraDriverImpl for Arc<AndroidCamera> {
    /// Translation of `ANDROIDCAMERA_DetectDevices()`.
    fn detect_devices(&self) {
        let camera2 = &self.camera2;
        let mut list: *mut ACameraIdList = ptr::null_mut();
        // SAFETY: the manager; the list is deleted once.
        let res = unsafe { (camera2.ACameraManager_getCameraIdList)(self.camera_mgr, &mut list) };

        if res == ACAMERA_OK && !list.is_null() {
            // SAFETY: the manager's list, which holds `numCameras` IDs.
            unsafe {
                let total = (*list).numCameras.max(0) as usize;
                for i in 0..total {
                    maybe_add_device(self, CStr::from_ptr(*(*list).cameraIds.add(i)));
                }

                (camera2.ACameraManager_deleteCameraIdList)(list);
            }
        }

        // SAFETY: the manager and the static callbacks.
        unsafe {
            (camera2.ACameraManager_registerAvailabilityCallback)(
                self.camera_mgr,
                &CAMERA_AVAILABILITY_LISTENER.0,
            )
        };
    }

    /// Translation of `ANDROIDCAMERA_OpenDevice()`.
    fn open_device(
        &self,
        device: &Arc<CameraDevice>,
        spec: &CameraSpec,
    ) -> Result<Arc<dyn CameraBackend>> {
        // !!! FIXME: for now, we'll just let this fail if it is going to fail, without checking for this
        /* Cannot open a second camera, while the first one is opened.
         * If you want to play several camera, they must all be opened first, then played.
         *
         * https://developer.android.com/reference/android/hardware/camera2/CameraManager
         * "All camera devices intended to be operated concurrently, must be opened using openCamera(String, CameraDevice.StateCallback, Handler),
         * before configuring sessions on any of the camera devices.  * "
         *
         */

        let backend = Arc::new(AndroidCameraDevice {
            driver: self.clone(),
            hidden: Mutex::new(Some(PrivateCameraData {
                device: ptr::null_mut(),
                reader: ptr::null_mut(),
                window: ptr::null_mut(),
                session_output: ptr::null_mut(),
                session_output_container: ptr::null_mut(),
                output_target: ptr::null_mut(),
                request: ptr::null_mut(),
                session: ptr::null_mut(),
                // just in case SDL_OpenCamera is overwriting device->spec as CameraPermissionCallback runs, we work from a different copy.
                requested_spec: *spec,
                rotation: 0,
            })),
        });

        ref_physical_camera(device); // ref'd until permission callback fires.

        let callback_backend = backend.clone();
        let callback_device = device.clone();
        if let Err(e) = crate::core::android::request_android_permission(
            "android.permission.CAMERA",
            Box::new(move |_permission: &str, granted: bool| {
                camera_permission_callback(&callback_backend, &callback_device, granted);
            }),
        ) {
            unref_physical_camera(device);
            return Err(e);
        }

        Ok(backend) // we don't open the camera until permission is granted, so always succeed for now.
    }

    /// Translation of `ANDROIDCAMERA_FreeDeviceHandle()`.
    fn free_device_handle(&self, _device: &CameraDevice) {
        // (the handle, the ID's copy, drops with the device)
    }

    /// Translation of `ANDROIDCAMERA_Deinitialize()`.
    fn deinitialize(&self) {
        // SAFETY: the manager and the static callbacks.
        unsafe {
            (self.camera2.ACameraManager_unregisterAvailabilityCallback)(
                self.camera_mgr,
                &CAMERA_AVAILABILITY_LISTENER.0,
            )
        };
        // (DestroyCameraManager(): once the last reference drops, this one
        // or a running callback's)
        *DRIVER.write().unwrap_or_else(|e| e.into_inner()) = None;
    }

    fn provides_own_callback_thread(&self) -> bool {
        true
    }
}

/// A library of the driver's, loaded as upstream's `dlopen()` (with the
/// log line it writes when it can't).
fn load_library(name: &str) -> Option<SharedObject> {
    match SharedObject::load(name) {
        Ok(lib) => Some(lib),
        Err(e) => {
            crate::log!("CAMERA: {name} can't be loaded: {}", e.message());
            None
        }
    }
}

/// Translation of `ANDROIDCAMERA_Init()`.
fn androidcamera_init() -> Result<Arc<dyn CameraDriverImpl>> {
    // !!! FIXME: slide this off into a subroutine
    // system libraries are in android-24 and later; we currently target android-21 and later, so check if they exist at runtime.
    let failed = || Error::new("Couldn't load the camera libraries");
    let libcamera2 = load_library("libcamera2ndk.so").ok_or_else(failed)?;
    let libmedia = load_library("libmediandk.so").ok_or_else(failed)?;

    // FIXME (upstream): upstream goes on when a function is missing (see
    // the module's documentation); here the driver doesn't start.
    let camera2 = Camera2Ndk::load(libcamera2).ok_or_else(failed)?;
    let media = MediaNdk::load(libmedia).ok_or_else(failed)?;

    // Translation of `CreateCameraManager()`.
    // SAFETY: plain call.
    let camera_mgr = unsafe { (camera2.ACameraManager_create)() };
    let driver = Arc::new(AndroidCamera {
        camera2,
        media,
        camera_mgr,
    });
    if camera_mgr.is_null() {
        return Err(Error::new("Error creating ACameraManager"));
    }

    *DRIVER.write().unwrap_or_else(|e| e.into_inner()) = Some(driver.clone());

    Ok(Arc::new(driver))
}

/// Translation of `ANDROIDCAMERA_bootstrap`.
pub(super) static ANDROIDCAMERA_BOOTSTRAP: CameraBootStrap = CameraBootStrap {
    name: "android",
    desc: "SDL Android camera driver",
    init: androidcamera_init,
    demand_only: false,
};

#[cfg(test)]
mod tests;
