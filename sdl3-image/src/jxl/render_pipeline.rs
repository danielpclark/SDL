// Rust translation of lib/jxl/render_pipeline/render_pipeline.h,
// lib/jxl/render_pipeline/render_pipeline.cc and
// lib/jxl/render_pipeline/render_pipeline_stage.h from libjxl
// (https://github.com/libjxl/libjxl, at the revision SDL_image's
// external/libjxl pins: libjxl 0.7.3 with SDL's patches).
// Copyright (c) the JPEG XL Project Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The rendering pipeline: the stages that turn decoded (group) data into
//! output pixels.
//!
//! Upstream, stages see rows as `float*` pointers into the pipeline's
//! buffers; here a row is a [`RowPtr`] (a buffer of the pipeline's arena and
//! the index of the pixel the pointer points to), read and written through
//! [`StageRows`]. The objects stages write to (the output image bundle, the
//! RGB8 buffer, ...) and the state they read (reference frames, image
//! features, ...) are passed in a [`StageCtx`] instead of being pointed to
//! by the stages. Only the low-memory pipeline is translated: the decoder
//! never asks for the simple one.

pub(crate) mod low_memory_render_pipeline;
pub(crate) mod stage_blending;
pub(crate) mod stage_chroma_upsampling;
pub(crate) mod stage_epf;
pub(crate) mod stage_from_linear;
pub(crate) mod stage_gaborish;
pub(crate) mod stage_noise;
pub(crate) mod stage_patches;
pub(crate) mod stage_splines;
pub(crate) mod stage_spot;
pub(crate) mod stage_to_linear;
pub(crate) mod stage_tone_mapping;
pub(crate) mod stage_upsampling;
pub(crate) mod stage_write;
pub(crate) mod stage_xyb;
pub(crate) mod stage_ycbcr;

use super::base::{Status, StatusCode};
use super::passes_state::ReferenceFrame;
use super::frame_header::FrameOrigin;
use super::image::{Image3F, ImageF, Rect};
use super::image_bundle::ImageBundle;
use super::passes_state::ImageFeatures;

pub(crate) use low_memory_render_pipeline::RenderPipeline;

// --- render_pipeline_stage.h ---

/// The first pixel in the input to RenderPipelineStage will be located at
/// this position. Pixels before this position may be accessed as padding.
/// This should be at least the RoundUpTo(maximum padding / 2, maximum vector
/// size) times 2: this is realized when using Gaborish + EPF + upsampling +
/// chroma subsampling.
pub(crate) const K_RENDER_PIPELINE_X_OFFSET: usize = 32;

/// Translation of `RenderPipelineChannelMode`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum RenderPipelineChannelMode {
    /// This channel is not modified by this stage.
    Ignored = 0,
    /// This channel is modified in-place.
    InPlace = 1,
    /// This channel is modified and written to a new buffer.
    InOut = 2,
    /// This channel is only read. These are the only stages that are assumed
    /// to have observable effects, i.e. calls to ProcessRow for other stages
    /// may be omitted if it can be shown they can't affect any kInput stage
    /// ProcessRow call that happens inside image boundaries.
    Input = 3,
}

/// Translation of `RenderPipelineStage::Settings`.
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct Settings {
    /// Amount of padding required in the various directions by all channels
    /// that have kInOut mode.
    pub border_x: usize,
    pub border_y: usize,

    /// Log2 of the number of columns/rows of output that this stage will
    /// produce for every input row for kInOut channels.
    pub shift_x: usize,
    pub shift_y: usize,
}

impl Settings {
    pub(crate) fn shift_x(shift: usize, border: usize) -> Settings {
        Settings {
            border_x: border,
            shift_x: shift,
            ..Settings::default()
        }
    }

    pub(crate) fn shift_y(shift: usize, border: usize) -> Settings {
        Settings {
            border_y: border,
            shift_y: shift,
            ..Settings::default()
        }
    }

    pub(crate) fn symmetric(shift: usize, border: usize) -> Settings {
        Settings {
            border_x: border,
            border_y: border,
            shift_x: shift,
            shift_y: shift,
        }
    }

    pub(crate) fn symmetric_border_only(border: usize) -> Settings {
        Settings::symmetric(0, border)
    }
}

/// A row of a pipeline buffer: the buffer (an index into the pipeline's
/// arena of planes) and the index, in that plane's storage, of the pixel the
/// upstream `float*` points to. The index may be negative or past a row's
/// end, exactly like the pointer; only the pixels a stage accesses must be
/// within the storage.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct RowPtr {
    pub buf: usize,
    pub off: isize,
}

impl RowPtr {
    /// The row pointer advanced by `n` pixels.
    #[inline]
    pub(crate) fn add(self, n: isize) -> RowPtr {
        RowPtr {
            buf: self.buf,
            off: self.off + n,
        }
    }
}

/// Rows of each channel. Translation of `RenderPipelineStage::RowInfo`.
pub(crate) type RowInfo = Vec<Vec<RowPtr>>;

/// The rows a stage processes, over the pipeline's buffers.
pub(crate) struct StageRows<'a> {
    pub input_rows: &'a RowInfo,
    pub output_rows: &'a RowInfo,
    pub bufs: &'a mut [ImageF],
    pub settings: Settings,
}

impl StageRows<'_> {
    /// Returns a pointer to the input row of channel `c` with offset `y`.
    /// `y` must be in [-settings_.border_y, settings_.border_y]. `c` must be
    /// such that `GetChannelMode(c) != kIgnored`. The returned pointer points
    /// to the offset-ed row (i.e. kRenderPipelineXOffset has been applied).
    /// Translation of `GetInputRow()`.
    #[inline]
    pub(crate) fn get_input_row(&self, c: usize, offset: isize) -> RowPtr {
        debug_assert!(-offset <= self.settings.border_y as isize);
        debug_assert!(offset <= self.settings.border_y as isize);
        self.input_rows[c][(self.settings.border_y as isize + offset) as usize].add(K_RENDER_PIPELINE_X_OFFSET as isize)
    }

    /// Similar to `GetInputRow`, but can only be used if `GetChannelMode(c)
    /// == kInOut`. Offset must be less than `1<<settings_.shift_y`.. The
    /// returned pointer points to the offset-ed row (i.e.
    /// kRenderPipelineXOffset has been applied). Translation of
    /// `GetOutputRow()`.
    #[inline]
    pub(crate) fn get_output_row(&self, c: usize, offset: usize) -> RowPtr {
        debug_assert!(offset <= 1usize << self.settings.shift_y);
        self.output_rows[c][offset].add(K_RENDER_PIPELINE_X_OFFSET as isize)
    }

    /// The `n` pixels of the row starting at offset `x` from the pointer.
    #[inline]
    pub(crate) fn row(&self, p: RowPtr, x: isize, n: usize) -> &[f32] {
        let s = (p.off + x) as usize;
        &self.bufs[p.buf].data()[s..s + n]
    }

    /// The `n` pixels of the row starting at offset `x` from the pointer
    /// (mutable).
    #[inline]
    pub(crate) fn row_mut(&mut self, p: RowPtr, x: isize, n: usize) -> &mut [f32] {
        let s = (p.off + x) as usize;
        &mut self.bufs[p.buf].data_mut()[s..s + n]
    }

    /// A copy of `n` pixels of the row starting at offset `x`.
    #[inline]
    pub(crate) fn load(&self, p: RowPtr, x: isize, n: usize) -> Vec<f32> {
        self.row(p, x, n).to_vec()
    }

    /// Writes `v` to the row starting at offset `x`.
    #[inline]
    pub(crate) fn store(&mut self, p: RowPtr, x: isize, v: &[f32]) {
        self.row_mut(p, x, v.len()).copy_from_slice(v);
    }
}

/// What the stages write to and read from, besides the pipeline's buffers.
/// Upstream, the stages hold pointers to these objects.
pub(crate) struct StageCtx<'a> {
    /// The output image bundle (`decoded`).
    pub decoded: &'a mut ImageBundle,
    /// `PassesDecoderState::frame_storage_for_referencing`.
    pub frame_storage_for_referencing: &'a mut ImageBundle,
    /// `PassesSharedState::dc_frames`.
    pub dc_frames: &'a mut [Image3F; 4],
    /// The RGB8 output buffer (`PassesDecoderState::rgb_output`).
    pub rgb_output: &'a mut Option<Vec<u8>>,
    /// `PassesSharedState::reference_frames`.
    pub reference_frames: &'a [ReferenceFrame; 4],
    /// `PassesSharedState::image_features`.
    pub image_features: &'a ImageFeatures,
    /// `PassesDecoderState::sigma`.
    pub sigma: &'a ImageF,
}

/// Translation of `RenderPipelineStage`.
pub(crate) trait RenderPipelineStage {
    /// Translation of `settings_`.
    fn settings(&self) -> Settings;

    /// Processes one row of input, producing the appropriate number of rows
    /// of output. Input/output rows can be obtained by calls to
    /// `GetInputRow`/`GetOutputRow`. `xsize+2*xextra` represents the total
    /// number of pixels to be processed in the input row, where the first
    /// pixel is at position `kRenderPipelineXOffset-xextra`. All pixels in
    /// the `[kRenderPipelineXOffset-xextra-border_x,
    /// kRenderPipelineXOffset+xsize+xextra+border_x)` range are initialized
    /// and accessible. `xpos` and `ypos` represent the position of the first
    /// (non-extra, i.e. in position kRenderPipelineXOffset) pixel in the
    /// center row of the input in the full image. `xpos` is a multiple of
    /// `GroupBorderAssigner::kPaddingXRound`. If `settings_.temp_buffer_size`
    /// is nonzero, `temp` will point to an HWY-aligned buffer of at least
    /// that number of floats; concurrent calls will have different buffers.
    #[allow(clippy::too_many_arguments)]
    fn process_row(
        &self,
        rows: &mut StageRows<'_>,
        xextra: usize,
        xsize: usize,
        xpos: usize,
        ypos: usize,
        thread_id: usize,
        ctx: &mut StageCtx<'_>,
    );

    /// How each channel will be processed. Channels are numbered starting
    /// from color channels (always 3) and followed by all other channels.
    fn get_channel_mode(&self, c: usize) -> RenderPipelineChannelMode;

    fn is_initialized(&self) -> Status {
        Ok(())
    }

    /// Informs the stage about the total size of each channel. Few stages
    /// will actually need to use this information.
    fn set_input_sizes(&mut self, _input_sizes: &[(usize, usize)], _ctx: &mut StageCtx<'_>) -> Status {
        Ok(())
    }

    fn prepare_for_threads(&mut self, _num_threads: usize) -> Status {
        Ok(())
    }

    /// Indicates whether, from this stage on, the pipeline will operate on
    /// an image- rather than frame-sized buffer. Only one stage in the
    /// pipeline should return true, and it should implement
    /// ProcessPaddingRow below too. It is assumed that, if there is a
    /// SwitchToImageDimensions() == true stage, all kInput stages appear
    /// after it.
    fn switch_to_image_dimensions(&self) -> bool {
        false
    }

    /// If SwitchToImageDimensions returns true, then this should set xsize
    /// and ysize to the image size, and frame_origin to the location of the
    /// frame within the image. Otherwise, this is not called at all.
    fn get_image_dimensions(&self) -> (usize, usize, FrameOrigin) {
        (0, 0, FrameOrigin { x0: 0, y0: 0 })
    }

    /// Produces the appropriate output data outside of the frame dimensions.
    /// xpos and ypos are now relative to the full image.
    fn process_padding_row(
        &self,
        _rows: &mut StageRows<'_>,
        _xsize: usize,
        _xpos: usize,
        _ypos: usize,
        _ctx: &mut StageCtx<'_>,
    ) {
    }

    #[allow(dead_code)]
    fn get_name(&self) -> &'static str;
}

// --- render_pipeline.h ---

/// Translation of `RenderPipelineInput`: the buffers (arena indices, and the
/// rect to write) where the input of a group must be stored.
#[derive(Debug, Default)]
pub(crate) struct RenderPipelineInput {
    pub group_id: usize,
    pub thread_id: usize,
    pub buffers: Vec<(usize, Rect)>,
}

impl RenderPipelineInput {
    /// Translation of `GetBuffer()`.
    pub(crate) fn get_buffer(&self, c: usize) -> Result<(usize, Rect), StatusCode> {
        // JXL_ASSERT(c < buffers_.size())
        self.buffers.get(c).copied().ok_or(StatusCode::GenericError)
    }
}

/// Translation of `RenderPipeline::Builder`.
pub(crate) struct Builder {
    stages: Vec<Box<dyn RenderPipelineStage>>,
    num_c: usize,
}

impl Builder {
    pub(crate) fn new(num_c: usize) -> Builder {
        debug_assert!(num_c > 0);
        Builder {
            stages: Vec::new(),
            num_c,
        }
    }

    /// Adds a stage to the pipeline. Must be called at least once; the last
    /// added stage cannot have kInOut channels.
    pub(crate) fn add_stage(&mut self, stage: Box<dyn RenderPipelineStage>) {
        self.stages.push(stage);
    }

    /// Finalizes setup of the pipeline. Shifts for all channels should be 0
    /// at this point. Translation of `Finalize()`.
    pub(crate) fn finalize(
        self,
        frame_dimensions: super::base::FrameDimensions,
        ctx: &mut StageCtx<'_>,
    ) -> Result<RenderPipeline, StatusCode> {
        // Check that the last stage is not an kInOut stage for any channel, and
        // that there is at least one stage.
        if self.stages.is_empty() {
            return Err(StatusCode::GenericError);
        }
        for c in 0..self.num_c {
            if self.stages[self.stages.len() - 1].get_channel_mode(c) == RenderPipelineChannelMode::InOut {
                return Err(StatusCode::GenericError);
            }
        }

        let stages = self.stages;
        let num_c = self.num_c;
        let mut padding: Vec<Vec<(usize, usize)>> = vec![Vec::new(); stages.len()];
        for i in (0..stages.len()).rev() {
            let stage = &stages[i];
            padding[i].resize(num_c, (0, 0));
            if i + 1 == stages.len() {
                continue;
            }
            let s = stage.settings();
            for c in 0..num_c {
                if stage.get_channel_mode(c) == RenderPipelineChannelMode::InOut {
                    padding[i][c].0 = super::base::div_ceil(padding[i + 1][c].0, 1 << s.shift_x) + s.border_x;
                    padding[i][c].1 = super::base::div_ceil(padding[i + 1][c].1, 1 << s.shift_y) + s.border_y;
                } else {
                    padding[i][c] = padding[i + 1][c];
                }
            }
        }

        let mut channel_shifts: Vec<Vec<(usize, usize)>> = vec![Vec::new(); stages.len()];
        channel_shifts[0].resize(num_c, (0, 0));
        for i in 1..stages.len() {
            let stage = &stages[i - 1];
            let s = stage.settings();
            for c in 0..num_c {
                if stage.get_channel_mode(c) == RenderPipelineChannelMode::InOut {
                    channel_shifts[0][c].0 += s.shift_x;
                    channel_shifts[0][c].1 += s.shift_y;
                }
            }
        }
        for i in 1..stages.len() {
            let stage = &stages[i - 1];
            let s = stage.settings();
            channel_shifts[i].resize(num_c, (0, 0));
            for c in 0..num_c {
                if stage.get_channel_mode(c) == RenderPipelineChannelMode::InOut {
                    channel_shifts[i][c].0 = channel_shifts[i - 1][c].0.wrapping_sub(s.shift_x);
                    channel_shifts[i][c].1 = channel_shifts[i - 1][c].1.wrapping_sub(s.shift_y);
                } else {
                    channel_shifts[i][c] = channel_shifts[i - 1][c];
                }
            }
        }
        RenderPipeline::new(stages, channel_shifts, padding, frame_dimensions, ctx)
    }
}
