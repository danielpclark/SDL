// Rust translation of the decoder of src/IMG_gif.c from SDL_image.
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
//! Translated: the frame decoder (`IMG_CreateGIFAnimationDecoder()` and
//! its callbacks), [`is_gif`] and [`load_gif_io`], which reads the first
//! frame through it. The animation API around the decoder
//! (`IMG_anim_decoder.c`) and the GIF encoder (`IMG_SaveGIF_IO()` and the
//! animation encoder, with its LZW compressor and color quantizers) are
//! not translated yet; the part of `IMG_AnimationDecoder` this decoder uses
//! is in [`GifDecoder`].

// The loops index several arrays at once and the range checks are
// written as upstream's; both kept as written.
#![allow(
    clippy::explicit_counter_loop,
    clippy::manual_range_contains,
    clippy::needless_range_loop
)]

use sdl3::error::{Error, Result};
use sdl3::io::{IoStream, IoWhence};
use sdl3::properties::Properties;
use sdl3::video::{share_palette, Palette, PixelFormat, Rect, Surface};

use crate::util::read_ok;

/// `IMG_PROP_METADATA_LOOP_COUNT_NUMBER`.
const PROP_METADATA_LOOP_COUNT_NUMBER: &str = "SDL_image.metadata.loop_count";
/// `IMG_PROP_METADATA_DESCRIPTION_STRING`.
const PROP_METADATA_DESCRIPTION_STRING: &str = "SDL_image.metadata.description";

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

        let mut block = [0u8; 256];
        let ret = get_data_block(src, &mut block, state);
        let count = if ret > 0 {
            state.buf[2..2 + ret as usize].copy_from_slice(&block[..ret as usize]);
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

/// The status of a decoder. Translation of `IMG_AnimationDecoderStatus`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)] // for the animation API
pub(crate) enum DecoderStatus {
    Invalid,
    Ok,
    Failed,
    Complete,
}

/// Translation of `struct IMG_AnimationDecoderContext` (the GIF one).
#[allow(dead_code)] // the fields upstream keeps but doesn't read
struct GifContext {
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

/// A GIF frame decoder over a stream: the GIF context with the part of
/// `IMG_AnimationDecoder` it uses (its status, start position, time base
/// and accumulated presentation time, and its metadata properties).
pub(crate) struct GifDecoder<'s, 'a> {
    status: DecoderStatus,
    props: Properties,
    src: &'s mut IoStream<'a>,
    start: i64,
    timebase_numerator: i32,
    timebase_denominator: i32,
    accumulated_pts: u64,
    ctx: GifContext,
}

impl GifDecoder<'_, '_> {
    /// Translation of `IMG_GetDecoderDuration()`.
    fn decoder_duration(&mut self, duration: u64, timebase_denominator: u64) -> u64 {
        let value = crate::img::timebase_duration(
            self.accumulated_pts,
            duration,
            1,
            timebase_denominator,
            self.timebase_numerator as u64,
            self.timebase_denominator as u64,
        );
        self.accumulated_pts += duration;
        value
    }

    /// Read the GIF header (once): the screen descriptor, the global color
    /// map, and unless the properties are ignored, the loop count and
    /// comment from the extensions before the first image. Translation of
    /// `IMG_AnimationDecoderGetGIFHeader()`.
    fn get_gif_header(
        &mut self,
        mut comment: Option<&mut Option<String>>,
        mut loop_count: Option<&mut i64>,
    ) -> Result<()> {
        if let Some(comment) = comment.as_deref_mut() {
            *comment = None;
        }

        if let Some(loop_count) = loop_count.as_deref_mut() {
            *loop_count = 1;
        }

        let ctx = &mut self.ctx;
        let src = &mut *self.src;
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
    #[allow(dead_code)] // for the animation API
    pub(crate) fn reset(&mut self) -> Result<()> {
        if self.src.seek(self.start, IoWhence::Set).ok() != Some(self.start) {
            return Err(Error::new("Failed to seek to beginning of GIF file"));
        }

        let ctx = &mut self.ctx;
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

        self.get_gif_header(None, None)
    }

    /// Decode the next frame: in single-frame mode the frame itself when it
    /// covers the whole screen, otherwise the frame composited onto the
    /// canvas (with the previous frame's disposal applied). `Ok(None)` at
    /// the end of the file (the `COMPLETE` status). Translation of
    /// `IMG_AnimationDecoderGetNextFrame_Internal()`.
    fn get_next_frame(&mut self) -> Result<Option<(Surface<'static>, u64)>> {
        let mut frames_loaded = 0;

        if self.ctx.got_eof {
            self.status = DecoderStatus::Complete;
            return Ok(None);
        }

        self.get_gif_header(None, None)?;

        let frames_to_load = 1;
        let mut retval = None;
        let mut duration = 0;
        while frames_loaded < frames_to_load {
            let ctx = &mut self.ctx;
            let src = &mut *self.src;
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
                duration = self.decoder_duration(10, 100);
            } else {
                duration = self.decoder_duration(delay_time as u64, 100);
            }
            let ctx = &mut self.ctx;
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
            if self.ctx.got_eof {
                self.status = DecoderStatus::Complete;
                return Ok(None);
            }
            return Err(Error::new("Failed to load any frames"));
        }

        Ok(retval.map(|frame| (frame, duration)))
    }

    /// Create a GIF decoder reading from the stream's position, with the
    /// time base `timebase_numerator / timebase_denominator` seconds; for
    /// `single_frame`, frames are returned as they are decoded when they
    /// cover the screen, without a canvas. Translation of
    /// `IMG_CreateGIFAnimationDecoder()` (with the `IMG_AnimationDecoder`
    /// set-up of `IMG_CreateAnimationDecoderWithProperties()`).
    pub(crate) fn new<'s, 'a>(
        src: &'s mut IoStream<'a>,
        single_frame: bool,
        ignore_props: bool,
    ) -> Result<GifDecoder<'s, 'a>> {
        let start = src.tell().unwrap_or(-1);
        let mut state = State::zeroed();
        state.gif89.transparent = -1;
        state.gif89.delay_time = -1;
        state.gif89.input_flag = -1;
        state.gif89.disposal = GIF_DISPOSE_NA;

        let ctx = GifContext {
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
            single_frame,
            global_colormap: [[0; MAXCOLORMAPSIZE]; 3],
            global_colormap_size: 0,
            has_global_colormap: false,
            global_grayscale: false,
            last_duration: 0,
            ignore_props: false,
        };

        let mut decoder = GifDecoder {
            status: DecoderStatus::Ok,
            props: Properties::new(),
            src,
            start,
            timebase_numerator: 1,
            timebase_denominator: 1000,
            accumulated_pts: 0,
            ctx,
        };

        let mut comment = None;
        let mut loop_count = 1;
        decoder.get_gif_header(Some(&mut comment), Some(&mut loop_count))?;

        decoder.ctx.ignore_props = ignore_props;
        if !ignore_props {
            // Set well-defined properties.
            let _ = decoder
                .props
                .set(PROP_METADATA_LOOP_COUNT_NUMBER, loop_count);

            // Get other well-defined properties and set them in our props.
            if let Some(comment) = comment {
                let _ = decoder.props.set(PROP_METADATA_DESCRIPTION_STRING, comment);
            }
        }

        Ok(decoder)
    }

    /// Decode the next frame and its duration. Translation of
    /// `IMG_GetAnimationDecoderFrame()`: `Ok(None)` when the file is
    /// complete (where upstream returns false with the error cleared).
    pub(crate) fn get_frame(&mut self) -> Result<Option<(Surface<'static>, u64)>> {
        // Reset the status before trying to get the next frame
        self.status = DecoderStatus::Ok;

        let result = self.get_next_frame();
        if result.is_err() {
            self.status = DecoderStatus::Failed;
        }
        result
    }

    /// The decoder's metadata (the loop count and comment).
    #[allow(dead_code)] // for the animation API
    pub(crate) fn properties(&self) -> &Properties {
        &self.props
    }

    /// The decoder's status. Translation of `IMG_GetAnimationDecoderStatus()`.
    #[allow(dead_code)] // for the animation API
    pub(crate) fn status(&self) -> DecoderStatus {
        self.status
    }
}

impl std::fmt::Debug for GifDecoder<'_, '_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GifDecoder")
            .field("status", &self.status)
            .field("width", &self.ctx.width)
            .field("height", &self.ctx.height)
            .field("current_frame", &self.ctx.current_frame)
            .finish_non_exhaustive()
    }
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
    let start = src.tell().unwrap_or(-1);
    let mut decoder = match GifDecoder::new(src, true, false) {
        Ok(decoder) => decoder,
        Err(e) => {
            // FIXME (upstream): when the GIF decoder can't be created,
            // IMG_CreateAnimationDecoderWithProperties() falls back to its
            // single-frame decoder, which loads the image with
            // IMG_LoadTyped_IO(): that detects a GIF and calls
            // IMG_LoadGIF_IO() again, without end. Here the GIF decoder's
            // error is returned instead.
            let _ = src.seek(start, IoWhence::Set);
            return Err(e);
        }
    };

    let frame = decoder.get_frame();
    // (IMG_CloseAnimationDecoder(): the stream stays open)
    match frame? {
        Some((frame, _pts)) => Ok(frame),
        // Note (upstream): NULL with the error cleared.
        None => Err(Error::new("GIF file contains no images")),
    }
}
