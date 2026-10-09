// Rust translation of src/lib.c and include/dav1d/dav1d.h from dav1d
// (https://code.videolan.org/videolan/dav1d, at the revision SDL_image's
// external/dav1d pins).
// Copyright © 2018-2021, VideoLAN and dav1d authors
// Copyright © 2018, Two Orioles, LLC
// SPDX-License-Identifier: BSD-2-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! dav1d, the AV1 decoder SDL_image's libavif uses.

// (the translation of dav1d keeps upstream's loops over indices, argument
// lists, late initializations, explicit arithmetic and switch layouts as
// they are)
#![allow(
    dead_code,
    clippy::absurd_extreme_comparisons,
    clippy::collapsible_else_if,
    clippy::collapsible_if,
    clippy::collapsible_match,
    clippy::derivable_impls,
    clippy::erasing_op,
    clippy::explicit_counter_loop,
    clippy::identity_op,
    clippy::int_plus_one,
    clippy::manual_is_multiple_of,
    clippy::manual_range_contains,
    clippy::misrefactored_assign_op,
    clippy::needless_late_init,
    clippy::needless_range_loop,
    clippy::precedence,
    clippy::too_many_arguments
)]

mod bitdepth;
mod cdef;
mod cdef_apply;
mod cdf;
mod data;
mod decode;
mod dequant_tables;
mod env;
mod fg_apply;
mod filmgrain;
mod getbits;
mod headers;
mod internal;
mod intops;
mod intra_edge;
mod ipred;
mod ipred_prepare;
mod itx;
mod itx_1d;
mod levels;
mod lf_apply;
mod lf_mask;
mod log;
mod loopfilter;
mod looprestoration;
mod lr_apply;
mod mc;
mod mem;
mod msac;
mod obu;
mod picture;
mod qm;
mod recon;
mod refmvs;
mod scan;
mod tables;
mod warpmv;
mod wedge;

use std::sync::Arc;

// (the decoder's API, for the AVIF loader)
#[allow(unused_imports)]
pub(crate) use data::{Dav1dData, Dav1dDataProps};
#[allow(unused_imports)]
pub(crate) use headers::{
    Dav1dFrameHeader, Dav1dSequenceHeader, DAV1D_PIXEL_LAYOUT_I400, DAV1D_PIXEL_LAYOUT_I420,
    DAV1D_PIXEL_LAYOUT_I422, DAV1D_PIXEL_LAYOUT_I444,
};
#[allow(unused_imports)]
pub(crate) use internal::{
    Dav1dContext, Dav1dLogger, DAV1D_DECODEFRAMETYPE_ALL, DAV1D_DECODEFRAMETYPE_INTRA,
    DAV1D_DECODEFRAMETYPE_KEY, DAV1D_DECODEFRAMETYPE_REFERENCE, DAV1D_INLOOPFILTER_ALL,
    DAV1D_INLOOPFILTER_CDEF, DAV1D_INLOOPFILTER_DEBLOCK, DAV1D_INLOOPFILTER_NONE,
    DAV1D_INLOOPFILTER_RESTORATION,
};
#[allow(unused_imports)]
pub(crate) use picture::{Dav1dPicture, PicPlanes, PictureData};

use decode::{dav1d_err, EAGAIN, EINVAL, ENOMEM};
use internal::{Dav1dFrameContext, Dav1dTaskContext};
use picture::{picture_alloc_copy, Dav1dThreadPicture};

/// `DAV1D_ERR(EAGAIN)`: the data or picture isn't ready yet.
pub(crate) const fn dav1d_err_eagain() -> i32 {
    dav1d_err(EAGAIN)
}

/// `DAV1D_MAX_THREADS`
pub(crate) const DAV1D_MAX_THREADS: i32 = 256;
/// `DAV1D_MAX_FRAME_DELAY`
pub(crate) const DAV1D_MAX_FRAME_DELAY: i32 = 256;

/// Translation of `Dav1dSettings`. The picture allocator is always the
/// default one (zeroed planes in vectors); `n_threads` and
/// `max_frame_delay` are validated like upstream but decoding always runs
/// on the calling thread with a frame delay of one.
#[derive(Clone)]
pub(crate) struct Dav1dSettings {
    /// number of threads (0 = number of logical cores in host system, default 0)
    pub(crate) n_threads: i32,
    /// Set to 1 for low-latency decoding (0 = ceil(sqrt(n_threads)), default 0)
    pub(crate) max_frame_delay: i32,
    /// whether to apply film grain on output frames (default 1)
    pub(crate) apply_grain: bool,
    /// select an operating point for scalable AV1 bitstreams (0 - 31, default 0)
    pub(crate) operating_point: i32,
    /// output all spatial layers of a scalable AV1 biststream (default 1)
    pub(crate) all_layers: bool,
    /// maximum frame size, in pixels (0 = unlimited, default 0)
    pub(crate) frame_size_limit: u32,
    /// Logger callback.
    pub(crate) logger: Dav1dLogger,
    /// strictly validate compliance with the AV1 specification (default 0)
    pub(crate) strict_std_compliance: bool,
    /// output invisibly coded frames (in coding order) in addition
    /// to all visible frames. Because of show-existing-frame, this
    /// means some frames may appear twice (once when coded,
    /// once when shown, default 0)
    pub(crate) output_invisible_frames: bool,
    /// postfilters to enable during decoding (default DAV1D_INLOOPFILTER_ALL)
    pub(crate) inloop_filters: u32,
    /// frame types to decode (default DAV1D_DECODEFRAMETYPE_ALL)
    pub(crate) decode_frame_type: u32,
}

/// Translation of `dav1d_version()`.
pub(crate) fn dav1d_version() -> &'static str {
    "1.2.1"
}

impl Default for Dav1dSettings {
    /// Translation of `dav1d_default_settings()`.
    fn default() -> Self {
        Dav1dSettings {
            n_threads: 0,
            max_frame_delay: 0,
            apply_grain: true,
            operating_point: 0,
            all_layers: true, // just until the tests are adjusted
            frame_size_limit: 0,
            logger: log::log_default_callback(),
            strict_std_compliance: false,
            output_invisible_frames: false,
            inloop_filters: DAV1D_INLOOPFILTER_ALL,
            decode_frame_type: DAV1D_DECODEFRAMETYPE_ALL,
        }
    }
}

/// Translation of `dav1d_get_frame_delay()`: with one frame decoded at a
/// time, this is always 1.
pub(crate) fn dav1d_get_frame_delay(s: &Dav1dSettings) -> Result<i32, i32> {
    if !(0..=DAV1D_MAX_THREADS).contains(&s.n_threads)
        || !(0..=DAV1D_MAX_FRAME_DELAY).contains(&s.max_frame_delay)
    {
        return Err(dav1d_err(EINVAL));
    }
    Ok(1)
}

/// Translation of `dav1d_open()`.
pub(crate) fn dav1d_open(s: &Dav1dSettings) -> Result<Box<Dav1dContext>, i32> {
    if !(0..=DAV1D_MAX_THREADS).contains(&s.n_threads)
        || !(0..=DAV1D_MAX_FRAME_DELAY).contains(&s.max_frame_delay)
        || !(0..=31).contains(&s.operating_point)
        || !(DAV1D_DECODEFRAMETYPE_ALL..=DAV1D_DECODEFRAMETYPE_KEY).contains(&s.decode_frame_type)
    {
        return Err(dav1d_err(EINVAL));
    }

    let mut c = Box::new(Dav1dContext {
        fc: Box::new(Dav1dFrameContext::default()),
        tc: Box::new(Dav1dTaskContext::default()),
        tile: Vec::new(),
        n_tile_data: 0,
        n_tiles: 0,
        seq_hdr: None,
        frame_hdr: None,
        content_light: None,
        mastering_display: None,
        itut_t35: None,
        in_: Dav1dData::default(),
        out: Dav1dThreadPicture::default(),
        cache: Dav1dThreadPicture::default(),
        refs: Default::default(),
        cdf: Default::default(),
        apply_grain: s.apply_grain,
        operating_point: s.operating_point,
        operating_point_idc: 0,
        all_layers: s.all_layers,
        max_spatial_id: 0,
        frame_size_limit: s.frame_size_limit,
        strict_std_compliance: s.strict_std_compliance,
        output_invisible_frames: s.output_invisible_frames,
        inloop_filters: s.inloop_filters,
        decode_frame_type: s.decode_frame_type,
        drain: false,
        frame_flags: 0,
        event_flags: 0,
        cached_error_props: Dav1dDataProps::default(),
        cached_error: 0,
        logger: s.logger.clone(),
    });

    /* On 32-bit systems extremely large frame sizes can cause overflows in
     * dav1d_decode_frame() malloc size calculations. Prevent that from occuring
     * by enforcing a maximum frame size limit, chosen to roughly correspond to
     * the largest size possible to decode without exhausting virtual memory. */
    if std::mem::size_of::<usize>() < 8 && s.frame_size_limit.wrapping_sub(1) >= 8192 * 8192 {
        c.frame_size_limit = 8192 * 8192;
        if s.frame_size_limit != 0 {
            log::dav1d_log(
                &c,
                &format!(
                    "Frame size limit reduced from {} to {}.\n",
                    s.frame_size_limit, c.frame_size_limit
                ),
            );
        }
    }

    Ok(c)
}

fn has_grain(pic: &Dav1dPicture) -> bool {
    let fgdata = &pic
        .frame_hdr
        .as_ref()
        .expect("frame header")
        .film_grain
        .data;
    fgdata.num_y_points != 0
        || fgdata.num_uv_points[0] != 0
        || fgdata.num_uv_points[1] != 0
        || (fgdata.clip_to_restricted_range != 0 && fgdata.chroma_scaling_from_luma != 0)
}

fn output_image(c: &mut Dav1dContext, out: &mut Dav1dPicture) -> i32 {
    let mut res = 0;

    let use_out = c.all_layers || c.max_spatial_id == 0;
    let in_ = if use_out {
        std::mem::take(&mut c.out)
    } else {
        std::mem::take(&mut c.cache)
    };
    if !c.apply_grain || !has_grain(&in_.p) {
        *out = in_.p;
    } else {
        res = dav1d_apply_grain(c, out, &in_.p);
    }
    if !c.all_layers && c.max_spatial_id != 0 && c.out.p.data.is_some() {
        c.cache = std::mem::take(&mut c.out);
    }
    res
}

fn output_picture_ready(c: &mut Dav1dContext, drain: bool) -> bool {
    if c.cached_error != 0 {
        return true;
    }
    if !c.all_layers && c.max_spatial_id != 0 {
        if c.out.p.data.is_some() && c.cache.p.data.is_some() {
            if c.max_spatial_id
                == c.cache
                    .p
                    .frame_hdr
                    .as_ref()
                    .expect("frame header")
                    .spatial_id
                || c.out.flags & picture::PICTURE_FLAG_NEW_TEMPORAL_UNIT != 0
            {
                return true;
            }
            c.cache = std::mem::take(&mut c.out);
            return false;
        } else if c.cache.p.data.is_some() && drain {
            return true;
        } else if c.out.p.data.is_some() {
            c.cache = std::mem::take(&mut c.out);
            return false;
        }
    }

    c.out.p.data.is_some()
}

// (drain_picture() drains the frame threads' delayed output queue)

fn gen_picture(c: &mut Dav1dContext) -> i32 {
    if output_picture_ready(c, false) {
        return 0;
    }

    while c.in_.sz > 0 {
        let in_ = c.in_.clone();
        let res = obu::dav1d_parse_obus(c, &in_);
        match res {
            Err(_) => c.in_.unref(),
            Ok(n) => {
                debug_assert!(n <= c.in_.sz);
                c.in_.sz -= n;
                c.in_.offset += n;
                if c.in_.sz == 0 {
                    c.in_.unref();
                }
            }
        }
        if output_picture_ready(c, false) {
            break;
        }
        if let Err(e) = res {
            return e;
        }
    }

    0
}

/// Translation of `dav1d_send_data()`: on success the data is consumed
/// (`in_` is unreferenced); on `DAV1D_ERR(EAGAIN)` it is kept for a later
/// call.
pub(crate) fn dav1d_send_data(c: &mut Dav1dContext, in_: &mut Dav1dData) -> i32 {
    if in_.has_data() {
        if in_.sz == 0 || in_.sz > usize::MAX / 2 {
            return dav1d_err(EINVAL);
        }
        c.drain = false;
    }
    if c.in_.has_data() {
        return dav1d_err(EAGAIN);
    }
    c.in_ = in_.clone();

    let res = gen_picture(c);
    if res == 0 {
        in_.unref();
    }

    res
}

/// Translation of `dav1d_get_picture()`.
pub(crate) fn dav1d_get_picture(c: &mut Dav1dContext, out: &mut Dav1dPicture) -> i32 {
    c.drain = true;

    let res = gen_picture(c);
    if res < 0 {
        return res;
    }

    if c.cached_error != 0 {
        let res = c.cached_error;
        c.cached_error = 0;
        return res;
    }

    if output_picture_ready(c, true) {
        return output_image(c, out);
    }

    dav1d_err(EAGAIN)
}

/// Translation of `dav1d_apply_grain()`.
pub(crate) fn dav1d_apply_grain(
    c: &mut Dav1dContext,
    out: &mut Dav1dPicture,
    in_: &Dav1dPicture,
) -> i32 {
    let _ = c;
    if !has_grain(in_) {
        *out = in_.clone();
        return 0;
    }

    let (mut pic, mut data) = match picture_alloc_copy(in_.p.w, in_) {
        Ok(v) => v,
        Err(_) => {
            out.unref();
            return dav1d_err(ENOMEM);
        }
    };

    let in_data = in_.data.as_deref().expect("picture data");
    let fgdata = &in_
        .frame_hdr
        .as_ref()
        .expect("frame header")
        .film_grain
        .data;
    let is_id = in_.seq_hdr.as_ref().expect("sequence header").mtrx == headers::DAV1D_MC_IDENTITY;
    match pic.p.bpc {
        8 => fg_apply::apply_grain::<u8>(&mut data, in_data, &pic.p, fgdata, is_id),
        _ => fg_apply::apply_grain::<u16>(&mut data, in_data, &pic.p, fgdata, is_id),
    }
    pic.data = Some(Arc::new(data));
    *out = pic;

    0
}

/// Translation of `dav1d_flush()`.
pub(crate) fn dav1d_flush(c: &mut Dav1dContext) {
    c.in_.unref();
    c.out.unref();
    c.cache.unref();

    c.drain = false;
    c.cached_error = 0;

    for i in 0..8 {
        c.refs[i].p.unref();
        c.refs[i].segmap = None;
        c.refs[i].refmvs = None;
        c.cdf[i] = Default::default();
    }
    c.frame_hdr = None;
    c.seq_hdr = None;

    c.mastering_display = None;
    c.content_light = None;
    c.itut_t35 = None;

    c.cached_error_props = Dav1dDataProps::default();
}

/// Translation of `dav1d_close()`: flushes and frees the context.
pub(crate) fn dav1d_close(c: &mut Option<Box<Dav1dContext>>) {
    if let Some(ctx) = c.as_mut() {
        dav1d_flush(ctx);
    }
    *c = None;
}

/// Translation of `dav1d_get_event_flags()`.
pub(crate) fn dav1d_get_event_flags(c: &mut Dav1dContext) -> u32 {
    let flags = c.event_flags;
    c.event_flags = 0;
    flags
}

/// Translation of `dav1d_get_decode_error_data_props()`.
pub(crate) fn dav1d_get_decode_error_data_props(c: &mut Dav1dContext) -> Dav1dDataProps {
    std::mem::take(&mut c.cached_error_props)
}

/// Translation of `dav1d_parse_sequence_header()`.
pub(crate) fn dav1d_parse_sequence_header(data: &[u8]) -> Result<Dav1dSequenceHeader, i32> {
    let mut out = Dav1dSequenceHeader::default();
    obu::dav1d_parse_sequence_header(&mut out, data)?;
    Ok(out)
}

#[cfg(test)]
mod tests;
