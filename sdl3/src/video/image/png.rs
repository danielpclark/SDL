// Rust translation of the PNG decoder of src/video/stb_image.h, from
// Simple DirectMedia Layer.
// public domain "baseline" PNG decoder   v0.10  Sean Barrett 2006-11-18
// SDL's palette loading is Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>.
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The PNG decoder: 1/2/4/8/16-bit samples, every color type, interlacing,
//! `tRNS` transparency, and (an SDL addition) loading a paletted image as
//! indices plus its palette. No CRC checking; uses the zlib decoder with
//! fast huffman decoding.

use super::stb_image::{
    convert_format16, convert_format8, err, mad2sizes_valid, mad3sizes_valid, malloc_mad2,
    malloc_mad3, Context, Image, Pixels, STBI_MAX_DIMENSIONS,
};
use super::zlib::zlib_decode_malloc_guesssize_headerflag;
use crate::error::Result;

/// Translation of `stbi__pngchunk`.
struct PngChunk {
    length: u32,
    ty: u32,
}

/// Translation of `stbi__get_chunk_header()`.
fn get_chunk_header(s: &mut Context<'_>) -> PngChunk {
    let length = s.get32be();
    let ty = s.get32be();
    PngChunk { length, ty }
}

/// Translation of `stbi__check_png_header()`.
fn check_png_header(s: &mut Context<'_>) -> Result<()> {
    const PNG_SIG: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];
    for &b in &PNG_SIG {
        if s.get8() != b {
            return err("Not a PNG");
        }
    }
    Ok(())
}

/// Translation of `stbi__png`.
struct Png<'c, 'a> {
    s: &'c mut Context<'a>,
    out: Vec<u8>,
    depth: i32,
}

const STBI__F_NONE: u8 = 0;
const STBI__F_SUB: u8 = 1;
const STBI__F_UP: u8 = 2;
const STBI__F_AVG: u8 = 3;
const STBI__F_PAETH: u8 = 4;
// synthetic filter used for first scanline to avoid needing a dummy row of 0s
const STBI__F_AVG_FIRST: u8 = 5;

const FIRST_ROW_FILTER: [u8; 5] = [
    STBI__F_NONE,
    STBI__F_SUB,
    STBI__F_NONE,
    STBI__F_AVG_FIRST,
    STBI__F_SUB, // Paeth with b=c=0 turns out to be equivalent to sub
];

/// Translation of `stbi__paeth()`.
fn paeth(a: i32, b: i32, c: i32) -> i32 {
    // This formulation looks very different from the reference in the PNG spec, but is
    // actually equivalent and has favorable data dependencies and admits straightforward
    // generation of branch-free code, which helps performance significantly.
    let thresh = c * 3 - (a + b);
    let lo = if a < b { a } else { b };
    let hi = if a < b { b } else { a };
    let t0 = if hi <= thresh { lo } else { c };
    if thresh <= lo {
        hi
    } else {
        t0
    }
}

const DEPTH_SCALE_TABLE: [u8; 9] = [0, 0xff, 0x55, 0, 0x11, 0, 0, 0, 0x01];

/// Adds an extra all-255 alpha channel; `img_n` must be 1 or 3, and the
/// source is the first `x * img_n` bytes of `buf` when `src` is `None`
/// (dest == src is legal). Translation of `stbi__create_png_alpha_expand8()`.
fn create_png_alpha_expand8(dest: &mut [u8], src: Option<&[u8]>, x: u32, img_n: i32) {
    // must process data backwards since we allow dest==src
    let x = x as usize;
    if img_n == 1 {
        for i in (0..x).rev() {
            let v = src.map_or(dest[i], |s| s[i]);
            dest[i * 2 + 1] = 255;
            dest[i * 2] = v;
        }
    } else {
        crate::sdl_assert!(img_n == 3);
        for i in (0..x).rev() {
            let (r, g, b) = match src {
                Some(s) => (s[i * 3], s[i * 3 + 1], s[i * 3 + 2]),
                None => (dest[i * 3], dest[i * 3 + 1], dest[i * 3 + 2]),
            };
            dest[i * 4 + 3] = 255;
            dest[i * 4 + 2] = b;
            dest[i * 4 + 1] = g;
            dest[i * 4] = r;
        }
    }
}

impl Png<'_, '_> {
    /// Create the png data from post-deflated data.
    /// Translation of `stbi__create_png_image_raw()`.
    #[allow(clippy::too_many_arguments)]
    fn create_png_image_raw(
        &mut self,
        raw: &[u8],
        out_n: i32,
        x: u32,
        y: u32,
        depth: i32,
        color: i32,
    ) -> Result<()> {
        let bytes = if depth == 16 { 2 } else { 1 };
        let stride = (x * out_n as u32 * bytes) as usize;
        let img_n = self.s.img_n; // copy it into a local for later

        let output_bytes = out_n * bytes as i32;
        let mut filter_bytes = (img_n * bytes as i32) as usize;
        let mut width = x as usize;

        crate::sdl_assert!(out_n == self.s.img_n || out_n == self.s.img_n + 1);
        // extra bytes to write off the end into
        self.out = match malloc_mad3(x as i32, y as i32, output_bytes, 0) {
            Some(o) => o,
            None => return err("Out of memory"),
        };

        // note: error exits here don't need to clean up a->out individually,
        // stbi__do_png always does on error.
        if !mad3sizes_valid(img_n, x as i32, depth, 7) {
            return err("Corrupt PNG");
        }
        let img_width_bytes = ((img_n as u32 * x * depth as u32) + 7) >> 3;
        if !mad2sizes_valid(img_width_bytes as i32, y as i32, img_width_bytes as i32) {
            return err("Corrupt PNG");
        }
        let img_len = (img_width_bytes + 1) * y;

        // we used to check for exact match between raw_len and img_len on non-interlaced PNGs,
        // but issue #276 reported a PNG in the wild that had extra data at the end (all zeros),
        // so just check for raw_len < img_len always.
        if (raw.len() as u32) < img_len {
            return err("Corrupt PNG");
        }

        // Allocate two scan lines worth of filter workspace buffer.
        let Some(mut filter_buf) = malloc_mad2(img_width_bytes as i32, 2, 0) else {
            return err("Out of memory");
        };
        let iwb = img_width_bytes as usize;

        // Filtering for low-bit-depth images
        if depth < 8 {
            filter_bytes = 1;
            width = iwb;
        }

        let mut raw_pos = 0usize;
        for j in 0..y as usize {
            // cur/prior filter buffers alternate
            let (lo, hi) = filter_buf.split_at_mut(iwb);
            let (cur, prior) = if j & 1 == 0 { (lo, &*hi) } else { (hi, &*lo) };
            let nk = width * filter_bytes;
            let mut filter = raw[raw_pos];
            raw_pos += 1;
            let raw_row = &raw[raw_pos..raw_pos + nk];

            // check filter type
            if filter > 4 {
                return err("Corrupt PNG");
            }

            // if first row, use special filter that doesn't sample previous row
            if j == 0 {
                filter = FIRST_ROW_FILTER[filter as usize];
            }

            // perform actual filtering
            match filter {
                STBI__F_NONE => cur[..nk].copy_from_slice(raw_row),
                STBI__F_SUB => {
                    cur[..filter_bytes].copy_from_slice(&raw_row[..filter_bytes]);
                    for k in filter_bytes..nk {
                        cur[k] = raw_row[k].wrapping_add(cur[k - filter_bytes]);
                    }
                }
                STBI__F_UP => {
                    for k in 0..nk {
                        cur[k] = raw_row[k].wrapping_add(prior[k]);
                    }
                }
                STBI__F_AVG => {
                    for k in 0..filter_bytes {
                        cur[k] = raw_row[k].wrapping_add(prior[k] >> 1);
                    }
                    for k in filter_bytes..nk {
                        cur[k] = raw_row[k].wrapping_add(
                            ((prior[k] as u32 + cur[k - filter_bytes] as u32) >> 1) as u8,
                        );
                    }
                }
                STBI__F_PAETH => {
                    for k in 0..filter_bytes {
                        cur[k] = raw_row[k].wrapping_add(prior[k]); // prior[k] == stbi__paeth(0,prior[k],0)
                    }
                    for k in filter_bytes..nk {
                        cur[k] = raw_row[k].wrapping_add(paeth(
                            cur[k - filter_bytes] as i32,
                            prior[k] as i32,
                            prior[k - filter_bytes] as i32,
                        ) as u8);
                    }
                }
                STBI__F_AVG_FIRST => {
                    cur[..filter_bytes].copy_from_slice(&raw_row[..filter_bytes]);
                    for k in filter_bytes..nk {
                        cur[k] = raw_row[k].wrapping_add(cur[k - filter_bytes] >> 1);
                    }
                }
                _ => {}
            }

            raw_pos += nk;

            // expand decoded bits in cur to dest, also adding an extra alpha channel if desired
            let dest = &mut self.out[stride * j..stride * (j + 1)];
            if depth < 8 {
                let scale = if color == 0 {
                    DEPTH_SCALE_TABLE[depth as usize]
                } else {
                    1
                }; // scale grayscale values to 0..255 range
                let nsmp = x as usize * img_n as usize;
                let mut inb = 0u8;
                let mut input = cur.iter();

                // expand bits to bytes first
                let (per_byte, shift) = match depth {
                    4 => (2, 4),
                    2 => (4, 6),
                    _ => {
                        crate::sdl_assert!(depth == 1);
                        (8, 7)
                    }
                };
                for (i, out) in dest[..nsmp].iter_mut().enumerate() {
                    if i % per_byte == 0 {
                        inb = *input.next().unwrap_or(&0);
                    }
                    *out = scale.wrapping_mul(inb >> shift);
                    inb = ((inb as u32) << depth) as u8;
                }

                // insert alpha=255 values if desired
                if img_n != out_n {
                    create_png_alpha_expand8(dest, None, x, img_n);
                }
            } else if depth == 8 {
                if img_n == out_n {
                    dest[..x as usize * img_n as usize]
                        .copy_from_slice(&cur[..x as usize * img_n as usize]);
                } else {
                    create_png_alpha_expand8(dest, Some(cur), x, img_n);
                }
            } else if depth == 16 {
                // convert the image data from big-endian to platform-native
                let nsmp = x as usize * img_n as usize;
                let be = |c: &[u8], i: usize| ((c[i * 2] as u16) << 8) | c[i * 2 + 1] as u16;
                let put = |dest: &mut [u8], i: usize, v: u16| {
                    dest[i * 2..i * 2 + 2].copy_from_slice(&v.to_ne_bytes());
                };

                if img_n == out_n {
                    for i in 0..nsmp {
                        put(dest, i, be(cur, i));
                    }
                } else {
                    crate::sdl_assert!(img_n + 1 == out_n);
                    if img_n == 1 {
                        for i in 0..x as usize {
                            put(dest, i * 2, be(cur, i));
                            put(dest, i * 2 + 1, 0xffff);
                        }
                    } else {
                        crate::sdl_assert!(img_n == 3);
                        for i in 0..x as usize {
                            put(dest, i * 4, be(cur, i * 3));
                            put(dest, i * 4 + 1, be(cur, i * 3 + 1));
                            put(dest, i * 4 + 2, be(cur, i * 3 + 2));
                            put(dest, i * 4 + 3, 0xffff);
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Translation of `stbi__create_png_image()`.
    fn create_png_image(
        &mut self,
        image_data: &[u8],
        out_n: i32,
        depth: i32,
        color: i32,
        interlaced: bool,
    ) -> Result<()> {
        let bytes = if depth == 16 { 2 } else { 1 };
        let out_bytes = (out_n * bytes) as usize;
        if !interlaced {
            return self.create_png_image_raw(
                image_data,
                out_n,
                self.s.img_x,
                self.s.img_y,
                depth,
                color,
            );
        }

        // de-interlacing
        let Some(mut final_) = malloc_mad3(
            self.s.img_x as i32,
            self.s.img_y as i32,
            out_bytes as i32,
            0,
        ) else {
            return err("Out of memory");
        };
        let mut image_data = image_data;
        for p in 0..7 {
            const XORIG: [u32; 7] = [0, 4, 0, 2, 0, 1, 0];
            const YORIG: [u32; 7] = [0, 0, 4, 0, 2, 0, 1];
            const XSPC: [u32; 7] = [8, 8, 4, 4, 2, 2, 1];
            const YSPC: [u32; 7] = [8, 8, 8, 4, 4, 2, 2];
            // pass1_x[4] = 0, pass1_x[5] = 1, pass1_x[12] = 1
            let x = (self
                .s
                .img_x
                .wrapping_sub(XORIG[p])
                .wrapping_add(XSPC[p] - 1))
                / XSPC[p];
            let y = (self
                .s
                .img_y
                .wrapping_sub(YORIG[p])
                .wrapping_add(YSPC[p] - 1))
                / YSPC[p];
            if x != 0 && y != 0 {
                let img_len = ((((self.s.img_n as u32 * x * depth as u32) + 7) >> 3) + 1) * y;
                self.create_png_image_raw(image_data, out_n, x, y, depth, color)?;
                for j in 0..y {
                    for i in 0..x {
                        let out_y = (j * YSPC[p] + YORIG[p]) as usize;
                        let out_x = (i * XSPC[p] + XORIG[p]) as usize;
                        let dst = out_y * self.s.img_x as usize * out_bytes + out_x * out_bytes;
                        let src = (j * x + i) as usize * out_bytes;
                        final_[dst..dst + out_bytes]
                            .copy_from_slice(&self.out[src..src + out_bytes]);
                    }
                }
                image_data = &image_data[img_len as usize..];
            }
        }
        self.out = final_;

        Ok(())
    }

    /// Translation of `stbi__compute_transparency()`.
    fn compute_transparency(&mut self, tc: [u8; 3], out_n: i32) {
        let pixel_count = (self.s.img_x * self.s.img_y) as usize;

        // compute color-based transparency, assuming we've
        // already got 255 as the alpha value in the output
        crate::sdl_assert!(out_n == 2 || out_n == 4);

        if out_n == 2 {
            for p in self.out.chunks_exact_mut(2).take(pixel_count) {
                p[1] = if p[0] == tc[0] { 0 } else { 255 };
            }
        } else {
            for p in self.out.chunks_exact_mut(4).take(pixel_count) {
                if p[0] == tc[0] && p[1] == tc[1] && p[2] == tc[2] {
                    p[3] = 0;
                }
            }
        }
    }

    /// Translation of `stbi__compute_transparency16()`.
    fn compute_transparency16(&mut self, tc: [u16; 3], out_n: i32) {
        let pixel_count = (self.s.img_x * self.s.img_y) as usize;
        let get = |p: &[u8], i: usize| u16::from_ne_bytes([p[i * 2], p[i * 2 + 1]]);

        // compute color-based transparency, assuming we've
        // already got 65535 as the alpha value in the output
        crate::sdl_assert!(out_n == 2 || out_n == 4);

        if out_n == 2 {
            for p in self.out.chunks_exact_mut(4).take(pixel_count) {
                let a: u16 = if get(p, 0) == tc[0] { 0 } else { 65535 };
                p[2..4].copy_from_slice(&a.to_ne_bytes());
            }
        } else {
            for p in self.out.chunks_exact_mut(8).take(pixel_count) {
                if get(p, 0) == tc[0] && get(p, 1) == tc[1] && get(p, 2) == tc[2] {
                    p[6..8].fill(0);
                }
            }
        }
    }

    /// Translation of `stbi__expand_png_palette()`.
    fn expand_png_palette(&mut self, palette: &[u8], pal_img_n: i32) -> Result<()> {
        let pixel_count = (self.s.img_x * self.s.img_y) as usize;
        let orig = &self.out;

        let Some(mut p) = malloc_mad2(pixel_count as i32, pal_img_n, 0) else {
            return err("Out of memory");
        };

        let n = pal_img_n as usize;
        for (i, dst) in p.chunks_exact_mut(n).enumerate() {
            let e = orig[i] as usize * 4;
            dst.copy_from_slice(&palette[e..e + n]);
        }
        self.out = p;

        Ok(())
    }

    /// Translation of `stbi__parse_png_file()` with `STBI__SCAN_load`.
    fn parse_png_file(
        &mut self,
        req_comp: i32,
        palette_buffer: Option<&mut [u8; 1024]>,
    ) -> Result<()> {
        let mut own_palette = [0u8; 1024];
        let has_palette_buffer = palette_buffer.is_some();
        let palette: &mut [u8; 1024] = match palette_buffer {
            Some(buf) => {
                if req_comp != 1 {
                    return err("req_comp must be 1 when loading paletted");
                }
                buf
            }
            None => &mut own_palette,
        };
        let mut pal_img_n = 0i32;
        let mut has_trans = false;
        let mut tc = [0u8; 3];
        let mut tc16 = [0u16; 3];
        let mut idata: Option<Vec<u8>> = None;
        let mut ioff = 0u32;
        let mut pal_len = 0u32;
        let mut first = true;
        let mut interlace = 0;
        let mut color = 0;
        let mut is_iphone = false;

        self.out = Vec::new();

        check_png_header(self.s)?;

        const fn png_type(t: &[u8; 4]) -> u32 {
            u32::from_be_bytes(*t)
        }

        loop {
            let c = get_chunk_header(self.s);
            match c.ty {
                t if t == png_type(b"CgBI") => {
                    is_iphone = true;
                    self.s.skip(c.length as i32);
                }
                t if t == png_type(b"IHDR") => {
                    if !first {
                        return err("Corrupt PNG");
                    }
                    first = false;
                    if c.length != 13 {
                        return err("Corrupt PNG");
                    }
                    self.s.img_x = self.s.get32be();
                    self.s.img_y = self.s.get32be();
                    if self.s.img_y > STBI_MAX_DIMENSIONS {
                        return err("Very large image (corrupt?)");
                    }
                    if self.s.img_x > STBI_MAX_DIMENSIONS {
                        return err("Very large image (corrupt?)");
                    }
                    self.depth = self.s.get8() as i32;
                    if ![1, 2, 4, 8, 16].contains(&self.depth) {
                        return err("PNG not supported: 1/2/4/8/16-bit only");
                    }
                    color = self.s.get8() as i32;
                    if color > 6 {
                        return err("Corrupt PNG");
                    }
                    if color == 3 && self.depth == 16 {
                        return err("Corrupt PNG");
                    }
                    if color == 3 {
                        pal_img_n = 3;
                    } else if color & 1 != 0 {
                        return err("Corrupt PNG");
                    }
                    let comp = self.s.get8();
                    if comp != 0 {
                        return err("Corrupt PNG");
                    }
                    let filter = self.s.get8();
                    if filter != 0 {
                        return err("Corrupt PNG");
                    }
                    interlace = self.s.get8();
                    if interlace > 1 {
                        return err("Corrupt PNG");
                    }
                    if self.s.img_x == 0 || self.s.img_y == 0 {
                        return err("Corrupt PNG");
                    }
                    if pal_img_n == 0 {
                        self.s.img_n = (if color & 2 != 0 { 3 } else { 1 })
                            + (if color & 4 != 0 { 1 } else { 0 });
                        if (1u32 << 30) / self.s.img_x / (self.s.img_n as u32) < self.s.img_y {
                            return err("Image too large to decode");
                        }
                    } else {
                        // if paletted, then pal_n is our final components, and
                        // img_n is # components to decompress/filter.
                        self.s.img_n = 1;
                        if (1u32 << 30) / self.s.img_x / 4 < self.s.img_y {
                            return err("Corrupt PNG");
                        }
                    }
                    // even with SCAN_header, have to scan to see if we have a tRNS
                }

                t if t == png_type(b"PLTE") => {
                    if first {
                        return err("Corrupt PNG");
                    }
                    if c.length > 256 * 3 {
                        return err("Corrupt PNG");
                    }
                    pal_len = c.length / 3;
                    if pal_len * 3 != c.length {
                        return err("Corrupt PNG");
                    }
                    for i in 0..pal_len as usize {
                        palette[i * 4] = self.s.get8();
                        palette[i * 4 + 1] = self.s.get8();
                        palette[i * 4 + 2] = self.s.get8();
                        palette[i * 4 + 3] = 255;
                    }
                }

                t if t == png_type(b"tRNS") => {
                    if first {
                        return err("Corrupt PNG");
                    }
                    if idata.is_some() {
                        return err("Corrupt PNG");
                    }
                    if pal_img_n != 0 {
                        if pal_len == 0 {
                            return err("Corrupt PNG");
                        }
                        if c.length > pal_len {
                            return err("Corrupt PNG");
                        }
                        pal_img_n = 4;
                        for i in 0..c.length as usize {
                            palette[i * 4 + 3] = self.s.get8();
                        }
                    } else {
                        if self.s.img_n & 1 == 0 {
                            return err("Corrupt PNG");
                        }
                        if c.length != self.s.img_n as u32 * 2 {
                            return err("Corrupt PNG");
                        }
                        has_trans = true;
                        // non-paletted with tRNS = constant alpha. if header-scanning, we can stop now.
                        if self.depth == 16 {
                            for t in tc16.iter_mut().take(self.s.img_n.min(3) as usize) {
                                *t = self.s.get16be() as u16; // copy the values as-is
                            }
                        } else {
                            for t in tc.iter_mut().take(self.s.img_n.min(3) as usize) {
                                // non 8-bit images will be larger
                                *t = ((self.s.get16be() & 255) as u8)
                                    .wrapping_mul(DEPTH_SCALE_TABLE[self.depth as usize]);
                            }
                        }
                    }
                }

                t if t == png_type(b"IDAT") => {
                    if first {
                        return err("Corrupt PNG");
                    }
                    if pal_img_n != 0 && pal_len == 0 {
                        return err("Corrupt PNG");
                    }
                    if c.length > (1u32 << 30) {
                        return err("IDAT section larger than 2^30 bytes");
                    }
                    if (ioff.wrapping_add(c.length) as i32) < ioff as i32 {
                        // (upstream fails without setting an error)
                        return err("Corrupt PNG");
                    }
                    let buf = idata.get_or_insert_with(Vec::new);
                    if (ioff + c.length) as usize > buf.len() {
                        let mut idata_limit = buf.len() as u32;
                        if idata_limit == 0 {
                            idata_limit = c.length.max(4096);
                        }
                        while ioff + c.length > idata_limit {
                            idata_limit *= 2;
                        }
                        buf.resize(idata_limit as usize, 0);
                    }
                    if !self
                        .s
                        .getn(&mut buf[ioff as usize..(ioff + c.length) as usize])
                    {
                        return err("Corrupt PNG");
                    }
                    ioff += c.length;
                }

                t if t == png_type(b"IEND") => {
                    if first {
                        return err("Corrupt PNG");
                    }
                    let Some(data) = idata.take() else {
                        return err("Corrupt PNG");
                    };
                    // initial guess for decoded data size to avoid unnecessary reallocs
                    let bpl = (self.s.img_x * self.depth as u32).div_ceil(8); // bytes per line, per component
                    let raw_len = bpl
                        .wrapping_mul(self.s.img_y)
                        .wrapping_mul(self.s.img_n as u32)
                        .wrapping_add(self.s.img_y); /* pixels */
                    /* filter mode per row */
                    let expanded = zlib_decode_malloc_guesssize_headerflag(
                        &data[..ioff as usize],
                        raw_len as i32,
                        !is_iphone,
                    )?; // zlib should set error
                    drop(data);
                    if (req_comp == self.s.img_n + 1 && req_comp != 3 && pal_img_n == 0)
                        || has_trans
                    {
                        self.s.img_out_n = self.s.img_n + 1;
                    } else {
                        self.s.img_out_n = self.s.img_n;
                    }
                    self.create_png_image(
                        &expanded,
                        self.s.img_out_n,
                        self.depth,
                        color,
                        interlace != 0,
                    )?;
                    if has_trans {
                        if self.depth == 16 {
                            self.compute_transparency16(tc16, self.s.img_out_n);
                        } else {
                            self.compute_transparency(tc, self.s.img_out_n);
                        }
                    }
                    // (stbi__de_iphone() runs only with stbi__de_iphone_flag, which SDL never sets)
                    let _ = is_iphone;
                    if pal_img_n != 0 {
                        // pal_img_n == 3 or 4
                        self.s.img_n = pal_img_n; // record the actual colors we had
                        self.s.img_out_n = pal_img_n;
                        if req_comp >= 3 {
                            self.s.img_out_n = req_comp;
                        }
                        if !has_palette_buffer {
                            let pal = *palette;
                            self.expand_png_palette(&pal, self.s.img_out_n)?;
                        }
                    } else if has_trans {
                        // non-paletted image with tRNS -> source image has (constant) alpha
                        self.s.img_n += 1;
                    }
                    // end of PNG chunk, read and skip CRC
                    self.s.get32be();
                    if self.s.has_io() && self.s.img_buffer_end > self.s.img_buffer {
                        // rewind the additional bytes that have been read to the buffer
                        let back = self.s.img_buffer as i32 - self.s.img_buffer_end as i32;
                        self.s.io_skip(back);
                    }
                    return Ok(());
                }

                _ => {
                    // if critical, fail
                    if first {
                        return err("Corrupt PNG");
                    }
                    if (c.ty & (1 << 29)) == 0 {
                        return err("PNG not supported: unknown PNG chunk type");
                    }
                    self.s.skip(c.length as i32);
                }
            }
            // end of PNG chunk, read and skip CRC
            self.s.get32be();
        }
    }
}

/// Translation of `stbi__do_png()` / `stbi__png_load()`.
pub(crate) fn png_load(
    s: &mut Context<'_>,
    req_comp: i32,
    palette_buffer: Option<&mut [u8; 1024]>,
) -> Result<Image<Pixels>> {
    let has_palette_buffer = palette_buffer.is_some();
    if has_palette_buffer && req_comp != 1 {
        return err("req_comp must be 1 if loading paletted image without expansion");
    }
    if !(0..=4).contains(&req_comp) {
        return err("Internal error");
    }
    let mut p = Png {
        s,
        out: Vec::new(),
        depth: 0,
    };
    p.parse_png_file(req_comp, palette_buffer)?;
    let bits_per_channel = if p.depth <= 8 {
        8
    } else if p.depth == 16 {
        16
    } else {
        return err("PNG not supported: unsupported color depth");
    };
    let out = std::mem::take(&mut p.out);
    let (img_x, img_y, img_out_n) = (p.s.img_x, p.s.img_y, p.s.img_out_n);
    let mut result = if bits_per_channel == 8 {
        Pixels::Bits8(out)
    } else {
        Pixels::Bits16(
            out.chunks_exact(2)
                .map(|b| u16::from_ne_bytes([b[0], b[1]]))
                .collect(),
        )
    };
    if req_comp != 0 && req_comp != img_out_n {
        if has_palette_buffer {
        } else {
            result = match result {
                Pixels::Bits8(d) => {
                    Pixels::Bits8(convert_format8(d, img_out_n, req_comp, img_x, img_y)?)
                }
                Pixels::Bits16(d) => {
                    Pixels::Bits16(convert_format16(d, img_out_n, req_comp, img_x, img_y)?)
                }
            };
        }
        p.s.img_out_n = req_comp;
    }
    Ok(Image {
        x: img_x as i32,
        y: img_y as i32,
        comp: if has_palette_buffer { 1 } else { p.s.img_n },
        data: result,
    })
}

/// Translation of `stbi__png_test()`.
pub(crate) fn png_test(s: &mut Context<'_>) -> bool {
    let r = check_png_header(s).is_ok();
    s.rewind();
    r
}
