// Rust translation of the JPEG decoder of src/video/stb_image.h, from
// Simple DirectMedia Layer.
// stb_image - v2.30 - public domain image loader - http://nothings.org/stb
// SDL's NV12 output is Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>.
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The "baseline" JPEG/JFIF decoder: baseline and progressive, 8 bits per
//! channel, grayscale/YCbCr/RGB/CMYK/YCCK, and (an SDL addition) decoding
//! straight to NV12.
//!
//!  - doesn't support delayed output of y-dimension
//!  - doesn't try to recover corrupt jpegs
//!  - upsampled channels are bilinearly interpolated, even across blocks
//!  - quality integer IDCT derived from IJG's 'slow'
//!
//! stb's SSE2 and NEON kernels (IDCT, YCbCr to RGB, 2x2 upsampling) are
//! written to give bit-identical results to the scalar ones translated here.

// stb's and miniz's constants are written with more digits than f32
// holds, and their loops index several arrays at once; both kept as written.
#![allow(
    clippy::excessive_precision,
    clippy::needless_range_loop,
    clippy::too_many_arguments
)]
use super::stb_image::{
    addints_valid, compute_y, err, mad3sizes_valid, malloc_mad2, mul2shorts_valid, Context, Image,
    STBI_MAX_DIMENSIONS,
};
use crate::error::Result;

// huffman decoding acceleration
const FAST_BITS: u32 = 9; // larger handles more cases; smaller stomps less cache

/// Translation of `stbi__huffman`.
struct Huffman {
    fast: [u8; 1 << FAST_BITS],
    // weirdly, repacking this into AoS is a 10% speed loss, instead of a win
    code: [u16; 256],
    values: [u8; 256],
    size: [u8; 257],
    maxcode: [u32; 18],
    delta: [i32; 17], // old 'firstsymbol' - old 'firstcode'
}

impl Default for Huffman {
    fn default() -> Self {
        Huffman {
            fast: [0; 1 << FAST_BITS],
            code: [0; 256],
            values: [0; 256],
            size: [0; 257],
            maxcode: [0; 18],
            delta: [0; 17],
        }
    }
}

/// Definition of jpeg image component. Translation of `stbi__jpeg::img_comp[]`.
#[derive(Default)]
struct ImgComp {
    id: i32,
    h: i32,
    v: i32,
    tq: i32,
    hd: i32,
    ha: i32,
    dc_pred: i32,

    x: i32,
    y: i32,
    w2: i32,
    h2: i32,
    data: Vec<u8>,
    linebuf: Vec<u8>,
    coeff: Vec<i16>, // progressive only
    coeff_w: i32,    // number of 8x8 coefficient blocks
    coeff_h: i32,
}

/// Translation of `stbi__jpeg`.
struct Jpeg<'c, 'a> {
    s: &'c mut Context<'a>,
    huff_dc: [Huffman; 4],
    huff_ac: [Huffman; 4],
    dequant: [[u16; 64]; 4],
    fast_ac: [[i16; 1 << FAST_BITS]; 4],

    // sizes for components, interleaved MCUs
    img_h_max: i32,
    img_v_max: i32,
    img_mcu_x: i32,
    img_mcu_y: i32,
    img_mcu_w: i32,
    img_mcu_h: i32,

    img_comp: [ImgComp; 4],

    code_buffer: u32, // jpeg entropy-coded buffer
    code_bits: i32,   // number of valid bits
    marker: u8,       // marker seen while filling entropy buffer
    nomore: bool,     // flag if we saw a marker so must stop

    progressive: bool,
    spec_start: i32,
    spec_end: i32,
    succ_high: i32,
    succ_low: i32,
    eob_run: i32,
    jfif: bool,
    app14_color_transform: i32, // Adobe APP14 tag
    rgb: i32,

    scan_n: i32,
    order: [usize; 4],
    restart_interval: i32,
    todo: i32,
}

/// Where SDL's MJPG conversion wants NV12 output. Translation of `stbi__nv12`.
pub(crate) struct Nv12<'b> {
    pub(crate) w: i32,
    pub(crate) h: i32,
    pub(crate) pitch: i32,
    /// The Y plane followed by the interleaved UV plane.
    pub(crate) dst: &'b mut [u8],
}

const STBI__SCAN_LOAD: i32 = 0;
const STBI__SCAN_TYPE: i32 = 1;

/// Translation of `stbi__build_huffman()`.
fn build_huffman(h: &mut Huffman, count: &[i32; 16]) -> Result<()> {
    let mut k = 0usize;
    // build size list for each symbol (from JPEG spec)
    for (i, &c) in count.iter().enumerate() {
        for _ in 0..c {
            h.size[k] = (i + 1) as u8;
            k += 1;
            if k >= 257 {
                return err("Corrupt JPEG");
            }
        }
    }
    h.size[k] = 0;

    // compute actual symbols (from jpeg spec)
    let mut code = 0u32;
    k = 0;
    let mut j = 1usize;
    while j <= 16 {
        // compute delta to add to code to compute symbol id
        h.delta[j] = k as i32 - code as i32;
        if h.size[k] as usize == j {
            while h.size[k] as usize == j {
                h.code[k] = code as u16;
                k += 1;
                code += 1;
            }
            if code > (1u32 << j) {
                return err("Corrupt JPEG");
            }
        }
        // compute largest code + 1 for this size, preshifted as needed later
        h.maxcode[j] = code << (16 - j);
        code <<= 1;
        j += 1;
    }
    h.maxcode[j] = 0xffffffff;

    // build non-spec acceleration table; 255 is flag for not-accelerated
    h.fast.fill(255);
    for i in 0..k {
        let s = h.size[i] as u32;
        if s <= FAST_BITS {
            let c = (h.code[i] as usize) << (FAST_BITS - s);
            let m = 1usize << (FAST_BITS - s);
            for j in 0..m {
                h.fast[c + j] = i as u8;
            }
        }
    }
    Ok(())
}

/// Build a table that decodes both magnitude and value of small ACs in one
/// go. Translation of `stbi__build_fast_ac()`.
fn build_fast_ac(fast_ac: &mut [i16; 1 << FAST_BITS], h: &Huffman) {
    for i in 0..(1usize << FAST_BITS) {
        let fast = h.fast[i];
        fast_ac[i] = 0;
        if fast < 255 {
            let rs = h.values[fast as usize] as i32;
            let run = (rs >> 4) & 15;
            let magbits = rs & 15;
            let len = h.size[fast as usize] as i32;

            if magbits != 0 && len + magbits <= FAST_BITS as i32 {
                // magnitude code followed by receive_extend code
                let mut k =
                    (((i as i32) << len) & ((1 << FAST_BITS) - 1)) >> (FAST_BITS as i32 - magbits);
                let m = 1 << (magbits - 1);
                if k < m {
                    k = k.wrapping_add((!0u32 << magbits) as i32).wrapping_add(1);
                }
                // if the result is small enough, we can fit it in fast_ac table
                if (-128..=127).contains(&k) {
                    fast_ac[i] = ((k * 256) + (run * 16) + (len + magbits)) as i16;
                }
            }
        }
    }
}

// (1 << n) - 1
const STBI__BMASK: [u32; 17] = [
    0, 1, 3, 7, 15, 31, 63, 127, 255, 511, 1023, 2047, 4095, 8191, 16383, 32767, 65535,
];

// bias[n] = (-1<<n) + 1
const STBI__JBIAS: [i32; 16] = [
    0, -1, -3, -7, -15, -31, -63, -127, -255, -511, -1023, -2047, -4095, -8191, -16383, -32767,
];

// given a value that's at position X in the zigzag stream,
// where does it appear in the 8x8 matrix coded as row-major?
const STBI__JPEG_DEZIGZAG: [u8; 64 + 15] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20,
    13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59,
    52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
    // let corrupt input sample past end
    63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63,
];

const STBI__MARKER_NONE: u8 = 0xff;

/// Translation of `STBI__RESTART()`.
fn is_restart(x: u8) -> bool {
    (0xd0..=0xd7).contains(&x)
}

/// Take a -128..127 value and clamp it and convert to 0..255.
/// Translation of `stbi__clamp()`.
#[inline]
fn clamp(x: i32) -> u8 {
    // trick to use a single test to catch both cases
    if (x as u32) > 255 {
        if x < 0 {
            return 0;
        }
        if x > 255 {
            return 255;
        }
    }
    x as u8
}

/// `stbi__f2f()`.
const fn f2f(x: f32) -> i32 {
    (x * 4096.0 + 0.5) as i32
}

/// `stbi__fsh()`.
#[inline]
fn fsh(x: i32) -> i32 {
    x.wrapping_mul(4096)
}

/// The 1D IDCT derived from jidctint (DCT_ISLOW). Translation of
/// `STBI__IDCT_1D()`: returns `(t0, t1, t2, t3, x0, x1, x2, x3)`.
#[inline]
#[allow(clippy::too_many_arguments)]
fn idct_1d(s0: i32, s1: i32, s2: i32, s3: i32, s4: i32, s5: i32, s6: i32, s7: i32) -> [i32; 8] {
    let w = |a: i32, b: i32| a.wrapping_mul(b);
    let mut p2 = s2;
    let mut p3 = s6;
    let mut p1 = w(p2.wrapping_add(p3), f2f(0.5411961));
    let mut t2 = p1.wrapping_add(w(p3, f2f(-1.847759065)));
    let mut t3 = p1.wrapping_add(w(p2, f2f(0.765366865)));
    p2 = s0;
    p3 = s4;
    let mut t0 = fsh(p2.wrapping_add(p3));
    let mut t1 = fsh(p2.wrapping_sub(p3));
    let x0 = t0.wrapping_add(t3);
    let x3 = t0.wrapping_sub(t3);
    let x1 = t1.wrapping_add(t2);
    let x2 = t1.wrapping_sub(t2);
    t0 = s7;
    t1 = s5;
    t2 = s3;
    t3 = s1;
    p3 = t0.wrapping_add(t2);
    let mut p4 = t1.wrapping_add(t3);
    p1 = t0.wrapping_add(t3);
    p2 = t1.wrapping_add(t2);
    let p5 = w(p3.wrapping_add(p4), f2f(1.175875602));
    t0 = w(t0, f2f(0.298631336));
    t1 = w(t1, f2f(2.053119869));
    t2 = w(t2, f2f(3.072711026));
    t3 = w(t3, f2f(1.501321110));
    p1 = p5.wrapping_add(w(p1, f2f(-0.899976223)));
    p2 = p5.wrapping_add(w(p2, f2f(-2.562915447)));
    p3 = w(p3, f2f(-1.961570560));
    p4 = w(p4, f2f(-0.390180644));
    t3 = t3.wrapping_add(p1.wrapping_add(p4));
    t2 = t2.wrapping_add(p2.wrapping_add(p3));
    t1 = t1.wrapping_add(p2.wrapping_add(p4));
    t0 = t0.wrapping_add(p1.wrapping_add(p3));
    [t0, t1, t2, t3, x0, x1, x2, x3]
}

/// Translation of `stbi__idct_block()`: write the 8x8 block at `out`.
fn idct_block(out: &mut [u8], out_stride: usize, data: &[i16; 64]) {
    let mut val = [0i32; 64];

    // columns
    for i in 0..8 {
        let d = |k: usize| data[i + k] as i32;
        // if all zeroes, shortcut -- this avoids dequantizing 0s and IDCTing
        if d(8) == 0
            && d(16) == 0
            && d(24) == 0
            && d(32) == 0
            && d(40) == 0
            && d(48) == 0
            && d(56) == 0
        {
            //    no shortcut                 0     seconds
            //    (1|2|3|4|5|6|7)==0          0     seconds
            //    all separate               -0.047 seconds
            //    1 && 2|3 && 4|5 && 6|7:    -0.047 seconds
            let dcterm = d(0) * 4;
            for k in 0..8 {
                val[i + k * 8] = dcterm;
            }
        } else {
            let [t0, t1, t2, t3, mut x0, mut x1, mut x2, mut x3] =
                idct_1d(d(0), d(8), d(16), d(24), d(32), d(40), d(48), d(56));
            // constants scaled things up by 1<<12; let's bring them back
            // down, but keep 2 extra bits of precision
            x0 = x0.wrapping_add(512);
            x1 = x1.wrapping_add(512);
            x2 = x2.wrapping_add(512);
            x3 = x3.wrapping_add(512);
            val[i] = x0.wrapping_add(t3) >> 10;
            val[i + 56] = x0.wrapping_sub(t3) >> 10;
            val[i + 8] = x1.wrapping_add(t2) >> 10;
            val[i + 48] = x1.wrapping_sub(t2) >> 10;
            val[i + 16] = x2.wrapping_add(t1) >> 10;
            val[i + 40] = x2.wrapping_sub(t1) >> 10;
            val[i + 24] = x3.wrapping_add(t0) >> 10;
            val[i + 32] = x3.wrapping_sub(t0) >> 10;
        }
    }

    for i in 0..8 {
        let v = &val[i * 8..i * 8 + 8];
        let o = &mut out[i * out_stride..i * out_stride + 8];
        // no fast case since the first 1D IDCT spread components out
        let [t0, t1, t2, t3, mut x0, mut x1, mut x2, mut x3] =
            idct_1d(v[0], v[1], v[2], v[3], v[4], v[5], v[6], v[7]);
        // constants scaled things up by 1<<12, plus we had 1<<2 from first
        // loop, plus horizontal and vertical each scale by sqrt(8) so together
        // we've got an extra 1<<3, so 1<<17 total we need to remove.
        // so we want to round that, which means adding 0.5 * 1<<17,
        // aka 65536. Also, we'll end up with -128 to 127 that we want
        // to encode as 0..255 by adding 128, so we'll add that before the shift
        let bias = 65536 + (128 << 17);
        x0 = x0.wrapping_add(bias);
        x1 = x1.wrapping_add(bias);
        x2 = x2.wrapping_add(bias);
        x3 = x3.wrapping_add(bias);
        // tried computing the shifts into temps, or'ing the temps to see
        // if any were out of range, but that was slower
        o[0] = clamp(x0.wrapping_add(t3) >> 17);
        o[7] = clamp(x0.wrapping_sub(t3) >> 17);
        o[1] = clamp(x1.wrapping_add(t2) >> 17);
        o[6] = clamp(x1.wrapping_sub(t2) >> 17);
        o[2] = clamp(x2.wrapping_add(t1) >> 17);
        o[5] = clamp(x2.wrapping_sub(t1) >> 17);
        o[3] = clamp(x3.wrapping_add(t0) >> 17);
        o[4] = clamp(x3.wrapping_sub(t0) >> 17);
    }
}

impl Jpeg<'_, '_> {
    /// Translation of `stbi__grow_buffer_unsafe()`.
    fn grow_buffer_unsafe(&mut self) {
        loop {
            let b = if self.nomore { 0 } else { self.s.get8() as u32 };
            if b == 0xff {
                let mut c = self.s.get8();
                while c == 0xff {
                    c = self.s.get8(); // consume fill bytes
                }
                if c != 0 {
                    self.marker = c;
                    self.nomore = true;
                    return;
                }
            }
            self.code_buffer |= b << (24 - self.code_bits);
            self.code_bits += 8;
            if self.code_bits > 24 {
                break;
            }
        }
    }

    /// Decode a jpeg huffman value from the bitstream.
    /// Translation of `stbi__jpeg_huff_decode()`.
    fn huff_decode(&mut self, h: &Huffman) -> i32 {
        if self.code_bits < 16 {
            self.grow_buffer_unsafe();
        }

        // look at the top FAST_BITS and determine what symbol ID it is,
        // if the code is <= FAST_BITS
        let c = ((self.code_buffer >> (32 - FAST_BITS)) & ((1 << FAST_BITS) - 1)) as usize;
        let k = h.fast[c] as usize;
        if k < 255 {
            let s = h.size[k] as i32;
            if s > self.code_bits {
                return -1;
            }
            self.code_buffer = self.code_buffer.wrapping_shl(s as u32);
            self.code_bits -= s;
            return h.values[k] as i32;
        }

        // naive test is to shift the code_buffer down so k bits are
        // valid, then test against maxcode. To speed this up, we've
        // preshifted maxcode left so that it has (16-k) 0s at the
        // end; in other words, regardless of the number of bits, it
        // wants to be compared against something shifted to have 16;
        // that way we don't need to shift inside the loop.
        let temp = self.code_buffer >> 16;
        let mut k = FAST_BITS as usize + 1;
        loop {
            if temp < h.maxcode[k] {
                break;
            }
            k += 1;
        }
        if k == 17 {
            // error! code not found
            self.code_bits -= 16;
            return -1;
        }

        if k as i32 > self.code_bits {
            return -1;
        }

        // convert the huffman code to the symbol id
        let c = ((self.code_buffer >> (32 - k)) & STBI__BMASK[k]) as i32 + h.delta[k];
        if !(0..256).contains(&c) {
            // symbol id out of bounds!
            return -1;
        }
        let c = c as usize;
        crate::sdl_assert!(
            ((self.code_buffer >> (32 - h.size[c] as u32)) & STBI__BMASK[h.size[c] as usize])
                == h.code[c] as u32
        );

        // convert the id to a symbol
        self.code_bits -= k as i32;
        self.code_buffer = self.code_buffer.wrapping_shl(k as u32);
        h.values[c] as i32
    }

    /// Combined JPEG 'receive' and JPEG 'extend', since baseline always
    /// extends everything it receives. Translation of `stbi__extend_receive()`.
    fn extend_receive(&mut self, n: i32) -> i32 {
        if self.code_bits < n {
            self.grow_buffer_unsafe();
        }
        if self.code_bits < n {
            return 0; // ran out of bits from stream, return 0s intead of continuing
        }

        let sgn = (self.code_buffer >> 31) as i32; // sign bit always in MSB; 0 if MSB clear (positive), 1 if MSB set (negative)
        let mut k = self.code_buffer.rotate_left(n as u32);
        self.code_buffer = k & !STBI__BMASK[n as usize];
        k &= STBI__BMASK[n as usize];
        self.code_bits -= n;
        (k as i32).wrapping_add(STBI__JBIAS[n as usize] & (sgn - 1))
    }

    /// Get some unsigned bits. Translation of `stbi__jpeg_get_bits()`.
    fn get_bits(&mut self, n: i32) -> i32 {
        if self.code_bits < n {
            self.grow_buffer_unsafe();
        }
        if self.code_bits < n {
            return 0; // ran out of bits from stream, return 0s intead of continuing
        }
        let mut k = self.code_buffer.rotate_left(n as u32);
        self.code_buffer = k & !STBI__BMASK[n as usize];
        k &= STBI__BMASK[n as usize];
        self.code_bits -= n;
        k as i32
    }

    /// Translation of `stbi__jpeg_get_bit()`.
    fn get_bit(&mut self) -> bool {
        if self.code_bits < 1 {
            self.grow_buffer_unsafe();
        }
        if self.code_bits < 1 {
            return false; // ran out of bits from stream, return 0s intead of continuing
        }
        let k = self.code_buffer;
        self.code_buffer <<= 1;
        self.code_bits -= 1;
        k & 0x80000000 != 0
    }

    /// Decode one 64-entry block. Translation of `stbi__jpeg_decode_block()`.
    fn decode_block(
        &mut self,
        data: &mut [i16; 64],
        hd: usize,
        ha: usize,
        b: usize,
        tq: usize,
    ) -> Result<()> {
        if self.code_bits < 16 {
            self.grow_buffer_unsafe();
        }
        let hdc = std::mem::take(&mut self.huff_dc[hd]);
        let t = self.huff_decode(&hdc);
        self.huff_dc[hd] = hdc;
        if !(0..=15).contains(&t) {
            return err("Corrupt JPEG");
        }

        // 0 all the ac values now so we can do it 32-bits at a time
        data.fill(0);

        let diff = if t != 0 { self.extend_receive(t) } else { 0 };
        if !addints_valid(self.img_comp[b].dc_pred, diff) {
            return err("Corrupt JPEG");
        }
        let dc = self.img_comp[b].dc_pred + diff;
        self.img_comp[b].dc_pred = dc;
        let dequant = self.dequant[tq];
        if !mul2shorts_valid(dc, dequant[0] as i32) {
            return err("Corrupt JPEG");
        }
        data[0] = (dc * dequant[0] as i32) as i16;

        // decode AC components, see JPEG spec
        let hac = std::mem::take(&mut self.huff_ac[ha]);
        let result = (|| {
            let mut k = 1usize;
            loop {
                if self.code_bits < 16 {
                    self.grow_buffer_unsafe();
                }
                let c = ((self.code_buffer >> (32 - FAST_BITS)) & ((1 << FAST_BITS) - 1)) as usize;
                let r = self.fast_ac[ha][c] as i32;
                if r != 0 {
                    // fast-AC path
                    k += ((r >> 4) & 15) as usize; // run
                    let s = r & 15; // combined length
                    if s > self.code_bits {
                        return err("Combined length longer than code bits available");
                    }
                    self.code_buffer = self.code_buffer.wrapping_shl(s as u32);
                    self.code_bits -= s;
                    // decode into unzigzag'd location
                    let zig = STBI__JPEG_DEZIGZAG[k] as usize;
                    k += 1;
                    data[zig] = ((r >> 8).wrapping_mul(dequant[zig] as i32)) as i16;
                } else {
                    let rs = self.huff_decode(&hac);
                    if rs < 0 {
                        return err("Corrupt JPEG");
                    }
                    let s = rs & 15;
                    let r = rs >> 4;
                    if s == 0 {
                        if rs != 0xf0 {
                            break; // end block
                        }
                        k += 16;
                    } else {
                        k += r as usize;
                        // decode into unzigzag'd location
                        let zig = STBI__JPEG_DEZIGZAG[k] as usize;
                        k += 1;
                        data[zig] =
                            (self.extend_receive(s).wrapping_mul(dequant[zig] as i32)) as i16;
                    }
                }
                if k >= 64 {
                    break;
                }
            }
            Ok(())
        })();
        self.huff_ac[ha] = hac;
        result
    }

    /// Translation of `stbi__jpeg_decode_block_prog_dc()`.
    fn decode_block_prog_dc(&mut self, data: &mut [i16], hd: usize, b: usize) -> Result<()> {
        if self.spec_end != 0 {
            return err("Corrupt JPEG");
        }

        if self.code_bits < 16 {
            self.grow_buffer_unsafe();
        }

        if self.succ_high == 0 {
            // first scan for DC coefficient, must be first
            data.fill(0); // 0 all the ac values now
            let hdc = std::mem::take(&mut self.huff_dc[hd]);
            let t = self.huff_decode(&hdc);
            self.huff_dc[hd] = hdc;
            if !(0..=15).contains(&t) {
                return err("Corrupt JPEG");
            }
            let diff = if t != 0 { self.extend_receive(t) } else { 0 };

            if !addints_valid(self.img_comp[b].dc_pred, diff) {
                return err("Corrupt JPEG");
            }
            let dc = self.img_comp[b].dc_pred + diff;
            self.img_comp[b].dc_pred = dc;
            if !mul2shorts_valid(dc, 1 << self.succ_low) {
                return err("Corrupt JPEG");
            }
            data[0] = (dc * (1 << self.succ_low)) as i16;
        } else {
            // refinement scan for DC coefficient
            if self.get_bit() {
                data[0] = data[0].wrapping_add((1 << self.succ_low) as i16);
            }
        }
        Ok(())
    }

    /// Translation of `stbi__jpeg_decode_block_prog_ac()`.
    // @OPTIMIZE: store non-zigzagged during the decode passes,
    // and only de-zigzag when dequantizing
    fn decode_block_prog_ac(&mut self, data: &mut [i16], ha: usize) -> Result<()> {
        if self.spec_start == 0 {
            return err("Corrupt JPEG");
        }
        let hac = std::mem::take(&mut self.huff_ac[ha]);
        let result = self.decode_block_prog_ac_with(data, &hac, ha);
        self.huff_ac[ha] = hac;
        result
    }

    fn decode_block_prog_ac_with(
        &mut self,
        data: &mut [i16],
        hac: &Huffman,
        ha: usize,
    ) -> Result<()> {
        if self.succ_high == 0 {
            let shift = self.succ_low;

            if self.eob_run != 0 {
                self.eob_run -= 1;
                return Ok(());
            }

            let mut k = self.spec_start as usize;
            loop {
                if self.code_bits < 16 {
                    self.grow_buffer_unsafe();
                }
                let c = ((self.code_buffer >> (32 - FAST_BITS)) & ((1 << FAST_BITS) - 1)) as usize;
                let r = self.fast_ac[ha][c] as i32;
                if r != 0 {
                    // fast-AC path
                    k += ((r >> 4) & 15) as usize; // run
                    let s = r & 15; // combined length
                    if s > self.code_bits {
                        return err("Combined length longer than code bits available");
                    }
                    self.code_buffer = self.code_buffer.wrapping_shl(s as u32);
                    self.code_bits -= s;
                    let zig = STBI__JPEG_DEZIGZAG[k] as usize;
                    k += 1;
                    data[zig] = ((r >> 8).wrapping_mul(1 << shift)) as i16;
                } else {
                    let rs = self.huff_decode(hac);
                    if rs < 0 {
                        return err("Corrupt JPEG");
                    }
                    let s = rs & 15;
                    let r = rs >> 4;
                    if s == 0 {
                        if r < 15 {
                            self.eob_run = 1 << r;
                            if r != 0 {
                                self.eob_run += self.get_bits(r);
                            }
                            self.eob_run -= 1;
                            break;
                        }
                        k += 16;
                    } else {
                        k += r as usize;
                        let zig = STBI__JPEG_DEZIGZAG[k] as usize;
                        k += 1;
                        data[zig] = (self.extend_receive(s).wrapping_mul(1 << shift)) as i16;
                    }
                }
                if k as i32 > self.spec_end {
                    break;
                }
            }
        } else {
            // refinement scan for these AC coefficients

            let bit = (1i32 << self.succ_low) as i16;

            if self.eob_run != 0 {
                self.eob_run -= 1;
                for k in self.spec_start as usize..=self.spec_end as usize {
                    let p = STBI__JPEG_DEZIGZAG[k] as usize;
                    if data[p] != 0 && self.get_bit() && (data[p] & bit) == 0 {
                        if data[p] > 0 {
                            data[p] = data[p].wrapping_add(bit);
                        } else {
                            data[p] = data[p].wrapping_sub(bit);
                        }
                    }
                }
            } else {
                let mut k = self.spec_start as usize;
                loop {
                    let rs = self.huff_decode(hac); // @OPTIMIZE see if we can use the fast path here, advance-by-r is so slow, eh
                    if rs < 0 {
                        return err("Corrupt JPEG");
                    }
                    let mut s = rs & 15;
                    let mut r = rs >> 4;
                    if s == 0 {
                        if r < 15 {
                            self.eob_run = (1 << r) - 1;
                            if r != 0 {
                                self.eob_run += self.get_bits(r);
                            }
                            r = 64; // force end of block
                        } else {
                            // r=15 s=0 should write 16 0s, so we just do
                            // a run of 15 0s and then write s (which is 0),
                            // so we don't have to do anything special here
                        }
                    } else {
                        if s != 1 {
                            return err("Corrupt JPEG");
                        }
                        // sign bit
                        if self.get_bit() {
                            s = bit as i32;
                        } else {
                            s = -(bit as i32);
                        }
                    }

                    // advance by r
                    while k as i32 <= self.spec_end {
                        let p = STBI__JPEG_DEZIGZAG[k] as usize;
                        k += 1;
                        if data[p] != 0 {
                            if self.get_bit() && (data[p] & bit) == 0 {
                                if data[p] > 0 {
                                    data[p] = data[p].wrapping_add(bit);
                                } else {
                                    data[p] = data[p].wrapping_sub(bit);
                                }
                            }
                        } else {
                            if r == 0 {
                                data[p] = s as i16;
                                break;
                            }
                            r -= 1;
                        }
                    }
                    if k as i32 > self.spec_end {
                        break;
                    }
                }
            }
        }
        Ok(())
    }

    /// If there's a pending marker from the entropy stream, return that;
    /// otherwise, fetch from the stream and get a marker. If there's no
    /// marker, return 0xff, which is never a valid marker value.
    /// Translation of `stbi__get_marker()`.
    fn get_marker(&mut self) -> u8 {
        if self.marker != STBI__MARKER_NONE {
            let x = self.marker;
            self.marker = STBI__MARKER_NONE;
            return x;
        }
        let mut x = self.s.get8();
        if x != 0xff {
            return STBI__MARKER_NONE;
        }
        while x == 0xff {
            x = self.s.get8(); // consume repeated 0xff fill bytes
        }
        x
    }

    /// After a restart interval, reset the entropy decoder and the dc
    /// prediction. Translation of `stbi__jpeg_reset()`.
    fn reset(&mut self) {
        self.code_bits = 0;
        self.code_buffer = 0;
        self.nomore = false;
        for c in &mut self.img_comp {
            c.dc_pred = 0;
        }
        self.marker = STBI__MARKER_NONE;
        self.todo = if self.restart_interval != 0 {
            self.restart_interval
        } else {
            0x7fffffff
        };
        self.eob_run = 0;
        // no more than 1<<31 MCUs if no restart_interal? that's plenty safe,
        // since we don't even allow 1<<30 pixels
    }

    /// Count down the restart interval after an MCU: `Some(())` to stop
    /// the scan (no restart marker), `None` to go on.
    fn mcu_done(&mut self) -> bool {
        self.todo -= 1;
        if self.todo <= 0 {
            if self.code_bits < 24 {
                self.grow_buffer_unsafe();
            }
            // if it's NOT a restart, then just bail, so we get corrupt data
            // rather than no data
            if !is_restart(self.marker) {
                return true;
            }
            self.reset();
        }
        false
    }

    /// IDCT a block into component `n` at (`x`, `y`) pixels.
    fn idct_into(&mut self, n: usize, x: usize, y: usize, data: &[i16; 64]) {
        let comp = &mut self.img_comp[n];
        let w2 = comp.w2 as usize;
        idct_block(&mut comp.data[w2 * y + x..], w2, data);
    }

    /// Translation of `stbi__parse_entropy_coded_data()`.
    fn parse_entropy_coded_data(&mut self) -> Result<()> {
        self.reset();
        if !self.progressive {
            let mut data = [0i16; 64];
            if self.scan_n == 1 {
                let n = self.order[0];
                // non-interleaved data, we just need to process one block at a time,
                // in trivial scanline order
                // number of blocks to do just depends on how many actual "pixels" this
                // component has, independent of interleaved MCU blocking and such
                let w = (self.img_comp[n].x + 7) >> 3;
                let h = (self.img_comp[n].y + 7) >> 3;
                for j in 0..h as usize {
                    for i in 0..w as usize {
                        let c = &self.img_comp[n];
                        let (hd, ha, tq) = (c.hd as usize, c.ha as usize, c.tq as usize);
                        self.decode_block(&mut data, hd, ha, n, tq)?;
                        self.idct_into(n, i * 8, j * 8, &data);
                        // every data block is an MCU, so countdown the restart interval
                        if self.mcu_done() {
                            return Ok(());
                        }
                    }
                }
                Ok(())
            } else {
                // interleaved
                for j in 0..self.img_mcu_y as usize {
                    for i in 0..self.img_mcu_x as usize {
                        // scan an interleaved mcu... process scan_n components in order
                        for k in 0..self.scan_n as usize {
                            let n = self.order[k];
                            // scan out an mcu's worth of this component; that's just determined
                            // by the basic H and V specified for the component
                            let c = &self.img_comp[n];
                            let (cv, ch) = (c.v as usize, c.h as usize);
                            for y in 0..cv {
                                for x in 0..ch {
                                    let x2 = (i * ch + x) * 8;
                                    let y2 = (j * cv + y) * 8;
                                    let c = &self.img_comp[n];
                                    let (hd, ha, tq) =
                                        (c.hd as usize, c.ha as usize, c.tq as usize);
                                    self.decode_block(&mut data, hd, ha, n, tq)?;
                                    self.idct_into(n, x2, y2, &data);
                                }
                            }
                        }
                        // after all interleaved components, that's an interleaved MCU,
                        // so now count down the restart interval
                        if self.mcu_done() {
                            return Ok(());
                        }
                    }
                }
                Ok(())
            }
        } else if self.scan_n == 1 {
            let n = self.order[0];
            // non-interleaved data, we just need to process one block at a time,
            // in trivial scanline order
            // number of blocks to do just depends on how many actual "pixels" this
            // component has, independent of interleaved MCU blocking and such
            let w = (self.img_comp[n].x + 7) >> 3;
            let h = (self.img_comp[n].y + 7) >> 3;
            for j in 0..h as usize {
                for i in 0..w as usize {
                    let at = 64 * (i + j * self.img_comp[n].coeff_w as usize);
                    let mut coeff = std::mem::take(&mut self.img_comp[n].coeff);
                    let data = &mut coeff[at..at + 64];
                    let r = if self.spec_start == 0 {
                        let hd = self.img_comp[n].hd as usize;
                        self.decode_block_prog_dc(data, hd, n)
                    } else {
                        let ha = self.img_comp[n].ha as usize;
                        self.decode_block_prog_ac(data, ha)
                    };
                    self.img_comp[n].coeff = coeff;
                    r?;
                    // every data block is an MCU, so countdown the restart interval
                    if self.mcu_done() {
                        return Ok(());
                    }
                }
            }
            Ok(())
        } else {
            // interleaved
            for j in 0..self.img_mcu_y as usize {
                for i in 0..self.img_mcu_x as usize {
                    // scan an interleaved mcu... process scan_n components in order
                    for k in 0..self.scan_n as usize {
                        let n = self.order[k];
                        // scan out an mcu's worth of this component; that's just determined
                        // by the basic H and V specified for the component
                        let (cv, ch) = (self.img_comp[n].v as usize, self.img_comp[n].h as usize);
                        for y in 0..cv {
                            for x in 0..ch {
                                let x2 = i * ch + x;
                                let y2 = j * cv + y;
                                let at = 64 * (x2 + y2 * self.img_comp[n].coeff_w as usize);
                                let hd = self.img_comp[n].hd as usize;
                                let mut coeff = std::mem::take(&mut self.img_comp[n].coeff);
                                let r = self.decode_block_prog_dc(&mut coeff[at..at + 64], hd, n);
                                self.img_comp[n].coeff = coeff;
                                r?;
                            }
                        }
                    }
                    // after all interleaved components, that's an interleaved MCU,
                    // so now count down the restart interval
                    if self.mcu_done() {
                        return Ok(());
                    }
                }
            }
            Ok(())
        }
    }

    /// Translation of `stbi__jpeg_finish()`: dequantize and idct the data
    /// of a progressive image.
    fn finish(&mut self) {
        if self.progressive {
            for n in 0..self.s.img_n as usize {
                let w = (self.img_comp[n].x + 7) >> 3;
                let h = (self.img_comp[n].y + 7) >> 3;
                for j in 0..h as usize {
                    for i in 0..w as usize {
                        let at = 64 * (i + j * self.img_comp[n].coeff_w as usize);
                        let dequant = self.dequant[self.img_comp[n].tq as usize];
                        let mut data = [0i16; 64];
                        // stbi__jpeg_dequantize()
                        for (k, d) in data.iter_mut().enumerate() {
                            *d = self.img_comp[n].coeff[at + k].wrapping_mul(dequant[k] as i16);
                            self.img_comp[n].coeff[at + k] = *d;
                        }
                        self.idct_into(n, i * 8, j * 8, &data);
                    }
                }
            }
        }
    }

    /// Translation of `stbi__process_marker()`; `Ok(false)` is upstream's
    /// failure without an error message.
    fn process_marker(&mut self, m: u8) -> Result<bool> {
        match m {
            STBI__MARKER_NONE => {
                // no marker found
                return err("Corrupt JPEG");
            }

            0xDD => {
                // DRI - specify restart interval
                if self.s.get16be() != 4 {
                    return err("Corrupt JPEG");
                }
                self.restart_interval = self.s.get16be();
                return Ok(true);
            }

            0xDB => {
                // DQT - define quantization table
                let mut l = self.s.get16be() - 2;
                while l > 0 {
                    let q = self.s.get8() as i32;
                    let p = q >> 4;
                    let sixteen = p != 0;
                    let t = (q & 15) as usize;
                    if p != 0 && p != 1 {
                        return err("Corrupt JPEG");
                    }
                    if t > 3 {
                        return err("Corrupt JPEG");
                    }

                    for i in 0..64 {
                        self.dequant[t][STBI__JPEG_DEZIGZAG[i] as usize] = if sixteen {
                            self.s.get16be() as u16
                        } else {
                            self.s.get8() as u16
                        };
                    }
                    l -= if sixteen { 129 } else { 65 };
                }
                return Ok(l == 0);
            }

            0xC4 => {
                // DHT - define huffman table
                let mut l = self.s.get16be() - 2;
                while l > 0 {
                    let mut sizes = [0i32; 16];
                    let mut n = 0i32;
                    let q = self.s.get8() as i32;
                    let tc = q >> 4;
                    let th = (q & 15) as usize;
                    if tc > 1 || th > 3 {
                        return err("Corrupt JPEG");
                    }
                    for s in sizes.iter_mut() {
                        *s = self.s.get8() as i32;
                        n += *s;
                    }
                    if n > 256 {
                        return err("Corrupt JPEG"); // Loop over i < n would write past end of values!
                    }
                    l -= 17;
                    let h = if tc == 0 {
                        &mut self.huff_dc[th]
                    } else {
                        &mut self.huff_ac[th]
                    };
                    build_huffman(h, &sizes)?;
                    for i in 0..n as usize {
                        let v = self.s.get8();
                        if tc == 0 {
                            self.huff_dc[th].values[i] = v;
                        } else {
                            self.huff_ac[th].values[i] = v;
                        }
                    }
                    if tc != 0 {
                        build_fast_ac(&mut self.fast_ac[th], &self.huff_ac[th]);
                    }
                    l -= n;
                }
                return Ok(l == 0);
            }
            _ => {}
        }

        // check for comment block or APP blocks
        if (0xE0..=0xEF).contains(&m) || m == 0xFE {
            let mut l = self.s.get16be();
            if l < 2 {
                return err("Corrupt JPEG");
            }
            l -= 2;

            if m == 0xE0 && l >= 5 {
                // JFIF APP0 segment
                const TAG: [u8; 5] = *b"JFIF\0";
                let mut ok = true;
                for &t in &TAG {
                    if self.s.get8() != t {
                        ok = false;
                    }
                }
                l -= 5;
                if ok {
                    self.jfif = true;
                }
            } else if m == 0xEE && l >= 12 {
                // Adobe APP14 segment
                const TAG: [u8; 6] = *b"Adobe\0";
                let mut ok = true;
                for &t in &TAG {
                    if self.s.get8() != t {
                        ok = false;
                    }
                }
                l -= 6;
                if ok {
                    self.s.get8(); // version
                    self.s.get16be(); // flags0
                    self.s.get16be(); // flags1
                    self.app14_color_transform = self.s.get8() as i32; // color transform
                    l -= 6;
                }
            }

            self.s.skip(l);
            return Ok(true);
        }

        err("Corrupt JPEG")
    }

    /// After we see SOS. Translation of `stbi__process_scan_header()`.
    fn process_scan_header(&mut self) -> Result<()> {
        let ls = self.s.get16be();
        self.scan_n = self.s.get8() as i32;
        if self.scan_n < 1 || self.scan_n > 4 || self.scan_n > self.s.img_n {
            return err("Corrupt JPEG");
        }
        if ls != 6 + 2 * self.scan_n {
            return err("Corrupt JPEG");
        }
        for i in 0..self.scan_n as usize {
            let id = self.s.get8() as i32;
            let q = self.s.get8() as i32;
            let Some(which) = (0..self.s.img_n as usize).find(|&w| self.img_comp[w].id == id)
            else {
                // (upstream fails without setting an error)
                return err("Corrupt JPEG"); // no match
            };
            self.img_comp[which].hd = q >> 4;
            if self.img_comp[which].hd > 3 {
                return err("Corrupt JPEG");
            }
            self.img_comp[which].ha = q & 15;
            if self.img_comp[which].ha > 3 {
                return err("Corrupt JPEG");
            }
            self.order[i] = which;
        }

        self.spec_start = self.s.get8() as i32;
        self.spec_end = self.s.get8() as i32; // should be 63, but might be 0
        let aa = self.s.get8() as i32;
        self.succ_high = aa >> 4;
        self.succ_low = aa & 15;
        if self.progressive {
            if self.spec_start > 63
                || self.spec_end > 63
                || self.spec_start > self.spec_end
                || self.succ_high > 13
                || self.succ_low > 13
            {
                return err("Corrupt JPEG");
            }
        } else {
            if self.spec_start != 0 {
                return err("Corrupt JPEG");
            }
            if self.succ_high != 0 || self.succ_low != 0 {
                return err("Corrupt JPEG");
            }
            self.spec_end = 63;
        }

        Ok(())
    }

    /// Translation of `stbi__process_frame_header()`.
    fn process_frame_header(&mut self, scan: i32) -> Result<()> {
        let mut h_max = 1;
        let mut v_max = 1;
        let lf = self.s.get16be();
        if lf < 11 {
            return err("Corrupt JPEG"); // JPEG
        }
        let p = self.s.get8();
        if p != 8 {
            return err("JPEG format not supported: 8-bit only"); // JPEG baseline
        }
        self.s.img_y = self.s.get16be() as u32;
        if self.s.img_y == 0 {
            return err("JPEG format not supported: delayed height"); // Legal, but we don't handle it--but neither does IJG
        }
        self.s.img_x = self.s.get16be() as u32;
        if self.s.img_x == 0 {
            return err("Corrupt JPEG"); // JPEG requires
        }
        if self.s.img_y > STBI_MAX_DIMENSIONS {
            return err("Very large image (corrupt?)");
        }
        if self.s.img_x > STBI_MAX_DIMENSIONS {
            return err("Very large image (corrupt?)");
        }
        let c = self.s.get8() as i32;
        if c != 3 && c != 1 && c != 4 {
            return err("Corrupt JPEG");
        }
        self.s.img_n = c;
        for comp in &mut self.img_comp[..c as usize] {
            comp.data = Vec::new();
            comp.linebuf = Vec::new();
        }

        if lf != 8 + 3 * self.s.img_n {
            return err("Corrupt JPEG");
        }

        self.rgb = 0;
        for i in 0..self.s.img_n as usize {
            const RGB: [u8; 3] = *b"RGB";
            self.img_comp[i].id = self.s.get8() as i32;
            if self.s.img_n == 3 && self.img_comp[i].id == RGB[i] as i32 {
                self.rgb += 1;
            }
            let q = self.s.get8() as i32;
            self.img_comp[i].h = q >> 4;
            if self.img_comp[i].h == 0 || self.img_comp[i].h > 4 {
                return err("Corrupt JPEG");
            }
            self.img_comp[i].v = q & 15;
            if self.img_comp[i].v == 0 || self.img_comp[i].v > 4 {
                return err("Corrupt JPEG");
            }
            self.img_comp[i].tq = self.s.get8() as i32;
            if self.img_comp[i].tq > 3 {
                return err("Corrupt JPEG");
            }
        }

        if scan != STBI__SCAN_LOAD {
            return Ok(());
        }

        if !mad3sizes_valid(self.s.img_x as i32, self.s.img_y as i32, self.s.img_n, 0) {
            return err("Image too large to decode");
        }

        for comp in &self.img_comp[..self.s.img_n as usize] {
            h_max = h_max.max(comp.h);
            v_max = v_max.max(comp.v);
        }

        // check that plane subsampling factors are integer ratios; our resamplers can't deal with fractional ratios
        // and I've never seen a non-corrupted JPEG file actually use them
        for comp in &self.img_comp[..self.s.img_n as usize] {
            if h_max % comp.h != 0 {
                return err("Corrupt JPEG");
            }
            if v_max % comp.v != 0 {
                return err("Corrupt JPEG");
            }
        }

        // compute interleaved mcu info
        self.img_h_max = h_max;
        self.img_v_max = v_max;
        self.img_mcu_w = h_max * 8;
        self.img_mcu_h = v_max * 8;
        // these sizes can't be more than 17 bits
        self.img_mcu_x = (self.s.img_x as i32 + self.img_mcu_w - 1) / self.img_mcu_w;
        self.img_mcu_y = (self.s.img_y as i32 + self.img_mcu_h - 1) / self.img_mcu_h;

        for i in 0..self.s.img_n as usize {
            let (img_x, img_y) = (self.s.img_x as i32, self.s.img_y as i32);
            let (mcu_x, mcu_y, progressive) = (self.img_mcu_x, self.img_mcu_y, self.progressive);
            let comp = &mut self.img_comp[i];
            // number of effective pixels (e.g. for non-interleaved MCU)
            comp.x = (img_x * comp.h + h_max - 1) / h_max;
            comp.y = (img_y * comp.v + v_max - 1) / v_max;
            // to simplify generation, we'll allocate enough memory to decode
            // the bogus oversized data from using interleaved MCUs and their
            // big blocks (e.g. a 16x16 iMCU on an image of width 33); we won't
            // discard the extra data until colorspace conversion
            //
            // img_mcu_x, img_mcu_y: <=17 bits; comp[i].h and .v are <=4 (checked earlier)
            // so these muls can't overflow with 32-bit ints (which we require)
            comp.w2 = mcu_x * comp.h * 8;
            comp.h2 = mcu_y * comp.v * 8;
            comp.coeff = Vec::new();
            comp.linebuf = Vec::new();
            // (upstream aligns the blocks for idct using mmx/sse)
            comp.data = match malloc_mad2(comp.w2, comp.h2, 15) {
                Some(d) => d,
                None => return err("Out of memory"),
            };
            if progressive {
                // w2, h2 are multiples of 8 (see above)
                comp.coeff_w = comp.w2 / 8;
                comp.coeff_h = comp.h2 / 8;
                if !mad3sizes_valid(comp.w2, comp.h2, 2, 15) {
                    return err("Out of memory");
                }
                comp.coeff = vec![0; (comp.w2 * comp.h2) as usize];
            }
        }

        Ok(())
    }

    /// Translation of `stbi__decode_jpeg_header()`; `Ok(false)` is
    /// upstream's failure without an error message.
    fn decode_jpeg_header(&mut self, scan: i32) -> Result<bool> {
        self.jfif = false;
        self.app14_color_transform = -1; // valid values are 0,1,2
        self.marker = STBI__MARKER_NONE; // initialize cached marker to empty
        let mut m = self.get_marker();
        if m != 0xd8 {
            return err("Corrupt JPEG");
        }
        if scan == STBI__SCAN_TYPE {
            return Ok(true);
        }
        m = self.get_marker();
        while !(m == 0xc0 || m == 0xc1 || m == 0xc2) {
            if !self.process_marker(m)? {
                return Ok(false);
            }
            m = self.get_marker();
            while m == STBI__MARKER_NONE {
                // some files have extra padding after their blocks, so ok, we'll scan
                if self.s.at_eof() {
                    return err("Corrupt JPEG");
                }
                m = self.get_marker();
            }
        }
        self.progressive = m == 0xc2;
        self.process_frame_header(scan)?;
        Ok(true)
    }

    /// Some JPEGs have junk at end, skip over it but if we find what looks
    /// like a valid marker, resume there. Translation of
    /// `stbi__skip_jpeg_junk_at_end()`.
    fn skip_jpeg_junk_at_end(&mut self) -> u8 {
        while !self.s.at_eof() {
            let mut x = self.s.get8();
            while x == 0xff {
                // might be a marker
                if self.s.at_eof() {
                    return STBI__MARKER_NONE;
                }
                x = self.s.get8();
                if x != 0x00 && x != 0xff {
                    // not a stuffed zero or lead-in to another marker, looks
                    // like an actual marker, return it
                    return x;
                }
                // stuffed zero has x=0 now which ends the loop, meaning we go
                // back to regular scan loop.
                // repeated 0xff keeps trying to read the next byte of the marker.
            }
        }
        STBI__MARKER_NONE
    }

    /// Decode image to YCbCr format. Translation of `stbi__decode_jpeg_image()`.
    fn decode_jpeg_image(&mut self) -> Result<()> {
        for c in &mut self.img_comp {
            c.data = Vec::new();
            c.coeff = Vec::new();
        }
        self.restart_interval = 0;
        if !self.decode_jpeg_header(STBI__SCAN_LOAD)? {
            return err("Corrupt JPEG");
        }
        let mut m = self.get_marker();
        while m != 0xd9 {
            if m == 0xda {
                self.process_scan_header()?;
                self.parse_entropy_coded_data()?;
                if self.marker == STBI__MARKER_NONE {
                    self.marker = self.skip_jpeg_junk_at_end();
                    // if we reach eof without hitting a marker, stbi__get_marker() below will fail and we'll eventually return 0
                }
                m = self.get_marker();
                if is_restart(m) {
                    m = self.get_marker();
                }
            } else if m == 0xdc {
                let ld = self.s.get16be();
                let nl = self.s.get16be() as u32;
                if ld != 4 {
                    return err("Corrupt JPEG");
                }
                if nl != self.s.img_y {
                    return err("Corrupt JPEG");
                }
                m = self.get_marker();
            } else {
                match self.process_marker(m) {
                    Ok(true) => {}
                    // (upstream stops decoding and reports success here)
                    _ => return Ok(()),
                }
                m = self.get_marker();
            }
        }
        if self.progressive {
            self.finish();
        }
        Ok(())
    }
}

/// Static jfif-centered resampling (across block boundaries).
/// `resample_row_func`: the resampled row is written to `out` unless the
/// function returns `in_near` itself (`None` here).
type ResampleRowFunc =
    fn(out: &mut [u8], in_near: &[u8], in_far: &[u8], w: usize, hs: usize) -> bool;

/// `stbi__div4()`.
fn div4(x: i32) -> u8 {
    (x >> 2) as u8
}

/// `stbi__div16()`.
fn div16(x: i32) -> u8 {
    (x >> 4) as u8
}

/// Translation of `resample_row_1()`: the row is used as is.
fn resample_row_1(_out: &mut [u8], _in_near: &[u8], _in_far: &[u8], _w: usize, _hs: usize) -> bool {
    false
}

/// Translation of `stbi__resample_row_v_2()`.
fn resample_row_v_2(out: &mut [u8], in_near: &[u8], in_far: &[u8], w: usize, _hs: usize) -> bool {
    // need to generate two samples vertically for every one in input
    for i in 0..w {
        out[i] = div4(3 * in_near[i] as i32 + in_far[i] as i32 + 2);
    }
    true
}

/// Translation of `stbi__resample_row_h_2()`.
fn resample_row_h_2(out: &mut [u8], in_near: &[u8], _in_far: &[u8], w: usize, _hs: usize) -> bool {
    // need to generate two samples horizontally for every one in input
    let input = in_near;
    let inp = |i: usize| input[i] as i32;

    if w == 1 {
        // if only one sample, can't do any interpolation
        out[0] = input[0];
        out[1] = input[0];
        return true;
    }

    out[0] = input[0];
    out[1] = div4(inp(0) * 3 + inp(1) + 2);
    let mut i = 1;
    while i < w - 1 {
        let n = 3 * inp(i) + 2;
        out[i * 2] = div4(n + inp(i - 1));
        out[i * 2 + 1] = div4(n + inp(i + 1));
        i += 1;
    }
    out[i * 2] = div4(inp(w - 2) * 3 + inp(w - 1) + 2);
    out[i * 2 + 1] = input[w - 1];

    true
}

/// Translation of `stbi__resample_row_hv_2()`.
fn resample_row_hv_2(out: &mut [u8], in_near: &[u8], in_far: &[u8], w: usize, _hs: usize) -> bool {
    // need to generate 2x2 samples for every one in input
    if w == 1 {
        let v = div4(3 * in_near[0] as i32 + in_far[0] as i32 + 2);
        out[0] = v;
        out[1] = v;
        return true;
    }

    let mut t1 = 3 * in_near[0] as i32 + in_far[0] as i32;
    out[0] = div4(t1 + 2);
    for i in 1..w {
        let t0 = t1;
        t1 = 3 * in_near[i] as i32 + in_far[i] as i32;
        out[i * 2 - 1] = div16(3 * t0 + t1 + 8);
        out[i * 2] = div16(3 * t1 + t0 + 8);
    }
    out[w * 2 - 1] = div4(t1 + 2);

    true
}

/// Translation of `stbi__resample_row_generic()`: resample with
/// nearest-neighbor.
fn resample_row_generic(
    out: &mut [u8],
    in_near: &[u8],
    _in_far: &[u8],
    w: usize,
    hs: usize,
) -> bool {
    for i in 0..w {
        for j in 0..hs {
            out[i * hs + j] = in_near[i];
        }
    }
    true
}

/// `stbi__float2fixed()`.
const fn float2fixed(x: f32) -> i32 {
    ((x * 4096.0 + 0.5) as i32) << 8
}

/// This is a reduced-precision calculation of YCbCr-to-RGB introduced to
/// make sure the code produces the same results in both SIMD and scalar.
/// Translation of `stbi__YCbCr_to_RGB_row()`.
fn ycbcr_to_rgb_row(out: &mut [u8], y: &[u8], pcb: &[u8], pcr: &[u8], count: usize, step: usize) {
    for i in 0..count {
        let y_fixed = ((y[i] as i32) << 20) + (1 << 19); // rounding
        let cr = pcr[i] as i32 - 128;
        let cb = pcb[i] as i32 - 128;
        let mut r = y_fixed + cr * float2fixed(1.40200);
        let mut g = y_fixed
            + (cr * -float2fixed(0.71414))
            + ((cb * -float2fixed(0.34414)) & 0xffff0000u32 as i32);
        let mut b = y_fixed + cb * float2fixed(1.77200);
        r >>= 20;
        g >>= 20;
        b >>= 20;
        let o = &mut out[i * step..];
        o[0] = clamp(r);
        o[1] = clamp(g);
        o[2] = clamp(b);
        o[3] = 255;
    }
}

/// Translation of `stbi__resample`.
struct Resample {
    resample: ResampleRowFunc,
    /// Offsets of the two input rows in the component's data.
    line0: usize,
    line1: usize,
    hs: usize,
    vs: usize,      // expansion factor in each axis
    w_lores: usize, // horizontal pixels pre-expansion
    ystep: usize,   // how far through vertical expansion we are
    ypos: i32,      // which pre-expansion row we're on
}

/// Fast 0..255 * 0..255 => 0..255 rounded multiplication.
/// Translation of `stbi__blinn_8x8()`.
fn blinn_8x8(x: u8, y: u8) -> u8 {
    let t = x as u32 * y as u32 + 128;
    ((t + (t >> 8)) >> 8) as u8
}

/// Translation of `output_jpeg_nv12()`.
fn output_jpeg_nv12(z: &Jpeg<'_, '_>, nv12: &mut Nv12<'_>) -> Result<()> {
    let img_x = z.s.img_x as usize;
    let img_y = z.s.img_y as usize;
    let pitch = nv12.pitch as usize;

    // FIXME (upstream): the planes are read with the image width as their
    // stride, but each component's rows are `w2` bytes apart; images whose
    // width isn't a multiple of the MCU width come out sheared.
    // Copy the Y plane
    let y_plane = &z.img_comp[0].data;
    if pitch == img_x {
        nv12.dst[..img_y * img_x].copy_from_slice(&y_plane[..img_y * img_x]);
    } else {
        for i in 0..img_y {
            nv12.dst[i * pitch..i * pitch + img_x]
                .copy_from_slice(&y_plane[i * img_x..(i + 1) * img_x]);
        }
    }

    let uv_start = nv12.h as usize * pitch;
    if z.s.img_n == 3 {
        // NV12: U and V are interleaved, each subsampled by 2
        const NV12_HS: isize = 2;
        const NV12_VS: isize = 2;
        let u_hs = (z.img_h_max / z.img_comp[1].h) as isize;
        let u_vs = (z.img_v_max / z.img_comp[1].v) as isize;
        let v_hs = (z.img_h_max / z.img_comp[2].h) as isize;
        let v_vs = (z.img_v_max / z.img_comp[2].v) as isize;
        let (u, v) = (&z.img_comp[1], &z.img_comp[2]);
        // (upstream reads outside the planes when a component is subsampled
        // by more than 2, stepping backwards through them)
        let at = |data: &[u8], i: isize| -> Result<u8> {
            usize::try_from(i)
                .ok()
                .and_then(|i| data.get(i).copied())
                .ok_or_else(|| crate::error::Error::new("Unexpected chroma subsampling"))
        };
        for i in 0..(img_y as isize + 1) / 2 {
            let mut src_u = i * (1 + (NV12_VS - u_vs)) * u.x as isize;
            let mut src_v = i * (1 + (NV12_VS - v_vs)) * v.x as isize;
            let mut dst = uv_start + i as usize * pitch;
            for _ in 0..img_x.div_ceil(2) {
                nv12.dst[dst] = at(&u.data, src_u)?;
                dst += 1;
                src_u += 1 + (NV12_HS - u_hs);
                nv12.dst[dst] = at(&v.data, src_v)?;
                dst += 1;
                src_v += 1 + (NV12_HS - v_hs);
            }
        }
    } else {
        // Grayscale
        for i in 0..img_y.div_ceil(2) {
            let row = uv_start + i * pitch;
            nv12.dst[row..row + img_x.div_ceil(2) * 2].fill(0x80);
        }
    }

    Ok(())
}

/// The decoded components of a JPEG and its size (`load_jpeg_image()`'s
/// result).
pub(crate) struct JpegImage {
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) comp: i32,
    pub(crate) data: Vec<u8>,
}

/// Translation of `load_jpeg_image()`.
fn load_jpeg_image(
    z: &mut Jpeg<'_, '_>,
    req_comp: i32,
    nv12: Option<&mut Nv12<'_>>,
) -> Result<JpegImage> {
    z.s.img_n = 0; // make stbi__cleanup_jpeg safe

    // validate req_comp
    if !(0..=4).contains(&req_comp) {
        return err("Internal error");
    }

    // load a jpeg image from whichever source, but leave in YCbCr format
    z.decode_jpeg_image()?;

    // determine actual number of components to generate
    let n = if req_comp != 0 {
        req_comp
    } else if z.s.img_n >= 3 {
        3
    } else {
        1
    } as usize;

    let is_rgb = z.s.img_n == 3 && (z.rgb == 3 || (z.app14_color_transform == 0 && !z.jfif));

    let decode_n = if z.s.img_n == 3 && n < 3 && !is_rgb {
        1
    } else {
        z.s.img_n
    } as usize;

    // nothing to do if no components requested; check this now to avoid
    // accessing uninitialized coutput[0] later
    if decode_n == 0 {
        // (upstream fails without setting an error)
        return err("Corrupt JPEG");
    }

    let img_x = z.s.img_x as usize;
    let img_y = z.s.img_y as usize;

    // resample and color-convert
    if let Some(nv12) = nv12 {
        if nv12.w != img_x as i32 || nv12.h != img_y as i32 {
            return err("Unexpected size");
        }

        if is_rgb {
            return err("Can't convert RGB to NV12");
        }

        output_jpeg_nv12(z, nv12)?;
        return Ok(JpegImage {
            x: img_x as i32,
            y: img_y as i32,
            comp: if z.s.img_n >= 3 { 3 } else { 1 },
            data: Vec::new(),
        });
    }

    let mut res_comp: Vec<Resample> = Vec::with_capacity(decode_n);
    for k in 0..decode_n {
        // allocate line buffer big enough for upsampling off the edges
        // with upsample factor of 4
        z.img_comp[k].linebuf = vec![0; img_x + 3];

        let hs = (z.img_h_max / z.img_comp[k].h) as usize;
        let vs = (z.img_v_max / z.img_comp[k].v) as usize;
        let resample: ResampleRowFunc = match (hs, vs) {
            (1, 1) => resample_row_1,
            (1, 2) => resample_row_v_2,
            (2, 1) => resample_row_h_2,
            (2, 2) => resample_row_hv_2,
            _ => resample_row_generic,
        };
        res_comp.push(Resample {
            resample,
            line0: 0,
            line1: 0,
            hs,
            vs,
            ystep: vs >> 1,
            w_lores: img_x.div_ceil(hs),
            ypos: 0,
        });
    }

    // can't error after this so, this is safe
    if !mad3sizes_valid(n as i32, img_x as i32, img_y as i32, 1) {
        return err("Out of memory");
    }
    let mut output = vec![0u8; n * img_x * img_y + 1];

    // now go ahead and resample
    // (whether each component's row is its line buffer or a data row)
    let mut use_line: [Option<usize>; 4] = [None; 4];
    for j in 0..img_y {
        let out = &mut output[n * img_x * j..];
        for k in 0..decode_n {
            let r = &mut res_comp[k];
            let comp = &mut z.img_comp[k];
            let y_bot = r.ystep >= (r.vs >> 1);
            let (near, far) = if y_bot {
                (r.line1, r.line0)
            } else {
                (r.line0, r.line1)
            };
            let resampled = (r.resample)(
                &mut comp.linebuf,
                &comp.data[near..],
                &comp.data[far..],
                r.w_lores,
                r.hs,
            );
            use_line[k] = if resampled { None } else { Some(near) };
            r.ystep += 1;
            if r.ystep >= r.vs {
                r.ystep = 0;
                r.line0 = r.line1;
                r.ypos += 1;
                if r.ypos < comp.y {
                    r.line1 += comp.w2 as usize;
                }
            }
        }
        let comps = &z.img_comp;
        let coutput = |k: usize| -> &[u8] {
            match use_line[k] {
                Some(at) => &comps[k].data[at..],
                None => &comps[k].linebuf,
            }
        };
        if n >= 3 {
            let y = coutput(0);
            if z.s.img_n == 3 {
                if is_rgb {
                    for i in 0..img_x {
                        let o = &mut out[i * n..];
                        o[0] = y[i];
                        o[1] = coutput(1)[i];
                        o[2] = coutput(2)[i];
                        if n > 3 {
                            o[3] = 255;
                        }
                    }
                } else {
                    ycbcr_to_rgb_row_n(out, y, coutput(1), coutput(2), img_x, n);
                }
            } else if z.s.img_n == 4 {
                if z.app14_color_transform == 0 {
                    // CMYK
                    for i in 0..img_x {
                        let m = coutput(3)[i];
                        let o = &mut out[i * n..];
                        o[0] = blinn_8x8(coutput(0)[i], m);
                        o[1] = blinn_8x8(coutput(1)[i], m);
                        o[2] = blinn_8x8(coutput(2)[i], m);
                        if n > 3 {
                            o[3] = 255;
                        }
                    }
                } else if z.app14_color_transform == 2 {
                    // YCCK
                    ycbcr_to_rgb_row_n(out, y, coutput(1), coutput(2), img_x, n);
                    for i in 0..img_x {
                        let m = coutput(3)[i];
                        let o = &mut out[i * n..];
                        o[0] = blinn_8x8(255 - o[0], m);
                        o[1] = blinn_8x8(255 - o[1], m);
                        o[2] = blinn_8x8(255 - o[2], m);
                    }
                } else {
                    // YCbCr + alpha?  Ignore the fourth channel for now
                    ycbcr_to_rgb_row_n(out, y, coutput(1), coutput(2), img_x, n);
                }
            } else {
                for i in 0..img_x {
                    let o = &mut out[i * n..];
                    o[0] = y[i];
                    o[1] = y[i];
                    o[2] = y[i];
                    if n > 3 {
                        o[3] = 255; // not used if n==3
                    }
                }
            }
        } else if is_rgb {
            if n == 1 {
                for i in 0..img_x {
                    out[i] = compute_y(
                        coutput(0)[i] as i32,
                        coutput(1)[i] as i32,
                        coutput(2)[i] as i32,
                    );
                }
            } else {
                for i in 0..img_x {
                    out[i * 2] = compute_y(
                        coutput(0)[i] as i32,
                        coutput(1)[i] as i32,
                        coutput(2)[i] as i32,
                    );
                    out[i * 2 + 1] = 255;
                }
            }
        } else if z.s.img_n == 4 && z.app14_color_transform == 0 {
            for i in 0..img_x {
                let m = coutput(3)[i];
                let r = blinn_8x8(coutput(0)[i], m);
                let g = blinn_8x8(coutput(1)[i], m);
                let b = blinn_8x8(coutput(2)[i], m);
                out[i * n] = compute_y(r as i32, g as i32, b as i32);
                out[i * n + 1] = 255;
            }
        } else if z.s.img_n == 4 && z.app14_color_transform == 2 {
            for i in 0..img_x {
                out[i * n] = blinn_8x8(255 - coutput(0)[i], coutput(3)[i]);
                out[i * n + 1] = 255;
            }
        } else {
            let y = coutput(0);
            if n == 1 {
                out[..img_x].copy_from_slice(&y[..img_x]);
            } else {
                for i in 0..img_x {
                    out[i * 2] = y[i];
                    out[i * 2 + 1] = 255;
                }
            }
        }
    }
    output.truncate(n * img_x * img_y);
    Ok(JpegImage {
        x: img_x as i32,
        y: img_y as i32,
        comp: if z.s.img_n >= 3 { 3 } else { 1 }, // report original components, not output
        data: output,
    })
}

/// `YCbCr_to_RGB_kernel(out, y, pcb, pcr, count, step)`: the C kernel
/// always writes a fourth byte, which for `step == 3` the next pixel
/// overwrites (and the extra byte allocated past the image absorbs).
fn ycbcr_to_rgb_row_n(out: &mut [u8], y: &[u8], pcb: &[u8], pcr: &[u8], count: usize, step: usize) {
    if step == 4 || count == 0 {
        ycbcr_to_rgb_row(out, y, pcb, pcr, count, step);
    } else {
        // (the last pixel's fourth byte lands on the next row, or on the
        // allocation's spare byte, and is overwritten or unused)
        ycbcr_to_rgb_row(out, y, pcb, pcr, count, step);
    }
}

/// Translation of `stbi__jpeg_load()`; with `nv12`, the image is decoded
/// to that NV12 buffer instead (and the result has no pixels).
pub(crate) fn jpeg_load(
    s: &mut Context<'_>,
    req_comp: i32,
    nv12: Option<&mut Nv12<'_>>,
) -> Result<JpegImage> {
    let mut j = new_jpeg(s);
    load_jpeg_image(&mut j, req_comp, nv12)
}

fn new_jpeg<'c, 'a>(s: &'c mut Context<'a>) -> Jpeg<'c, 'a> {
    Jpeg {
        s,
        huff_dc: Default::default(),
        huff_ac: Default::default(),
        dequant: [[0; 64]; 4],
        fast_ac: [[0; 1 << FAST_BITS]; 4],
        img_h_max: 0,
        img_v_max: 0,
        img_mcu_x: 0,
        img_mcu_y: 0,
        img_mcu_w: 0,
        img_mcu_h: 0,
        img_comp: Default::default(),
        code_buffer: 0,
        code_bits: 0,
        marker: 0,
        nomore: false,
        progressive: false,
        spec_start: 0,
        spec_end: 0,
        succ_high: 0,
        succ_low: 0,
        eob_run: 0,
        jfif: false,
        app14_color_transform: 0,
        rgb: 0,
        scan_n: 0,
        order: [0; 4],
        restart_interval: 0,
        todo: 0,
    }
}

/// Translation of `stbi__jpeg_test()`.
pub(crate) fn jpeg_test(s: &mut Context<'_>) -> bool {
    let r = {
        let mut j = new_jpeg(s);
        matches!(j.decode_jpeg_header(STBI__SCAN_TYPE), Ok(true))
    };
    s.rewind();
    r
}

#[allow(unused_imports)]
use Image as _;
