// Rust translation of lib/jxl/render_pipeline/low_memory_render_pipeline.h,
// lib/jxl/render_pipeline/low_memory_render_pipeline.cc and the parts of
// lib/jxl/render_pipeline/render_pipeline.h/.cc that belong to a pipeline
// object, from libjxl (https://github.com/libjxl/libjxl, at the revision
// SDL_image's external/libjxl pins: libjxl 0.7.3 with SDL's patches).
// Copyright (c) the JPEG XL Project Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! A multithreaded, low-memory rendering pipeline that only allocates a
//! minimal amount of buffers. (Run on one thread here.)

use super::super::base::{ceil_log2_nonzero_u64, div_ceil, round_up_to, FrameDimensions, Status, StatusCode};
use super::super::dec_group_border::GroupBorderAssigner;
use super::super::frame_header::FrameOrigin;
use super::super::image::{copy_image_to_rect, mirror, ImageF, Rect};
use super::{
    RenderPipelineChannelMode, RenderPipelineInput, RenderPipelineStage, RowInfo, RowPtr, StageCtx, StageRows,
    K_RENDER_PIPELINE_X_OFFSET,
};

/// A rect with signed coordinates. Translation of `RectT<ssize_t>`.
#[derive(Clone, Copy, Debug)]
struct SRect {
    x0: i64,
    y0: i64,
    xsize: i64,
    ysize: i64,
}

impl SRect {
    fn new(x0: i64, y0: i64, xsize: i64, ysize: i64) -> SRect {
        SRect { x0, y0, xsize, ysize }
    }
    fn from_rect(r: &Rect) -> SRect {
        SRect::new(r.x0() as i64, r.y0() as i64, r.xsize() as i64, r.ysize() as i64)
    }
    fn x1(&self) -> i64 {
        self.x0 + self.xsize
    }
    fn y1(&self) -> i64 {
        self.y0 + self.ysize
    }
    fn clamped_size(begin: i64, size_max: i64, end: i64) -> i64 {
        if begin + size_max <= end {
            size_max
        } else if end > begin {
            end - begin
        } else {
            0
        }
    }
    fn intersection(&self, other: &SRect) -> SRect {
        let x0 = self.x0.max(other.x0);
        let y0 = self.y0.max(other.y0);
        SRect::new(
            x0,
            y0,
            Self::clamped_size(x0, self.xsize, self.x1().min(other.x1())),
            Self::clamped_size(y0, self.ysize, self.y1().min(other.y1())),
        )
    }
    fn translate(&self, x_offset: i64, y_offset: i64) -> SRect {
        SRect::new(self.x0 + x_offset, self.y0 + y_offset, self.xsize, self.ysize)
    }
    fn shift_left(&self, shift: usize) -> SRect {
        SRect::new(
            self.x0 * (1 << shift),
            self.y0 * (1 << shift),
            self.xsize << shift,
            self.ysize << shift,
        )
    }
    fn ceil_shift_right(&self, shift: (usize, usize)) -> SRect {
        // JXL_ASSERT(x0_ % (1 << shiftx) == 0); JXL_ASSERT(y0_ % (1 << shifty) == 0);
        debug_assert!(self.x0 % (1 << shift.0) == 0);
        debug_assert!(self.y0 % (1 << shift.1) == 0);
        SRect::new(
            self.x0 / (1 << shift.0),
            self.y0 / (1 << shift.1),
            (self.xsize + (1 << shift.0) - 1) / (1 << shift.0),
            (self.ysize + (1 << shift.1) - 1) / (1 << shift.1),
        )
    }
}

/// Translation of `RenderPipeline` together with `LowMemoryRenderPipeline`.
pub(crate) struct RenderPipeline {
    // --- RenderPipeline
    stages: Vec<Box<dyn RenderPipelineStage>>,
    // Shifts for every channel at the input of each stage.
    channel_shifts: Vec<Vec<(usize, usize)>>,

    // Amount of (cumulative) padding required by each stage and channel, in
    // either direction.
    padding: Vec<Vec<(usize, usize)>>,

    frame_dimensions: FrameDimensions,

    group_completed_passes: Vec<u8>,

    // --- LowMemoryRenderPipeline
    use_group_ids: bool,

    // All the planes of the pipeline: group data, stage data and out-of-frame
    // data (upstream, separate vectors of images).
    arena: Vec<ImageF>,

    // Storage for borders between groups. Borders of adjacent groups are stacked
    // together, e.g. bottom border of current group is followed by top border
    // of next group.
    borders_horizontal: Vec<ImageF>,
    borders_vertical: Vec<ImageF>,

    // Manages the status of borders.
    group_border_assigner: GroupBorderAssigner,

    // Size (in color-channel-pixels) of the border around each group that might
    // be assigned to that group.
    group_border: (usize, usize),
    // base_color_shift_ defines the size of groups in terms of final image
    // pixels.
    base_color_shift: usize,

    // Buffer for decoded pixel data for a group, indexed by [thread][channel] or
    // [group][channel] depending on `use_group_ids_` (arena indices).
    group_data: Vec<Vec<usize>>,

    // Borders for storing group data.
    group_data_x_border: usize,
    group_data_y_border: usize,

    // Buffers for intermediate rows for the various stages, indexed by
    // [thread][channel][stage] (arena indices).
    stage_data: Vec<Vec<Vec<Option<usize>>>>,

    // Buffers for out-of-frame data, indexed by [thread]; every row is a
    // different channel (arena indices).
    out_of_frame_data: Vec<usize>,

    // For each stage, a non-kIgnored channel.
    anyc: Vec<usize>,

    // Size of the image at each stage.
    image_rect: Vec<Rect>,

    // For each stage, for each channel, keep track of the kInOut stage that
    // produced the input to that stage (which corresponds to the buffer index
    // containing the data). -1 if data comes from the original input.
    stage_input_for_channel: Vec<Vec<i32>>,

    // Number of (virtual) extra rows that must be processed at each stage
    // to produce sufficient output for future stages.
    virtual_ypadding_for_output: Vec<i32>,

    // Same thing for columns, except these are real columns and not virtual ones.
    xpadding_for_output: Vec<i32>,

    // First stage that doesn't have any kInOut channel.
    first_trailing_stage: usize,

    // Origin and size of the frame after switching to image dimensions.
    frame_origin: FrameOrigin,
    full_image_xsize: usize,
    full_image_ysize: usize,
    first_image_dim_stage: usize,
}

/// Information about where the *output* of each stage is stored.
/// Translation of `Rows`.
struct Rows {
    rows: Vec<Vec<RowsInfo>>,
}

#[derive(Clone, Copy, Default)]
struct RowsInfo {
    // Pointer to beginning of the first row.
    base_ptr: RowPtr,
    // Modulo value for the y axis minus 1 (ymod is guaranteed to be a power of
    // 2, which allows efficient mod computation by masking).
    ymod_minus_1: i64,
    // Number of floats per row.
    stride: usize,
}

impl Rows {
    /// Stage -1 refers to the input data; all other values must be
    /// nonnegative and refer to the data for the output of that stage.
    #[inline]
    fn get_buffer(&self, stage: i32, y: i64, c: usize) -> RowPtr {
        let info = &self.rows[(stage + 1) as usize][c];
        info.base_ptr.add((info.stride as i64 * (y & info.ymod_minus_1)) as isize)
    }
}

#[inline]
fn get_mirrored_y(y: i64, group_y0: i64, image_ysize: i64) -> i64 {
    if group_y0 == 0 && (y < 0 || y + group_y0 >= image_ysize) {
        return mirror(y, image_ysize);
    }
    if y + group_y0 >= image_ysize {
        // Here we know that the one mirroring step is sufficient.
        return 2 * image_ysize - (y + group_y0) - 1 - group_y0;
    }
    y
}

#[inline]
fn apply_x_mirroring(plane: &mut ImageF, row: RowPtr, borderx: i64, group_x0: i64, group_xsize: i64, image_xsize: i64) {
    let data = plane.data_mut();
    let at = |i: i64| -> usize { (row.off as i64 + i) as usize };
    let off = K_RENDER_PIPELINE_X_OFFSET as i64;
    if image_xsize <= borderx {
        if group_x0 == 0 {
            for ix in 0..borderx {
                data[at(off - ix - 1)] = data[at(off + mirror(-ix - 1, image_xsize))];
            }
        }
        if group_xsize + borderx + group_x0 >= image_xsize {
            for ix in 0..borderx {
                data[at(off + image_xsize + ix - group_x0)] =
                    data[at(off + mirror(image_xsize + ix, image_xsize) - group_x0)];
            }
        }
    } else {
        // Here we know that the one mirroring step is sufficient.
        if group_x0 == 0 {
            for ix in 0..borderx {
                data[at(off - ix - 1)] = data[at(off + ix)];
            }
        }
        if group_xsize + borderx + group_x0 >= image_xsize {
            for ix in 0..borderx {
                data[at(off + image_xsize - group_x0 + ix)] = data[at(off + image_xsize - group_x0 - ix - 1)];
            }
        }
    }
}

impl RenderPipeline {
    /// The end of `RenderPipeline::Builder::Finalize()` and
    /// `LowMemoryRenderPipeline::Init()`.
    pub(crate) fn new(
        stages: Vec<Box<dyn RenderPipelineStage>>,
        channel_shifts: Vec<Vec<(usize, usize)>>,
        padding: Vec<Vec<(usize, usize)>>,
        frame_dimensions: FrameDimensions,
        ctx: &mut StageCtx<'_>,
    ) -> Result<RenderPipeline, StatusCode> {
        let num_groups = frame_dimensions.num_groups;
        let mut p = RenderPipeline {
            stages,
            channel_shifts,
            padding,
            frame_dimensions,
            group_completed_passes: vec![0; num_groups],
            use_group_ids: false,
            arena: Vec::new(),
            borders_horizontal: Vec::new(),
            borders_vertical: Vec::new(),
            group_border_assigner: GroupBorderAssigner::default(),
            group_border: (0, 0),
            base_color_shift: 0,
            group_data: Vec::new(),
            group_data_x_border: 0,
            group_data_y_border: 0,
            stage_data: Vec::new(),
            out_of_frame_data: Vec::new(),
            anyc: Vec::new(),
            image_rect: Vec::new(),
            stage_input_for_channel: Vec::new(),
            virtual_ypadding_for_output: Vec::new(),
            xpadding_for_output: Vec::new(),
            first_trailing_stage: 0,
            frame_origin: FrameOrigin { x0: 0, y0: 0 },
            full_image_xsize: 0,
            full_image_ysize: 0,
            first_image_dim_stage: 0,
        };
        p.init(ctx)?;
        Ok(p)
    }

    /// Translation of `IsInitialized()`.
    pub(crate) fn is_initialized(&self) -> Status {
        for stage in &self.stages {
            stage.is_initialized()?;
        }
        Ok(())
    }

    /// Translation of `PassesWithAllInput()`.
    #[allow(dead_code)]
    pub(crate) fn passes_with_all_input(&self) -> usize {
        self.group_completed_passes.iter().copied().min().unwrap_or(0) as usize
    }

    /// Translation of `ColorDimensionsToChannelDimensions()`.
    fn color_dimensions_to_channel_dimensions(&self, input: (usize, usize), c: usize, stage: usize) -> (usize, usize) {
        let shift = self.channel_shifts[stage][c];
        (
            ((input.0 << self.base_color_shift) + (1 << shift.0) - 1) >> shift.0,
            ((input.1 << self.base_color_shift) + (1 << shift.1) - 1) >> shift.1,
        )
    }

    /// Translation of `BorderToStore()`.
    fn border_to_store(&self, c: usize) -> (usize, usize) {
        let mut ret = self.color_dimensions_to_channel_dimensions(self.group_border, c, 0);
        ret.0 += self.padding[0][c].0;
        ret.1 += self.padding[0][c].1;
        ret
    }

    /// Translation of `SaveBorders()`.
    fn save_borders(&mut self, group_id: usize, c: usize, in_idx: usize) {
        let fd = &self.frame_dimensions;
        let gy = group_id / fd.xsize_groups;
        let gx = group_id % fd.xsize_groups;
        let hshift = self.channel_shifts[0][c].0;
        let vshift = self.channel_shifts[0][c].1;
        let x0 = gx * self.group_input_x_size(c);
        let x1 = ((gx + 1) * self.group_input_x_size(c)).min(div_ceil(fd.xsize_upsampled, 1 << hshift));
        let y0 = gy * self.group_input_y_size(c);
        let y1 = ((gy + 1) * self.group_input_y_size(c)).min(div_ceil(fd.ysize_upsampled, 1 << vshift));

        let borders = self.border_to_store(c);
        let borderx_write = borders.0;
        let bordery_write = borders.1;
        let gxb = self.group_data_x_border;
        let gyb = self.group_data_y_border;
        let xsize_groups = fd.xsize_groups;
        let ysize_groups = fd.ysize_groups;

        let input = &self.arena[in_idx];
        if gy > 0 {
            let from = Rect::new(gxb, gyb, x1 - x0, bordery_write);
            let to = Rect::new(x0, (gy * 2 - 1) * bordery_write, x1 - x0, bordery_write);
            copy_image_to_rect(&from, input, &to, &mut self.borders_horizontal[c]);
        }
        if gy + 1 < ysize_groups {
            let from = Rect::new(gxb, gyb + y1 - y0 - bordery_write, x1 - x0, bordery_write);
            let to = Rect::new(x0, (gy * 2) * bordery_write, x1 - x0, bordery_write);
            copy_image_to_rect(&from, input, &to, &mut self.borders_horizontal[c]);
        }
        if gx > 0 {
            let from = Rect::new(gxb, gyb, borderx_write, y1 - y0);
            let to = Rect::new((gx * 2 - 1) * borderx_write, y0, borderx_write, y1 - y0);
            copy_image_to_rect(&from, input, &to, &mut self.borders_vertical[c]);
        }
        if gx + 1 < xsize_groups {
            let from = Rect::new(gxb + x1 - x0 - borderx_write, gyb, borderx_write, y1 - y0);
            let to = Rect::new((gx * 2) * borderx_write, y0, borderx_write, y1 - y0);
            copy_image_to_rect(&from, input, &to, &mut self.borders_vertical[c]);
        }
    }

    /// Translation of `LoadBorders()`.
    fn load_borders(&mut self, group_id: usize, c: usize, r: &Rect, out_idx: usize) {
        let fd = &self.frame_dimensions;
        let gy = group_id / fd.xsize_groups;
        let gx = group_id % fd.xsize_groups;
        let hshift = self.channel_shifts[0][c].0;
        let vshift = self.channel_shifts[0][c].1;
        // Coordinates of the group in the image.
        let x0 = gx * self.group_input_x_size(c);
        let x1 = ((gx + 1) * self.group_input_x_size(c)).min(div_ceil(fd.xsize_upsampled, 1 << hshift));
        let y0 = gy * self.group_input_y_size(c);
        let y1 = ((gy + 1) * self.group_input_y_size(c)).min(div_ceil(fd.ysize_upsampled, 1 << vshift));

        let paddingx = self.padding[0][c].0;
        let paddingy = self.padding[0][c].1;

        let borders = self.border_to_store(c);
        let borderx_write = borders.0;
        let bordery_write = borders.1;
        let bcs = self.base_color_shift;
        let gxb = self.group_data_x_border;
        let gyb = self.group_data_y_border;

        // Limits of the area to copy from, in image coordinates.
        debug_assert!(r.x0() == 0 || (r.x0() << bcs) >= paddingx);
        let mut x0src = div_ceil(r.x0() << bcs, 1 << hshift);
        if x0src != 0 {
            x0src = x0src.wrapping_sub(paddingx);
        }
        // r may be such that r.x1 (namely x0() + xsize()) is within paddingx of the
        // right side of the image, so we use min() here.
        let mut x1src = div_ceil((r.x0() + r.xsize()) << bcs, 1 << hshift);
        x1src = (x1src + paddingx).min(div_ceil(fd.xsize_upsampled, 1 << hshift));

        // Similar computation for y.
        debug_assert!(r.y0() == 0 || (r.y0() << bcs) >= paddingy);
        let mut y0src = div_ceil(r.y0() << bcs, 1 << vshift);
        if y0src != 0 {
            y0src = y0src.wrapping_sub(paddingy);
        }
        let mut y1src = div_ceil((r.y0() + r.ysize()) << bcs, 1 << vshift);
        y1src = (y1src + paddingy).min(div_ceil(fd.ysize_upsampled, 1 << vshift));

        let ysize_groups = fd.ysize_groups;
        let xsize_groups = fd.xsize_groups;
        let out = &mut self.arena[out_idx];
        // Copy other groups' borders from the border storage.
        if y0src < y0 {
            debug_assert!(gy > 0);
            copy_image_to_rect(
                &Rect::new(x0src, (gy * 2 - 2) * bordery_write, x1src - x0src, bordery_write),
                &self.borders_horizontal[c],
                &Rect::new(gxb + x0src - x0, gyb - bordery_write, x1src - x0src, bordery_write),
                out,
            );
        }
        if y1src > y1 {
            // When copying the bottom border we must not be on the bottom groups.
            debug_assert!(gy + 1 < ysize_groups);
            let _ = ysize_groups;
            copy_image_to_rect(
                &Rect::new(x0src, (gy * 2 + 1) * bordery_write, x1src - x0src, bordery_write),
                &self.borders_horizontal[c],
                &Rect::new(gxb + x0src - x0, gyb + y1 - y0, x1src - x0src, bordery_write),
                out,
            );
        }
        if x0src < x0 {
            debug_assert!(gx > 0);
            copy_image_to_rect(
                &Rect::new((gx * 2 - 2) * borderx_write, y0src, borderx_write, y1src - y0src),
                &self.borders_vertical[c],
                &Rect::new(gxb - borderx_write, gyb + y0src - y0, borderx_write, y1src - y0src),
                out,
            );
        }
        if x1src > x1 {
            // When copying the right border we must not be on the rightmost groups.
            debug_assert!(gx + 1 < xsize_groups);
            let _ = xsize_groups;
            copy_image_to_rect(
                &Rect::new((gx * 2 + 1) * borderx_write, y0src, borderx_write, y1src - y0src),
                &self.borders_vertical[c],
                &Rect::new(gxb + x1 - x0, gyb + y0src - y0, borderx_write, y1src - y0src),
                out,
            );
        }
    }

    /// Translation of `GroupInputXSize()`.
    fn group_input_x_size(&self, c: usize) -> usize {
        (self.frame_dimensions.group_dim << self.base_color_shift) >> self.channel_shifts[0][c].0
    }

    /// Translation of `GroupInputYSize()`.
    fn group_input_y_size(&self, c: usize) -> usize {
        (self.frame_dimensions.group_dim << self.base_color_shift) >> self.channel_shifts[0][c].1
    }

    /// Translation of `EnsureBordersStorage()`.
    fn ensure_borders_storage(&mut self) -> Status {
        let nc = self.channel_shifts[0].len();
        if self.borders_horizontal.len() < nc {
            self.borders_horizontal.resize_with(nc, ImageF::empty);
            self.borders_vertical.resize_with(nc, ImageF::empty);
        }
        for c in 0..nc {
            let shifts = self.channel_shifts[0][c];
            let borders = self.border_to_store(c);
            let borderx = borders.0;
            let bordery = borders.1;
            let fd = &self.frame_dimensions;
            debug_assert!(fd.xsize_groups > 0);
            let num_xborders = (fd.xsize_groups - 1) * 2;
            debug_assert!(fd.ysize_groups > 0);
            let num_yborders = (fd.ysize_groups - 1) * 2;
            let downsampled_xsize = div_ceil(fd.xsize_upsampled_padded, 1 << shifts.0);
            let downsampled_ysize = div_ceil(fd.ysize_upsampled_padded, 1 << shifts.1);
            let horizontal = Rect::new(0, 0, downsampled_xsize, bordery * num_yborders);
            if !(horizontal.xsize() == self.borders_horizontal[c].xsize()
                && horizontal.ysize() == self.borders_horizontal[c].ysize())
            {
                self.borders_horizontal[c] = ImageF::new(horizontal.xsize(), horizontal.ysize())?;
            }
            let vertical = Rect::new(0, 0, borderx * num_xborders, downsampled_ysize);
            if !(vertical.xsize() == self.borders_vertical[c].xsize()
                && vertical.ysize() == self.borders_vertical[c].ysize())
            {
                self.borders_vertical[c] = ImageF::new(vertical.xsize(), vertical.ysize())?;
            }
        }
        Ok(())
    }

    /// Translation of `LowMemoryRenderPipeline::Init()`.
    fn init(&mut self, ctx: &mut StageCtx<'_>) -> Status {
        self.group_border = (0, 0);
        let fd = &self.frame_dimensions;
        self.base_color_shift = ceil_log2_nonzero_u64((fd.xsize_upsampled_padded / fd.xsize_padded) as u64);

        let nc = self.channel_shifts[0].len();

        // Ensure that each channel has enough many border pixels.
        for c in 0..nc {
            self.group_border.0 = self.group_border.0.max(div_ceil(
                self.padding[0][c].0 << self.channel_shifts[0][c].0,
                1 << self.base_color_shift,
            ));
            self.group_border.1 = self.group_border.1.max(div_ceil(
                self.padding[0][c].1 << self.channel_shifts[0][c].1,
                1 << self.base_color_shift,
            ));
        }

        // Ensure that all channels have an integer number of border pixels in the
        // input.
        for c in 0..nc {
            if self.channel_shifts[0][c].0 >= self.base_color_shift {
                self.group_border.0 = round_up_to(
                    self.group_border.0,
                    1 << (self.channel_shifts[0][c].0 - self.base_color_shift),
                );
            }
            if self.channel_shifts[0][c].1 >= self.base_color_shift {
                self.group_border.1 = round_up_to(
                    self.group_border.1,
                    1 << (self.channel_shifts[0][c].1 - self.base_color_shift),
                );
            }
        }
        // Ensure that the X border on color channels is a multiple of kBlockDim or
        // the vector size (required for EPF stages). Vectors on ARM NEON are never
        // wider than 4 floats, so rounding to multiples of 4 is enough.
        const K_GROUP_X_ALIGN: usize = 16;
        self.group_border.0 = round_up_to(self.group_border.0, K_GROUP_X_ALIGN);
        // Allocate borders in group images that are just enough for storing the
        // borders to be copied in, plus any rounding to ensure alignment.
        let mut max_border = (0usize, 0usize);
        for c in 0..nc {
            max_border.0 = self.border_to_store(c).0.max(max_border.0);
            max_border.1 = self.border_to_store(c).1.max(max_border.1);
        }
        self.group_data_x_border = round_up_to(max_border.0, K_GROUP_X_ALIGN);
        self.group_data_y_border = max_border.1;

        self.ensure_borders_storage()?;
        self.group_border_assigner.init(&self.frame_dimensions);

        self.first_trailing_stage = self.stages.len();
        while self.first_trailing_stage > 0 {
            let mut has_inout_c = false;
            for c in 0..nc {
                if self.stages[self.first_trailing_stage - 1].get_channel_mode(c) == RenderPipelineChannelMode::InOut {
                    has_inout_c = true;
                }
            }
            if has_inout_c {
                break;
            }
            self.first_trailing_stage -= 1;
        }

        self.first_image_dim_stage = self.stages.len();
        for i in 0..self.stages.len() {
            let fd = &self.frame_dimensions;
            let mut input_sizes: Vec<(usize, usize)> = vec![(0, 0); nc];
            for c in 0..nc {
                input_sizes[c] = (
                    div_ceil(fd.xsize_upsampled, 1 << self.channel_shifts[i][c].0),
                    div_ceil(fd.ysize_upsampled, 1 << self.channel_shifts[i][c].1),
                );
            }
            self.stages[i].set_input_sizes(&input_sizes, ctx)?;
            if self.stages[i].switch_to_image_dimensions() {
                // We don't allow kInOut after switching to image dimensions.
                if i < self.first_trailing_stage {
                    return Err(StatusCode::GenericError);
                }
                self.first_image_dim_stage = i + 1;
                let (xs, ys, origin) = self.stages[i].get_image_dimensions();
                self.full_image_xsize = xs;
                self.full_image_ysize = ys;
                self.frame_origin = origin;
                break;
            }
        }
        for i in self.first_image_dim_stage..self.stages.len() {
            if self.stages[i].switch_to_image_dimensions() {
                // JXL_ABORT("Cannot switch to image dimensions multiple times");
                return Err(StatusCode::GenericError);
            }
            let input_sizes: Vec<(usize, usize)> = vec![(self.full_image_xsize, self.full_image_ysize); nc];
            self.stages[i].set_input_sizes(&input_sizes, ctx)?;
        }

        self.anyc = vec![0; self.stages.len()];
        for i in 0..self.stages.len() {
            for c in 0..nc {
                if self.stages[i].get_channel_mode(c) != RenderPipelineChannelMode::Ignored {
                    self.anyc[i] = c;
                }
            }
        }

        self.stage_input_for_channel = vec![vec![0i32; nc]; self.stages.len()];
        for c in 0..nc {
            let mut input: i32 = -1;
            for i in 0..self.stages.len() {
                self.stage_input_for_channel[i][c] = input;
                if self.stages[i].get_channel_mode(c) == RenderPipelineChannelMode::InOut {
                    input = i as i32;
                }
            }
        }

        self.image_rect = vec![Rect::default(); self.stages.len()];
        for i in 0..self.stages.len() {
            let fd = &self.frame_dimensions;
            let x1 = div_ceil(fd.xsize_upsampled, 1 << self.channel_shifts[i][self.anyc[i]].0);
            let y1 = div_ceil(fd.ysize_upsampled, 1 << self.channel_shifts[i][self.anyc[i]].1);
            self.image_rect[i] = Rect::new(0, 0, x1, y1);
        }

        self.virtual_ypadding_for_output = vec![0; self.stages.len()];
        self.xpadding_for_output = vec![0; self.stages.len()];
        for c in 0..nc {
            let mut ypad: i32 = 0;
            let mut xpad: i32 = 0;
            for i in (0..self.stages.len()).rev() {
                if self.stages[i].get_channel_mode(c) != RenderPipelineChannelMode::Ignored {
                    self.virtual_ypadding_for_output[i] = ypad.max(self.virtual_ypadding_for_output[i]);
                    self.xpadding_for_output[i] = xpad.max(self.xpadding_for_output[i]);
                }
                if self.stages[i].get_channel_mode(c) == RenderPipelineChannelMode::InOut {
                    let s = self.stages[i].settings();
                    let sy = self.channel_shifts[i][c].1;
                    ypad = (div_ceil(ypad as usize, 1 << sy) as i32 + s.border_y as i32) << sy;
                    xpad = div_ceil(xpad as usize, 1 << s.shift_x) as i32 + s.border_x as i32;
                }
            }
        }
        Ok(())
    }

    /// Allocates a plane of the arena, reusing `slot` if given.
    fn alloc(&mut self, slot: Option<usize>, xsize: usize, ysize: usize) -> Result<usize, StatusCode> {
        let plane = ImageF::new(xsize, ysize)?;
        match slot {
            Some(i) => {
                self.arena[i] = plane;
                Ok(i)
            }
            None => {
                self.arena.push(plane);
                Ok(self.arena.len() - 1)
            }
        }
    }

    /// Allocates storage to run with `num` threads. If `use_group_ids` is
    /// true, storage is allocated for each group, not each thread. The
    /// behaviour is undefined if calling this function multiple times with
    /// a different value for `use_group_ids`. Translation of
    /// `PrepareForThreads()`.
    pub(crate) fn prepare_for_threads(&mut self, num: usize, use_group_ids: bool) -> Status {
        for stage in &mut self.stages {
            stage.prepare_for_threads(num)?;
        }
        self.prepare_for_threads_internal(num, use_group_ids)
    }

    /// Translation of `PrepareForThreadsInternal()`.
    fn prepare_for_threads_internal(&mut self, num: usize, use_group_ids: bool) -> Status {
        let nc = self.channel_shifts[0].len();

        self.use_group_ids = use_group_ids;
        let num_buffers = if self.use_group_ids {
            self.frame_dimensions.num_groups
        } else {
            num
        };
        for _t in self.group_data.len()..num_buffers {
            let mut v = Vec::with_capacity(nc);
            for c in 0..nc {
                let xs = self.group_input_x_size(c) + self.group_data_x_border * 2;
                let ys = self.group_input_y_size(c) + self.group_data_y_border * 2;
                v.push(self.alloc(None, xs, ys)?);
            }
            self.group_data.push(v);
        }
        // TODO(veluca): avoid reallocating buffers if not needed.
        self.stage_data.resize_with(num, Vec::new);
        let upsampling = 1usize << self.base_color_shift;
        let group_dim = self.frame_dimensions.group_dim * upsampling;
        let padding = 2 * self.group_data_x_border * upsampling + // maximum size of a rect
            2 * K_RENDER_PIPELINE_X_OFFSET; // extra padding for processing
        let stage_buffer_xsize = group_dim + padding;
        let nstages = self.stages.len();
        for t in 0..num {
            self.stage_data[t].resize_with(nc, Vec::new);
            for c in 0..nc {
                self.stage_data[t][c].resize(nstages, None);
                let mut next_y_border = 0usize;
                for i in (0..nstages).rev() {
                    if self.stages[i].get_channel_mode(c) == RenderPipelineChannelMode::InOut {
                        let s = self.stages[i].settings();
                        let mut stage_buffer_ysize = 2 * next_y_border + (1 << s.shift_y);
                        stage_buffer_ysize = 1 << ceil_log2_nonzero_u64(stage_buffer_ysize as u64);
                        next_y_border = s.border_y;
                        let slot = self.stage_data[t][c][i];
                        let idx = self.alloc(slot, stage_buffer_xsize, stage_buffer_ysize)?;
                        self.stage_data[t][c][i] = Some(idx);
                    }
                }
            }
        }
        if self.first_image_dim_stage != self.stages.len() {
            let fd = &self.frame_dimensions;
            let mut image_rect = SRect::new(0, 0, fd.xsize_upsampled as i64, fd.ysize_upsampled as i64);
            let full_image_rect = SRect::new(0, 0, self.full_image_xsize as i64, self.full_image_ysize as i64);
            image_rect = image_rect.translate(self.frame_origin.x0 as i64, self.frame_origin.y0 as i64);
            image_rect = image_rect.intersection(&full_image_rect);
            if image_rect.xsize == 0 || image_rect.ysize == 0 {
                image_rect = SRect::new(0, 0, 0, 0);
            }
            let left_padding = image_rect.x0 as usize;
            let middle_padding = group_dim;
            let right_padding = self.full_image_xsize.wrapping_sub(image_rect.x1() as usize);
            let out_of_frame_xsize = padding + left_padding.max(middle_padding.max(right_padding));
            self.out_of_frame_data.truncate(num);
            for t in 0..num {
                let slot = self.out_of_frame_data.get(t).copied();
                let idx = self.alloc(slot, out_of_frame_xsize, nc)?;
                if slot.is_none() {
                    self.out_of_frame_data.push(idx);
                }
            }
        }
        Ok(())
    }

    /// Retrieves a buffer where input data should be stored by the callee.
    /// When input has been provided for all buffers, the pipeline will
    /// complete its processing. Translation of `GetInputBuffers()` (and
    /// `PrepareBuffers()`).
    pub(crate) fn get_input_buffers(&self, group_id: usize, thread_id: usize) -> RenderPipelineInput {
        debug_assert!(group_id < self.group_completed_passes.len());
        let nc = self.channel_shifts[0].len();
        let fd = &self.frame_dimensions;
        let gx = group_id % fd.xsize_groups;
        let gy = group_id / fd.xsize_groups;
        let set = if self.use_group_ids { group_id } else { thread_id };
        let mut buffers = Vec::with_capacity(nc);
        for c in 0..nc {
            let idx = self.group_data[set][c];
            let rect = Rect::new_clamped(
                self.group_data_x_border,
                self.group_data_y_border,
                self.group_input_x_size(c),
                self.group_input_y_size(c),
                (div_ceil(fd.xsize_upsampled, 1 << self.channel_shifts[0][c].0))
                    .wrapping_sub(gx * self.group_input_x_size(c))
                    .wrapping_add(self.group_data_x_border),
                (div_ceil(fd.ysize_upsampled, 1 << self.channel_shifts[0][c].1))
                    .wrapping_sub(gy * self.group_input_y_size(c))
                    .wrapping_add(self.group_data_y_border),
            );
            buffers.push((idx, rect));
        }
        RenderPipelineInput {
            group_id,
            thread_id,
            buffers,
        }
    }

    /// A plane of the pipeline (a buffer of a `RenderPipelineInput`).
    pub(crate) fn buffer(&self, idx: usize) -> &ImageF {
        &self.arena[idx]
    }

    /// A plane of the pipeline (mutable).
    pub(crate) fn buffer_mut(&mut self, idx: usize) -> &mut ImageF {
        &mut self.arena[idx]
    }

    /// Three distinct planes of the pipeline (mutable).
    pub(crate) fn buffers_mut3(&mut self, idx: [usize; 3]) -> Result<[&mut ImageF; 3], StatusCode> {
        self.arena.get_disjoint_mut(idx).map_err(|_| StatusCode::GenericError)
    }

    /// Translation of `RenderPipelineInput::Done()` (`InputReady()`).
    pub(crate) fn input_ready(&mut self, input: &RenderPipelineInput, ctx: &mut StageCtx<'_>) {
        debug_assert!(input.group_id < self.group_completed_passes.len());
        self.group_completed_passes[input.group_id] = self.group_completed_passes[input.group_id].wrapping_add(1);
        self.process_buffers(input.group_id, input.thread_id, ctx);
    }

    /// Translation of `ClearDone()`.
    pub(crate) fn clear_done(&mut self, i: usize) {
        self.group_border_assigner.clear_done(i);
    }

    /// Translation of `RenderRect()`.
    fn render_rect(
        &mut self,
        thread_id: usize,
        input_data: &[usize],
        data_max_color_channel_rect: Rect,
        image_max_color_channel_rect: Rect,
        ctx: &mut StageCtx<'_>,
    ) {
        let nstages = self.stages.len();
        let nc = input_data.len();
        // For each stage, the rect corresponding to the image area currently being
        // processed, in the coordinates of that stage (i.e. with the scaling factor
        // that that stage has).
        let mut group_rect: Vec<Rect> = vec![Rect::default(); nstages];
        let image_area_rect = image_max_color_channel_rect
            .shift_left(self.base_color_shift, self.base_color_shift)
            .crop(self.frame_dimensions.xsize_upsampled, self.frame_dimensions.ysize_upsampled);
        for i in 0..nstages {
            let sh = self.channel_shifts[i][self.anyc[i]];
            group_rect[i] = image_area_rect.ceil_shift_right(sh.0, sh.1);
        }

        let frame_x0 = self.frame_origin.x0 as i64;
        let frame_y0 = self.frame_origin.y0 as i64;

        // Compute actual x-axis bounds for the current image area in the context of
        // the full image this frame is part of. As the left boundary may be negative,
        // we also create the x_pixels_skip value, defined as follows:
        // - both x_pixels_skip and full_image_x0 are >= 0, and at least one is 0;
        // - full_image_x0 - x_pixels_skip is the position of the current frame area
        //   in the full image.
        let mut full_image_x0 = frame_x0 + image_area_rect.x0() as i64;
        let mut x_pixels_skip: i64 = 0;
        if full_image_x0 < 0 {
            x_pixels_skip = -full_image_x0;
            full_image_x0 = 0;
        }
        let full_image_x1 = frame_x0 + image_area_rect.x1() as i64;

        let mut span: Vec<Rect> = vec![Rect::default(); nstages];
        for i in 0..nstages {
            if i < self.first_image_dim_stage {
                span[i] = Rect::new(group_rect[i].x0(), 0, group_rect[i].xsize(), self.image_rect[i].ysize());
            } else {
                let x0 = full_image_x0 as usize;
                let x1 = full_image_x1;
                let x_max = self.full_image_xsize as i64;
                let cropped_x1 = x1.min(x_max);
                span[i] = Rect::new(x0, 0, 0i64.max(cropped_x1 - x0 as i64) as usize, self.full_image_ysize);
            }
        }

        // Data structures to hold information about input/output rows and their
        // buffers.
        let rows = {
            let mut r = vec![vec![RowsInfo::default(); nc]; nstages + 1];
            for i in 0..nstages {
                for c in 0..nc {
                    if self.stages[i].get_channel_mode(c) == RenderPipelineChannelMode::InOut {
                        if let Some(idx) = self.stage_data[thread_id][c][i] {
                            let plane = &self.arena[idx];
                            r[i + 1][c].ymod_minus_1 = plane.ysize() as i64 - 1;
                            r[i + 1][c].base_ptr = RowPtr { buf: idx, off: 0 };
                            r[i + 1][c].stride = plane.pixels_per_row();
                        }
                    }
                }
            }
            for c in 0..nc {
                let channel_group_data_rect = SRect::from_rect(&data_max_color_channel_rect)
                    .translate(-(self.group_data_x_border as i64), -(self.group_data_y_border as i64))
                    .shift_left(self.base_color_shift)
                    .ceil_shift_right(self.channel_shifts[0][c])
                    .translate(
                        self.group_data_x_border as i64 - K_RENDER_PIPELINE_X_OFFSET as i64,
                        self.group_data_y_border as i64,
                    );
                let plane = &self.arena[input_data[c]];
                let stride = plane.pixels_per_row();
                r[0][c].base_ptr = RowPtr {
                    buf: input_data[c],
                    off: (channel_group_data_rect.y0 * stride as i64 + channel_group_data_rect.x0) as isize,
                };
                r[0][c].stride = stride;
                r[0][c].ymod_minus_1 = -1;
            }
            Rows { rows: r }
        };

        let fts = self.first_trailing_stage;
        let mut input_rows: Vec<RowInfo> = vec![Vec::new(); fts + 1];
        for row in input_rows.iter_mut().take(fts) {
            *row = vec![Vec::new(); nc];
        }
        input_rows[fts] = vec![vec![RowPtr::default(); 1]; nc];

        // Maximum possible shift is 3.
        let mut output_rows: RowInfo = vec![vec![RowPtr::default(); 8]; nc];

        // We pretend that every stage has a vertical shift of 0, i.e. it is as tall
        // as the final image.
        // We call each such row a "virtual" row, because it may or may not correspond
        // to an actual row of the current processing stage; actual processing happens
        // when vy % (1<<vshift) == 0.

        let num_extra_rows: i32 = self.virtual_ypadding_for_output.iter().copied().max().unwrap_or(0);

        let mut vy: i32 = -num_extra_rows;
        while vy < image_area_rect.ysize() as i32 + num_extra_rows {
            for i in 0..fts {
                let stage_vy = vy - num_extra_rows + self.virtual_ypadding_for_output[i];

                let vshift = self.channel_shifts[i][self.anyc[i]].1;
                if stage_vy % (1 << vshift) != 0 {
                    continue;
                }

                if stage_vy < -self.virtual_ypadding_for_output[i] {
                    continue;
                }

                let y = stage_vy >> vshift;

                let image_y = group_rect[i].y0() as i64 + y as i64;
                // Do not produce rows in out-of-bounds areas.
                if image_y < 0 {
                    continue;
                }
                if image_y >= span[i].y1() as i64 {
                    continue;
                }

                // Get the input/output rows and potentially apply mirroring to the input.
                // (prepare_io_rows)
                {
                    let s = self.stages[i].settings();
                    let bordery = s.border_y as i64;
                    let shifty = s.shift_y;
                    for c in 0..nc {
                        let mode = self.stages[i].get_channel_mode(c);
                        if mode == RenderPipelineChannelMode::Ignored {
                            continue;
                        }
                        let make_row = |arena: &mut Vec<ImageF>, iy: i64| -> RowPtr {
                            let mirrored_y = get_mirrored_y(
                                y as i64 + iy - bordery,
                                group_rect[i].y0() as i64,
                                self.image_rect[i].ysize() as i64,
                            );
                            let p = rows.get_buffer(self.stage_input_for_channel[i][c], mirrored_y, c);
                            apply_x_mirroring(
                                &mut arena[p.buf],
                                p,
                                s.border_x as i64,
                                group_rect[i].x0() as i64,
                                group_rect[i].xsize() as i64,
                                self.image_rect[i].xsize() as i64,
                            );
                            p
                        };
                        // If we already have rows from a previous iteration, we can just shift
                        // the rows by 1 and insert the new one.
                        if input_rows[i][c].len() == 2 * bordery as usize + 1 {
                            for iy in 0..(2 * bordery) as usize {
                                input_rows[i][c][iy] = input_rows[i][c][iy + 1];
                            }
                            let p = make_row(&mut self.arena, bordery * 2);
                            input_rows[i][c][(bordery * 2) as usize] = p;
                        } else {
                            input_rows[i][c].resize(2 * bordery as usize + 1, RowPtr::default());
                            for iy in 0..2 * bordery + 1 {
                                let p = make_row(&mut self.arena, iy);
                                input_rows[i][c][iy as usize] = p;
                            }
                        }

                        // If necessary, get the output buffers.
                        if mode == RenderPipelineChannelMode::InOut {
                            for iy in 0..(1usize << shifty) {
                                output_rows[c][iy] = rows.get_buffer(i as i32, (y as i64) * (1 << shifty) + iy as i64, c);
                            }
                        }
                    }
                }

                // Produce output rows.
                if span[i].xsize() == 0 {
                    continue;
                }
                let settings = self.stages[i].settings();
                let mut sr = StageRows {
                    input_rows: &input_rows[i],
                    output_rows: &output_rows,
                    bufs: &mut self.arena,
                    settings,
                };
                self.stages[i].process_row(
                    &mut sr,
                    self.xpadding_for_output[i] as usize,
                    span[i].xsize(),
                    span[i].x0(),
                    image_y as usize,
                    thread_id,
                    ctx,
                );
            }

            // Process trailing stages, i.e. the final set of non-kInOut stages; they
            // all have the same input buffer and no need to use any mirroring.

            let y = vy - num_extra_rows;

            for c in 0..nc {
                input_rows[fts][c][0] = rows.get_buffer(self.stage_input_for_channel[fts][c], y as i64, c);
            }

            // Check that we are not outside of the bounds for the current rendering
            // rect. Not doing so might result in overwriting some rows that have been
            // written (or will be written) by other threads.
            if y < 0 || y as i64 >= image_area_rect.ysize() as i64 {
                vy += 1;
                continue;
            }

            for i in fts..self.first_image_dim_stage {
                if span[i].xsize() == 0 {
                    continue;
                }
                let y0 = image_area_rect.y0() + y as usize;
                if y0 >= span[i].y1() {
                    continue;
                }
                let settings = self.stages[i].settings();
                let mut sr = StageRows {
                    input_rows: &input_rows[fts],
                    output_rows: &output_rows,
                    bufs: &mut self.arena,
                    settings,
                };
                self.stages[i].process_row(&mut sr, /*xextra=*/ 0, span[i].xsize(), span[i].x0(), y0, thread_id, ctx);
            }

            if self.first_image_dim_stage == nstages {
                vy += 1;
                continue;
            }

            // Skip pixels that are not part of the actual final image area.
            for c in 0..nc {
                input_rows[fts][c][0] = input_rows[fts][c][0].add(x_pixels_skip as isize);
            }
            // Avoid running pipeline stages on pixels that are outside the full image
            // area. As trailing stages have no borders, this is a free optimization
            // (and may be necessary for correctness, as some stages assume coordinates
            // are within bounds).
            let full_image_y = frame_y0 + image_area_rect.y0() as i64 + y as i64;
            if full_image_y < 0 {
                vy += 1;
                continue;
            }

            for i in self.first_image_dim_stage..nstages {
                if span[i].xsize() == 0 {
                    continue;
                }
                if full_image_y >= span[i].y1() as i64 {
                    continue;
                }
                let settings = self.stages[i].settings();
                let mut sr = StageRows {
                    input_rows: &input_rows[fts],
                    output_rows: &output_rows,
                    bufs: &mut self.arena,
                    settings,
                };
                self.stages[i].process_row(
                    &mut sr,
                    /*xextra=*/ 0,
                    span[i].xsize(),
                    span[i].x0(),
                    full_image_y as usize,
                    thread_id,
                    ctx,
                );
            }
            vy += 1;
        }
    }

    /// Translation of `RenderPadding()`.
    fn render_padding(&mut self, thread_id: usize, rect: Rect, ctx: &mut StageCtx<'_>) {
        if rect.xsize() == 0 {
            return;
        }
        let numc = self.channel_shifts[0].len();
        let oof = self.out_of_frame_data[thread_id];
        let stride = self.arena[oof].pixels_per_row();
        let input_rows: RowInfo = (0..numc)
            .map(|c| {
                vec![RowPtr {
                    buf: oof,
                    off: (c * stride) as isize,
                }]
            })
            .collect();
        let output_rows: RowInfo = Vec::new();

        for y in 0..rect.ysize() {
            {
                let stage = &self.stages[self.first_image_dim_stage - 1];
                let mut sr = StageRows {
                    input_rows: &input_rows,
                    output_rows: &output_rows,
                    bufs: &mut self.arena,
                    settings: stage.settings(),
                };
                stage.process_padding_row(&mut sr, rect.xsize(), rect.x0(), rect.y0() + y, ctx);
            }
            for i in self.first_image_dim_stage..self.stages.len() {
                let settings = self.stages[i].settings();
                let mut sr = StageRows {
                    input_rows: &input_rows,
                    output_rows: &output_rows,
                    bufs: &mut self.arena,
                    settings,
                };
                self.stages[i].process_row(&mut sr, /*xextra=*/ 0, rect.xsize(), rect.x0(), rect.y0() + y, thread_id, ctx);
            }
        }
    }

    /// Translation of `ProcessBuffers()`.
    fn process_buffers(&mut self, group_id: usize, thread_id: usize, ctx: &mut StageCtx<'_>) {
        let set = if self.use_group_ids { group_id } else { thread_id };
        let input_data: Vec<usize> = self.group_data[set].clone();

        // Copy the group borders to the border storage.
        for (c, &idx) in input_data.iter().enumerate() {
            self.save_borders(group_id, c, idx);
        }

        let fd = self.frame_dimensions.clone();
        let gy = group_id / fd.xsize_groups;
        let gx = group_id % fd.xsize_groups;

        if self.first_image_dim_stage != self.stages.len() {
            let group_dim = (fd.group_dim << self.base_color_shift) as i64;
            let mut group_rect = SRect::new(gx as i64 * group_dim, gy as i64 * group_dim, group_dim, group_dim);
            let mut image_rect = SRect::new(0, 0, fd.xsize_upsampled as i64, fd.ysize_upsampled as i64);
            let full_image_rect = SRect::new(0, 0, self.full_image_xsize as i64, self.full_image_ysize as i64);
            group_rect = group_rect.translate(self.frame_origin.x0 as i64, self.frame_origin.y0 as i64);
            image_rect = image_rect.translate(self.frame_origin.x0 as i64, self.frame_origin.y0 as i64);
            image_rect = image_rect.intersection(&full_image_rect);
            group_rect = group_rect.intersection(&image_rect);
            let x0 = group_rect.x0 as usize;
            let y0 = group_rect.y0 as usize;
            let x1 = group_rect.x1() as usize;
            let y1 = group_rect.y1() as usize;
            let fx = self.full_image_xsize;
            let fy = self.full_image_ysize;

            if group_id == 0 && (image_rect.xsize == 0 || image_rect.ysize == 0) {
                // If this frame does not intersect with the full image, we have to
                // initialize the whole image area with RenderPadding.
                self.render_padding(thread_id, Rect::new(0, 0, fx, fy), ctx);
            }

            // Render padding for groups that intersect with the full image. The case
            // where no groups intersect was handled above.
            if group_rect.xsize > 0 && group_rect.ysize > 0 {
                if gx == 0 && gy == 0 {
                    self.render_padding(thread_id, Rect::new(0, 0, x0, y0), ctx);
                }
                if gy == 0 {
                    self.render_padding(thread_id, Rect::new(x0, 0, x1 - x0, y0), ctx);
                }
                if gx == 0 {
                    self.render_padding(thread_id, Rect::new(0, y0, x0, y1 - y0), ctx);
                }
                if gx == 0 && gy + 1 == fd.ysize_groups {
                    self.render_padding(thread_id, Rect::new(0, y1, x0, fy - y1), ctx);
                }
                if gy + 1 == fd.ysize_groups {
                    self.render_padding(thread_id, Rect::new(x0, y1, x1 - x0, fy - y1), ctx);
                }
                if gy == 0 && gx + 1 == fd.xsize_groups {
                    self.render_padding(thread_id, Rect::new(x1, 0, fx - x1, y0), ctx);
                }
                if gx + 1 == fd.xsize_groups {
                    self.render_padding(thread_id, Rect::new(x1, y0, fx - x1, y1 - y0), ctx);
                }
                if gy + 1 == fd.ysize_groups && gx + 1 == fd.xsize_groups {
                    self.render_padding(thread_id, Rect::new(x1, y1, fx - x1, fy - y1), ctx);
                }
            }
        }

        let ready_rects = self.group_border_assigner.group_done(group_id, self.group_border.0, self.group_border.1);
        for image_max_color_channel_rect in ready_rects {
            for (c, &idx) in input_data.iter().enumerate() {
                self.load_borders(group_id, c, &image_max_color_channel_rect, idx);
            }
            let data_max_color_channel_rect = Rect::new(
                (self.group_data_x_border + image_max_color_channel_rect.x0()).wrapping_sub(gx * fd.group_dim),
                (self.group_data_y_border + image_max_color_channel_rect.y0()).wrapping_sub(gy * fd.group_dim),
                image_max_color_channel_rect.xsize(),
                image_max_color_channel_rect.ysize(),
            );
            self.render_rect(
                thread_id,
                &input_data,
                data_max_color_channel_rect,
                image_max_color_channel_rect,
                ctx,
            );
        }
    }
}

