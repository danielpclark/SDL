// AVFoundation, CoreMedia and CoreVideo, declared by hand for the CoreMedia
// camera driver (src/camera/coremedia/SDL_camera_coremedia.m).
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The C functions, constants and structures of the frameworks the driver
//! uses; the Objective-C classes are reached through the runtime. The
//! values were checked against the macOS SDK's headers with a C program.

use std::ffi::{c_char, c_void, CStr};

use crate::core::apple::foundation::{Boolean, CFStringRef};
use crate::core::apple::objc::Id;

/// `FourCharCode`.
pub(super) type FourCharCode = u32;
/// `OSStatus`.
pub(super) type OSStatus = i32;
/// `CMSampleBufferRef`.
pub(super) type CMSampleBufferRef = *const c_void;
/// `CMFormatDescriptionRef` (a `CMVideoFormatDescriptionRef` here).
pub(super) type CMFormatDescriptionRef = *const c_void;
/// `CVImageBufferRef` / `CVPixelBufferRef`.
pub(super) type CVPixelBufferRef = *mut c_void;
/// `CMMediaType`.
pub(super) type CMMediaType = FourCharCode;
/// `CMItemIndex`.
pub(super) type CMItemIndex = isize;
/// `CVOptionFlags` (`CVPixelBufferLockFlags`).
pub(super) type CVOptionFlags = u64;

/// `noErr`.
pub(super) const NO_ERR: OSStatus = 0;

/// `kCMMediaType_Video` (`'vide'`).
pub(super) const K_CM_MEDIA_TYPE_VIDEO: CMMediaType = fourcc(b"vide");

/// A `FourCharCode` literal: `'abcd'` packs the first character in the
/// most significant byte.
pub(super) const fn fourcc(c: &[u8; 4]) -> FourCharCode {
    u32::from_be_bytes(*c)
}

/// `kCMPixelFormat_16LE555`.
pub(super) const K_CM_PIXEL_FORMAT_16LE555: FourCharCode = fourcc(b"L555");
/// `kCMPixelFormat_16LE5551`.
pub(super) const K_CM_PIXEL_FORMAT_16LE5551: FourCharCode = fourcc(b"5551");
/// `kCMPixelFormat_16LE565`.
pub(super) const K_CM_PIXEL_FORMAT_16LE565: FourCharCode = fourcc(b"L565");
/// `kCMPixelFormat_24RGB`.
pub(super) const K_CM_PIXEL_FORMAT_24RGB: FourCharCode = 24;
/// `kCMPixelFormat_32ARGB`.
pub(super) const K_CM_PIXEL_FORMAT_32ARGB: FourCharCode = 32;
/// `kCMPixelFormat_32BGRA`.
pub(super) const K_CM_PIXEL_FORMAT_32BGRA: FourCharCode = fourcc(b"BGRA");
/// `kCMPixelFormat_422YpCbCr8`.
pub(super) const K_CM_PIXEL_FORMAT_422YPCBCR8: FourCharCode = fourcc(b"2vuy");
/// `kCMPixelFormat_422YpCbCr8_yuvs`.
pub(super) const K_CM_PIXEL_FORMAT_422YPCBCR8_YUVS: FourCharCode = fourcc(b"yuvs");
/// `kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange`.
pub(super) const K_CV_PIXEL_FORMAT_TYPE_420YPCBCR8_BIPLANAR_VIDEO_RANGE: FourCharCode =
    fourcc(b"420v");
/// `kCVPixelFormatType_420YpCbCr8BiPlanarFullRange`.
pub(super) const K_CV_PIXEL_FORMAT_TYPE_420YPCBCR8_BIPLANAR_FULL_RANGE: FourCharCode =
    fourcc(b"420f");
/// `kCVPixelFormatType_420YpCbCr10BiPlanarVideoRange`.
pub(super) const K_CV_PIXEL_FORMAT_TYPE_420YPCBCR10_BIPLANAR_VIDEO_RANGE: FourCharCode =
    fourcc(b"x420");
/// `kCVPixelFormatType_420YpCbCr10BiPlanarFullRange`.
pub(super) const K_CV_PIXEL_FORMAT_TYPE_420YPCBCR10_BIPLANAR_FULL_RANGE: FourCharCode =
    fourcc(b"xf20");

/// `AVCaptureDevicePositionUnspecified`.
pub(super) const AV_CAPTURE_DEVICE_POSITION_UNSPECIFIED: isize = 0;
/// `AVCaptureDevicePositionBack`.
pub(super) const AV_CAPTURE_DEVICE_POSITION_BACK: isize = 1;
/// `AVCaptureDevicePositionFront`.
pub(super) const AV_CAPTURE_DEVICE_POSITION_FRONT: isize = 2;

/// `AVAuthorizationStatusNotDetermined`.
pub(super) const AV_AUTHORIZATION_STATUS_NOT_DETERMINED: isize = 0;
/// `AVAuthorizationStatusAuthorized`.
pub(super) const AV_AUTHORIZATION_STATUS_AUTHORIZED: isize = 3;

/// `CMTime`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct CMTime {
    pub(super) value: i64,
    pub(super) timescale: i32,
    pub(super) flags: u32,
    pub(super) epoch: i64,
}

/// `CMSampleTimingInfo`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct CMSampleTimingInfo {
    pub(super) duration: CMTime,
    pub(super) presentation_time_stamp: CMTime,
    pub(super) decode_time_stamp: CMTime,
}

/// `CMVideoDimensions`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct CMVideoDimensions {
    pub(super) width: i32,
    pub(super) height: i32,
}

#[link(name = "CoreMedia", kind = "framework")]
unsafe extern "C" {
    pub(super) fn CMSampleBufferGetSampleTimingInfo(
        sbuf: CMSampleBufferRef,
        sample_index: CMItemIndex,
        timing_info_out: *mut CMSampleTimingInfo,
    ) -> OSStatus;
    pub(super) fn CMSampleBufferGetImageBuffer(sbuf: CMSampleBufferRef) -> CVPixelBufferRef;
    pub(super) fn CMTimeGetSeconds(time: CMTime) -> f64;
    pub(super) fn CMTimeMake(value: i64, timescale: i32) -> CMTime;
    pub(super) fn CMTimeCompare(time1: CMTime, time2: CMTime) -> i32;
    pub(super) fn CMFormatDescriptionGetMediaType(desc: CMFormatDescriptionRef) -> CMMediaType;
    pub(super) fn CMFormatDescriptionGetMediaSubType(desc: CMFormatDescriptionRef) -> FourCharCode;
    pub(super) fn CMVideoFormatDescriptionGetDimensions(
        video_desc: CMFormatDescriptionRef,
    ) -> CMVideoDimensions;
}

#[link(name = "CoreVideo", kind = "framework")]
unsafe extern "C" {
    pub(super) static kCVPixelBufferWidthKey: CFStringRef;
    pub(super) static kCVPixelBufferHeightKey: CFStringRef;
    pub(super) static kCVPixelBufferPixelFormatTypeKey: CFStringRef;

    pub(super) fn CVPixelBufferGetPlaneCount(pixel_buffer: CVPixelBufferRef) -> usize;
    pub(super) fn CVPixelBufferIsPlanar(pixel_buffer: CVPixelBufferRef) -> Boolean;
    pub(super) fn CVPixelBufferGetHeight(pixel_buffer: CVPixelBufferRef) -> usize;
    pub(super) fn CVPixelBufferGetBytesPerRow(pixel_buffer: CVPixelBufferRef) -> usize;
    pub(super) fn CVPixelBufferGetBaseAddress(pixel_buffer: CVPixelBufferRef) -> *mut c_void;
    pub(super) fn CVPixelBufferGetHeightOfPlane(
        pixel_buffer: CVPixelBufferRef,
        plane_index: usize,
    ) -> usize;
    pub(super) fn CVPixelBufferGetBytesPerRowOfPlane(
        pixel_buffer: CVPixelBufferRef,
        plane_index: usize,
    ) -> usize;
    pub(super) fn CVPixelBufferGetBaseAddressOfPlane(
        pixel_buffer: CVPixelBufferRef,
        plane_index: usize,
    ) -> *mut c_void;
    pub(super) fn CVPixelBufferLockBaseAddress(
        pixel_buffer: CVPixelBufferRef,
        lock_flags: CVOptionFlags,
    ) -> i32;
    pub(super) fn CVPixelBufferUnlockBaseAddress(
        pixel_buffer: CVPixelBufferRef,
        unlock_flags: CVOptionFlags,
    ) -> i32;
}

#[link(name = "AVFoundation", kind = "framework")]
unsafe extern "C" {
    /// `AVMediaTypeVideo` (an `NSString *`).
    pub(super) static AVMediaTypeVideo: Id;
    /// `AVCaptureSessionPresetHigh` (an `NSString *`).
    pub(super) static AVCaptureSessionPresetHigh: Id;
}

/// An `NSString *` constant that only newer systems export (upstream
/// weakly references the ones under `@available` checks), or nil.
pub(super) fn weak_ns_string_constant(name: &CStr) -> Id {
    // SAFETY: a NUL-terminated symbol name; the symbol, when present, is
    // an `NSString *const` variable.
    unsafe {
        let p = libc::dlsym(libc::RTLD_DEFAULT, name.as_ptr() as *const c_char) as *const Id;
        if p.is_null() {
            std::ptr::null_mut()
        } else {
            *p
        }
    }
}
