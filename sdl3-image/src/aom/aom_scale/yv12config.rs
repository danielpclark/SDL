// Rust translation of aom_scale/generic/yv12config.c and
// aom_scale/yv12config.h from aom (https://aomedia.googlesource.com/aom, at
// the revision SDL_image's external/aom pins: libaom 3.6.1 with SDL's
// patches).
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Frame buffers with borders. The encoder runs with high bit depth
//! buffers only (SDL_image's AVIF saver hands libavif 10-bit images), so
//! `buffer_alloc` holds 16-bit samples, and the plane "pointers" are
//! sample offsets into it (as upstream's CONVERT_TO_BYTEPTR() addresses
//! are). No external frame buffer callbacks, no 8-bit copy of the luma
//! plane (global motion is off), byte alignment 0.

pub(crate) const AOM_INTERP_EXTEND: i32 = 4;
pub(crate) const AOM_BORDER_IN_PIXELS: i32 = 288;
pub(crate) const AOM_ENC_NO_SCALE_BORDER: i32 = 160;
pub(crate) const AOM_ENC_ALLINTRA_BORDER: i32 = 64;
pub(crate) const AOM_DEC_BORDER_IN_PIXELS: i32 = 64;

pub(crate) const YV12_FLAG_HIGHBITDEPTH: i32 = 8;

/// Translation of `YV12_BUFFER_CONFIG`; the unions are the `[0]`/`[1]`
/// pairs (`widths` is `y_width`, `uv_width` and so on).
#[derive(Debug, Default, Clone)]
pub(crate) struct Yv12BufferConfig {
    pub(crate) widths: [i32; 2],
    pub(crate) heights: [i32; 2],
    pub(crate) crop_widths: [i32; 2],
    pub(crate) crop_heights: [i32; 2],
    pub(crate) strides: [i32; 2],
    /// `y_buffer`, `u_buffer`, `v_buffer`: offsets into `buffer_alloc`
    pub(crate) buffers: [usize; 3],

    pub(crate) buffer_alloc: Vec<u16>,
    pub(crate) buffer_alloc_sz: usize,
    pub(crate) border: i32,
    pub(crate) frame_size: usize,
    pub(crate) subsampling_x: i32,
    pub(crate) subsampling_y: i32,
    pub(crate) bit_depth: u32,
    pub(crate) color_primaries: i32,
    pub(crate) transfer_characteristics: i32,
    pub(crate) matrix_coefficients: i32,
    pub(crate) monochrome: u8,
    pub(crate) chroma_sample_position: i32,
    pub(crate) color_range: i32,
    pub(crate) render_width: i32,
    pub(crate) render_height: i32,

    pub(crate) corrupted: i32,
    pub(crate) flags: i32,
}

impl Yv12BufferConfig {
    pub(crate) fn y_width(&self) -> i32 {
        self.widths[0]
    }
    pub(crate) fn y_height(&self) -> i32 {
        self.heights[0]
    }
    pub(crate) fn y_crop_width(&self) -> i32 {
        self.crop_widths[0]
    }
    pub(crate) fn y_crop_height(&self) -> i32 {
        self.crop_heights[0]
    }
    pub(crate) fn uv_crop_width(&self) -> i32 {
        self.crop_widths[1]
    }
    pub(crate) fn uv_crop_height(&self) -> i32 {
        self.crop_heights[1]
    }
    pub(crate) fn y_stride(&self) -> i32 {
        self.strides[0]
    }
    pub(crate) fn uv_stride(&self) -> i32 {
        self.strides[1]
    }
}

#[inline]
pub(crate) fn aom_calc_y_stride(aligned_width: i32, border: i32) -> i32 {
    ((aligned_width + 2 * border) + 31) & !31
}

/// Translation of `aom_free_frame_buffer()`.
pub(crate) fn aom_free_frame_buffer(ybf: &mut Yv12BufferConfig) {
    *ybf = Yv12BufferConfig::default();
}

/// Translation of `realloc_frame_buffer_aligned()`, for high bit depth
/// (`Err(())` is AOM_CODEC_MEM_ERROR).
fn realloc_frame_buffer_aligned(
    ybf: &mut Yv12BufferConfig,
    width: i32,
    height: i32,
    ss_x: i32,
    ss_y: i32,
    use_highbitdepth: i32,
    border: i32,
    y_stride: i32,
    yplane_size: u64,
    uvplane_size: u64,
    aligned_width: i32,
    aligned_height: i32,
    uv_width: i32,
    uv_height: i32,
    uv_stride: i32,
    uv_border_w: i32,
    uv_border_h: i32,
    alloc_y_plane_only: i32,
) -> Result<(), ()> {
    debug_assert!(use_highbitdepth != 0);
    // (in samples: upstream's frame_size is in bytes, two per sample)
    let frame_size: u64 = yplane_size + 2 * uvplane_size;

    if frame_size as usize > ybf.buffer_alloc_sz {
        // Allocation to hold larger frame, or first allocation.
        ybf.buffer_alloc = Vec::new();
        ybf.buffer_alloc_sz = 0;

        if frame_size != frame_size as usize as u64 {
            return Err(());
        }

        let mut buffer_alloc = Vec::new();
        if buffer_alloc.try_reserve_exact(frame_size as usize).is_err() {
            return Err(());
        }

        ybf.buffer_alloc_sz = frame_size as usize;

        // This memset is needed for fixing valgrind error from C loop filter
        // due to access uninitialized memory in frame border. It could be
        // removed if border is totally removed.
        buffer_alloc.resize(frame_size as usize, 0);
        ybf.buffer_alloc = buffer_alloc;
    }

    ybf.crop_widths[0] = width;
    ybf.crop_heights[0] = height;
    ybf.widths[0] = aligned_width;
    ybf.heights[0] = aligned_height;
    ybf.strides[0] = y_stride;

    ybf.crop_widths[1] = (width + ss_x) >> ss_x;
    ybf.crop_heights[1] = (height + ss_y) >> ss_y;
    ybf.widths[1] = uv_width;
    ybf.heights[1] = uv_height;
    ybf.strides[1] = uv_stride;

    ybf.border = border;
    ybf.frame_size = 2 * frame_size as usize;
    ybf.subsampling_x = ss_x;
    ybf.subsampling_y = ss_y;

    // Store uint16 addresses when using 16bit framebuffers
    ybf.flags = YV12_FLAG_HIGHBITDEPTH;

    ybf.buffers[0] = (border as i64 * y_stride as i64 + border as i64) as usize;
    if alloc_y_plane_only == 0 {
        ybf.buffers[1] = (yplane_size as i64
            + uv_border_h as i64 * uv_stride as i64
            + uv_border_w as i64) as usize;
        ybf.buffers[2] = (yplane_size as i64
            + uvplane_size as i64
            + uv_border_h as i64 * uv_stride as i64
            + uv_border_w as i64) as usize;
    } else {
        ybf.buffers[1] = 0;
        ybf.buffers[2] = 0;
    }

    ybf.corrupted = 0; /* assume not corrupted by errors */
    Ok(())
}

fn calc_stride_and_planesize(
    ss_x: i32,
    ss_y: i32,
    aligned_width: i32,
    aligned_height: i32,
    border: i32,
    byte_alignment: i32,
    alloc_y_plane_only: i32,
    y_stride: &mut i32,
    uv_stride: &mut i32,
    yplane_size: &mut u64,
    uvplane_size: &mut u64,
    uv_height: i32,
) -> Result<(), ()> {
    /* Only support allocating buffers that have a border that's a multiple
     * of 32. The border restriction is required to get 16-byte alignment of
     * the start of the chroma rows without introducing an arbitrary gap
     * between planes, which would break the semantics of things like
     * aom_img_set_rect(). */
    if border & 0x1f != 0 {
        return Err(());
    }
    *y_stride = aom_calc_y_stride(aligned_width, border);
    *yplane_size =
        (aligned_height + 2 * border) as u64 * (*y_stride) as u64 + byte_alignment as u64;

    if alloc_y_plane_only == 0 {
        *uv_stride = *y_stride >> ss_x;
        *uvplane_size =
            (uv_height + 2 * (border >> ss_y)) as u64 * (*uv_stride) as u64 + byte_alignment as u64;
    } else {
        *uv_stride = 0;
        *uvplane_size = 0;
    }
    Ok(())
}

/// Translation of `aom_realloc_frame_buffer()` (no frame buffer
/// callback, no 8-bit luma copy).
pub(crate) fn aom_realloc_frame_buffer(
    ybf: &mut Yv12BufferConfig,
    width: i32,
    height: i32,
    ss_x: i32,
    ss_y: i32,
    use_highbitdepth: i32,
    border: i32,
    byte_alignment: i32,
    alloc_y_plane_only: i32,
) -> Result<(), ()> {
    let mut y_stride = 0;
    let mut uv_stride = 0;
    let mut yplane_size: u64 = 0;
    let mut uvplane_size: u64 = 0;
    let aligned_width = (width + 7) & !7;
    let aligned_height = (height + 7) & !7;
    let uv_width = aligned_width >> ss_x;
    let uv_height = aligned_height >> ss_y;
    let uv_border_w = border >> ss_x;
    let uv_border_h = border >> ss_y;

    calc_stride_and_planesize(
        ss_x,
        ss_y,
        aligned_width,
        aligned_height,
        border,
        byte_alignment,
        alloc_y_plane_only,
        &mut y_stride,
        &mut uv_stride,
        &mut yplane_size,
        &mut uvplane_size,
        uv_height,
    )?;
    realloc_frame_buffer_aligned(
        ybf,
        width,
        height,
        ss_x,
        ss_y,
        use_highbitdepth,
        border,
        y_stride,
        yplane_size,
        uvplane_size,
        aligned_width,
        aligned_height,
        uv_width,
        uv_height,
        uv_stride,
        uv_border_w,
        uv_border_h,
        alloc_y_plane_only,
    )
}

/// Translation of `aom_alloc_frame_buffer()`.
pub(crate) fn aom_alloc_frame_buffer(
    ybf: &mut Yv12BufferConfig,
    width: i32,
    height: i32,
    ss_x: i32,
    ss_y: i32,
    use_highbitdepth: i32,
    border: i32,
    byte_alignment: i32,
    alloc_y_plane_only: i32,
) -> Result<(), ()> {
    aom_free_frame_buffer(ybf);
    aom_realloc_frame_buffer(
        ybf,
        width,
        height,
        ss_x,
        ss_y,
        use_highbitdepth,
        border,
        byte_alignment,
        alloc_y_plane_only,
    )
}
