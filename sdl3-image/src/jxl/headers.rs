// Rust translation of lib/jxl/headers.h and lib/jxl/headers.cc from libjxl
// (https://github.com/libjxl/libjxl, at the revision SDL_image's
// external/libjxl pins: libjxl 0.7.3 with SDL's patches).
// Copyright (c) the JPEG XL Project Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Codestream headers, also stored in CodecInOut.

use super::base::Status;
use super::fields::{bits, bits_offset, bundle_init, val, Fields, Visitor};

// Reserved by ISO/IEC 10918-1. LF causes files opened in text mode to be
// rejected because the marker changes to 0x0D instead. The 0xFF prefix also
// ensures there were no 7-bit transmission limitations.
pub(crate) const K_CODESTREAM_MARKER: u8 = 0x0A;

/// Translation of the anonymous `Rational`.
struct Rational {
    num: u32,
    den: u32,
}

impl Rational {
    const fn new(num: u32, den: u32) -> Self {
        Rational { num, den }
    }

    // Returns floor(multiplicand * rational).
    const fn mul_truncate(&self, multiplicand: u32) -> u32 {
        (multiplicand as u64 * self.num as u64 / self.den as u64) as u32
    }
}

/// Translation of `FixedAspectRatios()`.
fn fixed_aspect_ratios(ratio: u32) -> Rational {
    debug_assert!(0 != ratio && ratio < 8);
    // Other candidates: 5/4, 7/5, 14/9, 16/10, 5/3, 21/9, 12/5
    const K_RATIOS: [Rational; 7] = [
        Rational::new(1, 1),   // square
        Rational::new(12, 10), //
        Rational::new(4, 3),   // camera
        Rational::new(3, 2),   // mobile camera
        Rational::new(16, 9),  // camera/display
        Rational::new(5, 4),   //
        Rational::new(2, 1),   //
    ];
    let r = &K_RATIOS[(ratio - 1) as usize];
    Rational::new(r.num, r.den)
}

/// Compact representation of image dimensions (best case: 9 bits) so
/// decoders can preallocate early. Translation of `SizeHeader`.
#[derive(Clone, Default, Debug)]
pub(crate) struct SizeHeader {
    small: bool, // xsize and ysize <= 256 and divisible by 8.

    ysize_div8_minus_1: u32,
    ysize: u32,

    ratio: u32,
    xsize_div8_minus_1: u32,
    xsize: u32,
}

impl SizeHeader {
    pub(crate) fn new() -> Self {
        let mut s = SizeHeader::default();
        bundle_init(&mut s);
        s
    }

    pub(crate) fn xsize(&self) -> usize {
        if self.ratio != 0 {
            return fixed_aspect_ratios(self.ratio).mul_truncate(self.ysize() as u32) as usize;
        }
        if self.small {
            (self.xsize_div8_minus_1 as usize + 1) * 8
        } else {
            self.xsize as usize
        }
    }
    pub(crate) fn ysize(&self) -> usize {
        if self.small {
            (self.ysize_div8_minus_1 as usize + 1) * 8
        } else {
            self.ysize as usize
        }
    }
}

impl Fields for SizeHeader {
    fn visit_fields(&mut self, visitor: &mut dyn Visitor) -> Status {
        visitor.bool_(false, &mut self.small)?;

        if visitor.conditional(self.small) {
            visitor.bits(5, 0, &mut self.ysize_div8_minus_1)?;
        }
        if visitor.conditional(!self.small) {
            // (Could still be small, but non-multiple of 8.)
            visitor.u32d(
                bits_offset(9, 1),
                bits_offset(13, 1),
                bits_offset(18, 1),
                bits_offset(30, 1),
                1,
                &mut self.ysize,
            )?;
        }

        visitor.bits(3, 0, &mut self.ratio)?;
        if visitor.conditional(self.ratio == 0 && self.small) {
            visitor.bits(5, 0, &mut self.xsize_div8_minus_1)?;
        }
        if visitor.conditional(self.ratio == 0 && !self.small) {
            visitor.u32d(
                bits_offset(9, 1),
                bits_offset(13, 1),
                bits_offset(18, 1),
                bits_offset(30, 1),
                1,
                &mut self.xsize,
            )?;
        }

        Ok(())
    }
}

/// (Similar to SizeHeader but different encoding because previews are
/// smaller) Translation of `PreviewHeader`.
#[derive(Clone, Default, Debug)]
pub(crate) struct PreviewHeader {
    div8: bool, // xsize and ysize divisible by 8.

    ysize_div8: u32,
    ysize: u32,

    ratio: u32,
    xsize_div8: u32,
    xsize: u32,
}

impl PreviewHeader {
    pub(crate) fn new() -> Self {
        let mut s = PreviewHeader::default();
        bundle_init(&mut s);
        s
    }

    pub(crate) fn xsize(&self) -> usize {
        if self.ratio != 0 {
            return fixed_aspect_ratios(self.ratio).mul_truncate(self.ysize() as u32) as usize;
        }
        if self.div8 {
            self.xsize_div8 as usize * 8
        } else {
            self.xsize as usize
        }
    }
    pub(crate) fn ysize(&self) -> usize {
        if self.div8 {
            self.ysize_div8 as usize * 8
        } else {
            self.ysize as usize
        }
    }
}

impl Fields for PreviewHeader {
    fn visit_fields(&mut self, visitor: &mut dyn Visitor) -> Status {
        visitor.bool_(false, &mut self.div8)?;

        if visitor.conditional(self.div8) {
            visitor.u32d(
                val(16),
                val(32),
                bits_offset(5, 1),
                bits_offset(9, 33),
                1,
                &mut self.ysize_div8,
            )?;
        }
        if visitor.conditional(!self.div8) {
            visitor.u32d(
                bits_offset(6, 1),
                bits_offset(8, 65),
                bits_offset(10, 321),
                bits_offset(12, 1345),
                1,
                &mut self.ysize,
            )?;
        }

        visitor.bits(3, 0, &mut self.ratio)?;
        if visitor.conditional(self.ratio == 0 && self.div8) {
            visitor.u32d(
                val(16),
                val(32),
                bits_offset(5, 1),
                bits_offset(9, 33),
                1,
                &mut self.xsize_div8,
            )?;
        }
        if visitor.conditional(self.ratio == 0 && !self.div8) {
            visitor.u32d(
                bits_offset(6, 1),
                bits_offset(8, 65),
                bits_offset(10, 321),
                bits_offset(12, 1345),
                1,
                &mut self.xsize,
            )?;
        }

        Ok(())
    }
}

/// Translation of `AnimationHeader`.
#[derive(Clone, Default, Debug)]
pub(crate) struct AnimationHeader {
    // Ticks per second (expressed as rational number to support NTSC)
    pub tps_numerator: u32,
    pub tps_denominator: u32,

    pub num_loops: u32, // 0 means to repeat infinitely.

    pub have_timecodes: bool,
}

impl AnimationHeader {
    pub(crate) fn new() -> Self {
        let mut s = AnimationHeader::default();
        bundle_init(&mut s);
        s
    }
}

impl Fields for AnimationHeader {
    fn visit_fields(&mut self, visitor: &mut dyn Visitor) -> Status {
        visitor.u32d(
            val(100),
            val(1000),
            bits_offset(10, 1),
            bits_offset(30, 1),
            1,
            &mut self.tps_numerator,
        )?;
        visitor.u32d(
            val(1),
            val(1001),
            bits_offset(8, 1),
            bits_offset(10, 1),
            1,
            &mut self.tps_denominator,
        )?;

        visitor.u32d(val(0), bits(3), bits(16), bits(32), 0, &mut self.num_loops)?;

        visitor.bool_(false, &mut self.have_timecodes)?;
        Ok(())
    }
}
