// Rust translation of the PSD loader of stb_image.h v2.30 (Sean Barrett and
// contributors, http://nothings.org/stb; "Photoshop PSD loader -- PD by
// Thatcher Ulrich, integration by Nicolas Schulz, tweaked by STB"), from
// the copy PlutoVG bundles (source/plutovg-stb-image.h in SDL_ttf's
// external/plutovg), which SDL's own copy compiles out (`STBI_ONLY_PNG`,
// `STBI_ONLY_JPEG`).
// stb_image is available under the MIT License or in the public domain
// (see LICENSE.txt).
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Photoshop PSD: the composited image of 8- and 16-bit RGB documents,
//! raw or RLE.

use super::stb_image::{
    convert_format16, convert_format8, err, mad3sizes_valid, Context, Image, Pixels,
    STBI_MAX_DIMENSIONS,
};
use crate::error::Result;

/// Translation of `stbi__psd_test()`.
pub(crate) fn psd_test(s: &mut Context<'_>) -> bool {
    let r = s.get32be() == 0x38425053;
    s.rewind();
    r
}

/// Translation of `stbi__psd_decode_rle()`: one channel of `pixel_count`
/// pixels into every fourth byte of `p`.
fn psd_decode_rle(s: &mut Context<'_>, p: &mut [u8], pixel_count: i32) -> bool {
    let mut count = 0;
    let mut o = 0usize;
    loop {
        let nleft = pixel_count - count;
        if nleft <= 0 {
            break;
        }
        let mut len = s.get8() as i32;
        if len == 128 {
            // No-op.
        } else if len < 128 {
            // Copy next len+1 bytes literally.
            len += 1;
            if len > nleft {
                return false; // corrupt data
            }
            count += len;
            while len != 0 {
                p[o] = s.get8();
                o += 4;
                len -= 1;
            }
        } else if len > 128 {
            // Next -len+1 bytes in the dest are replicated from next source byte.
            // (Interpret len as a negative 8-bit int.)
            len = 257 - len;
            if len > nleft {
                return false; // corrupt data
            }
            let val = s.get8();
            count += len;
            while len != 0 {
                p[o] = val;
                o += 4;
                len -= 1;
            }
        }
    }

    true
}

/// The C's conversion of a float to an 8- or 16-bit integer, as x86-64
/// compilers emit it (`cvttss2si` to 32 bits, of which the low bits are
/// kept; its result for values out of range is 0x80000000). Out of range
/// values are undefined in C.
#[inline]
fn f2i(v: f32) -> i32 {
    if (-2147483648.0..2147483648.0).contains(&v) {
        v as i32
    } else {
        i32::MIN
    }
}

/// Translation of `stbi__psd_load()`.
pub(crate) fn psd_load(s: &mut Context<'_>, req_comp: i32, bpc: i32) -> Result<Image<Pixels>> {
    // Check identifier
    if s.get32be() != 0x38425053 {
        // "8BPS"
        return err("Corrupt PSD image");
    }

    // Check file type version.
    if s.get16be() != 1 {
        return err("Unsupported version of PSD image");
    }

    // Skip 6 reserved bytes.
    s.skip(6);

    // Read the number of channels (R, G, B, A, etc).
    let channel_count = s.get16be();
    if !(0..=16).contains(&channel_count) {
        return err("Unsupported number of channels in PSD image");
    }

    // Read the rows and columns of the image.
    let h = s.get32be() as i32;
    let w = s.get32be() as i32;

    if h > STBI_MAX_DIMENSIONS as i32 {
        return err("Very large image (corrupt?)");
    }
    if w > STBI_MAX_DIMENSIONS as i32 {
        return err("Very large image (corrupt?)");
    }

    // Make sure the depth is 8 bits.
    let bitdepth = s.get16be();
    if bitdepth != 8 && bitdepth != 16 {
        return err("PSD bit depth is not 8 or 16 bit");
    }

    // Make sure the color mode is RGB.
    // Valid options are:
    //   0: Bitmap
    //   1: Grayscale
    //   2: Indexed color
    //   3: RGB color
    //   4: CMYK color
    //   7: Multichannel
    //   8: Duotone
    //   9: Lab color
    if s.get16be() != 3 {
        return err("PSD is not in RGB color format");
    }

    // Skip the Mode Data.  (It's the palette for indexed color; other info for other modes.)
    let n = s.get32be() as i32;
    s.skip(n);

    // Skip the image resources.  (resolution, pen tool paths, etc)
    let n = s.get32be() as i32;
    s.skip(n);

    // Skip the reserved data.
    let n = s.get32be() as i32;
    s.skip(n);

    // Find out if the data is compressed.
    // Known values:
    //   0: no compression
    //   1: RLE compressed
    let compression = s.get16be();
    if compression > 1 {
        return err("PSD has an unknown compression format");
    }

    // Check size
    if !mad3sizes_valid(4, w, h, 0) {
        return err("Corrupt PSD");
    }

    // Create the destination image.

    let bits16 = compression == 0 && bitdepth == 16 && bpc == 16;
    let pixel_count = w * h;
    // (malloc'd uninitialized in C; every byte is written below)
    let mut out8 = Vec::new();
    let mut out16 = Vec::new();
    if bits16 {
        if !mad3sizes_valid(8, w, h, 0) {
            return err("Out of memory");
        }
        out16 = vec![0u16; 4 * pixel_count as usize];
    } else {
        out8 = vec![0u8; 4 * pixel_count as usize];
    }

    // Initialize the data to zero.
    //memset( out, 0, pixelCount * 4 );

    // Finally, the image data.
    if compression != 0 {
        // RLE as used by .PSD and .TIFF
        // Loop until you get the number of unpacked bytes you are expecting:
        //     Read the next source byte into n.
        //     If n is between 0 and 127 inclusive, copy the next n+1 bytes literally.
        //     Else if n is between -127 and -1 inclusive, copy the next byte -n+1 times.
        //     Else if n is 128, noop.
        // Endloop

        // The RLE-compressed data is preceded by a 2-byte data count for each row in the data,
        // which we're going to just skip.
        s.skip(h * channel_count * 2);

        // Read the RLE data by channel.
        for channel in 0..4 {
            // (empty for an image of no pixels)
            let p = out8.get_mut(channel as usize..).unwrap_or_default();
            if channel >= channel_count {
                // Fill this channel with default data.
                for v in p.iter_mut().step_by(4) {
                    *v = if channel == 3 { 255 } else { 0 };
                }
            } else {
                // Read the RLE data.
                if !psd_decode_rle(s, p, pixel_count) {
                    return err("bad RLE data");
                }
            }
        }
    } else {
        // We're at the raw image data.  It's each channel in order (Red, Green, Blue, Alpha, ...)
        // where each channel consists of an 8-bit (or 16-bit) value for each pixel in the image.

        // Read the data by channel.
        for channel in 0..4 {
            if channel >= channel_count {
                // Fill this channel with default data.
                if bitdepth == 16 && bpc == 16 {
                    let val: u16 = if channel == 3 { 65535 } else { 0 };
                    for q in out16
                        .get_mut(channel as usize..)
                        .unwrap_or_default()
                        .iter_mut()
                        .step_by(4)
                    {
                        *q = val;
                    }
                } else {
                    let val: u8 = if channel == 3 { 255 } else { 0 };
                    for p in out8
                        .get_mut(channel as usize..)
                        .unwrap_or_default()
                        .iter_mut()
                        .step_by(4)
                    {
                        *p = val;
                    }
                }
            } else if bits16 {
                // output bpc
                for q in out16
                    .get_mut(channel as usize..)
                    .unwrap_or_default()
                    .iter_mut()
                    .step_by(4)
                {
                    *q = s.get16be() as u16;
                }
            } else if bitdepth == 16 {
                // input bpc
                for p in out8
                    .get_mut(channel as usize..)
                    .unwrap_or_default()
                    .iter_mut()
                    .step_by(4)
                {
                    *p = (s.get16be() >> 8) as u8;
                }
            } else {
                for p in out8
                    .get_mut(channel as usize..)
                    .unwrap_or_default()
                    .iter_mut()
                    .step_by(4)
                {
                    *p = s.get8();
                }
            }
        }
    }

    // remove weird white matte from PSD
    if channel_count >= 4 {
        if bits16 {
            for pixel in out16.chunks_exact_mut(4) {
                if pixel[3] != 0 && pixel[3] != 65535 {
                    let a = pixel[3] as f32 / 65535.0f32;
                    let ra = 1.0f32 / a;
                    let inv_a = 65535.0f32 * (1.0 - ra);
                    pixel[0] = f2i(pixel[0] as f32 * ra + inv_a) as u16;
                    pixel[1] = f2i(pixel[1] as f32 * ra + inv_a) as u16;
                    pixel[2] = f2i(pixel[2] as f32 * ra + inv_a) as u16;
                }
            }
        } else {
            for pixel in out8.chunks_exact_mut(4) {
                if pixel[3] != 0 && pixel[3] != 255 {
                    let a = pixel[3] as f32 / 255.0f32;
                    let ra = 1.0f32 / a;
                    let inv_a = 255.0f32 * (1.0 - ra);
                    pixel[0] = f2i(pixel[0] as f32 * ra + inv_a) as u8;
                    pixel[1] = f2i(pixel[1] as f32 * ra + inv_a) as u8;
                    pixel[2] = f2i(pixel[2] as f32 * ra + inv_a) as u8;
                }
            }
        }
    }

    // convert to desired output format
    let data = if bits16 {
        Pixels::Bits16(if req_comp != 0 && req_comp != 4 {
            convert_format16(out16, 4, req_comp, w as u32, h as u32)?
        } else {
            out16
        })
    } else {
        Pixels::Bits8(if req_comp != 0 && req_comp != 4 {
            convert_format8(out8, 4, req_comp, w as u32, h as u32)?
        } else {
            out8
        })
    };

    Ok(Image {
        x: w,
        y: h,
        comp: 4,
        data,
    })
}
