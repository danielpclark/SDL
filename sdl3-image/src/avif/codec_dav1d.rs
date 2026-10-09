// Rust translation of src/codec_dav1d.c from libavif
// (https://github.com/AOMediaCodec/libavif, at the revision SDL_image's
// external/libavif pins: libavif 1.1.1 with SDL's patches).
// Copyright 2019 Joe Drago. All rights reserved.
// SPDX-License-Identifier: BSD-2-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The AV1 decoder glue: libavif's dav1d codec, over the translation of
//! dav1d in `crate::dav1d`. As there is one codec, `avifCodec` is this
//! struct (its function pointers are its methods).

use crate::dav1d::{
    dav1d_close, dav1d_get_picture, dav1d_open, dav1d_send_data, dav1d_version, Dav1dContext,
    Dav1dData, Dav1dPicture, Dav1dSettings, DAV1D_MAX_THREADS,
};

use super::avif::{
    avif_image_free_planes, AvifChromaSamplePosition, AvifImage, AvifPixelFormat, AvifPlane,
    AvifPlaneData, AvifRange, AVIF_PLANES_A, AVIF_PLANES_ALL, AVIF_PLANES_YUV,
};
use super::internal::{AvifDecodeSample, AVIF_SPATIAL_ID_UNSET};

// For those building with an older version of dav1d (not recommended).
// (DAV1D_ERR() is the translation's)

/// `DAV1D_ERR(EAGAIN)`
fn dav1d_err_eagain() -> i32 {
    crate::dav1d::dav1d_err_eagain()
}

/// Translation of `struct avifCodecInternal`.
#[derive(Default)]
struct AvifCodecInternal {
    dav1d_context: Option<Box<Dav1dContext>>,
    dav1d_picture: Dav1dPicture,
    has_picture: bool,
    color_range: AvifRange,
}

/// Translation of `avifCodec` for dav1d (`csOptions` and `diag` are not
/// used by the decoder; `avifCodecDestroy()` is dropping it).
pub(crate) struct AvifCodec {
    internal: AvifCodecInternal,
    /// Operating point, defaults to 0.
    pub(crate) operating_point: u8,
    /// if true, the underlying codec must decode all layers, not just the best layer
    pub(crate) all_layers: bool,
}

impl std::fmt::Debug for AvifCodec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AvifCodec")
            .field("operating_point", &self.operating_point)
            .field("all_layers", &self.all_layers)
            .finish_non_exhaustive()
    }
}

// (avifDav1dFreeCallback(): the data is wrapped as a copy, which the
// decoder owns)

impl Drop for AvifCodec {
    /// Translation of `dav1dCodecDestroyInternal()`.
    fn drop(&mut self) {
        if self.internal.has_picture {
            self.internal.dav1d_picture.unref();
        }
        if self.internal.dav1d_context.is_some() {
            dav1d_close(&mut self.internal.dav1d_context);
        }
    }
}

impl AvifCodec {
    /// Translation of `dav1dCodecGetNextImage()`: `max_threads` and
    /// `image_size_limit` are the decoder's, `data` the sample's bytes.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn get_next_image(
        &mut self,
        max_threads: i32,
        image_size_limit: u32,
        sample: &AvifDecodeSample,
        data: &[u8],
        alpha: bool,
        is_limited_range_alpha: &mut bool,
        image: &mut AvifImage,
    ) -> bool {
        if self.internal.dav1d_context.is_none() {
            let mut dav1d_settings = Dav1dSettings::default();
            // Give all available threads to decode a single frame as fast as possible
            dav1d_settings.max_frame_delay = 1;
            dav1d_settings.n_threads = max_threads.clamp(1, DAV1D_MAX_THREADS);
            // Set a maximum frame size limit to avoid OOM'ing fuzzers. In 32-bit builds, if
            // frame_size_limit > 8192 * 8192, dav1d reduces frame_size_limit to 8192 * 8192 and logs
            // a message, so we set frame_size_limit to at most 8192 * 8192 to avoid the dav1d_log
            // message.
            dav1d_settings.frame_size_limit = if std::mem::size_of::<usize>() < 8 {
                image_size_limit.min(8192 * 8192)
            } else {
                image_size_limit
            };
            dav1d_settings.operating_point = self.operating_point as i32;
            dav1d_settings.all_layers = self.all_layers;

            match dav1d_open(&dav1d_settings) {
                Ok(c) => self.internal.dav1d_context = Some(c),
                Err(_) => return false,
            }
        }
        let Some(context) = self.internal.dav1d_context.as_mut() else {
            return false;
        };

        #[allow(unused_assignments)]
        let mut got_picture = false;
        let mut next_frame = Dav1dPicture::default();

        // (dav1d_data_wrap() of the sample's bytes, which fails for no data)
        if data.is_empty() {
            return false;
        }
        let mut dav1d_data = Dav1dData::from_slice(data);

        let mut res;
        loop {
            if dav1d_data.has_data() {
                res = dav1d_send_data(context, &mut dav1d_data);
                if (res < 0) && (res != dav1d_err_eagain()) {
                    dav1d_data.unref();
                    return false;
                }
            }

            res = dav1d_get_picture(context, &mut next_frame);
            if res == dav1d_err_eagain() {
                if dav1d_data.has_data() {
                    // send more data
                    continue;
                }
                return false;
            } else if res < 0 {
                // No more frames
                if dav1d_data.has_data() {
                    dav1d_data.unref();
                }
                return false;
            } else {
                // Got a picture!
                let spatial_id = next_frame.frame_hdr.as_ref().map_or(0, |h| h.spatial_id);
                if (sample.spatial_id != AVIF_SPATIAL_ID_UNSET)
                    && (sample.spatial_id as i32 != spatial_id)
                {
                    // Layer selection: skip this unwanted layer
                    next_frame.unref();
                } else {
                    got_picture = true;
                    break;
                }
            }
        }
        if dav1d_data.has_data() {
            dav1d_data.unref();
        }

        // Drain all buffered frames in the decoder.
        //
        // The sample should have only one frame of the desired layer. If there are more frames after
        // that frame, we need to discard them so that they won't be mistakenly output when the decoder
        // is used to decode another sample.
        let mut buffered_frame = Dav1dPicture::default();
        loop {
            res = dav1d_get_picture(context, &mut buffered_frame);
            if res < 0 {
                if res != dav1d_err_eagain() {
                    if got_picture {
                        next_frame.unref();
                    }
                    return false;
                }
            } else {
                buffered_frame.unref();
            }
            if res != 0 {
                break;
            }
        }

        if got_picture {
            self.internal.dav1d_picture.unref();
            self.internal.dav1d_picture = next_frame;
            self.internal.color_range = if self
                .internal
                .dav1d_picture
                .seq_hdr
                .as_ref()
                .is_some_and(|s| s.color_range != 0)
            {
                AvifRange::Full
            } else {
                AvifRange::Limited
            };
            self.internal.has_picture = true;
        } else if alpha && self.internal.has_picture {
            // Special case: reuse last alpha frame
        } else {
            return false;
        }

        let dav1d_image = &self.internal.dav1d_picture;
        let Some(pic_data) = dav1d_image.data.clone() else {
            return false;
        };
        let seq_hdr = dav1d_image.seq_hdr.as_deref();
        let is_color = !alpha;
        if is_color {
            // Color (YUV) planes - set image to correct size / format, fill color

            let yuv_format = match dav1d_image.p.layout {
                crate::dav1d::DAV1D_PIXEL_LAYOUT_I400 => AvifPixelFormat::Yuv400,
                crate::dav1d::DAV1D_PIXEL_LAYOUT_I420 => AvifPixelFormat::Yuv420,
                crate::dav1d::DAV1D_PIXEL_LAYOUT_I422 => AvifPixelFormat::Yuv422,
                crate::dav1d::DAV1D_PIXEL_LAYOUT_I444 => AvifPixelFormat::Yuv444,
                _ => AvifPixelFormat::None,
            };

            if image.width != 0 && image.height != 0 {
                if (image.width != dav1d_image.p.w as u32)
                    || (image.height != dav1d_image.p.h as u32)
                    || (image.depth != dav1d_image.p.bpc as u32)
                    || (image.yuv_format != yuv_format)
                {
                    // Throw it all out
                    avif_image_free_planes(image, AVIF_PLANES_ALL);
                }
            }
            image.width = dav1d_image.p.w as u32;
            image.height = dav1d_image.p.h as u32;
            image.depth = dav1d_image.p.bpc as u32;

            image.yuv_format = yuv_format;
            image.yuv_range = self.internal.color_range;
            image.yuv_chroma_sample_position =
                seq_hdr.map_or(0, |s| s.chr) as AvifChromaSamplePosition;

            image.color_primaries = seq_hdr.map_or(0, |s| s.pri) as u16;
            image.transfer_characteristics = seq_hdr.map_or(0, |s| s.trc) as u16;
            image.matrix_coefficients = seq_hdr.map_or(0, |s| s.mtrx) as u16;

            // Steal the pointers from the decoder's image directly
            avif_image_free_planes(image, AVIF_PLANES_YUV);
            let yuv_plane_count = if yuv_format == AvifPixelFormat::Yuv400 {
                1
            } else {
                3
            };
            for yuv_plane in 0..yuv_plane_count {
                image.yuv_planes[yuv_plane] = Some(AvifPlane {
                    data: AvifPlaneData::Picture(pic_data.clone(), yuv_plane),
                    offset: 0,
                });
                image.yuv_row_bytes[yuv_plane] =
                    dav1d_image.stride[if yuv_plane == 0 { 0 } else { 1 }] as u32;
            }
            image.image_owns_yuv_planes = false;
        } else {
            // Alpha plane - ensure image is correct size, fill color

            if image.width != 0 && image.height != 0 {
                if (image.width != dav1d_image.p.w as u32)
                    || (image.height != dav1d_image.p.h as u32)
                    || (image.depth != dav1d_image.p.bpc as u32)
                {
                    // Alpha plane doesn't match previous alpha plane decode, bail out
                    return false;
                }
            }
            image.width = dav1d_image.p.w as u32;
            image.height = dav1d_image.p.h as u32;
            image.depth = dav1d_image.p.bpc as u32;

            avif_image_free_planes(image, AVIF_PLANES_A);
            image.alpha_plane = Some(AvifPlane {
                data: AvifPlaneData::Picture(pic_data, 0),
                offset: 0,
            });
            image.alpha_row_bytes = dav1d_image.stride[0] as u32;
            *is_limited_range_alpha = self.internal.color_range == AvifRange::Limited;
            image.image_owns_alpha_plane = false;
        }
        true
    }
}

/// Translation of `avifCodecVersionDav1d()`.
pub(crate) fn avif_codec_version_dav1d() -> &'static str {
    dav1d_version()
}

/// Translation of `avifCodecCreateDav1d()`.
pub(crate) fn avif_codec_create_dav1d() -> Box<AvifCodec> {
    Box::new(AvifCodec {
        internal: AvifCodecInternal::default(),
        operating_point: 0,
        all_layers: false,
    })
}
