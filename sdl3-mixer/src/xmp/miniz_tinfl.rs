// Rust translation of src/miniz_tinfl.c and the tinfl parts of src/miniz.h from libxmp 4.7.3 (as vendored by SDL_mixer).
// Copyright 2013-2014 RAD Game Tools and Valve Software
// Copyright 2010-2014 Rich Geldreich and Tenacious Software LLC
// All Rights Reserved.
// (miniz_tinfl.c is under the MIT license; miniz.h, from miniz.c 2.2.0, calls
// itself public domain.) This is an altered (translated) version of the
// original software; see LICENSE.txt.

//! miniz's inflate (tinfl), for the MUSE container.
//!
//! `tinfl_decompress()` is a coroutine that returns
//! `TINFL_STATUS_HAS_MORE_OUTPUT` when its output buffer is full, for
//! `tinfl_decompress_mem_to_heap()` (its only user here) to grow the buffer
//! and call it again. Here the decompressor grows the buffer itself where it
//! would return, the same way: everything else it returns ends the
//! decompression, as `tinfl_decompress_mem_to_heap()` never sets
//! `TINFL_FLAG_HAS_MORE_INPUT`. (`tinfl_decompress_mem_to_mem()`,
//! `tinfl_decompress_mem_to_callback()` and the allocation functions aren't
//! used by libxmp.)

use super::common::LIBXMP_DEPACK_LIMIT;

/* Decompression flags used by tinfl_decompress(). */
/* TINFL_FLAG_PARSE_ZLIB_HEADER: If set, the input has a valid zlib header and ends with an adler32 checksum (it's a valid zlib stream). Otherwise, the input is a raw deflate stream. */
/* TINFL_FLAG_HAS_MORE_INPUT: If set, there are more input bytes available beyond the end of the supplied input buffer. If clear, the input buffer contains all remaining input. */
/* TINFL_FLAG_USING_NON_WRAPPING_OUTPUT_BUF: If set, the output buffer is large enough to hold the entire decompressed stream. If clear, the output buffer is at least the size of the dictionary (typically 32KB). */
/* TINFL_FLAG_COMPUTE_ADLER32: Force adler-32 checksum computation of the decompressed bytes. */
pub(crate) const TINFL_FLAG_PARSE_ZLIB_HEADER: u32 = 1;
pub(crate) const TINFL_FLAG_HAS_MORE_INPUT: u32 = 2;
pub(crate) const TINFL_FLAG_USING_NON_WRAPPING_OUTPUT_BUF: u32 = 4;
pub(crate) const TINFL_FLAG_COMPUTE_ADLER32: u32 = 8;

/* Return status. */
/* (only the ones that can be returned here) */
const TINFL_STATUS_FAILED_CANNOT_MAKE_PROGRESS: i32 = -4;
const TINFL_STATUS_ADLER32_MISMATCH: i32 = -2;
const TINFL_STATUS_FAILED: i32 = -1;
const TINFL_STATUS_DONE: i32 = 0;

/* Internal/private bits follow. */
const TINFL_MAX_HUFF_TABLES: usize = 3;
const TINFL_MAX_HUFF_SYMBOLS_0: usize = 288;
const TINFL_MAX_HUFF_SYMBOLS_1: usize = 32;
const TINFL_MAX_HUFF_SYMBOLS_2: usize = 19;
const TINFL_FAST_LOOKUP_BITS: u32 = 10;
const TINFL_FAST_LOOKUP_SIZE: usize = 1 << TINFL_FAST_LOOKUP_BITS;

/* (TINFL_USE_64BIT_BITBUF, as on the 64-bit targets; with the 32-bit bit
 * buffer the output is the same) */
type TinflBitBuf = u64;

/// Translation of `struct tinfl_decompressor_tag`.
struct TinflDecompressor {
    m_zhdr0: u32,
    m_zhdr1: u32,
    m_z_adler32: u32,
    m_final: u32,
    m_type: u32,
    m_check_adler32: u32,
    m_table_sizes: [u32; TINFL_MAX_HUFF_TABLES],
    m_look_up: [[i16; TINFL_FAST_LOOKUP_SIZE]; TINFL_MAX_HUFF_TABLES],
    m_tree_0: [i16; TINFL_MAX_HUFF_SYMBOLS_0 * 2],
    m_tree_1: [i16; TINFL_MAX_HUFF_SYMBOLS_1 * 2],
    m_tree_2: [i16; TINFL_MAX_HUFF_SYMBOLS_2 * 2],
    m_code_size_0: [u8; TINFL_MAX_HUFF_SYMBOLS_0],
    m_code_size_1: [u8; TINFL_MAX_HUFF_SYMBOLS_1],
    m_code_size_2: [u8; TINFL_MAX_HUFF_SYMBOLS_2],
    m_raw_header: [u8; 4],
    m_len_codes: [u8; TINFL_MAX_HUFF_SYMBOLS_0 + TINFL_MAX_HUFF_SYMBOLS_1 + 137],
}

impl TinflDecompressor {
    fn new() -> Box<Self> {
        Box::new(TinflDecompressor {
            m_zhdr0: 0,
            m_zhdr1: 0,
            m_z_adler32: 0,
            m_final: 0,
            m_type: 0,
            m_check_adler32: 0,
            m_table_sizes: [0; TINFL_MAX_HUFF_TABLES],
            m_look_up: [[0; TINFL_FAST_LOOKUP_SIZE]; TINFL_MAX_HUFF_TABLES],
            m_tree_0: [0; TINFL_MAX_HUFF_SYMBOLS_0 * 2],
            m_tree_1: [0; TINFL_MAX_HUFF_SYMBOLS_1 * 2],
            m_tree_2: [0; TINFL_MAX_HUFF_SYMBOLS_2 * 2],
            m_code_size_0: [0; TINFL_MAX_HUFF_SYMBOLS_0],
            m_code_size_1: [0; TINFL_MAX_HUFF_SYMBOLS_1],
            m_code_size_2: [0; TINFL_MAX_HUFF_SYMBOLS_2],
            m_raw_header: [0; 4],
            m_len_codes: [0; TINFL_MAX_HUFF_SYMBOLS_0 + TINFL_MAX_HUFF_SYMBOLS_1 + 137],
        })
    }

    /// The tree of table `t` (`pTrees[t]`).
    fn tree(&self, t: usize) -> &[i16] {
        match t {
            0 => &self.m_tree_0,
            1 => &self.m_tree_1,
            _ => &self.m_tree_2,
        }
    }

    /// Translation of `tinfl_clear_tree()`.
    fn tinfl_clear_tree(&mut self) {
        if self.m_type == 0 {
            self.m_tree_0.fill(0);
        } else if self.m_type == 1 {
            self.m_tree_1.fill(0);
        } else {
            self.m_tree_2.fill(0);
        }
    }
}

/// The input and output of `tinfl_decompress()`, with its bit buffer.
struct Stream<'a> {
    input: &'a [u8],
    in_cur: usize,
    out: Vec<u8>,
    out_cur: usize,
    num_bits: u32,
    bit_buf: TinflBitBuf,
}

/// Why the decompression ended early (its status).
type Fail = i32;

impl Stream<'_> {
    /// `pIn_buf_end - pIn_buf_cur`
    #[inline]
    fn in_left(&self) -> usize {
        self.input.len() - self.in_cur
    }

    /// `pOut_buf_end - pOut_buf_cur`
    #[inline]
    fn out_left(&self) -> usize {
        self.out.len() - self.out_cur
    }

    /// Where upstream returns `TINFL_STATUS_HAS_MORE_OUTPUT` (with the
    /// output full) to `tinfl_decompress_mem_to_heap()`: its growing of the
    /// output buffer.
    fn grow(&mut self) -> Result<(), Fail> {
        let out_buf_capacity = self.out.len();
        let mut new_out_buf_capacity = out_buf_capacity * 2;
        if new_out_buf_capacity < 128 {
            new_out_buf_capacity = 128;
        }
        /* (MZ_REALLOC() refuses more than LIBXMP_DEPACK_LIMIT) */
        if new_out_buf_capacity as i64 > LIBXMP_DEPACK_LIMIT
            || self
                .out
                .try_reserve_exact(new_out_buf_capacity - out_buf_capacity)
                .is_err()
        {
            return Err(TINFL_STATUS_FAILED);
        }
        self.out.resize(new_out_buf_capacity, 0);
        Ok(())
    }

    /// `while (pOut_buf_cur >= pOut_buf_end) TINFL_CR_RETURN(n, TINFL_STATUS_HAS_MORE_OUTPUT);`
    #[inline]
    fn need_out(&mut self) -> Result<(), Fail> {
        while self.out_cur >= self.out.len() {
            self.grow()?;
        }
        Ok(())
    }

    /// `TINFL_GET_BYTE()` (the input is all there is)
    #[inline]
    fn get_byte(&mut self) -> Result<u32, Fail> {
        if self.in_cur >= self.input.len() {
            return Err(TINFL_STATUS_FAILED_CANNOT_MAKE_PROGRESS);
        }
        let c = self.input[self.in_cur] as u32;
        self.in_cur += 1;
        Ok(c)
    }

    /// `TINFL_NEED_BITS()`
    #[inline]
    fn need_bits(&mut self, n: u32) -> Result<(), Fail> {
        loop {
            let c = self.get_byte()?;
            self.bit_buf |= (c as TinflBitBuf) << self.num_bits;
            self.num_bits += 8;
            if self.num_bits >= n {
                return Ok(());
            }
        }
    }

    /// `TINFL_SKIP_BITS()`
    #[inline]
    fn skip_bits(&mut self, n: u32) -> Result<(), Fail> {
        if self.num_bits < n {
            self.need_bits(n)?;
        }
        self.bit_buf >>= n;
        self.num_bits -= n;
        Ok(())
    }

    /// `TINFL_GET_BITS()`
    #[inline]
    fn get_bits(&mut self, n: u32) -> Result<u32, Fail> {
        if self.num_bits < n {
            self.need_bits(n)?;
        }
        let b = (self.bit_buf & ((1u64 << n) - 1)) as u32;
        self.bit_buf >>= n;
        self.num_bits -= n;
        Ok(b)
    }

    /* TINFL_HUFF_BITBUF_FILL() is only used rarely, when the number of bytes remaining in the input buffer falls below 2. */
    /* It reads just enough bytes from the input stream that are needed to decode the next Huffman code (and absolutely no more). It works by trying to fully decode a */
    /* Huffman code by using whatever bits are currently present in the bit buffer. If this fails, it reads another byte, and tries again until it succeeds or until the */
    /* bit buffer contains >=15 bits (deflate's max. Huffman code size). */
    /// `TINFL_HUFF_BITBUF_FILL()`
    fn huff_bitbuf_fill(&mut self, look_up: &[i16], tree: &[i16]) -> Result<(), Fail> {
        loop {
            let mut temp =
                look_up[(self.bit_buf & (TINFL_FAST_LOOKUP_SIZE as u64 - 1)) as usize] as i32;
            if temp >= 0 {
                let code_len = (temp >> 9) as u32;
                if code_len != 0 && self.num_bits >= code_len {
                    break;
                }
            } else if self.num_bits > TINFL_FAST_LOOKUP_BITS {
                let mut code_len = TINFL_FAST_LOOKUP_BITS;
                loop {
                    temp = tree[(!temp + ((self.bit_buf >> code_len) & 1) as i32) as usize] as i32;
                    code_len += 1;
                    if !(temp < 0 && self.num_bits > code_len) {
                        break;
                    }
                }
                if temp >= 0 {
                    break;
                }
            }
            let c = self.get_byte()?;
            self.bit_buf |= (c as TinflBitBuf) << self.num_bits;
            self.num_bits += 8;
            if self.num_bits >= 15 {
                break;
            }
        }
        Ok(())
    }

    /* TINFL_HUFF_DECODE() decodes the next Huffman coded symbol. It's more complex than you would initially expect because the zlib API expects the decompressor to never read */
    /* beyond the final byte of the deflate stream. (In other words, when this macro wants to read another byte from the input, it REALLY needs another byte in order to fully */
    /* decode the next Huffman code.) Handling this properly is particularly important on raw deflate (non-zlib) streams, which aren't followed by a byte aligned adler-32. */
    /* The slow path is only executed at the very end of the input buffer. */
    /* v1.16: The original macro handled the case at the very end of the passed-in input buffer, but we also need to handle the case where the user passes in 1+zillion bytes */
    /* following the deflate data and our non-conservative read-ahead path won't kick in here on this code. This is much trickier. */
    /// `TINFL_HUFF_DECODE()`: the symbol, and the code length
    /// (`code_len_hack`).
    fn huff_decode(&mut self, look_up: &[i16], tree: &[i16]) -> Result<(u32, u32), Fail> {
        let mut temp: i32;
        let code_len: u32;
        if self.num_bits < 15 {
            if self.in_left() < 2 {
                self.huff_bitbuf_fill(look_up, tree)?;
            } else {
                self.bit_buf |= ((self.input[self.in_cur] as TinflBitBuf) << self.num_bits)
                    | ((self.input[self.in_cur + 1] as TinflBitBuf) << (self.num_bits + 8));
                self.in_cur += 2;
                self.num_bits += 16;
            }
        }
        temp = look_up[(self.bit_buf & (TINFL_FAST_LOOKUP_SIZE as u64 - 1)) as usize] as i32;
        if temp >= 0 {
            code_len = (temp >> 9) as u32;
            temp &= 511;
        } else {
            let mut cl = TINFL_FAST_LOOKUP_BITS;
            loop {
                temp = tree[(!temp + ((self.bit_buf >> cl) & 1) as i32) as usize] as i32;
                cl += 1;
                if temp >= 0 {
                    break;
                }
            }
            code_len = cl;
        }
        let sym = temp as u32;
        self.bit_buf >>= code_len;
        self.num_bits -= code_len;
        Ok((sym, code_len)) /* FIXME: workaround for miniz/#229 */
    }
}

/// Translation of `tinfl_decompress()`, decompressing all of `input`
/// (`pIn_buf_next`) into `s.out` from its start, for the flags of
/// `tinfl_decompress_mem_to_heap()` (`TINFL_FLAG_USING_NON_WRAPPING_OUTPUT_BUF`
/// without `TINFL_FLAG_HAS_MORE_INPUT`): its status.
fn tinfl_decompress(r: &mut TinflDecompressor, s: &mut Stream<'_>, decomp_flags: u32) -> i32 {
    let status = match tinfl_decompress_body(r, s, decomp_flags) {
        Ok(()) => TINFL_STATUS_DONE,
        Err(status) => status,
    };

    /* common_exit: */
    /* As long as we aren't telling the caller that we NEED more input to make forward progress: */
    /* Put back any bytes from the bitbuf in case we've looked ahead too far on gzip, or other Deflate streams followed by arbitrary data. */
    /* We need to be very careful here to NOT push back any bytes we definitely know we need to make forward progress, though, or we'll lock the caller up into an inf loop. */
    if status != TINFL_STATUS_FAILED_CANNOT_MAKE_PROGRESS {
        while s.in_cur > 0 && s.num_bits >= 8 {
            s.in_cur -= 1;
            s.num_bits -= 8;
        }
    }
    let mut status = status;
    if (decomp_flags & (TINFL_FLAG_PARSE_ZLIB_HEADER | TINFL_FLAG_COMPUTE_ADLER32)) != 0
        && status >= 0
    {
        let mut ptr = &s.out[..s.out_cur];
        let mut buf_len = ptr.len();
        let mut s1 = r.m_check_adler32 & 0xffff;
        let mut s2 = r.m_check_adler32 >> 16;
        let mut block_len = buf_len % 5552;
        while buf_len != 0 {
            let mut i = 0;
            while i + 7 < block_len {
                for &b in &ptr[..8] {
                    s1 += b as u32;
                    s2 += s1;
                }
                i += 8;
                ptr = &ptr[8..];
            }
            while i < block_len {
                s1 += ptr[0] as u32;
                s2 += s1;
                ptr = &ptr[1..];
                i += 1;
            }
            s1 %= 65521;
            s2 %= 65521;
            buf_len -= block_len;
            block_len = 5552;
        }
        r.m_check_adler32 = (s2 << 16) + s1;
        if status == TINFL_STATUS_DONE
            && (decomp_flags & TINFL_FLAG_PARSE_ZLIB_HEADER) != 0
            && r.m_check_adler32 != r.m_z_adler32
        {
            status = TINFL_STATUS_ADLER32_MISMATCH;
        }
    }
    status
}

/// The coroutine of `tinfl_decompress()`, from `TINFL_CR_BEGIN` to
/// `TINFL_CR_FINISH`: `Ok` for `TINFL_STATUS_DONE`.
fn tinfl_decompress_body(
    r: &mut TinflDecompressor,
    s: &mut Stream<'_>,
    decomp_flags: u32,
) -> Result<(), Fail> {
    static S_LENGTH_BASE: [u16; 31] = [
        3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115,
        131, 163, 195, 227, 258, 0, 0,
    ];
    static S_LENGTH_EXTRA: [u8; 31] = [
        0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0, 0, 0,
    ];
    static S_DIST_BASE: [u16; 32] = [
        1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
        2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577, 0, 0,
    ];
    static S_DIST_EXTRA: [u8; 32] = [
        0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12,
        13, 13, 0, 0,
    ];
    static S_LENGTH_DEZIGZAG: [u8; 19] = [
        16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
    ];
    static S_MIN_TABLE_SIZES: [u16; 3] = [257, 1, 4];

    let non_wrapping = decomp_flags & TINFL_FLAG_USING_NON_WRAPPING_OUTPUT_BUF != 0;

    s.bit_buf = 0;
    s.num_bits = 0;
    let mut dist: u32;
    let mut counter: u32;
    let mut num_extra: u32;
    r.m_zhdr0 = 0;
    r.m_zhdr1 = 0;
    r.m_z_adler32 = 1;
    r.m_check_adler32 = 1;
    if decomp_flags & TINFL_FLAG_PARSE_ZLIB_HEADER != 0 {
        r.m_zhdr0 = s.get_byte()?;
        r.m_zhdr1 = s.get_byte()?;
        let c = ((r.m_zhdr0 * 256 + r.m_zhdr1) % 31 != 0)
            || (r.m_zhdr1 & 32) != 0
            || ((r.m_zhdr0 & 15) != 8);
        /* (the window size is checked against wrapping output buffers only) */
        if c {
            return Err(TINFL_STATUS_FAILED);
        }
    }

    loop {
        r.m_final = s.get_bits(3)?;
        r.m_type = r.m_final >> 1;
        if r.m_type == 0 {
            let n = s.num_bits & 7;
            s.skip_bits(n)?;
            for counter in 0..4 {
                if s.num_bits != 0 {
                    r.m_raw_header[counter] = s.get_bits(8)? as u8;
                } else {
                    r.m_raw_header[counter] = s.get_byte()? as u8;
                }
            }
            counter = r.m_raw_header[0] as u32 | ((r.m_raw_header[1] as u32) << 8);
            if counter != (0xFFFF ^ (r.m_raw_header[2] as u32 | ((r.m_raw_header[3] as u32) << 8)))
            {
                return Err(TINFL_STATUS_FAILED);
            }
            while counter != 0 && s.num_bits != 0 {
                dist = s.get_bits(8)?;
                s.need_out()?;
                s.out[s.out_cur] = dist as u8;
                s.out_cur += 1;
                counter -= 1;
            }
            while counter != 0 {
                s.need_out()?;
                if s.in_cur >= s.input.len() {
                    return Err(TINFL_STATUS_FAILED_CANNOT_MAKE_PROGRESS);
                }
                let n = s.out_left().min(s.in_left()).min(counter as usize);
                s.out[s.out_cur..s.out_cur + n].copy_from_slice(&s.input[s.in_cur..s.in_cur + n]);
                s.in_cur += n;
                s.out_cur += n;
                counter -= n as u32;
            }
        } else if r.m_type == 3 {
            return Err(TINFL_STATUS_FAILED);
        } else {
            if r.m_type == 1 {
                r.m_table_sizes[0] = 288;
                r.m_table_sizes[1] = 32;
                r.m_code_size_1.fill(5);
                let p = &mut r.m_code_size_0;
                p[..=143].fill(8);
                p[144..=255].fill(9);
                p[256..=279].fill(7);
                p[280..=287].fill(8);
            } else {
                for counter in 0..3 {
                    r.m_table_sizes[counter] = s.get_bits([5, 5, 4][counter])?;
                    r.m_table_sizes[counter] += S_MIN_TABLE_SIZES[counter] as u32;
                }
                r.m_code_size_2.fill(0);
                for counter in 0..r.m_table_sizes[2] as usize {
                    let v = s.get_bits(3)?;
                    r.m_code_size_2[S_LENGTH_DEZIGZAG[counter] as usize] = v as u8;
                }
                r.m_table_sizes[2] = 19;
            }
            loop {
                let t = r.m_type as usize;
                let mut total_syms = [0u32; 16];
                let mut next_code = [0u32; 17];
                r.m_look_up[t].fill(0);
                r.tinfl_clear_tree();
                let table_size = r.m_table_sizes[t] as usize;
                {
                    let code_size: &[u8] = match t {
                        0 => &r.m_code_size_0,
                        1 => &r.m_code_size_1,
                        _ => &r.m_code_size_2,
                    };
                    for &c in &code_size[..table_size] {
                        total_syms[c as usize] += 1;
                    }
                }
                let mut used_syms = 0;
                let mut total = 0;
                next_code[0] = 0;
                next_code[1] = 0;
                for i in 1..=15 {
                    used_syms += total_syms[i];
                    total = (total + total_syms[i]) << 1;
                    next_code[i + 1] = total;
                }
                if 65536 != total && used_syms > 1 {
                    return Err(TINFL_STATUS_FAILED);
                }
                let mut tree_next: i32 = -1;
                for sym_index in 0..table_size {
                    let code_size = match t {
                        0 => r.m_code_size_0[sym_index],
                        1 => r.m_code_size_1[sym_index],
                        _ => r.m_code_size_2[sym_index],
                    } as u32;
                    if code_size == 0 {
                        continue;
                    }
                    let mut cur_code = next_code[code_size as usize];
                    next_code[code_size as usize] += 1;
                    let mut rev_code = 0u32;
                    for _ in 0..code_size {
                        rev_code = (rev_code << 1) | (cur_code & 1);
                        cur_code >>= 1;
                    }
                    let look_up = &mut r.m_look_up[t];
                    if code_size <= TINFL_FAST_LOOKUP_BITS {
                        let k = ((code_size << 9) | sym_index as u32) as i16;
                        while (rev_code as usize) < TINFL_FAST_LOOKUP_SIZE {
                            look_up[rev_code as usize] = k;
                            rev_code += 1 << code_size;
                        }
                        continue;
                    }
                    let slot = rev_code as usize & (TINFL_FAST_LOOKUP_SIZE - 1);
                    let mut tree_cur = look_up[slot] as i32;
                    if tree_cur == 0 {
                        look_up[slot] = tree_next as i16;
                        tree_cur = tree_next;
                        tree_next -= 2;
                    }
                    let tree: &mut [i16] = match t {
                        0 => &mut r.m_tree_0,
                        1 => &mut r.m_tree_1,
                        _ => &mut r.m_tree_2,
                    };
                    rev_code >>= TINFL_FAST_LOOKUP_BITS - 1;
                    let mut j = code_size;
                    while j > TINFL_FAST_LOOKUP_BITS + 1 {
                        rev_code >>= 1;
                        tree_cur -= (rev_code & 1) as i32;
                        let idx = (-tree_cur - 1) as usize;
                        if tree[idx] == 0 {
                            tree[idx] = tree_next as i16;
                            tree_cur = tree_next;
                            tree_next -= 2;
                        } else {
                            tree_cur = tree[idx] as i32;
                        }
                        j -= 1;
                    }
                    rev_code >>= 1;
                    tree_cur -= (rev_code & 1) as i32;
                    tree[(-tree_cur - 1) as usize] = sym_index as i16;
                }
                if r.m_type == 2 {
                    counter = 0;
                    while counter < (r.m_table_sizes[0] + r.m_table_sizes[1]) {
                        let (d, _) = s.huff_decode(&r.m_look_up[2], &r.m_tree_2)?;
                        dist = d;
                        if dist < 16 {
                            r.m_len_codes[counter as usize] = dist as u8;
                            counter += 1;
                            continue;
                        }
                        if dist == 16 && counter == 0 {
                            return Err(TINFL_STATUS_FAILED);
                        }
                        num_extra = [2, 3, 7][(dist - 16) as usize];
                        let mut v = s.get_bits(num_extra)?;
                        v += [3, 3, 11][(dist - 16) as usize];
                        let fill = if dist == 16 {
                            r.m_len_codes[counter as usize - 1]
                        } else {
                            0
                        };
                        r.m_len_codes[counter as usize..(counter + v) as usize].fill(fill);
                        counter += v;
                    }
                    if (r.m_table_sizes[0] + r.m_table_sizes[1]) != counter {
                        return Err(TINFL_STATUS_FAILED);
                    }
                    let n0 = r.m_table_sizes[0] as usize;
                    let n1 = r.m_table_sizes[1] as usize;
                    r.m_code_size_0[..n0].copy_from_slice(&r.m_len_codes[..n0]);
                    r.m_code_size_1[..n1].copy_from_slice(&r.m_len_codes[n0..n0 + n1]);
                }
                if r.m_type == 0 {
                    break;
                }
                r.m_type -= 1;
            }
            loop {
                loop {
                    if s.in_left() < 4 || s.out_left() < 2 {
                        let (c, code_len_hack) = s.huff_decode(&r.m_look_up[0], r.tree(0))?;
                        counter = c;
                        /* FIXME: workaround for miniz/#229 */
                        if code_len_hack == 0 {
                            return Err(TINFL_STATUS_FAILED);
                        }
                        if counter >= 256 {
                            break;
                        }
                        s.need_out()?;
                        s.out[s.out_cur] = counter as u8;
                        s.out_cur += 1;
                    } else {
                        let mut sym2: i32;
                        let mut code_len: u32;
                        if s.num_bits < 30 {
                            let p = &s.input[s.in_cur..];
                            let v = u32::from_le_bytes([p[0], p[1], p[2], p[3]]);
                            s.bit_buf |= (v as TinflBitBuf) << s.num_bits;
                            s.in_cur += 4;
                            s.num_bits += 32;
                        }
                        sym2 = r.m_look_up[0]
                            [(s.bit_buf & (TINFL_FAST_LOOKUP_SIZE as u64 - 1)) as usize]
                            as i32;
                        if sym2 >= 0 {
                            code_len = (sym2 >> 9) as u32;
                        } else {
                            code_len = TINFL_FAST_LOOKUP_BITS;
                            loop {
                                sym2 = r.m_tree_0
                                    [(!sym2 + ((s.bit_buf >> code_len) & 1) as i32) as usize]
                                    as i32;
                                code_len += 1;
                                if sym2 >= 0 {
                                    break;
                                }
                            }
                        }
                        /* FIXME: workaround for miniz/#229 */
                        if code_len == 0 {
                            return Err(TINFL_STATUS_FAILED);
                        }
                        counter = sym2 as u32;
                        s.bit_buf >>= code_len;
                        s.num_bits -= code_len;
                        if counter & 256 != 0 {
                            break;
                        }

                        sym2 = r.m_look_up[0]
                            [(s.bit_buf & (TINFL_FAST_LOOKUP_SIZE as u64 - 1)) as usize]
                            as i32;
                        if sym2 >= 0 {
                            code_len = (sym2 >> 9) as u32;
                        } else {
                            code_len = TINFL_FAST_LOOKUP_BITS;
                            loop {
                                sym2 = r.m_tree_0
                                    [(!sym2 + ((s.bit_buf >> code_len) & 1) as i32) as usize]
                                    as i32;
                                code_len += 1;
                                if sym2 >= 0 {
                                    break;
                                }
                            }
                        }
                        s.bit_buf >>= code_len;
                        s.num_bits -= code_len;

                        s.out[s.out_cur] = counter as u8;
                        if sym2 & 256 != 0 {
                            s.out_cur += 1;
                            counter = sym2 as u32;
                            break;
                        }
                        s.out[s.out_cur + 1] = sym2 as u8;
                        s.out_cur += 2;
                    }
                }
                counter &= 511;
                if counter == 256 {
                    break;
                }

                num_extra = S_LENGTH_EXTRA[(counter - 257) as usize] as u32;
                counter = S_LENGTH_BASE[(counter - 257) as usize] as u32;
                if num_extra != 0 {
                    let extra_bits = s.get_bits(num_extra)?;
                    counter += extra_bits;
                }

                let (d, _) = s.huff_decode(&r.m_look_up[1], &r.m_tree_1)?;
                dist = d;
                num_extra = S_DIST_EXTRA[dist as usize] as u32;
                dist = S_DIST_BASE[dist as usize] as u32;
                if num_extra != 0 {
                    let extra_bits = s.get_bits(num_extra)?;
                    dist += extra_bits;
                }

                let mut dist_from_out_buf_start = s.out_cur;
                if (dist == 0
                    || dist as usize > dist_from_out_buf_start
                    || dist_from_out_buf_start == 0)
                    && non_wrapping
                {
                    return Err(TINFL_STATUS_FAILED);
                }

                /* (out_buf_size_mask is all ones for the non-wrapping buffer) */
                let mut src = dist_from_out_buf_start - dist as usize;

                if s.out_cur.max(src) + counter as usize > s.out.len() {
                    while counter != 0 {
                        counter -= 1;
                        s.need_out()?;
                        s.out[s.out_cur] = s.out[dist_from_out_buf_start - dist as usize];
                        dist_from_out_buf_start += 1;
                        s.out_cur += 1;
                    }
                    continue;
                } else if counter >= 9 && counter <= dist {
                    /* (MINIZ_USE_UNALIGNED_LOADS_AND_STORES) */
                    let src_end = src + (counter & !7) as usize;
                    loop {
                        s.out.copy_within(src..src + 8, s.out_cur);
                        s.out_cur += 8;
                        src += 8;
                        if src >= src_end {
                            break;
                        }
                    }
                    counter &= 7;
                    if counter < 3 {
                        if counter != 0 {
                            s.out[s.out_cur] = s.out[src];
                            if counter > 1 {
                                s.out[s.out_cur + 1] = s.out[src + 1];
                            }
                            s.out_cur += counter as usize;
                        }
                        continue;
                    }
                }
                while counter > 2 {
                    s.out[s.out_cur] = s.out[src];
                    s.out[s.out_cur + 1] = s.out[src + 1];
                    s.out[s.out_cur + 2] = s.out[src + 2];
                    s.out_cur += 3;
                    src += 3;
                    counter -= 3;
                }
                if counter > 0 {
                    s.out[s.out_cur] = s.out[src];
                    if counter > 1 {
                        s.out[s.out_cur + 1] = s.out[src + 1];
                    }
                    s.out_cur += counter as usize;
                }
            }
        }
        if r.m_final & 1 != 0 {
            break;
        }
    }

    /* Ensure byte alignment and put back any bytes from the bitbuf if we've looked ahead too far on gzip, or other Deflate streams followed by arbitrary data. */
    /* I'm being super conservative here. A number of simplifications can be made to the byte alignment part, and the Adler32 check shouldn't ever need to worry about reading from the bitbuf now. */
    let n = s.num_bits & 7;
    s.skip_bits(n)?;
    while s.in_cur > 0 && s.num_bits >= 8 {
        s.in_cur -= 1;
        s.num_bits -= 8;
    }
    s.bit_buf &= !(!(0 as TinflBitBuf) << s.num_bits);
    /* MZ_ASSERT(!num_bits); (compiled out) */
    /* if this assert fires then we've read beyond the end of non-deflate/zlib streams with following data (such as gzip streams). */

    if decomp_flags & TINFL_FLAG_PARSE_ZLIB_HEADER != 0 {
        for _ in 0..4 {
            let v = if s.num_bits != 0 {
                s.get_bits(8)?
            } else {
                s.get_byte()?
            };
            r.m_z_adler32 = (r.m_z_adler32 << 8) | v;
        }
    }
    Ok(())
}

/* Higher level helper functions. */
/// Translation of `tinfl_decompress_mem_to_heap()`: the decompressed data,
/// or `None` on failure.
pub(crate) fn tinfl_decompress_mem_to_heap(src_buf: &[u8], flags: u32) -> Option<Vec<u8>> {
    let mut decomp = TinflDecompressor::new();
    let mut s = Stream {
        input: src_buf,
        in_cur: 0,
        out: Vec::new(),
        out_cur: 0,
        num_bits: 0,
        bit_buf: 0,
    };
    let status = tinfl_decompress(
        &mut decomp,
        &mut s,
        (flags & !TINFL_FLAG_HAS_MORE_INPUT) | TINFL_FLAG_USING_NON_WRAPPING_OUTPUT_BUF,
    );
    if status < 0 {
        return None;
    }
    let mut out = s.out;
    out.truncate(s.out_cur);
    Some(out)
}
