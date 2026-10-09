// Rust translation of lib/jxl/modular/modular_image.h and
// lib/jxl/modular/modular_image.cc from libjxl
// (https://github.com/libjxl/libjxl, at the revision SDL_image's
// external/libjxl pins: libjxl 0.7.3 with SDL's patches).
// Copyright (c) the JPEG XL Project Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The modular image: its channels and the transforms to undo.

use super::super::base::StatusCode;
use super::super::image::{copy_image_to, Plane};
use super::encoding::context_predict::weighted;
use super::transform::transform::Transform;

pub(crate) type PixelType = i32; // can use int16_t if it's only for 8-bit images.
                                 // Need some wiggle room for YCoCg / Squeeze etc

pub(crate) type PixelTypeW = i64;

/// Translation of `Channel`.
#[derive(Debug, Default)]
pub(crate) struct Channel {
    pub plane: Plane<PixelType>,
    pub w: usize,
    pub h: usize,
    pub hshift: i32,
    pub vshift: i32, // w ~= image.w >> hshift;  h ~= image.h >> vshift
}

impl Channel {
    pub(crate) fn new(iw: usize, ih: usize, hsh: i32, vsh: i32) -> Result<Channel, StatusCode> {
        Ok(Channel {
            plane: Plane::new(iw, ih)?,
            w: iw,
            h: ih,
            hshift: hsh,
            vshift: vsh,
        })
    }

    /// Translation of `shrink()`.
    pub(crate) fn shrink(&mut self) -> Result<(), StatusCode> {
        if self.plane.xsize() == self.w && self.plane.ysize() == self.h {
            return Ok(());
        }
        let resizedplane = Plane::new(self.w, self.h)?;
        self.plane = resizedplane;
        Ok(())
    }

    /// Translation of `shrink(nw, nh)`.
    #[allow(dead_code)]
    pub(crate) fn shrink_to(&mut self, nw: usize, nh: usize) -> Result<(), StatusCode> {
        self.w = nw;
        self.h = nh;
        self.shrink()
    }

    #[inline]
    pub(crate) fn row(&self, y: usize) -> &[PixelType] {
        self.plane.row(y)
    }

    #[inline]
    pub(crate) fn row_mut(&mut self, y: usize) -> &mut [PixelType] {
        self.plane.row_mut(y)
    }
}

/// Translation of `Image` (the modular one).
#[derive(Debug)]
pub(crate) struct Image {
    // image data, transforms can dramatically change the number of channels and
    // their semantics
    pub channel: Vec<Channel>,
    // transforms that have been applied (and that have to be undone)
    pub transform: Vec<Transform>,

    // image dimensions (channels may have different dimensions due to transforms)
    pub w: usize,
    pub h: usize,
    pub bitdepth: i32,
    pub nb_meta_channels: usize, // first few channels might contain palette(s)
    pub error: bool,             // true if a fatal error occurred, false otherwise
}

impl Default for Image {
    /// Translation of `Image()`.
    fn default() -> Self {
        Image {
            channel: Vec::new(),
            transform: Vec::new(),
            w: 0,
            h: 0,
            bitdepth: 8,
            nb_meta_channels: 0,
            error: true,
        }
    }
}

impl Image {
    /// Translation of `Image(iw, ih, bitdepth, nb_chans)`.
    pub(crate) fn new(
        iw: usize,
        ih: usize,
        bitdepth: i32,
        nb_chans: i32,
    ) -> Result<Image, StatusCode> {
        let mut img = Image {
            channel: Vec::new(),
            transform: Vec::new(),
            w: iw,
            h: ih,
            bitdepth,
            nb_meta_channels: 0,
            error: false,
        };
        for _ in 0..nb_chans {
            img.channel.push(Channel::new(iw, ih, 0, 0)?);
        }
        Ok(img)
    }

    pub(crate) fn empty(&self) -> bool {
        for ch in &self.channel {
            if ch.w != 0 && ch.h != 0 {
                return false;
            }
        }
        true
    }

    /// Translation of `clone()`.
    pub(crate) fn clone_image(&self) -> Result<Image, StatusCode> {
        let mut c = Image::new(self.w, self.h, self.bitdepth, 0)?;
        c.nb_meta_channels = self.nb_meta_channels;
        c.error = self.error;
        c.transform = self.transform.clone();
        for ch in &self.channel {
            let mut a = Channel::new(ch.w, ch.h, ch.hshift, ch.vshift)?;
            copy_image_to(&ch.plane, &mut a.plane);
            c.channel.push(a);
        }
        Ok(c)
    }

    /// Translation of `undo_transforms()`.
    pub(crate) fn undo_transforms(&mut self, wp_header: &weighted::Header) {
        while let Some(t) = self.transform.last().cloned() {
            // JXL_DEBUG_V(4, "Undoing transform");
            let result = t.inverse(self, wp_header);
            if result.is_err() {
                // JXL_NOTIFY_ERROR("Error while undoing transform.");
                self.error = true;
                return;
            }
            self.transform.pop();
        }
    }
}
