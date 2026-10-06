// Rust translation of src/IMG_xpm.c from SDL_image.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! XPM (X PixMap) image loader:
//!
//! Supports the XPMv3 format, EXCEPT:
//! - hotspot coordinates are ignored
//! - only colour ('c') colour symbols are used
//! - rgb.txt is not used (for portability), so only RGB colours
//!   are recognized (#rrggbb etc) - only a few basic colour names are
//!   handled
//!
//! The result is an 8bpp indexed surface if possible, otherwise 32bpp.
//! The colourkey is correctly set if transparency is used.
//!
//! Besides the standard API, also provides
//!
//! * [`read_xpm_from_array`]
//! * [`read_xpm_from_array_to_rgb888`]
//!
//! that read the image data from an XPM file included in the source.
//! - 1st function returns an 8bpp indexed surface if possible, otherwise 32bpp.
//! - 2nd function returns always a 32bpp (RGB888) surface
//!
//! TODO: include rgb.txt here. The full table (from solaris 2.6) only
//! requires about 13K in binary form.

// The pixel loops index the row and the line at once, as upstream's do;
// the line length is set on both branches, as written there.
#![allow(clippy::needless_range_loop, clippy::needless_late_init)]

use std::collections::HashMap;

use sdl3::error::{Error, Result};
use sdl3::io::{IoStream, IoWhence};
use sdl3::stdlib::string::{strncasecmp, strtol};
use sdl3::video::{share_palette, Palette, PixelFormat, Surface};

use crate::util::{isspace, read_byte, read_ok, read_up_to, scan_ints};

/* See if an image is contained in a data source */

/// Whether `src` holds an XPM image (starting with `/* XPM */`); the
/// stream position is unchanged. Translation of `IMG_isXPM()`.
pub fn is_xpm(src: &mut IoStream<'_>) -> bool {
    let start = src.tell().unwrap_or(-1);
    let mut is_xpm = false;
    let mut magic = [0u8; 9];
    if read_ok(src, &mut magic) && &magic == b"/* XPM */" {
        is_xpm = true;
    }
    let _ = src.seek(start, IoWhence::Set);
    is_xpm
}

/* Hash table to look up colors from pixel strings */
const STARTING_HASH_SIZE: i32 = 256;

/// Translation of `struct hash_entry`: the key is an offset into the key
/// strings, `next` an index into the entries.
struct HashEntry {
    key: usize,
    color: u32,
    next: Option<usize>,
}

/// Translation of `struct color_hash`. The table holds the head of each
/// bucket's chain (a map instead of an array, so that a corrupt color
/// count costs no memory until the colors are there).
struct ColorHash {
    table: HashMap<i32, usize>,
    entries: Vec<HashEntry>, /* array of all entries */
    size: i32,
    _maxnum: i32,
}

/// Translation of `hash_key()`: the key's bytes are C `char`s, signed or
/// not as the platform's are.
fn hash_key(key: impl Fn(usize) -> u8, cpp: i32, size: i32) -> i32 {
    let mut hash: i32 = 0;
    for i in 0..cpp.max(0) as usize {
        hash = hash
            .wrapping_mul(33)
            .wrapping_add(key(i) as std::ffi::c_char as i32);
    }
    hash & (size - 1)
}

/// Translation of `create_colorhash()` (`None` for an allocation
/// failure).
fn create_colorhash(maxnum: i32) -> Option<ColorHash> {
    /* we know how many entries we need, so we can allocate
    everything here */

    /* use power-of-2 sized hash table for decoding speed */
    // FIXME (upstream): for more than 2^30 colors the size overflows and
    // the loop doesn't end; refused here.
    if maxnum > 1 << 30 {
        return None;
    }
    let mut s = STARTING_HASH_SIZE;
    while s < maxnum {
        s <<= 1;
    }
    let size = s;

    let bytes = size.wrapping_mul(std::mem::size_of::<usize>() as i32);
    /* Check for overflow */
    if (bytes as isize as usize / std::mem::size_of::<usize>()) as u32 != size as u32 {
        // (SDL_SetError("memory allocation overflow"), which the caller
        // replaces)
        return None;
    }

    // (struct hash_entry is two pointers and a Uint32: 24 bytes)
    let bytes = maxnum.wrapping_mul(24);
    /* Check for overflow */
    if (bytes as isize as usize / 24) as u32 != maxnum as u32 {
        return None;
    }
    let mut entries = Vec::new();
    entries.try_reserve_exact(maxnum as usize).ok()?;
    Some(ColorHash {
        table: HashMap::new(),
        entries,
        size,
        _maxnum: maxnum,
    })
}

impl ColorHash {
    /// Translation of `add_colorhash()`: `key` is at `key_at` in
    /// `keystrings`.
    fn add(&mut self, keystrings: &[u8], key_at: usize, cpp: i32, color: u32) {
        let index = hash_key(|i| keystrings[key_at + i], cpp, self.size);
        let e = self.entries.len();
        self.entries.push(HashEntry {
            key: key_at,
            color,
            next: self.table.get(&index).copied(),
        });
        self.table.insert(index, e);
    }

    /// Translation of `get_colorhash()`.
    fn get(&self, keystrings: &[u8], key: impl Fn(usize) -> u8, cpp: i32) -> u32 {
        let mut entry = self.table.get(&hash_key(&key, cpp, self.size)).copied();
        while let Some(e) = entry {
            let e = &self.entries[e];
            if (0..cpp as usize).all(|i| key(i) == keystrings[e.key + i]) {
                return e.color;
            }
            entry = e.next;
        }
        0 /* garbage in - garbage out */
    }
}

/// Convert colour spec to RGB (in 0xaarrggbb format).
/// Translation of `color_to_argb()` (`None` for 0).
fn color_to_argb(spec: &[u8]) -> Option<u32> {
    let speclen = spec.len();
    if spec.first() == Some(&b'#') {
        let mut buf = [0u8; 7];
        match speclen {
            4 => {
                buf[0] = spec[1];
                buf[1] = spec[1];
                buf[2] = spec[2];
                buf[3] = spec[2];
                buf[4] = spec[3];
                buf[5] = spec[3];
            }
            7 => buf[..6].copy_from_slice(&spec[1..7]),
            13 => {
                buf[0] = spec[1];
                buf[1] = spec[2];
                buf[2] = spec[5];
                buf[3] = spec[6];
                buf[4] = spec[9];
                buf[5] = spec[10];
            }
            // FIXME (upstream): for other lengths the buffer is left
            // uninitialized; it is empty here (and the color black).
            _ => {}
        }
        buf[6] = b'\0';
        let len = buf.iter().position(|&b| b == 0).unwrap_or(6);
        Some(0xff000000 | strtol(&buf[..len], 16).0 as u32)
    } else {
        for &(name, argb) in KNOWN {
            if strncasecmp(name, spec, speclen).is_eq() {
                return Some(argb);
            }
        }
        None
    }
}

/// How the loader fails: with one of its messages (`error`, after which
/// the stream is rewound), or with an error SDL already set.
enum Failure {
    Error(&'static str),
    Sdl(Error),
}

/// Where the lines come from: a data source, or an array of strings.
enum Lines<'s, 'a, 'x> {
    Io(&'s mut IoStream<'a>),
    Array(std::slice::Iter<'x, &'x str>),
}

/// The line reader's state (upstream's static `linebuf`, `buflen` and
/// `error`).
struct LineReader<'s, 'a, 'x> {
    lines: Lines<'s, 'a, 'x>,
    linebuf: Vec<u8>,
}

impl LineReader<'_, '_, '_> {
    /// Read next line from the source.
    /// If len > 0, it's assumed to be at least len chars (for efficiency).
    /// Return NULL and set error upon EOF or parse error.
    ///
    /// Translation of `get_next_line()`: the line, without the C string's
    /// terminating NUL (it may hold NULs of its own).
    fn get_next_line(&mut self, len: usize) -> std::result::Result<&[u8], Failure> {
        let src = match &mut self.lines {
            Lines::Array(lines) => {
                // FIXME (upstream): an array with too few lines is read past
                // its end; that is an error here.
                return match lines.next() {
                    Some(line) => Ok(line.as_bytes()),
                    None => Err(Failure::Error("Premature end of data")),
                };
            }
            Lines::Io(src) => src,
        };
        loop {
            match read_byte(src) {
                None => return Err(Failure::Error("Premature end of data")),
                Some(b'"') => break,
                Some(_) => {}
            }
        }
        let n;
        if len != 0 {
            let len = len.wrapping_add(3); /* "\",\n" */
            let Some(line) = read_up_to(src, len) else {
                return Err(Failure::Error("Out of memory"));
            };
            if line.len() != len {
                return Err(Failure::Error("Premature end of data"));
            }
            self.linebuf = line;
            n = len.wrapping_sub(1);
        } else {
            self.linebuf.clear();
            loop {
                let Some(c) = read_byte(src) else {
                    return Err(Failure::Error("Premature end of data"));
                };
                self.linebuf.push(c);
                if c == b'"' {
                    break;
                }
            }
            n = self.linebuf.len() - 1;
        }
        self.linebuf.truncate(n);
        Ok(&self.linebuf)
    }
}

/// `SDL_strlen()`: the C string's length.
fn strlen(s: &[u8]) -> usize {
    s.iter().position(|&b| b == 0).unwrap_or(s.len())
}

/* read XPM from either array or IOStream */
/// Translation of `load_xpm()`.
fn load_xpm(
    lines: Lines<'_, '_, '_>,
    force_32bit: bool,
) -> std::result::Result<Surface<'static>, Failure> {
    use Failure::Error as E;

    let mut reader = LineReader {
        lines,
        linebuf: Vec::new(),
    };

    let line = reader.get_next_line(0)?;
    /*
     * The header string of an XPMv3 image has the format
     *
     * <width> <height> <ncolors> <cpp> [ <hotspot_x> <hotspot_y> ]
     *
     * where the hotspot coords are intended for mouse cursors.
     * Right now we don't use the hotspots but it should be handled
     * one day.
     */
    let mut header = [0i32; 4];
    if scan_ints(&line[..strlen(line)], &mut header) != 4
        || header[0] <= 0
        || header[1] <= 0
        || header[2] <= 0
        || header[3] <= 0
    {
        return Err(E("Invalid format description"));
    }
    let [w, h, mut ncolors, cpp] = header;

    /* Check for allocation overflow */
    if ((ncolors as u32).wrapping_mul(cpp as u32) as usize / cpp as usize) as u32 != ncolors as u32
    {
        return Err(E("Invalid color specification"));
    }
    // (an int product: past INT_MAX, a size SDL_malloc() can't have)
    let key_bytes = ncolors.wrapping_mul(cpp);
    let mut keystrings: Vec<u8> = Vec::new();
    if key_bytes < 0 || keystrings.try_reserve_exact(key_bytes as usize).is_err() {
        return Err(E("Out of memory"));
    }

    /* Create the new surface */
    let indexed;
    let mut image;
    let mut im_colors = None;
    if ncolors <= 256 && !force_32bit {
        indexed = true;
        image = Surface::new(w, h, PixelFormat::INDEX8).map_err(Failure::Sdl)?;
        let Ok(palette) = image.create_palette() else {
            return Err(E("Couldn't create palette"));
        };
        let palette_ncolors = palette.read().unwrap_or_else(|e| e.into_inner()).len() as i32;
        if ncolors > palette_ncolors {
            ncolors = palette_ncolors;
        }
        // (`palette->ncolors = ncolors`)
        let Ok(palette) = Palette::new(ncolors as usize) else {
            return Err(E("Couldn't create palette"));
        };
        let palette = share_palette(palette);
        if image.set_palette(Some(palette.clone())).is_err() {
            return Err(E("Couldn't create palette"));
        }
        im_colors = Some(palette);
    } else {
        indexed = false;
        /* Hmm, some SDL error (out of memory?) */
        image = Surface::new(w, h, PixelFormat::ARGB8888).map_err(Failure::Sdl)?;
    }

    /* Read the colors */
    let Some(mut colors) = create_colorhash(ncolors) else {
        return Err(E("Out of memory"));
    };
    for index in 0..ncolors {
        let line = reader.get_next_line(0)?;
        let line = &line[..strlen(line)];
        let at = |i: usize| line.get(i).copied().unwrap_or(0);

        let mut p = cpp as usize + 1;
        if p >= line.len() {
            return Err(E("Invalid color specification"));
        }

        /* parse a colour definition */
        loop {
            while isspace(at(p)) {
                p += 1;
            }
            if at(p) == 0 {
                return Err(E("colour parse error"));
            }
            let nametype = at(p);
            while !isspace(at(p)) && at(p) != 0 {
                p += 1;
            }
            while isspace(at(p)) {
                p += 1;
            }
            let colname = p;
            while !isspace(at(p)) && at(p) != 0 {
                p += 1;
            }
            if nametype == b's' {
                continue; /* skip symbolic colour names */
            }

            let Some(argb) = color_to_argb(&line[colname..p]) else {
                continue;
            };

            let nextkey = keystrings.len();
            keystrings.extend_from_slice(&line[..cpp as usize]);
            let pixelvalue;
            if indexed {
                if let Some(palette) = &im_colors {
                    let mut palette = palette.write().unwrap_or_else(|e| e.into_inner());
                    let c = &mut palette.colors_mut()[index as usize];
                    c.a = (argb >> 24) as u8;
                    c.r = (argb >> 16) as u8;
                    c.g = (argb >> 8) as u8;
                    c.b = argb as u8;
                }
                pixelvalue = index as u32;
                if argb == 0x00000000 {
                    let _ = image.set_color_key(Some(pixelvalue));
                }
            } else {
                pixelvalue = argb;
            }
            colors.add(&keystrings, nextkey, cpp, pixelvalue);
            break;
        }
    }

    /* Read the pixels */
    let pixels_len = w.wrapping_mul(cpp) as isize as usize;
    let pitch = image.pitch() as usize;
    let dst_pixels: &mut [u8] = image.pixels_mut().unwrap_or(&mut []);
    let cpp_u = cpp as usize;
    for y in 0..h as usize {
        let line = reader.get_next_line(pixels_len)?;
        // FIXME (upstream): where the product of the width and the
        // characters per pixel overflows, or an array's line is short, the
        // pixels are read past the line; those read as NUL here.
        let at = |i: usize| line.get(i).copied().unwrap_or(0);
        let dst = &mut dst_pixels[y * pitch..];

        if indexed {
            /* optimization for some common cases */
            if cpp == 1 {
                for x in 0..w as usize {
                    /* fast lookup that works if cpp == 1 */
                    let entry = colors.table.get(&(at(x) as i32));
                    dst[x] = entry.map_or(0, |&e| colors.entries[e].color as u8);
                }
            } else {
                for x in 0..w as usize {
                    dst[x] = colors.get(&keystrings, |i| at(x * cpp_u + i), cpp) as u8;
                }
            }
        } else {
            for x in 0..w as usize {
                let color = colors.get(&keystrings, |i| at(x * cpp_u + i), cpp);
                dst[x * 4..x * 4 + 4].copy_from_slice(&color.to_ne_bytes());
            }
        }
    }

    Ok(image)
}

/// Load an XPM image: an INDEX8 surface for up to 256 colors (with a color
/// key for the transparent one), otherwise ARGB8888.
/// Translation of `IMG_LoadXPM_IO()`.
pub fn load_xpm_io(src: &mut IoStream<'_>) -> Result<Surface<'static>> {
    let start = src.tell().unwrap_or(-1);
    match load_xpm(Lines::Io(src), false) {
        Ok(image) => Ok(image),
        Err(Failure::Error(error)) => {
            let _ = src.seek(start, IoWhence::Set);
            Err(Error::new(error))
        }
        Err(Failure::Sdl(e)) => Err(e),
    }
}

fn load_xpm_array(xpm: &[&str], force_32bit: bool) -> Result<Surface<'static>> {
    match load_xpm(Lines::Array(xpm.iter()), force_32bit) {
        Ok(image) => Ok(image),
        Err(Failure::Error(error)) => Err(Error::new(error)),
        Err(Failure::Sdl(e)) => Err(e),
    }
}

/// Load an XPM image from the strings of an XPM file included in the
/// source (the C array's strings, without their quotes): an INDEX8
/// surface for up to 256 colors, otherwise ARGB8888.
/// Translation of `IMG_ReadXPMFromArray()`.
pub fn read_xpm_from_array(xpm: &[&str]) -> Result<Surface<'static>> {
    load_xpm_array(xpm, false)
}

/// Load an XPM image from the strings of an XPM file included in the
/// source, always as an ARGB8888 surface.
/// Translation of `IMG_ReadXPMFromArrayToRGB888()`.
pub fn read_xpm_from_array_to_rgb888(xpm: &[&str]) -> Result<Surface<'static>> {
    load_xpm_array(xpm, true)
}

/// Translation of `color_to_argb()`'s `known` table: poor man's rgb.txt.
#[rustfmt::skip]
static KNOWN: &[(&str, u32)] = &[
    ("none", 0x00000000),
    ("black", 0xff000000),
    ("white", 0xffffffff),
    ("red", 0xffff0000),
    ("green", 0xff00ff00),
    ("blue", 0xff0000ff),
    // #ifndef DISABLE_EXTENDED_XPM_COLORS
    ("aliceblue", 0xfff0f8ff),
    ("antiquewhite", 0xfffaebd7),
    ("antiquewhite1", 0xffffefdb),
    ("antiquewhite2", 0xffeedfcc),
    ("antiquewhite3", 0xffcdc0b0),
    ("antiquewhite4", 0xff8b8378),
    ("aqua", 0xff00ffff),
    ("aquamarine", 0xff7fffd4),
    ("aquamarine1", 0xff7fffd4),
    ("aquamarine2", 0xff76eec6),
    ("aquamarine3", 0xff66cdaa),
    ("aquamarine4", 0xff458b74),
    ("azure", 0xfff0ffff),
    ("azure1", 0xfff0ffff),
    ("azure2", 0xffe0eeee),
    ("azure3", 0xffc1cdcd),
    ("azure4", 0xff838b8b),
    ("beige", 0xfff5f5dc),
    ("bisque", 0xffffe4c4),
    ("bisque1", 0xffffe4c4),
    ("bisque2", 0xffeed5b7),
    ("bisque3", 0xffcdb79e),
    ("bisque4", 0xff8b7d6b),
    ("black", 0xff000000),
    ("blanchedalmond", 0xffffebcd),
    ("blue", 0xff0000ff),
    ("blue1", 0xff0000ff),
    ("blue2", 0xff0000ee),
    ("blue3", 0xff0000cd),
    ("blue4", 0xff00008b),
    ("blueviolet", 0xff8a2be2),
    ("brown", 0xffa52a2a),
    ("brown1", 0xffff4040),
    ("brown2", 0xffee3b3b),
    ("brown3", 0xffcd3333),
    ("brown4", 0xff8b2323),
    ("burlywood", 0xffdeb887),
    ("burlywood1", 0xffffd39b),
    ("burlywood2", 0xffeec591),
    ("burlywood3", 0xffcdaa7d),
    ("burlywood4", 0xff8b7355),
    ("cadetblue", 0xff5f9ea0),
    ("cadetblue", 0xff5f9ea0),
    ("cadetblue1", 0xff98f5ff),
    ("cadetblue2", 0xff8ee5ee),
    ("cadetblue3", 0xff7ac5cd),
    ("cadetblue4", 0xff53868b),
    ("chartreuse", 0xff7fff00),
    ("chartreuse1", 0xff7fff00),
    ("chartreuse2", 0xff76ee00),
    ("chartreuse3", 0xff66cd00),
    ("chartreuse4", 0xff458b00),
    ("chocolate", 0xffd2691e),
    ("chocolate1", 0xffff7f24),
    ("chocolate2", 0xffee7621),
    ("chocolate3", 0xffcd661d),
    ("chocolate4", 0xff8b4513),
    ("coral", 0xffff7f50),
    ("coral1", 0xffff7256),
    ("coral2", 0xffee6a50),
    ("coral3", 0xffcd5b45),
    ("coral4", 0xff8b3e2f),
    ("cornflowerblue", 0xff6495ed),
    ("cornsilk", 0xfffff8dc),
    ("cornsilk1", 0xfffff8dc),
    ("cornsilk2", 0xffeee8cd),
    ("cornsilk3", 0xffcdc8b1),
    ("cornsilk4", 0xff8b8878),
    ("crimson", 0xffdc143c),
    ("cyan", 0xff00ffff),
    ("cyan1", 0xff00ffff),
    ("cyan2", 0xff00eeee),
    ("cyan3", 0xff00cdcd),
    ("cyan4", 0xff008b8b),
    ("darkblue", 0xff00008b),
    ("darkcyan", 0xff008b8b),
    ("darkgoldenrod", 0xffb8860b),
    ("darkgoldenrod1", 0xffffb90f),
    ("darkgoldenrod2", 0xffeead0e),
    ("darkgoldenrod3", 0xffcd950c),
    ("darkgoldenrod4", 0xff8b6508),
    ("darkgray", 0xffa9a9a9),
    ("darkgreen", 0xff006400),
    ("darkgrey", 0xffa9a9a9),
    ("darkkhaki", 0xffbdb76b),
    ("darkmagenta", 0xff8b008b),
    ("darkolivegreen", 0xff556b2f),
    ("darkolivegreen1", 0xffcaff70),
    ("darkolivegreen2", 0xffbcee68),
    ("darkolivegreen3", 0xffa2cd5a),
    ("darkolivegreen4", 0xff6e8b3d),
    ("darkorange", 0xffff8c00),
    ("darkorange1", 0xffff7f00),
    ("darkorange2", 0xffee7600),
    ("darkorange3", 0xffcd6600),
    ("darkorange4", 0xff8b4500),
    ("darkorchid", 0xff9932cc),
    ("darkorchid1", 0xffbf3eff),
    ("darkorchid2", 0xffb23aee),
    ("darkorchid3", 0xff9a32cd),
    ("darkorchid4", 0xff68228b),
    ("darkred", 0xff8b0000),
    ("darksalmon", 0xffe9967a),
    ("darkseagreen", 0xff8fbc8f),
    ("darkseagreen1", 0xffc1ffc1),
    ("darkseagreen2", 0xffb4eeb4),
    ("darkseagreen3", 0xff9bcd9b),
    ("darkseagreen4", 0xff698b69),
    ("darkslateblue", 0xff483d8b),
    ("darkslategray", 0xff2f4f4f),
    ("darkslategray1", 0xff97ffff),
    ("darkslategray2", 0xff8deeee),
    ("darkslategray3", 0xff79cdcd),
    ("darkslategray4", 0xff528b8b),
    ("darkslategrey", 0xff2f4f4f),
    ("darkturquoise", 0xff00ced1),
    ("darkviolet", 0xff9400d3),
    ("darkviolet", 0xff9400d3),
    ("deeppink", 0xffff1493),
    ("deeppink1", 0xffff1493),
    ("deeppink2", 0xffee1289),
    ("deeppink3", 0xffcd1076),
    ("deeppink4", 0xff8b0a50),
    ("deepskyblue", 0xff00bfff),
    ("deepskyblue1", 0xff00bfff),
    ("deepskyblue2", 0xff00b2ee),
    ("deepskyblue3", 0xff009acd),
    ("deepskyblue4", 0xff00688b),
    ("dimgray", 0xff696969),
    ("dimgrey", 0xff696969),
    ("dodgerblue", 0xff1e90ff),
    ("dodgerblue1", 0xff1e90ff),
    ("dodgerblue2", 0xff1c86ee),
    ("dodgerblue3", 0xff1874cd),
    ("dodgerblue4", 0xff104e8b),
    ("firebrick", 0xffb22222),
    ("firebrick1", 0xffff3030),
    ("firebrick2", 0xffee2c2c),
    ("firebrick3", 0xffcd2626),
    ("firebrick4", 0xff8b1a1a),
    ("floralwhite", 0xfffffaf0),
    ("forestgreen", 0xff228b22),
    ("fractal", 0xff808080),
    ("fuchsia", 0xffff00ff),
    ("gainsboro", 0xffdcdcdc),
    ("ghostwhite", 0xfff8f8ff),
    ("gold", 0xffffd700),
    ("gold1", 0xffffd700),
    ("gold2", 0xffeec900),
    ("gold3", 0xffcdad00),
    ("gold4", 0xff8b7500),
    ("goldenrod", 0xffdaa520),
    ("goldenrod1", 0xffffc125),
    ("goldenrod2", 0xffeeb422),
    ("goldenrod3", 0xffcd9b1d),
    ("goldenrod4", 0xff8b6914),
    ("gray", 0xff7e7e7e),
    ("gray", 0xffbebebe),
    ("gray0", 0xff000000),
    ("gray1", 0xff030303),
    ("gray10", 0xff1a1a1a),
    ("gray100", 0xffffffff),
    ("gray11", 0xff1c1c1c),
    ("gray12", 0xff1f1f1f),
    ("gray13", 0xff212121),
    ("gray14", 0xff242424),
    ("gray15", 0xff262626),
    ("gray16", 0xff292929),
    ("gray17", 0xff2b2b2b),
    ("gray18", 0xff2e2e2e),
    ("gray19", 0xff303030),
    ("gray2", 0xff050505),
    ("gray20", 0xff333333),
    ("gray21", 0xff363636),
    ("gray22", 0xff383838),
    ("gray23", 0xff3b3b3b),
    ("gray24", 0xff3d3d3d),
    ("gray25", 0xff404040),
    ("gray26", 0xff424242),
    ("gray27", 0xff454545),
    ("gray28", 0xff474747),
    ("gray29", 0xff4a4a4a),
    ("gray3", 0xff080808),
    ("gray30", 0xff4d4d4d),
    ("gray31", 0xff4f4f4f),
    ("gray32", 0xff525252),
    ("gray33", 0xff545454),
    ("gray34", 0xff575757),
    ("gray35", 0xff595959),
    ("gray36", 0xff5c5c5c),
    ("gray37", 0xff5e5e5e),
    ("gray38", 0xff616161),
    ("gray39", 0xff636363),
    ("gray4", 0xff0a0a0a),
    ("gray40", 0xff666666),
    ("gray41", 0xff696969),
    ("gray42", 0xff6b6b6b),
    ("gray43", 0xff6e6e6e),
    ("gray44", 0xff707070),
    ("gray45", 0xff737373),
    ("gray46", 0xff757575),
    ("gray47", 0xff787878),
    ("gray48", 0xff7a7a7a),
    ("gray49", 0xff7d7d7d),
    ("gray5", 0xff0d0d0d),
    ("gray50", 0xff7f7f7f),
    ("gray51", 0xff828282),
    ("gray52", 0xff858585),
    ("gray53", 0xff878787),
    ("gray54", 0xff8a8a8a),
    ("gray55", 0xff8c8c8c),
    ("gray56", 0xff8f8f8f),
    ("gray57", 0xff919191),
    ("gray58", 0xff949494),
    ("gray59", 0xff969696),
    ("gray6", 0xff0f0f0f),
    ("gray60", 0xff999999),
    ("gray61", 0xff9c9c9c),
    ("gray62", 0xff9e9e9e),
    ("gray63", 0xffa1a1a1),
    ("gray64", 0xffa3a3a3),
    ("gray65", 0xffa6a6a6),
    ("gray66", 0xffa8a8a8),
    ("gray67", 0xffababab),
    ("gray68", 0xffadadad),
    ("gray69", 0xffb0b0b0),
    ("gray7", 0xff121212),
    ("gray70", 0xffb3b3b3),
    ("gray71", 0xffb5b5b5),
    ("gray72", 0xffb8b8b8),
    ("gray73", 0xffbababa),
    ("gray74", 0xffbdbdbd),
    ("gray75", 0xffbfbfbf),
    ("gray76", 0xffc2c2c2),
    ("gray77", 0xffc4c4c4),
    ("gray78", 0xffc7c7c7),
    ("gray79", 0xffc9c9c9),
    ("gray8", 0xff141414),
    ("gray80", 0xffcccccc),
    ("gray81", 0xffcfcfcf),
    ("gray82", 0xffd1d1d1),
    ("gray83", 0xffd4d4d4),
    ("gray84", 0xffd6d6d6),
    ("gray85", 0xffd9d9d9),
    ("gray86", 0xffdbdbdb),
    ("gray87", 0xffdedede),
    ("gray88", 0xffe0e0e0),
    ("gray89", 0xffe3e3e3),
    ("gray9", 0xff171717),
    ("gray90", 0xffe5e5e5),
    ("gray91", 0xffe8e8e8),
    ("gray92", 0xffebebeb),
    ("gray93", 0xffededed),
    ("gray94", 0xfff0f0f0),
    ("gray95", 0xfff2f2f2),
    ("gray96", 0xfff5f5f5),
    ("gray97", 0xfff7f7f7),
    ("gray98", 0xfffafafa),
    ("gray99", 0xfffcfcfc),
    ("green", 0xff008000),
    ("green", 0xff00ff00),
    ("green1", 0xff00ff00),
    ("green2", 0xff00ee00),
    ("green3", 0xff00cd00),
    ("green4", 0xff008b00),
    ("greenyellow", 0xffadff2f),
    ("grey", 0xffbebebe),
    ("grey0", 0xff000000),
    ("grey1", 0xff030303),
    ("grey10", 0xff1a1a1a),
    ("grey100", 0xffffffff),
    ("grey11", 0xff1c1c1c),
    ("grey12", 0xff1f1f1f),
    ("grey13", 0xff212121),
    ("grey14", 0xff242424),
    ("grey15", 0xff262626),
    ("grey16", 0xff292929),
    ("grey17", 0xff2b2b2b),
    ("grey18", 0xff2e2e2e),
    ("grey19", 0xff303030),
    ("grey2", 0xff050505),
    ("grey20", 0xff333333),
    ("grey21", 0xff363636),
    ("grey22", 0xff383838),
    ("grey23", 0xff3b3b3b),
    ("grey24", 0xff3d3d3d),
    ("grey25", 0xff404040),
    ("grey26", 0xff424242),
    ("grey27", 0xff454545),
    ("grey28", 0xff474747),
    ("grey29", 0xff4a4a4a),
    ("grey3", 0xff080808),
    ("grey30", 0xff4d4d4d),
    ("grey31", 0xff4f4f4f),
    ("grey32", 0xff525252),
    ("grey33", 0xff545454),
    ("grey34", 0xff575757),
    ("grey35", 0xff595959),
    ("grey36", 0xff5c5c5c),
    ("grey37", 0xff5e5e5e),
    ("grey38", 0xff616161),
    ("grey39", 0xff636363),
    ("grey4", 0xff0a0a0a),
    ("grey40", 0xff666666),
    ("grey41", 0xff696969),
    ("grey42", 0xff6b6b6b),
    ("grey43", 0xff6e6e6e),
    ("grey44", 0xff707070),
    ("grey45", 0xff737373),
    ("grey46", 0xff757575),
    ("grey47", 0xff787878),
    ("grey48", 0xff7a7a7a),
    ("grey49", 0xff7d7d7d),
    ("grey5", 0xff0d0d0d),
    ("grey50", 0xff7f7f7f),
    ("grey51", 0xff828282),
    ("grey52", 0xff858585),
    ("grey53", 0xff878787),
    ("grey54", 0xff8a8a8a),
    ("grey55", 0xff8c8c8c),
    ("grey56", 0xff8f8f8f),
    ("grey57", 0xff919191),
    ("grey58", 0xff949494),
    ("grey59", 0xff969696),
    ("grey6", 0xff0f0f0f),
    ("grey60", 0xff999999),
    ("grey61", 0xff9c9c9c),
    ("grey62", 0xff9e9e9e),
    ("grey63", 0xffa1a1a1),
    ("grey64", 0xffa3a3a3),
    ("grey65", 0xffa6a6a6),
    ("grey66", 0xffa8a8a8),
    ("grey67", 0xffababab),
    ("grey68", 0xffadadad),
    ("grey69", 0xffb0b0b0),
    ("grey7", 0xff121212),
    ("grey70", 0xffb3b3b3),
    ("grey71", 0xffb5b5b5),
    ("grey72", 0xffb8b8b8),
    ("grey73", 0xffbababa),
    ("grey74", 0xffbdbdbd),
    ("grey75", 0xffbfbfbf),
    ("grey76", 0xffc2c2c2),
    ("grey77", 0xffc4c4c4),
    ("grey78", 0xffc7c7c7),
    ("grey79", 0xffc9c9c9),
    ("grey8", 0xff141414),
    ("grey80", 0xffcccccc),
    ("grey81", 0xffcfcfcf),
    ("grey82", 0xffd1d1d1),
    ("grey83", 0xffd4d4d4),
    ("grey84", 0xffd6d6d6),
    ("grey85", 0xffd9d9d9),
    ("grey86", 0xffdbdbdb),
    ("grey87", 0xffdedede),
    ("grey88", 0xffe0e0e0),
    ("grey89", 0xffe3e3e3),
    ("grey9", 0xff171717),
    ("grey90", 0xffe5e5e5),
    ("grey91", 0xffe8e8e8),
    ("grey92", 0xffebebeb),
    ("grey93", 0xffededed),
    ("grey94", 0xfff0f0f0),
    ("grey95", 0xfff2f2f2),
    ("grey96", 0xfff5f5f5),
    ("grey97", 0xfff7f7f7),
    ("grey98", 0xfffafafa),
    ("grey99", 0xfffcfcfc),
    ("honeydew", 0xfff0fff0),
    ("honeydew1", 0xfff0fff0),
    ("honeydew2", 0xffe0eee0),
    ("honeydew3", 0xffc1cdc1),
    ("honeydew4", 0xff838b83),
    ("hotpink", 0xffff69b4),
    ("hotpink1", 0xffff6eb4),
    ("hotpink2", 0xffee6aa7),
    ("hotpink3", 0xffcd6090),
    ("hotpink4", 0xff8b3a62),
    ("indianred", 0xffcd5c5c),
    ("indianred1", 0xffff6a6a),
    ("indianred2", 0xffee6363),
    ("indianred3", 0xffcd5555),
    ("indianred4", 0xff8b3a3a),
    ("indigo", 0xff4b0082),
    ("ivory", 0xfffffff0),
    ("ivory1", 0xfffffff0),
    ("ivory2", 0xffeeeee0),
    ("ivory3", 0xffcdcdc1),
    ("ivory4", 0xff8b8b83),
    ("khaki", 0xfff0e68c),
    ("khaki1", 0xfffff68f),
    ("khaki2", 0xffeee685),
    ("khaki3", 0xffcdc673),
    ("khaki4", 0xff8b864e),
    ("lavender", 0xffe6e6fa),
    ("lavenderblush", 0xfffff0f5),
    ("lavenderblush1", 0xfffff0f5),
    ("lavenderblush2", 0xffeee0e5),
    ("lavenderblush3", 0xffcdc1c5),
    ("lavenderblush4", 0xff8b8386),
    ("lawngreen", 0xff7cfc00),
    ("lemonchiffon", 0xfffffacd),
    ("lemonchiffon1", 0xfffffacd),
    ("lemonchiffon2", 0xffeee9bf),
    ("lemonchiffon3", 0xffcdc9a5),
    ("lemonchiffon4", 0xff8b8970),
    ("lightblue", 0xffadd8e6),
    ("lightblue1", 0xffbfefff),
    ("lightblue2", 0xffb2dfee),
    ("lightblue3", 0xff9ac0cd),
    ("lightblue4", 0xff68838b),
    ("lightcoral", 0xfff08080),
    ("lightcyan", 0xffe0ffff),
    ("lightcyan1", 0xffe0ffff),
    ("lightcyan2", 0xffd1eeee),
    ("lightcyan3", 0xffb4cdcd),
    ("lightcyan4", 0xff7a8b8b),
    ("lightgoldenrod", 0xffeedd82),
    ("lightgoldenrod1", 0xffffec8b),
    ("lightgoldenrod2", 0xffeedc82),
    ("lightgoldenrod3", 0xffcdbe70),
    ("lightgoldenrod4", 0xff8b814c),
    ("lightgoldenrodyellow", 0xfffafad2),
    ("lightgray", 0xffd3d3d3),
    ("lightgreen", 0xff90ee90),
    ("lightgrey", 0xffd3d3d3),
    ("lightpink", 0xffffb6c1),
    ("lightpink1", 0xffffaeb9),
    ("lightpink2", 0xffeea2ad),
    ("lightpink3", 0xffcd8c95),
    ("lightpink4", 0xff8b5f65),
    ("lightsalmon", 0xffffa07a),
    ("lightsalmon1", 0xffffa07a),
    ("lightsalmon2", 0xffee9572),
    ("lightsalmon3", 0xffcd8162),
    ("lightsalmon4", 0xff8b5742),
    ("lightseagreen", 0xff20b2aa),
    ("lightskyblue", 0xff87cefa),
    ("lightskyblue1", 0xffb0e2ff),
    ("lightskyblue2", 0xffa4d3ee),
    ("lightskyblue3", 0xff8db6cd),
    ("lightskyblue4", 0xff607b8b),
    ("lightslateblue", 0xff8470ff),
    ("lightslategray", 0xff778899),
    ("lightslategrey", 0xff778899),
    ("lightsteelblue", 0xffb0c4de),
    ("lightsteelblue1", 0xffcae1ff),
    ("lightsteelblue2", 0xffbcd2ee),
    ("lightsteelblue3", 0xffa2b5cd),
    ("lightsteelblue4", 0xff6e7b8b),
    ("lightyellow", 0xffffffe0),
    ("lightyellow1", 0xffffffe0),
    ("lightyellow2", 0xffeeeed1),
    ("lightyellow3", 0xffcdcdb4),
    ("lightyellow4", 0xff8b8b7a),
    ("lime", 0xff00ff00),
    ("limegreen", 0xff32cd32),
    ("linen", 0xfffaf0e6),
    ("magenta", 0xffff00ff),
    ("magenta1", 0xffff00ff),
    ("magenta2", 0xffee00ee),
    ("magenta3", 0xffcd00cd),
    ("magenta4", 0xff8b008b),
    ("maroon", 0xff800000),
    ("maroon", 0xffb03060),
    ("maroon1", 0xffff34b3),
    ("maroon2", 0xffee30a7),
    ("maroon3", 0xffcd2990),
    ("maroon4", 0xff8b1c62),
    ("mediumaquamarine", 0xff66cdaa),
    ("mediumblue", 0xff0000cd),
    ("mediumforestgreen", 0xff32814b),
    ("mediumgoldenrod", 0xffd1c166),
    ("mediumorchid", 0xffba55d3),
    ("mediumorchid1", 0xffe066ff),
    ("mediumorchid2", 0xffd15fee),
    ("mediumorchid3", 0xffb452cd),
    ("mediumorchid4", 0xff7a378b),
    ("mediumpurple", 0xff9370db),
    ("mediumpurple1", 0xffab82ff),
    ("mediumpurple2", 0xff9f79ee),
    ("mediumpurple3", 0xff8968cd),
    ("mediumpurple4", 0xff5d478b),
    ("mediumseagreen", 0xff3cb371),
    ("mediumslateblue", 0xff7b68ee),
    ("mediumspringgreen", 0xff00fa9a),
    ("mediumturquoise", 0xff48d1cc),
    ("mediumvioletred", 0xffc71585),
    ("midnightblue", 0xff191970),
    ("mintcream", 0xfff5fffa),
    ("mistyrose", 0xffffe4e1),
    ("mistyrose1", 0xffffe4e1),
    ("mistyrose2", 0xffeed5d2),
    ("mistyrose3", 0xffcdb7b5),
    ("mistyrose4", 0xff8b7d7b),
    ("moccasin", 0xffffe4b5),
    ("navajowhite", 0xffffdead),
    ("navajowhite1", 0xffffdead),
    ("navajowhite2", 0xffeecfa1),
    ("navajowhite3", 0xffcdb38b),
    ("navajowhite4", 0xff8b795e),
    ("navy", 0xff000080),
    ("navyblue", 0xff000080),
    ("none", 0xff0000ff),
    ("oldlace", 0xfffdf5e6),
    ("olive", 0xff808000),
    ("olivedrab", 0xff6b8e23),
    ("olivedrab1", 0xffc0ff3e),
    ("olivedrab2", 0xffb3ee3a),
    ("olivedrab3", 0xff9acd32),
    ("olivedrab4", 0xff698b22),
    ("opaque", 0xff000000),
    ("orange", 0xffffa500),
    ("orange1", 0xffffa500),
    ("orange2", 0xffee9a00),
    ("orange3", 0xffcd8500),
    ("orange4", 0xff8b5a00),
    ("orangered", 0xffff4500),
    ("orangered1", 0xffff4500),
    ("orangered2", 0xffee4000),
    ("orangered3", 0xffcd3700),
    ("orangered4", 0xff8b2500),
    ("orchid", 0xffda70d6),
    ("orchid1", 0xffff83fa),
    ("orchid2", 0xffee7ae9),
    ("orchid3", 0xffcd69c9),
    ("orchid4", 0xff8b4789),
    ("palegoldenrod", 0xffeee8aa),
    ("palegreen", 0xff98fb98),
    ("palegreen1", 0xff9aff9a),
    ("palegreen2", 0xff90ee90),
    ("palegreen3", 0xff7ccd7c),
    ("palegreen4", 0xff548b54),
    ("paleturquoise", 0xffafeeee),
    ("paleturquoise1", 0xffbbffff),
    ("paleturquoise2", 0xffaeeeee),
    ("paleturquoise3", 0xff96cdcd),
    ("paleturquoise4", 0xff668b8b),
    ("palevioletred", 0xffdb7093),
    ("palevioletred1", 0xffff82ab),
    ("palevioletred2", 0xffee799f),
    ("palevioletred3", 0xffcd6889),
    ("palevioletred4", 0xff8b475d),
    ("papayawhip", 0xffffefd5),
    ("peachpuff", 0xffffdab9),
    ("peachpuff1", 0xffffdab9),
    ("peachpuff2", 0xffeecbad),
    ("peachpuff3", 0xffcdaf95),
    ("peachpuff4", 0xff8b7765),
    ("peru", 0xffcd853f),
    ("pink", 0xffffc0cb),
    ("pink1", 0xffffb5c5),
    ("pink2", 0xffeea9b8),
    ("pink3", 0xffcd919e),
    ("pink4", 0xff8b636c),
    ("plum", 0xffdda0dd),
    ("plum1", 0xffffbbff),
    ("plum2", 0xffeeaeee),
    ("plum3", 0xffcd96cd),
    ("plum4", 0xff8b668b),
    ("powderblue", 0xffb0e0e6),
    ("purple", 0xff800080),
    ("purple", 0xffa020f0),
    ("purple1", 0xff9b30ff),
    ("purple2", 0xff912cee),
    ("purple3", 0xff7d26cd),
    ("purple4", 0xff551a8b),
    ("red", 0xffff0000),
    ("red1", 0xffff0000),
    ("red2", 0xffee0000),
    ("red3", 0xffcd0000),
    ("red4", 0xff8b0000),
    ("rosybrown", 0xffbc8f8f),
    ("rosybrown1", 0xffffc1c1),
    ("rosybrown2", 0xffeeb4b4),
    ("rosybrown3", 0xffcd9b9b),
    ("rosybrown4", 0xff8b6969),
    ("royalblue", 0xff4169e1),
    ("royalblue1", 0xff4876ff),
    ("royalblue2", 0xff436eee),
    ("royalblue3", 0xff3a5fcd),
    ("royalblue4", 0xff27408b),
    ("saddlebrown", 0xff8b4513),
    ("salmon", 0xfffa8072),
    ("salmon1", 0xffff8c69),
    ("salmon2", 0xffee8262),
    ("salmon3", 0xffcd7054),
    ("salmon4", 0xff8b4c39),
    ("sandybrown", 0xfff4a460),
    ("seagreen", 0xff2e8b57),
    ("seagreen1", 0xff54ff9f),
    ("seagreen2", 0xff4eee94),
    ("seagreen3", 0xff43cd80),
    ("seagreen4", 0xff2e8b57),
    ("seashell", 0xfffff5ee),
    ("seashell1", 0xfffff5ee),
    ("seashell2", 0xffeee5de),
    ("seashell3", 0xffcdc5bf),
    ("seashell4", 0xff8b8682),
    ("sienna", 0xffa0522d),
    ("sienna1", 0xffff8247),
    ("sienna2", 0xffee7942),
    ("sienna3", 0xffcd6839),
    ("sienna4", 0xff8b4726),
    ("silver", 0xffc0c0c0),
    ("skyblue", 0xff87ceeb),
    ("skyblue1", 0xff87ceff),
    ("skyblue2", 0xff7ec0ee),
    ("skyblue3", 0xff6ca6cd),
    ("skyblue4", 0xff4a708b),
    ("slateblue", 0xff6a5acd),
    ("slateblue1", 0xff836fff),
    ("slateblue2", 0xff7a67ee),
    ("slateblue3", 0xff6959cd),
    ("slateblue4", 0xff473c8b),
    ("slategray", 0xff708090),
    ("slategray1", 0xffc6e2ff),
    ("slategray2", 0xffb9d3ee),
    ("slategray3", 0xff9fb6cd),
    ("slategray4", 0xff6c7b8b),
    ("slategrey", 0xff708090),
    ("snow", 0xfffffafa),
    ("snow1", 0xfffffafa),
    ("snow2", 0xffeee9e9),
    ("snow3", 0xffcdc9c9),
    ("snow4", 0xff8b8989),
    ("springgreen", 0xff00ff7f),
    ("springgreen1", 0xff00ff7f),
    ("springgreen2", 0xff00ee76),
    ("springgreen3", 0xff00cd66),
    ("springgreen4", 0xff008b45),
    ("steelblue", 0xff4682b4),
    ("steelblue1", 0xff63b8ff),
    ("steelblue2", 0xff5cacee),
    ("steelblue3", 0xff4f94cd),
    ("steelblue4", 0xff36648b),
    ("tan", 0xffd2b48c),
    ("tan1", 0xffffa54f),
    ("tan2", 0xffee9a49),
    ("tan3", 0xffcd853f),
    ("tan4", 0xff8b5a2b),
    ("teal", 0xff008080),
    ("thistle", 0xffd8bfd8),
    ("thistle1", 0xffffe1ff),
    ("thistle2", 0xffeed2ee),
    ("thistle3", 0xffcdb5cd),
    ("thistle4", 0xff8b7b8b),
    ("tomato", 0xffff6347),
    ("tomato1", 0xffff6347),
    ("tomato2", 0xffee5c42),
    ("tomato3", 0xffcd4f39),
    ("tomato4", 0xff8b3626),
    ("transparent", 0xff0000ff),
    ("turquoise", 0xff40e0d0),
    ("turquoise1", 0xff00f5ff),
    ("turquoise2", 0xff00e5ee),
    ("turquoise3", 0xff00c5cd),
    ("turquoise4", 0xff00868b),
    ("violet", 0xffee82ee),
    ("violetred", 0xffd02090),
    ("violetred1", 0xffff3e96),
    ("violetred2", 0xffee3a8c),
    ("violetred3", 0xffcd3278),
    ("violetred4", 0xff8b2252),
    ("wheat", 0xfff5deb3),
    ("wheat1", 0xffffe7ba),
    ("wheat2", 0xffeed8ae),
    ("wheat3", 0xffcdba96),
    ("wheat4", 0xff8b7e66),
    ("white", 0xffffffff),
    ("whitesmoke", 0xfff5f5f5),
    ("yellow", 0xffffff00),
    ("yellow1", 0xffffff00),
    ("yellow2", 0xffeeee00),
    ("yellow3", 0xffcdcd00),
    ("yellow4", 0xff8b8b00),
    ("yellowgreen", 0xff9acd32),
    // #endif /* !DISABLE_EXTENDED_XPM_COLORS */
];
