// Rust translation of src/IMG_webp.c from SDL_image.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! WebP images: the detector, the loader (lossy, lossless, with alpha, and
//! the first frame of an animation) and the animation decoder, over the
//! translation of the parts of libwebp they use (in `webp/`: the VP8 and
//! VP8L decoders, the alpha plane decoder and the demuxer), as upstream
//! builds it with libwebp linked in (no dynamic loading: `IMG_InitWEBP()`
//! has nothing to load). The encoders (`IMG_SaveWEBP_IO()` and the WebP
//! animation encoder) need libwebp's encoder and muxer, which are not
//! translated; they fail as upstream built without `SAVE_WEBP`.

//=============================================================================
//        File: SDL_webp.c
//     Purpose: A WEBP loader for the SDL library
//    Revision:
//  Created by: Michael Bonfils (Murlock) (26 November 2011)
//              murlock42@gmail.com
//
//=============================================================================

mod dec;
mod decode;
mod demux;
mod dsp;
mod utils;

use sdl3::error::{Error, Result};
use sdl3::io::{IoStream, IoWhence};
use sdl3::properties::Properties;
use sdl3::video::{BlendMode, PixelFormat, Rect, Surface};

use crate::anim_decoder::{
    DecoderCore, PROP_METADATA_AUTHOR_STRING, PROP_METADATA_COPYRIGHT_STRING,
    PROP_METADATA_CREATION_TIME_STRING, PROP_METADATA_DESCRIPTION_STRING,
    PROP_METADATA_FRAME_COUNT_NUMBER, PROP_METADATA_IGNORE_PROPS_BOOLEAN,
    PROP_METADATA_LOOP_COUNT_NUMBER, PROP_METADATA_TITLE_STRING,
};
use crate::util::{read_ok, read_up_to};
use crate::xmlman;
use dec::webp_dec::{webp_decode_rgb_into, webp_decode_rgba_into, webp_get_features};
use decode::{VP8StatusCode, WebPBitstreamFeatures, WebPMuxAnimBlend, WebPMuxAnimDispose};
use demux::{
    webp_demux, webp_demux_get_chunk, webp_demux_get_frame, webp_demux_next_frame,
    webp_demux_release_chunk_iterator, webp_demux_release_iterator, WebPChunkIterator, WebPDemuxer,
    WebPFormatFeature, WebPIterator,
};

/// Whether `src` holds a WebP image, and if so (with `datasize`) the size
/// of the data from the stream position to its end. Translation of
/// `webp_getinfo()`.
fn webp_getinfo(src: &mut IoStream<'_>, datasize: Option<&mut usize>) -> bool {
    let start = src.tell().unwrap_or(-1);
    let mut is_webp = false;
    let mut magic = [0u8; 20];
    if read_ok(src, &mut magic)
        && &magic[0..4] == b"RIFF"
        && &magic[8..12] == b"WEBP"
        && &magic[12..15] == b"VP8"
        && (magic[15] == b' ' || magic[15] == b'X' || magic[15] == b'L')
    {
        is_webp = true;
        if let Some(datasize) = datasize {
            let size = src.size().unwrap_or(-1);
            if size > 0 {
                *datasize = (size - start) as usize;
            } else {
                *datasize = 0;
            }
        }
    }
    let _ = src.seek(start, IoWhence::Set);
    is_webp
}

/* See if an image is contained in a data source */

/// Whether `src` holds a WebP image; the stream position is unchanged.
/// Translation of `IMG_isWEBP()`.
pub fn is_webp(src: &mut IoStream<'_>) -> bool {
    webp_getinfo(src, None)
}

/// Load a WebP image: an RGB24 surface, or RGBA32 with alpha; for an
/// animated WebP, the first frame of the animation (an RGBA32 or RGBX32
/// canvas, with the animation's metadata properties). Translation of
/// `IMG_LoadWEBP_IO()`.
pub fn load_webp_io(src: &mut IoStream<'_>) -> Result<Surface<'static>> {
    let start = src.tell().unwrap_or(-1);

    let result = load_webp(src, start);
    if result.is_err() {
        let _ = src.seek(start, IoWhence::Set);
    }
    result
}

/// The body of `IMG_LoadWEBP_IO()` (its `error:` path is the caller's).
fn load_webp(src: &mut IoStream<'_>, start: i64) -> Result<Surface<'static>> {
    let error = |message: &'static str| Err(Error::new(message));

    // (IMG_InitWEBP(): libwebp is linked in)

    let mut raw_data_size = usize::MAX;
    if !webp_getinfo(src, Some(&mut raw_data_size)) {
        return error("Invalid WEBP");
    }

    let Some(raw_data) = read_up_to(src, raw_data_size) else {
        return error("Failed to allocate enough buffer for WEBP");
    };

    if raw_data.len() != raw_data_size {
        return error("Failed to read WEBP");
    }

    let mut features = WebPBitstreamFeatures::default();
    if webp_get_features(&raw_data, &mut features) != VP8StatusCode::Ok {
        return error("WebPGetFeatures has failed");
    }

    // Special casing for animated WebP images to extract a single frame.
    if features.has_animation {
        if src.seek(start, IoWhence::Set).is_err() {
            return error("Failed to seek IO to read animated WebP");
        } else {
            // FIXME (upstream): IMG_DecodeAsAnimation(src, "webp", 1) falls
            // back to the single-frame decoder when the WebP animation
            // decoder can't be created (an animation the demuxer rejects),
            // which loads the image with IMG_LoadTyped_IO(): that detects a
            // WebP and calls IMG_LoadWEBP_IO() again, without end. Here the
            // animation decoder's error ends it.
            let animation =
                crate::anim_decoder::decode_as_animation_without_fallback(src, "webp", 1);
            match animation {
                Ok(mut animation) if animation.count() > 0 => {
                    // (frames[0] is never NULL here)
                    return Ok(animation.frames.swap_remove(0));
                }
                _ => {
                    return error("Received an animated WebP but the animation data was invalid");
                }
            }
        }
    }

    let format = if features.has_alpha {
        PixelFormat::RGBA32
    } else {
        PixelFormat::RGB24
    };

    let Ok(mut surface) = Surface::new(features.width, features.height, format) else {
        return error("Failed to allocate SDL_Surface");
    };

    let pitch = surface.pitch();
    let h = surface.height() as usize;
    let ret = match surface.pixels_mut() {
        Some(pixels) => {
            let size = (pitch as usize * h).min(pixels.len());
            let pixels = &mut pixels[..size];
            if features.has_alpha {
                webp_decode_rgba_into(&raw_data, pixels, pitch)
            } else {
                webp_decode_rgb_into(&raw_data, pixels, pitch)
            }
        }
        None => None,
    };

    if ret.is_none() {
        return error("Failed to decode WEBP");
    }

    Ok(surface)
}

/// The WebP animation decoder's state. Translation of IMG_webp.c's
/// `struct IMG_AnimationDecoderContext` (`raw_data`, which the demuxer
/// parses, is the demuxer's).
pub(crate) struct WebpDecoderContext {
    demuxer: WebPDemuxer,
    iter: WebPIterator,
    canvas: Surface<'static>,
    dispose_method: WebPMuxAnimDispose,
    bgcolor: u32,
    last_rect: Rect,
    has_alpha: bool,
}

impl WebpDecoderContext {
    /// Translation of `IMG_AnimationDecoderReset_Internal()`.
    pub(crate) fn reset(&mut self) {
        webp_demux_release_iterator(&mut self.iter);
        self.iter = WebPIterator::default();
        self.dispose_method = WebPMuxAnimDispose::Background;

        /* Reset disposal rect to full canvas */
        self.last_rect = Rect::new(0, 0, self.canvas.width(), self.canvas.height());
    }

    /// Translation of `IMG_AnimationDecoderGetNextFrame_Internal()`:
    /// `Ok(None)` (with the COMPLETE status) when there are no more frames.
    pub(crate) fn get_next_frame(
        &mut self,
        d: &mut DecoderCore<'_, '_>,
    ) -> Result<Option<(Surface<'static>, u64)>> {
        // Get the next frame from the demuxer.
        if self.iter.frame_num < 1 {
            if !webp_demux_get_frame(&self.demuxer, 1, &mut self.iter) {
                return Err(Error::new("Failed to get first frame from WEBP demuxer"));
            }
        } else if !webp_demux_next_frame(&self.demuxer, &mut self.iter) {
            d.status = crate::AnimationDecoderStatus::Complete;
            return Ok(None);
        }

        let total_frames = self.iter.num_frames;
        let available_frames = total_frames - (self.iter.frame_num - 1);

        if available_frames < 1 {
            d.status = crate::AnimationDecoderStatus::Complete;
            return Ok(None);
        }

        let bgcolor = self.bgcolor;
        let dispose_method = self.dispose_method;
        let iter = self.iter;
        let has_alpha = self.has_alpha;
        let last_rect = self.last_rect;
        let canvas = &mut self.canvas;

        let mut curr = Surface::new(iter.width, iter.height, PixelFormat::RGBA32)?;

        let pitch = curr.pitch();
        let h = curr.height() as usize;
        let decoded = match curr.pixels_mut() {
            Some(pixels) => {
                let size = (pitch as usize * h).min(pixels.len());
                webp_decode_rgba_into(
                    self.demuxer.bytes(iter.fragment),
                    &mut pixels[..size],
                    pitch,
                )
            }
            None => None,
        };
        if decoded.is_none() {
            return Err(Error::new("Failed to decode frame"));
        }

        let dst = Rect::new(iter.x_offset, iter.y_offset, iter.width, iter.height);
        /* Correctly handle both Disposal and Blend modes to prevent ghosting */
        if dispose_method == WebPMuxAnimDispose::Background
            || iter.blend_method == WebPMuxAnimBlend::NoBlend
        {
            /* For alpha WebPs, we clear to transparency regardless of bad bgcolor metadata */
            let fill_color = if has_alpha {
                canvas.map_rgba(0, 0, 0, 0)
            } else {
                bgcolor
            };

            /* If it's a disposal, clear the previous area; if it's NO_BLEND, clear the current area */
            if dispose_method == WebPMuxAnimDispose::Background {
                let _ = canvas.fill_rect(Some(&last_rect), fill_color);
            }

            if iter.blend_method == WebPMuxAnimBlend::NoBlend {
                let _ = canvas.fill_rect(Some(&dst), fill_color);
            }
        }

        if iter.blend_method == WebPMuxAnimBlend::Blend {
            curr.set_blend_mode(BlendMode::BLEND)?;
        } else {
            curr.set_blend_mode(BlendMode::NONE)?;
        }

        curr.blit(None, canvas, Some(&dst))?;
        drop(curr);

        let retval = canvas.duplicate()?;

        let duration = d.decoder_duration(iter.duration as u64, 1000);

        /* Update state for next frame */
        self.dispose_method = iter.dispose_method;
        self.last_rect = dst;

        Ok(Some((retval, duration)))
    }
}

/// Create the WebP decoder of an animation decoder: the whole stream
/// demuxed, the canvas (cleared to transparency, or with the background
/// color when the file has no alpha) and, unless
/// [`PROP_METADATA_IGNORE_PROPS_BOOLEAN`] is set, the frame and loop counts
/// and the XMP metadata in the decoder's properties. Translation of
/// `IMG_CreateWEBPAnimationDecoder()` (its `IMG_AnimationDecoderClose_Internal()`
/// is dropping the context).
pub(crate) fn create_webp_animation_decoder(
    d: &mut DecoderCore<'_, '_>,
    props: &Properties,
) -> Result<Box<WebpDecoderContext>> {
    // (IMG_InitWEBP(): libwebp is linked in)

    if !webp_getinfo(d.src(), None) {
        // (SDL_GetError() is whatever the last error was; this message is
        // dropped for the single-frame decoder's)
        return Err(Error::new("Not a valid WebP file: "));
    }

    let stream_size = d.src().size().unwrap_or(-1);
    if stream_size <= 0 {
        return Err(Error::new(format!(
            "Stream has no data (size: {stream_size})"
        )));
    }

    // (SDL_malloc(raw_data_size) and SDL_ReadIO() are read_up_to(), the
    // buffer growing as the data arrives)
    let raw_data_size = stream_size as usize;
    let start = d.start;
    if d.src().seek(start, IoWhence::Set).is_err() {
        return Err(crate::util::read_error(d.src()));
    }
    let Some(raw_data) = read_up_to(d.src(), raw_data_size) else {
        return Err(Error::out_of_memory());
    };
    if raw_data.len() != raw_data_size {
        return Err(crate::util::read_error(d.src()));
    }

    let Some(demuxer) = webp_demux(raw_data) else {
        return Err(Error::new(
            "WebPDemux failed to initialize demuxer (not a valid WebP file or corrupted data)",
        ));
    };

    let width = demuxer.get_i(WebPFormatFeature::CanvasWidth);
    let height = demuxer.get_i(WebPFormatFeature::CanvasHeight);
    let flags = demuxer.get_i(WebPFormatFeature::FormatFlags);

    let ignore_props = props
        .get_bool(PROP_METADATA_IGNORE_PROPS_BOOLEAN)
        .unwrap_or(false);
    if !ignore_props {
        // Allow implicit properties to be set which are not globalized but specific to the decoder.
        let _ = d.props.set(
            PROP_METADATA_FRAME_COUNT_NUMBER,
            demuxer.get_i(WebPFormatFeature::FrameCount) as i64,
        );

        // Set well-defined properties.
        let _ = d.props.set(
            PROP_METADATA_LOOP_COUNT_NUMBER,
            demuxer.get_i(WebPFormatFeature::LoopCount) as i64,
        );

        // Get other well-defined properties and set them in our props.
        let mut xmp_iter = WebPChunkIterator::default();
        if webp_demux_get_chunk(&demuxer, b"XMP ", 1, &mut xmp_iter) {
            let chunk = demuxer.bytes(xmp_iter.chunk);
            if !chunk.is_empty() {
                let desc = xmlman::get_xmp_description(chunk);
                let rights = xmlman::get_xmp_copyright(chunk);
                let title = xmlman::get_xmp_title(chunk);
                let creator = xmlman::get_xmp_creator(chunk);
                let createdate = xmlman::get_xmp_create_date(chunk);
                if let Some(desc) = desc {
                    let _ = d.props.set(PROP_METADATA_DESCRIPTION_STRING, desc);
                }
                if let Some(rights) = rights {
                    let _ = d.props.set(PROP_METADATA_COPYRIGHT_STRING, rights);
                }
                if let Some(title) = title {
                    let _ = d.props.set(PROP_METADATA_TITLE_STRING, title);
                }
                if let Some(creator) = creator {
                    let _ = d.props.set(PROP_METADATA_AUTHOR_STRING, creator);
                }
                if let Some(createdate) = createdate {
                    let _ = d.props.set(PROP_METADATA_CREATION_TIME_STRING, createdate);
                }
            }
            webp_demux_release_chunk_iterator(&mut xmp_iter);
        }
    }

    let has_alpha = (flags & 0x10) != 0;

    let mut canvas = Surface::new(
        width as i32,
        height as i32,
        if has_alpha {
            PixelFormat::RGBA32
        } else {
            PixelFormat::RGBX32
        },
    )?;

    let bgcolor = demuxer.get_i(WebPFormatFeature::BackgroundColor);
    #[cfg(target_endian = "big")]
    let bgcolor = canvas.map_rgba(
        ((bgcolor >> 8) & 0xFF) as u8,
        ((bgcolor >> 16) & 0xFF) as u8,
        ((bgcolor >> 24) & 0xFF) as u8,
        (bgcolor & 0xFF) as u8,
    );
    #[cfg(target_endian = "little")]
    let bgcolor = canvas.map_rgba(
        ((bgcolor >> 16) & 0xFF) as u8,
        ((bgcolor >> 8) & 0xFF) as u8,
        (bgcolor & 0xFF) as u8,
        ((bgcolor >> 24) & 0xFF) as u8,
    );

    if has_alpha {
        let transparent = canvas.map_rgba(0, 0, 0, 0);
        let _ = canvas.fill_rect(None, transparent);
    } else {
        let _ = canvas.fill_rect(None, bgcolor);
    }

    Ok(Box::new(WebpDecoderContext {
        demuxer,
        iter: WebPIterator::default(),
        canvas,
        dispose_method: WebPMuxAnimDispose::Background,
        bgcolor,
        last_rect: Rect::new(0, 0, width as i32, height as i32),
        has_alpha,
    }))
}

// (SAVE_WEBP: IMG_SaveWEBP_IO(), IMG_SaveWEBP() and the animation encoder
// need libwebp's encoder and muxer, which are not translated; see
// crate::img::save_typed_io and crate::anim_encoder for the !SAVE_WEBP
// errors.)

/// The bitstream's dimensions as WebPGetFeatures() reads them (the canvas,
/// for an animation), if its header parses.
#[cfg(test)]
pub(crate) fn bitstream_dimensions(data: &[u8]) -> Option<(i32, i32)> {
    let mut features = WebPBitstreamFeatures::default();
    (webp_get_features(data, &mut features) == VP8StatusCode::Ok)
        .then_some((features.width, features.height))
}
