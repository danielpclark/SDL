// Rust translation of src/pcf/pcf.h from FreeType (2.13.2, as SDL_ttf's
// external/freetype pins it).
//
// FreeType font driver for pcf fonts
//
// Copyright (C) 2000, 2001, 2002, 2003, 2006, 2010 by
// Francesco Zappa Nardelli
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT.  IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
// THE SOFTWARE.
//
// This is an altered (translated) version of the original software.

//! The PCF font records and format macros.

use std::sync::Arc;

use super::super::base::ftobjs::FtFaceRec;
use super::super::base::ftstream::FtSharedStream;
use super::super::fttypes::*;

/// `PCF_TableRec`
#[derive(Debug, Clone, Copy, Default)]
pub struct PcfTableRec {
    pub type_: FtULong,
    pub format: FtULong,
    pub size: FtULong,
    pub offset: FtULong,
}

/// `PCF_TocRec`
#[derive(Debug, Clone, Default)]
pub struct PcfTocRec {
    pub version: FtULong,
    pub count: FtULong,
    pub tables: Vec<PcfTableRec>,
}

/// `PCF_ParsePropertyRec`
#[derive(Debug, Clone, Copy, Default)]
pub struct PcfParsePropertyRec {
    pub name: FtLong,
    pub isString: FtByte,
    pub value: FtLong,
}

/// `PCF_PropertyRec` (the `value` union is `atom` for strings and `l`
/// otherwise)
#[derive(Debug, Clone, Default)]
pub struct PcfPropertyRec {
    pub name: Option<Vec<u8>>,
    pub isString: FtByte,

    /// `value.atom`
    pub atom: Option<Vec<u8>>,
    /// `value.l`
    pub l: FtLong,
}

impl PcfPropertyRec {
    /// `value.l` of any property.
    ///
    /// FIXME (upstream): the driver reads the size and resolution
    /// properties as integers without checking `isString`; of a string
    /// property, C reads the atom's pointer: a large positive value, as
    /// the address of any heap block (this value).
    pub fn value_l(&self) -> FtLong {
        if self.isString != 0 {
            0x0000_5555_5555_0000
        } else {
            self.l
        }
    }
}

/// `PCF_Compressed_MetricRec`
#[derive(Debug, Clone, Copy, Default)]
pub struct PcfCompressedMetricRec {
    pub leftSideBearing: FtByte,
    pub rightSideBearing: FtByte,
    pub characterWidth: FtByte,
    pub ascent: FtByte,
    pub descent: FtByte,
}

/// `PCF_MetricRec`
#[derive(Debug, Clone, Copy, Default)]
pub struct PcfMetricRec {
    pub leftSideBearing: FtShort,
    pub rightSideBearing: FtShort,
    pub characterWidth: FtShort,
    pub ascent: FtShort,
    pub descent: FtShort,
    pub attributes: FtShort,

    pub bits: FtULong, /* offset into the PCF_BITMAPS table */
}

/// `PCF_EncRec`
#[derive(Debug, Clone, Default)]
pub struct PcfEncRec {
    pub firstCol: FtUShort,
    pub lastCol: FtUShort,
    pub firstRow: FtUShort,
    pub lastRow: FtUShort,

    pub defaultChar: FtUShort,

    pub offset: Arc<[FtUShort]>,
}

/// `PCF_AccelRec`
#[derive(Debug, Clone, Copy, Default)]
pub struct PcfAccelRec {
    pub noOverlap: FtByte,
    pub constantMetrics: FtByte,
    pub terminalFont: FtByte,
    pub constantWidth: FtByte,
    pub inkInside: FtByte,
    pub inkMetrics: FtByte,
    pub drawDirection: FtByte,
    pub fontAscent: FtLong,
    pub fontDescent: FtLong,
    pub maxOverlap: FtLong,
    pub minbounds: PcfMetricRec,
    pub maxbounds: PcfMetricRec,
    pub ink_minbounds: PcfMetricRec,
    pub ink_maxbounds: PcfMetricRec,
}

/// `PCF_FaceRec`
///
/// This file uses X11 terminology for PCF data; an `encoding' in X11 speak
/// is the same as a `character code' in FreeType speak.
///
/// `comp_stream` is the face's stream while it reads a compressed font
/// (whether `face->stream == &face->comp_stream` is `comp_source` being
/// set); `comp_source` is the original stream, which the compressed
/// stream reads.
#[derive(Debug, Default)]
pub struct PcfFaceRec {
    pub root: FtFaceRec,

    pub comp_source: Option<FtSharedStream>,

    pub charset_encoding: Option<Vec<u8>>,
    pub charset_registry: Option<Vec<u8>>,

    pub toc: PcfTocRec,
    pub accel: PcfAccelRec,

    pub nprops: i32,
    pub properties: Vec<PcfPropertyRec>,

    pub nmetrics: FtULong,
    pub metrics: Vec<PcfMetricRec>,

    pub enc: PcfEncRec,

    pub bitmapsFormat: FtULong,
}

/// `PCF_DriverRec` (`PCF_CONFIG_OPTION_LONG_FAMILY_NAMES` is undefined)
#[derive(Debug, Clone, Copy, Default)]
pub struct PcfDriverRec {
    pub no_long_family_names: bool,
}

/* macros for pcf font format */

pub const LSBFirst: i32 = 0;
pub const MSBFirst: i32 = 1;

pub const PCF_FILE_VERSION: FtULong =
    ((b'p' as FtULong) << 24) | ((b'c' as FtULong) << 16) | ((b'f' as FtULong) << 8) | 1;
pub const PCF_FORMAT_MASK: FtULong = 0xFFFFFF00;

pub const PCF_DEFAULT_FORMAT: FtULong = 0x00000000;
pub const PCF_INKBOUNDS: FtULong = 0x00000200;
pub const PCF_ACCEL_W_INKBOUNDS: FtULong = 0x00000100;
pub const PCF_COMPRESSED_METRICS: FtULong = 0x00000100;

/// `PCF_FORMAT_MATCH`
pub const fn pcf_format_match(a: FtULong, b: FtULong) -> bool {
    (a & PCF_FORMAT_MASK) == (b & PCF_FORMAT_MASK)
}

pub const PCF_GLYPH_PAD_MASK: FtULong = 3 << 0;
pub const PCF_BYTE_MASK: FtULong = 1 << 2;
pub const PCF_BIT_MASK: FtULong = 1 << 3;
pub const PCF_SCAN_UNIT_MASK: FtULong = 3 << 4;

/// `PCF_BYTE_ORDER`
pub const fn pcf_byte_order(f: FtULong) -> i32 {
    if f & PCF_BYTE_MASK != 0 {
        MSBFirst
    } else {
        LSBFirst
    }
}
/// `PCF_BIT_ORDER`
pub const fn pcf_bit_order(f: FtULong) -> i32 {
    if f & PCF_BIT_MASK != 0 {
        MSBFirst
    } else {
        LSBFirst
    }
}
/// `PCF_GLYPH_PAD_INDEX`
pub const fn pcf_glyph_pad_index(f: FtULong) -> FtULong {
    f & PCF_GLYPH_PAD_MASK
}
/// `PCF_GLYPH_PAD`
pub const fn pcf_glyph_pad(f: FtULong) -> FtULong {
    1 << pcf_glyph_pad_index(f)
}
/// `PCF_SCAN_UNIT_INDEX`
pub const fn pcf_scan_unit_index(f: FtULong) -> FtULong {
    (f & PCF_SCAN_UNIT_MASK) >> 4
}
/// `PCF_SCAN_UNIT`
pub const fn pcf_scan_unit(f: FtULong) -> FtULong {
    1 << pcf_scan_unit_index(f)
}
/// `PCF_FORMAT_BITS`
pub const fn pcf_format_bits(f: FtULong) -> FtULong {
    f & (PCF_GLYPH_PAD_MASK | PCF_BYTE_MASK | PCF_BIT_MASK | PCF_SCAN_UNIT_MASK)
}

/// `PCF_SIZE_TO_INDEX`
pub const fn pcf_size_to_index(s: FtULong) -> FtULong {
    if s == 4 {
        2
    } else if s == 2 {
        1
    } else {
        0
    }
}
/// `PCF_INDEX_TO_SIZE`
pub const fn pcf_index_to_size(b: FtULong) -> FtULong {
    1 << b
}

/// `PCF_FORMAT`
pub const fn pcf_format(bit: i32, byte: i32, glyph: FtULong, scan: FtULong) -> FtULong {
    (pcf_size_to_index(scan) << 4)
        | ((if bit == MSBFirst { 1 } else { 0 }) << 3)
        | ((if byte == MSBFirst { 1 } else { 0 }) << 2)
        | pcf_size_to_index(glyph)
}

pub const PCF_PROPERTIES: FtULong = 1 << 0;
pub const PCF_ACCELERATORS: FtULong = 1 << 1;
pub const PCF_METRICS: FtULong = 1 << 2;
pub const PCF_BITMAPS: FtULong = 1 << 3;
pub const PCF_INK_METRICS: FtULong = 1 << 4;
pub const PCF_BDF_ENCODINGS: FtULong = 1 << 5;
pub const PCF_SWIDTHS: FtULong = 1 << 6;
pub const PCF_GLYPH_NAMES: FtULong = 1 << 7;
pub const PCF_BDF_ACCELERATORS: FtULong = 1 << 8;

pub const GLYPHPADOPTIONS: usize = 4; /* I'm not sure about this */
