// Rust translation of src/IMG_jxl.c from SDL_image.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! JPEG XL images: the detector and the loader, over the translation of
//! libjxl's decoder in `jxl/`.

mod ac_context;
mod ac_strategy;
mod alpha;
mod ans_common;
mod base;
mod blending;
mod chroma_from_luma;
mod coeff_order;
mod color_encoding_internal;
#[allow(dead_code)]
mod color_management;
mod compressed_dc;
mod dec_ans;
mod dec_bit_reader;
mod dec_cache;
mod dec_context_map;
mod dec_external_image;
mod dec_frame;
mod dec_group;
mod dec_group_border;
mod dec_huffman;
mod dec_modular;
mod dec_noise;
mod dec_patch_dictionary;
mod dec_xyb;
mod decode;
mod epf;
mod fields;
mod frame_header;
mod headers;
mod huffman_table;
mod icc_codec;
mod image;
mod image_bundle;
mod image_metadata;
mod loop_filter;
mod math;
mod modular;
mod opsin_params;
mod passes_state;
mod quant_weights;
mod quantizer;
mod render_pipeline;
mod splines;
mod toc;
mod transfer_functions;

use sdl3::error::{Error, Result};
use sdl3::io::{IoStream, IoWhence};
use sdl3::video::PixelFormat;
use sdl3::video::Surface;

use crate::util::read_ok;
use decode::{
    JxlBasicInfo, JxlDataType, JxlDecoder, JxlDecoderStatus, JxlEndianness, JxlPixelFormat, JXL_DEC_BASIC_INFO,
    JXL_DEC_FULL_IMAGE,
};

/* See if an image is contained in a data source */

/// Whether `src` holds a JPEG XL codestream or container; the stream
/// position is unchanged. Translation of `IMG_isJXL()`.
pub fn is_jxl(src: &mut IoStream<'_>) -> bool {
    let start = src.tell().unwrap_or(-1);
    let mut is_jxl = false;
    let mut magic = [0u8; 12];
    if read_ok(src, &mut magic[..2]) {
        if magic[0] == 0xFF && magic[1] == 0x0A {
            /* This is a JXL codestream */
            is_jxl = true;
        } else if read_ok(src, &mut magic[2..])
            && magic
                == [
                    0x00, 0x00, 0x00, 0x0C, b'J', b'X', b'L', b' ', 0x0D, 0x0A, 0x87, 0x0A,
                ]
        {
            /* This is a JXL container */
            is_jxl = true;
        }
    }
    let _ = src.seek(start, IoWhence::Set);
    is_jxl
}

/* Load a JXL type image from an SDL datasource */

/// Load a JPEG XL image (the last frame of an animation) as an RGBA32
/// surface. Translation of `IMG_LoadJXL_IO()`; on failure the stream is
/// rewound to where it was.
pub(crate) fn load_jxl_io(src: &mut IoStream<'_>) -> Result<Surface<'static>> {
    let start = src.tell().unwrap_or(-1);

    // (IMG_InitJXL(): nothing to load)

    let result = load_jxl_io_internal(src);
    if result.is_err() {
        let _ = src.seek(start, IoWhence::Set);
    }
    result
}

fn load_jxl_io_internal(src: &mut IoStream<'_>) -> Result<Surface<'static>> {
    let data = src.load_all()?;

    let mut decoder = JxlDecoder::new();
    let format = JxlPixelFormat {
        num_channels: 4,
        data_type: JxlDataType::Uint8,
        endianness: JxlEndianness::Native,
        align: 0,
    };

    if decoder.subscribe_events(JXL_DEC_BASIC_INFO | JXL_DEC_FULL_IMAGE) != JxlDecoderStatus::Success {
        return Err(Error::new("Couldn't subscribe to JXL events"));
    }

    if decoder.set_input(&data) != JxlDecoderStatus::Success {
        return Err(Error::new("Couldn't set JXL input"));
    }

    let mut info = JxlBasicInfo::default();
    let mut pitch: usize = 0;
    let mut have_pixels = false;

    loop {
        let status = decoder.process_input();

        match status {
            JxlDecoderStatus::Error => {
                return Err(Error::new("JXL decoder error"));
            }
            JxlDecoderStatus::NeedMoreInput => {
                return Err(Error::new("Incomplete JXL image"));
            }
            JxlDecoderStatus::BasicInfo => {
                if decoder.get_basic_info(&mut info) != JxlDecoderStatus::Success {
                    return Err(Error::new("Couldn't get JXL image info"));
                }
            }
            JxlDecoderStatus::NeedImageOutBuffer => {
                let mut outputsize: usize = 0;
                if decoder.image_out_buffer_size(&format, &mut outputsize) != JxlDecoderStatus::Success {
                    return Err(Error::new("Couldn't get JXL image size"));
                }
                if info.xsize == 0 || info.ysize == 0 {
                    return Err(Error::new(format!(
                        "Couldn't get pixels for {}x{} JXL image",
                        info.xsize as i32, info.ysize as i32
                    )));
                }
                // (the previous frame's pixels are freed)
                let mut pixels: Vec<u8> = Vec::new();
                if pixels.try_reserve_exact(outputsize).is_err() {
                    return Err(Error::new("Out of memory"));
                }
                pixels.resize(outputsize, 0);
                if (outputsize / info.ysize as usize) > i32::MAX as usize {
                    return Err(Error::new("Out of memory"));
                }
                pitch = outputsize / info.ysize as usize;
                if decoder.set_image_out_buffer(&format, pixels) != JxlDecoderStatus::Success {
                    return Err(Error::new("Couldn't set JXL output buffer"));
                }
                have_pixels = true;
            }
            JxlDecoderStatus::FullImage => {
                /* We have a full image - in the case of an animation, keep decoding until the last frame */
            }
            JxlDecoderStatus::Success => {
                /* All done! */
                let pixels = if have_pixels { decoder.take_image_out_buffer() } else { None };
                return create_surface_from(info.xsize as i32, info.ysize as i32, pixels, pitch);
            }
            other => {
                return Err(Error::new(format!("Unknown JXL decoding status: {}", other.value())));
            }
        }
    }
}

/// `SDL_CreateSurfaceFrom(w, h, SDL_PIXELFORMAT_RGBA32, pixels, pitch)`,
/// with the surface taking over the pixels (copied into a new surface).
fn create_surface_from(w: i32, h: i32, pixels: Option<Vec<u8>>, pitch: usize) -> Result<Surface<'static>> {
    let mut surface = Surface::new(w, h, PixelFormat::RGBA32)?;
    let Some(pixels) = pixels else {
        // (SDL_CreateSurfaceFrom() with NULL pixels: an empty surface)
        return Ok(surface);
    };
    let dst_pitch = surface.pitch() as usize;
    let row = (w as usize) * 4;
    if let Some(dst) = surface.pixels_mut() {
        for y in 0..h as usize {
            let s = y * pitch;
            let d = y * dst_pitch;
            if s + row <= pixels.len() && d + row <= dst.len() {
                dst[d..d + row].copy_from_slice(&pixels[s..s + row]);
            }
        }
    }
    Ok(surface)
}
