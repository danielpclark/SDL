// Rust declarations of the parts of xf86drm.h, xf86drmMode.h, gbm.h and
// drm_fourcc.h (with the kernel's drm.h and drm_mode.h values they carry)
// used by the KMS/DRM video driver of Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The C types and constants of libdrm and libgbm, which the KMS/DRM driver
//! loads at run time, declared by hand (the function pointers are in
//! [`super::kmsdrmdyn`]). The layouts are those of libdrm 2.4.125 and Mesa 25.2's
//! gbm.h; the tests below pin them to the sizes and offsets a C program
//! built against those headers printed (on x86-64 Linux; aarch64 Linux has
//! the same LP64 layouts). On 32-bit Linux the pointers are 4 bytes and the
//! structures holding them are smaller: the declarations are written for
//! both (pointer fields as pointers), the pinned numbers are for 64-bit.

#![allow(non_camel_case_types)] // (the C names are kept)
#![allow(non_snake_case)] // (mmWidth, mmHeight)
#![allow(dead_code)] // (the declarations follow the headers)

use std::ffi::{c_char, c_int, c_uint, c_void};

// ---------------------------------------------------------------------------
// xf86drm.h
// ---------------------------------------------------------------------------

/// `drm_magic_t` (drm.h: `unsigned int`).
pub(crate) type drm_magic_t = c_uint;

/// `DRM_EVENT_CONTEXT_VERSION` (libdrm 2.4.125: 4, with
/// `sequence_handler`).
pub(crate) const DRM_EVENT_CONTEXT_VERSION: c_int = 4;

/// The vblank and page flip handlers of `drmEventContext`.
pub(crate) type drm_vblank_handler_t = Option<
    unsafe extern "C" fn(
        fd: c_int,
        sequence: c_uint,
        tv_sec: c_uint,
        tv_usec: c_uint,
        user_data: *mut c_void,
    ),
>;
/// `page_flip_handler2` of `drmEventContext` (with the CRTC).
pub(crate) type drm_page_flip_handler2_t = Option<
    unsafe extern "C" fn(
        fd: c_int,
        sequence: c_uint,
        tv_sec: c_uint,
        tv_usec: c_uint,
        crtc_id: c_uint,
        user_data: *mut c_void,
    ),
>;
/// `sequence_handler` of `drmEventContext`.
pub(crate) type drm_sequence_handler_t =
    Option<unsafe extern "C" fn(fd: c_int, sequence: u64, ns: u64, user_data: u64)>;

/// `drmEventContext`: the handlers `drmHandleEvent()` calls.
#[repr(C)]
#[derive(Default)]
pub(crate) struct drmEventContext {
    /// This struct is versioned so we can add more pointers if we
    /// add more events.
    pub(crate) version: c_int,
    pub(crate) vblank_handler: drm_vblank_handler_t,
    pub(crate) page_flip_handler: drm_vblank_handler_t,
    pub(crate) page_flip_handler2: drm_page_flip_handler2_t,
    pub(crate) sequence_handler: drm_sequence_handler_t,
}

// ---------------------------------------------------------------------------
// drm.h / drm_mode.h values (as xf86drm.h and xf86drmMode.h have them)
// ---------------------------------------------------------------------------

pub(crate) const DRM_CAP_ASYNC_PAGE_FLIP: u64 = 0x7;
pub(crate) const DRM_CAP_CURSOR_WIDTH: u64 = 0x8;
pub(crate) const DRM_CAP_CURSOR_HEIGHT: u64 = 0x9;

pub(crate) const DRM_CLIENT_CAP_UNIVERSAL_PLANES: u64 = 2;
pub(crate) const DRM_CLIENT_CAP_ATOMIC: u64 = 3;

pub(crate) const DRM_MODE_FLAG_INTERLACE: u32 = 1 << 4;
pub(crate) const DRM_MODE_FLAG_DBLSCAN: u32 = 1 << 5;
pub(crate) const DRM_MODE_TYPE_PREFERRED: u32 = 1 << 3;

pub(crate) const DRM_MODE_PROP_ENUM: u32 = 1 << 3;

pub(crate) const DRM_MODE_OBJECT_CRTC: u32 = 0xcccccccc;
pub(crate) const DRM_MODE_OBJECT_CONNECTOR: u32 = 0xc0c0c0c0;
pub(crate) const DRM_MODE_OBJECT_PLANE: u32 = 0xeeeeeeee;

pub(crate) const DRM_PLANE_TYPE_OVERLAY: u32 = 0;
pub(crate) const DRM_PLANE_TYPE_PRIMARY: u32 = 1;
pub(crate) const DRM_PLANE_TYPE_CURSOR: u32 = 2;

pub(crate) const DRM_MODE_PAGE_FLIP_EVENT: u32 = 0x01;
pub(crate) const DRM_MODE_PAGE_FLIP_ASYNC: u32 = 0x02;

pub(crate) const DRM_MODE_ATOMIC_NONBLOCK: u32 = 0x0200;
pub(crate) const DRM_MODE_ATOMIC_ALLOW_MODESET: u32 = 0x0400;

pub(crate) const DRM_MODE_FB_MODIFIERS: u32 = 1 << 1;

// ---------------------------------------------------------------------------
// xf86drmMode.h
// ---------------------------------------------------------------------------

pub(crate) const DRM_DISPLAY_MODE_LEN: usize = 32;
pub(crate) const DRM_PROP_NAME_LEN: usize = 32;

/// `drmModeConnection`.
pub(crate) type drmModeConnection = c_int;
pub(crate) const DRM_MODE_CONNECTED: drmModeConnection = 1;
pub(crate) const DRM_MODE_DISCONNECTED: drmModeConnection = 2;
pub(crate) const DRM_MODE_UNKNOWNCONNECTION: drmModeConnection = 3;

/// `drmModeSubPixel`.
pub(crate) type drmModeSubPixel = c_int;

/// `drmModeRes`: the resources of a card.
#[repr(C)]
pub(crate) struct drmModeRes {
    pub(crate) count_fbs: c_int,
    pub(crate) fbs: *mut u32,
    pub(crate) count_crtcs: c_int,
    pub(crate) crtcs: *mut u32,
    pub(crate) count_connectors: c_int,
    pub(crate) connectors: *mut u32,
    pub(crate) count_encoders: c_int,
    pub(crate) encoders: *mut u32,
    pub(crate) min_width: u32,
    pub(crate) max_width: u32,
    pub(crate) min_height: u32,
    pub(crate) max_height: u32,
}

/// `drmModeModeInfo` (`struct drm_mode_modeinfo`).
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct drmModeModeInfo {
    pub(crate) clock: u32,
    pub(crate) hdisplay: u16,
    pub(crate) hsync_start: u16,
    pub(crate) hsync_end: u16,
    pub(crate) htotal: u16,
    pub(crate) hskew: u16,
    pub(crate) vdisplay: u16,
    pub(crate) vsync_start: u16,
    pub(crate) vsync_end: u16,
    pub(crate) vtotal: u16,
    pub(crate) vscan: u16,
    pub(crate) vrefresh: u32,
    pub(crate) flags: u32,
    pub(crate) type_: u32,
    pub(crate) name: [c_char; DRM_DISPLAY_MODE_LEN],
}

impl drmModeModeInfo {
    /// A zeroed mode (`SDL_zero()`, or a CRTC's mode when it has none).
    pub(crate) const fn zeroed() -> drmModeModeInfo {
        drmModeModeInfo {
            clock: 0,
            hdisplay: 0,
            hsync_start: 0,
            hsync_end: 0,
            htotal: 0,
            hskew: 0,
            vdisplay: 0,
            vsync_start: 0,
            vsync_end: 0,
            vtotal: 0,
            vscan: 0,
            vrefresh: 0,
            flags: 0,
            type_: 0,
            name: [0; DRM_DISPLAY_MODE_LEN],
        }
    }
}

/// `drmModeFB`.
#[repr(C)]
pub(crate) struct drmModeFB {
    pub(crate) fb_id: u32,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) pitch: u32,
    pub(crate) bpp: u32,
    pub(crate) depth: u32,
    /// driver specific handle
    pub(crate) handle: u32,
}

/// `drmModeFB2`.
#[repr(C)]
pub(crate) struct drmModeFB2 {
    pub(crate) fb_id: u32,
    pub(crate) width: u32,
    pub(crate) height: u32,
    /// fourcc code from drm_fourcc.h
    pub(crate) pixel_format: u32,
    /// applies to all buffers
    pub(crate) modifier: u64,
    pub(crate) flags: u32,
    /// per-plane GEM handle; may be duplicate entries for multiple planes
    pub(crate) handles: [u32; 4],
    /// bytes
    pub(crate) pitches: [u32; 4],
    /// bytes
    pub(crate) offsets: [u32; 4],
}

/// `drmModePropertyBlobRes`.
#[repr(C)]
pub(crate) struct drmModePropertyBlobRes {
    pub(crate) id: u32,
    pub(crate) length: u32,
    pub(crate) data: *mut c_void,
}

/// `struct drm_mode_property_enum` (drm_mode.h).
#[repr(C)]
pub(crate) struct drm_mode_property_enum {
    pub(crate) value: u64,
    pub(crate) name: [c_char; DRM_PROP_NAME_LEN],
}

/// `drmModePropertyRes`.
#[repr(C)]
pub(crate) struct drmModePropertyRes {
    pub(crate) prop_id: u32,
    pub(crate) flags: u32,
    pub(crate) name: [c_char; DRM_PROP_NAME_LEN],
    pub(crate) count_values: c_int,
    /// store the blob lengths
    pub(crate) values: *mut u64,
    pub(crate) count_enums: c_int,
    pub(crate) enums: *mut drm_mode_property_enum,
    pub(crate) count_blobs: c_int,
    /// store the blob IDs
    pub(crate) blob_ids: *mut u32,
}

/// `drmModeCrtc`.
#[repr(C)]
pub(crate) struct drmModeCrtc {
    pub(crate) crtc_id: u32,
    /// FB id to connect to 0 = disconnect
    pub(crate) buffer_id: u32,
    /// Position on the framebuffer
    pub(crate) x: u32,
    pub(crate) y: u32,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) mode_valid: c_int,
    pub(crate) mode: drmModeModeInfo,
    /// Number of gamma stops
    pub(crate) gamma_size: c_int,
}

/// `drmModeEncoder`.
#[repr(C)]
pub(crate) struct drmModeEncoder {
    pub(crate) encoder_id: u32,
    pub(crate) encoder_type: u32,
    pub(crate) crtc_id: u32,
    pub(crate) possible_crtcs: u32,
    pub(crate) possible_clones: u32,
}

/// `drmModeConnector`.
#[repr(C)]
pub(crate) struct drmModeConnector {
    pub(crate) connector_id: u32,
    /// Encoder currently connected to
    pub(crate) encoder_id: u32,
    pub(crate) connector_type: u32,
    pub(crate) connector_type_id: u32,
    pub(crate) connection: drmModeConnection,
    /// HxW in millimeters
    pub(crate) mmWidth: u32,
    pub(crate) mmHeight: u32,
    pub(crate) subpixel: drmModeSubPixel,
    pub(crate) count_modes: c_int,
    pub(crate) modes: *mut drmModeModeInfo,
    pub(crate) count_props: c_int,
    /// List of property ids
    pub(crate) props: *mut u32,
    /// List of property values
    pub(crate) prop_values: *mut u64,
    pub(crate) count_encoders: c_int,
    /// List of encoder ids
    pub(crate) encoders: *mut u32,
}

/// `drmModeObjectProperties`.
#[repr(C)]
pub(crate) struct drmModeObjectProperties {
    pub(crate) count_props: u32,
    pub(crate) props: *mut u32,
    pub(crate) prop_values: *mut u64,
}

/// `drmModePlane`.
#[repr(C)]
pub(crate) struct drmModePlane {
    pub(crate) count_formats: u32,
    pub(crate) formats: *mut u32,
    pub(crate) plane_id: u32,
    pub(crate) crtc_id: u32,
    pub(crate) fb_id: u32,
    pub(crate) crtc_x: u32,
    pub(crate) crtc_y: u32,
    pub(crate) x: u32,
    pub(crate) y: u32,
    pub(crate) possible_crtcs: u32,
    pub(crate) gamma_size: u32,
}

/// `drmModePlaneRes`.
#[repr(C)]
pub(crate) struct drmModePlaneRes {
    pub(crate) count_planes: u32,
    pub(crate) planes: *mut u32,
}

/// `drmModeAtomicReq` (opaque: `struct _drmModeAtomicReq`).
pub(crate) enum drmModeAtomicReq {}

// ---------------------------------------------------------------------------
// gbm.h
// ---------------------------------------------------------------------------

/// `struct gbm_device` (opaque).
pub(crate) enum gbm_device {}
/// `struct gbm_bo` (opaque).
pub(crate) enum gbm_bo {}
/// `struct gbm_surface` (opaque).
pub(crate) enum gbm_surface {}

/// `union gbm_bo_handle`: an abstraction for the driver-specific handle of
/// a buffer object.
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) union gbm_bo_handle {
    pub(crate) ptr: *mut c_void,
    pub(crate) s32: i32,
    pub(crate) u32_: u32,
    pub(crate) s64: i64,
    pub(crate) u64_: u64,
}

/// The `destroy_user_data` callback of `gbm_bo_set_user_data()`.
pub(crate) type gbm_bo_user_data_destroy_t =
    Option<unsafe extern "C" fn(bo: *mut gbm_bo, data: *mut c_void)>;

/// Buffer is going to be presented to the screen using an API such as KMS
pub(crate) const GBM_BO_USE_SCANOUT: u32 = 1 << 0;
/// Buffer is going to be used as cursor
pub(crate) const GBM_BO_USE_CURSOR: u32 = 1 << 1;
/// Buffer is to be used for rendering - for example it is going to be used
/// as the storage for a color buffer
pub(crate) const GBM_BO_USE_RENDERING: u32 = 1 << 2;
/// Buffer can be used for gbm_bo_write.  This is guaranteed to work
/// with GBM_BO_USE_CURSOR, but may not work for other combinations.
pub(crate) const GBM_BO_USE_WRITE: u32 = 1 << 3;
/// Buffer is linear, i.e. not tiled.
pub(crate) const GBM_BO_USE_LINEAR: u32 = 1 << 4;

// ---------------------------------------------------------------------------
// drm_fourcc.h (gbm.h's GBM_FORMAT_* are the same codes)
// ---------------------------------------------------------------------------

/// `fourcc_code()` / `__gbm_fourcc_code()`.
pub(crate) const fn fourcc_code(a: u8, b: u8, c: u8, d: u8) -> u32 {
    (a as u32) | ((b as u32) << 8) | ((c as u32) << 16) | ((d as u32) << 24)
}

pub(crate) const GBM_FORMAT_XRGB1555: u32 = fourcc_code(b'X', b'R', b'1', b'5');
pub(crate) const GBM_FORMAT_XBGR1555: u32 = fourcc_code(b'X', b'B', b'1', b'5');
pub(crate) const GBM_FORMAT_RGBX5551: u32 = fourcc_code(b'R', b'X', b'1', b'5');
pub(crate) const GBM_FORMAT_BGRX5551: u32 = fourcc_code(b'B', b'X', b'1', b'5');
pub(crate) const GBM_FORMAT_ARGB1555: u32 = fourcc_code(b'A', b'R', b'1', b'5');
pub(crate) const GBM_FORMAT_ABGR1555: u32 = fourcc_code(b'A', b'B', b'1', b'5');
pub(crate) const GBM_FORMAT_RGBA5551: u32 = fourcc_code(b'R', b'A', b'1', b'5');
pub(crate) const GBM_FORMAT_BGRA5551: u32 = fourcc_code(b'B', b'A', b'1', b'5');
pub(crate) const GBM_FORMAT_RGB565: u32 = fourcc_code(b'R', b'G', b'1', b'6');
pub(crate) const GBM_FORMAT_BGR565: u32 = fourcc_code(b'B', b'G', b'1', b'6');
pub(crate) const GBM_FORMAT_XRGB8888: u32 = fourcc_code(b'X', b'R', b'2', b'4');
pub(crate) const GBM_FORMAT_XBGR8888: u32 = fourcc_code(b'X', b'B', b'2', b'4');
pub(crate) const GBM_FORMAT_RGBX8888: u32 = fourcc_code(b'R', b'X', b'2', b'4');
pub(crate) const GBM_FORMAT_BGRX8888: u32 = fourcc_code(b'B', b'X', b'2', b'4');
pub(crate) const GBM_FORMAT_ARGB8888: u32 = fourcc_code(b'A', b'R', b'2', b'4');
pub(crate) const GBM_FORMAT_ABGR8888: u32 = fourcc_code(b'A', b'B', b'2', b'4');
pub(crate) const GBM_FORMAT_RGBA8888: u32 = fourcc_code(b'R', b'A', b'2', b'4');
pub(crate) const GBM_FORMAT_BGRA8888: u32 = fourcc_code(b'B', b'A', b'2', b'4');
pub(crate) const GBM_FORMAT_XRGB2101010: u32 = fourcc_code(b'X', b'R', b'3', b'0');
pub(crate) const GBM_FORMAT_XBGR2101010: u32 = fourcc_code(b'X', b'B', b'3', b'0');
pub(crate) const GBM_FORMAT_RGBX1010102: u32 = fourcc_code(b'R', b'X', b'3', b'0');
pub(crate) const GBM_FORMAT_BGRX1010102: u32 = fourcc_code(b'B', b'X', b'3', b'0');
pub(crate) const GBM_FORMAT_ARGB2101010: u32 = fourcc_code(b'A', b'R', b'3', b'0');
pub(crate) const GBM_FORMAT_ABGR2101010: u32 = fourcc_code(b'A', b'B', b'3', b'0');
pub(crate) const GBM_FORMAT_RGBA1010102: u32 = fourcc_code(b'R', b'A', b'3', b'0');
pub(crate) const GBM_FORMAT_BGRA1010102: u32 = fourcc_code(b'B', b'A', b'3', b'0');

/// `DRM_FORMAT_MOD_INVALID`: `fourcc_mod_code(NONE, DRM_FORMAT_RESERVED)`.
pub(crate) const DRM_FORMAT_MOD_INVALID: u64 = 0x00ffffffffffffff;
/// `DRM_FORMAT_MOD_LINEAR`: `fourcc_mod_code(NONE, 0)`.
pub(crate) const DRM_FORMAT_MOD_LINEAR: u64 = 0;

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{align_of, offset_of, size_of};

    /// The sizes and offsets of the C structures, from a C program built
    /// against libdrm 2.4.125's and Mesa 25.2.8's headers (Ubuntu 24.04,
    /// x86-64 Linux; aarch64 Linux is the same).
    #[cfg(all(target_os = "linux", target_pointer_width = "64"))]
    #[test]
    fn layouts_match_c() {
        assert_eq!((size_of::<drmModeRes>(), align_of::<drmModeRes>()), (80, 8));
        assert_eq!(offset_of!(drmModeRes, count_fbs), 0);
        assert_eq!(offset_of!(drmModeRes, fbs), 8);
        assert_eq!(offset_of!(drmModeRes, count_crtcs), 16);
        assert_eq!(offset_of!(drmModeRes, crtcs), 24);
        assert_eq!(offset_of!(drmModeRes, count_connectors), 32);
        assert_eq!(offset_of!(drmModeRes, connectors), 40);
        assert_eq!(offset_of!(drmModeRes, count_encoders), 48);
        assert_eq!(offset_of!(drmModeRes, encoders), 56);
        assert_eq!(offset_of!(drmModeRes, min_width), 64);
        assert_eq!(offset_of!(drmModeRes, max_width), 68);
        assert_eq!(offset_of!(drmModeRes, min_height), 72);
        assert_eq!(offset_of!(drmModeRes, max_height), 76);

        assert_eq!(
            (size_of::<drmModeModeInfo>(), align_of::<drmModeModeInfo>()),
            (68, 4)
        );
        assert_eq!(offset_of!(drmModeModeInfo, clock), 0);
        assert_eq!(offset_of!(drmModeModeInfo, hdisplay), 4);
        assert_eq!(offset_of!(drmModeModeInfo, vscan), 22);
        assert_eq!(offset_of!(drmModeModeInfo, vrefresh), 24);
        assert_eq!(offset_of!(drmModeModeInfo, flags), 28);
        assert_eq!(offset_of!(drmModeModeInfo, type_), 32);
        assert_eq!(offset_of!(drmModeModeInfo, name), 36);

        assert_eq!((size_of::<drmModeFB>(), align_of::<drmModeFB>()), (28, 4));
        assert_eq!(offset_of!(drmModeFB, fb_id), 0);
        assert_eq!(offset_of!(drmModeFB, width), 4);
        assert_eq!(offset_of!(drmModeFB, height), 8);
        assert_eq!(offset_of!(drmModeFB, pitch), 12);
        assert_eq!(offset_of!(drmModeFB, bpp), 16);
        assert_eq!(offset_of!(drmModeFB, depth), 20);
        assert_eq!(offset_of!(drmModeFB, handle), 24);

        assert_eq!((size_of::<drmModeFB2>(), align_of::<drmModeFB2>()), (80, 8));
        assert_eq!(offset_of!(drmModeFB2, fb_id), 0);
        assert_eq!(offset_of!(drmModeFB2, pixel_format), 12);
        assert_eq!(offset_of!(drmModeFB2, modifier), 16);
        assert_eq!(offset_of!(drmModeFB2, flags), 24);
        assert_eq!(offset_of!(drmModeFB2, handles), 28);
        assert_eq!(offset_of!(drmModeFB2, pitches), 44);
        assert_eq!(offset_of!(drmModeFB2, offsets), 60);

        assert_eq!(
            (
                size_of::<drmModePropertyBlobRes>(),
                align_of::<drmModePropertyBlobRes>()
            ),
            (16, 8)
        );
        assert_eq!(offset_of!(drmModePropertyBlobRes, id), 0);
        assert_eq!(offset_of!(drmModePropertyBlobRes, length), 4);
        assert_eq!(offset_of!(drmModePropertyBlobRes, data), 8);

        assert_eq!(
            (
                size_of::<drmModePropertyRes>(),
                align_of::<drmModePropertyRes>()
            ),
            (88, 8)
        );
        assert_eq!(offset_of!(drmModePropertyRes, prop_id), 0);
        assert_eq!(offset_of!(drmModePropertyRes, flags), 4);
        assert_eq!(offset_of!(drmModePropertyRes, name), 8);
        assert_eq!(offset_of!(drmModePropertyRes, count_values), 40);
        assert_eq!(offset_of!(drmModePropertyRes, values), 48);
        assert_eq!(offset_of!(drmModePropertyRes, count_enums), 56);
        assert_eq!(offset_of!(drmModePropertyRes, enums), 64);
        assert_eq!(offset_of!(drmModePropertyRes, count_blobs), 72);
        assert_eq!(offset_of!(drmModePropertyRes, blob_ids), 80);

        assert_eq!(
            (
                size_of::<drm_mode_property_enum>(),
                align_of::<drm_mode_property_enum>()
            ),
            (40, 8)
        );
        assert_eq!(offset_of!(drm_mode_property_enum, value), 0);
        assert_eq!(offset_of!(drm_mode_property_enum, name), 8);

        assert_eq!(
            (size_of::<drmModeCrtc>(), align_of::<drmModeCrtc>()),
            (100, 4)
        );
        assert_eq!(offset_of!(drmModeCrtc, crtc_id), 0);
        assert_eq!(offset_of!(drmModeCrtc, buffer_id), 4);
        assert_eq!(offset_of!(drmModeCrtc, x), 8);
        assert_eq!(offset_of!(drmModeCrtc, y), 12);
        assert_eq!(offset_of!(drmModeCrtc, width), 16);
        assert_eq!(offset_of!(drmModeCrtc, height), 20);
        assert_eq!(offset_of!(drmModeCrtc, mode_valid), 24);
        assert_eq!(offset_of!(drmModeCrtc, mode), 28);
        assert_eq!(offset_of!(drmModeCrtc, gamma_size), 96);

        assert_eq!(
            (size_of::<drmModeEncoder>(), align_of::<drmModeEncoder>()),
            (20, 4)
        );
        assert_eq!(offset_of!(drmModeEncoder, encoder_id), 0);
        assert_eq!(offset_of!(drmModeEncoder, encoder_type), 4);
        assert_eq!(offset_of!(drmModeEncoder, crtc_id), 8);
        assert_eq!(offset_of!(drmModeEncoder, possible_crtcs), 12);
        assert_eq!(offset_of!(drmModeEncoder, possible_clones), 16);

        assert_eq!(
            (
                size_of::<drmModeConnector>(),
                align_of::<drmModeConnector>()
            ),
            (88, 8)
        );
        assert_eq!(offset_of!(drmModeConnector, connector_id), 0);
        assert_eq!(offset_of!(drmModeConnector, encoder_id), 4);
        assert_eq!(offset_of!(drmModeConnector, connector_type), 8);
        assert_eq!(offset_of!(drmModeConnector, connector_type_id), 12);
        assert_eq!(offset_of!(drmModeConnector, connection), 16);
        assert_eq!(offset_of!(drmModeConnector, mmWidth), 20);
        assert_eq!(offset_of!(drmModeConnector, mmHeight), 24);
        assert_eq!(offset_of!(drmModeConnector, subpixel), 28);
        assert_eq!(offset_of!(drmModeConnector, count_modes), 32);
        assert_eq!(offset_of!(drmModeConnector, modes), 40);
        assert_eq!(offset_of!(drmModeConnector, count_props), 48);
        assert_eq!(offset_of!(drmModeConnector, props), 56);
        assert_eq!(offset_of!(drmModeConnector, prop_values), 64);
        assert_eq!(offset_of!(drmModeConnector, count_encoders), 72);
        assert_eq!(offset_of!(drmModeConnector, encoders), 80);

        assert_eq!(
            (
                size_of::<drmModeObjectProperties>(),
                align_of::<drmModeObjectProperties>()
            ),
            (24, 8)
        );
        assert_eq!(offset_of!(drmModeObjectProperties, count_props), 0);
        assert_eq!(offset_of!(drmModeObjectProperties, props), 8);
        assert_eq!(offset_of!(drmModeObjectProperties, prop_values), 16);

        assert_eq!(
            (size_of::<drmModePlane>(), align_of::<drmModePlane>()),
            (56, 8)
        );
        assert_eq!(offset_of!(drmModePlane, count_formats), 0);
        assert_eq!(offset_of!(drmModePlane, formats), 8);
        assert_eq!(offset_of!(drmModePlane, plane_id), 16);
        assert_eq!(offset_of!(drmModePlane, crtc_id), 20);
        assert_eq!(offset_of!(drmModePlane, fb_id), 24);
        assert_eq!(offset_of!(drmModePlane, crtc_x), 28);
        assert_eq!(offset_of!(drmModePlane, crtc_y), 32);
        assert_eq!(offset_of!(drmModePlane, x), 36);
        assert_eq!(offset_of!(drmModePlane, y), 40);
        assert_eq!(offset_of!(drmModePlane, possible_crtcs), 44);
        assert_eq!(offset_of!(drmModePlane, gamma_size), 48);

        assert_eq!(
            (size_of::<drmModePlaneRes>(), align_of::<drmModePlaneRes>()),
            (16, 8)
        );
        assert_eq!(offset_of!(drmModePlaneRes, count_planes), 0);
        assert_eq!(offset_of!(drmModePlaneRes, planes), 8);

        assert_eq!(
            (size_of::<drmEventContext>(), align_of::<drmEventContext>()),
            (40, 8)
        );
        assert_eq!(offset_of!(drmEventContext, version), 0);
        assert_eq!(offset_of!(drmEventContext, vblank_handler), 8);
        assert_eq!(offset_of!(drmEventContext, page_flip_handler), 16);
        assert_eq!(offset_of!(drmEventContext, page_flip_handler2), 24);
        assert_eq!(offset_of!(drmEventContext, sequence_handler), 32);

        assert_eq!(
            (size_of::<gbm_bo_handle>(), align_of::<gbm_bo_handle>()),
            (8, 8)
        );
        assert_eq!(
            (size_of::<drm_magic_t>(), align_of::<drm_magic_t>()),
            (4, 4)
        );
    }

    /// The constants, as the same C program printed them.
    #[test]
    fn constants_match_c() {
        assert_eq!(DRM_EVENT_CONTEXT_VERSION, 0x4);
        assert_eq!(DRM_CAP_CURSOR_WIDTH, 0x8);
        assert_eq!(DRM_CAP_CURSOR_HEIGHT, 0x9);
        assert_eq!(DRM_CLIENT_CAP_UNIVERSAL_PLANES, 0x2);
        assert_eq!(DRM_CLIENT_CAP_ATOMIC, 0x3);
        assert_eq!(DRM_MODE_PAGE_FLIP_EVENT, 0x1);
        assert_eq!(DRM_MODE_ATOMIC_NONBLOCK, 0x200);
        assert_eq!(DRM_MODE_ATOMIC_ALLOW_MODESET, 0x400);
        assert_eq!(DRM_MODE_OBJECT_CRTC, 0xcccccccc);
        assert_eq!(DRM_MODE_OBJECT_CONNECTOR, 0xc0c0c0c0);
        assert_eq!(DRM_MODE_OBJECT_PLANE, 0xeeeeeeee);
        assert_eq!(DRM_MODE_FB_MODIFIERS, 0x2);
        assert_eq!(DRM_PLANE_TYPE_OVERLAY, 0x0);
        assert_eq!(DRM_PLANE_TYPE_PRIMARY, 0x1);
        assert_eq!(DRM_PLANE_TYPE_CURSOR, 0x2);
        assert_eq!(DRM_MODE_TYPE_PREFERRED, 0x8);
        assert_eq!(DRM_MODE_FLAG_INTERLACE, 0x10);
        assert_eq!(DRM_MODE_PROP_ENUM, 0x8);
        assert_eq!(GBM_FORMAT_XRGB8888, 0x34325258);
        assert_eq!(GBM_FORMAT_ARGB8888, 0x34325241);
        assert_eq!(GBM_FORMAT_RGB565, 0x36314752);
        assert_eq!(DRM_FORMAT_MOD_INVALID, 0xffffffffffffff);
        assert_eq!(DRM_FORMAT_MOD_LINEAR, 0x0);
        assert_eq!(GBM_BO_USE_SCANOUT, 0x1);
        assert_eq!(GBM_BO_USE_CURSOR, 0x2);
        assert_eq!(GBM_BO_USE_RENDERING, 0x4);
        assert_eq!(GBM_BO_USE_WRITE, 0x8);
        assert_eq!(GBM_BO_USE_LINEAR, 0x10);
    }
}
