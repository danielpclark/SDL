// Rust translation of src/camera/coremedia/SDL_camera_coremedia.m from
// Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Apple CoreMedia camera driver: the video capture devices
//! AVFoundation finds (`AVCaptureDeviceDiscoverySession`), their formats
//! and frame rate ranges collected into specs, and one `AVCaptureSession`
//! per opened camera, whose sample buffer delegate receives frames on a
//! Grand Central Dispatch queue and runs the device thread's iteration
//! itself (`ProvidesOwnCallbackThread`). AVFoundation, CoreMedia and
//! CoreVideo are linked, as upstream links them.
//!
//! Only the macOS build is translated: the iOS device types and the UIKit
//! device rotation (`USE_UIKIT_DEVICE_ROTATION`) are left out, so frames
//! are never rotated. Like upstream, the driver has no hotplug (its
//! `FIXME`s say how it could) and never asks for camera access itself:
//! starting a capture session makes the system ask, and frames are dropped
//! until the user has decided.
//!
//! Upstream needs this in the app's Info.plist:
//!
//! ```text
//! <key>NSCameraUsageDescription</key> <string>Access camera</string>
//! ```
//!
//! and, for a sandboxed macOS app, the `com.apple.security.device.camera`
//! entitlement.

mod sys;

use std::ffi::CString;
use std::sync::{Arc, Mutex, Weak};

use self::sys::*;
use super::{
    add_camera, camera_permission_outcome, camera_thread_iterate, camera_thread_name,
    find_physical_camera_by_callback, AcquiredFrame, CameraBackend, CameraBootStrap, CameraDevice,
    CameraDriverImpl, CameraFrameResult, CameraPermissionState, CameraPosition, CameraSpec,
};
use crate::core::apple::block::{dispatch_queue_create, dispatch_release};
use crate::core::apple::foundation::{available, ns_array_items, string_from_ns};
use crate::core::apple::objc::{
    autoreleasepool, class, imp, ivar_ptr, msg_send, msg_send_super, responds_to, sel, Bool, Class,
    ClassBuilder, DeclaredClass, Id, NSInteger, NSUInteger, Sel, StrongPtr,
};
use crate::error::{Error, Result};
use crate::video::{Colorspace, PixelFormat};

/// Translation of `CoreMediaFormatToSDL()`.
fn core_media_format_to_sdl(fmt: FourCharCode) -> (PixelFormat, Colorspace) {
    match fmt {
        // the 16LE ones should use 16BE if we're on a Bigendian system like PowerPC,
        // but at current time there is no bigendian Apple platform that has CoreMedia.
        K_CM_PIXEL_FORMAT_16LE555 => (PixelFormat::XRGB1555, Colorspace::SRGB),
        K_CM_PIXEL_FORMAT_16LE5551 => (PixelFormat::RGBA5551, Colorspace::SRGB),
        K_CM_PIXEL_FORMAT_16LE565 => (PixelFormat::RGB565, Colorspace::SRGB),
        K_CM_PIXEL_FORMAT_24RGB => (PixelFormat::RGB24, Colorspace::SRGB),
        K_CM_PIXEL_FORMAT_32ARGB => (PixelFormat::ARGB32, Colorspace::SRGB),
        K_CM_PIXEL_FORMAT_32BGRA => (PixelFormat::BGRA32, Colorspace::SRGB),
        K_CM_PIXEL_FORMAT_422YPCBCR8 => (PixelFormat::UYVY, Colorspace::BT709_LIMITED),
        K_CM_PIXEL_FORMAT_422YPCBCR8_YUVS => (PixelFormat::YUY2, Colorspace::BT709_LIMITED),
        K_CV_PIXEL_FORMAT_TYPE_420YPCBCR8_BIPLANAR_VIDEO_RANGE => {
            (PixelFormat::NV12, Colorspace::BT709_LIMITED)
        }
        K_CV_PIXEL_FORMAT_TYPE_420YPCBCR8_BIPLANAR_FULL_RANGE => {
            (PixelFormat::NV12, Colorspace::BT709_FULL)
        }
        K_CV_PIXEL_FORMAT_TYPE_420YPCBCR10_BIPLANAR_VIDEO_RANGE => {
            (PixelFormat::P010, Colorspace::BT2020_LIMITED)
        }
        K_CV_PIXEL_FORMAT_TYPE_420YPCBCR10_BIPLANAR_FULL_RANGE => {
            (PixelFormat::P010, Colorspace::BT2020_FULL)
        }
        _ => {
            // (upstream logs "CAMERA: Unknown format FourCharCode '%d'"
            // when built with DEBUG_CAMERA)
            (PixelFormat::UNKNOWN, Colorspace::UNKNOWN)
        }
    }
}

/// What the driver reads from an `AVCaptureDeviceFormat`: its format
/// description's media type, subtype and dimensions, and its
/// `videoSupportedFrameRateRanges` as (`minFrameRate`, `maxFrameRate`).
#[derive(Clone, Debug, Default, PartialEq)]
struct FormatInfo {
    media_type: CMMediaType,
    sub_type: FourCharCode,
    dims: CMVideoDimensions,
    frame_rate_ranges: Vec<(f64, f64)>,
}

impl FormatInfo {
    /// Read an `AVCaptureDeviceFormat`.
    ///
    /// # Safety
    ///
    /// `fmt` must be a valid `AVCaptureDeviceFormat`, in an autorelease pool.
    unsafe fn of(fmt: Id) -> FormatInfo {
        // SAFETY: the caller's contract; the format description and the
        // ranges belong to `fmt`.
        unsafe {
            let desc: CMFormatDescriptionRef = msg_send![fmt, formatDescription];
            let ranges: Id = msg_send![fmt, videoSupportedFrameRateRanges];
            FormatInfo {
                media_type: CMFormatDescriptionGetMediaType(desc),
                sub_type: CMFormatDescriptionGetMediaSubType(desc),
                dims: CMVideoFormatDescriptionGetDimensions(desc),
                frame_rate_ranges: ns_array_items(ranges)
                    .into_iter()
                    .map(|r| {
                        let min: f64 = msg_send![r, minFrameRate];
                        let max: f64 = msg_send![r, maxFrameRate];
                        (min, max)
                    })
                    .collect(),
            }
        }
    }
}

/// The formats of a device (`device.formats`).
///
/// # Safety
///
/// `avdevice` must be a valid `AVCaptureDevice`, in an autorelease pool.
unsafe fn device_formats(avdevice: Id) -> Vec<(Id, FormatInfo)> {
    // SAFETY: the caller's contract.
    unsafe {
        let formats: Id = msg_send![avdevice, formats];
        ns_array_items(formats)
            .into_iter()
            .map(|f| (f, FormatInfo::of(f)))
            .collect()
    }
}

/// The format of `formats` that `COREMEDIA_OpenDevice()` picks for `spec`
/// (its "Pick format that matches the spec" loop), by index.
fn choose_format(formats: &[FormatInfo], spec: &CameraSpec) -> Option<usize> {
    // Pick format that matches the spec
    let w = spec.width;
    let h = spec.height;
    let rate = spec.framerate_numerator as f32 / spec.framerate_denominator as f32;
    let mut spec_format: Option<usize> = None;
    for (i, format) in formats.iter().enumerate() {
        let (device_format, device_colorspace) = core_media_format_to_sdl(format.sub_type);
        if device_format != spec.format || device_colorspace != spec.colorspace {
            continue;
        }

        let dim = format.dims;
        if dim.width != w || dim.height != h {
            continue;
        }

        const FRAMERATE_EPSILON: f32 = 0.01;
        for &(min_frame_rate, max_frame_rate) in &format.frame_rate_ranges {
            // Check if the requested rate is within the supported range
            if rate as f64 >= (min_frame_rate - FRAMERATE_EPSILON as f64)
                && rate as f64 <= (max_frame_rate + FRAMERATE_EPSILON as f64)
            {
                // Prefer formats with narrower frame rate ranges that are closer to our target
                // This helps avoid formats that support a wide range (like 10-60 FPS)
                // when we want a specific rate (like 30 FPS)
                let should_select = match spec_format {
                    None => true,
                    Some(current) => {
                        // (`firstObject` of an empty array is nil, whose
                        // frame rates read as 0)
                        let (current_min, current_max) = formats[current]
                            .frame_rate_ranges
                            .first()
                            .copied()
                            .unwrap_or((0.0, 0.0));
                        let current_range_width = (current_max - current_min) as f32;
                        let new_range_width = (max_frame_rate - min_frame_rate) as f32;

                        // Prefer formats with narrower ranges, or if ranges are similar, prefer closer to target
                        if new_range_width < current_range_width {
                            true
                        } else if (new_range_width - current_range_width).abs() < 0.1 {
                            // Similar range width, prefer the one closer to our target rate
                            let current_distance = ((rate as f64 - current_min) as f32).abs();
                            let new_distance = ((rate as f64 - min_frame_rate) as f32).abs();
                            new_distance < current_distance
                        } else {
                            false
                        }
                    }
                };

                if should_select {
                    spec_format = Some(i);
                }
            }
        }

        if spec_format.is_some() {
            break;
        }
    }
    spec_format
}

/// The specs of a device's formats. Translation of `GatherCameraSpecs()`.
fn gather_camera_specs(formats: &[FormatInfo]) -> Vec<CameraSpec> {
    let mut specs = Vec::new();

    for fmt in formats {
        if fmt.media_type != K_CM_MEDIA_TYPE_VIDEO {
            continue;
        }

        let (device_format, device_colorspace) = core_media_format_to_sdl(fmt.sub_type);
        if device_format == PixelFormat::UNKNOWN {
            continue;
        }

        let w = fmt.dims.width;
        let h = fmt.dims.height;
        for &(min_frame_rate, max_frame_rate) in &fmt.frame_rate_ranges {
            // (SDL_CalculateFraction() takes a float)
            let (min_numerator, min_denominator) =
                crate::utils::approximate_fraction(min_frame_rate as f32);
            super::add_camera_format(
                &mut specs,
                device_format,
                device_colorspace,
                w,
                h,
                min_numerator,
                min_denominator,
            );
            let (max_numerator, max_denominator) =
                crate::utils::approximate_fraction(max_frame_rate as f32);
            if max_numerator != min_numerator || max_denominator != min_denominator {
                super::add_camera_format(
                    &mut specs,
                    device_format,
                    device_colorspace,
                    w,
                    h,
                    max_numerator,
                    max_denominator,
                );
            }
        }
    }
    specs
}

/// `+[AVCaptureDevice authorizationStatusForMediaType:AVMediaTypeVideo]`.
fn video_authorization_status() -> NSInteger {
    // SAFETY: a class method taking the media type constant.
    unsafe { msg_send![class!(AVCaptureDevice), authorizationStatusForMediaType: AVMediaTypeVideo] }
}

/// Translation of `CheckCameraPermissions()`.
fn check_camera_permissions(device: &Arc<CameraDevice>) -> bool {
    if device.permission() == CameraPermissionState::Pending {
        // still expecting a permission result.
        if available!(macos(14)) {
            let status = video_authorization_status();
            if status != AV_AUTHORIZATION_STATUS_NOT_DETERMINED {
                // NotDetermined == still waiting for an answer from the user.
                camera_permission_outcome(device, status == AV_AUTHORIZATION_STATUS_AUTHORIZED);
            }
        } else {
            camera_permission_outcome(device, true); // always allowed (or just unqueryable...?) on older macOS.
        }
    }

    device.permission() > CameraPermissionState::Pending
}

/// A device's handle: its `AVCaptureDevice` (upstream's
/// `CFBridgingRetain()`ed pointer, released with the device: there is no
/// `COREMEDIA_FreeDeviceHandle()` to translate).
struct CoreMediaHandle(StrongPtr);

/// The `AVCaptureDevice` of a device of this driver.
fn handle_of(device: &CameraDevice) -> Option<Id> {
    device
        .handle
        .downcast_ref::<CoreMediaHandle>()
        .map(|h| h.0.get())
}

/// A `CMSampleBufferRef` the delegate is lending to the device thread's
/// iteration.
#[derive(Clone, Copy)]
struct SampleBuffer(CMSampleBufferRef);

// SAFETY: only a pointer, read by acquire_frame() while the delegate that
// set it waits for the iteration it runs.
unsafe impl Send for SampleBuffer {}

/// Translation of `SDLPrivateCameraData` (the opened device's state).
struct CoreMediaCamera {
    session: Mutex<StrongPtr>,
    delegate: Mutex<StrongPtr>,
    current_sample: Mutex<SampleBuffer>,
}

impl CoreMediaCamera {
    fn set_current_sample(&self, sample: CMSampleBufferRef) {
        *self
            .current_sample
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = SampleBuffer(sample);
    }

    /// `hidden.current_sample`, then `hidden.current_sample = NULL`.
    fn take_current_sample(&self) -> CMSampleBufferRef {
        let mut s = self
            .current_sample
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        std::mem::replace(&mut *s, SampleBuffer(std::ptr::null())).0
    }
}

// ---------------------------------------------------------------------------
// SDLCaptureVideoDataOutputSampleBufferDelegate

// this delegate just receives new video frames on a Grand Central Dispatch queue, and fires off the
// main device thread iterate function directly to consume it.

/// What the delegate's `device` property points at: the device, and its
/// state while it is open (`device->hidden`). Both weak, so a frame
/// arriving after the device closed or went away is ignored.
struct DelegateTarget {
    device: Weak<CameraDevice>,
    hidden: Weak<CoreMediaCamera>,
}

/// The instance variable holding a `*mut DelegateTarget`.
const DELEGATE_TARGET_IVAR: &std::ffi::CStr = c"_device";

static DELEGATE_CLASS: DeclaredClass = DeclaredClass::new();

/// `@interface SDLCaptureVideoDataOutputSampleBufferDelegate :
/// NSObject<AVCaptureVideoDataOutputSampleBufferDelegate>`.
fn delegate_class() -> Class {
    DELEGATE_CLASS.get(|| {
        let mut b = ClassBuilder::new(
            c"SDLCaptureVideoDataOutputSampleBufferDelegate",
            class!(NSObject),
        )
        .expect("SDLCaptureVideoDataOutputSampleBufferDelegate is declared once");
        b.add_protocol(c"AVCaptureVideoDataOutputSampleBufferDelegate");
        b.add_ivar::<*mut DelegateTarget>(DELEGATE_TARGET_IVAR, c"^v");
        // SAFETY: each function has the signature its type encoding says.
        unsafe {
            b.add_method(
                sel!(captureOutput:didOutputSampleBuffer:fromConnection:),
                imp!(
                    delegate_did_output_sample_buffer
                        as extern "C" fn(Id, Sel, Id, CMSampleBufferRef, Id)
                ),
                c"v@:@^{opaqueCMSampleBuffer=}@",
            );
            b.add_method(
                sel!(captureOutput:didDropSampleBuffer:fromConnection:),
                imp!(
                    delegate_did_drop_sample_buffer
                        as extern "C" fn(Id, Sel, Id, CMSampleBufferRef, Id)
                ),
                c"v@:@^{opaqueCMSampleBuffer=}@",
            );
            b.add_method(
                sel!(dealloc),
                imp!(delegate_dealloc as extern "C" fn(Id, Sel)),
                c"v@:",
            );
        }
        b.register()
    })
}

/// The delegate's target slot.
///
/// # Safety
///
/// `this` must be an `SDLCaptureVideoDataOutputSampleBufferDelegate`.
unsafe fn delegate_target_slot(this: Id) -> *mut *mut DelegateTarget {
    // SAFETY: the caller's contract; the ivar is declared on the class.
    unsafe { ivar_ptr::<*mut DelegateTarget>(this, delegate_class(), DELEGATE_TARGET_IVAR) }
}

/// Translation of `-[SDLCaptureVideoDataOutputSampleBufferDelegate init:]`:
/// a delegate for `device`, whose state is `hidden` (owned).
fn new_delegate(target: DelegateTarget) -> StrongPtr {
    // SAFETY: allocating and initializing an instance of the class
    // declared above, then setting its instance variable.
    unsafe {
        let obj: Id = msg_send![delegate_class(), alloc];
        let obj: Id = msg_send![obj, init];
        if !obj.is_null() {
            *delegate_target_slot(obj) = Box::into_raw(Box::new(target));
        }
        StrongPtr::from_owned(obj)
    }
}

/// Translation of `-[SDLCaptureVideoDataOutputSampleBufferDelegate
/// captureOutput:didOutputSampleBuffer:fromConnection:]`.
extern "C" fn delegate_did_output_sample_buffer(
    this: Id,
    _sel: Sel,
    _output: Id,
    sample_buffer: CMSampleBufferRef,
    _connection: Id,
) {
    autoreleasepool(|| {
        // SAFETY: called on our delegate; the target lives until dealloc.
        let target = unsafe { *delegate_target_slot(this) };
        if target.is_null() {
            return;
        }
        // SAFETY: as above.
        let target = unsafe { &*target };
        let (Some(device), Some(hidden)) = (target.device.upgrade(), target.hidden.upgrade())
        else {
            return; // oh well.
        };
        // (upstream checks `device->hidden` here; the device being closed
        // has it until its CloseDevice ran, so a closing device is
        // skipped as well instead of waiting for the lock its closer
        // holds.)
        if device.shutting_down() {
            return;
        }

        if !check_camera_permissions(&device) {
            return; // nothing to do right now, dump what is probably a completely black frame.
        }

        hidden.set_current_sample(sample_buffer);
        camera_thread_iterate(&device);
        hidden.set_current_sample(std::ptr::null());
    });
}

/// Translation of `-[SDLCaptureVideoDataOutputSampleBufferDelegate
/// captureOutput:didDropSampleBuffer:fromConnection:]`.
extern "C" fn delegate_did_drop_sample_buffer(
    _this: Id,
    _sel: Sel,
    _output: Id,
    _sample_buffer: CMSampleBufferRef,
    _connection: Id,
) {
    // (upstream logs "CAMERA: Drop frame." when built with DEBUG_CAMERA)
}

/// `-dealloc` (ARC's, freeing the delegate's target).
extern "C" fn delegate_dealloc(this: Id, _sel: Sel) {
    // SAFETY: our delegate being freed: its target is reclaimed once, then
    // NSObject's dealloc runs.
    unsafe {
        let slot = delegate_target_slot(this);
        if !(*slot).is_null() {
            drop(Box::from_raw(*slot));
            *slot = std::ptr::null_mut();
        }
        let _: () = msg_send_super![this, delegate_class(), dealloc];
    }
}

// ---------------------------------------------------------------------------

impl CameraBackend for CoreMediaCamera {
    fn wait_device(&self, _device: &CameraDevice) -> bool {
        // Translation of `COREMEDIA_WaitDevice()`.
        true // this isn't used atm, since we run our own thread out of Grand Central Dispatch.
    }

    fn acquire_frame(&self, _device: &CameraDevice) -> CameraFrameResult {
        // Translation of `COREMEDIA_AcquireFrame()`.
        let sample_buffer = self.take_current_sample();
        crate::sdl_assert!(!sample_buffer.is_null()); // should only have been called from our delegate with a new frame.
        if sample_buffer.is_null() {
            // (upstream would go on with a NULL buffer)
            return CameraFrameResult::Skip;
        }

        let mut timinginfo = CMSampleTimingInfo::default();
        // SAFETY: the delegate's sample buffer, valid for its callback,
        // which is waiting for this; the timing info is written once.
        let timestamp_ns =
            if unsafe { CMSampleBufferGetSampleTimingInfo(sample_buffer, 0, &mut timinginfo) }
                == NO_ERR
            {
                // SAFETY: a CMTime by value.
                (unsafe { CMTimeGetSeconds(timinginfo.presentation_time_stamp) }
                    * crate::timer::NS_PER_SECOND as f64) as u64
            } else {
                crate::sdl_assert!(!"this shouldn't happen, I think.");
                0
            };

        // SAFETY: as above; the image buffer belongs to the sample buffer
        // (it is not retained, and we don't want it to be), and is read
        // between its lock and unlock.
        unsafe {
            let image = CMSampleBufferGetImageBuffer(sample_buffer); // does not retain `image` (and we don't want it to).
            let num_planes = CVPixelBufferGetPlaneCount(image) as i32;
            let planar = CVPixelBufferIsPlanar(image) as i32;

            // (upstream logs "CAMERA: buffer planar=%d numPlanes=%d %d x %d
            // sz=%d pitch=%d" when built with DEBUG_CAMERA)

            // !!! FIXME: this currently copies the data to the surface (see FIXME about non-contiguous planar surfaces, but in theory we could just keep this locked until ReleaseFrame...
            CVPixelBufferLockBaseAddress(image, 0);

            // (upstream sets the frame surface's w and h from the buffer;
            // the camera core here sizes frames from the opened spec, which
            // the output's video settings asked the buffers to have.)
            let h = CVPixelBufferGetHeight(image) as i32;

            let mut result = None;
            let mut pixels = Vec::new();
            let pitch;
            if planar == 0 && num_planes == 0 {
                pitch = CVPixelBufferGetBytesPerRow(image) as i32;
                let buflen = pitch as usize * h as usize;
                if pixels.try_reserve_exact(buflen).is_err() {
                    result = Some(CameraFrameResult::Error);
                } else {
                    let src = CVPixelBufferGetBaseAddress(image) as *const u8;
                    pixels.extend_from_slice(std::slice::from_raw_parts(src, buflen));
                }
            } else {
                // !!! FIXME: we have an open issue in SDL3 to allow SDL_Surface to support non-contiguous planar data, but we don't have it yet.
                let mut buflen = 0usize;
                for i in 0..num_planes as usize {
                    let plane_height = CVPixelBufferGetHeightOfPlane(image, i);
                    let plane_pitch = CVPixelBufferGetBytesPerRowOfPlane(image, i);
                    let plane_size = plane_pitch * plane_height;
                    buflen += plane_size;
                }

                pitch = CVPixelBufferGetBytesPerRowOfPlane(image, 0) as i32; // this is what SDL3 currently expects
                if pixels.try_reserve_exact(buflen).is_err() {
                    result = Some(CameraFrameResult::Error);
                } else {
                    for i in 0..num_planes as usize {
                        let src = CVPixelBufferGetBaseAddressOfPlane(image, i) as *const u8;
                        let plane_height = CVPixelBufferGetHeightOfPlane(image, i);
                        let plane_pitch = CVPixelBufferGetBytesPerRowOfPlane(image, i);
                        let plane_size = plane_pitch * plane_height;
                        pixels.extend_from_slice(std::slice::from_raw_parts(src, plane_size));
                    }
                }
            }

            CVPixelBufferUnlockBaseAddress(image, 0);

            result.unwrap_or(CameraFrameResult::Ready(AcquiredFrame {
                pixels,
                pitch,
                timestamp_ns,
                rotation: 0.0, // (USE_UIKIT_DEVICE_ROTATION is iOS only)
            }))
        }
    }

    fn release_frame(&self, _device: &CameraDevice, pixels: Vec<u8>) {
        // Translation of `COREMEDIA_ReleaseFrame()`.
        // !!! FIXME: this currently copies the data to the surface, but in theory we could just keep this locked until ReleaseFrame...
        drop(pixels);
    }

    fn close_device(&self, _device: &CameraDevice) {
        // Translation of `COREMEDIA_CloseDevice()`.
        autoreleasepool(|| {
            let session =
                std::mem::take(&mut *self.session.lock().unwrap_or_else(|e| e.into_inner()));
            if !session.is_nil() {
                // SAFETY: our running session, with the one input and the
                // one output COREMEDIA_OpenDevice() added (upstream indexes
                // them unchecked, which would throw on an empty array).
                unsafe {
                    let _: () = msg_send![session, stopRunning];
                    let inputs: Id = msg_send![session, inputs];
                    if let Some(&input) = ns_array_items(inputs).first() {
                        let _: () = msg_send![session, removeInput: input];
                    }
                    let outputs: Id = msg_send![session, outputs];
                    if let Some(&output) = ns_array_items(outputs).first() {
                        let _: () = msg_send![session, removeOutput: output];
                    }
                }
                drop(session);
            }

            drop(std::mem::take(
                &mut *self.delegate.lock().unwrap_or_else(|e| e.into_inner()),
            ));
            self.set_current_sample(std::ptr::null());
        });
    }
}

/// Whether `duration` lies in one of `format`'s frame rate ranges (between
/// their `minFrameDuration` and `maxFrameDuration`), the values the
/// `activeVideoMinFrameDuration`/`activeVideoMaxFrameDuration` setters
/// accept.
///
/// Upstream sets those in `@try` blocks and ignores the exception an
/// unsupported value raises ("Some devices don't support setting frame
/// duration, that's okay"); an Objective-C exception can't be caught here,
/// so the value is checked first and not set when it would be refused.
///
/// # Safety
///
/// `format` must be a valid `AVCaptureDeviceFormat`, in an autorelease pool.
unsafe fn frame_duration_supported(format: Id, duration: CMTime) -> bool {
    // SAFETY: the caller's contract.
    unsafe {
        let ranges: Id = msg_send![format, videoSupportedFrameRateRanges];
        ns_array_items(ranges).into_iter().any(|r| {
            let min: CMTime = msg_send![r, minFrameDuration];
            let max: CMTime = msg_send![r, maxFrameDuration];
            CMTimeCompare(duration, min) >= 0 && CMTimeCompare(duration, max) <= 0
        })
    }
}

/// Set the device's frame duration bounds to `frame_duration` (the `@try`
/// blocks of `COREMEDIA_OpenDevice()`), if the active format supports it.
///
/// # Safety
///
/// `avdevice` must be a valid `AVCaptureDevice`, locked for configuration,
/// in an autorelease pool.
unsafe fn set_frame_duration(avdevice: Id, frame_duration: CMTime) {
    // SAFETY: the caller's contract.
    unsafe {
        let active: Id = msg_send![avdevice, activeFormat];
        if frame_duration_supported(active, frame_duration) {
            let _: () = msg_send![avdevice, setActiveVideoMinFrameDuration: frame_duration];
            let _: () = msg_send![avdevice, setActiveVideoMaxFrameDuration: frame_duration];
        }
        // else: Some devices don't support setting frame duration, that's okay
    }
}

/// Translation of `COREMEDIA_OpenDevice()` (in an autorelease pool).
fn open_device(device: &Arc<CameraDevice>, spec: &CameraSpec) -> Result<Arc<CoreMediaCamera>> {
    let avdevice = handle_of(device).ok_or_else(|| Error::new("Not a CoreMedia device"))?;

    // SAFETY: the device's AVCaptureDevice, retained by its handle; every
    // object below is used while it is owned or autoreleased in the pool
    // the caller set up.
    unsafe {
        // Pick format that matches the spec
        let formats = device_formats(avdevice);
        let infos: Vec<FormatInfo> = formats.iter().map(|(_, info)| info.clone()).collect();
        let Some(chosen) = choose_format(&infos, spec) else {
            return Err(Error::new("camera spec format not available"));
        };
        let (spec_format, spec_format_info) = &formats[chosen];
        let spec_format = *spec_format;

        let locked: Bool = msg_send![avdevice, lockForConfiguration: std::ptr::null_mut::<Id>()];
        if !locked.is_true() {
            return Err(Error::new("Cannot lockForConfiguration"));
        }

        let _: () = msg_send![avdevice, setActiveFormat: spec_format];

        // Try to set the frame duration to enforce the requested frame rate
        let frame_duration =
            CMTimeMake(spec.framerate_denominator as i64, spec.framerate_numerator);

        // Check if the device supports setting frame duration
        if responds_to(avdevice, sel!(setActiveVideoMinFrameDuration:))
            && responds_to(avdevice, sel!(setActiveVideoMaxFrameDuration:))
        {
            set_frame_duration(avdevice, frame_duration);
        }

        let _: () = msg_send![avdevice, unlockForConfiguration];

        let session: Id = msg_send![class!(AVCaptureSession), alloc];
        let session = StrongPtr::from_owned(msg_send![session, init]);
        if session.is_nil() {
            return Err(Error::new("Failed to allocate/init AVCaptureSession"));
        }

        let _: () = msg_send![session, setSessionPreset: AVCaptureSessionPresetHigh];
        // (the iOS build turns off automaticallyConfiguresCaptureDeviceForWideColor)

        let mut error: Id = std::ptr::null_mut();
        let input: Id = msg_send![class!(AVCaptureDeviceInput), deviceInputWithDevice: avdevice
                                                                error: &mut error as *mut Id];
        if input.is_null() {
            return Err(Error::new("Cannot create AVCaptureDeviceInput"));
        }

        let output: Id = msg_send![class!(AVCaptureVideoDataOutput), alloc];
        let output = StrongPtr::from_owned(msg_send![output, init]);
        if output.is_nil() {
            return Err(Error::new("Cannot create AVCaptureVideoDataOutput"));
        }

        let keys: [Id; 3] = [
            kCVPixelBufferWidthKey as Id,
            kCVPixelBufferHeightKey as Id,
            kCVPixelBufferPixelFormatTypeKey as Id,
        ];
        let values: [Id; 3] = [
            msg_send![class!(NSNumber), numberWithInt: spec.width],
            msg_send![class!(NSNumber), numberWithInt: spec.height],
            msg_send![class!(NSNumber), numberWithUnsignedInt: spec_format_info.sub_type],
        ];
        let settings: Id = msg_send![class!(NSDictionary), dictionaryWithObjects: values.as_ptr()
                                                           forKeys: keys.as_ptr()
                                                           count: 3 as NSUInteger];
        let _: () = msg_send![output, setVideoSettings: settings];

        let threadname = CString::new(camera_thread_name(device)).unwrap_or_default();
        let queue = dispatch_queue_create(threadname.as_ptr(), std::ptr::null());
        //dispatch_queue_t queue = dispatch_get_global_queue(DISPATCH_QUEUE_PRIORITY_DEFAULT, 0);
        if queue.is_null() {
            return Err(Error::new("dispatch_queue_create() failed"));
        }

        let hidden = Arc::new(CoreMediaCamera {
            session: Mutex::new(StrongPtr::nil()),
            delegate: Mutex::new(StrongPtr::nil()),
            current_sample: Mutex::new(SampleBuffer(std::ptr::null())),
        });

        let delegate = new_delegate(DelegateTarget {
            device: Arc::downgrade(device),
            hidden: Arc::downgrade(&hidden),
        });
        if delegate.is_nil() {
            dispatch_release(queue);
            return Err(Error::new(
                "Cannot create SDLCaptureVideoDataOutputSampleBufferDelegate",
            ));
        }
        let _: () = msg_send![output, setSampleBufferDelegate: delegate.get() queue: queue];
        // (the output retains the queue; ARC releases upstream's reference
        // at the end of the function)
        dispatch_release(queue);

        let can: Bool = msg_send![session, canAddInput: input];
        if !can.is_true() {
            return Err(Error::new("Cannot add AVCaptureDeviceInput"));
        }
        let _: () = msg_send![session, addInput: input];

        let can: Bool = msg_send![session, canAddOutput: output.get()];
        if !can.is_true() {
            return Err(Error::new("Cannot add AVCaptureVideoDataOutput"));
        }
        let _: () = msg_send![session, addOutput: output.get()];

        // Try to set the frame rate on the device (preferred modern approach)
        let locked: Bool = msg_send![avdevice, lockForConfiguration: std::ptr::null_mut::<Id>()];
        if locked.is_true() {
            set_frame_duration(avdevice, frame_duration);
            let _: () = msg_send![avdevice, unlockForConfiguration];
        }

        let _: () = msg_send![session, commitConfiguration];

        *hidden.session.lock().unwrap_or_else(|e| e.into_inner()) = session.clone();
        *hidden.delegate.lock().unwrap_or_else(|e| e.into_inner()) = delegate;
        hidden.set_current_sample(std::ptr::null());

        // (USE_UIKIT_DEVICE_ROTATION's orientation tracking is iOS only)

        let _: () = msg_send![session, startRunning]; // !!! FIXME: docs say this can block while camera warms up and shouldn't be done on main thread. Maybe push through `queue`?

        check_camera_permissions(device); // check right away, in case the process is already granted permission.

        Ok(hidden)
    }
}

/// Translation of `MaybeAddDevice()`.
///
/// # Safety
///
/// `avdevice` must be a valid `AVCaptureDevice`, in an autorelease pool.
unsafe fn maybe_add_device(avdevice: Id) {
    // SAFETY: the caller's contract.
    unsafe {
        let connected: Bool = msg_send![avdevice, isConnected];
        let has_video: Bool = msg_send![avdevice, hasMediaType: AVMediaTypeVideo];
        let uniqueid: Id = msg_send![avdevice, uniqueID];
        if !connected.is_true() {
            return; // not connected.
        } else if !has_video.is_true() {
            return; // not a camera.
        } else if find_physical_camera_by_callback(|d| {
            handle_of(d).is_some_and(|avdev| {
                let other: Id = msg_send![avdev, uniqueID];
                let same: Bool = msg_send![uniqueid, isEqualToString: other];
                same.is_true()
            })
        })
        .is_ok()
        {
            return; // already have this one.
        }

        let formats: Vec<FormatInfo> = device_formats(avdevice)
            .into_iter()
            .map(|(_, info)| info)
            .collect();
        let specs = gather_camera_specs(&formats);
        if !specs.is_empty() {
            let pos: NSInteger = msg_send![avdevice, position];
            let position = if pos == AV_CAPTURE_DEVICE_POSITION_FRONT {
                CameraPosition::FrontFacing
            } else if pos == AV_CAPTURE_DEVICE_POSITION_BACK {
                CameraPosition::BackFacing
            } else {
                CameraPosition::Unknown
            };
            let name: Id = msg_send![avdevice, localizedName];
            let name = string_from_ns(name).unwrap_or_default();
            add_camera(
                &name,
                position,
                &specs,
                Box::new(CoreMediaHandle(StrongPtr::retain(avdevice))),
            );
        }
    }
}

/// The video capture devices (`COREMEDIA_DetectDevices()`'s list).
///
/// # Safety
///
/// Call in an autorelease pool; the devices are autoreleased.
unsafe fn video_devices() -> Vec<Id> {
    // SAFETY: class methods of AVFoundation with their arguments; the
    // arrays are autoreleased into the caller's pool.
    unsafe {
        let devices: Id = if available!(macos(10, 15), ios(13)) {
            // kind of annoying that there isn't a "give me anything that looks like a camera" option,
            // so this list will need to be updated when Apple decides to add
            // AVCaptureDeviceTypeBuiltInQuadrupleCamera some day.
            // (the iOS build lists its telephoto, dual, triple and ultra
            // wide cameras here instead of external ones)
            let device_types: Vec<Id> = [
                weak_ns_string_constant(c"AVCaptureDeviceTypeExternalUnknown"),
                weak_ns_string_constant(c"AVCaptureDeviceTypeBuiltInWideAngleCamera"),
            ]
            .into_iter()
            .filter(|t| !t.is_null())
            .collect();
            let device_types: Id = msg_send![class!(NSArray), arrayWithObjects: device_types.as_ptr()
                                                             count: device_types.len() as NSUInteger];

            let discovery_session: Id = msg_send![class!(AVCaptureDeviceDiscoverySession),
                discoverySessionWithDeviceTypes: device_types
                mediaType: AVMediaTypeVideo
                position: AV_CAPTURE_DEVICE_POSITION_UNSPECIFIED];

            // !!! FIXME: this can use Key Value Observation to get hotplug events.
            msg_send![discovery_session, devices]
        } else {
            // this is deprecated but works back to macOS 10.7; 10.15 added AVCaptureDeviceDiscoverySession as a replacement.
            // !!! FIXME: this can use AVCaptureDeviceWasConnectedNotification and AVCaptureDeviceWasDisconnectedNotification with NSNotificationCenter to get hotplug events.
            msg_send![class!(AVCaptureDevice), devicesWithMediaType: AVMediaTypeVideo]
        };
        ns_array_items(devices)
    }
}

/// The driver (`COREMEDIA_Init()`'s function table).
struct CoreMediaDriver;

impl CameraDriverImpl for CoreMediaDriver {
    fn detect_devices(&self) {
        // Translation of `COREMEDIA_DetectDevices()`.
        autoreleasepool(|| {
            // SAFETY: in a pool; each device is valid while it lasts.
            unsafe {
                for device in video_devices() {
                    maybe_add_device(device);
                }
            }
        });
    }

    fn open_device(
        &self,
        device: &Arc<CameraDevice>,
        spec: &CameraSpec,
    ) -> Result<Arc<dyn CameraBackend>> {
        let hidden = autoreleasepool(|| open_device(device, spec))?;
        Ok(hidden)
    }

    fn deinitialize(&self) {
        // Translation of `COREMEDIA_Deinitialize()`.
        // !!! FIXME: disable hotplug.
    }

    fn provides_own_callback_thread(&self) -> bool {
        true
    }
}

/// Translation of `COREMEDIA_Init()`.
fn coremedia_init() -> Result<Arc<dyn CameraDriverImpl>> {
    Ok(Arc::new(CoreMediaDriver))
}

/// Translation of `COREMEDIA_bootstrap`.
pub(super) static COREMEDIA_BOOTSTRAP: CameraBootStrap = CameraBootStrap {
    name: "coremedia",
    desc: "SDL Apple CoreMedia camera driver",
    init: coremedia_init,
    demand_only: false,
};

#[cfg(test)]
mod tests;
