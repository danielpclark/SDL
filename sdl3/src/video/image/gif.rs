// Rust translation of the GIF loader of stb_image.h v2.30 (Sean Barrett and
// contributors, http://nothings.org/stb; "GIF loader -- public domain by
// Jean-Marc Lienher -- simplified/shrunk by stb"), from the copy PlutoVG
// bundles (source/plutovg-stb-image.h in SDL_ttf's external/plutovg), which
// SDL's own copy compiles out (`STBI_ONLY_PNG`, `STBI_ONLY_JPEG`).
// stb_image is available under the MIT License or in the public domain
// (see LICENSE.txt).
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! GIF: the first frame (`stbi_load` never reads the others), always
//! four components.

use super::stb_image::{convert_format8, err, mad3sizes_valid, Context, Image, Pixels};
use crate::error::{Error, Result};

/// Translation of `stbi__gif_lzw`.
#[derive(Clone, Copy, Default)]
struct GifLzw {
    prefix: i16,
    first: u8,
    suffix: u8,
}

/// Which color table the frame uses (`stbi__gif::color_table`).
#[derive(Clone, Copy, PartialEq)]
enum ColorTable {
    Global,
    Local,
}

/// Translation of `stbi__gif`.
struct Gif {
    w: i32,
    h: i32,
    out: Vec<u8>,        // output buffer (always 4 components)
    background: Vec<u8>, // The current "background" as far as a gif is concerned
    history: Vec<u8>,
    flags: i32,
    bgindex: i32,
    #[allow(dead_code)]
    ratio: i32,
    transparent: i32,
    eflags: i32,
    pal: [[u8; 4]; 256],
    lpal: [[u8; 4]; 256],
    codes: Vec<GifLzw>,
    color_table: ColorTable,
    parse: i32,
    step: i32,
    lflags: i32,
    start_x: i32,
    start_y: i32,
    max_x: i32,
    max_y: i32,
    cur_x: i32,
    cur_y: i32,
    line_size: i32,
    #[allow(dead_code)]
    delay: i32,
}

impl Gif {
    /// `memset(&g, 0, sizeof(g))`.
    fn new() -> Gif {
        Gif {
            w: 0,
            h: 0,
            out: Vec::new(),
            background: Vec::new(),
            history: Vec::new(),
            flags: 0,
            bgindex: 0,
            ratio: 0,
            transparent: 0,
            eflags: 0,
            pal: [[0; 4]; 256],
            lpal: [[0; 4]; 256],
            codes: vec![GifLzw::default(); 8192],
            color_table: ColorTable::Global,
            parse: 0,
            step: 0,
            lflags: 0,
            start_x: 0,
            start_y: 0,
            max_x: 0,
            max_y: 0,
            cur_x: 0,
            cur_y: 0,
            line_size: 0,
            delay: 0,
        }
    }
}

/// Translation of `stbi__gif_test_raw()`.
fn gif_test_raw(s: &mut Context<'_>) -> bool {
    if s.get8() != b'G' || s.get8() != b'I' || s.get8() != b'F' || s.get8() != b'8' {
        return false;
    }
    let sz = s.get8();
    if sz != b'9' && sz != b'7' {
        return false;
    }
    if s.get8() != b'a' {
        return false;
    }
    true
}

/// Translation of `stbi__gif_test()`.
pub(crate) fn gif_test(s: &mut Context<'_>) -> bool {
    let r = gif_test_raw(s);
    s.rewind();
    r
}

/// Translation of `stbi__gif_parse_colortable()`.
fn gif_parse_colortable(
    s: &mut Context<'_>,
    pal: &mut [[u8; 4]; 256],
    num_entries: i32,
    transp: i32,
) {
    for (i, p) in pal.iter_mut().enumerate().take(num_entries as usize) {
        p[2] = s.get8();
        p[1] = s.get8();
        p[0] = s.get8();
        p[3] = if transp == i as i32 { 0 } else { 255 };
    }
}

/// Translation of `stbi__gif_header()`.
fn gif_header(s: &mut Context<'_>, g: &mut Gif, is_info: bool) -> Result<()> {
    if s.get8() != b'G' || s.get8() != b'I' || s.get8() != b'F' || s.get8() != b'8' {
        return err("Corrupt GIF");
    }

    let version = s.get8();
    if version != b'7' && version != b'9' {
        return err("Corrupt GIF");
    }
    if s.get8() != b'a' {
        return err("Corrupt GIF");
    }

    // (stbi__g_failure_reason = "")
    g.w = s.get16le();
    g.h = s.get16le();
    g.flags = s.get8() as i32;
    g.bgindex = s.get8() as i32;
    g.ratio = s.get8() as i32;
    g.transparent = -1;

    // (g->w and g->h are at most 65535: never larger than STBI_MAX_DIMENSIONS)

    // (*comp = 4: can't actually tell whether it's 3 or 4 until we parse the comments)

    if is_info {
        return Ok(());
    }

    if g.flags & 0x80 != 0 {
        gif_parse_colortable(s, &mut g.pal, 2 << (g.flags & 7), -1);
    }

    Ok(())
}

/// Translation of `stbi__out_gif_code()`. The C recurses to decode the
/// prefixes first (the linked list is backwards); this walks the chain
/// into a list and outputs it from its end, in the same order.
fn out_gif_code(g: &mut Gif, code: u16, chain: &mut Vec<u16>) {
    chain.clear();
    let mut c = code;
    chain.push(c);
    while g.codes[c as usize].prefix >= 0 {
        c = g.codes[c as usize].prefix as u16;
        chain.push(c);
    }

    while let Some(code) = chain.pop() {
        if g.cur_y >= g.max_y {
            continue;
        }

        let idx = (g.cur_x + g.cur_y) as usize;
        g.history[idx / 4] = 1;

        let suffix = g.codes[code as usize].suffix as usize;
        let c = match g.color_table {
            ColorTable::Global => g.pal[suffix],
            ColorTable::Local => g.lpal[suffix],
        };
        if c[3] > 128 {
            // don't render transparent pixels;
            let p = &mut g.out[idx..idx + 4];
            p[0] = c[2];
            p[1] = c[1];
            p[2] = c[0];
            p[3] = c[3];
        }
        g.cur_x += 4;

        if g.cur_x >= g.max_x {
            g.cur_x = g.start_x;
            g.cur_y += g.step;

            while g.cur_y >= g.max_y && g.parse > 0 {
                g.step = (1 << g.parse) * g.line_size;
                g.cur_y = g.start_y + (g.step >> 1);
                g.parse -= 1;
            }
        }
    }
}

/// Translation of `stbi__process_gif_raster()`: `Ok(false)` where the C
/// returns NULL without setting an error.
fn process_gif_raster(s: &mut Context<'_>, g: &mut Gif) -> Result<bool> {
    let lzw_cs = s.get8() as i32;
    if lzw_cs > 12 {
        return Ok(false);
    }
    let clear = 1 << lzw_cs;
    let mut first = true;
    let mut codesize = lzw_cs + 1;
    let mut codemask = (1 << codesize) - 1;
    let mut bits = 0i32;
    let mut valid_bits = 0;
    for init_code in 0..clear {
        let c = &mut g.codes[init_code as usize];
        c.prefix = -1;
        c.first = init_code as u8;
        c.suffix = init_code as u8;
    }

    // support no starting clear code
    let mut avail = clear + 2;
    let mut oldcode = -1;

    let mut chain = Vec::new();
    let mut len = 0;
    loop {
        if valid_bits < codesize {
            if len == 0 {
                len = s.get8() as i32; // start new block
                if len == 0 {
                    return Ok(true);
                }
            }
            len -= 1;
            bits |= (s.get8() as i32) << valid_bits;
            valid_bits += 8;
        } else {
            let code = bits & codemask;
            bits >>= codesize;
            valid_bits -= codesize;
            // @OPTIMIZE: is there some way we can accelerate the non-clear path?
            if code == clear {
                // clear code
                codesize = lzw_cs + 1;
                codemask = (1 << codesize) - 1;
                avail = clear + 2;
                oldcode = -1;
                first = false;
            } else if code == clear + 1 {
                // end of stream code
                s.skip(len);
                loop {
                    len = s.get8() as i32;
                    if len <= 0 {
                        break;
                    }
                    s.skip(len);
                }
                return Ok(true);
            } else if code <= avail {
                if first {
                    return err("Corrupt GIF");
                }

                if oldcode >= 0 {
                    let pi = avail as usize;
                    avail += 1;
                    if avail > 8192 {
                        return err("Corrupt GIF");
                    }

                    g.codes[pi].prefix = oldcode as i16;
                    g.codes[pi].first = g.codes[oldcode as usize].first;
                    // (code may be the entry just made: its first is set)
                    g.codes[pi].suffix = if code == avail {
                        g.codes[pi].first
                    } else {
                        g.codes[code as usize].first
                    };
                } else if code == avail {
                    return err("Corrupt GIF");
                }

                out_gif_code(g, code as u16, &mut chain);

                if (avail & codemask) == 0 && avail <= 0x0FFF {
                    codesize += 1;
                    codemask = (1 << codesize) - 1;
                }

                oldcode = code;
            } else {
                return err("Corrupt GIF");
            }
        }
    }
}

/// What `stbi__gif_load_next()` returns: a frame, or the end of the
/// stream (`(stbi_uc *) s`), or NULL without an error set.
enum Next {
    Frame,
    End,
    Null,
}

/// Translation of `stbi__gif_load_next()` (for the first frame:
/// stb_image's loader never reads the others).
fn gif_load_next(s: &mut Context<'_>, g: &mut Gif) -> Result<Next> {
    // on first frame, any non-written pixels get the background colour (non-transparent)
    gif_header(s, g, false)?; // stbi__g_failure_reason set by stbi__gif_header
    if !mad3sizes_valid(4, g.w, g.h, 0) {
        return err("GIF image is too large");
    }
    let pcount = (g.w * g.h) as usize;
    // image is treated as "transparent" at the start - ie, nothing overwrites the current background;
    // background colour is only used for pixels that are not rendered first frame, after that "background"
    // color refers to the color that was there the previous frame.
    g.out = vec![0; 4 * pcount];
    g.background = vec![0; 4 * pcount]; // state of the background (starts transparent)
    g.history = vec![0; pcount]; // pixels that were affected previous frame
    let first_frame = true;

    // clear my history;
    g.history.fill(0); // pixels that were affected previous frame

    loop {
        let tag = s.get8();
        match tag {
            0x2C => {
                /* Image Descriptor */
                let x = s.get16le();
                let y = s.get16le();
                let w = s.get16le();
                let h = s.get16le();
                if (x + w) > g.w || (y + h) > g.h {
                    return err("Corrupt GIF");
                }

                g.line_size = g.w * 4;
                g.start_x = x * 4;
                g.start_y = y * g.line_size;
                g.max_x = g.start_x + w * 4;
                g.max_y = g.start_y + h * g.line_size;
                g.cur_x = g.start_x;
                g.cur_y = g.start_y;

                // if the width of the specified rectangle is 0, that means
                // we may not see *any* pixels or the image is malformed;
                // to make sure this is caught, move the current y down to
                // max_y (which is what out_gif_code checks).
                if w == 0 {
                    g.cur_y = g.max_y;
                }

                g.lflags = s.get8() as i32;

                if g.lflags & 0x40 != 0 {
                    g.step = 8 * g.line_size; // first interlaced spacing
                    g.parse = 3;
                } else {
                    g.step = g.line_size;
                    g.parse = 0;
                }

                if g.lflags & 0x80 != 0 {
                    let transp = if g.eflags & 0x01 != 0 {
                        g.transparent
                    } else {
                        -1
                    };
                    gif_parse_colortable(s, &mut g.lpal, 2 << (g.lflags & 7), transp);
                    g.color_table = ColorTable::Local;
                } else if g.flags & 0x80 != 0 {
                    g.color_table = ColorTable::Global;
                } else {
                    return err("Corrupt GIF");
                }

                if !process_gif_raster(s, g)? {
                    return Ok(Next::Null);
                }

                // if this was the first frame,
                if first_frame && g.bgindex > 0 {
                    // if first frame, any pixel not drawn to gets the background color
                    for pi in 0..pcount {
                        if g.history[pi] == 0 {
                            g.pal[g.bgindex as usize][3] = 255; // just in case it was made transparent, undo that; It will be reset next frame if need be;
                            g.out[pi * 4..pi * 4 + 4].copy_from_slice(&g.pal[g.bgindex as usize]);
                        }
                    }
                }

                return Ok(Next::Frame);
            }

            0x21 => {
                // Comment Extension.
                let ext = s.get8();
                if ext == 0xF9 {
                    // Graphic Control Extension.
                    let len = s.get8() as i32;
                    if len == 4 {
                        g.eflags = s.get8() as i32;
                        g.delay = 10 * s.get16le(); // delay - 1/100th of a second, saving as 1/1000ths.

                        // unset old transparent
                        if g.transparent >= 0 {
                            g.pal[g.transparent as usize][3] = 255;
                        }
                        if g.eflags & 0x01 != 0 {
                            g.transparent = s.get8() as i32;
                            if g.transparent >= 0 {
                                g.pal[g.transparent as usize][3] = 0;
                            }
                        } else {
                            // don't need transparent
                            s.skip(1);
                            g.transparent = -1;
                        }
                    } else {
                        s.skip(len);
                        continue;
                    }
                }
                loop {
                    let len = s.get8() as i32;
                    if len == 0 {
                        break;
                    }
                    s.skip(len);
                }
            }

            0x3B => {
                // gif stream termination code
                return Ok(Next::End);
            }

            _ => return err("Corrupt GIF"),
        }
    }
}

/// Translation of `stbi__gif_load()`.
pub(crate) fn gif_load(s: &mut Context<'_>, req_comp: i32) -> Result<Image<Pixels>> {
    let mut g = Gif::new();

    match gif_load_next(s, &mut g)? {
        Next::Frame => {}
        // end of animated gif marker, or NULL: no frame, and no error set
        // (SDL keeps whatever error was already set)
        Next::End | Next::Null => return Err(Error::new("Corrupt GIF")),
    }
    let mut u = std::mem::take(&mut g.out);

    // moved conversion to after successful load so that the same
    // can be done for multiple frames.
    if req_comp != 0 && req_comp != 4 {
        u = convert_format8(u, 4, req_comp, g.w as u32, g.h as u32)?;
    }

    // free buffers needed for multiple frame loading;

    Ok(Image {
        x: g.w,
        y: g.h,
        comp: 4,
        data: Pixels::Bits8(u),
    })
}
