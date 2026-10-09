// Rust translation of lib/jxl/frame_header.h and lib/jxl/frame_header.cc
// from libjxl (https://github.com/libjxl/libjxl, at the revision SDL_image's
// external/libjxl pins: libjxl 0.7.3 with SDL's patches).
// Copyright (c) the JPEG XL Project Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Frame headers. (`nonserialized_metadata` is a shared copy of the
//! codestream's metadata instead of a pointer to it.)

use std::rc::Rc;

use super::base::{div_ceil, jxl_failure, pack_signed, unpack_signed, FrameDimensions, Status, K_MAX_NUM_PASSES};
use super::fields::{bits, bits_offset, bundle_init, val, Fields, U32Enc, Visitor};
use super::image_metadata::{visit_name_string, CodecMetadata};
use super::loop_filter::LoopFilter;

/// Translation of `FrameEncoding`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum FrameEncoding {
    VarDct,
    Modular,
}

/// Translation of `ColorTransform`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ColorTransform {
    Xyb,   // Values are encoded with XYB. May only be used if
           // ImageBundle::xyb_encoded.
    None,  // Values are encoded according to the attached color profile. May
           // only be used if !ImageBundle::xyb_encoded.
    YCbCr, // Values are encoded according to the attached color profile, but
           // transformed to YCbCr. May only be used if
           // !ImageBundle::xyb_encoded.
}

/// Translation of `JpegOrder()`.
#[allow(dead_code)]
pub(crate) fn jpeg_order(ct: ColorTransform, is_gray: bool) -> [i32; 3] {
    if is_gray {
        return [0, 0, 0];
    }
    debug_assert!(ct != ColorTransform::Xyb);
    if ct == ColorTransform::YCbCr {
        [1, 0, 2]
    } else {
        [0, 1, 2]
    }
}

/// Translation of `YCbCrChromaSubsampling`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct YCbCrChromaSubsampling {
    channel_mode: [u32; 3],
    maxhs: u8,
    maxvs: u8,
}

impl YCbCrChromaSubsampling {
    const K_H_SHIFT: [u8; 4] = [0, 1, 1, 0];
    const K_V_SHIFT: [u8; 4] = [0, 1, 0, 1];

    pub(crate) fn new() -> Self {
        let mut s = YCbCrChromaSubsampling::default();
        bundle_init(&mut s);
        s
    }

    pub(crate) fn h_shift(&self, c: usize) -> usize {
        (self.maxhs - Self::K_H_SHIFT[self.channel_mode[c] as usize]) as usize
    }
    pub(crate) fn v_shift(&self, c: usize) -> usize {
        (self.maxvs - Self::K_V_SHIFT[self.channel_mode[c] as usize]) as usize
    }

    pub(crate) fn max_h_shift(&self) -> u8 {
        self.maxhs
    }
    pub(crate) fn max_v_shift(&self) -> u8 {
        self.maxvs
    }

    #[allow(dead_code)]
    pub(crate) fn raw_h_shift(&self, c: usize) -> u8 {
        Self::K_H_SHIFT[self.channel_mode[c] as usize]
    }
    #[allow(dead_code)]
    pub(crate) fn raw_v_shift(&self, c: usize) -> u8 {
        Self::K_V_SHIFT[self.channel_mode[c] as usize]
    }

    pub(crate) fn is_444(&self) -> bool {
        for c in [0, 2] {
            if self.channel_mode[c] != self.channel_mode[1] {
                return false;
            }
        }
        true
    }

    pub(crate) fn is_420(&self) -> bool {
        self.channel_mode[0] == 1 && self.channel_mode[1] == 0 && self.channel_mode[2] == 1
    }

    // (These two return false when a channel *is* subsampled that way, as
    // upstream's do.)
    pub(crate) fn is_422(&self) -> bool {
        for c in [0, 2] {
            if Self::K_H_SHIFT[self.channel_mode[c] as usize]
                == Self::K_H_SHIFT[self.channel_mode[1] as usize] + 1
                && Self::K_V_SHIFT[self.channel_mode[c] as usize]
                    == Self::K_V_SHIFT[self.channel_mode[1] as usize]
            {
                return false;
            }
        }
        true
    }

    pub(crate) fn is_440(&self) -> bool {
        for c in [0, 2] {
            if Self::K_H_SHIFT[self.channel_mode[c] as usize]
                == Self::K_H_SHIFT[self.channel_mode[1] as usize]
                && Self::K_V_SHIFT[self.channel_mode[c] as usize]
                    == Self::K_V_SHIFT[self.channel_mode[1] as usize] + 1
            {
                return false;
            }
        }
        true
    }

    fn recompute(&mut self) {
        self.maxhs = 0;
        self.maxvs = 0;
        for i in 0..3 {
            self.maxhs = self.maxhs.max(Self::K_H_SHIFT[self.channel_mode[i] as usize]);
            self.maxvs = self.maxvs.max(Self::K_V_SHIFT[self.channel_mode[i] as usize]);
        }
    }
}

impl Fields for YCbCrChromaSubsampling {
    fn visit_fields(&mut self, visitor: &mut dyn Visitor) -> Status {
        // TODO(veluca): consider allowing 4x downsamples
        for i in 0..3 {
            visitor.bits(2, 0, &mut self.channel_mode[i])?;
        }
        self.recompute();
        Ok(())
    }
}

/// Indicates how to combine the current frame with a previously-saved one.
/// Can be independently controlled for color and extra channels. (See
/// upstream's frame_header.h for the formulas.) Translation of `BlendMode`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum BlendMode {
    // The new values (in the crop) replace the old ones: sample = new
    Replace = 0,
    // The new values (in the crop) get added to the old ones: sample = old + new
    Add = 1,
    // The new values (in the crop) replace the old ones if alpha>0
    Blend = 2,
    // The new values (in the crop) are added to the old ones if alpha>0
    AlphaWeightedAdd = 3,
    // The new values (in the crop) get multiplied by the old ones:
    // sample = old * new
    Mul = 4,
}

impl BlendMode {
    fn from_u32(v: u32) -> BlendMode {
        match v {
            0 => BlendMode::Replace,
            1 => BlendMode::Add,
            2 => BlendMode::Blend,
            3 => BlendMode::AlphaWeightedAdd,
            _ => BlendMode::Mul,
        }
    }
}

/// Translation of `VisitBlendMode()`.
fn visit_blend_mode(visitor: &mut dyn Visitor, default_value: BlendMode, blend_mode: &mut BlendMode) -> Status {
    let mut encoded = *blend_mode as u32;

    visitor.u32d(
        val(BlendMode::Replace as u32),
        val(BlendMode::Add as u32),
        val(BlendMode::Blend as u32),
        bits_offset(2, 3),
        default_value as u32,
        &mut encoded,
    )?;
    if encoded > 4 {
        return jxl_failure!("Invalid blend_mode");
    }
    *blend_mode = BlendMode::from_u32(encoded);
    Ok(())
}

/// Translation of `FrameType`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum FrameType {
    // A "regular" frame: might be a crop, and will be blended on a previous
    // frame, if any, and displayed or blended in future frames.
    RegularFrame = 0,
    // A DC frame: this frame is downsampled and will be *only* used as the DC of
    // a future frame and, possibly, for previews. Cannot be cropped, blended, or
    // referenced by patches or blending modes. Frames that *use* a DC frame
    // cannot have non-default sizes either.
    DcFrame = 1,
    // A PatchesSource frame: this frame will be only used as a source frame for
    // taking patches. Can be cropped, but cannot have non-(0, 0) x0 and y0.
    ReferenceOnly = 2,
    // Same as kRegularFrame, but not used for progressive rendering. This also
    // implies no early display of DC.
    SkipProgressive = 3,
}

/// Translation of `VisitFrameType()`.
fn visit_frame_type(visitor: &mut dyn Visitor, default_value: FrameType, frame_type: &mut FrameType) -> Status {
    let mut encoded = *frame_type as u32;

    visitor.u32d(
        val(FrameType::RegularFrame as u32),
        val(FrameType::DcFrame as u32),
        val(FrameType::ReferenceOnly as u32),
        val(FrameType::SkipProgressive as u32),
        default_value as u32,
        &mut encoded,
    )?;
    *frame_type = match encoded {
        0 => FrameType::RegularFrame,
        1 => FrameType::DcFrame,
        2 => FrameType::ReferenceOnly,
        _ => FrameType::SkipProgressive,
    };
    Ok(())
}

/// Translation of `BlendingInfo`.
#[derive(Clone, Debug)]
pub(crate) struct BlendingInfo {
    pub mode: BlendMode,
    // Which extra channel to use as alpha channel for blending, only encoded
    // for blend modes that involve alpha and if there are more than 1 extra
    // channels.
    pub alpha_channel: u32,
    // Clamp alpha or channel values to 0-1 range.
    pub clamp: bool,
    // Frame ID to copy from (0-3). Only encoded if blend_mode is not kReplace.
    pub source: u32,

    pub nonserialized_num_extra_channels: usize,
    pub nonserialized_is_partial_frame: bool,
}

impl BlendingInfo {
    pub(crate) fn new() -> Self {
        let mut s = BlendingInfo {
            mode: BlendMode::Replace,
            alpha_channel: 0,
            clamp: false,
            source: 0,
            nonserialized_num_extra_channels: 0,
            nonserialized_is_partial_frame: false,
        };
        bundle_init(&mut s);
        s
    }
}

impl Fields for BlendingInfo {
    fn visit_fields(&mut self, visitor: &mut dyn Visitor) -> Status {
        visit_blend_mode(visitor, BlendMode::Replace, &mut self.mode)?;
        if visitor.conditional(
            self.nonserialized_num_extra_channels > 0
                && (self.mode == BlendMode::Blend || self.mode == BlendMode::AlphaWeightedAdd),
        ) {
            // Up to 11 alpha channels for blending.
            visitor.u32d(
                val(0),
                val(1),
                val(2),
                bits_offset(3, 3),
                0,
                &mut self.alpha_channel,
            )?;
            if visitor.is_reading()
                && self.alpha_channel as usize >= self.nonserialized_num_extra_channels
            {
                return jxl_failure!("Invalid alpha channel for blending");
            }
        }
        if visitor.conditional(
            (self.nonserialized_num_extra_channels > 0
                && (self.mode == BlendMode::Blend || self.mode == BlendMode::AlphaWeightedAdd))
                || self.mode == BlendMode::Mul,
        ) {
            visitor.bool_(false, &mut self.clamp)?;
        }
        // 'old' frame for blending. Only necessary if this is not a full frame, or
        // blending is not kReplace.
        if visitor.conditional(self.mode != BlendMode::Replace || self.nonserialized_is_partial_frame) {
            visitor.u32d(val(0), val(1), val(2), val(3), 0, &mut self.source)?;
        }
        Ok(())
    }
}

/// Origin of the current frame. Not present for frames of type
/// kOnlyPatches. Translation of `FrameOrigin`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct FrameOrigin {
    pub x0: i32,
    pub y0: i32, // can be negative.
}

/// Size of the current frame. Translation of `FrameSize`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct FrameSize {
    pub xsize: u32,
    pub ysize: u32,
}

/// AnimationFrame defines duration of animation frames. Translation of
/// `AnimationFrame`.
#[derive(Clone, Debug, Default)]
pub(crate) struct AnimationFrame {
    // How long to wait [in ticks, see Animation{}] after rendering.
    // May be 0 if the current frame serves as a foundation for another frame.
    pub duration: u32,

    pub timecode: u32, // 0xHHMMSSFF

    // Must be set to the one ImageMetadata acting as the full codestream header,
    // with correct xyb_encoded, list of extra channels, etc...
    pub nonserialized_metadata: Option<Rc<CodecMetadata>>,
}

impl AnimationFrame {
    pub(crate) fn new(metadata: Option<Rc<CodecMetadata>>) -> Self {
        let mut s = AnimationFrame {
            duration: 0,
            timecode: 0,
            nonserialized_metadata: metadata,
        };
        bundle_init(&mut s);
        s
    }
}

impl Fields for AnimationFrame {
    fn visit_fields(&mut self, visitor: &mut dyn Visitor) -> Status {
        let have_animation = self
            .nonserialized_metadata
            .as_ref()
            .is_some_and(|m| m.m.have_animation);
        if visitor.conditional(have_animation) {
            visitor.u32d(val(0), val(1), bits(8), bits(32), 0, &mut self.duration)?;
        }

        let have_timecodes = self
            .nonserialized_metadata
            .as_ref()
            .is_some_and(|m| m.m.animation.have_timecodes);
        if visitor.conditional(have_timecodes) {
            visitor.bits(32, 0, &mut self.timecode)?;
        }
        Ok(())
    }
}

/// For decoding to lower resolutions. Only used for kRegular frames.
/// Translation of `Passes`.
#[derive(Clone, Debug, Default)]
pub(crate) struct Passes {
    pub num_passes: u32,     // <= kMaxNumPasses
    pub num_downsample: u32, // <= num_passes

    // Array of num_downsample pairs. downsample=1/last_pass=num_passes-1 and
    // downsample=8/last_pass=0 need not be specified; they are implicit.
    pub downsample: [u32; K_MAX_NUM_PASSES],
    pub last_pass: [u32; K_MAX_NUM_PASSES],
    // Array of shift values for each pass. It is implicitly assumed to be 0 for
    // the last pass.
    pub shift: [u32; K_MAX_NUM_PASSES],
}

impl Passes {
    pub(crate) fn new() -> Self {
        let mut s = Passes::default();
        bundle_init(&mut s);
        s
    }

    /// Translation of `GetDownsamplingBracket()`.
    pub(crate) fn get_downsampling_bracket(&self, pass: usize, min_shift: &mut i32, max_shift: &mut i32) {
        *max_shift = 2;
        *min_shift = 3;
        let mut i = 0usize;
        loop {
            for j in 0..self.num_downsample as usize {
                if i == self.last_pass[j] as usize {
                    if self.downsample[j] == 8 {
                        *min_shift = 3;
                    }
                    if self.downsample[j] == 4 {
                        *min_shift = 2;
                    }
                    if self.downsample[j] == 2 {
                        *min_shift = 1;
                    }
                    if self.downsample[j] == 1 {
                        *min_shift = 0;
                    }
                }
            }
            if i == self.num_passes as usize - 1 {
                *min_shift = 0;
            }
            if i == pass {
                return;
            }
            *max_shift = *min_shift - 1;
            i += 1;
        }
    }

    /// Translation of `GetDownsamplingTargetForCompletedPasses()`.
    #[allow(dead_code)]
    pub(crate) fn get_downsampling_target_for_completed_passes(&self, num_p: u32) -> u32 {
        if num_p >= self.num_passes {
            return 1;
        }
        let mut retval = 8u32;
        for i in 0..self.num_downsample as usize {
            if num_p > self.last_pass[i] {
                retval = retval.min(self.downsample[i]);
            }
        }
        retval
    }
}

impl Fields for Passes {
    fn visit_fields(&mut self, visitor: &mut dyn Visitor) -> Status {
        visitor.u32d(
            val(1),
            val(2),
            val(3),
            bits_offset(3, 4),
            1,
            &mut self.num_passes,
        )?;
        debug_assert!(self.num_passes as usize <= K_MAX_NUM_PASSES); // Cannot happen when reading

        if visitor.conditional(self.num_passes != 1) {
            visitor.u32d(
                val(0),
                val(1),
                val(2),
                bits_offset(1, 3),
                0,
                &mut self.num_downsample,
            )?;
            debug_assert!(self.num_downsample <= 4); // 1,2,4,8
            if self.num_downsample > self.num_passes {
                return jxl_failure!("num_downsample > num_passes");
            }

            for i in 0..self.num_passes.wrapping_sub(1).min(K_MAX_NUM_PASSES as u32) as usize {
                visitor.bits(2, 0, &mut self.shift[i])?;
            }
            if self.num_passes != 0 {
                self.shift[self.num_passes as usize - 1] = 0;
            }

            for i in 0..self.num_downsample as usize {
                visitor.u32d(val(1), val(2), val(4), val(8), 1, &mut self.downsample[i])?;
                if i > 0 && self.downsample[i] >= self.downsample[i - 1] {
                    return jxl_failure!("downsample sequence should be decreasing");
                }
            }
            for i in 0..self.num_downsample as usize {
                visitor.u32d(val(0), val(1), val(2), bits(3), 0, &mut self.last_pass[i])?;
                if i > 0 && self.last_pass[i] <= self.last_pass[i - 1] {
                    return jxl_failure!("last_pass sequence should be increasing");
                }
                if self.last_pass[i] >= self.num_passes {
                    return jxl_failure!("last_pass >= num_passes");
                }
            }
        }

        Ok(())
    }
}

/// Image/frame := one of more of these, where the last has is_last = true.
/// Starts at a byte-aligned address "a"; the next pass starts at "a + size".
/// Translation of `FrameHeader`.
#[derive(Clone, Debug)]
pub(crate) struct FrameHeader {
    pub all_default: bool,

    // Always present
    pub encoding: FrameEncoding,
    // Some versions of UBSAN complain in VisitFrameType if not initialized.
    pub frame_type: FrameType,

    pub flags: u64,

    pub color_transform: ColorTransform,
    pub chroma_subsampling: YCbCrChromaSubsampling,

    pub group_size_shift: u32, // only if encoding == kModular;

    pub x_qm_scale: u32, // only if VarDCT and color_transform == kXYB
    pub b_qm_scale: u32, // only if VarDCT and color_transform == kXYB

    pub name: Vec<u8>,

    // Skipped for kReferenceOnly.
    pub passes: Passes,

    // Skipped for kDCFrame
    pub custom_size_or_origin: bool,
    pub frame_size: FrameSize,

    // upsampling factors for color and extra channels.
    // Upsampling is always performed before applying any inverse color transform.
    // Skipped (1) if kUseDCFrame
    pub upsampling: u32,
    pub extra_channel_upsampling: Vec<u32>,

    // Only for kRegular frames.
    pub frame_origin: FrameOrigin,

    pub blending_info: BlendingInfo,
    pub extra_channel_blending_info: Vec<BlendingInfo>,

    // Animation info for this frame.
    pub animation_frame: AnimationFrame,

    // This is the last frame.
    pub is_last: bool,

    // ID to refer to this frame with. 0-3, not present if kDCFrame.
    // 0 has a special meaning for kRegular frames of nonzero duration: it defines
    // a frame that will not be referenced in the future.
    pub save_as_reference: u32,

    // Whether to save this frame before or after the color transform. A frame
    // that is saved before the color tansform can only be used for blending
    // through patches. On the contrary, a frame that is saved after the color
    // transform can only be used for blending through blending modes.
    // Irrelevant for extra channel blending. Can only be true if
    // blending_info.mode == kReplace and this is not a partial kRegularFrame; if
    // this is a DC frame, it is always true.
    pub save_before_color_transform: bool,

    pub dc_level: u32, // 1-4 if kDCFrame (0 otherwise).

    // Must be set to the one ImageMetadata acting as the full codestream header,
    // with correct xyb_encoded, list of extra channels, etc...
    pub nonserialized_metadata: Option<Rc<CodecMetadata>>,

    // NOTE: This is ignored by AllDefault.
    pub loop_filter: LoopFilter,

    pub nonserialized_is_preview: bool,

    pub extensions: u64,
}

// Optional postprocessing steps. These flags are the source of truth;
// Override must set/clear them rather than change their meaning. Values
// chosen such that typical flags == 0 (encoded in only two bits).
// Often but not always off => low bit value:

// Inject noise into decoded output.
pub(crate) const K_NOISE: u64 = 1;

// Overlay patches.
pub(crate) const K_PATCHES: u64 = 2;

// 4, 8 = reserved for future sometimes-off

// Overlay splines.
pub(crate) const K_SPLINES: u64 = 16;

pub(crate) const K_USE_DC_FRAME: u64 = 32; // Implies kSkipAdaptiveDCSmoothing.

// 64 = reserved for future often-off

// Almost always on => negated:

pub(crate) const K_SKIP_ADAPTIVE_DC_SMOOTHING: u64 = 128;

impl FrameHeader {
    pub(crate) fn new(metadata: Option<Rc<CodecMetadata>>) -> Self {
        let mut s = FrameHeader {
            all_default: false,
            encoding: FrameEncoding::VarDct,
            frame_type: FrameType::RegularFrame,
            flags: 0,
            color_transform: ColorTransform::Xyb,
            chroma_subsampling: YCbCrChromaSubsampling::new(),
            group_size_shift: 0,
            x_qm_scale: 0,
            b_qm_scale: 0,
            name: Vec::new(),
            passes: Passes::new(),
            custom_size_or_origin: false,
            frame_size: FrameSize::default(),
            upsampling: 0,
            extra_channel_upsampling: Vec::new(),
            frame_origin: FrameOrigin::default(),
            blending_info: BlendingInfo::new(),
            extra_channel_blending_info: Vec::new(),
            animation_frame: AnimationFrame::new(metadata.clone()),
            is_last: false,
            save_as_reference: 0,
            save_before_color_transform: false,
            dc_level: 0,
            nonserialized_metadata: metadata,
            loop_filter: LoopFilter::new(),
            nonserialized_is_preview: false,
            extensions: 0,
        };
        bundle_init(&mut s);
        s
    }

    // Returns true if this frame is supposed to be saved for future usage by
    // other frames.
    pub(crate) fn can_be_referenced(&self) -> bool {
        // DC frames cannot be referenced. The last frame cannot be referenced. A
        // duration 0 frame makes little sense if it is not referenced. A
        // non-duration 0 frame may or may not be referenced.
        !self.is_last
            && self.frame_type != FrameType::DcFrame
            && (self.animation_frame.duration == 0 || self.save_as_reference != 0)
    }

    pub(crate) fn default_xsize(&self) -> usize {
        let Some(m) = &self.nonserialized_metadata else {
            return 0;
        };
        if self.nonserialized_is_preview {
            return m.m.preview_size.xsize();
        }
        m.xsize()
    }

    pub(crate) fn default_ysize(&self) -> usize {
        let Some(m) = &self.nonserialized_metadata else {
            return 0;
        };
        if self.nonserialized_is_preview {
            return m.m.preview_size.ysize();
        }
        m.ysize()
    }

    /// Translation of `ToFrameDimensions()`.
    pub(crate) fn to_frame_dimensions(&self) -> FrameDimensions {
        let mut xsize = self.default_xsize();
        let mut ysize = self.default_ysize();

        xsize = if self.frame_size.xsize != 0 {
            self.frame_size.xsize as usize
        } else {
            xsize
        };
        ysize = if self.frame_size.ysize != 0 {
            self.frame_size.ysize as usize
        } else {
            ysize
        };

        if self.dc_level != 0 {
            xsize = div_ceil(xsize, 1 << (3 * self.dc_level));
            ysize = div_ceil(ysize, 1 << (3 * self.dc_level));
        }

        let mut frame_dim = FrameDimensions::default();
        frame_dim.set(
            xsize,
            ysize,
            self.group_size_shift as usize,
            self.chroma_subsampling.max_h_shift() as usize,
            self.chroma_subsampling.max_v_shift() as usize,
            self.encoding == FrameEncoding::Modular,
            self.upsampling as usize,
        );
        frame_dim
    }

    // True if a color transform should be applied to this frame.
    pub(crate) fn needs_color_transform(&self) -> bool {
        !self.save_before_color_transform
            || self.frame_type == FrameType::RegularFrame
            || self.frame_type == FrameType::SkipProgressive
    }
}

impl Fields for FrameHeader {
    fn visit_fields(&mut self, visitor: &mut dyn Visitor) -> Status {
        if visitor.all_default(&mut self.all_default) {
            // Overwrite all serialized fields, but not any nonserialized_*.
            visitor.set_default(self);
            return Ok(());
        }

        visit_frame_type(visitor, FrameType::RegularFrame, &mut self.frame_type)?;
        if visitor.is_reading()
            && self.nonserialized_is_preview
            && self.frame_type != FrameType::RegularFrame
        {
            return jxl_failure!("Only regular frame could be a preview");
        }

        // FrameEncoding.
        let mut is_modular = self.encoding == FrameEncoding::Modular;
        visitor.bool_(false, &mut is_modular)?;
        self.encoding = if is_modular {
            FrameEncoding::Modular
        } else {
            FrameEncoding::VarDct
        };

        // Flags
        visitor.u64_(0, &mut self.flags)?;

        // Color transform
        let xyb_encoded = self
            .nonserialized_metadata
            .as_ref()
            .is_none_or(|m| m.m.xyb_encoded);

        if xyb_encoded {
            self.color_transform = ColorTransform::Xyb;
        } else {
            // Alternate if kYCbCr.
            let mut alternate = self.color_transform == ColorTransform::YCbCr;
            visitor.bool_(false, &mut alternate)?;
            self.color_transform = if alternate {
                ColorTransform::YCbCr
            } else {
                ColorTransform::None
            };
        }

        // Chroma subsampling for YCbCr, if no DC frame is used.
        if visitor.conditional(
            self.color_transform == ColorTransform::YCbCr && ((self.flags & K_USE_DC_FRAME) == 0),
        ) {
            visitor.visit_nested(&mut self.chroma_subsampling)?;
        }

        let num_extra_channels = self
            .nonserialized_metadata
            .as_ref()
            .map_or(0, |m| m.m.extra_channel_info.len());

        // Upsampling
        if visitor.conditional((self.flags & K_USE_DC_FRAME) == 0) {
            visitor.u32d(val(1), val(2), val(4), val(8), 1, &mut self.upsampling)?;
            let metadata = self.nonserialized_metadata.clone();
            if metadata.is_some() && visitor.conditional(num_extra_channels != 0) {
                let metadata = metadata.unwrap();
                let extra_channels = &metadata.m.extra_channel_info;
                self.extra_channel_upsampling.resize(extra_channels.len(), 1);
                for i in 0..extra_channels.len() {
                    let dim_shift = metadata.m.extra_channel_info[i].dim_shift;
                    let ec_upsampling = &mut self.extra_channel_upsampling[i];
                    *ec_upsampling = ec_upsampling.wrapping_shr(dim_shift);
                    visitor.u32d(val(1), val(2), val(4), val(8), 1, ec_upsampling)?;
                    *ec_upsampling = ec_upsampling.wrapping_shl(dim_shift);
                    if *ec_upsampling < self.upsampling {
                        return jxl_failure!("EC upsampling < color upsampling, which is invalid.");
                    }
                    if *ec_upsampling > 8 {
                        return jxl_failure!("EC upsampling too large");
                    }
                }
            } else {
                self.extra_channel_upsampling.clear();
            }
        }

        // Modular- or VarDCT-specific data.
        if visitor.conditional(self.encoding == FrameEncoding::Modular) {
            visitor.bits(2, 1, &mut self.group_size_shift)?;
        }
        if visitor.conditional(
            self.encoding == FrameEncoding::VarDct && self.color_transform == ColorTransform::Xyb,
        ) {
            visitor.bits(3, 3, &mut self.x_qm_scale)?;
            visitor.bits(3, 2, &mut self.b_qm_scale)?;
        } else {
            self.x_qm_scale = 2; // noop
            self.b_qm_scale = 2; // noop
        }

        // Not useful for kPatchSource
        if visitor.conditional(self.frame_type != FrameType::ReferenceOnly) {
            visitor.visit_nested(&mut self.passes)?;
        }

        if visitor.conditional(self.frame_type == FrameType::DcFrame) {
            // Up to 4 pyramid levels - for up to 16384x downsampling.
            visitor.u32d(val(1), val(2), val(3), val(4), 1, &mut self.dc_level)?;
        }
        if self.frame_type != FrameType::DcFrame {
            self.dc_level = 0;
        }

        let mut is_partial_frame = false;
        if visitor.conditional(self.frame_type != FrameType::DcFrame) {
            visitor.bool_(false, &mut self.custom_size_or_origin)?;
            if visitor.conditional(self.custom_size_or_origin) {
                let enc = U32Enc::new(
                    bits(8),
                    bits_offset(11, 256),
                    bits_offset(14, 2304),
                    bits_offset(30, 18688),
                );
                // Frame offset, only if kRegularFrame or kSkipProgressive.
                if visitor.conditional(
                    self.frame_type == FrameType::RegularFrame
                        || self.frame_type == FrameType::SkipProgressive,
                ) {
                    let mut ux0 = pack_signed(self.frame_origin.x0);
                    let mut uy0 = pack_signed(self.frame_origin.y0);
                    visitor.u32_(enc, 0, &mut ux0)?;
                    visitor.u32_(enc, 0, &mut uy0)?;
                    self.frame_origin.x0 = unpack_signed(ux0 as usize) as i32;
                    self.frame_origin.y0 = unpack_signed(uy0 as usize) as i32;
                }
                // Frame size
                visitor.u32_(enc, 0, &mut self.frame_size.xsize)?;
                visitor.u32_(enc, 0, &mut self.frame_size.ysize)?;
                if self.custom_size_or_origin
                    && (self.frame_size.xsize == 0 || self.frame_size.ysize == 0)
                {
                    return jxl_failure!("Invalid crop dimensions for frame: zero width or height");
                }
                let image_xsize = self.default_xsize() as i32;
                let image_ysize = self.default_ysize() as i32;
                if self.frame_type == FrameType::RegularFrame
                    || self.frame_type == FrameType::SkipProgressive
                {
                    is_partial_frame |= self.frame_origin.x0 > 0;
                    is_partial_frame |= self.frame_origin.y0 > 0;
                    is_partial_frame |= (self.frame_size.xsize as i32)
                        .wrapping_add(self.frame_origin.x0)
                        < image_xsize;
                    is_partial_frame |= (self.frame_size.ysize as i32)
                        .wrapping_add(self.frame_origin.y0)
                        < image_ysize;
                }
            }
        }

        // Blending info, animation info and whether this is the last frame or not.
        if visitor.conditional(
            self.frame_type == FrameType::RegularFrame
                || self.frame_type == FrameType::SkipProgressive,
        ) {
            self.blending_info.nonserialized_num_extra_channels = num_extra_channels;
            self.blending_info.nonserialized_is_partial_frame = is_partial_frame;
            visitor.visit_nested(&mut self.blending_info)?;
            let mut replace_all = self.blending_info.mode == BlendMode::Replace;
            self.extra_channel_blending_info
                .resize_with(num_extra_channels, BlendingInfo::new);
            for i in 0..num_extra_channels {
                let ec_blending_info = &mut self.extra_channel_blending_info[i];
                ec_blending_info.nonserialized_is_partial_frame = is_partial_frame;
                ec_blending_info.nonserialized_num_extra_channels = num_extra_channels;
                visitor.visit_nested(ec_blending_info)?;
                replace_all &= ec_blending_info.mode == BlendMode::Replace;
            }
            if visitor.is_reading()
                && self.nonserialized_is_preview
                && (!replace_all || self.custom_size_or_origin)
            {
                return jxl_failure!("Preview is not compatible with blending");
            }
            let have_animation = self
                .nonserialized_metadata
                .as_ref()
                .is_some_and(|m| m.m.have_animation);
            if visitor.conditional(have_animation) {
                self.animation_frame.nonserialized_metadata = self.nonserialized_metadata.clone();
                visitor.visit_nested(&mut self.animation_frame)?;
            }
            visitor.bool_(true, &mut self.is_last)?;
        }
        if self.frame_type != FrameType::RegularFrame {
            self.is_last = false;
        }

        // ID of that can be used to refer to this frame. 0 for a non-zero-duration
        // frame means that it will not be referenced. Not necessary for the last
        // frame.
        if visitor.conditional(self.frame_type != FrameType::DcFrame && !self.is_last) {
            visitor.u32d(val(0), val(1), val(2), val(3), 0, &mut self.save_as_reference)?;
        }

        // If this frame is not blended on another frame post-color-transform, it may
        // be stored for being referenced either before or after the color transform.
        // If it is blended post-color-transform, it must be blended after. It must
        // also be blended after if this is a kRegular frame that does not cover the
        // full frame, as samples outside the partial region are from a
        // post-color-transform frame.
        if self.frame_type != FrameType::DcFrame {
            if visitor.conditional(
                self.can_be_referenced()
                    && self.blending_info.mode == BlendMode::Replace
                    && !is_partial_frame
                    && (self.frame_type == FrameType::RegularFrame
                        || self.frame_type == FrameType::SkipProgressive),
            ) {
                visitor.bool_(false, &mut self.save_before_color_transform)?;
            } else if visitor.conditional(self.frame_type == FrameType::ReferenceOnly) {
                visitor.bool_(true, &mut self.save_before_color_transform)?;
                let (mx, my) = self
                    .nonserialized_metadata
                    .as_ref()
                    .map_or((0, 0), |m| (m.xsize(), m.ysize()));
                if !self.save_before_color_transform
                    && ((self.frame_size.xsize as usize) < mx
                        || (self.frame_size.ysize as usize) < my
                        || self.frame_origin.x0 != 0
                        || self.frame_origin.y0 != 0)
                {
                    return jxl_failure!("non-patch reference frame with invalid crop");
                }
            }
        } else {
            self.save_before_color_transform = true;
        }

        visit_name_string(visitor, &mut self.name)?;

        self.loop_filter.nonserialized_is_modular = is_modular;
        visitor.visit_nested(&mut self.loop_filter)?;

        visitor.begin_extensions(&mut self.extensions)?;
        // Extensions: in chronological order of being added to the format.
        visitor.end_extensions()
    }
}

// Shared by enc/dec. 5F and 13 are by far the most common for d1/2/4/8, 0
// ensures low overhead for small images.
pub(crate) const K_ORDER_ENC: U32Enc = U32Enc::new(
    val(0x5F),
    val(0x13),
    val(0),
    bits(super::ac_strategy::K_NUM_ORDERS as u32),
);
