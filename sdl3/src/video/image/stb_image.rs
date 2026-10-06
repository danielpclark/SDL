// Rust translation of the parts of src/video/stb_image.h that SDL uses
// (the PNG and JPEG loaders, as configured by SDL_stb.c), from Simple
// DirectMedia Layer.
// stb_image - v2.30 - public domain image loader - http://nothings.org/stb
// (Sean Barrett and contributors); see the notice in upstream's stb_image.h.
// SDL's changes (palette loading, NV12 output, SDL error reporting) are
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>.
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The image loader core: the buffered reading context, size checks, and
//! component conversion.
//!
//! SDL defines `STBI_ONLY_PNG`, `STBI_ONLY_JPEG`, `STBI_NO_STDIO` and
//! `STBI_FAILURE_USERMSG` (errors are the user-facing messages, reported
//! through `SDL_SetError()`). stb's SSE2/NEON kernels produce results
//! identical to its scalar code, so only the scalar code is translated.
//! Where stb returns a failure without setting a message (and SDL reports
//! whatever error was already set), these return "Corrupt PNG" or
//! "Corrupt JPEG".

// stb's and miniz's constants are written with more digits than f32
// holds, and their loops index several arrays at once; both kept as written.
#![allow(
    clippy::excessive_precision,
    clippy::needless_range_loop,
    clippy::too_many_arguments
)]

use crate::error::{Error, Result};

pub(crate) use super::jpeg::{jpeg_load, jpeg_test};
use super::png::{png_load, png_test};

/// The largest width or height accepted. Translation of `STBI_MAX_DIMENSIONS`.
pub(crate) const STBI_MAX_DIMENSIONS: u32 = 1 << 24;

/// `stbi__err(x, y)` with `STBI_FAILURE_USERMSG`: the user message `y`.
pub(crate) fn err<T>(msg: &'static str) -> Result<T> {
    Err(Error::new(msg))
}

/// The I/O callbacks of a context. Translation of `stbi_io_callbacks`.
pub trait Callbacks {
    /// fill 'data' with 'size' bytes.  return number of bytes actually read
    fn read(&mut self, data: &mut [u8]) -> usize;
    /// skip the next 'n' bytes, or 'unget' the last -n bytes if negative
    fn skip(&mut self, n: i32);
    /// returns nonzero if we are at end of file/data
    fn eof(&mut self) -> bool;
}

/// The basic context used by all images: the I/O plus some basic image
/// information. Translation of `stbi__context`; the buffer pointers are
/// indices into the memory buffer or into `buffer_start`.
pub(crate) struct Context<'a> {
    pub(crate) img_x: u32,
    pub(crate) img_y: u32,
    pub(crate) img_n: i32,
    pub(crate) img_out_n: i32,

    io: Option<&'a mut dyn Callbacks>,

    read_from_callbacks: bool,
    buflen: usize,
    buffer_start: [u8; 128],
    callback_already_read: i32,

    /// The memory being decoded (empty when reading from callbacks).
    mem: &'a [u8],
    pub(crate) img_buffer: usize,
    pub(crate) img_buffer_end: usize,
    img_buffer_original: usize,
    img_buffer_original_end: usize,
}

impl<'a> Context<'a> {
    /// Initialize a memory-decode context. Translation of `stbi__start_mem()`.
    pub(crate) fn from_mem(buffer: &'a [u8]) -> Context<'a> {
        Context {
            img_x: 0,
            img_y: 0,
            img_n: 0,
            img_out_n: 0,
            io: None,
            read_from_callbacks: false,
            buflen: 0,
            buffer_start: [0; 128],
            callback_already_read: 0,
            mem: buffer,
            img_buffer: 0,
            img_buffer_end: buffer.len(),
            img_buffer_original: 0,
            img_buffer_original_end: buffer.len(),
        }
    }

    /// Initialize a callback-based context. Translation of `stbi__start_callbacks()`.
    pub(crate) fn from_callbacks(c: &'a mut dyn Callbacks) -> Context<'a> {
        let mut s = Context {
            img_x: 0,
            img_y: 0,
            img_n: 0,
            img_out_n: 0,
            io: Some(c),
            read_from_callbacks: true,
            buflen: 128,
            buffer_start: [0; 128],
            callback_already_read: 0,
            mem: &[],
            img_buffer: 0,
            img_buffer_end: 0,
            img_buffer_original: 0,
            img_buffer_original_end: 0,
        };
        s.refill_buffer();
        s.img_buffer_original_end = s.img_buffer_end;
        s
    }

    fn buf(&self) -> &[u8] {
        if self.io.is_some() {
            &self.buffer_start
        } else {
            self.mem
        }
    }

    /// Whether the context reads from callbacks (`s->io.read`).
    pub(crate) fn has_io(&self) -> bool {
        self.io.is_some()
    }

    /// `(s->io.skip)(s->io_user_data, n)`.
    pub(crate) fn io_skip(&mut self, n: i32) {
        if let Some(io) = &mut self.io {
            io.skip(n);
        }
    }

    /// Conceptually rewind SHOULD rewind to the beginning of the stream,
    /// but we just rewind to the beginning of the initial buffer, because
    /// we only use it after doing 'test', which only ever looks at at most
    /// 92 bytes. Translation of `stbi__rewind()`.
    pub(crate) fn rewind(&mut self) {
        self.img_buffer = self.img_buffer_original;
        self.img_buffer_end = self.img_buffer_original_end;
    }

    /// Translation of `stbi__refill_buffer()`.
    fn refill_buffer(&mut self) {
        let buflen = self.buflen;
        let io = self.io.as_mut().expect("refill without callbacks");
        let n = io.read(&mut self.buffer_start[..buflen]);
        self.callback_already_read +=
            (self.img_buffer as i32).wrapping_sub(self.img_buffer_original as i32);
        if n == 0 {
            // at end of file, treat same as if from memory, but need to handle case
            // where s->img_buffer isn't pointing to safe memory, e.g. 0-byte file
            self.read_from_callbacks = false;
            self.img_buffer = 0;
            self.img_buffer_end = 1;
            self.buffer_start[0] = 0;
        } else {
            self.img_buffer = 0;
            self.img_buffer_end = n;
        }
    }

    /// Translation of `stbi__get8()`.
    #[inline]
    pub(crate) fn get8(&mut self) -> u8 {
        if self.img_buffer < self.img_buffer_end {
            let b = self.buf()[self.img_buffer];
            self.img_buffer += 1;
            return b;
        }
        if self.read_from_callbacks {
            self.refill_buffer();
            let b = self.buffer_start[self.img_buffer];
            self.img_buffer += 1;
            return b;
        }
        0
    }

    /// Translation of `stbi__at_eof()`.
    pub(crate) fn at_eof(&mut self) -> bool {
        if let Some(io) = &mut self.io {
            if !io.eof() {
                return false;
            }
            // if feof() is true, check if buffer = end
            // special case: we've only got the special 0 character at the end
            if !self.read_from_callbacks {
                return true;
            }
        }

        self.img_buffer >= self.img_buffer_end
    }

    /// Translation of `stbi__skip()`.
    pub(crate) fn skip(&mut self, n: i32) {
        if n == 0 {
            return; // already there!
        }
        if n < 0 {
            self.img_buffer = self.img_buffer_end;
            return;
        }
        if let Some(io) = &mut self.io {
            let blen = (self.img_buffer_end - self.img_buffer) as i32;
            if blen < n {
                self.img_buffer = self.img_buffer_end;
                io.skip(n - blen);
                return;
            }
        }
        self.img_buffer += n as usize;
    }

    /// Translation of `stbi__getn()`.
    pub(crate) fn getn(&mut self, buffer: &mut [u8]) -> bool {
        let n = buffer.len();
        if let Some(io) = &mut self.io {
            let blen = self.img_buffer_end - self.img_buffer;
            if blen < n {
                buffer[..blen]
                    .copy_from_slice(&self.buffer_start[self.img_buffer..self.img_buffer_end]);

                let count = io.read(&mut buffer[blen..]);
                let res = count == n - blen;
                self.img_buffer = self.img_buffer_end;
                return res;
            }
        }

        if self.img_buffer + n <= self.img_buffer_end {
            let start = self.img_buffer;
            buffer.copy_from_slice(&self.buf()[start..start + n]);
            self.img_buffer += n;
            true
        } else {
            false
        }
    }

    /// Translation of `stbi__get16be()`.
    pub(crate) fn get16be(&mut self) -> i32 {
        let z = self.get8() as i32;
        (z << 8) + self.get8() as i32
    }

    /// Translation of `stbi__get32be()`.
    pub(crate) fn get32be(&mut self) -> u32 {
        let z = self.get16be() as u32;
        (z << 16) + self.get16be() as u32
    }
}

// stb_image uses ints pervasively, including for offset calculations.
// therefore the largest decoded image size we can support with the
// current code, even on 64-bit targets, is INT_MAX. this is not a
// significant limitation for the intended use case.
//
// we do, however, need to make sure our size calculations don't
// overflow. hence a few helper functions for size calculations that
// multiply integers together, making sure that they're non-negative
// and no overflow occurs.

/// Return true if the sum is valid, false on overflow; negative terms are
/// considered invalid. Translation of `stbi__addsizes_valid()`.
pub(crate) fn addsizes_valid(a: i32, b: i32) -> bool {
    if b < 0 {
        return false;
    }
    // now 0 <= b <= INT_MAX, hence also
    // 0 <= INT_MAX - b <= INTMAX.
    // And "a + b <= INT_MAX" (which might overflow) is the
    // same as a <= INT_MAX - b (no overflow)
    a <= i32::MAX - b
}

/// Returns true if the product is valid, false on overflow; negative
/// factors are considered invalid. Translation of `stbi__mul2sizes_valid()`.
pub(crate) fn mul2sizes_valid(a: i32, b: i32) -> bool {
    if a < 0 || b < 0 {
        return false;
    }
    if b == 0 {
        return true; // mul-by-0 is always safe
    }
    // portable way to check for no overflows in a*b
    a <= i32::MAX / b
}

/// Returns true if "a*b + add" has no negative terms/factors and doesn't
/// overflow. Translation of `stbi__mad2sizes_valid()`.
pub(crate) fn mad2sizes_valid(a: i32, b: i32, add: i32) -> bool {
    mul2sizes_valid(a, b) && addsizes_valid(a.wrapping_mul(b), add)
}

/// Returns true if "a*b*c + add" has no negative terms/factors and doesn't
/// overflow. Translation of `stbi__mad3sizes_valid()`.
pub(crate) fn mad3sizes_valid(a: i32, b: i32, c: i32, add: i32) -> bool {
    mul2sizes_valid(a, b)
        && mul2sizes_valid(a.wrapping_mul(b), c)
        && addsizes_valid(a.wrapping_mul(b).wrapping_mul(c), add)
}

/// Returns true if "a*b*c*d + add" has no negative terms/factors and
/// doesn't overflow. Translation of `stbi__mad4sizes_valid()`.
pub(crate) fn mad4sizes_valid(a: i32, b: i32, c: i32, d: i32, add: i32) -> bool {
    mul2sizes_valid(a, b)
        && mul2sizes_valid(a.wrapping_mul(b), c)
        && mul2sizes_valid(a.wrapping_mul(b).wrapping_mul(c), d)
        && addsizes_valid(a.wrapping_mul(b).wrapping_mul(c).wrapping_mul(d), add)
}

/// A zeroed buffer of `a*b + add` bytes, or `None` if the size overflows.
/// Translation of `stbi__malloc_mad2()`.
pub(crate) fn malloc_mad2(a: i32, b: i32, add: i32) -> Option<Vec<u8>> {
    if !mad2sizes_valid(a, b, add) {
        return None;
    }
    Some(vec![0; (a * b + add) as usize])
}

/// Translation of `stbi__malloc_mad3()`.
pub(crate) fn malloc_mad3(a: i32, b: i32, c: i32, add: i32) -> Option<Vec<u8>> {
    if !mad3sizes_valid(a, b, c, add) {
        return None;
    }
    Some(vec![0; (a * b * c + add) as usize])
}

/// Returns true if the sum of two signed ints is valid (between -2^31 and
/// 2^31-1 inclusive), false on overflow. Translation of `stbi__addints_valid()`.
pub(crate) fn addints_valid(a: i32, b: i32) -> bool {
    if (a >= 0) != (b >= 0) {
        return true; // a and b have different signs, so no overflow
    }
    if a < 0 && b < 0 {
        return a >= i32::MIN - b; // same as a + b >= INT_MIN; INT_MIN - b cannot overflow since b < 0.
    }
    a <= i32::MAX - b
}

/// Returns true if the product of two ints fits in a signed short, false
/// on overflow. Translation of `stbi__mul2shorts_valid()`.
pub(crate) fn mul2shorts_valid(a: i32, b: i32) -> bool {
    let (shrt_max, shrt_min) = (i16::MAX as i32, i16::MIN as i32);
    if b == 0 || b == -1 {
        return true; // multiplication by 0 is always 0; check for -1 so SHRT_MIN/b doesn't overflow
    }
    if (a >= 0) == (b >= 0) {
        return a <= shrt_max / b; // product is positive, so similar to mul2sizes_valid
    }
    if b < 0 {
        return a <= shrt_min / b; // same as a * b >= SHRT_MIN
    }
    a >= shrt_min / b
}

/// Decoded pixels: 8 or 16 bits per channel. Translation of the
/// `void *` result with `stbi__result_info::bits_per_channel`.
pub(crate) enum Pixels {
    Bits8(Vec<u8>),
    Bits16(Vec<u16>),
}

/// A decoded image: `x` by `y` pixels of `out_n` interleaved components
/// (`comp` is the number in the file).
#[derive(Debug)]
pub struct Image<P> {
    /// The width in pixels.
    pub x: i32,
    /// The height in pixels.
    pub y: i32,
    /// The number of components in the file.
    pub comp: i32,
    /// The pixels.
    pub data: P,
}

/// Translation of `stbi__compute_y()`.
pub(crate) fn compute_y(r: i32, g: i32, b: i32) -> u8 {
    (((r * 77) + (g * 150) + (29 * b)) >> 8) as u8
}

/// Translation of `stbi__compute_y_16()`.
fn compute_y_16(r: i32, g: i32, b: i32) -> u16 {
    (((r * 77) + (g * 150) + (29 * b)) >> 8) as u16
}

/// Generic converter from built-in img_n to req_comp (individual types do
/// this automatically as much as possible: jpeg does all cases internally
/// since it needs to colorspace convert anyway, and png can automatically
/// interleave an alpha=255 channel, but falls back to this for other
/// cases). Translation of `stbi__convert_format()` and
/// `stbi__convert_format16()`.
fn convert_format<T: Copy>(
    data: Vec<T>,
    img_n: i32,
    req_comp: i32,
    x: u32,
    y: u32,
    max: T,
    compute_y: fn(i32, i32, i32) -> T,
    to_i32: fn(T) -> i32,
) -> Result<Vec<T>> {
    if req_comp == img_n {
        return Ok(data);
    }
    crate::sdl_assert!((1..=4).contains(&req_comp));

    let bytes = std::mem::size_of::<T>() as i32;
    let valid = if bytes == 1 {
        mad3sizes_valid(req_comp, x as i32, y as i32, 0)
    } else {
        mad4sizes_valid(req_comp, x as i32, y as i32, bytes, 0)
    };
    if !valid {
        return err("Out of memory");
    }
    let mut good = vec![max; (req_comp as u32 * x * y) as usize];
    let (img_n, req_comp) = (img_n as usize, req_comp as usize);
    let cy = |s: &[T]| compute_y(to_i32(s[0]), to_i32(s[1]), to_i32(s[2]));

    for j in 0..y as usize {
        let src_row = &data[j * x as usize * img_n..][..x as usize * img_n];
        let dest_row = &mut good[j * x as usize * req_comp..][..x as usize * req_comp];
        // convert source image with img_n components to one with req_comp components;
        // avoid switch per pixel, so use switch per scanline and massive macros
        for (src, dest) in src_row
            .chunks_exact(img_n)
            .zip(dest_row.chunks_exact_mut(req_comp))
        {
            match (img_n, req_comp) {
                (1, 2) => {
                    dest[0] = src[0];
                    dest[1] = max;
                }
                (1, 3) => dest.fill(src[0]),
                (1, 4) => {
                    dest[..3].fill(src[0]);
                    dest[3] = max;
                }
                (2, 1) => dest[0] = src[0],
                (2, 3) => dest.fill(src[0]),
                (2, 4) => {
                    dest[..3].fill(src[0]);
                    dest[3] = src[1];
                }
                (3, 4) => {
                    dest[..3].copy_from_slice(&src[..3]);
                    dest[3] = max;
                }
                (3, 1) => dest[0] = cy(src),
                (3, 2) => {
                    dest[0] = cy(src);
                    dest[1] = max;
                }
                (4, 1) => dest[0] = cy(src),
                (4, 2) => {
                    dest[0] = cy(src);
                    dest[1] = src[3];
                }
                (4, 3) => dest.copy_from_slice(&src[..3]),
                _ => {
                    crate::sdl_assert!(false);
                    return err("Unsupported format conversion");
                }
            }
        }
    }

    Ok(good)
}

/// Translation of `stbi__convert_format()`.
pub(crate) fn convert_format8(
    data: Vec<u8>,
    img_n: i32,
    req_comp: i32,
    x: u32,
    y: u32,
) -> Result<Vec<u8>> {
    convert_format(data, img_n, req_comp, x, y, 255, compute_y, |v| v as i32)
}

/// Translation of `stbi__convert_format16()`.
pub(crate) fn convert_format16(
    data: Vec<u16>,
    img_n: i32,
    req_comp: i32,
    x: u32,
    y: u32,
) -> Result<Vec<u16>> {
    convert_format(data, img_n, req_comp, x, y, 0xffff, compute_y_16, |v| {
        v as i32
    })
}

/// Translation of `stbi__load_main()`: try each format SDL compiles in.
fn load_main(
    s: &mut Context<'_>,
    req_comp: i32,
    palette_buffer: Option<&mut [u8; 1024]>,
) -> Result<Image<Pixels>> {
    // test the formats with a very explicit header first (at least a FOURCC
    // or distinctive magic number first)
    if png_test(s) {
        return png_load(s, req_comp, palette_buffer);
    }

    // then the formats that can end up attempting to load with just 1 or 2
    // bytes matching expectations; these are prone to false positives, so
    // try them later
    if jpeg_test(s) {
        return jpeg_load(s, req_comp, None).map(|i| Image {
            x: i.x,
            y: i.y,
            comp: i.comp,
            data: Pixels::Bits8(i.data),
        });
    }

    err("Image not of any known type, or corrupt")
}

/// Translation of `stbi__convert_16_to_8()`.
fn convert_16_to_8(orig: Vec<u16>) -> Vec<u8> {
    // top half of each byte is sufficient approx of 16->8 bit scaling
    orig.into_iter().map(|v| ((v >> 8) & 0xFF) as u8).collect()
}

/// Translation of `stbi__load_indexed()`: the palette indices of a
/// paletted image (the palette, four bytes per entry, goes to
/// `palette_buffer`).
fn load_indexed(s: &mut Context<'_>, palette_buffer: &mut [u8; 1024]) -> Result<Image<Vec<u8>>> {
    let result = load_main(s, 1, Some(palette_buffer))?;

    // (upstream returns NULL here without setting an error)
    if result.comp != 1 {
        return err("Corrupt PNG");
    }

    let Pixels::Bits8(data) = result.data else {
        return err("Corrupt PNG");
    };

    // @TODO: move stbi__convert_format to here

    // (stbi__vertically_flip_on_load is never set in SDL)
    Ok(Image {
        x: result.x,
        y: result.y,
        comp: result.comp,
        data,
    })
}

/// Translation of `stbi__load_and_postprocess_8bit()`.
fn load_and_postprocess_8bit(s: &mut Context<'_>, req_comp: i32) -> Result<Image<Vec<u8>>> {
    let result = load_main(s, req_comp, None)?;

    // it is the responsibility of the loaders to make sure we get either 8 or 16 bit.
    let data = match result.data {
        Pixels::Bits8(d) => d,
        Pixels::Bits16(d) => convert_16_to_8(d),
    };

    // @TODO: move stbi__convert_format to here

    // (stbi__vertically_flip_on_load is never set in SDL)
    Ok(Image {
        x: result.x,
        y: result.y,
        comp: result.comp,
        data,
    })
}

/// Translation of `stbi_load_from_memory()`.
pub(crate) fn load_from_memory(buffer: &[u8], req_comp: i32) -> Result<Image<Vec<u8>>> {
    let mut s = Context::from_mem(buffer);
    load_and_postprocess_8bit(&mut s, req_comp)
}

/// Decode a PNG or JPEG image read through `clbk` to 8 bits per channel:
/// `req_comp` components per pixel, or the file's number for 0.
/// Translation of `stbi_load_from_callbacks()`.
pub fn load_from_callbacks(clbk: &mut dyn Callbacks, req_comp: i32) -> Result<Image<Vec<u8>>> {
    let mut s = Context::from_callbacks(clbk);
    load_and_postprocess_8bit(&mut s, req_comp)
}

/// Decode an 8-bit paletted PNG read through `clbk` to one index per
/// pixel, filling `palette_buffer` with its RGBA colors (the entries the
/// file doesn't set are left as they were).
/// Translation of `stbi_load_from_callbacks_with_palette()`.
pub fn load_from_callbacks_with_palette(
    clbk: &mut dyn Callbacks,
    palette_buffer: &mut [u8; 1024],
) -> Result<Image<Vec<u8>>> {
    let mut s = Context::from_callbacks(clbk);
    load_indexed(&mut s, palette_buffer)
}
