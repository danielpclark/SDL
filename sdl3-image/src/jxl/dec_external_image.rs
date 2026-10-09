// Rust translation of lib/jxl/dec_external_image.h and
// lib/jxl/dec_external_image.cc from libjxl (https://github.com/libjxl/libjxl,
// at the revision SDL_image's external/libjxl pins: libjxl 0.7.3 with SDL's
// patches).
// Copyright (c) the JPEG XL Project Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Converts an image bundle to an interleaved external buffer. (Output
//! callbacks and half-float output are not translated: SDL_image asks for
//! 8-bit RGBA in a buffer.)

use super::alpha::unpremultiply_alpha;
use super::base::{div_ceil, jxl_failure, Status, StatusCode, K_BITS_PER_BYTE};
use super::image::{copy_image_to, fill_image, Image3F, ImageF, Plane};
use super::image_bundle::ImageBundle;
use super::image_metadata::Orientation;
use super::math::{hwy_clamp, hwy_nearest_int};

/// Translation of `FloatToU32()` (one lane).
fn float_to_u32(input: &[f32], out: &mut [u32], num: usize, mul: f32) {
    let one = 1.0f32;
    let scale = mul;
    for x in 0..num {
        // Clamp turns NaN to 'min'.
        let v = hwy_clamp(input[x], 0.0, one);
        let i = hwy_nearest_int(v * scale);
        out[x] = i as u32;
    }
}

// The orientation may not be identity.
// TODO(lode): SIMDify where possible
/// Translation of `UndoOrientation()`.
fn undo_orientation(undo: Orientation, image: &ImageF) -> Result<ImageF, StatusCode> {
    let xsize = image.xsize();
    let ysize = image.ysize();

    let mut out;
    match undo {
        Orientation::FlipHorizontal => {
            out = ImageF::new(xsize, ysize)?;
            for y in 0..ysize {
                let row_in = image.row(y);
                let row_out = out.row_mut(y);
                for x in 0..xsize {
                    row_out[xsize - x - 1] = row_in[x];
                }
            }
        }
        Orientation::Rotate180 => {
            out = ImageF::new(xsize, ysize)?;
            for y in 0..ysize {
                let row_in = image.row(y);
                let row_out = out.row_mut(ysize - y - 1);
                for x in 0..xsize {
                    row_out[xsize - x - 1] = row_in[x];
                }
            }
        }
        Orientation::FlipVertical => {
            out = ImageF::new(xsize, ysize)?;
            for y in 0..ysize {
                let row_in = image.row(y);
                let row_out = out.row_mut(ysize - y - 1);
                row_out[..xsize].copy_from_slice(&row_in[..xsize]);
            }
        }
        Orientation::Transpose => {
            out = ImageF::new(ysize, xsize)?;
            for y in 0..ysize {
                let row_in = image.row(y);
                for x in 0..xsize {
                    out.row_mut(x)[y] = row_in[x];
                }
            }
        }
        Orientation::Rotate90 => {
            out = ImageF::new(ysize, xsize)?;
            for y in 0..ysize {
                let row_in = image.row(y);
                for x in 0..xsize {
                    out.row_mut(x)[ysize - y - 1] = row_in[x];
                }
            }
        }
        Orientation::AntiTranspose => {
            out = ImageF::new(ysize, xsize)?;
            for y in 0..ysize {
                let row_in = image.row(y);
                for x in 0..xsize {
                    out.row_mut(xsize - x - 1)[ysize - y - 1] = row_in[x];
                }
            }
        }
        Orientation::Rotate270 => {
            out = ImageF::new(ysize, xsize)?;
            for y in 0..ysize {
                let row_in = image.row(y);
                for x in 0..xsize {
                    out.row_mut(xsize - x - 1)[y] = row_in[x];
                }
            }
        }
        Orientation::Identity => {
            out = ImageF::empty();
        }
    }
    Ok(out)
}

// Maximum number of channels for the ConvertChannelsToExternal function.
const K_CONVERT_MAX_CHANNELS: usize = 4;

// Converts a list of channels to an interleaved image, applying transformations
// when needed.
// The input channels are given as a (non-const!) array of channel pointers and
// interleaved in that order.
//
// Note: if a pointer in channels[] is nullptr, a 1.0 value will be used
// instead. This is useful for handling when a user requests an alpha channel
// from an image that doesn't have one. The first channel in the list may not
// be nullptr, since it is used to determine the image size.
/// Translation of `ConvertChannelsToExternal()` (buffer output).
#[allow(clippy::too_many_arguments)]
fn convert_channels_to_external(
    channels_in: &[Option<&ImageF>],
    bits_per_sample: usize,
    float_out: bool,
    little_endian: bool,
    stride: usize,
    out_image: &mut [u8],
    undo: Orientation,
) -> Status {
    let num_channels = channels_in.len();
    debug_assert!(num_channels != 0 && num_channels <= K_CONVERT_MAX_CHANNELS);
    let valid = if float_out {
        bits_per_sample == 16 || bits_per_sample == 32
    } else {
        bits_per_sample > 0 && bits_per_sample <= 16
    };
    if !valid {
        // JXL_CHECK
        return Err(StatusCode::GenericError);
    }

    let bytes_per_channel = div_ceil(bits_per_sample, K_BITS_PER_BYTE);
    let bytes_per_pixel = num_channels * bytes_per_channel;

    // Channels used to store the transformed original channels if needed.
    let mut temp_channels: Vec<ImageF> = Vec::new();
    if undo != Orientation::Identity {
        for c in channels_in.iter() {
            temp_channels.push(match c {
                Some(ch) => undo_orientation(undo, ch)?,
                None => ImageF::empty(),
            });
        }
    }
    let channels: Vec<Option<&ImageF>> = if undo != Orientation::Identity {
        channels_in
            .iter()
            .enumerate()
            .map(|(c, ch)| ch.map(|_| &temp_channels[c]))
            .collect()
    } else {
        channels_in.to_vec()
    };

    // First channel may not be nullptr.
    let Some(first) = channels[0] else {
        return Err(StatusCode::GenericError);
    };
    let xsize = first.xsize();
    let ysize = first.ysize();
    if stride < bytes_per_pixel * xsize {
        return jxl_failure!("stride is smaller than scanline width in bytes");
    }
    if out_image.len() < (ysize.wrapping_sub(1)).wrapping_mul(stride).wrapping_add(bytes_per_pixel * xsize) {
        return jxl_failure!("out_size is too small to store image");
    }

    // Handle the case where a channel is nullptr by creating a single row with
    // ones to use instead.
    let mut ones = ImageF::empty();
    if channels.iter().any(|c| c.is_none()) {
        ones = ImageF::new(xsize, 1)?;
        fill_image(1.0f32, &mut ones);
    }

    if float_out {
        if bits_per_sample == 32 {
            for y in 0..ysize {
                let row_out = &mut out_image[stride * y..];
                for x in 0..xsize {
                    for (c, ch) in channels.iter().enumerate() {
                        let v = match ch {
                            Some(p) => p.row(y)[x],
                            None => ones.row(0)[x],
                        };
                        let b = if little_endian {
                            v.to_bits().to_le_bytes()
                        } else {
                            v.to_bits().to_be_bytes()
                        };
                        let o = (num_channels * x + c) * 4;
                        row_out[o..o + 4].copy_from_slice(&b);
                    }
                }
            }
        } else {
            // (16-bit float output is not translated.)
            return jxl_failure!("float other than 16-bit and 32-bit not supported");
        }
    } else {
        // Multiplier to convert from floating point 0-1 range to the integer
        // range.
        let mul = ((1u64 << bits_per_sample) - 1) as f32;
        let mut u32_cache: Plane<u32> = Plane::new(xsize, num_channels)?;
        for y in 0..ysize {
            for (c, ch) in channels.iter().enumerate() {
                let row_in = match ch {
                    Some(p) => p.row(y),
                    None => ones.row(0),
                };
                float_to_u32(row_in, u32_cache.row_mut(c), xsize, mul);
            }
            let row_out = &mut out_image[stride * y..];
            for x in 0..xsize {
                for c in 0..num_channels {
                    let value = u32_cache.row(c)[x];
                    if bits_per_sample <= 8 {
                        // Store8
                        row_out[num_channels * x + c] = (value & 0xff) as u8;
                    } else {
                        let o = (num_channels * x + c) * 2;
                        let v = value as u16;
                        let b = if little_endian { v.to_le_bytes() } else { v.to_be_bytes() };
                        row_out[o..o + 2].copy_from_slice(&b);
                    }
                }
            }
        }
    }
    Ok(())
}

/// Converts ib to interleaved void* pixel buffer with the given format.
/// Translation of `ConvertToExternal()` (for an image bundle, to a
/// buffer).
#[allow(clippy::too_many_arguments)]
pub(crate) fn convert_to_external(
    ib: &ImageBundle,
    bits_per_sample: usize,
    float_out: bool,
    num_channels: usize,
    little_endian: bool,
    stride: usize,
    out_image: &mut [u8],
    undo: Orientation,
    unpremul_alpha: bool,
) -> Status {
    let want_alpha = num_channels == 2 || num_channels == 4;
    let color_channels = if num_channels <= 2 { 1 } else { 3 };

    let mut color: &Image3F = ib.color();
    // Undo premultiplied alpha.
    let mut unpremul = Image3F::empty();
    let alpha_is_premultiplied = ib
        .metadata()
        .and_then(|m| m.find(super::image_metadata::ExtraChannel::Alpha))
        .is_some_and(|e| e.alpha_associated);
    if alpha_is_premultiplied && ib.has_alpha() && unpremul_alpha {
        let Some(alpha) = ib.alpha() else {
            return Err(StatusCode::GenericError);
        };
        unpremul = Image3F::new(color.xsize(), color.ysize())?;
        for c in 0..3 {
            copy_image_to(color.plane(c), unpremul.plane_mut(c));
        }
        let xs = unpremul.xsize();
        for y in 0..unpremul.ysize() {
            let [r, g, b] = unpremul.planes_mut();
            unpremultiply_alpha(r.row_mut(y), g.row_mut(y), b.row_mut(y), alpha.row(y), xs);
        }
        color = &unpremul;
    }

    let mut channels: Vec<Option<&ImageF>> = Vec::with_capacity(K_CONVERT_MAX_CHANNELS);
    for c in 0..color_channels {
        channels.push(Some(color.plane(c)));
    }
    if want_alpha {
        if ib.has_alpha() {
            match ib.alpha() {
                Some(a) => channels.push(Some(a)),
                None => return Err(StatusCode::GenericError), // JXL_ASSERT
            }
        } else {
            channels.push(None);
        }
    }
    if num_channels != channels.len() {
        return Err(StatusCode::GenericError);
    }

    convert_channels_to_external(
        &channels,
        bits_per_sample,
        float_out,
        little_endian,
        stride,
        out_image,
        undo,
    )
}
