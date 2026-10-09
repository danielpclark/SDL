// Rust translation of lib/jxl/image_bundle.h and lib/jxl/image_bundle.cc from
// libjxl (https://github.com/libjxl/libjxl, at the revision SDL_image's
// external/libjxl pins: libjxl 0.7.3 with SDL's patches).
// Copyright (c) the JPEG XL Project Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The main image or frame consists of a bundle of associated images.
//! (JPEG reconstruction data is not translated: SDL_image never asks for
//! it, so `IsJPEG()` is always false.)

use std::rc::Rc;

use super::base::{Status, StatusCode};
use super::color_encoding_internal::ColorEncoding;
use super::frame_header::FrameOrigin;
use super::image::{Image3F, ImageF};
use super::image_metadata::{ExtraChannel, ImageMetadata};

/// Translation of `ImageBundle`.
#[derive(Clone, Debug)]
pub(crate) struct ImageBundle {
    pub origin: FrameOrigin,

    pub duration: u32,

    metadata: Option<Rc<ImageMetadata>>,

    color: Image3F,             // If empty, planes_ is not; all planes equal if IsGray().
    c_current: ColorEncoding, // of color_

    extra_channels: Vec<ImageF>,
}

impl ImageBundle {
    /// Translation of `ImageBundle()` / `ImageBundle(metadata)`.
    pub(crate) fn new(metadata: Option<Rc<ImageMetadata>>) -> Self {
        ImageBundle {
            origin: FrameOrigin { x0: 0, y0: 0 },
            duration: 0,
            metadata,
            color: Image3F::empty(),
            c_current: ColorEncoding::new(),
            extra_channels: Vec::new(),
        }
    }

    // -- SIZE

    pub(crate) fn xsize(&self) -> usize {
        if self.color.xsize() != 0 {
            return self.color.xsize();
        }
        if self.extra_channels.is_empty() {
            0
        } else {
            self.extra_channels[0].xsize()
        }
    }
    pub(crate) fn ysize(&self) -> usize {
        if self.color.ysize() != 0 {
            return self.color.ysize();
        }
        if self.extra_channels.is_empty() {
            0
        } else {
            self.extra_channels[0].ysize()
        }
    }

    // -- COLOR

    /// Whether color() is valid/usable. Returns true in most cases. Even
    /// images with spot colors (one example of when !planes().empty()) typically
    /// have a part that can be converted to RGB.
    pub(crate) fn has_color(&self) -> bool {
        self.color.xsize() != 0
    }

    /// For resetting the size when switching from a reference to main frame.
    pub(crate) fn remove_color(&mut self) {
        self.color = Image3F::empty();
    }

    /// Do not use if !HasColor().
    pub(crate) fn color(&self) -> &Image3F {
        &self.color
    }

    pub(crate) fn color_mut(&mut self) -> &mut Image3F {
        &mut self.color
    }

    /// Both the color and the extra channels, for writing them at once.
    pub(crate) fn color_and_extra_channels_mut(&mut self) -> (&mut Image3F, &mut Vec<ImageF>) {
        (&mut self.color, &mut self.extra_channels)
    }

    /// Translation of `SetFromImage()` (called by all other SetFrom*).
    pub(crate) fn set_from_image(&mut self, color: Image3F, c_current: &ColorEncoding) -> Status {
        if color.xsize() == 0 || color.ysize() == 0 {
            // JXL_CHECK(color.xsize() != 0 && color.ysize() != 0)
            return Err(StatusCode::GenericError);
        }
        let md_is_gray = self.metadata.as_ref().is_some_and(|m| m.color_encoding.is_gray());
        if md_is_gray != c_current.is_gray() {
            // JXL_CHECK(metadata_->color_encoding.IsGray() == c_current.IsGray())
            return Err(StatusCode::GenericError);
        }
        self.color = color;
        self.c_current = c_current.clone();
        self.verify_sizes()
    }

    /// Returns the color encoding of the pixels in color(). Translation of
    /// `c_current()`.
    pub(crate) fn c_current(&self) -> &ColorEncoding {
        &self.c_current
    }

    // -- ALPHA

    pub(crate) fn has_alpha(&self) -> bool {
        self.metadata().is_some_and(|m| m.find(ExtraChannel::Alpha).is_some())
    }

    fn alpha_index(&self) -> Option<usize> {
        let m = self.metadata()?;
        m.extra_channel_info.iter().position(|e| e.type_ == ExtraChannel::Alpha)
    }

    /// Translation of `alpha()`.
    pub(crate) fn alpha(&self) -> Option<&ImageF> {
        let ec = self.alpha_index()?;
        // JXL_ASSERT(ec < extra_channels_.size())
        self.extra_channels.get(ec)
    }

    // -- EXTRA CHANNELS

    /// Translation of `ClearExtraChannels()`.
    pub(crate) fn clear_extra_channels(&mut self) {
        self.extra_channels.clear();
    }
    pub(crate) fn extra_channels(&self) -> &Vec<ImageF> {
        &self.extra_channels
    }
    pub(crate) fn extra_channels_mut(&mut self) -> &mut Vec<ImageF> {
        &mut self.extra_channels
    }

    pub(crate) fn metadata(&self) -> Option<&ImageMetadata> {
        self.metadata.as_deref()
    }

    pub(crate) fn metadata_rc(&self) -> Option<Rc<ImageMetadata>> {
        self.metadata.clone()
    }

    /// Translation of `VerifySizes()`.
    fn verify_sizes(&self) -> Status {
        let xs = self.xsize();
        let ys = self.ysize();

        if !self.extra_channels.is_empty() {
            if xs == 0 || ys == 0 {
                return Err(StatusCode::GenericError);
            }
            for ec in &self.extra_channels {
                if ec.xsize() != xs || ec.ysize() != ys {
                    return Err(StatusCode::GenericError);
                }
            }
        }
        Ok(())
    }
}
