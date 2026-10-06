// Rust translation of src/IMG_gif.c from SDL_image.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.
//
//        Some parts of the code have been adapted from XPaint:
// +-------------------------------------------------------------------+
// | Copyright 1990, 1991, 1993 David Koblas.                          |
// | Copyright 1996 Torsten Martinsen.                                 |
// |   Permission to use, copy, modify, and distribute this software   |
// |   and its documentation for any purpose and without fee is hereby |
// |   granted, provided that the above copyright notice appear in all |
// |   copies and that both that copyright notice and this permission  |
// |   notice appear in supporting documentation.  This software is    |
// |   provided "as is" without express or implied warranty.           |
// +-------------------------------------------------------------------+

//! This is a GIF image file loading framework
//!
//! Adapted for use in SDL by Sam Lantinga -- 7/20/98
//! Changes to work with SDL:
//!
//!   Include SDL header file
//!   Use SDL_Surface rather than xpaint Image structure
//!   Define SDL versions of RWSetMsg(), ImageNewCmap() and ImageSetCmap()
//!
//! Has been overwhelmingly modified by Xen (@lordofxen) and Sam Lantinga (@slouken) as of 11/9/2025.
//!
//! The frame decoder of the animation API (`IMG_CreateGIFAnimationDecoder()`
//! and its callbacks), [`is_gif`] and [`load_gif_io`], which reads the
//! first frame through it; and the encoder (`IMG_CreateGIFAnimationEncoder()`,
//! with its LZW compressor and octree color quantizer) with
//! [`save_gif_io`].

// The loops index several arrays at once and the range checks are
// written as upstream's; both kept as written.
#![allow(
    clippy::explicit_counter_loop,
    clippy::manual_range_contains,
    clippy::needless_range_loop,
    clippy::too_many_arguments
)]

use std::path::Path;

use sdl3::error::{Error, Result};
use sdl3::io::{IoStream, IoWhence};
use sdl3::properties::Properties;
use sdl3::video::{share_palette, Palette, PixelFormat, Rect, Surface};

use crate::anim_decoder::{
    AnimationDecoderStatus, DecoderCore, Stream,
    PROP_ANIMATION_DECODER_CREATE_GIF_SINGLE_IMAGE_BOOLEAN, PROP_METADATA_DESCRIPTION_STRING,
    PROP_METADATA_IGNORE_PROPS_BOOLEAN, PROP_METADATA_LOOP_COUNT_NUMBER,
};
use crate::anim_encoder::{AnimationEncoder, EncoderCore};
use crate::img::verify_can_save_surface;
use crate::util::{read_ok, write_error};

/* * * * * */

const GIF_DISPOSE_NA: i32 = 0; /* No disposal specified */
const GIF_DISPOSE_NONE: i32 = 1; /* Do not dispose */
const GIF_DISPOSE_RESTORE_BACKGROUND: i32 = 2; /* Restore to background */
const GIF_DISPOSE_RESTORE_PREVIOUS: i32 = 3; /* Restore to previous */

const MAXCOLORMAPSIZE: usize = 256;

const CM_RED: usize = 0;
const CM_GREEN: usize = 1;
const CM_BLUE: usize = 2;

const MAX_LWZ_BITS: i32 = 12;

const INTERLACE: u8 = 0x40;
const LOCALCOLORMAP: u8 = 0x80;
/// Translation of `BitSet()`.
fn bit_set(byte: u8, bit: u8) -> bool {
    (byte & bit) == bit
}

/// Translation of `LM_to_uint()`.
fn lm_to_uint(a: u8, b: u8) -> i32 {
    ((b as i32) << 8) | a as i32
}

type ColorMap = [[u8; MAXCOLORMAPSIZE]; 3];

/// Translation of the `GifScreen` part of `State_t`.
#[derive(Clone)]
#[allow(dead_code)] // the fields upstream keeps but doesn't read
struct GifScreen {
    width: u32,
    height: u32,
    color_map: ColorMap,
    bit_pixel: u32,
    color_resolution: u32,
    background: u32,
    aspect_ratio: u32,
    gray_scale: i32,
}

/// Translation of the `Gif89` part of `State_t`.
#[derive(Clone, Copy)]
struct Gif89 {
    transparent: i32,
    delay_time: i32,
    input_flag: i32,
    disposal: i32,
}

/// The decoding state. Translation of `State_t`.
#[derive(Clone)]
struct State {
    gif_screen: GifScreen,

    gif89: Gif89,

    buf: [u8; 280],
    curbit: i32,
    lastbit: i32,
    done: bool,
    last_byte: i32,

    fresh: bool,
    code_size: i32,
    set_code_size: i32,
    max_code: i32,
    max_code_size: i32,
    firstcode: i32,
    oldcode: i32,
    clear_code: i32,
    end_code: i32,
    table: [[i32; 1 << MAX_LWZ_BITS]; 2],
    stack: [i32; (1 << MAX_LWZ_BITS) * 2],
    /// An index into `stack` (upstream's `int *sp`).
    sp: usize,

    zero_data_block: bool,
}

impl State {
    /// The state as `SDL_memset(&state, 0, sizeof(state))` leaves it.
    fn zeroed() -> Box<State> {
        Box::new(State {
            gif_screen: GifScreen {
                width: 0,
                height: 0,
                color_map: [[0; MAXCOLORMAPSIZE]; 3],
                bit_pixel: 0,
                color_resolution: 0,
                background: 0,
                aspect_ratio: 0,
                gray_scale: 0,
            },
            gif89: Gif89 {
                transparent: 0,
                delay_time: 0,
                input_flag: 0,
                disposal: 0,
            },
            buf: [0; 280],
            curbit: 0,
            lastbit: 0,
            done: false,
            last_byte: 0,
            fresh: false,
            code_size: 0,
            set_code_size: 0,
            max_code: 0,
            max_code_size: 0,
            firstcode: 0,
            oldcode: 0,
            clear_code: 0,
            end_code: 0,
            table: [[0; 1 << MAX_LWZ_BITS]; 2],
            stack: [0; (1 << MAX_LWZ_BITS) * 2],
            sp: 0,
            zero_data_block: false,
        })
    }
}

/// Read a color map of `number` entries. Translation of `ReadColorMap()`
/// (`Err` for its `1`, with the "bad colormap" message).
fn read_color_map(
    src: &mut IoStream<'_>,
    number: usize,
    buffer: &mut ColorMap,
    gray: &mut i32,
) -> Result<()> {
    let mut rgb = [0u8; 3];
    let mut flag = true;

    for i in 0..number {
        if !read_ok(src, &mut rgb) {
            return Err(Error::new("bad colormap"));
        }
        buffer[CM_RED][i] = rgb[0];
        buffer[CM_GREEN][i] = rgb[1];
        buffer[CM_BLUE][i] = rgb[2];
        flag &= rgb[0] == rgb[1] && rgb[1] == rgb[2];
    }

    // (the PBM/PGM/PPM classification of XPaint is disabled upstream)
    let _ = flag;
    *gray = 0;

    Ok(())
}

/// Read an extension block; the graphic control extension updates the
/// frame state. Translation of `DoExtension()`.
fn do_extension(src: &mut IoStream<'_>, label: u8, state: &mut State) {
    let mut buf = [0u8; 256];

    match label {
        0x01 => { /* Plain Text Extension */ }
        0xff => { /* Application Extension */ }
        0xfe => {
            /* Comment Extension */
            while get_data_block(src, &mut buf, state) > 0 {}
            return;
        }
        0xf9 => {
            /* Graphic Control Extension */
            let _ = get_data_block(src, &mut buf, state);
            state.gif89.disposal = ((buf[0] >> 2) & 0x7) as i32;
            state.gif89.input_flag = ((buf[0] >> 1) & 0x1) as i32;
            state.gif89.delay_time = lm_to_uint(buf[1], buf[2]);
            if (buf[0] & 0x1) != 0 {
                state.gif89.transparent = buf[3] as i32;
            }

            while get_data_block(src, &mut buf, state) > 0 {}
            return;
        }
        _ => {}
    }

    while get_data_block(src, &mut buf, state) > 0 {}
}

/// Read a data sub-block into `buf`: its length, or -1 on a read error.
/// Translation of `GetDataBlock()`.
fn get_data_block(src: &mut IoStream<'_>, buf: &mut [u8], state: &mut State) -> i32 {
    let mut count = [0u8];

    if !read_ok(src, &mut count) {
        /* pm_message("error in getting DataBlock size" ); */
        return -1;
    }
    let count = count[0];
    state.zero_data_block = count == 0;

    if count != 0 && !read_ok(src, &mut buf[..count as usize]) {
        /* pm_message("error in reading DataBlock" ); */
        return -1;
    }
    count as i32
}

/// Read the next `code_size`-bit code, or reset the bit reader for `flag`.
/// Translation of `GetCode()`.
fn get_code(src: &mut IoStream<'_>, code_size: i32, flag: bool, state: &mut State) -> i32 {
    if flag {
        state.curbit = 0;
        state.lastbit = 0;
        state.done = false;
        return 0;
    }
    if (state.curbit + code_size) >= state.lastbit {
        if state.done {
            if state.curbit >= state.lastbit {
                // RWSetMsg("ran off the end of my bits"): the message is
                // replaced or ignored by every caller
            }
            return -1;
        }
        if state.last_byte > 2 {
            state.buf[0] = state.buf[(state.last_byte - 2) as usize];
            state.buf[1] = state.buf[(state.last_byte - 1) as usize];
        }

        // A short read leaves its bytes in the buffer, which the code read
        // below can still reach.
        let mut buf = state.buf;
        let ret = get_data_block(src, &mut buf[2..], state);
        state.buf = buf;
        let count = if ret > 0 {
            ret as u8
        } else {
            state.done = true;
            0
        };

        state.last_byte = 2 + count as i32;
        state.curbit = (state.curbit - state.lastbit) + 16;
        state.lastbit = (2 + count as i32) * 8;
    }
    let mut ret = 0;
    let mut i = state.curbit;
    for j in 0..code_size {
        ret |= (((state.buf[(i / 8) as usize] & (1 << (i % 8))) != 0) as i32) << j;
        i += 1;
    }

    state.curbit += code_size;

    ret
}

/// The LZW decoder: the next pixel value, or a negative value at the end
/// of the data (-2) or on an error (-3, -4, with `error` set). Translation
/// of `LWZReadByte()`.
fn lwz_read_byte(
    src: &mut IoStream<'_>,
    flag: bool,
    input_code_size: i32,
    state: &mut State,
    error: &mut Option<&'static str>,
) -> i32 {
    /* Fixed buffer overflow found by Michael Skladnikiewicz */
    if input_code_size > MAX_LWZ_BITS {
        return -1;
    }

    if flag {
        state.set_code_size = input_code_size;
        state.code_size = state.set_code_size + 1;
        state.clear_code = 1 << state.set_code_size;
        state.end_code = state.clear_code + 1;
        state.max_code_size = 2 * state.clear_code;
        state.max_code = state.clear_code + 2;

        get_code(src, 0, true, state);

        state.fresh = true;

        let mut i = 0;
        while i < state.clear_code {
            state.table[0][i as usize] = 0;
            state.table[1][i as usize] = i;
            i += 1;
        }
        state.table[1][0] = 0;
        while i < (1 << MAX_LWZ_BITS) {
            state.table[0][i as usize] = 0;
            i += 1;
        }

        state.sp = 0;

        return 0;
    } else if state.fresh {
        state.fresh = false;
        loop {
            let code = get_code(src, state.code_size, false, state);
            state.firstcode = code;
            state.oldcode = code;
            if state.firstcode != state.clear_code {
                break;
            }
        }
        return state.firstcode;
    }
    if state.sp > 0 {
        state.sp -= 1;
        return state.stack[state.sp];
    }

    let mut code;
    loop {
        code = get_code(src, state.code_size, false, state);
        if code < 0 {
            break;
        }
        if code == state.clear_code {
            let mut i = 0;
            while i < state.clear_code {
                state.table[0][i as usize] = 0;
                state.table[1][i as usize] = i;
                i += 1;
            }
            while i < (1 << MAX_LWZ_BITS) {
                state.table[0][i as usize] = 0;
                state.table[1][i as usize] = 0;
                i += 1;
            }
            state.code_size = state.set_code_size + 1;
            state.max_code_size = 2 * state.clear_code;
            state.max_code = state.clear_code + 2;
            state.sp = 0;
            let code = get_code(src, state.code_size, false, state);
            state.firstcode = code;
            state.oldcode = code;
            return state.firstcode;
        } else if code == state.end_code {
            let mut buf = [0u8; 260];

            if state.zero_data_block {
                return -2;
            }

            let mut count;
            loop {
                count = get_data_block(src, &mut buf, state);
                if count <= 0 {
                    break;
                }
            }

            if count != 0 {
                /*
                 * pm_message("missing EOD in data stream (common occurrence)");
                 */
            }
            return -2;
        }
        let incode = code;

        if code >= state.max_code {
            state.stack[state.sp] = state.firstcode;
            state.sp += 1;
            code = state.oldcode;
        }
        while code >= state.clear_code {
            /* Guard against buffer overruns */
            if code < 0 || code >= (1 << MAX_LWZ_BITS) {
                *error = Some("invalid LWZ data");
                return -3;
            }
            if state.sp == state.stack.len() {
                *error = Some("invalid LWZ data");
                return -3;
            }
            state.stack[state.sp] = state.table[1][code as usize];
            state.sp += 1;
            if code == state.table[0][code as usize] {
                *error = Some("circular table entry BIG ERROR");
                return -3;
            }
            code = state.table[0][code as usize];
        }

        /* Guard against buffer overruns */
        if code < 0 || code >= (1 << MAX_LWZ_BITS) {
            *error = Some("invalid LWZ data");
            return -4;
        }
        if state.sp == state.stack.len() {
            *error = Some("invalid LWZ data");
            return -4;
        }
        state.firstcode = state.table[1][code as usize];
        state.stack[state.sp] = state.firstcode;
        state.sp += 1;

        code = state.max_code;
        if code < (1 << MAX_LWZ_BITS) {
            state.table[0][code as usize] = state.oldcode;
            state.table[1][code as usize] = state.firstcode;
            state.max_code += 1;
            if (state.max_code >= state.max_code_size)
                && (state.max_code_size < (1 << MAX_LWZ_BITS))
            {
                state.max_code_size *= 2;
                state.code_size += 1;
            }
        }
        state.oldcode = incode;

        if state.sp > 0 {
            state.sp -= 1;
            return state.stack[state.sp];
        }
    }
    code
}

/// Decode a frame's image data into an INDEX8 surface with the color map
/// as its palette (and the transparent index, if any, as its color key).
/// Translation of `ReadImage()`; `ignore` (an "uninteresting picture") is
/// `None`.
#[allow(clippy::too_many_arguments)]
fn read_image(
    src: &mut IoStream<'_>,
    len: i32,
    height: i32,
    mut cmap_size: usize,
    cmap: &ColorMap,
    _gray: i32,
    interlace: bool,
    ignore: bool,
    state: &mut State,
) -> Result<Option<Surface<'static>>> {
    let mut xpos: i32 = 0;
    let mut ypos: i32 = 0;
    let mut pass = 0;
    let mut lwz_error = None;

    /*
     **  Initialize the compression routines
     */
    let mut c = [0u8];
    if !read_ok(src, &mut c) {
        return Err(Error::new("EOF / read error on image data"));
    }
    let c = c[0] as i32;
    if lwz_read_byte(src, true, c, state, &mut lwz_error) < 0 {
        return Err(Error::new("error reading image"));
    }
    /*
     **  If this is an "uninteresting picture" ignore it.
     */
    if ignore {
        while lwz_read_byte(src, false, c, state, &mut lwz_error) >= 0 {}
        return Ok(None);
    }
    let mut image = Surface::new(len, height, PixelFormat::INDEX8)?;

    // (SDL_CreateSurfacePalette() and then `palette->ncolors = cmapSize`)
    if cmap_size > 256 {
        cmap_size = 256;
    }
    let mut palette = Palette::new(cmap_size.max(1))?;
    for (i, color) in palette.colors_mut().iter_mut().enumerate().take(cmap_size) {
        color.r = cmap[CM_RED][i];
        color.g = cmap[CM_GREEN][i];
        color.b = cmap[CM_BLUE][i];
    }
    image.set_palette(Some(share_palette(palette)))?;

    if state.gif89.transparent >= 0 && (state.gif89.transparent as usize) < cmap_size {
        image.set_color_key(Some(state.gif89.transparent as u32))?;
    }

    let pitch = image.pitch();
    let mut pixels = image.pixels_mut();
    loop {
        let v = lwz_read_byte(src, false, c, state, &mut lwz_error);
        if v < 0 {
            break;
        }
        // Note (upstream): a frame of zero width or height has no pixels,
        // and writing the first one crashes there.
        if let Some(pixels) = pixels.as_deref_mut() {
            if let Some(p) = pixels.get_mut((xpos + ypos * pitch) as usize) {
                *p = v as u8;
            }
        }
        xpos += 1;
        if xpos == len {
            xpos = 0;
            if interlace {
                match pass {
                    0 | 1 => ypos += 8,
                    2 => ypos += 4,
                    3 => ypos += 2,
                    _ => {}
                }

                if ypos >= height {
                    pass += 1;
                    match pass {
                        1 => ypos = 4,
                        2 => ypos = 2,
                        3 => ypos = 1,
                        _ => break, /* goto fini */
                    }
                }
            } else {
                ypos += 1;
            }
        }
        if ypos >= height {
            break;
        }
    }

    /* fini: */

    // (an LWZ error ends the frame early; the rest of it stays 0)
    Ok(Some(image))
}

/// Translation of `struct IMG_AnimationDecoderContext` (the GIF one).
#[allow(dead_code)] // the fields upstream keeps but doesn't read
pub(crate) struct GifContext {
    state: Box<State>, /* GIF decoding state */

    buf: [u8; 256],   /* Buffer for reading chunks */
    version: [u8; 4], /* GIF version */

    width: i32,  /* Width of the GIF */
    height: i32, /* Height of the GIF */

    canvas: Option<Surface<'static>>, /* Canvas for compositing frames */
    prev_canvas: Option<Surface<'static>>, /* Previous canvas for DISPOSE_PREVIOUS */

    frame_count: i32,   /* Total number of frames seen */
    current_frame: i32, /* Current frame index */

    single_frame: bool, /* Whether this decoder will return a single frame */

    got_header: bool, /* Whether we've read the GIF header */
    got_eof: bool,    /* Whether we've reached the end of the GIF */

    /* Current frame info */
    current_disposal: i32,  /* Disposal method for current frame */
    current_delay: i32,     /* Delay time for current frame */
    transparent_index: i32, /* Transparent color index for current frame */

    /* Global color map */
    global_colormap: ColorMap,
    global_colormap_size: i32,
    has_global_colormap: bool,
    global_grayscale: bool,

    /* Frame info */
    last_duration: u64, /* The duration of the previous frame */
    last_disposal: i32, /* Disposal method from previous frame */
    restore_area: Rect, /* Area to restore when using DISPOSE_RESTORE_BACKGROUND */

    ignore_props: bool,
}

impl GifContext {
    /// Read the GIF header (once): the screen descriptor, the global color
    /// map, and unless the properties are ignored, the loop count and
    /// comment from the extensions before the first image. Translation of
    /// `IMG_AnimationDecoderGetGIFHeader()`.
    fn get_gif_header(
        &mut self,
        d: &mut DecoderCore<'_, '_>,
        mut comment: Option<&mut Option<String>>,
        mut loop_count: Option<&mut i64>,
    ) -> Result<()> {
        if let Some(comment) = comment.as_deref_mut() {
            *comment = None;
        }

        if let Some(loop_count) = loop_count.as_deref_mut() {
            *loop_count = 1;
        }

        let ctx = self;
        let src = d.src();
        if !ctx.got_header {
            if !read_ok(src, &mut ctx.buf[..6]) {
                return Err(Error::new("Error reading GIF magic number"));
            }

            if &ctx.buf[..3] != b"GIF" {
                return Err(Error::new("Not a GIF file"));
            }

            ctx.version[..3].copy_from_slice(&ctx.buf[3..6]);
            ctx.version[3] = b'\0';

            if &ctx.version[..3] != b"87a" && &ctx.version[..3] != b"89a" {
                return Err(Error::new("Bad version number, not '87a' or '89a'"));
            }

            if !read_ok(src, &mut ctx.buf[..7]) {
                return Err(Error::new("Failed to read screen descriptor"));
            }

            ctx.width = lm_to_uint(ctx.buf[0], ctx.buf[1]);
            ctx.height = lm_to_uint(ctx.buf[2], ctx.buf[3]);
            ctx.state.gif_screen.width = ctx.width as u32;
            ctx.state.gif_screen.height = ctx.height as u32;
            ctx.state.gif_screen.bit_pixel = 2 << (ctx.buf[4] & 0x07);
            ctx.state.gif_screen.color_resolution = (((ctx.buf[4] & 0x70) >> 3) + 1) as u32;
            ctx.state.gif_screen.background = ctx.buf[5] as u32;
            ctx.state.gif_screen.aspect_ratio = ctx.buf[6] as u32;

            ctx.has_global_colormap = bit_set(ctx.buf[4], LOCALCOLORMAP);

            if ctx.has_global_colormap {
                ctx.global_colormap_size = ctx.state.gif_screen.bit_pixel as i32;
                let mut g = if ctx.global_grayscale { 1 } else { 0 };
                if read_color_map(
                    src,
                    ctx.global_colormap_size as usize,
                    &mut ctx.global_colormap,
                    &mut g,
                )
                .is_err()
                {
                    return Err(Error::new("Error reading global colormap"));
                }
                ctx.state.gif_screen.color_map = ctx.global_colormap;
                ctx.state.gif_screen.gray_scale = ctx.global_grayscale as i32;
            }

            if !ctx.ignore_props {
                let stream_pos = src.tell().unwrap_or(-1);
                let mut processing_extensions = true;

                while processing_extensions {
                    let mut block_type = [0u8];
                    if !read_ok(src, &mut block_type) {
                        return Err(Error::new("Error reading GIF block type"));
                    }

                    match block_type[0] {
                        0x21 => {
                            // Extension Introducer
                            let mut extension_label = [0u8];
                            if !read_ok(src, &mut extension_label) {
                                return Err(Error::new("Error reading GIF extension label"));
                            }

                            match extension_label[0] {
                                0xFF => {
                                    // Application Extension
                                    // Read the application block size first (should be 11 for "NETSCAPE2.0")
                                    let mut app_block_size = [0u8];
                                    if !read_ok(src, &mut app_block_size) {
                                        return Err(Error::new(
                                            "Error reading application extension block size",
                                        ));
                                    }
                                    let app_block_size = app_block_size[0] as usize;

                                    let mut app_data = [0u8; 256];
                                    if !read_ok(src, &mut app_data[..app_block_size]) {
                                        return Err(Error::new(
                                            "Error reading GIF application extension block",
                                        ));
                                    }

                                    // Check for NETSCAPE2.0 extension (loop count)
                                    if app_block_size == 11 && &app_data[..11] == b"NETSCAPE2.0" {
                                        let mut sub_block_size = [0u8];
                                        if !read_ok(src, &mut sub_block_size) {
                                            return Err(Error::new(
                                                "Error reading Netscape sub-block size",
                                            ));
                                        }
                                        if sub_block_size[0] == 3 {
                                            let mut sub_block_data = [0u8; 3];
                                            if !read_ok(src, &mut sub_block_data) {
                                                return Err(Error::new(
                                                    "Error reading Netscape sub-block data",
                                                ));
                                            }
                                            if sub_block_data[0] == 0x01 {
                                                if let Some(loop_count) = loop_count.as_deref_mut()
                                                {
                                                    let repeat_count = lm_to_uint(
                                                        sub_block_data[1],
                                                        sub_block_data[2],
                                                    )
                                                        as u16;
                                                    *loop_count = if repeat_count != 0 {
                                                        repeat_count as i64 + 1
                                                    } else {
                                                        0
                                                    };
                                                }
                                            }
                                            // Terminator
                                            if !read_ok(src, &mut sub_block_size)
                                                || sub_block_size[0] != 0x00
                                            {
                                                return Err(Error::new("Netscape extension block not terminated correctly"));
                                            }
                                        } else {
                                            // Skip unexpected sub-block sizes
                                            let _ =
                                                src.seek(sub_block_size[0] as i64, IoWhence::Cur);
                                            let mut terminator = [0u8];
                                            if !read_ok(src, &mut terminator) || terminator[0] != 0
                                            {
                                                return Err(Error::new(
                                                    "Extension block not terminated correctly",
                                                ));
                                            }
                                        }
                                    } else {
                                        // Skip all sub-blocks for non-Netscape extensions
                                        skip_sub_blocks(src);
                                    }
                                }

                                0xFE => {
                                    // Comment Extension
                                    if let Some(comment) = comment.as_deref_mut() {
                                        let mut c: Option<Vec<u8>> = None;
                                        let mut sub_block_size = [0u8];
                                        while read_ok(src, &mut sub_block_size)
                                            && sub_block_size[0] > 0
                                        {
                                            let v = c.get_or_insert_with(Vec::new);
                                            let current_len = v.len();
                                            v.resize(current_len + sub_block_size[0] as usize, 0);
                                            if !read_ok(src, &mut v[current_len..]) {
                                                return Err(Error::new(
                                                    "Error reading GIF comment data",
                                                ));
                                            }
                                        }
                                        // (the comment ends at its first NUL, as a C string)
                                        *comment = c.map(|v| {
                                            let end =
                                                v.iter().position(|&b| b == 0).unwrap_or(v.len());
                                            String::from_utf8_lossy(&v[..end]).into_owned()
                                        });
                                    }
                                    // Note (upstream): without `comment`
                                    // the comment's sub-blocks are left
                                    // unread, and parsed as blocks.
                                }

                                _ => {
                                    // Other extensions
                                    skip_sub_blocks(src);
                                }
                            }
                        }

                        0x2C => {
                            // Image Descriptor
                            processing_extensions = false;
                        }

                        0x3B => {
                            // Trailer
                            return Err(Error::new("GIF file contains no images"));
                        }

                        block_type => {
                            // Unknown block type
                            return Err(Error::new(format!(
                                "Unknown GIF block type: 0x{block_type:02X}"
                            )));
                        }
                    }
                }
                let _ = src.seek(stream_pos, IoWhence::Set);
            }

            if !ctx.single_frame {
                if ctx.canvas.is_none() {
                    let Ok(mut canvas) = Surface::new(ctx.width, ctx.height, PixelFormat::RGBA32)
                    else {
                        return Err(Error::new("Failed to create canvas surface"));
                    };

                    if canvas.fill_rect(None, 0).is_err() {
                        return Err(Error::new(
                            "Failed to fill canvas surface with transparent color",
                        ));
                    }
                    ctx.canvas = Some(canvas);
                }

                if ctx.prev_canvas.is_none() {
                    let Ok(mut prev_canvas) =
                        Surface::new(ctx.width, ctx.height, PixelFormat::RGBA32)
                    else {
                        return Err(Error::new("Failed to create previous canvas surface"));
                    };

                    if prev_canvas.fill_rect(None, 0).is_err() {
                        return Err(Error::new(
                            "Failed to fill previous canvas surface with transparent color",
                        ));
                    }
                    ctx.prev_canvas = Some(prev_canvas);
                }
            }

            ctx.got_header = true;
        }
        Ok(())
    }

    /// Rewind to the first frame. Translation of
    /// `IMG_AnimationDecoderReset_Internal()`.
    pub(crate) fn reset(&mut self, d: &mut DecoderCore<'_, '_>) -> Result<()> {
        let start = d.start;
        if d.src().seek(start, IoWhence::Set).ok() != Some(start) {
            return Err(Error::new("Failed to seek to beginning of GIF file"));
        }

        let ctx = &mut *self;
        ctx.state = State::zeroed();
        ctx.state.gif89.transparent = -1;
        ctx.state.gif89.delay_time = -1;
        ctx.state.gif89.input_flag = -1;
        ctx.state.gif89.disposal = GIF_DISPOSE_NA;

        ctx.current_frame = 0;
        ctx.current_disposal = GIF_DISPOSE_NA;
        ctx.current_delay = 100;
        ctx.transparent_index = -1;
        ctx.got_header = false;
        ctx.got_eof = false;
        ctx.last_disposal = GIF_DISPOSE_NONE;
        ctx.restore_area = Rect::new(0, 0, 0, 0);

        // We don't care about metadata when resetting to re-read.
        ctx.ignore_props = true;

        if let Some(canvas) = &mut ctx.canvas {
            let _ = canvas.fill_rect(None, 0);
        }

        if let Some(prev_canvas) = &mut ctx.prev_canvas {
            let _ = prev_canvas.fill_rect(None, 0);
        }

        self.get_gif_header(d, None, None)
    }

    /// Decode the next frame: in single-frame mode the frame itself when it
    /// covers the whole screen, otherwise the frame composited onto the
    /// canvas (with the previous frame's disposal applied). `Ok(None)` at
    /// the end of the file (the `COMPLETE` status). Translation of
    /// `IMG_AnimationDecoderGetNextFrame_Internal()`.
    pub(crate) fn get_next_frame(
        &mut self,
        d: &mut DecoderCore<'_, '_>,
    ) -> Result<Option<(Surface<'static>, u64)>> {
        let mut frames_loaded = 0;

        if self.got_eof {
            d.status = AnimationDecoderStatus::Complete;
            return Ok(None);
        }

        self.get_gif_header(d, None, None)?;

        let frames_to_load = 1;
        let mut retval = None;
        let mut duration = 0;
        while frames_loaded < frames_to_load {
            let ctx = &mut *self;
            let src = d.src();
            let mut c = [0u8];
            if !read_ok(src, &mut c) {
                ctx.got_eof = true;
                break;
            }

            if c[0] == b';' {
                ctx.got_eof = true;
                break;
            }

            if c[0] == b'!' {
                if !read_ok(src, &mut c) {
                    ctx.got_eof = true;
                    break;
                }
                do_extension(src, c[0], &mut ctx.state);
                continue;
            }

            if c[0] != b',' {
                continue;
            }

            if !read_ok(src, &mut ctx.buf[..9]) {
                ctx.got_eof = true;
                break;
            }

            let use_global_colormap = !bit_set(ctx.buf[8], LOCALCOLORMAP);
            let bit_pixel = 1usize << ((ctx.buf[8] & 0x07) + 1);
            let left = lm_to_uint(ctx.buf[0], ctx.buf[1]);
            let top = lm_to_uint(ctx.buf[2], ctx.buf[3]);
            let width = lm_to_uint(ctx.buf[4], ctx.buf[5]);
            let height = lm_to_uint(ctx.buf[6], ctx.buf[7]);

            let mut local_color_map: ColorMap = [[0; MAXCOLORMAPSIZE]; 3];
            let mut gray_scale = 0;

            if !use_global_colormap
                && read_color_map(src, bit_pixel, &mut local_color_map, &mut gray_scale).is_err()
            {
                ctx.got_eof = true;
                break;
            }

            match ctx.last_disposal {
                GIF_DISPOSE_NONE => { /* Leave canvas as is */ }

                GIF_DISPOSE_RESTORE_BACKGROUND => {
                    let restore_area = ctx.restore_area;
                    let filled = match &mut ctx.canvas {
                        Some(canvas) => canvas.fill_rect(Some(&restore_area), 0),
                        None => Err(Error::invalid_param("dst")),
                    };
                    if filled.is_err() {
                        return Err(Error::new("Failed to fill canvas with background color"));
                    }
                }

                GIF_DISPOSE_RESTORE_PREVIOUS => {
                    /* Restore canvas to previous state */
                    if let Some(prev_canvas) = &mut ctx.prev_canvas {
                        let blitted = match &mut ctx.canvas {
                            Some(canvas) => prev_canvas.blit(None, canvas, None),
                            None => Err(Error::invalid_param("dst")),
                        };
                        if blitted.is_err() {
                            return Err(Error::new("Failed to restore previous canvas"));
                        }
                    }
                }

                _ => { /* Default is to do nothing */ }
            }

            /* If current disposal method is RESTORE_PREVIOUS, save current canvas */
            if ctx.state.gif89.disposal == GIF_DISPOSE_RESTORE_PREVIOUS {
                let blitted = match (&mut ctx.canvas, &mut ctx.prev_canvas) {
                    (Some(canvas), Some(prev_canvas)) => canvas.blit(None, prev_canvas, None),
                    _ => Err(Error::invalid_param("src")),
                };
                if blitted.is_err() {
                    return Err(Error::new("Failed to save current canvas for restoration"));
                }
            } else if ctx.state.gif89.disposal == GIF_DISPOSE_RESTORE_BACKGROUND {
                ctx.restore_area = Rect::new(left, top, width, height);
            }

            let interlace = bit_set(ctx.buf[8], INTERLACE);
            let image = if !use_global_colormap {
                read_image(
                    src,
                    width,
                    height,
                    bit_pixel,
                    &local_color_map,
                    gray_scale,
                    interlace,
                    false,
                    &mut ctx.state,
                )
            } else {
                let screen = ctx.state.gif_screen.clone();
                read_image(
                    src,
                    width,
                    height,
                    screen.bit_pixel as usize,
                    &screen.color_map,
                    screen.gray_scale,
                    interlace,
                    false,
                    &mut ctx.state,
                )
            };

            // Incorrect animation is harder to detect than a direct failure,
            // so it's better to fail than try to animate a GIF without the
            // full set of frames it has in the file.

            // Only set the error if ReadImage did not do it.
            // (ReadImage always does here)
            let Some(mut image) = image? else {
                return Err(Error::new("Failed to decode frame."));
            };

            if ctx.single_frame
                && left == 0
                && top == 0
                && image.width() == ctx.width
                && image.height() == ctx.height
            {
                retval = Some(image);
            } else {
                /* Composite the frame onto the canvas */
                // FIXME (upstream): in single-frame mode (IMG_LoadGIF_IO())
                // there is no canvas, so a first frame that doesn't cover
                // the whole screen fails to load here.
                let dest = Rect::new(left, top, width, height);
                let blitted = match &mut ctx.canvas {
                    Some(canvas) => image.blit(None, canvas, Some(&dest)),
                    None => Err(Error::invalid_param("dst")),
                };
                if blitted.is_err() {
                    return Err(Error::new("Failed to blit frame onto canvas"));
                }

                /* Store the frame in the output array */
                let duplicated = ctx.canvas.as_ref().map(|canvas| canvas.duplicate());
                match duplicated {
                    Some(Ok(frame)) => retval = Some(frame),
                    _ => return Err(Error::new("Failed to duplicate frame surface")),
                }
            }

            let (delay_time, last_duration) = (ctx.state.gif89.delay_time, ctx.last_duration);
            if delay_time < 0 && last_duration != 0 {
                duration = last_duration;
            } else if delay_time < 2 {
                /* Default animation delay, matching browser and Qt */
                duration = d.decoder_duration(10, 100);
            } else {
                duration = d.decoder_duration(delay_time as u64, 100);
            }
            let ctx = &mut *self;
            ctx.last_duration = duration;

            ctx.last_disposal = ctx.state.gif89.disposal;

            ctx.state.gif89.transparent = -1;
            ctx.state.gif89.delay_time = -1;
            ctx.state.gif89.input_flag = -1;
            ctx.state.gif89.disposal = GIF_DISPOSE_NA;

            frames_loaded += 1;
            ctx.current_frame += 1;
            ctx.frame_count += 1;
        }

        if frames_loaded == 0 {
            if self.got_eof {
                d.status = AnimationDecoderStatus::Complete;
                return Ok(None);
            }
            return Err(Error::new("Failed to load any frames"));
        }

        Ok(retval.map(|frame| (frame, duration)))
    }
}

/// Create the GIF decoder of an animation decoder, reading the header
/// (and unless [`PROP_METADATA_IGNORE_PROPS_BOOLEAN`] is set, the loop
/// count and comment into the decoder's properties); with
/// [`PROP_ANIMATION_DECODER_CREATE_GIF_SINGLE_IMAGE_BOOLEAN`], frames that
/// cover the screen are returned as decoded, without a canvas.
/// Translation of `IMG_CreateGIFAnimationDecoder()`.
pub(crate) fn create_gif_animation_decoder(
    d: &mut DecoderCore<'_, '_>,
    props: &Properties,
) -> Result<Box<GifContext>> {
    let mut state = State::zeroed();
    state.gif89.transparent = -1;
    state.gif89.delay_time = -1;
    state.gif89.input_flag = -1;
    state.gif89.disposal = GIF_DISPOSE_NA;

    let mut ctx = Box::new(GifContext {
        state,
        buf: [0; 256],
        version: [0; 4],
        width: 0,
        height: 0,
        canvas: None,
        prev_canvas: None,
        transparent_index: -1,
        got_header: false,
        got_eof: false,
        current_frame: 0,
        frame_count: 0,
        current_delay: 100,
        current_disposal: GIF_DISPOSE_NA,
        last_disposal: GIF_DISPOSE_NONE,
        restore_area: Rect::new(0, 0, 0, 0),
        single_frame: props
            .get_bool(PROP_ANIMATION_DECODER_CREATE_GIF_SINGLE_IMAGE_BOOLEAN)
            .unwrap_or(false),
        global_colormap: [[0; MAXCOLORMAPSIZE]; 3],
        global_colormap_size: 0,
        has_global_colormap: false,
        global_grayscale: false,
        last_duration: 0,
        ignore_props: false,
    });

    let mut comment = None;
    let mut loop_count = 1;
    ctx.get_gif_header(d, Some(&mut comment), Some(&mut loop_count))?;

    let ignore_props = props
        .get_bool(PROP_METADATA_IGNORE_PROPS_BOOLEAN)
        .unwrap_or(false);
    ctx.ignore_props = ignore_props;
    if !ignore_props {
        // Set well-defined properties.
        let _ = d.props.set(PROP_METADATA_LOOP_COUNT_NUMBER, loop_count);

        // Get other well-defined properties and set them in our props.
        if let Some(comment) = comment {
            let _ = d.props.set(PROP_METADATA_DESCRIPTION_STRING, comment);
        }
    }

    Ok(ctx)
}

/// Skip a run of data sub-blocks, up to and including the terminator.
fn skip_sub_blocks(src: &mut IoStream<'_>) {
    let mut sub_block_size = [0u8];
    while read_ok(src, &mut sub_block_size) && sub_block_size[0] > 0 {
        let _ = src.seek(sub_block_size[0] as i64, IoWhence::Cur);
    }
}

/* See if an image is contained in a data source */

/// Whether `src` holds a GIF image (87a or 89a); the stream position is
/// unchanged. Translation of `IMG_isGIF()`.
pub fn is_gif(src: &mut IoStream<'_>) -> bool {
    let start = src.tell().unwrap_or(-1);
    let mut is_gif = false;
    let mut magic = [0u8; 6];
    if read_ok(src, &mut magic)
        && &magic[..3] == b"GIF"
        && (&magic[3..6] == b"87a" || &magic[3..6] == b"89a")
    {
        is_gif = true;
    }
    let _ = src.seek(start, IoWhence::Set);
    is_gif
}

/// Load the first frame of a GIF image as an INDEX8 surface (with the
/// transparent color, if any, as its color key). Translation of
/// `IMG_LoadGIF_IO()`.
pub fn load_gif_io(src: &mut IoStream<'_>) -> Result<Surface<'static>> {
    // (IMG_CreateAnimationDecoderWithProperties() for "gif", single image)
    let props = Properties::new();
    let _ = props.set(PROP_ANIMATION_DECODER_CREATE_GIF_SINGLE_IMAGE_BOOLEAN, true);
    let start = src.tell().unwrap_or(-1);
    let mut d = DecoderCore {
        status: AnimationDecoderStatus::Ok,
        props: Properties::new(),
        src: Stream::Borrowed(src),
        start,
        timebase_numerator: 1,
        timebase_denominator: 1000,
        accumulated_pts: 0,
    };
    let mut ctx = match create_gif_animation_decoder(&mut d, &props) {
        Ok(ctx) => ctx,
        Err(e) => {
            // FIXME (upstream): when the GIF decoder can't be created,
            // IMG_CreateAnimationDecoderWithProperties() falls back to its
            // single-frame decoder, which loads the image with
            // IMG_LoadTyped_IO(): that detects a GIF and calls
            // IMG_LoadGIF_IO() again, without end. Here the GIF decoder's
            // error is returned instead.
            let _ = d.src().seek(start, IoWhence::Set);
            return Err(e);
        }
    };

    // (IMG_GetAnimationDecoderFrame(); IMG_CloseAnimationDecoder() leaves
    // the stream open)
    match ctx.get_next_frame(&mut d)? {
        Some((frame, _pts)) => Ok(frame),
        // Note (upstream): NULL with the error cleared.
        None => Err(Error::new("GIF file contains no images")),
    }
}

// ---------------------------------------------------------------------------
// The GIF encoder (SAVE_GIF, with SAVE_GIF_OCTREE: upstream's defaults)
// ---------------------------------------------------------------------------

// (GifHeaderAndLSD, GraphicsControlExtension, ImageDescriptor and
// NetscapeExtension are written field by field below)

// Function to write a byte to the SDL_IOStream
/// Translation of `writeByte()`.
fn write_byte(io: &mut IoStream<'_>, byte: u8) -> bool {
    io.write(&[byte]) == 1
}

// Function to write a 16-bit word (little-endian) to the SDL_IOStream
/// Translation of `writeWord()`.
fn write_word(io: &mut IoStream<'_>, word: u16) -> bool {
    let bytes = [(word & 0xFF) as u8, ((word >> 8) & 0xFF) as u8];
    io.write(&bytes) == 2
}

/// Translation of `struct IMG_AnimationEncoderContext` (the GIF one).
pub(crate) struct GifEncoderContext {
    width: u16,
    height: u16,
    global_color_table: [[u8; 3]; 256],
    num_global_colors: u16,
    transparent_color_index: i32,
    first_frame: bool,
    color_map_lut: Box<[[[u8; 32]; 32]; 32]>,
    lut_initialized: bool,
    use_lut: bool,
    metadata: Option<Properties>,
}

const LZW_MAX_CODES: usize = 4096;
const LZW_MAX_BITS: i32 = 12;

/// Translation of `BitStream`.
struct BitStream {
    buffer: Vec<u8>,
    current_byte: usize,
    current_bit: i32,
    allocated_size: usize,
}

impl BitStream {
    /// Translation of `BitStream_Init()`.
    fn init(allocated_size: usize) -> Option<BitStream> {
        let mut buffer = Vec::new();
        buffer.try_reserve_exact(allocated_size).ok()?;
        buffer.resize(allocated_size, 0);
        Some(BitStream {
            buffer,
            current_byte: 0,
            current_bit: 0,
            allocated_size,
        })
    }

    /// Translation of `BitStream_WriteCode()`.
    fn write_code(&mut self, code: i32, num_bits: i32) -> Result<()> {
        let required_bytes =
            self.current_byte + ((self.current_bit + num_bits + 7) / 8) as usize + 1;
        if required_bytes >= self.allocated_size {
            // TODO: For now we basically allocate 2x the current size, but we could implement a more sophisticated resizing strategy for large images.

            let mut new_size = self.allocated_size * 2;
            if new_size < required_bytes {
                new_size = required_bytes + 256;
            }
            if self
                .buffer
                .try_reserve_exact(new_size - self.allocated_size)
                .is_err()
            {
                return Err(Error::new("Failed to reallocate BitStream buffer."));
            }

            self.buffer.resize(new_size, 0);
            self.allocated_size = new_size;
        }

        for i in 0..num_bits {
            if code & (1 << i) != 0 {
                self.buffer[self.current_byte] |= 1 << self.current_bit;
            }

            self.current_bit += 1;
            if self.current_bit == 8 {
                self.current_bit = 0;
                self.current_byte += 1;
                self.buffer[self.current_byte] = 0;
            }
        }

        Ok(())
    }

    /// Translation of `BitStream_Flush()`.
    fn flush(&mut self) -> usize {
        if self.current_bit > 0 {
            self.current_byte += 1;
        }
        self.current_byte
    }
}

/// LZW-compress `width` by `height` indexed pixels: the data and its size.
/// Translation of `lzwCompress()`.
fn lzw_compress(
    indexed_pixels: &[u8],
    width: u16,
    height: u16,
    min_code_size: u8,
    quality: i32,
) -> Result<(Vec<u8>, usize)> {
    // Validate minCodeSize is in reasonable range
    if min_code_size < 2 || min_code_size as i32 >= LZW_MAX_BITS {
        return Err(Error::new(format!(
            "Invalid minCodeSize {} (must be between 2 and {})",
            min_code_size,
            LZW_MAX_BITS - 1
        )));
    }

    const HASH_TABLE_SIZE: u32 = 32771;

    // Please do not lower the threshold for <50, lowering threshold too much will result in corrupted frames after lzw compression.
    let quality_threshold = if quality < 50 {
        3072
    } else if quality < 75 {
        3584
    } else {
        4096
    };

    // (an int product there)
    let mut allocated_size = (width as i32).wrapping_mul(height as i32) as isize as usize;
    if allocated_size < 4096 {
        allocated_size = 4096;
    }

    let Some(mut bs) = BitStream::init(allocated_size) else {
        return Err(Error::new("Failed to allocate bitstream buffer"));
    };

    /// (`DictEntry`)
    #[derive(Clone, Copy, Default)]
    struct DictEntry {
        prefix: i32,
        suffix: u8,
        next_in_chain: i32,
    }

    let mut dict = vec![DictEntry::default(); LZW_MAX_CODES];

    let mut hash_table = vec![-1i32; HASH_TABLE_SIZE as usize];

    let clear_code = 1i32 << min_code_size;
    let eoi_code = clear_code + 1;
    let pixel_count = width as usize * height as usize;

    let hash_func = |prefix: i32, suffix: u8| -> usize {
        (((((prefix as u32) << 13) ^ ((suffix as u32) << 5)).wrapping_mul(2654435761))
            ^ ((prefix as u32 >> 7).wrapping_mul(16777619))) as usize
            % HASH_TABLE_SIZE as usize
    };

    let mut next_code = eoi_code + 1;
    let mut cur_code_size = min_code_size as i32 + 1;

    bs.write_code(clear_code, cur_code_size)?;

    if pixel_count > 0 {
        let mut cur_string = indexed_pixels[0] as i32;

        for &pixel in &indexed_pixels[1..pixel_count] {
            let hash_key = hash_func(cur_string, pixel);
            let mut code = hash_table[hash_key];

            let mut found = false;
            while code != -1 {
                let entry = dict[code as usize];
                if entry.prefix == cur_string && entry.suffix == pixel {
                    cur_string = code;
                    found = true;
                    break;
                }
                code = entry.next_in_chain;
            }

            if !found {
                bs.write_code(cur_string, cur_code_size)?;

                if next_code < quality_threshold {
                    dict[next_code as usize] = DictEntry {
                        prefix: cur_string,
                        suffix: pixel,
                        next_in_chain: hash_table[hash_key],
                    };
                    hash_table[hash_key] = next_code;

                    if next_code == (1 << cur_code_size) && cur_code_size < LZW_MAX_BITS {
                        cur_code_size += 1;
                    }
                    next_code += 1;
                } else {
                    // Dictionary reset is controlled by quality threshold
                    bs.write_code(clear_code, cur_code_size)?;
                    cur_code_size = min_code_size as i32 + 1;
                    next_code = eoi_code + 1;
                    hash_table.fill(-1);
                }
                cur_string = pixel as i32;
            }
        }
        bs.write_code(cur_string, cur_code_size)?;
    }

    bs.write_code(eoi_code, cur_code_size)?;

    let compressed_size = bs.flush();
    Ok((bs.buffer, compressed_size))
}

const OCTREE_MAX_LEVELS: usize = 8;

/// Translation of `OctreeNode` (links are indices into the octree's nodes).
#[derive(Clone, Default)]
struct OctreeNode {
    children: [Option<usize>; 8],
    parent: Option<usize>,
    pixel_count: u32,
    r_sum: u64,
    g_sum: u64,
    b_sum: u64,
    palette_index: i32,
    level: usize,
    is_leaf: bool,
    in_leaf_list: bool,
    next_leaf: Option<usize>,
    prev_leaf: Option<usize>,
}

/// Translation of `Octree`.
struct Octree {
    nodes: Vec<OctreeNode>,
    root: usize,
    head_leaf_list: [Option<usize>; OCTREE_MAX_LEVELS],
    tail_leaf_list: [Option<usize>; OCTREE_MAX_LEVELS],
    leaf_count: u32,
    max_colors: u32,
    palette: Vec<u8>,
    palette_size: u32,
}

impl Octree {
    /// Translation of `OctreeNode_Create()`.
    fn node_create(&mut self, level: usize, parent: Option<usize>) -> usize {
        self.nodes.push(OctreeNode {
            level,
            parent,
            is_leaf: true,
            in_leaf_list: false,
            palette_index: -1,
            next_leaf: None,
            prev_leaf: None,
            ..OctreeNode::default()
        });
        self.nodes.len() - 1
    }

    /// Translation of `Octree_Init()`.
    fn init(max_colors: u32) -> Octree {
        let mut octree = Octree {
            nodes: Vec::new(),
            root: 0,
            head_leaf_list: [None; OCTREE_MAX_LEVELS],
            tail_leaf_list: [None; OCTREE_MAX_LEVELS],
            leaf_count: 0,
            max_colors,
            palette: Vec::new(),
            palette_size: 0,
        };
        octree.root = octree.node_create(0, None);
        octree
    }

    /// Translation of `Octree_AddLeaf()`.
    fn add_leaf(&mut self, node: usize) {
        if self.nodes[node].in_leaf_list {
            return;
        }

        let level = self.nodes[node].level;
        self.nodes[node].next_leaf = None;
        self.nodes[node].prev_leaf = self.tail_leaf_list[level];
        if let Some(tail) = self.tail_leaf_list[level] {
            self.nodes[tail].next_leaf = Some(node);
        } else {
            self.head_leaf_list[level] = Some(node);
        }
        self.tail_leaf_list[level] = Some(node);
        self.nodes[node].in_leaf_list = true;
        self.leaf_count = self.leaf_count.wrapping_add(1);
    }

    /// Translation of `Octree_RemoveLeaf()`.
    fn remove_leaf(&mut self, node: usize) {
        if !self.nodes[node].in_leaf_list {
            return;
        }

        let level = self.nodes[node].level;
        let (prev, next) = (self.nodes[node].prev_leaf, self.nodes[node].next_leaf);
        if let Some(prev) = prev {
            self.nodes[prev].next_leaf = next;
        } else {
            self.head_leaf_list[level] = next;
        }
        if let Some(next) = next {
            self.nodes[next].prev_leaf = prev;
        } else {
            self.tail_leaf_list[level] = prev;
        }
        self.nodes[node].next_leaf = None;
        self.nodes[node].prev_leaf = None;
        self.nodes[node].in_leaf_list = false;
        self.leaf_count = self.leaf_count.wrapping_sub(1);
    }

    /// Translation of `Octree_InsertColor()`.
    fn insert_color(&mut self, node: usize, r: u8, g: u8, b: u8, level: usize) {
        if self.nodes[node].is_leaf && level < OCTREE_MAX_LEVELS - 1 {
            if self.nodes[node].pixel_count > 0 {
                self.remove_leaf(node);
            }
            self.nodes[node].is_leaf = false;
        }

        let n = &mut self.nodes[node];
        n.pixel_count = n.pixel_count.wrapping_add(1);
        n.r_sum += r as u64;
        n.g_sum += g as u64;
        n.b_sum += b as u64;

        if level == OCTREE_MAX_LEVELS - 1 {
            self.add_leaf(node);
            return;
        }

        let mut index = 0;
        if r & (1 << (7 - level)) != 0 {
            index |= 4;
        }
        if g & (1 << (7 - level)) != 0 {
            index |= 2;
        }
        if b & (1 << (7 - level)) != 0 {
            index |= 1;
        }

        let child = match self.nodes[node].children[index] {
            Some(child) => child,
            None => {
                let child = self.node_create(level + 1, Some(node));
                self.nodes[node].children[index] = Some(child);
                child
            }
        };

        self.insert_color(child, r, g, b, level + 1);
    }

    /// Translation of `OctreeNode_AllChildrenAreLeaves()`.
    fn all_children_are_leaves(&self, node: usize) -> bool {
        if self.nodes[node].is_leaf {
            return false;
        }

        let mut has_any_children = false;
        for child in self.nodes[node].children.iter().flatten() {
            has_any_children = true;
            if !self.nodes[*child].is_leaf {
                return false;
            }
        }
        has_any_children
    }

    /// Translation of `Octree_Reduce()`.
    fn reduce(&mut self) -> Result<()> {
        let mut node_to_collapse = None;
        let mut min_pixel_count = u32::MAX;

        for level in (0..OCTREE_MAX_LEVELS - 1).rev() {
            let mut current_leaf = self.head_leaf_list[level + 1];
            while let Some(leaf) = current_leaf {
                if let Some(parent_node) = self.nodes[leaf].parent {
                    if self.nodes[parent_node].level == level
                        && !self.nodes[parent_node].is_leaf
                        && self.all_children_are_leaves(parent_node)
                    {
                        let mut current_pixel_count = 0u32;
                        for child in self.nodes[parent_node].children.iter().flatten() {
                            current_pixel_count =
                                current_pixel_count.wrapping_add(self.nodes[*child].pixel_count);
                        }

                        if current_pixel_count < min_pixel_count {
                            min_pixel_count = current_pixel_count;
                            node_to_collapse = Some(parent_node);
                        }
                    }
                }
                current_leaf = self.nodes[leaf].next_leaf;
            }
        }

        let Some(node) = node_to_collapse else {
            return Err(Error::new(
                "Octree_Reduce: No suitable node found to collapse to reduce leafCount.",
            ));
        };

        let n = &mut self.nodes[node];
        n.r_sum = 0;
        n.g_sum = 0;
        n.b_sum = 0;
        n.pixel_count = 0;

        for i in 0..8 {
            if let Some(child) = self.nodes[node].children[i] {
                self.remove_leaf(child);

                let c = self.nodes[child].clone();
                let n = &mut self.nodes[node];
                n.r_sum += c.r_sum;
                n.g_sum += c.g_sum;
                n.b_sum += c.b_sum;
                n.pixel_count = n.pixel_count.wrapping_add(c.pixel_count);

                n.children[i] = None;
            }
        }

        self.nodes[node].is_leaf = true;
        self.add_leaf(node);

        Ok(())
    }

    /// Translation of `Octree_BuildPalette()`.
    fn build_palette(&mut self, node: usize, palette_index_counter: &mut i32) -> bool {
        if self.nodes[node].is_leaf {
            let i = *palette_index_counter as usize * 3;
            let n = &self.nodes[node];
            let rgb = if n.pixel_count > 0 {
                [
                    (n.r_sum / n.pixel_count as u64) as u8,
                    (n.g_sum / n.pixel_count as u64) as u8,
                    (n.b_sum / n.pixel_count as u64) as u8,
                ]
            } else {
                [0, 0, 0]
            };
            if self.palette.len() < i + 3 {
                // (the palette has room for every leaf the reduction keeps)
                return false;
            }
            self.palette[i..i + 3].copy_from_slice(&rgb);
            self.nodes[node].palette_index = *palette_index_counter;
            *palette_index_counter += 1;
            self.palette_size += 1;
            return true;
        }

        for i in 0..8 {
            if let Some(child) = self.nodes[node].children[i] {
                if !self.build_palette(child, palette_index_counter) {
                    return false;
                }
            }
        }

        true
    }

    /// Translation of `Octree_GetPaletteIndex()`.
    fn get_palette_index(&self, node: usize, r: u8, g: u8, b: u8, level: usize) -> i32 {
        let n = &self.nodes[node];
        if n.is_leaf {
            return n.palette_index;
        }

        let mut index = 0;
        if r & (1 << (7 - level)) != 0 {
            index |= 4;
        }
        if g & (1 << (7 - level)) != 0 {
            index |= 2;
        }
        if b & (1 << (7 - level)) != 0 {
            index |= 1;
        }

        if let Some(child) = n.children[index] {
            self.get_palette_index(child, r, g, b, level + 1)
        } else {
            let mut best_dist = 196608; // 3 * 256 * 256, larger than any possible distance
            let mut best_index = -1;
            let dist_to = |c: &OctreeNode| {
                let dr = r as i32 - (c.r_sum / c.pixel_count as u64) as u8 as i32;
                let dg = g as i32 - (c.g_sum / c.pixel_count as u64) as u8 as i32;
                let db = b as i32 - (c.b_sum / c.pixel_count as u64) as u8 as i32;
                dr * dr + dg * dg + db * db
            };

            // Find the first valid child to initialize the search
            for child in n.children.iter().flatten() {
                let c = &self.nodes[*child];
                if c.pixel_count > 0 {
                    best_dist = dist_to(c);
                    best_index = c.palette_index;
                    break;
                }
            }

            // Now search the rest of the children for a better match
            for child in n.children.iter().flatten() {
                let c = &self.nodes[*child];
                if c.pixel_count > 0 {
                    let dist = dist_to(c);

                    if dist < best_dist {
                        best_dist = dist;
                        best_index = c.palette_index;
                    }
                }
            }

            if best_index >= 0 {
                best_index
            } else {
                0
            }
        }
    }
}

/// Translation of `count_set_bits()`.
fn count_set_bits(n: u32) -> i32 {
    n.count_ones() as i32
}

/// Translation of `buildColorMapLUT()`.
fn build_color_map_lut(
    lut: &mut [[[u8; 32]; 32]; 32],
    palette: &[[u8; 3]; 256],
    num_colors: u16,
    has_transparency: bool,
) {
    let color_count = if has_transparency {
        num_colors as i32 - 1
    } else {
        num_colors as i32
    };

    for r in 0..32 {
        for g in 0..32 {
            for b in 0..32 {
                // Map the 5-bit color back to 8-bit to find the closest match
                let r8 = ((r << 3) | (r >> 2)) as i32;
                let g8 = ((g << 3) | (g >> 2)) as i32;
                let b8 = ((b << 3) | (b >> 2)) as i32;

                let mut best_match = 0;
                let mut min_dist = 195076; // 3 * 255*255 + 1

                for i in 0..color_count.max(0) as usize {
                    let dr = r8 - palette[i][0] as i32;
                    let dg = g8 - palette[i][1] as i32;
                    let db = b8 - palette[i][2] as i32;
                    let dist = dr * dr + dg * dg + db * db;
                    if dist < min_dist {
                        min_dist = dist;
                        best_match = i;
                        if min_dist == 0 {
                            break; // Exact match
                        }
                    }
                }
                lut[r][g][b] = best_match as u8;
            }
        }
    }
}

/// The pixels of a surface, locking it if it must be: the surface's own
/// or (for a surface that needs it) a lock's.
fn with_pixels<T>(
    surf: &mut Surface<'_>,
    lock_error: &'static str,
    f: impl FnOnce(&[u8], usize) -> T,
) -> Result<T> {
    let pitch = surf.pitch() as usize;
    if surf.must_lock() {
        let Ok(lock) = surf.lock() else {
            return Err(Error::new(lock_error));
        };
        Ok(f(lock.pixels().unwrap_or(&[]), pitch))
    } else {
        Ok(f(surf.pixels().unwrap_or(&[]), pitch))
    }
}

/// A pixel of a 32-bit surface.
fn pixel32(pixels: &[u8], at: usize) -> u32 {
    u32::from_ne_bytes([pixels[at], pixels[at + 1], pixels[at + 2], pixels[at + 3]])
}

/// Translation of `mapSurfaceToExistingPalette()`.
fn map_surface_to_existing_palette(
    psurf: &mut Surface<'_>,
    lut: &[[[u8; 32]; 32]; 32],
    indexed_pixels: &mut [u8],
    transparent_index: i32,
) -> Result<()> {
    let color_key = psurf.color_key().unwrap_or(0);
    let has_transparency = psurf.has_color_key();

    let mut converted;
    let surf: &mut Surface<'_> =
        if psurf.format() != PixelFormat::RGBA32 && psurf.format() != PixelFormat::ARGB32 {
            converted = match psurf.convert(PixelFormat::RGBA32) {
                Ok(s) => s,
                Err(_) => {
                    return Err(Error::new(
                        "Failed to convert surface to RGBA32 for palette mapping.",
                    ))
                }
            };
            &mut converted
        } else {
            psurf
        };

    let details = *surf.format_details();

    let r_bpp = count_set_bits(details.Rmask);
    let g_bpp = count_set_bits(details.Gmask);
    let b_bpp = count_set_bits(details.Bmask);

    let (w, h) = (surf.width() as usize, surf.height() as usize);
    with_pixels(
        surf,
        "Failed to lock surface for palette mapping.",
        |pixels, pitch| {
            for y in 0..h {
                let dst_row = &mut indexed_pixels[y * w..(y + 1) * w];
                for x in 0..w {
                    let pixel = pixel32(pixels, y * pitch + x * 4);
                    if has_transparency && pixel == color_key {
                        dst_row[x] = transparent_index as u8;
                        continue;
                    }

                    let r = (((pixel & details.Rmask) >> details.Rshift) << (8 - r_bpp)) as u8;
                    let g = (((pixel & details.Gmask) >> details.Gshift) << (8 - g_bpp)) as u8;
                    let b = (((pixel & details.Bmask) >> details.Bshift) << (8 - b_bpp)) as u8;

                    // Use the pre-calculated LUT for an O(1) color lookup.
                    dst_row[x] = lut[(r >> 3) as usize][(g >> 3) as usize][(b >> 3) as usize];
                }
            }
        },
    )
}

/// Translation of `quantizeSurfaceToIndexedPixels()` (its octree version).
fn quantize_surface_to_indexed_pixels(
    psurf: &mut Surface<'_>,
    palette: &mut [[u8; 3]; 256],
    num_palette_colors: u16,
    indexed_pixels: &mut [u8],
    transparent_index: i32,
) -> Result<()> {
    if num_palette_colors == 0 || (num_palette_colors & (num_palette_colors - 1)) != 0 {
        return Err(Error::new("Invalid arguments for quantizeSurfaceToIndexedPixels: numPaletteColors must be a power of 2."));
    }

    let mut has_transparency = false;

    let mut color_key = 0u32;
    if transparent_index >= 0 {
        if let Some(key) = psurf.color_key() {
            color_key = key;
        }
        has_transparency = true;
    }
    let psurf_has_color_key = psurf.has_color_key();
    let usable_colors = if has_transparency {
        num_palette_colors as i32 - 1
    } else {
        num_palette_colors as i32
    };

    if psurf.format() == PixelFormat::INDEX8 {
        let colors: Vec<_> = match psurf.palette() {
            Some(p) => p
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .colors()
                .to_vec(),
            None => return Err(Error::new("INDEX8 surface has no palette.")),
        };

        if colors.len() as i32 > usable_colors {
            return Err(Error::new(
                "INDEX8 surface palette has too many colors for target.",
            ));
        }

        for (i, c) in colors.iter().enumerate() {
            palette[i] = [c.r, c.g, c.b];
        }

        for entry in palette
            .iter_mut()
            .take(num_palette_colors as usize)
            .skip(colors.len())
        {
            *entry = [0, 0, 0];
        }

        if has_transparency {
            palette[transparent_index as usize] = [0, 0, 0];
        }

        let (w, h) = (psurf.width() as usize, psurf.height() as usize);
        // FIXME (upstream): without a color key the key compared is 0, so
        // the pixels of index 0 are made transparent.
        return with_pixels(psurf, "Failed to lock INDEX8 surface", |pixels, pitch| {
            for y in 0..h {
                let src_row = &pixels[y * pitch..y * pitch + w];
                let dst_row = &mut indexed_pixels[y * w..(y + 1) * w];

                if has_transparency {
                    for x in 0..w {
                        let index = src_row[x];
                        let pixel = index as u32;

                        if pixel == color_key {
                            dst_row[x] = transparent_index as u8;
                        } else {
                            dst_row[x] = index;
                        }
                    }
                } else {
                    dst_row.copy_from_slice(src_row);
                }
            }
        });
    }

    let mut converted;
    let surf: &mut Surface<'_> =
        if psurf.format() != PixelFormat::RGBA32 && psurf.format() != PixelFormat::ARGB32 {
            converted = match psurf.convert(PixelFormat::RGBA32) {
                Ok(s) => s,
                Err(_) => return Err(Error::new("Failed to convert surface to RGBA32 format.")),
            };
            &mut converted
        } else {
            psurf
        };
    let details = *surf.format_details();

    let mut octree = Octree::init(usable_colors as u32);
    let mut octree_palette = Vec::new();
    if octree_palette
        .try_reserve_exact(num_palette_colors as usize * 3)
        .is_err()
    {
        return Err(Error::new("Failed to allocate palette memory."));
    }
    octree_palette.resize(num_palette_colors as usize * 3, 0);
    octree.palette = octree_palette;
    octree.palette_size = 0;
    octree.max_colors = usable_colors as u32;
    octree.leaf_count = 0;
    let root = octree.root;

    let r_bpp = count_set_bits(details.Rmask);
    let g_bpp = count_set_bits(details.Gmask);
    let b_bpp = count_set_bits(details.Bmask);

    let current_amask = details.Amask;
    let current_ashift = details.Ashift;
    let current_a_bpp = if current_amask == 0 {
        0
    } else {
        count_set_bits(current_amask)
    };

    let is_transparent = |pixel: u32| -> bool {
        if has_transparency {
            if current_amask != 0 && current_a_bpp > 0 {
                let a = (((pixel & current_amask) >> current_ashift) << (8 - current_a_bpp)) as u8;
                if a < 128 {
                    // Alpha threshold
                    return true;
                }
            } else if psurf_has_color_key && pixel == color_key {
                return true;
            }
        }
        false
    };
    let rgb = |pixel: u32| -> (u8, u8, u8) {
        (
            (((pixel & details.Rmask) >> details.Rshift) << (8 - r_bpp)) as u8,
            (((pixel & details.Gmask) >> details.Gshift) << (8 - g_bpp)) as u8,
            (((pixel & details.Bmask) >> details.Bshift) << (8 - b_bpp)) as u8,
        )
    };

    let (w, h) = (surf.width() as usize, surf.height() as usize);
    with_pixels(
        surf,
        "Failed to lock RGBA32 surface",
        |pixels, pitch| -> Result<()> {
            for y in 0..h {
                for x in 0..w {
                    let pixel = pixel32(pixels, y * pitch + x * 4);

                    if is_transparent(pixel) {
                        continue;
                    } else {
                        let (r, g, b) = rgb(pixel);
                        octree.insert_color(root, r, g, b, 0);
                    }
                }
            }

            while octree.leaf_count > octree.max_colors && octree.leaf_count > 1 {
                octree.reduce()?;
            }

            let mut palette_index_counter = 0;
            octree.palette_size = 0;
            if !octree.build_palette(octree.root, &mut palette_index_counter) {
                return Err(Error::new("Failed to build palette from Octree."));
            }
            if palette_index_counter > octree.max_colors as i32 {
                return Err(Error::new("Octree built more colors than expected."));
            }

            let mut dest_index = 0u32;
            for i in 0..octree.palette_size as usize {
                if has_transparency && dest_index == transparent_index as u32 {
                    dest_index += 1;
                }

                palette[dest_index as usize] = [
                    octree.palette[i * 3],
                    octree.palette[i * 3 + 1],
                    octree.palette[i * 3 + 2],
                ];
                dest_index += 1;
            }

            for i in dest_index as usize..num_palette_colors as usize {
                palette[i] = [0, 0, 0];
            }

            if has_transparency {
                palette[transparent_index as usize] = [0, 0, 0];
            }

            for y in 0..h {
                let dst_row = &mut indexed_pixels[y * w..(y + 1) * w];
                for x in 0..w {
                    let pixel = pixel32(pixels, y * pitch + x * 4);

                    if is_transparent(pixel) {
                        dst_row[x] = transparent_index as u8;
                    } else {
                        let (r, g, b) = rgb(pixel);
                        let mut index = octree.get_palette_index(octree.root, r, g, b, 0);

                        if has_transparency && index >= transparent_index {
                            index += 1;
                        }

                        dst_row[x] = index as u8;
                    }
                }
            }
            Ok(())
        },
    )?
}

/// Translation of `writeGifHeader()`.
fn write_gif_header(
    io: &mut IoStream<'_>,
    width: u16,
    height: u16,
    has_global_color_table: bool,
    color_resolution: u8,
    sorted_color_table: bool,
    background_color_index: u8,
    pixel_aspect_ratio: u8,
    gct_size_field_value: u8,
) -> Result<()> {
    // Write signature and version (6 bytes)
    if io.write(b"GIF89a") != 6 {
        return Err(Error::new("Failed to write GIF signature and version."));
    }

    // Write width (2 bytes, little-endian)
    if !write_word(io, width) {
        return Err(Error::new("Failed to write GIF width."));
    }

    // Write height (2 bytes, little-endian)
    if !write_word(io, height) {
        return Err(Error::new("Failed to write GIF height."));
    }

    // Pack Global Color Table Info byte
    let mut global_color_table_info = 0u8;
    if has_global_color_table {
        global_color_table_info |= 0x80; // Set GCT Flag (bit 7)
        global_color_table_info |= gct_size_field_value & 0x07; // Set GCT Size (bits 0-2)
    }
    global_color_table_info |= (color_resolution.wrapping_sub(1) & 0x07) << 4; // Set Color Resolution (bits 4-6)
    if sorted_color_table {
        global_color_table_info |= 0x08; // Set Sorted Flag (bit 3)
    }

    // Write packed fields byte
    if !write_byte(io, global_color_table_info) {
        return Err(Error::new("Failed to write GIF packed fields byte."));
    }

    // Write background color index
    if !write_byte(io, background_color_index) {
        return Err(Error::new("Failed to write GIF background color index."));
    }

    // Write pixel aspect ratio
    if !write_byte(io, pixel_aspect_ratio) {
        return Err(Error::new("Failed to write GIF pixel aspect ratio."));
    }

    Ok(())
}

/// Translation of `writeColorTable()`.
fn write_color_table(
    io: &mut IoStream<'_>,
    colors: &[[u8; 3]; 256],
    num_colors: u16,
) -> Result<()> {
    if num_colors == 0 {
        return Err(write_error(io));
    }

    let bytes: Vec<u8> = colors[..num_colors as usize]
        .iter()
        .flatten()
        .copied()
        .collect();
    if io.write(&bytes) != bytes.len() {
        return Err(Error::new("Failed to write color table data"));
    }

    Ok(())
}

/// Translation of `writeGraphicsControlExtension()`.
fn write_graphics_control_extension(
    io: &mut IoStream<'_>,
    delay_time: u16,
    transparent_color_index: i32,
    disposal_method: u8,
) -> bool {
    // Extension Introducer
    if !write_byte(io, 0x21) {
        return false;
    }
    // Graphic Control Label
    if !write_byte(io, 0xF9) {
        return false;
    }
    // Block Size
    if !write_byte(io, 0x04) {
        return false;
    }

    let mut packed_fields = 0u8;
    // Reserved (3 bits) - 0
    // Disposal Method (3 bits)
    packed_fields |= (disposal_method & 0x07) << 2;
    // User Input Flag (1 bit) - 0 (no user input)
    // Transparent Color Flag (1 bit)
    if transparent_color_index != -1 {
        packed_fields |= 1 << 0;
    }
    if !write_byte(io, packed_fields) {
        return false;
    }

    if !write_word(io, delay_time) {
        return false;
    }
    if !write_byte(
        io,
        if transparent_color_index != -1 {
            transparent_color_index as u8
        } else {
            0x00
        },
    ) {
        return false;
    }
    // Block Terminator
    write_byte(io, 0x00)
}

/// Translation of `writeImageDescriptor()`.
fn write_image_descriptor(
    io: &mut IoStream<'_>,
    left: u16,
    top: u16,
    width: u16,
    height: u16,
    has_local_color_table: bool,
    interlace: bool,
    sorted_color_table: bool,
    local_color_table_size: u8,
) -> bool {
    // Image Separator
    if !write_byte(io, 0x2C) {
        return false;
    }
    if !write_word(io, left) || !write_word(io, top) {
        return false;
    }
    if !write_word(io, width) || !write_word(io, height) {
        return false;
    }

    let mut packed_fields = 0u8;
    // Local Color Table Flag (1 bit)
    if has_local_color_table {
        packed_fields |= 1 << 7;
    }
    // Interlace Flag (1 bit)
    if interlace {
        packed_fields |= 1 << 6;
    }
    // Sort Flag (1 bit)
    if sorted_color_table {
        packed_fields |= 1 << 5;
    }
    // Reserved (2 bits) - 0
    // Size of Local Color Table (3 bits)
    if has_local_color_table {
        packed_fields |= local_color_table_size & 0x07;
    }
    write_byte(io, packed_fields)
}

/// Translation of `writeImageData()`.
fn write_image_data(
    io: &mut IoStream<'_>,
    min_code_size: u8,
    compressed_data: &[u8],
    data_size: usize,
) -> bool {
    // LZW Minimum Code Size
    if !write_byte(io, min_code_size) {
        return false;
    }

    if data_size == 0 {
        // For empty images, write a zero-sized data block
        // Empty block
        write_byte(io, 0x00);
    } else {
        // Write data sub-blocks (max 255 bytes per sub-block)
        let mut bytes_written = 0;
        while bytes_written < data_size {
            let sub_block_size = (data_size - bytes_written).min(255);
            // Block Size
            if !write_byte(io, sub_block_size as u8) {
                return false;
            }
            if io.write(&compressed_data[bytes_written..bytes_written + sub_block_size]) == 0 {
                return false;
            }
            bytes_written += sub_block_size;
        }
    }

    // Block Terminator (end of image data)
    write_byte(io, 0x00)
}

/// Translation of `writeNetscapeLoopExtension()`.
fn write_netscape_loop_extension(io: &mut IoStream<'_>, loop_count: u16) -> bool {
    // Omit the extension if the loop count is 1, since 1 can't be represented
    if loop_count == 1 {
        return true;
    }

    // Extension Introducer
    if !write_byte(io, 0x21) {
        return false;
    }
    // Application Extension Label
    if !write_byte(io, 0xFF) {
        return false;
    }
    // Block Size
    if !write_byte(io, 0x0B) {
        return false;
    }

    // Application Identifier and Authentication Code
    if io.write(b"NETSCAPE2.0") != 11 {
        return false;
    }

    // Sub-block Size
    if !write_byte(io, 0x03) {
        return false;
    }
    // Sub-block ID
    if !write_byte(io, 0x01) {
        return false;
    }
    // Loop Count
    let repeat_count = if loop_count > 0 { loop_count - 1 } else { 0 };
    if !write_word(io, repeat_count) {
        return false;
    }
    // Block Terminator
    write_byte(io, 0x00)
}

/// Translation of `writeCommentExtension()`: `comment` is a C string.
fn write_comment_extension(io: &mut IoStream<'_>, comment: &[u8]) -> bool {
    let comment = &comment[..comment
        .iter()
        .position(|&b| b == 0)
        .unwrap_or(comment.len())];
    if comment.is_empty() {
        return true;
    }

    // Extension Introducer: 0x21
    if !write_byte(io, 0x21) {
        return false;
    }
    // Comment Extension Label: 0xFE
    if !write_byte(io, 0xFE) {
        return false;
    }

    // Write the comment in sub-blocks of up to 255 bytes
    for block in comment.chunks(255) {
        if !write_byte(io, block.len() as u8) {
            return false;
        }

        if io.write(block) != block.len() {
            return false;
        }
    }

    // Block Terminator: 0x00
    write_byte(io, 0x00)
}

/// Translation of `writeGifTrailer()`.
fn write_gif_trailer(io: &mut IoStream<'_>) -> bool {
    // GIF Trailer
    write_byte(io, 0x3B)
}

impl GifEncoderContext {
    /// Translation of `AnimationEncoder_AddFrame()`.
    pub(crate) fn add_frame(
        &mut self,
        e: &mut EncoderCore<'_, '_>,
        surface: &mut Surface<'_>,
        duration: u64,
    ) -> Result<()> {
        let ctx = self;
        let num_colors = ctx.num_global_colors;
        let mut palette_bits_per_pixel = 0u8;
        let mut local_color_table = [[0u8; 3]; 256];
        let use_local_color_table = !ctx.first_frame;

        if num_colors > 1 {
            let mut temp_colors = num_colors;
            while temp_colors > 1 && palette_bits_per_pixel < 8 {
                temp_colors >>= 1;
                palette_bits_per_pixel += 1;
            }
        } else {
            palette_bits_per_pixel = 1;
        }

        let (w, h) = (surface.width(), surface.height());
        let pixel_buffer_size = w as usize * h as usize;
        if pixel_buffer_size == 0 {
            return Err(Error::new("Surface dimensions too large for GIF encoding"));
        }
        let mut indexed_pixels = Vec::new();
        if indexed_pixels.try_reserve_exact(pixel_buffer_size).is_err() {
            return Err(Error::new("Failed to allocate indexed pixel buffer."));
        }
        // (uninitialized there; every pixel is written)
        indexed_pixels.resize(pixel_buffer_size, 0);

        if ctx.first_frame {
            ctx.width = w as u16;
            ctx.height = h as u16;

            quantize_surface_to_indexed_pixels(
                surface,
                &mut ctx.global_color_table,
                num_colors,
                &mut indexed_pixels,
                ctx.transparent_color_index,
            )?;

            // Build the fast lookup table for subsequent frames.
            if ctx.use_lut && !ctx.lut_initialized {
                build_color_map_lut(
                    &mut ctx.color_map_lut,
                    &ctx.global_color_table,
                    num_colors,
                    ctx.transparent_color_index != -1,
                );
                ctx.lut_initialized = true;
            }

            let io = e.dst();
            let gct_size_field_value = if palette_bits_per_pixel > 0 {
                palette_bits_per_pixel - 1
            } else {
                0
            };
            write_gif_header(
                io,
                ctx.width,
                ctx.height,
                true,
                8,
                false,
                0,
                0,
                gct_size_field_value,
            )?;
            write_color_table(io, &ctx.global_color_table, num_colors)?;

            let mut loop_count = 0;
            let mut description = None;
            if let Some(metadata) = &ctx.metadata {
                loop_count = metadata
                    .get_number(PROP_METADATA_LOOP_COUNT_NUMBER)
                    .unwrap_or(0)
                    .max(0) as i32;
                description = metadata.get_string(PROP_METADATA_DESCRIPTION_STRING);
            }

            if !write_netscape_loop_extension(io, loop_count as u16) {
                return Err(write_error(io));
            }

            if let Some(description) = description {
                if !write_comment_extension(io, description.as_bytes()) {
                    return Err(write_error(io));
                }
            }
        } else {
            if w != ctx.width as i32 || h != ctx.height as i32 {
                return Err(Error::new(format!(
                    "Frame dimensions ({}x{}) do not match GIF canvas dimensions ({}x{}).",
                    w, h, ctx.width, ctx.height
                )));
            }

            if ctx.use_lut {
                // For subsequent frames, map pixels to the existing global palette using the fast LUT.
                map_surface_to_existing_palette(
                    surface,
                    &ctx.color_map_lut,
                    &mut indexed_pixels,
                    ctx.transparent_color_index,
                )?;
            } else {
                // For subsequent frames, create a new optimal palette
                quantize_surface_to_indexed_pixels(
                    surface,
                    &mut local_color_table,
                    num_colors,
                    &mut indexed_pixels,
                    ctx.transparent_color_index,
                )?;
            }
        }

        let resolved_duration = e.encoder_duration(duration, 100) as u16;
        let io = e.dst();
        let disposal_method = if ctx.transparent_color_index != -1 {
            2
        } else {
            1
        };
        if !write_graphics_control_extension(
            io,
            resolved_duration,
            ctx.transparent_color_index,
            disposal_method,
        ) {
            return Err(write_error(io));
        }

        if ctx.use_lut {
            // Write image descriptor, indicating we are NOT using a local color table.
            if !write_image_descriptor(io, 0, 0, w as u16, h as u16, false, false, false, 0) {
                return Err(write_error(io));
            }
        } else {
            // Write image descriptor with local color table for non-first frames
            if !write_image_descriptor(
                io,
                0,
                0,
                w as u16,
                h as u16,
                use_local_color_table,
                false,
                false,
                if use_local_color_table {
                    palette_bits_per_pixel.wrapping_sub(1)
                } else {
                    0
                },
            ) {
                return Err(write_error(io));
            }

            // Write local color table for non-first frames
            if use_local_color_table {
                write_color_table(io, &local_color_table, num_colors)?;
            }
        }

        let lzw_min_code_size = palette_bits_per_pixel.max(2);

        let (compressed_data, compressed_size) = lzw_compress(
            &indexed_pixels,
            w as u16,
            h as u16,
            lzw_min_code_size,
            e.quality,
        )?;

        let io = e.dst();
        if !write_image_data(io, lzw_min_code_size, &compressed_data, compressed_size) {
            return Err(write_error(io));
        }

        if ctx.first_frame {
            ctx.first_frame = false;
        }

        Ok(())
    }

    /// Translation of `AnimationEncoder_End()`.
    pub(crate) fn end(&mut self, e: &mut EncoderCore<'_, '_>) -> Result<()> {
        self.metadata = None;

        if !write_gif_trailer(e.dst()) {
            return Err(Error::new("Failed to write GIF trailer."));
        }

        Ok(())
    }
}

/// Create the GIF encoder of an animation encoder: its colors and
/// transparent index from the properties
/// ([`PROP_ANIMATION_DECODER_CREATE_GIF_NUM_COLORS_NUMBER`](crate::PROP_ANIMATION_DECODER_CREATE_GIF_NUM_COLORS_NUMBER)
/// and the transparent color index, by default the last color), the
/// metadata (unless ignored) and whether later frames use the first frame's
/// palette ([`PROP_ANIMATION_ENCODER_CREATE_GIF_USE_LUT_BOOLEAN`](crate::PROP_ANIMATION_ENCODER_CREATE_GIF_USE_LUT_BOOLEAN)).
/// Translation of `IMG_CreateGIFAnimationEncoder()`.
pub(crate) fn create_gif_animation_encoder(
    e: &mut EncoderCore<'_, '_>,
    props: &Properties,
) -> Result<Box<GifEncoderContext>> {
    let mut transparent_index = props
        .get_number(
            crate::anim_decoder::PROP_ANIMATION_DECODER_CREATE_GIF_TRANSPARENT_COLOR_INDEX_NUMBER,
        )
        .unwrap_or(-1) as i32;
    let globalcolors = props
        .get_number(crate::anim_decoder::PROP_ANIMATION_DECODER_CREATE_GIF_NUM_COLORS_NUMBER)
        .unwrap_or(256);
    if globalcolors <= 1 || (globalcolors & (globalcolors - 1)) != 0 || globalcolors > 256 {
        return Err(Error::new(
            "GIF stream property 'num_colors' must be a power of 2 (starting from 2, up to 256).",
        ));
    }

    let num_global_colors = globalcolors as u16;

    if transparent_index >= num_global_colors as i32 {
        return Err(Error::new(format!(
            "Transparent color index {transparent_index} exceeds palette size {num_global_colors}"
        )));
    }

    if transparent_index < 0 {
        transparent_index = num_global_colors as i32 - 1;
    }

    if e.quality < 0 {
        e.quality = 75;
    } else if e.quality > 100 {
        e.quality = 100;
    }

    let ignore_props = props
        .get_bool(PROP_METADATA_IGNORE_PROPS_BOOLEAN)
        .unwrap_or(false);
    let mut metadata = None;
    if !ignore_props {
        let m = Properties::new();
        m.copy_from(props)?;
        metadata = Some(m);
    }

    Ok(Box::new(GifEncoderContext {
        width: 0,
        height: 0,
        global_color_table: [[0; 3]; 256],
        num_global_colors,
        transparent_color_index: transparent_index,
        first_frame: true,
        color_map_lut: Box::new([[[0; 32]; 32]; 32]),
        lut_initialized: false,
        use_lut: props
            .get_bool(crate::anim_encoder::PROP_ANIMATION_ENCODER_CREATE_GIF_USE_LUT_BOOLEAN)
            .unwrap_or(false),
        metadata,
    }))
}

/// Save a surface as a GIF image: quantized to 255 colors with an octree
/// (or an INDEX8 surface's palette, with the last index transparent).
/// Translation of `IMG_SaveGIF_IO()`.
pub fn save_gif_io(surface: &mut Surface<'_>, dst: &mut IoStream<'_>) -> Result<()> {
    verify_can_save_surface(surface)?;
    let mut encoder = AnimationEncoder::from_io(dst, "gif")?;

    if let Err(e) = encoder.add_frame(surface, 0) {
        let _ = encoder.close();
        return Err(e);
    }

    encoder.close()
}

/// Save a surface to a GIF file. Translation of `IMG_SaveGIF()`.
pub fn save_gif(surface: &mut Surface<'_>, file: impl AsRef<Path>) -> Result<()> {
    verify_can_save_surface(surface)?;
    let mut dst = IoStream::from_file(file, "wb")?;
    let result = save_gif_io(surface, &mut dst);
    let closed = dst.close();
    result.and(closed)
}
