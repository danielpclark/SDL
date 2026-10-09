// Rust translation of src/scale.c from libavif
// (https://github.com/AOMediaCodec/libavif, at the revision SDL_image's
// external/libavif pins: libavif 1.1.1 with SDL's patches), over the
// translation of the libyuv subset it bundles (`super::libyuv`, LIBYUV_VERSION
// 1880).
// Copyright 2021 Joe Drago. All rights reserved.
// SPDX-License-Identifier: BSD-2-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Scaling a decoded image to the size its container gives.

use super::avif::{
    avif_dimensions_too_large, avif_image_allocate_planes, avif_image_plane_height,
    avif_image_plane_width, avif_result_to_string, AvifImage, AvifResult, AVIF_CHAN_U, AVIF_CHAN_Y,
    AVIF_PLANES_A, AVIF_PLANES_YUV, AVIF_PLANE_COUNT_YUV,
};
use super::diag::{self, AvifDiagnostics};
use super::libyuv::{scale_plane, scale_plane_12, FilterMode};

/// `avifDiagnosticsPrintf()`
macro_rules! diag_printf {
    ($diag:expr, $($arg:tt)*) => {
        diag::printf($diag, format_args!($($arg)*))
    };
}

// This should be configurable and/or smarter. kFilterBox has the highest quality but is the slowest.
const AVIF_LIBYUV_FILTER_MODE: FilterMode = FilterMode::Box;

/// Translation of `avifImageScaleWithLimit()`.
pub(crate) fn avif_image_scale_with_limit(
    image: &mut AvifImage,
    dst_width: u32,
    dst_height: u32,
    image_size_limit: u32,
    image_dimension_limit: u32,
    diag: Option<&AvifDiagnostics>,
) -> AvifResult {
    if (image.width == dst_width) && (image.height == dst_height) {
        // Nothing to do
        return AvifResult::Ok;
    }

    if (dst_width == 0) || (dst_height == 0) {
        diag_printf!(
            diag,
            "avifImageScaleWithLimit requested invalid dst dimensions [{}x{}]",
            dst_width,
            dst_height
        );
        return AvifResult::InvalidArgument;
    }
    if avif_dimensions_too_large(
        dst_width,
        dst_height,
        image_size_limit,
        image_dimension_limit,
    ) {
        diag_printf!(
            diag,
            "avifImageScaleWithLimit requested dst dimensions that are too large [{}x{}]",
            dst_width,
            dst_height
        );
        return AvifResult::NotImplemented;
    }

    let mut src_yuv_planes: [Option<super::avif::AvifPlane>; AVIF_PLANE_COUNT_YUV] =
        Default::default();
    let mut src_yuv_row_bytes = [0u32; AVIF_PLANE_COUNT_YUV];
    for i in 0..AVIF_PLANE_COUNT_YUV {
        src_yuv_planes[i] = image.yuv_planes[i].take();
        src_yuv_row_bytes[i] = image.yuv_row_bytes[i];
        image.yuv_row_bytes[i] = 0;
    }
    // (srcImageOwnsYUVPlanes: dropping the planes frees what the image owned)
    image.image_owns_yuv_planes = false;

    let src_alpha_plane = image.alpha_plane.take();
    let src_alpha_row_bytes = image.alpha_row_bytes;
    image.alpha_row_bytes = 0;
    image.image_owns_alpha_plane = false;

    let src_width = image.width;
    let src_height = image.height;
    let src_uv_width = avif_image_plane_width(image, AVIF_CHAN_U);
    let src_uv_height = avif_image_plane_height(image, AVIF_CHAN_U);
    image.width = dst_width;
    image.height = dst_height;

    if src_yuv_planes[0].is_some() || src_alpha_plane.is_some() {
        // A simple conservative check to avoid integer overflows in libyuv's ScalePlane() and
        // ScalePlane_12() functions.
        if src_width > 16384 {
            diag_printf!(
                diag,
                "avifImageScaleWithLimit requested invalid width scale for libyuv [{} -> {}]",
                src_width,
                dst_width
            );
            return AvifResult::NotImplemented;
        }
        if src_height > 16384 {
            diag_printf!(
                diag,
                "avifImageScaleWithLimit requested invalid height scale for libyuv [{} -> {}]",
                src_height,
                dst_height
            );
            return AvifResult::NotImplemented;
        }
    }

    if src_yuv_planes[0].is_some() {
        let allocation_result = avif_image_allocate_planes(image, AVIF_PLANES_YUV);
        if allocation_result != AvifResult::Ok {
            diag_printf!(
                diag,
                "Allocation of YUV planes failed: {}",
                avif_result_to_string(allocation_result)
            );
            return AvifResult::OutOfMemory;
        }

        for i in 0..AVIF_PLANE_COUNT_YUV {
            let Some(src_plane) = &src_yuv_planes[i] else {
                continue;
            };

            let src_w = if i == AVIF_CHAN_Y {
                src_width
            } else {
                src_uv_width
            };
            let src_h = if i == AVIF_CHAN_Y {
                src_height
            } else {
                src_uv_height
            };
            let dst_w = avif_image_plane_width(image, i);
            let dst_h = avif_image_plane_height(image, i);
            let failure = if image.depth > 8 {
                let src_stride = src_yuv_row_bytes[i] / 2;
                let dst_stride = image.yuv_row_bytes[i] / 2;
                let Some(dst_plane) = image.yuv_planes[i].as_mut() else {
                    continue;
                };
                let failure = scale_plane_12(
                    src_plane.u16s(),
                    src_stride as i32,
                    src_w as i32,
                    src_h as i32,
                    dst_plane.u16s_mut(),
                    dst_stride as i32,
                    dst_w as i32,
                    dst_h as i32,
                    AVIF_LIBYUV_FILTER_MODE,
                );
                if failure != 0 {
                    diag_printf!(diag, "ScalePlane_12() failed ({})", failure);
                }
                failure
            } else {
                let src_stride = src_yuv_row_bytes[i];
                let dst_stride = image.yuv_row_bytes[i];
                let Some(dst_plane) = image.yuv_planes[i].as_mut() else {
                    continue;
                };
                let failure = scale_plane(
                    src_plane.u8s(),
                    src_stride as i32,
                    src_w as i32,
                    src_h as i32,
                    dst_plane.u8s_mut(),
                    dst_stride as i32,
                    dst_w as i32,
                    dst_h as i32,
                    AVIF_LIBYUV_FILTER_MODE,
                );
                if failure != 0 {
                    diag_printf!(diag, "ScalePlane() failed ({})", failure);
                }
                failure
            };
            if failure != 0 {
                return if failure == 1 {
                    AvifResult::OutOfMemory
                } else {
                    AvifResult::UnknownError
                };
            }
        }
    }

    if let Some(src_alpha_plane) = &src_alpha_plane {
        let allocation_result = avif_image_allocate_planes(image, AVIF_PLANES_A);
        if allocation_result != AvifResult::Ok {
            diag_printf!(
                diag,
                "Allocation of alpha plane failed: {}",
                avif_result_to_string(allocation_result)
            );
            // FIXME (upstream): returns without freeing the source planes
            // (dropped here).
            return AvifResult::OutOfMemory;
        }

        let Some(dst_plane) = image.alpha_plane.as_mut() else {
            return AvifResult::OutOfMemory;
        };
        let failure = if image.depth > 8 {
            let src_stride = src_alpha_row_bytes / 2;
            let dst_stride = image.alpha_row_bytes / 2;
            let failure = scale_plane_12(
                src_alpha_plane.u16s(),
                src_stride as i32,
                src_width as i32,
                src_height as i32,
                dst_plane.u16s_mut(),
                dst_stride as i32,
                dst_width as i32,
                dst_height as i32,
                AVIF_LIBYUV_FILTER_MODE,
            );
            if failure != 0 {
                diag_printf!(diag, "ScalePlane_12() failed ({})", failure);
            }
            failure
        } else {
            let src_stride = src_alpha_row_bytes;
            let dst_stride = image.alpha_row_bytes;
            let failure = scale_plane(
                src_alpha_plane.u8s(),
                src_stride as i32,
                src_width as i32,
                src_height as i32,
                dst_plane.u8s_mut(),
                dst_stride as i32,
                dst_width as i32,
                dst_height as i32,
                AVIF_LIBYUV_FILTER_MODE,
            );
            if failure != 0 {
                diag_printf!(diag, "ScalePlane() failed ({})", failure);
            }
            failure
        };
        if failure != 0 {
            return if failure == 1 {
                AvifResult::OutOfMemory
            } else {
                AvifResult::UnknownError
            };
        }
    }

    // (cleanup: the source planes are dropped)
    AvifResult::Ok
}

// (avifImageScale() is not used by the decoder)
