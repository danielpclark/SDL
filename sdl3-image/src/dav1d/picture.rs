// Rust translation of include/dav1d/picture.h, src/picture.c and
// src/picture.h from dav1d (https://code.videolan.org/videolan/dav1d, at
// the revision SDL_image's external/dav1d pins).
// Copyright © 2018-2021, VideoLAN and dav1d authors
// Copyright © 2018, Two Orioles, LLC
// SPDX-License-Identifier: BSD-2-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Decoded pictures.
//!
//! A picture's pixels are planes of `u8` (8 bits/component) or `u16` (10
//! and 12) owned by a [`PictureData`], shared by `Arc` between the output
//! and the reference slots, as upstream shares the `Dav1dRef`'d buffer.
//! The custom allocator callbacks of `Dav1dPicAllocator` are not
//! translated: pictures are allocated the way
//! `dav1d_default_picture_alloc()` lays them out (128-aligned dimensions,
//! the same strides), without its buffer pool.

use std::sync::Arc;

use super::data::Dav1dDataProps;
use super::headers::{
    Dav1dContentLightLevel, Dav1dFrameHeader, Dav1dITUTT35, Dav1dMasteringDisplay,
    Dav1dSequenceHeader, DAV1D_PIXEL_LAYOUT_I400, DAV1D_PIXEL_LAYOUT_I420, DAV1D_PIXEL_LAYOUT_I444,
};
use super::mem::try_vec;

/// Number of bytes to align AND pad picture memory buffers by, so that SIMD
/// implementations can over-read by a few bytes, and use aligned read/write
/// instructions.
pub(crate) const DAV1D_PICTURE_ALIGNMENT: usize = 64;

/// Translation of `Dav1dPictureParameters`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Dav1dPictureParameters {
    /// width (in pixels)
    pub(crate) w: i32,
    /// height (in pixels)
    pub(crate) h: i32,
    /// format of the picture
    pub(crate) layout: i32,
    /// bits per pixel component (8 or 10)
    pub(crate) bpc: i32,
}

/// The planes of a picture: Y is [0], U is [1], V is [2] (empty for
/// monochrome pictures). 10 and 12 bpc pixels are located in the LSB
/// bits, so that values range between [0, 1023] or [0, 4095].
#[derive(Clone, Debug)]
pub(crate) enum PicPlanes {
    U8([Vec<u8>; 3]),
    U16([Vec<u16>; 3]),
}

/// The pixel buffer of a picture (upstream's allocation behind
/// `Dav1dPicture.data[]`): the planes and their strides in pixels.
#[derive(Clone, Debug)]
pub(crate) struct PictureData {
    pub(crate) planes: PicPlanes,
    /// Pixels between 2 lines for luma [0] or chroma [1].
    pub(crate) stride: [usize; 2],
}

impl PictureData {
    /// A plane of an 8 bpc picture.
    pub(crate) fn plane_u8(&self, i: usize) -> Option<&[u8]> {
        match &self.planes {
            PicPlanes::U8(p) => Some(&p[i]),
            PicPlanes::U16(_) => None,
        }
    }

    /// A plane of a 10 or 12 bpc picture.
    pub(crate) fn plane_u16(&self, i: usize) -> Option<&[u16]> {
        match &self.planes {
            PicPlanes::U16(p) => Some(&p[i]),
            PicPlanes::U8(_) => None,
        }
    }
}

/// Translation of `dav1d_default_picture_alloc()`: the planes of a
/// picture of `p`'s size and format, zeroed. Returns the strides in bytes
/// (`p->stride[]`) with the data.
pub(crate) fn default_picture_alloc(
    p: &Dav1dPictureParameters,
) -> Result<(PictureData, [isize; 2]), ()> {
    let hbd = (p.bpc > 8) as usize;
    let aligned_w = ((p.w + 127) & !127) as usize;
    let aligned_h = ((p.h + 127) & !127) as usize;
    let has_chroma = p.layout != DAV1D_PIXEL_LAYOUT_I400;
    let ss_ver = (p.layout == DAV1D_PIXEL_LAYOUT_I420) as usize;
    let ss_hor = (p.layout != DAV1D_PIXEL_LAYOUT_I444) as usize;
    let mut y_stride = aligned_w << hbd;
    let mut uv_stride = if has_chroma { y_stride >> ss_hor } else { 0 };
    /* Due to how mapping of addresses to sets works in most L1 and L2 cache
     * implementations, strides of multiples of certain power-of-two numbers
     * may cause multiple rows of the same superblock to map to the same set,
     * causing evictions of previous rows resulting in a reduction in cache
     * hit rate. Avoid that by slightly padding the stride when necessary. */
    if y_stride & 1023 == 0 {
        y_stride += DAV1D_PICTURE_ALIGNMENT;
    }
    if uv_stride & 1023 == 0 && has_chroma {
        uv_stride += DAV1D_PICTURE_ALIGNMENT;
    }
    let y_sz = (y_stride >> hbd).checked_mul(aligned_h).ok_or(())?;
    let uv_sz = if has_chroma {
        (uv_stride >> hbd)
            .checked_mul(aligned_h >> ss_ver)
            .ok_or(())?
    } else {
        0
    };

    let planes = if hbd != 0 {
        PicPlanes::U16([
            try_vec(0u16, y_sz)?,
            try_vec(0u16, uv_sz)?,
            try_vec(0u16, uv_sz)?,
        ])
    } else {
        PicPlanes::U8([
            try_vec(0u8, y_sz)?,
            try_vec(0u8, uv_sz)?,
            try_vec(0u8, uv_sz)?,
        ])
    };

    Ok((
        PictureData {
            planes,
            stride: [y_stride >> hbd, uv_stride >> hbd],
        },
        [y_stride as isize, uv_stride as isize],
    ))
}

/// Translation of `Dav1dPicture`. The `*_ref` allocation origins are the
/// `Arc`s themselves.
#[derive(Clone, Debug, Default)]
pub(crate) struct Dav1dPicture {
    pub(crate) seq_hdr: Option<Arc<Dav1dSequenceHeader>>,
    pub(crate) frame_hdr: Option<Arc<Dav1dFrameHeader>>,

    /// The planar image data (`data[3]`, `ref`).
    pub(crate) data: Option<Arc<PictureData>>,

    /// Number of bytes between 2 lines in data[] for luma [0] or chroma [1].
    pub(crate) stride: [isize; 2],

    pub(crate) p: Dav1dPictureParameters,
    pub(crate) m: Dav1dDataProps,

    /// High Dynamic Range Content Light Level metadata applying to this picture,
    /// as defined in section 5.8.3 and 6.7.3
    pub(crate) content_light: Option<Arc<Dav1dContentLightLevel>>,
    /// High Dynamic Range Mastering Display Color Volume metadata applying to
    /// this picture, as defined in section 5.8.4 and 6.7.4
    pub(crate) mastering_display: Option<Arc<Dav1dMasteringDisplay>>,
    /// Array of ITU-T T.35 metadata as defined in section 5.8.2 and 6.7.2
    pub(crate) itut_t35: Option<Arc<Vec<Dav1dITUTT35>>>,
}

impl Dav1dPicture {
    /// Translation of `dav1d_picture_unref()`
    /// (`dav1d_picture_unref_internal()`).
    pub(crate) fn unref(&mut self) {
        *self = Dav1dPicture::default();
    }

    /// Translation of `dav1d_picture_copy_props()`.
    pub(crate) fn copy_props(
        &mut self,
        content_light: &Option<Arc<Dav1dContentLightLevel>>,
        mastering_display: &Option<Arc<Dav1dMasteringDisplay>>,
        itut_t35: &Option<Arc<Vec<Dav1dITUTT35>>>,
        props: &Dav1dDataProps,
    ) {
        self.m = props.clone();
        self.content_light = content_light.clone();
        self.mastering_display = mastering_display.clone();
        self.itut_t35 = itut_t35.clone();
    }
}

// enum PlaneType
pub(crate) const PLANE_TYPE_Y: i32 = 0;
pub(crate) const PLANE_TYPE_UV: i32 = 1;
pub(crate) const PLANE_TYPE_BLOCK: i32 = 2;
pub(crate) const PLANE_TYPE_ALL: i32 = 3;

// enum PictureFlags
pub(crate) const PICTURE_FLAG_NEW_SEQUENCE: u32 = 1 << 0;
pub(crate) const PICTURE_FLAG_NEW_OP_PARAMS_INFO: u32 = 1 << 1;
pub(crate) const PICTURE_FLAG_NEW_TEMPORAL_UNIT: u32 = 1 << 2;

/// Translation of `Dav1dThreadPicture` (without the frame threading
/// `progress` counters).
#[derive(Clone, Debug, Default)]
pub(crate) struct Dav1dThreadPicture {
    pub(crate) p: Dav1dPicture,
    pub(crate) visible: i32,
    // This can be set for inter frames, non-key intra frames, or for invisible
    // keyframes that have not yet been made visible using the show-existing-frame
    // mechanism.
    pub(crate) showable: i32,
    pub(crate) flags: u32,
}

impl Dav1dThreadPicture {
    /// Translation of `dav1d_thread_picture_unref()`.
    pub(crate) fn unref(&mut self) {
        *self = Dav1dThreadPicture::default();
    }
}

/// Translation of `picture_alloc_with_edges()`: the picture's metadata and
/// its (zeroed) planes.
#[allow(clippy::too_many_arguments)]
pub(crate) fn picture_alloc_with_edges(
    w: i32,
    h: i32,
    seq_hdr: &Option<Arc<Dav1dSequenceHeader>>,
    frame_hdr: &Option<Arc<Dav1dFrameHeader>>,
    bpc: i32,
) -> Result<(Dav1dPicture, PictureData), ()> {
    debug_assert!(bpc > 0 && bpc <= 16);

    let layout = seq_hdr.as_ref().map_or(0, |s| s.layout);
    let mut p = Dav1dPicture {
        p: Dav1dPictureParameters { w, h, layout, bpc },
        seq_hdr: seq_hdr.clone(),
        frame_hdr: frame_hdr.clone(),
        ..Default::default()
    };
    let (data, stride) = default_picture_alloc(&p.p)?;
    p.stride = stride;
    Ok((p, data))
}

/// Translation of `dav1d_picture_alloc_copy()`: allocate a picture with
/// identical metadata to an existing picture. The width is a separate
/// argument so this function can be used for super-res, where the width
/// changes, but everything else is the same. For the more typical use case
/// of allocating a new image of the same dimensions, use src->p.w as width.
pub(crate) fn picture_alloc_copy(
    w: i32,
    src: &Dav1dPicture,
) -> Result<(Dav1dPicture, PictureData), ()> {
    let (mut dst, data) =
        picture_alloc_with_edges(w, src.p.h, &src.seq_hdr, &src.frame_hdr, src.p.bpc)?;
    dst.copy_props(
        &src.content_light,
        &src.mastering_display,
        &src.itut_t35,
        &src.m,
    );
    Ok((dst, data))
}

// enum Dav1dEventFlags
/// The last returned picture contains a reference to a new Sequence Header,
/// either because it's the start of a new coded sequence, or the decoder was
/// flushed before it was generated.
pub(crate) const DAV1D_EVENT_FLAG_NEW_SEQUENCE: u32 = 1 << 0;
/// The last returned picture contains a reference to a Sequence Header with
/// new operating parameters information for the current coded sequence.
pub(crate) const DAV1D_EVENT_FLAG_NEW_OP_PARAMS_INFO: u32 = 1 << 1;

/// Translation of `dav1d_picture_get_event_flags()`.
pub(crate) fn picture_get_event_flags(p: &Dav1dThreadPicture) -> u32 {
    if p.flags == 0 {
        return 0;
    }

    let mut flags = 0;
    if p.flags & PICTURE_FLAG_NEW_SEQUENCE != 0 {
        flags |= DAV1D_EVENT_FLAG_NEW_SEQUENCE;
    }
    if p.flags & PICTURE_FLAG_NEW_OP_PARAMS_INFO != 0 {
        flags |= DAV1D_EVENT_FLAG_NEW_OP_PARAMS_INFO;
    }

    flags
}
