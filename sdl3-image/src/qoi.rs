// Rust translation of src/IMG_qoi.c and src/qoi.h from SDL_image.
// IMG_qoi.c: Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// qoi.h: QOI - The "Quite OK Image" format for fast, lossless image
// compression, Dominic Szablewski - https://phoboslab.org
// Copyright(c) 2021 Dominic Szablewski, under the MIT License (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! This file use QOI library:
//! <https://github.com/phoboslab/qoi>

use sdl3::error::{Error, Result};
use sdl3::io::{IoStream, IoWhence};
use sdl3::video::{PixelFormat, Surface};

use crate::util::read_ok;

/* See if an image is contained in a data source */

/// Whether `src` holds a QOI image; the stream position is unchanged.
/// Translation of `IMG_isQOI()`.
pub fn is_qoi(src: &mut IoStream<'_>) -> bool {
    let start = src.tell().unwrap_or(-1);
    let mut is_qoi = false;
    let mut magic = [0u8; 4];
    if read_ok(src, &mut magic) && &magic == b"qoif" {
        is_qoi = true;
    }
    let _ = src.seek(start, IoWhence::Set);
    is_qoi
}

/// Load a QOI image (the rest of the stream) as RGBA32.
/// Translation of `IMG_LoadQOI_IO()`.
pub fn load_qoi_io(src: &mut IoStream<'_>) -> Result<Surface<'static>> {
    let data = src.load_all()?;
    if data.len() > i32::MAX as usize {
        return Err(Error::new("QOI image is too big."));
    }

    let Some((pixel_data, image_info)) = codec::decode(&data, data.len() as i32, 4) else {
        return Err(Error::new("Couldn't parse QOI image"));
    };
    /* pixel_data is in R,G,B,A order regardless of endianness */
    drop(data);

    let Ok(surface) = Surface::from_vec(
        image_info.width as i32,
        image_info.height as i32,
        PixelFormat::RGBA32,
        pixel_data,
        (image_info.width * 4) as i32,
    ) else {
        return Err(Error::new("Couldn't create SDL_Surface"));
    };

    /* Let SDL manage the memory now */

    Ok(surface)
}

/// The QOI codec. Translation of `qoi.h` (with `QOI_NO_STDIO`).
///
/// -- About
///
/// QOI encodes and decodes images in a lossless format. Compared to stb_image and
/// stb_image_write QOI offers 20x-50x faster encoding, 3x-4x faster decoding and
/// 20% better compression.
///
/// -- Data Format
///
/// A QOI file has a 14 byte header, followed by any number of data "chunks" and an
/// 8-byte end marker.
///
/// ```text
/// struct qoi_header_t {
///     char     magic[4];   // magic bytes "qoif"
///     uint32_t width;      // image width in pixels (BE)
///     uint32_t height;     // image height in pixels (BE)
///     uint8_t  channels;   // 3 = RGB, 4 = RGBA
///     uint8_t  colorspace; // 0 = sRGB with linear alpha, 1 = all channels linear
/// };
/// ```
///
/// Images are encoded row by row, left to right, top to bottom. The decoder and
/// encoder start with {r: 0, g: 0, b: 0, a: 255} as the previous pixel value. An
/// image is complete when all pixels specified by width * height have been covered.
///
/// Pixels are encoded as
///  - a run of the previous pixel
///  - an index into an array of previously seen pixels
///  - a difference to the previous pixel value in r,g,b
///  - full r,g,b or r,g,b,a values
///
/// The color channels are assumed to not be premultiplied with the alpha channel
/// ("un-premultiplied alpha").
///
/// A running array\[64\] (zero-initialized) of previously seen pixel values is
/// maintained by the encoder and decoder. Each pixel that is seen by the encoder
/// and decoder is put into this array at the position formed by a hash function of
/// the color value. In the encoder, if the pixel value at the index matches the
/// current pixel, this index position is written to the stream as QOI_OP_INDEX.
/// The hash function for the index is:
///
/// ```text
///     index_position = (r * 3 + g * 5 + b * 7 + a * 11) % 64
/// ```
///
/// Each chunk starts with a 2- or 8-bit tag, followed by a number of data bits. The
/// bit length of chunks is divisible by 8 - i.e. all chunks are byte aligned. All
/// values encoded in these data bits have the most significant bit on the left.
///
/// The 8-bit tags have precedence over the 2-bit tags. A decoder must check for the
/// presence of an 8-bit tag first.
///
/// The byte stream's end is marked with 7 0x00 bytes followed a single 0x01 byte.
///
/// The possible chunks are:
///
/// ```text
/// .- QOI_OP_INDEX ----------.
/// |         Byte[0]         |
/// |  7  6  5  4  3  2  1  0 |
/// |-------+-----------------|
/// |  0  0 |     index       |
/// `-------------------------`
/// 2-bit tag b00
/// 6-bit index into the color index array: 0..63
///
/// A valid encoder must not issue 2 or more consecutive QOI_OP_INDEX chunks to the
/// same index. QOI_OP_RUN should be used instead.
///
/// .- QOI_OP_DIFF -----------.
/// |         Byte[0]         |
/// |  7  6  5  4  3  2  1  0 |
/// |-------+-----+-----+-----|
/// |  0  1 |  dr |  dg |  db |
/// `-------------------------`
/// 2-bit tag b01
/// 2-bit   red channel difference from the previous pixel between -2..1
/// 2-bit green channel difference from the previous pixel between -2..1
/// 2-bit  blue channel difference from the previous pixel between -2..1
///
/// The difference to the current channel values are using a wraparound operation,
/// so "1 - 2" will result in 255, while "255 + 1" will result in 0.
///
/// Values are stored as unsigned integers with a bias of 2. E.g. -2 is stored as
/// 0 (b00). 1 is stored as 3 (b11).
///
/// The alpha value remains unchanged from the previous pixel.
///
/// .- QOI_OP_LUMA -------------------------------------.
/// |         Byte[0]         |         Byte[1]         |
/// |  7  6  5  4  3  2  1  0 |  7  6  5  4  3  2  1  0 |
/// |-------+-----------------+-------------+-----------|
/// |  1  0 |  green diff     |   dr - dg   |  db - dg  |
/// `---------------------------------------------------`
/// 2-bit tag b10
/// 6-bit green channel difference from the previous pixel -32..31
/// 4-bit   red channel difference minus green channel difference -8..7
/// 4-bit  blue channel difference minus green channel difference -8..7
///
/// The green channel is used to indicate the general direction of change and is
/// encoded in 6 bits. The red and blue channels (dr and db) base their diffs off
/// of the green channel difference and are encoded in 4 bits. I.e.:
///     dr_dg = (cur_px.r - prev_px.r) - (cur_px.g - prev_px.g)
///     db_dg = (cur_px.b - prev_px.b) - (cur_px.g - prev_px.g)
///
/// The difference to the current channel values are using a wraparound operation,
/// so "10 - 13" will result in 253, while "250 + 7" will result in 1.
///
/// Values are stored as unsigned integers with a bias of 32 for the green channel
/// and a bias of 8 for the red and blue channel.
///
/// The alpha value remains unchanged from the previous pixel.
///
/// .- QOI_OP_RUN ------------.
/// |         Byte[0]         |
/// |  7  6  5  4  3  2  1  0 |
/// |-------+-----------------|
/// |  1  1 |       run       |
/// `-------------------------`
/// 2-bit tag b11
/// 6-bit run-length repeating the previous pixel: 1..62
///
/// The run-length is stored with a bias of -1. Note that the run-lengths 63 and 64
/// (b111110 and b111111) are illegal as they are occupied by the QOI_OP_RGB and
/// QOI_OP_RGBA tags.
///
/// .- QOI_OP_RGB ------------------------------------------.
/// |         Byte[0]         | Byte[1] | Byte[2] | Byte[3] |
/// |  7  6  5  4  3  2  1  0 | 7 .. 0  | 7 .. 0  | 7 .. 0  |
/// |-------------------------+---------+---------+---------|
/// |  1  1  1  1  1  1  1  0 |   red   |  green  |  blue   |
/// `-------------------------------------------------------`
/// 8-bit tag b11111110
/// 8-bit   red channel value
/// 8-bit green channel value
/// 8-bit  blue channel value
///
/// The alpha value remains unchanged from the previous pixel.
///
/// .- QOI_OP_RGBA ---------------------------------------------------.
/// |         Byte[0]         | Byte[1] | Byte[2] | Byte[3] | Byte[4] |
/// |  7  6  5  4  3  2  1  0 | 7 .. 0  | 7 .. 0  | 7 .. 0  | 7 .. 0  |
/// |-------------------------+---------+---------+---------+---------|
/// |  1  1  1  1  1  1  1  1 |   red   |  green  |  blue   |  alpha  |
/// `-----------------------------------------------------------------`
/// 8-bit tag b11111111
/// 8-bit   red channel value
/// 8-bit green channel value
/// 8-bit  blue channel value
/// 8-bit alpha channel value
/// ```
pub(crate) mod codec {
    /* A pointer to a qoi_desc struct has to be supplied to all of qoi's functions.
    It describes either the input format (for qoi_write and qoi_encode), or is
    filled with the description read from the file header (for qoi_read and
    qoi_decode).

    The colorspace in this qoi_desc is an enum where
        0 = sRGB, i.e. gamma scaled RGB channels and a linear alpha channel
        1 = all channels are linear
    You may use the constants QOI_SRGB or QOI_LINEAR. The colorspace is purely
    informative. It will be saved to the file header, but does not affect
    how chunks are en-/decoded. */

    #[allow(dead_code)]
    pub(crate) const QOI_SRGB: u8 = 0;
    #[allow(dead_code)]
    pub(crate) const QOI_LINEAR: u8 = 1;

    /// Translation of `qoi_desc`.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub(crate) struct Desc {
        pub(crate) width: u32,
        pub(crate) height: u32,
        pub(crate) channels: u8,
        pub(crate) colorspace: u8,
    }

    const QOI_OP_INDEX: u8 = 0x00; /* 00xxxxxx */
    const QOI_OP_DIFF: u8 = 0x40; /* 01xxxxxx */
    const QOI_OP_LUMA: u8 = 0x80; /* 10xxxxxx */
    const QOI_OP_RUN: u8 = 0xc0; /* 11xxxxxx */
    const QOI_OP_RGB: u8 = 0xfe; /* 11111110 */
    const QOI_OP_RGBA: u8 = 0xff; /* 11111111 */

    const QOI_MASK_2: u8 = 0xc0; /* 11000000 */

    /// Translation of `QOI_COLOR_HASH()`.
    fn color_hash(c: Rgba) -> usize {
        c.r as usize * 3 + c.g as usize * 5 + c.b as usize * 7 + c.a as usize * 11
    }

    const QOI_MAGIC: u32 =
        (b'q' as u32) << 24 | (b'o' as u32) << 16 | (b'i' as u32) << 8 | (b'f' as u32);
    const QOI_HEADER_SIZE: usize = 14;

    /* 2GB is the max file size that this implementation can safely handle. We guard
    against anything larger than that, assuming the worst case with 5 bytes per
    pixel, rounded down to a nice clean value. 400 million pixels ought to be
    enough for anybody. */
    const QOI_PIXELS_MAX: u32 = 400000000;

    /// Translation of `qoi_rgba_t` (the union's `v` is the comparison).
    #[derive(Clone, Copy, Default, PartialEq, Eq)]
    struct Rgba {
        r: u8,
        g: u8,
        b: u8,
        a: u8,
    }

    static QOI_PADDING: [u8; 8] = [0, 0, 0, 0, 0, 0, 0, 1];

    /// Translation of `qoi_write_32()`.
    fn write_32(bytes: &mut Vec<u8>, v: u32) {
        bytes.extend_from_slice(&v.to_be_bytes());
    }

    /// Translation of `qoi_read_32()`.
    fn read_32(bytes: &[u8], p: &mut usize) -> u32 {
        let a = bytes[*p] as u32;
        let b = bytes[*p + 1] as u32;
        let c = bytes[*p + 2] as u32;
        let d = bytes[*p + 3] as u32;
        *p += 4;
        a << 24 | b << 16 | c << 8 | d
    }

    /// Encode raw RGB or RGBA pixels into a QOI image in memory.
    ///
    /// The function either returns NULL on failure (invalid parameters or malloc
    /// failed) or a pointer to the encoded data on success. On success the out_len
    /// is set to the size in bytes of the encoded data.
    ///
    /// Translation of `qoi_encode()` (`None` for NULL, the length is the
    /// `Vec`'s).
    #[allow(dead_code)] // SDL_image has no QOI saver; used by the tests
    pub(crate) fn encode(data: &[u8], desc: &Desc) -> Option<Vec<u8>> {
        if desc.width == 0
            || desc.height == 0
            || desc.channels < 3
            || desc.channels > 4
            || desc.colorspace > 1
            || desc.height >= QOI_PIXELS_MAX / desc.width
        {
            return None;
        }

        let max_size = desc.width as usize * desc.height as usize * (desc.channels as usize + 1)
            + QOI_HEADER_SIZE
            + QOI_PADDING.len();

        let mut bytes = Vec::with_capacity(max_size);

        write_32(&mut bytes, QOI_MAGIC);
        write_32(&mut bytes, desc.width);
        write_32(&mut bytes, desc.height);
        bytes.push(desc.channels);
        bytes.push(desc.colorspace);

        let pixels = data;

        let mut index = [Rgba::default(); 64];

        let mut run = 0u8;
        let mut px_prev = Rgba {
            r: 0,
            g: 0,
            b: 0,
            a: 255,
        };
        let mut px = px_prev;

        let px_len = desc.width as usize * desc.height as usize * desc.channels as usize;
        let px_end = px_len - desc.channels as usize;
        let channels = desc.channels as usize;

        let mut px_pos = 0;
        while px_pos < px_len {
            px.r = pixels[px_pos];
            px.g = pixels[px_pos + 1];
            px.b = pixels[px_pos + 2];

            if channels == 4 {
                px.a = pixels[px_pos + 3];
            }

            if px == px_prev {
                run += 1;
                if run == 62 || px_pos == px_end {
                    bytes.push(QOI_OP_RUN | (run - 1));
                    run = 0;
                }
            } else {
                if run > 0 {
                    bytes.push(QOI_OP_RUN | (run - 1));
                    run = 0;
                }

                let index_pos = color_hash(px) % 64;

                if index[index_pos] == px {
                    bytes.push(QOI_OP_INDEX | index_pos as u8);
                } else {
                    index[index_pos] = px;

                    if px.a == px_prev.a {
                        let vr = px.r.wrapping_sub(px_prev.r) as i8;
                        let vg = px.g.wrapping_sub(px_prev.g) as i8;
                        let vb = px.b.wrapping_sub(px_prev.b) as i8;

                        let vg_r = vr.wrapping_sub(vg);
                        let vg_b = vb.wrapping_sub(vg);

                        if vr > -3 && vr < 2 && vg > -3 && vg < 2 && vb > -3 && vb < 2 {
                            bytes.push(
                                QOI_OP_DIFF
                                    | ((vr + 2) as u8) << 4
                                    | ((vg + 2) as u8) << 2
                                    | (vb + 2) as u8,
                            );
                        } else if vg_r > -9
                            && vg_r < 8
                            && vg > -33
                            && vg < 32
                            && vg_b > -9
                            && vg_b < 8
                        {
                            bytes.push(QOI_OP_LUMA | (vg + 32) as u8);
                            bytes.push(((vg_r + 8) as u8) << 4 | (vg_b + 8) as u8);
                        } else {
                            bytes.push(QOI_OP_RGB);
                            bytes.push(px.r);
                            bytes.push(px.g);
                            bytes.push(px.b);
                        }
                    } else {
                        bytes.push(QOI_OP_RGBA);
                        bytes.push(px.r);
                        bytes.push(px.g);
                        bytes.push(px.b);
                        bytes.push(px.a);
                    }
                }
            }
            px_prev = px;
            px_pos += channels;
        }

        bytes.extend_from_slice(&QOI_PADDING);

        Some(bytes)
    }

    /// Decode a QOI image from memory.
    ///
    /// The function either returns NULL on failure (invalid parameters or malloc
    /// failed) or a pointer to the decoded pixels. On success, the qoi_desc struct
    /// is filled with the description from the file header.
    ///
    /// Translation of `qoi_decode()` (`None` for NULL; the pixels and the
    /// description are returned together). If channels is 0, the number of
    /// channels from the file header is used.
    pub(crate) fn decode(data: &[u8], size: i32, mut channels: i32) -> Option<(Vec<u8>, Desc)> {
        if (channels != 0 && channels != 3 && channels != 4)
            || size < (QOI_HEADER_SIZE + QOI_PADDING.len()) as i32
        {
            return None;
        }
        let bytes = &data[..size as usize];

        let mut p = 0usize;
        let header_magic = read_32(bytes, &mut p);
        let mut desc = Desc {
            width: read_32(bytes, &mut p),
            height: read_32(bytes, &mut p),
            ..Desc::default()
        };
        desc.channels = bytes[p];
        p += 1;
        desc.colorspace = bytes[p];
        p += 1;

        if desc.width == 0
            || desc.height == 0
            || desc.channels < 3
            || desc.channels > 4
            || desc.colorspace > 1
            || header_magic != QOI_MAGIC
            || desc.height >= QOI_PIXELS_MAX / desc.width
        {
            return None;
        }

        if channels == 0 {
            channels = desc.channels as i32;
        }

        let channels = channels as usize;
        let px_len = desc.width as usize * desc.height as usize * channels;
        let mut pixels = vec![0u8; px_len];

        let mut index = [Rgba::default(); 64];
        let mut px = Rgba {
            r: 0,
            g: 0,
            b: 0,
            a: 255,
        };

        let mut run = 0;
        let chunks_len = size as usize - QOI_PADDING.len();
        let mut px_pos = 0;
        while px_pos < px_len {
            if run > 0 {
                run -= 1;
            } else if p < chunks_len {
                let b1 = bytes[p];
                p += 1;

                if b1 == QOI_OP_RGB {
                    px.r = bytes[p];
                    px.g = bytes[p + 1];
                    px.b = bytes[p + 2];
                    p += 3;
                } else if b1 == QOI_OP_RGBA {
                    px.r = bytes[p];
                    px.g = bytes[p + 1];
                    px.b = bytes[p + 2];
                    px.a = bytes[p + 3];
                    p += 4;
                } else if (b1 & QOI_MASK_2) == QOI_OP_INDEX {
                    px = index[b1 as usize];
                } else if (b1 & QOI_MASK_2) == QOI_OP_DIFF {
                    px.r = px.r.wrapping_add(((b1 >> 4) & 0x03).wrapping_sub(2));
                    px.g = px.g.wrapping_add(((b1 >> 2) & 0x03).wrapping_sub(2));
                    px.b = px.b.wrapping_add((b1 & 0x03).wrapping_sub(2));
                } else if (b1 & QOI_MASK_2) == QOI_OP_LUMA {
                    let b2 = bytes[p] as i32;
                    p += 1;
                    let vg = (b1 & 0x3f) as i32 - 32;
                    px.r = (px.r as i32 + vg - 8 + ((b2 >> 4) & 0x0f)) as u8;
                    px.g = (px.g as i32 + vg) as u8;
                    px.b = (px.b as i32 + vg - 8 + (b2 & 0x0f)) as u8;
                } else if (b1 & QOI_MASK_2) == QOI_OP_RUN {
                    run = b1 & 0x3f;
                }

                index[color_hash(px) % 64] = px;
            }

            pixels[px_pos] = px.r;
            pixels[px_pos + 1] = px.g;
            pixels[px_pos + 2] = px.b;

            if channels == 4 {
                pixels[px_pos + 3] = px.a;
            }
            px_pos += channels;
        }

        Some((pixels, desc))
    }
}
