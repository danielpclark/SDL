// Rust translation of the zlib decoder of src/video/stb_image.h, from
// Simple DirectMedia Layer.
// public domain zlib decode    v0.2  Sean Barrett 2006-11-18
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The zlib decoder PNG reads its image data with.
//!
//! Simple implementation: all input must be provided in an upfront buffer
//! and all output is written to a single output buffer; fast huffman.

use super::stb_image::err;
use crate::error::Result;

// fast-way is faster to check than jpeg huffman, but slow way is slower
const STBI__ZFAST_BITS: u32 = 9; // accelerate all cases in default tables
const STBI__ZFAST_MASK: u32 = (1 << STBI__ZFAST_BITS) - 1;
const STBI__ZNSYMS: usize = 288; // number of symbols in literal/length alphabet

/// zlib-style huffman encoding (jpegs packs from left, zlib from right, so
/// can't share code). Translation of `stbi__zhuffman`.
#[derive(Clone)]
struct ZHuffman {
    fast: [u16; 1 << STBI__ZFAST_BITS],
    firstcode: [u16; 16],
    maxcode: [i32; 17],
    firstsymbol: [u16; 16],
    size: [u8; STBI__ZNSYMS],
    value: [u16; STBI__ZNSYMS],
}

impl Default for ZHuffman {
    fn default() -> Self {
        ZHuffman {
            fast: [0; 1 << STBI__ZFAST_BITS],
            firstcode: [0; 16],
            maxcode: [0; 17],
            firstsymbol: [0; 16],
            size: [0; STBI__ZNSYMS],
            value: [0; STBI__ZNSYMS],
        }
    }
}

/// Translation of `stbi__bitreverse16()`.
fn bitreverse16(mut n: i32) -> i32 {
    n = ((n & 0xAAAA) >> 1) | ((n & 0x5555) << 1);
    n = ((n & 0xCCCC) >> 2) | ((n & 0x3333) << 2);
    n = ((n & 0xF0F0) >> 4) | ((n & 0x0F0F) << 4);
    n = ((n & 0xFF00) >> 8) | ((n & 0x00FF) << 8);
    n
}

/// Translation of `stbi__bit_reverse()`.
fn bit_reverse(v: i32, bits: i32) -> i32 {
    crate::sdl_assert!(bits <= 16);
    // to bit reverse n bits, reverse 16 and shift
    // e.g. 11 bits, bit reverse and shift away 5
    bitreverse16(v) >> (16 - bits)
}

/// Translation of `stbi__zbuild_huffman()`.
fn zbuild_huffman(z: &mut ZHuffman, sizelist: &[u8]) -> Result<()> {
    let mut k = 0i32;
    let mut next_code = [0i32; 16];
    let mut sizes = [0i32; 17];

    // DEFLATE spec for generating codes
    z.fast.fill(0);
    for &s in sizelist {
        sizes[s as usize] += 1;
    }
    sizes[0] = 0;
    for (i, &s) in sizes.iter().enumerate().take(16).skip(1) {
        if s > (1 << i) {
            return err("Corrupt PNG");
        }
    }
    let mut code = 0i32;
    for i in 1..16 {
        next_code[i] = code;
        z.firstcode[i] = code as u16;
        z.firstsymbol[i] = k as u16;
        code += sizes[i];
        if sizes[i] != 0 && code > (1 << i) {
            return err("Corrupt PNG");
        }
        z.maxcode[i] = code << (16 - i); // preshift for inner loop
        code <<= 1;
        k += sizes[i];
    }
    z.maxcode[16] = 0x10000; // sentinel
    for (i, &s) in sizelist.iter().enumerate() {
        let s = s as usize;
        if s != 0 {
            let c = (next_code[s] - z.firstcode[s] as i32 + z.firstsymbol[s] as i32) as usize;
            let fastv = ((s << 9) | i) as u16;
            z.size[c] = s as u8;
            z.value[c] = i as u16;
            if s <= STBI__ZFAST_BITS as usize {
                let mut j = bit_reverse(next_code[s], s as i32);
                while j < (1 << STBI__ZFAST_BITS) {
                    z.fast[j as usize] = fastv;
                    j += 1 << s;
                }
            }
            next_code[s] += 1;
        }
    }
    Ok(())
}

/// zlib-from-memory implementation for PNG reading: because PNG allows
/// splitting the zlib stream arbitrarily, and it's annoying structurally to
/// have PNG call ZLIB call PNG, we require PNG read all the IDATs and
/// combine them into a single memory buffer. Translation of `stbi__zbuf`.
struct ZBuf<'a> {
    br: BitReader<'a>,

    zout: Vec<u8>,
    z_expandable: bool,
    /// The output limit (`zout_end - zout_start`).
    limit: u32,

    z_length: ZHuffman,
    z_distance: ZHuffman,
}

/// The input side of `stbi__zbuf`.
struct BitReader<'a> {
    zbuffer: &'a [u8],
    pos: usize,
    num_bits: i32,
    hit_zeof_once: bool,
    code_buffer: u32,
}

const ZLENGTH_BASE: [i32; 31] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258, 0, 0,
];

const ZLENGTH_EXTRA: [i32; 31] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0, 0, 0,
];

const ZDIST_BASE: [i32; 32] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577, 0, 0,
];

const ZDIST_EXTRA: [i32; 32] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13, 0, 0,
];

/// Translation of `stbi__zdefault_length`.
const ZDEFAULT_LENGTH: [u8; STBI__ZNSYMS] = {
    let mut t = [0u8; STBI__ZNSYMS];
    let mut i = 0;
    while i < STBI__ZNSYMS {
        t[i] = if i <= 143 {
            8
        } else if i <= 255 {
            9
        } else if i <= 279 {
            7
        } else {
            8
        };
        i += 1;
    }
    t
};

/// Translation of `stbi__zdefault_distance`.
const ZDEFAULT_DISTANCE: [u8; 32] = [5; 32];

impl BitReader<'_> {
    /// Translation of `stbi__zeof()`.
    fn zeof(&self) -> bool {
        self.pos >= self.zbuffer.len()
    }

    /// Translation of `stbi__zget8()`.
    fn zget8(&mut self) -> u8 {
        if self.zeof() {
            0
        } else {
            let b = self.zbuffer[self.pos];
            self.pos += 1;
            b
        }
    }

    /// Translation of `stbi__fill_bits()`.
    fn fill_bits(&mut self) {
        loop {
            if self.code_buffer as u64 >= (1u64 << self.num_bits) {
                self.pos = self.zbuffer.len(); /* treat this as EOF so we fail. */
                return;
            }
            self.code_buffer |= (self.zget8() as u32) << self.num_bits;
            self.num_bits += 8;
            if self.num_bits > 24 {
                break;
            }
        }
    }

    /// Translation of `stbi__zreceive()`.
    fn zreceive(&mut self, n: i32) -> u32 {
        if self.num_bits < n {
            self.fill_bits();
        }
        let k = self.code_buffer & ((1u32 << n) - 1);
        self.code_buffer >>= n;
        self.num_bits -= n;
        k
    }

    /// Translation of `stbi__zhuffman_decode_slowpath()`.
    fn zhuffman_decode_slowpath(&mut self, z: &ZHuffman) -> i32 {
        // not resolved by fast table, so compute it the slow way
        // use jpeg approach, which requires MSbits at top
        let k = bit_reverse(self.code_buffer as i32, 16);
        let mut s = STBI__ZFAST_BITS as usize + 1;
        loop {
            if k < z.maxcode[s] {
                break;
            }
            s += 1;
        }
        if s >= 16 {
            return -1; // invalid code!
        }
        // code size is s, so:
        let b = (k >> (16 - s)) - z.firstcode[s] as i32 + z.firstsymbol[s] as i32;
        if b < 0 || b as usize >= STBI__ZNSYMS {
            return -1; // some data was corrupt somewhere!
        }
        if z.size[b as usize] as usize != s {
            return -1; // was originally an assert, but report failure instead.
        }
        self.code_buffer >>= s;
        self.num_bits -= s as i32;
        z.value[b as usize] as i32
    }

    /// Translation of `stbi__zhuffman_decode()`.
    fn zhuffman_decode(&mut self, z: &ZHuffman) -> i32 {
        if self.num_bits < 16 {
            if self.zeof() {
                if !self.hit_zeof_once {
                    // This is the first time we hit eof, insert 16 extra padding btis
                    // to allow us to keep going; if we actually consume any of them
                    // though, that is invalid data. This is caught later.
                    self.hit_zeof_once = true;
                    self.num_bits += 16; // add 16 implicit zero bits
                } else {
                    // We already inserted our extra 16 padding bits and are again
                    // out, this stream is actually prematurely terminated.
                    return -1;
                }
            } else {
                self.fill_bits();
            }
        }
        let b = z.fast[(self.code_buffer & STBI__ZFAST_MASK) as usize] as i32;
        if b != 0 {
            let s = b >> 9;
            self.code_buffer >>= s;
            self.num_bits -= s;
            return b & 511;
        }
        self.zhuffman_decode_slowpath(z)
    }
}

impl ZBuf<'_> {
    /// Make room for `n` more bytes of output. Translation of `stbi__zexpand()`.
    fn zexpand(&mut self, n: u32) -> Result<()> {
        if !self.z_expandable {
            return err("Corrupt PNG");
        }
        let cur = self.zout.len() as u32;
        let mut limit = self.limit;
        if u32::MAX - cur < n {
            return err("Out of memory");
        }
        while cur + n > limit {
            if limit > u32::MAX / 2 {
                return err("Out of memory");
            }
            limit *= 2;
        }
        self.zout.reserve(limit as usize - self.zout.len());
        self.limit = limit;
        Ok(())
    }

    /// Translation of `stbi__parse_huffman_block()`.
    fn parse_huffman_block(&mut self) -> Result<()> {
        loop {
            let mut z = self.br.zhuffman_decode(&self.z_length);
            if z < 256 {
                if z < 0 {
                    return err("Corrupt PNG"); // error in huffman codes
                }
                if self.zout.len() as u32 >= self.limit {
                    self.zexpand(1)?;
                }
                self.zout.push(z as u8);
            } else {
                if z == 256 {
                    if self.br.hit_zeof_once && self.br.num_bits < 16 {
                        // The first time we hit zeof, we inserted 16 extra zero bits into our bit
                        // buffer so the decoder can just do its speculative decoding. But if we
                        // actually consumed any of those bits (which is the case when num_bits < 16),
                        // the stream actually read past the end so it is malformed.
                        return err("Corrupt PNG");
                    }
                    return Ok(());
                }
                if z >= 286 {
                    return err("Corrupt PNG"); // per DEFLATE, length codes 286 and 287 must not appear in compressed data
                }
                z -= 257;
                let mut len = ZLENGTH_BASE[z as usize];
                if ZLENGTH_EXTRA[z as usize] != 0 {
                    len += self.br.zreceive(ZLENGTH_EXTRA[z as usize]) as i32;
                }
                z = self.br.zhuffman_decode(&self.z_distance);
                if !(0..30).contains(&z) {
                    return err("Corrupt PNG"); // per DEFLATE, distance codes 30 and 31 must not appear in compressed data
                }
                let mut dist = ZDIST_BASE[z as usize];
                if ZDIST_EXTRA[z as usize] != 0 {
                    dist += self.br.zreceive(ZDIST_EXTRA[z as usize]) as i32;
                }
                if (self.zout.len() as i32) < dist {
                    return err("Corrupt PNG");
                }
                if len as i64 > self.limit as i64 - self.zout.len() as i64 {
                    self.zexpand(len as u32)?;
                }
                let start = self.zout.len() - dist as usize;
                if dist == 1 {
                    // run of one byte; common in images.
                    let v = self.zout[start];
                    self.zout.extend(std::iter::repeat_n(v, len as usize));
                } else {
                    for i in 0..len as usize {
                        let b = self.zout[start + i];
                        self.zout.push(b);
                    }
                }
            }
        }
    }

    /// Translation of `stbi__compute_huffman_codes()`.
    fn compute_huffman_codes(&mut self) -> Result<()> {
        const LENGTH_DEZIGZAG: [u8; 19] = [
            16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
        ];
        let mut z_codelength = ZHuffman::default();
        let mut lencodes = [0u8; 286 + 32 + 137]; //padding for maximum single op
        let mut codelength_sizes = [0u8; 19];

        let hlit = self.br.zreceive(5) as usize + 257;
        let hdist = self.br.zreceive(5) as usize + 1;
        let hclen = self.br.zreceive(4) as usize + 4;
        let ntot = hlit + hdist;

        for &d in &LENGTH_DEZIGZAG[..hclen] {
            let s = self.br.zreceive(3);
            codelength_sizes[d as usize] = s as u8;
        }
        zbuild_huffman(&mut z_codelength, &codelength_sizes)?;

        let mut n = 0usize;
        while n < ntot {
            let mut c = self.br.zhuffman_decode(&z_codelength);
            if !(0..19).contains(&c) {
                return err("Corrupt PNG");
            }
            if c < 16 {
                lencodes[n] = c as u8;
                n += 1;
            } else {
                let mut fill = 0u8;
                if c == 16 {
                    c = self.br.zreceive(2) as i32 + 3;
                    if n == 0 {
                        return err("Corrupt PNG");
                    }
                    fill = lencodes[n - 1];
                } else if c == 17 {
                    c = self.br.zreceive(3) as i32 + 3;
                } else if c == 18 {
                    c = self.br.zreceive(7) as i32 + 11;
                } else {
                    return err("Corrupt PNG");
                }
                if ntot - n < c as usize {
                    return err("Corrupt PNG");
                }
                lencodes[n..n + c as usize].fill(fill);
                n += c as usize;
            }
        }
        if n != ntot {
            return err("Corrupt PNG");
        }
        let mut length = ZHuffman::default();
        let mut distance = ZHuffman::default();
        zbuild_huffman(&mut length, &lencodes[..hlit])?;
        self.z_length = length;
        zbuild_huffman(&mut distance, &lencodes[hlit..hlit + hdist])?;
        self.z_distance = distance;
        Ok(())
    }

    /// Translation of `stbi__parse_uncompressed_block()`.
    fn parse_uncompressed_block(&mut self) -> Result<()> {
        let mut header = [0u8; 4];
        if self.br.num_bits & 7 != 0 {
            self.br.zreceive(self.br.num_bits & 7); // discard
        }
        // drain the bit-packed data into header
        let mut k = 0;
        while self.br.num_bits > 0 {
            header[k] = (self.br.code_buffer & 255) as u8; // suppress MSVC run-time check
            k += 1;
            self.br.code_buffer >>= 8;
            self.br.num_bits -= 8;
        }
        if self.br.num_bits < 0 {
            return err("Corrupt PNG");
        }
        // now fill header the normal way
        while k < 4 {
            header[k] = self.br.zget8();
            k += 1;
        }
        let len = header[1] as usize * 256 + header[0] as usize;
        let nlen = header[3] as usize * 256 + header[2] as usize;
        if nlen != (len ^ 0xffff) {
            return err("Corrupt PNG");
        }
        if self.br.pos + len > self.br.zbuffer.len() {
            return err("Corrupt PNG");
        }
        if self.zout.len() + len > self.limit as usize {
            self.zexpand(len as u32)?;
        }
        self.zout
            .extend_from_slice(&self.br.zbuffer[self.br.pos..self.br.pos + len]);
        self.br.pos += len;
        Ok(())
    }

    /// Translation of `stbi__parse_zlib_header()`.
    fn parse_zlib_header(&mut self) -> Result<()> {
        let cmf = self.br.zget8() as i32;
        let cm = cmf & 15;
        /* int cinfo = cmf >> 4; */
        let flg = self.br.zget8() as i32;
        if self.br.zeof() {
            return err("Corrupt PNG"); // zlib spec
        }
        if (cmf * 256 + flg) % 31 != 0 {
            return err("Corrupt PNG"); // zlib spec
        }
        if flg & 32 != 0 {
            return err("Corrupt PNG"); // preset dictionary not allowed in png
        }
        if cm != 8 {
            return err("Corrupt PNG"); // DEFLATE required for png
        }
        // window = 1 << (8 + cinfo)... but who cares, we fully buffer output
        Ok(())
    }

    /// Translation of `stbi__parse_zlib()`.
    fn parse_zlib(&mut self, parse_header: bool) -> Result<()> {
        if parse_header {
            self.parse_zlib_header()?;
        }
        self.br.num_bits = 0;
        self.br.code_buffer = 0;
        self.br.hit_zeof_once = false;
        loop {
            let last = self.br.zreceive(1);
            let ty = self.br.zreceive(2);
            if ty == 0 {
                self.parse_uncompressed_block()?;
            } else if ty == 3 {
                // (upstream fails without setting an error)
                return err("Corrupt PNG");
            } else {
                if ty == 1 {
                    // use fixed code lengths
                    zbuild_huffman(&mut self.z_length, &ZDEFAULT_LENGTH)?;
                    zbuild_huffman(&mut self.z_distance, &ZDEFAULT_DISTANCE)?;
                } else {
                    self.compute_huffman_codes()?;
                }
                self.parse_huffman_block()?;
            }
            if last != 0 {
                break;
            }
        }
        Ok(())
    }
}

/// Decode a zlib stream (or, without `parse_header`, raw deflate data)
/// into a growing buffer that starts at `initial_size` bytes.
/// Translation of `stbi_zlib_decode_malloc_guesssize_headerflag()`.
pub(crate) fn zlib_decode_malloc_guesssize_headerflag(
    buffer: &[u8],
    initial_size: i32,
    parse_header: bool,
) -> Result<Vec<u8>> {
    let mut a = ZBuf {
        br: BitReader {
            zbuffer: buffer,
            pos: 0,
            num_bits: 0,
            hit_zeof_once: false,
            code_buffer: 0,
        },
        zout: Vec::with_capacity(initial_size.max(0) as usize),
        z_expandable: true,
        limit: initial_size.max(0) as u32,
        z_length: ZHuffman::default(),
        z_distance: ZHuffman::default(),
    };
    a.parse_zlib(parse_header)?;
    Ok(a.zout)
}
