// Rust translation of IMG_isJXL() from src/IMG_jxl.c from SDL_image.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! JPEG XL images: the detector and the loader, over the translation of
//! libjxl's decoder in `jxl/`.

#[allow(dead_code)]
mod ac_strategy;
#[allow(dead_code)]
mod ans_common;
#[allow(dead_code)]
mod base;
#[allow(dead_code)]
mod coeff_order;
#[allow(dead_code)]
mod color_encoding_internal;
#[allow(dead_code)]
mod color_management;
#[allow(dead_code)]
mod dec_ans;
#[allow(dead_code)]
mod dec_bit_reader;
#[allow(dead_code)]
mod dec_context_map;
#[allow(dead_code)]
mod dec_huffman;
#[allow(dead_code)]
mod fields;
#[allow(dead_code)]
mod frame_header;
#[allow(dead_code)]
mod headers;
#[allow(dead_code)]
mod huffman_table;
#[allow(dead_code)]
mod image;
#[allow(dead_code)]
mod image_metadata;
#[allow(dead_code)]
mod loop_filter;
#[allow(dead_code)]
mod math;
#[allow(dead_code)]
mod modular;
#[allow(dead_code)]
mod opsin_params;
#[allow(dead_code)]
mod quantizer;
#[allow(dead_code)]
mod toc;
#[allow(dead_code)]
mod transfer_functions;

use sdl3::io::{IoStream, IoWhence};

use crate::util::read_ok;

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
